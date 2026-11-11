//! Light shapes: cookies and angular profiles for clustered lights.
//!
//! A real lamp is rarely uniform: a flashlight's reflector throws a hot
//! centre and rings, a lampshade a bright rim, a ceiling fitting scallops on
//! the walls. Two ways to describe that, both optional per light
//! ([`LightShape`]):
//!
//! - A **cookie**: a 256x256 greyscale image projected through a spot's
//!   outer cone (centre = the spot's axis, edge = the outer cone), multiplied
//!   into its light. For spots (outer cone under 87 degrees).
//! - An **angular profile**: intensity against the angle from the light's
//!   axis, 0 to 180 degrees in 256 steps (an IES-like candela curve), for
//!   spots and omnis.
//!
//! Both multiply the light's `intensity_cd` (0..1, so a cookie or profile
//! scales down; raise the intensity to keep the peak). They apply to surface
//! light and to the shadowed volumetric fog. One texture array holds them:
//! layers 0..14 are cookies 1..=15, layer 15 holds profiles 1..=255, one per
//! row.
use crate::Gpu;

pub const SIZE: u32 = 256;
pub const COOKIES: u8 = 15;
pub const PROFILES: u8 = 255;
const LAYERS: u32 = 16;

/// Which cookie and profile a light uses: 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LightShape {
    /// 1..=15: a cookie set with `Renderer::set_light_cookie`.
    pub cookie: u8,
    /// 1..=255: a profile set with `Renderer::set_light_profile`.
    pub profile: u8,
}

impl LightShape {
    /// The bits shaders read from a light's fourth `source` word, above the
    /// shadow face count (0..7).
    pub fn packed(self) -> f32 { (self.cookie.min(COOKIES) as u32 * 8 + self.profile as u32 * 128) as f32 }
}

pub(crate) struct LightShapes {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl LightShapes {
    pub fn new(gpu: &Gpu) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("light cookies and profiles"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: LAYERS }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { texture, view }
    }

    fn write(&self, gpu: &Gpu, layer: u32, row: u32, rows: u32, data: &[u8]) {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: row, z: layer }, aspect: wgpu::TextureAspect::All },
            data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SIZE), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: SIZE, height: rows, depth_or_array_layers: 1 });
    }

    /// Cookie `index` (1..=15) from 256x256 values in 0..1, row by row.
    pub fn set_cookie(&self, gpu: &Gpu, index: u8, values: &[f32]) -> Result<(), String> {
        if !(1..=COOKIES).contains(&index) || values.len() != (SIZE * SIZE) as usize {
            return Err(format!("a light cookie is index 1..={COOKIES} with {}x{} values", SIZE, SIZE));
        }
        self.write(gpu, index as u32 - 1, 0, SIZE, &quantise(values));
        Ok(())
    }

    /// Profile `index` (1..=255) from 256 values in 0..1, from the light's
    /// axis (0 degrees) to straight behind it (180 degrees).
    pub fn set_profile(&self, gpu: &Gpu, index: u8, values: &[f32]) -> Result<(), String> {
        if index == 0 || values.len() != SIZE as usize {
            return Err(format!("a light profile is index 1..={PROFILES} with {SIZE} values"));
        }
        self.write(gpu, LAYERS - 1, index as u32 - 1, 1, &quantise(values));
        Ok(())
    }
}

fn quantise(values: &[f32]) -> Vec<u8> {
    values.iter().map(|v| (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * 255.0 + 0.5) as u8).collect()
}
