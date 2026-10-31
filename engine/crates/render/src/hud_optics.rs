//! Optional per-viewport glass projection for HUDs. Games opt in; menus remain flat.
use crate::{
    Gpu,
    hud::{HudDrawList, HudRenderer, HudStats},
};
use wgpu::util::DeviceExt;
#[derive(Clone, Copy, Debug)]
pub struct Optics {
    pub rect: [f32; 4],
    pub curvature: f32,
    pub glow: f32,
    pub quiet: bool,
}
pub(crate) struct Pass {
    size: [u32; 2],
    hud: Box<HudRenderer>,
    target: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}
impl Pass {
    pub fn new(gpu: &Gpu, size: [u32; 2], format: wgpu::TextureFormat) -> Result<Self, String> {
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HUD optics canvas"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("HUD optics views"),
                contents: &[0u8; 144],
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("HUD optics"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let view = target.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HUD optics"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("HUD optics"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/hud_optics.wgsl").into()),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("HUD optics"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("HUD optics"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        Ok(Self {
            size,
            hud: Box::new(HudRenderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?),
            target,
            pipeline,
            bind,
            uniform,
        })
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        list: &HudDrawList,
        srgb: bool,
    ) -> Result<HudStats, String> {
        let mut data = [[0f32; 4]; 9];
        data[0] = [
            list.size[0] as f32,
            list.size[1] as f32,
            list.optics.len() as f32,
            if srgb { 1. } else { 0. },
        ];
        for (i, v) in list.optics.iter().enumerate() {
            data[1 + i * 2] = v.rect;
            data[2 + i * 2] = [v.curvature, v.glow, if v.quiet { 1. } else { 0. }, 0.];
        }
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
        let mut flat = list.clone();
        flat.optics.clear();
        let stats = self.hud.prepare(gpu, &flat)?;
        let view = self.target.create_view(&Default::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("HUD optics projection"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD optics instruments"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.hud.render(&mut pass)?;
        }
        gpu.queue.submit([encoder.finish()]);
        Ok(stats)
    }
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
