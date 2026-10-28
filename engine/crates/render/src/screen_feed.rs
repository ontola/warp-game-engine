//! Reuse a completed scene view on in-world displays, without another camera
//! render or CPU readback. Updated after every view, before HUD: the next frame
//! sees the previous broadcast. Only mip 2 (512 square) and below are written.
//! Pair the layer with EMISSIVE | LIVE_SCREEN; larger mips stay unused/black.
use crate::{Gpu, ViewRect};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

pub const BASE_MIP: u32 = 2;

#[derive(Clone, Copy, Debug)]
pub struct Feed {
    pub layer: u32,
    pub source_view: usize,
    /// Physical display width / height, used for letterboxing.
    pub aspect: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params { crop: [f32; 4], fit: [f32; 4] }

fn params(view: &ViewRect, size: [u32; 2], aspect: f32) -> Params {
    let source_aspect = view.w as f32 / view.h.max(1) as f32;
    let fit = if source_aspect > aspect { [1., aspect / source_aspect] }
        else { [source_aspect / aspect, 1.] };
    Params {
        crop: [view.x as f32 / size[0] as f32, view.y as f32 / size[1] as f32,
            view.w as f32 / size[0] as f32, view.h as f32 / size[1] as f32],
        fit: [fit[0], fit[1], 1., 0.],
    }
}

struct Layer {
    feed: Feed,
    uniform: wgpu::Buffer,
    targets: Vec<wgpu::TextureView>,
    mips: Vec<wgpu::BindGroup>,
}

pub(crate) struct LiveScreens {
    layers: Vec<Layer>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    source: Option<(wgpu::Texture, Vec<wgpu::BindGroup>)>,
    last_update: Option<Instant>,
    last_layout: Vec<[u32; 4]>,
}

impl LiveScreens {
    pub(crate) fn new(gpu: &Gpu, albedo: &wgpu::Texture, feeds: &[Feed]) -> Self {
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live screen"), entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("live screen"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("live screen"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live screen"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8UnormSrgb, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache(),
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("live screen"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let mip_params = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("live screen mip params"), contents: bytemuck::bytes_of(&Params { crop: [0., 0., 1., 1.], fit: [1., 1., 0., 0.] }), usage: wgpu::BufferUsages::UNIFORM });
        let layers = feeds.iter().map(|&feed| {
            assert!(feed.layer < albedo.depth_or_array_layers() && feed.aspect.is_finite() && feed.aspect > 0.);
            let targets: Vec<_> = (BASE_MIP..albedo.mip_level_count()).map(|mip| albedo.create_view(&wgpu::TextureViewDescriptor {
                label: Some("live screen layer"), dimension: Some(wgpu::TextureViewDimension::D2), base_mip_level: mip, mip_level_count: Some(1), base_array_layer: feed.layer, array_layer_count: Some(1), ..Default::default()
            })).collect();
            let mips = targets.iter().take(targets.len() - 1).map(|source| bind(gpu, &layout, &sampler, source, &mip_params)).collect();
            let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("live screen crop"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            Layer { feed, uniform, targets, mips }
        }).collect();
        Self { layers, pipeline, layout, sampler, source: None, last_update: None, last_layout: Vec::new() }
    }

    pub(crate) fn due(&self, views: &[ViewRect], source: &wgpu::Texture) -> bool {
        self.last_update.is_none_or(|at| at.elapsed() >= Duration::from_secs_f64(1. / 30.))
            || self.source.as_ref().is_none_or(|(old, _)| old != source)
            || self.last_layout.len() != views.len()
            || self.last_layout.iter().zip(views).any(|(a, v)| *a != [v.x, v.y, v.w, v.h])
    }

    pub(crate) fn encode(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, source: &wgpu::Texture, views: &[ViewRect]) {
        if self.source.as_ref().is_none_or(|(old, _)| old != source) {
            let view = source.create_view(&Default::default());
            let binds = self.layers.iter().map(|layer| bind(gpu, &self.layout, &self.sampler, &view, &layer.uniform)).collect();
            self.source = Some((source.clone(), binds));
        }
        for (i, layer) in self.layers.iter().enumerate() {
            let view = views.get(layer.feed.source_view).filter(|v| v.w > 0 && v.h > 0);
            if let Some(view) = view {
                gpu.queue.write_buffer(&layer.uniform, 0, bytemuck::bytes_of(&params(view, [source.width(), source.height()], layer.feed.aspect)));
            }
            // Missing source views clear to black instead of displaying stale
            // footage from a player who left. Every mip remains initialized.
            for (mip, target) in layer.targets.iter().enumerate() {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("live screen blit/mip"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
                if view.is_some() {
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, if mip == 0 { &self.source.as_ref().unwrap().1[i] } else { &layer.mips[mip - 1] }, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }
        self.last_update = Some(Instant::now());
        self.last_layout.clear();
        self.last_layout.extend(views.iter().map(|v| [v.x, v.y, v.w, v.h]));
    }
}

fn bind(gpu: &Gpu, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, source: &wgpu::TextureView, params: &wgpu::Buffer) -> wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("live screen"), layout, entries: &[
        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
    ] })
}

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct Params { crop: vec4<f32>, fit: vec4<f32> };
@group(0) @binding(2) var<uniform> params: Params;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    let local = (in.uv - 0.5) / params.fit.xy + 0.5;
    if any(local < vec2<f32>(0.0)) || any(local > vec2<f32>(1.0)) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    let half_texel = vec2<f32>(0.5) / vec2<f32>(textureDimensions(source));
    let uv = clamp(params.crop.xy + local * params.crop.zw, params.crop.xy + half_texel, params.crop.xy + params.crop.zw - half_texel);
    var rgb = textureSampleLevel(source, samp, uv, 0.0).rgb;
    // Frame output is Rgba8Unorm holding sRGB numbers. Decode once before
    // the sRGB render target re-encodes. Mips already sample linear values.
    if params.fit.z > 0.5 { rgb = select(pow((rgb + 0.055) / 1.055, vec3<f32>(2.4)), rgb / 12.92, rgb <= vec3<f32>(0.04045)); }
    return vec4<f32>(rgb, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_crop_keeps_source_view_and_display_aspect() {
        let v = ViewRect { cam: crate::Camera { pos: glam::Vec3::Z, target: glam::Vec3::ZERO, near: 0.1, fov_y: 1. }, x: 960, y: 540, w: 960, h: 540 };
        let p = params(&v, [1920, 1080], 16. / 9.);
        assert_eq!(p.crop, [0.5, 0.5, 0.5, 0.5]);
        assert_eq!(p.fit, [1., 1., 1., 0.]);
        let p = params(&v, [1920, 1080], 1.);
        assert_eq!(p.fit[0], 1.);
        assert_eq!(p.fit[1], 9. / 16.);
    }

    #[test]
    fn gpu_feed_preserves_gamma_crops_views_resizes_and_clears_missing_sources() {
        let gpu = Gpu::new().unwrap();
        let texture = |w, h, levels, format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live screen test"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: levels, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[],
        });
        let albedo = texture(2048, 2048, 12, wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let make_source = |w: u32, color: [u8; 4]| {
            let t = texture(w, 8, 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
            let pixels: Vec<u8> = (0..8).flat_map(|_| (0..w).flat_map(|x| if x < w / 2 { [255, 0, 0, 255] } else { color })).collect();
            gpu.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &pixels, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(8) },
                wgpu::Extent3d { width: w, height: 8, depth_or_array_layers: 1 });
            t
        };
        let mut screens = LiveScreens::new(&gpu, &albedo, &[Feed { layer: 0, source_view: 1, aspect: 1. }]);
        let view = |x, w| ViewRect { cam: crate::Camera { pos: glam::Vec3::Z, target: glam::Vec3::ZERO, near: 0.1, fov_y: 1. }, x, y: 0, w, h: 8 };
        let read_mip = |mip: u32| {
            let side = 2048 >> mip;
            let result = texture(side, side, 1, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC);
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            enc.copy_texture_to_texture(wgpu::TexelCopyTextureInfo { texture: &albedo, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: &result, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 });
            gpu.queue.submit([enc.finish()]);
            let pixels = gpu.read_rgba8(&result);
            let i = ((side / 2 * side + side / 2) * 4) as usize;
            pixels[i..i + 4].to_vec()
        };
        let first = make_source(16, [128, 128, 128, 255]);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        screens.encode(&gpu, &mut enc, &first, &[view(0, 8), view(8, 8)]);
        gpu.queue.submit([enc.finish()]);
        for mip in [2, 5, 11] {
            let px = read_mip(mip);
            assert!(px[..3].iter().all(|&c| c.abs_diff(128) <= 2), "gamma or crop at mip {mip}: {px:?}");
            assert_eq!(px[3], 255);
        }
        let resized = make_source(32, [0, 255, 0, 255]);
        assert!(screens.due(&[view(0, 16), view(16, 16)], &resized));
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        screens.encode(&gpu, &mut enc, &resized, &[view(0, 16), view(16, 16)]);
        gpu.queue.submit([enc.finish()]);
        assert_eq!(read_mip(2), [0, 255, 0, 255]);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        screens.encode(&gpu, &mut enc, &resized, &[view(0, 32)]);
        gpu.queue.submit([enc.finish()]);
        assert_eq!(read_mip(2), [0, 0, 0, 255]);
        assert_eq!(read_mip(11), [0, 0, 0, 255]);
    }
}
