//! Shared scenery collision. Swept hull tests cannot tunnel through a building
//! at racing speed; the renderer supplies the same bounds it draws.
use crate::motion::Contact;
use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct Obstacle {
    pub min: Vec3,
    pub max: Vec3,
}
impl Obstacle {
    pub fn trace(&self, from: Vec3, to: Vec3) -> Option<(f32, Contact)> {
        let lo = self.min - Vec3::splat(2.5);
        let hi = self.max + Vec3::splat(2.5);
        let delta = to - from;
        let mut enter = 0.0_f32;
        let mut exit = 1.0_f32;
        let mut normal = Vec3::Y;
        for axis in 0..3 {
            if delta[axis].abs() < 1e-7 {
                if from[axis] < lo[axis] || from[axis] > hi[axis] {
                    return None;
                }
                continue;
            }
            let a = (lo[axis] - from[axis]) / delta[axis];
            let b = (hi[axis] - from[axis]) / delta[axis];
            if a.min(b) > enter {
                enter = a.min(b);
                normal = Vec3::ZERO;
                normal[axis] = -delta[axis].signum();
            }
            exit = exit.min(a.max(b));
            if exit < enter {
                return None;
            }
        }
        (enter <= 1. && exit >= 0.).then_some((
            enter,
            Contact {
                position: from + delta * enter,
                normal,
            },
        ))
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct Environment {
    pub ground: f32,
    pub hills: f32,
    pub spacing: f32,
    pub skip: f32,
    pub height: [f32; 2],
    pub sun: f32,
    pub sky: f32,
    pub exposure: f32,
}
impl Environment {
    pub fn height(&self, x: f32, z: f32) -> f32 {
        self.ground
            + self.hills
                * ((x * 0.0028).sin() * (z * 0.0023).cos() * 0.65 + ((x + z) * 0.006).sin() * 0.35)
    }
    pub fn trace(&self, obstacles: &[Obstacle], a: Vec3, b: Vec3) -> Option<Contact> {
        let mut hit = obstacles
            .iter()
            .filter_map(|o| o.trace(a, b))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        let before = a.y - self.height(a.x, a.z) - 2.;
        let after = b.y - self.height(b.x, b.z) - 2.;
        if after < 0. {
            let t = if before <= 0. {
                0.
            } else {
                (before / (before - after)).clamp(0., 1.)
            };
            if hit.as_ref().is_none_or(|h| t < h.0) {
                let p = a.lerp(b, t);
                let n = Vec3::new(
                    self.height(p.x - 1., p.z) - self.height(p.x + 1., p.z),
                    2.,
                    self.height(p.x, p.z - 1.) - self.height(p.x, p.z + 1.),
                )
                .normalize();
                hit = Some((
                    t,
                    Contact {
                        position: p,
                        normal: n,
                    },
                ));
            }
        }
        hit.map(|h| h.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swept_obstacles_stop_fast_flight() {
        let b = Obstacle {
            min: Vec3::new(-10., 0., -10.),
            max: Vec3::new(10., 100., 10.),
        };
        let (_, hit) = b
            .trace(Vec3::new(-80., 50., 0.), Vec3::new(80., 50., 0.))
            .unwrap();
        assert_eq!(hit.normal, -Vec3::X);
        assert_eq!(hit.position.x, -12.5);
        assert!(b
            .trace(Vec3::new(-80., 150., 0.), Vec3::new(80., 150., 0.))
            .is_none());
    }
}
