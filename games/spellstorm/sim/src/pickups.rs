//! Monster bullets (bullet.gd) and what lies on the floor to take: blood,
//! the only way to heal (globe.gd), and mana shards (soul.gd).

use crate::El;
use crate::balance as b;
use crate::fx;
use crate::world::{Id, Mix, World, flat};
use glam::{Vec2, Vec3};

/// A monster's bullet. A wizard's swing knocks it back; then it turns gold
/// and hurts monsters.
#[derive(Clone, Debug)]
pub struct Bullet {
    pub pos: Vec3,
    pub vel: Vec3,
    pub mix: Mix,
    pub primary: El,
    /// How hard it hits (Enemy._attack_spell's scale).
    pub scale: f64,
    pub arrow: bool,
    pub src: Option<Id>,
    pub team: u8,
    pub radius: f32,
    pub life: f64,
    pub power: f64,
    /// Knocked back by a staff.
    pub hot: bool,
}

#[derive(Clone, Debug)]
pub struct Globe {
    pub pos: Vec3,
    pub vel: Vec3,
    pub t: f64,
    /// Who drinks it, and how far through the slurp.
    pub drinker: Option<Id>,
    pub slurp: f64,
    /// How hard it reaches for the nearest wizard (the shader's pull).
    pub pull: f64,
    pub to: Vec2,
}

pub const GLOBE_HEAL: f64 = 30.0;
pub const GLOBE_MAGNET: f32 = 2.2;
pub const GLOBE_TAKE: f32 = 0.8;
pub const GLOBE_LIFE: f64 = 30.0;
pub const GLOBE_SLURP: f64 = 0.5;

impl Globe {
    pub fn new(pos: Vec3, vel: Vec3) -> Globe {
        Globe {
            pos,
            vel,
            t: 0.0,
            drinker: None,
            slurp: 0.0,
            pull: 0.0,
            to: Vec2::ZERO,
        }
    }

    /// Flickers out over its last seconds.
    pub fn visible(&self) -> bool {
        self.t < GLOBE_LIFE - 3.0 || (self.t * 6.0).fract() < 0.6
    }
}

#[derive(Clone, Debug)]
pub struct Soul {
    pub pos: Vec3,
    pub vel: Vec3,
    pub mana: f64,
    pub t: f64,
}

pub const SOUL_MAGNET: f32 = 2.4;
pub const SOUL_TAKE: f32 = 0.75;
pub const SOUL_LIFE: f64 = 14.0;
pub const SOUL_COLOR: fx::C = fx::rgb(0.25, 0.5, 1.0);

impl Soul {
    pub fn new(pos: Vec3, vel: Vec3, mana: f64, t0: f64) -> Soul {
        Soul {
            pos,
            vel,
            mana,
            t: t0,
        }
    }

    pub fn visible(&self) -> bool {
        self.t < SOUL_LIFE - 2.0 || (self.t * 8.0).fract() < 0.6
    }
}

impl World {
    /// Fire a bullet from `from` (Bullet.fire).
    #[allow(clippy::too_many_arguments)]
    pub fn fire_bullet(
        &mut self,
        from: Id,
        at: Vec3,
        dir: Vec3,
        speed: f32,
        el: El,
        scale: f64,
        size: f32,
        arrow: bool,
    ) {
        let team = self.team(from);
        let b = Bullet {
            pos: Vec3::new(at.x, 1.0, at.z),
            vel: dir.normalize() * speed,
            mix: vec![(el, 1.0)],
            primary: el,
            scale,
            arrow,
            src: Some(from),
            team,
            radius: size,
            life: 7.0,
            power: 1.0,
            hot: false,
        };
        self.spawn_fx(b);
        self.count("bullets");
    }

    /// Knocked back by `by`'s staff: it flies where they aim, faster and
    /// harder, and now only hurts monsters.
    pub fn deflect(&mut self, id: Id, by: Id, dir: Vec3) {
        let team = self.team(by);
        let mut bu = self.ecs.get::<&mut Bullet>(id).unwrap();
        bu.src = Some(by);
        bu.team = team;
        bu.vel = dir.normalize() * (bu.vel.length() * 1.6).max(12.0);
        bu.power = 2.0;
        bu.life = 2.0;
        bu.hot = true;
        let at = bu.pos;
        drop(bu);
        self.burst(at, fx::rgb(1.0, 0.85, 0.4), 8, 3.0, 0.2, None);
    }

    /// The bullets in flight now.
    pub fn bullets(&self) -> Vec<Id> {
        self.fx
            .iter()
            .copied()
            .filter(|&id| self.ecs.satisfies::<&Bullet>(id).unwrap_or(false))
            .collect()
    }

    pub fn globes(&self) -> Vec<Id> {
        self.fx
            .iter()
            .copied()
            .filter(|&id| {
                self.ecs
                    .get::<&Globe>(id)
                    .is_ok_and(|g| g.drinker.is_none())
            })
            .collect()
    }

    pub fn souls(&self) -> Vec<Id> {
        self.fx
            .iter()
            .copied()
            .filter(|&id| self.ecs.satisfies::<&Soul>(id).unwrap_or(false))
            .collect()
    }

    pub fn pickup_step(&mut self, id: Id, dt: f64) {
        if self.ecs.satisfies::<&Bullet>(id).unwrap_or(false) {
            self.bullet_step(id, dt);
        } else if self.ecs.satisfies::<&Globe>(id).unwrap_or(false) {
            self.globe_step(id, dt);
        } else if self.ecs.satisfies::<&Soul>(id).unwrap_or(false) {
            self.soul_step(id, dt);
        }
    }

    fn bullet_step(&mut self, id: Id, dt: f64) {
        let mut bu = self.ecs.get::<&Bullet>(id).map(|r| (*r).clone()).unwrap();
        bu.life -= dt;
        if bu.life <= 0.0 {
            let _ = self.ecs.despawn(id);
            return;
        }
        let from = bu.pos;
        let to = from + bu.vel * dt as f32;
        if self.ray(from, to).is_some() {
            self.burst(from, fx::el_c(bu.primary), 8, 3.0, 0.2, None);
            let _ = self.ecs.despawn(id);
            return;
        }
        bu.pos = to;
        for a in self.living() {
            let (alive, team, inv, ap, ar) = {
                let ac = self.a(a);
                (ac.alive, ac.team, ac.invuln, ac.pos, ac.radius)
            };
            if !alive || team == bu.team || inv > 0.0 {
                continue;
            }
            let d = Vec2::new(ap.x - to.x, ap.z - to.z).length();
            if d < ar + bu.radius {
                self.count(if bu.team == 1 {
                    "bullet_hits"
                } else {
                    "deflect_hits"
                });
                let src = bu.src.filter(|&s| self.ecs.contains(s));
                self.hit(
                    a,
                    &bu.mix,
                    bu.scale * bu.power,
                    src,
                    bu.vel.normalize(),
                    1.0,
                );
                self.burst(to, fx::el_c(bu.primary), 8, 3.0, 0.2, None);
                let _ = self.ecs.despawn(id);
                return;
            }
        }
        *self.ecs.get::<&mut Bullet>(id).unwrap() = bu;
    }

    fn globe_step(&mut self, id: Id, dt: f64) {
        let mut g = self.ecs.get::<&Globe>(id).map(|r| (*r).clone()).unwrap();
        g.t += dt;
        if let Some(w) = g.drinker {
            // Being drunk: the pool runs along its tongue into the drinker.
            g.slurp += dt / GLOBE_SLURP;
            if g.slurp >= 1.0 || !self.ecs.contains(w) {
                let _ = self.ecs.despawn(id);
                return;
            }
            let to = flat(self.pos(w) - g.pos);
            g.to = Vec2::new(to.x, to.z);
            g.pull = 1.0;
            g.pos += to * (4.0 * dt as f32).min(1.0);
            *self.ecs.get::<&mut Globe>(id).unwrap() = g;
            return;
        }
        if g.t > GLOBE_LIFE {
            let _ = self.ecs.despawn(id);
            return;
        }
        g.vel *= (-3.0 * dt).exp() as f32;
        let mut pos = g.pos;
        let mut pull = 0.0;
        if g.t > 0.3 {
            let mut best = None;
            let mut best_d = GLOBE_MAGNET;
            for &w in &self.wizards {
                let a = self.a(w);
                if !a.alive || a.hp >= a.max_hp - 0.5 {
                    continue;
                }
                let d = Vec2::new(a.pos.x - pos.x, a.pos.z - pos.z).length();
                if d < best_d {
                    best = Some(w);
                    best_d = d;
                }
            }
            if let Some(w) = best {
                let to = flat(self.pos(w) - pos);
                g.to = Vec2::new(to.x, to.z);
                pull = 1.0 - best_d / GLOBE_MAGNET;
                if best_d < GLOBE_TAKE {
                    let heal = GLOBE_HEAL * self.mods(w).heal;
                    self.am(w).healed(heal);
                    self.count("globes");
                    g.drinker = Some(w);
                    *self.ecs.get::<&mut Globe>(id).unwrap() = g;
                    return;
                }
                g.vel += to.normalize() * (10.0 + 20.0 * pull) * dt as f32;
            }
        }
        g.pull += (pull as f64 - g.pull) * (1.0 - (-10.0 * dt).exp());
        pos += g.vel * dt as f32;
        g.pos = self.level.clamp_inside(pos);
        *self.ecs.get::<&mut Globe>(id).unwrap() = g;
    }

    fn soul_step(&mut self, id: Id, dt: f64) {
        let mut s = self.ecs.get::<&Soul>(id).map(|r| (*r).clone()).unwrap();
        s.t += dt;
        if s.t > SOUL_LIFE {
            let _ = self.ecs.despawn(id);
            return;
        }
        let mut pos = s.pos;
        s.vel *= (-3.0 * dt).exp() as f32;
        if s.t > 0.35 {
            let mut best = None;
            // Nearest by the share of its pull (a Lodestone pulls from further).
            let mut best_d = SOUL_MAGNET;
            let mut best_k = 1.0;
            let mut best_r = SOUL_MAGNET;
            for &w in &self.wizards {
                let a = self.a(w);
                let wz = self.wiz(w);
                if !a.alive || wz.mana >= wz.mana_max() - 0.5 {
                    continue;
                }
                let r = SOUL_MAGNET * a.mods.magnet as f32;
                let d = Vec2::new(a.pos.x - pos.x, a.pos.z - pos.z).length();
                if d < r && d / r < best_k {
                    best = Some(w);
                    best_d = d;
                    best_k = d / r;
                    best_r = r;
                }
            }
            if let Some(w) = best {
                if best_d < SOUL_TAKE {
                    let souls = self.a(w).mods.souls;
                    self.wiz_mut(w).add_mana(s.mana * souls);
                    let at = self.pos(w) + Vec3::Y * 1.2;
                    self.burst(at, SOUL_COLOR, 6, 2.0, 0.12, None);
                    self.count("souls");
                    let _ = self.ecs.despawn(id);
                    return;
                }
                let to = self.pos(w) + Vec3::Y * 0.9 - pos;
                s.vel += to.normalize() * (14.0 + 30.0 * (1.0 - best_d / best_r)) * dt as f32;
            }
        }
        pos += s.vel * dt as f32;
        let k = 1.0 - (-4.0 * dt).exp();
        pos.y += ((0.6 + (s.t * 3.0).sin() * 0.12) as f32 - pos.y) * k as f32;
        let inside = self.level.clamp_inside(pos);
        s.pos = Vec3::new(inside.x, pos.y, inside.z);
        *self.ecs.get::<&mut Soul>(id).unwrap() = s;
    }

    /// Scatter the souls of a slain monster round where it fell.
    pub fn drop_souls(&mut self, at: Vec3, max_hp: f64) {
        let total = max_hp * b::SOUL_MANA_PER_HP;
        let count = ((total / 5.0).ceil() as i64).clamp(1, 10);
        for _ in 0..count {
            use crate::rng::Rand;
            let a = self.rng.f() * std::f32::consts::TAU;
            let v = Vec3::new(a.cos(), 0.0, a.sin()) * self.rng.range(2.0, 4.5);
            let t0 = self.rng.f() as f64 * 0.2;
            self.spawn_fx(Soul::new(at + Vec3::Y * 0.9, v, total / count as f64, t0));
        }
    }
}
