//! Anything with health that spells land on: wizards and monsters. Owns the
//! status rules (actor.gd), so every spell, field and monster plays by the
//! same ones:
//!   Wet:      water soaks. Fire dries you instead of burning you, frost
//!             freezes you on the spot, lightning does double damage.
//!   Burning:  fire and magma. Hurts over time; water or frost puts it out.
//!   Poisoned: poison seeps in and hurts slowly for a long time; nothing
//!             washes it out, and the undead feel it less.
//!   Chilled:  frost and ice stack up and slow; three stacks freeze solid.
//!   Frozen:   cannot move. Earth shatters it for double damage, fire thaws.
//!   Shocked:  lightning stuns for a moment.
//!   Slowed:   mud halves your speed.

use crate::El;
use crate::balance as b;
use crate::el::Effect;
use crate::relics::Mods;
use glam::Vec3;

pub const HURT_TIME: f64 = 0.14;

/// Damage and healing since the last floating number, and whether a hit in
/// that time found a weakness or a resistance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shown {
    pub dmg: f64,
    pub heal: f64,
    pub weak: bool,
    pub resist: bool,
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub team: u8,
    pub wizard: bool,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Which way the body faces (Godot rotation.y).
    pub yaw: f32,
    pub hp: f64,
    pub max_hp: f64,
    pub radius: f32,
    pub speed: f64,
    pub mass: f64,
    pub undead: bool,
    /// Base element -> resistance; 1 is immune, below 0 a weakness.
    pub resist: [f64; 13],
    /// Healed instead of hurt by this base element (imps bathe in fire).
    pub heals_from: Option<El>,
    pub alive: bool,
    pub armor: f64,
    pub wet: f64,
    pub burn: f64,
    pub poison: f64,
    pub chill: f64,
    pub frozen: f64,
    pub freeze_guard: f64,
    pub stun: f64,
    pub last_el: Option<El>,
    pub last_dir: Vec3,
    pub last_melee: bool,
    pub melee_hit: bool,
    pub overkill: f64,
    pub slow: f64,
    pub slick: f64,
    pub push: Vec3,
    pub shown: Shown,
    pub flash: f64,
    pub hurt_t: f64,
    pub hurt_dir: Vec3,
    pub hurt_k: f64,
    pub hurt_acc: f64,
    pub haste: f64,
    pub invuln: f64,
    /// Killing blows it may still survive this level (Phoenix Feather).
    pub winds: i32,
    pub wading: bool,
    pub ripple_t: f64,
    /// How long a freeze holds and how long frost cannot freeze again after.
    pub freeze_time: f64,
    pub freeze_immunity: f64,
    /// Bumps into other actors (Godot collision_mask has layer 2). A rolling
    /// wizard goes through monsters.
    pub blocks: bool,
    /// Other actors bump into this one (collision_layer 2). A lurker under
    /// the floor is not there.
    pub solid: bool,
    /// The caster's tomes, read by every hit it lands.
    pub mods: Mods,
    /// A shieldbearer's tower shield, and how long it is knocked aside.
    pub shield: bool,
    pub open_t: f64,
    /// Set by a death; World handles what follows after the current step.
    pub dying: bool,
}

impl Actor {
    pub fn new(team: u8, pos: Vec3, hp: f64, radius: f32, speed: f64) -> Actor {
        Actor {
            team,
            wizard: false,
            pos,
            vel: Vec3::ZERO,
            yaw: 0.0,
            hp,
            max_hp: hp,
            radius,
            speed,
            mass: 1.0,
            undead: false,
            resist: [0.0; 13],
            heals_from: None,
            alive: true,
            armor: 1.0,
            wet: 0.0,
            burn: 0.0,
            poison: 0.0,
            chill: 0.0,
            frozen: 0.0,
            freeze_guard: 0.0,
            stun: 0.0,
            last_el: None,
            last_dir: Vec3::ZERO,
            last_melee: false,
            melee_hit: false,
            overkill: 0.0,
            slow: 0.0,
            slick: 0.0,
            push: Vec3::ZERO,
            shown: Shown::default(),
            flash: 0.0,
            hurt_t: 0.0,
            hurt_dir: Vec3::ZERO,
            hurt_k: 0.0,
            hurt_acc: 0.0,
            haste: 1.0,
            invuln: 0.0,
            winds: 0,
            wading: false,
            ripple_t: 0.0,
            freeze_time: b::FREEZE_TIME,
            freeze_immunity: 0.0,
            blocks: true,
            solid: true,
            mods: Mods::default(),
            shield: false,
            open_t: 0.0,
            dying: false,
        }
    }

    /// Resistance to one element; fused ones average their parts.
    pub fn resistance(&self, e: El) -> f64 {
        let parts = e.parts();
        let mut r = 0.0;
        for p in &parts {
            let mut x = self.resist[p.idx()];
            if self.undead && *p == El::Poison {
                x += 0.4;
            }
            r += x;
        }
        (r / parts.len() as f64).clamp(-1.0, 1.0)
    }

    pub fn afflicted(&self) -> bool {
        self.wet > 0.0
            || self.burn > 0.0
            || self.chill > 0.0
            || self.poison > 0.0
            || self.frozen > 0.0
    }

    /// A hit lands: for HURT_TIME the body flashes and jerks away.
    pub fn hurt(&mut self, dmg: f64, dir: Vec3) {
        self.hurt_acc += dmg;
        if self.hurt_acc < 1.0 || self.hurt_t > HURT_TIME * 0.35 {
            return;
        }
        self.hurt_k = (0.45 + self.hurt_acc / 12.0).clamp(0.45, 1.3) / self.mass.max(1.0).sqrt();
        self.hurt_acc = 0.0;
        self.hurt_t = HURT_TIME;
        self.hurt_dir = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    }

    /// The statuses one element brings, `n` strong. Returns true when it put
    /// a fire out with water (a puff of steam shows).
    pub fn status(&mut self, eff: &Effect, n: f64) -> bool {
        let mut doused = false;
        if eff.wet > 0.0 {
            if self.burn > 0.0 {
                self.burn = 0.0;
                doused = true;
            }
            self.wet = (self.wet + eff.wet * n * 6.0).min(7.0);
        }
        if eff.burn > 0.0 {
            if self.wet > 0.0 {
                self.wet = (self.wet - eff.burn * n * 5.0).max(0.0);
            } else {
                self.burn = (self.burn + eff.burn * n * 4.0).min(b::BURN_MAX);
            }
            if self.frozen > 0.0 || self.chill > 0.0 {
                self.frozen = 0.0;
                self.chill = 0.0;
            }
        }
        if eff.chill > 0.0 {
            if self.wet > 0.0 && n > 0.05 {
                self.freeze();
            } else {
                self.chill += eff.chill * n;
                if self.chill >= 3.0 {
                    self.freeze();
                }
            }
        }
        if eff.slow > 0.0 {
            self.slow = self.slow.max(3.0 * eff.slow * (n * 4.0).min(1.0));
        }
        if eff.poison > 0.0 {
            self.poison = (self.poison + eff.poison * n * b::POISON_DOSE).min(b::POISON_CAP);
        }
        doused
    }

    pub fn freeze(&mut self) {
        if self.freeze_guard > 0.0 {
            // Still thawing out: frost only slows.
            self.chill = 2.0;
            return;
        }
        self.frozen = self.freeze_time;
        self.freeze_guard = self.frozen + self.freeze_immunity;
        self.chill = 0.0;
        self.wet = 0.0;
        self.burn = 0.0;
    }

    /// Heal by `amount`, up to full, and show it.
    pub fn healed(&mut self, amount: f64) {
        let before = self.hp;
        self.hp = self.max_hp.min(self.hp + amount);
        self.shown.heal += self.hp - before;
    }

    /// Lose `dmg`. Returns true when this blow killed.
    pub fn take(&mut self, dmg: f64) -> bool {
        if !self.alive || self.invuln > 0.0 {
            return false;
        }
        // Iron Skin, Blood Pact: a wizard's tomes change what reaches them.
        let dmg = dmg * self.mods.guard;
        self.shown.dmg += dmg.min(self.hp);
        self.hp -= dmg;
        if dmg > 2.5 {
            self.flash = self.flash.max(0.08);
        }
        // Phoenix Feather: a blow that would kill leaves 1 health instead.
        if self.hp <= 0.0 && self.winds > 0 {
            self.winds -= 1;
            self.hp = 1.0;
            self.invuln = self.invuln.max(1.5);
            return false;
        }
        if self.hp <= 0.0 {
            self.overkill = -self.hp;
            self.hp = 0.0;
            self.alive = false;
            self.dying = true;
            return true;
        }
        false
    }

    pub fn can_act(&self) -> bool {
        self.alive && self.frozen <= 0.0 && self.stun <= 0.0
    }

    pub fn move_speed(&self) -> f64 {
        let mut m = self.speed * self.haste;
        m *= 1.0 - (self.chill * 0.18).min(0.6);
        if self.slow > 0.0 {
            m *= 0.5;
        }
        if self.wading {
            m *= 0.72;
        }
        m
    }

    /// Whether a shot of radius `r` moving from `a` to `b` this tick touches
    /// this actor: the whole path, a little generous (the body you see is
    /// wider than the capsule, and shots fly above the ground).
    pub fn hit_by(&self, a: Vec3, b: Vec3, r: f64) -> bool {
        let p = glam::Vec2::new(self.pos.x, self.pos.z);
        let s = glam::Vec2::new(a.x, a.z);
        let e = glam::Vec2::new(b.x, b.z);
        let seg = e - s;
        let mut t = 0.0;
        if seg.length_squared() > 0.0001 {
            t = ((p - s).dot(seg) / seg.length_squared()).clamp(0.0, 1.0);
        }
        (p.distance(s + seg * t) as f64) < self.radius as f64 * 1.3 + r + 0.1
    }

    /// The timers that run down by themselves (the second half of
    /// tick_status; burning and poison are World::tick_status).
    pub fn cool(&mut self, dt: f64) {
        self.wet = (self.wet - dt).max(0.0);
        self.chill = (self.chill - 0.6 * dt).max(0.0);
        self.frozen = (self.frozen - dt).max(0.0);
        self.freeze_guard = (self.freeze_guard - dt).max(0.0);
        self.stun = (self.stun - dt).max(0.0);
        self.slow = (self.slow - dt).max(0.0);
        self.slick = (self.slick - dt).max(0.0);
        self.flash = (self.flash - dt).max(0.0);
        self.hurt_t = (self.hurt_t - dt).max(0.0);
        self.hurt_acc = (self.hurt_acc - dt * 2.0).max(0.0);
        self.invuln = (self.invuln - dt).max(0.0);
    }

    /// The death style Remains.style_for picks from the last hit.
    pub fn death_style(&self) -> crate::fx::Death {
        use crate::fx::Death::*;
        let e = self.last_el;
        let is = |l: &[El]| e.is_some_and(|x| l.contains(&x));
        if self.frozen > 0.0 || is(&[El::Frost, El::Ice]) {
            return Freeze;
        }
        if self.overkill > 22.0 && (e.is_none() || is(&[El::Earth, El::Magma, El::Mud])) {
            return Gore;
        }
        if self.burn > 0.0 || is(&[El::Fire, El::Magma, El::Steam]) {
            return Burn;
        }
        if is(&[El::Lightning, El::Storm]) {
            return Shock;
        }
        if e == Some(El::Void) {
            return Void;
        }
        if e == Some(El::Air) {
            return Blown;
        }
        if self.overkill > 22.0 {
            return Gore;
        }
        Fall
    }
}
