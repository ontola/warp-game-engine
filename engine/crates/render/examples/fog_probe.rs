//! Real-renderer regression for directional fog and conservative camera culling.
//! Run only in an exclusive GPU window: fog_probe --out FRESH_DIRECTORY.
//! Native pixels, no temporal accumulation. This is not a gameplay benchmark.

use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, TWO_SIDED};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::{Path, PathBuf};

const WIDTH: u32 = 257;
const HEIGHT: u32 = 193;

fn scene(visible: bool) -> Scene {
    let mut scene = Scene::default();
    // Directional HDR input deliberately differs across the sky. A global
    // horizon-average fog colour cannot pass this test by coincidence.
    let (w, h) = (128, 64);
    let pixels = (0..h).flat_map(|y| (0..w).map(move |x| {
        let u = (x as f32 + 0.5) / w as f32;
        let v = (y as f32 + 0.5) / h as f32;
        [0.1 + u * 0.8, 0.15 + v * 0.45, 0.2 + (1.0 - u) * 0.7, 1.0]
    })).collect();
    scene.sky = Some((w, h, pixels));
    let material = scene.add_material(Material { tint: [0.36, 0.065, 0.025], layer: -1,
        roughness: 1.0, flags: TWO_SIDED, scale: 1.0, metal: 0.0 });
    let points = [(0.0, -14.0), (-30.0, -12.0), (-22.0, 4.0), (-10.0, 16.0),
        (0.0, 8.0), (12.0, 20.0), (20.0, 5.0), (30.0, -12.0)];
    let vertices: Vec<_> = points.into_iter().map(|(x, y)| Vertex {
        pos: [x, y, 0.0], normal: Vec3::Z.to_array(), uv: [0.0; 2],
    }).collect();
    let indices: Vec<_> = (1..7u32).flat_map(|i| [0, i + 1, i]).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &indices, material }]));
    // Retain valid scene buffers for the background reference, but put its
    // geometry entirely behind both cameras. GI/shadow interactions are off.
    scene.add_instance(mesh, Affine3A::from_translation(Vec3::new(0.0, 0.0, if visible { -80.0 } else { 80.0 })));
    scene
}

fn camera(z: f32) -> Camera {
    let pos = Vec3::new(0.0, 0.0, z);
    Camera { pos, target: pos + Vec3::NEG_Z, fov_y: 64f32.to_radians(), near: 0.1 }
}
fn panel(pixels: &[u8], output_width: u32, x: u32) -> Vec<u8> {
    (0..HEIGHT).flat_map(|y| {
        let start = ((y * output_width + x) * 4) as usize;
        pixels[start..start + WIDTH as usize * 4].iter().copied()
    }).collect()
}
fn mismatches(a: &[u8], b: &[u8]) -> (usize, u8) {
    assert_eq!(a.len(), b.len());
    let mut count = 0; let mut maximum = 0;
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let difference = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap();
        count += usize::from(difference != 0); maximum = maximum.max(difference);
    }
    (count, maximum)
}
fn record(name: &str, stats: &FrameStats) -> String {
    format!("{{\"name\":\"{name}\",\"retained_clusters\":{},\"retained_triangles\":{},\"submitted_vertices\":{},\"shadow_triangles\":{}}}",
        stats.clusters, stats.triangles, stats.submitted_vertices, stats.shadow_triangles)
}
fn save(out: &Path, name: &str, width: u32, pixels: &[u8]) -> Result<(), String> {
    render::write_png(&out.join(format!("{name}.png")), width, HEIGHT, pixels)
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(args.windows(2).find(|p| p[0] == "--out").ok_or("use --out FRESH_DIRECTORY")?[1].clone());
    if out.exists() { return Err("output directory must be new".into()); }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::new()?;
    let mut light = Lighting { sun_lux: 0.0, sky_lux: 3000.0, ev100: 12.0,
        fog_start: 10.0, fog_end: 30.0, fog_density: 0.0, fog_culling: false,
        taa: 0.0, sharpen: 0.0, gi: 0.0, contact_occlusion: 0.0, reflection_occlusion: 0.0,
        shadow_strength: 0.0, near_shadows: 0.0, lod_pixels: 0.0, ..Lighting::default() };
    let background_scene = scene(false); let mountain_scene = scene(true);
    let background = Renderer::new(&gpu, &background_scene, &light);
    let mountain = Renderer::new(&gpu, &mountain_scene, &light);
    let far = camera(0.0); let near = camera(-60.0);
    let pick = [(WIDTH / 2, HEIGHT / 2)];
    let mut records = Vec::new(); let mut failures = Vec::new();

    let (sky_far, _) = background.render(&gpu, &far, &light, WIDTH, HEIGHT);
    let (full_unculled, full_stats) = mountain.render_picking(&gpu, &far, &light, WIDTH, HEIGHT, &pick);
    records.push(record("full-fog-unculled", &full_stats));
    let full_diff = mismatches(&sky_far, &full_unculled);
    if full_diff.0 != 0 { failures.push(format!("full fog vs background: {} differing pixels, max {}", full_diff.0, full_diff.1)); }
    if full_stats.picks.first().copied().flatten().is_none() || full_stats.triangles == 0 {
        failures.push("unculled fully fogged mountain must really cover the centre pixel".into());
    }
    light.fog_culling = true;
    let (full_culled, culled_stats) = mountain.render_picking(&gpu, &far, &light, WIDTH, HEIGHT, &pick);
    records.push(record("full-fog-culled", &culled_stats));
    if full_culled != full_unculled { failures.push("full fog culling changed native pixels".into()); }
    if culled_stats.triangles >= full_stats.triangles || culled_stats.submitted_vertices >= full_stats.submitted_vertices {
        failures.push("full fog culling did not reduce retained triangles/submitted vertices".into());
    }
    if culled_stats.picks.first().copied().flatten().is_some() { failures.push("fully culled mountain still writes centre depth/visibility".into()); }
    save(&out, "background-far", WIDTH, &sky_far)?;
    save(&out, "full-fog-unculled", WIDTH, &full_unculled)?;
    save(&out, "full-fog-culled", WIDTH, &full_culled)?;

    light.fog_culling = false;
    let (sky_near, _) = background.render(&gpu, &near, &light, WIDTH, HEIGHT);
    let (partial_unculled, partial_stats) = mountain.render_picking(&gpu, &near, &light, WIDTH, HEIGHT, &pick);
    records.push(record("partial-fog-unculled", &partial_stats));
    let partial_diff = mismatches(&sky_near, &partial_unculled);
    if partial_diff.0 < 100 || partial_diff.1 < 8 { failures.push("partial fog erased the mountain contrast".into()); }
    light.fog_culling = true;
    let (partial_culled, partial_cull_stats) = mountain.render_picking(&gpu, &near, &light, WIDTH, HEIGHT, &pick);
    records.push(record("partial-fog-culled", &partial_cull_stats));
    if partial_culled != partial_unculled { failures.push("fog culling removed partly visible mountain geometry".into()); }
    if partial_cull_stats.picks.first().copied().flatten().is_none() { failures.push("partly visible mountain lost centre visibility".into()); }
    save(&out, "background-near", WIDTH, &sky_near)?;
    save(&out, "partial-fog-unculled", WIDTH, &partial_unculled)?;
    save(&out, "partial-fog-culled", WIDTH, &partial_culled)?;

    // One source mountain is fully obscured for one camera and still visible
    // for another. Per-view culling must not remove shared scene instances.
    let views = [ViewRect { cam: far, x: 0, y: 0, w: WIDTH, h: HEIGHT },
        ViewRect { cam: near, x: WIDTH, y: 0, w: WIDTH, h: HEIGHT }];
    let split_picks = [pick[0], (WIDTH + WIDTH / 2, HEIGHT / 2)];
    let (split, split_stats) = mountain.render_views(&gpu, &views, &light, WIDTH * 2, HEIGHT, &split_picks);
    records.push(record("two-cameras-independent-distance", &split_stats));
    if panel(&split, WIDTH * 2, 0) != full_culled || panel(&split, WIDTH * 2, WIDTH) != partial_culled {
        failures.push("split cameras differ from their corresponding single-camera results".into());
    }
    if split_stats.picks[0].is_some() || split_stats.picks[1].is_none() {
        failures.push("split views did not cull the mountain independently".into());
    }
    save(&out, "split-full-and-partial", WIDTH * 2, &split)?;
    let report = format!("{{\"schema\":1,\"scope\":\"native directional fog image convergence and retained geometry; not draw-call or gameplay performance claims\",\"output_panel\":[{WIDTH},{HEIGHT}],\"full_fog_rgb_different_pixels\":{},\"full_fog_max_channel_error\":{},\"partial_fog_rgb_different_pixels\":{},\"partial_fog_max_channel_error\":{},\"cases\":[{}],\"failures\":{:?},\"passed\":{}}}",
        full_diff.0, full_diff.1, partial_diff.0, partial_diff.1, records.join(","), failures, failures.is_empty());
    std::fs::write(out.join("report.json"), report).map_err(|e| e.to_string())?;
    println!("directional fog: full {} changed pixels, partial {}, {} failures", full_diff.0, partial_diff.0, failures.len());
    if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
}

fn main() { if let Err(error) = run() { eprintln!("{error}"); std::process::exit(1); } }
