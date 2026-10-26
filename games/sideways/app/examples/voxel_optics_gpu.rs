//! Actual-renderer optical goldens, independent of gameplay or source assets.
//! cargo run --release -p cli --example voxel_optics_gpu -- --out FRESH_DIR
//! Synthetic centre-sampled densities have an analytic tent-function integral;
//! this is not a second implementation of the GPU traversal/cache/skip logic.
use glam::{Affine3A, Mat4, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::foliage_volume_data::{PackLimits, VolumeBinding, pack};
use render::foliage_voxel::{BakeStats, Voxel, VoxelAsset, VoxelLevel};
use render::scene::{Material, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

const SIZE: u32 = 128;
const CELL: f32 = 0.25;
const DIMS: [u32; 3] = [12, 12, 24];
const ORIGIN: [f32; 3] = [-1.5, -1.5, -3.0];
const ALPHA_TOLERANCE: f64 = 0.003;
const PROBES: [(u32, u32); 4] = [(63, 63), (64, 63), (63, 64), (64, 64)];

#[derive(Clone)]
struct Medium { sigma: f32, color: [f32; 3], planes: Vec<u32> }
#[derive(Clone)]
struct Body { asset: usize, transform: Affine3A }
struct Case {
    name: &'static str, media: Vec<Medium>, bodies: Vec<Body>,
    reverse_bindings: bool, camera_z: f32, opaque_z: Option<f32>,
    require_empty_skip: bool,
}
struct ResultImage { name: &'static str, raw: Vec<[f32; 4]>, report: Value }

fn medium(sigma: f32, color: [f32; 3], sparse: bool) -> Medium {
    Medium { sigma, color, planes: if sparse { vec![2, 3, 19, 20] } else { (0..DIMS[2]).collect() } }
}
fn cases() -> Vec<Case> {
    let uniform = medium(0.16, [0.20, 0.35, 0.10], false);
    let one = |name, value: Medium| Case { name, media: vec![value],
        bodies: vec![Body { asset: 0, transform: Affine3A::IDENTITY }],
        reverse_bindings: false, camera_z: 10.0, opaque_z: None, require_empty_skip: false };
    let mut empty = one("empty", uniform.clone()); empty.media[0].planes.clear();
    let mut negative = one("sparse_negative_z", medium(0.45, [0.30, 0.15, 0.12], true));
    negative.require_empty_skip = true;
    let mut positive = one("sparse_positive_z", negative.media[0].clone());
    positive.camera_z = -10.0; positive.require_empty_skip = true;
    let overlap = |name, reverse_bindings| Case { name,
        media: vec![medium(0.12, [0.40, 0.10, 0.08], false), medium(0.19, [0.05, 0.35, 0.12], false)],
        // Coincident volumes admit a closed-form mixed-radiance oracle too.
        // Reversing equal-entry owners must not select only the first medium.
        bodies: vec![Body { asset: 0, transform: Affine3A::IDENTITY },
            Body { asset: 1, transform: Affine3A::IDENTITY }],
        reverse_bindings, camera_z: 10.0, opaque_z: None, require_empty_skip: false };
    let mut scaled = one("nonuniform_scale", uniform.clone());
    scaled.bodies[0].transform = Affine3A::from_scale(Vec3::new(0.7, 1.4, 1.8));
    let mut half = one("opaque_half", uniform.clone()); half.opaque_z = Some(0.0);
    let mut front = one("opaque_front", uniform.clone()); front.opaque_z = Some(4.0);
    let mut behind = one("opaque_behind", uniform.clone()); behind.opaque_z = Some(-4.0);
    let mut disjoint = one("disjoint_intervals", uniform.clone());
    disjoint.camera_z = 16.0;
    disjoint.bodies = vec![Body { asset: 0, transform: Affine3A::from_translation(Vec3::Z * 4.0) },
        Body { asset: 0, transform: Affine3A::from_translation(Vec3::Z * -4.0) }];
    vec![empty, one("uniform", uniform), negative, positive, overlap("overlap_ab", false),
        overlap("overlap_ba", true), scaled, half, front, behind, disjoint]
}

fn asset(medium: &Medium) -> VoxelAsset {
    let mut cells = BTreeMap::<[u32; 3], f32>::new();
    for x in 0..DIMS[0] { for y in 0..DIMS[1] { for &z in &medium.planes {
        cells.insert([x, y, z], medium.sigma * CELL.powi(3));
    }}}
    let mut dimensions = DIMS; let mut cell_size = CELL; let mut levels = Vec::new();
    for level in 0..3 {
        levels.push(VoxelLevel { cell_size, dimensions, voxels: cells.iter().map(|(&coord, &area)| Voxel {
            coord, surface_area: area * 2.0, projected_area: [area; 3], directional_area: [area; 13],
            albedo: medium.color, roughness: 0.9, leaf_fraction: 0.0, normal_weights: [1.0 / 6.0; 6],
            normal_centroids: [[1.,0.,0.],[-1.,0.,0.],[0.,1.,0.],[0.,-1.,0.],[0.,0.,1.],[0.,0.,-1.]],
        }).collect() });
        if level == 2 { break; }
        let mut parents = BTreeMap::new();
        for (coord, area) in cells { *parents.entry(coord.map(|v| v / 2)).or_insert(0.0) += area; }
        cells = parents; dimensions = dimensions.map(|v| v.div_ceil(2)); cell_size *= 2.0;
    }
    VoxelAsset { projection_measure: Default::default(), origin: ORIGIN, source_bounds: [ORIGIN, [1.5, 1.5, 3.0]], levels, stats: BakeStats::default() }
}
fn material() -> Material {
    Material { tint: [0.2; 3], layer: -1, roughness: 0.9, flags: TWO_SIDED, scale: 1.0, metal: 0.0 }
}
fn mesh(scene: &mut Scene, positions: &[[f32; 3]], indices: &[u32], material: u32) -> usize {
    let vertices: Vec<_> = positions.iter().map(|&pos| Vertex { pos, normal: [0.0, 0.0, 1.0], uv: [0.0; 2] }).collect();
    scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices, material }]))
}
fn fixture(case: &Case) -> Result<Scene, String> {
    let mut scene = Scene::default();
    let crown = scene.add_material(material());
    let solid = scene.add_material(material());
    // These owners provide real instance transforms. Their tiny triangle is
    // replaced by the volume path, never an accidental opaque depth blocker.
    let dummy = mesh(&mut scene, &[[-0.1, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]], &[0, 1, 2], crown);
    let mut bindings = Vec::new();
    for body in &case.bodies {
        let owner = scene.instances.len() as u32;
        scene.add_instance(dummy, body.transform); scene.mark_no_shadow();
        bindings.push(VolumeBinding { owner, asset: body.asset as u32, crown_material: crown });
    }
    if case.reverse_bindings { bindings.reverse(); }
    if let Some(z) = case.opaque_z {
        let plane = mesh(&mut scene, &[[-20.0, -20.0, z], [20.0, -20.0, z], [-20.0, 20.0, z], [20.0, 20.0, z]],
            &[0, 1, 2, 2, 1, 3], solid);
        scene.add_instance(plane, Affine3A::IDENTITY); scene.mark_no_shadow();
    }
    let assets: Vec<_> = case.media.iter().map(asset).collect();
    scene.voxel_forest = Some(pack(&assets, &bindings, &scene.instances,
        PackLimits { first_level_bounds: true, ..PackLimits::default() })?);
    Ok(scene)
}
fn camera(case: &Case) -> Camera {
    Camera { pos: Vec3::new(0.0, 0.0, case.camera_z), target: Vec3::ZERO, fov_y: 35f32.to_radians(), near: 0.1 }
}
fn pixel_ray(camera: &Camera, pixel: (u32, u32)) -> (Vec3, f64) {
    let inverse = (Mat4::perspective_infinite_reverse_rh(camera.fov_y, 1.0, camera.near)
        * Mat4::look_at_rh(camera.pos, camera.target, Vec3::Y)).inverse();
    let ndc = glam::Vec2::new((pixel.0 as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0,
        1.0 - (pixel.1 as f32 + 0.5) / SIZE as f32 * 2.0);
    let far = inverse * glam::Vec4::new(ndc.x, ndc.y, 1e-4, 1.0);
    let near = inverse * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
    ((far.truncate() / far.w - camera.pos).normalize(), camera.pos.distance(near.truncate() / near.w) as f64)
}
// Antiderivative of the normalized triangular basis max(1-|u|,0).
// Its full integral is one: each centre-sampled cell retains exactly h mass
// when the half-cell filter support beyond the grid is included.
fn tent_cdf(u: f64) -> f64 {
    if u <= -1.0 { 0.0 } else if u < 0.0 { 0.5 * (u + 1.0).powi(2) }
    else if u < 1.0 { 1.0 - 0.5 * (1.0 - u).powi(2) } else { 1.0 }
}
fn expected_alpha(case: &Case, camera: &Camera, pixel: (u32, u32)) -> Result<(f64, f64), String> {
    let (direction, near) = pixel_ray(camera, pixel);
    let limit = case.opaque_z.map(|z| ((z - camera.pos.z) / direction.z) as f64).unwrap_or(1_000.0);
    if limit <= near { return Ok((0.0, 0.0)); }
    let mut tau = 0.0;
    for body in &case.bodies {
        let medium = &case.media[body.asset];
        if medium.planes.is_empty() { continue; }
        let inverse = body.transform.inverse();
        let origin = inverse.transform_point3(camera.pos);
        let local_direction = inverse.transform_vector3(direction);
        let dz = local_direction.z as f64;
        if dz.abs() < 1e-8 { return Err("analytic fixture requires nonparallel z rays".into()); }
        let lo = ORIGIN[2] as f64 - CELL as f64 * 0.5;
        let hi = ORIGIN[2] as f64 + DIMS[2] as f64 * CELL as f64 + CELL as f64 * 0.5;
        let a = (lo - origin.z as f64) / dz; let b = (hi - origin.z as f64) / dz;
        let enter = near.max(a.min(b)); let leave = limit.min(a.max(b));
        if leave <= enter { continue; }
        // The four central rays remain in the exactly constant x/y plateau.
        // Reject a fixture/camera change that invalidates this analytic proof.
        for t in [enter, leave] { for axis in 0..2 {
            let p = origin[axis] as f64 + local_direction[axis] as f64 * t;
            let low = ORIGIN[axis] as f64 + CELL as f64 * 0.5;
            let high = ORIGIN[axis] as f64 + (DIMS[axis] as f64 - 0.5) * CELL as f64;
            if p < low || p > high { return Err("analytic probe leaves homogeneous x/y plateau".into()); }
        }}
        let za = origin.z as f64 + dz * enter; let zb = origin.z as f64 + dz * leave;
        let mut integral = 0.0;
        for &z in &medium.planes {
            let centre = ORIGIN[2] as f64 + (z as f64 + 0.5) * CELL as f64;
            integral += CELL as f64 * (tent_cdf((za.max(zb) - centre) / CELL as f64)
                - tent_cdf((za.min(zb) - centre) / CELL as f64));
        }
        tau += medium.sigma as f64 * local_direction.length() as f64 / dz.abs() * integral;
    }
    Ok((1.0 - (-tau).exp(), tau))
}
fn half_to_f32(bits: u16) -> f32 {
    let exponent = (bits >> 10) & 31; let fraction = bits & 1023;
    if exponent == 0 { let v = fraction as f32 / 16_777_216.0; return if bits & 0x8000 != 0 { -v } else { v }; }
    f32::from_bits(((bits as u32 & 0x8000) << 16) | ((if exponent == 31 { 255 } else { exponent as u32 + 112 }) << 23)
        | ((fraction as u32) << 13))
}
fn check(checks: &mut Vec<Value>, name: &str, passed: bool, evidence: Value) {
    checks.push(json!({"name":name,"pass":passed,"evidence":evidence}));
}
fn render_case(gpu: &Gpu, out: &std::path::Path, case: &Case, light: &Lighting) -> Result<ResultImage, String> {
    let scene = fixture(case)?; let camera = camera(case);
    let renderer = Renderer::new(gpu, &scene, light);
    let begun = Instant::now();
    let (pixels, stats) = renderer.render(gpu, &camera, light, SIZE, SIZE);
    let wall_ms = begun.elapsed().as_secs_f64() * 1_000.0;
    render::write_png(&out.join(format!("{}-render.png", case.name)), SIZE, SIZE, &pixels)?;
    let mut layers = renderer.voxel_layers_rgba16(gpu)?;
    if layers.len() != 1 { return Err(format!("{}: expected one raw voxel layer", case.name)); }
    let (w, h, bytes) = layers.remove(0);
    if (w, h) != (SIZE, SIZE) || bytes.len() != SIZE as usize * SIZE as usize * 8 { return Err("invalid raw layer size".into()); }
    std::fs::write(out.join(format!("{}-rgba16f.bin", case.name)), &bytes).map_err(|e| e.to_string())?;
    let raw: Vec<[f32; 4]> = bytes.chunks_exact(8).map(|p|
        std::array::from_fn(|k| half_to_f32(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])))).collect();
    let finite = raw.iter().flatten().all(|v| v.is_finite());
    let valid_alpha = raw.iter().all(|p| (0.0..=1.0001).contains(&p[3]));
    let alpha_image: Vec<u8> = raw.iter().flat_map(|p| {
        let c = (p[3].clamp(0.0, 1.0) * 255.0).round() as u8; [c, c, c, 255]
    }).collect();
    render::write_png(&out.join(format!("{}-alpha.png", case.name)), SIZE, SIZE, &alpha_image)?;
    let mut checks = Vec::new();
    check(&mut checks, "finite_raw_rgba16f", finite && valid_alpha, json!({"finite":finite,"alpha_in_range":valid_alpha}));
    let diagnostics = renderer.voxel_diagnostics(gpu)?.ok_or("missing voxel diagnostics")?;
    if diagnostics.len() != 1 { return Err("expected one diagnostic view".into()); }
    let d = &diagnostics[0];
    check(&mut checks, "complete_bounded_integration", d.step_overflow_rays == 0,
        json!({"step_overflow_rays":d.step_overflow_rays}));
    if case.require_empty_skip {
        check(&mut checks, "sparse_fixture_exercises_empty_brick_skip", d.max_empty_brick_skips > 0,
            json!({"max_empty_brick_skips":d.max_empty_brick_skips}));
    }
    if case.name == "empty" || case.name == "opaque_front" {
        let maximum = raw.iter().flatten().map(|v| v.abs()).fold(0.0f32, f32::max);
        check(&mut checks, "empty_or_fully_occluded_layer_is_zero", finite && maximum == 0.0, json!({"maximum_absolute_rgba":maximum}));
    }
    for pixel in PROBES {
        let (expected, tau) = expected_alpha(case, &camera, pixel)?;
        let actual = raw[(pixel.1 * SIZE + pixel.0) as usize][3] as f64;
        check(&mut checks, &format!("analytic_alpha_{}_{}", pixel.0, pixel.1),
            actual.is_finite() && (actual - expected).abs() <= ALPHA_TOLERANCE,
            json!({"pixel":[pixel.0,pixel.1],"expected_alpha":expected,"actual_alpha":actual,
                "optical_depth":tau,"absolute_error":(actual-expected).abs(),"tolerance":ALPHA_TOLERANCE}));
        if case.name.starts_with("overlap_") {
            // Both roots are exactly (0,0): source hash2(0,0)=0, giving
            // multiplier .82. Debug-albedo radiance is 6000*albedo*.82.
            // Joint integration must use the extinction-weighted colour,
            // not composite whole trees in an arbitrary front-to-back order.
            let sum: f64 = case.media.iter().map(|m|m.sigma as f64).sum();
            let value = raw[(pixel.1 * SIZE + pixel.0) as usize];
            let target: [f64; 3] = std::array::from_fn(|k| actual * 6000.0 * 0.82
                * case.media.iter().map(|m|m.sigma as f64 * m.color[k] as f64).sum::<f64>() / sum);
            let error: f64 = (0..3).map(|k|(value[k] as f64-target[k]).abs()/target[k].max(1.0)).fold(0.0,f64::max);
            check(&mut checks, &format!("joint_mixed_radiance_{}_{}", pixel.0,pixel.1), error <= 0.002,
                json!({"expected_premultiplied_rgb":target,"actual_rgb":[value[0],value[1],value[2]],"maximum_relative_error":error,"tolerance":0.002}));
        }
    }
    let diag = json!({"candidate_overflow_tiles":d.candidate_overflow_tiles,"step_overflow_rays":d.step_overflow_rays,
        "traced_rays":d.traced_rays,"max_steps_used":d.max_steps_used,"max_tile_candidates":d.max_tile_candidates,
        "max_ray_candidates":d.max_ray_candidates,"ray_candidate_overflow_rays":d.ray_candidate_overflow_rays,
        "max_active_volumes":d.max_active_volumes,"active_cache_overflow_rays":d.active_cache_overflow_rays,
        "max_empty_brick_skips":d.max_empty_brick_skips});
    let report = json!({"name":case.name,"pass":checks.iter().all(|c|c["pass"]==true),"checks":checks,
        "camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y,"near":camera.near},
        "opaque_z":case.opaque_z,"reverse_bindings":case.reverse_bindings,
        "media":case.media.iter().map(|m|json!({"sigma_per_local_metre":m.sigma,"albedo_linear":m.color,"occupied_z_planes":m.planes})).collect::<Vec<_>>(),
        "bodies":case.bodies.iter().map(|b|json!({"asset":b.asset,"transform_columns":Mat4::from(b.transform).to_cols_array()})).collect::<Vec<_>>(),
        "diagnostics":diag,"first_frame_wall_ms":wall_ms,"gpu_ms":stats.gpu_ms});
    std::fs::write(out.join(format!("{}-report.json", case.name)), serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("{}: {}", case.name, if report["pass"] == true { "PASS" } else { "FAIL" });
    Ok(ResultImage { name: case.name, raw, report })
}

fn run(out: PathBuf) -> Result<(), String> {
    if out.as_os_str().is_empty() || out.exists() { return Err("--out must name a fresh directory".into()); }
    if let Some(parent) = out.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    std::fs::create_dir(&out).map_err(|e| e.to_string())?;
    let begun = Instant::now(); let gpu = Gpu::new()?;
    let light = Lighting { debug: 2.0, taa: 0.0, sharpen: 0.0, gi: 0.0, contact_occlusion: 0.0,
        reflection_occlusion: 0.0, near_shadows: 0.0, shadow_strength: 0.0,
        fog_start: 1e6, fog_end: 2e6, fog_density: 0.0, ..Lighting::default() };
    let mut results = Vec::new();
    for case in cases() { results.push(render_case(&gpu, &out, &case, &light)?); }
    let mut comparisons = Vec::new();
    for (a, b) in [("overlap_ab", "overlap_ba"), ("uniform", "opaque_behind")] {
        let first = results.iter().find(|r| r.name == a).unwrap();
        let second = results.iter().find(|r| r.name == b).unwrap();
        let mut alpha_error = 0.0f32; let mut color_relative_error = 0.0f32;
        for (x, y) in first.raw.iter().zip(&second.raw) {
            alpha_error = alpha_error.max((x[3] - y[3]).abs());
            for k in 0..3 { color_relative_error = color_relative_error.max((x[k] - y[k]).abs() / x[k].abs().max(y[k].abs()).max(1.0)); }
        }
        check(&mut comparisons, &format!("full_image_{a}_versus_{b}"), alpha_error <= 0.001 && color_relative_error <= 0.002,
            json!({"max_alpha_error":alpha_error,"max_hdr_relative_error":color_relative_error,"alpha_tolerance":0.001,"hdr_relative_tolerance":0.002}));
    }
    let passed = results.iter().all(|r| r.report["pass"] == true) && comparisons.iter().all(|c|c["pass"]==true);
    let report = json!({"schema":1,"purpose":"Actual GPU optical goldens for the live volume shader; no gameplay FPS or source-mesh fidelity claim",
        "pass":passed,"adapter":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"output_size":[SIZE,SIZE],
        "cell_size_m":CELL,"dimensions":DIMS,"origin":ORIGIN,"levels":3,"taa":false,"lighting_debug":"unlit albedo",
        "oracle":"Closed-form integral of triangular reconstruction bases along each transformed central ray, with exact opaque plane and near clipping; Beer-Lambert exp(-sum tau). No shader cache/skip/traversal emulation.",
        "limitations":["Analytic alpha checks cover four central pixels in the constant x/y interior; full images are retained for inspection.",
            "Synthetic isotropic density validates optics and traversal, not real foliage coverage or lighting fidelity.",
            "At most two overlapping volumes: 16/64/256-capacity fallback requires a separate stress fixture.",
            "Tolerance allows midpoint integration, f32 arithmetic and RGBA16F storage; first-frame timing includes initialization effects."],
        "comparisons":comparisons,"runs":results.iter().map(|r|&r.report).collect::<Vec<_>>(),"elapsed_seconds":begun.elapsed().as_secs_f64()});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if passed { println!("voxel optics GPU: PASS ({})", out.display()); Ok(()) }
    else { Err(format!("optical checks failed; retained report and images in {}", out.display())) }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.len() == 2 && args[0] == "--out" { run(PathBuf::from(&args[1])) }
        else { Err("usage: voxel_optics_gpu --out FRESH_DIRECTORY (128x128, bounded 11 cases)".into()) };
    if let Err(error) = result { eprintln!("voxel_optics_gpu: {error}"); std::process::exit(1); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analytic_tents_keep_halo_mass_and_half_clip() {
        assert_eq!(tent_cdf(-1.0), 0.0); assert_eq!(tent_cdf(0.0), 0.5); assert_eq!(tent_cdf(1.0), 1.0);
        // Clipping to the original cell interval loses a quarter of the tent.
        assert_eq!(tent_cdf(0.5) - tent_cdf(-0.5), 0.75);
        let fixtures = cases(); let uniform = fixtures.iter().find(|c|c.name=="uniform").unwrap();
        let half = fixtures.iter().find(|c|c.name=="opaque_half").unwrap();
        let full_tau = expected_alpha(uniform, &camera(uniform), PROBES[0]).unwrap().1;
        let half_tau = expected_alpha(half, &camera(half), PROBES[0]).unwrap().1;
        assert!((full_tau - 0.96).abs() < 0.0001);
        assert!((half_tau * 2.0 - full_tau).abs() < 1e-6);
    }
    #[test]
    fn synthetic_sparse_hierarchy_preserves_directional_mass() {
        let source = asset(&medium(0.45, [0.2; 3], true));
        let masses: Vec<f64> = source.levels.iter().map(|l|l.voxels.iter().map(|v|v.directional_area[0] as f64).sum()).collect();
        assert!(masses.iter().all(|mass|(mass-masses[0]).abs() < 1e-5));
        assert!(source.levels[2].voxels.len() < (source.levels[2].dimensions.iter().product::<u32>() as usize));
    }
    #[test]
    fn raw_half_decode_does_not_hide_nonfinite_values() {
        assert_eq!(half_to_f32(0), 0.0); assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(1), 2f32.powi(-24)); assert!(half_to_f32(0x7c00).is_infinite());
        assert!(half_to_f32(0x7e00).is_nan());
    }
}
