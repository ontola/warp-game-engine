//! Small native GPU A/B of the retained tree and one opaque species assembly.
//! cargo run --locked --release -p cli --example tree_species_compare --
//!   --root GAME_ROOT --out FRESH_DIRECTORY [--candidate spruce] [--time 4.25] [--taa 0|1]
//! Same camera, ground, light and wind; each source retains its material path.
//! No game process, forest generation, voxel bake or performance claim.

#[allow(dead_code)]
#[path = "../src/plant_species.rs"]
mod plant_species;
#[allow(dead_code)]
#[path = "../src/tree_assets.rs"]
mod tree_assets;

use glam::{Affine3A, Mat4, Vec2, Vec3};
use image::RgbaImage;
use render::cluster::{ClusterMesh, GpuCluster, Part, SimplificationPolicy, Vertex};
use render::scene::{
    ALPHA_TEST, FOLIAGE, GeometryCategory, Heightfield, Material, PART_WIND, TWO_SIDED,
};
use render::vegetation_asset::{AttachLimits, CookedPlant, PlantInstance, SceneVegetation};
use render::wind::WindState;
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use serde_json::{Value, json};
use sideways::vegetation::TreeArchetype;
use std::path::PathBuf;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FULLNESS: f32 = 1.25;
const SEED: u64 = 20261028;

struct Args {
    root: PathBuf,
    out: PathBuf,
    candidate: String,
    time: f64,
    taa: bool,
}
struct Asset {
    bounds: [Vec3; 2],
    report: Value,
}

fn args() -> Result<Args, String> {
    let raw: Vec<_> = std::env::args().skip(1).collect();
    let usage = "usage: tree_species_compare --root GAME_ROOT --out FRESH_DIRECTORY [--candidate spruce] [--time 4.25] [--taa 0|1]";
    if raw.is_empty() || raw.len() % 2 != 0 {
        return Err(usage.into());
    }
    let mut values = std::collections::HashMap::new();
    for pair in raw.chunks_exact(2) {
        if !["--root", "--out", "--candidate", "--time", "--taa"].contains(&pair[0].as_str())
            || values.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(format!("unknown or repeated option {}. {usage}", pair[0]));
        }
    }
    let time: f64 = values
        .get("--time")
        .copied()
        .unwrap_or("4.25")
        .parse()
        .map_err(|_| "--time requires seconds as a finite nonnegative number")?;
    if !time.is_finite() || time < 0.0 {
        return Err("--time requires seconds as a finite nonnegative number".into());
    }
    let taa = match values.get("--taa").copied().unwrap_or("0") {
        "0" => false,
        "1" => true,
        _ => return Err("--taa must be 0 or 1".into()),
    };
    Ok(Args {
        root: PathBuf::from(values.get("--root").ok_or(usage)?),
        out: PathBuf::from(values.get("--out").ok_or(usage)?),
        candidate: values
            .get("--candidate")
            .copied()
            .unwrap_or("spruce")
            .into(),
        time,
        taa,
    })
}

fn materials(scene: &mut Scene, root: &std::path::Path) -> Result<[u32; 4], String> {
    // Exact retained Sideways/paired_forest material setup. add_texture also
    // loads each sibling normal map, including authored_pine_normal.png.
    let mut ids = [0; 4];
    for (index, (file, flags, tint)) in [
        ("trunk_a_albedo.jpg", 0, [1.0; 3]),
        ("trunk_b_albedo.jpg", 0, [1.0; 3]),
        ("bark_albedo.jpg", 0, [1.0; 3]),
        (
            "authored_pine_albedo.png",
            ALPHA_TEST | TWO_SIDED,
            [0.62, 0.66, 0.55],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let layer = scene.add_texture(&root.join("assets/trees").join(file))?;
        ids[index] = scene.add_material(Material {
            tint,
            layer,
            roughness: 0.9,
            flags,
            scale: 1.0,
            metal: 0.0,
        });
    }
    Ok(ids)
}

fn cook_reference(
    scene: &mut Scene,
    surfaces: tree_assets::Surfaces,
    materials: [u32; 4],
    id: &str,
) -> Result<Asset, String> {
    let fronds = tree_assets::crown_frond_count(&surfaces)?;
    let mut bounds = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
    let mut rows = Vec::new();
    let mut parts = Vec::new();
    for (name, vertices, indices) in &surfaces {
        for vertex in vertices {
            let p = Vec3::from(vertex.pos);
            if !p.is_finite() {
                return Err(format!("{id}: nonfinite source vertex"));
            }
            bounds[0] = bounds[0].min(p);
            bounds[1] = bounds[1].max(p);
        }
        let material = if name.contains("trunk_b") {
            materials[1]
        } else if name.contains("trunk") {
            materials[0]
        } else if name.contains("bark") {
            materials[2]
        } else {
            materials[3]
        };
        rows.push(json!({"name": name, "vertices": vertices.len(), "triangles": indices.len()/3,
            "material": material, "source_geometry_bytes": vertices.len()*std::mem::size_of::<Vertex>()+indices.len()*4}));
        parts.push(Part {
            vertices,
            indices,
            material,
        });
    }
    let mesh = ClusterMesh::build_with_policy(&parts, |part| {
        if part.material == materials[3] {
            SimplificationPolicy::AlphaCards
        } else {
            SimplificationPolicy::TexturedGeometry
        }
    });
    let mesh_id = scene.add_mesh(&mesh);
    scene.add_classified_instance(mesh_id, Affine3A::IDENTITY, GeometryCategory::Trees);
    scene.mark_foliage();
    Ok(Asset {
        bounds,
        report: json!({
            "id": id, "fullness": FULLNESS, "fronds": fronds, "surfaces": rows,
            "material_path": "retained textured alpha cards and normal maps; AlphaCards crown CLOD",
            "bounds_m": bounds.map(|p| p.to_array()), "source_extent_m": (bounds[1]-bounds[0]).to_array(),
            "source_triangles": mesh.leaf_triangles(), "root_triangles": mesh.root_triangles(),
            "clusters": mesh.clusters.len(), "levels": mesh.levels,
            "cooked_geometry_bytes": mesh.vertices.len()*std::mem::size_of::<Vertex>()
                +(mesh.cluster_verts.len()+mesh.tris.len())*4+mesh.clusters.len()*std::mem::size_of::<GpuCluster>(),
        }),
    })
}

fn cook_candidate(scene: &mut Scene, id: &str) -> Result<Asset, String> {
    let species = plant_species::find(id)?;
    let assembly = plant_species::build(id, SEED)?;
    assembly.validate()?;
    let palette: Vec<_> = assembly
        .materials
        .iter()
        .map(|material| {
            json!({"tint": material.tint, "layer": material.layer,
            "flags": material.flags, "roughness": material.roughness})
        })
        .collect();
    // Measure rest bounds without allocating transformed copies of the source.
    // The GPU receives shared primitives, compact placements and branch poses.
    let mut bounds = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
    let branches = assembly.branch_transforms()?;
    for placement in &assembly.instances {
        let transform = branches[placement.branch as usize] * placement.rest;
        for vertex in &assembly.parts[placement.part as usize].vertices {
            let p = transform.transform_point3(Vec3::from(vertex.pos));
            if !p.is_finite() {
                return Err(format!("{id}: nonfinite transformed source vertex"));
            }
            bounds[0] = bounds[0].min(p);
            bounds[1] = bounds[1].max(p);
        }
    }
    if !bounds[0].is_finite() || !bounds[1].is_finite() {
        return Err(format!("{id}: source has no finite bounds"));
    }
    let cooked = CookedPlant::cook_source(&assembly)?;
    let stats = cooked.stats();
    let limits = AttachLimits {
        max_primitive_instances: render::part_wind::MAX_ATTACHMENTS,
        ..Default::default()
    };
    let mut builder = SceneVegetation::new(scene, limits)?;
    let handle = builder.append(
        &cooked,
        PlantInstance {
            id: SEED,
            generation: 0,
            root: Mat4::IDENTITY,
            category: GeometryCategory::Trees,
        },
    )?;
    let attachment = builder.finish_source()?;
    if scene.instances[handle.instance_range.clone()]
        .iter()
        .any(|instance| instance.flags & PART_WIND == 0 || instance.flags & FOLIAGE != 0)
    {
        return Err("shared candidate must use PartWind without whole-tree shear".into());
    }
    Ok(Asset {
        bounds,
        report: json!({
            "id": species.id, "name": species.name, "is_tree": species.is_tree, "seed": SEED,
            "material_path": "validated opaque untextured assembly palette; shared CookedPlant primitives",
            "wind_path": "shared parent-first GPU branch poses and primitive attachments; no whole-tree shear",
            "materials": palette, "bounds_m": bounds.map(|p| p.to_array()),
            "source_extent_m": (bounds[1]-bounds[0]).to_array(),
            "assembly_primitive_count": assembly.parts.len(),
            "cook_stats": {
                "primitive_meshes": stats.primitive_meshes, "primitive_vertices": stats.primitive_vertices,
                "primitive_triangles": stats.primitive_triangles, "primitive_bytes": stats.primitive_bytes,
                "placements": stats.placements, "branches": stats.branches, "volume_assets": stats.volume_assets,
                "expanded_source_vertices": stats.expanded_source_vertices,
                "expanded_source_triangles": stats.expanded_source_triangles,
                "expanded_source_bytes": stats.expanded_source_bytes,
            },
            "attachment_report": {
                "plants": attachment.plants, "primitive_meshes": attachment.primitive_meshes,
                "primitive_instances": attachment.primitive_instances, "branch_proxies": attachment.branch_proxies,
                "pose_nodes": attachment.pose_nodes, "volume_instances": attachment.volume_instances,
                "volume_assets": attachment.volume_assets, "source_vertices_shared": attachment.source_vertices_shared,
                "resident_vertices_shared": attachment.resident_vertices_shared,
                "source_triangles_shared": attachment.source_triangles_shared,
                "shared_geometry_bytes": attachment.shared_geometry_bytes,
                "expanded_source_bytes": attachment.expanded_source_bytes,
                "packed_volume_bytes": attachment.packed_volume_bytes,
            },
            "accounting_limits": "Expanded source counts/bytes are an arithmetic counterfactual, not allocated geometry. Shared geometry bytes include all resident primitive CLOD levels; instance, attachment and pose buffers are additional.",
        }),
    })
}

fn ground(scene: &mut Scene) {
    let mat = scene.add_material(Material {
        tint: [0.42; 3],
        layer: -1,
        roughness: 1.0,
        flags: 0,
        scale: 1.0,
        metal: 0.0,
    });
    let vertices = [
        [-400.0, -0.015, -400.0],
        [-400.0, -0.015, 400.0],
        [400.0, -0.015, -400.0],
        [400.0, -0.015, 400.0],
    ]
    .map(|pos| Vertex {
        pos,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0; 2],
    });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 1, 2, 2, 1, 3],
        material: mat,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield {
        origin: [-400.0; 2],
        cell: 4.0,
        w: 201,
        h: 201,
        heights: vec![-0.015; 201 * 201],
    };
}

fn camera(bounds: [Vec3; 2]) -> Camera {
    let centre = (bounds[0] + bounds[1]) * 0.5;
    let half = (bounds[1] - bounds[0]) * 0.5;
    let direction = Vec3::new(0.34, 0.10, 1.0).normalize();
    let right = Vec3::Y.cross(direction).normalize();
    let up = direction.cross(right);
    let fov_y = 40.0f32.to_radians();
    let tan_y = (fov_y * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut distance = 1.0f32;
    for corner in 0..8 {
        let p = half
            * Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
        distance = distance.max(p.dot(direction) + 1.25 * p.dot(right).abs() / (tan_y * aspect));
        distance = distance.max(p.dot(direction) + 1.25 * p.dot(up).abs() / tan_y);
    }
    Camera {
        pos: centre + direction * distance,
        target: centre,
        fov_y,
        near: 0.05,
    }
}

fn capture(
    gpu: &Gpu,
    renderer: &Renderer,
    light: &Lighting,
    camera: Camera,
    path: &std::path::Path,
) -> Result<(RgbaImage, Value), String> {
    renderer.set_lighting_focus(&[camera.pos])?;
    renderer.set_sun_shadow_focus(&[Vec3::ZERO])?;
    renderer.reset_history();
    renderer.invalidate_shadow();
    let warmup_frames = if light.taa > 0.0 { 32 } else { 2 };
    // Wind time is fixed by the caller for this entire capture. TAA accumulates
    // one stationary pose; this does not exercise temporal motion quality.
    for _ in 0..warmup_frames {
        renderer.render(gpu, &camera, light, WIDTH, HEIGHT);
    }
    renderer.invalidate_shadow();
    let (pixels, stats) = renderer.render(gpu, &camera, light, WIDTH, HEIGHT);
    if stats.triangles == 0 || stats.shadow_triangles == 0 {
        return Err("capture submitted no geometry or shadows".into());
    }
    let image =
        RgbaImage::from_raw(WIDTH, HEIGHT, pixels).ok_or("invalid renderer pixel buffer")?;
    image.save(path).map_err(|e| e.to_string())?;
    Ok((
        image,
        json!({"file":path.file_name().and_then(|n|n.to_str()),"size":[WIDTH,HEIGHT],
        "camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y,"near":camera.near,
            "vertical_metres_at_target":2.0*camera.pos.distance(camera.target)*(camera.fov_y*0.5).tan(),
            "pixels_per_metre_at_target":HEIGHT as f32/(2.0*camera.pos.distance(camera.target)*(camera.fov_y*0.5).tan())},
        "lod_pixels":light.lod_pixels,"triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles,
        "taa":light.taa,"warmup_frames":warmup_frames,"history_reset_before_warmup":true,
        "gpu_timestamps_available":!stats.gpu_ms.is_empty(),"gpu_ms":stats.gpu_ms,
        "gpu_ms_scope":"One captured-frame GPU timestamp sample after fixed-pose warmup and a forced shadow redraw, captured through synchronous readback. Not steady-state timing, a pass-sum benchmark, frame wall time or gameplay FPS. Empty means unavailable.",
        "temporal_scope":"Stationary camera and fixed wind time throughout accumulation; this does not establish temporal acceptance."}),
    ))
}

fn pair(left: &RgbaImage, right: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::new(WIDTH, HEIGHT / 2);
    for (index, image) in [left, right].into_iter().enumerate() {
        let thumbnail = image::imageops::resize(
            image,
            WIDTH / 2,
            HEIGHT / 2,
            image::imageops::FilterType::Lanczos3,
        );
        image::imageops::replace(&mut out, &thumbnail, index as i64 * (WIDTH / 2) as i64, 0);
    }
    out
}

fn run(args: Args) -> Result<(), String> {
    if args.out.as_os_str().is_empty() || args.out.exists() {
        return Err("--out must name a fresh directory".into());
    }
    let mut reference_scene = Scene::default();
    let material_ids = materials(&mut reference_scene, &args.root)?;
    reference_scene.load_sky(&args.root.join("assets/sky/sky_2k.hdr"))?;
    let reference = cook_reference(
        &mut reference_scene,
        tree_assets::load_with_fullness(&args.root, TreeArchetype::Canopy, FULLNESS)?,
        material_ids,
        "reference_canopy",
    )?;
    let mut candidate_scene = Scene::default();
    candidate_scene.load_sky(&args.root.join("assets/sky/sky_2k.hdr"))?;
    let candidate = cook_candidate(&mut candidate_scene, &args.candidate)?;
    let close = camera([
        reference.bounds[0].min(candidate.bounds[0]),
        reference.bounds[1].max(candidate.bounds[1]),
    ]);
    let far = Camera {
        pos: close.target + (close.pos - close.target) * 8.0,
        ..close
    };
    let detail = camera(candidate.bounds);
    ground(&mut reference_scene);
    ground(&mut candidate_scene);
    let mut light = Lighting {
        taa: if args.taa { 1.0 } else { 0.0 },
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
    let wind = WindState {
        strength: 0.8,
        time_seconds: args.time,
        direction: Vec2::new(0.8, 0.6),
        ..Default::default()
    };
    std::fs::create_dir_all(&args.out).map_err(|e| e.to_string())?;
    let gpu = Gpu::new()?;
    let mut images: [Vec<RgbaImage>; 2] = [Vec::new(), Vec::new()];
    let mut comparisons = Vec::new();
    let mut captures = Vec::new();
    // Isolated scenes avoid hiding instances owned by the GPU attachment pass,
    // and prevent the inactive source contributing any lighting or shadows.
    for (label, scene) in [
        ("reference", &reference_scene),
        ("candidate", &candidate_scene),
    ] {
        light.lod_pixels = 0.0;
        let renderer = Renderer::new(&gpu, scene, &light);
        renderer.set_wind(&gpu, &wind)?;
        for (view, (distance, camera, lod)) in [("close", close, 0.0), ("far", far, 1.0)]
            .into_iter()
            .enumerate()
        {
            light.lod_pixels = lod;
            let (image, mut report) = capture(
                &gpu,
                &renderer,
                &light,
                camera,
                &args.out.join(format!("{distance}-{label}.png")),
            )?;
            report["source"] = json!(label);
            captures.push(report);
            images[view].push(image);
        }
        if label == "candidate" {
            light.lod_pixels = 0.0;
            // Same distant camera with all source geometry: isolate triangle
            // simplification loss from subpixel rasterization/material loss.
            let (_, mut report) = capture(
                &gpu,
                &renderer,
                &light,
                far,
                &args.out.join("far-source-candidate.png"),
            )?;
            report["source"] = json!(label);
            report["framing"] = json!(
                "same far camera with source CLOD; compare against far-candidate.png to isolate simplification"
            );
            captures.push(report);
            let (_, mut report) = capture(
                &gpu,
                &renderer,
                &light,
                detail,
                &args.out.join("detail-candidate.png"),
            )?;
            report["source"] = json!(label);
            report["framing"] = json!(
                "candidate-only fit; inspect detail, not a matched-scale reference comparison"
            );
            captures.push(report);
        }
    }
    for (view, distance) in ["close", "far"].into_iter().enumerate() {
        let comparison = pair(&images[view][0], &images[view][1]);
        comparison
            .save(args.out.join(format!("comparison-{distance}.png")))
            .map_err(|e| e.to_string())?;
        comparisons.push(comparison);
    }
    let mut overview = RgbaImage::new(WIDTH, HEIGHT);
    for (row, comparison) in comparisons.iter().enumerate() {
        image::imageops::replace(
            &mut overview,
            comparison,
            0,
            row as i64 * (HEIGHT / 2) as i64,
        );
    }
    overview
        .save(args.out.join("comparison.png"))
        .map_err(|e| e.to_string())?;
    let info = gpu.summary();
    let report = json!({"schema":4,"fixture":"retained_tree_species_comparison","candidate":args.candidate,
        "root":args.root,"command":std::env::args().collect::<Vec<_>>(),"size":[WIDTH,HEIGHT],
        "gpu":{"name":info.name,"backend":info.backend,"driver":info.driver},
        "sources":[reference.report,candidate.report],"captures":captures,
        "reference_material_setup":"paired_forest retained textures and normal maps; bark tint1, crown tint [.62,.66,.55], roughness .9, ALPHA_TEST|TWO_SIDED",
        "candidate_material_setup":"plant_species::build opaque palette and shared primitive meshes through CookedPlant::cook_source/SceneVegetation::finish_source; THIN_FOLIAGE materials retain their transmission response; no textures or alpha test",
        "lighting":{"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,"sky_lux":light.sky_lux,
            "ev100":light.ev100,"gi":light.gi,"near_shadows":light.near_shadows,"alpha_cut":light.alpha_cut,
            "sky":"assets/sky/sky_2k.hdr","taa":args.taa,
            "warmup_frames_per_capture":if args.taa {32} else {2}},
        "wind":{"strength":wind.strength,"time_seconds":wind.time_seconds,"direction":wind.direction.to_array(),
            "reference":"retained whole-tree GPU shear","reference_tree_shear":wind.tree_shear,
            "candidate":"GPU PartWind rigid branch hierarchy; no extra whole-tree shear"},
        "composition":{"comparison.png":"1920x1080: top close, bottom far; left reference, right candidate",
            "far-source-candidate.png":"1920x1080: candidate at the identical far camera with source CLOD instead of 1px simplification",
            "detail-candidate.png":"1920x1080: candidate-only fit with independently reported camera scale",
            "comparison-close.png":"1920x540: left reference, right candidate",
            "comparison-far.png":"1920x540: left reference, right candidate",
            "method":"2:1 Lanczos downsample of complete GPU captures, without crop, recolouring or relighting"},
        "limits":["One reference and one candidate, same origin and identical cameras/light/wind field. Each has an isolated scene/renderer; no inactive tree participates in shadows or lighting.",
            "Close forces source CLOD for inspection; far uses ordinary 1px CLOD at eight times the close-camera distance.",
            "Native single-view synthetic ground fixture, not stage population, gameplay FPS, temporal smoothness or optical acceptance.",
            "This is a visual source comparison across different material and animation paths, not an isolated shader or simplification-policy experiment.",
            "The candidate uses bounded shared source geometry and GPU branch poses. This fixture does not attach derived voxels or claim whole-forest streaming/residency.",
            "One fixed wind pose only. TAA uses 32 stationary warmup frames; raw uses two. Static accumulation does not validate moving-camera or wind temporal quality.",
            "Counts include synthetic ground. GPU scope values describe only each captured frame with forced shadow redraw, not a steady benchmark or gameplay cost.",
            "Source sizes are deliberately not normalized; the common camera preserves metre scale and exposes stature differences."]});
    std::fs::write(
        args.out.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "Tree comparison saved to {}. Reference left, candidate right; close above, far below.",
        args.out.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = args().and_then(run) {
        eprintln!("tree species compare: {error}");
        std::process::exit(1);
    }
}
