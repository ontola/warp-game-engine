//! RMB specials and everything they throw or call down: grenade (the
//! default), cluster grenade, teleport, zoom, sword, air strike and ion
//! cannon, with echo repeats, tactical reload and the refund on hit. Also
//! the other explosives that live in the world: impact mines and backblast
//! dash bombs, and the ninja and sword melee (`_melee_swung`).
//!
//! Unlike Godot, any number of air strikes and ion cannons can be under
//! way at once: Godot's single "pending" gate swallowed echoes and
//! simultaneous calls, which was a bug.
use crate::player::Blow;
use crate::{look_dir, shape, Blast, Carve, Delayed, Event, Projectile, ProjectileKind, Special, Strike, StrikeKind, World, DT, DT64};
use glam::Vec3;

pub const GRENADE_RELOAD: f64 = 3.0;
pub const CLUSTER_RELOAD: f64 = 4.0;
pub const AIR_STRIKE_RELOAD: f64 = 9.5;
pub const ION_CANNON_RELOAD: f64 = 14.0;
pub const MELEE_RELOAD: f64 = 0.5;
pub const TELEPORT_RELOAD: f64 = 2.0;
/// Ghost mines (unreachable in Godot: ghosts have 0 health and skip input).
#[allow(dead_code)]
pub const MINE_RELOAD: f64 = 2.5;
pub const ECHO_DELAY: f64 = 0.16;

pub const GRENADE_RADIUS: f32 = 0.2;
const GRENADE_SPEED: f32 = 22.0;
const GRENADE_LIFT: f32 = 4.0;
/// gravity_scale 1.5 x 9.8.
const GRENADE_GRAVITY: f32 = 14.7;
const GRENADE_DAMP: f32 = 0.1;
const MINE_LIFETIME: f64 = 10.0;
const MINE_ARM: f64 = 0.7;
const MINE_TRIGGER: f32 = 1.25;
const DASH_BOMB_FUSE: f64 = 1.0;

const TELEPORT_RANGE: f32 = 45.0;
const TELEPORT_OFFSET: f32 = 0.8;
const STRIKE_RANGE: f32 = 800.0;
pub const AIR_STRIKE_TRAVEL: f64 = 2.3;
pub const ION_CHARGE: f64 = 3.8;
pub const AIR_STRIKE_RADIUS: f32 = 30.0;
pub const AIR_STRIKE_DAMAGE: f32 = 165.0;
pub const ION_RADIUS: f32 = 38.0;
pub const ION_DAMAGE: f32 = 175.0;

const MELEE_RANGE: f32 = 5.0;
const MELEE_RADIUS: f32 = 1.2;
const MELEE_BACKSTAB: i32 = 9999;

impl ProjectileKind {
    /// (arm delay, fuse, proximity radius) for the thrown kinds.
    fn timing(self) -> (f64, f64, f32) {
        match self {
            ProjectileKind::Howitzer | ProjectileKind::SiegeBolt => (0.0, 15.0, 0.0),
            ProjectileKind::Grenade => (0.04, 4.0, 0.62),
            ProjectileKind::Cluster => (0.04, 4.0, 0.62 * 1.22),
            ProjectileKind::Fragment => (0.12, 1.8, 0.62 * 0.55),
            ProjectileKind::Mine => (MINE_ARM, MINE_LIFETIME, MINE_TRIGGER),
            ProjectileKind::DashBomb => (DASH_BOMB_FUSE, DASH_BOMB_FUSE, 0.0),
        }
    }
    fn thrown(self) -> bool { matches!(self, ProjectileKind::Grenade | ProjectileKind::Cluster | ProjectileKind::Fragment | ProjectileKind::Howitzer) }
}

impl World {
    /// Godot `_use_special`.
    pub(crate) fn use_special(&mut self, i: usize) {
        let p = &mut self.players[i];
        let mult = p.weapon.special_cooldown_mult as f64;
        if p.air_strike_charges > 0 {
            p.air_strike_charges -= 1;
            p.special_cd = AIR_STRIKE_RELOAD * mult;
            self.tactical_reload(i);
            self.events.push(Event::Special { player: i, special: Special::AirStrike });
            self.call_strike(i, StrikeKind::AirStrike);
            return;
        }
        let special = p.weapon.special;
        p.special_cd = match special {
            Special::ClusterGrenade => CLUSTER_RELOAD,
            Special::AirStrike => AIR_STRIKE_RELOAD,
            Special::IonCannon => ION_CANNON_RELOAD,
            Special::Sword => MELEE_RELOAD,
            Special::Teleport => TELEPORT_RELOAD,
            Special::Zoom | Special::Grenade => GRENADE_RELOAD,
        } * mult;
        self.tactical_reload(i);
        self.events.push(Event::Special { player: i, special });
        self.special_effect(i);
        for k in 0..self.players[i].weapon.special_echo_count.max(0) {
            self.later(ECHO_DELAY * (k + 1) as f64, Delayed::Echo { player: i });
        }
    }

    fn tactical_reload(&mut self, i: usize) {
        let p = &mut self.players[i];
        let n = p.weapon.special_reload_amount;
        if n > 0 {
            p.ammo = (p.ammo + n as u32).min(p.mag_size());
            if p.ammo > 0 && p.reloading { p.reloading = false; p.rifle_cd = 0.0; }
        }
    }

    /// An echo re-reads the special, aim and position at the time it fires.
    pub(crate) fn echo(&mut self, i: usize) {
        let p = &self.players[i];
        if !p.ghost && p.alive && p.health > 0 { self.special_effect(i); }
    }

    fn special_effect(&mut self, i: usize) {
        match self.players[i].weapon.special {
            Special::Teleport => self.teleport(i),
            Special::Sword => { let p = &self.players[i]; let (o, d) = (p.eye(), p.forward()); self.melee(i, o, d, 1.0, 1.0); }
            Special::AirStrike => self.call_strike(i, StrikeKind::AirStrike),
            Special::IonCannon => self.call_strike(i, StrikeKind::IonCannon),
            Special::ClusterGrenade => self.throw(i, ProjectileKind::Cluster),
            Special::Grenade | Special::Zoom => self.throw(i, ProjectileKind::Grenade),
        }
    }

    /// Godot `_fire_grenade_internal`: from the muzzle along the camera.
    fn throw(&mut self, i: usize, kind: ProjectileKind) {
        let p = &self.players[i];
        let tracked = self.tracked_ray(i);
        let (muzzle, dir) = tracked.unwrap_or((p.muzzle(), p.forward()));
        // Tracked weapons already sit at the physical muzzle; do not push a
        // grenade through cover by adding the desktop forward offset.
        let at = if tracked.is_some() { muzzle } else { muzzle + dir * 0.6 };
        self.spawn_projectile(kind, at, dir * GRENADE_SPEED + Vec3::Y * GRENADE_LIFT, i);
    }

    pub(crate) fn spawn_projectile(&mut self, kind: ProjectileKind, pos: Vec3, vel: Vec3, owner: usize) {
        self.projectiles.push(Projectile { kind, pos, vel, owner, age: 0.0, contact: false });
    }


    /// Godot `_use_teleport`: to where the aim meets the world (or 45 m out).
    fn teleport(&mut self, i: usize) {
        let p = &self.players[i];
        let (eye, dir) = (p.eye(), p.forward());
        let to = match self.ray_world(eye, dir, TELEPORT_RANGE, &[]) {
            Some((t, n, _)) => eye + dir * t + n * TELEPORT_OFFSET,
            None => eye + dir * TELEPORT_RANGE,
        };
        let from = p.pos;
        let r = p.weapon.teleport_blast_radius;
        let kb = p.weapon.knockback_mult;
        let p = &mut self.players[i];
        p.pos = to;
        p.vel = Vec3::ZERO;
        self.events.push(Event::Teleport { player: i, from, to });
        if r > 0.0 {
            self.blast(from + Vec3::Y * 0.1, r, 55.0, Some(i), Blast::bullet(kb));
            self.blast(to + Vec3::Y * 0.1, r, 55.0, Some(i), Blast::bullet(kb));
        }
    }

    /// Godot `_air_strike_target`: the aim point, players included.
    fn strike_target(&self, i: usize) -> Vec3 {
        let p = &self.players[i];
        let (eye, dir) = (p.eye(), p.forward());
        match self.ray_all(eye, dir, STRIKE_RANGE, &[i as u16], &[], false) {
            Some((t, _)) => eye + dir * t + Vec3::Y * 0.05,
            None => eye + dir * STRIKE_RANGE,
        }
    }

    fn call_strike(&mut self, i: usize, kind: StrikeKind) {
        if self.phase != crate::Phase::Playing { return; }
        let target = self.strike_target(i);
        self.strikes.push(Strike { kind, target, owner: Some(i), age: 0.0 });
    }

    pub(crate) fn step_strikes(&mut self) {
        let mut due = Vec::new();
        self.strikes.retain_mut(|s| {
            s.age += DT64;
            let t = match s.kind { StrikeKind::AirStrike => AIR_STRIKE_TRAVEL, StrikeKind::IonCannon => ION_CHARGE };
            if s.age + 1e-9 >= t { due.push(s.clone()); false } else { true }
        });
        for s in due {
            match s.kind {
                StrikeKind::AirStrike => {
                    let kb = s.owner.map_or(1.0, |o| self.players[o].weapon.knockback_mult);
                    self.blast(s.target + Vec3::Y * 0.1, AIR_STRIKE_RADIUS, AIR_STRIKE_DAMAGE, s.owner, Blast::bullet(kb));
                    self.carves.push(Carve { pos: s.target, radius: AIR_STRIKE_RADIUS * 0.52, damage: AIR_STRIKE_DAMAGE });
                }
                StrikeKind::IonCannon => self.ion_detonate(s.target, s.owner),
            }
            self.flow.strike_finished(s.kind, s.owner.is_none());
        }
    }

    /// The beam: a 38 m vertical column through any cover.
    fn ion_detonate(&mut self, target: Vec3, owner: Option<usize>) {
        self.events.push(Event::Explosion { pos: target, radius: ION_RADIUS, owner: owner.unwrap_or(0), kind: crate::ExplosionKind::Ion });
        let bottom = self.arena.lava_surface() - 20.0;
        let top = target.y + 880.0;
        for j in 0..self.players.len() {
            let p = &self.players[j];
            if p.ghost { continue; }
            if let Some(o) = owner && o != j && self.allied(j, o) { continue; }
            if p.pos.y < bottom || p.pos.y > top { continue; }
            let flat = glam::Vec2::new(p.pos.x - target.x, p.pos.z - target.z).length();
            if flat > ION_RADIUS { continue; }
            let falloff = 0.58 + (1.0 - 0.58) * (1.0 - flat / ION_RADIUS);
            let mut dmg = (ION_DAMAGE * falloff) as i32;
            if Some(j) == owner { dmg = (dmg as f32 * 0.5) as i32; }
            let away = p.pos - target;
            let dir = if away.length_squared() <= 0.001 { Vec3::Y } else { away.normalize() };
            let kb = crate::Weapon::knockback_from_damage(dmg as f32, 1.0, true);
            let impulse = dir * kb + Vec3::Y * kb * 0.25;
            if dmg > 0 {
                let dmg = match owner { Some(o) => self.hooked_damage(j, o, dmg, false, true), None => dmg };
                self.apply_damage(j, dmg, owner, Blow { pos: target, dir, head: false, blast_radius: ION_RADIUS, severity: falloff, env: false, lava: None });
                if let Some(o) = owner && o != j { self.dealt_damage(o, dmg); }
            }
            self.knockback(j, impulse);
        }
        self.carves.push(Carve { pos: target, radius: ION_RADIUS * 0.52, damage: ION_DAMAGE });
    }

    /// Godot `_melee_swung`: a capsule sweep along `dir` against the hit
    /// spheres; a strike from behind is a backstab.
    pub(crate) fn melee(&mut self, i: usize, origin: Vec3, dir: Vec3, damage_mult: f32, range_mult: f32) {
        let w = self.players[i].weapon.clone();
        let range = MELEE_RANGE * w.melee_scale * range_mult;
        let radius = MELEE_RADIUS * w.melee_scale * range_mult;
        let height = range.max(radius * 2.5);
        self.events.push(Event::Slash { player: i, from: origin, dir, range: height });
        let (a, b) = (origin + dir * radius, origin + dir * (height - radius).max(radius));
        let mut hits = Vec::new();
        for (j, p) in self.players.iter().enumerate() {
            if j == i || !p.targetable() || self.allied(j, i) { continue; }
            let touches = p.hitboxes().iter().any(|(c, r, _)| seg_dist(a, b, *c) <= radius + r.max_element());
            if !touches { continue; }
            let to = p.pos + Vec3::Y * 0.6 - origin;
            let len = to.length();
            if len > 1e-4 && shape::raycast(&self.arena.grid, origin, to / len, len).is_some() { continue; }
            hits.push(j);
        }
        for j in hits {
            let backstab = look_dir(self.players[j].yaw, 0.0).dot(dir) > 0.4;
            let dmg = if backstab && damage_mult >= 0.99 { MELEE_BACKSTAB } else {
                let base = w.melee_damage() as f32 * damage_mult;
                (if backstab { base * 2.0 } else { base }) as i32
            };
            let dmg = self.hooked_damage(j, i, dmg, false, false);
            let kb = dmg as f32 * crate::weapon::KNOCKBACK_PER_DAMAGE * w.knockback_mult;
            let at = self.players[j].pos + Vec3::Y * 0.6;
            self.apply_damage(j, dmg, Some(i), Blow { pos: at, dir, head: false, blast_radius: 0.0, severity: 0.0, env: false, lava: None });
            if kb > 0.0 { self.knockback(j, dir * kb + Vec3::Y * kb * 0.25); }
            self.dealt_damage(i, dmg);
        }
    }

    /// Grenades, fragments, mines and dash bombs: fuse, proximity, flight, contact.
    pub(crate) fn step_projectiles(&mut self) {
        let mut k = 0;
        while k < self.projectiles.len() {
            let mut g = self.projectiles[k].clone();
            g.age += DT64;
            if g.kind == ProjectileKind::SiegeBolt {
                if g.age >= 8.0 { self.projectiles.remove(k); continue; }
                g.vel.y -= GRENADE_GRAVITY * DT;
                let step = g.vel * DT;
                let len = step.length();
                let dir = step.normalize_or_zero();
                let wall = shape::raycast(&self.arena.grid, g.pos, dir, len + 0.22);
                let mut limit = wall.map_or(len, |(t,_,_)| (t-0.22).max(0.0));
                let mut victim = None;
                for (j,p) in self.players.iter().enumerate() {
                    if j == g.owner || !p.alive || p.ghost || self.allied(j,g.owner) { continue; }
                    for (c,radii,_) in p.hitboxes() {
                        if let Some((t,_)) = crate::combat::ray_ellipsoid(g.pos,dir,c,radii+Vec3::splat(0.22),p.yaw,limit) {
                            limit=t; victim=Some(j);
                        }
                    }
                }
                if let Some(j) = victim {
                    let damage=self.players[j].health.max(1);
                    self.apply_damage(j,damage,Some(g.owner),Blow { dir, ..Blow::plain(g.pos+dir*limit) });
                }
                if victim.is_some() || wall.is_some() { self.projectiles.remove(k); continue; }
                g.pos += step;
                self.projectiles[k]=g;
                k+=1;
                continue;
            }
            let (arm, fuse, near) = g.kind.timing();
            let mut boom = false;
            match g.kind {
                ProjectileKind::Mine if g.age >= MINE_LIFETIME => { self.projectiles.remove(k); continue; }
                ProjectileKind::Mine => {}
                _ if g.age >= fuse => boom = true,
                _ => {}
            }
            if !boom && near > 0.0 && g.age >= arm {
                let mine = g.kind == ProjectileKind::Mine;
                boom = self.players.iter().any(|p| !p.ghost && (mine || p.health > 0 || p.downed) && p.pos.distance(g.pos) <= near);
            }
            if !boom && g.kind.thrown() {
                // Damped, then gravity, then the move (GodotPhysics order).
                if g.kind != ProjectileKind::Howitzer { g.vel *= (1.0 - GRENADE_DAMP * DT).max(0.0); }
                g.vel.y -= GRENADE_GRAVITY * DT;
                let step = g.vel * DT;
                let len = step.length();
                let mut touching = false;
                if len > 1e-6 {
                    let dir = step / len;
                    match shape::raycast(&self.arena.grid, g.pos, dir, len + GRENADE_RADIUS) {
                        Some((t, n, _)) => {
                            touching = true;
                            g.pos += dir * (t - GRENADE_RADIUS).max(0.0) + n * 0.002;
                            // No bounce, full friction: it stops on the surface.
                            g.vel -= n * g.vel.dot(n);
                            g.vel *= 0.5;
                        }
                        None => g.pos += step,
                    }
                }
                if !touching {
                    touching = shape::raycast(&self.arena.grid, g.pos, Vec3::NEG_Y, GRENADE_RADIUS + 0.01).is_some();
                }
                if touching && !g.contact && g.age >= arm { boom = true; }
                g.contact = touching;
            }
            if boom {
                self.projectiles.remove(k);
                self.explode_projectile(&g);
                continue;
            }
            self.projectiles[k] = g;
            k += 1;
        }
    }

    fn explode_projectile(&mut self, g: &Projectile) {
        let (pos, owner) = (g.pos, Some(g.owner));
        let (radius, damage, b) = match g.kind {
            ProjectileKind::Grenade | ProjectileKind::Mine => (6.0, 100.0, Blast::grenade(0.4, 20)),
            ProjectileKind::Cluster => (3.5, 35.0, Blast::grenade(0.35, 12)),
            ProjectileKind::Fragment => (3.2, 48.0, Blast::grenade(0.4, 14)),
            ProjectileKind::Howitzer => (10.0, 120.0, Blast::grenade(0.5, 28)),
            ProjectileKind::DashBomb => (2.8, 42.0, Blast::grenade(0.4, 10)),
            ProjectileKind::SiegeBolt => return,
        };
        self.blast(pos, radius, damage, owner, b);
        self.carves.push(Carve { pos, radius: radius * 0.52, damage });
        if g.kind == ProjectileKind::Cluster { self.cluster_fragments(pos, g.owner); }
    }

    /// Seven fragments on a golden-angle sphere, tilted up, jittered.
    fn cluster_fragments(&mut self, pos: Vec3, owner: usize) {
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        for i in 0..7 {
            let y = 1.0 - (i as f32 + 0.5) / 7.0 * 2.0;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let th = golden * i as f32;
            let d = Vec3::new(th.cos() * r, y, th.sin() * r).normalize();
            let d = d.lerp(Vec3::Y, 0.18).normalize();
            let d = glam::Quat::from_rotation_y(self.rng.range(-0.35, 0.35)) * d;
            let v = d * (13.0 + self.rng.range(-1.5, 2.0)) + Vec3::Y * (5.0 + self.rng.range(-0.5, 1.0));
            self.spawn_projectile(ProjectileKind::Fragment, pos + Vec3::Y * 0.15, v, owner);
        }
    }

    /// A bullet hit a projectile: grenades go off, mines are disarmed.
    pub(crate) fn detonate_projectile(&mut self, k: usize) {
        if k >= self.projectiles.len() { return; }
        let g = self.projectiles.remove(k);
        match g.kind {
            ProjectileKind::Mine => {}
            ProjectileKind::DashBomb => self.projectiles.insert(k, g),
            _ => self.explode_projectile(&g),
        }
    }

}

/// Distance from `p` to the segment a..b.
fn seg_dist(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-9 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}
