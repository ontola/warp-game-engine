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
    shadow: wgpu::RenderPipeline,
    shadow_group: wgpu::BindGroup,
    atmosphere: BindGroups<8>,
    atmosphere_layout: wgpu::BindGroupLayout,
}

/// Points per grown strand (assets' warp_hair::POINTS; the shader's 10).
const GROWN_POINTS: usize = 10;

impl PlayerStrands {
    pub(crate) fn new(
        gpu: &Gpu,
        descriptors: &[(usize, usize, Groom)],
        grown: &[[f32; 4]],
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
        // Grown hair fits the device's storage binding, like the people
        // pools: heads whose strands lie past it go without (bald), rather
        // than an invalid bind group stopping every frame.
        let max_strands = (gpu.limits.max_storage_buffer_binding_size as usize / 16) / GROWN_POINTS;
        let fits = |g: &Groom| g.info[2] != 3 || g.info[3] as usize + g.weights[0] as usize <= max_strands;
        let dropped = descriptors.iter().filter(|(_, _, g)| !fits(g)).count();
        if dropped > 0 {
            eprintln!("humans: grown hair for {dropped} of {} heads does not fit this device's {} MB storage binding; they go without", descriptors.len(), gpu.limits.max_storage_buffer_binding_size >> 20);
        }
        let grown = &grown[..grown.len().min(max_strands * GROWN_POINTS)];
        let data: Vec<_> = descriptors
            .iter()
            .filter(|(_, _, g)| fits(g))
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
        let grown = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grown hair strands"),
                contents: bytemuck::cast_slice(if grown.is_empty() { &[[0f32; 4]] } else { grown }),
                usage: U::STORAGE,
            });
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
        let shadow = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hair fibres into a sun map"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("strand_shadow"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: Default::default(),
                multisample: Default::default(),
                // As the shadow raster (frame.rs raster_pipe).
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    depth_write_enabled: Some(true),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                fragment: None,
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
        let draw_group = {
            let entries = [(0, &camera), (1, &heads), (2, owners), (3, palette), (8, &guides), (9, &grown)]
                .map(|(binding, b)| wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() });
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("procedural hair sources"),
                layout: &render.get_bind_group_layout(0),
                entries: &entries,
            })
        };
        let shadow_group = {
            let entries = [(0, &camera), (1, &heads), (2, owners), (3, palette), (9, &grown)]
                .map(|(binding, b)| wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() });
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hair shadow sources"),
                layout: &shadow.get_bind_group_layout(0),
                entries: &entries,
            })
        };
        let atmosphere_layout = render.get_bind_group_layout(1);
        Self {
            camera,
            commands,
            count,
            select,
            render,
            shadow,
            cull_group,
            draw_group,
            shadow_group,
            atmosphere: Default::default(),
            atmosphere_layout,
        }
    }

    /// Casts grown hair into a sun map: `vp` its projection, `texels` per
    /// metre. A quarter of the strands (the shader's SUN_SHARE), a texel wide.
    pub(crate) fn cast(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, vp: Mat4, texels: f32, depth: &wgpu::TextureView) {
        self.choose(gpu, enc, vp, texels, 16, 0.);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hair sun shadow"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.shadow);
        pass.set_bind_group(0, &self.shadow_group, &[]);
        for i in 0..self.count {
            pass.draw_indirect(&self.commands, u64::from(i) * 16);
        }
    }

    /// How many strands of each head this view draws.
    fn choose(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, vp: Mat4, focal: f32, slot: usize, time: f32) {
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
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("strand density"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.select);
        pass.set_bind_group(0, &self.cull_group, &[]);
        pass.dispatch_workgroups(self.count.div_ceil(32), 1, 1);
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
        self.choose(gpu, enc, vp, focal, slot, time);
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
