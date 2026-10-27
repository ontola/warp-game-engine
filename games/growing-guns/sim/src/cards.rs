//! Cards that grow the gun, after Godot's `card_library.gd` (the stat
//! cards and explosive rounds; ricochet, shotgun and the specials are not
//! ported). Losers of a
//! round pick one of three; effects stack for the rest of the match.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Card { Damage, Healthy, SteadyHands, Precision, Sneakers, RapidFire, FastRounds, BigMag, QuickReload, Explosive }

pub const ALL: [Card; 10] = [Card::Damage, Card::Healthy, Card::SteadyHands, Card::Precision, Card::Sneakers, Card::RapidFire, Card::FastRounds, Card::BigMag, Card::QuickReload, Card::Explosive];

impl Card {
    pub fn name(self) -> &'static str {
        match self {
            Card::Damage => "DAMAGE", Card::Healthy => "HEALTHY", Card::SteadyHands => "STEADY HANDS", Card::Precision => "PRECISION",
            Card::Sneakers => "SNEAKERS", Card::RapidFire => "RAPID FIRE", Card::FastRounds => "FAST ROUNDS", Card::BigMag => "BIG MAG", Card::QuickReload => "QUICK RELOAD", Card::Explosive => "EXPLOSIVE ROUNDS",
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Card::Damage => "x1.5 damage", Card::Healthy => "+35 max health", Card::SteadyHands => "Tighter spread", Card::Precision => "Tighter spread, half recoil",
            Card::Sneakers => "+15% move speed", Card::RapidFire => "x1.6 fire rate", Card::FastRounds => "x1.5 bullet speed", Card::BigMag => "+20 rounds", Card::QuickReload => "x2 reload speed", Card::Explosive => "Bullets explode on impact",
        }
    }
    /// sRGB menu colour, as in the Godot pool.
    pub fn color(self) -> [f32; 3] {
        match self {
            Card::Damage => [1.0, 0.25, 0.25], Card::Healthy => [0.45, 1.0, 0.55], Card::SteadyHands | Card::Precision => [0.55, 0.8, 1.0],
            Card::Sneakers => [0.9, 0.9, 0.4], Card::RapidFire => [1.0, 0.6, 0.2], Card::FastRounds => [0.65, 0.92, 1.0], Card::BigMag | Card::QuickReload => [0.85, 0.7, 1.0], Card::Explosive => [1.0, 0.4, 0.1],
        }
    }
    /// Improves the gun itself (one such card is always offered).
    pub fn firepower(self) -> bool { !matches!(self, Card::Healthy | Card::Sneakers) }
}

/// Accumulated card effects for one player.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mods {
    pub damage: f32,
    pub fire_rate: f32,
    pub spread: f32,
    pub recoil: f32,
    pub speed: f32,
    pub bullet_speed: f32,
    pub reload: f32,
    pub mag_bonus: u32,
    pub hp_bonus: i32,
    /// Splash radius in metres (0: no explosion) and damage at the centre.
    #[serde(default)]
    pub explosive_radius: f32,
    #[serde(default)]
    pub explosive_damage: f32,
}

impl Default for Mods {
    fn default() -> Self { Mods { damage: 1.0, fire_rate: 1.0, spread: 1.0, recoil: 1.0, speed: 1.0, bullet_speed: 1.0, reload: 1.0, mag_bonus: 0, hp_bonus: 0, explosive_radius: 0.0, explosive_damage: 0.0 } }
}

impl Mods {
    pub fn apply(&mut self, c: Card) {
        match c {
            Card::Damage => self.damage *= 1.5,
            Card::Healthy => self.hp_bonus += 35,
            Card::SteadyHands => self.spread *= 0.65,
            Card::Precision => { self.spread *= 0.4; self.recoil *= 0.5; }
            Card::Sneakers => self.speed *= 1.15,
            Card::RapidFire => self.fire_rate *= 1.6,
            Card::FastRounds => self.bullet_speed *= 1.5,
            Card::BigMag => self.mag_bonus += 20,
            Card::QuickReload => self.reload *= 2.0,
            // First stack gives a sizeable blast, later ones grow it.
            Card::Explosive => {
                self.explosive_radius += if self.explosive_radius <= 0.0 { 4.0 } else { 2.5 };
                self.explosive_damage += 25.0;
            }
        }
    }
}

/// Three different cards, at least one firepower card.
pub fn offer(rng: &mut crate::rng::Rng) -> [Card; 3] {
    let mut pool = ALL.to_vec();
    let mut out = [Card::Damage; 3];
    for o in &mut out { *o = pool.remove(rng.below(pool.len() as u32) as usize); }
    if !out.iter().any(|c| c.firepower()) {
        let fp: Vec<Card> = ALL.iter().copied().filter(|c| c.firepower()).collect();
        out[rng.below(3) as usize] = fp[rng.below(fp.len() as u32) as usize];
    }
    out
}
