//! The visible face of a light: the panel, disc or bulb that emits it. A
//! game declares only the shape and size; the engine derives the rest from
//! the light, so what the eye sees, the light's specular highlight and its
//! reflections agree:
//!
//! - the face's radiance is the light's intensity over the face's area
//!   (a Lambertian emitter of luminance L and area A sends L A candela
//!   along its axis), so a brighter light has a brighter face;
//! - the light's source radius is that of the sphere with the face's
//!   projected area, which sizes its specular highlight
//!   (`sphere_light_specular` in clustered_lights.wgsl) and soft shadows;
//! - the face's material is marked as a light's face, so screen-space
//!   reflections leave it to the light's analytic specular instead of
//!   adding it a second time (and only while it is on screen).
//!
//! See engine/docs/interior-light.md.
use crate::headlights::LocalLight;
use crate::scene::{Material, EMISSIVE};
use glam::{Affine3A, Quat, Vec3};

/// The emitting surface of a light, facing along the light's direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightFace {
    /// A flat panel `width` by `depth` metres (a ceiling light).
    Rect { width_m: f32, depth_m: f32 },
    /// A flat disc (a downlight).
    Disc { radius_m: f32 },
    /// A ball (a bare bulb).
    Sphere { radius_m: f32 },
}

/// Thickness of the box drawn for a flat face: thin, but solid geometry.
const FLAT_M: f32 = 0.02;

impl LightFace {
    /// The face's area seen along the light's axis.
    pub fn area_m2(&self) -> f32 {
        match *self {
            Self::Rect { width_m, depth_m } => width_m * depth_m,
            Self::Disc { radius_m } | Self::Sphere { radius_m } => std::f32::consts::PI * radius_m * radius_m,
        }
    }

    /// The radius of the sphere with the same projected area: the light's
    /// source radius.
    pub fn equivalent_radius_m(&self) -> f32 { (self.area_m2() / std::f32::consts::PI).sqrt() }

    /// `light` with this face: its source radius from the face.
    pub fn apply(&self, light: LocalLight) -> LocalLight { LocalLight { source_radius_m: self.equivalent_radius_m(), ..light } }

    /// The face's radiance (nits, linear RGB) for `light`: intensity over area.
    pub fn radiance(&self, light: &LocalLight) -> Vec3 { light.intensity_cd / self.area_m2().max(1e-8) }

    /// The material that shows the face lit by `light`: emissive at
    /// [`Self::radiance`], marked as a light's face (a negative `scale`,
    /// see [`EMISSIVE`]).
    pub fn material(&self, light: &LocalLight) -> Material {
        let radiance = self.radiance(light);
        let peak = radiance.max_element().max(1e-8);
        Material { tint: (radiance / peak).to_array(), layer: -1, roughness: 1.0, flags: EMISSIVE, scale: -peak, metal: 0.0 }
    }

    /// The face's shape at unit size around the origin, emitting towards -Y
    /// for flat faces; place it with [`Self::transform`].
    pub fn primitive(&self, material: u32) -> crate::primitives::Primitive {
        match self {
            Self::Rect { .. } => crate::primitives::cube(material),
            Self::Disc { .. } => crate::primitives::cylinder(material, 1.0, 1.0, 24),
            Self::Sphere { .. } => crate::primitives::sphere(material, 8, 12),
        }
    }

    /// Where the face of `light` goes: its emitting side at the light's
    /// position, facing the light's direction (a flat face's thickness lies
    /// behind it). The light's own shadow should exclude it.
    pub fn transform(&self, light: &LocalLight) -> Affine3A {
        let down = light.direction.normalize_or(Vec3::NEG_Y);
        let turn = Quat::from_rotation_arc(Vec3::NEG_Y, down);
        match *self {
            Self::Rect { width_m, depth_m } => Affine3A::from_scale_rotation_translation(Vec3::new(width_m, FLAT_M, depth_m), turn, light.position - down * (FLAT_M * 0.5)),
            Self::Disc { radius_m } => Affine3A::from_scale_rotation_translation(Vec3::new(radius_m, FLAT_M, radius_m), turn, light.position - down * (FLAT_M * 0.5)),
            Self::Sphere { radius_m } => Affine3A::from_scale_rotation_translation(Vec3::splat(radius_m), turn, light.position),
        }
    }
}

impl LightFace {
    /// The face as the clustered light record's fifth vec4: xyz the face's
    /// first half axis (world, half the width or the radius), w half the
    /// depth for a rectangle, -1 for a disc; zeros for a sphere (its
    /// highlight is the sphere light's).
    pub fn gpu(&self, light: &LocalLight) -> [f32; 4] {
        let down = light.direction.normalize_or(Vec3::NEG_Y);
        let axis = Quat::from_rotation_arc(Vec3::NEG_Y, down) * Vec3::X;
        match *self {
            Self::Rect { width_m, depth_m } => (axis * (width_m * 0.5)).extend(depth_m * 0.5).to_array(),
            Self::Disc { radius_m } => (axis * radius_m).extend(-1.0).to_array(),
            Self::Sphere { .. } => [0.0; 4],
        }
    }
}

/// Linearly transformed cosines for GGX (Heitz et al. 2016), fitted to the
/// engine's specular by `examples/ltc_fit.rs`: 64 x 64 entries over
/// roughness and sqrt(1 - n.v), each the inverse matrix (m00, m02, m20, m22,
/// normalised by m11) and the albedo without Fresnel and its Schlick weight.
pub const LTC_SIZE: usize = 64;
const LTC_FLOATS: usize = 6;
const LTC_TABLE: &[u8] = include_bytes!("ltc_ggx.bin");
/// Vec4s the tables take on the GPU: two per entry (the matrix, then the
/// albedo pair and padding).
pub const LTC_VEC4S: u32 = (LTC_SIZE * LTC_SIZE * 2) as u32;

fn ltc_entry(i: usize) -> [f32; LTC_FLOATS] {
    std::array::from_fn(|k| f32::from_le_bytes(LTC_TABLE[(i * LTC_FLOATS + k) * 4..][..4].try_into().unwrap()))
}

/// The tables as the GPU reads them (`LTC_VEC4S` vec4s).
pub fn ltc_words() -> Vec<[f32; 4]> {
    (0..LTC_SIZE * LTC_SIZE).flat_map(|i| { let e = ltc_entry(i); [[e[0], e[1], e[2], e[3]], [e[4], e[5], 0.0, 0.0]] }).collect()
}

/// CPU twin of `ltc_*` in clustered_lights.wgsl, for tests: the integral
/// of the specular (no Fresnel) over the polygon `corners` (as seen from
/// `p` with normal `n`, view `v`), with its albedo.
#[cfg(test)]
fn ltc_polygon(n: Vec3, v: Vec3, p: Vec3, rough: f32, corners: &[Vec3]) -> f32 {
    let ndv = n.dot(v).clamp(1e-4, 1.0);
    let f = glam::Vec2::new(rough.clamp(0.0, 1.0), (1.0 - ndv).sqrt()) * (LTC_SIZE - 1) as f32;
    let i0 = f.floor().min(glam::Vec2::splat((LTC_SIZE - 2) as f32));
    let t = f - i0;
    let at = |x: usize, y: usize| ltc_entry(x + y * LTC_SIZE);
    let (x, y) = (i0.x as usize, i0.y as usize);
    let mut e = [0.0f32; LTC_FLOATS];
    for k in 0..LTC_FLOATS {
        let a = at(x, y)[k] * (1.0 - t.x) + at(x + 1, y)[k] * t.x;
        let b = at(x, y + 1)[k] * (1.0 - t.x) + at(x + 1, y + 1)[k] * t.x;
        e[k] = a * (1.0 - t.y) + b * t.y;
    }
    let minv = glam::Mat3::from_cols(Vec3::new(e[0], 0.0, e[1]), Vec3::Y, Vec3::new(e[2], 0.0, e[3]));
    let t1 = (v - n * n.dot(v)).normalize_or(n.any_orthonormal_vector());
    let t2 = n.cross(t1);
    let frame = glam::Mat3::from_cols(t1, t2, n).transpose();
    let m = minv * frame;
    let pts: Vec<Vec3> = corners.iter().map(|c| m * (*c - p)).collect();
    // Clip to the horizon (z >= 0).
    let mut clipped = Vec::new();
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        if a.z >= 0.0 { clipped.push(a); }
        if (a.z >= 0.0) != (b.z >= 0.0) { clipped.push(a + (b - a) * (a.z / (a.z - b.z))); }
    }
    if clipped.len() < 3 { return 0.0; }
    let mut sum = 0.0;
    for k in 0..clipped.len() {
        let (a, b) = (clipped[k].normalize(), clipped[(k + 1) % clipped.len()].normalize());
        let c = a.dot(b).clamp(-1.0, 1.0);
        let th = c.acos();
        sum += a.cross(b).z * if th > 1e-3 { th / th.sin() } else { 1.0 };
    }
    sum.abs() / (2.0 * std::f32::consts::PI) * e[4]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's specular times the cosine, without Fresnel (`ggx` in
    /// shade.wgsl with f0 = 1).
    fn ggx_cos(n: Vec3, v: Vec3, l: Vec3, rough: f32) -> f32 {
        let h = (v + l).normalize();
        let (ndl, ndv, ndh) = (n.dot(l).max(0.0), n.dot(v).max(1e-4), n.dot(h).max(0.0));
        let a = (rough * rough).max(2e-3);
        let a2 = a * a;
        let d = a2 / (std::f32::consts::PI * (ndh * ndh * (a2 - 1.0) + 1.0).powi(2));
        let vis = 0.5 / (ndl * (ndv * ndv * (1.0 - a2) + a2).sqrt() + ndv * (ndl * ndl * (1.0 - a2) + a2).sqrt() + 1e-5);
        d * vis * ndl
    }

    /// LTC against brute force over the area of a 0.5 m ceiling panel seen
    /// in a floor, for several roughnesses and views.
    #[test]
    fn ltc_matches_brute_force_on_a_panel() {
        let n = Vec3::Y;
        let centre = Vec3::new(0.0, 2.9, 0.0);
        let corners = [Vec3::new(-0.25, 2.9, -0.25), Vec3::new(0.25, 2.9, -0.25), Vec3::new(0.25, 2.9, 0.25), Vec3::new(-0.25, 2.9, 0.25)];
        // Energy-weighted error over all views, LTC against the equal-area
        // sphere (Karis' representative point).
        let (mut err_ltc, mut err_sphere, mut total) = (0.0f32, 0.0f32, 0.0f32);
        let radius = LightFace::Rect { width_m: 0.5, depth_m: 0.5 }.equivalent_radius_m();
        for &rough in &[0.2f32, 0.35, 0.6, 0.9] {
            // Points 0, 1 and 2.5 m from under the panel; the camera behind
            // (backscatter), above, or ahead (the panel near the mirror
            // direction: the visible streak).
            for &(px, cam) in &[(0.0f32, Vec3::new(-2.0, 1.6, 0.3)), (1.0, Vec3::new(-1.5, 1.6, -0.5)), (2.5, Vec3::new(-0.5, 1.6, 0.4)),
                (1.0, Vec3::new(2.5, 1.6, 0.2)), (2.5, Vec3::new(4.0, 1.6, -0.3)), (1.5, Vec3::new(4.5, 1.6, 0.0)), (0.5, Vec3::new(1.0, 1.6, 0.1))] {
                let p = Vec3::new(px, 0.0, 0.0);
                let v = (cam - p).normalize();
                // Brute force: area integral of brdf * cos(n) * cos(face) / d^2.
                let steps = 200;
                let mut brute = 0.0;
                for i in 0..steps {
                    for j in 0..steps {
                        let q = centre + Vec3::new((i as f32 + 0.5) / steps as f32 - 0.5, 0.0, (j as f32 + 0.5) / steps as f32 - 0.5) * 0.5;
                        let to = q - p;
                        let d2 = to.length_squared();
                        let l = to / d2.sqrt();
                        brute += ggx_cos(n, v, l, rough) * l.y / d2 * (0.25 / (steps * steps) as f32);
                    }
                }
                let ltc = ltc_polygon(n, v, p, rough, &corners);
                // The sphere light: radiance L = I / A over the sphere's disc
                // (pi r^2 = A), as sphere_light_specular evaluates it, with
                // E = L A / d^2 (the face's cosine at the representative point).
                let to = centre - p;
                let d = to.length();
                let r = (2.0 * n.dot(v) * n - v).normalize();
                let to_ray = to.dot(r) * r - to;
                let closest = to + to_ray * (radius / to_ray.length().max(1e-6)).clamp(0.0, 1.0);
                let a = (rough * rough).max(2e-3);
                let wide = (a + radius / (2.0 * d)).min(1.0);
                let sphere = ggx_cos(n, v, closest.normalize(), rough) * (a / wide).powi(2) * (to.normalize().y) * 0.25 / (d * d);
                let rel = (ltc - brute).abs() / brute.max(1e-6);
                eprintln!("rough {rough} at {px}, cam {cam}: brute {brute:.5} ltc {ltc:.5} ({:.0} %) sphere {sphere:.5} ({:.0} %)",
                    rel * 100.0, (sphere - brute).abs() / brute.max(1e-6) * 100.0);
                err_ltc += (ltc - brute).abs();
                err_sphere += (sphere - brute).abs();
                total += brute;
            }
        }
        eprintln!("energy-weighted error: ltc {:.1} %, sphere {:.1} %", err_ltc / total * 100.0, err_sphere / total * 100.0);
        assert!(err_ltc < err_sphere, "LTC must beat the sphere approximation");
        assert!(err_ltc / total < 0.1, "LTC energy-weighted error {}", err_ltc / total);
    }

    fn light() -> LocalLight {
        LocalLight { position: Vec3::new(0.0, 2.9, 0.0), direction: Vec3::NEG_Y, intensity_cd: Vec3::new(400.0, 300.0, 200.0), range_m: 10.0,
            inner_cos: 0.6, outer_cos: 0.2, source_radius_m: 0.0, falloff: 2.0 }
    }

    #[test]
    fn face_sends_the_light_intensity() {
        let face = LightFace::Rect { width_m: 0.5, depth_m: 0.5 };
        let m = face.material(&light());
        let radiance = Vec3::from(m.tint) * -m.scale;
        // Lambertian: on-axis intensity = radiance * area.
        assert!((radiance * face.area_m2() - light().intensity_cd).abs().max_element() < 1e-3);
        assert!(m.scale < 0.0 && m.flags & EMISSIVE != 0);
    }

    #[test]
    fn equivalent_sphere_has_the_face_area() {
        let face = LightFace::Rect { width_m: 0.5, depth_m: 0.5 };
        let r = face.equivalent_radius_m();
        assert!((std::f32::consts::PI * r * r - 0.25).abs() < 1e-5);
        assert_eq!(face.apply(light()).source_radius_m, r);
    }

    #[test]
    fn flat_face_sits_behind_the_light() {
        let face = LightFace::Rect { width_m: 0.5, depth_m: 0.5 };
        let t = face.transform(&light());
        // The emitting (-Y) side of the unit cube lands on the light.
        let bottom = t.transform_point3(Vec3::new(0.0, -0.5, 0.0));
        assert!((bottom - light().position).length() < 1e-5);
    }
}
