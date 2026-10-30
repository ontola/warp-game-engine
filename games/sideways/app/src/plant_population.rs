//! Game-owned species selection for the live mixed forest, planted through
//! the engine's recipe forest (full assemblies near the cameras, whole-plant
//! voxel proxies elsewhere).
use glam::{Affine3A, Quat, Vec3};
use render::{Scene, scene::GeometryCategory};
use sideways::{builder::Chunk, stage::Stage};

pub const TREE_IDS: [&str; 4] = ["spruce", "pine", "birch", "broadleaf"];
const SHRUB_IDS: [&str; 2] = ["juniper", "shrub"];
const MAX_SHRUBS: usize = 512;

pub fn mode(value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("reference") => Ok(false),
        Some("mixed") => Ok(true),
        _ => Err("SIDEWAYS_TREE_SPECIES must be reference or mixed".into()),
    }
}

pub fn configured() -> Result<bool, String> {
    mode(std::env::var("SIDEWAYS_TREE_SPECIES").ok().as_deref())
}

fn mix(mut v: u64) -> u64 {
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    v ^ (v >> 31)
}

fn root_key(seed: u64, x: f32, z: f32) -> u64 {
    mix(seed ^ u64::from(x.to_bits()).rotate_left(17) ^ u64::from(z.to_bits()).rotate_left(41))
}

/// A dominant species per 40 m grove, with a stable minority of companions.
/// This consumes no simulation RNG and has no camera/season/growth input.
pub fn tree_index(seed: u64, x: f32, z: f32) -> usize {
    let gx = (x / 40.0).floor() as i32;
    let gz = (z / 40.0).floor() as i32;
    let grove = mix(seed ^ u64::from(gx as u32) ^ (u64::from(gz as u32) << 32));
    let local = root_key(seed ^ 0xa034_b2ad_71a6_f951, x, z);
    let identity = if local % 5 == 0 { local >> 8 } else { grove };
    identity as usize % TREE_IDS.len()
}

pub fn stage_index(stage: &Stage, x: f32, z: f32) -> usize {
    tree_index(
        stage.noise_seed as u64 ^ stage.vegetation.seed.rotate_left(23),
        x,
        z,
    )
}

/// The live mixed forest on the engine's recipe forest
/// (render::recipe_forest): each species cooked once (cached in
/// build/cook-cache) and shared by every tree. Every tree is a whole-plant
/// voxel proxy; a pool of SIDEWAYS_POOL_SLOTS (4) full assemblies per species
/// follows the cameras (Pool::update each frame), standing in for the
/// nearest trees; past them, SIDEWAYS_CARD_SLOTS (250) card trees per species
/// (rigid card meshes) stand in within SIDEWAYS_CARD_RADIUS (80 m).
pub fn plant_forest(scene: &mut Scene, stage: &Stage, chunks: &[Chunk], cameras: &[Vec3])
    -> Result<(serde_json::Value, render::recipe_forest::Pool), String> {
    let knob = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
    let slots = knob("SIDEWAYS_POOL_SLOTS", 4.0) as usize;
    let (card_slots, card_radius) = (knob("SIDEWAYS_CARD_SLOTS", 250.0) as usize, knob("SIDEWAYS_CARD_RADIUS", 80.0));
    let started = std::time::Instant::now();
    let variants: Vec<_> = TREE_IDS.iter().map(|id| crate::plant_species::find(id).map(|s| (s.recipe, 1)))
        .collect::<Result<_, _>>()?;
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
    let cooked = render::recipe_forest::cook_variants(&variants, Some(&cache))?;
    let cards = if card_slots > 0 { render::recipe_forest::cook_cards(&variants, Some(&cache))? } else { Vec::new() };
    let cooked_ms = started.elapsed().as_secs_f64() * 1e3;
    let loaded = cooked.iter().filter(|c| c.loaded).count();
    let plants: Vec<_> = cooked.into_iter().map(|c| c.plant).collect();
    let near = |p: Vec3| cameras.iter().map(|c| c.distance(p)).fold(f32::INFINITY, f32::min);
    let mut trees: Vec<(f32, render::recipe_forest::Tree)> = chunks.iter().flat_map(|c| &c.trees).map(|spot| {
        let pos = Vec3::new(spot.pos.0, spot.pos.1, spot.pos.2);
        (near(pos), render::recipe_forest::Tree { pos, yaw: spot.yaw,
            scale: crate::tree_assets::native_scale(spot).y, variant: stage_index(stage, spot.pos.0, spot.pos.2) })
    }).collect();
    trees.sort_by(|a, b| a.0.total_cmp(&b.0));
    let trees: Vec<_> = trees.into_iter().map(|(_, t)| t).collect();
    let mut counts = [0usize; TREE_IDS.len()];
    for t in &trees { counts[t.variant] += 1; }
    // Proxies take 8 px of error, loosening from 20 m: the stage's forest
    // stands close to the road, and at 4 px from 50 m its splats cost two
    // split-screen views ~30 ms more per frame for no visible gain.
    let policy = render::aggregate_lod::Policy { proxy_pixels: 8.0, proxy_pixels_from: 20.0, ..render::recipe_forest::game_policy() };
    let (_, mut pool) = render::recipe_forest::plant_streamed(scene, &plants, &trees, slots, policy)?;
    if card_slots > 0 { render::recipe_forest::add_cards(scene, &mut pool, &cards, card_slots, card_radius)?; }
    Ok((serde_json::json!({"route":"render::recipe_forest::Pool","species":TREE_IDS,"species_counts":counts,"trees":trees.len(),
        "pool_slots_per_species":slots,"card_slots_per_species":card_slots,"card_radius_m":card_radius,"cooked_or_loaded_ms":cooked_ms,"loaded_from_cache":loaded,
        "total_ms":started.elapsed().as_secs_f64() * 1e3}), pool))
}

/// Reuse game-authored decorative fern habitat instead of inventing CPU
/// streaming or placing plants on roads. A fixed population cap is explicit.
pub fn add_shrubs(scene: &mut Scene, stage: &Stage, chunks: &[Chunk], meshes: &[usize]) -> usize {
    let mut candidates: Vec<_> = chunks
        .iter()
        .flat_map(|c| &c.ferns)
        .map(|spot| {
            (
                root_key(stage.vegetation.seed, spot.pos.0, spot.pos.2),
                spot,
            )
        })
        .collect();
    candidates.sort_by_key(|(key, _)| *key);
    candidates.truncate(MAX_SHRUBS);
    for (key, spot) in &candidates {
        let transform = Affine3A::from_scale_rotation_translation(
            Vec3::new(spot.scale, spot.scale_y, spot.scale),
            Quat::from_rotation_y(spot.yaw),
            Vec3::new(spot.pos.0, spot.pos.1, spot.pos.2),
        );
        scene.add_classified_instance(
            meshes[*key as usize % meshes.len()],
            transform,
            GeometryCategory::Other,
        );
        scene.mark_foliage();
    }
    candidates.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_is_stable_world_fixed_and_all_species_are_reachable() {
        let mut counts = [0; 4];
        for i in -1000..1000 {
            let x = i as f32 * 1.337;
            let z = i as f32 * -2.53;
            let chosen = tree_index(71, x, z);
            assert_eq!(chosen, tree_index(71, x, z));
            counts[chosen] += 1;
        }
        assert!(counts.iter().all(|count| *count > 200));
        assert!(!mode(None).unwrap());
        assert!(mode(Some("mixed")).unwrap());
        assert!(mode(Some("unknown")).is_err());
    }
}
