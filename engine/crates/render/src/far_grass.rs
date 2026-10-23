//! Terrain-column grass density. Four vertical bins follow the exact native
//! terrain triangles; no displaced flat plane or arbitrary last-metres clip.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use std::{sync::Mutex, time::Duration};
use wgpu::util::DeviceExt;

#[derive(Clone, Debug)]
pub struct FarGrassField {
    pub origin: [f32; 2],
    pub cell: f32,
    /// Dimensions of the terrain/road point grids, not the cell grid.
    pub width: u32,
    pub height: u32,
    pub heights: Vec<f32>,
    pub road: Vec<f32>,
    /// Row-major (width-1)*(height-1), linear normalized habitat coverage.
    pub coverage: Vec<u8>,
    /// Extinction per metre for full coverage and a horizontal view ray.
    /// Includes the three-card/random-yaw factor; four equal height bins.
    pub profile: [f32; 4],
    pub max_height: f32,
    pub albedo: [f32; 3],
}
#[derive(Clone, Debug, Default)]
pub struct FarGrassDiagnostics {
    pub traced_rays: u32,
    pub overflow_rays: u32,
    pub max_iterations: u32,
}
#[derive(Clone)]
pub(crate) struct FarGrassLayer {
    pub color: wgpu::TextureView,
    pub distance: wgpu::TextureView,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    grid: [f32; 4], // origin x/z, cell, maximum height
    dims: [u32; 4], // point width/height, half-output width/height
    full: [u32; 4], // full viewport width/height, tile width/height
    profile: [f32; 4],
    albedo: [f32; 4],
    range: [f32; 4],  // inner radius, outer-fade start/end, road clearance
    bounds: [f32; 4], // minimum/maximum occupied world height
}
struct Targets {
    size: (u32, u32),
    _color: wgpu::Texture,
    _distance: wgpu::Texture,
    layer: FarGrassLayer,
    params: wgpu::Buffer,
    bind: wgpu::BindGroup,
    counters: wgpu::Buffer,
}
pub(crate) struct FarGrassRenderer {
    pipeline: wgpu::ComputePipeline,
    data_layout: wgpu::BindGroupLayout,
    terrain: wgpu::TextureView,
    coverage: wgpu::TextureView,
    tiles: wgpu::TextureView,
    base: Params,
    targets: Mutex<Vec<Option<Targets>>>,
}
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}",
        include_str!("shaders/shade.wgsl"),
        include_str!("shaders/far_grass.wgsl")
    )
}
fn prepare(field: &FarGrassField) -> Result<(Vec<[f32; 2]>, Vec<[f32; 2]>, Params), String> {
    let (w, h) = (field.width, field.height);
    if w < 2
        || h < 2
        || !field.cell.is_finite()
        || field.cell <= 0.0
        || !field.max_height.is_finite()
        || field.max_height <= 0.0
        || !field.origin.iter().all(|x| x.is_finite())
        || field.profile.iter().any(|x| !x.is_finite() || *x < 0.0)
        || field.albedo.iter().any(|x| !x.is_finite() || *x < 0.0)
    {
        return Err("invalid far-grass field geometry/optics".into());
    }
    let points = u64::from(w) * u64::from(h);
    let cells = u64::from(w - 1) * u64::from(h - 1);
    if points != field.heights.len() as u64
        || points != field.road.len() as u64
        || cells != field.coverage.len() as u64
        || field
            .heights
            .iter()
            .chain(&field.road)
            .any(|x| !x.is_finite())
    {
        return Err("far-grass field dimensions or finite samples invalid".into());
    }
    if !(field.origin[0] + (w - 1) as f32 * field.cell).is_finite()
        || !(field.origin[1] + (h - 1) as f32 * field.cell).is_finite()
        || field
            .heights
            .iter()
            .any(|height| !(height + field.max_height).is_finite())
    {
        return Err("far-grass world bounds overflow".into());
    }
    let terrain = field
        .heights
        .iter()
        .zip(&field.road)
        .map(|(&a, &b)| [a, b])
        .collect();
    let (tw, th) = ((w - 1).div_ceil(8), (h - 1).div_ceil(8));
    let mut tiles = vec![[1e30f32, -1e30]; (tw * th) as usize];
    let mut bounds = [1e30f32, -1e30];
    for z in 0..h - 1 {
        for x in 0..w - 1 {
            if field.coverage[(z * (w - 1) + x) as usize] == 0 {
                continue;
            }
            let a = (z * w + x) as usize;
            let corners = [a, a + 1, a + w as usize, a + w as usize + 1];
            if corners.iter().all(|&i| field.road[i] < 0.9) {
                continue;
            }
            let lo = corners
                .iter()
                .map(|&i| field.heights[i])
                .fold(f32::INFINITY, f32::min);
            let hi = corners
                .iter()
                .map(|&i| field.heights[i])
                .fold(f32::NEG_INFINITY, f32::max)
                + field.max_height;
            let tile = &mut tiles[(z / 8 * tw + x / 8) as usize];
            tile[0] = tile[0].min(lo);
            tile[1] = tile[1].max(hi);
            bounds[0] = bounds[0].min(lo);
            bounds[1] = bounds[1].max(hi);
        }
    }
    Ok((
        terrain,
        tiles,
        Params {
            grid: [
                field.origin[0],
                field.origin[1],
                field.cell,
                field.max_height,
            ],
            dims: [w, h, 0, 0],
            full: [0, 0, tw, th],
            profile: field.profile,
            albedo: [field.albedo[0], field.albedo[1], field.albedo[2], 0.0],
            range: [40.0, 160.0, 240.0, 0.9],
            bounds: [bounds[0], bounds[1], 0.0, 0.0],
        },
    ))
}
impl FarGrassRenderer {
    pub(crate) fn new(
        gpu: &Gpu,
        field: &FarGrassField,
        scene_layout: &wgpu::BindGroupLayout,
        volume_view_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, String> {
        let (terrain_data, tiles_data, base) = prepare(field)?;
        if field.width > gpu.limits.max_texture_dimension_2d
            || field.height > gpu.limits.max_texture_dimension_2d
        {
            return Err("far-grass field exceeds adapter texture dimensions".into());
        }
        let texture = |label, w, h, format, bpp, bytes: &[u8]| {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            gpu.queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * bpp),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            texture.create_view(&Default::default())
        };
        let terrain = texture(
            "far grass terrain/road",
            field.width,
            field.height,
            wgpu::TextureFormat::Rg32Float,
            8,
            bytemuck::cast_slice(&terrain_data),
        );
        let coverage = texture(
            "far grass habitat",
            field.width - 1,
            field.height - 1,
            wgpu::TextureFormat::R8Unorm,
            1,
            &field.coverage,
        );
        let tiles = texture(
            "far grass minmax",
            base.full[2],
            base.full[3],
            wgpu::TextureFormat::Rg32Float,
            8,
            bytemuck::cast_slice(&tiles_data),
        );
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let tex = |sample_type, view_dimension| wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        };
        let unfiltered = tex(
            wgpu::TextureSampleType::Float { filterable: false },
            wgpu::TextureViewDimension::D2,
        );
        let layout = |label, items: &[(u32, wgpu::BindingType)]| {
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries: &items
                        .iter()
                        .map(|&(binding, ty)| wgpu::BindGroupLayoutEntry {
                            binding,
                            ty,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            count: None,
                        })
                        .collect::<Vec<_>>(),
                })
        };
        let storage_texture = |format| wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let data_layout = layout(
            "far grass data",
            &[
                (0, uniform),
                (1, unfiltered),
                (2, unfiltered),
                (3, unfiltered),
                (4, storage_texture(wgpu::TextureFormat::Rgba16Float)),
                (5, storage_texture(wgpu::TextureFormat::R32Float)),
                (
                    6,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        );
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("far grass"),
                bind_group_layouts: &[
                    Some(scene_layout),
                    Some(volume_view_layout),
                    Some(&data_layout),
                ],
                immediate_size: 0,
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("far grass"),
                source: wgpu::ShaderSource::Wgsl(shader_source().into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("far grass integrate"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("far_grass_integrate"),
                compilation_options: Default::default(),
                cache: None,
            });
        Ok(Self {
            pipeline,
            data_layout,
            terrain,
            coverage,
            tiles,
            base,
            targets: Mutex::new(Vec::new()),
        })
    }
    pub(crate) fn set_view_count(&self, count: usize) {
        let mut targets = self.targets.lock().unwrap();
        targets.truncate(count);
        targets.resize_with(count, || None);
    }
    fn target(&self, gpu: &Gpu, size: (u32, u32)) -> Targets {
        let (w, h) = (size.0.div_ceil(2), size.1.div_ceil(2));
        let texture = |label, format| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let color = texture("far grass C/alpha", wgpu::TextureFormat::Rgba16Float);
        let distance = texture(
            "far grass representative root distance",
            wgpu::TextureFormat::R32Float,
        );
        let layer = FarGrassLayer {
            color: color.create_view(&Default::default()),
            distance: distance.create_view(&Default::default()),
        };
        let params = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("far grass params"),
                contents: bytemuck::bytes_of(&self.base),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let counters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("far grass diagnostics"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("far grass field/output"),
            layout: &self.data_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.terrain),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.coverage),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.tiles),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&layer.color),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&layer.distance),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: counters.as_entire_binding(),
                },
            ],
        });
        Targets {
            size,
            _color: color,
            _distance: distance,
            layer,
            params,
            bind,
            counters,
        }
    }
    pub(crate) fn render(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        scene: &wgpu::BindGroup,
        view: &wgpu::BindGroup,
        split: bool,
        timestamp: Option<(&wgpu::QuerySet, u32)>,
    ) -> FarGrassLayer {
        let mut targets = self.targets.lock().unwrap();
        if !targets[index]
            .as_ref()
            .is_some_and(|target| target.size == size)
        {
            targets[index] = Some(self.target(gpu, size));
        }
        let target = targets[index].as_ref().unwrap();
        let mut params = self.base;
        params.dims[2] = size.0.div_ceil(2);
        params.dims[3] = size.1.div_ceil(2);
        params.full[0] = size.0;
        params.full[1] = size.1;
        params.range[0] = if split { 20.0 } else { 40.0 };
        gpu.queue
            .write_buffer(&target.params, 0, bytemuck::bytes_of(&params));
        encoder.clear_buffer(&target.counters, 0, None);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("far grass integrate"),
            timestamp_writes: timestamp.map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            }),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, scene, &[]);
        pass.set_bind_group(1, view, &[]);
        pass.set_bind_group(2, &target.bind, &[]);
        pass.dispatch_workgroups(params.dims[2].div_ceil(8), params.dims[3].div_ceil(8), 1);
        target.layer.clone()
    }
    /// Explicit offline readback only; normal rendering never maps/waits.
    /// Actual half-resolution dimensions; tightly packed little-endian
    /// RGBA16F contains premultiplied linear radiance and alpha. Near/far
    /// handoff is not applied here; the outer 160..240 m fade is included.
    pub(crate) fn layers_rgba16(&self, gpu: &Gpu) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
        let targets = self.targets.lock().unwrap();
        let mut result = Vec::new();
        for target in targets.iter().filter_map(Option::as_ref) {
            let (w, h) = (target.size.0.div_ceil(2), target.size.1.div_ceil(2));
            let row = (w * 8).div_ceil(256) * 256;
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("far grass raw layer readback"),
                size: u64::from(row) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target._color.as_image_copy(),
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
                .map_err(|e| format!("far grass raw layer wait: {e:?}"))?;
            receive
                .try_recv()
                .map_err(|e| format!("far grass raw layer callback: {e}"))?
                .map_err(|e| format!("far grass raw layer mapping: {e:?}"))?;
            let packed = {
                let bytes = read
                    .slice(..)
                    .get_mapped_range()
                    .map_err(|e| format!("far grass raw layer range: {e:?}"))?;
                let mut packed = Vec::with_capacity(w as usize * h as usize * 8);
                for y in 0..h {
                    let start = y as usize * row as usize;
                    packed.extend_from_slice(&bytes[start..start + w as usize * 8]);
                }
                packed
            };
            read.unmap();
            result.push((w, h, packed));
        }
        Ok(result)
    }
    /// Explicit offline readback only; normal rendering never maps/waits.
    pub(crate) fn diagnostics(&self, gpu: &Gpu) -> Result<Vec<FarGrassDiagnostics>, String> {
        let targets = self.targets.lock().unwrap();
        let targets: Vec<_> = targets.iter().filter_map(Option::as_ref).collect();
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("far grass diagnostics readback"),
            size: targets.len() as u64 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for (i, target) in targets.iter().enumerate() {
            encoder.copy_buffer_to_buffer(&target.counters, 0, &read, i as u64 * 16, 16);
        }
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
            .map_err(|e| format!("far grass diagnostics wait: {e:?}"))?;
        receive
            .try_recv()
            .map_err(|e| format!("far grass diagnostics callback: {e}"))?
            .map_err(|e| format!("far grass diagnostics mapping: {e:?}"))?;
        let result = {
            let bytes = read
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("far grass diagnostics range: {e:?}"))?;
            bytes
                .chunks_exact(16)
                .map(|b| {
                    let v = |i| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
                    FarGrassDiagnostics {
                        traced_rays: v(0),
                        overflow_rays: v(4),
                        max_iterations: v(8),
                    }
                })
                .collect()
        };
        read.unmap();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn field() -> FarGrassField {
        FarGrassField {
            origin: [-4.0, -8.0],
            cell: 2.0,
            width: 3,
            height: 2,
            heights: vec![1.0, 2.0, 5.0, 3.0, 4.0, 6.0],
            road: vec![3.0; 6],
            coverage: vec![255, 0],
            profile: [1.0, 2.0, 3.0, 0.0],
            max_height: 0.65,
            albedo: [0.1, 0.2, 0.05],
        }
    }
    #[test]
    fn source_shader_and_parameter_layout_validate() {
        assert_eq!(std::mem::size_of::<Params>(), 112);
        let source = shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    }
    #[test]
    fn minmax_excludes_uninhabited_cells_and_fully_blocked_road() {
        let mut f = field();
        let (terrain, tiles, params) = prepare(&f).unwrap();
        assert_eq!(terrain[3], [3.0, 3.0]);
        assert_eq!(tiles, vec![[1.0, 4.65]]);
        assert_eq!(params.bounds[..2], [1.0, 4.65]);
        // A mixed road cell remains conservative: exact triangle clipping is
        // performed in the GPU, never by deleting all roadside coverage.
        f.road = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        assert_eq!(prepare(&f).unwrap().1, tiles);
        f.road[4] = 0.8;
        let (_, tiles, params) = prepare(&f).unwrap();
        assert!(tiles[0][0] > tiles[0][1]);
        assert!(params.bounds[0] > params.bounds[1]);
    }
    #[test]
    fn invalid_optics_and_mismatched_grid_are_rejected() {
        let mut f = field();
        f.profile[1] = f32::NAN;
        assert!(prepare(&f).is_err());
        let mut f = field();
        f.profile[0] = -1.0;
        assert!(prepare(&f).is_err());
        let mut f = field();
        f.coverage.pop();
        assert!(prepare(&f).is_err());
        let mut f = field();
        f.road[0] = f32::INFINITY;
        assert!(prepare(&f).is_err());
        let mut f = field();
        f.width = 1;
        assert!(prepare(&f).is_err());
    }
}
