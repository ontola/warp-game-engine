//! A shared, deterministic quarried-stone surface, cooked once on the CPU.
//!
//! RGB is neutral sRGB albedo, intended to multiply the material's stone tint.
//! Albedo alpha is opaque; normal alpha is a **linear roughness multiplier**
//! in 0.80..=1.00. The linear-UNORM normal uses OpenGL's +U / -V tangent frame. Both maps
//! describe the same 1.5 m periodic height field; there is no baked lighting.
//! The ordinary texture-array mip chain filters away detail at distance.

use crate::scene::{Scene, TextureSource};
use image::{Rgba, RgbaImage};
use std::sync::OnceLock;

/// Physical width of one repeat. Keep the shader's stone mapping in sync.
pub const REPEAT_METRES: f32 = 1.5;

const SIZE: usize = 512;
const NAME: &str = "warp-quarried-stone-v1";
static SURFACE: OnceLock<TextureSource> = OnceLock::new();

/// Return an owned source for the renderer; the expensive bake happens once.
pub fn texture() -> TextureSource {
    let source = SURFACE.get_or_init(cook);
    TextureSource {
        name: source.name.clone(),
        rgba: source.rgba.clone(),
        normal: source.normal.clone(),
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

#[derive(Clone, Copy, Default)]
struct Facet {
    position: [f32; 2],
    slope: [f32; 2],
    height: f32,
    albedo: f32,
}

// Squared-distance band in lattice-cell units. With 15 cm cells this softens
// a typical plate boundary over ~15 mm, retaining broad, genuinely flat faces.
const FACET_BLEND: f32 = 0.10;

/// Blend ALL planes whose Voronoi regions meet the narrow transition band.
/// A closest-distance/nearest-two blend introduces derivative jumps where a
/// third plane enters. Pairwise compact C2 weights avoid that: a nearest
/// site always has weight one, and an isolated face is exactly its plane.
fn sample_facets(sites: &[Facet], p: [f32; 2], metres_per_cell: f32) -> Surface {
    let mut distances = [0.0f32; 25];
    let mut minimum = f32::MAX;
    for (i, site) in sites.iter().enumerate() {
        let dx = p[0] - site.position[0];
        let dy = p[1] - site.position[1];
        distances[i] = dx * dx + dy * dy;
        minimum = minimum.min(distances[i]);
    }
    let mut active = [0usize; 25];
    let mut count = 0;
    for (i, &distance) in distances[..sites.len()].iter().enumerate() {
        if distance - minimum < FACET_BLEND {
            active[count] = i;
            count += 1;
        }
    }
    let mut result = Surface::default();
    let mut total = 0.0;
    for &i in &active[..count] {
        let mut weight = 1.0;
        for &j in &active[..count] {
            // A rejected site is farther than every active site, so it
            // contributes factor one here. Pruning cannot introduce a seam.
            weight *= 1.0 - smootherstep((distances[i] - distances[j]) / FACET_BLEND);
        }
        let site = sites[i];
        let height = site.height
            + ((p[0] - site.position[0]) * site.slope[0]
                + (p[1] - site.position[1]) * site.slope[1])
                * metres_per_cell;
        result.height += weight * height;
        result.albedo += weight * site.albedo;
        total += weight;
    }
    result.height /= total;
    result.albedo /= total;
    result
}

/// Periodic, jittered fracture plates, mostly 8..25 cm across. Neighbourhoods
/// are prepared per cell once, keeping modulo and hashing out of the 25-site
/// inner loop. Five-by-five includes all possible competitors at cell edges.
struct Facets {
    period: usize,
    neighbourhoods: Vec<[Facet; 25]>,
}

impl Facets {
    fn new(period: usize, seed: u32) -> Self {
        let sites: Vec<_> = (0..period * period)
            .map(|i| {
                let i = i as u32;
                let (sn, cs) = (random(i, seed ^ 0x8211) * std::f32::consts::TAU).sin_cos();
                let slope = 0.025 + random(i, seed ^ 0x8212) * 0.055;
                Facet {
                    position: [
                        0.22 + random(i, seed ^ 0x8213) * 0.56,
                        0.22 + random(i, seed ^ 0x8214) * 0.56,
                    ],
                    slope: [cs * slope, sn * slope],
                    height: (random(i, seed ^ 0x8215) - 0.5) * 0.006,
                    albedo: (random(i, seed ^ 0x8216) - 0.5) * 0.045,
                }
            })
            .collect();
        let neighbourhoods = (0..period * period)
            .map(|i| {
                std::array::from_fn(|j| {
                    let dx = (j % 5) as i32 - 2;
                    let dy = (j / 5) as i32 - 2;
                    let x = ((i % period) as i32 + dx).rem_euclid(period as i32) as usize;
                    let y = ((i / period) as i32 + dy).rem_euclid(period as i32) as usize;
                    let mut site = sites[y * period + x];
                    site.position[0] += dx as f32;
                    site.position[1] += dy as f32;
                    site
                })
            })
            .collect();
        Self {
            period,
            neighbourhoods,
        }
    }

    fn sample(&self, u: f32, v: f32) -> Surface {
        let x = u * self.period as f32;
        let y = v * self.period as f32;
        let ix = (x.floor() as i32).rem_euclid(self.period as i32) as usize;
        let iy = (y.floor() as i32).rem_euclid(self.period as i32) as usize;
        sample_facets(
            &self.neighbourhoods[iy * self.period + ix],
            [x - x.floor(), y - y.floor()],
            REPEAT_METRES / self.period as f32,
        )
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

/// Small four-sided chisel scallops. The interior walls are planar; only
/// their ridges and outer rim are softened, unlike a rounded noise dimple.
fn chisel(field: &mut [Surface], center: [f32; 2], radius: [f32; 2], angle: f32, depth: f32) {
    let (sn, cs) = angle.sin_cos();
    let bound = radius[0].max(radius[1]) * 1.1;
    let ramp = |t: f32| {
        let width = 0.12;
        if t <= 0.0 {
            0.0
        } else if t >= width {
            t - width * 0.5
        } else {
            let u = t / width;
            width * u * u * u * (1.0 - u * 0.5)
        }
    };
    for y in (center[1] - bound).floor() as i32..=(center[1] + bound).ceil() as i32 {
        for x in (center[0] - bound).floor() as i32..=(center[0] + bound).ceil() as i32 {
            let dx = x as f32 + 0.5 - center[0];
            let dy = y as f32 + 0.5 - center[1];
            let rx = (cs * dx + sn * dy) / radius[0];
            let ry = (-sn * dx + cs * dy) / radius[1];
            let diamond = (rx * rx + 0.0025).sqrt() + (ry * ry + 0.0025).sqrt() - 0.1;
            field[wrapped_index(x, y)].add(
                ramp(1.0 - diamond),
                Surface {
                    height: -depth,
                    albedo: -0.008,
                    roughness: 0.012,
                },
            );
        }
    }
}

/// A short, round-ended fracture segment. Compact support avoids dark webs;
/// joining a few shallow segments gives the break an irregular direction.
fn fracture(field: &mut [Surface], a: [f32; 2], b: [f32; 2], width: f32) {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let inverse_length_squared = 1.0 / (delta[0] * delta[0] + delta[1] * delta[1]).max(1e-6);
    let inverse_width_squared = 1.0 / (width * width);
    let detail = Surface {
        height: -0.00065,
        albedo: -0.035,
        roughness: 0.018,
    };
    for y in (a[1].min(b[1]) - width).floor() as i32..=(a[1].max(b[1]) + width).ceil() as i32 {
        for x in (a[0].min(b[0]) - width).floor() as i32..=(a[0].max(b[0]) + width).ceil() as i32 {
            let px = x as f32 + 0.5 - a[0];
            let py = y as f32 + 0.5 - a[1];
            let t = ((px * delta[0] + py * delta[1]) * inverse_length_squared).clamp(0.0, 1.0);
            let dx = px - t * delta[0];
            let dy = py - t * delta[1];
            field[wrapped_index(x, y)]
                .add(kernel((dx * dx + dy * dy) * inverse_width_squared), detail);
        }
    }
}

fn height_field() -> Vec<Surface> {
    let facets = Facets::new(10, 0x17bc_930d);
    let mottling = Noise::new(11, 0xa51c_8e29);
    let chips = Noise::new(31, 0x63d8_35a7);
    let grains = Noise::new(83, 0xc02b_79e3);
    let grit = Noise::new(181, 0x95f1_b863);
    let mut field = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) / SIZE as f32;
            let v = (y as f32 + 0.5) / SIZE as f32;
            let coarse = chips.sample(u, v);
            let medium = grains.sample(u, v);
            let fine = grit.sample(u, v);
            let plate = facets.sample(u, v);
            field.push(Surface {
                // Broad angular plates carry the 3..12 mm fracture relief;
                // subdued 8..48 mm grain roughens their otherwise flat faces.
                height: plate.height + coarse * 0.00045 + medium * 0.00055 + fine * 0.00019,
                albedo: 0.87
                    + plate.albedo
                    + mottling.sample(u, v) * 0.035
                    + coarse * 0.035
                    + medium * 0.025,
                roughness: 0.93 + medium * 0.025 - fine * 0.018,
            });
        }
    }

    // Sparse chipped cavities, fine pores, and mixed pale/dark mineral grains.
    // A seed is local to each feature family, so changes do not reshuffle all
    // other features and no global random generator or source asset is needed.
    for i in 0..180 {
        let radius = 4.0 + random(i, 0x1981) * 8.0;
        chisel(
            &mut field,
            [
                random(i, 0x1982) * SIZE as f32,
                random(i, 0x1983) * SIZE as f32,
            ],
            [radius, radius * (0.45 + random(i, 0x1984) * 0.45)],
            random(i, 0x1985) * std::f32::consts::TAU,
            0.0015 + random(i, 0x1986) * 0.0035,
        );
    }
    for i in 0..240 {
        let radius = 3.0 + random(i, 0x2341) * 6.0;
        grain(
            &mut field,
            [
                random(i, 0x2342) * SIZE as f32,
                random(i, 0x2343) * SIZE as f32,
            ],
            [radius, radius * (0.35 + random(i, 0x2344) * 0.45)],
            random(i, 0x2345) * std::f32::consts::TAU,
            Surface {
                height: -0.0014,
                albedo: -0.025,
                roughness: 0.028,
            },
        );
    }
    for i in 0..1900 {
        let radius = 1.2 + random(i, 0x3811) * 2.6;
        grain(
            &mut field,
            [
                random(i, 0x3812) * SIZE as f32,
                random(i, 0x3813) * SIZE as f32,
            ],
            [radius, radius * (0.55 + random(i, 0x3814) * 0.45)],
            random(i, 0x3815) * std::f32::consts::TAU,
            Surface {
                height: -0.00055 - random(i, 0x3816) * 0.0008,
                albedo: -0.045,
                roughness: 0.035,
            },
        );
    }
    for i in 0..1500 {
        let radius = 0.9 + random(i, 0x4c21) * 1.9;
        let pale = random(i, 0x4c22) > 0.36;
        grain(
            &mut field,
            [
                random(i, 0x4c23) * SIZE as f32,
                random(i, 0x4c24) * SIZE as f32,
            ],
            [radius, radius * (0.45 + random(i, 0x4c25) * 0.5)],
            random(i, 0x4c26) * std::f32::consts::TAU,
            Surface {
                height: if pale { 0.00035 } else { -0.0002 },
                albedo: if pale { 0.075 } else { -0.07 },
                roughness: -0.065,
            },
        );
    }
    for i in 0..48 {
        let mut a = [
            random(i, 0x5711) * SIZE as f32,
            random(i, 0x5712) * SIZE as f32,
        ];
        let angle = random(i, 0x5713) * std::f32::consts::TAU;
        let step = 4.0 + random(i, 0x5714) * 12.0;
        let width = 1.1 + random(i, 0x5715) * 1.2;
        for segment in 0..3 {
            let turn = angle + (random(i * 3 + segment, 0x5716) - 0.5) * 0.7;
            let (sn, cs) = turn.sin_cos();
            let b = [a[0] + cs * step, a[1] + sn * step];
            fracture(&mut field, a, b, width);
            a = b;
        }
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
                    encode(n[2]),
                    (sample.roughness.clamp(0.8, 1.0) * 255.0).round() as u8,
                ]),
            );
        }
    }
    TextureSource {
        name: NAME.to_owned(),
        rgba,
        normal: Some(normals),
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
    fn fracture_plates_are_planar_inside_and_periodic_at_cell_and_tile_edges() {
        let facets = Facets::new(10, 0x17bc_930d);
        let cell_metres = REPEAT_METRES / facets.period as f32;
        for (i, sites) in facets.neighbourhoods.iter().enumerate() {
            let site = sites[12];
            let u = ((i % facets.period) as f32 + site.position[0]) / facets.period as f32;
            let v = ((i / facets.period) as f32 + site.position[1]) / facets.period as f32;
            for [x, y] in [[0.0, 0.0], [0.015, -0.02], [-0.015, 0.02]] {
                let sampled =
                    facets.sample(u + x / facets.period as f32, v + y / facets.period as f32);
                let plane = site.height + (x * site.slope[0] + y * site.slope[1]) * cell_metres;
                assert!((sampled.height - plane).abs() < 2e-7);
            }
        }
        // These boundaries change the finite candidate window; they must not
        // change the height or slope even when a blend spans several cells.
        // A first-order left/right difference measures curvature as well as
        // a seam: the narrow ledges can change slope appreciably over 30 um.
        // Binary steps also avoid unequal f32 coordinate rounding at the
        // two sides. Evaluate second-order slopes with f64 accumulation.
        let step = 1.0f32 / 32768.0;
        for boundary in [0.0, 0.1, 0.4, 0.9, 1.0] {
            for v in [0.0713, 0.4187, 0.935] {
                for transpose in [false, true] {
                    let sample = |u| {
                        if transpose {
                            facets.sample(v, u).height
                        } else {
                            facets.sample(u, v).height
                        }
                    };
                    assert!((sample(boundary) - sample(boundary + 1.0)).abs() < 2e-6);
                    let slope_gap = |h: f32| {
                        let center = sample(boundary) as f64;
                        let left = (3.0 * center - 4.0 * sample(boundary - h) as f64
                            + sample(boundary - 2.0 * h) as f64)
                            / (2.0 * h as f64);
                        let right = (-3.0 * center + 4.0 * sample(boundary + h) as f64
                            - sample(boundary + 2.0 * h) as f64)
                            / (2.0 * h as f64);
                        (left - right).abs()
                    };
                    let coarse = slope_gap(step);
                    let fine = slope_gap(step * 0.5);
                    assert!(
                        fine < 0.001,
                        "plate seam at {boundary}, {v}, transpose={transpose}: {fine}"
                    );
                    // Below this roundoff floor, further shrinking the step
                    // amplifies source f32 noise rather than resolving a seam.
                    assert!(
                        fine <= coarse * 0.75 + 0.0003,
                        "plate slope failed to converge: {coarse} -> {fine}"
                    );
                }
            }
        }
    }

    #[test]
    fn third_plate_does_not_kink_nearest_site_boundaries() {
        // The third site is within the blend band, but not tied for nearest.
        // A distance-minus-nearest weighting fails this case despite passing
        // a two-plane boundary or a perfectly symmetric three-way junction.
        let sites = [
            Facet {
                position: [-0.5, 0.0],
                height: -0.003,
                slope: [0.03, 0.01],
                ..Facet::default()
            },
            Facet {
                position: [0.5, 0.0],
                height: 0.004,
                slope: [-0.02, 0.04],
                ..Facet::default()
            },
            Facet {
                position: [0.0, 0.28f32.sqrt()],
                height: 0.001,
                slope: [0.01, -0.03],
                ..Facet::default()
            },
        ];
        let sample = |x| sample_facets(&sites, [x, 0.0], 0.15).height;
        for step in [0.001, 0.0003] {
            let left = (sample(0.0) - sample(-step)) / step;
            let right = (sample(step) - sample(0.0)) / step;
            assert!(
                (left - right).abs() < 0.003,
                "third-site kink: {left} / {right}"
            );
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
        fracture(&mut field, [-5.0, 1.0], [7.0, 1.0], 1.5);
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
        assert_eq!(source.rgba.dimensions(), (512, 512));
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
        for pixel in normals.pixels() {
            assert!((204..=255).contains(&pixel[3]));
            // Narrow fracture ledges can tilt much more than the fine grain,
            // but stay in the outward hemisphere without vertical walls.
            assert!(pixel[2] > 160);
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
        });
        let first = add_to_scene(&mut scene);
        let second = add_to_scene(&mut scene);
        assert_eq!(first, 1);
        assert_eq!(second, first);
        assert_eq!(scene.textures.len(), 2);
    }
}
