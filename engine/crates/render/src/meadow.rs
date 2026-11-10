//! Procedural meadow generated, culled, animated and drawn entirely on the GPU.
//! The CPU supplies immutable terrain and a small shared plant recipe library.
//! Per-view grids are bounded; world-cell identity does not depend on the grid
//! origin or compacted draw slot. The first implementation uses source mesh
//! LODs; it does not claim scalable source-derived voxel aggregation yet.
use crate::{wind::WindState, Gpu, Scene};
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;
use wgpu::util::DeviceExt;
use crate::frame::resource_cache::{BindGroups, TextureViews};
#[path = "meadow_group_cache.rs"]
pub mod group_cache;
#[path = "meadow_group_gpu.rs"]
mod group_gpu;
#[path = "meadow_visibility.rs"]
mod visibility;
#[path = "meadow_half.rs"]
mod half;
#[path = "meadow_spatial_filter.rs"]
mod spatial_filter;
#[path = "meadow_capture.rs"]
mod capture;
#[path = "meadow_contact.rs"]
mod contact;
pub use contact::Stats as MeadowEdgeShadowStats;
pub use capture::{MeadowAggregateRaw, MeadowRawLayer};
#[cfg(test)]
#[path = "meadow_sample_prune_tests.rs"]
mod sample_prune_tests;
#[cfg(test)]
#[path = "meadow_wind_tests.rs"]
mod wind_tests;
#[cfg(test)]
#[path = "meadow_pool_tests.rs"]
mod pool_tests;
pub use group_gpu::Stats as GroupCacheStats;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Settings {
    field: [f32; 4],
    grid: [u32; 4],
    shape: [f32; 4],
    range: [f32; 4],
    previous: [f32; 4],
    lods: [[u32; 4]; 3],
    dispatch: [i32; 4],
    distances: [f32; 4],
    support: [f32; 4],
    edits: [f32; 4], // RGB multiplier, retained world-cell fraction
    animation: [f32; 4], // bounded gust/direction scroll, current then previous
    wind_history: [f32; 4], // prior direction/amplitude; 0 classic, 1 mixed, 2 compact windswept
    previous_camera: [f32; 4], // previous rendered camera XYZ, history valid
}
impl Settings {
    fn with_live_options(mut self, options: crate::debug_settings::DebugRenderOptions) -> Self {
        self.shape[0] *= options.meadow_size_scale;
        self.distances[2] = options.meadow_lod_pixels;
        self.edits = [options.meadow_color_multiplier[0], options.meadow_color_multiplier[1],
            options.meadow_color_multiplier[2], options.meadow_density];
        self
    }
    fn for_frame(self, options: crate::debug_settings::DebugRenderOptions, clock: &Clock) -> Result<Self, String> {
        options.validate()?;
        let ranges = crate::debug_settings::meadow_ranges([self.shape[3], self.range[0]], options.meadow_distance_scale);
        let mut settings = self.with_live_options(options);
        settings.shape[3] = ranges[0];
        settings.range[0] = ranges[1];
        let phase = |time: f64| (time.rem_euclid(120.0) * std::f64::consts::TAU / 120.0) as f32;
        settings.range[1] = clock.current.strength;
        settings.range[2] = clock.current.grass_shear / 0.4;
        settings.range[3] = phase(clock.current.time_seconds);
        settings.previous = [
            phase(if clock.valid { clock.previous.time_seconds } else { clock.current.time_seconds }),
            clock.current.direction.x,
            clock.current.direction.y,
            if clock.valid { 1.0 } else { 0.0 },
        ];
        // Scrolling value noise repeats its integer lattice every 4096 cells.
        // Wrap each scroll independently in f64: unlike the old 120s phase,
        // these wraps are continuous and retain sub-frame time precision.
        let prior = if clock.valid { clock.previous } else { clock.current };
        let scroll = |time: f64, speed: f64| (time * speed).rem_euclid(4096.0) as f32;
        settings.animation = [scroll(clock.current.time_seconds, 1.0), scroll(clock.current.time_seconds, 0.05),
            scroll(prior.time_seconds, 1.0), scroll(prior.time_seconds, 0.05)];
        settings.wind_history = [prior.direction.x, prior.direction.y, prior.strength * prior.grass_shear / 0.4, self.wind_history[3]];
        Ok(settings)
    }
}
#[derive(Clone, Debug)]
pub struct MeadowStats {
    pub view: usize,
    pub plants: [u32; 3],
    pub size_scale: f32,
    /// Requested projected error threshold; the windswept profile instead
    /// uses its authored, growth-scaled source morph distances.
    pub lod_pixels: f32,
    pub density: f32,
    pub color_multiplier: [f32; 3],
    pub triangles: [u64; 3],
    pub capacity_per_view: u32,
    pub pool_bytes: u64,
    pub indexed: bool,
    /// Shared index-buffer allocation, not multiplied by views or plants.
    pub index_bytes: u64,
    pub source_vertices_per_patch: [u32; 3],
    pub depth_unique_vertices_per_patch: [u32; 3],
    pub color_unique_vertices_per_patch: [u32; 3],
    /// Ideal distinct vertex references. Hardware cache misses can execute a
    /// vertex shader more often; these are not measured invocation counters.
    pub depth_vertex_reference_budget: [u64; 3],
    pub color_vertex_reference_budget: [u64; 3],
    /// Opt-in single geometry submission followed by visible-source resolve.
    pub visibility_enabled: bool,
    pub visibility_target_bytes: u64,
    pub source_geometry_passes: u32,
    /// Submitted triangles per LOD, including every source geometry pass.
    pub submitted_source_triangles: [u64; 3],
    /// Explicit diagnostic readback only, not asynchronous invocation counts.
    pub visibility_covered_pixels: Option<u64>,
    pub visibility_unique_primitives: Option<u64>,
    pub visibility_unique_roots: Option<u64>,
    pub visibility_invalid_ids: u64,
    pub skip_hidden_shade_enabled: bool,
    /// Explicit request; the shading filter only runs for native source-only
    /// authored blades, with neither temporal reconstruction nor motion output.
    pub spatial_filter_requested: bool,
    /// The last encoded composite used the filter; individual near/empty
    /// pixels may remain unchanged. This is not a changed-pixel counter.
    pub spatial_filter_enabled: bool,
    pub spatial_filter_max_strength: f32,
    /// Mean cooked basal diameter times the current live growth multiplier.
    /// Filter onset uses half this value: the coarse triangle's mean width.
    pub spatial_filter_blade_width_m: f32,
    /// Actual opaque shade early returns in this view, not eligible source area.
    pub opaque_shade_skipped_pixels: u64,
    /// Padded ID copy plus its counter/header; per-view allocation.
    pub hidden_shade_buffer_bytes: u64,
    pub aggregate_enabled: bool,
    /// Actual constructed aggregate pipeline specialization, not a fresh env read.
    pub aggregate_sample_prune_enabled: bool,
    /// Opt-in spatial reconstruction approximation; source and merge remain full size.
    pub aggregate_half_resolution: bool,
    pub aggregate_target_size: [u32; 2],
    /// Actual far/source dimension ratios, including ceil rounding on odd inputs.
    pub aggregate_resolution_scale: [f32; 2],
    pub aggregate_plants: u32,
    pub aggregate_triangles: u64,
    pub aggregate_capacity: u32,
    pub aggregate_pool_bytes: u64,
    pub aggregate_recipe_bytes: u64,
    pub aggregate_cook_ms: f64,
    pub aggregate_transition_m: [f32; 2],
    pub aggregate_outer_fade_m: [f32; 2],
    /// Additional far/merge targets, including currently unused merge depth;
    /// excludes the original source targets and bounded root pool.
    pub aggregate_target_bytes: u64,
    /// Experimental shared world-page work. Per-frame cache counts are shared
    /// across views; active_pages/page_triangles are specific to this view.
    pub group_cache: GroupCacheStats,
}
struct IndexBuffers {
    depth: [wgpu::Buffer; 3],
    color: [wgpu::Buffer; 3],
}
// The reference draw reads the first four words (count, instances, 0, 0).
// Indexed draws read all five (count, instances, first_index, base_vertex, 0).
// Both paths preserve the original triangle count and ordering.
const DRAW_WORDS: usize = 5;
const DRAW_STRIDE: u64 = DRAW_WORDS as u64 * 4;
const ARGS_BYTES: u64 = DRAW_STRIDE * 3;
// Only the serialized raster pool is compacted. MeadowPlant and the brick
// producer retain their full branch-bearing ABI. Mixed recipes need branches.
fn meadow_pool_profile(geometry: &crate::meadow_data::MeadowGeometry) -> f32 {
    if geometry.vertices.iter().all(|vertex| vertex.anchor[3] > 0.0) { 2.0 }
    else if geometry.vertices.iter().any(|vertex| vertex.anchor[3] > 0.0) { 1.0 }
    else { 0.0 }
}
fn meadow_pool_plant_words(profile: f32) -> u32 { if profile > 1.5 { 12 } else { 28 } }
fn meadow_pool_bytes(capacity: u32, profile: f32) -> u64 {
    u64::from(capacity) * u64::from(meadow_pool_plant_words(profile) + 3) * 4
}
fn initial_draw_args(vertices: [u32; 3]) -> [u32; DRAW_WORDS * 3] {
    std::array::from_fn(|i| {
        if i % DRAW_WORDS == 0 {
            vertices[i / DRAW_WORDS]
        } else {
            0
        }
    })
}
struct Targets {
    size: (u32, u32),
    color_texture: wgpu::Texture,
    color: wgpu::TextureView,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    motion: Option<wgpu::TextureView>,
    motion_texture: Option<wgpu::Texture>,
    capture_ready: bool,
    motion_written: bool,
    reveal: Option<wgpu::TextureView>,
    material: Option<MaterialTargets>,
    reduced_opaque: Option<half::OpaqueTarget>,
    composite: wgpu::BindGroup,
}
struct MaterialTargets {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    layers: [wgpu::TextureView; 3],
}
struct AtlasTextures {
    material: wgpu::TextureView,
    surface: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bytes: u64,
}
struct ViewResources {
    settings: wgpu::Buffer,
    pool: wgpu::Buffer,
    args: wgpu::Buffer,
    generation: wgpu::BindGroup,
    draws: [wgpu::BindGroup; 3],
    targets: Option<Targets>,
    visibility: Option<visibility::Target>,
    prepared_visibility: Option<((u32,u32), Settings)>,
    hidden_used_this_frame: bool,
    spatial_filter_enabled: bool,
    pyramid: Option<crate::depth_pyramid::PyramidTarget>,
    opaque_view: TextureViews<1>,
    placement_bind: BindGroups<1>,
    lighting_bind: BindGroups<1>,
    resolve_bind: BindGroups<1>,
    merge_bind: BindGroups<1>,
    resolve_draw: Option<wgpu::Buffer>,
    aggregate: Option<Box<ViewResources>>,
    resolved: Option<Targets>,
    /// Last encoded source transition and far fade, for truthful diagnostics.
    ranges: [f32; 4],
    edits: [f32; 4],
    size_scale: f32,
    lod_pixels: f32,
}
fn consume_prepared_visibility(prepared:&mut Option<((u32,u32),Settings)>,size:(u32,u32),settings:&Settings)->Result<(),String> {
    let previous=prepared.take().ok_or("hidden meadow visibility was not prepared for this frame")?;
    if previous.0!=size || bytemuck::bytes_of(&previous.1)!=bytemuck::bytes_of(settings) {
        return Err("hidden meadow source changed between visibility and illumination".into());
    }
    Ok(())
}
struct AggregateResources {
    half_depth: Option<half::DepthReduce>,
    sample_prune_enabled: bool,
    dimension: u32,
    capacity: u32,
    pool_bytes: u64,
    recipe_bytes: u64,
    cook_ms: f64,
    atlas_base: u32,
    depth: [wgpu::RenderPipeline; 2],
    pipeline: wgpu::RenderPipeline,
    merge_layout: wgpu::BindGroupLayout,
    merge: [wgpu::RenderPipeline; 2],
    transition: wgpu::Buffer,
    atlas: AtlasTextures,
    draw_layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    resolve: wgpu::RenderPipeline,
}
struct Clock {
    current: WindState,
    previous: WindState,
    valid: bool,
}
#[derive(Default)]
struct CameraHistory {
    pending: Vec<glam::Vec3>,
    presented: Vec<glam::Vec3>,
}
impl CameraHistory {
    fn prepare(&mut self, positions: impl IntoIterator<Item = glam::Vec3>) {
        self.pending = positions.into_iter().collect();
        if self.pending.len() != self.presented.len() { self.presented.clear(); }
    }
    fn previous(&self, index: usize) -> [f32; 4] {
        self.presented.get(index).map_or([0.0; 4], |p| [p.x, p.y, p.z, 1.0])
    }
    fn finish(&mut self) { self.presented.clone_from(&self.pending); }
}
// The brick producer calls the shared camera-independent root function. Its
// borrowed generation layout still requires pool/args/Hi-Z bindings, although
// it never reads/writes them. Avoid allocating a full source/far view pool.
struct BrickSourceBindings {
    settings: wgpu::Buffer,
    generation: wgpu::BindGroup,
    empty_hiz: wgpu::TextureView,
}
pub struct MeadowRenderer {
    depth_pyramid: crate::depth_pyramid::DepthPyramid,
    settings: Settings,
    capacity: u32,
    dimension: u32,
    vertices: [u32; 3],
    indices: Option<IndexBuffers>,
    index_bytes: u64,
    depth_unique_vertices: [u32; 3],
    color_unique_vertices: [u32; 3],
    pool_bytes: u64,
    ground: wgpu::Buffer,
    recipe: wgpu::Buffer,
    edge_shadow: Option<contact::Renderer>,
    generate: wgpu::ComputePipeline,
    generate_layout: wgpu::BindGroupLayout,
    generate_view_layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    view_layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::RenderPipeline,
    pipelines: [wgpu::RenderPipeline; 2],
    visibility: Option<visibility::Renderer>,
    skip_hidden_shade: bool,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    spatial_filter_requested: bool,
    spatial_filter: Option<spatial_filter::Filter>,
    spatial_filter_temporal_active: std::sync::atomic::AtomicBool,
    views: Mutex<Vec<Option<ViewResources>>>,
    clock: Mutex<Clock>,
    cameras: Mutex<CameraHistory>,
    brick_source: Mutex<Option<BrickSourceBindings>>,
    aggregate: Option<AggregateResources>,
    groups: Option<group_gpu::GpuGroups>,
}
fn buffer(gpu: &Gpu, label: &str, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage,
        })
}
fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}
fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}
fn texture(
    sample_type: wgpu::TextureSampleType,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type,
        view_dimension,
        multisampled: false,
    }
}
fn layout(
    gpu: &Gpu,
    label: &str,
    types: &[(u32, wgpu::BindingType)],
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayout {
    gpu.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &types
                .iter()
                .map(|(binding, ty)| wgpu::BindGroupLayoutEntry {
                    binding: *binding,
                    visibility,
                    ty: *ty,
                    count: None,
                })
                .collect::<Vec<_>>(),
        })
}
fn shader(gpu: &Gpu, label: &str, source: String) -> wgpu::ShaderModule {
    gpu.device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
}
pub fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        crate::wind::shade_source(),
        include_str!("shaders/meadow_common.wgsl"),
        include_str!("shaders/meadow_transition.wgsl"),
        include_str!("shaders/meadow_render.wgsl"),
        include_str!("shaders/meadow_aggregate.wgsl")
    )
}
pub fn generate_source() -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("shaders/meadow_field.wgsl"),
        include_str!("shaders/meadow_common.wgsl"),
        include_str!("shaders/meadow_generate.wgsl")
    )
}
pub fn aggregate_resolve_source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        crate::wind::shade_source(),
        include_str!("shaders/meadow_common.wgsl"),
        include_str!("shaders/meadow_transition.wgsl"),
        include_str!("shaders/meadow_render.wgsl"),
        include_str!("shaders/meadow_aggregate_resolve.wgsl")
    )
}
fn upload_atlas(
    gpu: &Gpu,
    data: &crate::meadow_aggregate::AggregateTextureData,
) -> Result<AtlasTextures, String> {
    if data.layers > gpu.limits.max_texture_array_layers {
        return Err("meadow atlas exceeds texture-array layer limit".into());
    }
    let make = |material: bool| {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(if material {
                "meadow atlas filtered material"
            } else {
                "meadow atlas source surface"
            }),
            size: wgpu::Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: data.layers,
            },
            mip_level_count: data.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, mip) in data.mips.iter().enumerate() {
            let bytes = if material {
                &mip.material
            } else {
                &mip.surface
            };
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(bytes),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip.width * 8),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: data.layers,
                },
            );
        }
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        })
    };
    Ok(AtlasTextures {
        material: make(true),
        surface: make(false),
        sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("meadow atlas bilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        }),
        bytes: data
            .mips
            .iter()
            .map(|m| (m.material.len() + m.surface.len()) as u64 * 2)
            .sum(),
    })
}
const COMPOSITE: &str = "@group(0) @binding(0) var layer:texture_2d<f32>; @vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32>{let p=array<vec2<f32>,3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));return vec4<f32>(p[i],0.,1.);} @fragment fn fs(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{return textureLoad(layer,vec2<i32>(p.xy),0);}";
impl MeadowRenderer {
    pub fn new(
        gpu: &Gpu,
        scene: &Scene,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> Result<Option<Self>, String> {
        let Some(field) = &scene.meadow else {
            return Ok(None);
        };
        field.validate()?;
        let pool_profile = meadow_pool_profile(&field.geometry);
        let windswept = pool_profile > 0.5;
        // Tall nonlinear ribbons use real geometry at distance, as in the
        // reference. A patch-wide rest atlas turns their conservative sweep
        // into enormous overlapping fragments and cannot preserve coverage.
        let aggregate_enabled = !windswept && std::env::var("KERN_MEADOW_AGGREGATE").as_deref() == Ok("1");
        let aggregate_half = aggregate_enabled && std::env::var("KERN_MEADOW_AGGREGATE_HALF_RES").as_deref() == Ok("1");
        let aggregate_sample_prune = aggregate_enabled && std::env::var("KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE").as_deref() == Ok("1");
        let groups_enabled = std::env::var("KERN_MEADOW_GROUP_CACHE").as_deref() == Ok("1");
        if groups_enabled && windswept {
            return Err("windswept blades require KERN_MEADOW_GROUP_CACHE=0: world-page pose certificates support rigid branch wind only".into());
        }
        if aggregate_half && groups_enabled {
            return Err("KERN_MEADOW_AGGREGATE_HALF_RES does not support KERN_MEADOW_GROUP_CACHE".into());
        }
        if aggregate_sample_prune && groups_enabled {
            return Err("KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE does not support KERN_MEADOW_GROUP_CACHE".into());
        }
        if std::env::var("KERN_MEADOW_GROUP_REPLACE").as_deref() == Ok("1") && !groups_enabled {
            return Err("KERN_MEADOW_GROUP_REPLACE requires KERN_MEADOW_GROUP_CACHE=1".into());
        }
        if groups_enabled && !aggregate_enabled {
            return Err("KERN_MEADOW_GROUP_CACHE requires KERN_MEADOW_AGGREGATE=1".into());
        }
        let index_streams = crate::meadow_index::MeadowIndices::build(&field.geometry)?;
        let indexed = std::env::var("KERN_MEADOW_INDEXED").as_deref() != Ok("0");
        if field.width < 2
            || field.height < 2
            || field.samples.len() != field.width as usize * field.height as usize
            || !field.cell.is_finite()
            || field.cell <= 0.0
            || !field.radius.is_finite()
            || !(10.0..=256.0).contains(&field.radius)
            || field.fade_start >= field.radius
        {
            return Err("invalid meadow field dimensions/range".into());
        }
        let dimension = ((field.radius / crate::meadow_data::ROOT_CELL).ceil() as u32 + 3) * 2 + 1;
        let capacity = dimension
            .checked_mul(dimension)
            .ok_or("meadow capacity overflow")?;
        let pool_bytes = meadow_pool_bytes(capacity, pool_profile);
        let mut packed = vec![[0.0f32; 4]; 21];
        let mut vertices = [0u32; 3];
        for variant in 0..3 {
            for lod in 0..3 {
                let [offset, count] = field.geometry.lod_ranges[variant][lod];
                if count == 0
                    || count % 3 != 0
                    || (offset as usize + count as usize) > field.geometry.vertices.len()
                {
                    return Err("invalid meadow recipe draw range".into());
                }
                if variant == 0 {
                    vertices[lod] = count;
                } else if vertices[lod] != count {
                    return Err("meadow variants require equal vertex counts per LOD".into());
                }
                packed[variant * 3 + lod] =
                    [f32::from_bits(offset), f32::from_bits(count), 0.0, 0.0];
            }
            for branch in 0..4 {
                packed[9 + variant * 4 + branch] = field.geometry.branch_pivots[variant][branch];
            }
        }
        for vertex in &field.geometry.vertices {
            packed.push(vertex.position);
            packed.push(vertex.normal);
            packed.push(vertex.color);
            packed.push(vertex.anchor);
        }
        // Header X/Y remain bitcast draw ranges. Z is an ordinary f32 world
        // patch scale; zero retains the original independent variant hash.
        packed[0][2] = field.geometry.variant_patch_metres;
        let atlas_base = packed.len() as u32;
        let atlas = if aggregate_enabled {
            let atlas = crate::meadow_aggregate::AggregateAtlas::cook(&field.geometry)?;
            let textures = atlas.texture_data()?;
            packed.extend_from_slice(&textures.headers);
            // Exact anchor and blade-relative bounds, preserving growth about
            // each fixed blade base. The shader adds the certified wind margin.
            for variant in 0..3 {
                let [first, count] = field.geometry.lod_ranges[variant][0];
                let mut lo_anchor = glam::Vec3::splat(f32::INFINITY);
                let mut hi_anchor = -lo_anchor;
                let mut lo_relative = lo_anchor;
                let mut hi_relative = hi_anchor;
                for vertex in &field.geometry.vertices[first as usize..(first + count) as usize] {
                    let anchor = glam::Vec3::from_array(vertex.anchor[..3].try_into().unwrap());
                    let relative =
                        glam::Vec3::from_array(vertex.position[..3].try_into().unwrap()) - anchor;
                    lo_anchor = lo_anchor.min(anchor);
                    hi_anchor = hi_anchor.max(anchor);
                    lo_relative = lo_relative.min(relative);
                    hi_relative = hi_relative.max(relative);
                }
                for p in [lo_anchor, hi_anchor, lo_relative, hi_relative] {
                    packed.push(p.extend(0.0).to_array());
                }
            }
            Some((upload_atlas(gpu, &textures)?, atlas.cook_ms))
        } else {
            None
        };
        let aggregate_dimension =
            ((180.0 / crate::meadow_data::ROOT_CELL).ceil() as u32 + 3) * 2 + 1;
        let aggregate_capacity = aggregate_dimension * aggregate_dimension;
        let aggregate_pool_bytes = meadow_pool_bytes(aggregate_capacity, 0.0);
        for bytes in [
            pool_bytes,
            (field.samples.len() * 16) as u64,
            (packed.len() * 16) as u64,
            if aggregate_enabled {
                aggregate_pool_bytes
            } else {
                0
            },
        ] {
            if bytes > u64::from(gpu.limits.max_storage_buffer_binding_size)
                || bytes > gpu.limits.max_buffer_size
            {
                return Err("meadow buffers exceed device storage limit".into());
            }
        }
        let ground = buffer(
            gpu,
            "meadow terrain",
            bytemuck::cast_slice(&field.samples),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let recipe = buffer(
            gpu,
            "meadow shared recipes",
            bytemuck::cast_slice(&packed),
            wgpu::BufferUsages::STORAGE,
        );
        let indices = indexed.then(|| IndexBuffers {
            depth: std::array::from_fn(|lod| {
                buffer(
                    gpu,
                    "meadow depth indices",
                    bytemuck::cast_slice(&index_streams.depth[lod]),
                    wgpu::BufferUsages::INDEX,
                )
            }),
            color: std::array::from_fn(|lod| {
                buffer(
                    gpu,
                    "meadow color indices",
                    bytemuck::cast_slice(&index_streams.color[lod]),
                    wgpu::BufferUsages::INDEX,
                )
            }),
        });
        let index_bytes = indices.as_ref().map_or(0, |i| {
            i.depth.iter().chain(&i.color).map(wgpu::Buffer::size).sum()
        });
        let settings = Settings {
            field: [
                field.origin[0],
                field.origin[1],
                field.cell,
                crate::meadow_data::ROOT_CELL,
            ],
            grid: [field.width, field.height, field.seed, 3],
            shape: [
                field.growth,
                field.geometry.max_tuft_radius,
                field.geometry.max_tuft_height,
                field.fade_start,
            ],
            range: [field.radius, 0.0, 1.0, 0.0],
            previous: [0.0, 1.0, 0.0, 0.0],
            lods: std::array::from_fn(|i| {
                [
                    0,
                    vertices[i],
                    field
                        .geometry
                        .lod_errors
                        .iter()
                        .map(|e| e[i])
                        .fold(0.0f32, f32::max)
                        .to_bits(),
                    0,
                ]
            }),
            dispatch: [0, 0, dimension as i32, capacity as i32],
            distances: [
                20.0,
                50.0,
                1.0,
                if std::env::var("KERN_MEADOW_OCCLUSION").as_deref() == Ok("0") {
                    0.0
                } else {
                    1.0
                },
            ],
            support: [
                field.geometry.max_anchor_radius,
                field.geometry.max_blade_radius,
                if aggregate_enabled { 1.0 } else { 0.0 },
                f32::from_bits(atlas_base),
            ],
            edits: [1.0; 4],
            animation: [0.0; 4],
            wind_history: [1.0, 0.0, 0.0, pool_profile],
            previous_camera: [0.0; 4],
        };
        let generate_layout = layout(
            gpu,
            "meadow generate",
            &[
                (0, uniform()),
                (1, storage(true)),
                (2, storage(false)),
                (3, storage(false)),
                (4, storage(true)),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let generate_view_layout = layout(
            gpu,
            "meadow generate view",
            &[
                (0, uniform()),
                (
                    1,
                    texture(
                        wgpu::TextureSampleType::Float { filterable: false },
                        wgpu::TextureViewDimension::D2,
                    ),
                ),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let gl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow generate"),
                bind_group_layouts: &[Some(&generate_layout), Some(&generate_view_layout)],
                immediate_size: 0,
            });
        let gs = shader(gpu, "meadow generate", generate_source());
        let generate = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meadow generate"),
                layout: Some(&gl),
                module: &gs,
                entry_point: Some("generate_meadow"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        let d2 = wgpu::TextureViewDimension::D2;
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let view_layout = layout(
            gpu,
            "meadow lighting",
            &[
                (0, uniform()),
                (2, uniform()),
                (5, texture(wgpu::TextureSampleType::Depth, d2)),
                (
                    6,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                ),
                (7, texture(unfiltered, d2)),
                (10, texture(unfiltered, d2)),
                (11, uniform()),
                (
                    13,
                    texture(
                        wgpu::TextureSampleType::Float { filterable: true },
                        wgpu::TextureViewDimension::D3,
                    ),
                ),
                (
                    14,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                (15, texture(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array)),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let draw_layout = layout(
            gpu,
            "meadow draw",
            &[
                (0, uniform()),
                (1, storage(true)),
                (2, storage(true)),
                (3, uniform()),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let skip_hidden_shade=std::env::var("KERN_MEADOW_SKIP_HIDDEN_SHADE").as_deref()==Ok("1");
        if skip_hidden_shade && std::env::var("KERN_MEADOW_VISIBILITY").as_deref()!=Ok("1") {
            return Err("KERN_MEADOW_SKIP_HIDDEN_SHADE requires KERN_MEADOW_VISIBILITY=1".into());
        }
        let visibility = if std::env::var("KERN_MEADOW_VISIBILITY").as_deref() == Ok("1") {
            Some(visibility::Renderer::new(gpu, scene_layout, &view_layout, &draw_layout)?)
        } else { None };
        let aggregate_draw_layout = layout(
            gpu,
            "meadow aggregate draw",
            &[
                (0, uniform()),
                (1, storage(true)),
                (2, storage(true)),
                (3, uniform()),
                (
                    4,
                    texture(
                        wgpu::TextureSampleType::Float { filterable: true },
                        wgpu::TextureViewDimension::D2Array,
                    ),
                ),
                (5, texture(unfiltered, wgpu::TextureViewDimension::D2Array)),
                (
                    6,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let dl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow source draw"),
                bind_group_layouts: &[Some(scene_layout), Some(&view_layout), Some(&draw_layout)],
                immediate_size: 0,
            });
        let ds = shader(gpu, "meadow source draw", shader_source());
        let aggregate_dl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow aggregate draw"),
                bind_group_layouts: &[
                    Some(scene_layout),
                    Some(&view_layout),
                    Some(&aggregate_draw_layout),
                ],
                immediate_size: 0,
            });
        let pipeline = |motion: bool, depth: bool, aggregate: bool| {
            let constants = [
                ("MEADOW_AGGREGATE_HALF_RES", if aggregate && aggregate_half {1.0} else {0.0}),
                ("MEADOW_AGGREGATE_SAMPLE_PRUNE", if aggregate && aggregate_sample_prune {1.0} else {0.0}),
            ];
            let mut targets = if depth {
                vec![]
            } else {
                vec![Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })]
            };
            if motion {
                targets.push(Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba32Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }));
            }
            if aggregate && !depth {
                let additive = wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                };
                let reveal = wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                };
                targets = vec![
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState {
                            color: additive,
                            alpha: additive,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState {
                            color: additive,
                            alpha: additive,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState {
                            color: reveal,
                            alpha: reveal,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ];
            }
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("meadow source pipeline"),
                    layout: Some(if aggregate { &aggregate_dl } else { &dl }),
                    vertex: wgpu::VertexState {
                        module: &ds,
                        entry_point: Some(if aggregate {
                            "meadow_aggregate_vs"
                        } else {
                            "meadow_vs"
                        }),
                        compilation_options: wgpu::PipelineCompilationOptions {constants:&constants,..Default::default()},
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(depth),
                        depth_compare: Some(if depth || aggregate {
                            wgpu::CompareFunction::GreaterEqual
                        } else {
                            wgpu::CompareFunction::Equal
                        }),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &ds,
                        entry_point: Some(if aggregate && depth && motion {
                            "meadow_aggregate_depth_motion_fs"
                        } else if aggregate && depth {
                            "meadow_aggregate_depth_fs"
                        } else if aggregate {
                            "meadow_aggregate_fs"
                        } else if depth {
                            "meadow_depth_fs"
                        } else if motion {
                            "meadow_motion_fs"
                        } else {
                            "meadow_fs"
                        }),
                        compilation_options: wgpu::PipelineCompilationOptions {constants:&constants,..Default::default()},
                        targets: &targets,
                    }),
                    multiview_mask: None,
                    cache: gpu.pipeline_cache(),
                })
        };
        let depth_pipeline = pipeline(false, true, false);
        let pipelines = [pipeline(false, false, false), pipeline(true, false, false)];
        let aggregate = atlas.map(|(atlas,cook_ms)| {
            let resolve_layout=layout(gpu,"meadow aggregate illumination",&[
                (0,uniform()),(1,storage(true)),(2,storage(true)),(3,uniform()),
                (4,texture(unfiltered,wgpu::TextureViewDimension::D2Array)),
                (5,texture(wgpu::TextureSampleType::Depth,d2)),
            ],wgpu::ShaderStages::VERTEX_FRAGMENT);
            let rpl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:Some("meadow aggregate illumination"),bind_group_layouts:&[Some(scene_layout),Some(&view_layout),Some(&resolve_layout)],immediate_size:0,
            });
            let rs=shader(gpu,"meadow aggregate illumination",aggregate_resolve_source());
            let resolve=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:Some("meadow aggregate illumination"),layout:Some(&rpl),
                vertex:wgpu::VertexState {module:&rs,entry_point:Some("meadow_resolve_vs"),compilation_options:Default::default(),buffers:&[]},
                primitive:Default::default(),depth_stencil:None,multisample:Default::default(),
                fragment:Some(wgpu::FragmentState {module:&rs,entry_point:Some("meadow_resolve_fs"),compilation_options:Default::default(),targets:&[
                    Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),
                multiview_mask:None,cache:gpu.pipeline_cache(),
            });
            let merge_layout = layout(gpu, "meadow representation merge", &[
                (0,texture(unfiltered,d2)),(1,texture(unfiltered,d2)),
                (2,texture(wgpu::TextureSampleType::Depth,d2)),(3,texture(wgpu::TextureSampleType::Depth,d2)),
                (4,texture(unfiltered,d2)),(5,texture(unfiltered,d2)),(6,uniform()),(7,uniform()),(8,texture(wgpu::TextureSampleType::Depth,d2))
            ],wgpu::ShaderStages::FRAGMENT);
            let ml = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:Some("meadow representation merge"),bind_group_layouts:&[Some(&merge_layout)],immediate_size:0,
            });
            let ms = shader(gpu,"meadow representation merge",include_str!("shaders/meadow_merge.wgsl").into());
            let merge = std::array::from_fn(|motion| {
                let constants=[("MEADOW_AGGREGATE_HALF_RES",if aggregate_half {1.0} else {0.0})];
                let mut targets=vec![Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,blend:None,write_mask:wgpu::ColorWrites::ALL})];
                if motion!=0 {targets.push(Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba32Float,blend:None,write_mask:wgpu::ColorWrites::ALL}));}
                gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label:Some("meadow representation merge"),layout:Some(&ml),
                    vertex:wgpu::VertexState {module:&ms,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},
                    primitive:Default::default(),depth_stencil:None,multisample:Default::default(),
                    fragment:Some(wgpu::FragmentState {module:&ms,entry_point:Some(if motion==0 {"fs"} else {"motion_fs"}),compilation_options:wgpu::PipelineCompilationOptions {constants:&constants,..Default::default()},targets:&targets}),
                    multiview_mask:None,cache:gpu.pipeline_cache(),
                })
            });
            let transition=buffer(gpu,"meadow transition",bytemuck::cast_slice(&[field.fade_start,field.radius,160.0,180.0]),wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
            let recipe_bytes=atlas.bytes+(216+12)*16;
            eprintln!("GPU meadow surface aggregate: {}–180 m, {} candidates/view, {} pool bytes/view, {} atlas+bounds bytes, {:.2} ms CPU cook; hardware filtered source material, one lighting resolve per pixel, NOT voxel transport",field.fade_start,aggregate_capacity,aggregate_pool_bytes,recipe_bytes,cook_ms);
            eprintln!("GPU meadow aggregate half resolution: {aggregate_half}; reduced depth/material/lighting, full source/merge; spatial reconstruction approximation");
            eprintln!("GPU meadow aggregate sample pruning: {aggregate_sample_prune}; exact-zero angular reads skipped, representative point/anchor fetched once; unchanged resolution/coverage/pose");
            AggregateResources {half_depth:aggregate_half.then(||half::DepthReduce::new(gpu)),sample_prune_enabled:aggregate_sample_prune,dimension:aggregate_dimension,capacity:aggregate_capacity,pool_bytes:aggregate_pool_bytes,
                recipe_bytes,cook_ms,atlas_base,
                depth:[pipeline(false,true,true),pipeline(true,true,true)],pipeline:pipeline(false,false,true),
                merge_layout,merge,transition,atlas,draw_layout:aggregate_draw_layout,resolve_layout,resolve}
        });
        let composite_layout = layout(
            gpu,
            "meadow composite",
            &[(0, texture(unfiltered, d2))],
            wgpu::ShaderStages::FRAGMENT,
        );
        let cl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow composite"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let cs = shader(gpu, "meadow composite", COMPOSITE.into());
        let composite = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("meadow composite"),
                layout: Some(&cl),
                vertex: wgpu::VertexState {
                    module: &cs,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &cs,
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
        eprintln!(
            "GPU meadow: {} m radius, {} candidates/view, {} MiB pool/view, source triangles {:?}; immutable terrain/recipes, GPU roots/LOD/branch wind",
            field.radius,
            capacity,
            pool_bytes / 1048576,
            vertices.map(|n| n / 3)
        );
        eprintln!(
            "GPU meadow: indexed={indexed}, shared index allocation={index_bytes} bytes (payload {}), flat vertices {:?}, common unique depth {:?}, color {:?}; unique references are ideal budgets, not GPU invocation counts",
            if indexed { index_streams.bytes() } else { 0 },
            vertices, index_streams.depth_unique, index_streams.color_unique
        );
        let groups=if groups_enabled {
            eprintln!("GPU meadow world pages: experimental, shared 256 × 64² pages, four cooks/frame; uncertain depth/pose/direction retains per-root atlas");
            Some(group_gpu::GpuGroups::new(gpu,field,&generate_layout,&generate_view_layout)?)
        } else {None};
        let edge_shadow = contact::Renderer::new(gpu, field, &ground, &recipe,
            &generate_layout, &generate_view_layout);
        let spatial_filter_requested = std::env::var("KERN_MEADOW_SPATIAL_FILTER").as_deref() == Ok("1");
        let spatial_filter = if spatial_filter_requested && aggregate.is_none() {
            spatial_filter::source_dimensions(&field.geometry).map(|dimensions| spatial_filter::Filter::new(gpu,dimensions))
        } else {None};
        if spatial_filter_requested {
            eprintln!("GPU meadow spatial filter: compatible={}, native-only 3x3 source shading, strength <= {}; alpha/geometry unchanged, no temporal history",spatial_filter.is_some(),spatial_filter::MAX_STRENGTH);
        }
        Ok(Some(Self {
            depth_pyramid: crate::depth_pyramid::DepthPyramid::new(gpu),
            settings,
            capacity,
            dimension,
            vertices,
            indices,
            index_bytes,
            depth_unique_vertices: index_streams.depth_unique,
            color_unique_vertices: index_streams.color_unique,
            pool_bytes,
            ground,
            recipe,
            edge_shadow,
            generate,
            generate_layout,
            generate_view_layout,
            draw_layout,
            view_layout,
            depth_pipeline,
            pipelines,
            visibility,
            skip_hidden_shade,
            composite,
            composite_layout,
            spatial_filter_requested,
            spatial_filter,
            spatial_filter_temporal_active: std::sync::atomic::AtomicBool::new(false),
            views: Mutex::new(Vec::new()),
            clock: Mutex::new(Clock {
                current: WindState::default(),
                previous: WindState::default(),
                valid: false,
            }),
            cameras: Mutex::new(CameraHistory::default()),
            brick_source: Mutex::new(None),
            aggregate,
            groups,
        }))
    }
    /// Replace only the sampled ground under a retained meadow. Geometry,
    /// pipelines, view pools and world-space root identities are preserved.
    pub fn update_ground(&mut self, gpu: &Gpu, origin: [f32; 2], samples: &[[f32; 4]]) -> Result<(), String> {
        if origin.iter().any(|x| !x.is_finite()) || samples.len() != self.settings.grid[0] as usize * self.settings.grid[1] as usize
            || samples.iter().flatten().any(|x| !x.is_finite()) {
            return Err("invalid retained meadow ground update".into());
        }
        gpu.queue.write_buffer(&self.ground, 0, bytemuck::cast_slice(samples));
        self.settings.field[0] = origin[0];
        self.settings.field[1] = origin[1];
        if let Some(groups) = &self.groups { groups.invalidate(); }
        Ok(())
    }

    pub fn set_wind(&self, state: &WindState) -> Result<bool, String> {
        let state = state.validated()?;
        let mut c = self.clock.lock().unwrap();
        let reset = c.valid
            && (state.direction != c.current.direction
                || state.strength != c.current.strength
                || state.grass_shear != c.current.grass_shear);
        c.current = state;
        if reset {
            c.valid = false;
        }
        Ok(reset)
    }
    pub fn aggregate_enabled(&self) -> bool {
        self.aggregate.is_some()
    }
    pub fn visibility_enabled(&self) -> bool { self.visibility.is_some() }
    pub fn resource_cache_stats(&self) -> crate::frame::ResourceCacheStats {
        let mut stats = crate::frame::ResourceCacheStats::default();
        let views = self.views.lock().unwrap();
        for view in views.iter().flatten() {
            for resources in std::iter::once(view).chain(view.aggregate.as_deref()) {
                for other in [resources.opaque_view.stats(), resources.placement_bind.stats(),
                    resources.lighting_bind.stats(), resources.resolve_bind.stats(), resources.merge_bind.stats()] {
                    stats.add(other);
                }
                if let Some(pyramid) = &resources.pyramid { stats.add(pyramid.resource_cache_stats()); }
                if let Some(opaque) = resources.targets.as_ref().and_then(|t| t.reduced_opaque.as_ref()) {
                    stats.add(opaque.resource_cache_stats());
                }
            }
        }
        stats
    }
    pub fn skip_hidden_shade_enabled(&self)->bool {self.skip_hidden_shade}
    /// The frame renderer updates this before each view. Source-only fixtures
    /// default to native; motion output also independently disables the filter.
    pub fn set_spatial_filter_temporal_active(&self, active: bool) {
        self.spatial_filter_temporal_active.store(active, std::sync::atomic::Ordering::Relaxed);
    }
    /// Experimental cached pages do not yet preserve previous-frame coverage
    /// through page/reference transitions. Do not feed them to reconstruction.
    pub fn group_cache_enabled(&self) -> bool { self.groups.is_some() }
    /// Prepare per-view morph history before either visibility or shading.
    /// Only finish_frame advances the last encoded camera set.
    pub fn set_views(&self, views:&[crate::ViewRect], frame_serial:u64) {
        self.cameras.lock().unwrap().prepare(views.iter().map(|v| v.cam.pos));
        if let Some(groups)=&self.groups {groups.set_views(views,frame_serial);}
    }
    /// Publish newly cooked pages only after their command buffer was submitted.
    pub fn group_cache_submitted(&self) {
        if let Some(groups)=&self.groups {groups.submitted();}
    }
    pub fn finish_frame(&self) {
        let mut c = self.clock.lock().unwrap();
        c.previous = c.current;
        c.valid = true;
        self.cameras.lock().unwrap().finish();
    }
    pub(crate) fn invalidate_history(&self) {
        self.clock.lock().unwrap().valid = false;
        self.cameras.lock().unwrap().presented.clear();
        if let Some(groups)=&self.groups {groups.invalidate();}
    }
    pub fn set_view_count(&self, count: usize) {
        let mut views=self.views.lock().unwrap();
        views.resize_with(count, || None);
        // A skipped/aborted previous frame can never certify a new frame.
        for view in views.iter_mut().flatten() {
            view.prepared_visibility=None;view.hidden_used_this_frame=false;
            view.spatial_filter_enabled=false;
            if let Some(target)=view.aggregate.as_mut().and_then(|far|far.targets.as_mut()) {
                target.capture_ready=false;target.motion_written=false;
            }
        }
    }
    fn frame_settings(&self, options: crate::debug_settings::DebugRenderOptions) -> Result<Settings, String> {
        self.settings.for_frame(options, &self.clock.lock().unwrap())
    }
    /// Build the bounded source-derived producer against the exact layouts,
    /// terrain and recipe used by this renderer. This does not alter or suppress
    /// ordinary source draws. The fixture must enclose its entire source habitat
    /// plus deformed support inside the plan when using it as a source oracle.
    pub fn new_brick_field(
        &self,
        gpu: &Gpu,
        samples: &crate::meadow_voxel_samples::MeadowVoxelSamples,
        plan: crate::meadow_brick_field::BrickPlan,
    ) -> Result<crate::meadow_brick_field::MeadowBrickField, String> {
        if plan.root_spacing != self.settings.field[3] {
            return Err("meadow brick root spacing must match the source renderer".into());
        }
        crate::meadow_brick_field::MeadowBrickField::new(
            gpu, samples, plan, &self.generate_layout, &self.generate_view_layout,
        )
    }
    /// Encode once per submission, shared by every consumer view. Call set_wind
    /// first and finish_frame only after all source/brick work is encoded. The
    /// camera uniform is borrowed unchanged; page-mode root production bypasses
    /// camera/range/Hi-Z selection, while preserving habitat, anchors and poses.
    /// Like the producer's own encode API, repeated calls before submission are
    /// unsupported because its frame parameters use queue writes.
    pub fn encode_brick_field(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        field: &crate::meadow_brick_field::MeadowBrickField,
        camera_uniform: &wgpu::Buffer,
        options: crate::debug_settings::DebugRenderOptions,
        generation: u32,
        times: [Option<(&wgpu::QuerySet, u32)>; 8],
    ) -> Result<(), String> {
        let settings = self.frame_settings(options)?;
        let support = settings.support[0] + settings.support[1] * settings.shape[0].max(0.0) * 1.3;
        if field.plan().root_spacing != settings.field[3] || support > field.plan().root_support_radius {
            return Err("meadow brick plan does not enclose the current source root support".into());
        }
        let mut source = self.brick_source.lock().unwrap();
        let source = source.get_or_insert_with(|| {
            let settings_buffer = buffer(gpu, "meadow brick shared source settings",
                bytemuck::bytes_of(&settings), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
            let unused_pool = buffer(gpu, "meadow brick unused root pool", &[0; 4], wgpu::BufferUsages::STORAGE);
            let unused_args = buffer(gpu, "meadow brick unused indirect arguments",
                &[0; ARGS_BYTES as usize], wgpu::BufferUsages::STORAGE);
            let generation = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("meadow brick shared source generation"), layout: &self.generate_layout,
                entries: &[(0, &settings_buffer), (1, &self.ground), (2, &unused_pool),
                    (3, &unused_args), (4, &self.recipe)].map(|(binding, b)| wgpu::BindGroupEntry {
                        binding, resource: b.as_entire_binding(),
                    }),
            });
            let empty_hiz = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("meadow brick unused zero Hi-Z"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }).create_view(&Default::default());
            BrickSourceBindings { settings: settings_buffer, generation, empty_hiz }
        });
        gpu.queue.write_buffer(&source.settings, 0, bytemuck::bytes_of(&settings));
        let view = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow brick shared source camera"), layout: &self.generate_view_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&source.empty_hiz) },
            ],
        });
        field.encode_profiled(gpu, encoder, &source.generation, &view, generation, times)
    }
    fn new_view(&self, gpu: &Gpu) -> ViewResources {
        self.new_view_kind(gpu, false)
    }
    fn new_view_kind(&self, gpu: &Gpu, aggregate: bool) -> ViewResources {
        let capacity = if aggregate {
            self.aggregate.as_ref().unwrap().capacity
        } else {
            self.capacity
        };
        let pool_bytes = if aggregate {
            self.aggregate.as_ref().unwrap().pool_bytes
        } else {
            self.pool_bytes
        };
        let settings = buffer(
            gpu,
            "meadow view settings",
            bytemuck::bytes_of(&self.settings),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let pool = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meadow world roots and compact owners"),
            size: pool_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let args = buffer(
            gpu,
            "meadow indirect draws",
            &[0; ARGS_BYTES as usize],
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        );
        let generation = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow generate"),
            layout: &self.generate_layout,
            entries: &[
                (0, &settings),
                (1, &self.ground),
                (2, &pool),
                (3, &args),
                (4, &self.recipe),
            ]
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            }),
        });
        let draws = std::array::from_fn(|lod| {
            let draw = buffer(
                gpu,
                "meadow LOD",
                bytemuck::cast_slice(&[lod as u32, capacity, u32::from(self.skip_hidden_shade && !aggregate), 0]),
                wgpu::BufferUsages::UNIFORM,
            );
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.recipe.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: draw.as_entire_binding(),
                },
            ];
            if aggregate {
                let a = self.aggregate.as_ref().unwrap();
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&a.atlas.material),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&a.atlas.surface),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::Sampler(&a.atlas.sampler),
                    },
                ]);
            }
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("meadow draw"),
                layout: if aggregate {
                    &self.aggregate.as_ref().unwrap().draw_layout
                } else {
                    &self.draw_layout
                },
                entries: &entries,
            })
        });
        ViewResources {
            settings,
            pool,
            args,
            generation,
            draws,
            targets: None,
            visibility: None,
            prepared_visibility: None,
            hidden_used_this_frame: false,
            spatial_filter_enabled: false,
            pyramid: None,
            opaque_view: Default::default(),
            placement_bind: Default::default(),
            lighting_bind: Default::default(),
            resolve_bind: Default::default(),
            merge_bind: Default::default(),
            resolve_draw: aggregate.then(|| buffer(gpu, "meadow lighting draw",
                bytemuck::cast_slice(&[0u32; 4]), wgpu::BufferUsages::UNIFORM)),
            aggregate: if !aggregate && self.aggregate.is_some() {
                Some(Box::new(self.new_view_kind(gpu, true)))
            } else {
                None
            },
            resolved: None,
            ranges: crate::debug_settings::meadow_ranges([self.settings.shape[3], self.settings.range[0]], 1.0),
            edits: [1.0; 4],
            size_scale: 1.0,
            lod_pixels: self.settings.distances[2],
        }
    }
    fn targets(&self, gpu: &Gpu, size: (u32, u32)) -> Targets {
        let make = |format, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("meadow layer"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color_texture = make(
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let color = color_texture.create_view(&Default::default());
        let depth = make(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_view = depth.create_view(&Default::default());
        let composite = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow composite"),
            layout: &self.composite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color),
            }],
        });
        Targets {
            size,
            color_texture,
            color,
            depth,
            depth_view,
            motion: None,
            motion_texture: None,
            capture_ready: false,
            motion_written: false,
            reveal: None,
            material: None,
            reduced_opaque: None,
            composite,
        }
    }
    /// Explicit source-only optical fixture access. No readback in rendering.
    pub(crate) fn source_fixture_layers(&self) -> Vec<wgpu::Texture> {
        self.views.lock().unwrap().iter().filter_map(|v|v.as_ref()?.targets.as_ref()
            .map(|t|t.color_texture.clone())).collect()
    }
    /// Synchronous, explicit diagnostic readback of the last encoded aggregate
    /// pass. Call only after its submission, and attach the host frame identity.
    /// No readback or filesystem work is performed by ordinary rendering.
    pub fn aggregate_raw(&self, gpu: &Gpu) -> Result<Vec<MeadowAggregateRaw>, String> {
        let Some(aggregate) = &self.aggregate else { return Ok(Vec::new()); };
        let inputs = {
            let views = self.views.lock().unwrap();
            let mut inputs = Vec::new();
            for (view, resources) in views.iter().enumerate() {
                let Some(target) = resources.as_ref().and_then(|v| v.aggregate.as_ref())
                    .and_then(|v| v.targets.as_ref()).filter(|t| t.capture_ready) else { continue; };
                let material = target.material.as_ref().ok_or("aggregate material capture missing")?;
                let motion = if target.motion_written {
                    Some(target.motion_texture.clone().ok_or("aggregate motion capture missing")?)
                } else { None };
                inputs.push(capture::Input {
                    view, size: [target.size.0, target.size.1],
                    sample_prune_enabled: aggregate.sample_prune_enabled,
                    half_resolution: aggregate.half_depth.is_some(),
                    color: target.color_texture.clone(), depth: target.depth.clone(),
                    material: material.texture.clone(), motion,
                });
            }
            inputs
        };
        capture::read(gpu, inputs)
    }
    pub fn motion_layer(&self, index: usize) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        let views = self.views.lock().unwrap();
        let v = views.get(index)?.as_ref()?;
        let t = if self.aggregate.is_some() {
            v.resolved.as_ref()?
        } else {
            v.targets.as_ref()?
        };
        Some((t.color.clone(), t.motion.as_ref()?.clone()))
    }
    fn ensure_motion(gpu: &Gpu, t: &mut Targets, motion: bool) {
        if !motion || t.motion.is_some() {
            return;
        }
        let texture = gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("meadow temporal positions"),
                    size: wgpu::Extent3d {
                        width: t.size.0,
                        height: t.size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
        t.motion = Some(texture.create_view(&Default::default()));
        t.motion_texture = Some(texture);
    }
    pub(crate) fn edge_shadow_buffer(&self) -> Option<&wgpu::Buffer> {
        self.edge_shadow.as_ref().map(|r| &r.buffer)
    }
    pub fn edge_shadow_stats(&self) -> Option<MeadowEdgeShadowStats> {
        self.edge_shadow.as_ref().map(|r| r.stats())
    }
    /// A small camera-local cache of camera-independent habitat decisions.
    /// It includes off-screen occluders and fades before its guarded boundary.
    pub(crate) fn prepare_edge_shadow(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder,
        index: usize, camera: &wgpu::Buffer, options: crate::debug_settings::DebugRenderOptions,
        time: Option<(&wgpu::QuerySet, u32)>) -> Result<(), String> {
        let Some(contact) = &self.edge_shadow else { return Ok(()); };
        contact.prepare(gpu, enc, index, camera, self.frame_settings(options)?, options.draws_meadow(), time)
    }
    /// Encode exact source visibility before opaque lighting. The later render
    /// consumes it once; settings/size changes are rejected rather than reused.
    pub(crate) fn prepare_visibility(
        &self, gpu:&Gpu, enc:&mut wgpu::CommandEncoder, index:usize, size:(u32,u32),
        scene:&wgpu::BindGroup, bindings:&[(u32,wgpu::BindingResource<'_>)],
        hdr:&wgpu::TextureView, opaque_depth:&wgpu::Texture,
        times:[Option<(&wgpu::QuerySet,u32)>;10], motion:bool,
        options:crate::debug_settings::DebugRenderOptions,
    )->Result<wgpu::Buffer,String> {
        if !self.skip_hidden_shade {return Err("hidden meadow preparation is disabled".into());}
        visibility::validate_hidden_size(gpu,size)?;
        self.render_stage(gpu,enc,index,size,scene,bindings,hdr,opaque_depth,times,motion,options,true)?;
        self.views.lock().unwrap()[index].as_ref().and_then(|v|v.visibility.as_ref())
            .and_then(|t|t.hidden_buffer()).ok_or_else(||"hidden meadow ID buffer missing".into())
    }
    pub fn render(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        scene: &wgpu::BindGroup,
        bindings: &[(u32, wgpu::BindingResource<'_>)],
        hdr: &wgpu::TextureView,
        opaque_depth: &wgpu::Texture,
        times: [Option<(&wgpu::QuerySet, u32)>; 10],
        motion: bool,
        options: crate::debug_settings::DebugRenderOptions,
    ) -> Result<(), String> {
        self.render_stage(gpu,enc,index,size,scene,bindings,hdr,opaque_depth,times,motion,options,false)
    }
    fn render_stage(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        scene: &wgpu::BindGroup,
        bindings: &[(u32, wgpu::BindingResource<'_>)],
        hdr: &wgpu::TextureView,
        opaque_depth: &wgpu::Texture,
        times: [Option<(&wgpu::QuerySet, u32)>; 10],
        motion: bool,
        options: crate::debug_settings::DebugRenderOptions,
        prepare_only: bool,
    ) -> Result<(), String> {
        options.validate()?;
        let ranges = crate::debug_settings::meadow_ranges([self.settings.shape[3], self.settings.range[0]], options.meadow_distance_scale);
        let mut views = self.views.lock().unwrap();
        if views.len() <= index {
            views.resize_with(index + 1, || None);
        }
        if views[index].is_none() {
            views[index] = Some(self.new_view(gpu));
        }
        let v = views[index].as_mut().unwrap();
        let mut settings = self.frame_settings(options)?;
        settings.previous_camera = self.cameras.lock().unwrap().previous(index);
        settings.previous_camera[3] *= settings.previous[3];
        let prepared = if self.skip_hidden_shade && !prepare_only {
            consume_prepared_visibility(&mut v.prepared_visibility,size,&settings)?;
            true
        } else {false};
        if prepare_only {v.prepared_visibility=None;}
        v.ranges = ranges;
        v.edits = [options.meadow_color_multiplier[0], options.meadow_color_multiplier[1], options.meadow_color_multiplier[2], options.meadow_density];
        v.size_scale = options.meadow_size_scale;
        v.lod_pixels = options.meadow_lod_pixels;
        v.spatial_filter_enabled = false;
        if !v.targets.as_ref().is_some_and(|t| t.size == size) {
            v.targets = Some(self.targets(gpu, size));
        }
        if let Some(far) = v.aggregate.as_mut() {
            let half=self.aggregate.as_ref().is_some_and(|a|a.half_depth.is_some());
            let far_size=half::size(size,half);
            if !far.targets.as_ref().is_some_and(|t| t.size == far_size) {
                far.targets = Some(self.targets(gpu, far_size));
            }
            Self::ensure_motion(gpu, far.targets.as_mut().unwrap(), motion);
            let target = far.targets.as_mut().unwrap();
            target.capture_ready = false;
            target.motion_written = false;
            if half && target.reduced_opaque.is_none() {
                target.reduced_opaque=Some(half::OpaqueTarget::new(gpu,far_size));
            }
            if target.material.is_none() {
                let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("meadow material normal reveal moments"),
                    size: wgpu::Extent3d {
                        width: far_size.0,
                        height: far_size.1,
                        depth_or_array_layers: 3,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let layers = std::array::from_fn(|layer| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: layer as u32,
                        array_layer_count: Some(1),
                        ..Default::default()
                    })
                });
                target.reveal = Some(layers[2].clone());
                target.material = Some(MaterialTargets {
                    view: texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        ..Default::default()
                    }),
                    layers,
                    texture,
                });
            }
            if !v.resolved.as_ref().is_some_and(|t| t.size == size) {
                v.resolved = Some(self.targets(gpu, size));
            }
            Self::ensure_motion(gpu, v.resolved.as_mut().unwrap(), motion);
        }
        if !v.pyramid.as_ref().is_some_and(|p| p.size == size) {
            v.pyramid = Some(self.depth_pyramid.target(gpu, size));
        }
        let pyramid = v.pyramid.as_ref().unwrap();
        let opaque_view = v.opaque_view.get(opaque_depth);
        if !prepared {self.depth_pyramid.encode(
            gpu,
            enc,
            &opaque_view,
            pyramid,
            times[0],
        );}
        let t = v.targets.as_mut().unwrap();
        Self::ensure_motion(gpu, t, motion);
        if !prepared {
        // Shader derives integer grid origin from the camera already in View.
        gpu.queue
            .write_buffer(&v.settings, 0, bytemuck::bytes_of(&settings));
        let initial = initial_draw_args(self.vertices);
        gpu.queue
            .write_buffer(&v.args, 0, bytemuck::cast_slice(&initial));
        }
        let view_resource = bindings
            .iter()
            .find(|(b, _)| *b == 0)
            .ok_or("meadow view binding missing")?
            .1
            .clone();
        let gen_view = v.placement_bind.entries(gpu, &self.generate_view_layout,
            "meadow placement view", &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view_resource.clone(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&pyramid.view),
                },
            ],
        );
        if !prepared {
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meadow placement"),
                timestamp_writes: times[0].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(i + 1),
                }),
            });
            pass.set_pipeline(&self.generate);
            pass.set_bind_group(0, &v.generation, &[]);
            pass.set_bind_group(1, &gen_view, &[]);
            pass.dispatch_workgroups(self.dimension.div_ceil(8), self.dimension.div_ceil(8), 1);
        }
        enc.copy_texture_to_texture(
            opaque_depth.as_image_copy(),
            t.depth.as_image_copy(),
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        }
        let view = v.lighting_bind.get(gpu, &self.view_layout, "meadow lights", bindings);
        if let Some(visibility) = &self.visibility {
            if v.visibility.as_ref().is_none_or(|target| target.size != size) {
                v.visibility = Some(visibility.target(gpu, size, &v.settings, &v.pool, &self.recipe, self.capacity, self.skip_hidden_shade));
            }
            visibility.render(enc, scene, &view, &v.draws, &v.args, t, v.visibility.as_ref().unwrap(),
                self.indices.as_ref(), motion, [times[6], times[7]], !prepared, !prepare_only);
            if prepare_only {
                v.visibility.as_ref().unwrap().copy_hidden(enc);
                if let Some((query_set,i))=times[6] {
                    let _end=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label:Some("meadow visibility ID copy end"),timestamp_writes:Some(wgpu::ComputePassTimestampWrites {
                            query_set,beginning_of_pass_write_index:None,end_of_pass_write_index:Some(i+1)}),
                    });
                }
                v.prepared_visibility=Some((size,settings));
                v.hidden_used_this_frame=true;
                return Ok(());
            }
        } else { for depth in [true, false] {
            let mut attachments = if depth {
                vec![]
            } else {
                vec![Some(wgpu::RenderPassColorAttachment {
                    view: &t.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })]
            };
            if !depth && motion {
                attachments.push(Some(wgpu::RenderPassColorAttachment {
                    view: t.motion.as_ref().unwrap(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }));
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if depth {
                    "meadow depth"
                } else {
                    "meadow shading"
                }),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: times[1].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: depth.then_some(i),
                    end_of_pass_write_index: (!depth).then_some(i + 1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(if depth {
                &self.depth_pipeline
            } else {
                &self.pipelines[usize::from(motion)]
            });
            pass.set_bind_group(0, scene, &[]);
            pass.set_bind_group(1, &view, &[]);
            for lod in 0..3 {
                pass.set_bind_group(2, &v.draws[lod], &[]);
                if let Some(indices) = &self.indices {
                    let index = if depth {
                        &indices.depth[lod]
                    } else {
                        &indices.color[lod]
                    };
                    pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed_indirect(&v.args, lod as u64 * DRAW_STRIDE);
                } else {
                    pass.draw_indirect(&v.args, lod as u64 * DRAW_STRIDE);
                }
            }
        }
        }
        if let (Some(aggregate), Some(far), Some(resolved)) =
            (&self.aggregate, v.aggregate.as_mut(), v.resolved.as_ref())
        {
            let ft = far.targets.as_ref().unwrap();
            let materials = ft.material.as_ref().unwrap();
            let mut far_settings = settings;
            far_settings.support[2] = 2.0;
            far_settings.support[3] = f32::from_bits(aggregate.atlas_base);
            far_settings.range[0] = ranges[3];
            // shape.w retains source fade start: no duplicate interior roots.
            far_settings.distances[0] = settings.range[0];
            far_settings.distances[1] = ranges[2];
            // Every view uses one frame-wide scale. Queue writes therefore
            // agree even though the merge uniform is shared between views.
            gpu.queue.write_buffer(&aggregate.transition, 0, bytemuck::cast_slice(&ranges));
            far_settings.dispatch = [0, 0, aggregate.dimension as i32, aggregate.capacity as i32];
            gpu.queue
                .write_buffer(&far.settings, 0, bytemuck::bytes_of(&far_settings));
            gpu.queue.write_buffer(
                &far.args,
                0,
                bytemuck::cast_slice(&initial_draw_args([6, 0, 0])),
            );
            if let Some(groups)=&self.groups {
                // This interval includes page request/cook GPU work as well
                // as ordinary far-root placement. Never report only the cheap
                // consumer while hiding cache production from the budget.
                if let Some((query_set,i))=times[2] {
                    let _pass=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label:Some("meadow group preparation start"),timestamp_writes:Some(wgpu::ComputePassTimestampWrites {
                            query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:None,
                        }),
                    });
                }
                groups.prepare(gpu,enc,self,far_settings,ranges)?;
            }
            let replacement_generated = if let Some(groups) = &self.groups {
                groups.generate_replacement(gpu, enc, index, &far.settings, &far.generation,
                    &gen_view, bindings, &opaque_view, aggregate.dimension, times[2])?
            } else { false };
            if !replacement_generated {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("meadow aggregate roots"),
                    timestamp_writes: times[2].map(|(query_set, i)| {
                        wgpu::ComputePassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: self.groups.is_none().then_some(i),
                            end_of_pass_write_index: Some(i + 1),
                        }
                    }),
                });
                pass.set_pipeline(&self.generate);
                pass.set_bind_group(0, &far.generation, &[]);
                pass.set_bind_group(1, &gen_view, &[]);
                pass.dispatch_workgroups(
                    aggregate.dimension.div_ceil(8),
                    aggregate.dimension.div_ceil(8),
                    1,
                );
            }
            if let (Some(reduce),Some(low))=(&aggregate.half_depth,&ft.reduced_opaque) {
                reduce.encode(gpu,enc,&opaque_view,low,times[3]);
            }
            let far_opaque=ft.reduced_opaque.as_ref().map_or(opaque_depth,|low|&low.texture);
            let far_opaque_view=ft.reduced_opaque.as_ref().map_or(&opaque_view,|low|&low.view);
            enc.copy_texture_to_texture(
                far_opaque.as_image_copy(),
                ft.depth.as_image_copy(),
                wgpu::Extent3d {
                    width: ft.size.0,
                    height: ft.size.1,
                    depth_or_array_layers: 1,
                },
            );
            let group_bind=if let Some(groups)=&self.groups {
                Some((groups.view_bind(gpu,bindings)?,groups.draw_bind(gpu,index,&far.settings,&far.pool,&self.recipe,
                    &aggregate.atlas,aggregate.capacity,&opaque_view),groups.stats(index).active_pages,
                    groups.page_draw_args(index)))
            } else {None};
            for depth in [true, false] {
                let mut attachments = if depth {
                    vec![]
                } else {
                    vec![Some(wgpu::RenderPassColorAttachment {
                        view: &materials.layers[0],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })]
                };
                if depth && motion {
                    attachments.push(Some(wgpu::RenderPassColorAttachment {
                        view: ft.motion.as_ref().unwrap(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }));
                }
                if !depth {
                    attachments.push(Some(wgpu::RenderPassColorAttachment {
                        view: &materials.layers[1],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }));
                    attachments.push(Some(wgpu::RenderPassColorAttachment {
                        view: ft.reveal.as_ref().unwrap(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    }));
                }
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(if depth {
                        "meadow aggregate depth"
                    } else {
                        "meadow aggregate material"
                    }),
                    color_attachments: &attachments,
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: if depth { &ft.depth_view } else { far_opaque_view },
                        depth_ops: depth.then_some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: times[if depth { 3 } else { 8 }].map(|(query_set, i)| {
                        wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: (!(depth && aggregate.half_depth.is_some())).then_some(i),
                            end_of_pass_write_index: Some(i + 1),
                        }
                    }),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if let (Some(groups),Some((camera,draw,active,page_args)))=(&self.groups,&group_bind) {
                    pass.set_pipeline(if depth {&groups.reference_depth[usize::from(motion)]}else{&groups.reference_material});
                    pass.set_bind_group(0,&groups.empty_scene,&[]);pass.set_bind_group(1,camera,&[]);pass.set_bind_group(2,draw,&[]);
                    pass.draw_indirect(&far.args,0);
                    pass.set_pipeline(if depth {&groups.page_depth[usize::from(motion)]}else{&groups.page_material});
                    if let Some(args) = page_args {
                        pass.draw_indirect(args, group_gpu::PAGE_DRAW_OFFSET);
                    } else { pass.draw(0..6,0..*active); }
                } else {
                    pass.set_pipeline(if depth { &aggregate.depth[usize::from(motion)] } else { &aggregate.pipeline });
                    pass.set_bind_group(0, scene, &[]);pass.set_bind_group(1, &view, &[]);pass.set_bind_group(2, &far.draws[0], &[]);
                    pass.draw_indirect(&far.args, 0);
                }
            }
            let resolve = far.resolve_bind.entries(gpu, &aggregate.resolve_layout,
                "meadow aggregate illumination", &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: far.settings.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: far.pool.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.recipe.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: far.resolve_draw.as_ref().unwrap().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&materials.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&ft.depth_view),
                    },
                ],
            );
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("meadow aggregate illumination"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &ft.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: times[9].map(|(query_set, i)| {
                        wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: Some(i),
                            end_of_pass_write_index: Some(i + 1),
                        }
                    }),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&aggregate.resolve);
                pass.set_bind_group(0, scene, &[]);
                pass.set_bind_group(1, &view, &[]);
                pass.set_bind_group(2, &resolve, &[]);
                pass.draw(0..3, 0..1);
            }
            let merge = v.merge_bind.entries(gpu, &aggregate.merge_layout,
                "meadow representation merge", &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&t.color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&ft.color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&t.depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&ft.depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(
                            t.motion.as_ref().unwrap_or(&t.color),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(
                            ft.motion.as_ref().unwrap_or(&ft.color),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: view_resource.clone(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: aggregate.transition.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&opaque_view),
                    },
                ],
            );
            let mut attachments = vec![Some(wgpu::RenderPassColorAttachment {
                view: &resolved.color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })];
            if motion {
                attachments.push(Some(wgpu::RenderPassColorAttachment {
                    view: resolved.motion.as_ref().unwrap(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }));
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meadow representation merge"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: times[4].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(i),
                    end_of_pass_write_index: Some(i + 1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&aggregate.merge[usize::from(motion)]);
            pass.set_bind_group(0, &merge, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            let target = far.targets.as_mut().unwrap();
            target.capture_ready = true;
            target.motion_written = motion;
        }
        {
            let filter = self.spatial_filter.as_ref().filter(|_| !motion && v.resolved.is_none()
                && !self.spatial_filter_temporal_active.load(std::sync::atomic::Ordering::Relaxed));
            let filtered = filter.map(|filter| {
                let opaque_view = opaque_depth.create_view(&Default::default());
                filter.bind(gpu,&t.color,&t.depth_view,&opaque_view,view_resource,&v.settings)
            });
            v.spatial_filter_enabled = filtered.is_some();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meadow composite"),
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
                timestamp_writes: times[5].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(i),
                    end_of_pass_write_index: Some(i + 1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let (Some(filter),Some(bind)) = (filter,filtered.as_ref()) {
                pass.set_pipeline(&filter.pipeline);
                pass.set_bind_group(0,bind,&[]);
            } else {
                pass.set_pipeline(&self.composite);
                pass.set_bind_group(0, &v.resolved.as_ref().unwrap_or(t).composite, &[]);
            }
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }
    /// Explicit readback only; normal gameplay never waits for GPU counters.
    pub fn diagnostics(&self, gpu: &Gpu) -> Result<Vec<MeadowStats>, String> {
        let views = self.views.lock().unwrap();
        let mut result = Vec::new();
        for (index, view) in views.iter().enumerate() {
            let Some(v) = view else { continue };
            let args_bytes = ARGS_BYTES * if v.aggregate.is_some() { 2 } else { 1 };
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("meadow draw counts"),
                size: args_bytes + if self.groups.is_some() { group_gpu::DIAGNOSTIC_BYTES } else { 0 },
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(&v.args, 0, &read, 0, ARGS_BYTES);
            if let Some(far) = &v.aggregate {
                enc.copy_buffer_to_buffer(&far.args, 0, &read, ARGS_BYTES, ARGS_BYTES);
            }
            if let Some(groups) = &self.groups {
                groups.copy_diagnostics(&mut enc, index, &read, args_bytes);
            }
            gpu.queue.submit([enc.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            gpu.wait();
            let data = read
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("meadow readback: {e}"))?;
            let words: &[u32] = bytemuck::cast_slice(&data);
            let plants = std::array::from_fn(|lod| words[lod * DRAW_WORDS + 1]);
            if plants.iter().any(|n| *n > self.capacity) {
                return Err("meadow compact overflow".into());
            }
            let aggregate_plants = if self.aggregate.is_some() {
                words[DRAW_WORDS * 3 + 1]
            } else {
                0
            };
            if aggregate_plants > self.aggregate.as_ref().map_or(0, |a| a.capacity) {
                return Err("meadow aggregate compact overflow".into());
            }
            let coverage = v.visibility.as_ref().map(|target|
                visibility::read_coverage(gpu, target, self.capacity, self.vertices)).transpose()?;
            let source_geometry_passes = if self.visibility.is_some() { 1 } else { 2 };
            result.push(MeadowStats {
                view: index,
                plants,
                size_scale: v.size_scale,
                lod_pixels: v.lod_pixels,
                density: v.edits[3],
                color_multiplier: v.edits[..3].try_into().unwrap(),
                triangles: std::array::from_fn(|lod| {
                    u64::from(plants[lod]) * u64::from(self.vertices[lod] / 3)
                }),
                capacity_per_view: self.capacity,
                pool_bytes: self.pool_bytes,
                indexed: self.indices.is_some(),
                index_bytes: self.index_bytes,
                source_vertices_per_patch: self.vertices,
                depth_unique_vertices_per_patch: self.depth_unique_vertices,
                color_unique_vertices_per_patch: self.color_unique_vertices,
                depth_vertex_reference_budget: std::array::from_fn(|lod| {
                    u64::from(plants[lod])
                        * u64::from(if self.indices.is_some() {
                            self.depth_unique_vertices[lod]
                        } else {
                            self.vertices[lod]
                        })
                }),
                color_vertex_reference_budget: std::array::from_fn(|lod| {
                    u64::from(!self.visibility.is_some()) * u64::from(plants[lod])
                        * u64::from(if self.indices.is_some() {
                            self.color_unique_vertices[lod]
                        } else {
                            self.vertices[lod]
                        })
                }),
                visibility_enabled: self.visibility.is_some(),
                visibility_target_bytes: v.visibility.as_ref().map_or(0, |target| target.bytes()),
                source_geometry_passes,
                submitted_source_triangles: std::array::from_fn(|lod|
                    u64::from(plants[lod]) * u64::from(self.vertices[lod] / 3) * u64::from(source_geometry_passes)),
                visibility_covered_pixels: coverage.as_ref().map(|c| c.pixels),
                visibility_unique_primitives: coverage.as_ref().map(|c| c.primitives),
                visibility_unique_roots: coverage.as_ref().map(|c| c.roots),
                visibility_invalid_ids: coverage.as_ref().map_or(0, |c| c.invalid),
                skip_hidden_shade_enabled:self.skip_hidden_shade,
                spatial_filter_requested:self.spatial_filter_requested,
                spatial_filter_enabled:v.spatial_filter_enabled,
                spatial_filter_max_strength:if v.spatial_filter_enabled {spatial_filter::MAX_STRENGTH} else {0.0},
                spatial_filter_blade_width_m:self.spatial_filter.as_ref().map_or(0.0,|filter|filter.blade_width*self.settings.shape[0]*v.size_scale),
                opaque_shade_skipped_pixels:if v.hidden_used_this_frame {coverage.as_ref().map_or(0,|c|c.skipped)} else {0},
                hidden_shade_buffer_bytes:v.visibility.as_ref().map_or(0,|t|t.hidden_bytes()),
                aggregate_enabled: self.aggregate.is_some(),
                aggregate_sample_prune_enabled: self.aggregate.as_ref().is_some_and(|a|a.sample_prune_enabled),
                aggregate_half_resolution: self.aggregate.as_ref().is_some_and(|a|a.half_depth.is_some()),
                aggregate_target_size: v.aggregate.as_ref().and_then(|f|f.targets.as_ref()).map_or([0;2],|t|[t.size.0,t.size.1]),
                aggregate_resolution_scale: match (v.targets.as_ref(),v.aggregate.as_ref().and_then(|f|f.targets.as_ref())) {
                    (Some(source),Some(far))=>[far.size.0 as f32/source.size.0 as f32,far.size.1 as f32/source.size.1 as f32],
                    _=>[0.0;2],
                },
                aggregate_plants,
                aggregate_triangles: u64::from(aggregate_plants) * 2,
                aggregate_capacity: self.aggregate.as_ref().map_or(0, |a| a.capacity),
                aggregate_pool_bytes: self.aggregate.as_ref().map_or(0, |a| a.pool_bytes),
                aggregate_recipe_bytes: self.aggregate.as_ref().map_or(0, |a| a.recipe_bytes),
                aggregate_cook_ms: self.aggregate.as_ref().map_or(0.0, |a| a.cook_ms),
                aggregate_transition_m: [v.ranges[0], v.ranges[1]],
                aggregate_outer_fade_m: [v.ranges[2], v.ranges[3]],
                aggregate_target_bytes: match (v.resolved.as_ref(),v.aggregate.as_ref().and_then(|f|f.targets.as_ref())) {
                    (Some(resolved),Some(far))=>half::target_bytes(resolved.size,far.size,far.motion.is_some(),far.reduced_opaque.is_some()),
                    _=>0,
                },
                group_cache:{let mut stats=self.groups.as_ref().map_or_else(GroupCacheStats::default,|g|g.stats(index));
                    if stats.enabled {
                        stats.reference_roots_submitted=aggregate_plants;
                        group_gpu::GpuGroups::apply_diagnostics(&mut stats, &words[(args_bytes / 4) as usize..])?;
                    } stats},
            });
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_morph_camera_history_is_per_view_and_commits_only_after_all_views() {
        use glam::Vec3;
        let mut history=super::CameraHistory::default();
        history.prepare([Vec3::X,Vec3::Y]);
        assert_eq!(history.previous(0),[0.0;4]);
        history.finish();
        history.prepare([Vec3::X*2.0,Vec3::Y*3.0]);
        assert_eq!(history.previous(0),[1.0,0.0,0.0,1.0]);
        assert_eq!(history.previous(1),[0.0,1.0,0.0,1.0]);
        // Re-encoding/preparing an aborted frame must not become history.
        history.prepare([Vec3::X*4.0,Vec3::Y*5.0]);
        assert_eq!(history.previous(0),[1.0,0.0,0.0,1.0]);
        history.finish();
        assert_eq!(history.previous(0),[4.0,0.0,0.0,1.0]);
        assert_eq!(history.previous(1),[0.0,5.0,0.0,1.0]);
        history.prepare([Vec3::Z]);
        assert_eq!(history.previous(0),[0.0;4],"layout changes discard old view identities");
    }
    #[test]
    fn hidden_source_preparation_is_single_use_and_rejects_resize_or_changed_wind() {
        use bytemuck::Zeroable;
        let settings=super::Settings::zeroed();
        let mut prepared=Some(((1920,1080),settings));
        assert!(super::consume_prepared_visibility(&mut prepared,(1920,1080),&settings).is_ok());
        assert!(super::consume_prepared_visibility(&mut prepared,(1920,1080),&settings).is_err());
        prepared=Some(((1920,1080),settings));
        assert!(super::consume_prepared_visibility(&mut prepared,(1280,720),&settings).is_err());
        assert!(prepared.is_none());
        prepared=Some(((1920,1080),settings));
        let mut changed=settings;changed.range[3]=0.1;
        assert!(super::consume_prepared_visibility(&mut prepared,(1920,1080),&changed).is_err());
        assert!(prepared.is_none());
    }
    #[test]
    fn brick_and_source_settings_share_rendered_wind_clock_and_live_edits() {
        use bytemuck::Zeroable;
        let mut base = super::Settings::zeroed();
        base.field = [-4.0, -4.0, 0.25, 0.45];
        base.grid = [33, 33, 73, 3];
        base.shape = [1.25, 0.4, 0.6, 60.0];
        base.range = [80.0, 0.0, 0.0, 0.0];
        base.support = [0.38, 0.5, 1.0, 0.0];
        base.lods = [[0, 5400, 0, 0], [5400, 3240, 0, 0], [8640, 1800, 0, 0]];
        base.dispatch = [0, 0, 363, 131769];
        let options = crate::debug_settings::DebugRenderOptions {
            meadow_size_scale: 1.5, meadow_distance_scale: 0.5,
            meadow_lod_pixels: 2.0, meadow_density: 0.5,
            meadow_color_multiplier: [1.25, 0.5, 1.0], ..Default::default()
        };
        let current = crate::wind::WindState {
            time_seconds: 135.0, direction: glam::Vec2::new(0.6, 0.8),
            strength: 0.7, grass_shear: 0.2, ..Default::default()
        };
        let previous = crate::wind::WindState { time_seconds: 119.75, ..current };
        for valid in [false, true] {
            let clock = super::Clock { current, previous, valid };
            let source = base.for_frame(options, &clock).unwrap();
            let brick = base.for_frame(options, &clock).unwrap();
            assert_eq!(bytemuck::bytes_of(&source), bytemuck::bytes_of(&brick));
            let mut expected = base;
            expected.shape[0] = 1.875;
            expected.shape[3] = 30.0;
            expected.range = [40.0, 0.7, 0.5, (std::f64::consts::TAU * 15.0 / 120.0) as f32];
            expected.previous = [
                if valid { (std::f64::consts::TAU * 119.75 / 120.0) as f32 } else { expected.range[3] },
                0.6, 0.8, if valid { 1.0 } else { 0.0 },
            ];
            expected.distances[2] = 2.0;
            expected.edits = [1.25, 0.5, 1.0, 0.5];
            expected.animation = [135.0, 6.75, if valid {119.75} else {135.0}, if valid {5.9875} else {6.75}];
            expected.wind_history = [0.6, 0.8, 0.7 * 0.2 / 0.4, 0.0];
            assert_eq!(bytemuck::bytes_of(&source), bytemuck::bytes_of(&expected),
                "only shared frame controls may change; recipe, terrain, root identity and capacity stay exact");
            assert_eq!(clock.current, current);
            assert_eq!(clock.previous, previous);
            assert_eq!(clock.valid, valid, "encoding must not advance displayed history");
        }
        let clock = super::Clock { current, previous, valid: true };
        assert!(base.for_frame(crate::debug_settings::DebugRenderOptions {
            meadow_size_scale: f32::NAN, ..options
        }, &clock).is_err());
    }

    #[test]
    fn live_meadow_lod_only_changes_existing_error_threshold() {
        use bytemuck::Zeroable;
        let mut original = super::Settings::zeroed();
        original.shape = [1.25, 0.38, 0.5, 60.0];
        original.range = [80.0, 0.6, 1.0, 0.5];
        original.distances = [20.0, 50.0, 1.0, 1.0];
        original.edits = [1.0; 4];
        original.dispatch = [0, 0, 363, 131769];
        let defaults = crate::debug_settings::DebugRenderOptions::default();
        assert_eq!(bytemuck::bytes_of(&original), bytemuck::bytes_of(&original.with_live_options(defaults)));
        for pixels in [0.5, 1.0, 2.0, 4.0] {
            let actual = original.with_live_options(crate::debug_settings::DebugRenderOptions {
                meadow_lod_pixels: pixels, ..defaults
            });
            let mut expected = original;
            expected.distances[2] = pixels;
            assert_eq!(bytemuck::bytes_of(&actual), bytemuck::bytes_of(&expected),
                "LOD must not change distance, geometry bounds, density, wind or pool capacity");
        }
        assert_eq!(std::mem::size_of::<super::Settings>(), 240);
    }

    fn placement_random(cell: [i32; 2], salt: u32, seed: u32) -> f64 {
        let mut x = (cell[0] as u32).wrapping_mul(0x9e3779b1)
            ^ (cell[1] as u32).wrapping_mul(0x85ebca77)
            ^ salt.wrapping_mul(0xc2b2ae3d) ^ seed;
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb352d);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846ca68b);
        f64::from((x ^ (x >> 16)) >> 8) / 16777216.0
    }

    #[test]
    fn world_growth_is_bounded_and_smoother_than_independent_patch_heights() {
        let module = naga::front::wgsl::parse_str(&super::generate_source()).unwrap();
        let constant = |name: &str| {
            let c = module.constants.iter().find(|(_, c)| c.name.as_deref() == Some(name)).unwrap().1;
            let naga::Expression::Literal(naga::Literal::F32(value)) = module.global_expressions[c.init]
                else { panic!("growth contract must have explicit scalar bounds") };
            f64::from(value)
        };
        let broad_metres = constant("MEADOW_GROWTH_BROAD_METRES");
        let local_metres = constant("MEADOW_GROWTH_LOCAL_METRES");
        let broad_weight = constant("MEADOW_GROWTH_BROAD_WEIGHT");
        let local_weight = constant("MEADOW_GROWTH_LOCAL_WEIGHT");
        let root_weight = constant("MEADOW_GROWTH_ROOT_WEIGHT");
        assert!((broad_weight+local_weight+root_weight-1.0).abs() < 1e-7);
        assert!(broad_metres >= 9.0 && local_metres >= 3.0 && root_weight <= 0.05000001);
        let growth_fn = module.functions.iter()
            .find(|(_, f)| f.name.as_deref() == Some("meadow_growth_factor")).unwrap().1;
        assert!(growth_fn.expressions.iter().all(|(_, e)| !matches!(e, naga::Expression::GlobalVariable(_))),
            "the growth field must not directly depend on camera/time/LOD uniforms");
        let noise = |p: [f64; 2], seed| {
            let cell = p.map(|v| v.floor() as i32);
            let f = [p[0]-f64::from(cell[0]), p[1]-f64::from(cell[1])].map(|v| v*v*(3.0-2.0*v));
            let mix = |a: f64, b: f64, t: f64| a+(b-a)*t;
            mix(mix(placement_random(cell,103,seed),placement_random([cell[0]+1,cell[1]],103,seed),f[0]),
                mix(placement_random([cell[0],cell[1]+1],103,seed),placement_random([cell[0]+1,cell[1]+1],103,seed),f[0]),f[1])
        };
        let root = |cell: [i32; 2], seed| [0,1].map(|axis|
            (f64::from(cell[axis])+0.5+(placement_random(cell,[157,163][axis],seed)*2.0-1.0)*0.04)*0.45);
        let growth = |p: [f64; 2], cell, seed| (0.7+0.6*(
            broad_weight*noise([p[0]/broad_metres-26.3,p[1]/broad_metres+17.1],seed)
            +local_weight*noise([p[0]/local_metres+5.7,p[1]/local_metres-12.9],seed)
            +root_weight*placement_random(cell,9,seed))).clamp(0.7,1.3);
        // Smoothstep value noise has axis derivative <=1.5. This analytic
        // neighbour bound includes the worst independent root jitter and
        // root-level variation; it does not rely on observing a lucky seed.
        let slope = 0.6*1.5*(broad_weight/broad_metres+local_weight/local_metres);
        let max_delta = slope*(0.45+4.0*0.018)+0.6*root_weight;
        assert!(max_delta < 0.101);
        let mut new_squared = 0.0;
        let mut old_squared = 0.0;
        for seed in [0,73,u32::MAX] {
            for z in -32..32 {
                for x in -32..32 {
                    let a = [x,z]; let b = [x+1,z];
                    let ga = growth(root(a,seed),a,seed); let gb = growth(root(b,seed),b,seed);
                    assert!((0.7..=1.3).contains(&ga) && (0.7..=1.3).contains(&gb));
                    assert!((ga-gb).abs() <= max_delta+1e-12);
                    new_squared += (ga-gb).powi(2);
                    old_squared += (0.6*(placement_random(a,9,seed)-placement_random(b,9,seed))).powi(2);
                }
            }
        }
        assert!(new_squared < old_squared*0.0625, "neighbour RMS must fall by at least fourfold");
    }

    #[test]
    fn bounded_world_root_jitter_preserves_cells_spacing_and_grid_coverage() {
        // Read the shader's bound, then test geometric invariants rather than
        // asserting a particular sequence of random samples. This is a CPU
        // contract test; actual GPU placement parity is tested separately.
        let module = naga::front::wgsl::parse_str(&super::generate_source()).unwrap();
        let constant = module.constants.iter()
            .find(|(_, c)| c.name.as_deref() == Some("MEADOW_ROOT_JITTER")).unwrap().1;
        let naga::Expression::Literal(naga::Literal::F32(jitter)) = module.global_expressions[constant.init]
            else { panic!("root jitter must have an explicit f32 bound") };
        let jitter = f64::from(jitter);
        assert!(jitter > 0.0 && jitter <= 0.04);
        let spacing = f64::from(crate::meadow_data::ROOT_CELL);
        let fraction = |cell, seed| [157, 163].map(|salt|
            0.5 + (placement_random(cell, salt, seed) * 2.0 - 1.0) * jitter);
        let mut yaw_bins = [0u32; 32];
        for seed in [0, 73, u32::MAX] {
            for z in -32..32 {
                for x in -32..32 {
                    let f = fraction([x, z], seed);
                    assert!(f.iter().all(|v| *v >= 0.5-jitter && *v < 0.5+jitter));
                    for axis in 0..2 {
                        let mut neighbor = [x, z]; neighbor[axis] += 1;
                        let separation = (1.0 + fraction(neighbor, seed)[axis] - f[axis])*spacing;
                        assert!(separation >= (1.0-2.0*jitter)*spacing);
                    }
                    yaw_bins[(placement_random([x, z], 3, seed)*32.0) as usize] += 1;
                }
            }
        }
        assert!(yaw_bins.iter().all(|n| *n > 0), "yaw must not collapse to quarter turns");
        // First cell outside either source/far pool cannot jitter into the
        // radius, even with a camera just before the next world-cell boundary.
        for radius in [10.0f64, 80.0, 100.0, 180.0, 250.0, 256.0] {
            let half_grid = (radius/spacing).ceil() + 3.0;
            for camera_fraction in [0.0, 0.5, 1.0] {
                let right = (half_grid+1.0+0.5-jitter-camera_fraction)*spacing;
                let left = (camera_fraction+half_grid+1.0-0.5-jitter)*spacing;
                assert!(right > radius && left > radius);
            }
        }
    }

    #[test]
    fn full_angle_patch_rotation_preserves_wind_and_terrain_sphere_bounds() {
        use glam::{DQuat, DVec3};
        // Independent double-precision rotations: fixed anchor + scaled blade,
        // then full yaw and native terrain shear. No axis-aligned-square
        // assumption is allowed in the frustum/Hi-Z/road radial bound.
        // Current tapered recipe: nominal .36 m plus .02 m boundary lobes.
        let anchor_radius = 0.38;
        let blade_radius = 0.6;
        let max_angle = 0.22;
        for yaw_step in 0..127 {
            let yaw = DQuat::from_rotation_y(yaw_step as f64 * std::f64::consts::TAU/127.0);
            // Full configured growth(.1..1.5), world variation(.7..1.3),
            // and live size(.5..2) range, not merely the default field size.
            for scale in [0.035, 0.5, 1.0, 1.3, 2.6, 3.9] {
                for angle in [-max_angle*1.5, 0.0, max_angle*1.5] {
                    let axis = DVec3::new(0.3, 0.0, 0.7).normalize();
                    let blade = DQuat::from_axis_angle(axis, angle)
                        * DVec3::new(0.2, 0.5, -0.1).normalize()*blade_radius*scale;
                    let p = yaw*(DVec3::new(1.0, 0.0, 1.0).normalize()*anchor_radius+blade);
                    for slope in [[0.0f64, 0.0], [0.6, -0.8], [-0.8, 0.6]] {
                        let length = slope[0].hypot(slope[1]);
                        let ground_scale = (1.0+0.25*length*length).sqrt()+0.5*length;
                        let actual = p+DVec3::Y*(p.x*slope[0]+p.z*slope[1]);
                        let bound = (anchor_radius+blade_radius*scale*(1.0+max_angle*1.5))*ground_scale;
                        assert!(actual.length() <= bound+1e-12);
                    }
                }
            }
        }
    }

    #[test]
    fn resolved_union_fades_once_independent_of_overlap_count() {
        let union = |alphas: &[f64]| 1.0 - alphas.iter().map(|a| 1.0 - a).product::<f64>();
        let fade = |distance: f64| {
            let t = ((distance - 160.0) / 20.0).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
        assert!((union(&[0.2, 0.9]) - 0.92).abs() < 1e-14);
        for layers in [1, 2, 50, 1000] {
            let covered = union(&vec![0.2; layers]);
            for (distance, expected_ratio) in [(160.0, 1.0), (170.0, 0.5), (180.0, 0.0)] {
                let resolved_alpha = covered * fade(distance);
                assert!((resolved_alpha / covered - expected_ratio).abs() < 1e-14);
                // Scaling premultiplied radiance and alpha together preserves
                // the material/lighting colour instead of darkening its RGB.
                if resolved_alpha > 0.0 {
                    let radiance = 1200.0 * covered * fade(distance);
                    assert!((radiance / resolved_alpha - 1200.0).abs() < 1e-10);
                }
            }
        }
        // Concrete regression: per-fragment fade stays visibly opaque until
        // the hard range boundary, despite a requested two-percent envelope.
        let wrong = union(&vec![0.02; 50]);
        assert!(wrong > 0.63);
        let correct = union(&vec![1.0; 50]) * 0.02;
        assert!((correct - 0.02).abs() < 1e-14);
    }
    #[test]
    fn shaders_validate() {
        for source in [
            super::generate_source(),
            super::shader_source(),
            super::aggregate_resolve_source(),
            super::COMPOSITE.into(),
            include_str!("shaders/meadow_merge.wgsl").into(),
            include_str!("shaders/meadow_half_depth.wgsl").into(),
            crate::depth_pyramid::SOURCE.into(),
        ] {
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
    }
    #[test]
    fn settings_abi() {
        assert_eq!(std::mem::size_of::<super::Settings>(), 240);
        assert_eq!(std::mem::offset_of!(super::Settings, edits),176);
        for source in [super::generate_source(),super::shader_source(),super::aggregate_resolve_source()] {
            let module=naga::front::wgsl::parse_str(&source).unwrap();
            let ty=module.types.iter().find(|(_,t)|t.name.as_deref()==Some("MeadowSettings")).unwrap().1;
            let naga::TypeInner::Struct{members,span}=&ty.inner else {panic!("settings must be a struct")};
            assert_eq!(*span,240);
            for (name,offset) in [("edits",176),("animation",192),("wind_history",208),("previous_camera",224)] {
                assert_eq!(members.iter().find(|m|m.name.as_deref()==Some(name)).unwrap().offset,offset);
            }
        }
    }

    #[test]
    fn live_patch_count_is_world_fixed_monotone_and_shared_by_both_representations() {
        let source=super::generate_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap();
        let function=module.functions.iter().find(|(_,f)|f.name.as_deref()==Some("meadow_patch_retained")).unwrap().1;
        assert!(function.expressions.iter().all(|(_,e)|!matches!(e,naga::Expression::GlobalVariable(_))),
            "selection must not depend on view/time/draw slots");
        let constant=module.constants.iter().find(|(_,c)|c.name.as_deref()==Some("MEADOW_DENSITY_SALT")).unwrap().1;
        let naga::Expression::Literal(naga::Literal::U32(salt))=module.global_expressions[constant.init] else {panic!("explicit count hash salt")};
        let producer=module.functions.iter().find(|(_,f)|f.name.as_deref()==Some("meadow_generate_cell")).unwrap().0;
        let selection=module.functions.iter().find(|(_,f)|f.name.as_deref()==Some("meadow_patch_retained")).unwrap().0;
        assert!(module.functions[producer].expressions.iter().any(|(_,e)|matches!(e,naga::Expression::CallResult(h) if *h==selection)),
            "the shared producer must apply the stable world-cell subset");
        for entry in ["generate_meadow","generate_meadow_page"] {
            let function=&module.entry_points.iter().find(|e|e.name==entry).unwrap().function;
            assert!(function.expressions.iter().any(|(_,e)|matches!(e,naga::Expression::CallResult(h) if *h==producer)),
                "visible and cached roots must call the same root producer");
        }
        assert!(source.find("if !meadow_patch_retained").unwrap()<source.find("atomicAdd(&meadow_args").unwrap());
        let mut counts=[0;5];
        for seed in [0,73,u32::MAX] {
            for z in -32..32 {for x in -32..32 {
                let value=placement_random([x,z],salt,seed);
                let selected=[0.0,0.25,0.5,0.75,1.0].map(|fraction|value<fraction);
                assert!(!selected[0] && selected[4]);
                assert!(selected.windows(2).all(|s|!s[0] || s[1]));
                for (count,present) in counts.iter_mut().zip(selected) {*count+=u32::from(present);}
                // Re-indexing the same world cell in a different view grid
                // cannot change the subset or its animation/variant identity.
                for origin in [[-41,-52],[7,13]] {
                    let local=[x-origin[0],z-origin[1]];
                    let reconstructed=[local[0]+origin[0],local[1]+origin[1]];
                    assert_eq!(value,placement_random(reconstructed,salt,seed));
                }
            }}
        }
        let total=3*64*64;
        assert_eq!(counts[4],total);
        for (count,fraction) in counts.into_iter().zip([0.0,0.25,0.5,0.75,1.0]) {
            assert!((f64::from(count)/f64::from(total)-fraction).abs()<0.02);
        }
        // Explicit pigment multiplication occurs before identical leaf BRDFs,
        // not in lighting or opacity and not via the quantized root pigment.
        assert!(include_str!("shaders/meadow_render.wgsl").contains("blade_tint*meadow.edits.rgb"));
        assert!(include_str!("shaders/meadow_aggregate.wgsl").contains("tint*meadow.edits.rgb"));
    }
    #[test]
    fn indirect_record_matches_both_draw_commands_and_wgsl() {
        let mut args = super::initial_draw_args([5400, 3240, 1800]);
        let counts = [19, 37, 83];
        for lod in 0..3 {
            args[lod * super::DRAW_WORDS + 1] = counts[lod];
            let record = &args[lod * super::DRAW_WORDS..][..super::DRAW_WORDS];
            assert_eq!(record, &[[5400, 3240, 1800][lod], counts[lod], 0, 0, 0]);
            // Non-indexed first_vertex/first_instance and indexed
            // first_index/base_vertex/first_instance are all exactly zero.
            assert_eq!(&record[2..], &[0, 0, 0]);
        }
        let module = naga::front::wgsl::parse_str(&super::generate_source()).unwrap();
        let ty = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("MeadowDrawArgs"))
            .unwrap()
            .1;
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("draw args must be a struct")
        };
        assert_eq!(*span as u64, super::DRAW_STRIDE);
        assert_eq!(
            members.iter().map(|m| m.offset).collect::<Vec<_>>(),
            [0, 4, 8, 12, 16]
        );
        assert_eq!(std::mem::size_of_val(&args) as u64, super::ARGS_BYTES);
    }
}
