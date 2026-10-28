//! Cards that grow the gun. Losers of a round pick one of three; effects
//! stack for the rest of the match. The cards themselves are data (the game
//! loads them from `mods/*.luau`, see the app's `scripts` module): a name,
//! a text, a colour and stat changes the sim applies in f32, plus an
//! optional script hook for anything a table of numbers cannot say.
use serde::{Deserialize, Serialize};

/// A card: an index into the match's [`Book`]. Stable across hot reloads
/// (new cards append, removed ones are switched off, never renumbered).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card(pub u16);

/// A stat a card can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stat { Damage, FireRate, Spread, Recoil, Speed, BulletSpeed, Reload, MagBonus, HpBonus, ExplosiveRadius, ExplosiveDamage }

pub const STATS: [(Stat, &str, &str); 11] = [
    (Stat::Damage, "damage", "bullet damage multiplier"),
    (Stat::FireRate, "fire_rate", "shots per second multiplier"),
    (Stat::Spread, "spread", "spread cone multiplier (lower is tighter)"),
    (Stat::Recoil, "recoil", "recoil multiplier"),
    (Stat::Speed, "speed", "move speed multiplier"),
    (Stat::BulletSpeed, "bullet_speed", "bullet speed multiplier"),
    (Stat::Reload, "reload", "reload speed multiplier"),
    (Stat::MagBonus, "mag_bonus", "extra rounds per magazine"),
    (Stat::HpBonus, "hp_bonus", "extra max health"),
    (Stat::ExplosiveRadius, "explosive_radius", "bullet blast radius in metres (0: no blast)"),
    (Stat::ExplosiveDamage, "explosive_damage", "blast damage at the centre"),
];

impl Stat {
    pub fn named(name: &str) -> Option<Stat> { STATS.iter().find(|s| s.1 == name).map(|s| s.0) }
    pub fn name(self) -> &'static str { STATS.iter().find(|s| s.0 == self).map_or("?", |s| s.1) }
}

/// One card's definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardDef {
    /// Stable id, e.g. `rapid_fire`.
    pub id: String,
    pub name: String,
    pub text: String,
    /// sRGB menu colour.
    pub color: [f32; 3],
    /// Improves the gun itself (one such card is always offered).
    pub firepower: bool,
    /// Applied in order on pick: multiply, then add.
    pub mul: Vec<(Stat, f32)>,
    pub add: Vec<(Stat, f32)>,
    /// Still in the mods (a removed card stays numbered but is not offered).
    pub enabled: bool,
}

/// The match's cards.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book { cards: Vec<CardDef> }

impl Book {
    pub fn new(cards: Vec<CardDef>) -> Self { Book::default().merged(cards) }

    pub fn get(&self, c: Card) -> Option<&CardDef> { self.cards.get(c.0 as usize) }
    pub fn name(&self, c: Card) -> &str { self.get(c).map_or("?", |d| d.name.as_str()) }
    pub fn find(&self, id: &str) -> Option<Card> { self.cards.iter().position(|d| d.id == id && d.enabled).map(|i| Card(i as u16)) }
    /// Cards that can be offered and given, in book order.
    pub fn enabled(&self) -> impl Iterator<Item = (Card, &CardDef)> {
        self.cards.iter().enumerate().filter(|(_, d)| d.enabled).map(|(i, d)| (Card(i as u16), d))
    }

    /// This book with `defs` as the new set: known ids keep their number,
    /// new ones append, ids not in `defs` are switched off.
    pub fn merged(&self, defs: Vec<CardDef>) -> Book {
        let mut cards = self.cards.clone();
        for c in &mut cards { c.enabled = false; }
        for mut d in defs {
            d.enabled = true;
            match cards.iter().position(|c| c.id == d.id) { Some(i) => cards[i] = d, None => cards.push(d) }
        }
        Book { cards }
    }

    /// The card's stat changes.
    pub fn apply(&self, c: Card, m: &mut Mods) {
        let Some(d) = self.get(c) else { return };
        for &(s, k) in &d.mul { m.set(s, m.get(s) * k); }
        for &(s, k) in &d.add { m.set(s, m.get(s) + k); }
    }

    /// Three different cards, at least one firepower card; `None` with
    /// fewer than three cards.
    pub fn offer(&self, rng: &mut crate::rng::Rng) -> Option<[Card; 3]> {
        let mut pool: Vec<Card> = self.enabled().map(|(c, _)| c).collect();
        if pool.len() < 3 { return None; }
        let mut out = [Card(0); 3];
        for o in &mut out { *o = pool.remove(rng.below(pool.len() as u32) as usize); }
        let fp = |c: &Card| self.get(*c).is_some_and(|d| d.firepower);
        if !out.iter().any(fp) {
            let all: Vec<Card> = self.enabled().map(|(c, _)| c).filter(fp).collect();
            if !all.is_empty() { out[rng.below(3) as usize] = all[rng.below(all.len() as u32) as usize]; }
        }
        Some(out)
    }
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
    pub fn get(&self, s: Stat) -> f32 {
        match s {
            Stat::Damage => self.damage, Stat::FireRate => self.fire_rate, Stat::Spread => self.spread, Stat::Recoil => self.recoil,
            Stat::Speed => self.speed, Stat::BulletSpeed => self.bullet_speed, Stat::Reload => self.reload,
            Stat::MagBonus => self.mag_bonus as f32, Stat::HpBonus => self.hp_bonus as f32,
            Stat::ExplosiveRadius => self.explosive_radius, Stat::ExplosiveDamage => self.explosive_damage,
        }
    }
    /// Multipliers stay positive; the counts round.
    pub fn set(&mut self, s: Stat, v: f32) {
        let pos = v.max(0.01);
        match s {
            Stat::Damage => self.damage = pos, Stat::FireRate => self.fire_rate = pos, Stat::Spread => self.spread = v.max(0.0), Stat::Recoil => self.recoil = v.max(0.0),
            Stat::Speed => self.speed = pos, Stat::BulletSpeed => self.bullet_speed = pos, Stat::Reload => self.reload = pos,
            Stat::MagBonus => self.mag_bonus = v.round().clamp(0.0, 10_000.0) as u32, Stat::HpBonus => self.hp_bonus = v.round().clamp(-90.0, 100_000.0) as i32,
            Stat::ExplosiveRadius => self.explosive_radius = v.clamp(0.0, 40.0), Stat::ExplosiveDamage => self.explosive_damage = v.max(0.0),
        }
    }
}

#[cfg(test)]
pub(crate) fn test_book() -> Book {
    let c = |id: &str, firepower, mul: Vec<(Stat, f32)>, add: Vec<(Stat, f32)>| CardDef { id: id.into(), name: id.to_uppercase(), text: String::new(), color: [1.0; 3], firepower, mul, add, enabled: true };
    Book::new(vec![
        c("damage", true, vec![(Stat::Damage, 1.5)], vec![]),
        c("healthy", false, vec![], vec![(Stat::HpBonus, 35.0)]),
        c("sneakers", false, vec![(Stat::Speed, 1.15)], vec![]),
        c("big_mag", true, vec![], vec![(Stat::MagBonus, 20.0)]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_keeps_numbers_and_switches_off_removed_cards() {
        let b = test_book();
        let mag = b.find("big_mag").unwrap();
        let mut defs: Vec<CardDef> = b.enabled().map(|(_, d)| d.clone()).filter(|d| d.id != "healthy").collect();
        defs.push(CardDef { id: "new".into(), ..defs[0].clone() });
        let b2 = b.merged(defs);
        assert_eq!(b2.find("big_mag"), Some(mag));
        assert_eq!(b2.find("healthy"), None);
        assert_eq!(b2.find("new"), Some(Card(4)));
        assert_eq!(b2.name(Card(1)), "HEALTHY", "a removed card still has its name");
    }

    #[test]
    fn stats_apply_in_f32() {
        let b = test_book();
        let mut m = Mods::default();
        b.apply(b.find("sneakers").unwrap(), &mut m);
        assert_eq!(m.speed, 1.0f32 * 1.15f32);
        b.apply(b.find("big_mag").unwrap(), &mut m);
        assert_eq!(m.mag_bonus, 20);
    }
}
