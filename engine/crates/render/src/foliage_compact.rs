//! Stable GPU owner-list compaction, independent of a vegetation predicate.
//!
//! Selection writes each input's local inclusive rank (zero means rejected)
//! and one count per 256-owner block. Finalization scans the block counts and
//! scatters the surviving original owner IDs into a same-capacity list. Draw
//! order is unchanged, including equal-depth ties. No atomics, overflow pool,
//! per-frame allocation or CPU readback. One list belongs to one view/range;
//! it is reusable across viewport resizes because capacity is instance-based.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const BLOCK_SIZE: u32 = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct OwnerRange {
    pub first: u32,
    pub count: u32,
    pub vertices: u32,
    blocks: u32,
}
impl OwnerRange {
    pub fn new(first: u32, count: u32, vertices: u32) -> Result<Self, String> {
        first
            .checked_add(count)
            .ok_or("foliage owner range exceeds u32")?;
        Ok(Self {
            first,
            count,
            vertices,
            blocks: count.div_ceil(BLOCK_SIZE),
        })
    }
    pub fn blocks(self) -> u32 {
        self.blocks
    }
    pub fn bytes(self) -> u64 {
        32 + u64::from(self.count.max(1)) * 8 + u64::from(self.blocks.max(1)) * 4
    }
    pub fn validate(self, limits: &wgpu::Limits) -> Result<(), String> {
        if self.blocks != self.count.div_ceil(BLOCK_SIZE)
            || self.first.checked_add(self.count).is_none()
        {
            return Err("invalid foliage compact range".into());
        }
        let largest = (u64::from(self.count.max(1)) * 4).max(16);
        if largest > u64::from(limits.max_storage_buffer_binding_size)
            || largest > limits.max_buffer_size
            || self.blocks > limits.max_compute_workgroups_per_dimension
            || limits.max_compute_invocations_per_workgroup < BLOCK_SIZE
            || limits.max_compute_workgroup_size_x < BLOCK_SIZE
            || limits.max_compute_workgroup_storage_size < BLOCK_SIZE * 4 + 4
        {
            return Err("foliage compact buffers/workgroups exceed GPU limits".into());
        }
        Ok(())
    }
}

pub struct OwnerList {
    range: OwnerRange,
    /// Read-only binding 3 for a vertex shader: array of original scene IDs.
    pub draw: wgpu::BindGroup,
    /// Selection bindings: uniform/ranks/block totals/owners/indirect arguments.
    pub selection: wgpu::BindGroup,
    pub owners: wgpu::Buffer,
    pub indirect: wgpu::Buffer,
}
impl OwnerList {
    pub fn range(&self) -> OwnerRange {
        self.range
    }
}
pub struct Compactor {
    pub selection_layout: wgpu::BindGroupLayout,
    pub draw_layout: wgpu::BindGroupLayout,
    offsets: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
}

pub fn shader_source() -> &'static str {
    include_str!("shaders/foliage_compact.wgsl")
}
/// Append at bind group 3 in an application selector. It must call compact_rank
/// uniformly for all 256 lanes; rejected/out-of-range lanes contribute zero.
pub fn selection_source() -> String {
    shader_source().replace("@group(0)", "@group(3)")
}

impl Compactor {
    pub fn new(gpu: &Gpu) -> Self {
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let selection_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("foliage compact work"),
                    entries: &(0..5)
                        .map(|binding| wgpu::BindGroupLayoutEntry {
                            binding,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: if binding == 0 {
                                wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Uniform,
                                    has_dynamic_offset: false,
                                    min_binding_size: None,
                                }
                            } else {
                                storage(false)
                            },
                            count: None,
                        })
                        .collect::<Vec<_>>(),
                });
        let draw_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("foliage compact owners"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: storage(true),
                    count: None,
                }],
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("foliage compact finalize"),
                bind_group_layouts: &[Some(&selection_layout)],
                immediate_size: 0,
            });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("foliage compact"),
                source: wgpu::ShaderSource::Wgsl(shader_source().into()),
            });
        let pipeline = |entry| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        Self {
            offsets: pipeline("compact_offsets"),
            scatter: pipeline("compact_scatter"),
            selection_layout,
            draw_layout,
        }
    }
    pub fn list(&self, gpu: &Gpu, range: OwnerRange) -> Result<OwnerList, String> {
        range.validate(&gpu.limits)?;
        let params = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("foliage compact range"),
                contents: bytemuck::bytes_of(&range),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let storage = |label, bytes, extra| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | extra,
                mapped_at_creation: false,
            })
        };
        let ranks = storage(
            "foliage local ranks",
            u64::from(range.count.max(1)) * 4,
            wgpu::BufferUsages::empty(),
        );
        let blocks = storage(
            "foliage block offsets",
            u64::from(range.blocks.max(1)) * 4,
            wgpu::BufferUsages::empty(),
        );
        let owners = storage(
            "foliage compact owner IDs",
            u64::from(range.count.max(1)) * 4,
            wgpu::BufferUsages::COPY_SRC,
        );
        let indirect = storage(
            "foliage compact indirect draw",
            16,
            wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
        );
        let selection = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("foliage compact work"),
            layout: &self.selection_layout,
            entries: &[&params, &ranks, &blocks, &owners, &indirect]
                .into_iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        let draw = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("foliage compact owners"),
            layout: &self.draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 3,
                resource: owners.as_entire_binding(),
            }],
        });
        Ok(OwnerList {
            range,
            draw,
            selection,
            owners,
            indirect,
        })
    }
    /// Call after the selector has written every local rank/block count. Even
    /// an empty range resets the indirect count, so old visible IDs cannot leak.
    pub fn finish(&self, encoder: &mut wgpu::CommandEncoder, list: &OwnerList) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("foliage compact finalize"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &list.selection, &[]);
        pass.set_pipeline(&self.offsets);
        pass.dispatch_workgroups(1, 1, 1);
        if list.range.blocks > 0 {
            pass.set_pipeline(&self.scatter);
            pass.dispatch_workgroups(list.range.blocks, 1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROBE_SELECTOR: &str = r#"
@group(1) @binding(0) var<uniform> probe_mode: vec4<u32>;
@compute @workgroup_size(256)
fn probe_select(@builtin(global_invocation_id) id:vec3<u32>,
    @builtin(local_invocation_index) lane:u32,@builtin(workgroup_id) block:vec3<u32>) {
    let keep=id.x<compact_params.count && (probe_mode.x==1u ||
        (probe_mode.x==2u && (id.x%7u==0u || id.x%13u==2u)));
    let rank=compact_rank(lane,select(0u,1u,keep));
    if id.x<compact_params.count {compact_ranks[id.x]=select(0u,rank,keep);}
    if lane==255u {compact_blocks[block.x]=rank;}
}
"#;
    #[test]
    fn capacity_covers_every_owner_with_bounded_per_view_storage() {
        for count in [0, 1, 255, 256, 257, 70_000, 115_000, 250_000, 460_000] {
            let range = OwnerRange::new(900, count, 18).unwrap();
            range.validate(&wgpu::Limits::default()).unwrap();
            assert_eq!(range.blocks(), count.div_ceil(BLOCK_SIZE));
            assert!(
                range.bytes()
                    <= u64::from(count) * 8 + u64::from(count.div_ceil(BLOCK_SIZE)) * 4 + 44
            );
        }
        assert!(OwnerRange::new(u32::MAX, 1, 18).is_err());
        let range = OwnerRange::new(0, 115_000, 18).unwrap();
        assert!(4 * range.bytes() < 4 * 1024 * 1024);
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 100,
            ..Default::default()
        };
        assert!(range.validate(&limits).is_err());
        let limits = wgpu::Limits {
            max_compute_workgroups_per_dimension: 1,
            ..Default::default()
        };
        assert!(range.validate(&limits).is_err());
        assert_eq!(std::mem::size_of::<OwnerRange>(), 16);
    }
    #[test]
    fn compaction_shader_validates() {
        let src = format!("{}\n{}", shader_source(), PROBE_SELECTOR);
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    }

    /// Isolated GPU validation, never part of ordinary CPU `cargo test`.
    /// Run only after stopping the playtest: cargo test -p render --lib
    /// foliage_compact::tests::gpu_lists_match_serial_filter -- --ignored --nocapture
    #[test]
    #[ignore = "requires exclusive GPU validation window"]
    fn gpu_lists_match_serial_filter() {
        let gpu = Gpu::new().unwrap();
        let compact = Compactor::new(&gpu);
        let mode_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("compact probe mode"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let modes: Vec<_> = (0u32..3)
            .map(|mode| {
                let uniform = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("compact probe mode"),
                        contents: bytemuck::cast_slice(&[mode, 0, 0, 0]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("compact probe mode"),
                    layout: &mode_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                })
            })
            .collect();
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("compact probe"),
                bind_group_layouts: &[Some(&compact.selection_layout), Some(&mode_layout)],
                immediate_size: 0,
            });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("compact probe"),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}\n{}", shader_source(), PROBE_SELECTOR).into(),
                ),
            });
        let selector = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("compact probe"),
                layout: Some(&layout),
                module: &shader,
                entry_point: Some("probe_select"),
                compilation_options: Default::default(),
                cache: None,
            });
        for count in [0u32, 1, 255, 256, 257, 70_000, 115_000, 460_000] {
            let lists: Vec<_> = (0..4)
                .map(|view| {
                    compact
                        .list(&gpu, OwnerRange::new(31 + view * 101, count, 18).unwrap())
                        .unwrap()
                })
                .collect();
            for round in 0..4 {
                let selected: Vec<usize> = (0..4)
                    .map(|view| if round == 3 { 0 } else { (view + round) % 3 })
                    .collect();
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                // Encode all four lists before readback; changed order must not
                // alias another view's owners, counts or reset state.
                for view in (0..4).rev() {
                    let list = &lists[view];
                    if list.range.blocks > 0 {
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&selector);
                        pass.set_bind_group(0, &list.selection, &[]);
                        pass.set_bind_group(1, &modes[selected[view]], &[]);
                        pass.dispatch_workgroups(list.range.blocks, 1, 1);
                    }
                    compact.finish(&mut encoder, list);
                }
                let readbacks: Vec<_> = lists
                    .iter()
                    .map(|list| {
                        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("compact oracle readback"),
                            size: 16 + u64::from(count.max(1)) * 4,
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        encoder.copy_buffer_to_buffer(&list.indirect, 0, &buffer, 0, 16);
                        encoder.copy_buffer_to_buffer(
                            &list.owners,
                            0,
                            &buffer,
                            16,
                            u64::from(count.max(1)) * 4,
                        );
                        buffer
                    })
                    .collect();
                gpu.queue.submit([encoder.finish()]);
                for buffer in &readbacks {
                    buffer
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
                }
                gpu.wait();
                for (view, buffer) in readbacks.iter().enumerate() {
                    let mapped = buffer.slice(..).get_mapped_range().unwrap();
                    let actual: &[u32] = bytemuck::cast_slice(&mapped);
                    let expected: Vec<_> = (0..count)
                        .filter(|i| {
                            selected[view] == 1
                                || (selected[view] == 2 && (i % 7 == 0 || i % 13 == 2))
                        })
                        .map(|i| lists[view].range.first + i)
                        .collect();
                    assert_eq!(
                        &actual[..4],
                        &[18, expected.len() as u32, 0, 0],
                        "count {count}, view {view}, round {round}"
                    );
                    assert_eq!(
                        &actual[4..4 + expected.len()],
                        expected.as_slice(),
                        "count {count}, view {view}, round {round}"
                    );
                }
            }
            println!("stable compact {count} slots x 4 views: full/empty/mixed/reused PASS");
        }
    }
}
