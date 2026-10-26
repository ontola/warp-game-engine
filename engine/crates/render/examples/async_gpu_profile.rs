//! Bounded exclusive-GPU acceptance for timestamp plumbing, not an FPS test.
//! This fixture waits explicitly only to finish assertions; the profiler does not.
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};

fn main() { if let Err(e)=run() { eprintln!("{e}"); std::process::exit(1); } }
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=std::path::PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists() { return Err("fresh output directory required".into()); }
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    if !gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY) { return Err("timestamp queries unsupported; fixture not run".into()); }
    let mut scene=Scene::default();
    scene.sky=Some((16,8,vec![[1.,1.,1.,1.];128]));
    let material=scene.add_material(Material {tint:[0.5;3],layer:-1,roughness:1.,flags:TWO_SIDED,scale:1.,metal:0.});
    let vertices:Vec<_>=[[-2.,-2.,-4.],[2.,-2.,-4.],[2.,2.,-4.],[-2.,2.,-4.]].into_iter()
        .zip([[0.,0.],[1.,0.],[1.,1.],[0.,1.]]).map(|(pos,uv)|Vertex{pos,normal:[0.,0.,1.],uv}).collect();
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,0,2,3],material}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);
    let camera=Camera{pos:Vec3::ZERO,target:Vec3::NEG_Z,fov_y:55f32.to_radians(),near:0.05};
    let light=Lighting{gi:0.,near_shadows:0.,taa:0.,fog_density:0.,..Default::default()};
    let renderer=Renderer::new(&gpu,&scene,&light);
    let single=[ViewRect{cam:camera,x:0,y:0,w:128,h:128}];
    let initial=renderer.drain_gpu_profiles(&gpu);
    assert!(!initial.enabled && initial.pending==0 && initial.samples.is_empty());
    // Five submissions without any drain must occupy exactly four slots and
    // drop one sample, even if the GPU has already finished in the meantime.
    for id in 10..15 {
        renderer.set_async_gpu_profiling(&gpu,true)?;
        renderer.set_gpu_profile_frame_id(id);
        renderer.try_render_texture(&gpu,&single,&light,128,128)?;
    }
    gpu.wait();
    let first=renderer.drain_gpu_profiles(&gpu);
    assert_eq!(first.samples.iter().map(|s|s.frame_id).collect::<Vec<_>>(),vec![10,11,12,13]);
    assert_eq!((first.pending,first.dropped,first.errors),(0,1,0));
    for sample in &first.samples {
        assert_eq!((sample.output_size,sample.views),([128,128],1));
        assert!(sample.gpu_ms.iter().any(|(name,_)|*name=="shade"));
        assert!(sample.covered_interval_ms>0. && sample.instrumented_gpu_ms>0.);
    }
    // A sample in flight survives disabling, then a new stage/resize can use
    // the same API without reallocation or losing its previous drop count.
    renderer.set_gpu_profile_frame_id(20);
    renderer.try_render_texture(&gpu,&single,&light,128,128)?;
    renderer.set_async_gpu_profiling(&gpu,false)?;
    gpu.wait();
    let disabled=renderer.drain_gpu_profiles(&gpu);
    assert!(!disabled.enabled && disabled.samples.len()==1 && disabled.samples[0].frame_id==20);
    renderer.set_async_gpu_profiling(&gpu,true)?;
    let four:Vec<_>=(0..4).map(|i|ViewRect{cam:camera,x:i%2*128,y:i/2*128,w:128,h:128}).collect();
    renderer.set_gpu_profile_frame_id(30);
    // Both readers consume the same actual query results, so differences here
    // would indicate bookkeeping/resolve errors rather than timing noise.
    let (_,sync)=renderer.try_render_texture_profiled(&gpu,&four,&light,256,256)?;
    let asynchronous=renderer.drain_gpu_profiles(&gpu);
    assert_eq!((asynchronous.samples.len(),asynchronous.pending,asynchronous.dropped,asynchronous.errors),(1,0,1,0));
    let sample=&asynchronous.samples[0];
    assert_eq!((sample.frame_id,sample.views,sample.output_size),(30,4,[256,256]));
    assert_eq!(sync.gpu_ms.len(),sample.gpu_ms.len());
    for (name,ms) in &sync.gpu_ms {
        let measured=sample.gpu_ms.iter().find(|(n,_)|n==name).ok_or("missing asynchronous pass")?.1;
        assert!((measured-ms).abs()<1e-9,"same-frame mismatch for {name}: {measured} vs {ms}");
    }
    let passes=sample.gpu_ms.iter().map(|(name,ms)|format!("[\"{name}\",{ms}]")).collect::<Vec<_>>().join(",");
    let report=format!("{{\"passed\":true,\"scope\":\"native timestamp plumbing; not FPS or DLSS validation\",\"ring_capacity\":4,\"ring_dropped\":{},\"errors\":{},\"four_view_frame_id\":{},\"instrumented_gpu_ms\":{},\"covered_interval_ms\":{},\"gpu_ms\":[{}]}}",
        asynchronous.dropped,asynchronous.errors,sample.frame_id,sample.instrumented_gpu_ms,sample.covered_interval_ms,passes);
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    println!("async GPU profile PASS: full ring, preserved pending, 4-view same-frame parity");
    Ok(())
}
