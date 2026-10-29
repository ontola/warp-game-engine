//! warp: run a game made only of Luau scripts, synth patches and assets.
//!
//!   warp DIR                      (as `warp run DIR`)
//!   warp                          (a packaged game: runs the `game` folder next to the executable)
//!   warp run DIR [--players N] [--bots N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]
//!   warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]
//!   warp serve DIR [--live [PORT]] [--seconds T] [--size WxH] [--sky SKY] [--seed N]
//!   warp check DIR [--ticks N]
//!   warp audio render FILE.luau [options] | check FILES | units
//!   warp new DIR [--title TITLE]
//!
//! See engine/docs/warp-cli.md.
use std::path::{Path, PathBuf};
use warp::{LuaGame, Overrides};

const USAGE: &str = "warp: run a game made of Luau scripts, synth patches and assets (engine/docs/warp-cli.md)

  warp run DIR [--players N] [--bots N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]
      play in a window; --live serves MCP for an AI (engine/docs/live-control.md).
      A GameNight party host (GAMENIGHT=1 or GAMENIGHT_ADDR) decides the players.
  warp DIR
      the same as warp run DIR
  warp run GAME.luau, warp GAME.luau (and script, serve, check)
      a one-file game: settings in its leading `--! key = value` lines
  warp
      a packaged game: runs the game folder next to warp.exe (how GameNight starts it)
  warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]
      a headless scripted playtest (app::script format); exits 1 on a failed check
  warp serve DIR [--live [PORT]] [--seconds T] [--size WxH]
      headless in real time with live control (default port 7878)
  warp check DIR [--ticks N]
      load, init and tick without a GPU; print errors and the state
  warp audio render FILE.luau [...] | check FILES | units
      render and measure sound patches (as warp-audio)
  warp new DIR [--title TITLE]
      write a starter game (warp new GAME.luau: the same game in one file)

SKY: gradient | sunrise | morning | noon | afternoon | sunset | night, optionally :clear | :haze | :mist";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    Ok(Overrides { players: parse(args, "--players")?, bots: parse(args, "--bots")?, seed: parse(args, "--seed")?, sky: value(args, "--sky") })
}

/// The game folder or one-file game: the first argument after the command.
fn dir(args: &[String]) -> Result<PathBuf, String> {
    args.get(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).ok_or_else(|| format!("{}: which game (a folder or a .luau file)?\n\n{USAGE}", args[0]))
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("run") => play(&args),
        Some("script") => script(&args),
        Some("serve") => serve(&args),
        Some("check") => check(&args),
        Some("audio") => audio::cli::run(args[1..].to_vec()).map_err(|e| match e.strip_prefix("usage: ") { Some(u) => format!("usage: warp audio {u}"), None => e }),
        Some("new") => new(&args),
        None => match bundled() {
            Some(game) => play(&["run".to_string(), game.display().to_string()]),
            None => { println!("{USAGE}"); Ok(()) }
        },
        Some("help" | "--help" | "-h") => { println!("{USAGE}"); Ok(()) }
        // A game folder (or one-file game) on its own: a runtime launch (the
        // GameNight installer passes the folder; a file manager, the file).
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

fn play(args: &[String]) -> Result<(), String> {
    println!("{} warp: starting", app::manners::stamp());
    let dir = dir(args)?;
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
    let mut game = LuaGame::load(&dir, &overrides(args)?, sender)?;
    if game.connect_party() { println!("{} warp: connected to the party host", app::manners::stamp()); }
    if let Some(cfg) = live::Config::from_args(&game.live_name(), args)? { game.start_live(cfg)?; }
    else if game.party.is_some() && std::env::var("WARP_LIVE").as_deref() != Ok("0") {
        // Under GameNight the session assistant changes the game through
        // live control (engine/docs/live-control.md): any free local port.
        let mut cfg = live::Config::new(&game.live_name());
        cfg.port = 0;
        cfg.announce = false;
        if let Err(e) = game.start_live(cfg) { eprintln!("live: {e}"); }
    }
    let cfg = app::Config {
        title: game.cfg.title.clone(), size: size(args, game.cfg.size)?, tick_hz: game.cfg.tick_rate,
        fullscreen: args.iter().any(|a| a == "--fullscreen"), keyboard: true, frames: parse(args, "--frames")?.unwrap_or(0), max_render_height: None,
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
    let mut game = LuaGame::load(&dir, &overrides(args)?, Some(tx))?;
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
    let mut game = LuaGame::load(&dir, &overrides(args)?, Some(tx))?;
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
    let mut game = LuaGame::load(&dir, &overrides(args)?, None)?;
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
