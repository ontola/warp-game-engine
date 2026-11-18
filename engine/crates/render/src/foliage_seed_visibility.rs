//! Conservative root visibility from CURRENT-frame seed depth, before refinement.
//! No historical depth, CPU selection, or shadow rejection. Every camera contributes.
use crate::{Gpu,foliage_select::Selector};
use glam::Mat4;

pub(crate) struct CameraDepth<'a> {pub projection:Mat4,pub pyramid:&'a wgpu::TextureView}
pub(crate) struct Visibility {pipeline:wgpu::ComputePipeline,layout:wgpu::BindGroupLayout,uniform:wgpu::Buffer,stride:u64,binds:std::sync::Mutex<crate::frame::resource_cache::BindGroups<32>>}
impl Visibility {
 pub fn new(gpu:&Gpu)->Self {
  let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("foliage seed visibility"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_seed_visibility.wgsl").into())});
  let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("foliage seed visibility"),layout:None,module:&shader,entry_point:Some("classify"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
  let layout=pipeline.get_bind_group_layout(0);
  let stride=u64::from(gpu.device.limits().min_uniform_buffer_offset_alignment.max(256));
  let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage seed cameras"),size:32*stride,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
  Self {pipeline,layout,uniform,stride,binds:Default::default()}
 }
 pub fn encode(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,s:&Selector,cameras:&[CameraDepth<'_>])->Result<(),String> {
  if cameras.is_empty() || cameras.len()>32 || cameras.iter().any(|c|!c.projection.is_finite()) {return Err("invalid foliage current-depth cameras".into());}
  enc.clear_buffer(&s.seed_visibility,0,None);
  for (index,camera) in cameras.iter().enumerate() {
   // Separate aligned camera ranges prevent later views replacing an earlier
   // view's matrix. Reuse buffers/bind groups across frames and target resizes.
   let mut words=[0u32;20];words[..16].copy_from_slice(&camera.projection.to_cols_array().map(f32::to_bits));words[16]=s.count;
   let offset=index as u64*self.stride;gpu.queue.write_buffer(&self.uniform,offset,bytemuck::cast_slice(&words));
   let entries=[wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer:&self.uniform,offset,size:std::num::NonZeroU64::new(80)})},wgpu::BindGroupEntry {binding:1,resource:s._nodes.binding()},
    wgpu::BindGroupEntry {binding:2,resource:s._instances.binding()},wgpu::BindGroupEntry {binding:3,resource:wgpu::BindingResource::TextureView(camera.pyramid)},
    wgpu::BindGroupEntry {binding:4,resource:s.seed_visibility.as_entire_binding()}];
   let bind=self.binds.lock().unwrap().entries(gpu,&self.layout,"foliage seed camera",&entries);
   let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(s.count.div_ceil(64),1,1);
  }
  Ok(())
 }
}
