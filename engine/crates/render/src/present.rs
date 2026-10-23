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
    /// Explicit screenshot path only. Recompose the existing scene image and
    /// prepared HUD using the exact surface format. No scene or simulation step
    /// is repeated, and normal presentation never allocates/maps this readback.
    pub fn capture_rgba(&self, gpu: &Gpu, picture: &wgpu::Texture, format: wgpu::TextureFormat,
        hud: Option<&crate::hud::HudRenderer>) -> Result<Vec<u8>, String> {
        if !matches!(format, wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb|wgpu::TextureFormat::Bgra8Unorm|wgpu::TextureFormat::Bgra8UnormSrgb) {
            return Err(format!("Screenshot surface format {format:?} is unsupported"));
        }
        let (width,height)=(picture.width(),picture.height());
        let target=gpu.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("F12 final composite"), size:picture.size(), mip_level_count:1,sample_count:1,
            dimension:wgpu::TextureDimension::D2,format,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[],
        });
        self.present(gpu,picture,&target.create_view(&Default::default()),hud,None)?;
        let row=(width*4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)*wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback=gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label:Some("F12 readback"),size:u64::from(row)*u64::from(height),
            usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false,
        });
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo{texture:&target,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
            wgpu::TexelCopyBufferInfo{buffer:&readback,layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(row),rows_per_image:Some(height)}},
            target.size());
        let submission=gpu.queue.submit([encoder.finish()]);
        let (sender,receiver)=std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read,move|result|{let _=sender.send(result);});
        gpu.device.poll(wgpu::PollType::Wait{submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(5))}).map_err(|e|format!("Screenshot GPU wait: {e:?}"))?;
        receiver.try_recv().map_err(|e|format!("Screenshot callback: {e}"))?.map_err(|e|format!("Screenshot mapping: {e:?}"))?;
        let result=(|| {
            let bytes=readback.slice(..).get_mapped_range().map_err(|e|format!("Screenshot mapped range: {e:?}"))?;
            let mut rgba=Vec::with_capacity(width as usize*height as usize*4);
            for y in 0..height {let start=(y*row) as usize;rgba.extend_from_slice(&bytes[start..start+width as usize*4]);}
            if matches!(format,wgpu::TextureFormat::Bgra8Unorm|wgpu::TextureFormat::Bgra8UnormSrgb){for pixel in rgba.chunks_exact_mut(4){pixel.swap(0,2);}}
            Ok(rgba)
        })();
        readback.unmap();
        result
    }

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
