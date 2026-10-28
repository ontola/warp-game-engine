//! The bits of Godot the lobby's room builder leans on, reproduced exactly:
//! Color maths, Image pixel writes, Basis rotations and the primitive meshes
//! (BoxMesh, CylinderMesh, SphereMesh, PlaneMesh, QuadMesh), vertex for vertex.

use glam::{Affine3A, EulerRot, Mat3A, Quat, Vec3};

/// Godot's Color: single floats, sRGB-encoded, may exceed 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

pub const fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

impl Color {
    pub const WHITE: Color = rgb(1.0, 1.0, 1.0);
    pub const BLACK: Color = rgb(0.0, 0.0, 0.0);
    pub const TRANSPARENT: Color = rgba(1.0, 1.0, 1.0, 0.0);

    /// `Color("5a3826")` or `Color("#5a3826")`.
    pub fn hex(s: &str) -> Color {
        let s = s.trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
        Color { r: byte(0), g: byte(2), b: byte(4), a: if s.len() >= 8 { byte(6) } else { 1.0 } }
    }

    /// `Color.from_string(s, default)`: a hex colour, or the default.
    pub fn from_string(s: &str, default: Color) -> Color {
        let h = s.trim_start_matches('#');
        if (h.len() == 6 || h.len() == 8) && h.chars().all(|c| c.is_ascii_hexdigit()) { Color::hex(h) } else { default }
    }

    pub fn from_hsv(h: f32, s: f32, v: f32) -> Color {
        if s == 0.0 {
            return rgb(v, v, v);
        }
        let h = (h * 6.0) % 6.0;
        let i = h.floor() as i32;
        let f = h - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        match i {
            0 => rgb(v, t, p),
            1 => rgb(q, v, p),
            2 => rgb(p, v, t),
            3 => rgb(p, q, v),
            4 => rgb(t, p, v),
            _ => rgb(v, p, q),
        }
    }

    pub fn lerp(self, to: Color, t: f32) -> Color {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Color { r: l(self.r, to.r), g: l(self.g, to.g), b: l(self.b, to.b), a: l(self.a, to.a) }
    }

    pub fn lightened(self, amount: f32) -> Color {
        let l = |c: f32| c + (1.0 - c) * amount;
        Color { r: l(self.r), g: l(self.g), b: l(self.b), a: self.a }
    }

    pub fn darkened(self, amount: f32) -> Color {
        let d = |c: f32| c * (1.0 - amount);
        Color { r: d(self.r), g: d(self.g), b: d(self.b), a: self.a }
    }

    pub fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    pub fn array(self) -> [f64; 4] {
        [self.r as f64, self.g as f64, self.b as f64, self.a as f64]
    }

    /// Image::set_pixel for RGBA8: truncated, clamped.
    pub fn rgba8(self) -> [u8; 4] {
        let c = |v: f32| (v * 255.0).clamp(0.0, 255.0) as u8;
        [c(self.r), c(self.g), c(self.b), c(self.a)]
    }
}

pub fn set_pixel(img: &mut image::RgbaImage, x: i64, y: i64, c: Color) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, image::Rgba(c.rgba8()));
    }
}

/// Godot's Node3D `rotation` (Euler YXZ) as a basis.
pub fn euler(x: f32, y: f32, z: f32) -> Mat3A {
    Mat3A::from_quat(Quat::from_euler(EulerRot::YXZ, y, x, z))
}

/// `Basis.looking_at(target - eye, up)`: -z towards the target.
pub fn looking_at(dir: Vec3, up: Vec3) -> Mat3A {
    let z = -dir.normalize();
    let x = up.cross(z).normalize();
    let y = z.cross(x);
    Mat3A::from_cols(x.into(), y.into(), z.into())
}

/// A node at `pos` with `basis`, as Godot's Transform3D.
pub fn xf(basis: Mat3A, pos: Vec3) -> Affine3A {
    Affine3A { matrix3: basis, translation: pos.into() }
}

/// One surface as Godot generates it: indices in Godot's (clockwise) order.
#[derive(Clone, Debug, Default)]
pub struct Arrays {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<i32>,
}

impl Arrays {
    fn push(&mut self, p: [f32; 3], n: [f32; 3], uv: [f32; 2]) {
        self.positions.push(p);
        self.normals.push(n);
        self.uvs.push(uv);
    }

    pub fn aabb(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for p in &self.positions {
            lo = lo.min(Vec3::from_array(*p));
            hi = hi.max(Vec3::from_array(*p));
        }
        (lo, hi)
    }
}

const ONE_THIRD: f32 = 1.0 / 3.0;
const TWO_THIRDS: f32 = 2.0 / 3.0;

/// The paired-face index pattern BoxMesh uses (two faces per vertex pair).
fn box_quad(a: &mut Arrays, prevrow: i32, thisrow: i32, i: i32) {
    let i2 = i * 2;
    a.indices.extend([prevrow + i2 - 2, prevrow + i2, thisrow + i2 - 2, prevrow + i2, thisrow + i2, thisrow + i2 - 2]);
    a.indices.extend([prevrow + i2 - 1, prevrow + i2 + 1, thisrow + i2 - 1, prevrow + i2 + 1, thisrow + i2 + 1, thisrow + i2 - 1]);
}

/// BoxMesh with no subdivisions.
pub fn box_mesh(size: Vec3) -> Arrays {
    let mut a = Arrays::default();
    let start = size * -0.5;
    let mut point = 0;
    // Front and back.
    let mut y = start.y;
    let mut thisrow = point;
    let mut prevrow = 0;
    for j in 0..=1 {
        let mut x = start.x;
        for i in 0..=1 {
            let u = i as f32 / 3.0;
            let v = j as f32 / 2.0;
            a.push([x, -y, -start.z], [0.0, 0.0, 1.0], [u, v]);
            a.push([-x, -y, start.z], [0.0, 0.0, -1.0], [TWO_THIRDS + u, v]);
            point += 2;
            if i > 0 && j > 0 {
                box_quad(&mut a, prevrow, thisrow, i);
            }
            x += size.x;
        }
        y += size.y;
        prevrow = thisrow;
        thisrow = point;
    }
    // Left and right.
    let mut y = start.y;
    thisrow = point;
    prevrow = 0;
    for j in 0..=1 {
        let mut z = start.z;
        for i in 0..=1 {
            let u = i as f32 / 3.0;
            let v = j as f32 / 2.0;
            a.push([-start.x, -y, -z], [1.0, 0.0, 0.0], [ONE_THIRD + u, v]);
            a.push([start.x, -y, z], [-1.0, 0.0, 0.0], [u, 0.5 + v]);
            point += 2;
            if i > 0 && j > 0 {
                box_quad(&mut a, prevrow, thisrow, i);
            }
            z += size.z;
        }
        y += size.y;
        prevrow = thisrow;
        thisrow = point;
    }
    // Top and bottom.
    let mut z = start.z;
    thisrow = point;
    prevrow = 0;
    for j in 0..=1 {
        let mut x = start.x;
        for i in 0..=1 {
            let u = i as f32 / 3.0;
            let v = j as f32 / 2.0;
            a.push([-x, -start.y, -z], [0.0, 1.0, 0.0], [ONE_THIRD + u, 0.5 + v]);
            a.push([x, start.y, -z], [0.0, -1.0, 0.0], [TWO_THIRDS + u, 0.5 + v]);
            point += 2;
            if i > 0 && j > 0 {
                box_quad(&mut a, prevrow, thisrow, i);
            }
            x += size.x;
        }
        z += size.z;
        prevrow = thisrow;
        thisrow = point;
    }
    let _ = point;
    a
}

/// CylinderMesh (64 radial segments, 4 rings, both caps by default).
pub fn cylinder_mesh(top: f32, bottom: f32, height: f32, radial: i32, rings: i32) -> Arrays {
    let mut a = Arrays::default();
    let mut point = 0;
    let mut thisrow = 0;
    let mut prevrow = 0;
    let side_normal_y = (bottom - top) / height;
    for j in 0..=rings + 1 {
        let v = j as f32 / (rings + 1) as f32;
        let radius = top + (bottom - top) * v;
        let y = height * 0.5 - height * v;
        for i in 0..=radial {
            let u = i as f32 / radial as f32;
            let x = (u * std::f32::consts::TAU).sin();
            let z = (u * std::f32::consts::TAU).cos();
            let n = Vec3::new(x, side_normal_y, z).normalize();
            a.push([x * radius, y, z * radius], n.to_array(), [u, v * 0.5]);
            point += 1;
            if i > 0 && j > 0 {
                a.indices.extend([prevrow + i - 1, prevrow + i, thisrow + i - 1, prevrow + i, thisrow + i, thisrow + i - 1]);
            }
        }
        prevrow = thisrow;
        thisrow = point;
    }
    if top > 0.0 {
        let y = height * 0.5;
        thisrow = point;
        a.push([0.0, y, 0.0], [0.0, 1.0, 0.0], [0.25, 0.75]);
        point += 1;
        for i in 0..=radial {
            let r = i as f32 / radial as f32;
            let x = (r * std::f32::consts::TAU).sin();
            let z = (r * std::f32::consts::TAU).cos();
            a.push([x * top, y, z * top], [0.0, 1.0, 0.0], [(x + 1.0) * 0.25, 0.5 + (z + 1.0) * 0.25]);
            point += 1;
            if i > 0 {
                a.indices.extend([thisrow, point - 1, point - 2]);
            }
        }
    }
    if bottom > 0.0 {
        let y = height * -0.5;
        thisrow = point;
        a.push([0.0, y, 0.0], [0.0, -1.0, 0.0], [0.75, 0.75]);
        point += 1;
        for i in 0..=radial {
            let r = i as f32 / radial as f32;
            let x = (r * std::f32::consts::TAU).sin();
            let z = (r * std::f32::consts::TAU).cos();
            a.push([x * bottom, y, z * bottom], [0.0, -1.0, 0.0], [0.5 + (x + 1.0) * 0.25, 1.0 - (z + 1.0) * 0.25]);
            point += 1;
            if i > 0 {
                a.indices.extend([thisrow, point - 2, point - 1]);
            }
        }
    }
    a
}

/// SphereMesh (64 radial segments, 32 rings by default).
pub fn sphere_mesh(radius: f32, height: f32, radial: i32, rings: i32) -> Arrays {
    let mut a = Arrays::default();
    let scale = height * 0.5;
    let mut point = 0;
    let mut thisrow = 0;
    let mut prevrow = 0;
    for j in 0..=rings + 1 {
        let v = j as f32 / (rings + 1) as f32;
        let w = (std::f32::consts::PI * v).sin();
        let y = scale * (std::f32::consts::PI * v).cos();
        for i in 0..=radial {
            let u = i as f32 / radial as f32;
            let x = (u * std::f32::consts::TAU).sin();
            let z = (u * std::f32::consts::TAU).cos();
            let n = Vec3::new(x * w * scale, radius * (y / scale), z * w * scale).normalize_or_zero();
            a.push([x * radius * w, y, z * radius * w], n.to_array(), [u, v]);
            point += 1;
            if i > 0 && j > 0 {
                a.indices.extend([prevrow + i - 1, prevrow + i, thisrow + i - 1, prevrow + i, thisrow + i, thisrow + i - 1]);
            }
        }
        prevrow = thisrow;
        thisrow = point;
    }
    a
}

/// PlaneMesh facing +y (`face_z` false) or QuadMesh facing +z (`face_z` true).
pub fn plane_mesh(w: f32, h: f32, face_z: bool) -> Arrays {
    let mut a = Arrays::default();
    let normal = if face_z { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    let mut z = -h * 0.5;
    let mut point = 0;
    let mut thisrow = 0;
    let mut prevrow = 0;
    for j in 0..=1 {
        let mut x = -w * 0.5;
        for i in 0..=1 {
            let (u, v) = (i as f32, j as f32);
            let p = if face_z { [-x, z, 0.0] } else { [-x, 0.0, -z] };
            a.push(p, normal, [1.0 - u, 1.0 - v]);
            point += 1;
            if i > 0 && j > 0 {
                a.indices.extend([prevrow + i - 1, prevrow + i, thisrow + i - 1, prevrow + i, thisrow + i, thisrow + i - 1]);
            }
            x += w;
        }
        z += h;
        prevrow = thisrow;
        thisrow = point;
    }
    a
}
