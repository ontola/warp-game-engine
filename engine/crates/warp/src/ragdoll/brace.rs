//! A fall recruits a modest local flexion bias, then returns to the resting
//! joints. All rotations use Warp Human's bind frame (+Y up, +Z forward).
use glam::Vec3;
use physics::ragdoll::JointBrace;

pub(super) fn fall(part: usize) -> JointBrace {
    let (degrees, stiffness, damping) = match part {
        1 => ([16., 0., 0.], 3., 0.65),           // round the torso slightly
        2 => ([17., 0., 0.], 1., 0.15),           // tuck the chin
        3 => ([-10., -28., -55.], 2.1, 0.32),     // bring the arms forward
        5 => ([-10., 28., 55.], 2.1, 0.32),
        4 => ([0., -85., 0.], 1.25, 0.17),        // draw the forearms toward the body
        6 => ([0., 85., 0.], 1.25, 0.17),
        7 | 9 => ([-44., 0., 0.], 4.2, 0.58),     // hips and knees flex together
        8 | 10 => ([74., 0., 0.], 2.6, 0.32),
        _ => unreachable!("only articulated limb joints have a fall response"),
    };
    JointBrace {
        target: Vec3::from_array(degrees.map(f32::to_radians)),
        stiffness,
        damping,
        ramp: 0.19,
        hold: 0.28,
        release: 0.78,
    }
}

#[cfg(test)]
mod tests {
    use crate::{config::HumanPool, humans::Humans};
    use glam::{Quat, Vec3};
    use std::collections::BTreeMap;

    #[test]
    fn a_falling_human_bends_both_knees_and_draws_both_hands_in() {
        let pool = HumanPool { count: 1, seed: 45, height: [1.78; 2], ..Default::default() };
        let config = BTreeMap::from([("test".into(), pool)]);
        for yaw in [0., std::f32::consts::FRAC_PI_2] {
            let mut humans = Humans::new(&config);
            let actor = humans.spawn("test", Vec3::ZERO, yaw).unwrap().unwrap();
            for _ in 0..10 { humans.tick(1. / 60.); }
            let measure = |humans: &Humans, knee, foot, hand| {
                let joint = |name| humans.joint(actor, name).unwrap();
                let thigh = joint(knee) - joint("hips");
                let shin = joint(foot) - joint(knee);
                (thigh.angle_between(shin).to_degrees(), joint(hand).distance(joint("chest")))
            };
            let limbs = [("knee_l", "foot_l", "hand_l"), ("knee_r", "foot_r", "hand_r")];
            let start = limbs.map(|(k, f, h)| measure(&humans, k, f, h));
            let motion = Quat::from_rotation_y(yaw) * Vec3::new(1., 4.6, 0.);
            assert!(humans.ragdoll(actor, motion).unwrap());
            for _ in 0..18 { humans.tick(1. / 60.); }
            for ((k, f, h), (initial_knee, initial_reach)) in limbs.into_iter().zip(start) {
                let (knee, reach) = measure(&humans, k, f, h);
                assert!(knee > initial_knee + 18., "{k} flexion {initial_knee} -> {knee}");
                assert!(reach < initial_reach - 0.055, "{h} distance {initial_reach} -> {reach}");
            }
        }
    }
}
