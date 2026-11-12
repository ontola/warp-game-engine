//! Surfaces of a home, drawn in code: tileable albedo and normal maps for
//! plaster, brick, oak planks, floor tiles and a flat roof, at real sizes.
//! No image files: every texture is a function of its pixel, so the look is
//! tuned here and nothing has to be downloaded or licensed.
use image::{Rgba, RgbaImage};
use render::scene::{Material, Scene, TWO_SIDED};

/// What a surface is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Painted plaster, inside walls and ceilings.
    Plaster,
    /// Facing brick, outside walls.
    Brick,
    /// Oak planks, living rooms and bedrooms.
    Oak,
    /// Large stone-look tiles, halls, kitchens and wet rooms.
    Tile,
    /// Gravel on a flat roof.
    Roof,
    /// Painted window and door frames.
    Frame,
    /// Upholstery and cabinets.
    Furniture,
}

/// Texture size in pixels (square, tileable).
const SIZE: u32 = 512;

impl Surface {
    /// Metres covered by one repeat of the texture.
    pub fn tile_metres(self) -> f32 {
        match self { Surface::Plaster => 2.0, Surface::Brick => 1.0, Surface::Oak => 2.4, Surface::Tile => 1.2, Surface::Roof => 1.0, Surface::Frame | Surface::Furniture => 1.0 }
    }

    /// Add this surface's texture (if any) and material to `scene`.
    pub fn add(self, scene: &mut Scene) -> u32 {
        let (tint, roughness, layer) = match self {
            Surface::Frame => ([0.08, 0.085, 0.09], 0.45, -1),
            Surface::Furniture => ([0.55, 0.52, 0.48], 0.85, -1),
            _ => {
                // One texture per surface per scene, however many houses.
                let name = format!("architecture/{self:?}");
                let layer = match scene.textures.iter().position(|t| t.name == name) {
                    Some(l) => l as i32,
                    None => {
                        let (albedo, normal) = self.draw();
                        let layer = scene.add_image(&name, albedo);
                        scene.textures[layer as usize].normal = Some(normal);
                        layer
                    }
                };
                let rough = match self { Surface::Oak => 0.45, Surface::Tile => 0.3, Surface::Plaster => 0.9, _ => 0.85 };
                ([1.0; 3], rough, layer)
            }
        };
        scene.add_material(Material { tint, layer, roughness, flags: if self == Surface::Frame { TWO_SIDED } else { 0 }, scale: 1.0, metal: 0.0 })
    }

    /// Albedo (sRGB) and height-derived normal map.
    fn draw(self) -> (RgbaImage, RgbaImage) {
        let n = SIZE as f32;
        let mut height = vec![0f32; (SIZE * SIZE) as usize];
        let mut albedo = RgbaImage::new(SIZE, SIZE);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (u, v) = (x as f32 / n, y as f32 / n);
                let (c, h) = self.texel(u, v);
                height[(y * SIZE + x) as usize] = h;
                albedo.put_pixel(x, y, Rgba([to_srgb(c[0]), to_srgb(c[1]), to_srgb(c[2]), 255]));
            }
        }
        // Normal from height differences (wrapping), in metres per metre.
        let texel_m = self.tile_metres() / n;
        let at = |x: i32, y: i32| height[((y.rem_euclid(SIZE as i32)) as u32 * SIZE + x.rem_euclid(SIZE as i32) as u32) as usize];
        let normal = RgbaImage::from_fn(SIZE, SIZE, |x, y| {
            let (x, y) = (x as i32, y as i32);
            let dx = (at(x + 1, y) - at(x - 1, y)) / (2.0 * texel_m);
            let dy = (at(x, y + 1) - at(x, y - 1)) / (2.0 * texel_m);
            // OpenGL convention: green points to the image's top row.
            let v = glam::Vec3::new(-dx, dy, 1.0).normalize();
            Rgba([((v.x * 0.5 + 0.5) * 255.0) as u8, ((v.y * 0.5 + 0.5) * 255.0) as u8, ((v.z * 0.5 + 0.5) * 255.0) as u8, 255])
        });
        (albedo, normal)
    }

    /// Linear colour and height in metres at (u, v) in 0..1 of one repeat.
    fn texel(self, u: f32, v: f32) -> ([f32; 3], f32) {
        let m = self.tile_metres();
        let (px, py) = (u * m, v * m);
        match self {
            Surface::Plaster => {
                let a = fbm(u, v, 8, 4, 1) * 0.04 + fbm(u, v, 64, 2, 2) * 0.015;
                let c = 0.80 + a;
                ([c, c * 0.985, c * 0.96], a * 0.0006)
            }
            Surface::Brick => {
                // Running bond: 0.25 m x 0.0625 m modules (Dutch waalformaat
                // with 10 mm joints), half-brick offset every course.
                let course = (py / 0.0625).floor();
                let off = if course as i32 % 2 == 0 { 0.0 } else { 0.125 };
                let col = ((px + off) / 0.25).floor();
                let (bx, by) = (((px + off) / 0.25).fract() * 0.25, (py / 0.0625).fract() * 0.0625);
                let joint = bx < 0.01 || by < 0.0125;
                let id = hash(col as i32 & 3, course as i32 & 15, 7);
                if joint {
                    let g = 0.42 + fbm(u, v, 64, 2, 9) * 0.05;
                    ([g, g * 0.97, g * 0.92], -0.008)
                } else {
                    let t = 0.75 + id * 0.35 + fbm(u, v, 32, 3, 11) * 0.12;
                    let burnt = hash(col as i32 & 3, course as i32 & 15, 13) < 0.15;
                    let base = if burnt { [0.16, 0.08, 0.05] } else { [0.36, 0.13, 0.07] };
                    ([base[0] * t, base[1] * t, base[2] * t], fbm(u, v, 48, 2, 5) * 0.0015)
                }
            }
            Surface::Oak => {
                // 0.2 m planks along u, lengths of 1.2 m staggered per row.
                let row = (py / 0.2).floor();
                let off = hash(row as i32 & 15, 0, 3) * 1.2;
                let seg = ((px + off) / 1.2).floor();
                let (lx, ly) = (((px + off) / 1.2).fract() * 1.2, (py / 0.2).fract() * 0.2);
                let groove = lx < 0.002 || ly < 0.0015;
                let id = hash(seg as i32 & 1, row as i32 & 15, 21);
                // Grain: noise stretched along the plank.
                let grain = fbm(u * 0.15 + id, v * 4.0, 16, 4, 31);
                let ring = ((grain * 9.0 + id * 6.0).sin() * 0.5 + 0.5).powf(3.0);
                let t = 0.8 + id * 0.35 - ring * 0.18;
                let c = [0.42 * t, 0.26 * t, 0.13 * t];
                if groove { ([c[0] * 0.4, c[1] * 0.4, c[2] * 0.4], -0.0015) } else { (c, ring * 0.0002) }
            }
            Surface::Tile => {
                // 0.6 m tiles, 3 mm grout.
                let (tx, ty) = ((px / 0.6).fract() * 0.6, (py / 0.6).fract() * 0.6);
                let grout = tx < 0.003 || ty < 0.003;
                let id = hash((px / 0.6) as i32, (py / 0.6) as i32, 41);
                if grout { ([0.32, 0.31, 0.29], -0.002) } else {
                    let g = 0.52 + id * 0.04 + fbm(u, v, 12, 4, 43) * 0.06;
                    ([g, g * 0.98, g * 0.95], 0.0)
                }
            }
            Surface::Roof => {
                let g = 0.12 + fbm(u, v, 128, 2, 51) * 0.08;
                ([g, g, g * 0.95], fbm(u, v, 128, 2, 51) * 0.004)
            }
            Surface::Frame | Surface::Furniture => ([0.5; 3], 0.0),
        }
    }
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Hash of integer coordinates to 0..1.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 0xffffff as f32
}

/// Tileable value noise: `cells` cells over 0..1, wrapping.
fn value(u: f32, v: f32, cells: i32, seed: u32) -> f32 {
    let (x, y) = (u * cells as f32, v * cells as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |a: i32, b: i32| hash(a.rem_euclid(cells), b.rem_euclid(cells), seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bottom = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bottom - top) * sy - 0.5
}

/// Fractal value noise, roughly -0.5..0.5, tileable.
fn fbm(u: f32, v: f32, cells: i32, octaves: u32, seed: u32) -> f32 {
    (0..octaves).map(|o| value(u, v, cells << o, seed + o) / (1 << o) as f32).sum::<f32>()
}
