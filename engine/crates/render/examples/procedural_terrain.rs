//! Procedural terrain look test: renders a few shots of one seed, from a
//! stream bank up to a view over the whole range, with only the function,
//! the patch pool and Sideways' ground textures.
//! Run: procedural_terrain --out DIR [--seed N] [--size 1920x1080] [--only NAME]
use glam::Vec3;
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use render::procedural_terrain::{self, ProceduralTerrain, Settings, TerrainView};
use render::scene::{Material, SURFACE, TERRAIN};
use render::foliage_assembly::MeadowBladeBudget;
use render::meadow_data::{MeadowField, MeadowGeometry};
use render::terrain_field;
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use std::path::PathBuf;

fn main() { if let Err(e) = run() { eprintln!("procedural terrain: {e}"); std::process::exit(1); } }

struct Shot { name: &'static str, pos: Vec3, target: Vec3, fov: f32 }

/// First point on a spiral around the origin that `want` accepts.
fn find(seed: u32, want: impl Fn(&terrain_field::Sample) -> bool) -> Option<(f32, f32)> {
    for k in 0..40000 {
        let r = 40.0 * (k as f32).sqrt();
        let a = k as f32 * 2.399963;
        let (x, z) = (r * a.cos(), r * a.sin());
        if want(&terrain_field::eval(x, z, 0.0, seed)) { return Some((x, z)); }
    }
    None
}

fn ground(x: f32, z: f32, seed: u32) -> f32 { terrain_field::height(x, z, seed) }

fn shots(seed: u32) -> Vec<Shot> {
    let mut v = Vec::new();
    let eye = |x: f32, z: f32, h: f32| Vec3::new(x, ground(x, z, seed) + h, z);
    if let Some((x, z)) = find(seed, |s| s.water > 0.3 && s.amp < 200.0) {
        let pos = eye(x - 9.0, z - 6.0, 1.7);
        v.push(Shot { name: "stream", pos, target: Vec3::new(x + 12.0, ground(x + 12.0, z + 8.0, seed), z + 8.0), fov: 60.0 });
    }
    // A beck: shallow water away from the wide river floors.
    if let Some((x, z)) = find(seed, |s| s.water > 0.15 && s.water < 0.4 && s.up < 0.97 && s.amp > 120.0) {
        let pos = eye(x - 7.0, z - 5.0, 1.7);
        v.push(Shot { name: "beck", pos, target: Vec3::new(x + 10.0, ground(x + 10.0, z + 7.0, seed), z + 7.0), fov: 60.0 });
    }
    if let Some((x, z)) = find(seed, |s| s.rock > 0.9 && s.up < 0.8) {
        let pos = eye(x - 30.0, z - 30.0, 1.7);
        v.push(Shot { name: "rock", pos, target: Vec3::new(x, ground(x, z, seed) + 4.0, z), fov: 60.0 });
        let pos = eye(x - 2.5, z - 2.5, 1.2);
        v.push(Shot { name: "rock-close", pos, target: Vec3::new(x, ground(x, z, seed), z), fov: 60.0 });
    }
    if let Some((x, z)) = find(seed, |s| s.rock < 0.1 && s.amp < 150.0 && s.up > 0.97 && s.wet < 0.01) {
        let pos = eye(x, z, 1.7);
        v.push(Shot { name: "hills", pos, target: pos + Vec3::new(100.0, -4.0, 60.0), fov: 60.0 });
        let pos = eye(x, z, 0.6);
        v.push(Shot { name: "ground-close", pos, target: pos + Vec3::new(2.0, -1.2, 1.0), fov: 60.0 });
    }
    // Mountains: a view along a high valley, and crags from below.
    if let Some((x, z)) = find(seed, |s| s.amp > 550.0 && s.up > 0.95 && s.water == 0.0 && s.rock < 0.2) {
        let pos = eye(x, z, 2.0);
        let t = Vec3::new(x + 600.0, ground(x + 600.0, z + 350.0, seed) + 60.0, z + 350.0);
        v.push(Shot { name: "mountains", pos, target: t, fov: 60.0 });
    }
    if let Some((x, z)) = find(seed, |s| s.amp > 400.0 && s.rock > 0.8 && s.up < 0.6) {
        let pos = eye(x - 90.0, z - 60.0, 2.0);
        v.push(Shot { name: "crags", pos, target: Vec3::new(x, ground(x, z, seed) + 10.0, z), fov: 60.0 });
    }
    let pos = eye(0.0, 0.0, 40.0);
    v.push(Shot { name: "down", pos, target: pos + Vec3::new(0.01, -1.0, 0.0), fov: 60.0 });
    let pos = eye(0.0, 0.0, 350.0);
    v.push(Shot { name: "valley", pos, target: pos + Vec3::new(1000.0, -260.0, 700.0), fov: 55.0 });
    let pos = eye(0.0, 0.0, 2600.0);
    v.push(Shot { name: "aerial", pos, target: pos + Vec3::new(4000.0, -2200.0, 3000.0), fov: 55.0 });
    v
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let out = PathBuf::from(arg("--out").ok_or("--out DIR")?);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let seed: u32 = arg("--seed").map(|s| s.parse().map_err(|_| "--seed N")).transpose()?.unwrap_or(1);
    let (w, h) = arg("--size").map(|s| {
        let (a, b) = s.split_once('x').ok_or("--size WxH")?;
        Ok::<_, String>((a.parse().map_err(|_| "--size")?, b.parse().map_err(|_| "--size")?))
    }).transpose()?.unwrap_or((1920u32, 1080u32));
    let only = arg("--only");

    let grass = args.iter().any(|a| a == "--grass");
    let capacity = arg("--capacity").and_then(|s| s.parse().ok()).unwrap_or(16384);
    let cube = args.iter().any(|a| a == "--cube");
    let (scene, pool, mut light) = make_scene(seed, capacity, cube, None)?;
    if let Some(d) = arg("--debug") { light.debug = d.parse().map_err(|_| "--debug N")?; }
    if args.iter().any(|a| a == "--no-fog") { light.fog_density = 0.0; }
    eprintln!("fog density {} height {}", light.fog_density, light.fog_height_m);

    let gpu = Gpu::new()?;
    let mut world = Some((Renderer::new(&gpu, &scene, &light), ProceduralTerrain::new(pool, Settings::default())));
    for shot in shots(seed) {
        if only.as_deref().is_some_and(|o| o != shot.name) { continue; }
        // Grass needs its field around this camera, so each shot gets a scene.
        if grass {
            let (scene, pool, _) = make_scene(seed, capacity, cube, Some(meadow(seed, shot.pos)?))?;
            world = Some((Renderer::new(&gpu, &scene, &light), ProceduralTerrain::new(pool, Settings::default())));
        }
        let (renderer, terrain) = world.as_mut().unwrap();
        render_shot(&gpu, renderer, terrain, &light, &shot, (w, h), &out, seed, args.iter().any(|a| a == "--dump"))?;
    }
    Ok(())
}

/// Windswept meadow roots on a 0.5 m grid of the terrain function, 300 m
/// around `centre`. Rock, scree and water are holes.
fn meadow(seed: u32, centre: Vec3) -> Result<MeadowField, String> {
    let (cell, n) = (0.5f32, 1200u32);
    let origin = [(centre.x / cell).floor() * cell - n as f32 * cell * 0.5, (centre.z / cell).floor() * cell - n as f32 * cell * 0.5];
    let mut samples = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let s = terrain_field::eval(origin[0] + i as f32 * cell, origin[1] + j as f32 * cell, 0.0, seed);
            let bare = s.rock > 0.5 || s.scree > 0.8 || s.water > 0.02;
            samples.push(if bare { [0.0; 4] } else { [s.pos.y, 100.0, 1.0, 0.0] });
        }
    }
    let field = MeadowField {
        origin, cell, width: n, height: n, samples, seed, growth: 1.0, dry_season: 0.0, radius: 250.0, fade_start: 200.0,
        geometry: MeadowGeometry::cook_windswept(seed as u64, 0.0, MeadowBladeBudget::Blades120)?,
    };
    field.validate()?;
    Ok(field)
}

fn make_scene(seed: u32, capacity: u32, cube: bool, meadow: Option<MeadowField>) -> Result<(Scene, procedural_terrain::TerrainSpec, Lighting), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets/textures");
    let mut scene = Scene::default();
    scene.meadow = meadow;
    // Layers in the order terrain_surface.wgsl reads them.
    let t0 = scene.add_texture(&root.join("grass_ground_albedo.jpg"))?;
    scene.add_texture(&root.join("forest_leaves_02_albedo.jpg"))?; // moor
    scene.add_texture(&root.join("forest_ground_04_albedo.jpg"))?; // soil
    scene.add_texture(&root.join("rock_face_03_albedo.jpg"))?;
    scene.add_texture_with_normal_encoding(&root.join("gravel_ground_01_albedo.jpg"), render::scene::NormalEncoding::SrgbTransferred)?;
    scene.add_texture(&root.join("mossy_rock_albedo.jpg"))?;
    let material = scene.add_material(Material { tint: [1.0, 1.0, 1.0], layer: t0, roughness: 0.9, flags: TERRAIN | SURFACE, scale: 4.0, metal: 0.0 });
    if cube {
        let m = scene.add_mesh(&render::primitives::cube(material).mesh());
        scene.add_instance(m, glam::Affine3A::from_scale_rotation_translation(Vec3::splat(100.0), glam::Quat::IDENTITY, Vec3::new(300.0, 0.0, 200.0)));
    }
    let pool = procedural_terrain::reserve(&mut scene, capacity, material, seed);

    let mut light = Lighting::default();
    let mut cache = AtmosphereCache::default();
    let sky = cache.get_or_bake(&Atmosphere::default(), SolarState::from_degrees(28.0, 135.0)?, BakeQuality::default())?;
    sky.apply(&mut scene, &mut light);
    light.fog_density = 0.00003;
    light.fog_end = 60000.0;
    Ok((scene, pool, light))
}

fn render_shot(gpu: &Gpu, renderer: &Renderer, terrain: &mut ProceduralTerrain, light: &Lighting, shot: &Shot, (w, h): (u32, u32),
    out: &std::path::Path, seed: u32, dump: bool) -> Result<(), String> {
    {
        let cam = Camera { pos: shot.pos, target: shot.target, fov_y: shot.fov.to_radians(), near: 0.05 };
        let started = std::time::Instant::now();
        let mut rgba = Vec::new();
        let mut stats = Default::default();
        for frame in 0..6 {
            let s = terrain.update(gpu, renderer, &[TerrainView::new(&cam, w, h)])?;
            if frame == 0 { stats = s; }
            let (img, fs) = renderer.render(gpu, &cam, light, w, h);
            rgba = img;
            if frame == 5 { eprintln!("clusters {} triangles {} broadphase {}", fs.clusters, fs.triangles, fs.instance_broadphase); }
        }
        if dump {
            // GPU vertices against the CPU mirror (interior points only).
            let (mut bad, mut total) = (0, 0);
            for slot in 0..stats.patches as u32 {
                let (v, _, _) = renderer.procedural_terrain_debug(gpu, slot).unwrap();
                let s = v[8] - v[0];
                if !(s > 0.0) { continue; }
                for j in 1..8 {
                    for i in 1..8 {
                        let k = (j * 9 + i) * 8;
                        let (x0, z0) = (v[0] + i as f32 * s, v[2] + j as f32 * s);
                        let c = terrain_field::eval(x0, z0, s, seed);
                        let up = (v[k + 7] as u32 & 255) as f32 / 255.0;
                        let rock = (v[k + 6] as u32 & 255) as f32 / 255.0;
                        total += 1;
                        if (c.up - up).abs() > 0.1 || (c.rock.clamp(0.0, 1.0) - rock).abs() > 0.2 || (c.pos.y - v[k + 1]).abs() > 1.0 {
                            if bad < 8 { eprintln!("mismatch at ({x0}, {z0}) cut {s}: cpu up {} rock {} y {}; gpu up {up} rock {rock} y {}", c.up, c.rock, c.pos.y, v[k + 1]); }
                            bad += 1;
                        }
                    }
                }
            }
            eprintln!("{bad} of {total} vertices differ");
        }
        let path = out.join(format!("{}.png", shot.name));
        image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        eprintln!("{}: {:?} at {:.0?}; {} patches ({} built, overflow {}), finest level {}", shot.name, shot.pos, started.elapsed(),
            stats.patches, stats.built, stats.overflow, stats.finest_level);
    }
    Ok(())
}
