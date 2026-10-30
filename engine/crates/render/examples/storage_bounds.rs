//! Headless driver probe: report runtime array lengths and boundary reads.
use wgpu::util::DeviceExt;
#[cfg(target_os="android")]
#[link(name="c++_shared")]
unsafe extern "C" {}
fn main() {
    let gpu=render::Gpu::new().unwrap();
    let mut data=vec![0u32;80];
    for i in 0..4 {data[i*20+17]=i as u32+1;}
    let input=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("four 80-byte records"),contents:bytemuck::cast_slice(&data),usage:wgpu::BufferUsages::STORAGE});
    let output=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:32,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:32,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:None,source:wgpu::ShaderSource::Wgsl(r#"
struct Inst {r0:vec4<f32>,r1:vec4<f32>,r2:vec4<f32>,sphere:vec4<f32>,first:u32,count:u32,scale:f32,flags:u32}
@group(0) @binding(0) var<storage,read> instances:array<Inst>;
@group(0) @binding(1) var<storage,read> words:array<u32>;
@group(0) @binding(2) var<storage,read_write> output:array<u32,8>;
@compute @workgroup_size(1) fn main() {
 output[0]=arrayLength(&instances);output[1]=arrayLength(&words);
 output[2]=instances[0].count;output[3]=instances[1].count;
 output[4]=instances[2].count;output[5]=instances[3].count;
 output[6]=words[77];output[7]=12345u;
}"#.into())});
    let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:None,layout:None,module:&shader,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
    let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
        wgpu::BindGroupEntry {binding:0,resource:input.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:input.as_entire_binding()},wgpu::BindGroupEntry {binding:2,resource:output.as_entire_binding()}]});
    let mut enc=gpu.device.create_command_encoder(&Default::default());
    {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);}
    enc.copy_buffer_to_buffer(&output,0,&read,0,32);gpu.queue.submit([enc.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
    let bytes=read.slice(..).get_mapped_range().unwrap();let result:&[u32]=bytemuck::cast_slice(&bytes);
    println!("GPU {}: expected [4,80,1,2,3,4,4,12345], got {result:?}",gpu.info.name);
}
