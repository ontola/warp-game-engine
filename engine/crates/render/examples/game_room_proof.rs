//! A Game Room-like lobby scene built only from engine features: box room,
//! neon tubes, a baked irradiance volume, 20+ local lights, TV and plaque
//! screens, pixel-art billboards and canvas name tags. A look test for
//! porting the Godot lobby, not a regression.
//! Run: game_room_proof --out room.png [--size 1280x720] [--no-probes]
use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::headlights::LocalLight;
use render::hud::{FontFace, HudDrawList, ImageFilter, TextAlign, TextRun};
use render::irradiance_volume::{self, Block, BoxRoom, Grid};
use render::picture::Picture;
use render::scene::{Material, EMISSIVE, TWO_SIDED};
use render::screen::{Screen, SIZE};
use render::sprites::{self, Canvas, Sprite, SpriteImage};
use render::{Camera, Gpu, Lighting, Renderer, Scene};

const BACK: f32 = -2.6;

fn hex(c: u32) -> Vec3 {
    let s = |v: u32| { let v = v as f32 / 255.0; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
    Vec3::new(s(c >> 16 & 255), s(c >> 8 & 255), s(c & 255))
}
fn srgb(c: u32) -> [f32; 4] { [(c >> 16 & 255) as f32 / 255.0, (c >> 8 & 255) as f32 / 255.0, (c & 255) as f32 / 255.0, 1.0] }

fn block(min: [f32; 3], max: [f32; 3], albedo: u32) -> Block {
    Block { min: Vec3::from(min), max: Vec3::from(max), albedo: hex(albedo), emission: Vec3::ZERO }
}
fn tube(min: [f32; 3], max: [f32; 3], color: u32, nits: f32) -> Block {
    Block { min: Vec3::from(min), max: Vec3::from(max), albedo: Vec3::ZERO, emission: hex(color) * nits }
}
fn point(p: [f32; 3], color: u32, cd: f32, range: f32) -> LocalLight {
    LocalLight { position: Vec3::from(p), direction: Vec3::NEG_Y, intensity_cd: hex(color) * cd, range_m: range,
        inner_cos: -0.9, outer_cos: -1.0, source_radius_m: 0.1, falloff: 2.0 }
}
fn spot(p: [f32; 3], dir: [f32; 3], color: u32, cd: f32, range: f32, inner: f32, outer: f32) -> LocalLight {
    LocalLight { position: Vec3::from(p), direction: Vec3::from(dir).normalize(), intensity_cd: hex(color) * cd, range_m: range,
        inner_cos: inner.to_radians().cos(), outer_cos: outer.to_radians().cos(), source_radius_m: 0.1, falloff: 2.0 }
}

fn room() -> BoxRoom {
    let wall = 0x4a4290;
    let wood = 0x6b4a33;
    let mut blocks = vec![
        block([-12.0, -0.2, -2.8], [12.0, 0.0, 6.0], 0x8a5a3a),
        block([-12.0, 0.0, -2.8], [12.0, 10.6, BACK], wall),
        block([-11.8, 0.0, -2.8], [-11.6, 10.6, 6.0], wall),
        block([11.6, 0.0, -2.8], [11.8, 10.6, 6.0], wall),
        block([-12.0, 10.4, -2.8], [12.0, 10.6, 6.0], 0x2e2a58),
        // Bookcase, display stand, up next cabinet, TV console, TV frame.
        block([-10.3, 0.0, BACK], [-8.9, 2.65, -1.6], wood),
        block([-8.7, 0.0, -2.4], [-6.5, 0.8, -1.5], 0x3a2c4a),
        block([-6.2, 0.0, -2.5], [-3.6, 2.0, -1.7], 0x1c1a26),
        block([-3.3, 0.0, -2.5], [3.3, 0.9, -1.4], 0x4a3326),
        block([-2.95, 2.3, BACK], [2.95, 5.6, -2.45], 0x101014),
        // Couch seat and back, speakers, rug-ish coffee table.
        block([4.9, 0.0, -2.3], [8.0, 0.85, -0.9], 0x5b3f99),
        block([5.0, 0.85, -2.5], [7.9, 1.55, -2.0], 0x4d3487),
        block([-4.7, 2.9, -2.5], [-3.7, 3.6, -2.0], 0x24222e),
        block([3.7, 2.9, -2.5], [4.7, 3.6, -2.0], 0x24222e),
        block([-1.4, 0.0, 0.6], [1.4, 0.4, 1.4], 0x2a2a30),
        // Door on the left, window frame.
        block([-11.6, 0.0, -2.0], [-11.4, 2.6, -0.6], 0x3c5a5a),
        block([-8.7, 4.6, BACK], [-6.3, 7.6, -2.5], 0x161430),
    ];
    // Shelves from the Godot layout: [x0, x1, top].
    for [x0, x1, top] in [[-11.6, -8.6, 4.7], [-7.4, -5.2, 5.9], [-2.6, 2.6, 6.55], [5.2, 7.7, 4.3], [8.6, 11.6, 3.15],
        [-11.6, -9.6, 7.3], [-6.4, -4.4, 8.2], [-1.2, 1.2, 9.2], [4.4, 6.4, 8.2], [9.0, 11.6, 6.6]] {
        blocks.push(block([x0, top - 0.1, BACK], [x1, top, -1.95], wood));
    }
    // Neon: tubes under the wordmark shelf, along the TV, a pink stripe by
    // the door, a green one under the couch shelf, the window's night glow.
    blocks.extend([
        tube([-2.5, 6.3, -2.55], [2.5, 6.36, -2.49], 0xb9a6ff, 260.0),
        tube([-3.05, 2.2, -2.55], [3.05, 2.25, -2.4], 0x7c5cff, 320.0),
        tube([-11.2, 0.3, -2.58], [-11.14, 3.8, -2.52], 0xff4f8b, 380.0),
        tube([5.3, 4.12, -2.55], [7.6, 4.16, -2.49], 0x3ddc97, 300.0),
        tube([-8.5, 4.8, -2.5], [-6.5, 7.4, -2.49], 0x2b3a8f, 4.0),
        tube([-11.0, 0.05, -2.0], [-10.5, 2.4, -1.98], 0x38d6ff, 22.0),
    ]);
    let mut lights = vec![
        point([-6.3, 3.0, -1.0], 0xffb454, 70.0, 6.5),
        point([0.0, 3.9, -1.2], 0x7c5cff, 140.0, 10.0),
        spot([0.0, 9.8, 1.5], [0.0, -1.0, -0.4], 0xffd7a0, 900.0, 14.0, 22.0, 40.0),
        point([0.0, 6.1, -2.0], 0xb9a6ff, 60.0, 3.2),
        point([8.6, 2.6, -1.6], 0xffb36b, 90.0, 5.5),
        point([3.6, 1.2, -1.0], 0xff8fb8, 45.0, 3.5),
        point([-10.6, 2.6, -1.2], 0x3ddc97, 40.0, 3.5),
        spot([-6.0, 0.15, 3.0], [-0.2, 0.05, -1.0], 0x38d6ff, 300.0, 9.0, 15.0, 35.0),
        spot([6.0, 0.15, 3.0], [0.2, 0.05, -1.0], 0x3ddc97, 300.0, 9.0, 15.0, 35.0),
        spot([-5.0, 9.8, 2.0], [0.15, -1.0, -0.1], 0xff4f8b, 500.0, 12.0, 18.0, 35.0),
        spot([5.0, 9.8, 2.0], [-0.15, -1.0, -0.1], 0x38d6ff, 500.0, 12.0, 18.0, 35.0),
        point([3.3, 1.0, 0.5], 0xff4f8b, 40.0, 3.0),
    ];
    // Fairy lights along the top of the back wall.
    let bulb = [0xffe2a8, 0xff8fb8, 0xa78bfa, 0x7fe3ff];
    for i in 0..13 {
        let x = -10.5 + i as f32 * 1.75;
        let y = 8.7 - 0.25 * ((i as f32 * 1.75 + 0.9) % 3.5 - 1.75).abs();
        lights.push(point([x, y, -2.3], bulb[i % 4], 6.0, 2.8));
    }
    BoxRoom { blocks, lights, sky: Vec3::ZERO }
}

/// A box with its uv mapped onto the top-left `[u, v]` of the layer, facing +z.
fn panel(scene: &mut Scene, center: Vec3, size: [f32; 2], uv: [f32; 2], material: u32) {
    let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
    let vertices: Vec<Vertex> = [(-hx, -hy, 0.0, uv[1]), (hx, -hy, uv[0], uv[1]), (hx, hy, uv[0], 0.0), (-hx, hy, 0.0, 0.0)]
        .into_iter().map(|(x, y, u, v)| Vertex { pos: (center + Vec3::new(x, y, 0.0)).to_array(), normal: [0.0, 0.0, 1.0], uv: [u, v] }).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &[0, 1, 2, 0, 2, 3], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
}

fn text(id: u64, s: &str, at: [f32; 2], size: f32, color: [f32; 4], font: FontFace, align: TextAlign, width: f32, bounds: [f32; 2]) -> TextRun {
    let c = |v: f32| (v * 255.0).round() as u8;
    TextRun { id, text: s.into(), position: at, bounds: [0.0, 0.0, bounds[0], bounds[1]], font_size: size,
        color: [c(color[0]), c(color[1]), c(color[2]), c(color[3])], font, align, width }
}

/// A 16x22 pixel character: hat, face, shirt, legs.
fn character(hat: u32, shirt: u32, skin: u32) -> Picture {
    let (w, h) = (16u32, 22u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut put = |x: u32, y: u32, c: u32| { let i = ((y * w + x) * 4) as usize; rgba[i..i + 4].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]); };
    for y in 0..22 { for x in 0..16 {
        let c = match (x, y) {
            (4..=11, 0..=2) | (3..=12, 3) => Some(hat),
            (4..=11, 4..=10) => Some(if (y == 6 || y == 7) && (x == 6 || x == 9) { 0x1a1420 } else if y == 9 && (7..=8).contains(&x) { 0x8a3a3a } else { skin }),
            (3..=12, 11..=16) => Some(if x == 3 || x == 12 { skin } else { shirt }),
            (5..=6 | 9..=10, 17..=20) => Some(0x2a2440),
            (4..=6 | 9..=11, 21) => Some(0x121018),
            _ => None,
        };
        if let Some(c) = c { put(x, y, c); }
    }}
    Picture::new(w, h, rgba).unwrap()
}

fn main() { if let Err(e) = run() { eprintln!("game room proof: {e}"); std::process::exit(1); } }

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let out = arg("--out").ok_or("--out room.png required")?;
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280), b.parse().unwrap_or(720)))).unwrap_or((1280u32, 720u32));
    let probes_on = !args.iter().any(|a| a == "--no-probes");
    let started = std::time::Instant::now();

    let mut room = room();
    // The open front stands in for Godot's flat ambient fill (3b3170): rays
    // that leave the room see this dim violet "sky". The TV glows into the bake.
    room.sky = hex(0x3b3170) * arg("--fill").and_then(|v| v.parse().ok()).unwrap_or(20.0);
    let mut lit_room = room.clone();
    lit_room.blocks.push(tube([-2.8, 2.38, -2.46], [2.8, 5.52, -2.44], 0x5fb8b8, 30.0));
    let mut scene = Scene::default();
    scene.sky = Some((4, 2, vec![[0.0, 0.0, 0.0, 1.0]; 8]));
    let cube = |scene: &mut Scene, m: u32| scene.add_mesh(&render::primitives::cube(m).mesh());
    for b in &room.blocks {
        let emissive = b.emission.max_element() > 0.0;
        let nits = b.emission.max_element().max(1.0);
        let tint = if emissive { (b.emission / nits).to_array() } else { b.albedo.to_array() };
        let m = scene.add_material(Material { tint, layer: -1, roughness: 0.8, flags: if emissive { EMISSIVE } else { 0 }, scale: nits, metal: 0.0 });
        let mesh = cube(&mut scene, m);
        scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(b.max - b.min, Quat::IDENTITY, (b.min + b.max) * 0.5));
    }
    // Fairy light bulbs.
    for lamp in room.lights.iter().skip(12) {
        let c = lamp.intensity_cd / lamp.intensity_cd.max_element();
        let m = scene.add_material(Material { tint: c.to_array(), layer: -1, roughness: 1.0, flags: EMISSIVE, scale: 900.0, metal: 0.0 });
        let mesh = scene.add_mesh(&render::primitives::sphere(m, 6, 8).mesh());
        scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(0.12), Quat::IDENTITY, lamp.position));
    }
    // Live screens: the TV, the up next cabinet and the QR plaque.
    let tv_layer = scene.add_screen_texture("tv");
    let next_layer = scene.add_screen_texture("up next");
    let plaque_layer = scene.add_screen_texture("plaque");
    let tv_m = scene.add_material(Material { tint: [1.0; 3], layer: tv_layer, roughness: 1.0, flags: EMISSIVE, scale: 45.0, metal: 0.0 });
    let next_m = scene.add_material(Material { tint: [1.0; 3], layer: next_layer, roughness: 1.0, flags: EMISSIVE, scale: 35.0, metal: 0.0 });
    let plaque_m = scene.add_material(Material { tint: [1.0; 3], layer: plaque_layer, roughness: 1.0, flags: EMISSIVE | TWO_SIDED, scale: 20.0, metal: 0.0 });
    panel(&mut scene, Vec3::new(0.0, 3.95, -2.44), [5.6, 3.15], [1.0, 0.5625], tv_m);
    panel(&mut scene, Vec3::new(-4.9, 1.3, -1.69), [2.4, 1.3], [1.0, 0.5417], next_m);
    panel(&mut scene, Vec3::new(10.1, 4.9, -2.52), [2.2, 2.6], [0.846, 1.0], plaque_m);
    let grid = Grid { origin: Vec3::new(-11.2, 0.4, -2.2), cell: 0.8, dims: [29, 13, 10] };
    scene.reserve_irradiance_volume(grid)?;

    let mut light = Lighting { sun_lux: 0.0, sky_lux: 0.0, fog_density: 0.0, ev100: arg("--ev").and_then(|v| v.parse().ok()).unwrap_or(2.0), ..Default::default() };
    light.fog_start = 1000.0;
    light.fog_end = 2000.0;
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    renderer.set_local_lights(&room.lights)?;
    let setup = started.elapsed();

    let bake_start = std::time::Instant::now();
    let probes = lit_room.bake(&grid, 96, 2)?;
    let bake = bake_start.elapsed();
    let probe = |p: [f32; 3]| irradiance_volume::sample(&grid, &probes, Vec3::from(p), Vec3::Z);
    println!("probe lux: wall left {:?} wall mid {:?}, direct there {:?}", probe([-9.0, 6.0, -2.4]), probe([-4.0, 5.0, -2.4]), sprites::irradiance(&room.lights, Vec3::new(-4.0, 5.0, -2.4), Vec3::Z, Vec3::ZERO));
    if probes_on { renderer.set_irradiance_volume(&gpu, &probes)?; }

    // Screens.
    let mut screen = Screen::new(&gpu)?;
    let s = SIZE as f32;
    let clip = [0.0, 0.0, s, s];
    let mut tv = HudDrawList::new([SIZE, SIZE]);
    let th = s * 0.5625;
    for (i, c) in [0x1d5d6b, 0x267a85, 0x2f8f94, 0x3aa59f, 0x4fb8a8].iter().enumerate() {
        tv.rect([0.0, th * 0.12 * i as f32, s, th * 0.12], srgb(*c), clip);
    }
    for i in 0..40 { let x = (i * 397 % 2048) as f32; let y = (i * 211 % 400) as f32 + 60.0; tv.rect([x, y, 6.0, 6.0], [1.0, 1.0, 1.0, 0.8], clip); }
    tv.polygon(&[[0.0, th * 0.62], [300.0, th * 0.55], [700.0, th * 0.63], [1100.0, th * 0.54], [1500.0, th * 0.62], [2048.0, th * 0.56], [2048.0, th * 0.78], [0.0, th * 0.78]], srgb(0x6ad1c0), clip);
    tv.rect([900.0, th * 0.52, 240.0, 40.0], srgb(0xffe2a8), clip);
    tv.rect([0.0, th * 0.78, s, th * 0.22], srgb(0x0f1a24), clip);
    tv.rect([60.0, 50.0, 330.0, 80.0], srgb(0x0b0b12), clip);
    tv.rect([60.0, 150.0, 1500.0, 14.0], [1.0, 1.0, 1.0, 0.25], clip);
    tv.rect([60.0, 150.0, 1100.0, 14.0], srgb(0xffe2a8), clip);
    for (i, c) in [0xff4f8b, 0xffb454, 0x3ddc97, 0x38d6ff, 0xa78bfa, 0xff8fb8].iter().enumerate() {
        tv.circle([1600.0 + i as f32 * 62.0, th * 0.89], 22.0, srgb(*c), clip);
    }
    tv.text.push(text(1, "LOADING 73%", [80.0, 62.0], 52.0, srgb(0xffb454), FontFace::Title, TextAlign::Left, 600.0, [s, s]));
    tv.text.push(text(2, "Space Racer", [70.0, th * 0.8], 120.0, srgb(0xf2f1f8), FontFace::Title, TextAlign::Left, 1400.0, [s, s]));
    tv.text.push(text(3, "Loading", [74.0, th * 0.93], 44.0, [0.8, 0.8, 0.9, 1.0], FontFace::Body, TextAlign::Left, 600.0, [s, s]));
    screen.draw(&gpu, &renderer, tv_layer, [0.0, 0.0, 0.0, 1.0], &tv)?;

    let mut next = HudDrawList::new([SIZE, SIZE]);
    let nh = s * 0.5417;
    next.rect([0.0, 0.0, s, 150.0], srgb(0x14131c), clip);
    next.rect([0.0, 150.0, s, nh * 0.55], srgb(0x2e6b3f), clip);
    next.rect([200.0, 150.0 + nh * 0.27, 1600.0, 8.0], [1.0, 1.0, 1.0, 0.6], clip);
    next.circle([1000.0, 150.0 + nh * 0.2], 30.0, [1.0, 1.0, 1.0, 1.0], clip);
    next.text.push(text(10, "UP NEXT", [40.0, 40.0], 80.0, [0.85, 0.85, 0.95, 1.0], FontFace::Title, TextAlign::Left, 1000.0, [s, s]));
    next.text.push(text(11, "Ball Kickers", [40.0, 170.0 + nh * 0.55], 130.0, srgb(0xf2f1f8), FontFace::Title, TextAlign::Left, 1900.0, [s, s]));
    next.text.push(text(12, "God Game", [40.0, 340.0 + nh * 0.55], 90.0, [0.75, 0.75, 0.85, 1.0], FontFace::Body, TextAlign::Left, 1900.0, [s, s]));
    screen.draw(&gpu, &renderer, next_layer, srgb(0x0b0b12), &next)?;

    // The QR plaque: 25 modules from a fixed pattern (looks like a code, scans as nothing).
    let mut plaque = HudDrawList::new([SIZE, SIZE]);
    let pw = s * 0.846;
    let (qx, qy, cell) = (pw / 2.0 - 25.0 * 26.0, 420.0, 52.0);
    plaque.rect([qx - 40.0, qy - 40.0, 25.0 * cell + 80.0, 25.0 * cell + 80.0], [1.0; 4], clip);
    let mut seed = 0x9e37_79b9u32;
    for y in 0..25 { for x in 0..25 {
        seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
        let finder = |a: i32, b: i32| { let (a, b) = (a.rem_euclid(25), b.rem_euclid(25)); let d = |v: i32, o: i32| (v - o).abs();
            [(3, 3), (21, 3), (3, 21)].iter().any(|&(fx, fy)| { let m = d(a, fx).max(d(b, fy)); m <= 3 && m != 2 }) };
        let near_finder = [(3, 3), (21, 3), (3, 21)].iter().any(|&(fx, fy): &(i32, i32)| (x as i32 - fx).abs() <= 4 && (y as i32 - fy).abs() <= 4);
        let on = if near_finder { finder(x, y) } else { seed & 1 == 1 };
        if on { plaque.rect([qx + x as f32 * cell, qy + y as f32 * cell, cell, cell], srgb(0x0b0b12), clip); }
    }}
    plaque.text.push(text(20, "Phone: scan to join", [0.0, 150.0], 90.0, [0.85, 0.85, 0.95, 1.0], FontFace::Body, TextAlign::Center, pw, [pw, s]));
    plaque.text.push(text(21, "KJX4", [0.0, qy + 25.0 * cell + 90.0], 220.0, [1.0; 4], FontFace::Title, TextAlign::Center, pw, [pw, s]));
    screen.draw(&gpu, &renderer, plaque_layer, srgb(0x0b0b12), &plaque)?;

    // Wordmark and tags as glowing canvas sprites; characters as lit pixel billboards.
    let mut sprite_list = vec![];
    let mut canvases = vec![];
    let mut tag = |gpu: &Gpu, id: u64, size: [u32; 2], runs: Vec<TextRun>| -> Result<(), String> {
        let mut canvas = Canvas::new(gpu, size)?;
        let mut list = HudDrawList::new(size);
        list.text = runs;
        canvas.draw(gpu, &list)?;
        renderer.set_sprite_texture(gpu, SpriteImage(id), canvas.texture(), ImageFilter::Linear);
        canvases.push(canvas);
        Ok(())
    };
    tag(&gpu, 100, [1536, 256], vec![text(100, "GAMENIGHT", [0.0, 10.0], 210.0, [1.0; 4], FontFace::Title, TextAlign::Center, 1536.0, [1536.0, 256.0])])?;
    tag(&gpu, 101, [1024, 96], vec![text(101, "COUCH MULTIPLAYER CLUB", [0.0, 12.0], 64.0, [1.0; 4], FontFace::Title, TextAlign::Center, 1024.0, [1024.0, 96.0])])?;
    sprite_list.push(Sprite { alpha_cut: 0.0, ..Sprite::fixed(Vec3::new(0.0, 7.6, -2.5), [8.4, 1.4], Vec3::X, Vec3::Y, SpriteImage(100), [0.0, 0.0, 1.0, 1.0], hex(0xfff1dc) * 450.0) });
    sprite_list.push(Sprite { alpha_cut: 0.0, ..Sprite::fixed(Vec3::new(0.0, 6.75, -2.5), [3.6, 0.34], Vec3::X, Vec3::Y, SpriteImage(101), [0.0, 0.0, 1.0, 1.0], hex(0xf2f1f8) * 75.0) });
    let people = [
        ("Joep", [-6.4, 0.0, -0.6], 0xa78bfa, 0xff8fb8, 0xf2d0b0),
        ("Pickle", [-9.6, 2.65, -1.95], 0x6fbf7f, 0x4a9a5a, 0xe8c09a),
        ("Mango", [0.6, 0.0, 0.0], 0xf2f1f8, 0x2a2a40, 0xf0c8a0),
        ("Waffle", [6.4, 0.85, -1.5], 0xff8a4c, 0xd8b0e8, 0xf0b890),
        ("Disco", [1.4, 6.55, -2.0], 0xffb454, 0xe0b040, 0xe8c8a0),
        ("Rocket", [9.4, 0.0, -0.4], 0x3a6ab0, 0xb0304a, 0x7a4a30),
    ];
    let names: Vec<TextRun> = people.iter().enumerate().map(|(i, p)| text(200 + i as u64, p.0, [0.0, 4.0], 44.0, [1.0; 4], FontFace::Body, TextAlign::Center, 256.0, [256.0, 64.0])).collect();
    let ambient_at = |p: Vec3| if probes_on { irradiance_volume::sample(&grid, &probes, p, Vec3::Z) } else { Vec3::ZERO };
    for (i, (name, feet, hat, shirt, skin)) in people.iter().enumerate() {
        let id = 10 + i as u64;
        renderer.set_sprite_image(&gpu, SpriteImage(id), &character(*hat, *shirt, *skin), ImageFilter::Nearest);
        let center = Vec3::from(*feet) + Vec3::new(0.0, 0.83, 0.0);
        let e = sprites::irradiance(&room.lights, center, Vec3::Z, ambient_at(center));
        sprite_list.push(Sprite::billboard(center, [1.2, 1.65], SpriteImage(id), [0.0, 0.0, 1.0, 1.0], 0.8 * e / std::f32::consts::PI));
        tag(&gpu, 300 + i as u64, [256, 64], vec![names[i].clone()])?;
        let color = [0x3ddc97, 0xb6f09c, 0xffb454, 0xffd7a0, 0xffd27a, 0xff8fb8][i];
        sprite_list.push(Sprite { alpha_cut: 0.0, ..Sprite::fixed(center + Vec3::new(0.0, 1.15, 0.1), [1.4, 0.35], Vec3::X, Vec3::Y, SpriteImage(300 + i as u64), [0.0, 0.0, 1.0, 1.0], hex(color) * 60.0) });
        let _ = name;
    }
    renderer.set_sprites(&gpu, &sprite_list)?;

    let camera = Camera { pos: Vec3::new(0.0, 4.7, 20.4), target: Vec3::new(0.0, 4.55, 0.0), fov_y: 36f32.to_radians(), near: 0.1 };
    let first = std::time::Instant::now();
    let mut rgba = vec![];
    for _ in 0..8 { rgba = renderer.render(&gpu, &camera, &light, w, h).0; }
    let frames = first.elapsed();
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    println!("setup {:.2}s, bake {} probes {:.2}s, 8 frames {:.2}s, lights {}, probes {}",
        setup.as_secs_f32(), probes.len(), bake.as_secs_f32(), frames.as_secs_f32(), room.lights.len(), if probes_on { "on" } else { "off" });
    Ok(())
}
