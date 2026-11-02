//! Source respawn camera: carried momentum, elevated quintic flight, seamless chase handoff.
use glam::{Mat3, Quat, Vec3};
use spaceracer::{chase::Chase, race::Race, race_rules::Pose};
pub fn progress(race: &Race, i: usize, alpha: f32) -> f32 {
    let p = &race.pilots[i];
    if p.crashed {
        return 1.;
    }
    if p.recovery <= 0. {
        return 1.;
    }
    (1. - (p.recovery as f32 + (1. - alpha) / 120.) / 0.65).clamp(0., 1.)
}
#[derive(Clone)]
struct Entry {
    serial: u64,
    crash: bool,
    position: Vec3,
    rotation: Quat,
    velocity: Vec3,
    fov: f64,
    target: Pose,
}
#[derive(Clone, Default)]
pub struct Camera {
    entry: Option<Entry>,
}
fn curve(start: Vec3, end: Vec3, velocity: Vec3, duration: f32, u: f32) -> (Vec3, f32) {
    let u = u.clamp(0., 1.);
    let blend = u * u * u * (u * (u * 6. - 15.) + 10.);
    let arc = 16. * u * u * (1. - u) * (1. - u);
    (
        start.lerp(end, blend)
            + velocity.clamp_length_max(120.) * duration * u * (1. - u).powi(3)
            + Vec3::Y * (start.distance(end) * 0.12).min(55.) * arc,
        blend,
    )
}
impl Camera {
    pub fn update(
        &mut self,
        race: &Race,
        i: usize,
        alpha: f32,
        chase: &mut Chase,
        aspect: f64,
        tuning: spaceracer::chase::Tuning,
    ) -> bool {
        let p = &race.pilots[i];
        let d = &race.drivers[i];
        if !p.crashed && p.recovery <= 0. {
            self.entry = None;
            return false;
        }
        let new = self
            .entry
            .as_ref()
            .is_none_or(|e| e.serial != d.motion.crash_id || (p.crashed && !e.crash));
        if new {
            self.entry = Some(Entry {
                serial: d.motion.crash_id,
                crash: p.crashed,
                position: chase.position,
                rotation: Quat::from_mat3(&chase.frame),
                velocity: chase.velocity,
                fov: chase.fov,
                target: if p.crashed {
                    race.respawn_pose(i)
                } else {
                    race.pose(i)
                },
            });
        }
        let e = self.entry.as_ref().unwrap();
        let (duration, elapsed) = if e.crash {
            (
                race.balance.respawn_seconds,
                if p.crashed {
                    (d.crash_time - (1. - alpha as f64) / 120.).max(0.)
                } else {
                    race.balance.respawn_seconds
                },
            )
        } else {
            (0.65, 0.65 - p.recovery - (1. - alpha as f64) / 120.)
        };
        let mut end = Chase::default();
        end.update_tuned(e.target, 90., false, 0., 0., true, false, aspect, tuning);
        let (pos, t) = curve(
            e.position,
            end.position,
            e.velocity,
            duration as f32,
            (elapsed / (duration - 0.12).max(0.01)) as f32,
        );
        // Initialize the normal chase at the target, including its internal rotation/distance.
        chase.update_tuned(e.target, 90., false, 0., 0., true, false, aspect, tuning);
        chase.position = pos;
        chase.frame = Mat3::from_quat(e.rotation.slerp(Quat::from_mat3(&end.frame), t));
        chase.fov = e.fov + (end.fov - e.fov) * t as f64;
        chase.velocity = Vec3::ZERO;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_carries_velocity_and_stops_at_destination() {
        let a = Vec3::new(12., 8., -40.);
        let b = Vec3::new(-5., 30., 100.);
        let v = Vec3::new(30., 0., 20.);
        assert_eq!(curve(a, b, v, 2., 0.).0, a);
        assert_eq!(curve(a, b, v, 2., 1.).0, b);
        let epsilon = 0.0001;
        assert!(((curve(a, b, v, 2., epsilon).0 - a) / (2. * epsilon) - v).length() < 0.15);
        assert!((b - curve(a, b, v, 2., 1. - epsilon).0).length() < 0.001);
    }
    #[test]
    fn recovery_reveal_finishes_and_pause_is_stable() {
        let mut r = Race::new(80385, "hard", "city", 1, 3);
        r.reset(0);
        assert!(progress(&r, 0, 1.) < 0.001);
        r.pilots[0].recovery = 0.325;
        assert!((progress(&r, 0, 1.) - 0.5).abs() < 0.001);
        let mut c = Camera::default();
        let mut chase = Chase::default();
        c.update(&r, 0, 1., &mut chase, 16. / 9.,Default::default());
        let at = chase.position;
        c.update(&r, 0, 1., &mut chase, 16. / 9.,Default::default());
        assert_eq!(chase.position, at);
        r.pilots[0].recovery = 0.;
        assert_eq!(progress(&r, 0, 1.), 1.);
    }
}
