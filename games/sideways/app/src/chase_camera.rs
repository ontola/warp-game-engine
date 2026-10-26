//! Small stability rules for the chase camera, separate from vehicle physics.
use glam::{Quat, Vec3};

#[derive(Clone, Copy, Default)]
pub struct RestAnchor { pose: Option<(Vec3, Vec3)> }

impl RestAnchor {
    pub fn sample(&mut self, position: Vec3, rotation: Quat, velocity: Vec3, angular_velocity: Vec3) -> (Vec3, Vec3) {
        let forward = rotation * Vec3::NEG_Z;
        let heading = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
        if let Some((held_position, held_heading)) = self.pose {
            let yaw_change = held_heading.cross(heading).y.atan2(held_heading.dot(heading));
            // Suspension/tyre relaxation can move a parked rigid body by a
            // few millimetres. Do not turn that into whole-screen movement.
            // This is a bounded rest deadband, not lag while the car drives.
            if velocity.length() < 0.03 && angular_velocity.y.abs() < 0.02
                && position.distance(held_position) <= 0.015 && yaw_change.abs() <= 0.000_524 {
                return (held_position, held_heading);
            }
        }
        self.pose = Some((position, heading));
        (position, heading)
    }
}

/// Both local pads and GameNight pass raw [-1,1] axes through this one radial
/// gate. Hysteresis prevents threshold noise from alternating orbit/recenter.
pub fn orbit(yaw: f32, look: [f32; 2], dt: f32, active: &mut bool) -> f32 {
    let look = look.map(|v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 });
    let length = look[0].hypot(look[1]);
    *active = if *active { length > 0.16 } else { length > 0.24 };
    let target = if *active { -look[0].atan2(look[1]) } else { 0.0 };
    let yaw = if yaw.is_finite() { yaw } else { 0.0 };
    let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
    let delta = (target - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let next = yaw + delta * (1.0 - (-dt * 12.0).exp());
    let next = (next + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    if !*active && next.abs() < 0.000_1 { 0.0 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_parked_pose_noise_does_not_move_the_whole_view() {
        // Actual CPU idle probe: 9.14 mm x-range, 0.00278 degrees yaw,
        // speed <= 0.009 m/s over five seconds on the seed-1 starting road.
        let mut camera = RestAnchor::default();
        let original = camera.sample(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, Vec3::ZERO);
        for tick in 0..600 {
            let phase = tick as f32 * 0.3;
            let position = Vec3::new(0.00457 * phase.sin(), 0.00009 * phase.cos(), 0.00051 * phase.sin());
            let rotation = Quat::from_rotation_y(0.0000243 * phase.sin());
            assert_eq!(camera.sample(position, rotation, Vec3::X * 0.009, Vec3::Y * 0.001), original);
        }
    }

    #[test]
    fn actual_driving_turning_and_teleport_follow_without_added_lag() {
        let mut camera = RestAnchor::default();
        camera.sample(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, Vec3::ZERO);
        let moving = Vec3::new(0.001, 0.0, 0.0);
        assert_eq!(camera.sample(moving, Quat::IDENTITY, Vec3::X, Vec3::ZERO).0, moving);
        let turn = Quat::from_rotation_y(0.1);
        assert_eq!(camera.sample(moving, turn, Vec3::ZERO, Vec3::Y).1, turn * Vec3::NEG_Z);
        let teleported = Vec3::new(80.0, 4.0, 10.0);
        assert_eq!(camera.sample(teleported, turn, Vec3::ZERO, Vec3::ZERO).0, teleported);
        let slow_creep = teleported + Vec3::X * 0.02;
        assert_eq!(camera.sample(slow_creep, turn, Vec3::ZERO, Vec3::ZERO).0, slow_creep,
            "the rest deadband must never accumulate unlimited physical drift");
    }

    #[test]
    fn stick_threshold_chatter_cannot_alternate_orbit_and_recenter() {
        let mut active = false; let mut yaw = 0.0;
        for i in 0..120 { yaw = orbit(yaw, [0.2 + if i%2==0 {0.01}else{-0.01}, 0.0], 1.0/120.0, &mut active); }
        assert_eq!(yaw, 0.0); assert!(!active);
        yaw = orbit(yaw, [1.0, 0.0], 1.0/120.0, &mut active); assert!(active);
        for i in 0..120 { yaw = orbit(yaw, [0.2 + if i%2==0 {0.01}else{-0.01}, 0.0], 1.0/120.0, &mut active); assert!(active); }
        assert!((yaw + std::f32::consts::FRAC_PI_2).abs() < 0.001);
        for _ in 0..180 { yaw = orbit(yaw, [0.0, 0.0], 1.0/120.0, &mut active); }
        assert!(!active); assert_eq!(yaw, 0.0);
    }

    #[test]
    fn cardinal_orbit_directions_and_pi_wrap_stay_correct() {
        for (look, target) in [([0.0,1.0],0.0),([1.0,0.0],-std::f32::consts::FRAC_PI_2),
            ([-1.0,0.0],std::f32::consts::FRAC_PI_2),([0.0,-1.0],-std::f32::consts::PI)] {
            let mut active = false; let mut yaw = 0.0;
            for _ in 0..180 { yaw = orbit(yaw, look, 1.0/120.0, &mut active); }
            assert!((yaw-target).abs()<0.0001);
        }
        let mut active=true; let yaw=orbit(std::f32::consts::PI-0.001, [0.001,-1.0], 1.0/120.0,&mut active);
        let difference = (yaw-std::f32::consts::PI+std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)-std::f32::consts::PI;
        assert!(difference.abs()<0.002,"crossing +/-pi must not swing around the other way");
        for turns in [-3.0, -1.0, 1.0, 3.0] {
            let mut yaw = turns * std::f32::consts::TAU;
            for _ in 0..180 { yaw = orbit(yaw, [0.0,0.0], 1.0/120.0, &mut active); }
            assert_eq!(yaw, 0.0, "returning from an orbit must restore exact neutral state");
        }
    }
}
