//! What the spells look like: the sim's one-shot events (fx::Ev) and the
//! live spells, shots and pickups as particles, glows, rings and lightning,
//! after the Godot game's src/fx (Fx, Parts, BoltFx, NovaFx...). Particles
//! run on the CPU like Godot's CPUParticles3D and are drawn as Warp sprites;
//! blasts use the engine's fire billows.
use crate::look::linear3;
use glam::Vec3;
use render::headlights::LocalLight;
use render::picture::Picture;
use render::sprites::{Facing, Sprite, SpriteImage};
use spellstorm::fx::{Death, Ev, C};
use spellstorm::{El, World};
use std::collections::HashMap;
use std::f32::consts::PI;

pub const IMAGE: SpriteImage = SpriteImage(0x5ee1_0001);
const SIDE: usize = 5;
const CELL: usize = 128;
/// Godot light energy to Warp (view.rs LIGHT).
pub(crate) const LIGHT: f32 = PI;
/// About how bright an unlit see-through thing shows in the dark levels
/// (smoke, drops, chips, puddles), as if the moon and lamps lit it.
pub(crate) const LIT: f32 = 0.3;

/// The particle shapes, one cell each in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    Dot,
    Flame,
    Drop,
    Spark,
    Flake,
    Streak,
    Puff,
    HaloOut,
    HaloIn,
    Ring,
    Line,
    Patch,
    Shock,
    Slice,
    Wisp,
    Band,
    Pillar,
    Runes,
}

const CELLS: [Cell; 18] = [
    Cell::Dot,
    Cell::Flame,
    Cell::Drop,
    Cell::Spark,
    Cell::Flake,
    Cell::Streak,
    Cell::Puff,
    Cell::HaloOut,
    Cell::HaloIn,
    Cell::Ring,
    Cell::Line,
    Cell::Patch,
    Cell::Shock,
    Cell::Slice,
    Cell::Wisp,
    Cell::Band,
    Cell::Pillar,
    Cell::Runes,
];

/// The width and height of a whole cell in the atlas, as `uv` gives it.
pub(crate) fn cell_size() -> f32 {
    1.0 / SIDE as f32 - 1.0 / (SIDE * CELL) as f32
}

pub(crate) fn uv(c: Cell) -> [f32; 4] {
    let i = c as usize;
    let s = 1.0 / SIDE as f32;
    let pad = 0.5 / (SIDE * CELL) as f32;
    [(i % SIDE) as f32 * s + pad, (i / SIDE) as f32 * s + pad, s - 2.0 * pad, s - 2.0 * pad]
}

/// The cell an atlas rectangle lies in (a crop of one, mirrored or not).
pub(crate) fn cell_at(r: [f32; 4]) -> Option<Cell> {
    let (x, y) = (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5);
    if !(0.0..1.0).contains(&x) || !(0.0..1.0).contains(&y) {
        return None;
    }
    let i = (y * SIDE as f32) as usize * SIDE + (x * SIDE as f32) as usize;
    CELLS.get(i).copied()
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One texel of a cell: brightness and alpha (Parts._texture, Fx._soft_dot,
/// halo.gdshader, the ring and zap shaders).
fn texel(c: Cell, u: f32, v: f32) -> (f32, f32) {
    let r = (u * u + v * v).sqrt();
    match c {
        Cell::Dot => {
            let a = if r < 0.35 { 1.0 - r / 0.35 * 0.25 } else { 0.75 * (1.0 - ((r - 0.35) / 0.65).min(1.0)) };
            (1.0, a)
        }
        Cell::Flame => {
            let w = 0.55 * (1.0 - ((-v + 0.2) / 1.2).clamp(0.0, 1.0)) + 0.15;
            let a = (1.0 - u.abs() / w).clamp(0.0, 1.0) * (1.0 - v).clamp(0.0, 1.0) * (1.0 - smooth(0.6, 1.0, r));
            (1.0, a.powf(0.8))
        }
        Cell::Drop => {
            let a = 1.0 - smooth(0.7, 0.82, r);
            let g = ((u + 0.3).powi(2) + (v + 0.3).powi(2)).sqrt();
            (0.8 + 0.2 * (1.0 - smooth(0.3, 0.75, r)) + 0.25 * (1.0 - smooth(0.05, 0.25, g)), a)
        }
        Cell::Spark => ((1.0), ((1.0 - smooth(0.0, 0.9, r)).powf(2.2) + (1.0 - smooth(0.0, 0.18, r))).clamp(0.0, 1.0)),
        Cell::Flake => {
            let d = u.abs() * 2.2 + v.abs();
            let a = ((1.0 - smooth(0.55, 0.7, d)) + 0.25 * (1.0 - smooth(0.0, 1.0, r)).powi(2)).clamp(0.0, 1.0);
            (0.8 + 0.2 * (1.0 - smooth(0.0, 0.5, d)), a)
        }
        Cell::Streak => (1.0, (1.0 - v.abs() * 7.0).max(0.0) * (1.0 - u.abs()).max(0.0).powf(1.5)),
        Cell::Puff => (1.0, (1.0 - smooth(0.0, 1.0, r)).powf(1.6)),
        // halo.gdshader's wide faint glow and its brighter middle, each
        // already times its alpha (blend_add).
        Cell::HaloOut => {
            let k = (1.0 - r).clamp(0.0, 1.0).powf(2.4);
            (1.0, k * k.min(1.0))
        }
        Cell::HaloIn => {
            let k = (1.0 - r).clamp(0.0, 1.0).powf(2.4);
            (1.0, (1.0 - r * 2.2).clamp(0.0, 1.0).powi(2) * k)
        }
        // A bright band at the rim with a faint fill inside (shockwave).
        Cell::Ring => {
            let band = (-((r - 0.86) / 0.07).powi(2)).exp();
            let fill = if r < 0.86 { 0.12 * r } else { 0.0 };
            (1.0, (band + fill) * (1.0 - smooth(0.96, 1.0, r)))
        }
        // Across a zap: a white-hot core in a soft edge; u runs across.
        Cell::Line => {
            let x = u.abs();
            let core = 1.0 - smooth(0.0, 0.3, x);
            let edge = (1.0 - x).max(0.0).powi(2);
            (1.0, (core + 0.6 * edge).min(1.0))
        }
        // A piece of a pillar of light (grave_beam.gdshader): bright down
        // the middle, soft at the sides, and a tent along it, so pieces
        // half their height apart add up to one even column.
        Cell::Pillar => {
            let x = u.abs();
            let core = (1.0 - x).max(0.0).powi(2);
            (1.0, core * (1.0 - v.abs()))
        }
        // Where a charging spell will land (cast_mark.gdshader): two circles
        // with rune marks between them; the mark turns as a whole.
        Cell::Runes => {
            let ang = v.atan2(u);
            let ring = 1.0 - smooth(0.0, 0.05, (r - 0.9).abs());
            let inner = (1.0 - smooth(0.0, 0.035, (r - 0.72).abs())) * 0.6;
            let turn = |n: f32| (ang * n / std::f32::consts::TAU).rem_euclid(1.0);
            // Soft edges: a hard step aliases into blocks at this cell size.
            let band = |x: f32, a: f32, b: f32, f: f32| smooth(a - f, a + f, x) * (1.0 - smooth(b - f, b + f, x));
            let marks = band(turn(12.0), 0.55, 0.95, 0.06) * band(r, 0.76, 0.86, 0.02) * (0.5 + 0.5 * band(turn(3.0), 0.5, 0.98, 0.03));
            (1.0, ((ring * 0.9 + inner + marks * 0.7) * (1.0 - smooth(0.95, 1.0, r))).min(1.0))
        }
        // A ragged patch for a field on the ground: solid, frayed rim.
        Cell::Patch => {
            let ang = v.atan2(u);
            let fray = 0.06 * (ang * 5.0).sin() + 0.04 * (ang * 11.0 + 1.3).sin() + 0.03 * (ang * 23.0 + 0.4).sin();
            let a = 1.0 - smooth(0.72, 0.98, r + fray);
            (1.0 - 0.15 * r, a)
        }
        // One slice of the sword's arc on the ground (slash.gdshader), its
        // circle's middle at the cell's: brightest at the rim, a faint
        // band at the inner edge. Up the cell (v < 0) is the slice's middle.
        Cell::Slice => {
            // A tent across the slice, twice its width: side by side the
            // slices add up to one even arc, with no seams between them.
            let ang = u.atan2(-v).abs();
            let tent = (1.0 - ang / SLICE).clamp(0.0, 1.0);
            let y = ((r - SLICE_IN) / (1.0 - SLICE_IN)).clamp(0.0, 1.0);
            let ring = smooth(0.0, 0.02, r - SLICE_IN) * (1.0 - smooth(0.97, 1.0, r));
            let rim = smooth(0.75, 1.0, y) + 0.35 * (1.0 - smooth(0.0, 0.25, y));
            let body = 0.25 + 0.75 * y * y;
            (1.0, ((body * 0.55 + rim * 0.8) * ring).min(1.0) * tent)
        }
        // One piece of a ring to stand in (Run._fill_ring, slash.gdshader
        // round a whole circle): the cell holds only the piece near the rim,
        // BAND_AT out from the circle's middle along up (v < 0), BAND_SIZE
        // across, both in radii. A tent across it, so pieces side by side
        // add up to one even ring.
        Cell::Band => {
            let (px, py) = (u * BAND_SIZE * 0.5, BAND_AT - v * BAND_SIZE * 0.5);
            let r = (px * px + py * py).sqrt();
            let tent = (1.0 - px.atan2(py).abs() / BAND).clamp(0.0, 1.0);
            let y = ((r - BAND_IN) / (1.0 - BAND_IN)).clamp(0.0, 1.0);
            let ring = smooth(0.0, 0.02, r - BAND_IN) * (1.0 - smooth(0.97, 1.0, r));
            let rim = smooth(0.75, 1.0, y) + 0.35 * (1.0 - smooth(0.0, 0.25, y));
            let body = 0.25 + 0.75 * y * y;
            (1.0, ((body * 0.55 + rim * 0.8) * ring).min(1.0) * tent)
        }
        // A curl of haze for the mist walls: a lumpy soft blob.
        Cell::Wisp => {
            let ang = v.atan2(u);
            let lump = 0.12 * (ang * 3.0 + 0.7).sin() + 0.08 * (ang * 5.0 + 2.1).sin() + 0.05 * (ang * 9.0).sin();
            let a = (1.0 - smooth(0.1, 0.95, r + lump * r)).powf(1.4) * (1.0 - smooth(0.7, 0.98, r));
            (0.85 + 0.15 * (1.0 - r), a)
        }
        // A nova's shock: a hot wide front.
        Cell::Shock => {
            let band = (-((r - 0.8) / 0.14).powi(2)).exp();
            (1.0, band * (1.0 - smooth(0.95, 1.0, r)))
        }
    }
}

/// The sword arc's slices: how far apart they stand (each spreads twice that), and where the ring starts
/// (inner over outer radius of Wizard._arc_mesh).
pub(crate) const SLICE: f32 = 2.0 * spellstorm::wizard::SWING_ARC / SLICES as f32;
pub(crate) const SLICES: usize = 24;
pub(crate) const SLICE_IN: f32 = 0.55 / (spellstorm::wizard::SWING_REACH + 0.3);

/// The pieces of a ring to stand in: how many round it, how far apart
/// they stand (each spreads twice that), where the band starts (inner over
/// outer radius, Run._fill_ring's r - 0.18 at ALTAR_R), and the square each
/// piece's cell covers: its middle out from the circle's and its side.
pub(crate) const BANDS: usize = 32;
pub(crate) const BAND: f32 = std::f32::consts::TAU / BANDS as f32;
pub(crate) const BAND_IN: f32 = 1.0 - 0.18 / spellstorm::run::ALTAR_R;
pub(crate) const BAND_AT: f32 = 0.5 + 0.5 * BAND_IN * 0.98;
pub(crate) const BAND_SIZE: f32 = 0.42;

/// The atlas of every cell, to give the renderer once.
pub fn atlas() -> Picture {
    let n = SIDE * CELL;
    let mut rgba = vec![0u8; n * n * 4];
    for (i, &c) in CELLS.iter().enumerate() {
        let (ox, oy) = ((i % SIDE) * CELL, (i / SIDE) * CELL);
        for y in 0..CELL {
            for x in 0..CELL {
                let u = (x as f32 + 0.5) / CELL as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / CELL as f32 * 2.0 - 1.0;
                let (lum, a) = texel(c, u, v);
                let o = ((oy + y) * n + ox + x) * 4;
                let l = (lum.clamp(0.0, 1.0) * 255.0) as u8;
                rgba[o..o + 4].copy_from_slice(&[l, l, l, (a.clamp(0.0, 1.0) * 255.0) as u8]);
            }
        }
    }
    Picture { width: n as u32, height: n as u32, rgba }
}

/// A colour gradient over a particle's life (Godot Gradient, sRGB).
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    n: usize,
    t: [f32; 5],
    c: [C; 5],
}

impl Ramp {
    fn new(cs: &[C], ts: &[f32]) -> Ramp {
        let mut r = Ramp { n: cs.len().min(5), t: [0.0; 5], c: [[0.0; 4]; 5] };
        for i in 0..r.n {
            r.t[i] = ts[i];
            r.c[i] = cs[i];
        }
        r
    }

    fn at(&self, x: f32) -> C {
        if x <= self.t[0] {
            return self.c[0];
        }
        for i in 1..self.n {
            if x <= self.t[i] {
                let k = (x - self.t[i - 1]) / (self.t[i] - self.t[i - 1]).max(1e-6);
                return [0, 1, 2, 3].map(|j| self.c[i - 1][j] + (self.c[i][j] - self.c[i - 1][j]) * k);
            }
        }
        self.c[self.n - 1]
    }
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> C {
    [r, g, b, a]
}

fn lerp_c(a: C, b: C, t: f32) -> C {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

const WHITE: C = [1.0, 1.0, 1.0, 1.0];

fn el_c(e: El) -> C {
    let c = e.color();
    [c[0], c[1], c[2], 1.0]
}

/// Fx.ramp: fades in, holds, fades out darker.
fn fx_ramp(c: C) -> Ramp {
    Ramp::new(
        &[rgba(c[0], c[1], c[2], 0.0), rgba(c[0], c[1], c[2], 0.9), rgba(c[0], c[1], c[2], 0.5), rgba(c[0] * 0.5, c[1] * 0.5, c[2] * 0.5, 0.0)],
        &[0.0, 0.12, 0.6, 1.0],
    )
}

/// A CPUParticles3D's settings.
#[derive(Clone, Copy, Debug)]
pub struct Em {
    pub amount: u32,
    pub life: f32,
    pub size: f32,
    pub cell: Cell,
    /// Radiance per unit of colour; 0 for a see-through, unlit look.
    pub glow: f32,
    pub ramp: Ramp,
    pub tint: (Vec3, Vec3),
    pub dir: Vec3,
    pub spread: f32,
    pub flat: f32,
    pub v: (f32, f32),
    pub gravity: Vec3,
    pub damp: (f32, f32),
    pub radial: (f32, f32),
    pub sphere: f32,
    pub scale: (f32, f32),
    pub curve: [f32; 3],
    pub angle: (f32, f32),
    pub spin: (f32, f32),
    /// Lit chips: drawn darker, not glowing.
    pub solid: bool,
}

/// Fx.particles: soft additive dots in `c`.
pub fn particles(c: C, amount: u32, life: f32, size: f32) -> Em {
    Em {
        amount,
        life,
        size: size * 0.8,
        cell: Cell::Dot,
        glow: 1.4,
        ramp: fx_ramp(c),
        tint: (Vec3::ONE, Vec3::ONE),
        dir: Vec3::X,
        spread: 45.0,
        flat: 0.0,
        v: (0.0, 0.0),
        gravity: Vec3::new(0.0, -9.8, 0.0),
        damp: (0.0, 0.0),
        radial: (0.0, 0.0),
        sphere: 0.0,
        scale: (1.0, 1.0),
        curve: [1.0, 1.0, 1.0],
        angle: (0.0, 0.0),
        spin: (0.0, 0.0),
        solid: false,
    }
}

/// Parts.style: element `e`'s own particles. Returns the smoke that curls
/// off fire too, when `smoke`.
pub fn style(p: &mut Em, e: El, smoke: bool) -> Option<Em> {
    let c = el_c(e);
    let size = p.size;
    let mut out = None;
    let smoke_of = |p: &Em| {
        let mut s = *p;
        s.amount = (p.amount / 3).max(4);
        s.life = p.life * 2.0;
        s.size = size * 1.3;
        s.cell = Cell::Puff;
        s.glow = 0.0;
        s.ramp = Ramp::new(&[rgba(0.2, 0.18, 0.17, 0.0), rgba(0.22, 0.2, 0.19, 0.35), rgba(0.3, 0.3, 0.3, 0.0)], &[0.0, 0.25, 1.0]);
        s.tint = (Vec3::ONE, Vec3::ONE);
        s.v = (p.v.0 * 0.5, p.v.1 * 0.5);
        s.gravity = Vec3::Y * 1.6;
        s.radial = (0.0, 0.0);
        s.curve = [0.5, 1.2, 1.6];
        s.angle = (-180.0, 180.0);
        s.spin = (0.0, 0.0);
        s
    };
    match e {
        El::Fire => {
            p.size = size * 0.9;
            p.cell = Cell::Flame;
            p.glow = 1.6;
            p.ramp = Ramp::new(
                &[rgba(1.0, 0.95, 0.6, 0.0), rgba(1.0, 0.85, 0.35, 1.0), rgba(1.0, 0.45, 0.08, 0.85), rgba(0.85, 0.12, 0.03, 0.5), rgba(0.25, 0.05, 0.02, 0.0)],
                &[0.0, 0.1, 0.35, 0.7, 1.0],
            );
            p.tint = (Vec3::ONE, Vec3::new(1.0, 0.75, 0.6));
            p.gravity = Vec3::Y * 3.0;
            p.curve = [1.0, 0.8, 0.2];
            p.angle = (-15.0, 15.0);
            if smoke {
                out = Some(smoke_of(p));
            }
        }
        El::Magma => {
            p.size = size * 0.4;
            p.cell = Cell::Drop;
            p.glow = 1.6;
            p.ramp = Ramp::new(&[rgba(1.0, 0.9, 0.5, 1.0), rgba(1.0, 0.4, 0.05, 1.0), rgba(0.5, 0.08, 0.02, 0.0)], &[0.0, 0.5, 1.0]);
            p.gravity = Vec3::NEG_Y * 6.0;
            p.curve = [1.0, 0.85, 0.7];
            if smoke {
                out = Some(smoke_of(p));
            }
        }
        El::Water | El::Poison => {
            p.size = size * 0.3;
            p.cell = Cell::Drop;
            p.glow = 0.0;
            let light = lerp_c(c, WHITE, 0.55);
            p.ramp = Ramp::new(&[rgba(light[0], light[1], light[2], 0.95), rgba(c[0], c[1], c[2], 0.9), rgba(c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, 0.0)], &[0.0, 0.6, 1.0]);
            p.tint = (Vec3::ONE, Vec3::new(0.75, 0.8, 0.75));
            p.gravity = Vec3::NEG_Y * 12.0;
            p.curve = [0.8, 1.0, 0.6];
        }
        El::Lightning | El::Storm => {
            p.size = size * 0.4;
            p.cell = Cell::Spark;
            p.glow = 1.6;
            p.amount = (p.amount as f32 * 1.5) as u32;
            let hot = lerp_c(c, WHITE, 0.7);
            p.ramp = Ramp::new(&[WHITE, rgba(hot[0], hot[1], hot[2], 1.0), rgba(c[0], c[1], c[2], 0.0)], &[0.0, 0.4, 1.0]);
            p.gravity = Vec3::NEG_Y * 4.0;
            p.life = p.life.min(0.3);
            p.v = (p.v.0 * 1.6, p.v.1 * 2.2);
            p.spread = p.spread.max(70.0);
            p.damp = (4.0, 8.0);
            p.curve = [1.0, 0.7, 0.4];
            p.angle = (-180.0, 180.0);
        }
        El::Frost | El::Ice => {
            p.size = size * 0.26;
            p.cell = Cell::Flake;
            p.glow = 1.6;
            p.amount = (p.amount as f32 * 1.4) as u32;
            let pale = lerp_c(c, WHITE, 0.6);
            p.ramp = Ramp::new(&[rgba(1.0, 1.0, 1.0, 0.0), WHITE, rgba(pale[0], pale[1], pale[2], 0.8), rgba(c[0], c[1], c[2], 0.0)], &[0.0, 0.1, 0.5, 1.0]);
            p.gravity = Vec3::NEG_Y * 1.5;
            p.angle = (-180.0, 180.0);
            p.spin = (-360.0, 360.0);
        }
        El::Earth | El::Mud => {
            p.size = size * 0.3;
            p.cell = Cell::Drop;
            p.glow = 0.0;
            p.solid = true;
            let d = [c[0] * 0.85, c[1] * 0.85, c[2] * 0.85, 1.0];
            p.ramp = Ramp::new(&[d, d], &[0.0, 1.0]);
            p.gravity = Vec3::NEG_Y * 14.0;
            p.curve = [1.0, 1.0, 0.3];
        }
        El::Void => {
            p.size = size * 0.5;
            p.cell = Cell::Drop;
            p.glow = 1.6;
            let deep = [c[0] * 0.7, c[1] * 0.7, c[2] * 0.7, 0.7];
            p.ramp = Ramp::new(&[rgba(c[0], c[1], c[2], 0.0), rgba(1.0, 0.6, 1.0, 1.0), deep, rgba(0.1, 0.0, 0.15, 0.0)], &[0.0, 0.15, 0.6, 1.0]);
            p.gravity = Vec3::ZERO;
            p.radial = (-8.0, -4.0);
            p.curve = [1.0, 0.6, 0.1];
        }
        El::Air => {
            p.size = size * 0.7;
            p.cell = Cell::Streak;
            p.glow = 1.6;
            p.ramp = Ramp::new(&[rgba(1.0, 1.0, 1.0, 0.0), rgba(0.9, 1.0, 0.95, 0.45), rgba(c[0], c[1], c[2], 0.0)], &[0.0, 0.3, 1.0]);
            p.gravity = Vec3::ZERO;
            p.angle = (-180.0, 180.0);
        }
        El::Steam => {}
    }
    out
}

struct Part {
    pos: Vec3,
    vel: Vec3,
    home: Vec3,
    age: f32,
    life: f32,
    size: f32,
    angle: f32,
    spin: f32,
    damp: f32,
    radial: f32,
    tint: Vec3,
    em: u16,
}

/// A flat ring growing over the floor (Fx._wave), or a nova's shock.
struct Wave {
    at: Vec3,
    c: C,
    r: f32,
    age: f32,
    time: f32,
    cell: Cell,
    glow: f32,
}

/// A camera-facing glow that shrinks away (Fx.flash), or one that follows
/// nothing and fades (a spell's halo is drawn live instead).
struct Glow {
    at: Vec3,
    c: C,
    size: f32,
    age: f32,
    time: f32,
}

/// A light that fades out (Fx.light_pop).
struct Pop {
    at: Vec3,
    c: [f32; 3],
    energy: f32,
    range: f32,
    age: f32,
    time: f32,
}

/// A jagged line of lightning, gone in a blink.
struct Bolt {
    pts: Vec<Vec3>,
    c: C,
    width: f32,
    age: f32,
    time: f32,
}

/// A monster's lob in the air.
struct Lob {
    from: Vec3,
    to: Vec3,
    el: El,
    age: f32,
    time: f32,
}

/// Continuous emitters: spawn debt per live thing, kept between frames.
#[derive(Default)]
struct Debt(HashMap<(u64, u8), f32>);

pub struct Fx {
    ems: Vec<Em>,
    em_index: HashMap<u64, u16>,
    parts: Vec<Part>,
    waves: Vec<Wave>,
    glows: Vec<Glow>,
    pops: Vec<Pop>,
    zaps: Vec<Bolt>,
    lobs: Vec<Lob>,
    debt: Debt,
    rng: u64,
    pub shake: f32,
    pub blasts: render::fx::Effects,
    /// Sprites of live things, made in `update`.
    live: Vec<Sprite>,
    live_lights: Vec<LocalLight>,
    pub time: f32,
}

const MAX_PARTS: usize = 3000;

fn em_key(e: &Em) -> u64 {
    // Settings that make two emitters the same look.
    let mut h: u64 = 1469598103934665603;
    let mut f = |x: f32| {
        h ^= x.to_bits() as u64;
        h = h.wrapping_mul(1099511628211);
    };
    f(e.life);
    f(e.size);
    f(e.cell as u32 as f32);
    f(e.glow);
    for i in 0..e.ramp.n {
        f(e.ramp.t[i]);
        e.ramp.c[i].iter().for_each(|&x| f(x));
    }
    e.tint.0.to_array().iter().chain(e.tint.1.to_array().iter()).for_each(|&x| f(x));
    e.gravity.to_array().iter().for_each(|&x| f(x));
    e.curve.iter().for_each(|&x| f(x));
    f(if e.solid { 1.0 } else { 0.0 });
    h
}

impl Default for Fx {
    fn default() -> Self {
        let mut blasts = render::fx::Effects::default();
        blasts.light_scale = LIGHT;
        Fx {
            ems: vec![],
            em_index: HashMap::new(),
            parts: vec![],
            waves: vec![],
            glows: vec![],
            pops: vec![],
            zaps: vec![],
            lobs: vec![],
            debt: Debt::default(),
            rng: 0x9e37_79b9_7f4a_7c15,
            shake: 0.0,
            blasts,
            live: vec![],
            live_lights: vec![],
            time: 0.0,
        }
    }
}

impl Fx {
    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }

    fn em(&mut self, e: Em) -> u16 {
        let k = em_key(&e);
        if let Some(&i) = self.em_index.get(&k) {
            return i;
        }
        let i = self.ems.len() as u16;
        self.ems.push(e);
        self.em_index.insert(k, i);
        i
    }

    /// CPUParticles3D's spread round its direction.
    fn direction(&mut self, e: &Em) -> Vec3 {
        let s = e.spread.to_radians();
        let a1 = (self.f() * 2.0 - 1.0) * s;
        let a2 = (self.f() * 2.0 - 1.0) * s * (1.0 - e.flat);
        let xz = Vec3::new(a1.sin(), 0.0, a1.cos());
        let mut yz = Vec3::new(0.0, a2.sin(), a2.cos());
        yz.z /= yz.z.abs().sqrt().max(0.0001);
        let d = Vec3::new(xz.x * yz.z, yz.y, xz.z * yz.z);
        let n = e.dir.normalize_or(Vec3::Y);
        let mut b = Vec3::Y.cross(n);
        if b.length_squared() < 1e-8 {
            b = Vec3::Z;
        }
        let b = b.normalize();
        let nn = b.cross(n);
        (b * d.x + nn * d.y + n * d.z).normalize_or_zero()
    }

    /// `n` particles of `e` at `at`, the first `age0` seconds into life.
    pub fn emit(&mut self, e: Em, at: Vec3, n: u32) {
        let idx = self.em(e);
        for _ in 0..n {
            if self.parts.len() >= MAX_PARTS {
                self.parts.swap_remove(0);
            }
            let mut pos = at;
            if e.sphere > 0.0 {
                loop {
                    let p = Vec3::new(self.f() * 2.0 - 1.0, self.f() * 2.0 - 1.0, self.f() * 2.0 - 1.0);
                    if p.length_squared() <= 1.0 {
                        pos += p * e.sphere;
                        break;
                    }
                }
            }
            let dir = self.direction(&e);
            let speed = self.range(e.v.0, e.v.1);
            let k = self.f();
            let p = Part {
                pos,
                vel: dir * speed,
                home: at,
                age: 0.0,
                life: e.life.max(0.01),
                size: e.size * self.range(e.scale.0, e.scale.1),
                angle: self.range(e.angle.0, e.angle.1).to_radians(),
                spin: self.range(e.spin.0, e.spin.1).to_radians(),
                damp: self.range(e.damp.0, e.damp.1),
                radial: self.range(e.radial.0, e.radial.1),
                tint: e.tint.0.lerp(e.tint.1, k),
                em: idx,
            };
            self.parts.push(p);
        }
    }

    /// A one-shot of `e` with its element look and smoke.
    fn shot(&mut self, mut e: Em, at: Vec3, el: Option<El>, smoke: bool) {
        let s = el.and_then(|x| style(&mut e, x, smoke));
        self.emit(e, at, e.amount);
        if let Some(s) = s {
            self.emit(s, at, s.amount);
        }
    }

    /// A stream of `e` from a live thing: `amount` per `life`, like a
    /// CPUParticles3D left emitting.
    fn stream(&mut self, key: (u64, u8), e: Em, at: Vec3, dt: f32) {
        let rate = e.amount as f32 / e.life.max(0.05);
        let d = self.debt.0.entry(key).or_insert(0.0);
        *d += rate * dt;
        let n = *d as u32;
        *d -= n as f32;
        if n > 0 {
            self.emit(e, at, n);
        }
    }

    /// Fx.burst.
    pub fn burst(&mut self, at: Vec3, c: C, n: u32, speed: f32, size: f32, el: Option<El>) {
        let mut p = particles(c, n, 0.6, size);
        p.dir = Vec3::Y;
        p.spread = 180.0;
        p.v = (speed * 0.4, speed);
        p.gravity = Vec3::new(0.0, -6.0, 0.0);
        p.damp = (2.0, 4.0);
        self.shot(p, at, el, true);
    }

    /// Fx.puff.
    pub fn puff(&mut self, at: Vec3, c: C, size: f32) {
        let mut p = particles(c, 14, 1.4, size);
        p.size = size * 1.2;
        p.cell = Cell::Puff;
        p.glow = 0.0;
        p.ramp = Ramp::new(
            &[rgba(c[0], c[1], c[2], 0.0), rgba(c[0], c[1], c[2], 0.35), rgba(c[0] * 0.9, c[1] * 0.9, c[2] * 0.9, 0.2), rgba(c[0] * 0.8, c[1] * 0.8, c[2] * 0.8, 0.0)],
            &[0.0, 0.15, 0.55, 1.0],
        );
        p.dir = Vec3::Y;
        p.spread = 70.0;
        p.v = (0.6, 1.8);
        p.gravity = Vec3::Y * 0.6;
        p.scale = (0.6, 1.4);
        self.emit(p, at, 14);
    }

    pub fn flash(&mut self, at: Vec3, c: C, size: f32, time: f32) {
        self.glows.push(Glow { at, c, size, age: 0.0, time });
    }

    pub fn light_pop(&mut self, at: Vec3, c: C, energy: f32, range: f32, time: f32) {
        self.pops.push(Pop { at: at + Vec3::Y, c: [c[0], c[1], c[2]], energy, range, age: 0.0, time });
        self.flash(at + Vec3::Y * 0.8, c, range * 0.3 * (energy / 3.0).clamp(0.5, 1.2), time * 0.8);
    }

    pub fn ring(&mut self, at: Vec3, c: C, r: f32, time: f32) {
        self.waves.push(Wave { at, c, r, age: 0.0, time, cell: Cell::Ring, glow: 1.6 });
    }

    /// A jagged line from `a` to `b` (BoltFx._zap, _spark_line).
    fn zap(&mut self, a: Vec3, b: Vec3, c: C, width: f32, jitter: f32, step: f32, time: f32) {
        let steps = ((a.distance(b) / step) as usize).max(3);
        let mut pts = vec![a];
        for s in 1..=steps {
            let mut p = a.lerp(b, s as f32 / steps as f32);
            if s < steps {
                p += Vec3::new(self.f() - 0.5, self.f() - 0.5, self.f() - 0.5) * jitter;
            }
            pts.push(p);
        }
        self.zaps.push(Bolt { pts, c, width, age: 0.0, time });
    }

    /// The sim's events of the last tick.
    pub fn absorb(&mut self, w: &World) {
        for e in &w.events {
            self.event(w, e);
        }
    }

    fn event(&mut self, w: &World, e: &Ev) {
        match *e {
            Ev::Burst { at, c, n, speed, size, el } => self.burst(at, c, n, speed, size, el),
            Ev::Puff { at, c, size } => self.puff(at, c, size),
            Ev::Ring { at, c, r, time } => self.ring(at, c, r, time),
            Ev::Ripple { at, r, time } => self.waves.push(Wave { at, c: rgba(0.75, 0.85, 1.0, 0.35), r, age: 0.0, time, cell: Cell::Ring, glow: 0.0 }),
            Ev::LightPop { at, c, energy, range, time } => self.light_pop(at, c, energy, range, time),
            Ev::Flash { at, c, size, time } => self.flash(at, c, size, time),
            // No text over characters (Joep).
            Ev::Text { .. } => {}
            Ev::Fountain { at, el, n, speed, spread, size } => {
                let mut p = particles(el_c(el), n, 0.7, size);
                p.dir = Vec3::Y;
                p.spread = spread;
                p.v = (speed * 0.5, speed);
                p.damp = (0.5, 1.5);
                self.shot(p, at + Vec3::Y * 0.1, Some(el), false);
            }
            Ev::Zap { a, b, c, width, time } => {
                self.zap(a, b, lerp_c(c, WHITE, 0.0), width.max(0.05), 0.45, 0.5, time.max(0.06));
                self.flash(b, c, 1.4, 0.14);
            }
            Ev::Chain { ref pts, c } => {
                for p in pts.windows(2) {
                    self.zap(p[0], p[1], c, 0.12, 0.4, 0.5, 0.12);
                }
                if let Some(&l) = pts.last() {
                    self.flash(l, c, 1.2, 0.12);
                }
            }
            Ev::Thunder { at, top, c, width, radius } => {
                self.zap(top, at, c, width.max(0.2), 1.2, 1.2, 0.18);
                self.flash(at + Vec3::Y * 0.5, c, radius * 1.6, 0.25);
                self.ring(at, c, radius, 0.35);
                self.light_pop(at, c, 6.0, radius * 3.0, 0.3);
                self.burst(at, c, 18, 6.0, 0.35, Some(El::Lightning));
            }
            Ev::Nova { at, r, ref cs, el } => {
                let c0 = cs.first().copied().unwrap_or(WHITE);
                self.waves.push(Wave { at, c: c0, r: r + 0.4, age: 0.0, time: 0.55, cell: Cell::Shock, glow: 1.8 });
                for (i, &c) in cs.iter().enumerate().skip(1) {
                    self.ring(at, c, r * (1.0 - 0.12 * i as f32), 0.3 + 0.08 * i as f32);
                }
                self.flash(at + Vec3::Y * 0.6, c0, r * 1.2, 0.3);
                let mut p = particles(c0, 70, 0.55, 0.6);
                p.dir = Vec3::X;
                p.spread = 180.0;
                p.flat = 0.85;
                p.v = (r * 2.0, r * 2.8);
                p.damp = (r * 2.0, r * 3.0);
                self.shot(p, at + Vec3::Y * 0.5, el, true);
                self.light_pop(at, c0, 3.5, r * 2.5, 0.4);
            }
            Ev::Boom { at, r, .. } => {
                self.blasts.explosion(self.time, Vec3::new(at.x, 0.0, at.z), r, Vec3::new(1.0, 0.55, 0.15));
            }
            Ev::Shake(a) => self.shake = self.shake.max(a),
            Ev::Summon { at } => {
                let c = el_c(El::Void);
                self.ring(at, c, 1.4, 0.9);
                self.flash(at + Vec3::Y * 0.3, c, 2.0, 0.6);
                let mut p = particles(c, 24, 0.9, 0.3);
                p.dir = Vec3::Y;
                p.spread = 20.0;
                p.sphere = 0.8;
                p.v = (1.0, 2.5);
                p.gravity = Vec3::Y * 1.0;
                self.emit(p, at, 24);
            }
            Ev::PropBroke { prop, from, power } => {
                if let Some(p) = w.level.props.get(prop) {
                    let at = p.pos + Vec3::Y * 0.6;
                    let mut e = particles([0.42, 0.38, 0.33, 1.0], 18, 1.1, 0.5);
                    style(&mut e, El::Earth, false);
                    e.dir = (at - from).normalize_or(Vec3::Y) + Vec3::Y;
                    e.spread = 60.0;
                    e.v = (2.0 + power * 0.3, 5.0 + power * 0.6);
                    e.sphere = 0.4;
                    self.emit(e, at, 18);
                    self.puff(at, rgba(0.55, 0.5, 0.45, 1.0), 1.4);
                }
            }
            Ev::PropChip { prop } => {
                if let Some(p) = w.level.props.get(prop) {
                    self.puff(p.pos + Vec3::Y * 0.6, rgba(0.55, 0.5, 0.45, 1.0), 0.6);
                }
            }
            Ev::WizardDown { seat, how, .. } => {
                if let Some(&id) = w.wizards.get(seat) {
                    self.death(w.pos(id), how, 1.0);
                }
            }
            Ev::WizardUp { seat } => {
                if let Some(&id) = w.wizards.get(seat) {
                    let at = w.pos(id);
                    self.ring(at, rgba(1.0, 0.9, 0.6, 1.0), 2.0, 0.5);
                    self.flash(at + Vec3::Y, rgba(1.0, 0.9, 0.6, 1.0), 2.4, 0.4);
                }
            }
            Ev::MonsterDied { id, how, .. } => {
                let found = w.corpses.iter().chain(&w.enemies).find(|e| e.to_bits().get() == id).copied();
                if let Some(e) = found {
                    let s = w.actor(e).map(|a| a.radius as f32).unwrap_or(0.5);
                    self.death(w.pos(e), how, s.max(0.4) / 0.5);
                }
            }
            Ev::Lob { from, to, el, time } => self.lobs.push(Lob { from, to, el, age: 0.0, time }),
            Ev::Spikes { at, el, r, .. } => {
                let mut p = particles(el_c(el), (8.0 + r * 6.0) as u32, 0.7, 0.5);
                p.dir = Vec3::Y;
                p.spread = 25.0;
                p.sphere = r * 0.7;
                p.v = (3.0, 6.0);
                self.shot(p, at, Some(el), false);
                self.ring(at, el_c(el), r, 0.3);
            }
            _ => {}
        }
    }

    /// How a body leaves (Remains): its element's particles where it fell.
    fn death(&mut self, at: Vec3, how: Death, s: f32) {
        let at = at + Vec3::Y * 0.6;
        match how {
            Death::Freeze => self.burst(at, el_c(El::Ice), (16.0 * s) as u32, 4.0, 0.5, Some(El::Ice)),
            Death::Burn => {
                self.burst(at, el_c(El::Fire), (14.0 * s) as u32, 3.0, 0.6, Some(El::Fire));
                self.puff(at, rgba(0.2, 0.18, 0.17, 1.0), 1.2 * s);
            }
            Death::Shock => self.burst(at, el_c(El::Lightning), (14.0 * s) as u32, 5.0, 0.4, Some(El::Lightning)),
            Death::Void => self.burst(at, el_c(El::Void), (18.0 * s) as u32, 3.0, 0.5, Some(El::Void)),
            Death::Blown => self.burst(at, el_c(El::Air), (12.0 * s) as u32, 5.0, 0.6, Some(El::Air)),
            Death::Gore => {
                let mut p = particles(rgba(0.45, 0.04, 0.03, 1.0), (16.0 * s) as u32, 0.8, 0.5);
                style(&mut p, El::Water, false);
                p.ramp = Ramp::new(&[rgba(0.5, 0.05, 0.03, 0.95), rgba(0.3, 0.02, 0.02, 0.0)], &[0.0, 1.0]);
                p.dir = Vec3::Y;
                p.spread = 70.0;
                p.v = (2.0, 5.0);
                self.emit(p, at, p.amount);
            }
            Death::Fall => {}
        }
    }

    /// Age everything by `dt` and make the sprites and lights of the live
    /// spells, shots and pickups.
    pub fn update(&mut self, w: &World, dt: f32) {
        self.time += dt;
        self.shake = (self.shake - 2.5 * dt).max(0.0);
        // Particles.
        let ems = &self.ems;
        self.parts.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life {
                return false;
            }
            let e = &ems[p.em as usize];
            let mut acc = e.gravity;
            if p.radial != 0.0 {
                acc += (p.pos - p.home).normalize_or_zero() * p.radial;
            }
            p.vel += acc * dt;
            if p.damp > 0.0 {
                let s = p.vel.length();
                if s > 0.0 {
                    p.vel *= (s - p.damp * dt).max(0.0) / s;
                }
            }
            p.pos += p.vel * dt;
            p.angle += p.spin * dt;
            true
        });
        for x in &mut self.waves {
            x.age += dt;
        }
        self.waves.retain(|x| x.age < x.time);
        for x in &mut self.glows {
            x.age += dt;
        }
        self.glows.retain(|x| x.age < x.time);
        for x in &mut self.pops {
            x.age += dt;
        }
        self.pops.retain(|x| x.age < x.time);
        for x in &mut self.zaps {
            x.age += dt;
        }
        self.zaps.retain(|x| x.age < x.time);
        let mut landed = vec![];
        for x in &mut self.lobs {
            x.age += dt;
            if x.age >= x.time {
                landed.push((x.to, x.el));
            }
        }
        self.lobs.retain(|x| x.age < x.time);
        for (at, el) in landed {
            self.burst(at, el_c(el), 10, 3.0, 0.4, Some(el));
        }
        self.live(w, dt);
    }

    /// A cataclysm while it plays (OverloadFx): the meteor falling, the
    /// tidal wave rolling, the black hole pulling, the storm's cloud. The
    /// looks of the wave and the hole are fx.wgsl `wave` and `black_hole`.
    fn overload(&mut self, o: &spellstorm::spells::Overload, key: u64, dt: f32) {
        use spellstorm::spells::{WAVE_TIME, WAVE_WIDTH};
        let t = self.time;
        let ot = o.t as f32;
        let c = o.spell.primary.map(el_c).unwrap_or([1.0, 0.8, 0.5, 1.0]);
        match o.spell.primary {
            Some(El::Fire) => {
                let Some(off) = o.thing else { return };
                let at = o.target + off;
                // A molten rock with a long burning tail.
                self.halo(at, c, 6.0, 1.1);
                let mut rock = glow_sprite(at, 2.2, Cell::Puff, linear3([1.0, 0.75, 0.4]) * 6.0);
                rock.effect = crate::effects::MOLTEN;
                rock.params = [(key % 97) as f32, t, 0.0, 0.0];
                self.live.push(rock);
                let mut p = particles(c, 80, 0.6, 1.4);
                p.cell = Cell::Flame;
                p.sphere = 0.9;
                p.gravity = Vec3::ZERO;
                self.stream((key, 1), p, at, dt);
                self.light(at, c, 4.0, 10.0);
            }
            Some(El::Water) => {
                let Some(at) = o.thing else { return };
                let dir = o.wave_dir.normalize_or(Vec3::NEG_Z);
                let side = Vec3::new(-dir.z, 0.0, dir.x);
                // It rears up as it sets off and sinks into a flood at the end.
                let rise = (ot / 0.25).clamp(0.0, 1.0);
                let sink = 1.0 - ((ot - WAVE_TIME as f32 + 0.3) / 0.3).clamp(0.0, 1.0);
                let h = 2.7 * ((0.2 + 0.8 * rise) * sink).max(0.05);
                // Leaning forward a little, the way it rolls.
                let up = (Vec3::Y + dir * 0.35).normalize();
                let mut wall = Sprite::fixed(at + up * h * 0.5, [WAVE_WIDTH, h], side, up, IMAGE, uv(Cell::Patch), Vec3::splat(1.0));
                wall.alpha_cut = 0.0;
                wall.depth_bias = 0.2;
                wall.effect = crate::effects::WAVE;
                wall.params = [t, sink, 0.0, 0.0];
                self.live.push(wall);
                // Spray torn off the crest, foam churning along its foot.
                let white = [0.9, 0.96, 1.0, 1.0];
                let mut spray = particles(white, 160, 0.8, 0.35);
                spray.dir = (Vec3::Y + dir * 1.5).normalize();
                spray.spread = 25.0;
                spray.v = (2.0, 5.0);
                spray.gravity = Vec3::new(0.0, -9.0, 0.0);
                for k in 0..5 {
                    let x = (k as f32 / 4.0 - 0.5) * WAVE_WIDTH * 0.9;
                    self.stream((key, 10 + k), Em { amount: 32, ..spray }, at + side * x + Vec3::Y * h * 0.95 + dir * 0.6, dt);
                    let mut foot = particles(white, 24, 0.6, 0.45);
                    foot.cell = Cell::Puff;
                    foot.dir = (Vec3::Y + dir).normalize();
                    foot.v = (0.0, 2.5);
                    self.stream((key, 20 + k), foot, at + side * x + dir * 1.0 + Vec3::Y * 0.1, dt);
                }
                self.light(at + Vec3::Y * 2.0 + dir * 1.5, [0.4, 0.75, 1.0, 1.0], 1.5, 7.0);
            }
            Some(El::Void) => {
                // Opens smoothly, holds, then falls in on itself.
                let open = smooth(0.0, 0.6, ot) * (1.0 - smooth(2.6, 3.0, ot) * 0.9);
                let at = o.target + Vec3::Y * 1.2;
                // Facing the camera, which looks down the level from above.
                let up = Vec3::new(0.0, 12.0, -20.0).normalize();
                let mut hole = Sprite::fixed(at, [7.5, 7.5], Vec3::X, up, IMAGE, uv(Cell::Patch), linear3([c[0], c[1], c[2]]) * 2.0);
                hole.alpha_cut = 0.0;
                hole.depth_bias = 1.5;
                hole.effect = crate::effects::BLACK_HOLE;
                hole.params = [open, t, 0.0, 0.0];
                self.live.push(hole);
                // Motes swirling round and in.
                let c3 = linear3([c[0], c[1], c[2]]);
                for j in 0..60 {
                    let h = key as f32 * 0.37 + j as f32 * 1.618;
                    let ph = (t * 0.9 + (h * 12.9898).sin().abs().fract()).fract();
                    let r = 3.6 * (1.0 - ph) * open.max(0.2);
                    let a = h * 6.283 + t * 2.0 + 3.0 * ph * ph;
                    let p = at + Vec3::new(a.cos() * r, (h * 3.1).sin() * 0.6 * (1.0 - ph), a.sin() * r);
                    self.live.push(glow_sprite(p, 0.22, Cell::Dot, c3 * 1.8 * smooth(0.0, 0.2, ph) * (1.0 - smooth(0.8, 1.0, ph))));
                }
                self.light(at, c, 2.0 * open, 8.0);
            }
            Some(El::Lightning) => {
                let Some(cloud) = o.cloud else { return };
                // A dark cloud gathering over the fight, see-through.
                for j in 0..9 {
                    let a = j as f32 * 2.399 + key as f32;
                    let r = if j == 0 { 0.0 } else { 1.6 + (j % 3) as f32 * 0.9 };
                    let at = cloud + Vec3::new(a.cos() * r, 6.0 + (j % 2) as f32 * 0.5, a.sin() * r);
                    let mut s = glow_sprite(at, 4.2, Cell::Puff, Vec3::splat(0.04)).solid();
                    s.opacity = 0.45 * smooth(0.0, 0.4, ot);
                    self.live.push(s);
                }
            }
            _ => {}
        }
    }

    fn halo(&mut self, at: Vec3, c: C, size: f32, strength: f32) {
        let c3 = linear3([c[0], c[1], c[2]]);
        let inner = linear3([c[0] + (1.0 - c[0]) * 0.6, c[1] + (1.0 - c[1]) * 0.6, c[2] + (1.0 - c[2]) * 0.6]) * 0.8;
        self.live.push(glow_sprite(at, size, Cell::HaloOut, c3 * strength * 1.4));
        self.live.push(glow_sprite(at, size, Cell::HaloIn, inner * strength * 1.4));
    }

    fn light(&mut self, at: Vec3, c: C, energy: f32, range: f32) {
        self.live_lights.push(LocalLight {
            position: at,
            direction: Vec3::NEG_Y,
            intensity_cd: linear3([c[0], c[1], c[2]]) * energy * LIGHT,
            range_m: range.max(0.5),
            inner_cos: -0.999,
            outer_cos: -1.0,
            source_radius_m: 0.05,
            falloff: 1.0,
        });
    }

    /// The live spells, shots and pickups: halos, trails, lines, lights.
    fn live(&mut self, w: &World, dt: f32) {
        use spellstorm::ground::Field;
        use spellstorm::pickups::{Bullet, Globe, Soul};
        use spellstorm::spells::{Beam, Bolt as SBolt, Projectile, Sentry, Spray, Trap};
        self.live.clear();
        self.live_lights.clear();
        let t = self.time;
        for &id in &w.fx {
            let key = id.to_bits().get();
            if let Ok(o) = w.ecs.get::<&spellstorm::spells::Overload>(id) {
                self.overload(&o, key, dt);
                continue;
            }
            if let Ok(b) = w.ecs.get::<&SBolt>(id) {
                if b.flight.zap {
                    continue;
                }
                let look = bolt_look(b.el);
                let size = b.size;
                let edge = look.edge;
                let jitter = matches!(b.el, Some(El::Lightning | El::Storm));
                let bloom = if jitter { 0.5 } else { 1.0 };
                let pulse = if look.flicker > 0.5 { 1.0 + 0.25 * (t * 13.0 + b.pos.x * 3.0).sin() } else { 1.0 };
                let hs = (1.1 * size * look.size + 0.3) * if jitter { 0.8 } else { 1.0 };
                self.halo(b.pos, edge, hs, (0.35 + 0.25 * size) * bloom * pulse);
                // The body: a hot core in the element's own colours.
                let flick = 1.0 - look.flicker * 0.25 * (t * look.speed * 2.0 + key as f32).sin().abs();
                let core = linear3([look.core[0], look.core[1], look.core[2]]);
                let body = 0.4 * size * look.size;
                let solid = matches!(b.el, Some(El::Earth | El::Mud));
                if solid {
                    self.live.push(Sprite { opacity: 1.0, ..glow_sprite(b.pos, body, Cell::Drop, core * 0.4) }.solid());
                } else {
                    self.live.push(glow_sprite(b.pos, body * 1.2, Cell::Spark, core * 2.4 * flick));
                }
                let mut tr = particles(look.trail, (3.0 + 6.0 * size) as u32, 0.3 + 0.2 * size, 0.18 * size);
                tr.sphere = 0.14;
                tr.dir = Vec3::Y;
                tr.spread = 180.0;
                tr.v = (0.2, 0.8);
                tr.gravity = Vec3::new(0.0, look.rise, 0.0);
                if let Some(e) = b.el {
                    style(&mut tr, e, false);
                }
                self.stream((key, 0), tr, b.pos, dt);
            } else if let Ok(p) = w.ecs.get::<&Projectile>(id) {
                let e = p.spell.primary.unwrap_or(El::Earth);
                let c = el_c(e);
                let r = p.radius as f32;
                let glows = matches!(e, El::Magma | El::Fire | El::Ice | El::Frost);
                if glows {
                    self.halo(p.pos, c, r * 4.0, 0.6);
                    self.live.push(glow_sprite(p.pos, r * 2.0, Cell::Spark, linear3([c[0], c[1], c[2]]) * 2.0));
                } else {
                    self.live.push(glow_sprite(p.pos, r * 2.2, Cell::Drop, linear3([c[0], c[1], c[2]]) * 0.35).solid());
                }
                let mut tr = particles(c, 8, 0.5, 0.3);
                tr.sphere = r * 0.6;
                tr.dir = Vec3::Y;
                tr.spread = 180.0;
                tr.v = (0.2, 0.6);
                tr.gravity = Vec3::ZERO;
                style(&mut tr, e, true);
                self.stream((key, 0), tr, p.pos, dt);
            } else if let Ok(b) = w.ecs.get::<&Bullet>(id) {
                let c = el_c(b.primary);
                let r = b.radius * 0.72;
                let energy = if matches!(b.primary, El::Frost | El::Ice | El::Air | El::Steam) { 0.85 } else { 1.3 };
                let c3 = linear3([c[0], c[1], c[2]]);
                if b.arrow {
                    let dir = b.vel.normalize_or(Vec3::Z);
                    self.live.push(Sprite::shaft(b.pos, [r * 0.6, r * 7.0], dir, IMAGE, uv(Cell::Line), Vec3::new(1.0, 0.9, 0.75) * 0.6));
                } else {
                    self.live.push(glow_sprite(b.pos, r * 2.4, Cell::Spark, c3 * 2.0 * energy * if b.hot { 1.6 } else { 1.0 }));
                    self.halo(b.pos, c, r * 6.0, 0.45);
                    let mut tr = particles(c, 6, 0.3, r * 1.4);
                    tr.dir = Vec3::Y;
                    tr.spread = 180.0;
                    tr.v = (0.1, 0.4);
                    tr.gravity = Vec3::ZERO;
                    style(&mut tr, b.primary, false);
                    self.stream((key, 0), tr, b.pos, dt);
                }
            } else if let Ok(g) = w.ecs.get::<&Globe>(id) {
                if g.visible() {
                    // A pool of blood on the floor that reaches for the
                    // nearest wizard and runs into them (spells.wgsl `blood`).
                    let to = g.to;
                    let reach = 0.8 + to.length();
                    let mut pool = flat_sprite(Vec3::new(g.pos.x, 0.03, g.pos.z), reach, Cell::Patch, Vec3::splat(LIT * 2.5)).solid();
                    pool.effect = crate::effects::BLOOD;
                    let pull = (g.pull as f32).clamp(0.0, 0.999);
                    let suck = (g.slurp / spellstorm::pickups::GLOBE_SLURP).clamp(0.0, 1.0) as f32;
                    pool.params = [to.x, to.y, pull + (suck * 1000.0).floor(), t + (key % 97) as f32];
                    self.live.push(pool);
                }
            } else if let Ok(s) = w.ecs.get::<&Soul>(id) {
                let c = rgba(0.25, 0.5, 1.0, 1.0);
                let at = s.pos + Vec3::Y * (0.9 + 0.1 * (t * 4.0 + key as f32).sin());
                self.live.push(glow_sprite(at, 0.22, Cell::Spark, linear3([0.45, 0.65, 1.0]) * 3.0));
                self.halo(at, c, 0.12 * 6.0 * 2.0, 0.5);
            } else if let Ok(s) = w.ecs.get::<&Spray>(id) {
                let e = s.spell.primary.unwrap_or(El::Fire);
                let mut p = particles(el_c(e), 50, 0.45, 0.7);
                p.dir = s.dir;
                p.spread = 14.0;
                p.v = (8.0, 11.0);
                p.gravity = Vec3::ZERO;
                p.damp = (2.0, 4.0);
                let smoke = style(&mut p, e, true);
                self.stream((key, 0), p, s.at, dt);
                if let Some(sm) = smoke {
                    self.stream((key, 1), sm, s.at, dt);
                }
                self.light(s.at + s.dir * 2.0, el_c(e), 1.2, 6.0);
            } else if let Ok(b) = w.ecs.get::<&Beam>(id) {
                let e = b.spell.primary.unwrap_or(El::Lightning);
                let c = el_c(e);
                let c3 = linear3([c[0], c[1], c[2]]);
                let d = b.to - b.from;
                let len = d.length();
                if len > 0.05 {
                    let mid = (b.from + b.to) * 0.5;
                    let wob = 1.0 + 0.15 * (t * 30.0).sin();
                    self.live.push(Sprite::shaft(mid, [0.35 * wob, len], d / len, IMAGE, uv(Cell::Line), c3 * 2.0));
                    self.live.push(Sprite::shaft(mid, [0.1, len], d / len, IMAGE, uv(Cell::Line), Vec3::ONE * 3.0));
                }
                self.halo(b.to, c, 1.4, 0.8);
                self.light(b.to + Vec3::Y * 0.3, c, 1.5, 5.0);
            } else if let Ok(tr) = w.ecs.get::<&Trap>(id) {
                if !tr.done {
                    let e = tr.spell.primary.unwrap_or(El::Fire);
                    let c = el_c(e);
                    let arm = (tr.t as f32 / spellstorm::spells::TRAP_ARM as f32).min(1.0);
                    let pulse = 0.6 + 0.4 * (t * 4.0).sin();
                    let r = tr.trigger_r() as f32;
                    self.live.push(flat_sprite(tr.pos + Vec3::Y * 0.05, r, Cell::Ring, linear3([c[0], c[1], c[2]]) * 1.2 * arm * pulse));
                    self.halo(tr.pos + Vec3::Y * 0.2, c, 0.8, 0.5 * arm);
                }
            } else if let Ok(s) = w.ecs.get::<&Sentry>(id) {
                let e = s.spell.primary.unwrap_or(El::Fire);
                let c = el_c(e);
                let at = s.pos + Vec3::Y * 1.2;
                self.live.push(glow_sprite(at, 0.4, Cell::Spark, linear3([c[0], c[1], c[2]]) * 2.5));
                self.halo(at, c, 1.6, 0.6);
                self.light(at, c, 0.8, 4.0);
            }
        }
        for &id in &w.fields {
            let Ok(f) = w.ecs.get::<&Field>(id) else { continue };
            let key = id.to_bits().get();
            let c = el_c(f.kind);
            let c3 = linear3([c[0], c[1], c[2]]);
            let r = f.radius as f32;
            let fade = if f.fading { 0.4 } else { 1.0 } * if !f.permanent { (f.ttl as f32 / 1.0).clamp(0.0, 1.0) } else { 1.0 };
            let at = f.pos + Vec3::Y * 0.04;
            match f.kind {
                El::Fire | El::Magma => {
                    // fx.wgsl `ground`: scorched earth and embers, or molten seams.
                    let mut s = flat_sprite(at, r, Cell::Patch, Vec3::splat(1.2)).solid();
                    s.opacity = fade;
                    s.effect = crate::effects::GROUND;
                    s.params = [f.kind.idx() as f32, r, (key % 97) as f32, t];
                    self.live.push(s);
                    let mut p = particles(c, (3.0 * r * r) as u32 + 4, 1.1, 0.5);
                    p.dir = Vec3::Y;
                    p.sphere = r * 0.8;
                    p.v = (0.6, 1.8);
                    p.gravity = Vec3::Y * 1.5;
                    p.flat = 0.0;
                    let sm = style(&mut p, f.kind, true);
                    let fl = at + Vec3::Y * 0.3;
                    self.stream((key, 0), p, fl, dt);
                    if let Some(sm) = sm {
                        self.stream((key, 1), sm, fl, dt);
                    }
                    self.light(at + Vec3::Y * 0.8, c, 1.4 * fade, r * 2.5);
                }
                El::Lightning | El::Storm => {
                    self.live.push(flat_sprite(at, r, Cell::Patch, c3 * 0.5 * fade));
                }
                El::Steam | El::Air => {
                    let mut p = particles(c, (2.0 * r * r) as u32 + 3, 1.6, 1.2);
                    p.cell = Cell::Puff;
                    p.glow = 0.0;
                    p.ramp = Ramp::new(&[rgba(c[0], c[1], c[2], 0.0), rgba(c[0], c[1], c[2], 0.25), rgba(c[0], c[1], c[2], 0.0)], &[0.0, 0.3, 1.0]);
                    p.dir = Vec3::Y;
                    p.sphere = r * 0.8;
                    p.v = (0.2, 0.8);
                    p.gravity = Vec3::Y * 0.3;
                    self.stream((key, 0), p, at + Vec3::Y * 0.3, dt);
                }
                _ => {
                    // Puddles, slicks, mud, poison and void: a see-through
                    // patch over the floor.
                    // ground_field.gdshader: dark still water, pale ice,
                    // brown mud; other elements their colour, a little lit.
                    let (col, a, glow) = match f.kind {
                        El::Water => ([0.07, 0.18, 0.3], 0.82, 0.0),
                        El::Frost => ([0.3, 0.38, 0.46], 0.7, 0.0),
                        El::Ice => ([0.3, 0.38, 0.46], 0.85, 0.0),
                        El::Mud => ([0.19, 0.13, 0.07], 0.95, 0.0),
                        _ => ([c[0] * 0.6, c[1] * 0.6, c[2] * 0.6], 0.7, 0.25),
                    };
                    // fx.wgsl `ground` draws the kinds it knows in their own
                    // colours, the rest in the element's.
                    let own = matches!(f.kind, El::Water | El::Frost | El::Ice | El::Mud | El::Steam | El::Poison | El::Void);
                    let tint = if own { Vec3::splat(LIT * 2.2) } else { linear3(col) * LIT };
                    let mut s = flat_sprite(at, r, Cell::Patch, tint).solid();
                    s.opacity = fade;
                    s.effect = crate::effects::GROUND;
                    s.params = [f.kind.idx() as f32, r, (key % 97) as f32, t];
                    self.live.push(s);
                    let _ = a;
                    if glow > 0.0 && !own {
                        self.live.push(flat_sprite(at + Vec3::Y * 0.01, r, Cell::Patch, c3 * glow * fade));
                    }
                    if f.kind == El::Poison || f.kind == El::Void {
                        let mut p = particles(c, (r * r) as u32 + 2, 1.2, 0.3);
                        p.dir = Vec3::Y;
                        p.sphere = r * 0.7;
                        p.v = (0.2, 0.6);
                        p.gravity = Vec3::Y * 0.4;
                        self.stream((key, 0), p, at + Vec3::Y * 0.1, dt);
                    }
                }
            }
            if f.electrified > 0.0 && self.f() < dt * 20.0 {
                let a = at + Vec3::new(self.range(-1.0, 1.0), 0.1, self.range(-1.0, 1.0)) * r * 0.85;
                let b = a + Vec3::new(self.range(-0.5, 0.5), 0.05, self.range(-0.5, 0.5));
                self.zap(a, b, el_c(El::Lightning), 0.04, 0.15, 0.2, 0.06);
            }
        }
        // Debt of things that are gone.
        let alive: std::collections::HashSet<u64> = w.fx.iter().chain(&w.fields).map(|e| e.to_bits().get()).collect();
        self.debt.0.retain(|k, _| alive.contains(&k.0));
        // Lobs in the air.
        let lobs: Vec<(Vec3, El)> = self
            .lobs
            .iter()
            .map(|l| {
                let k = (l.age / l.time).clamp(0.0, 1.0);
                let h = 1.0 + l.from.distance(l.to) * 0.35;
                (l.from.lerp(l.to, k) + Vec3::Y * (4.0 * h * k * (1.0 - k)), l.el)
            })
            .collect();
        for (i, (at, el)) in lobs.into_iter().enumerate() {
            let c = el_c(el);
            self.live.push(glow_sprite(at, 0.5, Cell::Spark, linear3([c[0], c[1], c[2]]) * 2.0));
            self.halo(at, c, 1.6, 0.6);
            let mut tr = particles(c, 8, 0.4, 0.3);
            tr.dir = Vec3::Y;
            tr.spread = 180.0;
            tr.v = (0.2, 0.6);
            tr.gravity = Vec3::ZERO;
            style(&mut tr, el, false);
            self.stream((u64::MAX - i as u64, 0), tr, at, dt);
        }
    }

    /// Every sprite this frame, for the renderer.
    pub fn sprites(&self, cam: &render::Camera) -> Vec<Sprite> {
        let face = crate::view::facing(cam);
        let (right, up) = (face.x_axis, face.y_axis);
        let mut out = Vec::with_capacity(self.parts.len() + self.live.len() + 64);
        for x in &self.waves {
            let k = x.age / x.time;
            let ease = 1.0 - (1.0 - k).powi(3);
            let r = x.r * (0.2 + 0.8 * ease);
            let c3 = linear3([x.c[0], x.c[1], x.c[2]]);
            let fade = (1.0 - k).powf(1.2);
            if x.glow > 0.0 {
                out.push(flat_sprite(x.at + Vec3::Y * 0.06, r, x.cell, c3 * x.glow * fade));
            } else {
                let mut s = flat_sprite(x.at + Vec3::Y * 0.06, r, x.cell, c3 * LIT * 2.0).solid();
                s.opacity = x.c[3] * fade;
                out.push(s);
            }
        }
        let flash_k = spellstorm::knobs::flash_brightness() as f32;
        for g in &self.glows {
            let k = g.age / g.time;
            let size = g.size * (1.0 - 0.85 * k * k);
            let c3 = linear3([g.c[0], g.c[1], g.c[2]]);
            let inner = c3.lerp(Vec3::ONE, 0.6) * 0.8;
            out.push(glow_sprite(g.at, size, Cell::HaloOut, c3 * 0.9 * 1.4 * flash_k));
            out.push(glow_sprite(g.at, size, Cell::HaloIn, inner * 0.9 * 1.4 * flash_k));
        }
        for z in &self.zaps {
            let c3 = linear3([z.c[0], z.c[1], z.c[2]]);
            let core = c3.lerp(Vec3::ONE, 0.8);
            for p in z.pts.windows(2) {
                let d = p[1] - p[0];
                let len = d.length();
                if len < 1e-3 {
                    continue;
                }
                let mid = (p[0] + p[1]) * 0.5;
                out.push(Sprite::shaft(mid, [z.width * 3.0, len + z.width], d / len, IMAGE, uv(Cell::Line), c3 * 2.0));
                out.push(Sprite::shaft(mid, [z.width, len + z.width * 0.5], d / len, IMAGE, uv(Cell::Line), core * 4.0));
            }
        }
        out.extend(self.live.iter().copied());
        for p in &self.parts {
            let e = &self.ems[p.em as usize];
            let k = p.age / p.life;
            let c = e.ramp.at(k);
            let s = p.size * curve(&e.curve, k);
            let col = linear3([c[0], c[1], c[2]]) * p.tint;
            let (r, u) = if p.angle != 0.0 {
                let (sn, cs) = p.angle.sin_cos();
                (right * cs + up * sn, up * cs - right * sn)
            } else {
                (right, up)
            };
            let mut sp = Sprite::fixed(p.pos, [s, s], r, u, IMAGE, uv(e.cell), col);
            sp.alpha_cut = 0.0;
            if e.glow > 0.0 {
                sp.additive = true;
                sp.radiance = col * e.glow;
                sp.opacity = c[3];
            } else {
                // Unlit and see-through: smoke, drops and chips show their
                // colour dimmed to the dark of a dungeon.
                sp.radiance = col * LIT;
                sp.opacity = c[3];
            }
            out.push(sp);
        }
        // Glows face the camera fully: one that keeps world up would flatten
        // to a smear seen from the high camera.
        for sp in &mut out {
            if sp.facing == Facing::Billboard {
                sp.facing = Facing::Fixed { right, up };
            }
        }
        if out.len() > render::sprites::MAX_SPRITES {
            out.truncate(render::sprites::MAX_SPRITES);
        }
        out
    }

    /// The flashes and burning things this frame, brightest first.
    pub fn lights(&self) -> Vec<LocalLight> {
        let mut out = self.live_lights.clone();
        let light_k = spellstorm::knobs::flash_light() as f32;
        for p in &self.pops {
            let k = (1.0 - p.age / p.time) * light_k;
            out.push(LocalLight {
                position: p.at,
                direction: Vec3::NEG_Y,
                intensity_cd: linear3(p.c) * p.energy * k * LIGHT,
                range_m: p.range.max(0.5),
                inner_cos: -0.999,
                outer_cos: -1.0,
                source_radius_m: 0.05,
                falloff: 1.0,
            });
        }
        out.extend(self.blasts.lights(self.time).into_iter().map(|l| LocalLight { intensity_cd: l.intensity_cd * light_k, ..l }));
        out
    }

    /// The camera's shake offset this frame.
    pub fn shake_offset(&mut self) -> Vec3 {
        if self.shake <= 0.0 {
            return Vec3::ZERO;
        }
        let a = self.shake * self.shake * 0.4;
        Vec3::new(self.range(-a, a), self.range(-a, a) * 0.5, self.range(-a, a))
    }
}

fn curve(c: &[f32; 3], k: f32) -> f32 {
    if k < 0.5 {
        c[0] + (c[1] - c[0]) * k * 2.0
    } else {
        c[1] + (c[2] - c[1]) * (k - 0.5) * 2.0
    }
}

trait Solid {
    fn solid(self) -> Self;
}

impl Solid for Sprite {
    /// See-through, not added: smoke, drops, patches.
    fn solid(mut self) -> Self {
        self.additive = false;
        self.alpha_cut = 0.0;
        self
    }
}

fn glow_sprite(at: Vec3, size: f32, cell: Cell, radiance: Vec3) -> Sprite {
    let mut s = Sprite::billboard(at, [size, size], IMAGE, uv(cell), radiance);
    s.additive = true;
    s.alpha_cut = 0.0;
    s.depth_bias = 0.3;
    s
}

/// Flat on the floor, `r` from the middle to the rim.
fn flat_sprite(at: Vec3, r: f32, cell: Cell, radiance: Vec3) -> Sprite {
    let mut s = Sprite::fixed(at, [r * 2.0, r * 2.0], Vec3::X, Vec3::NEG_Z, IMAGE, uv(cell), radiance);
    s.additive = true;
    s.alpha_cut = 0.0;
    s.depth_bias = 0.25;
    s
}

struct BoltLook {
    core: C,
    edge: C,
    flicker: f32,
    speed: f32,
    size: f32,
    trail: C,
    rise: f32,
}

/// BoltFx.LOOK.
fn bolt_look(e: Option<El>) -> BoltLook {
    let l = |core: [f32; 3], edge: [f32; 3], flicker: f32, speed: f32, size: f32, trail: [f32; 4], rise: f32| BoltLook {
        core: [core[0], core[1], core[2], 1.0],
        edge: [edge[0], edge[1], edge[2], 1.0],
        flicker,
        speed,
        size,
        trail,
        rise,
    };
    match e {
        Some(El::Fire) => l([1.0, 0.92, 0.55], [1.0, 0.32, 0.04], 1.0, 10.0, 1.0, [1.0, 0.45, 0.08, 1.0], 2.5),
        Some(El::Frost) => l([0.95, 1.0, 1.0], [0.35, 0.75, 1.0], 0.25, 3.0, 0.8, [0.85, 0.95, 1.0, 1.0], -1.0),
        Some(El::Water) => l([0.7, 0.9, 1.0], [0.1, 0.35, 0.95], 0.6, 5.0, 1.05, [0.3, 0.55, 1.0, 1.0], -3.0),
        Some(El::Lightning) => l([1.0, 1.0, 1.0], [0.65, 0.55, 1.0], 1.0, 40.0, 0.75, [0.8, 0.75, 1.0, 1.0], 0.0),
        Some(El::Earth) => l([0.75, 0.6, 0.4], [0.45, 0.32, 0.18], 0.0, 0.0, 1.15, [0.55, 0.45, 0.32, 1.0], -2.0),
        Some(El::Air) => l([0.95, 1.0, 0.97], [0.55, 0.85, 0.8], 0.9, 14.0, 1.1, [0.8, 0.95, 0.9, 0.6], 0.5),
        Some(El::Poison) => l([0.85, 1.0, 0.45], [0.4, 0.8, 0.05], 0.5, 5.0, 0.95, [0.45, 0.75, 0.1, 1.0], -0.5),
        Some(El::Void) => l([0.25, 0.0, 0.35], [0.75, 0.25, 1.0], 0.7, 6.0, 1.1, [0.55, 0.15, 0.85, 1.0], 0.0),
        Some(El::Steam) => l([1.0, 1.0, 1.0], [0.75, 0.8, 0.9], 0.9, 7.0, 1.2, [0.9, 0.92, 0.95, 0.6], 2.0),
        Some(El::Ice) => l([0.95, 1.0, 1.0], [0.45, 0.8, 1.0], 0.15, 2.0, 1.0, [0.75, 0.92, 1.0, 1.0], -1.5),
        Some(El::Mud) => l([0.5, 0.38, 0.22], [0.3, 0.22, 0.12], 0.0, 0.0, 1.1, [0.4, 0.3, 0.18, 1.0], -4.0),
        Some(El::Magma) => l([1.0, 0.85, 0.3], [1.0, 0.25, 0.02], 0.8, 5.0, 1.25, [1.0, 0.35, 0.05, 1.0], 1.0),
        Some(El::Storm) => l([1.0, 1.0, 1.0], [0.45, 0.55, 1.0], 1.0, 35.0, 0.95, [0.6, 0.7, 1.0, 1.0], 0.5),
        None => l([1.0, 0.95, 0.75], [1.0, 0.75, 0.3], 0.3, 6.0, 0.7, [1.0, 0.85, 0.45, 1.0], 0.5),
    }
}
