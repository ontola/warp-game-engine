//! Per-view render-resolution motion for temporal reconstruction.
//!
//! Vectors point from the current unjittered position to the previous one in
//! render pixels (X right, Y down). Rigid instance/wheel/branch-primitive motion
//! and affine wind are reconstructed from the actual visible depth. The live
//! primitive transform already includes its branch pose and attachment rest;
//! no duplicate per-leaf pose evaluation or extra pose binding is required.
//! GPU snapshots are advanced
//! once after ALL views; a recycled grass slot never inherits its old owner's
//! history. Partial layers use a dominant surface plus a bias-current mask.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

#[derive(Clone, Copy)]
pub struct MotionView {
    /// Stable camera/player identity, not its mutable couch order.
    pub key: u64,
    pub size: (u32, u32),
    pub inverse_jittered_view_proj: Mat4,
    pub view_proj: Mat4,
    pub camera: Vec3,
    pub split: bool,
    /// Same source-to-volume handoff as the color renderer.
    pub grass_detail: crate::vegetation_detail::DistanceProfile,
    pub reset: bool,
}
pub struct MotionInputs<'a> {
    pub opaque_depth: &'a wgpu::TextureView,
    pub visibility: &'a wgpu::TextureView,
    pub visible: &'a wgpu::Buffer,
    /// RG32Uint: (owner+1)<<8 | coverage8, bitcast<u32>(reversed depth).
    pub grass: Option<&'a wgpu::TextureView>,
    pub transparency: Option<&'a wgpu::TextureView>,
    /// Existing half-resolution premultiplied colour/alpha and HORIZONTAL
    /// optical-centroid distance from the native far-grass compositor.
    pub far_grass: Option<(&'a wgpu::TextureView, &'a wgpu::TextureView)>,
    /// Full-resolution RGBA16Float premultiplied voxel color and RGBA32Float
    /// opacity-weighted reprojection: mean(previous_world-current_world).xyz,
    /// current ray distance.w. This centroid is not an opaque surface hit.
    pub voxel: Option<(&'a wgpu::TextureView, &'a wgpu::TextureView)>,
    /// Full-resolution procedural meadow, composited after near/far grass and
    /// before glass. Same color/reprojection contract as the voxel layer.
    pub meadow: Option<(&'a wgpu::TextureView, &'a wgpu::TextureView)>,
}
#[derive(Clone)]
pub struct MotionOutput {
    pub vectors: wgpu::Texture,
    /// R32Float, reversed-Z, matching the selected dominant motion surface.
    pub depth: wgpu::Texture,
    /// R32Float [0,1]. The temporal consumer MUST bind this for invalid slots
    /// and partial-volume ambiguity; zero motion alone does not reject history.
    pub bias: wgpu::Texture,
    pub reset: bool,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    inverse_current: [[f32;4];4],
    current: [[f32;4];4],
    previous: [[f32;4];4],
    camera: [f32;4],
    size: [f32;4],
    flags: [u32;4],
    grass: [f32;4],
}
struct ViewHistory {
    key: u64,
    size: (u32,u32),
    previous: Mat4,
    camera: Vec3,
    direction: Vec3,
    frame: u64,
    outputs: MotionOutput,
}
struct History {
    frame: u64,
    snapshot_valid: bool,
    views: Vec<Option<ViewHistory>>,
}
pub struct MotionRenderer {
    pipeline: wgpu::RenderPipeline,
    current_instances: wgpu::Buffer,
    current_wind: wgpu::Buffer,
    previous_instances: wgpu::Buffer,
    previous_wind: wgpu::Buffer,
    instance_bytes: u64,
    wind_bytes: u64,
    empty_metadata: wgpu::TextureView,
    empty_color: wgpu::TextureView,
    empty_distance: wgpu::TextureView,
    empty_reprojection: wgpu::TextureView,
    history: Mutex<History>,
}

fn texture(gpu:&Gpu,label:&str,size:(u32,u32),format:wgpu::TextureFormat)->wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label:Some(label),size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,
        usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::COPY_DST,
        view_formats:&[],
    })
}
fn empty(gpu:&Gpu,format:wgpu::TextureFormat,bytes:usize)->wgpu::TextureView {
    let tex=texture(gpu,"empty motion metadata",(1,1),format);
    gpu.queue.write_texture(tex.as_image_copy(),&[0;16][..bytes],wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:None,rows_per_image:None},wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1});
    tex.create_view(&Default::default())
}
impl MotionRenderer {
    pub fn new(gpu:&Gpu,instances:&wgpu::Buffer,wind:&wgpu::Buffer,instance_count:usize)->Result<Self,String> {
        if instance_count >= 0x00ff_ffff {return Err("motion owner metadata supports fewer than 16777215 instances".into());}
        let instance_bytes=instance_count as u64*80;
        let wind_bytes=instance_count as u64*16;
        if instances.size()<instance_bytes || wind.size()<wind_bytes
            || !instances.usage().contains(wgpu::BufferUsages::COPY_SRC)
            || !wind.usage().contains(wgpu::BufferUsages::COPY_SRC) {
            return Err("motion requires complete COPY_SRC instance and wind buffers".into());
        }
        let previous=|label,size:u64|gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label:Some(label),size:size.max(80),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false,
        });
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("surface motion"),source:wgpu::ShaderSource::Wgsl(shader_source().into())});
        let entries=(0..16).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::FRAGMENT,count:None,
            ty:match binding {
                0=>wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},
                1..=5=>wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:true},has_dynamic_offset:false,min_binding_size:None},
                _=>wgpu::BindingType::Texture {sample_type:match binding {
                    6|8|9=>wgpu::TextureSampleType::Uint,7=>wgpu::TextureSampleType::Depth,
                    _=>wgpu::TextureSampleType::Float {filterable:false}},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},
            }}).collect::<Vec<_>>();
        let bind_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("motion typed inputs"),entries:&entries});
        let layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("motion typed inputs"),bind_group_layouts:&[Some(&bind_layout)],immediate_size:0});
        // RG16Float is a guaranteed render target, but not a portable storage
        // texture. One fullscreen MRT pass avoids an extra conversion image.
        let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("surface motion"),layout:Some(&layout),
            vertex:wgpu::VertexState {module:&module,entry_point:Some("motion_vs"),compilation_options:Default::default(),buffers:&[]},
            primitive:Default::default(),depth_stencil:None,multisample:Default::default(),
            fragment:Some(wgpu::FragmentState {module:&module,entry_point:Some("motion_fs"),compilation_options:Default::default(),
                targets:&[wgpu::TextureFormat::Rg16Float,wgpu::TextureFormat::R32Float,wgpu::TextureFormat::R32Float].map(|format|Some(wgpu::ColorTargetState {format,blend:None,write_mask:wgpu::ColorWrites::ALL}))}),
            multiview_mask:None,cache: gpu.pipeline_cache(),
        });
        Ok(Self {pipeline,current_instances:instances.clone(),current_wind:wind.clone(),
            previous_instances:previous("previous motion instances",instance_bytes),previous_wind:previous("previous plant wind",wind_bytes),
            instance_bytes,wind_bytes,empty_metadata:empty(gpu,wgpu::TextureFormat::Rg32Uint,8),
            empty_color:empty(gpu,wgpu::TextureFormat::Rgba16Float,8),empty_distance:empty(gpu,wgpu::TextureFormat::R32Float,4),
            empty_reprojection:empty(gpu,wgpu::TextureFormat::Rgba32Float,16),
            history:Mutex::new(History {frame:0,snapshot_valid:false,views:Vec::new()})})
    }

    pub fn reset(&self) {
        let mut history=self.history.lock().unwrap();
        history.snapshot_valid=false;
        history.views.clear();
    }

    /// Read-only transforms from the previous rendered frame, including empty
    /// branch proxies and all source primitive attachments. Consumers must
    /// reject history on reset/first frame; the buffer is initialized only by
    /// `finish_frame`, after every view has consumed the old snapshot.
    pub(crate) fn previous_instances(&self)->&wgpu::Buffer {&self.previous_instances}
    pub(crate) fn snapshot_valid(&self)->bool {self.history.lock().unwrap().snapshot_valid}

    pub fn render(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,index:usize,view:MotionView,
        inputs:MotionInputs<'_>,timestamp:Option<(&wgpu::QuerySet,u32)>)->Result<MotionOutput,String> {
        if index>=4 || view.size.0==0 || view.size.1==0
            || view.size.0>gpu.limits.max_texture_dimension_2d || view.size.1>gpu.limits.max_texture_dimension_2d
            || !view.inverse_jittered_view_proj.is_finite() || !view.view_proj.is_finite() || !view.camera.is_finite()
            || view.inverse_jittered_view_proj.determinant().abs()<1e-20 || view.view_proj.determinant().abs()<1e-20 {
            return Err("motion view has invalid index, extent, camera or singular matrices".into());
        }
        let exact=|v:&wgpu::TextureView,format|v.texture().width()==view.size.0
            && v.texture().height()==view.size.1 && v.texture().format()==format;
        if !exact(inputs.opaque_depth,wgpu::TextureFormat::Depth32Float)
            || !exact(inputs.visibility,wgpu::TextureFormat::R32Uint)
            || inputs.grass.is_some_and(|v|!exact(v,wgpu::TextureFormat::Rg32Uint))
            || inputs.transparency.is_some_and(|v|!exact(v,wgpu::TextureFormat::Rg32Uint))
            || inputs.visible.size()<8 {
            return Err("motion inputs require matching full render extents and typed depth/owner formats".into());
        }
        if let Some((color,distance))=inputs.far_grass {
            if color.texture().size()!=distance.texture().size()
                || color.texture().width()!=view.size.0.div_ceil(2) || color.texture().height()!=view.size.1.div_ceil(2)
                || color.texture().format()!=wgpu::TextureFormat::Rgba16Float || distance.texture().format()!=wgpu::TextureFormat::R32Float {
                return Err("motion far-grass inputs require matching half-resolution color and distance".into());
            }
        }
        if let Some((color,reprojection))=inputs.voxel {
            if !exact(color,wgpu::TextureFormat::Rgba16Float)
                || !exact(reprojection,wgpu::TextureFormat::Rgba32Float) {
                return Err("motion voxel inputs require matching full-resolution RGBA16Float color and RGBA32Float reprojection".into());
            }
        }
        if let Some((color,reprojection))=inputs.meadow {
            if !exact(color,wgpu::TextureFormat::Rgba16Float)
                || !exact(reprojection,wgpu::TextureFormat::Rgba32Float) {
                return Err("motion meadow inputs require matching full-resolution RGBA16Float color and RGBA32Float reprojection".into());
            }
        }
        let mut history=self.history.lock().unwrap();
        while history.views.len()<=index {history.views.push(None);}
        let frame=history.frame;
        let near=view.inverse_jittered_view_proj*Vec4::new(0.0,0.0,1.0,1.0);
        if !near.is_finite() || near.w.abs()<1e-10 {return Err("motion view cannot reconstruct its near-plane centre".into());}
        let direction=(near.truncate()/near.w-view.camera).try_normalize().ok_or("motion camera direction is degenerate")?;
        let reset=view.reset || !history.snapshot_valid || history.views[index].as_ref().is_none_or(|h|
            h.key!=view.key || h.size!=view.size || h.frame.checked_add(1)!=Some(frame)
            || h.camera.distance_squared(view.camera)>400.0 || h.direction.dot(direction)<0.5);
        let previous=if reset {view.view_proj}else{history.views[index].as_ref().unwrap().previous};
        if history.views[index].as_ref().is_none_or(|h|h.size!=view.size) {
            let outputs=MotionOutput {vectors:texture(gpu,"DLSS pixel motion",view.size,wgpu::TextureFormat::Rg16Float),
                depth:texture(gpu,"DLSS dominant depth",view.size,wgpu::TextureFormat::R32Float),
                bias:texture(gpu,"DLSS bias current color",view.size,wgpu::TextureFormat::R32Float),reset:true};
            history.views[index]=Some(ViewHistory {key:view.key,size:view.size,previous:view.view_proj,camera:view.camera,direction,frame,outputs});
        }
        let target=history.views[index].as_mut().unwrap();
        target.key=view.key;target.previous=view.view_proj;target.camera=view.camera;target.direction=direction;target.frame=frame;target.outputs.reset=reset;
        let grass_band=view.grass_detail.for_views(if view.split {2}else{1});
        let uniform=Uniform {inverse_current:view.inverse_jittered_view_proj.to_cols_array_2d(),current:view.view_proj.to_cols_array_2d(),
            previous:previous.to_cols_array_2d(),camera:view.camera.extend(u32::from(view.split) as f32).to_array(),
            size:[view.size.0 as f32,view.size.1 as f32,1.0/view.size.0 as f32,1.0/view.size.1 as f32],
            flags:[u32::from(reset),u32::from(inputs.grass.is_some()),u32::from(inputs.transparency.is_some()),u32::from(inputs.far_grass.is_some())],
            grass:[grass_band.fade_start,grass_band.fade_end,u32::from(inputs.voxel.is_some()) as f32,u32::from(inputs.meadow.is_some()) as f32]};
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("motion view"),contents:bytemuck::bytes_of(&uniform),usage:wgpu::BufferUsages::UNIFORM});
        let entries=[
            (0,uniform.as_entire_binding()),(1,self.current_instances.as_entire_binding()),(2,self.previous_instances.as_entire_binding()),
            (3,self.current_wind.as_entire_binding()),(4,self.previous_wind.as_entire_binding()),(5,inputs.visible.as_entire_binding()),
            (6,wgpu::BindingResource::TextureView(inputs.visibility)),(7,wgpu::BindingResource::TextureView(inputs.opaque_depth)),
            (8,wgpu::BindingResource::TextureView(inputs.grass.unwrap_or(&self.empty_metadata))),
            (9,wgpu::BindingResource::TextureView(inputs.transparency.unwrap_or(&self.empty_metadata))),
            (10,wgpu::BindingResource::TextureView(inputs.far_grass.map_or(&self.empty_color,|v|v.0))),
            (11,wgpu::BindingResource::TextureView(inputs.far_grass.map_or(&self.empty_distance,|v|v.1))),
            (12,wgpu::BindingResource::TextureView(inputs.voxel.map_or(&self.empty_color,|v|v.0))),
            (13,wgpu::BindingResource::TextureView(inputs.voxel.map_or(&self.empty_reprojection,|v|v.1))),
            (14,wgpu::BindingResource::TextureView(inputs.meadow.map_or(&self.empty_color,|v|v.0))),
            (15,wgpu::BindingResource::TextureView(inputs.meadow.map_or(&self.empty_reprojection,|v|v.1))),
        ];
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("surface motion"),layout:&self.pipeline.get_bind_group_layout(0),
            entries:&entries.into_iter().map(|(binding,resource)|wgpu::BindGroupEntry {binding,resource}).collect::<Vec<_>>()});
        let views=[&target.outputs.vectors,&target.outputs.depth,&target.outputs.bias].map(|t|t.create_view(&Default::default()));
        let attachments=views.iter().map(|v|Some(wgpu::RenderPassColorAttachment {view:v,depth_slice:None,resolve_target:None,
            ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})).collect::<Vec<_>>();
        let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("surface motion"),color_attachments:&attachments,
            depth_stencil_attachment:None,occlusion_query_set:None,multiview_mask:None,
            timestamp_writes:timestamp.map(|(query_set,start)|wgpu::RenderPassTimestampWrites {query_set,beginning_of_pass_write_index:Some(start),end_of_pass_write_index:Some(start+1)})});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&bind,&[]);pass.draw(0..3,0..1);drop(pass);
        Ok(target.outputs.clone())
    }

    /// Call once after all motion consumers have been encoded. Copies execute
    /// in queue order, so the next frame always sees the previous rendered pose.
    pub fn finish_frame(&self,encoder:&mut wgpu::CommandEncoder) {
        if self.instance_bytes>0 {encoder.copy_buffer_to_buffer(&self.current_instances,0,&self.previous_instances,0,self.instance_bytes);}
        if self.wind_bytes>0 {encoder.copy_buffer_to_buffer(&self.current_wind,0,&self.previous_wind,0,self.wind_bytes);}
        let mut history=self.history.lock().unwrap();
        history.snapshot_valid=true;history.frame=history.frame.wrapping_add(1);
    }
}

pub(crate) fn shader_source()->String {
    format!("{}\n{}",include_str!("shaders/grass_common.wgsl"),concat!(include_str!("shaders/virtual_member.wgsl"), "\n", include_str!("shaders/motion.wgsl")))
}

/// Mathematical diagnostic, independent of jitter and raster sample location.
pub fn pixel_motion(current:Mat4,previous:Mat4,current_point:Vec4,previous_point:Vec4,size:Vec2)->Option<Vec2> {
    let a=current*current_point;let b=previous*previous_point;
    if !a.is_finite() || !b.is_finite() || a.w<=1e-7 || b.w<=1e-7 {return None;}
    Some((b.truncate().truncate()/b.w-a.truncate().truncate()/a.w)*Vec2::new(0.5,-0.5)*size)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn pixel_units_sign_and_translation_free_sky() {
        let p=Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2,1.0,0.1);
        let size=Vec2::splat(100.0);
        let current=Vec3::new(1.0,0.0,-10.0).extend(1.0);let previous=Vec3::new(0.0,0.0,-10.0).extend(1.0);
        assert!((pixel_motion(p,p,current,previous,size).unwrap()-Vec2::new(-5.0,0.0)).length()<1e-5);
        let up=Vec3::new(0.0,1.0,-10.0).extend(1.0);
        assert!((pixel_motion(p,p,up,previous,size).unwrap()-Vec2::new(0.0,5.0)).length()<1e-5);
        let direction=Vec4::new(0.3,0.1,-1.0,0.0);
        let translated=p*Mat4::from_translation(Vec3::new(17.0,-3.0,29.0));
        assert!(pixel_motion(translated,p,direction,direction,size).unwrap().length()<1e-5);
        assert!(pixel_motion(p,p,Vec4::ZERO,previous,size).is_none());
    }
    #[test] fn source_affine_and_wind_round_trip_preserve_local_wheel_point() {
        let xf=glam::Affine3A::from_scale_rotation_translation(Vec3::new(2.0,0.7,1.3),glam::Quat::from_rotation_y(0.8),Vec3::new(3.0,7.0,-8.0));
        let local=Vec3::new(0.2,3.4,-0.7);let shear=Vec2::new(0.14,-0.11);
        let rest=xf.transform_point3(local);let current=rest+Vec3::new(shear.x,0.0,shear.y)*(rest.y-xf.translation.y);
        let unsheared=current-Vec3::new(shear.x,0.0,shear.y)*(current.y-xf.translation.y);
        assert!((xf.inverse().transform_point3(unsheared)-local).length()<1e-5);
        assert_eq!(std::mem::size_of::<Uniform>(),256);
    }
    #[test] fn shared_branch_attachment_round_trip_retains_source_motion_in_four_views() {
        use crate::part_wind::{PartWind,sample_poses};
        use crate::wind::WindState;
        use glam::Quat;
        // Nonuniform assembly scale followed by rotated branches/attachments
        // deliberately produces sheared world matrices. A transpose-only
        // inverse or branch-proxy motion cannot reconstruct these leaf points.
        let parts=[
            PartWind {rest:Mat4::from_scale_rotation_translation(Vec3::new(1.8,0.7,1.3),Quat::from_rotation_y(0.4),Vec3::new(0.4,-1.0,-12.0)),..Default::default()},
            PartWind {parent:Some(0),rest:Mat4::from_rotation_translation(Quat::from_rotation_z(0.3),Vec3::Y),max_angle:0.2,phase:0.4,..Default::default()},
            PartWind {parent:Some(1),rest:Mat4::from_rotation_translation(Quat::from_rotation_x(-0.2),Vec3::new(0.3,0.8,0.0)),max_angle:0.15,harmonic:17,phase:1.1,..Default::default()},
        ];
        let attachment=Mat4::from_scale_rotation_translation(Vec3::new(0.12,0.8,0.2),Quat::from_rotation_y(0.7)*Quat::from_rotation_z(-0.35),Vec3::new(0.2,0.4,-0.1));
        let local=Vec3::new(0.13,0.86,-0.04);
        let mut moved=false;
        // Include a skipped simulation interval and unchanged paused frame:
        // correspondence uses the previous rendered pose, not t-dt.
        for (before,after) in [(1.2,1.25),(1.25,7.4),(7.4,7.4)] {
            let pose=|time_seconds|sample_poses(&parts,&WindState {time_seconds,strength:0.8,..Default::default()}).unwrap()[2]*attachment;
            let old=pose(before);let now=pose(after);
            let world=now.transform_point3(local);let expected=old.transform_point3(local);
            // Shader's cofactor formulation, checked against independently
            // generated previous source positions rather than another inverse.
            let r0=now.row(0).truncate();let r1=now.row(1).truncate();let r2=now.row(2).truncate();
            let relative=world-now.w_axis.truncate();
            let recovered=(r1.cross(r2)*relative.x+r2.cross(r0)*relative.y+r0.cross(r1)*relative.z)/r0.dot(r1.cross(r2));
            let previous=old.transform_point3(recovered);
            assert!(previous.distance(expected)<3e-5);
            assert!(r0.normalize().dot(r1.normalize()).abs()>0.02,"fixture must exercise nonorthogonal basis");
            for view in 0..4 {
                let camera=Vec3::new((view as f32-1.5)*0.25,0.15,0.0);
                let projection=Mat4::perspective_infinite_reverse_rh(1.2,16.0/9.0,0.1);
                let a=projection*Mat4::look_to_rh(camera,Vec3::NEG_Z,Vec3::Y);
                let b=projection*Mat4::look_to_rh(camera-Vec3::new(0.015,0.0,0.0),Vec3::NEG_Z,Vec3::Y);
                let measured=pixel_motion(a,b,world.extend(1.0),previous.extend(1.0),Vec2::new(1920.0,1080.0)).unwrap();
                let oracle=pixel_motion(a,b,world.extend(1.0),expected.extend(1.0),Vec2::new(1920.0,1080.0)).unwrap();
                assert!(measured.distance(oracle)<0.004);
            }
            if before==after {assert!(previous.distance(world)<3e-5);}
            else {moved|=previous.distance(world)>0.005;}
        }
        assert!(moved,"fixture must contain real animated source displacement");
    }
    #[test] fn shader_validates() {
        let source=shader_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let mut bindings=module.global_variables.iter().filter_map(|(_,v)|v.binding.as_ref().map(|b|(b.group,b.binding))).collect::<Vec<_>>();
        bindings.sort_unstable();
        assert_eq!(bindings,(0..16).map(|binding|(0,binding)).collect::<Vec<_>>());
        assert_eq!(module.global_variables.iter().filter(|(_,v)|matches!(v.space,naga::AddressSpace::Storage {..})).count(),5,
            "branch motion must reuse existing instance snapshots, not add per-view pose buffers");
        assert_eq!(module.global_variables.iter().filter(|(_,v)|matches!(module.types[v.ty].inner,naga::TypeInner::Image {..})).count(),10,
            "opaque, grass, voxel and meadow motion inputs must remain below the portable16 sampled-texture budget");
        assert_eq!(std::mem::size_of::<Uniform>(),256);
    }
}
