//! Shared tree and shrub species. `grown` provides architectural growth
//! rules, stand age and foliage parts for the desert, Norway and rainforest.
//! `named` provides the woody recipes used by the established landscapes.
use crate::plant_recipe::{CrownProfile, FoliageKind, WoodyRecipe};

mod growth;
pub use growth::{grown, GrownSpecies};

const NEEDLE: [f32; 3] = [0.065, 0.135, 0.045];
const LEAF: [f32; 3] = [0.11, 0.20, 0.055];
const BARK: [f32; 3] = [0.16, 0.105, 0.062];

/// Every species by name.
pub const NAMES: [&str; 16] = [
    "spruce", "pine", "birch", "broadleaf", "willow", "juniper", "shrub", "dwarf_birch", "desert_shrub",
    "saltbush", "upland_spruce", "mountain_birch", "kapok", "african_mahogany", "forest_coffee", "raffia_palm",
];

/// A species' recipe by name.
pub fn named(name: &str) -> Option<WoodyRecipe> {
    let tree = |id, height, crown_base, crown_radius, crown_profile, tiers, branches_per_tier, branch_rise, branch_radius, stiffness,
                foliage, sprays_per_branch, spray_scale, spray_axial_bias, foliage_colour, wood_colour| WoodyRecipe {
        id, height, trunk_radius: 0.22, crown_base, crown_radius, crown_profile, tiers, branches_per_tier, branch_rise, branch_radius,
        stiffness, foliage, sprays_per_branch, spray_scale, spray_axial_bias, foliage_colour, wood_colour, protected_trunk_height: 5.5 };
    let shrub = |id, height, trunk_radius, crown_radius, crown_profile, tiers, branch_rise, branch_radius, foliage, sprays_per_branch,
                 spray_scale, foliage_colour, wood_colour| WoodyRecipe {
        id, height, trunk_radius, crown_base: 0.1, crown_radius, crown_profile, tiers, branches_per_tier: 5, branch_rise, branch_radius,
        stiffness: 0.45, foliage, sprays_per_branch, spray_scale, spray_axial_bias: 0.5, foliage_colour, wood_colour,
        protected_trunk_height: 0.0 };
    Some(match name {
        "spruce" => tree(0x5350_5255_4345, 16.0, 0.18, 2.9, CrownProfile::Conical, 10, 3, 0.04, 0.065, 0.72, FoliageKind::Needle, 84, 1.15, 0.40, NEEDLE, BARK),
        "pine" => tree(0x5049_4e45, 14.5, 0.55, 3.10, CrownProfile::Umbrella, 6, 5, 0.18, 0.085, 0.64, FoliageKind::Needle, 78, 1.12, 0.65, NEEDLE, BARK),
        "birch" => tree(0x4249_5243_48, 15.5, 0.36, 1.80, CrownProfile::Columnar, 6, 5, 0.85, 0.045, 0.45, FoliageKind::Broadleaf, 48, 1.00, 0.50, LEAF, [0.45, 0.43, 0.36]),
        "broadleaf" => tree(0x4252_4f41_444c, 12.0, 0.40, 4.00, CrownProfile::Rounded, 7, 4, 0.15, 0.080, 0.57, FoliageKind::Broadleaf, 56, 1.15, 0.55, LEAF, BARK),
        // A pollard willow along a ditch: a short thick trunk, a ball of
        // upright shoots with narrow grey-green leaves.
        "willow" => WoodyRecipe { trunk_radius: 0.32, protected_trunk_height: 2.2,
            ..tree(0x5749_4c4c_4f57, 7.0, 0.35, 2.6, CrownProfile::Rounded, 5, 6, 1.1, 0.04, 0.4, FoliageKind::Broadleaf, 52, 0.9, 0.7,
                [0.13, 0.18, 0.09], [0.22, 0.19, 0.15]) },
        "juniper" => WoodyRecipe { crown_base: 0.12, branches_per_tier: 5, spray_axial_bias: 0.5, stiffness: 0.48,
            ..shrub(0x4a55_4e49_5045_52, 1.15, 0.065, 1.10, CrownProfile::Spreading, 4, 0.30, 0.018, FoliageKind::Needle, 36, 0.52, NEEDLE, BARK) },
        "shrub" => WoodyRecipe { crown_base: 0.08, stiffness: 0.40,
            ..shrub(0x5348_5255_42, 1.15, 0.045, 0.80, CrownProfile::Rounded, 4, 0.38, 0.012, FoliageKind::Broadleaf, 12, 0.60, LEAF, BARK) },
        // Tundra: knee-high, spreading, small dark leaves.
        "dwarf_birch" => shrub(0x4457_4152_4642, 0.6, 0.03, 0.7, CrownProfile::Spreading, 3, 0.25, 0.01, FoliageKind::Broadleaf, 18, 0.45,
            [0.09, 0.13, 0.05], [0.25, 0.17, 0.12]),
        // Desert: a sparse open bush of grey-olive twigs (creosote, saltbush).
        "desert_shrub" => shrub(0x4445_5345_5254, 1.3, 0.04, 0.9, CrownProfile::Rounded, 4, 0.6, 0.012, FoliageKind::Needle, 10, 0.5,
            [0.16, 0.17, 0.09], [0.3, 0.25, 0.18]),
        // Pale low mats on the sand. Short upright sprays at the ends of
        // widely spreading stems distinguish these from the taller dry scrub.
        "saltbush" => WoodyRecipe { branches_per_tier: 4, spray_axial_bias: 0.30,
            ..shrub(0x5341_4c54_4255, 0.42, 0.03, 0.95, CrownProfile::Spreading, 2, 0.22, 0.017,
                FoliageKind::Broadleaf, 12, 0.42, [0.30, 0.32, 0.225], [0.22, 0.17, 0.105]) },
        // A skirt of low, downturned boughs tapering to a narrow leader.
        // The terminal sprays point along the boughs and remain blue-green.
        "upland_spruce" => tree(0x5550_5350_5255, 11.0, 0.10, 3.35, CrownProfile::Conical, 7, 3, -0.29, 0.063, 0.73,
            FoliageKind::Needle, 40, 1.28, 0.65, [0.035, 0.089, 0.061], [0.13, 0.088, 0.06]),
        // Shorter and wider than the temperate birch, with an open, light crown.
        "mountain_birch" => tree(0x4d54_4249_5243, 7.7, 0.28, 2.1, CrownProfile::Rounded, 4, 3, 0.73, 0.048, 0.49,
            FoliageKind::Broadleaf, 19, 0.83, 0.41, [0.16, 0.205, 0.088], [0.51, 0.46, 0.39]),
        // An emergent kapok: a clear grey bole with an elevated, wide crown.
        // Most of its limbs carry leaves in the top quarter of the tree.
        "kapok" => tree(0x4b41_504f_4b, 20.5, 0.69, 5.4, CrownProfile::Umbrella, 3, 4, 0.22, 0.16, 0.77,
            FoliageKind::Broadleaf, 42, 1.48, 0.56, [0.073, 0.18, 0.057], [0.17, 0.15, 0.105]),
        // Mahogany foliage starts much lower, on spreading scaffold limbs.
        // Its dark rounded crown closes the gap under the kapok canopy.
        "african_mahogany" => tree(0x4b48_4159_41, 12.8, 0.32, 4.25, CrownProfile::Rounded, 4, 4, 0.21, 0.105, 0.66,
            FoliageKind::Broadleaf, 39, 1.40, 0.39, [0.043, 0.148, 0.061], [0.15, 0.083, 0.054]),
        // Coffee forms a lower, rounded thicket with broad horizontal sprays.
        "forest_coffee" => WoodyRecipe { branches_per_tier: 3, spray_axial_bias: 0.34,
            ..shrub(0x434f_4646_4545, 1.9, 0.045, 1.12, CrownProfile::Rounded, 3, 0.06, 0.022,
                FoliageKind::Broadleaf, 11, 0.89, [0.04, 0.13, 0.041], [0.10, 0.078, 0.045]) },
        // A juvenile raffia palm: one whorl of long radial fronds close
        // to the ground, the leaflet sprays spreading laterally along each rachis.
        "raffia_palm" => WoodyRecipe { crown_base: 0.78, branches_per_tier: 8, spray_axial_bias: 0.15,
            ..shrub(0x5241_4646_4941, 0.65, 0.11, 2.1, CrownProfile::Spreading, 1, 0.34, 0.038,
                FoliageKind::Broadleaf, 12, 0.77, [0.111, 0.204, 0.054], [0.17, 0.14, 0.069]) },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_species_builds() {
        use crate::plant_recipe::{self, BuildOptions};
        let mut ids = std::collections::BTreeSet::new();
        for name in super::NAMES {
            let recipe = super::named(name).unwrap();
            assert!(ids.insert(recipe.id), "{name}: id reused");
            plant_recipe::build(&recipe, BuildOptions::new(1), plant_recipe::recipe_limits()).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        assert!(super::named("baobab").is_none());
    }

    #[test]
    fn rainforest_recipes_fill_distinct_canopy_and_understory_layers() {
        use crate::plant_recipe::{build, recipe_limits, BuildOptions};
        for seed in [1, 7, 31] {
            let dimensions = |name| {
                let plant = build(&super::named(name).unwrap(), BuildOptions::new(seed), recipe_limits()).unwrap();
                let [lo, hi] = plant.bounds().unwrap();
                ((hi[0] - lo[0]).max(hi[2] - lo[2]), hi[1] - lo[1])
            };
            let (_, emergent) = dimensions("kapok");
            let (_, canopy) = dimensions("african_mahogany");
            let (_, coffee) = dimensions("forest_coffee");
            let (span, palm) = dimensions("raffia_palm");
            assert!(emergent > canopy * 1.4);
            assert!(canopy > coffee * 4.0);
            assert!(span > palm * 1.6, "young palm has a low fan of fronds");
        }
    }
}
