//! Bounded acceptance fixture for the production roadside crowd path.
//! Run only with an exclusive GPU. --cpu-only never creates a GPU.
//! Characters are the procedural rigid-part placeholders, not finished art.
#![allow(dead_code)]
#[path="../src/car_look.rs"] mod car_look;
#[path="../src/car_model.rs"] mod car_model;
#[path="../src/distant_forest.rs"] mod distant_forest;
#[path="../src/ground_cover.rs"] mod ground_cover;
#[path="../src/grass_voxel_asset.rs"] mod grass_voxel_asset;
#[path="../src/grass_field.rs"] mod grass_field;
#[path="../src/live_ground_cover.rs"] mod live_ground_cover;
#[path="../src/far_ground_cover.rs"] mod far_ground_cover;
#[path="../src/procedural_rocks.rs"] mod procedural_rocks;
#[path="../src/roadside_crowds.rs"] mod roadside_crowds;
#[path="../src/tree_assets.rs"] mod tree_assets;
#[path="../src/voxel_tree_patch.rs"] mod voxel_tree_patch;
#[path="../src/visuals.rs"] mod visuals;

use glam::{Mat4,Vec3,Vec4};
use render::{Camera,Gpu,Lighting,Renderer,ViewRect,baked_animation::BakedAnimations,scene::Scene,upscale::UpscaleMode};
use serde_json::{Value,json};
use sideways::stage::{Stage,CHUNK};
use std::{collections::{BTreeSet,HashSet},path::{Path,PathBuf}};

const WIDTH:u32=1920;
const HEIGHT:u32=1080;
const LAST_FRAME:u32=15;

#[derive(Clone)]
struct Case {name:&'static str,camera:Camera,actors:Vec<usize>}

fn stage()->Stage {Stage::generate(&mut gdcompat::Pcg::new(1),1600.0)}
fn vec(p:gdcompat::math::V3)->Vec3 {Vec3::new(p.x,p.y,p.z)}
fn root(a:&BakedAnimations,i:usize)->Vec3 {a.actors[i].root.w_axis.truncate()}

fn cases(stage:&Stage,animation:&BakedAnimations)->Result<Vec<Case>,String> {
    let mut along:Vec<_>=animation.actors.iter().enumerate().map(|(i,a)| {
        let p=a.root.w_axis;let located=stage.locate(gdcompat::math::v3(p.x,p.y,p.z),-1);(located.along,i)
    }).collect();
    along.sort_by(|a,b|a.0.total_cmp(&b.0));
    let mut groups:Vec<Vec<usize>>=Vec::new();let mut previous=f64::NEG_INFINITY;
    for (at,i) in along {if at-previous>60.0 {groups.push(Vec::new());}groups.last_mut().unwrap().push(i);previous=at;}
    let close=groups.iter().find(|g|g.len()>=6 && g.iter().any(|i|animation.actors[*i].clip==0)
        && g.iter().any(|i|animation.actors[*i].clip==1)).ok_or("no supported mixed idle/cheer group")?;
    let large=groups.iter().max_by_key(|g|g.len()).ok_or("no crowd groups")?;
    if large.len()<16 {return Err(format!("fixture has no large group: largest={}",large.len()));}
    let setup=|name,group:&Vec<usize>,distance:f32,elevation:f32| {
        let center=group.iter().map(|i|root(animation,*i)).sum::<Vec3>()/group.len() as f32;
        let facing=group.iter().map(|i|animation.actors[*i].root.z_axis.truncate().normalize()).sum::<Vec3>().normalize();
        let target=center+Vec3::Y*1.0;
        let mut pos=target+facing*distance+Vec3::Y*elevation;let mut lift=0.0f32;
        // The opposing verge can rise above a nominal medium-distance camera.
        // Keep the sampled sightline above the same authoritative landscape;
        // never solve a blocked crowd image by moving spectators off support.
        for step in 0..=18 {let t=step as f32/20.0;let p=pos.lerp(target,t);
            let ground=stage.height(p.x as f64,p.z as f64) as f32;
            lift=lift.max((ground+0.35+0.65*(1.0-t)-p.y)/(1.0-t));
        }
        pos.y+=lift;
        Case {name,camera:Camera {pos,target,fov_y:58.0f32.to_radians(),near:0.1},actors:group.clone()}
    };
    Ok(vec![setup("close",close,8.0,1.0),setup("medium",close,24.0,5.0),setup("large",large,27.0,8.0)])
}

fn cpu_scene(stage:&Stage)->Result<Scene,String> {
    let mut cells=BTreeSet::new();
    for along in (0..1600).step_by(20) {let p=stage.point_at(along as f64);
        let cell=((p.x/CHUNK as f32).floor() as i32,(p.z/CHUNK as f32).floor() as i32);
        for dx in -1..=1 {for dz in -1..=1 {cells.insert((cell.0+dx,cell.1+dz));}}
    }
    let chunks:Vec<_>=cells.into_iter().map(|c|sideways::builder::terrain_chunk(stage,c)).collect();
    let mut scene=Scene::default();let mut names=Vec::new();
    roadside_crowds::append(&mut scene,stage,&chunks,&mut names)?;Ok(scene)
}

fn case_json(c:&Case)->Value {json!({"name":c.name,"actors":c.actors.len(),"camera":{"position":c.camera.pos.to_array(),
    "target":c.camera.target.to_array(),"fov_y":c.camera.fov_y,"near":c.camera.near}})}

fn read_parts(gpu:&Gpu,renderer:&Renderer,a:&BakedAnimations)->Result<(usize,Vec<u8>),String> {
    let source=renderer.baked_animation_debug_instances().ok_or("baked actor GPU buffer missing")?;
    let first=a.parts.iter().map(|p|p.owner as usize).min().ok_or("no parts")?;
    let end=a.parts.iter().map(|p|p.owner as usize).max().unwrap()+1;
    let stride=std::mem::size_of::<render::scene::GpuInstance>();
    if stride!=80 {return Err("GPU instance ABI changed; update independent readback decoder".into());}
    let size=((end-first)*stride) as u64;
    let target=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("crowd validation only"),size,
        usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source,(first*stride)as u64,&target,0,size);gpu.queue.submit([encoder.finish()]);
    let(tx,rx)=std::sync::mpsc::channel();target.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});gpu.wait();
    rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let bytes=target.slice(..).get_mapped_range().map_err(|e|e.to_string())?.to_vec();target.unmap();Ok((first,bytes))
}
fn float(bytes:&[u8],offset:usize)->f32 {f32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap())}
fn matrix(bytes:&[u8],offset:usize)->Mat4 {
    let r:[[f32;4];3]=std::array::from_fn(|i|std::array::from_fn(|j|float(bytes,offset+i*16+j*4)));
    Mat4::from_cols(Vec4::new(r[0][0],r[1][0],r[2][0],0.0),Vec4::new(r[0][1],r[1][1],r[2][1],0.0),
        Vec4::new(r[0][2],r[1][2],r[2][2],0.0),Vec4::new(r[0][3],r[1][3],r[2][3],1.0))
}
fn error(a:Mat4,b:Mat4)->f32 {a.to_cols_array().into_iter().zip(b.to_cols_array()).map(|(a,b)|(a-b).abs()).fold(0.0,f32::max)}

fn verify_poses(gpu:&Gpu,renderer:&Renderer,scene:&Scene,seconds:f32)->Result<Value,String> {
    let a=scene.baked_animations.as_ref().unwrap();let(first,bytes)=read_parts(gpu,renderer,a)?;
    let(mut pose,mut sphere,mut feet)=(0.0f32,0.0f32,0.0f32);let mut finite=true;let mut identity=true;
    for part in &a.parts {let offset=(part.owner as usize-first)*80;let actual=matrix(&bytes,offset);
        let expected=a.sample(part.actor as usize,part.bone,seconds);
        finite&=actual.is_finite();pose=pose.max(error(actual,expected));
        let center=Vec3::new(float(&bytes,offset+48),float(&bytes,offset+52),float(&bytes,offset+56));
        let expected_center=expected.transform_point3(Vec3::from_slice(&part.local_sphere[..3]));
        let radius=float(&bytes,offset+60);let scale=a.actors[part.actor as usize].root.x_axis.truncate().length();
        sphere=sphere.max(center.distance(expected_center)).max((radius-part.local_sphere[3]*scale).abs());
        finite&=center.is_finite()&&radius.is_finite();
        let original=&scene.instances[part.owner as usize];
        for (field,wanted) in [(64,original.first_cluster),(68,original.cluster_count),(76,original.flags)] {
            identity&=u32::from_le_bytes(bytes[offset+field..offset+field+4].try_into().unwrap())==wanted;
        }
        if [11,14].contains(&part.bone) {feet=feet.max(error(actual,a.sample(part.actor as usize,part.bone,0.0)));}
    }
    Ok(json!({"time_seconds":seconds,"parts":a.parts.len(),"passed":finite&&identity&&pose<0.001&&sphere<0.001&&feet<0.001,
        "max_matrix_component_error":pose,"max_sphere_error_m":sphere,"max_planted_foot_matrix_error":feet,
        "finite":finite,"instance_identity_preserved":identity,"oracle":"actual GPU instance buffer versus independent CPU quaternion sample"}))
}

fn picks(case:&Case,a:&BakedAnimations,time:f32)->Vec<(u32,u32)> {
    let cam=&case.camera;
    let vp=Mat4::perspective_rh(cam.fov_y,WIDTH as f32/HEIGHT as f32,cam.near,2000.0)*Mat4::look_at_rh(cam.pos,cam.target,Vec3::Y);
    case.actors.iter().filter_map(|i| {
        let p=a.sample(*i,0,time).w_axis;let q=vp*p;if q.w<=0.0{return None;}
        let q=q.truncate()/q.w;let x=(q.x*0.5+0.5)*WIDTH as f32;let y=(0.5-q.y*0.5)*HEIGHT as f32;
        (x>=0.0&&y>=0.0&&x<WIDTH as f32&&y<HEIGHT as f32).then_some((x as u32,y as u32))
    }).collect()
}

fn shot(gpu:&Gpu,renderer:&Renderer,scene:&Scene,light:&Lighting,c:&Case,mode:UpscaleMode,time:f32,path:&Path)->Result<Value,String> {
    let a=scene.baked_animations.as_ref().unwrap();let selected=picks(c,a,time);
    let view=[ViewRect {cam:c.camera.clone(),x:0,y:0,w:WIDTH,h:HEIGHT}];
    let(pixels,stats)=renderer.try_render_views(gpu,&view,light,WIDTH,HEIGHT,&selected)?;
    render::write_png(path,WIDTH,HEIGHT,&pixels)?;
    if stats.upscale.mode!=mode || stats.upscale.views.len()!=1 {return Err("requested upscaler was not evaluated".into());}
    let owners:HashSet<_>=a.parts.iter().map(|p|p.owner).collect();
    let visible=stats.picks.iter().flatten().filter(|(owner,_,_)|owners.contains(owner)).count();
    Ok(json!({"image":path,"case":c.name,"mode":mode.as_str(),"time_seconds":time,"gpu_pass_ms":stats.gpu_ms,
        "size":[WIDTH,HEIGHT],"input_size":stats.upscale.views[0].input,"output_size":stats.upscale.views[0].output,
        "projected_torso_picks":selected.len(),"actual_crowd_hits":visible,"triangles":stats.triangles,
        "shadow_triangles":stats.shadow_triangles,"poses":verify_poses(gpu,renderer,scene,time)?}))
}

fn run(out:&Path,cpu_only:bool,four_views:bool)->Result<(),String> {
    if out.exists(){return Err("output directory must be fresh".into());}std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
    let stage=stage();
    if cpu_only {
        let scene=cpu_scene(&stage)?;let a=scene.baked_animations.as_ref().ok_or("no crowd actor pool")?;
        a.validate(&scene.instances)?;let c=cases(&stage,a)?;
        let report=json!({"scope":"CPU only; no GPU created","crowds":roadside_crowds::metadata(&scene),"cases":c.iter().map(case_json).collect::<Vec<_>>()});
        std::fs::write(out.join("cpu.json"),serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;
        println!("{report}");return Ok(());
    }
    let looks=car_look::Looks::load(None,vec![])?;
    let mut built=visuals::build_live(&stage,&[],&[],&looks)?;
    for line in &built.log {println!("{line}");}
    let cases=cases(&stage,built.scene.baked_animations.as_ref().ok_or("production scene has no crowd actor pool")?)?;
    built.update_ground_cover(&cases.iter().map(|c|c.camera.pos).collect::<Vec<_>>());
    let gpu=Gpu::new()?;let light=Lighting {taa:0.0,..Default::default()};let renderer=Renderer::new(&gpu,&built.scene,&light);
    renderer.set_frame_time_ms(1000.0/60.0);
    // Freeze vegetation so the actor-only sequence really isolates actors.
    renderer.set_wind(&gpu,&render::wind::WindState {strength:0.0,..Default::default()})?;
    let mut images=Vec::new();let mut split=Vec::new();let mut passed=true;
    for mode in [UpscaleMode::Native,UpscaleMode::DlssQuality] {
        renderer.set_upscale(&gpu,mode)?;
        for c in &cases {
            renderer.reset_history();renderer.set_animation_time(0.0)?;
            let views=[ViewRect {cam:c.camera.clone(),x:0,y:0,w:WIDTH,h:HEIGHT}];
            for _ in 0..8 {renderer.try_render_texture(&gpu,&views,&light,WIDTH,HEIGHT)?;}gpu.wait();
            for frame in 0..=LAST_FRAME {
                let time=frame as f32/60.0;renderer.set_animation_time(time as f64)?;
                if frame==0 || frame==LAST_FRAME || (c.name=="close"&&frame==8) {
                    let path=out.join(format!("{}-{}-actor-{frame:02}.png",mode.as_str(),c.name));
                    let row=shot(&gpu,&renderer,&built.scene,&light,c,mode,time,&path)?;
                    passed&=row["poses"]["passed"]==true && row["actual_crowd_hits"].as_u64().unwrap_or(0)>0;images.push(row);
                } else {renderer.try_render_texture(&gpu,&views,&light,WIDTH,HEIGHT)?;}
            }
        }
        // Hold actor poses and translate only the close camera. History stays
        // live across the 16 frames, exercising rigid screen motion in DLSS.
        let start=&cases[0];renderer.reset_history();renderer.set_animation_time(0.25)?;
        let side=(start.camera.target-start.camera.pos).cross(Vec3::Y).normalize();
        for frame in 0..=LAST_FRAME {
            let mut c=start.clone();let shift=side*(frame as f32/LAST_FRAME as f32*0.55);
            c.camera.pos+=shift;c.camera.target+=shift;
            if frame==0||frame==8||frame==LAST_FRAME {
                let row=shot(&gpu,&renderer,&built.scene,&light,&c,mode,0.25,&out.join(format!("{}-close-camera-{frame:02}.png",mode.as_str())))?;
                passed&=row["poses"]["passed"]==true;images.push(row);
            }else{renderer.try_render_texture(&gpu,&[ViewRect {cam:c.camera,x:0,y:0,w:WIDTH,h:HEIGHT}],&light,WIDTH,HEIGHT)?;}
        }
        if four_views {
            renderer.reset_history();renderer.set_animation_time(0.25)?;
            let views:Vec<_>=[0,1,2,0].into_iter().enumerate().map(|(i,c)|ViewRect {cam:cases[c].camera.clone(),
                x:(i as u32%2)*WIDTH,y:(i as u32/2)*HEIGHT,w:WIDTH,h:HEIGHT}).collect();
            for _ in 0..4 {renderer.try_render_texture(&gpu,&views,&light,WIDTH*2,HEIGHT*2)?;}gpu.wait();
            let(pixels,stats)=renderer.try_render_views(&gpu,&views,&light,WIDTH*2,HEIGHT*2,&[])?;
            let path=out.join(format!("{}-four-views.png",mode.as_str()));render::write_png(&path,WIDTH*2,HEIGHT*2,&pixels)?;
            let poses=verify_poses(&gpu,&renderer,&built.scene,0.25)?;passed&=poses["passed"]==true&&stats.upscale.views.len()==4;
            split.push(json!({"image":path,"mode":mode.as_str(),"size":[WIDTH*2,HEIGHT*2],"views":stats.upscale.views.iter().map(|v|json!({"input":v.input,"output":v.output})).collect::<Vec<_>>(),
                "poses":poses,"shared_world_actors":built.scene.baked_animations.as_ref().unwrap().actors.len(),"gpu_pass_ms":stats.gpu_ms}));
        }
    }
    let report=json!({"schema":1,"numeric_checks_passed":passed,"visual_review":"pending human/model image inspection",
        "adapter":gpu.info.name,"crowds":roadside_crowds::metadata(&built.scene),"stage":{"seed":1,"length_m":1600,
            "noise_seed":stage.noise_seed,"detail_seed":stage.detail_seed},"cases":cases.iter().map(case_json).collect::<Vec<_>>(),
        "images":images,"four_views":split,"limitations":["procedural rigid placeholder characters","no cars or HUD in fixture",
            "16-frame sequences are short diagnostic evidence, not a route benchmark","GPU pose correspondence is checked; motion texture values are not independently read back",
            "held-camera pass timings include the whole scene; no paired crowd-off cost measurement","4K/four-player/120FPS is not established"]});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;
    println!("crowd GPU report: {}",out.join("report.json").display());
    if !passed{return Err("crowd GPU numeric/visibility acceptance failed; inspect report and images".into());}Ok(())
}

fn main() {
    // This executable owns its environment, before any workers or GPU exist.
    unsafe {std::env::set_var("SIDEWAYS_CROWDS","1");}
    let args:Vec<_>=std::env::args().skip(1).collect();
    let result=(||{let out=args.first().ok_or("usage: crowds_gpu FRESH_DIR [--cpu-only] [--four-views]")?;
        if args.iter().skip(1).any(|a|a!="--cpu-only"&&a!="--four-views"){return Err("unknown crowds fixture argument".into());}
        run(&PathBuf::from(out),args.iter().any(|a|a=="--cpu-only"),args.iter().any(|a|a=="--four-views"))})();
    if let Err(error)=result {eprintln!("crowd fixture: {error}");std::process::exit(1);}
}
