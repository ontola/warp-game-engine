//! Several buildings from different sources in one scene, side by side
//! along a street (the street runs along x, in front of them at -z): views
//! along the street, one from each front, and the largest rooms inside.
//!   cargo run --release -p architecture --example street -- OUT_DIR A.glb[:YAW_DEG] B.glb[:YAW_DEG] ...
//!     [--rays 1024] [--size 1280x720] [--rooms 2] [--site LAT,LON,NORTH_DEG] [--when DAY,UTC_HOURS]
//! IFC files: convert first with tools/ifc_to_glb.py.
use architecture::{sun::Site, House, EYE};
use glam::{Vec2, Vec3};
use render::{primitives, Camera, Gpu, Lighting, Renderer, Scene};
use std::path::Path;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let dir = args.get(1).ok_or("street OUT_DIR A.glb B.glb ...")?.clone();
    let files: Vec<&String> = args[2..].iter().take_while(|a| !a.starts_with("--")).collect();
    let rays = arg("--rays").and_then(|v| v.parse().ok()).unwrap_or(1024);
    let rooms = arg("--rooms").and_then(|v| v.parse().ok()).unwrap_or(2usize);
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((1280u32, 720u32));
    let nums = |k: &str| arg(k).map(|v| v.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>());
    let site = nums("--site").filter(|v| v.len() == 3).map_or(Site { latitude: 52.37, longitude: 4.9, north_deg: 180.0 }, |v| Site { latitude: v[0], longitude: v[1], north_deg: v[2] });
    let when = nums("--when").filter(|v| v.len() == 2).unwrap_or(vec![105.0, 9.5]);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut scene = Scene::default();
    scene.sky = Some(primitives::gradient_sky([0.3, 0.45, 0.7], [0.75, 0.8, 0.85], [0.35, 0.33, 0.3]));
    // Each house is loaded once where its file puts it to learn its width,
    // then placed: fronts on z = 0, 6 m between houses.
    let mut houses: Vec<(String, House)> = Vec::new();
    let mut x = 0.0;
    for spec in &files {
        let (file, yaw) = spec.rsplit_once(':').filter(|(_, y)| y.parse::<f32>().is_ok()).map_or((spec.as_str(), 0.0), |(f, y)| (f, y.parse::<f32>().unwrap().to_radians()));
        let mut probe = Scene::default();
        let size = { let h = House::load_at(&mut probe, Path::new(file), Some(Vec2::ZERO), yaw)?; h.max - h.min };
        let house = House::load_at(&mut scene, Path::new(file), Some(Vec2::new(x + size.x * 0.5, size.z * 0.5)), yaw)?;
        println!("{file}: {} triangles, {} elements, {} spaces, {:.1} x {:.1} x {:.1} m", house.triangles, house.elements.len(), house.spaces().count(), size.x, size.y, size.z);
        x += size.x + 6.0;
        let name = Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or("house").to_string();
        houses.push((name, house));
    }
    let (min, max) = houses.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, b), (_, h)| (a.min(h.min), b.max(h.max)));
    // One ground for the street, under the lowest slab so no floor meets it.
    let grade = houses.iter().map(|(_, h)| h.grade()).fold(0.0f32, f32::min);
    architecture::add_ground(&mut scene, (min + max) * 0.5, grade);
    let grid = architecture::probe_grid(min, max);
    scene.reserve_irradiance_volume(grid)?;
    println!("street {:.0} m, probes {:?} ({})", max.x - min.x, grid.dims, grid.count());

    let mut light = Lighting::default();
    light.fog_start = 1000.0;
    light.fog_end = 2000.0;
    light.fog_density = 0.0;
    light.sun_dir = site.sun_dir(when[0] as u32, when[1]);
    light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, speed_light: 0.0, speed_dark: 0.0, ..Default::default() });
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    let t = std::time::Instant::now();
    let occluders = render::room_bake::Occluders::from_scene(&scene, |_| true);
    let settings = render::room_bake::Settings { sky_rays: rays, ..Default::default() };
    let probes = render::room_bake::bake(&grid, &occluders, light.sun_dir, &settings)?;
    renderer.set_sky_sun_separated(&gpu, &probes, &render::room_bake::separation_planes(&grid, &occluders))?;
    println!("room bake {:.1} s", t.elapsed().as_secs_f64());

    let mid = (min + max) * 0.5;
    let len = max.x - min.x;
    let mut shots = vec![
        ("street-left".to_string(), Camera { pos: Vec3::new(min.x - 6.0, EYE, -9.0), target: Vec3::new(mid.x, EYE + 1.0, mid.z * 0.5), fov_y: 45f32.to_radians(), near: 0.1 }),
        ("street-right".to_string(), Camera { pos: Vec3::new(max.x + 6.0, EYE, -9.0), target: Vec3::new(mid.x, EYE + 1.0, mid.z * 0.5), fov_y: 45f32.to_radians(), near: 0.1 }),
        ("overview".to_string(), Camera { pos: Vec3::new(mid.x, len * 0.45, -len * 0.55), target: Vec3::new(mid.x, 0.0, mid.z), fov_y: 50f32.to_radians(), near: 0.1 }),
    ];
    for (name, house) in &houses {
        let c = house.center();
        let d = (house.max.x - house.min.x).max(house.max.y * 1.6) * 1.3;
        shots.push((format!("{name}-front"), Camera { pos: Vec3::new(c.x, EYE, house.min.z - d), target: Vec3::new(c.x, EYE, c.z), fov_y: 50f32.to_radians(), near: 0.1 }));
        let mut inside = house.room_shots();
        let area = |n: &str| house.spaces().find(|s| s.name == n).map_or(0.0, |s| (s.max.x - s.min.x) * (s.max.z - s.min.z));
        inside.sort_by(|a, b| area(&b.0).total_cmp(&area(&a.0)));
        shots.extend(inside.into_iter().take(rooms).map(|(n, cam)| (format!("{name}-{n}"), cam)));
    }
    for (i, (name, camera)) in shots.into_iter().enumerate() {
        renderer.reset_history();
        let mut rgba = vec![];
        for _ in 0..16 { rgba = renderer.render(&gpu, &camera, &light, w, h).0; }
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
        let file = format!("{dir}/{i:02}-{safe}.png");
        println!("{name}: EV {:.1} -> {file}", renderer.exposure_ev100(0).unwrap_or(light.ev100));
        image::save_buffer(&file, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    }
    Ok(())
}
