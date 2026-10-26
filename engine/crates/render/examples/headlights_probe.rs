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
    mesh_flags(scene,positions,normal,0)
}
fn mesh_flags(scene:&mut Scene,positions:&[[f32;3]],normal:Vec3,flags:u32)->usize {
    // The GRASS shader always samples its photograph. A -1 layer is invalid
    // even for a white diagnostic receiver; use a real opaque white texture.
    let layer=if flags&render::scene::GRASS!=0 {
        scene.add_image("headlight probe white grass receiver",image::RgbaImage::from_pixel(2,2,image::Rgba([255;4])))
    }else{-1};
    let m=scene.add_material(Material {tint:[0.45;3],layer,roughness:1.0,flags:TWO_SIDED|flags,scale:1.0,metal:0.0});
    let uvs=[[0.0,1.0],[1.0,1.0],[1.0,0.0],[0.0,0.0]];
    let vertices:Vec<_>=positions.iter().enumerate().map(|(i,&p)|Vertex {pos:p,normal:normal.to_array(),uv:uvs[i]}).collect();
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
    // Entire visible grass receiver is ABOVE the lamp. The former world-UP
    // Lambert rejection made it black despite its inclined blade distribution.
    let grass=mesh_flags(&mut scene,&[[3.0,1.8,-10.0],[4.0,1.8,-10.0],[4.0,2.8,-10.0],[3.0,2.8,-10.0]],Vec3::Y,render::scene::GRASS);
    let grass_owner=scene.instances.len();
    scene.add_instance(grass,Affine3A::IDENTITY);
    scene.instances[grass_owner].flags|=render::scene::NO_SHADOW;
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
    let grass_point=pixel(cam,Vec3::new(3.5,2.3,-10.0));
    let grass_off=region(&dark,grass_point);let grass_on=region(&open,grass_point);
    let mut failures=Vec::new();
    if grass_off>1.0||grass_on<10.0 {failures.push(format!("grass above lamp must be dark with lamp off and lit when on: {grass_off}/{grass_on}"));}
    if off>1.0 {failures.push(format!("zero-light floor unexpectedly bright: {off}"));}
    if lit<15.0 {failures.push(format!("headlight did not light receiver: {lit}"));}
    if shadow>lit*0.25+1.0 {failures.push(format!("light leaks through real wall: shadow={shadow}, open={lit}"));}
    // Same world/light set, four independently rendered cameras; no extra
    // lamp shadow passes or ambient light should appear in split screen.
    // Exercise the highest supported layer too: two sources for four cars.
    let lamps:Vec<_>=(0..8).map(|i|SpotLight {position:lamp.position+Vec3::X*(i as f32*0.15),intensity_cd:lamp.intensity_cd/8.0,..lamp}).collect();
    renderer.set_headlights(&lamps)?;
    let (eight_lamp_reference,_)=renderer.render(&gpu,&cam,&light,W,H);
    render::write_png(&out.join("eight_lamp_reference.png"),W,H,&eight_lamp_reference)?;
    let eight_rgb=region(&eight_lamp_reference,point);
    let mut highest_only=lamps.clone();
    for l in highest_only.iter_mut().take(7) {l.intensity_cd=Vec3::ZERO;}
    renderer.set_headlights(&highest_only)?;
    let (last_layer,_)=renderer.render(&gpu,&cam,&light,W,H);
    let last_rgb=region(&last_layer,point);
    if last_rgb<3.0 {failures.push(format!("highest headlamp layer 7 does not illuminate receiver: {last_rgb}"));}
    if eight_rgb<=last_rgb {failures.push("eight active lights do not add to last-layer illumination".into());}
    renderer.set_headlights(&lamps)?;
    let views:Vec<_>=(0..4).map(|i|ViewRect {cam,x:(i%2)*W,y:(i/2)*H,w:W,h:H}).collect();
    let (split,split_stats)=renderer.render_views(&gpu,&views,&light,W*2,H*2,&[]);
    let mut differences=Vec::new();let mut difference_image=vec![0u8;split.len()];
    for i in 0..4 {
        let(mut pixels,mut channels,mut maximum)=(0u32,0u32,0u8);
        let(mut lo,mut hi)=([W,H],[0u32;2]);let mut worst=[0u32;2];let mut samples=Vec::new();
        for y in 0..H {for x in 0..W {
            let a=(((y+(i/2)*H)*W*2+(i%2)*W+x)*4)as usize;let b=((y*W+x)*4)as usize;
            let delta:Vec<_>=(0..4).map(|c|split[a+c].abs_diff(eight_lamp_reference[b+c])).collect();
            difference_image[a..a+3].copy_from_slice(&delta[..3]);difference_image[a+3]=255;
            if delta.iter().all(|d|*d==0) {continue;}
            pixels+=1;channels+=delta.iter().filter(|d|**d!=0).count()as u32;
            lo=[lo[0].min(x),lo[1].min(y)];hi=[hi[0].max(x),hi[1].max(y)];
            let local_max=*delta.iter().max().unwrap();if local_max>maximum {maximum=local_max;worst=[x,y];}
            if samples.len()<32 {samples.push(format!("{{\"pixel\":[{x},{y}],\"reference_rgba\":{:?},\"split_rgba\":{:?},\"absolute_delta\":{delta:?}}}",&eight_lamp_reference[b..b+4],&split[a..a+4]));}
        }}
        let bounds=if pixels==0 {"null".to_owned()}else{format!("[{lo:?},{hi:?}]")};
        differences.push(format!("{{\"view\":{i},\"different_pixels\":{pixels},\"different_channels\":{channels},\"max_absolute_u8\":{maximum},\"max_pixel\":{worst:?},\"bounds_inclusive\":{bounds},\"first_differences\":[{}]}}",samples.join(",")));
        if pixels!=0 {failures.push(format!("split view {i} differs in {pixels} pixels, max channel delta {maximum} at {worst:?}, bounds {bounds}"));}
    }
    render::write_png(&out.join("four-view-shared-light.png"),W*2,H*2,&split)?;
    render::write_png(&out.join("four-view-absolute-difference.png"),W*2,H*2,&difference_image)?;
    let times=|stats:&render::FrameStats|stats.gpu_ms.iter().map(|(name,ms)|format!("[{name:?},{ms}]")).collect::<Vec<_>>().join(",");
    let report=format!("{{\"schema\":3,\"scope\":\"isolated native zero-sun/sky fixture; not game FPS\",\"grass_receiver\":\"explicit opaque white 2x2 texture, UV [0,1] quad\",\"receiver\":[{},{}],\"off_rgb\":{off},\"blocked_rgb\":{shadow},\"open_rgb\":{lit},\"grass_off_rgb\":{grass_off},\"grass_above_lamp_rgb\":{grass_on},\"eight_lamps_rgb\":{eight_rgb},\"last_layer_only_rgb\":{last_rgb},\"split_exact_differences\":[{}],\"blocked_gpu_ms\":[{}],\"open_gpu_ms\":[{}],\"split_gpu_ms\":[{}],\"failures\":{:?},\"passed\":{}}}",
        point.0,point.1,differences.join(","),times(&blocked_stats),times(&open_stats),times(&split_stats),failures,failures.is_empty());
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("headlights: off={off}, shadow={shadow}, open={lit}; four views match");Ok(())}else{Err(failures.join("\n"))}
}
