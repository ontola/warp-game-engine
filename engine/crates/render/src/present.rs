//! GPU-only presentation, used solely after an explicit interactive launch.
use crate::Gpu;
use wgpu::util::DeviceExt;

pub struct Presenter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

impl Presenter {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live presentation"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("live presentation"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/present.wgsl").into()) });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("live presentation"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live presentation"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("surface encoding"), contents: bytemuck::cast_slice(&[u32::from(format.is_srgb()), 0, 0, 0]), usage: wgpu::BufferUsages::UNIFORM });
        Self { layout, pipeline, sampler, uniform }
    }

    pub fn present(&self, gpu: &Gpu, picture: &wgpu::Texture, target: &wgpu::TextureView,
        hud: Option<&crate::hud::HudRenderer>, mut profile: Option<&mut crate::hud_profile::HudProfiler>) -> Result<(), String> {
        let view = picture.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live frame"), layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.uniform.as_entire_binding() },
            ],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live presentation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(hud) = hud {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD at output resolution"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: profile.as_ref().map(|p|p.writes()),
                occlusion_query_set: None, multiview_mask: None,
            });
            hud.render(&mut pass)?;
        }
        if hud.is_some() { if let Some(profile) = profile.as_mut() { profile.resolve(&mut encoder)?; } }
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }
}
