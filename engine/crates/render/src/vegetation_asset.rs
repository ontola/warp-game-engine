//! Cooked shared vegetation primitives and bounded scene attachment.
//!
//! Source-only cooking and attachment never expand placement geometry. The
//! optional voxel cook may expand the source for optical derivation:
//! every blade/needle/wood member references an immutable primitive mesh and
//! a branch-local transform. Equal geometry, material AND CLOD policy share a mesh,
//! even across different cooked variants. This is not yet hierarchical coarse
//! geometry aggregation or a whole-forest residency/streaming system.

use crate::{
    cluster::{ClusterMesh, Part, SimplificationPolicy},
    foliage_assembly::Assembly,
    foliage_representation::{Binding, Group, GroupKey, OpticalEvidence, Policy, SourceGroup},
    foliage_volume_data::{self, ALL_MATERIALS, PackLimits, VolumeBinding},
    foliage_voxel::{BakeConfig, VoxelAsset},
    part_wind::{MAX_ASSEMBLY_PARTS, MAX_PARTS, PartAttachment, PartWind},
    scene::{FOLIAGE, GeometryCategory, GpuInstance, Material, PART_WIND, Scene},
};
use glam::{Affine3A, Mat4, Vec3, Vec4};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct Primitive {
    mesh: ClusterMesh,
    material: Material,
    // Exact comparison, not a lossy geometry ID or unchecked hash collision.
    key: Vec<u8>,
}

/// The actual mesh builder belongs to shared-mesh identity. An optical cook
/// and a source-only cook can request different fallback coverage constraints
/// for identical source bytes; recognized closed sweeps use the same builder.
#[repr(u8)]
enum PrimitivePolicy { ClosedSweep, TexturedGeometry, ThinFoliage }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Placement {
    id: u64,
    primitive: usize,
    branch: usize,
    rest: Mat4,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Branch {
    id: u64,
    parent: Option<u32>,
    rest: Mat4,
    assembly_rest: Mat4,
    max_angle: f32,
    volume: Option<VoxelAsset>,
    aggregate: Vec<crate::aggregate_lod::Candidate>,
    aggregate_motion_error: f32,
}

/// Immutable CPU asset. Its source meshes contain only unique primitives,
/// never transformed copies of every leaf in a branch.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CookedPlant {
    primitives: Vec<Primitive>,
    placements: Vec<Placement>,
    branches: Vec<Branch>,
    stats: CookStats,
    /// The whole plant as one voxel pyramid in the root branch frame.
    proxy: Option<VoxelAsset>,
    /// Unique per cook_proxy call (or load), so append_proxy finds the
    /// shared asset and its support without comparing voxels.
    #[serde(skip)]
    proxy_id: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CookStats {
    pub primitive_meshes: usize,
    /// Resident primitive vertices, including intermediate CLOD levels.
    pub primitive_vertices: usize,
    /// Finest source triangles only; coarse DAG triangles are excluded.
    pub primitive_triangles: usize,
    /// Cluster mesh payload, excluding the compact placements and voxel assets.
    pub primitive_bytes: usize,
    pub placements: usize,
    pub branches: usize,
    pub volume_assets: usize,
    pub expanded_source_vertices: usize,
    pub expanded_source_triangles: usize,
    /// Unclustered expanded vertex + u32 index payload, as an oracle comparison.
    pub expanded_source_bytes: usize,
}

fn mesh_bytes(mesh: &ClusterMesh) -> usize {
    mesh.vertices.len() * std::mem::size_of::<crate::cluster::Vertex>()
        + (mesh.cluster_verts.len() + mesh.tris.len()) * 4
        + mesh.clusters.len() * std::mem::size_of::<crate::cluster::GpuCluster>()
}

fn finest_vertex_count(mesh: &ClusterMesh) -> usize {
    let mut vertices = BTreeSet::new();
    for cluster in mesh.clusters.iter().filter(|c| c.own_error == 0.0) {
        for &triangle in &mesh.tris[cluster.first_tri as usize..][..cluster.tri_count as usize] {
            for local in [triangle & 255, (triangle >> 8) & 255, (triangle >> 16) & 255] {
                vertices.insert(mesh.cluster_verts[(cluster.first_vert + local) as usize]);
            }
        }
    }
    vertices.len()
}

fn primitive_key(
    vertices: &[crate::cluster::Vertex],
    indices: &[u32],
    material: &Material,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(bytemuck::bytes_of(material));
    bytes.extend_from_slice(&(vertices.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(indices.len() as u64).to_le_bytes());
    bytes.extend_from_slice(bytemuck::cast_slice(vertices));
    bytes.extend_from_slice(bytemuck::cast_slice(indices));
    bytes
}

impl CookedPlant {
    /// Cook reusable source primitives and branch placements without expanding
    /// whole plants or baking volumes. Runtime still uses the bounded shared
    /// GPU part-pose/attachment path, not a forest-scale assembly hierarchy.
    pub fn cook_source(source: &Assembly) -> Result<Self, String> {
        Self::cook_internal(source, None)
    }

    pub fn cook(source: &Assembly, config: BakeConfig) -> Result<Self, String> {
        Self::cook_internal(source, Some(config))
    }

    fn cook_internal(source: &Assembly, config: Option<BakeConfig>) -> Result<Self, String> {
        source.validate()?;
        for instance in &source.instances {
            validate_root(instance.rest).map_err(|e| format!("primitive {}: {e}", instance.id))?;
        }
        if source.branches.len() > MAX_ASSEMBLY_PARTS {
            return Err(format!(
                "cooked plant exceeds {MAX_ASSEMBLY_PARTS} branch poses"
            ));
        }
        let rest = source.branch_transforms()?;
        // The only expanded geometry path in this module: offline/cook-time
        // optical derivation, not the meshes subsequently attached to Scene.
        let baked = match config {
            Some(config) => source.bake_branches(config)?,
            None => Vec::new(),
        };
        let mut branches: Vec<_> = source
            .branches
            .iter()
            .enumerate()
            .map(|(i, branch)| Branch {
                id: branch.id,
                parent: branch.parent,
                rest: branch.rest,
                assembly_rest: rest[i],
                max_angle: if branch.parent.is_none() {
                    0.0
                } else {
                    0.22 * (1.0 - branch.stiffness).clamp(0.15, 1.0)
                },
                volume: None,
                aggregate: Vec::new(),
                aggregate_motion_error: 0.0,
            })
            .collect();
        for branch in baked {
            branches[branch.branch as usize].volume = Some(branch.asset);
        }
        let mut primitives = Vec::new();
        let mut keys = BTreeMap::<Vec<u8>, usize>::new();
        let mut placements = Vec::with_capacity(source.instances.len());
        let mut stats = CookStats::default();
        for instance in &source.instances {
            let geometry = &source.parts[instance.part as usize];
            let material = source.materials[instance.material as usize];
            let key = primitive_key(&geometry.vertices, &geometry.indices, &material);
            let primitive = if let Some(&index) = keys.get(&key) {
                index
            } else {
                let part = Part {
                    vertices: &geometry.vertices,
                    indices: &geometry.indices,
                    material: 0,
                };
                // Tiny closed blades/needles fit below the generic cluster
                // target and otherwise never get intermediate source LODs.
                // Derive closed reductions from the same source. Source-only
                // leafy sprays may contain disconnected leaves; keep their
                // orientation/projected area when the sweep recognizer declines.
                // Existing optical cooks retain their established CLOD bytes.
                let thin_source = config.is_none()
                    && material.flags & crate::scene::THIN_FOLIAGE != 0;
                let sweep = if material.layer < 0 && material.flags & crate::scene::THIN_FOLIAGE != 0 {
                    crate::vegetation_clod::build(&part)
                } else { None };
                let (mesh, policy) = if let Some(mesh) = sweep {
                    (mesh, PrimitivePolicy::ClosedSweep)
                } else if thin_source {
                    (ClusterMesh::build_with_policy(&[part], |_| SimplificationPolicy::ThinFoliage),
                        PrimitivePolicy::ThinFoliage)
                } else {
                    (ClusterMesh::build_with_policy(&[part], |_| SimplificationPolicy::TexturedGeometry),
                        PrimitivePolicy::TexturedGeometry)
                };
                stats.primitive_vertices += mesh.vertices.len();
                stats.primitive_triangles += mesh.leaf_triangles();
                stats.primitive_bytes += mesh_bytes(&mesh);
                let index = primitives.len();
                // Within one cook the configuration is constant, so the source
                // key can avoid rebuilding identical primitives. Across cooks,
                // retain the concrete builder in the scene-sharing key as well.
                keys.insert(key.clone(), index);
                let mut key = key;
                key.push(policy as u8);
                primitives.push(Primitive {
                    mesh,
                    material,
                    key,
                });
                index
            };
            stats.expanded_source_vertices += geometry.vertices.len();
            stats.expanded_source_triangles += geometry.indices.len() / 3;
            stats.expanded_source_bytes += geometry.vertices.len()
                * std::mem::size_of::<crate::cluster::Vertex>()
                + geometry.indices.len() * std::mem::size_of::<u32>();
            placements.push(Placement {
                id: instance.id,
                primitive,
                branch: instance.branch as usize,
                rest: instance.rest,
            });
        }
        stats.primitive_meshes = primitives.len();
        stats.placements = placements.len();
        stats.branches = branches.len();
        stats.volume_assets = branches.iter().filter(|b| b.volume.is_some()).count();
        Ok(Self {
            primitives,
            placements,
            branches,
            stats,
            proxy: None,
            proxy_id: 0,
        })
    }

    /// Build branch-local aggregates and one coarse subtree root without
    /// materializing the expanded tree. Optical diagnostics remain sampled;
    /// runtime use requires an explicit aggregate policy opt-in.
    pub fn cook_aggregate(source: &Assembly, config: crate::vegetation_aggregate::Config) -> Result<Self, String> {
        let mut plant = Self::cook_source(source)?;
        let roots: Vec<_> = plant.branches.iter().enumerate().filter(|(_,b)|b.parent.is_none()).map(|(i,_)|i).collect();
        if roots.len()!=1 || roots[0]!=0 { return Err("aggregate plants require one leading root".into()); }
        if !plant.placements.iter().any(|p|p.branch==0) {
            return Err("aggregate hierarchy currently requires source members on its root".into());
        }
        let cooker = crate::vegetation_aggregate::Cooker::new(source,config)?;
        for branch in 0..plant.branches.len() {
            let members: Vec<u32> = plant.placements.iter().enumerate().filter(|(_,p)|p.branch==branch).map(|(i,_)|i as u32).collect();
            if members.is_empty() { continue; }
            let asset = cooker.cook(&members,plant.branches[branch].assembly_rest)
                .map_err(|e|format!("aggregate branch {branch}: {e}"))?;
            plant.branches[branch].aggregate = aggregate_candidates(&asset,0.0);
            plant.branches[branch].volume = Some(asset.volume);
        }
        // Root represents every descendant at rest. Conservatively bound the
        // discarded relative animation with each branch's full ancestor chain.
        let motion = plant.branches.iter().enumerate().filter_map(|(i,b)|b.volume.as_ref().map(|a|wind_margin(&plant,i,a,1.0)))
            .fold(0.0,f32::max);
        let root_config = crate::vegetation_aggregate::Config {
            // Coarse subtree uses the same explicit work/memory caps. Callers
            // must budget for their full root; never silently enlarge a cap.
            reference_cell_size: config.reference_cell_size * 4.0,
            triangle_refinements: false,
            ..config
        };
        let root_cooker = crate::vegetation_aggregate::Cooker::new(source,root_config)?;
        let all:Vec<_>=(0..source.instances.len() as u32).collect();
        let root = root_cooker.cook_static_subtree(&all,0,plant.branches[0].assembly_rest)
            .map_err(|e|format!("aggregate subtree root: {e}"))?;
        plant.branches[0].aggregate = aggregate_candidates(&root,motion);
        plant.branches[0].aggregate_motion_error = motion;
        plant.branches[0].volume = Some(root.volume);
        plant.stats.volume_assets = plant.branches.iter().filter(|b|b.volume.is_some()).count();
        Ok(plant)
    }

    /// Also cook the whole plant as one voxel pyramid: finest cell
    /// `config.reference_cell_size`, then `levels - 1` doublings. A distant
    /// plant then costs one instance (SceneVegetation::append_proxy).
    pub fn cook_proxy(&mut self, source: &Assembly, config: crate::vegetation_aggregate::Config, levels: u32) -> Result<(), String> {
        let cooker = crate::vegetation_aggregate::Cooker::new(source, config)?;
        let asset = crate::vegetation_aggregate::cook_whole(&cooker, levels).map_err(|e| format!("plant proxy: {e}"))?;
        let root = self.branches.first().ok_or("plant proxy needs a root branch")?;
        if !root.assembly_rest.abs_diff_eq(source.branch_transforms()?[0], 1e-5) {
            return Err("plant proxy frame differs from the root branch".into());
        }
        self.proxy = Some(asset);
        self.proxy_id = next_proxy_id();
        Ok(())
    }

    pub fn proxy(&self) -> Option<&VoxelAsset> {
        self.proxy.as_ref()
    }

    pub fn stats(&self) -> CookStats {
        self.stats
    }

    /// Stable source placement ID and its branch ID; not scene buffer indices.
    pub fn source_ids(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.placements
            .iter()
            .map(|p| (p.id, self.branches[p.branch].id))
    }
}

fn aggregate_candidates(asset: &crate::vegetation_aggregate::ClusterAsset, motion:f32) -> Vec<crate::aggregate_lod::Candidate> {
    let mut candidates:Vec<_>=asset.triangle_refinements.iter().map(|m|crate::aggregate_lod::Candidate {
        source_only:true,level:0,spatial_error:m.cut_error_m+motion,
        voxel_coverage_error:1.0,voxel_color_error:1.0,
        triangle_coverage_error:m.sampled_error.rms_opacity as f32,
        triangle_color_error:m.sampled_error.premultiplied_albedo_rmse as f32,
        triangle_error:m.cut_error_m,
    }).collect();
    candidates.extend(asset.levels.iter().enumerate().map(|(i,m)|crate::aggregate_lod::Candidate {
        source_only:false,level:i as u32,
        spatial_error:1.5*3.0f32.sqrt()*asset.volume.levels[i].cell_size+motion,
        voxel_coverage_error:m.sampled_error.rms_opacity as f32,
        voxel_color_error:m.sampled_error.premultiplied_albedo_rmse as f32,
        triangle_coverage_error:m.triangle_sampled_error.rms_opacity as f32,
        triangle_color_error:m.triangle_sampled_error.premultiplied_albedo_rmse as f32,
        triangle_error:m.triangle_cut_error_m,
    }));
    candidates
}

#[derive(Clone, Copy, Debug)]
pub struct PlantInstance {
    /// World-stable plant identity, independent of camera or scene-array index.
    pub id: u64,
    pub generation: u32,
    /// Proper rotation, translation and positive scale. No shear/reflection.
    pub root: Mat4,
    pub category: GeometryCategory,
}

#[derive(Clone, Debug)]
pub struct PlantHandle {
    pub instance_range: Range<usize>,
    pub pose_range: Range<usize>,
    pub volume_range: Range<usize>,
    /// Original assembly branch order; source-only and empty rig branches have no proxy.
    pub branch_owners: Vec<Option<u32>>,
    /// Original assembly branch order; source-only branches have no volume representation.
    pub group_keys: Vec<Option<GroupKey>>,
    /// What Renderer::move_plant needs to move this full plant at runtime.
    pub movable: MovablePlant,
}

/// How a whole-plant proxy draws (Renderer::set_plant_proxies).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProxyShow {
    Shown = 0,
    /// A full assembly stands in for it, shadow included.
    Hidden = 1,
    /// A cheaper stand-in (a card tree) draws it; the proxy still casts sun.
    ShadowOnly = 2,
}

/// A full plant that can stand at another root without rebuilding the scene
/// (Renderer::move_plant): a pool of these streams full assemblies to the
/// trees nearest the cameras (recipe_forest::Pool).
#[derive(Clone, Debug)]
pub struct MovablePlant {
    /// The root it was appended with. Moves keep it rigid and may only shrink it.
    pub built: Mat4,
    /// Its root pose (PartWind index).
    pub pose: u32,
    /// Its aggregate groups, in Scene::aggregate_lod order.
    pub groups: Range<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct AttachLimits {
    pub max_poses: usize,
    pub max_primitive_instances: usize,
    pub max_primitive_meshes: usize,
    pub pack: PackLimits,
}

impl Default for AttachLimits {
    fn default() -> Self {
        Self {
            max_poses: MAX_PARTS,
            max_primitive_instances: 16_384,
            max_primitive_meshes: 256,
            pack: PackLimits {
                max_assets: 256,
                levels_per_asset: 16,
                first_level_bounds: false,
                max_dense_indices: 8_388_608,
                max_sparse_cells: 1_048_576,
                ..Default::default()
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttachmentReport {
    pub plants: usize,
    pub primitive_meshes: usize,
    pub primitive_instances: usize,
    pub branch_proxies: usize,
    pub pose_nodes: usize,
    pub volume_instances: usize,
    pub volume_assets: usize,
    /// Unique vertices referenced by the finest source DAG, per shared mesh.
    pub source_vertices_shared: usize,
    /// All resident vertex records, including coarse CLOD levels.
    pub resident_vertices_shared: usize,
    pub source_triangles_shared: usize,
    /// Complete resident mesh payload, including every CLOD level.
    pub shared_geometry_bytes: usize,
    /// Oracle vertex/index payload summed across placed plants, not allocated.
    pub expanded_source_bytes: usize,
    pub packed_volume_bytes: usize,
    /// Whole plants appended as one voxel instance each.
    pub plant_proxies: usize,
}

struct Checkpoint {
    vertices: usize,
    cluster_verts: usize,
    tris: usize,
    clusters: usize,
    meshes: usize,
    instances: usize,
    materials: usize,
    policy: Policy,
    lod_pixels: f32,
    active_tiles: bool,
    part_wind_lod: bool,
}

/// One scene-construction transaction. Normal scene geometry may already be
/// present, but the bounded experimental foliage pool must be empty. Drop or
/// failed finish removes this transaction's additions; completed assets/poses
/// are immutable input to the shared GPU attachment pass.
pub struct SceneVegetation<'a> {
    scene: &'a mut Scene,
    checkpoint: Checkpoint,
    limits: AttachLimits,
    meshes: BTreeMap<Vec<u8>, usize>,
    materials: BTreeMap<Vec<u8>, u32>,
    assets: Vec<VoxelAsset>,
    bindings: Vec<VolumeBinding>,
    ids: BTreeSet<u64>,
    report: AttachmentReport,
    committed: bool,
    aggregate_groups: Vec<crate::aggregate_lod::Group>,
    proxy_assets: Vec<VoxelAsset>,
    proxy_bindings: Vec<VolumeBinding>,
    /// CookedPlant::proxy_id: its proxy_assets index and support.
    proxy_lookup: std::collections::HashMap<u64, (usize, Vec3, Vec3)>,
}

fn max_axis(m: Mat4) -> f32 {
    [m.x_axis, m.y_axis, m.z_axis]
        .iter()
        .map(|a| a.truncate().length())
        .fold(0.0, f32::max)
}

fn validate_root(root: Mat4) -> Result<(), String> {
    if !root.is_finite() || !root.row(3).abs_diff_eq(Vec4::W, 1e-6) || !root.inverse().is_finite() {
        return Err("plant root must be a finite invertible affine transform".into());
    }
    let axes = [
        root.x_axis.truncate(),
        root.y_axis.truncate(),
        root.z_axis.truncate(),
    ];
    let lengths = axes.map(Vec3::length);
    if lengths
        .iter()
        .any(|n| !n.is_finite() || *n < 1e-6 || *n > 1e6)
    {
        return Err("plant root scale must be positive and bounded".into());
    }
    let unit = [
        axes[0] / lengths[0],
        axes[1] / lengths[1],
        axes[2] / lengths[2],
    ];
    if unit[0].dot(unit[1]).abs() > 1e-4
        || unit[1].dot(unit[2]).abs() > 1e-4
        || unit[0].dot(unit[2]).abs() > 1e-4
        || unit[0].dot(unit[1].cross(unit[2])) < 0.9999
    {
        return Err(
            "plant root requires proper rotation and positive scales, without shear/reflection"
                .into(),
        );
    }
    Ok(())
}

fn support(asset: &VoxelAsset) -> (Vec3, Vec3) {
    let mut lo = Vec3::from(asset.source_bounds[0]);
    let mut hi = Vec3::from(asset.source_bounds[1]);
    for level in &asset.levels {
        let bounds = level.local_bounds(asset.origin);
        let filter = Vec3::splat(level.cell_size * 0.5);
        lo = lo.min(Vec3::from(bounds[0]) - filter);
        hi = hi.max(Vec3::from(bounds[1]) + filter);
    }
    (lo, hi)
}

/// Triangle inequality over the entire ancestor chain. The lever arm is the
/// sum of pivot distances plus the descendant support radius, so ancestor
/// rotations cannot invalidate a rest-distance-only estimate.
fn wind_margin(plant: &CookedPlant, branch: usize, asset: &VoxelAsset, root_scale: f32) -> f32 {
    let (lo, hi) = support(asset);
    let mut lever = lo.abs().max(hi.abs()).length();
    let mut current = branch;
    let mut margin = 0.0;
    loop {
        let b = &plant.branches[current];
        margin += 2.0 * (0.5 * b.max_angle).sin() * lever;
        let Some(parent) = b.parent else {
            break;
        };
        lever += b.rest.w_axis.truncate().length();
        current = parent as usize;
    }
    margin * root_scale
}

fn phase(id: u64, branch: u64) -> f32 {
    let mut x = id ^ branch;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 40) as f32 / 16_777_216.0 * std::f32::consts::TAU
}

impl<'a> SceneVegetation<'a> {
    pub fn new(scene: &'a mut Scene, limits: AttachLimits) -> Result<Self, String> {
        if scene.voxel_forest.is_some()
            || !scene.part_wind.is_empty()
            || !scene.foliage_representations.is_empty()
            || !scene.foliage_source_groups.is_empty()
            || !scene.part_attachments.is_empty()
            || scene.aggregate_lod.is_some()
        {
            return Err("shared vegetation requires an unused bounded foliage/part pool".into());
        }
        if limits.max_primitive_instances == 0
            || limits.max_primitive_instances > crate::part_wind::RuntimeLimits::default().max_attachments
            || limits.max_poses == 0
            || limits.max_poses > crate::part_wind::RuntimeLimits::default().max_poses
            || limits.max_primitive_meshes == 0
            || limits.pack.max_instances > foliage_volume_data::MAX_INSTANCES
            || limits.pack.first_level_bounds
            || limits.pack.levels_per_asset == 0
            || limits.pack.levels_per_asset > crate::foliage_representation::MAX_LEVELS
        {
            return Err("invalid shared vegetation capacity or mip-support limits".into());
        }
        let checkpoint = Checkpoint {
            vertices: scene.vertices.len(),
            cluster_verts: scene.cluster_verts.len(),
            tris: scene.tris.len(),
            clusters: scene.clusters.len(),
            meshes: scene.meshes.len(),
            instances: scene.instances.len(),
            materials: scene.materials.len(),
            policy: scene.foliage_representation_policy,
            lod_pixels: scene.voxel_lod_pixels,
            active_tiles: scene.voxel_active_tiles,
            part_wind_lod: scene.part_wind_lod,
        };
        Ok(Self {
            scene,
            checkpoint,
            limits,
            meshes: BTreeMap::new(),
            materials: BTreeMap::new(),
            assets: Vec::new(),
            bindings: Vec::new(),
            ids: BTreeSet::new(),
            report: AttachmentReport::default(),
            committed: false,
            aggregate_groups: Vec::new(),
            proxy_assets: Vec::new(),
            proxy_bindings: Vec::new(),
            proxy_lookup: std::collections::HashMap::new(),
        })
    }

    pub fn append(
        &mut self,
        plant: &CookedPlant,
        instance: PlantInstance,
    ) -> Result<PlantHandle, String> {
        validate_root(instance.root)?;
        if self.ids.contains(&instance.id) {
            return Err("duplicate stable plant ID in scene vegetation".into());
        }
        if self.scene.part_wind.len() + plant.branches.len() > self.limits.max_poses
            || self.bindings.len() + plant.stats.volume_assets > self.limits.pack.max_instances
            || self.report.primitive_instances + plant.placements.len()
                > self.limits.max_primitive_instances
        {
            return Err("shared vegetation exceeds explicit pose/volume/member capacity".into());
        }
        let new_meshes = plant
            .primitives
            .iter()
            .filter(|p| !self.meshes.contains_key(&p.key))
            .count();
        if self.meshes.len() + new_meshes > self.limits.max_primitive_meshes {
            return Err("shared vegetation primitive mesh capacity exceeded".into());
        }
        if self
            .scene
            .instances
            .len()
            .checked_add(plant.branches.len() + plant.placements.len())
            .is_none_or(|n| n > u32::MAX as usize)
        {
            return Err("shared vegetation instance index overflow".into());
        }
        let pose_base = self.scene.part_wind.len();
        let owner_base = self.scene.instances.len();
        let volume_base = self.bindings.len();
        let scale = max_axis(instance.root);
        let aggregate_base = self.aggregate_groups.len();
        // Prepare every fallible geometric decision before appending source
        // meshes/instances. No partial plant is committed on this error path.
        let mut groups = Vec::with_capacity(plant.branches.len());
        for (i, branch) in plant.branches.iter().enumerate() {
            let value = if let Some(asset) = &branch.volume {
                if asset.levels.len() > self.limits.pack.levels_per_asset {
                    return Err(
                        "shared vegetation pack must retain every selected asset mip".into(),
                    );
                }
                let rest = instance.root * branch.assembly_rest;
                let group = Group::from_voxels(
                    GroupKey {
                        instance: instance.id,
                        branch: branch.id,
                        generation: instance.generation,
                    },
                    asset,
                    rest,
                    wind_margin(plant, i, asset, scale) + branch.aggregate_motion_error * scale,
                    OpticalEvidence::Unvalidated,
                )?;
                Some(group)
            } else {
                None
            };
            groups.push(value);
        }
        let mut asset_indices = vec![None; plant.branches.len()];
        let mut new_assets = Vec::<VoxelAsset>::new();
        for (i, branch) in plant.branches.iter().enumerate() {
            if let Some(asset) = &branch.volume {
                let index = self
                    .assets
                    .iter()
                    .position(|a| a == asset)
                    .unwrap_or_else(|| {
                        self.assets.len()
                            + new_assets
                                .iter()
                                .position(|a| a == asset)
                                .unwrap_or_else(|| {
                                    new_assets.push(asset.clone());
                                    new_assets.len() - 1
                                })
                    });
                asset_indices[i] = Some(index as u32);
            }
        }
        if self.assets.len() + new_assets.len() > self.limits.pack.max_assets {
            return Err("shared vegetation volume asset capacity exceeded".into());
        }
        self.assets.extend(new_assets);
        let mut mesh_indices = Vec::with_capacity(plant.primitives.len());
        for primitive in &plant.primitives {
            let mesh = if let Some(&mesh) = self.meshes.get(&primitive.key) {
                mesh
            } else {
                let material_key = bytemuck::bytes_of(&primitive.material).to_vec();
                let material = if let Some(&id) = self.materials.get(&material_key) {
                    id
                } else {
                    let id = self.scene.add_material(primitive.material);
                    self.materials.insert(material_key, id);
                    id
                };
                let first = self.scene.clusters.len();
                let mesh = self.scene.add_mesh(&primitive.mesh);
                for cluster in &mut self.scene.clusters[first..] {
                    cluster.material = material;
                }
                self.report.source_vertices_shared += finest_vertex_count(&primitive.mesh);
                self.report.resident_vertices_shared += primitive.mesh.vertices.len();
                self.report.source_triangles_shared += primitive.mesh.leaf_triangles();
                self.report.shared_geometry_bytes += mesh_bytes(&primitive.mesh);
                self.meshes.insert(primitive.key.clone(), mesh);
                mesh
            };
            mesh_indices.push(mesh);
        }
        let mut branch_owners = vec![None; plant.branches.len()];
        let mut group_keys = vec![None; plant.branches.len()];
        for (i, branch) in plant.branches.iter().enumerate() {
            let rest = instance.root * branch.assembly_rest;
            let (owner, volume) = if let Some(group) = groups[i].take() {
                let asset = branch.volume.as_ref().unwrap();
                let (lo, hi) = support(asset);
                let centre = rest.transform_point3((lo + hi) * 0.5);
                let owner = self.scene.instances.len() as u32;
                let mut proxy = GpuInstance {
                    rows: [
                        rest.row(0).to_array(),
                        rest.row(1).to_array(),
                        rest.row(2).to_array(),
                    ],
                    sphere: centre.extend((hi - lo).length() * 0.5 * scale).to_array(),
                    first_cluster: 0,
                    cluster_count: 0,
                    scale,
                    flags: PART_WIND,
                };
                proxy.set_category(instance.category);
                self.scene.instances.push(proxy);
                let volume = self.bindings.len() as u32;
                self.bindings.push(VolumeBinding {
                    owner,
                    asset: asset_indices[i].unwrap(),
                    crown_material: ALL_MATERIALS,
                });
                if !branch.aggregate.is_empty() {
                    self.aggregate_groups.push(crate::aggregate_lod::Group {
                        volume,center:group.center,radius:group.radius,scale,
                        parent:if i==0 {None} else {Some(aggregate_base as u32)},
                        candidates:branch.aggregate.clone(),
                    });
                }
                group_keys[i] = Some(group.key);
                self.scene.foliage_representations.push(Binding {
                    owner,
                    volume,
                    group,
                });
                branch_owners[i] = Some(owner);
                (Some(owner), Some(volume))
            } else {
                (None, None)
            };
            let mut members = Vec::new();
            for placement in plant.placements.iter().filter(|p| p.branch == i) {
                let mesh = mesh_indices[placement.primitive];
                let member = self.scene.instances.len() as u32;
                self.scene.add_classified_instance(
                    mesh,
                    Affine3A::from_mat4(rest * placement.rest),
                    instance.category,
                );
                let source = self.scene.instances.last_mut().unwrap();
                source.flags |= PART_WIND;
                // Keep Scene's scale/radius convention here. The shared
                // attachment pass derives the primitive-local sphere and
                // applies the conservative root x rest stretch once.
                self.scene.part_attachments.push(PartAttachment {
                    pose: (pose_base + i) as u32,
                    owner: member,
                    rest: placement.rest,
                });
                members.push(member);
            }
            if let Some(owner) = owner {
                self.scene.foliage_source_groups.push(SourceGroup { owner, members });
            }
            self.scene.part_wind.push(PartWind {
                parent: branch.parent.map(|p| pose_base as u32 + p),
                owner,
                volume,
                rest: if branch.parent.is_none() {
                    instance.root * branch.rest
                } else {
                    branch.rest
                },
                max_angle: branch.max_angle,
                harmonic: 7 + (branch.id % 13) as u32,
                phase: phase(instance.id, branch.id),
            });
        }
        self.ids.insert(instance.id);
        self.report.plants += 1;
        self.report.primitive_instances += plant.placements.len();
        self.report.expanded_source_bytes += plant.stats.expanded_source_bytes;
        self.report.branch_proxies += plant.stats.volume_assets;
        self.report.pose_nodes = self.scene.part_wind.len();
        self.report.volume_instances = self.bindings.len();
        self.report.volume_assets = self.assets.len();
        self.report.primitive_meshes = self.meshes.len();
        Ok(PlantHandle {
            instance_range: owner_base..self.scene.instances.len(),
            pose_range: pose_base..self.scene.part_wind.len(),
            volume_range: volume_base..self.bindings.len(),
            branch_owners,
            group_keys,
            movable: MovablePlant { built: instance.root, pose: pose_base as u32,
                groups: aggregate_base as u32..self.aggregate_groups.len() as u32 },
        })
    }

    /// The whole plant as one voxel instance (CookedPlant::cook_proxy): no
    /// branches, members or wind. For plants past the distance where their
    /// full assembly is worth drawing. finish_aggregate packs them after the
    /// aggregate plants; they draw and cast sun at any distance.
    pub fn append_proxy(&mut self, plant: &CookedPlant, instance: PlantInstance) -> Result<usize, String> {
        validate_root(instance.root)?;
        if self.ids.contains(&instance.id) {
            return Err("duplicate stable plant ID in scene vegetation".into());
        }
        let asset = plant.proxy.as_ref().ok_or("plant proxy missing: call CookedPlant::cook_proxy")?;
        // Tens of thousands of trees share a few proxies: look them up by
        // cook, never by comparing voxels.
        let (index, lo, hi) = match self.proxy_lookup.get(&plant.proxy_id) {
            Some(&found) => found,
            None => {
                let index = match self.proxy_assets.iter().position(|a| a == asset) {
                    Some(index) => index,
                    None => {
                        if self.proxy_assets.len() >= self.limits.pack.max_assets {
                            return Err("shared vegetation proxy asset capacity exceeded".into());
                        }
                        self.proxy_assets.push(asset.clone());
                        self.proxy_assets.len() - 1
                    }
                };
                // The plant itself, not its voxel support: the canopy volume
                // (GI, sky light) sizes the crown from this sphere, and the
                // support's padded coarse cells made crowns 1.8x too wide,
                // darkening clearings. The voxel pass culls by pack_tree's
                // own (padded) level bounds.
                let [lo, hi] = asset.source_bounds.map(Vec3::from);
                self.proxy_lookup.insert(plant.proxy_id, (index, lo, hi));
                (index, lo, hi)
            }
        };
        let rest = instance.root * plant.branches[0].assembly_rest;
        let scale = max_axis(instance.root);
        let centre = rest.transform_point3((lo + hi) * 0.5);
        let owner = u32::try_from(self.scene.instances.len()).map_err(|_| "shared vegetation instance index overflow")?;
        let mut proxy = GpuInstance {
            rows: [rest.row(0).to_array(), rest.row(1).to_array(), rest.row(2).to_array()],
            sphere: centre.extend((hi - lo).length() * 0.5 * scale).to_array(),
            first_cluster: 0,
            cluster_count: 0,
            scale,
            // The canopy volume (GI, sky light) counts it like any tree.
            flags: FOLIAGE,
        };
        proxy.set_category(instance.category);
        self.scene.instances.push(proxy);
        self.proxy_bindings.push(VolumeBinding { owner, asset: index as u32, crown_material: ALL_MATERIALS });
        self.ids.insert(instance.id);
        self.report.plant_proxies += 1;
        Ok(owner as usize)
    }

    /// Commit source-only assemblies. Optically cooked assets must use finish
    /// instead; never silently discard their requested volume representations.
    pub fn finish_source(mut self) -> Result<AttachmentReport, String> {
        if self.report.plants == 0 {
            return Err("cannot finish an empty vegetation attachment".into());
        }
        if !self.assets.is_empty() || !self.bindings.is_empty() || !self.proxy_bindings.is_empty() {
            return Err("source-only finish requires plants cooked with cook_source".into());
        }
        crate::part_wind::validate_bindings(&self.scene.part_wind, &self.scene.part_attachments,
            &self.scene.instances, None)?;
        self.committed = true;
        Ok(self.report.clone())
    }

    pub fn finish_aggregate(mut self, policy: crate::aggregate_lod::Policy) -> Result<AttachmentReport, String> {
        if (self.aggregate_groups.is_empty() && self.proxy_bindings.is_empty()) || self.aggregate_groups.len()!=self.bindings.len() {
            return Err("aggregate finish requires exclusively aggregate-cooked plants".into());
        }
        if !(policy.proxy_pixels_from.is_finite() && policy.proxy_pixels_from>=0.0) {
            return Err("aggregate proxy pixel growth distance must be finite and non-negative".into());
        }
        if !(policy.proxy_pixels.is_finite() && policy.proxy_pixels>0.0) {
            return Err("aggregate proxy pixel error must be positive".into());
        }
        let selection=crate::aggregate_lod::Selection { groups:self.aggregate_groups.clone(),policy };
        let mut forest=if self.bindings.is_empty() {foliage_volume_data::GpuForest::default()} else {
            crate::aggregate_lod::validate(&selection)?;
            let mut forest=foliage_volume_data::pack(&self.assets,&self.bindings,&self.scene.instances,self.limits.pack)?;
            foliage_volume_data::pack_bricks(&mut forest,128)?;
            forest
        };
        // Whole-plant proxies follow, densely indexed: they are larger than
        // a 128-brick branch cluster and only the rasterized path draws them.
        if !self.proxy_bindings.is_empty() {
            let cells=self.proxy_assets.iter().flat_map(|a|&a.levels).map(|l|l.voxels.len()).sum();
            let packed=foliage_volume_data::pack_assets(&self.proxy_assets,foliage_volume_data::AssetPackLimits {
                max_assets:self.proxy_assets.len(),levels_per_asset:crate::foliage_representation::MAX_LEVELS,first_level_bounds:false,
                max_dense_indices:1<<28,max_sparse_cells:cells})?;
            let mut proxies=packed.forest.clone();
            for binding in &self.proxy_bindings {
                proxies.instances.push(packed.instance(*binding,&self.scene.instances[binding.owner as usize])?);
            }
            forest.append(proxies)?;
        }
        self.report.packed_volume_bytes=forest.byte_len();
        self.scene.voxel_proxies=self.proxy_bindings.len() as u32;
        self.scene.voxel_proxy_pixels=policy.proxy_pixels;
        self.scene.voxel_proxy_pixels_from=policy.proxy_pixels_from;
        self.scene.voxel_forest=Some(forest);
        self.scene.aggregate_lod=(!self.aggregate_groups.is_empty()).then_some(selection);
        crate::aggregate_lod::validate_scene(self.scene)?;
        self.scene.part_wind_lod=true;
        self.scene.voxel_lod_pixels=policy.max_error_pixels;
        self.scene.voxel_active_tiles=true;
        crate::foliage_representation::validate_bindings(self.scene)?;
        crate::part_wind::validate_bindings(&self.scene.part_wind,&self.scene.part_attachments,
            &self.scene.instances,self.scene.voxel_forest.as_ref())?;
        self.committed=true;
        Ok(self.report.clone())
    }

    pub fn finish(mut self, policy: Policy) -> Result<AttachmentReport, String> {
        if self.report.plants == 0 {
            return Err("cannot finish an empty vegetation attachment".into());
        }
        if self.bindings.is_empty() {
            return Err("source-only vegetation must use finish_source".into());
        }
        let forest = foliage_volume_data::pack(
            &self.assets,
            &self.bindings,
            &self.scene.instances,
            self.limits.pack,
        )?;
        self.report.packed_volume_bytes = forest.byte_len();
        self.scene.voxel_forest = Some(forest);
        self.scene.foliage_representation_policy = policy;
        self.scene.voxel_lod_pixels = policy.max_error_pixels;
        self.scene.voxel_active_tiles = true;
        crate::foliage_representation::validate_bindings(self.scene)?;
        // Use the same CPU validation as GPU upload, including composed
        // attachment transforms and bounds, while rollback is still possible.
        crate::part_wind::validate_bindings(&self.scene.part_wind, &self.scene.part_attachments,
            &self.scene.instances, self.scene.voxel_forest.as_ref())?;
        self.committed = true;
        Ok(self.report.clone())
    }
}

impl Drop for SceneVegetation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let c = &self.checkpoint;
        self.scene.vertices.truncate(c.vertices);
        self.scene.cluster_verts.truncate(c.cluster_verts);
        self.scene.tris.truncate(c.tris);
        self.scene.clusters.truncate(c.clusters);
        self.scene.meshes.truncate(c.meshes);
        self.scene.instances.truncate(c.instances);
        self.scene.materials.truncate(c.materials);
        self.scene.voxel_forest = None;
        self.scene.voxel_proxies = 0;
        self.scene.voxel_proxy_pixels_from = 0.0;
        self.scene.part_wind.clear();
        self.scene.part_attachments.clear();
        self.scene.foliage_representations.clear();
        self.scene.foliage_source_groups.clear();
        self.scene.foliage_representation_policy = c.policy;
        self.scene.voxel_lod_pixels = c.lod_pixels;
        self.scene.voxel_active_tiles = c.active_tiles;
        self.scene.aggregate_lod = None;
        self.scene.part_wind_lod = c.part_wind_lod;
    }
}

fn next_proxy_id() -> u64 {
    static NEXT_PROXY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_PROXY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// Cooked plants on disk (CookedPlant::cached): a header, then the plant in
// postcard. The cook code itself is hashed into every key, so a cook that
// changes never loads an old file; bump the version when only the file
// layout changes.
const COOK_MAGIC: &[u8; 8] = b"WARPCOOK";
const COOK_FILE_VERSION: u32 = 1;
const COOK_SOURCES: [&str; 8] = [include_str!("vegetation_asset.rs"), include_str!("vegetation_aggregate.rs"),
    include_str!("vegetation_clod.rs"), include_str!("foliage_voxel.rs"), include_str!("foliage_assembly.rs"),
    include_str!("plant_recipe.rs"), include_str!("cluster.rs"), include_str!("aggregate_lod.rs")];

fn fnv64(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

/// The cache key for a cook: `what` names everything it depends on outside
/// the engine (recipe, build seed, configs; their Debug text will do).
pub fn cook_key(what: &str) -> u64 {
    static CODE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let code = *CODE.get_or_init(|| COOK_SOURCES.iter().fold(0xcbf29ce484222325, |h, s| fnv64(h, s.as_bytes())));
    fnv64(fnv64(code, &COOK_FILE_VERSION.to_le_bytes()), what.as_bytes())
}

impl CookedPlant {
    /// Write this plant to `path` under `key` (cook_key), atomically.
    pub fn save(&self, path: &std::path::Path, key: u64) -> Result<(), String> {
        let body = postcard::to_allocvec(self).map_err(|e| format!("cooked plant encode: {e}"))?;
        let mut bytes = Vec::with_capacity(body.len() + 36);
        bytes.extend_from_slice(COOK_MAGIC);
        bytes.extend_from_slice(&COOK_FILE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&fnv64(0xcbf29ce484222325, &body).to_le_bytes());
        bytes.extend_from_slice(&body);
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?; }
        let partial = path.with_extension(format!("partial{}", std::process::id()));
        std::fs::write(&partial, &bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
        std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read a plant saved under `key`; any other key, version or damage is
    /// an error (the caller cooks again).
    pub fn load(path: &std::path::Path, key: u64) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let field = |at: usize| bytes.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
        if bytes.len() < 36 || &bytes[..8] != COOK_MAGIC || bytes[8..12] != COOK_FILE_VERSION.to_le_bytes() {
            return Err(format!("{}: not a cooked plant of this version", path.display()));
        }
        if field(12) != Some(key) { return Err(format!("{}: cooked for another key", path.display())); }
        let body = &bytes[36..];
        if field(20) != Some(body.len() as u64) || field(28) != Some(fnv64(0xcbf29ce484222325, body)) {
            return Err(format!("{}: truncated or damaged", path.display()));
        }
        let mut plant: Self = postcard::from_bytes(body).map_err(|e| format!("{}: {e}", path.display()))?;
        if plant.proxy.is_some() { plant.proxy_id = next_proxy_id(); }
        Ok(plant)
    }

    /// The plant `what` describes (cook_key) from `dir`, or from `cook`
    /// when no valid file is there yet, saving it for next time. Returns
    /// whether it came from disk. A failed save only costs the next start.
    pub fn cached(dir: &std::path::Path, what: &str, cook: impl FnOnce() -> Result<Self, String>) -> Result<(Self, bool), String> {
        let key = cook_key(what);
        let path = dir.join(format!("{key:016x}.cook"));
        if let Ok(plant) = Self::load(&path, key) { return Ok((plant, true)); }
        let plant = cook()?;
        let _ = plant.save(&path, key);
        Ok((plant, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        foliage_assembly::{self, Limits, Parameters},
        foliage_voxel::ProjectionMeasure,
    };
    use glam::{Mat3, Quat};

    fn cook(p: Parameters) -> (Assembly, CookedPlant) {
        let source = foliage_assembly::generate(p, Limits::default()).unwrap();
        let cooked = CookedPlant::cook(
            &source,
            BakeConfig {
                projection_measure: ProjectionMeasure::ClosedSurfaceEntries,
                cell_size: 0.08,
                levels: 2,
                samples_per_cell: 2,
                max_samples: 1_000_000,
                max_voxels: 65_536,
                max_texture_texels: 0,
                ..Default::default()
            },
        )
        .unwrap();
        (source, cooked)
    }
    fn instance(id: u64, root: Mat4) -> PlantInstance {
        PlantInstance {
            id,
            generation: 3,
            root,
            category: GeometryCategory::Grass,
        }
    }
    fn matrix(instance: &GpuInstance) -> Mat4 {
        let r = instance.rows;
        Mat4::from_cols(
            Vec4::new(r[0][0], r[1][0], r[2][0], 0.0),
            Vec4::new(r[0][1], r[1][1], r[2][1], 0.0),
            Vec4::new(r[0][2], r[1][2], r[2][2], 0.0),
            Vec4::new(r[0][3], r[1][3], r[2][3], 1.0),
        )
    }

    #[test]
    fn equal_primitives_across_variants_and_plants_share_mesh_payload() {
        let (_, a) = cook(Parameters::grass(101));
        let (_, b) = cook(Parameters::grass(303));
        let mut scene = Scene::default();
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        builder.append(&a, instance(1, Mat4::IDENTITY)).unwrap();
        let first_bytes = builder.report.shared_geometry_bytes;
        let first_vertices = builder.scene.vertices.len();
        builder
            .append(&b, instance(2, Mat4::from_translation(Vec3::X)))
            .unwrap();
        builder
            .append(&a, instance(3, Mat4::from_translation(Vec3::Z)))
            .unwrap();
        // Both recipes use the same three shapes/material palette. Some
        // combinations may first appear in b; a repeated plant adds none.
        let expected: BTreeSet<_> = a
            .primitives
            .iter()
            .chain(&b.primitives)
            .map(|p| &p.key)
            .collect();
        assert_eq!(builder.meshes.len(), expected.len());
        assert!(first_vertices < a.stats.expanded_source_vertices);
        assert!(
            first_bytes
                < a.stats.expanded_source_vertices * std::mem::size_of::<crate::cluster::Vertex>()
        );
        let before_repeat = builder.report.shared_geometry_bytes;
        builder
            .append(&b, instance(4, Mat4::from_translation(Vec3::Y)))
            .unwrap();
        assert_eq!(builder.report.shared_geometry_bytes, before_repeat);
        let report = builder.finish(Policy::default()).unwrap();
        assert_eq!(report.primitive_instances, 4 * 36);
        assert_eq!(report.pose_nodes, 20);
        assert_eq!(report.branch_proxies, 16);
        assert!(
            scene
                .foliage_source_groups
                .iter()
                .all(|g| scene.instances[g.owner as usize].cluster_count == 0)
        );
        assert_eq!(scene.part_attachments.len(), report.primitive_instances);
        crate::foliage_representation::validate_bindings(&scene).unwrap();
    }

    #[test]
    fn source_and_resident_vertex_counts_remain_separate_with_shared_clod() {
        let (source, cooked) = cook(Parameters::grass(101));
        let unique_parts: BTreeSet<_> = source.instances.iter().map(|p| (p.part, p.material)).collect();
        let expected_source_vertices: usize = unique_parts.iter()
            .map(|&(part, _)| source.parts[part as usize].vertices.len()).sum();
        let expected_source_triangles: usize = unique_parts.iter()
            .map(|&(part, _)| source.parts[part as usize].indices.len() / 3).sum();
        let mut scene = Scene::default();
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        builder.append(&cooked, instance(1, Mat4::IDENTITY)).unwrap();
        let first_source = builder.report.source_vertices_shared;
        let first_resident = builder.report.resident_vertices_shared;
        let first_bytes = builder.report.shared_geometry_bytes;
        builder.append(&cooked, instance(2, Mat4::from_translation(Vec3::X))).unwrap();
        let report = builder.finish(Policy::default()).unwrap();
        assert_eq!(report.source_vertices_shared, expected_source_vertices);
        assert_eq!(report.source_triangles_shared, expected_source_triangles);
        assert_eq!(report.resident_vertices_shared, scene.vertices.len());
        assert!(report.resident_vertices_shared > report.source_vertices_shared);
        assert_eq!((report.source_vertices_shared, report.resident_vertices_shared, report.shared_geometry_bytes),
            (first_source, first_resident, first_bytes));
        assert_eq!(report.shared_geometry_bytes, cooked.primitives.iter().map(|p| mesh_bytes(&p.mesh)).sum::<usize>());
    }

    #[test]
    fn member_vertices_normals_and_rest_poses_equal_expanded_oracle() {
        for parameters in [Parameters::grass(5), Parameters::conifer_twig(7)] {
            let (source, optical) = cook(parameters);
            for (cooked, source_only) in [(optical, false), (CookedPlant::cook_source(&source).unwrap(), true)] {
                let oracle = source.flatten_rest().unwrap();
                let root = Mat4::from_scale_rotation_translation(
                    Vec3::new(1.2, 0.8, 1.4),
                    Quat::from_rotation_y(0.43),
                    Vec3::new(3.0, 1.0, -7.0),
                );
                let root_normal = Mat3::from_mat4(root).inverse().transpose();
                let mut scene = Scene::default();
                let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
                let handle = builder.append(&cooked, instance(19, root)).unwrap();
                if source_only { builder.finish_source().unwrap(); }
                else { builder.finish(Policy::default()).unwrap(); }
                let poses = crate::part_wind::sample_poses(
                    &scene.part_wind,
                    &crate::wind::WindState::default(),
                )
                .unwrap();
                for attachment in &scene.part_attachments {
                    let local_branch = attachment.pose as usize - handle.pose_range.start;
                    let members: Vec<_> = scene
                        .part_attachments
                        .iter()
                        .filter(|a| a.pose == attachment.pose)
                        .collect();
                    let member_index = members
                        .iter()
                        .position(|a| a.owner == attachment.owner)
                        .unwrap();
                    let placement = cooked
                        .placements
                        .iter()
                        .filter(|p| p.branch == local_branch)
                        .nth(member_index)
                        .unwrap();
                    let primitive = &cooked.primitives[placement.primitive];
                    let expected = oracle
                        .iter()
                        .find(|p| p.identity.instance == placement.id)
                        .unwrap();
                    let actual = matrix(&scene.instances[attachment.owner as usize]);
                    assert!(
                        actual.abs_diff_eq(poses[attachment.pose as usize] * attachment.rest, 2e-5)
                    );
                    // Resident meshes now also contain coarse CLOD vertices.
                    // Reconstruct the actual finest DAG cut, and prove it still
                    // references every original vertex and oriented triangle
                    // exactly once before comparing the transformed source.
                    let triangle_key = |t: [u32; 3]| {
                        let first = if t[0] <= t[1] && t[0] <= t[2] { 0 }
                            else if t[1] <= t[2] { 1 } else { 2 };
                        [t[first], t[(first + 1) % 3], t[(first + 2) % 3]]
                    };
                    let mut source_vertices = BTreeSet::new();
                    let mut actual_triangles = BTreeMap::new();
                    for cluster in primitive.mesh.clusters.iter().filter(|c| c.own_error == 0.0) {
                        for &packed in &primitive.mesh.tris[cluster.first_tri as usize..][..cluster.tri_count as usize] {
                            let indices = [packed & 255, (packed >> 8) & 255, (packed >> 16) & 255]
                                .map(|local| primitive.mesh.cluster_verts[(cluster.first_vert + local) as usize]);
                            source_vertices.extend(indices);
                            *actual_triangles.entry(triangle_key(indices)).or_insert(0usize) += 1;
                        }
                    }
                    let mut expected_triangles = BTreeMap::new();
                    for triangle in expected.indices.chunks_exact(3) {
                        *expected_triangles.entry(triangle_key([triangle[0], triangle[1], triangle[2]]))
                            .or_insert(0usize) += 1;
                    }
                    assert_eq!(actual_triangles, expected_triangles);
                    assert_eq!(source_vertices, (0..expected.vertices.len() as u32).collect());
                    let normal_matrix = Mat3::from_mat4(actual).inverse().transpose();
                    for (&index, expanded) in source_vertices.iter().zip(&expected.vertices) {
                        let vertex = &primitive.mesh.vertices[index as usize];
                        let p = actual.transform_point3(Vec3::from(vertex.pos));
                        let n = (normal_matrix * Vec3::from(vertex.normal)).normalize();
                        let expected_p = root.transform_point3(Vec3::from(expanded.pos));
                        let expected_n = (root_normal * Vec3::from(expanded.normal)).normalize();
                        assert!(p.distance(expected_p) < 3e-5);
                        assert!(n.distance(expected_n) < 3e-5);
                        assert_eq!(vertex.uv, expanded.uv);
                    }
                    assert_eq!(source_vertices.len(), expected.vertices.len());
                }
            }
        }
    }

    #[test]
    fn source_only_repeated_plants_share_geometry_without_volume_resources() {
        let source = foliage_assembly::generate(Parameters::grass(101), Limits::default()).unwrap();
        let cooked = CookedPlant::cook_source(&source).unwrap();
        assert_eq!(cooked.stats.volume_assets, 0);
        assert!(cooked.branches.iter().all(|b| b.volume.is_none()));
        let mut scene = Scene::default();
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        let first = builder.append(&cooked, instance(1, Mat4::IDENTITY)).unwrap();
        let bytes = builder.report.shared_geometry_bytes;
        let vertices = builder.scene.vertices.len();
        let second = builder.append(&cooked, instance(2, Mat4::from_translation(Vec3::X))).unwrap();
        assert_eq!(builder.report.shared_geometry_bytes, bytes);
        assert_eq!(builder.scene.vertices.len(), vertices);
        for handle in [first, second] {
            assert_eq!(handle.instance_range.len(), source.instances.len());
            assert!(handle.volume_range.is_empty());
            assert!(handle.branch_owners.iter().all(Option::is_none));
            assert!(handle.group_keys.iter().all(Option::is_none));
        }
        let report = builder.finish_source().unwrap();
        assert_eq!(report.primitive_instances, source.instances.len() * 2);
        assert_eq!(report.pose_nodes, source.branches.len() * 2);
        assert_eq!(report.source_triangles_shared, cooked.stats.primitive_triangles);
        assert_eq!(report.branch_proxies + report.volume_instances + report.volume_assets + report.packed_volume_bytes, 0);
        assert!(scene.voxel_forest.is_none());
        assert!(scene.foliage_representations.is_empty() && scene.foliage_source_groups.is_empty());
        assert_eq!(scene.instances.len(), report.primitive_instances);
        assert_eq!(scene.part_attachments.len(), report.primitive_instances);
        assert!(scene.instances.iter().all(|i| i.cluster_count > 0 && i.flags & PART_WIND != 0));
        assert!(scene.part_wind.iter().all(|p| p.owner.is_none() && p.volume.is_none()));
        crate::foliage_representation::validate_bindings(&scene).unwrap();
    }

    #[test]
    fn source_only_disconnected_leaf_spray_retains_all_source_triangles() {
        let mut source = foliage_assembly::generate(Parameters::grass(17), Limits::default()).unwrap();
        let first = source.instances[0].clone();
        let part = &mut source.parts[first.part as usize];
        let leaf = part.clone();
        part.vertices.clear();
        part.indices.clear();
        for copy in 0..16 {
            let offset = part.vertices.len() as u32;
            let translation = Vec3::new((copy % 4) as f32 * 0.13, (copy / 4) as f32 * 0.17, 0.0);
            part.vertices.extend(leaf.vertices.iter().map(|v| {
                let mut v = *v;
                v.pos = (Vec3::from(v.pos) + translation).to_array();
                v
            }));
            part.indices.extend(leaf.indices.iter().map(|i| i + offset));
        }
        assert!(crate::vegetation_clod::build(&Part { vertices: &part.vertices,
            indices: &part.indices, material: first.material }).is_none());
        let source_triangles = part.indices.len() / 3;
        let source_vertices = part.vertices.len();
        source.instances = vec![first.clone(), crate::foliage_assembly::PartInstance {
            id: first.id.wrapping_add(1), rest: Mat4::from_translation(Vec3::X * 2.0) * first.rest, ..first
        }];
        let cooked = CookedPlant::cook_source(&source).unwrap();
        assert_eq!(cooked.stats.primitive_meshes, 1);
        assert_eq!(cooked.stats.primitive_triangles, source_triangles);
        assert_eq!(cooked.stats.expanded_source_triangles, source_triangles * 2);
        assert_eq!(finest_vertex_count(&cooked.primitives[0].mesh), source_vertices);
        let mut scene = Scene::default();
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        builder.append(&cooked, instance(1, Mat4::IDENTITY)).unwrap();
        let report = builder.finish_source().unwrap();
        assert_eq!(report.primitive_instances, 2);
        assert_eq!(report.source_triangles_shared, source_triangles);
        assert_eq!(scene.instances[0].first_cluster, scene.instances[1].first_cluster);
        assert!(scene.voxel_forest.is_none());
    }

    #[test]
    fn mixed_cooks_share_only_matching_clod_policies_in_either_append_order() {
        for disconnected in [false, true] {
            let mut source = foliage_assembly::generate(
                Parameters { density: 1, ..Parameters::grass(17) }, Limits::default()).unwrap();
            if disconnected {
                let part = &mut source.parts[source.instances[0].part as usize];
                let leaf = part.clone();
                let offset = part.vertices.len() as u32;
                part.vertices.extend(leaf.vertices.iter().map(|v| {
                    let mut v = *v;
                    v.pos[0] += 2.0;
                    v
                }));
                part.indices.extend(leaf.indices.iter().map(|i| i + offset));
                assert!(crate::vegetation_clod::build(&Part { vertices: &part.vertices,
                    indices: &part.indices, material: 0 }).is_none());
            }
            let source_only = CookedPlant::cook_source(&source).unwrap();
            let optical = CookedPlant::cook(&source, BakeConfig {
                projection_measure: ProjectionMeasure::ClosedSurfaceEntries,
                cell_size: 0.08, levels: 2, samples_per_cell: 2,
                max_samples: 1_000_000, max_voxels: 65_536, max_texture_texels: 0,
                ..Default::default()
            }).unwrap();
            assert_eq!(source_only.primitives.len(), 1);
            assert_eq!(optical.primitives.len(), 1);
            assert_eq!(source_only.primitives[0].key == optical.primitives[0].key, !disconnected,
                "recognized closed sweeps share across cook modes; distinct fallback policies do not");
            let expected_meshes = if disconnected { 2 } else { 1 };
            let expected_bytes = source_only.stats.primitive_bytes
                + if disconnected { optical.stats.primitive_bytes } else { 0 };
            for reverse in [false, true] {
                let order = if reverse { [&optical, &source_only] } else { [&source_only, &optical] };
                let mut scene = Scene::default();
                let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
                builder.append(order[0], instance(1, Mat4::IDENTITY)).unwrap();
                builder.append(order[1], instance(2, Mat4::from_translation(Vec3::X))).unwrap();
                let owners: Vec<_> = builder.scene.part_attachments.iter().map(|a| a.owner as usize).collect();
                assert_eq!(builder.scene.instances[owners[0]].first_cluster
                    == builder.scene.instances[owners[1]].first_cluster, !disconnected);
                builder.append(order[1], instance(3, Mat4::from_translation(Vec3::Z))).unwrap();
                let report = builder.finish(Policy::default()).unwrap();
                assert_eq!(report.primitive_meshes, expected_meshes);
                assert_eq!(report.shared_geometry_bytes, expected_bytes);
                assert_eq!(report.primitive_instances, 3);
                crate::foliage_representation::validate_bindings(&scene).unwrap();
            }
        }
    }

    #[test]
    fn source_only_capacity_and_failed_finish_preserve_existing_scene() {
        let source = foliage_assembly::generate(Parameters::grass(13), Limits::default()).unwrap();
        let cooked = CookedPlant::cook_source(&source).unwrap();
        let mut scene = Scene::default();
        scene.add_material(Material { tint: [0.1; 3], layer: -1, roughness: 1.0,
            flags: 0, scale: 1.0, metal: 0.0 });
        let mesh = scene.add_mesh(&cooked.primitives[0].mesh);
        scene.add_instance(mesh, Affine3A::IDENTITY);
        let before = (scene.vertices.len(), scene.cluster_verts.len(), scene.tris.len(),
            scene.clusters.len(), scene.meshes.len(), scene.materials.len(), scene.instances.len());
        {
            let limits = AttachLimits { max_primitive_instances: source.instances.len(), ..Default::default() };
            let mut builder = SceneVegetation::new(&mut scene, limits).unwrap();
            builder.append(&cooked, instance(1, Mat4::IDENTITY)).unwrap();
            let added = (builder.scene.instances.len(), builder.scene.vertices.len(),
                builder.scene.part_wind.len(), builder.scene.part_attachments.len(), builder.report.plants);
            assert!(builder.append(&cooked, instance(2, Mat4::IDENTITY)).is_err());
            assert_eq!((builder.scene.instances.len(), builder.scene.vertices.len(),
                builder.scene.part_wind.len(), builder.scene.part_attachments.len(), builder.report.plants), added);
            assert!(builder.append(&cooked, instance(1, Mat4::IDENTITY)).is_err());
            // Source-only assets cannot accidentally enter the volume finish path.
            assert!(builder.finish(Policy::default()).is_err());
        }
        assert_eq!((scene.vertices.len(), scene.cluster_verts.len(), scene.tris.len(),
            scene.clusters.len(), scene.meshes.len(), scene.materials.len(), scene.instances.len()), before);
        assert!(scene.part_wind.is_empty() && scene.part_attachments.is_empty());
        assert!(scene.voxel_forest.is_none() && scene.foliage_source_groups.is_empty());
        {
            let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
            for id in 0..MAX_PARTS / source.branches.len() {
                builder.append(&cooked, instance(id as u64, Mat4::IDENTITY)).unwrap();
            }
            let count = builder.scene.part_wind.len();
            assert!(builder.append(&cooked, instance(u64::MAX, Mat4::IDENTITY)).is_err());
            assert_eq!(builder.scene.part_wind.len(), count);
            // Dropping an uncommitted successful batch restores the checkpoint too.
        }
        assert_eq!((scene.vertices.len(), scene.cluster_verts.len(), scene.tris.len(),
            scene.clusters.len(), scene.meshes.len(), scene.materials.len(), scene.instances.len()), before);
        assert!(scene.part_wind.is_empty() && scene.part_attachments.is_empty());
    }

    #[test]
    fn source_finish_rejects_optical_assets_instead_of_discarding_them() {
        let (_, optical) = cook(Parameters::grass(19));
        let mut scene = Scene::default();
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        builder.append(&optical, instance(1, Mat4::IDENTITY)).unwrap();
        assert!(builder.finish_source().unwrap_err().contains("cook_source"));
        assert!(scene.instances.is_empty() && scene.vertices.is_empty() && scene.materials.is_empty());
        assert!(scene.part_wind.is_empty() && scene.part_attachments.is_empty());
        assert!(scene.voxel_forest.is_none() && scene.foliage_representations.is_empty());
    }

    #[test]
    fn source_finish_rejects_composed_member_overflow_and_restores_existing_scene() {
        let source = foliage_assembly::generate(
            Parameters { density: 1, ..Parameters::grass(19) }, Limits::default()).unwrap();
        let ordinary = CookedPlant::cook_source(&source).unwrap();
        let mut shifted = source;
        let shift = Mat4::from_translation(Vec3::X * 2e38);
        shifted.instances[0].rest = shift;
        let cooked = CookedPlant::cook_source(&shifted).unwrap();
        let mut scene = Scene::default();
        scene.add_material(ordinary.primitives[0].material);
        let mesh = scene.add_mesh(&ordinary.primitives[0].mesh);
        scene.add_instance(mesh, Affine3A::IDENTITY);
        let original_instance = bytemuck::bytes_of(&scene.instances[0]).to_vec();
        let before = (scene.vertices.len(), scene.cluster_verts.len(), scene.tris.len(),
            scene.clusters.len(), scene.meshes.len(), scene.materials.len(), scene.instances.len());
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        builder.append(&cooked, instance(1, shift)).unwrap();
        assert!(builder.scene.part_wind.iter().all(|part| part.rest.is_finite()));
        let owner = builder.scene.part_attachments[0].owner as usize;
        assert!(!matrix(&builder.scene.instances[owner]).is_finite());
        let error = builder.finish_source().unwrap_err();
        assert!(error.contains("world bind transform is singular/non-finite"), "{error}");
        assert_eq!((scene.vertices.len(), scene.cluster_verts.len(), scene.tris.len(),
            scene.clusters.len(), scene.meshes.len(), scene.materials.len(), scene.instances.len()), before);
        assert_eq!(bytemuck::bytes_of(&scene.instances[0]), original_instance.as_slice());
        assert!(scene.part_wind.is_empty() && scene.part_attachments.is_empty());
        assert!(scene.voxel_forest.is_none() && scene.foliage_representations.is_empty()
            && scene.foliage_source_groups.is_empty());
    }

    #[test]
    fn both_finish_routes_validate_member_bounds_and_scale_before_commit() {
        let (source, optical) = cook(Parameters { density: 1, ..Parameters::grass(23) });
        let source_only = CookedPlant::cook_source(&source).unwrap();
        for source_mode in [true, false] {
            for invalid_scale in [false, true] {
                let mut scene = Scene::default();
                let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
                builder.append(if source_mode { &source_only } else { &optical },
                    instance(1, Mat4::IDENTITY)).unwrap();
                let owner = builder.scene.part_attachments[0].owner as usize;
                // A finite rig/member matrix does not prove the derived sphere
                // or its scale is usable. Exercise those independent upload checks.
                assert!(matrix(&builder.scene.instances[owner]).is_finite());
                if invalid_scale { builder.scene.instances[owner].scale = 0.0; }
                else { builder.scene.instances[owner].sphere[3] = f32::INFINITY; }
                let result = if source_mode { builder.finish_source() }
                    else { builder.finish(Policy::default()) };
                assert!(result.is_err());
                assert!(scene.instances.is_empty() && scene.vertices.is_empty() && scene.materials.is_empty());
                assert!(scene.part_wind.is_empty() && scene.part_attachments.is_empty());
                assert!(scene.voxel_forest.is_none() && scene.foliage_representations.is_empty()
                    && scene.foliage_source_groups.is_empty());
            }
        }
    }

    #[test]
    fn invalid_or_overbudget_attachment_rolls_back_and_preserves_scene() {
        let (_, cooked) = cook(Parameters::grass(11));
        let mut scene = Scene::default();
        scene.add_material(Material {
            tint: [0.1; 3],
            layer: -1,
            roughness: 1.0,
            flags: 0,
            scale: 1.0,
            metal: 0.0,
        });
        let limits = AttachLimits {
            pack: PackLimits {
                max_dense_indices: 1,
                ..AttachLimits::default().pack
            },
            ..Default::default()
        };
        {
            let mut builder = SceneVegetation::new(&mut scene, limits).unwrap();
            builder
                .append(&cooked, instance(1, Mat4::IDENTITY))
                .unwrap();
            assert!(
                builder
                    .append(&cooked, instance(1, Mat4::IDENTITY))
                    .is_err()
            );
            assert!(
                builder
                    .append(
                        &cooked,
                        instance(2, Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)))
                    )
                    .is_err()
            );
            assert!(builder.finish(Policy::default()).is_err());
        }
        assert_eq!(scene.materials.len(), 1);
        assert!(scene.instances.is_empty() && scene.meshes.is_empty() && scene.vertices.is_empty());
        assert!(scene.part_wind.is_empty() && scene.part_attachments.is_empty());
        assert!(scene.voxel_forest.is_none() && scene.foliage_source_groups.is_empty());
    }

    #[test]
    fn hierarchical_wind_envelopes_contain_every_mip_support_corner() {
        let (mut source, _) = cook(Parameters::conifer_twig(29));
        // Exercise a deeper rig, not just the current root + child authoring.
        source.branches[2].parent = Some(1);
        let cooked = CookedPlant::cook(
            &source,
            BakeConfig {
                cell_size: 0.08,
                levels: 2,
                samples_per_cell: 2,
                max_texture_texels: 0,
                projection_measure: ProjectionMeasure::ClosedSurfaceEntries,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = Scene::default();
        let root = Mat4::from_scale_rotation_translation(
            Vec3::new(1.2, 0.8, 1.4),
            Quat::from_rotation_y(0.4),
            Vec3::new(3.0, 1.0, 2.0),
        );
        let mut builder = SceneVegetation::new(&mut scene, AttachLimits::default()).unwrap();
        builder.append(&cooked, instance(77, root)).unwrap();
        builder.finish(Policy::default()).unwrap();
        for time in [0.0, 0.7, 3.1, 19.0, 61.0, 119.9] {
            let wind = crate::wind::WindState {
                strength: 1.0,
                time_seconds: time,
                ..Default::default()
            };
            let poses = crate::part_wind::sample_poses(&scene.part_wind, &wind).unwrap();
            for binding in &scene.foliage_representations {
                let node = scene
                    .part_wind
                    .iter()
                    .position(|p| p.owner == Some(binding.owner))
                    .unwrap();
                let (lo, hi) = support(cooked.branches[node].volume.as_ref().unwrap());
                for corner in 0..8 {
                    let local = Vec3::new(
                        if corner & 1 == 0 { lo.x } else { hi.x },
                        if corner & 2 == 0 { lo.y } else { hi.y },
                        if corner & 4 == 0 { lo.z } else { hi.z },
                    );
                    assert!(
                        poses[node]
                            .transform_point3(local)
                            .distance(binding.group.center)
                            <= binding.group.radius + 1e-4
                    );
                }
            }
        }
    }

    // Tiny closed solids exercise the real optical cooker and scene adapter
    // without depending on game recipes or flattening a production tree.
    fn aggregate_fixture() -> Assembly {
        use crate::foliage_assembly::{BranchGroup,PartGeometry,PartInstance};
        let positions=[Vec3::ZERO,Vec3::X*0.24,Vec3::Y*0.38,Vec3::Z*0.20];
        let mut geometry=PartGeometry {id:1,vertices:vec![],indices:vec![]};
        for tri in [[0,2,1],[0,1,3],[0,3,2],[1,2,3]] {
            let [a,b,c]=tri.map(|i|positions[i]);let normal=(b-a).cross(c-a).normalize();
            for p in [a,b,c] {
                geometry.indices.push(geometry.vertices.len()as u32);
                geometry.vertices.push(crate::cluster::Vertex {pos:p.to_array(),normal:normal.to_array(),uv:[0.0;2]});
            }
        }
        let branches=vec![
            BranchGroup {id:10,parent:None,rest:Mat4::IDENTITY,stiffness:1.0},
            BranchGroup {id:11,parent:Some(0),rest:Mat4::from_translation(Vec3::new(0.35,0.2,0.0)),stiffness:0.7},
            BranchGroup {id:12,parent:Some(1),rest:Mat4::from_translation(Vec3::new(0.2,0.3,0.05)),stiffness:0.4},
        ];
        let instances=[0,1,1,2].into_iter().enumerate().map(|(i,branch)|PartInstance {
            id:100+i as u64,part:0,branch,material:0,
            rest:Mat4::from_translation(if i==2 {Vec3::Z*0.15}else{Vec3::ZERO})
        }).collect();
        Assembly::from_parts(Parameters {density:1,..Parameters::conifer_twig(17)},vec![geometry],branches,
            instances,vec![Material {tint:[0.17,0.32,0.09],layer:-1,roughness:0.8,
                flags:crate::scene::THIN_FOLIAGE|crate::scene::TWO_SIDED,scale:1.0,metal:0.0}],Limits::default()).unwrap()
    }

    fn aggregate_config() -> crate::vegetation_aggregate::Config {
        crate::vegetation_aggregate::Config {reference_cell_size:0.05,triangle_refinements:true,levels:2,diagnostic_resolution:4,
            max_placements:4,max_source_triangles:16,max_samples:100_000,max_reference_voxels:4096,
            max_diagnostic_steps:2_000_000,..Default::default()}
    }

    /// A full plant moves to another root at runtime (recipe_forest::Pool):
    /// cargo test -p render --lib vegetation_asset::tests::a_full_plant_moves_without_a_rebuild -- --ignored
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_full_plant_moves_without_a_rebuild() {
        use crate::{Camera, Lighting, Renderer, ViewRect};
        let source=aggregate_fixture();
        let mut cooked=CookedPlant::cook_aggregate(&source,aggregate_config()).unwrap();
        cooked.cook_proxy(&source,aggregate_config(),3).unwrap();
        let mut scene=Scene::default();
        let built=Mat4::from_scale_rotation_translation(Vec3::splat(2.0),Quat::IDENTITY,Vec3::ZERO);
        let mut builder=SceneVegetation::new(&mut scene,AttachLimits::default()).unwrap();
        let handle=builder.append(&cooked,instance(1,built)).unwrap();
        let target=Vec3::new(20.0,0.0,0.0);
        builder.append_proxy(&cooked,instance(2,Mat4::from_translation(target))).unwrap();
        let report=builder.finish_aggregate(crate::aggregate_lod::Policy {allow_sampled_optics:true,..Default::default()}).unwrap();
        assert_eq!(handle.movable.pose,0);
        assert_eq!(handle.movable.groups,0..3);
        let gpu=crate::Gpu::new().unwrap();
        let light=Lighting::default();
        let renderer=Renderer::new(&gpu,&scene,&light);
        let view=[ViewRect {cam:Camera {pos:target+Vec3::new(0.3,0.4,1.6),target:target+Vec3::new(0.3,0.4,0.0),fov_y:1.0,near:0.05},x:0,y:0,w:64,h:64}];
        let picks:Vec<_>=(0..8).flat_map(|y|(0..8).map(move|x|(4+x*8,4+y*8))).collect();
        let hits=|renderer:&Renderer|renderer.try_render_views(&gpu,&view,&light,64,64,&picks).unwrap().1.picks.iter()
            .filter(|p|p.is_some_and(|(i,_,_)|handle.instance_range.contains(&(i as usize)))).count();
        assert_eq!(hits(&renderer),0,"the full plant stands at its built root");
        let moved=Mat4::from_scale_rotation_translation(Vec3::splat(1.5),Quat::from_rotation_y(0.3),target);
        assert!(renderer.move_plant(&gpu,&handle.movable,Mat4::from_scale_rotation_translation(Vec3::splat(2.5),Quat::IDENTITY,target)).is_err(),"never grows");
        renderer.move_plant(&gpu,&handle.movable,moved).unwrap();
        renderer.set_plant_proxies(&gpu,&[(report.volume_instances as u32,ProxyShow::Hidden)]).unwrap();
        assert!(renderer.set_plant_proxies(&gpu,&[(0,ProxyShow::Hidden)]).is_err(),"only proxies hide");
        assert!(hits(&renderer)>0,"the moved plant draws at its new root");
    }

    #[test]
    fn aggregate_integration_preserves_memberships_hierarchy_bricks_and_scales_once() {
        let source=aggregate_fixture();let cooked=CookedPlant::cook_aggregate(&source,aggregate_config()).unwrap();
        assert_eq!(cooked.stats.volume_assets,3);
        assert_eq!(cooked.source_ids().collect::<Vec<_>>(),source.instances.iter()
            .map(|p|(p.id,source.branches[p.branch as usize].id)).collect::<Vec<_>>());
        let root_asset=cooked.branches[0].volume.as_ref().unwrap();
        let source_poses=source.branch_transforms().unwrap();
        for p in &source.instances {for v in &source.parts[p.part as usize].vertices {
            let point=(source_poses[p.branch as usize]*p.rest).transform_point3(Vec3::from(v.pos));
            assert!(point.cmpge(Vec3::from(root_asset.source_bounds[0])-Vec3::splat(1e-6)).all());
            assert!(point.cmple(Vec3::from(root_asset.source_bounds[1])+Vec3::splat(1e-6)).all());
        }}
        assert!(cooked.branches[0].aggregate_motion_error>0.0);
        assert!(cooked.branches[0].aggregate.iter().all(|c|!c.source_only));
        for branch in &cooked.branches[1..] {
            assert_eq!(branch.aggregate.iter().filter(|c|c.source_only).count(),3);
            assert!(branch.aggregate[..3].iter().all(|c|c.source_only));
        }
        let mut scene=Scene::default();let mut builder=SceneVegetation::new(&mut scene,AttachLimits::default()).unwrap();
        let small=builder.append(&cooked,instance(31,Mat4::from_scale(Vec3::splat(0.5)))).unwrap();
        let large=builder.append(&cooked,instance(32,Mat4::from_scale_rotation_translation(Vec3::splat(2.0),
            Quat::IDENTITY,Vec3::X*5.0))).unwrap();
        let report=builder.finish_aggregate(crate::aggregate_lod::Policy {allow_sampled_optics:true,..Default::default()}).unwrap();
        assert_eq!(report.plants,2);assert_eq!(report.primitive_instances,8);
        assert_eq!(report.volume_instances,6);assert_eq!(report.volume_assets,3);
        assert_eq!(report.primitive_meshes,1);assert_eq!(report.pose_nodes,6);
        let selection=scene.aggregate_lod.as_ref().unwrap();assert_eq!(selection.groups.len(),6);
        let mut seen=BTreeSet::new();
        for (plant_index,handle) in [&small,&large].into_iter().enumerate() {
            for branch in 0..3 {
                let g=&selection.groups[plant_index*3+branch];
                assert_eq!(g.volume,(handle.volume_range.start+branch)as u32);
                assert_eq!(g.parent,if branch==0 {None}else{Some((plant_index*3)as u32)});
                assert_eq!(g.scale,if plant_index==0 {0.5}else{2.0});
                let owner=handle.branch_owners[branch].unwrap();
                let members=&scene.foliage_source_groups.iter().find(|g|g.owner==owner).unwrap().members;
                let expected=scene.part_attachments.iter().filter(|p|p.pose==(handle.pose_range.start+branch)as u32)
                    .map(|p|p.owner).collect::<BTreeSet<_>>();
                assert_eq!(members.iter().copied().collect::<BTreeSet<_>>(),expected);
                assert_eq!(members.len(),[1,2,1][branch]);
                for &member in members {assert!(seen.insert(member));}
                assert_eq!(scene.part_wind[handle.pose_range.start+branch].parent,
                    source.branches[branch].parent.map(|p|handle.pose_range.start as u32+p));
                for (actual,local) in g.candidates.iter().zip(&cooked.branches[branch].aggregate) {
                    // CPU metadata stays local; the selector applies g.scale
                    // exactly once. Multiplying here too caused scale squared.
                    assert_eq!(actual.spatial_error.to_bits(),local.spatial_error.to_bits());
                    assert_eq!(actual.triangle_error.to_bits(),local.triangle_error.to_bits());
                }
            }
        }
        assert_eq!(seen.len(),scene.part_attachments.len());
        for branch in 0..3 {
            let a=&selection.groups[branch];let b=&selection.groups[3+branch];
            assert!((b.radius/a.radius-4.0).abs()<1e-5);
            for (ac,bc) in a.candidates.iter().zip(&b.candidates) {
                assert!((bc.spatial_error*b.scale/(ac.spatial_error*a.scale)-4.0).abs()<1e-5);
            }
        }
        let forest=scene.voxel_forest.as_ref().unwrap();
        assert!(forest.levels.iter().all(|l|l.dims_offset[3]&foliage_volume_data::BRICK_INDIRECTION!=0));
        assert_eq!(forest.instances.len(),6);assert_eq!(report.packed_volume_bytes,forest.byte_len());
        crate::foliage_representation::validate_bindings(&scene).unwrap();
        crate::part_wind::validate_bindings(&scene.part_wind,&scene.part_attachments,&scene.instances,Some(forest)).unwrap();
    }

    #[test]
    fn cooked_plants_round_trip_through_the_cache() {
        let source=aggregate_fixture();
        let mut cooked=CookedPlant::cook_aggregate(&source,aggregate_config()).unwrap();
        cooked.cook_proxy(&source,aggregate_config(),3).unwrap();
        let dir=std::env::temp_dir().join(format!("warp-cook-test-{}",std::process::id()));
        let (key,other)=(cook_key("fixture"),cook_key("fixture 2"));
        assert_ne!(key,other);
        let path=dir.join("plant.cook");
        cooked.save(&path,key).unwrap();
        let loaded=CookedPlant::load(&path,key).unwrap();
        // Byte-identical, and a fresh proxy identity (its own scene asset).
        assert_eq!(postcard::to_allocvec(&loaded).unwrap(),postcard::to_allocvec(&cooked).unwrap());
        assert_eq!(loaded.stats(),cooked.stats());
        assert_eq!(loaded.proxy(),cooked.proxy());
        assert_ne!(loaded.proxy_id,cooked.proxy_id);
        assert!(CookedPlant::load(&path,other).is_err(),"another key");
        let mut bytes=std::fs::read(&path).unwrap();
        let last=bytes.len()-1;bytes[last]^=1;std::fs::write(&path,&bytes).unwrap();
        assert!(CookedPlant::load(&path,key).is_err(),"damaged");
        bytes.truncate(100);std::fs::write(&path,&bytes).unwrap();
        assert!(CookedPlant::load(&path,key).is_err(),"truncated");
        // cached: a miss cooks and saves, the next call loads.
        let mut cooks=0;
        let (_,hit)=CookedPlant::cached(&dir,"fixture",||{cooks+=1;CookedPlant::cook_aggregate(&source,aggregate_config())}).unwrap();
        assert!(!hit);
        let (again,hit)=CookedPlant::cached(&dir,"fixture",||{cooks+=1;CookedPlant::cook_aggregate(&source,aggregate_config())}).unwrap();
        assert!(hit&&cooks==1);
        assert_eq!(again.stats(),cooked.stats());
        let _=std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whole_plant_proxies_trail_the_aggregate_forest_with_one_instance_each() {
        let source=aggregate_fixture();
        let mut cooked=CookedPlant::cook_aggregate(&source,aggregate_config()).unwrap();
        let mut scene=Scene::default();
        assert!(SceneVegetation::new(&mut scene,AttachLimits::default()).unwrap()
            .append_proxy(&cooked,instance(1,Mat4::IDENTITY)).is_err(),"no proxy cooked yet");
        cooked.cook_proxy(&source,aggregate_config(),3).unwrap();
        let proxy=cooked.proxy().unwrap().clone();
        // Level zero is the reference grid; every level keeps all optical mass.
        assert_eq!(proxy.levels[0].cell_size,aggregate_config().reference_cell_size);
        let mass=|l:&crate::foliage_voxel::VoxelLevel|l.voxels.iter().map(|v|f64::from(v.surface_area)).sum::<f64>();
        for level in &proxy.levels[1..] {assert!((mass(level)/mass(&proxy.levels[0])-1.0).abs()<1e-4);}
        let mut builder=SceneVegetation::new(&mut scene,AttachLimits::default()).unwrap();
        builder.append(&cooked,instance(1,Mat4::IDENTITY)).unwrap();
        let instances=builder.scene.instances.len();
        let far=Mat4::from_scale_rotation_translation(Vec3::splat(2.0),Quat::from_rotation_y(0.4),Vec3::new(40.0,0.0,3.0));
        let owner=builder.append_proxy(&cooked,instance(2,far)).unwrap();
        builder.append_proxy(&cooked,instance(3,Mat4::from_translation(Vec3::Z*60.0))).unwrap();
        assert!(builder.append_proxy(&cooked,instance(3,Mat4::IDENTITY)).is_err(),"duplicate id");
        assert_eq!((owner,builder.scene.instances.len()),(instances,instances+2));
        let report=builder.finish_aggregate(crate::aggregate_lod::Policy {allow_sampled_optics:true,..Default::default()}).unwrap();
        assert_eq!((report.plants,report.plant_proxies),(1,2));
        let forest=scene.voxel_forest.as_ref().unwrap();
        assert_eq!(scene.voxel_proxies,2);assert_eq!(scene.voxel_proxy_pixels,4.0);assert_eq!(scene.voxel_proxy_pixels_from,50.0);
        assert_eq!(scene.aggregate_lod.as_ref().unwrap().groups.len()+2,forest.instances.len());
        let tree=forest.instances[forest.instances.len()-2];
        let source_instance=scene.instances[owner];
        assert_eq!((tree.meta[0],tree.meta[2]),(owner as u32,proxy.levels.len() as u32));
        assert_eq!((source_instance.cluster_count,source_instance.flags&(FOLIAGE|PART_WIND)),(0,FOLIAGE));
        // World bounds of the proxy contain its rest-pose source.
        let poses=source.branch_transforms().unwrap();
        for p in &source.instances {for v in &source.parts[p.part as usize].vertices {
            let w=far.transform_point3((poses[p.branch as usize]*p.rest).transform_point3(Vec3::from(v.pos)));
            assert!(w.cmpge(Vec3::from_slice(&tree.bounds_min)-1e-4).all() && w.cmple(Vec3::from_slice(&tree.bounds_max)+1e-4).all());
        }}
        // Both proxies share one dense asset after the bricked branch groups.
        let [a,b]=[forest.instances.len()-2,forest.instances.len()-1].map(|i|forest.instances[i].meta[1]);
        assert_eq!(a,b);
        assert_eq!(forest.levels[a as usize].dims_offset[3]&foliage_volume_data::BRICK_INDIRECTION,0);
        crate::aggregate_lod::validate_scene(&scene).unwrap();
        crate::foliage_representation::validate_bindings(&scene).unwrap();
    }

    #[test]
    fn aggregate_integration_invalid_policy_and_late_validation_restore_existing_scene() {
        let cooked=CookedPlant::cook_aggregate(&aggregate_fixture(),aggregate_config()).unwrap();
        for invalid_policy in [true,false] {
            let mut scene=Scene::default();scene.add_material(cooked.primitives[0].material);
            let mesh=scene.add_mesh(&cooked.primitives[0].mesh);scene.add_instance(mesh,Affine3A::IDENTITY);
            scene.voxel_lod_pixels=2.75;scene.voxel_active_tiles=false;scene.part_wind_lod=false;
            scene.foliage_representation_policy.max_error_pixels=3.25;
            let counts=(scene.vertices.len(),scene.cluster_verts.len(),scene.tris.len(),scene.clusters.len(),
                scene.meshes.len(),scene.instances.len(),scene.materials.len());
            let prior=bytemuck::bytes_of(&scene.instances[0]).to_vec();
            let vertices=bytemuck::cast_slice::<_,u8>(&scene.vertices).to_vec();
            let mut builder=SceneVegetation::new(&mut scene,AttachLimits::default()).unwrap();
            builder.append(&cooked,instance(50,Mat4::IDENTITY)).unwrap();
            let mut policy=crate::aggregate_lod::Policy::default();
            if invalid_policy {policy.max_error_pixels=f32::NAN;} else {
                let owner=builder.scene.part_attachments[0].owner as usize;
                builder.scene.instances[owner].scale=0.0;
            }
            assert!(builder.finish_aggregate(policy).is_err());
            assert_eq!((scene.vertices.len(),scene.cluster_verts.len(),scene.tris.len(),scene.clusters.len(),
                scene.meshes.len(),scene.instances.len(),scene.materials.len()),counts);
            assert_eq!(bytemuck::bytes_of(&scene.instances[0]),prior.as_slice());
            assert_eq!(bytemuck::cast_slice::<_,u8>(&scene.vertices),vertices.as_slice());
            assert!(scene.aggregate_lod.is_none()&&scene.voxel_forest.is_none());
            assert!(scene.part_wind.is_empty()&&scene.part_attachments.is_empty());
            assert!(scene.foliage_representations.is_empty()&&scene.foliage_source_groups.is_empty());
            assert_eq!(scene.voxel_lod_pixels,2.75);assert!(!scene.voxel_active_tiles&&!scene.part_wind_lod);
            assert_eq!(scene.foliage_representation_policy.max_error_pixels,3.25);
        }
    }

    #[test]
    fn aggregate_integration_root_cook_honors_explicit_work_limits_and_root_membership() {
        let source=aggregate_fixture();
        // Each individual branch fits both limits; the entire subtree does
        // not. These failures prove root cooking does not silently enlarge caps.
        for config in [crate::vegetation_aggregate::Config {max_source_triangles:12,..aggregate_config()},
            crate::vegetation_aggregate::Config {max_placements:3,..aggregate_config()}] {
            let error=CookedPlant::cook_aggregate(&source,config).err().expect("root budget must fail");
            assert!(error.contains("aggregate subtree root"),"{error}");
        }
        let mut empty_root=source;empty_root.instances[0].branch=1;
        let error=CookedPlant::cook_aggregate(&empty_root,aggregate_config()).err().expect("empty root must fail at cook");
        assert!(error.contains("source members on its root"),"{error}");
    }
}
