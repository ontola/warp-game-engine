//! Growth rules and source foliage shared by the terrain viewer and games.
use super::{basal, creeping, shoots::ShootRecipe};
use crate::{
    foliage_assembly::{Assembly, Limits},
    plant_growth::Species,
    plant_growth_assembly::{self, Counts, Look, Parts},
};

pub(super) fn seed_unit(seed: u64, index: u32, channel: u32) -> f32 {
    let mut h = seed
        ^ u64::from(index).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ u64::from(channel).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((h ^ (h >> 31)) >> 40) as f32 / 16_777_216.0
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Habit {
    Woody,
    Basal(basal::BasalHabit),
    Creeping(creeping::MatHabit),
}

#[derive(Clone, Copy, Debug)]
pub struct GrownSpecies {
    pub rules: Species,
    pub look: Look,
    /// Representative stand age, in years.
    pub age: u32,
    pub seed: u64,
    /// A woody species without an authored shoot uses the growth lab's needle brush.
    shoot: Option<ShootRecipe>,
    pub(crate) habit: Habit,
}

impl GrownSpecies {
    pub fn parts(&self) -> Parts {
        let mut parts = Parts::for_look(&self.look);
        if matches!(self.shoot, Some(ShootRecipe::Juniper)) {
            let a = super::shoots::juniper::shoot(0);
            let b = super::shoots::juniper::shoot(1);
            parts.tufts = vec![a.needles, b.needles, a.wood, a.bands, b.bands];
        } else if matches!(self.shoot, Some(ShootRecipe::ScotsPine)) {
            parts.tufts = super::scots_pine::parts(&self.look);
        } else if matches!(self.habit, Habit::Creeping(_)) {
            parts.tufts = creeping::dryas_parts();
        } else if let Some(recipe) = self.shoot {
            parts.tufts = (0..3).map(|v| recipe.build(v)).collect();
        }
        parts
    }

    /// Build the same source assembly for the viewer, game previews and voxel cooks.
    pub fn assemble(&self, parts: &Parts, age: u32, seed: u64, limits: Limits) -> Result<(Assembly, Counts), String> {
        match self.habit {
            Habit::Basal(habit) => habit.assemble(&self.rules, self.look, parts, age, seed, limits),
            Habit::Creeping(habit) => habit.assemble(&self.rules, self.look, parts, age, seed, limits),
            Habit::Woody => {
                let mut plant = crate::plant_growth::grow(&self.rules, age, seed);
                if matches!(self.shoot, Some(ShootRecipe::Juniper)) {
                    super::shoots::juniper::settle_limbs(&mut plant, seed);
                } else if matches!(self.shoot, Some(ShootRecipe::Crowberry)) {
                    // The oldest axis trails along the ground. Keep every attachment
                    // on its supporting wood while the leafy tips turn up.
                    let bearing = seed_unit(seed, 0, 21) * std::f32::consts::TAU;
                    let trail = glam::Vec3::new(bearing.cos(), 0.21, bearing.sin());
                    let prostrate = |p: glam::Vec3| glam::Vec3::new(p.x, 0.0, p.z) + trail * p.y;
                    for s in &mut plant.segments {
                        s.from = prostrate(s.from);
                        s.to = prostrate(s.to);
                    }
                    for tip in &mut plant.tufts {
                        tip.at = prostrate(tip.at);
                        tip.axis = (tip.axis + glam::Vec3::Y * 1.35).normalize();
                    }
                }
                use crate::scene::{Material, THIN_FOLIAGE, TWO_SIDED};
                let stripe = [Material { tint: JUNIPER_BAND, layer: -1, roughness: 0.88,
                    flags: THIN_FOLIAGE | TWO_SIDED, scale: 1.0, metal: 0.0 }];
                let layers = match self.shoot {
                    Some(ShootRecipe::Juniper) => Some(plant_growth_assembly::ShootLayers {
                        variants: &[&[(2, 2), (3, 3)], &[(2, 2), (4, 3)]], materials: &stripe,
                    }),
                    Some(ShootRecipe::ScotsPine) => Some(plant_growth_assembly::ShootLayers {
                        variants: &[&[(2, 2)], &[(3, 2)]], materials: &[],
                    }),
                    _ => None,
                };
                plant_growth_assembly::assemble_shoots(&plant, parts, self.look, seed, limits, layers)
            }
        }
    }

    pub fn creeping_mat(&self) -> bool {
        matches!(self.habit, Habit::Creeping(_))
    }

    pub fn layered_shoot(&self) -> bool {
        self.creeping_mat() || matches!(self.shoot, Some(ShootRecipe::Juniper | ShootRecipe::ScotsPine))
    }

    /// Keep existing woody and basal cook identities. Custom recipes also
    /// hash their parts; their placement rules belong in the same key.
    pub(crate) fn custom_cook_key(&self) -> Option<String> {
        match self.habit {
            Habit::Woody if matches!(self.shoot, Some(ShootRecipe::Juniper)) => Some(format!("juniper v2 {JUNIPER_BAND:?}")),
            Habit::Woody if matches!(self.shoot, Some(ShootRecipe::ScotsPine)) => Some("paired pine v1".into()),
            Habit::Woody if matches!(self.shoot, Some(ShootRecipe::Crowberry)) => Some("crowberry trailing axes v1".into()),
            Habit::Woody => None,
            Habit::Basal(habit) => Some(format!("basal v1 {habit:?}")),
            Habit::Creeping(habit) => Some(format!("creeping v1 {habit:?}")),
        }
    }
}

const JUNIPER_BAND: [f32; 3] = [0.40, 0.46, 0.40];

/// Keys accepted by the architectural growth and voxel assembly route.
pub const GROWN_NAMES: [&str; 19] = [
    "pine", "upland_spruce", "scots_pine", "juniper", "mountain_birch", "dwarf_birch",
    "desert_shrub", "saltbush", "kapok", "african_mahogany", "forest_coffee", "raffia_palm",
    "woolly_willow", "white_broom", "forest_dracaena", "dune_grass", "forest_fern", "dryas", "crowberry",
];

/// Architectural rules for the woody species used by the desert, Norwegian
/// uplands and African rainforest. The explicit pine and spruce models
/// also supply the reference conifers used by the terrain flight demo.
pub fn grown(name: &str) -> Option<GrownSpecies> {
    let recipe = super::named(name)?;
    let (seed, foliage, wood) = (recipe.id, recipe.foliage_colour, recipe.wood_colour);
    let native = |rules, look, age| GrownSpecies { rules, look, age, seed: 1, shoot: None, habit: Habit::Woody };
    let authored = |rules, age, shoot| GrownSpecies {
        rules, age, seed, shoot: Some(shoot), habit: Habit::Woody,
        look: Look { needles: foliage, wood, ..Look::default() },
    };
    let leafy = |rules, age, twig| authored(rules, age, ShootRecipe::LeafyTwig(twig));
    use super::shoots::leafy_twigs::{DWARF_BIRCH, MOUNTAIN_BIRCH, NITRARIA, SALTBUSH};
    let herb = |habit: basal::BasalHabit, pace, shape, age, shoot| GrownSpecies {
        habit: Habit::Basal(habit),
        ..authored(Species { height: habit.height_m, pace, shape, ..Species::default() }, age, shoot)
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
        "scots_pine" => GrownSpecies { shoot: Some(ShootRecipe::ScotsPine), ..native(Species {
            height: 31.0, pace: 36.0, shape: 1.55, whorl: [1, 4],
            limb_step: 0.34, angle: [1.36, 1.66], rise: 0.038,
            side_length: 0.76, laterals: 1.25, needle_years: 5, live_crown: 0.56, lift_years: 28.0, dead_years: 7,
            tip_radius: 0.0021, order_wood: [0.53, 0.31, 0.19, 0.10],
            ..Species::jeffrey_pine()
        }, Look {
            needles: foliage, wood,
            needle_length: 0.056, needle_width: 0.00115, brush_reach: 0.165,
            brush_needles: 160, tip_spread: 0.51, brush_spread: 1.36, ..Look::jeffrey_pine()
        }, 28) },
        "upland_spruce" => native(Species::norway_spruce(), Look::norway_spruce(), 18),
        // Low Norwegian common juniper: long basal limbs spread sideways,
        // then divide into short ascending sprays that stay green near the ground.
        "juniper" => GrownSpecies {
            rules: Species {
                height: 0.22, pace: 8.2, shape: 1.6, first_whorl: 1, whorl: [3, 4],
                limb_step: 0.041, side_length: 0.60, angle: [1.38, 1.38], rise: 0.18,
                droop: 0.015, laterals: 0.84, max_order: 2, needle_years: 6,
                shade: 0.016, crowding: 38, plumes: 1.13, tip_radius: 0.00058,
                wood: 0.00062, limb_wood: 0.66, order_wood: [0.72, 0.39, 0.22, 0.0],
                ..shrub
            },
            look: Look { wood: [0.125, 0.085, 0.065], needles: [0.037, 0.095, 0.060],
                         ..Look::default() },
            shoot: Some(ShootRecipe::Juniper), habit: Habit::Woody, age: 6, seed: 1,
        },
        "crowberry" => authored(Species {
            height: 0.12, pace: 6.8, shape: 1.8, first_whorl: 1, whorl: [3, 5],
            limb_step: 0.039, side_length: 0.68, angle: [1.48, 1.68], rise: 0.075,
            laterals: 1.02, max_order: 3, needle_years: 6, crowding: 28, plumes: 2.45,
            tip_radius: 0.00050, wood: 0.00038, order_wood: [0.71, 0.44, 0.28, 0.0],
            ..shrub
        }, 8, ShootRecipe::Crowberry),
        "mountain_birch" => leafy(Species {
            height: 5.9, pace: 13.5, shape: 2.3, first_whorl: 1, whorl: [2, 3],
            limb_step: 0.35, laterals: 0.85, side_length: 0.64, crowding: 18,
            angle: [0.64, 0.97], rise: 0.045, live_crown: 0.85, plumes: 2.0,
            tip_radius: 0.002, wood: 0.0052, order_wood: [0.72, 0.35, 0.0, 0.0],
            ..broadleaf
        }, 18, &MOUNTAIN_BIRCH),
        "dwarf_birch" => leafy(Species {
            height: 0.23, pace: 4.6, shape: 1.95, first_whorl: 1, whorl: [3, 4],
            limb_step: 0.071, side_length: 0.44, limb_life: [5, 10], angle: [1.04, 1.12],
            rise: 0.09, droop: 0.001, laterals: 1.12, needle_years: 4, shade: 0.008,
            crowding: 26, plumes: 2.0, tip_radius: 0.0009, wood: 0.0011,
            order_wood: [0.61, 0.26, 0.0, 0.0], ..shrub
        }, 7, &DWARF_BIRCH),
        "desert_shrub" => leafy(Species {
            height: 0.74, pace: 5.3, shape: 2.12, first_whorl: 1, whorl: [2, 4],
            laterals: 0.88, limb_step: 0.185, side_length: 0.49, limb_life: [8, 16],
            angle: [1.06, 1.21], rise: 0.078, droop: 0.006, shade: 0.013,
            crowding: 21, wood: 0.0024, tip_radius: 0.0012, needle_years: 5,
            order_wood: [0.65, 0.31, 0.0, 0.0], ..shrub
        }, 9, &NITRARIA),
        "saltbush" => leafy(Species {
            height: 0.48, pace: 6.5, shape: 1.65, first_whorl: 1, whorl: [3, 4],
            limb_step: 0.077, side_length: 0.30, limb_life: [6, 13], angle: [0.80, 0.98],
            rise: 0.20, droop: 0.001, laterals: 1.12, shade: 0.013, crowding: 26,
            plumes: 1.8, needle_years: 3, wood: 0.0013, tip_radius: 0.001,
            order_wood: [0.60, 0.27, 0.0, 0.0], ..shrub
        }, 6, &SALTBUSH),
        // The emergent sets its first scaffold limbs high on a clean bole.
        // Its spreading crown overtops the shorter, darker mahogany.
        "kapok" => authored(Species {
            height: 34.0, pace: 17.0, first_whorl: 9, whorl: [2, 3],
            limb_step: 1.155, angle: [1.47, 1.55], rise: 0.055, droop: 0.025,
            wood: 0.017, live_crown: 0.38, lift_years: 18.0, dead_years: 4, ..broadleaf
        }, 17, ShootRecipe::KAPOK),
        "african_mahogany" => authored(Species {
            height: 22.0, pace: 17.0, first_whorl: 4, whorl: [1, 3],
            limb_step: 0.60, laterals: 1.1, wood: 0.015,
            needle_years: 3, ..broadleaf
        }, 17, ShootRecipe::MAHOGANY),
        "forest_coffee" => authored(Species {
            height: 2.9, pace: 7.5, whorl: [2, 2], limb_step: 0.16,
            angle: [1.37, 1.54], rise: -0.03, droop: 0.09, needle_years: 6, plumes: 1.35,
            wood: 0.0036, ..shrub
        }, 9, ShootRecipe::COFFEE),
        "raffia_palm" => herb(basal::RAFFIA, 3.9, 1.2, 11, ShootRecipe::Raffia),
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
        "dune_grass" => herb(basal::DRINN, 1.65, 1.0, 5, ShootRecipe::Drinn),
        "forest_fern" => herb(basal::SWORD_FERN, 1.35, 1.07, 4, ShootRecipe::Fern),
        "dryas" => GrownSpecies {
            rules: Species { height: creeping::DRYAS.height_m, pace: 4.5, shape: 1.12, ..shrub },
            look: Look { needles: foliage, wood, ..Look::default() },
            age: 6, seed, shoot: None, habit: Habit::Creeping(creeping::DRYAS),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{biome::Biome, plant_growth, plant_growth_assembly};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn crowberry_keeps_a_low_mature_mat_and_attached_juvenile_foliage() {
        let species = grown("crowberry").unwrap();
        let parts = species.parts();
        for seed in [species.seed, 24] {
            for age in [1, species.age] {
                let (mat, counts) = species.assemble(&parts, age, seed, plant_growth_assembly::limits()).unwrap();
                let [lo, hi] = mat.bounds().unwrap();
                let height = hi[1] - lo[1];
                let width = (hi[0] - lo[0]).max(hi[2] - lo[2]);
                assert!(counts.tufts > 0 && counts.tufts_dropped == 0);
                if age == species.age {
                    assert!(height < 0.16 && width > 3.0 * height);
                }
            }
        }
    }

    #[test]
    fn regional_growth_assemblies_build_with_distinct_foliage_and_canopy_layers() {
        let mut dimensions = BTreeMap::new();
        let mut leaf_ids = BTreeSet::new();
        for region in ["desert", "norway", "rainforest"] {
            for (name, _) in Biome::named(region).unwrap().species() {
                let s = grown(name).unwrap_or_else(|| panic!("{region}: no grown {name}"));
                let parts = s.parts();
                let (assembly, placed) = s.assemble(&parts, s.age, s.seed, plant_growth_assembly::limits())
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(placed.tufts > 0, "{name}");
                assert_eq!(placed.stems == 0, matches!(s.habit, Habit::Basal(_)), "{name}");
                if s.shoot.is_some() {
                    for p in &parts.tufts {
                        assert!(leaf_ids.insert(p.id), "{name}: reused shoot identity");
                        if !matches!(s.shoot, Some(ShootRecipe::ScotsPine)) {
                            assert!(p.vertices.iter().all(|v| v.uv[1] >= 0.0), "{name}: needle brush");
                        }
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
                let (assembly, _) = model.assemble(&parts, model.age, seed, plant_growth_assembly::limits()).unwrap();
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
