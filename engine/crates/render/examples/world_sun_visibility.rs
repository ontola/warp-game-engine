//! Real-renderer, receiver-based terrain sunlight regression. Exclusive GPU.
//! world_sun_visibility --out FRESH_DIRECTORY
//! Separate-process baseline: KERN_WORLD_TERRAIN_VISIBILITY=0 (default: 1 here).
//! Native and fixed exposure. Heightfield terrain only, no foliage/mesh-GI claim.
use glam::{Affine3A, Vec3};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Heightfield, Material, TWO_SIDED};
use std::path::{Path, PathBuf};

const W:u32=257;
const H:u32=193;
const GW:u32=385;
const GH:u32=161;
const CELL:f32=2.0;
const ORIGIN:[f32;2]=[-64.0,-160.0];

fn height(x:f32,z:f32)->f32 {
    let along=((x-576.0)/16.0).clamp(0.0,1.0)*((624.0-x)/16.0).clamp(0.0,1.0);
    let across=((84.0-z.abs())/12.0).clamp(0.0,1.0);
    30.0*along*across
}
#[derive(Clone,Copy)]
struct Receiver {name:&'static str,p:Vec3,owner:u32,blocked:bool}
fn make_scene()->(Scene,[Receiver;4]) {
    let mut scene=Scene::default();
    scene.terrain_visibility_complete=true;
    scene.sky_physical=true;
    scene.sky=Some((2,1,vec![[80.0,95.0,120.0,1.0];2]));
    let material=scene.add_material(Material{tint:[0.3;3],layer:-1,roughness:1.0,metal:0.0,flags:TWO_SIDED,scale:1.0});
    let mut vertices=Vec::with_capacity((GW*GH)as usize);let mut heights=Vec::with_capacity(vertices.capacity());
    for z in 0..GH{for x in 0..GW{
        let px=ORIGIN[0]+x as f32*CELL;let pz=ORIGIN[1]+z as f32*CELL;let y=height(px,pz);
        let dx=(height(px+CELL,pz)-height(px-CELL,pz))/(2.0*CELL);
        let dz=(height(px,pz+CELL)-height(px,pz-CELL))/(2.0*CELL);
        vertices.push(Vertex{pos:[px,y,pz],normal:Vec3::new(-dx,1.0,-dz).normalize().to_array(),uv:[0.0;2]});
        heights.push(y);
    }}
    let mut indices=Vec::new();
    for z in 0..GH-1{for x in 0..GW-1{let a=z*GW+x;indices.extend([a,a+GW,a+1,a+1,a+GW,a+GW+1]);}}
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&indices,material}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);
    scene.heightfield=Heightfield{origin:ORIGIN,cell:CELL,w:GW,h:GH,heights};
    let receivers=[("floor",Vec3::new(0.0,0.01,0.0),true),
        ("low-roof",Vec3::new(0.0,3.0,40.0),true),
        ("high-roof",Vec3::new(0.0,40.0,-40.0),false),
        ("open-floor",Vec3::new(0.0,0.01,110.0),false)]
        .map(|(name,p,blocked)|{
            // Separate owners prove each tested pixel belongs to this exact
            // world receiver, rather than an unrelated background triangle.
            let vertices=[[-5.0,0.0,-5.0],[-5.0,0.0,5.0],[5.0,0.0,-5.0],[5.0,0.0,5.0]]
                .map(|pos|Vertex{pos,normal:[0.0,1.0,0.0],uv:[0.0;2]});
            let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,2,1,3],material}]));
            let owner=scene.instances.len()as u32;scene.add_instance(mesh,Affine3A::from_translation(p));
            Receiver{name,p,owner,blocked}
        });
    (scene,receivers)
}
fn camera(receiver:Receiver,offset:Vec3)->Camera {
    Camera{pos:receiver.p+offset,target:receiver.p,fov_y:12f32.to_radians(),near:0.1}
}
fn sample_pixels()->Vec<(u32,u32)>{
    (-2..=2).flat_map(|y|(-3..=3).map(move|x|((W as i32/2+x*3)as u32,(H as i32/2+y*2)as u32))).collect()
}
fn rgb(bytes:&[u8],width:u32,p:(u32,u32))->[u8;3]{let i=((p.1*width+p.0)*4)as usize;bytes[i..i+3].try_into().unwrap()}
fn unknown(c:[u8;3])->bool {c[0]>32&&c[2]>32&&c[1]<c[0]/3}
fn frame_record(name:&str,s:&FrameStats)->String {
    let times=s.gpu_ms.iter().map(|(n,t)|format!("[{n:?},{t}]")).collect::<Vec<_>>().join(",");
    format!("{{\"name\":{name:?},\"world_terrain_visibility\":{},\"bytes\":{},\"setup_cpu_ms\":{},\"triangles\":{},\"gpu_ms\":[{times}]}}",
        s.world_terrain_visibility,s.world_terrain_bytes,s.world_terrain_setup_cpu_ms,s.triangles)
}
fn save(out:&Path,name:&str,width:u32,height:u32,pixels:&[u8])->Result<(),String>{render::write_png(&out.join(format!("{name}.png")),width,height,pixels)}
fn panel(bytes:&[u8],i:usize)->Vec<u8>{
    (0..H).flat_map(|y|{let x=(i as u32%2)*W;let y=y+(i as u32/2)*H;
        let a=((y*W*2+x)*4)as usize;bytes[a..a+(W*4)as usize].iter().copied()}).collect()
}
fn run()->Result<(),String>{
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|p|p[0]=="--out").ok_or("use --out FRESH_DIRECTORY")?[1]);
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    // Explicit opt-in in this diagnostic process only. Do not force ON over
    // a caller's requested OFF baseline or mutate another running process.
    if std::env::var_os("KERN_WORLD_TERRAIN_VISIBILITY").is_none(){unsafe{std::env::set_var("KERN_WORLD_TERRAIN_VISIBILITY","1");}}
    unsafe{std::env::set_var("KERN_FAR_SUN_SHADOW","0");}
    let enabled=std::env::var("KERN_WORLD_TERRAIN_VISIBILITY").as_deref()==Ok("1");
    let elevation=2f32.to_radians();let sun=Vec3::new(elevation.cos(),elevation.sin(),0.0);
    let light=Lighting{sun_dir:sun,sun_lux:55_000.0,sun_color:Vec3::ONE,sky_lux:300.0,ev100:10.0,
        gi:0.0,near_shadows:1.0,shadow_strength:1.0,lod_pixels:0.0,taa:0.0,sharpen:0.0,
        fog_density:0.0,fog_start:10000.0,fog_end:20000.0,contact_occlusion:0.0,reflection_occlusion:0.0,..Default::default()};
    let (scene,receivers)=make_scene();let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);
    let mut no_sun=light.clone();no_sun.sun_lux=0.0;
    let mut diagnostic=light.clone();diagnostic.debug=23.0;
    let mut work=light.clone();work.debug=24.0;
    let picks=sample_pixels();let mut failures=Vec::new();let mut frames=Vec::new();let mut measurements=Vec::new();
    let mut close_reference=Vec::new();let mut close_cameras=Vec::new();
    for receiver in receivers {
        for (variant,offset) in [("far",Vec3::new(0.0,24.0,140.0)),("near",Vec3::new(0.0,8.0,40.0)),("orbit",Vec3::new(-36.0,9.0,18.0))] {
            let camera=camera(receiver,offset);let name=format!("{}-{variant}",receiver.name);
            renderer.set_lighting_focus(&[camera.pos])?;
            let (mask,stats)=renderer.render_picking(&gpu,&camera,&diagnostic,W,H,&picks);
            let (cost,cs)=renderer.render_picking(&gpu,&camera,&work,W,H,&picks);
            let (lit,ls)=renderer.render_picking(&gpu,&camera,&light,W,H,&picks);
            let (ambient,sa)=renderer.render_picking(&gpu,&camera,&no_sun,W,H,&picks);
            for (suffix,pixels,stats) in [("visibility",&mask,&stats),("work",&cost,&cs),("lit",&lit,&ls),("no-sun",&ambient,&sa)]{
                save(&out,&format!("{name}-{suffix}"),W,H,pixels)?;frames.push(frame_record(&format!("{name}-{suffix}"),stats));
            }
            let valid:Vec<_>=picks.iter().enumerate().filter(|(i,_)|stats.picks[*i].is_some_and(|p|p.0==receiver.owner)
                &&ls.picks[*i].is_some_and(|p|p.0==receiver.owner)&&sa.picks[*i].is_some_and(|p|p.0==receiver.owner)).map(|(_,p)|*p).collect();
            if valid.len()<15{failures.push(format!("{name}: only{} picking-confirmed receiver samples",valid.len()));}
            let should_block=enabled&&receiver.blocked;let mut maximum_sun_delta=0u8;let mut total_sun_delta=0u64;let mut max_work=0u8;
            for &p in &valid {
                let c=rgb(&mask,W,p);let a=rgb(&lit,W,p);let b=rgb(&ambient,W,p);let cost=rgb(&cost,W,p);
                if unknown(c)||unknown(cost){failures.push(format!("{name}: unknown terrain visibility at{p:?}"));break;}
                if should_block&&c.into_iter().max().unwrap()>1{failures.push(format!("{name}: blocked receiver is lit in debug23 ({c:?})"));break;}
                if !should_block&&c.into_iter().min().unwrap()<80{failures.push(format!("{name}: exposed receiver is not certified clear ({c:?})"));break;}
                maximum_sun_delta=maximum_sun_delta.max((0..3).map(|k|a[k].abs_diff(b[k])).max().unwrap());
                total_sun_delta+=(0..3).map(|k|u64::from(a[k].saturating_sub(b[k]))).sum::<u64>();
                max_work=max_work.max(cost[0]);
            }
            if should_block&&maximum_sun_delta>1{failures.push(format!("{name}: terrain-blocked direct light changed RGB by{maximum_sun_delta}"));}
            let mean_delta=total_sun_delta as f64/(valid.len().max(1)*3)as f64;
            // OFF does not promise that another real shadow map misses the
            // ridge. Only genuinely open receivers must have direct sunlight.
            if !receiver.blocked&&mean_delta<2.0{failures.push(format!("{name}: open receiver has no visible direct sun ({mean_delta})"));}
            measurements.push(format!("{{\"name\":{name:?},\"owner\":{},\"receiver\":{:?},\"camera\":{:?},\"target\":{:?},\"expected_blocked\":{should_block},\"receiver_samples\":{},\"max_sun_delta\":{maximum_sun_delta},\"mean_sun_delta\":{mean_delta},\"work_display_max\":{max_work}}}",receiver.owner,receiver.p.to_array(),camera.pos.to_array(),camera.target.to_array(),valid.len()));
            if variant=="near"{close_reference.push(mask);close_cameras.push(camera);}
        }
    }
    let views:Vec<_>=close_cameras.into_iter().enumerate().map(|(i,cam)|ViewRect{cam,x:(i as u32%2)*W,y:(i as u32/2)*H,w:W,h:H}).collect();
    renderer.set_lighting_focus(&views.iter().map(|v|v.cam.pos).collect::<Vec<_>>())?;
    let (split,ss)=renderer.render_views(&gpu,&views,&diagnostic,W*2,H*2,&[]);save(&out,"split-visibility",W*2,H*2,&split)?;frames.push(frame_record("split-visibility",&ss));
    for (i,expected) in close_reference.iter().enumerate(){let actual=panel(&split,i);let changed=actual.chunks_exact(4).zip(expected.chunks_exact(4)).filter(|(a,b)|a[..3]!=b[..3]).count();
        if changed!=0{failures.push(format!("split view{i}: visibility differs in{changed} pixels from its single view"));}}
    let report=format!("{{\"schema\":1,\"passed\":{},\"enabled\":{enabled},\"scene_terrain_visibility_complete\":true,\"far_source_map\":false,\"sun_direction\":{:?},\"native_heightfield_cell_m\":{CELL},\"ridge_distance_m\":600,\"ridge_height_m\":30,\"node_budget_setting\":{:?},\"scope\":\"same native terrain mesh and heightfield, receiver-centred solar visibility; no foliage, local GI or gameplay FPS claim\",\"measurements\":[{}],\"frames\":[{}],\"failures\":{:?}}}",
        failures.is_empty(),sun.to_array(),std::env::var("KERN_TERRAIN_NODE_BUDGET").unwrap_or_else(|_|"default256".into()),measurements.join(","),frames.join(","),failures);
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("PASS: {}",out.join("report.json").display());Ok(())}else{Err(failures.join("\n"))}
}
fn main(){if let Err(e)=run(){eprintln!("world sun visibility: {e}");std::process::exit(1);}}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn world_sun_fixture_matches_native_terrain_and_distant_blocker(){
        let(scene,receivers)=make_scene();let hierarchy=render::terrain_trace::Hierarchy::with_coverage(&scene.heightfield,render::terrain_trace::Coverage::CompleteLoadedTerrain).unwrap();
        let a=2f32.to_radians();let sun=Vec3::new(a.cos(),a.sin(),0.0);
        for r in receivers{let trace=hierarchy.trace_upward(r.p+Vec3::Y*0.025,sun,0.01,256);
            assert_eq!(trace.status==render::terrain_trace::Status::Hit,r.blocked,"{}: {:?}",r.name,trace);
            if r.blocked{assert!(trace.distance>570.0&&trace.distance<610.0,"{trace:?}");}
            else{assert_eq!(trace.status,render::terrain_trace::Status::AboveTerrainBounds,"{trace:?}");}}
        assert!(scene.terrain_visibility_complete);assert_eq!(height(600.0,0.0),30.0);assert_eq!(height(600.0,110.0),0.0);
        assert_eq!(scene.heightfield.heights.len(),(GW*GH)as usize);
        assert!(sample_pixels().iter().all(|&(x,y)|x<W&&y<H));
    }
}
