//! Procedural arena, after Godot's `arena_generator.gd`: a square floor,
//! perimeter walls with corner castle towers, an optional centre tower,
//! mirrored building pairs with wall-jump ledges, approach pillars and
//! floaters, cover blocks and floating platforms. Every feature has a 180
//! degree mirror at (-x, -z), so the map plays fair from any spawn.
//!
//! Everything is destructible, as in Godot: each block is pre-split into
//! uneven chunks (see `destruct.rs`) and collision runs on the live chunks.
//!
//! The all-floor-lava island variant (Godot `_build_lava_island_layout`,
//! 18% of arenas): the floor is lava at 0.04 m, players stand on mirrored
//! stone columns within jump reach of each other.
//!
//! Not ported in this slice: bridges between buildings, tunnels, diagonal
//! walls, decorations and colosseum.
use crate::rng::Rng;
use crate::destruct::Chunk;
use crate::shape::{Block, BoxGrid, Kind};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// Linear scale of authored architecture in newly generated matches.
pub const LEVEL_SCALE: f32 = 0.8;
pub const STANDARD_SIZE: (f32, f32) = (44.0, 76.0);
pub const SNIPER_SIZE: (f32, f32) = (88.0, 96.0);

const TOWER_FOOTPRINT: f32 = 7.5;
const TOWER_LIFT: f32 = 3.5;
const PARAPET_OVERHANG: f32 = 1.15;
const PARAPET_HEIGHT: f32 = 1.35;
const MERLON: f32 = 1.2;
const MERLON_HEIGHT: f32 = 1.35;
pub const WALL_HEIGHT: f32 = 12.0;
/// Below this a player is in the lava under the floor (Godot LAVA_POOL_Y:
/// a kill that skips the phoenix).
pub const LAVA_KILL_Y: f32 = -3.0;
/// Within this much above the pool the heat burns 15 per half second.
pub const LAVA_BURN_BAND: f32 = 1.2;
/// Island arenas: the lava floor's surface (Godot LAVA_FLOOR_SURFACE_Y).
/// Below it a player dies; up to `LAVA_FLOOR_BURN` above it, off the
/// islands, the heat burns 15 per half second.
pub const LAVA_FLOOR_Y: f32 = 0.04;
pub const LAVA_FLOOR_BURN: f32 = 1.15;
const ISLAND_TOP_Y: f32 = 1.0;
const ISLAND_MAX_GAP: f32 = 5.6;
const ISLAND_MIN_TOP: f32 = 0.9;
const ISLAND_MAX_TOP: f32 = 9.0;
/// The tallest step between linked islands (jump plus double jump).
pub const ISLAND_MAX_STEP: f32 = 2.05;
/// Spawns keep the capsule this far in from an island's safe disk.
const SPAWN_INSET: f32 = 0.65;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Variant { Walled, Hole, Open, Island }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(into = "crate::destruct::ArenaWire", from = "crate::destruct::ArenaWire")]
pub struct Arena {
    pub seed: u64,
    pub size: f32,
    pub variant: Variant,
    pub blocks: Vec<Block>,
    /// Destructible pieces of `blocks` (built by `build_chunks`).
    pub chunks: Vec<Chunk>,
    /// Collision: one box per chunk (same indices; destroyed chunks
    /// removed), then the stadium's solid pieces.
    pub grid: BoxGrid,
    /// The colosseum around the arena.
    pub stadium: crate::stadium::Stadium,
    pub spawns: Vec<Vec3>,
    /// Roof/beacon light positions and colours (Godot omni lights).
    pub lights: Vec<(Vec3, [f32; 3])>,
    /// Square hole in the floor (centre XZ, side), lava visible below.
    pub hole: Option<(Vec2, f32)>,
    /// Island arenas: each island's top centre and half side. Its safe disk
    /// (no burn) has radius half side + 0.25, as in Godot, so the corners of
    /// the square column burn.
    pub islands: Vec<(Vec3, f32)>,
    /// Island arenas: the jumps the generator guarantees (top to top).
    pub links: Vec<(Vec3, Vec3)>,
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

/// Fixed view pool for the chunks and lights of standard and sniper arenas.
pub const MAX_PIECES: usize = 16_384;

impl Arena {
    /// No geometry (a world before its first round).
    pub fn empty() -> Self {
        Arena { seed: 0, size: 55.0, variant: Variant::Open, blocks: Vec::new(), chunks: Vec::new(), grid: BoxGrid::default(), stadium: crate::stadium::Stadium::new(55.0, false, WALL_HEIGHT), spawns: vec![Vec3::new(0.0, 1.0, 0.0)], lights: Vec::new(), hole: None, islands: Vec::new(), links: Vec::new() }
    }

    /// Standard match size in world metres.
    pub fn generate(seed: u64) -> Self { Self::generate_sized(seed, STANDARD_SIZE.0, STANDARD_SIZE.1) }

    pub fn generate_sized(seed: u64, lo: f32, hi: f32) -> Self { Self::generate_with(seed, lo, hi, false) }

    /// `island`: always the island layout (Godot `force_all_floor_lava`).
    pub fn generate_with(seed: u64, lo: f32, hi: f32, island: bool) -> Self {
        let mut rng = Rng::new(seed);
        let size = rng.range(lo / LEVEL_SCALE, hi / LEVEL_SCALE).round().clamp(lo / LEVEL_SCALE, hi / LEVEL_SCALE);
        // One roll picks the variant (Godot `variant_roll`): 18% islands,
        // 24% open, 16% hole, 42% walled. Nothing else is drawn here, so the
        // other variants draw the same stream whether or not islands exist.
        let roll = rng.f();
        let variant = if island || roll < 0.18 { Variant::Island } else if roll < 0.42 { Variant::Open }
            else if roll < 0.58 { Variant::Hole } else { Variant::Walled };
        let mut a = Arena { seed, size, variant, blocks: Vec::new(), chunks: Vec::new(), grid: BoxGrid::default(), stadium: crate::stadium::Stadium::new(size, variant != Variant::Open, WALL_HEIGHT), spawns: Vec::new(), lights: Vec::new(), hole: None, islands: Vec::new(), links: Vec::new() };
        let mut placer = Placer { placed: Vec::new() };
        let half = size * 0.5;
        if variant == Variant::Island {
            // Walls and corner towers, no floor: lava at 0.04 m instead.
            a.build_walls(half, &mut placer);
            a.island_layout(&mut rng, &mut placer, half - 3.0);
            a.finish_layout();
            return a;
        }
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
        a.finish_layout();
        if let Some((hc, side)) = a.hole {
            a.spawns.retain(|s| (s.x - hc.x).abs() > side * 0.5 + 1.0 || (s.z - hc.y).abs() > side * 0.5 + 1.0);
        }
        a
    }

    /// Convert the complete authored layout once, before cutting chunks or building collision.
    /// Spawn markers sit one metre above their support, independently of the architecture.
    fn finish_layout(&mut self) {
        let scale = LEVEL_SCALE;
        self.size *= scale;
        for b in &mut self.blocks {
            b.center *= scale;
            b.half *= scale;
        }
        for pos in &mut self.spawns { *pos = (*pos - Vec3::Y) * scale + Vec3::Y; }
        for (pos, _) in &mut self.lights { *pos *= scale; }
        for (top, radius) in &mut self.islands { *top *= scale; *radius *= scale; }
        for (a, b) in &mut self.links { *a *= scale; *b *= scale; }
        if let Some((center, side)) = &mut self.hole { *center *= scale; *side *= scale; }
        if self.variant == Variant::Island {
            let spots = std::mem::take(&mut self.spawns);
            self.spawns = spots.into_iter().filter(|p| self.spawn_safe(*p)).collect();
        }
        self.stadium = crate::stadium::Stadium::for_arena(self.size, self.variant != Variant::Open, scale);
        self.build_chunks();
    }

    fn build_floor(&mut self) {
        let s = self.size;
        let half = s * 0.5;
        // Three 1 m layers (Godot `_add_floor_slab`): the walk surface at
        // y = 0, lava under the bottom at y = -3. Dig through and you burn.
        let mut slabs = Vec::new();
        match self.hole {
            None => slabs.push((Vec2::ZERO, s, s)),
            Some((h, side)) => {
                let hh = side * 0.5;
                let n = (h.y - hh) + half;
                if n > 0.1 { slabs.push((Vec2::new(0.0, (-half + h.y - hh) * 0.5), s, n)); }
                let so = half - (h.y + hh);
                if so > 0.1 { slabs.push((Vec2::new(0.0, (h.y + hh + half) * 0.5), s, so)); }
                let w = (h.x - hh) + half;
                if w > 0.1 { slabs.push((Vec2::new((-half + h.x - hh) * 0.5, h.y), w, side)); }
                let e = half - (h.x + hh);
                if e > 0.1 { slabs.push((Vec2::new((h.x + hh + half) * 0.5, h.y), e, side)); }
            }
        }
        for (c, sx, sz) in slabs {
            for layer in 0..3 {
                let y = -0.5 - layer as f32;
                self.blocks.push(Block::new(Vec3::new(c.x, y, c.y), Vec3::new(sx, 1.0, sz), 0.0, Kind::Floor).tiled());
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
        ] { self.blocks.push(Block::new(c, size, 0.0, Kind::Wall).tiled()); }
        for (x, z) in [(half, -half), (half, half), (-half, half), (-half, -half)] {
            self.tower(Vec3::new(x, 0.0, z), TOWER_FOOTPRINT, WALL_HEIGHT + TOWER_LIFT, Kind::Tower);
            placer.placed.push((Vec2::new(x, z), TOWER_FOOTPRINT * 0.55 + PARAPET_OVERHANG + 1.5));
        }
    }

    /// Castle tower: shaft, overhanging parapet and a merlon ring.
    fn tower(&mut self, base: Vec3, footprint: f32, shaft: f32, kind: Kind) {
        self.blocks.push(Block::new(Vec3::new(base.x, shaft * 0.5, base.z), Vec3::new(footprint, shaft, footprint), 0.0, kind).tiled());
        let w = footprint + PARAPET_OVERHANG * 2.0;
        self.blocks.push(Block::new(Vec3::new(base.x, shaft + PARAPET_HEIGHT * 0.5, base.z), Vec3::new(w, PARAPET_HEIGHT, w), 0.0, Kind::Ledge).tiled());
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
        let off = if ledge_face.is_multiple_of(2) { size.x } else { size.z } * 0.5 + 1.0;
        let ledge = pos + Vec3::new(ang.cos() * off, size.y * 0.6 - size.y * 0.5, ang.sin() * off);
        let align = if ledge_face.is_multiple_of(2) { PI * 0.5 } else { 0.0 };
        self.blocks.push(Block::new(ledge, Vec3::new(6.0, 0.5, 2.0), yaw + align, Kind::Ledge));
        self.lights.push((pos + Vec3::new(0.0, size.y * 0.5 + 2.0, 0.0), light));
    }

    fn platform(&mut self, pos: Vec3) {
        self.blocks.push(Block::new(pos, Vec3::new(5.0, 0.6, 5.0), 0.0, Kind::Platform));
    }

    /// Godot `_build_lava_island_layout` (not the cinematic fort): a centre
    /// island, a spine of 3 to 5 rising islands towards -X, 10 to 18 more
    /// grown off random islands of the -X half, two stepping stones on the
    /// axis, all mirrored; then planks over some links and cover on some
    /// islands (neither mirrored, as in Godot). Every island is within jump
    /// reach of the one it grew from (`max_gap`, `ISLAND_MAX_STEP`).
    fn island_layout(&mut self, rng: &mut Rng, placer: &mut Placer, perim: f32) {
        let center_r = 4.2;
        let center = Vec3::new(0.0, ISLAND_TOP_Y, 0.0);
        self.island(center, center_r, 1.0);
        placer.placed.push((Vec2::ZERO, center_r + 0.9));
        let (mut anchor, mut anchor_r) = (center, center_r);
        for _ in 0..3 + rng.below(3) {
            let radius = rng.range(3.0, 4.7);
            let step = rng.range(1.25, ISLAND_MAX_STEP);
            let top = (anchor.y + step).clamp(ISLAND_MIN_TOP, ISLAND_MAX_TOP);
            let gap = rng.range(2.7, max_gap(top - anchor.y));
            let dist = anchor_r + radius + gap;
            let dir = Vec2::new(-1.0, rng.range(-0.28, 0.28)).normalize();
            let pos = Vec3::new(anchor.x + dir.x * dist, top, anchor.z + dir.y * dist);
            if pos.x.abs() + radius > perim || pos.z.abs() + radius > perim { break; }
            if !placer.try_place(pos, radius + 0.9) { break; }
            let extra = rng.range(0.6, 1.4);
            self.island(pos, radius, extra);
            self.island(mirror(pos), radius, extra);
            self.links.push((anchor, pos));
            self.links.push((mirror(anchor), mirror(pos)));
            (anchor, anchor_r) = (pos, radius);
        }
        let pairs = ((self.size / 6.0).round() as i32 + 1 + rng.below(4) as i32).clamp(10, 18);
        for _ in 0..pairs {
            for _ in 0..80 {
                let (mut from, from_r) = self.islands[rng.below(self.islands.len() as u32) as usize];
                if from.x > 0.0 { from = mirror(from); }
                let radius = rng.range(2.2, 5.2);
                let angle = rng.f() * TAU;
                let mut step = rng.range(-1.55, ISLAND_MAX_STEP);
                if rng.f() < 0.58 && from.y < ISLAND_MAX_TOP - 0.7 { step = rng.range(0.6, ISLAND_MAX_STEP); }
                let top = (from.y + step).clamp(ISLAND_MIN_TOP, ISLAND_MAX_TOP);
                let gap = rng.range(2.8, max_gap(top - from.y));
                let dist = from_r + radius + gap;
                let pos = Vec3::new(from.x + angle.cos() * dist, top, from.z + angle.sin() * dist);
                if pos.x > -0.35 { continue; }
                if pos.x.abs() + radius > perim || pos.z.abs() + radius > perim { continue; }
                if (pos.y - from.y).abs() > ISLAND_MAX_STEP { continue; }
                if !placer.try_place(pos, radius + 0.9) { continue; }
                let extra = rng.range(0.45, 1.5);
                self.island(pos, radius, extra);
                self.island(mirror(pos), radius, extra);
                self.links.push((from, pos));
                self.links.push((mirror(from), mirror(pos)));
                break;
            }
        }
        // Stepping stones across the middle, each within reach of the centre.
        let axis = Vec3::new(-(center_r + 3.2 + ISLAND_MAX_GAP * 0.55), ISLAND_TOP_Y + 0.15, 0.0);
        if placer.try_place(axis, 3.1) {
            self.island(axis, 3.0, 1.25);
            self.island(mirror(axis), 3.0, 1.25);
            self.links.push((center, axis));
            self.links.push((center, mirror(axis)));
        }
        // Planks over some links (Godot `_build_lava_island_bridges`). Godot
        // turns them by atan2(dz, dx) as rotation.y, which mirrors them in Z
        // unless the link runs along X; kept as it plays.
        for k in 0..self.links.len() {
            if rng.f() > 0.22 { continue; }
            let (p, q) = self.links[k];
            let d = Vec2::new(q.x - p.x, q.z - p.z);
            let len = d.length();
            if len < 7.0 { continue; }
            let mut mid = (p + q) * 0.5;
            mid.y = p.y.min(q.y) + 0.08;
            self.blocks.push(Block::new(mid, Vec3::new(len - 4.8, 0.22, 1.0), d.y.atan2(d.x), Kind::Island));
        }
        // Low cover on some of the larger islands (`_add_lava_island_cover`).
        for k in 0..self.islands.len() {
            if rng.f() > 0.38 { continue; }
            let (top, radius) = self.islands[k];
            if radius < 3.0 { continue; }
            let angle = rng.f() * TAU;
            let size = Vec3::new(rng.range(1.8, 2.8), 1.1, rng.range(0.8, 1.2));
            let min_off = (size.x.max(size.z) * 0.5 + 1.35).max(radius * 0.58);
            let max_off = (min_off + 0.2).max(radius * 0.88);
            let off = rng.range(min_off, max_off);
            let at = Vec3::new(top.x + angle.cos() * off, top.y + 0.55, top.z + angle.sin() * off);
            let yaw = rng.f() * TAU;
            self.blocks.push(Block::new(at, size, yaw, Kind::Island));
        }
        // `_prune_lava_spawn_positions`.
        let spawns = std::mem::take(&mut self.spawns);
        self.spawns = spawns.into_iter().filter(|&s| self.spawn_safe(s)).collect();
        if self.spawns.is_empty() { self.spawns.push(center + Vec3::Y); }
    }

    /// Godot `_register_lava_platform`: a square column from below the lava
    /// up to `top`, its safe disk and a spawn on it.
    fn island(&mut self, top: Vec3, radius: f32, extra: f32) {
        let h = (top.y - LAVA_FLOOR_Y + extra).max(0.45);
        self.blocks.push(Block::new(Vec3::new(top.x, top.y - h * 0.5, top.z), Vec3::new(radius * 2.0, h, radius * 2.0), 0.0, Kind::Island));
        self.islands.push((top, radius));
        self.spawns.push(top + Vec3::Y);
    }

    /// The lava's surface: the floor of an island arena, else the pool.
    pub fn lava_surface(&self) -> f32 { if self.variant == Variant::Island { LAVA_FLOOR_Y } else { LAVA_KILL_Y } }

    /// Fell into the lava (below the floor, or the floor of an island arena).
    pub fn in_lava(&self, p: Vec3) -> bool { p.y < self.lava_surface() }

    /// Over an island's safe disk (Godot `_point_in_lava_safe_zone`).
    pub fn on_island(&self, xz: Vec2) -> bool {
        self.islands.iter().any(|&(t, r)| xz.distance(Vec2::new(t.x, t.z)) <= r + 0.25)
    }

    /// Godot `is_lava_spawn_safe`: always on other arenas; on islands, a
    /// capsule centre clear of the burn band, well inside a safe disk.
    pub fn spawn_safe(&self, p: Vec3) -> bool {
        if self.variant != Variant::Island { return true; }
        if p.y < LAVA_FLOOR_Y + LAVA_FLOOR_BURN + 0.5 { return false; }
        let xz = Vec2::new(p.x, p.z);
        self.islands.iter().any(|&(t, r)| xz.distance(Vec2::new(t.x, t.z)) <= (r + 0.25 - SPAWN_INSET).max(0.35))
    }
}

/// Godot `_lava_max_gap_for_height_step`: the widest jump for a climb.
pub fn max_gap(step: f32) -> f32 {
    let t = (step.abs() / ISLAND_MAX_STEP).clamp(0.0, 1.0);
    ISLAND_MAX_GAP + (3.1 - ISLAND_MAX_GAP) * t
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
            assert!((STANDARD_SIZE.0..=STANDARD_SIZE.1).contains(&a.size));
            // Every non-floor block has a mirror twin (island planks and
            // cover are not mirrored, as in Godot).
            for blk in a.blocks.iter().filter(|b| b.kind != Kind::Floor && b.kind != Kind::Wall && b.kind != Kind::Island) {
                let m = mirror(blk.center);
                assert!(a.blocks.iter().any(|o| o.center.distance(m) < 1e-3 && o.half == blk.half), "seed {seed} {blk:?}");
            }
        }
        let variants: std::collections::HashSet<_> = (0..60).map(|s| format!("{:?}", Arena::generate(s).variant)).collect();
        assert_eq!(variants.len(), 4);
    }

    /// Views keep the arena in a fixed pool of MAX_PIECES slots.
    #[test]
    fn the_largest_arenas_fit_the_view_pool() {
        let mut worst = 0;
        for seed in 0..120u64 {
            for (lo, hi) in [STANDARD_SIZE, SNIPER_SIZE] {
                let a = Arena::generate_sized(seed.wrapping_mul(2654435761) & 0xffff_ffff, lo, hi);
                worst = worst.max(a.chunks.len() + a.lights.len());
            }
        }
        assert!(worst > 6_000, "the sniper arenas are the big ones ({worst})");
        assert!(worst < MAX_PIECES * 9 / 10, "{worst} pieces leave too little headroom");
    }

    /// Godot `_validate_lava_platform_graph`: every link within jump reach,
    /// every island reachable from the centre; spawns on safe ground.
    #[test]
    fn island_arenas_are_connected_and_spawn_safely() {
        let mut islands = 0;
        for seed in 0..200 {
            let a = Arena::generate(seed);
            if a.variant != Variant::Island { assert!(a.islands.is_empty() && a.links.is_empty()); continue; }
            islands += 1;
            assert!(a.islands.len() >= 8, "seed {seed}: {} islands", a.islands.len());
            let find = |p: Vec3| a.islands.iter().position(|&(t, _)| t.distance(p) < 1e-3).unwrap_or_else(|| panic!("seed {seed}: link to no island {p}"));
            let mut adj = vec![Vec::new(); a.islands.len()];
            for &(p, q) in &a.links {
                let (i, j) = (find(p), find(q));
                let gap = Vec2::new(p.x - q.x, p.z - q.z).length() - a.islands[i].1 - a.islands[j].1;
                assert!(gap <= max_gap(q.y - p.y) + 0.05, "seed {seed}: gap {gap}");
                assert!((p.y - q.y).abs() <= ISLAND_MAX_STEP + 0.05, "seed {seed}: step");
                adj[i].push(j);
                adj[j].push(i);
            }
            let mut seen = vec![false; a.islands.len()];
            let mut todo = vec![0];
            seen[0] = true;
            while let Some(i) = todo.pop() { for &j in &adj[i] { if !seen[j] { seen[j] = true; todo.push(j); } } }
            assert!(seen.iter().all(|&s| s), "seed {seed}: islands not all reachable");
            assert!(a.spawns.len() >= 4 && a.spawns.iter().all(|&s| a.spawn_safe(s) && a.on_island(glam::Vec2::new(s.x, s.z))), "seed {seed}");
            assert!(a.islands.iter().all(|&(t, r)| t.x.abs() + r <= a.size * 0.5 - 3.0 * LEVEL_SCALE + 1e-3 && t.z.abs() + r <= a.size * 0.5 - 3.0 * LEVEL_SCALE + 1e-3));
            assert_eq!(a.lava_surface(), LAVA_FLOOR_Y);
        }
        assert!((25..=50).contains(&islands), "{islands} of 200 arenas are islands (Godot 18%)");
        let forced = Arena::generate_with(3, 55.0, 95.0, true);
        assert_eq!(forced.variant, Variant::Island);
    }
}
