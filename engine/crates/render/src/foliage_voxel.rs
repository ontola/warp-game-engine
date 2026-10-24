//! Original offline foliage aggregation, independent of Unreal/Nanite code.
//!
//! This produces sparse *surface-density* cells, not opaque cubes or a solid
//! filled tree. Triangle quadrature samples cutout coverage and premultiplied
//! linear albedo. Coarser levels conserve surface/projected-area mass, including
//! empty space. This is asset data only; it is not a complete voxel renderer.
//! Transparent card borders must never be interpreted as solid geometry.

use crate::cluster::Part;
use crate::scene::{ALPHA_TEST, GRASS, Material, ROAD, TERRAIN, THIN_FOLIAGE, TRIPLANAR, TWO_SIDED, TextureSource};
use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

/// Antipodal directions share one extinction sample: 3 axes, 6 face
/// diagonals and 4 body diagonals of a cube. These measure geometric coverage,
/// independently of the six lighting-normal lobes below.
pub const PROJECTION_DIRECTIONS: [[f32; 3]; 13] = [
    [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0],
    [std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0],
    [std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2, 0.0],
    [std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2],
    [std::f32::consts::FRAC_1_SQRT_2, 0.0, -std::f32::consts::FRAC_1_SQRT_2],
    [0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2],
    [0.0, std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2],
    [0.57735026, 0.57735026, 0.57735026], [0.57735026, 0.57735026, -0.57735026],
    [0.57735026, -0.57735026, 0.57735026], [0.57735026, -0.57735026, -0.57735026],
];

/// What one optical crossing represents. This is geometry semantics, not an
/// artistic density multiplier. Never infer closedness from TWO_SIDED.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectionMeasure {
    /// Every covered source sheet contributes |n dot d| dA. Existing cutout
    /// assets use this convention; an open leaf has only one geometric side.
    #[default]
    SheetCrossings,
    /// Entry area of outward, closed opaque components: 1/2 |n dot d| dA.
    /// Every through-ray enters and exits a closed component. For a convex
    /// leaf this is its exact projected silhouette area; concave components
    /// can contribute multiple intervals. It is NOT an exact Beer-opacity
    /// solution for correlated opaque leaves. Each part is validated first.
    ClosedSurfaceEntries,
}

#[derive(Clone, Copy, Debug)]
pub struct BakeConfig {
    pub projection_measure: ProjectionMeasure,
    pub cell_size: f32,
    /// Includes the finest level. Each subsequent cell is twice as wide.
    pub levels: u32,
    pub samples_per_cell: u32,
    /// Match the non-jittered material raster threshold used for comparison.
    pub alpha_cut: f32,
    pub max_samples: u64,
    /// Total sparse cells across all levels, not a per-level limit.
    pub max_voxels: usize,
    /// Prepared texels, including mips, across the materials actually used.
    pub max_texture_texels: usize,
    pub max_axis_cells: u32,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self { projection_measure: ProjectionMeasure::SheetCrossings,
            cell_size: 0.25, levels: 6, samples_per_cell: 4, alpha_cut: 0.42,
            max_samples: 8_000_000, max_voxels: 262_144,
            max_texture_texels: 24_000_000, max_axis_cells: 1024 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Voxel {
    pub coord: [u32; 3],
    /// Covered one-sided geometric surface area in square local metres.
    pub surface_area: f32,
    /// Optical projected area under the asset's declared projection measure,
    /// in square local metres. Closed components count entry and exit once.
    /// Retained for mass diagnostics and the legacy upper-bound comparison.
    pub projected_area: [f32; 3],
    /// Optical integral of |geometric normal dot direction| dA for the 13 directions
    /// above. Unlike the sum of absolute axis projections, these retain
    /// cancellation for oblique/edge-on leaves. Additive through coarse levels.
    pub directional_area: [f32; 13],
    /// Coverage-weighted linear RGB, with the source material tint applied.
    pub albedo: [f32; 3],
    pub roughness: f32,
    /// Fraction of covered surface authored as thin foliage, not bark. This
    /// material moment survives mip aggregation; it is not baked illumination.
    pub leaf_fraction: f32,
    /// Normal distribution in +X,-X,+Y,-Y,+Z,-Z lobes; sums to one.
    /// Two-sided leaves distribute equal weight to opposite lobes. These are
    /// shading statistics, not normals selected randomly on every frame.
    pub normal_weights: [f32; 6],
}

impl Voxel {
    /// Legacy axis area per unit volume, retained for diagnostics/A-B only.
    /// Do not dot this with abs(ray_direction): that overestimates oblique
    /// extinction. Rendering should use density_along and segment length.
    pub fn projected_density(&self, cell_size: f32) -> [f32; 3] {
        self.projected_area.map(|a| a / cell_size.powi(3))
    }

    /// Continuous angular interpolation on 48 spherical cube triangles.
    /// Normalized nonnegative weights preserve an isotropic distribution and
    /// match the 13 stored directions exactly. Interpolation between them is
    /// approximate; it is not an exact solution for every leaf orientation.
    /// The old abs-axis sum is an upper bound and must not be used as this
    /// extinction coefficient: |nx dx|+|ny dy|+|nz dz| can be nonzero even when
    /// the actual leaf is edge-on and |dot(n,d)| is zero.
    pub fn projected_area_along(&self, direction: [f32; 3]) -> f32 {
        let direction = Vec3::from(direction);
        if !direction.is_finite() { return 0.0; }
        let Some(direction) = direction.try_normalize() else { return 0.0; };
        let d = direction.to_array();
        let magnitude = d.map(f32::abs);
        let mut axes = [0usize, 1, 2];
        axes.sort_unstable_by(|&a, &b| magnitude[b].total_cmp(&magnitude[a]));
        let [a, b, c] = axes;
        let pair = match (a.min(b), a.max(b)) { (0, 1) => 0, (0, 2) => 1, _ => 2 };
        let diagonal = 3 + pair * 2 + usize::from(d[a] * d[b] < 0.0);
        let corner = 9 + usize::from(d[0] * d[1] < 0.0) * 2 + usize::from(d[0] * d[2] < 0.0);
        let weights = [magnitude[a] - magnitude[b], (magnitude[b] - magnitude[c]) * std::f32::consts::SQRT_2,
            magnitude[c] * 3.0f32.sqrt()];
        let value = self.directional_area[a] * weights[0] + self.directional_area[diagonal] * weights[1]
            + self.directional_area[corner] * weights[2];
        value / weights.iter().sum::<f32>()
    }

    /// Local-space extinction per metre, for a unit-length segment integral.
    /// Instance scaling belongs in the ray transform/segment length, not here.
    pub fn density_along(&self, cell_size: f32, direction: [f32; 3]) -> f32 {
        self.projected_area_along(direction) / cell_size.powi(3)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoxelLevel {
    pub cell_size: f32,
    pub dimensions: [u32; 3],
    /// Lexicographically ordered coordinates for deterministic upload/cache.
    pub voxels: Vec<Voxel>,
}

impl VoxelLevel {
    /// Ray traversal bounds include cell volume, including planar source parts.
    /// Source geometry bounds alone can have zero thickness and miss all rays.
    pub fn local_bounds(&self, origin: [f32; 3]) -> [[f32; 3]; 2] {
        [origin, std::array::from_fn(|k| origin[k] + self.dimensions[k] as f32 * self.cell_size)]
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakeStats {
    pub triangles: u64,
    pub degenerate_triangles: u64,
    pub samples: u64,
    pub prepared_texture_texels: usize,
    pub closed_parts_checked: u64,
    pub closed_components_checked: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoxelAsset {
    pub projection_measure: ProjectionMeasure,
    /// All levels use this same, finest-grid-aligned local-space origin.
    pub origin: [f32; 3],
    /// Original geometry bounds; use VoxelLevel::local_bounds for ray traversal.
    pub source_bounds: [[f32; 3]; 2],
    pub levels: Vec<VoxelLevel>,
    pub stats: BakeStats,
}

// Accumulate in f64 and retain the accumulators between levels. Reconstructing
// mass from rounded f32 means at every level would gradually lose coverage.
#[derive(Clone, Default)]
struct Mass {
    area: f64,
    projected: [f64; 3],
    directional: [f64; 13],
    color: [f64; 3],
    roughness: f64,
    leaf_area: f64,
    normal: [f64; 6],
}

impl Mass {
    fn add(&mut self, other: &Self) {
        self.area += other.area;
        self.roughness += other.roughness;
        self.leaf_area += other.leaf_area;
        for k in 0..3 { self.projected[k] += other.projected[k]; self.color[k] += other.color[k]; }
        for k in 0..13 { self.directional[k] += other.directional[k]; }
        for k in 0..6 { self.normal[k] += other.normal[k]; }
    }

    fn finish(&self, coord: [u32; 3]) -> Voxel {
        Voxel { coord, surface_area: self.area as f32,
            projected_area: self.projected.map(|x| x as f32),
            directional_area: self.directional.map(|x| x as f32),
            albedo: self.color.map(|x| (x / self.area) as f32),
            roughness: (self.roughness / self.area) as f32,
            leaf_fraction: (self.leaf_area / self.area) as f32,
            normal_weights: self.normal.map(|x| (x / self.area) as f32) }
    }
}

struct Mip {
    width: usize,
    height: usize,
    /// Linear RGB premultiplied by covered fraction, followed by coverage.
    texels: Vec<[f32; 4]>,
}

struct Texture {
    mips: Vec<Mip>,
    clamp: bool,
}

fn srgb(v: u8) -> f32 {
    let x = v as f32 / 255.0;
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}

impl Texture {
    fn new(source: &TextureSource, material: &Material, config: BakeConfig, stats: &mut BakeStats) -> Result<Self, String> {
        let (mut w, mut h) = (source.rgba.width() as usize, source.rgba.height() as usize);
        if w == 0 || h == 0 { return Err("foliage voxel texture is empty".into()); }
        let mut count = 0usize;
        loop {
            count = count.checked_add(w.checked_mul(h).ok_or("texture size overflow")?).ok_or("texture mip size overflow")?;
            if w == 1 && h == 1 { break; }
            w = w.div_ceil(2); h = h.div_ceil(2);
        }
        if count > config.max_texture_texels.saturating_sub(stats.prepared_texture_texels) {
            return Err("foliage voxel texture budget exceeded; use smaller input textures or increase the explicit budget".into());
        }
        stats.prepared_texture_texels += count;
        let cutout = material.flags & ALPHA_TEST != 0;
        let threshold = if material.flags & GRASS != 0 { 0.5 } else { config.alpha_cut };
        let texels = source.rgba.pixels().map(|p| {
            let a = if !cutout || p[3] as f32 / 255.0 >= threshold { 1.0 } else { 0.0 };
            [srgb(p[0]) * a, srgb(p[1]) * a, srgb(p[2]) * a, a]
        }).collect();
        let mut mips = vec![Mip { width: source.rgba.width() as usize, height: source.rgba.height() as usize, texels }];
        while mips.last().unwrap().width != 1 || mips.last().unwrap().height != 1 {
            let previous = mips.last().unwrap();
            let (width, height) = (previous.width.div_ceil(2), previous.height.div_ceil(2));
            // Area-resample rather than duplicating the final row/column. For
            // odd-sized textures this preserves the mean of the entire image.
            let mut texels = Vec::with_capacity(width * height);
            for y in 0..height { for x in 0..width {
                let x0 = x as f64 * previous.width as f64 / width as f64;
                let x1 = (x + 1) as f64 * previous.width as f64 / width as f64;
                let y0 = y as f64 * previous.height as f64 / height as f64;
                let y1 = (y + 1) as f64 * previous.height as f64 / height as f64;
                let mut sum = [0.0f64; 4];
                for sy in y0.floor() as usize..y1.ceil() as usize { for sx in x0.floor() as usize..x1.ceil() as usize {
                    let weight = (x1.min((sx + 1) as f64) - x0.max(sx as f64)) * (y1.min((sy + 1) as f64) - y0.max(sy as f64));
                    let p = previous.texels[sy.min(previous.height - 1) * previous.width + sx.min(previous.width - 1)];
                    for k in 0..4 { sum[k] += p[k] as f64 * weight; }
                }}
                texels.push(sum.map(|v| (v / ((x1 - x0) * (y1 - y0))) as f32));
            }}
            mips.push(Mip { width, height, texels });
        }
        Ok(Self { mips, clamp: material.flags & GRASS != 0 })
    }

    fn bilinear(&self, uv: Vec2, level: usize) -> [f32; 4] {
        let mip = &self.mips[level];
        let uv = if self.clamp { uv.clamp(Vec2::ZERO, Vec2::ONE) } else { uv - uv.floor() };
        let at = uv * Vec2::new(mip.width as f32, mip.height as f32) - Vec2::splat(0.5);
        let base = at.floor(); let f = at - base;
        let index = |v: i64, n: usize| -> usize {
            if self.clamp { v.clamp(0, n as i64 - 1) as usize } else { v.rem_euclid(n as i64) as usize }
        };
        let mut out = [0.0; 4];
        for y in 0..2 { for x in 0..2 {
            let weight = if x == 0 { 1.0 - f.x } else { f.x } * if y == 0 { 1.0 - f.y } else { f.y };
            let p = mip.texels[index(base.y as i64 + y, mip.height) * mip.width + index(base.x as i64 + x, mip.width)];
            for k in 0..4 { out[k] += p[k] * weight; }
        }}
        out
    }

    fn sample(&self, uv: Vec2, footprint: f32) -> [f32; 4] {
        let lod = footprint.max(1.0).log2().clamp(0.0, (self.mips.len() - 1) as f32);
        let lower = lod.floor() as usize;
        let a = self.bilinear(uv, lower);
        let b = self.bilinear(uv, (lower + 1).min(self.mips.len() - 1));
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * lod.fract())
    }
}

/// Validate closed topology by geometric positions rather than vertex indices:
/// hard-normal meshes deliberately duplicate their vertices. Exact welding is
/// intentional; a near-open seam must not silently acquire solid semantics.
/// Each edge-connected component must have consistent outward orientation.
/// This checks topology/orientation, not arbitrary triangle self-intersection.
pub fn validate_closed_part(part: &Part<'_>) -> Result<usize, String> {
    if part.indices.is_empty() || part.indices.len() % 3 != 0 {
        return Err("closed foliage needs complete nonempty triangles".into());
    }
    let mut welded = BTreeMap::<[u32; 3], usize>::new();
    let mut points = Vec::new();
    let mut triangles = Vec::new();
    for triangle in part.indices.chunks_exact(3) {
        let mut ids = [0usize; 3];
        for (slot, &index) in triangle.iter().enumerate() {
            let p = part.vertices.get(index as usize).ok_or("closed foliage vertex out of range")?.pos;
            if p.iter().any(|v| !v.is_finite()) { return Err("non-finite closed foliage position".into()); }
            let key = p.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
            let next = points.len();
            ids[slot] = *welded.entry(key).or_insert_with(|| { points.push(Vec3::from(p).as_dvec3()); next });
        }
        let [a, b, c] = ids.map(|i| points[i]);
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[2] == ids[0]
            || (b-a).cross(c-a).length_squared() <= 4e-24 {
            return Err("closed foliage has a degenerate face".into());
        }
        triangles.push(ids);
    }
    let mut edges = BTreeMap::<(usize, usize), Vec<(usize, bool)>>::new();
    for (i, triangle) in triangles.iter().enumerate() {
        for side in 0..3 {
            let (a, b) = (triangle[side], triangle[(side+1)%3]);
            edges.entry((a.min(b), a.max(b))).or_default().push((i, a < b));
        }
    }
    let mut neighbours = vec![Vec::new(); triangles.len()];
    for edge in edges.values() {
        if edge.len() != 2 || edge[0].1 == edge[1].1 {
            return Err("closed foliage requires two opposite incidences on every edge".into());
        }
        neighbours[edge[0].0].push(edge[1].0); neighbours[edge[1].0].push(edge[0].0);
    }
    let mut seen = vec![false; triangles.len()];
    let mut components = 0;
    for start in 0..triangles.len() {
        if seen[start] { continue; }
        components += 1;
        let origin = points[triangles[start][0]];
        let mut pending = vec![start]; let mut volume6 = 0.0; let mut absolute6 = 0.0;
        seen[start] = true;
        while let Some(i) = pending.pop() {
            let [a,b,c] = triangles[i].map(|p| points[p] - origin);
            let v = a.dot(b.cross(c)); volume6 += v; absolute6 += v.abs();
            for &next in &neighbours[i] { if !seen[next] { seen[next] = true; pending.push(next); } }
        }
        if !volume6.is_finite() || volume6 <= absolute6 * 1e-10 || absolute6 == 0.0 {
            return Err("closed foliage components must enclose positive outward volume".into());
        }
    }
    Ok(components)
}

/// Bake untransformed source parts before cluster simplification. Instance
/// transforms remain outside this asset, so all copies share one bake.
///
/// UV textures use the current renderer's repeat address mode; GRASS clamps.
/// Coverage mips are built from the thresholded base image and premultiplied
/// linear RGB. This deliberately approximates subpixel cutout coverage rather
/// than filling cards or relying on a screen-space/random dither pattern.
/// Normal maps, procedural materials and transmissive shading are not baked.
pub fn bake(parts: &[Part<'_>], materials: &[Material], textures: &[TextureSource], config: BakeConfig) -> Result<VoxelAsset, String> {
    if !config.cell_size.is_finite() || config.cell_size <= 0.0 || !(1..=16).contains(&config.levels)
        || !(1..=16).contains(&config.samples_per_cell) || !config.alpha_cut.is_finite()
        || !(0.0..=1.0).contains(&config.alpha_cut) || config.max_axis_cells == 0 || config.max_axis_cells > 65_536 {
        return Err("invalid foliage voxel bake configuration".into());
    }
    let largest_cell = config.cell_size * (1u32 << (config.levels - 1)) as f32;
    if config.cell_size.powi(3) <= 0.0 || !largest_cell.powi(3).is_finite() {
        return Err("foliage voxel cell volume is not representable".into());
    }
    let mut stats = BakeStats::default();
    let mut prepared = BTreeMap::new();
    let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for part in parts {
        if part.indices.is_empty() { continue; }
        let material = materials.get(part.material as usize).ok_or("foliage part material index out of range")?;
        if material.flags & (TERRAIN | ROAD | TRIPLANAR) != 0 { return Err("foliage voxels require UV-mapped source materials".into()); }
        if material.tint.iter().any(|x| !x.is_finite() || *x < 0.0) || !material.roughness.is_finite() {
            return Err("foliage material contains invalid shading values".into());
        }
        if part.indices.len() % 3 != 0 { return Err("foliage indices do not contain whole triangles".into()); }
        for &i in part.indices {
            let vertex = part.vertices.get(i as usize).ok_or("foliage vertex index out of range")?;
            if vertex.pos.iter().chain(&vertex.normal).chain(&vertex.uv).any(|x| !x.is_finite()) { return Err("foliage vertex is not finite".into()); }
            let p = Vec3::from_array(vertex.pos); lo = lo.min(p); hi = hi.max(p);
        }
        if config.projection_measure == ProjectionMeasure::ClosedSurfaceEntries {
            if material.flags & ALPHA_TEST != 0 {
                return Err("closed foliage projection cannot use alpha-cut holes; use sheet crossings".into());
            }
            stats.closed_components_checked += validate_closed_part(part)? as u64;
            stats.closed_parts_checked += 1;
        }
        if material.layer >= 0 && !prepared.contains_key(&part.material) {
            let texture = textures.get(material.layer as usize).ok_or("foliage texture index out of range")?;
            prepared.insert(part.material, Texture::new(texture, material, config, &mut stats)?);
        }
    }
    if !lo.is_finite() { return Ok(VoxelAsset { projection_measure: config.projection_measure,
        origin: [0.0; 3], source_bounds: [[0.0; 3]; 2], levels: Vec::new(), stats }); }
    let projection_scale = match config.projection_measure {
        ProjectionMeasure::SheetCrossings => 1.0,
        ProjectionMeasure::ClosedSurfaceEntries => 0.5,
    };
    let origin = (lo / config.cell_size).floor() * config.cell_size;
    let cells = ((hi - origin) / config.cell_size).floor() + Vec3::ONE;
    if !origin.is_finite() || !cells.is_finite() || cells.max_element() > config.max_axis_cells as f32 {
        return Err("foliage voxel dimensions exceed the axis budget".into());
    }
    let dimensions = cells.to_array().map(|x| x as u32);
    let mut fine: BTreeMap<[u32; 3], Mass> = BTreeMap::new();
    for part in parts {
        if part.indices.is_empty() { continue; }
        let material = &materials[part.material as usize];
        let texture = prepared.get(&part.material);
        for indices in part.indices.chunks_exact(3) {
            stats.triangles += 1;
            let vertices: [_; 3] = std::array::from_fn(|i| part.vertices[indices[i] as usize]);
            let p = vertices.map(|v| Vec3::from_array(v.pos));
            let uv = vertices.map(|v| Vec2::from_array(v.uv));
            let normals = vertices.map(|v| Vec3::from_array(v.normal));
            let cross = (p[1] - p[0]).cross(p[2] - p[0]);
            let area = cross.length() * 0.5;
            if !area.is_finite() { return Err("foliage triangle area overflow".into()); }
            if area <= 1e-12 { stats.degenerate_triangles += 1; continue; }
            let geometric = cross.normalize();
            let directional = PROJECTION_DIRECTIONS.map(|direction| geometric.dot(Vec3::from(direction)).abs());
            let edge = (p[1] - p[0]).length().max((p[2] - p[0]).length()).max((p[2] - p[1]).length());
            let divisions = (edge / config.cell_size * config.samples_per_cell as f32).ceil().max(1.0);
            if !divisions.is_finite() || divisions > 65_536.0 { return Err("foliage triangle quadrature is too large".into()); }
            let n = divisions as u32;
            let count = n as u64 * n as u64;
            if count > config.max_samples.saturating_sub(stats.samples) { return Err("foliage voxel sample budget exceeded; increase cell size or the explicit budget".into()); }
            stats.samples += count;
            let sample_area = area as f64 / count as f64;
            let footprint = texture.map(|t| {
                let size = Vec2::new(t.mips[0].width as f32, t.mips[0].height as f32);
                ((uv[1] - uv[0]) * size).length().max(((uv[2] - uv[0]) * size).length()).max(((uv[2] - uv[1]) * size).length()) / n as f32
            }).unwrap_or(1.0);
            let mut sample = |u: f32, v: f32| -> Result<(), String> {
                let weights = Vec3::new(1.0 - u - v, u, v);
                let point = p[0] * weights.x + p[1] * weights.y + p[2] * weights.z;
                let texcoord = uv[0] * weights.x + uv[1] * weights.y + uv[2] * weights.z;
                let texel = texture.map(|t| t.sample(texcoord, footprint)).unwrap_or([1.0; 4]);
                if texel[3] <= 0.0 { return Ok(()); }
                let coord = ((point - origin) / config.cell_size).floor().to_array();
                let coord = std::array::from_fn(|k| (coord[k].max(0.0) as u32).min(dimensions[k] - 1));
                if !fine.contains_key(&coord) && fine.len() >= config.max_voxels { return Err("foliage voxel cell budget exceeded".into()); }
                let cell = fine.entry(coord).or_default();
                let covered = sample_area * texel[3] as f64;
                cell.area += covered;
                cell.roughness += covered * material.roughness.clamp(0.0, 1.0) as f64;
                if material.flags & THIN_FOLIAGE != 0 { cell.leaf_area += covered; }
                for k in 0..3 {
                    cell.projected[k] += covered * geometric[k].abs() as f64 * projection_scale;
                    cell.color[k] += sample_area * texel[k] as f64 * material.tint[k] as f64;
                }
                for k in 0..13 { cell.directional[k] += covered * directional[k] as f64 * projection_scale; }
                let normal = (normals[0] * weights.x + normals[1] * weights.y + normals[2] * weights.z).try_normalize().unwrap_or(geometric);
                for k in 0..3 {
                    let mass = covered * normal[k] as f64 * normal[k] as f64;
                    if material.flags & TWO_SIDED != 0 { cell.normal[k * 2] += mass * 0.5; cell.normal[k * 2 + 1] += mass * 0.5; }
                    else { cell.normal[k * 2 + usize::from(normal[k] < 0.0)] += mass; }
                }
                Ok(())
            };
            // n^2 equal-area triangle centroids; no RNG and no camera input.
            for row in 0..n { for column in 0..n - row {
                sample((row as f32 + 1.0 / 3.0) / n as f32, (column as f32 + 1.0 / 3.0) / n as f32)?;
                if row + column + 1 < n { sample((row as f32 + 2.0 / 3.0) / n as f32, (column as f32 + 2.0 / 3.0) / n as f32)?; }
            }}
        }
    }
    let mut levels = Vec::new();
    let mut current = fine;
    let mut dimensions = dimensions;
    let mut cell_size = config.cell_size;
    let mut total_cells = 0usize;
    for level in 0..config.levels {
        if current.len() > config.max_voxels.saturating_sub(total_cells) { return Err("foliage voxel total mip cell budget exceeded".into()); }
        total_cells += current.len();
        levels.push(VoxelLevel { cell_size, dimensions, voxels: current.iter().map(|(&coord, mass)| mass.finish(coord)).collect() });
        if level + 1 == config.levels || dimensions == [1; 3] { break; }
        let mut parents: BTreeMap<[u32; 3], Mass> = BTreeMap::new();
        for (coord, mass) in current { parents.entry(coord.map(|v| v / 2)).or_default().add(&mass); }
        current = parents;
        cell_size *= 2.0;
        dimensions = dimensions.map(|v| v.div_ceil(2));
    }
    Ok(VoxelAsset { projection_measure: config.projection_measure,
        origin: origin.to_array(), source_bounds: [lo.to_array(), hi.to_array()], levels, stats })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::Vertex;

    fn card() -> ([Vertex; 4], [u32; 6]) {
        (std::array::from_fn(|i| { let x = (i % 2) as f32; let y = (i / 2) as f32;
            Vertex { pos: [x, y, 0.0], normal: [0.0, 0.0, 1.0], uv: [x, y] } }), [0, 1, 2, 2, 1, 3])
    }
    fn material() -> Material { Material { tint: [1.0; 3], layer: 0, roughness: 0.8, flags: ALPHA_TEST | TWO_SIDED, scale: 1.0, metal: 0.0 } }
    fn texture(pixels: &[[u8; 4]]) -> TextureSource { TextureSource { name: "test".into(), rgba: image::RgbaImage::from_fn(pixels.len() as u32, 1, |x, _| image::Rgba(pixels[x as usize])), normal: None } }
    fn run(source: TextureSource, config: BakeConfig) -> VoxelAsset {
        let (vertices, indices) = card();
        bake(&[Part { vertices: &vertices, indices: &indices, material: 0 }], &[material()], &[source], config).unwrap()
    }
    fn area(level: &VoxelLevel) -> f64 { level.voxels.iter().map(|v| v.surface_area as f64).sum() }

    // Analytic opaque leaf oracle: a convex rectangular blade with actual
    // thickness. Face order is independent of the production blade generator.
    fn box_leaf(size: Vec3, rotation: glam::Quat) -> (Vec<Vertex>, Vec<u32>) {
        let p: Vec<_> = (0..8).map(|i| size * Vec3::new(
            if i&1==0 {-0.5} else {0.5}, if i&2==0 {-0.5} else {0.5},
            if i&4==0 {-0.5} else {0.5})).collect();
        let mut vertices=Vec::new();let mut indices=Vec::new();
        for face in [[0,4,6,2],[1,3,7,5],[0,1,5,4],[2,6,7,3],[0,2,3,1],[4,5,7,6]] {
            for t in [[0,1,2],[0,2,3]] {
                let [a,b,c]=t.map(|j|rotation*p[face[j]]);
                let normal=(b-a).cross(c-a).normalize().to_array();
                for q in [a,b,c] {indices.push(vertices.len() as u32);vertices.push(Vertex {pos:q.to_array(),normal,uv:[0.0;2]});}
            }
        }
        (vertices,indices)
    }

    #[test]
    fn closed_leaf_entry_area_matches_analytic_projected_silhouette_in_every_mip() {
        let size=Vec3::new(0.07,0.8,0.003);
        for rotation in [glam::Quat::IDENTITY,glam::Quat::from_euler(glam::EulerRot::XYZ,0.41,-0.77,0.19)] {
            let (vertices,indices)=box_leaf(size,rotation);
            let parts=[Part {vertices:&vertices,indices:&indices,material:0}];
            let materials=[Material {layer:-1,flags:THIN_FOLIAGE|TWO_SIDED,..material()}];
            let config=BakeConfig {projection_measure:ProjectionMeasure::ClosedSurfaceEntries,
                cell_size:0.1,levels:5,samples_per_cell:1,..Default::default()};
            let closed=bake(&parts,&materials,&[],config).unwrap();
            let sheets=bake(&parts,&materials,&[],BakeConfig {projection_measure:ProjectionMeasure::SheetCrossings,..config}).unwrap();
            assert_eq!(closed.stats.closed_parts_checked,1);
            assert_eq!(closed.stats.closed_components_checked,1);
            for (level,legacy) in closed.levels.iter().zip(&sheets.levels) {
                for (k,direction) in PROJECTION_DIRECTIONS.into_iter().enumerate() {
                    let d=(rotation.conjugate()*Vec3::from(direction)).abs();
                    let expected=size.y*size.z*d.x+size.x*size.z*d.y+size.x*size.y*d.z;
                    let measured:f32=level.voxels.iter().map(|v|v.directional_area[k]).sum();
                    assert!((measured-expected).abs()<expected*3e-5+1e-8,"dir {k}: {measured} != {expected}");
                }
                assert_eq!(area(level),area(legacy));
                for (a,b) in level.voxels.iter().zip(&legacy.voxels) {
                    assert_eq!(a.albedo,b.albedo);assert_eq!(a.normal_weights,b.normal_weights);
                    assert_eq!(a.roughness,b.roughness);assert_eq!(a.leaf_fraction,b.leaf_fraction);
                    assert_eq!(a.directional_area,b.directional_area.map(|x|x*0.5));
                }
            }
        }
    }

    #[test]
    fn closed_optics_reject_open_inward_nonmanifold_and_cutout_sources() {
        let (vertices,indices)=box_leaf(Vec3::new(0.07,0.8,0.003),glam::Quat::IDENTITY);
        let material=Material {layer:-1,flags:THIN_FOLIAGE|TWO_SIDED,..material()};
        let config=BakeConfig {projection_measure:ProjectionMeasure::ClosedSurfaceEntries,..Default::default()};
        let mut inward=indices.clone(); for t in inward.chunks_exact_mut(3) {t.swap(1,2);}
        let mut inconsistent=indices.clone();inconsistent.swap(1,2);
        let mut nonmanifold=indices.clone();nonmanifold.extend_from_slice(&indices[..3]);
        for bad in [&indices[..indices.len()-3],&inward,&inconsistent,&nonmanifold] {
            assert!(bake(&[Part {vertices:&vertices,indices:bad,material:0}],&[material],&[],config).is_err());
        }
        assert!(bake(&[Part {vertices:&vertices,indices:&indices,material:0}],
            &[Material {flags:ALPHA_TEST|TWO_SIDED,..material}],&[],config).is_err());
        let (sheet,sheet_indices)=card();
        assert!(bake(&[Part {vertices:&sheet,indices:&sheet_indices,material:0}],&[material],&[],config).is_err());
        // A correctly oriented component cannot hide an inverted neighbour.
        let mut both=vertices.clone();both.extend(vertices.iter().map(|v|Vertex {pos:[v.pos[0]+1.0,v.pos[1],v.pos[2]],..*v}));
        let mut both_indices=indices.clone();both_indices.extend(inward.iter().map(|i|i+vertices.len() as u32));
        assert!(validate_closed_part(&Part {vertices:&both,indices:&both_indices,material:0}).is_err());
        both_indices.truncate(indices.len());both_indices.extend(indices.iter().map(|i|i+vertices.len() as u32));
        assert_eq!(validate_closed_part(&Part {vertices:&both,indices:&both_indices,material:0}).unwrap(),2);
    }

    #[test]
    fn legacy_sheet_projection_is_default_and_is_not_automatically_halved() {
        let config=BakeConfig::default();
        assert_eq!(config.projection_measure,ProjectionMeasure::SheetCrossings);
        let default=run(texture(&[[255;4]]),config);
        let explicit=run(texture(&[[255;4]]),BakeConfig {projection_measure:ProjectionMeasure::SheetCrossings,..config});
        assert_eq!(default,explicit);
        for level in default.levels {
            let projected:f32=level.voxels.iter().map(|v|v.directional_area[2]).sum();
            assert!((projected-1.0).abs()<1e-6);
        }
    }

    #[test]
    fn leaf_and_bark_material_mass_survives_every_mip_without_baked_lighting() {
        let (bark, indices) = card();
        let mut leaf = bark;
        // Half the surface area, spatially separate at the finest level.
        for v in &mut leaf { v.pos[0] = v.pos[0] * 0.5 + 2.0; }
        let materials = [
            Material { layer: -1, flags: 0, ..material() },
            Material { layer: -1, flags: TWO_SIDED | THIN_FOLIAGE, ..material() },
        ];
        let asset = bake(&[
            Part { vertices: &bark, indices: &indices, material: 0 },
            Part { vertices: &leaf, indices: &indices, material: 1 },
        ], &materials, &[], BakeConfig::default()).unwrap();
        for level in &asset.levels {
            let leaf_area: f64 = level.voxels.iter().map(|v| (v.surface_area * v.leaf_fraction) as f64).sum();
            assert!((leaf_area - 0.5).abs() < 1e-6);
            assert!((area(level) - 1.5).abs() < 1e-6);
            assert!(level.voxels.iter().all(|v| (0.0..=1.0).contains(&v.leaf_fraction)));
        }
        let coarsest = asset.levels.last().unwrap();
        assert!(coarsest.voxels.iter().any(|v| v.leaf_fraction > 0.0 && v.leaf_fraction < 1.0));
    }

    #[test]
    fn empty_and_full_alpha_do_not_make_solid_cards() {
        let empty = run(texture(&[[255, 255, 255, 0]]), BakeConfig::default());
        assert!(empty.levels.iter().all(|l| l.voxels.is_empty()));
        let full = run(texture(&[[255, 255, 255, 255]]), BakeConfig::default());
        assert!((area(&full.levels[0]) - 1.0).abs() < 1e-6);
        for voxel in &full.levels[0].voxels { assert_eq!(voxel.projected_area[0], 0.0); assert_eq!(voxel.projected_area[1], 0.0); }
    }

    #[test]
    fn cutout_coverage_and_premultiplied_linear_color_survive_mips() {
        let config = BakeConfig { cell_size: 1.0, samples_per_cell: 1, ..Default::default() };
        let result = run(texture(&[[255, 0, 255, 0], [128, 128, 128, 255]]), config);
        assert!((area(&result.levels[0]) - 0.5).abs() < 1e-6);
        for voxel in &result.levels[0].voxels { for color in voxel.albedo { assert!((color - srgb(128)).abs() < 1e-6); } }
    }

    #[test]
    fn coarse_cells_preserve_area_and_integrated_directional_density() {
        let asset = run(texture(&[[30, 180, 60, 255]]), BakeConfig::default());
        for level in &asset.levels {
            assert!((area(level) - 1.0).abs() < 1e-6);
            let integral: f64 = level.voxels.iter().map(|v| (v.projected_density(level.cell_size)[2] * level.cell_size.powi(3)) as f64).sum();
            assert!((integral - 1.0).abs() < 1e-6);
            for voxel in &level.voxels {
                assert!((voxel.normal_weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                assert!((voxel.normal_weights[4] - 0.5).abs() < 1e-6);
                assert!((voxel.normal_weights[5] - 0.5).abs() < 1e-6);
            }
        }
        let fine = asset.levels[0].voxels.iter().map(|v| v.projected_density(asset.levels[0].cell_size)[2]).fold(0.0f32, f32::max);
        let coarse = asset.levels.last().unwrap();
        assert!(coarse.voxels[0].projected_density(coarse.cell_size)[2] < fine);
    }

    #[test]
    fn bake_is_deterministic_and_sparse_coordinates_are_sorted() {
        let a = run(texture(&[[10, 80, 20, 255], [250, 0, 0, 0]]), BakeConfig::default());
        let b = run(texture(&[[10, 80, 20, 255], [250, 0, 0, 0]]), BakeConfig::default());
        assert_eq!(a, b);
        for level in a.levels { assert!(level.voxels.windows(2).all(|w| w[0].coord < w[1].coord)); }
    }

    #[test]
    fn texture_filtering_is_linear_and_wrap_mode_matches_material() {
        let mut stats = BakeStats::default(); let source = texture(&[[0, 0, 0, 255], [255, 255, 255, 255]]);
        let repeated = Texture::new(&source, &material(), BakeConfig::default(), &mut stats).unwrap();
        assert!((repeated.sample(Vec2::new(0.5, 0.5), 1.0)[0] - 0.5).abs() < 1e-6);
        assert_eq!(repeated.sample(Vec2::new(0.25, 0.5), 1.0), repeated.sample(Vec2::new(-0.75, 0.5), 1.0));
        let clamped = Texture::new(&source, &Material { flags: GRASS | ALPHA_TEST, ..material() }, BakeConfig::default(), &mut stats).unwrap();
        assert_eq!(clamped.sample(Vec2::new(-2.0, 0.5), 1.0)[0], 0.0);
    }

    #[test]
    fn odd_texture_mips_preserve_coverage_without_repeating_the_edge() {
        let mut stats = BakeStats::default();
        let t = Texture::new(&texture(&[[255, 0, 0, 0], [255, 0, 0, 0], [0, 255, 0, 255]]), &material(), BakeConfig::default(), &mut stats).unwrap();
        let final_pixel = t.mips.last().unwrap().texels[0];
        assert!((final_pixel[3] - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(final_pixel[0], 0.0);
        assert!((final_pixel[1] - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn planar_geometry_gets_traversable_cell_bounds_and_directional_statistics() {
        let (mut vertices, indices) = card();
        for vertex in &mut vertices { vertex.normal = [std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2]; }
        let asset = bake(&[Part { vertices: &vertices, indices: &indices, material: 0 }], &[material()], &[texture(&[[255; 4]])], BakeConfig::default()).unwrap();
        assert_eq!(asset.source_bounds[0][2], asset.source_bounds[1][2]);
        for level in &asset.levels {
            let bounds = level.local_bounds(asset.origin);
            assert!(bounds[1][2] > bounds[0][2]);
            for voxel in &level.voxels {
                for k in [0, 1, 4, 5] { assert!((voxel.normal_weights[k] - 0.25).abs() < 1e-6); }
                assert_eq!(voxel.normal_weights[2], 0.0);
            }
        }
    }

    #[test]
    fn invalid_inputs_and_explicit_budgets_fail_without_partial_asset() {
        let (vertices, indices) = card(); let parts = [Part { vertices: &vertices, indices: &indices, material: 0 }];
        for config in [BakeConfig { max_samples: 1, ..Default::default() }, BakeConfig { max_voxels: 0, ..Default::default() }, BakeConfig { cell_size: f32::NAN, ..Default::default() }, BakeConfig { max_texture_texels: 0, ..Default::default() }] {
            assert!(bake(&parts, &[material()], &[texture(&[[255; 4]])], config).is_err());
        }
        let bad = [Part { vertices: &vertices, indices: &[0, 1, 9], material: 0 }];
        assert!(bake(&bad, &[material()], &[texture(&[[255; 4]])], BakeConfig::default()).is_err());
    }

    #[test]
    fn edge_on_oblique_card_does_not_occlude_as_the_axis_upper_bound_does() {
        let (mut vertices, indices) = card();
        let rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        for vertex in &mut vertices {
            vertex.pos = (rotation * Vec3::from(vertex.pos)).to_array();
            vertex.normal = (rotation * Vec3::from(vertex.normal)).to_array();
        }
        let asset = bake(&[Part { vertices: &vertices, indices: &indices, material: 0 }],
            &[material()], &[texture(&[[255; 4]])], BakeConfig::default()).unwrap();
        let direction = PROJECTION_DIRECTIONS[6];
        for level in &asset.levels {
            let actual: f32 = level.voxels.iter().map(|v| v.projected_area_along(direction)).sum();
            let old: f32 = level.voxels.iter().map(|v| Vec3::from(v.projected_area).dot(Vec3::from(direction).abs())).sum();
            assert!(actual < 1e-5, "edge-on projected area {actual}");
            assert!(old > 0.99, "legacy method should expose the original false-occlusion bug");
        }
    }

    #[test]
    fn directional_samples_and_coarse_integrals_are_preserved() {
        let asset = run(texture(&[[30, 180, 60, 255]]), BakeConfig::default());
        let ray = [0.87, -0.14, 0.49];
        let fine: f32 = asset.levels[0].voxels.iter().map(|v| v.projected_area_along(ray)).sum();
        for level in &asset.levels {
            let integrated: f32 = level.voxels.iter().map(|v| v.density_along(level.cell_size, ray) * level.cell_size.powi(3)).sum();
            assert!((integrated - fine).abs() < 1e-6);
            for voxel in &level.voxels {
                for (k, direction) in PROJECTION_DIRECTIONS.into_iter().enumerate() {
                    assert!((voxel.projected_area_along(direction) - voxel.directional_area[k]).abs() < 1e-6);
                    assert!((voxel.projected_area_along(direction.map(|v| -v)) - voxel.directional_area[k]).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn angular_interpolation_is_continuous_and_preserves_isotropic_density() {
        let isotropic = Mass { area: 1.0, directional: [0.5; 13], ..Default::default() }.finish([0; 3]);
        for i in 0..180 {
            let angle = i as f32 * std::f32::consts::PI / 90.0;
            let direction = [angle.sin(), 0.37, angle.cos()];
            assert!((isotropic.projected_area_along(direction) - 0.5).abs() < 1e-6);
        }
        let mut varied = isotropic.clone();
        varied.directional_area = std::array::from_fn(|i| i as f32 / 13.0);
        for direction in [[1.0, 1.0, 0.3], [1.0, 0.3, 0.3], [1.0, 0.0, 0.3], [-1.0, 1.0, 1.0]] {
            let a = varied.projected_area_along([direction[0], direction[1] - 1e-6, direction[2]]);
            let b = varied.projected_area_along([direction[0], direction[1] + 1e-6, direction[2]]);
            assert!((a - b).abs() < 1e-5);
        }
        assert_eq!(isotropic.projected_area_along([0.0; 3]), 0.0);
        assert_eq!(isotropic.projected_area_along([f32::NAN; 3]), 0.0);
    }
}
