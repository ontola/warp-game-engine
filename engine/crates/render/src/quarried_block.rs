//! Angular, inward-cut masonry with shared seam positions and planar normals.
//! The collision envelope remains a unit cube. Cooking is deterministic and
//! intended for a small shared mesh palette, never per-frame deformation.

use crate::{cluster::Vertex, primitives::Primitive};
use glam::{DVec3, Vec3};
use std::collections::BTreeMap;

/// A unit stone block centred on the origin, clipped into irregular quarry
/// fractures. `wear` is clamped to 0..1; nonfinite input means intact stone.
/// Wear rounds to the nearest of five quarter-step chip stages; geometry
/// is not continuously deformed. Each stage retains
/// all earlier cuts, so increasing wear cannot restore previously lost stone.
///
/// Twelve uneven edge cuts and up to eight corner cuts leave broad, hard,
/// planar faces. More wear deepens the same cuts and exposes more corner
/// fractures. All six original box planes retain a flat central 0.32 m square,
/// including the top and bottom load-bearing surfaces. Nothing protrudes
/// beyond the unit collision box. At most 26 convex faces require at most
/// 92 triangles; no smooth normals or displacement noise are involved.
pub fn quarried_block(material: u32, wear: f32, seed: u32) -> Primitive {
    let wear = if wear.is_finite() { wear.clamp(0., 1.) } else { 0. };
    let mut faces = cube();
    let last_stage = (wear * 4.).round() as u32;
    for stage in 0..=last_stage { chip_stage(&mut faces, stage as f32 * 0.25, seed); }
    triangulate(material, faces)
}

fn chip_stage(faces: &mut Vec<Face>, wear: f32, seed: u32) {
    let mut rng = Rng(seed);
    // A few dressed stones retain straighter edges among the rough quarry
    // pieces. It is a seed choice, so damage does not reshuffle the shape.
    let dressed = if seed & 3 == 0 { 0.48 } else { 1.0 };
    // Each pair of axes identifies four edges. A small component along the
    // edge makes the cut taper, while unequal cross-edge weights avoid the
    // uniform chamfer of a manufactured or rounded cube.
    for (a, b, along) in [(0, 1, 2), (0, 2, 1), (1, 2, 0)] {
        for sa in [-1., 1.] { for sb in [-1., 1.] {
            let mut n = Vec3::ZERO;
            n[a] = sa * rng.range(0.65, 1.35);
            n[b] = sb * rng.range(0.65, 1.35);
            n[along] = rng.range(-0.10, 0.10);
            n = n.normalize();
            let depth = rng.range(0.035, 0.12) * dressed * (1. + wear * 0.85);
            clip(faces, n.as_dvec3(), f64::from(plane_offset(n, depth)));
        }}
    }
    let mut corners = [0u32, 1, 2, 3, 4, 5, 6, 7];
    for i in (1..corners.len()).rev() { corners.swap(i, rng.next() as usize % (i + 1)); }
    for (i, corner) in corners.into_iter().enumerate() {
        let n = Vec3::from_array(std::array::from_fn(|axis| {
            let sign = if corner & (1 << axis) == 0 { -1. } else { 1. };
            sign * rng.range(0.65, 1.35)
        })).normalize();
        // Generate every corner's parameters for every wear, so the cuts
        // are nested as damage increases rather than jumping to a new seed.
        let intact = rng.range(0.12, 0.21) * dressed;
        let extra = rng.range(0.07, 0.13);
        let depth = if i < 4 { intact + wear * extra } else { wear * (intact + extra) };
        if depth > 0. { clip(faces, n.as_dvec3(), f64::from(plane_offset(n, depth))); }
    }
}

/// Keep a flat central square on every original axis-aligned face. This
/// limits exceptionally deep cuts without moving any supporting box plane.
fn plane_offset(n: Vec3, depth: f32) -> f32 {
    let a = n.abs();
    let support = a.element_sum() * 0.5;
    let core_support = a.max_element() * 0.5 + (a.element_sum() - a.max_element()) * 0.16;
    // A small clearance keeps independently limited planes from meeting at
    // exactly the same protected-core corner, avoiding zero-area slivers.
    (support - depth).max(core_support + 0.004)
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9);
        let mut h = self.0;
        h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
        h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
        h ^ (h >> 16)
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((self.next() >> 8) as f32 / 16_777_216.)
    }
}

// Keep intersection geometry in double precision until the final upload.
// Repeated f32 clipping otherwise warps nominally planar faces by enough that
// a narrow triangle amplifies the error into a visibly inconsistent normal.
struct Face { points: Vec<DVec3>, normal: DVec3 }

fn frame(n: DVec3) -> (DVec3, DVec3) {
    let n = n.normalize();
    let reference = if n.y.abs() < 0.8 { DVec3::Y } else { DVec3::X };
    let u = reference.cross(n).normalize();
    (u, n.cross(u))
}

fn cube() -> Vec<Face> {
    [DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z].into_iter().map(|normal| {
        let (u, v) = frame(normal);
        let points = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
            .into_iter().map(|(x, y)| normal * 0.5 + u * x + v * y).collect();
        Face { points, normal }
    }).collect()
}

fn precise_key(p: DVec3) -> [u64; 3] { p.to_array().map(f64::to_bits) }

fn intersection(a: DVec3, b: DVec3, n: DVec3, h: f64, cache: &mut BTreeMap<([u64; 3], [u64; 3]), DVec3>) -> DVec3 {
    // Adjacent faces encounter the same edge in opposite directions. Compute
    // it once in canonical order so their seams are bit-identical, not merely
    // close enough for a tolerance weld.
    let (a, b) = if precise_key(a) <= precise_key(b) { (a, b) } else { (b, a) };
    *cache.entry((precise_key(a), precise_key(b))).or_insert_with(|| {
        let da = n.dot(a) - h;
        let db = n.dot(b) - h;
        if da == 0. { return a; }
        if db == 0. { return b; }
        let t = (da / (da - db)).clamp(0., 1.);
        (a + (b - a) * t).clamp(DVec3::splat(-0.5), DVec3::splat(0.5))
    })
}

fn clip(faces: &mut Vec<Face>, n: DVec3, mut h: f64) {
    // Do not shave a very narrow sliver off an existing vertex. Such a
    // sliver has no useful silhouette at this mesh's scale, but its long,
    // needle-shaped triangle magnifies f32 upload rounding. Move the cutting
    // plane outward by only the required clearance; the protected flat core
    // and collision envelope therefore remain untouched.
    const CLEARANCE: f64 = 0.006;
    loop {
        let nearby = faces.iter().flat_map(|face| &face.points).map(|&p| n.dot(p))
            .filter(|&distance| (distance - h).abs() < CLEARANCE)
            .max_by(f64::total_cmp);
        let Some(distance) = nearby else { break; };
        h = distance + CLEARANCE + 1e-12;
    }
    let mut cache = BTreeMap::new();
    let mut cap_points = BTreeMap::new();
    let mut result = Vec::with_capacity(faces.len() + 1);
    for face in faces.iter() {
        let mut points = Vec::with_capacity(face.points.len() + 1);
        for i in 0..face.points.len() {
            let a = face.points[i];
            let b = face.points[(i + 1) % face.points.len()];
            let a_inside = n.dot(a) <= h;
            let b_inside = n.dot(b) <= h;
            if a_inside != b_inside {
                let p = intersection(a, b, n, h, &mut cache);
                points.push(p); cap_points.insert(precise_key(p), p);
            }
            if b_inside { points.push(b); }
        }
        points.dedup_by(|a, b| precise_key(*a) == precise_key(*b));
        if points.len() > 1 && precise_key(points[0]) == precise_key(points[points.len() - 1]) { points.pop(); }
        if points.len() >= 3 { result.push(Face { points, normal: face.normal }); }
    }
    if cap_points.len() >= 3 {
        let mut points: Vec<DVec3> = cap_points.into_values().collect();
        let centre = points.iter().copied().sum::<DVec3>() / points.len() as f64;
        let (u, v) = frame(n);
        let angle = |p: DVec3| { let d = p - centre; d.dot(v).atan2(d.dot(u)) };
        points.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
        result.push(Face { points, normal: n });
    }
    // Two almost parallel older fracture planes can make a short cap edge
    // even when the cut is clear of their vertices. Retry slightly outward
    // rather than keeping a needle face or welding away part of the shell.
    // Original faces remain available until a valid cut is accepted.
    if result.iter().any(|face| (0..face.points.len()).any(|i| {
        face.points[i].distance_squared(face.points[(i + 1) % face.points.len()]) < 0.003f64.powi(2)
    })) {
        clip(faces, n, h + 0.003);
        return;
    }
    *faces = result;
}

fn triangulate(material: u32, faces: Vec<Face>) -> Primitive {
    let mut result = Primitive { material, ..Default::default() };
    for mut face in faces {
        // Choose the best fan rather than anchoring a long, thin triangle at
        // an arbitrary first vertex. This retains every boundary edge and the
        // same triangle budget while reducing error after f32 quantization.
        let count = face.points.len();
        let quality = |root: usize| (1..count - 1).map(|i| {
            let a = face.points[root];
            let b = face.points[(root + i) % count];
            let c = face.points[(root + i + 1) % count];
            let edge = (b - a).length_squared().max((c - a).length_squared()).max((c - b).length_squared());
            (b - a).cross(c - a).length_squared() / (edge * edge)
        }).fold(f64::INFINITY, f64::min);
        let root = (0..count).max_by(|&a, &b| quality(a).total_cmp(&quality(b))).unwrap();
        face.points.rotate_left(root);
        let first = result.vertices.len() as u32;
        let (u, v) = frame(face.normal);
        for &p in &face.points {
            result.vertices.push(Vertex { pos: p.as_vec3().to_array(), normal: face.normal.normalize().as_vec3().to_array(), uv: [(p.dot(u) + 0.5) as f32, (p.dot(v) + 0.5) as f32] });
        }
        for i in 1..face.points.len() - 1 {
            result.indices.extend([first, first + i as u32, first + i as u32 + 1]);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn key(p: Vec3) -> [u32; 3] { p.to_array().map(f32::to_bits) }

    fn volume(mesh: &Primitive) -> f64 {
        mesh.indices.chunks_exact(3).map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.vertices[i as usize].pos).as_dvec3());
            a.dot(b.cross(c)) / 6.
        }).sum()
    }

    fn assert_valid(mesh: &Primitive) {
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!((40..=92).contains(&(mesh.indices.len() / 3)), "triangle budget: {}", mesh.indices.len() / 3);
        let vertices: BTreeSet<_> = mesh.vertices.iter().map(|v| key(Vec3::from(v.pos))).collect();
        let mut edges: BTreeMap<([u32; 3], [u32; 3]), (u32, i32)> = BTreeMap::new();
        let mut support_area = [0f32; 6];
        for v in &mesh.vertices {
            let p = Vec3::from(v.pos); let n = Vec3::from(v.normal);
            assert!(p.is_finite() && n.is_finite() && v.uv.iter().all(|v| v.is_finite()));
            assert!(p.abs().max_element() <= 0.5);
            assert!((n.length_squared() - 1.).abs() < 2e-6);
        }
        for triangle in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| Vec3::from(mesh.vertices[i as usize].pos));
            let cross = (b - a).cross(c - a);
            assert!(cross.length_squared() > 1e-16, "degenerate triangle");
            let normal = cross.normalize();
            assert!(normal.dot(a + b + c) > 0., "inward triangle");
            for &index in triangle {
                assert!(normal.dot(Vec3::from(mesh.vertices[index as usize].normal)) > 0.9999, "nonplanar shading normal");
            }
            for v in &mesh.vertices {
                assert!(normal.dot(Vec3::from(v.pos) - a) < 1e-5, "nonconvex or reversed face");
            }
            for (i, n) in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z].into_iter().enumerate() {
                if normal.dot(n) > 0.99999 && (n.dot(a) - 0.5).abs() < 1e-6 { support_area[i] += cross.length() * 0.5; }
            }
            for (a, b) in [(a, b), (b, c), (c, a)] {
                let (a, b) = (key(a), key(b));
                assert_ne!(a, b);
                let (edge, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                let entry = edges.entry(edge).or_default(); entry.0 += 1; entry.1 += sign;
            }
        }
        assert!(edges.values().all(|&(count, direction)| count == 2 && direction == 0), "open seam or nonmanifold welded edge");
        assert_eq!(vertices.len() as isize - edges.len() as isize + (mesh.indices.len() / 3) as isize, 2, "closed convex shell Euler characteristic");
        assert!(support_area.iter().all(|&area| area >= 0.10), "lost flat supporting face: {support_area:?}");
        assert!((0.4..1.).contains(&volume(mesh)), "unexpected signed volume");
    }

    #[test]
    fn seeded_stones_are_closed_convex_outward_and_within_collision_box() {
        // Seed453 previously produced a near-parallel, needle-shaped cap.
        for seed in (0..64).chain([453]) {
            for wear in [0., 0.25, 0.5, 0.75, 1.] { assert_valid(&quarried_block(7, wear, seed)); }
        }
    }

    #[test]
    fn same_seed_repeats_distinct_seeds_vary_and_damage_only_removes_stone() {
        let mut shapes = BTreeSet::new();
        // Include seed24, whose independently repaired worn mesh once grew
        // slightly beyond the previous stage despite losing total volume.
        for seed in (0..64).chain([453]) {
            let intact = quarried_block(3, 0., seed);
            let repeated = quarried_block(3, 0., seed);
            assert_eq!(intact.material, 3);
            assert_eq!(intact.indices, repeated.indices);
            assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&intact.vertices), bytemuck::cast_slice::<Vertex, u8>(&repeated.vertices));
            shapes.insert(intact.vertices.iter().map(|v| v.pos.map(f32::to_bits)).collect::<Vec<_>>());
            let mut previous = intact;
            for wear in [0.25, 0.5, 0.75, 1.] {
                let worn = quarried_block(3, wear, seed);
                assert!(volume(&worn) < volume(&previous) - 1e-4);
                for t in previous.indices.chunks_exact(3) {
                    let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(previous.vertices[i as usize].pos));
                    let n = (b - a).cross(c - a).normalize();
                    assert!(worn.vertices.iter().all(|v| n.dot(Vec3::from(v.pos) - a) < 1e-5), "damage expanded the existing stone");
                }
                previous = worn;
            }
        }
        assert_eq!(shapes.len(), 65, "seed variants must have distinct geometry");
    }

    #[test]
    fn invalid_wear_is_bounded_and_material_does_not_change_geometry() {
        let intact = quarried_block(1, 0., 17);
        for wear in [-4., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mesh = quarried_block(99, wear, 17);
            assert_eq!(mesh.material, 99);
            assert_eq!(mesh.indices, intact.indices);
            assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&mesh.vertices), bytemuck::cast_slice::<Vertex, u8>(&intact.vertices));
        }
        let worn = quarried_block(1, 1., 17);
        let capped = quarried_block(1, 4., 17);
        assert_eq!(worn.indices, capped.indices);
        assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&worn.vertices), bytemuck::cast_slice::<Vertex, u8>(&capped.vertices));
    }
}
