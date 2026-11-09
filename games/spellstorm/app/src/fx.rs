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
    /// Lit by the lights round it like the floor (blood), not a set dim.
    pub lit: bool,
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
        lit: false,
    }
}

/// Parts.style: element `e`'s own particles. Returns the smoke that curls
/// off fire too, when `smoke`.
/// How heavy a spell looks: 1 with one of its element, more with two or
/// three of the same (Joep: two fire is a bigger, fiercer flamethrower).
fn heft(s: &spellstorm::Spell) -> f32 {
    let k = s.counts.iter().map(|c| c.1).max().unwrap_or(1).max(1) as f32;
    1.0 + 0.6 * (k - 1.0)
}

/// A spell's elements besides its main one, each with how many: they run
/// through its look (poison and fire make a poison fire, Joep).
fn others(s: &spellstorm::Spell) -> Vec<(El, u8)> {
    s.counts.iter().filter(|c| Some(c.0) != s.primary && c.1 > 0).copied().collect()
}

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
    /// GPU sparks and embers for eruptions and bombs (opt-in: KERN_GPU_PARTICLES=1).
    pub sparks: render::gpu_particles::GpuParticles,
    /// Props broken this tick: where the blow came from and how hard, for
    /// the view to throw their chunks.
    pub prop_hits: HashMap<usize, (Vec3, f32)>,
    /// The light round the level last frame (the view sets it): local lights
    /// and the even light from above (ambient and moon, lux).
    pub env_lights: Vec<LocalLight>,
    pub env_light: Vec3,
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
        // Growing Guns' blasts and baked volumetric smoke (render::fx), for
        // eruptions, bombs and clouds; SPELLSTORM_BAKED_SMOKE=0 for the
        // cheap billows instead.
        let mut blasts = render::fx::Effects::with_explosion_budget(render::fx::ExplosionBudget::FULL);
        blasts.set_baked_smoke(std::env::var("SPELLSTORM_BAKED_SMOKE").as_deref() != Ok("0"));
        blasts.light_scale = LIGHT;
        let mut sparks = render::gpu_particles::GpuParticles::new(65_536);
        // The floor of every level is at 0.
        sparks.set_floor(0.0);
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
            sparks,
            prop_hits: HashMap::new(),
            env_lights: Vec::new(),
            env_light: Vec3::ZERO,
            live: vec![],
            live_lights: vec![],
            time: 0.0,
        }
    }
}

impl Fx {
    pub fn f(&mut self) -> f32 {
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
    /// A small spurt of blood off a hit wizard, thrown the way the blow
    /// pushed them.
    /// Radiance of a white surface facing up at `p`, lit as the floor is:
    /// nearly black in the dark, bright under a torch.
    pub fn lit_at(&self, p: Vec3) -> Vec3 {
        render::sprites::irradiance(&self.env_lights, p, Vec3::Y, self.env_light) / std::f32::consts::PI
    }

    pub fn spurt(&mut self, at: Vec3, dir: Vec3) {
        let mut p = particles(rgba(0.45, 0.04, 0.03, 1.0), 9, 0.5, 0.22);
        style(&mut p, El::Water, false);
        p.lit = true;
        p.ramp = Ramp::new(&[rgba(0.5, 0.05, 0.03, 0.95), rgba(0.3, 0.02, 0.02, 0.0)], &[0.0, 1.0]);
        p.dir = (dir.normalize_or_zero() + Vec3::Y * 0.8).normalize();
        p.spread = 35.0;
        p.v = (1.5, 3.5);
        self.emit(p, at, p.amount);
    }

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

    /// Sand and grit kicked up from the floor round `at`, falling back.
    fn sand(&mut self, at: Vec3, r: f32) {
        let mut p = particles(rgba(0.52, 0.45, 0.36, 1.0), (10.0 + r * 4.0) as u32, 0.9, 0.22);
        style(&mut p, El::Earth, false);
        p.dir = Vec3::Y;
        p.spread = 35.0;
        p.sphere = r * 0.35;
        p.v = (3.0, 6.5);
        p.scale = (0.4, 1.0);
        self.emit(p, at + Vec3::Y * 0.1, p.amount);
        self.puff(at + Vec3::Y * 0.15, rgba(0.5, 0.45, 0.38, 1.0), 0.5 + r * 0.2);
    }

    /// A burst of fire (a barrel, a fireball): the flamethrower's flames
    /// (fx.wgsl flame) welling up and out with their smoke, so fire looks
    /// the same everywhere.
    fn fire_burst(&mut self, at: Vec3, r: f32) {
        let r = r.max(0.5);
        let mut p = particles(el_c(El::Fire), (16.0 + r * 14.0) as u32, 0.7, 0.5 + r * 0.18);
        let smoke = style(&mut p, El::Fire, true);
        p.dir = Vec3::Y;
        p.spread = 75.0;
        p.sphere = r * 0.35;
        p.v = (1.0 + r * 0.5, 2.5 + r * 1.2);
        p.damp = (1.5, 3.0);
        p.curve = [0.35, 1.1, 1.7];
        p.gravity = Vec3::Y * 2.0;
        self.emit(p, at + Vec3::Y * 0.3, p.amount);
        if let Some(mut s) = smoke {
            s.amount = (s.amount * 2).max(8);
            s.sphere = r * 0.4;
            self.emit(s, at + Vec3::Y * 0.6, s.amount);
        }
        // Its light only: light_pop's halo would be a glowing ball.
        self.pops.push(Pop { at: at + Vec3::Y, c: [1.0, 0.62, 0.22], energy: 4.0 + r, range: 4.0 + r * 2.5, age: 0.0, time: 0.45 });
    }

    /// Thousands of GPU sparks off a blast at `at`, `r` across, in the
    /// colour `c` (linear): streaks that skid over the floor, embers that
    /// swirl up on the heat. `grit`: lit dust thrown out low as well.
    fn gpu_burst(&mut self, at: Vec3, r: f32, c: Vec3, grit: bool) {
        use render::gpu_particles::Emitter;
        if !self.sparks.enabled() { return; }
        let r = r.clamp(0.5, 12.0);
        // Saturated: the tone curve whitens anything this bright anyway.
        let hot = c * Vec3::new(1.0, 0.8, 0.5);
        let t = self.time;
        self.sparks.emit(t, &Emitter { position: at + Vec3::Y * 0.3, radius: 0.2 * r, spread: 1.5, speed: [2.0 * r, 5.5 * r],
            count: (r * 500.0) as u32, life: [0.3, 1.2], size: [0.05, 0.02], glow: 5.0 * LIGHT,
            color: [hot.extend(1.0), (c * Vec3::new(1.0, 0.4, 0.15)).extend(0.0)], ..Emitter::sparks() });
        self.sparks.emit(t, &Emitter { position: at + Vec3::Y * 0.5, radius: 0.5 * r, speed: [0.3, 0.8 * r],
            count: (r * 160.0) as u32, duration: 0.6, life: [1.2, 3.0], size: [0.06, 0.025], glow: 4.0 * LIGHT,
            color: [c.extend(1.0), (c * Vec3::new(0.8, 0.25, 0.08)).extend(0.0)], ..Emitter::embers() });
        // Burning gobs: thrown out, they arc, bounce and roll, trailing fire,
        // and glow on the floor as they cool.
        self.sparks.emit(t, &Emitter { position: at + Vec3::Y * 0.4, radius: 0.3 * r, spread: 1.2, speed: [1.0 * r, 3.5 * r],
            count: (r * 140.0) as u32, life: [2.0, 5.0], size: [0.08, 0.04], glow: 2.5 * LIGHT,
            color: [(c * Vec3::new(1.0, 0.7, 0.3)).extend(1.0), (c * Vec3::new(0.6, 0.1, 0.02)).extend(0.0)], ..Emitter::burning() });
        if grit {
            self.sparks.emit(t, &Emitter { position: at + Vec3::Y * 0.2, radius: 0.3 * r, spread: 1.4, speed: [0.6 * r, 2.0 * r],
                count: (r * 120.0) as u32, life: [1.0, 2.5], size: [0.05, 0.12], ..Emitter::dust() });
        }
    }

    /// Growing Guns' volumetric smoke (render::fx::smoke_cloud): `tint` its
    /// linear albedo, `density` 0.3 a wisp .. 1.5 thick.
    fn smoke(&mut self, at: Vec3, r: f32, tint: [f32; 3], density: f32) {
        self.blasts.smoke_cloud(self.time, at, r, Vec3::from(tint), density);
    }

    /// Chunks of `e` flung up and out from `at` that fall back: rock, mud,
    /// gobs of lava, ice.
    fn chunks(&mut self, at: Vec3, r: f32, e: El, n: u32, size: f32) {
        let mut p = particles(el_c(e), n, 1.1, size);
        style(&mut p, e, false);
        p.size = size;
        p.dir = Vec3::Y;
        p.spread = 55.0;
        p.sphere = r * 0.25;
        p.v = (3.0 + r * 1.2, 5.5 + r * 2.0);
        p.gravity = Vec3::NEG_Y * 14.0;
        p.damp = (0.0, 0.5);
        p.angle = (-180.0, 180.0);
        p.spin = (-540.0, 540.0);
        p.curve = [1.0, 1.0, 0.6];
        if matches!(e, El::Earth | El::Mud) {
            p.solid = true;
            p.glow = 0.0;
        }
        self.emit(p, at + Vec3::Y * 0.2, n);
    }

    /// `e`'s own particles welling up at `n` points round a ring `r` out.
    fn ring_of(&mut self, at: Vec3, r: f32, e: El, n: u32, each: u32, up: (f32, f32), size: f32) {
        let mut p = particles(el_c(e), each, 0.7, size);
        let smoke = style(&mut p, e, false);
        let _ = smoke;
        p.dir = Vec3::Y;
        p.spread = 25.0;
        p.sphere = 0.25;
        p.v = up;
        let off = self.f() * std::f32::consts::TAU;
        for i in 0..n {
            let a = off + i as f32 / n as f32 * std::f32::consts::TAU;
            let q = at + Vec3::new(a.cos() * r, 0.15, a.sin() * r);
            self.emit(p, q, each);
        }
    }

    /// A bolt ending at `at`: a small splash of its element, never smoke
    /// or a blast (there are many of them).
    fn impact(&mut self, at: Vec3, el: Option<El>, size: f32) {
        let k = (0.6 + size * 0.5).clamp(0.5, 1.6);
        let at = if at.y < 0.1 { at + Vec3::Y * 0.1 } else { at };
        let Some(e) = el else {
            self.burst(at, rgba(1.0, 0.95, 0.85, 1.0), (6.0 * k) as u32, 3.0, 0.08, None);
            return;
        };
        let c = el_c(e);
        let mut p = particles(c, (10.0 * k) as u32 + 4, 0.4, 0.3 * k);
        let _ = style(&mut p, e, false);
        p.dir = Vec3::Y;
        p.spread = 70.0;
        p.v = (1.5 * k, 4.0 * k);
        match e {
            El::Earth | El::Mud | El::Magma | El::Ice => {
                self.chunks(at, 0.4 * k, e, (6.0 * k) as u32 + 2, 0.12 * k);
                if e == El::Earth {
                    self.puff(at, rgba(0.5, 0.45, 0.38, 1.0), 0.4 * k);
                }
            }
            El::Fire => {
                p.gravity = Vec3::Y * 3.0;
                p.v = (0.8, 2.2 * k);
                self.emit(p, at, p.amount);
                self.puff(at + Vec3::Y * 0.2, rgba(0.2, 0.18, 0.17, 1.0), 0.35 * k);
            }
            El::Steam | El::Air => self.puff(at, if e == El::Steam { rgba(0.95, 0.96, 1.0, 1.0) } else { rgba(0.8, 0.85, 0.8, 1.0) }, 0.5 * k),
            El::Void => {
                p.sphere = 0.6 * k;
                p.v = (0.0, 0.2);
                p.radial = (-6.0, -4.0);
                self.emit(p, at, p.amount);
            }
            _ => self.emit(p, at, p.amount),
        }
        self.flash(at, c, 0.5 * k, 0.12);
    }

    /// How element `e` crashes out at `at`, `r` across, at `k` of its full
    /// size (a nova's element, or one riding in it at half): every element
    /// its own way (Joep: a lame Crushing Eruption). `thrown`: a bomb or a
    /// boulder landing rather than a nova from the caster's feet.
    pub fn crash(&mut self, at: Vec3, r: f32, e: El, k: f32, thrown: bool) {
        let at = Vec3::new(at.x, at.y.min(0.3).max(0.0), at.z);
        let n = |x: f32| ((x * k) as u32).max(3);
        match e {
            El::Magma => {
                // An eruption: a blast out of the ground, burning gobs flung
                // high, black smoke over it.
                self.blasts.explosion(self.time, at + Vec3::Y * 0.2, (r * 0.75 * k).max(0.8), Vec3::new(1.0, 0.45, 0.1));
                self.gpu_burst(at, r * 0.6 * k, Vec3::new(1.0, 0.42, 0.08), false);
                self.chunks(at, r * k, El::Magma, n(26.0 + r * 4.0), 0.22);
                self.fire_burst(at, r * 0.4 * k);
                self.smoke(at, r * 0.6 * k + 0.6, [0.1, 0.09, 0.08], 0.8);
                self.shake = self.shake.max(0.25 * k);
            }
            El::Fire => {
                // A ring of flame that leaps up where the wave goes.
                if thrown {
                    self.fire_burst(at, r * 0.6 * k);
                } else {
                    self.ring_of(at, r * 0.85, El::Fire, n(10.0 + r * 2.0), 6, (1.5, 3.5), 0.55);
                    self.fire_burst(at, 0.6 * k);
                }
                self.smoke(at, r * 0.45 * k + 0.4, [0.16, 0.15, 0.14], 0.45);
            }
            El::Earth => {
                // Rock thrown up and a ring of dust rolling out.
                self.chunks(at, r * k, El::Earth, n(18.0 + r * 3.0), 0.24);
                let m = n(5.0 + r).min(8);
                let off = self.f() * std::f32::consts::TAU;
                for i in 0..m {
                    let a = off + i as f32 / m as f32 * std::f32::consts::TAU;
                    let q = at + Vec3::new(a.cos(), 0.0, a.sin()) * r * if thrown { 0.4 } else { 0.75 };
                    self.smoke(q, 0.5 + r * 0.18 * k, [0.42, 0.36, 0.28], 0.35);
                }
                self.sand(at, r * k);
                self.shake = self.shake.max(0.2 * k);
            }
            El::Mud => {
                self.chunks(at, r * k, El::Mud, n(22.0 + r * 3.0), 0.2);
                self.smoke(at, 0.4 + r * 0.3 * k, [0.3, 0.24, 0.16], 0.25);
            }
            El::Steam => {
                // A scalding cloud billowing out.
                self.smoke(at, r * 0.9 * k + 0.5, [0.92, 0.94, 0.96], 0.9);
                let m = n(4.0);
                for i in 0..m {
                    let a = i as f32 / m as f32 * std::f32::consts::TAU;
                    self.smoke(at + Vec3::new(a.cos(), 0.0, a.sin()) * r * 0.6, 0.5 + r * 0.3 * k, [0.9, 0.92, 0.95], 0.6);
                }
            }
            El::Poison => {
                // A sickly green cloud that hangs where it burst.
                self.smoke(at, r * 0.8 * k + 0.5, [0.38, 0.62, 0.18], 0.7);
                self.ring_of(at, r * 0.6, El::Poison, n(8.0), 4, (2.0, 4.0), 0.5);
            }
            El::Void => {
                // Everything pulled in to a point, then a dark bloom.
                let mut p = particles(el_c(El::Void), n(60.0), 0.45, 0.4);
                style(&mut p, El::Void, false);
                p.sphere = r * 0.9;
                p.dir = Vec3::Y;
                p.spread = 180.0;
                p.v = (0.0, 0.3);
                p.radial = (-r * 9.0, -r * 6.0);
                self.emit(p, at + Vec3::Y * 0.8, p.amount);
                self.smoke(at, r * 0.4 * k + 0.4, [0.12, 0.05, 0.16], 0.6);
            }
            El::Frost | El::Ice => {
                // Shards and frost thrown out low, a cold mist after.
                let mut p = particles(el_c(e), n(40.0 + r * 6.0), 0.6, if e == El::Ice { 0.9 } else { 0.6 });
                style(&mut p, e, false);
                p.dir = Vec3::X;
                p.spread = 180.0;
                p.flat = 0.8;
                p.v = (r * 2.0, r * 3.2);
                p.damp = (r * 2.5, r * 3.5);
                self.emit(p, at + Vec3::Y * 0.4, p.amount);
                if e == El::Ice {
                    self.chunks(at, r * 0.6 * k, El::Ice, n(14.0), 0.3);
                }
                self.smoke(at, r * 0.7 * k + 0.4, [0.82, 0.9, 0.98], 0.3);
            }
            El::Water => {
                // A splash: a crown of water thrown up that rains back.
                let mut p = particles(el_c(El::Water), n(60.0 + r * 8.0), 0.9, 0.5);
                style(&mut p, El::Water, false);
                p.dir = Vec3::Y;
                p.spread = 50.0;
                p.sphere = r * 0.3;
                p.v = (3.0 + r, 6.0 + r * 1.5);
                self.emit(p, at + Vec3::Y * 0.2, p.amount);
                self.waves.push(Wave { at, c: rgba(0.75, 0.85, 1.0, 0.5), r: r * 1.1, age: 0.0, time: 0.7, cell: Cell::Ring, glow: 0.0 });
            }
            El::Lightning | El::Storm => {
                // Bolts forking out to the edge of it.
                let c = el_c(e);
                let m = n(7.0 + r);
                let off = self.f() * std::f32::consts::TAU;
                for i in 0..m {
                    let a = off + (i as f32 + self.range(-0.3, 0.3)) / m as f32 * std::f32::consts::TAU;
                    let end = at + Vec3::new(a.cos() * r, self.range(0.1, 0.6), a.sin() * r);
                    self.zap(at + Vec3::Y * 0.8, end, c, 0.06 * k + 0.03, 0.35, 0.35, 0.18);
                }
                if e == El::Storm {
                    // A thundercloud overhead, bolts down from it.
                    self.smoke(at + Vec3::Y * 2.5, r * 0.6 * k + 0.6, [0.2, 0.22, 0.28], 0.7);
                    for _ in 0..n(3.0) {
                        let q = at + Vec3::new(self.range(-1.0, 1.0), 0.0, self.range(-1.0, 1.0)) * r * 0.7;
                        self.zap(q + Vec3::Y * 4.0, q, rgba(0.85, 0.9, 1.0, 1.0), 0.09, 0.5, 0.5, 0.2);
                    }
                }
                self.light_pop(at, c, 4.0 * k, r * 2.0, 0.25);
            }
            El::Air => {
                // A gust: streaks swirling out and dust lifted off the floor.
                let mut p = particles(el_c(El::Air), n(40.0), 0.5, 0.9);
                style(&mut p, El::Air, false);
                p.dir = Vec3::X;
                p.spread = 180.0;
                p.flat = 0.9;
                p.v = (r * 2.5, r * 3.5);
                p.damp = (r * 3.0, r * 4.0);
                self.emit(p, at + Vec3::Y * 0.5, p.amount);
                self.sand(at, r * 0.8 * k);
                self.smoke(at, r * 0.5 * k + 0.4, [0.75, 0.75, 0.72], 0.2);
            }
        }
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
            // Damage numbers are the HUD's (Hud::draw_numbers).
            Ev::Text { .. } => {}
            Ev::Fountain { at, el, n, speed, spread, size } => {
                let mut p = particles(el_c(el), n, 0.7, size);
                p.dir = Vec3::Y;
                p.spread = spread;
                p.v = (speed * 0.5, speed);
                p.damp = (0.5, 1.5);
                self.shot(p, at + Vec3::Y * 0.1, Some(el), false);
            }
            // No halo where a zap lands: a struck body flashes white itself
            // (a haze in front of it hid the monster).
            Ev::Zap { a, b, c, width, time } => {
                self.zap(a, b, lerp_c(c, WHITE, 0.0), width.max(0.05), 0.45, 0.5, time.max(0.06));
            }
            Ev::Chain { ref pts, c } => {
                for p in pts.windows(2) {
                    self.zap(p[0], p[1], c, 0.12, 0.4, 0.5, 0.12);
                }
            }
            // Where lightning strikes: a few sparks and sand thrown up, no
            // ring on the floor (Joep).
            Ev::Thunder { at, top, c, width, radius } => {
                self.zap(top, at, c, width.max(0.2), 1.2, 1.2, 0.18);
                self.flash(at + Vec3::Y * 0.5, c, radius * 1.6, 0.25);
                self.light_pop(at, c, 6.0, radius * 3.0, 0.3);
                self.burst(at, c, 8, 6.0, 0.35, Some(El::Lightning));
                self.sand(at, radius);
            }
            Ev::Impact { at, el, size } => self.impact(at, el, size),
            Ev::Land { at, r, el, ref payload } => {
                if let Some(e) = el {
                    self.crash(at, r, e, 1.0, true);
                }
                for &e in payload.iter().take(2) {
                    self.crash(at, r * 0.7, e, 0.5, true);
                }
            }
            Ev::Nova { at, r, ref cs, el, ref payload } => {
                if let Some(e) = el {
                    self.crash(at, r, e, 1.0, false);
                }
                for &e in payload.iter().take(2) {
                    self.crash(at, r * 0.8, e, 0.5, false);
                }
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
                let at = Vec3::new(at.x, 0.0, at.z);
                self.fire_burst(at, r);
                if r >= 1.0 {
                    self.blasts.explosion(self.time, at + Vec3::Y * 0.3, r * 1.2, Vec3::new(1.0, 0.6, 0.2));
                    self.gpu_burst(at, r, Vec3::new(1.0, 0.6, 0.2), true);
                } else {
                    self.smoke(at + Vec3::Y * 0.2, 0.6 + r, [0.16, 0.15, 0.14], 0.5);
                }
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
                self.prop_hits.insert(prop, (from, power));
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
            Death::Freeze => {
                self.burst(at, el_c(El::Ice), (16.0 * s) as u32, 4.0, 0.5, Some(El::Ice));
                self.smoke(at - Vec3::Y * 0.5, 0.8 * s, [0.8, 0.88, 0.95], 0.25);
            }
            Death::Burn => {
                self.burst(at, el_c(El::Fire), (14.0 * s) as u32, 3.0, 0.6, Some(El::Fire));
                self.puff(at, rgba(0.2, 0.18, 0.17, 1.0), 1.2 * s);
                // Burnt to death it smokes as it falls.
                self.smoke(at - Vec3::Y * 0.5, 0.9 * s, [0.13, 0.12, 0.11], 0.6);
            }
            Death::Shock => self.burst(at, el_c(El::Lightning), (14.0 * s) as u32, 5.0, 0.4, Some(El::Lightning)),
            Death::Void => self.burst(at, el_c(El::Void), (18.0 * s) as u32, 3.0, 0.5, Some(El::Void)),
            Death::Blown => self.burst(at, el_c(El::Air), (12.0 * s) as u32, 5.0, 0.6, Some(El::Air)),
            Death::Gore => {
                let mut p = particles(rgba(0.45, 0.04, 0.03, 1.0), (16.0 * s) as u32, 0.8, 0.5);
                style(&mut p, El::Water, false);
                p.lit = true;
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
    fn overload(&mut self, o: &spellstorm::spells::Overload, field: Option<Vec3>, key: u64, dt: f32) {
        use spellstorm::spells::{WAVE_TIME, WAVE_WIDTH};
        let t = self.time;
        let ot = o.t as f32;
        let c = o.spell.primary.map(el_c).unwrap_or([1.0, 0.8, 0.5, 1.0]);
        // Its other elements burn, swirl or crackle round the big thing
        // too, so the mix shows (Joep).
        if let Some(at) = o.thing.map(|th| if o.spell.primary == Some(El::Fire) { o.target + th } else { th }) {
            for (i, (e2, n2)) in others(&o.spell).into_iter().enumerate() {
                let mut q = particles(el_c(e2), 20 * n2 as u32 + 12, 0.6, 0.7);
                let _ = style(&mut q, e2, false);
                q.sphere = 1.0;
                q.dir = Vec3::Y;
                q.spread = 120.0;
                q.v = (0.5, 2.0);
                self.stream((key, 20 + i as u8), q, at, dt);
            }
        }
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
                // The water it pushes along the floor (the Godot game's block
                // of water, Joep's "vierkantje"): exactly the strip that
                // shoves monsters, moving with it (spells.wgsl `surge`).
                let mut sea = Sprite::fixed(at + Vec3::Y * 0.05 - dir * 0.2, [WAVE_WIDTH, 3.2], side, dir, IMAGE, uv(Cell::Patch), Vec3::splat(1.0));
                sea.alpha_cut = 0.0;
                sea.additive = false;
                sea.depth_bias = 0.25;
                sea.effect = crate::effects::SURGE;
                sea.params = [t, sink, (key % 97) as f32, 0.0];
                self.live.push(sea);
                // Small drops, not blobs.
                let mut spray = particles(white, 160, 0.8, 0.35);
                spray.dir = (Vec3::Y + dir * 1.5).normalize();
                spray.spread = 25.0;
                spray.v = (2.0, 5.0);
                spray.gravity = Vec3::new(0.0, -9.0, 0.0);
                spray.cell = Cell::Drop;
                spray.size = 0.09;
                spray.glow = 0.6;
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
                // A black cloud low over the fight, flickering from inside, and
                // rain driving down out of it. (At its own height it hung out of
                // the top of the frame, so it is drawn just over their heads.)
                let low = Vec3::new(cloud.x, 3.4, cloud.z);
                if (ot * 6.0).fract() < dt * 6.0 {
                    let a = ot * 14.4 + key as f32;
                    let r = 1.0 + (ot * 7.3).fract() * 4.0;
                    self.smoke(low + Vec3::new(a.cos() * r, 0.0, a.sin() * r), 3.4, [0.08, 0.08, 0.12], 1.8);
                }
                if (t * 9.0 + key as f32).sin() > 0.9 {
                    self.light(low, [0.6, 0.65, 1.0, 1.0], 6.0, 14.0);
                }
                let mut rain = particles(rgba(0.75, 0.82, 1.0, 1.0), 140, 0.25, 0.12);
                let _ = style(&mut rain, El::Water, false);
                rain.size = 0.08;
                rain.glow = 0.5;
                rain.sphere = 4.5;
                rain.dir = Vec3::NEG_Y;
                rain.spread = 4.0;
                rain.v = (13.0, 17.0);
                rain.gravity = Vec3::ZERO;
                self.stream((key, 40), rain, low, dt);
            }
            // A tornado: rings of dust turning up a funnel, wider as it rises,
            // with grit flung round it.
            Some(El::Air) => {
                let Some(at) = field else { return };
                let grow = smooth(0.0, 0.5, ot);
                let dust = self.lit_at(at + Vec3::Y) * 0.2 + Vec3::new(0.012, 0.012, 0.01);
                for layer in 0..16 {
                    let k = layer as f32 / 15.0;
                    let r = 0.35 + 2.4 * k * k;
                    for j in 0..3 {
                        let a = t * (7.0 - 3.5 * k) + layer as f32 * 0.9 + j as f32 * 2.094;
                        let p = at + Vec3::new(a.cos() * r, 0.2 + k * 6.0 * grow, a.sin() * r);
                        let mut s = glow_sprite(p, 0.9 + 1.6 * k, Cell::Puff, dust).solid();
                        s.opacity = 0.4 * grow;
                        self.live.push(s);
                    }
                }
                let mut grit = particles(rgba(0.42, 0.36, 0.3, 1.0), 40, 0.8, 0.3);
                let _ = style(&mut grit, El::Earth, false);
                grit.dir = Vec3::Y;
                grit.spread = 30.0;
                grit.sphere = 1.3;
                grit.v = (3.0, 7.0);
                grit.gravity = Vec3::NEG_Y * 5.0;
                self.stream((key, 40), grit, at + Vec3::Y * 0.3, dt);
            }
            // A blizzard: snow driven slantwise over the whole field, cold mist
            // along the floor.
            Some(El::Frost) => {
                let Some(at) = field else { return };
                let mut snow = particles(WHITE, 500, 1.0, 0.3);
                let _ = style(&mut snow, El::Frost, false);
                snow.amount = 500;
                snow.size = 0.26;
                snow.sphere = 5.0;
                snow.dir = (Vec3::NEG_Y + Vec3::X * 0.45).normalize();
                snow.spread = 8.0;
                snow.v = (6.0, 9.0);
                snow.gravity = Vec3::ZERO;
                self.stream((key, 40), snow, at + Vec3::Y * 6.5, dt);
                let mut mist = particles(WHITE, 60, 1.6, 2.2);
                mist.cell = Cell::Puff;
                mist.glow = 0.12;
                mist.ramp = Ramp::new(&[rgba(0.9, 0.96, 1.0, 0.0), rgba(0.9, 0.96, 1.0, 0.32), rgba(0.9, 0.96, 1.0, 0.0)], &[0.0, 0.4, 1.0]);
                mist.sphere = 4.5;
                mist.dir = Vec3::X;
                mist.v = (0.3, 1.0);
                mist.gravity = Vec3::ZERO;
                self.stream((key, 41), mist, at + Vec3::Y * 0.5, dt);
                self.light(at + Vec3::Y * 3.0, [0.6, 0.8, 1.0, 1.0], 1.6, 10.0);
            }
            // A plague: spores rising off the pool, sick clouds now and then.
            Some(El::Poison) => {
                let Some(at) = field else { return };
                let mut spores = particles(c, 90, 1.5, 0.2);
                spores.sphere = 4.5;
                spores.dir = Vec3::Y;
                spores.spread = 25.0;
                spores.v = (0.6, 1.8);
                spores.gravity = Vec3::Y * 0.4;
                self.stream((key, 40), spores, at + Vec3::Y * 0.3, dt);
                if (ot * 1.5).fract() < dt * 1.5 {
                    let a = ot * 2.399;
                    self.smoke(at + Vec3::new(a.cos() * 2.5, 0.8, a.sin() * 2.5), 2.2, [0.3, 0.55, 0.12], 0.7);
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
                let field = o.field.and_then(|f| w.ecs.get::<&Field>(f).ok().map(|x| x.pos));
                self.overload(&o, field, key, dt);
                continue;
            }
            if let Ok(b) = w.ecs.get::<&SBolt>(id) {
                if b.flight.zap {
                    continue;
                }
                // Its other elements trail off it in their own way: a fire
                // bolt with poison in it sheds green flames too.
                for (i, &(e2, n2)) in b.spell.counts.iter().filter(|c| Some(c.0) != b.el && c.1 > 0).enumerate() {
                    let mut q = particles(el_c(e2), (5.0 * n2 as f32 + 4.0 * b.size) as u32 + 3, 0.35, 0.16 + 0.12 * b.size);
                    let _ = style(&mut q, e2, false);
                    q.sphere = 0.1 + 0.05 * b.size;
                    q.dir = -b.vel.normalize_or(Vec3::Z) + Vec3::Y * 0.3;
                    q.spread = 40.0;
                    q.v = (0.3, 1.2);
                    self.stream((key, 3 + i as u8), q, b.pos, dt);
                }
                if b.el == Some(El::Fire) {
                    // A fire bolt is a knot of the flamethrower's flames
                    // licking back along its path, with a little smoke; no
                    // glowing ball (Joep).
                    let size = b.size;
                    let mut p = particles(el_c(El::Fire), (24.0 + 16.0 * size) as u32, 0.32, 0.3 + 0.22 * size);
                    let smoke = style(&mut p, El::Fire, true);
                    p.dir = -b.vel.normalize_or(Vec3::Z) + Vec3::Y * 0.4;
                    p.spread = 30.0;
                    p.sphere = 0.08 + 0.05 * size;
                    p.v = (0.5, 1.6);
                    p.curve = [0.35, 1.1, 1.4];
                    self.stream((key, 0), p, b.pos, dt);
                    if let Some(mut s) = smoke {
                        s.amount = (s.amount / 2).max(3);
                        self.stream((key, 1), s, b.pos, dt);
                    }
                    self.light(b.pos, rgba(1.0, 0.62, 0.22, 1.0), 1.0 + 0.4 * size, 5.0);
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
                    let rock = linear3([c[0], c[1], c[2]]) * 0.09;
                    self.live.push(glow_sprite(p.pos, r * 2.8, Cell::Drop, rock).solid());
                    for j in 0..5 {
                        let a = j as f32 * 1.257 + self.time * 5.0;
                        let at = p.pos + Vec3::new(a.cos(), 0.3 * (j as f32 - 2.0), a.sin()) * r * 0.75;
                        let shade = 0.7 + 0.35 * (j % 3) as f32;
                        self.live.push(glow_sprite(at, r * 1.5, Cell::Drop, rock * shade).solid());
                    }
                    if matches!(e, El::Earth | El::Mud) {
                        let mut dust = particles(rgba(0.5, 0.44, 0.36, 1.0), 16, 0.7, 0.6);
                        dust.cell = Cell::Puff;
                        dust.glow = 0.0;
                        dust.ramp = Ramp::new(&[rgba(0.5, 0.44, 0.36, 0.0), rgba(0.5, 0.44, 0.36, 0.35), rgba(0.5, 0.44, 0.36, 0.0)], &[0.0, 0.2, 1.0]);
                        dust.dir = Vec3::Y;
                        dust.v = (0.2, 0.6);
                        dust.gravity = Vec3::ZERO;
                        self.stream((key, 1), dust, p.pos, dt);
                    }
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
                    // Lit like the floor under it: black in the dark, wet red by a torch.
                    let mut pool = flat_sprite(Vec3::new(g.pos.x, 0.03, g.pos.z), reach, Cell::Patch, self.lit_at(g.pos)).solid();
                    pool.effect = crate::effects::BLOOD;
                    let pull = (g.pull as f32).clamp(0.0, 0.999);
                    let suck = (g.slurp / spellstorm::pickups::GLOBE_SLURP).clamp(0.0, 1.0) as f32;
                    pool.params = [to.x, to.y, pull + (suck * 1000.0).floor(), t + (key % 97) as f32];
                    self.live.push(pool);
                }
            } else if let Ok(s) = w.ecs.get::<&Soul>(id) {
                let c = rgba(0.25, 0.5, 1.0, 1.0);
                let at = s.pos + Vec3::Y * (0.9 + 0.1 * (t * 4.0 + key as f32).sin());
                // A small glowing marble: a hard-edged bead with its sheen
                // (the Drop cell) and a bright heart, the faintest halo (Joep).
                self.live.push(glow_sprite(at, 0.2, Cell::Drop, linear3([0.35, 0.58, 1.0]) * 1.1));
                self.live.push(glow_sprite(at, 0.07, Cell::Spark, linear3([0.75, 0.88, 1.0]) * 1.6));
                self.halo(at, c, 0.4, 0.12);
            } else if let Ok(s) = w.ecs.get::<&Spray>(id) {
                let e = s.spell.primary.unwrap_or(El::Fire);
                let mut p = particles(el_c(e), 50, 0.45, 0.7);
                p.dir = s.dir;
                p.spread = 14.0;
                p.v = (8.0, 11.0);
                p.gravity = Vec3::ZERO;
                p.damp = (2.0, 4.0);
                let smoke = style(&mut p, e, true);
                if e == El::Fire {
                    // A flamethrower: a tight, fast jet from the staff whose
                    // flames swell as they burn out down the stream.
                    p.amount = 90;
                    p.spread = 8.0;
                    // As far as it burns (Joep): slowing by d m/s each
                    // second a flame goes v t - d t^2 / 2; speeds so they
                    // die out where the spell's range ends (less the swollen
                    // flame's own half size), 90% of their life in.
                    let reach = s.spell.n.range as f32 - 0.35;
                    let (d, t) = (3.0, 0.9 * p.life);
                    let v = (reach + d * t * t * 0.5) / t;
                    p.damp = (d, d);
                    p.v = (v * 0.92, v * 1.05);
                    p.curve = [0.35, 1.1, 1.7];
                    p.gravity = Vec3::Y * 1.5;
                }
                if matches!(e, El::Frost | El::Air) {
                    // Breath and gust: dense and as long as they reach, a cold
                    // or pale cloud rolling round them (a few sparkles were lame).
                    let reach = s.spell.n.range as f32 - 0.3;
                    let (d, life) = (3.0, 0.9 * p.life);
                    let v = (reach + d * life * life * 0.5) / life;
                    p.damp = (d, d);
                    p.v = (v * 0.9, v * 1.05);
                    p.amount = 140;
                    p.spread = 11.0;
                    if e == El::Frost {
                        p.size = 0.2;
                    }
                    let mut puff = p;
                    puff.amount = 30;
                    puff.cell = Cell::Puff;
                    puff.size = 0.8;
                    puff.glow = 0.16;
                    puff.spread = 15.0;
                    puff.curve = [0.5, 1.1, 1.8];
                    let k = lerp_c(el_c(e), WHITE, 0.6);
                    puff.ramp = Ramp::new(&[rgba(k[0], k[1], k[2], 0.0), rgba(k[0], k[1], k[2], 0.3), rgba(k[0], k[1], k[2], 0.0)], &[0.0, 0.3, 1.0]);
                    self.stream((key, 2), puff, s.at, dt);
                }
                if e == El::Water {
                    // A hose: a dense white jet of water that sags a little
                    // and breaks into spray at its end, not a blue haze (Joep).
                    p.amount = 160;
                    p.life = 0.4;
                    p.spread = 3.5;
                    p.v = (12.0, 15.0);
                    p.damp = (0.5, 1.5);
                    p.gravity = Vec3::NEG_Y * 5.0;
                    p.size = 0.16;
                    p.cell = Cell::Drop;
                    p.glow = 0.7;
                    p.solid = false;
                    p.ramp = Ramp::new(&[rgba(1.0, 1.0, 1.0, 1.0), rgba(0.9, 0.96, 1.0, 0.95), rgba(0.75, 0.88, 1.0, 0.0)], &[0.0, 0.7, 1.0]);
                    p.tint = (Vec3::ONE, Vec3::ONE);
                    p.curve = [0.7, 1.0, 1.6];
                    let mut mist = p;
                    mist.amount = 24;
                    mist.cell = Cell::Puff;
                    mist.size = 0.45;
                    mist.glow = 0.25;
                    mist.spread = 9.0;
                    mist.v = (8.0, 11.0);
                    mist.ramp = Ramp::new(&[rgba(1.0, 1.0, 1.0, 0.0), rgba(0.95, 0.98, 1.0, 0.3), rgba(0.9, 0.95, 1.0, 0.0)], &[0.0, 0.5, 1.0]);
                    self.stream((key, 2), mist, s.at, dt);
                    // Its body: spells.wgsl `jet` on a shaft along the spray,
                    // the drops and mist above flying off it.
                    let len = s.spell.n.range as f32;
                    let mut body = Sprite::shaft(s.at + s.dir * len * 0.5, [0.55, len], s.dir, IMAGE, uv(Cell::Patch), Vec3::splat(0.7));
                    body.additive = false;
                    body.depth_bias = 0.3;
                    body.effect = crate::effects::JET;
                    body.params = [t, (key % 97) as f32, 0.0, 0.0];
                    self.live.push(body);
                }
                // Two or three of it: a bigger, denser, wider stream.
                let h = heft(&s.spell);
                p.amount = (p.amount as f32 * h) as u32;
                p.size *= 1.0 + 0.3 * (h - 1.0);
                p.spread *= 1.0 + 0.4 * (h - 1.0);
                // Its other elements burn, drip or crackle through it.
                for (i, (e2, n2)) in others(&s.spell).into_iter().enumerate() {
                    let mut q = particles(el_c(e2), ((p.amount as f32) * 0.35 * n2 as f32) as u32 + 6, p.life, p.size);
                    let sm2 = style(&mut q, e2, false);
                    let _ = sm2;
                    q.dir = p.dir;
                    q.spread = p.spread * 1.2;
                    q.v = p.v;
                    q.damp = p.damp;
                    self.stream((key, 3 + i as u8), q, s.at, dt);
                }
                self.stream((key, 0), p, s.at, dt);
                if let Some(mut sm) = smoke {
                    sm.amount = (sm.amount as f32 * h) as u32;
                    self.stream((key, 1), sm, s.at, dt);
                }
                let h_light = h;
                if e == El::Fire {
                    // A flamethrower floods its surroundings: brighter, wider
                    // and warmer (orange-yellow) than other sprays, and lit
                    // along the jet as well as at the staff.
                    let warm = rgba(1.0, 0.62, 0.22, 1.0);
                    self.light(s.at + s.dir * 1.0, warm, 3.2 * h_light, 11.0 + 3.0 * (h_light - 1.0));
                    self.light(s.at + s.dir * 4.0, warm, 2.4 * h_light, 9.0 + 3.0 * (h_light - 1.0));
                } else {
                    self.light(s.at + s.dir * 2.0, el_c(e), 1.2 * h_light, 6.0);
                }
            } else if let Ok(b) = w.ecs.get::<&Beam>(id) {
                let e = b.spell.primary.unwrap_or(El::Lightning);
                let c = el_c(e);
                let c3 = linear3([c[0], c[1], c[2]]);
                let d = b.to - b.from;
                let len = d.length();
                if len > 0.05 {
                    // Its element crawling along it, not only a thin line.
                    let mut q = particles(c, 24, 0.35, 0.55);
                    let _ = style(&mut q, e, false);
                    q.dir = Vec3::Y;
                    q.spread = 180.0;
                    q.v = (0.3, 1.2);
                    q.sphere = 0.22;
                    q.gravity = Vec3::ZERO;
                    for i in 0..6u8 {
                        let k = (i as f32 + 0.5) / 6.0;
                        self.stream((key, 30 + i), q, b.from.lerp(b.to, k), dt);
                    }
                }
                let h = heft(&b.spell);
                if len > 0.05 {
                    let mid = (b.from + b.to) * 0.5;
                    let dir = d / len;
                    let wob = 1.0 + 0.15 * (t * 30.0).sin();
                    // Joep: a beam's strength trembles, never one steady
                    // glow: a few unrelated waves and a quick flutter.
                    let ph = (key % 97) as f32;
                    let flick = (0.82 + 0.1 * (t * 7.3 + ph).sin() + 0.06 * (t * 17.9 + ph * 2.3).sin() + 0.08 * (t * 41.0 + ph * 0.7).sin().abs()).clamp(0.6, 1.1);
                    let wob = wob * (0.92 + 0.08 * flick);
                    // Mixed, its colour runs between them (poison fire: a
                    // sickly green-orange, not plain green).
                    let total: f32 = b.spell.counts.iter().map(|c| c.1 as f32).sum::<f32>().max(1.0);
                    let mut mixed = c3 * (b.spell.count(e) as f32).max(1.0);
                    for (e2, n2) in others(&b.spell) {
                        let c2 = el_c(e2);
                        mixed += linear3([c2[0], c2[1], c2[2]]) * n2 as f32;
                    }
                    let c3 = c3.lerp(mixed / total, 0.6);
                    self.live.push(Sprite::shaft(mid, [0.35 * wob * h, len], dir, IMAGE, uv(Cell::Line), c3 * 2.0 * flick));
                    self.live.push(Sprite::shaft(mid, [0.1 * h * (0.9 + 0.1 * flick), len], dir, IMAGE, uv(Cell::Line), Vec3::ONE * 3.0 * flick));
                    // Its other elements twist round it and fly off it all
                    // along: a poison beam with fire in it burns green and
                    // orange, flames licking up off it.
                    for (i, (e2, n2)) in others(&b.spell).into_iter().enumerate() {
                        let c2 = el_c(e2);
                        let w2 = (0.55 + 0.15 * n2 as f32) * h * (1.0 + 0.3 * (t * 23.0 + i as f32 * 2.1).sin());
                        self.live.push(Sprite::shaft(mid, [w2, len], dir, IMAGE, uv(Cell::Line), linear3([c2[0], c2[1], c2[2]]) * 1.4 * flick));
                        let mut q = particles(c2, (16.0 * n2 as f32 * h) as u32 + 10, 0.5, 0.45 * h);
                        let _ = style(&mut q, e2, false);
                        q.dir = Vec3::Y;
                        q.spread = 60.0;
                        q.v = (0.4, 1.4);
                        q.sphere = 0.16 * h;
                        let pieces = ((len / 1.5) as usize).clamp(1, 8);
                        for j in 0..pieces {
                            let at = b.from + d * ((j as f32 + 0.5) / pieces as f32);
                            self.stream((key, 10 + (i * 8 + j) as u8), q, at, dt);
                        }
                    }
                }
                let ph = (key % 97) as f32;
                let tremble = (0.85 + 0.1 * (t * 7.3 + ph).sin() + 0.08 * (t * 23.0 + ph).sin()).clamp(0.65, 1.05);
                self.halo(b.to, c, 1.4 * h, 0.8 * tremble);
                self.light(b.to + Vec3::Y * 0.3, c, 1.5 * h * tremble, 5.0);
                // Where it burns in: its element spits off the spot.
                let mut q = particles(c, (30.0 * h) as u32 + 10, 0.35, 0.35 * h);
                let _ = style(&mut q, e, false);
                q.dir = Vec3::Y - d.normalize_or_zero() * 0.6;
                q.spread = 70.0;
                q.v = (1.5, 4.0);
                self.stream((key, 60), q, b.to, dt);
            } else if let Ok(tr) = w.ecs.get::<&Trap>(id) {
                if !tr.done {
                    let e = tr.spell.primary.unwrap_or(El::Fire);
                    let c = el_c(e);
                    let arm = (tr.t as f32 / spellstorm::spells::TRAP_ARM as f32).min(1.0);
                    // A circle of runes worn into the floor, lit from below in
                    // its element's colour and breathing (spells.wgsl trap).
                    let r = tr.trigger_r() as f32;
                    let h = heft(&tr.spell);
                    let mut s = flat_sprite(tr.pos + Vec3::Y * 0.05, r, Cell::Patch, linear3([c[0], c[1], c[2]]) * 1.6 * h).solid();
                    s.effect = crate::effects::TRAP;
                    s.params = [arm, (key % 97) as f32 * 0.37, t, 0.0];
                    self.live.push(s);
                }
            } else if let Ok(s) = w.ecs.get::<&Sentry>(id) {
                let e = s.spell.primary.unwrap_or(El::Fire);
                let c = el_c(e);
                let at = s.pos + Vec3::Y * 1.2;
                let h = heft(&s.spell);
                self.live.push(glow_sprite(at, 0.4 * h, Cell::Spark, linear3([c[0], c[1], c[2]]) * 2.5));
                self.halo(at, c, 1.6 * h, 0.6);
                self.light(at, c, 0.8 * h, 4.0 + 1.5 * (h - 1.0));
                for (i, (e2, _)) in others(&s.spell).into_iter().enumerate() {
                    let c2 = el_c(e2);
                    let a = t * 2.5 + i as f32 * 2.1;
                    let orb = at + Vec3::new(a.cos(), 0.15 * (a * 2.0).sin(), a.sin()) * 0.45 * h;
                    self.live.push(glow_sprite(orb, 0.18 * h, Cell::Spark, linear3([c2[0], c2[1], c2[2]]) * 2.0));
                }
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
                    // Magma: fx.wgsl `ground`'s molten seams. Fire is only
                    // its flames, no scorched patch (Joep).
                    if f.kind == El::Magma {
                        let mut s = flat_sprite(at, r, Cell::Patch, Vec3::splat(1.2)).solid();
                        s.opacity = fade;
                        s.effect = crate::effects::GROUND;
                        s.params = [f.kind.idx() as f32, r, (key % 97) as f32, t];
                        self.live.push(s);
                    }
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
                    // A steam cloud keeps rolling up off it (render::fx smoke).
                    if f.kind == El::Steam && self.f() < dt / 1.3 {
                        let q = at + Vec3::new(self.range(-0.5, 0.5), 0.0, self.range(-0.5, 0.5)) * r;
                        self.smoke(q, 0.5 + r * 0.45, [0.9, 0.92, 0.95], 0.45 * fade);
                    }
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
                    // A poison pool breathes out a low green cloud.
                    if f.kind == El::Poison && self.f() < dt / 1.6 {
                        let q = at + Vec3::new(self.range(-0.5, 0.5), 0.0, self.range(-0.5, 0.5)) * r;
                        self.smoke(q, 0.4 + r * 0.4, [0.36, 0.58, 0.16], 0.35 * fade);
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
            // fx.wgsl gets a seed that stays with the particle (from its own
            // random life and size, not where it is) and its age 0..1;
            // effects::tag keeps both (params.w = 1).
            sp.params = [(p.life * 91.7 + p.size * 37.3).fract() * 97.0, 0.0, k, 1.0];
            if e.glow > 0.0 {
                sp.additive = true;
                sp.radiance = col * e.glow;
                sp.opacity = c[3];
            } else if e.lit {
                // Blood: its colour as an albedo under the light round it.
                sp.radiance = col * self.lit_at(p.pos);
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
