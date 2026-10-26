//! Explicit native-4K vegetation draw-distance accounting, never an automatic GPU test.
//! Run with no concurrent game. Writes JSON/CSV, no full-resolution image readbacks.
use glam::{Affine3A, Mat4, Quat, Vec2, Vec3};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::foliage_assembly::{self, Limits, Parameters};
use render::foliage_representation::Policy;
use render::foliage_voxel::{BakeConfig, ProjectionMeasure};
use render::scene::{GeometryCategory, Heightfield, Material};
use render::vegetation_accounting::{Accounting, ViewCounts};
use render::vegetation_asset::{AttachLimits, CookedPlant, PlantInstance, SceneVegetation};
use render::voxel_forest::VoxelViewDiagnostics;
use render::wind::WindState;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const DISTANCES: [f32; 11] = [5., 15., 30., 50., 75., 100., 150., 250., 500., 1000., 2000.];
const WARMUP: usize = 3;
const SAMPLES: usize = 8;

fn scene(allow_volumes: bool, meadow: bool) -> Result<Scene, String> {
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.7, 0.8, 1., 1.]; 128]));
    let material = scene.add_material(Material { tint: [0.19, 0.17, 0.12], layer: -1,
        roughness: 1., flags: 0, scale: 1., metal: 0. });
    let vertices = [[-100., -0.01, 100.], [100., -0.01, 100.], [-100., -0.01, -100.], [100., -0.01, -100.]]
        .map(|pos| Vertex { pos, normal: Vec3::Y.to_array(), uv: [0.; 2] });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices,
        indices: &[0, 1, 2, 1, 3, 2], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield { origin: [-100., -100.], cell: 2., w: 101, h: 101,
        heights: vec![-0.01; 10201] };
    let config = BakeConfig { projection_measure: ProjectionMeasure::ClosedSurfaceEntries,
        cell_size: 0.025, levels: 4, samples_per_cell: 4, max_samples: 4_000_000,
        max_voxels: 131072, max_texture_texels: 0, ..Default::default() };
    let plants = [if meadow {Parameters::meadow_grass(101)} else {Parameters::grass(101)}, Parameters::conifer_twig(909)].into_iter()
        .map(|p| CookedPlant::cook(&foliage_assembly::generate(p, Limits::default())?, config))
        .collect::<Result<Vec<_>, String>>()?;
    let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default())?;
    for i in 0..6 {
        let root = Mat4::from_scale_rotation_translation(
            Vec3::new(0.85 + i as f32 * 0.04, 1.05, 0.91), Quat::from_rotation_y(i as f32 * 0.71),
            Vec3::new((i % 3) as f32 * 0.8 - 0.8, 0., (i / 3) as f32 * 0.8 - 0.4));
        builder.append(&plants[i % 2], PlantInstance { id: i as u64, generation: 1, root,
            category: if i % 2 == 0 { GeometryCategory::Grass } else { GeometryCategory::Trees } })?;
    }
    builder.finish(Policy { allow_unvalidated_optics: allow_volumes, ..Default::default() })?;
    Ok(scene)
}

fn views(distances: &[f32]) -> Vec<ViewRect> {
    let split = distances.len() == 4;
    let (w, h) = if split { (WIDTH / 2, HEIGHT / 2) } else { (WIDTH, HEIGHT) };
    distances.iter().enumerate().map(|(i, &d)| ViewRect {
        // Identical orientation isolates viewport resolution. Mixed-distance row
        // exercises independent per-view selection; no extrapolated times.
        cam: Camera { pos: Vec3::new(0., 0.65 + d * 0.07, d), target: Vec3::new(0., 0.35, 0.),
            fov_y: 50f32.to_radians(), near: 0.05 },
        x: if split { i as u32 % 2 * w } else { 0 }, y: if split { i as u32 / 2 * h } else { 0 }, w, h,
    }).collect()
}

fn counts_json(counts: &ViewCounts) -> Value {
    json!({"view": counts.view_index, "frame_id": counts.frame_id, "render_sequence": counts.render_sequence,
        "viewport": counts.viewport, "input_size": counts.input_size, "output_size": counts.output_size,
        "categories": counts.categories.iter().map(|c| json!({"category": c.category.name(),
            "instances": c.raster.instances, "clusters": c.raster.clusters, "triangles": c.raster.triangles,
            "padded_vertices": c.raster.submitted_vertices, "source_groups": c.selected_source_groups,
            "volume_groups": c.selected_volume_groups, "selected_maximum_mips": c.selected_mips.iter().map(|m|
                json!({"max_mip": m.max_mip, "volume_instances": m.volume_instances,
                    "instance_occupied_cells": m.instance_occupied_cells, "unique_packed_levels": m.unique_packed_levels,
                    "unique_resident_occupied_cells": m.unique_resident_occupied_cells})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()})
}

fn volume_json(d: &VoxelViewDiagnostics) -> Value {
    json!({"traced_rays": d.traced_rays, "total_steps": d.total_steps, "volume_samples": d.volume_samples,
        "occupied_cell_samples": d.occupied_cell_samples, "aabb_interval_tests": d.aabb_interval_tests,
        "active_tiles": d.active_tiles, "scheduled_pixels": d.scheduled_pixels,
        "max_steps": d.max_steps_used, "max_ray_candidates": d.max_ray_candidates,
        "max_active_volumes": d.max_active_volumes, "candidate_overflow_tiles": d.candidate_overflow_tiles,
        "step_overflow_rays": d.step_overflow_rays, "ray_candidate_overflow_rays": d.ray_candidate_overflow_rays,
        "active_cache_overflow_rays": d.active_cache_overflow_rays})
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 0 { (values[n / 2 - 1] + values[n / 2]) * 0.5 } else { values[n / 2] }
}

/// Source-only optical probe. Each actual visible owner is mapped back to its
/// plant; full-frame averages cannot hide a disappearing small plant.
fn source_lod_visual(out: &std::path::Path, scene: &Scene, gpu: &Gpu, light: &Lighting, meadow:bool) -> Result<(),String> {
    if light.lod_pixels<=0. {return Err("source LOD visual probe requires a positive error budget".into());}
    let mut owner_plants = vec![None; scene.instances.len()];
    for binding in &scene.foliage_representations {
        let group = scene.foliage_source_groups.iter().find(|g|g.owner==binding.owner).ok_or("missing source group")?;
        for &owner in &group.members { owner_plants[owner as usize]=Some(binding.group.key.instance as usize); }
    }
    let renderer=Renderer::new(gpu,scene,light);
    let mut report=Vec::new(); let mut failed=Vec::new();
    for height in [1080u32,2160] { for angle in [0f32,0.8] { for distance in [5f32,15.,30.,50.,100.,150.] { for wind in [0f32,0.7,1.9] {
        let width=height/9*16;
        let view=ViewRect {cam:Camera {pos:Vec3::new(angle.sin()*distance,0.65+distance*0.07,angle.cos()*distance),
            target:Vec3::new(0.,0.35,0.),fov_y:50f32.to_radians(),near:0.05},x:0,y:0,w:width,h:height};
        renderer.set_wind(gpu,&WindState {strength:0.8,time_seconds:f64::from(wind),direction:Vec2::new(0.8,0.6),..Default::default()})?;
        let mut captures=Vec::new();
        for error in [0.,light.lod_pixels] {
            let lighting=Lighting {lod_pixels:error,..light.clone()};
            let (rgba,stats)=renderer.try_render_views(gpu,&[view],&lighting,width,height,&[])?;
            let owner_capture=renderer.debug_geometry_owner_pixels(gpu,0,None)?;
            if owner_capture.input_size!=[width,height] {return Err("unexpected diagnostic resolution".into());}
            let owners=owner_capture.owners.into_iter().map(|owner| if owner==u32::MAX {None} else {owner_plants.get(owner as usize).copied().flatten()}).collect::<Vec<_>>();
            captures.push((rgba,owners,stats.triangles));
        }
        let (reference,reference_owners,reference_triangles)=&captures[0];
        let (candidate,candidate_owners,candidate_triangles)=&captures[1];
        let mut plants=Vec::new();
        for plant in 0..6 {
            let mut a=0u64;let mut b=0u64;let mut intersection=0u64;let mut union=0u64;let mut sum=0u64;let mut roi_count=0u64;
            let mut roi=vec![false;(width*height)as usize];
            for i in 0..roi.len() {
                let ar=reference_owners[i]==Some(plant);let br=candidate_owners[i]==Some(plant);
                a+=u64::from(ar);b+=u64::from(br);intersection+=u64::from(ar&&br);union+=u64::from(ar||br);
                if ar||br {
                    let x=i as u32%width;let y=i as u32/width;
                    for yy in y.saturating_sub(2)..=(y+2).min(height-1) {for xx in x.saturating_sub(2)..=(x+2).min(width-1) {roi[(yy*width+xx)as usize]=true;}}
                }
            }
            let mut reference_luma=0u64;let mut candidate_luma=0u64;
            for (i,&inside) in roi.iter().enumerate() {if inside {roi_count+=1;for channel in 0..3 {sum+=reference[i*4+channel].abs_diff(candidate[i*4+channel])as u64;reference_luma+=reference[i*4+channel]as u64;candidate_luma+=candidate[i*4+channel]as u64;}}}
            if a>=8 && b==0 {failed.push(format!("resolved plant {plant} disappeared at {height}/{angle}/{distance}/{wind}"));}
            // Well-resolved plants have a relative coverage gate. Tiny plants
            // are pixel-quantized; allow three pixels, but never disappearance.
            let coverage_tolerance=if a>=100 {a as f64*0.10} else {3f64.max(a as f64*0.10)};
            if a>=8 && a.abs_diff(b)as f64>coverage_tolerance {failed.push(format!("plant {plant} coverage {a}->{b} at {height}/{angle}/{distance}/{wind}"));}
            let roi_error=if roi_count==0 {0.} else {sum as f64/(roi_count*3)as f64};
            if a>=8 && roi_error>if a>=100 {2.} else {5.} {failed.push(format!("plant {plant} RGB error {roi_error} at {height}/{angle}/{distance}/{wind}"));}
            plants.push(json!({"plant":plant,"reference_pixels":a,"candidate_pixels":b,"intersection":intersection,"union":union,
                "roi_pixels":roi_count,"roi_rgb_mae_255":if roi_count==0 {0.} else {sum as f64/(roi_count*3)as f64},
                "roi_reference_rgb_mean":if roi_count==0 {0.} else {reference_luma as f64/(roi_count*3)as f64},
                "roi_candidate_rgb_mean":if roi_count==0 {0.} else {candidate_luma as f64/(roi_count*3)as f64}}));
        }
        if candidate_triangles>reference_triangles {failed.push("LOD increased triangles".into());}
        if distance==150. && candidate_triangles>=reference_triangles {failed.push("far source LOD did not reduce any triangles".into());}
        let exact=reference==candidate;
        if distance==5. && !exact {failed.push(format!("near image changed {height}/{angle}/{wind}"));}
        let name=format!("{height}-{angle}-{distance}-{wind}");
        if wind==0.7 && [5.,50.,150.].contains(&distance) {
            for (suffix,pixels) in [("source",reference),("lod",candidate)] {
                let image=image::RgbaImage::from_raw(width,height,pixels.clone()).ok_or("invalid pixels")?;
                image::DynamicImage::ImageRgba8(image).to_rgb8().save(out.join(format!("{name}-{suffix}.jpg"))).map_err(|e|e.to_string())?;
            }
        }
        report.push(json!({"input_size":[width,height],"angle_radians":angle,"distance_m":distance,"wind_time":wind,
            "source_triangles":reference_triangles,"lod_triangles":candidate_triangles,"byte_identical":exact,"plants":plants}));
        println!("source LOD {name}: {reference_triangles} -> {candidate_triangles}, exact={exact}");
    }}}}
    std::fs::write(out.join("source-lod-visual.json"),serde_json::to_vec_pretty(&json!({"passed":failed.is_empty(),"failures":failed,
        "source_error_pixels":light.lod_pixels,"cases":report,
        "optical_gates":{"near_5m_byte_identical":true,"resolved_plant_min_pixels":8,"well_resolved_min_pixels":100,"coverage_relative_tolerance":0.10,"small_plant_absolute_pixel_tolerance":3,"roi_rgb_mae_255_max":2,"small_plant_roi_rgb_mae_255_max":5,"must_reduce_at_150m":true},
        "grass_recipe":if meadow {"meadow_grass_v1"}else{"reference_grass_tuft"},
        "scope":"Same six plants, source-only policy, actual visibility owner masks, two orientations and three wind poses. Single native1080p/4K views; four-view scheduling tested separately. RGB errors use union masks dilated by two pixels. Acceptance is bounded to this fixture/sun/material, not universal optical equivalence. No timing from readback frames."})).unwrap()).map_err(|e|e.to_string())?;
    if !failed.is_empty() {return Err(format!("{} source LOD validation failures; see report",failed.len()));}
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(&args.windows(2).find(|a| a[0] == "--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists() { return Err("output already exists".into()); }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let source_visual = args.iter().any(|s| s == "--source-lod-visual");
    let source_error: f32 = args.windows(2).find(|a| a[0] == "--source-error-pixels")
        .map(|a| a[1].parse()).transpose().map_err(|_| "invalid source error")?.unwrap_or(1.);
    if !source_error.is_finite() || source_error < 0. { return Err("invalid source error".into()); }
    let distances: Vec<f32> = args.windows(2).find(|a| a[0] == "--distances")
        .map(|a| a[1].split(',').map(str::parse).collect()).transpose().map_err(|_| "invalid distances")?
        .unwrap_or_else(|| DISTANCES.to_vec());
    if distances.iter().any(|d| !d.is_finite() || *d <= 0.) { return Err("invalid distance".into()); }
    let view_counts: Vec<usize> = args.windows(2).find(|a| a[0] == "--views")
        .map(|a| a[1].split(',').map(str::parse).collect()).transpose().map_err(|_| "invalid views")?
        .unwrap_or_else(|| vec![1,4]);
    if view_counts.iter().any(|n| ![1,4].contains(n)) { return Err("views must be 1 or 4".into()); }
    let meadow = args.iter().any(|s|s=="--meadow");
    let scene = scene(!source_visual, meadow)?;
    let accounting = Accounting::new(&scene)?;
    let inventory = accounting.inventory();
    let source_triangles: u64 = inventory.categories.iter().map(|c| c.source_triangles_full_detail).sum();
    // Deliberate recipe regression ceilings: changing asset complexity requires
    // reviewing this fixture, not silently spending extra triangles per plant.
    if source_triangles != 6960 || scene.part_attachments.len() != 315 || scene.part_wind.len() != 30 {
        return Err(format!("six-plant recipe budget changed: {source_triangles} triangles, {} primitives, {} poses",
            scene.part_attachments.len(), scene.part_wind.len()));
    }
    let gpu = Gpu::new()?;
    let lighting = Lighting { taa: 0., sharpen: 0., lod_pixels: source_error, gi: 0., contact_occlusion: 0.,
        reflection_occlusion: 0., near_shadows: 0., fog_density: 0., fog_start: 10000., fog_end: 20000.,
        sun_dir: Vec3::new(0.4, 0.65, -0.6).normalize(), ..Default::default() };
    if source_visual { return source_lod_visual(&out, &scene, &gpu, &lighting, meadow); }
    let renderer = Renderer::new(&gpu, &scene, &lighting);
    renderer.set_wind(&gpu, &WindState { strength: 0.8, time_seconds: 1.9,
        direction: Vec2::new(0.8, 0.6), ..Default::default() })?;
    let mut cases = Vec::new();
    let mut csv = String::from("views,distance_m,view,grass_triangles,tree_triangles,clusters,padded_vertices,selected_volumes,selected_mip_cells,ray_steps,occupied_cell_samples,aabb_tests,frame_gpu_pass_sum_median_ms\n");
    let mut configurations: Vec<_> = view_counts.into_iter().flat_map(|n| distances.iter().map(move |&d| vec![d; n])).collect();
    if !args.iter().any(|s| s == "--no-mixed") { configurations.push(vec![5., 50., 250., 1000.]); }
    for distances in configurations {
        let views = views(&distances);
        let mut timings = Vec::new();
        let mut walls = Vec::new();
        let mut stats = FrameStats::default();
        for sample in 0..WARMUP + SAMPLES {
            let start = Instant::now();
            let (_, frame) = renderer.try_render_texture_profiled(&gpu, &views, &lighting, WIDTH, HEIGHT)?;
            let wall = start.elapsed().as_secs_f64() * 1000.;
            if sample >= WARMUP {
                let mut passes = BTreeMap::<String, f64>::new();
                for &(name, ms) in &frame.gpu_ms {
                    if !ms.is_finite() || ms < 0. { return Err("invalid GPU timestamps".into()); }
                    *passes.entry(name.into()).or_default() += ms;
                }
                if passes.is_empty() { return Err("GPU timestamps unavailable".into()); }
                timings.push(passes); walls.push(wall);
            }
            stats = frame;
        }
        // All debug mapping occurs after the measured interval, not inside samples.
        let diagnostics = renderer.voxel_diagnostics(&gpu)?.ok_or("missing volume counters")?;
        if diagnostics.len() != views.len() { return Err("stale volume view slots".into()); }
        let mut totals: Vec<_> = timings.iter().map(|p| p.values().sum()).collect();
        let gpu_ms = median(&mut totals);
        let mut rows = Vec::new();
        let mut vegetation_triangles = 0u64;
        let mut raster_all = 0u64;
        for (i, view) in views.iter().enumerate() {
            let capture = renderer.debug_visible_clusters(&gpu, i, None)?;
            let counts = accounting.view(&capture, stats.cluster_buckets)?;
            raster_all += capture.pairs.iter().map(|pair| scene.clusters[pair[1] as usize].tri_count as u64).sum::<u64>();
            let mut triangles = 0; let mut clusters = 0; let mut padded = 0; let mut volumes = 0; let mut cells = 0;
            for (c, limit) in counts.categories.iter().zip(&inventory.categories) {
                if c.raster.triangles > limit.source_triangles_full_detail { return Err("selected colour exceeded source triangle budget".into()); }
                triangles += c.raster.triangles; clusters += c.raster.clusters; padded += c.raster.submitted_vertices;
                volumes += c.selected_volume_groups;
                cells += c.selected_mips.iter().map(|m| m.instance_occupied_cells).sum::<u64>();
            }
            let d = &diagnostics[i];
            if d.candidate_overflow_tiles > 0 || d.step_overflow_rays > 0 || d.ray_candidate_overflow_rays > 0 || d.active_cache_overflow_rays > 0 {
                return Err(format!("volume budget overflow at {:?} view {i}: {d:?}", distances));
            }
            if d.total_steps > u64::from(d.traced_rays) * 4096 || (volumes == 0 && (d.total_steps != 0 || d.occupied_cell_samples != 0)) {
                return Err("invalid volume work counters".into());
            }
            if distances[i] >= 1000. && triangles != 0 { return Err("far fixture still draws source vegetation triangles".into()); }
            vegetation_triangles += triangles;
            let mut row = counts_json(&counts);
            row["distance_parameter_m"] = json!(distances[i]);
            row["camera_to_target_m"] = json!(view.cam.pos.distance(view.cam.target));
            row["volume_work"] = volume_json(d);
            row["tile_volume_selection_checks"] = json!(u64::from(view.w.div_ceil(16)) * u64::from(view.h.div_ceil(16)) * scene.voxel_forest.as_ref().unwrap().instances.len() as u64);
            rows.push(row);
            csv.push_str(&format!("{},{},{},{},{},{},{},{},{},{},{},{},{}\n", views.len(), distances[i], i,
                counts.categories[0].raster.triangles, counts.categories[1].raster.triangles, clusters, padded,
                volumes, cells, d.total_steps, d.occupied_cell_samples, d.aabb_interval_tests, gpu_ms));
        }
        if raster_all != u64::from(stats.triangles) { return Err("GPU cluster readback disagrees with frame triangle counter".into()); }
        if args.iter().any(|s| s == "--capture") {
            let (pixels,_) = renderer.try_render_views(&gpu,&views,&lighting,WIDTH,HEIGHT,&[])?;
            image::save_buffer(out.join(format!("capture-{}-{}.png",views.len(),distances.iter().map(|d|d.to_string()).collect::<Vec<_>>().join("-"))), &pixels, WIDTH, HEIGHT, image::ColorType::Rgba8).map_err(|e|e.to_string())?;
        }
        cases.push(json!({"distances_m": distances, "views": rows, "vegetation_triangles_all_views": vegetation_triangles,
            "all_colour_triangles": stats.triangles, "all_colour_padded_vertices": stats.submitted_vertices,
            "shadow_cache_triangles": stats.shadow_triangles, "shadow_vertices_submitted_this_frame": stats.shadow_submitted_vertices,
            "shadow_note": "Shared-map cache geometry count can be nonzero on cache reuse. Submitted vertices indicate new raster work. Counts include fixture floor; source shadows are retained by voxel colour LOD.",
            "gpu_pass_sum_median_ms": gpu_ms, "synchronous_metrics_wall_median_ms": median(&mut walls), "gpu_pass_samples_ms": timings}));
        println!("{} views {:?}: {vegetation_triangles} vegetation triangles, instrumented GPU {gpu_ms:.3} ms", views.len(), distances);
    }
    let report = json!({"passed": true, "adapter": gpu.info.name, "output": [WIDTH, HEIGHT], "reconstruction": "native", "fov_degrees": 50,
        "source_error_pixels": source_error,
        "grass_recipe":if meadow {"meadow_grass_v1"}else{"reference_grass_tuft"},
        "empty_ceiling": std::env::var("KERN_VOXEL_EMPTY_CEILING").unwrap_or_else(|_|"1".into()),
        "plants": 6, "branch_poses": scene.part_wind.len(), "primitive_placements": scene.part_attachments.len(),
        "source_triangles_per_view": source_triangles, "unique_shared_triangles": inventory.shared_triangles_full_detail,
        "resident_sparse_cell_records": inventory.resident_sparse_cell_records, "resident_levels": inventory.resident_packed_levels,
        "warmup_frames_per_case": WARMUP, "measured_frames_per_case": SAMPLES, "cases": cases,
        "scope": "Fixed six-plant native-4K accounting fixture, three grass and three conifer twigs. Counts use actual GPU colour selection, not CPU guesses. Four views are each 1920x1080. No box meshes: cells are sampled volume data. Selected maximum-mip cells count resident potential with instance multiplicity; cell samples count repeated actual reads. Neither is a unique visible voxel count. Instrumented synchronous GPU pass sums are not full gameplay FPS. No optical-equivalence claim.",
        "budget_gates": {"source_triangles": 6960, "primitive_placements": 315, "branch_poses": 30,
            "selected_source_never_exceeds_full_source": true, "all_volume_overflows_zero": true,
            "far_1000m_source_vegetation_triangles_zero": true, "frame_triangles_match_gpu_pairs": true}});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(out.join("distances.csv"), csv).map_err(|e| e.to_string())?;
    Ok(())
}

fn main() { if let Err(e) = run() { eprintln!("vegetation_distance_budget: {e}"); std::process::exit(1); } }
