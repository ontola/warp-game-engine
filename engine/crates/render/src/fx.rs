//! Blast effects: fire and smoke billows, flame tongues, ember streaks, a
//! refracting heat shell and the flash and fireball lights, ported from
//! Growing Guns' `violence.gd` (`spawn_bullet_blast`). Drawn in HDR after
//! temporal reconstruction (no ghosting, never in TAA history), clipped and
//! softened against the opaque depth. All motion is evaluated on the GPU
//! from one clock: the game spawns, then only sends the time.
//!
//! ```ignore
//! let mut fx = Effects::default();
//! fx.explosion(time, pos, 6.5, Vec3::new(1.0, 0.7, 0.2));   // on the event
//! fx.flush(&gpu, &renderer, time)?;                          // per frame
//! lights.extend(fx.lights(time));                            // local lights
//! ```
use crate::headlights::LocalLight;
use crate::{Camera, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Mutex;

/// Local lights accept at most 1e8 (lux at 1 m); near a blast the fp16 HDR
/// target saturates anyway.
const MAX_CD: f32 = 100_000_000.0;
const SPHERE_VERTICES: u32 = 16 * 9 * 6;
const MAX_PUFFS: usize = 16384;
/// Heat shells only for blasts at least this big (Godot BLAST_DISTORTION_MIN_RADIUS).
pub const DISTORTION_MIN_RADIUS: f32 = 5.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Puff {
    pub center_kind: [f32; 4],
    pub ax: [f32; 4],
    pub ay: [f32; 4],
    pub az: [f32; 4],
    pub body: [f32; 4],
    pub timing: [f32; 4],
    pub extra: [f32; 4],
}

const SMOKE: f32 = 0.0;
const FIRE: f32 = 1.0;
const EMBER: f32 = 2.0;
const HEAT: f32 = 3.0;

impl Puff {
    fn kind(&self) -> u32 { self.center_kind[3] as u32 }
    fn end(&self) -> f32 { self.timing[0] + self.timing[1] }
}

/// A point light that flashes and fades (Godot's blast lights).
#[derive(Clone, Copy, Debug)]
pub struct LightPop { pub pos: Vec3, pub birth: f32, pub kind: u8, pub radius: f32, pub color: Vec3 }

/// Godot TRANS_EXPO EASE_OUT.
fn expo_out(t: f32) -> f32 { if t >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) } }

/// Live effects owned by the game.
pub struct Effects {
    puffs: Vec<Puff>,
    pops: Vec<LightPop>,
    rng: u64,
    changed: bool,
    /// Candela per unit of Godot light energy: Godot lights a surface at
    /// distance d with energy / d^attenuation times its sun, so one unit at
    /// 1 m is about a sun's worth (37,000 lux horizontal at midday).
    pub light_scale: f32,
}

impl Default for Effects {
    fn default() -> Self { Effects { puffs: Vec::new(), pops: Vec::new(), rng: 0x2545_F491_4F6C_DD1D, changed: false, light_scale: 37_000.0 } }
}

impl Effects {
    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }

    pub fn len(&self) -> usize { self.puffs.len() }
    pub fn is_empty(&self) -> bool { self.puffs.is_empty() }
    pub fn clear(&mut self) { self.puffs.clear(); self.pops.clear(); self.changed = true; }

    /// A full blast at `time` (seconds on the clock passed to `flush`):
    /// smoke, fireball, flame tongues, embers, heat shell and lights.
    /// `color` tints embers (Godot passes the bullet colour).
    pub fn explosion(&mut self, time: f32, pos: Vec3, radius: f32, color: Vec3) {
        let r = radius.max(0.5);
        // Smoke: dark, lingering, rising (spawn_blast_fire_smoke).
        let n = ((r * 1.1) as i32).clamp(8, 24) as usize;
        self.billows(time, pos, r, n, false, 0.5 + r * 0.13, 0.5, 1.3, r * 0.7, 1.0, 0.0);
        // Fireball (spawn_blast_fire_clouds).
        let n = ((r * 0.8) as i32).clamp(6, 18) as usize;
        self.billows(time, pos, r, n, true, 0.35 + r * 0.055, 0.3, 0.7, r * 0.3, 1.0, 0.0);
        // Flame tongues: stretched fire puffs, a beat later (spawn_blast_flame_shards).
        let n = ((r * 0.7) as i32).clamp(5, 12) as usize;
        self.billows(time, pos, r, n, true, 0.34 + r * 0.05, 0.28, 0.6, r * 0.28, 2.6, 0.14);
        self.embers(time, pos, r, color);
        if r >= DISTORTION_MIN_RADIUS {
            let duration = (0.42 + r * 0.018).clamp(0.42, 0.95);
            let strength = (r * 0.026).clamp(0.11, 0.26);
            self.puffs.push(Puff { center_kind: pos.extend(HEAT).to_array(), ax: [r * 2.2, 0.0, 0.0, 0.0], timing: [time, duration, 0.0, 0.0],
                extra: [strength, 0.0, 0.0, 0.0], body: [0.0, 0.0, 0.0, 1.0], ..Default::default() });
        }
        self.pops.push(LightPop { pos, birth: time, kind: 0, radius: r, color });
        self.pops.push(LightPop { pos, birth: time, kind: 1, radius: r, color });
        self.changed = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn billows(&mut self, time: f32, pos: Vec3, r: f32, count: usize, fire: bool, life: f32, scale_lo: f32, scale_hi: f32, rise: f32, stretch: f32, delay_bias: f32) {
        let spread = r * if fire { 0.45 } else { 0.65 };
        let base = pos + Vec3::Y * r * 0.1;
        for _ in 0..count {
            let ang = self.f() * std::f32::consts::TAU;
            let rad = self.range(0.0, spread);
            let mut off = Vec3::new(ang.cos() * rad, self.range(-0.2, 0.6) * r * 0.45, ang.sin() * rad);
            let s = r * self.range(scale_lo, scale_hi);
            let (mut ax, mut ay, mut az) = (Vec3::X * s, Vec3::Y * s, Vec3::Z * s);
            if stretch > 1.01 {
                // Tongues fire off radially, long axis along their direction.
                let azim = self.f() * std::f32::consts::TAU;
                let elev = self.range(-0.25, 0.85);
                let dir = Vec3::new(azim.cos() * elev.cos(), elev.sin(), azim.sin() * elev.cos());
                off = dir * (rad + r * 0.15);
                let up = if dir.y.abs() < 0.95 { Vec3::Y } else { Vec3::X };
                let ex = up.cross(dir).normalize();
                let ez = dir.cross(ex);
                (ax, ay, az) = (ex * s, dir * s * stretch, ez * s);
            }
            let seed = self.f();
            let delay = delay_bias + self.range(0.0, if fire { 0.22 } else { 0.12 });
            let (heat, body, opk) = if fire {
                ((1.0 - (rad / spread.max(0.001)).clamp(0.0, 1.0)).powf(1.4), Vec3::ONE, self.range(0.75, 0.95))
            } else {
                let h = self.range(0.2, 0.55);
                let g = self.range(0.07, 0.26);
                (h, Vec3::new(g * 1.08, g, g * 0.9), self.range(0.6, 0.85))
            };
            let speed = self.range(0.0, 0.8);
            let tail = if fire { 2.2 } else { 0.38 };
            self.puffs.push(Puff {
                center_kind: (base + off).extend(if fire { FIRE } else { SMOKE }).to_array(),
                ax: ax.extend(seed).to_array(), ay: ay.extend(delay).to_array(), az: az.extend(heat).to_array(),
                body: body.extend(opk).to_array(),
                timing: [time, life, rise, speed],
                extra: [0.32, 0.35, tail, 2.1],
            });
        }
    }

    fn embers(&mut self, time: f32, pos: Vec3, r: f32, color: Vec3) {
        let count = ((r * 5.0) as i32).clamp(30, 110);
        let hot = color.lerp(Vec3::new(1.0, 0.86, 0.5), 0.4);
        let life = 0.3 + r * 0.03;
        for _ in 0..count {
            let theta = self.f() * std::f32::consts::TAU;
            let elev = self.range(-0.2, 1.35);
            let dir = Vec3::new(theta.cos() * elev.cos(), elev.sin(), theta.sin() * elev.cos()).normalize();
            let dist = r * self.range(0.8, 2.0);
            let drop = r * self.range(0.2, 0.7);
            let travel = dir * dist + Vec3::NEG_Y * drop;
            let sz = self.range(0.4, 1.15);
            let streak = 1.0 + (dist / r).clamp(0.0, 2.0) * self.range(1.6, 3.6);
            let delay = self.range(0.0, 0.15);
            let bright = self.range(0.6, 1.0);
            self.puffs.push(Puff {
                center_kind: pos.extend(EMBER).to_array(),
                ax: travel.extend(delay).to_array(),
                body: hot.extend(bright).to_array(),
                timing: [time, life, 0.0, 0.0],
                extra: [sz, ((streak - 1.0) / 7.0).clamp(0.0, 1.0), 1.0, 5.0],
                ..Default::default()
            });
        }
    }

    /// The blast lights at `time`: a hot white flash and the longer orange
    /// fireball glow (Godot `_spawn_blast_flash_light`,
    /// `_spawn_blast_fireball_light`).
    pub fn lights(&self, time: f32) -> Vec<LocalLight> {
        let hot = Vec3::new(1.0, 0.995, 0.98);
        let warm = Vec3::new(1.0, 0.54, 0.12);
        let mut out = Vec::new();
        for p in self.pops.iter().rev() {
            let t = time - p.birth;
            if t < 0.0 { continue; }
            let r = p.radius;
            // Godot falloff: flash d^-1 (default attenuation), fireball d^-0.42.
            let falloff = if p.kind == 0 { 1.0 } else { 0.42 };
            let (energy, color, range) = if p.kind == 0 {
                let peak = 6.0 + r * 6.5;
                let hold = (0.018 + r * 0.0035).clamp(0.018, 0.13);
                let decay = (0.05 + r * 0.008).clamp(0.05, 0.32);
                if t > hold + decay { continue; }
                let e = if t < hold { peak } else { peak * (1.0 - expo_out((t - hold) / decay)) };
                (e, hot.lerp(warm, (t / (hold + decay)).clamp(0.0, 1.0)), (r * 6.5).clamp(45.0, 190.0))
            } else {
                let grow = (0.12 + r * 0.007).clamp(0.12, 0.45);
                let fade = (0.08 + r * 0.005).clamp(0.08, 0.28);
                let life = (grow + fade) * 1.8;
                if t > life { continue; }
                let hold = life * 0.5;
                let sustain = 178.0 * (r / 30.0).clamp(0.0, 1.0).powf(0.92);
                let spike_s = (grow * 0.2).min(0.045);
                let settle_s = (grow * 0.25).min(0.055);
                let e = if t < spike_s { sustain * 1.45 }
                    else if t < spike_s + settle_s { sustain * (1.45 - 0.45 * (1.0 - (1.0 - (t - spike_s) / settle_s).powi(2))) }
                    else if t < hold { sustain }
                    else { sustain * (1.0 - ((t - hold) / (life - hold)).powi(2)) };
                let c0 = p.color.lerp(hot, 0.45);
                let c1 = p.color.lerp(warm, 0.5);
                let c = if t < life * 0.45 { c0.lerp(c1, t / (life * 0.45)) } else { c1.lerp(c0.lerp(warm, 0.65), (t - life * 0.45) / (life * 0.55)) };
                let ball = (r * 0.58).max(0.5);
                let range = (ball * 1.2).clamp(10.0, 36.0) + ((ball * 3.8).clamp(16.0, 92.0) - (ball * 1.2).clamp(10.0, 36.0)) * (t / life);
                (e, c, range)
            };
            if energy <= 0.0 { continue; }
            out.push(LocalLight { position: p.pos, direction: Vec3::NEG_Y, intensity_cd: (color * energy * self.light_scale).min(Vec3::splat(MAX_CD)), range_m: range.min(200.0),
                inner_cos: -0.99, outer_cos: -1.0, source_radius_m: (r * 0.3).clamp(0.2, 1.0), falloff });
        }
        out
    }

    /// Drop finished effects and send the live ones (only when changed).
    pub fn flush(&mut self, gpu: &Gpu, renderer: &crate::Renderer, time: f32) -> Result<(), String> {
        let before = self.puffs.len();
        self.puffs.retain(|p| p.end() > time);
        self.pops.retain(|p| time - p.birth < 2.0);
        // Oldest go first when over budget.
        if self.puffs.len() > MAX_PUFFS { let extra = self.puffs.len() - MAX_PUFFS; self.puffs.drain(..extra); }
        let changed = std::mem::take(&mut self.changed) || self.puffs.len() != before;
        renderer.set_effects(gpu, time, changed.then_some(&self.puffs[..]))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform { view_proj: [[f32; 4]; 4], depth_vp: [[f32; 4]; 4], view: [[f32; 4]; 4], camera_near: [f32; 4], size: [f32; 4], clock: [f32; 4] }

struct Target { size: (u32, u32), output: wgpu::Texture, uniform: wgpu::Buffer, order: wgpu::Buffer }

/// Renderer side, created on first use.
pub(crate) struct FxRenderer {
    layout: wgpu::BindGroupLayout,
    billow: wgpu::RenderPipeline,
    ember: wgpu::RenderPipeline,
    heat: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    puffs: wgpu::Buffer,
    source: Vec<Puff>,
    time: f32,
    targets: Vec<Option<Target>>,
}

pub fn shader_source() -> &'static str { include_str!("shaders/fx.wgsl") }

impl FxRenderer {
    fn new(gpu: &Gpu) -> Self {
        let entries: Vec<_> = (0..6).map(|binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, count: None,
            ty: match binding {
                0 => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                1 | 2 => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                3 => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                4 => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                _ => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            } }).collect();
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fx"), entries: &entries });
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fx"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fx"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let additive = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } };
        let pipe = |entry: &str, blend: wgpu::BlendState| gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some(entry), layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("fx_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() }, depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache() });
        let billow = pipe("fx_billow", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let ember = pipe("fx_ember", additive);
        let heat = pipe("fx_heat", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fx background"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let puffs = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fx puffs"), size: (MAX_PUFFS * std::mem::size_of::<Puff>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        FxRenderer { layout, billow, ember, heat, sampler, puffs, source: Vec::new(), time: 0.0, targets: Vec::new() }
    }

    fn set(&mut self, gpu: &Gpu, time: f32, puffs: Option<&[Puff]>) {
        self.time = time;
        if let Some(p) = puffs {
            let p = &p[..p.len().min(MAX_PUFFS)];
            self.source.clear();
            self.source.extend_from_slice(p);
            if !p.is_empty() { gpu.queue.write_buffer(&self.puffs, 0, bytemuck::cast_slice(p)); }
        }
    }

    /// Draw over a copy of `input`; returns the new HDR output, or None when
    /// nothing is live.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, index: usize, input: &wgpu::Texture, depth: &wgpu::TextureView,
        camera: Camera, view: Mat4, output_vp: Mat4, depth_vp: Mat4, white: f32) -> Result<Option<wgpu::Texture>, String> {
        if self.source.iter().all(|p| p.end() <= self.time || p.timing[0] > self.time) { return Ok(None); }
        // Heat first (it refracts the plain scene), then billows back to front, then embers.
        let mut heat = Vec::new();
        let mut billows = Vec::new();
        let mut embers = Vec::new();
        for (i, p) in self.source.iter().enumerate() {
            if p.end() <= self.time || p.timing[0] > self.time { continue; }
            let c = Vec3::from_slice(&p.center_kind[..3]);
            match p.kind() {
                0 | 1 => billows.push((i as u32, c.distance_squared(camera.pos))),
                2 => embers.push(i as u32),
                _ => heat.push(i as u32),
            }
        }
        billows.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let ids: Vec<u32> = heat.iter().copied().chain(billows.iter().map(|b| b.0)).chain(embers.iter().copied()).collect();
        let (nh, nb, ne) = (heat.len() as u32, billows.len() as u32, embers.len() as u32);
        if input.format() != wgpu::TextureFormat::Rgba16Float || !input.usage().contains(wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING) {
            return Err("fx requires copyable sampleable HDR input".into());
        }
        let size = (input.width(), input.height());
        if self.targets.len() <= index { self.targets.resize_with(index + 1, || None); }
        if self.targets[index].as_ref().is_none_or(|t| t.size != size) {
            let output = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("fx HDR output (not temporal history)"), size: input.size(), mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: input.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
            let buffer = |label, size, usage| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            self.targets[index] = Some(Target { size, output,
                uniform: buffer("fx view", std::mem::size_of::<Uniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
                order: buffer("fx order", (MAX_PUFFS * 4) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST) });
        }
        let t = self.targets[index].as_ref().unwrap();
        let uniform = Uniform { view_proj: output_vp.to_cols_array_2d(), depth_vp: depth_vp.to_cols_array_2d(), view: view.to_cols_array_2d(),
            camera_near: camera.pos.extend(camera.near).to_array(), size: [size.0 as f32, size.1 as f32, 1.0 / size.0 as f32, 1.0 / size.1 as f32],
            clock: [self.time, white, 0.0, 0.0] };
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&uniform));
        gpu.queue.write_buffer(&t.order, 0, bytemuck::cast_slice(&ids));
        enc.copy_texture_to_texture(input.as_image_copy(), t.output.as_image_copy(), input.size());
        let background = input.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fx view"), layout: &self.layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: t.uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: self.puffs.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: t.order.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&background) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(depth) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) }] });
        let output = t.output.create_view(&Default::default());
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("fx"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &output, depth_slice: None,
            resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        pass.set_bind_group(0, &bind, &[]);
        if nh > 0 { pass.set_pipeline(&self.heat); pass.draw(0..SPHERE_VERTICES, 0..nh); }
        if nb > 0 { pass.set_pipeline(&self.billow); pass.draw(0..SPHERE_VERTICES, nh..nh + nb); }
        if ne > 0 { pass.set_pipeline(&self.ember); pass.draw(0..SPHERE_VERTICES, nh + nb..nh + nb + ne); }
        drop(pass);
        Ok(Some(t.output.clone()))
    }
}

/// Lazily created effects renderer shared by all views.
#[derive(Default)]
pub(crate) struct FxSlot(pub Mutex<Option<FxRenderer>>);

impl FxSlot {
    pub fn set(&self, gpu: &Gpu, time: f32, puffs: Option<&[Puff]>) {
        let mut g = self.0.lock().unwrap();
        if g.is_none() && puffs.is_none_or(|p| p.is_empty()) { return; }
        g.get_or_insert_with(|| FxRenderer::new(gpu)).set(gpu, time, puffs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fx_wgsl_validates_and_layouts_match() {
        let module = naga::front::wgsl::parse_str(shader_source()).unwrap_or_else(|e| panic!("{}", e.emit_to_string(shader_source())));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<Puff>(), 112);
        assert_eq!(std::mem::size_of::<Uniform>(), 240);
    }

    #[test]
    fn an_explosion_follows_the_godot_recipe() {
        let mut fx = Effects::default();
        fx.explosion(1.0, Vec3::ZERO, 9.0, Vec3::new(1.0, 0.7, 0.2));
        let count = |k: f32| fx.puffs.iter().filter(|p| p.center_kind[3] == k).count();
        assert_eq!(count(SMOKE), 9);
        assert_eq!(count(FIRE), 7 + 6);
        assert_eq!(count(EMBER), 45);
        assert_eq!(count(HEAT), 1);
        // Smoke lingers longest: 0.5 + 9 * 0.13 s.
        let last = fx.puffs.iter().map(Puff::end).fold(0.0, f32::max);
        assert!((last - (1.0 + 0.5 + 9.0 * 0.13)).abs() < 1e-4);
        // Lights: both at birth, only the fireball glow later, none at the end.
        assert_eq!(fx.lights(1.0).len(), 2);
        assert_eq!(fx.lights(1.2).len(), 1);
        assert!(fx.lights(2.2).is_empty());
        // Small blasts get no heat shell.
        let mut small = Effects::default();
        small.explosion(0.0, Vec3::ZERO, 4.0, Vec3::ONE);
        assert!(small.puffs.iter().all(|p| p.center_kind[3] != HEAT));
    }
}
