//! CPU-only native-terrain ray probes for an F12 grass report.
//! No scene/GPU is created. Ignores foreground objects and render CLOD;
//! distances locate the native terrain, not a claimed exact visible-pixel ID.
use glam::Vec3;
use serde_json::json;
use sideways::stage::{Stage, GRID, RUNOUT};

fn ground(stage: &Stage, x: f32, z: f32) -> f32 {
    let x0 = (x as f64 / GRID).floor() * GRID;
    let z0 = (z as f64 / GRID).floor() * GRID;
    let u = (x as f64 - x0) / GRID;
    let v = (z as f64 - z0) / GRID;
    let h = |x,z| stage.height(x,z) as f32 as f64;
    if u+v <= 1.0 {
        (h(x0,z0)*(1.0-u-v)+h(x0+GRID,z0)*u+h(x0,z0+GRID)*v) as f32
    } else {
        (h(x0+GRID,z0+GRID)*(u+v-1.0)+h(x0,z0+GRID)*(1.0-u)+h(x0+GRID,z0)*(1.0-v)) as f32
    }
}
fn main() {
    let path = std::env::args().nth(1).expect("F12 JSON path");
    let capture: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let seed = capture["seed"].as_u64().unwrap();
    let mut rng = gdcompat::Pcg::new(seed); rng.next_u32();
    let km = capture["state"]["active_settings"]["stage_km"].as_f64().unwrap();
    let stage = Stage::generate(&mut rng,km*1000.0+RUNOUT);
    assert_eq!(stage.noise_seed as i64,capture["stage_noise_seed"].as_i64().unwrap(),"only initial-stage captures supported");
    assert_eq!(stage.detail_seed as i64,capture["stage_detail_seed"].as_i64().unwrap());
    let camera = &capture["views"][0]["camera"];
    let vec3 = |v:&serde_json::Value| Vec3::new(v[0].as_f64().unwrap() as f32,v[1].as_f64().unwrap() as f32,v[2].as_f64().unwrap() as f32);
    let pos = vec3(&camera["position"]); let forward = (vec3(&camera["target"])-pos).normalize();
    let right = forward.cross(Vec3::Y).normalize(); let up = right.cross(forward);
    let size = &capture["capture"]["size"];
    let w = size[0].as_f64().unwrap() as f32; let h = size[1].as_f64().unwrap() as f32;
    let scale = (camera["fov_y"].as_f64().unwrap() as f32*0.5).tan();
    let mut samples = Vec::new();
    for [x,y] in [[1000,800],[1400,820],[1800,850],[2200,820],[2600,820],[1800,960],[1800,1050]] {
        let dir = (forward+right*((x as f32+0.5)/w*2.0-1.0)*scale*w/h+up*(1.0-(y as f32+0.5)/h*2.0)*scale).normalize();
        let clear = |t:f32| {let p=pos+dir*t;p.y-ground(&stage,p.x,p.z)};
        let mut previous = 0.1f32; let mut hit = None;
        for i in 1..=1600 {
            let t = i as f32*0.5;
            if clear(previous)>0.0 && clear(t)<=0.0 {
                let mut lo=previous; let mut hi=t;
                for _ in 0..18 {let mid=(lo+hi)*0.5;if clear(mid)>0.0{lo=mid}else{hi=mid}}
                let p=pos+dir*((lo+hi)*0.5); let d=(p-pos).xz().length();
                hit=Some(json!({"world":p.to_array(),"horizontal_distance_m":d,
                    "near_grass":if d<40.0{"opaque"}else if d<60.0{"transition"}else{"absent"},
                    "far_grass":if d<40.0{"absent"}else if d<160.0{"full"}else if d<240.0{"outer_fade"}else{"absent"}}));break;
            }
            previous=t;
        }
        samples.push(json!({"pixel":[x,y],"native_terrain":hit}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"capture":path,"limitation":"CPU native-height rays; ignores objects, CLOD and background extension","samples":samples})).unwrap());
}

use glam::Vec3Swizzles;
