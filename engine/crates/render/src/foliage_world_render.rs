//! World-scale consumer of shared optical plant assets. No per-tree boxes or
//! triangles are rasterized; disjoint grid segments bound the ray candidate
//! set. Native-only experimental backend until motion/optical gates pass.
use crate::{Gpu, foliage_world::WorldField};
use bytemuck::{Pod, Zeroable};
use std::{sync::Mutex, time::Duration};
use wgpu::util::DeviceExt;

const STATS_BYTES: u64 = 80;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Config {
    size_count: [u32; 4],
    march: [f32; 4],
    world_min: [f32; 4],
    world_max: [f32; 4],
    selection: [u32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct WorldFoliageDiagnostics {
    pub view: usize,
    pub groups: u32,
    pub selected_groups: u32,
    pub source_owners_suppressed: u32,
    pub source_owners: u32,
    pub shared_bytes: u64,
    pub target_bytes: u64,
    pub traced_rays: u32,
    pub failed_rays: u32,
    pub max_cell_references: u32,
    pub max_ray_hits: u32,
    pub complete_cell_fallbacks: u32,
    pub max_iterations: u32,
    pub grid_segments: u64,
    pub iterations: u64,
    pub interval_tests: u64,
    pub volume_samples: u64,
    pub occupied_cell_reads: u64,
    pub empty_jumps: u64,
}
struct Target {
    size: (u32, u32),
    layer: wgpu::Texture,
    layer_view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    composite: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    counters: wgpu::Buffer,
    selection: wgpu::Buffer,
    members: wgpu::Buffer,
}
pub(crate) struct WorldRenderer {
    data: [wgpu::Buffer; 5],
    data_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    integrate: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    config: Config,
    group_counts: Vec<u32>,
    shared_bytes: u64,
    targets: Mutex<Vec<Option<Target>>>,
    owners: Vec<u32>,
    pub(crate) max_wind_shear: f32,
}

/// Reuse the existing fractional transport, authored normal lobes, source
/// material response and current sun/terrain visibility. Only the scheduling
/// and pose adapter differ. Marker checks make accidental library drift fail.
pub(crate) fn shader_source() -> Result<String, String> {
    let source = crate::shaders::Snapshot::load()?;
    let forest = source.source("voxel_forest.wgsl");
    let start = forest
        .find("fn volume_interval(")
        .ok_or("volume helper start missing")?;
    let end = forest.find("// SHARED_VOLUME_HELPERS_END")
        .ok_or("volume helper end missing")?;
    let mut helpers = forest[start..end].to_owned();
    for (from, to) in [
        ("volume_trees[index]", "world_tree(index)"),
        (
            "volume_trees[ray.identity>>12u]",
            "world_tree(ray.identity>>12u)",
        ),
        (
            "min(tree.identity.z-1u,volume_selection_at(ray.identity>>12u).y)",
            "tree.identity.z-1u",
        ),
        // This consumer uses the legacy footprint-selected mip chain, not
        // aggregate candidate decisions. Specialize the shared exact-level
        // branches to the explicit legacy sentinel without adding a binding.
        (
            "volume_selection_at(ray.identity>>12u)",
            "vec4<u32>(1u,0xffffffffu,0u,0u)",
        ),
        // The world-space wind shear has norm <= 1+|shear|. Keep the footprint
        // conservative without rebuilding instance matrices on the CPU.
        (
            "let stretch=min(frobenius,sqrt(norm_inf*norm_one));",
            "let stretch=min(frobenius,sqrt(norm_inf*norm_one))*(1.0+length(wind_coefficients[tree.identity.x].xy));",
        ),
    ] {
        helpers = helpers.replace(from, to);
    }
    // Replacement scope ALL_MATERIALS is not a shading-model tag. These are
    // the current source tree's leaf/bark constants. Mixed cells remain an
    // optical approximation; no brightness factor is fitted to screenshots.
    for (from, to) in [
        (
            "let procedural=tree.identity.w==0xffffffffu;",
            "let procedural=false;\n    let leaf=clamp(sample.leaf_fraction,0.0,1.0);",
        ),
        (
            "if !procedural {base*=0.82+0.3*hash2(vec2<f32>(owner.r0.w,owner.r2.w));}",
            "base*=mix(1.0,0.82+0.3*hash2(vec2<f32>(owner.r0.w,owner.r2.w)),leaf);",
        ),
        ("vec3<f32>(0.01)", "vec3<f32>(mix(0.04,0.01,leaf))"),
        ("))*0.85;", "))*mix(1.0,0.85,leaf);"),
        ("max(-ndl,0.0);", "max(-ndl,0.0)*leaf;"),
        ("))*0.85*weight;", "))*mix(1.0,0.85,leaf)*weight;"),
    ] {
        helpers = helpers.replace(from, to);
    }
    let query = crate::source_replacement::query_source()
        .replace("spatial_members", "world_members")
        .replace("spatial_owners", "world_selection");
    let body =
        include_str!("shaders/foliage_world.wgsl").replace("// SHARED_VOLUME_HELPERS", &helpers);
    Ok(format!("{}\n{query}\n{body}", source.shade()))
}

impl WorldRenderer {
    pub(crate) fn new(
        gpu: &Gpu,
        field: &WorldField,
        owners: u32,
        scene_layout: &wgpu::BindGroupLayout,
        view_layout: &wgpu::BindGroupLayout,
        lod_pixels: f32,
    ) -> Result<Self, String> {
        if field.forest.instances.is_empty() || field.forest.instances.len() > 1_048_575 {
            return Err("world foliage requires 1..1048575 source owners".into());
        }
        // Recheck public, potentially edited WorldField data before upload
        // and readiness publication. No unchecked coarse empty-space proof.
        if !crate::voxel_forest::valid_skip_levels(&field.forest, true) {
            return Err("world foliage requires a complete dyadic occupancy hierarchy for empty-space traversal".into());
        }
        if !lod_pixels.is_finite() || lod_pixels <= 0.0 {
            return Err("invalid world foliage pixel budget".into());
        }
        let grid = &field.grid;
        let span_offset = 8u32;
        let reference_offset = span_offset + grid.spans.len() as u32 * 2;
        let group_offset = reference_offset + grid.references.len() as u32;
        let mut words = vec![
            grid.origin[0] as u32,
            grid.origin[1] as u32,
            grid.dimensions[0],
            grid.dimensions[1],
            grid.cell_metres.to_bits(),
            span_offset,
            reference_offset,
            group_offset,
        ];
        words.extend(grid.spans.iter().flatten());
        words.extend(&grid.references);
        words.extend(&field.tree_groups);
        let packed=crate::foliage_volume_data::packed_cells(&field.forest.cells);
        let bytes: [&[u8]; 5] = [
            bytemuck::cast_slice(&field.forest.levels),
            bytemuck::cast_slice(&field.forest.indices),
            bytemuck::cast_slice(&packed),
            bytemuck::cast_slice(&field.forest.instances),
            bytemuck::cast_slice(&words),
        ];
        for data in bytes {
            if data.is_empty()
                || data.len() as u64 > u64::from(gpu.limits.max_storage_buffer_binding_size)
            {
                return Err("world foliage provider exceeds device storage buffer budget".into());
            }
        }
        let data = bytes.map(|contents| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("world foliage immutable shared provider"),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        });
        let shared_bytes = data.iter().map(wgpu::Buffer::size).sum();
        let mut lo = glam::Vec3::splat(f32::INFINITY);
        let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
        for tree in &field.forest.instances {
            lo = lo.min(glam::Vec3::from_slice(&tree.bounds_min[..3]));
            hi = hi.max(glam::Vec3::from_slice(&tree.bounds_max[..3]));
        }
        let config = Config {
            size_count: [0, 0, field.groups.len() as u32, owners],
            march: [0.0, 0.0, 0.0, lod_pixels],
            world_min: lo.extend(0.).to_array(),
            world_max: hi.extend(0.).to_array(),
            selection: [owners + 8, owners, 0, 0],
        };
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let data_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("world foliage data"),
                entries: &(0..10)
                    .map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        count: None,
                        ty: match binding {
                            5 => storage(false),
                            6 => wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: wgpu::TextureFormat::Rgba16Float,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            7 => wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            _ => storage(true),
                        },
                    })
                    .collect::<Vec<_>>(),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("world foliage integration"),
                bind_group_layouts: &[Some(scene_layout), Some(view_layout), Some(&data_layout)],
                immediate_size: 0,
            });
        let source = shader_source()?;
        let (integrate, composite_layout, composite) = crate::shaders::checked(gpu, || {
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("world foliage"),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
            let integrate = gpu
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("world foliage grid integrate"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("world_integrate"),
                    compilation_options: Default::default(),
                    cache: gpu.pipeline_cache(),
                });
            let composite_layout =
                gpu.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("world foliage composite"),
                        entries: &[wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            count: None,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                        }],
                    });
            let layout = gpu
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("world foliage composite"),
                    bind_group_layouts: &[Some(&composite_layout)],
                    immediate_size: 0,
                });
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("fractional volume composite"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("shaders/voxel_composite.wgsl").into(),
                    ),
                });
            let composite = gpu
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("world foliage composite"),
                    layout: Some(&layout),
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
                    cache: gpu.pipeline_cache(),
                });
            (integrate, composite_layout, composite)
        })?;
        let mut owners: Vec<_> = field.forest.instances.iter().map(|t| t.meta[0]).collect();
        owners.sort_unstable();
        Ok(Self {
            data,
            data_layout,
            composite_layout,
            integrate,
            composite,
            config,
            shared_bytes,
            owners,
            max_wind_shear: field.max_wind_shear,
            group_counts: field
                .groups
                .iter()
                .map(|g| g.members.len() as u32)
                .collect(),
            targets: Mutex::new(Vec::new()),
        })
    }
    pub(crate) fn set_view_count(&self, count: usize) {
        let mut t = self.targets.lock().unwrap();
        t.truncate(count);
        t.resize_with(count, || None);
    }
    pub(crate) fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        let index = self
            .owners
            .partition_point(|&owner| (owner as usize) < first);
        self.owners
            .get(index)
            .is_some_and(|&owner| (owner as usize) < first.saturating_add(count))
    }
    fn data_bind(
        &self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        counters: &wgpu::Buffer,
        uniform: &wgpu::Buffer,
        selection: &wgpu::Buffer,
        members: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let mut entries: Vec<_> = self
            .data
            .iter()
            .enumerate()
            .map(|(binding, buf)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buf.as_entire_binding(),
            })
            .collect();
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 5,
                resource: counters.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: selection.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: members.as_entire_binding(),
            },
        ]);
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world foliage view provider"),
            layout: &self.data_layout,
            entries: &entries,
        })
    }
    fn target(
        &self,
        gpu: &Gpu,
        size: (u32, u32),
        selection: &wgpu::Buffer,
        members: &wgpu::Buffer,
    ) -> Target {
        let layer = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("world foliage fractional radiance"),
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
        let view = layer.create_view(&Default::default());
        let counters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world foliage work counters"),
            size: STATS_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut config = self.config;
        config.size_count[0] = size.0;
        config.size_count[1] = size.1;
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("world foliage view"),
                contents: bytemuck::bytes_of(&config),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = self.data_bind(gpu, &view, &counters, &uniform, selection, members);
        let composite = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world foliage composite"),
            layout: &self.composite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Target {
            size,
            layer,
            layer_view: view,
            bind,
            composite,
            uniform,
            counters,
            selection: selection.clone(),
            members: members.clone(),
        }
    }
    pub(crate) fn render(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        scene: &wgpu::BindGroup,
        view: &wgpu::BindGroup,
        hdr: &wgpu::TextureView,
        selection: &wgpu::Buffer,
        members: &wgpu::Buffer,
        times: [Option<(&wgpu::QuerySet, u32)>; 2],
    ) {
        let mut targets = self.targets.lock().unwrap();
        if !targets[index]
            .as_ref()
            .is_some_and(|target| target.size == size)
        {
            targets[index] = Some(self.target(gpu, size, selection, members));
        } else {
            let target = targets[index].as_mut().unwrap();
            // Camera scratch may be replaced every frame. Retain expensive
            // targets/config, but never sample or diagnose an older buffer.
            // Buffer equality compares backend identity, not its Rust address.
            if &target.selection != selection || &target.members != members {
                target.bind = self.data_bind(
                    gpu,
                    &target.layer_view,
                    &target.counters,
                    &target.uniform,
                    selection,
                    members,
                );
                target.selection = selection.clone();
                target.members = members.clone();
            }
        }
        let target = targets[index].as_ref().unwrap();
        enc.clear_buffer(&target.counters, 0, None);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("world foliage integrate"),
            timestamp_writes: times[0].map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(i),
                end_of_pass_write_index: Some(i + 1),
            }),
        });
        pass.set_pipeline(&self.integrate);
        pass.set_bind_group(0, scene, &[]);
        pass.set_bind_group(1, view, &[]);
        pass.set_bind_group(2, &target.bind, &[]);
        pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        drop(pass);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world foliage composite"),
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
            timestamp_writes: times[1].map(|(query_set, i)| wgpu::RenderPassTimestampWrites {
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
    pub(crate) fn diagnostics(&self, gpu: &Gpu) -> Result<Vec<WorldFoliageDiagnostics>, String> {
        let targets = self.targets.lock().unwrap();
        let mut out = Vec::new();
        for (view, t) in targets
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().map(|t| (i, t)))
        {
            let raw = read_buffer(gpu, &t.counters, 0, STATS_BYTES)?;
            let word = |i: usize| u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap());
            let wide = |i| u64::from(word(i)) | (u64::from(word(i + 1)) << 32);
            let states = read_buffer(
                gpu,
                &t.selection,
                u64::from(self.config.selection[1]) * 4,
                (u64::from(crate::source_replacement::HEADER_WORDS)
                    + self.group_counts.len() as u64)
                    * 4,
            )?;
            let (selected_groups, source_owners_suppressed) =
                published_selection_counts(&states, self.config.size_count[3], &self.group_counts)?;
            out.push(WorldFoliageDiagnostics {
                view,
                groups: self.group_counts.len() as u32,
                selected_groups,
                source_owners_suppressed,
                source_owners: self.group_counts.iter().sum(),
                shared_bytes: self.shared_bytes,
                target_bytes: u64::from(t.size.0) * u64::from(t.size.1) * 8
                    + STATS_BYTES
                    + t.uniform.size(),
                traced_rays: word(0),
                failed_rays: word(1),
                max_cell_references: word(2),
                max_ray_hits: word(3),
                complete_cell_fallbacks: word(4),
                max_iterations: word(5),
                grid_segments: wide(8),
                iterations: wide(10),
                interval_tests: wide(12),
                volume_samples: wide(14),
                occupied_cell_reads: wide(16),
                empty_jumps: wide(18),
            });
        }
        Ok(out)
    }
    pub(crate) fn layers(&self, gpu: &Gpu) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
        let targets = self.targets.lock().unwrap();
        let mut out = Vec::new();
        for t in targets.iter().filter_map(Option::as_ref) {
            let (w, h) = t.size;
            let row = (w * 8).div_ceil(256) * 256;
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("world foliage layer readback"),
                size: u64::from(row) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                t.layer.as_image_copy(),
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
            let submit = gpu.queue.submit([enc.finish()]);
            let raw = map_buffer(gpu, &read, submit)?;
            let mut packed = Vec::with_capacity((w * h * 8) as usize);
            for r in raw.chunks_exact(row as usize) {
                packed.extend_from_slice(&r[..w as usize * 8]);
            }
            out.push((w, h, packed));
        }
        Ok(out)
    }
}
/// Validate the same published header the GPU source/replacement predicate
/// consumes. A disabled but valid publication retains source despite stale
/// flag bytes. This checks publication consistency, not an external frame ID.
fn published_selection_counts(
    bytes: &[u8],
    owners: u32,
    group_counts: &[u32],
) -> Result<(u32, u32), String> {
    let header = crate::source_replacement::HEADER_WORDS as usize;
    if bytes.len() != (header + group_counts.len()) * 4 {
        return Err("world foliage selection readback length mismatch".into());
    }
    let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    if word(0) != crate::source_replacement::MAGIC
        || word(1) == 0
        || word(1) != word(2)
        || word(3) != group_counts.len() as u32
        || word(4) > 1
        || word(5) != owners
    {
        return Err("world foliage selection publication is invalid or incomplete".into());
    }
    if word(4) == 0 {
        return Ok((0, 0));
    }
    let mut selected = 0;
    let mut suppressed = 0u32;
    for (index, &count) in group_counts.iter().enumerate() {
        if word(header + index) == 1 {
            selected += 1;
            suppressed = suppressed
                .checked_add(count)
                .ok_or("world foliage selected owner count overflow")?;
        }
    }
    if suppressed > owners {
        return Err("world foliage selected owner count exceeds source owners".into());
    }
    Ok((selected, suppressed))
}
fn map_buffer(
    gpu: &Gpu,
    read: &wgpu::Buffer,
    submit: wgpu::SubmissionIndex,
) -> Result<Vec<u8>, String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = send.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submit),
            timeout: Some(Duration::from_secs(60)),
        })
        .map_err(|e| format!("world foliage wait: {e:?}"))?;
    receive
        .try_recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("world foliage map: {e:?}"))?;
    let bytes = read
        .slice(..)
        .get_mapped_range()
        .map_err(|e| format!("world foliage read: {e:?}"))?
        .to_vec();
    read.unmap();
    Ok(bytes)
}
fn read_buffer(
    gpu: &Gpu,
    source: &wgpu::Buffer,
    offset: u64,
    size: u64,
) -> Result<Vec<u8>, String> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("world foliage diagnostics"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(source, offset, &read, 0, size);
    let submit = gpu.queue.submit([enc.finish()]);
    map_buffer(gpu, &read, submit)
}
#[cfg(test)]
mod tests {
    fn publication() -> Vec<u32> {
        vec![
            crate::source_replacement::MAGIC,
            23,
            23,
            3,
            1,
            12,
            0,
            0,
            1,
            0,
            1,
        ]
    }
    #[test]
    fn diagnostics_require_complete_matching_publication() {
        use super::published_selection_counts;
        let valid = publication();
        let groups = [3, 4, 5];
        assert_eq!(
            published_selection_counts(bytemuck::cast_slice(&valid), 12, &groups).unwrap(),
            (2, 8)
        );
        for (index, value) in [(0, 0), (1, 0), (2, 22), (3, 2), (4, 2), (5, 13)] {
            let mut bad = valid.clone();
            bad[index] = value;
            assert!(published_selection_counts(bytemuck::cast_slice(&bad), 12, &groups).is_err());
        }
        assert!(
            published_selection_counts(bytemuck::cast_slice(&valid[..10]), 12, &groups).is_err()
        );
        assert!(published_selection_counts(bytemuck::cast_slice(&valid), 12, &[7, 4, 6]).is_err());
    }
    #[test]
    fn diagnostics_match_gpu_exact_flags_and_disabled_fallback_per_view() {
        use super::published_selection_counts;
        let mut a = publication();
        let mut b = publication();
        let groups = [3, 4, 5];
        // Only the exact state1 hides source; unknown flag2 does not.
        a[10] = 2;
        assert_eq!(
            published_selection_counts(bytemuck::cast_slice(&a), 12, &groups).unwrap(),
            (1, 3)
        );
        b[4] = 0;
        assert_eq!(
            published_selection_counts(bytemuck::cast_slice(&b), 12, &groups).unwrap(),
            (0, 0)
        );
        b[1] = 24;
        b[2] = 24;
        b[4] = 1;
        b[8] = 0;
        b[9] = 1;
        assert_eq!(
            published_selection_counts(bytemuck::cast_slice(&b), 12, &groups).unwrap(),
            (2, 9)
        );
        assert_eq!(
            published_selection_counts(bytemuck::cast_slice(&a), 12, &groups).unwrap(),
            (1, 3)
        );
    }
    #[test]
    fn world_consumer_shader_validates() {
        let source = super::shader_source().unwrap();
        assert!(!source.contains("volume_selection["),
            "legacy world optics must not acquire the aggregate selection ABI");
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
