//! Screens in the world: draw a HUD list (shapes, text, pictures) into an
//! albedo layer reserved with `Scene::add_screen_texture`, then rebuild that
//! layer's mips on the GPU. The surface is an ordinary material, so it gets
//! depth, TAA and lighting; with EMISSIVE it glows like a TV.
//!
//! Draw only when the content changes; a layer redraw is one HUD pass at
//! `TEXTURE_SIZE` plus a short mip chain.
use crate::frame::TEXTURE_SIZE;
use crate::hud::{HudDrawList, HudImage, HudRenderer, ImageFilter};
use crate::{Gpu, Renderer};

pub const SIZE: u32 = TEXTURE_SIZE;

pub struct Screen {
    hud: HudRenderer,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Screen {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let hud = HudRenderer::new(gpu, format)?;
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("screen mip"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ] });
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("screen mip"), source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()) });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("screen mip"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen mip"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache(),
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("screen mip"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Ok(Self { hud, pipeline, layout, sampler })
    }

    /// Pictures for this screen's draw lists (separate from the main HUD's).
    pub fn set_image(&mut self, gpu: &Gpu, id: HudImage, picture: &crate::picture::Picture, filter: ImageFilter) { self.hud.set_image(gpu, id, picture, filter); }
    pub fn remove_image(&mut self, id: HudImage) { self.hud.remove_image(id); }

    /// Draw `list` (sized `[SIZE, SIZE]`) over `background` (straight-alpha
    /// sRGB) into albedo `layer`, and rebuild its mips. Layer uv (0,0) is the
    /// list's top-left.
    pub fn draw(&mut self, gpu: &Gpu, renderer: &Renderer, layer: i32, background: [f32; 4], list: &HudDrawList) -> Result<(), String> {
        let texture = renderer.albedo_array();
        if layer < 0 || layer as u32 >= texture.depth_or_array_layers() { return Err(format!("screen layer {layer} is not in the albedo array")); }
        if list.size != [SIZE, SIZE] { return Err(format!("screen draw lists are {SIZE}x{SIZE}")); }
        self.hud.prepare(gpu, list)?;
        let view = |mip: u32| texture.create_view(&wgpu::TextureViewDescriptor { label: Some("screen layer"), dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: mip, mip_level_count: Some(1), base_array_layer: layer as u32, array_layer_count: Some(1), ..Default::default() });
        let linear = |c: f32| { let c = c.clamp(0.0, 1.0) as f64; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } };
        let a = background[3].clamp(0.0, 1.0) as f64;
        let clear = wgpu::Color { r: linear(background[0]) * a, g: linear(background[1]) * a, b: linear(background[2]) * a, a };
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("screen draw") });
        {
            let target = view(0);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("screen HUD"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            self.hud.render(&mut pass)?;
        }
        for mip in 1..texture.mip_level_count() {
            let source = view(mip - 1);
            let target = view(mip);
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("screen mip"), layout: &self.layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }] });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("screen mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }
}

// One full-screen triangle; the bilinear tap at each target texel's centre is
// the 2x2 box average of the finer level (linear light, sRGB views).
const MIP_SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> { return textureSampleLevel(source, samp, in.uv, 0.0); }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Material, EMISSIVE};
    use glam::{Affine3A, Vec3};

    /// A glowing screen facing the camera: its left half is drawn red, its
    /// right half green, after the renderer exists.
    #[test]
    fn gpu_screen_redraws_a_live_layer_seen_in_the_scene() {
        let gpu = Gpu::new().unwrap();
        let mut scene = crate::Scene::default();
        let layer = scene.add_screen_texture("tv");
        let material = scene.add_material(Material { tint: [1.0; 3], layer, roughness: 1.0, flags: EMISSIVE, scale: 4000.0, metal: 0.0 });
        let mesh = scene.add_mesh(&crate::primitives::quad(material).mesh());
        scene.add_instance(mesh, Affine3A::from_scale(Vec3::new(2.0, 1.0, 1.0)));
        let mut light = crate::Lighting::default();
        light.sun_lux = 0.0;
        light.sky_lux = 0.0;
        let renderer = Renderer::new(&gpu, &scene, &light);
        let mut screen = Screen::new(&gpu).unwrap();
        let mut list = HudDrawList::new([SIZE, SIZE]);
        let clip = [0.0, 0.0, SIZE as f32, SIZE as f32];
        list.rect([0.0, 0.0, SIZE as f32 / 2.0, SIZE as f32], [1.0, 0.0, 0.0, 1.0], clip);
        list.rect([SIZE as f32 / 2.0, 0.0, SIZE as f32 / 2.0, SIZE as f32], [0.0, 1.0, 0.0, 1.0], clip);
        screen.draw(&gpu, &renderer, layer, [0.0, 0.0, 0.0, 1.0], &list).unwrap();
        let camera = crate::Camera { pos: Vec3::new(0.0, 0.0, 2.5), target: Vec3::ZERO, fov_y: 50f32.to_radians(), near: 0.1 };
        let (w, h) = (96, 64);
        let (rgba, _) = renderer.render(&gpu, &camera, &light, w, h);
        let px = |x: u32, y: u32| { let i = ((y * w + x) * 4) as usize; [rgba[i], rgba[i + 1], rgba[i + 2]] };
        let (left, right) = (px(w / 2 - 12, h / 2), px(w / 2 + 12, h / 2));
        assert!(left[0] > 100 && left[0] > 4 * left[1].max(1), "left glows red: {left:?}");
        assert!(right[1] > 100 && right[1] > 4 * right[0].max(1), "right glows green: {right:?}");
    }
}
