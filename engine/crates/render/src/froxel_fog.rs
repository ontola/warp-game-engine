//! Froxel fog (Wronski 2014, "Volumetric fog"; Hillaire 2015, Frostbite):
//! a participating medium in a camera-aligned grid of 160x90x64 cells
//! (froxels), slices spaced exponentially from 0.1 m to `FroxelFog::far_m`
//! of view depth.
//!
//! Per frame and view, before shade (`frame.rs`):
//!
//! - `fog_inject` (shaders/froxel_fog.wgsl, compiled with shade so it lights
//!   the air as shade lights surfaces): per cell, at a depth jittered within
//!   the cell each frame, the light the medium scatters towards the camera
//!   (Henyey-Greenstein phase) from every clustered light with its shadow,
//!   cookie and profile, from the sun through its shadow maps, and from the
//!   room's ambient light (the irradiance volume, sky through openings
//!   included), times the scattering coefficient; and the extinction. It is
//!   blended with last frame's cell where the point was (reprojected, 0.9
//!   kept), so the jitter averages out.
//! - `integrate` (below): per column, front to back, the light scattered
//!   in front of each slice and the transmittance to it (Hillaire's energy-
//!   conserving step).
//! - Shade reads the integrated volume trilinearly at each pixel's depth:
//!   the surface times the transmittance, plus the in-scattered light.
//!
//! The game sets the medium (`Lighting::froxel_fog`): density, albedo and
//! anisotropy; no boxes. Glass (drawn after shade) reads the volume at its
//! own depth; reflection probe captures march the medium along their rays.
use crate::Gpu;
use glam::{Mat4, Vec3};

/// Cells per view: x, y, depth slices (keep equal to FOG_GRID).
pub const GRID: [u32; 3] = [160, 90, 64];
/// The first slice's near plane (keep equal to FOG_NEAR).
pub const NEAR: f32 = 0.1;

/// `Lighting::froxel_fog`: a homogeneous medium.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FroxelFog {
    /// Extinction per metre (0.02: a hazy room; 0.1: smoke).
    pub density: f32,
    /// Share of the extinction that scatters, per channel (1: white smoke).
    pub albedo: Vec3,
    /// Henyey-Greenstein g, -0.9..0.9 (0 isotropic, positive: forward).
    pub anisotropy: f32,
    /// How far the grid reaches (view depth, metres); beyond it the last
    /// slice holds.
    pub far_m: f32,
    /// Where the medium is (min, max); `None`: everywhere. A building's
    /// inside keeps the outdoor sky and sun out of its smoke where a froxel
    /// straddles a wall or ceiling.
    pub bounds: Option<(Vec3, Vec3)>,
}

impl Default for FroxelFog {
    fn default() -> Self { Self { density: 0.05, albedo: Vec3::splat(0.9), anisotropy: 0.3, far_m: 48.0, bounds: None } }
}

impl FroxelFog {
    pub fn validated(self) -> Result<Self, String> {
        if !(self.density.is_finite() && self.density >= 0.0) { return Err("froxel fog density must be finite and not negative".into()); }
        if !self.albedo.is_finite() || self.albedo.min_element() < 0.0 { return Err("froxel fog albedo must be finite and not negative".into()); }
        if !(self.anisotropy.abs() < 0.95) { return Err("froxel fog anisotropy must be within -0.95..0.95".into()); }
        if !(self.far_m > NEAR * 2.0 && self.far_m.is_finite()) { return Err("froxel fog far_m must be beyond 0.2 m".into()); }
        Ok(self)
    }
}

const INTEGRATE: &str = "
struct U { tan_half: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var cells: texture_3d<f32>;
@group(0) @binding(2) var out: texture_storage_3d<rgba16float, write>;

// x, y: tan of half the field of view; z: near; w: far.
fn slice_depth(s: f32) -> f32 { return u.tan_half.z * pow(u.tan_half.w / u.tan_half.z, s / f32(textureDimensions(cells).z)); }

@compute @workgroup_size(8, 8)
fn integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(cells);
    if any(id.xy >= dims.xy) { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(dims.xy);
    // Metres along the ray per metre of view depth.
    let along = length(vec3<f32>((uv.x * 2.0 - 1.0) * u.tan_half.x, (1.0 - uv.y * 2.0) * u.tan_half.y, 1.0));
    var light = vec3<f32>(0.0);
    var through = 1.0;
    var w0 = u.tan_half.z;
    for (var z = 0u; z < dims.z; z++) {
        let w1 = slice_depth(f32(z + 1u));
        let c = textureLoad(cells, vec3<u32>(id.xy, z), 0);
        let ds = (w1 - w0) * along;
        let sigma = max(c.a, 1e-7);
        let step = exp(-sigma * ds);
        // Hillaire 2015: the in-scattering integrated over the step.
        light += through * (c.rgb - c.rgb * step) / sigma;
        through *= step;
        textureStore(out, vec3<u32>(id.xy, z), vec4<f32>(light, through));
        w0 = w1;
    }
}
";

struct View {
    cells: [wgpu::Texture; 2],
    integrated: wgpu::Texture,
    uniform: wgpu::Buffer,
    cur: usize,
    prev_vp: Option<Mat4>,
    frame: u32,
}

/// This frame's volumes for one view.
pub(crate) struct Frame {
    pub cells_out: wgpu::TextureView,
    pub cells_prev: wgpu::TextureView,
    pub integrated: wgpu::TextureView,
    pub prev_view_proj: Option<Mat4>,
    pub frame: u32,
    pub integrate_bind: wgpu::BindGroup,
}

pub(crate) struct FroxelVolumes {
    integrate: wgpu::ComputePipeline,
    views: Vec<Option<View>>,
    /// Bound where a view has no fog.
    pub fallback: wgpu::TextureView,
}

fn volume(gpu: &Gpu, label: &str, size: [u32; 3], usage: wgpu::TextureUsages) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] }, mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::Rgba16Float, usage, view_formats: &[] })
}

impl FroxelVolumes {
    pub fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("froxel fog integrate"), source: wgpu::ShaderSource::Wgsl(INTEGRATE.into()) });
        let integrate = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("froxel fog integrate"), layout: None,
            module: &module, entry_point: Some("integrate"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let rw = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        Self { integrate, views: Vec::new(),
            fallback: volume(gpu, "no froxel fog", [1, 1, 1], rw).create_view(&Default::default()) }
    }

    /// Swap view `index`'s cell volumes for a frame seen through
    /// `view_proj` (unjittered), with a horizontal and vertical half field
    /// of view `tan_half` and the grid reaching `far_m`.
    pub fn begin(&mut self, gpu: &Gpu, index: usize, view_proj: Mat4, tan_half: (f32, f32), far_m: f32) -> Frame {
        if self.views.len() <= index { self.views.resize_with(index + 1, || None); }
        let slot = &mut self.views[index];
        let rw = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let v = slot.get_or_insert_with(|| View {
            cells: [volume(gpu, "froxel fog cells a", GRID, rw), volume(gpu, "froxel fog cells b", GRID, rw)],
            integrated: volume(gpu, "froxel fog", GRID, rw),
            uniform: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("froxel fog integrate"), size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            cur: 0, prev_vp: None, frame: 0 });
        v.cur = 1 - v.cur;
        v.frame = v.frame.wrapping_add(1);
        let prev_view_proj = v.prev_vp.replace(view_proj);
        gpu.queue.write_buffer(&v.uniform, 0, bytemuck::cast_slice(&[tan_half.0, tan_half.1, NEAR, far_m]));
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let cells_out = view(&v.cells[v.cur]);
        let integrated = view(&v.integrated);
        let integrate_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("froxel fog integrate"),
            layout: &self.integrate.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: v.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&cells_out) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&integrated) },
            ] });
        Frame { cells_prev: view(&v.cells[1 - v.cur]), cells_out, integrated, prev_view_proj, frame: v.frame, integrate_bind }
    }

    /// Integrate this frame's cells (after `fog_inject`).
    pub fn integrate(&self, enc: &mut wgpu::CommandEncoder, frame: &Frame, timestamps: Option<(&wgpu::QuerySet, u32)>) {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("froxel fog integrate"),
            timestamp_writes: timestamps.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
        pass.set_pipeline(&self.integrate);
        pass.set_bind_group(0, &frame.integrate_bind, &[]);
        pass.dispatch_workgroups(GRID[0].div_ceil(8), GRID[1].div_ceil(8), 1);
    }

    /// Forget every view's history (a cut).
    pub fn reset(&mut self) { for v in self.views.iter_mut().flatten() { v.prev_vp = None; } }
}
