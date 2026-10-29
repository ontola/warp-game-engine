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
use std::collections::{BTreeMap, BTreeSet};

pub const BRICK_WIDTH: u32 = 4;
pub const EMPTY_CELL: u32 = u32::MAX;
pub const MAX_BRICKS_PER_LEVEL: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub reference_cell_size: f32,
    /// Measure three finer triangle cuts before the first voxel candidate.
    pub triangle_refinements: bool,
    /// Candidate zero begins at twice the reference cell width. If it exceeds
    /// the brick cap it is reduced further; this never implies acceptable error.
    pub levels: u32,
    pub samples_per_cell: u32,
    pub diagnostic_resolution: u32,
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
            diagnostic_resolution: 16, max_bricks_per_level: 128,
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
    fn sample(&mut self, area: f64, directional: [f64; 13], normal: DVec3, material: &Material) {
        self.area += area; self.roughness += area * f64::from(material.roughness);
        if material.flags & THIN_FOLIAGE != 0 { self.leaf += area; }
        for i in 0..13 { self.directional[i] += area * directional[i] * 0.5; }
        for i in 0..3 { self.color[i] += area * f64::from(material.tint[i]); }
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

fn triangle(cells: &mut Cells, vertices: [Vertex; 3], material: &Material, origin: DVec3,
    dimensions: [u32; 3], c: Config, stats: &mut Stats) -> Result<(), String>
{
    let p = vertices.map(|v| Vec3::from(v.pos).as_dvec3());
    let normals = vertices.map(|v| Vec3::from(v.normal).as_dvec3());
    let cross = (p[1]-p[0]).cross(p[2]-p[0]);
    let area = cross.length() * 0.5;
    if !area.is_finite() { return Err("aggregate triangle area overflow".into()); }
    if area <= 1e-18 { return Ok(()); }
    let geometric = cross.normalize();
    let directions = PROJECTION_DIRECTIONS.map(|d| geometric.dot(Vec3::from(d).as_dvec3()).abs());
    let edge = p[0].distance(p[1]).max(p[1].distance(p[2])).max(p[2].distance(p[0]));
    let n = (edge / f64::from(c.reference_cell_size) * f64::from(c.samples_per_cell)).ceil().max(1.0);
    if n > 65_536.0 { return Err("aggregate triangle quadrature dimension exceeded".into()); }
    let n = n as u32;
    let count = u64::from(n) * u64::from(n);
    if count > c.max_samples.saturating_sub(stats.quadrature_samples) {
        return Err("aggregate source/candidate quadrature budget exceeded".into());
    }
    stats.quadrature_samples += count;
    let sample_area = area / count as f64;
    let mut sample = |u: f64, v: f64| -> Result<(), String> {
        let w = 1.0-u-v;
        let point = p[0]*w+p[1]*u+p[2]*v;
        let q = ((point-origin)/f64::from(c.reference_cell_size)).floor();
        if q.min_element() < 0.0 || (0..3).any(|k| q[k] >= f64::from(dimensions[k])) {
            return Err("aggregate candidate escaped the common reference grid".into());
        }
        let coord = q.to_array().map(|x| x as u32);
        if !cells.contains_key(&coord) {
            if cells.len() >= c.max_reference_voxels { return Err("aggregate reference cell budget exceeded".into()); }
            check_memory(c, cells.len()+1, stats)?;
        }
        let normal = (normals[0]*w+normals[1]*u+normals[2]*v).try_normalize().unwrap_or(geometric);
        cells.entry(coord).or_default().sample(sample_area, directions, normal, material);
        Ok(())
    };
    for row in 0..n { for column in 0..n-row {
        sample((f64::from(row)+1.0/3.0)/f64::from(n), (f64::from(column)+1.0/3.0)/f64::from(n))?;
        if row+column+1<n { sample((f64::from(row)+2.0/3.0)/f64::from(n),
            (f64::from(column)+2.0/3.0)/f64::from(n))?; }
    }}
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

struct Grid<'a> { level: &'a VoxelLevel, origin: DVec3, lookup: BTreeMap<[u32;3],usize> }
impl<'a> Grid<'a> {
    fn new(level: &'a VoxelLevel, origin: DVec3) -> Self {
        Self { level, origin, lookup: level.voxels.iter().enumerate().map(|(i,v)|(v.coord,i)).collect() }
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
        if let Some(&index)=grid.lookup.get(&coord.map(|v|v as u32)) {
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
        for y in 0..n {for x in 0..n {
            let start=u*(minimum.x+(f64::from(x)+0.5)*spacing.x)
                +v*(minimum.y+(f64::from(y)+0.5)*spacing.y)+d*(minimum.z-f64::from(config.reference_cell_size));
            let a=ray(reference,start,d,direction_index,config,stats)?;
            let b=ray(candidate,start,d,direction_index,config,stats)?;
            let delta=b.alpha-a.alpha;
            error.mean_abs_opacity+=delta.abs();error.rms_opacity+=delta*delta;
            error.max_abs_opacity=error.max_abs_opacity.max(delta.abs());error.opacity_bias+=delta;
            for k in 0..3 {error.premultiplied_albedo_rmse+=(b.color[k]-a.color[k]).powi(2)/3.0;}
            for k in 0..24 {error.normal_moment_mean_abs+=(b.normal[k]-a.normal[k]).abs()/24.0;}
        }}
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
}
impl<'a> Cooker<'a> {
    pub fn new(source: &'a Assembly, config: Config) -> Result<Self,String> {
        validate_config(config)?;source.validate()?;
        if source.projection_measure()!=ProjectionMeasure::ClosedSurfaceEntries {
            return Err("aggregate cooker currently requires closed opaque source components".into());
        }
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
                entry.insert(foliage_voxel::validate_closed_part(&part)?);
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
        Ok(Self {source,config,poses,meshes,components,clod_bytes})
    }
    pub fn cook(&self, placement_indices: &[u32], cluster_to_assembly: Mat4) -> Result<ClusterAsset,String> {
        cook_prepared(self,placement_indices,cluster_to_assembly,None)
    }
    /// Cook root_branch and/or descendants in their common rest-pose frame.
    /// Membership may be a subset, but must contain only descendants of this
    /// root. This explicitly loses relative descendant animation; the returned
    /// flag requires an external displacement bound before runtime selection.
    /// Use a coarser reference grid and explicit work limits for whole plants.
    pub fn cook_static_subtree(&self, placement_indices: &[u32], root_branch: u32,
        cluster_to_assembly: Mat4) -> Result<ClusterAsset,String>
    {
        cook_prepared(self,placement_indices,cluster_to_assembly,Some(root_branch))
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

fn cook_prepared(cooker: &Cooker<'_>, placement_indices: &[u32], cluster_to_assembly: Mat4,
    subtree_root: Option<u32>)
    -> Result<ClusterAsset,String>
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
    let cooked=&cooker.meshes;
    let mut fine=Cells::new();let mut scratch=Vec::new();
    for (&index,&matrix) in memberships.iter().zip(&matrices) {
        let instance=&source.instances[index as usize];let part=&source.parts[instance.part as usize];
        transformed(&part.vertices,matrix,&mut scratch,config,&mut stats)?;
        for tri in part.indices.chunks_exact(3) {
            triangle(&mut fine,[scratch[tri[0]as usize],scratch[tri[1]as usize],scratch[tri[2]as usize]],
                &source.materials[instance.material as usize],origin64,dimensions,config,&mut stats)?;
        }
    }
    if fine.is_empty() {return Err("aggregate has no covered source cells".into());}
    stats.reference_voxels=fine.len();stats.reference_dimensions=dimensions;stats.reference_cell_size=cell;
    let reference_level=level(&fine,dimensions,cell);let reference=Grid::new(&reference_level,origin64);
    let reference_mass:[f64;13]=std::array::from_fn(|k|fine.values().map(|m|m.directional[k]).sum());
    let source_samples=stats.quadrature_samples;
    let mut current=reduce(fine);let mut downsample=2u32;
    let mut level_dimensions=dimensions.map(|v|v.div_ceil(2));
    while brick_count(&current)>config.max_bricks_per_level {
        if downsample>=1<<16 {return Err("aggregate cannot satisfy brick budget".into());}
        current=reduce(current);downsample*=2;level_dimensions=level_dimensions.map(|v|v.div_ceil(2));
    }
    stats.first_level_downsample=downsample;
    drop(scratch); // Release the source transform buffer before measuring CLOD cuts.
    let triangle_alternate=|cut_error:f32,stats:&mut Stats|->Result<(VoxelLevel,u64),String> {
        let mut scratch=Vec::new();let mut alternate=Cells::new();let mut triangle_count=0u64;
        for (&index,&matrix) in memberships.iter().zip(&matrices) {
            let instance=&source.instances[index as usize];
            let material=&source.materials[instance.material as usize];
            let mesh=&cooked[&(instance.part as usize,material.flags&THIN_FOLIAGE!=0)];
            transformed(&mesh.vertices,matrix,&mut scratch,config,stats)?;
            let threshold=cut_error/scale_bound(matrix);
            for cluster in mesh.clusters.iter().filter(|c|c.own_error<=threshold && c.parent_error>threshold) {
                for &packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri+cluster.tri_count)as usize] {
                    let vertices=[packed&255,(packed>>8)&255,(packed>>16)&255].map(|i|
                        scratch[mesh.cluster_verts[(cluster.first_vert+i)as usize]as usize]);
                    triangle(&mut alternate,vertices,&source.materials[instance.material as usize],
                        origin64,dimensions,config,stats)?;
                    triangle_count+=1;
                }
            }
        }
        Ok((level(&alternate,dimensions,cell),triangle_count))
    };
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
        let sampled_error=compare(&reference,&candidate,comparison_hi,config,&mut stats)?;
        let (alternate_level,triangle_count)=triangle_alternate(cell_size,&mut stats)?;
        let alternate_grid=Grid::new(&alternate_level,origin64);
        let triangle_sampled_error=compare(&reference,&alternate_grid,comparison_hi,config,&mut stats)?;
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
            let (alternate,triangles)=triangle_alternate(cut_error_m,&mut stats)?;
            let sampled_error=compare(&reference,&Grid::new(&alternate,origin64),comparison_hi,config,&mut stats)?;
            triangle_refinements.push(TriangleRefinement {cut_error_m,triangles,sampled_error});
        }
    }
    let bake_stats=BakeStats {triangles:stats.source_triangles,samples:source_samples,
        closed_parts_checked:stats.unique_parts_validated as u64,
        closed_components_checked:stats.closed_components_validated as u64,..Default::default()};
    let volume=VoxelAsset {projection_measure:ProjectionMeasure::ClosedSurfaceEntries,origin:origin.to_array(),
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
            assert!(metadata.sampled_error.max_abs_opacity>0.01);
            assert!(metadata.sampled_error.rms_opacity>0.001);
            assert_eq!(metadata.sampled_error.directions.len(),13);
            assert!(metadata.sampled_error.premultiplied_albedo_rmse>0.0);
            assert!(metadata.sampled_error.normal_moment_mean_abs>0.0);
            for (voxel,triangle) in metadata.sampled_error.directions.iter()
                .zip(&metadata.triangle_sampled_error.directions) {
                assert_eq!(voxel.sample_spacing_m,triangle.sample_spacing_m);
                assert_eq!(voxel.rays,triangle.rays);
            }
        }
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
        assert!(refined.stats.quadrature_samples>ordinary.stats.quadrature_samples);
        assert!(refined.stats.diagnostic_steps>ordinary.stats.diagnostic_steps);
        let error=cook(&source,&[0],Mat4::IDENTITY,Config {triangle_refinements:true,
            max_samples:ordinary.stats.quadrature_samples,..config()}).unwrap_err();
        assert!(error.contains("quadrature budget"),"{error}");
    }
}
