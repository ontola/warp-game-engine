//! Exclusive-GPU regression for the shared static source-alpha sun map.
//! KERN_FAR_SUN_SHADOW=1 far_sun_visibility --out FRESH_DIRECTORY
//! No window, gameplay-performance claim, or automatic application restart.
use glam::{Affine3A, Mat4, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{ALPHA_TEST, FOLIAGE, Material, SOLID, TWO_SIDED};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::{Path, PathBuf};

const W: u32 = 640;
const H: u32 = 384;

fn quad(scene: &mut Scene, positions: [[f32; 3]; 4], normal: Vec3, alpha: bool) -> usize {
    // A broad opaque middle strip and transparent sides. Both probe points
    // lie well inside their respective regions, beyond PCF/mip edge support.
    let layer = if alpha {
        let image = image::RgbaImage::from_fn(128, 128, |x, _| {
            image::Rgba([255, 255, 255, if (52..76).contains(&x) { 255 } else { 0 }])
        });
        scene.add_image("far-sun alpha strip", image)
    } else {
        -1
    };
    let material = scene.add_material(Material {
        tint: [0.5; 3],
        layer,
        roughness: 1.0,
        flags: TWO_SIDED | if alpha { ALPHA_TEST } else { 0 },
        scale: 1.0,
        metal: 0.0,
    });
    let uv = [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
    let vertices: Vec<_> = positions
        .into_iter()
        .zip(uv)
        .map(|(pos, uv)| Vertex {
            pos,
            normal: normal.to_array(),
            uv,
        })
        .collect();
    scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 1, 2, 0, 2, 3],
        material,
    }]))
}

fn pixel(camera: Camera, world: Vec3) -> (u32, u32) {
    let vp = Mat4::perspective_infinite_reverse_rh(camera.fov_y, W as f32 / H as f32, camera.near)
        * Mat4::look_at_rh(camera.pos, camera.target, Vec3::Y);
    let p = vp.project_point3(world);
    (
        ((p.x * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - p.y * 0.5) * H as f32) as u32,
    )
}

fn region(pixels: &[u8], point: (u32, u32)) -> f64 {
    let mut sum = 0u32;
    for y in point.1 - 2..=point.1 + 2 {
        for x in point.0 - 2..=point.0 + 2 {
            sum += pixels[((y * W + x) * 4) as usize] as u32;
        }
    }
    sum as f64 / 25.0
}

fn record(rows: &mut Vec<String>, name: &str, stats: &FrameStats, blocked: f64, gap: f64) {
    let times = stats
        .gpu_ms
        .iter()
        .map(|(name, ms)| format!("[{name:?},{ms}]"))
        .collect::<Vec<_>>()
        .join(",");
    rows.push(format!(
        "{{\"name\":{name:?},\"blocked_probe\":{blocked},\"gap_probe\":{gap},\"cache_updated\":{},\"extent_m\":{},\"texel_m\":{},\"far_clusters\":{},\"far_triangles\":{},\"far_submitted_vertices\":{},\"gpu_ms\":[{times}]}}",
        stats.far_sun_cache_updated, stats.far_sun_extent_m, stats.far_sun_texel_m,
        stats.far_sun_clusters, stats.far_sun_triangles, stats.far_sun_submitted_vertices,
    ));
}

fn cache_check(failures: &mut Vec<String>, name: &str, stats: &FrameStats, updated: bool) {
    if stats.far_sun_cache_updated != updated {
        failures.push(format!(
            "{name}: cache_updated={}, expected {updated}",
            stats.far_sun_cache_updated
        ));
    }
    if updated && stats.far_sun_submitted_vertices == 0 {
        failures.push(format!("{name}: cold cache submitted no source geometry"));
    }
    if !updated && stats.far_sun_submitted_vertices != 0 {
        failures.push(format!(
            "{name}: held cache submitted {} vertices",
            stats.far_sun_submitted_vertices
        ));
    }
}

fn capture(
    renderer: &Renderer,
    gpu: &Gpu,
    camera: Camera,
    light: &Lighting,
    out: &Path,
    name: &str,
    rows: &mut Vec<String>,
) -> Result<(Vec<u8>, FrameStats, f64, f64), String> {
    let (pixels, stats) = renderer.try_render_views(
        gpu,
        &[ViewRect {
            cam: camera,
            x: 0,
            y: 0,
            w: W,
            h: H,
        }],
        light,
        W,
        H,
        &[],
    )?;
    render::write_png(&out.join(format!("{name}.png")), W, H, &pixels)?;
    let blocked = region(&pixels, pixel(camera, Vec3::ZERO));
    let gap = region(&pixels, pixel(camera, Vec3::Z * 8.0));
    record(rows, name, &stats, blocked, gap);
    Ok((pixels, stats, blocked, gap))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("far sun fixture: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    if std::env::var("KERN_FAR_SUN_SHADOW").as_deref() != Ok("1") {
        return Err("set KERN_FAR_SUN_SHADOW=1 explicitly for this fixture".into());
    }
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(
        &args
            .windows(2)
            .find(|a| a[0] == "--out")
            .ok_or("--out FRESH_DIRECTORY required")?[1],
    );
    if out.exists() {
        return Err("output directory must be fresh".into());
    }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut scene = Scene::default();
    scene.sky = Some((4, 2, vec![[0.0, 0.0, 0.0, 1.0]; 8]));
    scene.sky_physical = true;
    let floor = quad(
        &mut scene,
        [
            [-22.0, 0.0, 22.0],
            [22.0, 0.0, 22.0],
            [22.0, 0.0, -22.0],
            [-22.0, 0.0, -22.0],
        ],
        Vec3::Y,
        false,
    );
    scene.add_instance(floor, Affine3A::IDENTITY);
    let wall_points = [
        [600.0, 16.0, -20.0],
        [600.0, 30.0, -20.0],
        [600.0, 30.0, 20.0],
        [600.0, 16.0, 20.0],
    ];
    let wall = quad(&mut scene, wall_points, Vec3::NEG_X, true);
    scene.add_instance(wall, Affine3A::IDENTITY);
    scene.instances[1].flags |= FOLIAGE;
    let car = quad(&mut scene, wall_points, Vec3::NEG_X, false);
    scene.add_instance(car, Affine3A::from_translation(Vec3::Z * 80.0));
    scene.instances[2].flags |= SOLID;

    // The floor lies >180m from the camera, outside the ordinary 96m map.
    // At 2deg the source ray reaches y20.95m at x600m: inside the alpha wall.
    let camera = Camera {
        pos: Vec3::new(0.0, 80.0, 180.0),
        target: Vec3::ZERO,
        fov_y: 16f32.to_radians(),
        near: 0.1,
    };
    let sun = |degrees: f32| Vec3::new(degrees.to_radians().cos(), degrees.to_radians().sin(), 0.0);
    let mut light = Lighting {
        sun_dir: sun(2.0),
        sun_lux: 80_000.0,
        sky_lux: 0.0,
        ev100: 11.0,
        gi: 0.0,
        shadow_strength: 1.0,
        near_shadows: 1.0,
        contact_occlusion: 0.0,
        reflection_occlusion: 0.0,
        fog_density: 0.0,
        fog_culling: false,
        taa: 0.0,
        sharpen: 0.0,
        lod_pixels: 0.0,
        debug: 9.0,
        ..Default::default()
    };
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    let (reference, initial, blocked, gap) =
        capture(&renderer, &gpu, camera, &light, &out, "initial", &mut rows)?;
    cache_check(&mut failures, "initial", &initial, true);
    if initial.far_sun_extent_m < 1024.0 {
        failures.push("far extent below1024m".into());
    }
    if blocked > 3.0 || gap < 225.0 {
        failures.push(format!(
            "source alpha did not preserve blocked/gap probes: {blocked}/{gap}"
        ));
    }
    let (held, stats, _, _) = capture(&renderer, &gpu, camera, &light, &out, "held", &mut rows)?;
    cache_check(&mut failures, "held", &stats, false);
    if held != reference {
        failures.push("held frame differs from initial image".into());
    }
    let moved_camera = Camera {
        pos: camera.pos + Vec3::X * 20.0,
        ..camera
    };
    let (_, stats, _, _) = capture(
        &renderer,
        &gpu,
        moved_camera,
        &light,
        &out,
        "camera-moved",
        &mut rows,
    )?;
    cache_check(&mut failures, "camera-moved", &stats, false);
    renderer.update_instances(&gpu, 1, &scene.instances[1..2])?;
    let (_, stats, _, _) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "identical-static-upload",
        &mut rows,
    )?;
    cache_check(&mut failures, "identical-static-upload", &stats, false);

    renderer.set_wind(
        &gpu,
        &render::wind::WindState {
            time_seconds: 3.0,
            strength: 0.6,
            ..Default::default()
        },
    )?;
    let (wind, stats, _, _) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "wind-rest-cache",
        &mut rows,
    )?;
    cache_check(&mut failures, "wind-rest-cache", &stats, false);
    if wind != reference {
        failures.push("far rest-pose shadow changed with wind".into());
    }
    renderer.set_wind(&gpu, &Default::default())?;

    // Put an excluded opaque car-like owner across both light rays. Its
    // dynamic near maps are outside the receiver; it must not enter this cache.
    scene.update_instance(2, car, Affine3A::IDENTITY);
    renderer.update_instances(&gpu, 2, &scene.instances[2..3])?;
    let (_, stats, _, dynamic_gap) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "dynamic-update",
        &mut rows,
    )?;
    cache_check(&mut failures, "dynamic-update", &stats, false);
    if dynamic_gap < 225.0 {
        failures.push(format!(
            "excluded dynamic owner entered far map: {dynamic_gap}"
        ));
    }

    scene.update_instance(1, wall, Affine3A::from_translation(Vec3::Z * 80.0));
    renderer.update_instances(&gpu, 1, &scene.instances[1..2])?;
    let (_, stats, open, _) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "static-moved",
        &mut rows,
    )?;
    cache_check(&mut failures, "static-moved", &stats, true);
    if open < 225.0 {
        failures.push(format!("static caster update left stale shadow: {open}"));
    }

    scene.update_instance(1, wall, Affine3A::IDENTITY);
    renderer.update_instances(&gpu, 1, &scene.instances[1..2])?;
    scene.update_instance(2, car, Affine3A::from_translation(Vec3::Z * 80.0));
    renderer.update_instances(&gpu, 2, &scene.instances[2..3])?;
    let views: Vec<_> = (0..4)
        .map(|i| ViewRect {
            cam: camera,
            x: i % 2 * W,
            y: i / 2 * H,
            w: W,
            h: H,
        })
        .collect();
    let (split, stats) = renderer.try_render_views(&gpu, &views, &light, W * 2, H * 2, &[])?;
    cache_check(&mut failures, "four-view-cold", &stats, true);
    if stats.far_sun_submitted_vertices != initial.far_sun_submitted_vertices {
        failures.push(format!(
            "shared four-view submission differs: {} vs {}",
            stats.far_sun_submitted_vertices, initial.far_sun_submitted_vertices
        ));
    }
    let mut split_differences = Vec::new();
    for i in 0..4 {
        let mut changed = 0usize;
        let mut maximum = 0u8;
        for y in 0..H {
            for x in 0..W {
                let a = (((y + i / 2 * H) * W * 2 + x + i % 2 * W) * 4) as usize;
                let b = ((y * W + x) * 4) as usize;
                let delta = (0..4)
                    .map(|c| split[a + c].abs_diff(reference[b + c]))
                    .max()
                    .unwrap();
                changed += usize::from(delta > 0);
                maximum = maximum.max(delta);
            }
        }
        split_differences.push(format!(
            "{{\"view\":{i},\"different_pixels\":{changed},\"max_byte_delta\":{maximum}}}"
        ));
        if changed != 0 {
            failures.push(format!(
                "four-view {i} has {changed} changed pixels; maximum {maximum}"
            ));
        }
    }
    let first_view: Vec<u8> = (0..H)
        .flat_map(|y| {
            let start = (y * W * 2 * 4) as usize;
            split[start..start + (W * 4) as usize].iter().copied()
        })
        .collect();
    record(
        &mut rows,
        "four-view-cold",
        &stats,
        region(&first_view, pixel(camera, Vec3::ZERO)),
        region(&first_view, pixel(camera, Vec3::Z * 8.0)),
    );
    render::write_png(&out.join("four-view.png"), W * 2, H * 2, &split)?;
    let (held_split, stats) = renderer.try_render_views(&gpu, &views, &light, W * 2, H * 2, &[])?;
    cache_check(&mut failures, "four-view-held", &stats, false);
    if held_split != split {
        failures.push("held four-view frame differs from cold frame".into());
    }
    record(
        &mut rows,
        "four-view-held",
        &stats,
        region(&first_view, pixel(camera, Vec3::ZERO)),
        region(&first_view, pixel(camera, Vec3::Z * 8.0)),
    );

    light.sun_dir = sun(6.0);
    let (_, stats, sun_open, _) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "sun-changed",
        &mut rows,
    )?;
    cache_check(&mut failures, "sun-changed", &stats, true);
    if sun_open < 225.0 {
        failures.push(format!("sun update left stale shadow: {sun_open}"));
    }
    light.alpha_cut = 0.7;
    let (_, stats, _, _) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "alpha-changed",
        &mut rows,
    )?;
    cache_check(&mut failures, "alpha-changed", &stats, true);
    light.lod_pixels = 1.0;
    let (_, stats, _, _) = capture(
        &renderer,
        &gpu,
        camera,
        &light,
        &out,
        "lod-changed",
        &mut rows,
    )?;
    cache_check(&mut failures, "lod-changed", &stats, true);

    let report = format!(
        "{{\"schema\":1,\"scope\":\"isolated native640x384 far-map regression, not forest optics or gameplay FPS\",\"camera\":{:?},\"receiver\":[0,0,0],\"caster_sunward_m\":600,\"far_wind_policy\":\"static rest pose; dynamic classes remain in near maps\",\"rows\":[{}],\"split_differences\":[{}],\"failures\":{:?},\"passed\":{}}}",
        camera.pos.to_array(),
        rows.join(","),
        split_differences.join(","),
        failures,
        failures.is_empty()
    );
    std::fs::write(out.join("report.json"), report).map_err(|e| e.to_string())?;
    if failures.is_empty() {
        println!("far sun fixture PASS");
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
