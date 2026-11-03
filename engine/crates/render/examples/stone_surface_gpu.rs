//! Bounded native stone-material comparison; run serially with no game on the GPU.
//! --mode legacy|textured|parallax --out FRESH_DIR [--frames 120] [--views 1|4]
//! [--size 1920x1080] [--camera field|close|grazing|detail|far|gallery] [--captures]
//! --geometry quarried|weathered (default quarried); hold this fixed for material comparisons.
//! Captures/readbacks are outside timings.
use glam::{Affine3A, Mat4, Quat, Vec3};
use render::debug_settings::DebugRenderOptions;
use render::gpu_profile::GpuProfileSample;
use render::scene::{Material, ROCK};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}, time::Instant};

const WARMUP: u32 = 30;
const PATCH_Z: f32 = -7.5;
const GALLERY_X: f32 = 80.;

struct Options {
    out: PathBuf,
    mode: String,
    geometry: String,
    frames: u32,
    views: usize,
    size: [u32; 2],
    camera: String,
    captures: bool,
}
impl Options {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let (mut out, mut mode) = (None, None);
        let mut geometry = String::from("quarried");
        let (mut frames, mut views, mut size) = (120, 1, [1920, 1080]);
        let (mut camera, mut captures) = (String::from("field"), false);
        while let Some(arg) = args.next() {
            if arg == "--captures" { captures = true; continue; }
            let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
            match arg.as_str() {
                "--out" => out = Some(PathBuf::from(value)),
                "--mode" => mode = Some(value),
                "--geometry" => geometry = value,
                "--frames" => frames = value.parse().map_err(|_| "invalid --frames")?,
                "--views" => views = value.parse().map_err(|_| "invalid --views")?,
                "--camera" => camera = value,
                "--size" => {
                    let (w, h) = value.split_once('x').ok_or("--size expects WIDTHxHEIGHT")?;
                    size = [w.parse().map_err(|_| "invalid width")?, h.parse().map_err(|_| "invalid height")?];
                }
                _ => return Err(format!("unknown option {arg}")),
            }
        }
        if !["quarried", "weathered"].contains(&geometry.as_str()) { return Err("invalid --geometry".into()); }
        let mode = mode.ok_or("--mode legacy|textured|parallax is required")?;
        if !["legacy", "textured", "parallax"].contains(&mode.as_str()) { return Err("invalid --mode".into()); }
        if !(1..=3600).contains(&frames) || ![1, 4].contains(&views) { return Err("frames must be 1..=3600; views must be 1 or 4".into()); }
        if size[0] < 320 || size[1] < 180 || size[0] > 7680 || size[1] > 4320 { return Err("size must be between 320x180 and 7680x4320".into()); }
        if !["field", "close", "grazing", "detail", "far", "gallery"].contains(&camera.as_str()) { return Err("invalid --camera".into()); }
        Ok(Self { geometry, out: out.ok_or("--out FRESH_DIR is required")?, mode, frames, views, size, camera, captures })
    }
}

fn scene(textured: bool, weathered: bool) -> (Scene, u32) {
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.65, 0.75, 0.95, 1.0]; 128]));
    // Populate both modes identically, including resources; only the material
    // layer selects legacy arithmetic or the shared, mipmapped stone surface.
    scene.textures.push(render::stone_surface::texture());
    // Eight quarry shapes include both dressed and rough seeds. Wear reuses
    // each seed, while restrained tint variation remains identical in A/B.
    let mut meshes = [[0usize; 8]; 3];
    for (wear, damage) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        for seed in 0..8 {
            let shade = [0.92, 1.0, 0.96, 1.08][seed % 4];
            let material = scene.add_material(Material {
                tint: [0.60f32, 0.57, 0.53].map(|c| ((c * shade + 0.055) / 1.055).powf(2.4)),
                layer: if textured { 0 } else { -1 }, roughness: 0.85, flags: ROCK, scale: damage, metal: 0.,
            });
            meshes[wear][seed] = scene.add_mesh(&(if weathered { render::weathered_block::weathered_block(material, damage, seed as u32) } else { render::quarried_block::quarried_block(material, damage, seed as u32) }).mesh());
        }
    }
    for z in 0..32 {
        for x in 0..32 {
            let k = (x * 13 + z * 7) % 3;
            let pos = Vec3::new((x as f32 - 15.5) * 1.52, 0.25, (z as f32 - 15.5) * 1.52);
            scene.add_instance(meshes[k][(x + z * 3) % 8], Affine3A::from_scale_rotation_translation(Vec3::new(1.5, 0.5, 1.5), Quat::IDENTITY, pos));
        }
    }
    // One flat central face gives a common physical patch at every camera
    // distance, while its fractures and the tiled field exercise geometric
    // LOD. Centre its guaranteed flat face core on the existing sample patch.
    let wall_owner = scene.instances.len() as u32;
    scene.add_instance(meshes[0][0], Affine3A::from_scale_rotation_translation(Vec3::new(8., 6., 1.), Quat::IDENTITY, Vec3::new(0., 2.4, -8.)));
    // A rotated, stretched damaged block makes mapping mistakes visible.
    scene.add_instance(meshes[2][3], Affine3A::from_scale_rotation_translation(Vec3::new(3.1, 2., 1.2), Quat::from_rotation_y(0.63), Vec3::new(-7., 1.5, -6.)));
    // A separate, human-scale masonry view avoids judging the material only
    // on the deliberately oversized measurement wall. Its position keeps the
    // benchmark field outside the gallery camera's frustum and vice versa.
    for row in 0..5 {
        let mut left = GALLERY_X - 4. + (row % 2) as f32 * 0.48;
        for column in 0..8 {
            let width = [0.92, 1.06, 0.97, 1.10, 0.88, 1.04, 1.0, 1.03][(column + row) % 8];
            let seed = (row * 5 + column * 3) % 8;
            let age = (column + row * 3) % 7;
            let wear = if age == 0 { 2 } else if age == 3 { 1 } else { 0 };
            let pos = Vec3::new(left + width * 0.5, 0.3 + row as f32 * 0.6, 0.);
            scene.add_instance(meshes[wear][seed], Affine3A::from_scale_rotation_translation(Vec3::new(width, 0.6, 0.6), Quat::IDENTITY, pos));
            left += width + 0.012;
        }
    }
    for row in 0..3 { for column in 0..10 {
        let pos = Vec3::new(GALLERY_X + (column as f32 - 4.5) * 1.02, -0.06, 0.6 + row as f32 * 0.72);
        scene.add_instance(meshes[0][(row * 3 + column) % 8], Affine3A::from_scale_rotation_translation(Vec3::new(1., 0.12, 0.7), Quat::IDENTITY, pos));
    }}
    scene.add_instance(meshes[1][5], Affine3A::from_scale_rotation_translation(Vec3::new(1.05, 0.64, 0.66), Quat::from_rotation_y(-0.24), Vec3::new(GALLERY_X + 0.45, 0.32, 1.55)));
    (scene, wall_owner)
}

fn lighting() -> Lighting {
    Lighting {
        sun_dir: Vec3::new(0.65, 0.42, 0.63).normalize(), sun_lux: 60_000., sky_lux: 12_000.,
        ev100: 13.5, taa: 0., sharpen: 0., gi: 0., contact_occlusion: 0., reflection_occlusion: 0.,
        near_shadows: 0., fog_density: 0., fog_start: 1e6, fog_end: 2e6, fog_culling: false,
        lod_pixels: 1., ..Default::default()
    }
}

fn gallery_lighting() -> Lighting {
    Lighting { sun_dir: Vec3::new(0.82, 0.52, 0.22).normalize(), sky_lux: 9_000., ..lighting() }
}

fn camera(name: &str) -> Camera {
    let target = Vec3::new(0., 2.4, PATCH_Z);
    let (pos, target) = match name {
        "close" => (Vec3::new(0., 2.4, -5.), target),
        "detail" => (Vec3::new(1.5, 2.4, -7.0), target),
        "grazing" => (Vec3::new(7., 2.4, -5.), target),
        "far" => (Vec3::new(0., 7., 65.), target),
        "gallery" => (Vec3::new(GALLERY_X + 2.6, 1.4, 4.2), Vec3::new(GALLERY_X, 1.05, 0.4)),
        _ => (Vec3::new(0., 6., 15.), Vec3::new(0., 0.8, -5.)),
    };
    Camera { pos, target, fov_y: 55f32.to_radians(), near: 0.05 }
}

fn views(name: &str, count: usize, size: [u32; 2]) -> Vec<ViewRect> {
    render::split_rects(count, size[0], size[1]).into_iter().enumerate().map(|(i, (x, y, w, h))| {
        let mut cam = camera(name);
        if count > 1 {
            let offset = Vec3::new((i % 2) as f32 * 0.5 - 0.25, (i / 2) as f32 * 0.16, 0.);
            cam.pos += offset; cam.target += offset;
        }
        ViewRect { cam, x, y, w, h }
    }).collect()
}

fn camera_json(v: &ViewRect) -> Value {
    json!({"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),"fov_y":v.cam.fov_y,
        "near":v.cam.near,"viewport":[v.x,v.y,v.w,v.h]})
}

fn summary(values: &[f64]) -> Value {
    if values.is_empty() { return Value::Null; }
    let mut sorted = values.to_vec(); sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    json!({"count":sorted.len(),"mean":sorted.iter().sum::<f64>()/sorted.len() as f64,
        "p50":percentile(0.5),"p95":percentile(0.95),"min":sorted[0],"max":sorted[sorted.len()-1]})
}

fn selected_geometry(gpu: &Gpu, renderer: &Renderer, scene: &Scene, count: usize) -> Result<Value, String> {
    let mut reports = Vec::new();
    for view in 0..count {
        let d = renderer.debug_visible_clusters(gpu, view, None)?;
        let mut triangles = 0u64;
        let mut depths: BTreeMap<u32, [u64; 2]> = BTreeMap::new();
        let mut owners = BTreeSet::new();
        for &[owner, cluster] in &d.pairs {
            let c = scene.clusters.get(cluster as usize).ok_or("invalid GPU cluster ID")?;
            if owner as usize >= scene.instances.len() { return Err("invalid GPU instance ID".into()); }
            owners.insert(owner); triangles += u64::from(c.tri_count);
            let entry = depths.entry(c.depth).or_default(); entry[0] += 1; entry[1] += u64::from(c.tri_count);
        }
        reports.push(json!({"view":view,"selected_clusters":d.pairs.len(),"selected_instances":owners.len(),
            "selected_triangles":triangles,"lod_depth_clusters_triangles":depths,
            "input_size":d.input_size,"output_size":d.output_size}));
    }
    Ok(json!(reports))
}

fn save_image(path: &Path, size: [u32; 2], rgba: &[u8]) -> Result<(), String> {
    render::write_png(&path.with_extension("png"), size[0], size[1], rgba)?;
    let rgb: Vec<_> = rgba.chunks_exact(4).flat_map(|p| p[..3].iter().copied()).collect();
    let file = std::fs::File::create(path.with_extension("jpg")).map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 94)
        .encode(&rgb, size[0], size[1], image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())
}

// Equal-area world samples of a 1.4 m square on the reference wall. Reporting
// repeated far pixels separately avoids pretending all 1024 samples are independent.
fn patch_pixels(v: &ViewRect) -> Result<Vec<(u32, u32)>, String> {
    let matrix = Mat4::perspective_infinite_reverse_rh(v.cam.fov_y, v.w as f32 / v.h as f32, v.cam.near)
        * Mat4::look_at_rh(v.cam.pos, v.cam.target, Vec3::Y);
    let mut pixels = Vec::new();
    for y in 0..32 { for x in 0..32 {
        let p = Vec3::new(-0.7 + (x as f32 + 0.5) / 32. * 1.4, 1.7 + (y as f32 + 0.5) / 32. * 1.4, PATCH_Z);
        let clip = matrix * p.extend(1.);
        if clip.w <= 0. { return Err("reference patch is behind camera".into()); }
        let ndc = clip.truncate() / clip.w;
        let px = (ndc.x * 0.5 + 0.5) * v.w as f32;
        let py = (0.5 - ndc.y * 0.5) * v.h as f32;
        if px < 0. || py < 0. || px >= v.w as f32 || py >= v.h as f32 { return Err("reference patch is outside capture".into()); }
        pixels.push((v.x + px as u32, v.y + py as u32));
    }}
    Ok(pixels)
}

fn patch_mean(image: &[u8], width: u32, pixels: &[(u32, u32)]) -> [f64; 3] {
    let mut sum = [0.; 3];
    for &(x, y) in pixels {
        let offset = ((y * width + x) * 4) as usize;
        for c in 0..3 { sum[c] += f64::from(image[offset + c]); }
    }
    sum.map(|v| v / pixels.len() as f64)
}

fn captures(gpu: &Gpu, renderer: &Renderer, scene: &Scene, wall: u32, options: &Options, light: &Lighting) -> Result<Value, String> {
    let gallery_view = views("gallery", 1, options.size);
    let gallery_light = gallery_lighting();
    let (gallery_image, _) = renderer.try_render_views(gpu, &gallery_view, &gallery_light, options.size[0], options.size[1], &[])?;
    save_image(&options.out.join("gallery-lit"), options.size, &gallery_image)?;
    let gallery = json!({"image":"gallery-lit.png","preview":"gallery-lit.jpg","view":camera_json(&gallery_view[0]),
        "stone_dimensions_metres":[1.0,0.6,0.6],"courses":5,"stones_per_course":8,"quarry_seeds":8,
        "near_stone_dimensions_metres":[1.05,0.64,0.66],"sun_direction":gallery_light.sun_dir.to_array(),
        "sun_lux":gallery_light.sun_lux,"sky_lux":gallery_light.sky_lux,"ev100":gallery_light.ev100,
        "selected_geometry":selected_geometry(gpu,renderer,scene,1)?});
    let mut reports = Vec::new();
    let mut means = Vec::new();
    for name in ["close", "grazing", "far"] {
        let view = views(name, 1, options.size);
        let pixels = patch_pixels(&view[0])?;
        let mut images = Vec::new();
        for (component, debug) in [("lit", 0.), ("albedo", 2.), ("normal", 11.), ("source-normal", 12.)] {
            let mut component_light = light.clone(); component_light.debug = debug;
            let (image, stats) = renderer.try_render_views(gpu, &view, &component_light, options.size[0], options.size[1], &pixels)?;
            if stats.picks.len() != pixels.len() || stats.picks.iter().any(|p| p.is_none_or(|hit| hit.0 != wall)) {
                return Err(format!("{name}/{component}: reference patch does not belong entirely to the wall"));
            }
            save_image(&options.out.join(format!("{name}-{component}")), options.size, &image)?;
            images.push(image);
        }
        let mean = patch_mean(&images[1], options.size[0], &pixels); means.push(mean);
        let mut changed = 0usize; let mut delta = 0f64;
        let unique: BTreeSet<_> = pixels.iter().copied().collect();
        for &(x, y) in &unique {
            let offset = ((y * options.size[0] + x) * 4) as usize;
            let a = &images[2][offset..offset + 3]; let b = &images[3][offset..offset + 3];
            if a != b { changed += 1; }
            delta += a.iter().zip(b).map(|(&a, &b)| (f64::from(a) - f64::from(b)).abs()).sum::<f64>();
        }
        reports.push(json!({"camera":name,"view":camera_json(&view[0]),"world_patch_samples":pixels.len(),
            "unique_patch_pixels":unique.len(),"patch_albedo_display_rgb8_mean":mean,
            "mapped_vs_source_normal_changed_pixels":changed,
            "mapped_vs_source_normal_mean_absolute_rgb8_delta":delta/(unique.len()*3) as f64,
            "selected_geometry":selected_geometry(gpu,renderer,scene,1)?}));
    }
    Ok(json!({"gallery":gallery,"views":reports,"patch_world_bounds":[[-0.7,1.7,PATCH_Z],[0.7,3.1,PATCH_Z]],
        "far_minus_close_albedo_display_rgb8":std::array::from_fn::<_,3,_>(|c| means[2][c]-means[0][c]),
        "interpretation":"Equal-area samples on the same physical wall, validated by GPU owner picks. RGB statistics are fixed-exposure, tonemapped diagnostic pixels, not linear albedo or angular normal errors. Far samples may share pixels. No image-quality pass threshold is inferred."}))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    std::fs::write(path, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let options = Options::parse()?;
    if options.out.exists() { return Err("--out must be a fresh directory".into()); }
    std::fs::create_dir_all(&options.out).map_err(|e| e.to_string())?;
    let (mut scene, wall) = scene(options.mode != "legacy", options.geometry == "weathered");
    if options.mode == "textured" {
        for texture in &mut scene.textures {
            if let Some(normal) = &mut texture.normal {
                for pixel in normal.pixels_mut() { pixel[2] = 255; }
            }
        }
    }
    let mut digest = Sha256::new();
    let geometry_bytes: [&[u8]; 4] = [bytemuck::cast_slice(&scene.vertices), bytemuck::cast_slice(&scene.clusters), bytemuck::cast_slice(&scene.instances), bytemuck::cast_slice(&scene.tris)];
    for bytes in geometry_bytes { digest.update(bytes); }
    let geometry_hash = format!("{:x}", digest.finalize());
    let gpu = Gpu::new()?;
    if !gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY) { return Err("timestamp queries unavailable; no timing claim".into()); }
    let light = if options.camera == "gallery" { gallery_lighting() } else { lighting() };
    let renderer = Renderer::new(&gpu, &scene, &light);
    renderer.set_debug_options(DebugRenderOptions { bloom_enabled: false, ..Default::default() })?;
    let view = views(&options.camera, options.views, options.size);
    let output = Renderer::output_texture(&gpu, options.size[0], options.size[1]);
    for _ in 0..WARMUP { renderer.try_render_into(&gpu, &view, &light, &output)?; gpu.wait(); }
    renderer.set_async_gpu_profiling(&gpu, true)?;
    let mut samples: BTreeMap<u64, GpuProfileSample> = BTreeMap::new();
    let (mut submit, mut wall_ms) = (Vec::new(), Vec::new());
    let (mut dropped, mut errors) = (0, 0);
    for id in 0..options.frames {
        renderer.set_gpu_profile_frame_id(u64::from(id));
        let start = Instant::now();
        renderer.try_render_into(&gpu, &view, &light, &output)?;
        submit.push(start.elapsed().as_secs_f64() * 1000.);
        gpu.wait(); wall_ms.push(start.elapsed().as_secs_f64() * 1000.);
        let drain = renderer.drain_gpu_profiles(&gpu);
        dropped = drain.dropped; errors = drain.errors;
        for sample in drain.samples { samples.insert(sample.frame_id, sample); }
    }
    gpu.wait();
    let drain = renderer.drain_gpu_profiles(&gpu);
    dropped = dropped.max(drain.dropped); errors = errors.max(drain.errors);
    let pending = drain.pending;
    for sample in drain.samples { samples.insert(sample.frame_id, sample); }
    renderer.set_async_gpu_profiling(&gpu, false)?;
    let geometry = selected_geometry(&gpu, &renderer, &scene, options.views)?;
    save_image(&options.out.join("benchmark"), options.size, &gpu.read_rgba8(&output))?;
    let mut passes: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut raw = Vec::new();
    for sample in samples.values() {
        for &(name, ms) in &sample.gpu_ms { passes.entry(name).or_default().push(ms); }
        raw.push(json!({"frame_id":sample.frame_id,"gpu_pass_ms":sample.gpu_ms,
            "covered_union_ms":sample.covered_union_ms,"instrumented_gpu_ms":sample.instrumented_gpu_ms,
            "covered_interval_ms":sample.covered_interval_ms,"output_size":sample.output_size,"views":sample.views}));
    }
    let passes: BTreeMap<_, _> = passes.into_iter().map(|(name, v)| (name, summary(&v))).collect();
    let (capture_report, capture_error) = if options.captures {
        match captures(&gpu, &renderer, &scene, wall, &options, &light) {
            Ok(report) => (report, None),
            Err(error) => (json!({"error":error}), Some(error)),
        }
    } else { (Value::Null, None) };
    let report = json!({"mode":options.mode,"scope":"Native offscreen material fixture, serialized completion; no game, HUD, audio or reconstruction. Not presentation FPS. Warmup, source construction, captures and diagnostics excluded from timings.",
        "adapter":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"output_size":options.size,"reconstruction":"native","sample_count":1,
        "benchmark_camera":options.camera,"views":view.iter().map(camera_json).collect::<Vec<_>>(),"warmup_frames":WARMUP,"measured_frames":options.frames,
        "gpu_sample_coverage":{"collected":samples.len(),"requested":options.frames,"dropped":dropped,"errors":errors,"pending":pending},
        "geometry":options.geometry,"geometry_sha256":geometry_hash,"instances":scene.instances.len(),"meshes":scene.meshes.len(),"stored_clusters":scene.clusters.len(),
        "stored_lod_triangles":scene.clusters.iter().map(|c|u64::from(c.tri_count)).sum::<u64>(),
        "source_mesh_triangles":scene.clusters.iter().filter(|c|c.own_error==0.).map(|c|u64::from(c.tri_count)).sum::<u64>(),
        "root_mesh_triangles":scene.clusters.iter().filter(|c|c.parent_error>1e30).map(|c|u64::from(c.tri_count)).sum::<u64>(),
        "stone_repeat_metres":render::stone_surface::REPEAT_METRES,"selected_geometry":geometry,"lod_pixels":light.lod_pixels,
        "lighting":{"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100,"taa":light.taa,"bloom":false,"gi":light.gi,"near_shadows":light.near_shadows},
        "cpu_submit_ms":summary(&submit),"cpu_serialized_wall_ms":summary(&wall_ms),"gpu_pass_ms":passes,
        "gpu_covered_union_ms":summary(&samples.values().map(|s|s.covered_union_ms).collect::<Vec<_>>()),
        "gpu_instrumented_sum_ms":summary(&samples.values().map(|s|s.instrumented_gpu_ms).collect::<Vec<_>>()),
        "gpu_covered_interval_ms":summary(&samples.values().map(|s|s.covered_interval_ms).collect::<Vec<_>>()),
        "captures":capture_report});
    write_json(&options.out.join("samples.json"), &json!({"gpu":raw,"cpu_submit_ms":submit,"cpu_serialized_wall_ms":wall_ms}))?;
    write_json(&options.out.join("report.json"), &report)?;
    if let Some(error) = capture_error { return Err(error); }
    if samples.len() != options.frames as usize || errors != 0 || dropped != 0 || pending != 0 {
        return Err("incomplete GPU sample coverage; inspect report.json before using timings".into());
    }
    println!("stone surface {}: {} measured frames at {}x{}, {} views; {}", options.mode, options.frames, options.size[0], options.size[1], options.views, options.out.display());
    Ok(())
}

fn main() { if let Err(error) = run() { eprintln!("{error}"); std::process::exit(1); } }
