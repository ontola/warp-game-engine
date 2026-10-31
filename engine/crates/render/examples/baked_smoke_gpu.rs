//! Bounded GPU acceptance for baked explosion smoke. No window, no gameplay FPS claim.
//! cargo run --release -p render --example baked_smoke_gpu -- --out build/smoke-probe
use glam::{Affine3A, Quat, Vec3};
use render::fx::{Effects, Puff};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const LIFE: f32 = 4.6;
const AGE: f32 = 0.85;

fn smoke() -> Puff {
    Puff {
        center_kind: [0.0, 0.0, 0.0, 7.0],
        ax: [8.0, 0.0, 0.0, 0.0], ay: [0.0, 8.0, 0.0, 0.0], az: [0.0, 0.0, 8.0, 0.0],
        body: [1.0, 0.97, 0.92, 10.0], timing: [0.0, LIFE, 0.0, 0.0], ..Default::default()
    }
}
fn camera(position: Vec3, target: Vec3) -> Camera {
    Camera { pos: position, target, fov_y: 55f32.to_radians(), near: 0.05 }
}
fn changed(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)| a[..3] != b[..3]).count()
}
fn mae(a: &[u8], b: &[u8]) -> f64 {
    let sum: u64 = a.chunks_exact(4).zip(b.chunks_exact(4))
        .map(|(a,b)| (0..3).map(|c| a[c].abs_diff(b[c]) as u64).sum::<u64>()).sum();
    sum as f64 / ((a.len() / 4 * 3).max(1) as f64)
}
fn capture(renderer: &Renderer, gpu: &Gpu, light: &Lighting, cam: Camera, size: [u32; 2]) -> Result<Vec<u8>, String> {
    renderer.try_render_views(gpu, &[ViewRect { cam, x: 0, y: 0, w: size[0], h: size[1] }], light, size[0], size[1], &[]).map(|r| r.0)
}
fn save(out: &Path, name: &str, size: [u32; 2], rgba: &[u8]) -> Result<(), String> {
    render::write_png(&out.join(format!("{name}.png")), size[0], size[1], rgba)
}
// Exercise a sub-low-resolution-pixel foreground strip and the lifecycle of
// per-view targets. Bloom is disabled here so a halo is not mistaken for a
// compositing leak; the ordinary game-like checks below retain their defaults.
fn adaptive_checks(gpu: &Gpu, light: &Lighting, size: [u32; 2], out: &Path,
    failures: &mut Vec<String>) -> Result<Value, String> {
    let mut scene = Scene::default();
    scene.sky = Some((16,8,vec![[0.4,0.5,0.65,1.0];128]));
    let ground = scene.add_material(render::scene::Material { tint:[0.45,0.48,0.52],layer:-1,roughness:1.0,flags:0,scale:1.0,metal:0.0 });
    let strip_material = scene.add_material(render::scene::Material { tint:[0.22,0.06,0.025],layer:-1,roughness:1.0,flags:0,scale:1.0,metal:0.0 });
    let floor_mesh = scene.add_mesh(&render::primitives::cube(ground).mesh());
    let strip_mesh = scene.add_mesh(&render::primitives::cube(strip_material).mesh());
    scene.add_instance(floor_mesh,Affine3A::from_scale_rotation_translation(Vec3::new(60.0,0.2,60.0),Quat::IDENTITY,Vec3::new(0.0,-0.2,0.0)));
    let cam = camera(Vec3::new(0.0,3.0,12.0),Vec3::new(0.0,3.0,0.0));
    let front_z = 6.06;
    let distance = cam.pos.z-front_z;
    let tan_y = (cam.fov_y*0.5).tan();
    let pixels_per_metre = size[1] as f32/(2.0*tan_y*distance);
    // Four output pixels wide: this is thinner than an eighth-resolution
    // texel, with a slanted edge that changes its phase against the low grid.
    let strip_width = 4.0/pixels_per_metre;
    let rotation = Quat::from_rotation_z(0.23);
    let inverse_rotation=rotation.inverse();
    let strip_local=|x:u32,y:u32,centre:Vec3| {
        let front_z=centre.z+0.06;
        let distance=cam.pos.z-front_z;
        let ndc_x=2.0*(x as f32+0.5)/size[0] as f32-1.0;
        let ndc_y=1.0-2.0*(y as f32+0.5)/size[1] as f32;
        let front=Vec3::new(ndc_x*tan_y*size[0] as f32/size[1] as f32*distance,
            cam.pos.y+ndc_y*tan_y*distance,front_z);
        inverse_rotation*(front-centre)
    };
    let strip_slot = scene.instances.len();
    scene.add_instance(strip_mesh,Affine3A::from_scale_rotation_translation(Vec3::new(strip_width,5.0,0.12),rotation,Vec3::new(0.0,3.0,6.0)));
    let renderer = Renderer::new(gpu,&scene,light);
    renderer.set_debug_options(render::debug_settings::DebugRenderOptions {bloom_enabled:false,..Default::default()})?;
    let swarm = |time: f32| -> Vec<Puff> {
        (0..64).map(|_| {let mut p=smoke();p.timing[0]=time-AGE;p}).collect()
    };
    let one = |time: f32| {let mut p=smoke();p.timing[0]=time-AGE;p};
    let mut edge_cases = Vec::new();
    for (case,x_offset) in [-0.18f32,0.19].into_iter().enumerate() {
        let centre = Vec3::new(x_offset,3.0,6.0);
        scene.update_instance(strip_slot,strip_mesh,Affine3A::from_scale_rotation_translation(Vec3::new(strip_width,5.0,0.12),rotation,centre));
        renderer.update_instances(gpu,strip_slot,&scene.instances[strip_slot..])?;
        let now=6.0+case as f32;
        renderer.set_effects(gpu,now,Some(&[]))?;
        let empty=capture(&renderer,gpu,light,cam,size)?;
        renderer.set_effects(gpu,now,Some(&swarm(now)))?;
        let smoky=capture(&renderer,gpu,light,cam,size)?;
        let mut tested=0usize;
        let mut leaked=0usize;
        let mut boundary_tested=0usize;
        let mut boundary_leaked=0usize;
        for y in 0..size[1] {for x in 0..size[0] {
            let local=strip_local(x,y,centre);
            // A quarter-pixel guard avoids ambiguous raster-edge coverage;
            // the outer 1.25-pixel band still tests the foreground silhouette.
            let edge_distance=strip_width*0.5-local.x.abs();
            if edge_distance>0.25/pixels_per_metre && local.y.abs()<2.4 {
                let i=((y*size[0]+x)*4) as usize;
                let different=empty[i..i+3]!=smoky[i..i+3];
                tested+=1;leaked+=usize::from(different);
                if edge_distance<1.25/pixels_per_metre {boundary_tested+=1;boundary_leaked+=usize::from(different);}
            }
        }}
        if tested<50 || boundary_tested<20 {failures.push(format!("thin occluder mask too small: {tested}, boundary {boundary_tested}"));}
        if leaked!=0 {failures.push(format!("smoke leaked over thin foreground in position {case}: {leaked}/{tested}, boundary {boundary_leaked}/{boundary_tested}"));}
        let visible=changed(&empty,&smoky);
        if visible<100 {failures.push(format!("thin-occluder test did not exercise visible smoke: {visible}"));}
        save(out,&format!("thin-occluder-{case}-empty"),size,&empty)?;
        save(out,&format!("thin-occluder-{case}-smoke"),size,&smoky)?;
        edge_cases.push(json!({"x_offset_m":x_offset,"masked_foreground_pixels":tested,"changed_foreground_pixels":leaked,
            "masked_boundary_pixels":boundary_tested,"changed_boundary_pixels":boundary_leaked,"visible_changed_pixels":visible}));
    }

    // A background strip must not cut a hole through smoke in front of it.
    // The full smoke cube ends at z=-4; the strip's nearest face is z=-5.94.
    // Find opaque smoke independently: doubling 32 layers to 64 changes RGB
    // by at most one code value, while smoke changes the empty scene by >=12.
    scene.update_instance(strip_slot,strip_mesh,Affine3A::from_scale_rotation_translation(
        Vec3::new(strip_width,5.0,0.12),rotation,Vec3::new(1000.0,3.0,-6.0)));
    renderer.update_instances(gpu,strip_slot,&scene.instances[strip_slot..])?;
    renderer.set_effects(gpu,8.0,Some(&[]))?;
    let unobstructed_empty=capture(&renderer,gpu,light,cam,size)?;
    let dense_swarm=swarm(8.0);
    renderer.set_effects(gpu,8.0,Some(&dense_swarm[..32]))?;
    let density_reference=capture(&renderer,gpu,light,cam,size)?;
    renderer.set_effects(gpu,8.0,Some(&dense_swarm))?;
    let unobstructed_smoke=capture(&renderer,gpu,light,cam,size)?;
    save(out,"thin-background-unobstructed",size,&unobstructed_smoke)?;
    let rear_pixels_per_metre=size[1] as f32/(2.0*tan_y*(cam.pos.z+5.94));
    let rear_width=4.0/rear_pixels_per_metre;
    let mut rear_cases=Vec::new();
    for (case,x_offset) in [-0.18f32,0.19].into_iter().enumerate() {
        let centre=Vec3::new(x_offset,3.0,-6.0);
        scene.update_instance(strip_slot,strip_mesh,Affine3A::from_scale_rotation_translation(
            Vec3::new(rear_width,5.0,0.12),rotation,centre));
        renderer.update_instances(gpu,strip_slot,&scene.instances[strip_slot..])?;
        let now=8.1+case as f32*0.1;
        renderer.set_effects(gpu,now,Some(&[]))?;
        let empty=capture(&renderer,gpu,light,cam,size)?;
        renderer.set_effects(gpu,now,Some(&swarm(now)))?;
        let smoky=capture(&renderer,gpu,light,cam,size)?;
        let mut tested=0usize;
        let mut unchanged=0usize;
        let mut mismatch=0usize;
        let mut max_reference_delta=0u8;
        for y in 0..size[1] {for x in 0..size[0] {
            let local=strip_local(x,y,centre);
            if rear_width*0.5-local.x.abs()<=0.25/rear_pixels_per_metre || local.y.abs()>=2.4 {continue;}
            let i=((y*size[0]+x)*4) as usize;
            let convergence=(0..3).map(|c|unobstructed_smoke[i+c].abs_diff(density_reference[i+c])).max().unwrap();
            let signal=(0..3).map(|c|unobstructed_smoke[i+c].abs_diff(unobstructed_empty[i+c])).max().unwrap();
            if convergence>1 || signal<12 {continue;}
            tested+=1;
            unchanged+=usize::from(empty[i..i+3]==smoky[i..i+3]);
            let delta=(0..3).map(|c|smoky[i+c].abs_diff(unobstructed_smoke[i+c])).max().unwrap();
            max_reference_delta=max_reference_delta.max(delta);
            mismatch+=usize::from(delta>6);
        }}
        if tested<20 {failures.push(format!("background strip dense-smoke mask too small in position {case}: {tested}"));}
        if unchanged!=0 || mismatch!=0 {failures.push(format!("background strip cut through dense smoke in position {case}: {unchanged}/{tested} unchanged, {mismatch} differ from unobstructed smoke by >6, max {max_reference_delta}"));}
        save(out,&format!("thin-background-{case}-empty"),size,&empty)?;
        save(out,&format!("thin-background-{case}-smoke"),size,&smoky)?;
        rear_cases.push(json!({"x_offset_m":x_offset,"masked_dense_smoke_pixels":tested,"unchanged_pixels":unchanged,
            "reference_mismatch_pixels":mismatch,"max_reference_rgb_delta":max_reference_delta}));
    }
    // Keep the original foreground scene for the target-lifecycle checks.
    scene.update_instance(strip_slot,strip_mesh,Affine3A::from_scale_rotation_translation(
        Vec3::new(strip_width,5.0,0.12),rotation,Vec3::new(0.19,3.0,6.0)));
    renderer.update_instances(gpu,strip_slot,&scene.instances[strip_slot..])?;

    // Resize while overloaded, then lower the load at the same final size.
    // Keep smoke's age constant while advancing the policy clock past its
    // recovery hysteresis, so a fresh renderer is an exact image reference.
    let close=camera(Vec3::new(0.0,2.0,0.0),Vec3::new(0.0,3.0,-5.0));
    renderer.set_effects(gpu,10.0,Some(&swarm(10.0)))?;
    capture(&renderer,gpu,light,close,size)?;
    capture(&renderer,gpu,light,close,[size[0]/2,size[1]/2])?;
    capture(&renderer,gpu,light,close,size)?;
    renderer.set_effects(gpu,10.1,Some(&[one(10.1)]))?;
    capture(&renderer,gpu,light,cam,size)?;
    renderer.set_effects(gpu,10.5,Some(&[one(10.5)]))?;
    let restored=capture(&renderer,gpu,light,cam,size)?;
    let fresh=Renderer::new(gpu,&scene,light);
    fresh.set_debug_options(render::debug_settings::DebugRenderOptions {bloom_enabled:false,..Default::default()})?;
    fresh.set_effects(gpu,10.5,Some(&[one(10.5)]))?;
    let reference=capture(&fresh,gpu,light,cam,size)?;
    let resize_delta=changed(&restored,&reference);
    if resize_delta!=0 {failures.push(format!("adaptive target/quality recovery differs from fresh renderer: {resize_delta} pixels"));}
    save(out,"adaptive-restored",size,&restored)?;

    renderer.set_effects(gpu,11.0,Some(&swarm(11.0)))?;
    capture(&renderer,gpu,light,close,size)?;
    renderer.set_effects(gpu,AGE,Some(&[smoke()]))?;
    fresh.set_effects(gpu,AGE,Some(&[smoke()]))?;
    let rewind_delta=changed(&capture(&renderer,gpu,light,cam,size)?,&capture(&fresh,gpu,light,cam,size)?);
    if rewind_delta!=0 {failures.push(format!("effect-clock rewind retained stale adaptive state: {rewind_delta} pixels"));}
    renderer.set_effects(gpu,AGE,Some(&[]))?;
    fresh.set_effects(gpu,AGE,Some(&[]))?;
    let removed_delta=changed(&capture(&renderer,gpu,light,cam,size)?,&capture(&fresh,gpu,light,cam,size)?);
    if removed_delta!=0 {failures.push(format!("removed adaptive smoke retained color: {removed_delta} pixels"));}
    Ok(json!({"thin_foreground":{"output_width_pixels":4,"bloom_enabled":false,"smoke_volumes":64,"cases":edge_cases},
        "thin_background":{"output_width_pixels":4,"smoke_volumes":64,"nearest_depth_metres":17.94,
            "dense_mask_max_32_to_64_rgb_delta":1,"dense_mask_min_signal":12,"max_reference_rgb_tolerance":6,"cases":rear_cases},
        "resize_recovery_changed_pixels":resize_delta,"rewind_changed_pixels":rewind_delta,"removed_changed_pixels":removed_delta}))
}

fn quantiles(values: &mut [f64]) -> Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() { return Value::Null; }
    let n = values.len();
    json!({"samples": n, "median_ms": values[n / 2], "p95_ms": values[((n as f64 * 0.95).ceil() as usize - 1).min(n-1)], "min_ms": values[0], "max_ms": values[n-1]})
}

fn timings(gpu: &Gpu, renderer: &Renderer, light: &Lighting, cams: &[Camera; 4], size: [u32; 2], samples: usize) -> Result<Value, String> {
    if !gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
        return Ok(json!({"available":false,"reason":"adapter lacks encoder timestamp queries; no wall-time substitute"}));
    }
    let mut result = Vec::new();
    for count in [1, 4] {
        let views: Vec<_> = render::split_rects(count, size[0], size[1]).iter().zip(cams).map(|(r,c)|
            ViewRect { cam:*c,x:r.0,y:r.1,w:r.2,h:r.3 }).collect();
        for mode in ["baseline", "legacy", "baked"] {
            renderer.set_effects(gpu, AGE, Some(&[]))?;
            if mode != "baseline" {
                let mut fx = Effects::default();
                fx.set_baked_smoke(mode == "baked");
                fx.explosion(0.0, Vec3::ZERO, 3.3, Vec3::new(1.0, 0.7, 0.2));
                fx.flush(gpu, renderer, AGE)?;
            }
            let mut by_pass: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
            // Shader compilation, allocation and all twelve warmup renders are excluded.
            for frame in 0..samples + 12 {
                let (_, stats) = renderer.try_render_texture_profiled(gpu, &views, light, size[0], size[1])?;
                if frame >= 12 {
                    for (name, milliseconds) in stats.gpu_ms {
                        if milliseconds.is_finite() && milliseconds >= 0.0 { by_pass.entry(name).or_default().push(milliseconds); }
                    }
                }
            }
            let passes: BTreeMap<_, _> = by_pass.into_iter().map(|(name, mut times)| (name, quantiles(&mut times))).collect();
            println!("timing {mode}, {count} views: fx {}", passes.get("fx").unwrap_or(&Value::Null));
            result.push(json!({"mode":mode,"views":count,"output_size":size,"per_pass":passes}));
        }
    }
    Ok(json!({"available":true,"warmup_frames_per_case":12,"frames_per_case":samples,
        "scope":"GPU pass timestamps in a bounded static fixture; readback, compilation and startup excluded; not gameplay FPS or 4K acceptance",
        "cases":result}))
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let arg = |key: &str| args.windows(2).find(|a| a[0] == key).map(|a| a[1].as_str());
    let out = Path::new(arg("--out").unwrap_or("build/smoke-probe"));
    let size: [u32; 2] = match arg("--size") {
        Some(s) => { let (w,h)=s.split_once('x').ok_or("size must be WIDTHxHEIGHT")?; [w.parse().map_err(|_|"invalid width")?,h.parse().map_err(|_|"invalid height")?] },
        None => [1280,720],
    };
    if size.iter().any(|v| *v < 128 || *v > 4096 || v % 2 != 0) { return Err("size must be even and 128..4096 per axis".into()); }
    let samples: usize = arg("--samples").unwrap_or("24").parse().map_err(|_|"invalid samples")?;
    if !(8..=240).contains(&samples) { return Err("samples must be 8..240".into()); }
    std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
    let mut gpu = Gpu::new()?;
    let no_timings=args.iter().any(|a|a=="--no-timings");
    if no_timings { gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS); }
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.4, 0.5, 0.65, 1.0]; 128]));
    let mat = scene.add_material(render::scene::Material { tint:[0.45,0.48,0.52],layer:-1,roughness:1.0,flags:0,scale:1.0,metal:0.0 });
    let mesh = scene.add_mesh(&render::primitives::cube(mat).mesh());
    for (size,position) in [(Vec3::new(60.0,0.2,60.0),Vec3::new(0.0,-0.2,0.0)),
        (Vec3::new(40.0,20.0,0.2),Vec3::new(0.0,7.0,-7.0)),
        (Vec3::new(0.2,20.0,40.0),Vec3::new(-9.0,7.0,0.0))] {
        scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(size, Quat::IDENTITY, position));
    }
    let blocker_slot = scene.instances.len();
    scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(Vec3::new(100.0,100.0,0.2),Quat::IDENTITY,Vec3::new(1000.0,0.0,6.0)));
    let light = Lighting { taa:0.0,ev100:10.0,sun_lux:800.0,sky_lux:600.0,gi:0.0,shadow_strength:0.0,near_shadows:0.0,
        contact_occlusion:0.0,reflection_occlusion:0.0,fog_density:0.0,sharpen:0.0,lod_pixels:0.0,time:1.0,..Default::default() };
    let renderer = Renderer::new(&gpu,&scene,&light);
    let cams = [camera(Vec3::new(10.0,6.0,15.0),Vec3::new(0.0,3.0,0.0)),
        camera(Vec3::new(-7.0,5.0,12.0),Vec3::new(0.0,3.0,0.0)),
        camera(Vec3::new(0.0,2.0,0.0),Vec3::new(0.0,3.0,-5.0)),
        camera(Vec3::new(0.0,9.0,1.0),Vec3::new(0.1,2.0,0.0))];
    let mut failures = Vec::new();
    let mut checks = serde_json::Map::new();
    renderer.set_effects(&gpu,AGE,Some(&[]))?;
    let base = capture(&renderer,&gpu,&light,cams[0],size)?;
    save(out,"baseline",size,&base)?;
    renderer.set_effects(&gpu,AGE,Some(&[smoke()]))?;
    let active = capture(&renderer,&gpu,&light,cams[0],size)?;
    save(out,"smoke",size,&active)?;
    let visible = changed(&active,&base);
    if visible < 100 { failures.push(format!("outside smoke missing: {visible} changed pixels")); }
    checks.insert("outside_changed_pixels".into(),json!(visible));

    for (name,time) in [("before-birth",-0.1),("expired",LIFE),("after-expiry",LIFE+0.1)] {
        renderer.set_effects(&gpu,time,Some(&[smoke()]))?;
        let pixels = capture(&renderer,&gpu,&light,cams[0],size)?;
        let n = changed(&pixels,&base);
        if n != 0 { failures.push(format!("{name} affected {n} pixels")); }
        checks.insert(name.into(),json!({"changed_pixels":n}));
    }
    renderer.set_effects(&gpu,AGE,Some(&[]))?;
    let inside_off = capture(&renderer,&gpu,&light,cams[2],size)?;
    renderer.set_effects(&gpu,AGE,Some(&[smoke()]))?;
    let inside = capture(&renderer,&gpu,&light,cams[2],size)?;
    let n = changed(&inside,&inside_off);
    if n < 100 { failures.push(format!("camera inside smoke missing: {n} pixels")); }
    checks.insert("inside_changed_pixels".into(),json!(n));
    save(out,"inside",size,&inside)?;

    let mut time_frames = Vec::new();
    for (name,time) in [("early",0.35),("middle",1.1),("late",2.0),("tail",LIFE-0.01)] {
        renderer.set_effects(&gpu,time,Some(&[smoke()]))?;
        let pixels = capture(&renderer,&gpu,&light,cams[0],size)?;
        save(out,name,size,&pixels)?;
        time_frames.push((name,mae(&pixels,&base),pixels));
    }
    let animation_delta = mae(&time_frames[0].2,&time_frames[1].2);
    if animation_delta < 0.01 { failures.push(format!("density animation is visually static: MAE {animation_delta}")); }
    let tail_mae = time_frames[3].1;
    let peak_mae = time_frames[..3].iter().map(|f|f.1).fold(0.0,f64::max);
    if tail_mae > (peak_mae * 0.05).max(0.05) { failures.push(format!("visible hard cutoff risk: tail MAE {tail_mae}, peak {peak_mae}")); }
    checks.insert("animation".into(),json!({"early_middle_mae":animation_delta,"tail_mae":tail_mae,"peak_mae":peak_mae}));

    // Measure the rendered tail, including the former 2.6 s cutoff. Checking
    // only the last frame misses a plume that dissipates much too early.
    let mut tail_curve = Vec::new();
    let mut previous_mae: Option<f64> = None;
    let mut largest_drop = 0.0f64;
    let mut remaining_at_old_end = 0.0;
    let mut remaining_at_three_two = 0.0;
    for sample in 0..=29 {
        let time = 1.7 + sample as f32 * 0.1;
        renderer.set_effects(&gpu,time,Some(&[smoke()]))?;
        let pixels = capture(&renderer,&gpu,&light,cams[0],size)?;
        let contrast = mae(&pixels,&base);
        let n = changed(&pixels,&base);
        if let Some(previous) = previous_mae { largest_drop = largest_drop.max(previous-contrast); }
        previous_mae = Some(contrast);
        if sample == 9 { remaining_at_old_end = contrast; save(out,"tail-2.6s",size,&pixels)?; }
        if sample == 15 { remaining_at_three_two = contrast; save(out,"tail-3.2s",size,&pixels)?; }
        if sample == 21 { save(out,"tail-3.8s",size,&pixels)?; }
        if sample == 27 { save(out,"tail-4.4s",size,&pixels)?; }
        tail_curve.push(json!({"time_seconds":time,"mae_to_empty":contrast,"changed_pixels":n}));
    }
    if remaining_at_old_end < (peak_mae*0.08).max(0.005) {
        failures.push(format!("smoke still disappears at the old cutoff: 2.6 s MAE {remaining_at_old_end}, peak {peak_mae}"));
    }
    if remaining_at_three_two < (peak_mae*0.02).max(0.002) {
        failures.push(format!("extended wisps missing: 3.2 s MAE {remaining_at_three_two}, peak {peak_mae}"));
    }
    // A generous screen-space continuity bound catches a dropped animation
    // frame or lifetime pop without requiring every moving wisp to be monotone.
    if largest_drop > (peak_mae*0.20).max(0.025) {
        failures.push(format!("abrupt tail contrast drop over 0.1 s: {largest_drop}, peak {peak_mae}"));
    }
    checks.insert("tail".into(),json!({"old_cutoff_seconds":2.6,"lifetime_seconds":LIFE,
        "sample_interval_seconds":0.1,"largest_0_1s_mae_drop":largest_drop,
        "remaining_at_old_cutoff_mae":remaining_at_old_end,"remaining_at_3_2s_mae":remaining_at_three_two,
        "curve":tail_curve}));

    scene.update_instance(blocker_slot,mesh,Affine3A::from_scale_rotation_translation(Vec3::new(100.0,100.0,0.2),Quat::IDENTITY,Vec3::new(0.0,0.0,6.0)));
    renderer.update_instances(&gpu,blocker_slot,&scene.instances[blocker_slot..])?;
    renderer.set_effects(&gpu,AGE,Some(&[]))?;
    let blocked_off = capture(&renderer,&gpu,&light,cams[0],size)?;
    renderer.set_effects(&gpu,AGE,Some(&[smoke()]))?;
    let blocked = capture(&renderer,&gpu,&light,cams[0],size)?;
    let n = changed(&blocked,&blocked_off);
    if n != 0 { failures.push(format!("smoke leaked through opaque foreground: {n} pixels")); }
    checks.insert("occluded_changed_pixels".into(),json!(n));
    save(out,"occluded",size,&blocked)?;
    scene.update_instance(blocker_slot,mesh,Affine3A::from_scale_rotation_translation(Vec3::new(100.0,100.0,0.2),Quat::IDENTITY,Vec3::new(1000.0,0.0,6.0)));
    renderer.update_instances(&gpu,blocker_slot,&scene.instances[blocker_slot..])?;

    let views: Vec<_> = render::split_rects(4,size[0],size[1]).iter().zip(cams).map(|(r,cam)| ViewRect {cam,x:r.0,y:r.1,w:r.2,h:r.3}).collect();
    let (split,_) = renderer.try_render_views(&gpu,&views,&light,size[0],size[1],&[])?;
    let mut split_deltas = Vec::new();
    for (i,v) in views.iter().enumerate() {
        let single_renderer=Renderer::new(&gpu,&scene,&light);
        single_renderer.set_effects(&gpu,AGE,Some(&[smoke()]))?;
        let single = capture(&single_renderer,&gpu,&light,v.cam,[v.w,v.h])?;
        let mut tile = Vec::with_capacity(single.len());
        for y in 0..v.h { let start=(((v.y+y)*size[0]+v.x)*4) as usize; tile.extend_from_slice(&split[start..start+(v.w*4)as usize]); }
        let n=changed(&single,&tile);
        if n != 0 { failures.push(format!("split view {i} differs from equivalent single view: {n} pixels")); }
        split_deltas.push(n);
    }
    checks.insert("split_changed_pixels".into(),json!(split_deltas));
    save(out,"four-views",size,&split)?;

    // Effects must not remain in either history after the effect is removed.
    let a=Renderer::new(&gpu,&scene,&light);
    let b=Renderer::new(&gpu,&scene,&light);
    let temporal=Lighting {taa:1.0,..light.clone()};
    a.set_effects(&gpu,AGE,Some(&[smoke()]))?;
    for _ in 0..3 {capture(&a,&gpu,&temporal,cams[0],size)?;capture(&b,&gpu,&temporal,cams[0],size)?;}
    a.set_effects(&gpu,AGE,Some(&[]))?;
    let n=changed(&capture(&a,&gpu,&temporal,cams[0],size)?,&capture(&b,&gpu,&temporal,cams[0],size)?);
    if n!=0 {failures.push(format!("smoke contaminated TAA history: {n} pixels"));}
    checks.insert("taa_history_changed_pixels".into(),json!(n));

    checks.insert("adaptive".into(),adaptive_checks(&gpu,&light,size,out,&mut failures)?);
    let timing=if no_timings {json!({"available":false,"reason":"explicitly disabled by --no-timings"})}
        else {timings(&gpu,&renderer,&light,&cams,size,samples)?};
    let report=json!({"scope":"bounded native smoke renderer fixture, not full Growing Guns gameplay or target FPS acceptance",
        "adapter":{"name":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"driver":gpu.info.driver},
        "size":size,"rendering":"native output, HDR, no reconstruction; TAA only in history check",
        "smoke_quality":std::env::var("WARP_SMOKE_QUALITY").unwrap_or_else(|_|"adaptive".into()),
        "checks":checks,"timing":timing,"passed":failures.is_empty(),"failures":failures});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if failures.is_empty() {println!("PASS: smoke visibility, occlusion, lifetime, animation, gradual tail, adaptive depth edges/targets, four views and temporal-history isolation");Ok(())}
    else {Err(failures.join("\n"))}
}
fn main() {if let Err(error)=run(){eprintln!("baked smoke fixture: {error}");std::process::exit(1);}}
