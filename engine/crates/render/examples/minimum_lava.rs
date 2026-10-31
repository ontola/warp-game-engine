//! Visual regression and isolated full-resolution stereo cost for minimum lava.
//! minimum_lava OUT_DIRECTORY [--flat] [--bench] [--covered] [--paired]
#[cfg(target_os="android")]
#[link(name="c++_shared")]
unsafe extern "C" {}
use glam::{Affine3A,Quat,Vec3};
use render::{Gpu,Scene,Renderer,Lighting,Camera,ViewRect};
fn main()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();let out=std::path::Path::new(args.get(1).ok_or("minimum_lava OUT_DIRECTORY [--flat] [--bench] [--covered] [--paired]")?);
    std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
    let covered=args.iter().any(|s|s=="--covered");
    let flat=args.iter().any(|s|s=="--flat");let bench=args.iter().any(|s|s=="--bench");
    let gpu=Gpu::new()?;let mut scene=Scene::default();scene.graphics.minimal_forward=true;
    let lava=scene.add_material(render::scene::Material{tint:[1.,0.92,0.],layer:-1,roughness:1.,flags:if flat{render::scene::EMISSIVE}else{render::scene::LAVA},scale:45000.,metal:0.});
    let stone=scene.add_material(render::scene::Material{tint:[0.14,0.16,0.19],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
    let lava_mesh=scene.add_mesh(&render::primitives::cube(lava).mesh());
    scene.add_instance(lava_mesh,Affine3A::from_scale_rotation_translation(Vec3::new(900.,0.2,900.),Quat::IDENTITY,Vec3::new(0.,-0.1,0.)));
    let stone_mesh=scene.add_mesh(&render::primitives::cube(stone).mesh());
    for (x,z,h) in [(-3.,-2.,1.),(-1.,-2.,1.),(1.,-2.,1.7),(3.,-2.,1.),(-3.,0.,0.9),(3.,0.,1.2)] {
        scene.add_instance(stone_mesh,Affine3A::from_scale_rotation_translation(Vec3::new(1.94,h,1.94),Quat::IDENTITY,Vec3::new(x,h*0.5,z)));
    }
    if covered {scene.add_instance(stone_mesh,Affine3A::from_scale_rotation_translation(Vec3::new(900.,0.5,900.),Quat::IDENTITY,Vec3::new(0.,0.25,0.)));}
    let mut light=Lighting{ev100:12.,sun_lux:12000.,sky_lux:2500.,time:0.,..Default::default()};
    let renderer=Renderer::new(&gpu,&scene,&light);
    let (w,h)=if bench{(1440,1584)}else{(960,640)};
    let targets:[wgpu::Texture;2]=std::array::from_fn(|_|gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("lava direct eye"),size:wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]}));
    let views=[-0.032,0.032].map(|ipd|ViewRect{cam:Camera{pos:Vec3::new(7.+ipd,8.,11.),target:Vec3::new(ipd,0.,0.),fov_y:1.0,near:0.05},x:0,y:0,w,h});
    let draw=|light:&Lighting|renderer.try_render_eye_targets(&gpu,&views,light,&[&targets[0],&targets[1]]);
    draw(&light)?;let before=gpu.read_rgba8(&targets[0]);
    let unorm=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("lava unorm parity"),size:targets[0].size(),mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
    renderer.try_render_eye_targets(&gpu,&views[..1],&light,&[&unorm])?;
    let ordinary=gpu.read_rgba8(&unorm);
    assert!(ordinary.iter().zip(&before).all(|(a,b)|a.abs_diff(*b)<=2),"lava differs between ordinary output and direct sRGB eyes");
    render::write_png(&out.join(if flat{"flat.png"}else{"lava.png"}),w,h,&before)?;
    light.time=3.;draw(&light)?;let after=gpu.read_rgba8(&targets[0]);
    if !flat&&!covered {
        let changed=before.chunks_exact(4).zip(after.chunks_exact(4)).filter(|(a,b)|a[..3].iter().zip(*b).any(|(a,b)|a.abs_diff(*b)>10)).count();
        assert!(changed>(w*h/50) as usize,"lava did not animate: {changed}");
        let hot=before.chunks_exact(4).filter(|p|p[0]>210&&p[1]>100&&p[2]<180).count();
        let crust=before.chunks_exact(4).filter(|p|p[0]<100&&p[1]<100).count();
        assert!(hot>500&&crust>500,"missing glowing cracks or dark crust: {hot}/{crust}");
        render::write_png(&out.join("lava-3s.png"),w,h,&after)?;
        println!("PASS: hot cracks {hot}, dark pixels {crust}, animated pixels {changed}");
    }
    if args.iter().any(|s|s=="--paired") {
        renderer.set_async_gpu_profiling(&gpu,true)?;
        let material=scene.materials[lava as usize];let mut samples=String::from("cycle,slot,detail,cpu_ms,geometry_gpu_ms,fx_gpu_ms,sprites_gpu_ms\n");
        light.time=3.;
        // One renderer, same mesh order/pose/time/targets. ABBA balances linear
        // clock drift; upload a material in both arms, outside the wall timer.
        for cycle in 0..74 {for (slot,detail) in [false,true,true,false].into_iter().enumerate() {
            let mut m=material;m.flags=if detail{render::scene::LAVA}else{render::scene::EMISSIVE};
            renderer.update_materials(&gpu,lava as usize,&[m])?;
            let start=std::time::Instant::now();draw(&light)?;gpu.wait();let cpu=start.elapsed().as_secs_f64()*1000.;
            let drain=renderer.drain_gpu_profiles(&gpu);
            if drain.errors>0||drain.samples.len()!=1 {return Err(format!("missing GPU sample: errors {}, samples {}",drain.errors,drain.samples.len()));}
            let sample=&drain.samples[0];let ms=|name|sample.gpu_ms.iter().find(|v|v.0==name).map_or(0.,|v|v.1);
            if cycle>=10 {samples.push_str(&format!("{},{},{},{:.6},{:.6},{:.6},{:.6}\n",cycle-10,slot,detail,cpu,ms("minimum_geometry"),ms("minimum_fx"),ms("minimum_sprites")));}
        }}
        std::fs::write(out.join("paired.csv"),samples).map_err(|e|e.to_string())?;
        println!("PAIRED: 64 ABBA cycles after 40 warmup frames, {w}x{h} per eye on {}; covered={covered}; timestamps include diagnostic overhead, no compositor",gpu.info.name);
        return Ok(());
    }
    if bench {
        for i in 0..20 {light.time=i as f32/120.;draw(&light)?;gpu.wait();}
        let mut elapsed=Vec::new();
        for i in 0..120 {light.time=i as f32/120.;let start=std::time::Instant::now();draw(&light)?;gpu.wait();elapsed.push(start.elapsed().as_secs_f64()*1000.);}
        elapsed.sort_by(f64::total_cmp);
        println!("{} on {}: {}x{} per eye, 120 serial stereo frames; median {:.3} ms, p95 {:.3} ms; isolated renderer, no OpenXR compositor",if flat{if covered{"flat covered"}else{"flat"}}else{if covered{"lava covered"}else{"lava"}},gpu.info.name,w,h,elapsed[60],elapsed[114]);
    }
    Ok(())
}
