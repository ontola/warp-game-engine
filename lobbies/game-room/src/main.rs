//! The GameNight Game Room lobby on Warp.
//!
//!   game-room shot [--dump DIR] --out room.png [--size 1280x720]
//!   game-room play [--players 1] [--bots 3] [--fullscreen]
//!   game-room live-shot --out room.png [--ticks 180] [--bots 3] [--items] [--fx]
//!
//! For now the room comes from the Godot lobby's export (tools/
//! warp_export.gd in gamenight/lobbies/game-room), so both engines draw the
//! same meshes, materials and textures while the look is matched.
use game_room::dump::{linear, Room};
use game_room::stage::{godot_ev, Stage};
use glam::Vec3;
use render::{Gpu, Lighting, Renderer, Scene};
use std::path::PathBuf;


fn main() { if let Err(e) = run() { eprintln!("game-room: {e}"); std::process::exit(1); } }

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let flag = |k: &str| args.iter().any(|a| a == k);
    let numf = |k: &str, d: f32| arg(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    if args.get(1).map(String::as_str) == Some("listing") {
        // The comparable listing of a Godot export, or of the Rust-built room.
        let text = match arg("--dump") {
            Some(dir) => { let r = Room::load(std::path::Path::new(&dir))?; game_room::fixture::listing(&r.dump, &r.bin, &[]) }
            None => { let mut b = game_room::room::build(); b.pose(numf("--time", 0.0), 0.0, &Default::default()); game_room::fixture::listing(&b.dump, &b.bin, &b.hidden) }
        };
        print!("{text}");
        return Ok(());
    }
    // Started by the GameNight host (bundled or registered lobby): the env
    // carries GAMENIGHT_LOBBY_API, so no subcommand means "be the lobby".
    let hosted = args.len() == 1 && std::env::var("GAMENIGHT_LOBBY_API").is_ok_and(|v| v == "1");
    if hosted || args.get(1).map(String::as_str) == Some("lobby") {
        // The GameNight lobby: connects to the host from GAMENIGHT_ADDR,
        // fullscreen unless --windowed.
        let options = game_room::stage::Options::from_args(numf, flag, |k| arg(k).is_some());
        let mut lobby = game_room::live::Lobby::connect(options)?;
        // Controllers come from the host as frames; no local keyboard or mouse.
        let cfg = app::Config { title: "GameNight Game Room".into(), size: [1600, 900], fullscreen: !flag("--windowed"), keyboard: false, frames: numf("--frames", 0.0) as u64, ..Default::default() };
        return app::run(cfg, &mut lobby);
    }
    if matches!(args.get(1).map(String::as_str), Some("play") | Some("live-shot")) {
        // The live lobby: `play` in a window, `live-shot` headless after
        // --ticks; with --host it joins the GameNight host instead of a demo.
        let options = game_room::stage::Options::from_args(numf, flag, |k| arg(k).is_some());
        let local = numf("--players", 1.0) as usize;
        let bots = numf("--bots", 3.0) as usize;
        let mut lobby = if flag("--host") { game_room::live::Lobby::connect(options)? } else { game_room::live::Lobby::new(options, local, bots)? };
        if args[1] == "play" {
            let cfg = app::Config { title: "GameNight Game Room".into(), size: [1600, 900], fullscreen: flag("--fullscreen"), frames: numf("--frames", 0.0) as u64, ..Default::default() };
            return app::run(cfg, &mut lobby);
        }
        let out = arg("--out").ok_or("--out FILE required")?;
        let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280), b.parse().unwrap_or(720)))).unwrap_or((1280u32, 720u32));
        let gpu = Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], 60.0)?;
        if flag("--items") {
            // One of every item along the floor, for a look at the props.
            use game_room_sim::world::Kind::*;
            for (i, k) in [Bomb, Blaster, Popcorn, Minigun, Bazooka, Laser, Saber, Ball].into_iter().enumerate() {
                lobby.world.spawn_item(k, -7.0 + i as f64 * 2.0, 3.0);
            }
        }
        let ticks = numf("--ticks", 180.0) as u32;
        let mut rgba = headless.frame(&mut lobby)?;
        for t in 0..ticks {
            if flag("--fx") && t + 8 == ticks {
                // An explosion, a laser and a spawn, caught mid-effect.
                use game_room_sim::world::{Ev, Event};
                let e = |ev, x, y, value| Event { ev, x, y, player: None, kind: None, value };
                for ev in [e(Ev::Explode, 3.0, 1.5, 0.0), e(Ev::Laser, -8.0, 3.0, -2.0), e(Ev::Spawn, -5.0, 0.0, 0.0), e(Ev::Hit, 6.0, 2.0, 0.0)] { lobby.event(ev); }
            }
            app::Game::tick(&mut lobby, &vec![app::Controls::default(); local]);
            // A real host answers in real time.
            if flag("--host") { std::thread::sleep(std::time::Duration::from_millis(16)); }
            rgba = headless.frame(&mut lobby)?;
        }
        return image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string());
    }
    if args.get(1).map(String::as_str) == Some("tone") { return tone(&arg("--out").ok_or("--out FILE")?, numf("--ev", godot_ev(1.05))); }
    if args.get(1).map(String::as_str) != Some("shot") { return Err("usage: game-room shot [--dump DIR] --out room.png [--size WxH]".into()); }
    let out = arg("--out").ok_or("--out FILE required")?;
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280), b.parse().unwrap_or(720)))).unwrap_or((1280u32, 720u32));
    let options = game_room::stage::Options::from_args(numf, flag, |k| arg(k).is_some());
    let started = std::time::Instant::now();

    // A Godot export with --dump, otherwise the room built in Rust.
    let room = match arg("--dump") {
        Some(dir) => Room::load(&PathBuf::from(dir))?,
        None => { let mut b = game_room::room::build(); b.pose(numf("--time", 0.0), 0.0, &Default::default()); Room::from_build(b) }
    };
    let stage = Stage::new(room, options, |_, _| {})?;
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &stage.scene, &stage.light);
    let mut screen = render::screen::Screen::new(&gpu)?;
    stage.upload(&gpu, &renderer, &mut screen)?;
    let show = game_room::paint::Show { time: numf("--time", 0.0), ..Default::default() };
    stage.paint(&gpu, &renderer, &mut screen, &show, &Default::default(), None)?;
    let setup = started.elapsed();

    let frames_start = std::time::Instant::now();
    let mut rgba = vec![];
    for _ in 0..numf("--frames", 8.0) as u32 { rgba = renderer.render(&gpu, &stage.room.camera, &stage.light, w, h).0; }
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    let room = &stage.room;
    println!("{} instances, {} local + {} shadowed lights, {} sprites, {} probes; setup {:.2}s (bake {:.2}s), frames {:.2}s",
        room.dump.instances.len(), room.locals.len(), room.spots.len(), room.dump.sprites.len() + room.dump.labels.len(), stage.probes.len(), setup.as_secs_f32(), stage.bake_seconds, frames_start.elapsed().as_secs_f32());
    Ok(())
}

/// Tonemapping check against Godot (AgX, exposure 1.05): unshaded squares
/// of the same colours as the calibration scene in Godot.
fn tone(out: &str, ev: f32) -> Result<(), String> {
    let mut scene = Scene::default();
    scene.sky = Some((4, 2, vec![[0.0, 0.0, 0.0, 1.0]; 8]));
    for (i, v) in [0.05f32, 0.2, 0.5, 1.0, 2.0, 4.0, 8.0].iter().enumerate() {
        let c = linear(&serde_json::json!([v, v * 0.5, v * 0.25, 1.0]));
        let peak = c.max_element();
        let m = scene.add_material(render::scene::Material { tint: (c / peak).to_array(), layer: -1, roughness: 1.0, flags: render::scene::EMISSIVE, scale: peak, metal: 0.0 });
        let mesh = scene.add_mesh(&render::primitives::quad(m).mesh());
        scene.add_instance(mesh, glam::Affine3A::from_scale_rotation_translation(Vec3::splat(0.9), glam::Quat::IDENTITY, Vec3::new(i as f32 - 3.0, 0.0, 0.0)));
    }
    let light = Lighting { sun_lux: 0.0, sky_lux: 0.0, ev100: ev, fog_density: 0.0, ..Default::default() };
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    let mut options = renderer.debug_options();
    options.bloom_enabled = false;
    renderer.set_debug_options(options)?;
    let cam = render::Camera { pos: Vec3::new(0.0, 0.0, 6.0), target: Vec3::ZERO, fov_y: 75f32.to_radians(), near: 0.05 };
    let mut rgba = vec![];
    for _ in 0..4 { rgba = renderer.render(&gpu, &cam, &light, 640, 200).0; }
    image::save_buffer(out, &rgba, 640, 200, image::ColorType::Rgba8).map_err(|e| e.to_string())
}
