//! Growing Guns on Warp.
//!
//!   growing-guns play [--players N] [--total N] [--seed S] [--fullscreen] [--no-kbm]
//!   growing-guns shot --out FILE.png [--views N] [--seed S] [--seconds T] [--size WxH]
//!   growing-guns sim [--seed S] [--seconds T]
mod hud;
mod play;
mod view;

use glam::Vec3;
use growing_guns::{Event, World, TICK_HZ};
use render::{Gpu, ViewRect};

pub struct Args { pub list: Vec<String> }

impl Args {
    pub fn value(&self, key: &str) -> Option<&str> {
        self.list.windows(2).find(|w| w[0] == key).map(|w| w[1].as_str())
    }
    pub fn num<T: std::str::FromStr>(&self, key: &str, default: T) -> T {
        self.value(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
    pub fn flag(&self, key: &str) -> bool { self.list.iter().any(|a| a == key) }
    pub fn size(&self, default: (u32, u32)) -> (u32, u32) {
        self.value("--size").and_then(|v| { let (a, b) = v.split_once('x')?; Some((a.parse().ok()?, b.parse().ok()?)) }).unwrap_or(default)
    }
}

fn main() {
    let list: Vec<String> = std::env::args().skip(1).collect();
    let cmd = list.first().cloned().unwrap_or_else(|| "play".into());
    let args = Args { list };
    let result = match cmd.as_str() {
        "play" => play::run(&args),
        "shot" => shot(&args),
        "sim" => { sim(&args); Ok(()) }
        _ => Err(format!("unknown command {cmd}; use play, shot or sim")),
    };
    if let Err(e) = result {
        eprintln!("growing-guns: {e}");
        std::process::exit(1);
    }
}

/// Bots only, no rendering: rounds, accuracy and tick cost.
fn sim(args: &Args) {
    let seed = args.num("--seed", 1u64);
    let seconds = args.num("--seconds", 120u32);
    let mut w = World::new(seed, &[true; 4]);
    let (mut shots, mut hits, mut rounds) = (0u32, 0u32, 0u32);
    let t = std::time::Instant::now();
    for _ in 0..seconds * TICK_HZ {
        w.step(&[]);
        for e in &w.events {
            match e { Event::Shot { .. } => shots += 1, Event::Hit { .. } => hits += 1, Event::RoundOver { .. } => rounds += 1, _ => {} }
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / (seconds * TICK_HZ) as f64;
    println!("seed {seed}: {seconds} s, {rounds} rounds, {shots} shots, {hits} hits, scores {:?}, {ms:.4} ms per tick, digest {:016x}",
        w.players.iter().map(|p| p.score).collect::<Vec<_>>(), w.digest());
}

/// Headless capture: bots play for a while, then every view is rendered
/// with its HUD and saved. No window is opened.
fn shot(args: &Args) -> Result<(), String> {
    let out = args.value("--out").ok_or("--out FILE.png required")?;
    let views = args.num("--views", 4usize).clamp(1, 4);
    let seed = args.num("--seed", 3u64);
    let seconds = args.num("--seconds", 4.0f32);
    let (w, h) = args.size((1600, 900));
    let mut world = World::new(seed, &[true; 4]);
    let mut prev: Vec<Vec3> = world.players.iter().map(|p| p.pos).collect();
    let mut vis = view::Visuals::build(&world);
    let mut fb = vec![hud::Feedback::default(); 4];
    for _ in 0..(seconds * TICK_HZ as f32) as u32 {
        prev = world.players.iter().map(|p| p.pos).collect();
        world.step(&[]);
        if world.events.contains(&Event::NewArena) { vis = view::Visuals::build(&world); }
        vis.absorb(&world);
        play::feedback(&mut fb, &world, 1.0 / TICK_HZ as f32);
        vis.pose(&world, &prev, 1.0, 1.0 / TICK_HZ as f32);
    }
    let gpu = Gpu::new()?;
    let mut light = view::lighting();
    light.taa = 0.0;
    let renderer = render::Renderer::new(&gpu, &vis.scene, &light);
    renderer.set_local_lights(&vis.local_lights())?;
    let rects = render::split_rects(views, w, h);
    let cams: Vec<ViewRect> = rects.iter().enumerate()
        .map(|(i, &(x, y, vw, vh))| ViewRect { cam: view::camera(&world, i, &prev, 1.0), x, y, w: vw, h: vh }).collect();
    let picture = renderer.try_render_texture(&gpu, &cams, &light, w, h)?;
    let mut list = render::hud::HudDrawList::new([w, h]);
    for (i, &(x, y, vw, vh)) in rects.iter().enumerate() {
        hud::view(&mut list, &world, i, [x as f32, y as f32, vw as f32, vh as f32], &fb[i]);
    }
    hud::global(&mut list, &world, None);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let presenter = render::present::Presenter::new(&gpu, format);
    let mut hud_r = render::hud::HudRenderer::new(&gpu, format)?;
    hud_r.prepare(&gpu, &list)?;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shot"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    presenter.present(&gpu, &picture, &target.create_view(&Default::default()), Some(&hud_r), None)?;
    let rgba = gpu.read_rgba8(&target);
    render::write_png(std::path::Path::new(out), w, h, &rgba)?;
    println!("{out}: {} views, round {}, tick {}, arena {} m {:?}, {} blocks, GPU {}",
        views, world.round, world.tick, world.arena.size, world.arena.variant, world.arena.blocks.len(), gpu.info.name);
    Ok(())
}
