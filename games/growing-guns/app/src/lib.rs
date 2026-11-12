//! Growing Guns on Warp.
//!
//!   growing-guns play [--players N] [--total N] [--seed S] [--fullscreen] [--no-kbm] [--no-music] [--music DIR] [--live [PORT]]
//!   (play, shot, script: [--sky godot|TIME[:WEATHER]|rounds|cycle], see sky.rs)
//!   growing-guns replay latest|FILE.ggreplay [--frame N] [--alpha A] [--out FILE.png] [--ui]
//!   growing-guns shot --out FILE.png [--views N] [--seed S] [--seconds T] [--size WxH]
//!   growing-guns sim [--seed S] [--seconds T] [--trace players.0.health,round [--rows N]]
//!   growing-guns nettest [--seconds T]   (host + client over iroh loopback, headless)
//!   growing-guns mods-api   (writes mods/API.md and mods/types.d.luau)
//!   growing-guns serve --live [PORT] [--seconds T] [--total N]   (headless, real time, live control; live.rs)
//!   growing-guns script FILE.json --out DIR [--players N] [--seed S]   (app::script; exit 1 on a failed expect)
//!
//! Online: `play --host` prints a ticket; `play --join TICKET` joins it.
//! `--lan` skips relays and address lookup (direct addresses only).
mod graphics;
mod xr;
mod bench;
mod replay;
mod hud;
mod live;
mod net;
mod play;
mod session;
mod party;
mod sounds;
mod crowd_sound;
mod gun;
mod gun_review;
mod human_review;
#[cfg(not(any(target_os="android",target_arch="wasm32")))]
mod human_play;
mod hand_animation;
mod animation_window;
mod knobs;
mod explosion_editor;
mod scripts;
mod sky;
mod blood;
mod stadium;
mod spectators;
mod music;
mod view;

use growing_guns::{Event, World, TICK_HZ};
use render::Gpu;

fn warp_humans_enabled() -> bool {
    !std::env::args_os().any(|arg| arg == "--no-warp-humans")
}

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

/// A macOS app (`tools/package-mac.sh`) keeps the game's files in
/// Contents/Resources and starts in a read-only `/`: point the game at its
/// files, and screenshots, replays and the pipeline cache at Application
/// Support. Variables already set win.
fn use_app_bundle() {
    let Some(macos) = std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::PathBuf::from)) else { return };
    let resources = macos.join("../Resources");
    if !macos.ends_with("Contents/MacOS") || !resources.join("mods").is_dir() { return; }
    let Some(home) = std::env::var_os("HOME") else { return };
    let data = std::path::PathBuf::from(home).join("Library/Application Support/Growing Guns");
    let _ = std::fs::create_dir_all(&data);
    let mut vars = vec![("GG_SOUNDS", resources.join("audio")), ("GG_MUSIC", resources.join("music")), ("GG_SFX", resources.join("sounds")),
        ("GG_MODS", resources.join("mods")), ("GG_CHARACTERS", resources.join("assets/characters")),
        ("WARP_AUDIO_IR_DIR", resources.join("engine/assets/sound/ir")), ("WARP_SCREENSHOTS", data.join("screenshots")),
        ("KERN_PIPELINE_CACHE_DIR", data.join("pipeline-cache"))];
    if resources.join("audio/crowd").is_dir() { vars.push(("GG_CROWD", resources.join("audio/crowd"))); }
    // Done before game/audio/renderer worker threads start.
    for (name, value) in vars {
        if std::env::var_os(name).is_none() { unsafe { std::env::set_var(name, value) } }
    }
    let _ = std::env::set_current_dir(&data);
}

/// The first word, or `play` with none or only flags (`open -a "Growing
/// Guns" --args --graphics low` has no command in front of its flags).
fn command(list: &[String]) -> String {
    list.first().filter(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| "play".into())
}

#[cfg(test)]
#[test]
fn flags_alone_mean_play() {
    let list = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(command(&list(&[])), "play");
    assert_eq!(command(&list(&["--emulate", "m1pro"])), "play");
    assert_eq!(command(&list(&["bench", "--views", "1"])), "bench");
}

pub fn desktop_main() {
    use_app_bundle();
    let list: Vec<String> = std::env::args().skip(1).collect();
    let cmd = command(&list);
    let args = Args { list };
    if let Err(e) = stadium::configure_density(&args) { eprintln!("{e}"); std::process::exit(1); }
    if let Some(profile) = args.value("--graphics").map(str::to_owned).or_else(|| std::env::var("GG_GRAPHICS").ok()) {
        if let Err(e) = graphics::Graphics::parse(&profile).and_then(graphics::configure) { eprintln!("{e}"); std::process::exit(1); }
    }
    let result = match cmd.as_str() {
        "play" => play::run(&args),
        "shot" => shot(&args),
        "animation-preview" => animation_window::run(&args),
        #[cfg(not(any(target_os="android",target_arch="wasm32")))]
        "human-play" => human_play::run(&args),
        "human-review" => human_review::run(&args),
        "gun-review" => gun_review::run(&args),
        "replay" => replay::run(&args),
        "xr-shot" => xr::shot(&args),
        "sim" => { sim(&args); Ok(()) }
        "bench" => bench::run(&args),
        "script" => script(&args),
        "nettest" => nettest(&args),
        "lantest" => lantest(),
        "serve" => live::serve(&args),
        "mods-api" => mods_api(),
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
    if let Err(e) = scripts::attach(&mut w) { eprintln!("mods: {e}"); }
    w.force_island = args.flag("--island");
    let (mut shots, mut hits, mut rounds) = (0u32, 0u32, 0u32);
    // The arena of each round and the tick it began.
    let mut arenas = vec![(0u64, format!("{:?}", w.arena.variant))];
    let t = std::time::Instant::now();
    for _ in 0..seconds * TICK_HZ {
        w.step(&[]);
        for e in &w.events {
            match e { Event::Shot { .. } => shots += 1, Event::Hit { .. } => hits += 1, Event::RoundOver { .. } => rounds += 1, Event::NewArena => arenas.push((w.tick, format!("{:?}", w.arena.variant))), _ => {} }
        }
    }
    if args.flag("--arenas") {
        println!("arenas: {}", arenas.iter().map(|(t, v)| format!("{v}@{t}")).collect::<Vec<_>>().join(" "));
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / (seconds * TICK_HZ) as f64;
    if let Some(paths) = args.value("--trace") {
        // Replay from the same seed, sampling the paths (kern::Trace).
        let paths: Vec<String> = paths.split(',').map(str::to_string).collect();
        let mut sim = World::new(seed, &[true; 4]);
        if let Err(e) = scripts::attach(&mut sim) { eprintln!("mods: {e}"); }
        let mut trace = kern::Trace::new(&paths);
        let mut timings = kern::Timings::default();
        for _ in 0..seconds * TICK_HZ / 6 { for _ in 0..6 { kern::Sim::step(&mut sim, &mut timings); } trace.sample(&sim); }
        print!("{}", trace.table(args.num("--rows", 24usize)));
    }
    println!("seed {seed}: {seconds} s, {rounds} rounds, {shots} shots, {hits} hits, scores {:?}, {ms:.4} ms per tick, digest {:016x}",
        w.players.iter().map(|p| p.score).collect::<Vec<_>>(), w.digest());
}

/// Regenerate the mod API reference from the Rust side.
fn mods_api() -> Result<(), String> {
    let dir = scripts::find_dir().ok_or("no mods folder (GG_MODS or games/growing-guns/mods)")?;
    for (file, text) in [("API.md", scripts::api_doc()), ("types.d.luau", scripts::api_types())] {
        std::fs::write(dir.join(file), text).map_err(|e| e.to_string())?;
        println!("{}", dir.join(file).display());
    }
    Ok(())
}

/// Headless capture through the same frame path as `play` (app::capture):
/// bots play for a while, then every view is rendered with its HUD.
fn shot(args: &Args) -> Result<(), String> {
    let out = args.value("--out").ok_or("--out FILE.png required")?;
    let views = args.num("--views", 4usize).clamp(1, 4);
    let seed = args.num("--seed", 3u64);
    let seconds = args.num("--seconds", 4.0f32);
    let (w, h) = args.size((1600, 900));
    let world = World::new_with(seed, &[true; 4], |w| play::match_options(args, w, false));
    let mut world = world;
    if args.flag("--start-in-stands") { stadium::start_in_stands(&mut world)?; }
    let mut game = play::Gg::new(world, session::Session::Local, (0..views).collect());
    game.world.release_cage();
    game.set_sky(sky::Sky::from_args(args.value("--sky"))?)?;
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
    let world = World::new_with(args.num("--seed", 3u64), &bots, |w| play::match_options(args, w, false));
    let mut world = world;
    if args.flag("--start-in-stands") { stadium::start_in_stands(&mut world)?; }
    let mut game = play::Gg::new(world, session::Session::Local, (0..humans).collect());
    // Headless scripts skip the entrance unless requested.
    if !args.flag("--round-entrance") && !args.flag("--cage") { game.world.release_cage(); }
    game.set_sky(sky::Sky::from_args(args.value("--sky"))?)?;
    game.light_mut().taa = 0.0;
    for kv in args.list.windows(2).filter(|w| w[0] == "--set").map(|w| w[1].clone()) { game.light_mut().set(&kv)?; }
    let samples = sounds::find_dir(args.value("--sounds"));
    let patches = sounds::find_patches(args.value("--sfx"));
    let music = play::music(args, samples.as_deref(), args.num("--seed", 3u64), false);
    let (mixer, tx, crowd) = sounds::offline(music.as_ref());
    game.set_crowd(args.value("--crowd"), samples.as_deref(), &crowd);
    game.set_sounds(sounds::Sounds::new(tx, patches, samples.as_deref()));
    if let Some(m) = music { game.set_music(m); }
    let gpu = Gpu::new()?;
    let failed = app::script::run_file(&mut game, &gpu, TICK_HZ as f32, std::path::Path::new(file), std::path::Path::new(out), Some(mixer))?;
    if failed > 0 { std::process::exit(1); }
    Ok(())
}

/// A host and a client in one process over iroh on loopback (no relays),
/// driven through the same Game::tick as play: the client walks and fires;
/// checks that the host treats it as a human, that it moves, and that the
/// client's world follows the host's ticks.
/// Two games in one process find each other as two headsets would: HOST LAN
/// GAME on one, JOIN LAN GAME on the other, no ticket typed. The client plays
/// as a headset, and its view and gun must reach the host's simulation.
fn lantest() -> Result<(), String> {
    use app::{Controls, Game};
    let aim = growing_guns::TrackedAim { offset: glam::Vec3::new(0.2, 0.3, -0.4), direction: glam::Vec3::NEG_X };
    let host = std::thread::spawn(move || -> Result<String, String> {
        let mut host = play::Gg::new(World::new(9, &[false, true, true, true]), session::Session::Local, vec![0]);
        host.command("lan host")?;
        if !matches!(host.session(), session::Session::Host { .. }) { return Err(format!("not hosting: {:?}", host.ui.notice)); }
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 25 {
            host.tick(&[Controls::default()]);
            if let Some((i, p)) = host.world.players.iter().enumerate().find(|(_, p)| p.tracked.is_some()) {
                if p.tracked.map(|t| t.direction) != Some(aim.direction) || (p.yaw - 1.0).abs() > 1e-5 { return Err(format!("player {i}: yaw {} tracked {:?}", p.yaw, p.tracked)); }
                return Ok(format!("host sees player {} with yaw {} and gun {:?}", i + 1, p.yaw, p.tracked.unwrap().direction));
            }
            std::thread::sleep(std::time::Duration::from_secs_f32(1.0 / TICK_HZ as f32));
        }
        Err("the host never saw the client's gun".into())
    });
    let t = std::time::Instant::now();
    let mut client = play::Gg::new(World::new(1, &[false, true]), session::Session::Local, vec![0]);
    client.lan_hosts = Some(::net::lan::Listener::start(&String::from_utf8_lossy(net::ALPN))?);
    client.xr_stereo = true;
    while !client.session().is_client() {
        if t.elapsed().as_secs() > 20 { return Err(format!("lantest failed: no join ({:?})", client.ui.notice)); }
        client.command("lan join")?;
        client.tick(&[Controls::default()]);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("found and joined in {:.0} ms as player {}", t.elapsed().as_secs_f64() * 1000.0, client.slots[0] + 1);
    client.xr_head = Some([1.0, 0.2]);
    client.xr_aim = Some(aim);
    while !host.is_finished() {
        client.tick(&[Controls::default()]);
        std::thread::sleep(std::time::Duration::from_secs_f32(1.0 / TICK_HZ as f32));
    }
    println!("{}", host.join().map_err(|_| "host panicked")??);
    println!("lantest ok");
    Ok(())
}

fn nettest(args: &Args) -> Result<(), String> {
    use app::{Buttons, Controls, Game};
    let seconds = args.num("--seconds", 4.0f32);
    let ticks = (seconds * TICK_HZ as f32) as u32;
    // COPY must not touch this machine's clipboard.
    app::clipboard::memory_only();
    let (ticket_tx, ticket_rx) = std::sync::mpsc::channel();
    // The host runs in real time on its own thread, as it would in play.
    let host = std::thread::spawn(move || -> Result<String, String> {
        let mut host = play::Gg::new(World::new(9, &[false, true, true, true]), session::Session::host(true)?, vec![0]);
        let copied = host.command("copy").map(|_| app::clipboard::get())?;
        ticket_tx.send((host.session().ticket().unwrap().to_string(), copied)).map_err(|e| e.to_string())?;
        let start = std::time::Instant::now();
        for k in 0..ticks + 120 {
            // Destruction mid-game: clients must end with the same arena.
            // Changes between ticks (debug console, live tools) reach clients.
            if k == 30 { host.command("round")?; host.command("give 2 explosive")?; }
            if k == 90 || k == 150 { host.command(&format!("blast {} 0 {} 5 400", k as f32 * 0.1 - 10.0, 6.0))?; }
            host.tick(&[Controls::default()]);
            let due = start + std::time::Duration::from_secs_f32((k + 1) as f32 / TICK_HZ as f32);
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) { std::thread::sleep(wait); }
        }
        host.state().ok_or("no state".into())
    });
    let (ticket, copied) = ticket_rx.recv_timeout(std::time::Duration::from_secs(20)).map_err(|e| format!("host did not start: {e}"))?;
    if copied != ticket { return Err(format!("nettest failed: COPY put {copied:?} on the clipboard")); }
    // The client joins as a player would: the ticket pasted into the pause
    // menu's join field (a valid ID connects at once).
    let t = std::time::Instant::now();
    let mut client = play::Gg::new(World::new(1, &[false, true]), session::Session::Local, vec![0]);
    client.lan = true;
    client.command("menu pause")?;
    client.command("focus join")?;
    client.text(app::TextEvent::Paste(format!("my game:\n{ticket}\n")));
    while !client.session().is_client() {
        if t.elapsed().as_secs() > 25 { return Err(format!("nettest failed: the menu join did not connect ({:?})", client.ui.notice)); }
        client.tick(&[Controls::default()]);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let slots = client.slots.clone();
    println!("joined from the menu in {:.0} ms as player {}", t.elapsed().as_secs_f64() * 1000.0, slots[0] + 1);
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
    // until the host has stopped (it plays on for two seconds after us).
    while !host.is_finished() || drain.elapsed().as_secs_f32() < 1.0 { client.receive(1.0 / TICK_HZ as f32); std::thread::sleep(std::time::Duration::from_millis(16)); }
    for _ in 0..30 { client.receive(1.0 / TICK_HZ as f32); std::thread::sleep(std::time::Duration::from_millis(10)); }
    let host_state: serde_json::Value = serde_json::from_str(&host.join().map_err(|_| "host thread panicked")??).map_err(|e| e.to_string())?;
    let host_broken = host_state["arena"]["chunks_broken"].as_u64().unwrap_or(0);
    let client_broken = (client.world.arena.chunks.len() - client.world.arena.alive_chunks()) as u64;
    println!("destruction: host broke {host_broken} chunks, client sees {client_broken}");
    if host_broken == 0 || host_broken != client_broken { return Err("nettest failed: arenas differ".into()); }
    let seeds = (host_state["arena"]["seed"].as_u64().unwrap_or(0), client.world.arena.seed);
    let card = client.world.players[2].cards.iter().any(|&c| client.world.cards.get(c).is_some_and(|d| d.id == "explosive"));
    println!("host-side tweaks: arena seed host {} client {}, player 3 has the given card on the client: {card}", seeds.0, seeds.1);
    if seeds.0 != seeds.1 || !card { return Err("nettest failed: a host tweak did not reach the client".into()); }
    let moved = start_pos.map_or(0.0, |p| p.distance(client.world.players[me].pos));
    let human = host_state["players"][me]["bot"] == serde_json::Value::Bool(false);
    println!("client followed to host tick {last_tick}; player {} moved {moved:.1} m; host sees it as human: {human}; ammo {}",
        me + 1, client.world.players[me].ammo);
    if !(human && moved > 2.0 && last_tick > (ticks / 2) as u64) { return Err("nettest failed".into()); }
    println!("nettest ok");
    Ok(())
}
