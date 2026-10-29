//! True GPU detail views of the shared spray source library, without tree assembly.
//! cargo run --locked --release -p cli --example plant_spray_gallery --
//!   --root GAME_ROOT --out FRESH_DIRECTORY
//! Six source views plus component-gallery.png; this is not a performance test.

#[allow(dead_code)]
#[path = "../src/plant_species.rs"]
mod plant_species;

use glam::{Affine3A, Vec3};
use image::RgbaImage;
use render::cluster::{ClusterMesh, GpuCluster, Part, SimplificationPolicy, Vertex};
use render::foliage_assembly::PartGeometry;
use render::scene::{GeometryCategory, Heightfield, Material, THIN_FOLIAGE, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 1080;

struct Asset {
    id: String,
    owners: std::ops::Range<usize>,
    bounds: [Vec3; 2],
    report: Value,
}

fn args() -> Result<(PathBuf, PathBuf), String> {
    let raw: Vec<_> = std::env::args().skip(1).collect();
    let usage = "usage: plant_spray_gallery --root GAME_ROOT --out FRESH_DIRECTORY";
    if raw.len() != 4 {
        return Err(usage.into());
    }
    let mut values = std::collections::HashMap::new();
    for pair in raw.chunks_exact(2) {
        if !["--root", "--out"].contains(&pair[0].as_str())
            || values.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(usage.into());
        }
    }
    Ok((
        PathBuf::from(values.get("--root").ok_or(usage)?),
        PathBuf::from(values.get("--out").ok_or(usage)?),
    ))
}

fn add_asset(scene: &mut Scene, family: &str, variant: u32) -> Result<Asset, String> {
    let (wood, foliage): (PartGeometry, PartGeometry) = match family {
        "needle" => render::plant_spray::needle_spray(variant),
        "broadleaf" => render::plant_spray::broadleaf_spray(variant),
        _ => return Err("unknown spray family".into()),
    };
    let recipe = &plant_species::find(if family == "needle" {
        "spruce"
    } else {
        "broadleaf"
    })?
    .recipe;
    let id = format!("{family}-spray-{variant}");
    let mut bounds = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
    for geometry in [&wood, &foliage] {
        if geometry.vertices.is_empty() || geometry.indices.is_empty() {
            return Err(format!("{id}: empty source geometry"));
        }
        for vertex in &geometry.vertices {
            let p = Vec3::from(vertex.pos);
            if !p.is_finite() {
                return Err(format!("{id}: nonfinite source vertex"));
            }
            bounds[0] = bounds[0].min(p);
            bounds[1] = bounds[1].max(p);
        }
    }
    let source_bounds = bounds;
    // Seat each component on the same neutral ground using a rigid instance
    // transform. Its source vertices, indices, normals and size remain intact.
    let offset = Vec3::new(0.0, -bounds[0].y, 0.0);
    bounds = bounds.map(|p| p + offset);
    let start = scene.instances.len();
    let mut parts = Vec::new();
    for (name, geometry, is_foliage) in [("wood", &wood, false), ("foliage", &foliage, true)] {
        let material = Material {
            tint: if is_foliage {
                recipe.foliage_colour
            } else {
                recipe.wood_colour
            },
            layer: -1,
            roughness: if is_foliage { 0.86 } else { 0.96 },
            flags: if is_foliage {
                THIN_FOLIAGE | TWO_SIDED
            } else {
                0
            },
            scale: 1.0,
            metal: 0.0,
        };
        let material_id = scene.add_material(material);
        let mesh = ClusterMesh::build_with_policy(
            &[Part {
                vertices: &geometry.vertices,
                indices: &geometry.indices,
                material: material_id,
            }],
            |_| {
                if is_foliage {
                    SimplificationPolicy::ThinFoliage
                } else {
                    SimplificationPolicy::TexturedGeometry
                }
            },
        );
        let mesh_id = scene.add_mesh(&mesh);
        scene.add_classified_instance(
            mesh_id,
            Affine3A::from_translation(offset),
            GeometryCategory::Trees,
        );
        parts.push(json!({"name":name,"source_id":geometry.id,
            "source_vertices":geometry.vertices.len(),"source_triangles":geometry.indices.len()/3,
            "source_geometry_bytes":geometry.vertices.len()*std::mem::size_of::<Vertex>()+geometry.indices.len()*4,
            "cooked_geometry_bytes":mesh.vertices.len()*std::mem::size_of::<Vertex>()
                +(mesh.cluster_verts.len()+mesh.tris.len())*4+mesh.clusters.len()*std::mem::size_of::<GpuCluster>(),
            "clusters":mesh.clusters.len(),"levels":mesh.levels,
            "material":{"tint":material.tint,"roughness":material.roughness,"flags":material.flags}}));
    }
    Ok(Asset {
        id: id.clone(),
        owners: start..scene.instances.len(),
        bounds,
        report: json!({"id":id,"family":family,"variant":variant,"parts":parts,
            "source_bounds_m":source_bounds.map(|p|p.to_array()),
            "display_bounds_m":bounds.map(|p|p.to_array()),"placement_offset_m":offset.to_array(),
            "source_extent_m":(source_bounds[1]-source_bounds[0]).to_array(),
            "geometry_path":"Two ordinary scene instances reference the immutable wood/foliage source meshes. No whole-tree flattening, textures or alpha cards.",
            "material_source":if family=="needle" {"spruce recipe palette"} else {"broadleaf recipe palette"}}),
    })
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
        [-20.0, -0.015, -20.0],
        [-20.0, -0.015, 20.0],
        [20.0, -0.015, -20.0],
        [20.0, -0.015, 20.0],
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
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield {
        origin: [-20.0; 2],
        cell: 0.5,
        w: 81,
        h: 81,
        heights: vec![-0.015; 81 * 81],
    };
}

fn camera(bounds: [Vec3; 2]) -> Camera {
    let centre = (bounds[0] + bounds[1]) * 0.5;
    let half = (bounds[1] - bounds[0]) * 0.5;
    let direction = Vec3::new(0.25, 0.10, 1.0).normalize();
    let right = Vec3::Y.cross(direction).normalize();
    let up = direction.cross(right);
    let fov_y = 35.0f32.to_radians();
    let tan_y = (fov_y * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut distance = 0.1f32;
    for corner in 0..8 {
        let p = half
            * Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
        distance = distance.max(p.dot(direction) + 1.18 * p.dot(right).abs() / (tan_y * aspect));
        distance = distance.max(p.dot(direction) + 1.18 * p.dot(up).abs() / tan_y);
    }
    Camera {
        pos: centre + direction * distance,
        target: centre,
        fov_y,
        near: 0.01,
    }
}

fn run(root: &Path, out: &Path) -> Result<(), String> {
    if out.as_os_str().is_empty() || out.exists() {
        return Err("--out must name a fresh directory".into());
    }
    let mut scene = Scene::default();
    scene.load_sky(&root.join("assets/sky/sky_2k.hdr"))?;
    let mut assets = Vec::new();
    for family in ["needle", "broadleaf"] {
        for variant in 0..3 {
            assets.push(add_asset(&mut scene, family, variant)?);
        }
    }
    let sources = scene.instances.clone();
    ground(&mut scene);
    let mut light = Lighting {
        taa: 0.0,
        sharpen: 0.0,
        lod_pixels: 0.0,
        gi: 1.0,
        fog_density: 0.0,
        fog_start: 10000.0,
        fog_end: 20000.0,
        ..Default::default()
    };
    light.set("sun_elevation=36")?;
    light.set("sun_azimuth=135")?;
    let gpu = Gpu::new()?;
    let renderer = Renderer::new(&gpu, &scene, &light);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut gallery = RgbaImage::new(1920, 1080);
    let mut captures = Vec::new();
    for (index, asset) in assets.iter().enumerate() {
        let family = index / 3;
        let family_bounds = assets[family * 3..family * 3 + 3].iter().fold(
            [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)],
            |bounds, item| [bounds[0].min(item.bounds[0]), bounds[1].max(item.bounds[1])],
        );
        let camera = camera(family_bounds);
        let mut owners = sources.clone();
        for (owner, instance) in owners.iter_mut().enumerate() {
            if !asset.owners.contains(&owner) {
                instance.cluster_count = 0;
            }
        }
        renderer.update_instances(&gpu, 0, &owners)?;
        renderer.set_lighting_focus(&[camera.pos])?;
        renderer.set_sun_shadow_focus(&[camera.target])?;
        renderer.invalidate_shadow();
        for _ in 0..2 {
            renderer.render(&gpu, &camera, &light, WIDTH, HEIGHT);
        }
        renderer.invalidate_shadow();
        let (pixels, stats) = renderer.render(&gpu, &camera, &light, WIDTH, HEIGHT);
        if stats.triangles <= 2 || stats.shadow_triangles <= 2 {
            return Err(format!("{}: missing component geometry/shadows", asset.id));
        }
        let image =
            RgbaImage::from_raw(WIDTH, HEIGHT, pixels).ok_or("invalid renderer pixel buffer")?;
        let filename = format!("{}.png", asset.id);
        image.save(out.join(&filename)).map_err(|e| e.to_string())?;
        let thumbnail =
            image::imageops::resize(&image, 640, 540, image::imageops::FilterType::Lanczos3);
        image::imageops::replace(
            &mut gallery,
            &thumbnail,
            (index % 3) as i64 * 640,
            (index / 3) as i64 * 540,
        );
        captures.push(json!({"file":filename,"source":asset.id,"size":[WIDTH,HEIGHT],"gallery_cell":[index%3,index/3],
            "camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y,
                "pixels_per_metre_at_target":HEIGHT as f32/(2.0*camera.pos.distance(camera.target)*(camera.fov_y*0.5).tan())},
            "submitted_triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles,"warmup_frames":2}));
    }
    gallery
        .save(out.join("component-gallery.png"))
        .map_err(|e| e.to_string())?;
    let info = gpu.summary();
    let report = json!({"schema":1,"fixture":"shared_spray_source_gallery","root":root,
        "gpu":{"name":info.name,"backend":info.backend,"driver":info.driver},
        "assets":assets.iter().map(|asset|&asset.report).collect::<Vec<_>>(),"captures":captures,
        "lighting":{"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,"ev100":light.ev100,
            "gi":light.gi,"near_shadows":light.near_shadows,"sky":"assets/sky/sky_2k.hdr","taa":false,"lod_pixels":0.0},
        "composition":"component-gallery.png 1920x1080: needle variants 0,1,2 top; broadleaf variants 0,1,2 bottom. Each native 1280x1080 GPU image is downsampled 2:1, without crop or recolouring.",
        "limits":["A common camera per spray family preserves the physical size of its three variants. Family cameras can differ; inspect reported scale.",
            "Rest-pose source geometry only. No animation, voxel representation, forest population, streaming, or performance claim.",
            "Finest source CLOD is forced; this does not validate far-detail retention or temporal antialiasing.",
            "Triangle submission counts include the neutral ground. Source and resident byte counts exclude instance buffers."]});
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("Spray source gallery saved to {}", out.display());
    Ok(())
}

fn main() {
    if let Err(error) = args().and_then(|(root, out)| run(&root, &out)) {
        eprintln!("plant spray gallery: {error}");
        std::process::exit(1);
    }
}
