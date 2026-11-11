//! Look scene for indoor light: one closed room with a door on a sunlit
//! field at noon. Renders an outside view, two inside views and prints what
//! the exposure meter reads, so indoor/outdoor contrast can be compared.
//!
//! Run: interior_light <variant> <out-dir> [--fixed EV]
//! Variants: none (no indoor data), vis (sky visibility only), diffuse (+ one
//! sky bounce), sun (+ one sun bounce), sep (+ separation planes), room
//! (bake_room: multi-bounce sky and sun, separation planes).
use glam::{Affine3A, Quat, Vec3};
use render::irradiance_volume::Grid;
use render::scene::Material;
use render::{Camera, Gpu, Lighting, Renderer, Scene};

fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
    let (a, b) = (Vec3::from(min), Vec3::from(max));
    Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let variant = args.get(1).cloned().unwrap_or("sep".into());
    let dir = args.get(2).cloned().unwrap_or(".".into());
    let fixed: Option<f32> = args.windows(2).find(|w| w[0] == "--fixed").and_then(|w| w[1].parse().ok());

    let mut scene = Scene::default();
    let mut mat = |scene: &mut Scene, tint: [f32; 3], rough: f32| {
        let m = scene.add_material(Material { tint, layer: -1, roughness: rough, flags: 0, scale: 1.0, metal: 0.0 });
        scene.add_mesh(&render::primitives::cube(m).mesh())
    };
    let plaster = mat(&mut scene, [0.55, 0.52, 0.48], 0.85);
    let wood = mat(&mut scene, [0.30, 0.18, 0.10], 0.35);
    let grass = mat(&mut scene, [0.12, 0.25, 0.06], 0.9);
    let cloth = mat(&mut scene, [0.45, 0.10, 0.08], 0.9);
    scene.add_instance(grass, bx([-150.0, -0.2, -150.0], [150.0, 0.0, 150.0]));
    // 8 x 3 x 8 m inside, 0.3 m walls, a 1.2 x 2.2 m door in the +z wall.
    let shell = vec![
        bx([-4.3, 0.0, -4.3], [4.3, 3.0, -4.0]),
        bx([-4.3, 0.0, -4.0], [-4.0, 3.0, 4.0]),
        bx([4.0, 0.0, -4.0], [4.3, 3.0, 4.0]),
        bx([-4.3, 0.0, 4.0], [-0.6, 3.0, 4.3]),
        bx([0.6, 0.0, 4.0], [4.3, 3.0, 4.3]),
        bx([-0.6, 2.2, 4.0], [0.6, 3.0, 4.3]),
        bx([-4.3, 3.0, -4.3], [4.3, 3.3, 4.3]),
    ];
    let floor = bx([-4.0, 0.0, -4.0], [4.0, 0.02, 4.0]);
    let table = vec![bx([-3.0, 0.7, -3.2], [-1.2, 0.76, -2.2]), bx([-2.95, 0.0, -3.15], [-1.25, 0.7, -2.25])];
    let couch = vec![bx([1.0, 0.0, -3.9], [3.6, 0.45, -3.0]), bx([1.0, 0.45, -3.9], [3.6, 1.0, -3.6])];
    for w in &shell { scene.add_instance(plaster, *w); }
    scene.add_instance(wood, floor);
    for t in &table { scene.add_instance(wood, *t); }
    for c in &couch { scene.add_instance(cloth, *c); }
    let mut occluders: Vec<Affine3A> = shell.clone();
    occluders.push(bx([-150.0, -0.2, -150.0], [150.0, 0.0, 150.0]));
    occluders.extend(table.iter().chain(couch.iter()).copied());

    let grid = Grid { origin: Vec3::new(-5.75, -0.25, -5.75), cell: 0.5, dims: [24, 9, 24] };
    let indoor = variant != "none";
    if indoor { scene.reserve_irradiance_volume(grid).unwrap(); }

    let mut light = Lighting::default();
    light.fog_start = 1000.0; light.fog_end = 2000.0; light.fog_density = 0.0;
    light.ev100 = fixed.unwrap_or(14.0);
    if args.iter().any(|a| a == "--no-sun") { light.sun_lux = 0.0; }
    if args.iter().any(|a| a == "--no-flare") { light.lens_flare = 0.0; }
    if args.iter().any(|a| a == "--no-bloom") { light.bloom_boost = 0.0; }
    if fixed.is_none() {
        light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, speed_light: 0.0, speed_dark: 0.0, ..Default::default() });
    }
    let gpu = Gpu::new().unwrap();
    let renderer = Renderer::new(&gpu, &scene, &light);
    let reflectance = 0.5;
    let rays: usize = args.windows(2).find(|w| w[0] == "--rays").and_then(|w| w[1].parse().ok()).unwrap_or(96);
    let t0 = std::time::Instant::now();
    match variant.as_str() {
        "none" => {}
        "vis" => renderer.set_sky_occlusion(&gpu, &render::sky_occlusion::bake(&grid, &occluders, rays).unwrap()).unwrap(),
        "diffuse" => renderer.set_sky_diffuse(&gpu, &render::sky_occlusion::bake_diffuse(&grid, &occluders, rays, reflectance).unwrap()).unwrap(),
        "sun" | "sep" => {
            let sky = render::sky_occlusion::bake_diffuse(&grid, &occluders, rays, reflectance).unwrap();
            let probes = render::sky_occlusion::bake_sun(&grid, &occluders, &sky, rays, reflectance, light.sun_dir).unwrap();
            if variant == "sep" {
                let planes = render::sky_occlusion::bake_separation_planes(&grid, &occluders).unwrap();
                renderer.set_sky_sun_separated(&gpu, &probes, &planes).unwrap();
            } else {
                renderer.set_sky_sun(&gpu, &probes).unwrap();
            }
        }
        "room" => {
            // Bake from the scene's own triangles: no hand-made occluders.
            let bounces = args.windows(2).find(|w| w[0] == "--bounces").and_then(|w| w[1].parse().ok()).unwrap_or(3);
            let occ = render::room_bake::Occluders::from_scene(&scene, |_| true);
            let settings = render::room_bake::Settings { sky_rays: rays, bounces, reflectance, ..Default::default() };
            let probes = render::room_bake::bake(&grid, &occ, light.sun_dir, &settings).unwrap();
            renderer.set_sky_sun_separated(&gpu, &probes, &render::room_bake::separation_planes(&grid, &occ)).unwrap();
        }
        v => panic!("unknown variant {v}"),
    }
    println!("{variant}: bake {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    let views = [
        ("outside", Camera { pos: Vec3::new(4.0, 1.7, 14.0), target: Vec3::new(0.0, 1.2, 4.0), fov_y: 60f32.to_radians(), near: 0.05 }),
        ("room", Camera { pos: Vec3::new(2.8, 1.6, 2.8), target: Vec3::new(-2.0, 0.8, -3.0), fov_y: 75f32.to_radians(), near: 0.05 }),
        ("corner", Camera { pos: Vec3::new(0.0, 1.4, 0.5), target: Vec3::new(4.0, 3.0, -4.0), fov_y: 60f32.to_radians(), near: 0.05 }),
        ("door", Camera { pos: Vec3::new(1.8, 1.6, -3.2), target: Vec3::new(-0.2, 1.1, 4.3), fov_y: 75f32.to_radians(), near: 0.05 }),
    ];
    let (w, h) = (640, 360);
    for (name, camera) in views {
        renderer.reset_history();
        let mut rgba = vec![];
        for _ in 0..16 { rgba = renderer.render(&gpu, &camera, &light, w, h).0; }
        println!("{variant} {name}: exposure EV {:.2}", renderer.exposure_ev100(0).unwrap_or(light.ev100));
        image::save_buffer(format!("{dir}/{variant}-{name}.png"), &rgba, w, h, image::ColorType::Rgba8).unwrap();
    }
}
