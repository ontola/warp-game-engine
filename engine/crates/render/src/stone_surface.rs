//! A shared, deterministic quarried-stone surface, cooked once on the CPU.
//!
//! RGB is neutral sRGB albedo, intended to multiply the material's stone tint.
//! Albedo alpha is opaque; normal alpha is a **linear roughness multiplier**
//! in 0.80..=1.00. The linear-UNORM normal uses OpenGL's +U / -V tangent frame.
//! Normal B stores height in a signed 32 mm range for near-field parallax. Both maps
//! describe the same 1.5 m periodic height field; there is no baked lighting.
//! The ordinary texture-array mip chain filters away detail at distance.

use crate::scene::{Scene, TextureSource};
use image::{Rgba, RgbaImage};
use std::sync::OnceLock;

/// Physical width of one repeat. Keep the shader's stone mapping in sync.
pub const REPEAT_METRES: f32 = 1.5;

const SIZE: usize = 1024;
const NAME: &str = "warp-porous-stone-v4-quarried-height";
/// Signed height range encoded in normal B, shared with procedural_surface.wgsl.
pub const HEIGHT_RANGE_METRES: f32 = 0.032;
static SURFACE: OnceLock<TextureSource> = OnceLock::new();

/// Return an owned source for the renderer; the expensive bake happens once.
pub fn texture() -> TextureSource {
    let source = SURFACE.get_or_init(cook);
    TextureSource {
        name: source.name.clone(),
        rgba: source.rgba.clone(),
        normal: source.normal.clone(),
        orm: None,
    }
}

/// Share one texture-array layer across every stone material in a scene.
/// Looking up an existing layer does not clone the source images.
pub fn add_to_scene(scene: &mut Scene) -> i32 {
    if let Some(layer) = scene.textures.iter().position(|source| source.name == NAME) {
        return layer as i32;
    }
    let layer = scene.textures.len() as i32;
    scene.textures.push(texture());
    layer
}

#[derive(Clone, Copy, Default)]
struct Surface {
    height: f32,
    albedo: f32,
    roughness: f32,
}

impl Surface {
    fn add(&mut self, amount: f32, detail: Self) {
        self.height += amount * detail.height;
        self.albedo += amount * detail.albedo;
        self.roughness += amount * detail.roughness;
    }
}

fn hash(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn random(index: u32, channel: u32) -> f32 {
    let value = hash(index.wrapping_mul(0x9e37_79b9) ^ channel);
    (value >> 8) as f32 / 16_777_216.0
}

fn smootherstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let polynomial = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    if t <= 0.5 {
        polynomial(t)
    } else {
        1.0 - polynomial(1.0 - t)
    }
}

/// A toroidal lattice with a quintic interpolant: value and the first two
/// derivatives agree across both the lattice cells and the tile boundary.
struct Noise {
    period: usize,
    values: Vec<f32>,
}

impl Noise {
    fn new(period: usize, seed: u32) -> Self {
        Self {
            period,
            values: (0..period * period)
                .map(|index| random(index as u32, seed) * 2.0 - 1.0)
                .collect(),
        }
    }

    fn sample(&self, u: f32, v: f32) -> f32 {
        let x = u * self.period as f32;
        let y = v * self.period as f32;
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let sx = smootherstep(x - x.floor());
        let sy = smootherstep(y - y.floor());
        let at = |x: i32, y: i32| {
            self.values[y.rem_euclid(self.period as i32) as usize * self.period
                + x.rem_euclid(self.period as i32) as usize]
        };
        let a = at(ix, iy);
        let b = at(ix + 1, iy);
        let c = at(ix, iy + 1);
        let d = at(ix + 1, iy + 1);
        let top = a + (b - a) * sx;
        top + (c + (d - c) * sx - top) * sy
    }
}

fn wrapped_index(x: i32, y: i32) -> usize {
    y.rem_euclid(SIZE as i32) as usize * SIZE + x.rem_euclid(SIZE as i32) as usize
}

/// Compact C2 kernel: neither a pit nor its slope leaves an outline at its rim.
fn kernel(distance_squared: f32) -> f32 {
    let t = (1.0 - distance_squared).max(0.0);
    t * t * t
}

/// Paint a compact, rotated elliptical feature across the periodic boundary.
/// Positions/radii are in texels; feature heights remain in metres.
fn grain(field: &mut [Surface], center: [f32; 2], radius: [f32; 2], angle: f32, detail: Surface) {
    let (sn, cs) = angle.sin_cos();
    let bound = radius[0].max(radius[1]);
    let inverse = [1.0 / radius[0], 1.0 / radius[1]];
    for y in (center[1] - bound).floor() as i32..=(center[1] + bound).ceil() as i32 {
        for x in (center[0] - bound).floor() as i32..=(center[0] + bound).ceil() as i32 {
            let dx = x as f32 + 0.5 - center[0];
            let dy = y as f32 + 0.5 - center[1];
            let rx = (cs * dx + sn * dy) * inverse[0];
            let ry = (-sn * dx + cs * dy) * inverse[1];
            field[wrapped_index(x, y)].add(kernel(rx * rx + ry * ry), detail);
        }
    }
}

/// Porous stone: overlapping irregular depressions in three physical size
/// bands, with restrained granular relief and mineral colour variation. No
/// Voronoi plate boundaries, drawn crack network, or baked directional light.
fn height_field() -> Vec<Surface> {
    let mottling = Noise::new(9, 0xa51c_8e29);
    let coarse = Noise::new(31, 0x63d8_35a7);
    let grains = Noise::new(181, 0xc02b_79e3);
    let grit = Noise::new(371, 0x95f1_b863);
    let mut field = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE { for x in 0..SIZE {
        let u = (x as f32 + 0.5) / SIZE as f32;
        let v = (y as f32 + 0.5) / SIZE as f32;
        let c = coarse.sample(u, v);
        let g = grains.sample(u, v);
        let f = grit.sample(u, v);
        field.push(Surface {
            height: c * 0.0012 + g * 0.00032 + f * 0.00013,
            albedo: 0.87 + mottling.sample(u,v) * 0.045 + c * 0.025 + g * 0.035,
            roughness: 0.93 + g * 0.018 - f * 0.012,
        });
    }}
    let texels_per_metre = SIZE as f32 / REPEAT_METRES;
    // Radius/depth in metres. Small pores dominate; a sparse larger family
    // creates occasional cavities. Two offset elliptical lobes keep pits
    // irregular instead of making a uniform polka-dot pattern.
    for (count, radius_lo, radius_hi, depth_lo, depth_hi, seed) in [
        (650u32, 0.012, 0.035, 0.0025, 0.0070, 0x1981u32),
        (9000, 0.0016, 0.0065, 0.0006, 0.0025, 0x3811),
        (50000, 0.0006, 0.0025, 0.00015, 0.0007, 0x2341),
    ] {
        for i in 0..count {
            let radius = (radius_lo + random(i, seed) * (radius_hi-radius_lo)) * texels_per_metre;
            let center = [random(i, seed+1) * SIZE as f32, random(i, seed+2) * SIZE as f32];
            let aspect = 0.45 + random(i, seed+3) * 0.5;
            let angle = random(i, seed+4) * std::f32::consts::TAU;
            let density = 0.8 + 0.2 * coarse.sample(center[0]/SIZE as f32, center[1]/SIZE as f32);
            let detail = Surface {
                height: -(depth_lo + random(i, seed+5) * (depth_hi-depth_lo)) * density,
                // Restrained mineral staining, not a painted lighting gradient.
                albedo: -0.14, roughness: 0.018,
            };
            grain(&mut field, center, [radius, radius * aspect], angle, detail);
            if i % 3 != 0 {
                let (sn,cs) = angle.sin_cos();
                grain(&mut field, [center[0]+cs*radius*0.55, center[1]+sn*radius*0.55],
                    [radius*0.65, radius*aspect*0.85], angle+0.7,
                    Surface { height: detail.height * 0.7, ..detail });
            }
        }
    }
    // Fine mineral flecks change colour and roughness independently of pores.
    for i in 0..6500 {
        let radius = (0.0020 + random(i, 0x4c21) * 0.0060) * texels_per_metre;
        let pale = random(i, 0x4c22) > 0.4;
        grain(&mut field, [random(i, 0x4c23)*SIZE as f32, random(i, 0x4c24)*SIZE as f32],
            [radius, radius*(0.45+random(i,0x4c25)*0.5)], random(i,0x4c26)*std::f32::consts::TAU,
            Surface { height: if pale {0.00015} else {-0.0001},
                albedo: if pale {0.065} else {-0.06}, roughness: -0.04 });
    }
    field
}

fn srgb(linear: f32) -> u8 {
    // The field is clamped above the linear section of the sRGB transfer.
    ((1.055 * linear.clamp(0.65, 1.0).powf(1.0 / 2.4) - 0.055) * 255.0).round() as u8
}

fn normal(field: &[Surface], x: i32, y: i32) -> [f32; 3] {
    // Central differences wrap across the actual sampled height field. Image
    // V grows downwards, whereas the renderer's bitangent points towards -V.
    let per_metre = SIZE as f32 / (2.0 * REPEAT_METRES);
    let dx =
        (field[wrapped_index(x + 1, y)].height - field[wrapped_index(x - 1, y)].height) * per_metre;
    let dy =
        (field[wrapped_index(x, y + 1)].height - field[wrapped_index(x, y - 1)].height) * per_metre;
    let inverse_length = 1.0 / (1.0 + dx * dx + dy * dy).sqrt();
    [-dx * inverse_length, dy * inverse_length, inverse_length]
}

fn cook() -> TextureSource {
    let field = height_field();
    let mut rgba = RgbaImage::new(SIZE as u32, SIZE as u32);
    let mut normals = RgbaImage::new(SIZE as u32, SIZE as u32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let sample = field[y * SIZE + x];
            let color = srgb(sample.albedo);
            rgba.put_pixel(x as u32, y as u32, Rgba([color, color, color, 255]));
            let n = normal(&field, x as i32, y as i32);
            let encode = |value: f32| ((value * 0.5 + 0.5) * 255.0).round() as u8;
            normals.put_pixel(
                x as u32,
                y as u32,
                Rgba([
                    encode(n[0]),
                    encode(n[1]),
                    ((0.5 + sample.height / HEIGHT_RANGE_METRES).clamp(0.0, 1.0) * 255.0).round() as u8,
                    (sample.roughness.clamp(0.8, 1.0) * 255.0).round() as u8,
                ]),
            );
        }
    }
    TextureSource {
        name: NAME.to_owned(),
        rgba,
        normal: Some(normals),
        orm: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(byte: u8) -> f64 {
        ((byte as f64 / 255.0 + 0.055) / 1.055).powf(2.4)
    }

    #[test]
    fn periodic_noise_has_continuous_values_and_slopes() {
        let noise = Noise::new(31, 0x63d8_35a7);
        let epsilon = 0.00002;
        for v in [0.0, 0.0713, 0.4187, 0.935] {
            assert!((noise.sample(-0.03125, v) - noise.sample(0.96875, v)).abs() < 2e-5);
            assert!((noise.sample(v, -0.03125) - noise.sample(v, 0.96875)).abs() < 2e-5);
            for transpose in [false, true] {
                let sample = |u| {
                    if transpose {
                        noise.sample(v, u)
                    } else {
                        noise.sample(u, v)
                    }
                };
                let left = (sample(0.0) - sample(-epsilon)) / epsilon;
                let right = (sample(epsilon) - sample(0.0)) / epsilon;
                assert!((sample(-epsilon) - sample(epsilon)).abs() < 2e-5);
                assert!(
                    (left - right).abs() < 0.03,
                    "slope discontinuity: {left} / {right}"
                );
            }
        }
    }

    #[test]
    fn features_and_normal_stencils_cross_both_tile_edges() {
        let mut field = vec![Surface::default(); SIZE * SIZE];
        grain(
            &mut field,
            [0.0, 0.0],
            [5.0, 5.0],
            0.0,
            Surface {
                height: -0.001,
                albedo: -0.05,
                roughness: 0.02,
            },
        );
        assert!(field[wrapped_index(-1, -1)].height < -0.0005);
        assert!(field[wrapped_index(0, 0)].height < -0.0005);
        for (x, y) in [(-1, -1), (0, 1), (3, -2), (511, 511)] {
            assert_eq!(
                normal(&field, x, y),
                normal(&field, x + SIZE as i32, y - SIZE as i32)
            );
        }
        // The smooth depression tilts towards its centre from either side.
        assert!(normal(&field, -3, -3)[0] > 0.0);
        assert!(normal(&field, 2, -3)[0] < 0.0);
    }

    #[test]
    fn normals_match_physical_height_and_opengl_green_direction() {
        let mut field = vec![Surface::default(); SIZE * SIZE];
        let spacing = REPEAT_METRES / SIZE as f32;
        for y in 0..SIZE {
            for x in 0..SIZE {
                field[y * SIZE + x].height = (0.23 * x as f32 - 0.17 * y as f32) * spacing;
            }
        }
        let n = normal(&field, 128, 176);
        // Orthogonality to the two independently constructed displaced world
        // tangents also catches a height-unit error or an inverted green axis.
        let world_normal = [n[0], -n[1], n[2]];
        assert!((world_normal[0] + 0.23 * world_normal[2]).abs() < 2e-5);
        assert!((world_normal[1] - 0.17 * world_normal[2]).abs() < 2e-5);
        assert!(n[0] < 0.0 && n[1] < 0.0);
    }

    #[test]
    fn source_is_deterministic_neutral_and_has_useful_relief() {
        let source = texture();
        let repeated = cook();
        assert_eq!(source.name, NAME);
        assert_eq!(source.rgba, repeated.rgba);
        assert_eq!(source.normal, repeated.normal);
        assert_eq!(source.rgba.dimensions(), (SIZE as u32, SIZE as u32));
        let mut min_color = 255;
        let mut max_color = 0;
        for pixel in source.rgba.pixels() {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            assert!((0.645..=1.0).contains(&linear(pixel[0])));
            assert_eq!(pixel[3], 255);
            min_color = min_color.min(pixel[0]);
            max_color = max_color.max(pixel[0]);
        }
        assert!(max_color - min_color > 12);
        let normals = source.normal.unwrap();
        let mut mean = [0.0f64; 2];
        let mut energy = [0.0f64; 2];
        let mut positive = [0usize; 2];
        let mut negative = [0usize; 2];
        let heights = height_field();
        for (pixel, surface) in normals.pixels().zip(&heights) {
            let decoded = (pixel[2] as f32 / 255.0 - 0.5) * HEIGHT_RANGE_METRES;
            assert!((decoded - surface.height).abs() <= HEIGHT_RANGE_METRES / 510.0 + 1e-6,
                "Packed height must preserve relief without clipping: {} -> {decoded}", surface.height);
            assert!((204..=255).contains(&pixel[3]));
            // Narrow fracture ledges can tilt much more than the fine grain,
            // but stay in the outward hemisphere without vertical walls.
            // B carries physical height; Z is reconstructed from normal XY.
            assert!(pixel[2] > 0 && pixel[2] < 255);
            for axis in 0..2 {
                let value = pixel[axis] as f64 / 127.5 - 1.0;
                mean[axis] += value;
                energy[axis] += value * value;
                positive[axis] += usize::from(value > 0.02);
                negative[axis] += usize::from(value < -0.02);
            }
        }
        for axis in 0..2 {
            assert!((mean[axis] / (SIZE * SIZE) as f64).abs() < 0.012);
            let rms = (energy[axis] / (SIZE * SIZE) as f64).sqrt();
            assert!((0.04..0.4).contains(&rms), "normal RMS {axis}: {rms}");
            assert!(positive[axis] > SIZE * SIZE / 5 && negative[axis] > SIZE * SIZE / 5);
        }
    }

    #[test]
    fn real_mip_chain_retains_albedo_and_roughness_without_far_sparkle() {
        let source = texture();
        // Match frame::texture_array's deployed upload paths, including the
        // base resize. Stone is opaque, but its albedo layer still receives
        // the standard preserve-alpha setting used by unblended materials.
        let albedo_settings = crate::texture_cache::Settings {
            size: crate::frame::TEXTURE_SIZE,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            fill: [255; 4],
            preserve_alpha: true,
            linear_cutout_rgb: true,
        };
        let normal_settings = crate::texture_cache::Settings {
            size: 1024, // frame::NORMAL_SIZE; the array constant is private.
            format: wgpu::TextureFormat::Rgba8Unorm,
            fill: [128, 128, 255, 255],
            preserve_alpha: false,
            linear_cutout_rgb: true,
        };
        let (mips, _) =
            crate::frame::texture_mips::build(Some(&source.rgba), albedo_settings, false, 0);
        let linear_table: [f64; 256] = std::array::from_fn(|byte| linear(byte as u8));
        let mean = |image: &RgbaImage, channel: usize| {
            image
                .pixels()
                .map(|p| {
                    if channel == 0 {
                        linear_table[p[0] as usize]
                    } else {
                        p[channel] as f64 / 255.0
                    }
                })
                .sum::<f64>()
                / (image.width() * image.height()) as f64
        };
        let base_albedo = mean(&source.rgba, 0);
        assert_eq!(mips[0].dimensions(), (2048, 2048));
        assert_eq!(mips.len(), 12);
        for mip in &mips {
            assert!((mean(mip, 0) - base_albedo).abs() < 0.025);
            assert_eq!(mean(mip, 3), 1.0);
        }
        let (normals, _) =
            crate::frame::texture_mips::build(source.normal.as_ref(), normal_settings, false, 0);
        assert_eq!(normals[0].dimensions(), (1024, 1024));
        assert_eq!(normals.len(), 11);
        let base_roughness = mean(source.normal.as_ref().unwrap(), 3);
        assert!((base_roughness - 0.93).abs() < 0.015);
        for mip in &normals {
            assert!((mean(mip, 3) - base_roughness).abs() < 0.015);
        }
        for mip in normals.iter().filter(|mip| mip.width() <= 4) {
            for pixel in mip.pixels() {
                let limit = if mip.width() == 1 { 3 } else { 8 };
                assert!((pixel[0] as i16 - 128).abs() <= limit);
                assert!((pixel[1] as i16 - 128).abs() <= limit);
            }
        }
    }

    #[test]
    fn scene_reuses_the_shared_layer() {
        let mut scene = Scene::default();
        scene.textures.push(TextureSource {
            name: "unrelated-material".to_owned(),
            rgba: RgbaImage::new(1, 1),
            normal: None,
            orm: None,
        });
        let first = add_to_scene(&mut scene);
        let second = add_to_scene(&mut scene);
        assert_eq!(first, 1);
        assert_eq!(second, first);
        assert_eq!(scene.textures.len(), 2);
    }
}
