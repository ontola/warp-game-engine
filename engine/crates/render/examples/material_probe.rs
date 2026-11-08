//! Material maps, weathering layers, compression and streaming on one small
//! scene. Engine examples may read Sideways' texture files.
//!
//! cargo run --release -p render --example material_probe -- --out DIR [--size 960x540]
//!
//! Writes `grid.png` (dry, wet, snow, moss), `maps.png` (material maps off
//! then on, dry) and `report.json` (renderer creation time, texture memory
//! format, streaming progress). `KERN_TEXTURE_COMPRESSION=0` keeps RGBA8;
//! `--stream` turns on `Scene::texture_streaming` and also saves the first
//! frame (`stream-first.png`) and every layer held at its streaming tail
//! (`stream-tail.png`), which must be soft but never black.
use glam::{Affine3A, Quat, Vec3};
use render::scene::{Material, TRIPLANAR};
use render::{Camera, Gpu, Lighting, MaterialLayers, Renderer, Scene, ViewRect, primitives};
use std::path::{Path, PathBuf};

fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets") }

fn scene(maps: bool, stream: bool) -> Result<(Scene, Vec<u32>), String> {
    let mut scene = Scene::default();
    scene.texture_streaming = stream;
    scene.sky = Some(primitives::gradient_sky([0.28, 0.42, 0.75], [0.75, 0.8, 0.88], [0.25, 0.23, 0.2]));
    let textures = root().join("textures");
    let mut layer = |name: &str| -> Result<i32, String> {
        let l = scene.add_texture(&textures.join(format!("{name}_albedo.jpg")))?;
        if !maps { scene.textures[l as usize].orm = None; }
        Ok(l)
    };
    let (rock, gravel, wall, ground) = (layer("mossy_rock")?, layer("gravel_ground_01")?, layer("stone_wall_04")?, layer("forest_ground_04")?);
    // The stump's packed occlusion/roughness/metalness map comes with its glTF.
    let stump = render::scene::Scene::add_texture(&mut scene, &root().join("props/tree_stump_01/textures/tree_stump_01_diff_1k.jpg"))?;
    if !maps { scene.textures[stump as usize].orm = None; }
    let m = |scene: &mut Scene, layer: i32, flags: u32, scale: f32, roughness: f32, metal: f32, tint: [f32; 3]|
        scene.add_material(Material { tint, layer, roughness, flags, scale, metal });
    let floor = m(&mut scene, gravel, TRIPLANAR, 2.5, 1.0, 0.0, [1.0; 3]);
    let boulder = m(&mut scene, rock, TRIPLANAR, 1.6, 1.0, 0.0, [1.0; 3]);
    let block = m(&mut scene, wall, 0, 1.0, 1.0, 0.0, [1.0; 3]);
    let soil = m(&mut scene, ground, TRIPLANAR, 1.2, 1.0, 0.0, [1.0; 3]);
    let wood = m(&mut scene, stump, 0, 1.0, 1.0, 0.0, [1.0; 3]);
    let steel = m(&mut scene, -1, 0, 1.0, 0.32, 1.0, [0.75, 0.74, 0.72]);
    // Stone and metal only take a film of water; soil and wood soak it up.
    scene.set_material_layers(boulder, MaterialLayers { porosity: 0.3, ..Default::default() });
    scene.set_material_layers(block, MaterialLayers { porosity: 0.2, ..Default::default() });
    scene.set_material_layers(steel, MaterialLayers { porosity: 0.0, ..Default::default() });
    scene.set_material_layers(soil, MaterialLayers { porosity: 1.0, ..Default::default() });
    scene.set_material_layers(wood, MaterialLayers { porosity: 0.8, ..Default::default() });
    let ground_mesh = scene.add_mesh(&primitives::cube(floor).mesh());
    scene.add_instance(ground_mesh, Affine3A::from_scale_rotation_translation(Vec3::new(40.0, 1.0, 40.0), Quat::IDENTITY, Vec3::new(0.0, -0.5, 0.0)));
    let sphere = scene.add_mesh(&primitives::sphere(boulder, 48, 96).mesh());
    scene.add_instance(sphere, Affine3A::from_scale_rotation_translation(Vec3::splat(1.1), Quat::IDENTITY, Vec3::new(-2.6, 1.1, 0.0)));
    let cube = scene.add_mesh(&primitives::cube(block).mesh());
    scene.add_instance(cube, Affine3A::from_scale_rotation_translation(Vec3::splat(1.6), Quat::from_rotation_y(0.5), Vec3::new(0.0, 0.8, -0.6)));
    let mound = scene.add_mesh(&primitives::sphere(soil, 32, 64).mesh());
    scene.add_instance(mound, Affine3A::from_scale_rotation_translation(Vec3::new(1.3, 0.6, 1.3), Quat::IDENTITY, Vec3::new(2.6, 0.0, 0.4)));
    let log = scene.add_mesh(&primitives::cylinder(wood, 0.5, 0.5, 48).mesh());
    scene.add_instance(log, Affine3A::from_scale_rotation_translation(Vec3::new(1.0, 1.1, 1.0), Quat::IDENTITY, Vec3::new(1.2, 0.55, 1.8)));
    let ball = scene.add_mesh(&primitives::sphere(steel, 32, 64).mesh());
    scene.add_instance(ball, Affine3A::from_scale_rotation_translation(Vec3::splat(0.55), Quat::IDENTITY, Vec3::new(-0.9, 0.55, 1.7)));
    Ok((scene, vec![floor, boulder, block, soil, wood]))
}

fn light() -> Lighting {
    Lighting { sun_dir: Vec3::new(0.55, 0.5, 0.35).normalize(), taa: 0.0, sharpen: 0.0, fog_density: 0.0,
        fog_start: 1e5, fog_end: 2e5, lod_pixels: 1.0, ..Default::default() }
}

fn view(w: u32, h: u32) -> Vec<ViewRect> {
    vec![ViewRect { cam: Camera { pos: Vec3::new(0.4, 2.6, 6.8), target: Vec3::new(0.0, 0.6, 0.2), fov_y: 50f32.to_radians(), near: 0.05 }, x: 0, y: 0, w, h }]
}

fn tile(grid: &mut image::RgbaImage, at: (u32, u32), rgba: &[u8], w: u32, h: u32) {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap();
    image::imageops::replace(grid, &img, at.0 as i64, at.1 as i64);
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let out = PathBuf::from(opt("--out").ok_or("--out DIR is required")?);
    let (w, h) = opt("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))).unwrap_or((960, 540));
    let stream = args.iter().any(|a| a == "--stream");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::new()?;
    let mut report = serde_json::json!({"adapter": gpu.info.name, "size": [w, h],
        "bc_supported": gpu.features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC)});
    let mut maps_grid = image::RgbaImage::new(w * 2, h);
    let mut weather_grid = image::RgbaImage::new(w * 2, h * 2);
    for maps in [false, true] {
        let (scene, moss_materials) = scene(maps, stream && maps)?;
        let start = std::time::Instant::now();
        let renderer = Renderer::new(&gpu, &scene, &light());
        let created_ms = start.elapsed().as_secs_f64() * 1e3;
        report[if maps { "maps_on" } else { "maps_off" }] = serde_json::json!({"renderer_ms": created_ms,
            "albedo_format": format!("{:?}", renderer.albedo_array().format()), "layers_streaming_at_start": renderer.textures_streaming()});
        if stream && maps {
            let (first, _) = renderer.try_render_views(&gpu, &view(w, h), &light(), w, h, &[])?;
            render::write_png(&out.join("stream-first.png"), w, h, &first)?;
            let start = std::time::Instant::now();
            renderer.finish_texture_streaming(&gpu);
            report["stream_finish_ms"] = serde_json::json!(start.elapsed().as_secs_f64() * 1e3);
            // What a layer looks like before its full mips arrive: every
            // layer held at the streaming tail (64 texels), no black mips.
            for (layer, t) in scene.textures.iter().enumerate() {
                let map = scene.textures[..layer].iter().filter(|t| t.orm.is_some()).count() as u32 + scene.textures.len() as u32;
                renderer.debug_texture_lod_floor(&gpu, layer as u32, 5.0, 4.0, t.orm.as_ref().map(|_| map));
            }
            let (held, _) = renderer.try_render_views(&gpu, &view(w, h), &light(), w, h, &[])?;
            render::write_png(&out.join("stream-tail.png"), w, h, &held)?;
            for (layer, t) in scene.textures.iter().enumerate() {
                let map = scene.textures[..layer].iter().filter(|t| t.orm.is_some()).count() as u32 + scene.textures.len() as u32;
                renderer.debug_texture_lod_floor(&gpu, layer as u32, 0.0, 0.0, t.orm.as_ref().map(|_| map));
            }
        }
        let (dry, _) = renderer.try_render_views(&gpu, &view(w, h), &light(), w, h, &[])?;
        tile(&mut maps_grid, (if maps { w } else { 0 }, 0), &dry, w, h);
        if !maps { continue; }
        tile(&mut weather_grid, (0, 0), &dry, w, h);
        let (wet, _) = renderer.try_render_views(&gpu, &view(w, h), &Lighting { wetness: 1.0, sun_lux: 30_000.0, sky_lux: 14_000.0, ev100: 12.6, ..light() }, w, h, &[])?;
        tile(&mut weather_grid, (w, 0), &wet, w, h);
        let (snow, _) = renderer.try_render_views(&gpu, &view(w, h), &Lighting { snow: 0.8, ..light() }, w, h, &[])?;
        tile(&mut weather_grid, (0, h), &snow, w, h);
        for &m in &moss_materials { renderer.set_material_layers(&gpu, m, MaterialLayers { moss: 1.0, ..Default::default() }); }
        let (moss, _) = renderer.try_render_views(&gpu, &view(w, h), &light(), w, h, &[])?;
        tile(&mut weather_grid, (w, h), &moss, w, h);
    }
    maps_grid.save(out.join("maps.png")).map_err(|e| e.to_string())?;
    weather_grid.save(out.join("grid.png")).map_err(|e| e.to_string())?;
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}
