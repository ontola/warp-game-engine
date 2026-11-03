//! Destruction, after Godot's `destructible_solid.gd` and
//! `destructible_manager.gd`. Every arena block is pre-split into uneven box
//! chunks (no runtime mesh rebuild); hits only lower chunk health and remove
//! chunks. Damage queued during a tick is applied once at its end, merged
//! per chunk, so a burst of explosions costs one sweep.
//!
//! The result of a tick is a [`Breakage`]: removed chunks, worn chunks and
//! debris bursts. The host applies it to its world and sends it to clients
//! reliably (it is the only thing that changes the arena after generation).
use crate::arena::Arena;
use crate::rng::Rng;
use crate::shape::{Block, BoxGrid, Kind, YawBox};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A hit must deal this much to cost a chunk health (weaker guns only mark it).
pub const MIN_DESTRUCTION_DAMAGE: f32 = 17.0;
/// A chunk breaks when its health runs out or one sweep's damage exceeds
/// what is left by this margin.
pub const MIN_REMOVE_DAMAGE: f32 = 12.0;
pub const CHUNK_BASE_HEALTH: f32 = 42.0;
pub const CHUNK_VOLUME_HEALTH: f32 = 4.0;
pub const TARGET_CELL: f32 = 2.4;
pub const MAX_CHUNKS_PER_AXIS: usize = 6;
pub const EDGE_JITTER: f32 = 0.38;
pub const TERRAIN_TILE_SIZE: f32 = 8.0;
pub const MAX_JAGGEDIZE_PER_CARVE: usize = 18;
/// Rifle chips: damage and radius from the bullet's damage ratio.
pub const RIFLE_CHIP_DAMAGE_BASE: f32 = 9.0;
pub const RIFLE_CHIP_DAMAGE_PER_RATIO: f32 = 7.0;
pub const RIFLE_CHIP_RADIUS_BASE: f32 = 0.7;
pub const RIFLE_CHIP_RADIUS_PER_RATIO: f32 = 0.045;
pub const RIFLE_CHIP_RADIUS_MAX: f32 = 2.2;
/// Largest chunk half diagonal, for broadphase padding.
const MAX_HALF_DIAG: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub center: Vec3,
    pub half: Vec3,
    pub yaw: f32,
    pub kind: Kind,
    /// Index of the arena block (volume) it was cut from.
    pub volume: u32,
    pub max_hp: f32,
    pub hp: f32,
    pub alive: bool,
    /// Per-brick variation seed for rendering.
    pub seed: u32,
}

impl Chunk {
    pub fn shape(&self) -> YawBox { YawBox { center: self.center, half: self.half, yaw: self.yaw } }
    pub fn half_diag(&self) -> f32 { self.half.length() }
    /// 0 intact, 1 about to break.
    pub fn wear(&self) -> f32 { (1.0 - self.hp / self.max_hp).clamp(0.0, 1.0) }
}

/// Damage queued against the arena this tick.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Carve { pub pos: Vec3, pub radius: f32, pub damage: f32 }

/// A cloud of rubble thrown from a blast centre (`count` chunks broke).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Burst { pub center: Vec3, pub radius: f32, pub count: u32, pub kind: Kind }

/// What one tick did to the arena.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Breakage {
    pub removed: Vec<u32>,
    /// Surviving chunks whose health changed, with the new health.
    pub worn: Vec<(u32, f32)>,
    pub bursts: Vec<Burst>,
}

impl Breakage {
    pub fn is_empty(&self) -> bool { self.removed.is_empty() && self.worn.is_empty() && self.bursts.is_empty() }
    pub fn extend(&mut self, o: Breakage) { self.removed.extend(o.removed); self.worn.extend(o.worn); self.bursts.extend(o.bursts); }
}

/// Godot's blast falloff: full at the centre, quadratic to zero at `radius`.
pub fn blast_damage_at(dist: f32, radius: f32, damage: f32) -> f32 {
    if radius <= 0.0 || damage <= 0.0 || dist >= radius { return 0.0; }
    let t = 1.0 - dist / radius;
    damage * t * t
}

/// Rifle impact on the arena: chip damage and radius for a bullet that
/// deals `ratio` times the base damage.
pub fn rifle_chip(ratio: f32) -> (f32, f32) {
    let ratio = ratio.max(0.2);
    (RIFLE_CHIP_DAMAGE_BASE + ratio * RIFLE_CHIP_DAMAGE_PER_RATIO,
     (RIFLE_CHIP_RADIUS_BASE + ratio * RIFLE_CHIP_RADIUS_PER_RATIO).clamp(RIFLE_CHIP_RADIUS_BASE, RIFLE_CHIP_RADIUS_MAX))
}

fn chunk_max_health(size: Vec3) -> f32 { CHUNK_BASE_HEALTH + (size.x * size.y * size.z).max(0.01) * CHUNK_VOLUME_HEALTH }

fn hash_block(b: &Block) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in [b.center.x, b.center.y, b.center.z, b.half.x, b.half.y, b.half.z, b.yaw] {
        h ^= v.to_bits() as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn axis_chunks(size: f32) -> usize { ((size / TARGET_CELL).ceil() as usize).clamp(1, MAX_CHUNKS_PER_AXIS) }

fn axis_edges(size: f32, count: usize, seed: u64) -> Vec<f32> {
    let mut e = vec![0.0; count + 1];
    e[0] = -size * 0.5;
    e[count] = size * 0.5;
    let step = size / count as f32;
    let mut rng = Rng::new(seed);
    for (i, edge) in e.iter_mut().enumerate().take(count).skip(1) {
        *edge = -size * 0.5 + i as f32 * step + rng.range(-EDGE_JITTER, EDGE_JITTER) * step;
    }
    e
}

/// Godot's `_add_destructible_region`: 8 m tiles, same rotation. Each tile
/// comes with per-axis edge seeds that depend only on the parent and the
/// tile's position along that axis, so courses line up across tiles (in
/// Godot every tile has its own seed and the running bond leaves gaps at
/// tile borders).
fn tiles(b: &Block) -> Vec<(Block, [u64; 3])> {
    let seed = hash_block(b);
    let axis_seeds = |t: [u32; 3]| [seed ^ 0xA341 ^ (t[0] as u64) << 32, seed ^ 0xC5F9 ^ (t[1] as u64) << 32, seed ^ 0x71D3 ^ (t[2] as u64) << 32];
    if !b.tiled { return vec![(*b, axis_seeds([0; 3]))]; }
    let size = b.half * 2.0;
    let counts = (size / TERRAIN_TILE_SIZE).ceil().max(Vec3::ONE);
    let cell = size / counts;
    let rot = Quat::from_rotation_y(b.yaw);
    let mut out = Vec::new();
    for z in 0..counts.z as u32 {
        for y in 0..counts.y as u32 {
            for x in 0..counts.x as u32 {
                let local = (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * cell - size * 0.5;
                let c = b.center + rot * local;
                out.push((Block { center: c, half: cell * 0.5, yaw: b.yaw, kind: b.kind, tiled: false }, axis_seeds([x, y, z])));
            }
        }
    }
    out
}

/// Split one volume into chunks (Godot `_build_chunks`): jittered edges per
/// axis and a running-bond shift on alternate courses.
fn split(b: &Block, seeds: [u64; 3], volume: u32, close_floor_start: bool, out: &mut Vec<Chunk>) {
    let size = b.half * 2.0;
    let seed = hash_block(b);
    let grid = [axis_chunks(size.x), axis_chunks(size.y), axis_chunks(size.z)];
    let ex = axis_edges(size.x, grid[0], seeds[0]);
    let ey = axis_edges(size.y, grid[1], seeds[1]);
    let ez = axis_edges(size.z, grid[2], seeds[2]);
    let rot = Quat::from_rotation_y(b.yaw);
    for z in 0..grid[2] {
        for y in 0..grid[1] {
            for x in 0..grid[0] {
                let lo = Vec3::new(ex[x], ey[y], ez[z]);
                let hi = Vec3::new(ex[x + 1], ey[y + 1], ez[z + 1]);
                let csize = hi - lo;
                let mut local = (lo + hi) * 0.5;
                let bond = (grid[0] > 1 && y % 2 == 1) || (grid[1] <= 1 && grid[2] > 1 && grid[0] > 1 && z % 2 == 1);
                if bond {
                    let shift = size.x / grid[0] as f32 * 0.5;
                    local.x += shift;
                    // Running bond shifts the entire course. Its first tile
                    // needs a closing stone, otherwise lava shows by the wall.
                    // Internal tile seams already have the preceding overhang.
                    if close_floor_start && x == 0 {
                        let cap_size = Vec3::new(shift, csize.y, csize.z);
                        let cap_pos = Vec3::new(-size.x * 0.5 + shift * 0.5, local.y, local.z);
                        let hp = chunk_max_health(cap_size);
                        out.push(Chunk { center: b.center + rot * cap_pos, half: cap_size * 0.5,
                            yaw: b.yaw, kind: b.kind, volume, max_hp: hp, hp, alive: true,
                            seed: (seed as u32) ^ 0xE0D6_E000 ^ ((y as u32) << 8) ^ z as u32 });
                    }
                }
                let hp = chunk_max_health(csize);
                let cs = (seed as u32) ^ ((x as u32) | (y as u32) << 8 | (z as u32) << 16).wrapping_mul(0x9E37_79B1);
                out.push(Chunk { center: b.center + rot * local, half: csize * 0.5, yaw: b.yaw, kind: b.kind, volume,
                    max_hp: hp, hp, alive: true, seed: cs });
            }
        }
    }
}

impl Arena {
    /// Cut every block into chunks and build the collision grid.
    pub fn build_chunks(&mut self) {
        let mut chunks = Vec::new();
        let mut volume = 0u32;
        for b in &self.blocks {
            for (t, seeds) in tiles(b) {
                let local = Quat::from_rotation_y(-b.yaw) * (t.center - b.center);
                let close_floor_start = b.kind == Kind::Floor && local.x - t.half.x <= -b.half.x + 0.001;
                split(&t, seeds, volume, close_floor_start, &mut chunks);
                volume += 1;
            }
        }
        self.stadium = crate::stadium::Stadium::new(self.size, self.variant != crate::arena::Variant::Open, crate::arena::WALL_HEIGHT);
        let mut boxes: Vec<YawBox> = chunks.iter().map(Chunk::shape).collect();
        boxes.extend(self.stadium.collision());
        self.grid = BoxGrid::new(boxes, 4.0);
        self.chunks = chunks;
    }

    pub fn alive_chunks(&self) -> usize { self.grid.live() - (self.grid.len() - self.chunks.len()) }

    /// The chunk a grid index refers to (None for stadium pieces).
    pub fn chunk(&self, grid_index: usize) -> Option<&Chunk> { self.chunks.get(grid_index) }

    /// Apply one tick's queued damage (Godot `apply_pending_carves`, all
    /// bodies at once): accumulate per chunk, one health sweep, remove what
    /// broke, weaken the rim of each hole and report rubble bursts.
    pub fn carve(&mut self, carves: &[Carve]) -> Breakage {
        let mut out = Breakage::default();
        let carves: Vec<&Carve> = carves.iter().filter(|c| c.damage >= MIN_DESTRUCTION_DAMAGE && c.radius > 0.0).collect();
        if carves.is_empty() { return out; }
        let mut accum: BTreeMap<u32, f32> = BTreeMap::new();
        // Per volume: the largest blast's centre and radius (the boom origin).
        let mut blast: BTreeMap<u32, (Vec3, f32)> = BTreeMap::new();
        for c in &carves {
            for i in self.grid.in_sphere(c.pos, c.radius + MAX_HALF_DIAG) {
                let Some(ch) = self.chunks.get(i) else { continue };
                let hd = ch.half_diag();
                let dist = ch.center.distance(c.pos);
                if dist > c.radius + hd { continue; }
                let d = blast_damage_at((dist - hd).max(0.0), c.radius, c.damage);
                *accum.entry(i as u32).or_default() += d;
                let e = blast.entry(ch.volume).or_insert((c.pos, c.radius));
                if c.radius > e.1 { *e = (c.pos, c.radius); }
            }
        }
        let mut removed_by_volume: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for (&i, &amount) in &accum {
            if amount <= 0.0 { continue; }
            let ch = &mut self.chunks[i as usize];
            ch.hp -= amount;
            if ch.hp <= 0.0 || amount >= ch.hp + MIN_REMOVE_DAMAGE {
                removed_by_volume.entry(ch.volume).or_default().push(i);
            } else {
                out.worn.push((i, ch.hp));
            }
        }
        for (volume, removed) in &removed_by_volume {
            let mut mean = Vec3::ZERO;
            for &i in removed {
                let ch = &mut self.chunks[i as usize];
                ch.alive = false;
                ch.hp = 0.0;
                mean += ch.center;
                self.grid.remove(i as usize);
                out.removed.push(i);
            }
            let kind = self.chunks[removed[0] as usize].kind;
            let (bc, br) = blast[volume];
            out.bursts.push(Burst { center: bc, radius: br, count: removed.len() as u32, kind });
            // Rim: chunks around the hole lose health by proximity so the
            // next blast cascades (Godot `_jaggedize_exposed_chunks`).
            mean /= removed.len() as f32;
            let reach = removed.iter().map(|&i| self.chunks[i as usize].center.distance(mean)).fold(0.0, f32::max) + TARGET_CELL;
            let mut n = 0;
            for i in self.grid.in_sphere(mean, reach + MAX_HALF_DIAG + TARGET_CELL * 0.75) {
                if n >= MAX_JAGGEDIZE_PER_CARVE { break; }
                let Some(ch) = self.chunks.get_mut(i) else { continue };
                if ch.volume != *volume { continue; }
                let dist = ch.center.distance(mean);
                if dist > reach + ch.half_diag() + TARGET_CELL * 0.75 { continue; }
                let halo = (1.0 - dist / reach.max(0.01)).clamp(0.0, 1.0);
                let halo_dmg = 0.22 + (0.82 - 0.22) * halo * halo;
                let capped = ch.hp.min(ch.max_hp * (1.0 - halo_dmg));
                if capped < ch.hp {
                    ch.hp = capped;
                    out.worn.retain(|w| w.0 != i as u32);
                    out.worn.push((i as u32, capped));
                }
                n += 1;
            }
        }
        out
    }

    /// Apply a host's breakage (clients).
    pub fn apply_breakage(&mut self, b: &Breakage) {
        for &i in &b.removed {
            if let Some(ch) = self.chunks.get_mut(i as usize) {
                ch.alive = false;
                ch.hp = 0.0;
                self.grid.remove(i as usize);
            }
        }
        for &(i, hp) in &b.worn {
            if let Some(ch) = self.chunks.get_mut(i as usize) { ch.hp = hp; }
        }
    }
}

/// The arena on the wire: generator output plus chunk state, without the
/// thousands of chunks (the receiver cuts them again; cutting is
/// deterministic).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArenaWire {
    seed: u64,
    size: f32,
    variant: crate::arena::Variant,
    blocks: Vec<Block>,
    spawns: Vec<Vec3>,
    lights: Vec<(Vec3, [f32; 3])>,
    hole: Option<(glam::Vec2, f32)>,
    /// Chunks that differ from fresh: (index, health); dead at health <= 0.
    wear: Vec<(u32, f32)>,
    #[serde(default)]
    islands: Vec<(Vec3, f32)>,
    #[serde(default)]
    links: Vec<(Vec3, Vec3)>,
}

impl From<Arena> for ArenaWire {
    fn from(a: Arena) -> Self {
        let wear = a.chunks.iter().enumerate().filter(|(_, c)| !c.alive || c.hp < c.max_hp)
            .map(|(i, c)| (i as u32, if c.alive { c.hp } else { 0.0 })).collect();
        ArenaWire { seed: a.seed, size: a.size, variant: a.variant, blocks: a.blocks, spawns: a.spawns, lights: a.lights, hole: a.hole, wear, islands: a.islands, links: a.links }
    }
}

impl From<ArenaWire> for Arena {
    fn from(w: ArenaWire) -> Self {
        let mut a = Arena { seed: w.seed, size: w.size, variant: w.variant, blocks: w.blocks, chunks: Vec::new(), grid: BoxGrid::default(),
            stadium: crate::stadium::Stadium::new(w.size, w.variant != crate::arena::Variant::Open, crate::arena::WALL_HEIGHT),
            spawns: w.spawns, lights: w.lights, hole: w.hole, islands: w.islands, links: w.links };
        a.build_chunks();
        let removed = w.wear.iter().filter(|x| x.1 <= 0.0).map(|x| x.0).collect();
        let worn = w.wear.iter().filter(|x| x.1 > 0.0).copied().collect();
        a.apply_breakage(&Breakage { removed, worn, bursts: Vec::new() });
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_tile_the_arena_deterministically() {
        // Seed 15: walled, 89 m (seeds 4, 5, 10 and 11 roll islands since
        // those are ported).
        let a = Arena::generate(15);
        let b = Arena::generate(15);
        assert_eq!(a.chunks, b.chunks);
        assert!(a.chunks.len() > 1000, "{}", a.chunks.len());
        assert!(a.chunks.iter().all(|c| c.half.max_element() * 2.0 < 4.5), "chunks stay brick sized");
        // Floor top surface is at y = 0.
        let top = a.chunks.iter().filter(|c| c.kind == Kind::Floor).map(|c| c.center.y + c.half.y).fold(f32::MIN, f32::max);
        assert!((top - 0.0).abs() < 1e-4, "{top}");
    }

    #[test]
    fn walls_and_floors_have_no_holes() {
        let solid = |a: &Arena, p: Vec3| a.grid.query(glam::Vec2::new(p.x, p.z), glam::Vec2::new(p.x, p.z)).into_iter()
            .any(|i| a.chunks[i].shape().push_out(p, 0.01, 0.01).is_some());
        for seed in [0, 15] {
            let a = Arena::generate(seed);
            let half = a.size * 0.5;
            let mut missing = Vec::new();
            for xi in 0..(a.size as i32 * 4 - 8) {
                for yi in 0..48 {
                    let p = Vec3::new(-half + 1.0 + xi as f32 * 0.25, 0.1 + yi as f32 * 0.25, -half);
                    if !solid(&a, p) { missing.push(p); }
                }
                for zi in 0..(a.size as i32 * 4 - 8) {
                    // Include the outer row ends that used to expose lava.
                    let p = Vec3::new(-half + 0.05 + xi as f32 * 0.25, -0.5, -half + 0.05 + zi as f32 * 0.25);
                    if p.x < half - 2.0 && p.z < half - 2.0 && !solid(&a, p) { missing.push(p); }
                }
            }
            assert!(missing.is_empty(), "seed {seed}: {} gaps, first {:?}", missing.len(), &missing[..missing.len().min(6)]);
        }
    }

    #[test]
    fn weak_hits_mark_strong_hits_break_and_rims_weaken() {
        let mut a = Arena::generate(15);
        let floor = a.chunks.iter().position(|c| c.kind == Kind::Floor && c.center.y > -1.0 && c.center.x.abs() < 10.0 && c.center.z.abs() < 10.0).unwrap();
        let p = a.chunks[floor].center + Vec3::Y * a.chunks[floor].half.y;
        // A weak round (a third of the base damage) only marks.
        let (dmg, r) = rifle_chip(0.2);
        assert!(dmg < MIN_DESTRUCTION_DAMAGE);
        assert!(a.carve(&[Carve { pos: p, radius: r, damage: dmg }]).is_empty());
        // Damage card x1.5: 19.5 per hit wears the chunk down.
        let (dmg, r) = rifle_chip(1.5);
        let mut shots = 0;
        while a.chunks[floor].alive {
            shots += 1;
            assert!(shots < 30, "never breaks");
            let b = a.carve(&[Carve { pos: p, radius: r, damage: dmg }]);
            assert!(!b.worn.is_empty() || !b.removed.is_empty());
        }
        assert!(shots > 2, "{shots}");
        assert!(!a.grid.is_alive(floor));
        // A big blast breaks many chunks at once and weakens the rim.
        let before = a.alive_chunks();
        let b = a.carve(&[Carve { pos: p, radius: 4.0, damage: 400.0 }]);
        assert!(b.removed.len() >= 4, "{:?}", b.removed.len());
        assert_eq!(a.alive_chunks(), before - b.removed.len());
        assert!(!b.bursts.is_empty());
        assert!(b.worn.iter().any(|&(i, hp)| hp < a.chunks[i as usize].max_hp * 0.8));
    }

    #[test]
    fn the_wire_form_carries_destruction() {
        let mut a = Arena::generate(9);
        let c = a.chunks[a.chunks.len() / 2].center;
        let b = a.carve(&[Carve { pos: c, radius: 3.0, damage: 300.0 }]);
        assert!(!b.removed.is_empty());
        let json = serde_json::to_string(&a).unwrap();
        let back: Arena = serde_json::from_str(&json).unwrap();
        assert_eq!(back.chunks, a.chunks);
        assert_eq!(back.alive_chunks(), a.alive_chunks());
        let mut fresh = Arena::generate(9);
        fresh.apply_breakage(&b);
        assert_eq!(fresh.chunks, a.chunks);
    }
}
