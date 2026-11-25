#![cfg_attr(windows, windows_subsystem = "windows")]
//! warp: run a game made only of Luau scripts, synth patches and assets.
//!
//!   warp DIR                      (as `warp run DIR`)
//!   warp                          (a packaged game: runs the `game` folder next to the executable)
//!   warp run DIR [--players N] [--bots N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]
//!   warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]
//!   warp serve DIR [--live [PORT]] [--seconds T] [--size WxH] [--sky SKY] [--seed N]
//!   warp check DIR [--ticks N]
//!   warp audio render FILE.luau [options] | check FILES | units
//!   warp samples DIR [--bundle] [--credits]
//!   warp new DIR [--title TITLE]
//!
//! See engine/docs/warp-cli.md.
use std::path::{Path, PathBuf};
use warp::{LuaGame, Overrides};

const USAGE: &str = "warp: run a game made of Luau scripts, synth patches and assets (engine/docs/warp-cli.md)

  warp run DIR [--players N] [--bots N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]
      play fullscreen (--windowed opts out); --live serves MCP for an AI (engine/docs/live-control.md).
      A Splitscreen Club party host (GAMENIGHT=1 or GAMENIGHT_ADDR) decides the players.
  warp DIR
      the same as warp run DIR
  warp run GAME.luau, warp GAME.luau (and script, serve, check)
      a one-file game: settings in its leading `--! key = value` lines
  warp
      a packaged game: runs the game folder next to warp.exe (how Splitscreen Club starts it)
  warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]
      a headless scripted playtest (app::script format); exits 1 on a failed check
  warp serve DIR [--live [PORT]] [--seconds T] [--size WxH]
      headless in real time with live control (default port 7878)
  warp check DIR [--ticks N]
      load, init and tick without a GPU; print errors and the state
  warp run DIR --host [--lan] [--ticket-file F]  |  warp run DIR --join TICKET [--lan]
      online couches: the host prints a ticket, another couch joins with it (engine/docs/online.md)
  warp nettest DIR [--ticks N]
      an online host and a follower over loopback, without a GPU: checks the mirror, prints the bandwidth
  --storage-dir DIR (run, script, serve, check)
      use this local save directory; headless commands otherwise use fresh in-memory storage
  warp audio render FILE.luau [...] | check FILES | units
      render and measure sound patches (as warp-audio)
  warp compare F12.json [--baseline main] [--out DIR] [--state-only]
  warp inspect F12.json [--pixel X Y | --preview]
  warp relight F12.json --without CONTROL [--pixel X Y] [--reference X Y] [--out DIR]
      compare Sideways driving, inspect a captured pixel or isolate a lighting effect (developer checkout, Python and Pillow)
  warp samples DIR [--bundle] [--credits]
      fetch the recordings samples.lock lists into the cache (--bundle: into DIR/samples for a package;
      --credits: print who made them). `warp run` fetches missing ones in the background.
  warp samples DIR [--bundle] [--credits]
      fetch the recordings samples.lock lists into the cache (--bundle: into DIR/samples for a package;
      --credits: print who made them). `warp run` fetches missing ones in the background.
  warp new DIR [--title TITLE] [--template NAME]
      write a starter game (warp new GAME.luau: the same game in one file);
      --template platformer starts from a game on the kit (warp templates lists them)
  warp templates
      list the templates

SKY: gradient | sunrise | morning | noon | afternoon | sunset | night, optionally :clear | :haze | :mist";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Read only the small manifest before creating the loading taskbar item.
    // Packaged Luau games get their own name and optional icon, not "Warp".
    let path=if args.first().is_some_and(|a|a=="run") {args.get(1).map(PathBuf::from)}
        else if args.first().is_some_and(|a|is_game(Path::new(a))) {args.first().map(PathBuf::from)}
        else if args.is_empty() {bundled()}else{None};
    let metadata=path.as_ref().and_then(|p|if p.is_file(){warp::config::GameToml::load_file(p).ok()}else{warp::config::GameToml::load(p).ok()});
    let icon=path.as_ref().and_then(|p| {
        let dir=if p.is_file(){p.parent().unwrap_or(p)}else{p};
        ["icon.png","assets/icon.png","assets/branding/logo.png"].iter().find_map(|f|std::fs::read(dir.join(f)).ok())
    });
    app::startup::init(metadata.as_ref().map_or("Warp",|m|m.title.as_str()),icon.as_deref());
    if let Err(e) = run(args) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn value(args: &[String], key: &str) -> Option<String> { args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone()) }

fn parse<T: std::str::FromStr>(args: &[String], key: &str) -> Result<Option<T>, String> {
    value(args, key).map(|v| v.parse().map_err(|_| format!("{key} {v}: not a valid value"))).transpose()
}

fn size(args: &[String], default: [u32; 2]) -> Result<[u32; 2], String> {
    let Some(s) = value(args, "--size") else { return Ok(default) };
    let (w, h) = s.split_once('x').ok_or_else(|| format!("--size {s}: WIDTHxHEIGHT"))?;
    Ok([w.parse().map_err(|_| format!("--size {s}"))?, h.parse().map_err(|_| format!("--size {s}"))?])
}

fn overrides(args: &[String]) -> Result<Overrides, String> {
    Ok(Overrides { players: parse(args, "--players")?, bots: parse(args, "--bots")?, seed: parse(args, "--seed")?, sky: value(args, "--sky"), fresh: false, seats: None })
}

/// The game folder or one-file game: the first argument after the command.
fn dir(args: &[String]) -> Result<PathBuf, String> {
    args.get(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).ok_or_else(|| format!("{}: which game (a folder or a .luau file)?\n\n{USAGE}", args[0]))
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("compare" | "inspect" | "relight") => {
            let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/drive-capture.py");
            let python = std::env::var_os("WARP_PYTHON").unwrap_or_else(|| if cfg!(windows) { "python".into() } else { "python3".into() });
            let status = std::process::Command::new(python).arg(script).args(&args).status()
                .map_err(|e| format!("capture tools require Python and Pillow: {e}"))?;
            if status.success() { Ok(()) } else { Err(format!("capture tool exited with {status}")) }
        }
        Some("run") => play(&args),
        Some("script") => script(&args),
        Some("serve") => serve(&args),
        Some("check") => check(&args),
        Some("nettest") => nettest(&args),
        Some("audio") => audio::cli::run(args[1..].to_vec()).map_err(|e| match e.strip_prefix("usage: ") { Some(u) => format!("usage: warp audio {u}"), None => e }),
        Some("new") => new(&args),
        Some("templates") => { for t in warp::starter::TEMPLATES { println!("{:<12} {}", t.name, t.about); } Ok(()) }
        Some("samples") => samples(&args),
        None => match bundled() {
            Some(game) => play(&["run".to_string(), game.display().to_string()]),
            None => { println!("{USAGE}"); Ok(()) }
        },
        Some("help" | "--help" | "-h") => { println!("{USAGE}"); Ok(()) }
        // A game folder (or one-file game) on its own: a runtime launch (the
        // Splitscreen Club installer passes the folder; a file manager, the file).
        Some(d) if is_game(Path::new(d)) => play(&[vec!["run".to_string()], args].concat()),
        Some(other) => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

/// A game folder (with main.luau) or a one-file game (a `.luau` file).
fn is_game(p: &Path) -> bool { warp::config::is_game(p) }

/// A packaged game: `game/main.luau` next to the executable
/// (engine/crates/warp/tools/package-windows.ps1 makes such a folder).
fn bundled() -> Option<PathBuf> {
    let game = std::env::current_exe().ok()?.parent()?.join("game");
    is_game(&game).then_some(game)
}

fn load_game(dir: &Path, args: &[String], sound: Option<audio::synth::SoundSender>, interactive: bool) -> Result<LuaGame, String> {
    use warp::storage::Location;
    let path = value(args, "--storage-dir");
    if args.iter().any(|s| s == "--storage-dir") && path.as_ref().is_none_or(|s| s.is_empty() || s.starts_with("--")) {
        return Err("--storage-dir requires a directory".into());
    }
    let location = match &path {
        Some(dir) => Location::Directory(Path::new(dir)),
        None if interactive => Location::User,
        None => Location::Memory,
    };
    // Played for real, a game with `fresh_seed` gets a new seed each round.
    let over = Overrides { fresh: interactive, ..overrides(args)? };
    LuaGame::load_with_storage(dir, &over, sound, location)
}

fn samples(args: &[String]) -> Result<(), String> {
    let dir = dir(args)?;
    let game = if dir.is_file() { dir.parent().unwrap_or(Path::new(".")).to_path_buf() } else { dir };
    let lock = warp::samples::read(&game)?.ok_or_else(|| format!("{}: no {}", game.display(), warp::samples::LOCK))?;
    if args.iter().any(|a| a == "--credits") { print!("{}", warp::samples::credits(&lock)); return Ok(()); }
    let into = args.iter().any(|a| a == "--bundle").then(|| game.join("samples"));
    let n = warp::samples::fetch(&game, into.as_deref(), &mut |l| println!("{l}"))?;
    println!("samples: {} files listed, {n} fetched{}", lock.file.len(), into.map(|d| format!(" into {}", d.display())).unwrap_or_default());
    Ok(())
}

fn play(args: &[String]) -> Result<(), String> {
    println!("{} warp: starting", app::manners::stamp());
    let dir = dir(args)?;
    warp::samples::fetch_later(if dir.is_file() { dir.parent().unwrap_or(Path::new(".")) } else { &dir });
    // The speakers, with the synth mixer for the device's channel layout.
    let mut sender = None;
    let speaker = app::sound::open(|rate, channels| {
        let (tx, mix) = audio::synth::mixer(audio::synth::Layout::for_channels(channels), 128, rate);
        tx.room(1.2, 0.4, 0.5);
        sender = Some(tx);
        Box::new(mix)
    });
    if let Err(e) = &speaker { eprintln!("sound off: {e}"); }
    println!("{} warp: sound open", app::manners::stamp());
    let mut game = load_game(&dir, args, sender, true)?;
    let lan = args.iter().any(|a| a == "--lan");
    if args.iter().any(|a| a == "--host") {
        let ticket = game.host_online(lan)?;
        println!("{} warp: online host. Another couch joins with:\n  warp run {} --join {ticket}", app::manners::stamp(), dir.display());
        if let Some(file) = value(args, "--ticket-file") { std::fs::write(&file, &ticket).map_err(|e| format!("{file}: {e}"))?; }
    }
    if let Some(ticket) = value(args, "--join") {
        let ticket = if std::path::Path::new(&ticket).is_file() { std::fs::read_to_string(&ticket).map_err(|e| format!("{ticket}: {e}"))?.trim().to_string() } else { ticket };
        println!("{} warp: joining the host", app::manners::stamp());
        game.follow_online(&ticket, lan, size(args, game.cfg.size)?)?;
        println!("{} warp: following the host", app::manners::stamp());
    }
    if game.connect_party() { println!("{} warp: connected to the party host", app::manners::stamp()); }
    if let Some(cfg) = live::Config::from_args(&game.live_name(), args)? { game.start_live(cfg)?; }
    else if game.party.is_some() && std::env::var("WARP_LIVE").as_deref() != Ok("0") {
        // Under Splitscreen Club the session assistant changes the game through
        // live control (engine/docs/live-control.md): any free local port.
        let mut cfg = live::Config::new(&game.live_name());
        cfg.port = 0;
        cfg.announce = false;
        if let Err(e) = game.start_live(cfg) { eprintln!("live: {e}"); }
    }
    let cfg = app::Config {
        title: game.cfg.title.clone(), size: size(args, game.cfg.size)?,
        // A follower shows the host's frames as they come (online::SEND_HZ).
        tick_hz: if game.is_following() { game.cfg.tick_rate / warp::online::send_every(game.cfg.tick_rate) as f32 } else { game.cfg.tick_rate },
        fullscreen: app::startup::fullscreen(args), keyboard: true, pointer: false, frames: parse(args, "--frames")?.unwrap_or(0), max_render_height: None, target_fps: None,
    };
    let r = app::run(cfg, &mut game);
    drop(speaker);
    r
}

fn script(args: &[String]) -> Result<(), String> {
    let dir = dir(args)?;
    let file = args.get(2).filter(|a| !a.starts_with("--")).ok_or("warp script DIR FILE.json --out OUT")?;
    let out = value(args, "--out").ok_or("warp script: --out OUT (a folder for frames and state dumps)")?;
    let (tx, mix) = warp::sounds::offline();
    let mut game = load_game(&dir, args, Some(tx), false)?;
    let gpu = render::Gpu::new()?;
    let hz = game.cfg.tick_rate;
    let failed = app::script::run_file(&mut game, &gpu, hz, Path::new(file), Path::new(&out), Some(Box::new(mix)))?;
    if failed > 0 { return Err(format!("{failed} check(s) failed")); }
    Ok(())
}

fn serve(args: &[String]) -> Result<(), String> {
    let dir = dir(args)?;
    // Sound goes into a mixer that is never rendered: plays and handles work, silently.
    let (tx, mix) = warp::sounds::offline();
    drop(mix);
    let mut game = load_game(&dir, args, Some(tx), false)?;
    let mut with_live: Vec<String> = args.to_vec();
    if !with_live.iter().any(|a| a == "--live") { with_live.push("--live".into()); }
    let cfg = live::Config::from_args(&game.live_name(), &with_live)?.expect("--live is set");
    game.start_live(cfg)?;
    let gpu = render::Gpu::new()?;
    let app_cfg = app::Config { title: game.cfg.title.clone(), size: size(args, [960, 540])?, tick_hz: game.cfg.tick_rate, ..Default::default() };
    app::run_headless(&app_cfg, &mut game, &gpu, parse(args, "--seconds")?.unwrap_or(0.0))
}

fn check(args: &[String]) -> Result<(), String> {
    let dir = dir(args)?;
    let mut game = load_game(&dir, args, None, false)?;
    let ticks: u32 = parse(args, "--ticks")?.unwrap_or(120);
    let players = app::Game::local_players(&game);
    for _ in 0..ticks { app::Game::tick(&mut game, &vec![app::Controls::default(); players]); }
    println!("{}", serde_json::to_string_pretty(&game.state_json()).unwrap());
    let errors = game.errors();
    if errors.is_empty() { Ok(()) } else { Err(format!("{} error(s):\n{}", errors.len(), errors.join("\n"))) }
}

/// An online host and a follower couch in one process over loopback: the
/// follower's world must match the host's, and the line says the bandwidth.
fn nettest(args: &[String]) -> Result<(), String> {
    let dir = dir(args)?;
    let ticks: u32 = parse(args, "--ticks")?.unwrap_or(600);
    let mut host = load_game(&dir, args, None, false)?;
    host.echo = false;
    let mut follower = load_game(&dir, args, None, false)?;
    follower.echo = false;
    let ticket = host.host_online(true)?;
    follower.follow_online(&ticket, true, [1920, 1080])?;
    let hz = host.cfg.tick_rate;
    let hp = app::Game::local_players(&host);
    let fp = app::Game::local_players(&follower);
    // The follower's people fly right and fire, so their input must reach the host.
    let push = app::Controls { mv: [1.0, 0.0], aim: [1.0, 0.0], held: app::Buttons(app::Buttons::RIGHT_TRIGGER), ..Default::default() };
    let t0 = std::time::Instant::now();
    for _ in 0..ticks {
        app::Game::tick(&mut host, &vec![app::Controls::default(); hp]);
        std::thread::sleep(std::time::Duration::from_millis(2));
        app::Game::tick(&mut follower, &vec![push; fp]);
    }
    // Let the last frames arrive.
    for _ in 0..50 { std::thread::sleep(std::time::Duration::from_millis(10)); app::Game::tick(&mut follower, &vec![push; fp]); }
    let (wrong, extra) = follower.mirror_errors(&host);
    let bytes = host.online_sent();
    let kbit = bytes as f64 * 8.0 / 1000.0 / (ticks as f64 / hz as f64);
    let state = host.state_json();
    let remote_people: Vec<&serde_json::Value> = state["players"].as_array().map(|a| a.iter().filter(|p| p["name"].as_str().is_some_and(|n| n.starts_with("Online"))).collect()).unwrap_or_default();
    // Where the game itself put the follower's people (any table with their name), to see their input arrive.
    fn find(v: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
        match v {
            serde_json::Value::Object(m) => {
                if m.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.starts_with("Online")) && m.contains_key("x") { out.push(serde_json::json!({ "name": m["name"], "x": m["x"], "y": m["y"] })); }
                for x in m.values() { find(x, out); }
            }
            serde_json::Value::Array(a) => for x in a { find(x, out); },
            _ => {}
        }
    }
    let mut in_game = Vec::new();
    find(&state["state"], &mut in_game);
    println!("{}", serde_json::json!({ "follower_people_in_game": in_game,
        "ticks": ticks, "seconds_real": t0.elapsed().as_secs_f32(), "entities_host": host.core().world.entities.len(), "entities_follower": follower.core().world.entities.len(),
        "mismatched": wrong, "extra_on_follower": extra, "bytes": bytes, "kbit_per_s_at_tick_rate": kbit.round(), "bytes_per_tick": bytes / ticks as u64, "bytes_full_poses_hud_rest": host.online_parts(),
        "follower": follower.online_state(), "host_online_people": remote_people.len(), "host_errors": host.errors(), "follower_errors": follower.errors(),
    }));
    if wrong > 0 || extra > 0 { return Err(format!("the follower's world differs: {wrong} wrong, {extra} extra")); }
    if remote_people.is_empty() { return Err("the follower's people never reached the host".into()); }
    Ok(())
}

fn new(args: &[String]) -> Result<(), String> {
    let dir = dir(args)?;
    let title = value(args, "--title").unwrap_or_else(|| {
        let name = dir.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Hello Warp".into());
        name.split(['-', '_', ' ']).filter(|w| !w.is_empty()).map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }).collect::<Vec<_>>().join(" ")
    });
    if dir.extension().is_some_and(|e| e == "luau") {
        warp::starter::write_file(&dir, &title)?;
        println!("wrote {} ({title}), a one-file game", dir.display());
        println!("play it:  warp run {}", dir.display());
        println!("check it: warp check {}  (no GPU)", dir.display());
        return Ok(());
    }
    let files = match value(args, "--template") {
        Some(name) => warp::starter::write_template(&dir, &title, warp::starter::template(&name)?)?,
        None => warp::starter::write(&dir, &title)?,
    };
    println!("wrote {} ({}): {}", dir.display(), title, files.join(", "));
    println!("play it:  warp run {}", dir.display());
    println!("check it: warp check {}  (no GPU)", dir.display());
    Ok(())
}
