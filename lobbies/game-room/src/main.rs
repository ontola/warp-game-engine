//! The GameNight Game Room lobby on Warp.
//!
//!   game-room shot --dump DIR --out room.png [--size 1280x720]
//!
//! For now the room comes from the Godot lobby's export (tools/
//! warp_export.gd in gamenight/lobbies/game-room), so both engines draw the
//! same meshes, materials and textures while the look is matched.
mod dump;

use dump::{linear, mat4, Room, Tuning};
use glam::Vec3;
use render::hud::{HudDrawList, ImageFilter, TextAlign, TextRun};
use render::irradiance_volume::{self, BoxRoom, Grid};
use render::picture::Picture;
use render::sprites::{self, Canvas, Sprite, SpriteImage};
use render::{Gpu, Lighting, Renderer, Scene};
use std::f32::consts::PI;
use std::path::PathBuf;

static OUTFIT_600: &[u8] = include_bytes!("../assets/fonts/outfit-600.ttf");
static OUTFIT_800: &[u8] = include_bytes!("../assets/fonts/outfit-800.ttf");

fn main() { if let Err(e) = run() { eprintln!("game-room: {e}"); std::process::exit(1); } }

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let flag = |k: &str| args.iter().any(|a| a == k);
    let numf = |k: &str, d: f32| arg(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    if args.get(1).map(String::as_str) == Some("tone") { return tone(&arg("--out").ok_or("--out FILE")?, numf("--ev", godot_ev(1.05))); }
    if args.get(1).map(String::as_str) != Some("shot") { return Err("usage: game-room shot --dump DIR --out room.png [--size WxH]".into()); }
    let dir = PathBuf::from(arg("--dump").ok_or("--dump DIR required")?);
    let out = arg("--out").ok_or("--out FILE required")?;
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280), b.parse().unwrap_or(720)))).unwrap_or((1280u32, 720u32));
    let tuning = Tuning { light: numf("--light", PI), ambient: numf("--ambient", PI) };
    let gi = numf("--gi", 1.6);
    let started = std::time::Instant::now();

    let mut room = Room::load(&dir)?;
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
        contrast: num(&room.dump.environment, "adjustment_contrast", 1.0), saturation: num(&room.dump.environment, "adjustment_saturation", 1.0), ..Default::default() };
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    renderer.set_local_lights(&room.locals)?;
    renderer.set_headlights(&room.spots)?;
    let env = &room.dump.environment;
    let fog = (env["volumetric_fog_enabled"].as_bool().unwrap_or(false) && !flag("--no-fog")).then(|| render::headlights::VolumetricFog {
        density: num(env, "volumetric_fog_density", 0.0) * numf("--fog-density", 1.0),
        albedo: linear(&env["volumetric_fog_albedo"]),
        emission: linear(&env["volumetric_fog_emission"]) * num(env, "volumetric_fog_emission_energy", 0.0),
        anisotropy: num(env, "volumetric_fog_anisotropy", 0.2),
        gi_inject: num(env, "volumetric_fog_gi_inject", 1.0) * numf("--fog-gi", 0.4),
        cone_blur: numf("--fog-blur", 12.0).to_radians(),
        length: num(env, "volumetric_fog_length", 64.0),
        steps: numf("--fog-steps", 24.0) as u32,
        spot_energy: room.spot_fog.iter().map(|e| e * numf("--fog", 0.6)).collect(),
        local_energy: room.local_fog.iter().map(|e| e * numf("--fog", 0.6)).collect(),
    });
    let fog_params = fog.as_ref().map(|f| (f.density, f.length));
    renderer.set_light_extras(fog, &room.spot_falloff);
    let mut options = renderer.debug_options();
    // Godot's glow: levels 3-6 (wide), threshold 0.9.
    options.bloom_strength = numf("--bloom", 0.3);
    options.bloom_rounds = numf("--bloom-rounds", 4.0) as u32;
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
    let ambient_at = |p: Vec3, n: Vec3| if probes_on { irradiance_volume::sample(&grid, &probes, p, n) } else { room.ambient };

    // Sprite3D players and door faces.
    let mut list = vec![];
    for (i, s) in room.dump.sprites.iter().enumerate() {
        let tex = s["texture"].as_i64().unwrap_or(-1);
        if tex < 0 { continue; }
        let img = &room.images[tex as usize];
        let id = SpriteImage(1000 + i as u64);
        let nearest = s["filter"].as_i64().unwrap_or(0) == 0 || s["filter"].as_i64() == Some(1) || s["filter"].as_i64() == Some(4);
        renderer.set_sprite_image(&gpu, id, &Picture::new(img.width(), img.height(), img.as_raw().clone())?, if nearest { ImageFilter::Nearest } else { ImageFilter::Linear });
        let (hf, vf) = (s["hframes"].as_u64().unwrap_or(1).max(1) as u32, s["vframes"].as_u64().unwrap_or(1).max(1) as u32);
        let frame = s["frame"].as_u64().unwrap_or(0) as u32;
        let px = num(s, "pixel_size", 0.01);
        let (fw, fh) = (img.width() / hf, img.height() / vf);
        let xf = mat4(&s["transform"]);
        let (right, up) = (xf.x_axis.truncate(), xf.y_axis.truncate());
        let offset = glam::Vec2::new(s["offset"][0].as_f64().unwrap_or(0.0) as f32, s["offset"][1].as_f64().unwrap_or(0.0) as f32);
        let center = xf.w_axis.truncate() + (right * offset.x + up * offset.y) * px;
        let size = [fw as f32 * px * right.length(), fh as f32 * px * up.length()];
        let mut uv = [(frame % hf) as f32 / hf as f32, (frame / hf) as f32 / vf as f32, 1.0 / hf as f32, 1.0 / vf as f32];
        if s["flip_h"].as_bool().unwrap_or(false) { uv[0] += uv[2]; uv[2] = -uv[2]; }
        let modulate = linear(&s["modulate"]);
        let normal = right.cross(up).normalize_or(Vec3::Z);
        let radiance = if s["shaded"].as_bool().unwrap_or(false) {
            modulate * lambert(&all_lights, center, normal, ambient_at(center, normal)) / PI
        } else { modulate };
        if flag("--debug-sprites") { println!("sprite {i} at {center:.2}: direct {:.2} indirect {:.2}", lambert(&all_lights, center, normal, Vec3::ZERO), ambient_at(center, normal)); }
        let mut sprite = Sprite::fixed(center, size, right.normalize(), up.normalize(), id, uv, radiance);
        if s["billboard"].as_i64().unwrap_or(0) != 0 { sprite = Sprite::billboard(center, size, id, uv, radiance); }
        sprite.depth_bias = 0.02;
        list.push(sprite);
    }

    // Label3D tags: canvas text (Outfit), unshaded.
    let semibold = render::hud::register_font(OUTFIT_600);
    let bold = render::hud::register_font(OUTFIT_800);
    let mut canvases = vec![];
    for (i, l) in room.dump.labels.iter().enumerate() {
        let text = l["text"].as_str().unwrap_or("").to_string();
        let face = if l["weight"].as_i64().unwrap_or(600) >= 700 { bold } else { semibold };
        let ss = 4.0;
        let size = num(l, "font_size", 32.0) * ss;
        let outline = num(l, "outline_size", 0.0) * ss;
        let [tw, th] = render::hud::measure(&text, face, size);
        let pad = outline + 4.0;
        let cw = ((tw + pad * 2.0).ceil() as u32).clamp(8, 4096);
        let ch = ((th + pad * 2.0).ceil() as u32).clamp(8, 4096);
        let modulate = linear(&l["modulate"]);
        let peak = modulate.max_element().max(1e-6);
        let srgb = |v: f32| if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        let c = modulate / peak;
        let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let color = [to8(srgb(c.x)), to8(srgb(c.y)), to8(srgb(c.z)), to8(dump_alpha(&l["modulate"]))];
        let oc = linear(&l["outline_modulate"]);
        let ocolor = [to8(srgb(oc.x / peak)), to8(srgb(oc.y / peak)), to8(srgb(oc.z / peak)), to8(dump_alpha(&l["outline_modulate"]))];
        let mut hud = HudDrawList::new([cw, ch]);
        let run = |id: u64, at: [f32; 2], color: [u8; 4]| TextRun { id, text: text.clone(), position: at, bounds: [0.0, 0.0, cw as f32, ch as f32],
            font_size: size, color, font: face, align: TextAlign::Left, width: cw as f32 };
        if outline > 0.0 {
            for k in 0..16 {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                hud.text.push(run(k + 1, [pad + a.cos() * outline * 0.5, pad + a.sin() * outline * 0.5], ocolor));
            }
        }
        hud.text.push(run(100, [pad, pad], color));
        let mut canvas = Canvas::new(&gpu, [cw, ch])?;
        canvas.draw(&gpu, &hud)?;
        let id = SpriteImage(2000 + i as u64);
        renderer.set_sprite_texture(&gpu, id, canvas.texture(), ImageFilter::Linear);
        canvases.push(canvas);
        let xf = mat4(&l["transform"]);
        let px = num(l, "pixel_size", 0.01) / ss;
        let (right, up) = (xf.x_axis.truncate(), xf.y_axis.truncate());
        let world = [cw as f32 * px * right.length(), ch as f32 * px * up.length()];
        let center = xf.w_axis.truncate();
        let mut sprite = Sprite::fixed(center, world, right.normalize(), up.normalize(), id, [0.0, 0.0, 1.0, 1.0], Vec3::splat(peak));
        if l["billboard"].as_i64().unwrap_or(0) != 0 { sprite = Sprite::billboard(center, world, id, [0.0, 0.0, 1.0, 1.0], Vec3::splat(peak)); }
        sprite.alpha_cut = 0.0;
        sprite.depth_bias = 0.02;
        list.push(sprite);
    }
    // The lamp fog between camera and sprite dims it like the scene behind.
    if let Some(f) = &fog_params {
        for s in &mut list { s.radiance *= (-f.0 * room.camera.pos.distance(s.center).min(f.1)).exp(); }
    }
    renderer.set_sprites(&gpu, &list)?;
    let setup = started.elapsed();

    let frames_start = std::time::Instant::now();
    let mut rgba = vec![];
    for _ in 0..numf("--frames", 8.0) as u32 { rgba = renderer.render(&gpu, &room.camera, &light, w, h).0; }
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    println!("{} instances, {} local + {} shadowed lights, {} sprites, {} probes; setup {:.2}s (bake {:.2}s), frames {:.2}s",
        room.dump.instances.len(), room.locals.len(), room.spots.len(), list.len(), probes.len(), setup.as_secs_f32(), bake.as_secs_f32(), frames_start.elapsed().as_secs_f32());
    Ok(())
}

/// Irradiance on a flat sprite from every lamp (unshadowed), like Godot's
/// shaded Sprite3D, plus the indirect light at its centre.
fn lambert(lights: &[render::headlights::LocalLight], p: Vec3, n: Vec3, ambient: Vec3) -> Vec3 {
    lights.iter().filter_map(|l| sprites::light_at(l, p)).map(|(to, e)| e * n.dot(to).max(0.0)).sum::<Vec3>() + ambient
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
