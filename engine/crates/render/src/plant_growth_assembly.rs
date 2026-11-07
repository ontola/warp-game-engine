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
    /// Thin, medium and thick stems: fewer sides where they cover fewer pixels.
    pub stems: [PartGeometry; 3],
    pub tufts: Vec<PartGeometry>,
}

fn stem(id: u64, sides: u32) -> PartGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for i in 0..=sides {
        let a = i as f32 / sides as f32 * TAU;
        let normal = [a.cos(), 0.0, a.sin()];
        for y in [0.0, 1.0] {
            vertices.push(Vertex { pos: [a.cos(), y, a.sin()], normal, uv: [i as f32 / sides as f32, y] });
        }
    }
    for i in 0..sides {
        let b = i * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    PartGeometry { id, vertices, indices }
}

fn unit(variant: u64, index: u64, tag: u64) -> f32 {
    let mut x = variant.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ tag.wrapping_mul(0x94d0_49bb_1331_11eb);
    x = (x ^ (x >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    ((x ^ (x >> 32)) >> 40) as f32 / (1u64 << 24) as f32
}

/// A ball of long needles, thin two-triangle blades fanning out from the shoot
/// tip: a stand-in until a proper tuft mesh replaces it.
fn tuft(id: u64, variant: u64, needles: u32, length: f32) -> PartGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for i in 0..needles {
        let v = i as u64;
        // Mostly forward and outward; a few fall back past the stem.
        let polar = (unit(variant, v, 1) * 1.0).sqrt() * 2.1;
        let around = unit(variant, v, 2) * TAU;
        let axis = Vec3::new(polar.sin() * around.cos(), polar.cos(), polar.sin() * around.sin());
        let side = axis.cross(if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize();
        let normal = axis.cross(side).normalize();
        let root = axis * 0.012;
        let long = length * (0.7 + 0.5 * unit(variant, v, 3));
        let tip = axis * long - Vec3::Y * 0.12 * long * unit(variant, v, 4);
        let half = side * 0.0028;
        let b = vertices.len() as u32;
        for (pos, y) in [(root - half, 0.0), (root + half, 0.0), (root * 0.5 + tip * 0.5 - half * 0.7, 0.5), (root * 0.5 + tip * 0.5 + half * 0.7, 0.5), (tip, 1.0)] {
            vertices.push(Vertex { pos: pos.to_array(), normal: normal.to_array(), uv: [0.5, y] });
        }
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3, b + 2, b + 3, b + 4]);
    }
    PartGeometry { id, vertices, indices }
}

impl Parts {
    /// Placeholder parts for a long-needled pine.
    pub fn long_needle_pine() -> Self {
        const STEM: u64 = 0x5047_5354_0000_0000;
        const TUFT: u64 = 0x5047_5446_0000_0000;
        Self {
            stems: [stem(STEM + 1, 4), stem(STEM + 2, 6), stem(STEM + 3, 12)],
            tufts: (0..3).map(|v| tuft(TUFT + v, v, 56, 0.22)).collect(),
        }
    }
}

/// Colours of the wood and the needles.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub wood: [f32; 3],
    pub needles: [f32; 3],
}

impl Look {
    pub fn jeffrey_pine() -> Self {
        Self { wood: [0.36, 0.19, 0.11], needles: [0.11, 0.21, 0.09] }
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
fn runs(plant: &Plant, reach: f32) -> Vec<(u32, Vec3, Vec3, f32)> {
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
                out.push((last as u32, start, plant.segments[at].to, radius));
                start = plant.segments[at].to;
                radius = plant.segments[at].radius[0];
                last = at;
            }
        }
        let end = plant.segments[at].to;
        if end.distance(start) > 1e-3 {
            out.push((last as u32, start, end, radius));
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
            stiffness: 0.6,
        });
    }
    let inverse: Vec<Mat4> = branches.iter().map(|b| b.rest.inverse()).collect();
    let bone_of = |segment: u32| limb.get(segment as usize).and_then(|l| bone.get(l)).copied().unwrap_or(0);

    let mut reach = 0.6;
    let stems = loop {
        let r = runs(plant, reach);
        if r.len() * 2 <= limits.max_instances || reach > 16.0 {
            break r;
        }
        reach *= 1.5;
    };
    let room = limits.max_instances.saturating_sub(stems.len());
    let keep = (room as f32 / plant.tufts.len().max(1) as f32).min(1.0);
    let mut instances = Vec::with_capacity(limits.max_instances);
    for (i, (segment, from, to, radius)) in stems.iter().enumerate() {
        let (from, to) = (*from, *to);
        let b = bone_of(*segment);
        let world = Mat4::from_scale_rotation_translation(Vec3::new(*radius, from.distance(to), *radius), along(from, to), from);
        let part = if *radius > 0.06 { 2 } else if *radius > 0.015 { 1 } else { 0 };
        instances.push(PartInstance { id: seed ^ (0x5354 << 40) ^ i as u64, part, branch: b, rest: affine(inverse[b as usize] * world), material: 0 });
    }
    let mut placed = 0;
    for (i, t) in plant.tufts.iter().enumerate() {
        if unit(seed, i as u64, 9) >= keep {
            continue;
        }
        let b = bone_of(t.segment);
        let size = 0.75 + 0.45 * t.fullness;
        let twist = Quat::from_axis_angle(Vec3::Y, unit(seed, i as u64, 10) * TAU);
        let world = Mat4::from_scale_rotation_translation(Vec3::splat(size), along(Vec3::ZERO, t.axis) * twist, t.at);
        let part = 3 + (unit(seed, i as u64, 11) * parts.tufts.len() as f32) as u32 % parts.tufts.len().max(1) as u32;
        instances.push(PartInstance { id: seed ^ (0x5446 << 40) ^ i as u64, part, branch: b, rest: affine(inverse[b as usize] * world), material: 1 });
        placed += 1;
    }
    let counts = Counts { bones: branches.len(), stems: stems.len(), tufts: placed, tufts_dropped: plant.tufts.len() - placed };
    let material = |tint, foliage: bool| Material {
        tint,
        layer: -1,
        roughness: if foliage { 0.8 } else { 0.96 },
        flags: if foliage { THIN_FOLIAGE | TWO_SIDED } else { 0 },
        scale: 1.0,
        metal: 0.0,
    };
    let geometry = parts.stems.iter().chain(&parts.tufts).cloned().collect();
    let parameters = Parameters { seed, archetype: Archetype::ConiferTwig, growth: 1.0, season: 0.5, density: placed.max(1) as u32 };
    let assembly = Assembly::from_parts(parameters, geometry, branches, instances, vec![material(look.wood, false), material(look.needles, true)], limits)?;
    Ok((assembly, counts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plant_growth::{grow, Species};

    #[test]
    fn every_life_stage_fits_the_recipe_limits() {
        let (species, parts) = (Species::jeffrey_pine(), Parts::long_needle_pine());
        for age in [1, 3, 6, 12, 25, 50, 80, 120, 200, 300] {
            let plant = grow(&species, age, 1);
            let (assembly, counts) = assemble(&plant, &parts, Look::jeffrey_pine(), 1, crate::plant_recipe::recipe_limits()).unwrap();
            assert_eq!(assembly.instances.len(), counts.stems + counts.tufts);
            assert!(counts.bones <= 64);
        }
    }

    #[test]
    fn an_older_tree_is_more_parts_not_bigger_parts() {
        let (species, parts) = (Species::jeffrey_pine(), Parts::long_needle_pine());
        let limits = crate::plant_recipe::recipe_limits();
        let (young, _) = assemble(&grow(&species, 6, 1), &parts, Look::jeffrey_pine(), 1, limits).unwrap();
        let (old, _) = assemble(&grow(&species, 80, 1), &parts, Look::jeffrey_pine(), 1, limits).unwrap();
        assert_eq!(young.parts.len(), old.parts.len());
        assert!(old.instances.len() > 10 * young.instances.len());
    }
}
