//! Rebuild the current scene at a verified F12 stage/camera and applied sky.
//! Diagnostic views omit cars and HUD; they are not gameplay/performance claims.
//! `--cpu-only` checks stage/camera and captured controls without creating a GPU.
//! `--no-meadow` explicitly overrides restored controls to isolate the ground.
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
#[path="../src/tree_tuning.rs"] mod tree_tuning;
#[path="../src/voxel_tree_patch.rs"] mod voxel_tree_patch;
#[path="../src/visuals.rs"] mod visuals;

use glam::{DVec3, Mat4, Vec2, Vec3};
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, BakedSky, SolarState};
use render::debug_settings::DebugRenderOptions;
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

fn rgb(v:&Value)->Result<[f32;3],String> {
    if v.as_array().is_none_or(|v|v.len()!=3) {return Err("captured RGB must contain exactly three numbers".into());}
    Ok(vector(v)?.to_array())
}

struct CapturedControls {
    options: DebugRenderOptions,
    trees: Option<tree_tuning::Settings>,
    historical_fields: Vec<&'static str>,
}

// Historical captures predate these controls. Freeze their neutral behavior
// here rather than importing today's saved game preset or renderer defaults.
// An absent field is recorded; a present but malformed field is an error.
fn captured_controls(capture:&Value)->Result<CapturedControls,String> {
    let debug=capture["render"].get("engine_debug");
    if debug.is_some_and(|v|!v.is_object()) {return Err("captured engine_debug must be an object".into());}
    let mut historical_fields=Vec::new();
    let mut field=|key:&'static str| {
        let value=debug.and_then(|d|d.get(key));
        if value.is_none(){historical_fields.push(key);}
        value
    };
    let boolean=|v:Option<&Value>|->Result<bool,String> {
        v.map(|v|v.as_bool().ok_or_else(||"captured feature toggle must be boolean".into())).unwrap_or(Ok(true))
    };
    let scalar=|v:Option<&Value>|->Result<f32,String> {v.map(number).unwrap_or(Ok(1.0))};
    let mut options=DebugRenderOptions {
        meadow_enabled:boolean(field("meadow"))?,
        headlights_enabled:boolean(field("headlights"))?,
        transparency_enabled:boolean(field("transparency"))?,
        exhaust_enabled:boolean(field("exhaust"))?,
        wheel_particles_enabled:boolean(field("wheel_particles"))?,
        bloom_enabled:false,bloom_strength:0.0,
        meadow_distance_scale:scalar(field("grass_distance_scale"))?,
        meadow_lod_pixels:scalar(field("grass_lod_pixels"))?,
        meadow_size_scale:scalar(field("grass_size_scale"))?,
        meadow_density:scalar(field("grass_patch_count_fraction"))?,
        meadow_color_multiplier:field("grass_color_multiplier").map(rgb).unwrap_or(Ok([1.0;3]))?,
    };
    if let Some(bloom)=field("bloom") {
        options.bloom_enabled=bloom["enabled"].as_bool().ok_or("captured bloom lacks boolean enabled")?;
        options.bloom_strength=number(&bloom["strength"])?;
        if number(&bloom["threshold_exposed"])?!=render::bloom::THRESHOLD
            || number(&bloom["knee_exposed"])?!=render::bloom::KNEE
        {return Err("captured bloom threshold/knee differ from this renderer".into());}
        if let Some(active)=bloom.get("active") {
            if active.as_bool()!=Some(options.bloom_enabled&&options.bloom_strength>0.0) {
                return Err("captured bloom active state contradicts its settings".into());
            }
        }
    }
    options.validate()?;
    let trees=field("tree_applied").map(|t| {
        let settings=tree_tuning::Settings {size_scale:number(&t["size_scale"])?,density:number(&t["density"])?,
            color_multiplier:rgb(&t["color_multiplier"])?};
        settings.validate()?;Ok::<_,String>(settings)
    }).transpose()?;
    Ok(CapturedControls{options,trees,historical_fields})
}

fn options_metadata(o:DebugRenderOptions)->Value {
    json!({"meadow":o.meadow_enabled,"headlights":o.headlights_enabled,"transparency":o.transparency_enabled,
        "exhaust":o.exhaust_enabled,"wheel_particles":o.wheel_particles_enabled,
        "grass_distance_scale":o.meadow_distance_scale,"grass_lod_pixels":o.meadow_lod_pixels,
        "grass_size_scale":o.meadow_size_scale,"grass_patch_count_fraction":o.meadow_density,
        "grass_color_multiplier":o.meadow_color_multiplier,
        "bloom":{"enabled":o.bloom_enabled,"strength":o.bloom_strength,
            "threshold_exposed":render::bloom::THRESHOLD,"knee_exposed":render::bloom::KNEE}})
}

fn replay_options(captured:DebugRenderOptions,no_meadow:bool)->DebugRenderOptions {
    DebugRenderOptions{meadow_enabled:captured.meadow_enabled&&!no_meadow,..captured}
}

fn restore_trees(scene:&mut render::scene::Scene,settings:Option<tree_tuning::Settings>)->Result<Value,String> {
    let Some(settings)=settings else {
        return Ok(json!({"origin":"historical authored source; tree_applied absent","applied":false}));
    };
    // This diagnostic has no simulated cars/colliders. Reuse exactly the game
    // source transform/subset/material transaction before canopy/shadow build.
    let mut physics=sideways::physics::Physics::new();
    let baseline=tree_tuning::Baseline::prepare_optional_stage(scene,&mut physics,settings)?;
    Ok(json!({"origin":"render.engine_debug.tree_applied","applied":true,
        "source":tree_tuning::Baseline::stage_metadata(baseline.as_ref(),settings),
        "physics":"not reconstructed in this static diagnostic; zero matched collider count is intentional"}))
}

fn validate_source_recipe(capture:&Value,grass:Option<&Value>,trees:&Value)->Result<(),String> {
    // These source controls remain process construction inputs. Refuse a
    // different recipe instead of changing process globals or hiding a mismatch.
    for (name,expected,actual,keys) in [
        ("procedural_grass",&capture["render"]["procedural_grass"],grass,&[
            "mode","recipe","source_blades_per_root","blade_budget","blade_width_multiplier",
            "triangles_per_variant_lod","radius_metres","fade_start_metres","growth_scale","dry_season"] as &[_]),
        ("tree_source",&capture["render"]["tree_source"],Some(trees),&["recipe","fullness","source_archetypes"] as &[_]),
    ] {
        if expected.is_null(){continue;}
        let actual=actual.ok_or_else(||format!("captured {name} exists but reconstructed source is absent"))?;
        for key in keys {
            if let Some(value)=expected.get(*key) {
                if actual.get(*key)!=Some(value) {
                    return Err(format!("captured {name}.{key} differs from reconstructed source: capture={value}, replay={}; restore the captured construction environment",actual[*key]));
                }
            }
        }
    }
    Ok(())
}

fn sun_focus_point(text:&str)->Result<Vec3,String> {
    let values=text.split(',').map(|s|s.trim().parse::<f32>()).collect::<Result<Vec<_>,_>>()
        .map_err(|_|"--sun-focus needs three finite comma-separated coordinates".to_string())?;
    if values.len()!=3 || values.iter().any(|v|!v.is_finite()) {
        return Err("--sun-focus needs three finite comma-separated coordinates".into());
    }
    Ok(Vec3::new(values[0],values[1],values[2]))
}

fn approach_distances(text:&str)->Result<Vec<f32>,String> {
    let values=text.split(',').map(|s|s.trim().parse::<f32>().map_err(|_|"--approach-m needs comma-separated metres".to_string()))
        .collect::<Result<Vec<_>,_>>()?;
    if values.is_empty() || values.len()>16 || values.iter().any(|v|!v.is_finite() || !(0.0..=512.0).contains(v))
        || values.windows(2).any(|w|w[1]<=w[0]) {
        return Err("--approach-m needs 1..16 strictly increasing finite distances in 0..512 metres".into());
    }
    Ok(values)
}

fn approach_axis(camera:&Camera)->Result<Vec3,String> {
    let d=camera.target-camera.pos;let horizontal=Vec3::new(d.x,0.0,d.z);
    if !horizontal.is_finite() || horizontal.length_squared()<1e-8 {
        return Err("approach requires a nonvertical finite captured view direction".into());
    }
    Ok(horizontal.normalize())
}

fn validate_probes(shadow:bool,world_terrain:bool,paged_foliage:bool)->Result<(),String> {
    if [shadow,world_terrain,paged_foliage].into_iter().filter(|v|*v).count()>1 {
        return Err("choose one of --shadow-probe, --world-terrain-probe or --paged-foliage-probe".into());
    }
    Ok(())
}

fn warmup_finished(paged:bool,frames:usize,updated_pages:u32)->bool {
    frames>=3 && (!paged || updated_pages==0 || frames>=40)
}

fn foliage_page_stats(stats:&render::FrameStats)->Value {
    json!({"requested":stats.foliage_sun_requested_pages,"resident":stats.foliage_sun_resident_pages,
        "missing":stats.foliage_sun_missing_pages,"updated":stats.foliage_sun_updated_pages,
        "bytes":stats.foliage_sun_cache_bytes,"clusters":stats.foliage_sun_clusters,
        "triangles":stats.foliage_sun_triangles})
}

/// Reconstruct the triangle selected by the real visibility buffer, using
/// the same pixel-centre ray and affine wind as shade.wgsl. This is NOT a
/// second scene raycast or an estimate from the instance bounding sphere.
fn picked_surface(scene:&render::scene::Scene,camera:&Camera,size:[u32;2],pixel:(u32,u32),
    hit:(u32,u32,u32),wind:&render::wind::WindState,sun:Vec3,focus:Vec3)->Value {
    let (owner,cluster,triangle)=hit;
    let Some(instance)=scene.instances.get(owner as usize) else{return json!({"status":"invalid_owner"});};
    if instance.flags&render::scene::PART_WIND!=0 || scene.baked_animations.as_ref().is_some_and(|a|
        a.parts.iter().any(|p|p.owner==owner)) {
        return json!({"status":"unsupported_gpu_articulated_pose"});
    }
    let Some(cl)=scene.clusters.get(cluster as usize) else{return json!({"status":"invalid_cluster"});};
    if triangle>=cl.tri_count {return json!({"status":"invalid_triangle"});}
    let root=Vec3::new(instance.rows[0][3],instance.rows[1][3],instance.rows[2][3]);
    let replacement=scene.voxel_forest.as_ref().is_some_and(|forest|forest.instances.iter().any(|t|t.meta[0]==owner));
    let shear=if instance.flags&25!=0 && !replacement {wind.sample_shear(root,instance.flags&24!=0)}else{Vec2::ZERO};
    let packed=scene.tris[(cl.first_tri+triangle) as usize];
    let points:[Vec3;3]=std::array::from_fn(|k|{
        let local=(packed>>(8*k))&255;
        let vertex=&scene.vertices[scene.cluster_verts[(cl.first_vert+local) as usize] as usize];
        let p=Vec3::from_array(vertex.pos);
        let mut world=Vec3::new(Vec3::from_slice(&instance.rows[0]).dot(p)+root.x,
            Vec3::from_slice(&instance.rows[1]).dot(p)+root.y,Vec3::from_slice(&instance.rows[2]).dot(p)+root.z);
        world+=Vec3::new(shear.x,0.0,shear.y)*(world.y-root.y);world
    });
    let inverse=(Mat4::perspective_infinite_reverse_rh(camera.fov_y,size[0] as f32/size[1] as f32,camera.near)
        *Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y)).inverse();
    let far=inverse*glam::Vec4::new((pixel.0 as f32+0.5)/size[0] as f32*2.0-1.0,
        1.0-(pixel.1 as f32+0.5)/size[1] as f32*2.0,1e-4,1.0);
    let ray=(far.truncate()/far.w-camera.pos).normalize();
    let e1=points[1]-points[0];let e2=points[2]-points[0];let cross=ray.cross(e2);let det=e1.dot(cross);
    if !det.is_finite() || det.abs()<1e-12 {return json!({"status":"degenerate_ray_triangle"});}
    let offset=camera.pos-points[0];let raw_u=offset.dot(cross)/det;let raw_v=ray.dot(offset.cross(e1))/det;
    let u=raw_u.clamp(0.0,1.0);let v=raw_v.clamp(0.0,1.0-u);
    let p=points[0]*(1.0-u-v)+points[1]*u+points[2]*v;
    let weights=[1.0-u-v,u,v];
    let local_normal=(0..3).map(|k|{
        let local=(packed>>(8*k))&255;
        Vec3::from_array(scene.vertices[scene.cluster_verts[(cl.first_vert+local)as usize]as usize].normal)*weights[k]
    }).sum::<Vec3>();
    let axes:[Vec3;3]=std::array::from_fn(|i|Vec3::new(instance.rows[0][i],instance.rows[1][i],instance.rows[2][i]));
    let mut normal=(0..3).map(|i|axes[i]*local_normal[i]/axes[i].length_squared().max(1e-8)).sum::<Vec3>();
    normal.y-=shear.dot(Vec2::new(normal.x,normal.z));normal=normal.normalize_or_zero();
    let Some(material)=scene.materials.get(cl.material as usize)else{return json!({"status":"invalid_material"});};
    let flags=material.flags;
    if normal.dot(ray)>0.0 && flags&render::scene::TWO_SIDED!=0 && flags&render::scene::GRASS==0 {normal=-normal;}
    let sign=normal.dot(sun)+1e-4;
    let sun_normal=if sign>0.0{normal}else if sign<0.0{-normal}else{Vec3::ZERO};
    let z=sun.normalize_or(Vec3::Y);let up=if z.y.abs()>0.99 {Vec3::Z}else{Vec3::Y};
    let x=up.cross(z).normalize();let y=z.cross(x);let delta=p-focus;
    json!({"status":"reconstructed_selected_triangle","world_position":p.to_array(),
        "geometric_normal":e1.cross(e2).normalize().to_array(),"surface_normal":normal.to_array(),
        "sun_receiver_normal":sun_normal.to_array(),"material_flags":flags,
        "camera_distance_m":p.distance(camera.pos),"camera_ray":ray.to_array(),
        "ray_residual_m":(p-camera.pos).cross(ray).length(),"barycentric":[1.0-u-v,u,v],
        "unclamped_barycentric":[1.0-raw_u-raw_v,raw_u,raw_v],
        "sun_plane_offset_m":[x.dot(delta),y.dot(delta)],
        "sun_plane_half_extent_required_m":x.dot(delta).abs().max(y.dot(delta).abs()),
        "instance_root":root.to_array(),"instance_scale_max":instance.scale,
        "instance_axis_scales":std::array::from_fn::<_,3,_>(|i|Vec3::new(instance.rows[0][i],instance.rows[1][i],instance.rows[2][i]).length()),
        "instance_flags":instance.flags,"wind_shear":shear.to_array(),
        "method":"native pixel centre plus selected source/CLOD triangle; CPU counterpart of captured affine wind"})
}

/// Same two native triangles and f32 receiver placement as ground_at and
/// world_terrain_sun_query. The hierarchy itself is the independent f64 CPU
/// tracer, not a claim to have read back the GPU's internal trace records.
fn source_ground_at(hf:&render::scene::Heightfield,p:Vec2)->f32 {
    let g=(p-Vec2::from_array(hf.origin))/hf.cell;let i=g.floor();
    if !g.is_finite()||i.x<0.0||i.y<0.0||i.x>=(hf.w-1)as f32||i.y>=(hf.h-1)as f32{return -1e9;}
    let x=i.x as usize;let z=i.y as usize;let w=hf.w as usize;let f=g-i;
    let [a,b,c,d]=[hf.heights[z*w+x],hf.heights[z*w+x+1],hf.heights[(z+1)*w+x],hf.heights[(z+1)*w+x+1]];
    if f.x+f.y<=1.0{a+(b-a)*f.x+(c-a)*f.y}else{d+(c-d)*(1.0-f.x)+(b-d)*(1.0-f.y)}
}
fn terrain_source_query(hierarchy:&render::terrain_trace::Hierarchy,surface:&Value,sun:Vec3,terrain_error:f32,budget:u32)->Value {
    if surface["status"]!="reconstructed_selected_triangle" {
        return json!({"status":"Skipped","reason":surface["status"]});
    }
    let Ok(p)=vector(&surface["world_position"])else{return json!({"status":"Skipped","reason":"missing_source_position"});};
    let Ok(n)=vector(&surface["sun_receiver_normal"])else{return json!({"status":"Skipped","reason":"missing_source_normal"});};
    if sun.y<=0.0{return json!({"status":"BelowHorizon","first_blocker":null,"source_position":p.to_array()});}
    let flags=surface["material_flags"].as_u64().unwrap_or(0)as u32;
    let error=if flags&(render::scene::TERRAIN|render::scene::ROAD)!=0{terrain_error.max(0.0)}else{0.0};
    let mut origin=p+n*0.025;let uncorrected=origin;
    let ground=source_ground_at(&hierarchy.field,Vec2::new(origin.x,origin.z));
    if ground> -1e8 && origin.y<ground+0.025 && origin.y>=ground-error-0.025 {origin.y=ground+0.025;}
    let trace=hierarchy.trace_upward(origin,sun,0.01,budget);
    let blocker=if trace.status==render::terrain_trace::Status::Hit{Some(json!({
        "position":trace.position.to_array(),"distance_from_query_origin_m":trace.distance,
        "distance_from_source_m":trace.position.distance(p),"normal":trace.normal.to_array()}))}else{None};
    json!({"status":format!("{:?}",trace.status),"reason":format!("{:?}",trace.reason),
        "source_position":p.to_array(),"source_surface_normal":surface["surface_normal"],"sun_receiver_normal":n.to_array(),
        "query_origin":origin.to_array(),"receiver_height_correction_m":origin.y-uncorrected.y,
        "native_ground_at_offset_m":ground,"representation_error_m":error,"sun_direction":sun.to_array(),
        "trace_distance_m":trace.distance,"trace_end_position":trace.position.to_array(),
        "visited_nodes":trace.visited_nodes,"triangle_tests":trace.triangle_tests,"node_budget":budget,
        "first_blocker":blocker})
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

fn captured_local_lights(capture:&Value)->Result<Vec<render::headlights::LocalLight>,String> {
    let Some(lamps)=capture["render"].get("local_lights") else {return Ok(Vec::new());};
    let rows=lamps["lights"].as_array().ok_or("captured local lights lack light list")?;
    if lamps["enabled"].as_bool()!=Some(!rows.is_empty()) {
        return Err("inconsistent captured local light enable state".into());
    }
    if lamps.get("vehicle_count").is_some() || lamps.get("exhaust_count").is_some() {
        let vehicle=lamps["vehicle_count"].as_u64().ok_or("invalid captured local vehicle light count")?;
        let exhaust=lamps["exhaust_count"].as_u64().ok_or("invalid captured local exhaust light count")?;
        if vehicle.checked_add(exhaust)!=Some(rows.len() as u64) {
            return Err("captured local light counts do not sum to the light list".into());
        }
    }
    let lights=rows.iter().map(|l|Ok(render::headlights::LocalLight {
        position:vector(&l["position"])?,direction:vector(&l["direction"])?,intensity_cd:vector(&l["intensity_cd"])?,
        range_m:number(&l["range_m"])?,inner_cos:number(&l["inner_cos"])?,outer_cos:number(&l["outer_cos"])?,
        source_radius_m:number(&l["source_radius_m"])?,
    })).collect::<Result<Vec<_>,String>>()?;
    render::headlights::validate_local(&lights)
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
    let night=&s["night_environment"];
    let night=if night.is_null() {
        // Older captures had no night floor or stellar catalogue. Keep their
        // historical sky exact instead of silently adopting engine defaults.
        render::atmosphere::NightSky {ambient_rgb_lux:DVec3::ZERO,star_radiance:0.0,star_seed:0}
    } else {
        let seed=night["star_seed"].as_u64().filter(|v|*v<(1<<24)).ok_or("invalid captured stellar seed")?;
        render::atmosphere::NightSky {
            ambient_rgb_lux:vector(&night["configured_ambient_rgb_lux"])?.as_dvec3(),
            star_radiance:number(&night["configured_star_radiance"])?,star_seed:seed as u32}
    };
    let model=Atmosphere {planet_radius_km:6360.0,height_km:100.0,observer_height_m:2.0,
        rayleigh_per_km:DVec3::new(0.005802,0.013558,0.033100),rayleigh_height_km:8.0,
        mie_scattering_per_km:0.003996*scale,mie_absorption_per_km:0.000444*scale,
        mie_height_km:height,mie_g:0.8,ozone_per_km:DVec3::new(0.000650,0.001881,0.000085),
        ground_albedo:DVec3::splat(0.15),solar_rgb_lux:DVec3::splat(127_500.0),
        night};
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
    light.night_stars=physical.night_stars;
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

fn run(capture_path:&Path,out:&Path,cpu_only:bool,lit_only:bool,overrides:&[String],yaw_degrees:f32,shadow_probe:bool,approach:Option<&[f32]>,world_terrain_probe:bool,sun_focus:Option<Vec3>,paged_foliage_probe:bool,no_meadow:bool)->Result<(),String> {
    validate_probes(shadow_probe,world_terrain_probe,paged_foliage_probe)?;
    let paged_foliage_enabled=std::env::var("KERN_PAGED_FOLIAGE_SUN").as_deref()==Ok("1");
    let capture:Value=serde_json::from_slice(&std::fs::read(capture_path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let (stage,mut camera,mut light,wind,sky)=context(&capture)?;
    let movement_axis=if approach.is_some(){approach_axis(&camera)?}else{Vec3::ZERO};
    if !yaw_degrees.is_finite(){return Err("nonfinite camera yaw override".into());}
    if yaw_degrees!=0.0 {
        camera.target=camera.pos+glam::Quat::from_rotation_y(yaw_degrees.to_radians())*(camera.target-camera.pos);
    }
    let player=capture["state"]["view_order"][0].as_u64().ok_or("missing captured view order")? as usize;
    let focus=vector(&capture["state"]["players"][player]["position"])?;
    let headlights=captured_headlights(&capture)?;
    let local_lights=captured_local_lights(&capture)?;
    let controls=captured_controls(&capture)?;
    let effective_options=replay_options(controls.options,no_meadow);
    for setting in overrides {light.set(setting)?;}
    if cpu_only {println!("stage/camera/sky verified: noise={}, detail={}, camera={:?}, procedural={}, exposure={}, fog={:?}",
        stage.noise_seed,stage.detail_seed,camera.pos,sky.is_some(),light.ev100,[light.fog_start,light.fog_end,light.fog_density]);
        println!("captured controls validated: {:?}, applied trees={:?}, local lights={}, historical fields={:?}; source construction not checked in --cpu-only",
            effective_options,controls.trees,local_lights.len(),controls.historical_fields);return Ok(());}
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
    let looks=car_look::Looks::load(None,vec![])?;
    let mut built=visuals::build_live(&stage,&[],&[camera.pos],&looks)?;
    validate_source_recipe(&capture,built.procedural_grass.as_ref(),&built.tree_source)?;
    let tree_replay=restore_trees(&mut built.scene,controls.trees)?;
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
    let terrain_budget=std::env::var("KERN_TERRAIN_NODE_BUDGET").ok().and_then(|s|s.parse::<u32>().ok())
        .unwrap_or(render::terrain_trace::DEFAULT_NODE_BUDGET).min(render::terrain_trace::MAX_NODE_BUDGET);
    let terrain_cpu_start=std::time::Instant::now();
    let terrain_cpu=if world_terrain_probe {
        let coverage=if built.scene.terrain_visibility_complete {render::terrain_trace::Coverage::CompleteLoadedTerrain}
            else{render::terrain_trace::Coverage::UnknownOutside};
        Some(render::terrain_trace::Hierarchy::with_coverage(&built.scene.heightfield,coverage)?)
    }else{None};
    let terrain_cpu_build_ms=terrain_cpu.as_ref().map(|_|terrain_cpu_start.elapsed().as_secs_f64()*1000.0);
    let gpu=Gpu::new()?;
    let renderer=Renderer::new(&gpu,&built.scene,&light);
    renderer.set_debug_options(effective_options)?;
    renderer.set_headlights(&headlights)?;
    renderer.set_local_lights(&local_lights)?;
    renderer.set_lighting_focus(&[focus])?;
    // Diagnostic raster-shadow placement only. The shared canopy/lighting
    // focus still follows the captured player; this stays fixed on approach.
    if let Some(point)=sun_focus {renderer.set_sun_shadow_focus(&[point])?;}
    renderer.set_wind(&gpu,&wind)?;renderer.set_animation_time(wind.time_seconds)?;
    renderer.set_frame_time_ms(1000.0/60.0);
    let source_w=number(&capture["capture"]["size"][0])?;
    let source_h=number(&capture["capture"]["size"][1])?;
    let width=1920u32;let height=(width as f32*source_h/source_w).round()as u32;
    if !(64..=4096).contains(&height){return Err("invalid replay aspect ratio".into());}
    let mut picks:Vec<_>=[(0.12,0.25),(0.25,0.25),(0.38,0.25),(0.5,0.25),(0.65,0.25),(0.8,0.25),
        (0.25,0.4),(0.5,0.4),(0.75,0.4)].into_iter().map(|(x,y)|((x*width as f32)as u32,(y*height as f32)as u32)).collect();
    // F12 192948-0: actual distant crown skyline above the old quarter-height
    // picks. Retain misses/terrain hits so the report does not cherry-pick trees.
    for y in [0.13,0.15,0.17,0.19,0.21,0.23] {
        for x in [0.12,0.2,0.28,0.36,0.44,0.52,0.6,0.68,0.76,0.84] {
            picks.push(((x*width as f32)as u32,(y*height as f32)as u32));
        }
    }
    // F12 102808-5: two completely black wall pixels and a textured wall
    // control, expressed in original 3840x2160 coordinates. Picks identify
    // the actual frontmost owner; do not assume every black pixel is the wall.
    picks.extend([(1300.0,795.0),(1500.0,820.0),(2100.0,870.0)].map(|(x,y)|
        ((x/3840.0*width as f32)as u32,(y/2160.0*height as f32)as u32)));
    // Sunrise F12 134304-1: nearest trunk foot and the same trunk above it.
    picks.extend([(1285.0,1427.0),(1285.0,1415.0)].map(|(x,y)|
        ((x/3840.0*width as f32)as u32,(y/2160.0*height as f32)as u32)));
    if world_terrain_probe && picks.len()>100{return Err("terrain source audit exceeds bounded 100 picks per camera".into());}
    let mut rows=Vec::new();
    let mut terrain_source_queries=Vec::new();
    let selected_debug=light.debug;
    let original_near=light.near_shadows;
    let cases=if paged_foliage_probe {vec![("lit",selected_debug,original_near),("foliage-pages",25.0,original_near)]}
        else if world_terrain_probe {vec![("lit",selected_debug,original_near),
        ("terrain-visibility",23.0,original_near),("terrain-work",24.0,original_near)]}else if shadow_probe {vec![
        ("lit",selected_debug,original_near),("shadow",9.0,original_near),
        ("far-lit",selected_debug,0.0),("far-shadow",9.0,0.0),
        ("direct",3.0,original_near),("normal",12.0,original_near),
        ("grass-shadow",10.0,original_near),("sky",1.0,original_near),
        ("shading-normal",11.0,original_near),("ndl",13.0,original_near),("surface-ndl",14.0,original_near),
        ("raw-near",15.0,original_near),("raw-far",16.0,original_near),("near-weight",17.0,original_near),
    ]}else{vec![("lit",selected_debug,original_near),("albedo",2.0,original_near),
        ("direct",3.0,original_near),("indirect",4.0,original_near)]};
    // Reuse the one scene, renderer, sky bake and immutable world animation.
    // Only the view and physical lighting receiver translate; no stage replay
    // or source geometry rebuild occurs between approach distances.
    for &distance in approach.unwrap_or(&[0.0]) {
      let movement=movement_axis*distance;
      let moved_camera=Camera {pos:camera.pos+movement,target:camera.target+movement,..camera};
      let moved_focus=focus+movement;
      let views=[ViewRect {cam:moved_camera,x:0,y:0,w:width,h:height}];
      renderer.set_lighting_focus(&[moved_focus])?;
      if approach.is_some(){renderer.reset_history();}
      for &(name,debug,near) in &cases {
        if lit_only && !shadow_probe && !world_terrain_probe && !paged_foliage_probe && name!="lit" {continue;}
        light.debug=debug;
        light.near_shadows=near;
        let mut warmup_gpu_ms=Vec::new();
        let mut warmup_far_sun=Vec::new();
        let mut warmup_foliage_pages=Vec::new();
        let warmup_limit=if paged_foliage_enabled {40}else{3};
        for frame in 0..warmup_limit {
            let (_,stats)=renderer.try_render_views(&gpu,&views,&light,width,height,&[])?;
            warmup_far_sun.push(json!({"updated":stats.far_sun_cache_updated,"half_extent_m":stats.far_sun_extent_m,
                "texel_m":stats.far_sun_texel_m,"lod_pixels":stats.far_sun_lod_pixels,"clusters":stats.far_sun_clusters,
                "triangles":stats.far_sun_triangles,"submitted_vertices":stats.far_sun_submitted_vertices}));
            warmup_foliage_pages.push(foliage_page_stats(&stats));
            let finished=warmup_finished(paged_foliage_enabled,frame+1,stats.foliage_sun_updated_pages);
            warmup_gpu_ms.push(stats.gpu_ms);
            if finished {break;}
        }
        let (pixels,stats)=renderer.try_render_views(&gpu,&views,&light,width,height,&picks)?;
        let filename=if approach.is_some(){format!("d{distance}-{name}.png")}else{format!("{name}.png")};
        let path=out.join(filename);render::write_png(&path,width,height,&pixels)?;
        let selection:Vec<_>=stats.picks.iter().zip(&picks).map(|(p,xy)|match p {
            None=>json!({"pixel":xy,"hit":null}),
            Some((owner,cluster,tri))=>{
                let instance=&built.scene.instances[*owner as usize];
                let mesh=built.scene.meshes.iter().position(|m|m.first_cluster==instance.first_cluster);
                json!({"pixel":xy,"owner":owner,"cluster":cluster,"triangle":tri,
                    "mesh":mesh.and_then(|i|built.mesh_names.get(i)),
                    "material":built.scene.clusters[*cluster as usize].material,
                    "surface":picked_surface(&built.scene,&moved_camera,[width,height],*xy,(*owner,*cluster,*tri),&wind,light.sun_dir,moved_focus)})
            }
        }).collect();
        if name=="lit" {
            if let Some(hierarchy)=terrain_cpu.as_ref() {
                let queries:Vec<_>=selection.iter().map(|pick|{
                    let query=if pick["surface"].is_null(){json!({"status":"Skipped","reason":"no_source_hit"})}
                        else{terrain_source_query(hierarchy,&pick["surface"],light.sun_dir,built.scene.terrain_visual_offset_m,terrain_budget)};
                    json!({"pixel":pick["pixel"],"owner":pick["owner"],"cluster":pick["cluster"],"triangle":pick["triangle"],
                        "mesh":pick["mesh"],"material":pick["material"],"query":query})
                }).collect();
                terrain_source_queries.push(json!({"approach_m":distance,"movement":movement.to_array(),
                    "camera":{"position":moved_camera.pos.to_array(),"target":moved_camera.target.to_array()},"queries":queries}));
            }
        }
        rows.push(json!({"name":name,"image":path,"debug":debug,"near_shadows":near,"gpu_ms":stats.gpu_ms,"warmup_gpu_ms":warmup_gpu_ms,"picks":selection,
            "approach_m":distance,"movement_axis":movement_axis.to_array(),"movement":movement.to_array(),
            "camera":{"position":moved_camera.pos.to_array(),"target":moved_camera.target.to_array(),"fov_y":moved_camera.fov_y,"near":moved_camera.near},
            "lighting_focus":moved_focus.to_array(),
            "sun_shadow_focus_override":sun_focus.map(|p|p.to_array()),
            "warmup_far_sun":warmup_far_sun,"far_sun":{"updated":stats.far_sun_cache_updated,"half_extent_m":stats.far_sun_extent_m,
                "texel_m":stats.far_sun_texel_m,"lod_pixels":stats.far_sun_lod_pixels,"clusters":stats.far_sun_clusters,"triangles":stats.far_sun_triangles,"submitted_vertices":stats.far_sun_submitted_vertices},
            "warmup_foliage_pages":warmup_foliage_pages,"foliage_pages":foliage_page_stats(&stats),
            "world_terrain":{"enabled":stats.world_terrain_visibility,"bytes":stats.world_terrain_bytes,"setup_cpu_ms":stats.world_terrain_setup_cpu_ms},
            "camera_sun_visibility":stats.camera_sun_visibility,"clusters":stats.clusters,"triangles":stats.triangles}));
      }
    }
    let mut report=json!({"schema":3,"capture":capture_path,"stage_verified":true,"lit_only":lit_only,"shadow_probe":shadow_probe,"world_terrain_probe":world_terrain_probe,"diagnostic_light_overrides":overrides,
        "stage_noise_seed":stage.noise_seed,"stage_detail_seed":stage.detail_seed,
        "source_camera":capture["views"][0]["camera"],
        "camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y,"near":camera.near},
        "camera_yaw_override_degrees":yaw_degrees,"lighting_focus":focus.to_array(),
        "sun_shadow_focus_override":sun_focus.map(|p|p.to_array()),
        "sun_focus_contract":"optional fixed world-space raster sun-shadow focus, independent of lighting/canopy focus; never translated by --approach-m; absent preserves normal shadow placement",
        "approach_m":approach,"movement_axis":movement_axis.to_array(),
        "approach_contract":"camera position, target and physical focus translate along captured horizontal view; sun, exposure, wind phase, source world and captured lamp positions stay fixed; no collision following or stage rebuild",
        "legacy_light_lookahead":std::env::var("KERN_LEGACY_LIGHT_LOOKAHEAD").is_ok_and(|v|v=="1"),
        "candidate_flags": (["KERN_SUN_RECEIVER_PLANE","KERN_SUN_SCENE_DEPTH","KERN_SUN_TERRAIN_ERROR","KERN_SUN_SURFACE_SOURCE","KERN_ROCK_VARIATION","SIDEWAYS_ADAPTIVE_CLIFF","KERN_FAR_SUN_SHADOW","KERN_WORLD_TERRAIN_VISIBILITY","KERN_PAGED_FOLIAGE_SUN"]
            .map(|key| (key,std::env::var(key).is_ok_and(|v|v=="1"))).into_iter().collect::<std::collections::BTreeMap<_,_>>()),
        "camera_sun_occlusion":std::env::var("KERN_CAMERA_SUN_OCCLUSION").as_deref()!=Ok("0"),
        "far_sun_lod_setting":std::env::var("KERN_FAR_SUN_LOD").ok(),
        "world_terrain_node_budget_setting":std::env::var("KERN_TERRAIN_NODE_BUDGET").ok(),
        "world_terrain_node_budget":terrain_budget,
        "terrain_visibility_complete":built.scene.terrain_visibility_complete,
        "terrain_source_queries":terrain_source_queries,
        "terrain_source_cpu_hierarchy":{"build_ms":terrain_cpu_build_ms,"bytes":terrain_cpu.as_ref().map(|h|h.storage_bytes()),
            "coverage":terrain_cpu.as_ref().map(|h|format!("{:?}",h.coverage)),"representation_error_m":built.scene.terrain_visual_offset_m},
        "terrain_source_query_contract":"once per lit-camera pick; actual selected source/CLOD triangle and authored normal with captured affine wind; same f32 receiver placement and material-specific shell as shader, then independent f64 CPU hierarchy. Not GPU trace readback; float-boundary differences are possible. No car/foliage occluders in terrain hierarchy; skipped hits remain explicit",
        "world_terrain_probe_contract":"debug23: clear white, terrain blocked black, unknown magenta; debug24: visited-node work divided by configured budget, unknown magenta; CPU setup time is not GPU hierarchy time",
        "compact_canopy_support":std::env::var("KERN_CANOPY_COMPACT_SUPPORT").is_ok_and(|v|v=="1"),"size":[width,height],"adapter":gpu.info.name,
        "source_image":capture["capture"]["png"],"overlay_meshes":overlays,"rows":rows,
        "sky_replay":sky.as_ref().map(|s|&s.metadata),"grass_detail_profile":built.scene.grass_detail.label(),
        "headlights_replay":capture["render"]["headlights"]});
    report["limitations"]=json!(["native diagnostic reconstruction, not original DLSS frame","no cars or HUD; captured lamp poses preserved, car self-shadow omitted",
            "no visible exhaust, wheel particles or emissive car geometry; their captured feature toggles cannot recreate omitted dynamic geometry",
            "source recipe settings are checked where recorded; external render flags/shader versions still require the captured environment, not inferred from saved defaults",
            "stage-only round history verified against both recorded stage seeds, with exact camera, applied sky, sun, exposure, fog and wind phase",
            "held-camera diagnostic timings are not gameplay FPS"]);
    report["local_lights_replay"]=json!({"captured":capture["render"]["local_lights"],"restored_count":local_lights.len(),
        "origin":if capture["render"].get("local_lights").is_some(){"captured emitters"}else{"historical absent; no emitters invented"}});
    report["vegetation_replay"]=json!({"trees":tree_replay,"captured_options":options_metadata(controls.options),
        "effective_options":options_metadata(renderer.debug_options()),"no_meadow_override":no_meadow,
        "historical_neutral_fields":controls.historical_fields,"constructed_grass":built.procedural_grass,
        "constructed_trees":built.tree_source,
        "scope":"Applied tree settings only, before renderer resource build; captured grass controls and feature toggles. Pending UI and saved presets are never replay inputs. Absent historical controls use explicitly recorded authored/neutral behavior; bloom absent means off."});
    report["paged_foliage_probe"]=json!(paged_foliage_probe);
    report["paged_foliage_probe_contract"]=json!("debug25: source-alpha blocked black, clear white, missing magenta. Resident counts current requests, not total occupied cache slots. Missing pages remain unresolved, not assumed clear");
    report["foliage_page_warmup"]=json!({"enabled":paged_foliage_enabled,"minimum_frames":3,"maximum_frames":if paged_foliage_enabled {40}else{3},
        "stop_rule":"after at least3 frames, stop when updated pages=0 or bounded40frames reached; capacity-limited missing pages may remain and are reported"});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("replay report: {}",out.join("report.json").display());Ok(())
}

fn main() {
    let args:Vec<_>=std::env::args().skip(1).collect();
    let result=(|| {
        let path=args.first().ok_or("usage: capture_replay F12.json FRESH_DIR [--cpu-only] [--lit-only] [--no-meadow]")?;
        let out=args.get(1).ok_or("missing fresh output directory")?;
        let mut settings=Vec::new();let mut yaw=0.0f32;let mut approach=None;let mut sun_focus=None;let mut i=2;
        while i<args.len() {match args[i].as_str() {
            "--cpu-only"|"--lit-only"|"--shadow-probe"|"--world-terrain-probe"|"--paged-foliage-probe"|"--no-meadow"=>{},
            "--yaw-deg"=>{i+=1;yaw=args.get(i).ok_or("--yaw-deg needs an angle")?.parse().map_err(|_|"invalid yaw angle")?;},
            "--set"=>{i+=1;settings.push(args.get(i).ok_or("--set needs key=value")?.clone());},
            "--approach-m"=>{if approach.is_some(){return Err("--approach-m may only be supplied once".into());}i+=1;approach=Some(approach_distances(args.get(i).ok_or("--approach-m needs comma-separated metres")?)?);},
            "--sun-focus"=>{if sun_focus.is_some(){return Err("--sun-focus may only be supplied once".into());}i+=1;sun_focus=Some(sun_focus_point(args.get(i).ok_or("--sun-focus needs x,y,z")?)?);},
            _=>return Err("unknown replay arguments".into()),
        }i+=1;}
        run(&PathBuf::from(path),&PathBuf::from(out),args.iter().any(|s|s=="--cpu-only"),args.iter().any(|s|s=="--lit-only"),&settings,yaw,args.iter().any(|s|s=="--shadow-probe"),approach.as_deref(),args.iter().any(|s|s=="--world-terrain-probe"),sun_focus,args.iter().any(|s|s=="--paged-foliage-probe"),args.iter().any(|s|s=="--no-meadow"))
    })();
    if let Err(e)=result {eprintln!("capture replay: {e}");std::process::exit(1);}
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    fn control_capture()->Value {
        json!({"render":{"engine_debug":{
            "meadow":true,"headlights":false,"transparency":false,"exhaust":true,"wheel_particles":false,
            "grass_distance_scale":0.75,"grass_lod_pixels":4.0,"lod_pixels":0.5,
            "grass_size_scale":2.0,"grass_patch_count_fraction":0.55,"grass_color_multiplier":[1.3,0.9,1.3],
            "bloom":{"enabled":true,"active":true,"strength":0.04,"threshold_exposed":2.0,"knee_exposed":1.0},
            "tree_applied":{"size_scale":1.5,"density":0.45,"color_multiplier":[1.0,1.15,0.9]},
            "tree_pending":{"size_scale":0.5,"density":0.0,"color_multiplier":[0.25,0.25,0.25]},
            "saved_vegetation_baseline":{"grass_patch_count_fraction":0.15,"tree_size_scale":0.75}
        }}})
    }
    #[test]
    fn captured_applied_vegetation_and_features_override_neutral_not_pending_or_saved_values() {
        let c=captured_controls(&control_capture()).unwrap();
        assert!(c.historical_fields.is_empty());
        assert_eq!(c.trees,Some(tree_tuning::Settings{size_scale:1.5,density:0.45,color_multiplier:[1.0,1.15,0.9]}));
        assert_eq!(c.options,DebugRenderOptions{meadow_enabled:true,headlights_enabled:false,
            transparency_enabled:false,exhaust_enabled:true,wheel_particles_enabled:false,bloom_enabled:true,bloom_strength:0.04,
            meadow_distance_scale:0.75,meadow_lod_pixels:4.0,meadow_size_scale:2.0,meadow_density:0.55,
            meadow_color_multiplier:[1.3,0.9,1.3]});
        let off=replay_options(c.options,true);
        assert_eq!(off,DebugRenderOptions{meadow_enabled:false,..c.options});
        assert_eq!(replay_options(c.options,false),c.options);
        assert_eq!(options_metadata(off)["grass_lod_pixels"],4.0);
    }
    #[test]
    fn missing_historical_controls_remain_authored_and_do_not_invent_bloom_or_saved_presets() {
        let mut capture=control_capture();
        let debug=capture["render"]["engine_debug"].as_object_mut().unwrap();
        debug.retain(|k,_|k=="tree_pending"||k=="saved_vegetation_baseline");
        let historical=captured_controls(&capture).unwrap();
        assert!(historical.trees.is_none());
        assert_eq!(historical.options,DebugRenderOptions{bloom_enabled:false,bloom_strength:0.0,..Default::default()});
        assert!(historical.historical_fields.contains(&"tree_applied"));
        assert!(historical.historical_fields.contains(&"grass_patch_count_fraction"));
        assert!(historical.historical_fields.contains(&"bloom"));
        assert_eq!(historical.options,captured_controls(&json!({"render":{}})).unwrap().options);
        let mut scene=render::scene::Scene::default();
        assert_eq!(restore_trees(&mut scene,None).unwrap()["applied"],false);
    }
    #[test]
    fn present_invalid_controls_fail_instead_of_using_current_defaults() {
        for (key,value) in [("meadow",json!(1)),("grass_size_scale",json!(2.1)),
            ("grass_lod_pixels",json!(0.0)),("grass_patch_count_fraction",Value::Null),
            ("grass_color_multiplier",json!([1,1,1,1])),("tree_applied",json!({"size_scale":1.5})),
            ("bloom",json!({"enabled":true,"strength":0.04})),
            ("bloom",json!({"enabled":true,"active":false,"strength":0.04,"threshold_exposed":2,"knee_exposed":1})),
            ("bloom",json!({"enabled":true,"strength":0.04,"threshold_exposed":3,"knee_exposed":1}))] {
            let mut broken=control_capture();broken["render"]["engine_debug"][key]=value;
            assert!(captured_controls(&broken).is_err(),"{key}");
        }
        let mut broken=control_capture();broken["render"]["engine_debug"]=Value::Null;
        assert!(captured_controls(&broken).is_err());
    }
    #[test]
    fn tree_replay_applies_size_pigment_and_population_before_renderer_construction() {
        use render::cluster::{ClusterMesh,Part,Vertex};
        use render::scene::{GeometryCategory,Material,ALPHA_TEST};
        let mut scene=render::scene::Scene::default();
        let material=scene.add_material(Material{tint:[0.2,0.4,0.6],flags:ALPHA_TEST,
            layer:-1,roughness:1.0,scale:1.0,metal:0.0});
        let vertices=[[-1.,0.,0.],[1.,0.,0.],[0.,2.,0.]].map(|pos|Vertex{pos,normal:[0.,0.,1.],uv:[0.;2]});
        let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2],material}]));
        let root=Vec3::new(7.,2.,-3.);
        scene.add_classified_instance(mesh,glam::Affine3A::from_translation(root),GeometryCategory::Trees);
        scene.mark_foliage();
        let original=scene.instances[0];
        let applied=tree_tuning::Settings{size_scale:1.5,density:1.,color_multiplier:[1.,1.15,0.9]};
        let metadata=restore_trees(&mut scene,Some(applied)).unwrap();
        let changed=scene.instances[0];
        assert_eq!([changed.rows[0][3],changed.rows[1][3],changed.rows[2][3]],root.to_array());
        assert_eq!(changed.rows[0][0],1.5);assert_eq!(changed.rows[1][1],1.5);
        assert_eq!(changed.sphere[3],original.sphere[3]*1.5);
        assert_eq!(scene.materials[material as usize].tint,[0.2,0.4*1.15,0.6*0.9]);
        assert_eq!(metadata["source"]["selected_source_trees"],1);
        restore_trees(&mut scene,Some(tree_tuning::Settings{density:0.,..Default::default()})).unwrap();
        assert_eq!(scene.instances[0].cluster_count,0);
        assert_eq!(scene.instances[0].flags&render::scene::FOLIAGE,0);
    }
    #[test]
    fn construction_guard_rejects_different_blade_budget_and_crown_source() {
        let grass=json!({"mode":"gpu_meadow_field","blade_budget":120,"radius_metres":80.0});
        let trees=json!({"recipe":"stem_anchored_frond_expansion_v1","fullness":1.25});
        let capture=json!({"render":{"procedural_grass":grass,"tree_source":trees}});
        assert!(validate_source_recipe(&capture,Some(&grass),&trees).is_ok());
        let mut bad=grass.clone();bad["blade_budget"]=json!(360);
        assert!(validate_source_recipe(&capture,Some(&bad),&trees).is_err());
        let mut bad=trees.clone();bad["fullness"]=json!(1.0);
        assert!(validate_source_recipe(&capture,Some(&grass),&bad).is_err());
        assert!(validate_source_recipe(&capture,None,&trees).is_err());
        assert!(validate_source_recipe(&json!({"render":{}}),None,&trees).is_ok());
    }
    #[test]
    fn local_light_capture_keeps_recorded_emitters_and_rejects_partial_or_inconsistent_data() {
        let lamp=json!({"position":[1,2,3],"direction":[0,0,1],"intensity_cd":[2.5,0.02,0.0075],
            "range_m":2.2,"inner_cos":0.866,"outer_cos":0.208,"source_radius_m":0.06});
        let capture=json!({"render":{"local_lights":{"enabled":true,"vehicle_count":1,"exhaust_count":0,"lights":[lamp]}}});
        let lights=captured_local_lights(&capture).unwrap();assert_eq!(lights.len(),1);
        assert_eq!(lights[0].position,Vec3::new(1.,2.,3.));assert_eq!(lights[0].range_m,2.2);
        assert_eq!(lights[0].intensity_cd,Vec3::new(2.5,0.02,0.0075));
        for (key,value) in [("source_radius_m",Value::Null),("outer_cos",json!(1)),("intensity_cd",json!([-1,0,0]))] {
            let mut bad=capture.clone();bad["render"]["local_lights"]["lights"][0][key]=value;
            assert!(captured_local_lights(&bad).is_err(),"{key}");
        }
        let mut bad=capture.clone();bad["render"]["local_lights"]["vehicle_count"]=json!(2);
        assert!(captured_local_lights(&bad).is_err());
        let mut bad=capture.clone();bad["render"]["local_lights"]["enabled"]=json!(false);
        assert!(captured_local_lights(&bad).is_err());
        assert!(captured_local_lights(&json!({"render":{}})).unwrap().is_empty());
    }
    #[test]
    fn paged_warmup_waits_for_updates_with_minimum_and_hard_limit() {
        for frames in 0..3 {assert!(!warmup_finished(true,frames,0));assert!(!warmup_finished(false,frames,0));}
        assert!(warmup_finished(false,3,8));
        assert!(warmup_finished(true,3,0));
        for frames in 3..40 {assert!(!warmup_finished(true,frames,8));}
        assert!(warmup_finished(true,40,8));
        // Missing pages deliberately do not gate convergence: the finite
        // resident budget may never cover every requested receiver page.
        let stats=render::FrameStats {foliage_sun_requested_pages:300,foliage_sun_resident_pages:256,
            foliage_sun_missing_pages:44,foliage_sun_updated_pages:0,..Default::default()};
        assert!(warmup_finished(true,32,stats.foliage_sun_updated_pages));
        assert_eq!(foliage_page_stats(&stats)["missing"],44);
    }
    #[test]
    fn diagnostic_probe_modes_are_mutually_exclusive() {
        for bits in 0u8..8 {assert_eq!(validate_probes(bits&1!=0,bits&2!=0,bits&4!=0).is_ok(),bits.count_ones()<=1);}
    }
    #[test]
    fn sun_focus_arguments_require_exactly_three_finite_coordinates() {
        assert_eq!(sun_focus_point("-107.5, 3, -1500").unwrap(),Vec3::new(-107.5,3.0,-1500.0));
        for invalid in ["", "0,1", "0,1,2,3", "0,1,", "NaN,1,2", "1,inf,2", "1,2,-inf", "1,2,1e40"] {
            assert!(sun_focus_point(invalid).is_err(),"{invalid}");
        }
    }
    #[test]
    fn approach_arguments_are_bounded_finite_unique_and_ordered() {
        assert_eq!(approach_distances("0, 20,40,60").unwrap(),vec![0.0,20.0,40.0,60.0]);
        assert_eq!(approach_distances("512").unwrap(),vec![512.0]);
        for invalid in ["", "0,", "NaN", "inf", "-1", "513", "20,0", "0,0", "-0,0",
            "0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16"] {
            assert!(approach_distances(invalid).is_err(),"{invalid}");
        }
    }
    #[test]
    fn approach_translates_position_target_and_focus_without_orbit_or_vertical_motion() {
        let camera=Camera {pos:Vec3::new(7.0,4.0,-20.0),target:Vec3::new(10.0,2.0,-24.0),fov_y:1.1,near:0.1};
        let axis=approach_axis(&camera).unwrap();assert!(axis.abs_diff_eq(Vec3::new(0.6,0.0,-0.8),1e-6));
        let focus=Vec3::new(9.0,2.0,-22.0);let delta=axis*60.0;
        let moved=Camera {pos:camera.pos+delta,target:camera.target+delta,..camera};
        assert!((moved.target-moved.pos).abs_diff_eq(camera.target-camera.pos,1e-6));
        assert!(((focus+delta)-moved.pos).abs_diff_eq(focus-camera.pos,1e-6));
        assert_eq!(moved.pos.y,camera.pos.y);assert_eq!(moved.fov_y,camera.fov_y);assert_eq!(moved.near,camera.near);
        assert!(approach_axis(&Camera {target:camera.pos+Vec3::Y,..camera}).is_err());
    }
    #[test]
    fn terrain_source_native_interpolation_and_material_shell_match_receiver_contract() {
        use render::{scene::{Heightfield,TERRAIN},terrain_trace::{Coverage,Hierarchy}};
        let diagonal=Heightfield {origin:[0.0,0.0],cell:1.0,w:2,h:2,heights:vec![0.0,0.0,0.0,8.0]};
        assert_eq!(source_ground_at(&diagonal,Vec2::splat(0.25)),0.0);
        assert_eq!(source_ground_at(&diagonal,Vec2::splat(0.75)),4.0);
        assert_eq!(source_ground_at(&diagonal,Vec2::new(-0.1,0.5)),-1e9);
        let flat=Hierarchy::with_coverage(&Heightfield {origin:[0.0,0.0],cell:1.0,w:5,h:5,heights:vec![0.0;25]},Coverage::CompleteLoadedTerrain).unwrap();
        let mut surface=json!({"status":"reconstructed_selected_triangle","world_position":[1.0,-0.5,1.0],
            "surface_normal":[0,1,0],"sun_receiver_normal":[0,1,0],"material_flags":TERRAIN});
        let road=terrain_source_query(&flat,&surface,Vec3::Y,1.10,256);
        assert_eq!(road["status"],"AboveTerrainBounds");
        assert!((number(&road["receiver_height_correction_m"]).unwrap()-0.5).abs()<1e-6);
        surface["material_flags"]=json!(0);
        let object=terrain_source_query(&flat,&surface,Vec3::Y,1.10,256);
        assert_eq!(object["status"],"Hit");assert_eq!(object["representation_error_m"],0.0);
        assert_eq!(object["receiver_height_correction_m"],0.0);
        assert!(object["first_blocker"]["position"].is_array());
    }
    #[test]
    fn terrain_source_query_reports_actual_first_ridge_and_bounded_unknown() {
        use render::{scene::Heightfield,terrain_trace::{Coverage,Hierarchy}};
        let mut hf=Heightfield {origin:[0.0,0.0],cell:1.0,w:17,h:5,heights:vec![0.0;85]};
        for z in 0..5 {hf.heights[z*17+8]=2.0;}
        let hierarchy=Hierarchy::with_coverage(&hf,Coverage::CompleteLoadedTerrain).unwrap();
        let surface=json!({"status":"reconstructed_selected_triangle","world_position":[1,0,2],
            "surface_normal":[0,1,0],"sun_receiver_normal":[0,1,0],"material_flags":0});
        let sun=Vec3::new(1.0,0.1,0.0).normalize();
        let result=terrain_source_query(&hierarchy,&surface,sun,1.10,256);
        assert_eq!(result["status"],"Hit");
        let hit=vector(&result["first_blocker"]["position"]).unwrap();
        // Source x=1, ridge's ascending face x=7..8: hit solves
        // 0.025+0.1*(x-1)=2*(x-7), independently of hierarchy traversal.
        assert!((hit.x-13.925/1.9).abs()<1e-4,"{result}");
        assert!(result["visited_nodes"].as_u64().unwrap()>0);
        assert!(result["triangle_tests"].as_u64().unwrap()>0);
        assert!((number(&result["first_blocker"]["distance_from_source_m"]).unwrap()-hit.distance(Vec3::new(1.0,0.0,2.0))).abs()<1e-5);
        let exhausted=terrain_source_query(&hierarchy,&surface,sun,1.10,0);
        assert_eq!(exhausted["status"],"Unknown");assert_eq!(exhausted["reason"],"Budget");
        assert!(exhausted["first_blocker"].is_null());
        let skipped=terrain_source_query(&hierarchy,&json!({"status":"unsupported_gpu_articulated_pose"}),sun,1.10,256);
        assert_eq!(skipped["status"],"Skipped");
    }
    #[test]
    fn selected_triangle_world_hit_matches_nonuniform_pose_and_shared_wind() {
        use render::cluster::{ClusterMesh,Part,Vertex};
        let vertices=[[-3.0,-3.0,-10.0],[3.0,-3.0,-10.0],[0.0,6.0,-10.0]]
            .map(|pos|Vertex {pos,normal:[0.0,0.0,1.0],uv:[0.0,0.0]});
        let mesh=ClusterMesh::build(&[Part {vertices:&vertices,indices:&[0,1,2],material:0}]);
        let mut scene=render::scene::Scene::default();let id=scene.add_mesh(&mesh);
        scene.add_material(render::scene::Material {tint:[1.0;3],layer:-1,roughness:1.0,flags:0,scale:1.0,metal:0.0});
        let root=Vec3::new(2.0,1.0,-30.0);let scale=Vec3::new(2.0,3.0,0.5);
        let owner=scene.instances.len();
        scene.add_instance(id,glam::Affine3A::from_scale_rotation_translation(scale,glam::Quat::IDENTITY,root));
        scene.mark_foliage();
        let wind=render::wind::WindState {strength:0.6,time_seconds:7.0,..Default::default()};
        let camera=Camera {pos:root,target:root-Vec3::Z,fov_y:1.0,near:0.1};
        let value=picked_surface(&scene,&camera,[101,101],(50,50),(owner as u32,0,0),&wind,Vec3::Y,root);
        assert_eq!(value["status"],"reconstructed_selected_triangle");
        let p=vector(&value["world_position"]).unwrap();
        // The triangle centroid is exactly at root height, where the affine
        // wind anchor cannot move it, while its three vertices do sway.
        assert!(p.abs_diff_eq(root-Vec3::Z*5.0,0.001),"{value}");
        assert!((number(&value["camera_distance_m"]).unwrap()-5.0).abs()<0.001);
        assert!(number(&value["ray_residual_m"]).unwrap()<0.001);
        assert_eq!(value["instance_axis_scales"],json!([2.0,3.0,0.5]));
        assert!(vector(&value["barycentric"]).unwrap().abs_diff_eq(Vec3::splat(1.0/3.0),0.001));
        let shear=wind.sample_shear(root,false);
        let expected_normal=Vec3::new(0.0,-shear.y,1.0).normalize();
        assert!(vector(&value["surface_normal"]).unwrap().abs_diff_eq(expected_normal,1e-6));
        assert!(vector(&value["geometric_normal"]).unwrap().dot(expected_normal)>0.99999);
        scene.instances[owner].flags|=render::scene::PART_WIND;
        assert_eq!(picked_surface(&scene,&camera,[101,101],(50,50),(owner as u32,0,0),&wind,Vec3::Y,root)["status"],
            "unsupported_gpu_articulated_pose");
    }
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
    fn captured_night_settings_are_explicit_and_old_captures_keep_the_zero_floor() {
        let mut capture=mist_capture();
        let (old,_,_,_)=sky_spec(&capture).unwrap().unwrap();
        assert_eq!(old.night.ambient_rgb_lux,DVec3::ZERO);assert_eq!(old.night.star_radiance,0.0);
        capture["render"]["sky"]["night_environment"]=json!({
            "configured_ambient_rgb_lux":[1.0,1.5,2.0],"configured_star_radiance":120.0,"star_seed":73});
        let (model,_,_,_)=sky_spec(&capture).unwrap().unwrap();
        assert_eq!(model.night.ambient_rgb_lux,DVec3::new(1.0,1.5,2.0));
        assert_eq!(model.night.star_radiance,120.0);assert_eq!(model.night.star_seed,73);
        capture["render"]["sky"]["night_environment"]["star_seed"]=json!(16_777_216);
        assert!(sky_spec(&capture).is_err());
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
