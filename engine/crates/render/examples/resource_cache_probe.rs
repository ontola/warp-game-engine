//! Exclusive-GPU resource identity regression, not a performance benchmark.
//! Run after stopping the game: resource_cache_probe --out FRESH_DIRECTORY.
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::frame::ResourceCacheStats;
use render::present::Presenter;
use render::scene::{Material, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const W: u32 = 96;
const H: u32 = 64;

fn main() {
    if let Err(error) = run() { eprintln!("resource cache probe: {error}"); std::process::exit(1); }
}

fn fixture() -> Scene {
    let mut scene = Scene::default();
    scene.sky = Some((8, 4, vec![[16.0, 8.0, 4.0, 1.0]; 32]));
    scene.sky_physical = true;
    let material = scene.add_material(Material { tint: [0.2, 0.5, 0.3], layer: -1,
        roughness: 1.0, flags: TWO_SIDED, scale: 1.0, metal: 0.0 });
    let vertices = [[-1.0, -1.0, -4.0], [1.0, -1.0, -4.0], [1.0, 1.0, -4.0], [-1.0, 1.0, -4.0]]
        .map(|pos| Vertex { pos, normal: [0.0, 0.0, 1.0], uv: [0.0; 2] });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices,
        indices: &[0, 1, 2, 0, 2, 3], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene
}

fn view(w: u32, h: u32) -> ViewRect {
    ViewRect { cam: Camera { pos: Vec3::ZERO, target: Vec3::NEG_Z,
        fov_y: 55f32.to_radians(), near: 0.05 }, x: 0, y: 0, w, h }
}

fn presentation_target(gpu: &Gpu, w: u32, h: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("cache probe presentation"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] })
}

fn pixels(gpu: &Gpu, texture: &wgpu::Texture) -> Result<Vec<u8>, String> {
    let bytes = gpu.read_rgba8(texture);
    if bytes.len() != (texture.width() * texture.height() * 4) as usize
        || !bytes.chunks_exact(4).all(|p| p[3] == 255)
        || !bytes.chunks_exact(4).any(|p| p[..3].iter().any(|&v| v > 16)) {
        return Err("output was not fully written, or retained a black/uninitialized target".into());
    }
    Ok(bytes)
}

fn same_pixels(label: &str, actual: &[u8], expected: &[u8], tolerance: u8) -> Result<(), String> {
    if actual.len() != expected.len() { return Err(format!("{label}: pixel lengths differ")); }
    let error = actual.iter().zip(expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
    if error > tolerance { return Err(format!("{label}: maximum pixel error {error} exceeds {tolerance}")); }
    Ok(())
}

fn record(rows: &mut Vec<Value>, label: &str, before: ResourceCacheStats, after: ResourceCacheStats) {
    rows.push(json!({"case": label, "hits": after.hits - before.hits,
        "misses": after.misses - before.misses, "retained_entries": after.entries}));
}

fn reused(label: &str, before: ResourceCacheStats, after: ResourceCacheStats) -> Result<(), String> {
    if after.misses != before.misses || after.hits <= before.hits {
        return Err(format!("{label}: expected only cache hits: {before:?} -> {after:?}"));
    }
    Ok(())
}

fn replaced(label: &str, before: ResourceCacheStats, after: ResourceCacheStats) -> Result<(), String> {
    if after.misses <= before.misses {
        return Err(format!("{label}: replacement identity did not rebuild resources: {before:?} -> {after:?}"));
    }
    Ok(())
}

fn frames(gpu: &Gpu, renderer: &Renderer, views: &[ViewRect], light: &Lighting,
    target: &wgpu::Texture, count: usize) -> Result<(), String> {
    for _ in 0..count { renderer.try_render_into(gpu, views, light, target)?; }
    Ok(())
}

fn steady(gpu: &Gpu, renderer: &Renderer, views: &[ViewRect], light: &Lighting,
    target: &wgpu::Texture, label: &str, rows: &mut Vec<Value>) -> Result<(), String> {
    // Two warmup frames cover both TAA histories; four measured frames revisit
    // each resource identity twice. No readback occurs inside those frames.
    frames(gpu, renderer, views, light, target, 2)?;
    let before = renderer.resource_cache_stats();
    frames(gpu, renderer, views, light, target, 4)?;
    let after = renderer.resource_cache_stats();
    reused(label, before, after)?;
    if after.entries > 2 + 14 * views.len() {
        return Err(format!("{label}: resource retention exceeded the per-view bound: {after:?}"));
    }
    pixels(gpu, target)?;
    record(rows, label, before, after);
    Ok(())
}

fn exercise(gpu: &Gpu) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    let mut light = Lighting { sun_lux: 0.0, sky_lux: 40.0, ev100: 4.0,
        gi: 0.0, near_shadows: 0.0, taa: 0.0, sharpen: 0.0, fog_density: 0.0,
        contact_occlusion: 0.0, reflection_occlusion: 0.0, ..Default::default() };
    let renderer = Renderer::new(gpu, &fixture(), &light);
    renderer.set_debug_options(render::debug_settings::DebugRenderOptions {
        bloom_enabled: false, exhaust_enabled: false, wheel_particles_enabled: false,
        ..Default::default()
    })?;
    let single = [view(W, H)];
    let first = Renderer::output_texture(gpu, W, H);
    steady(gpu, &renderer, &single, &light, &first, "native steady frames", &mut rows)?;
    let baseline = pixels(gpu, &first)?;

    // Dimensions and format are equal, but the output identity is different.
    // A stale tone bind writes first again and leaves replacement uninitialized.
    let replacement = Renderer::output_texture(gpu, W, H);
    let before = renderer.resource_cache_stats();
    frames(gpu, &renderer, &single, &light, &replacement, 1)?;
    let after = renderer.resource_cache_stats();
    replaced("same-size output", before, after)?;
    same_pixels("same-size output", &pixels(gpu, &replacement)?, &baseline, 0)?;
    record(&mut rows, "same-size output replacement", before, after);

    // Uniform writes must keep bindings, while visibly affecting the new target
    // and leaving the still-live old output untouched.
    light.ev100 += 3.0;
    let before = renderer.resource_cache_stats();
    frames(gpu, &renderer, &single, &light, &replacement, 1)?;
    let after = renderer.resource_cache_stats();
    reused("exposure uniform write", before, after)?;
    let darker = pixels(gpu, &replacement)?;
    if darker == baseline { return Err("exposure update did not affect the replacement output".into()); }
    same_pixels("old renderer target remains untouched", &pixels(gpu, &first)?, &baseline, 0)?;
    record(&mut rows, "uniform content update", before, after);

    let presenter = Presenter::new(gpu, wgpu::TextureFormat::Rgba8Unorm);
    let displayed = presentation_target(gpu, W, H);
    let display_view = displayed.create_view(&Default::default());
    presenter.present(gpu, &first, &display_view, None, None)?;
    let before = presenter.resource_cache_stats();
    presenter.present(gpu, &first, &display_view, None, None)?;
    let after = presenter.resource_cache_stats();
    reused("presenter steady source", before, after)?;
    same_pixels("presenter first source", &pixels(gpu, &displayed)?, &baseline, 1)?;
    record(&mut rows, "presenter steady source", before, after);

    let before = presenter.resource_cache_stats();
    presenter.present(gpu, &replacement, &display_view, None, None)?;
    let after = presenter.resource_cache_stats();
    replaced("presenter same-size source", before, after)?;
    same_pixels("presenter replacement source", &pixels(gpu, &displayed)?, &darker, 1)?;
    if after.entries != 2 { return Err(format!("presenter source cache grew: {after:?}")); }
    record(&mut rows, "presenter same-size source replacement", before, after);

    let other_display = presentation_target(gpu, W, H);
    let other_view = other_display.create_view(&Default::default());
    let before = presenter.resource_cache_stats();
    presenter.present(gpu, &replacement, &other_view, None, None)?;
    let after = presenter.resource_cache_stats();
    reused("presenter render-target replacement", before, after)?;
    same_pixels("presenter replacement target", &pixels(gpu, &other_display)?, &darker, 1)?;
    record(&mut rows, "presenter render-target replacement", before, after);
    // Switching back must use first's brighter pixels, and only the new target.
    presenter.present(gpu, &first, &other_view, None, None)?;
    same_pixels("presenter restored source", &pixels(gpu, &other_display)?, &baseline, 1)?;
    same_pixels("old presentation target remains untouched", &pixels(gpu, &displayed)?, &darker, 1)?;

    let resized_views = [view(80, 72)];
    let resized = Renderer::output_texture(gpu, 80, 72);
    let before = renderer.resource_cache_stats();
    frames(gpu, &renderer, &resized_views, &light, &resized, 1)?;
    let after = renderer.resource_cache_stats();
    replaced("resize", before, after)?;
    record(&mut rows, "resize", before, after);
    steady(gpu, &renderer, &resized_views, &light, &resized, "resized steady frames", &mut rows)?;
    let restored = Renderer::output_texture(gpu, W, H);
    steady(gpu, &renderer, &single, &light, &restored, "restored-size steady frames", &mut rows)?;
    same_pixels("restored-size image", &pixels(gpu, &restored)?, &darker, 0)?;

    light.taa = 1.0;
    renderer.reset_history();
    steady(gpu, &renderer, &single, &light, &restored, "TAA alternating histories", &mut rows)?;
    for generation in 0..2 {
        let before = renderer.resource_cache_stats();
        renderer.reset_history();
        frames(gpu, &renderer, &single, &light, &restored, 1)?;
        let after = renderer.resource_cache_stats();
        replaced("same-size TAA history reset", before, after)?;
        pixels(gpu, &restored)?;
        record(&mut rows, &format!("TAA history reset {generation}"), before, after);
        steady(gpu, &renderer, &single, &light, &restored, "TAA after history reset", &mut rows)?;
    }
    steady(gpu, &renderer, &resized_views, &light, &resized, "TAA resized histories", &mut rows)?;

    let split = [view(W, H), ViewRect { x: W, ..view(W, H) }];
    let split_output = Renderer::output_texture(gpu, W * 2, H);
    steady(gpu, &renderer, &split, &light, &split_output, "two independent TAA views", &mut rows)?;
    steady(gpu, &renderer, &single, &light, &restored, "view-count shrink", &mut rows)?;
    Ok(rows)
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(&args.windows(2).find(|a| a[0] == "--out").ok_or("--out FRESH_DIRECTORY required")?[1]);
    if out.exists() { return Err("fresh output directory required".into()); }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    // This process has not started GPU worker threads. Pin the allocation policy
    // under test instead of inheriting an interactive A/B experiment.
    unsafe {
        std::env::set_var("KERN_REUSE_SUN_TEXTURE", "1");
        std::env::set_var("KERN_ZERO_SUN_FAST_PATH", "1");
    }
    let gpu = Gpu::new()?;
    let uncaptured = Arc::new(Mutex::new(Vec::new()));
    let errors = uncaptured.clone();
    gpu.device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        errors.lock().unwrap().push(error.to_string());
    }));
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = exercise(&gpu);
    gpu.device.poll(wgpu::PollType::Wait { submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)) }).map_err(|e| format!("GPU completion: {e:?}"))?;
    let validation = pollster::block_on(scope.pop());
    if let Some(error) = validation { return Err(format!("GPU validation: {error}")); }
    if !uncaptured.lock().unwrap().is_empty() {
        return Err(format!("uncaptured GPU errors: {:?}", uncaptured.lock().unwrap()));
    }
    let rows = result?;
    let report = json!({"passed": true, "scope": "resource identity and image correctness; not FPS or DLSS validation",
        "adapter": gpu.info.name, "backend": format!("{:?}", gpu.info.backend),
        "validation_errors": 0, "cases": rows});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    println!("resource cache probe PASS: reuse, same-size replacement, uniform writes, presenter, resize, TAA reset and split views");
    Ok(())
}
