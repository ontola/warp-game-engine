//! Sprites: textured quads in the world, for pixel-art characters, name tags
//! and floating button prompts. Billboards face each view's camera; fixed
//! sprites keep their own axes (a 2.5D room seen from one side); shafts turn
//! about one axis towards each camera and add their light (beams, glows).
//!
//! Drawn in HDR after temporal reconstruction, like `fx`: crisp (no TAA blur
//! or ghosting on pixel art), hidden behind the opaque scene by its depth,
//! sorted back to front per view. They write no depth and cast no shadow.
//! Colour is radiance in nits for a white texel; light a sprite yourself
//! with `irradiance` (the scene's local lights) or make it glow (tags).
//!
//! ```ignore
//! renderer.set_sprite_image(&gpu, SpriteImage(1), &sheet, ImageFilter::Nearest);
//! renderer.set_sprites(&gpu, &[Sprite::billboard(pos, [0.6, 0.8], SpriteImage(1), frame_uv, rgb)]);
//! // Text: draw a HudDrawList into a Canvas, then use its texture as an image.
//! ```
use crate::headlights::LocalLight;
use crate::hud::{HudDrawList, HudRenderer, ImageFilter};
use crate::picture::Picture;
use crate::{Camera, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::Mutex;

pub const MAX_SPRITES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpriteImage(pub u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Facing {
    /// Turns to each camera; `up` stays world up (a standing character).
    Billboard,
    /// Faces the eye plane, including pitch/roll; minimum width scales both axes.
    Screen,
    /// Its own right and up axes (unit length; sizes scale them).
    Fixed { right: Vec3, up: Vec3 },
    /// Keeps `up` (unit length) and turns about it to face each camera's
    /// position: a light shaft, a laser or a column of smoke.
    Axis { up: Vec3 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub center: Vec3,
    /// Width and height in metres.
    pub size: [f32; 2],
    pub facing: Facing,
    pub image: SpriteImage,
    /// x/y/width/height in 0..1 of the image; negative width mirrors.
    pub uv: [f32; 4],
    /// Linear RGB radiance in nits for a white texel.
    pub radiance: Vec3,
    pub opacity: f32,
    /// Texels below this alpha are dropped (0.5 for pixel art, 0 to blend).
    pub alpha_cut: f32,
    /// Metres towards the camera before the depth test, so a sprite standing
    /// on a floor or against a wall is not cut by it.
    pub depth_bias: f32,
    /// Adds its light without covering what is behind it (glows, beams):
    /// the texel alpha and `opacity` only scale the radiance.
    pub additive: bool,
    /// Never narrower on screen than this many pixels (0: off). The texture
    /// keeps its radiance as it widens, so a thin bright laser far away
    /// still shows its core and glow, as a bloom would spread it.
    pub min_pixels: f32,
    /// 0: the image texel. Otherwise the game's own `game_sprite` WGSL
    /// (`set_game_sprite`) colours it, told this number and `params`.
    pub effect: u32,
    pub params: [f32; 4],
}

impl Sprite {
    pub fn billboard(center: Vec3, size: [f32; 2], image: SpriteImage, uv: [f32; 4], radiance: Vec3) -> Self {
        Self { center, size, facing: Facing::Billboard, image, uv, radiance, opacity: 1.0, alpha_cut: 0.5, depth_bias: 0.05, additive: false, min_pixels: 0.0, effect: 0, params: [0.0; 4] }
    }
    /// Turned about `up` towards the camera, additive and blended (no cut).
    pub fn shaft(center: Vec3, size: [f32; 2], up: Vec3, image: SpriteImage, uv: [f32; 4], radiance: Vec3) -> Self {
        Self { facing: Facing::Axis { up }, alpha_cut: 0.0, additive: true, ..Self::billboard(center, size, image, uv, radiance) }
    }
    pub fn fixed(center: Vec3, size: [f32; 2], right: Vec3, up: Vec3, image: SpriteImage, uv: [f32; 4], radiance: Vec3) -> Self {
        Self { facing: Facing::Fixed { right, up }, ..Self::billboard(center, size, image, uv, radiance) }
    }
}

/// Irradiance (lux) at `p` facing `normal` from the scene's local lights, the
/// same response the shade pass uses (unshadowed), plus `ambient` lux.
/// Radiance of a lit sprite with albedo `a` is `a * irradiance / PI`.
pub fn irradiance(lights: &[LocalLight], p: Vec3, normal: Vec3, ambient: Vec3) -> Vec3 {
    let mut sum = ambient;
    for lamp in lights {
        // Half-Lambert wrap: a flat sprite stands in for a rounded body.
        if let Some((to_light, e)) = light_at(lamp, p) { sum += e * (normal.dot(to_light) * 0.5 + 0.5).max(0.0); }
    }
    sum
}

/// One local light at `p`: the unit direction towards it and the irradiance
/// on a surface facing it (lux), as the shade pass evaluates it (unshadowed).
pub fn light_at(lamp: &LocalLight, p: Vec3) -> Option<(Vec3, Vec3)> {
    let offset = p - lamp.position;
    let d2 = offset.length_squared();
    let range = lamp.range_m;
    if d2 >= range * range || d2 < 1e-10 { return None; }
    let outward = offset / d2.sqrt();
    let (lo, hi) = (lamp.outer_cos, lamp.inner_cos);
    let t = ((outward.dot(lamp.direction.normalize_or_zero()) - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
    let cone = t * t * (3.0 - 2.0 * t);
    if cone <= 0.0 { return None; }
    let normalized = d2 / (range * range);
    let fade = (1.0 - normalized * normalized).max(0.0);
    let near = lamp.source_radius_m * lamp.source_radius_m;
    let spread = if lamp.falloff == 2.0 || lamp.falloff == 0.0 { d2.max(near) } else { d2.max(near).powf(lamp.falloff * 0.5) };
    Some((-outward, lamp.intensity_cd * (cone * fade * fade / spread)))
}

/// An offscreen HUD canvas (text, shapes, pictures) to show as a sprite.
pub struct Canvas { hud: HudRenderer, texture: wgpu::Texture }

impl Canvas {
    pub fn new(gpu: &Gpu, size: [u32; 2]) -> Result<Self, String> {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("sprite canvas"), size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        Ok(Self { hud: HudRenderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?, texture })
    }
    pub fn hud(&mut self) -> &mut HudRenderer { &mut self.hud }
    pub fn texture(&self) -> &wgpu::Texture { &self.texture }
    /// Redraw on a transparent background. `list.size` must be the canvas size.
    pub fn draw(&mut self, gpu: &Gpu, list: &HudDrawList) -> Result<(), String> {
        if list.size != [self.texture.width(), self.texture.height()] { return Err("canvas draw list size differs from the canvas".into()); }
        self.hud.prepare(gpu, list)?;
        let view = self.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("sprite canvas") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("sprite canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            self.hud.render(&mut pass)?;
        }
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuSprite { center: [f32; 4], right: [f32; 4], up: [f32; 4], uv: [f32; 4], color: [f32; 4], misc: [f32; 4], params: [f32; 4] }

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniform { view_proj: [[f32; 4]; 4], depth_vp: [[f32; 4]; 4], camera_near: [f32; 4], camera_right: [f32; 4], output: [f32;4] }

struct Target { size: (u32, u32), output: wgpu::Texture, uniform: wgpu::Buffer, instances: wgpu::Buffer }

pub(crate) struct SpriteRenderer {
    layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    ldr_pipeline: Option<(u64,wgpu::TextureFormat,wgpu::RenderPipeline)>,
    /// The GAME_SPRITE version `pipeline` was built from.
    game: u64,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    images: HashMap<SpriteImage, wgpu::BindGroup>,
    sprites: Vec<Sprite>,
    targets: Vec<Option<Target>>,
}

impl SpriteRenderer {
    fn new(gpu: &Gpu) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty, count: None };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("sprites"), entries: &[
            entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
            entry(2, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
        ] });
        let image_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("sprite image"), entries: &[
            entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        ] });
        let (game, pipeline) = Self::pipeline(gpu, &layout, &image_layout, wgpu::TextureFormat::Rgba16Float);
        let sampler = |mag, mip| gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("sprite"), address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge, mag_filter: mag, min_filter: wgpu::FilterMode::Linear, mipmap_filter: mip, ..Default::default() });
        Self { layout, image_layout, pipeline, ldr_pipeline:None, game, linear: sampler(wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear),
            nearest: sampler(wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Nearest), images: HashMap::new(), sprites: Vec::new(), targets: Vec::new() }
    }

    /// The pipeline with the game's sprite shader as it is now, and which
    /// version of it that is.
    fn pipeline(gpu: &Gpu, layout: &wgpu::BindGroupLayout, image_layout: &wgpu::BindGroupLayout, format:wgpu::TextureFormat) -> (u64, wgpu::RenderPipeline) {
        let (game, source) = { let g = GAME_SPRITE.lock().unwrap(); (g.0, shader_with(g.1.as_deref())) };
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sprites"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sprites"), bind_group_layouts: &[Some(layout), Some(image_layout)], immediate_size: 0 });
        (game, gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("sprites"), layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() }, depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache() }))
    }

    fn bind_texture(&mut self, gpu: &Gpu, id: SpriteImage, texture: &wgpu::Texture, filter: ImageFilter) {
        let view = texture.create_view(&Default::default());
        let sampler = match filter { ImageFilter::Linear => &self.linear, ImageFilter::Nearest => &self.nearest };
        self.images.insert(id, gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sprite image"), layout: &self.image_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) }] }));
    }

    /// Picture upload: sRGB texture of linear-premultiplied colour (the same
    /// encoding a Canvas renders), with a box-filtered mip chain.
    fn set_picture(&mut self, gpu: &Gpu, id: SpriteImage, picture: &Picture, filter: ImageFilter) {
        let to_linear = |c: u8| { let c = c as f32 / 255.0; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } };
        let to_srgb = |l: f32| { let l = l.clamp(0.0, 1.0); ((if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }) * 255.0 + 0.5) as u8 };
        let mut linear: Vec<f32> = picture.rgba.chunks_exact(4).flat_map(|p| { let a = p[3] as f32 / 255.0; [to_linear(p[0]) * a, to_linear(p[1]) * a, to_linear(p[2]) * a, a] }).collect();
        let (mut w, mut h) = (picture.width, picture.height);
        let mips = w.max(h).ilog2() + 1;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("sprite picture"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: mips, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        for mip in 0..mips {
            let bytes: Vec<u8> = linear.chunks_exact(4).flat_map(|p| [to_srgb(p[0]), to_srgb(p[1]), to_srgb(p[2]), (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]).collect();
            gpu.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
            if mip + 1 == mips { break; }
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut next = vec![0f32; (nw * nh * 4) as usize];
            for y in 0..nh { for x in 0..nw { for c in 0..4 {
                let mut sum = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] { sum += linear[((((y * 2 + dy).min(h - 1)) * w + (x * 2 + dx).min(w - 1)) * 4 + c) as usize]; }
                next[((y * nw + x) * 4 + c) as usize] = sum / 4.0;
            } } }
            linear = next; w = nw; h = nh;
        }
        self.bind_texture(gpu, id, &texture, filter);
    }

    /// Draw over a copy of `input`; None when there is nothing to draw.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, index: usize, input: &wgpu::Texture, depth: &wgpu::TextureView,
        camera: Camera, view: Mat4, output_vp: Mat4, depth_vp: Mat4) -> Result<Option<wgpu::Texture>, String> {
        self.render_output(gpu,enc,index,input,depth,camera,view,output_vp,depth_vp,0.0)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_output(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,index:usize,input:&wgpu::Texture,depth:&wgpu::TextureView,
        camera:Camera,view:Mat4,output_vp:Mat4,depth_vp:Mat4,exposure:f32)->Result<Option<wgpu::Texture>,String> {
        let ldr=exposure>0.0;
        let mut order: Vec<(usize, f32)> = self.sprites.iter().enumerate()
            .filter(|(_, s)| self.images.contains_key(&s.image) && s.opacity > 0.0)
            .map(|(i, s)| (i, s.center.distance_squared(camera.pos))).collect();
        if order.is_empty() { return Ok(None); }
        if !if ldr {matches!(input.format(),wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb)}else{input.format()==wgpu::TextureFormat::Rgba16Float} { return Err("sprites require an HDR input".into()); }
        if GAME_SPRITE.lock().unwrap().0 != self.game {
            (self.game, self.pipeline) = Self::pipeline(gpu, &self.layout, &self.image_layout, wgpu::TextureFormat::Rgba16Float);
        }
        if ldr && self.ldr_pipeline.as_ref().is_none_or(|(version,format,_)|*version!=self.game||*format!=input.format()) {
            let (version,pipeline)=Self::pipeline(gpu,&self.layout,&self.image_layout,input.format());
            self.ldr_pipeline=Some((version,input.format(),pipeline));
        }
        order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let inverse = view.inverse();
        let cam_right = inverse.x_axis.truncate().normalize_or_zero();
        let gpu_sprites: Vec<GpuSprite> = order.iter().map(|&(i, _)| {
            let s = &self.sprites[i];
            let (right, up) = match s.facing {
                Facing::Billboard => { let r = Vec3::new(cam_right.x, 0.0, cam_right.z).try_normalize().unwrap_or(Vec3::X); (r, Vec3::Y) }
                Facing::Screen => (cam_right, inverse.y_axis.truncate().normalize_or_zero()),
                Facing::Fixed { right, up } => (right, up),
                Facing::Axis { up } => {
                    let to_camera = camera.pos - s.center;
                    let r = up.cross(to_camera).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
                    (r, up)
                }
            };
            let mut size=s.size;
            // A glow must remain round when small or viewed along the bullet path.
            // Use this eye's actual projection, including asymmetric VR frusta.
            if s.facing==Facing::Screen && s.min_pixels>0.0 {
                let c=output_vp*s.center.extend(1.);
                let edge=output_vp*(s.center+right*size[0]).extend(1.);
                if c.w>camera.near && edge.w>camera.near {
                    let ndc=edge.truncate()/edge.w-c.truncate()/c.w;
                    let pixels=(glam::Vec2::new(ndc.x*input.width() as f32,ndc.y*input.height() as f32)*0.5).length();
                    let grow=(s.min_pixels/pixels.max(0.001)).max(1.);
                    size=size.map(|v|v*grow);
                }
            }
            GpuSprite { center: s.center.extend(s.depth_bias).to_array(), right: right.extend(size[0]).to_array(), up: up.extend(size[1]).to_array(),
                uv: s.uv, color: s.radiance.extend(s.opacity.clamp(0.0, 1.0)).to_array(), misc: [s.alpha_cut, if s.additive { 1.0 } else { 0.0 }, if s.facing==Facing::Screen {0.0}else{s.min_pixels.max(0.0)}, s.effect as f32], params: s.params }
        }).collect();
        let size = (input.width(), input.height());
        if self.targets.len() <= index { self.targets.resize_with(index + 1, || None); }
        if self.targets[index].as_ref().is_none_or(|t| t.size != size) {
            let output = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("sprite HDR output (not temporal history)"), size: input.size(), mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: input.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
            let buffer = |label, size, usage| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            self.targets[index] = Some(Target { size, output,
                uniform: buffer("sprites view", std::mem::size_of::<Uniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
                instances: buffer("sprites", (MAX_SPRITES * std::mem::size_of::<GpuSprite>()) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST) });
        }
        let t = self.targets[index].as_ref().unwrap();
        let uniform = Uniform { view_proj: output_vp.to_cols_array_2d(), depth_vp: depth_vp.to_cols_array_2d(),
            camera_near: camera.pos.extend(camera.near).to_array(), camera_right: cam_right.extend(size.0 as f32).to_array(),output:[exposure,if input.format().is_srgb(){1.0}else{0.0},0.,0.] };
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&uniform));
        gpu.queue.write_buffer(&t.instances, 0, bytemuck::cast_slice(&gpu_sprites));
        if !ldr {enc.copy_texture_to_texture(input.as_image_copy(), t.output.as_image_copy(), input.size());}
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sprites view"), layout: &self.layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: t.uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: t.instances.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) }] });
        let output = if ldr {input}else{&t.output}.create_view(&Default::default());
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("sprites"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &output, depth_slice: None,
            resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        pass.set_pipeline(if ldr {&self.ldr_pipeline.as_ref().unwrap().2}else{&self.pipeline});
        pass.set_bind_group(0, &bind, &[]);
        // Back to front; one draw per run of the same image.
        let mut start = 0;
        while start < order.len() {
            let image = self.sprites[order[start].0].image;
            let mut end = start + 1;
            while end < order.len() && self.sprites[order[end].0].image == image { end += 1; }
            pass.set_bind_group(1, &self.images[&image], &[]);
            pass.draw(0..6, start as u32..end as u32);
            start = end;
        }
        drop(pass);
        Ok(Some(if ldr {input.clone()}else{t.output.clone()}))
    }
}

pub fn shader_source() -> String { shader_with(None) }

/// The game's sprite effects, and how often they were replaced.
static GAME_SPRITE: Mutex<(u64, Option<String>)> = Mutex::new((0, None));

/// Without a game's own: every effect draws nothing.
const NO_GAME_SPRITE: &str = "fn game_sprite(s: GameSprite) -> vec4<f32> { return vec4<f32>(0.0); }";

fn shader_with(game: Option<&str>) -> String {
    format!("{SHADER}\n{}\n", game.unwrap_or(NO_GAME_SPRITE))
}

/// A game's own sprite looks: WGSL with
/// `fn game_sprite(s: GameSprite) -> vec4<f32>`, run for every sprite whose
/// `effect` is not 0. `GameSprite` holds `uv` (0..1 across the quad, y
/// down), `size` (metres), `world`, `effect`, `params` and `texel` (the
/// image under `uv`, premultiplied). It returns a colour that the sprite's
/// radiance scales and its coverage in alpha. Checked here: on an error
/// the shader in use stays. Sprites pick it up on their next frame.
pub fn set_game_sprite(gpu: &Gpu, source: &str) -> Result<(), String> {
    let full = shader_with(Some(source));
    crate::shaders::checked(gpu, || gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("game sprites"), source: wgpu::ShaderSource::Wgsl(full.into()) }))?;
    let mut g = GAME_SPRITE.lock().unwrap();
    g.0 += 1;
    g.1 = Some(source.to_owned());
    Ok(())
}

const SHADER: &str = r#"
struct Uniform { view_proj: mat4x4<f32>, depth_vp: mat4x4<f32>, camera_near: vec4<f32>, camera_right: vec4<f32>, output:vec4<f32> };
struct Sprite { center: vec4<f32>, right: vec4<f32>, up: vec4<f32>, uv: vec4<f32>, color: vec4<f32>, misc: vec4<f32>, params: vec4<f32> };
struct GameSprite { uv: vec2<f32>, size: vec2<f32>, world: vec3<f32>, effect: u32, params: vec4<f32>, texel: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var<storage, read> sprites: array<Sprite>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) world: vec3<f32>, @location(2) @interpolate(flat) id: u32, @location(3) local: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> Out {
    let s = sprites[i];
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    let c = corners[v];
    let along = s.center.xyz + s.up.xyz * (0.5 - c.y) * s.up.w;
    var width = s.right.w;
    if s.misc.z > 0.0 {
        // Pixels per metre across the sprite here (camera_right.w: width px).
        let a = u.view_proj * vec4<f32>(along, 1.0);
        let b = u.view_proj * vec4<f32>(along + s.right.xyz * 0.01, 1.0);
        if a.w > 1e-4 && b.w > 1e-4 {
            let px = length(b.xy / b.w - a.xy / a.w) * 0.5 * u.camera_right.w / 0.01;
            width = max(width, s.misc.z / max(px, 1e-6));
        }
    }
    let world = along + s.right.xyz * (c.x - 0.5) * width;
    var out: Out;
    out.position = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = s.uv.xy + c * s.uv.zw;
    out.world = world;
    out.id = i;
    out.local = c;
    return out;
}
fn depth_metres(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    return u.camera_near.w / max(textureLoad(scene_depth, px, 0), 1e-9);
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    let s = sprites[in.id];
    var texel = textureSample(image, image_sampler, in.uv);
    if s.misc.w > 0.5 {
        // The game's effect: straight colour and coverage, made premultiplied.
        let g = game_sprite(GameSprite(in.local, vec2<f32>(s.right.w, s.up.w), in.world, u32(s.misc.w + 0.5), s.params, texel));
        let ga = clamp(g.a, 0.0, 1.0);
        texel = vec4<f32>(max(g.rgb, vec3<f32>(0.0)) * ga, ga);
    }
    let clip = u.depth_vp * vec4<f32>(in.world, 1.0);
    let duv = clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
    if depth_metres(duv) + s.center.w < clip.w { discard; }
    if texel.a <= s.misc.x || texel.a <= 0.0 { discard; }
    // Linear premultiplied texel; cut-out sprites draw opaque inside.
    let a = select(texel.a, 1.0, s.misc.x > 0.0) * s.color.a;
    let rgb = texel.rgb / max(texel.a, 1.0 / 255.0) * s.color.rgb;
    // The HDR target is fp16: keep the hue and leave headroom for what an
    // additive sprite lands on (a very bright, thin laser may ask for more).
    var light = rgb * a;
    let peak = max(light.r, max(light.g, light.b));
    light *= min(1.0, 40000.0 / max(peak, 1e-6));
    if u.output.x>0.0 {
        // Additive alpha scales emitted energy BEFORE tone mapping. Applying
        // it afterwards extinguishes bright low-coverage trails and glows.
        let additive=s.misc.y>0.5;
        let exposed=max(rgb*select(1.0,a,additive)*u.output.x,vec3<f32>(0.0));
        var mapped=sqrt(exposed/(1.0+exposed));
        if u.output.y>0.5 {mapped=select(mapped/12.92,pow((mapped+0.055)/1.055,vec3<f32>(2.4)),mapped>vec3<f32>(0.04045));}
        light=mapped*select(a,1.0,additive);
    }
    // Additive: premultiplied with zero alpha keeps what is behind.
    return vec4<f32>(light, select(a, 0.0, s.misc.y > 0.5));
}
"#;

/// Lazily created sprite renderer shared by all views.
#[derive(Default)]
pub(crate) struct SpriteSlot(pub Mutex<Option<SpriteRenderer>>);

impl SpriteSlot {
    fn with<R>(&self, gpu: &Gpu, f: impl FnOnce(&mut SpriteRenderer) -> R) -> R {
        let mut g = self.0.lock().unwrap();
        f(g.get_or_insert_with(|| SpriteRenderer::new(gpu)))
    }
    pub fn set(&self, gpu: &Gpu, sprites: &[Sprite]) -> Result<(), String> {
        if sprites.len() > MAX_SPRITES { return Err(format!("at most {MAX_SPRITES} sprites")); }
        if sprites.iter().any(|s| !(s.center.is_finite() && s.radiance.is_finite() && s.size.iter().chain(&s.uv).all(|v| v.is_finite()))) { return Err("non-finite sprite".into()); }
        if sprites.is_empty() && self.0.lock().unwrap().is_none() { return Ok(()); }
        self.with(gpu, |r| { r.sprites.clear(); r.sprites.extend_from_slice(sprites); });
        Ok(())
    }
    pub fn set_picture(&self, gpu: &Gpu, id: SpriteImage, picture: &Picture, filter: ImageFilter) { self.with(gpu, |r| r.set_picture(gpu, id, picture, filter)); }
    pub fn set_texture(&self, gpu: &Gpu, id: SpriteImage, texture: &wgpu::Texture, filter: ImageFilter) { self.with(gpu, |r| r.bind_texture(gpu, id, texture, filter)); }
    pub fn remove(&self, id: SpriteImage) { if let Some(r) = self.0.lock().unwrap().as_mut() { r.images.remove(&id); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Material;
    use glam::Affine3A;

    #[test]
    fn sprite_wgsl_validates_and_layouts_match() {
        let source = shader_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<GpuSprite>(), 112);
        assert_eq!(std::mem::size_of::<Uniform>(), 176);
    }

    #[test]
    fn irradiance_follows_range_cone_and_facing() {
        let lamp = LocalLight { position: Vec3::new(0.0, 0.0, 1.0), direction: Vec3::NEG_Z, intensity_cd: Vec3::splat(100.0), range_m: 5.0,
            inner_cos: 0.9, outer_cos: 0.8, source_radius_m: 0.05, falloff: 2.0 };
        let facing = irradiance(&[lamp], Vec3::ZERO, Vec3::Z, Vec3::ZERO);
        assert!((facing.x - 100.0 * (1.0f32 - 1.0 / 625.0).powi(2)).abs() < 0.01, "{facing}");
        assert!(irradiance(&[lamp], Vec3::ZERO, Vec3::NEG_Z, Vec3::ZERO).x < 1e-3, "back face");
        assert_eq!(irradiance(&[lamp], Vec3::new(3.0, 0.0, 0.0), Vec3::Z, Vec3::ONE), Vec3::ONE, "outside the cone");
    }

    /// A red billboard in front of a grey wall shows; one behind the wall
    /// does not; a glowing canvas tag shows its drawn colour.
    #[test]
    fn gpu_sprites_draw_in_front_of_and_hide_behind_the_scene() {
        let mut gpu = Gpu::new().unwrap();
        // Pixel/occlusion assertions do not depend on GPU timer availability.
        gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY|wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
        for minimum in [false,true] {
        let mut scene = crate::Scene::default();
        let grey = scene.add_material(Material { tint: [0.3; 3], layer: -1, roughness: 1.0, flags: crate::scene::EMISSIVE, scale: 300.0, metal: 0.0 });
        let wall = scene.add_mesh(&crate::primitives::quad(grey).mesh());
        scene.add_instance(wall, Affine3A::from_scale(Vec3::new(4.0, 2.0, 1.0)));
        let mut light = crate::Lighting::default();
        light.sun_lux = 0.0;
        light.sky_lux = 0.0;
        scene.graphics.minimal_forward=minimum;
        let renderer = crate::Renderer::new(&gpu, &scene, &light);
        renderer.set_sprite_image(&gpu, SpriteImage(1), &Picture::solid(2, 2, [255, 255, 255, 255]), ImageFilter::Nearest);
        let mut canvas = Canvas::new(&gpu, [8, 8]).unwrap();
        let mut list = HudDrawList::new([8, 8]);
        list.rect([0.0, 0.0, 8.0, 8.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 8.0, 8.0]);
        canvas.draw(&gpu, &list).unwrap();
        renderer.set_sprite_texture(&gpu, SpriteImage(2), canvas.texture(), ImageFilter::Linear);
        let red = Vec3::new(4000.0, 0.0, 0.0);
        renderer.set_sprites(&gpu, &[
            Sprite::billboard(Vec3::new(-0.6, 0.0, 0.5), [0.5, 0.5], SpriteImage(1), [0.0, 0.0, 1.0, 1.0], red),
            Sprite::billboard(Vec3::new(0.0, 0.0, -0.5), [0.5, 0.5], SpriteImage(1), [0.0, 0.0, 1.0, 1.0], red),
            Sprite::fixed(Vec3::new(0.6, 0.0, 0.5), [0.5, 0.5], Vec3::X, Vec3::Y, SpriteImage(2), [0.0, 0.0, 1.0, 1.0], Vec3::splat(4000.0)),
        ]).unwrap();
        let camera = Camera { pos: Vec3::new(0.0, 0.0, 3.0), target: Vec3::ZERO, fov_y: 50f32.to_radians(), near: 0.1 };
        let (w, h) = (96, 64);
        let (rgba, _) = renderer.render(&gpu, &camera, &light, w, h);
        let px = |x: u32, y: u32| { let i = ((y * w + x) * 4) as usize; [rgba[i], rgba[i + 1], rgba[i + 2]] };
        let (front, behind, tag) = (px(w / 2 - 11, h / 2), px(w / 2, h / 2), px(w / 2 + 11, h / 2));
        assert!(front[0] > 100 && front[0] > 4 * front[1].max(1), "red sprite in front: {front:?}");
        assert!((front[0] as i32 - behind[0] as i32).abs() > 60 && behind[0] == behind[1], "sprite behind the wall is hidden: {behind:?}");
        assert!(tag[1] > 100 && tag[1] > 4 * tag[0].max(1), "canvas tag is green: {tag:?}");
        }
    }
    #[test]
    fn minimum_additive_trails_keep_radiance_through_low_alpha_tails() {
        let gpu=Gpu::new().unwrap();let mut scene=crate::Scene::default();scene.graphics.minimal_forward=true;
        let material=scene.add_material(Material{tint:[0.;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
        let mesh=scene.add_mesh(&crate::primitives::quad(material).mesh());scene.add_instance(mesh,Affine3A::IDENTITY);
        let light=crate::Lighting{sun_lux:0.,sky_lux:0.,ev100:10.,..Default::default()};
        let renderer=crate::Renderer::new(&gpu,&scene,&light);
        // Low alpha models a faint texel in the tapered tail. It scales emitted
        // energy, not opacity after tone mapping. The dark wall tests occlusion.
        renderer.set_sprite_image(&gpu,SpriteImage(90),&Picture::solid(2,2,[255,255,255,8]),ImageFilter::Nearest);
        let sprite=|z|Sprite::shaft(Vec3::new(0.,0.,z),[0.5,1.5],Vec3::Y,SpriteImage(90),[0.,0.,1.,1.],Vec3::new(50000.,0.,0.));
        let camera=Camera{pos:Vec3::new(0.,0.,3.),target:Vec3::ZERO,fov_y:1.,near:0.05};
        let views=[crate::ViewRect{cam:camera,x:0,y:0,w:64,h:64}];
        for format in [wgpu::TextureFormat::Rgba8Unorm,wgpu::TextureFormat::Rgba8UnormSrgb] {
            let target=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("additive trail regression"),size:wgpu::Extent3d{width:64,height:64,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
            for (z,visible) in [(0.5,true),(-0.5,false)] {
                renderer.set_sprites(&gpu,&[sprite(z)]).unwrap();
                renderer.try_render_eye_targets(&gpu,&views,&light,&[&target]).unwrap();
                let image=gpu.read_rgba8(&target);let red=image[(32*64+32)*4];
                if visible {assert!(red>150,"low-alpha trail lost its energy in {format:?}: {red}");}
                else {assert_eq!(red,0,"trail leaks through wall in {format:?}");}
            }
        }
    }

}
