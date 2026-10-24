//! Exclusive-GPU fixture: zero sun/sky, real spotlight shadows and four views.
//! Run: headlights_probe --out FRESH_DIRECTORY. Not a gameplay FPS claim.
use glam::{Affine3A,Mat4,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::{Material,TWO_SIDED};
use render::headlights::SpotLight;
use std::path::PathBuf;
const W:u32=640;const H:u32=384;
fn mesh(scene:&mut Scene,positions:&[[f32;3]],normal:Vec3)->usize {
    let m=scene.add_material(Material {tint:[0.45;3],layer:-1,roughness:1.0,flags:TWO_SIDED,scale:1.0,metal:0.0});
    let vertices:Vec<_>=positions.iter().map(|&p|Vertex {pos:p,normal:normal.to_array(),uv:[0.0;2]}).collect();
    scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&[0,1,2,0,2,3],material:m}]))
}
fn pixel(cam:Camera,p:Vec3)->(u32,u32) {
    let vp=Mat4::perspective_infinite_reverse_rh(cam.fov_y,W as f32/H as f32,cam.near)*Mat4::look_at_rh(cam.pos,cam.target,Vec3::Y);
    let p=vp.project_point3(p);(((p.x*0.5+0.5)*W as f32)as u32,((0.5-p.y*0.5)*H as f32)as u32)
}
fn region(image:&[u8],point:(u32,u32))->f32 {
    let mut sum=0.0;
    for y in point.1-2..=point.1+2 {for x in point.0-2..=point.0+2 {
        let at=((y*W+x)*4)as usize;sum+=image[at..at+3].iter().map(|v|*v as f32).sum::<f32>()/3.0;
    }}sum/25.0
}
fn main() {if let Err(e)=run(){eprintln!("headlights probe: {e}");std::process::exit(1);}}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIRECTORY required")?[1]);
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let mut scene=Scene::default();scene.sky=Some((16,8,vec![[0.0,0.0,0.0,1.0];128]));scene.sky_physical=true;
    let floor=mesh(&mut scene,&[[-30.0,0.0,5.0],[30.0,0.0,5.0],[30.0,0.0,-70.0],[-30.0,0.0,-70.0]],Vec3::Y);
    scene.add_instance(floor,Affine3A::IDENTITY);
    let wall=mesh(&mut scene,&[[-1.5,0.0,-9.0],[1.5,0.0,-9.0],[1.5,2.0,-9.0],[-1.5,2.0,-9.0]],Vec3::Z);
    scene.add_instance(wall,Affine3A::IDENTITY);
    let cam=Camera {pos:Vec3::new(4.0,6.0,5.0),target:Vec3::new(0.0,0.0,-18.0),fov_y:60f32.to_radians(),near:0.1};
    let light=Lighting {sun_lux:0.0,sky_lux:0.0,ev100:6.0,gi:0.0,contact_occlusion:0.0,reflection_occlusion:0.0,
        shadow_strength:0.0,near_shadows:0.0,fog_start:1000.0,fog_end:2000.0,fog_density:0.0,
        taa:0.0,sharpen:0.0,lod_pixels:0.0,..Default::default()};
    let lamp=SpotLight {position:Vec3::new(0.0,1.2,0.0),direction:Vec3::new(0.0,-0.045,-1.0).normalize(),up:Vec3::Y,
        intensity_cd:Vec3::splat(80_000.0),range_m:90.0,inner_angle_radians:17f32.to_radians(),outer_angle_radians:32f32.to_radians()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);
    let (dark,_)=renderer.render(&gpu,&cam,&light,W,H);render::write_png(&out.join("night-lamps-off.png"),W,H,&dark)?;
    renderer.set_headlights(&[lamp])?;
    let (blocked,blocked_stats)=renderer.render(&gpu,&cam,&light,W,H);render::write_png(&out.join("night-shadowed.png"),W,H,&blocked)?;
    scene.update_instance(1,wall,Affine3A::from_translation(Vec3::X*1000.0));renderer.update_instances(&gpu,1,&scene.instances[1..])?;
    let (open,open_stats)=renderer.render(&gpu,&cam,&light,W,H);render::write_png(&out.join("night-unblocked.png"),W,H,&open)?;
    let point=pixel(cam,Vec3::new(0.0,0.0,-18.0));let shadow=region(&blocked,point);let lit=region(&open,point);
    let off=region(&dark,point);
    let mut failures=Vec::new();
    if off>1.0 {failures.push(format!("zero-light floor unexpectedly bright: {off}"));}
    if lit<15.0 {failures.push(format!("headlight did not light receiver: {lit}"));}
    if shadow>lit*0.25+1.0 {failures.push(format!("light leaks through real wall: shadow={shadow}, open={lit}"));}
    // Same world/light set, four independently rendered cameras; no extra
    // lamp shadow passes or ambient light should appear in split screen.
    let lamps:Vec<_>=(0..4).map(|i|SpotLight {position:lamp.position+Vec3::X*(i as f32*0.3),intensity_cd:lamp.intensity_cd/4.0,..lamp}).collect();
    renderer.set_headlights(&lamps)?;
    let (four_lamp_reference,_)=renderer.render(&gpu,&cam,&light,W,H);
    let views:Vec<_>=(0..4).map(|i|ViewRect {cam,x:(i%2)*W,y:(i/2)*H,w:W,h:H}).collect();
    let (split,split_stats)=renderer.render_views(&gpu,&views,&light,W*2,H*2,&[]);
    for i in 0..4 {for y in 0..H {let a=(((y+(i/2)*H)*W*2+(i%2)*W)*4)as usize;let b=(y*W*4)as usize;
        if split[a..a+(W*4)as usize]!=four_lamp_reference[b..b+(W*4)as usize] {failures.push(format!("split view {i} differs at row{y}"));break;}
    }}
    render::write_png(&out.join("four-view-shared-light.png"),W*2,H*2,&split)?;
    let times=|stats:&render::FrameStats|stats.gpu_ms.iter().map(|(name,ms)|format!("[{name:?},{ms}]")).collect::<Vec<_>>().join(",");
    let report=format!("{{\"schema\":1,\"scope\":\"isolated native zero-sun/sky fixture; not game FPS\",\"receiver\":[{},{}],\"off_rgb\":{off},\"blocked_rgb\":{shadow},\"open_rgb\":{lit},\"blocked_gpu_ms\":[{}],\"open_gpu_ms\":[{}],\"split_gpu_ms\":[{}],\"failures\":{:?},\"passed\":{}}}",
        point.0,point.1,times(&blocked_stats),times(&open_stats),times(&split_stats),failures,failures.is_empty());
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("headlights: off={off}, shadow={shadow}, open={lit}; four views match");Ok(())}else{Err(failures.join("\n"))}
}
