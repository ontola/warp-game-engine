//! GPU-only presentation, used solely after an explicit interactive launch.
use crate::Gpu;
use wgpu::util::DeviceExt;
use crate::frame::resource_cache::{BindGroups, TextureViews};

#[derive(Default)]
struct SourceCache {
    view: TextureViews<1>,
    bind: BindGroups<1>,
}

pub struct Presenter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    source: std::sync::Mutex<SourceCache>,
    /// On an HDR (fp16 scRGB) surface: the HUD goes to its own SDR layer and
    /// one pass composes scene and HUD at the OS's SDR white.
    hdr: Option<HdrComposite>,
}

/// The HDR surface's composition: scene (the renderer's linear HDR output, or
/// the SDR picture decoded) under the HUD layer, both scaled to SDR white.
struct HdrComposite {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    /// What the uniform holds now: linear source, HUD, white scale.
    state: std::sync::Mutex<Option<(bool, bool, f32)>>,
    white_scale: std::sync::Mutex<f32>,
    /// The HUD layer at the target's size, and the bind group for the last sources.
    layer: std::sync::Mutex<Option<(wgpu::Texture, wgpu::TextureView)>>,
    bind: std::sync::Mutex<BindGroups<2>>,
    views: std::sync::Mutex<TextureViews<2>>,
    blank: wgpu::TextureView,
}

/// The format the HUD layer of an HDR presenter is drawn in.
pub const HDR_HUD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl Presenter {
    /// Explicit screenshot path only. Recompose the existing scene image and
    /// prepared HUD using the exact surface format. No scene or simulation step
    /// is repeated, and normal presentation never allocates/maps this readback.
    pub fn capture_rgba(&self, gpu: &Gpu, picture: &wgpu::Texture, format: wgpu::TextureFormat,
        hud: Option<&crate::hud::HudRenderer>) -> Result<Vec<u8>, String> {
        self.capture_rgba_scaled(gpu,picture,format,hud,0).map(|(_,_,rgba)|rgba)
    }

    /// Developer preview: compose at render resolution, then downsample on the
    /// GPU before readback. Zero retains native pixels; never changes gameplay
    /// render size, HUD layout, simulation ticks or scene rendering.
    pub fn capture_rgba_scaled(&self, gpu: &Gpu, picture: &wgpu::Texture, format: wgpu::TextureFormat,
        hud: Option<&crate::hud::HudRenderer>, max_width: u32) -> Result<(u32,u32,Vec<u8>), String> {
        if self.hdr.is_some() { return Err("Screenshots capture with an SDR presenter, not an HDR one".into()); }
        if !matches!(format, wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb|wgpu::TextureFormat::Bgra8Unorm|wgpu::TextureFormat::Bgra8UnormSrgb) {
            return Err(format!("Screenshot surface format {format:?} is unsupported"));
        }
        let (width,height)=preview_dimensions(picture.width(),picture.height(),max_width);
        let scaled=width!=picture.width();
        let target=gpu.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("developer capture"), size:wgpu::Extent3d{width,height,depth_or_array_layers:1}, mip_level_count:1,sample_count:1,
            dimension:wgpu::TextureDimension::D2,format,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[],
        });
        if scaled && hud.is_some() {
            // Glyph positions/viewport were prepared for the full output size.
            // Downsample the completed HUD too, rather than cropping its layout.
            let encoded_format=match format {
                wgpu::TextureFormat::Rgba8UnormSrgb=>wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Bgra8UnormSrgb=>wgpu::TextureFormat::Bgra8Unorm,
                other=>other,
            };
            let view_formats=if encoded_format!=format {vec![encoded_format]} else {vec![]};
            let composite=gpu.device.create_texture(&wgpu::TextureDescriptor {
                label:Some("preview full HUD composite"),size:picture.size(),mip_level_count:1,sample_count:1,
                dimension:wgpu::TextureDimension::D2,format,
                usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&view_formats,
            });
            self.present(gpu,picture,&composite.create_view(&Default::default()),hud,None)?;
            // Present expects encoded LDR numbers. An UNORM alias avoids a
            // second sRGB decode on hardware while sampling this intermediate.
            let source=composite.create_view(&wgpu::TextureViewDescriptor{format:Some(encoded_format),..Default::default()});
            self.present_view(gpu,&source,&target.create_view(&Default::default()),None,None)?;
        } else {
            self.present(gpu,picture,&target.create_view(&Default::default()),hud,None)?;
        }
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
            Ok((width,height,rgba))
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
            multiview_mask: None, cache: gpu.pipeline_cache(),
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("surface encoding"), contents: bytemuck::cast_slice(&[u32::from(format.is_srgb()), 0, 0, 0]), usage: wgpu::BufferUsages::UNIFORM });
        let hdr = (format == wgpu::TextureFormat::Rgba16Float).then(|| HdrComposite::new(gpu, format));
        Self { layout, pipeline, sampler, uniform, source: Default::default(), hdr }
    }

    /// True on an HDR surface (`Rgba16Float`, see [`crate::hdr::configure_surface`]).
    pub fn is_hdr(&self) -> bool { self.hdr.is_some() }

    /// The format to create the HUD renderer in for this presenter: the
    /// surface's own, or [`HDR_HUD_FORMAT`] on an HDR surface.
    pub fn hud_format(&self, surface: wgpu::TextureFormat) -> wgpu::TextureFormat {
        if self.hdr.is_some() { HDR_HUD_FORMAT } else { surface }
    }

    /// On an HDR surface: what SDR white is in surface units
    /// ([`crate::hdr::HdrDisplay::white_scale`]). Ignored on SDR surfaces.
    pub fn set_white_scale(&self, scale: f32) {
        if let Some(hdr) = &self.hdr { *hdr.white_scale.lock().unwrap() = if scale.is_finite() { scale.clamp(0.1, 100.0) } else { 1.0 }; }
    }

    /// Present with the renderer's HDR output ([`crate::Renderer::hdr_output`])
    /// on an HDR surface. Without one (or on an SDR surface) this is
    /// [`Self::present`]: an HDR surface then shows the SDR picture at SDR white.
    pub fn present_hdr(&self, gpu: &Gpu, picture: &wgpu::Texture, hdr: Option<&wgpu::Texture>, target: &wgpu::TextureView,
        hud: Option<&crate::hud::HudRenderer>, mut profile: Option<&mut crate::hud_profile::HudProfiler>) -> Result<(), String> {
        let Some(composite) = &self.hdr else { return self.present(gpu, picture, target, hud, profile) };
        let size = target.texture().size();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let layer = hud.map(|hud| {
            let mut layer = composite.layer.lock().unwrap();
            if layer.as_ref().is_none_or(|(t, _)| t.size() != size) {
                let t = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("HDR HUD layer"), size, mip_level_count: 1, sample_count: 1,
                    dimension: wgpu::TextureDimension::D2, format: HDR_HUD_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
                let v = t.create_view(&Default::default());
                *layer = Some((t, v));
            }
            let view = layer.as_ref().unwrap().1.clone();
            (hud, view)
        });
        if let Some((hud, view)) = &layer {
            // Premultiplied over transparent black: colour and coverage.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD layer for HDR"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: profile.as_ref().map(|p| p.writes()), occlusion_query_set: None, multiview_mask: None,
            });
            hud.render(&mut pass)?;
        }
        let state = (hdr.is_some(), layer.is_some(), *composite.white_scale.lock().unwrap());
        {
            let mut current = composite.state.lock().unwrap();
            if *current != Some(state) {
                gpu.queue.write_buffer(&composite.uniform, 0, bytemuck::cast_slice(&[u32::from(state.0), u32::from(state.1), state.2.to_bits(), 0]));
                *current = Some(state);
            }
        }
        let (picture_view, hdr_view) = {
            let mut views = composite.views.lock().unwrap();
            (views.get(picture), hdr.map(|t| views.get(t)))
        };
        let hud_view = layer.as_ref().map_or(&composite.blank, |(_, v)| v);
        let bind = composite.bind.lock().unwrap().get(gpu, &composite.layout, "HDR composition", &[
            (0, wgpu::BindingResource::TextureView(&picture_view)),
            (1, wgpu::BindingResource::Sampler(&self.sampler)),
            (2, composite.uniform.as_entire_binding()),
            (3, wgpu::BindingResource::TextureView(hdr_view.as_ref().unwrap_or(&composite.blank))),
            (4, wgpu::BindingResource::TextureView(hud_view)),
        ]);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HDR presentation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(&composite.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if layer.is_some() { if let Some(profile) = profile.as_mut() { profile.resolve(&mut encoder)?; } }
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    pub fn present(&self, gpu: &Gpu, picture: &wgpu::Texture, target: &wgpu::TextureView,
        hud: Option<&crate::hud::HudRenderer>, profile: Option<&mut crate::hud_profile::HudProfiler>) -> Result<(), String> {
        if self.hdr.is_some() { return self.present_hdr(gpu, picture, None, target, hud, profile); }
        let bind = {
            let mut source = self.source.lock().unwrap();
            let view = source.view.get(picture);
            source.bind.get(gpu, &self.layout, "cached live frame", &[
                (0, wgpu::BindingResource::TextureView(&view)),
                (1, wgpu::BindingResource::Sampler(&self.sampler)),
                (2, self.uniform.as_entire_binding()),
            ])
        };
        self.present_bind(gpu,&bind,target,hud,profile)
    }

    pub fn resource_cache_stats(&self) -> crate::frame::ResourceCacheStats {
        let source = self.source.lock().unwrap();
        let mut stats = source.view.stats();
        stats.add(source.bind.stats());
        stats
    }

    fn present_view(&self, gpu: &Gpu, view: &wgpu::TextureView, target: &wgpu::TextureView,
        hud: Option<&crate::hud::HudRenderer>, profile: Option<&mut crate::hud_profile::HudProfiler>) -> Result<(), String> {
        // A screenshot's alternate-format intermediate is transient and must
        // never evict the source retained for ordinary surface presentation.
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live frame"), layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.uniform.as_entire_binding() },
            ],
        });
        self.present_bind(gpu,&bind,target,hud,profile)
    }

    fn present_bind(&self, gpu: &Gpu, bind: &wgpu::BindGroup, target: &wgpu::TextureView,
        hud: Option<&crate::hud::HudRenderer>, mut profile: Option<&mut crate::hud_profile::HudProfiler>) -> Result<(), String> {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live presentation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
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

impl HdrComposite {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("HDR presentation"), entries: &[
            texture(0),
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            texture(3), texture(4),
        ]});
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("HDR presentation"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/present_hdr.wgsl").into()) });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("HDR presentation"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HDR presentation"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache(),
        });
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("HDR presentation"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let blank = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("HDR presentation placeholder"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default());
        HdrComposite { layout, pipeline, uniform, state: Default::default(), white_scale: std::sync::Mutex::new(1.0), layer: Default::default(),
            bind: Default::default(), views: Default::default(), blank }
    }
}

fn preview_dimensions(width:u32,height:u32,max_width:u32)->(u32,u32) {
    if max_width==0 || width<=max_width {(width,height)}
    else {(max_width,((u64::from(height)*u64::from(max_width)+u64::from(width)/2)/u64::from(width)).max(1) as u32)}
}
