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

fn noise(p: Vec2, seed: u32) -> f32 {
    let cell = p.floor(); let f = p - cell;
    let x = cell.x as i32; let y = cell.y as i32;
    let fx = smooth(f.x); let fy = smooth(f.y);
    let a = unit(hash(x, y, seed)); let b = unit(hash(x + 1, y, seed));
    let c = unit(hash(x, y + 1, seed)); let d = unit(hash(x + 1, y + 1, seed));
    ((a + (b - a) * fx) * (1.0 - fy) + (c + (d - c) * fx) * fy) * 2.0 - 1.0
}

fn rotate(p: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos(); Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c)
}

// Beds have independently jittered levels and gentle lateral warping. They
// also break off along the wall; there is no periodic rem_euclid stripe mask.
fn bed_height(index: i32, along: f32, seed: u32) -> f32 {
    index as f32 * 1.55 + (unit(hash(index, 71, seed)) - 0.5) * 1.05
        + noise(Vec2::new(along / 11.0, index as f32 * 0.71), seed ^ 0xba519) * 0.18
}

#[derive(Clone, Copy)]
struct PatchShape { length: f32, along_shift: f32, start: f32, end: f32, ragged: f32, seed: u32 }
impl PatchShape {
    fn varied(index: usize, seed: u32) -> Self {
        let key = hash(index as i32, 19, seed);
        Self { length: 11.5 + unit(key) * 8.0, along_shift: (unit(key.rotate_left(11)) - 0.5) * 4.4,
            start: 2.5 + unit(key.rotate_left(21)) * 0.6, end: 8.2 + unit(key.rotate_left(5)) * 1.5,
            ragged: 1.0, seed: key }
    }
    fn safe(index: usize, seed: u32) -> Self {
        Self { length: LENGTH, along_shift: 0.0, start: ACROSS_START, end: ACROSS_END, ragged: 0.0,
            seed: hash(index as i32, 19, seed) }
    }
    fn interval(self, index: usize) -> (f32, f32) {
        let centre = index as f32 * STEP as f32 + self.along_shift;
        // Include the bounded interior coordinate warp in the overlap guard.
        (centre - self.length * 0.5 - self.ragged * 0.45, centre + self.length * 0.5 + self.ragged * 0.45)
    }
}

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
    // One continuous metre-space geology field across a stage, not a reset
    // local pattern for every 14 m patch. Domain warping breaks the regular
    // fracture-cell lattice; differently oriented scales expose large slabs.
    let angle = (unit(hash(17, 31, seed)) - 0.5) * 0.9;
    let rotated = rotate(p, angle);
    let warp = Vec2::new(noise(rotated / 13.0, seed ^ 0x19351) * 1.25,
        noise(rotated / 19.0 + Vec2::new(7.1, 2.3), seed ^ 0x419af) * 0.85);
    let q = rotated + warp;
    let macro_size = 3.2 + noise(p / 43.0, seed ^ 0x99e31) * 0.95;
    let (face, joint) = fracture(q * Vec2::new(0.65, 1.08), macro_size, seed);
    let chip_angle = 0.55 + unit(hash(23, 9, seed)) * 0.75;
    let (chip, chip_joint) = fracture(rotate(q + Vec2::new(2.71, 4.23), chip_angle),
        1.15 + noise(p / 27.0, seed ^ 0x7153) * 0.35, seed ^ 0x374d13);
    let mut bedding = 1.0f32;
    let bed_index = (q.y / 1.55).floor() as i32;
    for index in bed_index - 2..=bed_index + 2 {
        let distance = (q.y - bed_height(index, q.x, seed)).abs();
        let continuity = smooth((noise(Vec2::new(q.x / 7.5, index as f32 * 1.91), seed ^ 0xabc713) + 0.1) / 0.85);
        bedding = bedding.min(1.0 - 0.28 * continuity * (1.0 - smooth(distance / 0.075)));
    }
    (0.025 + face * smooth(joint / 0.15) * bedding
        + chip * 0.13 * smooth(chip_joint / 0.055)).min(MAX_RELIEF)
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

fn patch(stage: &Stage, ground: &NativeGround, index: usize, shape: PatchShape) -> Option<(Vec<Vertex>, Vec<u32>)> {
    let side = stage.ledges[index].signum();
    let along_centre = index as f32 * STEP as f32 + shape.along_shift;
    let geology_seed = stage.noise_seed as u32 ^ 0x7a6e193;
    let mut vertices = Vec::with_capacity((ALONG_CELLS + 1) * (ACROSS_CELLS + 1));
    for j in 0..=ACROSS_CELLS {
        let v = j as f32 / ACROSS_CELLS as f32;
        for i in 0..=ALONG_CELLS {
            let u = i as f32 / ALONG_CELLS as f32;
            let along = along_centre + (u - 0.5) * shape.length
                + shape.ragged * 0.45 * noise(Vec2::new(v * 2.3, along_centre / 17.0), shape.seed) * (u * std::f32::consts::PI).sin();
            let row = (along / STEP as f32).floor() as usize;
            if row + 1 >= stage.points.len() || stage.ledges[row] * side < 0.55 { return None; }
            let t = along / STEP as f32 - row as f32;
            let a = stage.points[row]; let b = stage.points[row + 1];
            let centre = Vec3::new(a.x, a.y, a.z).lerp(Vec3::new(b.x, b.y, b.z), t);
            let da = stage.dirs[row]; let db = stage.dirs[row + 1];
            let dir = Vec3::new(da.x, 0.0, da.z).lerp(Vec3::new(db.x, 0.0, db.z), t).normalize();
            let uphill = Vec3::new(-dir.z, 0.0, dir.x) * side;
            let width = stage.widths[row] * (1.0 - t) + stage.widths[row + 1] * t;
            let lower = shape.start + shape.ragged * (0.15 + 0.55 * (0.5 + 0.5 * noise(Vec2::new(along / 3.7, 2.1), shape.seed)));
            let upper = shape.end + shape.ragged * 0.6 * noise(Vec2::new(along / 7.1, 9.4), shape.seed ^ 0x713);
            let p = centre + uphill * (width + lower + v * (upper - lower));
            let (height, road_distance) = ground.sample(p.x, p.z)?;
            if road_distance < 1.65 { return None; }
            let point = Vec3::new(p.x, height, p.z);
            let face = (-uphill * 0.94 + Vec3::Y * 0.34).normalize();
            let envelope = smooth(u.min(1.0 - u) * shape.length / 0.85)
                * smooth(v.min(1.0 - v) * (upper - lower) / 0.70);
            let detail = relief(Vec2::new(along, height), geology_seed);
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
    let count = candidates.len().min(MAX_PATCHES);
    let mut added = 0; let mut source_triangles = 0; let mut root_triangles = 0; let mut levels = 0;
    let mut first_along = None; let mut used_intervals: Vec<(f32,f32)> = Vec::new(); let mut safe_footprints = 0;
    let mut min_length = f32::INFINITY; let mut max_length = 0.0f32;
    for k in 0..count {
        let chosen = if count <= 1 { 0 } else { k * (candidates.len() - 1) / (count - 1) };
        let index = candidates[chosen];
        let seed = stage.noise_seed as u32;
        let varied = PatchShape::varied(index, seed);
        let safe = PatchShape::safe(index, seed);
        let mut chosen = None;
        for (shape, fallback) in [(varied,false),(safe,true)] {
            let interval=shape.interval(index);
            if used_intervals.iter().any(|&(a,b)|interval.0 < b+0.5 && interval.1 > a-0.5){continue;}
            if let Some(geometry)=patch(stage,&ground,index,shape){chosen=Some((geometry,shape,interval,fallback));break;}
        }
        let Some(((vertices,indices),shape,interval,fallback))=chosen else{continue;};
        used_intervals.push(interval);safe_footprints+=fallback as usize;
        min_length=min_length.min(shape.length);max_length=max_length.max(shape.length);
        // scale is metres per texture repeat. The previous 0.56 m repeats
        // turned this large cliff into a visibly tiled strip. Keep grain at
        // a geological scale, varied per exposure without moving the mesh.
        let texture_scale=3.2+unit(hash(index as i32,83,seed))*2.6;
        let material = scene.add_material(Material { tint: [0.78, 0.77, 0.74], layer, roughness: 0.94,
            flags: TRIPLANAR, scale: texture_scale, metal: 0.0 });
        let mesh = ClusterMesh::build(&[Part { vertices: &vertices, indices: &indices, material }]);
        source_triangles += mesh.leaf_triangles(); root_triangles += mesh.root_triangles(); levels = levels.max(mesh.levels);
        let id = scene.add_mesh(&mesh);
        scene.add_instance(id, Affine3A::IDENTITY);
        mesh_names.push(format!("original fractured cliff at {:.0} m", index as f64 * STEP));
        first_along.get_or_insert(index as f64 * STEP);
        added += 1;
    }
    Ok(format!("procedural rocks: {added} native cliff exposures, {source_triangles} source / {root_triangles} root LOD tris, up to {levels} levels, first at {:.0} m, {:.0} ms; varied {:.1}-{max_length:.1} m footprints ({safe_footprints} clearance fallbacks), 3.2-5.8 m texture repeats, at most {MAX_RELIEF:.2} m visual relief over unchanged terrain collision",
        first_along.unwrap_or(0.0), started.elapsed().as_secs_f64() * 1000.0,if min_length.is_finite(){min_length}else{0.0}))
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
    fn bedding_thickness_and_exposure_footprints_do_not_repeat_uniformly() {
        let mut min_gap=f32::INFINITY;let mut max_gap=0.0f32;
        for index in -20..20{
            let a=bed_height(index,73.2,47);let b=bed_height(index+1,73.2,47);
            assert!(b>a,"jittered beds should keep their geological order");
            min_gap=min_gap.min(b-a);max_gap=max_gap.max(b-a);
        }
        assert!(max_gap-min_gap>0.5,"bed spacing must vary materially, not remain a periodic stripe");
        let mut lengths=std::collections::BTreeSet::new();
        for index in (12..160).step_by(9){
            let a=PatchShape::varied(index,47);let b=PatchShape::varied(index,47);
            assert_eq!(a.length.to_bits(),b.length.to_bits());assert_eq!(a.seed,b.seed);
            assert!((11.5..=19.5).contains(&a.length));assert!(a.start>=ACROSS_START);
            assert!(a.end>a.start+4.0);assert!(a.along_shift.abs()<=2.2);
            let interval=a.interval(index);assert!(interval.0<interval.1);
            lengths.insert(a.length.to_bits());
        }
        assert!(lengths.len()>8,"exposures should not all retain the same rectangular footprint");
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
