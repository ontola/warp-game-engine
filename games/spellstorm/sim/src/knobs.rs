//! The balance and look numbers that can be turned while the game runs
//! (mods/base/knobs.luau, a `-- @knobs` block): each is a function
//! returning its value now. The file is the only copy of the defaults; the
//! app watches it and `spellstorm play --live` sets them with set_knob,
//! trial, keep and revert. Everything else stays a constant in the code.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The shipped knobs file, for runs outside the repository and the tests.
pub const BUILT_IN: &str = include_str!("../../mods/base/knobs.luau");

macro_rules! knobs {
    ($($name:ident;)*) => {
        /// Every knob the sim reads, by name.
        pub const NAMES: &[&str] = &[$(stringify!($name)),*];
        static VALUES: [AtomicU64; NAMES.len()] = [$({ let _ = stringify!($name); AtomicU64::new(UNSET) },)*];
        $(#[doc = concat!("The knob `", stringify!($name), "` (mods/base/knobs.luau).")] pub fn $name() -> f64 { get(index(stringify!($name)).unwrap()) })*
    };
}

knobs! {
    wizard_speed;
    roll_speed;
    roll_cooldown;
    swing_recover;
    swing_reach;
    mana_regen;
    spell_damage;
    flash_brightness;
    flash_light;
    number_size;
    shadow_softness;
    monster_hp;
    monster_damage;
    monster_count;
    monster_speed;
    wave_gap;
    first_wave;
    first_calm;
    revive_need;
    revive_rate;
    tome_time;
    wards;
    ward_needs_tomes;
    practice;
}

/// A value not set since the last reset: the built-in default answers.
const UNSET: u64 = u64::MAX;

/// Each knob's default and help, from BUILT_IN, in NAMES order.
fn defaults() -> &'static [(f64, String)] {
    static D: OnceLock<Vec<(f64, String)>> = OnceLock::new();
    D.get_or_init(|| {
        let parsed = script::knobs::parse(BUILT_IN);
        NAMES.iter().map(|n| {
            let k = parsed.iter().find(|k| k.name == *n).unwrap_or_else(|| panic!("mods/base/knobs.luau has no knob {n}"));
            (k.number().unwrap_or_else(|| panic!("mods/base/knobs.luau: {n} is not a number")), k.about.clone())
        }).collect()
    })
}

pub fn index(name: &str) -> Option<usize> {
    NAMES.iter().position(|k| *k == name)
}

fn get(i: usize) -> f64 {
    match VALUES[i].load(Ordering::Relaxed) {
        UNSET => defaults()[i].0,
        v => f64::from_bits(v),
    }
}

/// The value of knob `name` now.
pub fn value(name: &str) -> Option<f64> {
    index(name).map(get)
}

/// The shipped value of knob `name`.
pub fn default(name: &str) -> Option<f64> {
    index(name).map(|i| defaults()[i].0)
}

/// What knob `name` does (its comment in the file).
pub fn about(name: &str) -> Option<&'static str> {
    index(name).map(|i| defaults()[i].1.as_str())
}

/// Set knob `name`; false when there is no such knob or the value is not finite.
pub fn set(name: &str, v: f64) -> bool {
    match index(name) {
        Some(i) if v.is_finite() => {
            VALUES[i].store(v.to_bits(), Ordering::Relaxed);
            true
        }
        _ => false,
    }
}

/// Every knob back to its default.
pub fn reset() {
    for v in &VALUES {
        v.store(UNSET, Ordering::Relaxed);
    }
}

/// Apply a knobs file (the `-- @knobs` block of mods/base/knobs.luau).
/// Returns the problems (unknown names, values that are not numbers) as
/// messages; the good lines apply anyway, knobs it leaves out keep their
/// default.
pub fn load(text: &str) -> Vec<String> {
    reset();
    let mut errors = vec![];
    for k in script::knobs::parse(text) {
        match k.number() {
            Some(x) if set(&k.name, x) => {}
            Some(_) => errors.push(format!("no knob called {}", k.name)),
            None => errors.push(format!("{}: {} is not a number", k.name, k.value)),
        }
    }
    errors
}
