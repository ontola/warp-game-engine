//! Original fractured rock relief on the stage's existing uphill cut faces.
//! The native 2 m terrain remains the collider. This is a bounded render-only
//! detail layer, not new free-standing boulders with missing collisions.
//! Dense, welded source geometry enters the existing meshoptimizer cluster
//! LOD pipeline; there is no new distance switch, dither or runtime tessellation.

use glam::{Affine3A, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, Scene, TRIPLANAR};
use sideways::builder::{Chunk, N};
use sideways::stage::{CHUNK, GRID, STEP, Stage};
use std::{collections::HashMap, path::Path, time::Instant};

const MAX_PATCHES: usize = 12;
const ALONG_CELLS: usize = 96;
const ACROSS_CELLS: usize = 64;
const LENGTH: f32 = 14.0;
const ACROSS_START: f32 = 2.5;
const ACROSS_END: f32 = 8.5;
const MAX_RELIEF: f32 = 0.46;
const BURIED_BORDER: f32 = 0.09;

fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e3779b9)
        ^ (y as u32).wrapping_mul(0x85ebca6b) ^ seed;
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846ca68b);
    n ^ (n >> 16)
}
fn unit(n: u32) -> f32 { (n >> 8) as f32 / 16_777_216.0 }
fn smooth(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// A jittered fracture network in a local metre-space rock face. Each cell
/// has its own planar face; narrow recessed boundaries expose the joints.
/// No camera-dependent noise or material darkening is involved.
fn fracture(p: Vec2, size: f32, seed: u32) -> (f32, f32) {
    let q = p / size;
    let cell = q.floor();
    let mut best = f32::INFINITY;
    let mut second = f32::INFINITY;
    let mut nearest = Vec2::ZERO;
    let mut nearest_key = 0;
    let mut runner_up = Vec2::ZERO;
    for y in -1..=1 { for x in -1..=1 {
        let ix = cell.x as i32 + x;
        let iy = cell.y as i32 + y;
        let key = hash(ix, iy, seed);
        let site = Vec2::new(ix as f32 + 0.18 + unit(key) * 0.64,
            iy as f32 + 0.18 + unit(key.rotate_left(13)) * 0.64);
        let distance = q.distance_squared(site);
        if distance < best {
            second = best; runner_up = nearest;
            best = distance; nearest = site; nearest_key = key;
        } else if distance < second {
            second = distance; runner_up = site;
        }
    }}
    // Distance to the bisector, rather than F2-F1, keeps crack width roughly
    // constant on differently sized faces.
    let joint = (second - best) * size / (2.0 * nearest.distance(runner_up).max(0.01));
    let local = (q - nearest) * size;
    let tilt = Vec2::new(unit(nearest_key.rotate_left(7)) - 0.5,
        unit(nearest_key.rotate_left(21)) - 0.5);
    let plane = (0.15 + 0.17 * unit(nearest_key.rotate_left(3)) + local.dot(tilt) * 0.10)
        .clamp(0.06, 0.36);
    (plane, joint.max(0.0))
}

fn relief(p: Vec2, seed: u32) -> f32 {
    // Tilted strata and two joint scales produce slabs and chipped edges,
    // instead of a rounded noise blob or a visible regular voxel lattice.
    let q = Vec2::new(p.x + p.y * 0.19, p.y - p.x * 0.11);
    let (face, joint) = fracture(q, 2.15, seed);
    let (chip, chip_joint) = fracture(q + Vec2::new(0.71, 0.23), 0.63, seed ^ 0x374d13);
    let bed = (q.y * 0.83 + (q.x * 0.19).sin() * 0.12).rem_euclid(1.0);
    let bed_joint = bed.min(1.0 - bed) / 0.83;
    let strata = 0.65 + 0.35 * smooth(bed_joint / 0.11);
    (0.035 + face * smooth(joint / 0.18) * strata
        + chip * 0.12 * smooth(chip_joint / 0.07)).min(MAX_RELIEF)
}

fn triangle_sample(values: &[f32], i: usize, j: usize, u: f32, v: f32) -> f32 {
    let a = values[j * N + i];
    let b = values[j * N + i + 1];
    let c = values[(j + 1) * N + i];
    let d = values[(j + 1) * N + i + 1];
    if u + v <= 1.0 { a + (b - a) * u + (c - a) * v }
    else { d + (c - d) * (1.0 - u) + (b - d) * (1.0 - v) }
}

struct NativeGround<'a>(HashMap<(i32, i32), &'a Chunk>);
impl NativeGround<'_> {
    /// Match builder::Chunk::triangles, including its diagonal. Sampling the
    /// analytic stage instead would float above the actual rendered/colliding
    /// cut face. Background terrain is deliberately never sampled here.
    fn sample(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let cell = ((x / CHUNK as f32).floor() as i32, (z / CHUNK as f32).floor() as i32);
        for dz in [0, -1] { for dx in [0, -1] {
            let Some(c) = self.0.get(&(cell.0 + dx, cell.1 + dz)) else { continue; };
            let u = (x - c.x0 as f32) / GRID as f32;
            let v = (z - c.z0 as f32) / GRID as f32;
            let end = (N - 1) as f32;
            if !(0.0..=end).contains(&u) || !(0.0..=end).contains(&v) { continue; }
            let i = (u.floor() as usize).min(N - 2);
            let j = (v.floor() as usize).min(N - 2);
            return Some((triangle_sample(&c.heights, i, j, u - i as f32, v - j as f32),
                triangle_sample(&c.road_d, i, j, u - i as f32, v - j as f32)));
        }}
        None
    }
}

fn grid_indices(side: f32) -> Vec<u32> {
    let row = (ALONG_CELLS + 1) as u32;
    let mut indices = Vec::with_capacity(ALONG_CELLS * ACROSS_CELLS * 6);
    for j in 0..ACROSS_CELLS { for i in 0..ALONG_CELLS {
        let a = j as u32 * row + i as u32;
        if side > 0.0 { indices.extend_from_slice(&[a, a + row, a + 1, a + 1, a + row, a + row + 1]); }
        else { indices.extend_from_slice(&[a, a + 1, a + row, a + 1, a + row + 1, a + row]); }
    }}
    indices
}

fn normals(vertices: &mut [Vertex], indices: &[u32]) {
    let mut sums = vec![Vec3::ZERO; vertices.len()];
    for tri in indices.chunks_exact(3) {
        let p: [Vec3; 3] = std::array::from_fn(|k| Vec3::from(vertices[tri[k] as usize].pos));
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        for &index in tri { sums[index as usize] += n; }
    }
    for (vertex, sum) in vertices.iter_mut().zip(sums) { vertex.normal = sum.normalize_or(Vec3::Y).to_array(); }
}

fn patch(stage: &Stage, ground: &NativeGround, index: usize) -> Option<(Vec<Vertex>, Vec<u32>)> {
    let side = stage.ledges[index].signum();
    let along_centre = index as f32 * STEP as f32;
    let seed = stage.noise_seed as u32 ^ (index as u32).wrapping_mul(3779);
    let mut vertices = Vec::with_capacity((ALONG_CELLS + 1) * (ACROSS_CELLS + 1));
    for j in 0..=ACROSS_CELLS {
        let v = j as f32 / ACROSS_CELLS as f32;
        for i in 0..=ALONG_CELLS {
            let u = i as f32 / ALONG_CELLS as f32;
            let along = along_centre + (u - 0.5) * LENGTH;
            let row = (along / STEP as f32).floor() as usize;
            if row + 1 >= stage.points.len() || stage.ledges[row] * side < 0.55 { return None; }
            let t = along / STEP as f32 - row as f32;
            let a = stage.points[row]; let b = stage.points[row + 1];
            let centre = Vec3::new(a.x, a.y, a.z).lerp(Vec3::new(b.x, b.y, b.z), t);
            let da = stage.dirs[row]; let db = stage.dirs[row + 1];
            let dir = Vec3::new(da.x, 0.0, da.z).lerp(Vec3::new(db.x, 0.0, db.z), t).normalize();
            let uphill = Vec3::new(-dir.z, 0.0, dir.x) * side;
            let width = stage.widths[row] * (1.0 - t) + stage.widths[row + 1] * t;
            let p = centre + uphill * (width + ACROSS_START + v * (ACROSS_END - ACROSS_START));
            let (height, road_distance) = ground.sample(p.x, p.z)?;
            if road_distance < 1.65 { return None; }
            let point = Vec3::new(p.x, height, p.z);
            let face = (-uphill * 0.94 + Vec3::Y * 0.34).normalize();
            let envelope = smooth(u.min(1.0 - u) * LENGTH / 0.85)
                * smooth(v.min(1.0 - v) * (ACROSS_END - ACROSS_START) / 0.70);
            let detail = relief(Vec2::new(along, height - stage.points[index].y), seed);
            let displacement = (detail + BURIED_BORDER) * envelope - BURIED_BORDER;
            let position = point + face * displacement;
            // The small near-road relief is visual only; never protrude into
            // another road at a tight bend or a nearby return leg.
            let (_, final_clearance) = ground.sample(position.x, position.z)?;
            if final_clearance < 1.25 { return None; }
            vertices.push(Vertex { pos: position.to_array(), normal: Vec3::Y.to_array(), uv: [0.0, 0.0] });
        }
    }
    let indices = grid_indices(side);
    normals(&mut vertices, &indices);
    Some((vertices, indices))
}

/// Add at most twelve high-detail rock exposures to existing native cliffs.
/// Existing barriers and their colliders remain untouched. The source asset
/// texture is the already bundled CC0 Poly Haven rock_face_03; its attribution
/// remains in assets/licenses/CREDITS.txt. All geometry here is original.
pub fn add(scene: &mut Scene, stage: &Stage, chunks: &[Chunk], mesh_names: &mut Vec<String>) -> Result<String, String> {
    let started = Instant::now();
    if stage.points.len() < 24 || chunks.is_empty() { return Ok("procedural rocks: no native cliff candidates".into()); }
    let candidates: Vec<usize> = (12..stage.points.len() - 12).step_by(9)
        .filter(|&i| stage.ledges[i].abs() > 0.8 && stage.ledge_wall[i] > 5.0 && stage.curvature[i].abs() < 1.0 / 28.0)
        .collect();
    if candidates.is_empty() { return Ok("procedural rocks: this stage has no suitable uphill cut faces".into()); }
    let ground = NativeGround(chunks.iter().map(|c| (c.cell, c)).collect());
    let texture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/textures/rock_face_03_albedo.jpg");
    let existing_layer = scene.textures.iter().position(|source|
        Path::new(&source.name).file_name() == texture.file_name());
    let layer = match existing_layer { Some(index) => index as i32, None => scene.add_texture(&texture)? };
    let material = scene.add_material(Material { tint: [0.78, 0.77, 0.74], layer, roughness: 0.94,
        flags: TRIPLANAR, scale: 1.0 / 1.8, metal: 0.0 });
    let count = candidates.len().min(MAX_PATCHES);
    let mut added = 0; let mut source_triangles = 0; let mut root_triangles = 0; let mut levels = 0;
    let mut first_along = None;
    for k in 0..count {
        let chosen = if count <= 1 { 0 } else { k * (candidates.len() - 1) / (count - 1) };
        let index = candidates[chosen];
        let Some((vertices, indices)) = patch(stage, &ground, index) else { continue; };
        let mesh = ClusterMesh::build(&[Part { vertices: &vertices, indices: &indices, material }]);
        source_triangles += mesh.leaf_triangles(); root_triangles += mesh.root_triangles(); levels = levels.max(mesh.levels);
        let id = scene.add_mesh(&mesh);
        scene.add_instance(id, Affine3A::IDENTITY);
        mesh_names.push(format!("original fractured cliff at {:.0} m", index as f64 * STEP));
        first_along.get_or_insert(index as f64 * STEP);
        added += 1;
    }
    Ok(format!("procedural rocks: {added} native cliff exposures, {source_triangles} source / {root_triangles} root LOD tris, up to {levels} levels, first at {:.0} m, {:.0} ms; original geometry, at most {MAX_RELIEF:.2} m visual relief over unchanged terrain collision",
        first_along.unwrap_or(0.0), started.elapsed().as_secs_f64() * 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fracture_detail_is_repeatable_bounded_and_has_real_relief() {
        let mut lo = f32::INFINITY; let mut hi = 0.0f32;
        for j in -20..20 { for i in -20..20 {
            let p = Vec2::new(i as f32 * 0.137, j as f32 * 0.191);
            let value = relief(p, 47);
            assert_eq!(value.to_bits(), relief(p, 47).to_bits());
            assert!(value.is_finite() && (0.0..=MAX_RELIEF).contains(&value));
            lo = lo.min(value); hi = hi.max(value);
        }}
        assert!(hi - lo > 0.15);
    }

    #[test]
    fn native_sampler_uses_collision_diagonal_not_bilinear_interpolation() {
        let mut heights = vec![0.0; N * N];
        heights[0] = 0.0; heights[1] = 2.0; heights[N] = 4.0; heights[N + 1] = 10.0;
        assert!((triangle_sample(&heights, 0, 0, 0.25, 0.25) - 1.5).abs() < 1e-6);
        assert!((triangle_sample(&heights, 0, 0, 0.75, 0.75) - 6.5).abs() < 1e-6);
    }

    #[test]
    fn left_and_right_cliffs_are_welded_and_wound_outward() {
        for side in [-1.0, 1.0] {
            let mut vertices = Vec::new();
            for j in 0..=ACROSS_CELLS { for i in 0..=ALONG_CELLS {
                vertices.push(Vertex { pos: [-(j as f32) * side, j as f32 * 2.0, i as f32], normal: [0.0; 3], uv: [0.0; 2] });
            }}
            let indices = grid_indices(side);
            assert_eq!(indices.len(), ALONG_CELLS * ACROSS_CELLS * 6);
            normals(&mut vertices, &indices);
            let outward = Vec3::new(2.0 * side, 1.0, 0.0).normalize();
            for vertex in vertices { assert!(Vec3::from(vertex.normal).dot(outward) > 0.999); }
        }
    }
}
