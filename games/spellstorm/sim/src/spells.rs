//! Spells let loose in the world (src/fx in the Godot game): staff bolts,
//! thrown solids, sprays and beams held from the staff, novas, thunder,
//! traps and sentries planted at your feet, and the cataclysms. Each one in
//! flight is an entity; a nova or a strike lands at once.

use crate::balance::{self as b, Flight};
use crate::fx::{self, Ev};
use crate::grammar::{Numbers, STEP};
use crate::rng::Rand;
use crate::world::{Id, MONSTERS, WIZARDS, World, flat, mix_of, one, signed_angle, yaw_rot};
use crate::{El, Shape, Spell};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// Lobbed bolts fall this hard, landing at their range.
pub const GRAVITY: f32 = 12.0;

/// What one bolt carries: every loaded element, how many of each, and how
/// hard each hits (Balance.bolt_scales).
#[derive(Clone, Debug, Default)]
pub struct BoltSpell {
    pub counts: Vec<(El, u8)>,
    pub primary: Option<El>,
    pub scales: Vec<(El, f64)>,
}

impl BoltSpell {
    pub fn scale(&self, e: El) -> f64 {
        self.scales
            .iter()
            .find(|s| s.0 == e)
            .map(|s| s.1)
            .unwrap_or(0.0)
    }

    pub fn has(&self, e: El) -> bool {
        self.counts.iter().any(|c| c.0 == e)
    }
}

/// The base elements of an enchantment, in the order they first appear.
pub fn bases(counts: &[(El, u8)]) -> Vec<El> {
    let mut out = Vec::new();
    for &(e, _) in counts {
        for p in e.parts() {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// One staff bolt (bolt.gd). It flies through friends.
#[derive(Clone, Debug)]
pub struct Bolt {
    pub caster: Id,
    pub spell: BoltSpell,
    /// The element it looks like, None for a plain golden bolt.
    pub el: Option<El>,
    pub pos: Vec3,
    pub vel: Vec3,
    pub t: f64,
    pub flight: Flight,
    pub life: f64,
    /// The base elements of the enchantment it was fired with, if any.
    pub charmed: Vec<El>,
    pierced: Vec<Id>,
    /// This bolt's part of an enchanted volley.
    pub share: f64,
    /// Only the middle bolt of a volley leaves a pool of poison.
    pub lead: bool,
    /// How big it looks: a plain spark small, a full queue a fireball.
    pub size: f32,
}

impl Bolt {
    /// BoltFx.setup and _ready: flies from `at` along `dir`.
    pub fn new(
        caster: Id,
        spell: BoltSpell,
        dir: Vec3,
        at: Vec3,
        charmed: Vec<El>,
        share: f64,
        lead: bool,
    ) -> Bolt {
        let el = spell.primary;
        let flight = b::flight(el);
        let mut vel = dir.normalize() * flight.speed as f32;
        let mut life = flight.range / flight.speed;
        let loaded: u32 = spell.counts.iter().map(|c| c.1 as u32).sum();
        let heavy = ((6.0 / flight.rate).powf(0.3)).clamp(0.85, 1.6) as f32;
        let mut size = if loaded == 0 {
            0.45
        } else {
            0.6 + 0.13 * (loaded - 1) as f32
        };
        size *= heavy;
        if !charmed.is_empty() {
            size = 1.1 * heavy;
            if charmed.contains(&El::Air) {
                vel *= 1.5;
                life /= 1.5;
            } else if charmed.contains(&El::Earth) {
                vel *= 0.8;
                life /= 0.8;
            }
        }
        if flight.lob {
            // Thrown up just enough to come down at the end of its range.
            vel.y = ((GRAVITY as f64 * life * life * 0.5 - 1.1) / life) as f32;
        }
        Bolt {
            caster,
            spell,
            el,
            pos: Vec3::new(at.x, 1.1, at.z),
            vel,
            t: 0.0,
            flight,
            life,
            charmed,
            pierced: Vec::new(),
            share,
            lead,
            size,
        }
    }
}

/// The colour of a bolt's trail (BoltFx.LOOK).
pub fn bolt_trail(el: Option<El>) -> fx::C {
    use El::*;
    match el {
        Some(Fire) => fx::rgb(1.0, 0.45, 0.08),
        Some(Frost) => fx::rgb(0.85, 0.95, 1.0),
        Some(Water) => fx::rgb(0.3, 0.55, 1.0),
        Some(Lightning) => fx::rgb(0.8, 0.75, 1.0),
        Some(Earth) => fx::rgb(0.55, 0.45, 0.32),
        Some(Air) => fx::rgba(0.8, 0.95, 0.9, 0.6),
        Some(Poison) => fx::rgb(0.45, 0.75, 0.1),
        Some(Void) => fx::rgb(0.55, 0.15, 0.85),
        Some(Steam) => fx::rgba(0.9, 0.92, 0.95, 0.6),
        Some(Ice) => fx::rgb(0.75, 0.92, 1.0),
        Some(Mud) => fx::rgb(0.4, 0.3, 0.18),
        Some(Magma) => fx::rgb(1.0, 0.35, 0.05),
        Some(Storm) => fx::rgb(0.6, 0.7, 1.0),
        None => fx::rgb(1.0, 0.85, 0.45),
    }
}

/// A thrown solid: boulder, ice shard, mud ball, lava bomb (projectile.gd).
#[derive(Clone, Debug)]
pub struct Projectile {
    pub caster: Id,
    pub spell: Spell,
    pub pos: Vec3,
    pub vel: Vec3,
    pub power: f64,
    pub radius: f64,
    pub pierce: i32,
    pub t: f64,
    hit: Vec<Id>,
}

/// A stream from the staff (spray or beam), so crossing streams can meet.
#[derive(Clone, Debug)]
pub struct Stream {
    pub caster: Id,
    pub from: Vec3,
    pub to: Vec3,
    pub slots: Vec<El>,
    pub running: bool,
}

/// A cone of fire, frost, water, steam or air (spray.gd).
#[derive(Clone, Debug)]
pub struct Spray {
    pub caster: Id,
    pub spell: Spell,
    pub t: f64,
    touch_t: f64,
    /// Where the staff is and which way the cone points, for the app.
    pub at: Vec3,
    pub dir: Vec3,
}

/// A beam that stops at the first thing it touches (beam.gd).
#[derive(Clone, Debug)]
pub struct Beam {
    pub caster: Id,
    pub spell: Spell,
    pub t: f64,
    touch_t: f64,
    pub from: Vec3,
    pub to: Vec3,
    pub first: Option<Id>,
}

/// A trap at your feet: arms, then goes off when a monster comes close.
#[derive(Clone, Debug)]
pub struct Trap {
    pub caster: Id,
    pub spell: Spell,
    pub pos: Vec3,
    pub t: f64,
    pull: f64,
    pub done: bool,
}

pub const TRAP_ARM: f64 = 0.5;
pub const TRAP_PULL: f64 = 0.6;

impl Trap {
    /// How close a monster has to come: the ring you see.
    pub fn trigger_r(&self) -> f64 {
        1.0 + 0.1 * self.spell.slots.len() as f64 * STEP
    }

    pub fn enemy_team(&self, caster_team: u8) -> u8 {
        if caster_team == WIZARDS {
            MONSTERS
        } else {
            WIZARDS
        }
    }
}

/// A sentry that shoots bolts of your whole mix at the nearest monster.
#[derive(Clone, Debug)]
pub struct Sentry {
    pub caster: Id,
    pub spell: Spell,
    pub pos: Vec3,
    pub t: f64,
    shot_t: f64,
    pub head_yaw: f32,
}

pub const OVERLOAD_REACH: f32 = 7.0;
pub const WAVE_WIDTH: f32 = 7.2;
pub const WAVE_START: f32 = 1.5;
// Slow enough to watch it roll and carry monsters along (Joep).
pub const WAVE_SPEED: f32 = 8.0;
pub const WAVE_TIME: f64 = 1.8;
pub const WAVE_RUN: f32 = WAVE_SPEED * WAVE_TIME as f32;
pub const METEOR_FROM: Vec3 = Vec3::new(4.0, 22.0, -4.0);
pub const STORM_TIME: f64 = 3.2;
pub const STORM_EVERY: f64 = 0.22;

/// The venom cannon (three poison cast): globs of venom hosed out of the
/// staff for this long, one every `VENOM_EVERY`, landing between `VENOM_NEAR`
/// and `VENOM_FAR` ahead, within `VENOM_CONE` radians either side of the aim.
pub const VENOM_TIME: f64 = 2.4;
pub const VENOM_EVERY: f64 = 0.05;
pub const VENOM_NEAR: f32 = 2.5;
pub const VENOM_FAR: f32 = 10.0;
pub const VENOM_CONE: f32 = 0.38;
/// The boulder (three earth cast): it grows over the caster's head, is
/// thrown, then rolls `BOULDER_SPEED` m/s for `BOULDER_ROLL` seconds,
/// `BOULDER_R` metres round, crushing what it runs over and shoving the rest
/// aside.
pub const BOULDER_GROW: f64 = 0.8;
pub const BOULDER_THROW: f64 = 0.3;
pub const BOULDER_ROLL: f64 = 2.2;
pub const BOULDER_SPEED: f32 = 10.0;
pub const BOULDER_R: f32 = 1.5;
/// Where the boulder lands from the caster, and how far it rolls on.
pub const BOULDER_LAND: f32 = 3.0;
pub const BOULDER_RUN: f32 = BOULDER_SPEED * BOULDER_ROLL as f32;

/// How far round its spot each cataclysm reaches, for the charge mark.
pub fn overload_reach(e: Option<El>) -> f32 {
    match e {
        Some(El::Fire) => 5.0,
        Some(El::Frost) => 5.5,
        Some(El::Lightning) => 5.5,
        Some(El::Earth) => 8.5,
        Some(El::Air) => 2.4,
        Some(El::Poison) => 4.0,
        Some(El::Void) => 4.5,
        _ => 4.5,
    }
}

/// A full queue cast ahead: a cataclysm (overload.gd).
#[derive(Clone, Debug)]
pub struct Overload {
    pub caster: Id,
    pub spell: Spell,
    pub target: Vec3,
    pub t: f64,
    pub riders: Vec<El>,
    strikes: u32,
    pub field: Option<Id>,
    ride_t: f64,
    /// The meteor falling, the wave rolling or the void's disc.
    pub thing: Option<Vec3>,
    pub wave_dir: Vec3,
    /// The thunderstorm's cloud.
    pub cloud: Option<Vec3>,
    end_at: f64,
    /// The boulder's crushed, each crushed once; the venom's next glob.
    crushed: Vec<Id>,
    next_t: f64,
}

/// What a nova does: radius and strength.
#[derive(Clone, Copy, Debug)]
pub struct NovaN {
    pub radius: f64,
    pub rate: f64,
}

impl World {
    pub fn ff(&self) -> f64 {
        self.settings.ff
    }

    /// Every base element of a spell touches the ground there, so fire on a
    /// puddle makes steam and lightning on it electrifies it.
    pub fn touch(&mut self, counts: &[(El, u8)], at: Vec3, r: f64, caster: Option<Id>) {
        for &(e, _) in counts {
            self.touch_ground(at, r, e, caster);
        }
    }

    /// Land a bolt's mix on `a`, each element at its own strength times
    /// `mult` (BoltFx.strike). Friends are never hurt by it.
    pub fn strike(&mut self, a: Id, s: &BoltSpell, mult: f64, src: Id, dir: Vec3) {
        if self.team(a) == self.team(src) {
            return;
        }
        if s.counts.is_empty() {
            self.hit(
                a,
                &one(El::Earth),
                b::PLAIN_BOLT * mult,
                Some(src),
                dir,
                1.0,
            );
            return;
        }
        for &(e, _) in &s.counts {
            self.hit(a, &one(e), s.scale(e) * mult, Some(src), dir, 1.0);
        }
    }

    pub fn fx_step(&mut self, id: Id, dt: f64) {
        if self.ecs.satisfies::<&Bolt>(id).unwrap_or(false) {
            self.bolt_step(id, dt);
        } else if self.ecs.satisfies::<&Projectile>(id).unwrap_or(false) {
            self.projectile_step(id, dt);
        } else if self.ecs.satisfies::<&Spray>(id).unwrap_or(false) {
            self.spray_step(id, dt);
        } else if self.ecs.satisfies::<&Beam>(id).unwrap_or(false) {
            self.beam_step(id, dt);
        } else if self.ecs.satisfies::<&Trap>(id).unwrap_or(false) {
            self.trap_step(id, dt);
        } else if self.ecs.satisfies::<&Sentry>(id).unwrap_or(false) {
            self.sentry_step(id, dt);
        } else if self.ecs.satisfies::<&Overload>(id).unwrap_or(false) {
            self.overload_step(id, dt);
        } else {
            self.pickup_step(id, dt);
        }
    }

    fn get<T: hecs::Component + Clone>(&self, id: Id) -> T {
        (*self.ecs.get::<&T>(id).unwrap()).clone()
    }

    fn put<T: hecs::Component>(&mut self, id: Id, v: T) {
        if let Ok(mut c) = self.ecs.get::<&mut T>(id) {
            *c = v;
        }
    }

    fn gone(&mut self, id: Id) {
        let _ = self.ecs.despawn(id);
    }

    // --- Bolts -----------------------------------------------------------

    pub fn fire_bolt(&mut self, bolt: Bolt) -> Id {
        self.spawn_fx(bolt)
    }

    fn bolt_step(&mut self, id: Id, dt: f64) {
        let mut bo: Bolt = self.get(id);
        if bo.flight.zap {
            self.spark(&bo);
            self.gone(id);
            return;
        }
        bo.t += dt;
        if bo.t > bo.life {
            self.bolt_end(&bo, bo.pos);
            self.gone(id);
            return;
        }
        let from = bo.pos;
        if bo.flight.lob {
            bo.vel.y -= GRAVITY * dt as f32;
        }
        let to = from + bo.vel * dt as f32;
        if to.y <= 0.05 {
            bo.pos = to;
            self.bolt_end(&bo, Vec3::new(to.x, 0.05, to.z));
            self.gone(id);
            return;
        }
        if let Some((at, prop)) = self.ray(from, to) {
            if let Some(p) = prop {
                self.damage_prop(p, 2.0, bo.vel, None);
            }
            self.bolt_end(&bo, at);
            self.gone(id);
            return;
        }
        bo.pos = to;
        let reach = bo.flight.hit * if bo.charmed.is_empty() { 1.0 } else { 1.5 };
        let caster_team = self.team(bo.caster);
        for a in self.living() {
            if a == bo.caster || bo.pierced.contains(&a) {
                continue;
            }
            let (alive, team, touches) = {
                let ac = self.a(a);
                (ac.alive, ac.team, ac.hit_by(from, to, reach))
            };
            if !alive || team == caster_team || !touches {
                continue;
            }
            let dir = bo.vel.normalize();
            self.strike(a, &bo.spell, 1.0, bo.caster, dir);
            let n = (3.0 + 6.0 * bo.size) as u32;
            self.burst(
                to,
                bolt_trail(bo.el),
                n,
                2.0 + 2.0 * bo.size,
                0.12 * bo.size + 0.04,
                bo.el,
            );
            if !bo.charmed.is_empty() {
                self.charm_hit(&bo, a, to);
                // A frost shard goes on through, up to three.
                if bo.charmed.contains(&El::Frost) && bo.pierced.len() < 2 {
                    bo.pierced.push(a);
                    continue;
                }
            }
            self.bolt_end(&bo, to);
            self.gone(id);
            return;
        }
        self.put(id, bo);
    }

    /// What an enchanted bolt's elements do where it hits `a`.
    fn charm_hit(&mut self, bo: &Bolt, a: Id, at: Vec3) {
        let dir = bo.vel.normalize();
        let mass = self.a(a).mass as f32;
        {
            let mut ac = self.am(a);
            if bo.charmed.contains(&El::Earth) {
                ac.stun = ac.stun.max(0.35);
                ac.push += dir * 7.0 / mass;
            }
            if bo.charmed.contains(&El::Air) {
                ac.push += dir * 12.0 / mass;
            }
            if bo.charmed.contains(&El::Water) {
                ac.push += dir * 4.0 / mass;
            }
        }
        if bo.charmed.contains(&El::Lightning) {
            // The hit jumps on to the two nearest.
            let mut from = at;
            let mut done = vec![a];
            for _ in 0..2 {
                let mut best = None;
                let mut best_d = 4.0;
                for e in self.living_enemies() {
                    if done.contains(&e) {
                        continue;
                    }
                    let d = self.pos(e).distance(from);
                    if d < best_d {
                        best = Some(e);
                        best_d = d;
                    }
                }
                let Some(best) = best else { break };
                let p = self.pos(best) + Vec3::Y * 0.9;
                let s = b::bolt_scale(El::Lightning) * 1.2 * bo.share;
                self.hit(
                    best,
                    &one(El::Lightning),
                    s,
                    Some(bo.caster),
                    (p - from).normalize_or_zero(),
                    1.0,
                );
                self.zap(from, p);
                done.push(best);
                from = p;
            }
        }
    }

    /// A quick jagged flash for a jumping hit (BoltFx._zap).
    pub fn zap(&mut self, a: Vec3, b: Vec3) {
        let c = fx::el_c(El::Lightning);
        self.ev(Ev::Zap {
            a,
            b,
            c,
            width: 0.18,
            time: 0.1,
        });
        // No halo at `b`: the struck body flashes white itself.
    }

    /// A lightning bolt does not fly: a spark jumps at once from the staff
    /// to the first monster in line, or a wall, or dies at its range.
    fn spark(&mut self, bo: &Bolt) {
        let from = bo.pos;
        let dir = flat(bo.vel).normalize();
        let mut reach = bo.flight.range as f32;
        let mut hit = None;
        let caster_team = self.team(bo.caster);
        for a in self.living() {
            if a == bo.caster {
                continue;
            }
            let ac = self.a(a);
            if !ac.alive || ac.team == caster_team {
                continue;
            }
            let to = flat(ac.pos - from);
            let along = to.dot(dir);
            if along > 0.0
                && along < reach
                && (to - dir * along).length() < ac.radius + bo.flight.hit as f32 + 0.35
            {
                reach = along;
                hit = Some(a);
            }
        }
        let mut to_pt = from + dir * reach;
        if let Some((at, prop)) = self.ray(from, to_pt) {
            hit = None;
            to_pt = at;
            if let Some(p) = prop {
                self.damage_prop(p, 2.0, dir, None);
            }
        }
        if let Some(h) = hit {
            to_pt = self.pos(h) + Vec3::Y * 0.9;
            self.strike(h, &bo.spell, 1.0, bo.caster, dir);
            if !bo.charmed.is_empty() {
                self.charm_hit(bo, h, to_pt);
            }
        }
        let c = bo.el.map(fx::el_c).unwrap_or(fx::WHITE);
        self.ev(Ev::Zap {
            a: from,
            b: to_pt,
            c,
            width: 0.05 + 0.03 * bo.size,
            time: 0.06,
        });
        self.bolt_end(bo, to_pt);
    }

    fn bolt_end(&mut self, bo: &Bolt, at: Vec3) {
        self.ev(Ev::Impact {
            at,
            el: bo.el,
            size: bo.size as f32,
        });
        if !bo.charmed.is_empty() {
            self.charm_end(bo, at);
        }
        // A bolt that lands still touches the ground: fire bolts boil puddles.
        let counts = bo.spell.counts.clone();
        self.touch(&counts, flat(at), 0.5, Some(bo.caster));
    }

    fn charm_end(&mut self, bo: &Bolt, at: Vec3) {
        let ground = flat(at);
        if bo.charmed.contains(&El::Fire) {
            // Each fireball bursts: a small blast that burns whoever stands
            // by. Steam only hisses (Joep: far too big for so small a spell).
            let fire = bo.spell.has(El::Fire) || bo.spell.has(El::Magma);
            if fire {
                self.ev(Ev::Boom {
                    at: ground,
                    r: 0.5,
                    mark: false,
                });
            } else {
                self.burst(
                    ground + Vec3::Y * 0.4,
                    fx::el_c(El::Steam),
                    6,
                    1.6,
                    0.18,
                    Some(El::Steam),
                );
            }
            let burst = self.burst_fire(bo);
            for a in self.living_enemies() {
                let d = flat(self.pos(a) - ground);
                if d.length() < 1.8 + self.a(a).radius {
                    self.hit(
                        a,
                        &one(El::Fire),
                        burst,
                        Some(bo.caster),
                        d.normalize_or_zero(),
                        1.0,
                    );
                }
            }
            if fire {
                self.shake(0.06);
            }
        }
        if bo.charmed.contains(&El::Void) {
            for a in self.living_enemies() {
                let d = flat(ground - self.pos(a));
                if d.length() < 3.0 && d.length() > 0.3 {
                    let mut ac = self.am(a);
                    let m = ac.mass as f32;
                    ac.push += d.normalize() * 8.0 / m;
                }
            }
            self.burst(at, fx::el_c(El::Void), 14, -4.0, 0.25, Some(El::Void));
        }
        if bo.charmed.contains(&El::Poison) && bo.lead {
            let at2 = self.level.clamp_inside(ground);
            let f = self.add_field(at2, El::Poison, 0.9, 2.5, Some(bo.caster));
            self.field_mut(f).friendly = true;
            self.field_mut(f).splash = true;
        }
        // Every element lands with something you can see.
        for &e in &bo.charmed.clone() {
            self.impact(at, ground, e);
        }
    }

    fn impact(&mut self, at: Vec3, ground: Vec3, e: El) {
        let c = fx::el_c(e);
        let fountain = |w: &mut World, n: u32, speed: f32, spread: f32, size: f32| {
            w.ev(Ev::Fountain {
                at: at + Vec3::Y * 0.1,
                el: e,
                n,
                speed,
                spread,
                size,
            });
        };
        match e {
            El::Water => {
                fountain(self, 18, 7.0, 28.0, 0.75);
                self.ev(Ev::Ripple {
                    at: ground,
                    r: 2.2,
                    time: 0.6,
                });
            }
            El::Poison => fountain(self, 14, 5.0, 40.0, 0.7),
            El::Frost => {
                fountain(self, 12, 5.5, 75.0, 0.7);
                self.ev(Ev::Puff {
                    at,
                    c: fx::mix(c, fx::WHITE, 0.6),
                    size: 0.45,
                });
            }
            El::Earth => {
                fountain(self, 10, 6.5, 50.0, 0.9);
                self.ev(Ev::Puff {
                    at: ground + Vec3::Y * 0.2,
                    c: fx::rgb(0.45, 0.38, 0.3),
                    size: 0.9,
                });
            }
            El::Air => {
                self.ev(Ev::Ring {
                    at: ground,
                    c: fx::mix(c, fx::WHITE, 0.5),
                    r: 1.8,
                    time: 0.3,
                });
                fountain(self, 10, 5.0, 90.0, 0.7);
            }
            El::Lightning => {
                fountain(self, 8, 4.0, 90.0, 0.5);
                self.ev(Ev::LightPop {
                    at: ground,
                    c,
                    energy: 2.0,
                    range: 4.0,
                    time: 0.15,
                });
            }
            El::Void => self.ev(Ev::LightPop {
                at: ground,
                c,
                energy: 1.5,
                range: 3.5,
                time: 0.25,
            }),
            _ => {}
        }
    }

    /// How hard an enchanted fire bolt's burst hits.
    fn burst_fire(&self, bo: &Bolt) -> f64 {
        let s = &bo.spell;
        let f = if s.scales.iter().any(|x| x.0 == El::Fire) {
            s.scale(El::Fire)
        } else if s.scales.iter().any(|x| x.0 == El::Magma) {
            s.scale(El::Magma)
        } else {
            s.scale(El::Steam)
        };
        f.max(b::bolt_scale(El::Fire) * bo.share) * 0.5
    }

    // --- Thrown solids ---------------------------------------------------

    /// ProjectileFx.launch: always a full throw.
    pub fn throw(&mut self, caster: Id, s: &Spell, dir: Vec3, at: Vec3) {
        let charge = 1.0;
        let n = &s.n;
        let p = Projectile {
            caster,
            spell: s.clone(),
            pos: Vec3::new(at.x, 1.1, at.z),
            vel: dir * (n.speed * (0.8 + 0.6 * charge)) as f32,
            power: 0.55 + 1.2 * charge,
            radius: n.radius * (0.8 + 0.5 * charge),
            pierce: if n.pierce { 2 } else { 0 },
            t: 0.0,
            hit: Vec::new(),
        };
        self.spawn_fx(p);
    }

    fn projectile_step(&mut self, id: Id, dt: f64) {
        let mut p: Projectile = self.get(id);
        p.t += dt;
        let from = p.pos;
        let to = from + p.vel * dt as f32;
        if let Some((at, prop)) = self.ray(from, to) {
            if let Some(i) = prop {
                self.damage_prop(i, (p.power * 20.0) as f32, p.vel, None);
            }
            self.land(&p, at);
            self.gone(id);
            return;
        }
        p.pos = to;
        let ff = self.ff();
        for a in self.living() {
            if p.hit.contains(&a) || (a == p.caster && p.t < 0.25) {
                continue;
            }
            let touches = {
                let ac = self.a(a);
                ac.alive && ac.hit_by(from, to, p.radius)
            };
            if !touches {
                continue;
            }
            p.hit.push(a);
            let mix = mix_of(&p.spell.counts);
            self.hit(
                a,
                &mix,
                p.power * p.spell.n.rate,
                Some(p.caster),
                p.vel.normalize(),
                ff,
            );
            self.burst(to, color(&p.spell), 10, 4.0, 0.25, p.spell.primary);
            if p.pierce <= 0 {
                self.land(&p, to);
                self.gone(id);
                return;
            }
            p.pierce -= 1;
        }
        if p.t > 2.5 {
            self.land(&p, to);
            self.gone(id);
            return;
        }
        self.put(id, p);
    }

    fn land(&mut self, p: &Projectile, at: Vec3) {
        let c = color(&p.spell);
        self.burst(at, c, 26, 6.0, 0.35, p.spell.primary);
        self.ev(Ev::LightPop {
            at,
            c,
            energy: 2.5,
            range: 5.0,
            time: 0.35,
        });
        let splash = p.spell.n.splash;
        let ground = flat(at);
        self.ev(Ev::Land {
            at: ground,
            r: (splash * (0.8 + 0.3 * p.power)).max(0.8) as f32,
            el: p.spell.primary,
            payload: p.spell.payload.clone(),
        });
        if splash > 0.0 {
            let r = splash * (0.8 + 0.3 * p.power);
            self.ev(Ev::Ring {
                at: ground,
                c,
                r: r as f32,
                time: 0.35,
            });
            self.blast(at, r as f32, (10.0 * p.power) as f32);
            let ff = self.ff();
            let mix = mix_of(&p.spell.counts);
            for a in self.living() {
                if p.hit.contains(&a) {
                    continue;
                }
                let ap = self.pos(a);
                let d = Vec2::new(ap.x - at.x, ap.z - at.z);
                if (d.length() as f64) < r + self.a(a).radius as f64 {
                    let dir = Vec3::new(d.x, 0.0, d.y).normalize_or_zero();
                    self.hit(
                        a,
                        &mix,
                        p.power * 0.6 * p.spell.n.rate,
                        Some(p.caster),
                        dir,
                        ff,
                    );
                }
            }
            let kind = if p.spell.primary == Some(El::Magma) {
                El::Fire
            } else {
                El::Mud
            };
            let f = self.add_field(ground, kind, r * 0.7, 6.0, Some(p.caster));
            self.field_mut(f).splash = true;
            self.touch(&p.spell.counts.clone(), at, r, Some(p.caster));
        } else {
            self.touch(&p.spell.counts.clone(), at, 1.2, Some(p.caster));
        }
    }

    // --- Streams -----------------------------------------------------------

    pub fn start_spray(&mut self, caster: Id, s: &Spell, at: Vec3) -> Id {
        let dir = self.wiz(caster).aim_dir;
        let o = self.order();
        let id = self.ecs.spawn((
            o,
            Spray {
                caster,
                spell: s.clone(),
                t: 0.0,
                touch_t: 0.0,
                at,
                dir,
            },
            Stream {
                caster,
                from: at,
                to: at,
                slots: s.slots.clone(),
                running: true,
            },
        ));
        self.fx.push(id);
        id
    }

    pub fn start_beam(&mut self, caster: Id, s: &Spell, at: Vec3) -> Id {
        let o = self.order();
        let id = self.ecs.spawn((
            o,
            Beam {
                caster,
                spell: s.clone(),
                t: 0.0,
                touch_t: 0.0,
                from: at,
                to: at,
                first: None,
            },
            Stream {
                caster,
                from: at,
                to: at,
                slots: s.slots.clone(),
                running: true,
            },
        ));
        self.fx.push(id);
        id
    }

    /// A stream runs its whole time once started; tapping RT again or
    /// rolling stops it early.
    fn holding(&self, caster: Id, id: Id) -> bool {
        self.ecs
            .get::<&crate::wizard::Wizard>(caster)
            .is_ok_and(|w| w.channel == Some(id))
            && self.alive(caster)
    }

    fn spray_step(&mut self, id: Id, dt: f64) {
        let mut s: Spray = self.get(id);
        s.t += dt;
        let n = s.spell.n.clone();
        if !self.alive(s.caster) || !self.holding(s.caster, id) || s.t > n.duration {
            self.gone(id);
            return;
        }
        if !self.a(s.caster).can_act() {
            self.put(id, s);
            return;
        }
        let dir = self.wiz(s.caster).aim_dir;
        s.at = self.staff_tip(s.caster);
        s.dir = dir;
        let cp = self.pos(s.caster);
        {
            let mut st = self.ecs.get::<&mut Stream>(id).unwrap();
            st.from = cp;
            st.to = cp + dir * n.range as f32;
        }
        let half = (n.angle * 0.5).to_radians() as f32;
        let ff = self.ff();
        let mix = mix_of(&s.spell.counts);
        for a in self.living() {
            if a == s.caster {
                continue;
            }
            let (ap, ar) = {
                let ac = self.a(a);
                (ac.pos, ac.radius)
            };
            let to = flat(ap - cp);
            let d = to.length();
            if d > n.range as f32 + ar || d < 0.01 {
                continue;
            }
            if signed_angle(dir, to).abs() > half + ar.atan2(d) {
                continue;
            }
            let falloff = 1.0 - 0.4 * d as f64 / n.range;
            self.hit(
                a,
                &mix,
                dt * n.rate * falloff,
                Some(s.caster),
                to.normalize(),
                ff,
            );
        }
        s.touch_t -= dt;
        if s.touch_t <= 0.0 {
            s.touch_t = 0.25;
            let at = cp + dir * (n.range * 0.75) as f32;
            self.touch(
                &s.spell.counts.clone(),
                at,
                1.2 + n.range * 0.15,
                Some(s.caster),
            );
        }
        self.put(id, s);
    }

    fn beam_step(&mut self, id: Id, dt: f64) {
        let mut bm: Beam = self.get(id);
        bm.t += dt;
        let n = bm.spell.n.clone();
        if !self.alive(bm.caster) || !self.holding(bm.caster, id) || bm.t > n.duration {
            self.gone(id);
            return;
        }
        let dir = self.wiz(bm.caster).aim_dir;
        let from = self.staff_tip(bm.caster);
        let mut length = n.range as f32;
        if let Some((at, prop)) = self.ray(from, from + dir * length) {
            length = from.distance(at);
            if let Some(p) = prop {
                self.damage_prop(p, (12.0 * dt) as f32, dir, None);
            }
        }
        let mut first = None;
        let cp = self.pos(bm.caster);
        let can = self.a(bm.caster).can_act();
        if can {
            for a in self.living() {
                if a == bm.caster {
                    continue;
                }
                let ac = self.a(a);
                if !ac.alive {
                    continue;
                }
                let to = flat(ac.pos - cp);
                let along = to.dot(dir);
                if !(0.0..=length).contains(&along) {
                    continue;
                }
                if (to - dir * along).length() < ac.radius + n.width as f32 {
                    length = along;
                    first = Some(a);
                }
            }
        }
        let mut to_pos = from + dir * length;
        to_pos.y = from.y;
        {
            let mut st = self.ecs.get::<&mut Stream>(id).unwrap();
            st.from = cp;
            st.to = cp + dir * length;
        }
        bm.from = from;
        bm.to = to_pos;
        bm.first = first;
        if let Some(f) = first {
            if can {
                let mix = mix_of(&bm.spell.counts);
                let ff = self.ff();
                self.hit(f, &mix, dt * n.rate, Some(bm.caster), dir, ff);
            }
        }
        bm.touch_t -= dt;
        if bm.touch_t <= 0.0 {
            bm.touch_t = 0.3;
            self.touch(&bm.spell.counts.clone(), flat(to_pos), 0.8, Some(bm.caster));
        }
        self.put(id, bm);
    }

    // --- Bursts ------------------------------------------------------------

    /// Everything in a spell at once, in a ring round `at` (nova.gd): the
    /// queue's burst, rune mines, confluences, cataclysms, blasts.
    #[allow(clippy::too_many_arguments)]
    pub fn nova(
        &mut self,
        caster: Id,
        s: &Spell,
        mut n: NovaN,
        at: Vec3,
        spare_caster: bool,
        only_team: Option<u8>,
        power: f64,
    ) {
        // Wide Sigil.
        n.radius *= self.a(caster).mods.nova;
        let mix = mix_of(&s.counts);
        self.nova_mix(
            caster,
            &mix,
            s.primary,
            &s.payload,
            n,
            at,
            spare_caster,
            only_team,
            power,
        );
        let counts = s.counts.clone();
        self.touch(&counts, at, n.radius, Some(caster));
        self.blast(at, n.radius as f32, (8.0 * power * n.rate) as f32);
    }

    /// A nova of a plain mix (a monster's burst, a blow-up): no ground touch
    /// beyond its own elements.
    pub fn burst_nova(&mut self, caster: Id, e: El, n: NovaN, at: Vec3, only_team: Option<u8>) {
        let mix = one(e);
        self.nova_mix(caster, &mix, Some(e), &[], n, at, true, only_team, 1.0);
        self.touch_ground(at, n.radius, e, Some(caster));
        self.blast(at, n.radius as f32, (8.0 * n.rate) as f32);
    }

    #[allow(clippy::too_many_arguments)]
    fn nova_mix(
        &mut self,
        caster: Id,
        mix: &[(El, f64)],
        primary: Option<El>,
        payload: &[El],
        n: NovaN,
        at: Vec3,
        spare_caster: bool,
        only_team: Option<u8>,
        power: f64,
    ) {
        let radius = n.radius;
        let mut cs = vec![primary.map(fx::el_c).unwrap_or(fx::WHITE)];
        cs.extend(payload.iter().map(|&e| fx::el_c(e)));
        self.ev(Ev::Nova {
            at,
            r: radius as f32,
            cs,
            el: primary,
            payload: payload.to_vec(),
        });
        let ff = self.ff();
        for a in self.living() {
            if spare_caster && a == caster {
                continue;
            }
            let (team, ap, ar) = {
                let ac = self.a(a);
                (ac.team, ac.pos, ac.radius)
            };
            if only_team.is_some_and(|t| t != team) {
                continue;
            }
            let d = flat(ap - at);
            if (d.length() as f64) < radius + ar as f64 {
                let dir = if d.length() > 0.05 {
                    d.normalize()
                } else {
                    yaw_rot(Vec3::X, self.rng.f() * TAU)
                };
                let k = power * n.rate * (1.0 - 0.35 * d.length() as f64 / radius);
                self.hit(a, mix, k, Some(caster), dir, ff);
            }
        }
    }

    /// Lightning's RT spell: one thunderbolt out of the sky (strike.gd).
    pub fn thunder(&mut self, caster: Id, s: &Spell, at: Vec3) {
        let n = &s.n;
        let mut at = flat(at);
        let caster_team = self.team(caster);
        let mut struck = None;
        let mut best = n.radius as f32 + 0.6;
        for a in self.living() {
            if a == caster {
                continue;
            }
            let ac = self.a(a);
            if !ac.alive || ac.team == caster_team {
                continue;
            }
            let d = Vec2::new(ac.pos.x - at.x, ac.pos.z - at.z).length();
            if d < best {
                best = d;
                struck = Some(a);
            }
        }
        if let Some(s) = struck {
            at = flat(self.pos(s));
        }
        let top = at + Vec3::new(self.rng.range(-1.5, 1.5), 16.0, self.rng.range(-1.0, 1.0));
        let ff = self.ff();
        let mix = mix_of(&s.counts);
        for a in self.living() {
            if a == caster {
                continue;
            }
            let (alive, team, wet, ap, ar) = {
                let ac = self.a(a);
                (ac.alive, ac.team, ac.wet, ac.pos, ac.radius)
            };
            if !alive || (team == caster_team && wet <= 0.0) {
                continue;
            }
            let to = flat(ap - at);
            if Some(a) != struck && to.length() > n.radius as f32 + ar {
                continue;
            }
            let k = n.rate * if Some(a) == struck { 1.0 } else { 0.6 };
            let dir = if to.length() > 0.01 {
                to.normalize()
            } else {
                Vec3::NEG_Z
            };
            self.hit(a, &mix, k, Some(caster), dir, ff);
        }
        let counts = s.counts.clone();
        self.touch(&counts, at, n.radius, Some(caster));
        // A burning bolt sets where it strikes on fire (Joep): a patch of
        // flames the caster's side can walk through.
        let fire = counts.iter().find(|c| c.0 == El::Fire).map_or(0, |c| c.1);
        if fire > 0 && s.primary != Some(El::Fire) {
            let p = self.level.clamp_inside(at);
            let f = self.add_field(p, El::Fire, (n.radius * 0.8).max(1.4) + 0.3 * fire as f64, 4.0 + 1.5 * fire as f64, Some(caster));
            self.field_mut(f).friendly = true;
        }
        let c = color(s);
        let width = 0.32 + 0.05 * s.power as f32 * STEP as f32;
        let riders: Vec<El> = counts.iter().map(|c| c.0).filter(|&e| Some(e) != s.primary).collect();
        self.ev(Ev::Thunder {
            at,
            top,
            c,
            width,
            radius: n.radius as f32,
            riders,
        });
        self.ev(Ev::Flash {
            at: at + Vec3::Y * 0.5,
            c: fx::WHITE,
            size: 3.5 + n.radius as f32,
            time: 0.18,
        });
        self.ev(Ev::Flash {
            at: at + Vec3::Y * 0.3,
            c,
            size: 2.0 + n.radius as f32 * 1.5,
            time: 0.35,
        });
        let lc = fx::mix(c, fx::WHITE, 0.5);
        self.ev(Ev::LightPop {
            at: at + Vec3::Y * 2.0,
            c: lc,
            energy: 9.0,
            range: 10.0 + n.radius as f32 * 2.0,
            time: 0.3,
        });
        // No ring on the floor: a few sparks, and Fx throws up sand (Joep).
        self.burst(at + Vec3::Y * 0.3, c, 12, 9.0, 0.22, s.primary);
        self.shake(0.35);
    }

    // --- Planted -----------------------------------------------------------

    pub fn plant_trap(&mut self, caster: Id, s: &Spell, at: Vec3) -> Id {
        self.spawn_fx(Trap {
            caster,
            spell: s.clone(),
            pos: at,
            t: 0.0,
            pull: -1.0,
            done: false,
        })
    }

    pub fn plant_sentry(&mut self, caster: Id, s: &Spell, at: Vec3) -> Id {
        let c = color(s);
        self.burst(at + Vec3::Y, c, 20, 4.0, 0.35, None);
        self.ev(Ev::Ring {
            at,
            c,
            r: 2.0,
            time: 0.4,
        });
        self.spawn_fx(Sentry {
            caster,
            spell: s.clone(),
            pos: at,
            t: 0.0,
            shot_t: 0.0,
            head_yaw: 0.0,
        })
    }

    fn trap_step(&mut self, id: Id, dt: f64) {
        let mut tr: Trap = self.get(id);
        tr.t += dt;
        if tr.done {
            self.gone(id);
            return;
        }
        let n = tr.spell.n.clone();
        if tr.t > n.duration || !self.ecs.contains(tr.caster) {
            // Gone unsprung: it sinks away without going off.
            self.gone(id);
            return;
        }
        let enemy = tr.enemy_team(self.team(tr.caster));
        let armed = tr.t >= TRAP_ARM;
        if tr.pull >= 0.0 {
            tr.pull -= dt;
            for a in self.living() {
                if self.team(a) != enemy {
                    continue;
                }
                let mut ac = self.am(a);
                let d = flat(tr.pos - ac.pos);
                if (d.length() as f64) < n.radius * 2.0 && d.length() > 0.3 {
                    ac.push = ac.push.lerp(d.normalize() * 9.0, (6.0 * dt) as f32);
                }
            }
            if tr.pull < 0.0 {
                self.trap_blast(&tr, enemy);
                self.gone(id);
                return;
            }
            self.put(id, tr);
            return;
        }
        if !armed {
            self.put(id, tr);
            return;
        }
        for a in self.living() {
            let (team, ap, ar) = {
                let ac = self.a(a);
                (ac.team, ac.pos, ac.radius)
            };
            if team != enemy {
                continue;
            }
            let d = flat(ap - tr.pos);
            if (d.length() as f64) < tr.trigger_r() + ar as f64 {
                if tr.spell.primary == Some(El::Void) {
                    // The mine opens: everything near is dragged in, then it blows.
                    tr.pull = TRAP_PULL;
                    let c = color(&tr.spell);
                    self.ev(Ev::Ring {
                        at: tr.pos,
                        c,
                        r: (n.radius * 2.0) as f32,
                        time: TRAP_PULL as f32,
                    });
                    self.put(id, tr);
                    return;
                }
                self.trap_blast(&tr, enemy);
                self.gone(id);
                return;
            }
        }
        self.put(id, tr);
    }

    fn trap_blast(&mut self, tr: &Trap, enemy: u8) {
        let n = tr.spell.n.clone();
        let at = tr.pos;
        self.nova(
            tr.caster,
            &tr.spell,
            NovaN {
                radius: n.radius,
                rate: n.rate,
            },
            at,
            true,
            Some(enemy),
            1.0,
        );
        let p = tr.spell.primary.unwrap_or(El::Fire);
        match p.kind() {
            crate::el::Kind::Flow => self.trap_spill(tr, p),
            crate::el::Kind::Solid => {
                if p == El::Mud {
                    self.trap_spill(tr, El::Mud);
                } else {
                    // Stone or ice spikes shoot up and pin every monster in reach.
                    for a in self.living() {
                        let mut ac = self.am(a);
                        if ac.team != enemy {
                            continue;
                        }
                        if (flat(ac.pos - at).length() as f64) < n.radius + ac.radius as f64 {
                            ac.stun = ac.stun.max(n.stun);
                        }
                    }
                    self.ev(Ev::Spikes {
                        at,
                        el: p,
                        r: n.radius as f32,
                        time: n.stun as f32,
                    });
                }
            }
            crate::el::Kind::Energy => {
                let c = color(&tr.spell);
                self.ev(Ev::LightPop {
                    at,
                    c,
                    energy: 4.0,
                    range: (n.radius * 3.0) as f32,
                    time: 0.3,
                });
            }
            crate::el::Kind::Spirit => {
                if p == El::Poison {
                    self.trap_spill(tr, El::Poison);
                }
            }
        }
        self.shake(0.25);
    }

    /// Leave a field behind that only hurts monsters.
    fn trap_spill(&mut self, tr: &Trap, el: El) {
        let n = &tr.spell.n;
        let f = self.add_field(tr.pos, el, n.field, n.field_time, Some(tr.caster));
        let counts = mix_of(&tr.spell.counts);
        let mut fl = self.field_mut(f);
        fl.mix = counts;
        fl.rate = 0.35;
        fl.friendly = true;
        fl.keep_mix = true;
    }

    fn sentry_step(&mut self, id: Id, dt: f64) {
        let mut se: Sentry = self.get(id);
        se.t += dt;
        let n = se.spell.n.clone();
        if se.t > n.duration || !self.ecs.contains(se.caster) {
            let head = se.pos + Vec3::Y * 1.65;
            self.burst(head, color(&se.spell), 20, 4.0, 0.35, None);
            self.gone(id);
            return;
        }
        se.shot_t -= dt;
        let caster_team = self.team(se.caster);
        let mut best = None;
        let mut bd = n.range as f32;
        for a in self.living() {
            let ac = self.a(a);
            if ac.team == caster_team {
                continue;
            }
            let d = ac.pos.distance(se.pos);
            if d < bd {
                bd = d;
                best = Some(a);
            }
        }
        let Some(best) = best else {
            se.head_yaw += dt as f32 * 0.8;
            self.put(id, se);
            return;
        };
        let to = flat(self.pos(best) - se.pos);
        se.head_yaw = crate::world::lerp_angle(se.head_yaw, to.x.atan2(to.z), (10.0 * dt) as f32);
        if se.shot_t <= 0.0
            && self
                .ecs
                .satisfies::<&crate::wizard::Wizard>(se.caster)
                .unwrap_or(false)
        {
            se.shot_t = n.interval;
            let scales = se
                .spell
                .counts
                .iter()
                .map(|&(e, c)| (e, n.rate * c as f64))
                .collect();
            let bs = BoltSpell {
                counts: se.spell.counts.clone(),
                primary: se.spell.primary,
                scales,
            };
            let dir = to.normalize();
            let at = se.pos + Vec3::Y * 1.65 + dir * 0.5;
            let bolt = Bolt::new(se.caster, bs, dir, at, Vec::new(), 1.0, true);
            self.fire_bolt(bolt);
        }
        self.put(id, se);
    }

    // --- Cataclysms --------------------------------------------------------

    pub fn overload(&mut self, caster: Id, s: &Spell, target: Vec3) {
        let c = color(s);
        let riders = s.riders.clone();
        let mut o = Overload {
            caster,
            spell: s.clone(),
            target,
            t: 0.0,
            riders: riders.clone(),
            strikes: 0,
            field: None,
            ride_t: 0.0,
            thing: None,
            wave_dir: Vec3::NEG_Z,
            cloud: None,
            end_at: f64::INFINITY,
            crushed: Vec::new(),
            next_t: 0.0,
        };
        // The cannon and the boulder go the way the caster aims, as the
        // wave does (see Water below).
        let aim = {
            let pos = flat(self.pos(caster));
            let mut dir = flat(target - pos).normalize_or_zero();
            if dir == Vec3::ZERO {
                dir = self.ecs.get::<&crate::wizard::Wizard>(caster).map(|w| flat(w.aim_dir).normalize_or_zero()).unwrap_or(Vec3::ZERO);
            }
            if dir == Vec3::ZERO { Vec3::NEG_Z } else { dir }
        };
        self.shake(0.5);
        match s.primary {
            Some(El::Fire) => {
                o.thing = Some(METEOR_FROM);
                // With earth in it, a shower: more rocks rain down round it.
                if riders.contains(&El::Earth) {
                    for i in 0..4 {
                        let mut more = s.clone();
                        more.riders.clear();
                        let a = i as f32 * TAU / 4.0 + self.rng.f() * 0.8;
                        let off = yaw_rot(Vec3::X, a) * self.rng.range(3.0, 5.0);
                        let at = self.level.clamp_inside(target + off);
                        self.later(
                            0.25 + i as f64 * 0.2,
                            crate::world::Later::Meteor {
                                caster,
                                spell: Box::new(more),
                                at,
                            },
                        );
                    }
                }
            }
            Some(El::Frost) => {
                let f = self.add_field(target, El::Frost, 5.5, 9.0, Some(caster));
                self.field_mut(f).mix = vec![(El::Frost, 3.0)];
                self.field_mut(f).keep_mix = true;
                o.field = Some(f);
                self.ride(&o, target, 5.0);
            }
            Some(El::Water) => {
                // It rolls from the caster towards `target` (the caster
                // passes its aim that way): the Wizard component is not the
                // live one while its own cast runs, and read 0, so the wave
                // stood still (Joep).
                let pos = flat(self.pos(caster));
                let mut dir = flat(target - pos).normalize_or_zero();
                if dir == Vec3::ZERO {
                    dir = self.ecs.get::<&crate::wizard::Wizard>(caster).map(|w| flat(w.aim_dir).normalize_or_zero()).unwrap_or(Vec3::ZERO);
                }
                if dir == Vec3::ZERO {
                    dir = Vec3::NEG_Z;
                }
                o.thing = Some(pos + dir * WAVE_START);
                o.wave_dir = dir;
            }
            Some(El::Lightning) => {
                o.cloud = Some(target + Vec3::Y * 9.0);
                if riders.contains(&El::Water) {
                    let f = self.add_field(target, El::Water, 5.5, STORM_TIME + 6.0, Some(caster));
                    self.field_mut(f).friendly = true;
                    o.field = Some(f);
                }
                self.shake(0.4);
            }
            Some(El::Earth) => {
                // A boulder rises over the caster's head (Joep).
                o.wave_dir = aim;
                o.thing = Some(self.pos(caster) + Vec3::Y * 3.2);
                self.burst(self.pos(caster) + Vec3::Y * 0.3, fx::el_c(El::Earth), 16, 3.0, 0.3, Some(El::Earth));
            }
            Some(El::Air) => {
                let f = self.add_field(target, El::Air, 2.4, 8.0, Some(caster));
                let mut mix = vec![(El::Air, 2.0)];
                if !riders.is_empty() {
                    for &(e, n) in &s.counts {
                        if !mix.iter().any(|m| m.0 == e) {
                            mix.push((e, n as f64));
                        }
                    }
                }
                self.field_mut(f).mix = mix;
                self.field_mut(f).keep_mix = true;
                o.field = Some(f);
            }
            Some(El::Poison) => {
                // The venom cannon: hosed out of the staff (Joep).
                o.wave_dir = aim;
                o.thing = Some(self.staff_tip(caster));
                let _ = c;
            }
            Some(El::Void) => {
                o.thing = Some(target + Vec3::Y * 1.2);
            }
            _ => {}
        }
        self.spawn_fx(o);
    }

    fn overload_step(&mut self, id: Id, dt: f64) {
        let mut o: Overload = self.get(id);
        o.t += dt;
        let caster = o.caster;
        let done = match o.spell.primary {
            Some(El::Fire) => {
                if o.thing.is_some() {
                    // Falls faster and faster, and hits at full speed.
                    let k = (o.t / 0.9).clamp(0.0, 1.0) as f32;
                    o.thing = Some(METEOR_FROM.lerp(Vec3::ZERO, k * k));
                    if o.t > 0.9 {
                        o.thing = None;
                        let s = o.spell.clone();
                        self.nova(
                            caster,
                            &s,
                            NovaN {
                                radius: 5.0,
                                rate: 1.6,
                            },
                            o.target,
                            true,
                            None,
                            1.0,
                        );
                        self.add_field(o.target, El::Fire, 3.2, 8.0, Some(caster));
                        self.burst(
                            o.target + Vec3::Y * 0.5,
                            fx::el_c(El::Fire),
                            40,
                            9.0,
                            0.5,
                            Some(El::Fire),
                        );
                        self.ride(&o, o.target, 4.5);
                        self.shake(1.2);
                    }
                }
                o.t > 1.2
            }
            Some(El::Water) => {
                let dir = o.wave_dir;
                let mut at = o.thing.unwrap_or(o.target);
                at += dir * WAVE_SPEED * dt as f32;
                o.thing = Some(at);
                let side = Vec3::new(dir.z, 0.0, -dir.x).normalize_or_zero();
                let ff = self.ff();
                let mix = mix_of(&o.spell.counts);
                for a in self.living() {
                    if a == caster {
                        continue;
                    }
                    let d = flat(self.pos(a) - at);
                    if d.dot(dir).abs() < 1.4 && d.dot(side).abs() < WAVE_WIDTH * 0.5 {
                        {
                            let mut ac = self.am(a);
                            ac.push = ac.push.lerp(dir * 13.0, 0.5);
                        }
                        // As much in all as at its old 11 m/s: a slower wave holds
                        // a monster longer.
                        self.hit(a, &mix, dt * 1.2 * (WAVE_SPEED / 11.0) as f64, Some(caster), dir, ff);
                    }
                }
                if o.t > WAVE_TIME {
                    self.add_field(at, El::Water, 3.0, 10.0, Some(caster));
                    self.ride(&o, at, 4.0);
                    true
                } else {
                    false
                }
            }
            Some(El::Lightning) => {
                self.storm(&mut o, dt);
                o.t >= o.end_at
            }
            Some(El::Earth) => self.boulder(&mut o, dt),
            Some(El::Poison) => self.venom(&mut o, dt),
            Some(El::Void) => {
                let caster_team = self.team(caster);
                for a in self.living() {
                    let mut ac = self.am(a);
                    if ac.team == caster_team {
                        continue;
                    }
                    let d = flat(o.target - ac.pos);
                    if d.length() < 9.0 && d.length() > 0.5 {
                        ac.push = ac.push.lerp(d.normalize() * 7.0, (4.0 * dt) as f32);
                    }
                }
                if o.t > 3.0 {
                    let s = o.spell.clone();
                    self.nova(
                        caster,
                        &s,
                        NovaN {
                            radius: 4.5,
                            rate: 1.4,
                        },
                        o.target,
                        true,
                        Some(MONSTERS),
                        1.0,
                    );
                    self.ride(&o, o.target, 4.5);
                    self.shake(0.8);
                    true
                } else {
                    false
                }
            }
            Some(El::Frost) | Some(El::Air) => {
                let mut end = o.t > 9.0;
                match o.field.filter(|&f| self.ecs.contains(f)) {
                    Some(f) => {
                        if o.spell.primary == Some(El::Air) {
                            // The tornado hunts the nearest monster.
                            let fp = self.field_ref(f).pos;
                            let caster_team = self.team(caster);
                            let mut best = None;
                            let mut bd = f32::INFINITY;
                            for a in self.living() {
                                let ac = self.a(a);
                                if ac.team != caster_team {
                                    let d = ac.pos.distance(fp);
                                    if d < bd {
                                        bd = d;
                                        best = Some(ac.pos);
                                    }
                                }
                            }
                            if let Some(bp) = best {
                                self.field_mut(f).pos =
                                    crate::world::move_toward(fp, bp, 3.5 * dt as f32);
                            }
                            // What rides on the tornado is flung out of it now and then.
                            o.ride_t -= dt;
                            if o.ride_t <= 0.0 && !o.riders.is_empty() {
                                o.ride_t = 1.2;
                                let at = self.field_ref(f).pos;
                                self.ride(&o, at, 2.6);
                            }
                        }
                    }
                    None => end = true,
                }
                end
            }
            _ => o.t > 1.0,
        };
        if done {
            self.gone(id);
        } else {
            self.put(id, o);
        }
    }

    /// The boulder's tick: grows over the head, flies, rolls and crushes.
    /// True once it has stopped.
    fn boulder(&mut self, o: &mut Overload, dt: f64) -> bool {
        let caster = o.caster;
        let dir = o.wave_dir;
        let alive = self.ecs.contains(caster);
        if o.t < BOULDER_GROW {
            if alive {
                o.thing = Some(self.pos(caster) + Vec3::Y * 3.2);
                o.target = flat(self.pos(caster));
            }
            return false;
        }
        let from = o.target + Vec3::Y * 3.2;
        let land = o.target + dir * BOULDER_LAND + Vec3::Y * BOULDER_R;
        if o.t < BOULDER_GROW + BOULDER_THROW {
            let k = ((o.t - BOULDER_GROW) / BOULDER_THROW) as f32;
            o.thing = Some(from.lerp(land, k) + Vec3::Y * (1.5 * k * (1.0 - k)));
            return false;
        }
        let mut at = o.thing.unwrap_or(land);
        if o.t - dt < BOULDER_GROW + BOULDER_THROW {
            at = land;
            self.shake(0.6);
            self.burst(flat(at), fx::el_c(El::Earth), 24, 5.0, 0.35, Some(El::Earth));
        }
        let next = at + dir * BOULDER_SPEED * dt as f32;
        let held = self.level.clamp_inside(next);
        let blocked = flat(held).distance(flat(next)) > 0.05;
        at = Vec3::new(held.x, BOULDER_R, held.z);
        o.thing = Some(at);
        let side = Vec3::new(dir.z, 0.0, -dir.x);
        let mix = mix_of(&o.spell.counts);
        let ff = self.ff();
        let caster_team = if alive { Some(self.team(caster)) } else { None };
        for a in self.living() {
            if a == caster || Some(self.team(a)) == caster_team {
                continue;
            }
            let (p, r) = {
                let ac = self.a(a);
                (ac.pos, ac.radius)
            };
            let d = flat(p - at);
            if d.length() > BOULDER_R + r + 0.2 {
                continue;
            }
            // Out of its way: shoved to whichever side it was on, a little ahead.
            let way = if d.dot(side) >= 0.0 { side } else { -side };
            {
                let mut ac = self.am(a);
                ac.push = way * 14.0 + dir * 5.0;
                ac.stun = ac.stun.max(0.8);
            }
            if !o.crushed.contains(&a) {
                o.crushed.push(a);
                self.hit(a, &mix, 1.6, Some(caster), dir, ff);
                self.burst(p + Vec3::Y * 0.6, fx::el_c(El::Earth), 10, 4.0, 0.25, Some(El::Earth));
            }
        }
        if blocked || o.t > BOULDER_GROW + BOULDER_THROW + BOULDER_ROLL {
            // It breaks up where it stops.
            let s = o.spell.clone();
            let end = flat(at);
            self.nova(caster, &s, NovaN { radius: 3.0, rate: 0.8 }, end, true, Some(MONSTERS), 1.0);
            for i in 0..8 {
                let q = end + yaw_rot(Vec3::X, i as f32 * TAU / 8.0) * self.rng.range(0.5, 2.5);
                self.burst(q, fx::el_c(El::Earth), 10, 5.0, 0.35, Some(El::Earth));
            }
            self.ride(o, end, 3.5);
            self.shake(0.9);
            return true;
        }
        false
    }

    /// The venom cannon's tick: globs hosed from the staff where it aims.
    fn venom(&mut self, o: &mut Overload, dt: f64) -> bool {
        let caster = o.caster;
        if !self.ecs.contains(caster) || !self.alive(caster) {
            return true;
        }
        // It follows the staff: the caster can sweep it round.
        let aim = self.ecs.get::<&crate::wizard::Wizard>(caster).map(|w| flat(w.aim_dir).normalize_or_zero()).unwrap_or(Vec3::ZERO);
        if aim != Vec3::ZERO {
            o.wave_dir = aim;
        }
        let tip = self.staff_tip(caster);
        o.thing = Some(tip);
        o.next_t -= dt;
        while o.next_t <= 0.0 && o.t < VENOM_TIME {
            o.next_t += VENOM_EVERY;
            o.strikes += 1;
            let a = self.rng.range(-VENOM_CONE, VENOM_CONE);
            let d = VENOM_NEAR + (VENOM_FAR - VENOM_NEAR) * self.rng.f().sqrt();
            let to = flat(self.level.clamp_inside(flat(tip) + yaw_rot(o.wave_dir, a) * d));
            let flight = 0.2 + d as f64 * 0.03;
            self.ev(Ev::Lob { from: tip, to, el: El::Poison, time: flight as f32 });
            self.later(
                flight,
                crate::world::Later::Venom { caster, at: to, spell: Box::new(o.spell.clone()), puddle: o.strikes % 4 == 0 },
            );
        }
        if o.t >= VENOM_TIME && !o.riders.is_empty() && o.ride_t == 0.0 {
            o.ride_t = 1.0;
            let mid = flat(tip) + o.wave_dir * (VENOM_NEAR + VENOM_FAR) * 0.5;
            self.ride(o, mid, 4.0);
        }
        o.t >= VENOM_TIME + 0.6
    }

    /// One glob of the venom cannon lands: it splashes, and every few
    /// leave a puddle that eats at what walks in it.
    pub fn venom_splat(&mut self, caster: Id, at: Vec3, s: &Spell, puddle: bool) {
        let mix = mix_of(&s.counts);
        let ff = self.ff();
        let team = if self.ecs.contains(caster) { Some(self.team(caster)) } else { None };
        for a in self.living() {
            if a == caster || Some(self.team(a)) == team {
                continue;
            }
            let (p, r) = {
                let ac = self.a(a);
                (ac.pos, ac.radius)
            };
            let d = flat(p - at);
            if d.length() < 1.0 + r {
                self.hit(a, &mix, 0.09, Some(caster), d.normalize_or_zero(), ff);
            }
        }
        self.ev(Ev::Splat { at, el: El::Poison, big: puddle });
        if puddle {
            let f = self.add_field(at, El::Poison, 1.5, 7.0, Some(caster));
            self.field_mut(f).mix = vec![(El::Poison, 1.0)];
            self.field_mut(f).keep_mix = true;
            self.field_mut(f).friendly = true;
        }
    }

    fn storm(&mut self, o: &mut Overload, dt: f64) {
        let Some(mut cloud) = o.cloud else { return };
        // The cloud drifts after the nearest monster, but stays near.
        if let Some(best) = self.nearest_foe(o.caster, cloud, 9.0) {
            let mut want = self.pos(best);
            if want.distance(o.target) > 6.0 {
                want = o.target + (want - o.target).normalize() * 6.0;
            }
            let at = Vec3::new(want.x, cloud.y, want.z);
            cloud = crate::world::move_toward(cloud, at, 4.0 * dt as f32);
            o.cloud = Some(cloud);
        }
        if let Some(f) = o.field.filter(|&f| self.ecs.contains(f)) {
            let fp = self.field_ref(f).pos;
            self.field_mut(f).pos = crate::world::move_toward(fp, flat(cloud), 4.0 * dt as f32);
        }
        if o.t > 0.45 && o.t < STORM_TIME && o.t > 0.45 + o.strikes as f64 * STORM_EVERY {
            o.strikes += 1;
            self.storm_bolt(o);
        }
        if o.t > STORM_TIME {
            o.cloud = None;
            o.end_at = o.t + 0.5;
        }
    }

    fn nearest_foe(&self, caster: Id, from: Vec3, reach: f32) -> Option<Id> {
        let team = self.team(caster);
        let mut best = None;
        let mut bd = reach;
        for a in self.living() {
            let ac = self.a(a);
            if ac.team == team || !ac.alive {
                continue;
            }
            let d = flat(ac.pos).distance(flat(from));
            if d < bd {
                bd = d;
                best = Some(a);
            }
        }
        best
    }

    fn storm_bolt(&mut self, o: &Overload) {
        let under = o.cloud.map(flat).unwrap_or(o.target);
        // Most strikes find a monster under the cloud, the rest the ground.
        let best = if self.rng.f() < 0.7 {
            self.nearest_foe(o.caster, under, 5.5)
        } else {
            None
        };
        let at = match best {
            Some(b) => self.pos(b),
            None => under + Vec3::new(self.rng.range(-4.0, 4.0), 0.0, self.rng.range(-4.0, 4.0)),
        };
        self.arc_strike(o.caster, &o.spell, at);
        let c = fx::el_c(El::Lightning);
        self.ev(Ev::LightPop {
            at,
            c,
            energy: 6.0,
            range: 10.0,
            time: 0.2,
        });
        self.burst(at + Vec3::Y * 0.3, c, 14, 6.0, 0.25, Some(El::Lightning));
        self.touch_ground(at, 2.5, El::Lightning, Some(o.caster));
        if o.strikes % 3 == 0 {
            self.ride(o, at, 2.5);
        }
        self.shake(0.3);
    }

    /// One strike out of the storm cloud and its chain (ArcFx.zap with a
    /// fixed origin): it never jumps to the caster's side, wet or not.
    fn arc_strike(&mut self, caster: Id, s: &Spell, at: Vec3) {
        const RANGE: f32 = 2.5;
        const HOPS: usize = 2;
        const HOP_RANGE: f32 = 4.0;
        const RATE: f64 = 0.55;
        let origin = flat(at) + Vec3::Y * 9.0;
        let team = self.team(caster);
        let mut first = None;
        let mut best_score = f32::INFINITY;
        for a in self.living() {
            if a == caster {
                continue;
            }
            let ac = self.a(a);
            if !ac.alive {
                continue;
            }
            let d = flat(ac.pos - flat(origin)).length();
            if d > RANGE {
                continue;
            }
            let mut score = d;
            if ac.team == team {
                if ac.wet <= 0.0 {
                    continue;
                }
                score += 3.0;
            }
            if score < best_score {
                first = Some(a);
                best_score = score;
            }
        }
        let Some(first) = first else { return };
        let mut chain = vec![first];
        while chain.len() < 1 + HOPS + self.a(caster).mods.chain as usize {
            let last = *chain.last().unwrap();
            let (lp, lw) = {
                let l = self.a(last);
                (l.pos, l.wet)
            };
            let reach = HOP_RANGE * if lw > 0.0 { 1.8 } else { 1.0 };
            let mut best = None;
            let mut best_d = f32::INFINITY;
            for a in self.living() {
                if a == caster || chain.contains(&a) {
                    continue;
                }
                let ac = self.a(a);
                if !ac.alive || ac.team == team {
                    continue;
                }
                let d = ac.pos.distance(lp);
                if d < reach && d < best_d {
                    best = Some(a);
                    best_d = d;
                }
            }
            let Some(b) = best else { break };
            chain.push(b);
        }
        let mut pts = vec![origin];
        let mut prev = origin;
        let ff = self.ff();
        let mix = mix_of(&s.counts);
        for &a in &chain {
            let p = self.pos(a) + Vec3::Y * 0.9;
            let d = p - prev;
            self.hit(a, &mix, RATE, Some(caster), flat(d).normalize_or_zero(), ff);
            pts.push(p);
            prev = p;
        }
        let c = color(s);
        self.ev(Ev::Chain { pts, c });
        let fp = self.pos(chain[0]);
        self.ev(Ev::LightPop {
            at: fp,
            c,
            energy: 3.0,
            range: 6.0,
            time: 0.15,
        });
        for &a in &chain {
            let ap = self.pos(a);
            self.touch(&s.counts.clone(), ap, 0.6, Some(caster));
        }
    }

    /// Every element riding on the cataclysm adds itself at `at`.
    fn ride(&mut self, o: &Overload, at: Vec3, r: f64) {
        let ground = flat(at);
        let caster = o.caster;
        let team = self.team(caster);
        for &e in &o.riders {
            let c = fx::el_c(e);
            self.burst(ground + Vec3::Y * 0.5, c, 24, 7.0, 0.35, Some(e));
            match e {
                El::Water | El::Fire | El::Frost => {
                    if !(e == El::Water && o.spell.primary == Some(El::Lightning)) {
                        let f = self.add_field(ground, e, r * 0.7, 6.0, Some(caster));
                        self.field_mut(f).friendly = true;
                        self.field_mut(f).splash = true;
                    }
                }
                El::Poison => {
                    let f = self.add_field(ground, El::Poison, r * 0.7, 7.0, Some(caster));
                    self.field_mut(f).friendly = true;
                    self.field_mut(f).splash = true;
                }
                El::Lightning => {
                    self.touch_ground(ground, r, El::Lightning, Some(caster));
                    self.ev(Ev::LightPop {
                        at: ground,
                        c,
                        energy: 4.0,
                        range: r as f32 * 3.0,
                        time: 0.25,
                    });
                }
                El::Earth | El::Air | El::Void => {
                    for a in self.living() {
                        let mut ac = self.am(a);
                        if ac.team == team {
                            continue;
                        }
                        let d = flat(ac.pos - ground);
                        if d.length() as f64 > r + 1.5 {
                            continue;
                        }
                        match e {
                            El::Earth => ac.stun = ac.stun.max(1.2),
                            El::Air => ac.push += d.normalize_or_zero() * 14.0,
                            _ => ac.push += -d.normalize_or_zero() * 9.0,
                        }
                    }
                    if e == El::Earth {
                        self.ev(Ev::Ring {
                            at: ground,
                            c,
                            r: r as f32,
                            time: 0.4,
                        });
                    } else if e == El::Void {
                        self.ev(Ev::Ring {
                            at: ground,
                            c,
                            r: r as f32 * 1.3,
                            time: 0.5,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

/// The main colour of a spell.
pub fn color(s: &Spell) -> fx::C {
    s.primary.map(fx::el_c).unwrap_or(fx::WHITE)
}

/// Grammar numbers a nova reads, with Godot's defaults.
pub fn nova_n(n: &Numbers) -> NovaN {
    NovaN {
        radius: if n.radius > 0.0 { n.radius } else { 3.0 },
        rate: if n.has_rate { n.rate } else { 1.0 },
    }
}

/// Shapes the wizard winds up before they go.
pub fn winds_up(s: Shape) -> bool {
    matches!(
        s,
        Shape::Spray | Shape::Beam | Shape::Strike | Shape::Projectile | Shape::Overload
    )
}
