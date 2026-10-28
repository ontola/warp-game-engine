//! The ground: fields of element lying on it (field.gd), how they react when
//! an element touches them (Arena.touch_ground), blasts that break props, and
//! the props themselves breaking and spilling what they held (prop.gd).

use crate::El;
use crate::fx::{self, Ev};
use crate::level::Spill;
use crate::pickups::{Globe, Soul};
use crate::rng::Rand;
use crate::world::{Id, Mix, World, flat, one};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

#[derive(Clone, Debug)]
pub struct Field {
    pub kind: El,
    /// What it lands on whoever stands in it.
    pub mix: Mix,
    /// A trap's or a cataclysm's own mix, kept when the kind changes.
    pub keep_mix: bool,
    pub pos: Vec3,
    pub radius: f64,
    pub ttl: f64,
    pub permanent: bool,
    pub electrified: f64,
    pub owner: Option<Id>,
    pub rate: f64,
    /// Only hurts the other side of whoever laid it.
    pub friendly: bool,
    /// It just landed: the app plays the splash.
    pub splash: bool,
    /// Gone: the app fades it out.
    pub fading: bool,
    /// Bumped when it changes kind or size, so the app rebuilds its look.
    pub look: u32,
}

impl World {
    pub fn add_field(&mut self, at: Vec3, kind: El, r: f64, ttl: f64, owner: Option<Id>) -> Id {
        let o = self.order();
        let f = Field {
            kind,
            mix: one(kind),
            keep_mix: false,
            pos: flat(at),
            radius: r,
            ttl,
            permanent: false,
            electrified: 0.0,
            owner,
            rate: 0.55,
            friendly: false,
            splash: false,
            fading: false,
            look: 0,
        };
        let id = self.ecs.spawn((o, f));
        self.fields.push(id);
        id
    }

    pub fn field_mut(&mut self, id: Id) -> hecs::RefMut<'_, Field> {
        self.ecs.get::<&mut Field>(id).expect("field")
    }

    pub fn field_ref(&self, id: Id) -> hecs::Ref<'_, Field> {
        self.ecs.get::<&Field>(id).expect("field")
    }

    fn set_kind(&mut self, id: Id, k: El) {
        let mut f = self.field_mut(id);
        if f.kind == k {
            return;
        }
        f.kind = k;
        if f.keep_mix {
            f.mix = one(k);
        } else {
            f.mix = one(k);
            f.rate = 0.55;
        }
        f.electrified = 0.0;
        f.look += 1;
    }

    /// Fades out and stops working (Field.fade).
    pub fn fade_field(&mut self, id: Id) {
        let _ = self.ecs.despawn(id);
    }

    pub fn field_step(&mut self, id: Id, dt: f64) {
        let mut f = (*self.field_ref(id)).clone();
        if !f.permanent {
            f.ttl -= dt;
            if f.ttl <= 0.0 {
                self.fade_field(id);
                return;
            }
        }
        f.electrified = (f.electrified - dt).max(0.0);
        f.splash = false;
        let ff = self.ff();
        let src = f.owner.filter(|&o| self.ecs.contains(o));
        let src_team = src.map(|s| self.team(s));
        for a in self.living() {
            let (ap, team, firewalker) = {
                let ac = self.a(a);
                (ac.pos, ac.team, ac.mods.firewalker)
            };
            let d = flat(ap - f.pos);
            if d.length() as f64 > f.radius {
                continue;
            }
            if f.friendly && src_team == Some(team) {
                continue;
            }
            if firewalker {
                continue;
            }
            match f.kind {
                El::Frost | El::Ice => self.am(a).slick = 0.25,
                El::Air => {
                    let tangent = d.cross(Vec3::Y).normalize_or_zero();
                    let p = (tangent * 5.0 - d.normalize_or_zero() * 2.0) * (dt * 4.0) as f32;
                    self.am(a).push += p;
                }
                _ => {}
            }
            let dir = if d.length() > 0.1 {
                d.normalize()
            } else {
                Vec3::ZERO
            };
            self.hit(a, &f.mix, dt * f.rate, src, dir, ff * 0.6);
            if f.electrified > 0.0 {
                self.hit(a, &one(El::Lightning), dt * 0.7, src, Vec3::ZERO, ff * 0.6);
            }
        }
        if let Ok(mut c) = self.ecs.get::<&mut Field>(id) {
            c.ttl = f.ttl;
            c.electrified = f.electrified;
            c.splash = false;
        }
    }

    /// An element touches the ground: fields react the way you would guess.
    pub fn touch_ground(&mut self, at: Vec3, r: f64, el: El, src: Option<Id>) {
        for id in self.fields.clone() {
            if !self.ecs.contains(id) {
                continue;
            }
            let (fp, fr, k, permanent) = {
                let f = self.field_ref(id);
                (f.pos, f.radius, f.kind, f.permanent)
            };
            let d = Vec2::new(fp.x - at.x, fp.z - at.z).length() as f64;
            if d > fr + r {
                continue;
            }
            let hot = matches!(el, El::Fire | El::Magma);
            let cold = matches!(el, El::Frost | El::Ice);
            let wetting = matches!(el, El::Water | El::Steam | El::Mud);
            let zap = matches!(el, El::Lightning | El::Storm);
            if zap && k == El::Water {
                self.field_mut(id).electrified = 3.0;
                continue;
            }
            if permanent && k == El::Magma {
                continue;
            }
            if hot && k == El::Water {
                self.set_kind(id, El::Steam);
                self.field_mut(id).ttl = 5.0;
                self.steam_pop(fp, fr);
            } else if hot && matches!(k, El::Frost | El::Ice) {
                self.set_kind(id, El::Water);
                let mut f = self.field_mut(id);
                f.ttl = f.ttl.max(8.0);
            } else if cold && k == El::Water {
                self.set_kind(id, El::Frost);
                let mut f = self.field_mut(id);
                f.ttl = f.ttl.max(10.0);
            } else if cold && k == El::Steam {
                self.set_kind(id, El::Water);
                self.field_mut(id).ttl = 8.0;
            } else if (cold || wetting || el == El::Earth) && k == El::Fire && !permanent {
                self.steam_pop(fp, fr);
                self.fade_field(id);
                continue;
            } else if el == El::Air && k == El::Steam {
                self.fade_field(id);
                continue;
            } else if el == El::Air && k == El::Fire {
                let mut f = self.field_mut(id);
                f.radius = (f.radius * 1.12).min(4.5);
                f.look += 1;
            }
            if !permanent && src.is_some() {
                self.field_mut(id).owner = src;
            }
        }
    }

    fn steam_pop(&mut self, at: Vec3, r: f64) {
        self.ev(Ev::Puff {
            at: at + Vec3::Y * 0.5,
            c: fx::rgba(0.85, 0.85, 0.9, 0.6),
            size: r as f32,
        });
    }

    /// Something hit the ground hard: the props within `r` take `dmg`.
    pub fn blast(&mut self, at: Vec3, r: f32, dmg: f32) {
        for i in self.level.props_near(at, r) {
            self.damage_prop(i, dmg, Vec3::ZERO, Some(at));
        }
    }

    /// A prop takes `amount` from a hit along `dir` (zero for a blast from
    /// `at`). It breaks when its health is gone (Prop.damage, shatter).
    pub fn damage_prop(&mut self, i: usize, amount: f32, dir: Vec3, at: Option<Vec3>) {
        let p = &self.level.props[i];
        if p.broken || !p.breakable {
            return;
        }
        let pos = p.pos;
        if !self.level.damage_prop(i, amount) {
            self.ev(Ev::PropChip { prop: i });
            self.burst(
                pos + Vec3::Y * 0.6,
                fx::rgb(0.5, 0.47, 0.42),
                3,
                2.0,
                0.08,
                None,
            );
            return;
        }
        self.count("props_broken");
        let mut power = (amount * 0.35).clamp(1.2, 8.0);
        let d = flat(dir);
        let from = if d.length() > 0.05 {
            // Behind the prop, so everything flies on the way the hit went.
            pos - d.normalize() * 1.6 + Vec3::Y * 0.3
        } else if let Some(a) = at {
            a
        } else {
            power = 1.5;
            pos + Vec3::Y * 0.3
        };
        self.ev(Ev::PropBroke {
            prop: i,
            from,
            power,
        });
        self.spill(i);
    }

    fn spill(&mut self, i: usize) {
        let p = &self.level.props[i];
        let pos = p.pos;
        match p.spill {
            Spill::Souls => {
                for _ in 0..3 {
                    let a = self.rng.f() * TAU;
                    let v = Vec3::new(a.cos(), 0.0, a.sin()) * self.rng.range(1.5, 3.5);
                    let t0 = self.rng.f() as f64 * 0.2;
                    self.spawn_fx(Soul::new(pos + Vec3::Y * 0.6, v, 4.0, t0));
                }
            }
            Spill::Globe => {
                if self.rng.f() < 0.6 {
                    self.drop_globes(pos, 1);
                }
            }
            Spill::Blood => self.drop_globes(pos, 2),
            Spill::Fire => {
                // Burning oil splashes out: it scorches whoever stands close,
                // sets the ground alight and breaks what is next to it.
                let at = flat(pos);
                self.ev(Ev::Boom {
                    at,
                    r: 2.2,
                    mark: true,
                });
                self.burst(
                    at + Vec3::Y * 0.5,
                    fx::el_c(El::Fire),
                    30,
                    8.0,
                    0.3,
                    Some(El::Fire),
                );
                self.add_field(at, El::Fire, 2.4, 6.0, None);
                for a in self.living() {
                    let dd = flat(self.pos(a) - at);
                    if dd.length() < 2.8 {
                        let dir = if dd.length() > 0.1 {
                            dd.normalize()
                        } else {
                            Vec3::ZERO
                        };
                        self.hit(a, &one(El::Fire), 1.6, None, dir, 1.0);
                    }
                }
                self.shake(0.5);
                self.blast(at, 2.6, 8.0);
            }
            Spill::None => {}
        }
    }

    pub fn drop_globes(&mut self, at: Vec3, n: usize) {
        for i in 0..n {
            let a = self.rng.f() * TAU;
            let speed = if n > 1 || i > 0 {
                self.rng.range(1.0, 3.0)
            } else {
                0.5
            };
            let g = Globe::new(flat(at), Vec3::new(a.cos(), 0.0, a.sin()) * speed);
            self.spawn_fx(g);
        }
    }
}
