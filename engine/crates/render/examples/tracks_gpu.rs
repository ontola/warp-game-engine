//! Exclusive GPU material fixture, never gameplay FPS. Run --out FRESH_DIR.
use glam::{Affine3A,Mat4,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::{Material,ROAD};
use render::tracks::Stamp;
use std::path::PathBuf;
const W:u32=640;const H:u32=384;
fn plane(scene:&mut Scene,lo:Vec3,hi:Vec3,material:u32) {
    let positions=[[lo.x,lo.y,hi.z],[hi.x,lo.y,hi.z],[hi.x,lo.y,lo.z],[lo.x,lo.y,lo.z]];
    let uv=[[0.,0.],[0.9,0.],[0.9,16.],[0.,16.]];
    let vertices:Vec<_>=positions.into_iter().zip(uv).map(|(pos,uv)|Vertex{pos,normal:Vec3::Y.to_array(),uv}).collect();
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,0,2,3],material}]));scene.add_instance(mesh,Affine3A::IDENTITY);
}
fn count_difference(a:&[u8],b:&[u8])->usize {a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)|a[..3]!=b[..3]).count()}
fn pixel(camera:Camera,p:Vec3)->usize {
    let vp=Mat4::perspective_infinite_reverse_rh(camera.fov_y,W as f32/H as f32,camera.near)*Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y);
    let q=vp.project_point3(p);let x=((q.x*0.5+0.5)*W as f32)as u32;let y=((0.5-q.y*0.5)*H as f32)as u32;((y*W+x)*4)as usize
}
fn mean(values:&[f64])->f64 {values.iter().sum::<f64>()/values.len().max(1)as f64}
fn main() {if let Err(e)=run(){eprintln!("tracks fixture: {e}");std::process::exit(1);}}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();let out=PathBuf::from(&args.windows(2).find(|w|w[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists(){return Err("output directory must be fresh".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let mut scene=Scene::default();scene.sky=Some((16,8,vec![[0.,0.,0.,1.];128]));scene.sky_physical=true;
    let layer=scene.add_image("track fixture mineral soil",image::RgbaImage::from_pixel(4,4,image::Rgba([175,145,105,255])));
    scene.add_image("track fixture shoulder",image::RgbaImage::from_pixel(4,4,image::Rgba([175,145,105,255])));
    let material=scene.add_material(Material{tint:[1.;3],layer,roughness:0.9,flags:ROAD,scale:1.,metal:0.});
    plane(&mut scene,Vec3::new(-8.,0.,-10.),Vec3::new(8.,0.,8.),material);
    plane(&mut scene,Vec3::new(1.5,2.,-2.5),Vec3::new(4.5,2.,2.5),material);
    let camera=Camera{pos:Vec3::new(7.,10.,11.),target:Vec3::new(0.,0.,-1.),fov_y:58f32.to_radians(),near:0.05};
    let light=Lighting{sun_dir:Vec3::new(-0.3,0.85,0.4).normalize(),sun_lux:80_000.,sky_lux:0.,ev100:13.5,gi:0.,shadow_strength:0.,near_shadows:0.,
        contact_occlusion:0.,reflection_occlusion:0.,fog_start:1000.,fog_end:2000.,fog_density:0.,taa:0.,sharpen:0.,lod_pixels:0.,..Default::default()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);let mut off_times=Vec::new();let mut on_times=Vec::new();let mut off=Vec::new();
    for i in 0..4 {let (pixels,stats)=renderer.render(&gpu,&camera,&light,W,H);off=pixels;if i>0 {off_times.push(stats.gpu_ms.iter().find(|(name,_)|*name=="shade").map(|(_,ms)|*ms as f64).unwrap_or(0.));}}
    render::write_png(&out.join("no-marks.png"),W,H,&off)?;
    let mut stamps=Vec::new();for wheel in [-0.7f32,0.7] {for i in 0..80 {
        let z=i as f32*0.15-6.;let x=-2.+(z*0.45).sin()*1.2+wheel;
        let next_z=z+0.15;let next_x=-2.+(next_z*0.45).sin()*1.2+wheel;
        stamps.push(Stamp{start:[x,0.,z],end:[next_x,0.,next_z],start_normal:[0.,1.,0.],end_normal:[0.,1.,0.],width:0.24,compaction:0.85,rubber:0.});
    }}
    // Under the elevated surface: no mark may project onto its top.
    for i in 0..8 {stamps.push(Stamp{start:[2.+i as f32*0.25,0.,-2.],end:[2.+i as f32*0.25,0.,2.],start_normal:[0.,1.,0.],end_normal:[0.,1.,0.],width:0.24,compaction:1.,rubber:0.});}
    let atlas=renderer.update_tracks(&gpu,&stamps)?;let mut on=Vec::new();
    for i in 0..4 {let (pixels,stats)=renderer.render(&gpu,&camera,&light,W,H);on=pixels;if i>0 {on_times.push(stats.gpu_ms.iter().find(|(name,_)|*name=="shade").map(|(_,ms)|*ms as f64).unwrap_or(0.));}}
    render::write_png(&out.join("lit-marks.png"),W,H,&on)?;let changed=count_difference(&off,&on);let mut failures=Vec::new();
    if changed<100 {failures.push(format!("continuous marks invisible: {changed} pixels"));}
    let mut bridge_changes=0;for z in [-1.5f32,-0.5,0.5,1.5] {for x in [2.25f32,3.,3.75] {let at=pixel(camera,Vec3::new(x,2.,z));bridge_changes+=usize::from(off[at..at+3]!=on[at..at+3]);}}
    if bridge_changes>0 {failures.push(format!("marks leaked onto bridge at{bridge_changes} probes"));}
    let views:Vec<_>=(0..4).map(|i|ViewRect{cam:camera,x:i%2*W,y:i/2*H,w:W,h:H}).collect();let (split,split_stats)=renderer.render_views(&gpu,&views,&light,W*2,H*2,&[]);
    render::write_png(&out.join("four-shared-marks.png"),W*2,H*2,&split)?;
    for i in 0..4 {let tile:Vec<_>=(0..H).flat_map(|y|{let at=(((y+i/2*H)*W*2+i%2*W)*4)as usize;split[at..at+(W*4)as usize].iter().copied()}).collect();if count_difference(&on,&tile)!=0{failures.push(format!("view{i} changed the shared material marks"));}}
    let dark=Lighting{sun_lux:0.,..light};let (dark_on,_)=renderer.render(&gpu,&camera,&dark,W,H);render::write_png(&out.join("night-marks.png"),W,H,&dark_on)?;
    renderer.clear_tracks(&gpu);let (dark_off,_)=renderer.render(&gpu,&camera,&dark,W,H);let night_difference=count_difference(&dark_on,&dark_off);
    if night_difference>0 {failures.push(format!("marks emit or darken unlit geometry: {night_difference} pixels"));}
    let (cleared,_)=renderer.render(&gpu,&camera,&light,W,H);render::write_png(&out.join("stage-cleared.png"),W,H,&cleared)?;let clear_difference=count_difference(&off,&cleared);
    if clear_difference>0 {failures.push(format!("stage clear retains stale marks:{clear_difference}"));}
    let split_shade:Vec<_>=split_stats.gpu_ms.iter().filter(|(name,_)|*name=="shade").map(|(_,ms)|*ms as f64).collect();
    let report=format!("{{\"scope\":\"640x384 native material fixture, same-camera fourviews; not gameplay FPS\",\"passed\":{},\"changed_pixels\":{changed},\"bridge_changes\":{bridge_changes},\"night_difference\":{night_difference},\"clear_difference\":{clear_difference},\"shade_off_ms\":{},\"shade_on_ms\":{},\"split_shade_ms\":{:?},\"stamps\":{},\"tiles\":{},\"upload_bytes\":{},\"gpu_bytes\":{},\"failures\":{:?}}}",failures.is_empty(),mean(&off_times),mean(&on_times),split_shade,atlas.stamps,atlas.tiles_resident,atlas.last_upload_bytes,atlas.gpu_bytes,failures);
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("tracks fixture PASS");Ok(())}else{Err(failures.join("\n"))}
}
