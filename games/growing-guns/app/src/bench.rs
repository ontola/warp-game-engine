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
    use growing_guns::cards::ALL;
    let spec = args.value("--build").unwrap_or("rapidfire,rapidfire,explosive,explosive,damage");
    let build: Vec<_> = if spec == "none" { Vec::new() } else { spec.split(',')
        .map(|n| ALL.iter().copied().find(|c| format!("{c:?}").eq_ignore_ascii_case(n.trim())).ok_or(format!("unknown card {n}"))).collect::<Result<_, _>>()? };
    let mut world = World::new(args.num("--seed", 2u64), &[true; 4]);
    for p in &mut world.players { for &c in &build { p.mods.apply(c); p.cards.push(c); } p.ammo = p.mag_size(); }
    Ok((world, build))
}

/// Bots cannot die (Godot tops their health up every second).
fn top_up(world: &mut World, tick: usize) {
    if tick % TICK_HZ as usize == 0 { for p in &mut world.players { if p.alive { p.health = p.max_health(); } } }
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
    let empty = args.flag("--empty");
    for f in 0..frames + 30 {
        let t = Instant::now();
        if !empty {
            top_up(&mut world, f);
            world.step(&[]);
            vis.absorb(&world, &[]);
            vis.pose(&world, &prev, 1.0, 1.0 / 60.0);
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
            for s in renderer.drain_gpu_profiles(&gpu).samples {
                gputot.push(s.covered_union_ms);
                for (n, v) in s.gpu_ms { *passes.entry(n).or_default() += v; }
                samples += 1;
            }
        } else {
            renderer.drain_gpu_profiles(&gpu);
        }
    }
    if let Some((spans, n)) = renderer.cpu_spans() {
        let total: f64 = spans.iter().map(|s| s.1).sum();
        println!("cpu spans ms per frame ({n} frames, total {total:.2}): {}", spans.iter().map(|(k, v)| format!("{k} {v:.3}")).collect::<Vec<_>>().join(", "));
    }
    println!("GPU {} | {w}x{h}, {views} views, {} instances, light {}", gpu.info.name, vis.scene.instances.len(), args.value("--light").unwrap_or("full"));
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
