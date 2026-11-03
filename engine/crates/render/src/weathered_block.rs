//! Cook a closed masonry mesh from a noise-rounded box field. No runtime
//! raymarching: the resulting triangles use the ordinary cluster LOD/shadows.
//! Broad flat faces remain at the collision box; erosion stays inside it.
use crate::{cluster::Vertex, primitives::Primitive};
use glam::Vec3;
use std::collections::BTreeMap;

fn hash(p: [i32; 3], seed: u32) -> f32 {
    let mut h = seed ^ 0x85eb_ca6b;
    for x in p { h = (h ^ (x as u32).wrapping_mul(0x9e37_79b9)).wrapping_mul(0x7feb_352d); h ^= h >> 16; }
    (h >> 8) as f32 / 16_777_216.
}

fn noise(p: Vec3, seed: u32) -> f32 {
    let cell = p.floor().as_ivec3();
    let f = p - p.floor();
    let w = f * f * (Vec3::splat(3.) - 2. * f);
    let mut v = 0.;
    for z in 0..2 { for y in 0..2 { for x in 0..2 {
        let a = Vec3::new(x as f32, y as f32, z as f32);
        let weight = Vec3::ONE - (a - w).abs();
        v += weight.x * weight.y * weight.z * hash([cell.x + x, cell.y + y, cell.z + z], seed);
    }}}
    v
}

// Radius modulation means this is an implicit field, not an exact distance
// bound for sphere tracing. Bisection below does not assume a distance bound.
fn field(p: Vec3, wear: f32, seed: u32) -> f32 {
    let broad = noise(p * 3.4 + Vec3::splat(0.31), seed);
    let chips = noise(p * 7.1 + Vec3::splat(0.7), seed ^ 0x1234_5678);
    let chip = ((chips - 0.43) / 0.35).clamp(0., 1.);
    let chip = chip * chip * (3. - 2. * chip);
    let r = 0.012 + 0.025 * broad.powi(2) + 0.09 * chip.powi(2)
        + wear * (0.025 + 0.045 * broad);
    let q = p.abs() - Vec3::splat(0.5 - r);
    let mut d = q.max(Vec3::ZERO).length() + q.max_element().min(0.) - r;
    // Four sparse planar corner breaks retain the hard fracture character of
    // stone among the softened edges. Planes deepen with wear and never reseed.
    for cut in 0..4 {
        let n = Vec3::from(std::array::from_fn(|axis| {
            let sign = if hash([cut,axis as i32,21], seed) < 0.5 { -1. } else { 1. };
            sign * (0.6 + 0.8 * hash([cut,axis as i32,22], seed))
        })).normalize();
        let depth = 0.07 + 0.10 * hash([cut,0,23], seed) + wear * 0.04;
        d = d.max(p.dot(n) - n.abs().element_sum() * 0.5 + depth);
    }
    d
}

fn surface(q: Vec3, wear: f32, seed: u32) -> Vec3 {
    // Keep supporting face centres exactly at +/- 0.5. Edge samples move
    // inward on a fixed ray, shared by all damage levels and adjacent faces.
    if field(q, wear, seed) <= 1e-7 { return q; }
    let (mut lo, mut hi) = (0.5, 1.);
    for _ in 0..22 {
        let mid = (lo + hi) * 0.5;
        if field(q * mid, wear, seed) > 0. { hi = mid; } else { lo = mid; }
    }
    q * ((lo + hi) * 0.5)
}

/// Unit block, deterministically weathered for `seed`. Wear increases the
/// rounding without reseeding or moving noise; higher wear only removes mass.
/// A shared indexed cube grid starts at 768 triangles, then undergoes bounded
/// simplification before upload. Samples concentrate
/// near edges; the renderer builds its usual error-based coarser clusters.
/// Texture-scale relief belongs in the material, not this geometry.
pub fn weathered_block(material: u32, wear: f32, seed: u32) -> Primitive {
    let wear = if wear.is_finite() { wear.clamp(0., 1.) } else { 0. };
    const GRID: [i16; 9] = [-500, -465, -350, -180, 0, 180, 350, 465, 500];
    let mut mesh = Primitive { material, ..Default::default() };
    let mut shared = BTreeMap::new();
    for face in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
        let u = (if face.y.abs() > 0.5 { Vec3::X } else { Vec3::Y }).cross(face);
        let v = face.cross(u);
        let mut indices = [[0u32; GRID.len()]; GRID.len()];
        for (j, &b) in GRID.iter().enumerate() { for (i, &a) in GRID.iter().enumerate() {
            let key = (face * 500. + u * a as f32 + v * b as f32).to_array().map(|x| x as i16);
            indices[j][i] = *shared.entry(key).or_insert_with(|| {
                let p = surface(Vec3::from(key.map(|x| x as f32 / 1000.)), wear, seed);
                let eps = 0.0005;
                let normal = Vec3::from([Vec3::X, Vec3::Y, Vec3::Z].map(|axis|
                    field(p + axis * eps, wear, seed) - field(p - axis * eps, wear, seed))).normalize();
                let index = mesh.vertices.len() as u32;
                // ROCK uses object-space triplanar coordinates. No UV seams
                // are needed here; sharing vertices keeps LOD shells closed.
                mesh.vertices.push(Vertex { pos: p.to_array(), normal: normal.to_array(), uv: [0.; 2] });
                index
            });
        }}
        for j in 0..GRID.len() - 1 { for i in 0..GRID.len() - 1 {
            let [a, b, c, d] = [indices[j][i], indices[j][i+1], indices[j+1][i+1], indices[j+1][i]];
            mesh.indices.extend([a, b, c, a, c, d]);
        }}
    }
    // Discard redundant coplanar samples before building the runtime LOD DAG.
    // In particular, near-shadow maps must not rasterize the dense cooking
    // grid just to preserve flat face interiors. Keep ordinary topology rules
    // and a 3 mm error limit in this unit recipe (scaled with the instance).
    let adapter = meshopt::VertexDataAdapter::new(bytemuck::cast_slice(&mesh.vertices),
        std::mem::size_of::<Vertex>(), 0).expect("stone vertex layout");
    let simplified = meshopt::simplify(&mesh.indices, &adapter, 256 * 3, 0.003,
        meshopt::SimplifyOptions::ErrorAbsolute | meshopt::SimplifyOptions::PreserveFolds, None);
    // Tiny slivers at intersecting fracture planes can fold under collapse.
    // Keep the bounded original grid if the candidate loses radial winding.
    if simplified.chunks_exact(3).all(|t| {
        let [a,b,c] = [t[0],t[1],t[2]].map(|i| Vec3::from(mesh.vertices[i as usize].pos));
        (b-a).cross(c-a).dot(a+b+c) > 1e-8
    }) { mesh.indices = simplified; }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_outward_finite_and_damage_only_removes_stone() {
        for seed in 0..32 {
            let mut previous: Option<Primitive> = None;
            for wear in [0., 0.25, 0.5, 0.75, 1.] {
                let mesh = weathered_block(3, wear, seed);
                assert!((100..=768).contains(&(mesh.indices.len() / 3)));
                let mut edges = BTreeMap::new();
                for t in mesh.indices.chunks_exact(3) {
                    let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.vertices[i as usize].pos));
                    assert!((b-a).cross(c-a).dot(a+b+c) > 1e-8);
                    for (a, b) in [(t[0],t[1]), (t[1],t[2]), (t[2],t[0])] {
                        let entry = edges.entry((a.min(b), a.max(b))).or_insert((0, 0));
                        entry.0 += 1; entry.1 += if a < b { 1 } else { -1 };
                    }
                }
                assert!(edges.values().all(|&e| e == (2,0)));
                for (i, vertex) in mesh.vertices.iter().enumerate() {
                    let p = Vec3::from(vertex.pos); let n = Vec3::from(vertex.normal);
                    assert!(p.is_finite() && n.is_finite() && p.abs().max_element() <= 0.5);
                    assert!((n.length()-1.).abs() < 1e-5);
                    assert!(field(p, wear, seed).abs() < 2e-6, "seed={seed} wear={wear} p={p:?} field={}", field(p,wear,seed));
                    if let Some(prev) = &previous {
                        assert!(p.length() <= Vec3::from(prev.vertices[i].pos).length() + 1e-6);
                    }
                }
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    for sign in [-1.,1.] { assert!(mesh.vertices.iter().any(|v| Vec3::from(v.pos) == axis * sign * 0.5)); }
                }
                previous = Some(mesh);
            }
        }
    }

    #[test]
    fn deterministic_bounded_inputs_and_actual_lod() {
        let mesh = weathered_block(4, 0., 17);
        for wear in [0., -1., f32::NAN, f32::INFINITY] {
            let again = weathered_block(8, wear, 17);
            assert_eq!(bytemuck::cast_slice::<Vertex,u8>(&mesh.vertices), bytemuck::cast_slice::<Vertex,u8>(&again.vertices));
            assert_eq!(mesh.indices, again.indices);
        }
        assert_ne!(mesh.vertices[0].pos, weathered_block(4,0.,18).vertices[0].pos);
        let clustered = crate::cluster::ClusterMesh::build_with_policy(&[mesh.part()], |_| crate::cluster::SimplificationPolicy::TexturedGeometry);
        eprintln!("weathered seed 17: {} source triangles, {} root triangles, {} levels", mesh.indices.len()/3, clustered.root_triangles(), clustered.levels);
        assert!(clustered.root_triangles() < mesh.indices.len()/3/2);
        assert!(clustered.levels > 1);
        assert!(clustered.clusters.iter().any(|c| c.own_error > 0.));
    }
}
