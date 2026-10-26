//! GPU sampling of the bounded CPU radiance-cache reference. No gameplay hook.
//! Transport/update scheduling remains CPU-side; this module only uploads one
//! shared world cache and evaluates SH irradiance plus visibility on the GPU.
use crate::{
    Gpu,
    radiance_cache::{GpuMoment, GpuProbe, RAYS_PER_PROBE, RadianceCache, Sample},
};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

pub const MAX_PROBES: usize = 2048;
pub const MAX_QUERIES: usize = 1024 * 1024;
pub const SHADER: &str = include_str!("shaders/radiance_cache.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Query {
    pub position: [f32; 4],
    pub normal: [f32; 4],
}
impl Query {
    pub fn new(position: Vec3, normal: Vec3) -> Self {
        Self {
            position: [position.x, position.y, position.z, 0.0],
            normal: [normal.x, normal.y, normal.z, 0.0],
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Parameters {
    values: [f32; 4],
    counts: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawSample {
    irradiance_known: [f32; 4],
    visibility_count: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct UploadStats {
    pub probes: usize,
    pub valid_probes: usize,
    pub moments: usize,
    pub bytes: u64,
}
struct Packed {
    probes: Vec<GpuProbe>,
    moments: Vec<GpuMoment>,
    keys: Vec<[i32; 4]>,
    spacing: f32,
    bias: f32,
    stats: UploadStats,
}
impl Packed {
    fn new(cache: &RadianceCache) -> Result<Self, String> {
        let config = cache.config();
        let (probes, moments) = cache.packed();
        let keys: Vec<_> = cache
            .probes()
            .map(|p| [p.key.0[0], p.key.0[1], p.key.0[2], 0])
            .collect();
        if probes.len() > MAX_PROBES
            || probes.len() != keys.len()
            || moments.len() != probes.len() * RAYS_PER_PROBE
        {
            return Err("GPU radiance cache requires <=2048 probes with exactly96 moments each; no truncation".into());
        }
        for (i, p) in probes.iter().enumerate() {
            if p.position_spacing[3] != config.spacing
                || p.meta[2] as usize != i * RAYS_PER_PROBE
                || p.meta[3] as usize != RAYS_PER_PROBE
                || !p.position_spacing.iter().all(|v| v.is_finite())
                || !p.irradiance_sh.iter().flatten().all(|v| v.is_finite())
            {
                return Err("invalid packed radiance-cache probe layout".into());
            }
        }
        if moments
            .iter()
            .any(|m| !m.data.iter().all(|v| v.is_finite()))
        {
            return Err("non-finite directional radiance-cache moment".into());
        }
        let stats = UploadStats {
            probes: probes.len(),
            valid_probes: probes.iter().filter(|p| p.meta[0] != 0).count(),
            moments: moments.len(),
            bytes: (probes.len().max(1) * std::mem::size_of::<GpuProbe>()
                + moments.len().max(1) * 16
                + keys.len().max(1) * 16) as u64,
        };
        Ok(Self {
            probes,
            moments,
            keys,
            spacing: config.spacing,
            bias: config.ray_bias,
            stats,
        })
    }
}

pub struct GpuRadianceCache {
    probes: wgpu::Buffer,
    moments: wgpu::Buffer,
    keys: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    spacing: f32,
    bias: f32,
    pub stats: UploadStats,
}
fn buffer_limit(gpu: &Gpu, bytes: u64, label: &str) -> Result<(), String> {
    if bytes > gpu.limits.max_storage_buffer_binding_size as u64
        || bytes > gpu.limits.max_buffer_size
    {
        Err(format!(
            "{label}: {bytes} bytes exceeds device storage/buffer limit"
        ))
    } else {
        Ok(())
    }
}
fn upload<T: Pod + Zeroable>(gpu: &Gpu, label: &str, values: &[T]) -> Result<wgpu::Buffer, String> {
    let zero = [T::zeroed()];
    let values = if values.is_empty() { &zero[..] } else { values };
    let bytes = bytemuck::cast_slice(values);
    buffer_limit(gpu, bytes.len() as u64, label)?;
    Ok(gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        }))
}
impl GpuRadianceCache {
    pub fn new(gpu: &Gpu, cache: &RadianceCache) -> Result<Self, String> {
        let p = Packed::new(cache)?;
        let probes = upload(gpu, "radiance probes", &p.probes)?;
        let moments = upload(gpu, "radiance distance moments", &p.moments)?;
        let keys = upload(gpu, "radiance world identities", &p.keys)?;
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let image = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let entries = [
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            storage(true),
            storage(true),
            storage(true),
            storage(true),
            storage(false),
            image,
            image,
        ];
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("radiance cache sample"),
                entries: &entries
                    .into_iter()
                    .enumerate()
                    .map(|(i, ty)| wgpu::BindGroupLayoutEntry {
                        binding: i as u32,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty,
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("radiance cache sampling"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("radiance cache sample"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("radiance cache sample"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("sample_cache"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        Ok(Self {
            probes,
            moments,
            keys,
            layout,
            pipeline,
            spacing: p.spacing,
            bias: p.bias,
            stats: p.stats,
        })
    }

    /// One query per image pixel. This bounded diagnostic path writes raw HDR
    /// samples, a white-Lambert receiver visualization, and validity separately.
    /// It neither shades game materials nor traces new transport rays.
    pub fn encode(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        queries: &[Query],
        size: [u32; 2],
        exposure: f32,
    ) -> Result<Batch, String> {
        let [width, height] = size;
        if width == 0
            || height == 0
            || width.max(height) > gpu.limits.max_texture_dimension_2d
            || (width as usize).checked_mul(height as usize) != Some(queries.len())
            || queries.len() > MAX_QUERIES
            || !exposure.is_finite()
            || exposure <= 0.0
        {
            return Err("invalid bounded radiance query image dimensions/exposure".into());
        }
        if queries.iter().any(|q| {
            !q.position
                .iter()
                .chain(q.normal.iter())
                .all(|v| v.is_finite())
                || Vec3::from_slice(&q.normal).length_squared() > 1.0e20
        }) {
            return Err("radiance query position/normal must be finite and bounded".into());
        }
        if width.div_ceil(8) > gpu.limits.max_compute_workgroups_per_dimension
            || height.div_ceil(8) > gpu.limits.max_compute_workgroups_per_dimension
        {
            return Err("radiance query dispatch exceeds device limit".into());
        }
        let query = upload(gpu, "radiance sample positions", queries)?;
        let params = Parameters {
            values: [self.spacing, self.bias, exposure, 0.0],
            counts: [
                self.stats.probes as u32,
                queries.len() as u32,
                width,
                height,
            ],
        };
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("radiance sample dimensions"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bytes = (queries.len() * std::mem::size_of::<RawSample>()) as u64;
        buffer_limit(gpu, bytes, "radiance query output")?;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radiance sample HDR values"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radiance sample readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let image = |label| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let color = image("radiance cache receiver image");
        let validity = image("radiance cache coverage image");
        let cv = color.create_view(&Default::default());
        let vv = validity.create_view(&Default::default());
        let resources = [
            uniform.as_entire_binding(),
            self.probes.as_entire_binding(),
            self.moments.as_entire_binding(),
            self.keys.as_entire_binding(),
            query.as_entire_binding(),
            output.as_entire_binding(),
            wgpu::BindingResource::TextureView(&cv),
            wgpu::BindingResource::TextureView(&vv),
        ];
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radiance cache queries"),
            layout: &self.layout,
            entries: &resources
                .into_iter()
                .enumerate()
                .map(|(i, resource)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource,
                })
                .collect::<Vec<_>>(),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("radiance cache sampling"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &read, 0, bytes);
        Ok(Batch {
            read,
            color,
            validity,
            count: queries.len(),
        })
    }
}
pub struct Batch {
    read: wgpu::Buffer,
    color: wgpu::Texture,
    validity: wgpu::Texture,
    count: usize,
}
pub struct Readback {
    pub samples: Vec<Sample>,
    pub rgba: Vec<u8>,
    pub validity_rgba: Vec<u8>,
}
impl Batch {
    /// Submit the encoded command buffer before calling this method.
    pub fn read(self, gpu: &Gpu) -> Result<Readback, String> {
        self.read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.wait();
        let samples = {
            let data = self
                .read
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("radiance cache readback: {e:?}"))?;
            let raw: &[RawSample] = bytemuck::cast_slice(&data);
            raw[..self.count]
                .iter()
                .map(|s| Sample {
                    irradiance: Vec3::from_slice(&s.irradiance_known),
                    known_weight: s.irradiance_known[3],
                    visibility_weight: s.visibility_count[0],
                    contributing_probes: s.visibility_count[1] as u32,
                })
                .collect()
        };
        self.read.unmap();
        Ok(Readback {
            samples,
            rgba: gpu.read_rgba8(&self.color),
            validity_rgba: gpu.read_rgba8(&self.validity),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_and_upload_layout_are_valid() {
        let module = naga::front::wgsl::parse_str(SHADER)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(std::mem::size_of::<Parameters>(), 32);
        assert_eq!(std::mem::size_of::<Query>(), 32);
        assert_eq!(std::mem::size_of::<RawSample>(), 32);
    }
    #[test]
    fn upload_preserves_negative_world_ids_and_rejects_over_budget_without_truncating() {
        let mut cache = RadianceCache::new(crate::radiance_cache::Config {
            radius_cells: [1; 3],
            ..Default::default()
        })
        .unwrap();
        cache.set_views(&[Vec3::new(-8.0, 4.0, -12.0)]).unwrap();
        let packed = Packed::new(&cache).unwrap();
        assert_eq!(packed.keys.len(), 27);
        assert_eq!(packed.keys[0], [-3, 0, -4, 0]);
        assert_eq!(packed.stats.moments, 27 * 96);
        let mut large = RadianceCache::new(crate::radiance_cache::Config {
            radius_cells: [8; 3],
            capacity: 2049,
            ..Default::default()
        })
        .unwrap();
        large.set_views(&[Vec3::ZERO]).unwrap();
        assert!(Packed::new(&large).is_err());
        assert_eq!(large.probes().count(), 2049);
    }
}
