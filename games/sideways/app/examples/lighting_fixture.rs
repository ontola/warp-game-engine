//! Deterministic, offscreen indirect-lighting regression scene.
//! Build explicitly: cargo build -p cli --release --example lighting_fixture
//! No window, input, physics, host connection or running-game process is used.

#[allow(dead_code)]
#[path = "../src/car_look.rs"]
mod car_look;
#[allow(dead_code)]
#[path = "../src/car_model.rs"]
mod car_model;

use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Heightfield, Material};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

const GRID: u32 = 129;
const CELL: f32 = 2.0;
const ORIGIN: f32 = -128.0;

struct Args {
    out: PathBuf,
    obstacle: String,
    car: bool,
    camera: String,
    views: usize,
    size: (u32, u32),
    modes: Vec<u32>,
    warmup: u32,
    repeat: u32,
    contact: bool,
    reflection: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    if args.len() % 2 != 0 {
        return Err("arguments must be --key value pairs".into());
    }
    let allowed = [
        "--out",
        "--obstacle",
        "--car",
        "--camera",
        "--views",
        "--size",
        "--debug",
        "--warmup",
        "--repeat",
        "--contact",
        "--reflection",
    ];
    let mut values = HashMap::new();
    for pair in args.chunks_exact(2) {
        if !allowed.contains(&pair[0].as_str())
            || values.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err(format!("unknown or duplicate option {}", pair[0]));
        }
    }
    let get = |key: &str, default: &str| values.get(key).cloned().unwrap_or_else(|| default.into());
    let integer = |key: &str, default: &str| {
        get(key, default)
            .parse::<u32>()
            .map_err(|_| format!("{key} must be an integer"))
    };
    let toggle = |key: &str| match get(key, "on").as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{key} must be on or off")),
    };
    let obstacle = get("--obstacle", "open");
    if !["open", "mountain"].contains(&obstacle.as_str()) {
        return Err("obstacle must be open or mountain".into());
    }
    let camera = get("--camera", "side");
    if !["side", "contact"].contains(&camera.as_str()) {
        return Err("camera must be side or contact".into());
    }
    let views = integer("--views", "1")? as usize;
    if ![1, 4].contains(&views) {
        return Err("views must be 1 or 4".into());
    }
    let size = get("--size", "1280x720")
        .split('x')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid size")?;
    if size.len() != 2 || size.iter().any(|v| !(64..=16384).contains(v)) {
        return Err("size must be WIDTHxHEIGHT with each dimension in 64..16384".into());
    }
    let modes = get("--debug", "0,2,3,4,5,6,7")
        .split(',')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid debug modes")?;
    if modes.is_empty()
        || modes.iter().any(|v| *v > 7)
        || modes.iter().collect::<std::collections::HashSet<_>>().len() != modes.len()
    {
        return Err("debug modes must be distinct integers in 0..7".into());
    }
    let (warmup, repeat) = (integer("--warmup", "2")?, integer("--repeat", "4")?);
    if repeat == 0 || warmup.checked_add(repeat).is_none_or(|n| n > 4096) {
        return Err("repeat must be positive and warmup+repeat <=4096".into());
    }
    Ok(Args {
        out: PathBuf::from(values.get("--out").ok_or("--out is required")?),
        obstacle,
        car: toggle("--car")?,
        camera,
        views,
        size: (size[0], size[1]),
        modes,
        warmup,
        repeat,
        contact: toggle("--contact")?,
        reflection: toggle("--reflection")?,
    })
}

fn height(x: f32, z: f32, mountain: bool) -> f32 {
    if !mountain {
        return 0.0;
    }
    // A sampled mountain ridge on the +X side. Both raster mesh and the GI
    // heightfield use these exact grid samples; nothing floats above the floor.
    let across = ((x - 8.0) / 4.0)
        .clamp(0.0, 1.0)
        .min(((36.0 - x) / 4.0).clamp(0.0, 1.0));
    let along = ((72.0 - z.abs()) / 8.0).clamp(0.0, 1.0);
    18.0 * across * along
}

fn ground(scene: &mut Scene, mountain: bool) {
    let material = scene.add_material(Material {
        tint: [0.22, 0.22, 0.22],
        layer: -1,
        roughness: 0.92,
        flags: 0,
        scale: 1.0,
        metal: 0.0,
    });
    let mut vertices = Vec::new();
    let mut heights = Vec::new();
    let mut indices = Vec::new();
    for z in 0..GRID {
        for x in 0..GRID {
            let (wx, wz) = (ORIGIN + x as f32 * CELL, ORIGIN + z as f32 * CELL);
            let h = height(wx, wz, mountain);
            heights.push(h);
            let normal = Vec3::new(
                height(wx - CELL, wz, mountain) - height(wx + CELL, wz, mountain),
                2.0 * CELL,
                height(wx, wz - CELL, mountain) - height(wx, wz + CELL, mountain),
            )
            .normalize();
            vertices.push(Vertex {
                pos: [wx, h, wz],
                normal: normal.to_array(),
                uv: [wx, wz],
            });
            if x + 1 < GRID && z + 1 < GRID {
                let a = z * GRID + x;
                indices.extend_from_slice(&[a, a + GRID, a + 1, a + 1, a + GRID, a + GRID + 1]);
            }
        }
    }
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &indices,
        material,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield {
        origin: [ORIGIN, ORIGIN],
        cell: CELL,
        w: GRID,
        h: GRID,
        heights,
    };
}

fn sentinel(scene: &mut Scene) {
    // Keep old and new GI enablement identical with/without a car. This tiny
    // marked solid is buried 10m below the floor, remote from all test rays.
    let material = scene.add_material(Material {
        tint: [0.0; 3],
        layer: -1,
        roughness: 1.0,
        flags: 0,
        scale: 1.0,
        metal: 0.0,
    });
    let vertices = [
        Vertex {
            pos: [-0.005, 0.0, -0.005],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
        },
        Vertex {
            pos: [0.005, 0.0, -0.005],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
        },
        Vertex {
            pos: [0.0, 0.005, 0.005],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
        },
    ];
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 2, 1],
        material,
    }]));
    scene.add_instance(
        mesh,
        Affine3A::from_translation(Vec3::new(40.0, -10.0, 40.0)),
    );
    scene.mark_solid();
}

fn quantiles(mut samples: Vec<f64>) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    let median = if n % 2 == 0 {
        (samples[n / 2 - 1] + samples[n / 2]) * 0.5
    } else {
        samples[n / 2]
    };
    json!({"count":n,"median_ms":median,"p95_ms":samples[((n as f64*0.95).ceil() as usize).saturating_sub(1).min(n-1)]})
}

fn run(a: Args) -> Result<(), String> {
    if a.out.exists() {
        return Err(format!(
            "use a fresh evidence directory: {} already exists",
            a.out.display()
        ));
    }
    std::fs::create_dir_all(&a.out).map_err(|e| e.to_string())?;
    let root = sideways::game_assets::root();
    let mut scene = Scene::default();
    ground(&mut scene, a.obstacle == "mountain");
    sentinel(&mut scene);
    scene.load_sky(&root.join("assets/sky/sky_2k.hdr"))?;
    let spec = sideways::spec::Cars::load(None)?.get("asti")?;
    let car_position = Vec3::new(0.0, spec.wheel_radius as f32, 0.0);
    if a.car {
        let model = root.join(
            spec.model
                .strip_prefix("res://")
                .ok_or("fixture car must be a local model")?,
        );
        // Clean gloss paint: lighting regressions, not material wear.
        let clean = ["paint=gloss", "chips=0", "scratches=0", "swirls=0", "fade=0", "dust=0", "mud=0", "rust=0"];
        let looks = car_look::Looks::load(None, clean.map(String::from).to_vec())?;
        let meshes = car_model::load(
            &mut scene,
            &model,
            0,
            [0.19, 0.035, 0.025],
            &looks,
            &mut Vec::new(),
            &mut HashMap::new(),
        )?;
        let wheels: Vec<_> = (0..2)
            .flat_map(|axle| [-1.0, 1.0].into_iter().map(move |side| (axle, side)))
            .map(|(axle, side)| car_model::Wheel {
                anchor: Vec3::new(
                    (side
                        * if axle == 0 {
                            spec.track
                        } else {
                            spec.track_rear()
                        }
                        * 0.5) as f32,
                    spec.droop_y as f32,
                    (if axle == 0 { -1.0 } else { 1.0 } * spec.wheelbase * 0.5) as f32,
                ),
                compression: -spec.droop_y as f32,
                steer: 0.0,
                spin: 0.0,
            })
            .collect();
        car_model::place(
            &mut scene,
            &meshes,
            Affine3A::from_translation(car_position),
            spec.ride as f32,
            &wheels,
        );
    }
    let camera = if a.camera == "contact" {
        Camera {
            pos: Vec3::new(5.0, 0.26, 2.4),
            target: Vec3::new(0.0, 0.08, 0.0),
            fov_y: 42f32.to_radians(),
            near: 0.05,
        }
    } else {
        Camera {
            pos: Vec3::new(6.0, 1.6, 4.4),
            target: Vec3::new(0.0, 0.65, 0.0),
            fov_y: 52f32.to_radians(),
            near: 0.05,
        }
    };
    let views: Vec<_> = render::split_rects(a.views, a.size.0, a.size.1)
        .into_iter()
        .map(|(x, y, w, h)| ViewRect {
            cam: camera,
            x,
            y,
            w,
            h,
        })
        .collect();
    let mut light = Lighting::default();
    for setting in [
        "sun_elevation=30",
        "sun_azimuth=90",
        "fog_start=1000000",
        "fog_end=2000000",
        "taa=0",
        "gi=1",
        "lod_pixels=0.5",
    ] {
        light.set(setting)?;
    }
    light.set(&format!("contact_occlusion={}", u8::from(a.contact)))?;
    light.set(&format!("reflection_occlusion={}", u8::from(a.reflection)))?;
    let gpu = Gpu::new()?;
    if a.size.0 > gpu.limits.max_texture_dimension_2d
        || a.size.1 > gpu.limits.max_texture_dimension_2d
    {
        return Err("fixture size exceeds GPU limits".into());
    }
    let upload_started = Instant::now();
    let renderer = Renderer::new(&gpu, &scene, &light);
    let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
    let mut mode_reports = Vec::new();
    for mode in &a.modes {
        light.debug = *mode as f32;
        let mut samples = Vec::new();
        let mut pixels = Vec::new();
        for frame in 0..a.warmup + a.repeat {
            renderer.invalidate_shadow();
            let started = Instant::now();
            let (image, stats) =
                renderer.render_views(&gpu, &views, &light, a.size.0, a.size.1, &[]);
            let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            if stats.gpu_ms.iter().any(|(_, v)| !v.is_finite() || *v < 0.0) {
                return Err("invalid GPU sample".into());
            }
            let gpu_ms = if stats.gpu_ms.is_empty() {
                None
            } else {
                Some(stats.gpu_ms.iter().map(|(_, v)| v).sum::<f64>())
            };
            samples.push(json!({"frame":frame,"measured":frame>=a.warmup,"wall_ms":wall_ms,"gpu_pass_sum_ms":gpu_ms,"passes_ms":stats.gpu_ms,"triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles}));
            pixels = image;
        }
        let file = format!("debug-{mode}.png");
        render::write_png(&a.out.join(&file), a.size.0, a.size.1, &pixels)?;
        let measured = &samples[a.warmup as usize..];
        mode_reports.push(json!({"debug":mode,"file":file,"wall":quantiles(measured.iter().filter_map(|v|v["wall_ms"].as_f64()).collect()),"gpu_pass_sum":quantiles(measured.iter().filter_map(|v|v["gpu_pass_sum_ms"].as_f64()).collect()),"samples":samples}));
    }
    let info = gpu.summary();
    let report = json!({"schema":1,"fixture":"indirect_visibility_v1","executable":std::env::current_exe().map_err(|e|e.to_string())?,"command":std::env::args().collect::<Vec<_>>(),
        "obstacle":a.obstacle,"car_present":a.car,"contact_occlusion":a.contact,"reflection_occlusion":a.reflection,"camera_name":a.camera,
        "size":[a.size.0,a.size.1],"view_count":a.views,"camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y_radians":camera.fov_y,"near":camera.near},
        "view_rects":views.iter().map(|v|[v.x,v.y,v.w,v.h]).collect::<Vec<_>>(),"car":{"model":spec.model,"position":car_position.to_array(),"rotation_xyzw":[0,0,0,1],"ride":spec.ride,"wheel_compression":-spec.droop_y},
        "ground":{"grid":[GRID,GRID],"origin":[ORIGIN,ORIGIN],"cell_m":CELL,"albedo_linear":[0.22,0.22,0.22],"footprint_xz":[-0.65,-1.4,0.65,1.4]},
        "inert_solid_sentinel":{"center":[40,-10,40],"max_extent_m":0.01,"reason":"keeps GI enablement unchanged when removing the car"},
        "lighting":{"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100,"taa":false,"fog":false},
        "gpu":{"name":info.name,"backend":info.backend,"driver":info.driver,"timestamps":info.timestamps},"warmup":a.warmup,"repeat":a.repeat,"upload_ms":upload_ms,"modes":mode_reports,
        "limits":["Synthetic real-car fixture, not a captured road or gameplay benchmark.","Four views deliberately repeat the same fixed camera; isolates pixel cost without changing pose.","Wall time includes synchronous pixel readback; GPU total sums instrumented passes only.","Debug images are tonemapped; display-value ratios are not physical radiance ratios.","Every measured frame invalidates sun shadows. No HUD, TAA, controller input or simulation."]});
    std::fs::write(
        a.out.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("lighting fixture: {}", a.out.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!(
            "lighting_fixture --out FRESH_DIR [--obstacle open|mountain] [--car on|off] [--camera side|contact] [--views 1|4] [--size 1280x720] [--debug 0,2,3,4,5,6,7] [--warmup 2] [--repeat 4] [--contact on|off] [--reflection on|off]"
        );
        return;
    }
    if let Err(error) = parse(&args).and_then(run) {
        eprintln!("lighting fixture: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mountain_blocks_plus_x_sun_but_preserves_flat_car_footprint() {
        for x in [-2.0, 0.0, 2.0] {
            for z in [-2.0, 0.0, 2.0] {
                assert_eq!(height(x, z, true), 0.0);
            }
        }
        assert!(height(12.0, 0.0, true) > 12.0 * 30f32.to_radians().tan() + 1.0);
        assert_eq!(height(-12.0, 0.0, true), 0.0);
        assert_eq!(height(12.0, 0.0, false), 0.0);
    }
    #[test]
    fn comparison_summaries_preserve_per_frame_total_quantiles() {
        let q = quantiles(vec![9.0, 1.0, 4.0, 2.0]);
        assert_eq!(q["median_ms"], 3.0);
        assert_eq!(q["p95_ms"], 9.0);
        assert!(quantiles(vec![]).is_null());
    }
}
