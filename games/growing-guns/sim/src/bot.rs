//! Computer players, a port of Godot's bot AI (`player.gd` `_bot_physics`
//! and friends): a sticky nearest target, an aim point that turns at 3.2
//! rad/s with a wandering error in metres, a reaction time before the first
//! shot, full-auto fire with its own spread model, strafing at a held range,
//! gap jumps, ledge climbs, random hops and dashes, fleeing strikes, going
//! for pickups and using the RMB special by distance.
//!
//! On island arenas (Godot `_bot_on_lava_map`) every move, hop, dash, gap
//! jump, climb and pickup is checked against the islands' safe disks first,
//! except in a 0.6 s lapse a bot rolls into 7% of seconds. Two additions
//! (Godot's bots, as written, never gap-jump while careful, since the safe
//! direction is chosen before the gap check, and they turn back over the
//! gap in mid-air, since that check also runs in the air): a careful bot
//! keeps heading for its target when a gap lies ahead with a safe landing
//! across it, and flies a gap jump to the landing it checked: steering for
//! it in the air, braking over it and double jumping only if it would fall
//! short.
//!
//! Godot uses one global random stream; each bot here has its own, seeded
//! from the match seed, drawn in Godot's order.
use crate::rng::Rng;
use crate::{look_dir, move_toward, shape, Event, Phase, Special, World, DT, DT64};
use crate::{AIR_ACCEL, BOT_MOVE_SPEED, DASH_SPEED, DASH_TIME, DOUBLE_JUMP_VELOCITY, FELL_OFF_Y, GRAVITY, GROUND_ACCEL, JUMP_VELOCITY};
use glam::{Quat, Vec3};

const FOLLOW_DIST: f32 = 7.0;
const ROT_SPEED: f32 = 6.0;
const SPREAD: f32 = 0.09;
const MISS_CHANCE: f32 = 0.45;
const JUMP_CHANCE: f32 = 0.008;
const DASH_CHANCE: f32 = 0.018;
const EDGE_PROBE: f32 = 1.8;
const LAVA_EDGE_PROBE: f32 = 3.0;
const LAVA_MISTAKE_CHANCE: f32 = 0.07;
const LAVA_JUMP_CHANCE: f32 = 0.0015;
const STRIKE_FLEE: f32 = 40.0;
const STRIKE_PANIC: f32 = 22.0;
const GAP_LANDINGS: [f32; 5] = [4.5, 6.5, 8.5, 10.5, 12.0];
const REACTION: (f32, f32) = (0.2, 0.55);
const AIM_TURN: f32 = 3.2;
const PICKUP_RANGE: f32 = 22.0;
const PICKUP_NOTICE: f32 = 0.55;
const CLIMB: (f32, f32) = (1.0, 2.6);
const EYE: f32 = 0.7;
const AIM_AT: f32 = 0.4;

#[derive(Clone, Debug, Default)]
pub struct Brain {
    rng: Option<Rng>,
    target: Option<usize>,
    aim_point: Option<Vec3>,
    los_time: f32,
    reaction: f32,
    shoot_cd: f64,
    jump_cd: f32,
    dash_cd: f32,
    strafe_timer: f32,
    strafe: f32,
    approach: f32,
    stuck: f32,
    aim_retarget: f32,
    wander: Vec3,
    wander_target: Vec3,
    special_think: f32,
    climb_probe: f32,
    climb_until: f32,
    climb_y: f32,
    pickup: Option<Vec3>,
    pickup_scan: f32,
    pickup_give_up: f32,
    /// Island arenas: a lapse window in which the lava checks are skipped,
    /// and the countdown to the next roll for one.
    lava_risk: f32,
    lava_roll: f32,
    /// Island arenas: the landing (ground point) of a gap jump under way,
    /// until the bot lands (or until the tick in `leap_until`).
    leap: Option<Vec3>,
    leap_until: u64,
}

impl Brain {
    /// In a lapse of the lava checks (tests).
    #[cfg(test)]
    pub(crate) fn lapse(&self) -> bool { self.lava_risk > 0.0 }

    pub fn new(seed: u64) -> Self { Brain { rng: Some(Rng::new(seed)), reaction: 0.35, approach: 1.0, ..Default::default() } }
}

fn pick(rng: &mut Rng, of: &[f32]) -> f32 { of[rng.below(of.len() as u32) as usize] }

/// Island arenas: whether a gap jump at walking speed, with the double jump
/// and (past 7 m, as Godot's gap jump) a dash, lands `d` m on and `rise` m
/// up. The jumps top out 2.55 m up; the dash adds about 3 m.
fn leap_reaches(d: f32, rise: f32) -> bool {
    if rise > 2.0 { return false; }
    let t = (JUMP_VELOCITY + DOUBLE_JUMP_VELOCITY) / GRAVITY + (2.0 * (2.55 - rise) / GRAVITY).sqrt();
    d <= 0.85 * BOT_MOVE_SPEED * t + if d > 7.0 { 3.0 } else { 0.0 }
}

impl World {
    /// Godot `_bot_physics`, after the status effects and early outs.
    pub(crate) fn bot_physics(&mut self, i: usize) {
        if self.brains.len() < self.players.len() {
            let n = self.brains.len();
            self.brains.extend((n..self.players.len()).map(|k| Brain::new(self.tick ^ ((k as u64 + 1) * 0x9E37))));
        }
        let mut b = std::mem::take(&mut self.brains[i]);
        if b.rng.is_none() { b = Brain::new(i as u64 + 1); }
        self.bot_tick(i, &mut b);
        self.brains[i] = b;
    }

    fn bot_tick(&mut self, i: usize, b: &mut Brain) {
        let gravity = GRAVITY * self.gravity_mult();
        if self.bots_paused {
            let p = &mut self.players[i];
            if !p.on_floor { p.vel.y -= gravity * DT; }
            p.vel.x = move_toward(p.vel.x, 0.0, BOT_MOVE_SPEED * 3.0 * DT);
            p.vel.z = move_toward(p.vel.z, 0.0, BOT_MOVE_SPEED * 3.0 * DT);
            self.slide(i);
            return;
        }
        let mut rng = b.rng.take().unwrap_or_else(|| Rng::new(i as u64));
        b.shoot_cd = (b.shoot_cd - DT64).max(0.0);
        b.jump_cd = (b.jump_cd - DT).max(0.0);
        b.dash_cd = (b.dash_cd - DT).max(0.0);
        b.special_think = (b.special_think - DT).max(0.0);
        b.climb_probe = (b.climb_probe - DT).max(0.0);
        b.climb_until = (b.climb_until - DT).max(0.0);
        b.pickup_scan -= DT;
        b.pickup_give_up -= DT;
        // Island arenas: roll once a second for a lapse (Godot rolls on every
        // map; here only where it matters, so other arenas keep their stream).
        let lava = self.arena.variant == crate::arena::Variant::Island;
        if lava {
            b.lava_risk = (b.lava_risk - DT).max(0.0);
            b.lava_roll -= DT;
            if b.lava_roll <= 0.0 {
                b.lava_roll = 1.0;
                if rng.f() < LAVA_MISTAKE_CHANCE { b.lava_risk = 0.6; }
            }
        }
        let careful = lava && b.lava_risk <= 0.0;
        if self.players[i].on_floor || self.tick > b.leap_until { b.leap = None; }
        let probe = if lava { LAVA_EDGE_PROBE } else { EDGE_PROBE };
        // Aim wobble: a random walk in metres.
        b.aim_retarget -= DT;
        if b.aim_retarget <= 0.0 {
            b.aim_retarget = rng.range(0.25, 0.7);
            let v = Vec3::new(rng.range(-1.0, 1.0), rng.range(-0.6, 0.6), rng.range(-1.0, 1.0));
            b.wander_target = v * rng.range(0.1, 0.9);
        }
        b.wander = b.wander.lerp(b.wander_target, 1.0 - (-5.0 * DT).exp());
        self.cooldowns(i);
        {
            let p = &mut self.players[i];
            if !p.on_floor { p.vel.y -= gravity * DT; } else { p.jumps_left = 2 + p.weapon.extra_jumps; }
        }
        // Sticky nearest target.
        // Not allies, ghosts, downed humans or enemies still rising (coop).
        let valid = |w: &World, t: usize| t != i && t < w.players.len() && !w.players[t].ghost && !w.players[t].downed
            && !w.allied(i, t) && !w.waves.emerging(t);
        if b.target.is_none_or(|t| !valid(self, t)) {
            let me = self.players[i].pos;
            b.target = (0..self.players.len()).filter(|&t| valid(self, t))
                .min_by(|&a, &c| me.distance_squared(self.players[a].pos).total_cmp(&me.distance_squared(self.players[c].pos)));
        }
        let Some(t) = b.target else {
            let p = &mut self.players[i];
            p.vel.x = move_toward(p.vel.x, 0.0, BOT_MOVE_SPEED * 3.0 * DT);
            p.vel.z = move_toward(p.vel.z, 0.0, BOT_MOVE_SPEED * 3.0 * DT);
            self.slide(i);
            b.rng = Some(rng);
            return;
        };
        let me = self.players[i].pos;
        let tpos = self.players[t].pos;
        let to = tpos - me;
        let flat = Vec3::new(to.x, 0.0, to.z);
        let dist = flat.length();
        b.strafe_timer -= DT;
        if b.strafe_timer <= 0.0 {
            b.strafe_timer = rng.range(0.5, 1.6);
            b.strafe = pick(&mut rng, &[-1.0, -0.7, 0.0, 0.7, 1.0]);
            b.approach = pick(&mut rng, &[-0.6, 0.0, 0.0, 0.6, 1.0]);
        }
        let fwd = if dist > 0.01 { flat / dist } else { look_dir(self.players[i].yaw, 0.0) };
        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
        let chase = if dist > FOLLOW_DIST * 1.8 { 1.0 } else if dist < FOLLOW_DIST * 0.45 { -0.7 } else { b.approach };
        let mut mv = fwd * chase + right * b.strafe;
        if mv.length() > 1.0 { mv = mv.normalize(); }
        // Pickups (not in coop).
        if b.pickup.is_some() && (b.pickup_give_up <= 0.0 || !self.pickups.iter().any(|p| Some(p.pos) == b.pickup)) { b.pickup = None; }
        if b.pickup.is_none() && b.pickup_scan <= 0.0 {
            b.pickup_scan = rng.range(1.4, 2.8);
            if !self.coop && rng.f() < PICKUP_NOTICE {
                b.pickup = self.pickups.iter().filter(|p| p.grounded.is_some() && (-6.0..=3.0).contains(&(p.pos.y - me.y)) && p.pos.distance(me) < PICKUP_RANGE && self.arena.spawn_safe(p.pos))
                    .min_by(|a, c| a.pos.distance_squared(me).total_cmp(&c.pos.distance_squared(me))).map(|p| p.pos);
                b.pickup_give_up = rng.range(4.0, 7.0);
            }
        }
        if let Some(pk) = b.pickup {
            let d = Vec3::new(pk.x - me.x, 0.0, pk.z - me.z);
            if d.length() > 0.3 { mv = (d.normalize() * 0.85 + mv * 0.45).normalize_or_zero(); }
        }
        // Run from strikes, our own included.
        let strike = self.strikes.iter().map(|s| (glam::Vec2::new(me.x - s.target.x, me.z - s.target.z), s.kind))
            .min_by(|a, c| a.0.length().total_cmp(&c.0.length()));
        let strike_dist = strike.map_or(f32::INFINITY, |s| s.0.length());
        let mut flee = Vec3::ZERO;
        if strike_dist < STRIKE_FLEE {
            let away = strike.unwrap().0;
            flee = if away.length() > 1e-3 { Vec3::new(away.x, 0.0, away.y).normalize() } else { Vec3::new(rng.range(-1.0, 1.0), 0.0, rng.range(-1.0, 1.0)).normalize_or_zero() };
            if careful { flee = self.lava_safe_dir(i, me, flee); }
            if flee != Vec3::ZERO { mv = flee; b.approach = 0.0; b.strafe = 0.0; }
        } else if let Some(l) = b.leap.filter(|_| lava && !self.players[i].on_floor) {
            // Fly to the landing; brake over it.
            let to = Vec3::new(l.x - me.x, 0.0, l.z - me.z);
            mv = if to.length() < 0.4 { Vec3::ZERO } else { to.normalize() * (to.length() / 1.2).min(1.0) };
        } else if careful {
            // A gap towards the target with a safe landing across: jump it
            // (below) rather than turn away from the edge; past 7 m only
            // with a dash ready.
            let p = &self.players[i];
            let leap = p.on_floor && p.dash_timer <= 0.0 && mv.length_squared() > 0.01 && !self.floor_ahead(me, mv, probe, true)
                && { let d = self.gap_landing(me, mv, true); d > 0.0 && (d <= 7.0 || (p.dash_charges > 0 && b.dash_cd <= 0.0)) };
            if !leap { mv = self.lava_safe_dir(i, me, mv); }
        }
        // Edges and gaps.
        let on_floor = self.players[i].on_floor;
        let moving = mv.length_squared() > 0.01;
        let mut gap_jump = false;
        if on_floor && moving && !self.floor_ahead(me, mv, probe, careful) {
            let found = self.gap_landing_at(me, mv, careful);
            let landing = found.map_or(0.0, |f| f.0);
            if let Some((d, y)) = found {
                gap_jump = true;
                if careful {
                    let at = me + mv.normalize() * d;
                    (b.leap, b.leap_until) = (Some(Vec3::new(at.x, y, at.z)), self.tick + 90);
                }
                self.bot_jump(i, JUMP_VELOCITY, true);
                b.jump_cd = rng.range(0.8, 1.8);
                let p = &self.players[i];
                if landing > 7.0 && p.dash_timer <= 0.0 && p.dash_charges > 0 && b.dash_cd <= 0.0 {
                    self.start_dash(i, mv);
                    b.dash_cd = rng.range(2.0, 4.0);
                }
            } else if b.jump_cd <= 0.0 {
                self.bot_jump(i, JUMP_VELOCITY, true);
                b.jump_cd = rng.range(0.8, 1.6);
            } else {
                mv = Vec3::ZERO;
                b.strafe_timer = 0.0;
            }
        } else if let Some(l) = b.leap.filter(|_| !on_floor) {
            // Double jump only when the fall from here lands short.
            let p = &self.players[i];
            if p.vel.y < 1.0 && p.jumps_left > 0 {
                let drop = me.y - (l.y + crate::HALF_HEIGHT);
                let disc = p.vel.y * p.vel.y + 2.0 * gravity * drop;
                let fall = if disc > 0.0 { (p.vel.y + disc.sqrt()) / gravity } else { 0.0 };
                let dist = glam::Vec2::new(l.x - me.x, l.z - me.z).length();
                if BOT_MOVE_SPEED * fall < dist - 0.3 { self.bot_double_jump(i); }
            }
        } else if !on_floor && self.players[i].vel.y < 1.0 && self.players[i].jumps_left > 0 && moving
            && !self.floor_ahead(me, mv, probe, careful) && self.gap_landing(me, mv, careful) > 0.0 {
            self.bot_double_jump(i);
        }
        // Ledge climbs.
        if !gap_jump && on_floor && b.climb_probe <= 0.0 && moving {
            if to.y > 1.6 || rng.f() < 0.35 {
                let h = self.ledge_height(me, mv);
                if h > 0.0 {
                    self.bot_jump(i, JUMP_VELOCITY, true);
                    b.climb_until = 1.4;
                    b.climb_y = me.y + h;
                    b.jump_cd = rng.range(0.8, 1.6);
                }
            }
            b.climb_probe = rng.range(1.2, 2.8);
        }
        {
            let p = &self.players[i];
            if b.climb_until > 0.0 && !p.on_floor && p.vel.y < 1.0 && p.jumps_left > 0 && p.pos.y < b.climb_y - 0.2 { self.bot_double_jump(i); }
        }
        // Random hop, rarer on islands and never towards the lava.
        let hop = if lava { LAVA_JUMP_CHANCE } else { JUMP_CHANCE };
        if !gap_jump && self.players[i].on_floor && b.jump_cd <= 0.0 && rng.f() < hop && (!lava || self.move_lava_safe(me, mv, probe + 1.5)) {
            self.bot_jump(i, JUMP_VELOCITY, false);
            b.jump_cd = rng.range(2.0, 4.5);
        }
        // Dash: panic near strikes, else now and then.
        let panic = strike_dist < STRIKE_PANIC;
        let dash_ok = moving && (!careful || self.move_lava_safe(me, mv, probe + 1.2));
        {
            let p = &self.players[i];
            if p.dash_timer <= 0.0 && p.dash_charges > 0 && b.dash_cd <= 0.0 && (panic || (rng.f() < DASH_CHANCE && dash_ok)) {
                let wish = if panic && flee != Vec3::ZERO { flee } else if moving { mv } else { fwd };
                self.start_dash(i, wish);
                b.dash_cd = rng.range(2.5, 5.5);
            }
        }
        // Facing and velocity.
        let mut dash_ended = false;
        {
            let p = &mut self.players[i];
            let face = if p.dash_timer > 0.0 { p.dash_dir } else if dist > 0.05 { flat } else { mv };
            if face.length_squared() > 1e-6 {
                let want = (-face.x).atan2(-face.z);
                let d = (want - p.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                p.yaw += d * (DT * ROT_SPEED).min(1.0);
            }
            let mut target = mv * BOT_MOVE_SPEED * p.weapon.move_speed_mult * p.slow_mult;
            let accel = if p.dash_timer > 0.0 {
                p.dash_timer -= DT64;
                p.vel.y = p.vel.y.max(0.0);
                let k = (p.dash_timer / (DASH_TIME * 0.5)).clamp(0.0, 1.0) as f32;
                target = target.lerp(p.dash_dir * DASH_SPEED * p.slow_mult, k);
                if p.dash_timer <= 0.0 { dash_ended = true; }
                2000.0
            } else if p.on_floor { GROUND_ACCEL } else { AIR_ACCEL };
            p.vel.x = move_toward(p.vel.x, target.x, accel * DT);
            p.vel.z = move_toward(p.vel.z, target.z, accel * DT);
            let tv = Vec3::new(target.x, 0.0, target.z);
            let speed = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
            if tv.length_squared() > 0.25 && speed < 1.1 && p.on_floor && p.dash_timer <= 0.0 {
                b.stuck += DT;
            } else {
                b.stuck = 0.0;
            }
        }
        if dash_ended { self.dash_ended(i); }
        self.slide(i);
        if b.stuck > 0.5 && b.jump_cd <= 0.0 && self.players[i].on_floor {
            let p = &mut self.players[i];
            p.vel.y = JUMP_VELOCITY;
            p.jumps_left = p.jumps_left.max(1);
            self.events.push(Event::Jump { player: i });
            b.jump_cd = rng.range(0.8, 1.6);
            b.stuck = 0.0;
        }
        if self.players[i].alive && self.players[i].pos.y < FELL_OFF_Y {
            let hp = self.players[i].health.max(100);
            self.apply_damage(i, hp, Some(i), crate::player::Blow { env: true, ..crate::player::Blow::plain(self.players[i].pos) });
        }
        if !self.players[i].alive { b.rng = Some(rng); return; }
        // The aim point slews toward the target's chest at 3.2 rad/s.
        let eye = self.players[i].pos + Vec3::Y * EYE;
        let want = self.players[t].pos + Vec3::Y * AIM_AT - eye;
        let range = want.length().max(0.5);
        let mut cur = b.aim_point.map_or(Vec3::ZERO, |a| a - eye);
        if cur.length_squared() < 0.01 { cur = look_dir(self.players[i].yaw, 0.0) * range; }
        let ang = cur.angle_between(want);
        let max_turn = AIM_TURN * DT;
        if ang > max_turn {
            let axis = cur.cross(want).try_normalize().unwrap_or(Vec3::Y);
            cur = Quat::from_axis_angle(axis, max_turn) * cur;
        } else {
            cur = want;
        }
        let aim = eye + cur.normalize_or(Vec3::NEG_Z) * range;
        b.aim_point = Some(aim);
        // The gun (and the specials) point along the aim.
        let d = (aim - eye).normalize_or(Vec3::NEG_Z);
        self.players[i].pitch = d.y.clamp(-1.0, 1.0).asin().clamp(-crate::PITCH_LIMIT, crate::PITCH_LIMIT);
        // Line of sight and reaction.
        let target_at = self.players[t].pos + Vec3::Y * AIM_AT;
        let seg = target_at - eye;
        let los = seg.length() < 1e-3 || shape::raycast(&self.arena.grid, eye, seg.normalize(), seg.length()).is_none();
        if los { b.los_time += DT; } else if b.los_time > 0.0 { b.reaction = rng.range(REACTION.0, REACTION.1); b.los_time = 0.0; }
        let p = &self.players[i];
        if p.alive && !p.ghost && los && b.los_time >= b.reaction && self.phase == Phase::Playing {
            if b.shoot_cd <= 0.0 && !p.reloading && self.bot_shoot(i, t, b, &mut rng) { b.shoot_cd = self.players[i].weapon.fire_interval(); }
            if self.players[i].special_cd <= 0.0 && b.special_think <= 0.0 {
                b.special_think = rng.range(1.2, 3.2);
                self.bot_special(i, dist, aim, &mut rng);
            }
        }
        b.rng = Some(rng);
    }

    /// Godot `_bot_shoot`: from the eye at the slewed aim point, led by the
    /// target's speed, a metre-scale wobble, and a square spread that grows
    /// with distance and target speed.
    fn bot_shoot(&mut self, i: usize, t: usize, b: &Brain, rng: &mut Rng) -> bool {
        if self.players[i].reloading { return false; }
        if self.players[i].ammo == 0 { self.start_reload(i); return false; }
        let p = &mut self.players[i];
        p.ammo -= 1;
        p.since_shot = 0.0;
        let last = p.ammo == 0;
        let from = p.pos + Vec3::Y * EYE;
        let to = b.aim_point.unwrap_or(self.players[t].pos + Vec3::Y * AIM_AT);
        let dist = to.distance(from);
        let tv = self.players[t].vel;
        let w = &self.players[i].weapon;
        let t_hit = dist / w.bullet_speed().max(1.0);
        let lead = rng.range(0.55, 1.15);
        let mut pred = to + tv * t_hit * lead;
        let speed_err = (tv.length() / 7.0).clamp(0.2, 1.4);
        let settle = (1.6 - b.los_time * 0.45).clamp(0.6, 1.6);
        pred += b.wander * (dist / 16.0).clamp(0.5, 2.2) * settle * speed_err;
        let dir = (pred - from).normalize_or(Vec3::NEG_Z);
        let base = 0.018 + 0.03 * speed_err;
        let mut spread = base + (SPREAD * 0.7 * (dist / 40.0).clamp(0.0, 2.0)) * (0.4 + 0.6 * speed_err);
        if rng.f() < MISS_CHANCE * speed_err { spread *= rng.range(2.5, 5.0); }
        let right = Vec3::Y.cross(dir).try_normalize().unwrap_or(Vec3::X);
        for _ in 0..w.shots_per_trigger() {
            let (yaw, pitch) = (rng.range(-spread, spread), rng.range(-spread, spread));
            let shot = (Quat::from_axis_angle(right, pitch) * (Quat::from_axis_angle(Vec3::Y, yaw) * dir)).normalize();
            self.spawn_bullet(i, from, shot, last);
        }
        if self.players[i].ammo == 0 { self.start_reload(i); }
        true
    }

    /// Godot `_bot_try_special`, by flat distance to the target.
    fn bot_special(&mut self, i: usize, dist: f32, aim: Vec3, rng: &mut Rng) {
        let p = &self.players[i];
        let sp = if p.air_strike_charges > 0 { Special::AirStrike } else { p.weapon.special };
        let skip = match sp {
            Special::Zoom => true,
            Special::Sword => dist > 4.5,
            // Not on islands: the blink is blind to what it lands on.
            Special::Teleport => dist < 13.0 || self.arena.variant == crate::arena::Variant::Island || rng.f() < 0.4,
            Special::AirStrike | Special::IonCannon => dist < 9.0 || rng.f() < 0.35,
            Special::Grenade | Special::ClusterGrenade => !(5.0..=30.0).contains(&dist) || rng.f() < 0.35,
        };
        if skip { return; }
        let eye = p.eye();
        let mut at = aim;
        if matches!(sp, Special::Grenade | Special::ClusterGrenade) { at += Vec3::Y * p.pos.distance(aim) * 0.22; }
        let d = (at - eye).normalize_or(Vec3::NEG_Z);
        let p = &mut self.players[i];
        p.yaw = (-d.x).atan2(-d.z);
        p.pitch = d.y.clamp(-1.0, 1.0).asin();
        self.use_special(i);
    }

    fn bot_jump(&mut self, i: usize, v: f32, reset: bool) {
        let p = &mut self.players[i];
        p.vel.y = v;
        if reset { p.jumps_left = 1 + p.weapon.extra_jumps; }
        self.events.push(Event::Jump { player: i });
    }

    fn bot_double_jump(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.vel.y = DOUBLE_JUMP_VELOCITY;
        p.jumps_left -= 1;
        self.events.push(Event::Jump { player: i });
    }

    /// Godot `_bot_has_floor_ahead`; `careful` on islands: the spot ahead
    /// and the ground under it must be safe too.
    fn floor_ahead(&self, me: Vec3, dir: Vec3, d: f32, careful: bool) -> bool {
        if dir.length_squared() <= 0.001 { return true; }
        let ahead = me + dir.normalize() * d;
        let probe = ahead + Vec3::Y * 0.8;
        let Some((t, _, _)) = shape::raycast(&self.arena.grid, probe, Vec3::NEG_Y, 3.0) else { return false };
        if careful && (!self.arena.spawn_safe(ahead) || !self.arena.spawn_safe(probe - Vec3::Y * (t - 0.9))) { return false; }
        true
    }

    /// Godot `_bot_gap_landing_distance`: a floor to jump to, or 0;
    /// `careful` on islands skips landings off a safe disk.
    fn gap_landing(&self, me: Vec3, dir: Vec3, careful: bool) -> f32 { self.gap_landing_at(me, dir, careful).map_or(0.0, |f| f.0) }

    /// The same with the landing's height: (distance, ground y).
    fn gap_landing_at(&self, me: Vec3, dir: Vec3, careful: bool) -> Option<(f32, f32)> {
        let dir = dir.normalize_or_zero();
        for d in GAP_LANDINGS {
            let probe = me + dir * d + Vec3::Y * 3.0;
            if let Some((t, _, _)) = shape::raycast(&self.arena.grid, probe, Vec3::NEG_Y, 9.0) {
                let y = probe.y - t;
                if careful && !self.arena.spawn_safe(Vec3::new(probe.x, y + 0.9, probe.z)) { continue; }
                // Islands: only what a jump, a double jump and a dash reach.
                if careful && !leap_reaches(d, y - (me.y - crate::HALF_HEIGHT)) { continue; }
                if y >= me.y - 4.5 && y <= me.y + 3.5 { return Some((d, y)); }
            }
        }
        None
    }

    /// Godot `_bot_has_floor_at`: ground within 2.7 m below.
    fn has_floor_at(&self, p: Vec3) -> bool { shape::raycast(&self.arena.grid, p + Vec3::Y * 0.8, Vec3::NEG_Y, 3.5).is_some() }

    /// Godot `_bot_move_is_lava_safe`: on islands, safe ground with a floor
    /// under it 0.9, 1.6 and `look` m along `dir`.
    fn move_lava_safe(&self, me: Vec3, dir: Vec3, look: f32) -> bool {
        if self.arena.variant != crate::arena::Variant::Island || dir.length_squared() < 0.001 { return true; }
        let dir = dir.normalize();
        [0.9, 1.6, look].into_iter().all(|d| { let p = me + dir * d; self.arena.spawn_safe(p) && self.has_floor_at(p) })
    }

    /// Godot `_bot_lava_safe_move_dir`: the wanted direction if it is safe,
    /// else towards the centre or across it, else back, forward, left or
    /// right of the body; else stand still.
    fn lava_safe_dir(&self, i: usize, me: Vec3, want: Vec3) -> Vec3 {
        if want.length_squared() > 0.01 && self.move_lava_safe(me, want, LAVA_EDGE_PROBE) { return want.normalize(); }
        let mut cands = Vec::with_capacity(7);
        let to_centre = Vec3::new(-me.x, 0.0, -me.z);
        if to_centre.length_squared() > 0.01 {
            let c = to_centre.normalize();
            cands.extend([c, Vec3::new(-c.z, 0.0, c.x), Vec3::new(c.z, 0.0, -c.x)]);
        }
        let (s, c) = self.players[i].yaw.sin_cos();
        cands.extend([Vec3::new(s, 0.0, c), Vec3::new(-s, 0.0, -c), Vec3::new(-c, 0.0, s), Vec3::new(c, 0.0, -s)]);
        cands.into_iter().find(|&d| self.move_lava_safe(me, d, LAVA_EDGE_PROBE)).unwrap_or(Vec3::ZERO)
    }

    /// Godot `_bot_climbable_ledge_height`: a wall ahead with a top 1..2.6 m up.
    fn ledge_height(&self, me: Vec3, dir: Vec3) -> f32 {
        let dir = dir.normalize_or_zero();
        let from = me + Vec3::Y * 0.9;
        let Some((t, _, _)) = shape::raycast(&self.arena.grid, from, dir, 2.2) else { return 0.0 };
        let wall = from + dir * t;
        let top_from = Vec3::new(wall.x, me.y + 3.4, wall.z) + dir * 0.6;
        let Some((t2, _, _)) = shape::raycast(&self.arena.grid, top_from, Vec3::NEG_Y, 3.8) else { return 0.0 };
        let h = top_from.y - t2 - me.y;
        if !(CLIMB.0..=CLIMB.1).contains(&h) { return 0.0; }
        if !self.arena.spawn_safe(Vec3::new(top_from.x, top_from.y - t2 + 0.9, top_from.z)) { return 0.0; }
        h
    }
}
