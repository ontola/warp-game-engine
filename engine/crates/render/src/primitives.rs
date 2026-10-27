//! Unit primitives for quick scenes and prototypes: a unit cube, a UV sphere
//! of radius 1 and a capsule, centred on the origin, with outward normals and
//! counter-clockwise winding seen from outside. Scale and place them with the
//! instance transform.
//!
//! ```ignore
//! let m = scene.add_material(Material { tint: [0.6; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
//! let cube = scene.add_mesh(&primitives::cube(m).mesh());
//! scene.add_instance(cube, Affine3A::from_scale_rotation_translation(size, rot, centre));
//! ```
use crate::cluster::{ClusterMesh, Part, Vertex};
use glam::Vec3;

/// Owned triangles for one material; `mesh` builds the clustered mesh.
#[derive(Clone, Debug, Default)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: u32,
}

impl Primitive {
    pub fn part(&self) -> Part<'_> { Part { vertices: &self.vertices, indices: &self.indices, material: self.material } }
    pub fn mesh(&self) -> ClusterMesh { ClusterMesh::build(&[self.part()]) }

    /// Make every triangle face away from the origin (convex shapes only).
    fn outward(mut self) -> Self {
        for t in self.indices.chunks_exact_mut(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|k| Vec3::from(self.vertices[k as usize].pos));
            if (b - a).cross(c - a).dot(a + b + c) < 0.0 { t.swap(1, 2); }
        }
        self
    }
}

/// Unit cube (side 1), flat-shaded, uv 0..1 per face.
pub fn cube(material: u32) -> Primitive {
    let mut p = Primitive { material, ..Default::default() };
    for n in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
        let u = if n.y.abs() > 0.5 { Vec3::X } else { Vec3::Y }.cross(n).normalize();
        let w = n.cross(u);
        let base = p.vertices.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let pos = (n + u * a + w * b) * 0.5;
            p.vertices.push(Vertex { pos: pos.to_array(), normal: n.to_array(), uv: [(a + 1.0) * 0.5, (b + 1.0) * 0.5] });
        }
        p.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    p.outward()
}

/// Hand-hewn unit brick (side 1): a cube with `subdiv` quads per face edge,
/// edges rounded by `bevel` (0.09 reads as cut stone) and, when `rough` > 0,
/// the surface pitted inwards by up to `rough` with a pattern from `seed`
/// (worn or cracked stone). Smooth normals across the bevels; seams stay
/// closed because displacement depends on position only.
pub fn hewn_block(material: u32, subdiv: u32, bevel: f32, rough: f32, seed: u32) -> Primitive {
    let n = subdiv.max(1);
    let mut p = Primitive { material, ..Default::default() };
    let inner = 0.5 - bevel.clamp(0.0, 0.5);
    let hash = |q: Vec3| -> f32 {
        let k = (q * 1024.0).round().as_ivec3();
        let mut h = (k.x as u32).wrapping_mul(0x8da6_b343) ^ (k.y as u32).wrapping_mul(0xd816_3841) ^ (k.z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9e37_79b1);
        h ^= h >> 15; h = h.wrapping_mul(0x2c1b_3c6d); h ^= h >> 12;
        (h & 0xffff) as f32 / 65535.0
    };
    for face in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
        let u = if face.y.abs() > 0.5 { Vec3::X } else { Vec3::Y }.cross(face).normalize();
        let w = face.cross(u);
        let base = p.vertices.len() as u32;
        for j in 0..=n {
            for i in 0..=n {
                let (a, b) = (i as f32 / n as f32 * 2.0 - 1.0, j as f32 / n as f32 * 2.0 - 1.0);
                let q = (face + u * a + w * b) * 0.5;
                // Round the corners: clamp to the inner box, push out by the bevel.
                let c = q.clamp(Vec3::splat(-inner), Vec3::splat(inner));
                let mut pos = if (q - c).length() > 1e-6 { c + (q - c).normalize() * (0.5 - inner) } else { q };
                if rough > 0.0 {
                    let dir = (q - c).try_normalize().unwrap_or(face);
                    pos -= dir * rough * hash(q);
                }
                p.vertices.push(Vertex { pos: pos.to_array(), normal: face.to_array(), uv: [(a + 1.0) * 0.5, (b + 1.0) * 0.5] });
            }
        }
        let row = n + 1;
        for j in 0..n {
            for i in 0..n {
                let k = base + j * row + i;
                p.indices.extend([k, k + 1, k + row + 1, k, k + row + 1, k + row]);
            }
        }
    }
    // Normals: area-weighted, welded across faces by position.
    let key = |v: &Vertex| Vec3::from(v.pos).mul_add(Vec3::splat(4096.0), Vec3::ZERO).round().as_ivec3();
    let mut acc: std::collections::HashMap<glam::IVec3, Vec3> = std::collections::HashMap::new();
    let mut p = p.outward();
    for t in p.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|k| Vec3::from(p.vertices[k as usize].pos));
        let nrm = (b - a).cross(c - a);
        for &k in t { *acc.entry(key(&p.vertices[k as usize])).or_default() += nrm; }
    }
    for v in &mut p.vertices { v.normal = acc[&key(v)].normalize_or(Vec3::Y).to_array(); }
    p
}

/// Sphere of radius 1 with `rings` latitude bands and `segments` around.
pub fn sphere(material: u32, rings: u32, segments: u32) -> Primitive {
    capsule(material, 0.0, rings, segments)
}

/// Capsule: radius 1, a straight middle of length `middle` along Y.
pub fn capsule(material: u32, middle: f32, rings: u32, segments: u32) -> Primitive {
    let (rings, segments) = (rings.max(2) & !1, segments.max(3));
    let mut p = Primitive { material, ..Default::default() };
    for r in 0..=rings {
        let th = std::f32::consts::PI * r as f32 / rings as f32;
        let lift = if r <= rings / 2 { middle * 0.5 } else { -middle * 0.5 };
        for s in 0..=segments {
            let ph = std::f32::consts::TAU * s as f32 / segments as f32;
            let n = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
            p.vertices.push(Vertex { pos: (n + Vec3::Y * lift).to_array(), normal: n.to_array(), uv: [s as f32 / segments as f32, r as f32 / rings as f32] });
        }
        // Duplicate the equator ring for the straight part.
        if middle > 0.0 && r == rings / 2 {
            for s in 0..=segments {
                let ph = std::f32::consts::TAU * s as f32 / segments as f32;
                let n = Vec3::new(ph.cos(), 0.0, ph.sin());
                p.vertices.push(Vertex { pos: (n - Vec3::Y * middle * 0.5).to_array(), normal: n.to_array(), uv: [s as f32 / segments as f32, 0.5] });
            }
        }
    }
    let rows = rings + 1 + u32::from(middle > 0.0);
    for r in 0..rows - 1 {
        for s in 0..segments {
            let a = r * (segments + 1) + s;
            let b = a + segments + 1;
            let [pa, pb, pa1, pb1] = [a, b, a + 1, b + 1].map(|k| Vec3::from(p.vertices[k as usize].pos));
            if pa.distance(pa1) > 1e-6 { p.indices.extend([a, b, a + 1]); }
            if pb.distance(pb1) > 1e-6 { p.indices.extend([a + 1, b, b + 1]); }
        }
    }
    p.outward()
}

/// An equirectangular gradient sky for `Scene::sky`: `horizon` rising to
/// `zenith`, `ground` below. Colours are sRGB 0..1 (as picked in a colour
/// tool); the renderer scales the sky to `Lighting::sky_lux`.
///
/// `scene.sky = Some(primitives::gradient_sky([0.25, 0.45, 0.8], [0.75, 0.82, 0.9], [0.35, 0.33, 0.3]));`
pub fn gradient_sky(zenith: [f32; 3], horizon: [f32; 3], ground: [f32; 3]) -> (u32, u32, Vec<[f32; 4]>) {
    let lin = |c: [f32; 3]| Vec3::from(c.map(|v| v.powf(2.2)));
    let (top, hor, low) = (lin(zenith), lin(horizon), lin(ground));
    let (w, h) = (64u32, 32u32);
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let el = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * (y as f32 + 0.5) / h as f32;
        let c = if el > 0.0 { hor.lerp(top, (el / 1.2).min(1.0).powf(0.6)) } else { hor.lerp(low, (-el / 0.3).min(1.0)) };
        for _ in 0..w { px.push([c.x, c.y, c.z, 1.0]); }
    }
    (w, h, px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(p: &Primitive) {
        assert!(p.indices.len() % 3 == 0 && !p.indices.is_empty());
        for t in p.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|k| Vec3::from(p.vertices[k as usize].pos));
            let n = (b - a).cross(c - a);
            assert!(n.length() > 0.0, "degenerate triangle");
            assert!(n.dot(a + b + c) > 0.0, "inward triangle");
        }
    }

    #[test]
    fn primitives_are_closed_outward_and_sized() {
        let c = cube(0);
        check(&c);
        assert_eq!(c.indices.len(), 36);
        assert!(c.vertices.iter().all(|v| v.pos.iter().all(|x| x.abs() == 0.5)));
        let s = sphere(0, 8, 12);
        check(&s);
        assert!(s.vertices.iter().all(|v| (Vec3::from(v.pos).length() - 1.0).abs() < 1e-5));
        let k = capsule(0, 2.0, 8, 12);
        check(&k);
        let top = k.vertices.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!((top - 2.0).abs() < 1e-5, "{top}");
        assert!(!c.mesh().clusters.is_empty());
        for (rough, seed) in [(0.0, 0), (0.12, 3)] {
            let h = hewn_block(0, 4, 0.09, rough, seed);
            check(&h);
            assert_eq!(h.indices.len(), 6 * 16 * 6);
            assert!(h.vertices.iter().all(|v| v.pos.iter().all(|x| x.abs() <= 0.5 + 1e-5)));
            assert!(h.vertices.iter().all(|v| Vec3::from(v.normal).dot(Vec3::from(v.pos)) > 0.0), "normals point out");
            println!("hewn rough {rough}: {} clusters", h.mesh().clusters.len());
        }
    }
}
