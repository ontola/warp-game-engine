//! Cards (Godot `card_library.gd`). The pool is data: the game loads it
//! from `mods/*.luau`, where each card mutates the player's `Weapon` (a
//! script hook, `Hooks::apply_card`) and/or lists field multipliers and
//! additions the sim applies itself. Losers of a round pick one of three,
//! rare cards weighted down and the trailing players' odds up.
use crate::weapon::{Value, Weapon};
use serde::{Deserialize, Serialize};

/// A card: an index into the match's [`Book`]. Stable across hot reloads
/// (new cards append, removed ones are switched off, never renumbered).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card(pub u16);

/// Godot `_rarity_weight`: rare cards come up 0.15 as often.
pub const RARE_WEIGHT: f32 = 0.15;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardDef {
    /// Stable id, e.g. `rapid_fire`.
    pub id: String,
    pub name: String,
    pub desc: String,
    /// sRGB menu colour.
    pub color: [f32; 3],
    /// Improves the gun itself (one such card is always offered).
    pub firepower: bool,
    pub rare: bool,
    /// Weapon fields multiplied, then added to, on pick (f32).
    pub mul: Vec<(String, f32)>,
    pub add: Vec<(String, f32)>,
    /// Still in the mods (a removed card stays numbered but is not offered).
    pub enabled: bool,
}

impl CardDef {
    pub fn new(id: &str, name: &str) -> Self {
        CardDef { id: id.into(), name: name.into(), desc: String::new(), color: [1.0; 3], firepower: false, rare: false, mul: Vec::new(), add: Vec::new(), enabled: true }
    }
    fn weight(&self, score_factor: f32) -> f32 {
        // Repeatable baseline gun growth should be easier to build around.
        let basic = if matches!(self.id.as_str(), "dmg_up" | "big_mag" | "rapid_fire") { 3.0 } else { 1.0 };
        basic * if self.rare { RARE_WEIGHT * score_factor } else { 1.0 }
    }
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

    /// The card's data part: multiply, then add (unknown fields are skipped;
    /// the mods loader reports them).
    pub fn apply(&self, c: Card, w: &mut Weapon) {
        let Some(d) = self.get(c) else { return };
        for (f, k) in &d.mul { if let Some(Value::Num(v)) = w.get(f) { let _ = w.set(f, Value::Num(v * k)); } }
        for (f, k) in &d.add { if let Some(Value::Num(v)) = w.get(f) { let _ = w.set(f, Value::Num(v + k)); } }
    }

    /// Godot `random_ids`: `count` different cards, weighted without
    /// replacement, then the pity slot: with no firepower card in the draw
    /// one random slot becomes a (weighted) firepower card.
    pub fn offer(&self, rng: &mut crate::rng::Rng, score_factor: f32, count: usize) -> Vec<Card> {
        let mut pool: Vec<Card> = self.enabled().map(|(c, _)| c).collect();
        let mut out = Vec::new();
        while out.len() < count && !pool.is_empty() {
            let k = self.weighted(&pool, rng, score_factor);
            out.push(pool.remove(k));
        }
        let fp = |c: &Card| self.get(*c).is_some_and(|d| d.firepower);
        if !out.is_empty() && !out.iter().any(fp) {
            let fps: Vec<Card> = self.enabled().map(|(c, _)| c).filter(|c| fp(c) && !out.contains(c)).collect();
            if !fps.is_empty() {
                let slot = rng.below(out.len() as u32) as usize;
                out[slot] = fps[self.weighted(&fps, rng, score_factor)];
            }
        }
        out
    }

    fn weighted(&self, pool: &[Card], rng: &mut crate::rng::Rng, f: f32) -> usize {
        let w = |c: &Card| self.get(*c).map_or(0.0, |d| d.weight(f));
        let total: f32 = pool.iter().map(w).sum();
        let mut pick = rng.f() * total;
        for (i, c) in pool.iter().enumerate() {
            pick -= w(c);
            if pick <= 0.0 { return i; }
        }
        pool.len() - 1
    }
}

#[cfg(test)]
pub(crate) fn test_book() -> Book {
    let c = |id: &str, firepower, rare, mul: Vec<(&str, f32)>, add: Vec<(&str, f32)>| CardDef {
        firepower, rare, mul: mul.into_iter().map(|(f, v)| (f.to_string(), v)).collect(), add: add.into_iter().map(|(f, v)| (f.to_string(), v)).collect(), ..CardDef::new(id, &id.to_uppercase())
    };
    Book::new(vec![
        c("dmg_up", true, false, vec![("damage_mult", 1.5)], vec![]),
        c("healthy", false, false, vec![], vec![("max_hp_bonus", 35.0)]),
        c("sneakers", false, false, vec![("move_speed_mult", 1.15)], vec![]),
        c("big_mag", true, false, vec![], vec![("mag_size_bonus", 20.0)]),
        c("acrobat", false, true, vec![], vec![("extra_jumps", 2.0)]),
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
        defs.push(CardDef::new("new", "NEW"));
        let b2 = b.merged(defs);
        assert_eq!(b2.find("big_mag"), Some(mag));
        assert_eq!(b2.find("healthy"), None);
        assert_eq!(b2.find("new"), Some(Card(5)));
        assert_eq!(b2.name(Card(1)), "HEALTHY", "a removed card still has its name");
    }

    #[test]
    fn data_cards_change_weapon_fields() {
        let b = test_book();
        let mut w = Weapon::default();
        b.apply(b.find("sneakers").unwrap(), &mut w);
        b.apply(b.find("big_mag").unwrap(), &mut w);
        assert_eq!((w.move_speed_mult, w.mag_size()), (1.15, 25));
    }

    #[test]
    fn offers_are_distinct_have_firepower_and_rare_cards_are_rarer() {
        let b = test_book();
        let mut rng = crate::rng::Rng::new(3);
        let (mut rare, mut rare_behind) = (0, 0);
        for _ in 0..4000 {
            let o = b.offer(&mut rng, 1.0, 3);
            assert_eq!(o.len(), 3);
            assert!(o[0] != o[1] && o[1] != o[2] && o[0] != o[2]);
            assert!(o.iter().any(|c| b.get(*c).unwrap().firepower));
            rare += o.iter().filter(|c| b.get(**c).unwrap().rare).count();
            rare_behind += b.offer(&mut rng, 5.0, 3).iter().filter(|c| b.get(**c).unwrap().rare).count();
        }
        assert!(rare < 1200 && rare_behind > rare * 2, "{rare} {rare_behind}");
    }
    #[test]
    fn baseline_boosts_have_three_times_the_common_draw_weight() {
        let book = Book::new(["dmg_up", "big_mag", "rapid_fire", "ordinary"].map(|id| CardDef::new(id, id)).into());
        let mut rng = crate::rng::Rng::new(42);
        let mut hits = [0usize; 4];
        for _ in 0..20000 { hits[book.offer(&mut rng, 1.0, 1)[0].0 as usize] += 1; }
        for count in &hits[..3] {
            let ratio = *count as f32 / hits[3] as f32;
            assert!((2.7..3.3).contains(&ratio), "draw ratio {ratio}; hits {hits:?}");
        }
    }

}
