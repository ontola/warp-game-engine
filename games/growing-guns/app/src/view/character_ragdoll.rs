//! Map the imported humanoid's weighted bones to a small articulated physics rig.
//! The original textured mesh stays on the GPU; only its skin palette changes.
use assets::skinned::Character;
use glam::{Affine3A, Mat4, Vec3};
use physics::{ragdoll::{Ragdoll, Segment}, Physics};

const PARTS: usize = 11;

pub(super) struct Corpse {
    pub rig: Ragdoll,
    initial: Vec<Mat4>,
    parts: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    use kern::collide::YawBox;

    #[test]
    fn every_roster_skin_keeps_its_initial_pose_then_articulates_and_settles() {
        for name in ["knight", "paladin", "ch10"] {
            let model = Character::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../assets/characters/{name}.json"))).unwrap();
            let original = model.sample("pistol_run", 0.23, true).unwrap();
            let mut palette = original.clone();
            let transform = Affine3A::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(2.0, 1.5, -3.0));
            let mut physics = Physics::new(Vec3::new(0.0, -20.0, 0.0), 60.0);
            physics.add_static(&YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(80.0, 1.0, 80.0), 0.0));
            let mut corpse = Corpse::spawn(&model, &palette, transform, Vec3::X, Vec3::Z, 0, &mut physics).unwrap();
            let xf = Mat4::from(corpse.pose(1.0, &mut palette));
            for (&p, &initial) in palette.iter().zip(&original) {
                assert!((xf * p).abs_diff_eq(Mat4::from(transform) * initial, 0.0001), "{name}: death transition moved a bone");
            }
            for _ in 0..60 { physics.step(); corpse.rig.update(&physics); }
            assert!(!corpse.rig.delta(0, 1.0).abs_diff_eq(corpse.rig.delta(4, 1.0), 0.01), "{name}: arm remains a rigid frozen pose");
            for _ in 0..540 { physics.step(); corpse.rig.update(&physics); }
            let xf = Mat4::from(corpse.pose(1.0, &mut palette));
            for ((_, bind), skin) in model.joints().zip(&palette) {
                assert!(skin.is_finite(), "{name}: non-finite skin");
                let local = (*skin * bind).transform_point3(Vec3::ZERO);
                assert!(local.abs().max_element() < 4.0, "{name}: limb outside reserved bounds: {local}");
                let world = xf.transform_point3(local);
                assert!(world.y > -0.35 && world.y < 2.0, "{name}: joint did not settle on floor: {world}");
            }
            corpse.rig.remove(&mut physics);
            assert_eq!(physics.body_count(), 0);
        }
    }
}

fn canonical(name: &str) -> &str { name.rsplit(['_', ':']).next().unwrap_or(name) }

fn part(name: &str) -> usize {
    let name = canonical(name);
    match name {
        "Hips" => 0,
        "Neck" | "Head" | "LeftEye" | "RightEye" => 2,
        "LeftArm" => 3,
        "LeftForeArm" => 4,
        "RightArm" => 5,
        "RightForeArm" => 6,
        "LeftUpLeg" => 7,
        "LeftLeg" | "LeftFoot" | "LeftToeBase" => 8,
        "RightUpLeg" => 9,
        "RightLeg" | "RightFoot" | "RightToeBase" => 10,
        n if n.starts_with("LeftHand") => 4,
        n if n.starts_with("RightHand") => 6,
        _ => 1, // spine and clavicles follow the chest
    }
}

impl Corpse {
    pub fn spawn(model: &Character, palette: &[Mat4], transform: Affine3A, velocity: Vec3,
        direction: Vec3, actor: usize, physics: &mut Physics) -> Result<Self, String> {
        let model_world = Mat4::from(transform);
        let joint = |name: &str| -> Result<Vec3, String> {
            model.joints().enumerate().find(|(_, (n, _))| canonical(n) == name)
                .map(|(i, (_, bind))| (model_world * palette[i] * bind).transform_point3(Vec3::ZERO))
                .ok_or_else(|| format!("{} ragdoll missing {name}", model.name))
        };
        let hips = joint("Hips")?;
        let chest = joint("Spine2")?;
        let neck = joint("Neck")?;
        let head = joint("Head")?;
        let up = (neck - hips).normalize_or(Vec3::Y);
        let segment = |start, end, radius, parent, anchor| Segment { start, end, radius, parent, anchor };
        let mut segments = vec![
            segment(hips - up * 0.09, hips + up * 0.09, 0.14, None, hips),
            segment(hips, chest + up * 0.04, 0.16, Some(0), hips),
            segment(neck, head + up * 0.16, 0.105, Some(1), neck),
        ];
        for (side, upper, lower) in [("Left", 3, 4), ("Right", 5, 6)] {
            let a = joint(&format!("{side}Arm"))?;
            let b = joint(&format!("{side}ForeArm"))?;
            let c = joint(&format!("{side}Hand"))?;
            debug_assert_eq!(segments.len(), upper);
            segments.push(segment(a, b, 0.065, Some(1), a));
            segments.push(segment(b, c, 0.055, Some(upper), b));
            debug_assert_eq!(segments.len(), lower + 1);
        }
        for (side, upper) in [("Left", 7), ("Right", 9)] {
            let a = joint(&format!("{side}UpLeg"))?;
            let b = joint(&format!("{side}Leg"))?;
            let c = joint(&format!("{side}Foot"))?;
            segments.push(segment(a, b, 0.09, Some(0), a));
            segments.push(segment(b, c, 0.075, Some(upper), b));
        }
        debug_assert_eq!(segments.len(), PARTS);
        let push = direction.with_y(0.0).normalize_or(Vec3::Z);
        let rig = Ragdoll::spawn(physics, &segments, velocity + push * 3.5 + up * 1.2,
            up.cross(push) * 1.5, 1u32 << actor)?;
        Ok(Self { rig, initial: palette.iter().map(|p| model_world * *p).collect(),
            parts: model.joints().map(|(name, _)| part(name)).collect() })
    }

    pub fn pose(&self, alpha: f32, palette: &mut [Mat4]) -> Affine3A {
        // Keep geometry near its instance origin for conservative culling bounds.
        let origin = self.rig.root_position(alpha);
        let local = Mat4::from_translation(-origin);
        let deltas: [Mat4; PARTS] = std::array::from_fn(|i| local * self.rig.delta(i, alpha));
        for ((dst, initial), &part) in palette.iter_mut().zip(&self.initial).zip(&self.parts) {
            *dst = deltas[part] * *initial;
        }
        Affine3A::from_translation(origin)
    }
}
