use super::*;
use crate::frame::resource_cache::BindGroups;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Groom {
    pub fit: [[f32; 4]; 4],
    pub weights: [f32; 4],
    pub tint: [f32; 4],
    /// Owner instance, global head palette index, style, seed.
    pub info: [u32; 4],
}

pub(crate) fn guide_buffer(gpu: &Gpu) -> wgpu::Buffer {
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Warp hair guide surfaces"),
            contents: include_bytes!("../../../../assets/humans/strand-guides.bin"),
            usage: wgpu::BufferUsages::STORAGE,
        })
}

pub(crate) struct PlayerStrands {
    camera: wgpu::Buffer,
    commands: wgpu::Buffer,
    count: u32,
    select: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,
    cull_group: wgpu::BindGroup,
    draw_group: wgpu::BindGroup,
    atmosphere: BindGroups<8>,
    atmosphere_layout: wgpu::BindGroupLayout,
}

impl PlayerStrands {
    pub(crate) fn new(
        gpu: &Gpu,
        descriptors: &[(usize, usize, Groom)],
        actors: &[crate::skinning::Actor],
        owners: &wgpu::Buffer,
        palette: &wgpu::Buffer,
    ) -> Self {
        let mut offsets = Vec::new();
        let mut offset = 0;
        for actor in actors {
            offsets.push(offset);
            offset += actor.palette.len() as u32;
        }
        let data: Vec<_> = descriptors
            .iter()
            .map(|&(actor, head, mut groom)| {
                groom.info[0] = actors[actor].owner;
                groom.info[1] = offsets[actor] + head as u32;
                groom
            })
            .collect();
        let count = data.len() as u32;
        use wgpu::BufferUsages as U;
        let buffer = |label, size, usage| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let camera = buffer("strand camera", 80, U::UNIFORM | U::COPY_DST);
        let commands = buffer("strand draws", 16 * count as u64, U::INDIRECT | U::STORAGE);
        let heads = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fitted hair roots"),
                contents: bytemuck::cast_slice(&data),
                usage: U::STORAGE,
            });
        let guides = guide_buffer(gpu);
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("procedural player hair"),
                source: wgpu::ShaderSource::Wgsl(
                    concat!(
                        include_str!("../shaders/surface_fog.wgsl"),
                        "\n",
                        include_str!("strands.wgsl"),
                        "\n",
                        include_str!("player_strands.wgsl")
                    )
                    .into(),
                ),
            });
        let select = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("choose strand density"),
                layout: None,
                module: &module,
                entry_point: Some("choose"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        let render = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("individual hair fibres"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("player_strand"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: Default::default(),
                multisample: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    depth_write_enabled: Some(false),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("strand_light"),
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
        let group = |layout: &wgpu::BindGroupLayout, last, storage: &wgpu::Buffer| {
            let entries = [
                (0, &camera),
                (1, &heads),
                (2, owners),
                (3, palette),
                (last, storage),
            ]
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            });
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("procedural hair sources"),
                layout,
                entries: &entries,
            })
        };
        let cull_group = group(&select.get_bind_group_layout(0), 4, &commands);
        let draw_group = group(&render.get_bind_group_layout(0), 8, &guides);
        let atmosphere_layout = render.get_bind_group_layout(1);
        Self {
            camera,
            commands,
            count,
            select,
            render,
            cull_group,
            draw_group,
            atmosphere: Default::default(),
            atmosphere_layout,
        }
    }

    pub(crate) fn draw(
        &mut self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        vp: Mat4,
        focal: f32,
        slot: usize,
        time: f32,
        bindings: &[(u32, wgpu::BindingResource)],
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        let mut words = [0u32; 20];
        words[..16].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
        words[16..].copy_from_slice(&[focal.to_bits(), slot as u32, time.to_bits(), self.count]);
        let cam = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hair view"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        enc.copy_buffer_to_buffer(&cam, 0, &self.camera, 0, 80);
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("strand density"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.select);
            pass.set_bind_group(0, &self.cull_group, &[]);
            pass.dispatch_workgroups(self.count.div_ceil(32), 1, 1);
        }
        let atmosphere =
            self.atmosphere
                .get(gpu, &self.atmosphere_layout, "strand lighting", bindings);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("procedural hair"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.render);
        pass.set_bind_group(0, &self.draw_group, &[]);
        pass.set_bind_group(1, &atmosphere, &[]);
        for i in 0..self.count {
            pass.draw_indirect(&self.commands, u64::from(i) * 16);
        }
    }
}
