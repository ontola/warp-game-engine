//! Computer players: pick the nearest enemy, turn towards it at a capped
//! rate with a wandering aim error, fire once on target, strafe, jump over
//! obstacles, dash now and then, and never walk off a ledge into the lava.
use crate::rng::Rng;
use crate::{shape, Input, World, DT, RANGE};
use glam::{Vec2, Vec3};
use std::f32::consts::{PI, TAU};

const TURN_RATE: f32 = 7.0;
const REACTION: f32 = 0.3;

#[derive(Clone, Debug, Default)]
pub struct Brain {
    rng: Option<Rng>,
    target: Option<usize>,
    seen_for: f32,
    strafe: f32,
    strafe_timer: f32,
    aim_err: Vec2,
    dash_timer: f32,
    last_pos: Vec3,
    stuck: f32,
}

fn wrap(a: f32) -> f32 { (a + PI).rem_euclid(TAU) - PI }

impl Brain {
    pub fn new(seed: u64) -> Self { Brain { rng: Some(Rng::new(seed)), strafe: 1.0, dash_timer: 2.0, ..Default::default() } }

    pub fn think(&mut self, w: &World, me: usize) -> Input {
        let rng = self.rng.get_or_insert_with(|| Rng::new(me as u64));
        let p = &w.players[me];
        let eye = p.eye();
        let visible = |j: usize| {
            let to = w.players[j].pos + Vec3::Y * 0.3 - eye;
            let d = to.length();
            d < RANGE && shape::raycast(&w.arena.grid, eye, to / d, d).is_none()
        };
        let mut best: Option<(usize, f32, bool)> = None;
        for (j, o) in w.players.iter().enumerate() {
            if j == me || !o.alive { continue; }
            let d = o.pos.distance(p.pos);
            let vis = visible(j);
            // Prefer visible enemies, then the closest.
            let score = d - if vis { 1000.0 } else { 0.0 };
            if best.is_none_or(|b| score < b.1) { best = Some((j, score, vis)); }
        }
        let mut input = Input::default();
        let Some((t, _, vis)) = best else { return input };
        if self.target != Some(t) { self.seen_for = 0.0; self.target = Some(t); }
        self.seen_for = if vis { self.seen_for + DT } else { 0.0 };

        // Aim: chest, led by the target's velocity over the bullet flight.
        let o = &w.players[t];
        let flight = o.pos.distance(eye) / crate::BULLET_SPEED;
        let aim = o.pos + Vec3::Y * 0.2 + o.vel * flight - eye;
        let dist = aim.length();
        let d = aim / dist;
        self.aim_err = (self.aim_err + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)) * 0.004).clamp_length_max(0.05);
        let want_yaw = (-d.x).atan2(-d.z) + self.aim_err.x;
        let want_pitch = d.y.clamp(-1.0, 1.0).asin() + self.aim_err.y;
        let dy = wrap(want_yaw - p.yaw);
        let dp = want_pitch - p.pitch;
        let max = TURN_RATE * DT;
        input.look = [dy.clamp(-max, max), dp.clamp(-max, max)];
        input.fire = vis && self.seen_for > REACTION && dy.abs() < 0.06 && dp.abs() < 0.06;

        // Movement: hold a mid range and strafe across the line of fire.
        self.strafe_timer -= DT;
        if self.strafe_timer <= 0.0 {
            self.strafe = if rng.f() < 0.5 { -1.0 } else { 1.0 };
            self.strafe_timer = rng.range(0.5, 1.6);
        }
        let fwd = if !vis { 1.0 } else if dist > 20.0 { 0.8 } else if dist < 8.0 { -0.6 } else { 0.0 };
        let mut mv = Vec2::new(if vis { self.strafe } else { self.strafe * 0.3 }, fwd);
        // Look-relative move to world, to probe the ground ahead.
        let (sy, cy) = (p.yaw + input.look[0]).sin_cos();
        let to_world = |m: Vec2| Vec3::new(cy, 0.0, -sy) * m.x + Vec3::new(-sy, 0.0, -cy) * m.y;
        let solid_ahead = |m: Vec2| {
            let probe = p.pos + to_world(m).normalize_or_zero() * 2.5;
            shape::raycast(&w.arena.grid, probe + Vec3::Y * 0.5, Vec3::NEG_Y, 8.0).is_some()
        };
        if p.on_floor && mv != Vec2::ZERO && !solid_ahead(mv) {
            mv = -mv;
            self.strafe = -self.strafe;
            if !solid_ahead(mv) { mv = Vec2::ZERO; }
        }
        input.mv = mv.into();

        let moved = Vec2::new(p.pos.x - self.last_pos.x, p.pos.z - self.last_pos.z).length();
        self.last_pos = p.pos;
        self.stuck = if mv != Vec2::ZERO && moved < WALKING_STUCK { self.stuck + DT } else { 0.0 };
        input.jump = (p.on_floor && self.stuck > 0.15)
            || (p.wall_normal.is_some() && p.vel.y < 0.0)
            || (p.on_floor && vis && rng.f() < 0.008);
        self.dash_timer -= DT;
        if self.dash_timer <= 0.0 && p.on_floor && mv != Vec2::ZERO && solid_ahead(mv * 2.0) {
            input.dash = true;
            self.dash_timer = rng.range(2.0, 5.0);
        }
        input
    }
}

const WALKING_STUCK: f32 = 0.02;
