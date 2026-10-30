//! Depth-bounded emissive column: inside, outside, axial rays and occlusion.
use glam::{Affine3A, Quat, Vec3};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::fx::{AdditiveColumn, Effects};
fn main() -> Result<(), String> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).ok_or("column_volume_gpu OUT")?);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut gpu = Gpu::new()?;
    // Image assertions do not require optional encoder timestamps.
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY);
    let mut scene = Scene::default();
    let mat = scene.add_material(render::scene::Material { tint: [0.25; 3], layer: -1, roughness: 1.0, flags: 0, scale: 1.0, metal: 0.0 });
    let mesh = scene.add_mesh(&render::primitives::cube(mat).mesh());
    for (size, pos) in [(Vec3::new(40.0, 0.2, 40.0), Vec3::new(0.0, -0.1, 0.0)),
        (Vec3::new(40.0, 20.0, 0.2), Vec3::new(0.0, 5.0, -7.0)),
        (Vec3::new(4.0, 4.0, 0.2), Vec3::new(0.0, 2.0, -2.0))] {
        scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(size, Quat::IDENTITY, pos));
    }
    let light = Lighting { taa: 0.0, ev100: 10.0, sun_lux: 800.0, sky_lux: 200.0, gi: 0.0, shadow_strength: 0.0, contact_occlusion: 0.0, reflection_occlusion: 0.0, fog_density: 0.0, time: 1.0, ..Default::default() };
    let renderer = Renderer::new(&gpu, &scene, &light);
    let column = AdditiveColumn { position: Vec3::new(0.0, -1.0, 0.0), height: 30.0, radius: [5.0; 2], life: 10.0, opacity: [0.5; 2], color: [Vec3::new(0.38, 0.78, 1.0); 2], radiance: [2500.0; 2], ..Default::default() };
    let cam = |pos, direction| Camera { pos, target: pos + direction, fov_y: 65f32.to_radians(), near: 0.1 };
    let cameras = [
        ("inside-wall", cam(Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Z)),
        ("outside", cam(Vec3::new(0.0, 2.0, 12.0), Vec3::NEG_Z)),
        ("occluded", cam(Vec3::new(0.0, 2.0, -10.0), Vec3::Z)),
        ("inside-up", cam(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.001, 1.0, 0.0))),
        ("inside-down", cam(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.001, -1.0, 0.0))),
        ("edge-inside", cam(Vec3::new(4.99, 2.0, 0.0), Vec3::NEG_Z)),
        ("edge-outside", cam(Vec3::new(5.01, 2.0, 0.0), Vec3::NEG_Z)),
    ];
    let (w, h) = (640, 360);
    let mut edge = Vec::new();
    for (name, camera) in cameras {
        renderer.set_effects(&gpu, 1.0, Some(&[]))?;
        let (without, _) = renderer.render(&gpu, &camera, &light, w, h);
        // Resend the effect after the explicit empty baseline.
        let mut live = Effects::default();
        live.column(column)?;
        live.flush(&gpu, &renderer, 1.0)?;
        let (with, _) = renderer.render(&gpu, &camera, &light, w, h);
        let pixel = ((h / 2 * w + w / 2) * 4) as usize;
        let delta: u64 = with[pixel..pixel + 3].iter().zip(&without[pixel..pixel + 3]).map(|(a,b)| a.abs_diff(*b) as u64).sum();
        if name == "occluded" { assert!(with == without, "volume leaked through foreground wall"); }
        if name.starts_with("inside") { assert!(delta > 20, "{name}: missing luminous air in front of nearby geometry: {delta}"); }
        if name.starts_with("edge") { edge.push(with.clone()); }
        render::write_png(&out.join(format!("{name}.png")), w, h, &with)?;
        render::write_png(&out.join(format!("{name}-off.png")), w, h, &without)?;
        println!("{name}: center light delta {delta}");
    }
    let mean = edge[0].iter().zip(&edge[1]).map(|(a,b)| a.abs_diff(*b) as f64).sum::<f64>() / edge[0].len() as f64;
    assert!(mean < 2.0, "crossing the edge popped: mean channel delta {mean}");
    let views: Vec<_> = render::split_rects(4, w, h).iter().zip(cameras.iter()).map(|(r, (_, cam))| ViewRect { cam: *cam, x:r.0, y:r.1, w:r.2, h:r.3 }).collect();
    let (split, _) = renderer.try_render_views(&gpu, &views, &light, w, h, &[])?;
    for v in &views {
        let (single, _) = renderer.render(&gpu, &v.cam, &light, v.w, v.h);
        for y in 0..v.h {
            let dst = (((v.y+y)*w+v.x)*4) as usize;
            let src = (y*v.w*4) as usize;
            assert_eq!(&split[dst..dst+(v.w*4) as usize], &single[src..src+(v.w*4) as usize]);
        }
    }
    render::write_png(&out.join("four-views.png"), w, h, &split)?;
    println!("PASS: interior, opaque occlusion, axial rays, edge continuity ({mean:.3}), per-view rays");

    Ok(())
}
