//! One player's tick (Godot `Player._physics_process`): status effects,
//! cooldowns, gravity and jumps, movement, combat input, falling off
//! the map; and the one place health goes down (`_apply_damage`).
use crate::{flow::Phase, look_dir, move_toward, Event, Gib, Input, World};
use crate::{AIR_ACCEL, BOT_MOVE_SPEED, DOUBLE_JUMP_VELOCITY, DT, DT64, FELL_OFF_Y, FRICTION, GRAVITY};
use crate::{GROUND_ACCEL, JUMP_VELOCITY, MOVER, OVERKILL_HEALTH, PITCH_LIMIT, WALK_SPEED, WALL_JUMP_COOLDOWN, WALL_JUMP_H, WALL_JUMP_V};
use glam::{Vec2, Vec3};

/// Poison ticks once this much time has gathered.
const POISON_INTERVAL: f64 = 0.25;
const FLOOR_SNAP: f32 = 0.1;
/// Walk over ankle-height stones and chunk seams without spending a jump.
const STEP_HEIGHT: f32 = 0.35;
pub const PHOENIX_TICKS: u32 = 120;
pub const PHOENIX_RISE: f32 = 10.0;
/// Phoenix and coop revive drop: from this high, at this speed.
pub const SKY_DROP_HEIGHT: f32 = 60.0;
pub const SKY_DROP_SPEED: f32 = 80.0;

pub const HOWITZER_RELOAD: f64 = 4.5;
pub const HOWITZER_SPEED: f32 = 42.0;

#[derive(Clone, Copy, Debug)]
/// `lava`: a lava death if it kills, `Some(true)` falling in (Godot
/// `"lava_fall"`), `Some(false)` burnt standing in it.
pub struct Blow { pub pos: Vec3, pub dir: Vec3, pub head: bool, pub blast_radius: f32, pub severity: f32, pub env: bool, pub lava: Option<bool> }

impl Blow {
    pub fn plain(pos: Vec3) -> Self { Blow { pos, dir: Vec3::Y, head: false, blast_radius: 0.0, severity: 0.0, env: false, lava: None } }
}

impl World {
    pub(crate) fn player_tick(&mut self, i: usize, input: Input) {
        // Coop: an enemy rising out of the ground does nothing else.
        if self.coop && self.emerge_tick(i) { return; }
        self.status_tick(i);
        let p = &self.players[i];
        let special_edge = input.special && !p.held_special;
        let fire_edge = input.fire && !p.held_fire;
        {
            let p = &mut self.players[i];
            p.held_special = input.special;
            p.held_fire = input.fire;
            p.since_shot += DT;
        }
        let p = &self.players[i];
        if p.ghost {
            self.ghost_tick(i, input, fire_edge);
            return;
        }
        if p.downed { self.downed_tick(i, input.look); return; }
        if !p.alive { self.players[i].vel = Vec3::ZERO; return; }
        if p.phoenix.is_some() { self.phoenix_tick(i); return; }
        if let Some(speed) = p.launching { self.launch_tick(i, speed); return; }
        if p.frozen {
            let p = &mut self.players[i];
            p.vel = Vec3::ZERO;
            p.yaw += input.look[0];
            p.pitch = (p.pitch + input.look[1]).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            return;
        }
        if self.players[i].bot { self.bot_physics(i); return; }
        self.cooldowns(i);
        self.decay_recoil(i);
        self.move_player(i, input);
        self.combat_input(i, input, special_edge, fire_edge);
        if self.players[i].alive && self.players[i].pos.y < FELL_OFF_Y {
            let hp = self.players[i].health.max(100);
            self.apply_damage(i, hp, Some(i), Blow { env: true, ..Blow::plain(self.players[i].pos) });
        }
    }

    /// Client-side kinematic prediction for one local human slot `i`: steps
    /// look, movement, jumps, wall-jumps, and arena collision identically
    /// to `player_tick` without spawning server-authoritative projectiles or
    /// emitting duplicate events.
    pub fn predict_player(&mut self, i: usize, input: Input) {
        if i >= self.players.len() { return; }
        self.players[i].bot = false;
        if let Some(head) = input.head { self.players[i].apply_head(head); }
        let p = &self.players[i];
        let input = if p.offer.is_some() {
            Input { pick: input.pick, ..Default::default() }
        } else {
            input
        };
        {
            let p = &mut self.players[i];
            if p.iframes > 0.0 { p.iframes = (p.iframes - DT64).max(0.0); }
            if p.slow_timer > 0.0 {
                p.slow_timer = (p.slow_timer - DT64).max(0.0);
                if p.slow_timer <= 0.0 { p.slow_mult = 1.0; }
            }
            p.held_special = input.special;
            p.held_fire = input.fire;
            p.since_shot += DT;
        }
        let event_len = self.events.len();
        let p = &self.players[i];
        if p.ghost {
            if p.offer.is_none() && p.howitzer.is_some() {
                self.howitzer_aim(i, input);
                self.cooldowns(i);
            } else {
                self.players[i].vel = Vec3::ZERO;
            }
            self.events.truncate(event_len);
            return;
        }
        if p.downed {
            self.downed_tick(i, input.look);
            self.events.truncate(event_len);
            return;
        }
        if !p.alive {
            self.players[i].vel = Vec3::ZERO;
            return;
        }
        if p.phoenix.is_some() || p.launching.is_some() {
            return;
        }
        if p.frozen {
            let p = &mut self.players[i];
            p.vel = Vec3::ZERO;
            p.yaw += input.look[0];
            p.pitch = (p.pitch + input.look[1]).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            return;
        }
        self.cooldowns(i);
        self.move_player_with(i, input, false);
        let p = &mut self.players[i];
        if p.weapon.special == crate::Special::Zoom && p.air_strike_charges == 0 {
            p.zooming = input.special;
        } else {
            p.zooming = false;
        }
        self.events.truncate(event_len);
    }

    /// Eliminated humans operate a stationary audience gun after their pick.
    fn ghost_tick(&mut self, i: usize, input: Input, _fire_edge: bool) {
        let round_over = self.round_over();
        let p = &mut self.players[i];
        if p.bot || p.offer.is_some() {
            p.vel = Vec3::ZERO;
            return;
        }
        if p.howitzer.is_none() {
            if self.phase != Phase::Playing || round_over { return; }
            let angle = crate::stadium::Stadium::seg_ang() * (self.rng.below(crate::stadium::SEGMENTS as u32) as f32 + 0.5);
            let row = 6 + self.rng.below(6) as usize;
            let at = self.arena.stadium.seat_position(angle, row);
            let p = &mut self.players[i];
            p.howitzer = Some(at);
            p.siege_crossbow = self.rng.below(2) == 1;
            p.pos = at;
            p.yaw = at.x.atan2(at.z);
            p.pitch = -0.35;
            p.special_cd = 0.5;
            p.on_floor = true;
            p.wall_normal = None;
        }
        self.howitzer_aim(i, input);
        self.cooldowns(i);
        self.decay_recoil(i);
        // The seat keeps tracking aim and zoom until respawn. Combat ends with the result.
        if input.fire && !round_over && self.phase == Phase::Playing && self.players[i].special_cd <= 0.0 {
            let p = &mut self.players[i];
            p.special_cd = p.artillery_reload();
            p.since_shot = 0.0;
            let dir = look_dir(p.yaw, p.howitzer_elevation());
            let from = p.eye() - Vec3::Y*0.45 + dir*3.2;
            let (kind, speed) = if p.siege_crossbow { (crate::ProjectileKind::SiegeBolt, 150.0) } else { (crate::ProjectileKind::Howitzer, HOWITZER_SPEED) };
            self.spawn_projectile(kind, from, dir*speed, i);
            self.events.push(Event::Shot { player: i, from, dir });
        }
    }

    fn howitzer_aim(&mut self, i: usize, input: Input) {
        let p = &mut self.players[i];
        let Some(at) = p.howitzer else { return; };
        p.pos = at;
        p.vel = Vec3::ZERO;
        p.dash_timer = 0.0;
        p.zooming = p.siege_crossbow && input.special;
        let sensitivity = if p.zooming { 0.20 } else { 0.45 };
        let centre = at.x.atan2(at.z);
        let yaw = p.yaw + input.look[0]*sensitivity;
        let relative = (yaw-centre+std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)-std::f32::consts::PI;
        p.yaw = centre + relative.clamp(-1.1,1.1);
        p.pitch = (p.pitch + input.look[1]*sensitivity).clamp(-0.8,0.35);
    }

    pub fn lava_surface_at(&self, pos: Vec3) -> f32 {
        let l = &self.lava;
        let covered = l.full || l.floor_t() >= 1.0 || (l.floor_t() > 0.0
            && pos.x.abs() <= l.half && pos.z.abs() <= l.half
            && (pos.x.abs() >= l.half - l.inset() || pos.z.abs() >= l.half - l.inset()));
        if l.active && covered { self.arena.lava_surface().max(l.surface()) } else { self.arena.lava_surface() }
    }

    /// Godot `_tick_status_effects`: dodge, slow, poison; runs even while dead.
    fn status_tick(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.iframes > 0.0 { p.iframes = (p.iframes - DT64).max(0.0); }
        if p.slow_timer > 0.0 {
            p.slow_timer = (p.slow_timer - DT64).max(0.0);
            if p.slow_timer <= 0.0 { p.slow_mult = 1.0; }
        }
        if p.poison_left <= 0.0 || p.poison_dps <= 0.0 { return; }
        p.poison_accum += DT64;
        if p.poison_accum < POISON_INTERVAL { return; }
        let dt = p.poison_accum;
        p.poison_accum = 0.0;
        let amount = ((p.poison_dps as f64 * dt).ceil()).min((p.poison_left as f64).ceil()) as i32;
        if amount <= 0 { return; }
        p.poison_left -= amount as f32;
        if p.poison_left <= 0.0 { p.poison_dps = 0.0; }
        let from = p.poison_from;
        let pos = p.pos;
        self.apply_damage(i, amount, from, Blow::plain(pos));
    }

    /// Godot `_tick_cooldowns`: weapon and special, wall jump and reload.
    pub(crate) fn cooldowns(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.rifle_cd = (p.rifle_cd - DT64).max(0.0);
        p.special_cd = (p.special_cd - DT64).max(0.0);
        if p.reloading && p.rifle_cd <= 0.0 { p.ammo = p.mag_size(); p.reloading = false; }
        p.wall_jump_cd = (p.wall_jump_cd - DT64).max(0.0);
        // Legacy serialized fields remain readable; dash has no gameplay effect.
        p.dash_charges = 0;
        p.dash_recharge = 0.0;
        p.dash_timer = 0.0;
        p.dash_dir = Vec3::ZERO;
    }

    /// Godot `_decay_view_recoil`: bloom and the recoil kick wear off.
    fn decay_recoil(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.bloom *= 1.0 - (DT * 4.0).clamp(0.0, 1.0);
        p.recoil_pitch *= 1.0 - DT * 9.0;
    }

    /// Gravity, jumps and horizontal movement, then the slide (humans;
    /// bots move in `bot.rs`).
    fn move_player(&mut self, i: usize, input: Input) {
        self.move_player_with(i, input, true);
    }

    fn move_player_with(&mut self, i: usize, input: Input, authoritative: bool) {
        let gravity = GRAVITY * self.gravity_mult();
        let mut ev = Vec::new();
        {
            let p = &mut self.players[i];
            let bot = p.bot;
            p.yaw += input.look[0];
            p.pitch = (p.pitch + input.look[1]).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            let extra = p.weapon.extra_jumps;

            if !p.on_floor { p.vel.y -= gravity * DT; } else { p.jumps_left = 2 + extra; }
            // Landed, not just lifting off from a shot this tick.
            if p.on_floor && p.vel.y <= 0.0 { p.recoil_lift = 0.0; }
            if input.jump {
                if p.on_floor {
                    p.vel.y = JUMP_VELOCITY;
                    p.jumps_left = 1 + extra;
                    ev.push(Event::Jump { player: i });
                } else if let (Some(n), false, true) = (p.wall_normal, bot, p.wall_jump_cd <= 0.0) {
                    p.vel.y = WALL_JUMP_V;
                    p.vel.x += n.x * WALL_JUMP_H;
                    p.vel.z += n.z * WALL_JUMP_H;
                    p.wall_jump_cd = WALL_JUMP_COOLDOWN;
                    p.jumps_left = 1 + extra;
                    ev.push(Event::Jump { player: i });
                } else if p.jumps_left > 0 {
                    p.vel.y = DOUBLE_JUMP_VELOCITY;
                    p.jumps_left -= 1;
                    ev.push(Event::Jump { player: i });
                }
            }

        }
        {
            let p = &mut self.players[i];
            let bot = p.bot;
            let (sy, cy) = p.yaw.sin_cos();
            let fwd = Vec3::new(-sy, 0.0, -cy);
            let right = Vec3::new(cy, 0.0, -sy);
            let mv = Vec2::from(input.mv);
            let mv = if mv.length() > 1.0 { mv.normalize() } else { mv };
            let wish = right * mv.x + fwd * mv.y;
            let speed = if bot { BOT_MOVE_SPEED } else { WALK_SPEED };
            let walk = speed * p.weapon.move_speed_mult * p.slow_mult;
            let target = wish * walk;
            let horiz = Vec3::new(p.vel.x, 0.0, p.vel.z);
            let speeding = horiz.length() > walk + 0.1;
            let accel = if p.on_floor {
                if !bot && speeding && wish.dot(horiz.normalize_or_zero()) <= 0.5 { FRICTION } else { GROUND_ACCEL }
            } else {
                AIR_ACCEL
            };
            p.vel.x = move_toward(p.vel.x, target.x, accel * DT);
            p.vel.z = move_toward(p.vel.z, target.z, accel * DT);
            if !bot && p.on_floor && wish == Vec3::ZERO && !speeding {
                let f = FRICTION * DT * walk * 0.1;
                p.vel.x = move_toward(p.vel.x, 0.0, f);
                p.vel.z = move_toward(p.vel.z, 0.0, f);
            }
        }
        self.slide(i);
        if authoritative {
            self.events.extend(ev);
        }
    }

    /// `move_and_slide` with Godot's floor snap: a grounded body is pressed
    /// down by up to 0.1 m so it stays on the floor and walks down steps.
    /// Grounded movement can also step over obstacles up to 0.35 m high.
    pub(crate) fn slide(&mut self, i: usize) {
        let p = &mut self.players[i];
        let snap = p.on_floor && p.vel.y <= 0.0;
        let (was_on_floor, fall) = (p.on_floor, -p.vel.y);
        if snap { p.vel.y = -FLOOR_SNAP / DT; }
        // Sub-steps short enough that no thin block is skipped.
        let contact = MOVER.slide_with_step(&self.arena.grid, &mut p.pos, &mut p.vel, DT, 0.25, STEP_HEIGHT, snap);
        if snap && !contact.on_floor {
            // Nothing within the snap: we walked off a ledge; undo the press.
            p.pos.y += FLOOR_SNAP;
            p.vel.y = 0.0;
        }
        p.on_floor = contact.on_floor;
        p.wall_normal = if contact.on_floor { None } else { contact.wall_normal };
        if contact.on_floor && !was_on_floor { self.events.push(crate::Event::Landed { player: i, speed: fall.max(0.0) }); }
    }



    /// Sky drop: a fixed fall, no input, until the floor.
    fn launch_tick(&mut self, i: usize, speed: f32) {
        let p = &mut self.players[i];
        p.vel = Vec3::new(0.0, -speed, 0.0);
        let contact = MOVER.slide(&self.arena.grid, &mut p.pos, &mut p.vel, DT, 0.25);
        p.on_floor = contact.on_floor;
        if contact.on_floor || p.pos.y < FELL_OFF_Y {
            if contact.on_floor { self.events.push(crate::Event::Landed { player: i, speed }); }
            let p = &mut self.players[i];
            p.launching = None;
            p.vel = Vec3::ZERO;
        }
        if self.players[i].pos.y < FELL_OFF_Y {
            let hp = self.players[i].health.max(100);
            self.apply_damage(i, hp, Some(i), Blow { env: true, ..Blow::plain(self.players[i].pos) });
        }
    }

    /// Phoenix ascent: 10 m up over two seconds, immune; then a sky drop.
    fn phoenix_tick(&mut self, i: usize) {
        self.cooldowns(i);
        let Some((from, t)) = self.players[i].phoenix else { return };
        let t = t + 1;
        let p = &mut self.players[i];
        p.vel = Vec3::ZERO;
        p.pos = from + Vec3::Y * PHOENIX_RISE * (t as f32 / PHOENIX_TICKS as f32).min(1.0);
        p.phoenix = Some((from, t));
        if t >= PHOENIX_TICKS && self.phase == Phase::Playing {
            p.phoenix = None;
            let pos = p.pos;
            let at = self.sky_drop_spot(i, pos);
            let p = &mut self.players[i];
            p.pos = at;
            p.launching = Some(SKY_DROP_SPEED);
            if self.coop { self.coop_spawn_health(i); }
        }
    }

    /// Godot `_apply_damage`: the single place health goes down. Returns
    /// whether the damage landed.
    pub(crate) fn apply_damage(&mut self, i: usize, amount: i32, from: Option<usize>, blow: Blow) -> bool {
        let p = &self.players[i];
        if p.ghost || p.downed || p.frozen || !p.alive || p.health <= 0 || p.phoenix.is_some() { return false; }
        if let Some(f) = from && f != i && self.allied(i, f) { return false; }
        if p.iframes > 0.0 { return false; }
        let new = p.health - amount;
        let overkill = new <= OVERKILL_HEALTH;
        let p = &mut self.players[i];
        p.health = new.max(0);
        self.events.push(Event::Hit { victim: i, attacker: from.unwrap_or(i), damage: amount, head: blow.head, pos: blow.pos });
        if p.health > 0 { return true; }
        if p.phoenix_charges > 0 && !blow.env_lava() {
            p.phoenix_charges -= 1;
            p.health = ((p.max_health() as f32 * 0.35) as i32).max(1);
            p.poison_left = 0.0;
            p.poison_dps = 0.0;
            p.poison_accum = 0.0;
            p.slow_mult = 1.0;
            p.slow_timer = 0.0;
            p.vel = Vec3::ZERO;
            p.phoenix = Some((p.pos, 0));
            self.events.push(Event::Phoenix { player: i });
            return true;
        }
        // Coop: a human goes down while a teammate stands.
        if self.coop && self.coop_down(i) { return true; }
        let gib = if let Some(fall) = blow.lava {
            Gib::Lava { fall }
        } else if blow.blast_radius > 0.0 {
            let severity = if overkill { ((-new + OVERKILL_HEALTH) as f32 / 50.0).clamp(0.4, 2.5) } else { 0.0 };
            Gib::Burst { severity: severity.max(blow.severity.clamp(0.4, 2.5)), blast: true }
        } else if blow.head {
            Gib::Head
        } else if overkill {
            Gib::Burst { severity: ((-new + OVERKILL_HEALTH) as f32 / 50.0).clamp(0.4, 2.5), blast: false }
        } else {
            Gib::Ragdoll
        };
        self.kill(i, from.filter(|&f| f != i), gib, blow.dir);
        true
    }

    pub(crate) fn kill(&mut self, victim: usize, attacker: Option<usize>, gib: Gib, dir: Vec3) {
        let v = &mut self.players[victim];
        if !v.alive { return; }
        v.alive = false;
        v.health = 0;
        v.vel = Vec3::ZERO;
        v.reloading = false;
        if let Some(killer) = attacker.filter(|&a| a != victim).and_then(|a| self.players.get_mut(a)) {
            killer.kills += 1;
        }
        self.events.push(Event::Kill { victim, attacker, gib, dir });
        if let Some(h) = self.hooks.0.clone() { h.kill(self, victim, attacker); }
        self.report_kill(victim);
    }

    /// Godot `kill_environmental`: lethal, past the phoenix (lava).
    pub(crate) fn kill_environmental(&mut self, i: usize) {
        let p = &self.players[i];
        if p.ghost || !p.alive || p.phoenix.is_some() { return; }
        let lethal = p.health.max(p.max_health());
        let p = &mut self.players[i];
        p.launching = None;
        let (frozen, iframes) = (p.frozen, p.iframes);
        p.frozen = false;
        // Dodge still blocks it; the arena retries every tick.
        if iframes > 0.0 { p.frozen = frozen; return; }
        let pos = p.pos;
        self.events.push(crate::Event::Lava { player: i, fall: true });
        self.apply_damage(i, lethal, Some(i), Blow { env: true, severity: -1.0, lava: Some(true), ..Blow::plain(pos) });
    }

    /// Godot `apply_environmental_damage` (lava contact), from nobody.
    /// Lava damage (the only environmental damage over time).
    pub(crate) fn environmental_damage(&mut self, i: usize, amount: i32) {
        let p = &self.players[i];
        if p.ghost || !p.alive || p.frozen || p.launching.is_some() || p.phoenix.is_some() { return; }
        let pos = p.pos;
        let lethal = amount >= p.health;
        self.events.push(crate::Event::Lava { player: i, fall: false });
        if lethal { self.events.push(crate::Event::Lava { player: i, fall: true }); }
        self.apply_damage(i, amount, None, Blow { lava: Some(false), ..Blow::plain(pos) });
    }

    /// Godot `_on_dealt_damage`: scavenger, refreshed, lifesteal.
    pub(crate) fn dealt_damage(&mut self, shooter: usize, dmg: i32) {
        if dmg <= 0 { return; }
        let p = &mut self.players[shooter];
        let w = &p.weapon;
        if w.reload_on_hit > 0 {
            p.ammo = (p.ammo + w.reload_on_hit as u32).min(p.mag_size());
            if p.ammo > 0 && p.reloading { p.reloading = false; p.rifle_cd = 0.0; }
        }
        if w.special_cooldown_refund_on_hit > 0.0 { p.special_cd = (p.special_cd - w.special_cooldown_refund_on_hit as f64).max(0.0); }
        if w.lifesteal > 0.0 {
            let heal = (dmg as f32 * w.lifesteal) as i32;
            p.health = (p.health + heal).min(p.max_health());
        }
    }

    pub(crate) fn knockback(&mut self, i: usize, impulse: Vec3) {
        let p = &mut self.players[i];
        if p.ghost || p.frozen { return; }
        p.vel += impulse;
    }

    /// Teams: in coop, humans against bots.
    pub fn allied(&self, a: usize, b: usize) -> bool {
        self.coop && a != b && self.players[a].bot == self.players[b].bot
    }

    pub(crate) fn gravity_mult(&self) -> f32 { (if self.modifier == Some(crate::Modifier::LowGravity) { 0.6 } else { 1.0 }) * self.rules.gravity }

    /// Base lava under the floor: a kill below -3, 15 damage per half
    /// second in the metre above (Godot `arena_generator` lava pool), and
    /// the lava leak.
    ///
    /// Island arenas instead (Godot `_apply_all_floor_lava_damage`): a kill
    /// below the lava floor at 0.04 m, the burn up to 1.15 m above it except
    /// over an island's safe disk.
    pub(crate) fn lava_tick(&mut self) {
        let island = self.arena.variant == crate::arena::Variant::Island;
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if !p.alive || p.ghost { continue; }
            let y = p.pos.y;
            if island {
                use crate::arena::{LAVA_FLOOR_BURN, LAVA_FLOOR_Y};
                if y < LAVA_FLOOR_Y { self.kill_environmental(i); continue; }
                if y <= LAVA_FLOOR_Y + LAVA_FLOOR_BURN && !self.arena.on_island(glam::Vec2::new(p.pos.x, p.pos.z)) {
                    self.players[i].lava_accum += DT64;
                    while self.players[i].lava_accum >= 0.5 {
                        self.players[i].lava_accum -= 0.5;
                        self.environmental_damage(i, 15);
                    }
                } else if !self.lava.active {
                    self.players[i].lava_accum = 0.0;
                }
                continue;
            }
            if y < crate::arena::LAVA_KILL_Y { self.kill_environmental(i); continue; }
            if y <= crate::arena::LAVA_KILL_Y + crate::arena::LAVA_BURN_BAND {
                self.players[i].lava_accum += DT64;
                while self.players[i].lava_accum >= 0.5 {
                    self.players[i].lava_accum -= 0.5;
                    self.environmental_damage(i, 15);
                }
            } else if !self.lava.active {
                self.players[i].lava_accum = 0.0;
            }
        }
        self.lava_leak_tick();
    }
}

impl Blow {
    /// Lava kills skip the phoenix (Godot zeroes the charges for the call).
    fn env_lava(&self) -> bool { self.env && self.severity < 0.0 }
}

#[cfg(test)]
mod step_tests {
    use super::*;
    use crate::{shape::{BoxGrid, YawBox}, HALF_HEIGHT};

    #[test]
    fn grounded_player_steps_over_stones_without_a_jump_or_floor_snap_offset() {
        let mut world = World::new(1, &[false, false]);
        world.arena.grid = BoxGrid::new(vec![
            YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(20.0, 1.0, 20.0), 0.0),
            YawBox::new(Vec3::new(1.0, 0.15, 0.0), Vec3::new(1.0, 0.3, 2.0), 0.0),
        ], 1.0);
        let player = &mut world.players[0];
        player.pos = Vec3::new(0.0, HALF_HEIGHT, 0.0);
        player.vel = Vec3::new(12.0, 0.0, 0.0);
        player.on_floor = true;
        let jumps = player.jumps_left;
        world.slide(0);
        let player = &world.players[0];
        assert!(player.pos.x > 0.19 && player.on_floor, "{}", player.pos);
        assert!((player.pos.y - (HALF_HEIGHT + 0.3)).abs() < 1e-4, "{}", player.pos);
        assert_eq!(player.vel, Vec3::new(12.0, 0.0, 0.0));
        assert_eq!(player.jumps_left, jumps);
    }
}
