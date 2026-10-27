//! Growing Guns on Warp.
//!
//!   growing-guns play [--players N] [--total N] [--seed S] [--fullscreen] [--no-kbm]
//!   growing-guns shot --out FILE.png [--views N] [--seed S] [--seconds T] [--size WxH]
//!   growing-guns sim [--seed S] [--seconds T]
//!
//! Online: `play --host` prints a ticket; `play --join TICKET` joins it.
//! `--lan` skips relays and address lookup (direct addresses only).
mod bench;
mod hud;
mod net;
mod play;
mod session;
mod view;

use growing_guns::{Event, World, TICK_HZ};
use render::Gpu;

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
        "bench" => bench::run(&args),
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

/// Headless capture through the same frame path as `play` (app::capture):
/// bots play for a while, then every view is rendered with its HUD.
fn shot(args: &Args) -> Result<(), String> {
    let out = args.value("--out").ok_or("--out FILE.png required")?;
    let views = args.num("--views", 4usize).clamp(1, 4);
    let seed = args.num("--seed", 3u64);
    let seconds = args.num("--seconds", 4.0f32);
    let (w, h) = args.size((1600, 900));
    let world = World::new(seed, &[true; 4]);
    let mut game = play::Gg::new(world, session::Session::Local, (0..views).collect());
    game.light_mut().taa = 0.0;
    for kv in args.list.windows(2).filter(|w| w[0] == "--set").map(|w| w[1].clone()) { game.light_mut().set(&kv)?; }
    let gpu = Gpu::new()?;
    let rgba = app::capture(&mut game, &gpu, [w, h], (seconds * TICK_HZ as f32) as u32, TICK_HZ as f32)?;
    render::write_png(std::path::Path::new(out), w, h, &rgba)?;
    println!("{out}: {views} views, round {}, tick {}, arena {} m {:?}, {} blocks, GPU {}",
        game.world.round, game.world.tick, game.world.arena.size, game.world.arena.variant, game.world.arena.blocks.len(), gpu.info.name);
    Ok(())
}
