//! Splash damage, a port of Godot's `blast.gd` (`Blast.apply`): every
//! explosion in the game is this with different parameters. Damage and
//! knockback can land late, at the speed of sound.
use crate::player::Blow;
use crate::{shape, ticks_for, Delayed, Event, World};
use glam::Vec3;

/// Which line of sight a blast needs to reach a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Los {
    None,
    /// Blast point to the capsule centre (grenades, mines).
    Center,
    /// 0.1 above the blast to 1.0 above the capsule centre (bullet splash).
    Torso,
}

#[derive(Clone, Copy, Debug)]
pub struct Blast {
    /// Damage at the edge as a fraction of the centre's.
    pub min_falloff: f32,
    /// The owner takes this much of it.
    pub self_mult: f32,
    pub knockback_mult: f32,
    /// Upward share of the knockback.
    pub up: f32,
    pub los: Los,
    /// Falloff on sqrt(1 - d/r) (bullet splash) or linear.
    pub sqrt_falloff: bool,
    /// Floor on the damage before the self share.
    pub min_damage: i32,
    /// Hits land after d / this (0: now). Godot uses 343 for grenades.
    pub sound_speed: f32,
}

impl Blast {
    /// Godot `_apply_bullet_splash`: explosive rounds, air strike, teleport blast.
    pub fn bullet(knockback_mult: f32) -> Self {
        Blast { min_falloff: 0.2, self_mult: 0.5, knockback_mult, up: 0.25, los: Los::Torso, sqrt_falloff: true, min_damage: 0, sound_speed: 0.0 }
    }
    /// Godot `grenade.gd` blasts (grenade, cluster, fragment, mine, dash bomb).
    pub fn grenade(self_mult: f32, min_damage: i32) -> Self {
        Blast { min_falloff: 0.2, self_mult, knockback_mult: 1.0, up: 0.4, los: Los::Center, sqrt_falloff: false, min_damage, sound_speed: 343.0 }
    }
}

impl World {
    pub(crate) fn blast(&mut self, pos: Vec3, radius: f32, damage: f32, shooter: Option<usize>, b: Blast) {
        if radius <= 0.0 { return; }
        self.events.push(Event::Explosion { pos, radius, owner: shooter.unwrap_or(0), kind: crate::ExplosionKind::Conventional });
        for j in 0..self.players.len() {
            let p = &self.players[j];
            if p.ghost { continue; }
            let at = p.pos;
            let dist = pos.distance(at);
            if dist > radius { continue; }
            let clear = |from: Vec3, to: Vec3| {
                let d = to - from;
                let len = d.length();
                len <= 1e-4 || shape::raycast(&self.arena.grid, from, d / len, len).is_none()
            };
            let seen = match b.los {
                Los::None => true,
                Los::Center => clear(pos, at),
                Los::Torso => clear(pos + Vec3::Y * 0.1, at + Vec3::Y),
            };
            if !seen { continue; }
            if let Some(s) = shooter && s != j && self.allied(j, s) { continue; }
            let lin = (1.0 - dist / radius).clamp(0.0, 1.0);
            let falloff = b.min_falloff + (1.0 - b.min_falloff) * lin;
            let dmg_f = if b.sqrt_falloff { b.min_falloff + (1.0 - b.min_falloff) * lin.sqrt() } else { falloff };
            let mut dmg = (damage * dmg_f) as i32;
            if b.min_damage > 0 { dmg = dmg.max(b.min_damage); }
            if Some(j) == shooter { dmg = (dmg as f32 * b.self_mult) as i32; }
            if dmg > 0 && let Some(s) = shooter { dmg = self.hooked_damage(j, s, dmg, false, true); }
            let away = at - pos;
            let dir = if away.length_squared() <= 0.001 { Vec3::Y } else { away.normalize() };
            let kb = dmg.max(0) as f32 * crate::weapon::KNOCKBACK_PER_DAMAGE * crate::weapon::BLAST_KNOCKBACK_MULT * b.knockback_mult.max(0.0);
            let impulse = dir * kb + Vec3::Y * kb * b.up;
            let hit = Delayed::Hit { victim: j, from: shooter, dmg, impulse, pos, radius, severity: falloff };
            let delay = if b.sound_speed > 0.0 { dist as f64 / b.sound_speed as f64 } else { 0.0 };
            self.later(delay, hit);
            if let Some(s) = shooter
                && dmg > 0 && s != j {
                    let d = if b.sound_speed > 0.0 { pos.distance(self.players[s].pos) as f64 / b.sound_speed as f64 } else { 0.0 };
                    self.later(d, Delayed::Credit { shooter: s, dmg });
                }
        }
    }

    /// Run now if under 0.01 s, else on the tick the delay runs out.
    pub(crate) fn later(&mut self, seconds: f64, what: Delayed) {
        if seconds < 0.01 { self.run_delayed(what); } else { self.delayed.push((ticks_for(seconds), what)); }
    }

    fn run_delayed(&mut self, what: Delayed) {
        match what {
            Delayed::Hit { victim, from, dmg, impulse, pos, radius, severity } => {
                if dmg > 0 {
                    let dir = impulse.normalize_or(Vec3::Y);
                    self.apply_damage(victim, dmg, from, Blow { pos, dir, head: false, blast_radius: radius, severity: severity.max(0.4), env: false, lava: None });
                }
                self.knockback(victim, impulse);
            }
            Delayed::Credit { shooter, dmg } => self.dealt_damage(shooter, dmg),
            Delayed::Echo { player } => self.echo(player),
        }
    }

    /// Timers: blast sound delays (Godot idle-time timers, after physics).
    pub(crate) fn step_delayed(&mut self) {
        let mut due = Vec::new();
        self.delayed.retain_mut(|(t, d)| {
            *t = t.saturating_sub(1);
            if *t == 0 { due.push(d.clone()); false } else { true }
        });
        for d in due { self.run_delayed(d); }
    }
}
