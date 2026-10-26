//! Bounded particle acceptance. No gameplay/FPS claim; run only with exclusive GPU.
use glam::{Affine3A,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::{Material,TWO_SIDED};
use render::wheel_particles::{Birth,Kind};
use std::path::PathBuf;
const W:u32=512;const H:u32=320;
fn panel(scene:&mut Scene,points:[[f32;3];4],normal:[f32;3],layer:i32)->usize{
    let material=scene.add_material(Material{tint:[0.4;3],layer,roughness:1.,flags:TWO_SIDED,scale:1.,metal:0.});
    let vertices:Vec<_>=points.into_iter().zip([[0.,4.],[4.,4.],[4.,0.],[0.,0.]]).map(|(pos,uv)|Vertex{pos,normal,uv}).collect();
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,0,2,3],material}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);mesh
}
fn difference(a:&[u8],b:&[u8])->usize{a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)|a[..3]!=b[..3]).count()}
fn birth(kind:Kind)->Birth{Birth{position:Vec3::new(0.,-0.4,-3.),velocity:Vec3::new(0.5,3.,0.),normal:Vec3::Y,radius:0.18,lifetime:1.5,wetness:0.,kind,seed:12345678,birth_time:0.}}
fn save(out:&std::path::Path,name:&str,p:&[u8])->Result<(),String>{render::write_png(&out.join(format!("{name}.png")),W,H,p)}
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1)}}
fn run()->Result<(),String>{
    let args:Vec<_>=std::env::args().collect();let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists(){return Err("fresh directory required".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let mut scene=Scene::default();scene.sky=Some((16,8,vec![[1.,1.,1.,1.];128]));
    let checker=image::RgbaImage::from_fn(64,64,|x,y|if(x/4+y/4)%2==0{image::Rgba([150,130,100,255])}else{image::Rgba([45,65,90,255])});
    let layer=scene.add_image("particle background",checker);
    panel(&mut scene,[[-4.,-3.,-5.],[4.,-3.,-5.],[4.,3.,-5.],[-4.,3.,-5.]],[0.,0.,1.],layer);
    let blocker=panel(&mut scene,[[-2.,-2.,-2.],[2.,-2.,-2.],[2.,2.,-2.],[-2.,2.,-2.]],[0.,0.,1.],-1);
    scene.update_instance(1,blocker,Affine3A::from_translation(Vec3::X*1000.));
    // Sun comes from +X: this panel shades the particles without being between
    // them and the camera. Its own pixels are excluded by the particle ROI.
    let shade=panel(&mut scene,[[1.,-2.,-4.],[1.,-2.,-2.],[1.,2.,-2.],[1.,2.,-4.]],[1.,0.,0.],-1);
    scene.update_instance(2,shade,Affine3A::from_translation(Vec3::X*1000.));
    let camera=Camera{pos:Vec3::ZERO,target:Vec3::NEG_Z,fov_y:55f32.to_radians(),near:0.05};
    let light=Lighting{sun_dir:Vec3::X,sun_lux:18000.,sky_lux:600.,ev100:10.,gi:0.,contact_occlusion:0.,reflection_occlusion:0.,shadow_strength:1.,near_shadows:1.,fog_density:0.,fog_start:1000.,fog_end:2000.,taa:0.,sharpen:0.,lod_pixels:0.,..Default::default()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);let mut failures=Vec::new();let mut cases=Vec::new();
    let(off,_)=renderer.render(&gpu,&camera,&light,W,H);save(&out,"off",&off)?;
    renderer.update_wheel_particles(&gpu,0.25,&[])?;let(empty,_)=renderer.render(&gpu,&camera,&light,W,H);
    let empty_difference=difference(&off,&empty);if empty_difference!=0{failures.push(format!("empty births changed {empty_difference} pixels"));}
    let kinds=[("stone",Kind::Stone),("soil",Kind::SoilClod),("grass",Kind::GrassBlade),("mud",Kind::MudClod),("dust",Kind::Dust),("tyre-smoke",Kind::TyreSmoke),("water",Kind::WaterDrop)];
    for(name,kind)in kinds{
        renderer.reset_wheel_particles();renderer.update_wheel_particles(&gpu,0.25,&[birth(kind)])?;
        let(pixels,stats)=renderer.render(&gpu,&camera,&light,W,H);save(&out,name,&pixels)?;let changed=difference(&off,&pixels);
        if changed<10{failures.push(format!("{name} invisible ({changed} pixels)"));}
        let ms=stats.gpu_ms.iter().find(|x|x.0=="wheel particles").map(|x|x.1).unwrap_or(-1.);
        cases.push(format!("\"{name}\":{{\"changed_pixels\":{changed},\"particle_gpu_ms\":{ms}}}"));
    }
    renderer.reset_wheel_particles();let(reset,_)=renderer.render(&gpu,&camera,&light,W,H);let reset_difference=difference(&off,&reset);
    if reset_difference!=0{failures.push(format!("reset left {reset_difference} stale particle pixels"));}
    // Exercise the no-extra-copy path with the real post-temporal exhaust target.
    renderer.set_exhaust(&gpu,&[render::exhaust::Sprite{position_radius:[0.,-0.4,-3.3,0.25],axis_length:[0.,1.,0.,0.6],colour_alpha:[0.4,0.4,0.4,0.7],params:[1.,0.3,0.4,3.]}])?;
    let(heat_only,_)=renderer.render(&gpu,&camera,&light,W,H);
    renderer.update_wheel_particles(&gpu,0.25,&[birth(Kind::Dust)])?;let(combined,_)=renderer.render(&gpu,&camera,&light,W,H);save(&out,"exhaust-and-dust",&combined)?;
    let combined_difference=difference(&heat_only,&combined);if combined_difference<20{failures.push("particles missing when reusing exhaust target".into());}
    renderer.set_exhaust(&gpu,&[])?;renderer.reset_wheel_particles();
    let all:Vec<_>=kinds.iter().enumerate().map(|(i,(_,kind))|Birth{position:Vec3::new((i as f32-3.)*0.42,-0.4,-3.),radius:0.12,..birth(*kind)}).collect();
    renderer.update_wheel_particles(&gpu,0.25,&all)?;let(single,_)=renderer.render(&gpu,&camera,&light,W,H);save(&out,"all-kinds",&single)?;
    let views:Vec<_>=(0..4).map(|i|ViewRect{cam:camera,x:i%2*W,y:i/2*H,w:W,h:H}).collect();
    let(split,_)=renderer.render_views(&gpu,&views,&light,W*2,H*2,&[]);render::write_png(&out.join("four-views.png"),W*2,H*2,&split)?;
    let mut split_difference=0;
    for i in 0..4{let tile:Vec<_>=(0..H).flat_map(|y|{let at=(((y+i/2*H)*W*2+i%2*W)*4)as usize;split[at..at+(W*4)as usize].iter().copied()}).collect();split_difference+=difference(&single,&tile);}
    if split_difference!=0{failures.push(format!("same-camera four-view difference {split_difference}"));}
    scene.update_instance(1,blocker,Affine3A::IDENTITY);renderer.update_instances(&gpu,1,&scene.instances[1..])?;
    renderer.reset_wheel_particles();let(blocked_off,_)=renderer.render(&gpu,&camera,&light,W,H);
    renderer.update_wheel_particles(&gpu,0.25,&all)?;let(blocked_on,_)=renderer.render(&gpu,&camera,&light,W,H);save(&out,"blocked",&blocked_on)?;
    let blocked_difference=difference(&blocked_off,&blocked_on);if blocked_difference!=0{failures.push(format!("opaque blocker leaked {blocked_difference} pixels"));}
    scene.update_instance(1,blocker,Affine3A::from_translation(Vec3::X*1000.));renderer.update_instances(&gpu,1,&scene.instances[1..])?;
    // Compare the same puff's pixels before/after adding a sun-only occluder.
    renderer.reset_wheel_particles();renderer.update_wheel_particles(&gpu,0.25,&[birth(Kind::Dust)])?;
    let(sunlit,_)=renderer.render(&gpu,&camera,&light,W,H);save(&out,"dust-sunlit",&sunlit)?;
    scene.update_instance(2,shade,Affine3A::IDENTITY);renderer.update_instances(&gpu,2,&scene.instances[2..])?;
    let(shadowed,_)=renderer.render(&gpu,&camera,&light,W,H);save(&out,"dust-shadowed",&shadowed)?;
    let(mut sun_sum,mut shade_sum,mut shadow_pixels)=(0u64,0u64,0u64);
    for y in 110..220{for x in 220..315{let at=((y*W+x)*4)as usize;if sunlit[at..at+3]!=off[at..at+3]{sun_sum+=sunlit[at..at+3].iter().map(|v|*v as u64).sum::<u64>();shade_sum+=shadowed[at..at+3].iter().map(|v|*v as u64).sum::<u64>();shadow_pixels+=1;}}}
    if shadow_pixels<20||shade_sum>=sun_sum{failures.push(format!("sun occlusion did not darken dust: pixels={shadow_pixels},sun={sun_sum},shade={shade_sum}"));}
    scene.update_instance(2,shade,Affine3A::from_translation(Vec3::X*1000.));renderer.update_instances(&gpu,2,&scene.instances[2..])?;
    let dark=Lighting{sun_lux:0.,sky_lux:0.,..light.clone()};renderer.reset_wheel_particles();let(night_off,_)=renderer.render(&gpu,&camera,&dark,W,H);
    renderer.update_wheel_particles(&gpu,0.25,&all)?;let(night_on,_)=renderer.render(&gpu,&camera,&dark,W,H);save(&out,"no-light",&night_on)?;
    let mut brightened=0;for(a,b)in night_off.chunks_exact(4).zip(night_on.chunks_exact(4)){if(0..3).any(|i|b[i]>a[i]){brightened+=1;}}
    if brightened!=0{failures.push(format!("unlit ejecta emitted light at {brightened} pixels"));}
    // History parity uses independent renderers; only A receives particles.
    let a=Renderer::new(&gpu,&scene,&light);let b=Renderer::new(&gpu,&scene,&light);
    let use_dlss=args.iter().any(|a|a=="--dlss-quality");
    if use_dlss{a.set_upscale(&gpu,render::upscale::UpscaleMode::DlssQuality)?;b.set_upscale(&gpu,render::upscale::UpscaleMode::DlssQuality)?;}
    let temporal=Lighting{taa:1.,..light.clone()};a.update_wheel_particles(&gpu,0.25,&all)?;
    for _ in 0..4{a.render(&gpu,&camera,&temporal,W,H);b.render(&gpu,&camera,&temporal,W,H);}
    let(temporal_on,_)=a.render(&gpu,&camera,&temporal,W,H);let(temporal_off,_)=b.render(&gpu,&camera,&temporal,W,H);save(&out,if use_dlss{"dlss-on"}else{"taa-on"},&temporal_on)?;
    let temporal_visible=difference(&temporal_on,&temporal_off);if temporal_visible<20{failures.push("particles missing after temporal resolve".into());}
    a.reset_wheel_particles();let(a,_)=a.render(&gpu,&camera,&temporal,W,H);let(b,_)=b.render(&gpu,&camera,&temporal,W,H);
    let history_difference=difference(&a,&b);if history_difference!=0{failures.push(format!("particles contaminated temporal history ({history_difference} pixels)"));}
    let report=format!("{{\"scope\":\"bounded native particle fixture, optional DLSS history parity; not gameplay FPS\",\"cases\":{{{}}},\"empty_difference\":{empty_difference},\"reset_difference\":{reset_difference},\"combined_exhaust_difference\":{combined_difference},\"split_difference\":{split_difference},\"blocked_difference\":{blocked_difference},\"unlit_brightened_pixels\":{brightened},\"sun_shadow_pixels\":{shadow_pixels},\"sun_sum\":{sun_sum},\"shadow_sum\":{shade_sum},\"temporal_mode\":\"{}\",\"temporal_visible_pixels\":{temporal_visible},\"history_difference\":{history_difference},\"passed\":{},\"failures\":{:?}}}",cases.join(","),if use_dlss{"dlss-quality"}else{"taa"},failures.is_empty(),failures);
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;if failures.is_empty(){println!("wheel particle probe PASS");Ok(())}else{Err(failures.join("\n"))}
}
