//! Sideways' look as a Kern scene: terrain chunks, the road ribbon, the
//! forest (Godot's own tree meshes, exported by tools/export_meshes.gd),
//! barriers and cars, with the Poly Haven textures.

use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex, read_kmesh};
use render::scene::{ALPHA_TEST, Material, ROAD, Scene, TERRAIN, TRIPLANAR, TWO_SIDED};
use sideways::builder::{self, Chunk, N};
use sideways::stage::{GRID, Stage};
use std::path::{Path, PathBuf};

pub struct Built {
    pub scene: Scene,
    /// What each mesh id is, for picking.
    pub mesh_names: Vec<String>,
    pub log: Vec<String>,
    pub cars: Vec<DynamicCar>,
    live_ground_cover: Option<crate::live_ground_cover::LiveGroundCover>,
}

pub struct DynamicCar {
    pub first: usize,
    meshes: crate::car_model::CarMeshes,
    fallback: bool,
}

impl Built {
    pub fn update_ground_cover(&mut self, cameras: &[Vec3]) -> Option<std::ops::Range<usize>> {
        self.live_ground_cover.as_mut()?.update(&mut self.scene, cameras)
    }

    pub fn ground_cover_stats(&self) -> Option<crate::live_ground_cover::Stats> {
        self.live_ground_cover.as_ref().map(|stream| stream.stats())
    }

    pub fn update_cars(&mut self, poses: &[CarPose]) {
        for (car, pose) in self.cars.iter().zip(poses) {
            let xf = Affine3A::from_rotation_translation(pose.rot, pose.pos);
            if car.fallback {
                self.scene.update_instance(car.first, car.meshes.body, xf);
            } else {
                crate::car_model::update(&mut self.scene, car.first, &car.meshes, xf, pose.ride, &pose.wheels);
            }
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn v(p: Vec3, n: Vec3, uv: [f32; 2]) -> Vertex {
    Vertex { pos: p.to_array(), normal: n.to_array(), uv }
}

fn chunk_mesh(c: &Chunk, material: u32) -> ClusterMesh {
    let mut verts = Vec::with_capacity(N * N);
    for j in 0..N {
        for i in 0..N {
            let h = |i: isize, j: isize| c.heights[(j.clamp(0, N as isize - 1) as usize) * N + i.clamp(0, N as isize - 1) as usize];
            let (ii, jj) = (i as isize, j as isize);
            let dx = h(ii + 1, jj) - h(ii - 1, jj);
            let dz = h(ii, jj + 1) - h(ii, jj - 1);
            // At chunk borders each axis has its own one-sided derivative.
            // A shared denominator changed the z slope at x borders and
            // halved it at z borders, producing visible lighting seams.
            let span_x = GRID as f32 * if i == 0 || i == N - 1 { 1.0 } else { 2.0 };
            let span_z = GRID as f32 * if j == 0 || j == N - 1 { 1.0 } else { 2.0 };
            let n = Vec3::new(-dx / span_x, 1.0, -dz / span_z).normalize();
            let p = c.vertex(i, j);
            verts.push(v(Vec3::new(p.x, p.y, p.z), n, [c.road_d[j * N + i], 0.0]));
        }
    }
    let mut idx = Vec::new();
    for j in 0..N - 1 {
        for i in 0..N - 1 {
            let a = (j * N + i) as u32;
            let n = N as u32;
            idx.extend_from_slice(&[a, a + n, a + 1, a + 1, a + n, a + n + 1]);
        }
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// All chunks' heights on one 2 m grid (cells outside the corridor are deep).
fn heightfield(chunks: &[Chunk]) -> render::scene::Heightfield {
    let per = N - 1;
    let lo = chunks.iter().fold((i32::MAX, i32::MAX), |a, c| (a.0.min(c.cell.0), a.1.min(c.cell.1)));
    let hi = chunks.iter().fold((i32::MIN, i32::MIN), |a, c| (a.0.max(c.cell.0), a.1.max(c.cell.1)));
    let w = ((hi.0 - lo.0 + 1) as usize * per + 1) as u32;
    let h = ((hi.1 - lo.1 + 1) as usize * per + 1) as u32;
    let mut heights = vec![-1e9f32; (w * h) as usize];
    for c in chunks {
        let (ox, oz) = ((c.cell.0 - lo.0) as usize * per, (c.cell.1 - lo.1) as usize * per);
        for j in 0..N {
            for i in 0..N {
                heights[(oz + j) * w as usize + ox + i] = c.heights[j * N + i];
            }
        }
    }
    render::scene::Heightfield { origin: [lo.0 as f32 * 64.0, lo.1 as f32 * 64.0], cell: GRID as f32, w, h, heights }
}

fn road_mesh(s: &Stage, material: u32) -> ClusterMesh {
    let r = builder::road(s);
    let cols = r.cols;
    let rows = r.verts.len() / cols;
    let mut verts = Vec::with_capacity(r.verts.len());
    for i in 0..rows {
        for c in 0..cols {
            let k = i * cols + c;
            let at = |k: usize| Vec3::new(r.verts[k].x, r.verts[k].y, r.verts[k].z);
            let along = at(if i + 1 < rows { k + cols } else { k }) - at(if i > 0 { k - cols } else { k });
            let side = at(if c + 1 < cols { k + 1 } else { k }) - at(if c > 0 { k - 1 } else { k });
            let mut n = side.cross(along).normalize_or(Vec3::Y);
            if n.y < 0.0 {
                n = -n;
            }
            verts.push(v(at(k), n, [builder::ACROSS[c] as f32, i as f32 * 2.0]));
        }
    }
    let mut idx = Vec::new();
    for t in &r.tris {
        // Godot's winding faces down; Kern draws both sides of the ground
        // anyway, but keep it consistent with the terrain.
        idx.extend_from_slice(&[t[0], t[2], t[1]]);
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

fn box_mesh(half: Vec3, material: u32) -> ClusterMesh {
    let mut verts = Vec::new();
    let mut idx = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut n = Vec3::ZERO;
            n[axis] = sign;
            let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = verts.len() as u32;
            for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut p = n * half[axis];
                p[u] = a * half[u];
                p[w] = b * half[w];
                verts.push(v(p, n, [a * 0.5 + 0.5, b * 0.5 + 0.5]));
            }
            let quad = if sign > 0.0 { [0, 1, 2, 0, 2, 3] } else { [0, 2, 1, 0, 3, 2] };
            idx.extend(quad.iter().map(|q| base + q));
        }
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// A car: two boxes from its collision shape until the glTF bodies load.
fn car_mesh(boxes: &[[f64; 6]], material: u32) -> ClusterMesh {
    let mut verts = Vec::new();
    let mut idx = Vec::new();
    for b in boxes {
        let m = box_mesh(Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32) * 0.5, material);
        // box_mesh built clusters; rebuild from its vertices instead.
        let base = verts.len() as u32;
        for vx in &m.vertices {
            let mut vx = *vx;
            vx.pos = [vx.pos[0] + b[3] as f32, vx.pos[1] + b[4] as f32, vx.pos[2] + b[5] as f32];
            verts.push(vx);
        }
        for c in &m.clusters {
            for t in 0..c.tri_count {
                let p = m.tris[(c.first_tri + t) as usize];
                for k in 0..3 {
                    idx.push(base + m.cluster_verts[(c.first_vert + ((p >> (8 * k)) & 0xff)) as usize]);
                }
            }
        }
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

fn side_of(line: &[Vec3], k: usize) -> Vec3 {
    let f = line[(k + 1).min(line.len() - 1)] - line[k.saturating_sub(1)];
    Vec3::new(-f.z, 0.0, f.x).normalize_or(Vec3::X)
}

/// A profile swept along a line (stage_builder.gd _sweep), with normals
/// from the profile's own edges.
fn sweep(line: &[Vec3], profile: &[[f32; 2]], verts: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    let mut along = 0.0f32;
    for k in 0..line.len() {
        if k > 0 {
            along += line[k].distance(line[k - 1]);
        }
        let right = side_of(line, k);
        for (q, pr) in profile.iter().enumerate() {
            let prev = profile[q.saturating_sub(1)];
            let next = profile[(q + 1).min(profile.len() - 1)];
            let (dx, dy) = (next[0] - prev[0], next[1] - prev[1]);
            let n = (right * dy - Vec3::Y * dx).normalize_or(Vec3::Y);
            verts.push(v(line[k] + right * pr[0] + Vec3::Y * pr[1], n, [along, pr[1]]));
        }
    }
    let pn = profile.len() as u32;
    let base = verts.len() as u32 - pn * line.len() as u32;
    for k in 0..line.len() as u32 - 1 {
        for q in 0..pn - 1 {
            let a = base + k * pn + q;
            idx.extend_from_slice(&[a, a + pn, a + 1, a + 1, a + pn, a + pn + 1]);
        }
    }
}

fn wall_mesh(line: &[Vec3], material: u32) -> ClusterMesh {
    // sweep expects counterclockwise profiles. The previous reversed order
    // pointed both the side normals and triangle winding into the wall, and
    // its top normal down; the sunny side consequently received no direct sun.
    let profile = [[0.28, -0.5], [0.24, 0.62], [0.12, 0.74], [-0.12, 0.74], [-0.24, 0.62], [-0.28, -0.5]];
    let (mut verts, mut idx) = (Vec::new(), Vec::new());
    sweep(line, &profile, &mut verts, &mut idx);
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// Armco: a W-beam at bumper height on posts every two metres.
fn rail_mesh(line: &[Vec3], material: u32) -> ClusterMesh {
    let beam = [[0.0, 0.42], [0.07, 0.48], [0.0, 0.55], [0.07, 0.62], [0.0, 0.68], [-0.03, 0.68], [-0.03, 0.42]];
    let (mut verts, mut idx) = (Vec::new(), Vec::new());
    sweep(line, &beam, &mut verts, &mut idx);
    for k in 0..line.len() {
        let right = side_of(line, k);
        let f = (line[(k + 1).min(line.len() - 1)] - line[k.saturating_sub(1)]).normalize_or(Vec3::Z);
        let c = line[k] - right * 0.08 + Vec3::Y * 0.1;
        boxed(c, right * 0.05, Vec3::Y * 0.6, f * 0.04, &mut verts, &mut idx);
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// A box from its centre and three half-axes.
fn boxed(c: Vec3, x: Vec3, y: Vec3, z: Vec3, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    for (axis, u, w) in [(x, y, z), (y, z, x), (z, x, y)] {
        for s in [-1.0f32, 1.0] {
            let n = (axis * s).normalize_or(Vec3::Y);
            let base = verts.len() as u32;
            for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                verts.push(v(c + axis * s + u * a + w * b, n, [a, b]));
            }
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

pub struct CarPose {
    pub pos: Vec3,
    pub rot: Quat,
    pub color: Vec3,
    pub boxes: Vec<[f64; 6]>,
    /// res://assets/cars/x.glb, or empty for a box car.
    pub model: String,
    pub ride: f32,
    pub wheels: Vec<crate::car_model::Wheel>,
}

pub fn build(stage: &Stage, cars: &[CarPose], cameras: &[Vec3], ground_cover: bool) -> Result<Built, String> {
    build_inner(stage, cars, cameras, ground_cover, false)
}

pub fn build_live(stage: &Stage, cars: &[CarPose], cameras: &[Vec3]) -> Result<Built, String> {
    build_inner(stage, cars, cameras, false, std::env::var("SIDEWAYS_LIVE_GRASS").as_deref() != Ok("0"))
}

fn build_inner(stage: &Stage, cars: &[CarPose], cameras: &[Vec3], ground_cover: bool, live_cover: bool) -> Result<Built, String> {
    let mut log = Vec::new();
    let t = std::time::Instant::now();
    let mut scene = Scene::default();
    let r = root();
    let tex = |scene: &mut Scene, rel: &str| scene.add_texture(&r.join(rel));
    // Terrain layers, in the order shade.wgsl blends them.
    let t0 = tex(&mut scene, "assets/textures/forest_leaves_02_albedo.jpg")?; // moss
    tex(&mut scene, "assets/textures/forrest_ground_03_albedo.jpg")?; // needles
    tex(&mut scene, "assets/textures/grass_ground_albedo.jpg")?; // verge
    tex(&mut scene, "assets/textures/forest_ground_04_albedo.jpg")?; // bare ground
    tex(&mut scene, "assets/textures/rock_face_03_albedo.jpg")?; // rock
    let gravel = tex(&mut scene, "assets/textures/gravel_ground_01_albedo.jpg")?;
    tex(&mut scene, "assets/textures/forest_ground_04_albedo.jpg")?;
    let trunk_a = tex(&mut scene, "assets/trees/trunk_a_albedo.jpg")?;
    let trunk_b = tex(&mut scene, "assets/trees/trunk_b_albedo.jpg")?;
    let bark = tex(&mut scene, "assets/trees/bark_albedo.jpg")?;
    let crown = tex(&mut scene, "assets/trees/authored_pine_albedo.png")?;
    scene.load_sky(&r.join("assets/sky/sky_2k.hdr"))?;
    let m = |tint: [f32; 3], layer: i32, flags: u32, scale: f32| Material { tint, layer, roughness: 0.9, flags, scale, metal: 0.0 };
    let terrain_mat = scene.add_material(m([1.0, 1.0, 1.0], t0, TERRAIN, 4.0));
    let road_mat = scene.add_material(m([1.9, 1.85, 1.8], gravel, ROAD, 3.0));
    let trunk_a_mat = scene.add_material(m([1.0, 1.0, 1.0], trunk_a, 0, 1.0));
    let trunk_b_mat = scene.add_material(m([1.0, 1.0, 1.0], trunk_b, 0, 1.0));
    let bark_mat = scene.add_material(m([1.0, 1.0, 1.0], bark, 0, 1.0));
    // Godot's foliage.gdshader tint.
    let crown_mat = scene.add_material(m([0.62, 0.66, 0.55], crown, ALPHA_TEST | TWO_SIDED, 1.0));
    let barrier_mat = scene.add_material(m([0.55, 0.56, 0.57], -1, 0, 1.0));
    log.push(format!("textures and sky: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));

    // Terrain and trees, chunk by chunk.
    let t = std::time::Instant::now();
    let cells = builder::chunk_cells(stage);
    let chunks: Vec<Chunk> = std::thread::scope(|s| {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let per = cells.len().div_ceil(threads);
        let handles: Vec<_> = cells.chunks(per).map(|part| s.spawn(move || part.iter().map(|c| builder::terrain_chunk(stage, *c)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    log.push(format!("terrain data: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));
    let t = std::time::Instant::now();
    let meshes: Vec<ClusterMesh> = std::thread::scope(|s| {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let per = chunks.len().div_ceil(threads);
        let handles: Vec<_> = chunks.chunks(per).map(|part| s.spawn(move || part.iter().map(|c| chunk_mesh(c, terrain_mat)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let mut mesh_names = Vec::new();
    for (k, mesh) in meshes.iter().enumerate() {
        let id = scene.add_mesh(mesh);
        scene.add_instance(id, Affine3A::IDENTITY);
        mesh_names.push(format!("terrain chunk {:?}", chunks[k].cell));
    }
    scene.aux = stage.road_surfaces.iter().map(|s| *s as u32).collect();
    scene.heightfield = heightfield(&chunks);
    let rm = road_mesh(stage, road_mat);
    let road = scene.add_mesh(&rm);
    mesh_names.push("road".into());
    scene.add_instance(road, Affine3A::IDENTITY);
    log.push(format!("terrain + road clusters: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));

    // Trees: three variants, Godot's near meshes.
    let t = std::time::Instant::now();
    let mut tree_meshes = Vec::new();
    for variant in ["a", "b", "c"] {
        let path = r.join(format!("engine/assets/fir_{variant}_near.kmesh"));
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e} (run tools/export_meshes.gd)", path.display()))?;
        let surfaces = read_kmesh(&bytes)?;
        let parts: Vec<(Vec<Vertex>, Vec<u32>, u32)> = surfaces
            .into_iter()
            .map(|(name, verts, idx)| {
                let mat = if name.contains("trunk_b") {
                    trunk_b_mat
                } else if name.contains("trunk") {
                    trunk_a_mat
                } else if name.contains("bark") {
                    bark_mat
                } else {
                    crown_mat
                };
                (verts, idx, mat)
            })
            .collect();
        let p: Vec<Part> = parts.iter().map(|(v, i, m)| Part { vertices: v, indices: i, material: *m }).collect();
        let cm = ClusterMesh::build_with_policy(&p, |part| if part.material == crown_mat {
            render::cluster::SimplificationPolicy::AlphaCards
        } else {
            render::cluster::SimplificationPolicy::Geometry
        });
        log.push(format!("fir_{variant}: {} leaf tris, {} root tris, {} clusters, {} levels", cm.leaf_triangles(), cm.root_triangles(), cm.clusters.len(), cm.levels));
        tree_meshes.push(scene.add_mesh(&cm));
        mesh_names.push(format!("fir_{variant}"));
    }
    let mut trees = 0;
    for c in &chunks {
        for (k, spot) in c.trees.iter().enumerate() {
            let variant = ((spot.pos.0 * 7.31 + spot.pos.2 * 3.17).abs() as usize + k) % 3;
            let xf = Affine3A::from_scale_rotation_translation(
                Vec3::new(spot.scale, spot.scale_y, spot.scale),
                Quat::from_rotation_y(spot.yaw),
                Vec3::new(spot.pos.0, spot.pos.1, spot.pos.2),
            );
            scene.add_instance(tree_meshes[variant], xf);
            scene.mark_foliage();
            trees += 1;
        }
    }
    log.push(format!("trees: {trees} instances, {:.0} ms", t.elapsed().as_secs_f64() * 1e3));

    // World-fixed trees extend the forested horizon, using the same source
    // models and LOD as the near forest. Native road/physics are retained.
    if std::env::var("SIDEWAYS_DISTANT_FOREST").as_deref() != Ok("0") {
        match crate::distant_forest::add(&mut scene, stage, &chunks, terrain_mat, &tree_meshes, &mut mesh_names, Default::default()) {
            Ok(summary) => log.push(summary),
            Err(reason) => log.push(format!("distant forest unavailable: {reason}")),
        }
    }
    if std::env::var("SIDEWAYS_PROCEDURAL_ROCKS").as_deref() != Ok("0") {
        match crate::procedural_rocks::add(&mut scene, stage, &chunks, &mut mesh_names) {
            Ok(summary) => log.push(summary),
            Err(reason) => log.push(format!("procedural rocks unavailable: {reason}")),
        }
    }

    if let Some(summary)=crate::voxel_tree_patch::attach(&mut scene,&tree_meshes,crown_mat,cameras,&r)? {
        log.push(summary);
    }

    let live_ground_cover = if live_cover {
        let stream = crate::live_ground_cover::LiveGroundCover::reserve(&mut scene, &chunks, cameras, &r, &mut mesh_names)?;
        log.push(stream.summary());
        if std::env::var("SIDEWAYS_FAR_GRASS").as_deref() == Ok("1") {
            let photo = crate::ground_cover::photo(&r)?;
            let (field, summary) = crate::far_ground_cover::bake(&chunks, &photo)?;
            log.push(summary);
            scene.far_grass = Some(field);
        }
        Some(stream)
    } else if ground_cover {
        log.push(crate::ground_cover::add(&mut scene, &chunks, cameras, &r, &mut mesh_names)?);
        None
    } else {
        log.push("ground cover: disabled (matched A/B)".into());
        None
    };

    // Barriers along the ledges: Armco rails or dry-stone walls.
    let stone = tex(&mut scene, "assets/textures/stone_wall_04_albedo.jpg")?;
    let stone_mat = scene.add_material(m([0.78, 0.76, 0.72], stone, TRIPLANAR, 1.0 / 0.6));
    let _ = barrier_mat;
    // stage_builder.gd's steel: Color(0.62, 0.64, 0.66) (sRGB), metallic 0.7, roughness 0.42.
    let steel_mat = scene.add_material(Material { tint: [0.342, 0.368, 0.392], layer: -1, roughness: 0.42, flags: 0, scale: 1.0, metal: 0.7 });
    for line in builder::barrier_lines(stage) {
        let pts: Vec<Vec3> = line.points.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();
        let mesh = if line.rail { rail_mesh(&pts, steel_mat) } else { wall_mesh(&pts, stone_mat) };
        let id = scene.add_mesh(&mesh);
        mesh_names.push(if line.rail { "armco".into() } else { "stone wall".into() });
        scene.add_instance(id, Affine3A::IDENTITY);
    }
    let mut car_textures = std::collections::HashMap::new();
    let mut dynamic_cars = Vec::new();
    for car in cars {
        let first = scene.instances.len();
        let pose = Affine3A::from_rotation_translation(car.rot, car.pos);
        if let Some(rel) = car.model.strip_prefix("res://") {
            let t = std::time::Instant::now();
            let before = scene.meshes.len();
            let meshes = crate::car_model::load(&mut scene, &r.join(rel), car.color.to_array(), &mut car_textures)?;
            for k in before..scene.meshes.len() {
                mesh_names.push(format!("car {rel} part {}", k - before));
            }
            crate::car_model::place(&mut scene, &meshes, pose, car.ride, &car.wheels);
            dynamic_cars.push(DynamicCar { first, meshes, fallback: false });
            log.push(format!("car {rel}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));
        } else {
            let mat = scene.add_material(m(car.color.to_array(), -1, 0, 1.0));
            let id = scene.add_mesh(&car_mesh(&car.boxes, mat));
            mesh_names.push("car box".into());
            scene.add_instance(id, pose);
            dynamic_cars.push(DynamicCar { first, meshes: crate::car_model::CarMeshes { body: id, wheels: [None; 4] }, fallback: true });
        }
    }
    log.push(scene.stats());
    Ok(Built { scene, mesh_names, log, cars: dynamic_cars, live_ground_cover })
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn stone_wall_top_faces_up_and_sides_face_outward_with_matching_winding() {
        let mesh = wall_mesh(&[Vec3::ZERO, Vec3::Z * 4.0], 0);
        for v in &mesh.vertices {
            let p = Vec3::from(v.pos);
            let n = Vec3::from(v.normal);
            assert!(p.x * n.x > 0.0, "wall side normal points inward: {p:?}, {n:?}");
            if p.y > 0.73 {
                assert!(n.y > 0.8, "top shading normal must point up");
            }
        }
        let mut top_triangles = 0;
        for c in mesh.clusters.iter().filter(|c| c.own_error == 0.0) {
            for packed in &mesh.tris[c.first_tri as usize..(c.first_tri + c.tri_count) as usize] {
                let vertices: [Vertex; 3] = std::array::from_fn(|k| {
                    let local = (packed >> (8 * k)) & 0xff;
                    mesh.vertices[mesh.cluster_verts[(c.first_vert + local) as usize] as usize]
                });
                let p = vertices.map(|v| Vec3::from(v.pos));
                let face = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
                let normal = vertices.iter().map(|v| Vec3::from(v.normal)).sum::<Vec3>().normalize();
                assert!(face.dot(normal) > 0.5, "triangle winding disagrees with shading normals");
                if p.iter().all(|v| v.y > 0.73) {
                    assert!(face.y > 0.99, "top winding faces down");
                    top_triangles += 1;
                } else {
                    let x = p.iter().map(|v| v.x).sum::<f32>();
                    assert!(face.x * x > 0.0, "side winding faces inward");
                }
            }
        }
        assert_eq!(top_triangles, 2);
    }

    #[test]
    fn sloped_terrain_has_the_same_normal_at_all_chunk_edges_and_corners() {
        let heights = (0..N).flat_map(|j| (0..N).map(move |i| {
            (i as f32 * 0.25 - j as f32 * 0.75) * GRID as f32
        })).collect();
        let chunk = Chunk {
            cell: (0, 0), x0: 0.0, z0: 0.0, heights, road_d: vec![20.0; N * N],
            trees: vec![], rocks: vec![], grass: vec![], ferns: vec![], stumps: vec![],
        };
        let mesh = chunk_mesh(&chunk, 0);
        let expected = Vec3::new(-0.25, 1.0, 0.75).normalize();
        assert_eq!(mesh.vertices.len(), N * N);
        for v in &mesh.vertices {
            assert!(Vec3::from(v.normal).abs_diff_eq(expected, 1e-5), "wrong slope normal at {:?}: {:?}", v.pos, v.normal);
        }
    }
}
