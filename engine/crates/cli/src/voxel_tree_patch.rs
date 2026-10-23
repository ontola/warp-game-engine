//! Explicit bounded live experiment. This is a fixed patch of source-derived
//! tree crowns, not an automatic distance LOD or a whole-forest replacement.
use glam::Vec3;
use render::{
    Scene,
    cluster::{Part, read_kmesh},
    foliage_volume_data::{PackLimits, VolumeBinding, pack},
    foliage_voxel::{BakeConfig, bake},
};
use std::path::Path;

pub fn attach(
    scene: &mut Scene,
    tree_meshes: &[usize],
    crown_material: u32,
    cameras: &[Vec3],
    root: &Path,
) -> Result<Option<String>, String> {
    let Ok(value) = std::env::var("SIDEWAYS_VOXEL_TREE_PATCH") else {
        return Ok(None);
    };
    if value == "0" {
        return Ok(None);
    }
    let count = value
        .parse::<usize>()
        .map_err(|_| "SIDEWAYS_VOXEL_TREE_PATCH must be 1..256, or 0 to disable")?;
    if !(1..=256).contains(&count) {
        return Err("SIDEWAYS_VOXEL_TREE_PATCH must be 1..256".into());
    }
    let started = std::time::Instant::now();
    let camera = cameras
        .first()
        .copied()
        .ok_or("voxel tree patch requires a camera")?;
    // Initial rally heading is toward -Z. Select a fixed patch farther along
    // the starting view. Its owners never change as the player moves, so this
    // experiment cannot masquerade as a tested moving near/far LOD boundary.
    let centre = camera + Vec3::new(0.0, 0.0, -110.0);
    let mut candidates: Vec<_> = scene
        .instances
        .iter()
        .enumerate()
        .filter_map(|(owner, i)| {
            let asset = tree_meshes
                .iter()
                .position(|m| scene.meshes[*m].first_cluster == i.first_cluster)?;
            let p = Vec3::new(i.rows[0][3], i.rows[1][3], i.rows[2][3]);
            if p.distance(camera) < 60.0 {
                return None;
            }
            Some(((p - centre).length_squared(), owner as u32, asset as u32))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    if candidates.len() < count {
        return Err(format!(
            "voxel patch requires {count} trees, found {}",
            candidates.len()
        ));
    }
    let bindings: Vec<_> = candidates
        .iter()
        .take(count)
        .map(|(_, owner, asset)| VolumeBinding {
            owner: *owner,
            asset: *asset,
            crown_material,
        })
        .collect();
    let mut assets = Vec::new();
    for variant in ["a", "b", "c"] {
        let path = root.join(format!("engine/assets/fir_{variant}_near.kmesh"));
        let parts =
            read_kmesh(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
        let crowns: Vec<_> = parts
            .iter()
            .filter(|(name, _, _)| !name.contains("trunk") && !name.contains("bark"))
            .map(|(_, v, i)| Part {
                vertices: v,
                indices: i,
                material: crown_material,
            })
            .collect();
        assets.push(bake(
            &crowns,
            &scene.materials,
            &scene.textures,
            BakeConfig {
                cell_size: 0.25,
                levels: 1,
                ..Default::default()
            },
        )?);
    }
    let forest = pack(&assets, &bindings, &scene.instances, PackLimits::default())?;
    let bytes = forest.byte_len();
    scene.voxel_forest = Some(forest);
    Ok(Some(format!(
        "EXPERIMENTAL voxel tree patch: {count} fixed source-derived crowns / 3 shared assets, 0.25 m density cells, {} bytes packed, {:.0} ms bake/pack; original trunks and shadow casters retained; no automatic near/far transition or whole-forest scalability claim",
        bytes,
        started.elapsed().as_secs_f64() * 1000.0
    )))
}
