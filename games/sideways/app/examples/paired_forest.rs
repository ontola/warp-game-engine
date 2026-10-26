//! Same forest/camera, source or CLOD geometry, real maps or distant canopy.
//! A controlled native diagnostic, not a voxel scene or gameplay benchmark.
//! Usage: paired_forest FRESH_DIR [--cpu-only] [--full] [--dense] [--coarse]
//! Canopy bounds/support flags are read by the production renderer; use one
//! process per combination. --full adds the two crossed geometry/shadow cases.
#[path = "../src/tree_assets.rs"] mod tree_assets;

use glam::{Affine3A, Quat, Vec3};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use render::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex};
use render::scene::{Heightfield, Material, ALPHA_TEST, TWO_SIDED};
use serde_json::{json, Value};
use sideways::vegetation::TreeArchetype;
use std::path::{Path, PathBuf};

const W:u32=1280;
const H:u32=720;
const FLOOR_HALF:f32=85.;
const MEASURE_HALF:f32=43.;

fn sun()->Vec3 {
    let (el,az)=(60f32.to_radians(),125f32.to_radians());
    Vec3::new(az.sin()*el.cos(),el.sin(),-az.cos()*el.cos())
}

fn camera(coarse:bool)->Camera {
    // The forest is >96m from the ordinary sun-map centre along light-space X,
    // even at its nearest edge. It lies inside the medium canopy clipmap.
    let across=Vec3::Y.cross(sun()).normalize();
    let target=Vec3::Y*7.;let ratio=if coarse {2.5}else{1.};
    Camera {pos:target+(across*260.+Vec3::Y*78.-target)*ratio,target,
        fov_y:2.*((12f32.to_radians()).tan()/ratio).atan(),near:0.1}
}

fn placements(dense:bool)->Vec<(usize,Affine3A)> {
    let (cols,rows,pitch)=if dense {(12,8,5.)}else{(6,4,10.)};
    (0..rows).flat_map(|row|(0..cols).map(move|col| {
        let index=row*cols+col;
        let yaw=(index as f32*2.3999631).rem_euclid(std::f32::consts::TAU);
        let x=(col as f32-(cols-1) as f32*0.5)*pitch+(index as f32*1.7).sin()*1.8;
        let z=(row as f32-(rows-1) as f32*0.5)*pitch+(index as f32*2.1).cos()*1.8;
        let scale=0.65+0.13*((index*7)%4) as f32;
        (index%3,Affine3A::from_scale_rotation_translation(Vec3::splat(scale),Quat::from_rotation_y(yaw),Vec3::new(x,0.,z)))
    })).collect()
}

fn scene(root:&Path,dense:bool)->Result<(Scene,Value),String> {
    let mut scene=Scene::default();
    // Neutral, untextured receiving ground makes shadow support measurable.
    // Same physical flat ground is supplied to rasterisation and canopy rays.
    let floor_mat=scene.add_material(Material {tint:[0.42;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
    let verts:Vec<_>=[[-FLOOR_HALF,0.,FLOOR_HALF],[FLOOR_HALF,0.,FLOOR_HALF],
        [FLOOR_HALF,0.,-FLOOR_HALF],[-FLOOR_HALF,0.,-FLOOR_HALF]]
        .into_iter().map(|pos|Vertex {pos,normal:[0.,1.,0.],uv:[0.;2]}).collect();
    let floor=scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&verts,indices:&[0,1,2,0,2,3],material:floor_mat}]));
    scene.add_instance(floor,Affine3A::IDENTITY);
    scene.heightfield=Heightfield {origin:[-512.;2],cell:4.,w:257,h:257,heights:vec![0.;257*257]};
    let materials=[("trunk_a_albedo.jpg",0,[1.;3]),("trunk_b_albedo.jpg",0,[1.;3]),
        ("bark_albedo.jpg",0,[1.;3]),("authored_pine_albedo.png",ALPHA_TEST|TWO_SIDED,[0.62,0.66,0.55])]
        .into_iter().map(|(file,flags,tint)| {
            let layer=scene.add_texture(&root.join("assets/trees").join(file))?;
            Ok(scene.add_material(Material {tint,layer,roughness:0.9,flags,scale:1.,metal:0.}))
        }).collect::<Result<Vec<_>,String>>()?;
    let mut meshes=Vec::new();let mut source=Vec::new();
    for archetype in TreeArchetype::ALL {
        let parts:Vec<_>=tree_assets::load(root,archetype)?.into_iter().map(|(name,v,i)| {
            let mat=if name.contains("trunk_b") {materials[1]} else if name.contains("trunk") {materials[0]}
                else if name.contains("bark") {materials[2]} else {materials[3]};
            (v,i,mat)
        }).collect();
        let refs:Vec<_>=parts.iter().map(|(v,i,material)|Part {vertices:v,indices:i,material:*material}).collect();
        let mesh=ClusterMesh::build_with_policy(&refs,|part|if part.material==materials[3] {
            SimplificationPolicy::AlphaCards
        } else {SimplificationPolicy::TexturedGeometry});
        source.push(json!({"archetype":archetype.label(),"source_triangles":mesh.leaf_triangles(),
            "root_triangles":mesh.root_triangles(),"clusters":mesh.clusters.len(),"sphere":mesh.sphere}));
        meshes.push(scene.add_mesh(&mesh));
    }
    for (variant,xf) in placements(dense) {scene.add_instance(meshes[variant],xf);scene.mark_foliage();}
    let instances:Vec<_>=scene.instances.iter().skip(1).map(|i|json!({"rows":i.rows,"sphere":i.sphere,"flags":i.flags})).collect();
    Ok((scene,json!({"source":source,"instances":instances})))
}

fn ground_point(cam:&Camera,x:u32,y:u32)->Option<Vec3> {
    let f=(cam.target-cam.pos).normalize();let r=f.cross(Vec3::Y).normalize();let u=r.cross(f);
    let sy=(1.-2.*(y as f32+0.5)/H as f32)*(cam.fov_y*0.5).tan();
    let sx=(2.*(x as f32+0.5)/W as f32-1.)*(W as f32/H as f32)*(cam.fov_y*0.5).tan();
    let ray=f+r*sx+u*sy;
    if ray.y>=-1e-6{return None;}
    let p=cam.pos+ray*(-cam.pos.y/ray.y);
    (p.x.abs()<MEASURE_HALF && p.z.abs()<MEASURE_HALF).then_some(p)
}

struct Case {name:&'static str,source:bool,real:bool}
struct ResultRow {name:&'static str,ground:Vec<bool>,crowns:Vec<Option<u32>>,sun:Vec<u8>,sky:Vec<u8>,lit:Vec<[u8;3]>,direct:Vec<[u8;3]>}

fn flags()->Value {
    let names=["KERN_CANOPY_COMPACT_SUPPORT","KERN_SOURCE_CROWN_BOUNDS","KERN_SUN_RECEIVER_PLANE",
        "KERN_SUN_SCENE_DEPTH","KERN_SUN_TERRAIN_ERROR","KERN_SUN_SURFACE_SOURCE","KERN_FAR_SUN_SHADOW"];
    names.into_iter().map(|key|(key.to_string(),json!(std::env::var(key).ok().as_deref()==Some("1"))))
        .collect::<serde_json::Map<_,_>>().into()
}

fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().skip(1).collect();
    let out=PathBuf::from(args.first().ok_or("FRESH_DIR required")?);
    if out.exists(){return Err("output directory must be fresh".into());}
    for arg in args.iter().skip(1) {if !matches!(arg.as_str(),"--cpu-only"|"--full"|"--dense"|"--coarse") {return Err(format!("unknown argument {arg}"));}}
    if std::env::var("KERN_LEGACY_LIGHT_LOOKAHEAD").ok().as_deref()==Some("1") {
        return Err("legacy lookahead would invalidate the shadow-focus isolation".into());
    }
    let root=sideways::game_assets::root();
    let dense=args.iter().any(|a|a=="--dense");let coarse=args.iter().any(|a|a=="--coarse");
    let (mut scene,assets)=scene(&root,dense)?;
    let cam=camera(coarse);
    let mut light=Lighting {sun_dir:sun(),fog_density:0.,fog_culling:false,gi:1.,contact_occlusion:0.,
        reflection_occlusion:0.,taa:0.,sharpen:0.,shadow_strength:1.,near_shadows:1.,..Default::default()};
    let sky=render::atmosphere::AtmosphereCache::default().get_or_bake(&Default::default(),
        render::atmosphere::SolarState::from_degrees(60.,125.)?,Default::default())?;
    sky.apply(&mut scene,&mut light);
    // Include actual crown pixels, including those whose ray misses the floor
    // measurement area. Classify source material from the GPU visibility hit.
    let picks:Vec<_>=(2..H).step_by(4).flat_map(|y|(2..W).step_by(4).map(move|x|(x,y))).collect();
    let mut report=json!({"scope":"Same source-tree instances and native camera; source CLOD versus normal CLOD, real shadow maps versus current approximate canopy. NOT voxel trees, F12 replay, gameplay FPS, or optical ground truth.",
        "tree_count":scene.instances.len()-1,"dense":dense,"coarse":coarse,
        "size":[W,H],"camera":{"position":cam.pos.to_array(),"target":cam.target.to_array(),"fov_y":cam.fov_y},
        "sun_elevation_azimuth":[60,125],"sun_direction":sun().to_array(),"wind_strength":0,"canopy_focus":cam.pos.to_array(),
        "shadow_focus_real":[0,0,0],"shadow_focus_approx":cam.pos.to_array(),"flags":flags(),"assets":assets,
        "measurement":"4px lattice. Ground: common visible neutral ground within43m of centre. Crowns: same-instance hits on ALPHA_TEST materials. Metrics are tonemapped bytes, NOT linear transmittance. GI stays1 and canopy anchor fixed in all cases.",
        "sample_pixels":picks,"cases":[],"comparisons":[],"cpu_only":args.iter().any(|a|a=="--cpu-only")});
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    if args.iter().any(|a|a=="--cpu-only") {
        std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;
        println!("paired forest CPU scene ready: {} samples,{} trees; no GPU created",picks.len(),scene.instances.len()-1);return Ok(());
    }
    let gpu=Gpu::new()?;
    let renderer=Renderer::new(&gpu,&scene,&light);
    renderer.set_lighting_focus(&[cam.pos])?;
    renderer.set_wind(&gpu,&Default::default())?;
    let mut cases=vec![Case {name:"source-real",source:true,real:true},Case {name:"clod-approx",source:false,real:false}];
    if args.iter().any(|a|a=="--full") {cases.extend([Case {name:"source-approx",source:true,real:false},Case {name:"clod-real",source:false,real:true}]);}
    let mut rows=Vec::new();
    for case in cases {
        renderer.set_sun_shadow_focus(&[if case.real {Vec3::ZERO}else{cam.pos}])?;
        light.lod_pixels=if case.source {0.}else{1.};
        let mut row=ResultRow {name:case.name,ground:vec![false;picks.len()],crowns:vec![None;picks.len()],
            sun:Vec::new(),sky:Vec::new(),lit:Vec::new(),direct:Vec::new()};
        for (label,debug) in [("lit",0.),("sun",9.),("sky",1.),("direct",3.)] {
            light.debug=debug;
            let (pixels,stats)=renderer.render_picking(&gpu,&cam,&light,W,H,&picks);
            render::write_png(&out.join(format!("{}-{label}.png",case.name)),W,H,&pixels)?;
            let bytes:Vec<_>=picks.iter().map(|(x,y)|pixels[((y*W+x)*4) as usize]).collect();
            let rgb=||picks.iter().map(|(x,y)| {let i=((y*W+x)*4) as usize;[pixels[i],pixels[i+1],pixels[i+2]]}).collect();
            if label=="sun" {
                row.ground=stats.picks.iter().zip(&picks).map(|(p,(x,y))|p.is_some_and(|v|v.0==0)&&ground_point(&cam,*x,*y).is_some()).collect();
                row.crowns=stats.picks.iter().map(|p|p.and_then(|(instance,cluster,_)| {
                    let mat=scene.materials[scene.clusters[cluster as usize].material as usize];
                    (mat.flags&ALPHA_TEST!=0).then_some(instance)
                })).collect();row.sun=bytes;
            }
            else if label=="sky" {row.sky=bytes;}
            else if label=="lit" {row.lit=rgb();}
            else if label=="direct" {row.direct=rgb();}
            report["cases"].as_array_mut().unwrap().push(json!({"name":case.name,"component":label,
                "source_lod":case.source,"real_maps":case.real,"triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles,
                "canopy_pitch_m":stats.canopy_pitch_m,"canopy_updated_levels":stats.canopy_updated_levels,
                "gpu_ms":stats.gpu_ms.iter().map(|(name,value)|json!({"pass":name,"ms":value})).collect::<Vec<_>>()}));
        }
        rows.push(row);
    }
    for row in rows.iter().skip(1) {
        let reference=&rows[0];
        let indices:Vec<_>=(0..picks.len()).filter(|i|reference.ground[*i]&&row.ground[*i]).collect();
        let n=indices.len() as f64;
        let mean=|bytes:&[u8]|indices.iter().map(|i|bytes[*i] as f64).sum::<f64>()/n.max(1.);
        let delta:Vec<_>=indices.iter().map(|i|row.sun[*i] as f64-reference.sun[*i] as f64).collect();
        let sky_changed=indices.iter().filter(|i|row.sky[**i]!=reference.sky[**i]).count();
        report["comparisons"].as_array_mut().unwrap().push(json!({"reference":reference.name,"candidate":row.name,
            "common_ground_samples":indices.len(),"reference_sun_mean_byte":mean(&reference.sun),"candidate_sun_mean_byte":mean(&row.sun),
            "sun_rmse_byte":(delta.iter().map(|d|d*d).sum::<f64>()/n.max(1.)).sqrt(),
            "darker_by_over20_bytes":delta.iter().filter(|d|**d< -20.).count(),"lighter_by_over20_bytes":delta.iter().filter(|d|**d>20.).count(),
            "sky_changed_samples":sky_changed,"sky_should_be_identical":true}));
        let crown_ids:Vec<_>=(0..picks.len()).filter(|i|reference.crowns[*i].is_some()&&reference.crowns[*i]==row.crowns[*i]).collect();
        let cn=crown_ids.len().max(1) as f64;
        let crown_mean=|bytes:&[u8]|crown_ids.iter().map(|i|bytes[*i] as f64).sum::<f64>()/cn;
        let crown_rgb_mean=|bytes:&[[u8;3]]|std::array::from_fn::<_,3,_>(|channel|crown_ids.iter().map(|i|bytes[*i][channel] as f64).sum::<f64>()/cn);
        report["comparisons"].as_array_mut().unwrap().last_mut().unwrap()["crowns"]=json!({"same_instance_alpha_samples":crown_ids.len(),
            "reference_sun_mean_byte":crown_mean(&reference.sun),"candidate_sun_mean_byte":crown_mean(&row.sun),
            "reference_sky_mean_byte":crown_mean(&reference.sky),"candidate_sky_mean_byte":crown_mean(&row.sky),
            "reference_lit_rgb_mean":crown_rgb_mean(&reference.lit),"candidate_lit_rgb_mean":crown_rgb_mean(&row.lit),
            "reference_direct_rgb_mean":crown_rgb_mean(&reference.direct),"candidate_direct_rgb_mean":crown_rgb_mean(&row.direct),
            "sun_darker_over20":crown_ids.iter().filter(|i|i32::from(row.sun[**i])-i32::from(reference.sun[**i])< -20).count(),
            "sun_lighter_over20":crown_ids.iter().filter(|i|i32::from(row.sun[**i])-i32::from(reference.sun[**i])>20).count(),
            "sun_rmse_byte":(crown_ids.iter().map(|i|{let d=row.sun[*i] as f64-reference.sun[*i] as f64;d*d}).sum::<f64>()/cn).sqrt(),
            "sky_changed_samples":crown_ids.iter().filter(|i|row.sky[**i]!=reference.sky[**i]).count(),
            "identical_geometry":row.name=="source-approx"});
    }
    report["ground_samples"]=json!(rows.iter().map(|row|json!({"name":row.name,"visible":row.ground,"crown_owner":row.crowns,
        "sun_byte":row.sun,"sky_byte":row.sky,"lit_rgb":row.lit,"direct_rgb":row.direct})).collect::<Vec<_>>());
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;
    println!("paired forest diagnostic captured; inspect report and matched images (no automatic visual acceptance)");
    Ok(())
}

fn main(){if let Err(e)=run(){eprintln!("paired forest: {e}");std::process::exit(1);}}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn fixed_pair_keeps_forest_outside_ordinary_shadow_coverage() {
        let cam=camera(false);let axis=Vec3::Y.cross(sun()).normalize();
        for (_,xf) in placements(false) {
            let p=Vec3::from(xf.translation);
            assert!((p-cam.pos).dot(axis).abs()>220.);
            assert!(p.x.abs()<35.&&p.z.abs()<25.);
        }
        assert_eq!(placements(false).len(),24);assert_eq!(placements(true).len(),96);
        assert!(ground_point(&cam,W/2,H/2).is_some());
        assert!((camera(true).pos-camera(true).target).length()>(cam.pos-cam.target).length()*2.4);
    }
}
