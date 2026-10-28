//! The light lava casts (Godot `lava_floor_openness.gd` and the leak glow in
//! `arena_procedural.gd`): an orange omni light under the floor whose energy
//! follows how open the floor is, and one over the leak as it floods the
//! arena. Both are unshadowed, as in Godot, so the pool light reaches the
//! walls and buildings through the floor.
//!
//! At night (not in Godot) local glows join them, fading in with the
//! floodlights: one over each patch of exposed lava in the floor (holes and
//! broken floor, brighter the more lava shows) and a ring past the arena's
//! edge that lights the stands and the outside of the arena.
use glam::{Vec2, Vec3};
use growing_guns::arena::LAVA_KILL_Y;
use growing_guns::shape::Kind;
use growing_guns::World;
use kern::collide::BoxGridScratch;
use render::headlights::LocalLight;

/// Godot `LavaFloorOpenness.CELL` and `RAY_TOP_Y`.
const CELL: f32 = 2.8;
const RAY_TOP_Y: f32 = 1.35;
/// Godot light_color (1.0, 0.42, 0.12), in linear.
const GLOW_COLOR: [f32; 3] = [1.0, 0.147, 0.0129];
/// Godot `omni_attenuation`.
const ATTENUATION: f32 = 0.65;
/// Candela per unit of Godot light energy (as the blast lights).
const ENERGY_CD: f32 = 37_000.0;
/// Pool glow: Godot `setup(..., max_energy 5.5, omni_range arena * 0.85)`.
const POOL_ENERGY: f32 = 5.5;
/// Leak glow: Godot `LavaLeakGlow`, up to 7.5.
const LEAK_ENERGY: f32 = 7.5;
/// Island arenas: Godot `_build_lava_floor_surface`'s LavaGlow, energy 8,
/// range 0.78 x the arena, 0.35 m over the lava floor.
const FLOOR_ENERGY: f32 = 8.0;
/// Night glows over exposed floor lava: at most this many, one per tile of
/// `TILE` cells with at least `PATCH_MIN_M2` of lava showing.
pub const PATCH_LIGHTS: usize = 4;
const TILE: i32 = 4;
const PATCH_MIN_M2: f32 = 4.0;
/// Candela per square metre of exposed lava (inverse-square falloff).
const PATCH_CD_PER_M2: f32 = 12_000.0;
/// Night glows over the pool past the arena's edge, around the ring between
/// the edge and the stands.
pub const RING_LIGHTS: usize = 8;
/// Lux at 1 m of each ring glow (falloff 1, as the floodlights).
const RING_LUX: f32 = 100_000.0;
/// Most lights the lava asks for in a frame.
pub const MAX_LAVA_LIGHTS: usize = 2 + PATCH_LIGHTS + RING_LIGHTS;

/// Godot `ArenaGenerator.get_lava_noise_texture`: the 256 px tiling noise
/// the lava shader reads, FastNoiseLite OpenSimplex2S ("simplex smooth")
/// FBm, frequency 0.07, five octaves, seed 90210, remapped to 0..1. Row
/// major (x across, y down), as the Godot image.
pub const NOISE_SIZE: u32 = 256;

pub fn noise_texture() -> Vec<f32> {
    use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};
    let mut fl = FastNoiseLite::with_seed(90210);
    fl.set_noise_type(Some(NoiseType::OpenSimplex2S));
    fl.set_fractal_type(Some(FractalType::FBm));
    fl.set_frequency(Some(0.07));
    fl.set_fractal_octaves(Some(5));
    let n = NOISE_SIZE as usize;
    (0..n * n).map(|k| fl.get_noise_2d((k % n) as f32, (k / n) as f32) * 0.5 + 0.5).collect()
}

#[derive(Default)]
pub struct LavaLight {
    /// First cell index on each axis and cells per side.
    min: i32,
    side: i32,
    /// Sealed fraction (0 open .. 1 covered) per cell, Godot `_cell_block`.
    block: Vec<f32>,
    /// Lava actually showing per cell, 0..1 (nothing over it at all; unlike
    /// Godot's ray this reads an intact floor as covered).
    exposed: Vec<f32>,
    ratio: f32,
    /// Night glows: (position, exposed area m²) per patch of floor lava.
    patches: Vec<(Vec3, f32)>,
    /// How much of the night glows to show, 0 (day, Godot) .. 1 (night).
    pub night: f32,
    lights: Vec<LocalLight>,
    query: BoxGridScratch,
    dirty: Vec<bool>,
    dirty_cells: Vec<usize>,
}

impl LavaLight {
    /// A new arena: measure every column.
    pub fn set(&mut self, world: &World) {
        let half = world.arena.size * 0.5;
        self.min = (-half / CELL).floor() as i32;
        self.side = (half / CELL).floor() as i32 - self.min + 1;
        self.block = vec![1.0; (self.side * self.side) as usize];
        self.exposed = vec![0.0; self.block.len()];
        self.dirty.resize(self.block.len(), false);
        self.dirty.fill(false);
        self.dirty_cells.clear();
        for i in 0..self.block.len() { self.block[i] = self.measure(world, i); self.exposed[i] = self.exposure(world, i); }
        self.update_ratio();
    }

    /// Chunks broke: measure the columns under them again.
    pub fn broken(&mut self, world: &World, removed: &[u32]) {
        for &k in &self.dirty_cells { self.dirty[k] = false; }
        self.dirty_cells.clear();
        if removed.is_empty() || self.side == 0 { return; }
        for &i in removed {
            let c = &world.arena.chunks[i as usize];
            // Both samples ignore geometry outside this vertical interval.
            if c.center.y + c.half.y < LAVA_KILL_Y + 0.18 || c.center.y - c.half.y > RAY_TOP_Y { continue; }
            let r = c.shape().reach();
            let (lo, hi) = (self.cell_of(Vec2::new(c.center.x - r, c.center.z - r)), self.cell_of(Vec2::new(c.center.x + r, c.center.z + r)));
            for z in lo.1.max(0)..=hi.1.min(self.side - 1) {
                for x in lo.0.max(0)..=hi.0.min(self.side - 1) {
                    let k = (z * self.side + x) as usize;
                    if !self.dirty[k] {
                        self.dirty[k] = true;
                        self.dirty_cells.push(k);
                    }
                }
            }
        }
        if self.dirty_cells.is_empty() { return; }
        for i in 0..self.dirty_cells.len() {
            let k = self.dirty_cells[i];
            self.block[k] = self.measure(world, k);
            self.exposed[k] = self.exposure(world, k);
        }
        self.update_ratio();
    }

    /// This frame's lights (pool, then leak; on islands the lava floor's).
    pub fn pose(&mut self, world: &World) {
        self.lights.clear();
        let size = world.arena.size;
        // Island arenas: one glow over the lava floor and no pool or leak
        // light (Godot sets up no openness there and hides the leak glow).
        let island = world.arena.variant == growing_guns::arena::Variant::Island;
        let floor_y = super::things::floor_lava_y(world);
        if island {
            self.lights.push(glow(Vec3::new(0.0, floor_y + 0.35, 0.0), FLOOR_ENERGY, size * 0.78));
        } else if self.ratio > 0.0 {
            let energy = POOL_ENERGY * self.ratio.powf(0.72);
            self.lights.push(glow(Vec3::new(0.0, LAVA_KILL_Y + 0.6, 0.0), energy, size * 0.85));
        }
        let l = &world.lava;
        if l.active && !l.full && (l.floor_t() > 0.0 || !l.has_walls) {
            let exposure = self.ratio.max((l.inset() / l.half.max(0.01)).clamp(0.0, 1.0) * 0.92);
            let spread = (l.floor_t() * 1.4).clamp(0.0, 1.0);
            let energy = LEAK_ENERGY * spread * exposure.powf(0.72);
            if energy > 0.0 { self.lights.push(glow(Vec3::new(0.0, l.surface() + 0.45, 0.0), energy, l.half * 1.35)); }
        }
        let night = self.night.clamp(0.0, 1.0);
        if night <= 0.0 { return; }
        let color = Vec3::from(GLOW_COLOR);
        // Over each patch: higher and wider the larger it is, so the floor
        // around a hole catches the light; below the leak once it covers them.
        // On islands the patches are open lava between the islands, over the
        // lava floor.
        let base = if island { floor_y } else { 0.0 };
        for &(p, area) in &self.patches {
            let r = (area / std::f32::consts::PI).sqrt();
            let y = base + (0.3 + 0.3 * r).min(2.0);
            self.lights.push(LocalLight {
                position: p + Vec3::Y * y, direction: Vec3::NEG_Y, intensity_cd: color * (PATCH_CD_PER_M2 * area * night),
                range_m: (4.0 * r + 10.0).clamp(6.0, 40.0), inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 1.0, falloff: 2.0,
            });
        }
        // The ring: halfway from the arena's edge to the stands, above the
        // floor when there are no walls (the floor's rim catches it), below
        // the floor otherwise, so it does not shine through the walls onto it.
        let a = &world.arena;
        let half = size * 0.5;
        let y = if a.variant == growing_guns::arena::Variant::Open { 0.5 } else { -1.0 };
        for i in 0..RING_LIGHTS {
            let t = std::f32::consts::TAU * i as f32 / RING_LIGHTS as f32;
            let (s, c) = t.sin_cos();
            let edge = half / s.abs().max(c.abs());
            let r = 0.5 * (edge + a.stadium.inner_r.max(edge + 4.0));
            self.lights.push(LocalLight {
                position: Vec3::new(c * r, y, s * r), direction: Vec3::NEG_Y, intensity_cd: color * (RING_LUX * night),
                range_m: (size * 0.6).clamp(20.0, 80.0), inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 1.0, falloff: 1.0,
            });
        }
    }

    pub fn lights(&self) -> &[LocalLight] { &self.lights }

    fn update_ratio(&mut self) {
        self.ratio = if self.block.is_empty() { 0.0 } else { self.block.iter().map(|b| 1.0 - b.clamp(0.0, 1.0)).sum::<f32>() / self.block.len() as f32 };
        // Patches: the exposed lava per tile of TILE x TILE cells, the
        // largest first, at the centroid of what shows.
        let tiles = (self.side + TILE - 1) / TILE;
        let mut sums = vec![(Vec2::ZERO, 0.0f32); (tiles * tiles).max(0) as usize];
        for (k, &e) in self.exposed.iter().enumerate() {
            if e <= 0.0 { continue; }
            let (x, z) = (k as i32 % self.side, k as i32 / self.side);
            let c = Vec2::new(((x + self.min) as f32 + 0.5) * CELL, ((z + self.min) as f32 + 0.5) * CELL);
            let t = &mut sums[((z / TILE) * tiles + x / TILE) as usize];
            t.0 += c * e;
            t.1 += e;
        }
        let mut patches: Vec<_> = sums.into_iter().filter(|t| t.1 * CELL * CELL >= PATCH_MIN_M2).map(|(c, w)| (c / w, w * CELL * CELL)).collect();
        patches.sort_by(|a, b| b.1.total_cmp(&a.1));
        patches.truncate(PATCH_LIGHTS);
        self.patches = patches.into_iter().map(|(c, area)| (Vec3::new(c.x, 0.0, c.y), area)).collect();
    }

    /// How much of a cell shows lava: of 2 x 2 points, those with nothing
    /// over them between the lava and RAY_TOP_Y.
    fn exposure(&mut self, world: &World, k: usize) -> f32 {
        let (x, z) = ((k as i32 % self.side) + self.min, (k as i32 / self.side) + self.min);
        let mut open = 0;
        for (u, v) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let p = Vec2::new((x as f32 + u) * CELL, (z as f32 + v) * CELL);
            if !world.arena.grid.query_with_scratch(p, p, &mut self.query).iter().any(|&j| world.arena.chunk(j).is_some_and(|c| covers(c, p))) { open += 1; }
        }
        open as f32 / 4.0
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) { ((p.x / CELL).floor() as i32 - self.min, (p.y / CELL).floor() as i32 - self.min) }

    /// Godot `_measure_column_block`: a ray from just above the lava up to
    /// 1.35 m through the cell centre. Nothing in the way: open (0); a floor
    /// chunk first: the fraction of the column below its bottom (a ray that
    /// starts inside a floor chunk hits at once, so an intact floor reads
    /// nearly open, as in Godot); anything else: sealed (1).
    fn measure(&mut self, world: &World, k: usize) -> f32 {
        let (x, z) = ((k as i32 % self.side) + self.min, (k as i32 / self.side) + self.min);
        let p = Vec2::new((x as f32 + 0.5) * CELL, (z as f32 + 0.5) * CELL);
        let from = LAVA_KILL_Y + 0.18;
        let mut first: Option<(f32, Kind)> = None;
        for &j in world.arena.grid.query_with_scratch(p, p, &mut self.query) {
            let Some(c) = world.arena.chunk(j) else { continue };
            let (bottom, top) = (c.center.y - c.half.y, c.center.y + c.half.y);
            if top < from || bottom > RAY_TOP_Y { continue; }
            let (s, co) = c.yaw.sin_cos();
            let d = p - Vec2::new(c.center.x, c.center.z);
            let local = Vec2::new(co * d.x - s * d.y, s * d.x + co * d.y);
            if local.x.abs() > c.half.x || local.y.abs() > c.half.z { continue; }
            let hit = bottom.max(from);
            if first.is_none_or(|(y, _)| hit < y) { first = Some((hit, c.kind)); }
        }
        match first {
            None => 0.0,
            Some((y, Kind::Floor)) => ((y - LAVA_KILL_Y) / (RAY_TOP_Y - LAVA_KILL_Y).max(0.05)).clamp(0.0, 1.0),
            Some(_) => 1.0,
        }
    }
}

/// A chunk standing over `p` anywhere between the lava and RAY_TOP_Y.
fn covers(c: &growing_guns::Chunk, p: Vec2) -> bool {
    let (bottom, top) = (c.center.y - c.half.y, c.center.y + c.half.y);
    if top < LAVA_KILL_Y + 0.18 || bottom > RAY_TOP_Y { return false; }
    let (s, co) = c.yaw.sin_cos();
    let d = p - Vec2::new(c.center.x, c.center.z);
    let local = Vec2::new(co * d.x - s * d.y, s * d.x + co * d.y);
    local.x.abs() <= c.half.x && local.y.abs() <= c.half.z
}

fn glow(position: Vec3, energy: f32, range: f32) -> LocalLight {
    LocalLight {
        position, direction: Vec3::NEG_Y, intensity_cd: Vec3::from(GLOW_COLOR) * (energy * ENERGY_CD),
        range_m: range.clamp(1.0, 200.0), inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.5, falloff: ATTENUATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use growing_guns::{Arena, Breakage, Carve, Chunk};
    use growing_guns::rng::Rng;
    use growing_guns::shape::BoxGrid;

    // Independent copies of the pre-batching sampling and update algorithms.
    // In particular these allocate queries and revisit overlapping cells.
    fn reference_measure(l: &LavaLight, world: &World, k: usize) -> f32 {
        let (x, z) = ((k as i32 % l.side) + l.min, (k as i32 / l.side) + l.min);
        let p = Vec2::new((x as f32 + 0.5) * CELL, (z as f32 + 0.5) * CELL);
        let from = LAVA_KILL_Y + 0.18;
        let mut first: Option<(f32, Kind)> = None;
        for j in world.arena.grid.query(p, p) {
            let Some(c) = world.arena.chunk(j) else { continue };
            let (bottom, top) = (c.center.y - c.half.y, c.center.y + c.half.y);
            if top < from || bottom > RAY_TOP_Y { continue; }
            let (s, co) = c.yaw.sin_cos();
            let d = p - Vec2::new(c.center.x, c.center.z);
            let local = Vec2::new(co * d.x - s * d.y, s * d.x + co * d.y);
            if local.x.abs() > c.half.x || local.y.abs() > c.half.z { continue; }
            let hit = bottom.max(from);
            if first.is_none_or(|(y, _)| hit < y) { first = Some((hit, c.kind)); }
        }
        match first {
            None => 0.0,
            Some((y, Kind::Floor)) => ((y - LAVA_KILL_Y) / (RAY_TOP_Y - LAVA_KILL_Y).max(0.05)).clamp(0.0, 1.0),
            Some(_) => 1.0,
        }
    }

    fn reference_exposure(l: &LavaLight, world: &World, k: usize) -> f32 {
        let (x, z) = ((k as i32 % l.side) + l.min, (k as i32 / l.side) + l.min);
        let mut open = 0;
        for (u, v) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let p = Vec2::new((x as f32 + u) * CELL, (z as f32 + v) * CELL);
            if !world.arena.grid.query(p, p).into_iter().any(|j| world.arena.chunk(j).is_some_and(|c| covers(c, p))) { open += 1; }
        }
        open as f32 / 4.0
    }

    fn reference_set(l: &mut LavaLight, world: &World) {
        let half = world.arena.size * 0.5;
        l.min = (-half / CELL).floor() as i32;
        l.side = (half / CELL).floor() as i32 - l.min + 1;
        l.block = vec![1.0; (l.side * l.side) as usize];
        l.exposed = vec![0.0; l.block.len()];
        for k in 0..l.block.len() { l.block[k] = reference_measure(l, world, k); l.exposed[k] = reference_exposure(l, world, k); }
        l.update_ratio();
    }

    fn reference_broken(l: &mut LavaLight, world: &World, removed: &[u32]) -> usize {
        if removed.is_empty() || l.side == 0 { return 0; }
        let mut measured = 0;
        for &i in removed {
            let c = &world.arena.chunks[i as usize];
            let r = c.shape().reach();
            let (lo, hi) = (l.cell_of(Vec2::new(c.center.x - r, c.center.z - r)), l.cell_of(Vec2::new(c.center.x + r, c.center.z + r)));
            for z in lo.1.max(0)..=hi.1.min(l.side - 1) {
                for x in lo.0.max(0)..=hi.0.min(l.side - 1) {
                    let k = (z * l.side + x) as usize;
                    l.block[k] = reference_measure(l, world, k);
                    l.exposed[k] = reference_exposure(l, world, k);
                    measured += 1;
                }
            }
        }
        l.update_ratio();
        measured
    }

    fn bits(v: &[f32]) -> Vec<u32> { v.iter().map(|x| x.to_bits()).collect() }

    fn light_bits(l: &LocalLight) -> Vec<u32> {
        bits(&[
            l.position.x, l.position.y, l.position.z, l.direction.x, l.direction.y, l.direction.z,
            l.intensity_cd.x, l.intensity_cd.y, l.intensity_cd.z, l.range_m, l.inner_cos, l.outer_cos,
            l.source_radius_m, l.falloff,
        ])
    }

    fn assert_same(actual: &mut LavaLight, expected: &mut LavaLight, world: &World) {
        assert_eq!(bits(&actual.block), bits(&expected.block), "column coverage");
        assert_eq!(bits(&actual.exposed), bits(&expected.exposed), "exposed lava");
        assert_eq!(actual.ratio.to_bits(), expected.ratio.to_bits(), "pool energy input");
        let patches = |l: &LavaLight| l.patches.iter().map(|(p, area)| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), area.to_bits()]).collect::<Vec<_>>();
        assert_eq!(patches(actual), patches(expected), "patch order and centroids");
        for night in [0.0, 0.4, 1.0] {
            actual.night = night;
            expected.night = night;
            actual.pose(world);
            expected.pose(world);
            assert_eq!(actual.lights.iter().map(light_bits).collect::<Vec<_>>(), expected.lights.iter().map(light_bits).collect::<Vec<_>>(), "posed lights");
        }
    }

    #[test]
    fn lava_batch_matches_reference_through_seeded_blasts_and_arena_resets() {
        let mut actual = LavaLight::default();
        let mut expected = LavaLight::default();
        let mut removed_total = 0;
        for seed in [1, 19, 482] {
            let mut world = World::new(seed, &[false, false]);
            let original = world.arena.clone();
            let mut rng = Rng::new(seed ^ 0x5678);
            actual.set(&world);
            reference_set(&mut expected, &world);
            assert_same(&mut actual, &mut expected, &world);
            for tick in 0..18 {
                if tick == 9 {
                    world.arena = original.clone();
                    actual.set(&world);
                    reference_set(&mut expected, &world);
                }
                let at = world.arena.chunks[rng.below(world.arena.chunks.len() as u32) as usize].center;
                let mut removed = world.arena.carve(&[
                    Carve { pos: at, radius: 7.0, damage: 900.0 },
                    Carve { pos: at + Vec3::X, radius: 5.0, damage: 900.0 },
                ]).removed;
                if tick % 4 == 0 { removed.extend_from_within(..); removed.reverse(); }
                removed_total += removed.len();
                actual.broken(&world, &removed);
                reference_broken(&mut expected, &world, &removed);
                assert_same(&mut actual, &mut expected, &world);
            }
            actual.broken(&world, &[]);
            assert_same(&mut actual, &mut expected, &world);
        }
        assert!(removed_total > 100, "exercise actual destruction");
    }

    #[test]
    fn lava_batch_skips_only_irrelevant_heights_and_deduplicates_rotated_cells() {
        let mut world = World::new(4, &[false, false]);
        world.arena = Arena::empty();
        world.arena.size = 12.0;
        for (i, (y, half_y)) in [(5.0, 0.5), (-5.0, 0.5), (RAY_TOP_Y, 0.0), (LAVA_KILL_Y + 0.18, 0.0), (-1.0, 1.5), (-1.0, 1.5)].into_iter().enumerate() {
            world.arena.chunks.push(Chunk {
                center: Vec3::new(0.25 * i as f32, y, 0.0), half: Vec3::new(3.0, half_y, 2.5), yaw: 0.3 * i as f32,
                kind: if i == 4 { Kind::Floor } else { Kind::Wall }, volume: i as u32,
                max_hp: 50.0, hp: 50.0, alive: true, seed: i as u32,
            });
        }
        // An outside-arena chunk exercises empty, clipped cell rectangles.
        let mut outside = world.arena.chunks[4];
        outside.center.x = 40.0;
        world.arena.chunks.push(outside);
        world.arena.grid = BoxGrid::new(world.arena.chunks.iter().map(Chunk::shape).collect(), 4.0);
        let mut actual = LavaLight::default();
        let mut expected = LavaLight::default();
        actual.set(&world);
        reference_set(&mut expected, &world);
        assert_same(&mut actual, &mut expected, &world);
        for removed in [&[0, 1][..], &[2][..], &[3][..], &[4, 4, 5, 4, 5, 6][..]] {
            world.arena.apply_breakage(&Breakage { removed: removed.to_vec(), ..Default::default() });
            actual.broken(&world, removed);
            let old_measured = reference_broken(&mut expected, &world, removed);
            assert_same(&mut actual, &mut expected, &world);
            if removed[0] == 0 { assert!(actual.dirty_cells.is_empty(), "high and low chunks cannot change lava samples"); }
            else { assert!(!actual.dirty_cells.is_empty(), "interval boundaries remain relevant"); }
            if removed.len() > 2 { assert!(actual.dirty_cells.len() * 2 < old_measured, "overlapping cells are measured once"); }
        }
        let capacities = (actual.dirty.capacity(), actual.dirty_cells.capacity());
        for _ in 0..8 {
            actual.broken(&world, &[4, 5, 4, 5]);
            assert_same(&mut actual, &mut expected, &world);
            assert_eq!((actual.dirty.capacity(), actual.dirty_cells.capacity()), capacities);
        }
        world.arena = Arena::empty();
        world.arena.size = 2.0;
        actual.set(&world);
        reference_set(&mut expected, &world);
        actual.broken(&world, &[]);
        assert_same(&mut actual, &mut expected, &world);
    }

    /// Texels of Godot's own texture, exported from Godot 4.5.2 with
    /// `get_lava_noise_texture().get_image()` (all 65536 matched exactly).
    #[test]
    fn noise_matches_godot() {
        let t = super::noise_texture();
        for (k, v) in [(0, 0.5), (1, 0.4295789), (255, 0.6571982), (256, 0.5086587), (4097, 0.6051136), (33000, 0.646677), (65535, 0.49807352)] {
            assert!((t[k] - v).abs() < 1e-6, "texel {k}: {} vs Godot {v}", t[k]);
        }
    }
}
