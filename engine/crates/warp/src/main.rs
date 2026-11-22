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
  warp new DIR [--title TITLE]
      write a starter game (warp new GAME.luau: the same game in one file)

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
        Some("audio") => audio::cli::run(args[1..].to_vec()).map_err(|e| match e.strip_prefix("usage: ") { Some(u) => format!("usage: warp audio {u}"), None => e }),
        Some("new") => new(&args),
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
        title: game.cfg.title.clone(), size: size(args, game.cfg.size)?, tick_hz: game.cfg.tick_rate,
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
    let files = warp::starter::write(&dir, &title)?;
    println!("wrote {} ({}): {}", dir.display(), title, files.join(", "));
    println!("play it:  warp run {}", dir.display());
    println!("check it: warp check {}  (no GPU)", dir.display());
    Ok(())
}
