//! Growth rules and source foliage shared by the terrain viewer and games.
use crate::{
    foliage_assembly::PartGeometry,
    plant_growth::Species,
    plant_growth_assembly::{Look, Parts},
    plant_spray,
};
use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct GrownSpecies {
    pub rules: Species,
    pub look: Look,
    /// Representative stand age, in years.
    pub age: u32,
    pub seed: u64,
    /// Width, length along the shoot and depth of the metre-long leafy spray. Conifers use
    /// the original pine-lab needle brushes designed for their growth rules.
    leaf_spray: Option<[f32; 3]>,
}

impl GrownSpecies {
    pub fn parts(&self) -> Parts {
        let mut parts = Parts::for_look(&self.look);
        if let Some(scale) = self.leaf_spray {
            parts.tufts = (0..3).map(|v| leafy_shoot(v, Vec3::from(scale))).collect();
        }
        parts
    }
}

// Assembly puts each foliage part on a growing shoot tip. Keep the spray's
// young green stalk and petioles with its closed leaf blades in one part so
// the leaves remain connected, including in the voxel representation.
fn leafy_shoot(variant: u32, scale: Vec3) -> PartGeometry {
    let (mut shoot, blades) = plant_spray::broadleaf_spray(variant);
    let offset = shoot.vertices.len() as u32;
    shoot.vertices.extend(blades.vertices);
    shoot.indices.extend(blades.indices.into_iter().map(|i| offset + i));
    for (i, v) in shoot.vertices.iter_mut().enumerate() {
        let mut pos = Vec3::from(v.pos) * scale;
        let normal = (Vec3::from(v.normal) / scale).normalize();
        if i < offset as usize {
            // Retain a fine, solid green stalk when the metre spray is
            // shortened for dwarf shrubs (particularly its tiny petioles).
            pos += normal * 0.0004;
        }
        v.pos = pos.to_array();
        v.normal = normal.to_array();
    }
    // The grown asset cache uses part IDs rather than hashing its geometry.
    // Editing a blade, its attachment or our transform must invalidate
    // that cook even when the source triangle counts stay the same.
    let vertex_bits = shoot.vertices.iter()
        .flat_map(|v| v.pos.into_iter().chain(v.normal).chain(v.uv)).map(f32::to_bits);
    shoot.id = vertex_bits.chain(shoot.indices.iter().copied())
        .fold(0x4752_4f57_4c45_4146u64, |h, bits| (h ^ u64::from(bits)).wrapping_mul(0x100_0000_01b3));
    shoot
}

/// Architectural rules for the woody species used by the desert, Norwegian
/// uplands and African rainforest. The explicit pine and spruce models
/// also supply the reference conifers used by the terrain flight demo.
pub fn grown(name: &str) -> Option<GrownSpecies> {
    let recipe = super::named(name)?;
    let native = |rules, look, age| GrownSpecies { rules, look, age, seed: 1, leaf_spray: None };
    let leafy = |rules, age, scale| GrownSpecies {
        rules, age, seed: recipe.id, leaf_spray: Some(scale),
        look: Look { needles: recipe.foliage_colour, wood: recipe.wood_colour, ..Look::default() },
    };
    // A woody broadleaf keeps recent tips in leaf; lateral growth and the
    // rate at which its lower limbs are shed determine the crown.
    let broadleaf = Species {
        height: 15.0, pace: 15.0, shape: 1.15, first_whorl: 2, whorl: [1, 2],
        laterals: 0.85, max_order: 3, limb_step: 0.40, side_length: 0.62,
        limb_life: [10, 28], crowding: 8, angle: [0.95, 1.25], rise: 0.06, droop: 0.018,
        needle_years: 2, shade: 0.03, min_light: 0.3, tip_radius: 0.003,
        wood: 0.011, limb_wood: 0.48, dead_years: 5, order_wood: [0.0; 4],
        plumes: 1.6, live_crown: 0.66, lift_years: 21.0,
    };
    let shrub = Species {
        height: 2.0, pace: 7.0, shape: 0.95, first_whorl: 1, whorl: [2, 3],
        laterals: 0.8, max_order: 2, limb_step: 0.19, side_length: 0.6,
        angle: [0.91, 1.12], shade: 0.018, crowding: 12, wood: 0.005,
        tip_radius: 0.002, live_crown: 1.0, plumes: 1.0, ..broadleaf
    };
    Some(match name {
        "pine" => native(Species::jeffrey_pine(), Look::jeffrey_pine(), 28),
        // Scots pine sheds its lower whorls, leaving a copper-brown bole
        // beneath ascending limbs with short blue-green needles.
        "scots_pine" => native(Species {
            height: 31.0, pace: 36.0, shape: 1.55, whorl: [2, 4],
            limb_step: 0.31, side_length: 0.67, angle: [1.26, 1.48], rise: 0.065,
            needle_years: 4, live_crown: 0.55, lift_years: 28.0, dead_years: 7,
            ..Species::jeffrey_pine()
        }, Look {
            needles: recipe.foliage_colour, wood: recipe.wood_colour,
            needle_length: 0.135, needle_width: 0.0022, brush_reach: 0.19,
            brush_needles: 65, tip_spread: 0.55, ..Look::jeffrey_pine()
        }, 28),
        "upland_spruce" => native(Species::norway_spruce(), Look::norway_spruce(), 18),
        "juniper" => native(Species {
            height: 1.25, pace: 6.5, limb_step: 0.15, angle: [1.3, 1.55], rise: 0.015,
            needle_years: 4, ..shrub
        }, Look {
            wood: recipe.wood_colour, needles: recipe.foliage_colour, brush_needles: 44,
            needle_length: 0.028, brush_reach: 0.075, needle_width: 0.0009,
            ball: 1.4, ..Look::norway_spruce()
        }, 9),
        "mountain_birch" => leafy(Species {
            height: 9.0, pace: 13.0, limb_step: 0.26, laterals: 0.67,
            angle: [0.72, 1.05], live_crown: 0.62, ..broadleaf
        }, 18, [0.73, 0.66, 0.73]),
        "dwarf_birch" => leafy(Species {
            height: 0.62, pace: 5.0, limb_step: 0.10, rise: 0.015,
            angle: [1.40, 1.50], ..shrub
        }, 8, [0.28, 0.23, 0.28]),
        "desert_shrub" => leafy(Species {
            height: 1.36, pace: 5.8, whorl: [1, 2], laterals: 0.45, limb_step: 0.19,
            angle: [1.30, 1.38], rise: 0.04, wood: 0.004, needle_years: 1, ..shrub
        }, 9, [0.23, 0.27, 0.23]),
        "saltbush" => leafy(Species {
            height: 0.44, pace: 5.0, limb_step: 0.13, angle: [1.46, 1.60],
            rise: 0.02, ..shrub
        }, 8, [0.33, 0.23, 0.33]),
        // The emergent sets its first scaffold limbs high on a clean bole.
        // Its spreading crown overtops the shorter, darker mahogany.
        "kapok" => leafy(Species {
            height: 33.0, pace: 17.0, first_whorl: 9, whorl: [2, 3],
            limb_step: 1.12, angle: [1.47, 1.55], rise: 0.055, droop: 0.025,
            wood: 0.017, live_crown: 0.38, lift_years: 18.0, dead_years: 4, ..broadleaf
        }, 17, [1.55, 1.32, 1.55]),
        "african_mahogany" => leafy(Species {
            height: 22.0, pace: 17.0, first_whorl: 4, whorl: [1, 3],
            limb_step: 0.48, laterals: 1.1, wood: 0.015,
            needle_years: 3, ..broadleaf
        }, 17, [1.1, 1.08, 1.1]),
        "forest_coffee" => leafy(Species {
            height: 2.9, pace: 7.5, whorl: [2, 2], limb_step: 0.16,
            angle: [1.32, 1.47], needle_years: 3, ..shrub
        }, 9, [0.65, 0.47, 0.65]),
        // Young fronds rise from the crown; as their axes extend they bend
        // down. The leaflets on a frond sit close to one plane.
        "raffia_palm" => leafy(Species {
            height: 1.18, pace: 9.0, first_whorl: 7, whorl: [2, 3], max_order: 1,
            limb_step: 0.42, angle: [0.67, 1.10], rise: -0.20, droop: 0.14,
            needle_years: 3, limb_wood: 0.19, wood: 0.012, plumes: 1.0,
            live_crown: 0.4, lift_years: 12.0, ..shrub
        }, 12, [0.75, 1.02, 0.28]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{biome::Biome, plant_growth, plant_growth_assembly};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn regional_growth_assemblies_build_with_distinct_foliage_and_canopy_layers() {
        let mut dimensions = BTreeMap::new();
        let mut leaf_ids = BTreeSet::new();
        for region in ["desert", "norway", "rainforest"] {
            for (name, _) in Biome::named(region).unwrap().species() {
                let s = grown(name).unwrap_or_else(|| panic!("{region}: no grown {name}"));
                let parts = s.parts();
                let plant = plant_growth::grow(&s.rules, s.age, s.seed);
                let (assembly, placed) = plant_growth_assembly::assemble(
                    &plant, &parts, s.look, s.seed, plant_growth_assembly::limits(),
                ).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(placed.tufts > 0 && placed.stems > 0, "{name}");
                if s.leaf_spray.is_some() {
                    for p in &parts.tufts {
                        assert!(leaf_ids.insert(p.id), "{name}: reused shoot identity");
                        assert!(p.vertices.iter().all(|v| v.uv[1] >= 0.0), "{name}: needle brush");
                    }
                }
                let [lo, hi] = assembly.bounds().unwrap();
                dimensions.insert(name, (hi[1], (hi[0] - lo[0]).max(hi[2] - lo[2])));
            }
        }
        let height = |name| dimensions[name].0;
        let span = |name| dimensions[name].1;
        assert!(height("kapok") > 1.4 * height("african_mahogany"));
        assert!(span("kapok") > 0.75 * height("kapok"), "the emergent has a spreading crown");
        assert!(height("african_mahogany") > 4.0 * height("forest_coffee"));
        assert!(span("raffia_palm") > 1.6 * height("raffia_palm"));
        assert!(height("desert_shrub") > 2.0 * height("saltbush"));
        assert!(span("desert_shrub") > 1.5 * height("desert_shrub"), "scrub branches out close to the ground");
    }

    #[test]
    fn reference_conifers_reuse_the_forest_demos_growth_and_brushes() {
        for (name, rules, look) in [
            ("pine", Species::jeffrey_pine(), Look::jeffrey_pine()),
            ("upland_spruce", Species::norway_spruce(), Look::norway_spruce()),
        ] {
            let s = grown(name).unwrap();
            let expected = plant_growth::grow(&rules, s.age, 1);
            let actual = plant_growth::grow(&s.rules, s.age, s.seed);
            assert_eq!(expected.segments, actual.segments);
            assert_eq!(expected.tufts, actual.tufts);
            let expected = Parts::for_look(&look);
            let actual = s.parts();
            assert!(actual.tufts.iter().zip(expected.tufts).all(|(a, b)| a.id == b.id));
        }
    }
}
