//! Immutable source-derived whole-tree volumes and a bounded world XZ grid.
//!
//! Assets are shared; only transforms and spatial references scale with tree
//! count. A tree belongs to one source replacement group, but its complete
//! wind-expanded volume support can reference several grid cells. Consumers
//! must integrate only the half-open ray interval of the current world cell:
//! references in neighbouring cells never mean additional optical density.
//! This provider does not select source/volume, suppress shadows, or prove
//! optical equivalence. Any construction error leaves the caller's source intact.

use crate::foliage_volume_data::{pack_assets, AssetPackLimits, GpuForest, VolumeBinding, ALL_MATERIALS};
use crate::foliage_voxel::VoxelAsset;
use crate::scene::{GpuInstance, LIVE_GRASS, PART_WIND, WIND_GRASS};
use crate::source_replacement::Group;
use std::collections::{BTreeMap, HashSet};

pub const MAX_GRID_CELLS: usize = 1_000_000;
pub const MAX_REFERENCES: usize = 4_000_000;

#[derive(Clone, Copy, Debug)]
pub struct WorldConfig {
    pub cell_metres: f32,
    /// Bound on the length of the live world-space horizontal shear vector.
    /// The renderer must reject a wind configuration exceeding this bound.
    pub max_wind_shear: f32,
    pub max_instances: usize,
    pub max_grid_cells: usize,
    pub max_references: usize,
    pub generation: u32,
    pub assets: AssetPackLimits,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self { cell_metres: 64.0, max_wind_shear: 0.08, max_instances: 1_000_000,
            max_grid_cells: MAX_GRID_CELLS, max_references: MAX_REFERENCES,
            generation: 1, assets: AssetPackLimits::default() }
    }
}

#[derive(Clone, Debug)]
pub struct WorldGrid {
    /// Signed integer cell coordinates, not metres; X then Z.
    pub origin: [i32; 2],
    pub dimensions: [u32; 2],
    pub cell_metres: f32,
    /// X-major: spans[x + width*z] = [first reference, reference count].
    pub spans: Vec<[u32; 2]>,
    /// Indices into WorldField.forest.instances, never source owner IDs.
    pub references: Vec<u32>,
}

impl WorldGrid {
    pub fn cell_references(&self, cell: [i32; 2]) -> &[u32] {
        let x = i64::from(cell[0]) - i64::from(self.origin[0]);
        let z = i64::from(cell[1]) - i64::from(self.origin[1]);
        if x < 0 || z < 0 || x >= i64::from(self.dimensions[0]) || z >= i64::from(self.dimensions[1]) {
            return &[];
        }
        let span = self.spans[x as usize + self.dimensions[0] as usize * z as usize];
        &self.references[span[0] as usize..span[0] as usize + span[1] as usize]
    }

    pub fn byte_len(&self) -> usize { self.spans.len() * 8 + self.references.len() * 4 }
}

#[derive(Clone, Debug)]
pub struct WorldField {
    pub forest: GpuForest,
    pub grid: WorldGrid,
    pub groups: Vec<Group>,
    /// Canonical root-cell group for each packed tree; one group per tree.
    pub tree_groups: Vec<u32>,
    pub max_wind_shear: f32,
}

fn outward(value: f64, upper: bool) -> Result<f32, String> {
    let rounded = value as f32;
    if !value.is_finite() || !rounded.is_finite() { return Err("world foliage bounds overflow".into()); }
    // One extra ULP also covers subsequent f32 slab arithmetic at the boundary.
    let result = if upper { rounded.next_up() } else { rounded.next_down() };
    if !result.is_finite() { return Err("world foliage outward bounds overflow".into()); }
    Ok(result)
}

fn cell_index(value: f64, cell: f32) -> Result<i32, String> {
    let index = (value / f64::from(cell)).floor();
    if !index.is_finite() || index < f64::from(i32::MIN) || index > f64::from(i32::MAX) {
        return Err("world foliage grid coordinate overflow".into());
    }
    Ok(index as i32)
}

fn union(a: &mut [[f32; 3]; 2], b: [[f32; 3]; 2]) {
    for axis in 0..3 { a[0][axis] = a[0][axis].min(b[0][axis]); a[1][axis] = a[1][axis].max(b[1][axis]); }
}

fn wind_bounds(bounds: [[f64; 3]; 2], root_y: f64, shear: f32) -> Result<[[f32; 3]; 2], String> {
    let reach = (bounds[0][1] - root_y).abs().max((bounds[1][1] - root_y).abs()) * f64::from(shear);
    let mut result = [[0.0; 3]; 2];
    for axis in 0..3 {
        let pad = if axis == 1 { 0.0 } else { reach };
        result[0][axis] = outward(bounds[0][axis] - pad, false)?;
        result[1][axis] = outward(bounds[1][axis] + pad, true)?;
    }
    Ok(result)
}

fn transformed_bounds(source: &GpuInstance, local: [[f32; 3]; 2], shear: f32) -> Result<[[f32; 3]; 2], String> {
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for corner in 0..8 {
        let p: [f64; 3] = std::array::from_fn(|a| f64::from(local[usize::from(corner & (1 << a) != 0)][a]));
        for a in 0..3 {
            let value = f64::from(source.rows[a][3]) + (0..3).map(|k| f64::from(source.rows[a][k]) * p[k]).sum::<f64>();
            bounds[0][a] = bounds[0][a].min(value); bounds[1][a] = bounds[1][a].max(value);
        }
    }
    wind_bounds(bounds, f64::from(source.rows[1][3]), shear)
}

fn source_bounds(source: &GpuInstance, shear: f32) -> Result<[[f32; 3]; 2], String> {
    if source.sphere.iter().any(|v| !v.is_finite()) || source.sphere[3] <= 0.0 {
        return Err("whole-tree source requires a finite positive source sphere".into());
    }
    wind_bounds([
        std::array::from_fn(|a| f64::from(source.sphere[a]) - f64::from(source.sphere[3])),
        std::array::from_fn(|a| f64::from(source.sphere[a]) + f64::from(source.sphere[3])),
    ], f64::from(source.rows[1][3]), shear)
}

/// Upper bound on transformed finest-cell diameter, including arbitrary
/// nonuniform scale and the supported live shear. This is spatial error,
/// not a coverage/colour acceptance certificate.
fn reconstruction_error(source: &GpuInstance, cell: f32, shear: f32) -> Result<f32, String> {
    let norm1 = (0..3).map(|c| (0..3).map(|r| f64::from(source.rows[r][c]).abs()).sum::<f64>()).fold(0.0, f64::max);
    let norm_inf = source.rows.iter().map(|r| r[..3].iter().map(|&v| f64::from(v).abs()).sum::<f64>()).fold(0.0, f64::max);
    let frobenius = source.rows.iter().flat_map(|r| &r[..3]).map(|&v| f64::from(v).powi(2)).sum::<f64>().sqrt();
    let s = f64::from(shear);
    let shear_norm = (1.0 + s*s/4.0).sqrt() + s/2.0;
    outward(f64::from(cell) * 3.0f64.sqrt() * frobenius.min((norm1*norm_inf).sqrt()) * shear_norm, true)
}

impl WorldField {
    pub fn build(assets: &[VoxelAsset], bindings: &[VolumeBinding], instances: &[GpuInstance], config: WorldConfig) -> Result<Self, String> {
        if !config.cell_metres.is_finite() || config.cell_metres <= 0.0
            || !config.max_wind_shear.is_finite() || !(0.0..=1.0).contains(&config.max_wind_shear)
            || config.generation == 0 || config.max_instances == 0 || config.max_instances > u32::MAX as usize
            || config.max_grid_cells == 0 || config.max_grid_cells > MAX_GRID_CELLS
            || config.max_references == 0 || config.max_references > MAX_REFERENCES
            || config.assets.first_level_bounds
        {
            return Err("invalid world foliage budgets, generation, wind or mip-support policy".into());
        }
        if bindings.is_empty() || bindings.len() > config.max_instances {
            return Err("world foliage instance budget exceeded or empty input".into());
        }
        let mut packed = pack_assets(assets, config.assets)?;
        let mut owners = HashSet::with_capacity(bindings.len());
        let mut rectangles = Vec::with_capacity(bindings.len());
        let mut canonical = BTreeMap::<[i32; 2], Vec<usize>>::new();
        let mut envelopes = Vec::with_capacity(bindings.len());
        let mut errors = Vec::with_capacity(bindings.len());
        let mut grid_min = [i32::MAX; 2];
        let mut grid_max = [i32::MIN; 2];
        let mut reference_count = 0usize;
        for (tree_index, &binding) in bindings.iter().enumerate() {
            if binding.crown_material != ALL_MATERIALS || !owners.insert(binding.owner) {
                return Err("world foliage requires unique owners and complete ALL_MATERIALS bindings".into());
            }
            let source = instances.get(binding.owner as usize).ok_or("world foliage owner out of bounds")?;
            if source.cluster_count == 0 || source.flags & (PART_WIND | LIVE_GRASS | WIND_GRASS) != 0 {
                return Err("world foliage requires an active whole source with rigid tree-shear wind".into());
            }
            let mut tree = packed.instance(binding, source)?;
            let span = packed.spans[binding.asset as usize];
            let bounds = transformed_bounds(source, span.bounds, config.max_wind_shear)?;
            tree.bounds_min[..3].copy_from_slice(&bounds[0]);
            tree.bounds_max[..3].copy_from_slice(&bounds[1]);
            packed.forest.instances.push(tree);
            let mut envelope = source_bounds(source, config.max_wind_shear)?;
            union(&mut envelope, bounds);
            envelopes.push(envelope);
            let first_level = &packed.forest.levels[span.levels[0] as usize];
            errors.push(reconstruction_error(source, first_level.origin_cell[3], config.max_wind_shear)?);
            let root_cell = [cell_index(f64::from(source.rows[0][3]), config.cell_metres)?,
                cell_index(f64::from(source.rows[2][3]), config.cell_metres)?];
            canonical.entry(root_cell).or_default().push(tree_index);
            if canonical.len() > crate::source_replacement::MAX_GROUPS {
                return Err("world foliage canonical-group budget exceeded".into());
            }
            let low = [cell_index(f64::from(bounds[0][0]), config.cell_metres)?, cell_index(f64::from(bounds[0][2]), config.cell_metres)?];
            let high = [cell_index(f64::from(bounds[1][0]), config.cell_metres)?, cell_index(f64::from(bounds[1][2]), config.cell_metres)?];
            let cells_x = (i64::from(high[0]) - i64::from(low[0]) + 1) as u64;
            let cells_z = (i64::from(high[1]) - i64::from(low[1]) + 1) as u64;
            let count = cells_x.checked_mul(cells_z).ok_or("world foliage reference overflow")?;
            if count > config.max_references as u64 { return Err("world foliage per-tree reference budget exceeded".into()); }
            reference_count = reference_count.checked_add(count as usize).ok_or("world foliage reference count overflow")?;
            if reference_count > config.max_references { return Err("world foliage reference budget exceeded".into()); }
            for a in 0..2 { grid_min[a] = grid_min[a].min(low[a]); grid_max[a] = grid_max[a].max(high[a]); }
            rectangles.push([low, high]);
        }
        // Coarse empty-space certificates require real dyadic parents, not
        // merely increasing cell sizes. Reject before any source is omitted.
        if !crate::voxel_forest::valid_skip_levels(&packed.forest, true) {
            return Err("world foliage requires a complete dyadic occupancy hierarchy for empty-space traversal".into());
        }
        let dimensions64: [u64; 2] = std::array::from_fn(|a| (i64::from(grid_max[a]) - i64::from(grid_min[a]) + 1) as u64);
        let grid_count = dimensions64[0].checked_mul(dimensions64[1]).ok_or("world foliage grid size overflow")?;
        if grid_count > config.max_grid_cells as u64 { return Err("world foliage grid-cell budget exceeded".into()); }
        let dimensions = dimensions64.map(|v| v as u32);
        let mut counts = vec![0u32; grid_count as usize];
        let index = |x: i32, z: i32| (i64::from(x)-i64::from(grid_min[0])) as usize
            + dimensions[0] as usize * (i64::from(z)-i64::from(grid_min[1])) as usize;
        for [low, high] in &rectangles {
            for z in low[1]..=high[1] { for x in low[0]..=high[0] { counts[index(x,z)] += 1; } }
        }
        let mut cursor = 0u32;
        let spans: Vec<_> = counts.iter().map(|&count| { let first = cursor; cursor += count; [first, count] }).collect();
        let mut references = vec![0; reference_count];
        counts.fill(0);
        for (tree_index, [low, high]) in rectangles.iter().enumerate() {
            for z in low[1]..=high[1] { for x in low[0]..=high[0] {
                let cell = index(x,z);
                references[(spans[cell][0] + counts[cell]) as usize] = tree_index as u32;
                counts[cell] += 1;
            } }
        }
        let mut tree_groups = vec![0; bindings.len()];
        let mut groups = Vec::with_capacity(canonical.len());
        for (cell, trees) in canonical {
            let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
            let mut error_radius = 0.0f32;
            let mut members = Vec::with_capacity(trees.len());
            for tree_index in trees {
                tree_groups[tree_index] = groups.len() as u32;
                members.push(bindings[tree_index].owner);
                union(&mut bounds, envelopes[tree_index]);
                error_radius = error_radius.max(errors[tree_index]);
            }
            groups.push(Group { key: (u64::from(cell[0] as u32) << 32) | u64::from(cell[1] as u32),
                generation: config.generation, members, bounds, error_radius });
        }
        Ok(Self { forest: packed.forest, grid: WorldGrid { origin: grid_min, dimensions,
            cell_metres: config.cell_metres, spans, references }, groups, tree_groups,
            max_wind_shear: config.max_wind_shear })
    }

    /// Packed immutable provider arrays, excluding renderer-owned per-view
    /// selection, readiness, and source-replacement group uploads.
    pub fn byte_len(&self) -> usize {
        self.forest.byte_len() + self.grid.byte_len() + self.tree_groups.len() * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_voxel::{BakeStats, Voxel, VoxelLevel, AXIS_NORMAL_CENTROIDS};
    use bytemuck::Zeroable;
    use glam::{Affine3A, Mat4, Quat, Vec3};

    fn asset() -> VoxelAsset {
        VoxelAsset { projection_measure: Default::default(), origin: [-0.5, 0.0, -0.5],
            source_bounds: [[-0.5,0.0,-0.5], [0.5,2.0,0.5]], stats: BakeStats::default(),
            levels: vec![VoxelLevel { cell_size: 0.5, dimensions: [2,4,2], voxels: vec![Voxel {
                coord: [0,0,0], surface_area: 0.1, projected_area: [0.1;3], directional_area: [0.1;13],
                albedo: [0.2,0.4,0.1], roughness: 0.8, leaf_fraction: 0.8,
                normal_weights: [0.5,0.5,0.0,0.0,0.0,0.0], normal_centroids: AXIS_NORMAL_CENTROIDS,
            }] }] }
    }
    fn instance(at: Vec3) -> GpuInstance {
        let mut i = GpuInstance::zeroed();
        i.rows = [[1.0,0.0,0.0,at.x], [0.0,1.0,0.0,at.y], [0.0,0.0,1.0,at.z]];
        i.sphere = [at.x,at.y+1.0,at.z,2.0]; i.cluster_count=1; i.scale=1.0; i
    }
    fn bindings(n: usize) -> Vec<VolumeBinding> {
        (0..n).map(|i| VolumeBinding { owner:i as u32, asset:0, crown_material:ALL_MATERIALS }).collect()
    }

    #[test]
    fn shared_asset_has_no_patch_owner_limit_or_per_tree_cell_copy() {
        let instances:Vec<_> = (0..1024).map(|i| instance(Vec3::new((i%32) as f32*6.0,0.0,(i/32) as f32*6.0))).collect();
        let field=WorldField::build(&[asset()],&bindings(instances.len()),&instances,WorldConfig::default()).unwrap();
        assert_eq!(field.forest.instances.len(),1024);
        assert_eq!(field.forest.cells.len(),1);
        assert_eq!(field.forest.indices.len(),16);
        assert_eq!(field.groups.iter().map(|g|g.members.len()).sum::<usize>(),1024);
        for (tree,&group) in field.tree_groups.iter().enumerate() {
            assert!(field.groups[group as usize].members.contains(&(tree as u32)));
        }
        assert!(crate::foliage_volume_data::pack(&[asset()],&bindings(instances.len()),&instances,Default::default()).is_err());
    }

    #[test]
    fn boundary_crossing_references_are_complete_unique_and_ordered() {
        let instances=[instance(Vec3::new(-0.1,0.0,63.9)),instance(Vec3::new(64.1,0.0,-0.1))];
        let field=WorldField::build(&[asset()],&bindings(2),&instances,WorldConfig::default()).unwrap();
        for (tree,t) in field.forest.instances.iter().enumerate() {
            for corner in 0..8 {
                let p: [f32;3]=std::array::from_fn(|a| if corner&(1<<a)==0 {t.bounds_min[a]} else {t.bounds_max[a]});
                let cell=[cell_index(f64::from(p[0]),64.0).unwrap(),cell_index(f64::from(p[2]),64.0).unwrap()];
                assert!(field.grid.cell_references(cell).contains(&(tree as u32)));
            }
        }
        for span in &field.grid.spans {
            let refs=&field.grid.references[span[0] as usize..(span[0]+span[1]) as usize];
            assert!(refs.windows(2).all(|p|p[0]<p[1]));
        }
        let again=WorldField::build(&[asset()],&bindings(2),&instances,WorldConfig::default()).unwrap();
        assert_eq!(field.grid.references,again.grid.references);
        assert_eq!(field.grid.spans,again.grid.spans);
        assert_eq!(field.tree_groups,again.tree_groups);
    }

    #[test]
    fn affine_support_and_source_spheres_remain_inside_maximum_wind_groups() {
        let affine=Affine3A::from_scale_rotation_translation(Vec3::new(2.0,3.0,0.7),Quat::from_rotation_y(0.7),Vec3::new(-64.0,5.0,127.0));
        let rows=Mat4::from(affine).transpose();
        let mut source=instance(affine.translation.into());
        source.rows=[rows.x_axis.to_array(),rows.y_axis.to_array(),rows.z_axis.to_array()];
        source.sphere=[-64.0,8.0,127.0,8.0];
        let field=WorldField::build(&[asset()],&bindings(1),&[source],WorldConfig::default()).unwrap();
        let group=&field.groups[0];
        let local=[[-0.75,-0.25,-0.75],[0.75,2.25,0.75]];
        for direction in 0..32 {
            let phase=direction as f64*std::f64::consts::TAU/32.0;
            let shear=[phase.cos()*0.08,phase.sin()*0.08];
            for corner in 0..8 {
                let p:[f64;3]=std::array::from_fn(|a|f64::from(local[usize::from(corner&(1<<a)!=0)][a]));
                let mut w:[f64;3]=std::array::from_fn(|a|f64::from(source.rows[a][3])+(0..3).map(|k|p[k]*f64::from(source.rows[a][k])).sum::<f64>());
                w[0]+=shear[0]*(w[1]-5.0); w[2]+=shear[1]*(w[1]-5.0);
                for a in 0..3 {
                    assert!(w[a]>=f64::from(field.forest.instances[0].bounds_min[a]) && w[a]<=f64::from(field.forest.instances[0].bounds_max[a]));
                    assert!(w[a]>=f64::from(group.bounds[0][a]) && w[a]<=f64::from(group.bounds[1][a]));
                }
            }
        }
        for a in 0..3 {
            assert!(group.bounds[0][a]<=source.sphere[a]-source.sphere[3]);
            assert!(group.bounds[1][a]>=source.sphere[a]+source.sphere[3]);
        }
        assert!(group.error_radius>=0.5*3.0f32.sqrt()*3.0);
    }

    #[test]
    fn invalid_or_over_budget_fields_fail_without_changing_source() {
        let source=[instance(Vec3::ZERO)]; let baseline=bytemuck::bytes_of(&source[0]).to_vec();
        for config in [WorldConfig {max_references:1,..Default::default()},WorldConfig{max_grid_cells:1,..Default::default()},
            WorldConfig{max_wind_shear:f32::NAN,..Default::default()},WorldConfig{generation:0,..Default::default()}] {
            assert!(WorldField::build(&[asset()],&bindings(1),&source,config).is_err());
        }
        assert!(WorldField::build(&[asset()],&[bindings(1)[0],bindings(1)[0]],&source,Default::default()).is_err());
        let mut bad=bindings(1);bad[0].crown_material=2;
        assert!(WorldField::build(&[asset()],&bad,&source,Default::default()).is_err());
        let mut bad_source=source;bad_source[0].flags=PART_WIND;
        assert!(WorldField::build(&[asset()],&bindings(1),&bad_source,Default::default()).is_err());
        assert_eq!(bytemuck::bytes_of(&source[0]),baseline);
    }

    #[test]
    fn world_rejects_missing_and_non_dyadic_parents_before_source_publication() {
        let source = [instance(Vec3::ZERO)];
        let before = bytemuck::bytes_of(&source[0]).to_vec();
        let mut complete = asset();
        complete.levels.push(VoxelLevel { cell_size: 1.0, dimensions: [1,2,1],
            voxels: vec![complete.levels[0].voxels[0].clone()] });
        assert!(WorldField::build(&[complete.clone()], &bindings(1), &source, WorldConfig::default()).is_ok());
        let mut missing = complete.clone();
        missing.levels[1].voxels.clear();
        let mut non_dyadic = complete.clone();
        non_dyadic.levels[1].cell_size = 1.1;
        let mut wrong_dimensions = complete;
        wrong_dimensions.levels[1].dimensions[1] = 3;
        for malformed in [missing, non_dyadic, wrong_dimensions] {
            // Legal standalone density assets can still be sampled without
            // acceleration; the world path must not trust them as skip proofs.
            assert!(pack_assets(std::slice::from_ref(&malformed), Default::default()).is_ok());
            let error = WorldField::build(&[malformed], &bindings(1), &source, WorldConfig::default()).unwrap_err();
            assert!(error.contains("dyadic occupancy hierarchy"), "{error}");
        }
        assert_eq!(bytemuck::bytes_of(&source[0]), before);
    }
}
