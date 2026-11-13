//! A patch expands at the rooted ends of its woody shoots. Each piece of the
//! runner carries its own short leaf shoots and may flower after it matures.
use super::{dryas, growth::seed_unit};
use crate::{
    foliage_assembly::{Assembly, BranchGroup, Limits, Parameters, PartGeometry, PartInstance},
    plant_growth::Species,
    plant_growth_assembly::{Counts, Look, Parts},
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy)]
pub(crate) struct MatHabit {
    pub height_m: f32,
    primary_axes: u32,
    extension_units: f32,
    flower_frequency: f32,
    petals: [f32; 3],
    pollen: [f32; 3],
}

pub(crate) const DRYAS: MatHabit = MatHabit {
    height_m: 0.075,
    primary_axes: 5,
    extension_units: 5.8,
    flower_frequency: 0.38,
    petals: [0.88, 0.85, 0.73],
    pollen: [0.88, 0.53, 0.055],
};

/// The long runner keeps the very same wood when its side shoot flowers.
/// Leafy and flowering shoots can be drawn with individual organ materials.
pub(super) fn dryas_parts() -> Vec<PartGeometry> {
    let short = dryas::shoot_variant(0);
    let long = dryas::shoot_variant(1);
    let flowering = dryas::shoot_variant(2);
    let spur = dryas::shoot_variant(3);
    vec![
        short.wood,
        short.leaves,
        long.wood,
        long.leaves,
        flowering.leaves,
        flowering.petals,
        flowering.stamens,
        spur.wood,
        spur.leaves,
    ]
}

const SHORT: &[(u32, u32)] = &[(0, 0), (1, 1)];
const LONG: &[(u32, u32)] = &[(2, 0), (3, 1)];
const FLOWERING: &[(u32, u32)] = &[(2, 0), (4, 1), (5, 2), (6, 3)];

const SPUR: &[(u32, u32)] = &[(7, 0), (8, 1)];

struct GrowingMat {
    habit: MatHabit,
    seed: u64,
    size: f32,
    shoots: u32,
    instances: Vec<PartInstance>,
}

impl GrowingMat {
    fn place(
        &mut self,
        id: u32,
        base: Vec3,
        bearing: f32,
        scale: f32,
        organs: &[(u32, u32)],
    ) -> Mat4 {
        let rest = Mat4::from_scale_rotation_translation(
            Vec3::splat(scale),
            Quat::from_rotation_y(bearing),
            base,
        );
        for &(part, material) in organs {
            self.instances.push(PartInstance {
                id: self.seed ^ (u64::from(id) << 8 | u64::from(material)),
                part,
                branch: 0,
                material,
                rest,
            });
        }
        self.shoots += 1;
        rest
    }

    /// Short sterile shoots close the evergreen mat after a runner roots. They
    /// split again on last year's wood. Leaves keep their normal dimensions
    /// on these short internodes and each new shoot starts at a rooted node.
    fn flush(&mut self, parent_id: u32, buds: [Vec3; 2], bearing: f32, age: f32) {
        let seed = self.seed;
        for flank in 0..2 {
            let sign = if flank == 0 { -1.0 } else { 1.0 };
            let spur_id = 0x4000_0000 | parent_id << 4 | flank << 2;
            let unit = |ch| seed_unit(seed, spur_id, ch);
            let maturity = age - (0.55 + 0.62 * unit(0) + 0.33 * flank as f32);
            if maturity <= 0.0 {
                continue;
            }
            let angle = bearing + sign * (0.86 + 1.32 * unit(1));
            let scale = self.size * (0.81 + 0.27 * unit(2)) * (0.06 + 0.94 * maturity.min(1.0));
            let first = self.place(spur_id, buds[flank as usize], angle, scale, SPUR);
            let bud = first.transform_point3(dryas::tip(3));
            for side in 0..2 {
                let strength = maturity - (0.95 + 0.66 * unit(3 + side) + side as f32 * 0.43);
                if strength <= 0.0 {
                    continue;
                }
                let orientation =
                    angle + (side as f32 * 2.0 - 1.0) * (0.70 + 0.86 * unit(5 + side));
                let length =
                    self.size * (0.79 + 0.27 * unit(7 + side)) * (0.06 + 0.94 * strength.min(1.0));
                self.place(spur_id + side + 1, bud, orientation, length, SPUR);
            }
        }
    }

    /// The integer part of extension counts established shoots. The newest
    /// shoot unfolds at the last rooted tip. Its final bearing is seed-stable.
    fn extend(&mut self, axis: u32, mut base: Vec3, mut bearing: f32, extension: f32, order: u32) {
        let seed = self.seed;
        for node in 0..extension.ceil() as u32 {
            let id = axis * 16 + node;
            let unit = |channel| seed_unit(seed, id, channel);
            bearing += (unit(1) - 0.5) * 1.16;
            let long = unit(2) < 0.67;
            let shoot_age = extension - node as f32;
            let flower =
                order < 2 && long && shoot_age > 1.8 && unit(3) < self.habit.flower_frequency;
            let (variant, organs) = if flower {
                (2, FLOWERING)
            } else if long {
                (1, LONG)
            } else {
                (0, SHORT)
            };
            let length = self.size * (0.82 + 0.32 * unit(4)) * (0.06 + 0.94 * shoot_age.min(1.0));
            let rest = self.place(id, base, bearing, length, organs);
            base = rest.transform_point3(dryas::tip(variant));
            let middle = rest.transform_point3(dryas::node(variant, 1) - Vec3::Y * 0.00075);
            self.flush(id, [middle, base], bearing, shoot_age);
            let delay = if order == 0 { 0.95 } else { 1.30 };
            if order < 2 && shoot_age > delay {
                let side = if (axis + node) % 2 == 0 { -1.0 } else { 1.0 };
                let lateral_bearing = bearing + side * (0.72 + 0.83 * unit(5));
                let lateral_growth = (shoot_age - delay) * (0.56 + 0.24 * unit(6));
                self.extend(16 + id, base, lateral_bearing, lateral_growth, order + 1);
            }
        }
    }
}

impl MatHabit {
    pub fn assemble(
        self,
        rules: &Species,
        look: Look,
        parts: &Parts,
        age: u32,
        seed: u64,
        limits: Limits,
    ) -> Result<(Assembly, Counts), String> {
        if [rules.height, rules.pace, rules.shape]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("creeping growth needs positive finite height, pace and shape".into());
        }
        let years = age.max(1) as f32;
        let maturity = (-(-years / rules.pace).exp_m1()).powf(rules.shape);
        let mut mat = GrowingMat {
            habit: self,
            seed,
            size: rules.height / self.height_m,
            shoots: 0,
            instances: Vec::new(),
        };
        let primary_axes = self.primary_axes + u32::from(seed_unit(seed, 0, 9) > 0.55);
        let origin_bearing = TAU * seed_unit(seed, 0, 10);
        for axis in 0..primary_axes {
            let delay = axis as f32 * 0.31;
            let extension =
                self.extension_units * maturity * (0.84 + 0.27 * seed_unit(seed, axis, 11)) - delay;
            if extension > 0.0 {
                let bearing = origin_bearing
                    + axis as f32 * 2.399_963_1
                    + (seed_unit(seed, axis, 12) - 0.5) * 0.55;
                mat.extend(axis, Vec3::ZERO, bearing, extension, 0);
            }
        }
        let material = |tint, flags| Material {
            tint,
            flags,
            layer: -1,
            roughness: 0.8,
            scale: 1.0,
            metal: 0.0,
        };
        let leaf = THIN_FOLIAGE | TWO_SIDED;
        let materials = vec![
            material(look.wood, 0),
            material(look.needles, leaf),
            material(self.petals, leaf),
            material(self.pollen, 0),
        ];
        let root = BranchGroup {
            id: seed,
            parent: None,
            rest: Mat4::IDENTITY,
            stiffness: 1.0,
        };
        let parameters = Parameters {
            density: mat.shoots,
            ..Parameters::grass(seed)
        };
        let assembly = Assembly::from_parts(
            parameters,
            parts.tufts.clone(),
            vec![root],
            mat.instances,
            materials,
            limits,
        )?;
        Ok((
            assembly,
            Counts {
                bones: 1,
                stems: mat.shoots as usize,
                tufts: mat.shoots as usize,
                tufts_dropped: 0,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plant_species::grown;
    use std::collections::{HashMap, HashSet};

    /// Match the actual insertions, not the indices used to generate children.
    /// A flowering runner shares its wood with the non-flowering long shoot.
    #[test]
    fn every_runner_grows_from_existing_wood_through_all_ages() {
        let dryas = grown("dryas").unwrap();
        let parts = dryas.parts();
        for seed in [0, 7, dryas.seed] {
            let mut previous = HashSet::new();
            for age in [1, 3, dryas.age, 16] {
                let (mat, counts) = dryas
                    .assemble(&parts, age, seed, crate::plant_growth_assembly::limits())
                    .unwrap();
                let mut insertions = vec![Vec3::ZERO];
                let mut wood = HashMap::new();
                let mut flowers = HashMap::new();
                let mut hearts = 0;
                for organ in &mat.instances {
                    let root = organ.rest.transform_point3(Vec3::ZERO);
                    let id = (seed ^ organ.id) >> 8;
                    match organ.material {
                        0 => {
                            assert!(
                                insertions.iter().any(|&p| root.distance(p) < 1e-6),
                                "age {age}, seed {seed}: runner rooted in empty space at {root}"
                            );
                            assert!(wood.insert(id, organ.rest).is_none());
                            let variant = match organ.part {
                                0 => 0,
                                2 => 1,
                                7 => 3,
                                _ => unreachable!(),
                            };
                            insertions.push(organ.rest.transform_point3(dryas::tip(variant)));
                            insertions.push(
                                organ
                                    .rest
                                    .transform_point3(dryas::node(variant, 1) - Vec3::Y * 0.00075),
                            );
                        }
                        1 => assert_eq!(wood[&id], organ.rest),
                        2 => {
                            assert_eq!(wood[&id], organ.rest);
                            assert!(flowers.insert(id, organ.rest).is_none());
                        }
                        3 => {
                            assert_eq!(flowers[&id], organ.rest);
                            hearts += 1;
                        }
                        _ => unreachable!(),
                    }
                }
                assert_eq!(flowers.len(), hearts);
                assert_eq!(counts.stems, wood.len());
                let current: HashSet<_> = wood.keys().copied().collect();
                assert!(
                    previous.is_subset(&current),
                    "lost rooted runners, age {age} seed {seed}"
                );
                previous = current;
                if age == 1 {
                    assert!(flowers.is_empty());
                }
            }
        }
    }
}
