//! Opt-in reduced-resolution far meadow. Source placement and the final layer
//! stay at the original resolution; this is a spatial reconstruction approximation.
use crate::Gpu;

pub(super) fn size(full: (u32, u32), half: bool) -> (u32, u32) {
    if half { (full.0.div_ceil(2), full.1.div_ceil(2)) } else { full }
}

pub(super) fn target_bytes(full: (u32, u32), far: (u32, u32), motion: bool, half: bool) -> u64 {
    // Far color8 + nearest depth4 + material/normal/reveal24; merge color8 +
    // retained depth4. Each motion layer adds16. Half mode also owns low opaque4.
    u64::from(far.0) * u64::from(far.1) * (36 + (if motion {16} else {0}) + (if half {4} else {0}))
        + u64::from(full.0) * u64::from(full.1) * (12 + (if motion {16} else {0}))
}

pub(super) struct OpaqueTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}
impl OpaqueTarget {
    pub fn new(gpu: &Gpu, size: (u32, u32)) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("half meadow conservative opaque depth"),
            size: wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
            format:wgpu::TextureFormat::Depth32Float,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats:&[],
        });
        let view=texture.create_view(&Default::default());
        Self {texture,view}
    }
}
pub(super) struct DepthReduce {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}
impl DepthReduce {
    pub fn new(gpu: &Gpu) -> Self {
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:Some("half meadow depth input"),entries:&[wgpu::BindGroupLayoutEntry {
                binding:0,visibility:wgpu::ShaderStages::FRAGMENT,
                ty:wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None,
            }],
        });
        let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:Some("half meadow depth reduction"),bind_group_layouts:&[Some(&layout)],immediate_size:0,
        });
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:Some("half meadow depth reduction"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/meadow_half_depth.wgsl").into()),
        });
        let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("half meadow depth reduction"),layout:Some(&pl),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},
            primitive:Default::default(),
            depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Always),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[]}),
            multiview_mask:None,cache:gpu.pipeline_cache(),
        });
        Self {layout,pipeline}
    }
    /// Starts the existing aggregate-depth interval. The actual far depth pass
    /// ends it, so reduction and the seed copy are included in measured work.
    pub fn encode(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,source:&wgpu::TextureView,target:&OpaqueTarget,time:Option<(&wgpu::QuerySet,u32)>) {
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:Some("half meadow opaque input"),layout:&self.layout,
            entries:&[wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(source)}],
        });
        let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label:Some("half meadow conservative opaque reduction"),color_attachments:&[],
            depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                view:&target.view,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(0.0),store:wgpu::StoreOp::Store}),stencil_ops:None,
            }),
            timestamp_writes:time.map(|(query_set,i)|wgpu::RenderPassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:None}),
            occlusion_query_set:None,multiview_mask:None,
        });
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&bind,&[]);pass.draw(0..3,0..1);
    }
}

#[cfg(test)]
mod tests {
    fn footprint(x:u32,full:u32)->std::ops::Range<u32> {
        let low=full.div_ceil(2);(x*full/low)..(((x+1)*full).div_ceil(low).min(full))
    }
    #[test]
    fn odd_viewports_cover_every_overlapping_full_pixel_without_sky_loss() {
        for full in [1u32,2,3,5,538,717,958,1277,1916] {
            let low=full.div_ceil(2);let mut seen=vec![false;full as usize];
            for x in 0..low {
                let f=footprint(x,full);assert!(f.len()<=3 && !f.is_empty());
                for i in 0..full {
                    let overlap=i*low<(x+1)*full && (i+1)*low>x*full;
                    assert_eq!(f.contains(&i),overlap,"{full}/{low}: {x}, {i}");
                    if f.contains(&i) {seen[i as usize]=true;}
                }
                // A sky sample anywhere in the footprint must make the
                // conservative reversed-depth result zero, not nearest/MAX.
                for sky in f.clone() {
                    let reduced=f.clone().map(|i|if i==sky {0.0f32}else{0.8}).fold(1.0,f32::min);
                    assert_eq!(reduced,0.0);
                }
            }
            assert!(seen.into_iter().all(|x|x));
        }
        assert_eq!(super::size((1277,717),true),(639,359));
        assert_eq!(super::size((958,538),true),(479,269));
        assert_eq!(super::size((1277,717),false),(1277,717));
    }
    #[test]
    fn target_accounting_keeps_full_merge_and_counts_low_opaque_and_both_motion_layers() {
        let full=(1280,720);let low=super::size(full,true);let n=1280*720u64;
        assert_eq!(super::target_bytes(full,full,false,false),n*48);
        assert_eq!(super::target_bytes(full,full,true,false),n*80);
        assert_eq!(super::target_bytes(full,low,false,true),n*22);
        assert_eq!(super::target_bytes(full,low,true,true),n*42);
    }
    #[test]
    fn full_ray_reconstruction_keeps_depth_motion_and_foreground_rejection_consistent() {
        use glam::{Mat4,Vec2,Vec3};
        let mut maximum_pixel_error=0.0f32;
        let mut maximum_near_plane_pixel_error=0.0f32;
        for full in [(1277,717),(958,538),(1920,1080)] {
            let low=super::size(full,true);let dimensions=Vec2::new(full.0 as f32,full.1 as f32);
            let low_dimensions=Vec2::new(low.0 as f32,low.1 as f32);
            for view in 0..4 {
                let camera=Vec3::new(17.0+view as f32,4.0,30.0);
                let vp=Mat4::perspective_infinite_reverse_rh(1.2,dimensions.x/dimensions.y,0.1)
                    * Mat4::look_to_rh(camera,Vec3::new(-0.1,-0.15,-1.0),Vec3::Y);
                let inverse=vp.inverse();
                for pixel in [Vec2::new(1.0,1.0),dimensions*0.5,dimensions-Vec2::splat(2.0)] {
                    let pixel=pixel.floor();let uv=(pixel+Vec2::splat(0.5))/dimensions;
                    let ndc=uv*Vec2::new(2.0,-2.0)+Vec2::new(-1.0,1.0);
                    let tap=(uv*low_dimensions-Vec2::splat(0.5)).floor();
                    let tap_ndc=(tap+Vec2::splat(0.5))/low_dimensions*Vec2::new(2.0,-2.0)+Vec2::new(-1.0,1.0);
                    let tap4=inverse*tap_ndc.extend(0.001).extend(1.0);let tap_world=tap4.truncate()/tap4.w;
                    let direction=inverse*ndc.extend(0.0).extend(1.0);
                    let ray=(direction.truncate()-camera*direction.w).normalize();
                    let world=camera+ray*(tap_world-camera).dot(ray);
                    let clip=vp*world.extend(1.0);let projected=clip.truncate()/clip.w;
                    let error_ndc=(projected.truncate()-ndc).length();
                    let error_pixels=((projected.truncate()-ndc)*dimensions*0.5).length();
                    maximum_pixel_error=maximum_pixel_error.max(error_pixels);
                    // Negative control: the original near-plane subtraction
                    // loses subpixel direction precision even at these modest
                    // stage coordinates. Keep the fixture sensitive to it.
                    let near=inverse*ndc.extend(1.0).extend(1.0);
                    let near_ray=(near.truncate()/near.w-camera).normalize();
                    let near_world=camera+near_ray*(tap_world-camera).dot(near_ray);
                    let near_clip=vp*near_world.extend(1.0);
                    maximum_near_plane_pixel_error=maximum_near_plane_pixel_error.max(((near_clip.truncate().truncate()/near_clip.w-ndc)*dimensions*0.5).length());
                    assert!(error_ndc<4e-5,"full={full:?} low={low:?} view={view} pixel={pixel:?} ndc={ndc:?} projected={projected:?} ndc_error={error_ndc:.9} pixel_error={error_pixels:.9} ray={ray:?} world={world:?}");
                    // This is exactly the consumer's full-resolution motion
                    // reconstruction. Copying the low-ray distance is not it.
                    let distance=world.distance(camera);
                    assert!((camera+ray*distance).distance(world)<3e-5);
                    let delta=Vec3::new(0.02,-0.005,0.015);
                    let previous=camera+ray*distance+delta;
                    assert!(previous.distance(world+delta)<3e-5);
                    let front=vp*(camera+ray*(distance*0.8)).extend(1.0);
                    let behind=vp*(camera+ray*(distance*1.2)).extend(1.0);
                    assert!(projected.z<front.z/front.w,"near opaque must reject grass");
                    assert!(projected.z>=behind.z/behind.w,"grass in front of opaque remains");
                }
            }
        }
        eprintln!("half aggregate ray: maximum error={maximum_pixel_error:.9} input pixels; near-plane negative control={maximum_near_plane_pixel_error:.9} input pixels");
        assert!(maximum_near_plane_pixel_error>0.05);
        assert!(maximum_pixel_error<maximum_near_plane_pixel_error*0.01);
    }
    #[test]
    fn reduced_shader_specializations_and_binding_limits_validate() {
        use naga::ShaderStage::{Vertex,Fragment};
        for (source,entries) in [
            (super::super::shader_source(),vec![(Vertex,"meadow_vs"),(Fragment,"meadow_depth_fs"),(Fragment,"meadow_fs"),(Vertex,"meadow_aggregate_vs"),(Fragment,"meadow_aggregate_depth_fs"),(Fragment,"meadow_aggregate_fs"),(Fragment,"meadow_aggregate_depth_motion_fs")]),
            (include_str!("shaders/meadow_merge.wgsl").into(),vec![(Fragment,"fs"),(Fragment,"motion_fs")]),
        ] {
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            for value in [0.0,1.0] {
                let mut constants=naga::back::PipelineConstants::default();
                constants.insert("MEADOW_AGGREGATE_HALF_RES".into(),value);
                for &entry in &entries {
                    naga::back::pipeline_constants::process_overrides(&module,&info,Some(entry),&constants)
                        .unwrap_or_else(|e|panic!("{entry:?} half={value}: {e}"));
                }
            }
            if entries.len()==2 {
                let textures=module.global_variables.iter().filter(|(_,v)|matches!(module.types[v.ty].inner,naga::TypeInner::Image{..})).count();
                assert_eq!(textures,7,"reuse unused binding8, no expanded scene sampling layout");
                let opaque=module.global_variables.iter().find(|(_,v)|v.binding.as_ref().is_some_and(|b|b.group==0 && b.binding==8)).unwrap().1;
                assert!(matches!(module.types[opaque.ty].inner,naga::TypeInner::Image {class:naga::ImageClass::Depth{..},..}));
            }
        }
    }
}
