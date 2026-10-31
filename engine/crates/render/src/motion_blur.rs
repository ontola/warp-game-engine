//! Optional per-view camera AND rigid-object motion blur. No temporal feedback:
//! reads reconstructed HDR and writes a separate view-local image before bloom/HUD.
use crate::{Gpu, motion::MotionOutput};
use std::sync::Mutex;
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub strength: f32,
    pub shutter_seconds: f32,
    /// Elapsed presentation simulation time, zero on pause. Large gaps reset.
    pub delta_seconds: f32,
    /// At 720 vertical pixels; scaled with the individual viewport resolution.
    pub max_pixels: f32,
}
impl Settings {
    pub fn scale(self) -> f32 {
        if !self.strength.is_finite()
            || !self.shutter_seconds.is_finite()
            || !self.delta_seconds.is_finite()
            || !self.max_pixels.is_finite()
            || self.delta_seconds <= 0.
            || self.delta_seconds > 0.1
        {
            return 0.;
        }
        self.strength.clamp(0., 2.) * self.shutter_seconds.clamp(0., 1. / 30.)
            / self.delta_seconds.max(1. / 500.)
    }
}
struct Target {
    texture: wgpu::Texture,
    uniform: wgpu::Buffer,
}
pub(crate) struct Resources {
    pipeline: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    views: Mutex<Vec<Target>>,
    pub settings: Vec<Settings>,
}
impl Resources {
    pub fn new(gpu: &Gpu) -> Self {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("motion blur"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/motion_blur.wgsl").into()),
            });
        let entries = (0..7)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                count: None,
                ty: match binding {
                    4 => wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    5 => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    6 => wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    _ => wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float {
                            filterable: binding == 0,
                        },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
            })
            .collect::<Vec<_>>();
        let bind = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("motion blur"),
                entries: &entries,
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("motion blur"),
                bind_group_layouts: &[Some(&bind)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("motion blur"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            sampler,
            views: Mutex::new(Vec::new()),
            settings: Vec::new(),
        }
    }
    pub fn render(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        index: usize,
        input: &wgpu::Texture,
        motion: &MotionOutput,
        timestamps: Option<(&wgpu::QuerySet, u32)>,
    ) -> wgpu::Texture {
        let s = self.settings.get(index).copied().unwrap_or_default();
        if motion.reset || s.scale() <= 0. || s.max_pixels <= 0. {
            return input.clone();
        }
        let make = || Target {
            texture: gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("view-local motion blur"),
                size: input.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
            uniform: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("motion blur shutter"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        };
        let mut views = self.views.lock().unwrap();
        views.truncate(self.settings.len());
        while views.len() <= index {
            views.push(make());
        }
        let target = &mut views[index];
        if target.texture.size() != input.size() {
            *target = make();
        }
        gpu.queue.write_buffer(
            &target.uniform,
            0,
            bytemuck::cast_slice(&[
                s.scale(),
                s.max_pixels.clamp(0., 64.) * input.height() as f32 / 720.,
                0.,
                0.,
            ]),
        );
        let tex = [
            input,
            &motion.vectors,
            &motion.depth,
            &motion.bias,
            &target.texture,
        ]
        .map(|t| t.create_view(&Default::default()));
        let mut entries: Vec<_> = tex
            .iter()
            .enumerate()
            .map(|(i, t)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(t),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: target.uniform.as_entire_binding(),
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion blur"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("motion blur"),
            timestamp_writes: timestamps.map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            }),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(input.width().div_ceil(8), input.height().div_ceil(8), 1);
        target.texture.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_shutter_and_pause_cut_limits() {
        let s = Settings {
            strength: 1.,
            shutter_seconds: 1. / 60.,
            delta_seconds: 1. / 120.,
            max_pixels: 36.,
        };
        assert_eq!(s.scale(), 2.);
        assert_eq!(
            Settings {
                delta_seconds: 0.,
                ..s
            }
            .scale(),
            0.
        );
        assert_eq!(
            Settings {
                delta_seconds: 1.,
                ..s
            }
            .scale(),
            0.
        );
        assert_eq!(
            Settings {
                strength: f32::NAN,
                ..s
            }
            .scale(),
            0.
        );
        let slow = Settings {
            delta_seconds: 1. / 30.,
            ..s
        };
        assert!((s.scale() / 120. - slow.scale() / 30.).abs() < 1e-6);
    }
    #[test]
    fn shader_validates() {
        let m = naga::front::wgsl::parse_str(include_str!("shaders/motion_blur.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap();
    }
}
