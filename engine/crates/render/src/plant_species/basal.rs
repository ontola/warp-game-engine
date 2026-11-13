//! Basal growth recipes. Each culm or frond is one authored shoot with its own
//! foot and facing, in the same articulated source assembly as the woody plants.
use crate::{
    foliage_assembly::{Archetype, Assembly, BranchGroup, Limits, Parameters, PartInstance},
    plant_growth::Species,
    plant_growth_assembly::{Counts, Look, Parts},
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug)]
pub(crate) struct BasalHabit {
    /// Reference size for scaling authored geometry through the usual height setting.
    pub height_m: f32,
    /// Radius of the ground covered by shoot bases at maturity.
    radius_m: f32,
    mature_shoots: u32,
    /// Inclination from vertical at the centre and margin, in radians.
    tilt: [f32; 2],
    /// Relative size of newly emerging shoots in the centre.
    emerging_scale: f32,
    /// Share of older outer stems carrying dry foliage.
    dry_fraction: f32,
    /// Minimum individual shoot maturity for each authored organ variant.
    variant_maturity: [f32; 3],
    /// Extra outward lean near the edge, in radians per organ variant.
    edge_lean: [f32; 3],
}

pub(crate) const DRINN: BasalHabit = BasalHabit {
    height_m: 0.83,
    radius_m: 0.16,
    mature_shoots: 63,
    tilt: [0.07, 0.68],
    emerging_scale: 0.76,
    dry_fraction: 0.26,
    variant_maturity: [0.0, 0.0, 0.55],
    edge_lean: [0.05, 0.05, 0.0],
};
pub(crate) const SWORD_FERN: BasalHabit = BasalHabit {
    height_m: 0.90,
    radius_m: 0.075,
    mature_shoots: 14,
    tilt: [0.03, 0.36],
    emerging_scale: 0.58,
    dry_fraction: 0.0,
    variant_maturity: [0.0, 0.33, 0.56],
    edge_lean: [0.0; 3],
};

impl BasalHabit {
    pub(crate) fn assemble(
        self,
        rules: &Species,
        look: Look,
        parts: &Parts,
        age: u32,
        seed: u64,
        limits: Limits,
    ) -> Result<(Assembly, Counts), String> {
        if !rules.height.is_finite()
            || rules.height <= 0.0
            || !rules.pace.is_finite()
            || rules.pace <= 0.0
            || !rules.shape.is_finite()
            || rules.shape <= 0.0
        {
            return Err("basal growth needs positive finite height, pace and shape".into());
        }
        // Age recruits more axes and expands the footprint as well as growing
        // each organ. Seeded axis identities and bearings persist across ages.
        let maturity = (1.0 - (-(age.max(1) as f32) / rules.pace).exp()).powf(rules.shape);
        let size = rules.height / self.height_m;
        if parts.tufts.len() != self.variant_maturity.len() {
            return Err("basal shoot library and variant thresholds disagree".into());
        }
        let unit = |i: u32, channel: u32| {
            let mut h = seed
                ^ u64::from(i).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ u64::from(channel).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            ((h ^ (h >> 31)) >> 40) as f32 / 16_777_216.0
        };
        let count = 1
            + (self.mature_shoots as f32 * maturity.powf(1.5) * (0.85 + 0.22 * unit(0, 0))).round()
                as u32;
        let mut instances = Vec::with_capacity(count as usize);
        let heading = TAU * unit(0, 1);
        // Use a low-discrepancy turn, with some jitter. Outer axes open out,
        // while the most recently emerged organs unfold near the centre.
        let golden_angle = PI * (3.0 - 5.0_f32.sqrt());
        for i in 0..count {
            let radial_age = ((count - i) as f32 - 0.35) / count as f32;
            let radius = radial_age.sqrt() * (0.78 + 0.22 * unit(i, 2));
            let bearing = heading + i as f32 * golden_angle + (unit(i, 3) - 0.5) * 0.55;
            let outward = Quat::from_rotation_y(bearing);
            let base = outward * Vec3::Z * (radius * self.radius_m * maturity * size);
            let organ_maturity = maturity * (0.35 + 0.65 * radial_age);
            let variants = self
                .variant_maturity
                .partition_point(|threshold| *threshold <= organ_maturity);
            let variant = (unit(i, 6) * variants as f32) as usize;
            let margin = self.edge_lean[variant] * radial_age.powi(4) * unit(i, 8);
            let tilt = (self.tilt[0] + (self.tilt[1] - self.tilt[0]) * radial_age + margin)
                * (0.8 + 0.4 * unit(i, 4));
            let rotation = outward * Quat::from_rotation_x(tilt);
            let relative_length =
                self.emerging_scale + (1.0 - self.emerging_scale) * radial_age.sqrt();
            let length = size * maturity * relative_length * (0.88 + 0.27 * unit(i, 5));
            let dry = unit(i, 7) < self.dry_fraction * radial_age * 2.0 * maturity;
            instances.push(PartInstance {
                id: seed ^ (0x4241_5345_0000_0000 | u64::from(i)),
                part: variant as u32,
                branch: 0,
                material: u32::from(dry),
                rest: Mat4::from_scale_rotation_translation(Vec3::splat(length), rotation, base),
            });
        }
        let material = |tint| Material {
            tint,
            layer: -1,
            roughness: 0.8,
            flags: THIN_FOLIAGE | TWO_SIDED,
            scale: 1.0,
            metal: 0.0,
        };
        let root = BranchGroup {
            id: seed,
            parent: None,
            rest: Mat4::IDENTITY,
            stiffness: 1.0,
        };
        let parameters = Parameters {
            seed,
            archetype: Archetype::GrassTuft,
            growth: 1.0,
            season: 0.5,
            density: count,
        };
        let assembly = Assembly::from_parts(
            parameters,
            parts.tufts.clone(),
            vec![root],
            instances,
            vec![material(look.needles), material(look.wood)],
            limits,
        )?;
        Ok((
            assembly,
            Counts {
                bones: 1,
                stems: 0,
                tufts: count as usize,
                tufts_dropped: 0,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::{plant_growth_assembly::limits, plant_species::grown};
    use glam::Vec3;

    #[test]
    fn herbs_recruit_separate_basal_axes_and_keep_source_identities_as_they_grow() {
        for name in ["forest_fern", "dune_grass"] {
            let species = grown(name).unwrap();
            let parts = species.parts();
            for seed in [species.seed, 24, 71] {
                let build = |age| species.assemble(&parts, age, seed, limits()).unwrap().0;
                let young = build(1);
                let adult = build(species.age);
                assert!(young.instances.len() < adult.instances.len());
                assert!(young.bounds().unwrap()[1][1] < adult.bounds().unwrap()[1][1]);
                assert!(
                    adult.bounds().unwrap()[0][1] >= -0.004,
                    "{name}: below soil"
                );
                for (i, old) in young.instances.iter().enumerate() {
                    assert_eq!(adult.instances[i].id, old.id);
                    let base = old.rest.transform_point3(Vec3::ZERO);
                    assert_eq!(base.y, 0.0);
                    assert!(
                        young.instances[..i].iter().all(|axis| axis
                            .rest
                            .transform_point3(Vec3::ZERO)
                            .distance(base)
                            > 1e-5)
                    );
                }
                let repeat = build(species.age);
                assert!(
                    adult
                        .instances
                        .iter()
                        .zip(&repeat.instances)
                        .all(|(a, b)| a.rest == b.rest
                            && a.part == b.part
                            && a.material == b.material)
                );
                if name == "dune_grass" {
                    assert!(young.instances.iter().all(|axis| axis.part < 2));
                    assert!(adult.instances.iter().any(|axis| axis.part == 2));
                }
            }
        }
    }
}
