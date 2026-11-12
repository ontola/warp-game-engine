//! Map the imported humanoid's weighted bones to a small articulated physics rig.
//! The original textured mesh stays on the GPU; only its skin palette changes.
use assets::skinned::Character;
use glam::{Affine3A, Mat4, Quat, Vec3};
use physics::{ragdoll::{Ragdoll, Segment, AngularProfile}, Physics};

const PARTS: usize = 11;

pub(super) struct Corpse {
    pub rig: Ragdoll,
    initial: Vec<Mat4>,
    parts: Vec<usize>,
    headless: bool,
    neck_world: Vec3,
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    use kern::collide::YawBox;

    #[test]
    fn every_roster_skin_keeps_its_initial_pose_then_articulates_and_settles() {
        for name in [3, 11, 52] {
            let model = super::super::characters::preview_model(name).unwrap();
            let original = model.sample("pistol_run", 0.23, true).unwrap();
            let mut palette = original.clone();
            let transform = Affine3A::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(2.0, 1.5, -3.0));
            let mut physics = Physics::new(Vec3::new(0.0, -20.0, 0.0), 60.0);
            physics.add_static(&YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(80.0, 1.0, 80.0), 0.0));
            let mut corpse = Corpse::spawn(&model, &palette, transform, Vec3::X, Vec3::Z, 0, false, &mut physics).unwrap();
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

pub(super) fn part(name: &str) -> usize {
    let name = canonical(name);
    match name {
        "Hips" => 0,
        "Neck" | "Head" | "LeftEye" | "RightEye" | "LidXp" | "LidXn" | "LidYp" | "LidYn" | "LidZp" | "LidZn" => 2,
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

// Model coordinates: +Y up, +Z forward. Ranges are deliberately approximate
// game tuning in degrees relative to the bind skeleton, not clinical ROM data.
fn profile(part: usize) -> AngularProfile {
    let (min, max, neutral) = match part {
        1 => ([-25., -25., -20.], [35., 25., 20.], [5., 0., 0.]),
        2 => ([-40., -65., -35.], [50., 65., 35.], [8., 0., 0.]),
        3 => ([-80., -80., -100.], [90., 80., 50.], [10., 0., -65.]),
        5 => ([-80., -80., -50.], [90., 80., 100.], [10., 0., 65.]),
        4 => ([-12., -145., -12.], [12., 5., 12.], [0., -35., 0.]),
        6 => ([-12., -5., -12.], [12., 145., 12.], [0., 35., 0.]),
        7 | 9 => ([-110., -35., -45.], [30., 35., 45.], [-15., 0., 0.]),
        8 | 10 => ([-5., -10., -10.], [145., 10., 10.], [25., 0., 0.]),
        _ => unreachable!(),
    };
    let radians = |v: [f32; 3]| Vec3::from_array(v.map(f32::to_radians));
    AngularProfile { min: radians(min), max: radians(max), neutral: radians(neutral),
        soft_zone: 4.0_f32.to_radians(), stiffness: 0.35, limit_stiffness: 18.0,
        damping: 0.25, max_torque: 12.0 }
}

impl Corpse {
    pub fn spawn(model: &Character, palette: &[Mat4], transform: Affine3A, velocity: Vec3,
        direction: Vec3, actor: usize, headless: bool, physics: &mut Physics) -> Result<Self, String> {
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
        let mut rig = Ragdoll::spawn(physics, &segments, velocity + push * 3.5 + up * 1.2,
            up.cross(push) * 1.5, 1u32 << actor)?;
        // Recover each part's animated anatomical axes from the skin delta.
        // Unlike a frame captured from the capsule at death, bind-relative axes
        // retain the same rest target when dying mid-run or while aiming.
        let names = ["Hips", "Spine2", "Head", "LeftArm", "LeftForeArm",
            "RightArm", "RightForeArm", "LeftUpLeg", "LeftLeg", "RightUpLeg", "RightLeg"];
        let model_rotation = transform.to_scale_rotation_translation().1;
        let mut frames = [Quat::IDENTITY; PARTS];
        for (i, name) in names.into_iter().enumerate() {
            let index = model.joints().position(|(n, _)| canonical(n) == name).expect("validated skeleton");
            let bone_rotation = (model_rotation * palette[index].to_scale_rotation_translation().1).normalize();
            let body_rotation = physics.pose(rig.body(i)).expect("new body").1;
            frames[i] = (body_rotation.inverse() * bone_rotation).normalize();
        }
        for i in 1..PARTS {
            if let Err(error) = rig.set_angular_profile(physics, i,
                frames[segments[i].parent.unwrap()], frames[i], profile(i)) {
                rig.remove(physics);
                return Err(error);
            }
        }
        Ok(Self { rig, initial: palette.iter().map(|p| model_world * *p).collect(),
            parts: model.joints().map(|(name, _)| part(name)).collect(), headless, neck_world: neck })
    }

    pub fn pose(&self, alpha: f32, palette: &mut [Mat4]) -> Affine3A {
        // Keep geometry near its instance origin for conservative culling bounds.
        let origin = self.rig.root_position(alpha);
        let local = Mat4::from_translation(-origin);
        let mut deltas: [Mat4; PARTS] = std::array::from_fn(|i| local * self.rig.delta(i, alpha));
        if self.headless {
            let stump = deltas[1].transform_point3(self.neck_world);
            deltas[2] = Mat4::from_translation(stump) * Mat4::from_scale(Vec3::splat(0.001)) * Mat4::from_translation(-stump) * deltas[1];
        }
        for ((dst, initial), &part) in palette.iter_mut().zip(&self.initial).zip(&self.parts) {
            *dst = deltas[part] * *initial;
        }
        Affine3A::from_translation(origin)
    }
}
