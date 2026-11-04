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

#[path = "resource_cache.rs"]
pub(crate) mod resource_cache;
pub use resource_cache::CacheStats as ResourceCacheStats;

#[derive(Default)]
struct FrameResources {
    targets: [resource_cache::TextureViews<1>; 3],
    near_shadow: resource_cache::TextureViews<1>,
    history: resource_cache::TextureViews<2>,
    result: resource_cache::TextureViews<2>,
    shade: resource_cache::BindGroups<2>,
    taa: resource_cache::BindGroups<2>,
    tone: resource_cache::BindGroups<2>,
    volume: resource_cache::BindGroups<1>,
}
impl FrameResources {
    fn stats(&self) -> ResourceCacheStats {
        let mut stats = ResourceCacheStats::default();
        for target in &self.targets { stats.add(target.stats()); }
        for other in [self.near_shadow.stats(), self.history.stats(), self.result.stats(),
            self.shade.stats(), self.taa.stats(), self.tone.stats(), self.volume.stats()] { stats.add(other); }
        stats
    }
}

#[path = "texture_mips.rs"]
pub(crate) mod texture_mips;

pub const MAX_VISIBLE: u32 = 4 << 20;
/// Albedo array resolution: the Poly Haven sets are 2K.
/// Side of every albedo array layer; `screen::Screen` draws layers at this size.
pub const TEXTURE_SIZE: u32 = if cfg!(target_os = "android") { 512 } else { 2048 };
/// Normal map array resolution.
const NORMAL_SIZE: u32 = if cfg!(target_os = "android") { 512 } else { 1024 };
pub(crate) const SHADOW_SIZE: u32 = if cfg!(target_os = "android") { 2048 } else { 4096 };
const NEAR_SHADOW_SIZE: u32 = if cfg!(target_os = "android") { 1024 } else { 2048 };
const NEAR_SHADOW_EXTENT: f32 = 16.0;
/// The near shadow's cluster LOD error, in multiples of the camera's
/// (KERN_NEAR_SHADOW_LOD, for A/B runs). Its 1.6 cm texels pick full detail:
/// a full spruce assembly casts for ~2.5 ms (low2, 8 trees: 18 ms; 2x: 9.5,
/// 4x: 1.5). But coarser spray cards lose coverage, and the crowns' shadows
/// on the ground thin out visibly already at 2x, so 1 until their LOD keeps it.
/// The finest voxel cell a sun map splats (texel_m only picks the level):
/// at their texels (near 1.6 cm, shared ~4.4 cm) every tree would take its
/// 10 cm level, ~80k cells each. The near map takes 0.5 m cells (the 40 cm
/// level), the shared map, which only shades past it, 1 m. terrain_fly low2,
/// 3 full spruces: near sun splats 2.26 -> 0.63 ms, their raster 2.0 -> 0.44;
/// Sideways' 32k recipe trees, shared map: splats 30 -> 2.4 ms, raster 62 ->
/// 5.5. Crown shadows only get slightly softer. KERN_SUN_VOXEL_M sets both.
const NEAR_SUN_VOXEL_M: f32 = 0.5;
const SHARED_SUN_VOXEL_M: f32 = 1.0;
fn sun_voxel_m(texel: f32, cell: f32) -> f32 {
    static CELL: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| std::env::var("KERN_SUN_VOXEL_M").ok().and_then(|v| v.parse().ok())).unwrap_or(cell).max(texel)
}
fn near_sun_source() -> bool {
    static SOURCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SOURCE.get_or_init(|| std::env::var("KERN_NEAR_SUN_SOURCE").as_deref() == Ok("1"))
}
fn near_shadow_lod() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| std::env::var("KERN_NEAR_SHADOW_LOD").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0))
}
const TIMESTAMP_CAPACITY: u32 = 512; // includes per-view aggregate depth/material/illumination

/// Keep the grass rejection in culling identical to its opaque raster path.
pub(crate) fn cull_source() -> String {
    format!("{}\n{}\n{}\n{}\n{}", concat!(include_str!("shaders/virtual_member.wgsl"), "\n", include_str!("shaders/tree_history_key.wgsl"), "\n", include_str!("shaders/grass_common.wgsl")), concat!(include_str!("shaders/cull.wgsl"), "\n", include_str!("shaders/shadow_damage_query.wgsl")), include_str!("shaders/cluster_traversal.wgsl"), crate::receiver_shadow_demand::cull_source(),crate::source_replacement::query_source())
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

/// Allocation lifetime is independent of the validity of the rendered depth.
struct ShadowCache {
    content: ShadowContent,
    tex: wgpu::Texture,
    clusters: u32,
    triangles: u32,
    submitted_vertices: u32,
}

#[derive(Clone, Copy, Debug)]
struct ShadowContent {
    view_proj: Mat4,
    alpha_cut: f32,
    lod_pixels: f32,
    valid: bool,
}

impl ShadowContent {
    fn pending(view_proj: Mat4, alpha_cut: f32, lod_pixels: f32) -> Self {
        Self { view_proj, alpha_cut, lod_pixels, valid: false }
    }
    fn reusable(&self, view_proj: Mat4, alpha_cut: f32, lod_pixels: f32) -> bool {
        self.valid && self.view_proj.abs_diff_eq(view_proj, 1e-6)
            && self.alpha_cut.to_bits() == alpha_cut.to_bits()
            && self.lod_pixels.to_bits() == lod_pixels.to_bits()
    }
    fn invalidate(&mut self) { self.valid = false; }
    fn submitted(&mut self) { self.valid = true; }
}

/// Keep diagnostic shadow views operational even when direct sunlight is zero.
/// Nonfinite illuminance is not silently treated as a dark scene.
fn sun_visibility_required(fast_path: bool, sun_lux: f32, debug: f32) -> bool {
    !fast_path || !sun_lux.is_finite() || !(sun_lux <= 0.0) || debug != 0.0
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
    /// Bounded per-view temporal guard. A source/volume or selected-mip change
    /// resets that whole view initially, not a claim of mature streaming history.
    foliage_selection: Vec<Vec<u32>>,
}

fn update_foliage_history(previous:&mut Vec<u32>,current:&[u32])->bool {
    if previous.as_slice()==current {return false;}
    previous.clear();previous.extend_from_slice(current);true
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
    /// Camera: full-fog distance, cull enabled, terrain sun visibility, diagnostic exclusion mask.
    /// Orthographic sun views instead reserve z/w for caster LOD diagnostics.
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
    /// Stellar peak radiance, zenith optical depth, zero-sun fast path, stable seed.
    night_stars: [f32; 4],
    /// x: seconds for animated procedural materials (Lighting::time).
    anim: [f32; 4],
    /// Fog integration sequence phase and temporal accumulation enabled.
    fog_sampling: [f32; 4],
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
    bloom_strength:f32,
    screen:u32,
    vignette: f32,
    grain: f32,
    aberration: f32,
    time: f32,
    sun: [f32; 4], // view UV, angular footprint in UV-y, exposed flare strength
    sun_tint: [f32; 4],
}

/// Project a direction (w=0), so camera translation cannot move the distant sun.
fn solar_projection(cam: &Camera, view: Mat4, aspect: f32, light: &Lighting, visibility: f32) -> [f32; 4] {
    if !light.sun_disc || light.lens_flare <= 0.0 || light.sun_lux <= 0.0 || light.sun_dir.y <= 0.0 { return [0.0;4]; }
    let direction = light.sun_dir.normalize_or_zero();
    let projected = Mat4::perspective_infinite_reverse_rh(cam.fov_y, aspect, cam.near) * view * direction.extend(0.0);
    if projected.w <= 0.01 { return [0.0;4]; }
    let uv = projected.truncate() / projected.w;
    let radius = 0.00465 / (cam.fov_y * 0.5).tan() * 0.5 / projected.w;
    let strength = light.lens_flare * visibility * (light.sun_lux / (1.2 * 2f32.powf(light.ev100))).min(4.0);
    [uv.x * 0.5 + 0.5, 0.5 - uv.y * 0.5, radius, strength]
}

/// Where the camera is and what it sees.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
    pub near: f32,
}

/// A finite orthonormal view even when the craft points straight up/down.
fn camera_view(cam: Camera, requested_up: Option<Vec3>) -> Mat4 {
    let direction=(cam.target-cam.pos).try_normalize().unwrap_or(-Vec3::Z);
    let mut up=requested_up.filter(|u|u.is_finite()).and_then(Vec3::try_normalize).unwrap_or(Vec3::Y);
    if direction.cross(up).length_squared()<1e-8 {
        up=if direction.y.abs()<0.9 {Vec3::Y}else{Vec3::Z};
    }
    let target=if (cam.target-cam.pos).length_squared()>1e-12 {cam.target}else{cam.pos+direction};
    Mat4::look_at_rh(cam.pos,target,up)
}

#[cfg(test)]
mod flight_camera_tests {
    use super::*;
    #[test]
    fn view_roll_preserves_position_and_forward_and_handles_poles() {
        let cam=Camera{pos:Vec3::new(1.0,2.0,3.0),target:Vec3::new(1.0,2.0,4.0),fov_y:1.0,near:0.1};
        for up in [Vec3::Y,-Vec3::Y,Vec3::X,-Vec3::X] {
            let view=camera_view(cam,Some(up));
            assert!(view.transform_point3(cam.pos).length()<1e-5);
            assert!((view.transform_point3(cam.target)+Vec3::Z).length()<1e-5);
            assert!((view.transform_vector3(up)-Vec3::Y).length()<1e-5);
        }
        for direction in [Vec3::Y,-Vec3::Y,Vec3::ZERO] {
            let view=camera_view(Camera{target:cam.pos+direction,..cam},Some(Vec3::Y));
            assert!(view.is_finite());assert!((view.determinant()-1.0).abs()<1e-5);
        }
    }
}

/// Light and look for a frame, in physical units.
#[derive(Clone, Debug)]
pub struct Lighting {
    /// Unit vector towards the sun.
    pub sun_dir: Vec3,
    /// Illuminance of the sun on a surface facing it (lux).
    pub sun_lux: f32,
    pub sun_color: Vec3,
    /// Visible analytic solar disc; enabled by Warp Sky, off for legacy HDR skies.
    pub sun_disc: bool,
    /// Angular radius of the visible sun, in radians.
    pub sun_radius: f32,
    /// Optical glare/ghost strength, computed and occluded on the GPU per view.
    pub lens_flare: f32,
    /// Artistic highlight bloom multiplier over the debug/quality control.
    pub bloom_boost: f32,
    /// Minimum blur rounds for this game's look (1..=6).
    pub bloom_rounds: u32,
    /// Maximum exposed threshold for this game's highlights.
    pub bloom_threshold: f32,
    pub shadow_strength: f32,
    /// Horizontal irradiance of the sky without the sun (lux).
    pub sky_lux: f32,
    /// Background stars only: peak radiance, zenith optical depth, reserved,
    /// numeric 24-bit seed. Atmosphere::NightSky supplies this automatically.
    /// A zero peak disables stars for legacy HDR environments.
    /// Seconds on the clock that animated procedural materials (flowing
    /// lava) follow. Keep it running with the game, not the wall clock.
    pub time: f32,
    pub night_stars: [f32; 4],
    /// Exposure value at ISO 100 (sunny 16: about 15).
    pub ev100: f32,
    /// Exposure compensation for emitted light (EMISSIVE and LAVA materials,
    /// car lamps), 0 (off: physical) to 1: emission is shown as if the frame
    /// were exposed at `emissive_ev100`, so a glow tuned for one exposure
    /// keeps its colour when the exposure drops for night instead of
    /// clipping to white. Light that local lights cast is unchanged.
    pub emissive_compensation: f32,
    /// The exposure (EV100) emission is shown at with full compensation.
    pub emissive_ev100: f32,
    /// Blast fire and smoke (`fx`) are unlit and drawn as white under the
    /// sun and sky. They are drawn at least as bright as this fraction of
    /// the exposure's white (1.2 * 2^ev100), so a night lit by lamps does
    /// not turn them black. 0 (default): sun and sky only.
    pub fx_white_floor: f32,
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
    /// Low-cost diffuse room return in lux, attenuated by material/contact AO.
    pub indirect_fill_lux: f32,
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
    /// Darkening towards each view's corners, 0 (off) to 1.
    pub vignette: f32,
    /// Film grain, 0 (off) to 1 (strong); it moves with `time`.
    pub grain: f32,
    /// Chromatic aberration towards the corners, in pixels at the corner (0 off).
    pub aberration: f32,
    /// Opt-in in-scattering from local lights with a scatter weight
    /// (`Renderer::set_local_lights_scattering`); `None` adds no pass.
    pub volumetric: Option<crate::volumetric::Volumetric>,
}

impl Lighting {
    /// The factor on emitted radiance: 2^((ev100 - emissive_ev100) * compensation).
    pub fn emissive_scale(&self) -> f32 {
        let s = 2f32.powf((self.ev100 - self.emissive_ev100) * self.emissive_compensation.clamp(0.0, 1.0));
        if s.is_finite() { s } else { 1.0 }
    }

    /// Set a field by name (`sun_lux=50000`): tune the look without a rebuild.
    pub fn set(&mut self, kv: &str) -> Result<(), String> {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got {kv}"))?;
        let f: f32 = v.parse().map_err(|_| format!("{k}: not a number: {v}"))?;
        match k {
            "sun_lux" => self.sun_lux = f,
            "shadow_strength" => self.shadow_strength = f,
            "sky_lux" => self.sky_lux = f,
            "ev100" => self.ev100 = f,
            "sun_disc" if f == 0.0 || f == 1.0 => self.sun_disc = f > 0.0,
            "bloom_boost" if (0.0..=32.0).contains(&f) => self.bloom_boost = f,
            "bloom_rounds" if (1.0..=6.0).contains(&f) && f.fract() == 0.0 => self.bloom_rounds = f as u32,
            "bloom_threshold" if (0.0..=16.0).contains(&f) => self.bloom_threshold = f,
            "lens_flare" if (0.0..=2.0).contains(&f) => self.lens_flare = f,
            "emissive_compensation" if (0.0..=1.0).contains(&f) => self.emissive_compensation = f,
            "emissive_ev100" if f.is_finite() => self.emissive_ev100 = f,
            "fx_white_floor" if f.is_finite() && f >= 0.0 => self.fx_white_floor = f,
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
            "indirect_fill_lux" if f.is_finite() && f >= 0.0 => self.indirect_fill_lux = f,
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
            "vignette" if (0.0..=1.0).contains(&f) => self.vignette = f,
            "grain" if (0.0..=1.0).contains(&f) => self.grain = f,
            "aberration" if (0.0..=16.0).contains(&f) => self.aberration = f,
            // `volumetric=0` turns it off, `volumetric=1` on with defaults.
            "volumetric" if f == 0.0 || f == 1.0 => self.volumetric = (f == 1.0).then(|| self.volumetric.unwrap_or_default()),
            k if k.starts_with("vol_") => self.volumetric.get_or_insert_with(Default::default).set(k, f)?,
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
            sun_disc: false,
            sun_radius: 0.00465,
            lens_flare: 0.0,
            bloom_boost: 1.0,
            bloom_rounds: 1,
            bloom_threshold: crate::bloom::THRESHOLD,
            shadow_strength: 1.0,
            sky_lux: 18_000.0,
            night_stars: [0.0; 4],
            time: 0.0,
            ev100: 14.0,
            emissive_compensation: 0.0,
            emissive_ev100: 14.0,
            fx_white_floor: 0.0,
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
            indirect_fill_lux: 0.0,
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
            vignette: 0.0,
            grain: 0.0,
            aberration: 0.0,
            volumetric: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct FrameStats {
    /// Experimental exact static/dynamic partition of the shared sun map.
    pub static_sun_cache: bool,
    pub static_sun_redrawn: bool,
    pub static_sun_reused: bool,
    pub static_sun_seed_bytes: u64,
    /// Incremental retained GPU resources: depth, owner lists and static
    /// cull/raster targets. Excludes the already-existing final sun target.
    pub static_sun_gpu_bytes: u64,
    pub static_sun_casters: u32,
    pub dynamic_sun_casters: u32,
    /// Retained static source selection, whether newly drawn or cached.
    pub static_sun_clusters: u32,
    pub static_sun_triangles: u32,
    /// Actual submitted vertices this frame; zero on a cache hit.
    pub static_sun_submitted_vertices: u64,
    pub dynamic_sun_clusters: u32,
    pub dynamic_sun_triangles: u32,
    pub sun_cache_guard_m: f32,
    pub sun_cache_guard_recentered: bool,
    /// Wind freeze radius in use (0 = off) and whether the frozen-tree static
    /// casters were re-pinned around the cameras this frame.
    pub wind_freeze_m: f32,
    pub wind_freeze_repinned: bool,
    pub shared_sun_extent_m: f32,
    pub shared_sun_texel_m: f32,
    /// One shared static source-alpha map, regardless of player count.
    pub world_terrain_visibility: bool,
    pub world_terrain_bytes: u64,
    /// CPU construction / submission only; not a GPU build-time measurement.
    pub world_terrain_setup_cpu_ms: f64,
    pub far_sun_cache_updated: bool,
    pub far_sun_extent_m: f32,
    pub far_sun_texel_m: f32,
    pub far_sun_lod_pixels: f32,
    pub far_sun_clusters: u32,
    pub far_sun_triangles: u32,
    pub far_sun_submitted_vertices: u64,
    pub foliage_sun_requested_pages: u32,
    pub foliage_sun_resident_pages: u32,
    pub foliage_sun_missing_pages: u32,
    pub foliage_sun_updated_pages: u32,
    /// CPU indexing/request work; excludes raster and GPU waits.
    pub foliage_sun_prepare_cpu_ms: f64,
    pub foliage_sun_index_rebuilt: bool,
    pub foliage_sun_revision: u32,
    pub foliage_sun_cache_bytes: u64,
    pub foliage_sun_clusters: u32,
    pub foliage_sun_triangles: u32,
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
    /// Optional compact per-view owner list before unchanged cluster selection.
    pub instance_broadphase: bool,
    /// Current-pose spatial groups before camera instance/cluster selection.
    pub instance_groups: bool,
    pub sun_instance_groups: bool,
    pub headlight_instance_groups: bool,
    /// Conservative shared mesh-cluster BVH; independent of owner grouping.
    pub cluster_traversal: bool,
    /// Current-frame two-phase tree occlusion, never applied to shadows.
    pub tree_occlusion: bool,
    pub shadow_clusters: u32,
    pub shadow_triangles: u32,
    /// Subset of total shadow work spent on the per-view near maps.
    pub near_shadow_clusters: u32,
    pub near_shadow_triangles: u32,
    /// Per view: bit mask of rebuilt forest levels; zero is a cache hit.
    pub canopy_updated_levels: Vec<u32>,
    /// Per view: world voxel pitch of the three forest levels, in metres.
    pub canopy_pitch_m: Vec<[f32; 3]>,
    /// Per view: visible fraction of the sun above the camera's terrain horizon.
    /// Applies only to direct solar fog scattering, not diffuse sky illumination.
    pub camera_sun_visibility: Vec<f32>,
    /// GPU time per pass in ms, when the GPU has timestamps.
    pub gpu_ms: Vec<(&'static str, f64)>,
    /// For each requested composite-output pixel: (instance, cluster, triangle)
    /// or None for sky, a split-screen gap, or a point outside all views.
    pub picks: Vec<Option<(u32, u32, u32)>>,
}

#[derive(Clone)]
struct ViewTargets {
    uniform: wgpu::Buffer,
    foliage_selection: wgpu::Buffer,
    visible: wgpu::Buffer,
    counter: wgpu::Buffer,
    args: wgpu::Buffer,
    draw_map: wgpu::Buffer,
    spatial_owners: wgpu::Buffer,
    traversal_counter: wgpu::Buffer,
    instance_dispatch: Option<wgpu::Buffer>,
    cull_bind: wgpu::BindGroup,
    raster_bind: wgpu::BindGroup,
}

/// Identity and source geometry of one colour view in the last submitted frame.
/// This diagnostic is independent of internal target-pool slot numbering.
#[derive(Debug)]
pub struct VisibleClusterDiagnostic {
    pub frame_id: u64,
    pub render_sequence: u64,
    pub view_index: usize,
    pub view_count: u32,
    pub viewport: [u32; 4],
    pub input_size: [u32; 2],
    pub output_size: [u32; 2],
    pub camera: Camera,
    pub pairs: Vec<[u32; 2]>,
    pub categories: Option<[crate::geometry_diagnostics::Counts;5]>,
    /// Actual submitted per-volume [enabled, maximum mip, reserved, reserved]
    /// selection, read from this view's GPU buffer. Empty without foliage bindings.
    pub foliage_selection: Vec<[u32; 4]>,
}

struct ColorDiagnosticView {
    counter: wgpu::Buffer,
    traversal_counter: wgpu::Buffer,
    visible: wgpu::Buffer,
    view: ViewRect,
    input_size: [u32; 2],
    visibility: Option<wgpu::Texture>,
    foliage_selection: wgpu::Buffer,
}
#[derive(Default)]
struct ColorDiagnostics {
    context: Option<crate::gpu_profile::Context>,
    views: Vec<ColorDiagnosticView>,
    owners: Option<std::sync::Arc<Vec<crate::scene::GeometryCategory>>>,
}
impl ColorDiagnostics {
    fn submitted(&mut self, context: crate::gpu_profile::Context, views: &[ViewRect],
        rendered: &[(ViewTargets, wgpu::Texture, wgpu::Texture, wgpu::Texture)],
        owners: Option<std::sync::Arc<Vec<crate::scene::GeometryCategory>>>) {
        debug_assert_eq!(views.len(), rendered.len());
        self.views.clear();
        self.views.extend(views.iter().zip(rendered).map(|(&view,(target,visibility,..))| ColorDiagnosticView {
            counter: target.counter.clone(), traversal_counter:target.traversal_counter.clone(), visible: target.visible.clone(), view,
            input_size: [visibility.width(),visibility.height()],
            visibility: owners.is_some().then(||visibility.clone()),
            foliage_selection: target.foliage_selection.clone(),
        }));
        self.context = Some(context);
        self.owners = owners;
    }
}

/// Explicit bounded native-only experiment. Never selected by a game setting.
/// The fixture must enclose all source grass, read publication status, and fail
/// if any brick is invalid. This is not a world streaming/fallback policy.
struct MeadowBrickFixture {
    field: crate::meadow_brick_field::MeadowBrickField,
    renderer: crate::vegetation_brick_render::VegetationBrickRenderer,
    lod_pixels: f32,
    generation: std::sync::atomic::AtomicU32,
    frozen: bool,
}

pub struct Renderer {
    // Optional per-view roll for flight games. Empty preserves world-up cameras.
    camera_up_axes: std::sync::Mutex<Vec<Vec3>>,
    camera_projections: std::sync::Mutex<Vec<Mat4>>,
    resource_device: wgpu::Device,
    albedo_array: wgpu::Texture,
    live_screens: Option<std::sync::Mutex<crate::screen_feed::LiveScreens>>,
    aux_buffer: wgpu::Buffer,
    dents: Option<(usize, usize)>,
    dfao: Option<(usize, crate::dfao::Grid)>,
    irradiance_volume: Option<(usize, crate::irradiance_volume::Grid)>,
    sun_diagnostics: std::sync::Mutex<crate::shadow_diagnostics::State>,
    headlight_diagnostics: std::sync::Mutex<crate::shadow_diagnostics::State>,
    geometry_diagnostics: crate::geometry_diagnostics::State,
    debug_options: std::sync::Mutex<crate::debug_settings::DebugRenderOptions>,
    upscaling: std::sync::Mutex<Upscaling>,
    motion_blur: std::sync::Mutex<Option<crate::motion_blur::Resources>>,
    frame_time_ms: std::sync::atomic::AtomicU32,
    scene_bind: wgpu::BindGroup,
    cull_scene_bind: wgpu::BindGroup,
    far_cull_scene_bind: wgpu::BindGroup,
    foliage_cull_scene_bind: wgpu::BindGroup,
    voxel_scene_bind: wgpu::BindGroup,
    voxel_scene_layout: wgpu::BindGroupLayout,
    meadow_brick_fixture: Option<MeadowBrickFixture>,
    shadow_scene_bind: wgpu::BindGroup,
    far_sun_scene_bind: wgpu::BindGroup,
    far_sun: crate::far_sun::Resources,
    foliage_sun_pages: crate::foliage_sun_pages::Resources,
    async_shadow_validation: Option<std::sync::Mutex<crate::shadow_validation::State>>,
    foliage_sun_scene_bind: wgpu::BindGroup,
    foliage_page_targets: std::sync::Mutex<Vec<(u32, ViewTargets)>>,
    world_visibility: crate::world_visibility::Resources,
    far_sun_targets: std::sync::Mutex<Option<ViewTargets>>,
    headlights: crate::headlights::Resources,
    tracks: crate::tracks::Resources,
    exhaust: crate::exhaust::ExhaustRenderer,
    wheel_particles: crate::wheel_particles::WheelParticles,
    headlight_state: std::sync::Mutex<Vec<crate::headlights::SpotLight>>,
    headlight_exclusions: std::sync::Mutex<Vec<(u32,u32)>>,
    local_light_state: std::sync::Mutex<Vec<crate::headlights::LocalLight>>,
    spot_falloff: std::sync::Mutex<Vec<f32>>,
    spot_softness: std::sync::Mutex<f32>,
    lighting_focus: std::sync::Mutex<Vec<Vec3>>,
    diagnostic_sun_focus: std::sync::Mutex<Vec<Vec3>>,
    legacy_light_lookahead: bool,
    headlight_targets: std::sync::Mutex<Vec<ViewTargets>>,
    instance_buffer: wgpu::Buffer,
    wind: crate::wind::WindRenderer,
    part_wind: Option<crate::part_wind::PartWindRenderer>,
    gpu_forest: Option<(crate::foliage_runtime::Runtime,u32)>,
    part_wind_lod: bool,
    aggregate_lod: Option<crate::aggregate_lod::Selector>,
    aggregate_lod_enabled: std::sync::atomic::AtomicBool,
    baked_animations: Option<crate::baked_animation::AnimationRenderer>,
    skinning: Option<crate::skinning::SkinningRenderer>,
    debris: Option<crate::debris::DebrisRenderer>,
    procedural_terrain: Option<crate::procedural_terrain::TerrainRenderer>,
    crowd: Option<crate::crowd::CrowdRenderer>,
    fx: crate::fx::FxSlot,
    volumetric: crate::volumetric::VolumetricSlot,
    sprites: crate::sprites::SpriteSlot,
    transparency: Option<crate::transparency::Transparency>,
    grass: Option<crate::grass::GrassRenderer>,
    meadow: Option<crate::meadow::MeadowRenderer>,
    meadow_substrate_enabled: bool,
    meadow_substrate_buffer: wgpu::Buffer,
    meadow_substrate_settings: crate::meadow_substrate::Uniform,
    grass_detail: crate::vegetation_detail::DistanceProfile,
    far_grass: Option<crate::far_grass::FarGrassRenderer>,
    voxel_forest: Option<crate::voxel_forest::VoxelRenderer>,
    foliage_world: Option<crate::foliage_world_render::WorldRenderer>,
    source_replacement: Option<crate::source_replacement::Selector>,
    source_readiness: Option<wgpu::Buffer>,
    foliage_world_enabled: std::sync::atomic::AtomicBool,
    foliage_world_valid: std::sync::atomic::AtomicBool,
    foliage_world_error_pixels: f32,
    foliage_representations: Vec<crate::foliage_representation::Binding>,
    foliage_representation_policy: crate::foliage_representation::Policy,
    foliage_planners: std::sync::Mutex<Vec<crate::foliage_representation::ViewPlanner>>,
    /// Per view: size, visibility, depth and HDR targets reused across frames.
    #[allow(clippy::type_complexity)]
    view_targets: std::sync::Mutex<Vec<Option<((u32, u32), wgpu::Texture, wgpu::Texture, wgpu::Texture)>>>,
    /// Per view: the gi, view light, TAA and tone uniforms, rewritten each frame.
    view_uniforms: std::sync::Mutex<Vec<[Option<wgpu::Buffer>; 5]>>,
    frame_resources: std::sync::Mutex<Vec<FrameResources>>,
    output_view: std::sync::Mutex<resource_cache::TextureViews<1>>,
    shadow_view: std::sync::Mutex<resource_cache::TextureViews<1>>,
    volume_view_layout: wgpu::BindGroupLayout,
    cull: wgpu::ComputePipeline,
    split_sun_cull: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    compact_cull: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    spatial_cull: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    cached_sun_cull: Option<(wgpu::ComputePipeline,wgpu::ComputePipeline)>,
    sun_spatial_cull: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    headlight_spatial_cull: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    instance_groups: crate::cull_groups::Resources,
    receiver_demand: crate::receiver_shadow_demand::Resources,
    meadow_receivers: Option<crate::meadow_shadow_receivers::MeadowReceivers>,
    cluster_traversal: crate::cluster_traversal::Resources,
    finalize: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    prepare_tree_phase: Option<wgpu::ComputePipeline>,
    recover_tree_clusters:Option<wgpu::ComputePipeline>,
    tree_occlusion: crate::tree_occlusion::Resources,
    cluster_buckets: bool,
    simple_lighting: bool,
    sharp_sprites: bool,
    simple_cull: bool,
    small_cluster_vertices:u32,
    minimal_forward: Option<std::sync::Mutex<crate::forward::Forward>>,
    direct_bounds: Option<std::sync::Mutex<crate::direct_draw::Bounds>>,
    raster_large_first: bool,
    early_grass_cull: bool,
    raster: wgpu::RenderPipeline,
    raster_depth: wgpu::RenderPipeline,
    /// Opt-in sun receiver-plane experiment; headlights retain their old bias.
    raster_sun_depth: Option<wgpu::RenderPipeline>,
    raster_cached_sun:Option<wgpu::RenderPipeline>,
    shadow_damage:std::sync::Mutex<Option<crate::shadow_damage::Damage>>,
    legacy_shadow_wind:bool,
    /// Coarse cached source maps must not shift steep leaf casters by metres.
    raster_far_sun_depth: Option<wgpu::RenderPipeline>,
    sun_scene_bounds: Option<std::sync::Mutex<crate::stability::SunSceneBounds>>,
    sun_guard_bounds: Option<std::sync::Mutex<crate::stability::SunSceneBounds>>,
    sun_cache_guard_m: f32,
    sun_cache_anchor: std::sync::Mutex<crate::static_sun::GuardedAnchor>,
    /// `KERN_WIND_FREEZE=start,end`: trees past `end` from every camera stand
    /// still; with the static sun cache and a guard they are cached casters.
    wind_freeze: Option<crate::wind::WindFreeze>,
    /// Camera positions the frozen-tree partition was pinned around.
    wind_freeze_pin: std::sync::Mutex<Option<Vec<Vec3>>>,
    sun_terrain_error: bool,
    sun_surface_source: bool,
    camera_sun_occlusion: bool,
    shade: wgpu::ComputePipeline,
    /// Sky tiles (off by default, KERN_SKY_TILES=1: slower on an RTX 5070 Ti, shade 1.74 vs 1.56 ms at 4K x4): `shade_classify` writes
    /// the sky of 8x8 tiles without geometry and lists the rest, and `shade`
    /// runs indirectly over the listed tiles only. Same image either way.
    sky_tiles: bool,
    shade_classify: wgpu::ComputePipeline,
    /// Shade binding 18 without rasterized voxels: no voxel in front.
    voxel_depth_fallback: wgpu::TextureView,
    /// Rasterized voxels draw into the scene's visibility buffer and shade
    /// lights them from their surface (binding 20); stand-in without.
    voxel_surface: bool,
    voxel_surface_fallback: wgpu::TextureView,
    /// shade_voxels (KERN_VOXEL_PASS=1): voxel pixels in their own pass.
    voxel_pass: Option<wgpu::ComputePipeline>,
    /// shade for procedural terrain pixels only (SHADE_CLASS 1); `shade`
    /// then lights everything else (2).
    terrain_shade: Option<wgpu::ComputePipeline>,
    /// Constants of the shade pipelines with their diagnostics, and those
    /// pipelines (shade, terrain shade), built the first time
    /// Lighting::debug asks for a component view.
    diagnostic_constants: Vec<(&'static str, f64)>,
    shade_diagnostic: std::sync::OnceLock<(wgpu::ComputePipeline, Option<wgpu::ComputePipeline>)>,
    /// Coarse terrain material (shade.wgsl terrain_coarse; KERN_TERRAIN_COARSE=0
    /// turns it off), its half-resolution target per view, the stand-in
    /// bound while it is written, and its output layout.
    terrain_coarse: Option<wgpu::ComputePipeline>,
    terrain_coarse_targets: std::sync::Mutex<Vec<Option<(wgpu::Texture, wgpu::TextureView)>>>,
    terrain_coarse_fallback: wgpu::TextureView,
    terrain_coarse_layout: wgpu::BindGroupLayout,
    /// Group 2 of the coarse terrain pass, which writes through group 3.
    empty_bind: wgpu::BindGroup,
    /// Bakes each changed canopy level's sky transmittance into the canopy
    /// texture, so shading reads it instead of marching per pixel
    /// (`KERN_SKY_VIS_CACHE=0` keeps the per-pixel march).
    sky_vis_bake: Option<SkyVisBake>,
    /// Per view: tile list (count first) and the indirect dispatch
    /// arguments copied from it (a buffer cannot be storage and indirect in
    /// one dispatch).
    shade_tiles: std::sync::Mutex<Vec<Option<(wgpu::Buffer, wgpu::Buffer)>>>,
    /// Kept to rebuild `shade` when its source changes on disk.
    shade_pl: wgpu::PipelineLayout,
    shade_text: String,
    shader_wind_text: String,
    shader_generation: u64,
    rock_variation: bool,
    shade_features: crate::shade_features::ShadeFeatures,
    cpu_spans: Option<crate::cpu_spans::Collector>,
    /// No foliage instance, forest field or heightfield: GI rays cannot be
    /// attenuated, so sky, reflection and sun-fallback marches are skipped
    /// (exactly 1). Foliage cannot be added later without a rebuild.
    empty_gi_volume: bool,
    car_materials: wgpu::Buffer,
    car_material_count: usize,
    /// The scene's materials, rewritable with [`Renderer::update_materials`].
    materials: wgpu::Buffer,
    material_count: usize,
    tone: wgpu::ComputePipeline,
    bloom: crate::bloom::Bloom,
    shade_layout: wgpu::BindGroupLayout,
    hidden_meadow_fallback: wgpu::Buffer,
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
    async_gpu_profile: std::sync::Mutex<crate::gpu_profile::Profiler>,
    profile_sequence: std::sync::atomic::AtomicU64,
    profile_frame_id: std::sync::atomic::AtomicU64,
    startup_profile: Option<crate::startup_profile::StartupProfile>,
    taa: wgpu::ComputePipeline,
    taa_layout: wgpu::BindGroupLayout,
    clamp_samp: wgpu::Sampler,
    history: std::sync::Mutex<Vec<Option<History>>>,
    frame: std::sync::atomic::AtomicU32,
    shadow_cache: std::sync::Mutex<Option<ShadowCache>>,
    static_sun: Option<std::sync::Mutex<crate::static_sun::StaticSun>>,
    sun_owner_buffer: wgpu::Buffer,
    static_sun_targets: std::sync::Mutex<Option<ViewTargets>>,
    reuse_sun_texture: bool,
    zero_sun_fast_path: bool,
    shadow_extent: std::sync::Mutex<f32>,
    near_shadows: std::sync::Mutex<Vec<NearShadow>>,
    shadow_capacity: u32,
    cluster_views: std::sync::Mutex<Vec<ViewTargets>>,
    color_diagnostics: std::sync::Mutex<ColorDiagnostics>,
}

#[derive(Clone, Copy)]
enum RasterPass { Color, Sun, SunStatic(u32), SunDynamic(u32), FarSun, FoliagePage, Headlight }

fn raster_bucket_order(bucketed:bool,large_first:bool,kind:RasterPass)->&'static [u64] {
    if !bucketed {return &[0];}
    // Size-order experiment, NOT material classification. All indirect
    // records/visible IDs stay unchanged; exact-depth first-writer ties may
    // change in the colour view. Shadow/lamp/page order is never altered.
    if large_first && matches!(kind,RasterPass::Color) {&[3,2,1,0]} else {&[0,1,2,3]}
}

fn resolve_lighting_focus(views:&[ViewRect], requested:&[Vec3])->Result<Vec<Vec3>,String> {
    if !requested.is_empty() && requested.len()!=views.len() {
        return Err(format!("{} lighting receivers for {} views",requested.len(),views.len()));
    }
    let points=if requested.is_empty(){views.iter().map(|v|v.cam.pos).collect()} else {requested.to_vec()};
    if points.iter().any(|p|!p.is_finite()) {return Err("nonfinite lighting receiver".into());}
    Ok(points)
}

fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }
}

fn layout_entries(entries: &[(u32, wgpu::BindingType)]) -> Vec<wgpu::BindGroupLayoutEntry> {
    let all = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
    // Writable bindings are for compute only (vertex stages cannot write).
    let writable = |ty: &wgpu::BindingType| matches!(ty, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, .. } | wgpu::BindingType::StorageTexture { .. });
    entries
        .iter()
        .map(|(b, ty)| wgpu::BindGroupLayoutEntry { binding: *b, visibility: if writable(ty) { wgpu::ShaderStages::COMPUTE } else { all }, ty: *ty, count: None })
        .collect()
}

/// `shade.wgsl` `bake_sky_visibility`: its own group 1 (heightfield, gi,
/// canopy texture and sampler) and group 3 (scratch level, level index).
struct SkyVisBake {
    pipeline: wgpu::ComputePipeline,
    inputs: wgpu::BindGroupLayout,
    output: wgpu::BindGroupLayout,
    levels: [wgpu::Buffer; 3],
}

fn layout(gpu: &Gpu, label: &str, entries: &[(u32, wgpu::BindingType)]) -> wgpu::BindGroupLayout {
    let e = layout_entries(entries);
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
fn texture_array(gpu: &Gpu, label: &'static str, images: &[Option<&image::RgbaImage>], size: u32, format: wgpu::TextureFormat, fill: [u8; 4], cutout_layers: &[bool], profile: &mut crate::startup_profile::Trace) -> wgpu::Texture {
    let enabled=profile.enabled();
    let allocation_start=enabled.then(std::time::Instant::now);
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
        // RENDER_ATTACHMENT: `screen::Screen` redraws live layers on the GPU.
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let allocation_cpu_ms=crate::startup_profile::elapsed(allocation_start);
    let cache=crate::texture_cache::Cache::from_env();
    let chain = |src: Option<&image::RgbaImage>, preserve_alpha: bool, layer:usize| {
        let settings=crate::texture_cache::Settings{size,format,fill,preserve_alpha,linear_cutout_rgb};
        let total=enabled.then(std::time::Instant::now);
        let mut cache_profile=crate::startup_profile::TextureCacheProfile::default();
        let (cached,key)=cache.load(src,settings,enabled,&mut cache_profile);
        let (out,mut timing)=if let Some(out)=cached {
            (out,crate::startup_profile::TextureLayerProfile{layer,source_size:src.map_or([0,0],|s|[s.width(),s.height()]),..Default::default()})
        } else {
            let (out,timing)=texture_mips::build(src,settings,enabled,layer);
            if let Some(key)=key {cache.store(key,settings,&out,enabled,&mut cache_profile);}
            (out,timing)
        };
        timing.output_bytes=out.iter().map(|i|i.as_raw().len()as u64).sum();
        timing.total_cpu_ms=crate::startup_profile::elapsed(total);
        timing.cache=cache_profile;
        (out,timing)
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    // Linear RGBA scratch is 16 bytes/texel, plus resize/frontier storage.
    // Bound transient CPU memory while building large cutout colour arrays.
    let threads = if format == wgpu::TextureFormat::Rgba8UnormSrgb { threads.min(4) } else { threads };
    let build_start=enabled.then(std::time::Instant::now);
    let chains: Vec<(Vec<image::RgbaImage>,crate::startup_profile::TextureLayerProfile)> = std::thread::scope(|sc| {
        let per = images.len().div_ceil(threads).max(1);
        let handles: Vec<_> = images.chunks(per).enumerate().map(|(chunk, part)| sc.spawn(move || part.iter().enumerate().map(|(index, i)| chain(*i, cutout_layers.get(chunk * per + index).copied().unwrap_or(false),chunk*per+index)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let build_wall_ms=crate::startup_profile::elapsed(build_start);
    let upload_start=enabled.then(std::time::Instant::now);
    for (layer, (mips,_)) in chains.iter().enumerate() {
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
    let upload_cpu_ms=crate::startup_profile::elapsed(upload_start);
    let layers=if enabled {chains.iter().map(|(_,p)|p.clone()).collect()}else{Vec::new()};
    let release_start=enabled.then(std::time::Instant::now);
    drop(chains);
    profile.texture(crate::startup_profile::TextureArrayProfile {name:label,size,layers,workers:threads.min(images.len()),
        allocation_cpu_ms,build_wall_ms,upload_cpu_ms,release_cpu_ms:crate::startup_profile::elapsed(release_start)});
    tex
}

/// SH9 projection of an equirectangular sky (radiance in, irradiance
/// coefficients out), leaving out the sun: the sun is its own light.
fn scene_environment(gpu: &Gpu, scene: &Scene, light: &Lighting) -> (wgpu::Texture, [[f32;4];9], Vec3, f32, f32, f32) {
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
    (sky_tex, sky_sh, raw_horizon * sky_scale, sky_scale, sky_limit, rotation)
}

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
    /// Set independent up axes for split-screen cameras (roll/loops/free flight).
    /// Pass an empty slice to restore the legacy world-up convention.
    pub fn set_camera_up_axes(&self, axes: &[Vec3]) {
        let mut stored = self.camera_up_axes.lock().unwrap();
        stored.clear(); stored.extend_from_slice(axes);
    }

    /// Runtime-provided asymmetric, reversed-Z projections, one per view.
    /// XR callers update these together with the predicted eye poses. Empty restores perspective FOV.
    pub fn set_camera_projections(&self, projections: &[Mat4]) {
        *self.camera_projections.lock().unwrap() = projections.to_vec();
    }

    /// The albedo texture array (sRGB, TEXTURE_SIZE square, mipmapped). Live
    /// layers are redrawn through `screen::Screen`; do not resize it.
    pub fn albedo_array(&self) -> &wgpu::Texture { &self.albedo_array }

    /// Update a game-owned range reserved in `Scene::aux` before construction.
    /// The caller owns the range; engine reservations must not be overwritten.
    pub fn update_aux_words(&self, gpu: &Gpu, offset: usize, words: &[u32]) -> Result<(), String> {
        let end = offset.checked_add(words.len()).and_then(|n| n.checked_mul(4)).ok_or("aux range overflow")?;
        if end as u64 > self.aux_buffer.size() { return Err("aux write outside reserved buffer".into()); }
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(words));
        Ok(())
    }

    /// Replace the bullet dents reserved with `Scene::reserve_dents`.
    pub fn set_dents(&self, gpu: &Gpu, dents: &[crate::dents::Dent]) -> Result<(), String> {
        let (offset, capacity) = self.dents.ok_or("the scene reserved no dents")?;
        let words = crate::dents::pack(capacity, dents);
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(&words));
        Ok(())
    }

    /// Replace the 3D distance field reserved with `Scene::reserve_dfao`.
    pub fn set_dfao(&self, gpu: &Gpu, words: &[u32]) -> Result<(), String> {
        let (offset, grid) = self.dfao.ok_or("the scene reserved no dfao field")?;
        if words.len() != grid.words() { return Err("dfao word count differs from the grid".into()); }
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(words));
        Ok(())
    }

    /// Replace the probes reserved with `Scene::reserve_irradiance_volume`
    /// (for example after `irradiance_volume::BoxRoom::bake`).
    pub fn set_irradiance_volume(&self, gpu: &Gpu, probes: &[crate::irradiance_volume::Probe]) -> Result<(), String> {
        let (offset, grid) = self.irradiance_volume.ok_or("the scene reserved no irradiance volume")?;
        let words = crate::irradiance_volume::pack(&grid, probes)?;
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(&words));
        Ok(())
    }

    /// Upload geometry-only ambient visibility into the reserved probe grid.
    pub fn set_sky_occlusion(&self, gpu: &Gpu, probes: &[crate::irradiance_volume::Probe]) -> Result<(), String> {
        let (offset, grid) = self.irradiance_volume.ok_or("no reserved probe volume")?;
        let mut words = crate::irradiance_volume::pack(&grid, probes)?;
        words[7] = 1;
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(&words));
        Ok(())
    }

    /// Upload visibility, one diffuse return, and valid-probe weights.
    pub fn set_sky_diffuse(&self, gpu: &Gpu, probes: &[crate::irradiance_volume::Probe]) -> Result<(), String> {
        let (offset, grid) = self.irradiance_volume.ok_or("no reserved probe volume")?;
        let mut words = crate::irradiance_volume::pack(&grid, probes)?;
        words[7] = 2;
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(&words));
        Ok(())
    }

    /// Upload direct sky visibility, sky return and shadowed sun return.
    pub fn set_sky_sun(&self, gpu: &Gpu, probes: &[crate::irradiance_volume::Probe]) -> Result<(), String> {
        let (offset, grid) = self.irradiance_volume.ok_or("no reserved probe volume")?;
        let mut words = crate::irradiance_volume::pack(&grid, probes)?;
        words[7] = 3;
        if std::env::var("WARP_SKY_PACKED").as_deref()==Ok("1") || std::env::var("WARP_SKY_TRILINEAR").as_deref()==Ok("1") {
            words[7]=if std::env::var("WARP_SKY_TRILINEAR").as_deref()==Ok("1") { 6 } else { 4 };
            for (i,p) in probes.iter().enumerate() {
                for (a,v) in p.axes.iter().enumerate() {
                    let invalid=u32::from(v.z>1.5);
                    let q=|x:f32| (x.clamp(0.0,1.0)*1023.0).round() as u32;
                    words[8+i*18+a*3]=q(v.x)|(q(v.y)<<10)|(q(v.z)<<20)|(invalid<<31);
                }
            }
        }
        gpu.queue.write_buffer(&self.aux_buffer, (offset * 4) as u64, bytemuck::cast_slice(&words));
        Ok(())
    }

    /// Upload probe words prepared off the render thread with irradiance_volume::pack*.
    /// The reserved grid must match; encoding/validation of probes belongs to packing.
    pub fn set_packed_irradiance_volume(&self, gpu:&Gpu, words:&[u32])->Result<(),String> {
        let (offset,grid)=self.irradiance_volume.ok_or("no reserved probe volume")?;
        let header=[grid.origin.x.to_bits(),grid.origin.y.to_bits(),grid.origin.z.to_bits(),grid.cell.to_bits(),grid.dims[0],grid.dims[1],grid.dims[2]];
        if words.len()!=crate::irradiance_volume::HEADER_WORDS+grid.count()*crate::irradiance_volume::PROBE_WORDS
            || words[..7]!=header || words[7]>7 { return Err("packed irradiance grid mismatch".into()); }
        gpu.queue.write_buffer(&self.aux_buffer,(offset*4)as u64,bytemuck::cast_slice(words));
        Ok(())
    }

    /// Combine cached emitter light with the sky/sun cache, without extra bindings.
    pub fn set_sky_sun_emission(&self, gpu: &Gpu, probes: &[crate::irradiance_volume::Probe], emission: &[crate::irradiance_volume::Probe], planes: &[[f32;4]]) -> Result<(),String> {
        let (offset,grid)=self.irradiance_volume.ok_or("no reserved probe volume")?;
        let words=crate::irradiance_volume::pack_sky_emission(&grid,probes,emission,planes)?;
        gpu.queue.write_buffer(&self.aux_buffer,(offset*4) as u64,bytemuck::cast_slice(&words));
        Ok(())
    }

    /// Opt-in packed transport with a nearest-static-surface separation plane.
    pub fn set_sky_sun_separated(&self, gpu:&Gpu, probes:&[crate::irradiance_volume::Probe], planes:&[[f32;4]]) -> Result<(),String> {
        let (offset,grid)=self.irradiance_volume.ok_or("no reserved probe volume")?;
        let mut words=crate::irradiance_volume::pack(&grid,probes)?;
        if planes.len()!=probes.len() || planes.iter().flatten().any(|x|!x.is_finite()) { return Err("invalid separation planes".into()); }
        words[7]=5;
        for (i,p) in probes.iter().enumerate() {
            for (a,v) in p.axes.iter().enumerate() {
                let q=|x:f32| (x.clamp(0.0,1.0)*1023.0).round() as u32;
                words[8+i*18+a*3]=q(v.x)|(q(v.y)<<10)|(q(v.z)<<20)|(u32::from(v.z>1.5)<<31);
            }
            for (j,slot) in [1,2,4,5].into_iter().enumerate() { words[8+i*18+slot]=planes[i][j].to_bits(); }
        }
        gpu.queue.write_buffer(&self.aux_buffer,(offset*4) as u64,bytemuck::cast_slice(&words));
        Ok(())
    }

    pub fn new(gpu: &Gpu, scene: &Scene, light: &Lighting) -> Renderer {
        let mut startup=crate::startup_profile::Trace::new();
        let geometry_diagnostics=crate::geometry_diagnostics::State::new(scene);
        let u = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let forest_assets=crate::foliage_pages::cache::Cache::for_scene(gpu,scene).expect("invalid forest asset cache");
        let source_buffers=forest_assets.as_ref().and_then(|c|c.source.as_ref()).map(|s|s.buffers.clone());
        let vertices = source_buffers.as_ref().map(|s|s[0].clone()).unwrap_or_else(||buffer(gpu, "vertices", if scene.vertices.is_empty() {&[0u8;std::mem::size_of::<crate::cluster::Vertex>()]} else {bytemuck::cast_slice(&scene.vertices)}, u | wgpu::BufferUsages::COPY_SRC));
        let cverts = source_buffers.as_ref().map(|s|s[1].clone()).unwrap_or_else(||buffer(gpu, "cluster verts", bytemuck::cast_slice(&scene.cluster_verts), u));
        let tris = source_buffers.as_ref().map(|s|s[2].clone()).unwrap_or_else(||buffer(gpu, "tris", bytemuck::cast_slice(&scene.tris), u));
        let clusters = source_buffers.as_ref().map(|s|s[3].clone()).unwrap_or_else(||buffer(gpu, "clusters", if scene.clusters.is_empty() {&[0u8;std::mem::size_of::<crate::cluster::GpuCluster>()]} else {bytemuck::cast_slice(&scene.clusters)}, u | wgpu::BufferUsages::COPY_SRC));
        let instances = buffer(gpu, "instances", if scene.instances.is_empty() {&[0u8;std::mem::size_of::<crate::scene::GpuInstance>()]} else {bytemuck::cast_slice(&scene.instances)}, u | wgpu::BufferUsages::COPY_SRC);
        let materials = buffer(gpu, "materials", if scene.materials.is_empty() {&[0u8;std::mem::size_of::<crate::scene::Material>()]} else {bytemuck::cast_slice(&scene.materials)}, u);
        let aux = buffer(gpu, "aux", bytemuck::cast_slice(&scene.aux), u);
        // Never empty: a binding needs at least one element.
        let car_list = if scene.car_materials.is_empty() { vec![crate::scene::GpuCarMaterial::default()] } else { scene.car_materials.clone() };
        let car_materials = buffer(gpu, "car materials", bytemuck::cast_slice(&car_list), u);
        startup.mark("geometry_material_buffers");
        crate::foliage_representation::validate_bindings(scene).expect("invalid foliage representation bindings");
        let voxel_replacement = buffer(gpu, "voxel crown replacement", bytemuck::cast_slice(&crate::voxel_forest::replacement_materials(scene)), u);
        let wind = crate::wind::WindRenderer::new(gpu, &instances, &voxel_replacement, scene.instances.len())
            .expect("scene exceeds bounded wind storage");
        startup.mark("foliage_validation_wind_resources");
        let albedo_src: Vec<Option<&image::RgbaImage>> = scene.textures.iter().map(|t| Some(&t.rgba)).collect();
        // Continuous alpha must average coverage naturally. Applying foliage's
        // threshold-preserving mip scale to glass/decals changes their opacity.
        let mut cutout_layers = vec![true; scene.textures.len()];
        for &material in scene.blended_materials.keys() {
            let layer = scene.materials[material as usize].layer;
            if layer >= 0 { cutout_layers[layer as usize] = false; }
        }
        let albedo = texture_array(gpu, "albedo", &albedo_src, TEXTURE_SIZE, wgpu::TextureFormat::Rgba8UnormSrgb, [255; 4], &cutout_layers, &mut startup);
        let albedo_view = albedo.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        startup.mark("albedo_array");
        // Normal maps, same layers; flat where a texture has none.
        let normal_src: Vec<Option<&image::RgbaImage>> = scene.textures.iter().map(|t| t.normal.as_ref()).collect();
        let normals = texture_array(gpu, "normals", &normal_src, NORMAL_SIZE, wgpu::TextureFormat::Rgba8Unorm, [128, 128, 255, 255], &[], &mut startup);
        let normals_view = normals.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        startup.mark("normal_array");
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
        let scene_bindings = [
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
                (20, wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}),
                (21, wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Sint,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}),
                (22, uniform()),
                (23, wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false}),
                (24, storage(true)),
                (25, uniform()),
                (26, storage(true)),
                (27, uniform()),
                (28, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (29, storage(true)),
            ];
        let mut scene_entries = layout_entries(&scene_bindings);
        for entry in &mut scene_entries {
            // Full-scene replacement IDs are read only by raster vertices;
            // compute culling has its own reduced layout. Leaving this visible
            // to shade would exceed 16 storage buffers after adding contact.
            if entry.binding == 11 { entry.visibility = wgpu::ShaderStages::VERTEX; }
            if entry.binding == 29 { entry.visibility = wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE; }
        }
        let scene_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"), entries: &scene_entries,
        });
        let voxel_scene_bindings: Vec<_> = scene_bindings.iter().copied()
            .filter(|(binding, _)| crate::voxel_forest::SCENE_BINDINGS.contains(binding)).collect();
        let voxel_scene_layout = layout(gpu, "voxel lighting scene", &voxel_scene_bindings);
        startup.mark("scene_samplers_layouts");
        let transparency = crate::transparency::Transparency::new(gpu, scene, &scene_layout);
        startup.mark("transparency_resources_pipelines");
        let grass = crate::grass::GrassRenderer::new(gpu, scene, &scene_layout, &instances, wind.buffer());
        startup.mark("grass_resources_pipelines");
        let meadow = crate::meadow::MeadowRenderer::new(gpu, scene, &scene_layout)
            .expect("invalid procedural meadow scene");
        startup.mark("meadow_resources_pipelines");
        let volume_view_layout=crate::volume_view::create_layout(&gpu.device);
        let far_grass = scene.far_grass.as_ref().map(|field|
            // Far grass uses physical-lighting inputs only. Reuse the reduced
            // volume scene layout rather than unused material/track textures;
            // the full layout would exceed the portable16 sampled-texture cap.
            crate::far_grass::FarGrassRenderer::new(gpu, field, &voxel_scene_layout, &volume_view_layout)
                .expect("invalid far grass field"));
        startup.mark("far_grass_resources_pipelines");
        crate::aggregate_lod::validate_scene(scene).expect("valid aggregate scene");
        let aggregate_lod=scene.aggregate_lod.as_ref().map(|s|crate::aggregate_lod::Selector::new(gpu,s))
            .transpose().expect("invalid aggregate error selection");
        let voxel_forest = crate::voxel_forest::VoxelRenderer::new(gpu, scene, &voxel_scene_layout, &volume_view_layout,forest_assets);
        let gpu_forest=scene.gpu_forest.as_ref().map(|spec| crate::foliage_runtime::Runtime::new(gpu,spec,&instances,
            voxel_forest.as_ref().expect("GPU forest volumes").tree_buffer()).map(|runtime|(runtime,spec.first_owner)))
            .transpose().expect("invalid GPU forest runtime");
        let shadow_damage=scene.gpu_forest.as_ref().map(|spec|crate::shadow_damage::Damage::new(gpu,&instances,voxel_forest.as_ref().unwrap().tree_buffer(),&gpu_forest.as_ref().unwrap().0.choices,
            [scene.instances.len()as u32,(voxel_forest.as_ref().unwrap().tree_buffer().size()/96)as u32,spec.slots,spec.first_owner])).transpose().expect("invalid forest shadow damage cache");
        let legacy_shadow_wind=scene.instances.iter().any(|i|i.sphere[3]>=0. && (i.first_cluster!=0 || i.cluster_count!=0) && i.flags&crate::scene::PART_WIND==0 && i.flags&(crate::scene::FOLIAGE|crate::scene::WIND_GRASS)!=0);
        if let (Some(voxels),Some((forest,_)))=(&voxel_forest,&gpu_forest){voxels.initialize_pages(gpu,forest).expect("invalid voxel cache publication");}
        let foliage_world=scene.foliage_world.as_ref().map(|field|
            crate::foliage_world_render::WorldRenderer::new(gpu,field,scene.instances.len()as u32,
                &voxel_scene_layout,&volume_view_layout,scene.foliage_world_error_pixels))
            .transpose().expect("invalid world foliage provider");
        let replacement_plan=scene.foliage_world.as_ref().map(|field|
            crate::source_replacement::Plan::new(scene,&field.groups)).transpose().expect("invalid whole-owner source replacement");
        let source_replacement=replacement_plan.as_ref().map(|plan|crate::source_replacement::Selector::new(gpu,plan))
            .transpose().expect("invalid source replacement selector");
        let source_readiness=scene.foliage_world.as_ref().map(|field| {
            let ready:Vec<_>=field.groups.iter().map(|group|crate::source_replacement::Readiness {
                generation:group.generation,ready:1,valid:1,frame_generation:0}).collect();
            buffer(gpu,"world foliage immutable readiness",bytemuck::cast_slice(&ready),wgpu::BufferUsages::STORAGE)
        });
        startup.mark("voxel_forest_resources_pipelines");
        let part_wind = if scene.part_wind.is_empty() && scene.part_attachments.is_empty() { None } else {
            Some(crate::part_wind::PartWindRenderer::new(gpu, &scene.part_wind, &scene.part_attachments,
                &scene.instances, &instances,
                scene.voxel_forest.as_ref().zip(voxel_forest.as_ref().map(|f| f.tree_buffer())))
                .expect("invalid procedural vegetation part hierarchy"))
        };
        let baked_animations = scene.baked_animations.as_ref().map(|animation|
            crate::baked_animation::AnimationRenderer::new(gpu, animation, &scene.instances, &instances)
                .expect("invalid baked animation scene"));
        let skinning = if scene.skinning.actors.is_empty() { None } else {
            Some(crate::skinning::SkinningRenderer::new(gpu, &scene.skinning, &scene.vertices, &scene.instances, &vertices, &instances)
                .expect("invalid weighted skinning scene"))
        };
        let debris = scene.debris.map(|spec| crate::debris::DebrisRenderer::new(gpu, spec, &instances));
        let procedural_terrain = scene.procedural_terrain.clone().map(|spec| crate::procedural_terrain::TerrainRenderer::new(gpu, spec, &vertices, &clusters));
        let crowd = scene.crowd.map(|spec| crate::crowd::CrowdRenderer::new(gpu, &spec, &instances));
        startup.mark("part_wind_animation_resources");
        let grass_lighting=crate::grass_lighting::Resources::new(gpu,scene.far_grass.as_ref().zip(far_grass.as_ref()),scene.terrain_visual_offset_m);
        startup.mark("grass_lighting_resources");
        let headlights=crate::headlights::Resources::new(gpu);
        let tracks=crate::tracks::Resources::new(gpu);
        startup.mark("headlight_track_resources");
        let hf = scene.heightfield.clone();
        let (hw, hh, hdata) = if hf.w > 1 && hf.h > 1 { (hf.w, hf.h, hf.heights.clone()) } else { (2, 2, vec![-1e9f32; 4]) };
        // Green holds the baked sun visibility (Scene::terrain_sun), 1 without one.
        let sun_ok = scene.terrain_sun.len() == hdata.len();
        let hdata: Vec<f32> = hdata.iter().enumerate().flat_map(|(i, &h)| [h, if sun_ok { scene.terrain_sun[i] } else { 1.0 }]).collect();
        let hf_tex = gpu.device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("heightfield"),
                size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            Default::default(),
            bytemuck::cast_slice(&hdata),
        );

        let height_view=hf_tex.create_view(&Default::default());
        startup.mark("heightfield_clone_upload");
        let world_visibility=crate::world_visibility::Resources::new(gpu,scene,&height_view)
            .expect("world terrain visibility resources");
        startup.mark("world_terrain_hierarchy");
        let far_sun=crate::far_sun::Resources::new(gpu,scene);
        startup.mark("far_sun_resources");
        let foliage_sun_pages=crate::foliage_sun_pages::Resources::new(gpu,scene).expect("foliage sun pages");
        startup.mark("foliage_sun_page_resources");
        let meadow_substrate_enabled = crate::meadow_substrate::enabled(
            std::env::var("KERN_MEADOW_AGGREGATE").as_deref() == Ok("1"),
            std::env::var("KERN_MEADOW_SUBSTRATE").ok().as_deref(),
        );
        let meadow_contact_ao = std::env::var("KERN_MEADOW_CONTACT_AO").as_deref() == Ok("1");
        let meadow_substrate_settings = crate::meadow_substrate::Uniform::from_field(
            scene.meadow.as_ref(), meadow_substrate_enabled || meadow_contact_ao,
        ).expect("invalid meadow substrate material").with_features(meadow_substrate_enabled, meadow_contact_ao);
        if meadow_contact_ao {
            eprintln!("GPU meadow contact AO: actual strength {}; analytic retained-root basal footprint, terrain diffuse skylight only, not a cast shadow",
                f32::from_bits(meadow_substrate_settings.control[3]));
        }
        let meadow_substrate = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared meadow substrate material"),
            contents: bytemuck::bytes_of(&meadow_substrate_settings),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let meadow_edge_fallback = buffer(gpu, "disabled meadow edge occupancy", &[0; 256], wgpu::BufferUsages::STORAGE);
        let meadow_edge_buffer = meadow.as_ref().and_then(|m| m.edge_shadow_buffer()).unwrap_or(&meadow_edge_fallback);
        assert!(!(far_sun.enabled && foliage_sun_pages.enabled),"choose paged foliage or legacy experimental far sun map, not both");
        let noise_view = crate::noise_field::texture(gpu, scene.noise.as_ref());
        let make_scene_bind = |headlight_view:&wgpu::TextureView, far_view:&wgpu::TextureView, instance_data:&wgpu::Buffer, wind_data:&wgpu::Buffer| bind(
            gpu,
            &scene_layout,
            &[
                (0, vertices.as_entire_binding()),
                (1, cverts.as_entire_binding()),
                (2, tris.as_entire_binding()),
                (3, clusters.as_entire_binding()),
                (4, instance_data.as_entire_binding()),
                (5, materials.as_entire_binding()),
                (6, wgpu::BindingResource::TextureView(&albedo_view)),
                (7, wgpu::BindingResource::Sampler(&samp)),
                (8, aux.as_entire_binding()),
                (9, wgpu::BindingResource::TextureView(&normals_view)),
                (10, wgpu::BindingResource::Sampler(&plant_samp)),
                (11, voxel_replacement.as_entire_binding()),
                (12, wind_data.as_entire_binding()),
                (13, grass_lighting.uniform.as_entire_binding()),
                (14, wgpu::BindingResource::TextureView(&grass_lighting.terrain)),
                (15, wgpu::BindingResource::TextureView(&grass_lighting.coverage)),
                (16, wgpu::BindingResource::TextureView(&grass_lighting.tiles)),
                (17, headlights.uniform.as_entire_binding()),
                (18, wgpu::BindingResource::TextureView(headlight_view)),
                (19, car_materials.as_entire_binding()),
                (20, wgpu::BindingResource::TextureView(&tracks.atlas_view)),
                (21, wgpu::BindingResource::TextureView(&tracks.lookup_view)),
                (22, far_sun.uniform.as_entire_binding()),
                (23, wgpu::BindingResource::TextureView(far_view)),
                (24, world_visibility.buffer.as_entire_binding()),
                (25, world_visibility.uniform.as_entire_binding()),
                (26, foliage_sun_pages.metadata.as_entire_binding()),
                (27, meadow_substrate.as_entire_binding()),
                (28, wgpu::BindingResource::TextureView(&noise_view)),
                (29, meadow_edge_buffer.as_entire_binding()),
            ],
        );
        let sampled_far=if foliage_sun_pages.enabled {&foliage_sun_pages.sampled}else{&far_sun.view};
        let scene_bind=make_scene_bind(&headlights.sampled,sampled_far,&instances,wind.buffer());
        let shadow_scene_bind=make_scene_bind(&headlights.empty,&far_sun.empty,&instances,wind.buffer());
        let far_sun_scene_bind=make_scene_bind(&headlights.empty,&far_sun.empty,&far_sun.instances,&far_sun.wind);
        let foliage_sun_scene_bind=make_scene_bind(&headlights.empty,&foliage_sun_pages.empty,&foliage_sun_pages.instances,&foliage_sun_pages.wind);
        // Cull consumes five buffers, not the full material/lighting layout.
        // Spatial grouping, sun owners and cluster traversal keep this layout
        // within the device's sixteen-storage-buffer stage limit.
        let cull_scene_bindings:Vec<_>=scene_bindings.iter().copied()
            .filter(|(binding,_)|crate::cull_groups::SCENE_BINDINGS.contains(binding)).collect();
        let cull_scene_layout=layout(gpu,"cull scene",&cull_scene_bindings);
        let make_cull_bind=|instance_data:&wgpu::Buffer,wind_data:&wgpu::Buffer|bind(gpu,&cull_scene_layout,&[
            (3,clusters.as_entire_binding()),(4,instance_data.as_entire_binding()),(5,materials.as_entire_binding()),
            (11,voxel_replacement.as_entire_binding()),(12,wind_data.as_entire_binding())]);
        let cull_scene_bind=make_cull_bind(&instances,wind.buffer());
        let far_cull_scene_bind=make_cull_bind(&far_sun.instances,&far_sun.wind);
        let foliage_cull_scene_bind=make_cull_bind(&foliage_sun_pages.instances,&foliage_sun_pages.wind);
        let instance_groups=crate::cull_groups::Resources::new(gpu,&scene.instances,&instances,wind.buffer(),replacement_plan.as_ref())
            .expect("invalid spatial instance group resources");
        let receiver_demand=crate::receiver_shadow_demand::Resources::new(gpu,&instances,wind.buffer(),&clusters,scene.instances.len()as u32)
            .expect("invalid receiver shadow demand resources");
        let meadow_receivers=if receiver_demand.enabled() {
            scene.meadow.as_ref().map(|field|crate::meadow_shadow_receivers::MeadowReceivers::new(
                field,meadow.as_ref().is_some_and(|m|m.aggregate_enabled()))).transpose()
                .expect("invalid meadow receiver support")
        } else {None};
        let cluster_traversal=crate::cluster_traversal::Resources::new(gpu,scene)
            .expect("invalid shared cluster traversal resources");
        let voxel_scene_bind = bind(gpu, &voxel_scene_layout, &[
            (4, instances.as_entire_binding()),
            (12,wind.buffer().as_entire_binding()),
            (13, grass_lighting.uniform.as_entire_binding()),
            (14, wgpu::BindingResource::TextureView(&grass_lighting.terrain)),
            (15, wgpu::BindingResource::TextureView(&grass_lighting.coverage)),
            (16, wgpu::BindingResource::TextureView(&grass_lighting.tiles)),
            (17, headlights.uniform.as_entire_binding()),
            (18, wgpu::BindingResource::TextureView(&headlights.sampled)),
            (22, far_sun.uniform.as_entire_binding()),
            (23, wgpu::BindingResource::TextureView(sampled_far)),
            (24, world_visibility.buffer.as_entire_binding()),
            (25, world_visibility.uniform.as_entire_binding()),
            (26, foliage_sun_pages.metadata.as_entire_binding()),
        ]);
        let view_cull_layout = layout(gpu, "view cull", &[(0, uniform()), (1, storage(false)), (2, storage(false)), (3, storage(false)), (4, storage(false)), (5, uniform()), (6, uniform()),
            (7,wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}),
            (8,storage(true)),(9,storage(true)),(10,storage(false)),(11,storage(true)),
            (12,storage(true)),(13,storage(false)),(14,storage(false)),(15,uniform())]);
        let view_raster_layout = layout(gpu, "view raster", &[(0, uniform()), (1, storage(true)), (2, storage(true)), (3, storage(true)), (4, uniform())]);
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
                (16, storage(false)),
                (17, storage(false)),
                (18, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (19, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (20, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            ],
        );
        let canopy = crate::canopy::CanopyClipmap::new(gpu, scene, &scene_layout)
            .expect("scene exceeds the bounded canopy representation");
        startup.mark("scene_bindings_canopy_resources");
        let tone_layout = layout(
            gpu,
            "tone",
            &[
                (0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (1, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba8Unorm, view_dimension: wgpu::TextureViewDimension::D2 }),
                (2, uniform()),
                (3, wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}),
                (4, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                (5, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
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
                (6, float_tex(false)),
                (7, float_tex(false)),
            ],
        );
        let module = |name: &str, src: &str| gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(name), source: wgpu::ShaderSource::Wgsl(src.into()) });
        startup.mark("postprocess_layouts");
        let cull_mod = module("cull", &cull_source());
        startup.mark("cull_shader_module");
        let raster_mod = module("raster", &crate::grass::raster_source());
        startup.mark("raster_shader_module");
        let shade_mod = module("shade", &crate::wind::shade_source());
        startup.mark("shade_shader_module");
        let tone_mod = module("tonemap", include_str!("shaders/tonemap.wgsl"));
        let bloom=crate::bloom::Bloom::new(gpu);
        startup.mark("tone_shader_module");
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
                cache: gpu.pipeline_cache(),
            })
        };
        let cull_pl = pl(&[&cull_scene_layout, &view_cull_layout]);
        // Constructor-time switches permit same-binary ABBA comparisons with
        // identical assets, LOD thresholds, lighting and camera inputs.
        let cluster_buckets = std::env::var("KERN_CLUSTER_BUCKETS").as_deref() != Ok("0");
        let simple_lighting=std::env::var("KERN_SIMPLE_LIGHTING").map_or(scene.graphics.simple_lighting,|v|v=="1");
        let tree_occlusion = crate::tree_occlusion::Resources::new(gpu,cluster_buckets,scene.gpu_forest.is_some());
        let plain_meshes = scene.meadow.is_none() && scene.far_grass.is_none()
            && scene.voxel_forest.is_none() && scene.foliage_world.is_none()
            && scene.foliage_representations.is_empty() && scene.foliage_source_groups.is_empty()
            && scene.part_wind.is_empty() && scene.part_attachments.is_empty()
            && scene.aggregate_lod.is_none() && scene.baked_animations.is_none()
            && scene.procedural_terrain.is_none() && scene.crowd.is_none()
            && scene.instances.iter().all(|i|i.flags & (crate::scene::FOLIAGE | crate::scene::LIVE_GRASS
                | crate::scene::WIND_GRASS | crate::scene::PART_WIND)==0);
        let simple_cull=std::env::var("KERN_SIMPLE_CULL").map_or(scene.graphics.simple_mesh_culling,|v|v=="1")
            && plain_meshes && !instance_groups.allocated() && !cluster_traversal.enabled && !tree_occlusion.enabled
            && std::env::var("KERN_STATIC_SUN_CACHE").as_deref()!=Ok("1")
            && std::env::var("KERN_INSTANCE_BROADPHASE").as_deref()!=Ok("1");
        let early_grass_cull = std::env::var("KERN_EARLY_GRASS_CULL").as_deref() != Ok("0");
        let far_trees = if scene.voxel_forest.is_some() { f64::from(scene.voxel_far_distance_m) } else { 0.0 };
        let small_cluster_triangles=if simple_cull {12}else{8};
        let cull_constants = [("SMALL_CLUSTER_TRIANGLES",small_cluster_triangles as f64),("INSTANCE_COUNT",scene.instances.len() as f64),("CLUSTER_BUCKETS", u8::from(cluster_buckets) as f64),
                              ("EARLY_GRASS_CULL", u8::from(early_grass_cull) as f64), ("VOXEL_FAR_M", far_trees)];
        // Quest Adreno drivers silently drop indirect draws. Plain mesh scenes
        // can use a conservative direct draw with GPU-side count rejection.
        let adreno=cfg!(target_os="android") && gpu.info.backend==wgpu::Backend::Vulkan && gpu.info.vendor==0x5143;
        let bounded_draws=plain_meshes && std::env::var("KERN_BOUNDED_DRAWS").map_or(adreno && simple_cull,|v|v=="1");
        let minimal_forward=(scene.graphics.minimal_forward && plain_meshes && scene.skinning.actors.is_empty()
            && scene.debris.is_none_or(|d|d.capacity==0)).then(||std::sync::Mutex::new(crate::forward::Forward::new(gpu,scene,&materials)));
        let direct_bounds=bounded_draws.then(||std::sync::Mutex::new(crate::direct_draw::Bounds::new(scene,small_cluster_triangles)));
        let raster_constants = [("CLUSTER_BUCKETS", u8::from(cluster_buckets) as f64), ("VOXEL_FAR_M", far_trees), ("DIRECT_BOUNDED",u8::from(bounded_draws)as f64)];
        let cull_compute = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(&cull_pl), module: &cull_mod, entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &cull_constants, ..Default::default() },
            cache: gpu.pipeline_cache(),
        });
        let cull = cull_compute(if simple_cull {"cull_simple"} else {"cull"});
        let cached_sun_cull=scene.gpu_forest.as_ref().map(|_| {
            assert!(!simple_cull,"GPU forest requires cooperative cluster culling");
            let layout=pl(&[&cull_scene_layout,&view_cull_layout,&crate::shadow_damage::mask_layout(gpu)]);
            let make=|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label:Some(entry),layout:Some(&layout),module:&cull_mod,entry_point:Some(entry),
                compilation_options:wgpu::PipelineCompilationOptions {constants:&cull_constants,..Default::default()},cache:gpu.pipeline_cache(),
            });
            (make("cull_cached_sun"),make("cull_spatial_cached_sun"))
        });
        let static_sun = if std::env::var("KERN_STATIC_SUN_CACHE").as_deref() == Ok("1") {
            Some(crate::static_sun::StaticSun::new(gpu, scene, SHADOW_SIZE)
                .expect("static sun cache exceeds device/source limits"))
        } else { None };
        let split_sun_cull = static_sun.as_ref().map(|_| (cull_compute("cull_sun_static"), cull_compute("cull_sun_dynamic")));
        let sun_owner_buffer = static_sun.as_ref().map(|s| s.owner_buffer().clone()).unwrap_or_else(||
            buffer(gpu, "disabled static sun owners", bytemuck::cast_slice(&[0u32,0,4,4]), wgpu::BufferUsages::STORAGE));
        let compact_cull = (std::env::var("KERN_INSTANCE_BROADPHASE").as_deref() == Ok("1"))
            .then(|| (cull_compute("instance_broadphase"), cull_compute("instance_dispatch"), cull_compute("cull_compacted")));
        let spatial_cull=instance_groups.enabled.then(||(cull_compute("spatial_select"),cull_compute("spatial_dispatch"),cull_compute("cull_spatial")));
        let sun_spatial_cull=instance_groups.sun_enabled.then(||(cull_compute("sun_spatial_select"),cull_compute("spatial_dispatch"),cull_compute("cull_spatial")));
        let headlight_spatial_cull=instance_groups.headlight_enabled.then(||(cull_compute("headlight_spatial_select"),cull_compute("spatial_dispatch"),cull_compute("cull_spatial")));
        let finalize = cull_compute("finalize");
        let scatter = cull_compute("scatter");
        let prepare_tree_phase = tree_occlusion.enabled.then(||cull_compute("prepare_tree_phase"));
        let recover_tree_clusters=tree_occlusion.history_first.then(||cull_compute("recover_tree_clusters"));
        startup.mark("cull_finalize_scatter_pipelines");
        let raster_pl = pl(&[&scene_layout, &view_raster_layout]);
        let shadow_mask_layout=crate::shadow_damage::mask_layout(gpu);
        let cached_raster_pl=pl(&[&scene_layout,&view_raster_layout,&shadow_mask_layout]);
        let raster_pipe = |color: bool, sun_receiver_plane: bool, cached:bool| {
            let targets = [Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })];
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if color { "raster" } else { "raster depth" }),
                layout: Some(if cached {&cached_raster_pl}else{&raster_pl}),
                vertex: wgpu::VertexState { module: &raster_mod, entry_point: Some("vs"), compilation_options: wgpu::PipelineCompilationOptions { constants: &raster_constants, ..Default::default() }, buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    // Main view: reversed Z (1 near, 0 at infinity). Shadow: ordinary.
                    depth_compare: Some(if color { wgpu::CompareFunction::Greater } else { wgpu::CompareFunction::Less }),
                    stencil: Default::default(),
                    bias: if color { Default::default() } else { wgpu::DepthBiasState { constant: 2, slope_scale: if sun_receiver_plane { 0.0 } else { 2.0 }, clamp: 0.0 } },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &raster_mod,
                    entry_point: Some(if color { "fs" } else if cached {"fs_cached_depth"} else { "fs_depth" }),
                    compilation_options: wgpu::PipelineCompilationOptions {constants:if cached {&[("SHADOW_TILE_PIXELS",(SHADOW_SIZE/64)as f64)]}else{&[]},..Default::default()},
                    targets: if color { &targets } else { &[] },
                }),
                multiview_mask: None,
                cache: gpu.pipeline_cache(),
            })
        };
        let raster = raster_pipe(true, false,false);
        let raster_depth = raster_pipe(false, false,false);
        let raster_sun_depth = (std::env::var("KERN_SUN_RECEIVER_PLANE").as_deref() == Ok("1"))
            .then(|| raster_pipe(false, true,false));
        let raster_far_sun_depth=(far_sun.enabled || foliage_sun_pages.enabled).then(||raster_pipe(false,true,false));
        let raster_cached_sun=gpu_forest.as_ref().map(|_|raster_pipe(false,raster_sun_depth.is_some(),true));
        startup.mark("raster_shadow_pipelines");
        let sun_scene_bounds = (scene.gpu_forest.is_some() || std::env::var("KERN_SUN_SCENE_DEPTH").as_deref() == Ok("1"))
            .then(|| std::sync::Mutex::new(crate::stability::SunSceneBounds::from_scene(scene)));
        let sun_cache_guard_m=crate::static_sun::configured_guard(std::env::var("KERN_SUN_CACHE_GUARD_METRES").ok().as_deref());
        let sun_guard_bounds=(sun_cache_guard_m>0.0)
            .then(||std::sync::Mutex::new(crate::stability::SunSceneBounds::from_scene(scene)));
        startup.mark("sun_scene_bounds");
        let shade_pl = pl(&[&scene_layout, &shade_layout]);
        let rock_variation=std::env::var("KERN_ROCK_VARIATION").as_deref()==Ok("1");
        let skip_hidden_meadow=meadow.as_ref().is_some_and(|m|m.skip_hidden_shade_enabled());
        let hidden_meadow_fallback=crate::meadow::MeadowRenderer::hidden_shade_fallback(gpu);
        let meadow_edge_enabled=meadow.as_ref().is_some_and(|m|m.edge_shadow_stats().is_some_and(|s|s.enabled));
        let shade_features=crate::shade_features::ShadeFeatures::for_scene(scene,world_visibility.enabled,foliage_sun_pages.enabled,meadow_substrate_settings.control[0]!=0 || meadow_edge_enabled);
        let sky_tiles=!simple_lighting && std::env::var("KERN_SKY_TILES").as_deref()==Ok("1");
        let mut shade_constants=shade_features.constants(rock_variation,skip_hidden_meadow);
        shade_constants.push(("SKY_TILES", if sky_tiles {1.0} else {0.0}));
        shade_constants.push(("WORLD_TERRAIN_BAKED", if world_visibility.baked {1.0} else {0.0}));
        // KERN_VOXEL_SURFACE=0: rasterized voxels keep their own lighting
        // (resolve and composite) instead of drawing into the scene's
        // visibility buffer for shade (VOXEL_SURFACE).
        let voxel_surface = !simple_lighting && voxel_forest.as_ref().is_some_and(|f| f.casts_sun())
            && std::env::var("KERN_VOXEL_SURFACE").as_deref() != Ok("0");
        shade_constants.push(("VOXEL_OCCLUSION", if voxel_forest.as_ref().is_some_and(|f| f.casts_sun()) && !voxel_surface {1.0} else {0.0}));
        shade_constants.push(("VOXEL_SURFACE", if voxel_surface {1.0} else {0.0}));
        // Voxel pixels in their own pass (KERN_VOXEL_PASS=0 keeps them in
        // shade): the big kernel's register footprint was the limit, not
        // their lighting (playtest cameras 2560x1440: 29-34 -> 33-38 fps,
        // frames identical).
        let voxel_pass_on = voxel_surface && std::env::var("KERN_VOXEL_PASS").as_deref() != Ok("0");
        shade_constants.push(("VOXEL_PASS", if voxel_pass_on {1.0} else {0.0}));
        let terrain_coarse_on = !simple_lighting && shade_features.terrain && std::env::var("KERN_TERRAIN_COARSE").as_deref() != Ok("0");
        shade_constants.push(("TERRAIN_COARSE", if terrain_coarse_on {1.0} else {0.0}));
        if let Some(m) = std::env::var("KERN_COARSE_FOOTPRINT_M").ok().and_then(|v| v.parse::<f64>().ok()) { shade_constants.push(("COARSE_FOOTPRINT_M", m)); }
        // KERN_SHADE_CLASSES=0: one shade pipeline for terrain and the rest.
        let shade_classes = !simple_lighting && shade_features.terrain && std::env::var("KERN_SHADE_CLASSES").as_deref() != Ok("0");
        // The terrain class keeps every feature terrain pixels can reach.
        let terrain_constants: Vec<(&'static str, f64)> = shade_constants.iter().map(|&(k, v)| match k {
            "FEATURE_CARS" | "FEATURE_FOLIAGE" | "FEATURE_PROCEDURAL" | "FEATURE_GRASS" => (k, 0.0), _ => (k, v) })
            .chain([("SHADE_CLASS", 1.0)]).collect();
        // The rest class drops the terrain branch unless a car part carries
        // the TERRAIN bit (car materials keep their index in bits 16..).
        let car_terrain = scene.materials.iter().any(|m| m.flags & crate::scene::TERRAIN != 0 && m.flags >> 16 != 0);
        if shade_classes && !car_terrain {
            for c in shade_constants.iter_mut() { if c.0 == "FEATURE_TERRAIN" { c.1 = 0.0; } }
        }
        shade_constants.push(("SHADE_CLASS", if shade_classes {2.0} else {0.0}));
        // Component diagnostics (Lighting::debug) are compiled out of the
        // shade pipelines; the first frame that asks for one builds the
        // diagnostic pair (shade_diagnostic). They cost 1.3-3.6 fps at the
        // playtest cameras in registers alone.
        let diagnostic_constants: Vec<(&'static str, f64)> = shade_constants.clone();
        shade_constants.push(("SHADE_DIAGNOSTICS", 0.0));
        let shade = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some("shade"),layout:Some(&shade_pl),module:&shade_mod,entry_point:Some(if simple_lighting {"shade_simple"} else {"shade"}),
            compilation_options:wgpu::PipelineCompilationOptions {constants:&shade_constants,..Default::default()},cache: gpu.pipeline_cache(),
        });
        let voxel_depth_fallback = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("no voxel depth"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default());
        let empty_layout = layout(gpu, "empty", &[]);
        let empty_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("empty"), layout: &empty_layout, entries: &[] });
        let terrain_coarse_layout = layout(gpu, "terrain coarse out", &[(32, wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba32Uint, view_dimension: wgpu::TextureViewDimension::D2 })]);
        let terrain_coarse = terrain_coarse_on.then(|| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some("terrain coarse"),layout:Some(&pl(&[&scene_layout, &shade_layout, &empty_layout, &terrain_coarse_layout])),module:&shade_mod,entry_point:Some("terrain_coarse"),
            compilation_options:wgpu::PipelineCompilationOptions {constants:&shade_constants,..Default::default()},cache: gpu.pipeline_cache(),
        }));
        let terrain_coarse_fallback = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("no coarse terrain"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default());
        let terrain_shade = shade_classes.then(|| {
            let mut constants = terrain_constants.clone();
            constants.push(("SHADE_DIAGNOSTICS", 0.0));
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label:Some("shade terrain"),layout:Some(&shade_pl),module:&shade_mod,entry_point:Some("shade"),
                compilation_options:wgpu::PipelineCompilationOptions {constants:&constants,..Default::default()},cache: gpu.pipeline_cache(),
            })
        });
        let voxel_pass = voxel_pass_on.then(|| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some("shade voxels"),layout:Some(&shade_pl),module:&shade_mod,entry_point:Some("shade_voxels"),
            compilation_options:wgpu::PipelineCompilationOptions {constants:&shade_constants,..Default::default()},cache: gpu.pipeline_cache(),
        }));
        let shade_classify = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some("shade sky tiles"),layout:Some(&shade_pl),module:&shade_mod,entry_point:Some("classify"),
            compilation_options:wgpu::PipelineCompilationOptions {constants:&shade_constants,..Default::default()},cache: gpu.pipeline_cache(),
        });
        let sky_vis_bake = (std::env::var("KERN_SKY_VIS_CACHE").as_deref() != Ok("0")).then(|| {
            let inputs = layout(gpu, "sky transmittance inputs", &[
                (10, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                (11, uniform()),
                (13, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                (14, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ]);
            let output = layout(gpu, "sky transmittance output", &[
                (30, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D3 }),
                (31, uniform()),
            ]);
            let empty = layout(gpu, "empty", &[]);
            let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("bake sky transmittance"), layout: Some(&pl(&[&empty, &inputs, &empty, &output])), module: &shade_mod,
                entry_point: Some("bake_sky_visibility"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &shade_constants, ..Default::default() }, cache: gpu.pipeline_cache(),
            });
            let levels = [0u32, 1, 2].map(|l| buffer(gpu, "sky transmittance level", bytemuck::cast_slice(&[l, 0, 0, 0]), wgpu::BufferUsages::UNIFORM));
            SkyVisBake { pipeline, inputs, output, levels }
        });
        startup.mark("shade_pipeline");
        // Tonemap and TAA read their neighbourhoods from a shared tile;
        // KERN_TILED_POST=0 selects the direct versions (same image).
        let post_entry = if std::env::var("KERN_TILED_POST").as_deref() == Ok("0") { "main_direct" } else { "main" };
        let tone = compute(&pl(&[&tone_layout]), &tone_mod, post_entry);
        let taa_mod = module("taa", include_str!("shaders/taa.wgsl"));
        let taa = compute(&pl(&[&taa_layout]), &taa_mod, post_entry);
        startup.mark("tonemap_taa_pipelines");
        let clamp_samp = gpu.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let density_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("linear canopy"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let solids = scene.solid_occluders();
        let solid_count = solids.len() as u32;
        let solid_occluders = buffer(gpu, "solid occluders", if solids.is_empty() { &[0u8; 48] } else { bytemuck::cast_slice(&solids) }, wgpu::BufferUsages::STORAGE);
        startup.mark("solid_occluder_resources");

        let (sky_tex, sky_sh, raw_horizon_scaled, sky_scale, sky_limit, rotation) = scene_environment(gpu, scene, light);
        let shadow_samp = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        startup.mark("sky_sh_integration_upload");
        let query = if gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            let n = TIMESTAMP_CAPACITY;
            let qs = gpu.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("timestamps"), ty: wgpu::QueryType::Timestamp, count: n });
            let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 8 * n as u64, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 8 * n as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            Some((qs, resolve, read))
        } else {
            None
        };
        let async_gpu_profile = std::sync::Mutex::new(crate::gpu_profile::Profiler::new(query.is_some(), TIMESTAMP_CAPACITY));
        startup.mark("timestamp_resources");
        let exhaust=crate::exhaust::ExhaustRenderer::new(gpu);
        startup.mark("exhaust_resources_pipelines");
        let wheel_particles=crate::wheel_particles::WheelParticles::new(gpu);
        startup.mark("wheel_particle_resources_pipelines");
        let mut renderer=Renderer {
            resource_device: gpu.device.clone(),
            albedo_array: albedo.clone(),
            live_screens: (!scene.live_screen_feeds.is_empty()).then(|| std::sync::Mutex::new(crate::screen_feed::LiveScreens::new(gpu, &albedo, &scene.live_screen_feeds))),
            aux_buffer: aux.clone(),
            irradiance_volume: scene.irradiance_volume,
            dents: scene.dents,
            dfao: scene.dfao,
            geometry_diagnostics,
            debug_options: Default::default(),
            upscaling: Default::default(),
            motion_blur: Default::default(),
            async_gpu_profile,
            profile_sequence: std::sync::atomic::AtomicU64::new(0),
            profile_frame_id: std::sync::atomic::AtomicU64::new(u64::MAX),
            startup_profile: None,
            frame_time_ms: std::sync::atomic::AtomicU32::new((1000.0f32 / 60.0).to_bits()),
            scene_bind,
            cull_scene_bind,
            far_cull_scene_bind,
            foliage_cull_scene_bind,
            voxel_scene_bind,
            voxel_scene_layout,
            meadow_brick_fixture: None,
            shadow_scene_bind,
            far_sun_scene_bind,
            far_sun,
            foliage_sun_pages,
            // Opt-in comparison: the same executable retains the original
            // synchronous cache validation unless explicitly requested.
            async_shadow_validation: (std::env::var("KERN_ASYNC_SHADOW_VALIDATION").as_deref()==Ok("1"))
                .then(Default::default),
            foliage_sun_scene_bind,
            foliage_page_targets:Default::default(),
            world_visibility,
            far_sun_targets: Default::default(),
            headlights,
            tracks,
            exhaust,
            wheel_particles,
            headlight_state: Default::default(),
            headlight_exclusions: Default::default(),
            camera_up_axes: Default::default(),
            camera_projections: Default::default(),
            local_light_state: Default::default(),
            spot_falloff: Default::default(),
            spot_softness: Default::default(),
            lighting_focus: Default::default(),
            diagnostic_sun_focus: Default::default(),
            legacy_light_lookahead: std::env::var("KERN_LEGACY_LIGHT_LOOKAHEAD").is_ok_and(|v|v=="1"),
            headlight_targets: Default::default(),
            instance_buffer: instances,
            wind,
            part_wind,
            gpu_forest,
            part_wind_lod: scene.part_wind_lod,
            aggregate_lod,
            aggregate_lod_enabled: std::sync::atomic::AtomicBool::new(true),
            baked_animations,
            skinning,
            debris,
            procedural_terrain,
            crowd,
            fx: Default::default(),
            volumetric: Default::default(),
            sprites: Default::default(),
            transparency,
            grass,
            meadow,
            meadow_substrate_enabled: meadow_substrate_settings.control[0] != 0,
            meadow_substrate_buffer: meadow_substrate,
            meadow_substrate_settings,
            grass_detail: scene.grass_detail,
            far_grass,
            voxel_forest,
            foliage_world,source_replacement,source_readiness,
            foliage_world_enabled:std::sync::atomic::AtomicBool::new(scene.foliage_world_experimental),
            foliage_world_valid:std::sync::atomic::AtomicBool::new(true),
            foliage_world_error_pixels:scene.foliage_world_error_pixels,
            foliage_representations: scene.foliage_representations.clone(),
            foliage_representation_policy: scene.foliage_representation_policy,
            foliage_planners: Default::default(),
            view_targets: Default::default(),
            view_uniforms: Default::default(),
            frame_resources: Default::default(),
            output_view: Default::default(),
            shadow_view: Default::default(),
            volume_view_layout,
            cull,
            split_sun_cull,
            compact_cull,
            spatial_cull,
            cached_sun_cull,
            sun_spatial_cull,
            headlight_spatial_cull,
            instance_groups,
            receiver_demand,
            meadow_receivers,
            cluster_traversal,
            sun_diagnostics:Default::default(),
            headlight_diagnostics:Default::default(),
            finalize,
            scatter,
            prepare_tree_phase,recover_tree_clusters,
            tree_occlusion,
            cluster_buckets,
            simple_lighting,
            sharp_sprites: scene.graphics.sharp_sprites,
            simple_cull,
            small_cluster_vertices:small_cluster_triangles*3,
            direct_bounds,
            minimal_forward,
            raster_large_first: std::env::var("KERN_RASTER_LARGE_FIRST").as_deref()==Ok("1"),
            early_grass_cull,
            raster,
            raster_depth,
            raster_sun_depth,
            raster_cached_sun,shadow_damage:std::sync::Mutex::new(shadow_damage),legacy_shadow_wind,
            raster_far_sun_depth,
            sun_scene_bounds,
            sun_guard_bounds,
            sun_cache_guard_m,
            sun_cache_anchor: Default::default(),
            wind_freeze: crate::wind::WindFreeze::configured(std::env::var("KERN_WIND_FREEZE").ok().as_deref()),
            wind_freeze_pin: Default::default(),
            sun_terrain_error: std::env::var("KERN_SUN_TERRAIN_ERROR").as_deref()==Ok("1"),
            sun_surface_source: std::env::var("KERN_SUN_SURFACE_SOURCE").as_deref()==Ok("1"),
            camera_sun_occlusion: std::env::var("KERN_CAMERA_SUN_OCCLUSION").as_deref()!=Ok("0"),
            shade,
            shade_pl,
            sky_tiles,
            shade_classify,
            voxel_depth_fallback,
            voxel_surface,
            voxel_pass,
            terrain_shade,
            diagnostic_constants,
            shade_diagnostic: std::sync::OnceLock::new(),
            voxel_surface_fallback: gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("no voxel surface"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default()),
            terrain_coarse,
            terrain_coarse_targets: std::sync::Mutex::new(Vec::new()),
            terrain_coarse_fallback,
            terrain_coarse_layout,
            empty_bind,
            sky_vis_bake,
            shade_tiles: std::sync::Mutex::new(Vec::new()),
            shade_text: crate::wind::shade_source(),
            shader_wind_text: crate::wind::common_source(),
            shader_generation: 0,
            rock_variation,
            shade_features,
            cpu_spans: crate::cpu_spans::Collector::from_env(),
            empty_gi_volume: scene.heightfield.heights.iter().all(|&h| h <= -1e8) && scene.voxel_forest.is_none() && scene.foliage_world.is_none()
                && !scene.instances.iter().any(|i| i.flags & crate::scene::FOLIAGE != 0),
            car_materials,
            car_material_count: car_list.len(),
            materials,
            material_count: scene.materials.len(),
            tone,
            bloom,
            shade_layout,
            hidden_meadow_fallback,
            tone_layout,
            view_cull_layout,
            view_raster_layout,
            instances: scene.instances.len() as u32,
            sky_view: sky_tex.create_view(&Default::default()),
            sky_sh,
            sky_horizon: raw_horizon_scaled,
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
            heightfield: height_view,
            hf,
            query,
            taa,
            taa_layout,
            clamp_samp,
            history: Default::default(),
            frame: Default::default(),
            shadow_cache: Default::default(),
            static_sun: static_sun.map(std::sync::Mutex::new),
            sun_owner_buffer,
            static_sun_targets: Default::default(),
            // Explicit reference switches for paired captures; both preserve
            // the existing caster geometry, filtering and daylight lighting.
            reuse_sun_texture: std::env::var("KERN_REUSE_SUN_TEXTURE").as_deref() != Ok("0"),
            zero_sun_fast_path: std::env::var("KERN_ZERO_SUN_FAST_PATH").as_deref() != Ok("0"),
            shadow_extent: Default::default(),
            near_shadows: Default::default(),
            cluster_views: Default::default(),
            color_diagnostics: Default::default(),
            // Upper bound: all clusters of all shadow-casting instances. An
            // empty instance (a reserved slot, a GPU-driven particle) may later
            // hold any mesh, so it counts as the largest one.
            shadow_capacity: {
                let largest = scene.meshes.iter().map(|m| m.cluster_count as u64).max().unwrap_or(1);
                scene.instances.iter().filter(|i| i.flags & crate::scene::NO_SHADOW == 0)
                    .map(|i| if i.cluster_count == 0 { largest } else { i.cluster_count as u64 }).sum::<u64>().clamp(1, MAX_VISIBLE as u64) as u32
            },
        };
        let pipeline_cache=gpu.persist_pipeline_cache();
        startup.mark("pipeline_cache_persist");
        startup.pipeline_cache(pipeline_cache);
        renderer.startup_profile=startup.finish();
        renderer
    }

    fn view_targets(&self, gpu: &Gpu, label: &str, index: usize) -> ViewTargets {
        // These buffers depend on cluster capacity, not output resolution.
        // Reuse them across frames/resizes; queue writes remain submission-ordered.
        let mut views = self.cluster_views.lock().unwrap();
        while views.len() <= index {
            let camera= !views.is_empty(); // slot0 shared sun; slots1..N colour views
            views.push(self.view_targets_capacity(gpu, label, MAX_VISIBLE,camera,!camera&&self.instance_groups.sun_enabled));
        }
        views[index].clone()
    }

    /// Explicit blocking diagnostic, never called by normal rendering. Returns
    /// the actual GPU-selected (source instance, cluster) pairs for one colour
    /// view so optimizations can be compared independently of atomic draw order.
    /// `expected_render_sequence` rejects stale cross-call requests. Only views
    /// from the last successful submit are accessible; old pooled targets are not.
    pub fn debug_visible_clusters(&self, gpu: &Gpu, view: usize, expected_render_sequence: Option<u64>) -> Result<VisibleClusterDiagnostic, String> {
        // Hold this guard through readback: another render cannot overwrite the
        // referenced buffers while their immutable frame identity is being read.
        let diagnostics = self.color_diagnostics.lock().unwrap();
        let context = diagnostics.context.ok_or("no submitted colour frame is available")?;
        if expected_render_sequence.is_some_and(|sequence| sequence != context.render_sequence) {
            return Err("stale colour-frame diagnostic request".into());
        }
        let target = diagnostics.views.get(view).ok_or("colour view is not active in the last submitted frame")?;
        let read = |source: &wgpu::Buffer, bytes: u64| -> Result<Vec<u8>, String> {
            let dst = gpu.device.create_buffer(&wgpu::BufferDescriptor {label: Some("explicit cluster diagnostic"),
                size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false});
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(source, 0, &dst, 0, bytes);
            gpu.queue.submit([enc.finish()]);
            dst.slice(..).map_async(wgpu::MapMode::Read, |_| {}); gpu.wait();
            let result = dst.slice(..).get_mapped_range().map_err(|e|e.to_string())?.to_vec();
            dst.unmap(); Ok(result)
        };
        let bytes = read(&target.counter, 4)?;
        let count = u32::from_le_bytes(bytes.try_into().unwrap());
        if count > MAX_VISIBLE { return Err("visible-cluster capacity exceeded".into()); }
        let pairs = if count == 0 { Vec::new() } else {
            read(&target.visible, u64::from(count) * 8)?.chunks_exact(8).map(|b|
                [u32::from_le_bytes(b[..4].try_into().unwrap()),u32::from_le_bytes(b[4..].try_into().unwrap())]).collect()
        };
        let categories=diagnostics.owners.as_ref().map(|owners|self.geometry_diagnostics.count(&pairs,owners,self.cluster_buckets)).transpose()?;
        let foliage_selection=if self.foliage_representations.is_empty() {Vec::new()} else {
            read(&target.foliage_selection,target.foliage_selection.size())?
                .chunks_exact(4).map(|b|crate::voxel_forest::selection_unpack(u32::from_le_bytes(b.try_into().unwrap()))).collect()
        };
        Ok(VisibleClusterDiagnostic { frame_id:context.frame_id, render_sequence:context.render_sequence,
            view_index:view,view_count:context.views,output_size:context.output_size,input_size:target.input_size,
            viewport:[target.view.x,target.view.y,target.view.w,target.view.h],camera:target.view.cam,pairs,categories,foliage_selection })
    }

    /// Constructor-time diagnostic, absent unless KERN_DIAGNOSTIC_HIDE is set.
    /// 'none' enables baseline counts; exclusions affect colour geometry only.
    pub fn geometry_diagnostic_config(&self)->Result<crate::geometry_diagnostics::Config,String> {
        self.geometry_diagnostics.config()
    }

    pub fn debug_sun_culling(&self,gpu:&Gpu,expected_render_sequence:Option<u64>)
        ->Result<Vec<crate::shadow_diagnostics::Diagnostic>,String> {
        crate::shadow_diagnostics::capture(gpu,&self.instance_groups,&self.sun_diagnostics.lock().unwrap(),expected_render_sequence)
    }

    /// Independent perspective light selections; never camera/sun visibility.
    pub fn debug_headlight_culling(&self,gpu:&Gpu,expected_render_sequence:Option<u64>)
        ->Result<Vec<crate::shadow_diagnostics::Diagnostic>,String> {
        crate::shadow_diagnostics::capture(gpu,&self.instance_groups,&self.headlight_diagnostics.lock().unwrap(),expected_render_sequence)
    }

    pub fn debug_headlight_depth_hashes(&self,gpu:&Gpu,expected_render_sequence:Option<u64>)->Result<Vec<String>,String> {
        let state=self.headlight_diagnostics.lock().unwrap();
        let context=state.context.ok_or("no submitted headlight diagnostic frame")?;
        if expected_render_sequence.is_some_and(|s|s!=context.render_sequence){return Err("stale headlight depth request".into());}
        self.headlights.depth_hashes(gpu,state.passes.len())
    }

    /// Explicit current-frame request/caster accounting, never a gameplay wait.
    pub fn receiver_shadow_demand_diagnostics(&self,gpu:&Gpu)
        ->Result<crate::receiver_shadow_demand::Diagnostics,String> {
        self.receiver_demand.diagnostics(gpu)
    }
    /// Raw maps for the requested-texel oracle. A sparse map is not expected to
    /// match reference depth outside requested tiles. Call before another render.
    pub fn debug_sun_depth_textures(&self,expected_render_sequence:Option<u64>)->Result<Vec<wgpu::Texture>,String> {
        let diagnostics=self.sun_diagnostics.lock().unwrap();
        let context=diagnostics.context.ok_or("no submitted solar diagnostic frame")?;
        if expected_render_sequence.is_some_and(|s|s!=context.render_sequence){return Err("stale solar depth request".into());}
        let cache=self.shadow_cache.lock().unwrap();
        let mut textures=vec![cache.as_ref().ok_or("solar depth unavailable")?.tex.clone()];
        let near=self.near_shadows.lock().unwrap();
        for pass in &diagnostics.passes {
            if pass.name=="near_sun" {if let Some(i)=pass.view_index {textures.push(near.get(i).ok_or("missing active near shadow")?.tex.clone());}}
        }
        Ok(textures)
    }

    /// Exact current-view occlusion counters. Explicit blocking capture only;
    /// ordinary gameplay uses GPU-side decisions without counter readback.
    pub fn debug_tree_occlusion(&self,gpu:&Gpu,view:usize,expected_render_sequence:Option<u64>)
        ->Result<crate::tree_occlusion::Diagnostics,String> {
        let diagnostics=self.color_diagnostics.lock().unwrap();
        let context=diagnostics.context.ok_or("no submitted colour frame is available")?;
        if expected_render_sequence.is_some_and(|s|s!=context.render_sequence) {return Err("stale tree occlusion diagnostic request".into());}
        let target=diagnostics.views.get(view).ok_or("colour view is not active in last submitted frame")?;
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("tree occlusion counters"),
            size:crate::tree_occlusion::COUNTER_BYTES,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&target.counter,0,&read,0,crate::tree_occlusion::COUNTER_BYTES);
        gpu.queue.submit([enc.finish()]);read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        let result={let mapped=read.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
            let words:&[u32]=bytemuck::cast_slice(&mapped);
            if words[0]>MAX_VISIBLE {return Err("visible-cluster capacity exceeded".into());}
            self.tree_occlusion.decode(words,context.frame_id,context.render_sequence,view).and_then(|mut result|{
                result.instance_groups=Some(self.instance_groups.diagnostics(words)?);Ok(result)
            })};
        read.unmap();
        let mut result=result?;
        let bytes=crate::cluster_traversal::COUNTER_BYTES;
        let traversal_read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("cluster traversal counters"),
            size:bytes,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&target.traversal_counter,0,&traversal_read,0,bytes);
        gpu.queue.submit([enc.finish()]);traversal_read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        let decoded={let mapped=traversal_read.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
            self.cluster_traversal.diagnostics(bytemuck::cast_slice(&mapped))};
        traversal_read.unmap();result.cluster_traversal=Some(decoded?);Ok(result)
    }

    /// Blocking INPUT visibility-buffer coverage, including opaque grass but
    /// before transition/far grass, glass, effects, reconstruction and HUD.
    /// Run at explicit captures only, marking the frame diagnostic in profiles.
    pub fn debug_geometry_pixels(&self,gpu:&Gpu,view:usize,expected_render_sequence:Option<u64>)
        ->Result<crate::geometry_diagnostics::PixelCoverage,String> {
        let (capture,categories)=self.read_geometry_owner_pixels(gpu,view,expected_render_sequence)?;
        let mut pixels=[0u64;5];let mut uncovered=0;
        for &owner in &capture.owners {
            if owner==u32::MAX {uncovered+=1;}else{pixels[categories[owner as usize]as usize]+=1;}
        }
        Ok(crate::geometry_diagnostics::PixelCoverage {frame_id:capture.frame_id,render_sequence:capture.render_sequence,
            view_index:capture.view_index,input_size:capture.input_size,pixels,uncovered})
    }

    /// Explicit blocking source-instance mask at INPUT resolution. Requires
    /// KERN_DIAGNOSTIC_HIDE=none for a baseline, including experimental foliage.
    /// Only opaque visibility is represented: a selected volume has no owner
    /// pixels here. Keep these readbacks outside measured performance frames.
    pub fn debug_geometry_owner_pixels(&self,gpu:&Gpu,view:usize,expected_render_sequence:Option<u64>)
        ->Result<crate::geometry_diagnostics::OwnerPixels,String> {
        self.read_geometry_owner_pixels(gpu,view,expected_render_sequence).map(|(pixels,_)|pixels)
    }

    fn read_geometry_owner_pixels(&self,gpu:&Gpu,view:usize,expected_render_sequence:Option<u64>)
        ->Result<(crate::geometry_diagnostics::OwnerPixels,std::sync::Arc<Vec<crate::scene::GeometryCategory>>),String> {
        let selected=self.debug_visible_clusters(gpu,view,expected_render_sequence)?;
        let diagnostics=self.color_diagnostics.lock().unwrap();
        let context=diagnostics.context.ok_or("no submitted colour frame")?;
        if context.render_sequence!=selected.render_sequence {return Err("colour frame changed during diagnostic".into());}
        let target=diagnostics.views.get(view).ok_or("colour view no longer active")?;
        let texture=target.visibility.as_ref().ok_or("category diagnostics disabled; set KERN_DIAGNOSTIC_HIDE=none for baseline")?;
        let owners=diagnostics.owners.as_ref().ok_or("category diagnostic owner snapshot missing")?;
        // read_rgba8 copies four bytes per texel without colour conversion for
        // R32Uint, stripping row padding. No shader or additional GPU binding.
        let raw=gpu.read_rgba8(texture);
        let pixels=self.geometry_diagnostics.owner_pixels(&raw,target.input_size,&selected.pairs,owners.len())?;
        Ok((crate::geometry_diagnostics::OwnerPixels {frame_id:context.frame_id,render_sequence:context.render_sequence,
            view_index:view,input_size:target.input_size,owners:pixels},owners.clone()))
    }

    fn view_targets_capacity(&self, gpu: &Gpu, label: &str, capacity: u32,camera:bool,light_groups:bool) -> ViewTargets {
        let foliage_selection = buffer(gpu, "per-view foliage representation", bytemuck::cast_slice(&crate::voxel_forest::default_selection()),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: std::mem::size_of::<ViewUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let visible = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("visible"), size: (capacity as u64 + if camera && self.tree_occlusion.history_first {self.tree_occlusion.deferred_capacity as u64*3}else{0})*8, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let counter = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("counter"), size: crate::tree_occlusion::COUNTER_BYTES, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let args = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("args"), size: 80, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        // Separate from writable storage args: indirect dispatch cannot alias a
        // storage-write binding in the same compute pass. No CPU readback.
        let instance_dispatch = ((camera && (self.compact_cull.is_some() || self.spatial_cull.is_some() || self.tree_occlusion.history_first))
            || light_groups).then(|| gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance dispatch"), size: 12,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        let draw_map = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("bucket draw map"), size: capacity as u64 * 4, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let spatial_owners=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("per-view spatial owner scratch"),
            size:if camera&&(self.instance_groups.enabled||self.instance_groups.replacement_enabled){self.instance_groups.scratch_bytes()}else if light_groups{self.instance_groups.owner_scratch_bytes()}else{4},usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let traversal_counter=crate::cluster_traversal::Resources::counter_buffer(gpu);
        let cull_bind = bind(gpu, &self.view_cull_layout, &[(0, uniform.as_entire_binding()), (1, visible.as_entire_binding()), (2, counter.as_entire_binding()), (3, args.as_entire_binding()), (4, draw_map.as_entire_binding()), (5, foliage_selection.as_entire_binding()),
            (6,self.tree_occlusion.reference.as_entire_binding()),(7,wgpu::BindingResource::TextureView(&self.tree_occlusion.empty)),
            (8,self.instance_groups.groups.as_entire_binding()),(9,self.instance_groups.owners.as_entire_binding()),(10,spatial_owners.as_entire_binding()),
            (11,self.sun_owner_buffer.as_entire_binding()),
            (12,self.cluster_traversal.buffer.as_entire_binding()),(13,traversal_counter.as_entire_binding()),
            (14,self.receiver_demand.buffer().as_entire_binding()),(15,self.tree_occlusion.empty_history.as_entire_binding())]);
        let raster_bind = bind(gpu, &self.view_raster_layout, &[(0, uniform.as_entire_binding()), (1, visible.as_entire_binding()), (2, draw_map.as_entire_binding()), (3, args.as_entire_binding()), (4, foliage_selection.as_entire_binding())]);
        ViewTargets { uniform, foliage_selection, visible, counter, args, draw_map, spatial_owners,traversal_counter,instance_dispatch, cull_bind, raster_bind }
    }

    fn cull_and_raster(&self, enc: &mut wgpu::CommandEncoder, gpu: &Gpu, v: &ViewTargets, color: Option<&wgpu::TextureView>, depth: &wgpu::TextureView, ts: &mut u32, names: &mut Vec<&'static str>, name: &'static str, kind: RasterPass) {
        self.cull_and_raster_pass(enc,gpu,v,color,depth,ts,names,name,kind,None,false,None);
    }

    fn cull_and_raster_pass(&self, enc: &mut wgpu::CommandEncoder, gpu: &Gpu, v: &ViewTargets, color: Option<&wgpu::TextureView>, depth: &wgpu::TextureView, ts: &mut u32, names: &mut Vec<&'static str>, name: &'static str, kind: RasterPass, phase_bind:Option<&wgpu::BindGroup>, append:bool,damage:Option<&wgpu::BindGroup>) {
        if !append && matches!(kind,RasterPass::Color) {enc.clear_buffer(&v.traversal_counter,0,None);}
        let scene_bind=match kind {RasterPass::Color=>&self.scene_bind,RasterPass::FarSun=>&self.far_sun_scene_bind,RasterPass::FoliagePage=>&self.foliage_sun_scene_bind,_=>&self.shadow_scene_bind};
        let cull_scene_bind=match kind {RasterPass::FarSun=>&self.far_cull_scene_bind,RasterPass::FoliagePage=>&self.foliage_cull_scene_bind,_=>&self.cull_scene_bind};
        // Reset in command order without allocating upload staging. A tree's
        // appended phase must retain the preceding visible prefix and counters.
        if phase_bind.is_none() || !append { enc.clear_buffer(&v.counter, 0, None); }
        let cull_bind=phase_bind.unwrap_or(&v.cull_bind);
        let tw = |ts: &mut u32| {
            self.query.as_ref().map(|(qs, _, _)| {
                let i = timestamp_pair(ts);
                (qs, i)
            })
        };
        // Camera, solar and headlamp selections share bounds, never visibility.
        // Cached rest-pose pages and split-cache owners retain flat culling.
        let solar=matches!(kind,RasterPass::Sun)&&self.sun_spatial_cull.is_some();
        let lamp=matches!(kind,RasterPass::Headlight)&&self.headlight_spatial_cull.is_some();
        let spatial=(matches!(kind,RasterPass::Color)&&self.spatial_cull.is_some())||solar||lamp;
        let recover=append && phase_bind.is_some() && self.tree_occlusion.history_first;
        let compact = if recover {None}else if lamp {self.headlight_spatial_cull.as_ref()} else if solar {self.sun_spatial_cull.as_ref()} else if spatial {self.spatial_cull.as_ref()}
            else if matches!(kind, RasterPass::Color) && phase_bind.is_none() { self.compact_cull.as_ref() } else { None };
        if recover {
            let t=tw(ts);if t.is_some(){names.push("tree recovery dispatch");}
            {let mut pass=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("tree recovery dispatch"),timestamp_writes:t.map(|(qs,i)|wgpu::ComputePassTimestampWrites {query_set:qs,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
             pass.set_bind_group(0,cull_scene_bind,&[]);pass.set_bind_group(1,cull_bind,&[]);
             pass.set_pipeline(self.prepare_tree_phase.as_ref().unwrap());pass.dispatch_workgroups(1,1,1);}
            enc.copy_buffer_to_buffer(&v.args,0,v.instance_dispatch.as_ref().unwrap(),0,12);
        }
        if let Some((broadphase, dispatch, _)) = compact {
            let t = tw(ts);
            let label=if lamp {"headlight instance group select"}else if solar {"sun instance group select"}else if spatial {"instance group select"}else{"instance broadphase"};
            if t.is_some() { names.push(label); }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(label),
                    timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites {
                        query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
                });
                pass.set_bind_group(0, cull_scene_bind, &[]);
                pass.set_bind_group(1, cull_bind, &[]);
                // Reset phase-local scratch before selection, preserving the
                // previous visible prefix and all phase1 raster IDs.
                if append {pass.set_pipeline(self.prepare_tree_phase.as_ref().expect("tree phase preparation"));pass.dispatch_workgroups(1,1,1);}
                pass.set_pipeline(broadphase);
                let groups = if spatial {self.instance_groups.parents}else{self.instances.div_ceil(64)};
                pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
                pass.set_pipeline(dispatch);
                pass.dispatch_workgroups(1, 1, 1);
            }
            enc.copy_buffer_to_buffer(&v.args, 0, v.instance_dispatch.as_ref().unwrap(), 0, 12);
        }
        {
            let t = tw(ts);
            if t.is_some() {
                names.push(name);
            }
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull"),
                timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
            });
            let cull_pipeline = if damage.is_some() {
                let cached=self.cached_sun_cull.as_ref().expect("GPU forest shadow damage culling");
                if compact.is_some() {&cached.1}else{&cached.0}
            }else{match kind {
                RasterPass::SunStatic(_) => &self.split_sun_cull.as_ref().expect("enabled static sun cull").0,
                RasterPass::SunDynamic(_) => &self.split_sun_cull.as_ref().expect("enabled dynamic sun cull").1,
                _ => if recover {self.recover_tree_clusters.as_ref().unwrap()}else{compact.map_or(&self.cull, |(_, _, cull)| cull)},
            }};
            pass.set_pipeline(cull_pipeline);
            if let Some(mask)=damage {pass.set_bind_group(2,mask,&[]);}
            pass.set_bind_group(0, cull_scene_bind, &[]);
            pass.set_bind_group(1, cull_bind, &[]);
            if append && compact.is_none() && !recover {
                pass.set_pipeline(self.prepare_tree_phase.as_ref().expect("tree phase preparation"));
                pass.dispatch_workgroups(1,1,1);
                pass.set_pipeline(&self.cull);
            }
            if compact.is_some() || recover {
                pass.dispatch_workgroups_indirect(v.instance_dispatch.as_ref().unwrap(), 0);
            } else {
                let n = match kind { RasterPass::SunStatic(n) | RasterPass::SunDynamic(n) => n, _ => self.instances };
                let n=if self.simple_cull {n.div_ceil(64)} else {n};
                pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
            }
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
            names.push(if name=="tree occluder cull" {"tree occluder raster"} else if name=="tree remaining cull" {"tree remaining raster"} else if color.is_some() { "raster" } else if name == "near shadow cull" { "near shadow raster" }
                else if name=="headlight shadow cull" {"headlight shadow raster"}
                else if matches!(kind,RasterPass::SunStatic(_)) {"static sun raster"}
                else if matches!(kind,RasterPass::SunDynamic(_)) {"dynamic sun raster"}
                else if name=="far sun cull" {"far sun raster"} else if name=="foliage page cull" {"foliage page raster"} else { "shadow raster" });
        }
        let bounds=self.direct_bounds.as_ref().map(|b|b.lock().unwrap().counts(self.cluster_buckets));
        let direct = if bounds.is_none() && std::env::var("KERN_DIRECT_DRAWS").as_deref() == Ok("1") {
            let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("direct draw arguments"),size:80,
                usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            enc.copy_buffer_to_buffer(&v.args,0,&read,0,80);
            let pending=std::mem::replace(enc,gpu.device.create_command_encoder(&Default::default()));
            gpu.queue.submit([pending.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let words:Vec<u32>=bytemuck::cast_slice(&read.slice(..).get_mapped_range().expect("draw arguments")).to_vec();
            read.unmap();eprintln!("direct draw {name}: {words:?}");Some(words)
        } else {None};
        let attachments = [color.map(|c| wgpu::RenderPassColorAttachment {
            view: c,
            depth_slice: None,
            resolve_target: None,
            // Empty pixels read as u32::MAX.
            ops: wgpu::Operations { load: if append {wgpu::LoadOp::Load} else {wgpu::LoadOp::Clear(wgpu::Color { r: u32::MAX as f64, g: 0.0, b: 0.0, a: 0.0 })}, store: wgpu::StoreOp::Store },
        })];
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raster"),
            color_attachments: if color.is_some() { &attachments } else { &[] },
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: if damage.is_some() || append || matches!(kind,RasterPass::SunDynamic(_)) {wgpu::LoadOp::Load} else {wgpu::LoadOp::Clear(if color.is_some() { 0.0 } else { 1.0 })}, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: t.map(|(qs, i)| wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if damage.is_some(){self.raster_cached_sun.as_ref().expect("GPU forest cached sun pipeline")}else{match kind {
            RasterPass::Color => &self.raster,
            RasterPass::Sun | RasterPass::SunStatic(_) | RasterPass::SunDynamic(_) => self.raster_sun_depth.as_ref().unwrap_or(&self.raster_depth),
            RasterPass::FarSun => self.raster_far_sun_depth.as_ref().expect("enabled far sun pipeline"),
            RasterPass::FoliagePage => self.raster_far_sun_depth.as_ref().expect("enabled foliage page pipeline"),
            RasterPass::Headlight => &self.raster_depth,
        }});
        if let Some(mask)=damage {pass.set_bind_group(2,mask,&[]);}
        pass.set_bind_group(0, scene_bind, &[]);
        pass.set_bind_group(1, &v.raster_bind, &[]);
        for &bucket in raster_bucket_order(self.cluster_buckets,self.raster_large_first,kind) {
            if let Some(bounds)=bounds {
                let vertices=if self.cluster_buckets {[self.small_cluster_vertices,96,192,384][bucket as usize]}else{384};
                let start=if self.cluster_buckets {bucket as u32*384}else{0};
                pass.draw(start..start+vertices,0..bounds[bucket as usize]);
            } else if let Some(words)=&direct {
                let at=bucket as usize*4;
                pass.draw(words[at+2]..words[at+2]+words[at],words[at+3]..words[at+3]+words[at+1]);
            }else{pass.draw_indirect(&v.args, bucket * 16);}

        }
    }

    /// Invalidate all source depth after an external geometry/material change.
    /// Ordinary dynamic movement uses the narrower internal invalidation.
    pub fn invalidate_shadow(&self) {
        if let Some(cache) = &self.static_sun { cache.lock().unwrap().invalidate(); }
        self.invalidate_dynamic_shadow();
    }

    fn invalidate_dynamic_shadow(&self) {
        let mut cache = self.shadow_cache.lock().unwrap();
        if self.reuse_sun_texture {
            if let Some(cache) = cache.as_mut() { cache.content.invalidate(); }
        } else {
            *cache = None;
        }
    }

    /// Apply live visual controls atomically. Invalid input leaves both the
    /// previous settings and temporal history untouched. Scene resources and
    /// simulation emitters remain available for subsequent re-enabling.
    pub fn set_debug_options(&self, options: crate::debug_settings::DebugRenderOptions) -> Result<(), String> {
        options.validate()?;
        let reset = {
            let mut previous = self.debug_options.lock().unwrap();
            let reset = options.resets_history(*previous);
            *previous = options;
            reset
        };
        if reset {
            if let Some(meadow) = &self.meadow { meadow.invalidate_history(); }
            self.reset_history();
        }
        Ok(())
    }

    pub fn debug_options(&self) -> crate::debug_settings::DebugRenderOptions {
        *self.debug_options.lock().unwrap()
    }

    /// Replace the bounded car-light set atomically; zero lights disables it.
    /// Does not alter the sky, ambient light, simulation or temporal history.
    /// Commit persistent world-space material marks once before rendering all
    /// views. The caller retains pending stamps until this succeeds.
    pub fn update_tracks(&self,gpu:&Gpu,stamps:&[crate::tracks::Stamp])->Result<crate::tracks::Stats,String> {
        if stamps.len()>crate::tracks::MAX_STAMPS_PER_UPDATE {return Err("track update exceeds8192 stamps; upload bounded batches".into());}
        Ok(self.tracks.update(gpu,stamps))
    }
    pub fn clear_tracks(&self,gpu:&Gpu) {self.tracks.clear(gpu);}
    pub fn track_stats(&self)->crate::tracks::Stats {self.tracks.stats()}

    /// Transfer world-space effects when rebuilding the same scene on the
    /// same device. Call between frames, after construction succeeds and before
    /// discarding the old renderer. A different stage must start with fresh
    /// effects instead. Scene-dependent lighting, shadows, canopy, temporal
    /// reconstruction and profiling history deliberately remain fresh.
    ///
    /// Track textures stay bound to the replacement scene and receive retained
    /// CPU pages. Particle/exhaust resources move with their pool clocks, GPU
    /// buffers and self-contained pipelines; no birth replay or readback occurs.
    /// The previous renderer remains valid: its tracks are cleared and it takes
    /// the replacement's prior transient effects (empty after construction).
    pub fn take_runtime_effects_from(&mut self,gpu:&Gpu,previous:&mut Self)->Result<(),String> {
        if self.resource_device!=gpu.device || previous.resource_device!=gpu.device {
            return Err("runtime effects can only transfer between renderers on the supplied GPU device".into());
        }
        self.tracks.take_from(gpu,&mut previous.tracks);
        std::mem::swap(&mut self.exhaust,&mut previous.exhaust);
        std::mem::swap(&mut self.wheel_particles,&mut previous.wheel_particles);
        Ok(())
    }

    /// Distance exponent per shadowed spot, in `set_headlights` order
    /// (Godot's `spot_attenuation`); missing entries are 2 (inverse square).
    /// A negative value is the exponent with the local lights' range window
    /// (Godot's (1 - (d/range)^4)^2) instead of a fade over the last fifth.
    pub fn set_spot_falloff(&self,spot_falloff:&[f32]) {
        *self.spot_falloff.lock().unwrap()=spot_falloff.to_vec();
    }
    /// Soft shadows for the shadowed spots: the light's size in metres
    /// (0, the default: hard). Shadows are sharp at the foot of what casts
    /// them and widen with the distance to it (PCSS).
    pub fn set_spot_softness(&self,light_size_m:f32) {
        *self.spot_softness.lock().unwrap()=light_size_m;
    }
    pub fn set_headlights(&self,lights:&[crate::headlights::SpotLight])->Result<(),String> {
        let validated=crate::headlights::validate(lights)?;
        *self.headlight_state.lock().unwrap()=validated;
        self.headlight_exclusions.lock().unwrap().clear();Ok(())
    }

    /// Per-lamp owner instance ranges (start, count), in set_headlights order.
    /// Reset by set_headlights, so unrelated spotlights never inherit exclusions.
    pub fn set_headlight_exclusions(&self, ranges: &[(u32,u32)]) {
        *self.headlight_exclusions.lock().unwrap() = ranges.iter().copied().take(crate::headlights::MAX_LIGHTS).collect();
    }

    /// Short-range direct spill, independently bounded from shadowed lights.
    /// No shadow target allocation or culling/raster pass is added. Call with
    /// an empty set when emitters expire; invalid sets leave the old set intact.
    pub fn set_local_lights(&self,lights:&[crate::headlights::LocalLight])->Result<(),String> {
        self.set_local_lights_scattering(lights,&[])
    }

    /// `set_local_lights`, plus a scatter weight per light (in order; missing
    /// weights are zero) for `Lighting::volumetric`. Weight 1 scatters the
    /// light's own intensity; surface lighting is unaffected by the weights.
    pub fn set_local_lights_scattering(&self,lights:&[crate::headlights::LocalLight],scatter:&[f32])->Result<(),String> {
        let validated=crate::headlights::validate_local(lights)?;
        self.volumetric.set_lights(&validated,scatter);
        *self.local_light_state.lock().unwrap()=validated;Ok(())
    }

    /// Lights that only scatter in `Lighting::volumetric` and light no
    /// surface: for example narrow visible beams inside a wide floodlight
    /// whose surface light is one `set_local_lights` entry. Together with
    /// the weighted local lights at most 32 scatter (local lights first).
    pub fn set_volumetric_beams(&self,beams:&[crate::headlights::LocalLight])->Result<(),String> {
        let validated=crate::headlights::validate_local(beams)?;
        self.volumetric.set_beams(&validated);Ok(())
    }

    /// Physical receivers in view order (for example each player car), not
    /// camera targets. Orbiting a stationary receiver must not move its sun
    /// projection or density grids. Empty uses each camera position. A supplied
    /// list must match the next frame's views; stale split-screen mappings fail.
    pub fn set_lighting_focus(&self, points:&[Vec3])->Result<(),String> {
        if points.iter().any(|p|!p.is_finite()) {
            return Err("lighting focus must contain finite world positions".into());
        }
        *self.lighting_focus.lock().unwrap()=points.to_vec(); Ok(())
    }

    /// Diagnostic shadow-coverage override. Empty restores the normal physical
    /// lighting anchors. Nonempty points move only the sun maps; canopy grids,
    /// indirect light and camera remain unchanged for matched reference tests.
    /// A supplied list must match the next frame's views. Not a gameplay LOD API.
    pub fn set_sun_shadow_focus(&self, points:&[Vec3])->Result<(),String> {
        if points.iter().any(|p|!p.is_finite()) {
            return Err("diagnostic sun focus must contain finite world positions".into());
        }
        *self.diagnostic_sun_focus.lock().unwrap()=points.to_vec();Ok(())
    }

    /// Replace the shared world sky between frames. The caller bakes off the
    /// render thread; this uploads only the small radiance LUT and commits its
    /// diffuse SH, reflection/background data and direct solar state together.
    /// Geometry, instances, controllers and the race are not reconstructed.
    /// Update sky radiance/rotation without rebuilding geometry, pipelines or texture arrays.
    pub fn update_scene_environment(&mut self, gpu: &Gpu, scene: &Scene, light: &Lighting) {
        let (texture, sh, horizon, scale, limit, rotation) = scene_environment(gpu, scene, light);
        self.sky_view = texture.create_view(&Default::default());
        self.sky_sh = sh; self.sky_horizon = horizon; self.sky_scale = scale;
        self.sky_limit = limit; self.sky_rotation = rotation;
        self.invalidate_dynamic_shadow(); self.reset_history();
    }

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
        self.invalidate_dynamic_shadow();
        self.reset_history();
        Ok(())
    }

    /// One world wind field for every camera, source mesh LOD and sun map.
    /// This changes animation only; static canopy density is not rebuilt.
    /// Call before rendering, using simulation time rather than frame count.
    pub fn set_wind(&self, gpu: &Gpu, state: &crate::wind::WindState) -> Result<(), String> {
        let state_valid=state.validated()?;
        if self.foliage_world.as_ref().is_some_and(|f|state_valid.strength*state_valid.tree_shear>f.max_wind_shear) {
            self.foliage_world_valid.store(false,std::sync::atomic::Ordering::Relaxed);
        }
        if let Some((forest,_))=&self.gpu_forest {forest.set_wind(state)?;}
        if self.wind.update(gpu, state)? && (self.gpu_forest.is_none() || self.legacy_shadow_wind) { self.invalidate_dynamic_shadow(); }
        if let Some(parts) = &self.part_wind {
            // Branch wind moves the shared sun map's casters only when they
            // cast as source: with voxel casting their groups splat 1 m cells
            // there (the near map, redrawn each frame, keeps the sway).
            if parts.update(gpu, state)? && !self.voxel_forest.as_ref().is_some_and(|f|f.casts_sun()) {
                self.invalidate_dynamic_shadow();
            }
        }
        if let Some(meadow) = &self.meadow {
            // Ordinary elapsed time retains correspondence with the previous
            // rendered wind state. Only discontinuous field changes reset it.
            if meadow.set_wind(state)? { self.reset_history(); }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_warm_foliage_assets(&self,gpu:&Gpu){if let(Some(v),Some((r,_)))=(&self.voxel_forest,&self.gpu_forest){v.warm_page_fixture(gpu,r);}}
    #[cfg(test)]
    pub(crate) fn test_tree_deferred_capacity(&mut self,gpu:&Gpu,capacity:u32) {
        assert!(self.cluster_views.lock().unwrap().is_empty());
        self.tree_occlusion=crate::tree_occlusion::Resources::with_history_capacity(gpu,self.cluster_buckets,self.gpu_forest.is_some(),capacity);
    }
    pub fn gpu_forest_diagnostics(&self,gpu:&Gpu)->Result<Option<crate::foliage_runtime::Diagnostics>,String> {
        self.gpu_forest.as_ref().map(|(forest,_)|forest.diagnostics(gpu)).transpose()
    }
    /// Replace the loaded world population without replacing the renderer.
    /// Stable IDs preserve unchanged GPU residency and motion history.
    pub fn replace_gpu_forest_roots(&self,gpu:&Gpu,roots:&[crate::foliage_population::WorldRoot])->Result<crate::foliage_runtime::PopulationStats,String> {
        let prepared=self.gpu_forest_population_seed()?.prepare(roots)?;
        self.commit_gpu_forest_roots(gpu,prepared)
    }
    pub fn gpu_forest_population_seed(&self)->Result<crate::foliage_runtime::PopulationSeed,String> {
        Ok(self.gpu_forest.as_ref().ok_or("scene has no GPU forest")?.0.population_seed())
    }
    pub fn commit_gpu_forest_roots(&self,gpu:&Gpu,prepared:crate::foliage_runtime::PreparedPopulation)->Result<crate::foliage_runtime::PopulationStats,String> {
        let (runtime,_)=self.gpu_forest.as_ref().ok_or("scene has no GPU forest")?;
        runtime.commit_population(gpu,prepared,|changes| {
            if changes.is_empty(){return Ok(());}
            let budget=self.canopies.lock().unwrap()[0].prepare_proxy_edits(changes.iter().map(|c|(c.proxy.meta[0]as usize,&c.source)))?;
            if changes.iter().any(|c|c.proxy.meta[0]>=self.instances || c.source.cluster_count!=0) {return Err("invalid GPU forest root owner".into());}
            let mut first=0;
            while first<changes.len() {
                let mut end=first+1;while end<changes.len() && changes[end].proxy.meta[0]==changes[end-1].proxy.meta[0]+1 {end+=1;}
                let sources=changes[first..end].iter().map(|c|c.source).collect::<Vec<_>>();
                self.write_instances_metadata(gpu,changes[first].proxy.meta[0]as usize,&sources,true,false)?;
                first=end;
            }
            for canopy in self.canopies.lock().unwrap().iter_mut(){canopy.commit_proxy_update(&budget);}
            // Root proxies have zero source clusters, so ordinary instance
            // bounds deliberately skip them. Include the animated hierarchy
            // envelope on population transactions, independently of residency.
            for bounds in [&self.sun_scene_bounds,&self.sun_guard_bounds].into_iter().flatten() {
                let mut bounds=bounds.lock().unwrap();
                for change in changes {if let Some(s)=runtime.root_shadow_bound(change.root) {
                    bounds.sphere(Vec3::from_slice(&s),s[3]);
                }}
            }
            // Root generation and GPU-owned epoch invalidate changed roots only.
            // Do not reset_history(): retained roots/cameras keep their history.
            Ok(())
        })
    }

    /// Shared presentation clock for all rigid actors, shadows and cameras.
    /// Independent of wind strength and simulation input/focus.
    pub fn set_animation_time(&self, seconds: f64) -> Result<(), String> {
        if let Some(animation) = &self.baked_animations {
            if animation.set_time(seconds)? { self.invalidate_dynamic_shadow(); }
        }
        Ok(())
    }

    /// Upload one actor's final object-space skin matrices. Call before render;
    /// one shared compute pass updates color and all shadow views. The palette
    /// must remain within the actor mesh's declared animation bounds.
    pub fn set_skinning_palette(&self, gpu: &Gpu, actor: usize, matrices: &[Mat4]) -> Result<(), String> {
        self.skinning.as_ref().ok_or("scene has no weighted actors")?.set_palette(gpu, actor, matrices)?;
        self.invalidate_dynamic_shadow();
        Ok(())
    }

    /// Upload the complete actor palette set in one submission-ordered write.
    pub fn set_skinning_palettes<'a>(&self, gpu: &Gpu,
        palettes: impl ExactSizeIterator<Item = &'a [Mat4]> + Clone) -> Result<(), String> {
        if palettes.len() == 0 && self.skinning.is_none() { return Ok(()); }
        self.skinning.as_ref().ok_or("scene has no weighted actors")?.set_palettes(gpu, palettes)?;
        self.invalidate_dynamic_shadow();
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

    /// Re-read one shader generation (`KERN_SHADER_DIR`, see shaders.rs).
    /// Opaque, glass, near/far grass and voxel lighting commit together, only
    /// after every active consumer validates against its retained layout.
    /// Failure leaves all live pipelines and temporal histories unchanged.
    /// Wind deformation/layout changes require a renderer rebuild.
    pub fn reload_shaders(&mut self, gpu: &Gpu) -> Result<bool, String> {
        if self.foliage_world.is_some() {
            return Err("experimental world foliage currently requires a renderer rebuild for shader changes; no lighting pipelines were reloaded".into());
        }
        if self.meadow.is_some() {
            return Err("procedural meadow currently requires a renderer rebuild for shader changes; no lighting pipelines were reloaded".into());
        }
        let snapshot = crate::shaders::Snapshot::load()?;
        if snapshot.source("wind_common.wgsl") != self.shader_wind_text {
            return Err("wind_common.wgsl affects raster/cull/motion geometry; rebuild the renderer to change it".into());
        }
        let sources = snapshot.lighting();
        let shade = if sources.shade != self.shade_text {
            Some(crate::shaders::checked(gpu, || {
                let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shade"), source: wgpu::ShaderSource::Wgsl(sources.shade.as_str().into()) });
                let mut constants = self.shade_features.constants(self.rock_variation,self.meadow.as_ref().is_some_and(|m|m.skip_hidden_shade_enabled()));
                constants.push(("SKY_TILES", if self.sky_tiles {1.0} else {0.0}));
                constants.push(("WORLD_TERRAIN_BAKED", if self.world_visibility.baked {1.0} else {0.0}));
                constants.push(("VOXEL_OCCLUSION", if self.voxel_forest.as_ref().is_some_and(|f| f.casts_sun()) && !self.voxel_surface {1.0} else {0.0}));
                constants.push(("VOXEL_SURFACE", if self.voxel_surface {1.0} else {0.0}));
                constants.push(("VOXEL_PASS", if self.voxel_pass.is_some() {1.0} else {0.0}));
                constants.push(("SHADE_CLASS", if self.terrain_shade.is_some() {2.0} else {0.0}));
                constants.push(("SHADE_DIAGNOSTICS", 0.0));
                constants.push(("TERRAIN_COARSE", if self.terrain_coarse.is_some() {1.0} else {0.0}));
                if let Some(m) = std::env::var("KERN_COARSE_FOOTPRINT_M").ok().and_then(|v| v.parse::<f64>().ok()) { constants.push(("COARSE_FOOTPRINT_M", m)); }
                let make = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&self.shade_pl),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {constants:&constants,..Default::default()},
                    cache: gpu.pipeline_cache(),
                });
                (make(if self.simple_lighting {"shade_simple"}else{"shade"}), make("classify"))
            }).map_err(|e| format!("opaque reload: {e}"))?)
        } else { None };
        // No live assignment above or below until all prepares have succeeded.
        let transparency = self.transparency.as_ref().map(|t| t.prepare_reload(gpu, sources.transparency))
            .transpose().map_err(|e| format!("glass reload: {e}"))?.flatten();
        let grass = self.grass.as_ref().map(|g| g.prepare_reload(gpu, sources.grass, sources.grass_selection))
            .transpose().map_err(|e| format!("near grass reload: {e}"))?.flatten();
        let far_grass = self.far_grass.as_ref().map(|g| g.prepare_reload(gpu, sources.far_grass))
            .transpose().map_err(|e| format!("far grass reload: {e}"))?.flatten();
        let voxel = self.voxel_forest.as_ref().map(|v| v.prepare_reload(gpu, sources.voxel))
            .transpose().map_err(|e| format!("voxel reload: {e}"))?.flatten();
        let rebuilt = shade.is_some() || transparency.is_some() || grass.is_some() || far_grass.is_some() || voxel.is_some();
        snapshot.ensure_current()?;
        if !rebuilt { return Ok(false); }
        if let Some((pipeline, classify)) = shade { self.shade = pipeline; self.shade_classify = classify; self.shade_text = sources.shade; }
        if let Some(reload) = transparency { self.transparency.as_mut().unwrap().commit_reload(reload); }
        if let Some(reload) = grass { self.grass.as_mut().unwrap().commit_reload(reload); }
        if let Some(reload) = far_grass { self.far_grass.as_mut().unwrap().commit_reload(reload); }
        if let Some(reload) = voxel { self.voxel_forest.as_mut().unwrap().commit_reload(reload); }
        self.shader_generation += 1;
        self.invalidate_shadow();
        self.reset_history();
        gpu.persist_pipeline_cache();
        Ok(true)
    }

    /// Successful whole-lighting reloads since this renderer was constructed.
    pub fn shader_generation(&self) -> u64 { self.shader_generation }

    /// Live frame-cache counters; steady-frame deltas expose unexpected GPU
    /// resource creation without enabling timestamp profiling or readback.
    pub fn resource_cache_stats(&self) -> ResourceCacheStats {
        let mut stats = self.output_view.lock().unwrap().stats();
        stats.add(self.shadow_view.lock().unwrap().stats());
        for resources in self.frame_resources.lock().unwrap().iter() { stats.add(resources.stats()); }
        if let Some(fx) = self.fx.0.lock().unwrap().as_ref() { stats.add(fx.resource_cache_stats()); }
        stats
    }

    /// Forget TAA history: the next frame starts from scratch (a camera cut).
    pub fn reset_history(&self) {
        self.history.lock().unwrap().clear();
        self.foliage_planners.lock().unwrap().clear();
        let mut up = self.upscaling.lock().unwrap();
        if let Some(motion) = &up.motion { motion.reset(); }
        up.layout.clear();
        up.foliage_selection.clear();
    }

    /// Restart subpixel sampling for a deterministic stationary A/B capture.
    /// Render/profile sequence IDs remain monotonic. Normal camera cuts should
    /// call reset_history() and let the temporal sampling sequence continue.
    pub fn reset_capture_sampling(&self) {
        self.reset_history();
        self.frame.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Opt-in, one shutter policy per view. Empty disables the post effect.
    pub fn set_motion_blur(&self, gpu: &Gpu, settings: &[crate::motion_blur::Settings]) {
        let mut blur=self.motion_blur.lock().unwrap();
        if blur.is_none() && settings.iter().any(|s|s.strength>0.) {
            *blur=Some(crate::motion_blur::Resources::new(gpu));
        }
        if let Some(blur)=blur.as_mut() {blur.settings=settings.to_vec();}
    }

    /// Select an explicit reconstruction mode. Failure keeps the prior mode.
    /// Native allocates no DLSS feature; motion history is optional for blur.
    pub fn set_upscale(&self, gpu: &Gpu, mode: crate::upscale::UpscaleMode) -> Result<(), String> {
        if mode != crate::upscale::UpscaleMode::Native
            && self.meadow.as_ref().is_some_and(|m|m.group_cache_enabled()) {
            return Err("experimental meadow group cache is native-only until page/reference motion history is validated; disable KERN_MEADOW_GROUP_CACHE for DLSS".into());
        }
        let mut up = self.upscaling.lock().unwrap();
        if up.mode == mode { return Ok(()); }
        if mode == crate::upscale::UpscaleMode::Native {
            *up = Upscaling::default();
        } else {
            // Initialize everything before committing a requested mode.
            let backend = crate::dlss::DlssContext::new(gpu)?;
            let motion = crate::motion::MotionRenderer::new(gpu, &self.instance_buffer, self.wind.buffer(), self.instances as usize)?;
            if let Some(forest)=&self.voxel_forest {forest.prepare_temporal(gpu)?;}
            *up = Upscaling { mode, backend: Some(backend), motion: Some(motion),
                status: crate::upscale::UpscaleStatus { mode, views: Vec::new() }, layout: Vec::new(),foliage_selection:Vec::new() };
        }
        if let Some(forest)=&self.voxel_forest {forest.reset_targets();}
        drop(up);
        self.history.lock().unwrap().clear();
        gpu.persist_pipeline_cache();
        Ok(())
    }

    pub fn upscale_status(&self) -> crate::upscale::UpscaleStatus {
        self.upscaling.lock().unwrap().status.clone()
    }

    /// One bounded world-space visual pool, shared by every split view.
    pub fn set_exhaust(&self, gpu: &Gpu, sprites: &[crate::exhaust::Sprite]) -> Result<(), String> {
        self.exhaust.set(gpu, sprites)
    }

    /// Shared visual birth pool; advance using simulation time, never render delta.
    pub fn update_wheel_particles(&self, gpu: &Gpu, simulation_time: f64, births: &[crate::wheel_particles::Birth]) -> Result<(), String> {
        self.wheel_particles.update(gpu, simulation_time, births)
    }
    pub fn reset_wheel_particles(&self) { self.wheel_particles.reset(); }

    /// Render-frame duration, not CPU submission time. Pauses/hitches are bounded.
    pub fn set_frame_time_ms(&self, milliseconds: f32) {
        let ms = if milliseconds.is_finite() { milliseconds.clamp(1.0, 100.0) } else { 1000.0 / 60.0 };
        self.frame_time_ms.store(ms.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Enable delayed GPU pass timestamps. Repeated calls preserve in-flight
    /// samples and reuse the four buffers; unsupported devices return an error.
    pub fn set_async_gpu_profiling(&self, gpu: &Gpu, enabled: bool) -> Result<(), String> {
        if let Some(forward)=&self.minimal_forward {let mut f=forward.lock().unwrap();f.profiler.configure(gpu,enabled)?;f.profiling=enabled;return Ok(());}
        self.async_gpu_profile.lock().unwrap().configure(gpu, enabled)
    }

    /// Constructor CPU attribution, present only with KERN_STARTUP_PROFILE=1.
    /// Image decoding occurs before Renderer::new; queued GPU work is not timed.
    /// With KERN_CPU_SPANS=1: average CPU ms per frame for each phase of
    /// the frame since the last call, and the number of frames.
    pub fn cpu_spans(&self) -> Option<(Vec<(&'static str, f64)>, u64)> { self.cpu_spans.as_ref().map(|c| c.take()) }

    /// A per-view uniform buffer reused across frames: written with
    /// `write_buffer`, which lands before this frame's commands run and
    /// after the previous frame's (one queue), instead of a new buffer.
    fn view_uniform(&self, gpu: &Gpu, view: usize, slot: usize, label: &str, data: &[u8]) -> wgpu::Buffer {
        let mut all = self.view_uniforms.lock().unwrap();
        if all.len() <= view { all.resize_with(view + 1, Default::default); }
        let entry = &mut all[view][slot];
        if entry.as_ref().is_none_or(|b| b.size() != data.len() as u64) {
            *entry = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: data.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        let b = entry.clone().unwrap();
        gpu.queue.write_buffer(&b, 0, data);
        b
    }

    /// Advance queued GPU debris without shading a frame. Replay seeking uses
    /// this once per recorded tick so spawning/collisions retain their order.
    pub fn advance_replay_debris(&self, gpu: &Gpu) {
        if let Some(debris) = &self.debris {
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            if debris.encode(gpu, &mut enc, None) { gpu.queue.submit(Some(enc.finish())); }
        }
    }

    /// Optional shade-shader features this renderer's scene can reach.
    pub fn shade_features(&self) -> crate::shade_features::ShadeFeatures { self.shade_features }

    pub fn startup_profile(&self) -> Option<&crate::startup_profile::StartupProfile> {
        self.startup_profile.as_ref()
    }

    /// Associate the next render with the caller's frame ID. If not supplied,
    /// the renderer uses its own monotonically increasing render sequence.
    pub fn set_gpu_profile_frame_id(&self, frame_id: u64) {
        if let Some(forward)=&self.minimal_forward {forward.lock().unwrap().profile_frame_id=Some(frame_id);return;}
        self.profile_frame_id.store(frame_id, std::sync::atomic::Ordering::Relaxed);
    }

    /// Collect only completed timestamp copies, including while paused. This
    /// polls callbacks without waiting for a GPU submission to finish.
    pub fn drain_gpu_profiles(&self, gpu: &Gpu) -> crate::gpu_profile::GpuProfileDrain {
        if let Some(forward)=&self.minimal_forward {return forward.lock().unwrap().profiler.drain(gpu);}
        self.async_gpu_profile.lock().unwrap().drain(gpu)
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

    /// An output texture for `try_render_into`, made once per size.
    pub fn output_texture(gpu: &Gpu, out_w: u32, out_h: u32) -> wgpu::Texture {
        gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("frame output"),
            size: wgpu::Extent3d { width: out_w, height: out_h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
    }

    /// Like `try_render_texture`, but into the caller's texture (from
    /// `output_texture`), so a game loop allocates no output per frame.
    /// The texture is overwritten; present or copy it before the next call.
    pub fn try_render_into(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, target: &wgpu::Texture) -> Result<(), String> {
        self.frame_output_into(gpu, views, light, target.width(), target.height(), &[], Readback::None, Some(target)).map(|_| ())
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

    /// Replace the ENTIRE meadow only in an explicit bounded test scene.
    /// The caller supplies an enclosed source field and validates current GPU
    /// publication after every diagnostic render. No interactive game enables
    /// this API; live replacement additionally needs spatial source fallback.
    pub fn install_meadow_brick_fixture(&mut self, gpu: &Gpu,
        source: &crate::meadow_voxel_samples::MeadowVoxelSamples,
        plan: crate::meadow_brick_field::BrickPlan, lod_pixels: f32) -> Result<(), String> {
        if !lod_pixels.is_finite() || lod_pixels < 0.0 {
            return Err("invalid brick projected error".into());
        }
        let meadow = self.meadow.as_ref().ok_or("brick fixture requires source meadow")?;
        let field = meadow.new_brick_field(gpu, source, plan)?;
        let renderer = crate::vegetation_brick_render::VegetationBrickRenderer::new(gpu,
            &self.voxel_scene_layout, &self.volume_view_layout, field.levels(),
            field.levels_buffer(), field.cells_buffer(), field.status_buffer())?;
        self.meadow_brick_fixture = Some(MeadowBrickFixture { field, renderer, lod_pixels,
            generation: std::sync::atomic::AtomicU32::new(0), frozen: false });
        Ok(())
    }

    /// Explicit consumer-only fixture A/B. The caller must first read and
    /// validate publication. Freeze retains the same cells/status/generation;
    /// it is not a production current-pose cache and must be cleared on pose changes.
    pub fn set_meadow_brick_fixture_frozen(&mut self, frozen: bool) -> Result<(), String> {
        let b=self.meadow_brick_fixture.as_mut().ok_or("brick fixture not installed")?;
        if frozen && b.generation.load(std::sync::atomic::Ordering::Relaxed)==0 {
            return Err("cannot freeze brick fixture before first generation".into());
        }
        b.frozen=frozen;Ok(())
    }

    pub fn set_meadow_brick_fixture_empty_skip(&mut self, enabled: bool) -> Result<(), String> {
        self.meadow_brick_fixture.as_mut().ok_or("brick fixture not installed")?
            .renderer.set_empty_skip(enabled);Ok(())
    }

    pub fn set_meadow_brick_fixture_empty_jump(&mut self, enabled: bool) -> Result<(), String> {
        self.meadow_brick_fixture.as_mut().ok_or("brick fixture not installed")?
            .renderer.set_empty_jump(enabled);Ok(())
    }

    pub fn set_meadow_brick_fixture_smooth_mips(&mut self, enabled: bool) -> Result<(), String> {
        self.meadow_brick_fixture.as_mut().ok_or("brick fixture not installed")?
            .renderer.set_smooth_mips(enabled);Ok(())
    }
    pub fn set_meadow_brick_fixture_lod_pixels(&mut self, pixels:f32) -> Result<(),String> {
        if !pixels.is_finite() || pixels<0.0 {return Err("invalid brick projected error".into());}
        self.meadow_brick_fixture.as_mut().ok_or("brick fixture not installed")?.lod_pixels=pixels;Ok(())
    }

    /// Provider buffers are COPY_SRC; readback is explicit and excluded from
    /// GPU frame time. Returns generation, levels, cells, status, allocation.
    pub fn meadow_brick_fixture_buffers(&self) -> Option<(u32, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, u64)> {
        self.meadow_brick_fixture.as_ref().map(|b| (
            b.generation.load(std::sync::atomic::Ordering::Relaxed),
            b.field.levels_buffer().clone(), b.field.cells_buffer().clone(),
            b.field.status_buffer().clone(), b.field.gpu_bytes()))
    }

    pub fn meadow_brick_fixture_diagnostics(&self, gpu: &Gpu)
        -> Result<Vec<crate::vegetation_brick_render::VegetationBrickStats>, String> {
        self.meadow_brick_fixture.as_ref().map(|b| b.renderer.diagnostics(gpu)).unwrap_or(Ok(Vec::new()))
    }

    pub fn meadow_brick_fixture_layers(&self) -> Vec<wgpu::Texture> {
        self.meadow_brick_fixture.as_ref().map(|b| b.renderer.layers()).unwrap_or_default()
    }

    /// Actual source colour/alpha for bounded, explicit optical comparisons.
    /// Includes no GPU wait or readback and does not expose aggregate layers.
    pub fn meadow_source_fixture_layers(&self) -> Vec<wgpu::Texture> {
        self.meadow.as_ref().map(|m|m.source_fixture_layers()).unwrap_or_default()
    }
    /// Explicit capture-only raw aggregate layers, before scene composition or DLSS.
    pub fn meadow_aggregate_raw(&self, gpu: &Gpu) -> Result<Vec<crate::meadow::MeadowAggregateRaw>, String> {
        self.meadow.as_ref().map_or_else(|| Ok(Vec::new()), |m| m.aggregate_raw(gpu))
    }

    /// Explicit offline validation readback; normal frame rendering never waits.
    pub fn voxel_diagnostics(&self, gpu: &Gpu) -> Result<Option<Vec<crate::voxel_forest::VoxelViewDiagnostics>>,String> {
        self.voxel_forest.as_ref().map(|forest|forest.diagnostics(gpu)).transpose()
    }
    /// Explicit experimental native comparison; no interactive default changes.
    pub fn set_aggregate_lod_enabled(&self,enabled:bool) {
        self.aggregate_lod_enabled.store(enabled,std::sync::atomic::Ordering::Relaxed);
        self.reset_history();
    }
    pub fn set_foliage_world_enabled(&self,enabled:bool) {
        self.foliage_world_enabled.store(enabled,std::sync::atomic::Ordering::Relaxed);
    }
    pub fn foliage_world_diagnostics(&self,gpu:&Gpu)->Result<Vec<crate::foliage_world_render::WorldFoliageDiagnostics>,String> {
        self.foliage_world.as_ref().map_or_else(||Ok(Vec::new()),|f|f.diagnostics(gpu))
    }
    pub fn foliage_world_layers_rgba16(&self,gpu:&Gpu)->Result<Vec<(u32,u32,Vec<u8>)>,String> {
        self.foliage_world.as_ref().map_or_else(||Ok(Vec::new()),|f|f.layers(gpu))
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

    /// Explicit validation only: the debris pool's instances as the GPU
    /// last wrote them (waits for the GPU).
    pub fn debris_debug_instances(&self, gpu: &Gpu) -> Result<Vec<crate::scene::GpuInstance>, String> {
        let d = self.debris.as_ref().ok_or("no debris pool")?;
        d.read_instances(gpu, &self.instance_buffer)
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

    fn tree_cull_and_raster(&self,enc:&mut wgpu::CommandEncoder,gpu:&Gpu,v:&ViewTargets,color:&wgpu::TextureView,depth:&wgpu::TextureView,
        index:usize,size:(u32,u32),ts:&mut u32,names:&mut Vec<&'static str>) {
        let mut targets=self.tree_occlusion.targets.lock().unwrap();
        if targets.len()<=index {targets.resize_with(index+1,||None);}
        let pyramid=self.tree_occlusion.pyramid.as_ref().expect("enabled tree pyramid");
        if !targets[index].as_ref().is_some_and(|t|t.size==size) {targets[index]=Some(pyramid.target(gpu,size));}
        let target=targets[index].as_ref().unwrap();
        let history=self.tree_occlusion.history_buffer(gpu,index,self.instances);
        let phases: [wgpu::BindGroup;2]=std::array::from_fn(|phase|bind(gpu,&self.view_cull_layout,&[
            (0,v.uniform.as_entire_binding()),(1,v.visible.as_entire_binding()),(2,v.counter.as_entire_binding()),
            (3,v.args.as_entire_binding()),(4,v.draw_map.as_entire_binding()),(5,v.foliage_selection.as_entire_binding()),
            (6,self.tree_occlusion.phases[phase].as_entire_binding()),(7,wgpu::BindingResource::TextureView(&target.view)),
            (8,self.instance_groups.groups.as_entire_binding()),(9,self.instance_groups.owners.as_entire_binding()),(10,v.spatial_owners.as_entire_binding()),
            (11,self.sun_owner_buffer.as_entire_binding()),
            (12,self.cluster_traversal.buffer.as_entire_binding()),(13,v.traversal_counter.as_entire_binding()),
            (14,self.receiver_demand.buffer().as_entire_binding()),(15,history.as_entire_binding())]));
        self.cull_and_raster_pass(enc,gpu,v,Some(color),depth,ts,names,"tree occluder cull",RasterPass::Color,Some(&phases[0]),false,None);
        let time=self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs,_,_)|{names.push("tree occlusion pyramid");(qs,timestamp_pair(ts))});
        pyramid.encode(gpu,enc,depth,target,time);
        if let Some((qs,i))=time {enc.write_timestamp(qs,i+1);}
        self.cull_and_raster_pass(enc,gpu,v,Some(color),depth,ts,names,"tree remaining cull",RasterPass::Color,Some(&phases[1]),true,None);
        let time=self.query.as_ref().filter(|_|self.tree_occlusion.history_first).map(|(qs,_,_)|{names.push("tree visibility history");(qs,timestamp_pair(ts))});
        self.tree_occlusion.capture_history(gpu,enc,&history,color,&v.visible,size,time);
        if self.tree_occlusion.history_first {enc.copy_buffer_to_buffer(&v.counter,18*4,&history,32772,4);}
    }
    /// Blocking diagnostic only: per-input-pixel opacity-weighted
    /// (previous_world-current_world).xyz and current ray distance.w, RGBA32F.
    /// Empty with native reconstruction; normal frames never read this back.
    pub fn voxel_motion_layers_rgba32(&self,gpu:&Gpu)->Result<Vec<(u32,u32,Vec<u8>)>,String> {
        match &self.voxel_forest {Some(forest)=>forest.motion_layers_rgba32(gpu),None=>Ok(Vec::new())}
    }

    /// Explicit blocking diagnostic for the per-view GPU placement/LOD lists.
    /// Normal gameplay and asynchronous GPU profiling never read these back.
    /// Stream ground samples without replacing the renderer or meadow geometry.
    pub fn update_meadow_ground(&mut self, gpu: &Gpu, origin: [f32; 2], samples: &[[f32; 4]]) -> Result<(), String> {
        self.meadow.as_mut().ok_or("scene has no meadow")?.update_ground(gpu, origin, samples)
    }

    pub fn meadow_diagnostics(&self,gpu:&Gpu)->Result<Vec<crate::meadow::MeadowStats>,String> {
        if !self.debug_options().draws_meadow() { return Ok(Vec::new()); }
        match &self.meadow {Some(meadow)=>meadow.diagnostics(gpu),None=>Ok(Vec::new())}
    }

    /// Configured material state with the live meadow toggle applied. False
    /// when no meadow source exists in this scene; never rereads process env.
    pub fn meadow_substrate_enabled(&self) -> bool {
        self.meadow_substrate_enabled && self.debug_options().draws_meadow()
    }

    /// Actual configured/live analytic ground AO strength; never a fresh env
    /// query. Zero without a meadow field or when live meadow/count is off.
    pub fn meadow_contact_ao_strength(&self) -> f32 {
        f32::from_bits(self.meadow_substrate_settings.with_debug(self.debug_options()).control[3])
    }
    pub fn meadow_contact_ao_radius(&self) -> f32 {
        self.meadow_substrate_settings.with_debug(self.debug_options()).contact[1]
    }
    pub fn meadow_edge_shadow_stats(&self) -> Option<crate::meadow::MeadowEdgeShadowStats> {
        let mut stats = self.meadow.as_ref()?.edge_shadow_stats()?;
        stats.enabled &= self.debug_options().draws_meadow() && self.meadow_brick_fixture.is_none();
        Some(stats)
    }

    /// Blast effects (`fx::Effects::flush`): the clock, and the live puffs
    /// when they changed.
    /// Sprites for the next frames (`sprites::Sprite`), replacing the last set.
    pub fn set_sprites(&self, gpu: &Gpu, sprites: &[crate::sprites::Sprite]) -> Result<(), String> { self.sprites.set(gpu, sprites) }
    /// Upload a sprite picture (sheets, faces, prompts) under `id`.
    pub fn set_sprite_image(&self, gpu: &Gpu, id: crate::sprites::SpriteImage, picture: &crate::picture::Picture, filter: crate::hud::ImageFilter) { self.sprites.set_picture(gpu, id, picture, filter) }
    /// Use a texture (a `sprites::Canvas`, sRGB premultiplied) as sprite `id`.
    pub fn set_sprite_texture(&self, gpu: &Gpu, id: crate::sprites::SpriteImage, texture: &wgpu::Texture, filter: crate::hud::ImageFilter) { self.sprites.set_texture(gpu, id, texture, filter) }
    pub fn remove_sprite_image(&self, id: crate::sprites::SpriteImage) { self.sprites.remove(id) }

    /// Last per-view smoke resolution/sample decisions; no GPU readback.
    pub fn smoke_quality_stats(&self) -> Vec<crate::fx::SmokeQualityStats> {
        self.fx.0.lock().unwrap().as_ref().map_or_else(Vec::new, |fx| fx.quality_stats())
    }

    /// Compile effect pipelines during loading instead of the first explosion.
    pub fn prepare_effects(&self, gpu: &Gpu) { self.fx.prepare(gpu); }

    pub fn set_effects(&self, gpu: &Gpu, time: f32, puffs: Option<&[crate::fx::Puff]>) -> Result<(), String> {
        self.fx.set(gpu, time, puffs);
        Ok(())
    }

    /// The crowd's mood, flee ring and kills (`crowd::Crowd::flush`).
    pub fn update_crowd(&self, gpu: &Gpu, uniform: &[u8], seats: Option<&[u8]>, alive: Option<(usize, &[u32])>) -> Result<(), String> {
        self.crowd.as_ref().ok_or("this scene reserved no crowd")?.update(gpu, uniform, seats, alive)
    }

    /// Spawns, grid changes and simulation steps for the scene's debris pool
    /// (called by `debris::Debris::flush`).
    #[allow(clippy::too_many_arguments)]
    pub fn update_debris(&self, gpu: &Gpu, spawns: &[crate::debris::Shard], occupancy: Option<(usize, &[u32])>, meta: Option<&crate::debris::Occupancy>,
        steps: u32, reset: bool, active: bool, tuning: &crate::debris::Tuning) -> Result<(), String> {
        if let Some(bounds)=&self.direct_bounds {bounds.lock().unwrap().shards(spawns,reset)?;}
        self.debris.as_ref().ok_or("this scene reserved no debris pool")?.update(gpu, spawns, occupancy, meta, steps, reset, active, tuning)
    }

    /// Update an existing instance range without rebuilding static geometry.
    /// Rewrite materials from `first` (an emissive sign flickering, a lamp
    /// changing colour). Layers and flags may change; the blend set of the
    /// scene (which materials are drawn blended) stays as built.
    pub fn update_materials(&self, gpu: &Gpu, first: usize, materials: &[crate::scene::Material]) -> Result<(), String> {
        if first.checked_add(materials.len()).is_none_or(|end| end > self.material_count) { return Err("material update exceeds uploaded scene".into()); }
        gpu.queue.write_buffer(&self.materials, (first * std::mem::size_of::<crate::scene::Material>()) as u64, bytemuck::cast_slice(materials));
        Ok(())
    }

    /// Stand a full plant (SceneVegetation::append) at `root`: its built root
    /// moved rigidly and at most shrunk. Poses, members, branch volumes and
    /// aggregate groups follow on the GPU from the next frame; no rebuild.
    pub fn move_plant(&self, gpu: &Gpu, plant: &crate::vegetation_asset::MovablePlant, root: Mat4) -> Result<(), String> {
        let parts = self.part_wind.as_ref().ok_or("moved plant has no part wind rig")?;
        let ratio = max_column(root) / max_column(plant.built);
        let delta = root * plant.built.inverse();
        parts.move_assembly(gpu, plant.pose, delta, ratio)?;
        if !plant.groups.is_empty() {
            self.aggregate_lod.as_ref().ok_or("moved plant has no aggregate selection")?
                .move_groups(gpu, plant.groups.clone(), delta, ratio)?;
        }
        self.invalidate_dynamic_shadow();
        Ok(())
    }

    /// How whole-plant proxies (forest volume indices) draw while a moved
    /// full assembly or a card tree stands in for their plant.
    pub fn set_plant_proxies(&self, gpu: &Gpu, volumes: &[(u32, crate::vegetation_asset::ProxyShow)]) -> Result<(), String> {
        if volumes.is_empty() { return Ok(()); }
        let forest = self.voxel_forest.as_ref().ok_or("no voxel forest")?;
        for &(volume, show) in volumes { forest.set_proxy_show(gpu, volume, show)?; }
        self.invalidate_dynamic_shadow();
        Ok(())
    }

    /// Relocate reserved whole-plant proxies, preserving their owner indices
    /// and shared asset data. Branch volumes remain GPU-posed and immutable here.
    pub fn replace_plant_proxy_instances(&self, gpu: &Gpu, first: u32,
        volumes: &[crate::foliage_volume_data::GpuTree], sources: &[crate::scene::GpuInstance]) -> Result<(), String> {
        let forest = self.voxel_forest.as_ref().ok_or("no voxel forest")?;
        forest.validate_proxy_update(first, volumes)?;
        if volumes.is_empty() { return Ok(()); }
        let owner = volumes[0].meta[0] as usize;
        if sources.len() != volumes.len() || volumes.iter().enumerate().any(|(i,v)| v.meta[0] as usize != owner+i) ||
            sources.iter().any(|s| s.cluster_count != 0 || !s.rows.iter().flatten().chain(s.sphere.iter()).all(|x| x.is_finite())) {
            return Err("proxy owners must be contiguous, finite, geometry-free instances".into());
        }
        let budget = self.canopies.lock().unwrap()[0].prepare_proxy_update(owner, sources)?;
        self.write_instances_inner(gpu, owner, sources, true)?;
        for canopy in self.canopies.lock().unwrap().iter_mut() {canopy.commit_proxy_update(&budget);}
        gpu.queue.write_buffer(forest.tree_buffer(), first as u64 * std::mem::size_of::<crate::foliage_volume_data::GpuTree>() as u64, bytemuck::cast_slice(volumes));
        // Relocated volume IDs must not reproject the previous tree's pixels.
        self.reset_history();
        Ok(())
    }

    /// Update a same-sized baked terrain sun field without a new renderer.
    pub fn update_baked_terrain_sun(&mut self, gpu: &Gpu, hf: crate::scene::Heightfield, sun: &[f32]) -> Result<(), String> {
        if (self.world_visibility.enabled && !self.world_visibility.baked) || hf.w != self.hf.w || hf.h != self.hf.h || hf.heights.len() != sun.len() ||
            sun.len() != hf.w as usize * hf.h as usize || !hf.cell.is_finite() || hf.cell <= 0.0 ||
            hf.origin.iter().chain(hf.heights.iter()).chain(sun.iter()).any(|x| !x.is_finite()) {
            return Err("baked terrain sun update must preserve dimensions and finite samples".into());
        }
        let data: Vec<[f32;2]> = hf.heights.iter().zip(sun).map(|(&h,&v)| [h,v]).collect();
        gpu.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: self.heightfield.texture(), mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&data), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(hf.w*8), rows_per_image: Some(hf.h) },
            wgpu::Extent3d {width:hf.w,height:hf.h,depth_or_array_layers:1});
        self.hf = hf;
        self.invalidate_shadow();
        Ok(())
    }

    /// Restore the instance array of the same reserved scene during a rewind.
    /// Geometry and pool layout must be unchanged. The caller must also reset
    /// GPU simulation owners (debris/crowd/actors) before the next rendered frame.
    pub fn reset_instance_state(&self, gpu: &Gpu, instances: &[crate::scene::GpuInstance]) -> Result<(), String> {
        if instances.len() != self.instances as usize { return Err("rewind instance layout changed".into()); }
        self.write_instances(gpu, 0, instances)?;
        self.invalidate_dynamic_shadow();
        Ok(())
    }

    pub fn update_instances(&self, gpu: &Gpu, first: usize, instances: &[crate::scene::GpuInstance]) -> Result<(), String> {
        if self.baked_animations.as_ref().is_some_and(|animation|animation.contains_owner_range(first,instances.len())) {
            return Err("baked actor parts are driven by the shared GPU animation pass".into());
        }
        if self.crowd.as_ref().is_some_and(|c| c.contains_owner_range(first, instances.len())) {
            return Err("crowd instances are posed by the GPU crowd pass".into());
        }
        if self.debris.as_ref().is_some_and(|d| d.contains_owner_range(first, instances.len())) {
            return Err("debris instances are written by the GPU debris pass".into());
        }
        if self.procedural_terrain.as_ref().is_some_and(|t| t.contains_owner_range(first, instances.len())) {
            return Err("procedural terrain instances are written by procedural_terrain::ProceduralTerrain".into());
        }
        self.write_instances(gpu, first, instances)
    }

    /// Patch slot updates from `procedural_terrain::ProceduralTerrain`.
    pub fn write_procedural_instances(&self, gpu: &Gpu, first: usize, instances: &[crate::scene::GpuInstance]) -> Result<(), String> {
        let t = self.procedural_terrain.as_ref().ok_or("this scene reserved no procedural terrain")?;
        if !t.contains_owner_range(first, instances.len()) || !t.contains_owner_range(first + instances.len() - 1, 1) {
            return Err("write outside the procedural terrain pool".into());
        }
        self.write_instances(gpu, first, instances)
    }

    /// Validation readback of one procedural terrain slot.
    pub fn procedural_terrain_debug(&self, gpu: &Gpu, slot: u32) -> Option<(Vec<f32>, Vec<u32>, Vec<f32>)> {
        self.procedural_terrain.as_ref().map(|t| t.debug_slot(gpu, slot, &self.instance_buffer))
    }

    /// Queue patches for the GPU build at the start of the next frame.
    pub fn queue_procedural_terrain(&self, jobs: Vec<crate::procedural_terrain::Job>) -> Result<(), String> {
        self.procedural_terrain.as_ref().ok_or("this scene reserved no procedural terrain")?.queue(jobs);
        Ok(())
    }

    fn write_instances(&self, gpu: &Gpu, first: usize, instances: &[crate::scene::GpuInstance]) -> Result<(), String> {
        self.write_instances_inner(gpu, first, instances, false)
    }

    fn write_instances_inner(&self, gpu: &Gpu, first: usize, instances: &[crate::scene::GpuInstance], proxies: bool) -> Result<(), String> {
        self.write_instances_metadata(gpu,first,instances,proxies,true)
    }
    fn write_instances_metadata(&self,gpu:&Gpu,first:usize,instances:&[crate::scene::GpuInstance],proxies:bool,upload:bool)->Result<(),String> {
        if self.part_wind.as_ref().is_some_and(|parts|parts.contains_owner_range(first,instances.len())) {
            return Err("procedural part owners are driven by shared GPU poses, not CPU instance updates".into());
        }
        if first.checked_add(instances.len()).is_none_or(|end|end>self.instances as usize) { return Err("instance update exceeds uploaded scene".into()); }
        if !proxies && self.voxel_forest.as_ref().is_some_and(|forest|forest.contains_owner_range(first,instances.len())) {
            return Err("voxel test trees are static; rebuild their packed scene before changing transforms".into());
        }
        if self.foliage_world.as_ref().is_some_and(|forest|forest.contains_owner_range(first,instances.len())) {
            // Ordinary moving cars do not invalidate the immutable forest.
            // Changed source plants retain meshes until their provider is rebuilt.
            self.foliage_world_valid.store(false,std::sync::atomic::Ordering::Relaxed);
        }
        if !proxies { self.canopies.lock().unwrap()[0].validate_update(first, instances)?; }
        if let Some(bounds)=&self.direct_bounds {bounds.lock().unwrap().update(first,instances)?;}
        if let Some(forward)=&self.minimal_forward {forward.lock().unwrap().update(first,instances)?;}
        if let Some(cache) = &self.static_sun { cache.lock().unwrap().update_instances(gpu,first,instances)?; }
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
            self.geometry_diagnostics.update(first,instances);
            if let Some(bounds) = &self.sun_scene_bounds {
                bounds.lock().unwrap().extend_instances(instances);
            }
            if let Some(bounds) = &self.sun_guard_bounds {
                bounds.lock().unwrap().extend_instances(instances);
            }
            if upload {gpu.queue.write_buffer(&self.instance_buffer, (first * std::mem::size_of::<crate::scene::GpuInstance>()) as u64, bytemuck::cast_slice(instances));}
            self.far_sun.update(gpu,first,instances);
            self.foliage_sun_pages.update(gpu,first,instances);
            if let Some(transparency) = &self.transparency { transparency.update_instances(first, instances); }
            self.invalidate_dynamic_shadow();
        }
        Ok(())
    }

    /// Render a plain minimum-profile scene directly into one full-size colour
    /// attachment per view. External owners (e.g. OpenXR) retain the textures.
    pub fn try_render_eye_targets(&self,gpu:&Gpu,views:&[ViewRect],light:&Lighting,targets:&[&wgpu::Texture])->Result<FrameStats,String> {
        let forward=self.minimal_forward.as_ref().ok_or("direct eye targets require the minimum renderer")?;
        if views.is_empty()||views.len()>8||views.len()!=targets.len()||views.iter().zip(targets).any(|(v,t)|
            v.w==0||v.h==0||(v.w,v.h)!=(t.width(),t.height())||t.depth_or_array_layers()!=1||t.sample_count()!=1||
            !matches!(t.format(),wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb)||!t.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT)) {
            return Err("direct eyes require matching single-layer full-size RGBA8 colour attachments".into());
        }
        let axes=self.camera_up_axes.lock().unwrap();let projections=self.camera_projections.lock().unwrap();
        let camera_views:Vec<_>=views.iter().enumerate().map(|(vi,v)|camera_view(v.cam,axes.get(vi).copied())).collect();
        let matrices:Vec<_>=views.iter().enumerate().map(|(vi,v)|projections.get(vi).copied().unwrap_or_else(||
            Mat4::perspective_infinite_reverse_rh(v.cam.fov_y,v.w as f32/v.h as f32,v.cam.near))*camera_views[vi]).collect();
        forward.lock().unwrap().render(gpu,views,&matrices,&camera_views,&self.sprites,&self.fx,light,&self.headlight_state.lock().unwrap(),None,Some(targets))
    }

    pub fn foliage_page_stats(&self)->Option<crate::foliage_pages::cache::Stats>{self.voxel_forest.as_ref().and_then(|v|v.page_stats())}
    fn frame_output(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32, picks: &[(u32, u32)], readback: Readback) -> Result<(wgpu::Texture, Vec<u8>, FrameStats), String> {
        self.frame_output_into(gpu, views, light, out_w, out_h, picks, readback, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn frame_output_into(&self, gpu: &Gpu, views: &[ViewRect], light: &Lighting, out_w: u32, out_h: u32, picks: &[(u32, u32)], readback: Readback, target: Option<&wgpu::Texture>) -> Result<(wgpu::Texture, Vec<u8>, FrameStats), String> {
        if let Some(forward)=&self.minimal_forward {
            if views.len()>8 || views.iter().any(|v|v.w==0||v.h==0||v.x.checked_add(v.w).is_none_or(|n|n>out_w)||v.y.checked_add(v.h).is_none_or(|n|n>out_h)) {
                return Err("minimum renderer requires up to eight nonempty views inside its output".into());
            }
            if target.is_some_and(|t|(t.width(),t.height())!=(out_w,out_h)||t.format()!=wgpu::TextureFormat::Rgba8Unorm||!t.usage().contains(wgpu::TextureUsages::COPY_DST)) {
                return Err("minimum renderer requires Renderer::output_texture".into());
            }
            if !picks.is_empty() {return Err("pixel picking is unavailable in the minimum graphics profile".into());}
            let output=target.cloned().unwrap_or_else(||Self::output_texture(gpu,out_w,out_h));
            let axes=self.camera_up_axes.lock().unwrap();let projections=self.camera_projections.lock().unwrap();
            let camera_views:Vec<_>=views.iter().enumerate().map(|(vi,v)|camera_view(v.cam,axes.get(vi).copied())).collect();
            let matrices:Vec<_>=views.iter().enumerate().map(|(vi,v)|projections.get(vi).copied().unwrap_or_else(||
                Mat4::perspective_infinite_reverse_rh(v.cam.fov_y,v.w as f32/v.h as f32,v.cam.near))*camera_view(v.cam,axes.get(vi).copied())).collect();
            let stats=forward.lock().unwrap().render(gpu,views,&matrices,&camera_views,&self.sprites,&self.fx,light,&self.headlight_state.lock().unwrap(),Some(&output),None)?;
            let pixels=if readback==Readback::ImageAndMetrics {gpu.read_rgba8(&output)}else{Vec::new()};
            return Ok((output,pixels,stats));
        }
        // Serialize explicit diagnostic readbacks with writes to reused targets.
        // Errors before submission leave no stale frame advertised as current.
        let mut cpu = crate::cpu_spans::Spans::start(self.cpu_spans.as_ref());
        let mut color_diagnostics = self.color_diagnostics.lock().unwrap();
        color_diagnostics.context = None;
        if let (Some(voxels),Some((forest,_)))=(&self.voxel_forest,&self.gpu_forest){voxels.service_pages(gpu,forest)?;}
        let mut shadow_validation=self.async_shadow_validation.as_ref().map(|state|state.lock().unwrap());
        if let Some(state)=shadow_validation.as_mut() {state.drain(gpu)?;}
        let async_pages=readback==Readback::None && shadow_validation.is_some();
        let geometry_diagnostic=self.geometry_diagnostic_config()?;
        let debug_options = self.debug_options();
        self.bloom.set_view_count(views.len());
        let meadow_substrate_settings = self.meadow_substrate_settings.with_debug(debug_options);
        gpu.queue.write_buffer(&self.meadow_substrate_buffer, 0, bytemuck::bytes_of(&meadow_substrate_settings));
        let render_sequence = self.profile_sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let frame_id = self.profile_frame_id.swap(u64::MAX, std::sync::atomic::Ordering::Relaxed);
        let profile_context = crate::gpu_profile::Context {
            frame_id: if frame_id == u64::MAX { render_sequence } else { frame_id },
            render_sequence, output_size: [out_w, out_h], views: views.len() as u32,
        };
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
        let ldr_usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
        let ldr = match target {
            Some(t) => {
                if (t.width(), t.height()) != (out_w, out_h) || t.format() != wgpu::TextureFormat::Rgba8Unorm || !t.usage().contains(ldr_usage) {
                    return Err("render target must be Rgba8Unorm at the output size with STORAGE_BINDING, COPY_SRC and TEXTURE_BINDING (Renderer::output_texture)".into());
                }
                t.clone()
            }
            None => tex2d("ldr", out_w, out_h, wgpu::TextureFormat::Rgba8Unorm, ldr_usage),
        };
        let ldr_v = self.output_view.lock().unwrap().get(&ldr);
        let mut up = self.upscaling.lock().unwrap();
        let dlss_on = up.mode != crate::upscale::UpscaleMode::Native;
        let blur=self.motion_blur.lock().unwrap();
        // TAA reprojects with these per-pixel vectors too: wind-blown trees
        // otherwise lose their history every frame and shimmer.
        if (blur.is_some() || (light.taa > 0.5 && !dlss_on)) && up.motion.is_none() {
            up.motion=Some(crate::motion::MotionRenderer::new(gpu,&self.instance_buffer,self.wind.buffer(),self.instances as usize)?);
            // Voxel trees write their motion layer from here on (as for DLSS).
            // Where the device's binding limits allow its temporal pipelines;
            // otherwise voxel pixels reproject with the surface behind them.
            if let Some(forest)=&self.voxel_forest {
                if let Err(e)=forest.prepare_temporal(gpu) {eprintln!("voxel motion layer off: {e}");}
                forest.reset_targets();
            }
        }

        if self.meadow_brick_fixture.is_some() && (dlss_on || light.taa > 0.5) {
            return Err("meadow brick fixture supports native spatial frames only".into());
        }
        let motion_on = up.motion.is_some();
        let mode = up.mode;
        let view_layout: Vec<_> = views.iter().map(|v| (v.x, v.y, v.w, v.h)).collect();
        let layout_changed = up.layout != view_layout;
        if layout_changed {
            if let Some(motion) = &up.motion { motion.reset(); }
            up.foliage_selection.clear();
        }
        up.foliage_selection.resize_with(views.len(),Vec::new);
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

        // Anchor lighting to physical receivers. A look direction changes the
        // visible pixels, never the sun/occlusion field at a fixed world point.
        // The old policy remains an explicit diagnostic A/B only.
        let focus = resolve_lighting_focus(views,&self.lighting_focus.lock().unwrap())?;
        let ahead = |c: &Camera| {
            let f = (c.target-c.pos).normalize_or(Vec3::NEG_Z);
            c.pos + Vec3::new(f.x,0.0,f.z).normalize_or_zero()*54.0
        };
        let mut anchors:Vec<_>=if self.legacy_light_lookahead {
            views.iter().map(|v|ahead(&v.cam)).collect()
        } else {focus.clone()};
        let diagnostic_sun=self.diagnostic_sun_focus.lock().unwrap().clone();
        if !diagnostic_sun.is_empty() {anchors=resolve_lighting_focus(views,&diagnostic_sun)?;}
        let centre = anchors.iter().copied().sum::<Vec3>() / views.len().max(1) as f32;
        let spread = anchors.iter().map(|p|p.distance(centre)).fold(0.0f32,f32::max);
        let mut previous_extent = self.shadow_extent.lock().unwrap();
        // Metres the shared sun map covers around the focus (KERN_SHADOW_REACH
        // overrides). Past it the coarse distant visibility takes over, whose
        // dark crown-sphere blobs read as a seam against sharp tree shadows:
        // at 90 m that seam ran through the view from 140 m up (terrain_fly
        // playtest); 180 m costs no measurable time at 1440p.
        let reach = std::env::var("KERN_SHADOW_REACH").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(180.0);
        let required_extent = if self.sun_cache_guard_m>0.0 {
            (reach+spread+self.sun_cache_guard_m)/crate::static_sun::FULL_COVERAGE_NDC
        } else {reach+spread};
        let extent = crate::stability::stable_shadow_extent(required_extent, *previous_extent);
        *previous_extent = extent;
        drop(previous_extent);
        let texel = 2.0 * extent / SHADOW_SIZE as f32;
        // One conservative Z range for every sun map, including animated
        // caster bounds. XY coverage/snapping/LOD remain exactly independent.
        let sun_depth = self.sun_scene_bounds.as_ref().map(|bounds| bounds.lock().unwrap().depth_range(light.sun_dir, &anchors));
        let sun_projection = |focus, half_width, resolution| match sun_depth {
            Some(range) => crate::stability::stable_shadow_depth(focus, light.sun_dir, half_width, resolution, range),
            None => crate::stability::stable_shadow(focus, light.sun_dir, half_width, resolution),
        };
        let sun_eye = |focus:Vec3| match sun_depth {
            Some(range) => crate::stability::shadow_eye(focus, light.sun_dir, range),
            None => focus + light.sun_dir * 400.0,
        };
        // This switch is independent of the split cache so its cost can be
        // measured with IDENTICAL guarded projections in both A/B variants.
        // Per-view near maps below keep their original projection and Z range.
        let (shared_centre,guard_recentered)=if self.sun_cache_guard_m>0.0 {
            self.sun_cache_anchor.lock().unwrap().update(&anchors,light.sun_dir,extent,90.0,SHADOW_SIZE)?
        } else {(centre,false)};
        let guard_depth=self.sun_guard_bounds.as_ref().map(|b|b.lock().unwrap().depth_range(light.sun_dir,&anchors));
        let svp=if let Some(range)=guard_depth {
            crate::stability::stable_shadow_depth(shared_centre,light.sun_dir,extent,SHADOW_SIZE,range)
        } else {sun_projection(centre,extent,SHADOW_SIZE)};
        let shared_eye=guard_depth.map_or_else(||sun_eye(centre),|range|crate::stability::shadow_eye(shared_centre,light.sun_dir,range));
        let sun_needed = !self.simple_lighting && sun_visibility_required(self.zero_sun_fast_path, light.sun_lux, light.debug);
        // Snapshot the same lamps used by the shadow passes below. Animation
        // scheduling must see all consumers before any source transforms move.
        let headlights = if debug_options.headlights_enabled {
            self.headlight_state.lock().unwrap().clone()
        } else { Vec::new() };
        let camera_up_axes=self.camera_up_axes.lock().unwrap().clone();
        let camera_projections=self.camera_projections.lock().unwrap().clone();
        let mut part_animation_views=Vec::new();
        if (self.part_wind_lod && self.part_wind.is_some()) || self.gpu_forest.is_some() {
            use crate::part_wind::AnimationView;
            for (vi,view) in views.iter().enumerate() {
                let (w,h)=if let Some(backend)=&mut up.backend {backend.optimal_size(mode,(view.w,view.h))?}
                    else {(view.w,view.h)};
                let mut projection=camera_projections.get(vi).copied().unwrap_or_else(|| Mat4::perspective_infinite_reverse_rh(view.cam.fov_y,view.w as f32/view.h as f32,view.cam.near));
                projection.z_axis.x+=jx*2.0/w as f32;projection.z_axis.y+=jy*2.0/h as f32;
                part_animation_views.push(AnimationView {
                    view_projection:projection*camera_view(view.cam,camera_up_axes.get(vi).copied()),
                    // Preserve the error budget in final output pixels when
                    // the color pass shades at a lower internal resolution.
                    viewport:[view.w,view.h],
                });
            }
            if sun_needed {part_animation_views.push(AnimationView {view_projection:svp,viewport:[SHADOW_SIZE;2]});}
            if light.near_shadows>0.5 && light.shadow_strength>0.0 && light.sun_lux>0.0 {
                for (vi,view) in views.iter().enumerate() {
                    let center=if !diagnostic_sun.is_empty(){anchors[vi]}
                        else if self.legacy_light_lookahead{view.cam.target}else{focus[vi]};
                    part_animation_views.push(AnimationView {
                        view_projection:sun_projection(center,NEAR_SHADOW_EXTENT,NEAR_SHADOW_SIZE),
                        viewport:[NEAR_SHADOW_SIZE;2],
                    });
                }
            }
            for lamp in &headlights {part_animation_views.push(AnimationView {
                view_projection:lamp.shadow_matrix(),viewport:[crate::headlights::SHADOW_SIZE;2],
            });}
            // Far/static foliage maps own immutable copies of their instance
            // buffers and do not consume the animated PartWind transforms.
            // Nor does the shared map with voxel casting (see set_wind): the
            // views change every frame (TAA jitter), which redrew it each frame.
            if self.part_wind.as_ref().map(|p|p.animation_views_changed(&part_animation_views)).transpose()?.unwrap_or(false)
                && !self.voxel_forest.as_ref().is_some_and(|f|f.casts_sun()) {
                self.invalidate_dynamic_shadow();
            }
        }
        let page_views:Vec<_>=views.iter().map(|view|crate::foliage_sun_pages::ReceiverView {
            camera:view.cam,aspect:view.w as f32/view.h as f32,
        }).collect();
        let page_lod=light.lod_pixels.min(self.foliage_sun_pages.lod_pixels);
        // A saturated validation queue defers new pages. Never drop their
        // overflow checks or drain the GPU to make room during gameplay.
        let page_update_budget=if async_pages && !shadow_validation.as_ref().unwrap().available(){0}else{8};
        let page_prepare_started=std::time::Instant::now();
        let page_plan=if sun_needed {
            self.foliage_sun_pages.prepare(gpu,light.sun_dir,light.alpha_cut,page_lod,&page_views,page_update_budget)?
        } else { crate::foliage_sun_pages::Plan::disabled() };
        let page_prepare_cpu_ms=if sun_needed {page_prepare_started.elapsed().as_secs_f64()*1000.} else {0.};
        let mut page_targets=self.foliage_page_targets.lock().unwrap();
        for (index,page) in page_plan.updates.iter().enumerate() {
            let capacity=page.capacity.max(1);
            assert!(capacity<=MAX_VISIBLE,"foliage page exceeds bounded cluster capacity");
            if page_targets.len()<=index {
                page_targets.push((capacity,self.view_targets_capacity(gpu,"foliage sun page",capacity,false,false)));
            } else if page_targets[index].0<capacity {
                page_targets[index]=(capacity,self.view_targets_capacity(gpu,"foliage sun page",capacity,false,false));
            }
            let size=crate::foliage_sun_pages::SIZE as f32;
            let vu=view_uniform(page.vp,page.eye,size/(2.0*page.extent),
                [page_lod,1.0,1.0,capacity as f32],[size;4],light.alpha_cut);
            gpu.queue.write_buffer(&page_targets[index].1.uniform,0,bytemuck::bytes_of(&vu));
        }
        // A generated level bake must not change as adaptive camera quality changes.
        let far_lod=if self.far_sun.stage {self.far_sun.lod_pixels}else{crate::far_sun::effective_lod(self.far_sun.lod_pixels,light.lod_pixels)};
        let far_projection=if sun_needed {self.far_sun.prepare(gpu,light.sun_dir,light.alpha_cut,far_lod)} else {None};
        let far_updated=far_projection.is_some_and(|(_,updated)|updated);
        let mut far_targets=self.far_sun_targets.lock().unwrap();
        if let Some((projection,true))=far_projection {
            let target=far_targets.get_or_insert_with(||self.view_targets_capacity(gpu,"far static sun",self.far_sun.capacity,false,false));
            let size=crate::far_sun::SIZE as f32;
            let mut vu=view_uniform(projection.vp,projection.eye,size/(2.*projection.extent),
                [far_lod,1.,1.,self.far_sun.capacity as f32],[size;4],light.alpha_cut);
            configure_sun_terrain_error(&mut vu,self.sun_terrain_error);
            vu.fog_cull[3]=if self.sun_surface_source{1.}else{0.};
            gpu.queue.write_buffer(&target.uniform,0,bytemuck::bytes_of(&vu));
        }
        // Legacy split maps still redraw moving casters. The normal GPU forest
        // tracks per-tile source/voxel changes after shared materialization.
        if self.gpu_forest.is_some() && self.static_sun.is_some() {self.invalidate_dynamic_shadow();}
        let mut static_sun = self.static_sun.as_ref().map(|s| s.lock().unwrap());
        // Wind freeze: one view-independent radius for every pass (wind.wgsl).
        // More than four views would exceed the freeze uniform: keep wind.
        let cameras: Vec<Vec3> = views.iter().map(|v| v.cam.pos).collect();
        let wind_freeze = self.wind_freeze.filter(|_| cameras.len() <= crate::wind::MAX_FREEZE_CAMERAS);
        if self.wind.set_freeze(gpu, wind_freeze, &cameras) {
            self.invalidate_dynamic_shadow();
        }
        let mut wind_freeze_repinned = false;
        if let (Some(freeze), Some(cache)) = (wind_freeze, static_sun.as_mut()) {
            // Trees >= frozen_beyond + guard from the pinned cameras stay at
            // rest while no camera has moved more than guard: re-pin first.
            if self.sun_cache_guard_m > 0.0 {
                let guard = self.sun_cache_guard_m;
                let mut pin = self.wind_freeze_pin.lock().unwrap();
                let stale = pin.as_ref().is_none_or(|p| p.len() != cameras.len()
                    || p.iter().zip(&cameras).any(|(a, b)| glam::Vec2::new(a.x - b.x, a.z - b.z).length() > guard));
                if stale {
                    cache.pin_frozen_trees(gpu, &cameras, freeze.frozen_beyond + guard);
                    *pin = Some(cameras.clone());
                    wind_freeze_repinned = true;
                }
            }
        } else if let Some(cache) = static_sun.as_mut() {
            // More than four views disables the GPU freeze field. Previously
            // pinned trees must resume dynamic shadows as well.
            let mut pin = self.wind_freeze_pin.lock().unwrap();
            if pin.take().is_some() {
                cache.pin_frozen_trees(gpu, &[], 0.0);
                wind_freeze_repinned = true;
            }
        }
        let mut static_targets = self.static_sun_targets.lock().unwrap();
        let mut cache = self.shadow_cache.lock().unwrap();
        let shadow_lod = light.lod_pixels * 2.0;
        let static_plan = if sun_needed {
            static_sun.as_ref().map(|s| crate::static_sun::SunRasterKey::new(svp,light.alpha_cut,shadow_lod,self.shader_generation)
                .map(|key| s.prepare(key))).transpose()?
        } else { None };
        // Demand changes with cameras even when the light and all casters stay
        // fixed. Old partial depth never certifies newly requested tiles.
        let reuse = sun_needed && !self.receiver_demand.enabled()
            && cache.as_ref().is_some_and(|c| c.content.reusable(svp, light.alpha_cut, shadow_lod));
        let tile_cache_enabled=self.gpu_forest.is_some() && static_sun.is_none();
        let damage_force=cache.as_ref().is_none_or(|c|!c.content.valid || c.content.alpha_cut.to_bits()!=light.alpha_cut.to_bits() || c.content.lod_pixels.to_bits()!=shadow_lod.to_bits()) || !self.reuse_sun_texture
            || self.receiver_demand.enabled() || self.skinning.is_some() || self.crowd.is_some();
        let draw_main_sun = sun_needed && (tile_cache_enabled || !reuse || static_plan.is_some_and(|p|p.redraw_static));
        let draw_static_sun = draw_main_sun && static_plan.is_some_and(|p|p.redraw_static);
        let static_counts = static_sun.as_ref().map_or(Default::default(),|s|s.counts());
        let mut split_stats = FrameStats {
            cluster_buckets:self.cluster_buckets,early_grass_cull:self.early_grass_cull,
            instance_broadphase:self.compact_cull.is_some()&&!self.tree_occlusion.enabled&&!self.instance_groups.enabled,
            instance_groups:self.instance_groups.enabled,sun_instance_groups:self.instance_groups.sun_enabled,cluster_traversal:self.cluster_traversal.enabled,
            headlight_instance_groups:self.instance_groups.headlight_enabled,
            tree_occlusion:self.tree_occlusion.enabled,
            static_sun_cache: static_sun.is_some(), static_sun_redrawn: draw_static_sun,
            static_sun_reused: sun_needed && static_plan.is_some_and(|p|!p.redraw_static),
            static_sun_seed_bytes: if static_sun.is_some() && draw_main_sun {u64::from(SHADOW_SIZE).pow(2)*4}else{0},
            static_sun_gpu_bytes: static_sun.as_ref().map_or(0,|s|s.gpu_bytes()),
            static_sun_casters: static_counts.static_casters, dynamic_sun_casters: static_counts.dynamic_casters,
            sun_cache_guard_m:self.sun_cache_guard_m, sun_cache_guard_recentered:guard_recentered,
            wind_freeze_m: wind_freeze.map_or(0.0, |f| f.frozen_beyond), wind_freeze_repinned,
            shared_sun_extent_m:extent, shared_sun_texel_m:texel,
            ..Default::default()
        };
        if cache.is_none() || (draw_main_sun && !self.reuse_sun_texture) {
            let usage = if static_sun.is_some() {rt|wgpu::TextureUsages::COPY_DST}else{rt};
            *cache = Some(ShadowCache { content: ShadowContent::pending(svp, light.alpha_cut, shadow_lod), tex: tex2d("shadow", SHADOW_SIZE, SHADOW_SIZE, wgpu::TextureFormat::Depth32Float, usage), clusters: 0, triangles: 0, submitted_vertices: 0 });
        }
        if draw_main_sun {
            cache.as_mut().unwrap().content = ShadowContent::pending(svp, light.alpha_cut, shadow_lod);
        } else if !sun_needed {
            // Dawn must redraw, including after a dark frame with no instance
            // uploads and a sun direction numerically equal to the old one.
            cache.as_mut().unwrap().content.invalidate();
        }
        let shadow_v = self.shadow_view.lock().unwrap().get(&cache.as_ref().unwrap().tex);
        let shadow_t = self.view_targets(gpu, "sun view", 0);
        let mut shadow_uniform=view_uniform(svp, shared_eye, SHADOW_SIZE as f32 / (2.0 * extent),
            [shadow_lod, 1.0, 1.0, MAX_VISIBLE as f32], [SHADOW_SIZE as f32; 4], light.alpha_cut);
        configure_sun_terrain_error(&mut shadow_uniform,self.sun_terrain_error);
        shadow_uniform.fog_cull[3]=if self.sun_surface_source{1.0}else{0.0};
        // Positive extra.z is the camera material mip-gradient scale.
        // Reserve negative values for sun demand without changing alpha mips.
        if self.receiver_demand.enabled() {shadow_uniform.extra[2]=-1.0;}
        if draw_main_sun { gpu.queue.write_buffer(&shadow_t.uniform, 0, bytemuck::bytes_of(&shadow_uniform)); }
        // Rasterized voxels cast the replaced branch groups in the shared sun
        // map; their source stops casting there (cull.wgsl, selection w 2).
        if draw_main_sun && static_sun.is_none() && self.voxel_forest.as_ref().is_some_and(|f|f.casts_sun()) {
            gpu.queue.write_buffer(&shadow_t.foliage_selection,0,bytemuck::cast_slice(&crate::voxel_forest::VoxelRenderer::sun_selection()));
        }
        if draw_static_sun {
            let target = static_targets.get_or_insert_with(||self.view_targets_capacity(gpu,"static shared sun",self.shadow_capacity,false,false));
            gpu.queue.write_buffer(&target.uniform,0,bytemuck::bytes_of(&shadow_uniform));
        }
        if let Some(target)=static_targets.as_ref() {
            split_stats.static_sun_gpu_bytes += [&target.uniform,&target.foliage_selection,&target.visible,
                &target.counter,&target.args,&target.draw_map,&target.spatial_owners].iter().map(|b|b.size()).sum::<u64>()
                +target.instance_dispatch.as_ref().map_or(0,|b|b.size());
        }
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
            near_shadow: [2.0 * NEAR_SHADOW_EXTENT / NEAR_SHADOW_SIZE as f32, 0.8, 0.0, if self.raster_sun_depth.is_some() {1.0} else {0.0}],
            night_stars: [light.night_stars[0], light.night_stars[1],
                if self.zero_sun_fast_path {1.0} else {0.0}, light.night_stars[3]],
            anim: [light.time, light.emissive_scale(), 1.2 * 2f32.powf(light.ev100), if light.sun_disc {light.sun_radius} else {0.0}],
            fog_sampling: [((frame % 16) * 7 % 16) as f32 / 16.0, if taa_on { 1.0 } else { 0.0 }, light.indirect_fill_lux.max(0.0), 0.0],
        };
        let exposure = 1.0 / (1.2 * 2f32.powf(light.ev100));

        // Each camera retains its own coverage/resolution. A second player
        // moving away must not change this player's forest illumination.
        let revision = self.foliage_revision.load(std::sync::atomic::Ordering::Relaxed);
        cpu.mark("setup");
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        // wgpu30 requires native API commands on a separate encoder. All
        // tracked and native segments still submit once, in queue order.
        let mut segments = Vec::new();
        let mut ts = 0u32;
        let mut names = Vec::new();
        let mut timing_views = Vec::with_capacity(views.len());
        let mut timing_envelopes = Vec::with_capacity(views.len());
        // Compute once for the shared scene, before all camera and shadow
        // passes. The same frozen state still updates recycled grass slots.
        let wind_time = self.query.as_ref().map(|(qs, _, _)| (qs, ts));
        if self.wind.encode(&mut enc, wind_time) && wind_time.is_some() {
            timestamp_pair(&mut ts);
            names.push("plant wind");
        }
        if let Some(parts) = &self.part_wind {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, ts));
            let encoded=if self.part_wind_lod {parts.encode_with_views(gpu,&mut enc,&part_animation_views,times)?}
                else {parts.encode(gpu,&mut enc,times)};
            if encoded && times.is_some() {
                timestamp_pair(&mut ts);
                names.push("vegetation branch poses");
            }
        }
        if let Some(animation) = &self.baked_animations {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, timestamp_pair(&mut ts)));
            animation.encode(gpu, &mut enc, times);
            if times.is_some() { names.push("baked crowd animation"); }
        }
        if let Some(skinning) = &self.skinning {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, timestamp_pair(&mut ts)));
            skinning.encode(&mut enc, times);
            if times.is_some() { names.push("weighted character skinning"); }
        }
        if let Some(crowd) = &self.crowd {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, ts));
            if crowd.encode(&mut enc, times) && times.is_some() {
                timestamp_pair(&mut ts);
                names.push("crowd");
            }
        }
        if let Some(terrain) = &self.procedural_terrain {
            terrain.encode(gpu, &mut enc);
        }
        if let Some(debris) = &self.debris {
            let times = self.query.as_ref().map(|(qs, _, _)| (qs, ts));
            if debris.encode(gpu, &mut enc, times) && times.is_some() {
                timestamp_pair(&mut ts);
                names.push("debris");
            }
        }
        if let Some((forest,first_owner))=&self.gpu_forest {
            let time=self.query.as_ref().map(|(qs,_,_)| {let first=timestamp_pair(&mut ts);timestamp_pair(&mut ts);timestamp_pair(&mut ts);(qs,first)});
            if time.is_some(){names.extend(["GPU forest selection residency","GPU forest shared poses","GPU forest materialize"]);}
            // Cameras were appended first; all following consumers are shadow
            // maps (including perspective headlights). Do not infer roles from VP.
            let consumers=part_animation_views.iter().copied().enumerate().map(|(i,v)| {
                if i<views.len(){crate::foliage_select::ConsumerView::camera(v)}
                else{crate::foliage_select::ConsumerView::shadow(v)}
            }).collect::<Vec<_>>();
            forest.encode_timed(gpu,&mut enc,&consumers,*first_owner,time)?;
            enc.copy_buffer_to_buffer(&forest.choices,65536,&shadow_t.foliage_selection,0,65536);
        }
        // Actual updated instance/branch spheres and wind bounds, once shared
        // across cameras and solar views. They each select independent owners.
        let group_time=self.query.as_ref().map(|(qs,_,_)|(qs,ts));
        if self.instance_groups.encode(&mut enc,group_time) && group_time.is_some() {
            timestamp_pair(&mut ts);names.push("instance group refit");
        }
        if self.receiver_demand.enabled() && sun_needed {
            use crate::receiver_shadow_demand::{ReceiverCamera,ShadowMap};
            let near_enabled=light.near_shadows>0.5&&light.shadow_strength>0.0&&light.sun_lux>0.0;
            let mut maps=vec![ShadowMap {view_projection:svp,size:SHADOW_SIZE}];
            if near_enabled {for (vi,v) in views.iter().enumerate() {
                let center=if !diagnostic_sun.is_empty(){anchors[vi]}else if self.legacy_light_lookahead{v.cam.target}else{focus[vi]};
                maps.push(ShadowMap {view_projection:sun_projection(center,NEAR_SHADOW_EXTENT,NEAR_SHADOW_SIZE),size:NEAR_SHADOW_SIZE});
            }}
            let mut cameras=Vec::new();let mut extra=Vec::new();
            for (vi,v) in views.iter().enumerate() {
                let (w,h)=if let Some(backend)=&mut up.backend {backend.optimal_size(mode,(v.w,v.h))?}else{(v.w,v.h)};
                let mut proj=camera_projections.get(vi).copied().unwrap_or_else(|| Mat4::perspective_infinite_reverse_rh(v.cam.fov_y,v.w as f32/v.h as f32,v.cam.near));
                proj.z_axis.x+=jx*2.0/w as f32;proj.z_axis.y+=jy*2.0/h as f32;
                let vp=proj*camera_view(v.cam,camera_up_axes.get(vi).copied());
                let mask=1|if near_enabled {1<<(vi+1)}else{0};
                cameras.push(ReceiverCamera {view_projection:vp,map_mask:mask});
                if let Some(meadow)=&self.meadow_receivers {meadow.append(v.cam.pos,vp,0.5*v.h as f32/(v.cam.fov_y*0.5).tan(),mask,debug_options,&mut extra);}
            }
            extra.extend(self.wheel_particles.shadow_receiver_bounds((1u32<<maps.len())-1));
            // Unsupported receiver providers must keep full maps. Registration
            // is explicit; absence from opaque camera depth is not evidence.
            let mut fallback=0u32;
            if self.grass.is_some()||self.far_grass.is_some() {fallback|=1<<8;}
            if self.voxel_forest.is_some()||self.meadow_brick_fixture.is_some() {fallback|=1<<9;}
            if static_sun.is_some() {fallback|=1<<10;}
            if self.part_wind.is_some()||self.baked_animations.is_some()||self.skinning.is_some()||self.debris.is_some()||self.crowd.is_some() {fallback|=1<<11;}
            let time=self.query.as_ref().map(|(qs,_,_)|(qs,timestamp_pair(&mut ts)));
            if time.is_some(){names.push("shadow receiver demand");}
            self.receiver_demand.encode(gpu,&mut enc,(render_sequence as u32).wrapping_add(1).max(1),&cameras,&maps,&extra,fallback,time)?;
        }
        // One perspective map per headlamp, shared by every split view. The
        // shadow bind group contains a different dummy sampled texture so the
        // active depth attachment is never simultaneously bound for reading.
        let local_lights=self.local_light_state.lock().unwrap().clone();
        let spot_falloff=self.spot_falloff.lock().unwrap().clone();
        let softness=*self.spot_softness.lock().unwrap();
        self.headlights.upload(gpu,&headlights,&local_lights,&spot_falloff,softness);
        let mut headlight_targets=self.headlight_targets.lock().unwrap();
        while headlight_targets.len()<headlights.len() {
            headlight_targets.push(self.view_targets_capacity(gpu,"headlight shadow",self.shadow_capacity,false,self.instance_groups.headlight_enabled));
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
            if let Some(&(start,count))=self.headlight_exclusions.lock().unwrap().get(index) {
                vu.grass[2]=start as f32; vu.grass[3]=count as f32;
            }
            gpu.queue.write_buffer(&target.uniform,0,bytemuck::bytes_of(&vu));
            let layer=self.headlights.layer(index as u32);
            self.cull_and_raster(&mut enc,gpu,target,None,&layer,&mut ts,&mut names,"headlight shadow cull",RasterPass::Headlight);
        }
        let mut canopies = self.canopies.lock().unwrap();
        while canopies.len() < views.len() {
            let next = canopies[0].fork_view(gpu);
            canopies.push(next);
        }
        let mut canopy_updated_levels = Vec::with_capacity(views.len());
        let mut canopy_pitch_m = Vec::with_capacity(views.len());
        let mut camera_sun_visibility = Vec::with_capacity(views.len());
        let mut shadow_damage=self.shadow_damage.lock().unwrap();
        let damage_bind=if tile_cache_enabled && draw_main_sun {
            let damage=shadow_damage.as_mut().expect("GPU forest damage cache");
            if damage_force {damage.invalidate();}
            let time=self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(q,_,_)|(q,timestamp_pair(&mut ts)));
            if let Some((q,i))=time {names.push("shadow tile damage and clear");enc.write_timestamp(q,i);}
            damage.encode_scrolled(gpu,&mut enc,svp,self.shader_generation,SHADOW_SIZE,&cache.as_ref().unwrap().tex)?;
            damage.clear_tiles(&mut enc,&shadow_v);
            if let Some((q,i))=time {enc.write_timestamp(q,i+1);}
            Some(damage.raster_bind.clone())
        }else{None};
        if draw_main_sun {
            if let (Some(split),Some(plan)) = (static_sun.as_ref(),static_plan.as_ref()) {
                if plan.redraw_static {
                    let depth = split.texture().create_view(&Default::default());
                    self.cull_and_raster(&mut enc,gpu,static_targets.as_ref().unwrap(),None,&depth,
                        &mut ts,&mut names,"static sun cull",RasterPass::SunStatic(static_counts.static_casters));
                }
                let copy_time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS))
                    .map(|(qs,_,_)|(qs,timestamp_pair(&mut ts)));
                if let Some((qs,index)) = copy_time { names.push("static sun depth seed"); enc.write_timestamp(qs,index); }
                split.encode_seed(&mut enc,&cache.as_ref().unwrap().tex,plan)?;
                if let Some((qs,index)) = copy_time { enc.write_timestamp(qs,index+1); }
                self.cull_and_raster(&mut enc,gpu,&shadow_t,None,&shadow_v,
                    &mut ts,&mut names,"dynamic sun cull",RasterPass::SunDynamic(static_counts.dynamic_casters));
            } else {
                self.cull_and_raster_pass(&mut enc, gpu, &shadow_t, None, &shadow_v, &mut ts, &mut names, "shadow cull",RasterPass::Sun,None,false,damage_bind.as_ref());
                if let Some(forest)=self.voxel_forest.as_ref().filter(|f|f.casts_sun()) {
                    let times=["voxel sun splats","voxel sun splat raster"].map(|name|self.query.as_ref().map(|(qs,_,_)|{
                        let i=timestamp_pair(&mut ts);names.push(name);(qs,i)}));
                    forest.render_sun(gpu,&mut enc,&self.voxel_scene_bind,&shadow_t.foliage_selection,crate::voxel_forest::SunSplatView {
                        damage:damage_bind.as_ref(),depth:&shadow_v,view_proj:svp,planes:shadow_uniform.planes,direction:light.sun_dir,texel_m:sun_voxel_m(texel,SHARED_SUN_VOXEL_M)},times)?;
                }
            }
        }
        if far_updated {
            self.cull_and_raster(&mut enc,gpu,far_targets.as_ref().unwrap(),None,&self.far_sun.target,
                &mut ts,&mut names,"far sun cull",RasterPass::FarSun);
        }
        for (index,page) in page_plan.updates.iter().enumerate() {
            let layer=self.foliage_sun_pages.layer_view(page.layer);
            self.cull_and_raster(&mut enc,gpu,&page_targets[index].1,None,&layer,
                &mut ts,&mut names,"foliage page cull",RasterPass::FoliagePage);
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
                        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC),
                    targets: self.view_targets_capacity(gpu, "near sun view", self.shadow_capacity,false,self.instance_groups.sun_enabled),
                });
            }
        }
        if let Some(grass) = &self.grass { grass.set_view_count(views.len()); }
        if let Some(meadow) = &self.meadow {
            meadow.set_view_count(views.len());
            meadow.set_views(views, render_sequence);
            meadow.set_spatial_filter_temporal_active(dlss_on || taa_on);
        }
        if let Some(grass) = &self.far_grass { grass.set_view_count(views.len()); }
        if let Some(forest) = &self.voxel_forest { forest.set_view_count(views.len()); }
        if let Some(forest) = &self.foliage_world { forest.set_view_count(views.len()); }
        let mut foliage_planners = self.foliage_planners.lock().unwrap();
        if layout_changed { foliage_planners.clear(); }
        foliage_planners.resize_with(views.len(), Default::default);
        cpu.mark("shadows and prepasses");
        let mut keep = Vec::new(); // per view: targets and textures, alive until the readback
        let mut frame_resources = self.frame_resources.lock().unwrap();
        frame_resources.resize_with(views.len(), FrameResources::default);
        for (vi, view) in views.iter().enumerate() {
            let resources = &mut frame_resources[vi];
            let timing_start = names.len();
            let (w, h) = if let Some(backend) = &mut up.backend {
                backend.optimal_size(mode, (view.w, view.h))?
            } else { (view.w, view.h) };
            let cam = &view.cam;
            upscale_views.push(crate::upscale::UpscaleViewStatus { input: [w, h], output: [view.w, view.h] });
            let receivers = if self.legacy_light_lookahead { vec![cam.pos,cam.target,ahead(cam)] }
                else { vec![focus[vi]] };
            let canopy_times = if canopies[vi].needs_update(&receivers, revision) {
                ["canopy volume", "filter canopy"].map(|name| self.query.as_ref().map(|(qs, _, _)| {
                    names.push(name);
                    (qs, timestamp_pair(&mut ts))
                }))
            } else { [None, None] };
            cpu.mark("view start");
            let canopy = canopies[vi].prepare(gpu, &mut enc, &self.scene_bind,
                self.instances, &receivers, revision, canopy_times);
            cpu.mark("view canopy");
            canopy_updated_levels.push(canopy.updated_levels);
            canopy_pitch_m.push(canopy.origins.map(|origin| origin[3]));
            let density_view = canopy.view;
            let gi_u = GiUniform {
                vol_origins: canopy.origins,
                vol_anchors: canopy.anchors,
                vol_dims: [canopy.dims[0], canopy.dims[1], canopy.dims[2], if light.gi>0.0 {1|((canopy.compact_support as u32)<<1)|((self.empty_gi_volume as u32)<<2)
                    |((self.sky_vis_bake.is_some() as u32)<<3)} else {0}],
                hf: [self.hf.origin[0], self.hf.origin[1], self.hf.cell.max(1e-3),
                    if light.contact_occlusion > 0.0 || light.gi > 0.0 { self.dfao.map_or(0.0, |(offset, _)| offset as f32) } else { 0.0 }],
                hf_dims: [self.hf.w.max(2), self.hf.h.max(2), self.solid_count, self.dents.map_or(0, |(offset, _)| offset as u32)],
                // Reflectance, conditioned on local sun and sky visibility.
                bounce: [0.12 * 0.85, 0.12, 0.12 * 0.7, light.sky_lux],
                params: [light.contact_occlusion, light.reflection_occlusion, self.sky_limit,
                    if light.gi > 0.0 { self.irradiance_volume.map_or(0.0, |(offset, _)| offset as f32) } else { 0.0 }],
            };
            let gi_buf = self.view_uniform(gpu, vi, 0, "gi", bytemuck::bytes_of(&gi_u));
            // A rebuilt canopy level gets its sky transmittance baked now,
            // with GI on or off, so the cache is never stale when GI returns.
            if let Some(bake) = &self.sky_vis_bake && canopy.updated_levels != 0 && !self.empty_gi_volume {
                let mut bake_gi = gi_u;
                bake_gi.vol_dims[3] = 1 | ((canopy.compact_support as u32) << 1);
                let bake_gi = self.view_uniform(gpu, vi, 4, "gi bake", bytemuck::bytes_of(&bake_gi));
                let inputs = bind(gpu, &bake.inputs, &[
                    (10, wgpu::BindingResource::TextureView(&self.heightfield)), (11, bake_gi.as_entire_binding()),
                    (13, wgpu::BindingResource::TextureView(&density_view)), (14, wgpu::BindingResource::Sampler(&self.density_sampler))]);
                for level in 0..3u32 {
                    if canopy.updated_levels & (1 << level) == 0 { continue; }
                    let output = bind(gpu, &bake.output, &[
                        (30, wgpu::BindingResource::TextureView(&canopy.scratch)), (31, bake.levels[level as usize].as_entire_binding())]);
                    let time = self.query.as_ref().map(|(qs, _, _)| { names.push("canopy sky transmittance"); (qs, timestamp_pair(&mut ts)) });
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("bake sky transmittance"),
                        timestamp_writes: time.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
                    pass.set_pipeline(&bake.pipeline);
                    pass.set_bind_group(1, &inputs, &[]);
                    pass.set_bind_group(3, &output, &[]);
                    let [x, y, z] = canopy.dims;
                    pass.dispatch_workgroups(x.div_ceil(8), y.div_ceil(8), z.div_ceil(8));
                    drop(pass);
                    let scratch_texture = canopy.scratch.texture();
                    enc.copy_texture_to_texture(scratch_texture.as_image_copy(), wgpu::TexelCopyTextureInfo {
                        texture: &canopy.texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: canopy.dims[1] * (3 + level), z: 0 }, aspect: wgpu::TextureAspect::All },
                        wgpu::Extent3d { width: x, height: y, depth_or_array_layers: z });
                }
            }
            let mut view_light = lu;
            let near_view = if near_enabled {
                let near = &near_shadows[vi];
                let near_centre=if !diagnostic_sun.is_empty() {anchors[vi]}
                    else if self.legacy_light_lookahead {cam.target} else {focus[vi]};
                let near_vp = sun_projection(near_centre, NEAR_SHADOW_EXTENT, NEAR_SHADOW_SIZE);
                let mut near_uniform=view_uniform(
                    near_vp, sun_eye(near_centre), NEAR_SHADOW_SIZE as f32 / (2.0 * NEAR_SHADOW_EXTENT),
                    [light.lod_pixels * near_shadow_lod(), 1.0, 1.0, self.shadow_capacity as f32], [NEAR_SHADOW_SIZE as f32; 4], light.alpha_cut);
                configure_sun_terrain_error(&mut near_uniform,self.sun_terrain_error);
                near_uniform.fog_cull[3]=if self.sun_surface_source{1.0}else{0.0};
                if self.receiver_demand.enabled() {near_uniform.extra[2]=-((vi+2)as f32);}
                gpu.queue.write_buffer(&near.targets.uniform, 0, bytemuck::bytes_of(&near_uniform));
                let texture_view = resources.near_shadow.get(&near.tex);
                // Branch groups with voxels cast them here too, not their
                // source sprays: a full spruce's sprays cost ~2.5 ms each in
                // this 1.6 cm map, its voxels a fraction, at the same coverage
                // (a coarser source cut thins it). KERN_NEAR_SUN_SOURCE=1
                // keeps the source, for A/B runs.
                let voxel_casters=self.voxel_forest.as_ref().is_some_and(|f|f.casts_sun())&&!near_sun_source();
                if voxel_casters {
                    gpu.queue.write_buffer(&near.targets.foliage_selection,0,bytemuck::cast_slice(&crate::voxel_forest::VoxelRenderer::sun_selection()));
                }
                if let Some((forest,_))=&self.gpu_forest {enc.copy_buffer_to_buffer(&forest.choices,65536,&near.targets.foliage_selection,0,65536);}
                self.cull_and_raster(&mut enc, gpu, &near.targets, None, &texture_view, &mut ts, &mut names, "near shadow cull",RasterPass::Sun);
                // Whole-plant proxies have no source to cast here.
                if let Some(forest)=self.voxel_forest.as_ref().filter(|f|f.has_proxies()||voxel_casters) {
                    let times=["voxel near sun splats","voxel near sun splat raster"].map(|name|self.query.as_ref().map(|(qs,_,_)|{
                        let i=timestamp_pair(&mut ts);names.push(name);(qs,i)}));
                    forest.render_sun(gpu,&mut enc,&self.voxel_scene_bind,&near.targets.foliage_selection,crate::voxel_forest::SunSplatView {
                        damage:None,depth:&texture_view,view_proj:near_vp,planes:near_uniform.planes,direction:light.sun_dir,
                        texel_m:sun_voxel_m(2.0*NEAR_SHADOW_EXTENT/NEAR_SHADOW_SIZE as f32,NEAR_SUN_VOXEL_M)},times)?;
                }
                view_light.near_shadow_view_proj = near_vp.to_cols_array_2d();
                view_light.near_shadow[2] = 1.0;
                texture_view
            } else {
                // Valid but unused binding when the feature is disabled.
                shadow_v.clone()
            };
            let light_buf = self.view_uniform(gpu, vi, 1, "view light", bytemuck::bytes_of(&view_light));
            // Per-view targets live across frames while the view keeps its size:
            // allocating three full-screen textures per view per frame cost
            // CPU time and driver memory churn. Every pass writes them fully.
            let (vis, depth, hdr) = {
                let mut targets = self.view_targets.lock().unwrap();
                if targets.len() <= vi { targets.resize_with(vi + 1, || None); }
                match &targets[vi] {
                    Some(t) if t.0 == (w, h) => (t.1.clone(), t.2.clone(), t.3.clone()),
                    _ => {
                        let made = (tex2d("vis", w, h, wgpu::TextureFormat::R32Uint, rt),
                            tex2d("depth", w, h, wgpu::TextureFormat::Depth32Float, rt | wgpu::TextureUsages::COPY_SRC),
                            tex2d("hdr", w, h, wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC));
                        targets[vi] = Some(((w, h), made.0.clone(), made.1.clone(), made.2.clone()));
                        made
                    }
                }
            };
            let (vis_v, depth_v, hdr_v) = (resources.targets[0].get(&vis), resources.targets[1].get(&depth), resources.targets[2].get(&hdr));
            cpu.mark("view lights and targets");
            // Reversed-Z infinite perspective.
            let view_m = camera_view(*cam,camera_up_axes.get(vi).copied());
            let proj = camera_projections.get(vi).copied().unwrap_or_else(|| Mat4::perspective_infinite_reverse_rh(cam.fov_y, view.w as f32 / view.h as f32, cam.near));
            let mut jittered = proj;
            jittered.z_axis.x += jx * 2.0 / w as f32;
            jittered.z_axis.y += jy * 2.0 / h as f32;
            let main = self.view_targets(gpu, "view", vi + 1);
            if let Some((forest,_))=&self.gpu_forest {enc.copy_buffer_to_buffer(&forest.choices,0,&main.foliage_selection,0,65536);}
            let mut foliage_changed=false;
            if let Some(selector)=&self.aggregate_lod {
                let time=self.query.as_ref().map(|(q,_,_)|(q,timestamp_pair(&mut ts)));
                if time.is_some(){names.push("aggregate error selection");}
                selector.encode(gpu,&mut enc,&main.foliage_selection,proj*view_m,[view.w,view.h],
                    self.aggregate_lod_enabled.load(std::sync::atomic::Ordering::Relaxed),time)?;
            } else if !self.foliage_representations.is_empty() {
                // Output-pixel, unjittered selection; every split view owns its
                // own history and the same 4 KiB source/volume decision buffer.
                let decisions = foliage_planners[vi].plan(&self.foliage_representations,
                    crate::foliage_representation::View { view_projection: proj * view_m,
                        width: view.w, height: view.h }, self.foliage_representation_policy)?;
                let mut selection = crate::voxel_forest::default_selection();
                for decision in decisions {
                    selection[decision.volume as usize] = crate::voxel_forest::selection_pack(match decision.representation {
                        crate::foliage_representation::Representation::Source => [0, 0, 0, 0],
                        crate::foliage_representation::Representation::Voxel { level } => [1, level, 0, 0],
                    });
                }
                gpu.queue.write_buffer(&main.foliage_selection, 0, bytemuck::cast_slice(&selection));
                if dlss_on {foliage_changed=update_foliage_history(&mut up.foliage_selection[vi],&selection);}
            }
            // Preserve geometry error in output pixels even at lower shading resolution.
            let px_scale = proj.y_axis.y * 0.5 * view.h as f32;
            let vp = jittered * view_m;
            let mut main_uniform = view_uniform(vp, cam.pos, px_scale, [light.lod_pixels, cam.near, 0.0, MAX_VISIBLE as f32], [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32], cut);
            main_uniform.extra[1] = if views.len() > 1 { 1.0 } else { 0.0 };
            main_uniform.extra[2] = if dlss_on { h as f32 / view.h as f32 } else { 0.0 };
            main_uniform.extra[3] = light.grass_reference;
            let grass_band = self.grass_detail.for_views(views.len());
            main_uniform.grass = [grass_band.fade_start, grass_band.fade_end, (vi + 1) as f32, 0.0];
            let meadow_edge_active = self.meadow.as_ref().is_some_and(|m| m.edge_shadow_stats().is_some_and(|s| s.enabled))
                && self.meadow_brick_fixture.is_none() && debug_options.draws_meadow()
                && geometry_diagnostic.grass_enabled();
            main_uniform.grass[3] = if meadow_edge_active { 1.0 } else { 0.0 };
            main_uniform.fog_cull = camera_fog_cull(light);
            // One allocation-free terrain horizon query per physical camera.
            // Independent of camera rotation, jitter, exposure and shadow-map focus.
            let sun_visibility = if self.camera_sun_occlusion && light.sun_lux > 0.0 {
                crate::terrain_visibility::camera_sun_visibility(&self.hf, cam.pos, light.sun_dir)
            } else { 1.0 };
            main_uniform.fog_cull[2] = sun_visibility;
            main_uniform.fog_cull[3] = geometry_diagnostic.excluded_mask as f32;
            camera_sun_visibility.push(sun_visibility);
            gpu.queue.write_buffer(&main.uniform, 0, bytemuck::bytes_of(&main_uniform));
            cpu.mark("view foliage and camera");
            // TAA history for this view (new, or reset when the size changes).
            let hist = &mut histories[vi];
            let fresh = !hist.as_ref().is_some_and(|hh| hh.size == (w, h));
            if fresh && taa_on {
                let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
                *hist = Some(History { size: (w, h), tex: [tex2d("taa a", w, h, wgpu::TextureFormat::Rgba16Float, usage), tex2d("taa b", w, h, wgpu::TextureFormat::Rgba16Float, usage)], cur: 0, prev_view_proj: proj * view_m });
            }
            let taa_pass = if taa_on {
                let hh = hist.as_mut().unwrap();
                let (src, dst) = (resources.history.get(&hh.tex[hh.cur]), resources.history.get(&hh.tex[1 - hh.cur]));
                let u = TaaUniform {
                    inv_view_proj: vp.inverse().to_cols_array_2d(),
                    prev_view_proj: hh.prev_view_proj.to_cols_array_2d(),
                    size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
                    params: [exposure, light.taa_blend, fresh as u32 as f32, light.taa_gamma],
                };
                hh.prev_view_proj = proj * view_m;
                hh.cur = 1 - hh.cur;
                Some((src, dst, self.view_uniform(gpu, vi, 2, "taa", bytemuck::bytes_of(&u))))
            } else {
                None
            };
            let bloom_strength=(debug_options.bloom_strength*light.bloom_boost.clamp(0.0,32.0)).min(2.0);
            let bloom_on=crate::bloom::active(debug_options.bloom_enabled,bloom_strength);
            let solar = solar_projection(cam, view_m, view.w as f32 / view.h as f32, light, sun_visibility);
            let tone_buf = self.view_uniform(gpu, vi, 3, "tone", bytemuck::bytes_of(&ToneUniform { exposure, contrast: light.contrast, saturation: light.saturation, sharpen: if taa_on { light.sharpen } else { 0.0 }, offset: [view.x, view.y], bloom_strength:if bloom_on {bloom_strength}else{0.0},screen:u32::from(bloom_on&&debug_options.bloom_screen), vignette: light.vignette, grain: light.grain, aberration: light.aberration, time: light.time, sun: solar, sun_tint: [light.sun_color.x, light.sun_color.y, light.sun_color.z, 0.0] }));
            cpu.mark("view setup");
            if self.tree_occlusion.enabled {
                if let Some(selector)=&self.source_replacement {
                    let time=self.query.as_ref().map(|(qs,_,_)| {names.push("world foliage select");(qs,timestamp_pair(&mut ts))});
                    selector.encode(gpu,&mut enc,&self.instance_groups.owners,&main.spatial_owners,self.source_readiness.as_ref().unwrap(),
                        crate::source_replacement::View {view_projection:proj*view_m,output_size:[view.w,view.h],max_error_pixels:self.foliage_world_error_pixels,
                            generation:(render_sequence as u32).wrapping_add(1).max(1),enabled:self.foliage_world_enabled.load(std::sync::atomic::Ordering::Relaxed),
                            native_supported:!dlss_on&&!taa_on&&self.foliage_world_valid.load(std::sync::atomic::Ordering::Relaxed),optical_opt_in:true},time)?;
                }
                self.tree_cull_and_raster(&mut enc,gpu,&main,&vis_v,&depth_v,vi,(w,h),&mut ts,&mut names);
            } else {
                if let Some(selector)=&self.source_replacement {
                    let time=self.query.as_ref().map(|(qs,_,_)| {names.push("world foliage select");(qs,timestamp_pair(&mut ts))});
                    selector.encode(gpu,&mut enc,&self.instance_groups.owners,&main.spatial_owners,self.source_readiness.as_ref().unwrap(),
                        crate::source_replacement::View {view_projection:proj*view_m,output_size:[view.w,view.h],max_error_pixels:self.foliage_world_error_pixels,
                            generation:(render_sequence as u32).wrapping_add(1).max(1),enabled:self.foliage_world_enabled.load(std::sync::atomic::Ordering::Relaxed),
                            native_supported:!dlss_on&&!taa_on&&self.foliage_world_valid.load(std::sync::atomic::Ordering::Relaxed),optical_opt_in:true},time)?;
                }
                self.cull_and_raster(&mut enc, gpu, &main, Some(&vis_v), &depth_v, &mut ts, &mut names, "cull",RasterPass::Color);
            }
            cpu.mark("view cull and raster");
            let meadow_bindings = [
                            (0, main.uniform.as_entire_binding()), (2, light_buf.as_entire_binding()),
                            (5, wgpu::BindingResource::TextureView(&shadow_v)),
                            (6, wgpu::BindingResource::Sampler(&self.shadow_samp)),
                            (7, wgpu::BindingResource::TextureView(&self.sky_view)),
                            (10, wgpu::BindingResource::TextureView(&self.heightfield)),
                            (11, gi_buf.as_entire_binding()), (12, self.solid_occluders.as_entire_binding()),
                            (13, wgpu::BindingResource::TextureView(&density_view)),
                            (14, wgpu::BindingResource::Sampler(&self.density_sampler)),
                            (15, wgpu::BindingResource::TextureView(&near_view)),
                        ];
            if meadow_edge_active {
                let time = self.query.as_ref().map(|(qs, _, _)| {
                    names.push("meadow edge occupancy"); (qs, timestamp_pair(&mut ts))
                });
                self.meadow.as_ref().unwrap().prepare_edge_shadow(gpu, &mut enc, vi, &main.uniform, debug_options, time)?;
            }
            let hidden_meadow=if let Some(meadow)=self.meadow.as_ref().filter(|m|m.skip_hidden_shade_enabled()
                && self.meadow_brick_fixture.is_none() && debug_options.draws_meadow() && geometry_diagnostic.grass_enabled()) {
                let mut times=[None;10];
                for (slot,name) in [(0,"meadow placement"),(6,"meadow visibility")] {
                    times[slot]=self.query.as_ref().map(|(qs,_,_)| {names.push(name);(qs,timestamp_pair(&mut ts))});
                }
                Some(meadow.prepare_visibility(gpu,&mut enc,vi,(w,h),&self.scene_bind,&meadow_bindings,
                    &hdr_v,&depth,times,motion_on,debug_options)?)
            } else {None};
            // Tile list for this view: reset to an empty indirect dispatch.
            let shade_tiles = {
                let tiles = (w.div_ceil(8) * h.div_ceil(8)) as u64;
                let mut all = self.shade_tiles.lock().unwrap();
                if all.len() <= vi { all.resize_with(vi + 1, || None); }
                if all[vi].as_ref().is_none_or(|b| b.0.size() < 16 + 4 * tiles) {
                    all[vi] = Some((gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shade tiles"), size: 16 + 4 * tiles,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                        gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shade tile dispatch"), size: 16,
                        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })));
                }
                let b = all[vi].clone().unwrap();
                gpu.queue.write_buffer(&b.0, 0, bytemuck::cast_slice(&[0u32, 1, 1, 0]));
                b
            };
            let volume_view=(self.far_grass.is_some() || self.voxel_forest.is_some() || self.foliage_world.is_some() || self.meadow_brick_fixture.is_some()).then(||
                crate::volume_view::VolumeViewInputs {
                    camera:&main.uniform, light:&light_buf, opaque_depth:&depth_v,
                    sun_shadow:&shadow_v, shadow_sampler:&self.shadow_samp,
                    sky:&self.sky_view, heightfield:&self.heightfield, gi:&gi_buf,
                    canopy:&density_view, canopy_sampler:&self.density_sampler,
                    near_shadow:&near_view,
                }.bind_cached(gpu,&self.volume_view_layout,&mut resources.volume));
            // Rasterized voxel trees go down before the opaque shade, which
            // then skips the ground and trees they hide (shade.wgsl
            // VOXEL_OCCLUSION); they are resolved and composited after it.
            let voxel_times = self.voxel_forest.as_ref().map(|forest| forest.timing_names(self.voxel_surface).map(|name|{
                let name=name?;
                self.query.as_ref().map(|(qs,_,_)|{let i=timestamp_pair(&mut ts); names.push(name); (qs,i)})
            }));
            if let Some(forest) = &self.voxel_forest {
                forest.render(gpu,&mut enc,vi,(w,h),&self.voxel_scene_bind,
                    volume_view.as_ref().unwrap(),&hdr_v,&main.foliage_selection,
                    up.motion.as_ref().filter(|_|motion_on && forest.has_temporal()).map(|motion|crate::voxel_forest::TemporalInputs {
                        previous_instances:motion.previous_instances(),snapshot_valid:motion.snapshot_valid()}),voxel_times.unwrap(),
                    crate::voxel_forest::Stage::Splats, self.voxel_surface.then_some((&vis_v, &depth_v)))?;
            }
            let voxel_depth = self.voxel_forest.as_ref().and_then(|forest| forest.splat_depth(vi));
            let voxel_surface = self.voxel_forest.as_ref().filter(|_| self.voxel_surface).and_then(|forest| forest.surface(vi));
            let coarse_target = self.terrain_coarse.as_ref().map(|_| {
                let mut all = self.terrain_coarse_targets.lock().unwrap();
                if all.len() <= vi { all.resize_with(vi + 1, || None); }
                let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
                if all[vi].as_ref().is_none_or(|t| t.0.width() != cw || t.0.height() != ch) {
                    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("terrain coarse"),
                        size: wgpu::Extent3d { width: cw, height: ch, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                        dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba32Uint,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING, view_formats: &[] });
                    let view = texture.create_view(&Default::default());
                    all[vi] = Some((texture, view));
                }
                all[vi].clone().unwrap()
            });
            let shade_bind_with = |resources: &mut FrameResources, coarse: &wgpu::TextureView| resources.shade.get(
                gpu,
                &self.shade_layout,
                "cached shade view",
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
                    (16, hidden_meadow.as_ref().unwrap_or(&self.hidden_meadow_fallback).as_entire_binding()),
                    (17, shade_tiles.0.as_entire_binding()),
                    (18, wgpu::BindingResource::TextureView(voxel_depth.as_ref().unwrap_or(&self.voxel_depth_fallback))),
                    (19, wgpu::BindingResource::TextureView(coarse)),
                    (20, wgpu::BindingResource::TextureView(voxel_surface.as_ref().unwrap_or(&self.voxel_surface_fallback))),
                ],
            );
            let shade_bind = shade_bind_with(resources, coarse_target.as_ref().map_or(&self.terrain_coarse_fallback, |t| &t.1));
            let coarse_binds = coarse_target.as_ref().map(|t| (shade_bind_with(resources, &self.terrain_coarse_fallback),
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("terrain coarse out"), layout: &self.terrain_coarse_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 32, resource: wgpu::BindingResource::TextureView(&t.1) }] })));
            let mut passes: Vec<(&'static str, &wgpu::ComputePipeline, Vec<&wgpu::BindGroup>)> = Vec::new();
            if let (Some(coarse), Some((bind, out))) = (&self.terrain_coarse, &coarse_binds) { passes.push(("terrain coarse", coarse, vec![&self.scene_bind, bind, &self.empty_bind, out])); }
            let diagnostic = (light.debug != 0.0 && !self.simple_lighting).then(|| self.shade_diagnostic.get_or_init(|| {
                let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shade diagnostics"),
                    source: wgpu::ShaderSource::Wgsl(self.shade_text.as_str().into()) });
                let make = |constants: &[(&str, f64)]| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("shade diagnostics"), layout: Some(&self.shade_pl), module: &module, entry_point: Some("shade"),
                    compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() }, cache: gpu.pipeline_cache() });
                let terrain: Vec<(&str, f64)> = self.diagnostic_constants.iter().map(|&(k, v)| match k {
                    "FEATURE_CARS" | "FEATURE_FOLIAGE" | "FEATURE_PROCEDURAL" | "FEATURE_GRASS" => (k, 0.0), "FEATURE_TERRAIN" => (k, 1.0),
                    "SHADE_CLASS" => (k, 1.0), _ => (k, v) }).collect();
                (make(&self.diagnostic_constants), self.terrain_shade.as_ref().map(|_| make(&terrain)))
            }));
            // Terrain and voxels first: glossy surfaces in the main pass
            // reflect the already lit frame around them.
            if let Some(terrain) = &self.terrain_shade {
                passes.push(("shade terrain", diagnostic.and_then(|d| d.1.as_ref()).unwrap_or(terrain), vec![&self.scene_bind, &shade_bind]));
            }
            if let Some(voxels) = &self.voxel_pass { passes.push(("shade voxels", voxels, vec![&self.scene_bind, &shade_bind])); }
            passes.push(("shade", diagnostic.map_or(&self.shade, |d| &d.0), vec![&self.scene_bind, &shade_bind]));
            let mut far_motion_layer = None;
            for (label, pipe, bindings) in passes {
                let t = self.query.as_ref().map(|(qs, _, _)| {
                    let i = timestamp_pair(&mut ts);
                    (qs, i)
                });
                if t.is_some() {
                    names.push(label);
                }
                if label == "shade" && self.sky_tiles {
                    // Sky tiles, then the listed tiles; the timestamps span both.
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("shade sky tiles"),
                        timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: None }),
                    });
                    for (k, b) in bindings.iter().enumerate() { pass.set_bind_group(k as u32, *b, &[]); }
                    pass.set_pipeline(&self.shade_classify);
                    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                    drop(pass);
                    enc.copy_buffer_to_buffer(&shade_tiles.0, 0, &shade_tiles.1, 0, 16);
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some(label),
                        timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(i + 1) }),
                    });
                    for (k, b) in bindings.iter().enumerate() { pass.set_bind_group(k as u32, *b, &[]); }
                    pass.set_pipeline(pipe);
                    pass.dispatch_workgroups_indirect(&shade_tiles.1, 0);
                    drop(pass);
                } else {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some(label),
                        timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
                    });
                    for (k, b) in bindings.iter().enumerate() { pass.set_bind_group(k as u32, *b, &[]); }
                    pass.set_pipeline(pipe);
                    // The coarse terrain pass runs one invocation per 2x2 pixels.
                    let (dw, dh) = if label == "terrain coarse" { (w.div_ceil(2), h.div_ceil(2)) } else { (w, h) };
                    pass.dispatch_workgroups(dw.div_ceil(8), dh.div_ceil(8), 1);
                    drop(pass);
                }
                if label == "shade" {
                    let far_grass = self.far_grass.as_ref().filter(|_|geometry_diagnostic.grass_enabled()).map(|grass| {
                        let time = self.query.as_ref().map(|(qs,_,_)| {
                            let i=timestamp_pair(&mut ts); names.push("far grass volume"); (qs,i)
                        });
                        grass.render(gpu,&mut enc,vi,(w,h),&self.voxel_scene_bind,
                            volume_view.as_ref().unwrap(),views.len()>1,time)
                    });
                    if let Some(forest)=&self.foliage_world {
                        let times=["world foliage integrate","world foliage composite"].map(|name|self.query.as_ref().map(|(qs,_,_)|{names.push(name);(qs,timestamp_pair(&mut ts))}));
                        forest.render(gpu,&mut enc,vi,(w,h),&self.voxel_scene_bind,volume_view.as_ref().unwrap(),&hdr_v,
                            &main.spatial_owners,&self.instance_groups.owners,times);
                    }
                    if let Some(forest) = &self.voxel_forest {
                        forest.render(gpu,&mut enc,vi,(w,h),&self.voxel_scene_bind,
                            volume_view.as_ref().unwrap(),&hdr_v,&main.foliage_selection,
                            up.motion.as_ref().filter(|_|motion_on && forest.has_temporal()).map(|motion|crate::voxel_forest::TemporalInputs {
                                previous_instances:motion.previous_instances(),snapshot_valid:motion.snapshot_valid()}),voxel_times.unwrap(),
                            crate::voxel_forest::Stage::Rest, self.voxel_surface.then_some((&vis_v, &depth_v)))?;
                    }
                    if let Some(grass) = self.grass.as_ref().filter(|_|geometry_diagnostic.grass_enabled()) {
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
                        ], &hdr_v, &depth, far_grass.as_ref(), transition_times, composite_times, motion_on, light.grass_reference);
                    }
                    if let Some(brick) = &self.meadow_brick_fixture {
                        if vi == 0 {
                            brick.renderer.set_view_count(views.len());
                        }
                        if vi == 0 && !brick.frozen {
                            let generation = brick.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed).checked_add(1)
                                .ok_or("brick fixture generation exhausted")?;
                            let times = crate::meadow_brick_field::PASS_NAMES.map(|name|self.query.as_ref().map(|(qs,_,_)| {
                                names.push(name); (qs,timestamp_pair(&mut ts))
                            }));
                            self.meadow.as_ref().unwrap().encode_brick_field(gpu, &mut enc,
                                &brick.field, &main.uniform, debug_options, generation, times)?;
                        }
                        let times = ["meadow brick tiles", "meadow brick integrate", "meadow brick composite"]
                            .map(|name|self.query.as_ref().map(|(qs,_,_)| {
                                names.push(name); (qs,timestamp_pair(&mut ts))
                            }));
                        brick.renderer.render(gpu,&mut enc,vi,(w,h),&self.voxel_scene_bind,
                            volume_view.as_ref().unwrap(),&hdr_v,
                            brick.generation.load(std::sync::atomic::Ordering::Relaxed),brick.lod_pixels,times)?;
                    }
                    if let Some(meadow) = self.meadow.as_ref().filter(|_| self.meadow_brick_fixture.is_none()
                        && debug_options.draws_meadow() && geometry_diagnostic.grass_enabled()) {
                        let times = ["meadow placement", "meadow render", "meadow aggregate placement",
                            "meadow aggregate depth", "meadow merge", "meadow composite",
                            "meadow visibility", "meadow resolve", "meadow aggregate material",
                            "meadow aggregate illumination"].map(|name|
                            self.query.as_ref().filter(|_| meadow.aggregate_enabled()
                                || !matches!(name, "meadow aggregate placement" | "meadow aggregate depth"
                                    | "meadow aggregate material" | "meadow aggregate illumination" | "meadow merge"))
                            .filter(|_| match name {
                                "meadow render" => !meadow.visibility_enabled(),
                                "meadow visibility" => meadow.visibility_enabled() && !meadow.skip_hidden_shade_enabled(),
                                "meadow placement" => !meadow.skip_hidden_shade_enabled(),
                                "meadow resolve" => meadow.visibility_enabled(),
                                _ => true,
                            })
                            .map(|(qs,_,_)| {
                                names.push(name); (qs,timestamp_pair(&mut ts))
                            }));
                        if let (Some((_, depth)), Some((_, material)), Some((_, illumination))) = (times[3], times[8], times[9]) {
                            timing_envelopes.push(crate::gpu_profile::Envelope {
                                name: "meadow aggregate render", children: [depth, material, illumination],
                            });
                        }
                        meadow.render(gpu, &mut enc, vi, (w,h), &self.scene_bind, &meadow_bindings, &hdr_v, &depth, times, dlss_on, debug_options)?;
                    }
                    if let Some(transparency) = self.transparency.as_ref().filter(|_|debug_options.transparency_enabled) {
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
                            t.map(|(qs, i)| wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }), motion_on,geometry_diagnostic.excluded_mask);
                    }
                    far_motion_layer = far_grass;
                }
            }
            cpu.mark("view shade and layers");
            let motion_output = if up.motion.is_some() {
                let grass_motion = self.grass.as_ref().filter(|_|geometry_diagnostic.grass_enabled()).and_then(|g| g.motion_layer(vi));
                let glass_motion = self.transparency.as_ref().filter(|_|debug_options.transparency_enabled).and_then(|t| t.motion_layer(vi));
                let voxel_motion = self.voxel_forest.as_ref().and_then(|v|v.motion_layer(vi));
                let meadow_motion = self.meadow.as_ref().filter(|_|debug_options.draws_meadow() && geometry_diagnostic.grass_enabled()).and_then(|g|g.motion_layer(vi));
                let motion_time = self.query.as_ref().map(|(qs, _, _)| {
                    names.push("motion vectors"); (qs, timestamp_pair(&mut ts))
                });
                let motion = up.motion.as_ref().unwrap().render(gpu, &mut enc, vi,
                    crate::motion::MotionView { key: vi as u64, size: (w, h),
                        inverse_jittered_view_proj: vp.inverse(), view_proj: proj * view_m,
                        camera: cam.pos, reset: layout_changed || foliage_changed, split: views.len() > 1,
                        grass_detail: self.grass_detail },
                    crate::motion::MotionInputs { opaque_depth: &depth_v, visibility: &vis_v,
                        visible: &main.visible, grass: grass_motion.as_ref(), transparency: glass_motion.as_ref(),
                        far_grass: far_motion_layer.as_ref().map(|layer| (&layer.color, &layer.distance)),
                        voxel:voxel_motion.as_ref().map(|(color,reprojection)|(color,reprojection)),
                        meadow:meadow_motion.as_ref().map(|(color,reprojection)|(color,reprojection)) }, motion_time)?;
                Some(motion)
            } else {None};
            // TAA after the motion vectors it reprojects with.
            if let (Some((src, dst, u)), Some(motion)) = (taa_pass.as_ref(), motion_output.as_ref()) {
                let vectors = motion.vectors.create_view(&Default::default());
                let bias = motion.bias.create_view(&Default::default());
                let bind = resources.taa.get(gpu, &self.taa_layout, "cached TAA view", &[
                    (0, wgpu::BindingResource::TextureView(&hdr_v)),
                    (1, wgpu::BindingResource::TextureView(&depth_v)),
                    (2, wgpu::BindingResource::TextureView(src)),
                    (3, wgpu::BindingResource::TextureView(dst)),
                    (4, u.as_entire_binding()),
                    (5, wgpu::BindingResource::Sampler(&self.clamp_samp)),
                    (6, wgpu::BindingResource::TextureView(&vectors)),
                    (7, wgpu::BindingResource::TextureView(&bias)),
                ]);
                let t = self.query.as_ref().map(|(qs, _, _)| { names.push("taa"); (qs, timestamp_pair(&mut ts)) });
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("taa"),
                    timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
                pass.set_bind_group(0, &bind, &[]);
                pass.set_pipeline(&self.taa);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            let resolved = if dlss_on {
                let motion=motion_output.as_ref().unwrap();
                let dlss_time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| {
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
                result
            } else if taa_on {
                let hh = hist.as_ref().unwrap(); hh.tex[hh.cur].clone()
            } else { hdr.clone() };
            cpu.mark("view temporal");
            // Compose after temporal reconstruction into a separate output.
            // Never write heat/smoke back into TAA or DLSS history.
            // Blast effects (fire, smoke, embers, heat) over the resolved HDR.
            let fx_output = {
                let mut fx = self.fx.0.lock().unwrap();
                match fx.as_mut() {
                    Some(fx) => {
                        let white = ((light.sun_lux * light.sun_dir.y.max(0.0) + light.sky_lux) / std::f32::consts::PI)
                            .max(light.fx_white_floor.max(0.0) * 1.2 * 2f32.powf(light.ev100));
                        let time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| { names.push("fx"); (qs, timestamp_pair(&mut ts)) });
                        if let Some((qs, i)) = time { enc.write_timestamp(qs, i); }
                        let out = fx.render(gpu, &mut enc, vi, &resolved, &depth_v, *cam, view_m, proj * view_m, vp, white, light, Vec3::from_slice(&self.sky_sh[0][..3]) * 0.282095, self.sky_horizon, camera_sun_visibility[vi], Some((&shadow_v, svp)))?;
                        if let Some((qs, i)) = time { enc.write_timestamp(qs, i + 1); }
                        out
                    }
                    None => None,
                }
            };
            let resolved = fx_output.unwrap_or(resolved);
            // Opt-in in-scattering from local lights, into a separate output.
            let resolved = match light.volumetric.as_ref() {
                Some(medium) => {
                    let time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| { names.push("volumetric"); (qs, timestamp_pair(&mut ts)) });
                    if let Some((qs, i)) = time { enc.write_timestamp(qs, i); }
                    let out = self.volumetric.render(gpu, &mut enc, medium, crate::volumetric::ViewInput { index: vi, input: &resolved,
                        depth: &depth_v, depth_size: (w, h), camera: cam.pos, near: cam.near, depth_vp: vp })?;
                    if let Some((qs, i)) = time { enc.write_timestamp(qs, i + 1); }
                    out.unwrap_or(resolved)
                }
                None => resolved,
            };

            // Opt-in sharp effects: blur the moving world first. Sprite pixels
            // have no rigid-object velocities and otherwise inherit the road's,
            // producing multiple faded copies of one-frame electrical arcs.
            let resolved = if self.sharp_sprites {
                match (blur.as_ref(),motion_output.as_ref()) {
                    (Some(blur),Some(motion)) => {
                        let enabled=!motion.reset && blur.settings.get(vi).is_some_and(|s|s.scale()>0. && s.max_pixels>0.);
                        let times=self.query.as_ref().filter(|_|enabled).map(|(qs,_,_)|{names.push("motion blur");(qs,timestamp_pair(&mut ts))});
                        blur.render(gpu,&mut enc,vi,&resolved,motion,times)
                    },
                    _=>resolved,
                }
            }else{resolved};
            // Sprites (characters, tags, prompts) over the resolved HDR.
            let sprite_output = {
                let mut sprites = self.sprites.0.lock().unwrap();
                match sprites.as_mut() {
                    Some(sprites) => {
                        let time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| { names.push("sprites"); (qs, timestamp_pair(&mut ts)) });
                        if let Some((qs, i)) = time { enc.write_timestamp(qs, i); }
                        let out = sprites.render(gpu, &mut enc, vi, &resolved, &depth_v, *cam, view_m, proj * view_m, vp)?;
                        if let Some((qs, i)) = time { enc.write_timestamp(qs, i + 1); }
                        out
                    }
                    None => None,
                }
            };
            let resolved = sprite_output.unwrap_or(resolved);
            let exhaust_output = if debug_options.exhaust_enabled {
                let exhaust_time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| {
                    names.push("exhaust"); (qs, timestamp_pair(&mut ts))
                });
                if let Some((qs, i)) = exhaust_time { enc.write_timestamp(qs, i); }
                let output = self.exhaust.render(gpu, &mut enc, vi, &resolved, &depth_v,
                    *cam, proj * view_m, vp, light, None)?;
                if let Some((qs, i)) = exhaust_time { enc.write_timestamp(qs, i + 1); }
                output
            } else { None };
            // Reuse disposable exhaust output; a particle-only frame takes one
            // copy instead. Neither path writes to native/DLSS temporal history.
            let particle_output = if debug_options.wheel_particles_enabled {
                let particles_time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| {
                    names.push("wheel particles"); (qs, timestamp_pair(&mut ts))
                });
                if let Some((qs, i)) = particles_time { enc.write_timestamp(qs, i); }
                let output = self.wheel_particles.render(gpu, &mut enc, vi,
                    exhaust_output.as_ref().unwrap_or(&resolved), exhaust_output.is_some(),
                    &depth_v, *cam, proj * view_m, vp, light, crate::wheel_particles::Shadows {
                        far: &shadow_v, near: &near_view, sampler: &self.shadow_samp,
                        far_vp: Mat4::from_cols_array_2d(&view_light.shadow_view_proj),
                        near_vp: Mat4::from_cols_array_2d(&view_light.near_shadow_view_proj),
                        far_texel: view_light.sky[3], near_texel: view_light.near_shadow[0],
                        near_enabled: view_light.near_shadow[2] > 0.5,
                    })?;
                if let Some((qs, i)) = particles_time { enc.write_timestamp(qs, i + 1); }
                output
            } else { None };
            let result=particle_output.as_ref().or(exhaust_output.as_ref()).unwrap_or(&resolved);
            let blurred=match (blur.as_ref(),motion_output.as_ref()) {
                (Some(blur),Some(motion)) if !self.sharp_sprites => {
                    let enabled=!motion.reset && blur.settings.get(vi).is_some_and(|s|s.scale()>0. && s.max_pixels>0.);
                    let times=self.query.as_ref().filter(|_|enabled).map(|(qs,_,_)|{names.push("motion blur");(qs,timestamp_pair(&mut ts))});
                    Some(blur.render(gpu,&mut enc,vi,result,motion,times))
                },
                _=>None,
            };
            let result=blurred.as_ref().unwrap_or(result);

            // Extract the actual final HDR, including lens emission and exhaust,
            // after temporal reconstruction. Every view owns separate targets.
            let bloom_view=if bloom_on {
                let time=self.query.as_ref().map(|(qs,_,_)|{names.push("bloom");(qs,timestamp_pair(&mut ts))});
                self.bloom.render(gpu,&mut enc,vi,result,exposure,debug_options.bloom_threshold.min(light.bloom_threshold.clamp(0.0,16.0)),debug_options.bloom_rounds.max(light.bloom_rounds.clamp(1,crate::bloom::MAX_ROUNDS)),debug_options.bloom_screen,time)
            }else{self.bloom.disabled_view()};
            let result_view = resources.result.get(result);
            let tone_bind = resources.tone.get(gpu, &self.tone_layout, "cached tone view", &[
                (0, wgpu::BindingResource::TextureView(&result_view)),
                (1, wgpu::BindingResource::TextureView(&ldr_v)), (2, tone_buf.as_entire_binding()),
                (3,wgpu::BindingResource::TextureView(&bloom_view)),(4,wgpu::BindingResource::Sampler(&self.bloom.sampler)),(5,wgpu::BindingResource::TextureView(&depth_v))]);
            let t = self.query.as_ref().map(|(qs, _, _)| {
                names.push("tonemap"); (qs, timestamp_pair(&mut ts))
            });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tonemap resolved output"),
                    timestamp_writes: t.map(|(qs, i)| wgpu::ComputePassTimestampWrites {
                        query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
                pass.set_pipeline(&self.tone);
                pass.set_bind_group(0, &tone_bind, &[]);
                pass.dispatch_workgroups(view.w.div_ceil(8), view.h.div_ceil(8), 1);
            }
            keep.push((main, vis, depth, hdr));
            timing_views.push((vi as u32, timing_start..names.len()));
            cpu.mark("view effects, bloom, tonemap");
        }
        if let Some(motion) = &up.motion { motion.finish_frame(&mut enc); }
        // Every view has finished sampling the albedo array. Update shared
        // displays now for the NEXT frame, before app HUD/UI is composed.
        if let Some(screens) = &self.live_screens {
            let mut screens = screens.lock().unwrap();
            if screens.due(views, &ldr) {
                let screen_time = self.query.as_ref().filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)).map(|(qs, _, _)| {
                    names.push("live screens"); (qs, timestamp_pair(&mut ts))
                });
                if let Some((qs, i)) = screen_time { enc.write_timestamp(qs, i); }
                screens.encode(gpu, &mut enc, &ldr, views);
                if let Some((qs, i)) = screen_time { enc.write_timestamp(qs, i + 1); }
                cpu.mark("live screens");
            }
        }
        // One shared previous wind state, advanced only after every view has
        // encoded both meadow color and motion for this rendered frame.
        if let Some(meadow) = &self.meadow { meadow.finish_frame(); }
        up.layout = view_layout;
        up.status = crate::upscale::UpscaleStatus { mode, views: upscale_views };
        let upscale_status = up.status.clone();
        drop(up);
        drop(histories);
        drop(canopies);
        // Resolve once after every native/DLSS segment. The queued copy owns a
        // free ring slot; a full ring skips this sample instead of stalling.
        let timing_specs = crate::gpu_profile::describe_passes(&names, &timing_views, &timing_envelopes)
            .map_err(|()| "invalid submitted-frame GPU timing layout")?;
        let async_ticket = self.async_gpu_profile.lock().unwrap().reserve(
            profile_context, &timing_specs, ts, f64::from(gpu.queue.get_timestamp_period()));
        if let (Some(ticket), Some((qs, resolve, _))) = (&async_ticket, &self.query) {
            enc.resolve_query_set(qs, 0..ticket.query_count, resolve, 0);
            enc.copy_buffer_to_buffer(resolve, 0, &ticket.read, 0, u64::from(ticket.query_count) * 8);
        }
        // Cache updates always validate their raw visible count. The opt-in
        // page path checks a bounded asynchronous copy on a later frame;
        // captures, metrics and legacy far-sun updates keep their explicit wait.
        let mut solar_passes=Vec::new();
        let solar_record=|name,view_index,target:&ViewTargets,grouped|crate::shadow_diagnostics::Pass {
            name,view_index,counter:target.counter.clone(),visible:target.visible.clone(),
            capacity:(target.visible.size()/8)as u32,grouped};
        if draw_main_sun {
            solar_passes.push(solar_record(if static_sun.is_some(){"dynamic_sun"}else{"shared_sun"},None,&shadow_t,
                self.instance_groups.sun_enabled&&static_sun.is_none()));
        }
        if draw_static_sun {if let Some(target)=static_targets.as_ref() {
            solar_passes.push(solar_record("static_sun",None,target,false));
        }}
        if near_enabled {for (index,near) in near_shadows.iter().take(views.len()).enumerate() {
            solar_passes.push(solar_record("near_sun",Some(index),&near.targets,self.instance_groups.sun_enabled));
        }}
        let lamp_passes=headlight_targets.iter().take(headlights.len()).enumerate()
            .map(|(index,target)|solar_record("headlight",Some(index),target,self.instance_groups.headlight_enabled)).collect();
        if readback==Readback::None && !far_updated && (page_plan.updates.is_empty() || async_pages) {
            cpu.mark("diagnostics");
            let page_validation=if page_plan.updates.is_empty() {None}else{
                let pages:Vec<_>=page_plan.updates.iter().zip(page_targets.iter())
                    .map(|(page,(_,target))|(page.layer,page.capacity,&target.counter)).collect();
                Some(shadow_validation.as_mut().unwrap().encode(gpu,&mut enc,&pages)
                    .ok_or("foliage page validation queue unexpectedly full")?)
            };
            segments.push(enc.finish());
            gpu.queue.submit(segments);
            cpu.mark("submit");
            if let Some(ticket)=page_validation {shadow_validation.as_mut().unwrap().submitted(ticket);}
            *self.sun_diagnostics.lock().unwrap()=crate::shadow_diagnostics::State {context:Some(profile_context),passes:solar_passes};
            *self.headlight_diagnostics.lock().unwrap()=crate::shadow_diagnostics::State {context:Some(profile_context),passes:lamp_passes};
            if let Some(meadow) = &self.meadow { meadow.group_cache_submitted(); }
            if draw_main_sun { cache.as_mut().unwrap().content.submitted(); }
            if let (Some(split),Some(plan)) = (static_sun.as_mut(),static_plan) { split.submitted(plan); }
            color_diagnostics.submitted(profile_context, views, &keep,self.geometry_diagnostics.owners());
            if let Some(ticket) = async_ticket { ticket.submitted(); }
            // The ordered queue renders/clears every pending page before any
            // view samples it. Submission commits cache reuse; asynchronous
            // counter failure is a render error on the next completed poll.
            if sun_needed { self.foliage_sun_pages.submitted(&page_plan); }
            return Ok((ldr, Vec::new(), split_stats));
        }
        cpu.mark("diagnostics (slow path)");
        let near_count = if near_enabled { views.len() } else { 0 };
        let far_counter_index=views.len()+1+near_count+headlights.len();
        let page_counter_index=far_counter_index+usize::from(self.far_sun.enabled);
        let static_counter_index=page_counter_index+page_plan.updates.len();
        let counters = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16 * (static_counter_index+usize::from(static_sun.is_some())) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        if sun_needed { enc.copy_buffer_to_buffer(&shadow_t.counter, 0, &counters, 0, 16); }
        if sun_needed {if let Some(target)=static_targets.as_ref() {
            enc.copy_buffer_to_buffer(&target.counter,0,&counters,16*static_counter_index as u64,16);
        }}
        for (k, (main, ..)) in keep.iter().enumerate() {
            enc.copy_buffer_to_buffer(&main.counter, 0, &counters, 16 * (k as u64 + 1), 16);
        }
        for (k, near) in near_shadows.iter().take(near_count).enumerate() {
            enc.copy_buffer_to_buffer(&near.targets.counter, 0, &counters, 16 * (views.len() as u64 + 1 + k as u64), 16);
        }
        for (k,target) in headlight_targets.iter().take(headlights.len()).enumerate() {
            enc.copy_buffer_to_buffer(&target.counter,0,&counters,16*(views.len()+1+near_count+k)as u64,16);
        }
        if let Some(target)=far_targets.as_ref().filter(|_| sun_needed){
            enc.copy_buffer_to_buffer(&target.counter,0,&counters,16*far_counter_index as u64,16);
        }
        for (index,(_,target)) in page_targets.iter().take(page_plan.updates.len()).enumerate() {
            enc.copy_buffer_to_buffer(&target.counter,0,&counters,16*(page_counter_index+index)as u64,16);
        }
        if let Some((qs, resolve, read)) = &self.query {
            if async_ticket.is_none() { enc.resolve_query_set(qs, 0..ts, resolve, 0); }
            enc.copy_buffer_to_buffer(resolve, 0, read, 0, 8 * ts as u64);
        }
        segments.push(enc.finish());
        gpu.queue.submit(segments);
        *self.sun_diagnostics.lock().unwrap()=crate::shadow_diagnostics::State {context:Some(profile_context),passes:solar_passes};
        *self.headlight_diagnostics.lock().unwrap()=crate::shadow_diagnostics::State {context:Some(profile_context),passes:lamp_passes};
        if let Some(meadow) = &self.meadow { meadow.group_cache_submitted(); }
        if draw_main_sun { cache.as_mut().unwrap().content.submitted(); }
        if let (Some(split),Some(plan)) = (static_sun.as_mut(),static_plan) { split.submitted(plan); }
        color_diagnostics.submitted(profile_context, views, &keep,self.geometry_diagnostics.owners());
        if let Some(ticket) = async_ticket { ticket.submitted(); }
        if far_updated {self.far_sun.submitted();}
        let pixels = if readback==Readback::ImageAndMetrics { gpu.read_rgba8(&ldr) } else { Vec::new() };
        let mut stats = FrameStats {
            world_terrain_visibility:self.world_visibility.enabled,
            world_terrain_bytes:self.world_visibility.bytes,
            world_terrain_setup_cpu_ms:self.world_visibility.build_cpu_ms,
            far_sun_cache_updated:far_updated,
            far_sun_extent_m:far_projection.map_or(0.,|(p,_)|p.extent),
            far_sun_texel_m:far_projection.map_or(0.,|(p,_)|2.*p.extent/crate::far_sun::SIZE as f32),
            far_sun_lod_pixels:if self.far_sun.enabled && sun_needed{far_lod}else{0.},
            foliage_sun_requested_pages:page_plan.requested,
            foliage_sun_resident_pages:page_plan.resident,
            foliage_sun_missing_pages:page_plan.missing,
            foliage_sun_updated_pages:page_plan.updates.len()as u32,
            foliage_sun_prepare_cpu_ms:page_prepare_cpu_ms,
            foliage_sun_index_rebuilt:page_plan.cpu_index_rebuilt,
            foliage_sun_revision:page_plan.revision,
            foliage_sun_cache_bytes:self.foliage_sun_pages.bytes(),
            upscale: upscale_status,
            cluster_buckets: self.cluster_buckets,
            early_grass_cull: self.early_grass_cull,
            instance_broadphase: self.compact_cull.is_some() && !self.tree_occlusion.enabled && !self.instance_groups.enabled,
            instance_groups: self.instance_groups.enabled,
            cluster_traversal:self.cluster_traversal.enabled,
            tree_occlusion: self.tree_occlusion.enabled,
            canopy_updated_levels,
            canopy_pitch_m,
            camera_sun_visibility,
            ..split_stats
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
        // An explicit diagnostic already waited for this queue. Validate all
        // earlier asynchronous page copies too, including a final capture.
        if let Some(state)=shadow_validation.as_mut() {state.drain(gpu)?;}
        {
            let c = counters.slice(..).get_mapped_range().expect("counters");
            let c: &[u32] = bytemuck::cast_slice(&c);
            let sc = cache.as_mut().unwrap();
            if sun_needed {
                // The GPU counter also covers a preceding no-readback frame;
                // the retained CPU counts alone can otherwise be stale.
                (sc.clusters, sc.triangles) = (c[0], c[1]);
                sc.submitted_vertices = c[2];
                stats.shadow_clusters = sc.clusters;
                stats.shadow_triangles = sc.triangles;
                stats.shadow_submitted_vertices = if draw_main_sun { sc.submitted_vertices as u64 } else { 0 };
                if static_sun.is_some() {
                    let offset=4*static_counter_index;
                    assert!(c[offset]<=self.shadow_capacity,"static sun visible-cluster capacity exceeded");
                    assert!(c[0]<=MAX_VISIBLE,"dynamic sun visible-cluster capacity exceeded");
                    stats.static_sun_clusters=c[offset]; stats.static_sun_triangles=c[offset+1];
                    stats.static_sun_submitted_vertices=if draw_static_sun {u64::from(c[offset+2])}else{0};
                    stats.dynamic_sun_clusters=c[0]; stats.dynamic_sun_triangles=c[1];
                    stats.shadow_clusters+=stats.static_sun_clusters;
                    stats.shadow_triangles+=stats.static_sun_triangles;
                    stats.shadow_submitted_vertices+=stats.static_sun_submitted_vertices;
                }
            }
            for k in 0..views.len() {
                assert!(c[4 * (k + 1)] <= MAX_VISIBLE, "main visible-cluster capacity exceeded");
                stats.clusters += c[4 * (k + 1)];
                stats.triangles += c[4 * (k + 1) + 1];
                stats.submitted_vertices += c[4 * (k + 1) + 2] as u64;
                stats.early_grass_rejects += c[4 * (k + 1) + 3] as u64;
            }
            if let Some(transparency) = self.transparency.as_ref().filter(|_|debug_options.transparency_enabled) {
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
            if self.far_sun.enabled && sun_needed {
                let offset=4*far_counter_index;
                assert!(c[offset]<=self.far_sun.capacity,"far sun visible-cluster capacity exceeded: {} > {}",c[offset],self.far_sun.capacity);
                stats.far_sun_clusters=c[offset];stats.far_sun_triangles=c[offset+1];
                stats.far_sun_submitted_vertices=if far_updated{c[offset+2]as u64}else{0};
                stats.shadow_clusters+=stats.far_sun_clusters;stats.shadow_triangles+=stats.far_sun_triangles;
                stats.shadow_submitted_vertices+=stats.far_sun_submitted_vertices;
            }
            for (index,page) in page_plan.updates.iter().enumerate() {
                let offset=4*(page_counter_index+index);
                if c[offset]>page.capacity {
                    return Err(format!("foliage page{} cluster capacity exceeded: {}>{}; cache update not committed",page.layer,c[offset],page.capacity));
                }
                stats.foliage_sun_clusters+=c[offset];stats.foliage_sun_triangles+=c[offset+1];
                stats.shadow_clusters+=c[offset];stats.shadow_triangles+=c[offset+1];
                stats.shadow_submitted_vertices+=c[offset+2]as u64;
            }
        }
        if sun_needed { self.foliage_sun_pages.submitted(&page_plan); }
        if let Some((_, _, read)) = &self.query {
            let sample = {
                let t = read.slice(..).get_mapped_range().expect("timestamps");
                let t: &[u64] = bytemuck::cast_slice(&t);
                let period = gpu.queue.get_timestamp_period() as f64;
                if ts == 0 { Ok(Vec::new()) } else {
                    crate::gpu_profile::decode_timestamps(profile_context, &timing_specs, ts, period, t)
                        .map(|sample| sample.gpu_ms)
                }
            };
            read.unmap();
            stats.gpu_ms = sample.map_err(|()| "invalid synchronous GPU timestamps")?;
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
pub fn split_rects(n: usize, w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> { split_rects_with(n, w, h, SplitLayout::Stacked) }

/// How two views share the screen; three and four always use quarters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitLayout {
    /// One above the other (wide views, for racing).
    #[default]
    Stacked,
    /// Left and right halves (tall views, for shooters).
    SideBySide,
}

/// [`split_rects`] with a choice of two-view layout.
pub fn split_rects_with(n: usize, w: u32, h: u32, layout: SplitLayout) -> Vec<(u32, u32, u32, u32)> {
    let gap = ((h as f32 * 4.0 / 1080.0).round() as u32).max(1).min(w.saturating_sub(2)).min(h.saturating_sub(2));
    match n {
        0 | 1 => vec![(0, 0, w, h)],
        2 if layout == SplitLayout::SideBySide => {
            let hw = (w - gap) / 2;
            vec![(0, 0, hw, h), (hw + gap, 0, w - hw - gap, h)]
        }
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
    #[test]
    fn foliage_handoffs_reset_only_changed_views_and_not_stable_frames() {
        let mut views=[Vec::new(),Vec::new()];
        let pack=|s:[[u32;4];2]|s.map(crate::voxel_forest::selection_pack);
        let source=pack([[0,0,0,0],[0,0,0,0]]);
        let volume=pack([[1,2,0,0],[0,0,0,0]]);
        assert!(super::update_foliage_history(&mut views[0],&source));
        assert!(super::update_foliage_history(&mut views[1],&volume));
        assert!(!super::update_foliage_history(&mut views[0],&source));
        assert!(!super::update_foliage_history(&mut views[1],&volume));
        assert!(super::update_foliage_history(&mut views[0],&volume));
        assert!(!super::update_foliage_history(&mut views[1],&volume));
        let coarser=pack([[1,3,0,0],[0,0,0,0]]);
        assert!(super::update_foliage_history(&mut views[0],&coarser));
    }
    use super::{split_rects, split_rects_with, SplitLayout, timestamp_pair, LightUniform, NEAR_SHADOW_EXTENT, NEAR_SHADOW_SIZE, TIMESTAMP_CAPACITY};
    #[test]
    fn sun_visibility_fast_path_preserves_diagnostics_and_invalid_inputs() {
        use super::sun_visibility_required as required;
        for lux in [0.0, -0.0, -1.0] {
            assert!(!required(true, lux, 0.0));
            assert!(required(false, lux, 0.0), "reference path must retain work");
            for debug in [1.0, 8.0, 9.0, 10.0, 15.0, 16.0, 17.0, 23.0, 24.0, 25.0] {
                assert!(required(true, lux, debug), "diagnostic {debug} lost its input");
            }
        }
        for lux in [f32::MIN_POSITIVE, 0.0001, 80_000.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(required(true, lux, 0.0), "do not threshold away a nonzero sun or hide invalid input");
        }
        assert!(required(true, 0.0, f32::NAN));
    }

    #[test]
    fn retained_shadow_content_rebuilds_after_motion_night_and_caster_setting_changes() {
        use super::ShadowContent;
        let projection = glam::Mat4::IDENTITY;
        let mut content = ShadowContent::pending(projection, 0.5, 2.0);
        assert!(!content.reusable(projection, 0.5, 2.0), "allocation is not rendered content");
        content.submitted();
        assert!(content.reusable(projection, 0.5, 2.0));
        assert!(!content.reusable(projection, 0.51, 2.0), "alpha changes alter tree coverage");
        assert!(!content.reusable(projection, 0.5, 3.0), "caster LOD changes alter depth");
        assert!(!content.reusable(glam::Mat4::from_translation(glam::Vec3::X), 0.5, 2.0));
        for reason in ["moving car", "branch wind", "zero-sun frame before dawn"] {
            content.invalidate();
            assert_eq!(content.view_proj, projection, "{reason}: allocation content metadata can remain resident");
            assert!(!content.reusable(projection, 0.5, 2.0), "{reason}: retained storage must not imply valid depth");
            content = ShadowContent::pending(projection, 0.5, 2.0);
            assert!(!content.reusable(projection, 0.5, 2.0), "an unsubmitted frame must retry");
            content.submitted();
            assert!(content.reusable(projection, 0.5, 2.0));
        }
    }

    #[test]
    fn zero_sun_guard_returns_before_shared_direct_visibility_queries() {
        // Inspect composed WGSL, not an isolated replacement shader. An early
        // guard must precede any texture/world query in both public helpers.
        let source = format!("{}\n{}", include_str!("shaders/virtual_member.wgsl"), crate::shaders::SHARED_NAMES.map(crate::shaders::built_in).join("\n"));
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let guard = module.functions.iter().find(|(_, f)| f.name.as_deref() == Some("sun_visibility_required")).unwrap().0;
        for name in ["shadow_at_surface", "grass_sun_transmittance_receiver"] {
            let f = module.functions.iter().find(|(_, f)| f.name.as_deref() == Some(name)).unwrap().1;
            let mut guard_result = None;
            let mut returned = false;
            for statement in f.body.iter() {
                match statement {
                    naga::Statement::Emit(_) => {},
                    naga::Statement::Call { function, result, .. } if guard_result.is_none() => {
                        assert_eq!(*function, guard, "{name} queried visibility before its guard");
                        guard_result = *result;
                    },
                    naga::Statement::If { condition, accept, .. } => {
                        let naga::Expression::Unary { op: naga::UnaryOperator::LogicalNot, expr } = f.expressions[*condition]
                            else { panic!("{name} must return when the visibility guard is false") };
                        assert_eq!(Some(expr), guard_result);
                        assert!(accept.iter().any(|s| matches!(s, naga::Statement::Return { value: Some(_) })));
                        returned = true;
                        break;
                    },
                    _ => panic!("{name} executed work before its zero-sun return"),
                }
            }
            assert!(returned, "{name} lost its early return");
        }
    }

    #[test]
    fn large_first_changes_only_camera_order_not_indirect_records_or_triangle_ids() {
        use super::{raster_bucket_order,RasterPass};
        for kind in [RasterPass::Color,RasterPass::Sun,RasterPass::SunStatic(10),RasterPass::SunDynamic(10),RasterPass::FarSun,RasterPass::FoliagePage,RasterPass::Headlight] {
            assert_eq!(raster_bucket_order(false,true,kind),[0]);
            assert_eq!(raster_bucket_order(true,false,kind),[0,1,2,3]);
            let order=raster_bucket_order(true,true,kind);
            if matches!(kind,RasterPass::Color) {assert_eq!(order,[3,2,1,0]);}
            else {assert_eq!(order,[0,1,2,3]);}
            // Record byte offset, vertex count, first_vertex and original
            // visibility slot must travel together regardless of draw order.
            let records=[(24u32,0u32,19u32,6u32),(96,384,7,31),(192,768,28,47),(384,1152,3,127)];
            let mut ids=Vec::new();let mut submitted=0;
            for &bucket in order {
                let offset=bucket*16;let (vertices,first,slot,triangles)=records[(offset/16)as usize];
                submitted+=vertices;
                for vertex in (0..vertices).step_by(3) {
                    let vi=first+vertex;
                    assert_eq!(vi/384,bucket as u32);
                    let tri=(vi%384)/3;
                    if tri<triangles {ids.push((slot<<7)|tri);}
                }
            }
            let mut expected:Vec<_>=records.iter().flat_map(|&(_,_,slot,triangles)|(0..triangles).map(move |tri|(slot<<7)|tri)).collect();
            ids.sort_unstable();expected.sort_unstable();
            assert_eq!(ids,expected);assert_eq!(submitted,696);
        }
    }
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
            (naga::ShaderStage::Compute, "instance_broadphase"), (naga::ShaderStage::Compute, "instance_dispatch"),
            (naga::ShaderStage::Compute, "cull_compacted"),
            (naga::ShaderStage::Compute, "spatial_select"), (naga::ShaderStage::Compute, "spatial_dispatch"),
            (naga::ShaderStage::Compute, "sun_spatial_select"), (naga::ShaderStage::Compute, "headlight_spatial_select"),
            (naga::ShaderStage::Compute, "cull_spatial"),
            (naga::ShaderStage::Compute, "cull_cached_sun"), (naga::ShaderStage::Compute, "cull_spatial_cached_sun"),
            (naga::ShaderStage::Compute, "prepare_tree_phase"),
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
    fn sun_terrain_error_cap_is_equal_in_world_units_without_changing_other_view_settings() {
        let make=|scale,threshold|super::view_uniform(glam::Mat4::IDENTITY,glam::Vec3::ZERO,scale,
            [threshold,1.0,1.0,1024.0],[2048.0;4],0.5);
        let mut far=make(4096.0/192.0,2.0);let mut near=make(2048.0/32.0,1.0);
        let old_far=far;let old_near=near;
        super::configure_sun_terrain_error(&mut far,true);
        super::configure_sun_terrain_error(&mut near,true);
        assert_eq!(far.params,old_far.params);assert_eq!(near.params,old_near.params);
        assert_eq!(far.fog_cull,[0.0,0.0,0.015625,0.0]);assert_eq!(near.fog_cull,far.fog_cull);
        let threshold=|v:&super::ViewUniform|v.params[0].min(v.fog_cull[2]*v.cam[3]);
        assert_eq!(threshold(&far)/far.cam[3],threshold(&near)/near.cam[3]);
        // A representative parent/child hierarchy gets the identical cut at
        // both resolutions; source-only callers remain stricter (zero).
        for (own,parent) in [(0.0,0.01),(0.01,0.02),(0.02,0.08),(0.08,1e31)] {
            let keep=|v:&super::ViewUniform|own*v.cam[3]<=threshold(v) && parent*v.cam[3]>threshold(v);
            assert_eq!(keep(&far),keep(&near));
        }
        let mut exact=make(4096.0/192.0,0.0);super::configure_sun_terrain_error(&mut exact,true);
        assert_eq!(threshold(&exact),0.0);
        super::configure_sun_terrain_error(&mut far,false);
        assert_eq!(bytemuck::bytes_of(&far),bytemuck::bytes_of(&old_far));
        // Standard camera/headlight construction remains byte-identical.
        assert_eq!(make(40.0,1.0).fog_cull,[0.0;4]);
    }
    #[test]
    fn godot_split_layout_uses_one_4k_output() {
        assert_eq!(split_rects(2,3840,2160),vec![(0,0,3840,1076),(0,1084,3840,1076)]);
        assert_eq!(split_rects_with(2,3840,2160,SplitLayout::SideBySide),vec![(0,0,1916,2160),(1924,0,1916,2160)]);
        assert_eq!(split_rects(4,3840,2160),vec![(0,0,1916,1076),(1924,0,1916,1076),(0,1084,1916,1076),(1924,1084,1916,1076)]);
        let odd=split_rects(4,1921,1081);
        assert_eq!(odd[3].0+odd[3].2,1921);
        assert_eq!(odd[3].1+odd[3].3,1081);
        assert!(odd.iter().all(|r|r.2>0&&r.3>0&&r.0+r.2<=1921&&r.1+r.3<=1081));
    }

    #[test]
    fn four_views_with_near_maps_voxels_grass_transparency_and_taa_fit_timestamp_budget() {
        // Wind + branch poses + baked actors + shared far map: 5 passes,
        // plus two cull/raster passes for each of eight shared headlights.
        // Each view: canopy/filter, near cull/raster,
        // main cull/raster, shade, voxel tiles/integrate/composite,
        // far grass, grass transition/composite, transparency, TAA,
        // exhaust, wheel particles, tonemap: 18 intervals.
        let mut next = 0;
        for _ in 0..(5 + 8 * 2 + 4 * 18) { timestamp_pair(&mut next); }
        assert_eq!(next, 186);
        assert!(next <= TIMESTAMP_CAPACITY);
        // DLSS adds motion + reconstruction and replaces TAA, including the
        // supported fractional-voxel motion producer.
        next = 0;
        for _ in 0..(5 + 8 * 2 + 4 * 19) { timestamp_pair(&mut next); }
        assert_eq!(next, 194);
        assert!(next <= TIMESTAMP_CAPACITY);
        // Eight shared page updates, four compact owner passes and four grass
        // depth prepasses still fit the same bounded timestamp array.
        for _ in 0..(8 * 2 + 4 + 4) { timestamp_pair(&mut next); }
        assert_eq!(next, 242);
        assert!(next <= TIMESTAMP_CAPACITY);
        // Meadow placement, nearest blade rendering and composition are
        // separate intervals for each view, including four-player DLSS.
        for _ in 0..(4 * 3) { timestamp_pair(&mut next); }
        assert_eq!(next, 266);
        assert!(next <= TIMESTAMP_CAPACITY);
        // Two-phase tree camera culling replaces two intervals by five.
        // Conservatively retain the broadphase budget too although that
        // experiment is bypassed by the two-phase implementation.
        for _ in 0..(4*3) {timestamp_pair(&mut next);}
        assert_eq!(next,290);
        assert!(next<=TIMESTAMP_CAPACITY);
        // Far meadow generation, coverage accumulation and source/far merge
        // have independent intervals; no overlap with source/composite cost.
        for _ in 0..(4 * 3) { timestamp_pair(&mut next); }
        assert_eq!(next, 314);
        assert!(next <= TIMESTAMP_CAPACITY);
        // Spatial groups: once-frame refit plus two selection phases/view.
        // Four old broadphase intervals were already included above.
        for _ in 0..5 {timestamp_pair(&mut next);}
        assert_eq!(next,324);
        assert!(next<=TIMESTAMP_CAPACITY);
        // Split main sun replaces the original cull/raster with static plus
        // dynamic cull/raster and a separately measured depth seed copy.
        for _ in 0..3 {timestamp_pair(&mut next);}
        assert_eq!(next,330);
        assert!(next<=TIMESTAMP_CAPACITY);
        // Near meadow visibility+resolve replaces one source-render scope by
        // two. The bounded native brick fixture replaces all six meadow scopes
        // per view by eight shared construction + three integration scopes.
        for _ in 0..4 {timestamp_pair(&mut next);}
        assert_eq!(next,338);
        assert!(next<=TIMESTAMP_CAPACITY);
        // Independent light-space hierarchy traversal for up to eight lamps.
        for _ in 0..8 {timestamp_pair(&mut next);}
        assert_eq!(next,354);
        assert!(next<=TIMESTAMP_CAPACITY);
        // Up to five independent current-pose sun group selections (shared
        // plus four near). Conservative: shared split caching is exclusive.
        for _ in 0..5 {timestamp_pair(&mut next);}
        assert_eq!(next,364);
        assert!(next<=TIMESTAMP_CAPACITY);
        // Aggregate error selection and volume binning add two passes per view.
        for _ in 0..8 {timestamp_pair(&mut next);}
        assert_eq!(next,380);
        timestamp_pair(&mut next); // shared receiver demand build + OR hierarchy
        assert_eq!(next,382);
        assert!(next<=TIMESTAMP_CAPACITY);
        for _ in 0..4 {timestamp_pair(&mut next);} // one bounded bloom scope per view
        assert_eq!(next,390);
        assert!(next<=TIMESTAMP_CAPACITY);
        // The legacy aggregate envelope aliases depth-start/illumination-end.
        // Three physical pairs replace its one pair: only two extra per view.
        for _ in 0..(4 * 2) { timestamp_pair(&mut next); }
        assert_eq!(next,406);
        assert!(next<=TIMESTAMP_CAPACITY);
        for _ in 0..4 { timestamp_pair(&mut next); } // per-view local grass occupancy
        assert_eq!(next,414);
        for _ in 0..4 {timestamp_pair(&mut next);} // separate voxel visibility stream per view
        assert_eq!(next,422);
        assert!(next<=TIMESTAMP_CAPACITY);
        for _ in 0..4 {timestamp_pair(&mut next);} // optional motion blur after temporal resolve
        assert!(next<=TIMESTAMP_CAPACITY);
        let brick_fixture_upper_bound = 330 - 4*6*2 + (8+4*3)*2;
        assert!(brick_fixture_upper_bound <= TIMESTAMP_CAPACITY);
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
    fn physical_lighting_focus_is_camera_rotation_independent_and_split_ordered() {
        use super::{resolve_lighting_focus,ViewRect,Camera};
        use glam::Vec3;
        let cars=[Vec3::new(-42.0,8.0,-1197.0),Vec3::new(100.0,40.0,-1300.0)];
        let sun=Vec3::new(0.9653374,0.034899495,-0.25866139);
        let reference=crate::stability::stable_shadow(cars[0],sun,96.0,4096);
        for yaw in [-180.0f32,-29.0,0.0,29.0,90.0,180.0] {
            let orbit=glam::Quat::from_rotation_y(yaw.to_radians());
            let views:Vec<_>=cars.iter().map(|&p|ViewRect{cam:Camera {
                pos:p+orbit*Vec3::new(0.0,2.0,6.0),target:p+orbit*Vec3::NEG_Z,
                near:0.1,fov_y:1.0},x:0,y:0,w:960,h:1080}).collect();
            let resolved=resolve_lighting_focus(&views,&cars).unwrap();
            assert_eq!(resolved,cars);
            assert_eq!(crate::stability::stable_shadow(resolved[0],sun,96.0,4096),reference);
            assert!(resolve_lighting_focus(&views,&cars[..1]).is_err());
            let fallback=resolve_lighting_focus(&views,&[]).unwrap();
            assert_eq!(fallback,views.iter().map(|v|v.cam.pos).collect::<Vec<_>>());
        }
    }

    #[test]
    fn emission_follows_the_exposure_only_when_asked() {
        let mut light = crate::Lighting { ev100: 11.0, emissive_ev100: 13.6, ..Default::default() };
        assert_eq!(light.emissive_scale(), 1.0, "off by default: physical emission");
        light.emissive_compensation = 1.0;
        assert!((light.emissive_scale() - 2f32.powf(-2.6)).abs() < 1e-6, "shown as at EV 13.6");
        light.emissive_compensation = 0.5;
        assert!((light.emissive_scale() - 2f32.powf(-1.3)).abs() < 1e-6);
        light.ev100 = 13.6;
        assert_eq!(light.emissive_scale(), 1.0, "no change at the reference exposure");
        assert!(light.set("emissive_compensation=2").is_err());
    }

    #[test]
    fn near_light_uniform_and_projection_cover_each_separated_car() {
        use glam::Vec3;
        assert_eq!(std::mem::size_of::<LightUniform>(), 432);
        assert_eq!(std::mem::offset_of!(LightUniform, fog_sampling), 416);
        assert_eq!(std::mem::offset_of!(LightUniform, anim), 400);
        assert_eq!(std::mem::offset_of!(LightUniform, fog_scattering), 144);
        assert_eq!(std::mem::offset_of!(LightUniform, near_shadow_view_proj), 304);
        assert_eq!(std::mem::offset_of!(LightUniform, near_shadow), 368);
        assert_eq!(std::mem::offset_of!(LightUniform, night_stars), 384);
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

fn configure_sun_terrain_error(view:&mut ViewUniform,enabled:bool) {
    // Explicit sun-only call sites. z was reserved; x/y remain fog settings.
    // One near-map world texel is a cap, never coarsening a stricter caller.
    view.fog_cull[2]=if enabled {2.0*NEAR_SHADOW_EXTENT/NEAR_SHADOW_SIZE as f32}else{0.0};
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

#[cfg(test)]
mod solar_effect_tests {
    use super::*;
    #[test]
    fn solar_projection_is_directional_and_rejects_back_and_night() {
        let cam=Camera {pos:Vec3::ZERO,target:Vec3::new(0.0,0.2,-1.0),fov_y:1.0,near:0.1};
        let mut light=Lighting {sun_dir:cam.target.normalize(),sun_disc:true,lens_flare:0.22,..Default::default()};
        let sample=|cam:&Camera,light:&Lighting| solar_projection(cam,camera_view(*cam,None),16.0/9.0,light,1.0);
        let first=sample(&cam,&light);
        assert!((first[0]-0.5).abs()<1e-5 && (first[1]-0.5).abs()<1e-5 && first[3]>0.0);
        let shift=Vec3::new(100.0,30.0,-20.0);
        let moved=sample(&Camera {pos:cam.pos+shift,target:cam.target+shift,..cam},&light);
        for i in 0..4 { assert!((first[i]-moved[i]).abs()<1e-5); }
        light.sun_dir=-light.sun_dir; assert_eq!(sample(&cam,&light),[0.0;4]);
        light.sun_dir=cam.target.normalize(); light.sun_lux=0.0; assert_eq!(sample(&cam,&light),[0.0;4]);
    }
    #[test]
    fn solar_tonemap_shader_validates() {
        let src=include_str!("shaders/tonemap.wgsl");
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<ToneUniform>(),80);
    }
}

fn max_column(m: Mat4) -> f32 {
    m.x_axis.truncate().length().max(m.y_axis.truncate().length()).max(m.z_axis.truncate().length())
}
