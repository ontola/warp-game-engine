//! Render a glTF house headless: four views from outside and one per room,
//! with the room bake so rooms get their own (darker) light.
//!   cargo run --release -p architecture --example render_house -- HOUSE.glb OUT_DIR
//!     [--rays 2048] [--size 1280x720] [--site LAT,LON,NORTH_DEG] [--when DAY_OF_YEAR,UTC_HOURS] [--night]
//! Defaults: Amsterdam with the plan's up to the south (front in the sun), 15 April 09:30 UTC.
//! Every glTF camera in the file is an extra shot named after its node. Without `--when`, a
//! camera with `extras.viewTime` is lit by the sun of that time at `--site` (the room bake is
//! made again for each distinct sun). `--night` puts the sun below the horizon and darkens the
//! sky, so the file's lamps (KHR_lights_punctual) carry the picture.
use architecture::House;
use glam::Vec3;
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

    let night = args.iter().any(|a| a == "--night");
    let mut scene = Scene::default();
    scene.sky = Some(if night { primitives::gradient_sky([0.02, 0.03, 0.07], [0.05, 0.06, 0.1], [0.02, 0.02, 0.02]) }
        else { primitives::gradient_sky([0.3, 0.45, 0.7], [0.75, 0.8, 0.85], [0.35, 0.33, 0.3]) });
    let house = House::load(&mut scene, Path::new(path))?;
    scene.graphics.clustered_lights = !house.lights.is_empty();
    house.add_ground(&mut scene);
    let grid = house.probe_grid();
    scene.reserve_irradiance_volume(grid)?;
    println!("{path}: {} triangles, {} elements, {} spaces, {} lights, {} cameras, probes {:?}", house.triangles, house.elements.len(), house.spaces().count(), house.lights.len(), house.cameras.len(), grid.dims);

    let mut light = Lighting::default();
    light.fog_start = 1000.0;
    light.fog_end = 2000.0;
    light.fog_density = 0.0;
    let nums = |k: &str| arg(k).map(|v| v.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>());
    let site = nums("--site").filter(|v| v.len() == 3).map_or(architecture::sun::Site { latitude: 52.37, longitude: 4.9, north_deg: 180.0 },
        |v| architecture::sun::Site { latitude: v[0], longitude: v[1], north_deg: v[2] });
    let when = nums("--when").filter(|v| v.len() == 2);
    let day = when.as_ref().map_or(105.0, |w| w[0]);
    // At night the sun sits well below the horizon (UTC midnight) and gives no light.
    light.sun_dir = if night { site.sun_dir(day as u32, 0.0) } else { when.as_ref().map_or_else(|| site.sun_dir(105, 9.5), |w| site.sun_dir(w[0] as u32, w[1])) };
    if night { light.sun_lux = 0.0; light.sky_lux = 1.0; }
    println!("sun {:.2?} (elevation {:.0} deg)", light.sun_dir, light.sun_dir.y.asin().to_degrees());
    light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, speed_light: 0.0, speed_dark: 0.0, ..Default::default() });

    // Shots: outside, rooms, then the file's cameras. A camera's own sun (its
    // viewTime) applies only when neither --when nor --night fixes the sun.
    let default_sun = light.sun_dir;
    let mut shots: Vec<(String, render::Camera, Vec3)> = house.exterior_shots().into_iter().chain(house.room_shots()).map(|(n, c)| (n, c, default_sun)).collect();
    for v in &house.cameras {
        let sun = match v.view_time_ms {
            Some(ms) if when.is_none() && !night => { let (d, h) = architecture::sun::day_and_hours(ms); site.sun_dir(d, h) }
            _ => default_sun,
        };
        println!("camera {}: viewTime {:?} sun elevation {:.0} deg", v.name, v.view_time_ms, sun.y.asin().to_degrees());
        shots.push((v.name.clone(), v.camera, sun));
    }

    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    if !house.lights.is_empty() {
        renderer.set_clustered_lights(&house.clustered_lights())?;
    }
    let occluders = render::room_bake::Occluders::from_scene(&scene, |_| true);
    let settings = render::room_bake::Settings { sky_rays: rays, ..Default::default() };

    // One room bake per distinct sun, shots in order within each.
    let mut suns: Vec<Vec3> = vec![];
    for (_, _, s) in &shots { if !suns.iter().any(|k| k.distance(*s) < 0.005) { suns.push(*s); } }
    for sun in suns {
        let t = std::time::Instant::now();
        let probes = render::room_bake::bake(&grid, &occluders, sun, &settings)?;
        renderer.set_sky_sun_separated(&gpu, &probes, &render::room_bake::separation_planes(&grid, &occluders))?;
        println!("room bake for sun {:.2?}: {:.0} ms", sun, t.elapsed().as_secs_f64() * 1000.0);
        light.sun_dir = sun;
        for (i, (name, camera, _)) in shots.iter().enumerate().filter(|(_, s)| s.2.distance(sun) < 0.005) {
            renderer.reset_history();
            let mut rgba = vec![];
            for _ in 0..16 { rgba = renderer.render(&gpu, camera, &light, w, h).0; }
            let file = format!("{dir}/{i:02}-{}.png", name.replace(' ', "-").to_lowercase());
            println!("{name}: EV {:.1} -> {file}", renderer.exposure_ev100(0).unwrap_or(light.ev100));
            image::save_buffer(&file, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
