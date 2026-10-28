//! Wave survival (Godot coop, `game.gd` :1093-1316, :1767-2026,
//! :3242-3250, :3359-3389, `player.gd` `apply_enemy_archetype`): the humans
//! against endless waves of bot archetypes that rise out of the ground.
//!
//! Player slots: the first `seats` players are the match's own slots
//! (humans, and empty seats a remote player can take over); wave enemies
//! are bot players appended after them for the wave and dropped when the
//! next wave's round starts, so human indices never move. An empty seat
//! (a bot slot) is parked: dead, a ghost, out of play (Godot despawns all
//! bots when coop starts).
//!
//! A human who takes lethal damage while another human stands goes down
//! instead: invulnerable, rising 10 m over two seconds and holding there
//! until a standing teammate stays within 1.4 m (flat) of where they fell
//! for three seconds; then they drop from the sky. With nobody standing
//! the run is over.
use crate::player::{PHOENIX_RISE, PHOENIX_TICKS, SKY_DROP_SPEED};
use crate::{bot, flow::Phase, shape, Delayed, Event, Player, Weapon, World, DT64, HALF_HEIGHT};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const BASE_ENEMIES: u32 = 3;
pub const ENEMIES_PER_WAVE: u32 = 1;
pub const MAX_ENEMIES: u32 = 100;
pub const HUMAN_HEALTH_MULT: f64 = 3.0;
pub const ENEMY_HEALTH_MULT: f64 = 0.5;
pub const FIRST_SPAWN_DELAY: f64 = 0.65;
pub const SPAWN_INTERVAL: f64 = 1.35;
pub const TELEGRAPH: f64 = 0.5;
/// Rising out of the ground, then the glow fading (Godot HELL_EMERGE_*).
pub const EMERGE_RISE: f64 = 2.5;
pub const EMERGE_GLOW: f64 = 0.45;
const EMERGE_BURIAL_EXTRA: f32 = 0.5;
const HELL_CAPSULE_HEIGHT: f32 = 1.8;
const SPAWN_MIN_FROM_PLAYER: f32 = 8.0;
const SPAWN_MIN_FROM_TARGET: f32 = 10.0;
const SPAWN_MAX_FROM_TARGET: f32 = 24.0;
const SPAWN_MAX_GROUND_HEIGHT: f32 = 5.0;
const SPAWN_MEMORY: usize = 6;
const SPAWN_SAMPLES: usize = 48;
/// Godot SPAWN_CLEARANCE_*: a capsule 2 m tall, 0.58 m radius.
const CLEARANCE_RADIUS: f32 = 0.58;
const CLEARANCE_HALF: f32 = 1.0;
pub const REVIVE_RADIUS: f32 = 1.4;
pub const REVIVE_SECONDS: f64 = 3.0;
/// Where parked seats wait (never drawn: they are dead).
const PARK: Vec3 = Vec3::new(0.0, -500.0, 0.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Archetype { Grunt, Sniper, Grenadier, FlatFragger, Demolition }

impl Archetype {
    pub fn name(self) -> &'static str {
        match self { Archetype::Grunt => "grunt", Archetype::Sniper => "sniper", Archetype::Grenadier => "grenadier", Archetype::FlatFragger => "fragger", Archetype::Demolition => "bomber" }
    }

    /// Godot `_pick_coop_enemy_archetype`: fixed slots first, then chances
    /// (each roll only when its branch is reached).
    pub fn pick_for(wave: u32, index: u32, rng: &mut crate::rng::Rng) -> Self {
        if wave >= 8 && index == 0 { return Archetype::Demolition; }
        if wave >= 6 && index.is_multiple_of(6) { return Archetype::Grenadier; }
        if wave >= 5 && index.is_multiple_of(5) { return Archetype::Sniper; }
        if wave >= 4 && index.is_multiple_of(4) { return Archetype::FlatFragger; }
        if wave >= 7 && rng.f() < 0.22 { return Archetype::Demolition; }
        if wave >= 5 && rng.f() < 0.18 { return Archetype::Grenadier; }
        if wave >= 4 && rng.f() < 0.24 { return Archetype::Sniper; }
        if wave >= 3 && rng.f() < 0.2 { return Archetype::FlatFragger; }
        Archetype::Grunt
    }

    /// Godot `_apply_coop_archetype_body`.
    fn body(self, w: &mut Weapon) {
        (w.body_scale, w.body_scale_axes, w.head_scale) = match self {
            Archetype::Sniper => (0.92, Vec3::new(0.75, 1.6, 0.75), 1.0),
            Archetype::Grenadier => (1.28, Vec3::ONE, 1.0),
            Archetype::FlatFragger => (0.86, Vec3::new(0.65, 1.0, 1.45), 1.0),
            Archetype::Demolition => (1.08, Vec3::ONE, 1.0),
            Archetype::Grunt => (0.78, Vec3::ONE, 0.9),
        };
    }

    fn height_scale(self) -> f32 {
        let mut w = Weapon::default();
        self.body(&mut w);
        w.body_scale * w.body_scale_axes.y
    }

    /// Godot `apply_enemy_archetype(arch, wave)`: a fresh weapon (no cards).
    pub fn weapon(self, wave: u32) -> Weapon {
        let mut w = Weapon::default();
        self.body(&mut w);
        let wf = wave as f32;
        match self {
            Archetype::Sniper => {
                (w.damage_mult, w.fire_rate_mult, w.reload_mult, w.mag_size_bonus) = (1.45 + wf * 0.04, 0.38, 0.85, -3);
                (w.spread, w.recoil_per_shot, w.bullet_speed_mult, w.bullet_drop, w.headshot_mult) = (0.0018, 0.006, 2.8, 0.0, 1.35);
                w.bullet_color = [0.72, 0.38, 1.0];
                w.special = crate::Special::Zoom;
            }
            Archetype::Grenadier => {
                (w.damage_mult, w.fire_rate_mult, w.reload_mult, w.move_speed_mult, w.mag_size_bonus) = (0.62 + wf * 0.03, 0.32, 0.72, 0.72, -1);
                (w.spread, w.recoil_per_shot, w.bullet_speed_mult) = (0.014, 0.018, 0.68);
                w.max_hp_bonus += 60;
                w.explosive_radius = 3.4 + (wf * 0.12).min(1.6);
                w.explosive_damage = 28.0 + wf * 3.0;
                w.bullet_color = [1.0, 0.48, 0.14];
            }
            Archetype::FlatFragger => {
                (w.damage_mult, w.fire_rate_mult, w.reload_mult, w.mag_size_bonus) = (0.72 + wf * 0.028, 0.28, 0.62, -2);
                w.extra_projectiles += 2;
                (w.spread, w.recoil_per_shot, w.bullet_speed_mult) = (4f32.to_radians(), 0.022, 0.7);
                w.bullet_color = [1.0, 0.42, 0.62];
            }
            Archetype::Demolition => {
                (w.damage_mult, w.fire_rate_mult, w.reload_mult, w.mag_size_bonus) = (0.72 + wf * 0.025, 0.52, 0.8, -2);
                (w.spread, w.bullet_speed_mult) = (0.018, 0.72);
                w.explosive_radius = 3.2 + (wf * 0.18).min(2.8);
                w.explosive_damage = 35.0 + wf * 4.0;
                w.special = if wave >= 10 { crate::Special::ClusterGrenade } else { crate::Special::Grenade };
                w.special_cooldown_mult = 0.78;
                w.bullet_color = [1.0, 0.42, 0.12];
            }
            Archetype::Grunt => {
                (w.damage_mult, w.fire_rate_mult, w.reload_mult, w.mag_size_bonus) = (0.48 + wf * 0.025, 0.9 + (wf * 0.035).min(0.5), 1.12, -2);
                (w.spread, w.recoil_per_shot, w.bullet_speed_mult) = (0.018, 0.012, 0.82);
                w.bullet_color = [1.0, 0.82, 0.28];
            }
        }
        w
    }
}

/// The next enemy, telegraphed where it will rise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Incoming {
    pub archetype: Archetype,
    /// The ground point it rises from.
    pub ground: Vec3,
    /// Where its capsule centre ends up.
    pub stand: Vec3,
    pub yaw: f32,
    /// Seconds of telegraph left.
    pub left: f64,
}

/// A human who is down, waiting for a teammate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Downed {
    pub player: usize,
    /// Where they fell; a reviver stands here.
    pub corpse: Vec3,
    /// Seconds of revive by the current reviver.
    pub progress: f64,
    pub reviver: Option<usize>,
    /// Ticks of the ascent done (holds at PHOENIX_TICKS).
    pub ascent: u32,
}

/// An enemy rising out of the ground: hurtable, but it does not act.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Emerge { pub player: usize, pub from: Vec3, pub to: Vec3, pub elapsed: f64 }

/// Everything about the waves, sent to clients with each snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Waves {
    pub wave: u32,
    pub kills: u32,
    pub total: u32,
    /// Players before this index are the match's slots; after it, enemies.
    pub seats: usize,
    pub incoming: Option<Incoming>,
    pub downed: Vec<Downed>,
    pub emerging: Vec<Emerge>,
    /// This wave's enemies still to come, in order.
    pub queue: Vec<Archetype>,
    /// Seconds to the next pop (paused while one is incoming).
    pub timer: f64,
    /// The last few rise sites; new ones spread away from them.
    pub recent: Vec<Vec3>,
    /// Archetype per enemy slot (index - seats).
    pub archetypes: Vec<Archetype>,
}

impl Waves {
    pub fn is_enemy(&self, i: usize) -> bool { i >= self.seats }
    pub fn downed(&self, i: usize) -> Option<&Downed> { self.downed.iter().find(|d| d.player == i) }
    /// Who `i` is reviving, with progress 0..1.
    pub fn reviving(&self, i: usize) -> Option<(usize, f32)> {
        self.downed.iter().find(|d| d.reviver == Some(i)).map(|d| (d.player, (d.progress / REVIVE_SECONDS).clamp(0.0, 1.0) as f32))
    }
    pub fn emerging(&self, i: usize) -> bool { self.emerging.iter().any(|e| e.player == i) }
    pub fn archetype(&self, i: usize) -> Option<Archetype> { i.checked_sub(self.seats).and_then(|k| self.archetypes.get(k)).copied() }
}

/// Godot `_coop_enemy_count_for_wave`.
pub fn enemy_count(wave: u32, humans: u32) -> u32 {
    (BASE_ENEMIES + wave.saturating_sub(1) * ENEMIES_PER_WAVE + humans.max(1) - 1).clamp(1, MAX_ENEMIES)
}

impl World {
    /// A human slot, playing (not an empty seat, not an enemy).
    fn coop_human(&self, i: usize) -> bool { i < self.waves.seats && !self.players[i].bot }

    /// Godot `_standing_human_ids`: humans up and in the fight.
    fn standing(&self, i: usize) -> bool {
        let p = &self.players[i];
        self.coop_human(i) && p.alive && !p.ghost && !p.downed && !self.flow.is_eliminated(i) && (p.health > 0 || p.phoenix.is_some())
    }

    fn standing_humans(&self, except: Option<usize>) -> Vec<usize> {
        (0..self.players.len().min(self.waves.seats)).filter(|&i| Some(i) != except && self.standing(i)).collect()
    }

    pub(crate) fn coop_human_max_health(&self, i: usize) -> i32 {
        ((self.players[i].max_health() as f64 * HUMAN_HEALTH_MULT).ceil() as i32).max(1)
    }

    /// Godot `_apply_coop_spawn_health`.
    pub(crate) fn coop_spawn_health(&mut self, i: usize) {
        let mult = if self.players[i].bot { ENEMY_HEALTH_MULT } else { HUMAN_HEALTH_MULT };
        let p = &mut self.players[i];
        p.health = ((p.max_health() as f64 * mult).ceil() as i32).max(1);
    }

    /// Round start, before anyone respawns: the last wave's enemies go.
    /// The first time, the match's players become the seats.
    pub(crate) fn coop_clear_enemies(&mut self) {
        if self.waves.seats == 0 {
            self.waves = Waves { wave: self.waves.wave.max(1), seats: self.players.len(), ..Waves::default() };
        }
        let n = self.waves.seats.min(self.players.len());
        self.players.truncate(n);
        self.brains.truncate(n);
        // Nothing may point at a player that is gone.
        self.events.retain(|e| event_players(e).iter().all(|&p| p < n));
        self.delayed.retain(|(_, d)| match *d {
            Delayed::Hit { victim, from, .. } => victim < n && from.is_none_or(|f| f < n),
            Delayed::Credit { shooter, .. } => shooter < n,
            Delayed::Echo { player } => player < n,
        });
        self.damages.retain(|&(v, _, by)| v < n && by.is_none_or(|b| b < n));
        self.gives.retain(|&(p, _)| p < n);
        for p in &mut self.players { if p.poison_from.is_some_and(|f| f >= n) { p.poison_from = None; } }
        let (wave, seats) = (self.waves.wave.max(1), n);
        self.waves = Waves { wave, seats, ..Waves::default() };
    }

    /// Round start, after the spawns (Godot `_begin_coop_enemy_wave`):
    /// empty seats parked, humans at triple health, the wave queued.
    pub(crate) fn coop_begin_wave(&mut self) {
        for i in 0..self.waves.seats.min(self.players.len()) {
            if self.players[i].bot { park(&mut self.players[i]); } else { self.coop_spawn_health(i); }
        }
        let humans = (0..self.waves.seats.min(self.players.len())).filter(|&i| !self.players[i].bot).count() as u32;
        let total = enemy_count(self.waves.wave, humans);
        let wave = self.waves.wave;
        self.waves.queue = (0..total).map(|k| Archetype::pick_for(wave, k, &mut self.rng)).collect();
        self.waves.timer = if total > 0 { FIRST_SPAWN_DELAY } else { -1.0 };
        self.waves.total = total;
        self.waves.kills = 0;
        self.waves.recent.clear();
    }

    /// From `maybe_finish_picks`, before the next round starts.
    pub(crate) fn coop_next_wave(&mut self) { self.waves.wave += 1; }

    /// Start over from wave 1 (Godot rematch: "TRY AGAIN FROM WAVE 1").
    pub(crate) fn coop_restart(&mut self) { self.waves.wave = 1; }

    /// Every tick while playing (Godot `_process`): the spawner, then revives.
    pub(crate) fn coop_tick(&mut self) {
        // A seat whose remote player left becomes a bot: out of play.
        for i in 0..self.waves.seats.min(self.players.len()) {
            if self.players[i].bot && self.players[i].alive {
                self.waves.downed.retain(|d| d.player != i);
                park(&mut self.players[i]);
            }
        }
        self.spawner_tick();
        self.revive_tick();
    }

    /// Godot `_update_coop_enemy_spawner`: one enemy incoming at a time;
    /// the pop timer waits while one is.
    fn spawner_tick(&mut self) {
        if let Some(inc) = self.waves.incoming.as_mut() {
            inc.left -= DT64;
            if inc.left <= 0.0 {
                let inc = self.waves.incoming.take().expect("checked above");
                self.spawn_enemy(&inc);
            }
        }
        if self.waves.incoming.is_some() || self.waves.queue.is_empty() { return; }
        self.waves.timer -= DT64;
        if self.waves.timer > 0.0 { return; }
        let archetype = self.waves.queue.remove(0);
        let (ground, yaw) = self.pick_enemy_site();
        let half = HALF_HEIGHT;
        let stand = ground_below(self, ground).map_or(ground, |y| Vec3::new(ground.x, y + half, ground.z));
        let floor = stand - Vec3::Y * half;
        self.waves.incoming = Some(Incoming { archetype, ground: floor, stand, yaw, left: TELEGRAPH });
        self.waves.timer = SPAWN_INTERVAL;
    }

    /// Godot `_spawn_coop_enemy`: a bot with the archetype's weapon, half
    /// health, buried and rising.
    fn spawn_enemy(&mut self, inc: &Incoming) {
        let mut p = Player::new(true);
        p.weapon = inc.archetype.weapon(self.waves.wave);
        p.ammo = p.mag_size();
        p.jumps_left = 2 + p.weapon.extra_jumps;
        p.phoenix_charges = p.weapon.phoenix_revives;
        p.yaw = inc.yaw;
        let depth = HELL_CAPSULE_HEIGHT * inc.archetype.height_scale() + EMERGE_BURIAL_EXTRA;
        p.pos = inc.stand - Vec3::Y * depth;
        let i = self.players.len();
        let seed = self.rng.next_u32() as u64;
        self.players.push(p);
        self.brains.push(bot::Brain::new(seed));
        self.coop_spawn_health(i);
        let k = i - self.waves.seats;
        self.waves.archetypes.resize(k + 1, Archetype::Grunt);
        self.waves.archetypes[k] = inc.archetype;
        self.waves.emerging.push(Emerge { player: i, from: self.players[i].pos, to: inc.stand, elapsed: 0.0 });
    }

    /// Godot `_pick_coop_enemy_spawn`: ground in a ring around a random
    /// standing human (or wide around the centre), away from everyone and
    /// from the last few sites. Returns the site and the yaw facing the target.
    fn pick_enemy_site(&mut self) -> (Vec3, f32) {
        let humans: Vec<Vec3> = self.standing_humans(None).iter().map(|&i| self.players[i].pos).collect();
        let avoid: Vec<Vec3> = self.players.iter().enumerate()
            .filter(|(i, p)| !(*i < self.waves.seats && p.bot)).map(|(_, p)| p.pos).collect();
        let (target, lo, hi) = if humans.is_empty() {
            (Vec3::ZERO, 4.0, SPAWN_MAX_FROM_TARGET * 1.4)
        } else {
            (humans[self.rng.below(humans.len() as u32) as usize], SPAWN_MIN_FROM_TARGET, SPAWN_MAX_FROM_TARGET)
        };
        let mut best: Option<(f32, Vec3)> = None;
        for _ in 0..SPAWN_SAMPLES {
            let ang = self.rng.f() * std::f32::consts::TAU;
            let d = self.rng.range(lo, hi);
            let Some(ground) = self.ground_site(target + Vec3::new(ang.cos() * d, 0.0, ang.sin() * d)) else { continue };
            if min_flat(ground, &avoid) < SPAWN_MIN_FROM_PLAYER { continue; }
            let spread = if self.waves.recent.is_empty() { 30.0 } else { min_flat(ground, &self.waves.recent).min(30.0) };
            let score = spread + self.rng.range(0.0, 4.0);
            if best.is_none_or(|b| score > b.0) { best = Some((score, ground)); }
        }
        let site = match best {
            Some((_, g)) => g,
            None => {
                let mut spots = self.arena.spawns.clone();
                for k in (1..spots.len()).rev() { let j = self.rng.below(k as u32 + 1) as usize; spots.swap(k, j); }
                crate::flow::pick_spawn(&spots, &avoid)
            }
        };
        self.waves.recent.push(site);
        while self.waves.recent.len() > SPAWN_MEMORY { self.waves.recent.remove(0); }
        let yaw = (-(target.x - site.x)).atan2(-(target.z - site.z));
        (site, yaw)
    }

    /// Godot `_coop_ground_spawn_at`: flat ground at most 5 m up, above the
    /// lava, with room for a body; the point 1.7 m above it.
    fn ground_site(&self, probe: Vec3) -> Option<Vec3> {
        let from = probe + Vec3::Y * 14.0;
        let (t, n, _) = shape::raycast(&self.arena.grid, from, Vec3::NEG_Y, 40.0)?;
        if n.y < 0.75 { return None; }
        let ground = from - Vec3::Y * t;
        if ground.y > SPAWN_MAX_GROUND_HEIGHT || ground.y <= crate::arena::LAVA_KILL_Y + 0.25 { return None; }
        let pos = ground + Vec3::Y * 1.7;
        let r = Vec2::splat(CLEARANCE_RADIUS);
        let c = Vec2::new(pos.x, pos.z);
        let mut blocked = false;
        kern::collide::Boxes::near(&self.arena.grid, c - r, c + r, &mut |_, b| {
            blocked = blocked || b.push_out(pos, CLEARANCE_RADIUS, CLEARANCE_HALF).is_some();
        });
        (!blocked).then_some(pos)
    }

    /// The rise out of the ground (Godot `_update_hell_emerge`), in the
    /// enemy's own tick. True while it lasts: the enemy does nothing else.
    pub(crate) fn emerge_tick(&mut self, i: usize) -> bool {
        let Some(k) = self.waves.emerging.iter().position(|e| e.player == i) else { return false };
        if !self.players[i].alive { self.waves.emerging.remove(k); return false; }
        let e = &mut self.waves.emerging[k];
        e.elapsed += DT64;
        let t = (e.elapsed / EMERGE_RISE).clamp(0.0, 1.0) as f32;
        let (pos, done) = (e.from.lerp(e.to, t), e.elapsed >= EMERGE_RISE + EMERGE_GLOW);
        let p = &mut self.players[i];
        p.vel = Vec3::ZERO;
        p.pos = pos;
        if done { self.waves.emerging.remove(k); }
        true
    }

    /// Godot `handle_coop_human_death`: lethal damage downs a human while
    /// another human stands. False: they die as usual.
    pub(crate) fn coop_down(&mut self, i: usize) -> bool {
        if !self.coop_human(i) { return false; }
        if self.players[i].downed { return true; }
        if self.standing_humans(Some(i)).is_empty() { return false; }
        let p = &mut self.players[i];
        p.downed = true;
        p.health = 0;
        p.frozen = false;
        p.vel = Vec3::ZERO;
        p.reloading = false;
        p.launching = None;
        let corpse = p.pos;
        self.waves.downed.push(Downed { player: i, corpse, progress: 0.0, reviver: None, ascent: 0 });
        if self.standing_humans(None).is_empty() { self.end_coop_match(); }
        true
    }

    /// A downed human's tick: rise like a phoenix, then hold; look around.
    pub(crate) fn downed_tick(&mut self, i: usize, look: [f32; 2]) {
        let p = &mut self.players[i];
        p.vel = Vec3::ZERO;
        p.yaw += look[0];
        p.pitch = (p.pitch + look[1]).clamp(-crate::PITCH_LIMIT, crate::PITCH_LIMIT);
        let Some(d) = self.waves.downed.iter_mut().find(|d| d.player == i) else { return };
        d.ascent = (d.ascent + 1).min(PHOENIX_TICKS);
        p.pos = d.corpse + Vec3::Y * PHOENIX_RISE * (d.ascent as f32 / PHOENIX_TICKS as f32);
    }

    /// Godot `_update_coop_revives`: the nearest standing human within
    /// 1.4 m (flat) of the corpse spot revives; a new reviver starts over.
    fn revive_tick(&mut self) {
        let mut k = 0;
        while k < self.waves.downed.len() {
            let d = &self.waves.downed[k];
            let (victim, corpse) = (d.player, d.corpse);
            let mut best: Option<usize> = None;
            let mut best_d = REVIVE_RADIUS;
            for j in self.standing_humans(Some(victim)) {
                let p = self.players[j].pos;
                let dist = Vec2::new(p.x - corpse.x, p.z - corpse.z).length();
                if dist <= best_d { best_d = dist; best = Some(j); }
            }
            let d = &mut self.waves.downed[k];
            match best {
                None => { d.progress = 0.0; d.reviver = None; }
                Some(r) => {
                    if d.reviver != Some(r) { d.progress = 0.0; }
                    d.reviver = Some(r);
                    d.progress += DT64;
                    if d.progress + 1e-9 >= REVIVE_SECONDS {
                        self.waves.downed.remove(k);
                        self.revive(victim, corpse);
                        continue;
                    }
                }
            }
            k += 1;
        }
    }

    /// Godot `finish_phoenix_revive` for a downed human: a sky drop over
    /// where they fell, facing the centre, at full coop health.
    fn revive(&mut self, i: usize, corpse: Vec3) {
        let at = self.sky_drop_spot(i, corpse);
        let health = self.coop_human_max_health(i);
        let p = &mut self.players[i];
        p.downed = false;
        p.phoenix = None;
        p.pos = at;
        p.vel = Vec3::ZERO;
        let to = -Vec2::new(at.x, at.z);
        p.yaw = if to.length_squared() > 1e-4 { (-to.x).atan2(-to.y) } else { p.yaw };
        p.health = health;
        p.launching = Some(SKY_DROP_SPEED);
    }

    /// Godot `clear_coop_downed_state` for everyone down.
    fn clear_downed(&mut self) {
        for d in std::mem::take(&mut self.waves.downed) {
            let p = &mut self.players[d.player];
            p.downed = false;
            p.phoenix = None;
            p.launching = None;
            p.frozen = false;
            if p.health <= 0 && !p.ghost { p.health = p.max_health(); }
        }
    }

    /// Godot `report_kill`, coop branch (the death is already recorded).
    pub(crate) fn coop_kill(&mut self, victim: usize) {
        if self.waves.is_enemy(victim) { self.waves.kills = (self.waves.kills + 1).min(self.waves.total); }
        self.waves.emerging.retain(|e| e.player != victim);
        if self.standing_humans(None).is_empty() {
            self.end_coop_match();
        } else if self.enemies_alive() == 0 && self.waves.queue.is_empty() && self.waves.incoming.is_none() {
            self.end_coop_wave();
        }
    }

    pub fn enemies_alive(&self) -> usize { self.players.iter().skip(self.waves.seats).filter(|p| p.alive).count() }

    /// Godot `_end_coop_wave`: every human picks a card, then the next wave.
    fn end_coop_wave(&mut self) {
        self.clear_downed();
        self.phase = Phase::Picking;
        for i in 0..self.waves.seats.min(self.players.len()) {
            if !self.players[i].bot { self.begin_pick(i); }
        }
        // `flow_tick` moves on once every offer is taken (not from here:
        // this runs inside a kill, maybe inside a loop over the players).
    }

    /// Godot `_end_coop_match`: the run is over; everyone freezes.
    fn end_coop_match(&mut self) {
        self.clear_downed();
        self.waves.queue.clear();
        self.waves.timer = -1.0;
        self.waves.incoming = None;
        self.phase = Phase::MatchOver;
        for p in &mut self.players { p.frozen = true; }
    }
}

/// An empty seat: dead, a ghost, out of the way.
fn park(p: &mut Player) {
    p.alive = false;
    p.ghost = true;
    p.health = 0;
    p.frozen = true;
    p.downed = false;
    p.offer = None;
    p.pos = PARK;
    p.vel = Vec3::ZERO;
}

/// Godot `hell_emerge_stand_pos`: the floor under a point (from 3 m above
/// to 24 m below).
fn ground_below(w: &World, at: Vec3) -> Option<f32> {
    shape::raycast(&w.arena.grid, at + Vec3::Y * 3.0, Vec3::NEG_Y, 27.0).map(|(t, _, _)| at.y + 3.0 - t)
}

fn min_flat(p: Vec3, others: &[Vec3]) -> f32 {
    others.iter().map(|o| Vec2::new(o.x - p.x, o.z - p.z).length()).fold(f32::INFINITY, f32::min)
}

/// The player indices an event refers to.
fn event_players(e: &Event) -> Vec<usize> {
    match *e {
        Event::Shot { player, .. } | Event::Jump { player } | Event::Dash { player } | Event::Special { player, .. } | Event::Slash { player, .. }
        | Event::Teleport { player, .. } | Event::Phoenix { player } | Event::PickedUp { player, .. } | Event::Picked { player, .. }
        | Event::Landed { player, .. } | Event::Reload { player, .. } | Event::Empty { player } | Event::Lava { player, .. } => vec![player],
        Event::Explosion { owner, .. } => vec![owner],
        Event::Hit { victim, attacker, .. } => vec![victim, attacker],
        Event::Kill { victim, attacker, .. } => vec![victim, attacker.unwrap_or(victim)],
        Event::RoundOver { winner } => winner.into_iter().collect(),
        Event::MatchOver { winner } => vec![winner],
        Event::Impact { .. } | Event::Ricochet { .. } | Event::NewArena => vec![],
    }
}
