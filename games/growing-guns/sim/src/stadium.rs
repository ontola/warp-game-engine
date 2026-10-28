//! The colosseum around the arena, after Godot's `colosseum_builder.gd`: a
//! rounded-square bowl of stepped stands (56 segments, 18 rows) rising
//! above the castle, with a facade, a towering back wall, columns and
//! crenellations. The simulation needs its solid parts (shots land in the
//! stands); the app builds the look, roof, lights and crowd from the same
//! numbers.
use crate::shape::YawBox;
use glam::Vec3;
use std::f32::consts::{PI, TAU};

pub const SEGMENTS: usize = 56;
pub const ROWS: usize = 18;
pub const ROW_DEPTH: f32 = 2.2;
pub const ROW_RISE: f32 = 1.4;
pub const BACK_WALL_EXTRA: f32 = 6.0;
pub const BASE_ABOVE_WALLS: f32 = 2.5;
pub const BASE_NO_WALLS: f32 = 4.5;

/// Superellipse (n = 4) plan: radius factor at a ring angle, 1 on the side
/// midpoints and about 1.19 in the corners.
pub fn sq(ang: f32) -> f32 {
    let (c, s) = (ang.cos().abs(), ang.sin().abs());
    (c.powi(4) + s.powi(4)).powf(-0.25)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stadium {
    pub inner_r: f32,
    /// Height of the first row.
    pub base_y: f32,
}

/// A box of the bowl: centre, full size, yaw (Godot `_box_xform`).
#[derive(Clone, Copy, Debug)]
pub struct Piece { pub center: Vec3, pub size: Vec3, pub yaw: f32, pub kind: PieceKind }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieceKind { Facade, Column, Capital, Step, Wall, Crenel }

impl Stadium {
    pub fn new(arena_size: f32, walls: bool, wall_height: f32) -> Self {
        Stadium { inner_r: arena_size * 0.82, base_y: if walls { wall_height + BASE_ABOVE_WALLS } else { BASE_NO_WALLS } }
    }

    pub fn wall_r(&self) -> f32 { self.inner_r + ROWS as f32 * ROW_DEPTH + 1.4 }
    pub fn wall_h(&self) -> f32 { self.base_y + ROWS as f32 * ROW_RISE + BACK_WALL_EXTRA }
    pub fn seg_ang() -> f32 { TAU / SEGMENTS as f32 }

    /// Every box of the stands, as Godot builds them.
    pub fn pieces(&self) -> Vec<Piece> {
        let seg = Self::seg_ang();
        let (inner, base) = (self.inner_r, self.base_y);
        let mut out = Vec::new();
        for i in 0..SEGMENTS {
            let ang = seg * (i as f32 + 0.5);
            let s = sq(ang);
            let dir = Vec3::new(ang.cos(), 0.0, ang.sin());
            let yaw = -ang + PI * 0.5;
            let facade_r = (inner - 0.8) * s;
            out.push(Piece { center: dir * facade_r + Vec3::Y * base * 0.5, size: Vec3::new(2.0 * facade_r * (seg * 0.5).tan() + 0.8, base, 1.6), yaw, kind: PieceKind::Facade });
            if i % 2 == 0 {
                let col_r = (inner - 2.0) * s;
                out.push(Piece { center: dir * col_r + Vec3::Y * base * 0.55, size: Vec3::new(1.5, base * 1.1, 1.5), yaw, kind: PieceKind::Column });
                out.push(Piece { center: dir * col_r + Vec3::Y * (base * 1.1 + 0.6), size: Vec3::new(2.3, 1.2, 2.3), yaw, kind: PieceKind::Capital });
            }
            for row in 0..ROWS {
                let r_mid = (inner + (row as f32 + 0.5) * ROW_DEPTH) * s;
                let len = 2.0 * (r_mid + ROW_DEPTH * 0.5) * (seg * 0.5).tan() + 0.8;
                let top = base + row as f32 * ROW_RISE;
                let h = ROW_RISE + 0.6;
                out.push(Piece { center: dir * r_mid + Vec3::Y * (top - h * 0.5), size: Vec3::new(len, h, ROW_DEPTH + 0.15), yaw, kind: PieceKind::Step });
            }
            let wall_r = self.wall_r() * s;
            let wall_h = self.wall_h();
            let wall_len = 2.0 * wall_r * (seg * 0.5).tan() + 0.9;
            out.push(Piece { center: dir * wall_r + Vec3::Y * wall_h * 0.5, size: Vec3::new(wall_len, wall_h, 2.2), yaw, kind: PieceKind::Wall });
            if i % 2 == 0 {
                out.push(Piece { center: dir * wall_r + Vec3::Y * (wall_h + 1.1), size: Vec3::new(wall_len * 0.55, 2.2, 2.2), yaw, kind: PieceKind::Crenel });
            }
        }
        out
    }

    /// Solid parts for shots and movement: facade, steps and back wall.
    pub fn collision(&self) -> Vec<YawBox> {
        self.pieces().into_iter().filter(|p| matches!(p.kind, PieceKind::Facade | PieceKind::Step | PieceKind::Wall))
            .map(|p| YawBox::new(p.center, p.size, p.yaw)).collect()
    }

    /// Height of the tread under a point (0 inside the arena).
    pub fn ground(&self, p: Vec3) -> f32 {
        let r = (p.x * p.x + p.z * p.z).sqrt() / sq(p.z.atan2(p.x));
        if r < self.inner_r { return 0.0; }
        let row = ((r - self.inner_r) / ROW_DEPTH).floor().clamp(0.0, ROWS as f32 - 1.0);
        self.base_y + row * ROW_RISE
    }

    /// Distance from a point to the crowd band (0 inside it).
    pub fn distance_to_crowd(&self, p: Vec3) -> f32 {
        let r = (p.x * p.x + p.z * p.z).sqrt();
        let (r0, r1) = (self.inner_r, (self.inner_r + ROWS as f32 * ROW_DEPTH + 2.2) * 1.19);
        let (y0, y1) = (self.base_y, self.wall_h());
        let dr = (r0 - r).max(r - r1).max(0.0);
        let dy = (y0 - p.y).max(p.y - y1).max(0.0);
        (dr * dr + dy * dy).sqrt()
    }

    /// The nearest point among the seats, toward `p` (where crowd reactions
    /// are heard from).
    pub fn ring_point_toward(&self, p: Vec3) -> Vec3 {
        let d = glam::Vec2::new(p.x, p.z).try_normalize().unwrap_or(glam::Vec2::X);
        let r = self.inner_r + 3.0;
        Vec3::new(d.x * r, self.base_y + (self.wall_h() - self.base_y) * 0.35, d.y * r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bowl_rings_the_arena_and_rises_above_the_walls() {
        let s = Stadium::new(80.0, true, 12.0);
        assert!((s.inner_r - 65.6).abs() < 1e-4 && (s.base_y - 14.5).abs() < 1e-4);
        let p = s.pieces();
        assert_eq!(p.iter().filter(|p| p.kind == PieceKind::Step).count(), SEGMENTS * ROWS);
        assert_eq!(s.collision().len(), SEGMENTS * (ROWS + 2));
        assert!((s.ground(Vec3::new(s.inner_r + 1.0, 0.0, 0.0)) - s.base_y).abs() < 1e-4);
        assert_eq!(s.ground(Vec3::ZERO), 0.0);
        assert_eq!(s.distance_to_crowd(Vec3::new(s.inner_r + 5.0, s.base_y + 3.0, 0.0)), 0.0);
        assert!((sq(PI / 4.0) - 1.189).abs() < 1e-3);
    }
}
