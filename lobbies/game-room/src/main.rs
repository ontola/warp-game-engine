//! The GameNight Game Room lobby on Warp.
//!
//!   game-room shot --dump DIR --out room.png [--size 1280x720]
//!
//! For now the room comes from the Godot lobby's export (tools/
//! warp_export.gd in gamenight/lobbies/game-room), so both engines draw the
//! same meshes, materials and textures while the look is matched.
use game_room::dump;

use dump::{linear, mat4, Room, Tuning};
use glam::Vec3;
use render::hud::{HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::{BoxRoom, Grid};
use render::{Gpu, Lighting, Renderer, Scene};
use std::f32::consts::PI;
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
    if args.get(1).map(String::as_str) == Some("tone") { return tone(&arg("--out").ok_or("--out FILE")?, numf("--ev", godot_ev(1.05))); }
    if args.get(1).map(String::as_str) != Some("shot") { return Err("usage: game-room shot [--dump DIR] --out room.png [--size WxH]".into()); }
    let out = arg("--out").ok_or("--out FILE required")?;
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280), b.parse().unwrap_or(720)))).unwrap_or((1280u32, 720u32));
    let tuning = Tuning { light: numf("--light", PI), ambient: numf("--ambient", PI) };
    let gi = numf("--gi", 1.6);
    let started = std::time::Instant::now();

    // A Godot export with --dump, otherwise the room built in Rust.
    let mut room = match arg("--dump") {
        Some(dir) => Room::load(&PathBuf::from(dir))?,
        None => { let mut b = game_room::room::build(); b.pose(numf("--time", 0.0), 0.0, &Default::default()); Room::from_build(b) }
    };
    let mut scene = Scene::default();
    let bg = linear(&room.dump.environment["background_color"]);
    scene.sky = Some((4, 2, vec![[bg.x, bg.y, bg.z, 1.0]; 8]));
    room.build(&mut scene, &tuning)?;
    // Probes over the room interior, half a metre apart.
    let lo = room.min.max(Vec3::new(-12.0, 0.0, -3.0)) + Vec3::splat(0.25);
    let hi = room.max.min(Vec3::new(12.0, 11.0, 8.0)) - Vec3::splat(0.25);
    let cell = 0.5;
    let dims = ((hi - lo) / cell).ceil().max(Vec3::ONE);
    let grid = Grid { origin: lo, cell, dims: [dims.x as u32 + 1, dims.y as u32 + 1, dims.z as u32 + 1] };
    let probes_on = !flag("--no-probes");
    if probes_on { scene.reserve_irradiance_volume(grid)?; }

    let ev = numf("--ev", godot_ev(num(&room.dump.environment, "tonemap_exposure", 1.0)));
    let light = Lighting { sun_lux: 0.0, sky_lux: 0.0, fog_density: 0.0, fog_start: 1000.0, fog_end: 2000.0, ev100: ev,
        contrast: num(&room.dump.environment, "adjustment_contrast", 1.0), saturation: num(&room.dump.environment, "adjustment_saturation", 1.0),
        // The lobby uses 2x MSAA, no TAA: crisp pixel art.
        taa: numf("--taa", 0.0), sharpen: numf("--sharpen", 0.5), ..Default::default() };
    room.add_sprites(&mut scene, &room.camera);
    room.add_particles(&mut scene, &room.camera)?;
    let (label_layer, label_list) = add_labels(&mut scene, &room);
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    renderer.set_local_lights(&room.locals)?;
    renderer.set_headlights(&room.spots)?;
    renderer.set_spot_falloff(&room.spot_falloff);
    let mut options = renderer.debug_options();
    // Godot's glow: levels 3-6 summed, screen-blended. The 0.36 matches its
    // capture: Godot's levels are smaller per level than our wide rounds.
    options.bloom_screen = !flag("--bloom-add");
    options.bloom_strength = numf("--bloom", num(&room.dump.environment, "glow_intensity", 0.8) * 0.36);
    options.bloom_rounds = numf("--bloom-rounds", 6.0) as u32;
    options.bloom_threshold = numf("--bloom-threshold", num(&room.dump.environment, "glow_hdr_threshold", 2.0));
    renderer.set_debug_options(options)?;
    let all_lights = room.all_lights();

    // Indirect light: the room's own bounce, scaled like Godot's sdfgi_energy.
    let bake_start = std::time::Instant::now();
    let probes = if probes_on {
        // Like SDFGI: rays that leave the room see Godot's ambient colour.
        let bake = BoxRoom { blocks: room.blocks.clone(), lights: all_lights.clone(), sky: room.ambient / PI };
        let mut probes = bake.bake(&grid, numf("--rays", 64.0) as u32, 2)?;
        for p in &mut probes { for a in &mut p.axes { *a *= gi; } }
        renderer.set_irradiance_volume(&gpu, &probes)?;
        probes
    } else { vec![] };
    let bake = bake_start.elapsed();

    // Godot's volumetric fog: lamp beams, ambient light in the haze, and the
    // scene behind dimmed by it.
    let env = &room.dump.environment;
    let mut light = light;
    if env["volumetric_fog_enabled"].as_bool().unwrap_or(false) && !flag("--no-fog") {
        let albedo = linear(&env["volumetric_fog_albedo"]);
        // Mean probe irradiance stands in for SDFGI's injection into the froxels.
        let mean = if probes.is_empty() { room.ambient } else { probes.iter().map(|p| p.axes.iter().sum::<Vec3>() / 6.0).sum::<Vec3>() / probes.len() as f32 };
        let inject = num(env, "volumetric_fog_gi_inject", 1.0) * numf("--fog-gi", 0.4);
        let emission = linear(&env["volumetric_fog_emission"]) * num(env, "volumetric_fog_emission_energy", 0.0);
        light.volumetric = Some(render::volumetric::Volumetric {
            density: num(env, "volumetric_fog_density", 0.0) * numf("--fog-density", 1.0),
            color: albedo,
            anisotropy: num(env, "volumetric_fog_anisotropy", 0.2),
            intensity: numf("--fog", 0.6),
            extinction: 1.0,
            ambient: albedo * mean * inject / (4.0 * PI) + emission,
            length: num(env, "volumetric_fog_length", 64.0),
        });
        // Every lamp scatters by its volumetric fog energy, its cone widened
        // like Godot's coarse froxels (power kept).
        let blur = numf("--fog-blur", 12.0).to_radians();
        let beams: Vec<_> = all_lights.iter().zip(room.local_fog.iter().chain(&room.spot_fog)).filter(|(_, e)| **e > 0.0).map(|(l, e)| {
            let mut b = *l;
            if l.outer_cos > -0.99 {
                let (outer, inner) = (l.outer_cos.acos(), l.inner_cos.acos());
                let wide = (outer + blur).min(std::f32::consts::PI);
                b.outer_cos = wide.cos();
                b.inner_cos = (inner + blur).min(wide - 0.01).cos();
                b.intensity_cd *= (1.0 - l.outer_cos) / (1.0 - b.outer_cos).max(1e-4);
            }
            b.intensity_cd *= *e;
            b
        }).collect();
        renderer.set_volumetric_beams(&beams)?;
    }

    let mut screen = render::screen::Screen::new(&gpu)?;
    screen.draw(&gpu, &renderer, label_layer, [0.0; 4], &label_list)?;
    let setup = started.elapsed();

    let frames_start = std::time::Instant::now();
    let mut rgba = vec![];
    for _ in 0..numf("--frames", 8.0) as u32 { rgba = renderer.render(&gpu, &room.camera, &light, w, h).0; }
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    println!("{} instances, {} local + {} shadowed lights, {} sprites, {} probes; setup {:.2}s (bake {:.2}s), frames {:.2}s",
        room.dump.instances.len(), room.locals.len(), room.spots.len(), room.dump.sprites.len() + room.dump.labels.len(), probes.len(), setup.as_secs_f32(), bake.as_secs_f32(), frames_start.elapsed().as_secs_f32());
    Ok(())
}

/// Label3D tags (Outfit, unshaded) as glowing blended quads in the scene, so
/// fog, depth and anti-aliasing treat them like Godot does. All tags share
/// one screen layer; returns it and the draw list that fills it.
fn add_labels(scene: &mut Scene, room: &Room) -> (i32, HudDrawList) {
    let semibold = game_room::fonts::faces().regular;
    let bold = game_room::fonts::faces().bold;
    let side = render::screen::SIZE;
    let layer = scene.add_screen_texture("labels");
    let mut hud = HudDrawList::new([side, side]);
    let (mut x, mut y, mut row) = (0.0f32, 0.0f32, 0.0f32);
    let srgb = |v: f32| if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for l in &room.dump.labels {
        let text = l["text"].as_str().unwrap_or("").to_string();
        let face = if l["weight"].as_i64().unwrap_or(600) >= 700 { bold } else { semibold };
        let ss = 2.0;
        let size = num(l, "font_size", 32.0) * ss;
        let outline = num(l, "outline_size", 0.0) * ss;
        let [tw, th] = render::hud::measure(&text, face, size);
        let pad = outline + 4.0;
        let (cw, ch) = ((tw + pad * 2.0).ceil().min(side as f32), (th + pad * 2.0).ceil().min(side as f32));
        if x + cw > side as f32 { x = 0.0; y += row; row = 0.0; }
        if y + ch > side as f32 { eprintln!("game-room: label atlas full, skipping {text:?}"); continue; }
        let modulate = linear(&l["modulate"]);
        let peak = modulate.max_element().max(1e-6);
        let oc = linear(&l["outline_modulate"]) / peak;
        let color = [255, 255, 255, 255];
        // The material tints by the text colour; the outline keeps its own ratio to it.
        let c = modulate / peak;
        let ocolor = [to8(srgb(oc.x / c.x.max(1e-3))), to8(srgb(oc.y / c.y.max(1e-3))), to8(srgb(oc.z / c.z.max(1e-3))), to8(dump_alpha(&l["outline_modulate"]))];
        let run = |id: u64, at: [f32; 2], color: [u8; 4]| TextRun { id, text: text.clone(), position: at, bounds: [x, y, cw, ch],
            font_size: size, color, font: face, align: TextAlign::Left, width: cw };
        let base = hud.text.len() as u64 * 32;
        if outline > 0.0 {
            for k in 0..16 {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                hud.text.push(run(base + k + 1, [x + pad + a.cos() * outline * 0.5, y + pad + a.sin() * outline * 0.5], ocolor));
            }
        }
        hud.text.push(run(base + 31, [x + pad, y + pad], color));
        let px = num(l, "pixel_size", 0.01) / ss;
        let (w, h) = (cw * px, ch * px);
        let uv = |u: f32, v: f32| [(x + u * cw) / side as f32, (y + v * ch) / side as f32];
        let corner = |u: f32, v: f32| render::cluster::Vertex { pos: [(u - 0.5) * w, (0.5 - v) * h, 0.0], normal: [0.0, 0.0, 1.0], uv: uv(u, v) };
        let verts = [corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0), corner(0.0, 0.0)];
        let material = scene.add_material(render::scene::Material { tint: c.to_array(), layer, roughness: 1.0,
            flags: render::scene::EMISSIVE | render::scene::TWO_SIDED, scale: peak, metal: 0.0 });
        scene.set_material_blend(material, dump_alpha(&l["modulate"]), true);
        scene.set_material_object_sort(material);
        let mesh = scene.add_mesh(&render::cluster::ClusterMesh::build(&[render::cluster::Part { vertices: &verts, indices: &[0, 1, 2, 0, 2, 3], material }]));
        let mut xf = mat4(&l["transform"]);
        if l["billboard"].as_i64().unwrap_or(0) != 0 { xf = dump::billboard(xf, &room.camera); }
        let (scale, rot, trans) = xf.to_scale_rotation_translation();
        scene.add_instance(mesh, glam::Affine3A::from_scale_rotation_translation(scale, rot, trans));
        x += cw + 2.0;
        row = row.max(ch + 2.0);
    }
    (layer, hud)
}

fn num(v: &serde_json::Value, k: &str, d: f32) -> f32 { v[k].as_f64().map(|x| x as f32).unwrap_or(d) }
fn dump_alpha(c: &serde_json::Value) -> f32 { c[3].as_f64().unwrap_or(1.0) as f32 }

/// Warp's ev100 that matches Godot's AgX at `exposure` (measured with
/// `tone` against Godot 4.5: equal mid greys at +0.43 stop).
fn godot_ev(exposure: f32) -> f32 { (1.0 / (1.2 * exposure)).log2() + 0.43 }

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
