//! Bounded native comparison of many real shared tree owners and world groups.
//! Explicit invocation only. No window, game process management or FPS claim.
//! cargo run --release -p cli --example forest_group_gpu -- --out FRESH_DIR
#[path = "../src/tree_assets.rs"]
mod tree_assets;

use glam::{Affine3A, Quat, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex};
use render::foliage_volume_data::{ALL_MATERIALS, VolumeBinding};
use render::foliage_voxel::{BakeConfig, ProjectionMeasure, VoxelAsset, bake};
use render::foliage_world::{WorldConfig, WorldField};
use render::scene::{ALPHA_TEST, GeometryCategory, Heightfield, Material, THIN_FOLIAGE, TWO_SIDED};
use render::vegetation_accounting::Accounting;
use render::wind::WindState;
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use sideways::vegetation::{TreeArchetype, Vegetation};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug)]
struct Args {
    out: PathBuf,
    trees: usize,
    size: [u32; 2],
    views: usize,
    frames: usize,
    warmup: usize,
    mode: String,
    lights: String,
    seed: u64,
    cell: f32,
    wind_times: Vec<f64>,
    png: bool,
    lod_pixels: f32,
}
impl Default for Args {
    fn default() -> Self {
        Self {
            out: PathBuf::new(),
            trees: 16_384,
            size: [1280, 720],
            views: 4,
            frames: 24,
            warmup: 8,
            mode: "both".into(),
            lights: "both".into(),
            seed: 1,
            cell: 0.5,
            wind_times: vec![1.9, 2.7],
            png: true,
            lod_pixels: 1.0,
        }
    }
}
fn parse_args(raw: &[String]) -> Result<Args, String> {
    if raw.len() % 2 != 0 {
        return Err("expected --key value pairs".into());
    }
    let mut a = Args::default();
    for pair in raw.chunks_exact(2) {
        match pair[0].as_str() {
            "--out" => a.out = pair[1].clone().into(),
            "--trees" => a.trees = pair[1].parse().map_err(|_| "invalid tree count")?,
            "--views" => a.views = pair[1].parse().map_err(|_| "invalid view count")?,
            "--frames" => a.frames = pair[1].parse().map_err(|_| "invalid frame count")?,
            "--warmup" => a.warmup = pair[1].parse().map_err(|_| "invalid warmup")?,
            "--mode" => a.mode = pair[1].clone(),
            "--lights" => a.lights = pair[1].clone(),
            "--seed" => a.seed = pair[1].parse().map_err(|_| "invalid seed")?,
            "--cell" => a.cell = pair[1].parse().map_err(|_| "invalid voxel cell size")?,
            "--lod-pixels" => a.lod_pixels = pair[1].parse().map_err(|_| "invalid LOD pixels")?,
            "--wind-times" => {
                a.wind_times = pair[1]
                    .split(',')
                    .map(|s| s.parse().map_err(|_| "invalid wind time".into()))
                    .collect::<Result<_, String>>()?
            }
            "--png" => {
                a.png = match pair[1].as_str() {
                    "0" => false,
                    "1" => true,
                    _ => return Err("--png expects0 or1".into()),
                }
            }
            "--size" => {
                let (w, h) = pair[1].split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
                a.size = [
                    w.parse().map_err(|_| "invalid width")?,
                    h.parse().map_err(|_| "invalid height")?,
                ];
            }
            _ => return Err(format!("unknown argument {}", pair[0])),
        }
    }
    if a.out.as_os_str().is_empty()
        || !(1..=65_536).contains(&a.trees)
        || ![1, 4].contains(&a.views)
        || !(128..=3840).contains(&a.size[0])
        || !(128..=2160).contains(&a.size[1])
        || !(1..=128).contains(&a.frames)
        || a.warmup > 64
        || !["source", "group", "both"].contains(&a.mode.as_str())
        || !["clear", "low", "both"].contains(&a.lights.as_str())
        || !a.cell.is_finite()
        || !(0.25..=2.).contains(&a.cell)
        || !a.lod_pixels.is_finite()
        || !(0.25..=4.).contains(&a.lod_pixels)
        || a.wind_times.is_empty()
        || a.wind_times.len() > 4
        || a.wind_times.iter().any(|t| !t.is_finite() || *t < 0.)
    {
        return Err("bounded fixture: 1..65536 trees, 1|4 native views, 128..3840x128..2160, 1..128 frames, 0..64 warmup, source|group|both, clear|low|both, cell .25..2m, LOD .25..4px, 1..4 finite nonnegative wind times".into());
    }
    Ok(a)
}

#[derive(Clone, Copy)]
struct Hash(u64);
impl Hash {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 = (self.0 ^ u64::from(*x)).wrapping_mul(0x100000001b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn finish(self) -> String {
        format!("fnv1a64:{:016x}", self.0)
    }
}
fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
fn unit(seed: u64, index: usize, salt: u64) -> f32 {
    let mut n = seed ^ (index as u64).wrapping_mul(0x9e3779b97f4a7c15) ^ salt;
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((n ^ (n >> 31)) >> 40) as f32 / 16_777_216.
}
fn placements(count: usize, seed: u64) -> Vec<(usize, Affine3A)> {
    let columns = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(columns);
    let phenotype = Vegetation::default();
    (0..count)
        .map(|i| {
            let x = (i % columns) as f32 * 6. - (columns - 1) as f32 * 3.
                + (unit(seed, i, 11) - 0.5) * 1.8;
            let z = (i / columns) as f32 * 6. - (rows - 1) as f32 * 3.
                + (unit(seed, i, 89) - 0.5) * 1.8;
            let tree = phenotype.tree(seed, x, z);
            let scale = tree_assets::scale_from_units(unit(seed, i, 31), unit(seed, i, 71), tree);
            (
                tree.archetype.index(),
                Affine3A::from_scale_rotation_translation(
                    scale,
                    Quat::from_rotation_y(unit(seed, i, 17) * std::f32::consts::TAU),
                    Vec3::new(x, 0., z),
                ),
            )
        })
        .collect()
}
type OwnedPart = (Vec<Vertex>, Vec<u32>, u32);
struct Fixture {
    scene: Scene,
    volumes: Vec<VoxelAsset>,
    owners: Vec<(u32, u32)>,
    accounting: Accounting,
    half: f32,
    inventory: Value,
}
fn fixture(a: &Args) -> Result<Fixture, String> {
    let begin = Instant::now();
    let root = sideways::game_assets::root();
    let mut scene = Scene::default();
    let mut files = Vec::new();
    let mut material_ids = Vec::new();
    for (file, flags, tint) in [
        ("trunk_a_albedo.jpg", 0, [1.; 3]),
        ("trunk_b_albedo.jpg", 0, [1.; 3]),
        ("bark_albedo.jpg", 0, [1.; 3]),
        (
            "authored_pine_albedo.png",
            ALPHA_TEST | TWO_SIDED,
            [0.62, 0.66, 0.55],
        ),
    ] {
        let path = root.join("assets/trees").join(file);
        let mut hash = Hash::new();
        hash.bytes(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
        files.push(json!({"path":path,"content_hash":hash.finish()}));
        let layer = scene.add_texture(&path)?;
        material_ids.push(scene.add_material(Material {
            tint,
            layer,
            roughness: 0.9,
            flags,
            scale: 1.,
            metal: 0.,
        }));
    }
    let mut source_hash = Hash::new();
    let mut meshes = Vec::new();
    let mut volumes = Vec::new();
    let mut assets = Vec::new();
    let mut bake_ms = 0.;
    for kind in TreeArchetype::ALL {
        let path = root.join(format!(
            "assets/meshes/fir_{}_near.kmesh",
            ["a", "b", "c"][kind.index()]
        ));
        let mut hash = Hash::new();
        hash.bytes(&std::fs::read(&path).map_err(|e| e.to_string())?);
        files.push(json!({"path":path,"content_hash":hash.finish()}));
        let owned: Vec<OwnedPart> = tree_assets::load_with_fullness(&root, kind, 1.25)?
            .into_iter()
            .map(|(name, v, i)| {
                let m = if name.contains("trunk_b") {
                    0usize + 1
                } else if name.contains("trunk") {
                    0
                } else if name.contains("bark") {
                    2
                } else {
                    3
                };
                (v, i, material_ids[m])
            })
            .collect();
        for (v, i, m) in &owned {
            source_hash.u32(*m);
            for p in v {
                for x in p.pos.into_iter().chain(p.normal).chain(p.uv) {
                    source_hash.f32(x);
                }
            }
            for id in i {
                source_hash.u32(*id);
            }
        }
        let parts: Vec<_> = owned
            .iter()
            .map(|(v, i, m)| Part {
                vertices: v,
                indices: i,
                material: *m,
            })
            .collect();
        let mesh = ClusterMesh::build_with_policy(&parts, |part| {
            if part.material == material_ids[3] {
                SimplificationPolicy::AlphaCards
            } else {
                SimplificationPolicy::TexturedGeometry
            }
        });
        let mut asset = json!({"archetype":kind.label(),"finest_triangles":mesh.leaf_triangles(),"terminal_triangles":mesh.root_triangles(),"resident_clusters":mesh.clusters.len(),"sphere":mesh.sphere});
        meshes.push(scene.add_mesh(&mesh));
        if a.mode != "source" {
            let start = Instant::now();
            // Whole source tree, including wood. SheetCrossings is explicit:
            // this does not claim closed-solid or source optical equivalence.
            // The source alpha-card flag selects leaf lighting. The generic
            // baker requires this explicit semantic annotation; changing only
            // the bake copy keeps actual source rendering byte-identical.
            let mut bake_materials = scene.materials.clone();
            bake_materials[material_ids[3] as usize].flags |= THIN_FOLIAGE;
            let volume = bake(
                &parts,
                &bake_materials,
                &scene.textures,
                BakeConfig {
                    projection_measure: ProjectionMeasure::SheetCrossings,
                    cell_size: a.cell,
                    levels: 4,
                    samples_per_cell: 4,
                    alpha_cut: 0.42,
                    ..Default::default()
                },
            )?;
            bake_ms += start.elapsed().as_secs_f64() * 1000.;
            asset["voxel_levels"]=json!(volume.levels.iter().map(|l|json!({"cell_m":l.cell_size,"dimensions":l.dimensions,"occupied_cells":l.voxels.len()})).collect::<Vec<_>>());
            volumes.push(volume);
        }
        assets.push(asset);
    }
    let placements = placements(a.trees, a.seed);
    let mut owner_hash = Hash::new();
    let mut owners = Vec::new();
    let mut half = 20f32;
    let mut species = [0usize; 3];
    for (i, (kind, xf)) in placements.iter().enumerate() {
        let owner = scene.instances.len() as u32;
        scene.add_classified_instance(meshes[*kind], *xf, GeometryCategory::Trees);
        scene.mark_foliage();
        owners.push((owner, *kind as u32));
        species[*kind] += 1;
        let inst = &scene.instances[owner as usize];
        owner_hash.bytes(&(i as u64).to_le_bytes());
        owner_hash.u32(*kind as u32);
        for row in inst.rows {
            for x in row {
                owner_hash.f32(x);
            }
        }
        owner_hash.u32(inst.flags);
        half = half.max(xf.translation.x.abs().max(xf.translation.z.abs()) + inst.sphere[3] + 20.);
    }
    let ground = scene.add_material(Material {
        tint: [0.23, 0.21, 0.17],
        layer: -1,
        roughness: 1.,
        flags: 0,
        scale: 1.,
        metal: 0.,
    });
    let vertices = [
        [-half, 0., half],
        [half, 0., half],
        [-half, 0., -half],
        [half, 0., -half],
    ]
    .map(|pos| Vertex {
        pos,
        normal: Vec3::Y.to_array(),
        uv: [0.; 2],
    });
    let floor = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &[0, 1, 2, 1, 3, 2],
        material: ground,
    }]));
    scene.add_classified_instance(floor, Affine3A::IDENTITY, GeometryCategory::Terrain);
    let dim = (half / 4.).ceil() as u32 * 2 + 1;
    scene.heightfield = Heightfield {
        origin: [-half, -half],
        cell: 2. * half / (dim - 1) as f32,
        w: dim,
        h: dim,
        heights: vec![0.; (dim * dim) as usize],
    };
    scene.sky = Some((16, 8, vec![[1., 1., 1., 1.]; 128]));
    let accounting = Accounting::new(&scene)?;
    let inventory = json!({"trees":a.trees,"species_counts":species,"pitch_m":6.,"fullness":1.25,
        "source_geometry_hash":source_hash.finish(),"source_owner_hash":owner_hash.finish(),"source_owner_ids":"0..tree_count; stable tree ID equals owner ID",
        "assets":assets,"input_files":files,"bake_cpu_ms":bake_ms,"scene_cpu_ms":begin.elapsed().as_secs_f64()*1000.,
        "source_tree_triangles_finest":accounting.inventory().categories[1].source_triangles_full_detail,
        "shared_tree_triangles_finest":accounting.inventory().categories[1].shared_triangles_full_detail,
        "scope":"Shared current Sideways source trees; whole-tree sheet-crossing volumes, not crown-only replacement. Procedural flat fixture, not an actual stage or optical acceptance."});
    Ok(Fixture {
        scene,
        volumes,
        owners,
        accounting,
        half,
        inventory,
    })
}

fn views(a: &Args, half: f32) -> Vec<ViewRect> {
    let edge = (half - 20.).max(10.);
    let t = Vec3::new(0., 7., 0.);
    let poses = [
        (Vec3::new(0., 3., edge * 0.7), Vec3::new(0., 5., 0.)),
        (Vec3::new(edge + 140., 18., edge * 0.35), t),
        (Vec3::new(-edge * 1.1, edge * 0.9 + 40., edge * 1.1), t),
        (Vec3::new(-edge - 160., 12., -edge * 0.35), t),
    ];
    (0..a.views)
        .map(|i| {
            let (x, y, w, h) = if a.views == 1 {
                (0, 0, a.size[0], a.size[1])
            } else {
                let left = a.size[0] / 2;
                let top = a.size[1] / 2;
                (
                    if i % 2 == 0 { 0 } else { left + 1 },
                    if i / 2 == 0 { 0 } else { top + 1 },
                    if i % 2 == 0 {
                        left - 1
                    } else {
                        a.size[0] - left - 1
                    },
                    if i / 2 == 0 {
                        top - 1
                    } else {
                        a.size[1] - top - 1
                    },
                )
            };
            let (pos, target) = if a.views == 1 { poses[1] } else { poses[i] };
            ViewRect {
                cam: Camera {
                    pos,
                    target,
                    fov_y: 60f32.to_radians(),
                    near: 0.1,
                },
                x,
                y,
                w,
                h,
            }
        })
        .collect()
}
fn lighting(low: bool) -> Lighting {
    let elevation = if low { 7f32 } else { 55f32 }.to_radians();
    let az = 125f32.to_radians();
    Lighting {
        sun_dir: Vec3::new(
            az.sin() * elevation.cos(),
            elevation.sin(),
            -az.cos() * elevation.cos(),
        ),
        sun_lux: if low { 12000. } else { 80000. },
        sky_lux: if low { 3500. } else { 18000. },
        sun_color: if low {
            Vec3::new(1., 0.74, 0.49)
        } else {
            Vec3::ONE
        },
        ev100: if low { 12. } else { 14. },
        taa: 0.,
        sharpen: 0.,
        gi: 0.,
        contact_occlusion: 0.,
        reflection_occlusion: 0.,
        near_shadows: 1.,
        fog_density: 0.,
        fog_start: 10000.,
        fog_end: 20000.,
        ..Default::default()
    }
}
fn sky(light: &Lighting) -> Result<render::atmosphere::BakedSky, String> {
    let rgb = Vec3::new(0.85, 1., 1.2) * light.sky_lux;
    let radiance = rgb / std::f32::consts::PI;
    let mut sh = [[0.; 4]; 9];
    sh[0] = (rgb / 0.886227).extend(0.).to_array();
    let solar = render::atmosphere::SolarState::from_degrees(
        f64::from(light.sun_dir.y).asin().to_degrees(),
        f64::from(light.sun_dir.x)
            .atan2(f64::from(-light.sun_dir.z))
            .to_degrees(),
    )?;
    let sky = render::atmosphere::BakedSky {
        width: 16,
        height: 8,
        pixels: vec![radiance.extend(1.).to_array(); 128],
        sun_dir: light.sun_dir,
        sun_rgb_lux: light.sun_color * light.sun_lux,
        sky_horizontal_rgb_lux: rgb,
        sh,
        horizon: radiance,
        fog_phase_g: 0.,
        fog_scattering_albedo: Vec3::ONE,
        fog_height_m: 1000.,
        night_ambient_rgb_lux: Vec3::ZERO,
        night: Default::default(),
        night_stars: [0.; 4],
        solar,
        quality: render::atmosphere::BakeQuality {
            sky_width: 16,
            sky_height: 8,
            ..Default::default()
        },
    };
    sky.validate()?;
    Ok(sky)
}
fn camera_json(views: &[ViewRect]) -> Value {
    json!(
        views
            .iter()
            .map(
                |v| json!({"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),
    "fov_y":v.cam.fov_y,"near":v.cam.near,"viewport":[v.x,v.y,v.w,v.h]})
            )
            .collect::<Vec<_>>()
    )
}

fn source_counts(
    gpu: &Gpu,
    r: &Renderer,
    fixture: &Fixture,
    frame: &FrameStats,
    view_count: usize,
) -> Result<Value, String> {
    let mut rows = Vec::new();
    let mut sequence = None;
    for view in 0..view_count {
        let mut c = r.debug_visible_clusters(gpu, view, sequence)?;
        if let Some(s) = sequence {
            if c.render_sequence != s {
                return Err("view diagnostics belong to different submitted frames".into());
            }
        }
        sequence = Some(c.render_sequence);
        let counted = fixture.accounting.view(&c, frame.cluster_buckets)?;
        c.pairs.sort_unstable();
        let mut hash = Hash::new();
        for pair in &c.pairs {
            hash.u32(pair[0]);
            hash.u32(pair[1]);
        }
        let tree = &counted.categories[1].raster;
        let h = r.debug_tree_occlusion(gpu, view, sequence)?;
        rows.push(json!({"view":view,"render_sequence":c.render_sequence,"frame_id":c.frame_id,"input_size":c.input_size,"output_size":c.output_size,
            "source_owners":tree.instances,"source_clusters":tree.clusters,"source_triangles":tree.triangles,"source_padded_vertices":tree.submitted_vertices,
            "selected_pair_hash":hash.finish(),"instance_groups":h.instance_groups.map(|g|json!({"enabled":g.enabled,"owner_tests":g.owner_tests,
                "compacted_owners":g.compacted_owners,"resident_clusters_visited":g.resident_clusters_visited,"parent_groups_tested":g.parent_groups_tested,
                "leaf_groups_tested":g.leaf_groups_tested,"overflow_fallbacks":g.overflow_fallbacks})),
            "cluster_traversal":h.cluster_traversal.map(|g|json!({"enabled":g.enabled,"nodes_tested":g.nodes_tested,"clusters_tested":g.clusters_tested,
                "clusters_skipped":g.clusters_skipped,"fallback_owners":g.fallback_owners}))}));
    }
    Ok(json!(rows))
}
fn distribution(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p = |q: f64| {
        let x = q * (sorted.len() - 1) as f64;
        let i = x.floor() as usize;
        sorted[i] + (sorted[x.ceil() as usize] - sorted[i]) * (x - i as f64)
    };
    json!({"samples":values.len(),"median_ms":p(0.5),"p95_ms":p(0.95),"min_ms":sorted[0],"max_ms":sorted[sorted.len()-1]})
}
fn render_timed(
    gpu: &Gpu,
    r: &Renderer,
    cameras: &[ViewRect],
    light: &Lighting,
    a: &Args,
    id: &mut u64,
) -> Result<(wgpu::Texture, FrameStats, Value), String> {
    let mut rows = Vec::new();
    let mut last = None;
    let mut stats = FrameStats::default();
    for i in 0..a.warmup + a.frames {
        *id += 1;
        r.set_gpu_profile_frame_id(*id);
        let start = Instant::now();
        let (texture, frame) =
            r.try_render_texture_profiled(gpu, cameras, light, a.size[0], a.size[1])?;
        let wall = start.elapsed().as_secs_f64() * 1000.;
        gpu.wait();
        let mut drain = r.drain_gpu_profiles(gpu);
        if drain.pending > 0 {
            gpu.wait();
            let more = r.drain_gpu_profiles(gpu);
            drain.samples.extend(more.samples);
            drain.pending = more.pending;
            drain.errors = more.errors;
            drain.dropped = more.dropped;
        }
        if drain.errors != 0
            || drain.dropped != 0
            || drain.pending != 0
            || drain.samples.len() != 1
            || drain.samples[0].frame_id != *id
        {
            return Err(format!(
                "profile mismatch frame{id}: samples{} pending{} dropped{} errors{}",
                drain.samples.len(),
                drain.pending,
                drain.dropped,
                drain.errors
            ));
        }
        let sample = &drain.samples[0];
        if sample.output_size != a.size
            || sample.views != a.views as u32
            || !sample.covered_interval_ms.is_finite()
            || sample.covered_interval_ms <= 0.
        {
            return Err("invalid measured GPU frame".into());
        }
        if i >= a.warmup {
            rows.push(json!({"frame_id":*id,"render_sequence":sample.render_sequence,"covered_interval_ms":sample.covered_interval_ms,
            "covered_union_ms":sample.covered_union_ms,"instrumented_pass_sum_ms":sample.instrumented_gpu_ms,"cpu_wall_including_wait_ms":wall,
            "gpu_pass_ms":sample.gpu_ms}));
        }
        last = Some(texture);
        stats = frame;
    }
    let intervals = rows
        .iter()
        .map(|r| r["covered_interval_ms"].as_f64().unwrap())
        .collect::<Vec<_>>();
    let mut passes: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for row in &rows {
        let mut current: BTreeMap<String, f64> = BTreeMap::new();
        for p in row["gpu_pass_ms"].as_array().unwrap() {
            *current.entry(p[0].as_str().unwrap().into()).or_default() += p[1].as_f64().unwrap();
        }
        for (k, v) in current {
            passes.entry(k).or_default().push(v);
        }
    }
    let summary = json!({"warmup_frames":a.warmup,"steady_frames":a.frames,"covered_interval":distribution(&intervals),
        "passes":passes.into_iter().map(|(k,v)|(k,distribution(&v))).collect::<BTreeMap<_,_>>(),"raw_frames":rows,
        "scope":"Native offscreen explicitly synchronized fixture. GPU first-to-last timestamp interval, separate pass intervals; CPU wall includes waits. Image/counter captures occur afterward. No presentation cadence/FPS or sum-of-pass-medians claim."});
    Ok((last.ok_or("no rendered frames")?, stats, summary))
}

fn shadow_counts(gpu: &Gpu, r: &Renderer) -> Result<Value, String> {
    Ok(json!(r.debug_sun_culling(gpu,None)?.into_iter().map(|mut d|{
        d.pairs.sort_unstable();let mut hash=Hash::new();for p in &d.pairs{hash.u32(p[0]);hash.u32(p[1]);}
        json!({"name":d.name,"view":d.view_index,"clusters":d.pairs.len(),"triangles":d.selected_triangles,
            "padded_vertices":d.submitted_vertices,"selected_pair_hash":hash.finish()})
    }).collect::<Vec<_>>()))
}
fn tree_masks(gpu: &Gpu, r: &Renderer, trees: usize, n: usize) -> Result<Vec<Vec<bool>>, String> {
    (0..n)
        .map(|view| {
            let p = r.debug_geometry_owner_pixels(gpu, view, None)?;
            Ok(p.owners
                .into_iter()
                .map(|id| (id as usize) < trees)
                .collect())
        })
        .collect()
}
fn world_counts(gpu: &Gpu, r: &Renderer) -> Result<Value, String> {
    Ok(json!(r.foliage_world_diagnostics(gpu)?.into_iter().map(|d|json!({
        "view":d.view,"groups":d.groups,"selected_groups":d.selected_groups,
        "source_owners_suppressed":d.source_owners_suppressed,"source_owners":d.source_owners,
        "shared_bytes":d.shared_bytes,"target_bytes":d.target_bytes,"traced_rays":d.traced_rays,
        "failed_rays":d.failed_rays,"max_cell_references":d.max_cell_references,"max_ray_hits":d.max_ray_hits,
        "complete_cell_fallbacks":d.complete_cell_fallbacks,"max_iterations":d.max_iterations,
        "grid_segments":d.grid_segments,"iterations":d.iterations,"interval_tests":d.interval_tests,
        "volume_samples":d.volume_samples,"occupied_cell_reads":d.occupied_cell_reads,"empty_jumps":d.empty_jumps
    })).collect::<Vec<_>>()))
}
fn validate_world(rows: &Value, views: usize, enabled: bool) -> Result<(), String> {
    let rows = rows
        .as_array()
        .ok_or("world diagnostics are not an array")?;
    if rows.len() != views {
        return Err(format!(
            "world diagnostic view count {} != {views}",
            rows.len()
        ));
    }
    for (view, row) in rows.iter().enumerate() {
        let value = |k: &str| {
            row[k]
                .as_u64()
                .ok_or_else(|| format!("missing unsigned world field {k}"))
        };
        if value("view")? != view as u64
            || value("selected_groups")? > value("groups")?
            || value("source_owners_suppressed")? > value("source_owners")?
        {
            return Err(format!(
                "inconsistent world selection counters in view {view}"
            ));
        }
        if enabled && value("failed_rays")? != 0 {
            return Err(format!(
                "world view {view} failed {} rays; raw diagnostics and images were saved",
                value("failed_rays")?
            ));
        }
        if !enabled && value("source_owners_suppressed")? != 0 {
            return Err(format!(
                "disabled world mode suppressed source owners in view {view}"
            ));
        }
    }
    Ok(())
}
fn save_image(
    out: &Path,
    name: &str,
    rgba: &[u8],
    size: [u32; 2],
    png: bool,
) -> Result<Value, String> {
    if rgba.len() != size[0] as usize * size[1] as usize * 4 {
        return Err("image size mismatch".into());
    }
    let path = out.join(format!("{name}.jpg"));
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    image::codecs::jpeg::JpegEncoder::new_with_quality(
        std::fs::File::create(&path).map_err(|e| e.to_string())?,
        90,
    )
    .encode(&rgb, size[0], size[1], image::ExtendedColorType::Rgb8)
    .map_err(|e| e.to_string())?;
    let native = out.join(format!("{name}.png"));
    if png {
        image::save_buffer(&native, rgba, size[0], size[1], image::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
    }
    let mut hash = Hash::new();
    hash.bytes(rgba);
    Ok(
        json!({"preview":path,"png":if png{Some(native)}else{None},"output_size":size,"decoded_rgba8_hash":hash.finish()}),
    )
}
fn half(value: u16) -> f32 {
    let sign = if value & 0x8000 == 0 { 1. } else { -1. };
    let exponent = (value >> 10) & 31;
    let mantissa = value & 1023;
    if exponent == 0 {
        sign * f32::from(mantissa) * 2f32.powi(-24)
    } else if exponent == 31 {
        if mantissa == 0 {
            sign * f32::INFINITY
        } else {
            f32::NAN
        }
    } else {
        sign * (1. + f32::from(mantissa) / 1024.) * 2f32.powi(i32::from(exponent) - 15)
    }
}
fn save_layers(out: &Path, name: &str, gpu: &Gpu, r: &Renderer) -> Result<Value, String> {
    let mut rows = Vec::new();
    for (view, (w, h, bytes)) in r.foliage_world_layers_rgba16(gpu)?.into_iter().enumerate() {
        // Persist the unmodified bytes even if validation below finds a NaN.
        let file = out.join(format!("{name}-view{view}.rgba16f"));
        std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
        let mut hash = Hash::new();
        hash.bytes(&bytes);
        let mut nonfinite = 0usize;
        let mut bad_alpha = 0usize;
        let mut alpha = 0f64;
        let mut luma = 0f64;
        let mut occupied = 0usize;
        for pixel in bytes.chunks_exact(8) {
            let p: [f32; 4] =
                std::array::from_fn(|i| half(u16::from_le_bytes([pixel[i * 2], pixel[i * 2 + 1]])));
            nonfinite += p.iter().filter(|v| !v.is_finite()).count();
            bad_alpha += usize::from(p[3].is_finite() && !(0.0..=1.0).contains(&p[3]));
            if p.iter().all(|v| v.is_finite()) {
                alpha += f64::from(p[3]);
                luma += f64::from(p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722);
                occupied += usize::from(p[3] > 0.);
            }
        }
        rows.push(json!({"view":view,"size":[w,h],"file":file,"bytes":bytes.len(),"raw_hash":hash.finish(),
            "nonfinite_components":nonfinite,"invalid_alpha_pixels":bad_alpha,"covered_pixels":occupied,
            "summed_fractional_coverage":alpha,"summed_premultiplied_hdr_luminance":luma,
            "byte_count_valid":bytes.len()==w as usize*h as usize*8}));
    }
    Ok(json!(rows))
}
fn image_difference(
    a: &[u8],
    b: &[u8],
    size: [u32; 2],
    cameras: &[ViewRect],
    masks: Option<&[Vec<bool>]>,
) -> Value {
    let mut result = Vec::new();
    for (view, c) in cameras.iter().enumerate() {
        let measure = |only_tree: bool| {
            let mut histogram = [0u64; 256];
            let mut count = 0u64;
            let mut changed = 0u64;
            let mut sum = 0u64;
            let mut max = 0u8;
            for y in 0..c.h {
                for x in 0..c.w {
                    if only_tree && !masks.is_some_and(|m| m[view][(y * c.w + x) as usize]) {
                        continue;
                    }
                    let offset = (((c.y + y) * size[0] + c.x + x) * 4) as usize;
                    let mut differs = false;
                    for k in 0..3 {
                        let d = a[offset + k].abs_diff(b[offset + k]);
                        histogram[d as usize] += 1;
                        sum += u64::from(d);
                        max = max.max(d);
                        differs |= d != 0;
                    }
                    count += 1;
                    changed += u64::from(differs);
                }
            }
            let target = (count * 3 * 99).div_ceil(100);
            let mut accumulated = 0;
            let mut p99 = 0;
            for (i, n) in histogram.iter().enumerate() {
                accumulated += n;
                if accumulated >= target {
                    p99 = i;
                    break;
                }
            }
            json!({"pixels":count,"changed_pixels":changed,"mean_abs_rgb_bytes":if count>0{sum as f64/(count*3)as f64}else{0.},
                "p99_rgb_bytes":p99,"max_rgb_bytes":max,"exact":changed==0})
        };
        result.push(json!({"view":view,"all_pixels":measure(false),"source_tree_pixels":if masks.is_some(){measure(true)}else{Value::Null}}));
    }
    json!({"scope":"Decoded native RGB differences. Source-tree ROI is opaque source ownership only, not final group coverage. No tolerance or optical acceptance is inferred.","views":result})
}

fn run(a: &Args) -> Result<(), String> {
    let all = Instant::now();
    // Dedicated, explicitly invoked process only. This enables readback
    // snapshots, not category exclusion or a gameplay setting.
    unsafe {
        std::env::set_var("KERN_DIAGNOSTIC_HIDE", "none");
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut hash = Hash::new();
    hash.bytes(&std::fs::read(&exe).map_err(|e| e.to_string())?);
    let environment: BTreeMap<_, _> = [
        "KERN_ADAPTER",
        "KERN_SHADER_DIR",
        "KERN_CLUSTER_BUCKETS",
        "KERN_INSTANCE_GROUPS",
        "KERN_CLUSTER_TRAVERSAL",
        "KERN_TREE_OCCLUSION",
        "KERN_TREE_OCCLUSION_REJECT",
        "KERN_SUN_GROUPS",
        "KERN_WORLD_TERRAIN_SHADOW",
        "KERN_PAGED_FOLIAGE_SUN",
        "KERN_PIPELINE_CACHE",
        "KERN_DIAGNOSTIC_HIDE",
    ]
    .into_iter()
    .map(|name| (name, std::env::var(name).ok()))
    .collect();
    let mut report = json!({"schema":1,"mechanical_pass":false,"optical_accepted":false,"status":"building scene",
        "executable":exe,"executable_content_hash":hash.finish(),"hash_scope":"FNV-1a content identity only; external runner should record SHA-256",
        "args":{"trees":a.trees,"size":a.size,"views":a.views,"frames":a.frames,"warmup":a.warmup,"mode":a.mode,
            "lights":a.lights,"seed":a.seed,"voxel_cell_metres":a.cell,"source_lod_pixels":a.lod_pixels,"group_spatial_error_pixels":4.,
            "wind_times":a.wind_times,"png":a.png},"environment":environment,"records":[],
        "limitations":["Spatial cell diameter is not an optical quality certificate.",
            "Whole-tree sheet-crossing volumes approximate alpha-tested surfaces and solid wood.",
            "This flat deterministic forest is not the live stage or a presentation/FPS benchmark.",
            "Four view work counts intentionally repeat owners visible to multiple cameras.",
            "Provider invalidation uses a byte-identical source update; partial-page or corrupted-buffer publication is not exercised."]});
    write_json(&a.out.join("checkpoint.json"), &report)?;
    let mut f = fixture(a)?;
    report["inventory"] = f.inventory.clone();
    if a.mode != "source" {
        let start = Instant::now();
        let bindings = f
            .owners
            .iter()
            .map(|&(owner, asset)| VolumeBinding {
                owner,
                asset,
                crown_material: ALL_MATERIALS,
            })
            .collect::<Vec<_>>();
        let world = WorldField::build(
            &f.volumes,
            &bindings,
            &f.scene.instances,
            WorldConfig {
                assets: render::foliage_volume_data::AssetPackLimits {
                    levels_per_asset: 4,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        report["provider"] = json!({"cpu_ms":start.elapsed().as_secs_f64()*1000.,"grid_cell_metres":world.grid.cell_metres,
            "grid_origin":world.grid.origin,"grid_dimensions":world.grid.dimensions,"grid_cells":world.grid.spans.len(),
            "grid_references":world.grid.references.len(),"max_grid_references":world.grid.spans.iter().map(|s|s[1]).max().unwrap_or(0),
            "groups":world.groups.len(),"owners":world.forest.instances.len(),"packed_bytes":world.byte_len(),
            "packed_levels":world.forest.levels.len(),"resident_sparse_cells":world.forest.cells.len(),"generation":1,
            "max_wind_shear":world.max_wind_shear});
        f.scene.foliage_world = Some(world);
        f.scene.foliage_world_experimental = true;
        f.scene.foliage_world_error_pixels = 4.;
    }
    let cameras = views(a, f.half);
    report["cameras"] = camera_json(&cameras);
    report["status"] = json!("creating renderer");
    write_json(&a.out.join("checkpoint.json"), &report)?;
    let gpu = Gpu::new()?;
    report["adapter"] = json!({"name":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),
        "driver":gpu.info.driver,"driver_info":gpu.info.driver_info});
    let mut initial = lighting(false);
    initial.lod_pixels = a.lod_pixels;
    let create = Instant::now();
    let mut renderer = Renderer::new(&gpu, &f.scene, &initial);
    report["renderer_cpu_ms"] = json!(create.elapsed().as_secs_f64() * 1000.);
    renderer.set_async_gpu_profiling(&gpu, true)?;
    let focus = cameras.iter().map(|v| v.cam.target).collect::<Vec<_>>();
    renderer.set_lighting_focus(&focus)?;
    renderer.set_sun_shadow_focus(&focus)?;
    let phases = match a.lights.as_str() {
        "clear" => vec![false],
        "low" => vec![true],
        _ => vec![false, true],
    };
    let modes = match a.mode.as_str() {
        "source" => vec![false],
        "group" => vec![true],
        _ => vec![false, true],
    };
    let mut id = 0u64;
    let mut records = Vec::new();
    let mut exercised = false;
    let mut restore = None;
    for low in phases {
        for (wind_index, &time) in a.wind_times.iter().enumerate() {
            let label = if low { "low" } else { "clear" };
            let mut light = lighting(low);
            light.lod_pixels = a.lod_pixels;
            let baked = sky(&light)?;
            renderer.update_sky(&gpu, &baked, &mut light)?;
            let wind = WindState {
                time_seconds: time,
                direction: Vec2::new(0.8, 0.6),
                strength: 0.6,
                ..Default::default()
            };
            renderer.set_wind(&gpu, &wind)?;
            let input = json!({"light":label,"sun_direction":light.sun_dir.to_array(),"sun_lux":light.sun_lux,
            "sun_rgb":light.sun_color.to_array(),"sky_lux":light.sky_lux,"sky_sh":baked.sh,"ev100":light.ev100,
            "wind":{"time_seconds":time,"strength":wind.strength,"direction":wind.direction.to_array(),
                "tree_shear":wind.tree_shear,"grass_shear":wind.grass_shear},"cameras":camera_json(&cameras),
            "source_geometry_hash":f.inventory["source_geometry_hash"],"source_owner_hash":f.inventory["source_owner_hash"],
            "source_lod_pixels":a.lod_pixels,"group_spatial_error_pixels":4.,"native":true});
            let mut reference: Option<(Vec<u8>, Vec<Vec<bool>>, Value, Value)> = None;
            for &enabled in &modes {
                renderer.set_foliage_world_enabled(enabled);
                let mode = if enabled { "group" } else { "source" };
                let name = format!("{label}-wind{wind_index}-{mode}");
                report["status"] = json!(format!("rendering {name}"));
                write_json(&a.out.join("checkpoint.json"), &report)?;
                let (texture, stats, timing) =
                    render_timed(&gpu, &renderer, &cameras, &light, a, &mut id)?;
                // Timings are saved before any further blocking diagnostic or validation.
                write_json(&a.out.join(format!("{name}-timings.json")), &timing)?;
                let capture = Instant::now();
                let rgba = gpu.read_rgba8(&texture);
                let images = save_image(&a.out, &name, &rgba, a.size, a.png)?;
                let counts = source_counts(&gpu, &renderer, &f, &stats, a.views)?;
                let shadows = shadow_counts(&gpu, &renderer)?;
                let world = if f.scene.foliage_world.is_some() {
                    world_counts(&gpu, &renderer)?
                } else {
                    json!([])
                };
                write_json(&a.out.join(format!("{name}-world.json")), &world)?;
                let layers = if enabled {
                    save_layers(&a.out, &name, &gpu, &renderer)?
                } else {
                    json!([])
                };
                let mut record = json!({"name":name,"mode":mode,"inputs":input,"timing":timing,"images":images,
                "source_selection":counts,"source_shadow_selection":shadows,"world":world,"raw_layers":layers,
                "capture_cpu_wall_ms":capture.elapsed().as_secs_f64()*1000.});
                if let Some((before, masks, before_counts, before_shadows)) = &reference {
                    record["source_comparison"] =
                        image_difference(before, &rgba, a.size, &cameras, Some(masks));
                    record["source_shadow_selection_identical"] = json!(*before_shadows == shadows);
                    record["source_work_delta"]=json!((0..a.views).map(|view|{
                    let old=&before_counts[view];let now=&counts[view];
                    json!({"view":view,"owners_before":old["source_owners"],"owners_after":now["source_owners"],
                        "triangles_before":old["source_triangles"],"triangles_after":now["source_triangles"],
                        "padded_vertices_before":old["source_padded_vertices"],"padded_vertices_after":now["source_padded_vertices"]})
                }).collect::<Vec<_>>());
                }
                records.push(record);
                report["records"] = json!(records);
                write_json(&a.out.join("checkpoint.json"), &report)?;
                if f.scene.foliage_world.is_some() {
                    validate_world(&world, a.views, enabled)?;
                }
                if enabled {
                    if layers.as_array().map(Vec::len) != Some(a.views) {
                        return Err("world raw layers omit active views".into());
                    }
                    for row in layers.as_array().unwrap() {
                        if row["nonfinite_components"] != 0
                            || row["invalid_alpha_pixels"] != 0
                            || row["byte_count_valid"] != true
                        {
                            return Err(format!(
                                "invalid raw world layer in {name}; checkpoint and bytes saved"
                            ));
                        }
                    }
                    exercised |= world
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v["source_owners_suppressed"].as_u64().unwrap_or(0) > 0);
                    if let Some((_, _, _, before)) = &reference {
                        if *before != shadows {
                            return Err(format!("source shadow selection changed in {name}"));
                        }
                    }
                } else {
                    let masks = tree_masks(&gpu, &renderer, a.trees, a.views)?;
                    reference = Some((rgba.clone(), masks, counts.clone(), shadows));
                    restore = Some((rgba, counts, light.clone(), wind));
                }
            }
        }
    }
    // Re-enabling source must not retain omitted geometry or composite old layers.
    if a.mode == "both" {
        if let Some((expected, counts, light, wind)) = restore {
            renderer.set_foliage_world_enabled(false);
            renderer.set_wind(&gpu, &wind)?;
            id += 1;
            renderer.set_gpu_profile_frame_id(id);
            let (texture, stats) = renderer
                .try_render_texture_profiled(&gpu, &cameras, &light, a.size[0], a.size[1])?;
            gpu.wait();
            let _ = renderer.drain_gpu_profiles(&gpu);
            let actual = gpu.read_rgba8(&texture);
            let restored = source_counts(&gpu, &renderer, &f, &stats, a.views)?;
            let selection_equal = (0..a.views)
                .all(|i| counts[i]["selected_pair_hash"] == restored[i]["selected_pair_hash"]);
            let image_equal = actual == expected;
            let image = save_image(&a.out, "source-restored", &actual, a.size, a.png)?;
            report["source_restored"] = json!({"images":image,"decoded_rgba_exact":image_equal,"selection_exact":selection_equal,
            "difference":image_difference(&expected,&actual,a.size,&cameras,None)});
            write_json(&a.out.join("checkpoint.json"), &report)?;
            if !selection_equal || !image_equal {
                return Err(
                    "source restoration differs; unmodified difference evidence saved".into(),
                );
            }
            // Existing source-update API invalidates this immutable provider. The
            // bytes intentionally stay identical, making the fallback oracle exact.
            renderer.update_instances(&gpu, 0, &f.scene.instances[..1])?;
            renderer.set_foliage_world_enabled(true);
            id += 1;
            renderer.set_gpu_profile_frame_id(id);
            let (texture, frame) = renderer
                .try_render_texture_profiled(&gpu, &cameras, &light, a.size[0], a.size[1])?;
            gpu.wait();
            let _ = renderer.drain_gpu_profiles(&gpu);
            let fallback = gpu.read_rgba8(&texture);
            let fallback_counts = source_counts(&gpu, &renderer, &f, &frame, a.views)?;
            let fallback_world = world_counts(&gpu, &renderer)?;
            let pixels_equal = fallback == expected;
            let selection_equal = (0..a.views).all(|i| {
                counts[i]["selected_pair_hash"] == fallback_counts[i]["selected_pair_hash"]
            });
            report["invalid_provider_fallback"] = json!({"trigger":"update_instances resubmits one original source owner byte-for-byte; immutable provider becomes invalid",
            "world_requested":true,"world":fallback_world,"decoded_rgba_exact":pixels_equal,"selection_exact":selection_equal,
            "images":save_image(&a.out,"invalid-provider-source",&fallback,a.size,a.png)?,
            "difference":image_difference(&expected,&fallback,a.size,&cameras,None)});
            write_json(&a.out.join("checkpoint.json"), &report)?;
            validate_world(&fallback_world, a.views, false)?;
            if !pixels_equal || !selection_equal {
                return Err(
                    "invalid world provider did not preserve exact source; evidence saved".into(),
                );
            }
        }
    }
    report["status"] = json!("complete");
    report["mechanical_pass"] = json!(true);
    report["replacement_exercised"] = json!(exercised);
    report["optical_accepted"] = json!(false);
    report["elapsed_seconds"] = json!(all.elapsed().as_secs_f64());
    report["acceptance_scope"] = json!(
        "Mechanical pass means current diagnostics, finite raw layers, unchanged source shadows and exact source restoration. Replacement_exercised is separate; images require review. No full forest, optical, motion or 120 FPS acceptance."
    );
    write_json(&a.out.join("report.json"), &report)?;
    println!("{}",serde_json::to_string(&json!({"mechanical_pass":true,"replacement_exercised":exercised,"report":a.out.join("report.json")})).unwrap());
    Ok(())
}
fn main() {
    let result = (|| {
        let a = parse_args(&std::env::args().skip(1).collect::<Vec<_>>())?;
        if a.out.exists() {
            return Err(
                "--out must be a fresh directory; existing evidence is never overwritten".into(),
            );
        }
        std::fs::create_dir_all(&a.out).map_err(|e| e.to_string())?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&a)))
            .unwrap_or_else(|panic| {
                Err(panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).into()))
                    .unwrap_or_else(|| "GPU fixture panicked".into()))
            });
        if let Err(error) = &result {
            let _ = write_json(
                &a.out.join("failure.json"),
                &json!({"mechanical_pass":false,"error":error,"checkpoint":a.out.join("checkpoint.json")}),
            );
        }
        result
    })();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_bound_cost_and_accept_fast_smoke() {
        let a = parse_args(
            &[
                "--out", "unused", "--trees", "64", "--size", "128x128", "--views", "1",
                "--frames", "2", "--warmup", "1",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert_eq!((a.trees, a.size, a.frames), (64, [128, 128], 2));
        for args in [
            vec!["--out", "unused", "--trees", "0"],
            vec!["--out", "unused", "--views", "3"],
            vec!["--out", "unused", "--frames", "129"],
            vec!["--out", "unused", "--wind-times", "NaN"],
        ] {
            assert!(parse_args(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }
    #[test]
    fn owner_placement_is_stable_shared_and_contains_three_species() {
        let a = placements(16384, 1);
        let b = placements(16384, 1);
        assert_eq!(a, b);
        let mut kinds = [0; 3];
        for (kind, xf) in a {
            kinds[kind] += 1;
            assert!(xf.is_finite());
            assert_eq!(xf.translation.y, 0.);
        }
        assert!(kinds.into_iter().all(|n| n > 100));
        let a = placements(64, 1);
        let changed = placements(64, 2);
        assert_ne!(a, changed);
    }
    #[test]
    fn split_views_are_distinct_and_in_output_without_overlap() {
        let a = Args {
            size: [1281, 721],
            ..Default::default()
        };
        let c = views(&a, 400.);
        for (i, v) in c.iter().enumerate() {
            assert!(v.x + v.w <= a.size[0] && v.y + v.h <= a.size[1]);
            for other in &c[i + 1..] {
                assert!(
                    v.x + v.w <= other.x
                        || other.x + other.w <= v.x
                        || v.y + v.h <= other.y
                        || other.y + other.h <= v.y
                );
                assert_ne!(v.cam.pos, other.cam.pos);
            }
        }
    }
    #[test]
    fn exact_pixel_gate_and_half_validation_do_not_hide_single_pixel_errors() {
        assert_eq!(half(0x3c00), 1.);
        assert_eq!(half(0x0001), 2f32.powi(-24));
        assert!(half(0x7c00).is_infinite());
        assert!(half(0x7e00).is_nan());
        let a = Args {
            size: [128, 128],
            views: 1,
            ..Default::default()
        };
        let c = views(&a, 20.);
        let source = vec![0; 128 * 128 * 4];
        let mut candidate = source.clone();
        candidate[17] = 62;
        let result = image_difference(&source, &candidate, a.size, &c, None);
        assert_eq!(result["views"][0]["all_pixels"]["changed_pixels"], 1);
        assert_eq!(result["views"][0]["all_pixels"]["max_rgb_bytes"], 62);
    }
}
