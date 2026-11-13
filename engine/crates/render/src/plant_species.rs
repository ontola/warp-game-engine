//! The engine's plant species: recipes (plant_recipe::WoodyRecipe) a game
//! can grow without authoring its own. The trees and shrubs follow
//! Sideways' approved recipes (copied, as the engine does not depend on a
//! game). Biomes select from this catalogue for both trees and understory.
use crate::plant_recipe::{CrownProfile, FoliageKind, WoodyRecipe};

const NEEDLE: [f32; 3] = [0.065, 0.135, 0.045];
const LEAF: [f32; 3] = [0.11, 0.20, 0.055];
const BARK: [f32; 3] = [0.16, 0.105, 0.062];

/// Every species by name.
pub const NAMES: [&str; 14] = [
    "spruce", "pine", "birch", "broadleaf", "willow", "juniper", "shrub", "dwarf_birch", "desert_shrub",
    "saltbush", "mountain_birch", "kapok", "african_mahogany", "forest_coffee",
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
        // Low silvery pads of small leaves, below the desert's taller twiggy shrubs.
        "saltbush" => shrub(0x5341_4c54_4255, 0.55, 0.03, 0.72, CrownProfile::Spreading, 3, 0.12, 0.012, FoliageKind::Broadleaf, 14, 0.41,
            [0.29, 0.31, 0.20], [0.28, 0.22, 0.15]),
        // Shorter and wider than the temperate birch, with an open, light crown.
        "mountain_birch" => tree(0x4d54_4249_5243, 8.0, 0.37, 2.25, CrownProfile::Rounded, 5, 4, 0.59, 0.042, 0.49,
            FoliageKind::Broadleaf, 23, 0.82, 0.54, [0.14, 0.19, 0.077], [0.51, 0.44, 0.35]),
        // Young kapok: clear bole, high umbrella of heavy lateral limbs.
        "kapok" => tree(0x4b41_504f_4b, 16.5, 0.68, 4.3, CrownProfile::Umbrella, 5, 4, 0.10, 0.12, 0.77,
            FoliageKind::Broadleaf, 41, 1.3, 0.66, [0.075, 0.19, 0.064], [0.19, 0.18, 0.135]),
        // The lower forest canopy: close-set, rounded crowns of deep green leaves.
        "african_mahogany" => tree(0x4b48_4159_41, 13.0, 0.46, 3.35, CrownProfile::Rounded, 5, 5, 0.39, 0.085, 0.66,
            FoliageKind::Broadleaf, 43, 1.14, 0.63, [0.055, 0.155, 0.052], [0.19, 0.105, 0.07]),
        // Shade-tolerant understory with low horizontal tiers of broad leaves.
        "forest_coffee" => shrub(0x434f_4646_4545, 1.85, 0.045, 0.95, CrownProfile::Conical, 4, 0.06, 0.016,
            FoliageKind::Broadleaf, 9, 0.85, [0.045, 0.14, 0.052], [0.12, 0.095, 0.055]),
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
}
