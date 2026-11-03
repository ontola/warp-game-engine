//! Shared scenery collision. Swept hull tests cannot tunnel through a building
//! at racing speed; the renderer supplies the same bounds it draws.
use crate::motion::Contact;
use glam::{Vec3,Affine3A};

#[derive(Clone, Copy, Debug)]
pub struct Obstacle {
    pub min: Vec3,
    pub max: Vec3,
    inverse: Affine3A,
    lo: Vec3,
    hi: Vec3,
}
impl Obstacle {
    pub fn bounds(min:Vec3,max:Vec3)->Self {Self::oriented(min,max,Affine3A::IDENTITY)}
    pub fn oriented(lo:Vec3,hi:Vec3,transform:Affine3A)->Self {
        let center=transform.transform_point3((lo+hi)*0.5);
        let half=(hi-lo)*0.5;
        let extent=Vec3::from(transform.matrix3.x_axis).abs()*half.x+Vec3::from(transform.matrix3.y_axis).abs()*half.y+Vec3::from(transform.matrix3.z_axis).abs()*half.z;
        Self {min:center-extent,max:center+extent,inverse:transform.inverse(),lo,hi}
    }
    pub fn trace(&self, from: Vec3, to: Vec3) -> Option<(f32, Contact)> {
        self.trace_radius(from,to,0.)
    }
    pub fn trace_radius(&self, world_from:Vec3,world_to:Vec3,radius:f32)->Option<(f32,Contact)> {
        if world_from.min(world_to).cmpgt(self.max+Vec3::splat(radius)).any() || world_from.max(world_to).cmplt(self.min-Vec3::splat(radius)).any() {return None;}
        let from=self.inverse.transform_point3(world_from);let to=self.inverse.transform_point3(world_to);
        let m=self.inverse.matrix3.transpose();
        let padding=Vec3::new(m.x_axis.length(),m.y_axis.length(),m.z_axis.length())*radius;
        let lo=self.lo-padding;let hi=self.hi+padding;
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
                position: world_from.lerp(world_to,enter),
                normal: (self.inverse.matrix3.transpose()*normal).normalize_or_zero(),
            },
        ))
    }
}

/// Triangle-consistent scenery height field shared by rendering and crashes.
#[derive(Clone, Debug)]
pub struct Landscape {
    pub heights: Vec<f32>,
    pub cells: usize,
    pub step: f32,
    pub half: f32,
    pub outside: f32,
}
impl Landscape {
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let gx = (x + self.half) / self.step;
        let gz = (z + self.half) / self.step;
        if gx < 0. || gz < 0. || gx >= self.cells as f32 || gz >= self.cells as f32 {
            return self.outside;
        }
        let ix = gx.floor() as usize;
        let iz = gz.floor() as usize;
        let u = gx - ix as f32;
        let v = gz - iz as f32;
        let id = iz * (self.cells + 1) + ix;
        let a = self.heights[id];
        let b = self.heights[id + 1];
        let c = self.heights[id + self.cells + 1];
        let d = self.heights[id + self.cells + 2];
        if u + v <= 1. {
            a + (b - a) * u + (c - a) * v
        } else {
            d + (c - d) * (1. - u) + (b - d) * (1. - v)
        }
    }
    pub fn normal(&self, x: f32, z: f32) -> Vec3 {
        Vec3::new(
            self.height(x - 2., z) - self.height(x + 2., z),
            4.,
            self.height(x, z - 2.) - self.height(x, z + 2.),
        )
        .normalize()
    }
}
#[derive(Clone, serde::Deserialize)]
pub struct Environment {
    #[serde(skip)]
    pub landscape: Option<std::sync::Arc<Landscape>>,
    pub ground: f32,
    pub hills: f32,
    pub spacing: f32,
    pub skip: f32,
    pub height: [f32; 2],
    #[serde(default)]
    pub ambient: f32,
    pub sun: f32,
    pub sky: f32,
    pub exposure: f32,
}
impl Environment {
    pub fn height(&self, x: f32, z: f32) -> f32 {
        if let Some(t) = &self.landscape {
            return t.height(x, z);
        }
        self.ground
            + self.hills
                * ((x * 0.0028).sin() * (z * 0.0023).cos() * 0.65 + ((x + z) * 0.006).sin() * 0.35)
    }
    pub fn trace(&self, obstacles: &[Obstacle], a: Vec3, b: Vec3) -> Option<Contact> {
        let mut hit = obstacles
            .iter()
            .filter_map(|o| o.trace_radius(a, b, 1.3))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        if let Some(field) = &self.landscape {
            // Sample the whole sweep: both endpoints can be above a narrow ridge.
            let steps = (a.distance(b) / (field.step * 0.25)).ceil().max(1.) as usize;
            let mut previous = 0.;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let p = a.lerp(b, t);
                if p.y - 2. <= field.height(p.x, p.z) {
                    let mut lo = previous;
                    let mut hi = t;
                    for _ in 0..8 {
                        let mid = (lo + hi) * 0.5;
                        let q = a.lerp(b, mid);
                        if q.y - 2. > field.height(q.x, q.z) {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    if hit.as_ref().is_none_or(|h| hi < h.0) {
                        let mut p = a.lerp(b, hi);
                        p.y = field.height(p.x, p.z) + 2.;
                        hit = Some((
                            hi,
                            Contact {
                                position: p,
                                normal: field.normal(p.x, p.z),
                            },
                        ));
                    }
                    break;
                }
                previous = t;
            }
            return hit.map(|h| h.1);
        }
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
    fn rotated_thin_wall_does_not_fill_its_world_bounds() {
        let wall=Obstacle::oriented(Vec3::new(-20.,-5.,-0.5),Vec3::new(20.,5.,0.5),
            Affine3A::from_rotation_translation(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),Vec3::ZERO));
        assert!(wall.trace(Vec3::new(12.,-10.,12.),Vec3::new(12.,10.,12.)).is_none());
        assert!(wall.trace(Vec3::new(0.,-10.,0.),Vec3::new(0.,10.,0.)).is_some());
    }
    #[test]
    fn landscape_matches_triangle_diagonal() {
        let f = Landscape {
            heights: vec![0., 4., 2., 20.],
            cells: 1,
            step: 1.,
            half: 0.,
            outside: -10.,
        };
        assert_eq!(f.height(0.25, 0.25), 1.5);
        assert_eq!(f.height(0.75, 0.75), 11.5);
        assert_eq!(f.height(-1., 0.), -10.);
    }
    #[test]
    fn ground_sweep_catches_ridge_with_clear_endpoints() {
        let mut e = crate::rules::Rules::new(None)
            .unwrap()
            .environment("forest")
            .unwrap();
        e.landscape = Some(std::sync::Arc::new(Landscape {
            heights: vec![0., 20., 0., 0., 20., 0., 0., 20., 0.],
            cells: 2,
            step: 10.,
            half: 0.,
            outside: 0.,
        }));
        let hit = e
            .trace(&[], Vec3::new(0., 12., 5.), Vec3::new(19.9, 12., 5.))
            .unwrap();
        assert!((hit.position.x - 5.).abs() < 0.05);
    }
    #[test]
    fn swept_obstacles_stop_fast_flight() {
        let b = Obstacle::bounds(Vec3::new(-10., 0., -10.),Vec3::new(10.,100.,10.));
        let (_, hit) = b
            .trace(Vec3::new(-80., 50., 0.), Vec3::new(80., 50., 0.))
            .unwrap();
        assert_eq!(hit.normal, -Vec3::X);
        assert_eq!(hit.position.x, -10.);
        assert!(
            b.trace(Vec3::new(-80., 150., 0.), Vec3::new(80., 150., 0.))
                .is_none()
        );
    }
}
