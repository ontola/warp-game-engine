//! The frame: per view, cull clusters on the GPU, raster the survivors
//! into a visibility buffer, shade every pixel once in HDR, tonemap. The
//! sun's shadow map is a view of its own through the same cluster path.

use crate::Gpu;
use crate::scene::Scene;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::f32::consts::PI;
use wgpu::util::DeviceExt;

#[path = "cutout_mips.rs"]
mod cutout_mips;

pub const MAX_VISIBLE: u32 = 4 << 20;
/// Albedo array resolution: the Poly Haven sets are 2K.
const TEXTURE_SIZE: u32 = 2048;
/// Normal map array resolution.
const NORMAL_SIZE: u32 = 1024;
const SHADOW_SIZE: u32 = 4096;
const NEAR_SHADOW_SIZE: u32 = 2048;
const NEAR_SHADOW_EXTENT: f32 = 16.0;
const TIMESTAMP_CAPACITY: u32 = 192; // includes four shared headlight shadow pairs

/// Keep the grass rejection in culling identical to its opaque raster path.
pub(crate) fn cull_source() -> String {
    format!("{}\n{}", include_str!("shaders/grass_common.wgsl"), include_str!("shaders/cull.wgsl"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Readback { None, Metrics, ImageAndMetrics }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TaaUniform {
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    size: [f32; 4],
    params: [f32; 4],
}

/// One view's TAA history: last frame's result and camera.
struct History {
    size: (u32, u32),
    tex: [wgpu::Texture; 2],
    cur: usize,
    prev_view_proj: Mat4,
}

/// The sun's shadow map, kept while neither the sun box nor the scene moves.
struct ShadowCache {
    view_proj: Mat4,
    tex: wgpu::Texture,
    clusters: u32,
    triangles: u32,
    submitted_vertices: u32,
}

/// Per-camera sun detail, retained across frames rather than reallocating.
struct NearShadow {
    tex: wgpu::Texture,
    targets: ViewTargets,
}

#[derive(Default)]
struct Upscaling {
    mode: crate::upscale::UpscaleMode,
    backend: Option<crate::dlss::DlssContext>,
    motion: Option<crate::motion::MotionRenderer>,
    status: crate::upscale::UpscaleStatus,
    layout: Vec<(u32, u32, u32, u32)>,
}

fn timestamp_pair(next: &mut u32) -> u32 {
    assert!(*next <= TIMESTAMP_CAPACITY - 2, "frame exceeds timestamp query capacity");
    let first = *next;
    *next += 2;
    first
}

/// Halton(2, 3) sample k: sub-pixel offsets in [-0.5, 0.5).
fn halton(k: u32) -> (f32, f32) {
    let h = |mut i: u32, b: u32| {
        let (mut f, mut r) = (1.0f32, 0.0f32);
        while i > 0 {
            f /= b as f32;
            r += f * (i % b) as f32;
            i /= b;
        }
        r
    };
    (h(k, 2) - 0.5, h(k, 3) - 0.5)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    planes: [[f32; 4]; 6],
    params: [f32; 4],
    size: [f32; 4],
    extra: [f32; 4],
    grass: [f32; 4],
    /// Camera colour only: full-fog distance, enabled, reserved, reserved.
    /// Shadow views are constructed with this entire field disabled.
    fog_cull: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightUniform {
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    shadow_view_proj: [[f32; 4]; 4],
    sky: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    fog_scattering: [f32; 4],
    sh: [[f32; 4]; 9],
    near_shadow_view_proj: [[f32; 4]; 4],
    /// x: world texel size, y: blend-start NDC radius, z: enabled.
    near_shadow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GiUniform {
    vol_origins: [[f32; 4]; 3],
    vol_anchors: [[f32; 4]; 3],
    vol_dims: [u32; 4],
    hf: [f32; 4],
    hf_dims: [u32; 4],
    bounce: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ToneUniform {
    exposure: f32,
    contrast: f32,
    saturation: f32,
    sharpen: f32,
    offset: [u32; 2],
    pad2: [u32; 2],
}

/// Where the camera is and what it sees.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
    pub near: f32,
}

/// Light and look for a frame, in physical units.
#[derive(Clone, Debug)]
pub struct Lighting {
    /// Unit vector towards the sun.
    pub sun_dir: Vec3,
    /// Illuminance of the sun on a surface facing it (lux).
    pub sun_lux: f32,
    pub sun_color: Vec3,
    pub shadow_strength: f32,
    /// Horizontal irradiance of the sky without the sun (lux).
    pub sky_lux: f32,
    /// Exposure value at ISO 100 (sunny 16: about 15).
    pub ev100: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_density: f32,
    /// Zero retains legacy image fog. Positive values enable a local aerosol
    /// layer whose density decays above world y=0, in metres.
    pub fog_height_m: f32,
    /// Normalised aerosol phase asymmetry; shared with the atmosphere bake.
    pub fog_phase_g: f32,
    /// Scattering / extinction ratio, not a display colour multiplier.
    pub fog_scattering_albedo: Vec3,
    /// Camera-only rejection after exact full fog; disable for A/B fixtures.
    pub fog_culling: bool,
    pub contrast: f32,
    pub saturation: f32,
    /// Cluster error allowed on screen, in pixels.
    pub lod_pixels: f32,
    /// 1: sky visibility, canopy sun and bounce from the GI volume; 0: off.
    pub gi: f32,
    /// Analytic near-field visibility from moving opaque car geometry.
    pub contact_occlusion: f32,
    /// Visibility along the glossy reflected direction.
    pub reflection_occlusion: f32,
    /// Per-view high-resolution sun detail; set 0 for an isolated cost check.
    pub near_shadows: f32,
    /// Debug: 1 sky visibility, 2 albedo, 3 direct, 4 diffuse indirect,
    /// 5 environment specular, 6 contact visibility, 7 reflection visibility,
    /// 8 final sun visibility, 9 shadow maps/fallback, 10 local grass/terrain sun,
    /// 11 shaded normal, 12 source normal, 13 shaded N.L, 14 source N.L.
    pub debug: f32,
    /// Temporal anti-aliasing: 1 on (jittered frames, history), 0 off.
    pub taa: f32,
    /// Least weight of the new frame in the TAA blend (lower: smoother, more ghosting).
    pub taa_blend: f32,
    /// How far (in sigmas) a still image's history may differ from the frame.
    pub taa_gamma: f32,
    /// Alpha test threshold for needle cards (Godot's foliage: 0.42).
    pub alpha_cut: f32,
    /// Developer reference: 0 blended grass, 1 near geometry, 2 far volume.
    pub grass_reference: f32,
    /// Sharpening after TAA, 0..1.
    pub sharpen: f32,
}

impl Lighting {
    /// Set a field by name (`sun_lux=50000`): tune the look without a rebuild.
    pub fn set(&mut self, kv: &str) -> Result<(), String> {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got {kv}"))?;
        let f: f32 = v.parse().map_err(|_| format!("{k}: not a number: {v}"))?;
        match k {
            "sun_lux" => self.sun_lux = f,
            "shadow_strength" => self.shadow_strength = f,
            "sky_lux" => self.sky_lux = f,
            "ev100" => self.ev100 = f,
            "fog_start" => self.fog_start = f,
            "fog_end" => self.fog_end = f,
            "fog_density" => self.fog_density = f,
            "fog_height_m" if f.is_finite() && f >= 0.0 => self.fog_height_m = f,
            "fog_phase_g" if f.is_finite() && f.abs() < 0.99 => self.fog_phase_g = f,
            "fog_culling" if f == 0.0 || f == 1.0 => self.fog_culling = f == 1.0,
            "contrast" => self.contrast = f,
            "saturation" => self.saturation = f,
            "lod_pixels" => self.lod_pixels = f,
            "gi" => self.gi = f,
            "contact_occlusion" => self.contact_occlusion = f,
            "reflection_occlusion" => self.reflection_occlusion = f,
            "near_shadows" => self.near_shadows = f,
            "debug" => self.debug = f,
            "taa" => self.taa = f,
            "taa_blend" => self.taa_blend = f,
            "taa_gamma" => self.taa_gamma = f,
            "alpha_cut" => self.alpha_cut = f,
            "grass_reference" if f == 0.0 || f == 1.0 || f == 2.0 => self.grass_reference = f,
            "sharpen" => self.sharpen = f,
            "sun_elevation" | "sun_azimuth" => {
                let el = if k == "sun_elevation" { f } else { self.sun_dir.y.asin().to_degrees() };
                let az = if k == "sun_azimuth" { f } else { self.sun_dir.x.atan2(-self.sun_dir.z).to_degrees() };
                let (el, az) = (el.to_radians(), az.to_radians());
                self.sun_dir = Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos());
            }
            _ => return Err(format!("unknown light setting {k}")),
        }
        Ok(())
    }
}

impl Default for Lighting {
    fn default() -> Self {
        let el = 30f32.to_radians();
        let az = 125f32.to_radians();
        Lighting {
            sun_dir: Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()),
            sun_lux: 100_000.0,
            sun_color: Vec3::new(1.0, 0.96, 0.9),
            shadow_strength: 1.0,
            sky_lux: 18_000.0,
            ev100: 14.0,
            fog_start: 140.0,
            fog_end: 750.0,
            fog_density: 0.0,
            fog_height_m: 0.0,
            fog_phase_g: 0.8,
            fog_scattering_albedo: Vec3::splat(0.9),
            fog_culling: true,
            contrast: 1.0,
            saturation: 1.0,
            lod_pixels: 1.0,
            gi: 1.0,
            contact_occlusion: 1.0,
            reflection_occlusion: 1.0,
            near_shadows: 1.0,
            debug: 0.0,
            taa: 1.0,
            taa_blend: 0.1,
            taa_gamma: 1.5,
            alpha_cut: 0.42,
            grass_reference: 0.0,
            sharpen: 0.5,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct FrameStats {
    pub upscale: crate::upscale::UpscaleStatus,
    pub clusters: u32,
    pub triangles: u32,
    /// Actual vertex invocations submitted by the opaque cluster draws.
    /// Includes padding, excludes later glass/grass/volume passes.
    pub submitted_vertices: u64,
    pub shadow_submitted_vertices: u64,
    /// Live-grass instances whose opaque draw was already invisible.
    pub early_grass_rejects: u64,
    pub cluster_buckets: bool,
    pub early_grass_cull: bool,
    pub shadow_clusters: u32,
    pub shadow_triangles: u32,
    /// Subset of total shadow work spent on the per-view near maps.
    pub near_shadow_clusters: u32,
    pub near_shadow_triangles: u32,
    /// Per view: bit mask of rebuilt forest levels; zero is a cache hit.
    pub canopy_updated_levels: Vec<u32>,
    /// Per view: world voxel pitch of the three forest levels, in metres.
    pub canopy_pitch_m: Vec<[f32; 3]>,
    /// GPU time per pass in ms, when the GPU has timestamps.
    pub gpu_ms: Vec<(&'static str, f64)>,
    /// For each requested composite-output pixel: (instance, cluster, triangle)
    /// or None for sky, a split-screen gap, or a point outside all views.
    pub picks: Vec<Option<(u32, u32, u32)>>,
}

#[derive(Clone)]
struct ViewTargets {
    uniform: wgpu::Buffer,
    visible: wgpu::Buffer,
    counter: wgpu::Buffer,
    args: wgpu::Buffer,
    cull_bind: wgpu::BindGroup,
    raster_bind: wgpu::BindGroup,
}

pub struct Renderer {
    upscaling: std::sync::Mutex<Upscaling>,
    frame_time_ms: std::sync::atomic::AtomicU32,
    scene_bind: wgpu::BindGroup,
    shadow_scene_bind: wgpu::BindGroup,
    headlights: crate::headlights::Resources,
    headlight_state: std::sync::Mutex<Vec<crate::headlights::SpotLight>>,
    headlight_targets: std::sync::Mutex<Vec<ViewTargets>>,
    instance_buffer: wgpu::Buffer,
    wind: crate::wind::WindRenderer,
    part_wind: Option<crate::part_wind::PartWindRenderer>,
    baked_animations: Option<crate::baked_animation::AnimationRenderer>,
    transparency: Option<crate::transparency::Transparency>,
    grass: Option<crate::grass::GrassRenderer>,
    grass_detail: crate::vegetation_detail::DistanceProfile,
    far_grass: Option<crate::far_grass::FarGrassRenderer>,
    voxel_forest: Option<crate::voxel_forest::VoxelRenderer>,
    volume_view_layout: wgpu::BindGroupLayout,
    cull: wgpu::ComputePipeline,
    finalize: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    cluster_buckets: bool,
    early_grass_cull: bool,
    raster: wgpu::RenderPipeline,
    raster_depth: wgpu::RenderPipeline,
    shade: wgpu::ComputePipeline,
    /// Kept to rebuild `shade` when its source changes on disk.
    shade_pl: wgpu::PipelineLayout,
    shade_text: String,
    car_materials: wgpu::Buffer,
    car_material_count: usize,
    tone: wgpu::ComputePipeline,
    shade_layout: wgpu::BindGroupLayout,
    tone_layout: wgpu::BindGroupLayout,
    view_cull_layout: wgpu::BindGroupLayout,
    view_raster_layout: wgpu::BindGroupLayout,
    instances: u32,
    sky_view: wgpu::TextureView,
    sky_sh: [[f32; 4]; 9],
    sky_horizon: Vec3,
    sky_scale: f32,
    sky_limit: f32,
    sky_rotation: f32,
    shadow_samp: wgpu::Sampler,
    canopies: std::sync::Mutex<Vec<crate::canopy::CanopyClipmap>>,
    instance_flags: std::sync::Mutex<Vec<u32>>,
    foliage_revision: std::sync::atomic::AtomicU64,
    density_sampler: wgpu::Sampler,
    solid_occluders: wgpu::Buffer,
    solid_count: u32,
    heightfield: wgpu::TextureView,
    hf: crate::scene::Heightfield,
    query: Option<(wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer)>,
    taa: wgpu::ComputePipeline,
    taa_layout: wgpu::BindGroupLayout,
    clamp_samp: wgpu::Sampler,
    history: std::sync::Mutex<Vec<Option<History>>>,
    frame: std::sync::atomic::AtomicU32,
    shadow_cache: std::sync::Mutex<Option<ShadowCache>>,
    shadow_extent: std::sync::Mutex<f32>,
    near_shadows: std::sync::Mutex<Vec<NearShadow>>,
    shadow_capacity: u32,
    cluster_views: std::sync::Mutex<Vec<ViewTargets>>,
}

fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }
}

fn layout(gpu: &Gpu, label: &str, entries: &[(u32, wgpu::BindingType)]) -> wgpu::BindGroupLayout {
    let all = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
    // Writable bindings are for compute only (vertex stages cannot write).
    let writable = |ty: &wgpu::BindingType| matches!(ty, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, .. } | wgpu::BindingType::StorageTexture { .. });
    let e: Vec<_> = entries
        .iter()
        .map(|(b, ty)| wgpu::BindGroupLayoutEntry { binding: *b, visibility: if writable(ty) { wgpu::ShaderStages::COMPUTE } else { all }, ty: *ty, count: None })
        .collect();
    gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &e })
}

fn buffer(gpu: &Gpu, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    // Storage bindings cannot be empty.
    let pad = [0u8; 16];
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: if data.is_empty() { &pad } else { data }, usage })
}

fn bind(gpu: &Gpu, layout: &wgpu::BindGroupLayout, entries: &[(u32, wgpu::BindingResource)]) -> wgpu::BindGroup {
    let e: Vec<_> = entries.iter().map(|(b, r)| wgpu::BindGroupEntry { binding: *b, resource: r.clone() }).collect();
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries: &e })
}

/// Textures as one array with mips, all resized to `size`. Layers without
/// an image get `fill`. Cut-out albedo keeps its alpha-test coverage per mip.
/// Mips are built on all cores.
fn texture_array(gpu: &Gpu, label: &str, images: &[Option<&image::RgbaImage>], size: u32, format: wgpu::TextureFormat, fill: [u8; 4], cutout_layers: &[bool]) -> wgpu::Texture {
    let linear_cutout_rgb = std::env::var("KERN_CUTOUT_LINEAR_RGB").as_deref() != Ok("0");
    let layers = images.len().max(1) as u32;
    let mips = size.ilog2() + 1;
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let chain = |src: Option<&image::RgbaImage>, preserve_alpha: bool| -> Vec<image::RgbaImage> {
        let Some(src) = src else {
            return (0..mips).map(|m| image::RgbaImage::from_pixel((size >> m).max(1), (size >> m).max(1), image::Rgba(fill))).collect();
        };
        let mut img = if src.dimensions() == (size, size) { src.clone() } else { image::imageops::resize(src, size, size, image::imageops::FilterType::Triangle) };
        let cutout = preserve_alpha && img.pixels().any(|p| p[3] < 250);
        let target = coverage(&img, 1.0);
        let mut out = vec![img.clone()];
        for mip in 1..mips {
            let s = (size >> mip).max(1);
            img = image::imageops::resize(&img, s, s, image::imageops::FilterType::Triangle);
            if cutout {
                preserve_coverage(&mut img, target);
            }
            out.push(img.clone());
        }
        if linear_cutout_rgb && cutout && format == wgpu::TextureFormat::Rgba8UnormSrgb {
            // Keep the alpha chain byte-identical. RGB must start at the source,
            // before even the base resize can mix transparent black into leaves.
            cutout_mips::correct_rgb(src, &mut out);
        }
        out
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    // Linear RGBA scratch is 16 bytes/texel, plus resize/frontier storage.
    // Bound transient CPU memory while building large cutout colour arrays.
    let threads = if format == wgpu::TextureFormat::Rgba8UnormSrgb { threads.min(4) } else { threads };
    let chains: Vec<Vec<image::RgbaImage>> = std::thread::scope(|sc| {
        let per = images.len().div_ceil(threads).max(1);
        let handles: Vec<_> = images.chunks(per).enumerate().map(|(chunk, part)| sc.spawn(move || part.iter().enumerate().map(|(index, i)| chain(*i, cutout_layers.get(chunk * per + index).copied().unwrap_or(false))).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for (layer, mips) in chains.iter().enumerate() {
        for (mip, img) in mips.iter().enumerate() {
            let s = img.width();
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: mip as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                img,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * s), rows_per_image: Some(s) },
                wgpu::Extent3d { width: s, height: s, depth_or_array_layers: 1 },
            );
        }
    }
    tex
}

/// Fraction of texels that pass a 0.5 alpha test with alpha scaled by `k`.
fn coverage(img: &image::RgbaImage, k: f32) -> f32 {
    let n = img.pixels().filter(|p| p[3] as f32 * k >= 127.5).count();
    n as f32 / (img.width() * img.height()).max(1) as f32
}

/// Scale a mip's alpha so as many texels pass the alpha test as at full
/// size: foliage keeps its density in the distance instead of thinning out.
fn preserve_coverage(img: &mut image::RgbaImage, target: f32) {
    let (mut lo, mut hi) = (0.25f32, 8.0f32);
    for _ in 0..12 {
        let mid = (lo + hi) * 0.5;
        if coverage(img, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let k = (lo + hi) * 0.5;
    for p in img.pixels_mut() {
        p[3] = (p[3] as f32 * k).min(255.0) as u8;
    }
}

/// SH9 projection of an equirectangular sky (radiance in, irradiance
/// coefficients out), leaving out the sun: the sun is its own light.
fn sky_sh(w: u32, h: u32, px: &[[f32; 4]], rotation: f32, clamp: f32) -> ([[f32; 4]; 9], Vec3) {
    // A coarse HDRI texel covers a finite spherical patch. Point sampling a
    // 2x1 constant fallback (especially with step=4) turns it into a false
    // directional light. Integrate low-resolution texels exactly; retain the
    // existing sampled path for production-resolution HDRIs.
    if w < 64 || h < 32 { return sky_sh_small(w,h,px,rotation,clamp); }
    let mut sh = [[0.0f32; 4]; 9];
    let mut horizon = Vec3::ZERO;
    let mut horizon_w = 0.0f32;
    let step = 4;
    for y in (0..h).step_by(step) {
        let theta = (y as f32 + 0.5) / h as f32 * PI;
        let dw = (2.0 * PI / w as f32) * (PI / h as f32) * theta.sin() * (step * step) as f32;
        for x in (0..w).step_by(step) {
            let phi = ((x as f32 + 0.5) / w as f32 - 0.5) * 2.0 * PI;
            // Inverse of sky_dir_uv in the shader.
            let (sx, sz) = (phi.sin() * theta.sin(), -phi.cos() * theta.sin());
            let d = Vec3::new(sx * rotation.cos() + sz * rotation.sin(), theta.cos(), -sx * rotation.sin() + sz * rotation.cos());
            let c = px[(y * w + x) as usize];
            let c = Vec3::new(c[0].min(clamp), c[1].min(clamp), c[2].min(clamp));
            let basis = [
                0.282095,
                0.488603 * d.y,
                0.488603 * d.z,
                0.488603 * d.x,
                1.092548 * d.x * d.y,
                1.092548 * d.y * d.z,
                0.315392 * (3.0 * d.z * d.z - 1.0),
                1.092548 * d.x * d.z,
                0.546274 * (d.x * d.x - d.y * d.y),
            ];
            for k in 0..9 {
                sh[k][0] += c.x * basis[k] * dw;
                sh[k][1] += c.y * basis[k] * dw;
                sh[k][2] += c.z * basis[k] * dw;
            }
            if d.y.abs() < 0.1 {
                horizon += c;
                horizon_w += 1.0;
            }
        }
    }
    // Shader uses Ramamoorthi-Hanrahan coefficient order (y, z, x terms),
    // with s[6] for the z^2 term; this order matches it.
    (sh, horizon / horizon_w.max(1.0))
}

pub(crate) fn sky_sh_small(w:u32,h:u32,px:&[[f32;4]],rotation:f32,clamp:f32)->([[f32;4];9],Vec3) {
    use std::f64::consts::PI;
    let mut sh=[[0.0f64;3];9];let mut horizon=[0.0f64;3];let mut horizon_weight=0.0;
    let band=(0.1f64.acos(),(-0.1f64).acos());
    for y in 0..h {
        let t0=y as f64/h as f64*PI;let t1=(y+1) as f64/h as f64*PI;
        let (s0,c0)=t0.sin_cos();let (s1,c1)=t1.sin_cos();
        let sin1=c0-c1;
        let sin2=(t1-t0)*0.5-((2.0*t1).sin()-(2.0*t0).sin())*0.25;
        let sin3=(-c1+c1.powi(3)/3.0)-(-c0+c0.powi(3)/3.0);
        let sin_cos=(s1*s1-s0*s0)*0.5;
        let sin2_cos=(s1.powi(3)-s0.powi(3))/3.0;
        let sin_cos2=(c0.powi(3)-c1.powi(3))/3.0;
        let horizon_sin=if t1.min(band.1)>t0.max(band.0) {t0.max(band.0).cos()-t1.min(band.1).cos()}else{0.0};
        for x in 0..w {
            // World direction is (sin(phi)*sin(theta), cos(theta),
            // -cos(phi)*sin(theta)), with the HDRI rotation subtracted.
            let p0=(x as f64/w as f64-0.5)*2.0*PI-rotation as f64;
            let p1=((x+1)as f64/w as f64-0.5)*2.0*PI-rotation as f64;
            let dp=p1-p0;let sin_phi=p0.cos()-p1.cos();let cos_phi=p1.sin()-p0.sin();
            let sin2_phi=dp*0.5-((2.0*p1).sin()-(2.0*p0).sin())*0.25;
            let cos2_phi=dp-sin2_phi;let sin_cos_phi=(p1.sin().powi(2)-p0.sin().powi(2))*0.5;
            let area=sin1*dp;
            let moments=[0.282095*area,0.488603*sin_cos*dp,-0.488603*sin2*cos_phi,
                0.488603*sin2*sin_phi,1.092548*sin2_cos*sin_phi,-1.092548*sin2_cos*cos_phi,
                0.315392*(3.0*sin3*cos2_phi-area),-1.092548*sin3*sin_cos_phi,
                0.546274*(sin3*sin2_phi-sin_cos2*dp)];
            let horizon_area=horizon_sin*dp;horizon_weight+=horizon_area;
            for channel in 0..3 {
                let radiance=px[(y*w+x)as usize][channel].min(clamp)as f64;
                for k in 0..9 {sh[k][channel]+=radiance*moments[k];}
                horizon[channel]+=radiance*horizon_area;
            }
        }
    }
    (sh.map(|c|[c[0]as f32,c[1]as f32,c[2]as f32,0.0]),
        Vec3::from_array(horizon.map(|v|(v/horizon_weight.max(1e-12))as f32)))
}

fn eval_irradiance_up(sh: &[[f32; 4]; 9]) -> f32 {
    // Same formula as the shader for n = +Y (the shader's convention).
    let (c1, c2, c4, c5) = (0.429043, 0.511664, 0.886227, 0.247708);
    let g = |k: usize| sh[k][1];
    c1 * g(8) * (-1.0) + c4 * g(0) - c5 * g(6) + 2.0 * c2 * g(1)
}

impl Renderer {
    pub fn new(gpu: &Gpu, scene: &Scene, light: &Lighting) -> Renderer {
        let u = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let vertices = buffer(gpu, "vertices", bytemuck::cast_slice(&scene.vertices), u);
        let cverts = buffer(gpu, "cluster verts", bytemuck::cast_slice(&scene.cluster_verts), u);
        let tris = buffer(gpu, "tris", bytemuck::cast_slice(&scene.tris), u);
        let clusters = buffer(gpu, "clusters", bytemuck::cast_slice(&scene.clusters), u);
        let instances = buffer(gpu, "instances", bytemuck::cast_slice(&scene.instances), u | wgpu::BufferUsages::COPY_SRC);
        let materials = buffer(gpu, "materials", bytemuck::cast_slice(&scene.materials), u);
        let aux = buffer(gpu, "aux", bytemuck::cast_slice(&scene.aux), u);
        // Never empty: a binding needs at least one element.
        let car_list = if scene.car_materials.is_empty() { vec![crate::scene::GpuCarMaterial::default()] } else { scene.car_materials.clone() };
        let car_materials = buffer(gpu, "car materials", bytemuck::cast_slice(&car_list), u);
        let voxel_replacement = buffer(gpu, "voxel crown replacement", bytemuck::cast_slice(&crate::voxel_forest::replacement_materials(scene)), u);
        let wind = crate::wind::WindRenderer::new(gpu, &instances, &voxel_replacement, scene.instances.len())
            .expect("scene exceeds bounded wind storage");
        let albedo_src: Vec<Option<&image::RgbaImage>> = scene.textures.iter().map(|t| Some(&t.rgba)).collect();
        // Continuous alpha must average coverage naturally. Applying foliage's
        // threshold-preserving mip scale to glass/decals changes their opacity.
        let mut cutout_layers = vec![true; scene.textures.len()];
        for &material in scene.blended_materials.keys() {
            let layer = scene.materials[material as usize].layer;
            if layer >= 0 { cutout_layers[layer as usize] = false; }
        }
        let albedo = texture_array(gpu, "albedo", &albedo_src, TEXTURE_SIZE, wgpu::TextureFormat::Rgba8UnormSrgb, [255; 4], &cutout_layers);
        let albedo_view = albedo.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        // Normal maps, same layers; flat where a texture has none.
        let normal_src: Vec<Option<&image::RgbaImage>> = scene.textures.iter().map(|t| t.normal.as_ref()).collect();
        let normals = texture_array(gpu, "normals", &normal_src, NORMAL_SIZE, wgpu::TextureFormat::Rgba8Unorm, [128, 128, 255, 255], &[]);
        let normals_view = normals.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let samp = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        // A cropped plant is a single photograph, not a tiling surface.
        // Clamp both axes so anisotropic taps cannot wrap roots into tips.
        let plant_samp = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let tex_array = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false };
        let scene_layout = layout(
            gpu,
            "scene",
            &[
                (0, storage(true)),
                (1, storage(true)),
                (2, storage(true)),
                (3, storage(true)),
                (4, storage(true)),
                (5, storage(true)),
                (6, tex_array),
                (7, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                (8, storage(true)),
                (9, tex_array),
                (10, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                (11, storage(true)),
                (12, storage(true)),
                (13, uniform()),
                (14, wgpu::BindingType::Texture { sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false }),
                (15, wgpu::BindingType::Texture { sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false }),
                (16, wgpu::BindingType::Texture { sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false }),
                (17, uniform()),
                (18, wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false}),
                (19, storage(true)),
            ],
        );
        let transparency = crate::transparency::Transparency::new(gpu, scene, &scene_layout);
        let grass = crate::grass::GrassRenderer::new(gpu, scene, &scene_layout);
        let volume_view_layout=crate::volume_view::create_layout(&gpu.device);
        let far_grass = scene.far_grass.as_ref().map(|field|
            crate::far_grass::FarGrassRenderer::new(gpu, field, &scene_layout, &volume_view_layout)
                .expect("invalid far grass field"));
        let voxel_forest = crate::voxel_forest::VoxelRenderer::new(gpu, scene, &scene_layout, &volume_view_layout);
        let part_wind = if scene.part_wind.is_empty() { None } else {
            Some(crate::part_wind::PartWindRenderer::new(gpu, &scene.part_wind,
                &scene.instances, &instances,
                scene.voxel_forest.as_ref().zip(voxel_forest.as_ref().map(|f| f.tree_buffer())))
                .expect("invalid procedural vegetation part hierarchy"))
        };
        let baked_animations = scene.baked_animations.as_ref().map(|animation|
            crate::baked_animation::AnimationRenderer::new(gpu, animation, &scene.instances, &instances)
                .expect("invalid baked animation scene"));
        let grass_lighting=crate::grass_lighting::Resources::new(gpu,scene.far_grass.as_ref().zip(far_grass.as_ref()));
        let headlights=crate::headlights::Resources::new(gpu);
        let make_scene_bind = |headlight_view:&wgpu::TextureView| bind(
            gpu,
            &scene_layout,
            &[
                (0, vertices.as_entire_binding()),
                (1, cverts.as_entire_binding()),
                (2, tris.as_entire_binding()),
                (3, clusters.as_entire_binding()),
                (4, instances.as_entire_binding()),
                (5, materials.as_entire_binding()),
                (6, wgpu::BindingResource::TextureView(&albedo_view)),
                (7, wgpu::BindingResource::Sampler(&samp)),
                (8, aux.as_entire_binding()),
                (9, wgpu::BindingResource::TextureView(&normals_view)),
                (10, wgpu::BindingResource::Sampler(&plant_samp)),
                (11, voxel_replacement.as_entire_binding()),
                (12, wind.buffer().as_entire_binding()),
                (13, grass_lighting.uniform.as_entire_binding()),
                (14, wgpu::BindingResource::TextureView(&grass_lighting.terrain)),
                (15, wgpu::BindingResource::TextureView(&grass_lighting.coverage)),
                (16, wgpu::BindingResource::TextureView(&grass_lighting.tiles)),
                (17, headlights.uniform.as_entire_binding()),
                (18, wgpu::BindingResource::TextureView(headlight_view)),
                (19, car_materials.as_entire_binding()),
            ],
        );
        let scene_bind=make_scene_bind(&headlights.sampled);
        let shadow_scene_bind=make_scene_bind(&headlights.empty);
        let view_cull_layout = layout(gpu, "view cull", &[(0, uniform()), (1, storage(false)), (2, storage(false)), (3, storage(false)), (4, storage(false))]);
        let view_raster_layout = layout(gpu, "view raster", &[(0, uniform()), (1, storage(true)), (2, storage(true)), (3, storage(true))]);
        let shade_layout = layout(
            gpu,
            "shade",
            &[
                (0, uniform()),
                (1, storage(true)),
                (2, uniform()),
                (3, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (4, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (5, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (6, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                (7, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (8, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2 }),
                (10, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (11, uniform()),
                (12, storage(true)),
                (13, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                (14, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                (15, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            ],
        );
        let canopy = crate::canopy::CanopyClipmap::new(gpu, scene, &scene_layout)
            .expect("scene exceeds the bounded canopy representation");
        let tone_layout = layout(
            gpu,
            "tone",
            &[
                (0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (1, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba8Unorm, view_dimension: wgpu::TextureViewDimension::D2 }),
                (2, uniform()),
            ],
        );
        let float_tex = |filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let taa_layout = layout(
            gpu,
            "taa",
            &[
                (0, float_tex(false)),
                (1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (2, float_tex(true)),
                (3, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2 }),
                (4, uniform()),
                (5, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        );
        let module = |name: &str, src: &str| gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(name), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let cull_mod = module("cull", &cull_source());
        let raster_mod = module("raster", &crate::grass::raster_source());
        let shade_mod = module("shade", &crate::wind::shade_source());
        let tone_mod = module("tonemap", include_str!("shaders/tonemap.wgsl"));
        let pl = |layouts: &[&wgpu::BindGroupLayout]| {
            let l: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(|x| Some(*x)).collect();
            gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &l, immediate_size: 0 })
        };
        let compute = |layout: &wgpu::PipelineLayout, m: &wgpu::ShaderModule, entry: &str| {
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module: m,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let cull_pl = pl(&[&scene_layout, &view_cull_layout]);
        // Constructor-time switches permit same-binary ABBA comparisons with
        // identical assets, LOD thresholds, lighting and camera inputs.
        let cluster_buckets = std::env::var("KERN_CLUSTER_BUCKETS").as_deref() != Ok("0");
        let early_grass_cull = std::env::var("KERN_EARLY_GRASS_CULL").as_deref() != Ok("0");
        let cull_constants = [("CLUSTER_BUCKETS", u8::from(cluster_buckets) as f64),
                              ("EARLY_GRASS_CULL", u8::from(early_grass_cull) as f64)];
        let raster_constants = [("CLUSTER_BUCKETS", u8::from(cluster_buckets) as f64)];
        let cull_compute = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(&cull_pl), module: &cull_mod, entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &cull_constants, ..Default::default() },
            cache: None,
        });
        let cull = cull_compute("cull");
        let finalize = cull_compute("finalize");
        let scatter = cull_compute("scatter");
        let raster_pl = pl(&[&scene_layout, &view_raster_layout]);
        let raster_pipe = |color: bool| {
            let targets = [Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })];
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if color { "raster" } else { "raster depth" }),
                layout: Some(&raster_pl),
                vertex: wgpu::VertexState { module: &raster_mod, entry_point: Some("vs"), compilation_options: wgpu::PipelineCompilationOptions { constants: &raster_constants, ..Default::default() }, buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    // Main view: reversed Z (1 near, 0 at infinity). Shadow: ordinary.
                    depth_compare: Some(if color { wgpu::CompareFunction::Greater } else { wgpu::CompareFunction::Less }),
                    stencil: Default::default(),
                    bias: if color { Default::default() } else { wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &raster_mod,
                    entry_point: Some(if color { "fs" } else { "fs_depth" }),
                    compilation_options: Default::default(),
                    targets: if color { &targets } else { &[] },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let raster = raster_pipe(true);
        let raster_depth = raster_pipe(false);
        let shade_pl = pl(&[&scene_layout, &shade_layout]);
        let shade = compute(&shade_pl, &shade_mod, "shade");
        let tone = compute(&pl(&[&tone_layout]), &tone_mod, "main");
        let taa_mod = module("taa", include_str!("shaders/taa.wgsl"));
        let taa = compute(&pl(&[&taa_layout]), &taa_mod, "main");
        let clamp_samp = gpu.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let density_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("linear canopy"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let solids = scene.solid_occluders();
        let solid_count = solids.len() as u32;
        let solid_occluders = buffer(gpu, "solid occluders", if solids.is_empty() { &[0u8; 48] } else { bytemuck::cast_slice(&solids) }, wgpu::BufferUsages::STORAGE);
        let hf = scene.heightfield.clone();
        let (hw, hh, hdata) = if hf.w > 1 && hf.h > 1 { (hf.w, hf.h, hf.heights.clone()) } else { (2, 2, vec![-1e9f32; 4]) };
        let hf_tex = gpu.device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("heightfield"),
                size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            Default::default(),
            bytemuck::cast_slice(&hdata),
        );

        // Sky: turn the HDRI so its sun sits where our sun is, scale it so
        // its diffuse light matches `sky_lux`.
        let (sw, shh, spx) = scene.sky.clone().unwrap_or((2, 1, vec![[0.5, 0.6, 0.8, 1.0]; 2]));
        let (mut bx, mut by, mut bv) = (0, 0, 0.0f32);
        for y in 0..shh / 2 {
            for x in 0..sw {
                let c = spx[(y * sw + x) as usize];
                let l = c[0] + c[1] + c[2];
                if l > bv {
                    (bx, by, bv) = (x, y, l);
                }
            }
        }
        let hdri_az = ((bx as f32 + 0.5) / sw as f32 - 0.5) * 2.0 * PI;
        let our_az = light.sun_dir.x.atan2(-light.sun_dir.z);
        let rotation = if scene.sky_physical { 0.0 } else { hdri_az - our_az };
        let _ = by;
        let mean: f32 = spx.iter().map(|c| c[0] + c[1] + c[2]).sum::<f32>() / (spx.len() as f32 * 3.0);
        let (raw_sh, raw_horizon) = if scene.sky_physical {
            // Baked atmosphere is already calibrated and contains no solar
            // disc. Integrate every finite texel; no HDRI sun clipping.
            sky_sh_small(sw, shh, &spx, 0.0, f32::MAX)
        } else { sky_sh(sw, shh, &spx, rotation, mean * 8.0) };
        let up = eval_irradiance_up(&raw_sh).max(1e-6);
        let sky_scale = if scene.sky_physical { 1.0 } else { light.sky_lux / up };
        let sky_limit = if scene.sky_physical {
            spx.iter().flat_map(|p|p[..3].iter().copied()).fold(0.0,f32::max)
        } else { mean * 8.0 * sky_scale };
        let mut sky_sh = raw_sh;
        for c in sky_sh.iter_mut() {
            for k in 0..3 {
                c[k] *= sky_scale;
            }
        }
        let sky_tex = gpu.device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("sky"),
                size: wgpu::Extent3d { width: sw, height: shh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            Default::default(),
            bytemuck::cast_slice(&spx),
        );
        let shadow_samp = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let query = if gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            let n = TIMESTAMP_CAPACITY;
            let qs = gpu.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("timestamps"), ty: wgpu::QueryType::Timestamp, count: n });
            let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 8 * n as u64, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 8 * n as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            Some((qs, resolve, read))
        } else {
            None
        };
        Renderer {
            upscaling: Default::default(),
            frame_time_ms: std::sync::atomic::AtomicU32::new((1000.0f32 / 60.0).to_bits()),
            scene_bind,
            shadow_scene_bind,
            headlights,
            headlight_state: Default::default(),
            headlight_targets: Default::default(),
            instance_buffer: instances,
            wind,
            part_wind,
            baked_animations,
            transparency,
            grass,
            grass_detail: scene.grass_detail,
            far_grass,
            voxel_forest,
            volume_view_layout,
            cull,
            finalize,
            scatter,
            cluster_buckets,
            early_grass_cull,
            raster,
            raster_depth,
            shade,
            shade_pl,
            shade_text: crate::wind::shade_source(),
            car_materials,
            car_material_count: car_list.len(),
            tone,
            shade_layout,
            tone_layout,
            view_cull_layout,
            view_raster_layout,
            instances: scene.instances.len() as u32,
            sky_view: sky_tex.create_view(&Default::default()),
            sky_sh,
            sky_horizon: raw_horizon * sky_scale,
            sky_scale,
            sky_limit,
            sky_rotation: rotation,
            shadow_samp,
            canopies: std::sync::Mutex::new(vec![canopy]),
            instance_flags: std::sync::Mutex::new(scene.instances.iter().map(|i| i.flags).collect()),
            foliage_revision: Default::default(),
            density_sampler,
            solid_occluders,
            solid_count,
            heightfield: hf_tex.create_view(&Default::default()),
            hf,
            query,
            taa,
            taa_layout,
            clamp_samp,
            history: Default::default(),
            frame: Default::default(),
            shadow_cache: Default::default(),
            shadow_extent: Default::default(),
            near_shadows: Default::default(),
            cluster_views: Default::default(),
            // Upper bound: all clusters of all shadow-casting instances.
            shadow_capacity: scene.instances.iter().filter(|i| i.flags & crate::scene::NO_SHADOW == 0)
                .map(|i| i.cluster_count as u64).sum::<u64>().clamp(1, MAX_VISIBLE as u64) as u32,
        }
    }

    fn view_targets(&self, gpu: &Gpu, label: &str, index: usize) -> ViewTargets {
        // These buffers depend on cluster capacity, not output resolution.
        // Reuse them across frames/resizes; queue writes remain submission-ordered.
        let mut views = self.cluster_views.lock().unwrap();
        while views.len() <= index {
            views.push(self.view_targets_capacity(gpu, label, MAX_VISIBLE));
        }
        views[index].clone()
    }

    fn view_targets_capacity(&self, gpu: &Gpu, label: &str, capacity: u32) -> ViewTargets {
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: std::mem::size_of::<ViewUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let visible = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("visible"), size: capacity as u64 * 8, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let counter = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("counter"), size: 48, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let args = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("args"), size: 80, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT, mapped_at_creation: false });
        let draw_map = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("bucket draw map"), size: capacity as u64 * 4, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let cull_bind = bind(gpu, &self.view_cull_layout, &[(0, uniform.as_entire_binding()), (1, visible.as_entire_binding()), (2, counter.as_entire_binding()), (3, args.as_entire_binding()), (4, draw_map.as_entire_binding())]);
        let raster_bind = bind(gpu, &self.view_raster_layout, &[(0, uniform.as_entire_binding()), (1, visible.as_entire_binding()), (2, draw_map.as_entire_binding()), (3, args.as_entire_binding())]);
        ViewTargets { uniform, visible, counter, args, cull_bind, raster_bind }
    }

    fn cull_and_raster(&self, enc: &mut wgpu::CommandEncoder, gpu: &Gpu, v: &ViewTargets, color: Option<&wgpu::TextureView>, depth: &wgpu::TextureView, ts: &mut u32, names: &mut Vec<&'static str>, name: &'static str) {
        let scene_bind=if color.is_some(){&self.scene_bind}else{&self.shadow_scene_bind};
        gpu.queue.write_buffer(&v.counter, 0, &[0u8; 48]);
        let tw = |ts: &mut u32| {
            self.query.as_ref().map(|(qs, _, _)| {
                let i = timestamp_pair(ts);
                (qs, i)
            })
        };
        {
            let t = tw(ts);
            if t.is_some() {
                names.push(name);
            }
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull"),
                timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
            });
            pass.set_pipeline(&self.cull);
            pass.set_bind_group(0, scene_bind, &[]);
            pass.set_bind_group(1, &v.cull_bind, &[]);
            let n = self.instances;
            pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
            pass.set_pipeline(&self.finalize);
            pass.dispatch_workgroups(1, 1, 1);
            if self.cluster_buckets {
                pass.set_pipeline(&self.scatter);
                // Bounded persistent grid: no full-capacity empty scan, no
                // CPU count readback and no storage/indirect buffer alias.
                pass.dispatch_workgroups(256, 1, 1);
            }
        }
        let t = tw(ts);
        if t.is_some() {
            names.push(if color.is_some() { "raster" } else if name == "near shadow cull" { "near shadow raster" }
                else if name=="headlight shadow cull" {"headlight shadow raster"} else { "shadow raster" });
        }
        let attachments = [color.map(|c| wgpu::RenderPassColorAttachment {
            view: c,
            depth_slice: None,
            resolve_target: None,
            // Empty pixels read as u32::MAX.
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: u32::MAX as f64, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
        })];
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raster"),
            color_attachments: if color.is_some() { &attachments } else { &[] },
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(if color.is_some() { 0.0 } else { 1.0 }), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: t.map(|(qs, i)| wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if color.is_some() { &self.raster } else { &self.raster_depth });
        pass.set_bind_group(0, scene_bind, &[]);
        pass.set_bind_group(1, &v.raster_bind, &[]);
        for bucket in 0..if self.cluster_buckets { 4 } else { 1 } {
            pass.draw_indirect(&v.args, bucket * 16);
        }
    }

    /// Forget the cached sun shadow map (something in the scene moved).
    pub fn invalidate_shadow(&self) {
        *self.shadow_cache.lock().unwrap() = None;
    }

    /// Replace the bounded car-light set atomically; zero lights disables it.
    /// Does not alter the sky, ambient light, simulation or temporal history.
    pub fn set_headlights(&self,lights:&[crate::headlights::SpotLight])->Result<(),String> {
        let validated=crate::headlights::validate(lights)?;
        *self.headlight_state.lock().unwrap()=validated;Ok(())
    }

    /// Replace the shared world sky between frames. The caller bakes off the
    /// render thread; this uploads only the small radiance LUT and commits its
    /// diffuse SH, reflection/background data and direct solar state together.
    /// Geometry, instances, controllers and the race are not reconstructed.
    pub fn update_sky(&mut self, gpu:&Gpu, sky:&crate::atmosphere::BakedSky,
        light:&mut Lighting)->Result<(),String> {
        sky.validate()?;
        let texture=gpu.device.create_texture_with_data(&gpu.queue,&wgpu::TextureDescriptor {
            label:Some("live atmosphere"),
            size:wgpu::Extent3d {width:sky.width,height:sky.height,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
            format:wgpu::TextureFormat::Rgba32Float,
            usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[],
        },Default::default(),bytemuck::cast_slice(&sky.pixels));
        self.sky_view=texture.create_view(&Default::default());
        self.sky_sh=sky.sh;
        self.sky_horizon=sky.horizon;
        self.sky_scale=1.0;
        self.sky_rotation=0.0;
        self.sky_limit=sky.pixels.iter().flat_map(|p|p[..3].iter().copied()).fold(0.0,f32::max);
        sky.apply_lighting(light);
        self.invalidate_shadow();
        self.reset_history();
        Ok(())
    }

    /// One world wind field for every camera, source mesh LOD and sun map.
    /// This changes animation only; static canopy density is not rebuilt.
    /// Call before rendering, using simulation time rather than frame count.
    pub fn set_wind(&self, gpu: &Gpu, state: &crate::wind::WindState) -> Result<(), String> {
        if self.wind.update(gpu, state)? { self.invalidate_shadow(); }
        if let Some(parts) = &self.part_wind {
            if parts.update(gpu, state)? { self.invalidate_shadow(); }
        }
        Ok(())
    }

    /// Shared presentation clock for all rigid actors, shadows and cameras.
    /// Independent of wind strength and simulation input/focus.
    pub fn set_animation_time(&self, seconds: f64) -> Result<(), String> {
        if let Some(animation) = &self.baked_animations {
            if animation.set_time(seconds)? { self.invalidate_shadow(); }
        }
        Ok(())
    }

    /// Replace the procedural car surfaces, one per `Scene::car_materials`
    /// entry: retuning paint, wear or grime needs no new renderer.
    pub fn set_car_materials(&self, gpu: &Gpu, materials: &[crate::scene::GpuCarMaterial]) -> Result<(), String> {
        if materials.is_empty() {
            return Ok(());
        }
        if materials.len() != self.car_material_count {
            return Err(format!("{} car materials for a scene built with {}", materials.len(), self.car_material_count));
        }
        gpu.queue.write_buffer(&self.car_materials, 0, bytemuck::cast_slice(materials));
        Ok(())
    }

    /// Re-read the shading shaders (`KERN_SHADER_DIR`, see shaders.rs) and
    /// rebuild the shade and glass pipelines whose source changed. Code that
    /// fails validation leaves the running pipelines in place and returns
    /// the compiler's message. Ok(true) when something was rebuilt.
    pub fn reload_shaders(&mut self, gpu: &Gpu) -> Result<bool, String> {
        let mut rebuilt = false;
        let text = crate::wind::shade_source();
        if text != self.shade_text {
            self.shade = crate::shaders::checked(gpu, || {
                let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shade"), source: wgpu::ShaderSource::Wgsl(text.as_str().into()) });
                gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("shade"),
                    layout: Some(&self.shade_pl),
                    module: &module,
                    entry_point: Some("shade"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })?;
            self.shade_text = text;
            rebuilt = true;
        }
        if let Some(t) = &mut self.transparency {
            rebuilt |= t.reload(gpu)?;
        }
        if rebuilt {
            self.reset_history();
        }
        Ok(rebuilt)
    }

    /// Forget TAA history: the next frame starts from scratch (a camera cut).
    pub fn reset_history(&self) {
        self.history.lock().unwrap().clear();
        let mut up = self.upscaling.lock().unwrap();
        if let Some(motion) = &up.motion { motion.reset(); }
        up.layout.clear();
    }

    /// Select an explicit reconstruction mode. Failure keeps the prior mode.
    /// Native allocates no DLSS feature or motion history.
    pub fn set_upscale(&self, gpu: &Gpu, mode: crate::upscale::UpscaleMode) -> Result<(), String> {
        let mut up = self.upscaling.lock().unwrap();
        if up.mode == mode { return Ok(()); }
        if mode == crate::upscale::UpscaleMode::Native {
            *up = Upscaling::default();
        } else {
            if self.voxel_forest.is_some() || self.part_wind.is_some() {
                return Err("DLSS is not supported with experimental voxel/branch foliage until their motion reconstruction is connected".into());
            }
            // Initialize everything before committing a requested mode.
            let backend = crate::dlss::DlssContext::new(gpu)?;
            let motion = crate::motion::MotionRenderer::new(gpu, &self.instance_buffer, self.wind.buffer(), self.instances as usize)?;
            *up = Upscaling { mode, backend: Some(backend), motion: Some(motion),
                status: crate::upscale::UpscaleStatus { mode, views: Vec::new() }, layout: Vec::new() };
        }
        drop(up);
        self.history.lock().unwrap().clear();
        Ok(())
    }

    pub fn upscale_status(&self) -> crate::upscale::UpscaleStatus {
        self.upscaling.lock().unwrap().status.clone()
    }

    /// Render-frame duration, not CPU submission time. Pauses/hitches are bounded.
    pub fn set_frame_time_ms(&self, milliseconds: f32) {
        let ms = if milliseconds.is_finite() { milliseconds.clamp(1.0, 100.0) } else { 1000.0 / 60.0 };
        self.frame_time_ms.store(ms.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Render one view to an RGBA8 image.
    pub fn render(&self, gpu: &Gpu, cam: &Camera, light: &Lighting, w: u32, h: u32) -> (Vec<u8>, FrameStats) {
        self.render_picking(gpu, cam, light, w, h, &[])
    }

    /// Render, and report what covers each of `picks` (pixel coordinates).
    pub fn render_picking(&self, gpu: &Gpu, cam: &Camera, light: &Lighting, w: u32, h: u32, picks: &[(u32, u32)]) -> (Vec<u8>, FrameStats) {
        self.render_views(gpu, &[ViewRect { cam: *cam, x: 0, y: 0, w, h }], light, w, h, picks)
    }

    /// Split screen: every view is culled, rastered and shaded on its own
    /// and tonemapped into its rectangle of one `out_w` x `out_h` image.
    /// One sun shadow map covers all the cameras. Picks refer to view 0.
    pub fn render_views(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32, picks: &[(u32, u32)]) -> (Vec<u8>, FrameStats) {
        self.try_render_views(gpu, views, light, out_w, out_h, picks).expect("render views")
    }

    pub fn try_render_views(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32, picks: &[(u32, u32)]) -> Result<(Vec<u8>, FrameStats), String> {
        let (_, pixels, stats) = self.frame_output(gpu, views, light, out_w, out_h, picks, Readback::ImageAndMetrics)?;
        Ok((pixels, stats))
    }

    /// Render directly to a GPU texture for an explicit interactive surface.
    /// No counter, timestamp or full-image readback stalls the live frame.
    pub fn render_texture(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32) -> wgpu::Texture {
        self.try_render_texture(gpu, views, light, out_w, out_h).expect("render texture")
    }

    pub fn try_render_texture(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32) -> Result<wgpu::Texture, String> {
        Ok(self.frame_output(gpu, views, light, out_w, out_h, &[], Readback::None)?.0)
    }

    /// Measure the exact rendered frame for an explicit developer capture.
    /// This waits for GPU counters/timestamps, but avoids an image readback.
    /// Normal gameplay should use `render_texture` instead.
    pub fn render_texture_profiled(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32) -> (wgpu::Texture, FrameStats) {
        self.try_render_texture_profiled(gpu, views, light, out_w, out_h).expect("profiled render texture")
    }

    pub fn try_render_texture_profiled(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32) -> Result<(wgpu::Texture, FrameStats), String> {
        let (texture, _, stats)=self.frame_output(gpu, views, light, out_w, out_h, &[], Readback::Metrics)?;
        Ok((texture,stats))
    }

    /// Explicit offline validation readback; normal frame rendering never waits.
    pub fn voxel_diagnostics(&self, gpu: &Gpu) -> Result<Option<Vec<crate::voxel_forest::VoxelViewDiagnostics>>,String> {
        self.voxel_forest.as_ref().map(|forest|forest.diagnostics(gpu)).transpose()
    }

    /// Explicit development-fixture access to the actual post-render pose,
    /// source-instance and optional volume-instance buffers. No automatic
    /// readback, synchronization or CPU updates occur in the live renderer.
    pub fn part_wind_debug_buffers(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer, Option<&wgpu::Buffer>)> {
        self.part_wind.as_ref().map(|parts| (parts.poses(), &self.instance_buffer,
            self.voxel_forest.as_ref().map(|forest|forest.tree_buffer())))
    }

    /// Explicit validation only: read post-compute rigid actor transforms
    /// without a duplicate CPU animation path or normal-gameplay readback.
    pub fn baked_animation_debug_instances(&self)->Option<&wgpu::Buffer> {
        self.baked_animations.as_ref().map(|_|&self.instance_buffer)
    }

    /// Explicit offline validation; no readback or wait in normal play.
    pub fn far_grass_diagnostics(&self, gpu: &Gpu) -> Result<Option<Vec<crate::far_grass::FarGrassDiagnostics>>,String> {
        self.far_grass.as_ref().map(|grass|grass.diagnostics(gpu)).transpose()
    }

    pub fn far_grass_layers_rgba16(&self, gpu: &Gpu) -> Result<Vec<(u32,u32,Vec<u8>)>,String> {
        match &self.far_grass { Some(grass)=>grass.layers_rgba16(gpu), None=>Ok(Vec::new()) }
    }

    /// Explicit raw premultiplied HDR + alpha readback for voxel validation.
    pub fn voxel_layers_rgba16(&self,gpu:&Gpu)->Result<Vec<(u32,u32,Vec<u8>)>,String> {
        match &self.voxel_forest {Some(forest)=>forest.layers_rgba16(gpu),None=>Ok(Vec::new())}
    }

    /// Update an existing instance range without rebuilding static geometry.
    pub fn update_instances(&self, gpu: &Gpu, first: usize, instances: &[crate::scene::GpuInstance]) -> Result<(), String> {
        if self.baked_animations.as_ref().is_some_and(|animation|animation.contains_owner_range(first,instances.len())) {
            return Err("baked actor parts are driven by the shared GPU animation pass".into());
        }
        if self.part_wind.as_ref().is_some_and(|parts|parts.contains_owner_range(first,instances.len())) {
            return Err("procedural part owners are driven by shared GPU poses, not CPU instance updates".into());
        }
        if first + instances.len() > self.instances as usize { return Err("instance update exceeds uploaded scene".into()); }
        if self.voxel_forest.as_ref().is_some_and(|forest|forest.contains_owner_range(first,instances.len())) {
            return Err("voxel test trees are static; rebuild their packed scene before changing transforms".into());
        }
        self.canopies.lock().unwrap()[0].validate_update(first, instances)?;
        if !instances.is_empty() {
            // Canopy density only changes with foliage. Moving cars and the
            // streamed grass pool must not rebuild the static forest volumes.
            let mut flags = self.instance_flags.lock().unwrap();
            let old = &mut flags[first..first + instances.len()];
            if old.iter().zip(instances).any(|(&before, after)|
                (before | after.flags) & crate::scene::FOLIAGE != 0) {
                self.foliage_revision.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            for (before, after) in old.iter_mut().zip(instances) { *before = after.flags; }
            drop(flags);
            gpu.queue.write_buffer(&self.instance_buffer, (first * std::mem::size_of::<crate::scene::GpuInstance>()) as u64, bytemuck::cast_slice(instances));
            if let Some(transparency) = &self.transparency { transparency.update_instances(first, instances); }
            self.invalidate_shadow();
        }
        Ok(())
    }

    fn frame_output(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32, picks: &[(u32, u32)], readback: Readback) -> Result<(wgpu::Texture, Vec<u8>, FrameStats), String> {
        let tex2d = |label: &str, w: u32, h: u32, format, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
        let ldr = tex2d("ldr", out_w, out_h, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING);
        let ldr_v = ldr.create_view(&Default::default());
        let mut up = self.upscaling.lock().unwrap();
        let dlss_on = up.mode != crate::upscale::UpscaleMode::Native;
        let mode = up.mode;
        let view_layout: Vec<_> = views.iter().map(|v| (v.x, v.y, v.w, v.h)).collect();
        let layout_changed = up.layout != view_layout;
        if layout_changed {
            if let Some(motion) = &up.motion { motion.reset(); }
        }
        let mut upscale_views = Vec::with_capacity(views.len());
        // TAA: this frame's sub-pixel offset and alpha-test threshold.
        let frame = self.frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let taa_on = light.taa > 0.5 && !dlss_on;
        let (jx, jy) = if dlss_on { halton(frame % 32 + 1) } else if taa_on { halton(frame % 16 + 1) } else { (0.0, 0.0) };
        let cut = if taa_on {
            // A different threshold every frame: averaged, the needles get
            // soft partial coverage, like alpha to coverage with many samples.
            let mut i = frame % 25 + 1;
            let (mut f, mut r) = (1.0f32, 0.0f32);
            while i > 0 {
                f /= 5.0;
                r += f * (i % 5) as f32;
                i /= 5;
            }
            light.alpha_cut + (r - 0.5) * 0.3
        } else {
            light.alpha_cut
        };

        // Sun shadow: an orthographic box around what all the cameras see.
        let ahead = |c: &Camera| {
            let f = (c.target - c.pos).normalize_or(Vec3::NEG_Z);
            c.pos + Vec3::new(f.x, 0.0, f.z).normalize_or_zero() * 54.0
        };
        let centre = views.iter().map(|v| ahead(&v.cam)).sum::<Vec3>() / views.len().max(1) as f32;
        let spread = views.iter().map(|v| ahead(&v.cam).distance(centre)).fold(0.0f32, f32::max);
        let mut previous_extent = self.shadow_extent.lock().unwrap();
        let extent = crate::stability::stable_shadow_extent(90.0 + spread, *previous_extent);
        *previous_extent = extent;
        drop(previous_extent);
        let texel = 2.0 * extent / SHADOW_SIZE as f32;
        let svp = crate::stability::stable_shadow(centre, light.sun_dir, extent, SHADOW_SIZE);
        // Instance updates explicitly invalidate this cache. Fixed world texels
        // keep tree silhouettes stable despite small suspension/camera motions.
        let mut cache = self.shadow_cache.lock().unwrap();
        let reuse = cache.as_ref().is_some_and(|c| c.view_proj.abs_diff_eq(svp, 1e-6));
        if !reuse {
            *cache = Some(ShadowCache { view_proj: svp, tex: tex2d("shadow", SHADOW_SIZE, SHADOW_SIZE, wgpu::TextureFormat::Depth32Float, rt), clusters: 0, triangles: 0, submitted_vertices: 0 });
        }
        let shadow_v = cache.as_ref().unwrap().tex.create_view(&Default::default());
        let shadow_t = self.view_targets(gpu, "sun view", 0);
        gpu.queue.write_buffer(
            &shadow_t.uniform,
            0,
            bytemuck::bytes_of(&view_uniform(svp, centre + light.sun_dir * 400.0, SHADOW_SIZE as f32 / (2.0 * extent), [light.lod_pixels * 2.0, 1.0, 1.0, MAX_VISIBLE as f32], [SHADOW_SIZE as f32; 4], light.alpha_cut)),
        );
        let mut sh = self.sky_sh;
        sh.iter_mut().for_each(|c| c[3] = 0.0);
        let lu = LightUniform {
            sun_dir: [light.sun_dir.x, light.sun_dir.y, light.sun_dir.z, light.sun_lux],
            sun_color: [light.sun_color.x, light.sun_color.y, light.sun_color.z, light.shadow_strength],
            shadow_view_proj: svp.to_cols_array_2d(),
            sky: [self.sky_scale, self.sky_rotation, light.debug, texel],
            fog_color: [self.sky_horizon.x, self.sky_horizon.y, self.sky_horizon.z, light.fog_start],
            fog: [light.fog_end, light.fog_density, light.fog_height_m, 0.0],
            fog_scattering: [light.fog_scattering_albedo.x, light.fog_scattering_albedo.y,
                light.fog_scattering_albedo.z, light.fog_phase_g],
            sh,
            near_shadow_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            near_shadow: [2.0 * NEAR_SHADOW_EXTENT / NEAR_SHADOW_SIZE as f32, 0.8, 0.0, 0.0],
        };
        let exposure = 1.0 / (1.2 * 2f32.powf(light.ev100));

        // Each camera retains its own coverage/resolution. A second player
        // moving away must not change this player's forest illumination.
        let revision = self.foliage_revision.load(std::sync::atomic::Ordering::Relaxed);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        // wgpu30 requires native API commands on a separate encoder. All
        // tracked and native segments still submit once, in queue order.
        let mut segments = Vec::new();
        let mut ts = 0u32;
        let mut names = Vec::new();
        // Compute once for the shared scene, before all camera and shadow
        // passes. The same frozen state still updates recycled grass slots.
        let wind_time = self.query.as_ref().map(|(qs, _, _)| (qs, ts));
        if self.wind.encode(&mut enc, wind_time) && wind_time.is_some() {
            timestamp_pair(&mut ts);
            names.push("plant wind");
        }
        if let Some(parts) = &self.part_wind {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, ts));
            if parts.encode(gpu, &mut enc, times) && times.is_some() {
                timestamp_pair(&mut ts);
                names.push("vegetation branch poses");
            }
        }
        if let Some(animation) = &self.baked_animations {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, timestamp_pair(&mut ts)));
            animation.encode(gpu, &mut enc, times);
            if times.is_some() { names.push("baked crowd animation"); }
        }
        // One perspective map per headlamp, shared by every split view. The
        // shadow bind group contains a different dummy sampled texture so the
        // active depth attachment is never simultaneously bound for reading.
        let headlights=self.headlight_state.lock().unwrap().clone();
        self.headlights.upload(gpu,&headlights);
        let mut headlight_targets=self.headlight_targets.lock().unwrap();
        while headlight_targets.len()<headlights.len() {
            headlight_targets.push(self.view_targets_capacity(gpu,"headlight shadow",self.shadow_capacity));
        }
        for (index,lamp) in headlights.iter().enumerate() {
            let target=&headlight_targets[index];let size=crate::headlights::SHADOW_SIZE as f32;
            let mut vu=view_uniform(lamp.shadow_matrix(),lamp.position,lamp.focal_pixels(),
                [light.lod_pixels,crate::headlights::SHADOW_NEAR,0.0,self.shadow_capacity as f32],[size;4],light.alpha_cut);
            // Explicit perspective-shadow marker. params.z remains zero so
            // projected CLOD error uses perspective distance, not ortho math.
            // 1 is already a split-screen camera: it must retain NO_SHADOW
            // receivers. Reserve 2 for the actual perspective shadow pass.
            vu.extra[1]=2.0;
            gpu.queue.write_buffer(&target.uniform,0,bytemuck::bytes_of(&vu));
            let layer=self.headlights.layer(index as u32);
            self.cull_and_raster(&mut enc,gpu,target,None,&layer,&mut ts,&mut names,"headlight shadow cull");
        }
        let mut canopies = self.canopies.lock().unwrap();
        while canopies.len() < views.len() {
            let next = canopies[0].fork_view(gpu);
            canopies.push(next);
        }
        let mut canopy_updated_levels = Vec::with_capacity(views.len());
        let mut canopy_pitch_m = Vec::with_capacity(views.len());
        if !reuse {
            self.cull_and_raster(&mut enc, gpu, &shadow_t, None, &shadow_v, &mut ts, &mut names, "shadow cull");
        }
        let mut histories = self.history.lock().unwrap();
        if histories.len() < views.len() {
            histories.resize_with(views.len(), || None);
        }
        let near_enabled = light.near_shadows > 0.5 && light.shadow_strength > 0.0 && light.sun_lux > 0.0;
        let mut near_shadows = self.near_shadows.lock().unwrap();
        if near_enabled {
            while near_shadows.len() < views.len() {
                near_shadows.push(NearShadow {
                    tex: tex2d("near sun shadow", NEAR_SHADOW_SIZE, NEAR_SHADOW_SIZE, wgpu::TextureFormat::Depth32Float,
                        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING),
                    targets: self.view_targets_capacity(gpu, "near sun view", self.shadow_capacity),
                });
            }
        }
        if let Some(grass) = &self.grass { grass.set_view_count(views.len()); }
        if let Some(grass) = &self.far_grass { grass.set_view_count(views.len()); }
        if let Some(forest) = &self.voxel_forest { forest.set_view_count(views.len()); }
        let mut keep = Vec::new(); // per view: targets and textures, alive until the readback
        for (vi, view) in views.iter().enumerate() {
            let (w, h) = if let Some(backend) = &mut up.backend {
                backend.optimal_size(mode, (view.w, view.h))?
            } else { (view.w, view.h) };
            let cam = &view.cam;
            upscale_views.push(crate::upscale::UpscaleViewStatus { input: [w, h], output: [view.w, view.h] });
            let receivers = [cam.pos, cam.target, ahead(cam)];
            let canopy_times = if canopies[vi].needs_update(&receivers, revision) {
                ["canopy volume", "filter canopy"].map(|name| self.query.as_ref().map(|(qs, _, _)| {
                    names.push(name);
                    (qs, timestamp_pair(&mut ts))
                }))
            } else { [None, None] };
            let canopy = canopies[vi].prepare(gpu, &mut enc, &self.scene_bind,
                self.instances, &receivers, revision, canopy_times);
            canopy_updated_levels.push(canopy.updated_levels);
            canopy_pitch_m.push(canopy.origins.map(|origin| origin[3]));
            let density_view = canopy.view;
            let gi_u = GiUniform {
                vol_origins: canopy.origins,
                vol_anchors: canopy.anchors,
                vol_dims: [canopy.dims[0], canopy.dims[1], canopy.dims[2], (light.gi > 0.0) as u32],
                hf: [self.hf.origin[0], self.hf.origin[1], self.hf.cell.max(1e-3), 0.0],
                hf_dims: [self.hf.w.max(2), self.hf.h.max(2), self.solid_count, 0],
                // Reflectance, conditioned on local sun and sky visibility.
                bounce: [0.12 * 0.85, 0.12, 0.12 * 0.7, light.sky_lux],
                params: [light.contact_occlusion, light.reflection_occlusion, self.sky_limit, 0.0],
            };
            let gi_buf = buffer(gpu, "gi", bytemuck::bytes_of(&gi_u), wgpu::BufferUsages::UNIFORM);
            let mut view_light = lu;
            let near_view = if near_enabled {
                let near = &near_shadows[vi];
                let near_vp = crate::stability::stable_shadow(cam.target, light.sun_dir, NEAR_SHADOW_EXTENT, NEAR_SHADOW_SIZE);
                gpu.queue.write_buffer(&near.targets.uniform, 0, bytemuck::bytes_of(&view_uniform(
                    near_vp, cam.target + light.sun_dir * 400.0, NEAR_SHADOW_SIZE as f32 / (2.0 * NEAR_SHADOW_EXTENT),
                    [light.lod_pixels, 1.0, 1.0, self.shadow_capacity as f32], [NEAR_SHADOW_SIZE as f32; 4], light.alpha_cut)));
                let texture_view = near.tex.create_view(&Default::default());
                self.cull_and_raster(&mut enc, gpu, &near.targets, None, &texture_view, &mut ts, &mut names, "near shadow cull");
                view_light.near_shadow_view_proj = near_vp.to_cols_array_2d();
                view_light.near_shadow[2] = 1.0;
                texture_view
            } else {
                // Valid but unused binding when the feature is disabled.
                cache.as_ref().unwrap().tex.create_view(&Default::default())
            };
            let light_buf = buffer(gpu, "view light", bytemuck::bytes_of(&view_light), wgpu::BufferUsages::UNIFORM);
            let vis = tex2d("vis", w, h, wgpu::TextureFormat::R32Uint, rt);
            let depth = tex2d("depth", w, h, wgpu::TextureFormat::Depth32Float, rt | wgpu::TextureUsages::COPY_SRC);
            let hdr = tex2d("hdr", w, h, wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT);
            let (vis_v, depth_v, hdr_v) = (vis.create_view(&Default::default()), depth.create_view(&Default::default()), hdr.create_view(&Default::default()));
            // Reversed-Z infinite perspective.
            let view_m = Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y);
            let proj = Mat4::perspective_infinite_reverse_rh(cam.fov_y, view.w as f32 / view.h as f32, cam.near);
            let mut jittered = proj;
            jittered.z_axis.x += jx * 2.0 / w as f32;
            jittered.z_axis.y += jy * 2.0 / h as f32;
            let main = self.view_targets(gpu, "view", vi + 1);
            // Preserve geometry error in output pixels even at lower shading resolution.
            let px_scale = proj.y_axis.y * 0.5 * view.h as f32;
            let vp = jittered * view_m;
            let mut main_uniform = view_uniform(vp, cam.pos, px_scale, [light.lod_pixels, cam.near, 0.0, MAX_VISIBLE as f32], [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32], cut);
            main_uniform.extra[1] = if views.len() > 1 { 1.0 } else { 0.0 };
            main_uniform.extra[2] = if dlss_on { h as f32 / view.h as f32 } else { 0.0 };
            main_uniform.extra[3] = light.grass_reference;
            let grass_band = self.grass_detail.for_views(views.len());
            main_uniform.grass = [grass_band.fade_start, grass_band.fade_end, 0.0, 0.0];
            main_uniform.fog_cull = camera_fog_cull(light);
            gpu.queue.write_buffer(&main.uniform, 0, bytemuck::bytes_of(&main_uniform));
            // TAA history for this view (new, or reset when the size changes).
            let hist = &mut histories[vi];
            let fresh = !hist.as_ref().is_some_and(|hh| hh.size == (w, h));
            if fresh && taa_on {
                let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
                *hist = Some(History { size: (w, h), tex: [tex2d("taa a", w, h, wgpu::TextureFormat::Rgba16Float, usage), tex2d("taa b", w, h, wgpu::TextureFormat::Rgba16Float, usage)], cur: 0, prev_view_proj: proj * view_m });
            }
            let taa_pass = if taa_on {
                let hh = hist.as_mut().unwrap();
                let (src, dst) = (hh.tex[hh.cur].create_view(&Default::default()), hh.tex[1 - hh.cur].create_view(&Default::default()));
                let u = TaaUniform {
                    inv_view_proj: vp.inverse().to_cols_array_2d(),
                    prev_view_proj: hh.prev_view_proj.to_cols_array_2d(),
                    size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
                    params: [exposure, light.taa_blend, fresh as u32 as f32, light.taa_gamma],
                };
                hh.prev_view_proj = proj * view_m;
                hh.cur = 1 - hh.cur;
                Some((src, dst, buffer(gpu, "taa", bytemuck::bytes_of(&u), wgpu::BufferUsages::UNIFORM)))
            } else {
                None
            };
            let tone_buf = buffer(gpu, "tone", bytemuck::bytes_of(&ToneUniform { exposure, contrast: light.contrast, saturation: light.saturation, sharpen: if taa_on { light.sharpen } else { 0.0 }, offset: [view.x, view.y], pad2: [0, 0] }), wgpu::BufferUsages::UNIFORM);
            let shade_bind = bind(
                gpu,
                &self.shade_layout,
                &[
                    (0, main.uniform.as_entire_binding()),
                    (1, main.visible.as_entire_binding()),
                    (2, light_buf.as_entire_binding()),
                    (3, wgpu::BindingResource::TextureView(&vis_v)),
                    (4, wgpu::BindingResource::TextureView(&depth_v)),
                    (5, wgpu::BindingResource::TextureView(&shadow_v)),
                    (6, wgpu::BindingResource::Sampler(&self.shadow_samp)),
                    (7, wgpu::BindingResource::TextureView(&self.sky_view)),
                    (8, wgpu::BindingResource::TextureView(&hdr_v)),
                    (10, wgpu::BindingResource::TextureView(&self.heightfield)),
                    (11, gi_buf.as_entire_binding()),
                    (12, self.solid_occluders.as_entire_binding()),
                    (13, wgpu::BindingResource::TextureView(&density_view)),
                    (14, wgpu::BindingResource::Sampler(&self.density_sampler)),
                    (15, wgpu::BindingResource::TextureView(&near_view)),
                ],
            );
            let resolved = taa_pass.as_ref().map(|(_, dst, _)| dst).unwrap_or(&hdr_v);
            let tone_bind = bind(gpu, &self.tone_layout, &[(0, wgpu::BindingResource::TextureView(resolved)), (1, wgpu::BindingResource::TextureView(&ldr_v)), (2, tone_buf.as_entire_binding())]);
            let taa_bind = taa_pass.as_ref().map(|(src, dst, u)| {
                bind(
                    gpu,
                    &self.taa_layout,
                    &[
                        (0, wgpu::BindingResource::TextureView(&hdr_v)),
                        (1, wgpu::BindingResource::TextureView(&depth_v)),
                        (2, wgpu::BindingResource::TextureView(src)),
                        (3, wgpu::BindingResource::TextureView(dst)),
                        (4, u.as_entire_binding()),
                        (5, wgpu::BindingResource::Sampler(&self.clamp_samp)),
                    ],
                )
            });
            self.cull_and_raster(&mut enc, gpu, &main, Some(&vis_v), &depth_v, &mut ts, &mut names, "cull");
            let mut passes: Vec<(&'static str, &wgpu::ComputePipeline, Vec<&wgpu::BindGroup>)> = vec![("shade", &self.shade, vec![&self.scene_bind, &shade_bind])];
            if let Some(b) = &taa_bind {
                passes.push(("taa", &self.taa, vec![b]));
            }
            if !dlss_on { passes.push(("tonemap", &self.tone, vec![&tone_bind])); }
            let mut far_motion_layer = None;
            for (label, pipe, bindings) in passes {
                let t = self.query.as_ref().map(|(qs, _, _)| {
                    let i = timestamp_pair(&mut ts);
                    (qs, i)
                });
                if t.is_some() {
                    names.push(label);
                }
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(label),
                    timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
                });
                pass.set_pipeline(pipe);
                for (k, b) in bindings.iter().enumerate() {
                    pass.set_bind_group(k as u32, *b, &[]);
                }
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                drop(pass);
                if label == "shade" {
                    let volume_view=(self.far_grass.is_some() || self.voxel_forest.is_some()).then(||
                        crate::volume_view::VolumeViewInputs {
                            camera:&main.uniform, light:&light_buf, opaque_depth:&depth_v,
                            sun_shadow:&shadow_v, shadow_sampler:&self.shadow_samp,
                            sky:&self.sky_view, heightfield:&self.heightfield, gi:&gi_buf,
                            canopy:&density_view, canopy_sampler:&self.density_sampler,
                            near_shadow:&near_view,
                        }.bind(&gpu.device,&self.volume_view_layout));
                    let far_grass = self.far_grass.as_ref().map(|grass| {
                        let time = self.query.as_ref().map(|(qs,_,_)| {
                            let i=timestamp_pair(&mut ts); names.push("far grass volume"); (qs,i)
                        });
                        grass.render(gpu,&mut enc,vi,(w,h),&self.scene_bind,
                            volume_view.as_ref().unwrap(),views.len()>1,time)
                    });
                    if let Some(forest) = &self.voxel_forest {
                        let times = ["voxel tiles", "voxel integrate", "voxel composite"].map(|name|self.query.as_ref().map(|(qs,_,_)|{
                            let i=timestamp_pair(&mut ts); names.push(name); (qs,i)
                        }));
                        forest.render(gpu,&mut enc,vi,(w,h),&self.scene_bind,
                            volume_view.as_ref().unwrap(),&hdr_v,times);
                    }
                    if let Some(grass) = &self.grass {
                        let transition_times = self.query.as_ref().map(|(qs, _, _)| {
                            let i = timestamp_pair(&mut ts);
                            names.push("grass copy + nearest");
                            wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }
                        });
                        let composite_times = self.query.as_ref().map(|(qs, _, _)| {
                            let i = timestamp_pair(&mut ts);
                            names.push("grass composite");
                            wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }
                        });
                        grass.render(gpu, &mut enc, vi, (w, h), &self.scene_bind, &[
                            (0, main.uniform.as_entire_binding()), (2, light_buf.as_entire_binding()),
                            (5, wgpu::BindingResource::TextureView(&shadow_v)),
                            (6, wgpu::BindingResource::Sampler(&self.shadow_samp)),
                            (7, wgpu::BindingResource::TextureView(&self.sky_view)),
                            (10, wgpu::BindingResource::TextureView(&self.heightfield)),
                            (11, gi_buf.as_entire_binding()), (12, self.solid_occluders.as_entire_binding()),
                            (13, wgpu::BindingResource::TextureView(&density_view)),
                            (14, wgpu::BindingResource::Sampler(&self.density_sampler)),
                            (15, wgpu::BindingResource::TextureView(&near_view)),
                        ], &hdr_v, &depth, far_grass.as_ref(), transition_times, composite_times, dlss_on, light.grass_reference);
                    }
                    if let Some(transparency) = &self.transparency {
                        let t = self.query.as_ref().map(|(qs, _, _)| { let i = timestamp_pair(&mut ts); (qs, i) });
                        if t.is_some() { names.push("transparency"); }
                        transparency.render(gpu, &mut enc, vi, cam, &self.scene_bind, &[
                            (0, main.uniform.as_entire_binding()), (2, light_buf.as_entire_binding()),
                            (5, wgpu::BindingResource::TextureView(&shadow_v)),
                            (6, wgpu::BindingResource::Sampler(&self.shadow_samp)),
                            (7, wgpu::BindingResource::TextureView(&self.sky_view)),
                            (10, wgpu::BindingResource::TextureView(&self.heightfield)),
                            (11, gi_buf.as_entire_binding()), (12, self.solid_occluders.as_entire_binding()),
                            (13, wgpu::BindingResource::TextureView(&density_view)),
                            (14, wgpu::BindingResource::Sampler(&self.density_sampler)),
                            (15, wgpu::BindingResource::TextureView(&near_view)),
                        ], &hdr_v, &depth_v,
                            t.map(|(qs, i)| wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }), dlss_on);
                    }
                    far_motion_layer = far_grass;
                }
            }
            if dlss_on {
                let grass_motion = self.grass.as_ref().and_then(|g| g.motion_layer(vi));
                let glass_motion = self.transparency.as_ref().and_then(|t| t.motion_layer(vi));
                let motion_time = self.query.as_ref().map(|(qs, _, _)| {
                    names.push("motion vectors"); (qs, timestamp_pair(&mut ts))
                });
                let motion = up.motion.as_ref().unwrap().render(gpu, &mut enc, vi,
                    crate::motion::MotionView { key: vi as u64, size: (w, h),
                        inverse_jittered_view_proj: vp.inverse(), view_proj: proj * view_m,
                        camera: cam.pos, reset: layout_changed, split: views.len() > 1,
                        grass_detail: self.grass_detail },
                    crate::motion::MotionInputs { opaque_depth: &depth_v, visibility: &vis_v,
                        visible: &main.visible, grass: grass_motion.as_ref(), transparency: glass_motion.as_ref(),
                        far_grass: far_motion_layer.as_ref().map(|layer| (&layer.color, &layer.distance)) }, motion_time)?;
                let dlss_time = self.query.as_ref().map(|(qs, _, _)| {
                    names.push("dlss"); (qs, timestamp_pair(&mut ts))
                });
                if let Some((qs, i)) = dlss_time { enc.write_timestamp(qs, i); }
                let result = up.backend.as_mut().unwrap().evaluate(gpu, &mut enc, &mut segments, vi, mode,
                    (w, h), (view.w, view.h), crate::dlss::Inputs {
                        color: &hdr, depth: &motion.depth, motion: &motion.vectors, bias: Some(&motion.bias),
                        // Projection Z offsets move geometry by (-jx,+jy) pixels.
                        jitter: [-jx, jy], reset: motion.reset, exposure,
                        frame_time_ms: f32::from_bits(self.frame_time_ms.load(std::sync::atomic::Ordering::Relaxed)),
                    })?;
                if let Some((qs, i)) = dlss_time { enc.write_timestamp(qs, i + 1); }
                let result_view = result.create_view(&Default::default());
                let tone_bind = bind(gpu, &self.tone_layout, &[
                    (0, wgpu::BindingResource::TextureView(&result_view)),
                    (1, wgpu::BindingResource::TextureView(&ldr_v)), (2, tone_buf.as_entire_binding())]);
                let t = self.query.as_ref().map(|(qs, _, _)| {
                    names.push("tonemap"); (qs, timestamp_pair(&mut ts))
                });
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tonemap DLSS output"),
                    timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites {
                        query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
                pass.set_pipeline(&self.tone);
                pass.set_bind_group(0, &tone_bind, &[]);
                pass.dispatch_workgroups(view.w.div_ceil(8), view.h.div_ceil(8), 1);
            }
            keep.push((main, vis, depth, hdr));
        }
        if let Some(motion) = &up.motion { motion.finish_frame(&mut enc); }
        up.layout = view_layout;
        up.status = crate::upscale::UpscaleStatus { mode, views: upscale_views };
        let upscale_status = up.status.clone();
        drop(up);
        drop(histories);
        drop(canopies);
        if readback==Readback::None {
            segments.push(enc.finish());
            gpu.queue.submit(segments);
            return Ok((ldr, Vec::new(), FrameStats::default()));
        }
        let near_count = if near_enabled { views.len() } else { 0 };
        let counters = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16 * (views.len() as u64 + 1 + near_count as u64 + headlights.len()as u64), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_buffer_to_buffer(&shadow_t.counter, 0, &counters, 0, 16);
        for (k, (main, ..)) in keep.iter().enumerate() {
            enc.copy_buffer_to_buffer(&main.counter, 0, &counters, 16 * (k as u64 + 1), 16);
        }
        for (k, near) in near_shadows.iter().take(near_count).enumerate() {
            enc.copy_buffer_to_buffer(&near.targets.counter, 0, &counters, 16 * (views.len() as u64 + 1 + k as u64), 16);
        }
        for (k,target) in headlight_targets.iter().take(headlights.len()).enumerate() {
            enc.copy_buffer_to_buffer(&target.counter,0,&counters,16*(views.len()+1+near_count+k)as u64,16);
        }
        if let Some((qs, resolve, read)) = &self.query {
            enc.resolve_query_set(qs, 0..ts, resolve, 0);
            enc.copy_buffer_to_buffer(resolve, 0, read, 0, 8 * ts as u64);
        }
        segments.push(enc.finish());
        gpu.queue.submit(segments);
        let pixels = if readback==Readback::ImageAndMetrics { gpu.read_rgba8(&ldr) } else { Vec::new() };
        let mut stats = FrameStats {
            upscale: upscale_status,
            cluster_buckets: self.cluster_buckets,
            early_grass_cull: self.early_grass_cull,
            canopy_updated_levels,
            canopy_pitch_m,
            ..Default::default()
        };
        if !picks.is_empty() {
            stats.picks.resize(picks.len(), None);
            for (view, (main, vis, ..)) in views.iter().zip(&keep) {
                let requested: Vec<_> = picks.iter().enumerate().filter_map(|(i, &(x,y))|
                    view.local_pixel(x,y).map(|(x,y)| (i,
                        ((x as u64 * vis.width() as u64) / view.w as u64) as u32,
                        ((y as u64 * vis.height() as u64) / view.h as u64) as u32))).collect();
                if requested.is_empty() { continue; }
                let ids = gpu.read_rgba8(vis);
                let vis_list = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: MAX_VISIBLE as u64 * 8, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                let mut pick_encoder = gpu.device.create_command_encoder(&Default::default());
                pick_encoder.copy_buffer_to_buffer(&main.visible, 0, &vis_list, 0, MAX_VISIBLE as u64 * 8);
                gpu.queue.submit([pick_encoder.finish()]);
                vis_list.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                gpu.wait();
                {
                    let mapped = vis_list.slice(..).get_mapped_range().expect("visible list");
                    let list: &[u32] = bytemuck::cast_slice(&mapped);
                    for (i,x,y) in requested {
                        let k = ((y * vis.width() + x) * 4) as usize;
                        let id = u32::from_le_bytes(ids[k..k + 4].try_into().unwrap());
                        if id != u32::MAX {
                            stats.picks[i] = Some((list[(id >> 7) as usize * 2], list[(id >> 7) as usize * 2 + 1], id & 127));
                        }
                    }
                }
                vis_list.unmap();
            }
        }
        counters.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        if let Some((_, _, read)) = &self.query {
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }
        gpu.wait();
        {
            let c = counters.slice(..).get_mapped_range().expect("counters");
            let c: &[u32] = bytemuck::cast_slice(&c);
            let sc = cache.as_mut().unwrap();
            if !reuse {
                (sc.clusters, sc.triangles) = (c[0], c[1]);
                sc.submitted_vertices = c[2];
            }
            stats.shadow_clusters = sc.clusters;
            stats.shadow_triangles = sc.triangles;
            stats.shadow_submitted_vertices = if reuse { 0 } else { sc.submitted_vertices as u64 };
            for k in 0..views.len() {
                assert!(c[4 * (k + 1)] <= MAX_VISIBLE, "main visible-cluster capacity exceeded");
                stats.clusters += c[4 * (k + 1)];
                stats.triangles += c[4 * (k + 1) + 1];
                stats.submitted_vertices += c[4 * (k + 1) + 2] as u64;
                stats.early_grass_rejects += c[4 * (k + 1) + 3] as u64;
            }
            if let Some(transparency) = &self.transparency {
                stats.triangles += transparency.triangle_count() * views.len() as u32;
            }
            for k in 0..near_count {
                let offset = 4 * (views.len() + 1 + k);
                assert!(c[offset] <= self.shadow_capacity, "near shadow visible-cluster capacity exceeded");
                stats.near_shadow_clusters += c[offset];
                stats.near_shadow_triangles += c[offset + 1];
                stats.shadow_submitted_vertices += c[offset + 2] as u64;
            }
            stats.shadow_clusters += stats.near_shadow_clusters;
            for k in 0..headlights.len() {
                let offset=4*(views.len()+1+near_count+k);
                assert!(c[offset]<=self.shadow_capacity,"headlight shadow visible-cluster capacity exceeded");
                stats.shadow_clusters+=c[offset];stats.shadow_triangles+=c[offset+1];
                stats.shadow_submitted_vertices+=c[offset+2]as u64;
            }
            stats.shadow_triangles += stats.near_shadow_triangles;
        }
        if let Some((_, _, read)) = &self.query {
            {
                let t = read.slice(..).get_mapped_range().expect("timestamps");
                let t: &[u64] = bytemuck::cast_slice(&t);
                let period = gpu.queue.get_timestamp_period() as f64;
                // Same-named passes of different views add up.
                for (k, name) in names.iter().enumerate() {
                    let ms = t[2 * k + 1].saturating_sub(t[2 * k]) as f64 * period / 1e6;
                    match stats.gpu_ms.iter_mut().find(|(n, _)| n == name) {
                        Some(e) => e.1 += ms,
                        None => stats.gpu_ms.push((name, ms)),
                    }
                }
            }
            read.unmap();
        }
        Ok((ldr, pixels, stats))
    }
}

/// A camera and where its picture goes in the output image.
#[derive(Clone, Copy, Debug)]
pub struct ViewRect {
    pub cam: Camera,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Godot's split-screen layout for `n` views in a `w` x `h` image: one,
/// side by side for two, a 2x2 grid for three or four, 4 px apart.
pub fn split_rects(n: usize, w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> {
    let gap = ((h as f32 * 4.0 / 1080.0).round() as u32).max(1).min(w.saturating_sub(2)).min(h.saturating_sub(2));
    match n {
        0 | 1 => vec![(0, 0, w, h)],
        2 => {
            let hh = (h - gap) / 2;
            vec![(0, 0, w, hh), (0, hh + gap, w, h - hh - gap)]
        }
        _ => {
            let (hw, hh) = ((w - gap) / 2, (h - gap) / 2);
            vec![(0, 0, hw, hh), (hw + gap, 0, w-hw-gap, hh), (0, hh + gap, hw, h-hh-gap), (hw + gap, hh + gap, w-hw-gap, h-hh-gap)].into_iter().take(n.min(4)).collect()
        }
    }
}

impl ViewRect {
    fn local_pixel(&self, x:u32, y:u32) -> Option<(u32,u32)> {
        let local=(x.checked_sub(self.x)?,y.checked_sub(self.y)?);
        (local.0<self.w && local.1<self.h).then_some(local)
    }
}

#[cfg(test)]
mod sky_tests {
    use super::sky_sh;
    use glam::Vec3;
    fn irradiance(sh:&[[f32;4];9],n:Vec3)->Vec3 {
        // Shader convention; expected values below are independent analytic
        // integrals of a constant sphere or a uniformly lit hemisphere.
        let s=|i:usize|Vec3::new(sh[i][0],sh[i][1],sh[i][2]);
        0.429043*s(8)*(n.x*n.x-n.y*n.y)+0.743125*s(6)*n.z*n.z+0.886227*s(0)-0.247708*s(6)
            +2.0*0.429043*(s(4)*n.x*n.y+s(7)*n.x*n.z+s(5)*n.y*n.z)
            +2.0*0.511664*(s(3)*n.x+s(1)*n.y+s(2)*n.z)
    }
    fn directions()->Vec<Vec3> {
        let mut result=vec![Vec3::X,-Vec3::X,Vec3::Y,-Vec3::Y,Vec3::Z,-Vec3::Z];
        for i in 0..128 {
            let y=1.0-2.0*(i as f32+0.5)/128.0;let angle=i as f32*2.3999631;
            let radius=(1.0-y*y).sqrt();result.push(Vec3::new(radius*angle.cos(),y,radius*angle.sin()));
        }
        result
    }
    #[test]
    fn constant_small_sky_is_pi_radiance_for_every_normal() {
        let source=[0.5,0.6,2.0,1.0];let expected=Vec3::new(0.5,0.6,1.2);
        for (w,h) in [(1,1),(2,1),(1,2),(3,2),(7,5),(16,8),(31,17),(32,16)] {
            for rotation in [-2.7f32,0.0,1.9] {
                let (sh,horizon)=sky_sh(w,h,&vec![source;(w*h)as usize],rotation,1.2);
                assert!((horizon-expected).abs().max_element()<1e-6,"{w}x{h} horizon");
                for n in directions() {
                    let e=irradiance(&sh,n);
                    assert!((e-expected*std::f32::consts::PI).abs().max_element()<8e-6,
                        "{w}x{h} rotation={rotation} n={n:?} irradiance={e:?}");
                }
                assert!(sh[1..].iter().all(|c|c[..3].iter().all(|v|v.abs()<1e-5)));
            }
        }
    }
    #[test]
    fn small_hemisphere_sky_keeps_orientation_and_finite_texel_extent() {
        let lit=[1.0,0.5,0.25,1.0];let dark=[0.0,0.0,0.0,1.0];let color=Vec3::new(1.0,0.5,0.25);
        for rotation in [-2.0f32,0.0,0.7,2.9] {
            let (vertical,horizon)=sky_sh(1,2,&[lit,dark],rotation,100.0);
            assert!((horizon-color*0.5).abs().max_element()<1e-6);
            let (horizontal,_)=sky_sh(2,1,&[lit,dark],rotation,100.0);
            let normal=Vec3::new(-rotation.cos(),0.0,rotation.sin());
            for n in directions() {
                for (sh,axis) in [(&vertical,Vec3::Y),(&horizontal,normal)] {
                    let expected=color*(std::f32::consts::FRAC_PI_2*(1.0+n.dot(axis)));
                    assert!((irradiance(sh,n)-expected).abs().max_element()<8e-6,
                        "rotation={rotation} normal={n:?} axis={axis:?}");
                }
            }
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::{split_rects, timestamp_pair, LightUniform, NEAR_SHADOW_EXTENT, NEAR_SHADOW_SIZE, TIMESTAMP_CAPACITY};
    #[test]
    fn camera_fog_cull_has_exact_linear_cutoff_and_no_exponential_approximation() {
        let mut light = super::Lighting::default();
        light.set("fog_culling=0").unwrap();
        assert_eq!(super::camera_fog_cull(&light),[0.0;4]);
        light.set("fog_culling=1").unwrap();
        assert!(light.set("fog_culling=0.5").is_err());
        for (start, end) in [(140.0f32, 750.0f32), (10.0, 10.0), (20.0, 2.0), (0.0, 0.0)] {
            light.fog_start=start;light.fog_end=end;light.fog_density=0.0;
            let config=super::camera_fog_cull(&light);
            let full=start+(end-start).max(1.0);
            assert_eq!(config[1],1.0); assert!(config[0] > full && config[0] < full+0.02);
            // Raising exponential density never moves the hard distance.
            light.fog_density=1000.0;
            assert_eq!(super::camera_fog_cull(&light),config);
        }
        // Infinite linear distance is the exponential-only case; its tail
        // never reaches exactly zero and must not invent a clipping radius.
        light.fog_start=0.0;light.fog_end=f32::INFINITY;
        assert_eq!(super::camera_fog_cull(&light),[0.0;4]);
        light.fog_end=750.0;
        for debug in [0.1,1.0,2.0,7.0,f32::NAN] {
            light.debug=debug;assert_eq!(super::camera_fog_cull(&light),[0.0;4]);
        }
        light.debug=0.0;
        for invalid in [-1.0,f32::NAN,f32::INFINITY] {
            light.fog_density=invalid;assert_eq!(super::camera_fog_cull(&light),[0.0;4]);
        }
    }

    #[test]
    fn finite_aerosol_layer_never_uses_legacy_radial_fog_cutoff() {
        let mut light=super::Lighting::default();
        for height in [220.0,1200.0] {for density in [0.0,0.0012,0.011] {
            light.fog_height_m=height;light.fog_density=density;
            light.fog_start=20.0;light.fog_end=190.0;
            assert_eq!(super::camera_fog_cull(&light),[0.0;4],"finite-height extinction is not a fully opaque sphere");
        }}
        assert!(light.set("fog_height_m=-1").is_err());
        assert!(light.set("fog_phase_g=1").is_err());
        light.set("fog_height_m=0").unwrap();
        assert_eq!(super::camera_fog_cull(&light)[1],1.0,"legacy opaque fog remains available");
    }
    #[test]
    fn camera_fog_cull_preserves_wind_bounds_and_independent_split_views() {
        use glam::Vec3;
        let light=super::Lighting::default();let config=super::camera_fog_cull(&light);
        let hidden=|camera:Vec3,center:Vec3,radius:f32| camera.distance(center)>config[0]+radius;
        // Expanded wind sphere reaches into visible air, although its root
        // and its resting bound are both completely behind the mist.
        let center=Vec3::X*(config[0]+12.0);
        assert!(hidden(Vec3::ZERO,center,10.0));
        assert!(!hidden(Vec3::ZERO,center,10.0+3.0));
        let cameras=[Vec3::ZERO,center-Vec3::X*100.0,center+Vec3::Z*1200.0,center+Vec3::Y*50.0];
        assert_eq!(cameras.map(|camera|hidden(camera,center,13.0)),[false,false,true,false]);
        for camera in cameras { for radius in [0.01,2.0,13.0,40.0] {
            if !hidden(camera,center,radius) {continue;}
            for y in -8..=8 { for x in 0..32 {
                let up=y as f32/8.0;let angle=x as f32*std::f32::consts::TAU/32.0;
                let ring=(1.0-up*up).sqrt();
                let p=center+Vec3::new(ring*angle.cos(),up,ring*angle.sin())*radius;
                let linear=((p.distance(camera)-light.fog_start)/(light.fog_end-light.fog_start).max(1.0)).clamp(0.0,1.0);
                assert_eq!(linear,1.0,"culled sphere contained a not-fully-fogged point");
            }}
        }}
    }
    #[test]
    fn explicit_fog_view_layout_leaves_every_shadow_view_disabled() {
        use glam::{Mat4,Vec3};
        assert_eq!(std::mem::size_of::<super::ViewUniform>(),320);
        assert_eq!(std::mem::offset_of!(super::ViewUniform,fog_cull),304);
        for orthographic in [0.0,1.0] {
            let view=super::view_uniform(Mat4::IDENTITY,Vec3::ZERO,1.0,[1.0,0.1,orthographic,100.0],[1.0;4],0.5);
            assert_eq!(view.fog_cull,[0.0;4]);
        }
        // All consumers share the appended field; no existing grass/alpha
        // parameter slot was repurposed for a camera-only optimisation.
        for source in [include_str!("shaders/cull.wgsl"),include_str!("shaders/raster.wgsl"),include_str!("shaders/shade.wgsl")] {
            // The full composed shaders are separately validated. Here the
            // declaration itself is checked without their imported helpers.
            let view_source=source.split("struct View {").nth(1).unwrap().split("};").next().unwrap();
            let isolated=format!("struct View {{{view_source}}};");
            let module=naga::front::wgsl::parse_str(&isolated).unwrap();
            let view=module.types.iter().find(|(_,t)|t.name.as_deref()==Some("View")).unwrap().1;
            if let naga::TypeInner::Struct {members,span}= &view.inner {
                assert_eq!(*span,320);assert_eq!(members.last().unwrap().offset,304);
            } else {panic!("View is not a struct");}
        }
    }
    #[test]
    fn bucket_indirect_encoding_preserves_every_original_triangle_id() {
        let limits = [8u32, 32, 64, 128];
        // Includes every boundary, empty lists and capacity-truncated lists.
        let source: Vec<u32> = (1..=128).rev().chain([6, 8, 9, 32, 33, 64, 65, 128]).collect();
        for capacity in [0usize, 1, 7, 64, source.len()] {
            let visible = &source[..capacity];
            let mut buckets = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for (slot, &triangles) in visible.iter().enumerate() {
                buckets[limits.iter().position(|&n| n >= triangles).unwrap()].push(slot as u32);
            }
            // Scatter order is deliberately reversed: original visibility IDs
            // must survive arbitrary compute scheduling and bucket draw order.
            for b in &mut buckets { b.reverse(); }
            let map: Vec<_> = buckets.iter().flatten().copied().collect();
            let mut first = 0;
            let mut actual = Vec::new();
            let mut submitted = 0;
            for bucket in 0..4usize {
                let first_vertex = bucket as u32 * 384;
                for ii in 0..buckets[bucket].len() {
                    for vertex in 0..limits[bucket] * 3 {
                        let vi = first_vertex + vertex;
                        assert_eq!((vi / 384) as usize, bucket);
                        let slot = map[first + ii];
                        let tri = (vi % 384) / 3;
                        if tri < visible[slot as usize] && vertex % 3 == 0 {
                            actual.push((slot << 7) | tri);
                        }
                    }
                }
                submitted += limits[bucket] as usize * 3 * buckets[bucket].len();
                first += buckets[bucket].len();
            }
            let mut expected: Vec<_> = visible.iter().enumerate().flat_map(|(slot, &n)|
                (0..n).map(move |tri| ((slot as u32) << 7) | tri)).collect();
            actual.sort_unstable(); expected.sort_unstable();
            assert_eq!(actual, expected);
            assert!(submitted <= visible.len() * 384);
            assert_eq!(map.len(), visible.len());
        }
        assert!((super::MAX_VISIBLE as u64) * 384 <= u32::MAX as u64);
        assert!((super::MAX_VISIBLE as u64) * 384 * 4 > u32::MAX as u64);
    }

    #[test]
    fn cull_and_raster_specialize_all_performance_switch_combinations() {
        let cull = super::cull_source();
        let raster = crate::grass::raster_source();
        for (src, entries) in [(&cull, vec![(naga::ShaderStage::Compute, "cull"),
            (naga::ShaderStage::Compute, "finalize"), (naga::ShaderStage::Compute, "scatter")]),
            (&raster, vec![(naga::ShaderStage::Vertex, "vs")])] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            for buckets in [0.0, 1.0] {
                for early in [0.0, 1.0] {
                    let mut constants = naga::back::PipelineConstants::default();
                    constants.insert("CLUSTER_BUCKETS".into(), buckets);
                    if entries.len() > 1 { constants.insert("EARLY_GRASS_CULL".into(), early); }
                    for &entry in &entries {
                        naga::back::pipeline_constants::process_overrides(&module, &info, Some(entry), &constants)
                            .unwrap_or_else(|e| panic!("{entry:?}, buckets={buckets}, early={early}: {e}"));
                    }
                }
            }
        }
    }
    #[test]
    fn godot_split_layout_uses_one_4k_output() {
        assert_eq!(split_rects(2,3840,2160),vec![(0,0,3840,1076),(0,1084,3840,1076)]);
        assert_eq!(split_rects(4,3840,2160),vec![(0,0,1916,1076),(1924,0,1916,1076),(0,1084,1916,1076),(1924,1084,1916,1076)]);
        let odd=split_rects(4,1921,1081);
        assert_eq!(odd[3].0+odd[3].2,1921);
        assert_eq!(odd[3].1+odd[3].3,1081);
        assert!(odd.iter().all(|r|r.2>0&&r.3>0&&r.0+r.2<=1921&&r.1+r.3<=1081));
    }

    #[test]
    fn four_views_with_near_maps_voxels_grass_transparency_and_taa_fit_timestamp_budget() {
        // Wind + branch poses + baked actors + shared far map: 5 passes.
        // Each view: canopy/filter, near cull/raster,
        // main cull/raster, shade, voxel tiles/integrate/composite,
        // far grass, grass transition/composite, transparency, TAA, tonemap: 16 intervals.
        let mut next = 0;
        for _ in 0..(5 + 4 * 16) { timestamp_pair(&mut next); }
        assert_eq!(next, 138);
        assert!(next <= TIMESTAMP_CAPACITY);
        // DLSS adds motion + reconstruction and replaces TAA. This deliberately
        // also budgets experimental voxels even though that combination rejects.
        next = 0;
        for _ in 0..(5 + 4 * 17) { timestamp_pair(&mut next); }
        assert_eq!(next, 146);
        assert!(next <= TIMESTAMP_CAPACITY);
        next = TIMESTAMP_CAPACITY - 2;
        assert_eq!(timestamp_pair(&mut next), TIMESTAMP_CAPACITY - 2);
        assert_eq!(next, TIMESTAMP_CAPACITY);
    }

    #[test]
    #[should_panic(expected = "timestamp query capacity")]
    fn timestamp_overflow_fails_before_encoding_an_invalid_gpu_query() {
        let mut next = TIMESTAMP_CAPACITY;
        timestamp_pair(&mut next);
    }

    #[test]
    fn near_light_uniform_and_projection_cover_each_separated_car() {
        use glam::Vec3;
        assert_eq!(std::mem::size_of::<LightUniform>(), 384);
        assert_eq!(std::mem::offset_of!(LightUniform, fog_scattering), 144);
        assert_eq!(std::mem::offset_of!(LightUniform, near_shadow_view_proj), 304);
        assert_eq!(std::mem::offset_of!(LightUniform, near_shadow), 368);
        let sun = Vec3::new(0.4, 0.8, -0.3).normalize();
        for target in [Vec3::ZERO, Vec3::new(2500.0, 120.0, -4000.0)] {
            let matrix = crate::stability::stable_shadow(target, sun, NEAR_SHADOW_EXTENT, NEAR_SHADOW_SIZE);
            for point in [target, target+Vec3::X*2.5, target-Vec3::Z*2.5, target+Vec3::Y*2.0] {
                let projected = matrix.project_point3(point);
                assert!(projected.x.abs().max(projected.y.abs()) < 0.8, "car must remain fully inside near detail");
                assert!(projected.z > 0.0 && projected.z < 1.0);
            }
        }
    }
}

/// Only the finite linear term reaches exact coverage. Never infer a hard
/// clipping distance from exponential fog's asymptotic transmittance.
fn camera_fog_cull(light: &Lighting) -> [f32; 4] {
    // Physical extinction is asymptotic and height dependent. A camera-wide
    // radial cutoff would erase still-visible geometry, including clear air.
    if light.fog_height_m > 0.0 || !light.fog_culling || light.debug != 0.0 || ![light.fog_start, light.fog_end, light.fog_density]
        .iter().all(|v| v.is_finite() && *v >= 0.0) {
        return [0.0; 4];
    }
    // Matches shared distance_fog_amount, including end <= start. A small
    // outward allowance prevents float rounding from rejecting a tangent
    // sphere while its nearest point still has nonzero scene contribution.
    let full = light.fog_start + (light.fog_end - light.fog_start).max(1.0);
    let safe = full + full.max(1.0) * 1e-5 + 0.001;
    if safe.is_finite() { [safe, 1.0, 0.0, 0.0] } else { [0.0; 4] }
}

fn view_uniform(vp: Mat4, cam: Vec3, px_scale: f32, params: [f32; 4], size: [f32; 4], alpha_cut: f32) -> ViewUniform {
    // Frustum planes from the view-projection (Gribb-Hartmann), normalised.
    // Reversed/infinite depth: skip the far plane (keep it permissive).
    let m = vp.transpose();
    let rows = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
    let mut planes = [[0.0f32; 4]; 6];
    let raw: [Vec4; 6] = [rows[3] + rows[0], rows[3] - rows[0], rows[3] + rows[1], rows[3] - rows[1], rows[2], rows[3] - rows[2]];
    for (k, p) in raw.iter().enumerate() {
        let n = p.truncate().length();
        planes[k] = if n > 1e-6 { (*p / n).to_array() } else { [0.0, 0.0, 0.0, 1.0] };
    }
    // For reversed-Z the "far" plane (index 5) is z <= w at the near plane;
    // for orthographic shadows both depth planes are real. Planes stay as
    // computed: both cases keep geometry in [0, 1] depth.
    ViewUniform {
        view_proj: vp.to_cols_array_2d(),
        inv_view_proj: vp.inverse().to_cols_array_2d(),
        cam: [cam.x, cam.y, cam.z, px_scale],
        planes,
        params,
        size,
        extra: [alpha_cut, 0.0, 0.0, 0.0],
        grass: [40.0, 60.0, 0.0, 0.0],
        fog_cull: [0.0; 4],
    }
}
