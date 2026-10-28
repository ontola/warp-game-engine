//! A tiling scalar field for procedural materials: a square power-of-two
//! image a game supplies once (`Scene::noise`), uploaded as a mipmapped
//! R16Float texture with repeat addressing and trilinear filtering, like a
//! noise texture in a material. Shaders read it with
//! `noise_field(uv, lod)` (`uv` in texture repeats, `lod` in mip levels).
//! Growing Guns' lava reads Godot's FastNoiseLite texture from it; without
//! a field the shaders see a flat 0.5 and fall back to their own noise.

/// `size` x `size` values, row major (x across, y down), any finite range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoiseField {
    pub size: u32,
    pub values: Vec<f32>,
}

impl NoiseField {
    pub fn validate(&self) -> Result<(), String> {
        if !self.size.is_power_of_two() || self.size > 4096 {
            return Err(format!("noise field size {} must be a power of two up to 4096", self.size));
        }
        if self.values.len() != (self.size * self.size) as usize || !self.values.iter().all(|v| v.is_finite()) {
            return Err("noise field needs size * size finite values".into());
        }
        Ok(())
    }

    /// Every mip level, full size first, each the 2x2 mean of the one above
    /// (which a tiling field keeps tiling).
    pub fn mips(&self) -> Vec<Vec<f32>> {
        let mut levels = vec![self.values.clone()];
        let mut n = self.size as usize;
        while n > 1 {
            let above = levels.last().expect("level");
            let h = n / 2;
            let level = (0..h * h).map(|k| {
                let (x, y) = ((k % h) * 2, (k / h) * 2);
                (above[y * n + x] + above[y * n + x + 1] + above[(y + 1) * n + x] + above[(y + 1) * n + x + 1]) * 0.25
            }).collect();
            levels.push(level);
            n = h;
        }
        levels
    }
}

/// IEEE half from a float, rounded to nearest even; tiny values flush to
/// zero and huge ones saturate to infinity (fields are noise, near 0..1).
pub fn f16_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = b & 0x7f_ffff;
    if exp <= 0 { return sign; }
    if exp >= 31 { return sign | 0x7c00; }
    let mut h = u32::from(sign) | ((exp as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && h & 1 == 1) { h += 1; }
    h as u16
}

/// The field's texture view (a 1x1 0.5 without one).
pub(crate) fn texture(gpu: &crate::Gpu, field: Option<&NoiseField>) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let flat = NoiseField { size: 1, values: vec![0.5] };
    let field = match field {
        Some(f) => { f.validate().expect("invalid Scene::noise"); f }
        None => &flat,
    };
    let mips = field.mips();
    let data: Vec<u16> = mips.iter().flatten().map(|&v| f16_bits(v)).collect();
    let texture = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("noise field"),
            size: wgpu::Extent3d { width: field.size, height: field.size, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(&data),
    );
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_to_nearest() {
        for (v, h) in [(0.0, 0x0000), (0.5, 0x3800), (1.0, 0x3c00), (-2.0, 0xc000), (0.333_333_34, 0x3555), (65504.0, 0x7bff), (1e6, 0x7c00), (1e-9, 0)] {
            assert_eq!(f16_bits(v), h, "{v}");
        }
    }

    #[test]
    fn mips_average_down_to_one_texel() {
        let f = NoiseField { size: 4, values: (0..16).map(|k| k as f32).collect() };
        f.validate().unwrap();
        let m = f.mips();
        assert_eq!(m.len(), 3);
        assert_eq!(m[1], vec![2.5, 4.5, 10.5, 12.5]);
        assert_eq!(m[2], vec![7.5]);
        assert!(NoiseField { size: 3, values: vec![0.0; 9] }.validate().is_err());
        assert!(NoiseField { size: 2, values: vec![0.0; 3] }.validate().is_err());
    }
}
