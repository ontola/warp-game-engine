//! Explicit, CPU-side accounting of immutable vegetation and captured colour work.
//!
//! Construct after scene attachment and rebuild after geometry/category changes.
//! GPU poses may change without rebuilding. `view` consumes the renderer's exact
//! selection buffer, never reruns a CPU planner with different LOD history.
//! Volume cells are resident data, not raster boxes, triangles, unique visible
//! cells, or raymarch sample counts. Shadow work is intentionally separate.

use crate::{
    frame::VisibleClusterDiagnostic,
    geometry_diagnostics::{self, Counts},
    scene::{GeometryCategory, GpuInstance, Scene, THIN_FOLIAGE},
};
use std::collections::{BTreeMap, BTreeSet};

const CATEGORIES: [GeometryCategory; 2] = [GeometryCategory::Grass, GeometryCategory::Trees];

#[derive(Clone, Debug)]
pub struct CategoryInventory {
    pub category: GeometryCategory,
    /// Nonempty source instances, including legacy meshes if present.
    pub source_instances: u64,
    /// Subset of source instances attached to shared branch poses.
    pub source_primitive_placements: u64,
    /// Actual zero-cluster source-group owners, not unused pool slots.
    pub branch_proxies: u64,
    /// Finest DAG triangles multiplied by source-instance placement count.
    pub source_triangles_full_detail: u64,
    /// Finest DAG triangles counted once per resident cluster in this category.
    pub shared_triangles_full_detail: u64,
    /// Material THIN_FOLIAGE subset; not the DAG meaning of "leaf triangles".
    pub source_thin_foliage_triangles: u64,
    pub shared_thin_foliage_triangles: u64,
}

#[derive(Clone, Debug)]
pub struct Inventory {
    /// Grass then Trees. Category rows may reference the same shared asset.
    pub categories: [CategoryInventory; 2],
    /// Union across both categories, without counting cross-category sharing twice.
    pub shared_triangles_full_detail: u64,
    pub shared_thin_foliage_triangles: u64,
    pub resident_packed_levels: u64,
    pub resident_sparse_cell_records: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedMipCounts {
    /// Relative level selected by the renderer. Ray footprints may sample finer
    /// levels; this is a maximum allowed representation mip, not sample telemetry.
    pub max_mip: u32,
    pub volume_instances: u64,
    /// Nonempty grid cells at this mip, multiplied by selected placements.
    pub instance_occupied_cells: u64,
    /// Packed level records shared by those selected placements.
    pub unique_packed_levels: u64,
    pub unique_resident_occupied_cells: u64,
}

#[derive(Clone, Debug)]
pub struct CategoryViewCounts {
    pub category: GeometryCategory,
    /// Actual GPU-selected opaque colour work; padded vertices follow the
    /// active raster bucket mode, not triangle_count * 3.
    pub raster: Counts,
    /// Representation selection is not frustum/ray visibility. Culled groups
    /// can still select source or volume in this per-view buffer.
    pub selected_source_groups: u64,
    pub selected_volume_groups: u64,
    pub selected_mips: Vec<SelectedMipCounts>,
}

#[derive(Clone, Debug)]
pub struct ViewCounts {
    pub frame_id: u64,
    pub render_sequence: u64,
    pub view_index: usize,
    pub view_count: u32,
    pub viewport: [u32; 4],
    pub input_size: [u32; 2],
    pub output_size: [u32; 2],
    pub categories: [CategoryViewCounts; 2],
}

#[derive(Clone)]
struct Volume {
    category: GeometryCategory,
    index: usize,
    first_level: usize,
    level_count: usize,
}

/// Opt-in immutable scene snapshot. No GPU work, global toggles or per-frame
/// cost until a caller explicitly requests a captured view's accounting.
pub struct Accounting {
    inventory: Inventory,
    instances: Vec<GpuInstance>,
    categories: Vec<GeometryCategory>,
    triangles: Vec<u32>,
    volumes: Vec<Volume>,
    occupied_by_level: Vec<u64>,
}

fn category_index(category: GeometryCategory) -> Option<usize> {
    CATEGORIES.iter().position(|&c| c == category)
}

impl Accounting {
    pub fn new(scene: &Scene) -> Result<Self, String> {
        let mut categories = CATEGORIES.map(|category| CategoryInventory {
            category, source_instances: 0, source_primitive_placements: 0, branch_proxies: 0,
            source_triangles_full_detail: 0, shared_triangles_full_detail: 0,
            source_thin_foliage_triangles: 0, shared_thin_foliage_triangles: 0,
        });
        let mut shared: [BTreeSet<usize>; 2] = std::array::from_fn(|_| BTreeSet::new());
        let mut attachments = BTreeSet::new();
        for attachment in &scene.part_attachments {
            let instance = scene.instances.get(attachment.owner as usize).ok_or("accounting attachment owner out of range")?;
            if !attachments.insert(attachment.owner) || instance.cluster_count == 0 {
                return Err("accounting attachment must own a unique nonempty source instance".into());
            }
        }
        let thin = |cluster: usize| -> Result<bool, String> {
            let material = scene.clusters[cluster].material as usize;
            Ok(scene.materials.get(material).ok_or("accounting cluster material out of range")?.flags & THIN_FOLIAGE != 0)
        };
        for (owner, instance) in scene.instances.iter().enumerate() {
            let Some(category) = category_index(instance.category()) else { continue; };
            if instance.cluster_count == 0 { continue; }
            let first = instance.first_cluster as usize;
            let end = first.checked_add(instance.cluster_count as usize).ok_or("accounting source cluster range overflow")?;
            let clusters = scene.clusters.get(first..end).ok_or("accounting source cluster range out of bounds")?;
            let row = &mut categories[category];
            row.source_instances += 1;
            row.source_primitive_placements += u64::from(attachments.contains(&(owner as u32)));
            for (offset, cluster) in clusters.iter().enumerate().filter(|(_, c)| c.own_error == 0.0) {
                let id = first + offset;
                let count = u64::from(cluster.tri_count);
                row.source_triangles_full_detail += count;
                let is_thin = thin(id)?;
                if is_thin { row.source_thin_foliage_triangles += count; }
                if shared[category].insert(id) {
                    row.shared_triangles_full_detail += count;
                    if is_thin { row.shared_thin_foliage_triangles += count; }
                }
            }
        }
        let mut proxy_owners = BTreeSet::new();
        for group in &scene.foliage_source_groups {
            let proxy = scene.instances.get(group.owner as usize).ok_or("accounting proxy owner out of range")?;
            if proxy.cluster_count != 0 || !proxy_owners.insert(group.owner) {
                return Err("accounting source proxy must be unique and have zero clusters".into());
            }
            if let Some(category) = category_index(proxy.category()) { categories[category].branch_proxies += 1; }
        }
        let union: BTreeSet<_> = shared.into_iter().flatten().collect();
        let mut shared_triangles = 0;
        let mut shared_thin = 0;
        for cluster in union {
            let count = u64::from(scene.clusters[cluster].tri_count);
            shared_triangles += count;
            if thin(cluster)? { shared_thin += count; }
        }
        let mut occupied_by_level = Vec::new();
        let mut resident_sparse_cell_records = 0;
        if let Some(forest) = &scene.voxel_forest {
            resident_sparse_cell_records = forest.cells.len() as u64;
            for level in &forest.levels {
                let cells = level.dims_offset[..3].iter().try_fold(1usize, |n, &dim| n.checked_mul(dim as usize))
                    .ok_or("accounting voxel grid dimensions overflow")?;
                let first = level.dims_offset[3] as usize;
                let end = first.checked_add(cells).ok_or("accounting voxel grid range overflow")?;
                let indices = forest.indices.get(first..end).ok_or("accounting voxel grid range out of bounds")?;
                if indices.iter().any(|&id| id as usize > forest.cells.len()) {
                    return Err("accounting voxel sparse cell out of range".into());
                }
                occupied_by_level.push(indices.iter().filter(|&&id| id != 0).count() as u64);
            }
        }
        let mut volumes = Vec::new();
        let mut used_owners = BTreeSet::new();
        let mut used_volumes = BTreeSet::new();
        for binding in &scene.foliage_representations {
            if !used_owners.insert(binding.owner) || !used_volumes.insert(binding.volume) {
                return Err("accounting representation owner/volume must be unique".into());
            }
            let owner = scene.instances.get(binding.owner as usize).ok_or("accounting representation owner out of range")?;
            let forest = scene.voxel_forest.as_ref().ok_or("accounting representation has no volume forest")?;
            let volume = forest.instances.get(binding.volume as usize).ok_or("accounting representation volume out of range")?;
            let first = volume.meta[1] as usize;
            let count = volume.meta[2] as usize;
            if volume.meta[0] != binding.owner || count == 0 || first.checked_add(count).is_none_or(|end| end > occupied_by_level.len()) {
                return Err("accounting representation volume metadata is invalid".into());
            }
            volumes.push(Volume { category: owner.category(), index: binding.volume as usize, first_level: first, level_count: count });
        }
        Ok(Self {
            inventory: Inventory { categories, shared_triangles_full_detail: shared_triangles,
                shared_thin_foliage_triangles: shared_thin, resident_packed_levels: occupied_by_level.len() as u64,
                resident_sparse_cell_records },
            instances: scene.instances.clone(), categories: scene.instances.iter().map(GpuInstance::category).collect(),
            triangles: scene.clusters.iter().map(|c| c.tri_count).collect(), volumes, occupied_by_level,
        })
    }

    pub fn inventory(&self) -> &Inventory { &self.inventory }

    /// Count exactly one captured view. Summing four view reports intentionally
    /// counts a source instance drawn in four views four times. Inventory stays
    /// a once-per-scene count. Caller supplies the mode from that FrameStats.
    pub fn view(&self, capture: &VisibleClusterDiagnostic, cluster_buckets: bool) -> Result<ViewCounts, String> {
        let mut pairs = BTreeSet::new();
        for &[owner, cluster] in &capture.pairs {
            let instance = self.instances.get(owner as usize).ok_or("accounting selected owner out of range")?;
            if cluster < instance.first_cluster || cluster - instance.first_cluster >= instance.cluster_count {
                return Err("accounting selected cluster does not belong to its owner (or owner is a zero-cluster proxy)".into());
            }
            if !pairs.insert((owner, cluster)) { return Err("accounting selected owner/cluster pair is duplicated".into()); }
        }
        let raster = geometry_diagnostics::aggregate(&capture.pairs, &self.categories, &self.triangles, cluster_buckets)?;
        let mut categories = CATEGORIES.map(|category| CategoryViewCounts {
            category, raster: raster[category as usize], selected_source_groups: 0, selected_volume_groups: 0, selected_mips: Vec::new(),
        });
        let mut mips: BTreeMap<(usize, u32), (SelectedMipCounts, BTreeSet<usize>)> = BTreeMap::new();
        for volume in &self.volumes {
            let selection = capture.foliage_selection.get(volume.index).ok_or("accounting captured foliage selection is missing a volume")?;
            if selection[0] > 1 { return Err("accounting captured volume enable flag is invalid".into()); }
            let Some(category) = category_index(volume.category) else { continue; };
            if selection[0] == 0 { categories[category].selected_source_groups += 1; continue; }
            let mip = selection[1];
            if mip as usize >= volume.level_count { return Err("accounting captured volume mip out of range".into()); }
            categories[category].selected_volume_groups += 1;
            let packed_level = volume.first_level + mip as usize;
            let occupied = self.occupied_by_level[packed_level];
            let (row, unique) = mips.entry((category, mip)).or_insert_with(|| (SelectedMipCounts { max_mip: mip, ..Default::default() }, BTreeSet::new()));
            row.volume_instances += 1;
            row.instance_occupied_cells += occupied;
            if unique.insert(packed_level) {
                row.unique_packed_levels += 1;
                row.unique_resident_occupied_cells += occupied;
            }
        }
        for ((category, _), (row, _)) in mips { categories[category].selected_mips.push(row); }
        Ok(ViewCounts { frame_id: capture.frame_id, render_sequence: capture.render_sequence,
            view_index: capture.view_index, view_count: capture.view_count, viewport: capture.viewport,
            input_size: capture.input_size, output_size: capture.output_size, categories })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cluster::GpuCluster, foliage_representation::{Binding, Group, GroupKey, SourceGroup},
        foliage_volume_data::{GpuCell, GpuForest, GpuLevel, GpuTree}, part_wind::PartAttachment, scene::Material, Camera};
    use bytemuck::Zeroable;
    use glam::{Mat4, Vec3};

    fn scene() -> Scene {
        let mut scene = Scene::default();
        scene.materials.push(Material { tint: [1.;3], layer: -1, roughness: 1., flags: THIN_FOLIAGE, scale: 1., metal: 0. });
        scene.clusters = vec![GpuCluster { tri_count: 6, ..GpuCluster::zeroed() },
            GpuCluster { tri_count: 9, ..GpuCluster::zeroed() },
            GpuCluster { tri_count: 4, own_error: 1., ..GpuCluster::zeroed() }];
        for category in [GeometryCategory::Grass, GeometryCategory::Grass, GeometryCategory::Trees, GeometryCategory::Grass] {
            let mut instance = GpuInstance { cluster_count: 3, ..GpuInstance::zeroed() }; instance.set_category(category); scene.instances.push(instance);
        }
        scene.instances[3].cluster_count = 0;
        for owner in 0..3 { scene.part_attachments.push(PartAttachment { pose: 0, owner, rest: Mat4::IDENTITY }); }
        scene.foliage_source_groups.push(SourceGroup { owner: 3, members: vec![0,1] });
        scene
    }
    fn capture(pairs: Vec<[u32;2]>, view: usize, selection: Vec<[u32;4]>) -> VisibleClusterDiagnostic {
        VisibleClusterDiagnostic { frame_id: 7, render_sequence: 11, view_index: view, view_count: 4, viewport: [0,0,100,100],
            input_size: [100,100], output_size: [200,200], camera: Camera { pos: Vec3::Z, target: Vec3::ZERO, fov_y: 1., near: 0.1 },
            pairs, categories: None, foliage_selection: selection }
    }
    #[test]
    fn placement_and_view_duplication_do_not_duplicate_shared_storage() {
        let accounting = Accounting::new(&scene()).unwrap(); let inventory = accounting.inventory();
        let grass = &inventory.categories[0];
        assert_eq!((grass.source_instances, grass.source_primitive_placements, grass.branch_proxies), (2,2,1));
        assert_eq!((grass.source_triangles_full_detail, grass.shared_triangles_full_detail), (30,15));
        assert_eq!((grass.source_thin_foliage_triangles, grass.shared_thin_foliage_triangles), (30,15));
        assert_eq!(inventory.categories[1].source_triangles_full_detail, 15);
        assert_eq!(inventory.shared_triangles_full_detail, 15); // same mesh in both categories
        let pairs = vec![[0,0],[0,1],[1,0],[1,1],[2,2]];
        for view in 0..4 {
            let report = accounting.view(&capture(pairs.clone(), view, vec![]), true).unwrap();
            assert_eq!(report.view_index, view);
            let g = &report.categories[0].raster;
            assert_eq!((g.instances,g.clusters,g.triangles,g.submitted_vertices), (2,4,30,240));
            assert_eq!(report.categories[1].raster.triangles, 4); // selected coarse DAG node, not full-detail inventory
        }
        let flat = accounting.view(&capture(pairs, 0, vec![]), false).unwrap();
        assert_eq!(flat.categories[0].raster.submitted_vertices, 4*384);
        assert!(accounting.view(&capture(vec![[3,0]],0,vec![]),true).is_err());
        assert!(accounting.view(&capture(vec![[0,0],[0,0]],0,vec![]),true).is_err());
    }
    #[test]
    fn captured_mips_count_placement_weight_and_unique_residency_separately() {
        let mut scene = scene();
        scene.instances.push(scene.instances[3]);
        scene.voxel_forest = Some(GpuForest {
            levels: vec![GpuLevel { origin_cell:[0.,0.,0.,1.],dims_offset:[3,1,1,0] },
                GpuLevel { origin_cell:[0.,0.,0.,2.],dims_offset:[1,1,1,3] }],
            indices:vec![1,0,2,3],cells:vec![GpuCell::zeroed();3],
            instances:vec![GpuTree { meta:[3,0,2,0],..GpuTree::zeroed() },GpuTree { meta:[4,0,2,0],..GpuTree::zeroed() }],
        });
        for (volume,owner) in [3,4].into_iter().enumerate() {
            scene.foliage_representations.push(Binding { owner,volume:volume as u32, group:Group {
                key:GroupKey { instance:owner as u64,branch:0,generation:0 },center:Vec3::ZERO,radius:1.,candidates:vec![] } });
        }
        let accounting = Accounting::new(&scene).unwrap();
        let far = accounting.view(&capture(vec![],1,vec![[1,0,0,0],[1,0,0,0]]),true).unwrap();
        let row = &far.categories[0]; assert_eq!((row.selected_source_groups,row.selected_volume_groups),(0,2));
        assert_eq!(row.raster.triangles,0);
        assert_eq!(row.selected_mips,[SelectedMipCounts {max_mip:0,volume_instances:2,instance_occupied_cells:4,
            unique_packed_levels:1,unique_resident_occupied_cells:2}]);
        let mixed = accounting.view(&capture(vec![],2,vec![[0,u32::MAX,0,0],[1,1,0,0]]),true).unwrap();
        assert_eq!((mixed.categories[0].selected_source_groups,mixed.categories[0].selected_volume_groups),(1,1));
        assert_eq!(mixed.categories[0].selected_mips[0].instance_occupied_cells,1);
        assert!(accounting.view(&capture(vec![],0,vec![]),true).is_err());
        assert!(accounting.view(&capture(vec![],0,vec![[1,9,0,0],[0,0,0,0]]),true).is_err());
        assert_eq!(accounting.inventory().resident_sparse_cell_records,3);
    }
    #[test]
    fn invalid_sparse_cell_references_are_rejected() {
        let mut scene=Scene::default();
        scene.voxel_forest=Some(GpuForest { levels:vec![GpuLevel {origin_cell:[0.,0.,0.,1.],dims_offset:[1,1,1,0]}],
            indices:vec![1],..Default::default() });
        assert!(Accounting::new(&scene).is_err());
    }
}
