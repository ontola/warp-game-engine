//! Procedural arena, after Godot's `arena_generator.gd`: a square floor,
//! perimeter walls with corner castle towers, an optional centre tower,
//! mirrored building pairs with wall-jump ledges, approach pillars and
//! floaters, cover blocks and floating platforms. Every feature has a 180
//! degree mirror at (-x, -z), so the map plays fair from any spawn.
//!
//! Not ported in this slice: the all-floor-lava island layout, bridges,
//! tunnels, diagonal walls, decorations, colosseum and destruction.
use crate::rng::Rng;
use crate::shape::{Block, Kind, YawBox};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

const TOWER_FOOTPRINT: f32 = 7.5;
const TOWER_LIFT: f32 = 3.5;
const PARAPET_OVERHANG: f32 = 1.15;
const PARAPET_HEIGHT: f32 = 1.35;
const MERLON: f32 = 1.2;
const MERLON_HEIGHT: f32 = 1.35;
const WALL_HEIGHT: f32 = 12.0;
/// Below this a player is in the lava under the floor (instant death).
pub const LAVA_KILL_Y: f32 = -2.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Variant { Walled, Hole, Open }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Arena {
    pub seed: u64,
    pub size: f32,
    pub variant: Variant,
    pub blocks: Vec<Block>,
    /// Collision shapes of `blocks`, same order (call `reshape` after edits).
    pub shapes: Vec<YawBox>,
    pub spawns: Vec<Vec3>,
    /// Roof/beacon light positions and colours (Godot omni lights).
    pub lights: Vec<(Vec3, [f32; 3])>,
    /// Square hole in the floor (centre XZ, side), lava visible below.
    pub hole: Option<(Vec2, f32)>,
}

struct Placer { placed: Vec<(Vec2, f32)> }

impl Placer {
    fn try_place(&mut self, p: Vec3, r: f32) -> bool {
        let a = Vec2::new(p.x, p.z);
        let m = -a;
        for &(q, qr) in &self.placed {
            if a.distance(q) < r + qr || m.distance(q) < r + qr { return false; }
        }
        // A feature must not overlap its own mirror either.
        if a.length() * 2.0 < r * 2.0 { return false; }
        self.placed.push((a, r));
        self.placed.push((m, r));
        true
    }
}

fn mirror(p: Vec3) -> Vec3 { Vec3::new(-p.x, p.y, -p.z) }

impl Arena {
    pub fn generate(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let size = rng.range(50.0, 95.0).round();
        let roll = rng.f();
        // Godot: 18% island lava (not ported: becomes walled), 24% open, 16% hole.
        let variant = if (0.18..0.42).contains(&roll) { Variant::Open }
            else if (0.42..0.58).contains(&roll) { Variant::Hole } else { Variant::Walled };
        let mut a = Arena { seed, size, variant, blocks: Vec::new(), shapes: Vec::new(), spawns: Vec::new(), lights: Vec::new(), hole: None };
        let mut placer = Placer { placed: Vec::new() };
        let half = size * 0.5;
        if variant == Variant::Hole {
            let side = rng.range(10.0, 16.0);
            let max_off = size * 0.25;
            let c = Vec2::new(rng.range(-max_off, max_off), rng.range(-max_off, max_off));
            a.hole = Some((c, side));
            placer.placed.push((c, side * 0.5 + 1.5));
        }
        a.build_floor();
        if variant != Variant::Open {
            a.build_walls(half, &mut placer);
        }
        let perim = half - 1.5;
        if rng.f() < 0.6 {
            let h = rng.range(10.0, 14.0);
            a.tower(Vec3::ZERO, 6.5, h, Kind::Building);
            a.lights.push((Vec3::new(0.0, tower_top(h) + 1.5, 0.0), [1.0, 0.4, 0.3]));
            placer.placed.push((Vec2::ZERO, 4.5));
        } else {
            placer.placed.push((Vec2::ZERO, 2.0));
        }
        // Buildings: one pair per primary, each with a ledge 60% up.
        let mut buildings = Vec::new();
        for _ in 0..2 {
            for _ in 0..40 {
                let (w, d, h) = (rng.range(7.0, 11.0), rng.range(7.0, 11.0), rng.range(8.0, 12.0));
                let max_dim = w.max(d);
                let inset = max_dim * 0.5 + 2.0;
                let x = rng.range(-perim + inset, -max_dim * 0.5 - 4.0);
                let z = rng.range(-perim + inset, perim - inset);
                let pos = Vec3::new(x, h * 0.5, z);
                let yaw = if rng.f() < 0.55 { rng.f() * PI * 0.5 } else { 0.0 };
                if !placer.try_place(pos, (w * w + d * d).sqrt() * 0.5 + 1.5) { continue; }
                let face = rng.below(4);
                let light = if rng.f() < 0.5 { [1.0, 0.35, 0.2] } else { [1.0, 0.7, 0.2] };
                a.building(pos, Vec3::new(w, h, d), yaw, face, light);
                a.building(mirror(pos), Vec3::new(w, h, d), yaw + PI, face, light);
                a.spawns.push(Vec3::new(pos.x, h + 1.0, pos.z));
                a.spawns.push(mirror(Vec3::new(pos.x, h + 1.0, pos.z)));
                buildings.push((pos, Vec3::new(w, h, d)));
                break;
            }
        }
        // Approach pillars: wall-jump distance from a building face.
        for &(bp, bs) in &buildings {
            let extent = bs.x.max(bs.z) * 0.5;
            for _ in 0..20 {
                let ang = rng.f() * TAU;
                let dist = rng.range(5.0, 7.5);
                let (px, pz) = (bp.x + ang.cos() * (extent + dist), bp.z + ang.sin() * (extent + dist));
                if px.abs() > perim - 2.0 || pz.abs() > perim - 2.0 { continue; }
                let ph = bs.y * rng.range(0.4, 0.6);
                let pos = Vec3::new(px, ph * 0.5, pz);
                if !placer.try_place(pos, 2.5) { continue; }
                a.blocks.push(Block::new(pos, Vec3::new(3.0, ph, 3.0), 0.0, Kind::Pillar));
                a.blocks.push(Block::new(mirror(pos), Vec3::new(3.0, ph, 3.0), 0.0, Kind::Pillar));
                break;
            }
        }
        // Approach floaters at dash distance, mid-height.
        for &(bp, bs) in &buildings {
            if rng.f() >= 0.7 { continue; }
            let extent = bs.x.max(bs.z) * 0.5;
            for _ in 0..20 {
                let ang = rng.f() * TAU;
                let dist = rng.range(3.0, 5.5);
                let (px, pz) = (bp.x + ang.cos() * (extent + dist), bp.z + ang.sin() * (extent + dist));
                if px.abs() > perim - 2.0 || pz.abs() > perim - 2.0 { continue; }
                let pos = Vec3::new(px, bs.y * rng.range(0.55, 0.75), pz);
                if !placer.try_place(pos, 3.5) { continue; }
                a.platform(pos);
                a.platform(mirror(pos));
                break;
            }
        }
        // Free-standing pillar pair.
        for _ in 0..40 {
            let x = rng.range(-perim + 3.0, -2.0);
            let z = rng.range(-perim + 3.0, perim - 3.0);
            let pos = Vec3::new(x, 4.0, z);
            if !placer.try_place(pos, 3.0) { continue; }
            a.blocks.push(Block::new(pos, Vec3::new(3.0, 8.0, 3.0), 0.0, Kind::Pillar));
            a.blocks.push(Block::new(mirror(pos), Vec3::new(3.0, 8.0, 3.0), 0.0, Kind::Pillar));
            break;
        }
        // Low cover blocks at random angles.
        let cover = Vec3::new(3.5, 2.0, 1.2);
        for _ in 0..6 {
            for _ in 0..40 {
                let x = rng.range(-perim + 2.0, -1.5);
                let z = rng.range(-perim + 2.0, perim - 2.0);
                let pos = Vec3::new(x, cover.y * 0.5, z);
                if !placer.try_place(pos, cover.x * 0.55 + 0.5) { continue; }
                let yaw = rng.f() * TAU;
                a.blocks.push(Block::new(pos, cover, yaw, Kind::Cover));
                a.blocks.push(Block::new(mirror(pos), cover, yaw + PI, Kind::Cover));
                break;
            }
        }
        // Floating platform pair.
        for _ in 0..30 {
            let x = rng.range(-perim + 4.0, -3.0);
            let z = rng.range(-perim + 4.0, perim - 4.0);
            let pos = Vec3::new(x, rng.range(5.0, 9.0), z);
            if !placer.try_place(pos, 3.5) { continue; }
            a.platform(pos);
            a.platform(mirror(pos));
            break;
        }
        // Always-present corner ground spawns.
        let c = size * 0.42;
        for (x, z) in [(-c, -c), (c, c), (-c, c), (c, -c)] {
            a.spawns.push(Vec3::new(x, 1.0, z));
        }
        a.reshape();
        if let Some((hc, side)) = a.hole {
            a.spawns.retain(|s| (s.x - hc.x).abs() > side * 0.5 + 1.0 || (s.z - hc.y).abs() > side * 0.5 + 1.0);
        }
        a
    }

    fn build_floor(&mut self) {
        let s = self.size;
        let half = s * 0.5;
        let slab = |c: Vec2, sx: f32, sz: f32| Block::new(Vec3::new(c.x, -1.5, c.y), Vec3::new(sx, 3.0, sz), 0.0, Kind::Floor);
        match self.hole {
            None => self.blocks.push(slab(Vec2::ZERO, s, s)),
            Some((h, side)) => {
                let hh = side * 0.5;
                let n = (h.y - hh) + half;
                if n > 0.1 { self.blocks.push(slab(Vec2::new(0.0, (-half + h.y - hh) * 0.5), s, n)); }
                let so = half - (h.y + hh);
                if so > 0.1 { self.blocks.push(slab(Vec2::new(0.0, (h.y + hh + half) * 0.5), s, so)); }
                let w = (h.x - hh) + half;
                if w > 0.1 { self.blocks.push(slab(Vec2::new((-half + h.x - hh) * 0.5, h.y), w, side)); }
                let e = half - (h.x + hh);
                if e > 0.1 { self.blocks.push(slab(Vec2::new((h.x + hh + half) * 0.5, h.y), e, side)); }
            }
        }
    }

    fn build_walls(&mut self, half: f32, placer: &mut Placer) {
        let s = self.size;
        let y = WALL_HEIGHT * 0.5;
        for (c, size) in [
            (Vec3::new(0.0, y, -half), Vec3::new(s, WALL_HEIGHT, 1.0)),
            (Vec3::new(0.0, y, half), Vec3::new(s, WALL_HEIGHT, 1.0)),
            (Vec3::new(half, y, 0.0), Vec3::new(1.0, WALL_HEIGHT, s)),
            (Vec3::new(-half, y, 0.0), Vec3::new(1.0, WALL_HEIGHT, s)),
        ] { self.blocks.push(Block::new(c, size, 0.0, Kind::Wall)); }
        for (x, z) in [(half, -half), (half, half), (-half, half), (-half, -half)] {
            self.tower(Vec3::new(x, 0.0, z), TOWER_FOOTPRINT, WALL_HEIGHT + TOWER_LIFT, Kind::Tower);
            placer.placed.push((Vec2::new(x, z), TOWER_FOOTPRINT * 0.55 + PARAPET_OVERHANG + 1.5));
        }
    }

    /// Castle tower: shaft, overhanging parapet and a merlon ring.
    fn tower(&mut self, base: Vec3, footprint: f32, shaft: f32, kind: Kind) {
        self.blocks.push(Block::new(Vec3::new(base.x, shaft * 0.5, base.z), Vec3::new(footprint, shaft, footprint), 0.0, kind));
        let w = footprint + PARAPET_OVERHANG * 2.0;
        self.blocks.push(Block::new(Vec3::new(base.x, shaft + PARAPET_HEIGHT * 0.5, base.z), Vec3::new(w, PARAPET_HEIGHT, w), 0.0, Kind::Ledge));
        let y = shaft + PARAPET_HEIGHT + MERLON_HEIGHT * 0.5;
        let n = ((w / (MERLON * 2.0)).floor() as i32).max(2);
        let step = (w - MERLON) / (n - 1) as f32;
        for i in 0..n {
            let t = -w * 0.5 + MERLON * 0.5 + step * i as f32;
            for (dx, dz) in [(t, -w * 0.5 + MERLON * 0.5), (t, w * 0.5 - MERLON * 0.5), (-w * 0.5 + MERLON * 0.5, t), (w * 0.5 - MERLON * 0.5, t)] {
                self.blocks.push(Block::new(Vec3::new(base.x + dx, y, base.z + dz), Vec3::splat(MERLON).with_y(MERLON_HEIGHT), 0.0, Kind::Ledge));
            }
        }
    }

    fn building(&mut self, pos: Vec3, size: Vec3, yaw: f32, face: u32, light: [f32; 3]) {
        self.blocks.push(Block::new(pos, size, yaw, Kind::Building));
        let ledge_face = (face + 2) % 4;
        let ang = yaw + ledge_face as f32 * PI * 0.5;
        let off = if ledge_face % 2 == 0 { size.x } else { size.z } * 0.5 + 1.0;
        let ledge = pos + Vec3::new(ang.cos() * off, size.y * 0.6 - size.y * 0.5, ang.sin() * off);
        let align = if ledge_face % 2 == 0 { PI * 0.5 } else { 0.0 };
        self.blocks.push(Block::new(ledge, Vec3::new(6.0, 0.5, 2.0), yaw + align, Kind::Ledge));
        self.lights.push((pos + Vec3::new(0.0, size.y * 0.5 + 2.0, 0.0), light));
    }

    fn platform(&mut self, pos: Vec3) {
        self.blocks.push(Block::new(pos, Vec3::new(5.0, 0.6, 5.0), 0.0, Kind::Platform));
    }

    /// Rebuild `shapes` from `blocks`.
    pub fn reshape(&mut self) { self.shapes = self.blocks.iter().map(Block::shape).collect(); }

    /// Fell into the lava below the floor (hole or open arena edge).
    pub fn in_lava(&self, p: Vec3) -> bool { p.y < LAVA_KILL_Y }
}

fn tower_top(shaft: f32) -> f32 { shaft + PARAPET_HEIGHT + MERLON_HEIGHT }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arenas_are_deterministic_mirrored_and_have_spawns() {
        for seed in 0..40 {
            let a = Arena::generate(seed);
            let b = Arena::generate(seed);
            assert_eq!(a.blocks, b.blocks);
            assert!(a.spawns.len() >= 4, "seed {seed}");
            assert!((50.0..=95.0).contains(&a.size));
            // Every non-floor block has a mirror twin.
            for blk in a.blocks.iter().filter(|b| b.kind != Kind::Floor && b.kind != Kind::Wall) {
                let m = mirror(blk.center);
                assert!(a.blocks.iter().any(|o| o.center.distance(m) < 1e-3 && o.half == blk.half), "seed {seed} {blk:?}");
            }
        }
        let variants: std::collections::HashSet<_> = (0..60).map(|s| format!("{:?}", Arena::generate(s).variant)).collect();
        assert_eq!(variants.len(), 3);
    }
}
