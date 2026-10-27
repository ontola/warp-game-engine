//! Growing Guns on Warp.
//!
//!   growing-guns play [--players N] [--total N] [--seed S] [--fullscreen] [--no-kbm]
//!   growing-guns shot --out FILE.png [--views N] [--seed S] [--seconds T] [--size WxH]
//!   growing-guns sim [--seed S] [--seconds T] [--trace players.0.health,round [--rows N]]
//!   growing-guns nettest [--seconds T]   (host + client over iroh loopback, headless)
//!   growing-guns script FILE.json --out DIR [--players N] [--seed S]   (app::script; exit 1 on a failed expect)
//!
//! Online: `play --host` prints a ticket; `play --join TICKET` joins it.
//! `--lan` skips relays and address lookup (direct addresses only).
mod bench;
mod hud;
mod net;
mod play;
mod session;
mod sounds;
mod synth;
mod crowd_sound;
mod stadium;
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
        "script" => script(&args),
        "nettest" => nettest(&args),
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
    if let Some(paths) = args.value("--trace") {
        // Replay from the same seed, sampling the paths (kern::Trace).
        let paths: Vec<String> = paths.split(',').map(str::to_string).collect();
        let mut sim = World::new(seed, &[true; 4]);
        let mut trace = kern::Trace::new(&paths);
        let mut timings = kern::Timings::default();
        for _ in 0..seconds * TICK_HZ / 6 { for _ in 0..6 { kern::Sim::step(&mut sim, &mut timings); } trace.sample(&sim); }
        print!("{}", trace.table(args.num("--rows", 24usize)));
    }
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

/// Scripted headless playtest (see engine/crates/app/src/script.rs): local
/// players 0..N follow the script, the rest are bots.
fn script(args: &Args) -> Result<(), String> {
    let file = args.list.get(1).filter(|f| !f.starts_with("--")).ok_or("script FILE.json --out DIR")?;
    let out = args.value("--out").unwrap_or("script-out");
    let humans = args.num("--players", 1usize).clamp(1, 4);
    let bots: Vec<bool> = (0..4).map(|i| i >= humans).collect();
    let world = World::new(args.num("--seed", 3u64), &bots);
    let mut game = play::Gg::new(world, session::Session::Local, (0..humans).collect());
    game.light_mut().taa = 0.0;
    for kv in args.list.windows(2).filter(|w| w[0] == "--set").map(|w| w[1].clone()) { game.light_mut().set(&kv)?; }
    let mixer = sounds::find_dir(args.value("--sounds")).map(|dir| {
        let (tx, mix) = audio::sfx::sfx(64, audio::SAMPLE_RATE);
        game.set_crowd(args.value("--crowd"), &dir, &tx);
        game.set_sounds(sounds::Sounds::load(&dir, tx));
        mix
    });
    let gpu = Gpu::new()?;
    let failed = app::script::run_file(&mut game, &gpu, TICK_HZ as f32, std::path::Path::new(file), std::path::Path::new(out), mixer)?;
    if failed > 0 { std::process::exit(1); }
    Ok(())
}

/// A host and a client in one process over iroh on loopback (no relays),
/// driven through the same Game::tick as play: the client walks and fires;
/// checks that the host treats it as a human, that it moves, and that the
/// client's world follows the host's ticks.
fn nettest(args: &Args) -> Result<(), String> {
    use app::{Buttons, Controls, Game};
    let seconds = args.num("--seconds", 4.0f32);
    let ticks = (seconds * TICK_HZ as f32) as u32;
    let (ticket_tx, ticket_rx) = std::sync::mpsc::channel();
    // The host runs in real time on its own thread, as it would in play.
    let host = std::thread::spawn(move || -> Result<String, String> {
        let mut host = play::Gg::new(World::new(9, &[false, true, true, true]), session::Session::host(true)?, vec![0]);
        ticket_tx.send(host.session().ticket().unwrap().to_string()).map_err(|e| e.to_string())?;
        let start = std::time::Instant::now();
        for k in 0..ticks + 120 {
            // Destruction mid-game: clients must end with the same arena.
            if k == 90 || k == 150 { host.command(&format!("blast {} 0 {} 5 400", k as f32 * 0.1 - 10.0, 6.0))?; }
            host.tick(&[Controls::default()]);
            let due = start + std::time::Duration::from_secs_f32((k + 1) as f32 / TICK_HZ as f32);
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) { std::thread::sleep(wait); }
        }
        host.state().ok_or("no state".into())
    });
    let ticket = ticket_rx.recv_timeout(std::time::Duration::from_secs(20)).map_err(|e| format!("host did not start: {e}"))?;
    let t = std::time::Instant::now();
    let (session, world, slots) = session::Session::join(&ticket, 1, true)?;
    println!("joined in {:.0} ms as player {}", t.elapsed().as_secs_f64() * 1000.0, slots[0] + 1);
    let mut client = play::Gg::new(world, session, slots.clone());
    let me = slots[0];
    let mut start_pos = None;
    let mut last_tick = 0;
    for k in 0..ticks {
        let c = Controls { mv: [0.0, 1.0], held: Buttons(if k % 30 < 15 { Buttons::RIGHT_TRIGGER } else { 0 }), ..Default::default() };
        client.tick(&[c]);
        client.receive(1.0 / TICK_HZ as f32);
        if start_pos.is_none() && client.world.tick > 0 { start_pos = Some(client.world.players[me].pos); }
        last_tick = client.world.tick;
        std::thread::sleep(std::time::Duration::from_secs_f32(1.0 / TICK_HZ as f32));
    }
    // Drain what the host still sends while it finishes.
    let drain = std::time::Instant::now();
    while drain.elapsed().as_secs_f32() < 1.0 { client.receive(1.0 / TICK_HZ as f32); std::thread::sleep(std::time::Duration::from_millis(16)); }
    let host_state: serde_json::Value = serde_json::from_str(&host.join().map_err(|_| "host thread panicked")??).map_err(|e| e.to_string())?;
    let host_broken = host_state["arena"]["chunks_broken"].as_u64().unwrap_or(0);
    let client_broken = (client.world.arena.chunks.len() - client.world.arena.alive_chunks()) as u64;
    println!("destruction: host broke {host_broken} chunks, client sees {client_broken}");
    if host_broken == 0 || host_broken != client_broken { return Err("nettest failed: arenas differ".into()); }
    let moved = start_pos.map_or(0.0, |p| p.distance(client.world.players[me].pos));
    let human = host_state["players"][me]["bot"] == serde_json::Value::Bool(false);
    println!("client followed to host tick {last_tick}; player {} moved {moved:.1} m; host sees it as human: {human}; ammo {}",
        me + 1, client.world.players[me].ammo);
    if !(human && moved > 2.0 && last_tick > (ticks / 2) as u64) { return Err("nettest failed".into()); }
    println!("nettest ok");
    Ok(())
}
