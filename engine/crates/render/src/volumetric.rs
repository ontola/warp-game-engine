//! Opt-in volumetric light: single in-scattering from local lights in a
//! homogeneous medium, so floodlights show their beams and warm lamps a haze
//! (engine/docs/volumetric-light.md).
//!
//! Off by default and free when off: `Lighting::volumetric` is `None`, no
//! light carries a scatter weight, and the frame graph skips both passes
//! before touching any resource. When on, it runs after temporal
//! reconstruction (an analytic integral has no noise for TAA or DLSS to
//! hide), at half the internal resolution with a depth-aware upsample, and
//! writes a separate output; it never enters temporal history.
//!
//! Limits: no occlusion (lights are unshadowed, like their surface light),
//! no extinction of the scene behind the medium (the existing distance fog
//! owns that), single scattering, constant density.
use crate::Gpu;
use crate::headlights::{LocalLight, MAX_LOCAL_LIGHTS};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Mutex;

/// Gauss-Legendre nodes per light in the angular parameterisation (keep
/// equal to `SAMPLES` in volumetric.wgsl).
pub const SAMPLES: u32 = 6;
/// Ray length for pixels without depth (keep equal to `SKY_DISTANCE`).
pub const SKY_DISTANCE: f32 = 10_000.0;
/// Outer cone cosine above which the ray is clipped to the cone (half
/// angle below about 87 degrees, so the forward nappe is convex).
const CLIP_COS: f32 = 0.05;

pub fn shader_source() -> &'static str { include_str!("shaders/volumetric.wgsl") }

/// The scattering medium. Radiance added along a view ray is
/// `density * color * intensity * integral(phase * irradiance * transmittance)`
/// for every local light with a positive scatter weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumetric {
    /// Scattering coefficient per metre (also the extinction used inside
    /// the integral). 0.002 is a faint clear night, 0.02 thick mist.
    pub density: f32,
    /// Scattering albedo tint (linear RGB), 1 for neutral air.
    pub color: Vec3,
    /// Henyey-Greenstein g: 0 isotropic, positive forward (bright when
    /// looking towards a lamp), below 0.95 in magnitude.
    pub anisotropy: f32,
    /// Artistic multiplier on the physical result.
    pub intensity: f32,
    /// How much the medium dims the scene behind it: the surface radiance is
    /// multiplied by `exp(-extinction * density * distance)`. 0 (default)
    /// only adds light, as Growing Guns wants; 1 is physical (Godot's fog).
    pub extinction: f32,
    /// Light the medium scatters from everywhere (ambient and bounce light
    /// times the medium albedo, plus its own emission), as radiance it
    /// converges to over a long ray: adds `ambient * (1 - exp(-density * t))`.
    pub ambient: Vec3,
    /// Metres from the camera the medium reaches (Godot's fog length);
    /// 0 is unbounded.
    pub length: f32,
}

impl Default for Volumetric {
    fn default() -> Self { Self { density: 0.004, color: Vec3::ONE, anisotropy: 0.35, intensity: 1.0, extinction: 0.0, ambient: Vec3::ZERO, length: 0.0 } }
}

impl Volumetric {
    pub fn validated(self) -> Result<Self, String> {
        if !(self.density.is_finite() && (0.0..=1.0).contains(&self.density))
            || !self.color.is_finite() || self.color.min_element() < 0.0 || self.color.max_element() > 100.0
            || !(self.anisotropy.is_finite() && self.anisotropy.abs() < 0.95)
            || !(self.intensity.is_finite() && (0.0..=1000.0).contains(&self.intensity))
            || !(self.extinction.is_finite() && (0.0..=1.0).contains(&self.extinction))
            || !self.ambient.is_finite() || self.ambient.min_element() < 0.0
            || !(self.length.is_finite() && self.length >= 0.0) {
            return Err("volumetric light needs density 0..1 per metre, colour 0..100, |anisotropy| < 0.95, intensity 0..1000, extinction 0..1, ambient >= 0 and length >= 0".into());
        }
        Ok(self)
    }
    /// Whether it adds anything at all.
    pub fn active(&self) -> bool { self.density > 0.0 && ((self.intensity > 0.0 && self.color.max_element() > 0.0) || self.extinction > 0.0 || self.ambient.max_element() > 0.0) }

    /// The ray length the medium covers for a ray ending at `tmax`.
    pub fn reach(&self, tmax: f32) -> f32 { if self.length > 0.0 { tmax.min(self.length) } else { tmax } }

    /// Set a field from `Lighting::set` keys: `vol_density`, `vol_anisotropy`,
    /// `vol_intensity`, `vol_r`, `vol_g`, `vol_b`, `vol_extinction`, `vol_length`.
    pub fn set(&mut self, key: &str, value: f32) -> Result<(), String> {
        let mut next = *self;
        match key {
            "vol_density" => next.density = value,
            "vol_anisotropy" => next.anisotropy = value,
            "vol_intensity" => next.intensity = value,
            "vol_r" => next.color.x = value,
            "vol_g" => next.color.y = value,
            "vol_b" => next.color.z = value,
            "vol_extinction" => next.extinction = value,
            "vol_length" => next.length = value,
            _ => return Err(format!("unknown volumetric setting {key}")),
        }
        *self = next.validated()?;
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct GpuLight { pub position_range: [f32; 4], pub direction_outer: [f32; 4], pub intensity_inner: [f32; 4], pub source: [f32; 4] }

/// The lights that scatter, in order, with their intensity scaled by the
/// weight. Weights beyond the slice are zero; non-positive weights drop out.
pub fn scattering_lights(lights: &[LocalLight], weights: &[f32]) -> Vec<GpuLight> {
    lights.iter().zip(weights).filter(|(l, w)| w.is_finite() && **w > 0.0 && l.intensity_cd.max_element() > 0.0)
        .take(MAX_LOCAL_LIGHTS).map(|(l, &w)| GpuLight {
            position_range: l.position.extend(l.range_m).to_array(),
            direction_outer: l.direction.normalize().extend(l.outer_cos).to_array(),
            intensity_inner: (l.intensity_cd * w).extend(l.inner_cos).to_array(),
            source: [l.source_radius_m * l.source_radius_m, l.falloff, if l.outer_cos > CLIP_COS { 1.0 } else { 0.0 }, 0.0],
        }).collect()
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    inverse_depth_vp: [[f32; 4]; 4],
    camera_near: [f32; 4],
    sizes: [f32; 4],
    output: [f32; 4],
    medium: [f32; 4],
    tint: [f32; 4],
    ambient: [f32; 4],
    lights: [GpuLight; MAX_LOCAL_LIGHTS],
    march: [f32; 4],
    sun_dir: [f32; 4],
    sun_e: [f32; 4],
    bounds: [[f32; 4]; 2 * MAX_BOUNDS],
}

/// Bounds (rooms) the medium can be restricted to.
pub const MAX_BOUNDS: usize = 8;
/// Steps per view ray in the bounded, shadowed mode.
pub const MARCH_STEPS: u32 = 12;

// ---- CPU mirror of volumetric.wgsl (tests compare it to quadrature and
// the GPU example compares it to the shader) ----

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 { let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

pub fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = (1.0 + g2 - 2.0 * g * cos_theta).max(1e-4);
    (1.0 - g2) / (4.0 * std::f32::consts::PI * denom * denom.sqrt())
}

/// The ray interval inside the light's outer cone (forward nappe), within
/// [t0, t1]. Empty when `x >= y`.
pub fn clip_cone(o: Vec3, v: Vec3, light: &GpuLight, t0: f32, t1: f32) -> (f32, f32) {
    let axis = Vec3::from_slice(&light.direction_outer);
    let c2 = light.direction_outer[3] * light.direction_outer[3];
    let d0 = o - Vec3::from_slice(&light.position_range);
    let (va, d0a) = (v.dot(axis), d0.dot(axis));
    let qa = va * va - c2;
    let qb = va * d0a - c2 * d0.dot(v);
    let qc = d0a * d0a - c2 * d0.dot(d0);
    if qa.abs() < 1e-4 { return (t0, t1); }
    let disc = qb * qb - qa * qc;
    if disc < 0.0 { return if qa < 0.0 { (1.0, 0.0) } else { (t0, t1) }; }
    let s = disc.sqrt();
    let (r1, r2) = ((-qb - s) / qa, (-qb + s) / qa);
    let (lo, hi) = (r1.min(r2), r1.max(r2));
    let span = if qa < 0.0 {
        if d0a + va * 0.5 * (lo + hi) < 0.0 { return (1.0, 0.0); }
        (lo, hi)
    } else if va > 0.0 { (hi, 1e30) } else { (-1e30, lo) };
    (span.0.max(t0), span.1.min(t1))
}

/// The irradiance the integral assumes at offset `off` from the lamp:
/// `local_light_sample`'s, smoothed inside the source radius.
pub fn irradiance(light: &GpuLight, off: Vec3) -> Vec3 {
    let d2 = off.length_squared();
    let r2 = light.position_range[3] * light.position_range[3];
    if d2 >= r2 { return Vec3::ZERO; }
    let outward = off / d2.sqrt().max(1e-6);
    let cone = smoothstep(light.direction_outer[3], light.intensity_inner[3], outward.dot(Vec3::from_slice(&light.direction_outer)));
    let n = d2 / r2;
    let fade = (1.0 - n * n).max(0.0);
    let (s2, falloff) = (light.source[0], light.source[1]);
    // (d^2 + s^2)^(f/2): the surface model's max(d^2, s^2) outside the source.
    let spread = (d2 + s2).powf(falloff * 0.5);
    Vec3::from_slice(&light.intensity_inner) * (cone * fade * fade / spread)
}

const NODES: [f32; 6] = [-0.9324695, -0.6612094, -0.2386192, 0.2386192, 0.6612094, 0.9324695];
const WEIGHTS: [f32; 6] = [0.1713245, 0.3607616, 0.4679139, 0.4679139, 0.3607616, 0.1713245];

/// `light_scatter` from volumetric.wgsl.
pub fn light_scatter(o: Vec3, v: Vec3, tmax: f32, light: &GpuLight, sigma: f32, g: f32) -> Vec3 {
    let p = Vec3::from_slice(&light.position_range);
    let range = light.position_range[3];
    let rel = p - o;
    let tc = rel.dot(v);
    let h2 = (rel.dot(rel) - tc * tc).max(0.0);
    let r2 = range * range;
    if h2 >= r2 { return Vec3::ZERO; }
    let half_chord = (r2 - h2).sqrt();
    let mut span = ((tc - half_chord).max(0.0), (tc + half_chord).min(tmax));
    if light.source[2] > 0.5 { span = clip_cone(o, v, light, span.0, span.1); }
    if span.1 <= span.0 { return Vec3::ZERO; }
    let (s2, falloff) = (light.source[0], light.source[1]);
    let hp = (h2 + s2).sqrt();
    let tangent = falloff >= 1.75;
    let (x0, x1) = ((span.0 - tc) / hp, (span.1 - tc) / hp);
    let (a0, a1) = if tangent { (x0.atan(), x1.atan()) } else { (x0.asinh(), x1.asinh()) };
    let exponent = if tangent { 1.0 - 0.5 * falloff } else { 0.5 - 0.5 * falloff };
    let jacobian = if tangent { 1.0 / hp } else { 1.0 };
    let (centre, half_width) = (0.5 * (a0 + a1), 0.5 * (a1 - a0));
    let mut acc = 0.0;
    for k in 0..SAMPLES as usize {
        let a = centre + half_width * NODES[k];
        let t = tc + hp * if tangent { a.tan() } else { a.sinh() };
        let off = o + v * t - p;
        let d2 = off.length_squared();
        let dist = d2.sqrt();
        let outward = off / dist.max(1e-6);
        let cone = smoothstep(light.direction_outer[3], light.intensity_inner[3], outward.dot(Vec3::from_slice(&light.direction_outer)));
        let n = d2 / r2;
        let fade = (1.0 - n * n).max(0.0);
        let spread = if exponent != 0.0 { (d2 + s2).powf(exponent) } else { 1.0 };
        let phase = hg_phase(outward.dot(-v), g);
        let transmittance = (-sigma * (t + dist)).exp();
        acc += WEIGHTS[k] * cone * fade * fade * phase * transmittance * spread;
    }
    Vec3::from_slice(&light.intensity_inner) * (acc * half_width * jacobian)
}

/// Radiance the passes add along one view ray (`v` normalised).
pub fn scatter_ray(medium: &Volumetric, lights: &[GpuLight], o: Vec3, v: Vec3, tmax: f32) -> Vec3 {
    let tmax = medium.reach(tmax);
    let sum: Vec3 = lights.iter().map(|l| light_scatter(o, v, tmax, l, medium.density, medium.anisotropy)).sum();
    sum * medium.color * (medium.density * medium.intensity) + medium.ambient * (1.0 - (-medium.density * tmax).exp())
}

/// What the medium leaves of the surface radiance at `distance`.
pub fn transmittance(medium: &Volumetric, distance: f32) -> f32 { (-medium.extinction * medium.density * medium.reach(distance)).exp() }

// ---- GPU passes ----

struct Target { size: (u32, u32), half: (u32, u32), scatter: wgpu::Texture, output: wgpu::Texture, uniform: wgpu::Buffer, occluders: wgpu::Buffer }

struct Passes { scatter: wgpu::ComputePipeline, composite: wgpu::ComputePipeline, sampler: wgpu::Sampler, targets: Vec<Option<Target>> }

impl Passes {
    fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("volumetric"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let pipe = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: None, module: &module,
            entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("volumetric light shapes"),
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        Self { scatter: pipe("scatter"), composite: pipe("composite"), sampler, targets: Vec::new() }
    }
}

#[derive(Default)]
struct State { lights: Vec<GpuLight>, beams: Vec<GpuLight>, passes: Option<Passes>,
    /// Bounded, shadowed mode: occluder boxes (world-to-unit-box rows), the
    /// bounds and how much of the sun scatters in them.
    occluders: Vec<[[f32; 4]; 4]>, ranges: Vec<(u32, u32)>, bounds: Vec<(Vec3, Vec3)>, sun: f32 }

/// Per-renderer state: the scattering lights and lazily created passes.
#[derive(Default)]
pub(crate) struct VolumetricSlot(Mutex<State>);

/// What a view needs to draw volumetric light.
pub(crate) struct ViewInput<'a> {
    pub index: usize,
    pub input: &'a wgpu::Texture,
    pub depth: &'a wgpu::TextureView,
    pub depth_size: (u32, u32),
    pub camera: Vec3,
    pub near: f32,
    pub depth_vp: Mat4,
    /// The sun: direction towards it and its irradiance (lux, RGB).
    pub sun: (Vec3, Vec3),
    /// Light cookies and profiles (`light_shapes`).
    pub light_shapes: &'a wgpu::TextureView,
}

impl VolumetricSlot {
    /// Replace the scattering lights (already validated `lights`).
    pub fn set_lights(&self, lights: &[LocalLight], weights: &[f32]) {
        let mut state = self.0.lock().unwrap();
        if weights.is_empty() && state.lights.is_empty() { return; }
        state.lights = scattering_lights(lights, weights);
    }

    /// Every light that scatters (lights, then beams), for the low tier's
    /// per-pixel beams (forward_lit.wgsl).
    pub fn scattering(&self) -> Vec<GpuLight> {
        let state = self.0.lock().unwrap();
        state.lights.iter().chain(&state.beams).copied().collect()
    }

    /// Replace the scatter-only lights (already validated).
    pub fn set_beams(&self, beams: &[LocalLight]) { self.set_beams_shaped(beams, &[]); }

    /// `set_beams` with a cookie and profile per beam (missing: none); the
    /// bounded, shadowed mode applies them (`light_shapes`).
    pub fn set_beams_shaped(&self, beams: &[LocalLight], shapes: &[crate::light_shapes::LightShape]) {
        let mut state = self.0.lock().unwrap();
        if beams.is_empty() && state.beams.is_empty() { return; }
        state.beams = scattering_lights(beams, &vec![1.0; beams.len()]);
        for (light, shape) in state.beams.iter_mut().zip(shapes) { light.source[3] = shape.packed(); }
    }

    /// The bounded, shadowed mode: the medium fills only `bounds` (at most
    /// [`MAX_BOUNDS`]; empty returns to the unbounded analytic mode), every
    /// scattering light is ray-marched through them with its visibility
    /// tested against `occluders` (unit boxes under these transforms:
    /// walls, furniture, a door), and `sun` times the sun's irradiance
    /// scatters there too, occluded the same way (a door's shaft of day).
    pub fn set_bounded(&self, bounds: &[(Vec3, Vec3)], occluders: &[glam::Affine3A], sun: f32) {
        let mut state = self.0.lock().unwrap();
        state.bounds = bounds.iter().take(MAX_BOUNDS).copied().collect();
        // One list per bound of the boxes within half a metre of it: a sample
        // tests only its own bound's list, for every step of every light
        // (the walls between rooms and a door are in both rooms' lists).
        let rows = |xf: &glam::Affine3A| {
            let m = Mat4::from(*xf);
            // Rows of the inverse: the columns of its transpose.
            (m.determinant().abs() > 1e-12).then(|| { let r = m.inverse().transpose().to_cols_array_2d(); [r[0], r[1], r[2], [0.0; 4]] })
        };
        let mut lists = Vec::new();
        let mut ranges = Vec::new();
        for (lo, hi) in &state.bounds {
            let start = lists.len();
            for xf in occluders {
                let c = Vec3::from(xf.translation);
                let h = Vec3::from(xf.matrix3.x_axis.abs() * 0.5 + xf.matrix3.y_axis.abs() * 0.5 + xf.matrix3.z_axis.abs() * 0.5);
                if (c + h).cmpge(*lo - 0.5).all() && (c - h).cmple(*hi + 0.5).all() && let Some(r) = rows(xf) { lists.push(r); }
            }
            ranges.push((start as u32, (lists.len() - start) as u32));
        }
        state.occluders = lists;
        state.ranges = ranges;
        state.sun = if sun.is_finite() { sun.max(0.0) } else { 0.0 };
    }

    /// Encode both passes for one view; `None` when nothing scatters.
    pub fn render(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, medium: &Volumetric, view: ViewInput) -> Result<Option<wgpu::Texture>, String> {
        if !medium.active() { return Ok(None); }
        let mut state = self.0.lock().unwrap();
        let sun_scatters = !state.bounds.is_empty() && state.sun > 0.0 && view.sun.1.max_element() > 0.0;
        if state.lights.is_empty() && state.beams.is_empty() && !sun_scatters { return Ok(None); }
        let medium = medium.validated()?;
        let State { lights, beams, passes, occluders, ranges, bounds, sun } = &mut *state;
        let lights: Vec<GpuLight> = lights.iter().chain(beams.iter()).take(MAX_LOCAL_LIGHTS).copied().collect();
        let passes = passes.get_or_insert_with(|| Passes::new(gpu));
        let input = view.input;
        let size = (input.width(), input.height());
        let (dw, dh) = view.depth_size;
        let half = (dw.div_ceil(2).max(1), dh.div_ceil(2).max(1));
        if passes.targets.len() <= view.index { passes.targets.resize_with(view.index + 1, || None); }
        let occluder_bytes = (occluders.len().max(64) * 64) as u64;
        if passes.targets[view.index].as_ref().is_none_or(|t| t.size != size || t.half != half || t.occluders.size() < occluder_bytes) {
            let texture = |label, (w, h): (u32, u32), usage| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float, usage, view_formats: &[] });
            let scatter = texture("volumetric scatter (half resolution)", half, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
            let output = texture("volumetric HDR output (not temporal history)", size, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT);
            let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("volumetric view"), size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let occluders = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("volumetric occluders"), size: occluder_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            passes.targets[view.index] = Some(Target { size, half, scatter, output, uniform, occluders });
        }
        let t = passes.targets[view.index].as_ref().unwrap();
        let mut uniform = Uniform::zeroed();
        uniform.inverse_depth_vp = view.depth_vp.inverse().to_cols_array_2d();
        uniform.camera_near = view.camera.extend(view.near).to_array();
        uniform.sizes = [dw as f32, dh as f32, half.0 as f32, half.1 as f32];
        uniform.output = [size.0 as f32, size.1 as f32, 0.0, 0.0];
        uniform.medium = [medium.density, medium.anisotropy, medium.intensity, lights.len() as f32];
        uniform.tint = medium.color.extend(medium.extinction).to_array();
        uniform.ambient = medium.ambient.extend(medium.length).to_array();
        uniform.lights[..lights.len()].copy_from_slice(&lights);
        if !bounds.is_empty() {
            uniform.march = [occluders.len() as f32, bounds.len() as f32, MARCH_STEPS as f32, 0.0];
            for (i, (lo, hi)) in bounds.iter().enumerate() {
                uniform.bounds[2 * i] = lo.extend(ranges[i].0 as f32).to_array();
                uniform.bounds[2 * i + 1] = hi.extend(ranges[i].1 as f32).to_array();
            }
            if *sun > 0.0 && view.sun.0.y > 0.0 {
                uniform.sun_dir = view.sun.0.normalize().extend(1.0).to_array();
                uniform.sun_e = (view.sun.1 * *sun).extend(0.0).to_array();
            }
            if !occluders.is_empty() { gpu.queue.write_buffer(&t.occluders, 0, bytemuck::cast_slice(occluders.as_slice())); }
        }
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&uniform));
        let view_of = |tex: &wgpu::Texture| tex.create_view(&Default::default());
        let (scatter_v, output_v, input_v) = (view_of(&t.scatter), view_of(&t.output), view_of(input));
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let scatter_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("volumetric scatter"), layout: &passes.scatter.get_bind_group_layout(0), entries: &[
            entry(0, t.uniform.as_entire_binding()), entry(1, wgpu::BindingResource::TextureView(view.depth)), entry(2, wgpu::BindingResource::TextureView(&scatter_v)), entry(6, t.occluders.as_entire_binding()),
            entry(7, wgpu::BindingResource::TextureView(view.light_shapes)), entry(8, wgpu::BindingResource::Sampler(&passes.sampler))] });
        let composite_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("volumetric composite"), layout: &passes.composite.get_bind_group_layout(0), entries: &[
            entry(0, t.uniform.as_entire_binding()), entry(1, wgpu::BindingResource::TextureView(view.depth)), entry(3, wgpu::BindingResource::TextureView(&scatter_v)),
            entry(4, wgpu::BindingResource::TextureView(&input_v)), entry(5, wgpu::BindingResource::TextureView(&output_v))] });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("volumetric scatter"), timestamp_writes: None });
            pass.set_pipeline(&passes.scatter);
            pass.set_bind_group(0, &scatter_bind, &[]);
            pass.dispatch_workgroups(half.0.div_ceil(8), half.1.div_ceil(8), 1);
            pass.set_pipeline(&passes.composite);
            pass.set_bind_group(0, &composite_bind, &[]);
            pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        }
        Ok(Some(t.output.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omni(position: Vec3, range: f32, falloff: f32, source: f32) -> LocalLight {
        LocalLight { position, direction: Vec3::NEG_Y, intensity_cd: Vec3::new(1000.0, 800.0, 600.0), range_m: range,
            inner_cos: -0.99, outer_cos: -1.0, source_radius_m: source, falloff }
    }
    fn spot(position: Vec3, direction: Vec3, falloff: f32) -> LocalLight {
        LocalLight { position, direction: direction.normalize(), intensity_cd: Vec3::splat(50_000.0), range_m: 80.0,
            inner_cos: 30f32.to_radians().cos(), outer_cos: 38f32.to_radians().cos(), source_radius_m: 1.0, falloff }
    }

    /// Independent brute-force quadrature of the same medium in t.
    fn reference(light: &GpuLight, o: Vec3, v: Vec3, tmax: f32, sigma: f32, g: f32) -> Vec3 {
        let p = Vec3::from_slice(&light.position_range);
        let end = tmax.min(o.distance(p) + light.position_range[3]) as f64;
        let n = 50_000;
        let mut sum = Vec3::ZERO.as_dvec3();
        for i in 0..n {
            let t = (i as f64 + 0.5) / n as f64 * end;
            let x = o + v * t as f32;
            let off = x - p;
            let e = irradiance(light, off);
            if e == Vec3::ZERO { continue; }
            let phase = hg_phase((off / off.length().max(1e-6)).dot(-v), g);
            let tr = (-(sigma as f64) * (t + off.length() as f64)).exp();
            sum += e.as_dvec3() * (phase as f64 * tr * end / n as f64);
        }
        sum.as_vec3()
    }

    fn close(a: Vec3, b: Vec3, rel: f32, what: &str) {
        let scale = b.max_element().max(1e-6);
        assert!((a - b).abs().max_element() <= rel * scale, "{what}: integral {a} vs quadrature {b} (rel {})", (a - b).abs().max_element() / scale);
    }

    #[test]
    fn omni_integral_matches_numeric_quadrature() {
        let o = Vec3::new(0.0, 1.7, 0.0);
        for (light, falloff) in [(omni(Vec3::new(4.0, 3.0, -12.0), 20.0, 2.0, 0.15), 2.0), (omni(Vec3::new(-1.0, 6.0, -30.0), 40.0, 1.0, 0.5), 1.0)] {
            let gpu = &scattering_lights(&[light], &[1.0])[0];
            for dir in [Vec3::NEG_Z, Vec3::new(0.3, 0.2, -1.0), Vec3::new(-0.4, 0.05, -1.0), Vec3::X, Vec3::new(0.0, 0.1, 1.0)] {
                let v = dir.normalize();
                for tmax in [5.0, 13.0, 60.0, SKY_DISTANCE] {
                    for (sigma, g) in [(0.0, 0.0), (0.01, 0.4), (0.05, -0.3)] {
                        close(light_scatter(o, v, tmax, gpu, sigma, g), reference(gpu, o, v, tmax, sigma, g), 0.01,
                            &format!("falloff {falloff} dir {v} tmax {tmax} sigma {sigma} g {g}"));
                    }
                }
            }
        }
    }

    #[test]
    fn rays_through_and_from_inside_the_source_stay_finite() {
        let light = omni(Vec3::new(0.0, 0.0, -10.0), 20.0, 2.0, 0.15);
        let gpu = &scattering_lights(&[light], &[1.0])[0];
        // Straight through the lamp: h = 0, only the source radius bounds it.
        let through = light_scatter(Vec3::ZERO, Vec3::NEG_Z, SKY_DISTANCE, gpu, 0.01, 0.3);
        assert!(through.is_finite() && through.x > 0.0);
        close(through, reference(gpu, Vec3::ZERO, Vec3::NEG_Z, SKY_DISTANCE, 0.01, 0.3), 0.03, "through the source");
        // Camera inside the range, at the lamp.
        let at = light_scatter(Vec3::new(0.0, 0.0, -10.0), Vec3::X, SKY_DISTANCE, gpu, 0.01, 0.0);
        assert!(at.is_finite() && at.x > 0.0);
        close(at, reference(gpu, Vec3::new(0.0, 0.0, -10.0), Vec3::X, SKY_DISTANCE, 0.01, 0.0), 0.03, "from the source");
        // Pointing away from a lamp behind the camera, outside its range.
        assert_eq!(light_scatter(Vec3::new(0.0, 0.0, 15.0), Vec3::Z, SKY_DISTANCE, gpu, 0.01, 0.0), Vec3::ZERO);
    }

    #[test]
    fn spot_cone_clip_matches_quadrature_and_never_drops_lit_air() {
        // A floodlight high on a tower aimed down at the centre (Growing Guns).
        let lamp = Vec3::new(30.0, 22.0, 30.0);
        for falloff in [2.0, 1.0] {
            let light = spot(lamp, -lamp, falloff);
            let gpu = &scattering_lights(&[light], &[1.0])[0];
            assert_eq!(gpu.source[2], 1.0, "narrow spots clip the ray to the cone");
            let o = Vec3::new(-5.0, 2.0, 25.0);
            for k in 0..24 {
                let a = k as f32 / 24.0 * std::f32::consts::TAU;
                let v = Vec3::new(a.cos(), 0.15 + 0.3 * (a * 3.0).sin(), a.sin()).normalize();
                for tmax in [20.0, SKY_DISTANCE] {
                    close(light_scatter(o, v, tmax, gpu, 0.004, 0.35), reference(gpu, o, v, tmax, 0.004, 0.35), 0.01,
                        &format!("spot falloff {falloff} ray {k} tmax {tmax}"));
                    // The clip keeps every point where the cone is lit.
                    let (lo, hi) = clip_cone(o, v, gpu, 0.0, 200.0);
                    for i in 0..2000 {
                        let t = i as f32 / 2000.0 * 200.0;
                        let off = o + v * t - lamp;
                        let lit = (off.normalize().dot(light.direction)) > light.outer_cos + 1e-3;
                        if lit { assert!(t >= lo - 1e-2 && t <= hi + 1e-2, "lit t={t} outside clip [{lo},{hi}] ray {k}"); }
                    }
                }
            }
        }
    }

    #[test]
    fn hg_phase_is_normalised() {
        for g in [0.0, 0.35, 0.8, -0.5] {
            let n = 20_000;
            let integral: f32 = (0..n).map(|i| hg_phase(-1.0 + (i as f32 + 0.5) * 2.0 / n as f32, g) * 2.0 * std::f32::consts::PI * 2.0 / n as f32).sum();
            assert!((integral - 1.0).abs() < 1e-3, "g={g}: {integral}");
        }
    }

    #[test]
    fn weights_select_and_scale_lights_and_default_is_off() {
        let a = omni(Vec3::ZERO, 10.0, 2.0, 0.1);
        let b = spot(Vec3::Y * 10.0, Vec3::NEG_Y, 2.0);
        assert!(scattering_lights(&[a, b], &[]).is_empty());
        let picked = scattering_lights(&[a, b, a], &[0.0, 2.0]);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].intensity_inner[..3], [100_000.0; 3]);
        assert_eq!(scattering_lights(&[a], &[1.0])[0].source[2], 0.0, "omni lights are not cone-clipped");
        assert!(crate::frame::Lighting::default().volumetric.is_none());
        let mut v = Volumetric::default();
        v.set("vol_density", 0.01).unwrap();
        assert_eq!(v.density, 0.01);
        assert!(v.set("vol_anisotropy", 0.99).is_err());
        assert_eq!(v.anisotropy, Volumetric::default().anisotropy, "a rejected value leaves the medium unchanged");
        assert!(!Volumetric { density: 0.0, ..v }.active());
        // Equal medium, zero weights: nothing scatters.
        assert_eq!(scatter_ray(&v, &scattering_lights(&[a], &[0.0]), Vec3::ZERO, Vec3::X, SKY_DISTANCE), Vec3::ZERO);
    }

    fn half_to_f32(bits: u16) -> f32 {
        let (sign, exp, frac) = ((bits >> 15) as u32, ((bits >> 10) & 0x1f) as i32, (bits & 0x3ff) as f32);
        let v = if exp == 0 { frac / 1024.0 * 2f32.powi(-14) } else if exp == 31 { f32::INFINITY } else { (1.0 + frac / 1024.0) * 2f32.powi(exp - 15) };
        if sign == 1 { -v } else { v }
    }

    fn read_rgba16(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<[f32; 4]> {
        let (w, h) = (texture.width(), texture.height());
        let row = (w * 8).div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } }, texture.size());
        gpu.queue.submit([enc.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.wait();
        let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| {
            let at = (y * row + x * 8) as usize;
            std::array::from_fn(|c| half_to_f32(u16::from_le_bytes([bytes[at + 2 * c], bytes[at + 2 * c + 1]])))
        }).collect()
    }

    /// The real passes against the CPU mirror (any adapter;
    /// `KERN_ADAPTER=software` runs it on a CPU rasteriser):
    /// cargo test -p render --lib volumetric::tests::gpu_passes_match_the_cpu_integral -- --ignored
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_passes_match_the_cpu_integral() {
        let gpu = Gpu::new().unwrap();
        let slot = VolumetricSlot::default();
        let lights = crate::headlights::validate_local(&[spot(Vec3::new(6.0, 10.0, -20.0), Vec3::new(-0.3, -1.0, 0.2), 1.0),
            omni(Vec3::new(-3.0, 2.0, -12.0), 15.0, 2.0, 0.2), spot(Vec3::new(-8.0, 6.0, -30.0), Vec3::new(1.0, -0.4, 0.3), 2.0)]).unwrap();
        slot.set_lights(&lights, &[1.0, 1.0, 0.5]);
        let scattering = scattering_lights(&lights, &[1.0, 1.0, 0.5]);
        let medium = Volumetric { density: 0.01, anisotropy: 0.4, color: Vec3::new(1.0, 0.9, 0.8), intensity: 2.0, ..Default::default() };
        let (w, h) = (64u32, 48u32);
        let camera = Vec3::new(0.0, 1.5, 0.0);
        let vp = Mat4::perspective_infinite_reverse_rh(1.0, w as f32 / h as f32, 0.1) * Mat4::look_at_rh(camera, Vec3::new(0.0, 2.5, -10.0), Vec3::Y);
        let inverse = vp.inverse();
        for clear in [0.0f32, 0.004] {
            let depth = gpu.device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
            let depth_view = depth.create_view(&Default::default());
            let input = gpu.device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
            let input_view = input.create_view(&Default::default());
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &input_view, depth_slice: None, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.25, g: 0.5, b: 1.0, a: 1.0 }), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None }));
            let output = slot.render(&gpu, &mut enc, &medium, ViewInput { index: 0, input: &input, depth: &depth_view, depth_size: (w, h), camera, near: 0.1, depth_vp: vp, sun: (Vec3::Y, Vec3::ZERO), light_shapes: &crate::light_shapes::LightShapes::new(&gpu).view }).unwrap().unwrap();
            gpu.queue.submit([enc.finish()]);
            let half = read_rgba16(&gpu, &slot.0.lock().unwrap().passes.as_ref().unwrap().targets[0].as_ref().unwrap().scatter);
            let full = read_rgba16(&gpu, &output);
            let mut peak = 0f32;
            for j in 0..h / 2 { for i in 0..w / 2 {
                // Equal depths: the footprint's first texel.
                let ndc = |x: u32, y: u32| glam::Vec2::new((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / h as f32 * 2.0);
                let n = ndc(2 * i, 2 * j);
                let v = (inverse.project_point3(n.extend(1.0)) - camera).normalize();
                let tmax = if clear > 0.0 { inverse.project_point3(n.extend(clear)).distance(camera) } else { SKY_DISTANCE };
                let cpu = scatter_ray(&medium, &scattering, camera, v, tmax);
                let gpu_value = Vec3::from_slice(&half[(j * (w / 2) + i) as usize][..3]);
                peak = peak.max(cpu.max_element());
                assert!((gpu_value - cpu).abs().max_element() <= 0.01 * cpu.max_element() + 2e-4, "texel {i},{j} depth {clear}: gpu {gpu_value} cpu {cpu}");
            }}
            assert!(peak > 0.01, "the test rays see the lights: {peak}");
            // Composite: input plus the upsampled scatter, everywhere.
            for (k, px) in full.iter().enumerate() {
                let (x, y) = (k as u32 % w, k as u32 / w);
                // Equal depths: plain bilinear weights.
                let p = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * 0.5 - 0.5;
                let (base, f) = (p.floor(), p - p.floor());
                let tap = |dx: i32, dy: i32| {
                    let q = (base.as_ivec2() + glam::IVec2::new(dx, dy)).clamp(glam::IVec2::ZERO, glam::IVec2::new(w as i32 / 2 - 1, h as i32 / 2 - 1));
                    Vec3::from_slice(&half[(q.y as u32 * (w / 2) + q.x as u32) as usize][..3])
                };
                let s = tap(0, 0).lerp(tap(1, 0), f.x).lerp(tap(0, 1).lerp(tap(1, 1), f.x), f.y);
                let expected = Vec3::new(0.25, 0.5, 1.0) + s;
                assert!((Vec3::from_slice(&px[..3]) - expected).abs().max_element() <= 0.004 * expected.max_element() + 1e-3,
                    "composite at {x},{y}: {px:?} vs input + bilinear {expected}");
            }
        }
    }

    /// Wall time of both passes (submit to idle), sky depth (every ray at
    /// full length), for a GG-like night set. Prints; asserts nothing:
    /// cargo test --release -p render --lib volumetric::tests::gpu_cost -- --ignored --nocapture
    #[test]
    #[ignore = "needs a GPU adapter; a measurement"]
    fn gpu_cost() {
        let gpu = Gpu::new().unwrap();
        println!("adapter {:?} {:?}", gpu.info.name, gpu.info.device_type);
        let mut set = Vec::new();
        for i in 0..4 { let a = std::f32::consts::TAU * (i as f32 + 0.5) / 4.0; let p = Vec3::new(a.cos() * 50.0, 22.0, a.sin() * 50.0);
            set.push(LocalLight { range_m: 80.0, ..spot(p, -p, 1.0) });
            for k in 0..3 { let q = glam::Quat::from_rotation_y((k as f32 - 1.0) * 0.34);
                set.push(LocalLight { inner_cos: 2.5f32.to_radians().cos(), outer_cos: 6.5f32.to_radians().cos(), range_m: 80.0, ..spot(p, q * -p, 1.0) }); } }
        for i in 0..14 { let a = std::f32::consts::TAU * i as f32 / 14.0; set.push(omni(Vec3::new(a.cos() * 40.0, 1.0, a.sin() * 40.0), 40.0, 1.0, 1.0)); }
        let medium = Volumetric::default();
        for (w, h) in [(1280u32, 720u32), (1920, 1080)] { for count in [1usize, 8, set.len()] {
            let slot = VolumetricSlot::default();
            let lights = crate::headlights::validate_local(&set[..count]).unwrap();
            slot.set_lights(&lights, &vec![1.0; count]);
            let texture = |format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let depth = texture(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            let depth_view = depth.create_view(&Default::default());
            let input = texture(wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::TEXTURE_BINDING);
            let camera = Vec3::new(0.0, 8.0, 30.0);
            let vp = Mat4::perspective_infinite_reverse_rh(1.2, w as f32 / h as f32, 0.1) * Mat4::look_at_rh(camera, Vec3::new(0.0, 10.0, 0.0), Vec3::Y);
            let run = || { let mut enc = gpu.device.create_command_encoder(&Default::default());
                slot.render(&gpu, &mut enc, &medium, ViewInput { index: 0, input: &input, depth: &depth_view, depth_size: (w, h), camera, near: 0.1, depth_vp: vp, sun: (Vec3::Y, Vec3::ZERO), light_shapes: &crate::light_shapes::LightShapes::new(&gpu).view }).unwrap();
                gpu.queue.submit([enc.finish()]); gpu.wait(); };
            run();
            let n = 3;
            let t = web_time::Instant::now();
            for _ in 0..n { run(); }
            println!("{w}x{h} internal, {count} lights: {:.2} ms per view (mean of {n})", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }}
    }

    #[test]
    fn ambient_and_extinction_follow_the_medium_and_default_off() {
        let off = Volumetric::default();
        assert_eq!(transmittance(&off, 50.0), 1.0, "the default only adds light");
        assert_eq!(scatter_ray(&off, &[], Vec3::ZERO, Vec3::NEG_Z, 50.0), Vec3::ZERO);
        let fog = Volumetric { density: 0.05, extinction: 1.0, ambient: Vec3::new(2.0, 1.0, 0.5), length: 20.0, ..off };
        assert!(fog.active());
        // Bounded by the medium length: the same past 20 m.
        assert_eq!(transmittance(&fog, 100.0), transmittance(&fog, 20.0));
        assert!((transmittance(&fog, 10.0) - (-0.5f32).exp()).abs() < 1e-6);
        let a = scatter_ray(&fog, &[], Vec3::ZERO, Vec3::NEG_Z, 10.0);
        assert!((a - fog.ambient * (1.0 - (-0.5f32).exp())).abs().max_element() < 1e-5);
        // Surface radiance s: s * T + ambient * (1 - T) tends to the ambient.
        assert!(Volumetric { extinction: 1.5, ..fog }.validated().is_err());
    }

    #[test]
    fn wgsl_validates_and_layout_matches() {
        let module = naga::front::wgsl::parse_str(shader_source()).unwrap_or_else(|e| panic!("{}", e.emit_to_string(shader_source())));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(shader_source())));
        assert_eq!(std::mem::size_of::<GpuLight>(), 64);
        // Matrix, six vec4s, 32 lights, march + sun_dir + sun_e, 2 * MAX_BOUNDS vec4s.
        assert_eq!(std::mem::size_of::<Uniform>(), 64 + 6 * 16 + 32 * 64 + 3 * 16 + 2 * MAX_BOUNDS * 16);
        for (name, value) in [("SAMPLES", format!("{SAMPLES}u")), ("SKY_DISTANCE", format!("{SKY_DISTANCE:.1}"))] {
            assert!(shader_source().contains(&format!("const {name}: {} = {value};", if name == "SAMPLES" { "u32" } else { "f32" })), "{name} must match the CPU mirror");
        }
    }
}
