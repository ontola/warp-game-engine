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

pub fn run(args: &Args) -> Result<(), String> {
    let views = args.num("--views", 4usize).clamp(1, 4);
    let (w, h) = args.size((1920, 1080));
    let frames = args.num("--frames", 300usize);
    let t = Instant::now();
    let gpu = Gpu::new()?;
    let t_gpu = ms(t);
    let mut world = World::new(args.num("--seed", 2u64), &[true; 4]);
    for _ in 0..TICK_HZ { world.step(&[]); }
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
    if args.value("--light") == Some("lean") {
        light.gi = 0.0; light.near_shadows = 0.0; light.taa = 0.0; light.contact_occlusion = 0.0; light.reflection_occlusion = 0.0;
    }
    let t = Instant::now();
    let renderer = Renderer::new(&gpu, &vis.scene, &light);
    let t_renderer = ms(t);
    let rects = render::split_rects(views, w, h);
    let prev: Vec<Vec3> = world.players.iter().map(|p| p.pos).collect();
    let cams = |world: &World| -> Vec<ViewRect> { rects.iter().enumerate()
        .map(|(i, &(x, y, vw, vh))| ViewRect { cam: view::camera(world, i, &prev, 1.0), x, y, w: vw, h: vh }).collect() };
    let t = Instant::now();
    renderer.try_render_texture(&gpu, &cams(&world), &light, w, h)?;
    gpu.wait();
    let t_first = ms(t);
    let profiling = renderer.set_async_gpu_profiling(&gpu, true).is_ok();
    let (mut cpu, mut wall, mut gputot) = (Vec::new(), Vec::new(), Vec::new());
    let mut passes: BTreeMap<&'static str, f64> = BTreeMap::new();
    let mut samples = 0usize;
    let empty = args.flag("--empty");
    for f in 0..frames + 30 {
        let t = Instant::now();
        if !empty {
            world.step(&[]);
            vis.absorb(&world);
            vis.pose(&world, &prev, 1.0, 1.0 / 60.0);
            renderer.update_instances(&gpu, vis.first_dynamic, vis.dynamic())?;
            renderer.set_local_lights(&vis.local_lights())?;
        }
        renderer.set_gpu_profile_frame_id(f as u64);
        renderer.try_render_texture(&gpu, &cams(&world), &light, w, h)?;
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
