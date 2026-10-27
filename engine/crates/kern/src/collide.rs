//! Simple, deterministic collision for game prototypes: boxes rotated about
//! the vertical axis (most level geometry), an upright cylinder for
//! characters, rays and spheres, and a `move_and_slide` character mover.
//! No broadphase: fine for hundreds of boxes and a few movers per tick.
//!
//! Yaw convention: positive yaw turns +X towards -Z (right-handed, Y up;
//! the same as `glam::Quat::from_rotation_y`).
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// A box rotated about +Y.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct YawBox {
    pub center: Vec3,
    pub half: Vec3,
    pub yaw: f32,
}

impl YawBox {
    pub fn new(center: Vec3, size: Vec3, yaw: f32) -> Self { Self { center, half: size * 0.5, yaw } }

    fn to_local(&self, d: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
    }

    fn to_world(&self, d: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        Vec2::new(c * d.x + s * d.y, -s * d.x + c * d.y)
    }

    pub fn top(&self) -> f32 { self.center.y + self.half.y }

    /// Radius of the box's footprint circle, for cheap rejection.
    pub fn reach(&self) -> f32 { Vec2::new(self.half.x, self.half.z).length() }

    /// Push an upright cylinder (centre, radius, half height) out of this
    /// box along the shallower direction. None without overlap.
    pub fn push_out(&self, p: Vec3, radius: f32, half_height: f32) -> Option<Vec3> {
        let dy = p.y - self.center.y;
        let overlap_y = self.half.y + half_height - dy.abs();
        if overlap_y <= 0.0 { return None; }
        let local = self.to_local(Vec2::new(p.x - self.center.x, p.z - self.center.z));
        let h = Vec2::new(self.half.x, self.half.z);
        let delta = local - local.clamp(-h, h);
        let dist2 = delta.length_squared();
        let (n, depth) = if dist2 > 1e-12 {
            let dist = dist2.sqrt();
            if dist >= radius { return None; }
            (delta / dist, radius - dist)
        } else {
            let dx = h.x - local.x.abs();
            let dz = h.y - local.y.abs();
            if dx < dz { (Vec2::new(local.x.signum(), 0.0), dx + radius) } else { (Vec2::new(0.0, local.y.signum()), dz + radius) }
        };
        if overlap_y < depth {
            Some(Vec3::new(0.0, overlap_y * dy.signum(), 0.0))
        } else {
            let w = self.to_world(n) * depth;
            Some(Vec3::new(w.x, 0.0, w.y))
        }
    }

    /// Ray (unit `dir`) against the box: distance and outward surface normal.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let o2 = self.to_local(Vec2::new(origin.x - self.center.x, origin.z - self.center.z));
        let d2 = self.to_local(Vec2::new(dir.x, dir.z));
        let o = [o2.x, origin.y - self.center.y, o2.y];
        let d = [d2.x, dir.y, d2.y];
        let h = [self.half.x, self.half.y, self.half.z];
        let (mut t0, mut t1, mut axis, mut sign) = (0.0f32, max, usize::MAX, 0.0f32);
        for k in 0..3 {
            if d[k].abs() < 1e-12 {
                if o[k].abs() > h[k] { return None; }
                continue;
            }
            let inv = 1.0 / d[k];
            let (mut a, mut b) = ((-h[k] - o[k]) * inv, (h[k] - o[k]) * inv);
            let mut s = -1.0;
            if a > b { std::mem::swap(&mut a, &mut b); s = 1.0; }
            if a > t0 { t0 = a; axis = k; sign = s; }
            t1 = t1.min(b);
            if t0 > t1 { return None; }
        }
        if axis == usize::MAX { return None; }
        let mut n = [0.0; 3];
        n[axis] = sign;
        let w = self.to_world(Vec2::new(n[0], n[2]));
        Some((t0, Vec3::new(w.x, n[1], w.y)))
    }
}

/// Closest hit among boxes: distance, normal and index.
pub fn raycast<'a>(boxes: impl IntoIterator<Item = &'a YawBox>, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
    let mut best: Option<(f32, Vec3, usize)> = None;
    for (i, b) in boxes.into_iter().enumerate() {
        if let Some((t, n)) = b.ray(origin, dir, best.map_or(max, |h| h.0)) {
            if best.is_none_or(|h| t < h.0) { best = Some((t, n, i)); }
        }
    }
    best
}

/// Ray against a sphere: entry distance (0 when starting inside).
pub fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32, max: f32) -> Option<f32> {
    let m = origin - center;
    let b = m.dot(dir);
    let c = m.length_squared() - radius * radius;
    if c > 0.0 && b > 0.0 { return None; }
    let disc = b * b - c;
    if disc < 0.0 { return None; }
    let t = (-b - disc.sqrt()).max(0.0);
    (t <= max).then_some(t)
}

/// An upright cylinder character.
#[derive(Clone, Copy, Debug)]
pub struct Mover { pub radius: f32, pub half_height: f32 }

/// What a move touched.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Contact {
    pub on_floor: bool,
    pub on_ceiling: bool,
    /// Outward normal of the last wall touched while not on the floor.
    pub wall_normal: Option<Vec3>,
}

impl Mover {
    /// Move by `vel * dt`, sliding along boxes (Godot's move_and_slide for
    /// boxes): sub-steps of at most `max_step` metres, up to four push-out
    /// passes each. Velocity loses its component into walls, floors and
    /// ceilings. Small ledges (shallower than the side push) are stepped up.
    pub fn slide(&self, boxes: &[YawBox], pos: &mut Vec3, vel: &mut Vec3, dt: f32, max_step: f32) -> Contact {
        let travel = *vel * dt;
        let steps = (travel.length() / max_step).ceil().max(1.0) as usize;
        let mut c = Contact::default();
        for _ in 0..steps {
            *pos += travel / steps as f32;
            for _ in 0..4 {
                let mut any = false;
                for b in boxes {
                    let reach = b.half.x.max(b.half.z) * std::f32::consts::SQRT_2 + self.radius;
                    if (pos.x - b.center.x).abs() > reach || (pos.z - b.center.z).abs() > reach { continue; }
                    let Some(push) = b.push_out(*pos, self.radius, self.half_height) else { continue };
                    any = true;
                    *pos += push;
                    if push.y > 0.0 {
                        c.on_floor = true;
                        vel.y = vel.y.max(0.0);
                    } else if push.y < 0.0 {
                        c.on_ceiling = true;
                        vel.y = vel.y.min(0.0);
                    } else {
                        let n = push.normalize();
                        let into = vel.dot(n);
                        if into < 0.0 { *vel -= n * into; }
                        c.wall_normal = Some(n);
                    }
                }
                if !any { break; }
            }
        }
        if c.on_floor { c.wall_normal = None; }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_rotated_box_on_the_right_face() {
        let b = YawBox::new(Vec3::new(10.0, 1.0, 0.0), Vec3::splat(2.0), std::f32::consts::FRAC_PI_4);
        let (t, n) = b.ray(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 100.0).unwrap();
        assert!((t - (10.0 - 2f32.sqrt())).abs() < 1e-4, "{t}");
        assert!(n.x < -0.5 && n.y.abs() < 1e-6, "{n}");
        assert!(b.ray(Vec3::new(0.0, 3.5, 0.0), Vec3::X, 100.0).is_none());
        assert!(b.ray(Vec3::ZERO, Vec3::NEG_X, 100.0).is_none());
    }

    #[test]
    fn push_out_floor_and_wall() {
        let floor = YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(10.0, 1.0, 10.0), 0.0);
        let up = floor.push_out(Vec3::new(1.0, 0.85, 0.0), 0.4, 0.9).unwrap();
        assert!((up.y - 0.05).abs() < 1e-5 && up.x == 0.0);
        let wall = YawBox::new(Vec3::new(2.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 4.0), 0.0);
        let side = wall.push_out(Vec3::new(1.2, 1.0, 0.0), 0.4, 0.9).unwrap();
        assert!((side.x + 0.1).abs() < 1e-5 && side.y == 0.0, "{side}");
        assert!(wall.push_out(Vec3::new(0.5, 1.0, 0.0), 0.4, 0.9).is_none());
    }

    #[test]
    fn mover_lands_slides_along_walls_and_stops() {
        let world = [YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 1.0, 40.0), 0.0), YawBox::new(Vec3::new(3.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 10.0), 0.0)];
        let m = Mover { radius: 0.4, half_height: 0.9 };
        let (mut p, mut v) = (Vec3::new(0.0, 3.0, 0.0), Vec3::new(5.0, -10.0, 3.0));
        let mut touched_wall = false;
        for _ in 0..120 {
            v.y -= 30.0 / 60.0;
            let c = m.slide(&world, &mut p, &mut v, 1.0 / 60.0, 0.25);
            touched_wall |= c.wall_normal.is_some() || (p.x - 2.1).abs() < 0.02;
        }
        assert!((p.y - 0.9).abs() < 0.02, "rests on the floor: {p}");
        assert!(p.x <= 2.5 - 0.4 + 1e-3, "kept out of the wall: {p}");
        assert!(p.z > 3.0, "slid along it: {p}");
        assert!(touched_wall);
        assert!(ray_sphere(Vec3::ZERO, Vec3::Z, Vec3::new(0.0, 0.0, 5.0), 1.0, 10.0).is_some());
        assert_eq!(raycast(&world, Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0).map(|h| h.2), Some(1));
    }
}
