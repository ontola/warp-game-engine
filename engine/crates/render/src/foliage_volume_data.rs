//! Shared GPU packing for the bounded, source-derived foliage volume renderer.
//!
//! Assets are uploaded once, independently of their tree instances. Zero in a
//! level's dense indirection grid means empty; other entries address sparse
//! cell records plus one. No occupancy threshold turns density into a solid.
//! This module does not allocate GPU resources or change renderer defaults.

use crate::foliage_voxel::{Voxel, VoxelAsset};
use crate::scene::GpuInstance;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashSet;

/// Must match the runtime prototype's bounded candidate-list capacity.
pub const MAX_INSTANCES: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct PackLimits {
    pub max_instances: usize,
    pub max_assets: usize,
    /// Pack this many finest levels, or fewer if the source has fewer levels.
    /// This is an explicit representation choice, not a silent budget fallback.
    pub levels_per_asset: usize,
    /// Extra packed levels are occupancy acceleration only. Keep the ray
    /// bounds at the shaded first level, including its trilinear support.
    /// Leave false if a consumer may shade any of the packed levels.
    pub first_level_bounds: bool,
    pub max_dense_indices: usize,
    pub max_sparse_cells: usize,
}

impl Default for PackLimits {
    fn default() -> Self {
        Self {
            max_instances: MAX_INSTANCES,
            max_assets: 3,
            levels_per_asset: 3,
            first_level_bounds: false,
            max_dense_indices: 4_194_304,
            max_sparse_cells: 262_144,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VolumeBinding {
    /// Existing Scene.instances index; geometry, tint and shadows share it.
    pub owner: u32,
    /// Index in the source VoxelAsset slice, not a per-instance asset copy.
    pub asset: u32,
    /// Only this material's clusters are replaced; trunk/bark remain meshes.
    pub crown_material: u32,
}

/// WGSL: two vec4 records, 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuLevel {
    /// Common local origin xyz, cell size w.
    pub origin_cell: [f32; 4],
    /// Dimensions xyz, dense indirection-array offset w.
    pub dims_offset: [u32; 4],
}

/// WGSL: seven vec4f records, 112 bytes. Values remain linear and unquantized.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuCell {
    pub albedo_rough: [f32; 4],
    /// Thirteen directional extinction coefficients per local metre, padded
    /// with three zeroes. Same direction order as PROJECTION_DIRECTIONS.
    pub extinction: [[f32; 4]; 4],
    /// Six normalized shading lobes (+X,-X,+Y,-Y,+Z,-Z), then two zeroes.
    pub normals: [[f32; 4]; 2],
}

/// WGSL: six vec4 records, 96 bytes. Trees are static in this first path.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTree {
    /// Rows of world-to-local affine transform, including translation.
    pub inverse: [[f32; 4]; 3],
    pub bounds_min: [f32; 4],
    pub bounds_max: [f32; 4],
    /// Source owner, first packed level, level count, crown material.
    pub meta: [u32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct GpuForest {
    pub levels: Vec<GpuLevel>,
    pub indices: Vec<u32>,
    pub cells: Vec<GpuCell>,
    pub instances: Vec<GpuTree>,
}

impl GpuForest {
    pub fn byte_len(&self) -> usize {
        self.levels.len() * std::mem::size_of::<GpuLevel>()
            + self.indices.len() * std::mem::size_of::<u32>()
            + self.cells.len() * std::mem::size_of::<GpuCell>()
            + self.instances.len() * std::mem::size_of::<GpuTree>()
    }
}

fn valid_nonnegative(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite() && *value >= 0.0)
}

fn pack_cell(voxel: &Voxel, cell_size: f32) -> Result<GpuCell, String> {
    if !valid_nonnegative(&voxel.directional_area)
        || !valid_nonnegative(&voxel.albedo)
        || !valid_nonnegative(&voxel.normal_weights)
        || !voxel.roughness.is_finite()
        || !(0.0..=1.0).contains(&voxel.roughness)
    {
        return Err("volume cell has invalid optical/material data".into());
    }
    let normal_sum: f32 = voxel.normal_weights.iter().sum();
    if (normal_sum - 1.0).abs() > 0.001 {
        return Err("volume cell shading-normal weights must sum to one".into());
    }
    let volume = cell_size.powi(3);
    if !volume.is_finite() || volume <= 0.0 {
        return Err("volume cell size cannot represent a finite positive volume".into());
    }
    let mut cell = GpuCell::zeroed();
    cell.albedo_rough = [voxel.albedo[0], voxel.albedo[1], voxel.albedo[2], voxel.roughness];
    for (index, area) in voxel.directional_area.iter().enumerate() {
        let density = *area / volume;
        if !density.is_finite() {
            return Err("volume cell extinction overflows f32".into());
        }
        cell.extinction[index / 4][index % 4] = density;
    }
    for (index, weight) in voxel.normal_weights.iter().enumerate() {
        cell.normals[index / 4][index % 4] = *weight;
    }
    Ok(cell)
}

fn matrix(rows: [[f32; 4]; 3]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(rows[0][0], rows[1][0], rows[2][0], 0.0),
        Vec4::new(rows[0][1], rows[1][1], rows[2][1], 0.0),
        Vec4::new(rows[0][2], rows[1][2], rows[2][2], 0.0),
        Vec4::new(rows[0][3], rows[1][3], rows[2][3], 1.0),
    )
}

fn pack_tree(binding: VolumeBinding, instance: &GpuInstance, levels: [u32; 2],
    local_min: Vec3, local_max: Vec3) -> Result<GpuTree, String> {
    if !instance.rows.iter().flatten().all(|value| value.is_finite()) {
        return Err(format!("volume owner {} has non-finite transform", binding.owner));
    }
    let transform = matrix(instance.rows);
    let axes = [transform.x_axis.truncate(), transform.y_axis.truncate(), transform.z_axis.truncate()];
    let lengths = axes.map(Vec3::length);
    if lengths.iter().any(|length| !length.is_finite() || *length < 1e-6) {
        return Err(format!("volume owner {} has degenerate scale", binding.owner));
    }
    let relative_det = (axes[0] / lengths[0]).dot((axes[1] / lengths[1]).cross(axes[2] / lengths[2]));
    if !relative_det.is_finite() || relative_det.abs() < 1e-6 {
        return Err(format!("volume owner {} has singular transform", binding.owner));
    }
    let inverse = transform.inverse();
    if !inverse.to_cols_array().iter().all(|value| value.is_finite()) {
        return Err(format!("volume owner {} inverse is non-finite", binding.owner));
    }
    let rows = inverse.transpose();
    let mut world_min = Vec3::splat(f32::INFINITY);
    let mut world_max = Vec3::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let point = Vec3::new(
            if corner & 1 == 0 { local_min.x } else { local_max.x },
            if corner & 2 == 0 { local_min.y } else { local_max.y },
            if corner & 4 == 0 { local_min.z } else { local_max.z },
        );
        let point = transform.transform_point3(point);
        world_min = world_min.min(point);
        world_max = world_max.max(point);
    }
    if !world_min.is_finite() || !world_max.is_finite() {
        return Err(format!("volume owner {} bounds overflow", binding.owner));
    }
    Ok(GpuTree {
        inverse: [rows.x_axis.to_array(), rows.y_axis.to_array(), rows.z_axis.to_array()],
        bounds_min: world_min.extend(0.0).to_array(),
        bounds_max: world_max.extend(0.0).to_array(),
        meta: [binding.owner, levels[0], levels[1], binding.crown_material],
    })
}

/// Pack shared source crowns and validate the complete bounded instance patch.
///
/// In the shader, normalize inverse_linear * world_ray only for the angular
/// density lookup. Multiply that local density by the transformed direction's
/// length to obtain extinction per world metre; nonuniform tree scale matters.
/// Crown material indices are supplied by the scene builder, which owns the
/// material array. All source owner/asset/voxel indices are validated here.
pub fn pack(assets: &[VoxelAsset], bindings: &[VolumeBinding], scene_instances: &[GpuInstance],
    limits: PackLimits) -> Result<GpuForest, String> {
    if limits.max_instances > MAX_INSTANCES || bindings.len() > limits.max_instances {
        return Err(format!("volume instance budget exceeded: {} requested, {} allowed (hard maximum {MAX_INSTANCES})",
            bindings.len(), limits.max_instances));
    }
    if assets.len() > limits.max_assets || limits.levels_per_asset == 0 || limits.levels_per_asset > 16 {
        return Err("invalid volume asset/level budget".into());
    }
    let mut forest = GpuForest::default();
    let mut asset_spans = Vec::with_capacity(assets.len());
    for (asset_index, asset) in assets.iter().enumerate() {
        let origin = Vec3::from(asset.origin);
        if !origin.is_finite() || asset.levels.is_empty() {
            return Err(format!("volume asset {asset_index} has no valid level/origin"));
        }
        let first = u32::try_from(forest.levels.len()).map_err(|_| "volume level index overflow")?;
        let mut local_min = Vec3::splat(f32::INFINITY);
        let mut local_max = Vec3::splat(f32::NEG_INFINITY);
        let mut previous_cell = 0.0;
        for (level_index, level) in asset.levels.iter().take(limits.levels_per_asset).enumerate() {
            let dimensions = level.dimensions;
            if dimensions.contains(&0) || !level.cell_size.is_finite() || level.cell_size <= 0.0
                || (level_index > 0 && level.cell_size <= previous_cell)
            {
                return Err(format!("volume asset {asset_index} level {level_index} has invalid dimensions/cell size"));
            }
            previous_cell = level.cell_size;
            let count = dimensions.iter().try_fold(1usize, |count, &dimension|
                count.checked_mul(dimension as usize)).ok_or("volume dimensions overflow")?;
            let index_end = forest.indices.len().checked_add(count).ok_or("volume index count overflow")?;
            if index_end > limits.max_dense_indices || index_end > u32::MAX as usize {
                return Err("volume dense-indirection budget exceeded".into());
            }
            let sparse_end = forest.cells.len().checked_add(level.voxels.len()).ok_or("volume sparse count overflow")?;
            if sparse_end > limits.max_sparse_cells || sparse_end >= u32::MAX as usize {
                return Err("volume sparse-cell budget exceeded".into());
            }
            let base = forest.indices.len();
            forest.indices.resize(index_end, 0);
            for voxel in &level.voxels {
                if (0..3).any(|axis| voxel.coord[axis] >= dimensions[axis]) {
                    return Err(format!("volume asset {asset_index} level {level_index} has an out-of-bounds sparse coordinate"));
                }
                let [x, y, z] = voxel.coord.map(|value| value as usize);
                let index = base + x + dimensions[0] as usize * (y + dimensions[1] as usize * z);
                if forest.indices[index] != 0 {
                    return Err("volume level contains duplicate sparse coordinates".into());
                }
                let cell = pack_cell(voxel, level.cell_size)?;
                forest.indices[index] = forest.cells.len() as u32 + 1;
                forest.cells.push(cell);
            }
            forest.levels.push(GpuLevel {
                origin_cell: origin.extend(level.cell_size).to_array(),
                dims_offset: [dimensions[0], dimensions[1], dimensions[2], base as u32],
            });
            let bounds = level.local_bounds(asset.origin);
            // Cell-centred trilinear reconstruction has a half-cell tent
            // outside the grid's faces. Ray bounds must retain that support;
            // clamping samples to the edge would instead duplicate density.
            let filter_support = Vec3::splat(level.cell_size * 0.5);
            if level_index == 0 || !limits.first_level_bounds {
                local_min = local_min.min(Vec3::from(bounds[0]) - filter_support);
                local_max = local_max.max(Vec3::from(bounds[1]) + filter_support);
            }
        }
        if !local_min.is_finite() || !local_max.is_finite() {
            return Err(format!("volume asset {asset_index} cell bounds overflow"));
        }
        asset_spans.push(([first, forest.levels.len() as u32 - first], local_min, local_max));
    }
    let mut owners = HashSet::with_capacity(bindings.len());
    for &binding in bindings {
        if !owners.insert(binding.owner) {
            return Err(format!("duplicate volume source owner {} would double the density", binding.owner));
        }
        let instance = scene_instances.get(binding.owner as usize)
            .ok_or_else(|| format!("volume source owner {} is out of bounds", binding.owner))?;
        let &(levels, local_min, local_max) = asset_spans.get(binding.asset as usize)
            .ok_or_else(|| format!("volume source asset {} is out of bounds", binding.asset))?;
        forest.instances.push(pack_tree(binding, instance, levels, local_min, local_max)?);
    }
    Ok(forest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_voxel::{BakeStats, PROJECTION_DIRECTIONS, VoxelLevel};
    use glam::{Affine3A, Quat};

    fn voxel(coord: [u32; 3], area: f32) -> Voxel {
        Voxel { coord, surface_area: area, projected_area: [area, 0.0, 0.0],
            directional_area: PROJECTION_DIRECTIONS.map(|direction| direction[0].abs() * area),
            albedo: [0.2, 0.4, 0.1], roughness: 0.8,
            normal_weights: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0] }
    }
    fn asset() -> VoxelAsset {
        VoxelAsset { origin: [-0.5, 0.0, -0.5], source_bounds: [[-0.5, 0.0, -0.5], [0.0, 0.5, 0.0]],
            levels: vec![
                VoxelLevel { cell_size: 0.25, dimensions: [3, 3, 3], voxels: vec![voxel([0, 0, 0], 0.02), voxel([1, 1, 1], 0.04)] },
                VoxelLevel { cell_size: 0.5, dimensions: [2, 2, 2], voxels: vec![voxel([0, 0, 0], 0.06)] },
            ], stats: BakeStats::default() }
    }
    fn instance(transform: Affine3A) -> GpuInstance {
        let matrix = Mat4::from(transform).transpose();
        GpuInstance { rows: [matrix.x_axis.to_array(), matrix.y_axis.to_array(), matrix.z_axis.to_array()],
            ..GpuInstance::zeroed() }
    }
    fn binding(owner: u32, asset: u32) -> VolumeBinding { VolumeBinding { owner, asset, crown_material: 12 } }
    fn local_direction(tree: &GpuTree, direction: Vec3) -> Vec3 {
        Vec3::from_array(tree.inverse.map(|row| Vec4::from(row).dot(direction.extend(0.0))))
    }

    #[test]
    fn shared_assets_have_exact_gpu_layout_and_valid_zero_empty_indices() {
        assert_eq!(std::mem::size_of::<GpuLevel>(), 32);
        assert_eq!(std::mem::size_of::<GpuCell>(), 112);
        assert_eq!(std::mem::size_of::<GpuTree>(), 96);
        let instances = [instance(Affine3A::IDENTITY), instance(Affine3A::from_translation(Vec3::X * 8.0))];
        let forest = pack(&[asset()], &[binding(0, 0), binding(1, 0)], &instances, PackLimits::default()).unwrap();
        assert_eq!(forest.levels.len(), 2); assert_eq!(forest.cells.len(), 3);
        assert_eq!(forest.indices.len(), 27 + 8); assert_eq!(forest.indices[0], 1);
        assert_eq!(forest.indices[13], 2); assert_eq!(forest.indices[27], 3);
        assert_eq!(forest.indices.iter().filter(|&&index| index != 0).count(), 3);
        assert!(forest.indices.iter().all(|&index| index as usize <= forest.cells.len()));
        assert_eq!(forest.instances[0].meta, [0, 0, 2, 12]);
        assert_eq!(forest.instances[1].meta, [1, 0, 2, 12]);
        assert_eq!(forest.byte_len(), 2 * 32 + 35 * 4 + 3 * 112 + 2 * 96);
    }

    #[test]
    fn thirteen_directional_densities_keep_integrated_mass_across_levels() {
        let source = asset();
        let forest = pack(std::slice::from_ref(&source), &[], &[], PackLimits::default()).unwrap();
        for (level, source_level) in forest.levels.iter().zip(&source.levels) {
            let [nx, ny, nz, base] = level.dims_offset;
            for direction in 0..13 {
                let integral: f32 = forest.indices[base as usize..(base + nx * ny * nz) as usize].iter()
                    .filter(|&&index| index != 0).map(|&index|
                        forest.cells[index as usize - 1].extinction[direction / 4][direction % 4]
                            * level.origin_cell[3].powi(3)).sum();
                let expected: f32 = source_level.voxels.iter().map(|cell| cell.directional_area[direction]).sum();
                assert!((integral - expected).abs() < 1e-6);
            }
        }
        for cell in &forest.cells {
            assert_eq!(cell.extinction[3][1..], [0.0; 3]);
            assert_eq!(cell.normals[1][2..], [0.0; 2]);
        }
    }

    #[test]
    fn nonuniform_scale_rotates_local_directions_and_scales_world_sigma() {
        let rotation = Quat::from_rotation_y(0.7);
        let scale = Vec3::new(2.0, 3.0, 4.0);
        let transform = Affine3A::from_scale_rotation_translation(scale, rotation, Vec3::new(40.0, -2.0, 16.0));
        let forest = pack(&[asset()], &[binding(0, 0)], &[instance(transform)], PackLimits::default()).unwrap();
        let tree = &forest.instances[0];
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let world_direction = rotation * axis;
            let local = local_direction(tree, world_direction);
            let scale_along = scale.dot(axis);
            assert!((local.length() - 1.0 / scale_along).abs() < 1e-6);
            assert!((local.normalize() - axis).length() < 1e-6);
            // Equal local segment means equal optical depth, independently of
            // the world-space scale of that segment.
            let sigma_local = 0.7; let local_segment = 0.3;
            let tau_world = sigma_local * local.length() * local_segment * scale_along;
            assert!((tau_world - sigma_local * local_segment).abs() < 1e-6);
        }
        let world_point = transform.transform_point3(Vec3::new(0.2, 0.3, 0.4));
        let roundtrip = Vec3::from_array(tree.inverse.map(|row| Vec4::from(row).dot(world_point.extend(1.0))));
        assert!((roundtrip - Vec3::new(0.2, 0.3, 0.4)).length() < 1e-5);
    }

    #[test]
    fn world_bounds_include_coarse_cells_and_trilinear_filter_support() {
        let source = asset();
        let transform = Affine3A::from_scale_rotation_translation(Vec3::new(2.0, 3.0, 1.5),
            Quat::from_rotation_y(0.8), Vec3::new(12.0, 4.0, -8.0));
        let forest = pack(std::slice::from_ref(&source), &[binding(0, 0)], &[instance(transform)], PackLimits::default()).unwrap();
        let tree = forest.instances[0]; let lo = Vec3::from_slice(&tree.bounds_min); let hi = Vec3::from_slice(&tree.bounds_max);
        for level in &source.levels {
            let mut bounds = level.local_bounds(source.origin);
            for axis in 0..3 {
                bounds[0][axis] -= level.cell_size * 0.5;
                bounds[1][axis] += level.cell_size * 0.5;
            }
            for corner in 0..8 {
                let local = Vec3::from_array(std::array::from_fn(|axis| bounds[usize::from(corner & (1 << axis) != 0)][axis]));
                let world = transform.transform_point3(local);
                assert!((world - lo).min_element() >= -1e-5 && (hi - world).min_element() >= -1e-5);
            }
        }
    }

    #[test]
    fn occupancy_only_levels_do_not_change_shaded_level_ray_bounds() {
        let source = asset();
        let instances = [instance(Affine3A::from_scale_rotation_translation(
            Vec3::new(2.0, 3.0, 1.5), Quat::from_rotation_y(0.8), Vec3::new(12.0, 4.0, -8.0)))];
        let first = pack(std::slice::from_ref(&source), &[binding(0, 0)], &instances,
            PackLimits { levels_per_asset: 1, ..Default::default() }).unwrap();
        let accelerated = pack(&[source], &[binding(0, 0)], &instances,
            PackLimits { first_level_bounds: true, ..Default::default() }).unwrap();
        assert_eq!(accelerated.levels.len(), 2);
        assert_eq!(first.instances[0].bounds_min, accelerated.instances[0].bounds_min);
        assert_eq!(first.instances[0].bounds_max, accelerated.instances[0].bounds_max);
    }

    #[test]
    fn invalid_coordinates_transforms_and_budgets_fail_explicitly() {
        let source = asset(); let instances = [instance(Affine3A::IDENTITY)];
        for limits in [PackLimits { max_dense_indices: 1, ..Default::default() },
            PackLimits { max_sparse_cells: 1, ..Default::default() },
            PackLimits { max_instances: 0, ..Default::default() },
            PackLimits { max_instances: MAX_INSTANCES + 1, ..Default::default() }] {
            assert!(pack(std::slice::from_ref(&source), &[binding(0, 0)], &instances, limits).is_err());
        }
        assert!(pack(std::slice::from_ref(&source), &[binding(1, 0)], &instances, PackLimits::default()).is_err());
        assert!(pack(std::slice::from_ref(&source), &[binding(0, 1)], &instances, PackLimits::default()).is_err());
        assert!(pack(std::slice::from_ref(&source), &[binding(0, 0), binding(0, 0)], &instances, PackLimits::default()).is_err());
        let singular = [instance(Affine3A::from_scale(Vec3::new(1.0, 0.0, 1.0)))];
        assert!(pack(std::slice::from_ref(&source), &[binding(0, 0)], &singular, PackLimits::default()).is_err());
        let mut bad = source.clone(); bad.levels[0].voxels[0].coord = [3, 0, 0];
        assert!(pack(&[bad], &[], &[], PackLimits::default()).is_err());
        let mut bad = source.clone();
        let duplicate = bad.levels[0].voxels[0].clone();
        bad.levels[0].voxels.push(duplicate);
        assert!(pack(&[bad], &[], &[], PackLimits::default()).is_err());
        let mut bad = source; bad.levels[0].voxels[0].directional_area[0] = f32::NAN;
        assert!(pack(&[bad], &[], &[], PackLimits::default()).is_err());
    }
}
