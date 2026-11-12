//! Render a glTF house headless: four views from outside and one per room,
//! with the room bake so rooms get their own (darker) light.
//!   cargo run --release -p architecture --example render_house -- HOUSE.glb OUT_DIR [--rays 2048] [--size 1280x720]
use architecture::House;
use render::{primitives, Gpu, Lighting, Renderer, Scene};
use std::path::Path;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let path = args.get(1).ok_or("render_house HOUSE.glb OUT_DIR")?;
    let dir = args.get(2).cloned().unwrap_or_else(|| ".".into());
    let rays = arg("--rays").and_then(|v| v.parse().ok()).unwrap_or(2048);
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((1280u32, 720u32));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut scene = Scene::default();
    scene.sky = Some(primitives::gradient_sky([0.3, 0.45, 0.7], [0.75, 0.8, 0.85], [0.35, 0.33, 0.3]));
    let house = House::load(&mut scene, Path::new(path))?;
    house.add_ground(&mut scene);
    let grid = house.probe_grid();
    scene.reserve_irradiance_volume(grid)?;
    println!("{path}: {} triangles, {} elements, {} spaces, probes {:?}", house.model.triangles, house.elements.len(), house.spaces().count(), grid.dims);

    let mut light = Lighting::default();
    light.fog_start = 1000.0;
    light.fog_end = 2000.0;
    light.fog_density = 0.0;
    // Late morning, sun in front and to the left (the front faces -z).
    let (az, el) = (200f32.to_radians(), 35f32.to_radians());
    light.sun_dir = glam::Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos());
    light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, speed_light: 0.0, speed_dark: 0.0, ..Default::default() });

    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    let t = std::time::Instant::now();
    let occluders = render::room_bake::Occluders::from_scene(&scene, |_| true);
    let settings = render::room_bake::Settings { sky_rays: rays, ..Default::default() };
    let probes = render::room_bake::bake(&grid, &occluders, light.sun_dir, &settings)?;
    renderer.set_sky_sun_separated(&gpu, &probes, &render::room_bake::separation_planes(&grid, &occluders))?;
    println!("room bake {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);

    for (i, (name, camera)) in house.exterior_shots().into_iter().chain(house.room_shots()).enumerate() {
        renderer.reset_history();
        let mut rgba = vec![];
        for _ in 0..16 { rgba = renderer.render(&gpu, &camera, &light, w, h).0; }
        let file = format!("{dir}/{i:02}-{}.png", name.replace(' ', "-").to_lowercase());
        println!("{name}: EV {:.1} -> {file}", renderer.exposure_ev100(0).unwrap_or(light.ev100));
        image::save_buffer(&file, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    }
    Ok(())
}
