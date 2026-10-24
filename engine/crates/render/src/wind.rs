//! Bounded affine plant wind, evaluated once per instance on the GPU.
//!
//! All views, cluster LODs, depth shadows and deferred reconstruction consume
//! one coefficient buffer. No CPU plant transforms or per-vertex trigonometry.
//! This is whole-plant sway, not branch/leaf physics. The canopy and far-grass
//! density remain mean rest-pose fields; experimental volume-tree owners stay
//! static until their density sampling supports the identical deformation.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

const PERIOD: f64 = 120.0;

/// World-space wind direction is X/Z. Shear is horizontal metres per vertical
/// metre above the instance root. Strength zero preserves legacy rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindState {
    pub time_seconds: f64,
    pub direction: Vec2,
    pub strength: f32,
    pub tree_shear: f32,
    pub grass_shear: f32,
}
impl Default for WindState {
    fn default() -> Self {
        Self { time_seconds: 0.0, direction: Vec2::new(1.0, 0.35).normalize(),
            strength: 0.0, tree_shear: 0.025, grass_shear: 0.4 }
    }
}
impl WindState {
    pub fn validated(&self) -> Result<Self, String> {
        if !self.time_seconds.is_finite() || !self.direction.is_finite()
            || self.direction.length_squared() < 1e-12
            || !self.direction.length_squared().is_finite()
            || !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength)
            || !self.tree_shear.is_finite() || !(0.0..=0.08).contains(&self.tree_shear)
            || !self.grass_shear.is_finite() || !(0.0..=0.6).contains(&self.grass_shear) {
            return Err("wind requires finite time, a nonzero direction, strength 0..1, tree shear 0..0.08 and grass shear 0..0.6".into());
        }
        Ok(Self { direction: self.direction.normalize(), ..*self })
    }

    /// Diagnostic CPU counterpart of the compute field, for independent GPU
    /// image fixtures. Invalid states/roots return zero; set_wind rejects them.
    pub fn sample_shear(&self, root: Vec3, grass: bool) -> Vec2 {
        let Ok(state) = self.validated() else { return Vec2::ZERO; };
        if !root.is_finite() || state.strength == 0.0 { return Vec2::ZERO; }
        let p = Uniform::from_state(state);
        let direction = Vec2::new(p.field[0], p.field[1]);
        let cross = Vec2::new(-direction.y, direction.x);
        let root = Vec2::new(root.x, root.z);
        let along = root.dot(direction);
        let across = root.dot(cross);
        let phase = p.field[3];
        let a = along * 0.12 - phase * 9.0;
        let b = along * 0.057 + across * 0.033 - phase * 17.0;
        let c = along * 0.21 + across * 0.37 - phase * 41.0;
        let wave = 0.52 + 0.26 * a.sin() + 0.09 * b.sin();
        let amplitude = if grass { p.amplitudes[1] } else { p.amplitudes[0] };
        (direction * wave + cross * (0.13 * c.sin())) * (amplitude * p.field[2])
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniform {
    field: [f32; 4], // normalized X/Z direction, strength, bounded time phase
    amplitudes: [f32; 4], // tree shear, grass shear, reserved
}
impl Uniform {
    fn from_state(state: WindState) -> Self {
        if state.strength == 0.0 { return Self::zeroed(); }
        // Integer temporal harmonics make the wrapped phase continuous. Do the
        // modulo in f64 before narrowing so long sessions retain time precision.
        let phase = (state.time_seconds.rem_euclid(PERIOD) * std::f64::consts::TAU / PERIOD) as f32;
        Self { field: [state.direction.x, state.direction.y, state.strength, phase],
            amplitudes: [state.tree_shear, state.grass_shear, 0.0, 0.0] }
    }
}
struct Cache {
    uniform: Uniform,
    clear_pending: bool,
}

pub struct WindRenderer {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    coefficients: wgpu::Buffer,
    count: u32,
    cache: Mutex<Cache>,
}
impl WindRenderer {
    /// Instances and replacement masks are the live scene buffers. Streaming
    /// changes therefore need no CPU wind upload or allocation. Buffer records
    /// are vec4(shear_x, shear_z, conservative world displacement, reserved).
    pub fn new(gpu: &Gpu, instances: &wgpu::Buffer, replacements: &wgpu::Buffer,
        instance_count: usize) -> Result<Self, String> {
        let count = u32::try_from(instance_count).map_err(|_| "wind instance count exceeds u32")?;
        let bytes = u64::from(count.max(1)) * 16;
        if bytes > u64::from(gpu.limits.max_storage_buffer_binding_size)
            || count.div_ceil(64) > gpu.limits.max_compute_workgroups_per_dimension {
            return Err("wind coefficient buffer/workgroup count exceeds this GPU's limits".into());
        }
        let initial = Uniform::zeroed();
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wind state"), contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let coefficients = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plant wind coefficients"), size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("plant wind"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/wind.wgsl").into()),
        });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("plant wind"), layout: None, module: &module,
            entry_point: Some("update_wind"), compilation_options: Default::default(), cache: None,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plant wind"), layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: replacements.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: coefficients.as_entire_binding() },
            ],
        });
        Ok(Self { pipeline, bind, uniform, coefficients, count,
            cache: Mutex::new(Cache { uniform: initial, clear_pending: false }) })
    }

    pub fn buffer(&self) -> &wgpu::Buffer { &self.coefficients }

    /// Returns true only when the actual GPU field changes. Invalidate cached
    /// sun shadows on true, including active -> disabled. Disabled time changes
    /// are inert. No structural canopy invalidation is necessary.
    pub fn update(&self, gpu: &Gpu, state: &WindState) -> Result<bool, String> {
        let uniform = Uniform::from_state(state.validated()?);
        let mut cache = self.cache.lock().unwrap();
        if cache.uniform == uniform { return Ok(false); }
        cache.clear_pending |= cache.uniform.field[2] != 0.0 && uniform.field[2] == 0.0;
        cache.uniform = uniform;
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        Ok(true)
    }

    /// Encode once BEFORE any cull/raster/shadow pass. Active fixed-time fields
    /// still dispatch each frame because streaming can repurpose instance slots.
    /// A disabled field dispatches once to clear previously animated slots.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder,
        timestamp: Option<(&wgpu::QuerySet, u32)>) -> bool {
        let mut cache = self.cache.lock().unwrap();
        if self.count == 0 || (cache.uniform.field[2] == 0.0 && !cache.clear_pending) { return false; }
        cache.clear_pending = false;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("plant wind"),
            timestamp_writes: timestamp.map(|(query_set, start)| wgpu::ComputePassTimestampWrites {
                query_set, beginning_of_pass_write_index: Some(start), end_of_pass_write_index: Some(start + 1),
            }),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
        true
    }
}

pub(crate) fn common_source() -> &'static str { include_str!("shaders/wind_common.wgsl") }
pub(crate) fn shade_source() -> String {
    format!("{}\n{}", common_source(), include_str!("shaders/shade.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_and_disabled_time_are_safe() {
        let valid = WindState { strength: 0.3, ..Default::default() };
        for invalid in [WindState { strength: 1.1, ..valid }, WindState { time_seconds: f64::NAN, ..valid },
            WindState { direction: Vec2::ZERO, ..valid }, WindState { grass_shear: 0.7, ..valid }] {
            assert!(invalid.validated().is_err());
        }
        assert!(WindState { direction: Vec2::new(8.0, 3.0), ..valid }.validated().unwrap().direction.is_normalized());
        assert_eq!(std::mem::size_of::<Uniform>(), 32);
        assert!(Uniform::from_state(WindState::default()) == Uniform::zeroed());
        assert!(Uniform::from_state(WindState { time_seconds: 1e12, ..Default::default() }) == Uniform::zeroed());
    }

    #[test]
    fn phase_is_root_stable_bounded_and_continuous_across_wrap() {
        let root = Vec3::new(-386.3, 120.0, 234.6);
        let state = WindState { strength: 1.0, ..Default::default() };
        let mut previous = Vec2::ZERO;
        for i in 0..480 {
            let state = WindState { time_seconds: i as f64 * 0.25, ..state };
            let tree = state.sample_shear(root, false);
            let grass = state.sample_shear(root, true);
            assert!(tree.length() <= state.tree_shear);
            assert!(grass.length() <= state.grass_shear);
            assert_eq!(tree, state.sample_shear(root + Vec3::Y * 400.0, false));
            assert!((tree - state.sample_shear(root, false)).length() == 0.0);
            if i != 0 { assert!((tree - previous).length() < 0.015); }
            previous = tree;
        }
        let before = WindState { time_seconds: PERIOD - 1e-5, ..state }.sample_shear(root, true);
        let after = WindState { time_seconds: PERIOD + 1e-5, ..state }.sample_shear(root, true);
        assert!((before-after).length() < 1e-4);
    }

    #[test]
    fn anchored_shear_preserves_volume_and_normal_tangency_with_safe_bounds() {
        let root = Vec3::new(12.0, 8.0, -30.0);
        let shear = Vec2::new(0.035, -0.027);
        let deform = |p: Vec3| p + Vec3::new(shear.x, 0.0, shear.y) * (p.y-root.y);
        let normal = |n: Vec3| Vec3::new(n.x, n.y-shear.dot(Vec2::new(n.x,n.z)), n.z).normalize();
        assert_eq!(deform(root), root);
        let a = Vec3::new(3.0, 2.0, -1.0);
        let b = Vec3::new(-1.0, 4.0, 2.0);
        let da = deform(root+a)-root;
        let db = deform(root+b)-root;
        assert!(normal(a.cross(b)).dot(da).abs() < 1e-5);
        assert!(normal(a.cross(b)).dot(db).abs() < 1e-5);
        let basis = glam::Mat3::from_cols(Vec3::X, Vec3::new(shear.x,1.0,shear.y), Vec3::Z);
        assert!((basis.determinant()-1.0).abs()<1e-6);
        let centre = root+Vec3::new(1.0,7.0,-2.0);
        let radius = 10.0;
        let padding = ((centre.y-root.y).abs()+radius)*shear.length();
        for i in 0..360 {
            let angle = i as f32 * std::f32::consts::PI/180.0;
            let p = centre+Vec3::new(angle.cos(),angle.sin(),0.0)*radius;
            assert!(deform(p).distance(centre)<=radius+padding+1e-5);
        }
    }
}
