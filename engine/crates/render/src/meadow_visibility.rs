//! Optional source visibility resolve. Only the first pass submits plant geometry.
//! IDs refer to original recipe triangles, not a proxy or a second vertex stream.
use super::{buffer, layout, shader, storage, texture, uniform, Gpu, Targets, IndexBuffers, DRAW_STRIDE};
use std::collections::HashSet;

pub(super) struct Renderer {
    raster: wgpu::RenderPipeline,
    resolve: [wgpu::RenderPipeline; 2],
    layout: wgpu::BindGroupLayout,
}
pub(super) struct Target {
    pub size: (u32, u32),
    ids: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    hidden: Option<wgpu::Buffer>,
}
impl Target {
    pub fn bytes(&self) -> u64 { u64::from(self.size.0) * u64::from(self.size.1) * 8 + 16 }
    pub fn hidden_bytes(&self)->u64 {self.hidden.as_ref().map_or(0,|b|b.size())}
    pub fn hidden_buffer(&self)->Option<wgpu::Buffer> {self.hidden.clone()}
    pub fn copy_hidden(&self,enc:&mut wgpu::CommandEncoder) {
        let Some(buffer)=&self.hidden else {return;};
        enc.clear_buffer(buffer,0,Some(4));
        enc.copy_texture_to_buffer(self.ids.as_image_copy(),wgpu::TexelCopyBufferInfo {
            buffer,layout:wgpu::TexelCopyBufferLayout{offset:256,bytes_per_row:Some(hidden_row_bytes(self.size.0)),rows_per_image:Some(self.size.1)},
        },wgpu::Extent3d{width:self.size.0,height:self.size.1,depth_or_array_layers:1});
    }
}
fn hidden_row_bytes(width:u32)->u32 {(width*8).div_ceil(256)*256}
fn hidden_bytes(size:(u32,u32))->u64 {256+u64::from(hidden_row_bytes(size.0))*u64::from(size.1)}
pub(super) fn validate_hidden_size(gpu:&Gpu,size:(u32,u32))->Result<(),String> {
    let bytes=hidden_bytes(size);
    if bytes>u64::from(gpu.limits.max_storage_buffer_binding_size) || bytes>gpu.limits.max_buffer_size {
        return Err(format!("hidden meadow ID copy needs {bytes} bytes, exceeds the device buffer budget"));
    }
    Ok(())
}
pub(crate) fn hidden_fallback(gpu:&Gpu)->wgpu::Buffer {
    buffer(gpu,"disabled hidden meadow",&[0;264],wgpu::BufferUsages::STORAGE)
}
#[derive(Default, Debug)]
pub(super) struct Coverage {
    pub pixels: u64,
    pub primitives: u64,
    pub roots: u64,
    pub invalid: u64,
    pub skipped: u64,
}
fn source() -> String {
    format!("enable primitive_index;\n{}\n{}\n{}\n{}", crate::wind::shade_source(),
        include_str!("shaders/meadow_common.wgsl"), include_str!("shaders/meadow_render.wgsl"),
        include_str!("shaders/meadow_visibility.wgsl"))
}
impl Renderer {
    pub fn new(gpu: &Gpu, scene: &wgpu::BindGroupLayout, view: &wgpu::BindGroupLayout,
        draw: &wgpu::BindGroupLayout) -> Result<Self, String> {
        if !gpu.device.features().contains(wgpu::Features::PRIMITIVE_INDEX) {
            return Err("KERN_MEADOW_VISIBILITY requires GPU primitive-index support".into());
        }
        let layout = layout(gpu, "meadow visibility resolve", &[
            (0, uniform()), (1, storage(true)), (2, storage(true)), (3, uniform()),
            (4, texture(wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2)),
        ], wgpu::ShaderStages::VERTEX_FRAGMENT);
        // The raster layout deliberately does not contain the ID texture:
        // binding an attachment for sampling even when unused is a usage hazard.
        let raster_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meadow visibility write"),
            bind_group_layouts: &[Some(scene), Some(view), Some(draw)], immediate_size: 0,
        });
        let resolve_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meadow visibility resolve"),
            bind_group_layouts: &[Some(scene), Some(view), Some(&layout)], immediate_size: 0,
        });
        let module = shader(gpu, "meadow source visibility", source());
        let make = |raster: bool, motion: bool| {
            let mut targets = vec![Some(wgpu::ColorTargetState {
                format: if raster {wgpu::TextureFormat::Rg32Uint} else {wgpu::TextureFormat::Rgba16Float},
                blend: None, write_mask: wgpu::ColorWrites::ALL,
            })];
            if motion { targets.push(Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba32Float,
                blend:None,write_mask:wgpu::ColorWrites::ALL})); }
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if raster {"meadow visibility"} else {"meadow visibility illumination"}),
                layout: Some(if raster {&raster_layout} else {&resolve_layout}),
                vertex: wgpu::VertexState {module:&module, entry_point:Some(if raster {"meadow_visibility_vs"} else {"meadow_visibility_resolve_vs"}),
                    buffers:&[],compilation_options:Default::default()},
                fragment: Some(wgpu::FragmentState {module:&module,entry_point:Some(if raster {"meadow_visibility_fs"}
                    else if motion {"meadow_visibility_resolve_motion_fs"} else {"meadow_visibility_resolve_fs"}),
                    targets:&targets,compilation_options:Default::default()}),
                primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},
                depth_stencil: raster.then_some(wgpu::DepthStencilState {
                    format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),
                    depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
                multisample:Default::default(),multiview_mask:None,cache:gpu.pipeline_cache(),
            })
        };
        Ok(Self {raster:make(true,false),resolve:[make(false,false),make(false,true)],layout})
    }
    pub fn target(&self, gpu:&Gpu, size:(u32,u32), settings:&wgpu::Buffer, pool:&wgpu::Buffer,
        recipe:&wgpu::Buffer, capacity:u32, skip_hidden:bool) -> Target {
        let ids=gpu.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("meadow source primitive IDs"),size:wgpu::Extent3d{width:size.0,height:size.1,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rg32Uint,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_SRC,
            view_formats:&[],
        });
        let view=ids.create_view(&Default::default());
        let draw=buffer(gpu,"meadow visibility capacity",bytemuck::cast_slice(&[0u32,capacity,0,0]),wgpu::BufferUsages::UNIFORM);
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:Some("meadow source visibility resolve"),layout:&self.layout,entries:&[
                wgpu::BindGroupEntry{binding:0,resource:settings.as_entire_binding()},
                wgpu::BindGroupEntry{binding:1,resource:pool.as_entire_binding()},
                wgpu::BindGroupEntry{binding:2,resource:recipe.as_entire_binding()},
                wgpu::BindGroupEntry{binding:3,resource:draw.as_entire_binding()},
                wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(&view)},
            ],
        });
        let hidden=skip_hidden.then(|| {
            let result=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("certified meadow IDs and opaque skip count"),
                size:hidden_bytes(size),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            gpu.queue.write_buffer(&result,0,bytemuck::cast_slice(&[0u32,size.0,size.1,hidden_row_bytes(size.0)/8]));result
        });
        Target{size,ids,view,bind,hidden}
    }
    pub fn render(&self, enc:&mut wgpu::CommandEncoder, scene:&wgpu::BindGroup, view:&wgpu::BindGroup,
        draws:&[wgpu::BindGroup;3], args:&wgpu::Buffer, t:&Targets, target:&Target, indices:Option<&IndexBuffers>, motion:bool,
        times:[Option<(&wgpu::QuerySet,u32)>;2], raster:bool, resolve:bool) {
        let timestamps=|slot:usize|times[slot].map(|(query_set,i)|wgpu::RenderPassTimestampWrites {
            query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)});
        if raster {
            let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("meadow visibility"),color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&target.view,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                    view:&t.depth_view,depth_ops:Some(wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}),stencil_ops:None}),
                timestamp_writes:timestamps(0).map(|mut t| {if target.hidden.is_some() {t.end_of_pass_write_index=None;} t}),occlusion_query_set:None,multiview_mask:None,
            });
            pass.set_pipeline(&self.raster);pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,view,&[]);
            for lod in 0..3 {
                pass.set_bind_group(2,&draws[lod],&[]);
                if let Some(indices)=indices {
                    pass.set_index_buffer(indices.depth[lod].slice(..),wgpu::IndexFormat::Uint16);
                    pass.draw_indexed_indirect(args,lod as u64*DRAW_STRIDE);
                } else {pass.draw_indirect(args,lod as u64*DRAW_STRIDE);}
            }
        }
        if resolve {
            let mut attachments=vec![Some(wgpu::RenderPassColorAttachment {view:&t.color,depth_slice:None,resolve_target:None,
                ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})];
            if motion {attachments.push(Some(wgpu::RenderPassColorAttachment {view:t.motion.as_ref().unwrap(),depth_slice:None,
                resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}}));}
            let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("meadow visibility illumination"),
                color_attachments:&attachments,depth_stencil_attachment:None,timestamp_writes:timestamps(1),
                occlusion_query_set:None,multiview_mask:None});
            pass.set_pipeline(&self.resolve[usize::from(motion)]);pass.set_bind_group(0,scene,&[]);
            pass.set_bind_group(1,view,&[]);pass.set_bind_group(2,&target.bind,&[]);pass.draw(0..3,0..1);
        }
    }
}
fn coverage(words:impl Iterator<Item=[u32;2]>, capacity:u32, vertices:[u32;3])->Coverage {
    let mut result=Coverage::default();let mut roots=HashSet::new();let mut primitives=HashSet::new();
    for [owner,primitive] in words {
        let owner=owner&0x7fff_ffff;
        if owner==0 {continue;}
        let lod=(primitive>>30) as usize;let triangle=primitive&0x3fff_ffff;
        if owner-1>=capacity || lod>=3 || triangle>=vertices[lod]/3 {result.invalid+=1;continue;}
        result.pixels+=1;roots.insert(owner-1);primitives.insert((owner-1,primitive));
    }
    result.roots=roots.len() as u64;result.primitives=primitives.len() as u64;result
}
pub(super) fn read_coverage(gpu:&Gpu,target:&Target,capacity:u32,vertices:[u32;3])->Result<Coverage,String> {
    let row=(target.size.0*8).div_ceil(256)*256;
    let image_bytes=u64::from(row)*u64::from(target.size.1);
    let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("meadow visibility diagnostic"),size:image_bytes+4,
        usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(target.ids.as_image_copy(),wgpu::TexelCopyBufferInfo {buffer:&read,
        layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(row),rows_per_image:Some(target.size.1)}},
        wgpu::Extent3d{width:target.size.0,height:target.size.1,depth_or_array_layers:1});
    if let Some(hidden)=&target.hidden {encoder.copy_buffer_to_buffer(hidden,0,&read,image_bytes,4);}
    gpu.queue.submit([encoder.finish()]);read.slice(..).map_async(wgpu::MapMode::Read, |_|{});gpu.wait();
    let data=read.slice(..).get_mapped_range().map_err(|e|format!("meadow visibility readback: {e}"))?;
    let mut result=coverage(data[..image_bytes as usize].chunks_exact(row as usize).flat_map(|line|line[..target.size.0 as usize*8].chunks_exact(8))
        .map(|p|[u32::from_le_bytes(p[..4].try_into().unwrap()),u32::from_le_bytes(p[4..].try_into().unwrap())]),capacity,vertices);
    result.skipped=u64::from(u32::from_le_bytes(data[image_bytes as usize..image_bytes as usize+4].try_into().unwrap()));
    drop(data);read.unmap();Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visibility_ids_count_real_primitives_and_reject_invalid_indices() {
        let c=coverage([[0,u32::MAX],[1,0],[1,0],[1,1<<30],[2,2<<30],[4,0],[1,3<<30],[1,99]].into_iter(),3,[3,3,3]);
        assert_eq!((c.pixels,c.primitives,c.roots,c.invalid),(4,3,2,3));
        let tagged=coverage([[0x8000_0001,0],[1,0],[0x8000_0002,2<<30]].into_iter(),3,[3,3,3]);
        assert_eq!((tagged.pixels,tagged.primitives,tagged.roots,tagged.invalid),(3,2,2,0));
    }
    #[test]
    fn hidden_id_copy_budget_includes_alignment_and_header() {
        assert_eq!(hidden_bytes((1920,1080)),1920*1080*8+256);
        assert_eq!(hidden_bytes((639,359)),5120*359+256);
        assert_eq!(hidden_row_bytes(1),256);
    }
    #[test]
    fn certified_nearest_source_stays_before_aggregate_blend_on_same_pixel_ray() {
        use glam::{Mat4,Vec3,Vec4,Vec3Swizzles};
        // Same reverse-Z projection and inverse reconstruction as raster/merge.
        // A closer aggregate sample cannot move a certified source ray beyond
        // the blend start. Use real ranges plus camera cuts/translated worlds.
        for camera in [Vec3::new(0.,3.,7.),Vec3::new(41.,14.,-385.),Vec3::new(-1350.,37.,690.)] {
            let view=Mat4::look_at_rh(camera,camera+Vec3::new(0.3,-0.15,-1.),Vec3::Y);
            let proj=Mat4::perspective_infinite_reverse_rh(50f32.to_radians(),16./9.,0.1);
            let vp=proj*view;let inv=vp.inverse();
            for horizontal in [0.1f32,15.,30.,59.,59.9] {
                for yaw in [-0.35f32,0.,0.35] {
                    let ray=Vec3::new(yaw,-0.15,-1.).normalize();
                    let source=camera+ray*(horizontal/ray.xz().length());
                    let clip=vp*source.extend(1.);let ndc=clip/clip.w;
                    let reconstruct=|depth:f32| {let p=inv*Vec4::new(ndc.x,ndc.y,depth,1.);p.truncate()/p.w};
                    let distance=|p:Vec3|(p-camera).xz().length();
                    assert!(distance(reconstruct(ndc.z))<=60.);
                    for nearer in [1f32,1.001,1.25,2.,4.] {
                        assert!(distance(reconstruct(ndc.z*nearer))<=60.,"closer aggregate changed blend ownership");
                    }
                }
            }
        }
        // An opaque source erases every intervening premultiplied layer's
        // background contribution; any partial source must preserve it.
        for alpha in [0f32,0.2,0.7,1.] {
            let before=3.+(1.-alpha)*9.;let skipped=3.;
            assert_eq!(before==skipped,alpha==1.);
        }
        let source=include_str!("shaders/meadow_visibility.wgsl");
        assert!(source.contains("in.opacity==1.0"));
        assert!(source.contains("horizontal<=meadow.shape.w"));
    }
    #[test]
    fn homogeneous_weights_preserve_perspective_attributes_and_clipped_vertices() {
        use glam::{Vec2,Vec3,Vec4};
        for clip in [
            [Vec4::new(-1.,-1.,0.2,1.),Vec4::new(2.,-1.,0.4,2.),Vec4::new(0.,3.,0.8,4.)],
            [Vec4::new(-1.,-1.,-0.5,-0.2),Vec4::new(2.,-1.,0.4,2.),Vec4::new(0.,3.,0.8,4.)],
        ] {
            let expected=Vec3::new(0.2,0.3,0.5);let point=clip[0]*expected.x+clip[1]*expected.y+clip[2]*expected.z;
            let ndc=Vec2::new(point.x,point.y)/point.w;
            let p=clip.map(|v|Vec2::new(v.x,v.y)-ndc*v.w);
            let weights=Vec3::new(p[1].perp_dot(p[2]),p[2].perp_dot(p[0]),p[0].perp_dot(p[1]));
            let actual=weights/weights.element_sum();assert!((actual-expected).abs().max_element()<1e-6);
        }
    }
    #[test]
    fn visibility_shader_validates_with_shared_lighting_and_motion() {
        let source=source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
        let shade=module.entry_points.iter().position(|e|e.name=="shade").unwrap();
        let ep=info.get_entry_point(shade);let mut buffers=0;let mut textures=0;
        for (handle,global) in module.global_variables.iter() {
            if ep[handle].is_empty() {continue;}
            if matches!(global.space,naga::AddressSpace::Storage{..}) {buffers+=1;}
            if matches!(module.types[global.ty].inner,naga::TypeInner::Image{class:naga::ImageClass::Sampled{..}|naga::ImageClass::Depth{..},..}) {textures+=1;}
        }
        assert!(buffers<=16,"hidden shade exceeds storage binding budget: {buffers}");
        assert!(textures<=16,"hidden shade exceeds texture binding budget: {textures}");
        for name in ["meadow_visibility_vs","meadow_visibility_fs","meadow_visibility_resolve_fs","meadow_visibility_resolve_motion_fs"] {
            assert!(module.entry_points.iter().any(|e|e.name==name));
        }
        for (index,entry) in module.entry_points.iter().enumerate() {
            if !entry.name.starts_with("meadow_visibility_") {continue;}
            let ep=info.get_entry_point(index);let mut storage=0;
            for (handle,global) in module.global_variables.iter() {
                if ep[handle].is_empty() {continue;}
                if matches!(global.space,naga::AddressSpace::Storage {..}) {storage+=1;}
                let Some(binding)=&global.binding else {continue;};
                if binding.group==1 {assert!([0,2,5,6,7,10,11,12,13,14,15].contains(&binding.binding));}
                if binding.group==2 {
                    assert!(binding.binding<=4);
                    if !entry.name.contains("resolve") {assert!(binding.binding<4,"ID attachment cannot be sampled by its write pass");}
                }
            }
            assert!(storage<=16,"{} needs {storage} storage buffers",entry.name);
        }
    }
}
