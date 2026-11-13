//! One artist palette for a game's (or a biome's) vegetation and ground: the
//! forest (`ForestStyle`), the procedural terrain's ground tints and the
//! meadow's season. `procedural_terrain::vegetation_style` hands it to the
//! terrain shader; the default is the engine's own look.
//!
//! ```ignore
//! let style = VegetationStyle::named("alpine").unwrap_or_default();
//! procedural_terrain::vegetation_style(&mut scene, material, &style);
//! procedural_terrain::far_forest(&mut scene, material, start_m, density, &style.forest);
//! let meadow = MeadowGeometry::cook_windswept(seed, style.meadow_dry_season, budget)?;
//! ```
use crate::forest_field::ForestStyle;

/// Tints of the procedural terrain's ground layers, multiplied with their
/// textures (terrain_surface.wgsl field_surface; linear RGB). Passed to the
/// GPU as is: eight vec4s, each w a scalar of the same layer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GroundStyle {
    /// Grass on dry patches; w: the grass share in moor.
    pub grass_dry: [f32; 4],
    /// Grass in damp hollows and near water; w unused.
    pub grass_lush: [f32; 4],
    /// Moor (heather, moss, bracken); w: how dark the grass in it is.
    pub moor: [f32; 4],
    /// Bare soil on steep banks; w: how much damp ground darkens.
    pub soil: [f32; 4],
    /// Loose stone, after desaturating; w: the stone's saturation kept.
    pub gravel: [f32; 4],
    /// Bedrock, after desaturating; w: the rock's saturation kept.
    pub rock: [f32; 4],
    /// Lying snow; w: seasonal snow on open ground, 0..1 (biome.rs).
    pub snow: [f32; 4],
    /// Regional palette pointer, filled by procedural_terrain::region_ground.
    pub region_palette: f32,
    /// Share of the ground covered in sand (fine aggregate).
    pub sand: f32,
    /// Share covered in broad leaves and exposed clay.
    pub leaf_litter: f32,
    /// Snowpack lying below the tree line, shown beneath the distant canopy.
    pub snowpack: f32,
}

impl Default for GroundStyle {
    fn default() -> Self {
        GroundStyle {
            grass_dry: [0.43, 0.55, 0.23, 0.3],
            grass_lush: [0.32, 0.69, 0.28, 0.0],
            moor: [0.6, 0.72, 0.7, 0.8],
            soil: [0.85, 0.8, 0.75, 0.35],
            gravel: [0.95, 0.97, 1.0, 0.5],
            rock: [1.0, 1.04, 1.1, 0.35],
            snow: [0.84, 0.87, 0.92, 0.0],
            region_palette: 0.0, sand: 0.0, leaf_litter: 0.0, snowpack: 0.0,
        }
    }
}

/// The whole palette.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VegetationStyle {
    pub forest: ForestStyle,
    pub ground: GroundStyle,
    /// The meadow's season, 0 fresh to 1 dry: its blades are cooked with it
    /// (`MeadowGeometry::cook_*`'s `dry_season`).
    pub meadow_dry_season: f32,
}

impl VegetationStyle {
    /// The engine's palettes: "default", "lowland" (lush, broadleaf mixed in)
    /// and "alpine" (dry grass, cool grey stone, dark spruce).
    pub fn named(name: &str) -> Option<Self> {
        let base = Self::default();
        match name {
            "default" => Some(base),
            "lowland" => Some(Self {
                forest: ForestStyle { far: [0.07, 0.105, 0.035, 15.0], crown: [3.2, 0.375, 1.6, 0.4], ..base.forest },
                ground: GroundStyle { grass_dry: [0.4, 0.58, 0.22, 0.3], grass_lush: [0.28, 0.72, 0.26, 0.0], ..base.ground },
                meadow_dry_season: 0.0,
            }),
            "alpine" => Some(Self {
                forest: ForestStyle { needles: [0.042, 0.085, 0.03, 0.5], far: [0.07, 0.065, 0.035, 15.0], crown: [3.2, 0.375, 1.6, 0.1], ..base.forest },
                ground: GroundStyle {
                    grass_dry: [0.5, 0.52, 0.24, 0.4],
                    grass_lush: [0.36, 0.6, 0.27, 0.0],
                    gravel: [0.92, 0.96, 1.02, 0.35],
                    rock: [0.96, 1.02, 1.1, 0.25],
                    ..base.ground
                },
                meadow_dry_season: 0.4,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_terrain_shaders_own_look() {
        // terrain_surface.wgsl FIELD_GROUND_DEFAULT must hold these values.
        let src = include_str!("shaders/terrain_surface.wgsl");
        let g = GroundStyle::default();
        let fields: [[f32; 4]; 8] = bytemuck::cast(g);
        for v in fields {
            let text = format!("vec4<f32>({:?}, {:?}, {:?}, {:?})", v[0], v[1], v[2], v[3]);
            assert!(src.contains(&text), "terrain_surface.wgsl lacks {text}");
        }
        assert_eq!(VegetationStyle::named("default"), Some(VegetationStyle::default()));
        assert!(VegetationStyle::named("lowland").is_some() && VegetationStyle::named("alpine").is_some());
        assert!(VegetationStyle::named("mars").is_none());
    }
}
