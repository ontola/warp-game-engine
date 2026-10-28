//! Serial, native-resolution A/B captures of flat and rounded blade normals.
//! cargo run --locked -p render --example meadow_shading_gpu -- --out FRESH_DIR
//! Add --aggregate to include the existing source/aggregate handoff and far view.
//! Images require visual review; mechanical equality does not establish quality.
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::debug_settings::DebugRenderOptions;
use render::foliage_assembly::MeadowBladeBudget;
use render::meadow_data::{MeadowField, MeadowGeometry};
use render::scene::{Heightfield, Material};
use render::wind::WindState;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const W: u32 = 1280;
const H: u32 = 720;
const SEED: u64 = 73;
const HABITAT_HALF: f32 = 4.5;

fn scene(geometry: MeadowGeometry) -> Result<Scene, String> {
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.65, 0.75, 0.95, 1.0]; 128]));
    let material = scene.add_material(Material {
        tint: [0.16, 0.12, 0.075],
        layer: -1,
        roughness: 1.0,
        flags: 0,
        scale: 1.0,
        metal: 0.0,
    });
    let vertices = [
        [-200., -0.015, 200.],
        [200., -0.015, 200.],
        [-200., -0.015, -200.],
        [200., -0.015, -200.],
    ]
    .map(|pos| Vertex {
        pos,
        normal: Vec3::Y.to_array(),
        uv: [0.; 2],
    });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 1, 2, 1, 3, 2],
        material,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield {
        origin: [-200.; 2],
        cell: 4.,
        w: 101,
        h: 101,
        heights: vec![-0.015; 10201],
    };
    let field = MeadowField {
        origin: [-HABITAT_HALF; 2],
        cell: HABITAT_HALF / 8.,
        width: 17,
        height: 17,
        samples: vec![[0., 1_000_000., 1., 0.]; 17 * 17],
        seed: SEED as u32,
        growth: 1.,
        dry_season: 0.,
        radius: 80.,
        fade_start: 60.,
        geometry,
    };
    field.validate()?;
    scene.meadow = Some(field);
    Ok(scene)
}

fn lighting() -> Lighting {
    Lighting {
        // Raking light from the camera side exposes the transverse shading
        // gradient; keep identical illumination/exposure in the A/B pair.
        sun_dir: Vec3::new(0.65, 0.42, 0.63).normalize(),
        sun_lux: 60_000.,
        sky_lux: 12_000.,
        ev100: 13.0,
        taa: 0.,
        sharpen: 0.,
        gi: 0.,
        contact_occlusion: 0.,
        reflection_occlusion: 0.,
        near_shadows: 0.,
        fog_density: 0.,
        fog_start: 1e6,
        fog_end: 2e6,
        fog_culling: false,
        lod_pixels: 0.,
        ..Default::default()
    }
}

fn views(aggregate: bool) -> Vec<(&'static str, ViewRect)> {
    let mut positions = vec![
        ("close", Vec3::new(1.4, 0.65, 2.0)),
        ("source-lod", Vec3::new(-4.0, 3.0, 32.0)),
    ];
    if aggregate {
        positions.extend([
            ("handoff", Vec3::new(-4.0, 7.0, 70.0)),
            ("distant", Vec3::new(-4.0, 11.0, 110.0)),
        ]);
    }
    positions
        .into_iter()
        .map(|(name, pos)| {
            (
                name,
                ViewRect {
                    cam: Camera {
                        pos,
                        target: Vec3::new(0., 0.15, 0.),
                        fov_y: 50f32.to_radians(),
                        near: 0.05,
                    },
                    x: 0,
                    y: 0,
                    w: W,
                    h: H,
                },
            )
        })
        .collect()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn save_image(out: &Path, name: &str, rgba: &[u8]) -> Result<(), String> {
    render::write_png(&out.join(format!("{name}.png")), W, H, rgba)?;
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    let file = std::fs::File::create(out.join(format!("{name}.jpg"))).map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 94)
        .encode(&rgb, W, H, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())
}

fn side_by_side(out: &Path, name: &str, flat: &[u8], rounded: &[u8]) -> Result<(), String> {
    let mut combined = Vec::with_capacity((W * H * 4) as usize);
    let images = [flat, rounded].map(|pixels| {
        let image = image::RgbaImage::from_raw(W, H, pixels.to_vec()).unwrap();
        image::imageops::resize(&image, W / 2, H / 2, image::imageops::FilterType::Lanczos3)
    });
    for y in 0..H / 2 {
        for image in &images {
            let start = (y * W / 2 * 4) as usize;
            combined.extend_from_slice(&image.as_raw()[start..start + (W / 2 * 4) as usize]);
        }
    }
    let rgb: Vec<u8> = combined
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    let file = std::fs::File::create(out.join(format!("{name}-flat-left-rounded-right.jpg")))
        .map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 94)
        .encode(&rgb, W, H / 2, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())
}

/// Preserve the actual source alpha bits; never infer coverage from lit RGB.
fn source_alpha(gpu: &Gpu, renderer: &Renderer) -> Result<Vec<u8>, String> {
    let layers = renderer.meadow_source_fixture_layers();
    if layers.len() != 1 {
        return Err("expected one source color/coverage layer".into());
    }
    let texture = &layers[0];
    if texture.format() != wgpu::TextureFormat::Rgba16Float
        || texture.width() != W
        || texture.height() != H
    {
        return Err("unexpected source layer format or dimensions".into());
    }
    let stride =
        (W * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rounded meadow source alpha readback"),
        size: u64::from(stride) * u64::from(H),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.wait();
    rx.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| e.to_string())?;
    let alpha = mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| {
            row[..(W * 8) as usize]
                .chunks_exact(8)
                .flat_map(|pixel| pixel[6..8].iter().copied())
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    Ok(alpha)
}

struct Capture {
    image: Vec<u8>,
    alpha: Vec<u8>,
    counts: Value,
}

fn capture(
    gpu: &Gpu,
    renderer: &Renderer,
    view: &ViewRect,
    light: &Lighting,
    time: f64,
) -> Result<Capture, String> {
    renderer.set_wind(
        gpu,
        &WindState {
            time_seconds: time,
            strength: 0.8,
            ..Default::default()
        },
    )?;
    let mut output = None;
    // Settle previous pose and renderer resources; no frame timing claim.
    for _ in 0..3 {
        output = Some(renderer.try_render_texture(gpu, std::slice::from_ref(view), light, W, H)?);
    }
    let diagnostics = renderer.meadow_diagnostics(gpu)?;
    let d = diagnostics
        .first()
        .ok_or("missing source draw diagnostics")?;
    if diagnostics.len() != 1 || d.visibility_invalid_ids != 0 {
        return Err("invalid source view diagnostics".into());
    }
    let counts = json!({"plants":d.plants,"triangles":d.triangles,
        "source_geometry_passes":d.source_geometry_passes,"submitted_source_triangles":d.submitted_source_triangles,
        "source_vertices_per_patch":d.source_vertices_per_patch,"lod_pixels":d.lod_pixels,
        "density":d.density,"size_scale":d.size_scale,"color_multiplier":d.color_multiplier,
        "visibility_enabled":d.visibility_enabled,"visibility_covered_pixels":d.visibility_covered_pixels,
        "visibility_unique_primitives":d.visibility_unique_primitives,"visibility_invalid_ids":d.visibility_invalid_ids,
        "aggregate_enabled":d.aggregate_enabled,"aggregate_plants":d.aggregate_plants,
        "aggregate_triangles":d.aggregate_triangles,"aggregate_transition_m":d.aggregate_transition_m,
        "aggregate_outer_fade_m":d.aggregate_outer_fade_m});
    Ok(Capture {
        image: gpu.read_rgba8(&output.unwrap()),
        alpha: source_alpha(gpu, renderer)?,
        counts,
    })
}

fn run(out: &Path, aggregate: bool) -> Result<Value, String> {
    let flat = MeadowGeometry::cook_with_blade_budget(SEED, 0., MeadowBladeBudget::Blades120)?;
    let rounded = MeadowGeometry::cook_with_blade_budget_and_rounded_normals(
        SEED,
        0.,
        MeadowBladeBudget::Blades120,
    )?;
    let mut normalized = rounded.clone();
    if normalized.vertices.len() != flat.vertices.len() {
        return Err("normal mode changed vertex count".into());
    }
    for (a, b) in normalized.vertices.iter_mut().zip(&flat.vertices) {
        a.normal = b.normal;
    }
    if normalized.vertices != flat.vertices
        || rounded.lod_ranges != flat.lod_ranges
        || rounded.lod_errors != flat.lod_errors
        || rounded.branch_pivots != flat.branch_pivots
    {
        return Err(
            "normal mode changed source geometry, material, anchors or LOD selection".into(),
        );
    }
    let light = lighting();
    let gpu = Gpu::new()?;
    let flat_renderer = Renderer::new(&gpu, &scene(flat.clone())?, &light);
    let rounded_renderer = Renderer::new(&gpu, &scene(rounded.clone())?, &light);
    for renderer in [&flat_renderer, &rounded_renderer] {
        renderer.set_debug_options(DebugRenderOptions {
            bloom_enabled: false,
            ..Default::default()
        })?;
    }
    let mut cases = Vec::new();
    for (name, view) in views(aggregate) {
        for time in [0., 2.] {
            let flat = capture(&gpu, &flat_renderer, &view, &light, time)?;
            let rounded = capture(&gpu, &rounded_renderer, &view, &light, time)?;
            let prefix = format!("{name}-wind{}", time as u32);
            save_image(out, &format!("{prefix}-flat"), &flat.image)?;
            save_image(out, &format!("{prefix}-rounded"), &rounded.image)?;
            side_by_side(out, &prefix, &flat.image, &rounded.image)?;
            let source_alpha_equal = flat.alpha == rounded.alpha;
            let draw_counts_equal = flat.counts == rounded.counts;
            let covered = flat
                .alpha
                .chunks_exact(2)
                .filter(|p| u16::from_le_bytes([p[0], p[1]]) & 0x7fff != 0)
                .count();
            let changed_rgb = flat
                .image
                .chunks_exact(4)
                .zip(rounded.image.chunks_exact(4))
                .filter(|(a, b)| a[..3] != b[..3])
                .count();
            cases.push(json!({"name":prefix,"wind_time_seconds":time,
                "camera":{"position":view.cam.pos.to_array(),"target":view.cam.target.to_array(),"fov_y":view.cam.fov_y},
                "flat":flat.counts,"rounded":rounded.counts,"draw_counts_equal":draw_counts_equal,
                "source_alpha_equal":source_alpha_equal,"source_covered_pixels":covered,
                "source_alpha_sha256":{"flat":digest(&flat.alpha),"rounded":digest(&rounded.alpha)},
                "different_final_rgb_pixels":changed_rgb,
                "images":[format!("{prefix}-flat.jpg"),format!("{prefix}-rounded.jpg"),
                    format!("{prefix}-flat-left-rounded-right.jpg")]}));
            std::fs::write(
                out.join("cases.json"),
                serde_json::to_vec_pretty(&cases).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if !source_alpha_equal || !draw_counts_equal {
                return Err(format!("source coverage or draw counts changed: {prefix}"));
            }
            if name != "distant" && covered == 0 {
                return Err(format!("source view is empty: {prefix}"));
            }
            println!(
                "captured {prefix}: {covered} source pixels, {changed_rgb} changed final RGB pixels"
            );
        }
    }
    Ok(
        json!({"mechanical_pass":true,"visual_quality_accepted":false,"fixture":"rounded meadow shading A/B",
        "resolution":[W,H],"reconstruction":"native; no temporal AA, sharpening or bloom",
        "adapter":{"name":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"driver":gpu.info.driver},
        "seed":SEED,"blades_per_patch":120,"habitat_half_m":HABITAT_HALF,"wind_strength":0.8,
        "light":{"sun_dir":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100},
        "source_lod_triangles":flat.lod_ranges.map(|v|v.map(|r|r[1]/3)),"aggregate_enabled":aggregate,
        "geometry_positions_anchors_materials_and_lod_equal":true,
        "geometry_sha256":{"flat":digest(bytemuck::cast_slice(&flat.vertices)),"rounded":digest(bytemuck::cast_slice(&rounded.vertices))},
        "cases":cases,"limits":["Static wind poses; animation quality needs motion review.",
            "Actual source alpha and draw counts checked; no source depth texture is exposed by this fixture API.",
            "Aggregate uses the existing shading-normal geometric-bias approximation; aggregate optical equality is not asserted.",
            "Bounded diagnostic habitat with one view, not gameplay or four-player performance evidence.",
            "No subjective quality threshold: review the matched images."]}),
    )
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let aggregate = args.iter().any(|arg| arg == "--aggregate");
    let Some(out) = args
        .windows(2)
        .find(|p| p[0] == "--out")
        .map(|p| PathBuf::from(&p[1]))
    else {
        eprintln!("usage: meadow_shading_gpu --out FRESH_DIR [--aggregate]");
        std::process::exit(2);
    };
    if out.exists() {
        eprintln!("output directory already exists: {}", out.display());
        std::process::exit(2);
    }
    if let Err(error) = std::fs::create_dir_all(&out) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    // Match the existing serial GPU fixture convention: only process startup,
    // before wgpu or renderer threads, with all experiment controls recorded.
    unsafe {
        for (name, value) in [
            ("KERN_MEADOW_AGGREGATE", if aggregate { "1" } else { "0" }),
            ("KERN_MEADOW_GROUP_CACHE", "0"),
            ("KERN_MEADOW_GROUP_REPLACE", "0"),
            ("KERN_MEADOW_AGGREGATE_HALF_RES", "0"),
            ("KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE", "0"),
            ("KERN_MEADOW_SUBSTRATE", "0"),
            ("KERN_MEADOW_VISIBILITY", "1"),
            ("KERN_MEADOW_SKIP_HIDDEN_SHADE", "0"),
            ("KERN_TREE_OCCLUSION", "0"),
            ("KERN_STATIC_SUN_CACHE", "0"),
        ] {
            std::env::set_var(name, value);
        }
        std::env::remove_var("KERN_DIAGNOSTIC_HIDE");
    }
    match run(&out, aggregate) {
        Ok(report) => match std::fs::write(
            out.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        ) {
            Ok(()) => println!(
                "PASS mechanical source invariants; images require visual review: {}",
                out.display()
            ),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
