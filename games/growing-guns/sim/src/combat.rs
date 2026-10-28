//! The rifle (Godot `_handle_combat_input`, `_fire_rifle`) and its bullets
//! (`bullet.gd`): pellets in a centre-biased cone from the muzzle toward
//! where the camera ray lands, flight with drop and homing, rays every
//! other tick, hits on the three hit ellipsoids, ricochet, drill, body
//! bounce, explosive splash, impact mines, poison, slow and the shooter's
//! on-hit effects.
use crate::player::Blow;
use crate::{destruct, shape, Blast, Bullet, Carve, Event, Input, Player, ProjectileKind, Special, World, DT, RANGE};
use glam::{Quat, Vec3};

/// Faster than this a bullet is a single instant ray (Godot HITSCAN_SPEED).
const HITSCAN_SPEED: f32 = 550.0;
const MAX_BLOOM: f32 = 0.14;
const RIFLE_RECOIL_PITCH: f32 = 0.018;
const AIM_RAY: f32 = 200.0;
/// Homing looks for targets in this cone (cosine).
const HOMING_CONE: f32 = 0.82;

/// What a bullet ray found.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    World { normal: Vec3, chunk: usize },
    Player { index: usize, head: bool, normal: Vec3 },
    Projectile { index: usize, normal: Vec3 },
}

impl World {
    pub(crate) fn combat_input(&mut self, i: usize, input: Input, special_edge: bool, fire_edge: bool) {
        let playing = self.phase == crate::Phase::Playing || !self.players[i].bot;
        let p = &self.players[i];
        if fire_edge && playing && (p.reloading || p.ammo == 0) { self.events.push(crate::Event::Empty { player: i }); }
        let p = &self.players[i];
        if input.fire && playing && !p.reloading && p.ammo > 0 && p.rifle_cd <= 0.0 {
            let interval = p.weapon.fire_interval();
            let p = &mut self.players[i];
            p.rifle_cd = interval;
            p.ammo -= 1;
            p.since_shot = 0.0;
            self.fire_rifle(i);
            if self.players[i].ammo == 0 { self.start_reload(i); }
        }
        if input.reload { self.start_reload(i); }
        let p = &mut self.players[i];
        if p.weapon.special == Special::Zoom && p.air_strike_charges == 0 {
            p.zooming = input.special;
        } else {
            p.zooming = false;
            if special_edge && p.special_cd <= 0.0 { self.use_special(i); }
        }
    }

    pub(crate) fn start_reload(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.reloading || p.ammo >= p.mag_size() { return; }
        p.reloading = true;
        p.rifle_cd = p.weapon.reload_time();
        let seconds = p.rifle_cd as f32;
        self.events.push(crate::Event::Reload { player: i, seconds });
    }

    /// Godot `_fire_rifle`.
    fn fire_rifle(&mut self, i: usize) {
        let cam = self.players[i].eye();
        let dir_cam = self.players[i].forward();
        let aim = self.ray_all(cam, dir_cam, AIM_RAY, &[i as u16], &[], false).map_or(cam + dir_cam * AIM_RAY, |(t, _)| cam + dir_cam * t);
        let p = &self.players[i];
        let origin = p.muzzle();
        let base = (aim - origin).normalize_or(dir_cam);
        let w = &p.weapon;
        let last = p.ammo == 0;
        let scale = w.bullet_scale_for_shot(last);
        let recoil_scale = scale * (w.recoil() / crate::weapon::BASE_RECOIL).clamp(0.2, 2.5);
        let kick = (0.4 * (w.damage_for_shot(last) / crate::weapon::BASE_DAMAGE).powf(1.6)).clamp(0.1, 12.0);
        let shots = w.shots_per_trigger();
        let spread = self.effective_spread(i);
        let recoil = w.recoil();
        let jitter = self.rng.range(-0.004, 0.004) * recoil_scale;
        let p = &mut self.players[i];
        p.recoil_pitch += RIFLE_RECOIL_PITCH * recoil_scale;
        p.yaw += jitter;
        p.vel -= dir_cam * kick * shots as f32;
        p.bloom = (p.bloom + recoil).min(MAX_BLOOM);
        let (right, up, _) = self.players[i].basis();
        for _ in 0..shots {
            let mut dir = base;
            if spread > 0.0 {
                let theta = self.rng.f() * std::f32::consts::TAU;
                let r = spread * self.rng.f() * self.rng.f();
                dir = (Quat::from_axis_angle(right, r * theta.sin()) * (Quat::from_axis_angle(up, r * theta.cos()) * dir)).normalize();
            }
            self.spawn_bullet(i, origin, dir, last);
        }
    }

    /// Spread before this shot's bloom: moving widens it.
    pub fn effective_spread(&self, i: usize) -> f32 {
        let p = &self.players[i];
        let move_factor = (Vec3::new(p.vel.x, 0.0, p.vel.z).length() / crate::WALK_SPEED).clamp(0.0, 1.0);
        (p.weapon.spread() * (1.0 + move_factor) + p.bloom).min(MAX_BLOOM)
    }

    /// Godot `_rifle_fired` + `bullet.setup`.
    pub(crate) fn spawn_bullet(&mut self, owner: usize, origin: Vec3, dir: Vec3, last_in_mag: bool) {
        let w = &self.players[owner].weapon;
        let speed = w.bullet_speed();
        let mut b = Bullet {
            pos: origin, vel: dir * speed, owner, travelled: 0.0, last_in_mag, ricochet_left: w.ricochet_count, pierce_left: w.world_pierce_count,
            ricochet_hits: 0, body_bounces: 0, query_tick: false, last_query: None, skip_players: vec![owner as u16], skip_volumes: Vec::new(), fresh: true,
        };
        self.events.push(Event::Shot { player: owner, from: origin, dir });
        if speed >= HITSCAN_SPEED {
            // One instant ray; no ricochet, drill or pass-through after it.
            if let Some((t, hit)) = self.ray_all(origin, dir, RANGE, &b.skip_players, &[], true) {
                b.travelled = t;
                b.pos = origin + dir * t;
                self.bullet_hit(&mut b, hit);
            }
            return;
        }
        self.bullets.push(b);
    }

    pub(crate) fn step_bullets(&mut self) {
        let mut bullets = std::mem::take(&mut self.bullets);
        bullets.retain_mut(|b| {
            if b.fresh { b.fresh = false; return true; }
            self.bullet_tick(b)
        });
        // Bullets spawned during this pass (none today) come after.
        bullets.append(&mut self.bullets);
        self.bullets = bullets;
    }

    /// One tick of flight; false when the bullet is gone.
    fn bullet_tick(&mut self, b: &mut Bullet) -> bool {
        let Some(owner) = self.players.get(b.owner) else { return false };
        let w = &owner.weapon;
        let (drop, homing) = (w.bullet_drop * self.bullet_drop_mult(), w.homing);
        if drop > 0.0 { b.vel.y -= drop * DT; }
        let prev_dir = b.vel.normalize_or(Vec3::NEG_Z);
        if homing > 0.0 { self.home(b, prev_dir, homing); }
        let step = b.vel * DT;
        let from = *b.last_query.get_or_insert(b.pos);
        b.query_tick = !b.query_tick;
        let hit = if b.query_tick {
            let seg = b.pos + step - from;
            let len = seg.length();
            let hit = if len > 1e-6 { self.ray_all(from, seg / len, len, &b.skip_players, &b.skip_volumes, true).map(|(t, h)| (from + seg / len * t, h)) } else { None };
            if hit.is_none() { b.last_query = Some(b.pos + step); }
            hit
        } else { None };
        match hit {
            None => {
                b.pos += step;
                b.travelled += step.length();
            }
            Some((at, target)) => {
                b.pos = at;
                if !self.bullet_hit(b, target) { return false; }
                b.last_query = Some(b.pos);
            }
        }
        b.travelled <= RANGE
    }

    /// Godot bullet `_update_homing`: turn toward the best target ahead.
    fn home(&self, b: &mut Bullet, prev_dir: Vec3, deg_per_s: f32) {
        let mut best = HOMING_CONE;
        let mut target = None;
        for (j, p) in self.players.iter().enumerate() {
            if j == b.owner || !p.targetable() || p.health <= 0 || self.allied(j, b.owner) { continue; }
            let to = p.pos - b.pos;
            if to.length_squared() < 0.04 { continue; }
            let score = prev_dir.dot(to.normalize());
            if score > best { best = score; target = Some(p.pos); }
        }
        let Some(t) = target else { return };
        let v = b.vel.length();
        let d = if v < 1e-4 { prev_dir } else { b.vel / v };
        let to = (t - b.pos).normalize();
        let ang = d.dot(to).clamp(-1.0, 1.0).acos();
        if ang <= 1e-4 { return; }
        let turn = ang.min(deg_per_s.to_radians() * DT);
        b.vel = slerp(d, to, turn / ang).normalize() * v;
    }

    /// Closest of: the arena (minus drilled volumes), hit ellipsoids of
    /// players not skipped, and (for bullets) grenades and mines.
    pub(crate) fn ray_all(&self, from: Vec3, dir: Vec3, max: f32, skip_players: &[u16], skip_volumes: &[u32], projectiles: bool) -> Option<(f32, Target)> {
        let mut best: Option<(f32, Target)> = self.ray_world(from, dir, max, skip_volumes).map(|(t, normal, chunk)| (t, Target::World { normal, chunk }));
        let lim = |best: &Option<(f32, Target)>| best.map_or(max, |b| b.0);
        for (j, p) in self.players.iter().enumerate() {
            if !p.alive || skip_players.contains(&(j as u16)) { continue; }
            if p.pos.distance(from) > lim(&best) + 3.0 * p.weapon.body_scale.max(1.0) * p.weapon.head_scale.max(1.0) { continue; }
            for (c, radii, head) in p.hitboxes() {
                if let Some((t, n)) = ray_ellipsoid(from, dir, c, radii, p.yaw, lim(&best))
                    && best.is_none_or(|b| t < b.0) { best = Some((t, Target::Player { index: j, head, normal: n })); }
            }
        }
        if projectiles {
            for (k, g) in self.projectiles.iter().enumerate() {
                if g.kind == ProjectileKind::DashBomb { continue; }
                if let Some(t) = shape::ray_sphere(from, dir, g.pos, crate::specials::GRENADE_RADIUS, lim(&best)).filter(|&t| t > 0.0) {
                    let n = (from + dir * t - g.pos).normalize_or(Vec3::Y);
                    best = Some((t, Target::Projectile { index: k, normal: n }));
                }
            }
        }
        best
    }

    /// The arena, skipping drilled volumes.
    pub(crate) fn ray_world(&self, from: Vec3, dir: Vec3, max: f32, skip_volumes: &[u32]) -> Option<(f32, Vec3, usize)> {
        let mut start = 0.0;
        for _ in 0..64 {
            let (t, n, k) = shape::raycast(&self.arena.grid, from + dir * start, dir, max - start)?;
            let vol = self.arena.chunk(k).map(|c| c.volume);
            if vol.is_none_or(|v| !skip_volumes.contains(&v)) { return Some((start + t, n, k)); }
            start += t + 1e-3;
            if start >= max { return None; }
        }
        None
    }

    /// Godot bullet `_handle_collision`; false when the bullet stops.
    fn bullet_hit(&mut self, b: &mut Bullet, target: Target) -> bool {
        let owner = b.owner;
        let w = self.players[owner].weapon.clone();
        let mut damage = w.damage_for_shot(b.last_in_mag);
        if w.grow_damage_per_meter > 0.0 { damage *= 1.0 + b.travelled * w.grow_damage_per_meter; }
        if b.ricochet_hits > 0 && w.ricochet_damage_mult > 1.0 { damage *= w.ricochet_damage_mult.powi(b.ricochet_hits); }
        let dir = b.vel.normalize_or(Vec3::NEG_Z);
        let at = b.pos;
        let normal = match target {
            Target::Player { index, head, normal } => {
                let victim = &self.players[index];
                if victim.ghost || self.allied(index, owner) {
                    b.skip_players.push(index as u16);
                    return true;
                }
                let bounce = !head && victim.weapon.body_ricochet_count > b.body_bounces;
                if bounce { b.body_bounces += 1; }
                self.bullet_damage(owner, index, damage, head, at, dir, &w);
                if w.explosive_radius > 0.0 { self.blast(at + normal * 0.1, w.explosive_radius, w.explosive_damage, Some(owner), Blast::bullet(w.knockback_mult)); }
                if bounce {
                    b.vel = reflect(b.vel, normal);
                    b.pos += b.vel.normalize_or(Vec3::Y) * 0.05;
                    b.skip_players.push(index as u16);
                    return true;
                }
                return false;
            }
            Target::World { normal, .. } => normal,
            Target::Projectile { index, normal } => {
                self.detonate_projectile(index);
                normal
            }
        };
        // Arena chip, explosion, mine.
        let ratio = (damage / crate::weapon::BASE_DAMAGE).max(0.2);
        if w.explosive_radius > 0.0 {
            self.carves.push(Carve { pos: at, radius: w.explosive_radius * 0.52, damage: if w.explosive_damage > 0.0 { w.explosive_damage } else { 30.0 } });
        } else {
            let (chip, radius) = destruct::rifle_chip(ratio);
            let strong = chip >= destruct::MIN_DESTRUCTION_DAMAGE;
            self.events.push(Event::Impact { pos: at, normal, damage: chip, strong });
            self.carves.push(Carve { pos: at, radius, damage: chip });
        }
        if w.explosive_radius > 0.0 { self.blast(at + normal * 0.1, w.explosive_radius, w.explosive_damage, Some(owner), Blast::bullet(w.knockback_mult)); }
        if w.impact_mines > 0 { self.spawn_projectile(ProjectileKind::Mine, at + normal * 0.14, Vec3::ZERO, owner); }
        // Drill first, then ricochet.
        if let Target::World { chunk, .. } = target {
            if b.pierce_left > 0 {
                b.pierce_left -= 1;
                if let Some(c) = self.arena.chunk(chunk) { b.skip_volumes.push(c.volume); }
                b.pos = at + dir * 0.12;
                b.travelled += 0.12;
                self.events.push(Event::Ricochet { pos: at });
                return true;
            }
        } else if b.pierce_left > 0 {
            b.pierce_left -= 1;
            b.pos = at + dir * 0.12;
            b.travelled += 0.12;
            return true;
        }
        if b.ricochet_left > 0 {
            b.ricochet_left -= 1;
            b.ricochet_hits += 1;
            b.vel = reflect(b.vel, normal);
            b.pos += b.vel.normalize_or(Vec3::Y) * 0.05;
            self.events.push(Event::Ricochet { pos: at });
            return true;
        }
        false
    }

    /// Damage, poison, slow, knockback and on-hit credit for a bullet hit.
    #[allow(clippy::too_many_arguments)]
    fn bullet_damage(&mut self, owner: usize, victim: usize, damage: f32, head: bool, at: Vec3, dir: Vec3, w: &crate::Weapon) {
        let mut dmg = (damage * if head { w.headshot() } else { 1.0 }) as i32;
        if !head { dmg = (dmg as f32 * self.body_damage_mult()) as i32; }
        dmg = self.hooked_damage(victim, owner, dmg, head, false);
        let (mut poison, mut direct) = (0, dmg);
        if w.damage_over_time > 0.0 {
            poison = ((dmg as f32 * (1.0 + w.damage_over_time)).round() as i32).max(1);
            direct = 0;
        }
        let knock_dir = (dir + Vec3::new(0.0, 0.18, 0.0)).normalize();
        let knock = w.knockback_force(dmg.max(poison) as f32, false, head);
        if direct > 0 {
            self.apply_damage(victim, direct, Some(owner), Blow { pos: at, dir: knock_dir, head, blast_radius: w.explosive_radius, severity: if w.explosive_radius > 0.0 { 1.0 } else { 0.0 }, env: false });
        }
        if knock > 0.0 { self.knockback(victim, knock_dir * knock); }
        let v = &mut self.players[victim];
        if poison > 0 && w.dot_duration > 0.0 {
            v.poison_left += poison as f32;
            v.poison_dps += poison as f32 / w.dot_duration;
            v.poison_from = Some(owner);
        }
        if w.slow_on_hit < 1.0 && w.slow_duration > 0.0 {
            v.slow_mult = v.slow_mult.min(w.slow_on_hit.clamp(0.22, 1.0));
            v.slow_timer = v.slow_timer.max(w.slow_duration as f64);
        }
        self.dealt_damage(owner, direct.max(poison));
    }

    /// `headshots_only`: body hits deal a quarter.
    fn body_damage_mult(&self) -> f32 { if self.modifier == Some(crate::Modifier::HeadshotsOnly) { 0.25 } else { 1.0 } }
    pub(crate) fn bullet_drop_mult(&self) -> f32 { if self.modifier == Some(crate::Modifier::LowGravity) { 0.6 } else { 1.0 } }
}

impl Player {
    /// Seconds of reload done, 0..1, for the view.
    pub fn reload_progress(&self) -> f32 {
        if !self.reloading { return 0.0; }
        (1.0 - self.rifle_cd / self.weapon.reload_time()).clamp(0.0, 1.0) as f32
    }
}

fn reflect(v: Vec3, n: Vec3) -> Vec3 { v - 2.0 * v.dot(n) * n }

fn slerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let dot = a.dot(b).clamp(-1.0, 1.0);
    let theta = dot.acos() * t;
    let rel = (b - a * dot).normalize_or_zero();
    a * theta.cos() + rel * theta.sin()
}

/// Ray against an ellipsoid with semi-axes `radii` in a frame turned by
/// `yaw` about +Y. A ray starting inside reports nothing (as Godot).
pub fn ray_ellipsoid(from: Vec3, dir: Vec3, center: Vec3, radii: Vec3, yaw: f32, max: f32) -> Option<(f32, Vec3)> {
    let inv = Quat::from_rotation_y(-yaw);
    let o = (inv * (from - center)) / radii;
    let d = (inv * dir) / radii;
    let a = d.length_squared();
    if a < 1e-12 { return None; }
    let b = o.dot(d);
    let c = o.length_squared() - 1.0;
    if c <= 0.0 { return None; }
    let disc = b * b - a * c;
    if disc < 0.0 { return None; }
    let t = (-b - disc.sqrt()) / a;
    if t < 0.0 || t > max { return None; }
    let local = o + d * t;
    let n = (Quat::from_rotation_y(yaw) * (local / radii)).normalize_or(Vec3::Y);
    Some((t, n))
}
