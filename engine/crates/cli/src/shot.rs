//! `sideways shot out.png`: run the race to a tick, then render one chase
//! view offscreen. Prints what the frame cost and what it drew.

use crate::visuals::{self, CarPose};
use glam::{Quat, Vec3};
use kern::Sim;
use render::{Camera, Gpu, Lighting, Renderer, Stopwatch};

pub struct ShotArgs {
    pub upscale: render::upscale::UpscaleMode,
    pub nature: crate::nature::Nature,
    pub out: String,
    pub seed: u64,
    pub km: f64,
    pub at: f64,
    pub frames: u64,
    pub size: (u32, u32),
    pub weather: Option<String>,
    pub lod_pixels: f32,
    pub picks: Vec<(u32, u32)>,
    /// `--set key=value` light overrides.
    pub sets: Vec<String>,
    /// Split-screen views, one bot car each (no rivals when more than one).
    pub views: usize,
    /// Frames rendered for TAA to converge (the camera holds still); 1: no TAA.
    pub taa_frames: u32,
    /// Photographed grass; false is a matched no-cover performance baseline.
    pub ground_cover: bool,
    /// `--cam yaw=DEG,dist=M,height=M,car=N,fov=DEG`: a close camera on a
    /// car (material work) instead of the chase camera.
    pub cam: Option<String>,
    /// `--lineup N`: N copies of car 0's model side by side, cars 0..N in
    /// the car material data, for comparing paints in one frame.
    pub lineup: usize,
    /// Car material data instead of engine/data/car_materials.toml.
    pub car_materials: Option<String>,
    /// `--car-set [N:][part.]key=value` car material overrides.
    pub car_sets: Vec<String>,
    /// Keep running: re-render when the car material data or the shading
    /// shaders (`KERN_SHADER_DIR`) change, with no rebuild.
    pub watch: bool,
}

/// main.gd's player colours (sRGB).
const COLORS: [u32; 8] = [0xd8342c, 0x2f6fd6, 0xe8b21f, 0x2fa35a, 0x8a4fd1, 0xe46c1c, 0xd94f9a, 0x8c969e];

/// Car `i`'s paint, linear: players take COLORS[i], bots COLORS[(i + 3) % 8]
/// (main.gd), darkened and desaturated like car_model.gd's real paint.
fn paint(i: usize, humans: usize) -> Vec3 {
    let hex = COLORS[if i < humans { i } else { (i + 3) % 8 } % 8];
    let c = Vec3::new(((hex >> 16) & 255) as f32, ((hex >> 8) & 255) as f32, (hex & 255) as f32) / 255.0;
    let v = c.max_element();
    // HSV: s *= 0.72, v *= 0.72. Scaling saturation pulls each channel towards v.
    let c = (Vec3::splat(v) - (Vec3::splat(v) - c) * 0.72) * 0.72;
    let lin = |x: f32| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    Vec3::new(lin(c.x), lin(c.y), lin(c.z))
}

/// Everything a frame needs: the scene, light and camera at a moment of a race.
pub struct Setup {
    pub wind: render::wind::WindState,
    pub headlights: Vec<render::headlights::SpotLight>,
    pub lighting_focus: Vec<Vec3>,
    pub rear_lamps: Vec<crate::car_lamps::State>,
    pub exhaust: Vec<render::exhaust::Sprite>,
    pub wheel_effects: crate::wheel_effects::WheelEffects,
    pub scene: render::Scene,
    pub mesh_names: Vec<String>,
    pub light: Lighting,
    /// One chase camera per car.
    pub cams: Vec<Camera>,
    pub log: Vec<String>,
    /// Actual render inputs, written beside a shot for reproducible comparison.
    pub metadata: serde_json::Value,
    pub looks: crate::car_look::Looks,
    pub car_slots: Vec<crate::car_look::Slot>,
}

pub fn prepare(a: &ShotArgs) -> Result<Setup, String> {
    if !(1..=4).contains(&a.views) { return Err("views must be in 1..=4".into()); }
    let mut log = Vec::new();
    let t = Stopwatch::start();
    let mut settings = sideways::race::Settings::default();
    settings.stage_km = a.km;
    settings.start_at = a.at;
    settings.weather = a.weather.clone();
    settings.vegetation = a.nature.vegetation;
    if a.views > 1 {
        settings.bots = a.views;
        settings.rivals = 0;
    }
    let cars = sideways::spec::Cars::load(None).expect("cars");
    let mut race = sideways::race::Race::new(a.seed, settings, cars);
    let mut exhaust=crate::exhaust_fx::ExhaustFx::new(a.seed);exhaust.step(&race);
    let mut wheel_effects=crate::wheel_effects::WheelEffects::new(a.seed);wheel_effects.step(&race);
    let mut timings = kern::Timings::default();
    for _ in 1..a.frames {
        race.step(&mut timings);
        exhaust.step(&race);
        wheel_effects.step(&race);
    }
    log.push(format!("sim to tick {}: {:.0} ms", a.frames, t.ms()));
    let mut poses: Vec<CarPose> = race
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pose = p.car.pose(&race.phys);
            let t = pose.translation;
            let r = pose.rotation;
            let wheels = p
                .car
                .wheels
                .iter()
                .map(|w| crate::car_model::Wheel { anchor: Vec3::new(w.anchor.x, w.anchor.y, w.anchor.z), compression: w.compression as f32, steer: w.steer as f32, spin: w.spin as f32 })
                .collect();
            CarPose {
                pos: Vec3::new(t.x, t.y, t.z),
                rot: Quat::from_xyzw(r.x, r.y, r.z, r.w),
                color: paint(i, a.views.max(1)),
                boxes: p.car.spec.boxes.clone(),
                model: p.car.spec.model.clone(),
                ride: p.car.spec.ride as f32,
                wheels,
            }
        })
        .collect();
    if a.lineup > 0 {
        poses = lineup(&race.stage, &poses[0], a.lineup, a.views.max(1));
    }
    // Chase cameras (src/chase_camera.gd: 5.2 m back, 2.05 m up).
    let mut cams: Vec<Camera> = poses
        .iter()
        .map(|car| {
            let fwd = car.rot * Vec3::NEG_Z;
            let heading = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
            Camera { pos: car.pos - heading * 5.2 + Vec3::Y * 2.05, target: car.pos + Vec3::Y * 0.9 + heading * 4.0, fov_y: 64f32.to_radians(), near: 0.1 }
        })
        .collect();
    if let Some(spec) = &a.cam {
        // With a lineup in split screen, view i looks at car i: one frame
        // compares several materials from the same angle.
        for (i, c) in cams.iter_mut().enumerate() {
            let per_car = a.lineup > 1 && a.views > 1 && !spec.contains("car=");
            *c = close_camera(&if per_car { format!("{spec},car={i}") } else { spec.clone() }, &poses, a.lineup)?;
        }
    }
    let looks = crate::car_look::Looks::load(a.car_materials.as_deref().map(std::path::Path::new), a.car_sets.clone())?;
    let t = Stopwatch::start();
    let camera_positions: Vec<_> = cams.iter().take(a.views).map(|c| c.pos).collect();
    let mut built = visuals::build(&race.stage, &poses, &camera_positions, a.ground_cover, &looks)?;
    log.extend(built.log.iter().map(|l| format!("  {l}")));
    log.push(format!("scene: {:.0} ms", t.ms()));
    let mut light = Lighting::default();
    light.lod_pixels = a.lod_pixels;
    match race.weather {
        "overcast" => {
            light.sun_lux = 25_000.0;
            light.sky_lux = 14_000.0;
            light.shadow_strength = 0.6;
            light.ev100 = 12.5;
            light.fog_start = 50.0;
            light.fog_end = 420.0;
        }
        "mist" => {
            light.sun_lux = 30_000.0;
            light.sky_lux = 15_000.0;
            light.shadow_strength = 0.7;
            light.ev100 = 12.7;
            light.fog_start = 0.0;
            light.fog_end = 1e9;
            light.fog_density = 0.011;
        }
        "wet" => {
            light.sun_lux = 20_000.0;
            light.sky_lux = 12_000.0;
            light.shadow_strength = 0.5;
            light.ev100 = 12.2;
            light.fog_start = 30.0;
            light.fog_end = 320.0;
        }
        _ => {}
    }
    let sky_metadata = crate::procedural_sky::apply_environment(&mut built.scene, &mut light)?;
    for kv in &a.sets {
        light.set(kv)?;
    }
    // Derive lights after every sky/light override, exactly as interactive
    // play does. A night shot must not omit the car's only local light.
    let headlights=crate::car_headlights::from_poses(&poses,&(0..a.views).collect::<Vec<_>>(),&light);
    let lighting_focus=poses.iter().take(a.views).map(|p|p.pos).collect();
    let rear_lamps=crate::car_lamps::from_race(&race,&light,poses.len());
    crate::car_lamps::apply(&mut built.scene.car_materials,&built.car_slots,&rear_lamps)?;
    let mut metadata = serde_json::json!({
        "schema": 1, "renderer": "kern", "seed": a.seed, "km": a.km, "start_at_m": a.at,
        "upscale_requested": a.upscale.as_str(), "sky": sky_metadata,
        "headlights": crate::car_headlights::metadata_for_poses(&headlights,&poses,&(0..a.views).collect::<Vec<_>>()),
        "rear_lamps": crate::car_lamps::metadata(&rear_lamps,&built.car_slots),
        "nature": a.nature.metadata(race.frame as f64 / 120.0),
        "grass_detail_profile": built.scene.grass_detail.label(),
        "crowds": crate::roadside_crowds::metadata(&built.scene),
        "requested_frames": a.frames, "simulation_steps": a.frames.saturating_sub(1),
        "simulation_tick": race.tick(), "round": race.round, "race_phase": race.phase,
        "weather_requested": a.weather, "weather_actual": race.weather,
        "output_size": [a.size.0, a.size.1], "ground_cover": a.ground_cover,
        "taa_frames": a.taa_frames.max(1), "taa_enabled": a.taa_frames > 1 && light.taa > 0.5,
        "light_overrides": a.sets, "lod_pixels_requested": a.lod_pixels, "lod_pixels": light.lod_pixels,
        "light": {"sun_direction": light.sun_dir.to_array(), "sun_lux": light.sun_lux, "sky_lux": light.sky_lux, "ev100": light.ev100},
        "views": view_rects(&cams, a.views, a.size.0, a.size.1).iter().map(|v| serde_json::json!({
            "rect": [v.x, v.y, v.w, v.h], "position": v.cam.pos.to_array(), "target": v.cam.target.to_array(),
            "fov_y_radians": v.cam.fov_y, "near": v.cam.near
        })).collect::<Vec<_>>(),
        "cars": poses.iter().map(|p| serde_json::json!({
            "model": p.model, "position": p.pos.to_array(), "rotation_xyzw": p.rot.to_array(),
            "ride": p.ride, "paint_linear": p.color.to_array(),
            "wheels": p.wheels.iter().map(|w| serde_json::json!({"anchor": w.anchor.to_array(), "compression": w.compression, "steer": w.steer, "spin": w.spin})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    });
    metadata["exhaust"]=exhaust.metadata();metadata["exhaust_disabled_for_lineup"]=(a.lineup>0).into();
    metadata["wheel_effects"]=wheel_effects.metadata();
    metadata["wheel_effects_disabled_for_lineup"]=(a.lineup>0).into();
    if a.lineup>0 {wheel_effects=crate::wheel_effects::WheelEffects::new(a.seed);}
    metadata["procedural_grass"]=built.procedural_grass.clone().unwrap_or(serde_json::Value::Null);
    Ok(Setup { wind: a.nature.wind(race.frame as f64 / 120.0), headlights, lighting_focus, rear_lamps, exhaust:if a.lineup>0{vec![]}else{exhaust.sprites()}, wheel_effects, scene: built.scene, mesh_names: built.mesh_names, light, cams, log, metadata, looks, car_slots: built.car_slots })
}

/// `n` copies of `car` nose to tail along the road, 5.5 m apart, at its
/// distance from the centre line and height above the road: the same model
/// and suspension, so only their materials differ. Car 0 leads.
fn lineup(stage: &sideways::stage::Stage, car: &CarPose, n: usize, humans: usize) -> Vec<CarPose> {
    let at = stage.locate(gdcompat::math::v3(car.pos.x, car.pos.y, car.pos.z), -1);
    let lift = car.pos.y - at.height as f32;
    (0..n)
        .map(|i| {
            let along = at.along + ((n as f64 - 1.0) * 0.5 - i as f64) * 5.5;
            let (c, d) = (stage.point_at(along), stage.dir_at(along));
            let heading = Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::NEG_Z);
            let right = Vec3::new(-heading.z, 0.0, heading.x);
            let pos = Vec3::new(c.x, c.y + lift, c.z) + right * at.offset as f32;
            let wheels = car.wheels.iter().map(|w| crate::car_model::Wheel { anchor: w.anchor, compression: w.compression, steer: 0.0, spin: w.spin }).collect();
            CarPose { pos, rot: Quat::from_rotation_arc(Vec3::NEG_Z, heading), color: paint(i, humans), boxes: car.boxes.clone(), model: car.model.clone(), ride: car.ride, wheels }
        })
        .collect()
}

/// A camera a few metres from a car (or the middle of a lineup), from
/// `yaw=DEG,dist=M,height=M,car=N,fov=DEG`. Yaw 0 looks at the nose,
/// 90 at the right side; height is above the car's origin. `glint=1`
/// instead looks down the sun's mirror direction at the roof, where the
/// coat, flakes and swirl marks catch the sun (yaw then turns away from it).
fn close_camera(spec: &str, poses: &[CarPose], lineup: usize) -> Result<Camera, String> {
    let (mut yaw, mut dist, mut height, mut car, mut fov, mut glint) = (35.0f32, 4.5f32, 1.0f32, 0usize, 40.0f32, false);
    for kv in spec.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("--cam {kv}: expected key=value"))?;
        let x: f32 = v.parse().map_err(|_| format!("--cam {kv}: not a number"))?;
        match k {
            "yaw" => yaw = x,
            "dist" => dist = x,
            "height" => height = x,
            "car" => car = x as usize,
            "fov" => fov = x,
            "glint" => glint = x > 0.0,
            _ => return Err(format!("--cam {k}: expected yaw, dist, height, car, fov or glint")),
        }
    }
    let pose = poses.get(car).ok_or_else(|| format!("--cam car={car}: only {} cars", poses.len()))?;
    let centre = if lineup > 0 && !spec.contains("car=") { poses.iter().map(|p| p.pos).sum::<Vec3>() / poses.len() as f32 } else { pose.pos };
    if glint {
        let sun = Lighting::default().sun_dir;
        let target = centre + Vec3::Y * 0.9;
        let turn = if spec.contains("yaw=") { yaw } else { 0.0 };
        let mirror = Quat::from_rotation_y(-turn.to_radians()) * Vec3::new(-sun.x, sun.y, -sun.z).normalize();
        return Ok(Camera { pos: target + mirror * dist, target, fov_y: fov.to_radians(), near: 0.05 });
    }
    let target = centre + Vec3::Y * 0.35;
    let fwd = pose.rot * Vec3::NEG_Z;
    let dir = Quat::from_rotation_y(-yaw.to_radians()) * Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
    Ok(Camera { pos: centre + dir * dist + Vec3::Y * height, target, fov_y: fov.to_radians(), near: 0.05 })
}

pub fn run(a: &ShotArgs) -> i32 {
    let mut setup = match prepare(a) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shot: {e}");
            return 1;
        }
    };
    for l in &setup.log {
        println!("{l}");
    }
    let t = Stopwatch::start();
    let gpu = match Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("shot: {e}");
            return 1;
        }
    };
    let mut renderer = Renderer::new(&gpu, &setup.scene, &setup.light);
    if let Err(e)=renderer.set_headlights(&setup.headlights) {eprintln!("shot headlights: {e}");return 1;}
    if let Err(e)=renderer.set_exhaust(&gpu,&setup.exhaust) {eprintln!("shot exhaust: {e}");return 1;}
    if let Err(e)=setup.wheel_effects.upload(&gpu,&mut renderer) {eprintln!("shot wheel effects: {e}");return 1;}
    setup.metadata["wheel_effects"]=setup.wheel_effects.metadata();
    if let Err(e)=renderer.set_lighting_focus(&setup.lighting_focus) {eprintln!("shot lighting focus: {e}");return 1;}
    if let Err(e)=renderer.set_upscale(&gpu,a.upscale) {eprintln!("shot upscale: {e}");return 1;}
    if let Err(e)=renderer.set_wind(&gpu,&setup.wind) {eprintln!("shot wind: {e}");return 1;}
    if let Err(e)=renderer.set_animation_time(setup.wind.time_seconds) {eprintln!("shot animation: {e}");return 1;}
    // Held-pose temporal convergence has a fixed cadence, not the wall time
    // spent synchronously reading images back for this diagnostic command.
    renderer.set_frame_time_ms(1000.0/60.0);
    setup.metadata["frame_time_ms"] = serde_json::json!(1000.0f32/60.0);
    println!("gpu upload ({}): {:.0} ms", gpu.info.name, t.ms());
    let code = frame(a, &setup, &gpu, &renderer);
    if code != 0 || !a.watch {
        return code;
    }
    watch(a, setup, &gpu, &mut renderer)
}

/// `--watch`: poll the car material data and the shading shaders; on an
/// edit, apply it to the running renderer and write the shot again. Prints
/// how long after the file's modification the new PNG was written.
fn watch(a: &ShotArgs, setup: Setup, gpu: &Gpu, renderer: &mut Renderer) -> i32 {
    let shader_dir = render::shaders::dir().map(|d| d.display().to_string()).unwrap_or_else(|| "built in (set KERN_SHADER_DIR to edit)".into());
    let data = setup.looks.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "built in".into());
    println!("watching car materials {data} and shaders {shader_dir}; Ctrl-C stops");
    let mut live = crate::car_look::LiveEdit::new(setup.looks.clone());
    loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let t = Stopwatch::start();
        let Some((log, edited)) = live.poll(std::time::Duration::ZERO, gpu, renderer, &setup.car_slots) else { continue };
        log.iter().for_each(|l| println!("{l}"));
        // A paint-data reload starts with unlit lens slots. Restore the held
        // shot's physical controls before the next image is rendered.
        match live.looks().materials(&setup.car_slots).and_then(|mut materials|{
            crate::car_lamps::apply(&mut materials,&setup.car_slots,&setup.rear_lamps)?;
            renderer.set_car_materials(gpu,&materials)
        }) {Ok(())=>{},Err(e)=>{eprintln!("shot lamp state: {e}");return 1;}}
        renderer.reset_history();
        frame(a, &setup, gpu, renderer);
        let since = edited.and_then(|m| std::time::SystemTime::now().duration_since(m).ok()).map(|d| d.as_secs_f64() * 1e3).unwrap_or(0.0);
        println!("edit -> png: {since:.0} ms (reload and render {:.0} ms)", t.ms());
    }
}

/// Render the views, answer picks, and write the PNG with its metadata.
fn frame(a: &ShotArgs, setup: &Setup, gpu: &Gpu, renderer: &Renderer) -> i32 {
    let (w, h) = a.size;
    let t = Stopwatch::start();
    let views = view_rects(&setup.cams, a.views, w, h);
    let mut light = setup.light.clone();
    if a.taa_frames <= 1 {
        light.taa = 0.0;
    }
    let mut first_ms = 0.0;
    for _ in 1..a.taa_frames.max(1) {
        if let Err(e)=renderer.try_render_views(&gpu, &views, &light, w, h, &[]) {eprintln!("shot render: {e}");return 1;}
        if first_ms == 0.0 {
            first_ms = t.ms();
        }
    }
    let (px, stats) = match renderer.try_render_views(&gpu, &views, &light, w, h, &a.picks) {Ok(frame)=>frame,Err(e)=>{eprintln!("shot render: {e}");return 1;}};
    let mut metadata = setup.metadata.clone();
    metadata["upscale"] = crate::upscale::metadata(a.upscale,&renderer.upscale_status());
    metadata["taa_enabled"] = serde_json::json!(a.upscale==render::upscale::UpscaleMode::Native && a.taa_frames>1 && light.taa>0.5);
    metadata["temporal_reconstruction"] = serde_json::json!(if a.upscale==render::upscale::UpscaleMode::Native {"native_taa_setting"} else {"dlss"});
    for (k, p) in stats.picks.iter().enumerate() {
        let (x, y) = a.picks[k];
        match p {
            None => println!("pick {x},{y}: sky"),
            Some((inst, cluster, tri)) => {
                let i = &setup.scene.instances[*inst as usize];
                let c = &setup.scene.clusters[*cluster as usize];
                let mesh = setup.scene.meshes.iter().position(|m| m.first_cluster == i.first_cluster).unwrap_or(usize::MAX);
                let packed = setup.scene.tris[(c.first_tri + tri) as usize];
                let corners: Vec<String> = (0..3)
                    .map(|k| {
                        let v = setup.scene.vertices[setup.scene.cluster_verts[(c.first_vert + ((packed >> (8 * k)) & 0xff)) as usize] as usize].pos;
                        let w = [i.rows[0][0] * v[0] + i.rows[0][1] * v[1] + i.rows[0][2] * v[2] + i.rows[0][3], i.rows[1][0] * v[0] + i.rows[1][1] * v[1] + i.rows[1][2] * v[2] + i.rows[1][3], i.rows[2][0] * v[0] + i.rows[2][1] * v[1] + i.rows[2][2] * v[2] + i.rows[2][3]];
                        format!("({:.1},{:.2},{:.1})", w[0], w[1], w[2])
                    })
                    .collect();
                println!("pick {x},{y}: instance {inst} mesh {mesh} ({}) material {} cluster {cluster} own_err {:.3} tri {}", setup.mesh_names.get(mesh).cloned().unwrap_or_default(), c.material, c.own_error, corners.join(" "));
            }
        }
    }
    let frames = a.taa_frames.max(1);
    println!("{frames} frame(s) {w}x{h}: {:.0} ms wall (first {:.0} ms), {} clusters / {} tris drawn, shadow {} clusters / {} tris", t.ms(), first_ms, stats.clusters, stats.triangles, stats.shadow_clusters, stats.shadow_triangles);
    if !stats.gpu_ms.is_empty() {
        println!("  gpu: {}", stats.gpu_ms.iter().map(|(n, ms)| format!("{n} {ms:.1} ms")).collect::<Vec<_>>().join(", "));
    }
    match render::write_png(std::path::Path::new(&a.out), w, h, &px) {
        Ok(()) => {
            let sidecar = format!("{}.json", a.out);
            if let Err(e) = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string()).and_then(|bytes| std::fs::write(&sidecar, bytes).map_err(|e| e.to_string())) {
                eprintln!("shot metadata {sidecar}: {e}");
                return 1;
            }
            println!("shot: {}", a.out);
            println!("metadata: {sidecar}");
            0
        }
        Err(e) => {
            eprintln!("shot: {e}");
            1
        }
    }
}

/// The first `n` chase cameras laid out as Godot's split screen.
pub fn view_rects(cams: &[Camera], n: usize, w: u32, h: u32) -> Vec<render::ViewRect> {
    render::split_rects(n.max(1), w, h)
        .into_iter()
        .zip(cams.iter().cycle())
        .map(|((x, y, vw, vh), cam)| render::ViewRect { cam: *cam, x, y, w: vw, h: vh })
        .collect()
}
