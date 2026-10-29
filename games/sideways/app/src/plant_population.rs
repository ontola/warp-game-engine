//! Game-owned species selection and one-time assembly cooking for the live forest.
//! Detailed shared assemblies are preview-only until forest-scale pose/LOD
//! selection exists; the former flattened whole-tree path is disabled.
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

pub struct Sources {
    pub trees: Vec<usize>,
    pub shrubs: Vec<usize>,
    pub metadata: serde_json::Value,
}

/// The rejected v1 route flattened all spray geometry into whole-tree CLOD.
/// Detailed sources are intentionally not sent through that path: the shared
/// runtime still has a bounded pose pool and no aggregate distance hierarchy.
/// Fail before mutating the scene, rather than allocating millions of duplicated
/// vertices or silently substituting a different tree at distance.
pub fn cook(
    _scene: &mut Scene,
    _stage: &Stage,
    _fullness: f32,
    _names: &mut Vec<String>,
) -> Result<Sources, String> {
    Err("SIDEWAYS_TREE_SPECIES=mixed is unavailable while detailed assemblies lack forest-scale pose/LOD selection. Use tree_species_compare for the shared GPU assembly preview, or reference for the live forest.".into())
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
