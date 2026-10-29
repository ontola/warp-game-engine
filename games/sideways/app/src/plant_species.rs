//! Sideways' authored woody species catalogue. Labels describe stylized
//! silhouettes, not botanically measured or scanned species. Every entry uses
//! the same detailed spray library; only placements and species palettes differ.
use render::{
    foliage_assembly::Assembly,
    plant_recipe::{self, BuildOptions, CrownProfile, FoliageKind, WoodyRecipe},
};

#[derive(Clone, Copy, Debug)]
pub struct Species {
    pub id: &'static str,
    pub name: &'static str,
    pub is_tree: bool,
    pub recipe: WoodyRecipe,
}

const NEEDLE: [f32; 3] = [0.065, 0.135, 0.045];
const LEAF: [f32; 3] = [0.11, 0.20, 0.055];
const BARK: [f32; 3] = [0.16, 0.105, 0.062];

const SPECIES: [Species; 6] = [
    Species {
        id: "spruce",
        name: "Layered fir",
        is_tree: true,
        recipe: WoodyRecipe {
            id: 0x5350_5255_4345,
            height: 16.0,
            trunk_radius: 0.22,
            crown_base: 0.18,
            crown_radius: 2.9,
            crown_profile: CrownProfile::Conical,
            tiers: 10,
            branches_per_tier: 3,
            branch_rise: 0.04,
            branch_radius: 0.065,
            stiffness: 0.72,
            foliage: FoliageKind::Needle,
            sprays_per_branch: 84,
            spray_scale: 1.15,
            spray_axial_bias: 0.40,
            foliage_colour: NEEDLE,
            wood_colour: BARK,
            protected_trunk_height: 5.5,
        },
    },
    Species {
        id: "pine",
        name: "Open pine",
        is_tree: true,
        recipe: WoodyRecipe {
            id: 0x5049_4e45,
            height: 14.5,
            trunk_radius: 0.22,
            crown_base: 0.55,
            crown_radius: 3.10,
            crown_profile: CrownProfile::Umbrella,
            tiers: 6,
            branches_per_tier: 5,
            branch_rise: 0.18,
            branch_radius: 0.085,
            stiffness: 0.64,
            foliage: FoliageKind::Needle,
            sprays_per_branch: 78,
            spray_scale: 1.12,
            spray_axial_bias: 0.65,
            foliage_colour: NEEDLE,
            wood_colour: BARK,
            protected_trunk_height: 5.5,
        },
    },
    Species {
        id: "juniper",
        name: "Spreading juniper",
        is_tree: false,
        recipe: WoodyRecipe {
            id: 0x4a55_4e49_5045_52,
            height: 1.15,
            trunk_radius: 0.065,
            crown_base: 0.12,
            crown_radius: 1.10,
            crown_profile: CrownProfile::Spreading,
            tiers: 4,
            branches_per_tier: 5,
            branch_rise: 0.30,
            branch_radius: 0.018,
            stiffness: 0.48,
            foliage: FoliageKind::Needle,
            sprays_per_branch: 36,
            spray_scale: 0.52,
            spray_axial_bias: 0.50,
            foliage_colour: NEEDLE,
            wood_colour: BARK,
            protected_trunk_height: 0.0,
        },
    },
    Species {
        id: "birch",
        name: "Columnar broadleaf",
        is_tree: true,
        recipe: WoodyRecipe {
            id: 0x4249_5243_48,
            height: 15.5,
            trunk_radius: 0.22,
            crown_base: 0.36,
            crown_radius: 1.80,
            crown_profile: CrownProfile::Columnar,
            tiers: 6,
            branches_per_tier: 5,
            branch_rise: 0.85,
            branch_radius: 0.045,
            stiffness: 0.45,
            foliage: FoliageKind::Broadleaf,
            sprays_per_branch: 48,
            spray_scale: 1.00,
            spray_axial_bias: 0.50,
            foliage_colour: LEAF,
            wood_colour: [0.45, 0.43, 0.36],
            protected_trunk_height: 5.5,
        },
    },
    Species {
        id: "broadleaf",
        name: "Rounded broadleaf",
        is_tree: true,
        recipe: WoodyRecipe {
            id: 0x4252_4f41_444c,
            height: 12.0,
            trunk_radius: 0.22,
            crown_base: 0.40,
            crown_radius: 4.00,
            crown_profile: CrownProfile::Rounded,
            tiers: 7,
            branches_per_tier: 4,
            branch_rise: 0.15,
            branch_radius: 0.080,
            stiffness: 0.57,
            foliage: FoliageKind::Broadleaf,
            sprays_per_branch: 56,
            spray_scale: 1.15,
            spray_axial_bias: 0.55,
            foliage_colour: LEAF,
            wood_colour: BARK,
            protected_trunk_height: 5.5,
        },
    },
    Species {
        id: "shrub",
        name: "Rounded leafy shrub",
        is_tree: false,
        recipe: WoodyRecipe {
            id: 0x5348_5255_42,
            height: 1.15,
            trunk_radius: 0.045,
            crown_base: 0.08,
            crown_radius: 0.80,
            crown_profile: CrownProfile::Rounded,
            tiers: 4,
            branches_per_tier: 5,
            branch_rise: 0.38,
            branch_radius: 0.012,
            stiffness: 0.40,
            foliage: FoliageKind::Broadleaf,
            sprays_per_branch: 12,
            spray_scale: 0.60,
            spray_axial_bias: 0.50,
            foliage_colour: LEAF,
            wood_colour: BARK,
            protected_trunk_height: 0.0,
        },
    },
];

pub fn catalogue() -> &'static [Species] {
    &SPECIES
}
pub fn find(id: &str) -> Result<&'static Species, String> {
    SPECIES
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("unknown plant species '{id}'"))
}
pub fn build(id: &str, seed: u64) -> Result<Assembly, String> {
    build_with_options(id, BuildOptions::new(seed))
}
pub fn build_with_options(id: &str, options: BuildOptions) -> Result<Assembly, String> {
    plant_recipe::build(&find(id)?.recipe, options, plant_recipe::recipe_limits())
}
pub fn metadata(id: &str, seed: u64) -> Result<serde_json::Value, String> {
    let s = find(id)?;
    let a = build(id, seed)?;
    Ok(
        serde_json::json!({"id":s.id,"name":s.name,"is_tree":s.is_tree,
        "recipe_id":s.recipe.id,"seed":seed,"nominal_height_m":s.recipe.height,
        "nominal_crown_radius_m":s.recipe.crown_radius,"bounds":a.bounds()?,
        "primitive_library_meshes":a.parts.len(),"branch_poses":a.branches.len(),
        "placements":a.instances.len(),"spray_pairs":s.recipe.tiers*s.recipe.branches_per_tier*s.recipe.sprays_per_branch,
        "expanded_triangles":a.instances.iter().map(|i|a.parts[i.part as usize].indices.len()/3).sum::<usize>(),
        "collision_trunk_radius_m":if s.is_tree {Some(s.recipe.trunk_radius)} else {None},
        "collision_trunk_y_m":if s.is_tree {Some([-0.5,s.recipe.protected_trunk_height])} else {None},
        "source":"shared detailed woody spray assemblies v2","botanical_accuracy":false,
        "growth_execution":"static recipe cook; bounded runtime shared meshes + GPU branch poses"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_is_bounded_shared_and_has_distinct_silhouettes() {
        let mut ids = std::collections::BTreeSet::new();
        let mut bounds = std::collections::BTreeSet::new();
        let first = build(SPECIES[0].id, 11).unwrap();
        for s in catalogue() {
            assert!(ids.insert(s.id));
            let a = build(s.id, 11).unwrap();
            a.validate().unwrap();
            assert!(a.branches.len() <= 64);
            assert_eq!(a.parts.len(), 14);
            for (a, b) in a.parts.iter().zip(&first.parts) {
                assert_eq!(a.id, b.id);
                assert_eq!(a.indices, b.indices);
                for (a, b) in a.vertices.iter().zip(&b.vertices) {
                    assert_eq!(a.pos, b.pos);
                    assert_eq!(a.normal, b.normal);
                    assert_eq!(a.uv, b.uv);
                }
            }
            let b = a.bounds().unwrap();
            assert!(bounds.insert(b.map(|v| v.map(f32::to_bits))));
            if s.is_tree {
                assert_eq!(s.recipe.trunk_radius, 0.22);
                assert_eq!(s.recipe.protected_trunk_height, 5.5);
                assert_eq!(a.instances[0].rest.w_axis.y, -0.5);
                assert!(a.instances[0].rest.y_axis.y * 0.8 - 0.5 >= 5.5);
            } else {
                assert!(b[1][1] < 2.5);
            }
        }
        let dimensions = |id| {
            let b = build(id, 11).unwrap().bounds().unwrap();
            [
                (b[1][0] - b[0][0]).max(b[1][2] - b[0][2]),
                b[1][1] - b[0][1],
            ]
        };
        assert!(dimensions("birch")[0] < dimensions("broadleaf")[0] * 0.7);
        assert!(dimensions("juniper")[1] < dimensions("pine")[1] * 0.3);
        assert!(build("unknown", 1).is_err());
    }
}
