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

    /// Wind every triangle to face along its vertex normals (for shapes
    /// that are not convex around the origin, such as a torus).
    fn along_normals(mut self) -> Self {
        for t in self.indices.chunks_exact_mut(3) {
            let v = [t[0], t[1], t[2]].map(|k| self.vertices[k as usize]);
            let [a, b, c] = v.map(|v| Vec3::from(v.pos));
            let n: Vec3 = v.iter().map(|v| Vec3::from(v.normal)).sum();
            if (b - a).cross(c - a).dot(n) < 0.0 { t.swap(1, 2); }
        }
        self
    }
}

/// Cylinder of height 1 along Y (centred), radius `bottom` at y = -0.5 and
/// `top` at y = +0.5, `segments` around; smooth sides, flat caps.
pub fn cylinder(material: u32, top: f32, bottom: f32, segments: u32) -> Primitive {
    let segments = segments.max(3);
    let mut p = Primitive { material, ..Default::default() };
    let slope = (bottom - top).atan2(1.0);
    for s in 0..=segments {
        let ph = std::f32::consts::TAU * s as f32 / segments as f32;
        let (c, sn) = (ph.cos(), ph.sin());
        let n = Vec3::new(c * slope.cos(), slope.sin(), sn * slope.cos());
        for (r, y) in [(bottom, -0.5), (top, 0.5)] {
            p.vertices.push(Vertex { pos: [c * r, y, sn * r], normal: n.to_array(), uv: [s as f32 / segments as f32, y + 0.5] });
        }
    }
    for s in 0..segments {
        let a = s * 2;
        p.indices.extend([a, a + 1, a + 2, a + 1, a + 3, a + 2]);
    }
    for (r, y) in [(bottom, -0.5f32), (top, 0.5)] {
        if r <= 0.0 { continue; }
        let base = p.vertices.len() as u32;
        let n = [0.0, y.signum(), 0.0];
        p.vertices.push(Vertex { pos: [0.0, y, 0.0], normal: n, uv: [0.5, 0.5] });
        for s in 0..=segments {
            let ph = std::f32::consts::TAU * s as f32 / segments as f32;
            p.vertices.push(Vertex { pos: [ph.cos() * r, y, ph.sin() * r], normal: n, uv: [0.5 + ph.cos() * 0.5, 0.5 + ph.sin() * 0.5] });
        }
        for s in 0..segments { p.indices.extend([base, base + 1 + s, base + 2 + s]); }
    }
    p.along_normals()
}

/// Torus around Y: the tube's centre circle has radius 1, the tube radius
/// `tube`; `rings` around the axis, `sides` around the tube.
pub fn torus(material: u32, tube: f32, rings: u32, sides: u32) -> Primitive {
    let (rings, sides) = (rings.max(3), sides.max(3));
    let mut p = Primitive { material, ..Default::default() };
    for r in 0..=rings {
        let a = std::f32::consts::TAU * r as f32 / rings as f32;
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        for s in 0..=sides {
            let b = std::f32::consts::TAU * s as f32 / sides as f32;
            let n = out * b.cos() + Vec3::Y * b.sin();
            p.vertices.push(Vertex { pos: (out + n * tube).to_array(), normal: n.to_array(), uv: [r as f32 / rings as f32, s as f32 / sides as f32] });
        }
    }
    for r in 0..rings {
        for s in 0..sides {
            let a = r * (sides + 1) + s;
            let b = a + sides + 1;
            p.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    p.along_normals()
}

/// One piece of a shattered mesh: the shell triangles nearest one seed,
/// recentred on the piece's centroid, and a cap closing the cut (a fan from
/// the centroid to every open edge) in `cap` material. Both are drawn from
/// both sides so a tumbling piece never shows a hole.
#[derive(Clone, Debug)]
pub struct Shard { pub shell: Primitive, pub cap: Primitive, pub centroid: Vec3, pub half: Vec3 }

impl Shard {
    pub fn mesh(&self) -> ClusterMesh { ClusterMesh::build(&[self.shell.part(), self.cap.part()]) }
}

/// Break `p` into up to `cells` pieces (Growing Guns' Voronoi gibs): seeds
/// on the surface kept apart, each triangle to its nearest seed, each
/// piece capped where it was cut. Deterministic for a `seed`.
pub fn shatter(p: &Primitive, cells: usize, cap: u32, seed: u64) -> Vec<Shard> {
    let tris = p.indices.len() / 3;
    if tris < 2 || cells == 0 { return Vec::new(); }
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
    let pos = |k: u32| Vec3::from(p.vertices[k as usize].pos);
    let centre = |t: usize| (pos(p.indices[t * 3]) + pos(p.indices[t * 3 + 1]) + pos(p.indices[t * 3 + 2])) / 3.0;
    let (lo, hi) = p.vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), v| (a.min(v.pos.into()), b.max(v.pos.into())));
    let min_sep = (hi - lo).max_element() * 0.12;
    let mut seeds: Vec<Vec3> = Vec::new();
    for _ in 0..cells {
        let (mut best, mut best_sep) = (Vec3::ZERO, -1.0f32);
        for _ in 0..4 {
            let c = centre((next() % tris as u64) as usize);
            let sep = seeds.iter().map(|s| s.distance(c)).fold(f32::MAX, f32::min);
            if sep >= min_sep { best = c; break; }
            if sep > best_sep { best_sep = sep; best = c; }
        }
        seeds.push(best);
    }
    let mut buckets = vec![Vec::new(); cells];
    for t in 0..tris {
        let c = centre(t);
        let k = (0..cells).min_by(|&a, &b| seeds[a].distance_squared(c).total_cmp(&seeds[b].distance_squared(c))).unwrap();
        buckets[k].push(t);
    }
    let key = |v: Vec3| (v * 1e4).round().as_ivec3();
    buckets.into_iter().filter(|b| !b.is_empty()).map(|bucket| {
        let mut remap = std::collections::HashMap::new();
        let mut shell = Primitive { material: p.material, ..Default::default() };
        for &t in &bucket {
            for k in &p.indices[t * 3..t * 3 + 3] {
                let n = *remap.entry(*k).or_insert_with(|| { shell.vertices.push(p.vertices[*k as usize]); shell.vertices.len() as u32 - 1 });
                shell.indices.push(n);
            }
        }
        let centroid = shell.vertices.iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / shell.vertices.len() as f32;
        for v in &mut shell.vertices { v.pos = (Vec3::from(v.pos) - centroid).to_array(); }
        // Open edges: seen once, matched by position (primitives repeat
        // vertices along seams).
        let mut edges: std::collections::HashMap<(glam::IVec3, glam::IVec3), (u32, u32, u32)> = std::collections::HashMap::new();
        for t in shell.indices.chunks_exact(3) {
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                let (ka, kb) = (key(shell.vertices[a as usize].pos.into()), key(shell.vertices[b as usize].pos.into()));
                let k = if (ka.x, ka.y, ka.z) <= (kb.x, kb.y, kb.z) { (ka, kb) } else { (kb, ka) };
                edges.entry(k).and_modify(|c| c.2 += 1).or_insert((a, b, 1));
            }
        }
        let mut capp = Primitive { material: cap, ..Default::default() };
        let mut open: Vec<(u32, u32)> = edges.values().filter(|e| e.2 == 1).map(|e| (e.0, e.1)).collect();
        open.sort_unstable();
        for (a, b) in open {
            let (pa, pb) = (Vec3::from(shell.vertices[a as usize].pos), Vec3::from(shell.vertices[b as usize].pos));
            let n = pa.cross(pb).normalize_or(Vec3::Y);
            for (tri, n) in [([Vec3::ZERO, pa, pb], n), ([Vec3::ZERO, pb, pa], -n)] {
                let base = capp.vertices.len() as u32;
                for (i, v) in tri.iter().enumerate() { capp.vertices.push(Vertex { pos: v.to_array(), normal: n.to_array(), uv: [i as f32 * 0.5, 0.0] }); }
                capp.indices.extend([base, base + 1, base + 2]);
            }
        }
        // The shell's inside, for the gaps a fan cannot close.
        let back: Vec<u32> = shell.indices.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
        let flipped = shell.vertices.len() as u32;
        let inner: Vec<Vertex> = shell.vertices.iter().map(|v| Vertex { normal: (-Vec3::from(v.normal)).to_array(), ..*v }).collect();
        shell.vertices.extend(inner);
        shell.indices.extend(back.iter().map(|k| k + flipped));
        let (lo, hi) = shell.vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), v| (a.min(v.pos.into()), b.max(v.pos.into())));
        Shard { shell, cap: capp, centroid, half: ((hi - lo) * 0.5).max(Vec3::splat(0.025)) }
    }).collect()
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

/// Unit quad (side 1) in the XY plane facing +Z, counter-clockwise seen
/// from +Z. uv (0,0) is the top-left corner (+Y up), as a picture or a
/// `screen::Screen` draw list is laid out. One-sided unless the material is
/// TWO_SIDED.
pub fn quad(material: u32) -> Primitive {
    let mut p = Primitive { material, ..Default::default() };
    for (x, y) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
        p.vertices.push(Vertex { pos: [x, y, 0.0], normal: [0.0, 0.0, 1.0], uv: [x + 0.5, 0.5 - y] });
    }
    p.indices.extend([0, 1, 2, 0, 2, 3]);
    p
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

    #[test]
    fn a_shattered_sphere_keeps_every_triangle_and_caps_each_piece() {
        let s = sphere(0, 8, 12);
        let shards = shatter(&s, 3, 1, 7);
        assert!(shards.len() >= 2 && shards.len() <= 3);
        let kept: usize = shards.iter().map(|d| d.shell.indices.len() / 2).sum();
        assert_eq!(kept, s.indices.len());
        assert!(shards.iter().all(|d| !d.cap.indices.is_empty() && d.cap.material == 1 && d.half.max_element() < 1.01));
        let c: Vec3 = shards.iter().map(|d| d.centroid).sum();
        assert!(c.length() < 1.5);
        assert_eq!(shatter(&s, 3, 1, 7)[0].centroid, shards[0].centroid);
    }

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
