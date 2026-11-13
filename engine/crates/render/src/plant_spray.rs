//! Reusable, metre-scale branch sprays made entirely from small solid geometry.
//!
//! A spray is an authoring primitive, not a world instance per needle/leaf. Its
//! wood and foliage share one local frame (root at zero, main shoot along +Y),
//! so callers can place both parts with the same transform and separate bark
//! and foliage materials. The three variants are fixed geometry libraries;
//! species, placement seed, growth, season and wind never change their bytes.
use crate::{cluster::Vertex, foliage_assembly::PartGeometry};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

const VARIANTS: u32 = 3;
// Last byte is the variant; the preceding byte is the geometry revision.
const NEEDLE_WOOD: u64 = 0x5753_4e57_4f44_0300;
const NEEDLE_FOLIAGE: u64 = 0x5753_4e46_4f4c_0300;
const LEAF_WOOD: u64 = 0x5753_4c57_4f44_0100;
const LEAF_FOLIAGE: u64 = 0x5753_4c46_4f4c_0100;

pub(crate) fn unit(variant: u32, channel: u32, index: u32) -> f32 {
    let mut h = (variant as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (channel as u64).wrapping_mul(0xd1b54a32d192ed03)
        ^ (index as u64).wrapping_mul(0x94d049bb133111eb);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((h ^ (h >> 31)) >> 40) as f32 / 16_777_216.0
}

fn empty(id: u64, variant: u32) -> PartGeometry {
    PartGeometry {
        id: id + u64::from(variant),
        vertices: Vec::new(),
        indices: Vec::new(),
    }
}

/// Right-handed cross-section with u cross v along the shoot tangent.
fn frame(tangent: Vec3) -> (Vec3, Vec3) {
    let helper = if tangent.z.abs() < 0.93 {
        Vec3::Z
    } else {
        Vec3::X
    };
    let u = tangent.cross(helper).normalize();
    (u, tangent.cross(u).normalize())
}

/// Append one closed component. Corner-angle weighting keeps the two sides of
/// a thin leaf balanced at its rim; area weighting would bias its rim toward
/// whichever side happens to have the larger triangles. Accumulate in f64 so
/// sub-millimetre sections retain stable normals at metre-scale coordinates.
/// Components never share normals at intersections.
fn append(part: &mut PartGeometry, positions: &[Vec3], triangles: &[[usize; 3]], uv: &[[f32; 2]]) {
    let offset = part.vertices.len() as u32;
    let mut normals = vec![glam::DVec3::ZERO; positions.len()];
    for &[a, b, c] in triangles {
        let [pa, pb, pc] = [a, b, c].map(|i| positions[i].as_dvec3());
        let normal = (pb - pa).cross(pc - pa).normalize();
        for (i, j, k) in [(a, b, c), (b, c, a), (c, a, b)] {
            let e = positions[j].as_dvec3() - positions[i].as_dvec3();
            let f = positions[k].as_dvec3() - positions[i].as_dvec3();
            let angle = e.cross(f).length().atan2(e.dot(f));
            normals[i] += normal * angle;
        }
        part.indices
            .extend([offset + a as u32, offset + b as u32, offset + c as u32]);
    }
    part.vertices.extend(
        positions
            .iter()
            .zip(normals)
            .zip(uv)
            .map(|((&p, n), &uv)| Vertex {
                pos: p.to_array(),
                normal: n.normalize().as_vec3().to_array(),
                uv,
            }),
    );
}

fn connect_rings(triangles: &mut Vec<[usize; 3]>, a: usize, b: usize, sides: usize) {
    for i in 0..sides {
        let j = (i + 1) % sides;
        triangles.push([a + i, a + j, b + j]);
        triangles.push([a + i, b + j, b + i]);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Shoot {
    pub(crate) root: Vec3,
    pub(crate) delta: Vec3,
    pub(crate) bend: Vec3,
    pub(crate) radius: f32,
}
impl Shoot {
    pub(crate) fn point(self, t: f32) -> Vec3 {
        self.root + self.delta * t + self.bend * (4.0 * t * (1.0 - t))
    }
    fn tangent(self, t: f32) -> Vec3 {
        (self.delta + self.bend * (4.0 - 8.0 * t)).normalize()
    }
    fn radius(self, t: f32) -> f32 {
        self.radius * (1.0 - 0.86 * t)
    }
}

pub(crate) fn tube(part: &mut PartGeometry, shoot: Shoot, segments: usize, sides: usize) {
    let mut positions = Vec::new();
    let mut uv = Vec::new();
    let mut triangles = Vec::new();
    for ring in 0..=segments {
        let t = ring as f32 / segments as f32;
        let (u, v) = frame(shoot.tangent(t));
        for side in 0..sides {
            let angle = side as f32 * TAU / sides as f32;
            positions.push(shoot.point(t) + (u * angle.cos() + v * angle.sin()) * shoot.radius(t));
            uv.push([side as f32 / sides as f32, t]);
        }
        if ring > 0 {
            connect_rings(&mut triangles, (ring - 1) * sides, ring * sides, sides);
        }
    }
    for i in 1..sides - 1 {
        triangles.push([0, i + 1, i]);
        let last = segments * sides;
        triangles.push([last, last + i, last + i + 1]);
    }
    append(part, &positions, &triangles, &uv);
}

/// A curved, tapered triangular needle: 10 vertices, 16 outward triangles.
/// Width is the entire cross-section, not a half-width or placement multiplier.
fn needle(
    part: &mut PartGeometry,
    root: Vec3,
    direction: Vec3,
    width_axis: Vec3,
    length: f32,
    width: f32,
    bend: f32,
) {
    let normal = direction.cross(width_axis).normalize();
    let mut positions = Vec::new();
    let mut uv = Vec::new();
    let mut triangles = Vec::new();
    for (ring, (t, breadth)) in [(0.0, 0.30), (0.40, 1.0), (0.77, 0.62)]
        .into_iter()
        .enumerate()
    {
        let centre = root + direction * (length * t) + normal * (bend * t * t);
        // A solid three-dimensional needle, not a thin ribbon. Its full depth
        // is 0.60 times its width, so edge views retain physical silhouette.
        // Different needles never use crossed copies to fake that volume.
        for (x, z) in [(-0.5, -0.25), (0.5, -0.25), (0.0, 0.5)] {
            positions.push(
                centre + width_axis * (x * width * breadth) + normal * (z * width * 0.80 * breadth),
            );
            uv.push([x + 0.5, t]);
        }
        if ring > 0 {
            connect_rings(&mut triangles, (ring - 1) * 3, ring * 3, 3);
        }
    }
    positions.push(root + direction * length + normal * bend);
    uv.push([0.5, 1.0]);
    triangles.push([0, 2, 1]);
    for i in 0..3 {
        triangles.push([9, 6 + i, 6 + (i + 1) % 3]);
    }
    append(part, &positions, &triangles, &uv);
}

/// Closed ovate blade with four curved, six-sided sections and two pointed
/// ends: 26 vertices, 48 triangles. Real centimetre-scale foliage, not a card.
pub(crate) fn leaf(
    part: &mut PartGeometry,
    root: Vec3,
    direction: Vec3,
    across: Vec3,
    length: f32,
    width: f32,
    curl: f32,
) {
    let normal = direction.cross(across).normalize();
    let centre = |t: f32| {
        root + direction * (length * t)
            + normal * (curl * t * t)
            + across * (length * 0.018 * (PI * t).sin())
    };
    let mut positions = vec![root];
    let mut uv = vec![[0.5, 0.0]];
    let mut triangles = Vec::new();
    for (ring, (t, breadth)) in [(0.16, 0.48), (0.39, 1.0), (0.65, 0.88), (0.85, 0.47)]
        .into_iter()
        .enumerate()
    {
        for side in 0..6 {
            let angle = side as f32 * TAU / 6.0;
            let x = angle.cos() * breadth * 0.5;
            positions.push(
                centre(t) + across * (x * width) + normal * (angle.sin() * 0.00065 * breadth),
            );
            uv.push([x + 0.5, t]);
        }
        if ring > 0 {
            connect_rings(&mut triangles, 1 + (ring - 1) * 6, 1 + ring * 6, 6);
        }
    }
    let tip = positions.len();
    positions.push(centre(1.0));
    uv.push([0.5, 1.0]);
    for i in 0..6 {
        triangles.push([0, 1 + (i + 1) % 6, 1 + i]);
        triangles.push([tip, 19 + i, 19 + (i + 1) % 6]);
    }
    append(part, &positions, &triangles, &uv);
}

fn skeleton(variant: u32) -> (Shoot, Vec<Shoot>) {
    let main = Shoot {
        root: Vec3::ZERO,
        delta: Vec3::new(
            -0.025 + 0.025 * variant as f32,
            0.97 + 0.025 * variant as f32,
            0.025,
        ),
        bend: Vec3::new(0.022 * (variant as f32 - 1.0), 0.0, 0.024),
        radius: 0.0055,
    };
    let count = 8 + variant;
    let mut laterals = Vec::new();
    for i in 0..count {
        // Alternate insertion heights; do not make a ring of identical spokes.
        let t = 0.13 + 0.75 * (i as f32 + 0.20 + 0.60 * unit(variant, 1, i)) / count as f32;
        let side = if i % 2 == 0 { -1.0 } else { 1.0 };
        let depth = (unit(variant, 2, i) - 0.5) * 0.95;
        let direction = Vec3::new(side, 0.36 + 0.46 * unit(variant, 3, i), depth).normalize();
        let length = (0.31 + 0.075 * unit(variant, 4, i)) * (1.0 - 0.48 * t);
        laterals.push(Shoot {
            root: main.point(t),
            delta: direction * length,
            bend: Vec3::new(side * 0.010, -0.017, (unit(variant, 5, i) - 0.5) * 0.035),
            radius: 0.0018 + 0.00065 * (1.0 - t),
        });
    }
    (main, laterals)
}

fn needles_on(part: &mut PartGeometry, shoot: Shoot, variant: u32, twig: u32, count: u32) {
    for i in 0..count {
        let index = twig * 64 + i;
        let t = 0.10 + 0.87 * (i as f32 + 0.15 + 0.70 * unit(variant, 10, index)) / count as f32;
        let tangent = shoot.tangent(t);
        let (u, v) = frame(tangent);
        // Mostly comb-like needle ranks, with staggered bases and local roll.
        // Each needle points away from its own shoot instead of crossing it.
        let angle = if i % 2 == 0 { 0.0 } else { PI }
            + (unit(variant, 11, index) - 0.5) * 1.05
            + t * 0.35
            + (unit(variant, 17, twig) - 0.5) * 0.70;
        let outward = u * angle.cos() + v * angle.sin();
        let direction =
            (outward * 0.88 + tangent * (0.30 + 0.55 * unit(variant, 12, index))).normalize();
        // The broad face follows the local twig fan. Using tangent cross
        // direction here would turn every flattened needle edge-on to it.
        let flat = (tangent - direction * tangent.dot(direction)).normalize();
        // Needles roll independently along the twig: the spray should retain
        // directional area when its main branch fan is viewed edge-on.
        let roll = (unit(variant, 16, index) - 0.5) * 2.40;
        let across = flat * roll.cos() + direction.cross(flat) * roll.sin();
        let root = shoot.point(t) + outward * shoot.radius(t);
        needle(
            part,
            root,
            direction,
            across,
            0.039 + 0.010 * unit(variant, 13, index),
            0.0024 + 0.0006 * unit(variant, 14, index),
            (unit(variant, 15, index) - 0.25) * 0.004,
        );
    }
}

/// Short current-growth shoots spring from the older lateral's actual curve.
/// Their asymmetric insertion, forward growth and independent depth/curl make
/// a branched spray rather than adding coincident copies to a two-row comb.
fn needle_branchlets(variant: u32, twig: u32, parent: Shoot) -> Vec<(f32, Shoot)> {
    (0..3)
        .map(|shoot| {
            let index = twig * 3 + shoot;
            let t = 0.16 + 0.71 * (shoot as f32 + 0.18 + 0.62 * unit(variant, 50, index)) / 3.0;
            let tangent = parent.tangent(t);
            let (u, v) = frame(tangent);
            // A branchlet can grow on the same side as its predecessor; insertion
            // heights and depth differ, so no identical opposite pairs are made.
            let side = if unit(variant, 51, index) < 0.48 {
                -1.0
            } else {
                1.0
            };
            let depth = (unit(variant, 52, index) - 0.5) * 0.95;
            let direction =
                (tangent * (0.52 + 0.28 * unit(variant, 53, index)) + u * side * 0.78 + v * depth)
                    .normalize();
            let length = 0.065 + 0.065 * unit(variant, 54, index);
            let branchlet = Shoot {
                root: parent.point(t),
                delta: direction * length,
                bend: u * side * (0.003 + 0.004 * unit(variant, 55, index))
                    + v * ((unit(variant, 56, index) - 0.5) * 0.009),
                radius: 0.00065 + 0.00015 * unit(variant, 57, index),
            };
            (t, branchlet)
        })
        .collect()
}

/// Fixed variant 0..2 (other indices wrap). Returns `(wood, foliage)` in the
/// same metre-scale frame. Main shoot is about 1 m; needles are 3.9–4.9 cm long
/// and 2.4–3.0 mm wide with 1.44–1.8 mm depth. Independently rolled solid
/// sections retain directional area. Tertiary shoots carry most of the 600–800
/// needles (about 10–14k triangles including wood). One paired placement creates
/// the complete spray; no additional world instances or poses per branchlet.
pub fn needle_spray(variant: u32) -> (PartGeometry, PartGeometry) {
    let variant = variant % VARIANTS;
    let mut wood = empty(NEEDLE_WOOD, variant);
    let mut foliage = empty(NEEDLE_FOLIAGE, variant);
    let (main, laterals) = skeleton(variant);
    tube(&mut wood, main, 8, 6);
    for (i, &shoot) in laterals.iter().enumerate() {
        tube(&mut wood, shoot, 4, 5);
        let count = 12 + (unit(variant, 20, i as u32) * 5.0) as u32;
        needles_on(&mut foliage, shoot, variant, i as u32, count);
        for (j, (_, branchlet)) in needle_branchlets(variant, i as u32, shoot)
            .into_iter()
            .enumerate()
        {
            tube(&mut wood, branchlet, 2, 4);
            let index = i as u32 * 3 + j as u32;
            let count = 20 - variant / 2 + (unit(variant, 58, index) * 3.0) as u32;
            // Separate channels from lateral and terminal needle placement.
            needles_on(&mut foliage, branchlet, variant, 64 + index, count);
        }
    }
    // The terminal shoot carries needles too; avoid a bare antenna at the top.
    let terminal = Shoot {
        root: main.point(0.72),
        delta: main.point(1.0) - main.point(0.72),
        bend: main.bend * 0.08,
        radius: main.radius(0.72),
    };
    needles_on(&mut foliage, terminal, variant, 32, 52);
    (wood, foliage)
}

fn leaves_on(
    wood: &mut PartGeometry,
    foliage: &mut PartGeometry,
    shoot: Shoot,
    variant: u32,
    twig: u32,
    count: u32,
) {
    for i in 0..count {
        let index = twig * 16 + i;
        let t = 0.16 + 0.81 * (i as f32 + 0.20 + 0.55 * unit(variant, 30, index)) / count as f32;
        let tangent = shoot.tangent(t);
        let (u, v) = frame(tangent);
        let angle = if i % 2 == 0 { 0.0 } else { PI } + (unit(variant, 31, index) - 0.5) * 0.80;
        let outward = u * angle.cos() + v * angle.sin();
        let direction = (outward * 0.76 + tangent * 0.65).normalize();
        let petiole_root = shoot.point(t) + outward * shoot.radius(t);
        let petiole_length = 0.012 + 0.009 * unit(variant, 32, index);
        let petiole = Shoot {
            root: petiole_root,
            delta: direction * petiole_length,
            bend: v * 0.001,
            radius: 0.00065,
        };
        tube(wood, petiole, 1, 3);
        let (a, b) = frame(direction);
        let roll = (unit(variant, 33, index) - 0.5) * 1.20;
        let across = a * roll.cos() + b * roll.sin();
        leaf(
            foliage,
            petiole.point(1.0),
            direction,
            across,
            0.067 + 0.049 * unit(variant, 34, index),
            0.033 + 0.025 * unit(variant, 35, index),
            (0.045 + 0.065 * unit(variant, 36, index)) * 0.09,
        );
    }
}

/// Fixed variant 0..2 (other indices wrap). Returns `(wood, foliage)` using
/// separate material slots at the caller. Ovate blades are 6.7–11.6 cm long,
/// 3.3–5.8 cm wide; stems and petioles remain visible between individual leaves.
pub fn broadleaf_spray(variant: u32) -> (PartGeometry, PartGeometry) {
    let variant = variant % VARIANTS;
    let mut wood = empty(LEAF_WOOD, variant);
    let mut foliage = empty(LEAF_FOLIAGE, variant);
    let (main, laterals) = skeleton(variant);
    tube(&mut wood, main, 8, 6);
    for (i, &shoot) in laterals.iter().enumerate() {
        tube(&mut wood, shoot, 4, 5);
        let count = 4 + u32::from(unit(variant, 40, i as u32) > 0.60);
        leaves_on(&mut wood, &mut foliage, shoot, variant, i as u32, count);
    }
    let terminal = Shoot {
        root: main.point(0.80),
        delta: main.point(1.0) - main.point(0.80),
        bend: main.bend * 0.04,
        radius: main.radius(0.80),
    };
    leaves_on(&mut wood, &mut foliage, terminal, variant, 32, 4);
    (wood, foliage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn same(a: &PartGeometry, b: &PartGeometry) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            assert_eq!(a.pos.map(f32::to_bits), b.pos.map(f32::to_bits));
            assert_eq!(a.normal.map(f32::to_bits), b.normal.map(f32::to_bits));
            assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
        }
    }

    fn check_solid(part: &PartGeometry) {
        let mut edges = BTreeMap::<(u32, u32), (u32, i32)>::new();
        let mut parents: Vec<_> = (0..part.vertices.len()).collect();
        fn component(parents: &mut [usize], mut i: usize) -> usize {
            while parents[i] != i {
                parents[i] = parents[parents[i]];
                i = parents[i];
            }
            i
        }
        for tri in part.indices.chunks_exact(3) {
            let a = component(&mut parents, tri[0] as usize);
            for &i in &tri[1..] {
                let b = component(&mut parents, i as usize);
                parents[b] = a;
            }
        }
        let mut volumes = BTreeMap::<usize, f64>::new();
        for v in &part.vertices {
            assert!(
                v.pos
                    .iter()
                    .chain(&v.normal)
                    .chain(&v.uv)
                    .all(|v| v.is_finite())
            );
            assert!((Vec3::from(v.normal).length() - 1.0).abs() < 2e-5);
        }
        for tri in part.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| {
                let p = part.vertices[i as usize].pos;
                glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
            });
            let cross = (b - a).cross(c - a);
            assert!(cross.length() > 1e-13, "zero-area source triangle");
            let root = component(&mut parents, tri[0] as usize);
            let origin = Vec3::from(part.vertices[root].pos).as_dvec3();
            *volumes.entry(root).or_default() +=
                (a - origin).dot((b - origin).cross(c - origin)) / 6.0;
            let n = tri
                .iter()
                .map(|&i| Vec3::from(part.vertices[i as usize].normal))
                .sum::<Vec3>();
            assert!(
                cross.dot(n.as_dvec3()) > 0.0,
                "normal opposes outward winding"
            );
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let edge = edges.entry((a.min(b), a.max(b))).or_default();
                edge.0 += 1;
                edge.1 += if a < b { 1 } else { -1 };
            }
        }
        assert!(
            volumes.values().all(|&volume| volume > 0.0),
            "inward source solid"
        );
        assert!(
            edges
                .values()
                .all(|&(count, direction)| count == 2 && direction == 0),
            "each component must be closed with consistently oriented edges"
        );
    }

    #[test]
    fn grown_botanical_parts_are_closed_solids_with_distinct_stable_ids() {
        use crate::plant_species::grown;
        let mut ids = BTreeSet::new();
        for name in ["woolly_willow", "white_broom", "forest_dracaena", "forest_coffee", "african_mahogany", "kapok", "raffia_palm"] {
            let model = grown(name).unwrap();
            let parts = model.parts();
            for (part, repeat) in parts.tufts.iter().zip(model.parts().tufts) {
                check_solid(part);
                same(part, &repeat);
                assert!(ids.insert(part.id), "{name} reuses another shoot");
            }
        }
    }

    #[test]
    fn sprays_are_closed_well_shaded_small_solids() {
        for variant in 0..3 {
            for (wood, foliage) in [needle_spray(variant), broadleaf_spray(variant)] {
                check_solid(&wood);
                check_solid(&foliage);
                let triangles = (wood.indices.len() + foliage.indices.len()) / 3;
                let needle = foliage.id & !0xff == NEEDLE_FOLIAGE;
                let budget = if needle { 10_000..=14_000 } else { 1500..=4000 };
                assert!(budget.contains(&triangles), "triangles: {triangles}");
                let bytes = (wood.vertices.len() + foliage.vertices.len())
                    * std::mem::size_of::<Vertex>()
                    + (wood.indices.len() + foliage.indices.len()) * 4;
                assert!(bytes < if needle { 480 * 1024 } else { 160 * 1024 });
                let positions = wood
                    .vertices
                    .iter()
                    .chain(&foliage.vertices)
                    .map(|v| Vec3::from(v.pos));
                let (lo, hi) = positions.fold(
                    (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                    |(lo, hi), p| (lo.min(p), hi.max(p)),
                );
                assert!(lo.y > -0.01 && hi.y > 0.95 && hi.y < 1.15);
                assert!(hi.x - lo.x > 0.30 && hi.x - lo.x < if needle { 1.05 } else { 0.85 });
                assert!(hi.z - lo.z > 0.08 && hi.z - lo.z < 0.60);
            }
        }
    }

    #[test]
    fn needle_branchlets_start_on_parent_and_add_real_coverage() {
        for variant in 0..3 {
            let (_, laterals) = skeleton(variant);
            for (twig, parent) in laterals.into_iter().enumerate() {
                let branchlets = needle_branchlets(variant, twig as u32, parent);
                assert_eq!(branchlets.len(), 3);
                let mut previous = 0.0;
                for (t, shoot) in branchlets {
                    assert!(t > previous && t < 1.0);
                    previous = t;
                    assert_eq!(shoot.root, parent.point(t));
                    assert!((0.064..0.131).contains(&shoot.delta.length()));
                    assert!(shoot.delta.normalize().dot(parent.tangent(t)) > 0.30);
                }
            }
            let (_, foliage) = needle_spray(variant);
            let needles = foliage.vertices.len() / 10;
            assert!((600..=800).contains(&needles), "needle count: {needles}");
            // Sum the broad face's actual tapered-width strips before overlap
            // or world orientation. This is source area, not screen coverage.
            let area = foliage
                .vertices
                .chunks_exact(10)
                .map(|needle| {
                    let centre = |i: usize| {
                        needle[i..i + 3]
                            .iter()
                            .map(|v| Vec3::from(v.pos))
                            .sum::<Vec3>()
                            / 3.0
                    };
                    let width = |i: usize| {
                        Vec3::from(needle[i].pos).distance(Vec3::from(needle[i + 1].pos))
                    };
                    let p = [centre(0), centre(3), centre(6), Vec3::from(needle[9].pos)];
                    let w = [width(0), width(3), width(6), 0.0];
                    (0..3)
                        .map(|i| p[i + 1].distance(p[i]) * (w[i] + w[i + 1]) * 0.5)
                        .sum::<f32>()
                })
                .sum::<f32>();
            assert!((0.050..0.065).contains(&area), "needle face area: {area}");
            // Half the closed surface's absolute projected area counts each
            // convex needle once per direction. It is a sum over needles, not
            // their projected union, but detects an almost edge-on ribbon fan.
            let projected = foliage
                .indices
                .chunks_exact(3)
                .map(|tri| {
                    let [a, b, c] = [tri[0], tri[1], tri[2]]
                        .map(|i| Vec3::from(foliage.vertices[i as usize].pos).as_dvec3());
                    (b - a).cross(c - a).abs() * 0.25
                })
                .sum::<glam::DVec3>();
            assert!(
                projected.min_element() > 0.02,
                "projected needle areas: {projected}"
            );
            assert!(
                projected.min_element() / projected.max_element() > 0.55,
                "needle fan remains excessively directional: {projected}"
            );
        }
    }

    #[test]
    fn variants_are_stable_shared_geometry_not_placement_randomness() {
        let mut ids = BTreeSet::new();
        let mut first_vertices = Vec::new();
        for variant in 0..3 {
            for (build, pair) in [
                (
                    needle_spray as fn(u32) -> (PartGeometry, PartGeometry),
                    needle_spray(variant),
                ),
                (
                    broadleaf_spray as fn(u32) -> (PartGeometry, PartGeometry),
                    broadleaf_spray(variant),
                ),
            ] {
                let repeated = build(variant + 3);
                same(&pair.0, &repeated.0);
                same(&pair.1, &repeated.1);
                assert!(ids.insert(pair.0.id) && ids.insert(pair.1.id));
                first_vertices.push(pair.1.vertices[0].pos.map(f32::to_bits));
            }
        }
        assert_eq!(ids.len(), 12);
        assert_eq!(first_vertices.iter().collect::<BTreeSet<_>>().len(), 6);
    }

    #[test]
    fn foliage_elements_have_real_leaf_and_needle_dimensions() {
        for variant in 0..3 {
            let (_, needles) = needle_spray(variant);
            assert_eq!(needles.vertices.len() % 10, 0);
            for element in needles.vertices.chunks_exact(10) {
                let root = element[..3].iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / 3.0;
                let length = Vec3::from(element[9].pos).distance(root);
                let width = Vec3::from(element[3].pos).distance(Vec3::from(element[4].pos));
                let side_midpoint = (Vec3::from(element[3].pos) + Vec3::from(element[4].pos)) * 0.5;
                let depth = Vec3::from(element[5].pos).distance(side_midpoint);
                assert!((0.038..0.050).contains(&length));
                assert!((0.0023..0.0031).contains(&width));
                assert!(
                    (0.59..0.61).contains(&(depth / width)),
                    "solid needle depth: {depth}"
                );
            }
            let (_, leaves) = broadleaf_spray(variant);
            assert_eq!(leaves.vertices.len() % 26, 0);
            for element in leaves.vertices.chunks_exact(26) {
                let length = Vec3::from(element[0].pos).distance(Vec3::from(element[25].pos));
                let width = Vec3::from(element[7].pos).distance(Vec3::from(element[10].pos));
                assert!((0.066..0.118).contains(&length));
                assert!((0.032..0.059).contains(&width));
            }
        }
    }
}

#[cfg(test)]
mod coarse_first {
    use crate::cluster::{ClusterMesh, Part, SimplificationPolicy};

    #[test]
    fn sorted_spray_cuts_lie_in_their_coarse_prefix() {
        let (_, foliage) = super::needle_spray(0);
        let mut mesh = ClusterMesh::build_with_policy(&[Part { vertices: &foliage.vertices, indices: &foliage.indices, material: 0 }], |_| SimplificationPolicy::ThinFoliage);
        let cut = |m: &ClusterMesh, t: f32| -> Vec<(u32, u32)> {
            let mut c: Vec<_> = m.clusters.iter().filter(|c| c.own_error <= t && c.parent_error > t).map(|c| (c.first_tri, c.tri_count)).collect();
            c.sort_unstable(); c
        };
        let thresholds = [0.0f32, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 1e29];
        let before: Vec<_> = thresholds.iter().map(|&t| cut(&mesh, t)).collect();
        mesh.sort_coarse_first();
        for (&t, expected) in thresholds.iter().zip(&before) {
            // The same cut, and all of it within the prefix the cull visits.
            assert_eq!(&cut(&mesh, t), expected);
            let prefix = mesh.coarse_prefix(t);
            assert!(mesh.clusters[prefix..].iter().all(|c| !(c.own_error <= t && c.parent_error > t)));
        }
        // Far away the cull visits a fraction of the 244 clusters (16 drawn at 5 cm).
        assert_eq!(mesh.coarse_prefix(0.0), mesh.clusters.len());
        assert!(mesh.coarse_prefix(0.05) * 4 < mesh.clusters.len(), "{} of {}", mesh.coarse_prefix(0.05), mesh.clusters.len());
    }
}
