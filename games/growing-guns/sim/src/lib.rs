//! Growing Guns gameplay on a fixed 60 Hz tick: movement, rifle, rounds and
//! bots. Constants come from the Godot game (`player.gd`, `weapon.gd`). The
//! world is a plain value: the host steps it, clients render snapshots.
pub mod arena;
pub mod cards;
pub mod destruct;
mod bot;
pub mod rng;
pub mod shape;
pub mod stadium;

pub use arena::Arena;
pub use destruct::{Breakage, Carve, Chunk};
use glam::{Vec2, Vec3};
use rng::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Game rules from outside the sim (the app's Luau scripts). Called inline
/// at fixed points of the tick, in a fixed order, so a match with the same
/// scripts replays exactly. A hook may change the world; it must not step it.
pub trait Hooks: Send + Sync {
    /// After a card's stat changes were applied.
    fn pick(&self, _w: &mut World, _player: usize, _card: cards::Card) {}
    /// Before damage lands; returns the damage to deal (the given one by default).
    fn hit(&self, _w: &mut World, _hit: &HitInfo) -> i32 { _hit.damage }
    fn kill(&self, _w: &mut World, _victim: usize, _attacker: Option<usize>) {}
    /// At the end of every tick.
    fn tick(&self, _w: &mut World) {}
    /// A new round's arena and spawns are ready.
    fn round_start(&self, _w: &mut World) {}
}

#[derive(Clone, Copy, Debug)]
pub struct HitInfo { pub victim: usize, pub attacker: usize, pub damage: i32, pub head: bool, pub explosion: bool }

/// Hooks plus the knowledge of whether any are set, for `World`'s Debug.
#[derive(Clone, Default)]
pub struct HookSlot(pub Option<Arc<dyn Hooks>>);
impl std::fmt::Debug for HookSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str(if self.0.is_some() { "hooks" } else { "no hooks" }) }
}

/// Script requests per tick (explosions, card grants): enough for any
/// card, low enough that a broken script cannot flood the match.
pub const SCRIPT_QUOTA: usize = 32;

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
/// Explosions launch harder at the same damage.
pub const BLAST_KNOCKBACK_MULT: f32 = 1.5;
/// Splash damage at the edge of the radius, as a fraction (Godot EXPLOSION_EDGE_FALLOFF).
pub const EXPLOSION_EDGE_FALLOFF: f32 = 0.2;
/// Your own explosions hurt you this much.
pub const SELF_SPLASH: f32 = 0.5;

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
    /// A bullet hit the arena. `damage` is the chip damage to the stone
    /// (decal and chip size); `strong` hits also wear chunks down.
    Impact { pos: Vec3, normal: Vec3, damage: f32, strong: bool },
    /// An explosive round went off.
    Explosion { pos: Vec3, radius: f32, owner: usize },
    Hit { victim: usize, attacker: usize, damage: i32, head: bool, pos: Vec3 },
    /// `gib` says how the body goes (Godot `do_ragdoll` death modes) and
    /// `dir` which way the killing blow pushed.
    Kill { victim: usize, attacker: Option<usize>, #[serde(default)] gib: Gib, #[serde(default)] dir: Vec3 },
    Jump { player: usize },
    Dash { player: usize },
    RoundOver { winner: Option<usize> },
    Picked { player: usize, card: cards::Card },
    /// A fresh arena was generated: rebuild the scene.
    NewArena,
}

/// How a body goes when it dies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Gib {
    /// Tumbles whole (a regular hit; lava).
    #[default]
    Ragdoll,
    /// A headshot: the head bursts, the torso tumbles.
    Head,
    /// Overkill (health at or below OVERKILL_HEALTH) or an explosion: the
    /// whole body bursts into chunks. `severity` 0.4..2.5, `blast` when an
    /// explosion did it.
    Burst { severity: f32, blast: bool },
}

/// A killing blow that leaves health here or lower bursts the body.
pub const OVERKILL_HEALTH: i32 = -50;

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
    /// What this tick did to the arena (sent reliably to clients).
    pub breakage: Breakage,
    /// Debug: bots stand still and hold fire.
    pub bots_paused: bool,
    carves: Vec<Carve>,
    booms: Vec<(Vec3, f32, f32, usize)>,
    rng: Rng,
    brains: Vec<bot::Brain>,
    /// The match's cards (data from the mods; swapped on hot reload).
    pub cards: Arc<cards::Book>,
    pub hooks: HookSlot,
    /// Random numbers for scripts, apart from the sim's own stream.
    pub script_rng: Rng,
    gives: Vec<(usize, cards::Card)>,
    damages: Vec<(usize, i32, Option<usize>)>,
}

impl World {
    /// `bots[i]` makes player i computer controlled.
    pub fn new(seed: u64, bots: &[bool]) -> Self {
        let mut rng = Rng::new(seed);
        let arena = Arena::generate(rng.next_u32() as u64);
        let players = bots.iter().map(|&b| Player::new(b)).collect();
        let brains = (0..bots.len()).map(|i| bot::Brain::new(seed ^ ((i as u64 + 1) * 0x9E37))).collect();
        let mut w = World { arena, players, bullets: Vec::new(), tick: 0, round: 1, intermission: 0.0, pick_time: 0.0, ended: false, events: Vec::new(), breakage: Breakage::default(), bots_paused: false, carves: Vec::new(), booms: Vec::new(), rng, brains,
            cards: Arc::default(), hooks: HookSlot::default(), script_rng: Rng::new(seed ^ 0x5C21_97E5), gives: Vec::new(), damages: Vec::new() };
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
        self.breakage = Breakage::default();
        self.tick += 1;
        for (pos, radius, damage, owner) in std::mem::take(&mut self.booms) { self.explode_with(pos, owner, radius, damage); }
        for (player, card) in std::mem::take(&mut self.gives) { self.give(player, card); }
        for (victim, dmg, by) in std::mem::take(&mut self.damages) { self.direct_damage(victim, dmg, by); }
        let mut inp: Vec<Input> = (0..self.players.len()).map(|i| inputs.get(i).copied().unwrap_or_default()).collect();
        for (i, input) in inp.iter_mut().enumerate() {
            if self.players[i].bot && self.players[i].alive && !self.between_rounds() && !self.bots_paused {
                let mut brain = std::mem::take(&mut self.brains[i]);
                *input = brain.think(self, i);
                self.brains[i] = brain;
            }
        }
        for (i, &input) in inp.iter().enumerate() {
            if self.players[i].alive { self.move_player(i, input); }
        }
        for (i, &input) in inp.iter().enumerate() {
            if self.players[i].alive { self.weapon(i, input); }
        }
        self.step_bullets();
        let carves = std::mem::take(&mut self.carves);
        self.breakage = self.arena.carve(&carves);
        for i in 0..self.players.len() {
            if self.players[i].alive && self.arena.in_lava(self.players[i].pos) { self.kill(i, None, Gib::Ragdoll, Vec3::Y); }
        }
        self.picks(&inp);
        self.rounds();
        if let Some(h) = self.hooks.0.clone() { h.tick(self); }
    }

    /// A card's stat changes and hook, as when picked.
    pub fn give(&mut self, player: usize, card: cards::Card) {
        let p = &mut self.players[player];
        self.cards.apply(card, &mut p.mods);
        p.cards.push(card);
        if let Some(h) = self.hooks.0.clone() { h.pick(self, player, card); }
    }

    /// Give a card at the start of the next step (scripts); false over quota.
    pub fn queue_give(&mut self, player: usize, card: cards::Card) -> bool {
        if self.gives.len() >= SCRIPT_QUOTA || player >= self.players.len() { return false; }
        self.gives.push((player, card));
        true
    }

    /// Damage at the start of the next step (scripts), past the hit hooks;
    /// false over quota.
    pub fn queue_damage(&mut self, victim: usize, damage: i32, by: Option<usize>) -> bool {
        if self.damages.len() >= SCRIPT_QUOTA || victim >= self.players.len() { return false; }
        self.damages.push((victim, damage, by));
        true
    }

    fn direct_damage(&mut self, victim: usize, dmg: i32, by: Option<usize>) {
        let v = &mut self.players[victim];
        if !v.alive || dmg <= 0 { return; }
        v.health -= dmg;
        let (health, pos) = (v.health, v.pos);
        self.events.push(Event::Hit { victim, attacker: by.unwrap_or(victim), damage: dmg, head: false, pos });
        if health <= 0 { let g = Self::gib_for(health, false, None); self.kill(victim, by, g, Vec3::Y); }
    }

    /// Explosions queued for the next step.
    pub fn queued_explosions(&self) -> usize { self.booms.len() }

    /// Damage after the hooks had their say.
    fn hooked_damage(&mut self, victim: usize, attacker: usize, damage: i32, head: bool, explosion: bool) -> i32 {
        match self.hooks.0.clone() {
            Some(h) => h.hit(self, &HitInfo { victim, attacker, damage, head, explosion }),
            None => damage,
        }
    }

    fn move_player(&mut self, i: usize, input: Input) {
        let mut ev = Vec::new();
        let shapes = &self.arena.grid;
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
            let wall = shape::raycast(&self.arena.grid, b.pos, dir, len);
            let mut best = wall.map(|(t, _, _)| t).unwrap_or(len);
            let mut target = None;
            for (j, p) in self.players.iter().enumerate() {
                if j == b.owner || !p.alive { continue; }
                for &(oy, r, head) in &HITBOXES {
                    if let Some(t) = shape::ray_sphere(b.pos, dir, p.pos + Vec3::Y * oy, r, best).filter(|&t| t < best || (t <= best && head)) {
                        best = t;
                        target = Some((j, head));
                    }
                }
            }
            let at = b.pos + dir * best;
            let mods = self.players[b.owner].mods;
            if let Some((j, head)) = target {
                let dmg = (DAMAGE * mods.damage * if head { HEADSHOT_MULT } else { 1.0 }).round() as i32;
                self.damage(j, b.owner, dmg, head, at, dir);
                if mods.explosive_radius > 0.0 { self.explode(at - dir * 0.1, b.owner); }
                return false;
            }
            if let Some((_, n, _)) = wall {
                if mods.explosive_radius > 0.0 {
                    self.explode(at + n * 0.1, b.owner);
                } else {
                    // Rifle chip (Godot `carve_from_hit`): every hit marks the
                    // stone, strong ones wear it down.
                    let (damage, radius) = destruct::rifle_chip(mods.damage);
                    let strong = damage >= destruct::MIN_DESTRUCTION_DAMAGE;
                    self.events.push(Event::Impact { pos: at, normal: n, damage, strong });
                    self.carves.push(Carve { pos: at, radius, damage });
                }
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
        let dmg = self.hooked_damage(victim, attacker, dmg, head, false);
        if !self.players[victim].alive { return; }
        let v = &mut self.players[victim];
        v.health -= dmg;
        v.vel += dir * dmg as f32 * KNOCKBACK_PER_DAMAGE;
        self.events.push(Event::Hit { victim, attacker, damage: dmg, head, pos });
        if v.health <= 0 { let g = Self::gib_for(v.health, head, None); self.kill(victim, Some(attacker), g, dir); }
    }

    /// Damage the arena at the end of the next step (debug tools, scripted
    /// events); it goes through the same sweep, breakage and network path.
    pub fn queue_carve(&mut self, c: Carve) { self.carves.push(c); }

    /// Explosive round: carve the arena and splash players in the open
    /// (Godot `Blast.apply` with line of sight to the torso).
    fn explode(&mut self, pos: Vec3, owner: usize) {
        let mods = self.players[owner].mods;
        let (radius, damage) = (mods.explosive_radius, if mods.explosive_damage > 0.0 { mods.explosive_damage } else { 30.0 });
        self.explode_with(pos, owner, radius, damage);
    }

    /// An explosion at the start of the next step (debug tools, scripted
    /// events), credited to `owner`.
    pub fn queue_explosion(&mut self, pos: Vec3, radius: f32, damage: f32, owner: usize) { self.booms.push((pos, radius, damage, owner)); }

    fn explode_with(&mut self, pos: Vec3, owner: usize, radius: f32, damage: f32) {
        self.events.push(Event::Explosion { pos, radius, owner });
        self.carves.push(Carve { pos, radius: radius * 0.52, damage });
        for j in 0..self.players.len() {
            let (alive, at) = (self.players[j].alive, self.players[j].pos);
            if !alive { continue; }
            let dist = pos.distance(at);
            if dist > radius { continue; }
            let (from, to) = (pos + Vec3::Y * 0.1, at + Vec3::Y * 0.1);
            let d = to - from;
            if d.length() > 1e-3 && shape::raycast(&self.arena.grid, from, d.normalize(), d.length()).is_some() { continue; }
            let linear = (1.0 - dist / radius).clamp(0.0, 1.0);
            let falloff = EXPLOSION_EDGE_FALLOFF + (1.0 - EXPLOSION_EDGE_FALLOFF) * linear.sqrt();
            let mut dmg = (damage * falloff) as i32;
            if j == owner { dmg = (dmg as f32 * SELF_SPLASH) as i32; }
            let away = (at - pos).try_normalize().unwrap_or(Vec3::Y);
            let kb = dmg.max(0) as f32 * KNOCKBACK_PER_DAMAGE * BLAST_KNOCKBACK_MULT;
            self.players[j].vel += away * kb + Vec3::Y * kb * 0.25;
            if dmg > 0 { dmg = self.hooked_damage(j, owner, dmg, false, true); }
            if dmg > 0 && self.players[j].alive {
                self.players[j].health -= dmg;
                self.events.push(Event::Hit { victim: j, attacker: owner, damage: dmg, head: false, pos: at });
                if self.players[j].health <= 0 {
                    // Blast severity grows with how close and how big (Godot `blast_severity`).
                    let g = Self::gib_for(self.players[j].health, false, Some((linear * (radius / 4.0).clamp(0.5, 2.0)).clamp(0.4, 2.5)));
                    self.kill(j, Some(owner), g, away);
                }
            }
        }
    }

    fn kill(&mut self, victim: usize, attacker: Option<usize>, gib: Gib, dir: Vec3) {
        let v = &mut self.players[victim];
        if !v.alive { return; }
        v.alive = false;
        v.health = 0;
        self.events.push(Event::Kill { victim, attacker, gib, dir });
        if let Some(h) = self.hooks.0.clone() { h.kill(self, victim, attacker); }
    }

    /// The death mode for a blow that left `health` (Godot `do_ragdoll`).
    fn gib_for(health: i32, head: bool, blast: Option<f32>) -> Gib {
        let overkill = health <= OVERKILL_HEALTH;
        let severity = if overkill { ((-health + OVERKILL_HEALTH) as f32 / 50.0).clamp(0.4, 2.5) } else { 0.0 };
        match blast {
            Some(b) => Gib::Burst { severity: severity.max(b), blast: true },
            None if overkill => Gib::Burst { severity, blast: false },
            None if head => Gib::Head,
            None => Gib::Ragdoll,
        }
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
                self.players[i].offer = None;
                self.give(i, card);
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
                self.carves.clear();
                // Breakage indexes the old arena's chunks.
                self.breakage = Breakage::default();
                self.spawn_all();
                self.events.push(Event::NewArena);
                if let Some(h) = self.hooks.0.clone() { h.round_start(self); }
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
                if Some(i) != winner { self.players[i].offer = self.cards.offer(&mut self.rng); }
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
        World { arena, players: (0..players).map(|_| Player::new(true)).collect(), bullets: Vec::new(), tick: 0, round: 1, intermission: 0.0, pick_time: 0.0, ended: false, events: Vec::new(), breakage: Breakage::default(), bots_paused: false, carves: Vec::new(), booms: Vec::new(), rng: Rng::new(0), brains,
            cards: Arc::default(), hooks: HookSlot::default(), script_rng: Rng::new(0), gives: Vec::new(), damages: Vec::new() }
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
        mix(self.arena.alive_chunks() as u32);
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
        w.cards = Arc::new(cards::test_book());
        w.arena.blocks = vec![Block::new(Vec3::new(0.0, -1.5, 0.0), Vec3::new(100.0, 3.0, 100.0), 0.0, Kind::Floor)];
        w.arena.build_chunks();
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
        w.arena.build_chunks();
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
    fn explosive_rounds_dig_through_the_floor_into_the_lava() {
        let mut w = World::new(3, &[false, false]);
        let spot = Vec3::new(5.0, HALF_HEIGHT + 0.05, 5.0);
        w.players[0].pos = spot;
        w.players[1].pos = Vec3::new(4.0, HALF_HEIGHT + 0.05, 9.0);
        // Three explosive cards' worth.
        (w.players[0].mods.explosive_radius, w.players[0].mods.explosive_damage) = (9.0, 75.0);
        w.players[0].mods.mag_bonus = 200;
        w.players[0].ammo = 200;
        w.players[0].health = 100_000;
        w.players[1].health = 100_000;
        let before = w.arena.alive_chunks();
        let fire = Input { fire: true, ..Default::default() };
        let (mut booms, mut splashed, mut burned) = (0, false, false);
        for _ in 0..1200 {
            w.players[0].pitch = -1.5;
            w.step(&[fire, Input::default()]);
            booms += w.events.iter().filter(|e| matches!(e, Event::Explosion { .. })).count();
            splashed |= w.events.iter().any(|e| matches!(e, Event::Hit { victim: 1, attacker: 0, .. }));
            if w.events.iter().any(|e| matches!(e, Event::Kill { victim: 0, attacker: None, .. })) { burned = true; break; }
        }
        assert!(booms > 0);
        assert!(w.arena.alive_chunks() < before, "nothing broke");
        assert!(splashed, "a player 4 m away in the open takes splash");
        assert!(burned, "dug through three floor layers into the lava: {}", w.players[0].pos);
    }

    #[test]
    fn strong_rifle_hits_wear_a_wall_down_and_weak_ones_do_not() {
        let base = World::new(4, &[false, false]);
        let wall = base.arena.chunks.iter().position(|c| c.kind == Kind::Wall && c.center.y > 1.0 && c.center.y < 3.0).unwrap();
        let c = base.arena.chunks[wall];
        // Stand 6 m inside the arena, facing the wall chunk.
        let inward = -Vec3::new(c.center.x, 0.0, c.center.z).normalize();
        let shoot = |mods: cards::Mods| {
            let mut w = base.clone();
            w.players[0].mods = mods;
            w.players[0].mods.mag_bonus = 300;
            w.players[0].ammo = 300;
            w.players[1].pos = Vec3::new(0.0, -50.0, 0.0);
            w.players[1].health = 1_000_000;
            let eye = c.center + inward * (c.half.z.max(c.half.x) + 6.0);
            let fire = Input { fire: true, ..Default::default() };
            for _ in 0..600 {
                w.players[0].pos = eye - Vec3::Y * EYE;
                w.players[0].vel = Vec3::ZERO;
                let d = (c.center - eye).normalize();
                w.players[0].yaw = (-d.x).atan2(-d.z);
                w.players[0].pitch = d.y.asin();
                w.players[0].bloom = 0.0;
                w.step(&[fire, Input::default()]);
                w.players[1].alive = true;
                w.ended = false;
            }
            w.arena.chunks[wall]
        };
        let weak = shoot(cards::Mods::default());
        assert!(weak.alive && weak.hp == weak.max_hp, "base rifle only marks stone");
        let strong = cards::Mods { damage: 1.5, ..Default::default() };
        assert!(!shoot(strong).alive, "x1.5 damage breaks it");
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
    }

    /// Hooks see hits before they land and can change them, and run in
    /// the tick (a card's script part, round starts, every tick).
    #[test]
    fn hooks_change_damage_and_run_in_the_tick() {
        use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
        #[derive(Default)]
        struct H { ticks: AtomicU32, picks: AtomicU32 }
        impl Hooks for H {
            fn hit(&self, w: &mut World, h: &HitInfo) -> i32 { w.players[h.attacker].health += 1; h.damage * 10 }
            fn tick(&self, _: &mut World) { self.ticks.fetch_add(1, Relaxed); }
            fn pick(&self, w: &mut World, p: usize, _: cards::Card) { self.picks.fetch_add(1, Relaxed); w.players[p].mods.explosive_radius = 3.0; }
        }
        let h = Arc::new(H::default());
        let mut w = flat_world();
        w.hooks = HookSlot(Some(h.clone()));
        let fire = Input { fire: true, ..Default::default() };
        let mut killed = false;
        for _ in 0..120 {
            w.players[0].pitch = -0.02;
            w.step(&[fire, Input::default()]);
            if w.events.iter().any(|e| matches!(e, Event::Kill { victim: 1, .. })) { killed = true; break; }
        }
        assert!(killed, "x10 damage kills with the first hit");
        assert_eq!(w.players[0].health, MAX_HEALTH + 1, "the hook ran once, for the one hit");
        assert!(h.ticks.load(Relaxed) > 0);
        w.give(0, cards::Card(0));
        assert_eq!((h.picks.load(Relaxed), w.players[0].mods.explosive_radius, w.players[0].mods.damage), (1, 3.0, 1.5));
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
