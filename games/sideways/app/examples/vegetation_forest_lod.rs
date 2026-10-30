//! Repeated approved spruce source versus aggregate LOD, with matched cameras.
//! KERN_DIAGNOSTIC_HIDE=none enables native opacity/ownership captures.
//! cargo run --locked --release -p cli --example vegetation_forest_lod --
//!   --root GAME_ROOT --out FRESH_DIRECTORY --trees 4 --views 1 --mode both
//! GPU timestamps exclude warmup and image/debug readback. This is a dedicated offscreen
//! renderer fixture, not Sideways frame time, a quality gate or a forest claim.

#[path = "support/vegetation_image_metrics.rs"]
mod metrics;
#[allow(dead_code)]
#[path = "../src/plant_species.rs"]
mod plant_species;

use glam::{Affine3A, Mat4, Vec2, Vec3};
use image::{Rgba, RgbaImage};
use render::aggregate_lod::Policy;
use render::cluster::{ClusterMesh, GpuCluster, Part, Vertex};
use render::foliage_assembly::Assembly;
use render::gpu_profile::GpuProfileSample;
use render::scene::{GeometryCategory, GpuInstance, Heightfield, Material, THIN_FOLIAGE};
use render::vegetation_asset::{AttachLimits, CookedPlant, PlantInstance, SceneVegetation};
use render::wind::WindState;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SEED: u64 = 20261028;
const SPECIES: &str = "spruce";

struct Args {
    root: PathBuf,
    out: PathBuf,
    trees: usize,
    views: usize,
    size: [u32; 2],
    distances: Vec<f32>,
    spacing: f32,
    warmup: usize,
    samples: usize,
    time: f64,
    step: f64,
    taa: bool,
    mode: String,
    error: f32,
    shadow_lod: f32,
    reference_cell: f32,
    /// Off shows the shared sun map alone (it holds rasterized voxels).
    near_shadows: bool,
}

fn args() -> Result<Args, String> {
    let raw: Vec<_> = std::env::args().skip(1).collect();
    let usage = "vegetation_forest_lod --root GAME_ROOT --out FRESH_DIRECTORY [--trees 4] [--views 1|4] [--distances 50,150,400] [--spacing 12] [--width 1920] [--height 1080] [--warmup 32] [--samples 48] [--time 4.25] [--step 0.0166666667] [--taa 0|1] [--mode source|auto|both] [--error 1] [--shadow-lod 1] [--reference-cell 0.075] [--near-shadows 1|0]";
    if raw.is_empty() || raw.len() % 2 != 0 {
        return Err(usage.into());
    }
    let mut map = BTreeMap::new();
    for pair in raw.chunks_exact(2) {
        if ![
            "--root",
            "--out",
            "--trees",
            "--views",
            "--distances",
            "--spacing",
            "--width",
            "--height",
            "--warmup",
            "--samples",
            "--time",
            "--step",
            "--taa",
            "--mode",
            "--error",
            "--shadow-lod",
            "--reference-cell",
            "--near-shadows",
        ]
        .contains(&pair[0].as_str())
            || map.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(format!("unknown/repeated option {}: {usage}", pair[0]));
        }
    }
    let value = |key, default| map.get(key).copied().unwrap_or(default);
    let integer = |key, default| {
        value(key, default)
            .parse::<usize>()
            .map_err(|_| format!("invalid integer {key}"))
    };
    let number = |key, default| {
        value(key, default)
            .parse::<f64>()
            .map_err(|_| format!("invalid number {key}"))
    };
    let trees = integer("--trees", "4")?;
    let views = integer("--views", "1")?;
    let width = integer("--width", "1920")?;
    let height = integer("--height", "1080")?;
    let mode = value("--mode", "both").to_string();
    let distances = value("--distances", "50,150,400")
        .split(',')
        .map(|s| {
            s.parse::<f32>()
                .map_err(|_| "invalid --distances".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let spacing = number("--spacing", "12")?;
    let time = number("--time", "4.25")?;
    let step = number("--step", "0.0166666667")?;
    let error = number("--error", "1")?;
    let shadow_lod = number("--shadow-lod", "1")?;
    let reference_cell = number("--reference-cell", "0.075")?;
    let warmup = integer("--warmup", "32")?;
    let samples = integer("--samples", "48")?;
    let taa = match value("--taa", "0") {
        "0" => false,
        "1" => true,
        _ => return Err("--taa must be 0 or 1".into()),
    };
    if !(1..=64).contains(&trees)
        || ![1, 4].contains(&views)
        || !(64..=4096).contains(&width)
        || !(64..=4096).contains(&height)
        || width % 2 != 0
        || height % 2 != 0
        || distances.is_empty()
        || distances.len() > 16
        || distances
            .iter()
            .any(|x| !x.is_finite() || *x < 1.0 || *x > 5000.0)
        || !reference_cell.is_finite() || !(0.01..=0.5).contains(&reference_cell)
        || !spacing.is_finite()
        || !(4.0..=100.0).contains(&spacing)
        || !time.is_finite()
        || time < 0.0
        || !step.is_finite()
        || !(0.0..=0.1).contains(&step)
        || !error.is_finite()
        || !(0.01..=16.0).contains(&error)
        || !shadow_lod.is_finite() || !(0.0..=8.0).contains(&shadow_lod)
        || !(1..=1024).contains(&warmup)
        || !(1..=1024).contains(&samples)
        || !["source", "auto", "both"].contains(&mode.as_str())
    {
        return Err(format!("argument outside fixture bounds: {usage}"));
    }
    // Four full-detail trees already submit millions of source triangles.
    // Larger populations are opt-in auto-only until the source oracle is bounded.
    if trees > 4 && mode != "auto" {
        return Err("more than four trees requires --mode auto; the source oracle remains full detail, never silently degraded".into());
    }
    Ok(Args {
        root: PathBuf::from(map.get("--root").ok_or(usage)?),
        out: PathBuf::from(map.get("--out").ok_or(usage)?),
        trees,
        views,
        size: [width as u32, height as u32],
        distances,
        spacing: spacing as f32,
        warmup,
        samples,
        time,
        step,
        taa,
        mode,
        error: error as f32,
        shadow_lod: shadow_lod as f32,
        reference_cell: reference_cell as f32,
        near_shadows: value("--near-shadows", "1") != "0",
    })
}

fn source_bounds(assembly: &Assembly) -> Result<[Vec3; 2], String> {
    let branches = assembly.branch_transforms()?;
    let mut bounds = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
    for instance in &assembly.instances {
        let transform = branches[instance.branch as usize] * instance.rest;
        for vertex in &assembly.parts[instance.part as usize].vertices {
            let p = transform.transform_point3(vertex.pos.into());
            if !p.is_finite() {
                return Err("nonfinite source bounds".into());
            }
            bounds[0] = bounds[0].min(p);
            bounds[1] = bounds[1].max(p);
        }
    }
    if !bounds[0].is_finite() || !bounds[1].is_finite() {
        return Err("empty source bounds".into());
    }
    Ok(bounds)
}

fn corners(bounds: [Vec3; 2]) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        Vec3::new(
            bounds[usize::from(i & 1 != 0)].x,
            bounds[usize::from(i & 2 != 0)].y,
            bounds[usize::from(i & 4 != 0)].z,
        )
    })
}

fn roots(trees: usize, spacing: f32) -> Vec<Mat4> {
    let width = (trees as f32).sqrt().ceil() as usize;
    let height = trees.div_ceil(width);
    (0..trees)
        .map(|i| {
            let p = Vec3::new(
                (i % width) as f32 - (width - 1) as f32 * 0.5,
                0.0,
                (i / width) as f32 - (height - 1) as f32 * 0.5,
            ) * spacing;
            Mat4::from_translation(p) * Mat4::from_rotation_y((i % 7) as f32 * 0.37)
        })
        .collect()
}

fn ground(scene: &mut Scene) {
    let material = scene.add_material(Material {
        tint: [0.42; 3],
        layer: -1,
        roughness: 1.0,
        flags: 0,
        scale: 1.0,
        metal: 0.0,
    });
    let vertices = [
        [-2000.0, -0.015, -2000.0],
        [-2000.0, -0.015, 2000.0],
        [2000.0, -0.015, -2000.0],
        [2000.0, -0.015, 2000.0],
    ]
    .map(|pos| Vertex {
        pos,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0; 2],
    });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 1, 2, 2, 1, 3],
        material,
    }]));
    scene.add_classified_instance(mesh, Affine3A::IDENTITY, GeometryCategory::Terrain);
    scene.heightfield = Heightfield {
        origin: [-2000.0; 2],
        cell: 20.0,
        w: 201,
        h: 201,
        heights: vec![-0.015; 201 * 201],
    };
}

fn build_scene(args: &Args) -> Result<(Scene, [Vec3; 2], Value), String> {
    let started = Instant::now();
    let assembly = plant_species::build(SPECIES, SEED)?;
    let local = source_bounds(&assembly)?;
    let transforms = roots(args.trees, args.spacing);
    let mut bounds = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
    for root in &transforms {
        for p in corners(local) {
            let p = root.transform_point3(p);
            bounds[0] = bounds[0].min(p);
            bounds[1] = bounds[1].max(p);
        }
    }
    let config = render::vegetation_aggregate::Config {
        max_placements:8192,max_source_triangles:40_000_000,max_samples:150_000_000,
        triangle_refinements:true,reference_cell_size:args.reference_cell,
        max_reference_voxels:262_144,max_diagnostic_steps:80_000_000,
        ..Default::default()
    };
    let config_report = format!("{config:?}");
    eprintln!(
        "cooking one shared {SPECIES} asset for {} trees",
        args.trees
    );
    let cooked = CookedPlant::cook_aggregate(&assembly, config)?;
    let stats = cooked.stats();
    let mut scene = Scene::default();
    scene.load_sky(&args.root.join("assets/sky/sky_2k.hdr"))?;
    ground(&mut scene);
    let mut limits = AttachLimits {
        max_poses: args.trees * assembly.branches.len(),
        max_primitive_instances: args.trees * assembly.instances.len(),
        ..Default::default()
    };
    limits.pack.max_instances = args.trees * assembly.branches.len();
    let mut builder = SceneVegetation::new(&mut scene, limits)?;
    for (index, root) in transforms.iter().enumerate() {
        builder.append(
            &cooked,
            PlantInstance {
                id: SEED + index as u64,
                generation: 0,
                root: *root,
                category: GeometryCategory::Trees,
            },
        )?;
    }
    let policy = Policy {
        max_error_pixels: args.error,
        allow_sampled_optics: true,
        ..Default::default()
    };
    let attached = builder.finish_aggregate(policy)?;
    scene.part_wind_lod = true;
    let instance_bytes = scene.instances.len() * std::mem::size_of::<GpuInstance>();
    let geometry_bytes = scene.vertices.len() * std::mem::size_of::<Vertex>()
        + (scene.cluster_verts.len() + scene.tris.len()) * 4
        + scene.clusters.len() * std::mem::size_of::<GpuCluster>();
    let candidate_errors = scene.aggregate_lod.as_ref().map(|selection| {
        selection
            .groups
            .iter()
            .map(|group| {
                json!({
                    "volume":group.volume,"world_center":group.center.to_array(),
                    "world_radius":group.radius,"scale":group.scale,
                    "levels":group.candidates.iter().map(|candidate|json!({
                        "level":candidate.level,"source_only":candidate.source_only,"spatial_error":candidate.spatial_error,
                        "voxel_coverage_error":candidate.voxel_coverage_error,
                        "voxel_color_error":candidate.voxel_color_error,
                        "triangle_coverage_error":candidate.triangle_coverage_error,
                        "triangle_color_error":candidate.triangle_color_error,
                        "triangle_cut_error":candidate.triangle_error,
                    })).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
    });
    let report = json!({"species":SPECIES,"seed":SEED,"source_api":"plant_species::build; shared CookedPlant::cook_aggregate; no expanded source mesh",
        "trees":args.trees,"spacing_m":args.spacing,"root_matrices":transforms.iter().map(|m|m.to_cols_array()).collect::<Vec<_>>(),
        "single_rest_bounds_m":local.map(|p|p.to_array()),"forest_rest_bounds_m":bounds.map(|p|p.to_array()),
        "preparation_wall_seconds":started.elapsed().as_secs_f64(),"cooker_config":config_report,
        "selection_policy":{"max_error_pixels":policy.max_error_pixels,"max_coverage_error":policy.max_coverage_error,
            "max_color_error":policy.max_color_error,"allow_sampled_optics":policy.allow_sampled_optics},
        "cook_stats":{"primitive_meshes":stats.primitive_meshes,"primitive_vertices":stats.primitive_vertices,
            "primitive_triangles":stats.primitive_triangles,"primitive_bytes":stats.primitive_bytes,"placements":stats.placements,
            "branches":stats.branches,"volume_assets":stats.volume_assets,"expanded_source_vertices":stats.expanded_source_vertices,
            "expanded_source_triangles":stats.expanded_source_triangles,"expanded_source_bytes":stats.expanded_source_bytes},
        "attachment":{"plants":attached.plants,"primitive_meshes":attached.primitive_meshes,"primitive_instances":attached.primitive_instances,
            "branch_proxies":attached.branch_proxies,"pose_nodes":attached.pose_nodes,"volume_instances":attached.volume_instances,
            "volume_assets":attached.volume_assets,"source_vertices_shared":attached.source_vertices_shared,
            "resident_vertices_shared":attached.resident_vertices_shared,"source_triangles_shared":attached.source_triangles_shared,
            "shared_geometry_bytes":attached.shared_geometry_bytes,"expanded_source_bytes":attached.expanded_source_bytes,
            "packed_volume_bytes":attached.packed_volume_bytes},
        "resident_scene":{"instances":scene.instances.len(),"instance_records_bytes":instance_bytes,
            "meshes":scene.meshes.len(),"cluster_geometry_payload_bytes":geometry_bytes,"part_attachments":scene.part_attachments.len(),
            "pose_descriptors":scene.part_wind.len(),"packed_volume_payload_bytes":scene.voxel_forest.as_ref().map_or(0,|f|f.byte_len())},
        "inventory_scope":"CPU scene/upload payload counts, not total GPU memory. Source instance residency is O(placements*trees); geometry is shared. Expanded source bytes are arithmetic counterfactuals, not allocated meshes. GPU scratch, render targets, textures and scheduler buffers are additional.",
        "aggregate_candidate_errors":candidate_errors,
        "candidate_error_scope":"Cooker sampled optical model against its finer source medium, not ground-truth rendered opacity or a conservative perceptual bound; capture metrics are separate.",
        "part_wind_lod":true,"source_baseline":"Aggregate selection disabled: mapped source full CLOD error zero; unmapped geometry and shadows use the explicitly reported --shadow-lod setting in both modes."});
    Ok((scene, bounds, report))
}

fn views(args: &Args, bounds: [Vec3; 2], distance: f32) -> Vec<ViewRect> {
    let target = (bounds[0] + bounds[1]) * 0.5;
    (0..args.views)
        .map(|i| {
            let angle = 0.33 + i as f32 * std::f32::consts::FRAC_PI_2;
            let direction = Vec3::new(angle.sin(), 0.10, angle.cos()).normalize();
            let (w, h) = if args.views == 4 {
                (args.size[0] / 2, args.size[1] / 2)
            } else {
                (args.size[0], args.size[1])
            };
            ViewRect {
                cam: Camera {
                    pos: target + direction * distance,
                    target,
                    fov_y: 40.0f32.to_radians(),
                    near: 0.05,
                },
                x: if args.views == 4 {
                    (i as u32 % 2) * w
                } else {
                    0
                },
                y: if args.views == 4 {
                    (i as u32 / 2) * h
                } else {
                    0
                },
                w,
                h,
            }
        })
        .collect()
}

fn view_report(view: &ViewRect) -> Value {
    json!({"viewport":[view.x,view.y,view.w,view.h],"position":view.cam.pos.to_array(),"target":view.cam.target.to_array(),
        "fov_y":view.cam.fov_y,"near":view.cam.near,
        "pixels_per_metre_at_target":view.h as f32/(2.0*view.cam.pos.distance(view.cam.target)*(view.cam.fov_y*0.5).tan())})
}

fn roi(view: &ViewRect, bounds: [Vec3; 2]) -> metrics::Rect {
    let vp = Mat4::perspective_infinite_reverse_rh(
        view.cam.fov_y,
        view.w as f32 / view.h as f32,
        view.cam.near,
    ) * Mat4::look_at_rh(view.cam.pos, view.cam.target, Vec3::Y);
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    // Bounds are rest-pose diagnostics, expanded by 1 m for branch wind. This
    // is an image-comparison crop, not a conservative animation/culling bound.
    for p in corners([bounds[0] - Vec3::ONE, bounds[1] + Vec3::ONE]) {
        let clip = vp * p.extend(1.0);
        if clip.w <= 0.0 {
            return metrics::Rect {
                x: view.x,
                y: view.y,
                w: view.w,
                h: view.h,
            };
        }
        let n = clip.truncate() / clip.w;
        let pixel = Vec2::new(
            (n.x + 1.0) * 0.5 * view.w as f32,
            (1.0 - n.y) * 0.5 * view.h as f32,
        );
        lo = lo.min(pixel);
        hi = hi.max(pixel);
    }
    let x = lo.x.floor().clamp(0.0, view.w.saturating_sub(1) as f32) as u32;
    let y = lo.y.floor().clamp(0.0, view.h.saturating_sub(1) as f32) as u32;
    let right = hi.x.ceil().clamp((x + 1) as f32, view.w as f32) as u32;
    let bottom = hi.y.ceil().clamp((y + 1) as f32, view.h as f32) as u32;
    metrics::Rect {
        x: view.x + x,
        y: view.y + y,
        w: right - x,
        h: bottom - y,
    }
}

fn wind(renderer: &Renderer, gpu: &Gpu, time: f64) -> Result<(), String> {
    renderer.set_wind(
        gpu,
        &WindState {
            strength: 0.8,
            time_seconds: time,
            direction: Vec2::new(0.8, 0.6),
            ..Default::default()
        },
    )
}

fn profile_json(samples: &[GpuProfileSample]) -> Value {
    let mut passes: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for sample in samples {
        for &(name, _) in &sample.gpu_ms {
            passes.entry(name).or_default();
        }
    }
    for (name, values) in &mut passes {
        for sample in samples {
            values.push(
                sample
                    .gpu_ms
                    .iter()
                    .filter(|(n, _)| n == name)
                    .map(|(_, ms)| *ms)
                    .sum(),
            );
        }
    }
    json!({"covered_interval_ms":metrics::summarize(&samples.iter().map(|s|s.covered_interval_ms).collect::<Vec<_>>()),
        "covered_union_ms":metrics::summarize(&samples.iter().map(|s|s.covered_union_ms).collect::<Vec<_>>()),
        "root_scope_sum_ms":metrics::summarize(&samples.iter().map(|s|s.instrumented_gpu_ms).collect::<Vec<_>>()),
        "passes":passes.iter().map(|(name,values)|json!({"name":name,"ms":metrics::summarize(values)})).collect::<Vec<_>>(),
        "samples":samples.iter().map(|s|json!({"frame_id":s.frame_id,"render_sequence":s.render_sequence,
            "covered_interval_ms":s.covered_interval_ms,"covered_union_ms":s.covered_union_ms,"root_scope_sum_ms":s.instrumented_gpu_ms,
            "gpu_ms":s.gpu_ms,"view_pass_ms":s.view_pass_ms()})).collect::<Vec<_>>()})
}

fn benchmark(
    gpu: &Gpu,
    renderer: &Renderer,
    light: &Lighting,
    views: &[ViewRect],
    args: &Args,
    first_id: u64,
) -> Result<Value, String> {
    renderer.set_async_gpu_profiling(gpu, false)?;
    gpu.wait();
    let _ = renderer.drain_gpu_profiles(gpu);
    renderer.reset_capture_sampling();
    renderer.invalidate_shadow();
    let output = Renderer::output_texture(gpu, args.size[0], args.size[1]);
    for i in 0..args.warmup {
        wind(renderer, gpu, args.time + i as f64 * args.step)?;
        renderer.try_render_into(gpu, views, light, &output)?;
    }
    gpu.wait();
    renderer.set_async_gpu_profiling(gpu, true)?;
    let before = renderer.drain_gpu_profiles(gpu);
    if !before.supported {
        return Err("GPU timestamp queries unsupported; no benchmark result invented".into());
    }
    let mut samples = Vec::new();
    let mut pending = before.pending;
    let started = Instant::now();
    for i in 0..args.samples {
        // Bound pending timestamp slots below the four-slot ring. Polling may
        // throttle CPU submission; it is outside the GPU timestamp interval.
        let deadline = Instant::now() + Duration::from_secs(120);
        while pending >= 3 {
            let drain = renderer.drain_gpu_profiles(gpu);
            pending = drain.pending;
            samples.extend(drain.samples);
            if Instant::now() > deadline {
                return Err("GPU timestamp drain timed out".into());
            }
            if pending >= 3 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        wind(
            renderer,
            gpu,
            args.time + (args.warmup + i) as f64 * args.step,
        )?;
        renderer.set_gpu_profile_frame_id(first_id + i as u64);
        renderer.try_render_into(gpu, views, light, &output)?;
        let drain = renderer.drain_gpu_profiles(gpu);
        pending = drain.pending;
        samples.extend(drain.samples);
    }
    renderer.set_async_gpu_profiling(gpu, false)?;
    gpu.wait();
    let after = renderer.drain_gpu_profiles(gpu);
    samples.extend(after.samples);
    samples.sort_by_key(|s| s.frame_id);
    if samples.len() != args.samples
        || after.pending != 0
        || after.dropped != before.dropped
        || after.errors != before.errors
        || samples
            .iter()
            .enumerate()
            .any(|(i, s)| s.frame_id != first_id + i as u64)
    {
        return Err(format!(
            "incomplete GPU sample window: {}/{} samples, {} pending, {} new drops, {} new errors",
            samples.len(),
            args.samples,
            after.pending,
            after.dropped.saturating_sub(before.dropped),
            after.errors.saturating_sub(before.errors)
        ));
    }
    Ok(
        json!({"warmup_frames_excluded":args.warmup,"measured_frames":args.samples,"fixed_camera":true,
        "wind_time_start":args.time+args.warmup as f64*args.step,"wind_seconds_per_frame":args.step,
        "shadow_policy":"Normal renderer invalidation/cache behavior during deterministic wind; no forced redraw in the measured window.",
        "scope":"GPU timestamp coverage of this offscreen native renderer; other GPU workloads are not controlled. Warmup, image/debug readback and CPU preparation excluded. CPU submission is paced at three pending async slots. Not presentation, whole-frame FPS, hardware utilization or a gameplay benchmark. Root pass sums may overlap; do not add them to covered interval.",
        "submission_and_drain_wall_seconds_not_gpu_time":started.elapsed().as_secs_f64(),
        "dropped_samples":after.dropped-before.dropped,"profile_errors":after.errors-before.errors,
        "gpu":profile_json(&samples)}),
    )
}

struct Capture {
    image: RgbaImage,
    opacity: Vec<Option<Vec<f32>>>,
    report: Value,
}

fn capture(
    gpu: &Gpu,
    renderer: &Renderer,
    scene: &Scene,
    light: &Lighting,
    views: &[ViewRect],
    args: &Args,
    path: &Path,
) -> Result<Capture, String> {
    renderer.set_async_gpu_profiling(gpu, false)?;
    wind(renderer, gpu, args.time)?;
    renderer.reset_capture_sampling();
    renderer.invalidate_shadow();
    let output = Renderer::output_texture(gpu, args.size[0], args.size[1]);
    let warmup = if args.taa { 32 } else { 2 };
    for _ in 0..warmup {
        renderer.try_render_into(gpu, views, light, &output)?;
    }
    let (pixels, stats) =
        renderer.try_render_views(gpu, views, light, args.size[0], args.size[1], &[])?;
    let image =
        RgbaImage::from_raw(args.size[0], args.size[1], pixels).ok_or("invalid capture length")?;
    image.save(path).map_err(|e| e.to_string())?;
    let layers = renderer.voxel_layers_rgba16(gpu)?;
    let volume_diagnostics=format!("{:?}",renderer.voxel_diagnostics(gpu)?);
    let volume_count = scene.voxel_forest.as_ref().map_or(0, |f| f.instances.len());
    let mut diagnostics = Vec::new();
    let mut opacity = Vec::new();
    for (view_index, view) in views.iter().enumerate() {
        let selected = renderer.debug_visible_clusters(gpu, view_index, None)?;
        let mut owners = BTreeSet::new();
        let mut triangles = 0u64;
        let mut thin_triangles = 0u64;
        for &[owner, cluster] in &selected.pairs {
            if scene.instances[owner as usize].category() != GeometryCategory::Trees {
                continue;
            }
            owners.insert(owner);
            let c = &scene.clusters[cluster as usize];
            triangles += u64::from(c.tri_count);
            if scene.materials[c.material as usize].flags & THIN_FOLIAGE != 0 {
                thin_triangles += u64::from(c.tri_count);
            }
        }
        let mut decision_counts = [0u64; 3];
        let mut levels = BTreeMap::<u32, u64>::new();
        let mut cuts = Vec::new();
        for choice in selected.foliage_selection.iter().take(volume_count) {
            if let Some(n) = decision_counts.get_mut(choice[0] as usize) {
                *n += 1;
            }
            if choice[0] == 1 {
                *levels.entry(choice[1]).or_default() += 1;
            }
            if choice[0] == 0 && choice[3] != 0 {
                cuts.push(f64::from(f32::from_bits(choice[2])));
            }
        }
        let alpha = (|| -> Result<Vec<f32>, String> {
            let pixels = renderer.debug_geometry_owner_pixels(
                gpu,
                view_index,
                Some(selected.render_sequence),
            )?;
            if pixels.input_size != [view.w, view.h] {
                return Err("opacity comparison requires native input resolution".into());
            }
            let mut alpha: Vec<_> = pixels
                .owners
                .iter()
                .map(|&owner| {
                    if owner != u32::MAX
                        && scene.instances[owner as usize].category() == GeometryCategory::Trees
                    {
                        1.0
                    } else {
                        0.0
                    }
                })
                .collect();
            let (w, h, bytes) = layers
                .get(view_index)
                .ok_or("missing per-view volume alpha layer")?;
            if [*w, *h] != pixels.input_size || bytes.len() != alpha.len() * 8 {
                return Err("volume layer/owner mask resolution mismatch".into());
            }
            for (a, volume) in alpha.iter_mut().zip(metrics::rgba16f_alpha(bytes)?) {
                *a = 1.0 - (1.0 - *a) * (1.0 - volume);
            }
            Ok(alpha)
        })();
        let (alpha, alpha_report) = match alpha {
            Ok(alpha) => {
                let mask = RgbaImage::from_fn(view.w, view.h, |x, y| {
                    let a = (alpha[(y * view.w + x) as usize] * 255.0).round() as u8;
                    Rgba([a, a, a, 255])
                });
                let filename = format!(
                    "{}-view{view_index}-opacity.png",
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("capture")
                );
                mask.save(args.out.join(&filename))
                    .map_err(|e| e.to_string())?;
                (
                    Some(alpha),
                    json!({"available":true,"file":filename,"scope":"Native opaque Trees category union fractional foreground volume alpha; no shadows, TAA or output reconstruction."}),
                )
            }
            Err(error) => (None, json!({"available":false,"reason":error})),
        };
        opacity.push(alpha);
        diagnostics.push(json!({"view":view_index,"render_sequence":selected.render_sequence,
            "tree_submitted_owners":owners.len(),"tree_submitted_triangles":triangles,"thin_foliage_submitted_triangles":thin_triangles,
            "selected_source_groups":decision_counts[0],"selected_volume_groups":decision_counts[1],"offscreen_groups":decision_counts[2],
            "selected_volume_max_mips":levels,"source_cut_errors_world_m":metrics::summarize(&cuts),"opacity":alpha_report,
            "selection_scope":"Actual per-view GPU decisions, not rerun CPU LOD. Volume selection is not ray visibility; group/mip counts are not raster triangles or unique sampled cells."}));
    }
    Ok(Capture {
        image,
        opacity,
        report: json!({"file":path.file_name().and_then(|s|s.to_str()),"wind_time":args.time,
        "taa":args.taa,"fixed_pose_warmup_frames":warmup,"triangles":stats.triangles,"submitted_vertices":stats.submitted_vertices,
        "shadow_triangles":stats.shadow_triangles,"shadow_submitted_vertices":stats.shadow_submitted_vertices,
        "views":diagnostics,"volume_diagnostics":volume_diagnostics,"scope":"Stationary camera and frozen wind pose. TAA=1 accumulates 32 stationary frames; this is not temporal motion acceptance. All readbacks occur after the measured window."}),
    })
}

fn save_pair(source: &RgbaImage, auto: &RgbaImage, path: &Path) -> Result<(), String> {
    let mut pair = RgbaImage::new(source.width(), source.height() / 2);
    for (i, image) in [source, auto].into_iter().enumerate() {
        let thumb = image::imageops::resize(
            image,
            source.width() / 2,
            source.height() / 2,
            image::imageops::FilterType::Lanczos3,
        );
        image::imageops::replace(
            &mut pair,
            &thumb,
            i as i64 * i64::from(source.width() / 2),
            0,
        );
    }
    pair.save(path).map_err(|e| e.to_string())
}

fn run(args: Args) -> Result<(), String> {
    if args.out.as_os_str().is_empty() || args.out.exists() {
        return Err("--out must be a fresh directory".into());
    }
    if let Ok(value) = std::env::var("KERN_DIAGNOSTIC_HIDE") {
        if value != "none" {
            return Err("fixture requires KERN_DIAGNOSTIC_HIDE unset or none; category exclusions invalidate the comparison".into());
        }
    }
    let (scene, bounds, inventory) = build_scene(&args)?;
    let mut light = Lighting {
        taa: if args.taa { 1.0 } else { 0.0 },
        near_shadows: if args.near_shadows { 1.0 } else { 0.0 },
        sharpen: 0.0,
        // Explicit source cuts keep tree camera geometry exact. This separate
        // ordinary renderer CLOD setting bounds triangle shadow cost in both modes.
        lod_pixels: args.shadow_lod,
        gi: 1.0,
        fog_density: 0.0,
        fog_start: 10000.0,
        fog_end: 20000.0,
        ..Default::default()
    };
    light.set("sun_elevation=36")?;
    light.set("sun_azimuth=135")?;
    let gpu = Gpu::new()?;
    let info = gpu.summary();
    if !info.timestamps {
        return Err("GPU timestamps unavailable; fixture requires measured samples".into());
    }
    let renderer = Renderer::new(&gpu, &scene, &light);
    std::fs::create_dir_all(&args.out).map_err(|e| e.to_string())?;
    let mut report = json!({"schema":1,"fixture":"vegetation_forest_lod","inventory":inventory,
        "adapter":{"name":info.name,"backend":info.backend,"driver":info.driver,"device_type":info.device_type,"timestamps":info.timestamps},
        "size":args.size,"views":args.views,"taa":args.taa,"mode":args.mode,
        "requested_renderer_options":{"brick_visibility":std::env::var("KERN_VOXEL_BRICK_VISIBILITY").unwrap_or_default(),
            "global_intervals":std::env::var("KERN_VOXEL_GLOBAL_INTERVALS").unwrap_or_default(),
            "scope":"Requested options; inspect renderer startup log for admission fallback and actual enablement."},
        "lighting":{"sun_elevation_degrees":36,"sun_azimuth_degrees":135,"gi":light.gi,"fog_density":light.fog_density,"lod_pixels":light.lod_pixels,"shadow_focus_policy":"Diagnostic main and near sun maps stay centered on the forest in all views. This deliberately stresses source shadow geometry; it does not reproduce camera-relative gameplay shadow scheduling."},
        "comparison_scope":"Matched source and auto use one Scene/Renderer, identical cooked assets, poses, materials, lights and cameras. Full-detail source is the oracle. Sampled optical policy is explicitly enabled for this experiment; images and CPU metrics are evidence, not a quality pass.",
        "timing_scope":"GPU timestamp windows with animated deterministic wind; concurrent GPU workloads are not controlled by this fixture. Warmup, shader setup, cooking, diagnostic and image readback excluded. No interactive-game performance claim.",
        "cases":[]});
    let report_path = args.out.join("report.json");
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut frame_id = 1_000_000u64;
    for (case, distance) in args.distances.iter().copied().enumerate() {
        let views = views(&args, bounds, distance);
        renderer.set_lighting_focus(&views.iter().map(|v| v.cam.pos).collect::<Vec<_>>())?;
        let sun_center = Vec3::new(
            (bounds[0].x + bounds[1].x) * 0.5,
            0.0,
            (bounds[0].z + bounds[1].z) * 0.5,
        );
        renderer.set_sun_shadow_focus(&vec![sun_center; views.len()])?;
        let mut captures = Vec::new();
        let mut modes = Vec::new();
        for mode in ["source", "auto"] {
            if args.mode != "both" && args.mode != mode {
                continue;
            }
            eprintln!(
                "case {case}: {distance} m, {} views, {mode}, {} measured frames",
                args.views, args.samples
            );
            renderer.set_aggregate_lod_enabled(mode == "auto");
            // Read counters before timing too: an overflowing source shadow
            // pass must fail instead of producing deceptively fast samples.
            wind(&renderer,&gpu,args.time)?;
            renderer.try_render_texture_profiled(&gpu,&views,&light,args.size[0],args.size[1])?;
            let timing = benchmark(&gpu, &renderer, &light, &views, &args, frame_id)?;
            frame_id += 10_000;
            let path = args
                .out
                .join(format!("d{case:02}-{distance:.0}m-{mode}.png"));
            let capture = capture(&gpu, &renderer, &scene, &light, &views, &args, &path)?;
            modes.push(json!({"mode":mode,"timing":timing,"capture":capture.report}));
            captures.push(capture);
        }
        let comparison = if captures.len() == 2 {
            let (source, auto) = (&captures[0], &captures[1]);
            save_pair(
                &source.image,
                &auto.image,
                &args
                    .out
                    .join(format!("d{case:02}-source-left-auto-right.png")),
            )?;
            metrics::difference_image(&source.image, &auto.image)?
                .save(
                    args.out
                        .join(format!("d{case:02}-absolute-rgb-difference-x4.png")),
                )
                .map_err(|e| e.to_string())?;
            let mut per_view = Vec::new();
            for (i, view) in views.iter().enumerate() {
                per_view.push(json!({"view":i,
                    "full_view":metrics::compare(&source.image,&auto.image,metrics::Rect{x:view.x,y:view.y,w:view.w,h:view.h})?,
                    "rest_bounds_plus_1m_crop":metrics::compare(&source.image,&auto.image,roi(view,bounds))?,
                    "native_opacity":match (&source.opacity[i],&auto.opacity[i]) {
                        (Some(a),Some(b))=>metrics::opacity(a,b)?,_=>json!({"available":false,"reason":"see capture opacity diagnostics; no colour-threshold mask substituted"})}}));
            }
            json!(per_view)
        } else {
            Value::Null
        };
        report["cases"].as_array_mut().unwrap().push(json!({"distance_m":distance,"views":views.iter().map(view_report).collect::<Vec<_>>(),"modes":modes,"comparison":comparison}));
        std::fs::write(
            &report_path,
            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    println!("wrote {}", report_path.display());
    Ok(())
}

fn main() {
    if let Err(error) = args().and_then(run) {
        eprintln!("vegetation_forest_lod: {error}");
        std::process::exit(1);
    }
}
