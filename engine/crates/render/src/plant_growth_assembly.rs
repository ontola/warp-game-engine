//! A grown plant as an Assembly: a few reusable parts (stems of three sizes,
//! needle tufts) instanced where growth put them, on a bone per main limb. Age
//! only changes counts and placement, so better part meshes drop in through
//! `Parts` and every age picks them up. Wind moves the bones and parts follow
//! rigidly; virtual members expand the same parts on the GPU.
use crate::{
    cluster::Vertex,
    foliage_assembly::{Archetype, Assembly, BranchGroup, Limits, Parameters, PartGeometry, PartInstance},
    plant_growth::Plant,
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::TAU;

/// The parts a plant is assembled from. Stems are unit cylinders along +Y
/// (radius 1, height 1); a tuft grows from its origin along +Y.
#[derive(Clone, Debug)]
pub struct Parts {
    /// Stems that narrow to 1.0, 0.94, 0.86 and 0.72 of their base radius,
    /// eight-sided then sixteen-sided: a run picks the taper nearest its own,
    /// so consecutive runs meet at nearly the same ring.
    pub stems: [PartGeometry; 8],
    /// A unit ball for joints and branch collars.
    pub collar: PartGeometry,
    pub tufts: Vec<PartGeometry>,
}

const TAPERS: [f32; 4] = [1.0, 0.94, 0.86, 0.72];

fn stem(id: u64, sides: u32, taper: f32) -> PartGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    // Closed (capped) and with the seam on exactly the first ring's positions:
    // foliage optics need every edge shared by two faces.
    let ring = |i: u32| {
        let a = (i % sides) as f32 / sides as f32 * TAU;
        (a.cos(), a.sin())
    };
    for i in 0..=sides {
        let (c, s) = ring(i);
        for y in [0.0, 1.0] {
            let r = 1.0 + (taper - 1.0) * y;
            let slope = 1.0 - taper;
            let n = Vec3::new(c, slope, s).normalize();
            vertices.push(Vertex { pos: [c * r, y, s * r], normal: n.to_array(), uv: [i as f32 / sides as f32, y] });
        }
    }
    for i in 0..sides {
        let b = i * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    for (y, normal) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
        let centre = vertices.len() as u32;
        vertices.push(Vertex { pos: [0.0, y, 0.0], normal: [0.0, normal, 0.0], uv: [0.5, y] });
        let first = vertices.len() as u32;
        for i in 0..sides {
            let (c, s) = ring(i);
            let r = 1.0 + (taper - 1.0) * y;
            vertices.push(Vertex { pos: [c * r, y, s * r], normal: [0.0, normal, 0.0], uv: [0.5 + 0.5 * c, 0.5 + 0.5 * s] });
        }
        for i in 0..sides {
            let (a, b) = (first + i, first + (i + 1) % sides);
            indices.extend_from_slice(&if normal < 0.0 { [centre, a, b] } else { [centre, b, a] });
        }
    }
    PartGeometry { id, vertices, indices }
}

/// A closed unit ball: knuckles where stem runs meet and, stretched, the\n/// collar where a branch leaves its parent.
fn collar(id: u64) -> PartGeometry {
    let (rings, sides) = (8u32, 16u32);
    let mut vertices = vec![Vertex { pos: [0.0, -1.0, 0.0], normal: [0.0, -1.0, 0.0], uv: [0.5, 0.0] }];
    for r in 1..rings {
        let a = r as f32 / rings as f32 * std::f32::consts::PI;
        for i in 0..sides {
            let b = i as f32 / sides as f32 * TAU;
            let n = Vec3::new(a.sin() * b.cos(), -a.cos(), a.sin() * b.sin());
            vertices.push(Vertex { pos: n.to_array(), normal: n.to_array(), uv: [i as f32 / sides as f32, r as f32 / rings as f32] });
        }
    }
    vertices.push(Vertex { pos: [0.0, 1.0, 0.0], normal: [0.0, 1.0, 0.0], uv: [0.5, 1.0] });
    let top = vertices.len() as u32 - 1;
    let at = |r: u32, i: u32| 1 + (r - 1) * sides + i % sides;
    let mut indices = Vec::new();
    for i in 0..sides {
        indices.extend_from_slice(&[0, at(1, i), at(1, i + 1)]);
        indices.extend_from_slice(&[top, at(rings - 1, i + 1), at(rings - 1, i)]);
        for r in 1..rings - 1 {
            indices.extend_from_slice(&[at(r, i), at(r + 1, i), at(r + 1, i + 1), at(r, i), at(r + 1, i + 1), at(r, i + 1)]);
        }
    }
    PartGeometry { id, vertices, indices }
}

fn unit(variant: u64, index: u64, tag: u64) -> f32 {
    let mut x = variant.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ tag.wrapping_mul(0x94d0_49bb_1331_11eb);
    x = (x ^ (x >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    ((x ^ (x >> 32)) >> 40) as f32 / (1u64 << 24) as f32
}

/// A bottlebrush of long needles along the last 30 cm of a shoot (+Y, ending
/// at the origin), splaying out and forward; a stand-in until a proper mesh
/// replaces it.
fn tuft(id: u64, variant: u64, needles: u32, length: f32) -> PartGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for i in 0..needles {
        let v = i as u64;
        // Fascicles along the shoot, denser and more upright near the tip.
        let along = -0.3 * unit(variant, v, 5).powf(1.4);
        let tilt = 0.45 + 0.75 * (-along / 0.3) + 0.25 * unit(variant, v, 1);
        let around = unit(variant, v, 2) * TAU;
        let axis = Vec3::new(tilt.sin() * around.cos(), tilt.cos(), tilt.sin() * around.sin());
        let side = axis.cross(if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize();
        let up = axis.cross(side).normalize();
        let root = Vec3::Y * along + axis * 0.006;
        let long = length * (0.7 + 0.5 * unit(variant, v, 3));
        let tip = axis * long - Vec3::Y * 0.12 * long * unit(variant, v, 4);
        // A closed sliver (three-sided base, one tip): foliage optics need
        // every edge shared by two faces.
        let b = vertices.len() as u32;
        for k in 0..3 {
            let a = k as f32 / 3.0 * TAU;
            let out = side * a.cos() + up * a.sin();
            vertices.push(Vertex { pos: (root + out * 0.0025).to_array(), normal: out.to_array(), uv: [k as f32 / 3.0, 0.0] });
        }
        vertices.push(Vertex { pos: tip.to_array(), normal: (tip - root).normalize().to_array(), uv: [0.5, 1.0] });
        indices.extend_from_slice(&[b, b + 2, b + 1, b, b + 1, b + 3, b + 1, b + 2, b + 3, b + 2, b, b + 3]);
    }
    PartGeometry { id, vertices, indices }
}

impl Parts {
    /// Placeholder parts for a long-needled pine.
    pub fn long_needle_pine() -> Self {
        let look = Look::jeffrey_pine();
        Self::needle_brushes(look.brush_needles, look.needle_length)
    }

    /// Pine parts whose bottlebrushes carry `needles` needles about `length`
    /// metres long (Look::brush_needles and needle_length).
    pub fn needle_brushes(needles: u32, length: f32) -> Self {
        const STEM: u64 = 0x5047_5354_0000_0000;
        const TUFT: u64 = 0x5047_5446_0000_0000;
        let (needles, length) = (needles.clamp(1, 2000), length.clamp(0.01, 1.0));
        // The brush's needles and length are part of its identity (and so of every cook key).
        let tuft_id = TUFT ^ (u64::from(needles) << 8) ^ (u64::from(length.to_bits()) << 20);
        Self {
            stems: std::array::from_fn(|k| stem(STEM + 1 + k as u64, if k < 4 { 8 } else { 16 }, TAPERS[k % 4])),
            collar: collar(STEM + 16),
            tufts: (0..3).map(|v| tuft(tuft_id + v, v, needles, length)).collect(),
        }
    }
}

/// Limits for a grown plant: the recipe limits with room for an old pine's
/// ~5k stems and ~10k needle brushes.
pub fn limits() -> Limits {
    Limits { max_instances: 24576, ..crate::plant_recipe::recipe_limits() }
}

/// Colours of the wood and the needles, and the needle brushes
/// (Parts::needle_brushes takes the brush fields).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Look {
    pub wood: [f32; 3],
    pub needles: [f32; 3],
    /// Needles in one bottlebrush.
    pub brush_needles: u32,
    /// Typical needle length, metres.
    pub needle_length: f32,
}

impl Default for Look {
    fn default() -> Self { Self::jeffrey_pine() }
}

impl Look {
    pub fn jeffrey_pine() -> Self {
        Self { wood: [0.24, 0.12, 0.07], needles: [0.11, 0.21, 0.09], brush_needles: 140, needle_length: 0.22 }
    }
}

/// What an assembled plant came to.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub bones: usize,
    pub stems: usize,
    pub tufts: usize,
    /// Tufts left out to stay inside the placement limit.
    pub tufts_dropped: usize,
}

/// A product of affine matrices with its last row exact again.
fn affine(mut m: Mat4) -> Mat4 {
    m.x_axis.w = 0.0;
    m.y_axis.w = 0.0;
    m.z_axis.w = 0.0;
    m.w_axis.w = 1.0;
    m
}

fn along(from: Vec3, to: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Y, (to - from).normalize_or(Vec3::Y))
}

/// Long stem runs from year segments: consecutive shoots of one axis merge
/// until `reach` metres, so a tree is tens of trunk pieces, not one a year.
fn runs(plant: &Plant, reach: f32) -> Vec<(u32, Vec3, Vec3, f32, f32, bool)> {
    let n = plant.segments.len();
    let mut next = vec![None; n];
    for (i, s) in plant.segments.iter().enumerate() {
        if let Some(p) = s.parent {
            if plant.segments[p as usize].order == s.order {
                next[p as usize] = Some(i);
            }
        }
    }
    let mut out = Vec::new();
    for (i, s) in plant.segments.iter().enumerate() {
        // Start at axis starts only; the walk covers the rest of the axis.
        if s.parent.is_some_and(|p| plant.segments[p as usize].order == s.order) {
            continue;
        }
        let (mut start, mut radius, mut last, mut at) = (s.from, s.radius[0], i, i);
        while let Some(n) = next[at] {
            at = n;
            if plant.segments[at].to.distance(start) >= reach {
                out.push((last as u32, start, plant.segments[at].to, radius, plant.segments[at].radius[1], last == i));
                start = plant.segments[at].to;
                radius = plant.segments[at].radius[0];
                last = at;
            }
        }
        let end = plant.segments[at].to;
        if end.distance(start) > 1e-3 {
            out.push((last as u32, start, end, radius, plant.segments[at].radius[1], last == i));
        }
    }
    out
}

/// Build `plant` from `parts`. Main limbs with the most foliage get bones (up
/// to `limits.max_branches`); everything else rides the trunk's. Twig runs grow
/// and tufts thin only as far as `limits.max_instances` demands.
pub fn assemble(plant: &Plant, parts: &Parts, look: Look, seed: u64, limits: Limits) -> Result<(Assembly, Counts), String> {
    let n = plant.segments.len();
    // The limb each segment belongs to: the first order-1 segment on its way up.
    let mut limb = vec![u32::MAX; n];
    for (i, s) in plant.segments.iter().enumerate() {
        limb[i] = match s.parent {
            _ if s.order == 0 => u32::MAX,
            Some(p) if plant.segments[p as usize].order == 0 => i as u32,
            Some(p) => limb[p as usize],
            None => u32::MAX,
        };
    }
    let mut foliage = std::collections::HashMap::<u32, u32>::new();
    for t in &plant.tufts {
        let l = limb[t.segment as usize];
        if l != u32::MAX {
            *foliage.entry(l).or_default() += 1;
        }
    }
    let mut ranked: Vec<(u32, u32)> = foliage.into_iter().map(|(l, c)| (c, l)).collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    ranked.truncate(limits.max_branches.saturating_sub(1));
    ranked.sort_unstable_by_key(|r| r.1);
    let mut branches = vec![BranchGroup { id: seed ^ 0x4252_414e_4348, parent: None, rest: Mat4::IDENTITY, stiffness: 1.0 }];
    let mut bone = std::collections::HashMap::new();
    for (_, l) in &ranked {
        let s = plant.segments[*l as usize];
        bone.insert(*l, branches.len() as u32);
        branches.push(BranchGroup {
            id: seed ^ 0x4252_414e_4348 ^ ((*l as u64 + 1) << 20),
            parent: Some(0),
            rest: Mat4::from_rotation_translation(along(s.from, s.to), s.from),
            // Wind sway bounds every voxel level's error by angle x limb length:
            // at 0.6 a 10 m limb added ~0.9 m, so the camera never left the
            // source mesh and its splats filled the budget. Stiff, long limbs.
            stiffness: 0.92,
        });
    }
    let inverse: Vec<Mat4> = branches.iter().map(|b| b.rest.inverse()).collect();
    let bone_of = |segment: u32| limb.get(segment as usize).and_then(|l| bone.get(l)).copied().unwrap_or(0);

    let mut reach = 0.6;
    let stems = loop {
        let r = runs(plant, reach);
        if r.len() * 3 <= limits.max_instances || reach > 64.0 {
            break r;
        }
        reach *= 1.5;
    };
    // Rounded joints: a ball where one run of an axis meets the next and at
    // the end of a thick stem, a stretched ball where a branch leaves its
    // parent. Thin twigs' seams are too small to see and keep the budget.
    const JOINT_RADIUS: f32 = 0.012;
    let taper_of = |radius: f32, end: f32| {
        let wanted = (end / radius.max(1e-6)).clamp(0.6, 1.0);
        (0..4).min_by(|&a, &c| (TAPERS[a] - wanted).abs().total_cmp(&(TAPERS[c] - wanted).abs())).unwrap_or(0)
    };
    let continues = |i: usize| stems.get(i + 1).is_some_and(|r| !r.5);
    let collars: usize = (0..stems.len()).map(|i| {
        let r = stems[i];
        usize::from((r.5 && plant.segments[r.0 as usize].order > 0) || (!r.5 && r.3 > JOINT_RADIUS)) + usize::from(!continues(i) && r.4 > JOINT_RADIUS)
    }).sum();
    let room = limits.max_instances.saturating_sub(stems.len() + collars);
    let keep = (room as f32 / plant.tufts.len().max(1) as f32).min(1.0);
    let mut instances = Vec::with_capacity(limits.max_instances);
    for (i, &(segment, from, to, radius, end, first)) in stems.iter().enumerate() {
        let b = bone_of(segment);
        // Overlap the next run a little so the rings meet without a seam.
        let length = from.distance(to) + radius.min(0.05);
        let world = Mat4::from_scale_rotation_translation(Vec3::new(radius, length, radius), along(from, to), from);
        let taper = taper_of(radius, end);
        let part = taper as u32 + if radius > 0.03 { 4 } else { 0 };
        instances.push(PartInstance { id: seed ^ (0x5354 << 40) ^ i as u64, part, branch: b, rest: affine(inverse[b as usize] * world), material: 0 });
        let mut joint = |tag: u64, scale: Vec3, centre: Vec3| {
            let world = Mat4::from_scale_rotation_translation(scale, along(from, to), centre);
            instances.push(PartInstance { id: seed ^ (tag << 40) ^ i as u64, part: 8, branch: b, rest: affine(inverse[b as usize] * world), material: 0 });
        };
        let axis = (to - from).normalize_or(Vec3::Y);
        if first && plant.segments[segment as usize].order > 0 {
            // A branch collar: a swelling around the base, embedded in the
            // parent and easing into the branch over a few of its radii.
            joint(0x434c, Vec3::new(radius * 1.45, radius * 2.6, radius * 1.45), from + axis * radius * 0.8);
        } else if !first && radius > JOINT_RADIUS {
            // A knuckle as wide as the wider of the two meeting rings hides
            // the step and the crack where the axis bends.
            let before = stems[i - 1];
            let ring = (before.3 * TAPERS[taper_of(before.3, before.4)]).max(radius) * 1.03;
            joint(0x4b4e, Vec3::splat(ring), from);
        }
        if !continues(i) && end > JOINT_RADIUS {
            // A rounded end instead of a flat cap.
            joint(0x454e, Vec3::splat(radius * TAPERS[taper]), from + axis * length);
        }
    }
    let mut placed = 0;
    for (i, t) in plant.tufts.iter().enumerate() {
        // Kept at random to spread the thinning; the draw can overshoot the
        // room by chance, so stop at the limit instead of failing the plant.
        if unit(seed, i as u64, 9) >= keep || instances.len() >= limits.max_instances {
            continue;
        }
        let b = bone_of(t.segment);
        // Kept tufts grow to cover for the ones the placement limit leaves out.
        let size = (0.75 + 0.45 * t.fullness) / keep.sqrt();
        let twist = Quat::from_axis_angle(Vec3::Y, unit(seed, i as u64, 10) * TAU);
        let world = Mat4::from_scale_rotation_translation(Vec3::splat(size), along(Vec3::ZERO, t.axis) * twist, t.at);
        let part = 9 + (unit(seed, i as u64, 11) * parts.tufts.len() as f32) as u32 % parts.tufts.len().max(1) as u32;
        instances.push(PartInstance { id: seed ^ (0x5446 << 40) ^ i as u64, part, branch: b, rest: affine(inverse[b as usize] * world), material: 1 });
        placed += 1;
    }
    let counts = Counts { bones: branches.len(), stems: stems.len() + collars, tufts: placed, tufts_dropped: plant.tufts.len() - placed };
    let material = |tint, foliage: bool| Material {
        tint,
        layer: if foliage { -1 } else { crate::scene::BARK_LAYER },
        roughness: if foliage { 0.8 } else { 0.96 },
        flags: if foliage { THIN_FOLIAGE | TWO_SIDED } else { 0 },
        // Wood: procedural bark plates about 12 cm across (shade.wgsl
        // bark_surface). The tint stays the plates' and furrows' mean, for voxels.
        scale: if foliage { 1.0 } else { 0.12 },
        metal: 0.0,
    };
    let geometry = parts.stems.iter().chain(std::iter::once(&parts.collar)).chain(&parts.tufts).cloned().collect();
    let parameters = Parameters { seed, archetype: Archetype::ConiferTwig, growth: 1.0, season: 0.5, density: placed.max(1) as u32 };
    let assembly = Assembly::from_parts(parameters, geometry, branches, instances, vec![material(look.wood, false), material(look.needles, true)], limits)?;
    Ok((assembly, counts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plant_growth::{grow, Species};

    #[test]
    fn a_crowded_crown_thins_its_brushes_instead_of_failing() {
        // Pine-lab tuning that once failed at 31 years: more brushes than room,
        // and the random thinning drew a few too many.
        let species = Species { crowding: 60, height: 52.0, laterals: 1.1, needle_years: 7, plumes: 6.0, ..Species::jeffrey_pine() };
        let parts = Parts::long_needle_pine();
        for age in [31, 55] {
            for seed in 1..6 {
                let (assembly, counts) = assemble(&grow(&species, age, seed), &parts, Look::jeffrey_pine(), seed, limits()).unwrap();
                assert!(assembly.instances.len() <= limits().max_instances);
                assert!(counts.tufts_dropped > 0);
            }
        }
    }

    #[test]
    fn every_life_stage_fits_the_grown_limits() {
        let (species, parts) = (Species::jeffrey_pine(), Parts::long_needle_pine());
        for age in [1, 3, 6, 12, 25, 50, 80, 120, 200, 300] {
            let plant = grow(&species, age, 1);
            let (assembly, counts) = assemble(&plant, &parts, Look::jeffrey_pine(), 1, limits()).unwrap();
            assert_eq!(assembly.instances.len(), counts.stems + counts.tufts);
            assert!(counts.bones <= 64);
        }
    }

    #[test]
    fn parts_are_closed_with_outward_volume() {
        let parts = Parts::long_needle_pine();
        for part in parts.stems.iter().chain(std::iter::once(&parts.collar)).chain(&parts.tufts) {
            let mut edges = std::collections::BTreeMap::new();
            let mut volume = 0.0f64;
            for tri in part.indices.chunks_exact(3) {
                let p: [Vec3; 3] = std::array::from_fn(|i| Vec3::from(part.vertices[tri[i] as usize].pos));
                volume += p[0].as_dvec3().dot(p[1].as_dvec3().cross(p[2].as_dvec3())) / 6.0;
                for i in 0..3 {
                    let (a, b) = (p[i].to_array().map(f32::to_bits), p[(i + 1) % 3].to_array().map(f32::to_bits));
                    let (key, dir) = if a < b { ((a, b), 1i32) } else { ((b, a), -1) };
                    let e = edges.entry(key).or_insert((0u32, 0i32));
                    e.0 += 1;
                    e.1 += dir;
                }
            }
            assert!(volume > 0.0, "part {:x}: volume {volume}", part.id);
            assert!(edges.values().all(|&e| e == (2, 0)), "part {:x} is not closed", part.id);
        }
    }

    #[test]
    fn an_older_tree_is_more_parts_not_bigger_parts() {
        let (species, parts) = (Species::jeffrey_pine(), Parts::long_needle_pine());
        let limits = limits();
        let (young, _) = assemble(&grow(&species, 6, 1), &parts, Look::jeffrey_pine(), 1, limits).unwrap();
        let (old, _) = assemble(&grow(&species, 80, 1), &parts, Look::jeffrey_pine(), 1, limits).unwrap();
        assert_eq!(young.parts.len(), old.parts.len());
        assert!(old.instances.len() > 10 * young.instances.len());
    }
}
