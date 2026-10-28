//! Air-strike targeting laser from `scripts/air_strike.gd`: a white core
//! inside a red additive sheath, connecting the rocket's origin to its target.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::{Strike, StrikeKind, AIR_STRIKE_TRAVEL};
use render::scene::NO_SHADOW;
use render::Scene;

pub(super) const SKY_OFFSET: Vec3 = Vec3::new(0.0, 263.0, 93.0);
const MAX_STRIKES: usize = 32;
const WIDTHS: [f32; 2] = [0.035, 0.085];

pub(super) struct Lasers { first: usize, meshes: [usize; 2] }

pub(super) fn active(strike: &Strike) -> bool {
    strike.kind == StrikeKind::AirStrike && strike.target.is_finite()
        && strike.age.is_finite() && (0.0..AIR_STRIKE_TRAVEL).contains(&strike.age)
}

fn transform(target: Vec3, width: f32) -> Affine3A {
    Affine3A::from_scale_rotation_translation(
        Vec3::new(width, width, SKY_OFFSET.length()),
        Quat::from_rotation_arc(Vec3::Z, SKY_OFFSET.normalize()), target + SKY_OFFSET * 0.5)
}

impl Lasers {
    pub fn reserve(scene: &mut Scene) -> Self {
        let core = super::super::glow(scene, [1.0; 3], 30_000.0);
        let sheath = super::super::glow(scene, [1.0, 0.08, 0.04], 8_000.0);
        // Closed cubes need front faces only: drawing their backs too would
        // double the additive energy and risk overflowing the fp16 HDR target.
        scene.set_material_additive(core, 1.0, false);
        scene.set_material_additive(sheath, 0.9, false);
        let meshes = [core, sheath].map(|material| super::super::cube(scene, material));
        let first = scene.instances.len();
        for _ in 0..MAX_STRIKES {
            for mesh in meshes {
                // Transparency registers triangle owners once at startup.
                // Keep geometry in dormant slots; zero scale hides them.
                scene.add_instance(mesh, Affine3A::from_scale(Vec3::ZERO));
                scene.instances.last_mut().unwrap().flags |= NO_SHADOW;
            }
        }
        Self { first, meshes }
    }

    pub fn pose(&self, scene: &mut Scene, strikes: &[Strike]) {
        let mut count = 0;
        for strike in strikes.iter().filter(|strike| active(strike)).take(MAX_STRIKES) {
            for (layer, width) in WIDTHS.into_iter().enumerate() {
                scene.update_instance(self.first + count * 2 + layer, self.meshes[layer], transform(strike.target, width));
            }
            count += 1;
        }
        for slot in count * 2..MAX_STRIKES * 2 {
            if scene.instances[self.first + slot].scale > 0.0 {
                scene.update_instance(self.first + slot, self.meshes[slot % 2], Affine3A::from_scale(Vec3::ZERO));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strike(kind: StrikeKind, age: f64) -> Strike {
        Strike { kind, target: Vec3::new(4.0, 1.5, -8.0), owner: Some(0), age }
    }

    #[test]
    fn laser_spans_the_rocket_path_with_thin_nested_layers() {
        let target = strike(StrikeKind::AirStrike, 0.0).target;
        for width in WIDTHS {
            let xf = transform(target, width);
            assert!(xf.transform_point3(Vec3::NEG_Z * 0.5).abs_diff_eq(target, 0.0001));
            assert!(xf.transform_point3(Vec3::Z * 0.5).abs_diff_eq(target + SKY_OFFSET, 0.0001));
            assert!((xf.transform_vector3(Vec3::X).length() - width).abs() < 0.00001);
            assert!((xf.transform_vector3(Vec3::Y).length() - width).abs() < 0.00001);
        }
        assert!(WIDTHS[0] < WIDTHS[1]);
    }

    #[test]
    fn additive_laser_lasts_until_airstrike_impact_and_never_marks_ion() {
        let mut scene = Scene::default();
        let lasers = Lasers::reserve(&mut scene);
        assert!(scene.instances.iter().all(|i| i.cluster_count > 0 && i.scale == 0.0));
        assert!(scene.blended_materials.values().all(|m| m.additive && !m.double_sided));
        for age in [0.0, 0.5, AIR_STRIKE_TRAVEL - growing_guns::DT64] {
            lasers.pose(&mut scene, &[strike(StrikeKind::AirStrike, age), strike(StrikeKind::IonCannon, age)]);
            assert_eq!(scene.instances.iter().filter(|i| i.scale > 0.0).count(), 2);
            assert!(scene.instances.iter().all(|i| i.flags & NO_SHADOW != 0));
        }
        for age in [-1.0, AIR_STRIKE_TRAVEL, AIR_STRIKE_TRAVEL + 1.0, f64::NAN] {
            lasers.pose(&mut scene, &[strike(StrikeKind::AirStrike, age)]);
            assert!(scene.instances.iter().all(|i| i.scale == 0.0 && i.cluster_count > 0));
        }
        lasers.pose(&mut scene, &[strike(StrikeKind::AirStrike, 0.0)]);
        lasers.pose(&mut scene, &[]);
        assert!(scene.instances.iter().all(|i| i.scale == 0.0));
    }
}
