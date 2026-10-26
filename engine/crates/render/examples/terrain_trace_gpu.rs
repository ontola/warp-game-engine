//! Exclusive-GPU parity proof, never an automatic game restart.
//! terrain_trace_gpu --out FRESH_DIRECTORY
use glam::Vec3;
use render::{Gpu,scene::Heightfield,terrain_trace::{Coverage,Hierarchy,GpuHierarchy,Ray,Status,TraceResult}};
use std::path::PathBuf;

fn field(w:u32,h:u32,f:impl Fn(u32,u32)->f32)->Heightfield{
    Heightfield{origin:[0.,0.],cell:1.,w,h,heights:(0..h).flat_map(|z|(0..w).map(move|x|(x,z))).map(|(x,z)|f(x,z)).collect()}
}
fn ray(origin:Vec3,direction:Vec3,t_max:f32)->Ray{Ray{origin,direction,t_min:0.,t_max}}
fn verify(label:&str,expected:&[TraceResult],actual:&[TraceResult],budget:u32,errors:&mut Vec<String>){
    for (i,(a,b)) in expected.iter().zip(actual).enumerate(){
        if a.status!=b.status||a.reason!=b.reason||b.visited_nodes>budget{
            errors.push(format!("{label}[{i}]: CPU{a:?} GPU{b:?}"));continue;
        }
        if a.status==Status::Hit&&((a.distance-b.distance).abs()>0.003+a.distance.abs()*2e-6||!a.normal.abs_diff_eq(b.normal,0.002)){
            errors.push(format!("{label}[{i}]: hit mismatch CPU{a:?} GPU{b:?}"));
        }
    }
}
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);}}
fn run()->Result<(),String>{
    let args:Vec<_>=std::env::args().collect();let at=args.iter().position(|a|a=="--out").ok_or("--out FRESH_DIRECTORY required")?;
    let out=PathBuf::from(args.get(at+1).ok_or("missing output directory")?);if out.exists(){return Err("refusing to overwrite existing evidence directory".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;let gpu=Gpu::new()?;
    let mut rows=Vec::new();let mut errors=Vec::new();
    let mut hole=field(18,7,|_,_|4.);hole.heights[0]=-1e9;
    let mut invalid=hole.clone();invalid.heights[0]=f32::NAN;
    let cases=[("single-cell-layout",field(2,2,|x,z|x as f32+2.*z as f32),Coverage::UnknownOutside),
        ("flat",field(1025,1025,|_,_|0.),Coverage::UnknownOutside),
        ("distant-ridge",field(2049,3,|x,_|if x==1500{40.}else{0.}),Coverage::UnknownOutside),
        ("valley",field(65,49,|x,z|3.*(x as f32*0.2).sin()+2.*(z as f32*0.31).cos()),Coverage::UnknownOutside),
        ("complete-holes",hole,Coverage::CompleteLoadedTerrain),("unexpected-invalid",invalid,Coverage::CompleteLoadedTerrain)];
    for (label,hf,policy) in cases{
        let before=std::time::Instant::now();let cpu=Hierarchy::with_coverage(&hf,policy)?;let cpu_build_ms=before.elapsed().as_secs_f64()*1000.;
        let hierarchy=GpuHierarchy::new(&gpu,&hf,policy,None)?;
        let mut rays=Vec::new();let mut seed=15u32;let mut random=||{seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);(seed>>8)as f32/(1u32<<24)as f32};
        for _ in 0..256{rays.push(ray(Vec3::new(0.1+(hf.w as f32-1.2)*random(),60.,0.1+(hf.h as f32-1.2)*random()),Vec3::new(random()-0.5,-0.1-random(),random()-0.5),100.));}
        for direction in [Vec3::splat(1e29),Vec3::splat(1e-30),Vec3::ZERO]{rays.push(ray(Vec3::new(0.5,1.,0.5),direction,10.));}
        if label=="distant-ridge"{rays.push(ray(Vec3::new(1.,2.,1.),Vec3::new(1.,0.01,0.),1900.));}
        let single_count=rays.len();let single=rays.clone();for _ in 1..4{rays.extend_from_slice(&single);}
        let upward:Vec<_>=single.iter().map(|r|Ray{origin:r.origin-Vec3::Y*55.,direction:Vec3::new(1.,0.01,0.),t_min:0.,t_max:10000.}).collect();
        let expected:Vec<_>=rays.iter().map(|&r|cpu.trace(r,256)).collect();
        let expected_up:Vec<_>=upward.iter().map(|r|cpu.trace_upward(r.origin,r.direction,r.t_min,256)).collect();
        let expected_budget:Vec<_>=single[..8].iter().map(|&r|cpu.trace(r,0)).collect();
        let timing=gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
        let query=timing.then(||gpu.device.create_query_set(&wgpu::QuerySetDescriptor{label:Some("terrain proof timings"),ty:wgpu::QueryType::Timestamp,count:4}));
        let resolve=timing.then(||gpu.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:32,usage:wgpu::BufferUsages::QUERY_RESOLVE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false}));
        let time_read=timing.then(||gpu.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:32,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}));
        let mut enc=gpu.device.create_command_encoder(&Default::default());if let Some(q)=&query{enc.write_timestamp(q,0);}
        hierarchy.encode_build(&mut enc);if let Some(q)=&query{enc.write_timestamp(q,1);enc.write_timestamp(q,2);}
        let batch=hierarchy.encode_queries(&gpu,&mut enc,&rays,256,false)?;
        let up_batch=hierarchy.encode_queries(&gpu,&mut enc,&upward,256,true)?;
        let budget_batch=hierarchy.encode_queries(&gpu,&mut enc,&single[..8],0,false)?;
        if let Some(q)=&query{enc.write_timestamp(q,3);enc.resolve_query_set(q,0..4,resolve.as_ref().unwrap(),0);enc.copy_buffer_to_buffer(resolve.as_ref().unwrap(),0,time_read.as_ref().unwrap(),0,32);}
        gpu.queue.submit([enc.finish()]);let actual=batch.read(&gpu);let up=up_batch.read(&gpu);let limited=budget_batch.read(&gpu);
        verify(label,&expected,&actual,256,&mut errors);verify(&format!("{label}-upward"),&expected_up,&up,256,&mut errors);verify(&format!("{label}-budget0"),&expected_budget,&limited,0,&mut errors);
        let bytes=hierarchy.read_packed(&gpu);let exact=bytes==cpu.packed_bytes();if !exact{errors.push(format!("{label}: GPU hierarchy differs from CPU min/max bytes"));}
        for view in 1..4{for i in 0..single_count{let a=actual[i];let b=actual[view*single_count+i];if a.status!=b.status||a.distance.to_bits()!=b.distance.to_bits()||a.normal!=b.normal{errors.push(format!("{label}: shared field changes between view batches"));}}}
        let mut times=[0.;2];if let Some(read)=time_read{read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();{let mapped=read.slice(..).get_mapped_range().unwrap();let ticks:&[u64]=bytemuck::cast_slice(&mapped);let scale=gpu.queue.get_timestamp_period()as f64/1e6;times=[(ticks[1]-ticks[0])as f64*scale,(ticks[3]-ticks[2])as f64*scale];}read.unmap();}
        rows.push(format!("{{\"name\":{label:?},\"policy\":{},\"dimensions\":[{},{}],\"hierarchy_bytes\":{},\"gpu_mips_equal_cpu\":{exact},\"cpu_build_ms\":{cpu_build_ms},\"gpu_build_ms\":{},\"gpu_query_batch_ms\":{},\"queries\":{},\"max_nodes\":{},\"hits\":{},\"unknown\":{}}}",policy as u32,hf.w,hf.h,hierarchy.bytes(),times[0],times[1],rays.len()+upward.len()+8,actual.iter().map(|r|r.visited_nodes).max().unwrap_or(0),actual.iter().filter(|r|r.status==Status::Hit).count(),actual.iter().filter(|r|r.status==Status::Unknown).count()));
    }
    let failures=errors.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>().join(",");
    let report=format!("{{\"adapter\":{:?},\"passed\":{},\"scope\":\"GPU hierarchy and queries versus exact triangulated heightfield CPU reference; shared four batches, no gameplay FPS claim\",\"cases\":[{}],\"failures\":[{failures}]}}\n",gpu.info.name,errors.is_empty(),rows.join(","));
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if !errors.is_empty(){return Err(format!("{} terrain GPU parity failures; see report",errors.len()));}println!("terrain hierarchy GPU parity PASS: {}",out.display());Ok(())
}
