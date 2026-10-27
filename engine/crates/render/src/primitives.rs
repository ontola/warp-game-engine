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
    }
}
