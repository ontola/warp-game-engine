//! Small shared exhaust pool, composited after temporal reconstruction and
//! before tone mapping. Heat refracts a snapshot of the real HDR background;
//! both the plume and its displaced sample are clipped against opaque depth.
use crate::{Camera,Gpu,Lighting};
use bytemuck::{Pod,Zeroable};
use glam::{Mat4,Vec3};
use std::sync::Mutex;

pub const MAX_SPRITES:usize=128;
#[repr(C)]
#[derive(Clone,Copy,Debug,PartialEq,Pod,Zeroable)]
pub struct Sprite {
    pub position_radius:[f32;4],
    pub axis_length:[f32;4],
    /// Smoke: linear reflectance; flame: emitted HDR radiance; heat: unused RGB.
    pub colour_alpha:[f32;4],
    /// Kind (0 smoke, 1 heat, 2 flame), deterministic phase, age seconds, heat pixels.
    pub params:[f32;4],
}
impl Sprite {
    fn valid(&self)->bool {
        self.position_radius.iter().chain(&self.axis_length).chain(&self.colour_alpha).chain(&self.params).all(|v|v.is_finite())
            && self.position_radius[3]>0.0 && self.position_radius[3]<=2.0
            && self.axis_length[3]>=0.0 && self.axis_length[3]<=4.0
            && (0.0..=1.0).contains(&self.colour_alpha[3])
            && matches!(self.params[0],0.0|1.0|2.0) && (0.0..=6.0).contains(&self.params[3])
            && (self.params[0]<0.5||(self.axis_length[3]>0.0&&Vec3::from_slice(&self.axis_length[..3]).length_squared()>1e-8))
    }
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {
    output_vp:[[f32;4];4],depth_vp:[[f32;4];4],
    camera_near:[f32;4],right:[f32;4],up:[f32;4],size:[f32;4],ambient:[f32;4],
}
struct Target {size:(u32,u32),output:wgpu::Texture,uniform:wgpu::Buffer,order:wgpu::Buffer}
pub struct ExhaustRenderer {
    pipeline:wgpu::RenderPipeline,layout:wgpu::BindGroupLayout,sampler:wgpu::Sampler,
    sprites:wgpu::Buffer,source:Mutex<Vec<Sprite>>,targets:Mutex<Vec<Option<Target>>>,
}
pub fn shader_source()->&'static str {include_str!("shaders/exhaust.wgsl")}
impl ExhaustRenderer {
    pub fn new(gpu:&Gpu)->Self {
        let entries=(0..6).map(|binding|wgpu::BindGroupLayoutEntry {binding,
            visibility:wgpu::ShaderStages::VERTEX_FRAGMENT, count:None,
            ty:match binding {
                0=>wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},
                1|2=>wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:true},has_dynamic_offset:false,min_binding_size:None},
                3=>wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},
                4=>wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},
                _=>wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            }}).collect::<Vec<_>>();
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("exhaust"),entries:&entries});
        let pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("exhaust"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("exhaust"),source:wgpu::ShaderSource::Wgsl(shader_source().into())});
        let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {label:Some("exhaust heat and gas"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("exhaust_vs"),compilation_options:Default::default(),buffers:&[]},
            primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},depth_stencil:None,multisample:Default::default(),
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("exhaust_fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache: gpu.pipeline_cache()});
        let sprites=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("shared exhaust sprites"),size:(MAX_SPRITES*std::mem::size_of::<Sprite>())as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let sampler=gpu.device.create_sampler(&wgpu::SamplerDescriptor {label:Some("exhaust background"),mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,..Default::default()});
        Self {pipeline,layout,sampler,sprites,source:Mutex::new(Vec::new()),targets:Mutex::new(Vec::new())}
    }
    pub fn set(&self,gpu:&Gpu,sprites:&[Sprite])->Result<(),String> {
        if sprites.len()>MAX_SPRITES||sprites.iter().any(|s|!s.valid()){return Err("invalid or over-budget exhaust sprites".into());}
        let mut source=self.source.lock().unwrap_or_else(|e|e.into_inner());
        if *source==sprites{return Ok(());}
        source.clear();source.extend_from_slice(sprites);
        if !sprites.is_empty(){gpu.queue.write_buffer(&self.sprites,0,bytemuck::cast_slice(sprites));}Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,index:usize,target:&wgpu::Texture,depth:&wgpu::TextureView,
        camera:Camera,output_vp:Mat4,depth_vp:Mat4,light:&Lighting,timestamps:Option<wgpu::RenderPassTimestampWrites<'_>>)->Result<Option<wgpu::Texture>,String> {
        let source=self.source.lock().unwrap_or_else(|e|e.into_inner());
        if source.is_empty(){return Ok(None);}
        if index>=4{return Err("exhaust supports at most four views".into());}
        let mut order:Vec<_>=source.iter().enumerate().filter_map(|(i,s)| {
            let p=Vec3::from_slice(&s.position_radius[..3]);let r=s.position_radius[3]+s.axis_length[3];
            let clip=output_vp*p.extend(1.0);
            let margin=r*output_vp.y_axis.truncate().length().max(output_vp.x_axis.truncate().length());
            (p.distance(camera.pos)<120.0+r&&clip.w+r>camera.near&&clip.x.abs()<clip.w+margin&&clip.y.abs()<clip.w+margin)
                .then_some((i as u32,p.distance_squared(camera.pos)))
        }).collect();
        if order.is_empty(){return Ok(None);}
        order.sort_by(|a,b|b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let ids:Vec<_>=order.iter().map(|o|o.0).collect();drop(source);
        if target.format()!=wgpu::TextureFormat::Rgba16Float||!target.usage().contains(wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::TEXTURE_BINDING){return Err("exhaust requires copyable sampleable HDR input".into());}
        let size=(target.width(),target.height());
        let mut targets=self.targets.lock().unwrap_or_else(|e|e.into_inner());if targets.len()<=index {targets.resize_with(index+1,||None);}
        if targets[index].as_ref().is_none_or(|t|t.size!=size) {
            let output=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("exhaust HDR output (not temporal history)"),size:target.size(),mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:target.format(),usage:wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[]});
            let buffer=|label,size,usage|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage,mapped_at_creation:false});
            targets[index]=Some(Target {size,output,uniform:buffer("exhaust view",std::mem::size_of::<Uniform>() as u64,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST),order:buffer("exhaust per-view order",(MAX_SPRITES*4)as u64,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST)});
        }
        let t=targets[index].as_ref().unwrap();let forward=(camera.target-camera.pos).normalize_or(Vec3::NEG_Z);
        let right=forward.cross(Vec3::Y).normalize_or(Vec3::X);let up=right.cross(forward).normalize_or(Vec3::Y);
        // Low-opacity smoke uses shared sky irradiance, not an unshadowed sun
        // highlight. Directional volumetric self-shadowing is a later system.
        let ambient=Vec3::new(0.78,0.86,1.0)*light.sky_lux.max(0.0)/std::f32::consts::PI;
        let uniform=Uniform {output_vp:output_vp.to_cols_array_2d(),depth_vp:depth_vp.to_cols_array_2d(),camera_near:camera.pos.extend(camera.near).to_array(),right:right.extend(0.0).to_array(),up:up.extend(0.0).to_array(),size:[size.0 as f32,size.1 as f32,1.0/size.0 as f32,1.0/size.1 as f32],ambient:ambient.extend(0.0).to_array()};
        gpu.queue.write_buffer(&t.uniform,0,bytemuck::bytes_of(&uniform));gpu.queue.write_buffer(&t.order,0,bytemuck::cast_slice(&ids));
        // The input may be TAA/DLSS history: never write effects back into it.
        enc.copy_texture_to_texture(target.as_image_copy(),t.output.as_image_copy(),target.size());
        let background=target.create_view(&Default::default());
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("exhaust view"),layout:&self.layout,entries:&[
            wgpu::BindGroupEntry {binding:0,resource:t.uniform.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:self.sprites.as_entire_binding()},wgpu::BindGroupEntry {binding:2,resource:t.order.as_entire_binding()},
            wgpu::BindGroupEntry {binding:3,resource:wgpu::BindingResource::TextureView(&background)},wgpu::BindGroupEntry {binding:4,resource:wgpu::BindingResource::TextureView(depth)},wgpu::BindGroupEntry {binding:5,resource:wgpu::BindingResource::Sampler(&self.sampler)}]});
        let output=t.output.create_view(&Default::default());
        let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("exhaust"),color_attachments:&[Some(wgpu::RenderPassColorAttachment {view:&output,depth_slice:None,resolve_target:None,ops:wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:timestamps,occlusion_query_set:None,multiview_mask:None});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&bind,&[]);pass.draw(0..6,0..ids.len() as u32);drop(pass);Ok(Some(t.output.clone()))
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn shader_and_shared_buffer_layout_validate(){assert_eq!(std::mem::size_of::<Sprite>(),64);assert_eq!(std::mem::size_of::<Uniform>(),208);let module=naga::front::wgsl::parse_str(shader_source()).unwrap();naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();}
    #[test] fn rejects_bad_pool_data(){let mut s=Sprite::zeroed();assert!(!s.valid());s.position_radius[3]=0.1;s.colour_alpha[3]=0.1;assert!(s.valid());s.params[3]=f32::NAN;assert!(!s.valid());}
}
