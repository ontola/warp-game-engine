//! Camera-independent spatial index for source-derived foliage instances.
//!
//! Assets are shared, IDs are stable, and world-grid cells contain sorted
//! owner references rather than copies of asset data. Closed conservative
//! bounds include caller-selected reconstruction support and a caller-supplied
//! maximum wind displacement. The CPU ray oracle visits disjoint half-open
//! grid segments; an instance referenced in several cells is integrated only
//! inside each segment, never several times over its full ray interval.
//!
//! This module allocates no GPU resources and changes no rendering defaults.
//! Its POD records can be uploaded unchanged. Cells are lexicographically
//! sorted for a binary search; a future GPU grid/hash builder can emit the
//! same records. This index makes no optical-model assumption: source-sheet
//! coverage, a participating medium and geometry can share the same bounds.
//! The renderer must distinguish their transport rules. Surface coverage is
//! applied once at the owning hit, not repeatedly in every intersected cell.

use crate::foliage_voxel::VoxelAsset;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::mem::size_of;
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub struct SourceInstance {
    /// Stable application identity, independent of input order or pool slot.
    pub id: u64,
    /// Increment when an ID is reused or a stage epoch changes. Motion history
    /// must match BOTH id and generation before reusing correspondence.
    pub generation: u32,
    pub asset: u32,
    pub rest: Mat4,
    /// Maximum world-space displacement over the supported wind range, not
    /// just the current phase. A larger future range requires a rebuild.
    pub wind_margin: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct GridConfig {
    pub origin: [f32; 3],
    pub cell_size: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_assets: usize,
    pub max_instances: usize,
    pub max_cells: usize,
    pub max_references: usize,
    pub max_references_per_instance: usize,
    pub max_references_per_cell: usize,
    /// Conservative peak element-storage budget during build, including
    /// sorted input pointers, owner spans and temporary cell/owner pairs.
    /// Allocator bookkeeping and caller-owned source assets are excluded.
    pub max_build_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_assets: 64,
            max_instances: 262_144,
            max_cells: 65_536,
            max_references: 2_097_152,
            max_references_per_instance: 4096,
            max_references_per_cell: 4096,
            max_build_bytes: 128 * 1024 * 1024,
        }
    }
}

/// Four vec4 records, 64 bytes. Bounds are local; no per-instance asset copy.
/// Sample bounds include any reconstruction/filter support for every selected
/// representation. They must contain the source bounds. Wind is added after
/// transforming these bounds to world space. Fourth components are reserved.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct AssetBounds {
    pub source_min: [f32; 4],
    pub source_max: [f32; 4],
    pub sample_min: [f32; 4],
    pub sample_max: [f32; 4],
}

/// Nine vec4 records, 144 bytes. Matrices are three affine ROWS.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    pub rest: [[f32; 4]; 3],
    pub inverse: [[f32; 4]; 3],
    pub world_min: [f32; 4],
    pub world_max: [f32; 4],
    /// Stable id low/high words, asset, generation.
    pub identity: [u32; 4],
}
impl Instance {
    pub fn id(&self) -> u64 {
        self.identity[0] as u64 | ((self.identity[1] as u64) << 32)
    }
}

/// Two vec4 records, 32 bytes. Negative world cells remain signed.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Cell {
    pub coord: [i32; 4],
    /// First owner-reference offset, count, reserved, reserved.
    pub span: [u32; 4],
}
impl Cell {
    fn key(&self) -> [i32; 3] {
        [self.coord[0], self.coord[1], self.coord[2]]
    }
}

#[derive(Clone, Debug)]
pub struct Groups {
    pub config: GridConfig,
    pub assets: Vec<AssetBounds>,
    /// Sorted by stable ID, so input reordering cannot alter GPU owner order.
    pub instances: Vec<Instance>,
    pub cells: Vec<Cell>,
    /// Indices into instances, sorted and unique within each cell.
    pub owners: Vec<u32>,
    pub planned_build_bytes: usize,
    limits: Limits,
    cell_bounds: Option<[[i32; 3]; 2]>,
}

impl AssetBounds {
    /// Generic bounds for geometry, coverage sheets or media. Representation,
    /// mip selection and optical data are owned by the renderer, not this grid.
    pub fn new(source: [[f32; 3]; 2], sample: [[f32; 3]; 2]) -> Result<Self, String> {
        let [source_min, source_max] = source.map(Vec3::from);
        let [sample_min, sample_max] = sample.map(Vec3::from);
        if !source_min.is_finite()
            || !source_max.is_finite()
            || !sample_min.is_finite()
            || !sample_max.is_finite()
            || source_min.cmpgt(source_max).any()
            || sample_min.cmpgt(source_min).any()
            || sample_max.cmplt(source_max).any()
        {
            return Err("invalid foliage source/sample bounds".into());
        }
        Ok(Self {
            source_min: source_min.extend(0.0).to_array(),
            source_max: source_max.extend(0.0).to_array(),
            sample_min: sample_min.extend(0.0).to_array(),
            sample_max: sample_max.extend(0.0).to_array(),
        })
    }

    /// Convenience for existing voxel assets. Include only the rendered mip
    /// range: each selected grid adds half a cell of trilinear filter support.
    /// Occupancy-only levels need not enlarge it. This derives bounds only;
    /// voxel mass/optics validation belongs to the representation's uploader.
    pub fn from_voxels(asset: &VoxelAsset, levels: Range<usize>) -> Result<Self, String> {
        let source_min = Vec3::from(asset.source_bounds[0]);
        let source_max = Vec3::from(asset.source_bounds[1]);
        if !source_min.is_finite()
            || !source_max.is_finite()
            || source_min.cmpgt(source_max).any()
            || !asset.origin.iter().all(|v| v.is_finite())
            || levels.start >= levels.end
            || levels.end > asset.levels.len()
        {
            return Err("foliage asset has invalid source bounds/origin/levels".into());
        }
        let mut lo = source_min;
        let mut hi = source_max;
        let mut previous = 0.0;
        for level in &asset.levels[levels] {
            if !level.cell_size.is_finite()
                || level.cell_size <= previous
                || level.dimensions.contains(&0)
            {
                return Err("foliage asset has invalid selected mip dimensions/cell size".into());
            }
            let b = level.local_bounds(asset.origin);
            let support = Vec3::splat(level.cell_size * 0.5);
            lo = lo.min(Vec3::from(b[0]) - support);
            hi = hi.max(Vec3::from(b[1]) + support);
            previous = level.cell_size;
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Err("foliage asset sample bounds overflow".into());
        }
        Self::new(asset.source_bounds, [lo.to_array(), hi.to_array()])
    }
}

fn rows(m: Mat4) -> [[f32; 4]; 3] {
    let m = m.transpose();
    [
        m.x_axis.to_array(),
        m.y_axis.to_array(),
        m.z_axis.to_array(),
    ]
}
fn instance(source: &SourceInstance, asset: &AssetBounds) -> Result<Instance, String> {
    let m = source.rest;
    if !m.is_finite()
        || m.x_axis.w != 0.0
        || m.y_axis.w != 0.0
        || m.z_axis.w != 0.0
        || m.w_axis.w != 1.0
        || !source.wind_margin.is_finite()
        || source.wind_margin < 0.0
    {
        return Err("invalid foliage affine transform/wind margin".into());
    }
    let axes = [
        m.x_axis.truncate(),
        m.y_axis.truncate(),
        m.z_axis.truncate(),
    ];
    let lengths = axes.map(Vec3::length);
    if lengths.iter().any(|v| !v.is_finite() || *v < 1e-6)
        || (axes[0] / lengths[0])
            .dot((axes[1] / lengths[1]).cross(axes[2] / lengths[2]))
            .abs()
            < 1e-6
    {
        return Err("singular foliage affine transform".into());
    }
    let inverse = m.inverse();
    if !inverse.is_finite() {
        return Err("foliage inverse transform overflow".into());
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let p = Vec3::from_array(std::array::from_fn(|k| {
            if corner & (1 << k) == 0 {
                asset.sample_min[k]
            } else {
                asset.sample_max[k]
            }
        }));
        let world = m.transform_point3(p);
        lo = lo.min(world);
        hi = hi.max(world);
    }
    lo -= Vec3::splat(source.wind_margin);
    hi += Vec3::splat(source.wind_margin);
    // Keep conservative support after f32 matrix/bounds rounding.
    lo = Vec3::from_array(lo.to_array().map(f32::next_down));
    hi = Vec3::from_array(hi.to_array().map(f32::next_up));
    if !lo.is_finite() || !hi.is_finite() {
        return Err("foliage world bounds overflow".into());
    }
    Ok(Instance {
        rest: rows(m),
        inverse: rows(inverse),
        world_min: lo.extend(0.0).to_array(),
        world_max: hi.extend(0.0).to_array(),
        identity: [
            source.id as u32,
            (source.id >> 32) as u32,
            source.asset,
            source.generation,
        ],
    })
}

fn cell_coordinate(value: f64, origin: f32, size: f32) -> Result<i32, String> {
    let q = ((value - origin as f64) / size as f64).floor();
    // Reserve an endpoint on each side for DDA face arithmetic.
    if !q.is_finite() || q <= i32::MIN as f64 || q >= i32::MAX as f64 {
        return Err("foliage grid coordinate overflow".into());
    }
    Ok(q as i32)
}
fn checked_bytes(parts: &[(usize, usize)]) -> Result<usize, String> {
    parts
        .iter()
        .try_fold(0usize, |n, (count, size)| {
            n.checked_add(count.checked_mul(*size)?)
        })
        .ok_or("foliage memory size overflow".into())
}

impl Groups {
    pub fn build(
        assets: &[AssetBounds],
        sources: &[SourceInstance],
        config: GridConfig,
        limits: Limits,
    ) -> Result<Self, String> {
        if !config.origin.iter().all(|v| v.is_finite())
            || !config.cell_size.is_finite()
            || config.cell_size <= 0.0
        {
            return Err("invalid foliage grid configuration".into());
        }
        if assets.len() > limits.max_assets
            || sources.len() > limits.max_instances
            || sources.len() > u32::MAX as usize
            || assets.len() > u32::MAX as usize
        {
            return Err("foliage asset/instance budget exceeded".into());
        }
        let initial_bytes = checked_bytes(&[
            (assets.len(), size_of::<AssetBounds>()),
            (
                sources.len(),
                size_of::<Instance>() + size_of::<&SourceInstance>() + 24,
            ),
        ])?;
        if initial_bytes > limits.max_build_bytes {
            return Err("foliage build memory budget exceeded".into());
        }
        let mut asset_records = Vec::with_capacity(assets.len());
        for a in assets {
            asset_records.push(AssetBounds::new(
                [
                    std::array::from_fn(|k| a.source_min[k]),
                    std::array::from_fn(|k| a.source_max[k]),
                ],
                [
                    std::array::from_fn(|k| a.sample_min[k]),
                    std::array::from_fn(|k| a.sample_max[k]),
                ],
            )?);
        }
        let mut ordered: Vec<_> = sources.iter().collect();
        ordered.sort_unstable_by_key(|s| s.id);
        if ordered.windows(2).any(|p| p[0].id == p[1].id) {
            return Err("duplicate foliage stable ID would double density".into());
        }
        let mut instances = Vec::with_capacity(sources.len());
        let mut spans = Vec::with_capacity(sources.len());
        let mut total = 0usize;
        for source in ordered {
            let asset = asset_records
                .get(source.asset as usize)
                .ok_or("foliage instance asset is out of bounds")?;
            let packed = instance(source, asset)?;
            let mut lo = [0; 3];
            let mut hi = [0; 3];
            let mut count = 1usize;
            for k in 0..3 {
                lo[k] = cell_coordinate(
                    packed.world_min[k] as f64,
                    config.origin[k],
                    config.cell_size,
                )?;
                hi[k] = cell_coordinate(
                    packed.world_max[k] as f64,
                    config.origin[k],
                    config.cell_size,
                )?;
                count = count
                    .checked_mul((hi[k] as i64 - lo[k] as i64 + 1) as usize)
                    .ok_or("foliage cell span overflow")?;
            }
            if count > limits.max_references_per_instance {
                return Err("foliage per-instance reference budget exceeded".into());
            }
            total = total
                .checked_add(count)
                .ok_or("foliage reference count overflow")?;
            if total > limits.max_references || total > u32::MAX as usize {
                return Err("foliage reference budget exceeded".into());
            }
            instances.push(packed);
            spans.push((lo, hi));
        }
        let planned_build_bytes = checked_bytes(&[
            (1, initial_bytes),
            (total, 16 + size_of::<u32>()),
            (total.min(limits.max_cells), size_of::<Cell>()),
        ])?;
        if planned_build_bytes > limits.max_build_bytes {
            return Err("foliage build memory budget exceeded".into());
        }
        let mut references = Vec::with_capacity(total);
        for (owner, (lo, hi)) in spans.into_iter().enumerate() {
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        references.push(([x, y, z], owner as u32));
                    }
                }
            }
        }
        references.sort_unstable();
        let mut cells = Vec::with_capacity(total.min(limits.max_cells));
        let mut owners = Vec::with_capacity(total);
        for (coord, owner) in references {
            if cells.last().is_none_or(|c: &Cell| c.key() != coord) {
                if cells.len() >= limits.max_cells {
                    return Err("foliage occupied-cell budget exceeded".into());
                }
                cells.push(Cell {
                    coord: [coord[0], coord[1], coord[2], 0],
                    span: [owners.len() as u32, 0, 0, 0],
                });
            }
            let cell = cells.last_mut().unwrap();
            if cell.span[1] as usize >= limits.max_references_per_cell {
                return Err("foliage per-cell reference budget exceeded".into());
            }
            cell.span[1] += 1;
            owners.push(owner);
        }
        let cell_bounds = (!cells.is_empty()).then(|| {
            let mut lo = [i32::MAX; 3];
            let mut hi = [i32::MIN; 3];
            for cell in &cells {
                for k in 0..3 {
                    lo[k] = lo[k].min(cell.coord[k]);
                    hi[k] = hi[k].max(cell.coord[k]);
                }
            }
            [lo, hi]
        });
        Ok(Self {
            config,
            assets: asset_records,
            instances,
            cells,
            owners,
            planned_build_bytes,
            limits,
            cell_bounds,
        })
    }

    /// Transactional CPU rebuild for changed instances/assets. Failure leaves
    /// the previous immutable index intact. IDs, not the resulting row offsets,
    /// are the temporal correspondence contract; all cameras share one index.
    /// During replacement, old storage remains in addition to max_build_bytes.
    pub fn rebuild(
        &mut self,
        assets: &[AssetBounds],
        sources: &[SourceInstance],
    ) -> Result<(), String> {
        let replacement = Self::build(assets, sources, self.config, self.limits)?;
        *self = replacement;
        Ok(())
    }
    pub fn byte_len(&self) -> usize {
        self.assets.len() * size_of::<AssetBounds>()
            + self.instances.len() * size_of::<Instance>()
            + self.cells.len() * size_of::<Cell>()
            + self.owners.len() * size_of::<u32>()
    }
    pub fn cell_owners(&self, coord: [i32; 3]) -> &[u32] {
        let Ok(index) = self.cells.binary_search_by_key(&coord, Cell::key) else {
            return &[];
        };
        let span = self.cells[index].span;
        &self.owners[span[0] as usize..(span[0] + span[1]) as usize]
    }

    /// CPU traversal oracle. Ray direction need not be normalized; t uses its
    /// parameterization. Nonempty segments are emitted in increasing t and do
    /// not overlap. Intersect each referenced medium with the segment BEFORE
    /// integrating; accept surface hits only when Segment::contains returns
    /// true. An error invalidates previously emitted partial results.
    /// max_steps counts empty cells too; overflow is never silent truncation.
    pub fn trace(
        &self,
        ray: Ray,
        max_steps: usize,
        mut visit: impl FnMut(Segment<'_>),
    ) -> Result<usize, String> {
        if !ray
            .origin
            .iter()
            .chain(&ray.direction)
            .all(|v| v.is_finite())
            || ray.direction == [0.0; 3]
            || !ray.range.iter().all(|v| v.is_finite())
            || ray.range[0] < 0.0
            || ray.range[1] < ray.range[0]
        {
            return Err("invalid foliage trace ray".into());
        }
        if self.cells.is_empty() || ray.range[0] == ray.range[1] {
            return Ok(0);
        }
        // The scene union is tiny metadata, not a per-ray owner scan. Cell
        // extrema also bound empty gaps; a future hierarchy can skip them.
        let [low, high] = self.cell_bounds.unwrap();
        let lo = std::array::from_fn(|k| {
            self.config.origin[k] as f64 + low[k] as f64 * self.config.cell_size as f64
        });
        let hi = std::array::from_fn(|k| {
            self.config.origin[k] as f64 + (high[k] as f64 + 1.0) * self.config.cell_size as f64
        });
        let Some(span) = ray.box_interval(lo, hi, ray.range) else {
            return Ok(0);
        };
        let mut t = span[0];
        let end = span[1];
        let mut coord = [0; 3];
        for k in 0..3 {
            let q = (ray.origin[k] + ray.direction[k] * t - self.config.origin[k] as f64)
                / self.config.cell_size as f64;
            coord[k] = q.floor() as i32;
            if ray.direction[k] < 0.0 && q == q.floor() {
                coord[k] -= 1;
            }
        }
        let mut steps = 0;
        while t < end {
            if steps >= max_steps {
                return Err("foliage ray cell budget exceeded; partial integral invalid".into());
            }
            steps += 1;
            let next: [f64; 3] = std::array::from_fn(|k| {
                if ray.direction[k] == 0.0 {
                    return f64::INFINITY;
                }
                let face = coord[k] as f64 + if ray.direction[k] > 0.0 { 1.0 } else { 0.0 };
                (self.config.origin[k] as f64 + face * self.config.cell_size as f64 - ray.origin[k])
                    / ray.direction[k]
            });
            let exit = next.into_iter().fold(end, f64::min);
            if exit < t || !exit.is_finite() {
                return Err("foliage DDA lost numeric progress".into());
            }
            let owners = self.cell_owners(coord);
            if exit > t && !owners.is_empty() {
                visit(Segment {
                    cell: coord,
                    range: [t, exit],
                    owners,
                });
            }
            if exit >= end {
                break;
            }
            for k in 0..3 {
                if next[k] == exit {
                    coord[k] += if ray.direction[k] > 0.0 { 1 } else { -1 };
                }
            }
            t = exit;
        }
        Ok(steps)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
    pub range: [f64; 2],
}
impl Ray {
    /// Analytical slab clipping for geometric interval tests. This validates
    /// traversal, not the asset representation's optical transport model.
    pub fn box_interval(self, lo: [f64; 3], hi: [f64; 3], mut range: [f64; 2]) -> Option<[f64; 2]> {
        for k in 0..3 {
            if self.direction[k] == 0.0 {
                if self.origin[k] < lo[k] || self.origin[k] > hi[k] {
                    return None;
                }
            } else {
                let a = (lo[k] - self.origin[k]) / self.direction[k];
                let b = (hi[k] - self.origin[k]) / self.direction[k];
                range[0] = range[0].max(a.min(b));
                range[1] = range[1].min(a.max(b));
            }
        }
        (range[1] > range[0]).then_some(range)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Segment<'a> {
    pub cell: [i32; 3],
    pub range: [f64; 2],
    pub owners: &'a [u32],
}
impl Segment<'_> {
    /// Half-open ownership prevents a surface on a shared cell boundary from
    /// contributing twice. The trace range's terminal endpoint is excluded.
    pub fn contains(&self, t: f64) -> bool {
        t >= self.range[0] && t < self.range[1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_voxel::{BakeStats, VoxelLevel};

    fn asset() -> AssetBounds {
        AssetBounds::from_voxels(
            &VoxelAsset {
                projection_measure: Default::default(),
                origin: [0.0; 3],
                source_bounds: [[0.0; 3], [1.0; 3]],
                levels: vec![
                    VoxelLevel {
                        cell_size: 0.5,
                        dimensions: [2; 3],
                        voxels: vec![],
                    },
                    VoxelLevel {
                        cell_size: 1.0,
                        dimensions: [1; 3],
                        voxels: vec![],
                    },
                ],
                stats: BakeStats::default(),
            },
            0..2,
        )
        .unwrap()
    }
    fn config() -> GridConfig {
        GridConfig {
            origin: [0.0; 3],
            cell_size: 1.0,
        }
    }
    fn source(id: u64, p: Vec3) -> SourceInstance {
        SourceInstance {
            id,
            generation: 7,
            asset: 0,
            rest: Mat4::from_translation(p),
            wind_margin: 0.15,
        }
    }
    fn integral(index: &Groups, ray: Ray) -> (Vec<f64>, usize) {
        let mut sums = vec![0.0; index.instances.len()];
        let mut previous = ray.range[0];
        let steps = index
            .trace(ray, 10000, |segment| {
                assert!(segment.range[0] >= previous && segment.range[1] > segment.range[0]);
                previous = segment.range[1];
                assert!(
                    segment.owners.windows(2).all(|p| p[0] < p[1]),
                    "same owner cannot occur twice in a segment"
                );
                for &owner in segment.owners {
                    let instance = &index.instances[owner as usize];
                    if let Some(span) = ray.box_interval(
                        std::array::from_fn(|k| instance.world_min[k] as f64),
                        std::array::from_fn(|k| instance.world_max[k] as f64),
                        segment.range,
                    ) {
                        sums[owner as usize] += span[1] - span[0];
                    }
                }
            })
            .unwrap();
        for (sum, instance) in sums.iter().zip(&index.instances) {
            let expected = ray
                .box_interval(
                    std::array::from_fn(|k| instance.world_min[k] as f64),
                    std::array::from_fn(|k| instance.world_max[k] as f64),
                    ray.range,
                )
                .map_or(0.0, |s| s[1] - s[0]);
            assert!(
                (sum - expected).abs() < 1e-9,
                "optical interval lost or counted twice: {sum} vs {expected}, {ray:?}"
            );
        }
        (sums, steps)
    }

    #[test]
    fn dda_matches_analytic_integrals_across_negative_cells_faces_edges_and_corners() {
        let sources = [
            source(19, Vec3::new(-3.2, -1.0, 0.0)),
            source(3, Vec3::new(-0.5, 0.3, -1.0)),
            source(11, Vec3::new(2.2, -0.4, 1.5)),
        ];
        let index = Groups::build(&[asset()], &sources, config(), Limits::default()).unwrap();
        for direction in [
            [1., 0., 0.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
            [1., 1., 0.],
            [-1., -1., 0.],
            [1., 1., 1.],
            [-1., -1., -1.],
        ] {
            for x in -6..=6 {
                for y in -2..=3 {
                    integral(
                        &index,
                        Ray {
                            origin: [x as f64, y as f64, 0.0],
                            direction,
                            range: [0.0, 20.0],
                        },
                    );
                }
            }
        }
        // Deterministic non-grid rays, translated grid origin and partial t.
        let mut cfg = config();
        cfg.origin = [0.37, -2.17, 4.83];
        cfg.cell_size = 1.7;
        let index = Groups::build(&[asset()], &sources, cfg, Limits::default()).unwrap();
        let mut state = 9u64;
        let mut random = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 32) as f64 / u32::MAX as f64
        };
        for _ in 0..400 {
            let origin = std::array::from_fn(|_| random() * 12.0 - 6.0);
            let direction = std::array::from_fn(|_| random() * 2.0 - 1.0);
            integral(
                &index,
                Ray {
                    origin,
                    direction,
                    range: [0.13, 18.7],
                },
            );
        }
    }

    #[test]
    fn shared_ids_bounds_and_transforms_are_deterministic_for_all_views_and_updates() {
        let mut a = source(91, Vec3::new(-0.3, 2.1, 0.8));
        a.rest = Mat4::from_scale_rotation_translation(
            Vec3::new(0.4, 2.0, 1.7),
            glam::Quat::from_rotation_y(0.71),
            Vec3::new(-0.3, 2.1, 0.8),
        );
        let b = source(4, Vec3::new(5.0, 0.0, 0.0));
        let assets = [asset()];
        let mut first = Groups::build(&assets, &[a, b], config(), Limits::default()).unwrap();
        let second = Groups::build(&assets, &[b, a], config(), Limits::default()).unwrap();
        assert_eq!(first.instances, second.instances);
        assert_eq!(first.cells, second.cells);
        assert_eq!(first.owners, second.owners);
        assert_eq!(first.assets[0].sample_min, [-0.5, -0.5, -0.5, 0.0]);
        assert_eq!(first.assets[0].sample_max, [1.5, 1.5, 1.5, 0.0]);
        let packed = first.instances.iter().find(|i| i.id() == 91).unwrap();
        for local in [
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(1.5, 1.5, 1.5),
            Vec3::new(0.4, 0.7, 1.2),
        ] {
            let p = a.rest.transform_point3(local);
            let v = p.extend(1.0);
            let reconstructed =
                Vec3::from_array(packed.inverse.map(|row| glam::Vec4::from_array(row).dot(v)));
            assert!(reconstructed.distance(local) < 1e-5);
            for k in 0..3 {
                assert!(
                    p[k] - a.wind_margin >= packed.world_min[k]
                        && p[k] + a.wind_margin <= packed.world_max[k]
                );
            }
        }
        let bytes = bytemuck::cast_slice::<Instance, u8>(&first.instances).to_vec();
        for origin in [[-8., 1., 0.], [8., 1., 0.], [0., -8., 0.], [0., 1., -8.]] {
            integral(
                &first,
                Ray {
                    origin,
                    direction: [0.37, 0.19, 1.0],
                    range: [0.0, 30.0],
                },
            );
        }
        assert_eq!(
            bytes,
            bytemuck::cast_slice::<Instance, u8>(&first.instances),
            "four views cannot mutate placement"
        );
        let old_cells = first.cells.clone();
        a.rest.w_axis.x += 14.0;
        a.generation += 1;
        first.rebuild(&assets, &[a, b]).unwrap();
        assert_ne!(old_cells, first.cells);
        assert_eq!(
            first
                .instances
                .iter()
                .find(|i| i.id() == 91)
                .unwrap()
                .identity[3],
            8
        );
        integral(
            &first,
            Ray {
                origin: [-8., 2., 1.],
                direction: [1., 0., 0.],
                range: [0.0, 30.0],
            },
        );
        let before = first.instances.clone();
        assert!(first.rebuild(&assets, &[a, a]).is_err());
        assert_eq!(before, first.instances);
    }

    #[test]
    fn all_budgets_fail_explicitly_without_silent_truncation() {
        let sources = [source(1, Vec3::ZERO), source(2, Vec3::ZERO)];
        let assets = [asset()];
        for limits in [
            Limits {
                max_assets: 0,
                ..Default::default()
            },
            Limits {
                max_instances: 1,
                ..Default::default()
            },
            Limits {
                max_references: 1,
                ..Default::default()
            },
            Limits {
                max_references_per_instance: 1,
                ..Default::default()
            },
            Limits {
                max_references_per_cell: 1,
                ..Default::default()
            },
            Limits {
                max_cells: 1,
                ..Default::default()
            },
            Limits {
                max_build_bytes: 1,
                ..Default::default()
            },
        ] {
            assert!(Groups::build(&assets, &sources, config(), limits).is_err());
        }
        let index = Groups::build(&assets, &sources, config(), Limits::default()).unwrap();
        let ray = Ray {
            origin: [-10., 0., 0.],
            direction: [1., 0., 0.],
            range: [0.0, 20.0],
        };
        assert!(index.trace(ray, 1, |_| {}).is_err());
        for rest in [
            Mat4::ZERO,
            Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0)),
            Mat4::from_cols_array(&[f32::NAN; 16]),
        ] {
            let invalid = SourceInstance { rest, ..sources[0] };
            assert!(Groups::build(&assets, &[invalid], config(), Limits::default()).is_err());
        }
        assert!(index.planned_build_bytes >= index.byte_len());
        assert_eq!(
            [
                size_of::<AssetBounds>(),
                size_of::<Instance>(),
                size_of::<Cell>()
            ],
            [64, 144, 32]
        );
        assert!(
            Groups::build(&[], &[], config(), Limits::default())
                .unwrap()
                .trace(ray, 0, |_| panic!())
                .is_ok()
        );
    }

    #[test]
    fn generic_sheet_hits_have_one_owner_at_shared_boundaries() {
        // No voxel asset: a zero-thickness source sheet and a small filter
        // footprint. Its hit lies exactly on a cell boundary in both rays.
        let sheet = AssetBounds::new(
            [[0.0, -1.0, -1.0], [0.0, 1.0, 1.0]],
            [[-0.1, -1.1, -1.1], [0.1, 1.1, 1.1]],
        )
        .unwrap();
        let index = Groups::build(
            &[sheet],
            &[SourceInstance {
                wind_margin: 0.0,
                ..source(u64::MAX, Vec3::ZERO)
            }],
            config(),
            Limits::default(),
        )
        .unwrap();
        assert_eq!(index.instances[0].id(), u64::MAX);
        for direction in [-1.0, 1.0] {
            let ray = Ray {
                origin: [-2.0 * direction, 0.0, 0.0],
                direction: [direction, 0.0, 0.0],
                range: [0.0, 4.0],
            };
            let mut hits = 0;
            index
                .trace(ray, 20, |segment| {
                    if segment.contains(2.0) {
                        assert_eq!(segment.owners, &[0]);
                        hits += 1;
                    }
                })
                .unwrap();
            assert_eq!(hits, 1, "coverage must be applied once at the source hit");
            integral(&index, ray);
        }
    }

    #[test]
    fn wind_expanded_large_trees_need_an_explicit_grid_budget() {
        let tree = AssetBounds::new(
            [[-8.0, 0.0, -8.0], [8.0, 45.0, 8.0]],
            [[-9.0, -1.0, -9.0], [9.0, 46.0, 9.0]],
        )
        .unwrap();
        let moving = SourceInstance {
            wind_margin: 6.0,
            ..source(1, Vec3::new(-0.3, 0.0, 0.2))
        };
        let error = Groups::build(&[tree], &[moving], config(), Limits::default()).unwrap_err();
        assert!(error.contains("per-instance reference budget"), "{error}");
        // Changing the caller's spatial partition is explicit, not silently
        // dropping distant trees or skipping cells outside a small owner pool.
        let index = Groups::build(
            &[tree],
            &[moving],
            GridConfig {
                cell_size: 16.0,
                ..config()
            },
            Limits::default(),
        )
        .unwrap();
        assert!(index.owners.len() < 100);
        for (origin, direction) in [
            ([-40.0, 30.0, 0.0], [1.0, 0.0, 0.0]),
            ([0.0, 70.0, 0.0], [0.0, -1.0, 0.0]),
        ] {
            integral(
                &index,
                Ray {
                    origin,
                    direction,
                    range: [0.0, 100.0],
                },
            );
        }
    }

    #[test]
    fn invalid_bounds_grid_and_instance_data_fail_before_indexing() {
        assert!(AssetBounds::new([[0.0; 3], [1.0; 3]], [[0.0; 3], [0.5; 3]]).is_err());
        assert!(AssetBounds::new([[0.0; 3], [f32::NAN; 3]], [[0.0; 3], [1.0; 3]]).is_err());
        for cfg in [
            GridConfig {
                cell_size: 0.0,
                ..config()
            },
            GridConfig {
                origin: [f32::INFINITY; 3],
                ..config()
            },
        ] {
            assert!(
                Groups::build(&[asset()], &[source(1, Vec3::ZERO)], cfg, Limits::default())
                    .is_err()
            );
        }
        for input in [
            SourceInstance {
                asset: 1,
                ..source(1, Vec3::ZERO)
            },
            SourceInstance {
                wind_margin: -1.0,
                ..source(1, Vec3::ZERO)
            },
            source(1, Vec3::splat(1e30)),
        ] {
            assert!(Groups::build(&[asset()], &[input], config(), Limits::default()).is_err());
        }
    }
}
