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
}

impl LavaLight {
    /// A new arena: measure every column.
    pub fn set(&mut self, world: &World) {
        let half = world.arena.size * 0.5;
        self.min = (-half / CELL).floor() as i32;
        self.side = (half / CELL).floor() as i32 - self.min + 1;
        self.block = vec![1.0; (self.side * self.side) as usize];
        self.exposed = vec![0.0; self.block.len()];
        for i in 0..self.block.len() { self.block[i] = self.measure(world, i); self.exposed[i] = self.exposure(world, i); }
        self.update_ratio();
    }

    /// Chunks broke: measure the columns under them again.
    pub fn broken(&mut self, world: &World, removed: &[u32]) {
        if removed.is_empty() || self.side == 0 { return; }
        for &i in removed {
            let c = &world.arena.chunks[i as usize];
            let r = c.shape().reach();
            let (lo, hi) = (self.cell_of(Vec2::new(c.center.x - r, c.center.z - r)), self.cell_of(Vec2::new(c.center.x + r, c.center.z + r)));
            for z in lo.1.max(0)..=hi.1.min(self.side - 1) {
                for x in lo.0.max(0)..=hi.0.min(self.side - 1) {
                    let k = (z * self.side + x) as usize;
                    self.block[k] = self.measure(world, k);
                    self.exposed[k] = self.exposure(world, k);
                }
            }
        }
        self.update_ratio();
    }

    /// This frame's lights (pool, then leak).
    pub fn pose(&mut self, world: &World) {
        self.lights.clear();
        let size = world.arena.size;
        if self.ratio > 0.0 {
            let energy = POOL_ENERGY * self.ratio.powf(0.72);
            self.lights.push(glow(Vec3::new(0.0, LAVA_KILL_Y + 0.6, 0.0), energy, size * 0.85));
        }
        let l = &world.lava;
        if l.active && (l.floor_t() > 0.0 || !l.has_walls) {
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
        for &(p, area) in &self.patches {
            let r = (area / std::f32::consts::PI).sqrt();
            let y = (0.3 + 0.3 * r).min(2.0);
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
    fn exposure(&self, world: &World, k: usize) -> f32 {
        let (x, z) = ((k as i32 % self.side) + self.min, (k as i32 / self.side) + self.min);
        let mut open = 0;
        for (u, v) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let p = Vec2::new((x as f32 + u) * CELL, (z as f32 + v) * CELL);
            if !world.arena.grid.query(p, p).into_iter().any(|j| world.arena.chunk(j).is_some_and(|c| covers(c, p))) { open += 1; }
        }
        open as f32 / 4.0
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) { ((p.x / CELL).floor() as i32 - self.min, (p.y / CELL).floor() as i32 - self.min) }

    /// Godot `_measure_column_block`: a ray from just above the lava up to
    /// 1.35 m through the cell centre. Nothing in the way: open (0); a floor
    /// chunk first: the fraction of the column below its bottom (a ray that
    /// starts inside a floor chunk hits at once, so an intact floor reads
    /// nearly open, as in Godot); anything else: sealed (1).
    fn measure(&self, world: &World, k: usize) -> f32 {
        let (x, z) = ((k as i32 % self.side) + self.min, (k as i32 / self.side) + self.min);
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
