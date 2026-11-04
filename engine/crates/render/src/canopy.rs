//! Persistent world-anchored canopy extinction, shared by sun and sky queries.
//!
//! This is a rest-pose ellipsoid approximation, not source-needle voxel data.
//! `KERN_SOURCE_CROWN_BOUNDS=1` tests source-leaf envelopes instead of the
//! whole-tree sphere. This fixes placement/scale, not lost light transmission;
//! it remains opt-in pending source-alpha/mean-transmission GPU validation.
//! Extinction adds between crowns at EVERY level (the old field used max).
//! Structural tree changes invalidate the field; a future wind phase must not
//! be treated as a structural revision or trigger per-leaf rebuilds.
//! Each view owns its receiver window so other players cannot change its LOD
//! blend or lighting. Three levels cost 27 MiB per view, 108 MiB for four,
//! plus 25 MiB per view for the sky transmittance cache: three more Y slabs
//! in the same texture, baked by the renderer (`shade.wgsl`
//! `bake_sky_visibility`) through a scratch level whenever a level changes.
//! `KERN_CANOPY_COMPACT_SUPPORT=1` additionally bounds reconstruction by the
//! source leaves' vertical envelope. It preserves integrated proxy mass, not
//! directional gaps/mean transmission. Opt-in pending matched GPU acceptance;
//! scratch adds 18 MiB per view and sampling uses eight loads per active level.
use crate::{scene::{GpuInstance, Scene, FOLIAGE, ALPHA_TEST, THIN_FOLIAGE, GRASS}, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

pub const CANOPY_DIMS: [u32; 3] = [128, 48, 128];
pub const CANOPY_LEVELS: usize = 3;
/// Y slabs in the canopy texture: extinction levels 0..3, then the cached
/// sky transmittance of each level (L1: mean and gradient).
pub const CANOPY_SLABS: u32 = 6;
pub const CANOPY_SNAP_CELLS: f32 = 8.0;
/// Half the snap displacement plus one trilinear-filter cell.
pub const CANOPY_SUPPORT_GUARD_CELLS: f32 = 5.0;
pub const CANOPY_FADE_CELLS: f32 = 8.0;
const COUNT: u64 = 128 * 48 * 128;
const MAX_CROWN_CELLS: u64 = 32_768;
// Sideways supports 8 km stages: native corridor crowns are additional to
// the 200k distant-forest budget. This limit does not size the fixed atlas.
const MAX_FOLIAGE: usize = 1_048_575;
const RENDER_REACH: f32 = 1000.0;
const RAY_GUARD: f32 = 60.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FillUniform {
    origins: [[f32; 4]; 3],
    dims: [u32; 4],
    mass: [f32; 4],
    state: [u32; 4],
}

#[derive(Clone, Copy, PartialEq)]
struct Layout {
    origins: [[f32; 4]; 3],
    anchors: [[f32; 4]; 3],
}

#[derive(Default)]
struct Cache {
    origins: Option<[[f32; 4]; 3]>,
    revision: u64,
    coarse_pitch: f32,
}

pub struct CanopyFrame {
    pub view: wgpu::TextureView,
    /// The whole canopy texture, for copying a baked transmittance level
    /// into its slab (`CANOPY_DIMS[1] * (3 + level)`).
    pub texture: wgpu::Texture,
    /// One level's worth of scratch the transmittance bake writes.
    pub scratch: wgpu::TextureView,
    /// xyz is storage lower corner, w is metres per voxel.
    pub origins: [[f32; 4]; 3],
    /// Continuous centres for blending, independent of snapped storage.
    pub anchors: [[f32; 4]; 3],
    pub dims: [u32; 3],
    /// Bit mask of levels rebuilt this frame; zero means an actual cache hit.
    pub updated_levels: u32,
    /// GiUniform.vol_dims.w bit 1 selects bounded reconstruction (bit 0: GI).
    pub compact_support: bool,
}

pub struct CanopyClipmap {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    scratch: wgpu::TextureView,
    atomic: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    fill: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    cache: Mutex<Cache>,
    // Original enclosing radius and integrated source-envelope mass.
    initial_crowns: Arc<Vec<Option<[f64; 2]>>>,
    initial_heights: Arc<Vec<Option<[f32; 2]>>>,
    foliage_y: [f32; 2],
    mass_scale: f32,
    source_crowns: Arc<SourceCrowns>,
    source_buffer: wgpu::Buffer,
    compact_support: bool,
    source_bounds: bool,
}

/// Per-view resident data: 27 MiB, bounded at 108 MiB for four views.
/// Pipelines and scene validation metadata are shared by `fork_view`.
struct ViewResources {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    scratch: wgpu::TextureView,
    atomic: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

fn view_resources(gpu: &Gpu, layout: &wgpu::BindGroupLayout, source_buffer: &wgpu::Buffer, compact_support: bool) -> ViewResources {
    let atomic = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("canopy clipmap mass/support"), size: COUNT * 3 * 4 * if compact_support { 3 } else { 1 },
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("canopy clipmap extinction"),
        size: wgpu::Extent3d { width: 128, height: 48 * CANOPY_SLABS, depth_or_array_layers: 128 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let scratch = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("canopy sky transmittance scratch"),
        size: wgpu::Extent3d { width: 128, height: 48, depth_or_array_layers: 128 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }).create_view(&Default::default());
    let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("canopy clipmap fill parameters"), size: std::mem::size_of::<FillUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let view = texture.create_view(&Default::default());
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("canopy clipmap fill"), layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: atomic.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 3, resource: source_buffer.as_entire_binding() },
        ],
    });
    ViewResources { texture, view, scratch, atomic, uniform, bind }
}

fn legacy_crown(instance: &GpuInstance) -> (Vec3, Vec3) {
    let r = instance.sphere[3];
    (Vec3::new(instance.sphere[0], instance.sphere[1] + r * 0.25, instance.sphere[2]),
        Vec3::new(r * 0.38, r * 0.62, r * 0.38))
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct CrownBounds { centre: [f32; 4], half: [f32; 4] }

/// One compact ID per instance; local bounds are shared by every reuse of a
/// mesh. This buffer is shared across views, unlike the resident clipmaps.
struct SourceCrowns { ids: Vec<u32>, bounds: Vec<CrownBounds> }

impl SourceCrowns {
    fn new(scene: &Scene) -> Self {
        let mut result = Self { ids: Vec::with_capacity(scene.instances.len()), bounds: vec![CrownBounds::default()] };
        let mut meshes = HashMap::new();
        for instance in &scene.instances {
            if instance.flags & FOLIAGE == 0 { result.ids.push(0); continue; }
            let id = *meshes.entry((instance.first_cluster, instance.cluster_count)).or_insert_with(|| {
                let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
                let start = instance.first_cluster as usize;
                let end = start.saturating_add(instance.cluster_count as usize).min(scene.clusters.len());
                for c in scene.clusters.get(start..end).unwrap_or_default() {
                    if c.own_error != 0.0 { continue; }
                    let Some(m) = scene.materials.get(c.material as usize) else { continue; };
                    if m.flags & (ALPHA_TEST | THIN_FOLIAGE) == 0 || m.flags & GRASS != 0 { continue; }
                    for ti in c.first_tri..c.first_tri.saturating_add(c.tri_count) {
                        let Some(&tri) = scene.tris.get(ti as usize) else { continue; };
                        for corner in 0..3 {
                            let local = ((tri >> (corner * 8)) & 255) as usize;
                            let Some(v) = scene.cluster_verts.get(c.first_vert as usize + local).and_then(|i| scene.vertices.get(*i as usize)) else { continue; };
                            let p = Vec3::from(v.pos);
                            if p.is_finite() { lo = lo.min(p); hi = hi.max(p); }
                        }
                    }
                }
                let half = (hi - lo) * 0.5;
                // A flat source sheet cannot define a volumetric crown. Keep
                // the legacy fallback for unsupported/untagged foliage only.
                if !half.is_finite() || half.min_element() <= 0.001 { return 0; }
                let centre = (hi + lo) * 0.5;
                let id = result.bounds.len() as u32;
                result.bounds.push(CrownBounds { centre: [centre.x, centre.y, centre.z, 1.0], half: [half.x, half.y, half.z, 0.0] });
                id
            });
            result.ids.push(id);
        }
        result
    }

    fn packed(&self) -> Vec<u32> {
        let mut data = self.ids.clone();
        data.extend_from_slice(bytemuck::cast_slice(&self.bounds));
        data
    }

    fn crown(&self, index: usize, instance: &GpuInstance) -> (Vec3, Vec3) {
        let id = self.ids.get(index).copied().unwrap_or(0) as usize;
        if id == 0 { return legacy_crown(instance); }
        let local = self.bounds[id]; let p = Vec3::from_slice(&local.centre); let r = Vec3::from_slice(&local.half);
        let centre = Vec3::from_array(std::array::from_fn(|k| Vec3::from_slice(&instance.rows[k]).dot(p) + instance.rows[k][3]));
        let radii = Vec3::from_array(std::array::from_fn(|k| Vec3::from_slice(&instance.rows[k]).abs().dot(r)));
        (centre, radii)
    }

    fn proxy(&self, index: usize, instance: &GpuInstance, source_bounds: bool) -> (Vec3, Vec3) {
        if source_bounds { self.crown(index, instance) } else { legacy_crown(instance) }
    }
}
fn crown_mass(r: Vec3) -> f64 {
    (8.0 * std::f64::consts::PI / 15.0) * f64::from(r.x) * f64::from(r.y) * f64::from(r.z)
}
fn footprint_bound(r: Vec3, pitch: f32) -> u64 {
    [r.x, r.y, r.z].into_iter().map(|v| ((2.0 * v / pitch).ceil() as u64) + 2).product()
}

fn choose_layout(receivers: &[Vec3], foliage_y: [f32; 2], previous_pitch: f32) -> Layout {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for &p in receivers.iter().filter(|p| p.is_finite()) { lo = lo.min(p); hi = hi.max(p); }
    if !lo.is_finite() { lo = Vec3::ZERO; hi = Vec3::ZERO; }
    let centre = lo * 0.5 + hi * 0.5;
    let low_y = lo.y.min(foliage_y[0]);
    let high_y = hi.y.max(foliage_y[1]);
    let coarse_centre = Vec3::new(centre.x, low_y * 0.5 + high_y * 0.5, centre.z);
    let half = (hi - lo) * 0.5;
    // All desired receivers and their light rays remain in the full-weight
    // region, even after the storage window has moved by half a snap step.
    let usable = Vec3::new(64.0, 24.0, 64.0)
        - Vec3::splat(CANOPY_SUPPORT_GUARD_CELLS + CANOPY_FADE_CELLS);
    let needed = Vec3::new(half.x + RENDER_REACH + RAY_GUARD,
        (high_y - low_y) * 0.5 + RAY_GUARD, half.z + RENDER_REACH + RAY_GUARD);
    let required = (needed / usable).max_element();
    let mut coarse_pitch = previous_pitch.max(24.0);
    while coarse_pitch < required { coarse_pitch *= 2.0; }
    let mut origins = [[0.0; 4]; 3];
    let mut anchors = origins;
    let half_dims = Vec3::new(64.0, 24.0, 64.0);
    for (level, pitch) in [1.5, 6.0, coarse_pitch].into_iter().enumerate() {
        let anchor = if level == 2 { coarse_centre } else { centre };
        let desired = anchor - half_dims * pitch;
        let origin = (desired / (CANOPY_SNAP_CELLS * pitch)).round() * (CANOPY_SNAP_CELLS * pitch);
        origins[level] = [origin.x, origin.y, origin.z, pitch];
        anchors[level] = [anchor.x, anchor.y, anchor.z, pitch];
    }
    Layout { origins, anchors }
}

pub(crate) struct ProxyBudget {
    crowns: Arc<Vec<Option<[f64;2]>>>, heights: Arc<Vec<Option<[f32;2]>>>, y: [f32;2], scale: f32,
}

impl CanopyClipmap {
    pub fn new(gpu: &Gpu, scene: &Scene, scene_layout: &wgpu::BindGroupLayout) -> Result<Self, String> {
        let source_bounds = std::env::var("KERN_SOURCE_CROWN_BOUNDS").as_deref()==Ok("1");
        let compact_support = std::env::var("KERN_CANOPY_COMPACT_SUPPORT").as_deref()==Ok("1");
        let source_crowns = Arc::new(if source_bounds || compact_support {
            SourceCrowns::new(scene)
        } else { SourceCrowns {ids:Vec::new(),bounds:vec![CrownBounds::default()]} });
        let mut initial_crowns = vec![None; scene.instances.len()];
        let mut initial_heights = vec![None; scene.instances.len()];
        let mut foliage_y = [f32::INFINITY, f32::NEG_INFINITY];
        let mut total_mass = 0.0;
        let mut count = 0usize;
        for (index, instance) in scene.instances.iter().enumerate() {
            if instance.flags & FOLIAGE == 0 { continue; }
            let (centre, radii) = source_crowns.proxy(index, instance, source_bounds);
            if !centre.is_finite() || !radii.is_finite() || radii.min_element() <= 0.0
                || centre.abs().max_element() > 1_000_000.0 {
                return Err(format!("canopy instance {index} has invalid or unsupported world bounds"));
            }
            if footprint_bound(radii, 1.5) > MAX_CROWN_CELLS {
                return Err(format!("canopy instance {index} exceeds {MAX_CROWN_CELLS} finest-cell work limit"));
            }
            initial_crowns[index] = Some([f64::from(instance.sphere[3]), crown_mass(radii)]);
            foliage_y[0] = foliage_y[0].min(centre.y - radii.y);
            foliage_y[1] = foliage_y[1].max(centre.y + radii.y);
            if compact_support {
                let (c, r) = source_crowns.crown(index, instance);
                foliage_y[0] = foliage_y[0].min(c.y-r.y);
                foliage_y[1] = foliage_y[1].max(c.y+r.y);
            }
            let (c,r)=if compact_support {source_crowns.crown(index,instance)}else{(centre,radii)};
            initial_heights[index]=Some([(centre.y-radii.y).min(c.y-r.y),(centre.y+radii.y).max(c.y+r.y)]);
            total_mass += crown_mass(radii);
            count += 1;
        }
        if count > MAX_FOLIAGE { return Err(format!("canopy has {count} crowns; maximum is {MAX_FOLIAGE}")); }
        if count == 0 { foliage_y = [0.0, 0.0]; }
        // Every cell receives at most the scene's total crown mass. Leave a
        // factor-two margin for float/rounding error and one integer per tree.
        // Integrated mass quantization remains useful even in huge far cells.
        let budget = (u32::MAX as f64 * 0.5 - count as f64) / total_mass.max(1.0);
        let mass_scale = 2.0f64.powf(budget.log2().floor()).min(65_536.0) as f32;
        if !mass_scale.is_finite() || mass_scale <= 0.0 {
            return Err("canopy fixed-point mass scale is not representable".into());
        }
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canopy clipmap fill"), entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canopy clipmap"), bind_group_layouts: &[Some(scene_layout), Some(&layout)], immediate_size: 0,
        });
        let pipeline = |label, source| {
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source) });
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label), layout: Some(&pipeline_layout), module: &module, entry_point: Some("main"),
                compilation_options: Default::default(), cache: gpu.pipeline_cache(),
            })
        };
        let fill = pipeline("canopy mass fill", include_str!("shaders/canopy_fill.wgsl").into());
        let filter = pipeline("canopy mass to density", include_str!("shaders/canopy_filter.wgsl").into());
        let source_buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared source crown bounds"), contents: bytemuck::cast_slice(&source_crowns.packed()), usage: wgpu::BufferUsages::STORAGE,
        });
        let resources = view_resources(gpu, &layout, &source_buffer, compact_support);
        Ok(Self { texture: resources.texture, view: resources.view, scratch: resources.scratch, atomic: resources.atomic,
            uniform: resources.uniform, bind: resources.bind, bind_layout: layout,
            fill, filter, cache: Mutex::new(Cache::default()),
            initial_crowns: Arc::new(initial_crowns), initial_heights: Arc::new(initial_heights), foliage_y, mass_scale, source_crowns, source_buffer,
            compact_support, source_bounds })
    }

    /// Create an independent persistent view without reloading scene data or
    /// recompiling pipelines. A view's receiver slice must contain only that
    /// camera's position, target and lookahead, never the other players.
    /// Cache state starts empty: no inherited union/spread-dependent pitch.
    pub fn fork_view(&self, gpu: &Gpu) -> Self {
        let resources = view_resources(gpu, &self.bind_layout, &self.source_buffer, self.compact_support);
        Self { texture: resources.texture, view: resources.view, scratch: resources.scratch, atomic: resources.atomic,
            uniform: resources.uniform, bind: resources.bind, bind_layout: self.bind_layout.clone(),
            fill: self.fill.clone(), filter: self.filter.clone(), cache: Mutex::new(Cache::default()),
            initial_crowns: Arc::clone(&self.initial_crowns), initial_heights: Arc::clone(&self.initial_heights), foliage_y: self.foliage_y, mass_scale: self.mass_scale,
            source_crowns: Arc::clone(&self.source_crowns), source_buffer: self.source_buffer.clone(),
            compact_support: self.compact_support, source_bounds: self.source_bounds }
    }

    /// Validate a geometry-free proxy replacement and recompute the fixed-point
    /// mass budget. GPU texture/pipelines remain resident; every view shares
    /// the new metadata. Do not bypass overflow checks for streamed vegetation.
    pub(crate) fn prepare_proxy_update(&self, first: usize, instances: &[GpuInstance]) -> Result<ProxyBudget, String> {
        self.prepare_proxy_edits(instances.iter().enumerate().map(|(i,s)|(first+i,s)))
    }
    pub(crate) fn prepare_proxy_edits<'a>(&self, edits:impl IntoIterator<Item=(usize,&'a GpuInstance)>)->Result<ProxyBudget,String> {
        let mut crowns=(*self.initial_crowns).clone();
        let mut heights=(*self.initial_heights).clone();
        for (i,instance) in edits {
            if i>=crowns.len(){return Err("canopy proxy range exceeds reserved scene".into());}
            if instance.cluster_count!=0 || self.source_crowns.ids.get(i).copied().unwrap_or(0)!=0 {return Err("canopy relocation only supports geometry-free proxies".into());}
            crowns[i]=None;heights[i]=None;
            if instance.flags & FOLIAGE == 0 {continue;}
            let (c,r)=legacy_crown(instance);
            if !c.is_finite() || !r.is_finite() || r.min_element()<=0.0 || c.abs().max_element()>1_000_000.0 ||
                !instance.scale.is_finite() || instance.scale<=0.0 || footprint_bound(r,1.5)>MAX_CROWN_CELLS {
                return Err("streamed canopy exceeds finite world/work bounds".into());
            }
            crowns[i]=Some([instance.sphere[3] as f64,crown_mass(r)]);
            heights[i]=Some([c.y-r.y,c.y+r.y]);
        }
        let count=crowns.iter().flatten().count();
        if count>MAX_FOLIAGE {return Err("streamed canopy exceeds crown budget".into());}
        let mass=crowns.iter().flatten().map(|v|v[1]).sum::<f64>();
        let budget=(u32::MAX as f64*0.5-count as f64)/mass.max(1.0);
        let scale=2.0f64.powf(budget.log2().floor()).min(65_536.0) as f32;
        if !scale.is_finite() || scale<=0.0 {return Err("streamed canopy mass budget is not representable".into());}
        let mut y=[f32::INFINITY,f32::NEG_INFINITY];
        for h in heights.iter().flatten() {y[0]=y[0].min(h[0]);y[1]=y[1].max(h[1]);}
        if count==0 {y=[0.0,0.0];}
        Ok(ProxyBudget {crowns:Arc::new(crowns),heights:Arc::new(heights),y,scale})
    }
    pub(crate) fn commit_proxy_update(&mut self, budget: &ProxyBudget) {
        self.initial_crowns=Arc::clone(&budget.crowns);
        self.initial_heights=Arc::clone(&budget.heights);
        self.foliage_y=budget.y;self.mass_scale=budget.scale;
        *self.cache.lock().unwrap()=Cache::default();
    }

    /// Call before changing the shared instance buffer. Rigid motion is valid;
    /// new crowns, rescaling, or leaving the initial height envelope require a
    /// renderer rebuild so integer mass bounds remain checked, never wrapped.
    pub fn validate_update(&self, first: usize, instances: &[GpuInstance]) -> Result<(), String> {
        for (offset, instance) in instances.iter().enumerate() {
            if instance.flags & FOLIAGE == 0 { continue; }
            let index = first.checked_add(offset).ok_or("canopy update index overflow")?;
            let [original, original_mass] = self.initial_crowns.get(index).copied().flatten()
                .ok_or_else(|| format!("new foliage at instance {index} requires renderer rebuild"))?;
            let original = original as f32;
            // Rotation matrix normalization may perturb a bounding radius by
            // a few ulps. The fixed-point budget has a factor-two margin;
            // this tolerance does not permit an actual art/geometry rescale.
            let radius_tolerance = original.abs().max(1.0) * 1e-5;
            if !instance.sphere[3].is_finite() || (instance.sphere[3] - original).abs() > radius_tolerance {
                return Err(format!("rescaling foliage instance {index} requires renderer rebuild"));
            }
            let (centre, radii) = self.source_crowns.proxy(index, instance, self.source_bounds);
            // Preserve the original fixed-point precision. The existing 2x
            // headroom covers ordinary rigid envelope changes; an extreme
            // rotated/squeezed envelope must rebuild, never overflow atomics.
            if crown_mass(radii) > original_mass * 2.0 {
                return Err(format!("foliage instance {index} exceeded its validated mass envelope; rebuild renderer"));
            }
            if !centre.is_finite() || !radii.is_finite() || radii.min_element() <= 0.0
                || centre.abs().max_element() > 1_000_000.0
                || instance.rows.iter().flatten().any(|v| !v.is_finite())
                || !instance.scale.is_finite() || instance.scale <= 0.0
                || footprint_bound(radii, 1.5) > MAX_CROWN_CELLS
                || centre.y - radii.y < self.foliage_y[0] - radius_tolerance
                || centre.y + radii.y > self.foliage_y[1] + radius_tolerance {
                return Err(format!("foliage instance {index} left its validated world/height envelope; rebuild renderer"));
            }
            if self.compact_support {
                let (c, r) = self.source_crowns.crown(index, instance);
                if !c.is_finite() || !r.is_finite() || c.y-r.y < self.foliage_y[0]-radius_tolerance
                    || c.y+r.y > self.foliage_y[1]+radius_tolerance {
                    return Err(format!("foliage instance {index} left its source support envelope; rebuild renderer"));
                }
            }
        }
        Ok(())
    }

    pub fn needs_update(&self, receivers: &[Vec3], foliage_revision: u64) -> bool {
        let cache = self.cache.lock().unwrap();
        let layout = choose_layout(receivers, self.foliage_y, cache.coarse_pitch);
        cache.origins != Some(layout.origins) || cache.revision != foliage_revision
    }

    /// Emits two timestamp intervals only when a level actually changes.
    /// The caller must use `needs_update` before reserving its query indices.
    pub fn prepare(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup, instance_count: u32, receivers: &[Vec3], foliage_revision: u64,
        timestamps: [Option<(&wgpu::QuerySet, u32)>; 2]) -> CanopyFrame {
        let mut cache = self.cache.lock().unwrap();
        let layout = choose_layout(receivers, self.foliage_y, cache.coarse_pitch);
        let mut dirty = 0u32;
        for level in 0..3 {
            if cache.revision != foliage_revision || cache.origins.is_none_or(|o| o[level] != layout.origins[level]) {
                dirty |= 1 << level;
            }
        }
        if dirty != 0 {
            let fill_uniform = FillUniform { origins: layout.origins, dims: [128, 48, 128, 0],
                mass: [self.mass_scale, 1.0 / self.mass_scale, 0.0, 0.0], state: [dirty, instance_count, self.source_crowns.ids.len() as u32,
                    self.source_bounds as u32 | ((self.compact_support as u32)<<1)] };
            gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&fill_uniform));
            for level in 0..3 {
                let bytes = COUNT * 4 * if self.compact_support { 3 } else { 1 };
                if dirty & (1 << level) != 0 { encoder.clear_buffer(&self.atomic, level as u64 * bytes, Some(bytes)); }
            }
            for (index, pipeline) in [&self.fill, &self.filter].into_iter().enumerate() {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(if index == 0 { "canopy clipmap fill" } else { "canopy clipmap filter" }),
                    timestamp_writes: timestamps[index].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                        query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1),
                    }),
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, scene_bind, &[]);
                pass.set_bind_group(1, &self.bind, &[]);
                if index == 0 {
                    let groups = instance_count.div_ceil(64).max(1);
                    pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
                } else { pass.dispatch_workgroups(16, 36, 16); }
            }
            cache.origins = Some(layout.origins);
            cache.revision = foliage_revision;
            cache.coarse_pitch = layout.origins[2][3];
        }
        CanopyFrame { view: self.view.clone(), texture: self.texture.clone(), scratch: self.scratch.clone(), origins: layout.origins,
            anchors: layout.anchors, dims: CANOPY_DIMS, updated_levels: dirty, compact_support: self.compact_support }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_scene() -> Scene {
        use crate::cluster::{ClusterMesh, Part, Vertex};
        use crate::scene::Material;
        let mut scene = Scene::default();
        for flags in [0, ALPHA_TEST] {
            scene.materials.push(Material { tint: [1.0; 3], layer: -1, roughness: 1.0, flags, scale: 1.0, metal: 0.0 });
        }
        let box_vertices = |lo:Vec3,hi:Vec3| -> Vec<Vertex> {
            (0..8).map(|i| Vertex { pos:std::array::from_fn(|a| if i&(1<<a)==0 {lo[a]} else {hi[a]}), normal:[0.0,1.0,0.0], uv:[0.0;2] }).collect()
        };
        let indices = [0,2,1,1,2,3,4,5,6,5,7,6,0,1,4,1,5,4,2,6,3,3,6,7,0,4,2,2,4,6,1,3,5,3,7,5];
        let trunk = box_vertices(Vec3::new(-0.3,0.0,-0.3),Vec3::new(0.3,18.0,0.3));
        let leaves = box_vertices(Vec3::new(-2.0,7.0,-3.0),Vec3::new(2.0,16.0,3.0));
        let mesh = ClusterMesh::build(&[Part {vertices:&trunk,indices:&indices,material:0}, Part {vertices:&leaves,indices:&indices,material:1}]);
        let id = scene.add_mesh(&mesh);
        for transform in [glam::Affine3A::IDENTITY, glam::Affine3A::from_scale_rotation_translation(Vec3::new(2.0,0.5,1.0),glam::Quat::from_rotation_y(0.73),Vec3::new(-7.0,11.0,4.0))] {
            scene.add_instance(id,transform);scene.mark_foliage();
        }
        scene
    }

    #[test]
    fn source_leaf_bounds_exclude_stem_and_share_mesh_data() {
        let scene = source_scene(); let source = SourceCrowns::new(&scene);
        assert_eq!(source.bounds.len(),2); assert_eq!(source.ids,vec![1,1]);
        let (c,r) = source.crown(0,&scene.instances[0]);
        assert!(c.abs_diff_eq(Vec3::new(0.0,11.5,0.0),1e-6));
        assert!(r.abs_diff_eq(Vec3::new(2.0,4.5,3.0),1e-6));
        assert_eq!(c.y-r.y,7.0,"the bare stem must not become a low canopy");
        assert_eq!(source.packed().len()*4,2*4+2*32);
    }

    #[test]
    fn source_crown_envelope_follows_actual_nonuniform_transform() {
        let scene = source_scene(); let source = SourceCrowns::new(&scene);
        let inst = &scene.instances[1]; let (c,r) = source.crown(1,inst);
        let xf = glam::Affine3A::from_scale_rotation_translation(Vec3::new(2.0,0.5,1.0),glam::Quat::from_rotation_y(0.73),Vec3::new(-7.0,11.0,4.0));
        let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for x in [-2.0,2.0] {for y in [7.0,16.0] {for z in [-3.0,3.0] {
            let p = xf.transform_point3(Vec3::new(x,y,z)); lo=lo.min(p);hi=hi.max(p);
        }}}
        assert!((c-r).abs_diff_eq(lo,2e-6)); assert!((c+r).abs_diff_eq(hi,2e-6));
        assert_eq!(r.y,2.25,"vertical scale must not be replaced by largest axis scale");
        assert!(crown_mass(r)<=crown_mass(Vec3::splat(inst.sphere[3])));
    }

    #[test]
    fn untagged_opaque_foliage_keeps_legacy_fallback() {
        let mut scene = source_scene(); scene.materials[1].flags=0;
        let source = SourceCrowns::new(&scene); assert_eq!(source.ids,vec![0,0]);
        assert_eq!(source.crown(0,&scene.instances[0]),legacy_crown(&scene.instances[0]));
    }

    #[test]
    fn storage_and_uniform_contract_are_bounded() {
        assert_eq!(std::mem::size_of::<FillUniform>(), 96);
        assert_eq!(COUNT * 3 * (4 + 8), 27 * 1024 * 1024);
        assert_eq!(4 * COUNT * 3 * (4 + 8), 108 * 1024 * 1024);
        assert_eq!(std::mem::offset_of!(FillUniform, state), 80);
    }

    #[test]
    fn continuous_centres_do_not_jump_when_storage_recentres() {
        let a = choose_layout(&[Vec3::new(5.999, 0.0, 0.0)], [-10.0, 100.0], 24.0);
        let b = choose_layout(&[Vec3::new(6.001, 0.0, 0.0)], [-10.0, 100.0], 24.0);
        assert_ne!(a.origins[0], b.origins[0]);
        assert!((b.anchors[0][0] - a.anchors[0][0] - 0.002).abs() < 1e-5);
        for layout in [a, b] { for level in 0..3 {
            let p = layout.origins[level][3];
            let origin = Vec3::from_slice(&layout.origins[level]);
            let anchor = Vec3::from_slice(&layout.anchors[level]);
            let snapped_centre = origin + Vec3::new(64.0, 24.0, 64.0) * p;
            assert!((anchor - snapped_centre).abs().max_element() <= 4.0 * p + 1e-3);
        }}
    }

    #[test]
    fn far_level_covers_spread_players_and_high_hills_with_full_weight() {
        let cameras = [Vec3::new(-1800.0, -20.0, -1200.0), Vec3::new(1800.0, 400.0, 1200.0)];
        let layout = choose_layout(&cameras, [-100.0, 1000.0], 24.0);
        let pitch = layout.origins[2][3];
        let anchor = Vec3::from_slice(&layout.anchors[2]);
        let full = Vec3::new(51.0, 11.0, 51.0) * pitch;
        for p in cameras {
            let d = (p - anchor).abs();
            assert!(d.x + RENDER_REACH + RAY_GUARD <= full.x);
            assert!(d.z + RENDER_REACH + RAY_GUARD <= full.z);
        }
        assert!(1000.0 - anchor.y + RAY_GUARD <= full.y);
        assert!(anchor.y + 100.0 + RAY_GUARD <= full.y);
        let reunited = choose_layout(&[Vec3::ZERO], [-100.0, 1000.0], pitch);
        assert_eq!(reunited.origins[2][3], pitch);
    }

    #[test]
    fn kernels_have_known_mass_and_scale_cubically() {
        let r = Vec3::new(2.0, 5.0, 3.0);
        assert!((crown_mass(r) - 16.0 * std::f64::consts::PI).abs() < 1e-12);
        assert!((crown_mass(r * 2.0) / crown_mass(r) - 8.0).abs() < 1e-12);
    }

    // Independent f64 quadrature oracle for the cell-average construction.
    // It has no resident-grid loop or shader index arithmetic. Translation
    // exercises centre sampling versus overlap integration, including a
    // complete small crown wholly between a coarse cell's sample centres.
    fn numeric_cells(centre: [f64; 3], radii: [f64; 3], pitch: f64) -> Vec<f64> {
        let mut cells = Vec::new();
        let low: [i32; 3] = std::array::from_fn(|a| ((centre[a] - radii[a]) / pitch).floor() as i32);
        let high: [i32; 3] = std::array::from_fn(|a| ((centre[a] + radii[a]) / pitch).floor() as i32);
        let fine = pitch <= radii.into_iter().fold(f64::INFINITY, f64::min) * 0.5;
        let nodes = [-(3.0f64 / 5.0).sqrt(), 0.0, (3.0f64 / 5.0).sqrt()];
        let weights = [5.0 / 9.0, 8.0 / 9.0, 5.0 / 9.0];
        for z in low[2]..=high[2] { for y in low[1]..=high[1] { for x in low[0]..=high[0] {
            let base = [x, y, z].map(|i| f64::from(i) * pitch);
            let sample = |p: [f64; 3]| (1.0 - (0..3).map(|a| ((p[a] - centre[a]) / radii[a]).powi(2)).sum::<f64>()).max(0.0);
            if fine { cells.push(sample(base.map(|v| v + pitch * 0.5)) * pitch.powi(3)); continue; }
            let lo: [f64; 3] = std::array::from_fn(|a| base[a].max(centre[a] - radii[a]));
            let hi: [f64; 3] = std::array::from_fn(|a| (base[a] + pitch).min(centre[a] + radii[a]));
            let half: [f64; 3] = std::array::from_fn(|a| (hi[a] - lo[a]).max(0.0) * 0.5);
            let middle: [f64; 3] = std::array::from_fn(|a| (hi[a] + lo[a]) * 0.5);
            let mut integral = 0.0;
            for k in 0..27 {
                let n = [k % 3, k / 3 % 3, k / 9];
                let p = std::array::from_fn(|a| middle[a] + half[a] * nodes[n[a]]);
                integral += sample(p) * n.map(|i| weights[i]).into_iter().product::<f64>();
            }
            cells.push(integral * half.into_iter().product::<f64>());
        }}}
        cells
    }

    #[test]
    fn translated_small_and_large_crowns_preserve_mass_on_all_grids() {
        for radii in [[0.3, 0.7, 0.4], [3.0, 5.0, 3.5]] {
            let exact = 8.0 * std::f64::consts::PI / 15.0 * radii.into_iter().product::<f64>();
            for pitch in [1.5, 6.0, 24.0, 96.0] {
                for phase in [-1.000001, -1.0, -0.999999, -0.5, 0.0, 0.499999, 0.5, 0.500001, 0.999999] {
                    let cells = numeric_cells([phase * pitch, 0.37 * pitch, -0.23 * pitch], radii, pitch);
                    let integral: f64 = cells.iter().sum();
                    assert!(integral > 0.0, "coarse centre sampling lost a whole crown");
                    assert!((integral / exact - 1.0).abs() < 0.15, "bad overlap quadrature: {integral} / {exact}");
                    // The actual scheme preserves analytic mass before window
                    // clipping. Integer deposition's remaining error is at
                    // most half a quantum for every nonzero cell.
                    let encoded: f64 = cells.iter().map(|w| (w * exact / integral * 128.0).round()).sum();
                    let rounding_bound = cells.iter().filter(|w| **w > 0.0).count() as f64 * 0.5 / 128.0;
                    assert!((encoded / 128.0 - exact).abs() <= rounding_bound + 1e-10);
                    assert!(encoded > 0.0);
                    let near_density_integral: f64 = cells.iter().map(|w| (w * exact / integral / pitch.powi(3)) * pitch.powi(3)).sum();
                    assert!((near_density_integral - exact).abs() < exact * 1e-12);
                }
            }
        }
    }

    fn compact_kernel(u:f64,lo:f64,hi:f64,width:f64)->f64 {
        let smooth = |t:f64| {let t=t.clamp(0.0,1.0);t*t*(3.0-2.0*t)};
        let lower=if lo<=-1.0 {1.0} else {smooth((u-lo)/width)};
        let upper=if hi>=1.0 {1.0} else {smooth((hi-u)/width)};
        (1.0-u.abs()).max(0.0)*lower*upper
    }

    fn compact_integral(lo:f64,hi:f64,width:f64)->f64 {
        let cuts=[lo,lo+width,0.0f64.clamp(lo+width,hi-width),hi-width,hi];
        let nodes=[-(3.0f64/5.0).sqrt(),0.0,(3.0f64/5.0).sqrt()];
        let weights=[5.0/9.0,8.0/9.0,5.0/9.0];
        let mut integral=0.0;
        for pair in cuts.windows(2) {
            let at=0.0f64.clamp(pair[0],pair[1]);
            for (a,b) in [(pair[0],at),(at,pair[1])] {
                let middle=(a+b)*0.5;let half=(b-a)*0.5;
                for i in 0..3 {integral+=half*weights[i]*compact_kernel(middle+half*nodes[i],lo,hi,width);}
            }
        }
        integral
    }

    fn compact_cell(height:f64,centre:f64,pitch:f64,support:[f64;2],mass:f64)->f64 {
        let lo=(((support[0]-centre)/pitch).clamp(-1.0,1.0)*1024.0).ceil()/1024.0;
        let hi=(((support[1]-centre)/pitch).clamp(-1.0,1.0)*1024.0).floor()/1024.0;
        if hi<=lo {return 0.0;}
        let width=(0.25*(hi-lo)).min(0.5);
        let integral=compact_integral(lo,hi,width);
        mass/pitch.powi(3)/integral*compact_kernel((height-centre)/pitch,lo,hi,width)
    }

    #[test]
    fn compact_tent_normalization_matches_independent_numeric_integration() {
        assert_eq!(compact_integral(-1.0,1.0,0.5),1.0,"unclipped cells retain the original tent");
        // Independent midpoint quadrature catches the tent kink occurring
        // inside either taper, not only inside the central polynomial piece.
        for (lo,hi) in [(-1.0,1.0),(-0.9,-0.02),(-0.01,0.02),(-0.01,0.95),(-0.95,0.01),(0.02,0.9)] {
            let width=(0.25f64*(hi-lo)).min(0.5);
            let exact=compact_integral(lo,hi,width);
            let n=40_000;let step=(hi-lo)/n as f64;
            let numeric=(0..n).map(|i| compact_kernel(lo+(i as f64+0.5)*step,lo,hi,width)*step).sum::<f64>();
            assert!(exact.is_finite() && exact>0.0);
            assert!((numeric/exact-1.0).abs()<1e-8,"{lo}..{hi}: {numeric} vs {exact}");
        }
    }

    #[test]
    fn compact_source_support_keeps_air_clear_without_losing_cell_mass() {
        let scene=source_scene();let sources=SourceCrowns::new(&scene);
        let (centre,radius)=sources.crown(0,&scene.instances[0]);
        let support=[f64::from(centre.y-radius.y),f64::from(centre.y+radius.y)];
        assert_eq!(support,[7.0,16.0]);
        let (pitch,cell_centre,mass)=(24.0f64,12.0f64,200.0f64);
        // The old tent is nonzero at20m: FOUR metres above all source leaves.
        assert!((1.0f64-(20.0-cell_centre).abs()/pitch).max(0.0)>0.0);
        for height in [6.99,16.001,20.0,35.9] {
            assert_eq!(compact_cell(height,cell_centre,pitch,support,mass),0.0);
        }
        let n=40_000;let step=2.0*pitch/n as f64;
        let integrated=(0..n).map(|i| compact_cell(cell_centre-pitch+(i as f64+0.5)*step,cell_centre,pitch,support,mass)*step).sum::<f64>()*pitch* pitch;
        // X/Z tent integrals are each pitch. Y is numerically integrated.
        assert!((integrated/mass-1.0).abs()<1e-7,"{integrated} != {mass}");
    }

    #[test]
    fn compact_overlap_and_lod_blend_preserve_zero_above_source_union() {
        // Conservatively union two overlapping crowns in a coarse cell. We
        // intentionally do not claim to recover gaps within that union.
        let supports=[[7.0f64,16.0],[11.0,20.0]];
        let union=[supports[0][0].min(supports[1][0]),supports[0][1].max(supports[1][1])];
        for height in [20.01,21.0,35.0] {
            let levels=[(1.5,19.5),(6.0,21.0),(24.0,12.0)].map(|(p,c)|compact_cell(height,c,p,union,300.0));
            for weight in [0.0,0.1,0.5,0.9,1.0] {
                assert_eq!(levels[0]*weight+levels[1]*(1.0-weight),0.0);
                assert_eq!(levels[1]*weight+levels[2]*(1.0-weight),0.0);
            }
        }
        for pitch in [1.5,6.0,24.0] {
            let centre=(13.0f64/pitch).floor()*pitch+pitch*0.5;
            let n=40_000;let step=2.0*pitch/n as f64;
            let integral=(0..n).map(|i| compact_cell(centre-pitch+(i as f64+0.5)*step,centre,pitch,union,300.0)*step).sum::<f64>()*pitch*pitch;
            assert!((integral/300.0-1.0).abs()<1e-7);
        }
    }

    #[test]
    fn compact_support_does_not_silently_enable_source_proxy_mass_change() {
        let scene=source_scene();let source=SourceCrowns::new(&scene);
        for (index,instance) in scene.instances.iter().enumerate() {
            assert_eq!(source.proxy(index,instance,false),legacy_crown(instance));
            assert_eq!(source.proxy(index,instance,true),source.crown(index,instance));
        }
        let ordered=|v:f32| {let b=v.to_bits();if b&0x80000000!=0 {!b} else {b^0x80000000}};
        let decode=|k:u32| f32::from_bits(if k&0x80000000!=0 {k^0x80000000} else {!k});
        let heights=[-1000.0f32,-12.3,-0.01,0.0,0.01,12.3,1000.0];
        for pair in heights.windows(2) {assert!(ordered(pair[0])<ordered(pair[1]));}
        let min_key=heights.iter().fold(0u32,|key,&y|key.max(ordered(-y)));
        let max_key=heights.iter().fold(0u32,|key,&y|key.max(ordered(y)));
        assert_eq!(-decode(min_key),-1000.0);assert_eq!(decode(max_key),1000.0);
    }

    #[test]
    fn canopy_shaders_validate() {
        for source in [include_str!("shaders/canopy_fill.wgsl"), include_str!("shaders/canopy_filter.wgsl")] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        }
    }
}
