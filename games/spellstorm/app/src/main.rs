//! Spellstorm on Warp.
//!
//!   spellstorm shot --out frame.png [--seed 7] [--bots 3] [--ticks 240] [--size 1920x1080]
//!
//! The sim is games/spellstorm/sim (hecs, the Godot game's rules); the look
//! comes from the Godot game's own models and materials (assets/kits).
mod anim;
mod kits;
mod look;
mod view;

use render::{Gpu, Renderer};
use spellstorm::{Seat, Settings, World};

fn main() {
    if let Err(e) = run() {
        eprintln!("spellstorm: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let numf = |k: &str, d: f32| arg(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    match args.get(1).map(String::as_str) {
        Some("shot") => {
            let out = arg("--out").ok_or("--out FILE required")?;
            let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1920), b.parse().unwrap_or(1080)))).unwrap_or((1920u32, 1080u32));
            let seats: Vec<Seat> = (0..numf("--bots", 3.0) as usize).map(Seat::bot).collect();
            let mut world = World::new(&seats, numf("--seed", 7.0) as u64, Settings::default());
            let ticks = numf("--ticks", 240.0) as u32;
            render::set_game_surface(include_str!("surface.wgsl"));
            let mut view = view::View::new(kits::Kits::load(None)?);
            let gpu = Gpu::new()?;
            let started = std::time::Instant::now();
            view.build(&world)?;
            let renderer = Renderer::new(&gpu, &view.scene, &view.light);
            view.upload(&gpu, &renderer)?;
            eprintln!("scene: {} instances, {} meshes, {} materials, {} lamps in {:.2}s", view.scene.instances.len(), view.scene.meshes.len(), view.scene.materials.len(), view.lamps.len(), started.elapsed().as_secs_f32());
            for t in 0..ticks {
                world.step();
                if t % 4 == 0 || t + 1 == ticks {
                    view.pose(&gpu, &renderer, &world, 4.0 / 60.0)?;
                }
            }
            let mut rgba = vec![];
            for _ in 0..numf("--frames", 4.0) as u32 {
                rgba = renderer.render(&gpu, &view.camera, &view.light, w, h).0;
            }
            image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
        }
        _ => Err("usage: spellstorm shot --out frame.png [--seed 7] [--bots 3] [--ticks 240]".into()),
    }
}
