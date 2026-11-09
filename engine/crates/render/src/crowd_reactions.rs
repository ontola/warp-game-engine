use super::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::sync::mpsc;
use wgpu::util::DeviceExt;

pub(super) const REACTION_BYTES: u32 = 40;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Stimulus {
    a: [f32; 4],
    b: [f32; 4],
    tag: [u32; 4],
    mood: [f32; 4],
}
impl Stimulus {
    pub(super) fn new(
        kind: u32,
        from: Vec3,
        to: Vec3,
        radius: f32,
        time: f32,
        id: u64,
        mood: [f32; 2],
        scare: f32,
    ) -> Self {
        Self {
            a: from.extend(time).into(),
            b: to.extend(radius).into(),
            tag: [kind, id as u32, (id >> 32) as u32, 0],
            mood: [mood[0], mood[1], scare, 0.],
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Contact {
    seat: u32,
    event: u32,
    pad: [u32; 2],
    position: [f32; 4],
}

/// One confirmed shot or explosion, with the actual posed contacts at the event tick.
pub struct CrowdHits {
    pub time: f32,
    pub radius: f32,
    pub origin: Vec3,
    pub direction: Vec3,
    pub positions: Vec<Vec3>,
    /// Seat index for each contact position, in the same order.
    pub seats: Vec<u32>,
}
pub(super) struct Receipt {
    buffer: wgpu::Buffer,
    ready: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    events: Vec<Stimulus>,
}
impl Receipt {
    pub(super) fn collect(&self, alive: &mut [u32]) -> Result<Option<Vec<CrowdHits>>, String> {
        match self.ready.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(e) => return Err(e.to_string()),
            Ok(r) => r.map_err(|e| e.to_string())?,
        }
        let bytes = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let count = u32::from_ne_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let mut hits: Vec<Contact> = bytemuck::pod_collect_to_vec(&bytes[16..16 + count * 32]);
        hits.sort_unstable_by_key(|h| (h.event, h.seat));
        let mut groups: Vec<_> = self
            .events
            .iter()
            .map(|e| CrowdHits {
                time: e.a[3],
                radius: e.b[3],
                origin:Vec3::from_slice(&e.a),
                direction:(Vec3::from_slice(&e.b)-Vec3::from_slice(&e.a)).normalize_or_zero(),
                positions: Vec::new(),
                seats: Vec::new(),
            })
            .collect();
        for h in hits {
            alive[h.seat as usize] = 0;
            groups[h.event as usize].seats.push(h.seat);
            groups[h.event as usize]
                .positions
                .push(Vec3::from_slice(&h.position));
        }
        drop(bytes);
        self.buffer.unmap();
        groups.retain(|g| !g.positions.is_empty());
        Ok(Some(groups))
    }
}
pub(super) struct Reactions {
    pub state: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
}
pub(super) const REACT: &str = concat!(
    include_str!("shaders/crowd_displacement.wgsl"), "\n",
    include_str!("shaders/crowd_pose.wgsl"),
    "\n",
    include_str!("shaders/crowd_react.wgsl")
);
impl Reactions {
    pub fn new(
        gpu: &Gpu,
        capacity: usize,
        uniform: &wgpu::Buffer,
        seats: &wgpu::Buffer,
        alive: &wgpu::Buffer,
    ) -> Self {
        let state = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crowd reaction state"),
            size: capacity.max(1) as u64 * u64::from(REACTION_BYTES),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("crowd contacts"),
                source: wgpu::ShaderSource::Wgsl(REACT.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("crowd contacts"),
                layout: None,
                module: &module,
                entry_point: Some("react"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crowd contact state"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[(0, uniform), (1, seats), (2, alive), (4, &state)].map(|(binding, b)| {
                wgpu::BindGroupEntry {
                    binding,
                    resource: b.as_entire_binding(),
                }
            }),
        });
        Self {
            state,
            pipeline,
            bind,
        }
    }
    pub(super) fn dispatch(
        &self,
        gpu: &Gpu,
        reset: bool,
        events: &[Stimulus],
        count: usize,
    ) -> Vec<Receipt> {
        if !reset && events.is_empty() {
            return Vec::new();
        }
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("crowd event ticks"),
            });
        if reset {
            enc.clear_buffer(&self.state, 0, None);
        }
        let mut reads = Vec::new();
        for chunk in events.chunks(16) {
            let max_hits = if chunk.iter().any(|e| e.tag[0] == 2) {
                count
            } else {
                chunk.len().min(count)
            };
            let size = 16 + 32 * max_hits.max(1) as u64;
            let buf = |label, usage| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                })
            };
            let output = buf(
                "crowd confirmed contacts",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );
            let input = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("crowd stimuli"),
                    contents: bytemuck::cast_slice(chunk),
                    usage: wgpu::BufferUsages::STORAGE,
                });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[&input, &output]
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: b.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("crowd contacts and reactions"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.set_bind_group(1, &bind, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            if chunk.iter().any(|e| e.tag[0] != 0) {
                let read = buf(
                    "crowd FX notifications",
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                );
                enc.copy_buffer_to_buffer(&output, 0, &read, 0, size);
                reads.push((read, chunk.to_vec()));
            }
        }
        gpu.queue.submit([enc.finish()]);
        reads
            .into_iter()
            .map(|(buffer, events)| {
                let (tx, ready) = mpsc::channel();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx.send(r);
                });
                Receipt {
                    buffer,
                    ready,
                    events,
                }
            })
            .collect()
    }
}
