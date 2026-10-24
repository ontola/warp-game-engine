//! Bounded native GPU acceptance for exhaust, not gameplay/FPS evidence.
use glam::{Affine3A,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::{Material,TWO_SIDED};
use render::exhaust::Sprite;
use std::path::PathBuf;
const W:u32=512;const H:u32=320;
fn quad(scene:&mut Scene,z:f32,size:f32,layer:i32)->usize {
    let material=scene.add_material(Material {tint:[0.5;3],layer,roughness:1.0,flags:TWO_SIDED,scale:1.0,metal:0.0});
    let vertices:Vec<_>=[[-size,-size,z],[size,-size,z],[size,size,z],[-size,size,z]].into_iter()
        .zip([[0.,4.],[4.,4.],[4.,0.],[0.,0.]]).map(|(pos,uv)|Vertex {pos,normal:[0.,0.,1.],uv}).collect();
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&[0,1,2,0,2,3],material}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);mesh
}
fn difference(a:&[u8],b:&[u8])->usize {a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)|a[..3]!=b[..3]).count()}
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1)}}
fn run()->Result<(),String>{
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists(){return Err("fresh directory required".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let mut scene=Scene::default();scene.sky=Some((16,8,vec![[1.,1.,1.,1.];128]));
    let checker=image::RgbaImage::from_fn(64,64,|x,y|if (x/4+y/4)%2==0{image::Rgba([210,170,90,255])}else{image::Rgba([40,70,120,255])});
    let layer=scene.add_image("heat background checker",checker);quad(&mut scene,-5.0,3.0,layer);
    let blocker=quad(&mut scene,-2.0,2.0,-1);
    scene.update_instance(1,blocker,Affine3A::from_translation(Vec3::X*1000.));
    let camera=Camera {pos:Vec3::ZERO,target:Vec3::NEG_Z,fov_y:55f32.to_radians(),near:0.05};
    let light=Lighting {sun_lux:0.,sky_lux:1200.,ev100:9.,gi:0.,contact_occlusion:0.,reflection_occlusion:0.,
        shadow_strength:0.,near_shadows:0.,fog_density:0.,fog_start:1000.,fog_end:2000.,taa:0.,sharpen:0.,lod_pixels:0.,..Default::default()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);
    let base=Sprite {position_radius:[0.,-0.35,-3.,0.3],axis_length:[0.,1.,0.,0.8],colour_alpha:[0.4,0.4,0.4,0.8],params:[1.,0.3,0.5,4.]};
    let (off,_)=renderer.render(&gpu,&camera,&light,W,H);render::write_png(&out.join("off.png"),W,H,&off)?;
    let mut differences=Vec::new();let mut failures=Vec::new();let mut heat=Vec::new();
    for (name,sprite) in [("heat",base),("smoke",Sprite {params:[0.,0.3,0.5,0.],..base}),
        ("flame",Sprite {params:[2.,0.3,0.01,0.],colour_alpha:[9000.,2000.,100.,0.9],..base})] {
        renderer.set_exhaust(&gpu,&[sprite])?;let (pixels,stats)=renderer.render(&gpu,&camera,&light,W,H);
        render::write_png(&out.join(format!("{name}.png")),W,H,&pixels)?;
        let changed=difference(&off,&pixels);if changed<50{failures.push(format!("{name} invisible: {changed} changed pixels"));}
        differences.push(format!("\"{name}\":{{\"changed_pixels\":{changed},\"gpu_ms\":{:?}}}",stats.gpu_ms));
        if name=="heat"{heat=pixels;}
    }
    renderer.set_exhaust(&gpu,&[base])?;
    let views:Vec<_>=(0..4).map(|i|ViewRect {cam:camera,x:i%2*W,y:i/2*H,w:W,h:H}).collect();
    let (split,_)=renderer.render_views(&gpu,&views,&light,W*2,H*2,&[]);render::write_png(&out.join("heat-four.png"),W*2,H*2,&split)?;
    for i in 0..4{let single:Vec<_>=(0..H).flat_map(|y|{let at=(((y+i/2*H)*W*2+i%2*W)*4)as usize;split[at..at+(W*4)as usize].iter().copied()}).collect();
        if difference(&heat,&single)!=0{failures.push(format!("split view {i} differs from same-camera single view"));}}
    scene.update_instance(1,blocker,Affine3A::IDENTITY);renderer.update_instances(&gpu,1,&scene.instances[1..])?;
    renderer.set_exhaust(&gpu,&[])?;let (blocked_off,_)=renderer.render(&gpu,&camera,&light,W,H);
    renderer.set_exhaust(&gpu,&[base])?;let (blocked_on,_)=renderer.render(&gpu,&camera,&light,W,H);
    let blocked_difference=difference(&blocked_off,&blocked_on);
    if blocked_difference!=0{failures.push(format!("heat warped opaque foreground: {blocked_difference}"));}
    render::write_png(&out.join("heat-blocked.png"),W,H,&blocked_on)?;
    // Two independent temporal histories, differing only by past effect use.
    // Turning off effects must give exactly the untouched reference history.
    scene.update_instance(1,blocker,Affine3A::from_translation(Vec3::X*1000.));
    let a=Renderer::new(&gpu,&scene,&light);let b=Renderer::new(&gpu,&scene,&light);
    let temporal=Lighting {taa:1.,..light};
    a.set_exhaust(&gpu,&[base])?;
    for _ in 0..3{a.render(&gpu,&camera,&temporal,W,H);b.render(&gpu,&camera,&temporal,W,H);}
    a.set_exhaust(&gpu,&[])?;
    let (a,_)=a.render(&gpu,&camera,&temporal,W,H);let (b,_)=b.render(&gpu,&camera,&temporal,W,H);
    let history_difference=difference(&a,&b);if history_difference!=0{failures.push(format!("effects contaminated TAA history: {history_difference}"));}
    // Format times separately: Rust debug tuples are not valid JSON arrays.
    let metrics=differences.join(",").replace("(\"","[\"").replace(")", "]");
    let report=format!("{{\"scope\":\"native bounded exhaust fixture; not FPS\",\"runs\":{{{metrics}}},\"blocked_changed_pixels\":{blocked_difference},\"history_changed_pixels\":{history_difference},\"passed\":{},\"failures\":{:?}}}",failures.is_empty(),failures);
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("exhaust probe PASS");Ok(())}else{Err(failures.join("\n"))}
}
