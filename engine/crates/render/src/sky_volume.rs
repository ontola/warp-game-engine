//! Opt-in aerial perspective and volumetric clouds (engine/docs/atmosphere.md).
//!
//! `Lighting::sky_volume` turns it on. It runs after temporal
//! reconstruction (like volumetric light) and writes a separate output, so
//! nothing enters TAA or DLSS history:
//!
//! - Aerial perspective: per view a 32^3 camera volume (froxels) of the
//!   light the air scatters towards the camera and how much it lets through,
//!   integrated with the same medium and LUTs as the CPU sky bake
//!   (`atmosphere::AtmosphereTables`), multiple scattering included. It
//!   replaces the shade pass's distance fog, which is turned off while it is
//!   on (the shade fog stays the default without it).
//! - Clouds: one cumulus layer, raymarched at half the depth resolution up to
//!   the scene depth, sun light marched through the cloud with an octave
//!   approximation of multiple scattering, temporally accumulated in its own
//!   half-resolution history. A 2D weather field (`cloud_weather.wgsl`) says
//!   where clouds are; the shade pass reads the same field for cloud shadows
//!   on the ground, and the air in their shadow scatters less sun.
//!
//! Limits: the background sky texture is still the CPU bake seen from the
//! ground (no sky from altitude, no clouds in reflections or the SH9
//! skylight), terrain does not shadow the air, legacy grass and foliage cards in their separate passes
//! do not receive cloud shadows. The GPU meadow shares the ground's cloud transmittance.
use crate::Gpu;
use crate::atmosphere::{AtmosphereTables, MULTI_SCATTER_SIZE};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use glam::{DMat4, Mat4, Vec2, Vec3};
use std::sync::{Arc, Mutex};

pub fn shader_source() -> String {
    format!("{}\n{}", include_str!("shaders/cloud_weather.wgsl"), include_str!("shaders/sky_volume.wgsl"))
}

/// Froxel slices and their screen resolution (keep equal to `SLICES`).
pub const SLICES: u32 = 32;
/// Shape and detail noise sizes (texels per side).
const SHAPE_SIZE: u32 = 64;
const DETAIL_SIZE: u32 = 32;

/// One cumulus layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clouds {
    /// Share of the sky covered, 0..1 (0.45 fair-weather cumulus, 0.85 broken).
    pub coverage: f32,
    /// Cloud base above the datum (world y = 0), metres.
    pub base_m: f32,
    /// Layer thickness, metres.
    pub thickness_m: f32,
    /// Extinction inside a full-density cloud, per metre (0.02..0.1).
    pub density: f32,
    /// Wind, metres per second along world x and z. Offset = wind * Lighting::time.
    pub wind: Vec2,
    /// How much the clouds shade the sun on the ground and in the air, 0..1.
    pub shadow: f32,
    /// Raymarch samples per view ray (16..128) and towards the sun (2..8).
    pub steps: u32,
    pub light_steps: u32,
    /// Rays end this far from the camera (metres).
    pub far_m: f32,
}

impl Default for Clouds {
    fn default() -> Self {
        Self { coverage: 0.45, base_m: 1800.0, thickness_m: 1800.0, density: 0.03, wind: Vec2::new(4.0, 1.5),
            shadow: 0.85, steps: 64, light_steps: 6, far_m: 60_000.0 }
    }
}

/// `Lighting::sky_volume`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyVolume {
    /// Physically integrated aerial perspective on surfaces (replaces the
    /// shade pass's distance fog).
    pub aerial: bool,
    /// How far the aerial-perspective volume reaches (metres); beyond it the
    /// last slice holds.
    pub aerial_far_m: f32,
    pub clouds: Option<Clouds>,
}

impl Default for SkyVolume {
    fn default() -> Self { Self { aerial: true, aerial_far_m: 80_000.0, clouds: Some(Clouds::default()) } }
}

impl SkyVolume {
    pub fn validated(self) -> Result<Self, String> {
        if !(self.aerial_far_m.is_finite() && (1000.0..=400_000.0).contains(&self.aerial_far_m)) {
            return Err("sky volume needs aerial_far_m 1000..400000".into());
        }
        if let Some(c) = self.clouds {
            if !(c.coverage.is_finite() && (0.0..=1.0).contains(&c.coverage))
                || !(c.base_m.is_finite() && (0.0..=20_000.0).contains(&c.base_m))
                || !(c.thickness_m.is_finite() && (50.0..=10_000.0).contains(&c.thickness_m))
                || !(c.density.is_finite() && (0.0..=1.0).contains(&c.density))
                || !c.wind.is_finite() || c.wind.length() > 200.0
                || !(c.shadow.is_finite() && (0.0..=1.0).contains(&c.shadow))
                || !(16..=128).contains(&c.steps) || !(2..=8).contains(&c.light_steps)
                || !(c.far_m.is_finite() && (1000.0..=400_000.0).contains(&c.far_m)) {
                return Err("clouds need coverage 0..1, base 0..20000 m, thickness 50..10000 m, density 0..1 per m, wind under 200 m/s, shadow 0..1, steps 16..128, light steps 2..8 and far 1000..400000 m".into());
            }
        }
        Ok(self)
    }

    /// `Lighting::set` keys: `sky_aerial`, `sky_aerial_far`, `clouds` (0 off,
    /// 1 on with defaults), `cloud_coverage`, `cloud_base`, `cloud_thickness`,
    /// `cloud_density`, `cloud_wind_x`, `cloud_wind_z`, `cloud_shadow`,
    /// `cloud_steps`, `cloud_light_steps`, `cloud_far`.
    pub fn set(&mut self, key: &str, value: f32) -> Result<(), String> {
        let mut next = *self;
        let flag = |v: f32| if v == 0.0 || v == 1.0 { Ok(v == 1.0) } else { Err(format!("{key} is 0 or 1")) };
        let count = |v: f32| if v.fract() == 0.0 && v >= 0.0 { Ok(v as u32) } else { Err(format!("{key} is a whole number")) };
        match key {
            "sky_aerial" => next.aerial = flag(value)?,
            "sky_aerial_far" => next.aerial_far_m = value,
            "clouds" => next.clouds = flag(value)?.then(|| self.clouds.unwrap_or_default()),
            k if k.starts_with("cloud_") => {
                let c = next.clouds.get_or_insert_with(Default::default);
                match k {
                    "cloud_coverage" => c.coverage = value,
                    "cloud_base" => c.base_m = value,
                    "cloud_thickness" => c.thickness_m = value,
                    "cloud_density" => c.density = value,
                    "cloud_wind_x" => c.wind.x = value,
                    "cloud_wind_z" => c.wind.y = value,
                    "cloud_shadow" => c.shadow = value,
                    "cloud_steps" => c.steps = count(value)?,
                    "cloud_light_steps" => c.light_steps = count(value)?,
                    "cloud_far" => c.far_m = value,
                    _ => return Err(format!("unknown cloud setting {k}")),
                }
            }
            _ => return Err(format!("unknown sky volume setting {key}")),
        }
        *self = next.validated()?;
        Ok(())
    }

    /// The shade pass's cloud uniforms (`Light::clouds`, `Light::cloud_motion`
    /// in shade.wgsl, read by `cloud_sun_transmittance`).
    pub fn shade_uniforms(&self, time: f32) -> ([f32; 4], [f32; 4]) {
        match self.clouds {
            Some(c) => {
                let offset = c.wind * time;
                ([c.coverage, c.base_m, c.thickness_m, c.density], [offset.x, offset.y, c.shadow, 1.0])
            }
            None => ([0.0; 4], [0.0; 4]),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniform {
    inv_vp: [[f32; 4]; 4],
    prev_vp: [[f32; 4]; 4],
    inv_depth_vp: [[f32; 4]; 4],
    camera: [f32; 4],
    sizes: [f32; 4],
    output: [f32; 4],
    sun: [f32; 4],
    solar: [f32; 4],
    above: [f32; 4],
    below: [f32; 4],
    rayleigh: [f32; 4],
    mie: [f32; 4],
    ozone: [f32; 4],
    extent: [f32; 4],
    cloud: [f32; 4],
    cloud_motion: [f32; 4],
    cloud_shape: [f32; 4],
}

struct Target {
    size: (u32, u32),
    half: (u32, u32),
    froxels: wgpu::Texture,
    cloud: wgpu::Texture,
    cloud_depth: wgpu::Texture,
    history: [wgpu::Texture; 2],
    output: wgpu::Texture,
    uniform: wgpu::Buffer,
    prev_vp: Option<Mat4>,
    frame: u32,
}

struct Tables {
    source: Arc<AtmosphereTables>,
    transmittance: wgpu::TextureView,
    multi: wgpu::TextureView,
}

struct Passes {
    noise: wgpu::ComputePipeline,
    froxels: wgpu::ComputePipeline,
    clouds: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    composite: wgpu::ComputePipeline,
    clamp: wgpu::Sampler,
    repeat: wgpu::Sampler,
    /// Shape and detail noise, generated on first use.
    noise_textures: Option<(wgpu::TextureView, wgpu::TextureView)>,
    tables: Option<Tables>,
    targets: Vec<Option<Target>>,
}

impl Passes {
    fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky volume"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let pipe = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: None, module: &module,
            entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let sampler = |label, mode| gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some(label),
            address_mode_u: mode, address_mode_v: mode, address_mode_w: mode,
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Self { noise: pipe("noise"), froxels: pipe("froxels"), clouds: pipe("clouds"), resolve: pipe("cloud_resolve"), composite: pipe("composite"),
            clamp: sampler("sky volume clamp", wgpu::AddressMode::ClampToEdge), repeat: sampler("sky volume repeat", wgpu::AddressMode::Repeat),
            noise_textures: None, tables: None, targets: Vec::new() }
    }

    fn noise(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder) {
        if self.noise_textures.is_some() { return; }
        let mut make = |label, size: u32, detail: bool| {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: size }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
            let mut uniform = Uniform::zeroed();
            uniform.output = [size as f32, if detail { 1.0 } else { 0.0 }, 0.0, 0.0];
            let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cloud noise"),
                contents: bytemuck::bytes_of(&uniform), usage: wgpu::BufferUsages::UNIFORM });
            let view = texture.create_view(&Default::default());
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &self.noise.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 18, resource: wgpu::BindingResource::TextureView(&view) }] });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(label), timestamp_writes: None });
            pass.set_pipeline(&self.noise);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(size.div_ceil(4), size.div_ceil(4), size.div_ceil(4));
            view
        };
        let shape = make("cloud shape noise", SHAPE_SIZE, false);
        let detail = make("cloud detail noise", DETAIL_SIZE, true);
        self.noise_textures = Some((shape, detail));
    }

    fn tables(&mut self, gpu: &Gpu, source: &Arc<AtmosphereTables>) {
        if self.tables.as_ref().is_some_and(|t| Arc::ptr_eq(&t.source, source)) { return; }
        let upload = |label, (w, h): (u32, u32), pixels: &[[f32; 4]]| {
            let half: Vec<u16> = pixels.iter().flat_map(|p| p.map(crate::noise_field::f16_bits)).collect();
            gpu.device.create_texture_with_data(&gpu.queue, &wgpu::TextureDescriptor { label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }, Default::default(), bytemuck::cast_slice(&half))
                .create_view(&Default::default())
        };
        self.tables = Some(Tables { source: source.clone(),
            transmittance: upload("atmosphere transmittance LUT", source.transmittance_size, &source.transmittance),
            multi: upload("atmosphere multiple scattering LUT", (MULTI_SCATTER_SIZE, MULTI_SCATTER_SIZE), &source.multi_scatter) });
    }
}

#[derive(Default)]
struct State { passes: Option<Passes>, tables: Option<Arc<AtmosphereTables>>, ambient: Option<[Vec3; 2]> }

/// Per-renderer state: the atmosphere tables and lazily created passes.
#[derive(Default)]
pub(crate) struct SkyVolumeSlot(Mutex<State>);

/// What a view needs to draw aerial perspective and clouds.
pub(crate) struct ViewInput<'a> {
    pub index: usize,
    pub input: &'a wgpu::Texture,
    pub depth: &'a wgpu::TextureView,
    pub depth_size: (u32, u32),
    pub camera: Vec3,
    pub depth_vp: Mat4,
    /// The same view without the TAA/DLSS sub-pixel jitter: rays and
    /// reprojection use it, so the clouds do not hop with the jitter.
    pub view_proj: Mat4,
    pub light: &'a crate::Lighting,
    /// Restart temporal accumulation (camera cut, resize).
    pub reset: bool,
}

/// The default atmosphere's tables (baked once), for scenes without a
/// procedural sky and for hand-built `BakedSky` values.
pub fn default_tables() -> Arc<AtmosphereTables> {
    static TABLES: std::sync::OnceLock<Arc<AtmosphereTables>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let quality = crate::atmosphere::BakeQuality { sky_width: 64, sky_height: 32, sky_steps: 16, ..Default::default() };
        let solar = crate::atmosphere::SolarState::from_degrees(45.0, 0.0).unwrap();
        crate::atmosphere::AtmosphereCache::default().get_or_bake(&Default::default(), solar, quality).unwrap().tables.clone()
    }).clone()
}

impl SkyVolumeSlot {
    /// The atmosphere the passes integrate (from `BakedSky::apply` or
    /// `Renderer::update_sky`).
    pub fn set_tables(&self, tables: Option<Arc<AtmosphereTables>>) {
        let mut state = self.0.lock().unwrap();
        state.tables = tables;
        state.ambient = None;
        if let Some(passes) = state.passes.as_mut() { passes.targets.iter_mut().flatten().for_each(|t| t.prev_vp = None); }
    }

    /// Vary the light on the clouds without replacing weather LUTs or resetting history.
    pub fn set_ambient(&self, upper: Vec3, ground: Vec3) {
        self.0.lock().unwrap().ambient = Some([upper, ground]);
    }

    /// Encode the passes for one view; returns the composited HDR.
    pub fn render(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, settings: &SkyVolume, view: ViewInput) -> Result<wgpu::Texture, String> {
        let settings = settings.validated()?;
        let mut state = self.0.lock().unwrap();
        let State { passes, tables, ambient } = &mut *state;
        let tables = tables.clone().unwrap_or_else(default_tables);
        let [upper, ground] = ambient.unwrap_or([tables.sky_radiance_above, tables.ground_radiance]);
        let passes = passes.get_or_insert_with(|| Passes::new(gpu));
        passes.noise(gpu, enc);
        passes.tables(gpu, &tables);
        let input = view.input;
        let size = (input.width(), input.height());
        let (dw, dh) = view.depth_size;
        let half = (dw.div_ceil(2).max(1), dh.div_ceil(2).max(1));
        if passes.targets.len() <= view.index { passes.targets.resize_with(view.index + 1, || None); }
        if passes.targets[view.index].as_ref().is_none_or(|t| t.size != size || t.half != half) {
            let texture = |label, (w, h): (u32, u32), depth: u32, usage| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: depth }, mip_level_count: 1, sample_count: 1,
                dimension: if depth > 1 { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 },
                format: wgpu::TextureFormat::Rgba16Float, usage, view_formats: &[] });
            let work = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            passes.targets[view.index] = Some(Target { size, half,
                froxels: texture("aerial perspective froxels", (SLICES, SLICES), SLICES, work),
                cloud: texture("clouds (half resolution)", half, 1, work),
                cloud_depth: texture("cloud front distance (half resolution)", half, 1, work),
                history: [texture("cloud history A", half, 1, work), texture("cloud history B", half, 1, work)],
                output: texture("sky volume HDR output (not temporal history)", size, 1, work | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT),
                uniform: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky volume view"), size: std::mem::size_of::<Uniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                prev_vp: None, frame: 0 });
        }
        let t = passes.targets[view.index].as_mut().unwrap();
        let light = view.light;
        let model = &tables.model;
        let history_valid = !view.reset && t.prev_vp.is_some();
        // WARP_SKY_VOLUME_LOG=1: one line per view and frame, for checking the
        // cloud history on a machine we cannot watch (flashing clouds report).
        if std::env::var_os("WARP_SKY_VOLUME_LOG").is_some() {
            eprintln!("sky volume: view {} frame {} output {:?} depth {:?} half {:?} history_valid {} reset {} camera {:?}",
                view.index, t.frame, size, view.depth_size, half, history_valid, view.reset, view.camera);
        }
        let mut u = Uniform::zeroed();
        // Camera-relative matrices, built in f64: far from the origin a
        // world-space inverse loses the ray direction to f32 rounding (the
        // near-plane point minus a camera 100 km out), which tore the clouds
        // into blocks.
        let at = DMat4::from_translation(view.camera.as_dvec3());
        let relative = |m: Mat4| m.as_dmat4() * at;
        u.inv_vp = relative(view.view_proj).inverse().as_mat4().to_cols_array_2d();
        u.prev_vp = relative(t.prev_vp.unwrap_or(view.view_proj)).as_mat4().to_cols_array_2d();
        u.inv_depth_vp = relative(view.depth_vp).inverse().as_mat4().to_cols_array_2d();
        u.camera = view.camera.extend(t.frame as f32).to_array();
        u.sizes = [dw as f32, dh as f32, half.0 as f32, half.1 as f32];
        u.output = [size.0 as f32, size.1 as f32, if history_valid { 1.0 } else { 0.0 }, if settings.aerial { 1.0 } else { 0.0 }];
        u.sun = light.sun_dir.normalize_or(Vec3::Y).extend(0.0).to_array();
        u.solar = model.solar_rgb_lux.as_vec3().extend(0.0).to_array();
        u.above = upper.extend(0.0).to_array();
        u.below = ground.extend(0.0).to_array();
        u.rayleigh = model.rayleigh_per_km.as_vec3().extend(model.rayleigh_height_km as f32).to_array();
        u.mie = [model.mie_scattering_per_km as f32, model.mie_absorption_per_km as f32, model.mie_height_km as f32, model.mie_g as f32];
        u.ozone = model.ozone_per_km.as_vec3().extend(model.planet_radius_km as f32).to_array();
        let clouds = settings.clouds.unwrap_or_default();
        u.extent = [(model.planet_radius_km + model.height_km) as f32, settings.aerial_far_m * 0.001, clouds.far_m * 0.001, 0.0];
        (u.cloud, u.cloud_motion) = settings.shade_uniforms(light.time);
        u.cloud_shape = [3500.0, 900.0, clouds.steps as f32, clouds.light_steps as f32];
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&u));
        t.prev_vp = Some(view.view_proj);
        t.frame = t.frame.wrapping_add(1);
        let (cur, prev) = ((t.frame & 1) as usize, 1 - (t.frame & 1) as usize);

        let view_of = |tex: &wgpu::Texture| tex.create_view(&Default::default());
        let (froxels, cloud, cloud_depth, output, input_v) = (view_of(&t.froxels), view_of(&t.cloud), view_of(&t.cloud_depth), view_of(&t.output), view_of(input));
        let (history_cur, history_prev) = (view_of(&t.history[cur]), view_of(&t.history[prev]));
        let tv = passes.tables.as_ref().unwrap();
        let (shape, detail) = passes.noise_textures.as_ref().unwrap();
        use wgpu::BindingResource as R;
        let bind = |pipeline: &wgpu::ComputePipeline, label, entries: &[(u32, R)]| {
            let entries: Vec<_> = entries.iter().map(|(binding, resource)| wgpu::BindGroupEntry { binding: *binding, resource: resource.clone() }).collect();
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &pipeline.get_bind_group_layout(0), entries: &entries })
        };
        let uniform = t.uniform.as_entire_binding();
        let froxel_bind = bind(&passes.froxels, "sky volume froxels", &[(0, uniform.clone()), (2, R::TextureView(&tv.transmittance)),
            (3, R::TextureView(&tv.multi)), (4, R::Sampler(&passes.clamp)), (8, R::TextureView(&froxels))]);
        let composite_bind = bind(&passes.composite, "sky volume composite", &[(0, uniform.clone()), (1, R::TextureView(view.depth)),
            (4, R::Sampler(&passes.clamp)), (9, R::TextureView(&froxels)), (13, R::TextureView(&cloud_depth)),
            (14, R::TextureView(&history_cur)), (16, R::TextureView(&input_v)), (17, R::TextureView(&output))]);
        let cloud_binds = settings.clouds.is_some().then(|| (
            bind(&passes.clouds, "sky volume clouds", &[(0, uniform.clone()), (1, R::TextureView(view.depth)), (2, R::TextureView(&tv.transmittance)),
                (4, R::Sampler(&passes.clamp)), (5, R::TextureView(shape)), (6, R::TextureView(detail)), (7, R::Sampler(&passes.repeat)),
                (10, R::TextureView(&cloud)), (11, R::TextureView(&cloud_depth))]),
            bind(&passes.resolve, "sky volume cloud resolve", &[(0, uniform.clone()), (4, R::Sampler(&passes.clamp)),
                (12, R::TextureView(&cloud)), (13, R::TextureView(&cloud_depth)), (14, R::TextureView(&history_prev)), (15, R::TextureView(&history_cur))])));
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky volume"), timestamp_writes: None });
            pass.set_pipeline(&passes.froxels);
            pass.set_bind_group(0, &froxel_bind, &[]);
            pass.dispatch_workgroups(SLICES.div_ceil(8), SLICES.div_ceil(8), 1);
            if let Some((clouds_bind, resolve_bind)) = &cloud_binds {
                pass.set_pipeline(&passes.clouds);
                pass.set_bind_group(0, clouds_bind, &[]);
                pass.dispatch_workgroups(half.0.div_ceil(8), half.1.div_ceil(8), 1);
                pass.set_pipeline(&passes.resolve);
                pass.set_bind_group(0, resolve_bind, &[]);
                pass.dispatch_workgroups(half.0.div_ceil(8), half.1.div_ceil(8), 1);
            }
            pass.set_pipeline(&passes.composite);
            pass.set_bind_group(0, &composite_bind, &[]);
            pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        }
        Ok(t.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(source: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module).unwrap_or_else(|e| panic!("{e:?}"));
        module
    }

    #[test]
    fn shader_validates_and_stays_within_portable_limits() {
        let source = shader_source();
        let m = module(&source);
        for entry in ["noise", "froxels", "clouds", "cloud_resolve", "composite"] {
            assert!(m.entry_points.iter().any(|e| e.name == entry), "{entry}");
        }
        // No storage buffers at all; at most 4 storage textures per stage (WebGPU minimum).
        assert!(!source.contains("var<storage"));
        assert!(source.matches("texture_storage_").count() <= 8);
        assert_eq!(std::mem::size_of::<Uniform>() % 16, 0);
    }

    #[test]
    fn settings_parse_validate_and_default_off() {
        assert!(crate::Lighting::default().sky_volume.is_none());
        let mut light = crate::Lighting::default();
        light.set("clouds=1").unwrap();
        let s = light.sky_volume.unwrap();
        assert!(s.aerial && s.clouds.is_some());
        light.set("cloud_coverage=0.8").unwrap();
        assert_eq!(light.sky_volume.unwrap().clouds.unwrap().coverage, 0.8);
        assert!(light.set("cloud_coverage=1.5").is_err());
        assert_eq!(light.sky_volume.unwrap().clouds.unwrap().coverage, 0.8, "a rejected value changes nothing");
        light.set("clouds=0").unwrap();
        assert!(light.sky_volume.unwrap().clouds.is_none() && light.sky_volume.unwrap().aerial);
        light.set("sky_volume=0").unwrap();
        assert!(light.sky_volume.is_none());
        assert!(light.set("cloud_steps=12").is_err());
        let (cloud, motion) = SkyVolume::default().shade_uniforms(10.0);
        assert_eq!(cloud, [0.45, 1800.0, 1800.0, 0.03]);
        assert_eq!(motion, [40.0, 15.0, 0.85, 1.0]);
    }

    /// `cloud_sun_transmittance` on the CPU (cloud_weather.wgsl, line for line).
    fn hash2(x: i32, y: i32) -> f32 {
        let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841);
        h = (h ^ (h >> 16)).wrapping_mul(0x7feb352d);
        h = (h ^ (h >> 15)).wrapping_mul(0x846ca68b);
        h ^= h >> 16;
        (h & 0xffffff) as f32 / 16777215.0
    }
    fn value(p: Vec2) -> f32 {
        let i = p.floor();
        let f = p - i;
        let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
        let (x, y) = (i.x as i32, i.y as i32);
        let (a, b, d, e) = (hash2(x, y), hash2(x + 1, y), hash2(x, y + 1), hash2(x + 1, y + 1));
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        lerp(lerp(a, b, u.x), lerp(d, e, u.x), u.y)
    }
    fn weather(xz: Vec2, coverage: f32) -> f32 {
        if coverage <= 0.0 { return 0.0; }
        let p = xz / 4000.0;
        let n = 0.55 * value(p) + 0.3 * value(p * 2.03 + Vec2::new(17.1, 3.7)) + 0.15 * value(p * 4.11 + Vec2::new(5.3, 41.9));
        let threshold = 1.0 - coverage;
        let t = ((n - (threshold - 0.08)) / 0.16).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    #[test]
    fn coverage_sets_the_share_of_sky_with_cloud() {
        for coverage in [0.0, 0.2, 0.45, 0.8, 1.0] {
            let n = 200;
            let covered = (0..n * n).filter(|i| weather(Vec2::new((i % n) as f32 * 450.0, (i / n) as f32 * 450.0), coverage) > 0.5).count() as f32 / (n * n) as f32;
            assert!((covered - coverage).abs() < 0.2, "coverage {coverage}: {covered} of the sky is cloud");
        }
    }

    #[test]
    fn froxels_reach_their_far_distance_quadratically() {
        // slice_distance in sky_volume.wgsl.
        let far = 80.0;
        let slice = |i: u32| far * ((i + 1) as f32 / SLICES as f32).powi(2);
        assert_eq!(slice(SLICES - 1), far);
        assert!(slice(0) < 0.1, "the first slice holds the near air");
    }
}
