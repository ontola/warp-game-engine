//! The balance numbers that can be turned while the game runs: each is a
//! function returning its value now (its default until a knobs file or the
//! live tools set it). The app reads `knobs.txt` (`name = value  # what it
//! does`), and `spellstorm play --live` sets them with set_knob, trial,
//! keep and revert. Everything else stays a constant in the code.
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! knobs {
    ($($(#[doc = $doc:literal])* $name:ident = $default:expr;)*) => {
        /// Every knob: name, default and what it does.
        pub const LIST: &[(&str, f64, &str)] = &[$((stringify!($name), $default, concat!($($doc),*)),)*];
        static VALUES: [AtomicU64; LIST.len()] = [$(AtomicU64::new(f64::to_bits($default)),)*];
        $($(#[doc = $doc])* pub fn $name() -> f64 { get(index(stringify!($name)).unwrap()) })*
    };
}

knobs! {
    /// Wizard walking speed, m/s.
    wizard_speed = 5.2;
    /// Dodge roll speed at its push off, m/s.
    roll_speed = 12.5;
    /// Seconds between two rolls.
    roll_cooldown = 0.95;
    /// Seconds after a sword swing before the next one.
    swing_recover = 0.7;
    /// The sword's reach, m.
    swing_reach = 1.7;
    /// Mana back per second.
    mana_regen = 3.0;
    /// Scales the damage of every wizard spell, bolt and sword hit.
    spell_damage = 1.0;
    /// Scales how bright the glow of a hit, shot or burst is (looks only).
    flash_brightness = 0.5;
    /// Scales how much light a hit, shot or burst throws around (looks only).
    flash_light = 0.5;
    /// Scales monster health (on top of the difficulty setting).
    monster_hp = 1.0;
    /// Scales the damage monsters do (on top of the difficulty setting).
    monster_damage = 1.0;
    /// Scales how many monsters a wave brings (on top of the difficulty setting).
    monster_count = 1.0;
    /// Scales monster walking speed.
    monster_speed = 1.0;
    /// Metres of path between two waves.
    wave_gap = 30.0;
    /// Metres before the first wave of a level.
    first_wave = 8.0;
    /// Metres before the first wave of a run's first level (a calm start).
    first_calm = 22.0;
    /// Revive progress needed to raise a fallen wizard.
    revive_need = 40.0;
    /// Revive progress per second for each friend at the grave.
    revive_rate = 14.0;
    /// Seconds standing in an altar's ring to take its tome.
    tome_time = 0.9;
    /// 1: walls of mist hold the way on until the fight there is done; 0: no walls.
    wards = 1.0;
    /// 1: a shrine's wall opens once every wizard took a tome; 0: as soon as the shrine rises.
    ward_needs_tomes = 1.0;
    /// 1: players who know nothing start on the practice strip; 0: never
    /// (the default until its mist walls look as good as in Godot).
    practice = 0.0;
}

pub fn index(name: &str) -> Option<usize> {
    LIST.iter().position(|k| k.0 == name)
}

fn get(i: usize) -> f64 {
    f64::from_bits(VALUES[i].load(Ordering::Relaxed))
}

/// The value of knob `name` now.
pub fn value(name: &str) -> Option<f64> {
    index(name).map(get)
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
    for (i, k) in LIST.iter().enumerate() {
        VALUES[i].store(k.1.to_bits(), Ordering::Relaxed);
    }
}

/// Every knob with its value now, as `name = value  # what it does` lines.
pub fn file_text() -> String {
    let mut s = String::from("# Spellstorm's live knobs: `name = value  # what it does`. Read at start\n# and whenever this file changes; `spellstorm play --live` sets them too.\n");
    for (i, k) in LIST.iter().enumerate() {
        s += &format!("{} = {}  # {}\n", k.0, get(i), k.2.trim());
    }
    s
}

/// Apply a knobs file: every known `name = value` line. Returns the
/// problems (unknown names, values that do not parse) as messages; the
/// good lines are applied anyway, knobs it leaves out keep their default.
pub fn load(text: &str) -> Vec<String> {
    reset();
    let mut errors = vec![];
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, v)) = line.split_once('=') else {
            errors.push(format!("line {}: not `name = value`", n + 1));
            continue;
        };
        let (name, v) = (name.trim(), v.trim());
        match v.parse::<f64>() {
            Ok(x) if set(name, x) => {}
            Ok(_) => errors.push(format!("line {}: no knob called {name}", n + 1)),
            Err(_) => errors.push(format!("line {}: {v} is not a number", n + 1)),
        }
    }
    errors
}
