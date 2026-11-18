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
pub const MAX_INSTANCES: usize = crate::voxel_forest::MAX_VIEW_GROUPS;
/// Replace every source material for a procedural part. Its voxel albedo
/// already contains the individual source materials' linear colours.
pub const ALL_MATERIALS: u32 = u32::MAX;

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

/// Shared asset budgets, independent of any renderer's instance capacity.
#[derive(Clone, Copy, Debug)]
pub struct AssetPackLimits {
    pub max_assets: usize,
    pub levels_per_asset: usize,
    pub first_level_bounds: bool,
    pub max_dense_indices: usize,
    pub max_sparse_cells: usize,
}

impl From<PackLimits> for AssetPackLimits {
    fn from(limits: PackLimits) -> Self {
        Self { max_assets: limits.max_assets, levels_per_asset: limits.levels_per_asset,
            first_level_bounds: limits.first_level_bounds, max_dense_indices: limits.max_dense_indices,
            max_sparse_cells: limits.max_sparse_cells }
    }
}

impl Default for AssetPackLimits {
    fn default() -> Self { PackLimits::default().into() }
}

/// Local support includes every shaded mip's half-cell reconstruction tent.
#[derive(Clone, Copy, Debug)]
pub struct AssetSpan {
    pub levels: [u32; 2],
    pub bounds: [[f32; 3]; 2],
}

/// One immutable upload shared by any number of world instances. The empty
/// forest.instances vector is populated by a consumer with its own limits.
#[derive(Clone, Debug)]
pub struct PackedAssets {
    pub forest: GpuForest,
    pub spans: Vec<AssetSpan>,
}

impl PackedAssets {
    pub fn instance(&self, binding: VolumeBinding, source: &GpuInstance) -> Result<GpuTree, String> {
        let span = self.spans.get(binding.asset as usize)
            .ok_or_else(|| format!("volume source asset {} is out of bounds", binding.asset))?;
        pack_tree(binding, source, span.levels, Vec3::from(span.bounds[0]), Vec3::from(span.bounds[1]))
    }
}

impl Default for PackLimits {
    fn default() -> Self {
        Self {
            max_instances: 256,
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
    /// ALL_MATERIALS replaces the complete procedural part, including variation.
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

/// WGSL: eleven vec4f records, 176 bytes. Values remain linear and unquantized.
/// Actual normal centroids add 64 bytes/cell over the previous axis-only data.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuCell {
    pub albedo_rough: [f32; 4],
    /// Thirteen directional extinction coefficients per local metre, followed
    /// by leaf fraction and two zeroes. Direction order is unchanged.
    pub extinction: [[f32; 4]; 4],
    /// Six actual authored-normal centroids: xyz conditional mean, w lobe weight.
    /// The mean length is in 0..=1; do not normalize it before interpolation.
    /// Interpolate xyz with extinction*lobe_weight, then normalize the result;
    /// never average unweighted directions or substitute the signed basis axis.
    pub normals: [[f32; 4]; 6],
}

/// Storage form, 64 bytes (sixteen words): fourteen extinction values as
/// half floats (words 0..7, value `i` in word `i/2`, half `i%2`), colour and
/// roughness as unorm16 (words 7, 8), six normal moments as snorm8 xyzw
/// (words 9..15), one spare word. Do not normalize centroids before packing:
/// their length carries the distribution's concentration. Logical/cooked
/// cells remain f32.
#[repr(C)]
#[derive(Clone,Copy,Debug,Pod,Zeroable)]
pub struct PackedCell(pub [u32;16]);
/// Nearest half float of a non-negative value. A positive value never packs
/// to zero (an occupied direction stays occupied) and never to infinity.
fn half(v:f32)->u32 {
    if v.is_nan() {return 0x7e00;}
    if v<=0. {return 0;}
    let bits=v.min(65504.).to_bits();let e=((bits>>23)&0xff)as i32-112;let m=bits&0x7f_ffff;
    if e<=0 {
        if e< -10 {return 1;}
        let shift=(14-e)as u32;return (((m|0x80_0000)+(1<<(shift-1)))>>shift).max(1);
    }
    ((((e as u32)<<10)|(m>>13))+((m>>12)&1)).min(0x7bff)
}
fn half_value(h:u32)->f32 {
    let e=(h>>10)&31;let m=(h&0x3ff)as f32;
    if e==0 {m*2f32.powi(-24)} else if e==31 {if m==0. {f32::INFINITY}else{f32::NAN}} else {(1.+m/1024.)*2f32.powi(e as i32-15)}
}
impl From<&GpuCell> for PackedCell {
    fn from(cell:&GpuCell)->Self {
        let mut w=[0u32;16];
        for i in 0..14 {w[i/2]|=half(cell.extinction[i/4][i%4])<<(16*(i%2));}
        let unorm=|v:f32|(v.clamp(0.,1.)*65535.).round()as u32;
        w[7]=unorm(cell.albedo_rough[0])|(unorm(cell.albedo_rough[1])<<16);w[8]=unorm(cell.albedo_rough[2])|(unorm(cell.albedo_rough[3])<<16);
        let snorm=|v:f32|(v.clamp(-1.,1.)*127.).round()as i8 as u8 as u32;
        for (i,n) in cell.normals.iter().enumerate() {w[9+i]=snorm(n[0])|(snorm(n[1])<<8)|(snorm(n[2])<<16)|(snorm(n[3])<<24);}
        Self(w)
    }
}
impl PackedCell {
    pub fn decode(&self)->GpuCell {
        let w=&self.0;let mut cell=GpuCell::zeroed();
        for i in 0..14 {cell.extinction[i/4][i%4]=half_value((w[i/2]>>(16*(i%2)))&0xffff);}
        let unorm=|v:u32|(v&0xffff)as f32/65535.;
        cell.albedo_rough=[unorm(w[7]),unorm(w[7]>>16),unorm(w[8]),unorm(w[8]>>16)];
        for i in 0..6 {cell.normals[i]=std::array::from_fn(|k|((w[9+i]>>(8*k))as u8 as i8 as f32/127.).max(-1.));}
        cell
    }
}
pub fn packed_cells(cells:&[GpuCell])->Vec<PackedCell> {cells.iter().map(PackedCell::from).collect()}

/// WGSL: six vec4 records, 96 bytes. Shared part-wind can update these poses;
/// asset cells and grids stay immutable.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTree {
    /// Rows of world-to-local affine transform, including translation.
    pub inverse: [[f32; 4]; 3],
    pub bounds_min: [f32; 4],
    /// xyz bounds; w=0 for legacy, positive exact-float appearance seed for GPU forest.
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
    /// Append another densely indexed forest's levels, cells and trees,
    /// rebasing its indices. Trees keep their order after this forest's.
    pub fn append(&mut self, other: GpuForest) -> Result<(), String> {
        if other.levels.iter().any(|l| l.dims_offset[3] & BRICK_INDIRECTION != 0) {
            return Err("only dense volume levels can be appended".into());
        }
        let (levels, indices, cells) = (self.levels.len(), self.indices.len(), self.cells.len());
        if [levels + other.levels.len(), indices + other.indices.len(), cells + other.cells.len()].iter()
            .any(|&n| n >= BRICK_INDIRECTION as usize) {
            return Err("appended volume forest exceeds 31-bit addressing".into());
        }
        self.levels.extend(other.levels.into_iter().map(|mut l| { l.dims_offset[3] += indices as u32; l }));
        self.indices.extend(other.indices.into_iter().map(|i| if i == 0 { 0 } else { i + cells as u32 }));
        self.cells.extend(other.cells);
        self.instances.extend(other.instances.into_iter().map(|mut t| { t.meta[1] += levels as u32; t }));
        Ok(())
    }
    /// Expected appended splats per level before view rejection: the raster
    /// drops cells under 1/255 alpha and keeps cells under
    /// `SPLAT_MIN_ALPHA` with probability alpha/min (voxel_forest.wgsl
    /// volume_splat_append). Alpha is bounded by the strongest of the thirteen
    /// directions over one cell, so this is an upper bound on the expectation.
    /// Count grid entries, not unique payloads: aliases rasterize once per
    /// coordinate. Computed once for the shared asset catalog.
    pub(crate) fn raster_work_bounds(&self) -> Result<Vec<u32>, String> {
        // Largest chord over a cell in units of its size: the drawn disc's mean
        // chord 1/1.48711 (KERN_VOXEL_DISC_DEPTH), else the cube chord's maximum 1.
        let chord = if crate::switches::VOXEL_DISC_DEPTH.off() { 1.0 } else { 1.0 / 1.48711f32 };
        let kept: Vec<f32> = self.cells.iter().map(|c| c.extinction.as_flattened()[..13].iter().fold(0f32, |m, &v| m.max(v))).collect();
        let payload = |entries: &[u32], size: f32| -> Result<f64, String> {
            let mut sum = 0f64;
            for &i in entries.iter().filter(|&&i| i != 0) {
                let sigma = *kept.get(i as usize - 1).ok_or("raster work references a missing voxel cell")?;
                let alpha = 1.0 - (-sigma.max(0.0) * size * chord).exp();
                if alpha >= 1.0 / 255.0 { sum += f64::from((alpha / SPLAT_MIN_ALPHA).min(1.0)); }
            }
            Ok(sum)
        };
        self.levels.iter().map(|level| {
            let dims = &level.dims_offset[..3];
            if dims.contains(&0) { return Err("raster work has empty dimensions".into()); }
            let size = level.origin_cell[3];
            let bricked = level.dims_offset[3] & BRICK_INDIRECTION != 0;
            let length = dims.iter().try_fold(1usize, |n, &d|
                n.checked_mul(if bricked { d.div_ceil(4) } else { d } as usize))
                .ok_or("raster work grid overflow")?;
            let base = (level.dims_offset[3] & !BRICK_INDIRECTION) as usize;
            let end = base.checked_add(length).ok_or("raster work address overflow")?;
            let entries = self.indices.get(base..end).ok_or("raster work grid outside catalog")?;
            let expected = if !bricked { payload(entries, size)? } else {
                let mut sum = 0f64;
                for &address in entries.iter().filter(|&&i| i != 0) {
                    let start = (address - 1) as usize;
                    let end = start.checked_add(64).ok_or("raster work brick overflow")?;
                    let brick = self.indices.get(start..end).ok_or("raster work brick outside catalog")?;
                    sum += payload(brick, size)?;
                }
                sum
            };
            u32::try_from(expected.ceil() as u64).map_err(|_| "raster work exceeds u32".into())
        }).collect()
    }
    pub fn byte_len(&self) -> usize {
        self.levels.len() * std::mem::size_of::<GpuLevel>()
            + self.indices.len() * std::mem::size_of::<u32>()
            + self.cells.len() * std::mem::size_of::<GpuCell>()
            + self.instances.len() * std::mem::size_of::<GpuTree>()
    }
}

/// Aggregate levels use a sparse directory of 4x4x4 bricks. Bit31 of the
/// level offset tags this encoding; ordinary providers retain dense indices.
pub const BRICK_INDIRECTION: u32 = 1 << 31;
/// Bit30 on a bricked level: an occupied brick is three words (occupancy
/// mask low and high, address of its first cell) instead of 64 cell
/// addresses, its cells stored in mask-bit order. Voxel pages use it
/// (foliage_pages.rs): needles fill a brick thinly, and 64 words per brick
/// made the directory of a fine level several times the size of its cells.
pub const COMPACT_BRICKS: u32 = 1 << 30;
/// voxel_forest.wgsl VOLUME_SPLAT_MIN_ALPHA: fainter cells are thinned to this alpha.
pub(crate) const SPLAT_MIN_ALPHA: f32 = 0.5;
pub(crate) fn cell_index(forest:&GpuForest,level:&GpuLevel,c:[u32;3])->Option<u32> {
    if (0..3).any(|a|c[a]>=level.dims_offset[a]) {return Some(0);}
    let base=(level.dims_offset[3]&!(BRICK_INDIRECTION|COMPACT_BRICKS))as usize;
    if level.dims_offset[3]&BRICK_INDIRECTION==0 {
        return forest.indices.get(base+c[0]as usize+level.dims_offset[0]as usize*
            (c[1]as usize+level.dims_offset[1]as usize*c[2]as usize)).copied();
    }
    let dx=level.dims_offset[0].div_ceil(4)as usize;let dy=level.dims_offset[1].div_ceil(4)as usize;
    let brick=*forest.indices.get(base+(c[0]/4)as usize+dx*((c[1]/4)as usize+dy*(c[2]/4)as usize))?;
    if brick==0 {return Some(0);}
    let bit=c[0]%4+4*(c[1]%4)+16*(c[2]%4);
    if level.dims_offset[3]&COMPACT_BRICKS!=0 {
        let record=forest.indices.get((brick-1)as usize..(brick+2)as usize)?;
        let mask=record[0]as u64|(record[1]as u64)<<32;
        if mask>>bit&1==0 {return Some(0);}
        return Some(record[2]+(mask&((1u64<<bit)-1)).count_ones());
    }
    forest.indices.get((brick-1)as usize+bit as usize).copied()
}

/// Convert only an aggregate provider after packing. Every level is bounded
/// independently; failure leaves the original provider unchanged.
/// Most bricks one packed level may hold. The splat pass walks a level's
/// bricks 64 at a time in one workgroup, so this bounds that loop, not a
/// GPU structure; cooks keep 128 (MAX_BRICKS_PER_LEVEL) unless
/// KERN_BRANCH_BRICKS raises it for fine branch cells.
pub const MAX_PACKED_BRICKS_PER_LEVEL: usize = 4096;
pub fn pack_bricks(forest:&mut GpuForest,max_bricks_per_level:usize)->Result<Vec<usize>,String> {
    if max_bricks_per_level==0||max_bricks_per_level>MAX_PACKED_BRICKS_PER_LEVEL {return Err("aggregate brick limit must be 1..MAX_PACKED_BRICKS_PER_LEVEL".into());}
    let mut indices=Vec::new();let mut levels=forest.levels.clone();let mut counts=Vec::new();
    for level in &mut levels {
        let dims=[level.dims_offset[0],level.dims_offset[1],level.dims_offset[2]];
        if dims.contains(&0) {return Err("aggregate brick dimensions must be positive".into());}
        let bd=dims.map(|v|v.div_ceil(4));
        let count=bd.into_iter().try_fold(1usize,|n,v|n.checked_mul(v as usize)).ok_or("brick directory overflow")?;
        // A valid source already stores at least this many directory/dense
        // entries. Reject forged dimensions before allocating their directory;
        // neither dense nor already-bricked input can justify a larger one.
        let source_base=(level.dims_offset[3]&!BRICK_INDIRECTION)as usize;
        let source_count=if level.dims_offset[3]&BRICK_INDIRECTION!=0 {count} else {
            dims.into_iter().try_fold(1usize,|n,v|n.checked_mul(v as usize)).ok_or("original aggregate grid overflow")?
        };
        if count>forest.indices.len() || source_base.checked_add(source_count)
            .is_none_or(|end|end>forest.indices.len()) {
            return Err("invalid original aggregate directory/grid range".into());
        }
        let base=indices.len();let end=base.checked_add(count).ok_or("brick directory overflow")?;
        if end>=COMPACT_BRICKS as usize {return Err("brick directory exceeds 30 bit addressing".into());}
        indices.resize(end,0);let mut occupied=0;
        for z in 0..bd[2] {for y in 0..bd[1] {for x in 0..bd[0] {
            let mut cells=[0u32;64];
            for cz in 0..4 {for cy in 0..4 {for cx in 0..4 {
                let reference=cell_index(forest,level,[x*4+cx,y*4+cy,z*4+cz])
                    .ok_or("invalid original aggregate cell address")?;
                if reference as usize>forest.cells.len() {return Err("invalid original aggregate sparse-cell reference".into());}
                cells[(cx+4*cy+16*cz)as usize]=reference;
            }}}
            if cells.iter().all(|v|*v==0){continue;}
            occupied+=1;if occupied>max_bricks_per_level {return Err("aggregate level exceeds its brick budget".into());}
            if indices.len().checked_add(64).is_none_or(|end|end>=COMPACT_BRICKS as usize) {
                return Err("brick payload exceeds31bit addressing".into());
            }
            indices[base+x as usize+bd[0]as usize*(y as usize+bd[1]as usize*z as usize)]=indices.len()as u32+1;
            indices.extend(cells);
        }}}
        level.dims_offset[3]=base as u32|BRICK_INDIRECTION;counts.push(occupied);
    }
    forest.indices=indices;forest.levels=levels;Ok(counts)
}

fn valid_nonnegative(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite() && *value >= 0.0)
}

fn pack_cell(voxel: &Voxel, cell_size: f32) -> Result<GpuCell, String> {
    if !valid_nonnegative(&voxel.directional_area)
        || !valid_nonnegative(&voxel.albedo)
        || !valid_nonnegative(&voxel.normal_weights)
        || voxel.normal_centroids.iter().flatten().any(|x| !x.is_finite())
        || !voxel.roughness.is_finite()
        || !(0.0..=1.0).contains(&voxel.roughness)
        || !voxel.leaf_fraction.is_finite()
        || !(0.0..=1.0).contains(&voxel.leaf_fraction)
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
        if *weight == 0.0 { continue; }
        let normal = Vec3::from(voxel.normal_centroids[index]);
        let length_squared: f64 = voxel.normal_centroids[index].iter().map(|&x| f64::from(x).powi(2)).sum();
        if length_squared <= 0.0 || length_squared > 1.001 {
            return Err("occupied volume normal lobe requires a nonzero conditional mean of unit normals".into());
        }
        cell.normals[index] = normal.extend(*weight).to_array();
    }
    cell.extinction[3][1] = voxel.leaf_fraction;
    Ok(cell)
}

pub(crate) fn matrix(rows: [[f32; 4]; 3]) -> Mat4 {
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
    let mut packed = pack_assets(assets, limits.into())?;
    let mut owners = HashSet::with_capacity(bindings.len());
    for &binding in bindings {
        if !owners.insert(binding.owner) {
            return Err(format!("duplicate volume source owner {} would double the density", binding.owner));
        }
        let instance = scene_instances.get(binding.owner as usize)
            .ok_or_else(|| format!("volume source owner {} is out of bounds", binding.owner))?;
        let tree = packed.instance(binding, instance)?;
        packed.forest.instances.push(tree);
    }
    Ok(packed.forest)
}

/// Pack and validate source assets without imposing the old patch renderer's
/// 256-instance limit. Does not change the legacy pack() capacity or semantics.
pub fn pack_assets(assets: &[VoxelAsset], limits: AssetPackLimits) -> Result<PackedAssets, String> {
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
        asset_spans.push(AssetSpan { levels: [first, forest.levels.len() as u32 - first],
            bounds: [local_min.to_array(), local_max.to_array()] });
    }
    Ok(PackedAssets { forest, spans: asset_spans })
}

#[cfg(test)]
mod tests {
    #[test]
    fn raster_work_counts_grid_aliases_and_validates_addresses() {
        let mut f=super::GpuForest {
            levels:vec![super::GpuLevel {origin_cell:[0.,0.,0.,1.],dims_offset:[3,1,1,0]}],
            indices:vec![1,0,1],cells:vec![bytemuck::Zeroable::zeroed()],instances:vec![]};
        assert_eq!(f.raster_work_bounds().unwrap(),[0],"empty optics never rasterize");
        f.cells[0].extinction[1][2]=50.;
        assert_eq!(f.raster_work_bounds().unwrap(),[2]);
        // A faint cell is thinned to the minimum alpha: half a splat expected each.
        f.cells[0].extinction[1][2]=-(1.-0.25f32).ln();
        assert_eq!(f.raster_work_bounds().unwrap(),[1]);
        f.cells[0].extinction[1][2]=0.001;
        assert_eq!(f.raster_work_bounds().unwrap(),[0],"under 1/255 alpha is dropped");
        f.cells[0].extinction[1][2]=50.;
        super::pack_bricks(&mut f,128).unwrap();
        assert_eq!(f.raster_work_bounds().unwrap(),[2]);
        f.indices[0]=u32::MAX;
        assert!(f.raster_work_bounds().is_err());
        f.levels[0].dims_offset=[u32::MAX,u32::MAX,u32::MAX,0];
        assert!(f.raster_work_bounds().is_err());
    }

    use super::*;
    use crate::foliage_voxel::{BakeStats, PROJECTION_DIRECTIONS, VoxelLevel};
    use glam::{Affine3A, Quat};

    fn voxel(coord: [u32; 3], area: f32) -> Voxel {
        Voxel { coord, surface_area: area, projected_area: [area, 0.0, 0.0],
            directional_area: PROJECTION_DIRECTIONS.map(|direction| direction[0].abs() * area),
            albedo: [0.2, 0.4, 0.1], roughness: 0.8, leaf_fraction: 0.0,
            normal_weights: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0],
            normal_centroids: crate::foliage_voxel::AXIS_NORMAL_CENTROIDS }
    }
    fn asset() -> VoxelAsset {
        VoxelAsset { projection_measure: Default::default(), origin: [-0.5, 0.0, -0.5], source_bounds: [[-0.5, 0.0, -0.5], [0.0, 0.5, 0.0]],
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
        assert_eq!(std::mem::size_of::<GpuCell>(), 176);
        assert_eq!(std::mem::offset_of!(GpuCell, extinction), 16);
        assert_eq!(std::mem::offset_of!(GpuCell, normals), 80);
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
        assert_eq!(forest.byte_len(), 2 * 32 + 35 * 4 + 3 * 176 + 2 * 96);
    }

    #[test]
    fn independent_asset_packing_preserves_legacy_upload_bytes_and_instance_bounds() {
        let assets = [asset()];
        let source = instance(Affine3A::from_scale_rotation_translation(
            Vec3::new(0.7, 1.8, 2.1), Quat::from_rotation_y(0.37), Vec3::new(-7.0, 4.0, 31.0)));
        for first_level_bounds in [false, true] {
            let limits = PackLimits { first_level_bounds, ..Default::default() };
            let legacy = pack(&assets, &[binding(0,0)], &[source], limits).unwrap();
            let shared = pack_assets(&assets, limits.into()).unwrap();
            assert!(shared.forest.instances.is_empty());
            assert_eq!(bytemuck::cast_slice::<_,u8>(&legacy.levels), bytemuck::cast_slice::<_,u8>(&shared.forest.levels));
            assert_eq!(legacy.indices, shared.forest.indices);
            assert_eq!(bytemuck::cast_slice::<_,u8>(&legacy.cells), bytemuck::cast_slice::<_,u8>(&shared.forest.cells));
            assert_eq!(bytemuck::bytes_of(&legacy.instances[0]), bytemuck::bytes_of(&shared.instance(binding(0,0), &source).unwrap()));
        }
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
            assert_eq!(cell.normals[0], [1.0, 0.0, 0.0, 0.5]);
            assert_eq!(cell.normals[1], [-1.0, 0.0, 0.0, 0.5]);
            assert_eq!(cell.normals[2..], [[0.0; 4]; 4]);
        }
    }

    #[test]
    fn packed_actual_normal_means_preserve_density_colour_and_leaf_fraction() {
        let mut source = voxel([0; 3], 0.17);
        source.leaf_fraction = 0.75;
        // A dispersed lobe has a mean length below one. Normalizing it at
        // upload would change the first moment used by spatial interpolation.
        source.normal_centroids[0] = [0.4, 0.3, 0.0];
        source.normal_centroids[1] = [-0.4, -0.3, 0.0];
        let packed = pack_cell(&source, 0.25).unwrap();
        assert_eq!(packed.albedo_rough, [0.2, 0.4, 0.1, 0.8]);
        assert_eq!(packed.normals[0], [0.4, 0.3, 0.0, 0.5]);
        assert_eq!(packed.normals[1], [-0.4, -0.3, 0.0, 0.5]);
        assert_eq!(packed.normals[2..], [[0.0; 4]; 4]);
        for k in 0..13 {
            assert_eq!(packed.extinction[k / 4][k % 4].to_bits(),
                (source.directional_area[k] / 0.25f32.powi(3)).to_bits());
        }
        assert_eq!(packed.extinction[3][1..], [0.75, 0.0, 0.0]);
        for invalid in [[0.0; 3], [f32::NAN, 0.0, 0.0], [2.0, 0.0, 0.0]] {
            let mut invalid_source = source.clone();
            invalid_source.normal_centroids[0] = invalid;
            assert!(pack_cell(&invalid_source, 0.25).is_err());
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

    fn brick_fixture() -> GpuForest {
        let mut wide=asset();wide.origin=[-2.0,3.0,1.5];
        wide.levels=vec![
            VoxelLevel {cell_size:0.25,dimensions:[9,6,5],voxels:
                [[0,0,0],[3,3,3],[4,0,0],[8,5,4],[1,4,4]].into_iter()
                    .enumerate().map(|(i,c)|voxel(c,0.01*(i+1)as f32)).collect()},
            VoxelLevel {cell_size:0.5,dimensions:[5,3,3],voxels:
                vec![voxel([0,0,0],0.07),voxel([4,2,2],0.08)]},
            VoxelLevel {cell_size:1.0,dimensions:[3,2,2],voxels:vec![]},
        ];
        let instances=[instance(Affine3A::IDENTITY),instance(Affine3A::from_scale_rotation_translation(
            Vec3::new(1.2,0.8,1.7),Quat::from_rotation_y(0.3),Vec3::new(7.0,2.0,-4.0)))];
        pack(&[asset(),wide],&[binding(0,0),binding(1,1)],&instances,PackLimits::default()).unwrap()
    }

    fn assert_same_forest(a:&GpuForest,b:&GpuForest) {
        assert_eq!(bytemuck::cast_slice::<_,u8>(&a.levels),bytemuck::cast_slice::<_,u8>(&b.levels));
        assert_eq!(a.indices,b.indices);
        assert_eq!(bytemuck::cast_slice::<_,u8>(&a.cells),bytemuck::cast_slice::<_,u8>(&b.cells));
        assert_eq!(bytemuck::cast_slice::<_,u8>(&a.instances),bytemuck::cast_slice::<_,u8>(&b.instances));
    }

    #[test]
    fn appended_dense_forest_keeps_every_cell_after_a_bricked_one() {
        let mut forest=brick_fixture();pack_bricks(&mut forest,128).unwrap();
        let before=forest.clone();let other=brick_fixture();
        forest.append(other.clone()).unwrap();
        assert_eq!(forest.instances.len(),before.instances.len()+other.instances.len());
        for (i,tree) in other.instances.iter().enumerate() {
            let moved=forest.instances[before.instances.len()+i];
            assert_eq!(moved.meta[1],tree.meta[1]+before.levels.len() as u32);
            for l in 0..tree.meta[2] {
                let (a,b)=(&other.levels[(tree.meta[1]+l)as usize],&forest.levels[(moved.meta[1]+l)as usize]);
                let [x,y,z]=[a.dims_offset[0],a.dims_offset[1],a.dims_offset[2]];
                for c in (0..x).flat_map(|i|(0..y).flat_map(move|j|(0..z).map(move|k|[i,j,k]))) {
                    let (old,new)=(cell_index(&other,a,c).unwrap(),cell_index(&forest,b,c).unwrap());
                    assert_eq!(old==0,new==0);
                    if old!=0 {assert_eq!(bytemuck::bytes_of(&other.cells[old as usize-1]),bytemuck::bytes_of(&forest.cells[new as usize-1]));}
                }
            }
        }
        // The bricked part is untouched; bricked input cannot be appended.
        assert_eq!(bytemuck::cast_slice::<_,u8>(&forest.levels[..before.levels.len()]),bytemuck::cast_slice::<_,u8>(&before.levels));
        assert!(forest.clone().append(before).is_err());
    }

    #[test]
    fn pack_bricks_roundtrip_preserves_every_cell_across_assets_mips_and_partial_edges() {
        let dense=brick_fixture();let mut sparse=dense.clone();
        let counts=pack_bricks(&mut sparse,128).unwrap();
        assert_eq!(counts,[1,1,4,2,0]);
        assert_eq!(bytemuck::cast_slice::<_,u8>(&dense.cells),bytemuck::cast_slice::<_,u8>(&sparse.cells));
        assert_eq!(bytemuck::cast_slice::<_,u8>(&dense.instances),bytemuck::cast_slice::<_,u8>(&sparse.instances));
        for ((original,packed),&occupied) in dense.levels.iter().zip(&sparse.levels).zip(&counts) {
            assert_eq!(original.origin_cell,packed.origin_cell);
            assert_eq!(original.dims_offset[..3],packed.dims_offset[..3]);
            assert_ne!(packed.dims_offset[3]&BRICK_INDIRECTION,0);
            let [nx,ny,nz,old_base]=original.dims_offset;
            let base=(packed.dims_offset[3]&!BRICK_INDIRECTION)as usize;
            let brick_dims=[nx.div_ceil(4),ny.div_ceil(4),nz.div_ceil(4)];
            let directory_len=brick_dims.into_iter().product::<u32>()as usize;
            assert_eq!(sparse.indices[base..base+directory_len].iter().filter(|&&p|p!=0).count(),occupied);
            // Compare against the original dense addressing, independently of
            // the production lookup. Include zero cells and incomplete bricks.
            for z in 0..nz {for y in 0..ny {for x in 0..nx {
                let expected=dense.indices[(old_base+x+nx*(y+ny*z))as usize];
                let pointer=sparse.indices[base+(x/4+brick_dims[0]*(y/4+brick_dims[1]*(z/4)))as usize];
                let actual=if pointer==0 {0} else {
                    sparse.indices[(pointer-1+x%4+4*(y%4)+16*(z%4))as usize]
                };
                assert_eq!(actual,expected,"cell {x},{y},{z}");
                assert_eq!(cell_index(&sparse,packed,[x,y,z]),Some(expected));
            }}}
            for c in [[nx,0,0],[0,ny,0],[0,0,nz]] {
                assert_eq!(cell_index(&sparse,packed,c),Some(0));
            }
        }
        let once=sparse.clone();
        assert_eq!(pack_bricks(&mut sparse,128).unwrap(),counts);
        assert_same_forest(&once,&sparse);
    }

    #[test]
    fn pack_bricks_failure_rolls_back_completed_levels_and_all_original_payloads() {
        let original=brick_fixture();
        for cap in [0,1,MAX_PACKED_BRICKS_PER_LEVEL+1] {
            let mut forest=original.clone();
            assert!(pack_bricks(&mut forest,cap).is_err());
            assert_same_forest(&original,&forest);
        }
        // Bad data in the final level is reached after earlier temporary
        // levels have already been built; none of them may be committed.
        for bad in 0..4 {
            let mut forest=original.clone();
            let last=forest.levels.last_mut().unwrap();
            match bad {
                0=>last.dims_offset[0]=0,
                1=>last.dims_offset=[1_000_000,1_000_000,1_000_000,0],
                2=>last.dims_offset[3]=forest.indices.len()as u32,
                _=>forest.indices[last.dims_offset[3]as usize]=forest.cells.len()as u32+1,
            }
            let before=forest.clone();
            assert!(pack_bricks(&mut forest,128).is_err());
            assert_same_forest(&before,&forest);
        }
        let mut forest=original;pack_bricks(&mut forest,128).unwrap();
        let level=&forest.levels[2];let base=(level.dims_offset[3]&!BRICK_INDIRECTION)as usize;
        forest.indices[base]=u32::MAX;
        let before=forest.clone();
        assert!(pack_bricks(&mut forest,128).is_err());
        assert_same_forest(&before,&forest);
    }

    #[test]
    fn pack_bricks_accepts_exactly_128_occupied_bricks_and_rejects_129_transactionally() {
        let build=|bricks:u32| {
            let mut indices=vec![0;bricks as usize*4];
            for i in 0..bricks {indices[i as usize*4]=1;}
            GpuForest {levels:vec![GpuLevel {origin_cell:[0.0,0.0,0.0,0.25],dims_offset:[bricks*4,1,1,0]}],
                indices,cells:vec![pack_cell(&voxel([0;3],0.03),0.25).unwrap()],instances:vec![]}
        };
        let mut valid=build(128);
        assert_eq!(pack_bricks(&mut valid,128).unwrap(),[128]);
        assert_eq!(valid.indices.len(),128+128*64);
        for x in 0..512 {assert_eq!(cell_index(&valid,&valid.levels[0],[x,0,0]),Some(u32::from(x%4==0)));}
        let mut invalid=build(129);let before=invalid.clone();
        assert!(pack_bricks(&mut invalid,128).is_err());
        assert_same_forest(&before,&invalid);
    }
}

#[cfg(test)]
mod packed_cell_tests {
    use super::*;
    #[test]
    #[ignore="exclusive GPU packed-cell decoding"]
    fn foliage_packed_gpu_matches_cpu_optical_records() {
        use wgpu::util::DeviceExt;
        let gpu=crate::Gpu::new().unwrap();
        let cells=(0..257).map(|i|{let v=i as f32/128.0-1.0;let mut c=GpuCell::zeroed();
            c.albedo_rough=[0.11,0.75,0.31,0.9];c.extinction=[[1e-15,1e8,0.0,7.5];4];
            c.normals=std::array::from_fn(|n|[v,-v,v*0.31,(n+1)as f32/21.0]);c}).collect::<Vec<_>>();
        let packed=packed_cells(&cells);
        let input=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("packed optics test"),contents:bytemuck::cast_slice(&packed),usage:wgpu::BufferUsages::STORAGE});
        let size=(cells.len()*std::mem::size_of::<GpuCell>())as u64;
        let output=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let common=include_str!("shaders/foliage_volume.wgsl").split("// This specializes only").next().unwrap();
        let source=format!("{common}\n@group(0) @binding(0) var<storage,read> src:array<PackedVolumeCell>;\n@group(0) @binding(1) var<storage,read_write> dst:array<VolumeCell>;\n@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {{if id.x<arrayLength(&src) {{dst[id.x]=unpack_volume_cell(src[id.x]);}}}}");
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:None,source:wgpu::ShaderSource::Wgsl(source.into())});
        let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:None,layout:None,module:&module,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
            wgpu::BindGroupEntry {binding:0,resource:input.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:output.as_entire_binding()}]});
        let mut enc=gpu.device.create_command_encoder(&Default::default());
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(5,1,1);}
        enc.copy_buffer_to_buffer(&output,0,&read,0,size);gpu.queue.submit([enc.finish()]);read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        let bytes=read.slice(..).get_mapped_range().unwrap();
        for (raw,expected) in bytes.chunks_exact(176).zip(&packed) {
            let got:GpuCell=bytemuck::pod_read_unaligned(raw);let expected=expected.decode();
            for (a,b) in got.albedo_rough.iter().zip(&expected.albedo_rough){assert!((a-b).abs()<1e-7,"{a} != {b}");}
            assert_eq!(got.extinction,expected.extinction);
            for (a,b) in got.normals.iter().flatten().zip(expected.normals.iter().flatten()){assert!((a-b).abs()<1e-7,"{a} != {b}");}
        }
        drop(bytes);read.unmap();
    }
    #[test]
    fn foliage_packed_moments_preserve_direction_length_and_optics() {
        assert_eq!(std::mem::size_of::<PackedCell>(),64);
        let mut c=GpuCell::zeroed();c.albedo_rough=[0.11,0.75,0.31,0.9];
        // Tiny and huge densities stay occupied and finite; ordinary ones keep 11 bits.
        c.extinction=[[1e-15,1e8,0.0,7.5],[0.013,431.7,3e-6,65.],[0.5,1.,2.,1e-4],[19.3,0.75,0.,0.]];
        let d=PackedCell::from(&c).decode();
        for (a,b) in c.extinction.iter().flatten().zip(d.extinction.iter().flatten()) {
            assert_eq!(*a==0.,*b==0.,"{a} -> {b}");assert!(b.is_finite());
            if (1e-4..6e4).contains(a) {assert!((a-b).abs()<=a/2048.,"{a} -> {b}");}
        }
        assert_eq!(half_value(half(1e8)),65504.);assert_eq!(half(1e-15),1);assert_eq!(half(1.),0x3c00);assert_eq!(half(65519.),0x7bff);
        for sample in 0..4097 {
            let v=-1.0+sample as f32/2048.0;
            c.normals=std::array::from_fn(|i|[v,-v,v*0.31,(i as f32+1.0)/21.0]);
            let d=PackedCell::from(&c).decode();
            for (a,b) in c.albedo_rough.iter().zip(&d.albedo_rough) {assert!((a-b).abs()<=0.5/65535.+1e-7,"{a} -> {b}");}
            for (a,b) in c.normals.iter().flatten().zip(d.normals.iter().flatten()) {
                assert!((a-b).abs()<=1.0/254.0+1e-6,"{a} -> {b}");
            }
        }
        c.normals[0]=[0.,-1.,1.,0.];assert_eq!(PackedCell::from(&c).decode().normals[0],c.normals[0]);
    }
}
