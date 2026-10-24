//! Bounded validation renderer for source-derived fractional foliage volumes.
//!
//! Integrates all overlapping crown densities together, clipped by opaque
//! scene depth. Crowns write C/T, never a false opaque first-hit depth. Current
//! 256-part limit is deliberate, not full-forest streaming support. Optional
//! per-view projected mip selection shares the same immutable asset pyramid.
use crate::{Gpu, Scene};
use bytemuck::{Pod, Zeroable};
use std::{sync::Mutex, time::Duration};
use wgpu::util::DeviceExt;

const TILE: u32 = 16;
const CANDIDATES: u32 = 64;
const COUNTER_BYTES: u64 = 48;
// Volume shading only needs owner transforms and shared grass/headlight inputs.
// Reusing the entire mesh scene layout also counts unused mesh/car buffers
// against WebGPU's per-stage limit (10 scene + 7 volume = 17). Keep this
// interface small instead of increasing the requested hardware limit.
pub(crate) const SCENE_BINDINGS: &[u32] = &[4, 13, 14, 15, 16, 17, 18];
pub(crate) const MAX_VIEW_GROUPS: usize = 256;
pub(crate) fn default_selection() -> [[u32; 4]; MAX_VIEW_GROUPS] {
    [[1, u32::MAX, 0, 0]; MAX_VIEW_GROUPS]
}
#[derive(Clone, Debug, Default)]
pub struct VoxelViewDiagnostics {
    pub candidate_overflow_tiles: u32,
    pub step_overflow_rays: u32,
    pub traced_rays: u32,
    pub max_steps_used: u32,
    pub max_tile_candidates: u32,
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
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Config {
    size_count: [u32; 4],
    march: [f32; 4],
    dispatch: [u32; 4],
}
struct Targets {
    size: (u32, u32),
    layer: wgpu::Texture,
    bind: wgpu::BindGroup,
    composite: wgpu::BindGroup,
    counters: wgpu::Buffer,
    jobs: wgpu::Buffer,
    indirect: wgpu::Buffer,
}
pub(crate) struct VoxelRenderer {
    tiles: wgpu::ComputePipeline,
    clear: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    data_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    data: [wgpu::Buffer; 4],
    count: u32,
    step: f32,
    empty_skip: bool,
    lod_pixels: f32,
    active_tiles: bool,
    owners: Vec<u32>,
    targets: Mutex<Vec<Option<Targets>>>,
}
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}",
        crate::wind::shade_source(),
        include_str!("shaders/voxel_forest.wgsl")
    )
}

/// Prove the occupancy implication used by the shader's empty-space skip.
/// Arbitrary externally constructed levels may not form the baker's pyramid;
/// those assets still render through the unchanged fine sampling path.
fn valid_empty_skip_hierarchy(forest: &crate::foliage_volume_data::GpuForest) -> bool {
    valid_skip_levels(forest, false)
}
fn valid_skip_levels(forest: &crate::foliage_volume_data::GpuForest, all_levels: bool) -> bool {
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
        let index = |level: &crate::foliage_volume_data::GpuLevel, x: u32, y: u32, z: u32| {
            let offset = u64::from(level.dims_offset[3])
                + u64::from(x)
                + u64::from(level.dims_offset[0])
                    * (u64::from(y) + u64::from(level.dims_offset[1]) * u64::from(z));
            usize::try_from(offset)
                .ok()
                .and_then(|offset| forest.indices.get(offset))
                .copied()
        };
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
    result
}

impl VoxelRenderer {
    pub(crate) fn new(
        gpu: &Gpu,
        scene: &Scene,
        scene_layout: &wgpu::BindGroupLayout,
        volume_view_layout: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        let forest = scene.voxel_forest.as_ref()?;
        if forest.instances.is_empty() {
            return None;
        }
        assert!(
            forest.instances.len() <= 256,
            "bounded voxel renderer supports at most256 trees"
        );
        let upload = |name, data: &[u8]| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(name),
                    contents: if data.is_empty() { &[0; 112] } else { data },
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let data = [
            upload("voxel levels", bytemuck::cast_slice(&forest.levels)),
            upload("voxel indices", bytemuck::cast_slice(&forest.indices)),
            upload("voxel cells", bytemuck::cast_slice(&forest.cells)),
            gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxel trees"),
                contents: bytemuck::cast_slice(&forest.instances),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
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
        if lod_pixels > 0.0 {
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
        let empty_skip = if lod_pixels > 0.0 { valid_skip_levels(forest, true) }
            else { valid_empty_skip_hierarchy(forest) };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
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
        let data_layout = layout(
            "voxel data",
            &[
                (0, storage(true)),
                (1, storage(true)),
                (2, storage(true)),
                (3, storage(true)),
                (4, storage(false)),
                (5, storage(false)),
                (
                    6,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                (7, uniform),
                (8, storage(false)),
                (9, uniform),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
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
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("voxel forest"),
                source: wgpu::ShaderSource::Wgsl(shader_source().into()),
            });
        let compute = |entry| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let tiles = compute("voxel_tiles");
        let clear = compute("voxel_clear");
        let integrate = compute(if scene.voxel_active_tiles {
            "voxel_integrate_active"
        } else {
            "voxel_integrate"
        });
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
                cache: None,
            });
        Some(Self {
            tiles,
            clear,
            integrate,
            composite,
            data_layout,
            composite_layout,
            data,
            count: forest.instances.len() as u32,
            step,
            empty_skip,
            lod_pixels,
            active_tiles: scene.voxel_active_tiles,
            owners: forest.instances.iter().map(|t| t.meta[0]).collect(),
            targets: Mutex::new(Vec::new()),
        })
    }
    /// Shared rigid-part compute writes poses before volume tile/bin passes.
    /// Asset grids and cells remain immutable; the 96-byte tree layout is unchanged.
    pub(crate) fn tree_buffer(&self) -> &wgpu::Buffer { &self.data[3] }
    pub(crate) fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        self.owners
            .iter()
            .any(|&owner| (first..first + count).contains(&(owner as usize)))
    }
    pub(crate) fn set_view_count(&self, count: usize) {
        let mut targets = self.targets.lock().unwrap();
        targets.truncate(count);
        targets.resize_with(count, || None);
    }
    fn targets(&self, gpu: &Gpu, size: (u32, u32), selection: &wgpu::Buffer) -> Targets {
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
        let tile_count = u64::from(size.0.div_ceil(TILE)) * u64::from(size.1.div_ceil(TILE));
        let tiles = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel tile candidates"),
            size: tile_count * u64::from(CANDIDATES + 1) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let counters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel diagnostics"),
            size: COUNTER_BYTES,
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
            size: (4 + tile_count) * 4,
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
                    march: [self.step, 1024.0, u32::from(self.empty_skip) as f32, self.lod_pixels],
                    dispatch: [u32::from(self.active_tiles), 0, 0, 0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxel data"),
            layout: &self.data_layout,
            entries: &[
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
            ],
        });
        let composite = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxel composite"),
            layout: &self.composite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&layer_view),
            }],
        });
        Targets {
            size,
            layer,
            bind,
            composite,
            counters,
            jobs,
            indirect,
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
        hdr: &wgpu::TextureView,
        selection: &wgpu::Buffer,
        times: [Option<(&wgpu::QuerySet, u32)>; 3],
    ) {
        let mut targets = self.targets.lock().unwrap();
        if !targets[index].as_ref().is_some_and(|t| t.size == size) {
            targets[index] = Some(self.targets(gpu, size, selection));
        }
        let target = targets[index].as_ref().unwrap();
        encoder.clear_buffer(&target.counters, 0, None);
        if self.active_tiles {
            encoder.clear_buffer(&target.jobs, 0, Some(16));
        }
        for (stage, pipeline) in [&self.tiles, &self.integrate].into_iter().enumerate() {
            if stage == 1 && self.active_tiles {
                encoder.copy_buffer_to_buffer(&target.jobs, 0, &target.indirect, 0, 12);
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(if stage == 0 {
                    "voxel tiles"
                } else {
                    "voxel integrate"
                }),
                timestamp_writes: times[stage].map(|(query_set, i)| {
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
            if stage == 0 && self.active_tiles {
                // Count the necessary clear in the tile-pass GPU timestamp.
                // This entry point never reaches the large per-ray arrays.
                pass.set_pipeline(&self.clear);
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
            pass.set_pipeline(pipeline);
            let dims = if stage == 0 {
                (size.0.div_ceil(TILE), size.1.div_ceil(TILE))
            } else {
                size
            };
            if stage == 1 && self.active_tiles {
                pass.dispatch_workgroups_indirect(&target.indirect, 0);
            } else {
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
            timestamp_writes: times[2].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
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
    }
    /// Explicit offline readback only. Normal frames never map or wait here.
    pub(crate) fn diagnostics(&self, gpu: &Gpu) -> Result<Vec<VoxelViewDiagnostics>, String> {
        let targets = self.targets.lock().unwrap();
        let targets: Vec<_> = targets.iter().filter_map(Option::as_ref).collect();
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let size = COUNTER_BYTES * targets.len() as u64;
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
                i as u64 * COUNTER_BYTES,
                COUNTER_BYTES,
            );
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
                .chunks_exact(COUNTER_BYTES as usize)
                .map(|bytes| {
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
        let targets = self.targets.lock().unwrap();
        let mut result = Vec::new();
        for target in targets.iter().filter_map(Option::as_ref) {
            let (w, h) = target.size;
            let row = (w * 8).div_ceil(256) * 256;
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("voxel raw alpha readback"),
                size: u64::from(row) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.layer.as_image_copy(),
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
}

#[cfg(test)]
mod tests {
    #[test]
    fn volume_entry_points_fit_reduced_scene_binding_contract() {
        let source = super::shader_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all()).validate(&module).unwrap();
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
            // One owner transform buffer + seven bounded volume buffers.
            assert!(storage_count <= 8, "{} uses {storage_count} storage buffers", entry.name);
        }
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
        assert_eq!(std::mem::size_of::<super::Config>(), 48);
        assert_eq!(std::mem::offset_of!(super::Config, dispatch), 32);
        assert!(super::COUNTER_BYTES >= 12 * 4);
        assert_eq!(3840u64 * 2160 * 8, 66_355_200);
        let candidates = 3840u64.div_ceil(16) * 2160u64.div_ceil(16) * 65 * 4;
        assert_eq!(candidates, 8_424_000);
        let jobs = (3840u64.div_ceil(16) * 2160u64.div_ceil(16) + 4) * 4 + 12;
        assert_eq!(jobs, 129_628);
    }
}
