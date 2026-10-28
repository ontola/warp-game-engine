//! Directional, source-derived coverage for distant meadow patches.
//!
//! This is an opaque-surface atlas, not a volume density or a voxel marcher.
//! Each direction rasterizes the finest authored ribbon triangles and stores
//! their union coverage and nearest visible surface. Four-by-four subpixels
//! preserve holes without alpha/density amplification. Coarser levels average
//! coverage, while surface attributes remain conditional on one branch so a
//! fractional branch index can never animate an unrelated pose.
//!
//! Rest-pose coverage is an approximation when the runtime bends each blade.
//! The representative surface and its basal anchor can share current/previous
//! branch poses, but this atlas does not reproduce every moving silhouette.

use crate::meadow_data::{GpuMeadowVertex, MeadowGeometry, ROOT_CELL, VARIANTS};
use glam::DVec3;
use std::time::Instant;

pub const VIEW_COUNT: u32 = 24;
pub const SIZE: u32 = 32;
pub const MIP_COUNT: u32 = 6;
const SUPERSAMPLE: usize = 4;
const HEADER_WORDS: usize = 3;
const TEXEL_WORDS: usize = 4;
const MIP_TEXELS: usize = 32 * 32 + 16 * 16 + 8 * 8 + 4 * 4 + 2 * 2 + 1;

/// Two filterable RGBA16Float array textures, each with 144 layers. Pairing
/// attributes within two arrays stays below the portable 256-layer limit.
#[derive(Clone, Debug)]
pub struct AggregateTextureData {
    /// Three f32 projection words per (variant, direction), unchanged from the
    /// source atlas. GPU header strides are 3 words/view and 72 words/variant.
    pub headers: Vec<[f32; 4]>,
    pub layers: u32,
    pub mips: Vec<AggregateTextureMip>,
}

#[derive(Clone, Debug)]
pub struct AggregateTextureMip {
    pub width: u32,
    pub height: u32,
    /// Layer-major RGBA half-float bits. For view=(variant*24+direction),
    /// layer view*2 contains [RGB*coverage, coverage]; layer view*2+1 contains
    /// [normal*coverage, roughness*coverage]. Filter these moments, then divide
    /// by filtered coverage. Never interpolate unpremultiplied attributes.
    pub material: Vec<u16>,
    /// Same paired layer layout: [source_point.xyz, integer branch] then
    /// [basal_anchor.xyz,unbent_height]. Height is zero for legacy recipes.
    /// Use textureLoad at a covered representative texel;
    /// hardware filtering of these layers would interpolate branch identity.
    pub surface: Vec<u16>,
}

impl AggregateTextureData {
    /// Both texture payloads plus shared f32 projection headers. Excludes
    /// allocation padding, texture metadata, samplers and pipeline resources.
    pub fn payload_bytes(&self) -> u64 {
        (self.headers.len() * 16) as u64 + self.mips.iter()
            .map(|m| ((m.material.len() + m.surface.len()) * 2) as u64).sum::<u64>()
    }
}

#[derive(Clone, Debug)]
pub struct AggregateAtlas {
    /// Vec4 offsets, not byte offsets. Per view: three header words followed
    /// by all six row-major mip levels, four words per texel. Row zero lies
    /// along negative `up`; column zero lies along negative `right`.
    ///
    /// Header: [right.xyz,half_width], [up.xyz,half_height],
    /// [toward_viewer.xyz,center_y]. Texel: [source_point.xyz,branch_index],
    /// [source_normal.xyz,roughness], [linear_rgb,coverage], [anchor.xyz,height].
    pub data: Vec<[f32; 4]>,
    pub view_count: u32,
    pub size: u32,
    pub mip_count: u32,
    pub variant_stride: u32,
    pub view_stride: u32,
    /// Rest/growth-one +Y union coverage in a fixed ROOT_CELL square in XZ,
    /// measured independently at 64x64 sample centres. Not a density sum.
    pub top_coverage: [f32; VARIANTS],
    /// CPU construction duration only; never a GPU performance measurement.
    pub cook_ms: f64,
}

impl AggregateAtlas {
    /// Convert the CPU oracle into hardware-filterable moments. Geometry,
    /// coverage and mip selection are unchanged; only storage precision and
    /// interpolation implementation change. Boundary samples must still treat
    /// outside support as empty. ClampToEdge sampling needs the analytic valid
    /// bilinear weight at each explicit mip; it must not extend edge density.
    pub fn texture_data(&self) -> Result<AggregateTextureData, String> {
        let view_stride = HEADER_WORDS + TEXEL_WORDS * MIP_TEXELS;
        let views = VARIANTS * VIEW_COUNT as usize;
        if self.view_count != VIEW_COUNT || self.size != SIZE || self.mip_count != MIP_COUNT
            || self.view_stride as usize != view_stride
            || self.variant_stride as usize != view_stride * VIEW_COUNT as usize
            || self.data.len() != views * view_stride
            || self.data.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("invalid meadow atlas layout for texture conversion".into());
        }
        let headers = self.data.chunks_exact(view_stride)
            .flat_map(|view| view[..HEADER_WORDS].iter().copied()).collect();
        let mut result = AggregateTextureData { headers, layers: (views * 2) as u32, mips: Vec::new() };
        let mut side = SIZE as usize;
        let mut mip_offset = HEADER_WORDS;
        for _ in 0..MIP_COUNT {
            let texels = side * side;
            let count = views * 2 * texels * 4;
            let mut mip = AggregateTextureMip {
                width: side as u32, height: side as u32,
                material: Vec::with_capacity(count), surface: Vec::with_capacity(count),
            };
            for view in 0..views {
                let first = view * view_stride + mip_offset;
                for attribute in 0..2 {
                    for texel in self.data[first..first + texels * TEXEL_WORDS].chunks_exact(TEXEL_WORDS) {
                        let coverage = texel[2][3];
                        let branch = texel[0][3];
                        if !(0.0..=1.0).contains(&coverage)
                            || (coverage > 0.0 && (!(0.0..4.0).contains(&branch) || branch.fract() != 0.0))
                        {
                            return Err("invalid meadow atlas coverage or discrete branch".into());
                        }
                        let material = if attribute == 0 {
                            [texel[2][0] * coverage, texel[2][1] * coverage, texel[2][2] * coverage, coverage]
                        } else { texel[1].map(|v| v * coverage) };
                        let surface = texel[if attribute == 0 { 0 } else { 3 }];
                        for component in material { mip.material.push(finite_half(component)?); }
                        for component in surface { mip.surface.push(finite_half(component)?); }
                    }
                }
            }
            result.mips.push(mip);
            mip_offset += texels * TEXEL_WORDS;
            side = (side / 2).max(1);
        }
        Ok(result)
    }

    pub fn cook(geometry: &MeadowGeometry) -> Result<Self, String> {
        let start = Instant::now();
        validate_source(geometry)?;
        let view_stride = HEADER_WORDS + TEXEL_WORDS * MIP_TEXELS;
        let mut atlas = Self {
            data: Vec::with_capacity(VARIANTS * VIEW_COUNT as usize * view_stride),
            view_count: VIEW_COUNT,
            size: SIZE,
            mip_count: MIP_COUNT,
            variant_stride: view_stride as u32 * VIEW_COUNT,
            view_stride: view_stride as u32,
            top_coverage: top_coverage_validated(geometry),
            cook_ms: 0.0,
        };
        let center = DVec3::new(0.0, geometry.max_tuft_height as f64 * 0.5, 0.0);
        for variant in 0..VARIANTS {
            let vertices = source_vertices(geometry, variant);
            // Elevation-major, then azimuth atan2(x,z)=az*TAU/8.
            for elevation in [0.0f64, 30.0, 60.0] {
                for az in 0..8 {
                    let angle = az as f64 * std::f64::consts::TAU / 8.0;
                    let elev = elevation.to_radians();
                    // Round the basis to the exact values the GPU receives,
                    // then use f64 arithmetic for thin-triangle rasterization.
                    let toward = gpu_vec(DVec3::new(angle.sin() * elev.cos(), elev.sin(), angle.cos() * elev.cos()));
                    let right = gpu_vec(DVec3::new(angle.cos(), 0.0, -angle.sin()));
                    let up = gpu_vec(toward.cross(right));
                    let mut half = [1e-5f64; 2];
                    for vertex in vertices {
                        let relative = point(vertex) - center;
                        half[0] = half[0].max(relative.dot(right).abs());
                        half[1] = half[1].max(relative.dot(up).abs());
                    }
                    // Explicit outward padding survives f32 header rounding;
                    // do not clip a source extremum at the atlas boundary.
                    half = half.map(|v| ((v + v * 8.0 * f32::EPSILON as f64 + 1e-7) as f32) as f64);
                    atlas.data.extend([
                        word(right, half[0] as f32),
                        word(up, half[1] as f32),
                        word(toward, center.y as f32),
                    ]);
                    let projection = Projection { center, right, up, toward, half };
                    let samples = raster(vertices, projection, SIZE as usize * SUPERSAMPLE);
                    let mut level = resolve(&samples, SIZE as usize);
                    let mut side = SIZE as usize;
                    for _ in 0..MIP_COUNT {
                        for texel in &level { atlas.data.extend(texel.words()); }
                        if side > 1 { level = downsample(&level, side); side /= 2; }
                    }
                }
            }
        }
        if atlas.data.len() != VARIANTS * VIEW_COUNT as usize * view_stride
            || atlas.data.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("meadow aggregate produced an invalid atlas payload".into());
        }
        atlas.cook_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(atlas)
    }
}

/// Finite IEEE754 binary16, round-to-nearest ties-to-even. Atlas coordinates
/// are small, but reject overflow rather than producing GPU infinities. No new
/// dependency or optional GPU float32-filtering feature is needed.
fn finite_half(value: f32) -> Result<u16, String> {
    if !value.is_finite() || value.abs() > 65_504.0 {
        return Err("meadow atlas value cannot be stored as finite RGBA16Float".into());
    }
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127;
    if exponent < -25 { return Ok(sign); }
    let mantissa = (bits & 0x7fffff) | 0x800000;
    let rounded = |shift: u32| {
        let truncated = mantissa >> shift;
        let remainder = mantissa & ((1u32 << shift) - 1);
        let halfway = 1u32 << (shift - 1);
        truncated + u32::from(remainder > halfway || (remainder == halfway && truncated & 1 != 0))
    };
    let magnitude = if exponent < -14 { rounded((-exponent - 1) as u32) }
        else { (((exponent + 14) as u32) << 10) + rounded(13) };
    Ok(sign | magnitude as u16)
}

/// Cheap independent source-coverage cook for terrain substrate clients.
/// Does not create the directional atlas. Measures rest/growth-one geometry
/// clipped to the fixed .45m XZ cell; no terrain, wind or world pigment applied.
pub fn top_coverage(geometry: &MeadowGeometry) -> Result<[f32; VARIANTS], String> {
    validate_source(geometry)?;
    Ok(top_coverage_validated(geometry))
}

fn top_coverage_validated(geometry: &MeadowGeometry) -> [f32; VARIANTS] {
    let projection = Projection {
        center: DVec3::ZERO, right: DVec3::X, up: -DVec3::Z,
        toward: DVec3::Y, half: [ROOT_CELL as f64 * 0.5; 2],
    };
    std::array::from_fn(|variant| {
        let samples = raster(source_vertices(geometry, variant), projection, 64);
        samples.iter().filter(|sample| sample.covered).count() as f32 / (64 * 64) as f32
    })
}

fn validate_source(geometry: &MeadowGeometry) -> Result<(), String> {
    geometry.validate()?;
    for variant in 0..VARIANTS {
        for triangle in source_vertices(geometry, variant).chunks_exact(3) {
            if triangle[1..].iter().any(|vertex| vertex.position[3] != triangle[0].position[3]
                || vertex.anchor != triangle[0].anchor)
            {
                return Err("meadow aggregate requires one branch and basal anchor per source triangle".into());
            }
        }
    }
    Ok(())
}

fn source_vertices(geometry: &MeadowGeometry, variant: usize) -> &[GpuMeadowVertex] {
    let [start, count] = geometry.lod_ranges[variant][0];
    &geometry.vertices[start as usize..][..count as usize]
}
fn point(vertex: &GpuMeadowVertex) -> DVec3 { vec3(vertex.position) }
fn vec3(v: [f32; 4]) -> DVec3 { DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64) }
fn gpu_vec(v: DVec3) -> DVec3 { DVec3::new(v.x as f32 as f64, v.y as f32 as f64, v.z as f32 as f64) }
fn word(v: DVec3, w: f32) -> [f32; 4] { [v.x as f32, v.y as f32, v.z as f32, w] }

#[derive(Clone, Copy)]
struct Projection { center: DVec3, right: DVec3, up: DVec3, toward: DVec3, half: [f64; 2] }

#[derive(Clone, Copy)]
struct Sample { covered: bool, depth: f64, surface: Texel }
impl Default for Sample {
    fn default() -> Self { Self { covered: false, depth: f64::NEG_INFINITY, surface: Texel::default() } }
}

#[derive(Clone, Copy, Default)]
struct Texel {
    point: DVec3, normal: DVec3, color: DVec3, anchor: DVec3,
    roughness: f64, coverage: f64, branch: u8, height: f64,
}
impl Texel {
    fn words(self) -> [[f32; 4]; TEXEL_WORDS] {
        [word(self.point, self.branch as f32), word(self.normal, self.roughness as f32),
         word(self.color, self.coverage as f32), word(self.anchor, self.height as f32)]
    }
}

fn edge(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0]-a[0])*(p[1]-a[1]) - (b[1]-a[1])*(p[0]-a[0])
}

/// Orthographic opaque triangle union. Bounds are per triangle, not an
/// expensive all-triangles loop at every sample. Degenerate projections have
/// zero coverage; no minimum density/width is invented for edge-on blades.
fn raster(vertices: &[GpuMeadowVertex], projection: Projection, side: usize) -> Vec<Sample> {
    let mut result = vec![Sample::default(); side * side];
    for triangle in vertices.chunks_exact(3) {
        let positions = [point(&triangle[0]), point(&triangle[1]), point(&triangle[2])];
        let projected = positions.map(|p| {
            let r = p-projection.center;
            [(r.dot(projection.right)/projection.half[0]*0.5+0.5)*side as f64,
             (r.dot(projection.up)/projection.half[1]*0.5+0.5)*side as f64]
        });
        let area = edge(projected[0], projected[1], projected[2]);
        if !area.is_finite() || area.abs() <= 1e-14 { continue; }
        let min_x = projected.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
        let min_y = projected.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
        let max_x = projected.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max).ceil().clamp(0.0, side as f64) as usize;
        let max_y = projected.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max).ceil().clamp(0.0, side as f64) as usize;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let q = [x as f64+0.5,y as f64+0.5];
                let weights = [edge(projected[1],projected[2],q)/area,
                    edge(projected[2],projected[0],q)/area,edge(projected[0],projected[1],q)/area];
                if weights.iter().any(|v| *v < -1e-12) { continue; }
                let p = positions[0]*weights[0]+positions[1]*weights[1]+positions[2]*weights[2];
                let depth = p.dot(projection.toward);
                let sample = &mut result[y*side+x];
                if depth <= sample.depth { continue; }
                let mut normal = DVec3::ZERO; let mut color = DVec3::ZERO; let mut roughness = 0.0;
                for i in 0..3 {
                    normal += vec3(triangle[i].normal)*weights[i];
                    color += vec3(triangle[i].color)*weights[i];
                    roughness += triangle[i].color[3] as f64*weights[i];
                }
                if normal.length_squared() <= 1e-20 { normal = vec3(triangle[0].normal); }
                normal = normal.normalize();
                // Source two-sided BRDF orients real normals toward the view.
                // Do this before averaging to avoid antipodal cancellation.
                if normal.dot(projection.toward)<0.0 { normal = -normal; }
                *sample = Sample { covered: true, depth, surface: Texel {
                    point:p, normal, color, roughness,
                    anchor:vec3(triangle[0].anchor), height:f64::from(triangle[0].anchor[3]),
                    branch:triangle[0].position[3] as u8, coverage:1.0,
                }};
            }
        }
    }
    result
}

/// Coverage always averages the whole footprint. Surface moments are from
/// the most covered discrete branch only; tie breaking is stable by branch ID.
fn combine(values: &[Texel]) -> Texel {
    let mut masses = [0.0f64; 4];
    for value in values { masses[value.branch as usize] += value.coverage; }
    let branch = (1..4).fold(0, |best, i| if masses[i] > masses[best] { i } else { best });
    let weight = masses[branch];
    if weight <= 0.0 { return Texel::default(); }
    let mut out = Texel { branch:branch as u8, coverage:masses.iter().sum::<f64>() / values.len() as f64, ..Texel::default() };
    let mut fallback = DVec3::Y;
    for value in values.iter().filter(|v| v.branch as usize == branch && v.coverage > 0.0) {
        let w = value.coverage/weight;
        out.point += value.point*w; out.normal += value.normal*w;
        out.color += value.color*w; out.anchor += value.anchor*w;
        out.roughness += value.roughness*w; fallback = value.normal;
    }
    out.normal = if out.normal.length_squared()>1e-20 { out.normal.normalize() } else { fallback };
    // Nonlinear wind needs one actual blade descriptor. Averaging anchors or
    // heights invents a different lean hash and deforms a fictitious blade.
    // Keep material/normal moments weighted, but take point/anchor/height from
    // the strongest covered sample of the selected branch (first wins ties).
    // Legacy affine recipes retain their original byte-for-byte averaging.
    let mut representative = None::<&Texel>;
    for value in values.iter().filter(|v| v.branch as usize == branch && v.coverage > 0.0 && v.height > 0.0) {
        if representative.is_none_or(|old| value.coverage > old.coverage) { representative = Some(value); }
    }
    if let Some(value) = representative {
        out.point = value.point; out.anchor = value.anchor; out.height = value.height;
    }
    out
}

fn resolve(samples: &[Sample], side: usize) -> Vec<Texel> {
    let high = side*SUPERSAMPLE;
    let mut result = Vec::with_capacity(side*side);
    for y in 0..side {
        for x in 0..side {
            let values: [Texel; SUPERSAMPLE*SUPERSAMPLE] = std::array::from_fn(|i| {
                samples[(y*SUPERSAMPLE+i/SUPERSAMPLE)*high+x*SUPERSAMPLE+i%SUPERSAMPLE].surface
            });
            result.push(combine(&values));
        }
    }
    result
}

fn downsample(level: &[Texel], side: usize) -> Vec<Texel> {
    let next = side/2;
    (0..next*next).map(|i| {
        let x = (i%next)*2; let y = (i/next)*2;
        combine(&[level[y*side+x],level[y*side+x+1],level[(y+1)*side+x],level[(y+1)*side+x+1]])
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_value(bits: u16) -> f64 {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exponent = (bits >> 10) & 31;
        let fraction = bits & 1023;
        if exponent == 0 { sign * f64::from(fraction) * 2.0f64.powi(-24) }
        else { sign * f64::from(1024 + fraction) * 2.0f64.powi(i32::from(exponent) - 25) }
    }

    #[test]
    fn half_texture_encoding_roundtrips_finite_values_and_rounds_ties_even() {
        for bits in 0..=u16::MAX {
            if (bits >> 10) & 31 == 31 { continue; }
            assert_eq!(finite_half(half_value(bits) as f32).unwrap(), bits);
        }
        assert_eq!(finite_half(1.0 + 2.0f32.powi(-11)).unwrap(), 0x3c00);
        assert_eq!(finite_half(1.0 + 3.0 * 2.0f32.powi(-11)).unwrap(), 0x3c02);
        assert_eq!(finite_half(2.0f32.powi(-25)).unwrap(), 0);
        assert_eq!(finite_half(f32::from_bits(2.0f32.powi(-25).to_bits() + 1)).unwrap(), 1);
        for bad in [f32::INFINITY, f32::NAN, 65_520.0, -65_520.0] {
            assert!(finite_half(bad).is_err());
        }
    }

    #[test]
    fn clamped_filter_with_valid_weight_matches_empty_borders_at_every_mip() {
        // Independent scalar model applies to every premultiplied component.
        // No optional clamp-to-border sampler feature is required. Each mip
        // needs its own weight; applying one weight after trilinear is wrong.
        for side in [32usize,16,8,4,2,1] {
            let data: Vec<f64> = (0..side*side).map(|i| ((i*17+5)%29) as f64 / 29.0).collect();
            for u in [0.0,0.001,0.01,0.25,0.5,0.99,0.999,1.0] {
                for v in [0.0,0.01,0.37,0.99,1.0] {
                    let q = [u * side as f64 - 0.5, v * side as f64 - 0.5];
                    let start = [q[0].floor() as i32, q[1].floor() as i32];
                    let f = [q[0]-q[0].floor(),q[1]-q[1].floor()];
                    let mut empty = 0.0; let mut clamped = 0.0; let mut valid = 0.0;
                    for y in 0..2 { for x in 0..2 {
                        let ix = start[0]+x; let iy = start[1]+y;
                        let weight = if x == 0 { 1.0-f[0] } else { f[0] }
                            * if y == 0 { 1.0-f[1] } else { f[1] };
                        clamped += weight * data[iy.clamp(0,side as i32-1) as usize*side+ix.clamp(0,side as i32-1) as usize];
                        if ix >= 0 && iy >= 0 && ix < side as i32 && iy < side as i32 {
                            valid += weight; empty += weight * data[iy as usize*side+ix as usize];
                        }
                    }}
                    assert!((clamped * valid - empty).abs() < 1e-12);
                }
            }
        }
    }

    fn geometry(vertices: Vec<GpuMeadowVertex>) -> MeadowGeometry {
        let count = vertices.len() as u32;
        MeadowGeometry { vertices, lod_ranges:[[[0,count];3];3], lod_errors:[[0.0;3];3],
            branch_pivots:[[[0.0;4];4];3],max_tuft_radius:2.0,max_tuft_height:2.0,max_anchor_radius:2.0,max_blade_radius:3.0 }
    }
    fn vertex(p: [f32;3], normal: [f32;3], branch:u8, color:[f32;3]) -> GpuMeadowVertex {
        GpuMeadowVertex { position:[p[0],p[1],p[2],branch as f32],normal:[normal[0],normal[1],normal[2],0.0],
            color:[color[0],color[1],color[2],0.7],anchor:[0.0;4] }
    }
    fn quad_xz(x0:f32,x1:f32,z0:f32,z1:f32,y:f32,branch:u8,color:[f32;3])->Vec<GpuMeadowVertex> {
        [[x0,y,z0],[x1,y,z0],[x1,y,z1],[x0,y,z0],[x1,y,z1],[x0,y,z1]].map(|p|vertex(p,[0.0,1.0,0.0],branch,color)).to_vec()
    }

    #[test]
    fn top_union_preserves_empty_half_and_does_not_count_hidden_layers_twice() {
        let half=ROOT_CELL*0.5;
        let source=quad_xz(-half,0.0,-half,half,0.3,0,[0.1,0.4,0.2]);
        let mut layered=source.clone();layered.extend(quad_xz(-half,0.0,-half,half,0.2,1,[0.4,0.1,0.2]));
        assert_eq!(top_coverage(&geometry(source)).unwrap(),[0.5;3]);
        assert_eq!(top_coverage(&geometry(layered)).unwrap(),[0.5;3]);
    }

    #[test]
    fn nearest_opaque_surface_controls_branch_and_material_without_filling_holes() {
        let mut vertices=quad_xz(-0.2,0.0,-0.2,0.2,0.2,0,[0.1,0.2,0.3]);
        vertices.extend(quad_xz(-0.2,0.0,-0.2,0.2,0.4,2,[0.7,0.4,0.1]));
        let samples=raster(&vertices,Projection{center:DVec3::ZERO,right:DVec3::X,up:-DVec3::Z,toward:DVec3::Y,half:[0.2;2]},32);
        assert_eq!(samples.iter().filter(|s|s.covered).count(),512);
        for s in samples.iter().filter(|s|s.covered) {
            assert_eq!(s.surface.branch,2);assert!((s.surface.point.y-0.4).abs()<1e-6);
            assert!((s.surface.color.x-0.7).abs()<1e-6);
        }
    }

    #[test]
    fn mip_coverage_conserves_union_and_attributes_use_an_integer_branch() {
        let values=[Texel{point:DVec3::X,anchor:DVec3::X,normal:DVec3::Y,color:DVec3::X,roughness:0.7,coverage:1.0,branch:0,height:0.0},
            Texel{point:DVec3::Z,anchor:DVec3::Z,normal:DVec3::Z,color:DVec3::Z,roughness:0.9,coverage:0.5,branch:3,height:0.0},
            Texel::default(),Texel::default()];
        let coarse=downsample(&values,2)[0];assert_eq!(coarse.coverage,0.375);
        assert_eq!(coarse.branch,0);assert_eq!(coarse.point,DVec3::X);assert_eq!(coarse.anchor,DVec3::X);
        assert_eq!(coarse.normal,DVec3::Y);assert_eq!(coarse.color,DVec3::X);
    }

    #[test]
    fn windswept_mip_keeps_one_discrete_blade_descriptor_and_weighted_material() {
        let a = Texel { point: DVec3::new(0.1, 0.2, 0.3), anchor: DVec3::new(0.125, 0.0, -0.25),
            height: 0.75, normal: DVec3::Y, color: DVec3::X, roughness: 0.8, coverage: 1.0, branch: 1 };
        let b = Texel { point: DVec3::new(0.4, 0.5, 0.6), anchor: DVec3::new(-0.125, 0.0, 0.25),
            height: 0.5, normal: DVec3::Z, color: DVec3::Z, roughness: 0.6, coverage: 0.5, branch: 1 };
        let combined = combine(&[a, b, Texel::default(), Texel::default()]);
        assert_eq!(combined.coverage, 0.375);
        assert_eq!(combined.point, a.point); assert_eq!(combined.anchor, a.anchor); assert_eq!(combined.height, a.height);
        assert!(combined.color.distance((DVec3::X * 2.0 + DVec3::Z) / 3.0) < 1e-12);
        assert!(combined.normal.distance((DVec3::Y * 2.0 + DVec3::Z).normalize()) < 1e-12);
        let words = combined.words();
        for value in words[3] { assert_eq!(half_value(finite_half(value).unwrap()) as f32, value); }
        let next = combine(&[combined, b]);
        assert_eq!(next.point, b.point); assert_eq!(next.anchor, b.anchor); assert_eq!(next.height, b.height);
    }

    #[test]
    fn windswept_atlas_all_mips_retain_real_quantized_anchor_and_height() {
        let geometry = MeadowGeometry::cook_windswept(73, 0.0, crate::foliage_assembly::MeadowBladeBudget::Blades120).unwrap();
        let atlas = AggregateAtlas::cook(&geometry).unwrap();
        let anchors: std::collections::BTreeSet<_> = geometry.vertices.iter().map(|v| v.anchor.map(f32::to_bits)).collect();
        for view in atlas.data.chunks_exact(atlas.view_stride as usize) {
            for texel in view[3..].chunks_exact(4).filter(|t| t[2][3] > 0.0) {
                assert!(anchors.contains(&texel[3].map(f32::to_bits)), "mip must not invent a new nonlinear blade");
                for value in texel[3] { assert_eq!(half_value(finite_half(value).unwrap()) as f32, value); }
            }
        }
        atlas.texture_data().unwrap();
    }

    #[test]
    fn rejects_interpolated_branch_or_anchor_inside_source_triangle() {
        let mut source=quad_xz(-0.2,0.2,-0.2,0.2,0.2,0,[0.1,0.3,0.1]);
        source[1].position[3]=1.0;
        assert!(top_coverage(&geometry(source.clone())).unwrap_err().contains("one branch"));
        source[1].position[3]=0.0;source[1].anchor[0]=0.01;
        assert!(AggregateAtlas::cook(&geometry(source)).unwrap_err().contains("basal anchor"));
    }

    #[test]
    fn real_meadow_atlas_has_exact_abi_finite_moments_and_coverage_preserving_mips() {
        let source=MeadowGeometry::cook(0,0.0).unwrap();
        let before=bytemuck::cast_slice::<GpuMeadowVertex,u8>(&source.vertices).to_vec();
        let atlas=AggregateAtlas::cook(&source).unwrap();
        assert_eq!(atlas.view_stride,5463);assert_eq!(atlas.variant_stride,131112);
        assert_eq!(atlas.data.len()*16,6_293_376);
        assert_eq!(before,bytemuck::cast_slice::<GpuMeadowVertex,u8>(&source.vertices));
        assert_eq!(atlas.top_coverage,top_coverage(&source).unwrap());
        let textures = atlas.texture_data().unwrap();
        assert_eq!(textures.layers,144);
        assert_eq!(textures.headers.len(),216);
        assert_eq!(textures.mips.len(),6);
        assert_eq!(textures.payload_bytes(),3_148_416);
        let mut source_offset = HEADER_WORDS;
        for (mip_index,mip) in textures.mips.iter().enumerate() {
            let side = 32usize >> mip_index;
            assert_eq!([mip.width,mip.height],[side as u32;2]);
            assert_eq!(mip.material.len(),144*side*side*4);
            assert_eq!(mip.surface.len(),mip.material.len());
            for view in 0..72 {
                assert_eq!(&textures.headers[view*3..][..3],&atlas.data[view*atlas.view_stride as usize..][..3]);
                for pixel in 0..side*side {
                    let words=&atlas.data[view*atlas.view_stride as usize+source_offset+pixel*4..][..4];
                    for attribute in 0..2 {
                        let address=((view*2+attribute)*side*side+pixel)*4;
                        let coverage=words[2][3];
                        let material=if attribute==0 {
                            [words[2][0]*coverage,words[2][1]*coverage,words[2][2]*coverage,coverage]
                        } else { words[1].map(|v|v*coverage) };
                        let surface=words[if attribute==0 {0} else {3}];
                        for component in 0..4 {
                            for (actual,expected) in [(mip.material[address+component],material[component]),(mip.surface[address+component],surface[component])] {
                                let expected=f64::from(expected);
                                assert!((half_value(actual)-expected).abs()<=expected.abs()*0.0005+3e-8);
                            }
                        }
                        if attribute==0 { assert_eq!(half_value(mip.surface[address+3]),f64::from(words[0][3])); }
                    }
                }
            }
            source_offset+=side*side*4;
        }
        for coverage in atlas.top_coverage { assert!(coverage>0.0 && coverage<1.0); }
        for layer in atlas.data.chunks_exact(atlas.view_stride as usize) {
            let mut offset=HEADER_WORDS;let mut side=SIZE as usize;let mut expected:Option<f64>=None;
            for _ in 0..MIP_COUNT {
                let mut sum=0.0f64;
                for texel in layer[offset..offset+side*side*TEXEL_WORDS].chunks_exact(TEXEL_WORDS) {
                    let coverage=texel[2][3];assert!((0.0..=1.0).contains(&coverage));sum+=coverage as f64;
                    if coverage>0.0 {
                        assert!((0.0..4.0).contains(&texel[0][3]) && texel[0][3].fract()==0.0);
                        assert!((vec3(texel[1]).length()-1.0).abs()<1e-5);
                        assert!(texel[2][..3].iter().all(|v|(0.0..=1.0).contains(v)));
                    }
                }
                let mean=sum/(side*side) as f64;
                if let Some(fine)=expected { assert!((mean-fine).abs()<1e-7,"coverage changes across mip levels"); }
                else { expected=Some(mean); }
                offset+=side*side*TEXEL_WORDS;side=(side/2).max(1);
            }
            assert_eq!(offset,atlas.view_stride as usize);
        }
        eprintln!("meadow aggregate: top_coverage={:?}, resident_bytes={}, CPU_cook_ms={:.3}",atlas.top_coverage,atlas.data.len()*16,atlas.cook_ms);
    }
}
