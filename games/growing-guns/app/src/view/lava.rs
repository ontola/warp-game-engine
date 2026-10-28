//! The light lava casts (Godot `lava_floor_openness.gd` and the leak glow in
//! `arena_procedural.gd`): an orange omni light under the floor whose energy
//! follows how open the floor is, and one over the leak as it floods the
//! arena. Both are unshadowed, as in Godot, so the pool light reaches the
//! walls and buildings through the floor.
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
    ratio: f32,
    lights: Vec<LocalLight>,
}

impl LavaLight {
    /// A new arena: measure every column.
    pub fn set(&mut self, world: &World) {
        let half = world.arena.size * 0.5;
        self.min = (-half / CELL).floor() as i32;
        self.side = (half / CELL).floor() as i32 - self.min + 1;
        self.block = vec![1.0; (self.side * self.side) as usize];
        for i in 0..self.block.len() { self.block[i] = self.measure(world, i); }
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
    }

    pub fn lights(&self) -> &[LocalLight] { &self.lights }

    fn update_ratio(&mut self) {
        self.ratio = if self.block.is_empty() { 0.0 } else { self.block.iter().map(|b| 1.0 - b.clamp(0.0, 1.0)).sum::<f32>() / self.block.len() as f32 };
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
