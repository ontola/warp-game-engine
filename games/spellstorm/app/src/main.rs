#![cfg_attr(windows, windows_subsystem = "windows")]
//! Spellstorm on Warp.
//!
//!   spellstorm play [--players 1] [--bots 0] [--seed 7] [--fullscreen]
//!   spellstorm shot --out frame.png [--seed 7] [--bots 3] [--ticks 240] [--size 1920x1080] [--menu]
//!
//! The sim is games/spellstorm/sim (hecs, the Godot game's rules); the look
//! comes from the Godot game's own models and materials (assets/kits).
//! SPELLSTORM_EVENTS=1 makes a shot count the sim's events.
mod anim;
mod effects;
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
mod ragdoll;
mod rig;
mod shrines;
mod skins;
mod view;

use render::Gpu;

fn main() {
    app::startup::init("Spellstorm", Some(include_bytes!("../assets/branding/logo.png")));
    match run() {
        Ok(()) => app::startup::note_exit("ended"),
        Err(e) => {
            app::startup::note_exit(&format!("error: {e}"));
            std::process::exit(1);
        }
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
            let cfg = app::Config { title: "Spellstorm".into(), size: [1920, 1080], tick_hz: 60.0, fullscreen: !flag("--windowed"), keyboard: true, pointer: false, frames: numf("--frames", 0.0) as u64, max_render_height: None, target_fps: None };
            app::run(cfg, &mut game)
        }
        "shot" => {
            let out = arg("--out").ok_or("--out FILE required")?;
            let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1920), b.parse().unwrap_or(1080)))).unwrap_or((1920u32, 1080u32));
            // --humans N: seats nobody drives (they stand where they start).
            let mut game = play::Spellstorm::new(kits::Kits::load(None)?, numf("--humans", 0.0) as usize, numf("--bots", 3.0) as usize, seed);
            // Seats nobody drives would sit in the start menu.
            game.menu.open = false;
            // --biome N: start at level N+1 (0 chapel .. 5 pit).
            if let Some(b) = arg("--biome").and_then(|v| v.parse::<usize>().ok()) {
                game.world.run.biome_index = b;
                game.world.enter_level();
            }
            if flag("--practice") {
                // The practice strip, as for players who know nothing yet.
                game.world.settings.force_practice = true;
                game.world.new_run();
            }
            // --at M: the wizards M metres along the level, packs and walls
            // before it left out (a look at a wall, a clearing).
            if let Some(m) = arg("--at").and_then(|v| v.parse::<f32>().ok()) {
                game.step(&[]);
                game.world.skip_to(m);
            }
            // --spawn a,b,c: those monsters in a row 5 m ahead of the first
            // wizard (a look at each body).
            if let Some(list) = arg("--spawn") {
                game.step(&[]);
                let at = game.world.pos(game.world.wizards[0]);
                let keys: Vec<&str> = list.split(',').collect();
                for (i, k) in keys.iter().enumerate() {
                    let kind = spellstorm::enemy::KINDS.iter().position(|c| c.key == *k).ok_or(format!("no monster {k}"))?;
                    let x = (i as f32 - (keys.len() as f32 - 1.0) * 0.5) * 2.2;
                    game.world.spawn_enemy(kind, at + glam::Vec3::new(x, 0.0, -5.0), false, -1);
                }
            }
            // --enchant fire,fire,...: the first wizard's sword enchanted with
            // that queue (a look at the blade).
            if let Some(list) = arg("--enchant") {
                game.step(&[]);
                let els: Vec<spellstorm::El> = list.split(',').filter_map(|k| spellstorm::el::ALL.iter().copied().find(|e| format!("{e:?}").eq_ignore_ascii_case(k))).collect();
                let id = game.world.wizards[0];
                let mods = game.world.a(id).mods.clone();
                let s = spellstorm::compose(&els, spellstorm::Form::Enchant, &mods);
                let mut wz = game.world.wiz_mut(id);
                wz.enchant_time = s.n.duration.max(30.0);
                wz.enchant = Some(Box::new(s));
            }
            // --cast poison,fire,air: the first wizard's spray or beam of
            // that queue, running (a look at a stream).
            if let Some(list) = arg("--cast") {
                game.step(&[]);
                let els: Vec<spellstorm::El> = list.split(',').filter_map(|k| spellstorm::el::ALL.iter().copied().find(|e| format!("{e:?}").eq_ignore_ascii_case(k))).collect();
                let id = game.world.wizards[0];
                let mods = game.world.a(id).mods.clone();
                let s = spellstorm::compose(&els, spellstorm::Form::Cast, &mods);
                let at = game.world.pos(id) + glam::Vec3::new(0.0, 1.2, 0.0);
                eprintln!("cast: {} {:?}", s.name, s.shape);
                match s.shape {
                    spellstorm::Shape::Spray => {
                        let c = game.world.start_spray(id, &s, at);
                        game.world.wiz_mut(id).channel = Some(c);
                    }
                    spellstorm::Shape::Beam => {
                        let c = game.world.start_beam(id, &s, at);
                        game.world.wiz_mut(id).channel = Some(c);
                    }
                    _ => {}
                }
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
            // --lua CODE [--lua-at TICK]: a Luau line against the world, at
            // that tick or else just before the picture.
            let lua_at = arg("--lua-at").and_then(|v| v.parse::<u32>().ok());
            for t in 0..ticks * if until_swing { 20 } else { 1 } {
                if lua_at == Some(t) && let Some(code) = arg("--lua") {
                    eprintln!("lua: {}", spellstorm::rules::console(&mut game.world, &code)?);
                    // Its events (a nova, a burst) show: the next tick clears them.
                    game.view.fx.absorb(&game.world);
                }
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
            if let Some(code) = arg("--lua").filter(|_| lua_at.is_none()) {
                eprintln!("lua: {}", spellstorm::rules::console(&mut game.world, &code)?);
                game.view.fx.absorb(&game.world);
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
