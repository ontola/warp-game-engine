//! A forest of recipe plants, the way every game draws one: a few cooked
//! variants of a recipe are shared by every tree; the trees nearest the
//! camera are full assemblies (branch groups pick source or voxels by
//! projected error), every other tree is the same plant as one whole-plant
//! voxel proxy (docs/voxel-raster-plan.md). Cooks run in parallel and are
//! cached on disk (CookedPlant::cached), so a warm start loads in
//! milliseconds.

use crate::aggregate_lod::Policy;
use crate::plant_recipe::{self, BuildOptions, WoodyRecipe};
use crate::scene::GeometryCategory;
use crate::vegetation_aggregate::Config;
use crate::vegetation_asset::{AttachLimits, AttachmentReport, CookedPlant, PlantInstance, SceneVegetation};
use crate::Scene;
use glam::{Mat4, Quat, Vec3};
use std::path::Path;

/// Levels of the whole-plant proxy: 10 cm cells, then 20 cm .. 3.2 m.
pub const PROXY_LEVELS: u32 = 6;

/// The aggregate cook every recipe tree uses: each branch group also gets
/// voxel levels.
pub fn aggregate_config() -> Config {
    Config { max_placements: 8192, max_source_triangles: 40_000_000, max_samples: 150_000_000, triangle_refinements: true,
        max_reference_voxels: 262_144, max_diagnostic_steps: 80_000_000, ..Default::default() }
}

/// The whole-plant proxy cook: 10 cm reference cells.
pub fn proxy_config() -> Config {
    Config { reference_cell_size: 0.1, max_reference_voxels: 1 << 21, max_build_bytes: 1 << 30, ..aggregate_config() }
}

/// One cooked variant and whether it came from the cache.
pub struct Cooked {
    pub plant: CookedPlant,
    pub loaded: bool,
    pub millis: f64,
}

/// Cook each (recipe, seed) variant in parallel, each with its whole-plant
/// proxy. With `cache`, a variant cooked before with the same recipe, seed,
/// configs and engine cook code loads from disk instead.
pub fn cook_variants(variants: &[(WoodyRecipe, u64)], cache: Option<&Path>) -> Result<Vec<Cooked>, String> {
    let (config, proxy) = (aggregate_config(), proxy_config());
    std::thread::scope(|scope| {
        let jobs: Vec<_> = variants.iter().map(|(recipe, seed)| scope.spawn(move || -> Result<Cooked, String> {
            let seed = *seed;
            let started = std::time::Instant::now();
            let cook = || {
                let assembly = plant_recipe::build(recipe, BuildOptions::new(seed), plant_recipe::recipe_limits())?;
                let mut plant = CookedPlant::cook_aggregate(&assembly, config)?;
                plant.cook_proxy(&assembly, proxy, PROXY_LEVELS)?;
                Ok(plant)
            };
            let (plant, loaded) = match cache {
                Some(dir) => {
                    let what = format!("{recipe:?} {:?} {:?} {config:?} {proxy:?} {PROXY_LEVELS}",
                        BuildOptions::new(seed), plant_recipe::recipe_limits());
                    CookedPlant::cached(dir, &what, cook)?
                }
                None => (cook()?, false),
            };
            Ok(Cooked { plant, loaded, millis: started.elapsed().as_secs_f64() * 1000.0 })
        })).collect();
        jobs.into_iter().map(|job| job.join().map_err(|_| "recipe cook panicked".to_string())?).collect()
    })
}

/// One tree: its foot, turn about +Y, uniform scale and cooked variant.
#[derive(Clone, Copy, Debug)]
pub struct Tree {
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
    pub variant: usize,
}

/// Put `trees` into `scene`: the first `full` as full assemblies, the rest
/// as whole-plant proxies (pass them nearest first). `policy` sets the
/// screen error of both. Stable plant ids are the trees' indices + 1.
pub fn plant(scene: &mut Scene, variants: &[CookedPlant], trees: &[Tree], full: usize, policy: Policy) -> Result<AttachmentReport, String> {
    if trees.is_empty() { return Ok(AttachmentReport::default()); }
    if let Some(tree) = trees.iter().find(|t| t.variant >= variants.len()) {
        return Err(format!("recipe forest variant {} of {}", tree.variant, variants.len()));
    }
    let full = full.min(trees.len());
    let branches = variants.iter().map(|c| c.stats().branches).max().unwrap_or(64);
    let mut limits = AttachLimits { max_poses: branches.max(64) * full.max(4),
        max_primitive_instances: crate::part_wind::MAX_ATTACHMENTS, ..Default::default() };
    limits.pack.max_instances = limits.max_poses.min(crate::foliage_volume_data::MAX_INSTANCES);
    let mut vegetation = SceneVegetation::new(scene, limits)?;
    let instance = |i: usize, t: &Tree| PlantInstance { id: i as u64 + 1, generation: 0,
        root: Mat4::from_scale_rotation_translation(Vec3::splat(t.scale), Quat::from_rotation_y(t.yaw), t.pos),
        category: GeometryCategory::Trees };
    for (i, tree) in trees[..full].iter().enumerate() { vegetation.append(&variants[tree.variant], instance(i, tree))?; }
    for (i, tree) in trees[full..].iter().enumerate() {
        vegetation.append_proxy(&variants[tree.variant], instance(full + i, tree))?;
    }
    vegetation.finish_aggregate(policy)
}

/// The renderer policy for a recipe forest in a game: ordinary screen-error
/// cluster LOD, sampled optics, proxies at Policy's defaults.
pub fn game_policy() -> Policy {
    Policy { max_error_pixels: 1.0, allow_sampled_optics: true, measured_source_cuts: false, ..Default::default() }
}
