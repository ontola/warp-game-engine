//! Persistent world-anchored canopy extinction, shared by sun and sky queries.
//!
//! This is a rest-pose ellipsoid approximation, not source-needle voxel data.
//! Extinction adds between crowns at EVERY level (the old field used max).
//! Structural tree changes invalidate the field; a future wind phase must not
//! be treated as a structural revision or trigger per-leaf rebuilds.
//! Each view owns its receiver window so other players cannot change its LOD
//! blend or lighting. Three levels cost 27 MiB per view, 108 MiB for four.
use crate::{scene::{GpuInstance, Scene, FOLIAGE}, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::sync::{Arc, Mutex};

pub const CANOPY_DIMS: [u32; 3] = [128, 48, 128];
pub const CANOPY_LEVELS: usize = 3;
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
    /// xyz is storage lower corner, w is metres per voxel.
    pub origins: [[f32; 4]; 3],
    /// Continuous centres for blending, independent of snapped storage.
    pub anchors: [[f32; 4]; 3],
    pub dims: [u32; 3],
    /// Bit mask of levels rebuilt this frame; zero means an actual cache hit.
    pub updated_levels: u32,
}

pub struct CanopyClipmap {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    atomic: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    fill: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    cache: Mutex<Cache>,
    initial_radii: Arc<Vec<Option<f32>>>,
    foliage_y: [f32; 2],
    mass_scale: f32,
}

/// Per-view resident data: 27 MiB, bounded at 108 MiB for four views.
/// Pipelines and scene validation metadata are shared by `fork_view`.
struct ViewResources {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    atomic: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

fn view_resources(gpu: &Gpu, layout: &wgpu::BindGroupLayout) -> ViewResources {
    let atomic = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("canopy clipmap mass"), size: COUNT * 3 * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("canopy clipmap extinction"),
        size: wgpu::Extent3d { width: 128, height: 48 * 3, depth_or_array_layers: 128 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
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
        ],
    });
    ViewResources { texture, view, atomic, uniform, bind }
}

fn crown(instance: &GpuInstance) -> (Vec3, Vec3) {
    let r = instance.sphere[3];
    (Vec3::new(instance.sphere[0], instance.sphere[1] + r * 0.25, instance.sphere[2]),
        Vec3::new(r * 0.38, r * 0.62, r * 0.38))
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

impl CanopyClipmap {
    pub fn new(gpu: &Gpu, scene: &Scene, scene_layout: &wgpu::BindGroupLayout) -> Result<Self, String> {
        let mut initial_radii = vec![None; scene.instances.len()];
        let mut foliage_y = [f32::INFINITY, f32::NEG_INFINITY];
        let mut total_mass = 0.0;
        let mut count = 0usize;
        for (index, instance) in scene.instances.iter().enumerate() {
            if instance.flags & FOLIAGE == 0 { continue; }
            let (centre, radii) = crown(instance);
            if !centre.is_finite() || !radii.is_finite() || radii.min_element() <= 0.0
                || centre.abs().max_element() > 1_000_000.0 {
                return Err(format!("canopy instance {index} has invalid or unsupported world bounds"));
            }
            if footprint_bound(radii, 1.5) > MAX_CROWN_CELLS {
                return Err(format!("canopy instance {index} exceeds {MAX_CROWN_CELLS} finest-cell work limit"));
            }
            initial_radii[index] = Some(instance.sphere[3]);
            foliage_y[0] = foliage_y[0].min(centre.y - radii.y);
            foliage_y[1] = foliage_y[1].max(centre.y + radii.y);
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
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canopy clipmap"), bind_group_layouts: &[Some(scene_layout), Some(&layout)], immediate_size: 0,
        });
        let pipeline = |label, source| {
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source) });
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label), layout: Some(&pipeline_layout), module: &module, entry_point: Some("main"),
                compilation_options: Default::default(), cache: None,
            })
        };
        let fill = pipeline("canopy mass fill", include_str!("shaders/canopy_fill.wgsl").into());
        let filter = pipeline("canopy mass to density", include_str!("shaders/canopy_filter.wgsl").into());
        let resources = view_resources(gpu, &layout);
        Ok(Self { _texture: resources.texture, view: resources.view, atomic: resources.atomic,
            uniform: resources.uniform, bind: resources.bind, bind_layout: layout,
            fill, filter, cache: Mutex::new(Cache::default()),
            initial_radii: Arc::new(initial_radii), foliage_y, mass_scale })
    }

    /// Create an independent persistent view without reloading scene data or
    /// recompiling pipelines. A view's receiver slice must contain only that
    /// camera's position, target and lookahead, never the other players.
    /// Cache state starts empty: no inherited union/spread-dependent pitch.
    pub fn fork_view(&self, gpu: &Gpu) -> Self {
        let resources = view_resources(gpu, &self.bind_layout);
        Self { _texture: resources.texture, view: resources.view, atomic: resources.atomic,
            uniform: resources.uniform, bind: resources.bind, bind_layout: self.bind_layout.clone(),
            fill: self.fill.clone(), filter: self.filter.clone(), cache: Mutex::new(Cache::default()),
            initial_radii: Arc::clone(&self.initial_radii), foliage_y: self.foliage_y, mass_scale: self.mass_scale }
    }

    /// Call before changing the shared instance buffer. Rigid motion is valid;
    /// new crowns, rescaling, or leaving the initial height envelope require a
    /// renderer rebuild so integer mass bounds remain checked, never wrapped.
    pub fn validate_update(&self, first: usize, instances: &[GpuInstance]) -> Result<(), String> {
        for (offset, instance) in instances.iter().enumerate() {
            if instance.flags & FOLIAGE == 0 { continue; }
            let index = first.checked_add(offset).ok_or("canopy update index overflow")?;
            let original = self.initial_radii.get(index).copied().flatten()
                .ok_or_else(|| format!("new foliage at instance {index} requires renderer rebuild"))?;
            // Rotation matrix normalization may perturb a bounding radius by
            // a few ulps. The fixed-point budget has a factor-two margin;
            // this tolerance does not permit an actual art/geometry rescale.
            let radius_tolerance = original.abs().max(1.0) * 1e-5;
            if !instance.sphere[3].is_finite() || (instance.sphere[3] - original).abs() > radius_tolerance {
                return Err(format!("rescaling foliage instance {index} requires renderer rebuild"));
            }
            let (centre, radii) = crown(instance);
            if !centre.is_finite() || centre.abs().max_element() > 1_000_000.0
                || instance.rows.iter().flatten().any(|v| !v.is_finite())
                || !instance.scale.is_finite() || instance.scale <= 0.0
                || footprint_bound(radii, 1.5) > MAX_CROWN_CELLS
                || centre.y - radii.y < self.foliage_y[0] - radius_tolerance
                || centre.y + radii.y > self.foliage_y[1] + radius_tolerance {
                return Err(format!("foliage instance {index} left its validated world/height envelope; rebuild renderer"));
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
                mass: [self.mass_scale, 1.0 / self.mass_scale, 0.0, 0.0], state: [dirty, instance_count, 0, 0] };
            gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&fill_uniform));
            for level in 0..3 {
                if dirty & (1 << level) != 0 { encoder.clear_buffer(&self.atomic, level as u64 * COUNT * 4, Some(COUNT * 4)); }
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
        CanopyFrame { view: self.view.clone(), origins: layout.origins,
            anchors: layout.anchors, dims: CANOPY_DIMS, updated_levels: dirty }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn canopy_shaders_validate() {
        for source in [include_str!("shaders/canopy_fill.wgsl"), include_str!("shaders/canopy_filter.wgsl")] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        }
    }
}
