//! Rebuild the current scene at a verified F12 stage/camera and applied sky.
//! Diagnostic views omit cars and HUD; they are not gameplay/performance claims.
//! `--cpu-only` checks the stage/camera contract without creating a GPU.
#![allow(dead_code)]
#[path="../src/car_look.rs"] mod car_look;
#[path="../src/car_model.rs"] mod car_model;
#[path="../src/distant_forest.rs"] mod distant_forest;
#[path="../src/ground_cover.rs"] mod ground_cover;
#[path="../src/grass_voxel_asset.rs"] mod grass_voxel_asset;
#[path="../src/live_ground_cover.rs"] mod live_ground_cover;
#[path="../src/far_ground_cover.rs"] mod far_ground_cover;
#[path="../src/procedural_rocks.rs"] mod procedural_rocks;
#[path="../src/roadside_crowds.rs"] mod roadside_crowds;
#[path="../src/tree_assets.rs"] mod tree_assets;
#[path="../src/voxel_tree_patch.rs"] mod voxel_tree_patch;
#[path="../src/visuals.rs"] mod visuals;

use glam::{DVec3, Vec2, Vec3};
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, BakedSky, SolarState};
use render::{Camera, Gpu, Lighting, Renderer, ViewRect};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn number(v:&Value)->Result<f32,String> {
    let f=v.as_f64().ok_or("missing numeric capture field")? as f32;
    if !f.is_finite(){return Err("nonfinite capture value".into());} Ok(f)
}
fn vector(v:&Value)->Result<Vec3,String> {
    Ok(Vec3::new(number(&v[0])?,number(&v[1])?,number(&v[2])?))
}

fn captured_headlights(capture:&Value)->Result<Vec<render::headlights::SpotLight>,String> {
    let lamps=&capture["render"]["headlights"];
    // Older daytime captures predate spotlights. Never invent lamps for them.
    if lamps.is_null(){return Ok(Vec::new());}
    let rows=lamps["lights"].as_array().ok_or("captured headlights lack light list")?;
    if lamps["enabled"].as_bool()!=Some(!rows.is_empty()) {return Err("inconsistent captured headlight enable state".into());}
    let lights=rows.iter().map(|l|Ok(render::headlights::SpotLight {
        position:vector(&l["position"])?,direction:vector(&l["direction"])?,up:vector(&l["up"])?,
        intensity_cd:vector(&l["intensity_cd"])?,range_m:number(&l["range_m"])?,
        inner_angle_radians:number(&l["inner_angle_radians"])?,outer_angle_radians:number(&l["outer_angle_radians"])?
    })).collect::<Result<Vec<_>,String>>()?;
    render::headlights::validate(&lights)
}

struct ReplaySky {baked:Arc<BakedSky>,metadata:Value}

/// F12 v1 records the applied preset and baked RGB, not all coefficients.
/// Freeze that model here and verify its actual output; do not silently use
/// future engine defaults or the UI's possibly pending desired settings.
fn sky_spec(capture:&Value)->Result<Option<(Atmosphere,SolarState,BakeQuality,Option<Vec3>)>,String> {
    let s=&capture["render"]["sky"];
    match s["mode"].as_str() {
        None|Some("hdri")=>return Ok(None),
        Some("procedural_single_scattering")=>{},
        Some(mode)=>return Err(format!("unsupported captured sky mode: {mode}")),
    }
    let (scale,height)=match s["atmosphere"].as_str() {
        Some("clear")=>(1.0,1.2),Some("haze")=>(4.0,1.2),Some("mist")=>(14.0,0.22),
        None if s["transmittance_lut"]==json!([128,64])=>(1.0,1.2),
        _=>return Err("unsupported or missing captured atmosphere preset".into()),
    };
    let model=Atmosphere {planet_radius_km:6360.0,height_km:100.0,observer_height_m:2.0,
        rayleigh_per_km:DVec3::new(0.005802,0.013558,0.033100),rayleigh_height_km:8.0,
        mie_scattering_per_km:0.003996*scale,mie_absorption_per_km:0.000444*scale,
        mie_height_km:height,mie_g:0.8,ozone_per_km:DVec3::new(0.000650,0.001881,0.000085),
        ground_albedo:DVec3::splat(0.15),solar_rgb_lux:DVec3::splat(127_500.0)};
    let quality=BakeQuality {transmittance_width:128,transmittance_height:64,transmittance_steps:96,
        sky_width:256,sky_height:128,sky_steps:48};
    if s["sky_lut"]!=json!([256,128])
        || (!s["transmittance_lut"].is_null() && s["transmittance_lut"]!=json!([128,64])) {
        return Err("unsupported captured sky LUT quality; integration step counts were not recorded in F12 v1".into());
    }
    let solar=SolarState::from_degrees(s["solar_elevation_degrees"].as_f64().ok_or("missing solar elevation")?,
        s["solar_azimuth_degrees"].as_f64().ok_or("missing solar azimuth")?)?;
    if !solar.direction().as_vec3().abs_diff_eq(vector(&capture["render"]["sun_direction"])?,1e-5) {
        return Err("capture sky and rendered sun directions disagree".into());
    }
    let applied=&capture["state"]["live_sky_controls"]["applied"]["sky"];
    if !applied.is_null() && applied!=s {return Err("capture render sky differs from applied live sky metadata".into());}
    let fog=if !s["fog_start_end_density"].is_null() {
        let f=vector(&s["fog_start_end_density"])?;
        if f.x<0.0 || f.y<f.x || f.z<0.0 {return Err("invalid captured sky fog range/density".into());}
        Some(f)
    } else if s["atmosphere"].is_null() && applied.is_null() {
        // The original environment opt-in did not change play's distance fog.
        None
    } else {return Err("live procedural capture lacks its applied fog values".into());};
    model.validate()?;quality.validate()?;Ok(Some((model,solar,quality,fog)))
}

fn prepare_sky(capture:&Value,light:&mut Lighting)->Result<Option<ReplaySky>,String> {
    let Some((model,solar,quality,fog))=sky_spec(capture)? else{return Ok(None);};
    let baked=AtmosphereCache::default().get_or_bake(&model,solar,quality)?;
    baked.validate()?;
    let s=&capture["render"]["sky"];
    for (key,actual) in [("sun_rgb_lux",baked.sun_rgb_lux),("sky_horizontal_rgb_lux",baked.sky_horizontal_rgb_lux)] {
        let expected=vector(&s[key])?;
        if expected.min_element()<0.0 || (actual-expected).abs().max_element()>0.02+expected.abs().max_element()*2e-6 {
            return Err(format!("captured {key} differs from reconstructed v1 atmosphere; refusing an approximate sky"));
        }
    }
    let mut physical=light.clone();baked.apply_lighting(&mut physical);
    // Keep the actual rendered exposure/lux, including deliberate overrides.
    light.sun_color=physical.sun_color;
    // Compare the candidate finite-distance medium at the captured camera and
    // atmosphere. Older captures used background-colour fog; do not silently
    // retain that old rendering model when evaluating its replacement.
    light.fog_phase_g=physical.fog_phase_g;
    light.fog_scattering_albedo=physical.fog_scattering_albedo;
    light.fog_height_m=physical.fog_height_m;
    if let Some(f)=fog {light.fog_start=f.x;light.fog_end=f.y;light.fog_density=f.z;light.shadow_strength=1.0;}
    for key in ["fog_start","fog_end","fog_density","shadow_strength","contrast","saturation","alpha_cut","grass_reference"] {
        if !capture["render"][key].is_null(){light.set(&format!("{key}={}",number(&capture["render"][key])?))?;}
    }
    if !capture["render"]["sun_color"].is_null(){light.sun_color=vector(&capture["render"]["sun_color"])?;}
    let metadata=json!({"captured":s,"model_source":"F12 v1 named preset, frozen coefficients; sun/sky RGB verified after bake",
        "solar":{"elevation_degrees":solar.elevation_degrees,"azimuth_degrees":solar.azimuth_degrees},
        "model":{"planet_radius_km":model.planet_radius_km,"height_km":model.height_km,"observer_height_m":model.observer_height_m,
            "rayleigh_per_km":model.rayleigh_per_km.to_array(),"rayleigh_height_km":model.rayleigh_height_km,
            "mie_scattering_per_km":model.mie_scattering_per_km,"mie_absorption_per_km":model.mie_absorption_per_km,
            "mie_height_km":model.mie_height_km,"mie_g":model.mie_g,"ozone_per_km":model.ozone_per_km.to_array(),
            "ground_albedo":model.ground_albedo.to_array(),"solar_rgb_lux":model.solar_rgb_lux.to_array()},
        "quality":{"transmittance_lut":[quality.transmittance_width,quality.transmittance_height],"transmittance_steps":quality.transmittance_steps,
            "sky_lut":[quality.sky_width,quality.sky_height],"sky_steps":quality.sky_steps},
        "actual_light":{"sun_direction":light.sun_dir.to_array(),"sun_color":light.sun_color.to_array(),
            "sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100,"shadow_strength":light.shadow_strength,
            "fog_start_end_density":[light.fog_start,light.fog_end,light.fog_density],
            "fog_medium":{"mode":"candidate_local_exponential_height_single_scattering",
                "height_m":light.fog_height_m,"phase_g":light.fog_phase_g,
                "scattering_albedo":light.fog_scattering_albedo.to_array(),"hard_end":false}}});
    Ok(Some(ReplaySky {baked,metadata}))
}

fn context(capture:&Value)->Result<(sideways::stage::Stage,Camera,Lighting,render::wind::WindState,Option<ReplaySky>),String> {
    let seed=capture["seed"].as_u64().ok_or("missing seed")?;
    let km=capture["state"]["active_settings"]["stage_km"].as_f64().ok_or("missing stage length")?;
    if !km.is_finite() || !(0.1..=8.0).contains(&km){return Err("invalid captured stage length".into());}
    // Reproduce the bounded stage-only RNG consumption for an unchanged
    // roster/length history. Validate both recorded stage seeds afterwards;
    // unknown past setting changes must fail rather than show another map.
    let round=capture["state"]["race_round"].as_u64().ok_or("missing round")?;
    if !(1..=64).contains(&round){return Err("replay supports rounds 1..64".into());}
    let players=capture["state"]["players"].as_array().ok_or("missing roster")?.len();
    let rivals=capture["state"]["active_settings"]["rivals"].as_u64().ok_or("missing rivals")? as usize;
    let humans=capture["state"]["view_rects"].as_array().ok_or("missing views")?.len();
    let effective_rivals=if rivals==1 && humans>=2 {0}else{rivals};
    if players!=humans+effective_rivals || humans==0 || humans>4 {return Err("unsupported capture roster".into());}
    let mut rng=gdcompat::Pcg::new(seed);
    for _ in 1..round {
        rng.next_u32();
        sideways::stage::Stage::generate(&mut rng,km*1000.0+sideways::stage::RUNOUT);
        for player in 0..players {
            rng.skip(sideways::race::WHEEL_FX_PARTICLES);
            if player>=humans {rng.randf_range(0.86,0.97);}
            rng.randf_global();
        }
    }
    rng.next_u32();
    let mut stage=sideways::stage::Stage::generate(&mut rng,km*1000.0+sideways::stage::RUNOUT);
    if capture["stage_noise_seed"].as_i64()!=Some(stage.noise_seed as i64)
        || capture["stage_detail_seed"].as_i64()!=Some(stage.detail_seed as i64) {
        return Err("capture stage differs from reconstructed history; replay refuses a different map".into());
    }
    let nature=&capture["state"]["nature"];
    let plant=&nature["vegetation"];
    stage.vegetation=sideways::vegetation::Vegetation::new(
        plant["seed"].as_u64().ok_or("missing vegetation seed")?,number(&plant["growth"])?,
        plant["season"].as_str().ok_or("missing season")?.parse()?)?;
    let c=&capture["views"][0]["camera"];
    let camera=Camera {pos:vector(&c["position"])?,target:vector(&c["target"])?,
        fov_y:number(&c["fov_y"])?,near:number(&c["near"])?};
    if camera.near<=0.0 || !(0.05..3.0).contains(&camera.fov_y)
        || camera.pos.distance_squared(camera.target)<1e-8 {return Err("invalid capture camera".into());}
    let r=&capture["render"];
    let mut light=Lighting {taa:0.0,..Default::default()};
    match capture["weather"].as_str().ok_or("missing weather")? {
        "clear"=>{},"overcast"=>light.shadow_strength=0.6,
        "mist"=>light.fog_density=0.011,"wet"=>light.shadow_strength=0.5,
        _=>return Err("unknown capture weather".into()),
    }
    light.sun_dir=vector(&r["sun_direction"])?;
    for key in ["sun_lux","sky_lux","ev100","lod_pixels","gi","contact_occlusion","reflection_occlusion","near_shadows"] {
        light.set(&format!("{key}={}",number(&r[key])?))?;
    }
    let sky=prepare_sky(capture,&mut light)?;
    let w=&nature["wind"];
    let wind=render::wind::WindState {time_seconds:w["time_seconds"].as_f64().ok_or("missing wind time")?,
        direction:Vec2::new(number(&w["direction_xz"][0])?,number(&w["direction_xz"][1])?),
        strength:number(&w["strength"])?,tree_shear:number(&w["tree_shear"])?,grass_shear:number(&w["grass_shear"])?}.validated()?;
    Ok((stage,camera,light,wind,sky))
}

fn run(capture_path:&Path,out:&Path,cpu_only:bool,lit_only:bool,overrides:&[String],yaw_degrees:f32)->Result<(),String> {
    let capture:Value=serde_json::from_slice(&std::fs::read(capture_path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let (stage,mut camera,mut light,wind,sky)=context(&capture)?;
    if !yaw_degrees.is_finite(){return Err("nonfinite camera yaw override".into());}
    if yaw_degrees!=0.0 {
        camera.target=camera.pos+glam::Quat::from_rotation_y(yaw_degrees.to_radians())*(camera.target-camera.pos);
    }
    let player=capture["state"]["view_order"][0].as_u64().ok_or("missing captured view order")? as usize;
    let focus=vector(&capture["state"]["players"][player]["position"])?;
    let headlights=captured_headlights(&capture)?;
    for setting in overrides {light.set(setting)?;}
    if cpu_only {println!("stage/camera/sky verified: noise={}, detail={}, camera={:?}, procedural={}, exposure={}, fog={:?}",
        stage.noise_seed,stage.detail_seed,camera.pos,sky.is_some(),light.ev100,[light.fog_start,light.fog_end,light.fog_density]);return Ok(());}
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
    let looks=car_look::Looks::load(None,vec![])?;
    let mut built=visuals::build_live(&stage,&[],&[camera.pos],&looks)?;
    if let Some(expected)=capture["state"]["grass_detail_profile"].as_str() {
        if expected!=built.scene.grass_detail.label(){return Err(format!("grass profile differs: capture={expected}, replay={}; set SIDEWAYS_GRASS_DETAIL before replay",built.scene.grass_detail.label()));}
    }
    if let Some(ref sky)=sky {
        // Attach the one shared physical environment without replacing any of
        // the captured light/exposure/fog values reconstructed above.
        built.scene.sky=Some((sky.baked.width,sky.baked.height,sky.baked.pixels.clone()));
        built.scene.sky_physical=true;
    }
    for line in &built.log {println!("{line}");}
    let overlays:Vec<_>=built.mesh_names.iter().filter(|n|n.starts_with("original fractured cliff")).cloned().collect();
    let gpu=Gpu::new()?;
    let renderer=Renderer::new(&gpu,&built.scene,&light);
    renderer.set_headlights(&headlights)?;
    renderer.set_lighting_focus(&[focus])?;
    renderer.set_wind(&gpu,&wind)?;renderer.set_animation_time(wind.time_seconds)?;
    renderer.set_frame_time_ms(1000.0/60.0);
    let source_w=number(&capture["capture"]["size"][0])?;
    let source_h=number(&capture["capture"]["size"][1])?;
    let width=1920u32;let height=(width as f32*source_h/source_w).round()as u32;
    if !(64..=4096).contains(&height){return Err("invalid replay aspect ratio".into());}
    let views=[ViewRect {cam:camera,x:0,y:0,w:width,h:height}];
    let mut picks:Vec<_>=[(0.12,0.25),(0.25,0.25),(0.38,0.25),(0.5,0.25),(0.65,0.25),(0.8,0.25),
        (0.25,0.4),(0.5,0.4),(0.75,0.4)].into_iter().map(|(x,y)|((x*width as f32)as u32,(y*height as f32)as u32)).collect();
    // F12 102808-5: two completely black wall pixels and a textured wall
    // control, expressed in original 3840x2160 coordinates. Picks identify
    // the actual frontmost owner; do not assume every black pixel is the wall.
    picks.extend([(1300.0,795.0),(1500.0,820.0),(2100.0,870.0)].map(|(x,y)|
        ((x/3840.0*width as f32)as u32,(y/2160.0*height as f32)as u32)));
    let mut rows=Vec::new();
    let selected_debug=light.debug;
    for (name,debug) in [("lit",selected_debug),("albedo",2.0),("direct",3.0),("indirect",4.0)] {
        if lit_only && name!="lit" {continue;}
        light.debug=debug;
        for _ in 0..3 {renderer.try_render_views(&gpu,&views,&light,width,height,&[])?;}
        let (pixels,stats)=renderer.try_render_views(&gpu,&views,&light,width,height,&picks)?;
        let path=out.join(format!("{name}.png"));render::write_png(&path,width,height,&pixels)?;
        let selection:Vec<_>=stats.picks.iter().zip(&picks).map(|(p,xy)|match p {
            None=>json!({"pixel":xy,"hit":null}),
            Some((owner,cluster,tri))=>{
                let instance=&built.scene.instances[*owner as usize];
                let mesh=built.scene.meshes.iter().position(|m|m.first_cluster==instance.first_cluster);
                json!({"pixel":xy,"owner":owner,"cluster":cluster,"triangle":tri,
                    "mesh":mesh.and_then(|i|built.mesh_names.get(i)),
                    "material":built.scene.clusters[*cluster as usize].material})
            }
        }).collect();
        rows.push(json!({"name":name,"image":path,"gpu_ms":stats.gpu_ms,"picks":selection,
            "clusters":stats.clusters,"triangles":stats.triangles}));
    }
    let report=json!({"schema":2,"capture":capture_path,"stage_verified":true,"lit_only":lit_only,"diagnostic_light_overrides":overrides,
        "stage_noise_seed":stage.noise_seed,"stage_detail_seed":stage.detail_seed,
        "source_camera":capture["views"][0]["camera"],
        "camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y,"near":camera.near},
        "camera_yaw_override_degrees":yaw_degrees,"lighting_focus":focus.to_array(),
        "legacy_light_lookahead":std::env::var("KERN_LEGACY_LIGHT_LOOKAHEAD").is_ok_and(|v|v=="1"),
        "compact_canopy_support":std::env::var("KERN_CANOPY_COMPACT_SUPPORT").is_ok_and(|v|v=="1"),"size":[width,height],"adapter":gpu.info.name,
        "source_image":capture["capture"]["png"],"overlay_meshes":overlays,"rows":rows,
        "sky_replay":sky.as_ref().map(|s|&s.metadata),"grass_detail_profile":built.scene.grass_detail.label(),
        "headlights_replay":capture["render"]["headlights"],
        "limitations":["native diagnostic reconstruction, not original DLSS frame","no cars or HUD; captured lamp poses preserved, car self-shadow omitted",
            "stage-only round history verified against both recorded stage seeds, with exact camera, applied sky, sun, exposure, fog and wind phase",
            "held-camera diagnostic timings are not gameplay FPS"]});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("replay report: {}",out.join("report.json").display());Ok(())
}

fn main() {
    let args:Vec<_>=std::env::args().skip(1).collect();
    let result=(|| {
        let path=args.first().ok_or("usage: capture_replay F12.json FRESH_DIR [--cpu-only] [--lit-only]")?;
        let out=args.get(1).ok_or("missing fresh output directory")?;
        let mut settings=Vec::new();let mut yaw=0.0f32;let mut i=2;
        while i<args.len() {match args[i].as_str() {
            "--cpu-only"|"--lit-only"=>{},
            "--yaw-deg"=>{i+=1;yaw=args.get(i).ok_or("--yaw-deg needs an angle")?.parse().map_err(|_|"invalid yaw angle")?;},
            "--set"=>{i+=1;settings.push(args.get(i).ok_or("--set needs key=value")?.clone());},
            _=>return Err("unknown replay arguments".into()),
        }i+=1;}
        run(&PathBuf::from(path),&PathBuf::from(out),args.iter().any(|s|s=="--cpu-only"),args.iter().any(|s|s=="--lit-only"),&settings,yaw)
    })();
    if let Err(e)=result {eprintln!("capture replay: {e}");std::process::exit(1);}
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    #[test]
    fn headlight_capture_preserves_actual_origins_and_rejects_missing_fields() {
        let lamp=json!({"position":[1,2,3],"direction":[0,0,-1],"up":[0,1,0],
            "intensity_cd":[40000,38000,34000],"range_m":90,
            "inner_angle_radians":0.296706,"outer_angle_radians":0.558505});
        let capture=json!({"render":{"headlights":{"enabled":true,"lights":[lamp.clone()]}}});
        let actual=captured_headlights(&capture).unwrap();
        assert_eq!(actual.len(),1);assert_eq!(actual[0].position,Vec3::new(1.,2.,3.));
        assert_eq!(actual[0].intensity_cd,Vec3::new(40000.,38000.,34000.));
        let mut broken=capture.clone();broken["render"]["headlights"]["lights"][0]["position"]=Value::Null;
        assert!(captured_headlights(&broken).is_err());
        broken=capture.clone();broken["render"]["headlights"]["enabled"]=json!(false);
        assert!(captured_headlights(&broken).is_err());
        assert!(captured_headlights(&json!({"render":{}})).unwrap().is_empty());
    }
    fn mist_capture()->Value {
        json!({"render":{"sun_direction":[-0.9993908405303955,0.03489949554204941,1.835851136596192e-16],
            "sky":{"mode":"procedural_single_scattering","atmosphere":"mist","solar_elevation_degrees":2.0,
                "solar_azimuth_degrees":270.0,"sky_lut":[256,128],"fog_start_end_density":[20.0,190.0,0.011],
                "sun_rgb_lux":[33430.92578125,8628.9755859375,626.6815795898438],
                "sky_horizontal_rgb_lux":[1697.654541015625,1592.1688232421875,1749.6956787109375]}},
            "state":{"live_sky_controls":{"desired":{"atmosphere":"clear","time_of_day":"noon"},"pending":true}}})
    }
    #[test]
    fn applied_capture_not_pending_ui_selects_fog_and_atmosphere() {
        let c=mist_capture();let (model,solar,quality,fog)=sky_spec(&c).unwrap().unwrap();
        assert_eq!(model.mie_scattering_per_km,0.003996*14.0);
        assert_eq!(model.mie_height_km,0.22);assert_eq!(solar.elevation_degrees,2.0);
        assert_eq!(quality.sky_steps,48);assert_eq!(fog.unwrap(),Vec3::new(20.0,190.0,0.011));
        for (name,scale,height) in [("clear",1.0,1.2),("haze",4.0,1.2),("mist",14.0,0.22)] {
            let mut c=c.clone();c["render"]["sky"]["atmosphere"]=name.into();
            let (m,_,_,_)=sky_spec(&c).unwrap().unwrap();
            assert_eq!(m.mie_scattering_per_km,0.003996*scale);assert_eq!(m.mie_height_km,height);
        }
    }
    #[test]
    fn incomplete_unknown_and_inconsistent_capture_fails_before_gpu() {
        let c=mist_capture();
        for (key,value) in [("atmosphere",json!("rain")),("sky_lut",json!([128,64])),
            ("fog_start_end_density",Value::Null),("fog_start_end_density",json!([20,190,-1]))] {
            let mut broken=c.clone();broken["render"]["sky"][key]=value;assert!(sky_spec(&broken).is_err(),"{key}");
        }
        let mut broken=c.clone();broken["render"]["sun_direction"]=json!([0,1,0]);assert!(sky_spec(&broken).is_err());
        let mut broken=c.clone();broken["state"]["live_sky_controls"]["applied"]["sky"]=json!({"mode":"hdri"});
        assert!(sky_spec(&broken).is_err());
    }
    #[test]
    fn captured_mist_bake_matches_and_preserves_actual_exposure_and_lux() {
        let c=mist_capture();let mut light=Lighting {ev100:11.75,sun_lux:7000.0,sky_lux:1500.0,..Default::default()};
        let sky=prepare_sky(&c,&mut light).unwrap().unwrap();
        assert_eq!(light.ev100,11.75);assert_eq!(light.sun_lux,7000.0);assert_eq!(light.sky_lux,1500.0);
        assert_eq!(light.fog_start,20.0);assert_eq!(light.fog_end,190.0);assert_eq!(light.fog_density,0.011);
        assert_eq!(light.fog_height_m,220.0);assert_eq!(light.fog_phase_g,0.8);
        assert!(light.fog_scattering_albedo.abs_diff_eq(Vec3::splat(0.9),1e-6));
        assert!((light.sun_color.x-33430.92578125/8628.9755859375).abs()<1e-5);
        assert_eq!(sky.baked.pixels.len(),256*128);assert!(sky.baked.sh.iter().flatten().all(|v|v.is_finite()));
    }
}
