//! Plays a wizard for an AI seat by pressing the same buttons a player
//! would (bot.gd): it shoots at the nearest monster, swings at what comes
//! close, rolls away from bullets about to hit, and casts a recipe when it
//! has the mana.

use crate::El::{self, *};
use crate::balance as b;
use crate::pickups::Bullet;
use crate::queue;
use crate::rng::Rand;
use crate::run::State;
use crate::wizard::{NOVA_HOLD, Pad, Wizard, pair};
use crate::world::{Id, World, flat};
use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotForm {
    Cast,
    Nova,
    Plant,
    Enchant,
}

const RECIPES: [(&[El], BotForm, f64); 18] = [
    (&[Fire, Fire], BotForm::Cast, 2.0),
    (&[Earth, Fire], BotForm::Cast, 0.8),
    (&[Lightning, Lightning], BotForm::Cast, 1.6),
    (&[Water, Frost, Water, Frost], BotForm::Cast, 0.7),
    (&[Void, Fire], BotForm::Cast, 1.8),
    (&[Poison, Void], BotForm::Cast, 1.5),
    (&[Poison, Poison], BotForm::Plant, 0.0),
    (&[Earth, Earth], BotForm::Cast, 0.9),
    (&[Frost, Frost], BotForm::Cast, 1.5),
    (&[Water], BotForm::Cast, 1.2),
    (&[Water, Fire, Water, Fire], BotForm::Cast, 1.6),
    (&[Air, Lightning], BotForm::Cast, 1.4),
    (&[Earth, Earth], BotForm::Plant, 0.0),
    (&[Lightning], BotForm::Plant, 0.0),
    (&[Void, Earth], BotForm::Plant, 0.0),
    (&[Lightning, Lightning, Fire], BotForm::Plant, 0.0),
    (&[Fire, Fire, Fire], BotForm::Cast, 0.0),
    (&[Lightning, Lightning, Lightning], BotForm::Cast, 0.0),
];
const CLOSE: [&[El]; 4] = [
    &[Frost, Frost, Air],
    &[Fire, Fire, Earth],
    &[Earth, Earth, Air],
    &[Void, Lightning, Lightning],
];
const ENCHANTS: [&[El]; 3] = [&[Fire, Fire], &[Lightning, Lightning], &[Frost, Poison]];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Conjure,
    Cast,
    Hold,
    HoldRb,
    HoldRs,
    Rest,
}

#[derive(Clone, Debug)]
pub struct Bot {
    plan: Vec<El>,
    form: BotForm,
    hold: f64,
    press_t: f64,
    hold_t: f64,
    phase: Phase,
    think: f64,
    strafe: f32,
    /// For screenshots: always cast this recipe index.
    fixed_recipe: i32,
}

impl Bot {
    pub fn new(recipe: i32) -> Bot {
        Bot {
            plan: Vec::new(),
            form: BotForm::Cast,
            hold: 0.0,
            press_t: 0.0,
            hold_t: 0.0,
            phase: Phase::Idle,
            think: 0.0,
            strafe: 1.0,
            fixed_recipe: recipe,
        }
    }

    /// Roughly what the plan will cost; fusions make it a little cheaper.
    fn price(&self) -> f64 {
        let load = b::MANA_PER_LOAD * self.plan.len() as f64;
        if self.plan.len() >= queue::SIZE && self.form == BotForm::Cast {
            return load + b::OVERLOAD_COST;
        }
        load + b::MANA_PER_ELEMENT * self.plan.len() as f64
    }
}

fn press(pad: &mut Pad, e: El) {
    for bt in [Pad::A, Pad::B, Pad::X, Pad::Y] {
        if let Some(i) = pair(bt).iter().position(|&x| x == e) {
            pad.set(bt, true);
            pad.set(Pad::LB, i == 1);
            return;
        }
    }
}

impl World {
    /// The bot's turn at the controller (Bot.think).
    pub fn bot_think(&mut self, id: Id, w: &mut Wizard, dt: f64) {
        let Some(mut bot) = w.bot.take() else { return };
        self.bot_tick(id, w, &mut bot, dt);
        w.bot = Some(bot);
    }

    fn bot_tick(&mut self, id: Id, w: &mut Wizard, bot: &mut Bot, dt: f64) {
        let pad = &mut w.pad;
        for bt in [
            Pad::A,
            Pad::B,
            Pad::X,
            Pad::Y,
            Pad::LB,
            Pad::LT,
            Pad::RB,
            Pad::RS,
            Pad::UP,
            Pad::LS,
        ] {
            pad.set(bt, false);
        }
        pad.fire = false;
        if !self.alive(id) {
            pad.mv = Vec2::ZERO;
            return;
        }
        let me = self.pos(id);
        let enemy = self.nearest_enemy(me);
        let downed = self.downed_friend(id, me);
        bot.think -= dt;
        if bot.think <= 0.0 {
            bot.think = 1.2;
            if self.rng.f() < 0.3 {
                bot.strafe = -bot.strafe;
            }
        }
        let mut mv = Vec3::ZERO;
        let mut aim = Vec3::ZERO;
        let tome = if enemy.is_none() {
            self.tome_for(me, w)
        } else {
            None
        };
        if enemy.is_none() && (tome.is_some() || self.run.state == State::Cleared) {
            bot.phase = Phase::Idle;
            w.pad.set(Pad::RT, false);
            let mut goal = self.run.portal.unwrap_or(me);
            if let Some(t) = tome {
                goal = self.run.altars[t].pos;
            }
            mv = goal - me;
            w.pad.mv = if mv.length() > 0.3 {
                Vec2::new(mv.x, mv.z).normalize()
            } else {
                Vec2::ZERO
            };
            return;
        }
        // Nothing to fight: walk on along the path to the next monsters.
        if self.run.state == State::Fight && enemy.is_none() && downed.is_none() {
            let to_goal = flat(self.walk_goal(me) - me);
            mv = to_goal.normalize_or_zero() * 0.8;
        }
        // A friend down: go and stand at the grave to raise them.
        let target_pos = downed.or(enemy).map(|t| self.pos(t)).unwrap_or(me);
        let to = flat(target_pos - me);
        let d = to.length();
        if enemy.is_some() || downed.is_some() {
            aim = to.normalize_or_zero();
            if downed.is_some() {
                mv = if d > 0.8 {
                    to.normalize_or_zero()
                } else {
                    Vec3::ZERO
                };
                if let Some(e) = enemy {
                    aim = flat(self.pos(e) - me).normalize_or_zero();
                }
            } else {
                let n = to.normalize_or_zero();
                if d > 7.0 {
                    mv = n;
                } else if d < 4.5 {
                    mv = -n;
                }
                mv += n.cross(Vec3::Y) * bot.strafe * 0.6;
            }
        }
        // The practice strip's urns: shoot them the way a player would.
        let urn = if enemy.is_none() && downed.is_none() {
            self.next_urn(me)
        } else {
            None
        };
        if let Some(u) = urn {
            let tu = flat(u - me);
            aim = tu.normalize_or_zero();
            mv = if tu.length() > 7.0 {
                tu.normalize_or_zero() * 0.8
            } else {
                Vec3::ZERO
            };
        }
        // Low on mana: go and take the souls lying around.
        if let Some(s) = self.nearest_of(&self.souls(), me, 9.0) {
            if w.mana < w.mana_max() * 0.75 && downed.is_none() {
                mv = flat(s - me).normalize_or_zero() * 1.2;
            }
        }
        // Hurt: go for the nearest health globe.
        if let Some(g) = self.nearest_of(&self.globes(), me, 14.0) {
            let a = self.a(id);
            if a.hp < a.max_hp * 0.7 && downed.is_none() {
                drop(a);
                mv = flat(g - me).normalize_or_zero() * 1.2;
            }
        }
        let pad = &mut w.pad;
        pad.mv = Vec2::new(mv.x, mv.z).clamp_length_max(1.0);
        pad.aim = Vec2::new(aim.x, aim.z);
        let de = enemy
            .map(|e| me.distance(self.pos(e)))
            .unwrap_or(f32::INFINITY);
        pad.fire = (enemy.is_some() && de < 16.0) || urn.is_some();
        let threat = self.bullet_threat(id, me);
        if threat != Vec3::ZERO && self.rng.f() < 0.12 {
            w.pad.set(Pad::LT, true);
            w.pad.mv = Vec2::new(threat.x, threat.z);
        } else if bot.phase != Phase::HoldRb
            && enemy.is_some()
            && (d < 2.4 || self.bullet_in_reach(id, me))
            && self.rng.f() < 0.5
        {
            w.pad.set(Pad::RB, true);
        }
        match bot.phase {
            Phase::Idle => {
                w.pad.set(Pad::RT, false);
                if enemy.is_some() && d < 12.0 {
                    self.bot_choose(w, bot, d);
                    if w.mana >= bot.price() {
                        bot.phase = Phase::Conjure;
                        bot.press_t = 0.0;
                    } else {
                        bot.plan.clear();
                    }
                }
            }
            Phase::Conjure => {
                bot.press_t -= dt;
                if bot.press_t <= 0.0 {
                    if bot.plan.is_empty() {
                        bot.phase = Phase::Cast;
                    } else {
                        let e = bot.plan.remove(0);
                        press(&mut w.pad, e);
                        bot.press_t = 0.13;
                    }
                }
            }
            Phase::Cast => match bot.form {
                BotForm::Cast => {
                    w.pad.set(Pad::RT, true);
                    bot.hold_t = bot.hold;
                    bot.phase = Phase::Hold;
                }
                BotForm::Nova => {
                    w.pad.set(Pad::RB, true);
                    bot.hold_t = NOVA_HOLD + 0.1;
                    bot.phase = Phase::HoldRb;
                }
                BotForm::Plant => {
                    w.pad.set(Pad::LS, true);
                    bot.phase = Phase::Idle;
                }
                BotForm::Enchant => {
                    // Held: a tap would shake the staff empty.
                    w.pad.set(Pad::RS, true);
                    bot.hold_t = crate::wizard::ENCHANT_HOLD + 0.1;
                    bot.phase = Phase::HoldRs;
                }
            },
            Phase::Hold => {
                bot.hold_t -= dt;
                w.pad.set(Pad::RT, bot.hold_t > 0.0);
                if bot.hold_t <= 0.0 {
                    bot.phase = Phase::Rest;
                    bot.hold_t = self.rng.range(0.2, 0.6) as f64;
                }
            }
            Phase::HoldRb => {
                bot.hold_t -= dt;
                w.pad.set(Pad::RB, bot.hold_t > 0.0);
                if bot.hold_t <= 0.0 {
                    bot.phase = Phase::Idle;
                }
            }
            Phase::HoldRs => {
                bot.hold_t -= dt;
                w.pad.set(Pad::RS, bot.hold_t > 0.0);
                if bot.hold_t <= 0.0 {
                    bot.phase = Phase::Idle;
                }
            }
            Phase::Rest => {
                w.pad.set(Pad::RT, false);
                bot.hold_t -= dt;
                if bot.hold_t <= 0.0 {
                    bot.phase = Phase::Idle;
                }
            }
        }
    }

    fn bot_choose(&mut self, w: &Wizard, bot: &mut Bot, d: f32) {
        if bot.fixed_recipe >= 0 {
            let r = RECIPES[bot.fixed_recipe as usize % RECIPES.len()];
            bot.plan = r.0.to_vec();
            bot.form = r.1;
            bot.hold = r.2.max(0.3);
            return;
        }
        if w.enchant_time <= 0.0 && self.rng.f() < 0.15 {
            bot.plan = ENCHANTS[self.rng.pick(ENCHANTS.len())].to_vec();
            bot.form = BotForm::Enchant;
            return;
        }
        if d < 3.0 && self.rng.f() < 0.6 {
            bot.plan = CLOSE[self.rng.pick(CLOSE.len())].to_vec();
            bot.form = BotForm::Nova;
            return;
        }
        // Traps and sentries now and then, not every cast.
        let mut pool = Vec::new();
        for r in &RECIPES[..RECIPES.len() - 2] {
            if r.1 != BotForm::Plant || self.rng.f() < 0.25 {
                pool.push(*r);
            }
        }
        let r2 = if self.rng.f() < 0.95 {
            pool[self.rng.pick(pool.len())]
        } else {
            RECIPES[RECIPES.len() - 1 - self.rng.pick(2)]
        };
        bot.plan = r2.0.to_vec();
        bot.form = r2.1;
        bot.hold = r2.2 * self.rng.range(0.8, 1.3) as f64;
    }

    /// Which way to roll when a bullet is about to hit, or zero.
    fn bullet_threat(&self, id: Id, me: Vec3) -> Vec3 {
        let team = self.team(id);
        for b in self.bullets() {
            let bu = self.ecs.get::<&Bullet>(b).unwrap();
            if bu.team == team {
                continue;
            }
            let to = flat(me - bu.pos);
            if to.length() > 2.6 || bu.vel.dot(to) <= 0.0 {
                continue;
            }
            let side = bu.vel.normalize_or_zero().cross(Vec3::Y);
            return if side.dot(to) > 0.0 { side } else { -side };
        }
        Vec3::ZERO
    }

    fn bullet_in_reach(&self, id: Id, me: Vec3) -> bool {
        let team = self.team(id);
        self.bullets().into_iter().any(|b| {
            let bu = self.ecs.get::<&Bullet>(b).unwrap();
            bu.team != team && bu.pos.distance(me + Vec3::Y) < 2.2
        })
    }

    /// Where the nearest of `ids` lies, within `within`.
    fn nearest_of(&self, ids: &[Id], me: Vec3, within: f32) -> Option<Vec3> {
        let mut best = None;
        let mut bd = within;
        for &i in ids {
            let p = self.fx_pos(i);
            let d = me.distance(p);
            if d < bd {
                bd = d;
                best = Some(p);
            }
        }
        best
    }

    fn nearest_enemy(&self, me: Vec3) -> Option<Id> {
        let mut best = None;
        let mut bd = f32::INFINITY;
        for &e in &self.enemies {
            if !self.alive(e) {
                continue;
            }
            let d = me.distance(self.pos(e));
            if d < bd {
                bd = d;
                best = Some(e);
            }
        }
        best
    }

    fn downed_friend(&self, id: Id, me: Vec3) -> Option<Id> {
        self.wizards
            .iter()
            .copied()
            .find(|&o| o != id && !self.alive(o) && self.pos(o).distance(me) < 12.0)
    }

    /// Where a globe or a soul is.
    pub fn fx_pos(&self, id: Id) -> Vec3 {
        if let Ok(g) = self.ecs.get::<&crate::pickups::Globe>(id) {
            return g.pos;
        }
        if let Ok(s) = self.ecs.get::<&crate::pickups::Soul>(id) {
            return s.pos;
        }
        Vec3::ZERO
    }
}
