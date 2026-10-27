//! Static arena geometry: boxes rotated about Y (Godot's `_add_static_box`
//! with `rotation_y`). Collision for the player capsule and ray queries for
//! bullets, both exact for yaw-rotated boxes.
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Floor,
    Wall,
    Tower,
    Building,
    Ledge,
    Pillar,
    Cover,
    Platform,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub center: Vec3,
    pub half: Vec3,
    /// Rotation about +Y in radians (Godot convention: positive turns +X towards -Z).
    pub yaw: f32,
    pub kind: Kind,
}

impl Block {
    pub fn new(center: Vec3, size: Vec3, yaw: f32, kind: Kind) -> Self {
        Self { center, half: size * 0.5, yaw, kind }
    }

    /// World XZ offset into the box's local frame.
    fn to_local(&self, d: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        // Inverse of Godot's Y rotation: x' = c x - s z, z' = s x + c z.
        Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
    }

    fn to_world(&self, d: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        Vec2::new(c * d.x + s * d.y, -s * d.x + c * d.y)
    }

    pub fn top(&self) -> f32 { self.center.y + self.half.y }

    /// Push a vertical cylinder (centre, radius, half height) out of this
    /// box. Returns the world-space push, or None without overlap.
    pub fn push_out(&self, p: Vec3, radius: f32, half_height: f32) -> Option<Vec3> {
        let dy = p.y - self.center.y;
        let overlap_y = self.half.y + half_height - dy.abs();
        if overlap_y <= 0.0 { return None; }
        let local = self.to_local(Vec2::new(p.x - self.center.x, p.z - self.center.z));
        let h = Vec2::new(self.half.x, self.half.z);
        let closest = local.clamp(-h, h);
        let delta = local - closest;
        let dist2 = delta.length_squared();
        let horizontal = if dist2 > 1e-12 {
            // Centre outside the rectangle: circle against an edge or corner.
            let dist = dist2.sqrt();
            if dist >= radius { return None; }
            Some((delta / dist, radius - dist))
        } else {
            // Centre inside: leave through the nearest face.
            let dx = h.x - local.x.abs();
            let dz = h.y - local.y.abs();
            if dx < dz { Some((Vec2::new(local.x.signum(), 0.0), dx + radius)) }
            else { Some((Vec2::new(0.0, local.y.signum()), dz + radius)) }
        };
        let (n, depth) = horizontal?;
        if overlap_y < depth {
            Some(Vec3::new(0.0, overlap_y * dy.signum(), 0.0))
        } else {
            let w = self.to_world(n) * depth;
            Some(Vec3::new(w.x, 0.0, w.y))
        }
    }

    /// Ray against the box: distance along `dir` (unit) and the surface normal.
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
        if axis == usize::MAX { return None; } // origin inside the box
        let mut n = [0.0; 3];
        n[axis] = sign;
        let w = self.to_world(Vec2::new(n[0], n[2]));
        Some((t0, Vec3::new(w.x, n[1], w.y)))
    }
}

/// Closest hit among `blocks`.
pub fn raycast(blocks: &[Block], origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
    let mut best: Option<(f32, Vec3, usize)> = None;
    for (i, b) in blocks.iter().enumerate() {
        let limit = best.map_or(max, |h| h.0);
        if let Some((t, n)) = b.ray(origin, dir, limit) {
            if best.is_none_or(|h| t < h.0) { best = Some((t, n, i)); }
        }
    }
    best
}

/// Ray against a sphere: entry distance.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_rotated_box_on_the_right_face() {
        let b = Block::new(Vec3::new(10.0, 1.0, 0.0), Vec3::new(2.0, 2.0, 2.0), std::f32::consts::FRAC_PI_4, Kind::Cover);
        let (t, n) = b.ray(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 100.0).unwrap();
        // A 2 m cube turned 45 degrees reaches sqrt(2) towards the ray.
        assert!((t - (10.0 - 2f32.sqrt())).abs() < 1e-4, "{t}");
        assert!(n.x < -0.5 && n.y.abs() < 1e-6, "{n}");
        assert!(b.ray(Vec3::new(0.0, 3.5, 0.0), Vec3::X, 100.0).is_none());
        assert!(b.ray(Vec3::ZERO, Vec3::NEG_X, 100.0).is_none());
    }

    #[test]
    fn standing_capsule_is_pushed_up_and_walls_push_sideways() {
        let floor = Block::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(10.0, 1.0, 10.0), 0.0, Kind::Floor);
        let up = floor.push_out(Vec3::new(1.0, 0.85, 0.0), 0.4, 0.9).unwrap();
        assert!((up.y - 0.05).abs() < 1e-5 && up.x == 0.0);
        let wall = Block::new(Vec3::new(2.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 4.0), 0.0, Kind::Wall);
        let side = wall.push_out(Vec3::new(1.2, 1.0, 0.0), 0.4, 0.9).unwrap();
        assert!((side.x + 0.1).abs() < 1e-5 && side.y == 0.0, "{side}");
        assert!(wall.push_out(Vec3::new(0.5, 1.0, 0.0), 0.4, 0.9).is_none());
    }

    #[test]
    fn sphere_ray() {
        assert!((ray_sphere(Vec3::ZERO, Vec3::Z, Vec3::new(0.0, 0.0, 5.0), 1.0, 10.0).unwrap() - 4.0).abs() < 1e-5);
        assert!(ray_sphere(Vec3::ZERO, Vec3::Z, Vec3::new(2.0, 0.0, 5.0), 1.0, 10.0).is_none());
    }
}
