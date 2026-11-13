//! Growth rules and source foliage shared by the terrain viewer and games.
use super::shoots::ShootRecipe;
use crate::{
    plant_growth::Species,
    plant_growth_assembly::{Look, Parts},
};

#[derive(Clone, Copy, Debug)]
pub struct GrownSpecies {
    pub rules: Species,
    pub look: Look,
    /// Representative stand age, in years.
    pub age: u32,
    pub seed: u64,
    /// None selects the native conifer brush supplied by the growth lab.
    shoot: Option<ShootRecipe>,
}

impl GrownSpecies {
    pub fn parts(&self) -> Parts {
        let mut parts = Parts::for_look(&self.look);
        if let Some(recipe) = self.shoot {
            parts.tufts = (0..3).map(|v| recipe.build(v)).collect();
        }
        parts
    }
}

/// Keys accepted by the architectural growth and voxel assembly route.
pub const GROWN_NAMES: [&str; 15] = [
    "pine", "upland_spruce", "scots_pine", "juniper", "mountain_birch", "dwarf_birch",
    "desert_shrub", "saltbush", "kapok", "african_mahogany", "forest_coffee", "raffia_palm",
    "woolly_willow", "white_broom", "forest_dracaena",
];

/// Architectural rules for the woody species used by the desert, Norwegian
/// uplands and African rainforest. The explicit pine and spruce models
/// also supply the reference conifers used by the terrain flight demo.
pub fn grown(name: &str) -> Option<GrownSpecies> {
    let recipe = super::named(name)?;
    let (seed, foliage, wood) = (recipe.id, recipe.foliage_colour, recipe.wood_colour);
    let native = |rules, look, age| GrownSpecies { rules, look, age, seed: 1, shoot: None };
    let authored = |rules, age, shoot| GrownSpecies {
        rules, age, seed, shoot: Some(shoot),
        look: Look { needles: foliage, wood, ..Look::default() },
    };
    let leafy = |rules, age, scale| authored(rules, age, ShootRecipe::Broadleaf(scale));
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
            needles: foliage, wood,
            needle_length: 0.135, needle_width: 0.0022, brush_reach: 0.19,
            brush_needles: 65, tip_spread: 0.55, ..Look::jeffrey_pine()
        }, 28),
        "upland_spruce" => native(Species::norway_spruce(), Look::norway_spruce(), 18),
        "juniper" => native(Species {
            height: 1.25, pace: 6.5, limb_step: 0.15, angle: [1.3, 1.55], rise: 0.015,
            needle_years: 4, ..shrub
        }, Look {
            wood, needles: foliage, brush_needles: 44,
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
        "kapok" => authored(Species {
            height: 34.0, pace: 17.0, first_whorl: 9, whorl: [2, 3],
            limb_step: 1.155, angle: [1.47, 1.55], rise: 0.055, droop: 0.025,
            wood: 0.017, live_crown: 0.38, lift_years: 18.0, dead_years: 4, ..broadleaf
        }, 17, ShootRecipe::Kapok),
        "african_mahogany" => authored(Species {
            height: 22.0, pace: 17.0, first_whorl: 4, whorl: [1, 3],
            limb_step: 0.60, laterals: 1.1, wood: 0.015,
            needle_years: 3, ..broadleaf
        }, 17, ShootRecipe::Mahogany),
        "forest_coffee" => authored(Species {
            height: 2.9, pace: 7.5, whorl: [2, 2], limb_step: 0.16,
            angle: [1.37, 1.54], rise: -0.03, droop: 0.09, needle_years: 4,
            wood: 0.0036, ..shrub
        }, 9, ShootRecipe::Coffee),
        // Young fronds rise from the crown; as their axes extend they bend
        // down. The leaflets on a frond sit close to one plane.
        "raffia_palm" => authored(Species {
            height: 0.92, pace: 8.0, first_whorl: 4, whorl: [1, 2], max_order: 1,
            limb_step: 0.135, angle: [1.20, 1.25], rise: 0.02, droop: 0.022,
            needle_years: 1, limb_wood: 0.23, wood: 0.012, plumes: 1.0,
            live_crown: 0.80, lift_years: 12.0, ..shrub
        }, 11, ShootRecipe::Raffia),
        // Short early leader growth sets several stout limbs near the ground.
        // Older low limbs carry leafy side shoots into the outer crown.
        "woolly_willow" => authored(Species {
            height: 1.14, pace: 5.6, shape: 2.25, first_whorl: 1, whorl: [3, 4],
            limb_step: 0.10, angle: [1.27, 1.39], rise: 0.15, side_length: 0.69,
            laterals: 0.78, needle_years: 5, plumes: 1.14, crowding: 13,
            tip_radius: 0.0016, wood: 0.0019, limb_wood: 0.78, ..shrub
        }, 9, ShootRecipe::WoollyWillow),
        // A short basal axis gives way to several nearly co-dominant ascending
        // scaffolds. The youngest green branchlets arch from their outer limbs.
        "white_broom" => authored(Species {
            height: 1.05, pace: 6.1, shape: 1.95, first_whorl: 1, whorl: [1, 2],
            limb_step: 0.224, angle: [0.60, 0.60], rise: 0.035, droop: 0.024,
            laterals: 1.28, side_length: 0.85, needle_years: 5, crowding: 10,
            tip_radius: 0.00125, wood: 0.0032, order_wood: [0.88, 0.34, 0.0, 0.0],
            ..shrub
        }, 11, ShootRecipe::Broom),
        "forest_dracaena" => authored(Species {
            // The short first internode divides into slender ascending canes.
            height: 2.29, pace: 6.7, shape: 1.92, first_whorl: 1, whorl: [2, 3],
            limb_step: 0.138, angle: [0.52, 0.84], rise: 0.065, max_order: 2,
            laterals: 0.44, side_length: 0.59, limb_wood: 0.84, tip_radius: 0.0019,
            wood: 0.0013, needle_years: 5,
            ..shrub
        }, 9, ShootRecipe::Dracaena),
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
                if s.shoot.is_some() {
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
    fn all_catalog_names_resolve_and_biome_undergrowth_grows_between_ages_and_varies_by_seed() {
        let known: BTreeSet<_> = super::super::NAMES.into_iter().filter(|name| grown(name).is_some()).collect();
        assert_eq!(known, GROWN_NAMES.into_iter().collect());
        assert_eq!(known.len(), GROWN_NAMES.len());
        for name in ["woolly_willow", "white_broom", "forest_dracaena"] {
            let model = grown(name).unwrap();
            for seed in [3, 19, 27] {
                let juvenile = plant_growth::grow(&model.rules, 2, seed);
                let mature = plant_growth::grow(&model.rules, model.age, seed);
                assert!(juvenile.height() < mature.height() && juvenile.tufts.len() < mature.tufts.len(), "{name}");
                assert_ne!(mature.segments, plant_growth::grow(&model.rules, model.age, seed + 1).segments);
            }
        }
    }

    #[test]
    fn white_broom_forks_at_the_base_into_several_woody_scaffolds() {
        let broom = grown("white_broom").unwrap();
        for seed in [broom.seed, 3, 19, 74, 91] {
            let plant = plant_growth::grow(&broom.rules, broom.age, seed);
            let primary = plant.segments.iter().filter(|stem| stem.order == 1
                && stem.parent.is_some_and(|parent| plant.segments[parent as usize].order == 0));
            let basal: Vec<_> = primary.filter(|stem| stem.from.y < 0.20).collect();
            assert!(basal.len() >= 3 && basal.iter().any(|stem| stem.from.y < 0.06), "seed {seed}");
            assert!(basal.iter().any(|stem| stem.radius[0] > 0.6 * plant.trunk_radius()), "seed {seed}");
        }
    }

    #[test]
    fn broom_and_palm_keep_their_lowest_foliage_above_the_roots() {
        for name in ["white_broom", "raffia_palm"] {
            let model = grown(name).unwrap();
            let parts = model.parts();
            for seed in [model.seed, 13, 65, 74, 91] {
                let grown = plant_growth::grow(&model.rules, model.age, seed);
                let (assembly, _) = plant_growth_assembly::assemble(&grown, &parts, model.look, seed,
                    plant_growth_assembly::limits()).unwrap();
                let branches = assembly.branch_transforms().unwrap();
                for instance in &assembly.instances {
                    let part = &assembly.parts[instance.part as usize];
                    if parts.tufts.iter().any(|tip| tip.id == part.id) {
                        let transform = branches[instance.branch as usize] * instance.rest;
                        assert!(part.vertices.iter().all(|v| transform.transform_point3(v.pos.into()).y >= 0.0),
                            "{name} seed {seed}: foliage under the roots");
                    }
                }
            }
        }
    }

    #[test]
    fn mature_woolly_willow_forks_low_and_keeps_leafy_lower_limbs() {
        let model = grown("woolly_willow").unwrap();
        for seed in [model.seed, 3, 19, 47, 91] {
            let plant = plant_growth::grow(&model.rules, model.age, seed);
            let basal_limbs = plant.segments.iter().filter(|s| s.order == 1 && s.from.y < 0.10
                && s.parent.is_some_and(|p| plant.segments[p as usize].order == 0)).count();
            assert!(basal_limbs >= 3, "seed {seed}: too few basal limbs");
            assert!(plant.tufts.iter().any(|t| t.at.y < plant.height() * 0.25),
                "seed {seed}: bare lower quarter of the bush");
        }
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
