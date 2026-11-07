//! Growing Guns gameplay on a fixed 60 Hz tick, a port of the Godot game
//! (`player.gd`, `weapon.gd`, `bullet.gd`, `blast.gd`, `grenade.gd`,
//! `game.gd`) at gameplay parity: the same cards, specials, rounds and
//! hazards. The world is a plain value: the host steps it, clients render
//! snapshots.
//!
//! Godot keeps its timers in f64 with `delta = 1/60`, which decides on
//! which tick a cooldown runs out; gameplay timers here are f64 too (`DT64`)
//! so they expire on the same ticks.
pub mod arena;
pub mod cards;
pub mod destruct;
mod blast;
mod bot;
mod combat;
pub mod coop;
mod flow;
mod player;
pub mod rng;
pub mod shape;
mod specials;
pub mod stadium;
pub mod weapon;

pub use arena::Arena;
pub use blast::{Blast, Los};
pub use coop::{Archetype, Waves};
pub use destruct::{Breakage, Carve, Chunk};
pub use flow::{Modifier, Phase, PickupKind, CARD_PICK_TIMEOUT, MODIFIERS, PICKUPS, ROUNDS_TO_WIN};
pub use player::{PHOENIX_TICKS, HOWITZER_RELOAD};
pub use specials::{AIR_STRIKE_TRAVEL, ION_CHARGE, ION_RADIUS};
pub use weapon::{Special, Weapon};
use glam::Vec3;
use rng::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Game rules from outside the sim (the app's Luau scripts). Called inline
/// at fixed points of the tick, in a fixed order, so a match with the same
/// scripts replays exactly. A hook may change the world; it must not step it.
pub trait Hooks: Send + Sync {
    /// Apply a card's effect to a weapon (cards are data in the mods).
    fn apply_card(&self, _card: &cards::CardDef, _w: &mut Weapon) {}
    /// After a card was taken.
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
/// Godot's physics delta (GDScript floats are f64).
pub const DT64: f64 = 1.0 / TICK_HZ as f64;

pub const WALK_SPEED: f32 = 14.0;
pub const AIR_ACCEL: f32 = 70.0;
pub const GROUND_ACCEL: f32 = 100.0;
pub const FRICTION: f32 = 8.0;
pub const JUMP_VELOCITY: f32 = 9.01;
pub const DOUBLE_JUMP_VELOCITY: f32 = 8.5;
pub const WALL_JUMP_V: f32 = 10.5;
pub const WALL_JUMP_H: f32 = 4.0;
pub const WALL_JUMP_COOLDOWN: f64 = 0.14;
pub const DASH_SPEED: f32 = 28.0;
pub const DASH_TIME: f64 = 0.18;
pub const MAX_DASH_CHARGES: u32 = 2;
pub const DASH_RECHARGE_TIME: f64 = 3.0;
pub const GRAVITY: f32 = 30.0;
/// Bots run slower than people (Godot BOT_MOVE_SPEED).
pub const BOT_MOVE_SPEED: f32 = 8.0;

pub const RADIUS: f32 = 0.4;
pub const HALF_HEIGHT: f32 = 0.9;
const MOVER: kern::collide::Mover = kern::collide::Mover { radius: RADIUS, half_height: HALF_HEIGHT };
/// Camera height above the capsule centre (scaled by the body cards).
pub const EYE: f32 = 0.7;
pub const PITCH_LIMIT: f32 = 1.4;
pub const MAX_HEALTH: i32 = 100;
/// Gun muzzle in camera space (Godot `Muzzle` node).
pub const MUZZLE: Vec3 = Vec3::new(0.3, -0.3, -0.8);
pub const RANGE: f32 = 200.0;
/// Below this the body is gone (Godot `_handle_fell_off_map`).
pub const FELL_OFF_Y: f32 = -30.0;

/// Hit spheres relative to the capsule centre: (offset y, radius, is head).
/// Card body changes scale them into ellipsoids (see `Player::hitboxes`).
pub const HITBOXES: [(f32, f32, bool); 3] = [(-0.20, 0.31, false), (0.16, 0.37, false), (0.64, 0.27, true)];

/// A tracked weapon pose for one tick, in metres relative to the player's
/// capsule centre: a headset's hand, in `Input` so it crosses the network.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackedAim { pub offset: Vec3, pub direction: Vec3 }

/// One tick of input for one player. `jump` and `reload` are edges
/// (pressed this tick); `fire` and `special` are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Input {
    /// Strafe right (+x) and forward (+y), each in [-1, 1].
    pub mv: [f32; 2],
    /// Look delta in radians: yaw (left positive), pitch (up positive).
    pub look: [f32; 2],
    pub jump: bool,
    /// Retained for old replay/network input records; ignored by gameplay.
    pub dash: bool,
    pub fire: bool,
    pub reload: bool,
    /// RMB: the special (held; the sim finds the press).
    #[serde(default)]
    pub special: bool,
    /// Card choice (0..offer length) while this player has an offer.
    #[serde(default)]
    pub pick: Option<u8>,
    /// A headset's yaw and pitch, absolute: they replace `look`.
    #[serde(default)]
    pub head: Option<[f32; 2]>,
    /// A headset's gun.
    #[serde(default)]
    pub tracked: Option<TrackedAim>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    /// Look pitch; `recoil_pitch` rides on top and wears off.
    pub pitch: f32,
    pub recoil_pitch: f32,
    pub on_floor: bool,
    pub wall_normal: Option<Vec3>,
    pub jumps_left: i32,
    pub wall_jump_cd: f64,
    /// Legacy dash fields, retained for saved-state compatibility; always inactive.
    pub dash_timer: f64,
    pub dash_dir: Vec3,
    pub dash_charges: u32,
    pub dash_recharge: f64,
    pub health: i32,
    pub alive: bool,
    /// Dead in a 3+ player round: a walking spectator nobody can hurt.
    pub ghost: bool,
    /// Fixed audience artillery station after elimination.
    #[serde(default)]
    pub howitzer: Option<Vec3>,
    #[serde(default)]
    pub siege_crossbow: bool,
    pub ammo: u32,
    pub reloading: bool,
    /// Shared by firing and reloading (Godot `rifle_cooldown`).
    pub rifle_cd: f64,
    /// Shot-to-shot spread bloom (Godot `_recoil_spread`).
    pub bloom: f32,
    /// Round wins.
    pub score: u32,
    #[serde(default)]
    pub kills: u32,
    pub bot: bool,
    /// Seconds since the last shot, for the gun kick animation.
    pub since_shot: f32,
    /// Cards taken this match, in pick order.
    pub cards: Vec<cards::Card>,
    /// The cards' effect plus this round's pickups and modifier.
    pub weapon: Weapon,
    /// Cards to choose from (usually three).
    pub offer: Option<Vec<cards::Card>>,
    /// Seconds the offer has been open.
    pub offer_age: f64,
    /// Dodge immunity left.
    pub iframes: f64,
    pub slow_mult: f32,
    pub slow_timer: f64,
    pub poison_left: f32,
    pub poison_dps: f32,
    pub poison_accum: f64,
    pub poison_from: Option<usize>,
    pub phoenix_charges: i32,
    /// Phoenix ascent: start position and ticks done (120 in all).
    pub phoenix: Option<(Vec3, u32)>,
    /// Sky drop: falling at this speed until the floor.
    pub launching: Option<f32>,
    /// In the spawn cage: no movement, no damage.
    pub frozen: bool,
    pub lava_accum: f64,
    /// RMB cooldown (Godot `grenade_cooldown`).
    pub special_cd: f64,
    pub air_strike_charges: u32,
    pub zooming: bool,
    /// Buttons held last tick, for press edges.
    pub held_fire: bool,
    pub held_special: bool,
    /// Coop: down, waiting for a teammate.
    #[serde(default)]
    pub downed: bool,
    /// This tick's tracked gun, so others see where a headset player aims.
    #[serde(default)]
    pub tracked: Option<TrackedAim>,
    /// Upward speed recoil has given since leaving the floor (capped by
    /// `RECOIL_LIFT`, so shooting down cannot keep a player airborne).
    #[serde(default)]
    pub recoil_lift: f32,
}

impl Player {
    pub fn new(bot: bool) -> Self {
        Self {
            pos: Vec3::ZERO, vel: Vec3::ZERO, yaw: 0.0, pitch: 0.0, recoil_pitch: 0.0, on_floor: false, wall_normal: None,
            jumps_left: 2, wall_jump_cd: 0.0, dash_timer: 0.0, dash_dir: Vec3::ZERO,
            dash_charges: 0, dash_recharge: 0.0, health: MAX_HEALTH, alive: true, ghost: false, howitzer: None, siege_crossbow: false,
            ammo: weapon::BASE_MAG_SIZE as u32, reloading: false, rifle_cd: 0.0, bloom: 0.0, score: 0, kills: 0, bot, since_shot: 9.0,
            cards: Vec::new(), weapon: Weapon::default(), offer: None, offer_age: 0.0,
            iframes: 0.0, slow_mult: 1.0, slow_timer: 0.0, poison_left: 0.0, poison_dps: 0.0, poison_accum: 0.0, poison_from: None,
            phoenix_charges: 0, phoenix: None, launching: None, frozen: false, lava_accum: 0.0,
            special_cd: 0.0, air_strike_charges: 0, zooming: false, held_fire: false, held_special: false, downed: false, tracked: None, recoil_lift: 0.0,
        }
    }
    /// A headset's view, ignoring anything that is not a number.
    pub(crate) fn apply_head(&mut self, [yaw, pitch]: [f32; 2]) {
        if yaw.is_finite() && pitch.is_finite() { self.yaw = yaw; self.pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT); }
    }
    /// Camera height scales with the body cards.
    pub fn eye_height(&self) -> f32 { EYE * self.weapon.body_scale.max(0.1) * self.weapon.body_scale_axes.y.max(0.1) }
    pub fn eye(&self) -> Vec3 { self.pos + Vec3::Y * self.eye_height() }
    /// Where the aim points, recoil included.
    pub fn forward(&self) -> Vec3 { look_dir(self.yaw, self.pitch + self.recoil_pitch) }
    /// Camera basis: right, up, forward.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.forward();
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        (right, right.cross(f).normalize_or(Vec3::Y), f)
    }
    pub fn muzzle(&self) -> Vec3 {
        let (r, u, f) = self.basis();
        self.eye() + r * MUZZLE.x + u * MUZZLE.y - f * MUZZLE.z
    }
    pub fn reloading(&self) -> bool { self.reloading }
    pub fn max_health(&self) -> i32 { MAX_HEALTH + self.weapon.max_hp_bonus }
    pub fn mag_size(&self) -> u32 { self.weapon.mag_size() }
    /// Can be hurt and seen: alive, not a ghost.
    pub fn artillery_elevation(&self, pitch: f32) -> f32 { (pitch + if self.siege_crossbow { 0.18 } else { 0.55 }).clamp(-0.5, 1.25) }
    pub fn howitzer_elevation(&self) -> f32 { self.artillery_elevation(self.pitch) }
    pub fn artillery_reload(&self) -> f64 { if self.siege_crossbow { 3.0 } else { HOWITZER_RELOAD } }
    pub fn targetable(&self) -> bool { self.alive && !self.ghost }

    /// The hit ellipsoids: centre, semi-axes (in the yaw frame), is head.
    pub fn hitboxes(&self) -> [(Vec3, Vec3, bool); 3] {
        let w = &self.weapon;
        let bs = w.body_scale.max(0.1);
        let hs = w.head_scale.max(0.1);
        let ax = w.body_scale_axes.max(Vec3::splat(0.1));
        HITBOXES.map(|(y, r, head)| {
            let scale = if head { ax * (bs * hs) } else { ax * bs };
            (self.pos + Vec3::Y * (y * bs * ax.y), scale * r, head)
        })
    }
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
    #[serde(default)]
    pub last_in_mag: bool,
    #[serde(default)]
    pub ricochet_left: i32,
    #[serde(default)]
    pub pierce_left: i32,
    #[serde(default)]
    pub ricochet_hits: i32,
    #[serde(default)]
    pub body_bounces: i32,
    /// Rays run every other tick from here (Godot `_query_tick`).
    #[serde(default)]
    pub query_tick: bool,
    #[serde(default)]
    pub last_query: Option<Vec3>,
    /// Players this bullet passes through (shooter, ghosts, bounced bodies).
    #[serde(default)]
    pub skip_players: Vec<u16>,
    /// Arena volumes it drilled through.
    #[serde(default)]
    pub skip_volumes: Vec<u32>,
    /// Spawned this tick: first moves next tick.
    #[serde(default)]
    pub fresh: bool,
}

/// Grenades, cluster fragments, mines and dash bombs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectileKind { Grenade, Cluster, Fragment, Mine, DashBomb, Howitzer, SiegeBolt }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Projectile {
    pub kind: ProjectileKind,
    pub pos: Vec3,
    pub vel: Vec3,
    pub owner: usize,
    pub age: f64,
    /// Touching the world (contacts only count when they start).
    pub contact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrikeKind { AirStrike, IonCannon }

/// An air strike rocket or an ion cannon charging on a point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Strike {
    pub kind: StrikeKind,
    pub target: Vec3,
    pub owner: Option<usize>,
    pub age: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pickup {
    pub kind: PickupKind,
    pub pos: Vec3,
    pub vy: f32,
    /// Seconds on the ground (gone at 45).
    pub grounded: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// A shot from the muzzle (one per pellet): direction and whether the
    /// shooter's silencer hides it.
    Shot { player: usize, from: Vec3, dir: Vec3 },
    /// A bullet hit the arena. `damage` is the chip damage to the stone
    /// (decal and chip size); `strong` hits also wear chunks down.
    Impact { pos: Vec3, normal: Vec3, damage: f32, strong: bool },
    /// An explosion went off (bullet splash, grenade, mine, strike).
    Explosion { pos: Vec3, radius: f32, owner: usize, #[serde(default)] kind: ExplosionKind },
    Hit { victim: usize, attacker: usize, damage: i32, head: bool, pos: Vec3 },
    /// `gib` says how the body goes (Godot `do_ragdoll` death modes) and
    /// `dir` which way the killing blow pushed.
    Kill { victim: usize, attacker: Option<usize>, #[serde(default)] gib: Gib, #[serde(default)] dir: Vec3 },
    Jump { player: usize },
    Dash { player: usize },
    /// A bullet bounced or drilled through.
    Ricochet { pos: Vec3 },
    /// RMB used.
    Special { player: usize, special: Special },
    /// Melee swing (sword).
    Slash { player: usize, from: Vec3, dir: Vec3, range: f32 },
    Teleport { player: usize, from: Vec3, to: Vec3 },
    Phoenix { player: usize },
    PickedUp { player: usize, kind: PickupKind },
    RoundOver { winner: Option<usize> },
    MatchOver { winner: usize },
    Picked { player: usize, card: cards::Card },
    /// A fresh arena was generated: rebuild the scene.
    NewArena,
    /// Landed on the floor after falling at `speed` m/s.
    Landed { player: usize, speed: f32 },
    /// A reload started; it takes `seconds`.
    Reload { player: usize, seconds: f32 },
    /// The trigger was pulled with nothing to fire (reloading or empty).
    Empty { player: usize },
    /// Burned by lava (`fall`: the lava killed, Godot's fall sizzle).
    Lava { player: usize, fall: bool },
    /// A ghost planted a mine.
    MinePlanted { player: usize, pos: Vec3 },
    /// Append-only presentation event: a collision-checked bullet path, clipped
    /// at its first gameplay hit. Corpses react locally without becoming cover.
    BulletPath { from: Vec3, to: Vec3, damage: f32 },
}

/// Presentation identity, independent of damage radius. Older recordings
/// without this tag retain their conventional blast effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum ExplosionKind { #[default] Conventional, Ion }

/// Multipliers the party can set for the next round (Godot GameNight
/// `round_rules`): gravity, bullet drop, body-shot damage, pickup rate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules { pub gravity: f32, pub bullet_drop: f32, pub body_damage: f32, pub pickup_rate: f32 }

impl Default for Rules {
    fn default() -> Self { Rules { gravity: 1.0, bullet_drop: 1.0, body_damage: 1.0, pickup_rate: 1.0 } }
}

/// How a body goes when it dies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Gib {
    /// Tumbles whole (a regular hit).
    #[default]
    Ragdoll,
    /// A headshot: the head bursts, the torso tumbles.
    Head,
    /// Overkill (health at or below OVERKILL_HEALTH) or an explosion: the
    /// whole body bursts into chunks. `severity` 0.4..2.5, `blast` when an
    /// explosion did it.
    Burst { severity: f32, blast: bool },
    /// Lava (Godot `_lava_death`): the body takes the lava's look and
    /// sinks; `fall` when it fell in (a splash, a deeper, slower sink)
    /// rather than burning to death standing in it.
    Lava { fall: bool },
}

/// A killing blow that leaves health here or lower bursts the body.
pub const OVERKILL_HEALTH: i32 = -50;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub round: u32,
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub modifier: Option<Modifier>,
    /// Round-start cage countdown for remote presentation; clients never step.
    #[serde(default)]
    pub cage_left: Option<f32>,
    pub players: Vec<Player>,
    pub bullets: Vec<Bullet>,
    #[serde(default)]
    pub projectiles: Vec<Projectile>,
    #[serde(default)]
    pub strikes: Vec<Strike>,
    #[serde(default)]
    pub pickups: Vec<Pickup>,
    #[serde(default)]
    pub lava: flow::LavaLeak,
    /// Wave survival state (coop matches only).
    #[serde(default)]
    pub waves: Option<coop::Waves>,
    pub events: Vec<Event>,
}

/// Damage and knockback landing later (blast sound delay), and on-hit
/// credit for the shooter.
#[derive(Clone, Debug)]
pub(crate) enum Delayed {
    Hit { victim: usize, from: Option<usize>, dmg: i32, impulse: Vec3, pos: Vec3, radius: f32, severity: f32 },
    Credit { shooter: usize, dmg: i32 },
    /// An echo of the last special.
    Echo { player: usize },
}

#[derive(Clone, Debug)]
pub struct World {
    pub arena: Arena,
    pub players: Vec<Player>,
    pub bullets: Vec<Bullet>,
    pub projectiles: Vec<Projectile>,
    pub strikes: Vec<Strike>,
    pub pickups: Vec<Pickup>,
    pub tick: u64,
    pub round: u32,
    pub phase: Phase,
    /// Round wins that end the match.
    pub rounds_to_win: u32,
    pub modifier: Option<Modifier>,
    /// Chance a round rolls a modifier (Godot 0.30).
    pub modifier_chance: f32,
    /// Party rules on top of the modifier (GameNight settings; 1 is Godot's game).
    pub rules: Rules,
    /// Seconds a picker has before a card is chosen for them (Godot 10).
    pub card_pick_seconds: f64,
    /// Force this modifier every round (debug).
    pub forced_modifier: Option<Modifier>,
    /// The next round only: this modifier, or `Some(None)` for none
    /// (GameNight's "Next round" pick). Taken when the round starts.
    pub next_modifier: Option<Option<Modifier>>,
    /// Every new arena is an island arena (Godot `force_all_floor_lava`;
    /// `--island`, the `arena island` command).
    pub force_island: bool,
    /// Wave survival instead of versus.
    pub coop: bool,
    /// The waves, kills and downed humans (coop).
    pub waves: coop::Waves,
    pub lava: flow::LavaLeak,
    pub events: Vec<Event>,
    /// What this tick did to the arena (sent reliably to clients).
    pub breakage: Breakage,
    /// Debug: bots stand still and hold fire.
    pub bots_paused: bool,
    /// The match's cards (data from the mods; swapped on hot reload).
    pub cards: Arc<cards::Book>,
    pub hooks: HookSlot,
    /// Random numbers for scripts, apart from the sim's own stream.
    pub script_rng: Rng,
    pub(crate) flow: flow::State,
    carves: Vec<Carve>,
    booms: Vec<(Vec3, f32, f32, usize)>,
    rng: Rng,
    brains: Vec<bot::Brain>,
    tracked_aims: Vec<Option<TrackedAim>>,
    gives: Vec<(usize, cards::Card)>,
    damages: Vec<(usize, i32, Option<usize>)>,
    delayed: Vec<(u32, Delayed)>,
}

impl World {
    /// `bots[i]` makes player i computer controlled.
    pub fn new(seed: u64, bots: &[bool]) -> Self { Self::new_with(seed, bots, |_| {}) }

    /// A match with settings (modifier chance, rounds to win, coop) made
    /// before the first round starts.
    pub fn new_with(seed: u64, bots: &[bool], setup: impl FnOnce(&mut World)) -> Self {
        let rng = Rng::new(seed);
        let players = bots.iter().map(|&b| Player::new(b)).collect();
        let brains = (0..bots.len()).map(|i| bot::Brain::new(seed ^ ((i as u64 + 1) * 0x9E37))).collect();
        let mut w = World {
            arena: Arena::empty(), players, bullets: Vec::new(), projectiles: Vec::new(), strikes: Vec::new(), pickups: Vec::new(),
            tick: 0, round: 1, phase: Phase::Playing, rounds_to_win: flow::ROUNDS_TO_WIN, modifier: None, modifier_chance: flow::MODIFIER_CHANCE,
            rules: Rules::default(), card_pick_seconds: flow::CARD_PICK_TIMEOUT,
            forced_modifier: None, next_modifier: None, force_island: false, coop: false, waves: coop::Waves::default(), lava: flow::LavaLeak::default(),
            events: Vec::new(), breakage: Breakage::default(), bots_paused: false,
            cards: Arc::default(), hooks: HookSlot::default(), script_rng: Rng::new(seed ^ 0x5C21_97E5), flow: flow::State::default(),
            carves: Vec::new(), booms: Vec::new(), rng, brains, tracked_aims: Vec::new(), gives: Vec::new(), damages: Vec::new(), delayed: Vec::new(),
        };
        setup(&mut w);
        w.start_round();
        w
    }

    pub fn step(&mut self, inputs: &[Input]) {
        self.step_tracked(inputs, &[]);
    }

    /// Step with optional controller aiming. Movement/look still use Input;
    /// weapon rays use the controller. Invalid/lost poses fall back to normal
    /// aiming; the caller must release fire on tracking loss.
    pub fn step_tracked(&mut self, inputs: &[Input], aims: &[Option<TrackedAim>]) {
        self.tracked_aims.clear();
        // An aim handed in here, else one that came with the player's input.
        let chosen: Vec<Option<TrackedAim>> = (0..self.players.len()).map(|i| aims.get(i).copied().flatten().or(inputs.get(i).and_then(|input| input.tracked))).collect();
        self.tracked_aims.extend(chosen.iter().map(|a| a.filter(|a|
            a.offset.is_finite() && a.offset.length() <= 2.5 &&
            a.direction.is_finite() && a.direction.length_squared().is_finite() && a.direction.length_squared() > 0.5
        ).map(|a| TrackedAim { offset: a.offset, direction: a.direction.normalize() })));
        for (p, a) in self.players.iter_mut().zip(&self.tracked_aims) { p.tracked = *a; }
        for (p, head) in self.players.iter_mut().zip(inputs.iter().map(|i| i.head)) { if let Some(h) = head { p.apply_head(h); } }

        self.events.clear();
        self.breakage = Breakage::default();
        self.tick += 1;
        for (pos, radius, damage, owner) in std::mem::take(&mut self.booms) {
            self.blast(pos, radius, damage, Some(owner), Blast::bullet(self.players.get(owner).map_or(1.0, |p| p.weapon.knockback_mult)));
            self.carves.push(Carve { pos, radius: radius * 0.52, damage });
        }
        for (player, card) in std::mem::take(&mut self.gives) { self.give(player, card); }
        for (victim, dmg, by) in std::mem::take(&mut self.damages) { self.apply_damage(victim, dmg, by, player::Blow::plain(self.players[victim].pos)); }
        let mut inp: Vec<Input> = (0..self.players.len()).map(|i| inputs.get(i).copied().unwrap_or_default()).collect();
        for (i, input) in inp.iter_mut().enumerate() {
            let p = &self.players[i];
            // A player choosing a card gets no gameplay input.
            if p.offer.is_some() && !p.bot { *input = Input { pick: input.pick, ..Default::default() }; }
            // Bots think in their own physics step (bot.rs).
            if p.bot { *input = Input::default(); }
        }
        // Each player runs its whole tick in turn, as Godot's nodes do.
        for (i, &input) in inp.iter().enumerate() {
            self.player_tick(i, input);
            if self.players[i].alive && !self.players[i].ghost && self.arena.stadium.outside(self.players[i].pos) {
                self.players[i].launching = None;
                self.players[i].phoenix = None;
                self.players[i].downed = false;
                self.kill(i, None, Gib::Ragdoll, Vec3::Y);
            }
        }
        self.step_bullets();
        self.step_projectiles();
        self.step_pickups();
        self.step_strikes();
        let carves = std::mem::take(&mut self.carves);
        self.breakage = self.arena.carve(&carves);
        self.lava_tick();
        self.flow_tick(&inp);
        self.step_delayed();
        if let Some(h) = self.hooks.0.clone() { h.tick(self); }
    }

    /// Clip a tracked muzzle against solid geometry so a controller cannot
    /// shoot through cover while the player's body remains behind it.
    pub(crate) fn tracked_ray(&self, i: usize) -> Option<(Vec3, Vec3)> {
        let a = self.tracked_aims.get(i).copied().flatten()?;
        let p = &self.players[i];
        let desired = p.pos + a.offset;
        let delta = desired - p.eye();
        let distance = delta.length();
        let from = if distance > 0.001 {
            let direction = delta / distance;
            self.ray_world(p.eye(), direction, distance, &[])
                .map_or(desired, |(t, _, _)| p.eye() + direction * (t - 0.03).max(0.0))
        } else { desired };
        Some((from, a.direction))
    }

    /// A card's effect and hook, as when picked.
    pub fn give(&mut self, player: usize, card: cards::Card) {
        self.players[player].cards.push(card);
        self.apply_card_to(player, card);
        let p = &mut self.players[player];
        p.ammo = p.ammo.max(p.mag_size()).min(p.mag_size());
        // Godot apply_card tops up health and phoenix charges.
        p.health = if p.alive { p.health.max(p.max_health()) } else { p.health };
        p.phoenix_charges = p.phoenix_charges.max(p.weapon.phoenix_revives);
        if let Some(h) = self.hooks.0.clone() { h.pick(self, player, card); }
    }

    /// Remove one owned copy, rebuilding its stats without replaying pick hooks.
    pub fn remove_card(&mut self, player: usize, card: cards::Card) -> bool {
        let Some(p) = self.players.get_mut(player) else { return false };
        let Some(index) = p.cards.iter().rposition(|c| *c == card) else { return false };
        p.cards.remove(index);
        self.rebuild_weapon(player);
        if let Some(m) = self.modifier { flow::modifier_weapon(m, &mut self.players[player].weapon); }
        let p = &mut self.players[player];
        p.ammo = p.ammo.min(p.mag_size());
        p.health = p.health.min(p.max_health());
        p.phoenix_charges = p.phoenix_charges.min(p.weapon.phoenix_revives);
        true
    }

    fn apply_card_to(&mut self, player: usize, card: cards::Card) {
        let Some(def) = self.cards.get(card).cloned() else { return };
        let book = self.cards.clone();
        let w = &mut self.players[player].weapon;
        book.apply(card, w);
        if let Some(h) = self.hooks.0.clone() { h.apply_card(&def, w); }
    }

    /// The weapon from the owned cards alone (Godot `rebuild_weapon_from_cards`).
    pub fn rebuild_weapon(&mut self, player: usize) {
        self.players[player].weapon = Weapon::default();
        for c in self.players[player].cards.clone() { self.apply_card_to(player, c); }
    }

    /// Owned loadout without borrowed guns or temporary round modifiers.
    /// Presentation caches this at round boundaries, not once per frame.
    pub fn owned_weapon(&self, player: usize) -> Weapon {
        let mut weapon = Weapon::default();
        for &card in &self.players[player].cards {
            let Some(def) = self.cards.get(card) else { continue };
            self.cards.apply(card, &mut weapon);
            if let Some(h) = &self.hooks.0 { h.apply_card(def, &mut weapon); }
        }
        weapon
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

    /// Damage the arena at the end of the next step (debug tools, scripted
    /// events); it goes through the same sweep, breakage and network path.
    pub fn queue_carve(&mut self, c: Carve) { self.carves.push(c); }

    /// An explosion at the start of the next step (debug tools, scripted
    /// events), credited to `owner`, like an explosive round's splash.
    pub fn queue_explosion(&mut self, pos: Vec3, radius: f32, damage: f32, owner: usize) { self.booms.push((pos, radius, damage, owner)); }

    /// Explosions queued for the next step.
    pub fn queued_explosions(&self) -> usize { self.booms.len() }

    /// Damage after the hooks had their say.
    fn hooked_damage(&mut self, victim: usize, attacker: usize, damage: i32, head: bool, explosion: bool) -> i32 {
        match self.hooks.0.clone() {
            Some(h) => h.hit(self, &HitInfo { victim, attacker, damage, head, explosion }),
            None => damage,
        }
    }

    /// After a round's end, until every loser has picked a card.
    pub fn between_rounds(&self) -> bool { self.phase != Phase::Playing }

    /// Everything a remote viewer needs for one tick, arena excepted.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick, round: self.round, phase: self.phase, modifier: self.modifier, cage_left: self.cage_left(), players: self.players.clone(), bullets: self.bullets.clone(),
            projectiles: self.projectiles.clone(), strikes: self.strikes.clone(), pickups: self.pickups.clone(), lava: self.lava.clone(),
            waves: self.coop.then(|| self.waves.clone()), events: self.events.clone(),
        }
    }

    /// Replace the state with a host snapshot (clients never step).
    pub fn apply(&mut self, s: Snapshot) {
        self.tick = s.tick;
        self.round = s.round;
        self.phase = s.phase;
        self.modifier = s.modifier;
        self.apply_cage_left(s.cage_left);
        self.players = s.players;
        self.bullets = s.bullets;
        self.projectiles = s.projectiles;
        self.strikes = s.strikes;
        self.pickups = s.pickups;
        self.lava = s.lava;
        self.coop = s.waves.is_some();
        if let Some(w) = s.waves { self.waves = w; }
        self.events = s.events;
    }

    /// A client's view of a host world: the host's arena, empty state until
    /// the first snapshot arrives.
    pub fn remote(arena: Arena, players: usize) -> Self {
        let mut w = World::new(0, &vec![true; players]);
        w.arena = arena;
        w.apply_cage_left(None);
        w.events.clear();
        w
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
            mix(p.kills);
            mix(p.ammo);
            mix(p.cards.len() as u32);
        }
        mix(self.bullets.len() as u32);
        mix(self.projectiles.len() as u32);
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

pub(crate) fn move_toward(from: f32, to: f32, delta: f32) -> f32 {
    if (to - from).abs() <= delta { to } else { from + (to - from).signum() * delta }
}

/// Ticks until a delay of `seconds` has passed (Godot timers fire on the
/// first frame at or past it).
pub(crate) fn ticks_for(seconds: f64) -> u32 { (seconds * TICK_HZ as f64 - 1e-9).ceil().max(0.0) as u32 }

#[cfg(test)]
mod tests;
