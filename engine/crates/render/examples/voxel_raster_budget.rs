//! Isolate the production voxel raster shader; no selection, generation or shading.
//! Synthetic projected supports and overlap. Timings are NOT whole-renderer FPS.
use render::Gpu;
use wgpu::util::DeviceExt;
use serde_json::json;
fn main()->Result<(),String>{
 let out=std::env::args().nth(1).unwrap_or("build/voxel-raster-probe".into());std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
 let gpu=Gpu::new()?;
 if !gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY){return Err("GPU timestamps required".into());}
 let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("production voxel raster probe"),source:wgpu::ShaderSource::Wgsl(include_str!("../src/shaders/voxel_splat_raster.wgsl").into())});
 let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {label:None,layout:None,
  vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},primitive:Default::default(),
  depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Greater),stencil:Default::default(),bias:Default::default()}),multisample:Default::default(),
  fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::R32Uint,blend:None,write_mask:wgpu::ColorWrites::ALL}),Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::R32Uint,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:None});
 let (w,h)=(1920,1080);
 let tex=|format|gpu.device.create_texture(&wgpu::TextureDescriptor {label:None,size:wgpu::Extent3d {width:w,height:h,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
 let vis=tex(wgpu::TextureFormat::R32Uint);let ids=tex(wgpu::TextureFormat::R32Uint);let depth=tex(wgpu::TextureFormat::Depth32Float);
 let vv=vis.create_view(&Default::default());let iv=ids.create_view(&Default::default());let dv=depth.create_view(&Default::default());
 let queries=gpu.device.create_query_set(&wgpu::QuerySetDescriptor {label:None,ty:wgpu::QueryType::Timestamp,count:2});
 let resolved=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:256,usage:wgpu::BufferUsages::QUERY_RESOLVE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
 let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:16,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
 let mut results=Vec::new();
 for repeat in 0..3 {for diameter in [0.5f32,1.,2.,4.] {for count in [65536u32,262144,1048576,4194304] {
  // Header + one tree transform + packed source cells, exactly the production ABI.
  let cell=diameter/1.376;let mut data=vec![[0u32;4];16+count as usize];
  let vp=glam::Mat4::from_cols(glam::Vec4::new(2./w as f32,0.,0.,0.),glam::Vec4::new(0.,2./h as f32,0.,0.),glam::Vec4::new(0.,0.,0.001,0.),glam::Vec4::new(-0.9,-0.9,0.5,1.));
  for (i,col) in vp.to_cols_array_2d().iter().enumerate(){data[i]=col.map(f32::to_bits);}
  data[11][0]=1;data[12]=[1.,0.,0.,0.].map(f32::to_bits);data[13]=[0.,1.,0.,0.].map(f32::to_bits);data[14]=[0.,0.,1.,0.].map(f32::to_bits);data[15]=[0.,0.,0.,cell].map(f32::to_bits);
  for i in 0..count {let c=i%1048576;data[16+i as usize]=[0,i,(c%1024)|((c/1024)<<10),65535];}
  let buffer=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytemuck::cast_slice(&data),usage:wgpu::BufferUsages::STORAGE});
  let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[wgpu::BindGroupEntry {binding:0,resource:buffer.as_entire_binding()}]});
  let mut samples=Vec::new();
  for frame in 0..15 {
   let mut enc=gpu.device.create_command_encoder(&Default::default());
   {let attachments=[Some(wgpu::RenderPassColorAttachment {view:&vv,resolve_target:None,depth_slice:None,ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}}),Some(wgpu::RenderPassColorAttachment {view:&iv,resolve_target:None,depth_slice:None,ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})];
    let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:None,color_attachments:&attachments,depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&dv,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(0.),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:Some(wgpu::RenderPassTimestampWrites {query_set:&queries,beginning_of_pass_write_index:Some(0),end_of_pass_write_index:Some(1)}),occlusion_query_set:None,multiview_mask:None});
    pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.draw(0..6,0..count);
   }
   enc.resolve_query_set(&queries,0..2,&resolved,0);enc.copy_buffer_to_buffer(&resolved,0,&read,0,16);gpu.queue.submit([enc.finish()]);
   read.slice(..).map_async(wgpu::MapMode::Read,|r|r.unwrap());gpu.wait();let bytes=read.slice(..).get_mapped_range().map_err(|e|e.to_string())?;let stamps:&[u64]=bytemuck::cast_slice(&bytes);let ms=(stamps[1]-stamps[0])as f64*gpu.queue.get_timestamp_period()as f64/1e6;drop(bytes);read.unmap();if frame>=3 {samples.push(ms);}
  }
  samples.sort_by(f64::total_cmp);
  let raw=gpu.read_rgba8(&ids);let visible=raw.chunks_exact(4).filter(|b|u32::from_le_bytes((*b).try_into().unwrap())!=0).count();if visible==0{return Err("empty raster result".into());}
  let vis_raw=gpu.read_rgba8(&vis);let z_raw=gpu.read_rgba8(&depth);let expected_z=0.5+0.0005*cell;
  for ((id,v),z) in raw.chunks_exact(4).zip(vis_raw.chunks_exact(4)).zip(z_raw.chunks_exact(4)) {
   let id=u32::from_le_bytes(id.try_into().unwrap());let v=u32::from_le_bytes(v.try_into().unwrap());let z=f32::from_le_bytes(z.try_into().unwrap());
   if id==0 {assert_eq!(v,0);assert_eq!(z,0.);}else{assert!(id<=count);assert_eq!(v,0x80000000|id);assert!((z-expected_z).abs()<1e-6);}
  }
  let result=json!({"repeat":repeat,"nominal_diameter_px":diameter,"submitted_splats":count,"submitted_triangles":count as u64*2,"submitted_vertices":count as u64*6,"repeated_layers":count.div_ceil(1048576),"visible_pixels":visible,"median_gpu_ms":samples[6],"samples_ms":samples});println!("{result}");results.push(result);
  if repeat==0 && count==1048576 {let rgba=raw.chunks_exact(4).flat_map(|b|{let id=u32::from_le_bytes(b.try_into().unwrap());if id==0 {[15,15,15]}else{[80+(id%150)as u8,140,60]}}).collect::<Vec<_>>();image::save_buffer_with_format(format!("{out}/support-{diameter}.jpg"),&rgba,w,h,image::ColorType::Rgb8,image::ImageFormat::Jpeg).map_err(|e|e.to_string())?;}
 }}}
 std::fs::write(format!("{out}/results.json"),serde_json::to_vec_pretty(&json!({"gpu":gpu.info.name,"viewport":[w,h],"scope":"production vs/fs_main raster only, including target clears; excludes splat generation, selection, shading, shadows and reconstruction; synthetic overlap, not forest quality or game FPS","rows":results})).unwrap()).map_err(|e|e.to_string())?;Ok(())
}
