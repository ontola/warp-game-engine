//! Actual GPU comparison of source crowns and shared density volumes.
//! Bounded development fixture, not a full-world or play-FPS benchmark.
//! cargo run --release -p cli --example voxel_forest_gpu -- --out FRESH_DIR
use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex, read_kmesh};
use render::foliage_volume_data::{PackLimits, VolumeBinding, pack};
use render::foliage_voxel::{BakeConfig, VoxelAsset, bake};
use render::scene::{ALPHA_TEST, Heightfield, Material, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

struct Args {
    out: PathBuf,
    count: usize,
    size: (u32, u32),
    level: usize,
    azimuth: f32,
    elevation: f32,
    distance: f32,
    debug: f32,
    mode: String,
}
fn args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.len() % 2 != 0 {
        return Err("expected --key value pairs".into());
    }
    let mut a = Args {
        out: PathBuf::new(),
        count: 64,
        size: (960, 540),
        level: 0,
        azimuth: 0.0,
        elevation: 8.0,
        distance: 0.0,
        debug: 0.0,
        mode: "both".into(),
    };
    for pair in raw.chunks_exact(2) {
        match pair[0].as_str() {
            "--out" => a.out = pair[1].clone().into(),
            "--trees" => a.count = pair[1].parse().map_err(|_| "invalid tree count")?,
            "--size" => {
                let (w, h) = pair[1].split_once('x').ok_or("expected WIDTHxHEIGHT")?;
                a.size = (
                    w.parse().map_err(|_| "invalid width")?,
                    h.parse().map_err(|_| "invalid height")?,
                );
            }
            "--level" => a.level = pair[1].parse().map_err(|_| "invalid level")?,
            "--azimuth" => a.azimuth = pair[1].parse().map_err(|_| "invalid azimuth")?,
            "--elevation" => a.elevation = pair[1].parse().map_err(|_| "invalid elevation")?,
            "--distance" => a.distance = pair[1].parse().map_err(|_| "invalid distance")?,
            "--debug" => a.debug = pair[1].parse().map_err(|_| "invalid debug view")?,
            "--mode" => a.mode = pair[1].clone(),
            _ => return Err(format!("unknown option {}", pair[0])),
        }
    }
    if a.out.as_os_str().is_empty() || a.out.exists() {
        return Err("--out must name a fresh directory".into());
    }
    if !(1..=256).contains(&a.count)
        || a.level > 3
        || !(128..=3840).contains(&a.size.0)
        || !(128..=2160).contains(&a.size.1)
        || !a.azimuth.is_finite()
        || !a.elevation.is_finite()
        || !(-5.0..=80.0).contains(&a.elevation)
        || !a.distance.is_finite()
        || !(0.0..=800.0).contains(&a.distance)
        || !a.debug.is_finite()
        || !(0.0..=7.0).contains(&a.debug)
        || !["source", "voxel", "both"].contains(&a.mode.as_str())
    {
        return Err("bounded fixture: 1..256 trees, level 0..3, 128..3840 x 128..2160, finite angle, distance 0..800, mode source|voxel|both".into());
    }
    Ok(a)
}

type OwnedPart = (Vec<Vertex>, Vec<u32>, u32);
struct Assets {
    parts: Vec<Vec<OwnedPart>>,
    volumes: Vec<VoxelAsset>,
    materials: Vec<Material>,
    texture_paths: Vec<PathBuf>,
    crown: u32,
    root: PathBuf,
}
fn load_assets() -> Result<Assets, String> {
    let root = sideways::game_assets::root();
    let texture_paths: Vec<_> = [
        "assets/trees/trunk_a_albedo.jpg",
        "assets/trees/trunk_b_albedo.jpg",
        "assets/trees/bark_albedo.jpg",
        "assets/trees/authored_pine_albedo.png",
    ]
    .map(|p| root.join(p))
    .into();
    let mut scene = Scene::default();
    for path in &texture_paths {
        scene.add_texture(path)?;
    }
    for i in 0..4 {
        scene.add_material(Material {
            tint: if i == 3 { [0.62, 0.66, 0.55] } else { [1.0; 3] },
            layer: i,
            roughness: 0.9,
            flags: if i == 3 { ALPHA_TEST | TWO_SIDED } else { 0 },
            scale: 1.0,
            metal: 0.0,
        });
    }
    let mut parts = Vec::new();
    let mut volumes = Vec::new();
    for variant in ["a", "b", "c"] {
        let path = root.join(format!("assets/meshes/fir_{variant}_near.kmesh"));
        let surfaces = read_kmesh(&std::fs::read(&path).map_err(|e| e.to_string())?)?;
        let owned: Vec<OwnedPart> = surfaces
            .into_iter()
            .map(|(name, v, i)| {
                let material = if name.contains("trunk_b") {
                    1
                } else if name.contains("trunk") {
                    0
                } else if name.contains("bark") {
                    2
                } else {
                    3
                };
                (v, i, material)
            })
            .collect();
        let crowns: Vec<Part> = owned
            .iter()
            .filter(|(_, _, m)| *m == 3)
            .map(|(v, i, m)| Part {
                vertices: v,
                indices: i,
                material: *m,
            })
            .collect();
        let volume = bake(
            &crowns,
            &scene.materials,
            &scene.textures,
            BakeConfig {
                cell_size: 0.25,
                levels: 4,
                ..Default::default()
            },
        )?;
        parts.push(owned);
        volumes.push(volume);
    }
    Ok(Assets {
        parts,
        volumes,
        materials: scene.materials,
        texture_paths,
        crown: 3,
        root,
    })
}

fn positions(count: usize) -> Vec<Affine3A> {
    let side = (count as f32).sqrt().ceil() as usize;
    (0..count)
        .map(|i| {
            let x = i % side;
            let z = i / side;
            let hash = |salt: u32| {
                let mut n = (i as u32).wrapping_mul(747796405).wrapping_add(salt);
                n = (n ^ (n >> 16)).wrapping_mul(2246822519);
                (n & 65535) as f32 / 65535.0
            };
            let px = (x as f32 - (side as f32 - 1.0) * 0.5) * 5.6 + (hash(11) - 0.5) * 2.0;
            let pz = (z as f32 - (side as f32 - 1.0) * 0.5) * 5.6 + (hash(89) - 0.5) * 2.0;
            let scale = 0.85 + hash(31) * 0.3;
            Affine3A::from_scale_rotation_translation(
                Vec3::new(scale, scale * (0.9 + hash(71) * 0.25), scale),
                Quat::from_rotation_y(hash(17) * std::f32::consts::TAU),
                Vec3::new(px, 0.0, pz),
            )
        })
        .collect()
}

fn fixture(assets: &Assets, a: &Args, voxel: bool) -> Result<Scene, String> {
    let mut scene = Scene::default();
    for path in &assets.texture_paths {
        scene.add_texture(path)?;
    }
    for &material in &assets.materials {
        scene.add_material(material);
    }
    scene.load_sky(&assets.root.join("assets/sky/sky_2k.hdr"))?;
    let mut mesh_ids = Vec::new();
    for owned in &assets.parts {
        let parts: Vec<Part> = owned
            .iter()
            .map(|(v, i, m)| Part {
                vertices: v,
                indices: i,
                material: *m,
            })
            .collect();
        let mesh = ClusterMesh::build_with_policy(&parts, |p| {
            if p.material == assets.crown {
                SimplificationPolicy::AlphaCards
            } else {
                SimplificationPolicy::Geometry
            }
        });
        mesh_ids.push(scene.add_mesh(&mesh));
    }
    let mut bindings = Vec::new();
    for (i, xf) in positions(a.count).into_iter().enumerate() {
        let owner = scene.instances.len() as u32;
        scene.add_instance(mesh_ids[i % 3], xf);
        scene.mark_foliage();
        bindings.push(VolumeBinding {
            owner,
            asset: (i % 3) as u32,
            crown_material: assets.crown,
        });
    }
    let ground = scene.add_material(Material {
        tint: [0.16, 0.19, 0.12],
        layer: -1,
        roughness: 0.95,
        flags: 0,
        scale: 1.0,
        metal: 0.0,
    });
    let vertices: Vec<_> = [
        [-300.0, 0.0, -300.0],
        [-300.0, 0.0, 300.0],
        [300.0, 0.0, -300.0],
        [300.0, 0.0, 300.0],
    ]
    .map(|pos| Vertex {
        pos,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0; 2],
    })
    .into();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 1, 2, 2, 1, 3],
        material: ground,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield {
        origin: [-300.0, -300.0],
        cell: 10.0,
        w: 61,
        h: 61,
        heights: vec![0.0; 61 * 61],
    };
    if voxel {
        let mut volumes = assets.volumes.clone();
        for v in &mut volumes {
            // Keep coarser index levels for provably empty-region skipping.
            // The first remaining level is still the explicit shaded level.
            v.levels.drain(..a.level);
        }
        scene.voxel_forest = Some(pack(
            &volumes,
            &bindings,
            &scene.instances,
            PackLimits { first_level_bounds: true, ..Default::default() },
        )?);
    }
    Ok(scene)
}

fn run(a: Args) -> Result<(), String> {
    let begun = Instant::now();
    let assets = load_assets()?;
    let side = (a.count as f32).sqrt().ceil();
    let distance = if a.distance > 0.0 {
        a.distance
    } else {
        side * 5.6 * 0.9 + 20.0
    };
    let az = a.azimuth.to_radians();
    let el = a.elevation.to_radians();
    let target = Vec3::new(0.0, 6.0, 0.0);
    let cam = Camera {
        pos: target + Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos()) * distance,
        target,
        fov_y: 55f32.to_radians(),
        near: 0.1,
    };
    let mut light = Lighting::default();
    light.taa = 0.0;
    light.sharpen = 0.0;
    light.lod_pixels = 0.1;
    light.debug = a.debug;
    let gpu = Gpu::new()?;
    std::fs::create_dir_all(&a.out).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    let mut incomplete = false;
    let mut source_mask: Option<Vec<f32>> = None;
    for voxel in [false, true] {
        if (a.mode == "source" && voxel) || (a.mode == "voxel" && !voxel) {
            continue;
        }
        let mode = if voxel { "voxel" } else { "source" };
        let scene = fixture(&assets, &a, voxel)?;
        let renderer = Renderer::new(&gpu, &scene, &light);
        let started = Instant::now();
        let picks: Vec<_> = if !voxel {
            (0..a.size.1)
                .flat_map(|y| (0..a.size.0).map(move |x| (x, y)))
                .collect()
        } else {
            Vec::new()
        };
        let (pixels, stats) =
            renderer.render_picking(&gpu, &cam, &light, a.size.0, a.size.1, &picks);
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        if !voxel {
            source_mask = Some(
                stats
                    .picks
                    .iter()
                    .map(|p| {
                        if p.is_some_and(|(_, cluster, _)| {
                            scene.clusters[cluster as usize].material == assets.crown
                        }) {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            );
        }
        let diagnostics = renderer.voxel_diagnostics(&gpu)?;
        let diag:Vec<_>=diagnostics.unwrap_or_default().into_iter().map(|d|{
            if d.step_overflow_rays>0{incomplete=true;}
            json!({"candidate_overflow_tiles":d.candidate_overflow_tiles,"step_overflow_rays":d.step_overflow_rays,"traced_rays":d.traced_rays,"max_steps_used":d.max_steps_used,"max_tile_candidates":d.max_tile_candidates,
                "max_ray_candidates":d.max_ray_candidates,"ray_candidate_overflow_rays":d.ray_candidate_overflow_rays,
                "max_active_volumes":d.max_active_volumes,"active_cache_overflow_rays":d.active_cache_overflow_rays,
                "max_empty_brick_skips":d.max_empty_brick_skips})
        }).collect();
        let file = a.out.join(format!("{mode}.png"));
        render::write_png(&file, a.size.0, a.size.1, &pixels)?;
        let coverage = if voxel {
            let layers = renderer.voxel_layers_rgba16(&gpu)?;
            let (w, h, bytes) = layers.first().ok_or("voxel layer missing")?;
            if (*w, *h) != a.size || bytes.len() != (*w as usize) * (*h as usize) * 8 {
                return Err("voxel layer size mismatch".into());
            }
            std::fs::write(a.out.join("voxel-layer-rgba16f.bin"), bytes)
                .map_err(|e| e.to_string())?;
            let mut alpha = Vec::with_capacity((*w as usize) * (*h as usize));
            for pixel in bytes.chunks_exact(8) {
                let values: [f32; 4] = std::array::from_fn(|k| {
                    half_to_f32(u16::from_le_bytes([pixel[k * 2], pixel[k * 2 + 1]]))
                });
                if values.iter().any(|v| !v.is_finite()) || !(0.0..=1.001).contains(&values[3]) {
                    return Err("non-finite or invalid raw voxel layer".into());
                }
                alpha.push(values[3]);
            }
            let area = alpha.iter().map(|v| *v as f64).sum::<f64>();
            let agreement=source_mask.as_ref().map(|source|{
                let source_area=source.iter().map(|v|*v as f64).sum::<f64>();
                let intersection=source.iter().zip(&alpha).map(|(s,v)|s.min(*v) as f64).sum::<f64>();
                let union=source.iter().zip(&alpha).map(|(s,v)|s.max(*v) as f64).sum::<f64>();
                json!({"source_covered_pixels":source_area,"volume_to_source_area_ratio":area/source_area.max(1.0),"soft_iou":intersection/union.max(1.0),"reference":"binary actual source visibility, no TAA or supersampling; comparison is not a parity guarantee"})
            });
            let alpha_pixels: Vec<u8> = alpha
                .iter()
                .flat_map(|v| {
                    let c = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    [c, c, c, 255]
                })
                .collect();
            render::write_png(&a.out.join("voxel-alpha.png"), *w, *h, &alpha_pixels)?;
            json!({"raw_alpha_area_pixels":area,"source_comparison":agreement})
        } else {
            let source = source_mask.as_ref().unwrap();
            let pixels: Vec<u8> = source
                .iter()
                .flat_map(|v| {
                    let c = (*v * 255.0) as u8;
                    [c, c, c, 255]
                })
                .collect();
            render::write_png(
                &a.out.join("source-crown-mask.png"),
                a.size.0,
                a.size.1,
                &pixels,
            )?;
            json!({"source_covered_pixels":source.iter().sum::<f32>()})
        };
        let record = json!({"mode":mode,"file":file,"first_frame_wall_ms":wall_ms,"gpu_ms":stats.gpu_ms,"clusters":stats.clusters,"triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles,"diagnostics":diag,"coverage":coverage});
        println!("{record}");
        records.push(record);
    }
    let report = json!({"schema":1,"purpose":"bounded real-GPU density foliage fixture; not gameplay FPS","adapter":gpu.info.name,"tree_count":a.count,"shared_source_assets":3,"level":a.level,"cell_m":assets.volumes[0].levels[a.level].cell_size,"output_size":[a.size.0,a.size.1],"camera":{"position":cam.pos.to_array(),"target":cam.target.to_array(),"fov_y":cam.fov_y},"debug_view":a.debug,"taa":false,"source_crowns_retained_for_shadow":true,"automatic_near_far_transition":false,"total_wall_seconds":begun.elapsed().as_secs_f64(),"complete_rays":!incomplete,"runs":records});
    std::fs::write(
        a.out.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if incomplete {
        return Err(
            "voxel step budget exhausted: diagnostic image saved, not valid quality evidence"
                .into(),
        );
    }
    Ok(())
}
fn half_to_f32(bits: u16) -> f32 {
    let negative = bits & 0x8000 != 0;
    let exponent = (bits >> 10) & 31;
    let fraction = bits & 1023;
    if exponent == 0 {
        let v = fraction as f32 * (1.0 / 16_777_216.0);
        return if negative { -v } else { v };
    }
    let sign = (bits as u32 & 0x8000) << 16;
    let exponent = if exponent == 31 {
        255
    } else {
        exponent as u32 + 112
    };
    f32::from_bits(sign | (exponent << 23) | ((fraction as u32) << 13))
}
fn main() {
    if let Err(e) = args().and_then(run) {
        eprintln!("voxel_forest_gpu: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_readback_decodes_coverage_and_invalid_values() {
        assert_eq!(half_to_f32(0), 0.0);
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(1), 2f32.powi(-24));
        assert_eq!(half_to_f32(0xbc00), -1.0);
        assert!(half_to_f32(0x7c00).is_infinite());
        assert!(half_to_f32(0x7e00).is_nan());
    }
    #[test]
    fn fixture_instances_are_reproducible_and_include_nonuniform_scale() {
        let a = positions(256);
        let b = positions(256);
        assert_eq!(a, b);
        assert_eq!(a.len(), 256);
        assert!(
            a.iter()
                .all(|xf| xf.is_finite() && xf.matrix3.determinant() > 0.0)
        );
        assert!(
            a.iter()
                .any(|xf| (xf.matrix3.x_axis.length() - xf.matrix3.y_axis.length()).abs() > 0.03)
        );
    }
}
