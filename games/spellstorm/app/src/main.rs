//! Spellstorm on Warp.
//!
//!   spellstorm play [--players 1] [--bots 0] [--seed 7] [--fullscreen]
//!   spellstorm shot --out frame.png [--seed 7] [--bots 3] [--ticks 240] [--size 1920x1080] [--menu]
//!
//! The sim is games/spellstorm/sim (hecs, the Godot game's rules); the look
//! comes from the Godot game's own models and materials (assets/kits).
//! SPELLSTORM_EVENTS=1 makes a shot count the sim's events.
mod anim;
mod fx;
mod hud;
mod interp;
mod kits;
mod live;
mod look;
mod marks;
mod menu;
mod party;
mod play;
mod shrines;
mod view;

use render::Gpu;

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
    let flag = |k: &str| args.iter().any(|a| a == k);
    let seed = numf("--seed", 7.0) as u64;
    match args.get(1).map(String::as_str).unwrap_or("play") {
        "play" => {
            let humans = (numf("--players", 1.0) as usize).max(1);
            let party = party::Party::connect();
            let mut game = play::Spellstorm::with_party(kits::Kits::load(None)?, humans, numf("--bots", 0.0) as usize, seed, party);
            game.live = live::start(&args, live::game_dir().as_deref())?.map(Box::new);
            let cfg = app::Config { title: "Spellstorm".into(), size: [1920, 1080], tick_hz: 60.0, fullscreen: flag("--fullscreen"), keyboard: true, frames: numf("--frames", 0.0) as u64, max_render_height: None };
            app::run(cfg, &mut game)
        }
        "shot" => {
            let out = arg("--out").ok_or("--out FILE required")?;
            let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1920), b.parse().unwrap_or(1080)))).unwrap_or((1920u32, 1080u32));
            let mut game = play::Spellstorm::new(kits::Kits::load(None)?, 0, numf("--bots", 3.0) as usize, seed);
            if flag("--practice") {
                // The practice strip, as for players who know nothing yet.
                game.world.settings.force_practice = true;
                game.world.new_run();
            }
            let ticks = numf("--ticks", 240.0) as u32;
            // --skip: past this level's fights to its last shrine and portal.
            if flag("--skip") {
                game.step(&[]);
                game.world.skip_level();
            }
            let gpu = Gpu::new()?;
            let started = std::time::Instant::now();
            let mut shot = app::Headless::new(&gpu, [w, h], 60.0)?;
            // The first frame builds the scene; after it the view is posed
            // every 4 ticks, so the bodies' motion runs as in play.
            shot.frame(&mut game)?;
            let v = &game.view;
            eprintln!("scene: {} instances, {} meshes, {} materials, {} lamps in {:.2}s", v.scene.instances.len(), v.scene.meshes.len(), v.scene.materials.len(), v.lamps.len(), started.elapsed().as_secs_f32());
            let mut counts = std::collections::BTreeMap::new();
            // --until-swing: run on past --ticks until a sword is mid-swing.
            let until_swing = flag("--until-swing");
            for t in 0..ticks * if until_swing { 20 } else { 1 } {
                game.step(&[]);
                if until_swing && t >= ticks && game.world.wizards.iter().any(|&id| { let s = game.world.wiz(id).swing_t; let at: f64 = std::env::var("SPELLSTORM_SWING").ok().and_then(|v| v.parse().ok()).unwrap_or(0.15); s > at - 0.017 && s <= at }) {
                    break;
                }
                if std::env::var_os("SPELLSTORM_EVENTS").is_some() {
                    for e in &game.world.events {
                        let d = format!("{e:?}");
                        *counts.entry(d.split([' ', '(', '{']).next().unwrap_or("").to_string()).or_insert(0u32) += 1;
                    }
                }
                if t % 4 == 0 && (until_swing || t + 1 < ticks) {
                    if game.view.stale(&game.world) {
                        shot.frame(&mut game)?;
                    } else {
                        let r = shot.renderer().ok_or("no renderer")?;
                        game.view.pose(&gpu, r, &game.world, 4.0 / 60.0)?;
                    }
                }
            }
            if !counts.is_empty() {
                eprintln!("events: {counts:?}");
            }
            // --lua CODE: a Luau line against the world before the picture.
            if let Some(code) = arg("--lua") {
                eprintln!("lua: {}", spellstorm::rules::console(&mut game.world, &code)?);
                game.step(&[]);
            }
            if flag("--menu") {
                game.menu.show(false, &game.world);
            }
            let mut rgba = vec![];
            for _ in 0..numf("--frames", if until_swing { 1.0 } else { 4.0 }) as u32 {
                rgba = shot.frame(&mut game)?;
            }
            image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
        }
        _ => Err("usage: spellstorm play [--players N] [--bots N] [--live [PORT]] | shot --out frame.png [--seed 7] [--bots 3] [--ticks 240]".into()),
    }
}
