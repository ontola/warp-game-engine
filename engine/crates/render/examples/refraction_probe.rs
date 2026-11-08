//! Screen-space refraction, off against on, in one held frame each.
//! A striped wall and checkered floor behind a solid glass ball, a 6 cm
//! tilted slab and a 4 mm window pane. Run alone on the GPU:
//! refraction_probe --out FRESH_DIRECTORY
use glam::{Affine3A, Quat, Vec3};
use render::scene::{Material, Refraction, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::PathBuf;

const W: u32 = 640;
const H: u32 = 400;

fn camera() -> Camera {
    Camera { pos: Vec3::new(0.0, 1.6, 6.0), target: Vec3::new(0.0, 1.1, 0.0), fov_y: 50f32.to_radians(), near: 0.05 }
}

fn scene(refraction: bool) -> Scene {
    let mut scene = Scene::default();
    scene.graphics.refraction = refraction;
    let plain = |tint: [f32; 3], roughness| Material { tint, layer: -1, roughness, flags: TWO_SIDED, scale: 1.0, metal: 0.0 };
    let light = scene.add_material(plain([0.8, 0.78, 0.7], 0.8));
    let dark = scene.add_material(plain([0.05, 0.06, 0.08], 0.8));
    let red = scene.add_material(plain([0.7, 0.08, 0.05], 0.6));
    let cubes = [light, dark, red].map(|m| scene.add_mesh(&render::primitives::cube(m).mesh()));
    // Checkered floor and a striped wall: bending shows as broken lines.
    for x in -8..8 {
        for z in -6..4 {
            let at = Affine3A::from_scale_rotation_translation(Vec3::new(0.5, 0.02, 0.5), Quat::IDENTITY,
                Vec3::new(x as f32 * 0.5 + 0.25, -0.01, z as f32 * 0.5 + 0.25));
            scene.add_instance(cubes[((x + z) & 1) as usize], at);
        }
    }
    for x in -16..16 {
        let at = Affine3A::from_scale_rotation_translation(Vec3::new(0.25, 3.0, 0.05), Quat::IDENTITY,
            Vec3::new(x as f32 * 0.25 + 0.125, 1.5, -3.0));
        scene.add_instance(cubes[if x % 6 == 0 { 2 } else { (x & 1) as usize }], at);
    }
    let glass = |scene: &mut Scene, tint: [f32; 3], alpha: f32, refraction: Refraction| {
        let m = scene.add_material(Material { tint, layer: -1, roughness: 0.05, flags: TWO_SIDED, scale: 1.0, metal: 0.0 });
        scene.set_material_blend(m, alpha, true);
        scene.set_material_refraction(m, refraction);
        m
    };
    let ball = glass(&mut scene, [0.9, 0.95, 1.0], 0.04, Refraction { ior: 1.5, thickness: Refraction::SOLID });
    let ball_mesh = scene.add_mesh(&render::primitives::sphere(ball, 24, 32).mesh());
    scene.add_instance(ball_mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(0.7), Quat::IDENTITY, Vec3::new(-1.3, 0.75, 0.5)));
    let slab = glass(&mut scene, [0.75, 0.95, 0.85], 0.08, Refraction { ior: 1.5, thickness: 0.06 });
    let slab_mesh = scene.add_mesh(&render::primitives::quad(slab).mesh());
    scene.add_instance(slab_mesh, Affine3A::from_scale_rotation_translation(Vec3::new(1.2, 1.6, 1.0),
        Quat::from_rotation_y(0.9), Vec3::new(0.4, 0.9, 0.6)));
    let pane = glass(&mut scene, [0.5, 0.55, 0.6], 0.25, Refraction::WINDOW);
    let pane_mesh = scene.add_mesh(&render::primitives::quad(pane).mesh());
    scene.add_instance(pane_mesh, Affine3A::from_scale_rotation_translation(Vec3::new(1.0, 1.4, 1.0),
        Quat::from_rotation_y(-0.5), Vec3::new(1.9, 0.8, 0.8)));
    scene
}

fn main() { if let Err(e) = run() { eprintln!("refraction probe: {e}"); std::process::exit(1); } }

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(&args.windows(2).find(|w| w[0] == "--out").ok_or("--out FRESH_DIRECTORY required")?[1]);
    if out.exists() { return Err("output directory must be fresh".into()); }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut gpu = Gpu::new()?;
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let light = Lighting { taa: 0.0, fog_density: 0.0, ..Default::default() };
    let views = [ViewRect { cam: camera(), x: 0, y: 0, w: W, h: H }];
    // Split screen: two views share the renderer, each with its own copies.
    let split = [0u32, 1].map(|i| ViewRect { cam: camera(), x: i * W / 2, y: 0, w: W / 2, h: H });
    let mut images = Vec::new();
    let mut splits = Vec::new();
    for (name, on) in [("off", false), ("on", true)] {
        let renderer = Renderer::new(&gpu, &scene(on), &light);
        let mut rgba = Vec::new();
        // A few frames so temporal state (exposure, history) settles.
        for _ in 0..4 { rgba = renderer.try_render_views(&gpu, &views, &light, W, H, &[])?.0; }
        image::RgbaImage::from_raw(W, H, rgba.clone()).ok_or("image size")?
            .save(out.join(format!("{name}.png"))).map_err(|e| e.to_string())?;
        images.push(rgba);
        let mut both = Vec::new();
        for _ in 0..4 { both = renderer.try_render_views(&gpu, &split, &light, W, H, &[])?.0; }
        image::RgbaImage::from_raw(W, H, both.clone()).ok_or("image size")?
            .save(out.join(format!("{name}-split.png"))).map_err(|e| e.to_string())?;
        splits.push(both);
    }
    // Both halves see the same camera: refraction must match between them.
    let half = |rgba: &[u8], right: bool| -> Vec<u8> {
        rgba.chunks_exact(W as usize * 4)
            .flat_map(|row| row[if right { W as usize * 2 } else { 0 }..][..W as usize * 2].to_vec()).collect()
    };
    let left = half(&splits[1], false);
    let right = half(&splits[1], true);
    let mismatch = left.iter().zip(&right).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    let split_changed = half(&splits[0], false).iter().zip(&left).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    println!("split screen: {split_changed} channels changed by refraction, {mismatch} differ between the halves");
    if split_changed < 500 || mismatch > 200 { return Err("split-screen refraction differs per view".into()); }
    let changed = images[0].chunks_exact(4).zip(images[1].chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 8)).count();
    println!("refraction changed {changed} of {} pixels", W * H);
    if changed < 500 { return Err(format!("refraction changed only {changed} pixels")); }
    Ok(())
}
