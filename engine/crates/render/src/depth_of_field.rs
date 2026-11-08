//! Optional per-view thin-lens depth of field. Reads the final HDR (after
//! motion blur) and the view's depth, writes a separate view-local image
//! before exposure, bloom, tone and HUD. Never feeds TAA/DLSS history.
//! Textures and one uniform only: no storage buffers, its own bind group.
use crate::{Camera, Gpu};
use std::sync::Mutex;

/// One camera lens. `f_stop <= 0` (the default) disables the effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Aperture as an f-number: f/1.4 is very shallow, f/16 nearly all sharp.
    pub f_stop: f32,
    /// Focus distance in metres. Zero or less: autofocus on `focus_point`,
    /// read on the GPU from this frame's depth (no readback).
    pub focus_m: f32,
    /// Autofocus point in the view, 0..1 from the top left.
    pub focus_point: [f32; 2],
    /// Film height in millimetres (24 is full frame). With the camera's
    /// vertical field of view this gives the focal length.
    pub sensor_mm: f32,
    /// Largest blur radius in pixels at 720 vertical pixels; scales with
    /// each view's height. Clamped to 32.
    pub max_radius: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self { f_stop: 0.0, focus_m: 0.0, focus_point: [0.5, 0.5], sensor_mm: 24.0, max_radius: 12.0 }
    }
}
impl Settings {
    pub fn active(&self) -> bool {
        self.f_stop.is_finite() && self.f_stop > 0.0 && self.sensor_mm.is_finite() && self.sensor_mm > 0.0
            && self.max_radius.is_finite() && self.max_radius > 0.0 && self.focus_m.is_finite()
    }
    /// Focal length in metres for a vertical field of view.
    pub fn focal_m(&self, fov_y: f32) -> f32 {
        0.5 * self.sensor_mm * 1e-3 / (0.5 * fov_y).tan().max(1e-4)
    }
    fn max_radius_px(&self, view_h: u32) -> f32 {
        self.max_radius.clamp(0.0, 32.0) * view_h as f32 / 720.0
    }
    /// Signed blur radius in pixels (negative in front of focus) of a point
    /// `distance_m` away, for a view `view_h` pixels tall. Mirrors the shader.
    pub fn radius_px(&self, fov_y: f32, view_h: u32, focus_m: f32, distance_m: f32) -> f32 {
        let f = self.focal_m(fov_y);
        let s = focus_m.max(f * 1.01);
        let lens = self.lens(fov_y, view_h);
        let max = self.max_radius_px(view_h);
        (lens * (1.0 - s / distance_m.max(1e-4)) / (s - f)).clamp(-max, max)
    }
    /// Half the circle of confusion (f^2 / N per unit defocus) in pixels.
    fn lens(&self, fov_y: f32, view_h: u32) -> f32 {
        let f = self.focal_m(fov_y);
        0.5 * f * f / self.f_stop.max(0.5) / (self.sensor_mm * 1e-3) * view_h as f32
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    focus_uv: [f32; 2],
    focus_m: f32,
    near: f32,
    lens: f32,
    focal_m: f32,
    max_radius: f32,
    reach: f32,
}

struct Target {
    tiles: wgpu::Texture,
    output: wgpu::Texture,
    uniform: wgpu::Buffer,
}

pub(crate) struct Resources {
    tile_pipeline: wgpu::ComputePipeline,
    gather_pipeline: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    views: Mutex<Vec<Target>>,
    pub settings: Vec<Settings>,
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}

impl Resources {
    pub fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("depth of field"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth_of_field.wgsl").into()),
        });
        let texture = |filterable| wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let depth = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let storage = |format| wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        // Separate layouts: the tile texture is written by one pass and read
        // by the next, never both in one bind group.
        let layouts = [
            vec![entry(0, texture(true)), entry(1, depth), entry(2, storage(wgpu::TextureFormat::R32Float)), entry(6, uniform)],
            vec![entry(0, texture(true)), entry(1, depth), entry(3, texture(false)),
                entry(4, storage(wgpu::TextureFormat::Rgba16Float)),
                entry(5, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)), entry(6, uniform)],
        ];
        let [tile_pipeline, gather_pipeline] = [(0, "tile_max"), (1, "gather")].map(|(i, entry_point)| {
            let bind = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("depth of field"), entries: &layouts[i] });
            let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("depth of field"), bind_group_layouts: &[Some(&bind)], immediate_size: 0,
            });
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point), layout: Some(&layout), module: &module, entry_point: Some(entry_point),
                compilation_options: Default::default(), cache: gpu.pipeline_cache(),
            })
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { tile_pipeline, gather_pipeline, sampler, views: Mutex::new(Vec::new()), settings: Vec::new() }
    }

    pub fn active(&self, index: usize) -> bool {
        self.settings.get(index).is_some_and(Settings::active)
    }

    /// The blurred image, or `None` when this view has no active lens.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        index: usize,
        input: &wgpu::Texture,
        depth: &wgpu::TextureView,
        cam: &Camera,
        timestamps: Option<(&wgpu::QuerySet, u32)>,
    ) -> Option<wgpu::Texture> {
        let s = self.settings.get(index).copied().filter(Settings::active)?;
        let size = input.size();
        let tiles_size = wgpu::Extent3d { width: size.width.div_ceil(8), height: size.height.div_ceil(8), depth_or_array_layers: 1 };
        let make = || {
            let texture = |label, size, format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label), size, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[],
            });
            Target {
                tiles: texture("depth of field tiles", tiles_size, wgpu::TextureFormat::R32Float,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING),
                output: texture("view-local depth of field", size, wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC),
                uniform: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("depth of field lens"), size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
                }),
            }
        };
        let mut views = self.views.lock().unwrap();
        views.truncate(self.settings.len());
        while views.len() <= index { views.push(make()); }
        let target = &mut views[index];
        if target.output.size() != size { *target = make(); }
        let max_radius = s.max_radius_px(size.height);
        let params = Params {
            focus_uv: s.focus_point.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 }),
            focus_m: s.focus_m,
            near: cam.near,
            lens: s.lens(cam.fov_y, size.height),
            focal_m: s.focal_m(cam.fov_y),
            max_radius,
            // At most 9x9 tiles; a larger radius only loses some foreground spread.
            reach: (max_radius / 8.0).ceil().clamp(1.0, 4.0),
        };
        gpu.queue.write_buffer(&target.uniform, 0, bytemuck::bytes_of(&params));
        let input_v = input.create_view(&Default::default());
        let tiles_v = target.tiles.create_view(&Default::default());
        let output_v = target.output.create_view(&Default::default());
        let bind = |pipeline: &wgpu::ComputePipeline, entries: &[(u32, wgpu::BindingResource)]| {
            let entries: Vec<_> = entries.iter().cloned().map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource }).collect();
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("depth of field"), layout: &pipeline.get_bind_group_layout(0), entries: &entries })
        };
        let tile_bind = bind(&self.tile_pipeline, &[
            (0, wgpu::BindingResource::TextureView(&input_v)), (1, wgpu::BindingResource::TextureView(depth)),
            (2, wgpu::BindingResource::TextureView(&tiles_v)), (6, target.uniform.as_entire_binding()),
        ]);
        let gather_bind = bind(&self.gather_pipeline, &[
            (0, wgpu::BindingResource::TextureView(&input_v)), (1, wgpu::BindingResource::TextureView(depth)),
            (3, wgpu::BindingResource::TextureView(&tiles_v)), (4, wgpu::BindingResource::TextureView(&output_v)),
            (5, wgpu::BindingResource::Sampler(&self.sampler)), (6, target.uniform.as_entire_binding()),
        ]);
        for (i, (pipeline, group)) in [(&self.tile_pipeline, &tile_bind), (&self.gather_pipeline, &gather_bind)].into_iter().enumerate() {
            // One timestamp pair spans both passes.
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("depth of field"),
                timestamp_writes: timestamps.map(|(query_set, t)| wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: (i == 0).then_some(t),
                    end_of_pass_write_index: (i == 1).then_some(t + 1),
                }),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(tiles_size.width, tiles_size.height, 1);
        }
        Some(target.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fifty_mm() -> (Settings, f32) {
        // 50 mm on full-frame film: vertical field of view 2 atan(12/50).
        (Settings { f_stop: 2.0, max_radius: 32.0, ..Default::default() }, 2.0 * (12.0f32 / 50.0).atan())
    }
    #[test]
    fn thin_lens_circle_of_confusion() {
        let (s, fov) = fifty_mm();
        assert!((s.focal_m(fov) - 0.05).abs() < 1e-6);
        // f^2/(N(S-f)) = 0.0025/(2*2.95) m = 0.424 mm across 24 mm of film.
        let at_infinity = s.radius_px(fov, 720, 3.0, 1e7);
        assert!((at_infinity - 0.5 * 0.0025 / (2.0 * 2.95) / 0.024 * 720.0).abs() < 0.01, "{at_infinity}");
        assert_eq!(s.radius_px(fov, 720, 3.0, 3.0), 0.0);
        assert!(s.radius_px(fov, 720, 3.0, 1.5) < 0.0, "in front of focus is negative");
        // Twice the aperture number, half the blur; scales with view height.
        let f4 = Settings { f_stop: 4.0, ..s };
        assert!((f4.radius_px(fov, 720, 3.0, 10.0) * 2.0 - s.radius_px(fov, 720, 3.0, 10.0)).abs() < 1e-4);
        assert!((s.radius_px(fov, 1440, 3.0, 10.0) - 2.0 * s.radius_px(fov, 720, 3.0, 10.0)).abs() < 1e-4);
        // Clamped to the reference maximum.
        let tight = Settings { max_radius: 4.0, ..s };
        assert_eq!(tight.radius_px(fov, 1440, 3.0, 0.2), -8.0);
    }
    #[test]
    fn disabled_unless_valid() {
        assert!(!Settings::default().active());
        let (s, _) = fifty_mm();
        assert!(s.active());
        for bad in [Settings { f_stop: f32::NAN, ..s }, Settings { sensor_mm: 0.0, ..s }, Settings { max_radius: 0.0, ..s }, Settings { focus_m: f32::INFINITY, ..s }] {
            assert!(!bad.active(), "{bad:?}");
        }
    }
    #[test]
    fn shader_validates_and_uses_no_storage_buffers() {
        let src = include_str!("shaders/depth_of_field.wgsl");
        let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&m).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        assert!(m.global_variables.iter().all(|(_, g)| !matches!(g.space, naga::AddressSpace::Storage { .. })));
        assert_eq!(std::mem::size_of::<Params>(), 32);
    }
}
