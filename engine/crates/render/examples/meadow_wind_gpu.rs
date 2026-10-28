//! Native renderer captures of classic grass and the source-authored windswept profile.
//! Run GPU fixtures serially: --out FRESH_DIR [--animate] [--frames N] [--aggregate].
//! --distance-sweep instead captures a fixed-wind 8-to-3 m approach in lit/albedo views.
//! Animation is one real render per 1/30s timestamp, after three warmup frames.
//! PNG sequences are encoding inputs, not a measurement of real-time frame rate.
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::debug_settings::DebugRenderOptions;
use render::foliage_assembly::MeadowBladeBudget;
use render::meadow_data::MeadowGeometry;
use render::scene::{Heightfield, Material};
use render::wind::WindState;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const W: u32 = 1280;
const H: u32 = 720;
const FPS: u32 = 30;
const SEED: u64 = 73;
const FIELD_HALF: i32 = 15;
const WIND_STRENGTH: f32 = 0.8;

struct Options {
    out: PathBuf,
    frames: u32,
    aggregate: bool,
    distance_sweep: bool,
}
impl Options {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let (mut out, mut frames, mut animate, mut aggregate) = (None, None, false, false);
        let mut distance_sweep = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => out = Some(PathBuf::from(args.next().ok_or("--out needs a directory")?)),
                "--frames" => {
                    frames = Some(
                        args.next()
                            .ok_or("--frames needs a count")?
                            .parse::<u32>()
                            .map_err(|_| "--frames must be a positive integer")?,
                    )
                }
                "--animate" => animate = true,
                "--aggregate" => aggregate = true,
                "--distance-sweep" => distance_sweep = true,
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }
        if distance_sweep && (animate || aggregate) {
            return Err(
                "--distance-sweep uses fixed wind and source geometry; omit --animate/--aggregate"
                    .into(),
            );
        }
        let frames = frames.unwrap_or(if distance_sweep {
            101
        } else if animate {
            120
        } else {
            1
        });
        if !(1..=3600).contains(&frames) {
            return Err("--frames must be in 1..=3600".into());
        }
        Ok(Self {
            out: out.ok_or("--out FRESH_DIR is required")?,
            frames,
            aggregate,
            distance_sweep,
        })
    }
}

fn terrain_height(x: f32, z: f32) -> f32 {
    // A low crest hides the far edge of this bounded field in the close view.
    // Terrain triangles, the heightfield and meadow ground use the same grid.
    let across = (1.0 - (x / 40.0).powi(2)).max(0.0).powi(2);
    0.65 * (-((z + 7.0) / 7.0).powi(2)).exp() * across
}

fn scene(geometry: MeadowGeometry, field_half: i32) -> Result<Scene, String> {
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.65, 0.75, 0.95, 1.0]; 128]));
    let material = scene.add_material(Material {
        tint: [0.055, 0.075, 0.025],
        layer: -1,
        roughness: 1.,
        flags: 0,
        scale: 1.,
        metal: 0.,
    });
    let mut vertices = Vec::with_capacity(81 * 81);
    let mut heights = Vec::with_capacity(81 * 81);
    for z in -40..=40 {
        for x in -40..=40 {
            let (x, z) = (x as f32, z as f32);
            let y = terrain_height(x, z);
            let dx = terrain_height(x + 0.01, z) - terrain_height(x - 0.01, z);
            let dz = terrain_height(x, z + 0.01) - terrain_height(x, z - 0.01);
            vertices.push(Vertex {
                pos: [x, y, z],
                normal: Vec3::new(-dx, 0.02, -dz).normalize().to_array(),
                uv: [0.; 2],
            });
            heights.push(y);
        }
    }
    let mut indices = Vec::with_capacity(80 * 80 * 6);
    for z in 0..80u32 {
        for x in 0..80u32 {
            let a = z * 81 + x;
            // Exact same x+z<=1 triangle split as MeadowField interpolation.
            indices.extend_from_slice(&[a, a + 81, a + 1, a + 1, a + 81, a + 82]);
        }
    }
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &indices,
        material,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    let ground = [
        [-200., -0.02, 200.],
        [200., -0.02, 200.],
        [-200., -0.02, -200.],
        [200., -0.02, -200.],
    ]
    .map(|pos| Vertex {
        pos,
        normal: Vec3::Y.to_array(),
        uv: [0.; 2],
    });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &ground,
        indices: &[0, 1, 2, 1, 3, 2],
        material,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield {
        origin: [-40.; 2],
        cell: 1.,
        w: 81,
        h: 81,
        heights,
    };
    let samples = (-field_half..=field_half)
        .flat_map(|z| {
            (-field_half..=field_half)
                .map(move |x| [terrain_height(x as f32, z as f32), 1_000_000., 1., 0.])
        })
        .collect();
    let field = render::meadow_data::MeadowField {
        origin: [-field_half as f32; 2],
        cell: 1.,
        width: (2 * field_half + 1) as u32,
        height: (2 * field_half + 1) as u32,
        samples,
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
        sun_dir: Vec3::new(0.65, 0.42, 0.63).normalize(),
        sun_lux: 60_000.,
        sky_lux: 12_000.,
        ev100: 13.5,
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

fn view(pos: Vec3, target: Vec3, degrees: f32) -> ViewRect {
    ViewRect {
        cam: Camera {
            pos,
            target,
            fov_y: degrees.to_radians(),
            near: 0.05,
        },
        x: 0,
        y: 0,
        w: W,
        h: H,
    }
}
fn field_view() -> ViewRect {
    view(
        Vec3::new(0., terrain_height(0., 5.) + 0.72, 5.),
        Vec3::new(0., 0.87, -7.),
        50.,
    )
}
fn still_views(aggregate: bool) -> Vec<(&'static str, ViewRect)> {
    let mut result = vec![
        (
            "blade-detail",
            view(
                Vec3::new(0.55, terrain_height(0., 0.) + 0.50, 0.70),
                Vec3::new(0., terrain_height(0., 0.) + 0.30, 0.),
                45.,
            ),
        ),
        (
            "source-lod",
            view(Vec3::new(-4., 4., 32.), Vec3::new(0., 0.7, 0.), 50.),
        ),
    ];
    if aggregate {
        result.extend([
            (
                "handoff",
                view(Vec3::new(-4., 7., 70.), Vec3::new(0., 0.7, 0.), 50.),
            ),
            (
                "distant",
                view(Vec3::new(-4., 11., 110.), Vec3::new(0., 0.7, 0.), 50.),
            ),
        ]);
    }
    result
}
fn camera_json(view: &ViewRect) -> Value {
    json!({"position":view.cam.pos.to_array(),"target":view.cam.target.to_array(),"fov_y":view.cam.fov_y,"near":view.cam.near})
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
fn save_still(path: &Path, rgba: &[u8]) -> Result<(), String> {
    render::write_png(&path.with_extension("png"), W, H, rgba)?;
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    let file = std::fs::File::create(path.with_extension("jpg")).map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 94)
        .encode(&rgb, W, H, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())
}

/// Actual source coverage, before composition; RGB is not a coverage oracle.
fn source_alpha(gpu: &Gpu, renderer: &Renderer) -> Result<Vec<u8>, String> {
    let layers = renderer.meadow_source_fixture_layers();
    if layers.len() != 1 {
        return Err("expected one source color/alpha layer".into());
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
        label: Some("windswept source alpha capture"),
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
    let alpha: Vec<u8> = mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| {
            row[..(W * 8) as usize]
                .chunks_exact(8)
                .flat_map(|p| p[6..8].iter().copied())
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    if alpha.chunks_exact(2).any(|p| {
        let bits = u16::from_le_bytes([p[0], p[1]]);
        bits != 0x8000 && bits > 0x3c00
    }) {
        return Err("source coverage is nonfinite or outside [0,1]".into());
    }
    Ok(alpha)
}
fn changed_pixels(a: &[u8], b: &[u8], bytes_per_pixel: usize) -> usize {
    a.chunks_exact(bytes_per_pixel)
        .zip(b.chunks_exact(bytes_per_pixel))
        .filter(|(a, b)| a != b)
        .count()
}
fn covered_pixels(alpha: &[u8]) -> usize {
    alpha
        .chunks_exact(2)
        .filter(|p| u16::from_le_bytes([p[0], p[1]]) & 0x7fff != 0)
        .count()
}
fn geometry_report(geometry: &MeadowGeometry) -> Value {
    let anchors: Vec<Vec<usize>> = geometry
        .lod_ranges
        .iter()
        .map(|lods| {
            lods.iter()
                .map(|[first, count]| {
                    geometry.vertices[*first as usize..(*first + *count) as usize]
                        .iter()
                        .map(|v| {
                            [
                                v.anchor[0].to_bits(),
                                v.anchor[1].to_bits(),
                                v.anchor[2].to_bits(),
                            ]
                        })
                        .collect::<BTreeSet<_>>()
                        .len()
                })
                .collect()
        })
        .collect();
    json!({"vertices":geometry.vertices.len(),"vertex_bytes":std::mem::size_of_val(geometry.vertices.as_slice()),
        "lod_triangles":geometry.lod_ranges.map(|v|v.map(|r|r[1]/3)),"distinct_blade_anchors":anchors,
        "lod_errors":geometry.lod_errors,"max_tuft_radius":geometry.max_tuft_radius,
        "max_tuft_height":geometry.max_tuft_height,"max_blade_radius":geometry.max_blade_radius,
        "source_sha256":digest(bytemuck::cast_slice(&geometry.vertices))})
}
fn counts(gpu: &Gpu, renderer: &Renderer) -> Result<Value, String> {
    let all = renderer.meadow_diagnostics(gpu)?;
    let d = all.first().ok_or("missing meadow diagnostics")?;
    if all.len() != 1 || d.visibility_invalid_ids != 0 {
        return Err("invalid meadow diagnostics".into());
    }
    Ok(
        json!({"plants":d.plants,"triangles":d.triangles,"submitted_source_triangles":d.submitted_source_triangles,
        "source_vertices_per_patch":d.source_vertices_per_patch,"source_geometry_passes":d.source_geometry_passes,
        "lod_pixels":d.lod_pixels,"density":d.density,"size_scale":d.size_scale,
        "visibility_enabled":d.visibility_enabled,"visibility_covered_pixels":d.visibility_covered_pixels,
        "visibility_unique_primitives":d.visibility_unique_primitives,"visibility_invalid_ids":d.visibility_invalid_ids,
        "aggregate_enabled":d.aggregate_enabled,"aggregate_plants":d.aggregate_plants,
        "aggregate_triangles":d.aggregate_triangles,"aggregate_transition_m":d.aggregate_transition_m}),
    )
}
fn render_at(
    gpu: &Gpu,
    renderer: &Renderer,
    view: &ViewRect,
    light: &Lighting,
    time: f64,
    strength: f32,
) -> Result<wgpu::Texture, String> {
    renderer.set_wind(
        gpu,
        &WindState {
            time_seconds: time,
            strength,
            ..Default::default()
        },
    )?;
    renderer.try_render_texture(gpu, std::slice::from_ref(view), light, W, H)
}
fn warmup(
    gpu: &Gpu,
    renderer: &Renderer,
    view: &ViewRect,
    light: &Lighting,
    start: f64,
    strength: f32,
) -> Result<(), String> {
    for frame in [-3, -2, -1] {
        render_at(
            gpu,
            renderer,
            view,
            light,
            start + f64::from(frame) / f64::from(FPS),
            strength,
        )?;
    }
    Ok(())
}
struct Capture {
    image: Vec<u8>,
    alpha: Vec<u8>,
}
fn capture(
    gpu: &Gpu,
    renderer: &Renderer,
    view: &ViewRect,
    light: &Lighting,
    time: f64,
    strength: f32,
) -> Result<Capture, String> {
    // Exactly one draw: repeated draws would erase the preceding wind pose.
    let texture = render_at(gpu, renderer, view, light, time, strength)?;
    Ok(Capture {
        image: gpu.read_rgba8(&texture),
        alpha: source_alpha(gpu, renderer)?,
    })
}

fn run_mode(
    gpu: &Gpu,
    options: &Options,
    name: &str,
    geometry: MeadowGeometry,
    light: &Lighting,
) -> Result<Value, String> {
    geometry.validate()?;
    let source = geometry_report(&geometry);
    let renderer = Renderer::new(gpu, &scene(geometry, FIELD_HALF)?, light);
    renderer.set_debug_options(DebugRenderOptions {
        bloom_enabled: false,
        ..Default::default()
    })?;
    let camera = field_view();
    warmup(gpu, &renderer, &camera, light, 0., 0.)?;
    let still0 = capture(gpu, &renderer, &camera, light, 0., 0.)?;
    let still2 = capture(gpu, &renderer, &camera, light, 2., 0.)?;
    let zero_wind = json!({"times_seconds":[0,2],"source_covered_pixels":covered_pixels(&still0.alpha),
        "alpha_equal":still0.alpha==still2.alpha,"image_equal":still0.image==still2.image,
        "alpha_changed_pixels":changed_pixels(&still0.alpha,&still2.alpha,2),
        "image_changed_pixels":changed_pixels(&still0.image,&still2.image,4),
        "alpha_sha256":[digest(&still0.alpha),digest(&still2.alpha)],
        "image_sha256":[digest(&still0.image),digest(&still2.image)]});
    save_still(
        &options.out.join(format!("{name}-zero-wind")),
        &still0.image,
    )?;
    write_json(
        &options.out.join(format!("{name}-zero-wind.json")),
        &zero_wind,
    )?;
    if still0.alpha != still2.alpha
        || still0.image != still2.image
        || covered_pixels(&still0.alpha) == 0
    {
        return Err(format!(
            "{name}: zero-wind source is empty or changes with time; see zero-wind report"
        ));
    }
    drop((still0, still2));

    let sequence_dir = options.out.join(name);
    std::fs::create_dir(&sequence_dir).map_err(|e| e.to_string())?;
    warmup(gpu, &renderer, &camera, light, 0., WIND_STRENGTH)?;
    let mut previous: Option<Capture> = None;
    let mut frame_rows = Vec::with_capacity(options.frames as usize);
    let mut frames_with_alpha_change = 0;
    for frame in 0..options.frames {
        let time = f64::from(frame) / f64::from(FPS);
        let current = capture(gpu, &renderer, &camera, light, time, WIND_STRENGTH)?;
        let covered = covered_pixels(&current.alpha);
        if covered == 0 {
            return Err(format!("{name} frame {frame}: empty source coverage"));
        }
        let alpha_change = previous
            .as_ref()
            .map(|p| changed_pixels(&p.alpha, &current.alpha, 2));
        let image_change = previous
            .as_ref()
            .map(|p| changed_pixels(&p.image, &current.image, 4));
        if alpha_change.is_some_and(|n| n > 0) {
            frames_with_alpha_change += 1;
        }
        render::write_png(
            &sequence_dir.join(format!("frame-{frame:04}.png")),
            W,
            H,
            &current.image,
        )?;
        let count = if frame == 0 || frame + 1 == options.frames {
            counts(gpu, &renderer)?
        } else {
            Value::Null
        };
        frame_rows.push(json!({"frame":frame,"time_seconds":time,"source_covered_pixels":covered,
            "source_alpha_sha256":digest(&current.alpha),"rgba_sha256":digest(&current.image),
            "source_alpha_changed_pixels_from_previous":alpha_change,"rgba_changed_pixels_from_previous":image_change,
            "draw_counts":count}));
        if frame == 0 {
            save_still(&options.out.join(format!("{name}-field")), &current.image)?;
        }
        if frame % FPS == 0 || frame + 1 == options.frames {
            write_json(
                &options.out.join(format!("{name}-frames.json")),
                &json!(frame_rows),
            )?;
            println!(
                "{name}: captured {}/{}, t={time:.3}s, source pixels={covered}",
                frame + 1,
                options.frames
            );
        }
        previous = Some(current);
    }
    if options.frames > 1 && frames_with_alpha_change == 0 {
        return Err(format!(
            "{name}: advancing wind did not change source silhouette in any captured frame"
        ));
    }
    let mut stills = Vec::new();
    for (shot, camera) in still_views(options.aggregate) {
        warmup(gpu, &renderer, &camera, light, 2., WIND_STRENGTH)?;
        let current = capture(gpu, &renderer, &camera, light, 2., WIND_STRENGTH)?;
        let covered = covered_pixels(&current.alpha);
        if shot != "distant" && covered == 0 {
            return Err(format!("{name}/{shot}: source view is empty"));
        }
        save_still(&options.out.join(format!("{name}-{shot}")), &current.image)?;
        stills.push(json!({"name":shot,"time_seconds":2,"camera":camera_json(&camera),"source_covered_pixels":covered,
            "source_alpha_sha256":digest(&current.alpha),"rgba_sha256":digest(&current.image),"draw_counts":counts(gpu,&renderer)?}));
    }
    Ok(
        json!({"profile":name,"geometry":source,"zero_wind":zero_wind,
        "field_camera":camera_json(&camera),"frames":frame_rows,"frames_with_source_alpha_change":frames_with_alpha_change,
        "stills":stills,"sequence":format!("{name}/frame-%04d.png")}),
    )
}

fn run_distance_sweep(gpu: &Gpu, options: &Options, light: &Lighting) -> Result<Value, String> {
    const TIME: f64 = 2.;
    const HALF: i32 = 2;
    let geometry = MeadowGeometry::cook_windswept(SEED, 0., MeadowBladeBudget::Blades120)?;
    geometry.validate()?;
    let source = geometry_report(&geometry);
    let renderer = Renderer::new(gpu, &scene(geometry, HALF)?, light);
    renderer.set_debug_options(DebugRenderOptions {
        bloom_enabled: false,
        meadow_size_scale: 1.,
        meadow_density: 1.,
        ..Default::default()
    })?;
    let albedo_light = Lighting {
        debug: 2.,
        ..light.clone()
    };
    let target = Vec3::new(0., terrain_height(0., 0.) + 0.45, 0.);
    let camera_at = |distance: f32| view(target + Vec3::Z * distance, target, 35.);
    for _ in 0..3 {
        render_at(gpu, &renderer, &camera_at(8.), light, TIME, WIND_STRENGTH)?;
    }
    for pass in ["lit", "albedo"] {
        std::fs::create_dir(options.out.join(pass)).map_err(|e| e.to_string())?;
    }
    let mut rows = Vec::with_capacity(options.frames as usize);
    let mut previous: Option<(Capture, Capture)> = None;
    for frame in 0..options.frames {
        // At the default 101 frames, both endpoints and exactly 6 m are sampled.
        // Only position changes: orientation, FOV, wind pose and the roots stay fixed.
        let fraction = frame as f32 / options.frames.saturating_sub(1).max(1) as f32;
        let distance = 8. - 5. * fraction;
        let camera = camera_at(distance);
        let lit = capture(gpu, &renderer, &camera, light, TIME, WIND_STRENGTH)?;
        let draw_counts = counts(gpu, &renderer)?;
        let albedo = capture(gpu, &renderer, &camera, &albedo_light, TIME, WIND_STRENGTH)?;
        let covered = covered_pixels(&lit.alpha);
        if covered == 0 || lit.alpha != albedo.alpha {
            return Err(format!(
                "distance sweep frame {frame}: empty source or lit/albedo coverage differs"
            ));
        }
        for (pass, current) in [("lit", &lit), ("albedo", &albedo)] {
            render::write_png(
                &options.out.join(pass).join(format!("frame-{frame:04}.png")),
                W,
                H,
                &current.image,
            )?;
            let six_m_frame = ((options.frames - 1) as f32 * 0.4).round() as u32;
            if frame == 0 || frame == six_m_frame || frame + 1 == options.frames {
                save_still(
                    &options.out.join(format!(
                        "distance-{}cm-{pass}",
                        (distance * 100.).round() as u32
                    )),
                    &current.image,
                )?;
            }
        }
        rows.push(json!({"frame":frame,"playback_time_seconds":f64::from(frame)/f64::from(FPS),
            "wind_time_seconds":TIME,"distance_to_target_m":distance,"camera":camera_json(&camera),
            "view_direction":[0,0,-1],"source_covered_pixels":covered,
            "lit_albedo_source_alpha_equal":true,"source_alpha_sha256":digest(&lit.alpha),
            "lit_rgba_sha256":digest(&lit.image),"albedo_rgba_sha256":digest(&albedo.image),
            "source_alpha_changed_pixels_from_previous":previous.as_ref().map(|(p,_)|changed_pixels(&p.alpha,&lit.alpha,2)),
            "lit_rgba_changed_pixels_from_previous":previous.as_ref().map(|(p,_)|changed_pixels(&p.image,&lit.image,4)),
            "albedo_rgba_changed_pixels_from_previous":previous.as_ref().map(|(_,p)|changed_pixels(&p.image,&albedo.image,4)),
            "draw_counts":draw_counts}));
        if frame % FPS == 0 || frame + 1 == options.frames {
            write_json(
                &options.out.join("distance-sweep-frames.json"),
                &json!(rows),
            )?;
            println!(
                "distance sweep: captured {}/{}, distance={distance:.2}m, source pixels={covered}",
                frame + 1,
                options.frames
            );
        }
        previous = Some((lit, albedo));
    }
    Ok(
        json!({"mechanical_pass":true,"visual_quality_accepted":false,
        "fixture":"windswept fixed-pose source LOD distance sweep","resolution":[W,H],
        "reconstruction":"native; no TAA, sharpening or bloom","seed":SEED,
        "adapter":{"name":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"driver":gpu.info.driver},
        "geometry":source,"frames_per_pass":options.frames,"fps":FPS,"warmup_frames":3,
        "wind_time_seconds":TIME,"wind_strength":WIND_STRENGTH,"renders_per_camera":2,
        "camera_path":{"start_distance_m":8,"end_distance_m":if options.frames>1 {3} else {8},
            "target":target.to_array(),"direction":[0,0,-1],"fov_degrees":35},
        "size_scale":1,"density":1,"field_half_m":HALF,"aggregate_enabled":false,
        "light":{"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100},
        "albedo_debug":2,"sequences":["lit/frame-%04d.png","albedo/frame-%04d.png"],"frames":rows,
        "limits":["Camera distance is to the fixed field target. Individual roots have procedural growth and position offsets; normalized LOD distances differ.",
            "Per-frame images and coverage change with projection and occlusion. Their differences are diagnostics, not a matching-height color/AO oracle.",
            "Albedo debug removes lighting; it displays the material including any authored AO. Lit/albedo source alpha must match exactly at each camera.",
            "Wind time is fixed: playback shows camera approach, not wind animation or motion-vector validation.",
            "No subjective quality threshold or gameplay/performance claim; review the sequences and separate shader material oracle."]}),
    )
}

fn run(options: &Options) -> Result<Value, String> {
    let light = lighting();
    let gpu = Gpu::new()?;
    if options.distance_sweep {
        return run_distance_sweep(&gpu, options, &light);
    }
    let mut profiles = Vec::new();
    for name in ["classic", "windswept"] {
        let geometry = match name {
            "classic" => {
                MeadowGeometry::cook_with_blade_budget(SEED, 0., MeadowBladeBudget::Blades120)?
            }
            _ => MeadowGeometry::cook_windswept(SEED, 0., MeadowBladeBudget::Blades120)?,
        };
        let report = run_mode(&gpu, options, name, geometry, &light)?;
        write_json(&options.out.join(format!("{name}-report.json")), &report)?;
        profiles.push(report);
    }
    Ok(
        json!({"mechanical_pass":true,"visual_quality_accepted":false,"fixture":"classic versus windswept native meadow",
        "resolution":[W,H],"reconstruction":"native; no TAA, sharpening or bloom","seed":SEED,
        "adapter":{"name":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"driver":gpu.info.driver},
        "fps":FPS,"frames_per_profile":options.frames,"encoded_duration_seconds":f64::from(options.frames)/f64::from(FPS),
        "warmup_frames":3,"renders_per_animation_timestamp":1,"wind_strength":WIND_STRENGTH,
        "field_half_m":FIELD_HALF,"field_grid_m":1,"terrain":"shared analytic low crest, identical triangle grids",
        "aggregate_enabled":options.aggregate,
        "light":{"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100},
        "profiles":profiles,"limits":["Geometry, density and silhouette differ between profiles; no cross-profile alpha equality asserted.",
            "Animation is native rendered output; native source motion-vector generation is not enabled or verified.",
            "Source HDR alpha is checked; source depth has no public capture getter in this fixture.",
            "Static camera and diagnostic terrain; not a gameplay, temporal reconstruction or four-player performance claim.",
            "Image quality and temporal artifacts require visual review; frame capture duration is not rendering FPS."]}),
    )
}

fn main() {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(error) => {
            eprintln!(
                "{error}\nusage: meadow_wind_gpu --out FRESH_DIR [--animate | --distance-sweep] [--frames N] [--aggregate]"
            );
            std::process::exit(2);
        }
    };
    if options.out.exists() {
        eprintln!("output directory already exists: {}", options.out.display());
        std::process::exit(2);
    }
    if let Err(error) = std::fs::create_dir_all(&options.out) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    // Existing fixture convention: fix controls only at single-threaded
    // process startup, before wgpu or renderer workers are created.
    unsafe {
        for (name, value) in [
            (
                "KERN_MEADOW_AGGREGATE",
                if options.aggregate { "1" } else { "0" },
            ),
            ("KERN_MEADOW_GROUP_CACHE", "0"),
            ("KERN_MEADOW_GROUP_REPLACE", "0"),
            ("KERN_MEADOW_AGGREGATE_HALF_RES", "0"),
            ("KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE", "0"),
            ("KERN_MEADOW_SUBSTRATE", "0"),
            ("KERN_MEADOW_VISIBILITY", "1"),
            ("KERN_MEADOW_SKIP_HIDDEN_SHADE", "0"),
            ("KERN_MEADOW_INDEXED", "1"),
            ("KERN_TREE_OCCLUSION", "0"),
            ("KERN_STATIC_SUN_CACHE", "0"),
        ] {
            std::env::set_var(name, value);
        }
        std::env::remove_var("KERN_DIAGNOSTIC_HIDE");
    }
    match run(&options) {
        Ok(report) => {
            if let Err(error) = write_json(&options.out.join("report.json"), &report) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            println!(
                "PASS mechanical capture checks; review images and animation: {}",
                options.out.display()
            );
        }
        Err(error) => {
            let _ = write_json(
                &options.out.join("report.json"),
                &json!({"mechanical_pass":false,"error":error,"partial_evidence":"profile reports and frame JSON files"}),
            );
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
