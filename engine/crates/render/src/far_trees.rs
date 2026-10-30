//! Distant trees as rasterized voxels (docs/voxel-raster-plan.md, M3).
//!
//! Bakes each tree mesh once into a voxel pyramid (foliage_voxel), binds every
//! scene instance of those meshes, and sets the scene's far distance. Past it
//! the camera draws the tree's voxels instead of its mesh; the shared sun map
//! takes voxels for every bound tree. Near trees keep their mesh, wind and the
//! near shadow map. Any game with mesh trees can call this once after placing
//! them.
use crate::cluster::Part;
use crate::foliage_volume_data::{ALL_MATERIALS, AssetPackLimits, VolumeBinding, pack_assets};
use crate::foliage_voxel::{BakeConfig, bake};
use crate::Scene;

#[derive(Clone, Copy, Debug)]
pub struct FarTrees {
    /// Camera distance where a tree turns into voxels. Keep it past the wind
    /// freeze (wind::WindFreeze::DEFAULT), so the switch never cuts a sway.
    pub distance_m: f32,
    /// Finest cell; coarser levels double it.
    pub cell_m: f32,
    pub levels: u32,
    /// A tree uses the coarsest level whose cell stays under this many pixels.
    pub cell_pixels: f32,
}

impl Default for FarTrees {
    fn default() -> Self { Self { distance_m: 120.0, cell_m: 0.2, levels: 4, cell_pixels: 1.5 } }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub trees: usize,
    pub assets: usize,
    pub cells: usize,
    pub bytes: usize,
    pub bake_ms: f64,
}

/// `meshes`: scene mesh index and its parts, all materials (trunk included,
/// since the whole tree switches). Instances of other meshes are untouched.
pub fn attach(scene: &mut Scene, meshes: &[(usize, Vec<Part<'_>>)], config: FarTrees) -> Result<Report, String> {
    if scene.voxel_forest.is_some() { return Err("far trees: the scene already has a voxel forest".into()); }
    if !(config.distance_m.is_finite() && config.distance_m > 0.0) { return Err("far trees: distance must be positive".into()); }
    let started = std::time::Instant::now();
    let mut assets = Vec::with_capacity(meshes.len());
    for (_, parts) in meshes {
        assets.push(bake(parts, &scene.materials, &scene.textures,
            BakeConfig { cell_size: config.cell_m, levels: config.levels, ..Default::default() })?);
    }
    let cells = assets.iter().flat_map(|a| &a.levels).map(|l| l.voxels.len()).sum();
    let mut packed = pack_assets(&assets, AssetPackLimits { max_assets: assets.len().max(1), levels_per_asset: config.levels as usize,
        first_level_bounds: false, max_dense_indices: 1 << 26, max_sparse_cells: cells })?;
    let first_clusters: Vec<u32> = meshes.iter().map(|(mesh, _)| scene.meshes.get(*mesh).map(|m| m.first_cluster)
        .ok_or_else(|| format!("far trees: mesh {mesh} is not in the scene"))).collect::<Result<_, _>>()?;
    for (owner, instance) in scene.instances.iter().enumerate() {
        let Some(asset) = first_clusters.iter().position(|&c| c == instance.first_cluster) else { continue };
        let tree = packed.instance(VolumeBinding { owner: owner as u32, asset: asset as u32, crown_material: ALL_MATERIALS }, instance)?;
        packed.forest.instances.push(tree);
    }
    if packed.forest.instances.is_empty() { return Err("far trees: no instance uses the given meshes".into()); }
    let report = Report { trees: packed.forest.instances.len(), assets: assets.len(), cells, bytes: packed.forest.byte_len(),
        bake_ms: started.elapsed().as_secs_f64() * 1000.0 };
    scene.voxel_forest = Some(packed.forest);
    scene.voxel_far_distance_m = config.distance_m;
    scene.voxel_lod_pixels = config.cell_pixels;
    Ok(report)
}
