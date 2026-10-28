//! Engine measurement on a small game scene: startup phases, then steady
//! frames with CPU submit, wall (submit + GPU wait) and per-pass GPU time.
//!
//!   growing-guns bench [--views N] [--size WxH] [--frames N] [--light full|lean] [--empty]
use crate::{view, Args};
use glam::Vec3;
use growing_guns::{World, TICK_HZ};
use render::{Gpu, Renderer, ViewRect};
use std::collections::BTreeMap;
use std::time::Instant;

fn ms(t: Instant) -> f64 { t.elapsed().as_secs_f64() * 1000.0 }

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() { return 0.0; }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// The game's CPU side without a GPU, as Godot's perf_benchmark measures
/// it headless: four bots with a stacked build fight (health topped up each
/// second), 3 s warmup then 15 s of 60 Hz ticks: simulation step, then the
/// scene update (effects, debris, gore, guns, crowd) that feeds the renderer.
/// Godot perf_benchmark's stress build, from the cards both games have
/// (`--build a,b,c`; `--build none` for stock guns).
fn armed_world(args: &Args) -> Result<(World, Vec<growing_guns::cards::Card>), String> {
    let spec = args.value("--build").unwrap_or("rapid_fire,rapid_fire,explosive,explosive,dmg_up");
    let mut world = World::new_with(args.num("--seed", 2u64), &[true; 4], |w| w.modifier_chance = 0.0);
    world.release_cage();
    crate::scripts::attach(&mut world)?;
    let build: Vec<_> = if spec == "none" { Vec::new() } else { spec.split(',')
        .map(|n| world.cards.find(&n.trim().to_lowercase()).ok_or(format!("unknown card {n}"))).collect::<Result<_, _>>()? };
    for i in 0..world.players.len() { for &c in &build { world.give(i, c); } let p = &mut world.players[i]; p.ammo = p.mag_size(); }
    Ok((world, build))
}

/// Top up surviving bots once a second; this does not prevent deaths.
fn top_up(world: &mut World, tick: usize) {
    if tick.is_multiple_of(TICK_HZ as usize) { for p in &mut world.players { if p.alive { p.health = p.max_health(); } } }
}

pub fn sim(args: &Args) -> Result<(), String> {
    let (mut world, build) = armed_world(args)?;
    let t = Instant::now();
    let mut vis = view::Visuals::build(&world);
    let t_scene = ms(t);
    let prev: Vec<Vec3> = world.players.iter().map(|p| p.pos).collect();
    let (warm, measure) = (3 * TICK_HZ as usize, 15 * TICK_HZ as usize);
    let (mut step, mut scene, mut total) = (Vec::new(), Vec::new(), Vec::new());
    let (mut shots, mut booms, mut kills) = (0usize, 0usize, 0usize);
    let chunks0 = world.arena.alive_chunks();
    let mut chunks_at = chunks0;
    for tick in 0..warm + measure {
        top_up(&mut world, tick);
        let t = Instant::now();
        world.step(&[]);
        let a = ms(t);
        let t2 = Instant::now();
        vis.absorb(&world, &[]);
        vis.pose(&world, &prev, 1.0, 1.0 / TICK_HZ as f32);
        let b = ms(t2);
        if tick == warm { chunks_at = world.arena.alive_chunks(); }
        if tick >= warm {
            step.push(a); scene.push(b); total.push(a + b);
            for e in &world.events {
                match e { growing_guns::Event::Shot { .. } => shots += 1, growing_guns::Event::Explosion { .. } => booms += 1, growing_guns::Event::Kill { .. } => kills += 1, _ => {} }
            }
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let secs = measure as f64 / TICK_HZ as f64;
    println!("Warp sim bench | build {:?} | seed {} | arena {} m, {} chunks | scene build {t_scene:.0} ms", build, args.num("--seed", 2u64), world.arena.size, chunks0);
    println!("per tick ms (mean / p50 / p95 / max):");
    for (n, v) in [("sim step", &mut step), ("scene update", &mut scene), ("total", &mut total)] {
        let m = mean(v);
        let mx = v.iter().cloned().fold(0.0, f64::max);
        println!("  {n:<13} {m:7.3} / {:7.3} / {:7.3} / {mx:7.3}", pct(&mut v.clone(), 0.5), pct(v, 0.95));
    }
    println!("events per s: shots {:.0}, explosions {:.1}, kills {:.1}; chunks removed {} (~{:.0}/s); gore pieces {}",
        shots as f64 / secs, booms as f64 / secs, kills as f64 / secs, chunks_at.saturating_sub(world.arena.alive_chunks()), chunks_at.saturating_sub(world.arena.alive_chunks()) as f64 / secs, vis.gore_len());
    Ok(())
}

pub fn run(args: &Args) -> Result<(), String> {
    if let Some(mode) = args.value("--stress") { return stress(args, StressMode::parse(mode)?); }
    if args.flag("--sim") { return sim(args); }
    let views = args.num("--views", 4usize).clamp(1, 4);
    let (w, h) = args.size((1920, 1080));
    let frames = args.num("--frames", 300usize);
    let t = Instant::now();
    let gpu = Gpu::new()?;
    let t_gpu = ms(t);
    let (mut world, build) = armed_world(args)?;
    println!("build {build:?}");
    for t in 0..TICK_HZ as usize { top_up(&mut world, t); world.step(&[]); }
    let t = Instant::now();
    let mut vis = view::Visuals::build(&world);
    if args.flag("--empty") {
        // Engine floor: one cube, nothing else.
        let first = vis.first_dynamic;
        vis.scene.instances.truncate(1);
        vis.first_dynamic = first.min(1);
        vis.scene.sky = None;
    }
    let t_scene = ms(t);
    let mut light = view::lighting();
    for kv in args.list.windows(2).filter(|w| w[0] == "--set").map(|w| w[1].clone()) { light.set(&kv)?; }
    if args.value("--light") == Some("lean") {
        light.gi = 0.0; light.near_shadows = 0.0; light.taa = 0.0; light.contact_occlusion = 0.0; light.reflection_occlusion = 0.0;
    }
    let t = Instant::now();
    let renderer = Renderer::new(&gpu, &vis.scene, &light);
    let t_renderer = ms(t);
    if let Some(p) = renderer.startup_profile() {
        let mut v: Vec<_> = p.sections.iter().filter(|s| s.cpu_ms >= 1.0).map(|s| (s.name, s.cpu_ms)).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!("renderer startup sections ms: {}", v.iter().take(16).map(|(n, t)| format!("{n} {t:.0}")).collect::<Vec<_>>().join(", "));
    }
    let rects = render::split_rects(views, w, h);
    let prev: Vec<Vec3> = world.players.iter().map(|p| p.pos).collect();
    // `--overhead`: Godot perf_benchmark's static camera (0, 18, 18), 35 degrees down.
    let overhead = args.flag("--overhead");
    let cams = |world: &World| -> Vec<ViewRect> { rects.iter().enumerate()
        .map(|(i, &(x, y, vw, vh))| {
            let cam = if overhead {
                let pos = Vec3::new(0.0, 18.0, 18.0);
                let (s, c) = (-35f32).to_radians().sin_cos();
                render::Camera { pos, target: pos + Vec3::new(0.0, s, -c), fov_y: view::FOV_Y, near: 0.05 }
            } else { view::camera(world, i, &prev, 1.0) };
            ViewRect { cam, x, y, w: vw, h: vh }
        }).collect() };
    let t = Instant::now();
    renderer.try_render_texture(&gpu, &cams(&world), &light, w, h)?;
    gpu.wait();
    let t_first = ms(t);
    let output = Renderer::output_texture(&gpu, w, h);
    let profiling = renderer.set_async_gpu_profiling(&gpu, true).is_ok();
    let (mut cpu, mut wall, mut gputot) = (Vec::new(), Vec::new(), Vec::new());
    let mut passes: BTreeMap<&'static str, f64> = BTreeMap::new();
    let mut samples = 0usize;
    let mut profiles = BTreeMap::new();
    let (mut dropped, mut errors) = (0, 0);
    let empty = args.flag("--empty");
    for f in 0..frames + 30 {
        let t = Instant::now();
        if !empty {
            top_up(&mut world, f);
            world.step(&[]);
            if world.events.iter().any(|e| matches!(e, growing_guns::Event::NewArena)) {
                vis.set_arena(&world);
            }
            vis.absorb(&world, &[]);
            vis.pose(&world, &prev, 1.0, 1.0 / 60.0);
            vis.flush(&gpu, &renderer)?;
            renderer.update_instances(&gpu, vis.first_dynamic, vis.dynamic())?;
            renderer.set_local_lights(&vis.local_lights())?;
        }
        renderer.set_gpu_profile_frame_id(f as u64);
        renderer.try_render_into(&gpu, &cams(&world), &light, &output)?;
        let c = ms(t);
        gpu.wait();
        let wl = ms(t);
        if f >= 30 {
            cpu.push(c);
            wall.push(wl);
        }
        let drain = renderer.drain_gpu_profiles(&gpu);
        dropped = drain.dropped;
        errors = drain.errors;
        for s in drain.samples {
            if (30..(frames + 30) as u64).contains(&s.frame_id) { profiles.insert(s.frame_id, s); }
        }
    }
    gpu.wait();
    let drain = renderer.drain_gpu_profiles(&gpu);
    dropped = dropped.max(drain.dropped);
    errors = errors.max(drain.errors);
    for s in drain.samples {
        if (30..(frames + 30) as u64).contains(&s.frame_id) { profiles.insert(s.frame_id, s); }
    }
    for s in profiles.values() {
        gputot.push(s.covered_union_ms);
        for &(n, v) in &s.gpu_ms { *passes.entry(n).or_default() += v; }
        samples += 1;
    }
    if let Some((spans, n)) = renderer.cpu_spans() {
        let total: f64 = spans.iter().map(|s| s.1).sum();
        println!("cpu spans ms per frame ({n} frames, total {total:.2}): {}", spans.iter().map(|(k, v)| format!("{k} {v:.3}")).collect::<Vec<_>>().join(", "));
    }
    println!("GPU {} | {w}x{h}, {views} views, {} instances, light {}", gpu.info.name, vis.scene.instances.len(), args.value("--light").unwrap_or("full"));
    println!("backend {:?}; full scene/FX uploads; GPU coverage {samples}/{frames}, dropped {dropped}, errors {errors}", gpu.info.backend);
    println!("scope: offscreen CPU submit plus serialized GPU completion wait; renderer CPU spans include startup/warmup and exclude simulation/scene/FX uploads");
    println!("startup ms: device {t_gpu:.0}, scene {t_scene:.1}, renderer {t_renderer:.0}, first frame {t_first:.0}");
    println!("frame ms (median / p95): cpu submit {:.2} / {:.2}, wall {:.2} / {:.2}, gpu {:.2} / {:.2}",
        pct(&mut cpu.clone(), 0.5), pct(&mut cpu, 0.95), pct(&mut wall.clone(), 0.5), pct(&mut wall, 0.95),
        pct(&mut gputot.clone(), 0.5), pct(&mut gputot, 0.95));
    if profiling && samples > 0 {
        let mut v: Vec<_> = passes.into_iter().map(|(n, t)| (n, t / samples as f64)).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let line: Vec<String> = v.iter().take(14).map(|(n, t)| format!("{n} {t:.3}")).collect();
        println!("gpu passes ms ({samples} samples, {} distinct): {}", v.len(), line.join(", "));
    } else {
        println!("gpu passes: timestamp queries unavailable");
    }
    Ok(())
}

// These deliberately bounded capacity fixtures use the game's actual tick,
// effects, upload and rendering paths. Their workload is not a Godot replica.
// These sustained workloads are opt-in; the bot fixture remains separate.
const STRESS_WARMUP: usize = 180;
const STRESS_FRAMES: usize = 900;
const STRESS_RESET: usize = 180;

#[derive(Clone, Copy, PartialEq)]
enum StressMode { Bullets, Explosions }

impl StressMode {
    fn parse(s: &str) -> Result<Self, String> {
        match s { "bullets" => Ok(Self::Bullets), "explosions" => Ok(Self::Explosions), _ => Err("--stress must be bullets or explosions".into()) }
    }
    fn name(self) -> &'static str { match self { Self::Bullets => "bullets", Self::Explosions => "explosions" } }
}

#[derive(Default)]
struct StressCounts {
    requested: usize,
    shots: usize,
    explosions: usize,
    impacts: usize,
    kills: usize,
    removed: usize,
    worn: usize,
    resets: usize,
    fallback_targets: usize,
}

struct StressFrame {
    frame: usize,
    // Reset, input/simulation, scene update, FX flush, instance/light upload,
    // render submission, entire CPU submission, CPU plus GPU wait.
    times: [f64; 8],
    counts: StressCounts,
    bullets: usize,
    tracers: usize,
    alive: usize,
    chunks: usize,
    lights: usize,
    particles: usize,
    debris: usize,
    puffs: usize,
    debris_spawned: u64,
}

fn stress_pose(world: &mut World, poses: &[Vec3]) {
    world.phase = growing_guns::Phase::Playing;
    world.release_cage();
    for (i, p) in world.players.iter_mut().enumerate() {
        p.pos = poses[i];
        p.vel = Vec3::ZERO;
        p.yaw = 0.0;
        p.pitch = -0.15;
        p.bloom = 0.0;
        p.health = 1_000_000;
        p.alive = true;
        p.ammo = p.mag_size();
        p.reloading = false;
        p.rifle_cd = 0.0;
        p.offer = None;
    }
}

fn stress(args: &Args, mode: StressMode) -> Result<(), String> {
    use growing_guns::{Event, Input};
    use render::gpu_profile::GpuProfileSample;
    if args.flag("--sim") || args.flag("--empty") {
        return Err("--stress requires the rendered game scene; omit --sim and --empty".into());
    }
    if args.value("--build").is_some() {
        return Err("--stress defines its own weapon stats; omit --build".into());
    }
    let views = args.num("--views", 1usize).clamp(1, 4);
    let (w, h) = args.size((1920, 1080));
    let seed = args.num("--seed", 2u64);
    let mut world = World::new_with(seed, &[false; 4], |w| w.modifier_chance = 0.0);
    // Above the arena geometry: neither missing floor nor knockback can
    // stop the fixture. All four rifles shoot into the visible arena.
    let poses: Vec<_> = (0..4).map(|i| Vec3::new(-9.0 + i as f32 * 6.0, 18.0, 14.0)).collect();
    for p in &mut world.players {
        p.weapon.fire_rate_mult = 32.0; // One possible shot per 60 Hz tick.
        p.weapon.recoil_per_shot = 0.0;
        p.weapon.mag_size_bonus = 195;
    }
    stress_pose(&mut world, &poses);
    let original_arena = world.arena.clone();
    // Revisit live chunks in the central camera footprint, spreading the
    // blasts across intact surfaces instead of repeatedly hitting empty air.
    let targets: Vec<_> = world.arena.chunks.iter().enumerate()
        .filter(|(_, c)| c.center.x.abs() <= 12.0 && c.center.z.abs() <= 12.0 && (-1.0..=10.0).contains(&c.center.y))
        .map(|(i, _)| i).collect();
    if mode == StressMode::Explosions && targets.is_empty() { return Err("stress arena has no visible destruction targets".into()); }
    let mut target_cursor = 0usize;
    let t = Instant::now();
    let gpu = Gpu::new()?;
    let t_gpu = ms(t);
    let t = Instant::now();
    let mut vis = view::Visuals::build(&world);
    let t_scene = ms(t);
    let mut light = view::lighting();
    for kv in args.list.windows(2).filter(|w| w[0] == "--set").map(|w| w[1].clone()) { light.set(&kv)?; }
    if args.value("--light") == Some("lean") {
        light.gi = 0.0; light.near_shadows = 0.0; light.taa = 0.0; light.contact_occlusion = 0.0; light.reflection_occlusion = 0.0;
    }
    let t = Instant::now();
    let renderer = Renderer::new(&gpu, &vis.scene, &light);
    let t_renderer = ms(t);
    let rects = render::split_rects(views, w, h);
    let cameras: Vec<_> = rects.iter().map(|&(x, y, vw, vh)| ViewRect {
        cam: render::Camera { pos: Vec3::new(0.0, 24.0, 34.0), target: Vec3::new(0.0, 6.0, -2.0), fov_y: view::FOV_Y, near: 0.05 },
        x, y, w: vw, h: vh,
    }).collect();
    let output = Renderer::output_texture(&gpu, w, h);
    let t = Instant::now();
    vis.pose(&world, &poses, 1.0, growing_guns::DT);
    vis.flush(&gpu, &renderer)?;
    renderer.update_instances(&gpu, vis.first_dynamic, vis.dynamic())?;
    renderer.set_local_lights(&vis.local_lights())?;
    renderer.try_render_into(&gpu, &cameras, &light, &output)?;
    gpu.wait();
    let t_first = ms(t);
    let profiling = match renderer.set_async_gpu_profiling(&gpu, true) {
        Ok(()) => true,
        Err(e) => { eprintln!("GPU timestamps unavailable: {e}"); false }
    };
    println!("Warp stress {} | seed {seed} | {}x{} native output | {views} views | {} | light {}",
        mode.name(), w, h, gpu.info.name, args.value("--light").unwrap_or("full"));
    println!("fixed 60 Hz simulation: {STRESS_WARMUP} warmup ticks (3 s), {STRESS_FRAMES} measured ticks (15 s); one render per tick, no real-time pacing");
    println!("camera fixed overhead (0,24,34)->(0,6,-2); four pinned players with health/ammo restored per tick; audio/HUD excluded");
    println!("caps: {} rendered tracers, 768 CPU particles, 4096 GPU debris, 16384 effect puffs, 16 local lights; counters below distinguish live bullets from rendered tracers", view::TRACERS);
    match mode {
        StressMode::Bullets => println!("workload: 210 rifle bullets/s, deterministic 7 shots per 2 ticks across four rifles; damage 22, speed 150 m/s, no explosive rounds"),
        StressMode::Explosions => println!("workload: 40 real gameplay explosions/s, radius 6.5 m, damage 140, visible live-chunk targets; arena+debris reset every 180 ticks (3 s) or sooner when targets run out, reset costs included"),
    }
    println!("startup ms: device {t_gpu:.1}, scene {t_scene:.1}, renderer {t_renderer:.1}, first frame {t_first:.1}");
    let mut rows = Vec::with_capacity(STRESS_FRAMES);
    let mut profiles: BTreeMap<usize, GpuProfileSample> = BTreeMap::new();
    let mut profile_dropped = 0;
    let mut profile_errors = 0;
    let mut last_debris_spawned = 0;
    let mut measured_start = Instant::now();
    for f in 0..STRESS_WARMUP + STRESS_FRAMES {
        if f == STRESS_WARMUP { measured_start = Instant::now(); }
        let frame_start = Instant::now();
        let mut counts = StressCounts::default();
        let t = Instant::now();
        let depleted = mode == StressMode::Explosions && f % 3 != 2
            && !targets.iter().any(|&i| world.arena.chunks[i].alive);
        if mode == StressMode::Explosions && f > 0 && (f % STRESS_RESET == 0 || depleted) {
            world.arena = original_arena.clone();
            vis.set_arena(&world);
            target_cursor = 0;
            counts.resets = 1;
        }
        let reset_ms = ms(t);
        let t = Instant::now();
        stress_pose(&mut world, &poses);
        let mut inputs = [Input::default(); 4];
        match mode {
            StressMode::Bullets => {
                // Four on even ticks, three on odd ticks, rotating the idle
                // rifle: exactly 210/s without synthetic Bullet insertion.
                for (i, input) in inputs.iter_mut().enumerate() {
                    input.fire = f % 2 == 0 || i != (f / 2) % 4;
                    counts.requested += usize::from(input.fire);
                }
            }
            StressMode::Explosions if f % 3 != 2 => {
                let index = (0..targets.len()).map(|k| (target_cursor + k) % targets.len())
                    .find(|&k| world.arena.chunks[targets[k]].alive);
                let pos = if let Some(k) = index {
                    target_cursor = (k + 97) % targets.len();
                    let c = &world.arena.chunks[targets[k]];
                    c.center + Vec3::Y * (c.half.y + 0.08)
                } else {
                    counts.fallback_targets = 1;
                    Vec3::new(0.0, 1.0, 0.0)
                };
                world.queue_explosion(pos, 6.5, 140.0, 0);
                counts.requested = 1;
            }
            StressMode::Explosions => {}
        }
        world.step(&inputs);
        let sim_ms = ms(t);
        // Keep the posed fixture and camera stable after collision/gravity.
        // No events or breakage are replaced: real gameplay produces both.
        for (p, &pos) in world.players.iter_mut().zip(&poses) { p.pos = pos; p.vel = Vec3::ZERO; }
        let t = Instant::now();
        vis.absorb(&world, &[]);
        vis.pose(&world, &poses, 1.0, growing_guns::DT);
        let scene_ms = ms(t);
        let t = Instant::now();
        vis.flush(&gpu, &renderer)?;
        let flush_ms = ms(t);
        let t = Instant::now();
        renderer.update_instances(&gpu, vis.first_dynamic, vis.dynamic())?;
        let lights = vis.local_lights();
        renderer.set_local_lights(&lights)?;
        let upload_ms = ms(t);
        light.time = world.tick as f32 * growing_guns::DT;
        renderer.set_gpu_profile_frame_id(f as u64);
        let t = Instant::now();
        renderer.try_render_into(&gpu, &cameras, &light, &output)?;
        let render_ms = ms(t);
        let cpu_ms = ms(frame_start);
        gpu.wait();
        let wall_ms = ms(frame_start);
        let (particles, debris, puffs, debris_spawned) = vis.effect_counts();
        let spawned_this_frame = debris_spawned - last_debris_spawned;
        last_debris_spawned = debris_spawned;
        let drained = renderer.drain_gpu_profiles(&gpu);
        profile_dropped = drained.dropped;
        profile_errors = drained.errors;
        for sample in drained.samples {
            let id = sample.frame_id as usize;
            if (STRESS_WARMUP..STRESS_WARMUP + STRESS_FRAMES).contains(&id) { profiles.insert(id, sample); }
        }
        if f >= STRESS_WARMUP {
            for e in &world.events {
                match e {
                    Event::Shot { .. } => counts.shots += 1,
                    Event::Explosion { .. } => counts.explosions += 1,
                    Event::Impact { .. } => counts.impacts += 1,
                    Event::Kill { .. } => counts.kills += 1,
                    _ => {}
                }
            }
            counts.removed = world.breakage.removed.len();
            counts.worn = world.breakage.worn.len();
            rows.push(StressFrame {
                frame: f,
                times: [reset_ms, sim_ms, scene_ms, flush_ms, upload_ms, render_ms, cpu_ms, wall_ms],
                counts,
                bullets: world.bullets.len(),
                tracers: world.bullets.iter().take(view::TRACERS).filter(|b| b.travelled > 1.0).count(),
                alive: world.players.iter().filter(|p| p.alive).count(),
                chunks: world.arena.alive_chunks(),
                lights: lights.len(),
                particles, debris, puffs, debris_spawned: spawned_this_frame,
            });
        }
    }
    let elapsed = measured_start.elapsed().as_secs_f64();
    gpu.wait();
    let drained = renderer.drain_gpu_profiles(&gpu);
    profile_dropped = profile_dropped.max(drained.dropped);
    profile_errors = profile_errors.max(drained.errors);
    for sample in drained.samples {
        let id = sample.frame_id as usize;
        if (STRESS_WARMUP..STRESS_WARMUP + STRESS_FRAMES).contains(&id) { profiles.insert(id, sample); }
    }
    let sum = |get: fn(&StressCounts) -> usize| rows.iter().map(|r| get(&r.counts)).sum::<usize>();
    let requested = sum(|c| c.requested);
    let shots = sum(|c| c.shots);
    let explosions = sum(|c| c.explosions);
    let seconds = STRESS_FRAMES as f64 / TICK_HZ as f64;
    println!("measured workload over {seconds:.1} simulation s (actual loop elapsed {elapsed:.3} wall s): requested {requested}, shots {shots} ({:.1}/sim-s), explosions {explosions} ({:.1}/sim-s), impacts {}, kills {}",
        shots as f64 / seconds, explosions as f64 / seconds, sum(|c| c.impacts), sum(|c| c.kills));
    println!("chunks removed {} (sum across cycles), worn {}, alive at end {}; arena resets {}, exhausted-target fallbacks {}; all-player-alive frames {}/{}",
        sum(|c| c.removed), sum(|c| c.worn), world.arena.alive_chunks(), sum(|c| c.resets), sum(|c| c.fallback_targets), rows.iter().filter(|r| r.alive == 4).count(), rows.len());
    for (name, get) in [("live bullets", (|r: &StressFrame| r.bullets) as fn(&StressFrame) -> usize),
        ("rendered tracers", |r: &StressFrame| r.tracers), ("local lights", |r: &StressFrame| r.lights),
        ("CPU particles", |r: &StressFrame| r.particles), ("live GPU debris", |r: &StressFrame| r.debris),
        ("uploaded effect puffs", |r: &StressFrame| r.puffs)] {
        let mean = rows.iter().map(|r| get(r) as f64).sum::<f64>() / rows.len() as f64;
        let max = rows.iter().map(get).max().unwrap_or(0);
        println!("  {name}: mean {mean:.1}, max {max}");
    }
    println!("frames at pool cap: tracers {}, CPU particles {}, GPU debris {}, effect puffs {}, lights {}; debris spawned {}",
        rows.iter().filter(|r| r.tracers >= view::TRACERS).count(), rows.iter().filter(|r| r.particles >= 768).count(),
        rows.iter().filter(|r| r.debris >= 4096).count(), rows.iter().filter(|r| r.puffs >= 16384).count(),
        rows.iter().filter(|r| r.lights >= 16).count(), rows.iter().map(|r| r.debris_spawned).sum::<u64>());
    println!("CPU / wait timings ms (mean / median / p95 / max); reset costs included:");
    for (i, name) in ["arena reset", "input + sim", "scene update", "FX flush", "instances/lights", "render submit", "CPU total", "CPU + GPU wait"].iter().enumerate() {
        print_stress_times(name, rows.iter().map(|r| r.times[i]).collect());
    }
    println!("GPU timestamp coverage: {}/{} measured frames, enabled {profiling}, dropped {profile_dropped}, errors {profile_errors}; coverage is not whole-frame FPS", profiles.len(), rows.len());
    print_stress_times("GPU union", profiles.values().map(|p| p.covered_union_ms).collect());
    print_stress_times("GPU interval", profiles.values().map(|p| p.covered_interval_ms).collect());
    let mut passes: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for sample in profiles.values() {
        for &(name, duration) in &sample.gpu_ms { passes.entry(name).or_default().push(duration); }
    }
    let mut ranked: Vec<_> = passes.into_iter().collect();
    ranked.sort_by(|a, b| {
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        avg(&b.1).total_cmp(&avg(&a.1))
    });
    println!("GPU passes ms (mean / median / p95 / max), summed across views; non-additive:");
    for (name, values) in ranked { print_stress_times(name, values); }
    let default_csv = format!("stress-{}-frames.csv", mode.name());
    let csv = args.value("--csv").unwrap_or(&default_csv);
    write_stress_csv(csv, &rows, &profiles)?;
    println!("raw measured frames: {csv}; per-frame GPU passes: {csv}.gpu.csv");
    // Readback is deliberately after every measured frame and profile drain.
    if let Some(path) = args.value("--shot") {
        let path = std::path::Path::new(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        render::write_png(path, w, h, &gpu.read_rgba8(&output))?;
        println!("post-measurement screenshot: {}", path.display());
    }
    let (processed, expected) = match mode { StressMode::Bullets => (shots, 3150), StressMode::Explosions => (explosions, 600) };
    let empty_target_fallbacks = sum(|c| c.fallback_targets);
    if requested != expected || processed != requested || rows.iter().any(|r| r.alive != 4) || empty_target_fallbacks != 0 {
        return Err(format!("stress workload was not sustained: expected {expected}, requested {requested}, processed {processed}, empty-target fallbacks {empty_target_fallbacks}; inspect raw counters"));
    }
    Ok(())
}

fn print_stress_times(name: &str, mut values: Vec<f64>) {
    if values.is_empty() { println!("  {name:<22} unavailable"); return; }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let max = values.iter().copied().fold(0.0, f64::max);
    println!("  {name:<22} {mean:8.3} / {:8.3} / {:8.3} / {max:8.3}", pct(&mut values, 0.5), pct(&mut values, 0.95));
}

fn write_stress_csv(path: &str, rows: &[StressFrame], profiles: &BTreeMap<usize, render::gpu_profile::GpuProfileSample>) -> Result<(), String> {
    use std::io::Write;
    let path = std::path::Path::new(path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    writeln!(out, "frame,measured_tick,simulation_s,reset_ms,sim_ms,scene_ms,fx_flush_ms,upload_ms,render_submit_ms,cpu_total_ms,cpu_gpu_wait_ms,requested,shots,explosions,impacts,kills,chunks_removed,chunks_worn,arena_resets,fallback_targets,live_bullets,rendered_tracers,alive_players,alive_chunks,local_lights,cpu_particles,live_gpu_debris,effect_puffs,debris_spawned,gpu_union_ms,gpu_interval_ms,gpu_instrumented_ms").map_err(|e| e.to_string())?;
    for row in rows {
        write!(out, "{},{},{:.6}", row.frame, row.frame - STRESS_WARMUP, (row.frame + 1) as f64 / TICK_HZ as f64).map_err(|e| e.to_string())?;
        for value in row.times { write!(out, ",{value:.6}").map_err(|e| e.to_string())?; }
        let c = &row.counts;
        for value in [c.requested, c.shots, c.explosions, c.impacts, c.kills, c.removed, c.worn, c.resets, c.fallback_targets,
            row.bullets, row.tracers, row.alive, row.chunks, row.lights, row.particles, row.debris, row.puffs] {
            write!(out, ",{value}").map_err(|e| e.to_string())?;
        }
        write!(out, ",{}", row.debris_spawned).map_err(|e| e.to_string())?;
        if let Some(p) = profiles.get(&row.frame) {
            writeln!(out, ",{:.6},{:.6},{:.6}", p.covered_union_ms, p.covered_interval_ms, p.instrumented_gpu_ms).map_err(|e| e.to_string())?;
        } else { writeln!(out, ",,,").map_err(|e| e.to_string())?; }
    }
    out.flush().map_err(|e| e.to_string())?;
    let file = std::fs::File::create(format!("{}.gpu.csv", path.display())).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    writeln!(out, "frame,pass,gpu_ms").map_err(|e| e.to_string())?;
    for (&frame, profile) in profiles {
        for (name, value) in &profile.gpu_ms { writeln!(out, "{frame},{name},{value:.6}").map_err(|e| e.to_string())?; }
    }
    out.flush().map_err(|e| e.to_string())
}
