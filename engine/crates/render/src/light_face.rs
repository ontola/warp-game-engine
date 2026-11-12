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

#[cfg(test)]
mod tests {
    use super::*;

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
