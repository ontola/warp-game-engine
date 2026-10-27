//! Which optional parts of the shade shader a scene can reach. The main
//! shade pipeline is specialised with these as WGSL override constants, so a
//! scene without cars, terrain, grass, tracks or foliage never compiles that
//! code (about half the shade pipeline's compile time for a plain scene).
//!
//! Every feature is derived from data that is fixed for the renderer's life:
//! the scene's materials and the experimental resources chosen at
//! construction. A disabled feature is exactly the path the shader would
//! have taken anyway, so images do not change. `KERN_SHADE_FEATURES=all`
//! restores the unspecialised pipeline for comparisons.
use crate::scene::{Scene, ALPHA_TEST, CAR_SHIFT, GRASS, ROAD, TERRAIN, THIN_FOLIAGE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadeFeatures {
    /// Procedural car surfaces and lamp emission (`Scene::car_materials`).
    pub cars: bool,
    /// Grass lighting and sun transmittance (GRASS, TERRAIN or ROAD materials).
    pub grass: bool,
    /// Tyre tracks on TERRAIN or ROAD materials.
    pub tracks: bool,
    /// Experimental world terrain sun visibility.
    pub world_terrain: bool,
    /// Experimental paged foliage sun visibility.
    pub foliage_pages: bool,
    /// Procedural meadow substrate tint.
    pub meadow: bool,
    /// Terrain layer shading (TERRAIN materials).
    pub terrain: bool,
    /// Needle and leaf lighting (THIN_FOLIAGE or ALPHA_TEST materials).
    pub foliage: bool,
}

impl ShadeFeatures {
    pub const ALL: Self = Self { cars: true, grass: true, tracks: true, world_terrain: true, foliage_pages: true, meadow: true, terrain: true, foliage: true };

    pub fn for_scene(scene: &Scene, world_terrain: bool, foliage_pages: bool, meadow: bool) -> Self {
        if std::env::var("KERN_SHADE_FEATURES").as_deref() == Ok("all") { return Self::ALL; }
        let any = |mask: u32| scene.materials.iter().any(|m| m.flags & mask != 0);
        Self {
            cars: !scene.car_materials.is_empty() || any(!0u32 << CAR_SHIFT),
            grass: any(GRASS | TERRAIN | ROAD),
            tracks: any(TERRAIN | ROAD),
            world_terrain,
            foliage_pages,
            meadow,
            terrain: any(TERRAIN),
            foliage: any(THIN_FOLIAGE | ALPHA_TEST),
        }
    }

    /// Pipeline constants for the shade module, with the two existing ones.
    pub fn constants(&self, rock_variation: bool, skip_hidden_meadow: bool) -> Vec<(&'static str, f64)> {
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        vec![
            ("ROCK_VARIATION", b(rock_variation)),
            ("SKIP_HIDDEN_MEADOW", b(skip_hidden_meadow)),
            ("FEATURE_CARS", b(self.cars)),
            ("FEATURE_GRASS", b(self.grass)),
            ("FEATURE_TRACKS", b(self.tracks)),
            ("FEATURE_WORLD_TERRAIN", b(self.world_terrain)),
            ("FEATURE_FOLIAGE_PAGES", b(self.foliage_pages)),
            ("FEATURE_MEADOW", b(self.meadow)),
            ("FEATURE_TERRAIN", b(self.terrain)),
            ("FEATURE_FOLIAGE", b(self.foliage)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Material;

    #[test]
    fn plain_scene_disables_and_material_flags_enable() {
        let mut scene = Scene::default();
        scene.add_material(Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let f = ShadeFeatures::for_scene(&scene, false, false, false);
        assert!(!f.cars && !f.grass && !f.tracks && !f.terrain && !f.foliage);
        scene.add_material(Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: ROAD, scale: 1.0, metal: 0.0 });
        let f = ShadeFeatures::for_scene(&scene, false, false, true);
        assert!(f.grass && f.tracks && !f.cars && f.meadow);
        assert_eq!(ShadeFeatures::ALL.constants(false, false).len(), 10);
    }
}
