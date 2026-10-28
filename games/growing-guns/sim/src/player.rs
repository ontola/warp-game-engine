//! One player's tick (Godot `Player._physics_process`): status effects,
//! cooldowns, gravity, jumps and dash, movement, combat input, falling off
//! the map; and the one place health goes down (`_apply_damage`).
use crate::{flow::Phase, look_dir, move_toward, Event, Gib, Input, World};
use crate::{AIR_ACCEL, BOT_MOVE_SPEED, DASH_RECHARGE_TIME, DASH_SPEED, DASH_TIME, DOUBLE_JUMP_VELOCITY, DT, DT64, FELL_OFF_Y, FRICTION, GRAVITY};
use crate::{GROUND_ACCEL, JUMP_VELOCITY, MAX_DASH_CHARGES, MOVER, OVERKILL_HEALTH, PITCH_LIMIT, WALK_SPEED, WALL_JUMP_COOLDOWN, WALL_JUMP_H, WALL_JUMP_V};
use glam::{Vec2, Vec3};

/// Poison ticks once this much time has gathered.
const POISON_INTERVAL: f64 = 0.25;
const FLOOR_SNAP: f32 = 0.1;
pub const PHOENIX_TICKS: u32 = 120;
pub const PHOENIX_RISE: f32 = 10.0;
/// Phoenix and coop revive drop: from this high, at this speed.
pub const SKY_DROP_HEIGHT: f32 = 60.0;
pub const SKY_DROP_SPEED: f32 = 80.0;

/// What a blow carries besides its damage, for the death it may cause.
#[derive(Clone, Copy, Debug)]
pub struct Blow { pub pos: Vec3, pub dir: Vec3, pub head: bool, pub blast_radius: f32, pub severity: f32, pub env: bool }

impl Blow {
    pub fn plain(pos: Vec3) -> Self { Blow { pos, dir: Vec3::Y, head: false, blast_radius: 0.0, severity: 0.0, env: false } }
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
            // Ghosts can't act (Godot: their health is 0), but they can plant mines.
            self.ghost_tick(i, fire_edge);
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

    /// Godot `_tick_cooldowns`: weapon and special, wall jump, dash recharge, reload.
    pub(crate) fn cooldowns(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.rifle_cd = (p.rifle_cd - DT64).max(0.0);
        p.special_cd = (p.special_cd - DT64).max(0.0);
        if p.reloading && p.rifle_cd <= 0.0 { p.ammo = p.mag_size(); p.reloading = false; }
        p.wall_jump_cd = (p.wall_jump_cd - DT64).max(0.0);
        if p.dash_charges < MAX_DASH_CHARGES {
            p.dash_recharge += DT64;
            if p.dash_recharge >= DASH_RECHARGE_TIME { p.dash_charges += 1; p.dash_recharge = 0.0; }
        } else {
            p.dash_recharge = 0.0;
        }
    }

    /// Godot `_decay_view_recoil`: bloom and the recoil kick wear off.
    fn decay_recoil(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.bloom *= 1.0 - (DT * 4.0).clamp(0.0, 1.0);
        p.recoil_pitch *= 1.0 - DT * 9.0;
    }

    /// Gravity, jumps, dash and horizontal movement, then the slide (humans;
    /// bots move in `bot.rs`).
    fn move_player(&mut self, i: usize, input: Input) {
        let gravity = GRAVITY * self.gravity_mult();
        let mut ev = Vec::new();
        let mut dash_ended = false;
        let mut dash_started = None;
        {
            let p = &mut self.players[i];
            let bot = p.bot;
            p.yaw += input.look[0];
            p.pitch = (p.pitch + input.look[1]).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            let (sy, cy) = p.yaw.sin_cos();
            let fwd = Vec3::new(-sy, 0.0, -cy);
            let right = Vec3::new(cy, 0.0, -sy);
            let mv = Vec2::from(input.mv);
            let mv = if mv.length() > 1.0 { mv.normalize() } else { mv };
            let wish = right * mv.x + fwd * mv.y;
            let extra = p.weapon.extra_jumps;

            if !p.on_floor { p.vel.y -= gravity * DT; } else { p.jumps_left = 2 + extra; }
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
            if input.dash && p.dash_charges > 0 { dash_started = Some(if wish.length_squared() < 1e-4 { fwd } else { wish }); }

        }
        if let Some(d) = dash_started { self.start_dash(i, d); }
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
            let mut target = wish * walk;
            let horiz = Vec3::new(p.vel.x, 0.0, p.vel.z);
            let speeding = horiz.length() > walk + 0.1;
            let accel = if p.dash_timer > 0.0 {
                p.dash_timer -= DT64;
                p.vel.y = p.vel.y.max(0.0);
                let k = (p.dash_timer / (DASH_TIME * 0.5)).clamp(0.0, 1.0) as f32;
                target = target.lerp(p.dash_dir * DASH_SPEED * p.slow_mult, k);
                if p.dash_timer <= 0.0 { dash_ended = true; }
                2000.0
            } else if p.on_floor {
                if !bot && speeding && wish.dot(horiz.normalize_or_zero()) <= 0.5 { FRICTION } else { GROUND_ACCEL }
            } else {
                AIR_ACCEL
            };
            p.vel.x = move_toward(p.vel.x, target.x, accel * DT);
            p.vel.z = move_toward(p.vel.z, target.z, accel * DT);
            if !bot && p.on_floor && wish == Vec3::ZERO && p.dash_timer <= 0.0 && !speeding {
                let f = FRICTION * DT * walk * 0.1;
                p.vel.x = move_toward(p.vel.x, 0.0, f);
                p.vel.z = move_toward(p.vel.z, 0.0, f);
            }
        }
        self.slide(i);
        self.events.extend(ev);
        if dash_ended { self.dash_ended(i); }
    }

    /// Godot `_start_dash`: a charge, the dodge immunity, the backblast bomb.
    pub(crate) fn start_dash(&mut self, i: usize, dir: Vec3) {
        let p = &mut self.players[i];
        if p.dash_charges == 0 { return; }
        let dir = if dir.length_squared() < 1e-4 { look_dir(p.yaw, 0.0) } else { dir };
        p.dash_dir = dir.normalize();
        p.dash_timer = DASH_TIME;
        p.dash_charges -= 1;
        if p.weapon.dash_iframes { p.iframes = DASH_TIME; }
        let bomb = p.weapon.dash_spawn_bomb.then(|| p.pos + Vec3::Y * 0.08);
        self.events.push(Event::Dash { player: i });
        if let Some(at) = bomb { self.spawn_dash_bomb(i, at); }
    }

    /// `move_and_slide` with Godot's floor snap: a grounded body is pressed
    /// down by up to 0.1 m so it stays on the floor and walks down steps.
    pub(crate) fn slide(&mut self, i: usize) {
        let p = &mut self.players[i];
        let snap = p.on_floor && p.vel.y <= 0.0;
        let (was_on_floor, fall) = (p.on_floor, -p.vel.y);
        if snap { p.vel.y = -FLOOR_SNAP / DT; }
        // Sub-steps short enough that no thin block is skipped.
        let contact = MOVER.slide(&self.arena.grid, &mut p.pos, &mut p.vel, DT, 0.25);
        if snap && !contact.on_floor {
            // Nothing within the snap: we walked off a ledge; undo the press.
            p.pos.y += FLOOR_SNAP;
            p.vel.y = 0.0;
        }
        p.on_floor = contact.on_floor;
        p.wall_normal = if contact.on_floor { None } else { contact.wall_normal };
        if contact.on_floor && !was_on_floor { self.events.push(crate::Event::Landed { player: i, speed: fall.max(0.0) }); }
    }

    /// Godot `_on_dash_ended`: the ninja slash.
    pub(crate) fn dash_ended(&mut self, i: usize) {
        let p = &self.players[i];
        if !p.weapon.dash_end_melee || !p.alive || p.ghost { return; }
        let dir = if p.dash_dir.length_squared() > 0.001 { p.dash_dir } else { look_dir(p.yaw, 0.0) };
        let origin = p.pos + Vec3::Y * 0.9;
        self.melee(i, origin, dir.normalize(), crate::specials::NINJA_DAMAGE_MULT, crate::specials::NINJA_RANGE_MULT);
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
        let gib = if blow.blast_radius > 0.0 {
            let severity = if overkill { ((-new + OVERKILL_HEALTH) as f32 / 50.0).clamp(0.4, 2.5) } else { 0.0 };
            Gib::Burst { severity: severity.max(blow.severity.clamp(0.4, 2.5)), blast: true }
        } else if overkill {
            Gib::Burst { severity: ((-new + OVERKILL_HEALTH) as f32 / 50.0).clamp(0.4, 2.5), blast: false }
        } else if blow.head {
            Gib::Head
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
        self.apply_damage(i, lethal, Some(i), Blow { env: true, severity: -1.0, ..Blow::plain(pos) });
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
        self.apply_damage(i, amount, None, Blow::plain(pos));
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
    pub(crate) fn lava_tick(&mut self) {
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if !p.alive || p.ghost { continue; }
            let y = p.pos.y;
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
