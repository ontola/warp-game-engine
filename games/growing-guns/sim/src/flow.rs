//! Rounds and the match (Godot `game.gd`): the spawn cage, kills in the
//! order they happen, round wins, card offers with rarity, match end at
//! ten wins, round modifiers, pickups, the lava leak and modifier strikes.
use crate::player::SKY_DROP_HEIGHT;
use crate::{cards, shape, Arena, Event, Input, Pickup, Strike, StrikeKind, World, DT, DT64};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const ROUNDS_TO_WIN: u32 = 10;
pub const MODIFIER_CHANCE: f32 = 0.30;
pub const CARDS_PER_PICK: usize = 3;
pub const CARD_PICK_TIMEOUT: f64 = 10.0;
pub const BOT_PICK_DELAY: f64 = 1.2;
pub const ROUND_WIN_TO_PICK: f64 = 0.35;
pub const CAGE_SPAWN_HEIGHT: f32 = 45.0;
pub const CAGE_DROP: f32 = 23.0;
pub const CAGE_DESCEND: f64 = 2.6;
pub const CAGE_RELEASE: f64 = 2.75;
pub const SPAWN_SPACING: f32 = 8.0;
pub const LAVA_LEAK_START: f64 = 30.0;
pub const LAVA_LEAK_SPREAD: f32 = 20.0;
const LEAK_WALL_FLOW: f32 = 4.0;
const LEAK_RISE_SPEED: f32 = 1.5;
const LEAK_MAX_RISE: f32 = 18.0;
const PICKUP_MAX: usize = 4;
const PICKUP_HEIGHT: f32 = 52.0;
const PICKUP_LIFETIME: f64 = 45.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase { Waiting, #[default] Playing, Picking, MatchOver }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modifier { LowGravity, Fog, SupplyStorm, SuddenDeath, HeadshotsOnly, BoomBoom, Brrrrt, GunSwap, SniperFight, AirStrikes, IonCannon, Blackout }

pub const MODIFIERS: [(Modifier, &str, &str, &str); 12] = [
    (Modifier::LowGravity, "low_gravity", "LOW GRAVITY", "Floaty jumps and slow-falling bullets"),
    (Modifier::Fog, "fog", "FOG", "You can't see far"),
    (Modifier::SupplyStorm, "supply_storm", "SUPPLY STORM", "Pickups rain from the sky"),
    (Modifier::SuddenDeath, "sudden_death", "SUDDEN DEATH", "The lava leaks from the start"),
    (Modifier::HeadshotsOnly, "headshots_only", "HEADSHOTS ONLY", "Body shots barely scratch"),
    (Modifier::BoomBoom, "boom_boom", "BOOM BOOM", "Everyone has explosive rounds"),
    (Modifier::Brrrrt, "brrrrt", "BRRRRT", "Ten times the fire rate"),
    (Modifier::GunSwap, "gun_swap", "GUN SWAP", "Everyone plays someone else's cards"),
    (Modifier::SniperFight, "sniper_fight", "SNIPER FIGHT", "Huge arena, rail guns and zoom"),
    (Modifier::AirStrikes, "air_strikes", "AIR STRIKES", "Rockets fall from the sky"),
    (Modifier::IonCannon, "ion_cannon", "ION CANNON", "The orbital beam is hunting"),
    (Modifier::Blackout, "blackout", "BLACKOUT", "The lights go out"),
];

impl Modifier {
    pub fn named(n: &str) -> Option<Modifier> { MODIFIERS.iter().find(|m| m.1 == n).map(|m| m.0) }
    pub fn id(self) -> &'static str { MODIFIERS.iter().find(|m| m.0 == self).map_or("", |m| m.1) }
    pub fn title(self) -> &'static str { MODIFIERS.iter().find(|m| m.0 == self).map_or("", |m| m.2) }
    pub fn subtitle(self) -> &'static str { MODIFIERS.iter().find(|m| m.0 == self).map_or("", |m| m.3) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickupKind { Heart, Mushroom, Bomb, PlusOne, Laser, Dice, AirStrike }
pub const PICKUPS: [PickupKind; 7] = [PickupKind::Heart, PickupKind::Mushroom, PickupKind::Bomb, PickupKind::PlusOne, PickupKind::Laser, PickupKind::Dice, PickupKind::AirStrike];

/// The lava leak (sudden death): lava runs down the walls, floods the
/// floor from the edges inward, then rises.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LavaLeak {
    pub active: bool,
    pub elapsed: f32,
    pub spread: f32,
    pub rise: f32,
    pub has_walls: bool,
    pub half: f32,
    /// An island arena: the floor is lava already (Godot
    /// `_lava_floor_already_full`), so it rises from the start, burns 1.15 m
    /// up instead of 2.4, everywhere, and shows no fronts or sheets.
    #[serde(default)]
    pub full: bool,
}

impl LavaLeak {
    pub fn wall_t(&self) -> f32 { if self.has_walls { (self.elapsed / LEAK_WALL_FLOW).clamp(0.0, 1.0) } else { 1.0 } }
    fn wall_s(&self) -> f32 { if self.has_walls { LEAK_WALL_FLOW } else { 0.0 } }
    pub fn floor_t(&self) -> f32 {
        if self.full { return 1.0; }
        let fe = (self.elapsed - self.wall_s()).max(0.0);
        let mut t = (fe / (self.spread - self.wall_s()).max(0.1)).clamp(0.0, 1.0);
        if self.has_walls && self.wall_t() >= 1.0 && t <= 0.0 { t = 0.001; }
        t
    }
    /// How far the lava has crept in from each edge.
    pub fn inset(&self) -> f32 { let t = self.floor_t(); self.half * t * t * (3.0 - 2.0 * t) }
    pub fn surface(&self) -> f32 { 0.09 + self.rise }
}

/// Bookkeeping for the round in progress.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    eliminated: Vec<usize>,
    completed: Vec<usize>,
    round_ending: bool,
    pub winner: Option<usize>,
    /// Counting down to the card picks after a win.
    end_timer: Option<f64>,
    cage: Option<f64>,
    cage_targets: Vec<Vec3>,
    round_elapsed: f64,
    pickup_timer: f64,
    air_timer: Option<f64>,
    air_pending: bool,
    ion_timer: Option<f64>,
    ion_pending: bool,
}

impl State {
    pub(crate) fn is_eliminated(&self, i: usize) -> bool { self.eliminated.contains(&i) }

    pub(crate) fn strike_finished(&mut self, kind: StrikeKind, modifier: bool) {
        if !modifier { return; }
        match kind { StrikeKind::AirStrike => self.air_pending = false, StrikeKind::IonCannon => self.ion_pending = false }
    }
}

impl World {
    /// Godot `_start_round_now`.
    pub(crate) fn start_round(&mut self) {
        // The arena seed comes first, so a match seed keeps its first arena.
        let arena_seed = self.rng.next_u32() as u64;
        let prev_mult = self.pickup_mult();
        self.flow = State { pickup_timer: self.rng.range(10.0, 24.0) as f64 / prev_mult.max(0.1) as f64, ..State::default() };
        // Coop: the last wave's enemies go before anyone respawns.
        if self.coop { self.coop_clear_enemies(); }
        self.modifier = if self.coop { None } else if let Some(m) = self.next_modifier.take() { m } else if self.forced_modifier.is_some() { self.forced_modifier } else if self.rng.f() < self.modifier_chance {
            Some(MODIFIERS[self.rng.below(MODIFIERS.len() as u32) as usize].0)
        } else { None };
        let (lo, hi) = if self.modifier == Some(Modifier::SniperFight) { crate::arena::SNIPER_SIZE } else { crate::arena::STANDARD_SIZE };
        self.arena = Arena::generate_with(arena_seed, lo, hi, self.force_island);
        self.carves.clear();
        self.breakage = crate::Breakage::default();
        self.lava = LavaLeak::default();
        self.events.push(Event::NewArena);
        for i in 0..self.players.len() { self.rebuild_weapon(i); }
        if self.modifier == Some(Modifier::GunSwap) { self.gun_swap(); }
        if let Some(m) = self.modifier { for i in 0..self.players.len() { modifier_weapon(m, &mut self.players[i].weapon); } }
        self.spawn_all();
        self.bullets.clear();
        self.projectiles.clear();
        self.pickups.clear();
        self.strikes.clear();
        if self.modifier == Some(Modifier::SupplyStorm) { self.spawn_pickup(); }
        if self.modifier == Some(Modifier::SuddenDeath) { self.start_lava_leak(); }
        self.flow.cage = Some(0.0);
        if self.coop { self.coop_begin_wave(); }
        if self.modifier == Some(Modifier::AirStrikes) { self.flow.air_timer = Some(self.rng.range(0.5, 1.25) as f64); }
        if self.modifier == Some(Modifier::IonCannon) { self.flow.ion_timer = Some(self.rng.range(0.75, 1.5) as f64); }
        self.phase = Phase::Playing;
        if let Some(h) = self.hooks.0.clone() { h.round_start(self); }
    }

    /// Everyone plays someone else's cards this round (never the identity).
    fn gun_swap(&mut self) {
        let n = self.players.len();
        if n < 2 { return; }
        let decks: Vec<Vec<cards::Card>> = self.players.iter().map(|p| p.cards.clone()).collect();
        let mut order: Vec<usize> = (0..n).collect();
        for k in (1..n).rev() { let j = self.rng.below(k as u32 + 1) as usize; order.swap(k, j); }
        if order.iter().enumerate().all(|(k, &o)| k == o) { order = (0..n).map(|k| (k + 1) % n).collect(); }
        for i in 0..n {
            self.players[i].weapon = crate::Weapon::default();
            let book = self.cards.clone();
            for &c in &decks[order[i]] {
                let def = book.get(c).cloned();
                book.apply(c, &mut self.players[i].weapon);
                if let (Some(h), Some(d)) = (self.hooks.0.clone(), def) { h.apply_card(&d, &mut self.players[i].weapon); }
            }
        }
    }

    /// Greedy spawn picks with spacing, each in the cage 45 m up.
    fn spawn_all(&mut self) {
        let mut spots = self.arena.spawns.clone();
        for k in (1..spots.len()).rev() { let j = self.rng.below(k as u32 + 1) as usize; spots.swap(k, j); }
        let mut used: Vec<Vec3> = Vec::new();
        for i in 0..self.players.len() {
            let pick = pick_spawn(&spots, &used);
            used.push(pick);
            let p = &mut self.players[i];
            let keep = (p.score, p.kills, std::mem::take(&mut p.cards), p.weapon.clone(), p.air_strike_charges, p.offer.take());
            *p = crate::Player::new(p.bot);
            (p.score, p.kills, p.cards, p.weapon, p.air_strike_charges, p.offer) = keep;
            p.health = p.max_health();
            p.ammo = p.mag_size();
            p.phoenix_charges = p.weapon.phoenix_revives;
            p.jumps_left = 2 + p.weapon.extra_jumps;
            p.pos = pick + Vec3::Y * CAGE_SPAWN_HEIGHT;
            p.frozen = true;
            let to = -Vec2::new(pick.x, pick.z);
            p.yaw = if to.length_squared() > 1e-4 { (-to.x).atan2(-to.y) } else { 0.0 };
        }
        self.flow.cage_targets = used;
    }

    /// Godot `report_kill`: deaths count in the order they happen.
    pub(crate) fn report_kill(&mut self, victim: usize) {
        if self.phase != Phase::Playing || self.flow.eliminated.contains(&victim) { return; }
        self.flow.eliminated.push(victim);
        if self.coop {
            self.players[victim].ghost = self.players.len() >= 3 && !self.players[victim].bot;
            self.coop_kill(victim);
            return;
        }
        // Park eliminated bots for the remainder of a group round.
        self.players[victim].ghost = self.players[victim].bot && self.players.len() >= 3;
        if self.flow.round_ending { return; }
        let alive: Vec<usize> = (0..self.players.len()).filter(|i| !self.flow.eliminated.contains(i)).collect();
        if alive.len() <= 1 {
            let winner = alive.first().copied().or_else(|| (0..self.players.len()).find(|&k| k != victim));
            self.end_round(winner);
        } else {
            self.players[victim].ghost = true;
            self.begin_pick(victim);
        }
    }

    fn end_round(&mut self, winner: Option<usize>) {
        if self.flow.round_ending { return; }
        self.flow.round_ending = true;
        self.flow.winner = winner;
        if let Some(w) = winner {
            self.players[w].score += 1;
            if self.players[w].score >= self.rounds_to_win {
                self.phase = Phase::MatchOver;
                for (k, p) in self.players.iter_mut().enumerate() { if k != w { p.frozen = true; } }
                self.events.push(Event::MatchOver { winner: w });
                return;
            }
        }
        self.events.push(Event::RoundOver { winner });
        // Resolve all hits from this tick before opening the group picks.
        let group_picks = self.players.len() > 2 && self.cards.enabled().next().is_some();
        self.flow.end_timer = Some(if winner.is_some() && !group_picks { ROUND_WIN_TO_PICK } else { 0.0 });
    }

    fn start_picking(&mut self) {
        self.phase = Phase::Picking;
        for v in self.flow.eliminated.clone() { self.begin_pick(v); }
        self.maybe_finish_picks();
    }

    pub(crate) fn begin_pick(&mut self, i: usize) {
        if self.flow.completed.contains(&i) || self.players[i].offer.is_some() { return; }
        let f = self.score_factor(i);
        let cards = self.cards.offer(&mut self.rng, f, CARDS_PER_PICK);
        if cards.is_empty() { self.flow.completed.push(i); return; }
        let p = &mut self.players[i];
        p.offer = Some(cards);
        p.offer_age = 0.0;
    }

    /// Losers far behind see more rare cards.
    fn score_factor(&self, i: usize) -> f32 {
        let max = self.players.iter().map(|p| p.score).max().unwrap_or(0);
        if max == 0 { 1.0 } else { 1.0 + 0.4 * (max - self.players[i].score) as f32 }
    }

    fn finalize_pick(&mut self, i: usize, k: usize) {
        let Some(offer) = self.players[i].offer.take() else { return };
        let card = offer[k.min(offer.len() - 1)];
        self.give(i, card);
        self.events.push(Event::Picked { player: i, card });
        self.flow.completed.push(i);
        self.maybe_finish_picks();
    }

    pub(crate) fn maybe_finish_picks(&mut self) {
        if self.phase != Phase::Picking { return; }
        let waiting = if self.coop {
            // Every human who got an offer at the wave's end (a player who
            // joined since has none and does not hold the next wave up).
            self.players.iter().any(|p| p.offer.is_some())
        } else {
            self.flow.eliminated.iter().any(|i| !self.flow.completed.contains(i))
        };
        if waiting { return; }
        self.round += 1;
        if self.coop { self.coop_next_wave(); }
        self.start_round();
    }

    /// Per tick: picks, the cage, the win pause, strikes, pickups, the leak.
    pub(crate) fn flow_tick(&mut self, inputs: &[Input]) {
        let humans = self.players.iter().filter(|p| !p.bot).count();
        // A pick can start the next round, which in coop drops the wave's
        // enemies: the length is read every time.
        let mut next = 0;
        while next < self.players.len() {
            let i = next;
            next += 1;
            let Some(n) = self.players[i].offer.as_ref().map(|o| o.len()) else { continue };
            self.players[i].offer_age += DT64;
            let p = &self.players[i];
            let choice = if p.bot {
                (p.offer_age >= BOT_PICK_DELAY).then(|| self.rng.below(n as u32) as usize)
            } else if let Some(k) = inputs.get(i).and_then(|inp| inp.pick).filter(|&k| (k as usize) < n) {
                Some(k as usize)
            } else if humans > 1 && p.offer_age >= self.card_pick_seconds {
                Some(self.rng.below(n as u32) as usize)
            } else { None };
            if let Some(k) = choice { self.finalize_pick(i, k); }
        }
        // Coop: a wave's end where nobody got cards goes on here, at a point
        // where the enemies can be dropped.
        if self.coop { self.maybe_finish_picks(); }
        if let Some(t) = self.flow.cage.as_mut() {
            *t += DT64;
            let t = *t;
            for (i, p) in self.players.iter_mut().enumerate() {
                let Some(&ground) = self.flow.cage_targets.get(i) else { continue };
                if !p.frozen || self.phase == Phase::MatchOver { continue; }
                let u = (t / CAGE_DESCEND).min(1.0) as f32;
                p.pos = Vec3::new(ground.x, ground.y + CAGE_SPAWN_HEIGHT - CAGE_DROP * (1.0 - (1.0 - u) * (1.0 - u)), ground.z);
            }
            if t + 1e-9 >= CAGE_RELEASE {
                self.flow.cage = None;
                for p in &mut self.players { p.frozen = false; }
            }
        }
        if let Some(t) = self.flow.end_timer.as_mut() {
            *t -= DT64;
            if *t <= 1e-9 { self.flow.end_timer = None; self.start_picking(); }
        }
        if self.phase != Phase::Playing { return; }
        self.flow.round_elapsed += DT64;
        if !self.coop && !self.lava.active && self.flow.round_elapsed >= LAVA_LEAK_START { self.start_lava_leak(); }
        self.pickup_spawner();
        self.modifier_strikes();
        if self.coop { self.coop_tick(); }
    }

    pub(crate) fn pickup_mult(&self) -> f32 { (if self.modifier == Some(Modifier::SupplyStorm) { 3.0 } else { 1.0 }) * self.rules.pickup_rate }

    fn pickup_spawner(&mut self) {
        let mult = self.pickup_mult();
        if mult <= 0.0 || self.pickups.len() >= PICKUP_MAX { return; }
        self.flow.pickup_timer -= DT64;
        if self.flow.pickup_timer > 0.0 { return; }
        self.flow.pickup_timer = (20.0 + self.rng.range(-7.0, 7.0)) as f64 / mult.max(0.1) as f64;
        self.spawn_pickup();
    }

    fn spawn_pickup(&mut self) {
        let kind = PICKUPS[self.rng.below(PICKUPS.len() as u32) as usize];
        let xz = self.random_arena_xz();
        self.pickups.push(Pickup { kind, pos: Vec3::new(xz.x, PICKUP_HEIGHT, xz.y), vy: 0.0, grounded: None });
    }

    fn random_arena_xz(&mut self) -> Vec2 {
        let half = (self.arena.size * 0.5 - 6.0).max(14.0);
        Vec2::new(self.rng.range(-half + 4.0, half - 4.0), self.rng.range(-half + 4.0, half - 4.0))
    }

    pub(crate) fn step_pickups(&mut self) {
        let mut k = 0;
        while k < self.pickups.len() {
            let mut pk = self.pickups[k].clone();
            // Destruction can remove the stone supporting a settled item.
            if pk.grounded.is_some() && shape::raycast(&self.arena.grid, pk.pos, Vec3::NEG_Y, 0.6).is_none() {
                pk.grounded = None;
                pk.vy = 0.0;
            }
            if pk.pos.y < self.lava_surface_at(pk.pos) {
                self.pickups.remove(k);
                continue;
            }
            match pk.grounded.as_mut() {
                Some(t) => { *t += DT64; if *t >= PICKUP_LIFETIME { self.pickups.remove(k); continue; } }
                None => {
                    pk.vy = (pk.vy - 28.0 * DT).max(-55.0);
                    let step = pk.vy * DT;
                    match shape::raycast(&self.arena.grid, pk.pos, Vec3::NEG_Y, -step) {
                        Some((t, _, _)) => { pk.pos.y -= t; pk.pos.y += 0.42; pk.grounded = Some(0.0); }
                        None => pk.pos.y += step,
                    }
                    // Lost drops must release their slot and their beam, including islands.
                    if pk.pos.y < crate::FELL_OFF_Y { self.pickups.remove(k); continue; }
                }
            }
            let taker = self.players.iter().position(|p| {
                !p.ghost && p.alive && p.health > 0 && p.launching.is_none() && !(self.coop && p.bot)
                    && p.pos.distance(pk.pos) <= 1.45 * p.weapon.body_scale.max(1.0)
            });
            if let (Some(i), true) = (taker, self.phase == Phase::Playing) {
                self.pickups.remove(k);
                self.apply_pickup(i, pk.kind);
                continue;
            }
            self.pickups[k] = pk;
            k += 1;
        }
    }

    /// Godot `apply_round_pickup`: buffs for the rest of the round.
    fn apply_pickup(&mut self, i: usize, kind: PickupKind) {
        let p = &mut self.players[i];
        let w = &mut p.weapon;
        match kind {
            PickupKind::Heart => p.health += 50,
            PickupKind::Mushroom => { p.health += 100; w.body_scale *= 3.0; w.head_scale *= 3.0; }
            PickupKind::Bomb => for _ in 0..2 {
                w.explosive_radius += if w.explosive_radius <= 0.0 { 4.0 } else { 2.5 };
                w.explosive_damage += 25.0;
            },
            PickupKind::PlusOne => p.phoenix_charges += 1,
            PickupKind::Laser => {
                let (shots, mag) = (w.shots_per_trigger() as i32, w.mag_size() as i32);
                w.bullet_speed_mult *= 10.0;
                w.spread = (w.spread * 0.1).max(0.0002);
                w.recoil_per_shot *= 0.1;
                w.fire_rate_mult *= 10.0;
                w.extra_projectiles += 9 * shots;
                w.mag_size_bonus += 9 * mag;
                w.damage_mult *= 0.1;
            }
            PickupKind::Dice => {
                let mut order = [0usize, 1, 2, 3, 4, 5];
                for k in (1..6).rev() { let j = self.rng.below(k as u32 + 1) as usize; order.swap(k, j); }
                let w = &mut self.players[i].weapon;
                for (slot, mult) in [(order[0], 2.0), (order[1], 0.5)] {
                    let f = match slot { 0 => &mut w.damage_mult, 1 => &mut w.fire_rate_mult, 2 => &mut w.move_speed_mult, 3 => &mut w.bullet_speed_mult, 4 => &mut w.reload_mult, _ => &mut w.spread };
                    *f *= mult;
                }
            }
            PickupKind::AirStrike => p.air_strike_charges += 1,
        }
        let p = &mut self.players[i];
        p.ammo = p.ammo.max(p.mag_size()).min(p.mag_size());
        self.events.push(Event::PickedUp { player: i, kind });
    }

    /// The air_strikes and ion_cannon modifiers: one at a time each, on a timer.
    fn modifier_strikes(&mut self) {
        if let Some(t) = self.flow.air_timer.as_mut()
            && !self.flow.air_pending {
                *t -= DT64;
                if *t <= 0.0 {
                    self.flow.air_timer = Some(4.0 + self.rng.range(-1.5, 1.5) as f64);
                    self.flow.air_pending = true;
                    let target = self.random_strike_target();
                    self.strikes.push(Strike { kind: StrikeKind::AirStrike, target, owner: None, age: 0.0 });
                }
            }
        if let Some(t) = self.flow.ion_timer.as_mut()
            && !self.flow.ion_pending {
                *t -= DT64;
                if *t <= 0.0 {
                    self.flow.ion_timer = Some(5.5 + self.rng.range(-1.5, 1.5) as f64);
                    self.flow.ion_pending = true;
                    let target = self.random_strike_target();
                    self.strikes.push(Strike { kind: StrikeKind::IonCannon, target, owner: None, age: 0.0 });
                }
            }
    }

    fn random_strike_target(&mut self) -> Vec3 {
        let mut first = None;
        for _ in 0..14 {
            let xz = self.random_arena_xz();
            first.get_or_insert(xz);
            if let Some((t, n, _)) = shape::raycast(&self.arena.grid, Vec3::new(xz.x, 320.0, xz.y), Vec3::NEG_Y, 740.0)
                && n.y >= 0.55 { return Vec3::new(xz.x, 320.0 - t + 0.05, xz.y); }
        }
        let xz = first.unwrap_or_default();
        Vec3::new(xz.x, 0.0, xz.y)
    }

    fn start_lava_leak(&mut self) {
        let has_walls = self.arena.variant == crate::arena::Variant::Walled || self.arena.variant == crate::arena::Variant::Hole;
        let full = self.arena.variant == crate::arena::Variant::Island;
        self.lava = LavaLeak { active: true, elapsed: 0.0, spread: LAVA_LEAK_SPREAD, rise: 0.0, has_walls, half: self.arena.size * 0.5, full };
    }

    /// Start the leak as if it began `elapsed` seconds ago (Godot
    /// `start_lava_leak(spread, start_elapsed)`, the dev key L and late
    /// joiners): for scripts and tools.
    pub fn start_lava_leak_at(&mut self, elapsed: f32) {
        self.start_lava_leak();
        let l = &mut self.lava;
        l.elapsed = elapsed.max(0.0);
        let floor_s = (l.spread - l.wall_s()).max(0.1);
        l.rise = ((l.elapsed - l.wall_s()).max(0.0) - floor_s).max(0.0) * LEAK_RISE_SPEED;
        l.rise = l.rise.min(LEAK_MAX_RISE);
    }

    /// The leak's flow and its damage (runs in every phase until the next arena).
    pub(crate) fn lava_leak_tick(&mut self) {
        if !self.lava.active { return; }
        let l = &mut self.lava;
        l.elapsed += DT;
        if l.floor_t() >= 1.0 { l.rise = (l.rise + DT * LEAK_RISE_SPEED).min(LEAK_MAX_RISE); }
        let l = self.lava.clone();
        let (half, inset, surface, wall_t) = (l.half, l.inset(), l.surface(), l.wall_t());
        let floor_active = (l.elapsed - l.wall_s()) > 0.0 || !l.has_walls || wall_t >= 1.0;
        let wall_h = crate::arena::WALL_HEIGHT * self.arena.stadium.scale;
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if !p.alive || p.ghost { continue; }
            let (x, y, z) = (p.pos.x, p.pos.y, p.pos.z);
            if y < surface { self.kill_environmental(i); continue; }
            if y > surface + if l.full { 1.15 } else { 2.4 } { self.players[i].lava_accum = 0.0; continue; }
            let in_floor = floor_active && (x.abs() > half - inset || z.abs() > half - inset);
            let near = |a: f32, b: f32| (a.abs() - half).abs() <= 1.25 && b.abs() <= half;
            let in_wall = l.has_walls && (near(x, z) || near(z, x)) && y >= wall_h - (0.05 + (wall_h - 0.05) * wall_t) - 0.8 && y <= wall_h + 0.8;
            if in_floor || in_wall {
                self.players[i].lava_accum += DT64;
                while self.players[i].lava_accum >= 0.5 { self.players[i].lava_accum -= 0.5; self.environmental_damage(i, 15); }
            } else {
                self.players[i].lava_accum = 0.0;
            }
        }
    }

    /// A sky drop above `near` (phoenix, revive): 60 m up a valid spawn.
    pub(crate) fn sky_drop_spot(&self, _i: usize, near: Vec3) -> Vec3 {
        let ground = shape::raycast(&self.arena.grid, near + Vec3::Y * 0.2, Vec3::NEG_Y, 40.0)
            .filter(|(_, n, _)| n.y >= 0.65)
            .map(|(t, _, _)| near + Vec3::Y * (0.2 - t + crate::HALF_HEIGHT))
            .filter(|&g| self.arena.spawn_safe(g));
        let base = ground.unwrap_or_else(|| pick_spawn(&self.arena.spawns, &self.players.iter().map(|p| p.pos).collect::<Vec<_>>()));
        base + Vec3::Y * SKY_DROP_HEIGHT
    }

    /// Skip to a new round now (debug).
    pub fn next_round(&mut self) {
        for p in &mut self.players { p.offer = None; }
        self.round += 1;
        if self.coop { self.coop_next_wave(); }
        self.start_round();
    }

    /// Start over: scores and cards cleared (Godot `_restart_match`).
    pub fn restart_match(&mut self) {
        for p in &mut self.players { p.score = 0; p.kills = 0; p.cards.clear(); p.offer = None; p.air_strike_charges = 0; }
        self.round = 1;
        if self.coop { self.coop_restart(); }
        self.start_round();
    }

    /// Five more rounds: everyone but the winner picks a card first.
    pub fn extend_match(&mut self) {
        // Coop has no extend: a lost run is over (Godot offers a rematch).
        if self.phase != Phase::MatchOver || self.coop { return; }
        self.rounds_to_win += 5;
        self.phase = Phase::Picking;
        let winner = self.flow.winner;
        self.flow.eliminated = (0..self.players.len()).filter(|&k| Some(k) != winner).collect();
        self.flow.completed.clear();
        for p in &mut self.players { p.frozen = false; }
        for k in self.flow.eliminated.clone() { self.begin_pick(k); }
        self.maybe_finish_picks();
    }

    /// The round has a result (banner time until the next round).
    pub fn round_over(&self) -> bool { self.flow.round_ending || self.phase == Phase::MatchOver }

    /// The round's winner so far (versus).
    pub fn round_winner(&self) -> Option<usize> { self.flow.winner }

    /// Seconds left in the spawn cage, if it is closed.
    pub fn cage_left(&self) -> Option<f32> { self.flow.cage.map(|t| (CAGE_RELEASE - t).max(0.0) as f32) }

    /// Mirror the host's cage state for presentation (remote worlds never step).
    pub(crate) fn apply_cage_left(&mut self, left: Option<f32>) {
        self.flow.cage = left.filter(|t| t.is_finite()).map(|t| (CAGE_RELEASE - t as f64).clamp(0.0, CAGE_RELEASE));
    }
}

/// Godot `_pick_spawn`: as far from the others as possible, at similar
/// height, at least 8 m apart if it can.
pub(crate) fn pick_spawn(spots: &[Vec3], used: &[Vec3]) -> Vec3 {
    let mut best: Option<(f32, Vec3)> = None;
    let mut fallback: Option<(f32, Vec3)> = None;
    for &s in spots {
        let min_d = used.iter().map(|u| Vec2::new(u.x - s.x, u.z - s.z).length()).fold(100.0f32, f32::min);
        let score = min_d - used.first().map_or(0.0, |u| 1.5 * (s.y - u.y).abs());
        if used.is_empty() || min_d >= SPAWN_SPACING {
            if best.is_none_or(|b| score > b.0) { best = Some((score, s)); }
        } else if fallback.is_none_or(|f| min_d > f.0) {
            fallback = Some((min_d, s));
        }
    }
    best.or(fallback).map(|b| b.1).or_else(|| spots.first().copied()).unwrap_or(Vec3::new(0.0, 8.0, 0.0))
}

/// The round modifiers that change weapons, after the cards.
pub fn modifier_weapon(m: Modifier, w: &mut crate::Weapon) {
    match m {
        Modifier::BoomBoom => { w.explosive_radius += if w.explosive_radius <= 0.0 { 4.0 } else { 2.5 }; w.explosive_damage += 25.0; }
        Modifier::Brrrrt => { let mag = w.mag_size() as i32; w.fire_rate_mult *= 10.0; w.mag_size_bonus += 9 * mag; }
        Modifier::SniperFight => { w.special = crate::Special::Zoom; w.spread *= 0.1; w.bullet_speed_mult *= 10.0; }
        _ => {}
    }
}

impl World {
    /// Set everyone on a spawn of the current arena, spaced as a round's
    /// spawns but without the shuffle (debug: a new arena mid-round).
    pub fn place_on_spawns(&mut self) {
        let mut used: Vec<Vec3> = Vec::new();
        for i in 0..self.players.len() {
            let pick = pick_spawn(&self.arena.spawns, &used);
            used.push(pick);
            let p = &mut self.players[i];
            (p.pos, p.vel, p.frozen, p.lava_accum) = (pick, Vec3::ZERO, false, 0.0);
            let to = -Vec2::new(pick.x, pick.z);
            p.yaw = if to.length_squared() > 1e-4 { (-to.x).atan2(-to.y) } else { 0.0 };
        }
        self.flow.cage = None;
        self.flow.cage_targets = used;
    }

    /// Open the spawn cage now and set everyone on their spawn (tests, debug).
    pub fn release_cage(&mut self) {
        self.flow.cage = None;
        for (i, p) in self.players.iter_mut().enumerate() {
            if let Some(&g) = self.flow.cage_targets.get(i) && p.frozen { p.pos = g; }
            p.frozen = false;
        }
    }

    /// Hand slot `i` to a joining human: if the bot in that seat already died
    /// (a ghost in a 3+ player round, or a parked coop seat), spawn the new
    /// human into the round so they immediately have a visible body and gun.
    pub fn join_human(&mut self, i: usize) {
        let Some(p) = self.players.get_mut(i) else { return };
        p.bot = false;
        if self.phase == Phase::Playing && !self.round_over() && !self.players[i].alive {
            let used: Vec<Vec3> = self.players.iter().enumerate().filter(|&(k, p)| k != i && p.alive).map(|(_, p)| p.pos).collect();
            let pick = pick_spawn(&self.arena.spawns, &used);
            self.flow.eliminated.retain(|&x| x != i);
            self.flow.completed.retain(|&x| x != i);
            self.waves.downed.retain(|d| d.player != i);
            let p = &mut self.players[i];
            let keep = (p.score, p.kills, std::mem::take(&mut p.cards), p.weapon.clone(), p.air_strike_charges);
            *p = crate::Player::new(false);
            (p.score, p.kills, p.cards, p.weapon, p.air_strike_charges) = keep;
            p.health = p.max_health();
            p.ammo = p.mag_size();
            p.phoenix_charges = p.weapon.phoenix_revives;
            p.jumps_left = 2 + p.weapon.extra_jumps;
            let to = -Vec2::new(pick.x, pick.z);
            p.yaw = if to.length_squared() > 1e-4 { (-to.x).atan2(-to.y) } else { 0.0 };
            if self.flow.cage.is_some() {
                p.pos = pick + Vec3::Y * CAGE_SPAWN_HEIGHT;
                p.frozen = true;
                if i < self.flow.cage_targets.len() { self.flow.cage_targets[i] = pick; }
            } else {
                p.pos = pick;
                p.frozen = false;
                p.iframes = 1.5;
            }
            if self.coop { self.coop_spawn_health(i); }
        } else if self.phase == Phase::Picking && self.flow.eliminated.contains(&i) && !self.flow.completed.contains(&i) {
            self.begin_pick(i);
        }
    }

    /// Add a computer player (or a fresh seat) on a free spawn, with its own AI brain.
    pub fn add_bot(&mut self) -> usize {
        let i = self.players.len();
        let used: Vec<Vec3> = self.players.iter().filter(|p| p.alive).map(|p| p.pos).collect();
        let pick = pick_spawn(&self.arena.spawns, &used);
        let mut p = crate::Player::new(true);
        if let Some(m) = self.modifier { modifier_weapon(m, &mut p.weapon); }
        p.health = p.max_health();
        p.ammo = p.mag_size();
        let to = -Vec2::new(pick.x, pick.z);
        p.yaw = if to.length_squared() > 1e-4 { (-to.x).atan2(-to.y) } else { 0.0 };
        if self.flow.cage.is_some() {
            p.pos = pick + Vec3::Y * CAGE_SPAWN_HEIGHT;
            p.frozen = true;
            self.flow.cage_targets.push(pick);
        } else {
            p.pos = pick;
        }
        self.players.push(p);
        self.brains.push(crate::bot::Brain::new(self.arena.seed ^ ((i as u64 + 1) * 0x9E37)));
        i
    }

    /// Remove the last AI bot slot unless it is in `keep_slots` or fewer than
    /// 2 players would remain.
    pub fn remove_bot(&mut self, keep_slots: &[usize]) -> bool {
        if self.coop || self.players.len() <= 2 { return false; }
        let last = self.players.len() - 1;
        if !self.players[last].bot || keep_slots.contains(&last) { return false; }
        self.players.pop();
        self.brains.truncate(last);
        self.flow.cage_targets.truncate(last);
        self.flow.eliminated.retain(|&i| i < last);
        self.flow.completed.retain(|&i| i < last);
        if self.flow.winner == Some(last) { self.flow.winner = None; }
        self.bullets.retain(|b| b.owner < last);
        self.projectiles.retain(|p| p.owner < last);
        for s in &mut self.strikes { if s.owner == Some(last) { s.owner = None; } }
        self.events.retain(|e| crate::coop::event_players(e).iter().all(|&p| p < last));
        self.delayed.retain(|(_, d)| match *d {
            crate::Delayed::Hit { victim, from, .. } => victim < last && from.is_none_or(|f| f < last),
            crate::Delayed::Credit { shooter, .. } => shooter < last,
            crate::Delayed::Echo { player } => player < last,
        });
        self.damages.retain(|&(v, _, by)| v < last && by.is_none_or(|b| b < last));
        self.gives.retain(|&(p, _)| p < last);
        for p in &mut self.players { if p.poison_from == Some(last) { p.poison_from = None; } }
        if self.phase == Phase::Picking {
            self.maybe_finish_picks();
        } else if self.phase == Phase::Playing && !self.flow.round_ending {
            let alive: Vec<usize> = (0..self.players.len()).filter(|&i| self.players[i].alive && !self.flow.eliminated.contains(&i)).collect();
            if alive.len() <= 1 {
                self.end_round(alive.first().copied());
            }
        }
        true
    }
}
