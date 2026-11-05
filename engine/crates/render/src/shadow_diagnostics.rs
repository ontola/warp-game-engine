//! Explicit capture-only accounting of submitted solar/headlight geometry.
//! No normal gameplay readback; camera visibility never certifies casters.
use crate::{cull_groups, gpu_profile, Gpu};

pub(crate) struct Pass {
    pub name: &'static str,
    pub view_index: Option<usize>,
    pub counter: wgpu::Buffer,
    pub visible: wgpu::Buffer,
    pub capacity: u32,
    pub grouped: bool,
}
#[derive(Default)]
pub(crate) struct State {
    pub context: Option<gpu_profile::Context>,
    pub passes: Vec<Pass>,
}
pub struct Diagnostic {
    pub frame_id: u64,
    pub render_sequence: u64,
    pub name: &'static str,
    pub view_index: Option<usize>,
    pub groups: cull_groups::Diagnostics,
    pub selected_triangles: u32,
    pub submitted_vertices: u32,
    pub pairs: Vec<[u32;2]>,
}

fn read(gpu:&Gpu, source:&wgpu::Buffer, bytes:u64)->Result<Vec<u8>,String> {
    if bytes==0 {return Ok(Vec::new());}
    if bytes>source.size(){return Err("solar diagnostic read exceeds source buffer".into());}
    let target=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("solar capture readback"),size:bytes,
        usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source,0,&target,0,bytes);gpu.queue.submit([encoder.finish()]);
    target.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
    let data=target.slice(..).get_mapped_range().map_err(|e|e.to_string())?.to_vec();
    target.unmap();Ok(data)
}

pub(crate) fn capture(gpu:&Gpu, groups:&cull_groups::Resources, state:&State,
    expected_sequence:Option<u64>)->Result<Vec<Diagnostic>,String> {
    let context=state.context.ok_or("no submitted solar diagnostic frame")?;
    if expected_sequence.is_some_and(|s|s!=context.render_sequence) {return Err("stale solar diagnostic request".into());}
    let mut result=Vec::with_capacity(state.passes.len());
    for pass in &state.passes {
        let bytes=read(gpu,&pass.counter,crate::occlusion::COUNTER_BYTES)?;
        let words:Vec<u32>=bytes.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect();
        if words[0]>pass.capacity {return Err("solar diagnostic visible capacity exceeded".into());}
        let work=groups.diagnostics_for(&words,pass.grouped)?;
        let selected=read(gpu,&pass.visible,u64::from(words[0])*8)?;
        let pairs=selected.chunks_exact(8).map(|b|[
            u32::from_le_bytes(b[..4].try_into().unwrap()),u32::from_le_bytes(b[4..].try_into().unwrap())]).collect();
        result.push(Diagnostic {frame_id:context.frame_id,render_sequence:context.render_sequence,
            name:pass.name,view_index:pass.view_index,groups:work,selected_triangles:words[1],submitted_vertices:words[2],pairs});
    }
    Ok(result)
}
