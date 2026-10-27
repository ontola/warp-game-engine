//! Growing Guns gameplay on a fixed 60 Hz tick: movement, rifle, rounds and
//! bots. Constants come from the Godot game (`player.gd`, `weapon.gd`). The
//! world is a plain value: the host steps it, clients render snapshots.
pub mod arena;
pub mod cards;
mod bot;
pub mod rng;
pub mod shape;

pub use arena::Arena;
use glam::{Vec2, Vec3};
use rng::Rng;
use serde::{Deserialize, Serialize};

pub const TICK_HZ: u32 = 60;
pub const DT: f32 = 1.0 / TICK_HZ as f32;

pub const WALK_SPEED: f32 = 14.0;
pub const AIR_ACCEL: f32 = 70.0;
pub const GROUND_ACCEL: f32 = 100.0;
pub const FRICTION: f32 = 8.0;
pub const JUMP_VELOCITY: f32 = 9.01;
pub const DOUBLE_JUMP_VELOCITY: f32 = 8.5;
pub const WALL_JUMP_V: f32 = 10.5;
pub const WALL_JUMP_H: f32 = 4.0;
pub const WALL_JUMP_COOLDOWN: f32 = 0.14;
pub const DASH_SPEED: f32 = 28.0;
pub const DASH_TIME: f32 = 0.18;
pub const MAX_DASH_CHARGES: u32 = 2;
pub const DASH_RECHARGE_TIME: f32 = 3.0;
pub const GRAVITY: f32 = 30.0;
pub const MAX_FALL: f32 = 60.0;

pub const RADIUS: f32 = 0.4;
pub const HALF_HEIGHT: f32 = 0.9;
const MOVER: kern::collide::Mover = kern::collide::Mover { radius: RADIUS, half_height: HALF_HEIGHT };
/// Camera height above the capsule centre.
pub const EYE: f32 = 0.62;
pub const MAX_HEALTH: i32 = 100;

pub const DAMAGE: f32 = 22.0;
pub const HEADSHOT_MULT: f32 = 2.0;
pub const FIRE_INTERVAL: f32 = 0.22;
pub const MAG_SIZE: u32 = 5;
pub const RELOAD_TIME: f32 = 1.2;
pub const SPREAD: f32 = 0.008;
pub const RECOIL_BLOOM: f32 = 0.018;
pub const RECOIL_PITCH: f32 = 0.018;
pub const RANGE: f32 = 200.0;
pub const BULLET_SPEED: f32 = 150.0;
pub const KNOCKBACK_PER_DAMAGE: f32 = 0.12;

pub const INTERMISSION: f32 = 3.0;
/// Seconds a loser has to pick a card (Godot: CARD_PICK_TIMEOUT).
pub const PICK_TIME: f32 = 10.0;

/// Hit spheres relative to the capsule centre: (offset y, radius, is head).
pub const HITBOXES: [(f32, f32, bool); 3] = [(-0.45, 0.37, false), (0.1, 0.42, false), (0.65, 0.27, true)];

/// One tick of input for one player. `jump`, `dash` and `reload` are edges
/// (pressed this tick); `fire` is held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Input {
    /// Strafe right (+x) and forward (+y), each in [-1, 1].
    pub mv: [f32; 2],
    /// Look delta in radians: yaw (left positive), pitch (up positive).
    pub look: [f32; 2],
    pub jump: bool,
    pub dash: bool,
    pub fire: bool,
    pub reload: bool,
    /// Card choice (0..3) while this player has an offer.
    #[serde(default)]
    pub pick: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_floor: bool,
    pub wall_normal: Option<Vec3>,
    pub jumps_left: u32,
    pub wall_jump_cd: f32,
    pub dash_timer: f32,
    pub dash_dir: Vec3,
    pub dash_charges: u32,
    pub dash_recharge: f32,
    pub health: i32,
    pub alive: bool,
    pub ammo: u32,
    pub reload_timer: f32,
    pub fire_cd: f32,
    pub bloom: f32,
    pub score: u32,
    pub bot: bool,
    /// Seconds since the last shot, for the gun kick animation.
    pub since_shot: f32,
    /// Cards taken this match and their accumulated effect.
    pub cards: Vec<cards::Card>,
    pub mods: cards::Mods,
    /// Three cards to choose from after losing a round.
    pub offer: Option<[cards::Card; 3]>,
}

impl Player {
    fn new(bot: bool) -> Self {
        Self {
            pos: Vec3::ZERO, vel: Vec3::ZERO, yaw: 0.0, pitch: 0.0, on_floor: false, wall_normal: None,
            jumps_left: 1, wall_jump_cd: 0.0, dash_timer: 0.0, dash_dir: Vec3::ZERO,
            dash_charges: MAX_DASH_CHARGES, dash_recharge: 0.0, health: MAX_HEALTH, alive: true,
            ammo: MAG_SIZE, reload_timer: 0.0, fire_cd: 0.0, bloom: 0.0, score: 0, bot, since_shot: 9.0,
            cards: Vec::new(), mods: cards::Mods::default(), offer: None,
        }
    }
    pub fn eye(&self) -> Vec3 { self.pos + Vec3::Y * EYE }
    pub fn forward(&self) -> Vec3 { look_dir(self.yaw, self.pitch) }
    pub fn reloading(&self) -> bool { self.reload_timer > 0.0 }
    pub fn max_health(&self) -> i32 { MAX_HEALTH + self.mods.hp_bonus }
    pub fn mag_size(&self) -> u32 { MAG_SIZE + self.mods.mag_bonus }
}

/// Godot convention: yaw 0 looks down -Z, positive yaw turns left.
pub fn look_dir(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(-sy * cp, sp, -cy * cp)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bullet {
    pub pos: Vec3,
    pub vel: Vec3,
    pub owner: usize,
    pub travelled: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    Shot { player: usize, from: Vec3, dir: Vec3 },
    Impact { pos: Vec3, normal: Vec3 },
    Hit { victim: usize, attacker: usize, damage: i32, head: bool, pos: Vec3 },
    Kill { victim: usize, attacker: Option<usize> },
    Jump { player: usize },
    Dash { player: usize },
    RoundOver { winner: Option<usize> },
    Picked { player: usize, card: cards::Card },
    /// A fresh arena was generated: rebuild the scene.
    NewArena,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub round: u32,
    pub intermission: f32,
    #[serde(default)]
    pub pick_time: f32,
    #[serde(default)]
    pub ended: bool,
    pub players: Vec<Player>,
    pub bullets: Vec<Bullet>,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug)]
pub struct World {
    pub arena: Arena,
    pub players: Vec<Player>,
    pub bullets: Vec<Bullet>,
    pub tick: u64,
    pub round: u32,
    /// Counts down after a round ends; the next round starts at zero.
    pub intermission: f32,
    /// Seconds left for losers to pick a card.
    pub pick_time: f32,
    /// The round is over: intermission and card picks until the next one.
    pub ended: bool,
    pub events: Vec<Event>,
    rng: Rng,
    brains: Vec<bot::Brain>,
}

impl World {
    /// `bots[i]` makes player i computer controlled.
    pub fn new(seed: u64, bots: &[bool]) -> Self {
        let mut rng = Rng::new(seed);
        let arena = Arena::generate(rng.next_u32() as u64);
        let players = bots.iter().map(|&b| Player::new(b)).collect();
        let brains = (0..bots.len()).map(|i| bot::Brain::new(seed ^ (i as u64 + 1) * 0x9E37)).collect();
        let mut w = World { arena, players, bullets: Vec::new(), tick: 0, round: 1, intermission: 0.0, pick_time: 0.0, ended: false, events: Vec::new(), rng, brains };
        w.spawn_all();
        w
    }

    fn spawn_all(&mut self) {
        let mut spots = self.arena.spawns.clone();
        // Shuffle, then hand out; reuse with an offset if players outnumber spawns.
        for i in (1..spots.len()).rev() {
            let j = self.rng.below(i as u32 + 1) as usize;
            spots.swap(i, j);
        }
        for (i, p) in self.players.iter_mut().enumerate() {
            let s = spots[i % spots.len()] + Vec3::new((i / spots.len()) as f32 * 1.2, 0.0, 0.0);
            let (score, taken, mods) = (p.score, std::mem::take(&mut p.cards), p.mods);
            *p = Player::new(p.bot);
            p.score = score;
            p.cards = taken;
            p.mods = mods;
            p.health = p.max_health();
            p.ammo = p.mag_size();
            p.pos = s + Vec3::Y * 0.2;
            // Face the arena centre.
            let to = -Vec2::new(s.x, s.z);
            p.yaw = if to.length_squared() > 1e-4 { (-to.x).atan2(-to.y) } else { 0.0 };
        }
        self.bullets.clear();
    }

    pub fn step(&mut self, inputs: &[Input]) {
        self.events.clear();
        self.tick += 1;
        let mut inp: Vec<Input> = (0..self.players.len()).map(|i| inputs.get(i).copied().unwrap_or_default()).collect();
        for i in 0..self.players.len() {
            if self.players[i].bot && self.players[i].alive && !self.between_rounds() {
                let mut brain = std::mem::take(&mut self.brains[i]);
                inp[i] = brain.think(self, i);
                self.brains[i] = brain;
            }
        }
        for i in 0..self.players.len() {
            if self.players[i].alive { self.move_player(i, inp[i]); }
        }
        for i in 0..self.players.len() {
            if self.players[i].alive { self.weapon(i, inp[i]); }
        }
        self.step_bullets();
        for i in 0..self.players.len() {
            if self.players[i].alive && self.arena.in_lava(self.players[i].pos) { self.kill(i, None); }
        }
        self.picks(&inp);
        self.rounds();
    }

    fn move_player(&mut self, i: usize, input: Input) {
        let mut ev = Vec::new();
        let shapes = &self.arena.shapes;
        let p = &mut self.players[i];
        p.yaw += input.look[0];
        p.pitch = (p.pitch + input.look[1]).clamp(-1.5, 1.5);
        let (sy, cy) = p.yaw.sin_cos();
        let fwd = Vec3::new(-sy, 0.0, -cy);
        let right = Vec3::new(cy, 0.0, -sy);
        let mv = Vec2::from(input.mv);
        let mv = if mv.length() > 1.0 { mv.normalize() } else { mv };
        let wish = right * mv.x + fwd * mv.y;

        p.wall_jump_cd = (p.wall_jump_cd - DT).max(0.0);
        if p.dash_charges < MAX_DASH_CHARGES {
            p.dash_recharge += DT;
            if p.dash_recharge >= DASH_RECHARGE_TIME { p.dash_recharge = 0.0; p.dash_charges += 1; }
        }
        if input.jump {
            if p.on_floor {
                p.vel.y = JUMP_VELOCITY;
                p.jumps_left = 1;
                ev.push(Event::Jump { player: i });
            } else if let (Some(n), true) = (p.wall_normal, p.wall_jump_cd <= 0.0) {
                p.vel.y = WALL_JUMP_V;
                p.vel.x += n.x * WALL_JUMP_H;
                p.vel.z += n.z * WALL_JUMP_H;
                p.wall_jump_cd = WALL_JUMP_COOLDOWN;
                p.jumps_left = 1;
                ev.push(Event::Jump { player: i });
            } else if p.jumps_left > 0 {
                p.vel.y = DOUBLE_JUMP_VELOCITY;
                p.jumps_left -= 1;
                ev.push(Event::Jump { player: i });
            }
        }
        if input.dash && p.dash_charges > 0 {
            p.dash_dir = if wish.length_squared() < 1e-4 { fwd } else { wish.normalize() };
            p.dash_timer = DASH_TIME;
            p.dash_charges -= 1;
            ev.push(Event::Dash { player: i });
        }

        let walk = WALK_SPEED * p.mods.speed;
        let mut target = wish * walk;
        let horiz = Vec3::new(p.vel.x, 0.0, p.vel.z);
        let speeding = horiz.length() > walk + 0.1;
        let accel = if p.dash_timer > 0.0 {
            p.dash_timer -= DT;
            p.vel.y = p.vel.y.max(0.0);
            let k = (p.dash_timer / (DASH_TIME * 0.5)).clamp(0.0, 1.0);
            target = target.lerp(p.dash_dir * DASH_SPEED, k);
            2000.0
        } else if p.on_floor {
            if speeding && wish.dot(horiz.normalize_or_zero()) <= 0.5 { FRICTION } else { GROUND_ACCEL }
        } else {
            AIR_ACCEL
        };
        p.vel.x = move_toward(p.vel.x, target.x, accel * DT);
        p.vel.z = move_toward(p.vel.z, target.z, accel * DT);
        if p.on_floor && wish == Vec3::ZERO && p.dash_timer <= 0.0 && !speeding {
            let f = FRICTION * DT * WALK_SPEED * 0.1;
            p.vel.x = move_toward(p.vel.x, 0.0, f);
            p.vel.z = move_toward(p.vel.z, 0.0, f);
        }
        if p.dash_timer <= 0.0 { p.vel.y = (p.vel.y - GRAVITY * DT).max(-MAX_FALL); }

        // Sub-steps short enough that no thin block is skipped.
        let contact = MOVER.slide(shapes, &mut p.pos, &mut p.vel, DT, 0.25);
        p.on_floor = contact.on_floor;
        p.wall_normal = contact.wall_normal;
        if p.on_floor { p.wall_normal = None; p.jumps_left = p.jumps_left.max(1); }
        self.events.extend(ev);
    }

    fn weapon(&mut self, i: usize, input: Input) {
        let between = self.between_rounds();
        let p = &mut self.players[i];
        p.fire_cd = (p.fire_cd - DT).max(0.0);
        p.bloom = (p.bloom - DT * 0.12).max(0.0);
        p.since_shot += DT;
        if p.reload_timer > 0.0 {
            p.reload_timer -= DT;
            if p.reload_timer <= 0.0 { p.reload_timer = 0.0; p.ammo = p.mag_size(); }
            return;
        }
        if (input.reload && p.ammo < p.mag_size()) || p.ammo == 0 {
            p.reload_timer = RELOAD_TIME / p.mods.reload;
            return;
        }
        if !input.fire || p.fire_cd > 0.0 || between { return; }
        p.fire_cd = FIRE_INTERVAL / p.mods.fire_rate;
        p.ammo -= 1;
        p.since_shot = 0.0;
        let cone = SPREAD * p.mods.spread + p.bloom;
        let (yaw, pitch) = (p.yaw + self.rng.range(-cone, cone), p.pitch + self.rng.range(-cone, cone));
        let dir = look_dir(yaw, pitch);
        p.bloom = (p.bloom + RECOIL_BLOOM * p.mods.recoil).min(0.08);
        p.pitch = (p.pitch + RECOIL_PITCH * p.mods.recoil).min(1.5);
        let from = p.eye();
        let speed = BULLET_SPEED * p.mods.bullet_speed;
        self.bullets.push(Bullet { pos: from, vel: dir * speed, owner: i, travelled: 0.0 });
        self.events.push(Event::Shot { player: i, from, dir });
    }

    fn step_bullets(&mut self) {
        let mut bullets = std::mem::take(&mut self.bullets);
        bullets.retain_mut(|b| {
            let speed = b.vel.length();
            let len = speed * DT;
            let dir = b.vel / speed;
            let wall = shape::raycast(&self.arena.shapes, b.pos, dir, len);
            let mut best = wall.map(|(t, _, _)| t).unwrap_or(len);
            let mut target = None;
            for (j, p) in self.players.iter().enumerate() {
                if j == b.owner || !p.alive { continue; }
                for &(oy, r, head) in &HITBOXES {
                    if let Some(t) = shape::ray_sphere(b.pos, dir, p.pos + Vec3::Y * oy, r, best) {
                        if t < best || (t <= best && head) { best = t; target = Some((j, head)); }
                    }
                }
            }
            let at = b.pos + dir * best;
            if let Some((j, head)) = target {
                let dmg = (DAMAGE * self.players[b.owner].mods.damage * if head { HEADSHOT_MULT } else { 1.0 }).round() as i32;
                self.damage(j, b.owner, dmg, head, at, dir);
                return false;
            }
            if let Some((_, n, _)) = wall {
                self.events.push(Event::Impact { pos: at, normal: n });
                return false;
            }
            b.pos = at;
            b.travelled += len;
            b.travelled < RANGE
        });
        bullets.extend(std::mem::take(&mut self.bullets));
        self.bullets = bullets;
    }

    fn damage(&mut self, victim: usize, attacker: usize, dmg: i32, head: bool, pos: Vec3, dir: Vec3) {
        let v = &mut self.players[victim];
        v.health -= dmg;
        v.vel += dir * dmg as f32 * KNOCKBACK_PER_DAMAGE;
        self.events.push(Event::Hit { victim, attacker, damage: dmg, head, pos });
        if v.health <= 0 { self.kill(victim, Some(attacker)); }
    }

    fn kill(&mut self, victim: usize, attacker: Option<usize>) {
        let v = &mut self.players[victim];
        if !v.alive { return; }
        v.alive = false;
        v.health = 0;
        self.events.push(Event::Kill { victim, attacker });
    }

    /// After a round's end, until every loser has picked a card.
    pub fn between_rounds(&self) -> bool { self.ended }

    /// Apply card choices; bots choose at once.
    fn picks(&mut self, inputs: &[Input]) {
        for i in 0..self.players.len() {
            let Some(offer) = self.players[i].offer else { continue };
            let choice = if self.players[i].bot { Some(self.rng.below(3) as u8) } else { inputs.get(i).and_then(|inp| inp.pick) };
            if let Some(k) = choice.filter(|&k| k < 3) {
                let card = offer[k as usize];
                let p = &mut self.players[i];
                p.mods.apply(card);
                p.cards.push(card);
                p.offer = None;
                self.events.push(Event::Picked { player: i, card });
            }
        }
    }

    fn rounds(&mut self) {
        if self.ended {
            self.intermission = (self.intermission - DT).max(0.0);
            self.pick_time = (self.pick_time - DT).max(0.0);
            let waiting = self.players.iter().any(|p| p.offer.is_some());
            if waiting && self.pick_time <= 0.0 {
                // Out of time: take the first card.
                let auto: Vec<Input> = (0..self.players.len()).map(|_| Input { pick: Some(0), ..Default::default() }).collect();
                self.picks(&auto);
            }
            if self.intermission <= 0.0 && !self.players.iter().any(|p| p.offer.is_some()) {
                self.ended = false;
                self.round += 1;
                self.arena = Arena::generate(self.rng.next_u32() as u64);
                self.spawn_all();
                self.events.push(Event::NewArena);
            }
            return;
        }
        let alive: Vec<usize> = (0..self.players.len()).filter(|&i| self.players[i].alive).collect();
        let over = if self.players.len() > 1 { alive.len() <= 1 } else { alive.is_empty() };
        if over {
            let winner = alive.first().copied();
            if let Some(w) = winner { self.players[w].score += 1; }
            self.intermission = INTERMISSION;
            self.pick_time = PICK_TIME;
            self.ended = true;
            for i in 0..self.players.len() {
                if Some(i) != winner { self.players[i].offer = Some(cards::offer(&mut self.rng)); }
            }
            self.events.push(Event::RoundOver { winner });
        }
    }

    /// Everything a remote viewer needs for one tick, arena excepted.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { tick: self.tick, round: self.round, intermission: self.intermission, pick_time: self.pick_time, ended: self.ended, players: self.players.clone(), bullets: self.bullets.clone(), events: self.events.clone() }
    }

    /// Replace the state with a host snapshot (clients never step).
    pub fn apply(&mut self, s: Snapshot) {
        self.tick = s.tick;
        self.round = s.round;
        self.intermission = s.intermission;
        self.pick_time = s.pick_time;
        self.ended = s.ended;
        self.players = s.players;
        self.bullets = s.bullets;
        self.events = s.events;
    }

    /// A client's view of a host world: the host's arena, empty state until
    /// the first snapshot arrives.
    pub fn remote(arena: Arena, players: usize) -> Self {
        let brains = (0..players).map(|_| bot::Brain::default()).collect();
        World { arena, players: (0..players).map(|_| Player::new(true)).collect(), bullets: Vec::new(), tick: 0, round: 1, intermission: 0.0, pick_time: 0.0, ended: false, events: Vec::new(), rng: Rng::new(0), brains }
    }

    /// Hand a player slot to a human (remote join) or back to a bot.
    pub fn set_bot(&mut self, i: usize, bot: bool) { self.players[i].bot = bot; }

    /// Order-independent digest of the state, for determinism checks.
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |x: u32| { h ^= x as u64; h = h.wrapping_mul(0x100_0000_01b3); };
        mix(self.tick as u32);
        mix(self.round);
        for p in &self.players {
            for v in [p.pos.x, p.pos.y, p.pos.z, p.vel.x, p.vel.y, p.vel.z, p.yaw, p.pitch] { mix(v.to_bits()); }
            mix(p.health as u32);
            mix(p.score);
            mix(p.ammo);
        }
        mix(self.bullets.len() as u32);
        h
    }
}

/// Bots-only stepping for the engine's tools (`kern::run`, `Trace`,
/// `first_divergence`). Query paths address the snapshot JSON, e.g.
/// `players.0.health`, `players.2.pos.1`, `round`, `bullets`.
impl kern::Sim for World {
    fn step(&mut self, timings: &mut kern::Timings) { timings.span("world", || World::step(self, &[])); }
    fn tick(&self) -> u64 { self.tick }
    fn digest(&self, d: &mut kern::Digest) { d.u64(World::digest(self)); }
    fn query(&self, path: &str) -> Option<serde_json::Value> {
        let v = serde_json::to_value(self.snapshot()).ok()?;
        let parts: Vec<&str> = path.split('.').collect();
        kern::walk(&v, &parts).cloned()
    }
}

fn move_toward(from: f32, to: f32, delta: f32) -> f32 {
    if (to - from).abs() <= delta { to } else { from + (to - from).signum() * delta }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape::{Block, Kind};

    fn flat_world() -> World {
        let mut w = World::new(1, &[false, false]);
        w.arena.blocks = vec![Block::new(Vec3::new(0.0, -1.5, 0.0), Vec3::new(100.0, 3.0, 100.0), 0.0, Kind::Floor)];
        w.arena.reshape();
        w.players[0].pos = Vec3::new(0.0, HALF_HEIGHT, 0.0);
        w.players[1].pos = Vec3::new(0.0, HALF_HEIGHT, -20.0);
        w.players[0].yaw = 0.0;
        w
    }

    #[test]
    fn walking_reaches_walk_speed_and_stays_on_the_floor() {
        let mut w = flat_world();
        let fwd = Input { mv: [0.0, 1.0], ..Default::default() };
        for _ in 0..60 { w.step(&[fwd, Input::default()]); }
        let p = &w.players[0];
        assert!(p.on_floor);
        assert!((p.vel.length() - WALK_SPEED).abs() < 0.01, "{}", p.vel);
        assert!((p.pos.y - HALF_HEIGHT).abs() < 0.02, "{}", p.pos);
        assert!(p.pos.z < -10.0);
    }

    #[test]
    fn jump_height_matches_godot_and_double_jump_adds_more() {
        let mut w = flat_world();
        w.step(&[Input::default(); 2]);
        w.step(&[Input { jump: true, ..Default::default() }, Input::default()]);
        let mut peak: f32 = 0.0;
        for _ in 0..60 { w.step(&[Input::default(); 2]); peak = peak.max(w.players[0].pos.y); }
        let single = peak - HALF_HEIGHT;
        // v^2 / 2g = 9.01^2 / 60 = 1.35 m
        assert!((single - 1.35).abs() < 0.1, "{single}");
        let mut w = flat_world();
        w.step(&[Input::default(); 2]);
        w.step(&[Input { jump: true, ..Default::default() }, Input::default()]);
        for _ in 0..15 { w.step(&[Input::default(); 2]); }
        w.step(&[Input { jump: true, ..Default::default() }, Input::default()]);
        let mut peak: f32 = 0.0;
        for _ in 0..60 { w.step(&[Input::default(); 2]); peak = peak.max(w.players[0].pos.y); }
        assert!(peak - HALF_HEIGHT > single + 0.9, "{}", peak);
    }

    #[test]
    fn dash_covers_about_four_metres() {
        let mut w = flat_world();
        w.step(&[Input::default(); 2]);
        let z0 = w.players[0].pos.z;
        w.step(&[Input { dash: true, ..Default::default() }, Input::default()]);
        for _ in 0..30 { w.step(&[Input::default(); 2]); }
        let d = z0 - w.players[0].pos.z;
        assert!((3.0..6.0).contains(&d), "{d}");
        assert_eq!(w.players[0].dash_charges, 1);
    }

    #[test]
    fn wall_blocks_and_wall_jump_pushes_away() {
        let mut w = flat_world();
        w.arena.blocks.push(Block::new(Vec3::new(0.0, 5.0, -3.0), Vec3::new(10.0, 10.0, 1.0), 0.0, Kind::Wall));
        w.arena.reshape();
        let fwd = Input { mv: [0.0, 1.0], ..Default::default() };
        for _ in 0..40 { w.step(&[fwd, Input::default()]); }
        assert!(w.players[0].pos.z > -3.0 + 0.5 + RADIUS - 0.01, "{}", w.players[0].pos);
        w.step(&[Input { jump: true, ..fwd }, Input::default()]);
        for _ in 0..5 { w.step(&[fwd, Input::default()]); }
        assert!(w.players[0].wall_normal.is_some());
        let vz = w.players[0].vel.z;
        w.step(&[Input { jump: true, ..fwd }, Input::default()]);
        assert!(w.players[0].vel.y > 9.0);
        assert!(w.players[0].vel.z > vz, "pushed off the wall");
    }

    #[test]
    fn five_body_shots_kill_and_win_the_round() {
        let mut w = flat_world();
        let fire = Input { fire: true, ..Default::default() };
        let mut kills = 0;
        for _ in 0..200 {
            // Keep aim level on the target despite recoil.
            w.players[0].pitch = -0.02;
            w.step(&[fire, Input::default()]);
            kills += w.events.iter().filter(|e| matches!(e, Event::Kill { attacker: Some(0), .. })).count();
            if kills > 0 { break; }
        }
        assert_eq!(kills, 1);
        assert!(!w.players[1].alive);
        for _ in 0..2 { w.step(&[Input::default(); 2]); }
        assert_eq!(w.players[0].score, 1);
        assert!(w.intermission > 0.0);
    }

    #[test]
    fn losers_pick_cards_and_the_next_round_waits_for_them() {
        let mut w = flat_world();
        w.players[1].bot = false;
        let fire = Input { fire: true, ..Default::default() };
        for _ in 0..300 {
            w.players[0].pitch = -0.02;
            w.step(&[fire, Input::default()]);
            if w.between_rounds() { break; }
        }
        assert!(w.players[1].offer.is_some() && w.players[0].offer.is_none(), "the loser gets an offer");
        let round = w.round;
        for _ in 0..(INTERMISSION * 60.0) as usize + 30 { w.step(&[Input::default(); 2]); }
        assert_eq!(w.round, round, "waits for the human pick");
        let card = w.players[1].offer.unwrap()[2];
        w.step(&[Input::default(), Input { pick: Some(2), ..Default::default() }]);
        assert!(w.events.contains(&Event::Picked { player: 1, card }));
        assert_eq!(w.players[1].cards, vec![card]);
        w.step(&[Input::default(); 2]);
        assert_eq!(w.round, round + 1);
        assert_eq!(w.players[1].health, w.players[1].max_health());
        assert_eq!(w.players[1].ammo, w.players[1].mag_size());
        // A second loss without picking: the first card is taken at the deadline.
        let mut m = cards::Mods::default();
        m.apply(cards::Card::BigMag);
        assert_eq!(m.mag_bonus, 20);
    }

    #[test]
    fn bots_play_rounds_deterministically() {
        let run = || {
            let mut w = World::new(7, &[true; 4]);
            let mut rounds = 0;
            for _ in 0..60 * 120 {
                w.step(&[]);
                rounds += w.events.iter().filter(|e| matches!(e, Event::RoundOver { .. })).count();
            }
            (w.digest(), rounds, w.players.iter().map(|p| p.score).sum::<u32>())
        };
        let a = run();
        assert_eq!(a, run());
        assert!(a.1 >= 2, "bots finished {} rounds in two minutes", a.1);
    }
}
