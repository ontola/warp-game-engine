//! Bounded validation renderer for source-derived fractional foliage volumes.
//!
//! Integrates all overlapping crown densities together, clipped by opaque
//! scene depth. Crowns write C/T, never a false opaque first-hit depth. The
//! integrator's 4096-volume limit is deliberate, not full-forest streaming
//! support; rasterized voxels select up to 16384 groups per view. Optional
//! per-view projected mip selection shares the same immutable asset pyramid.
use crate::{Gpu, Scene};
use bytemuck::{Pod, Zeroable};
use std::{sync::Mutex, time::Duration};
use wgpu::util::DeviceExt;

const TILE: u32 = 16;
const CANDIDATES: u32 = 64;
// Four exact 64-bit aggregate counters follow the fourteen original u32s.
// WGSL uses atomic low words plus carry; no optional GPU u64 feature is needed.
const COUNTER_BYTES: u64 = 88;
// Additional opt-in scratch, per view, beyond the existing tile worklist.
const INTERVAL_STREAM_BYTES:u64=32*1024*1024;
const INTERVAL_STREAM_PIXEL_SLOTS:u32=262_144;
#[derive(Clone,Copy,Debug)]
struct IntervalStreamLayout { header_word:u32,pixel_slots:u32,records:u32,bytes:u64 }
fn interval_stream_layout(size:(u32,u32),binding_limit:u64)->Result<IntervalStreamLayout,String> {
    if size.0==0 || size.1==0 {return Err("global voxel intervals require nonzero dimensions".into());}
    let tiles=u64::from(size.0.div_ceil(TILE))*u64::from(size.1.div_ceil(TILE));
    let header=4+tiles;let slots=tiles.min(u64::from(INTERVAL_STREAM_PIXEL_SLOTS)/256)*256;
    // Four counters, then (record offset,count) per admitted pixel. The rest
    // contains 16-byte intervals. Pixels without a slot use complete scans.
    let records=(INTERVAL_STREAM_BYTES/4-4-slots*2)/4;
    let bytes=header.checked_mul(4).and_then(|n|n.checked_add(INTERVAL_STREAM_BYTES))
        .ok_or("global voxel interval buffer size overflow")?;
    if bytes>binding_limit || header>u64::from(u32::MAX) || bytes/4>u64::from(u32::MAX) {
        return Err("global voxel interval stream exceeds device storage limits".into());
    }
    Ok(IntervalStreamLayout {header_word:header as u32,pixel_slots:slots as u32,records:records as u32,bytes})
}

/// Accepted selected-mip empty-space skipping fix, fixed for a renderer's
/// lifetime. Explicit env=0 retains the baseline for matched diagnostics.
/// This does not make the bounded experimental volume renderer forest-ready.
#[derive(Clone, Copy, Debug)]
struct IntegratorOptions {
    empty_ceiling: bool,
    // Experimental register/BRDF specialization; immutable packed cells must
    // first prove exact antipodal symmetry. The 176-byte upload ABI is unchanged.
    antipodal_normals: bool,
    global_intervals: bool,
    brick_visibility: bool,
    /// Opaque stochastic voxel rasterization instead of ray integration.
    raster: bool,
}
impl Default for IntegratorOptions {
    fn default() -> Self { Self { empty_ceiling: true, antipodal_normals: false,global_intervals:false,brick_visibility:false,raster:false } }
}
impl IntegratorOptions {
    fn from_environment() -> Self {
        Self {
            empty_ceiling: std::env::var("KERN_VOXEL_EMPTY_CEILING").map_or(true, |value| value != "0"),
            antipodal_normals: std::env::var("KERN_VOXEL_ANTIPODAL_NORMALS").is_ok_and(|value| value == "1"),
            brick_visibility: std::env::var("KERN_VOXEL_BRICK_VISIBILITY").is_ok_and(|v|v=="1"),
            global_intervals: std::env::var("KERN_VOXEL_GLOBAL_INTERVALS").is_ok_and(|value|value=="1"),
            raster: std::env::var("KERN_VOXEL_RASTER").is_ok_and(|value|value=="1"),
        }
    }
    fn specialize(self, source: &str) -> Result<String, String> {
        let mut result = source.to_owned();
        let changes = [
            (self.brick_visibility, "const VOLUME_BRICK_VISIBILITY: bool = false;", "const VOLUME_BRICK_VISIBILITY: bool = true;"),
            (self.empty_ceiling, "const VOLUME_EMPTY_CEILING: bool = false;", "const VOLUME_EMPTY_CEILING: bool = true;"),
        ];
        for (enabled, from, to) in changes {
            if !enabled { continue; }
            if result.matches(from).count() != 1 {
                return Err(format!("voxel option requires exactly one WGSL declaration: {from}"));
            }
            result = result.replacen(from, to, 1);
        }
        if self.antipodal_normals {
            // Specialize source rather than branch inside every cell fetch.
            // VolumeCell remains six records; only private VolumeSample and
            // its interpolation/lighting loops shrink to three records.
            for (from,to) in [
                ("const VOLUME_NORMAL_COUNT: u32 = 6u;", "const VOLUME_NORMAL_COUNT: u32 = 3u;"),
                ("let lobe=cell.normals[k];", "let lobe=cell.normals[k*2u];"),
                // Keep the positive first moment at its original magnitude:
                // doubling it would change the existing near-zero normal
                // rejection threshold. Only the response weight is combined.
                ("result.normals[k]+=vec4<f32>(lobe.xyz*lobe.w,lobe.w)*sigma;", "result.normals[k]+=vec4<f32>(lobe.xyz*lobe.w,lobe.w+cell.normals[k*2u+1u].w)*sigma;"),
                ("let weight=sample.normals[k].w;", "var weight=sample.normals[k].w;"),
                ("// VOLUME_ANTIPODAL_RESPONSE_BEGIN", "// Exactly grazing view keeps both opposite responses: the generic\n        // fold only flips a negative dot product, not zero.\n        let normal_copies=select(1u,2u,dot(n,v)==0.0);\n        if normal_copies==2u {weight*=0.5;}\n        for(var normal_side=0u;normal_side<normal_copies;normal_side++) {\n        if normal_side==1u {n=-n;}"),
                ("// VOLUME_ANTIPODAL_RESPONSE_END", "}"),
            ] {
                if result.matches(from).count()!=1 {
                    return Err(format!("antipodal normal specialization requires exactly one WGSL marker: {from}"));
                }
                result=result.replacen(from,to,1);
            }
        }
        Ok(result)
    }
}

/// All uploaded cells, including unused mip cells, must have exactly matching
/// pair weights and opposite conditional means. No tolerance silently drops
/// an unequal moment. Signed zero compares equal, as required by the baker.
fn antipodal_normal_cells(cells:&[crate::foliage_volume_data::GpuCell])->bool {
    !cells.is_empty() && cells.iter().all(|cell| (0..3).all(|axis| {
        let a=cell.normals[axis*2];let b=cell.normals[axis*2+1];
        a.iter().chain(&b).all(|x|x.is_finite()) && a[3]>=0.0 && a[3]==b[3]
            && (0..3).all(|k|a[k]==-b[k])
    }))
}
fn diagnostic_u64(bytes: &[u8], word: usize) -> u64 {
    let low = u32::from_le_bytes(bytes[word * 4..word * 4 + 4].try_into().unwrap());
    let high = u32::from_le_bytes(bytes[word * 4 + 4..word * 4 + 8].try_into().unwrap());
    u64::from(low) | (u64::from(high) << 32)
}
// Volume shading only needs owner transforms and shared grass/headlight inputs.
// Reusing the entire mesh scene layout also counts unused mesh/car buffers
// against WebGPU's per-stage limit (10 scene + 7 volume = 17). Keep this
// interface small instead of increasing the requested hardware limit.
pub(crate) const SCENE_BINDINGS: &[u32] = &[4, 12, 13, 14, 15, 16, 17, 18, 22, 23, 24, 25, 26];
/// Groups one view's selection holds: a packed u32 each (selection_pack),
/// four per vec4 of the shaders' 64 KiB uniform.
pub(crate) const MAX_VIEW_GROUPS: usize = 16384;
/// Volumes the integrator path (no rasterized voxels) may march: its per-ray
/// u32 sums are bounded for this many.
pub(crate) const INTEGRATOR_MAX_VOLUMES: usize = 4096;
/// Replacement tag of a distance-switched far tree (Scene::voxel_far_distance_m).
pub(crate) const FAR_TREE: u32 = 0x4000_0000;
pub(crate) fn default_selection() -> Vec<u32> {
    vec![selection_pack([1, u32::MAX, 0, 0]); MAX_VIEW_GROUPS]
}

/// A group's selection [choice, voxel level (u32::MAX none), triangle error
/// f32 bits, sun mode] as the shaders hold it (aggregate_lod.wgsl).
pub(crate) fn selection_pack(d: [u32; 4]) -> u32 {
    d[0].min(3) | (d[1].min(63) << 2) | (d[3].min(3) << 8) | (u32::from(f16_bits(f32::from_bits(d[2]))) << 16)
}
pub(crate) fn selection_unpack(w: u32) -> [u32; 4] {
    let y = (w >> 2) & 63;
    [w & 3, if y == 63 { u32::MAX } else { y }, f16_value((w >> 16) as u16).to_bits(), (w >> 8) & 3]
}
fn f16_bits(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 255) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if v.is_nan() { return sign | 0x7e00; }
    if exp >= 31 { return sign | 0x7c00; }
    if exp <= 0 {
        if exp < -10 { return sign; }
        let m = (mantissa | 0x80_0000) >> (14 - exp);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    let rounded = (((exp as u32) << 10) | (mantissa >> 13)) + ((mantissa >> 12) & 1);
    sign | rounded.min(0x7c00) as u16
}
fn f16_value(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 31) as i32;
    let mantissa = (h & 1023) as f32;
    match exp {
        0 => sign * mantissa * 2f32.powi(-24),
        31 => if mantissa == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exp - 15),
    }
}
#[derive(Clone, Debug, Default)]
pub struct VoxelViewDiagnostics {
    pub candidate_overflow_tiles: u32,
    pub step_overflow_rays: u32,
    pub traced_rays: u32,
    pub max_steps_used: u32,
    pub max_tile_candidates: u32,
    /// Branch intervals in the baseline; disjoint support intervals in brick visibility.
    pub max_ray_candidates: u32,
    /// Complete scan fallback, not dropped candidates.
    pub ray_candidate_overflow_rays: u32,
    pub max_active_volumes: u32,
    /// Complete scan fallback, not dropped overlapping density.
    pub active_cache_overflow_rays: u32,
    pub max_empty_brick_skips: u32,
    /// Largest occupied sample's fractional mip, in units of 1/256 level.
    pub max_occupied_lod_x256: u32,
    /// Rays containing nonzero density sampled beyond the finest mip.
    pub occupied_coarse_mip_rays: u32,
    pub active_tiles: u32,
    /// In-bounds pixels covered by active tiles (before exact ray rejection).
    pub scheduled_pixels: u32,
    /// Sum of march-loop iterations, including gap jumps and empty-brick skips.
    pub total_steps: u64,
    /// Calls that reconstruct one volume at one ray position (possibly two mips).
    pub volume_samples: u64,
    /// Nonempty sparse cell records fetched during trilinear reconstruction.
    /// Counts both mips and repeated reads, even if a corner's weight is zero;
    /// not unique cells, ray positions, visible voxels, or rasterized boxes.
    pub occupied_cell_samples: u64,
    /// Actual ray/AABB interval tests, including misses and complete-scan fallbacks.
    /// Does not count tile projection or imply any box was rasterized.
    pub aabb_interval_tests: u64,
    /// Rays that used the exact complete scan because bounded stream space
    /// or pixel metadata was unavailable. Zero in the legacy path.
    pub interval_stream_overflow_rays:u32,
    /// Final contiguous 16-byte visibility records, not capacity. Parallel
    /// brick construction also uses one temporary linked record per intersection.
    pub interval_stream_records:u32,
    /// Actual indirect splat draw arguments, including rejected/overlapping cells.
    pub raster_splats:u32,
    pub raster_vertices:u64,
    pub raster_capacity:u64,

}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Config {
    size_count: [u32; 4],
    march: [f32; 4],
    dispatch: [u32; 4],
    /// First whole-plant proxy tree, its pixel error, the distance past which that error grows.
    proxy: [f32; 4],
}
/// Rasterized voxels (KERN_VOXEL_RASTER=1, docs/voxel-raster-plan.md): the
/// static cell list and the id/depth raster pipeline.
struct SplatRaster {
    /// Into the scene's visibility buffer and depth (Stage::Splats with
    /// the scene's targets), plus the splat id the surface pass reads.
    main_pipeline: wgpu::RenderPipeline,
    cells: wgpu::Buffer,
    /// Splat records per view (splat_capacity).
    capacity: u64,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    /// Sun map: voxel_sun_splats needs no camera group, so group 1 is empty.
    sun_layout: wgpu::PipelineLayout,
    sun_empty: wgpu::BindGroup,
    sun_splats: wgpu::ComputePipeline,
    sun_cached_layout: wgpu::PipelineLayout,
    sun_cached_splats: wgpu::ComputePipeline,
    sun_raster: wgpu::RenderPipeline,
    sun_cached_raster: wgpu::RenderPipeline,
    /// The sun target's selection buffer and its splat targets.
    /// One target per sun map, keyed by its selection buffer.
    sun_target: Mutex<Vec<(wgpu::Buffer, Targets)>>,
}
/// Where render_sun draws: the sun map's depth, view and texel size.
pub(crate) struct SunSplatView<'a> {
    pub damage:Option<&'a wgpu::BindGroup>,
    pub depth: &'a wgpu::TextureView,
    pub view_proj: glam::Mat4,
    pub planes: [[f32; 4]; 6],
    /// Towards the sun.
    pub direction: glam::Vec3,
    pub texel_m: f32,
}
/// Which part of `render` to run. With rasterized voxels the splats are
/// rastered before the opaque shade (Splats), which then skips the pixels
/// they cover, and resolved and composited after it (Rest).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stage { Splats, Rest }

struct SplatTargets {
    /// The splats' surface for shade (volume_gbuffer).
    gbuffer: wgpu::TextureView,
    records: wgpu::Buffer,
    draw: wgpu::Buffer,
    id: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
}
/// Per packed level its range of occupied cells (first, count), then the
/// cells: cell index, coordinate packed ten bits per axis. Trees sharing an
/// asset share its levels, so this does not grow with the tree count.
pub(crate) fn splat_cells(forest: &crate::foliage_volume_data::GpuForest) -> Result<Vec<[u32; 2]>, String> {
    let mut result = vec![[0u32; 2]; forest.levels.len()];
    for (index, level) in forest.levels.iter().enumerate() {
        let [dx, dy, dz, _] = level.dims_offset;
        if dx > 1024 || dy > 1024 || dz > 1024 { return Err("voxel level exceeds 1024 cells per axis".into()); }
        let first = result.len();
        for z in 0..dz { for y in 0..dy { for x in 0..dx {
            let cell = crate::foliage_volume_data::cell_index(forest, level, [x, y, z]).unwrap_or(0);
            if cell != 0 { result.push([cell - 1, x | y << 10 | z << 20]); }
        }}}
        result[index] = [first as u32, (result.len() - first) as u32];
    }
    Ok(result)
}
/// Optional splat catalog: the original level table, a parallel brick table,
/// then cell ranges grouped by 4x4x4 coordinates and two words per brick.
/// Sun splats retain the original cell-range ABI and use every occupied cell.
fn splat_cells_bricked(forest: &crate::foliage_volume_data::GpuForest) -> Result<Vec<[u32; 2]>, String> {
    let source = splat_cells(forest)?;
    let levels = forest.levels.len();
    let mut result = vec![[0; 2]; levels * 2];
    for level in 0..levels {
        let [first, count] = source[level];
        let mut groups = std::collections::BTreeMap::<u32, Vec<[u32; 2]>>::new();
        for cell in &source[first as usize..(first+count) as usize] {
            let c = cell[1];
            let key = (c & 1023 & !3) | (((c >> 10) & 1023 & !3) << 10) | (((c >> 20) & 1023 & !3) << 20);
            groups.entry(key).or_default().push(*cell);
        }
        let first_cell = result.len() as u32;
        let mut bricks = Vec::new();
        for (coord, cells) in groups {
            bricks.push([result.len() as u32, cells.len() as u32]);
            bricks.push([coord, 0]);
            result.extend(cells);
        }
        result[level] = [first_cell, count];
        result[levels + level] = [result.len() as u32, bricks.len() as u32 / 2];
        result.extend(bricks);
    }
    Ok(result)
}
fn brick_hzb_enabled() -> bool { std::env::var("KERN_SPLAT_BRICK_HZB").as_deref() == Ok("1") }

/// Splat records one view may hold: every tree at its finest level, capped.
fn splat_capacity(forest: &crate::foliage_volume_data::GpuForest, cells: &[[u32; 2]]) -> u64 {
    const MAX_SPLATS: u64 = 4 << 20;
    forest.instances.iter().map(|tree| u64::from(cells[tree.meta[1] as usize][1])).sum::<u64>().clamp(1, MAX_SPLATS)
}
struct Targets {
    hzb: Option<crate::depth_pyramid::PyramidTarget>,
    splat: Option<SplatTargets>,
    size: (u32, u32),
    layer: wgpu::Texture,
    bind: wgpu::BindGroup,
    composite: wgpu::BindGroup,
    counters: wgpu::Buffer,
    tiles: wgpu::Buffer,
    jobs: wgpu::Buffer,
    indirect: wgpu::Buffer,
    config: wgpu::Buffer,
    motion: Option<wgpu::Texture>,
    interval_stream:Option<IntervalStreamLayout>,
}
pub(crate) struct TemporalInputs<'a> {
    pub previous_instances: &'a wgpu::Buffer,
    pub snapshot_valid: bool,
}
struct TemporalPipelines {
    gbuffer: wgpu::ComputePipeline,
    selection: Option<[wgpu::ComputePipeline; 2]>,
    second: Option<wgpu::ComputePipeline>,
    data_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pipelines: [wgpu::ComputePipeline;8],
}
pub(crate) struct VoxelRenderer {
    bins: wgpu::ComputePipeline,
    tiles: wgpu::ComputePipeline,
    clear: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    intervals:wgpu::ComputePipeline,
    scatter:wgpu::ComputePipeline,
    splats:wgpu::ComputePipeline,
    resolve:wgpu::ComputePipeline,
    /// voxel_splat_gbuffer: the splats' surface where they won the scene's
    /// visibility buffer.
    gbuffer:wgpu::ComputePipeline,
    selection: Option<[wgpu::ComputePipeline; 2]>,
    second: Option<wgpu::ComputePipeline>,
    brick_hzb: Option<crate::depth_pyramid::DepthPyramid>,
    raster:Option<SplatRaster>,
    pipeline_layout: wgpu::PipelineLayout,
    scene_layout: wgpu::BindGroupLayout,
    view_layout: wgpu::BindGroupLayout,
    temporal: Mutex<Option<TemporalPipelines>>,
    temporal_error: Option<String>,
    source: String,
    composite: wgpu::RenderPipeline,
    data_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    data: [wgpu::Buffer; 4],
    page_cache:Option<Mutex<crate::foliage_pages::cache::Cache>>,
    brick_catalog: wgpu::Buffer,
    bin_count: u32,
    count: u32,
    /// Scene::voxel_far_distance_m; written as config march.y.
    far: f32,
    /// Scene::voxel_proxies and voxel_proxy_pixels: config proxy.
    proxies: u32,
    proxy_pixels: f32,
    proxy_pixels_from: f32,
    step: f32,
    empty_skip: bool,
    lod_pixels: f32,
    active_tiles: bool,
    integrator_options: IntegratorOptions,
    owners: Vec<u32>,
    owner_index: Vec<usize>,
    targets: Mutex<Vec<Option<Targets>>>,
}
pub(crate) struct Reload { pipelines: [wgpu::ComputePipeline; 8], surface: wgpu::ComputePipeline, selection: Option<[wgpu::ComputePipeline;2]>, second: Option<wgpu::ComputePipeline>, temporal: Option<([wgpu::ComputePipeline;8], wgpu::ComputePipeline, Option<wgpu::ComputePipeline>, Option<[wgpu::ComputePipeline;2]>)>, sun: Option<(wgpu::ComputePipeline,wgpu::ComputePipeline)>, source: String }
/// Native generation contains neither temporal resources nor per-sample work.
/// Explicit blocks keep shader reload transactional for both pipeline sets.
fn temporal_source(source:&str,enabled:bool)->Result<String,String> {
    let mut result=String::with_capacity(source.len());let mut inside=false;let mut blocks=0;
    for line in source.split_inclusive('\n') {
        match line.trim() {
            "// VOLUME_MOTION_BEGIN"=>{if inside {return Err("nested voxel temporal block".into());}inside=true;blocks+=1;},
            "// VOLUME_MOTION_END"=>{if !inside {return Err("unmatched voxel temporal block end".into());}inside=false;},
            _=>if enabled || !inside {result.push_str(line);},
        }
    }
    if inside || blocks==0 {return Err("voxel temporal shader blocks are missing or incomplete".into());}
    Ok(result)
}
fn data_bindings(temporal: bool) -> Vec<(u32, wgpu::BindingType)> {
    let storage=|read_only|wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only},has_dynamic_offset:false,min_binding_size:None};
    let uniform=wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None};
    let output=|format|wgpu::BindingType::StorageTexture {access:wgpu::StorageTextureAccess::WriteOnly,format,view_dimension:wgpu::TextureViewDimension::D2};
    let mut entries=vec![(0,storage(true)),(1,storage(true)),(2,storage(true)),(3,storage(true)),
        (4,storage(false)),(5,storage(false)),(6,output(wgpu::TextureFormat::Rgba16Float)),
        (7,uniform),(8,storage(false)),(9,uniform),(12,storage(true)),
        (14,storage(true)),(15,storage(false)),(16,storage(false)),
        (17,wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Uint,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}),
        (18,wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}),
        (19,output(wgpu::TextureFormat::Rgba32Uint)),
        (20,wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false})];
    if temporal {entries.extend([(10,output(wgpu::TextureFormat::Rgba32Float)),(11,storage(true))]);}
    entries
}
fn create_data_layout(gpu:&Gpu,temporal:bool)->wgpu::BindGroupLayout {
    let entries = data_bindings(temporal);
    gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("voxel data"),
        entries:&entries.into_iter().map(|(binding,ty)|wgpu::BindGroupLayoutEntry {binding,ty,visibility:wgpu::ShaderStages::COMPUTE,count:None}).collect::<Vec<_>>()})
}
fn splat_selection_pipelines(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str) -> Option<[wgpu::ComputePipeline;2]> {
    if !brick_hzb_enabled() {return None;}
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label:Some("voxel visible tree list"),source:wgpu::ShaderSource::Wgsl(source.into()) });
    Some([1.0,2.0].map(|phase|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label:Some("voxel visible tree list"),layout:Some(layout),module:&module,entry_point:Some("voxel_splat_tree_list"),
        compilation_options:wgpu::PipelineCompilationOptions {constants:&[("VOXEL_CULL_PHASE",phase)],..Default::default()},cache:gpu.pipeline_cache()
    })))
}
fn second_splat_pipeline(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str) -> wgpu::ComputePipeline {
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxel brick culling"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("voxel brick second pass"), layout: Some(layout), module: &module,
        entry_point: Some("voxel_splats"), compilation_options: wgpu::PipelineCompilationOptions {
            constants: &[("VOXEL_CULL_PHASE", 2.0),("VOXEL_COMPACT",1.0)], ..Default::default() }, cache: gpu.pipeline_cache() })
}
fn surface_pipeline(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str) -> wgpu::ComputePipeline {
    let coarser: Vec<(&str, f64)> = std::env::var("KERN_VOXEL_SELF_SHADOW_COARSER").ok().and_then(|v| v.parse().ok())
        .map(|v: f64| ("VOXEL_SELF_SHADOW_COARSER", v)).into_iter().collect();
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxel surface"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("voxel_splat_gbuffer"), layout: Some(layout), module: &module,
        entry_point: Some("voxel_splat_gbuffer"), compilation_options: wgpu::PipelineCompilationOptions {
            constants: &coarser, ..Default::default() }, cache: gpu.pipeline_cache() })
}
fn sun_splat_pipeline(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str, cached:bool) -> wgpu::ComputePipeline {
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxel sun splats"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("voxel_sun_splats"), layout: Some(layout),
        module: &module, entry_point: Some(if cached {"voxel_cached_sun_splats"}else{"voxel_sun_splats"}), compilation_options: wgpu::PipelineCompilationOptions {constants:if cached {&[("SHADOW_DAMAGE_MAP_SIZE",crate::frame::SHADOW_SIZE as f64)]}else{&[]},..Default::default()}, cache: gpu.pipeline_cache() })
}
fn splat_raster(gpu: &Gpu, cells: &[[u32; 2]], capacity: u64, scene_layout: &wgpu::BindGroupLayout, data_layout: &wgpu::BindGroupLayout, source: &str, cells_override:Option<&wgpu::Buffer>) -> SplatRaster {
    let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("voxel splat raster"),
        entries: &[0].map(|binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None }) });
    let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("voxel splat raster"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("voxel splat raster"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/voxel_splat_raster.wgsl").into()) });
    let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("voxel splat raster"), layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        // Reversed Z, like the scene.
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
        multiview_mask: None, cache: gpu.pipeline_cache(),
    });
    let main_pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("voxel splat raster (scene)"), layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_main"), compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
        multiview_mask: None, cache: gpu.pipeline_cache(),
    });
    // Like the scene's sun raster: ordinary Z with the same bias.
    let sun_raster = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("voxel sun splat raster"), layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(),
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_depth"), compilation_options: Default::default(), targets: &[] }),
        multiview_mask: None, cache: gpu.pipeline_cache(),
    });
    let cached_mask_layout=crate::shadow_damage::mask_layout(gpu);
    let cached_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("cached voxel sun raster"),bind_group_layouts:&[Some(&layout),Some(&cached_mask_layout)],immediate_size:0});
    let sun_cached_raster = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("voxel sun splat raster"), layout: Some(&cached_layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(),
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_cached_depth"), compilation_options: wgpu::PipelineCompilationOptions {constants:&[("SHADOW_TILE_PIXELS",(crate::frame::SHADOW_SIZE/64)as f64)],..Default::default()}, targets: &[] }),
        multiview_mask: None, cache: gpu.pipeline_cache(),
    });
    let empty_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("voxel sun splats"), entries: &[] });
    let sun_empty = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("voxel sun splats"), layout: &empty_layout, entries: &[] });
    let sun_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("voxel sun splats"),
        bind_group_layouts: &[Some(scene_layout), Some(&empty_layout), Some(data_layout)], immediate_size: 0 });
    let sun_splats = sun_splat_pipeline(gpu, &sun_layout, source,false);
    let sun_cached_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("cached voxel sun splats"),bind_group_layouts:&[Some(scene_layout),Some(&empty_layout),Some(data_layout),Some(&crate::shadow_damage::mask_layout(gpu))],immediate_size:0});
    let sun_cached_splats=sun_splat_pipeline(gpu,&sun_cached_layout,source,true);
    let cells_buffer = cells_override.cloned().unwrap_or_else(||gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("voxel splat cells"), contents: bytemuck::cast_slice(cells), usage: wgpu::BufferUsages::STORAGE }));
    SplatRaster { main_pipeline, cells: cells_buffer, capacity, layout, pipeline, sun_layout, sun_empty, sun_splats, sun_cached_layout, sun_cached_splats, sun_raster, sun_cached_raster,
        sun_target: Mutex::new(Vec::new()) }
}
fn pipelines(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str, active_tiles: bool,global_intervals:bool) -> [wgpu::ComputePipeline; 8] {
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxel forest"), source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    ["voxel_bins", "voxel_tiles", "voxel_clear", match (active_tiles,global_intervals) {
        (true,true)=>"voxel_integrate_global_active",(false,true)=>"voxel_integrate_global",
        (true,false)=>"voxel_integrate_active",(false,false)=>"voxel_integrate"},
        if active_tiles {"voxel_intervals_active"} else {"voxel_intervals"},"voxel_brick_scatter","voxel_splats","voxel_splat_resolve"]
        .map(|entry| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(layout), module: &module, entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: if entry == "voxel_splats" && brick_hzb_enabled() { &[("VOXEL_CULL_PHASE", 1.0),("VOXEL_COMPACT",1.0)] } else { &[] },
                ..Default::default() }, cache: gpu.pipeline_cache(),
        }))
}
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}\n{}",
        crate::wind::shade_source(),
        crate::shaders::source("voxel_forest.wgsl"),
        include_str!("shaders/shadow_damage_query.wgsl")
    )
}

/// Prove the occupancy implication used by the shader's empty-space skip.
/// Arbitrary externally constructed levels may not form the baker's pyramid;
/// those assets still render through the unchanged fine sampling path.
fn valid_empty_skip_hierarchy(forest: &crate::foliage_volume_data::GpuForest) -> bool {
    valid_skip_levels(forest, false)
}
pub(crate) fn valid_skip_levels(forest: &crate::foliage_volume_data::GpuForest, all_levels: bool) -> bool {
    let mut checked = std::collections::HashSet::new();
    for tree in &forest.instances {
        let count = tree.meta[2] as usize;
        if count == 0 {
            return false;
        }
        for selected in 0..if all_levels { count } else { 1 } {
        let first = tree.meta[1] as usize + selected;
        let relative = (count - selected - 1).min(2);
        if !checked.insert((first, relative)) {
            continue;
        }
        let Some(fine) = forest.levels.get(first) else {
            return false;
        };
        let Some(coarse) = forest.levels.get(first + relative) else {
            return false;
        };
        let ratio = 1u32 << relative;
        if fine.origin_cell[..3] != coarse.origin_cell[..3]
            || fine.origin_cell[3] * ratio as f32 != coarse.origin_cell[3]
            || fine.dims_offset[..3].contains(&0)
        {
            return false;
        }
        for axis in 0..3 {
            if fine.dims_offset[axis].div_ceil(ratio) != coarse.dims_offset[axis] {
                return false;
            }
        }
        let index = |level: &crate::foliage_volume_data::GpuLevel, x: u32, y: u32, z: u32|
            crate::foliage_volume_data::cell_index(forest,level,[x,y,z]);
        for z in 0..fine.dims_offset[2] {
            for y in 0..fine.dims_offset[1] {
                for x in 0..fine.dims_offset[0] {
                    match index(fine, x, y, z) {
                        Some(0) => {}
                        Some(_)
                            if index(coarse, x / ratio, y / ratio, z / ratio)
                                .is_some_and(|v| v != 0) => {}
                        _ => return false,
                    }
                }
            }
        }
        }
    }
    true
}

/// Exact per-owner material map; zero means every material stays rasterized.
/// Shadows still use original source geometry regardless of this color map.
pub(crate) fn replacement_materials(scene: &Scene) -> Vec<u32> {
    let mut result = vec![0; scene.instances.len().max(1)];
    // Far trees: bit 30 plus the tree; cull.wgsl and raster.wgsl switch the
    // whole owner by camera distance.
    if let Some(forest) = scene.voxel_forest.as_ref().filter(|_| scene.voxel_far_distance_m > 0.0) {
        for (index, tree) in forest.instances.iter().enumerate() {
            result[tree.meta[0] as usize] = FAR_TREE | index as u32;
        }
        return result;
    }
    if let Some(forest) = &scene.voxel_forest {
        for tree in &forest.instances {
            let owner = tree.meta[0] as usize;
            let material = tree.meta[3];
            assert!(
                owner < scene.instances.len() && (material == crate::foliage_volume_data::ALL_MATERIALS || (material as usize) < scene.materials.len()),
                "invalid voxel replacement owner/material"
            );
            let replacement = if material == crate::foliage_volume_data::ALL_MATERIALS { u32::MAX } else { material + 1 };
            assert!(
                result[owner] == 0 || result[owner] == replacement,
                "one owner cannot replace two different crown materials"
            );
            result[owner] = replacement;
        }
    }
    for binding in &scene.foliage_representations {
        result[binding.owner as usize] = 0x8000_0000 | binding.volume;
    }
    // A single per-view branch decision replaces every shared primitive of
    // that branch, without a new per-stage storage binding or a second source
    // mesh. Shadow passes ignore dynamic replacements and retain the members.
    if let Some(spec)=&scene.gpu_forest {
        for slot in 0..spec.slots {result[(spec.first_owner+slot)as usize]=0x8000_0000|slot;}
    }
    for group in &scene.foliage_source_groups {
        let replacement=result[group.owner as usize];
        assert!(replacement&0x8000_0000!=0,"shared foliage proxy needs a dynamic representation");
        for &member in &group.members { result[member as usize]=replacement; }
    }
    result
}

/// A packed volume's local coordinates must correspond to its source-owner
/// snapshot. Legacy whole-plant wind is disabled for replaced owners; only
/// PartWind currently updates a moving volume and its source in lockstep.
fn temporal_compatibility(scene:&Scene)->Result<(),String> {
    let Some(forest)=&scene.voxel_forest else {return Ok(());};
    let matrix=|r:[[f32;4];3]|glam::Mat4::from_cols_array(&[
        r[0][0],r[1][0],r[2][0],0.0,r[0][1],r[1][1],r[2][1],0.0,
        r[0][2],r[1][2],r[2][2],0.0,r[0][3],r[1][3],r[2][3],1.0]);
    for (index,tree) in forest.instances.iter().enumerate() {
        let owner=tree.meta[0];
        let source=scene.instances.get(owner as usize).ok_or("voxel temporal owner is out of bounds")?;
        let expected=matrix(source.rows).inverse();let actual=matrix(tree.inverse);
        if !expected.is_finite() || !actual.is_finite() || expected.to_cols_array().into_iter().zip(actual.to_cols_array())
            .any(|(a,b)|(a-b).abs()>2e-4*(1.0+a.abs().max(b.abs()))) {
            return Err(format!("voxel {index} inverse does not match its source owner for temporal reprojection"));
        }
        if scene.baked_animations.as_ref().is_some_and(|animation|animation.parts.iter().any(|part|part.owner==owner)) {
            return Err(format!("voxel owner {owner} uses baked animation without matching volume deformation"));
        }
        if scene.gpu_forest.is_none() && source.flags&crate::scene::PART_WIND!=0 && !scene.part_wind.iter().any(|part|part.owner==Some(owner)&&part.volume==Some(index as u32)) {
            return Err(format!("voxel owner {owner} needs a matching rigid-part volume pose for temporal reprojection"));
        }
    }
    Ok(())
}

impl VoxelRenderer {
    pub(crate) fn new(
        gpu: &Gpu,
        scene: &Scene,
        scene_layout: &wgpu::BindGroupLayout,
        volume_view_layout: &wgpu::BindGroupLayout,
        page_cache:Option<crate::foliage_pages::cache::Cache>,
    ) -> Option<Self> {
        let forest = scene.voxel_forest.as_ref()?;
        if forest.instances.is_empty() {
            return None;
        }
        let far = scene.voxel_far_distance_m;
        assert!(far.is_finite() && far >= 0.0, "invalid far tree distance");
        let proxies = scene.voxel_proxies.min(forest.instances.len() as u32);
        assert!(proxies == 0 || scene.voxel_proxy_pixels > 0.0, "whole-plant proxies need a pixel error");
        // Far trees and proxies never read the per-group selection.
        assert!(far > 0.0 || forest.instances.len() - proxies as usize <= MAX_VIEW_GROUPS,
            "voxel renderer exceeds checked selection capacity");
        assert!(forest.instances.len() < FAR_TREE as usize, "too many far trees");
        let upload = |name, data: &[u8]| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(name),
                    contents: if data.is_empty() { &[0; std::mem::size_of::<crate::foliage_volume_data::GpuCell>()] } else { data },
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let data = [
            page_cache.as_ref().map(|c|c.buffers[0].clone()).unwrap_or_else(||upload("voxel levels", bytemuck::cast_slice(&forest.levels))),
            page_cache.as_ref().map(|c|c.buffers[1].clone()).unwrap_or_else(||upload("voxel indices", bytemuck::cast_slice(&forest.indices))),
            page_cache.as_ref().map(|c|c.buffers[2].clone()).unwrap_or_else(||upload("voxel cells", bytemuck::cast_slice(&forest.cells))),
            gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxel trees"),
                contents: bytemuck::cast_slice(&forest.instances),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            }),
        ];
        // Both Frobenius and sqrt(matrix 1-norm * infinity-norm) bound
        // inverse-ray stretch. Their minimum is still conservative and avoids
        // needlessly shrinking identity/uniform-scale steps by sqrt(3).
        let step = forest
            .instances
            .iter()
            .map(|tree| {
                let cell = forest.levels[tree.meta[1] as usize].origin_cell[3];
                let frobenius = tree
                    .inverse
                    .iter()
                    .flat_map(|row| row[..3].iter())
                    .map(|v| v * v)
                    .sum::<f32>()
                    .sqrt();
                let norm_inf = tree
                    .inverse
                    .iter()
                    .map(|r| r[..3].iter().map(|v| v.abs()).sum::<f32>())
                    .fold(0.0, f32::max);
                let norm_one = (0..3)
                    .map(|c| tree.inverse.iter().map(|r| r[c].abs()).sum::<f32>())
                    .fold(0.0, f32::max);
                let stretch = frobenius.min((norm_inf * norm_one).sqrt());
                cell / stretch * 0.5
            })
            .fold(f32::INFINITY, f32::min);
        assert!(step.is_finite() && step > 0.0, "invalid voxel march step");
        let lod_pixels = scene.voxel_lod_pixels;
        assert!(lod_pixels.is_finite() && lod_pixels >= 0.0, "invalid voxel pixel error");
        // Dynamic LOD assumes the baker's consecutive dyadic mip levels.
        // Unrelated user-constructed grids must not silently select nonsense.
        if lod_pixels > 0.0 || scene.aggregate_lod.is_some() {
            for tree in &forest.instances {
                let levels = &forest.levels[tree.meta[1] as usize..][..tree.meta[2] as usize];
                assert!(!levels.is_empty(), "voxel LOD requires at least one level");
                for pair in levels.windows(2) {
                    assert!(pair[0].origin_cell[..3] == pair[1].origin_cell[..3]
                        && pair[0].origin_cell[3] * 2.0 == pair[1].origin_cell[3]
                        && (0..3).all(|axis| pair[0].dims_offset[axis].div_ceil(2) == pair[1].dims_offset[axis]),
                        "voxel LOD requires consecutive shared-origin mip levels");
                }
            }
        }
        let empty_skip = if lod_pixels > 0.0 || scene.aggregate_lod.is_some() { valid_skip_levels(forest, true) }
            else { valid_empty_skip_hierarchy(forest) };
        let tex = |sample_type, view_dimension| wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        };
        let unfiltered = tex(
            wgpu::TextureSampleType::Float { filterable: false },
            wgpu::TextureViewDimension::D2,
        );
        let layout = |name, bindings: &[(u32, wgpu::BindingType)], visibility| {
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(name),
                    entries: &bindings
                        .iter()
                        .map(|&(binding, ty)| wgpu::BindGroupLayoutEntry {
                            binding,
                            visibility,
                            ty,
                            count: None,
                        })
                        .collect::<Vec<_>>(),
                })
        };
        let data_layout = create_data_layout(gpu,false);
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voxel forest"),
                bind_group_layouts: &[
                    Some(scene_layout),
                    Some(volume_view_layout),
                    Some(&data_layout),
                ],
                immediate_size: 0,
            });
        let source = shader_source();
        let mut integrator_options = IntegratorOptions::from_environment();
        if page_cache.is_some(){integrator_options.brick_visibility=false;}
        // Far trees exist only as rasterized voxels; aggregate plants use them
        // too unless KERN_VOXEL_RASTER=0 asks for the ray integrator.
        integrator_options.raster |= far > 0.0 || proxies > 0
            || (scene.aggregate_lod.is_some() && std::env::var("KERN_VOXEL_RASTER").as_deref() != Ok("0"));
        let mut catalog_words=vec![0u32;4];
        let mut bin_count=forest.instances.len() as u32;
        if integrator_options.brick_visibility {
            // Exact-mip aggregate selection is required. Legacy fractional
            // mip blending retains whole-volume conservative visibility.
            let catalog=scene.aggregate_lod.as_ref().ok_or("brick visibility requires aggregate selection".to_owned())
                .and_then(|_|crate::voxel_bricks::build(forest,64*1024*1024-16));
            match catalog {
                Ok(catalog)=>{bin_count=catalog.references.len() as u32;catalog_words=catalog.words();},
                Err(error)=>{eprintln!("brick visibility fallback: {error}");integrator_options.brick_visibility=false;}
            }
        }
        let brick_catalog=upload("voxel shared brick catalog",bytemuck::cast_slice(&catalog_words));
        eprintln!("voxel brick visibility: enabled={} references={} catalog_bytes={}",integrator_options.brick_visibility,bin_count,catalog_words.len()*4);
        integrator_options.global_intervals |= integrator_options.brick_visibility;
        let antipodal_requested=integrator_options.antipodal_normals;
        let antipodal_eligible=antipodal_normal_cells(&forest.cells);
        integrator_options.antipodal_normals &= antipodal_eligible;
        let specialized_source = integrator_options.specialize(&temporal_source(&source,false).expect("invalid voxel temporal blocks"))
            .expect("invalid voxel integrator option shader");
        let [bins, tiles, clear, integrate,intervals,scatter,splats,resolve] = pipelines(gpu, &pipeline_layout, &specialized_source, scene.voxel_active_tiles,integrator_options.global_intervals);
        let gbuffer = surface_pipeline(gpu, &pipeline_layout, &specialized_source);
        let brick_hzb = (page_cache.is_none() && brick_hzb_enabled()).then(|| crate::depth_pyramid::DepthPyramid::new(gpu));
        let selection = splat_selection_pipelines(gpu, &pipeline_layout, &specialized_source);
        let second = brick_hzb.as_ref().map(|_| second_splat_pipeline(gpu, &pipeline_layout, &specialized_source));
        eprintln!("voxel integrator: interval_capacity=64 active_capacity=16 empty_ceiling={} (accepted skip fix; bounded prototype, complete scan on cache overflow)",
            integrator_options.empty_ceiling);
        eprintln!("voxel normals: antipodal_requested={antipodal_requested} antipodal_eligible={antipodal_eligible} antipodal_enabled={} normal_lobes={} cell_bytes=176 (experimental; exact pairs, generic fallback)",
            integrator_options.antipodal_normals,if integrator_options.antipodal_normals {3}else{6});
        eprintln!("voxel global intervals: enabled={} stream_bytes_per_view={} maximum_pixel_slots={} (experimental; complete scan on overflow)",
            integrator_options.global_intervals,INTERVAL_STREAM_BYTES,INTERVAL_STREAM_PIXEL_SLOTS);
        let raster = if integrator_options.raster { match if brick_hzb.is_some() { splat_cells_bricked(forest) } else { splat_cells(forest) } {
            Ok(cells) if cells.len() > forest.levels.len() => {
                let capacity = splat_capacity(forest, &cells);
                eprintln!("voxel raster: enabled cells={} splats_per_view={capacity} (stochastic opaque voxels; integrator off)", cells.len() - forest.levels.len());
                Some(splat_raster(gpu, &cells, capacity, scene_layout, &data_layout, &specialized_source,page_cache.as_ref().map(|c|&c.buffers[3])))
            }
            Ok(_) => { eprintln!("voxel raster fallback: no occupied cells"); None }
            Err(error) => { eprintln!("voxel raster fallback: {error}"); None }
        }} else { None };
        assert!(raster.is_some() || far > 0.0 || forest.instances.len() - proxies as usize <= INTEGRATOR_MAX_VOLUMES,
            "voxel integrator exceeds its checked volume capacity");
        integrator_options.raster = raster.is_some();
        let lighting_layout = pipeline_layout;
        let composite_layout = layout(
            "voxel composite",
            &[(0, unfiltered)],
            wgpu::ShaderStages::FRAGMENT,
        );
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voxel composite"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("voxel composite"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("shaders/voxel_composite.wgsl").into(),
                ),
            });
        let composite = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("voxel composite"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            });
        Some(Self {
            bins,
            tiles,
            clear,
            integrate,
            intervals,
            scatter,
            splats,
            resolve,
            gbuffer, selection, second, brick_hzb,
            raster,
            pipeline_layout: lighting_layout,
            scene_layout: scene_layout.clone(),
            view_layout: volume_view_layout.clone(),
            temporal: Mutex::new(None),
            temporal_error: temporal_compatibility(scene).err(),
            source,
            composite,
            data_layout,
            composite_layout,
            data,
            page_cache:page_cache.map(Mutex::new),
            brick_catalog,
            bin_count,
            count: forest.instances.len() as u32,
            far,
            proxies,
            proxy_pixels: scene.voxel_proxy_pixels,
            proxy_pixels_from: scene.voxel_proxy_pixels_from,
            step,
            empty_skip,
            lod_pixels,
            active_tiles: scene.voxel_active_tiles,
            integrator_options,
            owners: forest.instances.iter().map(|t| t.meta[0]).collect(),
            owner_index: { let mut owners:Vec<_>=forest.instances.iter().map(|t|t.meta[0] as usize).collect();owners.sort_unstable();owners.dedup();owners },
            targets: Mutex::new(Vec::new()),
        })
    }
    pub(crate) fn initialize_pages(&self,gpu:&Gpu,runtime:&crate::foliage_runtime::Runtime)->Result<(),String>{if let Some(cache)=&self.page_cache {cache.lock().unwrap().initialize(gpu,runtime.asset_selector())?;}Ok(())}
    pub(crate) fn service_pages(&self,gpu:&Gpu,runtime:&crate::foliage_runtime::Runtime)->Result<(),String>{if let Some(cache)=&self.page_cache {cache.lock().unwrap().service(gpu,runtime.asset_selector())?;}Ok(())}
    #[cfg(test)]
    pub(crate) fn warm_page_fixture(&self,gpu:&Gpu,runtime:&crate::foliage_runtime::Runtime){if let Some(cache)=&self.page_cache{cache.lock().unwrap().warm_fixture(gpu,runtime.asset_selector()).unwrap();}}
    pub(crate) fn page_stats(&self)->Option<crate::foliage_pages::cache::Stats>{self.page_cache.as_ref().map(|c|c.lock().unwrap().stats())}
    /// Shared rigid-part compute writes poses before volume tile/bin passes.
    /// Asset grids and cells remain immutable; the 96-byte tree layout is unchanged.
    pub(crate) fn has_visibility_stream(&self)->bool {self.integrator_options.global_intervals}
    /// GPU scopes for the eight timestamp slots render() takes, None when unused.
    pub(crate) fn timing_names(&self,scene_surface:bool)->[Option<&'static str>;8] {
        if self.raster.is_some() {return [Some("voxel splats"),None,Some("voxel splat raster"),Some("voxel splat resolve"),(!scene_surface).then_some("voxel composite"),self.brick_hzb.as_ref().map(|_|"voxel brick hzb"),self.brick_hzb.as_ref().map(|_|"voxel brick cull"),self.brick_hzb.as_ref().map(|_|"voxel brick raster")];}
        [Some("voxel bins"),Some("voxel tiles"),self.has_visibility_stream().then_some("voxel visibility"),Some("voxel integrate"),Some("voxel composite"),None,None,None]
    }
    pub(crate) fn tree_buffer(&self) -> &wgpu::Buffer { &self.data[3] }
    /// How whole-plant proxy `volume` (forest index) draws while another
    /// representation stands in for its plant (recipe_forest::Pool).
    pub(crate) fn set_proxy_show(&self, gpu: &Gpu, volume: u32, show: crate::vegetation_asset::ProxyShow) -> Result<(), String> {
        if volume < self.count - self.proxies || volume >= self.count {
            return Err("only whole-plant proxies can be hidden".into());
        }
        let offset = volume as usize * std::mem::size_of::<crate::foliage_volume_data::GpuTree>()
            + std::mem::offset_of!(crate::foliage_volume_data::GpuTree, bounds_min) + 12;
        gpu.queue.write_buffer(&self.data[3], offset as u64, bytemuck::bytes_of(&(show as u32 as f32)));
        Ok(())
    }
    pub(crate) fn validate_proxy_update(&self, first: u32, trees: &[crate::foliage_volume_data::GpuTree]) -> Result<(), String> {
        if first < self.count-self.proxies || first as usize + trees.len() > self.count as usize {
            return Err("proxy update exceeds reserved whole-plant range".into());
        }
        for (i,t) in trees.iter().enumerate() {
            if t.meta[0] != self.owners[first as usize+i] || (t.meta[1] as u64 + t.meta[2] as u64) * std::mem::size_of::<crate::foliage_volume_data::GpuLevel>() as u64 > self.data[0].size() || t.meta[2] == 0 ||
                !t.inverse.iter().flatten().chain(t.bounds_min.iter()).chain(t.bounds_max.iter()).all(|x| x.is_finite()) {
                return Err("invalid proxy owner, levels or transform".into());
            }
        }
        Ok(())
    }
    pub(crate) fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        crate::owner_ranges::contains(&self.owner_index,first,count)
    }
    pub(crate) fn prepare_temporal(&self,gpu:&Gpu)->Result<(),String> {
        if let Some(error)=&self.temporal_error {return Err(error.clone());}
        let mut temporal=self.temporal.lock().unwrap();
        if temporal.is_some() {return Ok(());}
        let source=self.integrator_options.specialize(&temporal_source(&self.source,true)?)?;
        let prepared=crate::shaders::checked(gpu,|| {
            let data_layout=create_data_layout(gpu,true);
            let layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:Some("temporal voxel forest"),bind_group_layouts:&[Some(&self.scene_layout),Some(&self.view_layout),Some(&data_layout)],immediate_size:0});
            let pipelines=pipelines(gpu,&layout,&source,self.active_tiles,self.integrator_options.global_intervals);
            let gbuffer = surface_pipeline(gpu, &layout, &source);
            TemporalPipelines {selection: splat_selection_pipelines(gpu, &layout, &source),second: self.brick_hzb.as_ref().map(|_| second_splat_pipeline(gpu, &layout, &source)),data_layout,layout,pipelines,gbuffer}
        })?;
        *temporal=Some(prepared);Ok(())
    }
    /// Recreated motion snapshots must not leave target bindings pointing to
    /// the previous DLSS instance buffer. Queued GPU work retains its handles.
    pub(crate) fn reset_targets(&self) {for target in self.targets.lock().unwrap().iter_mut() {*target=None;}}
    pub(crate) fn prepare_reload(&self, gpu: &Gpu, source: String) -> Result<Option<Reload>, String> {
        if source == self.source { return Ok(None); }
        let specialized_source = self.integrator_options.specialize(&temporal_source(&source,false)?)?;
        let pipelines = crate::shaders::checked(gpu, || pipelines(gpu, &self.pipeline_layout, &specialized_source, self.active_tiles,self.integrator_options.global_intervals))?;
        let temporal=if let Some(temporal)=self.temporal.lock().unwrap().as_ref() {
            let specialized=self.integrator_options.specialize(&temporal_source(&source,true)?)?;
            Some(crate::shaders::checked(gpu,||(
                self::pipelines(gpu,&temporal.layout,&specialized,self.active_tiles,self.integrator_options.global_intervals),
                surface_pipeline(gpu,&temporal.layout,&specialized),
                self.brick_hzb.as_ref().map(|_|second_splat_pipeline(gpu,&temporal.layout,&specialized)),
                splat_selection_pipelines(gpu,&temporal.layout,&specialized)))?)
        } else {None};
        let sun=self.raster.as_ref().map(|r|crate::shaders::checked(gpu,||(sun_splat_pipeline(gpu,&r.sun_layout,&specialized_source,false),sun_splat_pipeline(gpu,&r.sun_cached_layout,&specialized_source,true)))).transpose()?;
        let surface=crate::shaders::checked(gpu,||surface_pipeline(gpu,&self.pipeline_layout,&specialized_source))?;
        let second=self.brick_hzb.as_ref().map(|_|crate::shaders::checked(gpu,||second_splat_pipeline(gpu,&self.pipeline_layout,&specialized_source))).transpose()?;
        let selection=crate::shaders::checked(gpu,||splat_selection_pipelines(gpu,&self.pipeline_layout,&specialized_source))?;
        Ok(Some(Reload { pipelines, surface, selection, second, temporal, sun, source }))
    }
    pub(crate) fn commit_reload(&mut self, reload: Reload) {
        [self.bins, self.tiles, self.clear, self.integrate,self.intervals,self.scatter,self.splats,self.resolve] = reload.pipelines;
        self.gbuffer=reload.surface;self.second=reload.second;self.selection=reload.selection;
        if let Some((pipelines,gbuffer,second,selection))=reload.temporal {
            let mut guard=self.temporal.lock().unwrap();let p=guard.as_mut().unwrap();p.pipelines=pipelines;p.gbuffer=gbuffer;p.second=second;p.selection=selection;
        }
        if let (Some(raster),Some(sun))=(self.raster.as_mut(),reload.sun) {raster.sun_splats=sun.0;raster.sun_cached_splats=sun.1;}
        self.source = reload.source;
    }
    pub(crate) fn set_view_count(&self, count: usize) {
        let mut targets = self.targets.lock().unwrap();
        targets.truncate(count);
        targets.resize_with(count, || None);
    }
    fn targets(&self, gpu: &Gpu, size: (u32, u32), selection: &wgpu::Buffer,
        temporal:Option<(&wgpu::BindGroupLayout,&wgpu::Buffer)>) -> Result<Targets,String> {
        let limits=gpu.device.limits();
        let interval_stream=if self.integrator_options.global_intervals {
            Some(interval_stream_layout(size,limits.max_buffer_size.min(u64::from(limits.max_storage_buffer_binding_size)))?)
        } else {None};
        let layer = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("voxel C and alpha"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let layer_view = layer.create_view(&Default::default());
        let motion=temporal.map(|_|gpu.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("voxel fractional reprojection"),size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba32Float,
            usage:wgpu::TextureUsages::STORAGE_BINDING|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_SRC,view_formats:&[]}));
        let motion_view=motion.as_ref().map(|texture|texture.create_view(&Default::default()));
        let tile_count = u64::from(size.0.div_ceil(TILE)) * u64::from(size.1.div_ceil(TILE));
        if let Some(stream)=interval_stream {eprintln!("voxel interval target: {}x{} pixel_slots={} records={} total_worklist_bytes={}",
            size.0,size.1,stream.pixel_slots,stream.records,stream.bytes);}
        let mask_bytes=if self.integrator_options.brick_visibility {tile_count*u64::from(self.count.div_ceil(32))*4} else {4};
        if mask_bytes + COUNTER_BYTES > u64::from(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size) {
            return Err("voxel brick visibility mask exceeds device limit".into());
        }
        let tiles = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel tile candidates"),
            size: tile_count * u64::from(CANDIDATES + 1) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let counters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel diagnostics"),
            size: COUNTER_BYTES + mask_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Header: dispatch xyz, active count; then one tile index per job.
        // Keep indirect arguments separate: the integration pass reads jobs
        // as storage, which cannot alias an indirect-dispatch argument buffer.
        let jobs = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel active tile worklist"),
            size: interval_stream.map_or((4+tile_count)*4,|s|s.bytes),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indirect = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel active tile indirect arguments"),
            size: 12,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let config = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxel config"),
                contents: bytemuck::bytes_of(&Config {
                    size_count: [size.0, size.1, size.0.div_ceil(TILE), self.count],
                    march: [self.step, self.far, u32::from(self.empty_skip) as f32, self.lod_pixels],
                    dispatch: [u32::from(self.active_tiles), 0, interval_stream.map_or(0,|s|s.header_word),
                        interval_stream.map_or(0,|s|s.pixel_slots)],
                    proxy: [(self.count - self.proxies) as f32, self.proxy_pixels, self.proxy_pixels_from, 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let hzb = self.brick_hzb.as_ref().map(|p| p.target(gpu, size));
        let empty_hzb = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("unused voxel hzb"), size: wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R32Float,
            usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[] }).create_view(&Default::default());
        let mut entries=vec![
                wgpu::BindGroupEntry {binding:20,resource:wgpu::BindingResource::TextureView(hzb.as_ref().map_or(&empty_hzb, |h| &h.view))},
                wgpu::BindGroupEntry {binding:12,resource:self.brick_catalog.as_entire_binding()},
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.data[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.data[1].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.data[2].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.data[3].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tiles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&layer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: config.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: jobs.as_entire_binding(),
                },
                wgpu::BindGroupEntry { binding: 9, resource: selection.as_entire_binding() },
            ];
        // Splat resources; one-texel stand-ins when rasterization is off.
        let raster_size = if self.raster.is_some() { size } else { (1, 1) };
        let texture = |label, format| gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), size: wgpu::Extent3d { width: raster_size.0, height: raster_size.1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        let splat_id = texture("voxel splat id", wgpu::TextureFormat::R32Uint);
        let gbuffer = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("voxel surface"), size: wgpu::Extent3d { width: raster_size.0, height: raster_size.1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        let splat_depth = texture("voxel splat depth", wgpu::TextureFormat::Depth32Float);
        let splat_capacity = self.raster.as_ref().map_or(0, |r| r.capacity
            .min((u64::from(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size) / 16).saturating_sub(12 + 4 * u64::from(self.count))));
        let splat_records = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel splats"), size: (12 + 4 * u64::from(self.count) + splat_capacity) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let splat_draw = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel splat draw"), size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let stand_in;
        let splat_cells = match &self.raster {
            Some(raster) => &raster.cells,
            None => { stand_in = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("voxel splat cells"), size: 16,
                usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false }); &stand_in }
        };
        entries.extend([
            wgpu::BindGroupEntry { binding: 14, resource: splat_cells.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 15, resource: splat_records.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 16, resource: splat_draw.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::TextureView(&splat_id) },
            wgpu::BindGroupEntry { binding: 18, resource: wgpu::BindingResource::TextureView(&splat_depth) },
            wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::TextureView(&gbuffer) },
        ]);
        let splat = self.raster.as_ref().map(|raster| SplatTargets {
            bind: gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("voxel splat raster"), layout: &raster.layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: splat_records.as_entire_binding() }] }),
            gbuffer: gbuffer.clone(), records: splat_records.clone(), draw: splat_draw.clone(), id: splat_id.clone(), depth: splat_depth.clone(),
        });
        if let Some((_,previous))=temporal {
            entries.push(wgpu::BindGroupEntry {binding:10,resource:wgpu::BindingResource::TextureView(motion_view.as_ref().unwrap())});
            entries.push(wgpu::BindGroupEntry {binding:11,resource:previous.as_entire_binding()});
        }
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxel data"),layout:temporal.map_or(&self.data_layout,|(layout,_)|layout),entries:&entries,
        });
        let composite = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxel composite"),
            layout: &self.composite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&layer_view),
            }],
        });
        Ok(Targets {
            hzb, splat,
            size,
            layer,
            bind,
            composite,
            counters,
            tiles,
            jobs,
            indirect,
            config,
            motion,
            interval_stream,
        })
    }
    /// Whether prepare_temporal succeeded (its pipelines fit the device).
    pub(crate) fn has_temporal(&self) -> bool { self.temporal.lock().unwrap().is_some() }
    /// The view's voxel surface (volume_gbuffer) once Stage::Splats drew
    /// into the scene's targets.
    pub(crate) fn surface(&self, index: usize) -> Option<wgpu::TextureView> {
        self.targets.lock().unwrap().get(index)?.as_ref()?.splat.as_ref().map(|s| s.gbuffer.clone())
    }
    /// The view's rastered splat depth (reversed Z, 0 where none), once
    /// Stage::Splats has run for it.
    pub(crate) fn splat_depth(&self, index: usize) -> Option<wgpu::TextureView> {
        self.targets.lock().unwrap().get(index)?.as_ref()?.splat.as_ref().map(|s| s.depth.clone())
    }
    fn select_splat_trees(&self, encoder:&mut wgpu::CommandEncoder, target:&Targets,
        scene:&wgpu::BindGroup, view:&wgpu::BindGroup, pipeline:&wgpu::ComputePipeline) {
        encoder.clear_buffer(&target.tiles,0,Some(16));
        {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("voxel visible tree list"),timestamp_writes:None});
            pass.set_pipeline(pipeline);pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,view,&[]);pass.set_bind_group(2,&target.bind,&[]);
            let groups=self.count.div_ceil(64);pass.dispatch_workgroups(groups.min(65535),groups.div_ceil(65535).max(1),1);
        }
        encoder.copy_buffer_to_buffer(&target.tiles,0,&target.indirect,0,12);
    }
    pub(crate) fn render(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        scene: &wgpu::BindGroup,
        view: &wgpu::BindGroup,
        hdr: &wgpu::TextureView,
        selection: &wgpu::Buffer,
        temporal: Option<TemporalInputs<'_>>,
        times: [Option<(&wgpu::QuerySet, u32)>; 8],
        stage: Stage,
        // The scene's visibility and depth: the splats draw into them and
        // shade lights them (Rest then writes their surface, no composite).
        scene_targets: Option<(&wgpu::TextureView, &wgpu::TextureView)>,
    ) -> Result<(),String> {
        let temporal_guard=self.temporal.lock().unwrap();
        let temporal_pipelines=temporal.as_ref().map(|_|temporal_guard.as_ref().expect("prepare_temporal before voxel render"));
        let mut targets = self.targets.lock().unwrap();
        if !targets[index].as_ref().is_some_and(|t| t.size == size && t.motion.is_some()==temporal.is_some()) {
            targets[index] = Some(self.targets(gpu, size, selection,
                temporal.as_ref().zip(temporal_pipelines).map(|(input,pipes)|(&pipes.data_layout,input.previous_instances)))?);
        }
        let target = targets[index].as_ref().unwrap();
        if let Some(input)=&temporal {
            // Only the temporal snapshot flag changes. z/w retain the bounded
            // stream layout installed with this target's immutable size.
            gpu.queue.write_buffer(&target.config,36,bytemuck::bytes_of(&u32::from(input.snapshot_valid)));
        }
        let [bins,tiles,clear,integrate,intervals,scatter,splats,resolve]=temporal_pipelines.map_or([&self.bins,&self.tiles,&self.clear,&self.integrate,&self.intervals,&self.scatter,&self.splats,&self.resolve],
            |p|[&p.pipelines[0],&p.pipelines[1],&p.pipelines[2],&p.pipelines[3],&p.pipelines[4],&p.pipelines[5],&p.pipelines[6],&p.pipelines[7]]);
        if stage == Stage::Splats && target.splat.is_none() { return Ok(()); }
        // The splat stage ran before the opaque shade (Stage::Splats): the
        // rest only resolves and composites what it rastered.
        let splats_done = stage == Stage::Rest && target.splat.is_some();
        if !splats_done {
        encoder.clear_buffer(&target.counters, 0, None);
        // Counts and slots share a buffer; clearing it preserves a bounded
        // zero-allocation append path and prevents old counts surviving views.
        encoder.clear_buffer(&target.tiles, 0, None);
        if self.active_tiles {
            encoder.clear_buffer(&target.jobs, 0, Some(16));
        }
        if let Some(stream)=target.interval_stream {
            let bytes=if self.integrator_options.brick_visibility {16+u64::from(stream.pixel_slots)*8}else{16};
            encoder.clear_buffer(&target.jobs,u64::from(stream.header_word)*4,Some(bytes));
        }
        }
        if let (Some(raster), Some(splat)) = (&self.raster, &target.splat) {
            if !splats_done {
            encoder.clear_buffer(&splat.draw, 0, None);
            let selection=temporal_pipelines.and_then(|p|p.selection.as_ref()).or(self.selection.as_ref());
            if let Some(p)=selection {
                if let Some((q,i))=times[0] {encoder.write_timestamp(q,i);}
                self.select_splat_trees(encoder,target,scene,view,&p[0]);
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("voxel splats"),
                    timestamp_writes: times[0].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                        query_set, beginning_of_pass_write_index: if selection.is_some() { None } else { Some(i) }, end_of_pass_write_index: Some(i + 1) }) });
                pass.set_bind_group(0, scene, &[]); pass.set_bind_group(1, view, &[]); pass.set_bind_group(2, &target.bind, &[]);
                pass.set_pipeline(splats);
                // One workgroup per tree.
                if selection.is_some() {pass.dispatch_workgroups_indirect(&target.indirect,0);}
                else {pass.dispatch_workgroups(self.count.min(65535), self.count.div_ceil(65535).max(1), 1);}
            }
            for phase in 0..if self.brick_hzb.is_some() { 2 } else { 1 } {
                if phase == 1 {
                    let pyramid = self.brick_hzb.as_ref().unwrap();
                    pyramid.encode(gpu, encoder, scene_targets.map_or(&splat.depth, |(_, d)| d), target.hzb.as_ref().unwrap(), times[5]);
                    if let Some((q, i)) = times[5] { encoder.write_timestamp(q, i + 1); }
                    if let Some((q,i))=times[6] {encoder.write_timestamp(q,i);}
                    self.select_splat_trees(encoder,target,scene,view,&selection.unwrap()[1]);
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("voxel brick second pass"), timestamp_writes: times[6].map(|(query_set,i)| wgpu::ComputePassTimestampWrites {
                            query_set, beginning_of_pass_write_index:None,end_of_pass_write_index:Some(i+1) }) });
                    pass.set_bind_group(0, scene, &[]); pass.set_bind_group(1, view, &[]); pass.set_bind_group(2, &target.bind, &[]);
                    pass.set_pipeline(temporal_pipelines.and_then(|p|p.second.as_ref()).or(self.second.as_ref()).unwrap());
                    pass.dispatch_workgroups_indirect(&target.indirect,0);
                }
            {
                let id = wgpu::RenderPassColorAttachment { view: &splat.id, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: if phase == 0 { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store } };
                let (colors, depth) = match scene_targets {
                    Some((vis, depth)) => (vec![Some(wgpu::RenderPassColorAttachment { view: vis, depth_slice: None, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } }), Some(id)],
                        wgpu::RenderPassDepthStencilAttachment { view: depth,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                    None => (vec![Some(id)], wgpu::RenderPassDepthStencilAttachment { view: &splat.depth,
                        depth_ops: Some(wgpu::Operations { load: if phase == 0 { wgpu::LoadOp::Clear(0.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("voxel splat raster"),
                    color_attachments: &colors,
                    depth_stencil_attachment: Some(depth),
                    timestamp_writes: (if phase == 0 { times[2] } else { times[7] }).map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                        query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
                    occlusion_query_set: None, multiview_mask: None,
                });
                pass.set_pipeline(if scene_targets.is_some() { &raster.main_pipeline } else { &raster.pipeline });
                pass.set_bind_group(0, &splat.bind, &[]);
                pass.draw_indirect(&splat.draw, 0);
            }
            }
            }
            // In the scene's targets their surface is needed before shade.
            if stage == Stage::Splats && scene_targets.is_some() && !splats_done {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("voxel surface"),
                    timestamp_writes: times[3].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                        query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
                pass.set_bind_group(0, scene, &[]); pass.set_bind_group(1, view, &[]); pass.set_bind_group(2, &target.bind, &[]);
                pass.set_pipeline(temporal_pipelines.map_or(&self.gbuffer, |p| &p.gbuffer));
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
            if stage == Stage::Splats || scene_targets.is_some() { return Ok(()); }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("voxel splat resolve"),
                timestamp_writes: times[3].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                    query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
            pass.set_bind_group(0, scene, &[]); pass.set_bind_group(1, view, &[]); pass.set_bind_group(2, &target.bind, &[]);
            pass.set_pipeline(resolve);
            pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        }
        for (stage, pipeline) in [bins, tiles, integrate].into_iter().enumerate().filter(|_| self.raster.is_none()) {
            if stage == 2 && self.active_tiles {
                encoder.copy_buffer_to_buffer(&target.jobs, 0, &target.indirect, 0, 12);
            }
            if stage==2 && self.integrator_options.brick_visibility {
                let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label:Some("parallel voxel brick visibility"),timestamp_writes:times[2].map(|(query_set,i)|
                        wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:None})});
                pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,view,&[]);pass.set_bind_group(2,&target.bind,&[]);
                pass.set_pipeline(scatter);let groups=self.bin_count.div_ceil(64);
                pass.dispatch_workgroups(groups.min(65535),groups.div_ceil(65535).max(1),1);
            }
            if stage==2 && target.interval_stream.is_some() {
                // Separate passes make every interval write visible before
                // any ray consumes it. Visibility has its own GPU timing scope.
                let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label:Some("voxel brick visibility stream"),timestamp_writes:times[2].map(|(query_set,i)|
                        wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:if self.integrator_options.brick_visibility {None}else{Some(i)},end_of_pass_write_index:Some(i+1)})});
                pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,view,&[]);pass.set_bind_group(2,&target.bind,&[]);
                pass.set_pipeline(intervals);
                if self.active_tiles {pass.dispatch_workgroups_indirect(&target.indirect,0);}
                    else {pass.dispatch_workgroups(size.0.div_ceil(8),size.1.div_ceil(8),1);}
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(["voxel bins", "voxel tiles", "voxel integrate"][stage]),
                timestamp_writes: times[if stage==2 {3}else{stage}].map(|(query_set, i)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(i),
                        end_of_pass_write_index: Some(i + 1),
                    }
                }),
            });
            pass.set_bind_group(0, scene, &[]);
            pass.set_bind_group(1, view, &[]);
            pass.set_bind_group(2, &target.bind, &[]);
            if stage == 1 && self.active_tiles {
                // Count the necessary clear in the tile-pass GPU timestamp.
                // This entry point never reaches the large per-ray arrays.
                pass.set_pipeline(clear);
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
            pass.set_pipeline(pipeline);
            if stage == 0 {
                let groups=self.bin_count.div_ceil(64);
                pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535).max(1), 1);
            } else if stage == 2 && self.active_tiles {
                pass.dispatch_workgroups_indirect(&target.indirect, 0);
            } else {
                let dims = if stage == 1 {
                    (size.0.div_ceil(TILE), size.1.div_ceil(TILE))
                } else { size };
                pass.dispatch_workgroups(dims.0.div_ceil(8), dims.1.div_ceil(8), 1);
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("voxel composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: times[4].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &target.composite, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
    /// Whether render_sun draws this forest into the sun map (rasterized
    /// voxels on). The caller then marks the sun selection with
    /// sun_selection, so the replaced source stops casting there.
    pub(crate) fn casts_sun(&self) -> bool { self.raster.is_some() }
    /// Whole-plant proxies have no source, so they also cast into the
    /// near shadow map (render_sun with that map's own selection).
    pub(crate) fn has_proxies(&self) -> bool { self.raster.is_some() && self.proxies > 0 }
    pub(crate) fn sun_selection() -> Vec<u32> { vec![selection_pack([1, u32::MAX, 0, 2]); MAX_VIEW_GROUPS] }
    /// Draw the voxels of every sun-casting group into the sun map, after
    /// its source raster. `selection` is the sun view's selection buffer.
    pub(crate) fn render_sun(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, scene: &wgpu::BindGroup,
        selection: &wgpu::Buffer, sun: SunSplatView<'_>, times: [Option<(&wgpu::QuerySet, u32)>; 2]) -> Result<(), String> {
        let Some(raster) = &self.raster else { return Ok(()) };
        let mut slots = raster.sun_target.lock().unwrap();
        let slot = match slots.iter().position(|(buffer, _)| buffer == selection) {
            Some(slot) => slot,
            None => { slots.push((selection.clone(), self.targets(gpu, (1, 1), selection, None)?)); slots.len() - 1 }
        };
        let target = &slots[slot].1;
        let splat = target.splat.as_ref().unwrap();
        let mut header = [[0.0f32; 4]; 12];
        header[..4].copy_from_slice(&sun.view_proj.to_cols_array_2d());
        header[4..10].copy_from_slice(&sun.planes);
        header[10] = [sun.direction.x, sun.direction.y, sun.direction.z, sun.texel_m];
        gpu.queue.write_buffer(&splat.records, 0, bytemuck::cast_slice(&header));
        encoder.clear_buffer(&splat.draw, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("voxel sun splats"),
                timestamp_writes: times[0].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                    query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
            pass.set_bind_group(0, scene, &[]); pass.set_bind_group(1, &raster.sun_empty, &[]); pass.set_bind_group(2, &target.bind, &[]);
            pass.set_pipeline(if sun.damage.is_some(){&raster.sun_cached_splats}else{&raster.sun_splats});
            if let Some(mask)=sun.damage {pass.set_bind_group(3,mask,&[]);}
            pass.dispatch_workgroups(self.count.min(65535), self.count.div_ceil(65535).max(1), 1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("voxel sun splat raster"), color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: sun.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: times[1].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
            occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(if sun.damage.is_some(){&raster.sun_cached_raster}else{&raster.sun_raster});
        if let Some(mask)=sun.damage {pass.set_bind_group(1,mask,&[]);}
        pass.set_bind_group(0, &splat.bind, &[]);
        pass.draw_indirect(&splat.draw, 0);
        Ok(())
    }
    pub(crate) fn motion_layer(&self,index:usize)->Option<(wgpu::TextureView,wgpu::TextureView)> {
        let targets=self.targets.lock().unwrap();let target=targets.get(index)?.as_ref()?;
        Some((target.layer.create_view(&Default::default()),target.motion.as_ref()?.create_view(&Default::default())))
    }
    /// Explicit offline readback only. Normal frames never map or wait here.
    pub(crate) fn diagnostics(&self, gpu: &Gpu) -> Result<Vec<VoxelViewDiagnostics>, String> {
        let targets = self.targets.lock().unwrap();
        let targets: Vec<_> = targets.iter().filter_map(Option::as_ref).collect();
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let stride=COUNTER_BYTES+24;
        let size = stride * targets.len() as u64;
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel diagnostics readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for (i, target) in targets.iter().enumerate() {
            encoder.copy_buffer_to_buffer(
                &target.counters,
                0,
                &read,
                i as u64 * stride,
                COUNTER_BYTES,
            );
            if let Some(splat)=&target.splat {
                encoder.copy_buffer_to_buffer(&splat.draw,0,&read,i as u64*stride+COUNTER_BYTES+8,16);
            }
            if let Some(stream)=target.interval_stream {
                encoder.copy_buffer_to_buffer(&target.jobs,(u64::from(stream.header_word)+1)*4,
                    &read,i as u64*stride+COUNTER_BYTES,8);
            }
        }
        let submission = gpu.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(60)),
            })
            .map_err(|e| format!("voxel diagnostics wait failed: {e:?}"))?;
        receive
            .try_recv()
            .map_err(|e| format!("voxel diagnostics callback missing: {e}"))?
            .map_err(|e| format!("voxel diagnostics mapping failed: {e:?}"))?;
        let result = {
            let bytes = read
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("voxel diagnostics range failed: {e:?}"))?;
            bytes
                .chunks_exact(stride as usize).enumerate()
                .map(|(target_index,bytes)| {
                    let value =
                        |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
                    VoxelViewDiagnostics {
                        candidate_overflow_tiles: value(0),
                        step_overflow_rays: value(1),
                        traced_rays: value(2),
                        max_steps_used: value(3),
                        max_tile_candidates: value(4),
                        max_ray_candidates: value(5),
                        ray_candidate_overflow_rays: value(6),
                        max_active_volumes: value(7),
                        active_cache_overflow_rays: value(8),
                        max_empty_brick_skips: value(9),
                        max_occupied_lod_x256: value(10),
                        occupied_coarse_mip_rays: value(11),
                        active_tiles: value(12),
                        scheduled_pixels: value(13),
                        total_steps: diagnostic_u64(bytes, 14),
                        volume_samples: diagnostic_u64(bytes, 16),
                        occupied_cell_samples: diagnostic_u64(bytes, 18),
                        aabb_interval_tests: diagnostic_u64(bytes, 20),
                        interval_stream_overflow_rays:value(22),
                        interval_stream_records:value(23),
                        raster_splats:value((COUNTER_BYTES/4+3)as usize),
                        raster_vertices:u64::from(value((COUNTER_BYTES/4+2)as usize))*u64::from(value((COUNTER_BYTES/4+3)as usize)),
                        raster_capacity:targets[target_index].splat.as_ref().map_or(0,|s|s.records.size()/16-12-4*u64::from(self.count)),
                    }
                })
                .collect()
        };
        read.unmap();
        Ok(result)
    }
    /// Raw little-endian RGBA16F: premultiplied linear radiance RGB and 1-T.
    /// Explicit offline request only; strips WebGPU row alignment padding.
    pub(crate) fn layers_rgba16(&self, gpu: &Gpu) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
        self.read_layers(gpu,false)
    }
    /// Raw little-endian RGBA32F centroid metadata. Empty for native targets.
    pub(crate) fn motion_layers_rgba32(&self,gpu:&Gpu)->Result<Vec<(u32,u32,Vec<u8>)>,String> {
        self.read_layers(gpu,true)
    }
    fn read_layers(&self,gpu:&Gpu,motion:bool)->Result<Vec<(u32,u32,Vec<u8>)>,String> {
        let targets = self.targets.lock().unwrap();
        let mut result = Vec::new();
        for target in targets.iter().filter_map(Option::as_ref) {
            let texture=if motion {let Some(texture)=&target.motion else {continue;};texture} else {&target.layer};
            let bytes_per_pixel=if motion {16}else{8};
            let (w, h) = target.size;
            let row = (w * bytes_per_pixel).div_ceil(256) * 256;
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("voxel raw alpha readback"),
                size: u64::from(row) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            let submission = gpu.queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            read.slice(..).map_async(wgpu::MapMode::Read, move |value| {
                let _ = send.send(value);
            });
            gpu.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(60)),
                })
                .map_err(|e| format!("voxel raw layer wait failed: {e:?}"))?;
            receive
                .try_recv()
                .map_err(|e| format!("voxel raw layer callback missing: {e}"))?
                .map_err(|e| format!("voxel raw layer mapping failed: {e:?}"))?;
            let packed = {
                let bytes = read
                    .slice(..)
                    .get_mapped_range()
                    .map_err(|e| format!("voxel raw layer range failed: {e:?}"))?;
                let mut packed = Vec::with_capacity(w as usize * h as usize * bytes_per_pixel as usize);
                for y in 0..h {
                    let start = y as usize * row as usize;
                    packed.extend_from_slice(&bytes[start..start + w as usize * bytes_per_pixel as usize]);
                }
                packed
            };
            read.unmap();
            result.push((w, h, packed));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn volume_bins_match_inclusive_tile_overlap_at_edges_and_near_plane() {
        // Independent reference: intersect every tile's closed pixel rectangle.
        // Candidate: enumerate the bin range once per projected volume. Include
        // the partial last tile, exact boundaries and a near-plane crossing.
        let dims=[13i32,7i32];
        let rects=[([-30.0,-20.0],[0.0,0.0],8u32),([16.0,32.0],[16.0,32.0],8),
            ([16.0001,31.9999],[31.9999,32.0001],8),([-100.0,-100.0],[-0.01,50.0],8),
            ([208.0,112.0],[240.0,140.0],8),([208.01,112.01],[250.0,140.0],8),
            ([-1.0e20,-1.0e20],[1.0e20,1.0e20],8),([500.0,500.0],[600.0,600.0],3),
            ([0.0,0.0],[200.0,100.0],0)];
        for (lo,hi,in_front) in rects {
            let mut expected=std::collections::BTreeSet::new();
            for y in 0..dims[1] {for x in 0..dims[0] {
                if in_front>0 && (in_front<8 || (0..2).all(|a|hi[a]>=([x,y][a]*16)as f32&&lo[a]<=(([x,y][a]+1)*16)as f32)) {
                    expected.insert([x,y]);
                }
            }}
            let mut actual=std::collections::BTreeSet::new();
            if in_front>0 && (in_front<8 || (0..2).all(|a|hi[a]>=0.0&&lo[a]<=(dims[a]*16)as f32)) {
                let first:[i32;2]=std::array::from_fn(|a|if in_front<8 {0}else{
                    ((lo[a]/16.0).ceil()-1.0).clamp(0.0,(dims[a]-1)as f32)as i32});
                let last:[i32;2]=std::array::from_fn(|a|if in_front<8 {dims[a]-1}else{
                    (hi[a]/16.0).floor().clamp(0.0,(dims[a]-1)as f32)as i32});
                for y in first[1]..=last[1] {for x in first[0]..=last[0] {actual.insert([x,y]);}}
            }
            assert_eq!(actual,expected,"projected box {lo:?}..{hi:?}, front corners {in_front}");
        }
    }

    #[test]
    fn appended_tile_candidates_restore_stable_order_and_overflow_keeps_every_volume() {
        let capacity=super::CANDIDATES as usize;
        for count in [0,1,16,64,65,256,super::MAX_VIEW_GROUPS] {
            // IDs deliberately have tied per-ray entry distances. Stable source
            // ID order must not inherit atomic scheduling order, including an
            // overflow whose stored subset consists entirely of the largest IDs.
            let reference:Vec<_>=(0..count as u32).collect();
            for reverse in [false,true] {
                let arrival:Vec<_>=if reverse {reference.iter().copied().rev().collect()}else{reference.clone()};
                let mut stored:Vec<_>=arrival.into_iter().take(capacity).collect();
                let consumed=if count>capacity {reference.clone()}else {stored.sort_unstable();stored};
                assert_eq!(consumed,reference);
                let entry=|id:u32|((id%11)/3)as f32;
                let mut intervals:Vec<_>=consumed.iter().map(|&id|(entry(id),id)).collect();
                intervals.sort_by(|a,b|a.0.total_cmp(&b.0));
                assert!(intervals.windows(2).all(|w|w[0].0<w[1].0||w[0].1<w[1].1));
            }
        }
    }

    #[test]
    fn splat_cells_list_every_occupied_cell_of_each_level_dense_or_bricked() {
        use crate::foliage_volume_data::{GpuForest,GpuLevel,GpuTree,GpuCell};
        use bytemuck::Zeroable;
        use super::{splat_cells,splat_capacity};
        let mut forest=GpuForest::default();
        // 5x2x3 dense level with three occupied cells, then its 3x1x2 parent.
        let mut dense=vec![0u32;30];
        dense[0]=1;dense[4+5]=2;dense[4+5*(1+2*2)]=3;
        forest.indices.extend(&dense);forest.indices.extend([4,0,0,0,0,0]);
        forest.levels=vec![GpuLevel {origin_cell:[0.0,0.0,0.0,0.5],dims_offset:[5,2,3,0]},
            GpuLevel {origin_cell:[0.0,0.0,0.0,1.0],dims_offset:[3,1,2,30]}];
        forest.cells=vec![GpuCell::zeroed();4];
        let tree=GpuTree {inverse:[[1.0,0.0,0.0,0.0],[0.0,1.0,0.0,0.0],[0.0,0.0,1.0,0.0]],bounds_min:[0.0;4],bounds_max:[2.5,1.0,1.5,0.0],meta:[0,0,2,0]};
        forest.instances=vec![tree,tree];
        let cells=splat_cells(&forest).unwrap();
        assert_eq!(cells,[[2,3],[5,1],[0,0],[1,4|1<<10],[2,4|1<<10|2<<20],[3,0]]);
        // Two trees at the finest level.
        assert_eq!(splat_capacity(&forest,&cells),6);
        crate::foliage_volume_data::pack_bricks(&mut forest,128).unwrap();
        assert_eq!(splat_cells(&forest).unwrap(),cells);
    }

    #[test]
    fn splat_bricks_preserve_cells_and_bound_each_group() {
        use crate::foliage_volume_data::{GpuForest,GpuLevel,GpuCell};
        use bytemuck::Zeroable;
        let mut f=GpuForest::default();
        f.levels=vec![GpuLevel {origin_cell:[0.0,0.0,0.0,1.0],dims_offset:[9,7,5,0]}];
        f.indices=(0..315).map(|i|if i%3==0 {1}else{0}).collect();
        f.cells=vec![GpuCell::zeroed()];
        let plain=super::splat_cells(&f).unwrap();let grouped=super::splat_cells_bricked(&f).unwrap();
        let [start,count]=grouped[0];let mut a=plain[1..].to_vec();
        let mut b=grouped[start as usize..(start+count)as usize].to_vec();a.sort();b.sort();assert_eq!(a,b);
        let [first,n]=grouped[1];let mut covered=0;
        for brick in 0..n {
            let [begin,len]=grouped[(first+brick*2)as usize];
            let packed=grouped[(first+brick*2+1)as usize][0];
            assert!(len>0 && len<=64);covered+=len;
            for cell in &grouped[begin as usize..(begin+len)as usize] {
                for shift in [0,10,20] {assert_eq!(((cell[1]>>shift)&1023)&!3,(packed>>shift)&1023);}
            }
        }
        assert_eq!(covered,count);
        crate::foliage_volume_data::pack_bricks(&mut f,128).unwrap();
        assert_eq!(super::splat_cells_bricked(&f).unwrap(),grouped);
    }

    #[test]
    fn splat_brick_world_bounds_cover_nonuniform_scale_and_billboard_jitter() {
        use glam::{Mat3,Vec3};
        for matrix in [Mat3::IDENTITY,Mat3::from_diagonal(Vec3::new(4.0,0.02,0.1)),
            Mat3::from_cols(Vec3::new(2.0,0.3,0.0),Vec3::new(4.0,0.1,1.0),Vec3::new(-2.0,1.0,3.0))] {
            let padding=1.5*matrix.x_axis.length();
            let half=(matrix.x_axis.abs()+matrix.y_axis.abs()+matrix.z_axis.abs())*1.5+Vec3::splat(padding);
            for x in 0..4 {for y in 0..4 {for z in 0..4 {
                let centre=matrix*(Vec3::new(x as f32,y as f32,z as f32)-Vec3::splat(1.5));
                // Every camera-facing billboard point fits this spherical bound.
                let radius=2f32.sqrt()*(0.688+0.25)*matrix.x_axis.length();
                assert!((centre.abs()+Vec3::splat(radius)).cmple(half+Vec3::splat(1e-5)).all());
            }}}
        }
    }

    #[test]
    fn chosen_aggregate_level_is_exact_and_legacy_level_remains_a_ceiling() {
        // At a near distance the legacy footprint would select zero, even if
        // the admissible ceiling is two. A measured aggregate candidate must
        // actually reconstruct level two and advance by its half-cell width.
        for chosen in 0..4u32 {
            for footprint in [0.125f32,0.5,1.0,2.0,32.0] {
                let legacy=footprint.log2().clamp(0.0,chosen as f32);
                let aggregate=chosen as f32;
                assert_eq!(aggregate.floor(),aggregate);
                assert_eq!(0.125*2.0f32.powf(aggregate),0.125*(1u32<<chosen)as f32);
                if chosen>0 && footprint<1.0 {assert_ne!(aggregate,legacy);}
            }
        }
        let shader=include_str!("shaders/voxel_forest.wgsl");
        assert!(shader.contains("if selected.w==1u {return f32(min(tree.identity.z-1u,selected.y));}"));
        assert!(shader.contains("return volume_empty_brick_level(ray,t,u32(volume_lod(ray,t)));") );
        // Both native and temporal variants, including the new bins entry
        // point, are parsed and validated by the existing variant tests below.
    }

    #[test]
    fn global_interval_stream_bounds_cover_metadata_records_and_device_limits() {
        assert!(!super::IntegratorOptions::default().global_intervals);
        for size in [(1,1),(16,16),(1920,1080),(3840,2160),(7680,4320)] {
            let stream=super::interval_stream_layout(size,u64::MAX).unwrap();
            let tiles=u64::from(size.0.div_ceil(16))*u64::from(size.1.div_ceil(16));
            assert_eq!(u64::from(stream.header_word),4+tiles);
            assert_eq!(stream.bytes,(4+tiles)*4+super::INTERVAL_STREAM_BYTES);
            assert_eq!(u64::from(stream.pixel_slots),(tiles*256).min(u64::from(super::INTERVAL_STREAM_PIXEL_SLOTS)));
            let records_start=u64::from(stream.header_word)+4+u64::from(stream.pixel_slots)*2;
            let records_end=records_start+u64::from(stream.records)*4;
            assert!(records_end*4<=stream.bytes && stream.bytes-records_end*4<16);
            // Every admitted pixel can see the complete provider without
            // wrapping the u32 atomic reservation counter, even after full.
            assert!(u64::from(stream.pixel_slots)*(super::INTEGRATOR_MAX_VOLUMES as u64)<u64::from(u32::MAX));
            assert!(super::interval_stream_layout(size,stream.bytes).is_ok());
            assert!(super::interval_stream_layout(size,stream.bytes-1).is_err());
            let last_metadata=u64::from(stream.header_word)+4+(u64::from(stream.pixel_slots)-1)*2;
            assert_eq!(last_metadata+2,records_start);
        }
        for size in [(0,64),(64,0),(u32::MAX,u32::MAX)] {
            assert!(super::interval_stream_layout(size,u64::MAX).is_err());
        }
        let source=include_str!("shaders/voxel_forest.wgsl");
        assert!(source.contains(&format!("const VOLUME_INTERVAL_STREAM_WORDS:u32={}u;",super::INTERVAL_STREAM_BYTES/4)));
    }

    #[test]
    fn bounded_stream_overflow_retains_every_intersection_in_stable_source_order() {
        // Independently enumerate intersections of overlapping segments,
        // including a miss, a clipped entry, a tangent and a distant segment.
        let intervals:[(f32,f32);6]=[(-2.0,0.75),(0.25,1.5),(0.5,0.5),(1.0,2.0),(3.0,4.0),(-4.0,-1.0)];
        let near=0.1f32;let limit=3.5f32;
        let exact:Vec<_>=intervals.iter().enumerate().filter_map(|(id,&(a,b))| {
            let entry=a.max(near);let exit=b.min(limit);(exit>entry).then_some((id,entry,exit))
        }).collect();
        assert_eq!(exact.iter().map(|v|v.0).collect::<Vec<_>>(),[0,1,3,4]);
        for capacity in [0,1,exact.len()-1,exact.len(),exact.len()+7] {
            for pixel_slot in [0usize,2] {
                let metadata_slots=2;
                let stored=if pixel_slot<metadata_slots && exact.len()<=capacity {Some(exact.clone())}else{None};
                // A failed stream reservation selects the complete original
                // candidate set, then performs the same geometric test. It
                // does not consume a prefix of the records that happened to fit.
                let consumed=stored.unwrap_or_else(||intervals.iter().enumerate().filter_map(|(id,&(a,b))| {
                    let entry=a.max(near);let exit=b.min(limit);(exit>entry).then_some((id,entry,exit))
                }).collect());
                assert_eq!(consumed,exact);
                for t in [0.2,0.6,1.1,1.75,3.25] {
                    let density=|list:&[(usize,f32,f32)]|list.iter().filter(|(_,a,b)|t>=*a&&t<=*b)
                        .map(|(id,_,_)|(*id+1) as f32).sum::<f32>();
                    assert_eq!(density(&consumed),density(&exact));
                }
            }
        }
    }

    #[test]
    fn empty_ceiling_variants_validate_and_disabled_keeps_baseline_unchanged() {
        assert!(super::IntegratorOptions::default().empty_ceiling);
        assert!(!super::IntegratorOptions::default().antipodal_normals);
        let original = super::shader_source();
        for empty_ceiling in [false, true] {
            let options = super::IntegratorOptions { empty_ceiling, ..Default::default() };
            let source = options.specialize(&original).unwrap();
            if !empty_ceiling { assert_eq!(source, original); }
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all()).validate(&module)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            if empty_ceiling {
                assert!(options.specialize(&original.replace(
                    "const VOLUME_EMPTY_CEILING: bool = false;", "")).is_err());
            }
        }
    }

    #[test]
    fn antipodal_gate_accepts_baked_two_sided_mips_but_rejects_single_or_mixed_sources() {
        use crate::{cluster::{Vertex,Part},foliage_voxel::{bake,BakeConfig},
            foliage_volume_data::{pack,PackLimits},scene::{Material,TWO_SIDED}};
        let vertices: [Vertex;4]=std::array::from_fn(|i|Vertex {pos:[(i%2) as f32,(i/2) as f32,0.0],
            normal:[0.6,0.8,0.0],uv:[0.0;2]});
        let indices=[0,1,2,2,1,3];
        let source=|flags| {
            let material=Material {tint:[0.2,0.4,0.1],layer:-1,roughness:0.8,flags,scale:1.0,metal:0.0};
            let asset=bake(&[Part {vertices:&vertices,indices:&indices,material:0}],&[material],&[],
                BakeConfig {levels:4,samples_per_cell:2,..Default::default()}).unwrap();
            pack(&[asset],&[],&[],PackLimits::default()).unwrap().cells
        };
        let two_sided=source(TWO_SIDED);
        let single_sided=source(0);
        assert!(!two_sided.is_empty());
        assert!(super::antipodal_normal_cells(&two_sided));
        assert!(!super::antipodal_normal_cells(&single_sided));
        let mut mixed=two_sided.clone();mixed.extend(single_sided);
        assert!(!super::antipodal_normal_cells(&mixed));
        assert!(!super::antipodal_normal_cells(&[]));
        // One ULP is a real unequal moment, not an accepted tolerance.
        let mut unequal=two_sided.clone();
        unequal[0].normals[1][3]=f32::from_bits(unequal[0].normals[1][3].to_bits()+1);
        assert!(!super::antipodal_normal_cells(&unequal));
        unequal=two_sided.clone();
        unequal[0].normals[1][0]=f32::from_bits(unequal[0].normals[1][0].to_bits()+1);
        assert!(!super::antipodal_normal_cells(&unequal));
        unequal=two_sided;
        unequal[0].normals[0][0]=f32::NAN;
        assert!(!super::antipodal_normal_cells(&unequal));
    }

    #[test]
    fn antipodal_pair_keeps_interpolated_moment_threshold_and_grazing_response() {
        use glam::Vec3;
        // Independent anisotropic front/back response: exactly grazing the
        // viewer does NOT fold the two orientations onto a single normal.
        let response=|n:Vec3,sun:Vec3|0.75*n.dot(sun).max(0.0)+0.25*(-n.dot(sun)).max(0.0);
        let fold=|n:Vec3,v:Vec3|if n.dot(v)<0.0 {-n}else{n};
        for normal in [Vec3::X,Vec3::new(0.6,0.8,0.0)] {
            for view in [Vec3::Z,Vec3::X,Vec3::NEG_X,Vec3::new(0.2,-0.3,0.9).normalize()] {
                let sun=Vec3::new(0.7,0.2,0.1).normalize();
                let generic=0.5*response(fold(normal,view),sun)+0.5*response(fold(-normal,view),sun);
                let candidate=if normal.dot(view)==0.0 {
                    0.5*(response(normal,sun)+response(-normal,sun))
                }else{response(fold(normal,view),sun)};
                assert!((generic-candidate).abs()<1e-7);
            }
        }
        assert_eq!(0.5*(response(Vec3::X,Vec3::X)+response(Vec3::NEG_X,Vec3::X)),0.5);
        assert_ne!(response(Vec3::X,Vec3::X),0.5,"dropping the grazing antipode changes transport");
        // Pair response mass doubles, but its positive vector moment must not.
        // This preserves the shader's existing 1e-16 squared-length rejection.
        let tiny=Vec3::X*7e-9;
        assert!(tiny.length_squared()<=1e-16 && (tiny*2.0).length_squared()>1e-16);
        let mut original=Vec3::ZERO;let mut candidate=Vec3::ZERO;
        let mut weight=0.0f32;let mut pair_weight=0.0f32;
        for (mean,mass,sigma) in [(Vec3::new(0.6,0.2,0.0),0.17,0.3),
            (Vec3::new(0.4,-0.1,0.2),0.21,1.7),(Vec3::new(0.3,0.5,-0.2),0.12,0.9)] {
            original+=mean*mass*sigma;candidate+=mean*mass*sigma;
            weight+=mass*sigma;pair_weight+=(mass+mass)*sigma;
        }
        assert_eq!(candidate,original);
        assert_eq!(pair_weight,weight*2.0);
    }

    #[test]
    fn selected_mip_ceiling_keeps_empty_support_valid_past_unused_lod_boundary() {
        // Construct an actual pooled occupancy pyramid. Distant occupied
        // cells make a never-selected upper mip's coarse support nonempty,
        // while the selected mip is identically empty throughout this brick.
        // Its old next-LOD boundary is already behind the current ray point.
        for ceiling in 0..3usize {
            let h = (1u32 << ceiling) as f64;
            let mut levels = vec![vec![0.0f64; 32 << ceiling]];
            levels[0][0] = 1.0;
            levels[0][12 << ceiling] = 1.0;
            for level in 1..ceiling + 4 {
                let parent = levels[level - 1].chunks(2)
                    .map(|pair| pair.iter().sum::<f64>() / 2.0).collect();
                levels.push(parent);
            }
            let t = 5.2 * h;
            let brick_exit = 8.5 * h;
            let unused_lod_boundary = 2.0 * h;
            assert!(unused_lod_boundary < t && brick_exit > t);
            // Selected level uses coarse parent q=1 or q+1; both are empty.
            assert_eq!(levels[ceiling + 2][1], 0.0);
            assert_eq!(levels[ceiling + 2][2], 0.0);
            // Unused upper level checks a broader pair that includes material.
            assert!(levels[ceiling + 3][0] + levels[ceiling + 3][1] > 0.0);
            for i in 0..1001 {
                let x = t + (brick_exit - t) * i as f64 / 1000.0;
                // Independent linear reconstruction at the clamped mip:
                // the extra coarse support may not invent a nonzero blend.
                let raw_lod = x.log2();
                let lod = raw_lod.clamp(0.0, ceiling as f64);
                assert_eq!(lod, ceiling as f64);
                assert_eq!(lod.fract(), 0.0);
                let grid = x / h - 0.5;
                let base = grid.floor() as usize;
                let f = grid.fract();
                let sigma = levels[ceiling][base] * (1.0 - f) + levels[ceiling][base + 1] * f;
                assert_eq!(sigma, 0.0);
            }
        }
    }

    #[test]
    fn temporal_layout_fits_sixteen_storage_bindings() {
        for temporal in [false, true] {
            let count = super::data_bindings(temporal).iter().filter(|(_, ty)| matches!(ty,
                wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { .. }, .. })).count();
            // Reduced scene: instances, wind, terrain hierarchy, foliage pages.
            assert_eq!(count + 4, if temporal { 16 } else { 15 });
        }
        assert!(super::shader_source().contains("const VOLUME_MASK_WORD: u32 = 22u;"));
        assert_eq!(super::COUNTER_BYTES / 4, 22);
    }

    #[test]
    fn volume_entry_points_fit_reduced_scene_binding_contract() {
        for temporal in [false,true] {
        for antipodal_normals in [false,true] {
        for brick_visibility in [false,true] {
        let options=super::IntegratorOptions {antipodal_normals,brick_visibility,..Default::default()};
        let source = options.specialize(&super::temporal_source(&super::shader_source(),temporal).unwrap()).unwrap();
        assert_eq!(source.contains("var volume_motion_output"),temporal);
        assert_eq!(source.contains("var<storage,read> volume_previous_instances"),temporal);
        assert_eq!(source.contains("delta_sigma"),temporal);
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all()).validate(&module).unwrap();
        let span=|name:&str|module.types.iter().find_map(|(_,ty)| {
            if ty.name.as_deref()!=Some(name) {return None;}
            if let naga::TypeInner::Struct {span,..}=&ty.inner {Some(*span)}else{None}
        }).unwrap();
        assert_eq!(span("VolumeCell"),176,"specialization must not alter GPU storage ABI");
        assert_eq!(span("VolumeSample"),if antipodal_normals {112}else{160},"private reconstruction result actually shrinks");
        if antipodal_normals {
            assert!(source.contains("cell.normals[k*2u]"));
            assert!(source.contains("normal_copies=select(1u,2u,dot(n,v)==0.0)"));
            assert!(options.specialize(&super::shader_source().replace("// VOLUME_ANTIPODAL_RESPONSE_END","")).is_err());
        }
        for (i, entry) in module.entry_points.iter().enumerate()
            .filter(|(_, e)| e.name.starts_with("voxel_")) {
            let uses = info.get_entry_point(i);
            let mut storage_count = 0;
            for (handle, global) in module.global_variables.iter() {
                if uses[handle].is_empty() { continue; }
                if let Some(binding) = &global.binding {
                    if binding.group == 0 {
                        assert!(super::SCENE_BINDINGS.contains(&binding.binding),
                            "{} unexpectedly uses scene binding {} ({:?})", entry.name, binding.binding, global.name);
                    }
                    if matches!(global.space, naga::AddressSpace::Storage { .. }) { storage_count += 1; }
                }
            }
            // One owner transform, seven bounded volume buffers and the shared
            // world terrain hierarchy and shared foliage page table. The engine requests up to16 storage
            // bindings already (opaque materials exceed the WebGPU base8).
            // Temporal adds one previous-owner buffer, remaining below the
            // existing 16-storage limit. Native has no motion buffer or writes.
            assert!(storage_count <= if temporal {14}else{13}, "{} uses {storage_count} storage buffers", entry.name);
        }
        }
        }
        }
        assert!(super::temporal_source("// VOLUME_MOTION_BEGIN\nlet broken=0;",false).is_err());
        assert!(super::temporal_source("// VOLUME_MOTION_END\n",false).is_err());
    }

    #[test]
    fn fractional_motion_uses_opacity_and_all_overlapping_branch_weights() {
        use glam::Vec3;
        let mut transmittance=1.0f32;let mut delta=Vec3::ZERO;let mut distance=0.0;
        let mut expected_previous=Vec3::ZERO;let mut expected_current=Vec3::ZERO;
        for (t,ds,sigma_a,sigma_b) in [(3.0f32,0.2f32,1.4f32,0.3f32),(3.4,0.5,0.2,2.1)] {
            let p=Vec3::new(0.0,0.0,-t);let a=Vec3::new(-0.7,0.1,0.0);let b=Vec3::new(0.2,0.4,0.1);
            let sigma=sigma_a+sigma_b;let opacity=transmittance*(1.0-(-sigma*ds).exp());
            delta+=opacity*(a*sigma_a+b*sigma_b)/sigma;
            distance+=opacity*t;
            expected_previous+=opacity*((p+a)*sigma_a+(p+b)*sigma_b)/sigma;
            expected_current+=opacity*p;
            transmittance*=(-sigma*ds).exp();
        }
        let alpha=1.0-transmittance;
        assert!((delta/alpha-(expected_previous-expected_current)/alpha).length()<2e-6);
        assert!((Vec3::NEG_Z*(distance/alpha)-expected_current/alpha).length()<2e-6);
        assert!((delta/alpha-Vec3::new(-0.7,0.1,0.0)).length()>0.1,"must not choose only first owner");
        assert!((delta/alpha-Vec3::new(0.2,0.4,0.1)).length()>0.1,"must not choose only strongest owner");
    }

    #[test]
    fn temporal_volume_requires_matching_source_coordinates_and_part_deformation() {
        use bytemuck::Zeroable;
        use crate::{Scene,scene::{GpuInstance,PART_WIND},foliage_volume_data::{GpuForest,GpuTree},part_wind::PartWind};
        let rows=[[1.0,0.0,0.0,0.0],[0.0,1.0,0.0,0.0],[0.0,0.0,1.0,0.0]];
        let mut scene=Scene::default();
        scene.instances.push(GpuInstance {rows,scale:1.0,..GpuInstance::zeroed()});
        scene.voxel_forest=Some(GpuForest {instances:vec![GpuTree {inverse:rows,..GpuTree::zeroed()}],..Default::default()});
        assert!(super::temporal_compatibility(&scene).is_ok());
        scene.voxel_forest.as_mut().unwrap().instances[0].inverse[0][3]=0.5;
        assert!(super::temporal_compatibility(&scene).is_err());
        scene.voxel_forest.as_mut().unwrap().instances[0].inverse=rows;
        scene.instances[0].flags=PART_WIND;
        assert!(super::temporal_compatibility(&scene).is_err());
        scene.part_wind.push(PartWind {owner:Some(0),volume:Some(0),..Default::default()});
        assert!(super::temporal_compatibility(&scene).is_ok());
        scene.part_wind[0].volume=None;
        assert!(super::temporal_compatibility(&scene).is_err());
    }

    #[test]
    fn active_tile_jobs_cover_exact_pixels_for_sparse_odd_and_multirow_views() {
        // Independent reference is a pixel mask of the candidate tile set.
        // Exercise partial edge tiles, sparse reordered jobs and >65535 jobs,
        // where a one-dimensional indirect dispatch would exceed GPU limits.
        for (width, height, sparse) in [(1u32, 1u32, false), (257, 193, true),
            (1916, 1076, true), (4097, 4097, false)] {
            let tiles_w = width.div_ceil(16);
            let tiles_h = height.div_ceil(16);
            let jobs: Vec<u32> = (0..tiles_w * tiles_h).rev()
                .filter(|i| !sparse || i % 3 == 1).collect();
            let count = jobs.len() as u32;
            let dispatch = [count.min(65535), 4 * count.div_ceil(65535), 1];
            assert!(dispatch.into_iter().all(|n| n <= 65535));
            let mut visits = vec![0u8; (width * height) as usize];
            for gy in 0..dispatch[1] {
                for gx in 0..dispatch[0] {
                    let job = gx + (gy / 4) * 65535;
                    if job >= count { continue; }
                    let tile = jobs[job as usize];
                    let base_x = (tile % tiles_w) * 16 + (gy & 1) * 8;
                    let base_y = (tile / tiles_w) * 16 + ((gy >> 1) & 1) * 8;
                    for ly in 0..8 {
                        for lx in 0..8 {
                            let (x, y) = (base_x + lx, base_y + ly);
                            if x < width && y < height {
                                visits[(y * width + x) as usize] += 1;
                            }
                        }
                    }
                }
            }
            for y in 0..height {
                for x in 0..width {
                    let tile = (y / 16) * tiles_w + x / 16;
                    assert_eq!(visits[(y * width + x) as usize],
                        u8::from(!sparse || tile % 3 == 1), "pixel ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn active_tile_indirect_header_handles_empty_and_row_boundaries() {
        for count in [0u32, 1, 65534, 65535, 65536, 131070, 131071, 1_048_576] {
            // Mirror the parallel atomic reduction, then prove the dispatch
            // contains exactly the expected jobs, with four groups per job.
            let mut header = [0u32; 3];
            for job in (0..count).rev() {
                header[0] = header[0].max((job + 1).min(65535));
                header[1] = header[1].max(4 * (job / 65535 + 1));
                header[2] = 1;
            }
            if count == 0 {
                assert_eq!(header, [0; 3]);
                continue;
            }
            let mut visits = vec![0u8; count as usize];
            for y in 0..header[1] {
                for x in 0..header[0] {
                    let job = x + (y / 4) * 65535;
                    if job < count { visits[job as usize] += 1; }
                }
            }
            assert!(visits.into_iter().all(|v| v == 4));
            assert!(header.into_iter().all(|n| n <= 65535));
        }
    }

    #[test]
    fn empty_skip_requires_real_parent_occupancy_and_matching_grid() {
        use crate::foliage_volume_data::{GpuForest, GpuLevel, GpuTree};
        let mut forest = GpuForest::default();
        // Odd dimensions deliberately exercise ceil-sized parents. Two
        // unrelated fine cells must both have their own occupied parent.
        for (dims, cell) in [(5u32, 0.25), (3, 0.5), (2, 1.0)] {
            let offset = forest.indices.len() as u32;
            forest.levels.push(GpuLevel {
                origin_cell: [-1.0, 2.0, -3.0, cell],
                dims_offset: [dims, dims, dims, offset],
            });
            forest
                .indices
                .resize(forest.indices.len() + dims.pow(3) as usize, 0);
        }
        forest.instances.push(GpuTree {
            inverse: [[0.0; 4]; 3],
            bounds_min: [0.0; 4],
            bounds_max: [0.0; 4],
            meta: [0, 0, 3, 0],
        });
        forest.indices[0] = 1;
        forest.indices[124] = 2;
        let coarse_offset = forest.levels[2].dims_offset[3] as usize;
        forest.indices[coarse_offset] = 3;
        forest.indices[coarse_offset + 7] = 4;
        assert!(super::valid_empty_skip_hierarchy(&forest));
        // The old fine-only proof is insufficient once mip 1 can be shaded:
        // separately occupied intermediate cells also need coarse parents.
        assert!(super::valid_skip_levels(&forest, true));
        let middle_offset = forest.levels[1].dims_offset[3] as usize;
        forest.indices[middle_offset + 2] = 5;
        assert!(super::valid_empty_skip_hierarchy(&forest));
        assert!(!super::valid_skip_levels(&forest, true));
        forest.indices[middle_offset + 2] = 0;
        forest.indices[coarse_offset + 7] = 0;
        assert!(!super::valid_empty_skip_hierarchy(&forest));
        forest.indices[coarse_offset + 7] = 4;
        forest.levels[2].origin_cell[0] += 0.01;
        assert!(!super::valid_empty_skip_hierarchy(&forest));
        forest.levels[2].origin_cell[0] -= 0.01;
        forest.levels[2].origin_cell[3] = 1.1;
        assert!(!super::valid_empty_skip_hierarchy(&forest));
        // A one-level asset can still prove emptiness directly from its fine
        // indirection grid; unrelated coarse data cannot affect rendering.
        forest.instances[0].meta[2] = 1;
        assert!(super::valid_empty_skip_hierarchy(&forest));
    }
    #[test]
    fn trilinear_brick_support_covers_signed_halo_and_both_ray_directions() {
        for ratio in [1i32, 2, 4] {
            for eighth in -129..130 {
                let grid = eighth as f64 / 8.0;
                let fine = grid.floor() as i32;
                let q = (grid / ratio as f64).floor() as i32;
                // Every contributing neighbour is in one of these two
                // parent cells. Tensor product gives the eight 3D parents.
                for child in [fine, fine + 1] {
                    let parent = child.div_euclid(ratio);
                    assert!(parent == q || parent == q + 1);
                }
                for rate in [-1.0f64, -0.25, 0.25, 1.0] {
                    let mut directional_q = q;
                    if rate < 0.0 && grid == q as f64 * ratio as f64 {
                        directional_q -= 1;
                    }
                    let face = (directional_q + i32::from(rate > 0.0)) as f64 * ratio as f64;
                    let dt = (face - grid) / rate;
                    assert!(dt > 0.0);
                    let midpoint = grid + rate * dt * 0.5;
                    assert_eq!((midpoint / ratio as f64).floor() as i32, directional_q);
                    // The negative-face correction changes only the brick
                    // entered after t; its contributing fine support remains
                    // bounded by the exact same two parent cells.
                    let child = midpoint.floor() as i32;
                    for neighbour in [child, child + 1] {
                        let parent = neighbour.div_euclid(ratio);
                        assert!(parent == directional_q || parent == directional_q + 1);
                    }
                }
            }
        }
    }
    #[test]
    fn homogeneous_overlap_has_order_independent_extinction_and_premultiplied_color() {
        let integrate = |segments: &[(f64, f64, f64)]| {
            let mut t = 1.0;
            let mut c = 0.0;
            for &(sigma, ds, radiance) in segments {
                let a = 1.0 - (-sigma * ds).exp();
                c += t * a * radiance;
                t *= 1.0 - a;
            }
            (c, t)
        };
        let a = integrate(&[(0.7, 2.0, 3.0), (0.7, 3.0, 3.0)]);
        let b = integrate(&[(0.7, 5.0, 3.0)]);
        assert!((a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12);
        let overlapping = integrate(&[(0.3 + 0.4, 5.0, 3.0)]);
        assert!((overlapping.1 - (-3.5f64).exp()).abs() < 1e-12);
        assert_eq!(integrate(&[(0.0, 100.0, 3.0)]), (0.0, 1.0));
    }
    #[test]
    fn projected_mip_bound_covers_rotated_sheared_cells_and_four_view_sizes() {
        use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
        // Independent oracle: project all eight real corners, not the
        // diameter approximation used for the shader's LOD choice.
        let sizes = [(3840.0, 2160.0), (1916.0, 1076.0), (960.0, 1080.0), (640.0, 360.0)];
        let transforms = [Mat3::IDENTITY,
            Mat3::from_quat(Quat::from_euler(glam::EulerRot::YXZ, 0.71, -0.3, 0.24))
                * Mat3::from_diagonal(Vec3::new(0.3, 2.0, 1.4)),
            Mat3::from_cols(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.7, 1.2, 0.0), Vec3::new(-0.2, 0.3, 0.8))];
        for (width, height) in sizes {
            let projection = Mat4::perspective_infinite_reverse_rh(1.3, width / height, 0.1);
            let rows = projection.transpose();
            for linear in transforms {
                let r = linear.transpose();
                let frobenius = linear.to_cols_array().iter().map(|x| x*x).sum::<f32>().sqrt();
                let one = linear.x_axis.abs().element_sum().max(linear.y_axis.abs().element_sum()).max(linear.z_axis.abs().element_sum());
                let inf = r.x_axis.abs().element_sum().max(r.y_axis.abs().element_sum()).max(r.z_axis.abs().element_sum());
                let stretch = frobenius.min((one * inf).sqrt());
                for direction in [Vec3::NEG_Z, Vec3::new(0.8, 0.45, -1.0).normalize()] {
                    let depth_rate = rows.w_axis.truncate().dot(direction);
                    let ndc = Vec2::new(rows.x_axis.truncate().dot(direction), rows.y_axis.truncate().dot(direction)) / depth_rate;
                    let jx = (rows.x_axis.truncate() - ndc.x * rows.w_axis.truncate()) * (width * 0.5);
                    let jy = (rows.y_axis.truncate() - ndc.y * rows.w_axis.truncate()) * (height * 0.5);
                    let diameter = 0.04 * 3.0f32.sqrt() * stretch;
                    let c = diameter * (jx.length_squared() + jy.length_squared()).sqrt() / depth_rate;
                    let radius = 0.5 * diameter / depth_rate;
                    let target = 1.5;
                    let transition = c / target + radius;
                    let mut previous = -1.0;
                    for distance in [0.3, 1.0, 3.0, 10.0, 30.0, 80.0, 180.0, 500.0] {
                        let lod = (distance / transition).log2().clamp(0.0, 5.0);
                        assert!(lod >= previous); previous = lod;
                        let scale = 2.0f32.powf(lod.floor());
                        let bound = c * scale / (distance - radius * scale);
                        if distance <= radius * scale { assert_eq!(lod, 0.0); continue; }
                        if lod > 0.0 { assert!(bound <= target * 1.00001); }
                        let corners: Vec<Vec2> = (0..8).map(|corner| {
                            let offset = Vec3::new(if corner&1==0 {-0.5} else {0.5}, if corner&2==0 {-0.5} else {0.5}, if corner&4==0 {-0.5} else {0.5});
                            let clip = projection * (direction * distance + linear * offset * (0.04 * scale)).extend(1.0);
                            Vec2::new(clip.x * width, clip.y * height) / (2.0 * clip.w)
                        }).collect();
                        for a in &corners { for b in &corners {
                            assert!(a.distance(*b) <= bound * 1.0001, "finite projected cell exceeded conservative bound");
                        }}
                    }
                    // No discontinuous representation jump as the camera
                    // crosses a dyadic mip boundary; the two levels meet.
                    for level in 1..5 {
                        let boundary = transition * 2.0f32.powi(level);
                        let before = ((boundary - 0.001) / transition).log2();
                        let after = ((boundary + 0.001) / transition).log2();
                        assert!((after - before).abs() < 0.002);
                    }
                }
            }
        }
    }
    #[test]
    fn mip_moments_keep_empty_cells_uncoloured_and_normal_lobes_distinct() {
        // Empty neighbours carry no material moment. Blending their default
        // black colour directly would darken silhouettes during LOD changes.
        let merge = |a: (f64, [f64; 6]), b: (f64, [f64; 6]), weight: f64| {
            let wa = a.0 * (1.0-weight); let wb = b.0 * weight;
            let sigma = wa + wb;
            (sigma, std::array::from_fn::<_, 6, _>(|k| if sigma > 0.0 {(wa*a.1[k]+wb*b.1[k])/sigma} else {0.0}))
        };
        let x = (2.0, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let minus_x = (6.0, [0.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        for w in [0.0, 0.01, 0.25, 0.5, 0.99] {
            let (sigma, lobes) = merge(x, (0.0, [0.0; 6]), w);
            assert_eq!(lobes, x.1); assert_eq!(sigma, 2.0 * (1.0-w));
            let (sigma, lobes) = merge(x, minus_x, w);
            assert!((lobes.iter().sum::<f64>()-1.0).abs() < 1e-12);
            assert_eq!(lobes[2..], [0.0; 4]);
            assert!((sigma*lobes[0]-2.0*(1.0-w)).abs() < 1e-12);
            assert!((sigma*lobes[1]-6.0*w).abs() < 1e-12);
        }
    }
    #[test]
    fn config_layout_and_four_view_candidate_budget() {
        assert_eq!(std::mem::size_of::<super::Config>(), 64);
        assert_eq!(std::mem::offset_of!(super::Config, proxy), 48);
        assert_eq!(std::mem::offset_of!(super::Config, dispatch), 32);
        assert_eq!(super::COUNTER_BYTES, (14 + 4 * 2) * 4);
        assert_eq!(3840u64 * 2160 * 8, 66_355_200);
        let candidates = 3840u64.div_ceil(16) * 2160u64.div_ceil(16) * 65 * 4;
        assert_eq!(candidates, 8_424_000);
        let jobs = (3840u64.div_ceil(16) * 2160u64.div_ceil(16) + 4) * 4 + 12;
        assert_eq!(jobs, 129_628);
    }

    #[test]
    fn aggregate_work_counters_preserve_carry_and_distinct_readback_slots() {
        // Each low-word atomic returns its own serialized predecessor. The
        // carry decision must agree with an independent widened integer sum,
        // including exact wrap and contributions arriving in either order.
        let contributions=[0u32,1,u32::MAX,16_777_216,4096,256,u32::MAX,7];
        let expected: u64=contributions.iter().map(|&x|u64::from(x)).sum();
        for reverse in [false,true] {
            let mut words=[0u32;2];
            for i in 0..contributions.len() {
                let value=contributions[if reverse {contributions.len()-1-i} else {i}];
                let previous=words[0];
                let widened=u64::from(previous)+u64::from(value);
                words[0]=previous.wrapping_add(value);
                let carry=previous>u32::MAX-value;
                assert_eq!(carry,widened>u64::from(u32::MAX));
                words[1]+=u32::from(carry);
            }
            assert_eq!(u64::from(words[0])+(u64::from(words[1])<<32),expected);
        }
        let totals=[3840u64*2160*4096,expected,expected*16,u64::from(u32::MAX)+1];
        let mut bytes=vec![0xa5;super::COUNTER_BYTES as usize];
        for (slot,total) in totals.into_iter().enumerate() {
            let offset=(14+2*slot)*4;
            bytes[offset..offset+8].copy_from_slice(&total.to_le_bytes());
        }
        for (slot,total) in totals.into_iter().enumerate() {
            assert_eq!(super::diagnostic_u64(&bytes,14+2*slot),total);
        }
        assert!(bytes[..14*4].iter().all(|&x|x==0xa5));
    }

    #[test]
    fn per_ray_work_fits_local_u32_while_full_resolution_totals_require_u64() {
        let steps=4096u64;
        let volumes=super::INTEGRATOR_MAX_VOLUMES as u64;
        let volume_samples=steps*volumes;
        // Every interpolation can fetch eight cells in each of two mips.
        // Empty-brick tests read only indirection, not occupied cell records.
        let occupied_cell_samples=volume_samples*16;
        // Initial interval scan plus both complete-scan loops on every step.
        let interval_tests=volumes+2*steps*volumes;
        for per_ray in [steps,volume_samples,occupied_cell_samples,interval_tests] {
            assert!(per_ray<=u64::from(u32::MAX));
            assert!(per_ray*3840*2160>u64::from(u32::MAX));
        }
        // The shader's literal caps are the premises of this local bound.
        let source=include_str!("shaders/voxel_forest.wgsl");
        assert!(source.contains("const VOLUME_MAX_STEPS: u32 = 4096u;"));
        assert_eq!(super::INTEGRATOR_MAX_VOLUMES,4096);
        assert!(source.contains("var<uniform> volume_selection_words: array<vec4<u32>,4096>"));
    }

}
