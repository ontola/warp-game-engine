//! Cooked shared vegetation primitives and bounded scene attachment.
//!
//! Cooking may expand the source for the voxel baker. Attachment never does:
//! every blade/needle/wood member references an immutable primitive mesh and
//! a branch-local transform. Equal geometry AND material bytes share a mesh,
//! even across different cooked variants. This is not yet hierarchical coarse
//! geometry aggregation or a whole-forest residency/streaming system.

use crate::{
    cluster::{ClusterMesh, Part, SimplificationPolicy},
    foliage_assembly::Assembly,
    foliage_representation::{Binding, Group, GroupKey, OpticalEvidence, Policy, SourceGroup},
    foliage_volume_data::{self, ALL_MATERIALS, PackLimits, VolumeBinding},
    foliage_voxel::{BakeConfig, VoxelAsset},
    part_wind::{MAX_ASSEMBLY_PARTS, MAX_PARTS, PartAttachment, PartWind},
    scene::{GeometryCategory, GpuInstance, Material, PART_WIND, Scene},
};
use glam::{Affine3A, Mat4, Vec3, Vec4};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

struct Primitive {
    mesh: ClusterMesh,
    material: Material,
    // Exact comparison, not a lossy geometry ID or unchecked hash collision.
    key: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Placement {
    id: u64,
    primitive: usize,
    branch: usize,
    rest: Mat4,
}

#[derive(Clone, Debug)]
struct Branch {
    id: u64,
    parent: Option<u32>,
    rest: Mat4,
    assembly_rest: Mat4,
    max_angle: f32,
    volume: Option<VoxelAsset>,
}

/// Immutable CPU asset. Its source meshes contain only unique primitives,
/// never transformed copies of every leaf in a branch.
pub struct CookedPlant {
    primitives: Vec<Primitive>,
    placements: Vec<Placement>,
    branches: Vec<Branch>,
    stats: CookStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    pub fn cook(source: &Assembly, config: BakeConfig) -> Result<Self, String> {
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
        let baked = source.bake_branches(config)?;
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
                // Derive closed reductions from the same source; textures or
                // unrecognized topology keep the ordinary protected cooker.
                let mesh = if material.layer < 0 && material.flags & crate::scene::THIN_FOLIAGE != 0 {
                    crate::vegetation_clod::build(&part)
                } else { None }.unwrap_or_else(|| ClusterMesh::build_with_policy(&[part], |_| {
                    SimplificationPolicy::TexturedGeometry
                }));
                stats.primitive_vertices += mesh.vertices.len();
                stats.primitive_triangles += mesh.leaf_triangles();
                stats.primitive_bytes += mesh_bytes(&mesh);
                let index = primitives.len();
                keys.insert(key.clone(), index);
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
        })
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
    /// Original assembly branch order; empty rig branches have no proxy.
    pub branch_owners: Vec<Option<u32>>,
    /// Original assembly branch order; empty branches have no representation.
    pub group_keys: Vec<Option<GroupKey>>,
}

#[derive(Clone, Copy, Debug)]
pub struct AttachLimits {
    pub max_primitive_instances: usize,
    pub max_primitive_meshes: usize,
    pub pack: PackLimits,
}

impl Default for AttachLimits {
    fn default() -> Self {
        Self {
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
        {
            return Err("shared vegetation requires an unused bounded foliage/part pool".into());
        }
        if limits.max_primitive_instances == 0
            || limits.max_primitive_instances > crate::part_wind::MAX_ATTACHMENTS
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
        if self.scene.part_wind.len() + plant.branches.len() > MAX_PARTS
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
                    wind_margin(plant, i, asset, scale),
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
                group_keys[i] = Some(group.key);
                self.scene.foliage_representations.push(Binding {
                    owner,
                    volume,
                    group,
                });
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
                self.scene
                    .foliage_source_groups
                    .push(SourceGroup { owner, members });
                branch_owners[i] = Some(owner);
                (Some(owner), Some(volume))
            } else {
                (None, None)
            };
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
        })
    }

    pub fn finish(mut self, policy: Policy) -> Result<AttachmentReport, String> {
        if self.report.plants == 0 {
            return Err("cannot finish an empty vegetation attachment".into());
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
        // Also checks hierarchy, root scaling and every animation descriptor
        // without constructing a GPU resource or sampling a moving frame.
        crate::part_wind::sample_poses(&self.scene.part_wind, &crate::wind::WindState::default())?;
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
        self.scene.part_wind.clear();
        self.scene.part_attachments.clear();
        self.scene.foliage_representations.clear();
        self.scene.foliage_source_groups.clear();
        self.scene.foliage_representation_policy = c.policy;
        self.scene.voxel_lod_pixels = c.lod_pixels;
        self.scene.voxel_active_tiles = c.active_tiles;
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
        assert_eq!(report.shared_geometry_bytes, cooked.primitives.iter().map(|p| mesh_bytes(&p.mesh)).sum());
    }

    #[test]
    fn member_vertices_normals_and_rest_poses_equal_expanded_oracle() {
        for parameters in [Parameters::grass(5), Parameters::conifer_twig(7)] {
            let (source, cooked) = cook(parameters);
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
            builder.finish(Policy::default()).unwrap();
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
}
