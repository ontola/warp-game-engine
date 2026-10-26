//! Real-renderer terrain-horizon regression for local solar fog scattering.
//! Exclusive GPU only: sun_fog_visibility --out FRESH_DIRECTORY.
//! Optional separate baseline process: KERN_CAMERA_SUN_OCCLUSION=0.
//! Native pixels; no TAA/DLSS. This is not a gameplay performance benchmark.
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Heightfield, Material, TWO_SIDED};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::{Path, PathBuf};

const W: u32 = 321;
const H: u32 = 193;
const GRID: u32 = 129;
const PITCH: f32 = 2.0;
const ORIGIN: [f32; 2] = [-64.0, -128.0];
// Black metal suppresses normal material response but Schlick F90 remains
// at grazing incidence. A shaded camera can see an actually sunlit surface;
// allow only one quantized output level while requiring exact visibility=0.
const BLOCKED_RGB_RESIDUAL: u8 = 1;

fn terrain_height(x: f32, z: f32) -> f32 {
    let along = ((x - 12.0) / 6.0).clamp(0.0, 1.0)
        * ((32.0 - x) / 6.0).clamp(0.0, 1.0);
    let across = ((40.0 - z.abs()) / 8.0).clamp(0.0, 1.0);
    16.0 * along * across
}

fn scene() -> Scene {
    let mut scene = Scene::default();
    scene.sky_physical = true;
    // Fixed sun-disc-free sky: toggling sun lux must not change diffuse haze.
    scene.sky = Some((16, 8, vec![[400.0, 450.0, 550.0, 1.0]; 128]));
    // Black metal suppresses diffuse and normal-incidence Fresnel response.
    // Grazing F90 is retained by the real BRDF; see BLOCKED_RGB_RESIDUAL.
    let material = scene.add_material(Material { tint: [0.0; 3], layer: -1,
        roughness: 1.0, metal: 1.0, flags: TWO_SIDED, scale: 1.0 });
    let mut vertices = Vec::with_capacity((GRID * GRID) as usize);
    let mut heights = Vec::with_capacity(vertices.capacity());
    for z in 0..GRID { for x in 0..GRID {
        let px = ORIGIN[0] + x as f32 * PITCH;
        let pz = ORIGIN[1] + z as f32 * PITCH;
        let height = terrain_height(px, pz);
        let dx = (terrain_height(px + PITCH, pz) - terrain_height(px - PITCH, pz)) / (2.0 * PITCH);
        let dz = (terrain_height(px, pz + PITCH) - terrain_height(px, pz - PITCH)) / (2.0 * PITCH);
        vertices.push(Vertex { pos: [px, height, pz],
            normal: Vec3::new(-dx, 1.0, -dz).normalize().to_array(), uv: [0.0; 2] });
        heights.push(height);
    }}
    let mut indices = Vec::with_capacity(((GRID - 1) * (GRID - 1) * 6) as usize);
    for z in 0..GRID - 1 { for x in 0..GRID - 1 {
        let a = z * GRID + x;
        // Same diagonal as the native triangular heightfield evaluator.
        indices.extend_from_slice(&[a, a + GRID, a + 1, a + 1, a + GRID, a + GRID + 1]);
    }}
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &indices, material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield { origin: ORIGIN, cell: PITCH, w: GRID, h: GRID, heights };
    scene
}

fn cameras() -> [(&'static str, Camera, bool); 4] {
    let camera = |pos: Vec3, target: Vec3| Camera { pos, target, fov_y: 60f32.to_radians(), near: 0.1 };
    [
        ("blocked", camera(Vec3::new(0.0, 2.0, 0.0), Vec3::new(45.0, 3.0, 0.0)), true),
        ("blocked-rotated", camera(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 0.0, -45.0)), true),
        ("high-exposed", camera(Vec3::new(0.0, 24.0, 0.0), Vec3::new(48.0, 0.0, 0.0)), false),
        ("side-exposed", camera(Vec3::new(0.0, 2.0, 60.0), Vec3::new(55.0, 0.0, 60.0)), false),
    ]
}

fn picks() -> Vec<(u32, u32)> {
    // Picking, rather than screen position alone, proves each compared sample
    // is on actual scene geometry. Exclude the sky from material/fog tests.
    (0..7).flat_map(|y| (0..9).map(move |x| (W / 5 + x * W / 14, H / 2 + y * H / 17))).collect()
}

fn rgb(pixels: &[u8], width: u32, p: (u32, u32)) -> [u8; 3] {
    let at = ((p.1 * width + p.0) * 4) as usize;
    pixels[at..at + 3].try_into().unwrap()
}
fn mean_rgb(rgb: [u8; 3]) -> f32 { rgb.into_iter().map(f32::from).sum::<f32>() / 3.0 }
fn panel(pixels: &[u8], x: u32) -> Vec<u8> {
    (0..H).flat_map(|y| {
        let begin = ((y * W * 2 + x) * 4) as usize;
        pixels[begin..begin + W as usize * 4].iter().copied()
    }).collect()
}
fn save(out: &Path, name: &str, width: u32, pixels: &[u8]) -> Result<(), String> {
    render::write_png(&out.join(format!("{name}.png")), width, H, pixels)
}
fn stats(name: &str, stats: &FrameStats) -> String {
    let times = stats.gpu_ms.iter().map(|(name, ms)| format!("[{name:?},{ms}]")).collect::<Vec<_>>().join(",");
    format!("{{\"case\":{name:?},\"camera_sun_visibility\":{:?},\"retained_triangles\":{},\"shadow_triangles\":{},\"gpu_ms\":[{times}]}}",
        stats.camera_sun_visibility, stats.triangles, stats.shadow_triangles)
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(&args.windows(2).find(|x| x[0] == "--out").ok_or("use --out FRESH_DIRECTORY")?[1]);
    if out.exists() { return Err("output directory must be fresh".into()); }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let enabled = std::env::var("KERN_CAMERA_SUN_OCCLUSION").as_deref() != Ok("0");
    let elevation = 10f32.to_radians();
    let light = Lighting { sun_dir: Vec3::new(elevation.cos(), elevation.sin(), 0.0),
        sun_lux: 18_000.0, sun_color: Vec3::new(1.15, 0.82, 0.45), sky_lux: 1_500.0, ev100: 12.0,
        fog_start: 0.0, fog_end: 1000.0, fog_density: 0.025, fog_height_m: 80.0,
        fog_phase_g: 0.6, fog_scattering_albedo: Vec3::splat(0.9), fog_culling: false,
        gi: 0.0, contact_occlusion: 0.0, reflection_occlusion: 0.0, shadow_strength: 1.0,
        near_shadows: 1.0, taa: 0.0, sharpen: 0.0, lod_pixels: 0.0, ..Lighting::default() };
    let mut no_sun = light.clone(); no_sun.sun_lux = 0.0;
    let mut clear = no_sun.clone(); clear.fog_density = 0.0;
    let scene = scene(); let gpu = Gpu::new()?; let renderer = Renderer::new(&gpu, &scene, &light);
    let picks = picks(); let cameras = cameras(); let mut failures = Vec::new();
    let mut records = Vec::new(); let mut measured = Vec::new(); let mut singles = Vec::new();
    let mut visibility = Vec::new();
    for (name, camera, blocked) in &cameras {
        let (sun, ss) = renderer.render_picking(&gpu, camera, &light, W, H, &picks);
        let (ambient, sa) = renderer.render_picking(&gpu, camera, &no_sun, W, H, &picks);
        records.push(stats(&format!("{name}-sun"), &ss)); records.push(stats(&format!("{name}-no-sun"), &sa));
        save(&out, &format!("{name}-sun"), W, &sun)?; save(&out, &format!("{name}-no-sun"), W, &ambient)?;
        let vis = ss.camera_sun_visibility.first().copied().unwrap_or(-1.0); visibility.push(vis);
        let expected = if enabled && *blocked { 0.0 } else { 1.0 };
        if (vis - expected).abs() > 1e-6 { failures.push(format!("{name}: visibility {vis}, expected {expected}")); }
        let valid: Vec<_> = picks.iter().enumerate().filter(|(i, _)| ss.picks.get(*i).is_some_and(Option::is_some)
            && sa.picks.get(*i).is_some_and(Option::is_some)).collect();
        if valid.len() < 12 { failures.push(format!("{name}: only{} picking-confirmed scene samples", valid.len())); }
        let mut max_delta = 0u8; let mut sum_on = 0.0; let mut sum_off = 0.0; let mut rows = Vec::new();
        for &(_, &p) in &valid {
            let a = rgb(&sun, W, p); let b = rgb(&ambient, W, p);
            max_delta = max_delta.max((0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap());
            sum_on += mean_rgb(a); sum_off += mean_rgb(b);
            rows.push(format!("{{\"pixel\":[{},{}],\"sun_rgb\":{a:?},\"no_sun_rgb\":{b:?}}}", p.0, p.1));
        }
        let count = valid.len().max(1) as f32; let on = sum_on / count; let off = sum_off / count;
        if enabled && *blocked && max_delta > BLOCKED_RGB_RESIDUAL { failures.push(format!("{name}: blocked solar fog exceeds material/quantization allowance; scene pixels change by{max_delta}")); }
        if (!enabled || !*blocked) && on < off + 2.0 { failures.push(format!("{name}: exposed sun haze missing; mean {on} versus{off}")); }
        if off < 2.0 { failures.push(format!("{name}: diffuse haze unexpectedly black ({off})")); }
        if *name == "blocked" || *name == "high-exposed" {
            let (black, sb) = renderer.render_picking(&gpu, camera, &clear, W, H, &picks);
            records.push(stats(&format!("{name}-no-fog"), &sb)); save(&out, &format!("{name}-no-fog"), W, &black)?;
            let maximum = valid.iter().filter(|(i, _)| sb.picks.get(*i).is_some_and(Option::is_some))
                .map(|&(_, &p)| rgb(&black, W, p).into_iter().max().unwrap()).max().unwrap_or(255);
            if maximum > 1 { failures.push(format!("{name}: black receiver without fog emitted{maximum}")); }
        }
        measured.push(format!("{{\"name\":{name:?},\"position\":{:?},\"target\":{:?},\"scene_samples\":{},\"sun_mean_rgb\":{on},\"no_sun_mean_rgb\":{off},\"max_rgb_delta\":{max_delta},\"samples\":[{}]}}",
            camera.pos.to_array(), camera.target.to_array(), valid.len(), rows.join(",")));
        singles.push((sun, ambient));
    }
    if visibility[0] != visibility[1] { failures.push("camera rotation changed terrain sun visibility at the same origin".into()); }
    let views = [ViewRect { cam: cameras[0].1, x: 0, y: 0, w: W, h: H },
        ViewRect { cam: cameras[2].1, x: W, y: 0, w: W, h: H }];
    for (name, lights, reference) in [("split-sun", &light, 0usize), ("split-no-sun", &no_sun, 1)] {
        let (pixels, stats_value) = renderer.render_views(&gpu, &views, lights, W * 2, H, &[]);
        records.push(stats(name, &stats_value)); save(&out, name, W * 2, &pixels)?;
        if reference == 0 && stats_value.camera_sun_visibility != [visibility[0], visibility[2]] {
            failures.push("two cameras do not have independent terrain sun visibility".into());
        }
        for (panel_i, single_i) in [(0, 0), (1, 2)] {
            let expected = if reference == 0 { &singles[single_i].0 } else { &singles[single_i].1 };
            let got = panel(&pixels, panel_i * W);
            let changed = got.chunks_exact(4).zip(expected.chunks_exact(4)).filter(|(a, b)| a[..3] != b[..3]).count();
            if changed != 0 { failures.push(format!("{name} view{panel_i}: {changed} pixels differ from single-camera reference")); }
        }
    }
    let report = format!("{{\"schema\":1,\"scope\":\"real terrain mesh and matching heightfield, native solar fog visibility; no gameplay FPS claim\",\"camera_sun_occlusion\":{enabled},\"panel_size\":[{W},{H}],\"blocked_max_rgb_residual\":{BLOCKED_RGB_RESIDUAL},\"sun_direction\":{:?},\"sun_lux\":{},\"ev100\":{},\"fog_density\":{},\"fog_height_m\":{},\"geometry\":\"2m grid, ridge16m, blackmetal receiver, GIoff, real shadowmaps\",\"limitations\":[\"camera-origin visibility only, not per-segment shadowed volumetric scattering\",\"terrain blocker only; trees and other meshes are not part of this horizon query\",\"blackmetal still has Schlick grazing reflection; blocked comparisons permit at most1 output level while camera visibility must be exactly0\"],\"measurements\":[{}],\"frames\":[{}],\"failures\":{:?},\"passed\":{}}}",
        light.sun_dir.to_array(), light.sun_lux, light.ev100, light.fog_density, light.fog_height_m,
        measured.join(","), records.join(","), failures, failures.is_empty());
    std::fs::write(out.join("report.json"), report).map_err(|e| e.to_string())?;
    if failures.is_empty() { println!("solar fog: visibility {visibility:?}; scene pixels and split views PASS"); Ok(()) }
    else { Err(failures.join("\n")) }
}
fn main() { if let Err(error) = run() { eprintln!("sun fog visibility: {error}"); std::process::exit(1); } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_source_and_heightfield_share_the_actual_ridge() {
        let scene = scene(); let hf = &scene.heightfield;
        assert_eq!(hf.heights.len(), (GRID * GRID) as usize);
        for z in 0..GRID { for x in 0..GRID {
            assert_eq!(hf.heights[(z * GRID + x) as usize], terrain_height(ORIGIN[0] + x as f32 * PITCH, ORIGIN[1] + z as f32 * PITCH));
        }}
        assert_eq!(terrain_height(22.0, 0.0), 16.0);
        assert_eq!(terrain_height(22.0, 60.0), 0.0);
        assert!(scene.vertices.iter().all(|v| v.pos[1] == terrain_height(v.pos[0], v.pos[2])));
        let cases = cameras(); assert_eq!(cases[0].1.pos, cases[1].1.pos);
        assert_ne!(cases[0].1.target, cases[1].1.target);
        assert!(picks().iter().all(|&(x, y)| x < W && y < H));
    }
}
