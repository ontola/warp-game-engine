//! Same-position and dispersed explosion pressure, with an independent ion warning.
//! minimum_blasts OUT [--bench] [--spam] [--raw] [--inside]
#[cfg(target_os="android")]
#[link(name="c++_shared")]
unsafe extern "C" {}
use render::{Gpu,Scene,Renderer,Camera,ViewRect,Lighting,fx::{Effects,ExplosionBudget,AdditiveColumn}};
use glam::{Vec3,Quat,Affine3A};
fn main()->Result<(),String>{
 let args:Vec<_>=std::env::args().collect();let out=std::path::Path::new(args.get(1).ok_or("minimum_blasts OUT [--bench] [--spam] [--raw] [--inside]")?);std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
 let bench=args.iter().any(|s|s=="--bench");let spam=args.iter().any(|s|s=="--spam");let raw=args.iter().any(|s|s=="--raw");let inside=args.iter().any(|s|s=="--inside");if raw&&spam{return Err("raw spam is deliberately excluded from this bounded GPU probe".into());}
 let gpu=Gpu::new()?;let mut scene=Scene::default();scene.graphics.minimal_forward=true;
 let mat=scene.add_material(render::scene::Material{tint:[0.2;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});let mesh=scene.add_mesh(&render::primitives::cube(mat).mesh());
 scene.add_instance(mesh,Affine3A::from_scale_rotation_translation(Vec3::new(40.,0.5,40.),Quat::IDENTITY,Vec3::new(0.,-0.5,0.)));
 scene.add_instance(mesh,Affine3A::from_scale_rotation_translation(Vec3::new(1.5,3.,1.5),Quat::IDENTITY,Vec3::new(2.,1.,2.)));
 let light=Lighting{ev100:12.,sun_lux:12000.,sky_lux:2500.,..Default::default()};let renderer=Renderer::new(&gpu,&scene,&light);
 let mut fx=if raw{Effects::default()}else{Effects::with_explosion_budget(ExplosionBudget::MINIMUM)};
 for i in 0..if spam{512}else{8} {let pos=Vec3::new((i%4) as f32*3.-4.5,0.7,-(i%2) as f32*3.);fx.explosion(0.,pos,4.,Vec3::new(1.,0.5,0.1));}
 fx.column(AdditiveColumn{position:Vec3::new(6.,-1.,0.),height:10.,radius:[0.4;2],radiance:[50000.;2],color:[Vec3::new(0.05,0.5,1.);2],..Default::default()})?;
 fx.flush(&gpu,&renderer,0.2)?;
 let (w,h)=if bench{(1440,1584)}else{(960,640)};let eye=||gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("blast eye"),size:wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});let targets=[eye(),eye()];
 let views=[-0.032,0.032].map(|ipd|ViewRect{cam:Camera{pos:if inside{Vec3::new(-1.5+ipd,0.8,-3.)}else{Vec3::new(ipd,3.,12.)},target:Vec3::new(ipd,1.,-4.),fov_y:1.,near:0.05},x:0,y:0,w,h});
 let draw=||renderer.try_render_eye_targets(&gpu,&views,&light,&[&targets[0],&targets[1]]);
 draw()?;render::write_png(&out.join("active.png"),w,h,&gpu.read_rgba8(&targets[0]))?;
 println!("Blast population {} ({:?}), raw={raw}, spam={spam}, inside={inside}",fx.len(),fx.explosion_stats());
 if bench {renderer.set_async_gpu_profiling(&gpu,true)?;let mut csv=String::from("frame,cpu_ms,geometry_gpu_ms,fx_gpu_ms\n");
 for i in 0..80 {let start=std::time::Instant::now();draw()?;gpu.wait();let cpu=start.elapsed().as_secs_f64()*1000.;let drained=renderer.drain_gpu_profiles(&gpu);if drained.samples.len()!=1{return Err("missing GPU sample".into());}let sample=&drained.samples[0];let ms=|key|sample.gpu_ms.iter().find(|v|v.0==key).map_or(0.,|v|v.1);
 if i>=16 {csv.push_str(&format!("{},{},{},{}\n",i-16,cpu,ms("minimum_geometry"),ms("minimum_fx")));}}
 std::fs::write(out.join("timings.csv"),csv).map_err(|e|e.to_string())?;
 }
 fx.flush(&gpu,&renderer,10.)?;draw()?;render::write_png(&out.join("expired.png"),w,h,&gpu.read_rgba8(&targets[0]))?;Ok(())
}
