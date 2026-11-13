//! Shared biome recipes at Sideways' existing roadside and background roots.
//! The optional mixed forest on a native stage uses the moving recipe pool.
use glam::Vec3;
use render::{biome::{Biome, Flora}, Scene, foliage_runtime::RootPlacement, foliage_population::WorldRoot};
use sideways::{builder::{Chunk, N}, stage::{Stage, GRID}};
use std::collections::HashMap;

pub const TREE_IDS: [&str; 4] = ["spruce", "pine", "birch", "broadleaf"];
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
fn grove_key(seed: u64, x: f32, z: f32) -> u64 {
    let gx = (x / 40.0).floor() as i32;
    let gz = (z / 40.0).floor() as i32;
    let grove = mix(seed ^ u64::from(gx as u32) ^ (u64::from(gz as u32) << 32));
    let local = root_key(seed ^ 0xa034_b2ad_71a6_f951, x, z);
    if local % 5 == 0 { local >> 8 } else { grove }
}

pub fn tree_index(seed: u64, x: f32, z: f32) -> usize {
    grove_key(seed, x, z) as usize % TREE_IDS.len()
}

pub fn stage_index(stage: &Stage, x: f32, z: f32) -> usize {
    tree_index(
        stage.noise_seed as u64 ^ stage.vegetation.seed.rotate_left(23),
        x,
        z,
    )
}

fn unit(key: u64) -> f32 { (key >> 40) as f32 / 16_777_216.0 }

/// Near and distant trees share grove membership, age and species. Every
/// placed tree (including its collider) keeps the scale from stage construction.
pub fn biome_tree(stage: &Stage, flora: &Flora, x: f32, z: f32) -> usize {
    let seed = stage.noise_seed as u64 ^ stage.vegetation.seed.rotate_left(23);
    flora.tree_variant(unit(grove_key(seed, x, z))).expect("placed tree in biome with no trees")
}

/// Exact world identity for the stable GPU population.
pub fn tree_root_id(x: f32, z: f32) -> u64 {
    (u64::from(x.to_bits()) << 32) | u64::from(z.to_bits())
}

pub fn tree_root(stage: &Stage, biome: &Biome, pos: Vec3, yaw: f32, width: f32) -> WorldRoot {
    WorldRoot { id: tree_root_id(pos.x, pos.z), placement: RootPlacement {
        pos, up: Vec3::Y, yaw, scale: width, variant: biome_tree(stage, &biome.flora, pos.x, pos.z),
    } }
}

/// Publish one set of world roots to Warp's source/voxel forest. The sun
/// and all split-screen cameras share the same roots and species assets.
pub fn plant_biome(scene: &mut Scene, biome: Biome, roots: &[WorldRoot]) -> Result<serde_json::Value, String> {
    let catalogue = render::recipe_forest::BiomePlants::new(&render::biome::Lands { biomes: vec![biome] });
    let mut counts = vec![0usize; catalogue.len()];
    for root in roots {
        *counts.get_mut(root.placement.variant).ok_or("biome root has an unknown species")? += 1;
    }
    let mut slots = 0;
    if !roots.is_empty() {
        let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
        let plants: Vec<_> = catalogue.cook(Some(&cache))?.into_iter().map(|c| c.plant).collect();
        // terrain_fly's shared forest budget. A small planting needs no more
        // slots than the total number of branches it could actually admit.
        let possible: usize = counts.iter().zip(&plants).map(|(n, plant)| n * plant.stats().branches).sum();
        slots = possible.clamp(1, 65536) as u32;
        render::foliage_runtime::plant_population(scene, &plants, roots, roots.len(), slots,
            render::recipe_forest::BiomePlants::quality_policy())?;
    }
    let variants = catalogue.variants();
    Ok(serde_json::json!({"route":"render::foliage_runtime::plant_species_population","biome":biome.name,
        "species":variants.iter().map(|(name,_)|name).collect::<Vec<_>>(),"species_counts":counts,
        "variants":variants,"roots":roots.len(),"branch_slots":slots,"shared_near_far":true,
        "hierarchy_nodes":scene.gpu_forest.as_ref().map_or(0, |f| f.nodes.len())}))
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
        (near(pos), render::recipe_forest::Tree { up:Vec3::Y, pos, yaw: spot.yaw,
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

/// Sample Sideways' off-road fern sites in the visible verge, spreading the
/// bounded understory along the whole route independently of camera position.
pub fn shrubs(stage: &Stage, chunks: &[Chunk], biome: &Biome) -> Vec<WorldRoot> {
    let flora = &biome.flora;
    // Use the crowns of the actual stage trees, including trees across a chunk
    // boundary. The painted forest supplies the crown radius in world metres.
    let radius = biome.vegetation.forest.crown[0];
    let cell = |d: f32| (d / radius).floor() as i32;
    let mut crowns: HashMap<_, Vec<_>> = HashMap::new();
    for tree in chunks.iter().flat_map(|c| &c.trees) {
        let (x, z) = (tree.pos.0, tree.pos.2);
        let r = radius * tree.scale;
        if r <= 0.0 { continue; }
        for iz in cell(z - r)..=cell(z + r) {
            for ix in cell(x - r)..=cell(x + r) {
                crowns.entry((ix, iz)).or_default().push((x, z, r));
            }
        }
    }
    let mut candidates: Vec<_> = chunks
        .iter()
        .flat_map(|c| c.ferns.iter().filter(move |spot| {
            let cell = |d: f64| ((d / GRID).round() as usize).min(N - 1);
            let i = cell(f64::from(spot.pos.0) - c.x0);
            let j = cell(f64::from(spot.pos.2) - c.z0);
            (3.0..40.0).contains(&c.road_d[j * N + i])
        }))
        .filter_map(|spot| {
            let (x, z) = (spot.pos.0, spot.pos.2);
            let key = root_key(stage.vegetation.seed, x, z);
            let cover = crowns.get(&(cell(x), cell(z))).into_iter().flatten()
                .map(|&(cx, cz, r)| 1.0 - ((cx - x).powi(2) + (cz - z).powi(2)) / (r * r))
                .fold(0.0, f32::max);
            (unit(mix(key ^ 0x5348_4144_4500)) < flora.shrub_habitat(cover)).then_some((key, spot))
        })
        .collect();
    candidates.sort_by_key(|(key, _)| *key);
    candidates.truncate(MAX_SHRUBS);
    candidates.into_iter().filter_map(|(key, spot)| {
        let variant = flora.shrub_variant(unit(mix(key)))?;
        let pos = Vec3::new(spot.pos.0, spot.pos.1, spot.pos.2);
        Some(WorldRoot { id: tree_root_id(pos.x, pos.z) ^ 0x5352_5542_0000_0000,
            placement: RootPlacement { pos, up: Vec3::Y, yaw: spot.yaw, scale: spot.scale, variant } })
    }).collect()
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

    #[test]
    fn native_roots_use_the_same_weighted_biome_species_at_any_distance() {
        let stage = Stage::generate(&mut gdcompat::Pcg::new(19), 500.0);
        let rainforest = Biome::named("rainforest").unwrap();
        let species = rainforest.species();
        let mut counts = std::collections::BTreeMap::new();
        for i in 0..2000 {
            let (x, z) = (i as f32 * 49.0, i as f32 * -1.7);
            let index = biome_tree(&stage, &rainforest.flora, x, z);
            assert_eq!(index, biome_tree(&stage, &rainforest.flora, x, z));
            *counts.entry(species[index].0).or_insert(0) += 1;
        }
        assert_eq!(counts.len(), 2);
        assert!(counts["african_mahogany"] > 2 * counts["kapok"]);

        // Inspect actual roadside roots before committing the GPU population.
        let desert = Biome::named("desert").unwrap();
        assert!(desert.flora.tree_variant(0.5).is_none());
        let chunks: Vec<_> = (-1..=1).flat_map(|x| (-2..=0).map(move |z| (x, z)))
            .map(|cell| sideways::builder::terrain_chunk(&stage, cell)).collect();
        let roots = shrubs(&stage, &chunks, &desert);
        assert!(!roots.is_empty() && roots.len() <= MAX_SHRUBS);
        assert_eq!(roots, shrubs(&stage, &chunks, &desert));
        for i in 0..desert.species().len() {
            assert!(roots.iter().any(|r| r.placement.variant == i));
        }
        for r in roots {
            let pos = r.placement.pos;
            let p = gdcompat::math::v3(pos.x, pos.y, pos.z);
            let road = stage.locate(p, -1);
            // Past the end of the ribbon, distance to its infinite tangent
            // line can be zero. Check the actual road at the clamped station.
            let centre = stage.point_at(road.along);
            assert!((p.x - centre.x).hypot(p.z - centre.z) as f64 > stage.width_at(road.along));
        }
    }
}
