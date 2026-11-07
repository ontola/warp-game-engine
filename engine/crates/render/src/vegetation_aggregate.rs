//! Offline, bounded optical aggregates of shared opaque foliage primitives.
//!
//! Only one transformed primitive is materialized at a time. Source geometry
//! remains shared; triangle count is work, never an expanded allocation. A
//! A normal cluster owns placements on one rigid branch pose, so its volume and
//! source can use exactly the same animation. Explicit static subtree cooks
//! instead require the caller to budget discarded descendant motion at runtime.
//! Four-cubed bricks index existing sparse
//! Voxel records; empty slots carry no material or extinction.
//!
//! Diagnostics compare a finer source-derived medium with both a coarser
//! medium and an actual source CLOD cut. They are sampled optical-model errors,
//! NOT exact opaque triangle visibility, Hausdorff bounds or rendered lighting.
use crate::{
    cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex},
    foliage_assembly::Assembly,
    foliage_voxel::{self, BakeStats, ProjectionMeasure, Voxel, VoxelAsset, VoxelLevel, PROJECTION_DIRECTIONS},
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::{DVec3, Mat3, Mat4, Vec3, Vec4};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A cheap deterministic hash for voxel coordinates (lookups only; nothing
/// iterates these maps, so output order never depends on it).
#[derive(Default, Clone, Copy)]
struct CoordHash(u64);
impl std::hash::Hasher for CoordHash {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) { let mut w = [0u8; 8]; w[..chunk.len()].copy_from_slice(chunk); self.write_u64(u64::from_le_bytes(w)); }
    }
    fn write_u32(&mut self, x: u32) { self.write_u64(u64::from(x)); }
    fn write_u64(&mut self, x: u64) { self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7cc1_b727_220a_95); }
    fn write_usize(&mut self, x: usize) { self.write_u64(x as u64); }
}
type CoordMap<V> = HashMap<[u32; 3], V, std::hash::BuildHasherDefault<CoordHash>>;

pub const BRICK_WIDTH: u32 = 4;
pub const EMPTY_CELL: u32 = u32::MAX;
pub const MAX_BRICKS_PER_LEVEL: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub reference_cell_size: f32,
    /// Measure three finer triangle cuts before the first voxel candidate.
    pub triangle_refinements: bool,
    /// Candidate zero retains the reference grid when it fits the brick cap.
    /// Otherwise measured coarsening begins here; it never implies acceptable error.
    pub levels: u32,
    pub samples_per_cell: u32,
    pub diagnostic_resolution: u32,
    /// Estimate errors instead of measuring them with rays: voxel levels
    /// against the reference by projections along the three axes, triangle
    /// cuts from each part's own cut error, measured once per part.
    pub estimated_errors: bool,
    pub max_bricks_per_level: usize,
    pub max_total_bricks: usize,
    pub max_reference_voxels: usize,
    pub max_output_voxels: usize,
    pub max_axis_cells: u32,
    pub max_placements: usize,
    pub max_source_triangles: u64,
    /// Shared by source and all measured triangle alternatives.
    pub max_samples: u64,
    /// Shared by every reference/candidate diagnostic ray traversal.
    pub max_diagnostic_steps: u64,
    pub max_scratch_bytes: usize,
    /// Conservative working-element estimate, including BTree node allowance;
    /// excludes caller-owned Assembly bytes and allocator implementation details.
    pub max_build_bytes: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self { reference_cell_size: 0.075, triangle_refinements: false, levels: 3, samples_per_cell: 1,
            diagnostic_resolution: 16, estimated_errors: false, max_bricks_per_level: 128,
            max_total_bricks: 384, max_reference_voxels: 65_536,
            max_output_voxels: 24_576, max_axis_cells: 1024,
            max_placements: 4096, max_source_triangles: 4_000_000,
            max_samples: 32_000_000, max_diagnostic_steps: 20_000_000,
            max_scratch_bytes: 8 * 1024 * 1024, max_build_bytes: 256 * 1024 * 1024 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Brick {
    /// Brick coordinate, in groups of four cells relative to volume.origin.
    pub coord: [u32; 3],
    pub occupied: u64,
    /// Sparse VoxelLevel index, local slot x + 4*y + 16*z; EMPTY_CELL is empty.
    pub cell_indices: [u32; 64],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectionError {
    pub direction: [f32; 3],
    pub rays: u32,
    pub mean_abs_opacity: f64,
    pub rms_opacity: f64,
    pub max_abs_opacity: f64,
    pub opacity_bias: f64,
    pub premultiplied_albedo_rmse: f64,
    /// Mean absolute difference in integrated six lobe weights and eighteen
    /// conditional-normal first moments. No lighting is baked into this metric.
    pub normal_moment_mean_abs: f64,
    pub sample_spacing_m: [f64; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SampledError {
    pub filtered:[crate::aggregate_lod::FilteredOptics;4],
    pub directions: Vec<DirectionError>,
    pub mean_abs_opacity: f64,
    pub rms_opacity: f64,
    pub max_abs_opacity: f64,
    pub opacity_bias: f64,
    pub premultiplied_albedo_rmse: f64,
    pub normal_moment_mean_abs: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LevelMetadata {
    pub bricks: Vec<Brick>,
    pub sampled_error: SampledError,
    /// Actual CLOD cut evaluated in the SAME fine grid and diagnostic rays.
    pub triangle_sampled_error: SampledError,
    pub triangle_cut_triangles: u64,
    /// Physical cluster-local CLOD threshold; each placement's scale converts
    /// it to primitive-local error. This is not the optical error above.
    pub triangle_cut_error_m: f32,
    /// Relative total directional-area mass drift caused by f32 output storage.
    pub directional_mass_relative_error: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriangleRefinement {
    pub cut_error_m: f32,
    pub triangles: u64,
    pub sampled_error: SampledError,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub source_triangles: u64,
    pub quadrature_samples: u64,
    pub unique_parts_validated: usize,
    pub closed_components_validated: usize,
    pub reference_voxels: usize,
    pub reference_dimensions: [u32; 3],
    pub reference_cell_size: f32,
    pub first_level_downsample: u32,
    pub output_voxels: usize,
    pub output_bricks: usize,
    pub diagnostic_steps: u64,
    pub peak_transformed_vertices: usize,
    pub scratch_bytes: usize,
    pub clod_bytes: usize,
    pub planned_peak_build_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClusterAsset {
    pub volume: VoxelAsset,
    pub levels: Vec<LevelMetadata>,
    pub triangle_refinements: Vec<TriangleRefinement>,
    /// Sorted, unique indices into the input assembly; exact source membership.
    pub placement_indices: Vec<u32>,
    pub source_ids: Vec<u64>,
    pub branch: u32,
    pub cluster_to_assembly: Mat4,
    /// Distinct source poses represented by this aggregate, in sorted order.
    pub source_branches: Vec<u32>,
    /// Static subtree cooking discards relative descendant animation. Such a
    /// level must not become eligible until the caller supplies a conservative
    /// bound for that discarded displacement in the selection error budget.
    pub requires_external_animation_budget: bool,
    pub stats: Stats,
}

// Same additive optical moments as foliage_voxel. Keep f64 accumulators through
// reduction, rather than reconstructing parent mass from rounded f32 means.
#[derive(Clone, Default)]
struct Mass {
    area: f64, directional: [f64; 13], color: [f64; 3], roughness: f64,
    leaf: f64, normal: [f64; 6], vectors: [[f64; 3]; 6],
}
impl Mass {
    fn add(&mut self, b: &Self) {
        self.area += b.area; self.roughness += b.roughness; self.leaf += b.leaf;
        for i in 0..13 { self.directional[i] += b.directional[i]; }
        for i in 0..3 { self.color[i] += b.color[i]; }
        for i in 0..6 { self.normal[i] += b.normal[i];
            for j in 0..3 { self.vectors[i][j] += b.vectors[i][j]; } }
    }
    /// The parts of `sample` that do not depend on the normal, for `area` of
    /// samples on one triangle at once.
    fn flat(&mut self, area: f64, directional: [f64; 13], material: &Material, projection_scale: f64) {
        self.area += area; self.roughness += area * f64::from(material.roughness);
        if material.flags & THIN_FOLIAGE != 0 { self.leaf += area; }
        for i in 0..13 { self.directional[i] += area * directional[i] * projection_scale; }
        for i in 0..3 { self.color[i] += area * f64::from(material.tint[i]); }
    }
    #[cfg(test)]
    fn sample(&mut self, area: f64, directional: [f64; 13], normal: DVec3, material: &Material, projection_scale: f64) {
        self.flat(area, directional, material, projection_scale);
        self.lobes(area, normal, material);
    }
    /// The normal lobes of one sample.
    fn lobes(&mut self, area: f64, normal: DVec3, material: &Material) {
        for k in 0..3 {
            let weight = area * normal[k] * normal[k];
            if material.flags & TWO_SIDED != 0 {
                let positive = if normal[k] < 0.0 { -normal } else { normal };
                for axis in 0..3 { let moment = positive[axis] * weight * 0.5;
                    self.vectors[2*k][axis] += moment; self.vectors[2*k+1][axis] -= moment; }
                self.normal[2*k] += weight * 0.5; self.normal[2*k+1] += weight * 0.5;
            } else {
                let lobe = 2*k + usize::from(normal[k] < 0.0);
                self.normal[lobe] += weight;
                for axis in 0..3 { self.vectors[lobe][axis] += normal[axis] * weight; }
            }
        }
    }
    fn finish(&self, coord: [u32; 3]) -> Voxel {
        Voxel { coord, surface_area: self.area as f32,
            projected_area: std::array::from_fn(|i| self.directional[i] as f32),
            directional_area: self.directional.map(|x| x as f32),
            albedo: self.color.map(|x| (x/self.area) as f32), roughness: (self.roughness/self.area) as f32,
            leaf_fraction: (self.leaf/self.area) as f32, normal_weights: self.normal.map(|x| (x/self.area) as f32),
            normal_centroids: std::array::from_fn(|i| if self.normal[i] > 0.0 {
                self.vectors[i].map(|x| (x/self.normal[i]) as f32) } else { [0.0; 3] }) }
    }
}
type Cells = BTreeMap<[u32; 3], Mass>;

fn validate_config(c: Config) -> Result<(), String> {
    if !c.reference_cell_size.is_finite() || !(0.001..=10.0).contains(&c.reference_cell_size)
        || !(1..=8).contains(&c.levels) || !(1..=4).contains(&c.samples_per_cell)
        || !(2..=64).contains(&c.diagnostic_resolution)
        || !(1..=128).contains(&c.max_bricks_per_level) || c.max_total_bricks == 0
        || c.max_reference_voxels == 0 || c.max_output_voxels == 0 || c.max_axis_cells == 0
        || c.max_axis_cells > 65_536 || c.max_placements == 0 || c.max_source_triangles == 0
        || c.max_samples == 0 || c.max_diagnostic_steps == 0 || c.max_scratch_bytes == 0
        || c.max_build_bytes == 0 { return Err("invalid bounded aggregate configuration".into()); }
    Ok(())
}

fn rigid(m: Mat4) -> bool {
    let r = Mat3::from_mat4(m);
    m.is_finite() && m.row(3).abs_diff_eq(Vec4::W, 1e-5) && r.determinant() > 0.0
        && (r.transpose()*r).abs_diff_eq(Mat3::IDENTITY, 1e-4)
}

/// The uniform scale of a rotation-scale-translation matrix, if it is one.
fn similarity(m: Mat4) -> Option<f32> {
    let r = Mat3::from_mat4(m);
    let s = r.determinant().cbrt();
    (m.is_finite() && m.row(3).abs_diff_eq(Vec4::W, 1e-5) && s > 1e-6
        && (r.transpose()*r).abs_diff_eq(Mat3::IDENTITY * (s*s), 1e-4 * s * s)).then_some(s)
}

fn check_memory(c: Config, cells: usize, stats: &mut Stats) -> Result<(), String> {
    // Old/new sparse mass maps, fine/candidate Voxel arrays and lookups may
    // coexist. Eighty bytes/cell is an explicit conservative map-node allowance.
    let per_cell = 2 * (std::mem::size_of::<Mass>() + 80)
        + 3 * (std::mem::size_of::<Voxel>() + 48);
    let bytes = cells.checked_mul(per_cell).and_then(|n| n.checked_add(stats.clod_bytes))
        .and_then(|n| n.checked_add(stats.scratch_bytes))
        .and_then(|n| n.checked_add(stats.output_voxels * std::mem::size_of::<Voxel>()))
        .and_then(|n| n.checked_add(stats.output_bricks * std::mem::size_of::<Brick>()))
        .ok_or("aggregate working memory estimate overflow")?;
    stats.planned_peak_build_bytes = stats.planned_peak_build_bytes.max(bytes);
    if bytes > c.max_build_bytes { return Err("aggregate working memory budget exceeded".into()); }
    Ok(())
}

fn transformed(vertices: &[Vertex], matrix: Mat4, scratch: &mut Vec<Vertex>, c: Config,
    stats: &mut Stats) -> Result<(), String>
{
    let bytes = vertices.len().checked_mul(std::mem::size_of::<Vertex>()).ok_or("scratch overflow")?;
    if bytes > c.max_scratch_bytes { return Err("aggregate transformed primitive scratch budget exceeded".into()); }
    let normals = Mat3::from_mat4(matrix).inverse().transpose();
    scratch.clear();
    scratch.extend(vertices.iter().map(|v| Vertex { pos: matrix.transform_point3(Vec3::from(v.pos)).to_array(),
        normal: (normals * Vec3::from(v.normal)).normalize().to_array(), uv: v.uv }));
    if scratch.iter().any(|v| !Vec3::from(v.pos).is_finite() || !Vec3::from(v.normal).is_finite()) {
        return Err("aggregate transformed source is nonfinite".into());
    }
    stats.peak_transformed_vertices = stats.peak_transformed_vertices.max(vertices.len());
    stats.scratch_bytes = stats.scratch_bytes.max(bytes);
    Ok(())
}

/// Where a triangle is sampled: equal-area points, each `sample_area`.
struct Quadrature { p: [DVec3; 3], normals: [DVec3; 3], geometric: DVec3, sample_area: f64, n: u32, sliver: Option<usize> }
impl Quadrature {
    /// None for a degenerate triangle.
    fn new(vertices: [Vertex; 3], c: Config, stats: &mut Stats) -> Result<Option<Self>, String> {
        let p = vertices.map(|v| Vec3::from(v.pos).as_dvec3());
        let normals = vertices.map(|v| Vec3::from(v.normal).as_dvec3());
        let cross = (p[1]-p[0]).cross(p[2]-p[0]);
        let area = cross.length() * 0.5;
        if !area.is_finite() { return Err("aggregate triangle area overflow".into()); }
        if area <= 1e-18 { return Ok(None); }
        let edge = p[0].distance(p[1]).max(p[1].distance(p[2])).max(p[2].distance(p[0]));
        let n = (edge / f64::from(c.reference_cell_size) * f64::from(c.samples_per_cell)).ceil().max(1.0);
        if n > 65_536.0 { return Err("aggregate triangle quadrature dimension exceeded".into()); }
        let n = n as u32;
        // A sliver (a needle face) narrower than half a cell needs samples only
        // along its length: 2n strata of equal area from its apex, instead of an
        // n x n grid across a width that never leaves one cell.
        let sides = [p[1].distance(p[2]), p[2].distance(p[0]), p[0].distance(p[1])];
        let apex = (0..3).min_by(|&a, &b| sides[a].total_cmp(&sides[b])).unwrap_or(0);
        let sliver = (n > 1 && 2.0 * area / edge < 0.5 * f64::from(c.reference_cell_size) / f64::from(c.samples_per_cell)).then_some(apex);
        let count = if sliver.is_some() { 2 * u64::from(n) } else { u64::from(n) * u64::from(n) };
        if count > c.max_samples.saturating_sub(stats.quadrature_samples) {
            return Err("aggregate source/candidate quadrature budget exceeded".into());
        }
        stats.quadrature_samples += count;
        Ok(Some(Self { p, normals, geometric: cross.normalize(), sample_area: area / count as f64, n, sliver }))
    }
    /// Every sample point and its shading normal, in a fixed order.
    fn each(&self, mut f: impl FnMut(DVec3, DVec3) -> Result<(), String>) -> Result<(), String> {
        let mut at = |w: [f64; 3]| {
            let point = self.p[0]*w[0]+self.p[1]*w[1]+self.p[2]*w[2];
            let normal = (self.normals[0]*w[0]+self.normals[1]*w[1]+self.normals[2]*w[2]).try_normalize().unwrap_or(self.geometric);
            f(point, normal)
        };
        let nf = f64::from(self.n);
        if let Some(apex) = self.sliver {
            for i in 0..2 * self.n {
                let t = ((f64::from(i) + 0.5) / (2.0 * nf)).sqrt();
                let mut w = [t * 0.5; 3]; w[apex] = 1.0 - t;
                at(w)?;
            }
            return Ok(());
        }
        for row in 0..self.n { for column in 0..self.n-row {
            let (u, v) = ((f64::from(row)+1.0/3.0)/nf, (f64::from(column)+1.0/3.0)/nf);
            at([1.0-u-v, u, v])?;
            if row+column+1<self.n {
                let (u, v) = ((f64::from(row)+2.0/3.0)/nf, (f64::from(column)+2.0/3.0)/nf);
                at([1.0-u-v, u, v])?;
            }
        }}
        Ok(())
    }
}

fn projection_scale(measure: ProjectionMeasure) -> f64 {
    match measure { ProjectionMeasure::ClosedSurfaceEntries => 0.5, ProjectionMeasure::SheetCrossings => 1.0 }
}

fn triangle(cells: &mut Cells, vertices: [Vertex; 3], material: &Material, origin: DVec3,
    dimensions: [u32; 3], c: Config, measure: ProjectionMeasure, stats: &mut Stats) -> Result<(), String>
{
    let Some(q) = Quadrature::new(vertices, c, stats)? else { return Ok(()) };
    let directions = PROJECTION_DIRECTIONS.map(|d| q.geometric.dot(Vec3::from(d).as_dvec3()).abs());
    let (sample_area, scale) = (q.sample_area, projection_scale(measure));
    let inv_cell = 1.0 / f64::from(c.reference_cell_size);
    // Neighbouring samples mostly land in the same cell: gather a run of them
    // in one Mass and add it to the map once, instead of a map lookup per sample.
    let mut run: Option<([u32; 3], Mass, u32)> = None;
    let finish = |(coord, mut mass, count): ([u32; 3], Mass, u32)| {
        mass.flat(sample_area * f64::from(count), directions, material, scale);
        (coord, mass)
    };
    q.each(|point, normal| {
        let at = ((point-origin)*inv_cell).floor();
        if at.min_element() < 0.0 || (0..3).any(|k| at[k] >= f64::from(dimensions[k])) {
            return Err("aggregate candidate escaped the common reference grid".into());
        }
        let coord = at.to_array().map(|x| x as u32);
        if run.as_ref().is_none_or(|r| r.0 != coord) {
            if let Some(done) = run.take() { add_run(cells, finish(done), c, stats)?; }
            run = Some((coord, Mass::default(), 0));
        }
        let r = run.as_mut().unwrap();
        r.1.lobes(sample_area, normal, material);
        r.2 += 1;
        Ok(())
    })?;
    if let Some(done) = run.take() { add_run(cells, finish(done), c, stats)?; }
    Ok(())
}

/// Voxel stamps (KERN_VOXEL_STAMPS=0 turns them off).
fn stamps_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KERN_VOXEL_STAMPS").map_or(true, |v| v != "0"))
}

/// The surface of one stamp cell as rotation-ready moments: area-weighted
/// position, the second moments of the face and shading normals, and the
/// shading normals' mean.
#[derive(Clone, Default)]
struct StampCell { area: f64, centroid: DVec3, faces: [f64; 6], shading: [f64; 6], mean: DVec3, exact: Vec<(DVec3, f64)>, mixed: bool }
/// Face normals a stamp cell keeps exactly before it falls back to moments.
const STAMP_FACES: usize = 8;

fn sym_add(m: &mut [f64; 6], v: DVec3, w: f64) {
    m[0] += w*v.x*v.x; m[1] += w*v.y*v.y; m[2] += w*v.z*v.z; m[3] += w*v.x*v.y; m[4] += w*v.x*v.z; m[5] += w*v.y*v.z;
}
/// R M R^T for a symmetric M stored as xx yy zz xy xz yz, as a full matrix.
fn sym_rotate(m: &[f64; 6], r: &glam::DMat3) -> glam::DMat3 {
    let full = glam::DMat3::from_cols(DVec3::new(m[0], m[3], m[4]), DVec3::new(m[3], m[1], m[5]), DVec3::new(m[4], m[5], m[2]));
    *r * full * r.transpose()
}

/// A part's surface sampled once in its own frame into cells of the
/// reference size (STAMP_CELL). A rigid placement then costs one rotation per stamp cell
/// instead of re-sampling every triangle: projected areas come from the face
/// normals' second moments (|g.d| ~ 0.1875 + 0.9375 (g.d)^2, the least-squares
/// fit over directions), normal lobes from the shading normals' moments.
struct Stamp { cells: Vec<StampCell> }
/// Stamp cell size over the reference cell.
/// Half a cell keeps a needle's footprint.
const STAMP_CELL: f32 = 0.5;
impl Stamp {
    fn new(vertices: &[Vertex], indices: &[u32], c: Config, size: f32, stats: &mut Stats) -> Result<Self, String> {
        let half = Config { reference_cell_size: c.reference_cell_size * size, ..c };
        let inv = 1.0 / f64::from(c.reference_cell_size * size);
        let mut cells: BTreeMap<[i64; 3], StampCell> = BTreeMap::new();
        for tri in indices.chunks_exact(3) {
            let v = [vertices[tri[0] as usize], vertices[tri[1] as usize], vertices[tri[2] as usize]];
            let Some(q) = Quadrature::new(v, half, stats)? else { continue };
            let (a, g) = (q.sample_area, q.geometric);
            q.each(|point, normal| {
                let k = (point * inv).floor();
                let cell = cells.entry([k.x as i64, k.y as i64, k.z as i64]).or_default();
                cell.area += a; cell.centroid += point * a; cell.mean += normal * a;
                if !cell.mixed {
                    if let Some(face) = cell.exact.iter_mut().find(|(n, _)| *n == g) { face.1 += a; }
                    else if cell.exact.len() < STAMP_FACES { cell.exact.push((g, a)); }
                    else { cell.mixed = true; cell.exact = Vec::new(); }
                }
                sym_add(&mut cell.faces, g, a); sym_add(&mut cell.shading, normal, a);
                Ok(())
            })?;
        }
        Ok(Self { cells: cells.into_values().collect() })
    }

    fn apply(&self, cells: &mut Cells, matrix: Mat4, material: &Material, origin: DVec3, dimensions: [u32; 3],
        c: Config, measure: ProjectionMeasure, stats: &mut Stats) -> Result<(), String>
    {
        let m = matrix.as_dmat4();
        let size = f64::from(similarity(matrix).ok_or("voxel stamp needs a similarity transform")?);
        let r = glam::DMat3::from_mat4(m) / size;
        let s2 = size * size;
        let (scale, inv_cell) = (projection_scale(measure), 1.0 / f64::from(c.reference_cell_size));
        let two_sided = material.flags & TWO_SIDED != 0;
        let directions = PROJECTION_DIRECTIONS.map(|d| Vec3::from(d).as_dvec3());
        for cell in &self.cells {
            let a = cell.area * s2;
            let point = m.transform_point3(cell.centroid / cell.area);
            let at = ((point - origin) * inv_cell).floor();
            if at.min_element() < 0.0 || (0..3).any(|k| at[k] >= f64::from(dimensions[k])) {
                return Err("aggregate candidate escaped the common reference grid".into());
            }
            let faces = sym_rotate(&cell.faces, &r) * s2;
            let shading = sym_rotate(&cell.shading, &r) * s2;
            let mean = r * cell.mean * s2;
            let mut mass = Mass::default();
            // Few faces: their projected areas exactly; many: from the moments.
            let projected = if cell.mixed {
                directions.map(|d| (0.1875 * a + 0.9375 * d.dot(faces * d)).max(0.0) / a)
            } else {
                directions.map(|d| cell.exact.iter().map(|&(g, w)| w * (r * g).dot(d).abs()).sum::<f64>() / cell.area)
            };
            mass.flat(a, projected, material, scale);
            for k in 0..3 {
                let weight = shading.col(k)[k];
                if weight <= 0.0 { continue; }
                // Mean normal of each lobe: along the k-th row of the moments.
                let dir = shading.col(k).try_normalize().unwrap_or(DVec3::ZERO);
                let positive = if two_sided { 0.5 } else { (0.5 + 0.5 * mean[k] / (a * weight).sqrt().max(1e-30)).clamp(0.0, 1.0) };
                mass.normal[2*k] += weight * positive; mass.normal[2*k+1] += weight * (1.0 - positive);
                for axis in 0..3 {
                    mass.vectors[2*k][axis] += dir[axis] * weight * positive;
                    mass.vectors[2*k+1][axis] -= dir[axis] * weight * (1.0 - positive);
                }
            }
            add_run(cells, (at.to_array().map(|x| x as u32), mass), c, stats)?;
        }
        Ok(())
    }
}

/// Add a run of samples that fell in one cell, minding the cell budget.
fn add_run(cells: &mut Cells, (coord, mass): ([u32; 3], Mass), c: Config, stats: &mut Stats) -> Result<(), String> {
    let len = cells.len();
    match cells.entry(coord) {
        std::collections::btree_map::Entry::Occupied(mut e) => e.get_mut().add(&mass),
        std::collections::btree_map::Entry::Vacant(e) => {
            if len >= c.max_reference_voxels { return Err("aggregate reference cell budget exceeded".into()); }
            check_memory(c, len+1, stats)?;
            e.insert(mass);
        }
    }
    Ok(())
}

fn reduce(cells: Cells) -> Cells {
    let mut parents = Cells::new();
    for (coord, mass) in cells { parents.entry(coord.map(|v| v/2)).or_default().add(&mass); }
    parents
}
fn level(cells: &Cells, dimensions: [u32; 3], cell_size: f32) -> VoxelLevel {
    VoxelLevel { cell_size, dimensions, voxels: cells.iter().map(|(&p,m)|m.finish(p)).collect() }
}
fn brick_count(cells: &Cells) -> usize {
    cells.keys().map(|p|p.map(|v|v/4)).collect::<BTreeSet<_>>().len()
}
fn bricks(level: &VoxelLevel) -> Vec<Brick> {
    let mut bricks = BTreeMap::<[u32;3],Brick>::new();
    for (index, voxel) in level.voxels.iter().enumerate() {
        let coord = voxel.coord.map(|v|v/4);
        let [x,y,z] = voxel.coord.map(|v|v%4);
        let slot = x+4*y+16*z;
        let brick = bricks.entry(coord).or_insert(Brick { coord, occupied: 0, cell_indices: [EMPTY_CELL;64] });
        brick.occupied |= 1u64<<slot;
        brick.cell_indices[slot as usize] = index as u32;
    }
    bricks.into_values().collect()
}

/// Voxel index by cell: a dense table when the grid is small enough, else a map.
enum Lookup { Dense(Vec<u32>), Sparse(CoordMap<usize>) }
const DENSE_LOOKUP_CELLS: u64 = 1 << 23;
struct Grid<'a> { level: &'a VoxelLevel, origin: DVec3, lookup: Lookup }
impl<'a> Grid<'a> {
    fn new(level: &'a VoxelLevel, origin: DVec3) -> Self {
        let [x,y,z]=level.dimensions.map(u64::from);
        let lookup=if x*y*z<=DENSE_LOOKUP_CELLS && level.voxels.len()<u32::MAX as usize {
            let mut table=vec![u32::MAX;(x*y*z) as usize];
            for (i,v) in level.voxels.iter().enumerate() {
                let [a,b,c]=v.coord.map(u64::from);
                table[(a+x*(b+y*c)) as usize]=i as u32;
            }
            Lookup::Dense(table)
        } else { Lookup::Sparse(level.voxels.iter().enumerate().map(|(i,v)|(v.coord,i)).collect()) };
        Self { level, origin, lookup }
    }
    /// The voxel at an in-grid cell.
    fn get(&self, coord: [u32;3]) -> Option<usize> {
        match &self.lookup {
            Lookup::Dense(table) => {
                let [x,y,_]=self.level.dimensions.map(u64::from);let [a,b,c]=coord.map(u64::from);
                table.get((a+x*(b+y*c)) as usize).copied().filter(|&i|i!=u32::MAX).map(|i|i as usize)
            }
            Lookup::Sparse(map) => map.get(&coord).copied(),
        }
    }
    fn upper(&self) -> DVec3 {
        self.origin+DVec3::from(self.level.dimensions.map(f64::from))*f64::from(self.level.cell_size)
    }
}
#[derive(Default)]
struct Ray { alpha: f64, color: [f64;3], normal: [f64;24] }

fn ray(grid: &Grid<'_>, start: DVec3, direction: DVec3, direction_index: usize,
    config: Config, stats: &mut Stats) -> Result<Ray,String>
{
    let mut enter = 0.0f64; let mut exit = f64::INFINITY;
    let hi = grid.upper();
    for axis in 0..3 {
        if direction[axis].abs()<1e-12 {
            if start[axis]<grid.origin[axis] || start[axis]>=hi[axis] { return Ok(Ray::default()); }
        } else {
            let a=(grid.origin[axis]-start[axis])/direction[axis];
            let b=(hi[axis]-start[axis])/direction[axis];
            enter=enter.max(a.min(b)); exit=exit.min(a.max(b));
        }
    }
    if enter>=exit { return Ok(Ray::default()); }
    let cell=f64::from(grid.level.cell_size);
    let point=start+direction*(enter+cell*1e-8);
    let mut coord=std::array::from_fn::<_,3,_>(|axis| ((point[axis]-grid.origin[axis])/cell).floor()
        .clamp(0.0,f64::from(grid.level.dimensions[axis]-1)) as i32);
    let mut t=enter; let mut transmittance=1.0; let mut result=Ray::default();
    let max_steps=grid.level.dimensions.iter().map(|&v|u64::from(v)).sum::<u64>()+4;
    for _ in 0..max_steps {
        if stats.diagnostic_steps>=config.max_diagnostic_steps { return Err("aggregate diagnostic ray-step budget exceeded".into()); }
        stats.diagnostic_steps+=1;
        let boundaries=std::array::from_fn::<_,3,_>(|axis| if direction[axis].abs()<1e-12 { f64::INFINITY } else {
            let edge=coord[axis]+i32::from(direction[axis]>0.0);
            (grid.origin[axis]+f64::from(edge)*cell-start[axis])/direction[axis]
        });
        let end=boundaries.into_iter().fold(exit,f64::min);
        if end<t-1e-8*cell { return Err("aggregate diagnostic DDA did not progress".into()); }
        if let Some(index)=grid.get(coord.map(|v|v as u32)) {
            let v=&grid.level.voxels[index];
            let tau=f64::from(v.directional_area[direction_index])/cell.powi(3)*(end-t).max(0.0);
            let absorption=transmittance*(-(-tau).exp_m1());
            for axis in 0..3 {result.color[axis]+=absorption*f64::from(v.albedo[axis]);}
            for lobe in 0..6 {
                let weight=absorption*f64::from(v.normal_weights[lobe]);
                result.normal[lobe]+=weight;
                for axis in 0..3 {result.normal[6+lobe*3+axis]+=weight*f64::from(v.normal_centroids[lobe][axis]);}
            }
            transmittance*=(-tau).exp();
        }
        t=end;
        if t>=exit-cell*1e-10 {result.alpha=1.0-transmittance;return Ok(result);}
        for axis in 0..3 {if boundaries[axis]<=end+cell*1e-10 {
            coord[axis]+=if direction[axis]>0.0 {1}else{-1};
        }}
        if (0..3).any(|axis| coord[axis]<0 || coord[axis]>=grid.level.dimensions[axis] as i32) {
            result.alpha=1.0-transmittance;return Ok(result);
        }
    }
    Err("aggregate diagnostic DDA exhausted its geometric step bound".into())
}

fn filtered_squared(differences:&[[f64;4]],n:u32,factor:u32)->[f64;2] {
    let mut squared=[0f64;2];let side=n/factor;
    for by in 0..side {for bx in 0..side {
        let mut mean=[0f64;4];
        for dy in 0..factor {for dx in 0..factor {
            let d=differences[((by*factor+dy)*n+bx*factor+dx)as usize];
            for k in 0..4 {mean[k]+=d[k]/f64::from(factor*factor);}
        }}
        squared[0]+=mean[0]*mean[0];
        squared[1]+=(mean[1]*mean[1]+mean[2]*mean[2]+mean[3]*mean[3])/3.;
    }}
    squared
}

#[test]
fn optical_filter_cancels_subpixel_rearrangement_but_retains_bias() {
    let shifted=(0..64).map(|i|[if i%2==0 {1.}else{-1.};4]).collect::<Vec<_>>();
    assert_eq!(filtered_squared(&shifted,8,1),[64.;2]);
    for factor in [2,4,8] {assert_eq!(filtered_squared(&shifted,8,factor),[0.;2]);}
    let bias=vec![[0.25;4];64];
    for factor in [1,2,4,8] {
        let x=filtered_squared(&bias,8,factor);
        assert_eq!(x,[0.0625*f64::from((8/factor)*(8/factor));2]);
    }
}

/// Opacity and premultiplied albedo of a level seen along +axis, per column
/// of reference cells (`cell` wide) over the window starting at `lo`.
fn axis_image(level: &VoxelLevel, origin: DVec3, axis: usize, lo: DVec3, cell: f64, size: [usize; 2]) -> Vec<[f64; 4]> {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let [da, db] = [level.dimensions[a] as usize, level.dimensions[b] as usize];
    // Front to back along +axis: voxels in depth order into dense columns.
    let mut order: Vec<u32> = (0..level.voxels.len() as u32).collect();
    order.sort_unstable_by_key(|&i| level.voxels[i as usize].coord[axis]);
    let mut columns = vec![[1.0f64, 0.0, 0.0, 0.0]; da * db];
    let c = f64::from(level.cell_size);
    for i in order {
        let v = &level.voxels[i as usize];
        let column = &mut columns[v.coord[a] as usize + da * v.coord[b] as usize];
        let tau = f64::from(v.directional_area[axis]) / (c * c);
        let absorbed = column[0] * -(-tau).exp_m1();
        for k in 0..3 { column[1 + k] += absorbed * f64::from(v.albedo[k]); }
        column[0] *= (-tau).exp();
    }
    let mut image = vec![[0.0f64; 4]; size[0] * size[1]];
    let span = (c / cell).round().max(1.0) as i64;
    for y in 0..db { for x in 0..da {
        let column = columns[x + da * y];
        if column[0] >= 1.0 { continue; }
        let value = [1.0 - column[0], column[1], column[2], column[3]];
        // Cover every reference column under this cell.
        let x0 = ((origin[a] + x as f64 * c - lo[a]) / cell).round() as i64;
        let y0 = ((origin[b] + y as f64 * c - lo[b]) / cell).round() as i64;
        for py in y0.max(0)..(y0 + span).min(size[1] as i64) { for px in x0.max(0)..(x0 + span).min(size[0] as i64) {
            image[py as usize * size[0] + px as usize] = value;
        }}
    }}
    image
}

/// compare() estimated from projections along the three axes, binned to the
/// same diagnostic resolution and filtered the same way.
fn compare_axes(reference: &Grid<'_>, candidate: &Grid<'_>, comparison_hi: DVec3, config: Config,
    cache: &mut BTreeMap<(usize, [usize; 2]), Vec<[f64; 4]>>) -> SampledError {
    let lo = reference.origin;
    let cell = f64::from(reference.level.cell_size);
    // One window for every level of a cook (padded by the coarsest cell a
    // level can have), so the reference's images are made once.
    let pad = cell * f64::from(1u32 << config.levels.min(16));
    let hi = (reference.upper() + DVec3::splat(pad)).max(candidate.upper()).max(comparison_hi);
    let mut result = SampledError::default();
    let n = config.diagnostic_resolution.max(1);
    for axis in 0..3 {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let size = [((hi[a] - lo[a]) / cell).ceil().max(1.0) as usize, ((hi[b] - lo[b]) / cell).ceil().max(1.0) as usize];
        // The reference's image is the same for every candidate of one cook.
        let ra = cache.entry((axis, size)).or_insert_with(|| axis_image(reference.level, reference.origin, axis, lo, cell, size));
        let ca = axis_image(candidate.level, candidate.origin, axis, lo, cell, size);
        // Mean over each of n x n bins, as one diagnostic ray per bin would see it.
        let mut differences = vec![[0f64; 4]; (n * n) as usize];
        let mut counts = vec![0f64; (n * n) as usize];
        for y in 0..size[1] { for x in 0..size[0] {
            let bin = ((y * n as usize / size[1]) * n as usize + x * n as usize / size[0]) as usize;
            let (r, c) = (ra[y * size[0] + x], ca[y * size[0] + x]);
            for k in 0..4 { differences[bin][k] += c[k] - r[k]; }
            counts[bin] += 1.0;
        }}
        let mut error = DirectionError { direction: PROJECTION_DIRECTIONS[axis], rays: n * n,
            sample_spacing_m: [(hi[a] - lo[a]) / f64::from(n), (hi[b] - lo[b]) / f64::from(n)], ..Default::default() };
        for (d, &k) in differences.iter_mut().zip(&counts) {
            if k > 0.0 { for x in d.iter_mut() { *x /= k; } }
            let delta = d[0];
            error.mean_abs_opacity += delta.abs(); error.rms_opacity += delta * delta;
            error.max_abs_opacity = error.max_abs_opacity.max(delta.abs()); error.opacity_bias += delta;
            for k in 1..4 { error.premultiplied_albedo_rmse += d[k] * d[k] / 3.0; }
        }
        for (index, factor) in [2u32, 4, 8, 16].into_iter().enumerate() {
            if factor > n || n % factor != 0 { continue; }
            let squared = filtered_squared(&differences, n, factor); let side = n / factor;
            let f = &mut result.filtered[index];
            if axis == 0 { f.coverage_error = 0.; f.color_error = 0.; }
            f.footprint_m = f.footprint_m.max((error.sample_spacing_m[0].max(error.sample_spacing_m[1]) * f64::from(factor)) as f32);
            f.coverage_error += (squared[0] / f64::from(side * side) / 3.) as f32;
            f.color_error += (squared[1] / f64::from(side * side) / 3.) as f32;
        }
        let rays = f64::from(error.rays);
        error.mean_abs_opacity /= rays; error.rms_opacity = (error.rms_opacity / rays).sqrt();
        error.opacity_bias /= rays; error.premultiplied_albedo_rmse = (error.premultiplied_albedo_rmse / rays).sqrt();
        result.mean_abs_opacity += error.mean_abs_opacity / 3.0;
        result.rms_opacity += error.rms_opacity.powi(2) / 3.0;
        result.max_abs_opacity = result.max_abs_opacity.max(error.max_abs_opacity);
        result.opacity_bias += error.opacity_bias / 3.0;
        result.premultiplied_albedo_rmse += error.premultiplied_albedo_rmse.powi(2) / 3.0;
        result.directions.push(error);
    }
    for f in &mut result.filtered { f.coverage_error = f.coverage_error.sqrt(); f.color_error = f.color_error.sqrt(); }
    result.rms_opacity = result.rms_opacity.sqrt();
    result.premultiplied_albedo_rmse = result.premultiplied_albedo_rmse.sqrt();
    result
}

fn compare(reference: &Grid<'_>, candidate: &Grid<'_>, comparison_hi: DVec3,
    config: Config, stats: &mut Stats) -> Result<SampledError,String>
{
    let lo=reference.origin; let hi=reference.upper().max(candidate.upper()).max(comparison_hi);
    let mut result=SampledError::default();
    for (direction_index, direction) in PROJECTION_DIRECTIONS.into_iter().enumerate() {
        let d=Vec3::from(direction).as_dvec3().normalize();
        let u=d.cross(if d.y.abs()<0.9 {DVec3::Y}else{DVec3::X}).normalize();
        let v=d.cross(u);
        let mut minimum=DVec3::splat(f64::INFINITY); let mut maximum=DVec3::splat(f64::NEG_INFINITY);
        for bits in 0..8 {
            let p=DVec3::new(if bits&1==0{lo.x}else{hi.x},if bits&2==0{lo.y}else{hi.y},if bits&4==0{lo.z}else{hi.z});
            let q=DVec3::new(p.dot(u),p.dot(v),p.dot(d)); minimum=minimum.min(q);maximum=maximum.max(q);
        }
        let n=config.diagnostic_resolution;
        let spacing=(maximum-minimum)/f64::from(n);
        let mut error=DirectionError {direction,rays:n*n,sample_spacing_m:[spacing.x,spacing.y],..Default::default()};
        let bytes=n as usize*n as usize*std::mem::size_of::<[f64;4]>();
        if bytes>config.max_scratch_bytes {return Err("aggregate optical filter scratch budget exceeded".into());}
        let mut differences=vec![[0f64;4];(n*n)as usize];
        for y in 0..n {for x in 0..n {
            let start=u*(minimum.x+(f64::from(x)+0.5)*spacing.x)
                +v*(minimum.y+(f64::from(y)+0.5)*spacing.y)+d*(minimum.z-f64::from(config.reference_cell_size));
            let a=ray(reference,start,d,direction_index,config,stats)?;
            let b=ray(candidate,start,d,direction_index,config,stats)?;
            let delta=b.alpha-a.alpha;
            differences[(y*n+x)as usize]=[delta,b.color[0]-a.color[0],b.color[1]-a.color[1],b.color[2]-a.color[2]];
            error.mean_abs_opacity+=delta.abs();error.rms_opacity+=delta*delta;
            error.max_abs_opacity=error.max_abs_opacity.max(delta.abs());error.opacity_bias+=delta;
            for k in 0..3 {error.premultiplied_albedo_rmse+=(b.color[k]-a.color[k]).powi(2)/3.0;}
            for k in 0..24 {error.normal_moment_mean_abs+=(b.normal[k]-a.normal[k]).abs()/24.0;}
        }}
        for (index,factor) in [2u32,4,8,16].into_iter().enumerate() {
            if factor>n || n%factor!=0 {continue;}
            let squared=filtered_squared(&differences,n,factor);let side=n/factor;
            let f=&mut result.filtered[index];
            if direction_index==0 {f.coverage_error=0.;f.color_error=0.;}
            f.footprint_m=f.footprint_m.max((spacing.x.max(spacing.y)*f64::from(factor))as f32);
            f.coverage_error+=(squared[0]/f64::from(side*side)/13.)as f32;
            f.color_error+=(squared[1]/f64::from(side*side)/13.)as f32;
        }
        let rays=f64::from(error.rays);
        error.mean_abs_opacity/=rays; error.rms_opacity=(error.rms_opacity/rays).sqrt();
        error.opacity_bias/=rays;error.premultiplied_albedo_rmse=(error.premultiplied_albedo_rmse/rays).sqrt();
        error.normal_moment_mean_abs/=rays;
        result.mean_abs_opacity+=error.mean_abs_opacity/13.0;
        result.rms_opacity+=error.rms_opacity.powi(2)/13.0;
        result.max_abs_opacity=result.max_abs_opacity.max(error.max_abs_opacity);
        result.opacity_bias+=error.opacity_bias/13.0;
        result.premultiplied_albedo_rmse+=error.premultiplied_albedo_rmse.powi(2)/13.0;
        result.normal_moment_mean_abs+=error.normal_moment_mean_abs/13.0;
        result.directions.push(error);
    }
    for f in &mut result.filtered {f.coverage_error=f.coverage_error.sqrt();f.color_error=f.color_error.sqrt();}
    result.rms_opacity=result.rms_opacity.sqrt();
    result.premultiplied_albedo_rmse=result.premultiplied_albedo_rmse.sqrt();
    Ok(result)
}

fn scale_bound(matrix: Mat4) -> f32 {
    let a=[matrix.x_axis.truncate(),matrix.y_axis.truncate(),matrix.z_axis.truncate()];
    let lengths=a.map(Vec3::length);
    if [(0,1),(0,2),(1,2)].into_iter().all(|(i,j)|a[i].dot(a[j]).abs()<=lengths[i]*lengths[j]*1e-5) {
        lengths.into_iter().fold(0.0,f32::max)
    } else {lengths.into_iter().map(|x|x*x).sum::<f32>().sqrt()}
}

/// Reuse source validation and unique primitive CLOD across all branch clusters
/// of one immutable assembly. The caller-owned source is borrowed, not copied.
pub struct Cooker<'a> {
    source: &'a Assembly,
    config: Config,
    poses: Vec<Mat4>,
    meshes: BTreeMap<(usize, bool), ClusterMesh>,
    components: BTreeMap<usize, usize>,
    clod_bytes: usize,
    /// Needle parts voxelized once in their own frame, by (part, material).
    stamps: std::sync::Mutex<BTreeMap<(u32, u32), std::sync::Arc<Stamp>>>,
    /// Each part's own triangle-cut error, by (part, material, log2 cut, log2 cell).
    cut_errors: std::sync::Mutex<BTreeMap<(u32, u32, i32, i32), std::sync::Arc<(SampledError, f64)>>>,
}
impl<'a> Cooker<'a> {
    /// A part's cut at `threshold` against its full triangles, both sampled
    /// in its own frame at `cell` (both rounded to powers of two), and the
    /// part's projected bounding area there. Measured once per part.
    fn cut_error(&self, part: u32, material: u32, threshold: f32, cell: f32) -> Result<std::sync::Arc<(SampledError, f64)>, String> {
        // At most PART_ERROR_CELLS cells across the part: a long stem scaled
        // to metres would otherwise be sampled at a sliver of a millimetre.
        let reach = self.part_extent(part).max_element().max(1e-6);
        let cell = cell.max(reach / PART_ERROR_CELLS);
        let (tq, cq) = (threshold.max(1e-6).log2().round() as i32, cell.log2().round() as i32);
        let key = (part, material, tq, cq);
        if let Some(e) = self.cut_errors.lock().unwrap_or_else(|e| e.into_inner()).get(&key) { return Ok(e.clone()); }
        let (threshold, cell) = (2f32.powi(tq), 2f32.powi(cq));
        let materials = &self.source.materials;
        let thin = materials[material as usize].flags & THIN_FOLIAGE != 0;
        let mesh = &self.meshes[&(part as usize, thin)];
        let c = Config { reference_cell_size: cell, max_samples: u64::MAX, max_reference_voxels: usize::MAX / 2, max_build_bytes: usize::MAX / 4, ..self.config };
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &mesh.vertices { lo = lo.min(Vec3::from(v.pos)); hi = hi.max(Vec3::from(v.pos)); }
        let origin = ((lo / cell).floor() - Vec3::ONE) * cell;
        let dimensions = (((hi - origin) / cell).floor() + Vec3::splat(2.0)).to_array().map(|v| v as u32);
        let mut stats = Stats::default();
        let voxelize = |cut: &[u32], stats: &mut Stats| -> Result<VoxelLevel, String> {
            let mut cells = Cells::new();
            for &cluster in cut {
                let cluster = &mesh.clusters[cluster as usize];
                for &packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri + cluster.tri_count) as usize] {
                    let v = [packed & 255, (packed >> 8) & 255, (packed >> 16) & 255].map(|i| mesh.vertices[mesh.cluster_verts[(cluster.first_vert + i) as usize] as usize]);
                    triangle(&mut cells, v, &materials[material as usize], origin.as_dvec3(), dimensions, c, self.source.projection_measure(), stats)?;
                }
            }
            Ok(level(&cells, dimensions, cell))
        };
        let cut_of = |t: f32| -> Vec<u32> { mesh.clusters.iter().enumerate().filter(|(_, c)| c.own_error <= t && c.parent_error > t).map(|(i, _)| i as u32).collect() };
        let full = voxelize(&cut_of(0.0), &mut stats)?;
        let cut = voxelize(&cut_of(threshold), &mut stats)?;
        let (a, b) = (Grid::new(&full, origin.as_dvec3()), Grid::new(&cut, origin.as_dvec3()));
        let error = compare_axes(&a, &b, a.upper(), c, &mut BTreeMap::new());
        let extent = hi - lo;
        let area = f64::from((extent.x * extent.y + extent.y * extent.z + extent.z * extent.x) / 3.0);
        let entry = std::sync::Arc::new((error, area));
        self.cut_errors.lock().unwrap_or_else(|e| e.into_inner()).insert(key, entry.clone());
        Ok(entry)
    }
    fn part_extent(&self, part: u32) -> Vec3 {
        let g = &self.source.parts[part as usize];
        let (lo, hi) = g.vertices.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), v| (lo.min(Vec3::from(v.pos)), hi.max(Vec3::from(v.pos))));
        hi - lo
    }
    /// The stamp of a placement's source triangles, made on first use.
    fn stamp(&self, part: u32, material: u32, stats: &mut Stats) -> Result<std::sync::Arc<Stamp>, String> {
        if let Some(s) = self.stamps.lock().unwrap_or_else(|e| e.into_inner()).get(&(part, material)) { return Ok(s.clone()); }
        let g = &self.source.parts[part as usize];
        let stamp = std::sync::Arc::new(Stamp::new(&g.vertices, &g.indices, self.config, STAMP_CELL, stats)?);
        self.stamps.lock().unwrap_or_else(|e| e.into_inner()).insert((part, material), stamp.clone());
        Ok(stamp)
    }
    pub fn new(source: &'a Assembly, config: Config) -> Result<Self,String> {
        validate_config(config)?;source.validate()?;
        let poses=source.branch_transforms()?;
        let mut meshes=BTreeMap::new();let mut components=BTreeMap::new();let mut clod_bytes=0usize;
        let keys=source.instances.iter().map(|p|(p.part as usize,
            source.materials[p.material as usize].flags&THIN_FOLIAGE!=0)).collect::<BTreeSet<_>>();
        for (part_index,thin) in keys {
            let geometry=&source.parts[part_index];
            let scratch=geometry.vertices.len().checked_mul(std::mem::size_of::<Vertex>())
                .ok_or("aggregate primitive scratch estimate overflow")?;
            let topology=geometry.indices.len().checked_mul(96)
                .and_then(|n|n.checked_add(geometry.vertices.len()*64))
                .and_then(|n|n.checked_add(clod_bytes)).ok_or("aggregate topology estimate overflow")?;
            if scratch>config.max_scratch_bytes || topology>config.max_build_bytes {
                return Err("aggregate unique primitive preparation budget exceeded".into());
            }
            let part=Part {vertices:&geometry.vertices,indices:&geometry.indices,material:0};
            if let std::collections::btree_map::Entry::Vacant(entry)=components.entry(part_index) {
                entry.insert(match source.projection_measure() {
                    ProjectionMeasure::ClosedSurfaceEntries => foliage_voxel::validate_closed_part(&part)?,
                    ProjectionMeasure::SheetCrossings => 0,
                });
            }
            // Match CookedPlant::cook_source's actual primitive builder: tiny
            // closed sweeps first, then the appropriate generic policy.
            let sweep=if thin {crate::vegetation_clod::build(&part)} else {None};
            let mesh=sweep.unwrap_or_else(||ClusterMesh::build_with_policy(&[part], |_|if thin {
                SimplificationPolicy::ThinFoliage } else { SimplificationPolicy::TexturedGeometry }));
            clod_bytes=clod_bytes.checked_add(mesh.vertices.len()*std::mem::size_of::<Vertex>()
                +(mesh.cluster_verts.len()+mesh.tris.len())*4
                +mesh.clusters.len()*std::mem::size_of::<crate::cluster::GpuCluster>())
                .ok_or("aggregate shared CLOD memory overflow")?;
            if clod_bytes>config.max_build_bytes {return Err("aggregate shared CLOD budget exceeded".into());}
            meshes.insert((part_index,thin),mesh);
        }
        Ok(Self {source,config,poses,meshes,components,clod_bytes,stamps:Default::default(),cut_errors:Default::default()})
    }
    pub fn cook(&self, placement_indices: &[u32], cluster_to_assembly: Mat4) -> Result<ClusterAsset,String> {
        cook_prepared(self,placement_indices,cluster_to_assembly,None,None)
    }
    /// Cook root_branch and/or descendants in their common rest-pose frame.
    /// Membership may be a subset, but must contain only descendants of this
    /// root. This explicitly loses relative descendant animation; the returned
    /// flag requires an external displacement bound before runtime selection.
    /// Use a coarser reference grid and explicit work limits for whole plants.
    /// cook_static_subtree whose reference grid comes from `lattice`, which
    /// must hold exactly these members in root_branch's frame.
    pub fn cook_static_subtree_from(&self, placement_indices: &[u32], root_branch: u32,
        cluster_to_assembly: Mat4, lattice: &Lattice) -> Result<ClusterAsset,String>
    {
        if lattice.frame!=root_branch || lattice.members!=placement_indices.len() {return Err("lattice holds other members".into());}
        cook_prepared(self,placement_indices,cluster_to_assembly,Some(root_branch),Some(lattice))
    }
    /// Members of one branch sampled into `frame_branch`'s lattice.
    pub fn lattice(&self, placement_indices: &[u32], frame_branch: u32) -> Result<Lattice,String> {
        let frame=*self.poses.get(frame_branch as usize).ok_or("lattice frame out of range")?;
        let inverse=frame.inverse();
        let matrices:Vec<Mat4>=placement_indices.iter().map(|&i|{
            let instance=&self.source.instances[i as usize];inverse*self.poses[instance.branch as usize]*instance.rest}).collect();
        let cell=self.config.reference_cell_size;
        let origin=-(LATTICE_OFFSET as f64)*f64::from(cell);
        let mut cells=Cells::new();let mut stats=Stats::default();
        let mut sorted=placement_indices.to_vec();sorted.sort_unstable();
        let order:Vec<usize>={let mut o:Vec<usize>=(0..placement_indices.len()).collect();o.sort_by_key(|&i|placement_indices[i]);o};
        let matrices:Vec<Mat4>=order.iter().map(|&i|matrices[i]).collect();
        voxelize_members(self,&sorted,&matrices,DVec3::splat(origin),[2*LATTICE_OFFSET;3],&mut cells,&mut stats)?;
        Ok(Lattice {cell,frame:frame_branch,cells,members:placement_indices.len()})
    }
    pub fn cook_static_subtree(&self, placement_indices: &[u32], root_branch: u32,
        cluster_to_assembly: Mat4) -> Result<ClusterAsset,String>
    {
        cook_prepared(self,placement_indices,cluster_to_assembly,Some(root_branch),None)
    }
}

/// Cook a pose-coherent subset of an assembly. Membership must be nonempty and
/// all placements must use the same branch. cluster_to_assembly must equal that
/// branch's rest pose; reflected/nonrigid frames are rejected. No source mesh
/// is flattened across placements. Each output level has at most 128 bricks.
pub fn cook(source: &Assembly, placement_indices: &[u32], cluster_to_assembly: Mat4,
    config: Config) -> Result<ClusterAsset,String>
{
    Cooker::new(source,config)?.cook(placement_indices,cluster_to_assembly)
}

/// Source placements sampled into one reference grid in the frame of a branch.
struct Reference {
    memberships: Vec<u32>, matrices: Vec<Mat4>, source_branches: BTreeSet<u32>, branch: u32,
    lo: Vec3, hi: Vec3, origin: Vec3, dimensions: [u32; 3], fine: Cells, stats: Stats,
}

fn reference_cells(cooker: &Cooker<'_>, placement_indices: &[u32], cluster_to_assembly: Mat4,
    subtree_root: Option<u32>, lattice: Option<&Lattice>) -> Result<Reference,String>
{
    let source=cooker.source;let config=cooker.config;
    if placement_indices.is_empty() || placement_indices.len()>config.max_placements || !rigid(cluster_to_assembly) {
        return Err("aggregate requires bounded nonempty placements and a rigid cluster frame".into());
    }
    let mut memberships=placement_indices.to_vec();memberships.sort_unstable();
    if memberships.windows(2).any(|p|p[0]==p[1]) {return Err("aggregate membership contains duplicate placements".into());}
    let poses=&cooker.poses;
    let first=source.instances.get(memberships[0] as usize).ok_or("aggregate placement index out of range")?;
    let branch=subtree_root.unwrap_or(first.branch);
    let branch_pose=poses.get(branch as usize).ok_or("aggregate root branch index out of range")?;
    if !branch_pose.abs_diff_eq(cluster_to_assembly,1e-5) {
        return Err("aggregate frame differs from source branch rest pose".into());
    }
    let inverse=cluster_to_assembly.inverse();
    let mut matrices=Vec::new();let mut used=BTreeSet::new();let mut source_branches=BTreeSet::new();
    let mut stats=Stats::default();
    let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
    for &index in &memberships {
        let instance=source.instances.get(index as usize).ok_or("aggregate placement index out of range")?;
        if subtree_root.is_some() {
            let mut ancestor=Some(instance.branch);
            while ancestor.is_some_and(|b|b!=branch) {
                ancestor=source.branches[ancestor.unwrap()as usize].parent;
            }
            if ancestor!=Some(branch) {return Err("aggregate placement is outside requested subtree".into());}
        } else if instance.branch!=branch {
            return Err("aggregate placements must share one rigid branch pose".into());
        }
        source_branches.insert(instance.branch);
        let part=&source.parts[instance.part as usize];
        stats.source_triangles+=part.indices.len() as u64/3;
        if stats.source_triangles>config.max_source_triangles {return Err("aggregate source triangle work budget exceeded".into());}
        let matrix=inverse*poses[instance.branch as usize]*instance.rest;
        for vertex in &part.vertices {let p=matrix.transform_point3(Vec3::from(vertex.pos));lo=lo.min(p);hi=hi.max(p);}
        matrices.push(matrix);used.insert(instance.part as usize);
    }
    if !lo.is_finite() || !hi.is_finite() {return Err("aggregate source bounds are invalid".into());}
    let cell=config.reference_cell_size;
    // One-cell empty border prevents roundoff at an extremal source vertex
    // becoming a clamp that silently transports optical mass to another cell.
    let origin=((lo/cell).floor()-Vec3::ONE)*cell;
    let dims=(((hi-origin)/cell).floor()+Vec3::splat(2.0)).to_array();
    if dims.iter().any(|&v|!v.is_finite() || v>config.max_axis_cells as f32) {
        return Err("aggregate reference grid axis budget exceeded".into());
    }
    let dimensions=dims.map(|v|v as u32);let origin64=origin.as_dvec3();
    stats.unique_parts_validated=used.len();
    stats.closed_components_validated=used.iter().map(|i|cooker.components[i]).sum();
    stats.clod_bytes=cooker.clod_bytes;check_memory(config,0,&mut stats)?;
    let fine=match lattice {
        None => {
            let mut fine=Cells::new();
            voxelize_members(cooker,&memberships,&matrices,origin64,dimensions,&mut fine,&mut stats)?;
            fine
        }
        Some(lattice) => lattice.window(cooker,origin,dimensions,&mut stats)?,
    };
    if fine.is_empty() {return Err("aggregate has no covered source cells".into());}
    stats.reference_voxels=fine.len();stats.reference_dimensions=dimensions;stats.reference_cell_size=cell;
    Ok(Reference {memberships,matrices,source_branches,branch,lo,hi,origin,dimensions,fine,stats})
}

/// Every member placement sampled into `cells` (stamps for needle brushes).
fn voxelize_members(cooker: &Cooker<'_>, memberships: &[u32], matrices: &[Mat4], origin64: DVec3, dimensions: [u32; 3],
    cells: &mut Cells, stats: &mut Stats) -> Result<(), String>
{
    let (source,config)=(cooker.source,cooker.config);
    let mut scratch=Vec::new();
    let stamps=stamps_enabled();
    for (&index,&matrix) in memberships.iter().zip(matrices) {
        let instance=&source.instances[index as usize];let part=&source.parts[instance.part as usize];
        let material=&source.materials[instance.material as usize];
        if stamps && material.flags&THIN_FOLIAGE!=0 && similarity(matrix).is_some() {
            let stamp=cooker.stamp(instance.part,instance.material,stats)?;
            stats.peak_transformed_vertices=stats.peak_transformed_vertices.max(part.vertices.len());
            stamp.apply(cells,matrix,material,origin64,dimensions,config,source.projection_measure(),stats)?;
            continue;
        }
        transformed(&part.vertices,matrix,&mut scratch,config,stats)?;
        for tri in part.indices.chunks_exact(3) {
            triangle(cells,[scratch[tri[0]as usize],scratch[tri[1]as usize],scratch[tri[2]as usize]],
                material,origin64,dimensions,config,source.projection_measure(),stats)?;
        }
    }
    Ok(())
}

/// Placements sampled once into one frame's unbounded cell lattice (cells at
/// multiples of the reference size from the frame origin), so a group's
/// reference grid is the sum of its children's lattices instead of a fresh
/// sampling of every member. Cell keys are offset by LATTICE_OFFSET.
pub struct Lattice { cell: f32, frame: u32, cells: Cells, members: usize }
const LATTICE_OFFSET: u32 = 1 << 20;
/// Cells across a part when its own cut error is measured.
const PART_ERROR_CELLS: f32 = 32.0;
impl Lattice {
    /// Two lattices of one frame and cell size added together.
    pub fn merge(mut self, other: Lattice) -> Result<Lattice, String> {
        if self.frame != other.frame || self.cell != other.cell { return Err("lattices of different frames".into()); }
        let (mut big, small) = if self.cells.len() >= other.cells.len() { (std::mem::take(&mut self.cells), other.cells) } else { (other.cells, std::mem::take(&mut self.cells)) };
        for (k, m) in small { big.entry(k).or_default().add(&m); }
        Ok(Lattice { cell: self.cell, frame: self.frame, cells: big, members: self.members + other.members })
    }
    /// The lattice's cells inside a reference grid, in that grid's coordinates.
    fn window(&self, cooker: &Cooker<'_>, origin: Vec3, dimensions: [u32; 3], stats: &mut Stats) -> Result<Cells, String> {
        if self.cell != cooker.config.reference_cell_size { return Err("lattice cell differs from the cook's".into()); }
        let shift = (origin / self.cell).round().as_ivec3() + glam::IVec3::splat(LATTICE_OFFSET as i32);
        let mut out = Cells::new();
        for (k, m) in &self.cells {
            let local = glam::IVec3::new(k[0] as i32, k[1] as i32, k[2] as i32) - shift;
            if local.min_element() < 0 || (0..3).any(|a| local[a] as u32 >= dimensions[a]) {
                return Err("lattice cell outside the group's reference grid".into());
            }
            out.insert(local.to_array().map(|x| x as u32), m.clone());
        }
        check_memory(cooker.config, out.len(), stats)?;
        Ok(out)
    }
}

/// The whole plant at rest as one voxel pyramid in the root branch frame: no
/// bricks, diagnostics or triangle cuts. Level zero is the reference grid.
/// For a distant plant drawn as one instance; it never sways.
pub(crate) fn cook_whole(cooker: &Cooker<'_>, levels: u32) -> Result<VoxelAsset,String> {
    if !(1..=16).contains(&levels) {return Err("whole-plant voxels need 1..16 levels".into());}
    let all:Vec<u32>=(0..cooker.source.instances.len() as u32).collect();
    let frame=*cooker.poses.first().ok_or("whole-plant voxels need a root branch")?;
    let reference=reference_cells(cooker,&all,frame,Some(0),None)?;
    let mut current=reference.fine;let mut dimensions=reference.dimensions;
    let mut cell=cooker.config.reference_cell_size;let mut out=Vec::new();
    for _ in 0..levels {
        out.push(level(&current,dimensions,cell));
        if dimensions==[1;3] {break;}
        current=reduce(current);dimensions=dimensions.map(|v|v.div_ceil(2));cell*=2.0;
    }
    let stats=BakeStats {triangles:reference.stats.source_triangles,samples:reference.stats.quadrature_samples,
        closed_parts_checked:if cooker.source.projection_measure()==ProjectionMeasure::ClosedSurfaceEntries { reference.stats.unique_parts_validated as u64 } else { 0 },
        closed_components_checked:reference.stats.closed_components_validated as u64,..Default::default()};
    Ok(VoxelAsset {projection_measure:cooker.source.projection_measure(),origin:reference.origin.to_array(),
        source_bounds:[reference.lo.to_array(),reference.hi.to_array()],levels:out,stats})
}

fn cook_prepared(cooker: &Cooker<'_>, placement_indices: &[u32], cluster_to_assembly: Mat4,
    subtree_root: Option<u32>, lattice: Option<&Lattice>)
    -> Result<ClusterAsset,String>
{
    let source=cooker.source;let config=cooker.config;
    let Reference {memberships,matrices,source_branches,branch,lo,hi,origin,dimensions,fine,mut stats}=
        reference_cells(cooker,placement_indices,cluster_to_assembly,subtree_root,lattice)?;
    let cell=config.reference_cell_size;let origin64=origin.as_dvec3();
    let cooked=&cooker.meshes;
    let reference_level=level(&fine,dimensions,cell);let reference=Grid::new(&reference_level,origin64);
    let reference_mass:[f64;13]=std::array::from_fn(|k|fine.values().map(|m|m.directional[k]).sum());
    let source_samples=stats.quadrature_samples;
    // Keep the finest authored occupancy when it fits. Unconditionally
    // reducing here removed the only unfiltered fine-voxel candidate before
    // GPU LOD selection could ever see it.
    let mut current=fine;let mut downsample=1u32;
    let mut level_dimensions=dimensions;
    while brick_count(&current)>config.max_bricks_per_level {
        if downsample>=1<<16 {return Err("aggregate cannot satisfy brick budget".into());}
        current=reduce(current);downsample*=2;level_dimensions=level_dimensions.map(|v|v.div_ceil(2));
    }
    stats.first_level_downsample=downsample;
    // Each placement's cut voxelized on its own and kept while its cut stays
    // the same: locked needles never simplify, so most placements are
    // voxelized once for every level instead of once per level.
    let mut cuts:Vec<Option<(Vec<u32>,Cells,u64)>>=(0..memberships.len()).map(|_|None).collect();
    let stamps=stamps_enabled();
    let mut triangle_alternate=|cut_error:f32,stats:&mut Stats|->Result<(VoxelLevel,u64),String> {
        let mut scratch=Vec::new();let mut alternate=Cells::new();let mut triangle_count=0u64;
        for (slot,(&index,&matrix)) in memberships.iter().zip(&matrices).enumerate() {
            let instance=&source.instances[index as usize];
            let material=&source.materials[instance.material as usize];
            let mesh=&cooked[&(instance.part as usize,material.flags&THIN_FOLIAGE!=0)];
            let threshold=cut_error/scale_bound(matrix);
            let signature:Vec<u32>=mesh.clusters.iter().enumerate()
                .filter(|(_,c)|c.own_error<=threshold && c.parent_error>threshold).map(|(i,_)|i as u32).collect();
            // An unsimplified needle cut is the source itself: stamped like
            // the reference, so it measures exactly against it.
            let stamped=stamps && material.flags&THIN_FOLIAGE!=0 && similarity(matrix).is_some()
                && mesh.clusters.iter().enumerate().filter(|(_,c)|c.own_error<=0.0 && c.parent_error>0.0).map(|(i,_)|i as u32).eq(signature.iter().copied());
            if stamped && cuts[slot].as_ref().is_none_or(|c|c.0!=signature) {
                let mut own=Cells::new();
                cooker.stamp(instance.part,instance.material,stats)?
                    .apply(&mut own,matrix,material,origin64,dimensions,config,source.projection_measure(),stats)?;
                let count=signature.iter().map(|&i|u64::from(mesh.clusters[i as usize].tri_count)).sum::<u64>();
                cuts[slot]=Some((signature.clone(),own,count));
            }
            if cuts[slot].as_ref().is_none_or(|c|c.0!=signature) {
                transformed(&mesh.vertices,matrix,&mut scratch,config,stats)?;
                let mut own=Cells::new();let mut count=0u64;
                for &cluster in &signature {
                    let cluster=&mesh.clusters[cluster as usize];
                    for &packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri+cluster.tri_count)as usize] {
                        let vertices=[packed&255,(packed>>8)&255,(packed>>16)&255].map(|i|
                            scratch[mesh.cluster_verts[(cluster.first_vert+i)as usize]as usize]);
                        triangle(&mut own,vertices,material,origin64,dimensions,config,source.projection_measure(),stats)?;
                        count+=1;
                    }
                }
                cuts[slot]=Some((signature,own,count));
            }
            let (_,own,count)=cuts[slot].as_ref().unwrap();
            for (&coord,mass) in own { alternate.entry(coord).or_default().add(mass); }
            triangle_count+=count;
        }
        if alternate.len()>config.max_reference_voxels {return Err("aggregate reference cell budget exceeded".into());}
        check_memory(config,alternate.len(),stats)?;
        Ok((level(&alternate,dimensions,cell),triangle_count))
    };
    // Estimated triangle-cut error: each member's part error at its own cut,
    // weighted by the part's share of the group's projected area (errors in
    // separate places add in squares over the group's window).
    let window={let e=(Vec3::from(hi)-Vec3::from(lo)).max(Vec3::splat(cell));f64::from((e.x*e.y+e.y*e.z+e.z*e.x)/3.0)};
    let estimate_cut=|cut_error:f32,voxel:&SampledError,_stats:&mut Stats|->Result<(SampledError,u64),String> {
        let (mut rms,mut albedo,mut filtered,mut count)=(0f64,0f64,[0f64;8],0u64);
        for (&index,&matrix) in memberships.iter().zip(&matrices) {
            let instance=&source.instances[index as usize];
            let material=&source.materials[instance.material as usize];
            let mesh=&cooked[&(instance.part as usize,material.flags&THIN_FOLIAGE!=0)];
            let scale=scale_bound(matrix);
            let threshold=cut_error/scale;
            count+=mesh.clusters.iter().filter(|c|c.own_error<=threshold && c.parent_error>threshold).map(|c|u64::from(c.tri_count)).sum::<u64>();
            let part=cooker.cut_error(instance.part,instance.material,threshold,cell/scale)?;
            let w=(part.1*f64::from(scale*scale)/window).min(1.0);
            rms+=w*part.0.rms_opacity.powi(2);albedo+=w*part.0.premultiplied_albedo_rmse.powi(2);
            for k in 0..4 {filtered[2*k]+=w*f64::from(part.0.filtered[k].coverage_error).powi(2);filtered[2*k+1]+=w*f64::from(part.0.filtered[k].color_error).powi(2);}
        }
        let mut out=SampledError {rms_opacity:rms.min(1.0).sqrt(),premultiplied_albedo_rmse:albedo.min(1.0).sqrt(),..Default::default()};
        out.mean_abs_opacity=out.rms_opacity;out.max_abs_opacity=out.rms_opacity;
        for k in 0..4 {out.filtered[k]=crate::aggregate_lod::FilteredOptics {footprint_m:voxel.filtered[k].footprint_m,
            coverage_error:filtered[2*k].min(1.0).sqrt() as f32,color_error:filtered[2*k+1].min(1.0).sqrt() as f32};}
        Ok((out,count))
    };
    let mut reference_images=BTreeMap::new();
    let mut levels=Vec::new();let mut metadata=Vec::new();
    for level_index in 0..config.levels {
        let cell_size=cell*downsample as f32;
        let output=level(&current,level_dimensions,cell_size);let brick_records=bricks(&output);
        if brick_records.len()>config.max_bricks_per_level {return Err("aggregate level exceeded brick cap".into());}
        stats.output_voxels+=output.voxels.len();stats.output_bricks+=brick_records.len();
        if stats.output_voxels>config.max_output_voxels || stats.output_bricks>config.max_total_bricks {
            return Err("aggregate total output budget exceeded".into());
        }
        check_memory(config,stats.reference_voxels,&mut stats)?;
        let candidate=Grid::new(&output,origin64);let comparison_hi=candidate.upper();
        let (sampled_error,triangle_sampled_error,triangle_count)=if config.estimated_errors {
            let voxel=compare_axes(&reference,&candidate,comparison_hi,config,&mut reference_images);
            let (triangle,count)=estimate_cut(cell_size,&voxel,&mut stats)?;
            (voxel,triangle,count)
        } else {
            let sampled_error=compare(&reference,&candidate,comparison_hi,config,&mut stats)?;
            let (alternate_level,triangle_count)=triangle_alternate(cell_size,&mut stats)?;
            let alternate_grid=Grid::new(&alternate_level,origin64);
            (sampled_error,compare(&reference,&alternate_grid,comparison_hi,config,&mut stats)?,triangle_count)
        };
        let mass_error=(0..13).map(|k| {
            let total=output.voxels.iter().map(|v|f64::from(v.directional_area[k])).sum::<f64>();
            (total-reference_mass[k]).abs()/reference_mass[k].max(1e-20)
        }).fold(0.0f64,f64::max);
        metadata.push(LevelMetadata {bricks:brick_records,sampled_error,triangle_sampled_error,
            triangle_cut_triangles:triangle_count,triangle_cut_error_m:cell_size,directional_mass_relative_error:mass_error});
        levels.push(output);
        if level_dimensions==[1;3] || level_index+1==config.levels {break;}
        current=reduce(current);downsample*=2;level_dimensions=level_dimensions.map(|v|v.div_ceil(2));
    }
    let mut triangle_refinements=Vec::new();
    if config.triangle_refinements {
        let comparison_hi=Grid::new(&levels[0],origin64).upper();
        for fraction in [1.0/64.0,1.0/16.0,1.0/4.0] {
            let cut_error_m=levels[0].cell_size*fraction;
            let (sampled_error,triangles)=if config.estimated_errors {
                estimate_cut(cut_error_m,&metadata[0].sampled_error,&mut stats)?
            } else {
                let (alternate,triangles)=triangle_alternate(cut_error_m,&mut stats)?;
                (compare(&reference,&Grid::new(&alternate,origin64),comparison_hi,config,&mut stats)?,triangles)
            };
            triangle_refinements.push(TriangleRefinement {cut_error_m,triangles,sampled_error});
        }
    }
    let bake_stats=BakeStats {triangles:stats.source_triangles,samples:source_samples,
        closed_parts_checked:if source.projection_measure()==ProjectionMeasure::ClosedSurfaceEntries { stats.unique_parts_validated as u64 } else { 0 },
        closed_components_checked:stats.closed_components_validated as u64,..Default::default()};
    let volume=VoxelAsset {projection_measure:cooker.source.projection_measure(),origin:origin.to_array(),
        source_bounds:[lo.to_array(),hi.to_array()],levels,stats:bake_stats};
    Ok(ClusterAsset {volume,levels:metadata,triangle_refinements,source_ids:memberships.iter().map(|&i|source.instances[i as usize].id).collect(),
        placement_indices:memberships,branch,cluster_to_assembly,
        source_branches:source_branches.into_iter().collect(),
        requires_external_animation_budget:subtree_root.is_some(),stats})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_assembly::{Archetype,BranchGroup,Limits,Parameters,PartGeometry,PartInstance};
    use glam::Quat;

    fn fixture(copies: usize, pivot: Mat4) -> Assembly {
        let positions=[Vec3::ZERO,Vec3::X*0.24,Vec3::Y*0.38,Vec3::Z*0.20];
        let mut part=PartGeometry {id:1,vertices:vec![],indices:vec![]};
        for tri in [[0,2,1],[0,1,3],[0,3,2],[1,2,3]] {
            let [a,b,c]=tri.map(|i|positions[i]);let normal=(b-a).cross(c-a).normalize();
            for p in [a,b,c] {part.indices.push(part.vertices.len()as u32);
                part.vertices.push(Vertex {pos:p.to_array(),normal:normal.to_array(),uv:[0.0;2]});}
        }
        let branches=vec![BranchGroup {id:10,parent:None,rest:Mat4::IDENTITY,stiffness:1.0},
            BranchGroup {id:11,parent:Some(0),rest:pivot,stiffness:0.7}];
        let instances=(0..copies).map(|i|PartInstance {id:100+i as u64,part:0,branch:1,material:0,
            rest:Mat4::from_translation(Vec3::new(i as f32*0.32,0.0,0.03*(i%2)as f32))}).collect();
        Assembly::from_parts(Parameters {seed:1,archetype:Archetype::ConiferTwig,growth:1.0,season:0.0,density:1},
            vec![part],branches,instances,vec![Material {tint:[0.18,0.37,0.11],layer:-1,
                roughness:0.8,flags:THIN_FOLIAGE|TWO_SIDED,scale:1.0,metal:0.0}],Limits::default()).unwrap()
    }
    fn config() -> Config {
        Config {reference_cell_size:0.05,levels:2,diagnostic_resolution:8,
            max_source_triangles:100,max_samples:100_000,max_reference_voxels:4096,
            max_diagnostic_steps:2_000_000,..Default::default()}
    }
    fn directional(level: &VoxelLevel) -> [f64;13] {
        std::array::from_fn(|i|level.voxels.iter().map(|v|f64::from(v.directional_area[i])).sum())
    }

    #[test]
    fn estimated_errors_follow_the_measured_ones() {
        // Axis projections: zero for the reference itself, and the coarser a
        // level the larger its error, like the ray-measured error.
        let source=fixture(3,Mat4::IDENTITY);
        let measured=Cooker::new(&source,config()).unwrap().cook(&[0,1,2],Mat4::IDENTITY).unwrap();
        let estimated=Cooker::new(&source,Config {estimated_errors:true,..config()}).unwrap().cook(&[0,1,2],Mat4::IDENTITY).unwrap();
        assert_eq!(measured.volume,estimated.volume,"estimates never change the voxels");
        let origin=DVec3::from(Vec3::from(estimated.volume.origin).as_dvec3());
        let reference=Grid::new(&estimated.volume.levels[0],origin);
        assert_eq!(compare_axes(&reference,&reference,reference.upper(),config(),&mut BTreeMap::new()).rms_opacity,0.0);
        let errors:Vec<f64>=estimated.levels.iter().map(|l|l.sampled_error.rms_opacity).collect();
        assert!(errors.windows(2).all(|w|w[1]>=w[0]),"{errors:?}");
        for (m,e) in measured.levels.iter().zip(&estimated.levels).skip(1) {
            let (m,e)=(m.sampled_error.rms_opacity,e.sampled_error.rms_opacity);
            assert!(e>0.2*m && e<5.0*m,"estimated {e} measured {m}");
        }
    }

    #[test]
    fn a_needle_stamp_matches_sampling_the_brush_in_place() {
        // A rotated, scaled pine brush: stamped against sampled triangle by triangle.
        let parts=crate::plant_growth_assembly::Parts::long_needle_pine();
        let tuft=&parts.tufts[0];
        let material=Material {tint:[0.1,0.2,0.1],layer:-1,roughness:0.8,flags:THIN_FOLIAGE|TWO_SIDED,scale:1.0,metal:0.0};
        let c=Config {reference_cell_size:0.075,max_reference_voxels:1<<20,..Config::default()};
        let matrix=Mat4::from_scale_rotation_translation(Vec3::splat(1.2),Quat::from_euler(glam::EulerRot::XYZ,0.7,-1.1,0.4),Vec3::splat(1.0));
        let (origin,dimensions)=(DVec3::ZERO,[64;3]);
        let mut stats=Stats::default();
        let mut exact=Cells::new();let mut scratch=Vec::new();
        transformed(&tuft.vertices,matrix,&mut scratch,c,&mut stats).unwrap();
        for t in tuft.indices.chunks_exact(3) {
            triangle(&mut exact,[scratch[t[0]as usize],scratch[t[1]as usize],scratch[t[2]as usize]],&material,origin,dimensions,c,ProjectionMeasure::SheetCrossings,&mut stats).unwrap();
        }
        let mut stamped=Cells::new();
        Stamp::new(&tuft.vertices,&tuft.indices,c,STAMP_CELL,&mut stats).unwrap()
            .apply(&mut stamped,matrix,&material,origin,dimensions,c,ProjectionMeasure::SheetCrossings,&mut stats).unwrap();
        let total=|cells:&Cells,f:&dyn Fn(&Mass)->f64|cells.values().map(f).sum::<f64>();
        let area=total(&exact,&|m|m.area);
        assert!((total(&stamped,&|m|m.area)/area-1.0).abs()<1e-5,"area {} vs {area}",total(&stamped,&|m|m.area));
        for k in 0..13 {
            let (a,b)=(total(&exact,&|m|m.directional[k]),total(&stamped,&|m|m.directional[k]));
            assert!((b/a-1.0).abs()<0.08,"direction {k}: stamped {b} sampled {a}");
        }
        // Mass stays where it was: the area-weighted centre moves well under a cell.
        let centre=|cells:&Cells|cells.iter().map(|(k,m)|DVec3::from(k.map(f64::from))*m.area).sum::<DVec3>()/area;
        assert!(centre(&exact).distance(centre(&stamped))<0.25,"{} vs {}",centre(&exact),centre(&stamped));
        // Most occupied cells survive.
        let shared=stamped.keys().filter(|k|exact.contains_key(*k)).count();
        assert!(shared as f64>0.6*exact.len() as f64,"{shared} of {} cells",exact.len());
    }

    #[test]
    fn open_sheet_aggregates_preserve_full_projected_area() {
        // One open triangle, not a closed proxy: its face-on optical area is
        // its full geometric area. Closing it artificially would double it.
        let mut source=fixture(1,Mat4::IDENTITY);
        source.parameters.archetype=Archetype::MeadowPatch;
        source.parts[0].indices.truncate(3);
        source.parts[0].vertices.truncate(3);
        let cooker=Cooker::new(&source,config()).unwrap();
        let cluster=cooker.cook(&[0],Mat4::IDENTITY).unwrap();
        let whole=cook_whole(&cooker,4).unwrap();
        let expected=0.24f64*0.38*0.5;
        for volume in [&cluster.volume,&whole] {
            assert_eq!(volume.projection_measure,ProjectionMeasure::SheetCrossings);
            assert_eq!(volume.stats.closed_parts_checked,0);
            assert_eq!(volume.stats.closed_components_checked,0);
            for level in &volume.levels {
                let mass=directional(level);
                for (i,d) in PROJECTION_DIRECTIONS.iter().enumerate() {
                    assert!((mass[i]-expected*f64::from(d[2]).abs()).abs()<1e-7);
                }
            }
        }
        // Closed-volume mode must still reject this same open geometry.
        source.parameters.archetype=Archetype::ConiferTwig;
        assert!(Cooker::new(&source,config()).is_err());
    }

    #[test]
    fn streamed_shared_placements_conserve_optical_mass_and_brick_membership() {
        let source=fixture(3,Mat4::IDENTITY);
        let cooker=Cooker::new(&source,config()).unwrap();
        let one=cooker.cook(&[0],Mat4::IDENTITY).unwrap();
        let all=cooker.cook(&[2,0,1],Mat4::IDENTITY).unwrap();
        assert_eq!(all.placement_indices,[0,1,2]);assert_eq!(all.source_ids,[100,101,102]);
        assert_eq!(all.stats.unique_parts_validated,1);assert_eq!(all.stats.closed_components_validated,1);
        assert_eq!(all.stats.source_triangles,12);
        assert_eq!(all.stats.peak_transformed_vertices,source.parts[0].vertices.len());
        let mass=directional(&one.volume.levels[0]);
        for (level,metadata) in all.volume.levels.iter().zip(&all.levels) {
            for (actual,expected) in directional(level).into_iter().zip(mass) {
                assert!((actual-3.0*expected).abs()<1e-7);
            }
            assert!(metadata.bricks.len()<=128);
            let mut seen=BTreeSet::new();
            for brick in &metadata.bricks {for slot in 0..64 {
                let index=brick.cell_indices[slot];
                assert_eq!(index!=EMPTY_CELL,brick.occupied&(1u64<<slot)!=0);
                if index!=EMPTY_CELL {
                    assert!(seen.insert(index));let v=&level.voxels[index as usize];
                    assert_eq!(v.coord,[brick.coord[0]*4+(slot%4)as u32,
                        brick.coord[1]*4+((slot/4)%4)as u32,brick.coord[2]*4+(slot/16)as u32]);
                }
            }}
            assert_eq!(seen.len(),level.voxels.len());
            for v in &level.voxels {
                assert!((v.leaf_fraction-1.0).abs()<1e-6);
                assert!((v.normal_weights.iter().sum::<f32>()-1.0).abs()<1e-5);
                assert!(v.albedo.iter().zip([0.18,0.37,0.11]).all(|(a,b)|(a-b).abs()<1e-6));
            }
            assert!(metadata.directional_mass_relative_error<1e-6);
        }
        assert_eq!(all,cooker.cook(&[0,1,2],Mat4::IDENTITY).unwrap());
    }

    #[test]
    fn actual_triangle_alternative_and_voxel_errors_use_same_fine_reference() {
        let source=fixture(1,Mat4::IDENTITY);
        let asset=cook(&source,&[0],Mat4::IDENTITY,config()).unwrap();
        for metadata in &asset.levels {
            // A four-face tetrahedron fits in one unchanged CLOD cluster.
            assert_eq!(metadata.triangle_cut_triangles,4);
            assert!(metadata.triangle_sampled_error.max_abs_opacity<1e-6);
            if metadata.sampled_error.max_abs_opacity!=0.0 {
                assert!(metadata.sampled_error.max_abs_opacity>0.01);
                assert!(metadata.sampled_error.rms_opacity>0.001);
            }
            assert_eq!(metadata.sampled_error.directions.len(),13);
            if metadata.sampled_error.max_abs_opacity!=0.0 {
                assert!(metadata.sampled_error.premultiplied_albedo_rmse>0.0);
                assert!(metadata.sampled_error.normal_moment_mean_abs>0.0);
            }
            for (voxel,triangle) in metadata.sampled_error.directions.iter()
                .zip(&metadata.triangle_sampled_error.directions) {
                assert_eq!(voxel.sample_spacing_m,triangle.sample_spacing_m);
                assert_eq!(voxel.rays,triangle.rays);
            }
        }
    }

    #[test]
    fn fine_reference_is_a_real_candidate_when_it_fits() {
        let source=fixture(1,Mat4::IDENTITY);
        let asset=cook(&source,&[0],Mat4::IDENTITY,config()).unwrap();
        assert_eq!(asset.stats.first_level_downsample,1);
        assert_eq!(asset.volume.levels[0].cell_size,config().reference_cell_size);
        assert_eq!(asset.levels[0].sampled_error.max_abs_opacity,0.0);
        assert_eq!(asset.levels[0].sampled_error.premultiplied_albedo_rmse,0.0);
        assert!(asset.volume.levels[1].cell_size>asset.volume.levels[0].cell_size);
        assert!(asset.levels[1].sampled_error.max_abs_opacity>0.01);
    }

    #[test]
    fn brick_caps_trigger_measured_coarsening_and_invalid_budgets_fail() {
        let source=fixture(3,Mat4::IDENTITY);
        let c=Config {max_bricks_per_level:1,max_total_bricks:2,..config()};
        let asset=cook(&source,&[0,1,2],Mat4::IDENTITY,c).unwrap();
        assert!(asset.stats.first_level_downsample>2);
        assert!(asset.levels.iter().all(|l|l.bricks.len()<=1));
        assert!(asset.levels[0].sampled_error.max_abs_opacity>0.01);
        assert!(cook(&source,&[0,0],Mat4::IDENTITY,config()).is_err());
        assert!(cook(&source,&[9],Mat4::IDENTITY,config()).is_err());
        for c in [Config {max_source_triangles:1,..config()},
            Config {max_reference_voxels:1,..config()},Config {max_scratch_bytes:1,..config()},
            Config {max_samples:1,..config()},Config {max_diagnostic_steps:1,..config()},
            Config {max_build_bytes:1,..config()},Config {max_bricks_per_level:129,..config()}] {
            assert!(cook(&source,&[0],Mat4::IDENTITY,c).is_err());
        }
    }

    #[test]
    fn cluster_frame_must_follow_exact_source_branch_pose() {
        let pivot=Mat4::from_rotation_translation(Quat::from_rotation_y(0.6),Vec3::new(4.0,2.0,-3.0));
        let source=fixture(1,pivot);
        let asset=cook(&source,&[0],pivot,config()).unwrap();
        assert_eq!(asset.branch,1);assert_eq!(asset.cluster_to_assembly,pivot);
        assert!(asset.volume.source_bounds[0][0].abs()<1e-5);
        assert!(asset.volume.source_bounds[1][1]<0.381);
        assert!(cook(&source,&[0],Mat4::IDENTITY,config()).is_err());
        let mut mixed=fixture(2,Mat4::IDENTITY);mixed.instances[1].branch=0;
        assert!(cook(&mixed,&[0,1],Mat4::IDENTITY,config()).is_err());
    }

    #[test]
    fn static_subtree_is_explicit_and_rejects_unrelated_placements() {
        let pivot=Mat4::from_translation(Vec3::new(0.7,0.4,0.0));
        let mut source=fixture(2,pivot);source.instances[1].branch=0;
        let cooker=Cooker::new(&source,config()).unwrap();
        assert!(cooker.cook(&[0,1],Mat4::IDENTITY).is_err());
        let subtree=cooker.cook_static_subtree(&[0,1],0,Mat4::IDENTITY).unwrap();
        assert!(subtree.requires_external_animation_budget);
        assert_eq!(subtree.source_branches,[0,1]);assert_eq!(subtree.branch,0);
        assert!((subtree.volume.source_bounds[1][0]-0.94).abs()<1e-5);
        assert!((subtree.volume.source_bounds[1][1]-0.78).abs()<1e-5);
        assert!(cooker.cook_static_subtree(&[0,1],1,pivot).is_err());
        assert!(cooker.cook_static_subtree(&[0],99,pivot).is_err());
        assert!(cooker.cook_static_subtree(&[0],0,pivot).is_err());
        let branch=cooker.cook(&[0],pivot).unwrap();
        assert!(!branch.requires_external_animation_budget);
        assert_eq!(branch.source_branches,[1]);
    }

    #[test]
    fn tiny_thin_primitives_use_the_same_closed_sweep_clod_as_source_rendering() {
        let source=crate::foliage_assembly::generate(Parameters::conifer_twig(9),Limits::default()).unwrap();
        let cooker=Cooker::new(&source,Config::default()).unwrap();
        let geometry=&source.parts[0];
        let expected=crate::vegetation_clod::build(&Part {vertices:&geometry.vertices,
            indices:&geometry.indices,material:0}).expect("needle has recognized closed sweep LODs");
        let actual=&cooker.meshes[&(0,true)];
        assert!(expected.levels>1);
        assert_eq!(actual.levels,expected.levels);
        assert_eq!(actual.tris,expected.tris);
        assert_eq!(actual.cluster_verts,expected.cluster_verts);
        assert_eq!(bytemuck::cast_slice::<_,u8>(&actual.vertices),bytemuck::cast_slice::<_,u8>(&expected.vertices));
        assert_eq!(bytemuck::cast_slice::<_,u8>(&actual.clusters),bytemuck::cast_slice::<_,u8>(&expected.clusters));
    }

    #[test]
    fn triangle_refinements_measure_real_finer_cuts_without_changing_voxel_mass_or_caps() {
        let source=fixture(1,Mat4::IDENTITY);
        let ordinary=cook(&source,&[0],Mat4::IDENTITY,config()).unwrap();
        assert!(ordinary.triangle_refinements.is_empty());
        let refined=cook(&source,&[0],Mat4::IDENTITY,Config {triangle_refinements:true,..config()}).unwrap();
        assert_eq!(ordinary.volume,refined.volume);
        assert_eq!(ordinary.levels,refined.levels);
        assert_eq!(refined.triangle_refinements.len(),3);
        for (cut,fraction) in refined.triangle_refinements.iter().zip([1.0/64.0,1.0/16.0,1.0/4.0]) {
            assert_eq!(cut.cut_error_m,refined.volume.levels[0].cell_size*fraction);
            // The actual tetrahedron CLOD contains one irreducible closed
            // component: every fine cut must reproduce those same four faces.
            assert_eq!(cut.triangles,4);
            assert!(cut.sampled_error.max_abs_opacity<1e-6);
            assert_eq!(cut.sampled_error.directions.len(),13);
            for (fine,coarse) in cut.sampled_error.directions.iter().zip(&refined.levels[0].sampled_error.directions) {
                assert_eq!(fine.rays,coarse.rays);assert_eq!(fine.sample_spacing_m,coarse.sample_spacing_m);
            }
        }
        // The irreducible cut is the same at every refinement, so it is
        // voxelized once and reused: no extra quadrature, but extra rays.
        assert_eq!(refined.stats.quadrature_samples,ordinary.stats.quadrature_samples);
        assert!(refined.stats.diagnostic_steps>ordinary.stats.diagnostic_steps);
        let error=cook(&source,&[0],Mat4::IDENTITY,Config {triangle_refinements:true,
            max_samples:ordinary.stats.quadrature_samples-1,..config()}).unwrap_err();
        assert!(error.contains("quadrature budget"),"{error}");
    }
}
