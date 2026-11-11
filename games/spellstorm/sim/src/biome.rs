//! The biomes of a run, one level each (Arena.BIOMES), read from
//! mods/base/biomes.luau. Colours are Godot colours (sRGB), as the Godot
//! game wrote them.

use crate::el::El;
use std::sync::atomic::{AtomicPtr, Ordering};

pub type Rgb = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallStyle {
    Masonry,
    Ossuary,
    Rock,
    Castle,
    Cliff,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Biome {
    pub id: &'static str,
    pub name: &'static str,
    pub moss: f32,
    pub moss_c: Rgb,
    /// The props of each of the three zones along the level.
    pub zones: [&'static [&'static str]; 3],
    pub floor_a: Rgb,
    pub floor_b: Rgb,
    pub tiles: f32,
    pub wet: f32,
    pub wall: Rgb,
    pub fog: Rgb,
    pub ambient: Rgb,
    pub moon: Rgb,
    pub moon_energy: f32,
    pub mist: f32,
    pub mist_c: Rgb,
    pub props: &'static [&'static str],
    /// Monster kinds and how many of each go in a wave's pool.
    pub enemies: &'static [(&'static str, u32)],
    pub boss: &'static str,
    /// Permanent fields laid about the level, a set every 9 m of its length.
    pub fields: &'static [El],
    pub path_style: i32,
    pub path_a: Rgb,
    pub path_b: Rgb,
    /// "nave": a built biome with a straight middle stretch.
    pub nave: bool,
    pub corridor: bool,
    pub outside: &'static [&'static str],
    pub wall_style: WallStyle,
    pub wall_props: &'static [&'static str],
    pub wall_every: f32,
    pub isles: bool,
    pub low_walls: bool,
    pub water_props: &'static [&'static str],
    pub density: f32,
    pub outside_lava: bool,
}

/// The biomes of a run, in order (mods/base/biomes.luau, or the list a mod
/// installed).
pub fn biomes() -> &'static [Biome] {
    let p = INSTALLED.load(Ordering::Acquire);
    if p.is_null() { crate::rules::builtin_biomes() } else { unsafe { &*p } }
}

static INSTALLED: AtomicPtr<Vec<Biome>> = AtomicPtr::new(std::ptr::null_mut());

/// The biomes from now on; None goes back to the built-in ones. Each
/// installed list is leaked, so a level built from the one before stays
/// valid.
pub fn set_biomes(b: Option<Vec<Biome>>) {
    let p = b.map(|b| Box::into_raw(Box::new(b))).unwrap_or(std::ptr::null_mut());
    INSTALLED.store(p, Ordering::Release);
}

/// How the ground shifts from zone to zone (Arena.ZONE_TINTS).
pub const ZONE_TINTS: [Rgb; 3] = [[1.0, 1.0, 1.0], [0.82, 0.92, 0.8], [1.08, 0.93, 0.88]];

/// Godot's `Color.darkened(k)`.
pub fn darkened(c: Rgb, k: f32) -> Rgb {
    [c[0] * (1.0 - k), c[1] * (1.0 - k), c[2] * (1.0 - k)]
}

/// Godot's `Color.lightened(k)`.
pub fn lightened(c: Rgb, k: f32) -> Rgb {
    [
        c[0] + (1.0 - c[0]) * k,
        c[1] + (1.0 - c[1]) * k,
        c[2] + (1.0 - c[2]) * k,
    ]
}

