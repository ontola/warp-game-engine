//! A player's wizard (wizard.gd). Twin-stick at heart: the left stick walks,
//! the right stick aims and shoots staff bolts. Face buttons load elements
//! into the queue for a little mana, and the queue is ammunition: every bolt
//! carries its mix, and the oldest element burns out after a few bolts. RT
//! spends what is left at once as a spell, for mana per element.
//!
//!   A Poison / Void   B Fire / Frost     X Water / Air    Y Lightning / Earth
//!   (hold LB for the second of each pair)
//!   right stick  aim and shoot bolts
//!   RB  sword swing, knocks bullets back; hold and let go: burst the queue
//!   LT  dodge roll, monsters cannot touch you while you roll
//!   RT  cast the queue ahead: it winds up first, then goes on release
//!   LS click  plant the queue at your feet: a trap, or a sentry when full
//!   RS click  enchant the sword
//!   D-pad up  conjure your last spell again
//!   D-pad left / right  tap to conjure a saved spell, hold to save the queue

use crate::actor::Actor;
use crate::balance as b;
use crate::bot::Bot;
use crate::fx::{self, C, Ev};
use crate::grammar::compose;
use crate::queue::{self, Added, Queue};
use crate::relics::Mods;
use crate::rng::Rand;
use crate::spells::{Bolt, BoltSpell, OVERLOAD_REACH, WAVE_RUN, WAVE_START, bases, nova_n};
use crate::world::{Id, Seat, Settings, WIZARDS, World, flat, lerp_angle, signed_angle, yaw_rot};
use crate::{El, Form, Shape, Spell};
use glam::{Mat3, Vec2, Vec3};

pub const REVIVE_NEED: f64 = 40.0;
pub const REVIVE_RATE: f64 = 14.0;
pub const REVIVE_REACH: f32 = 1.9;
pub const SAVE_HOLD: f64 = 0.6;
pub const SWING_COOLDOWN: f64 = 0.34;
pub const SWING_RECOVER: f64 = 0.7;
/// The sword's reach by default (knob `swing_reach`).
pub const SWING_REACH: f32 = 1.7;

/// The sword's reach now.
pub fn swing_reach() -> f32 {
    crate::knobs::swing_reach() as f32
}
pub const SWING_ARC: f32 = 1.3;
pub const NOVA_HOLD: f64 = 0.45;
pub const ROLL_TIME: f64 = 0.5;
pub const ROLL_SPEED: f64 = 12.5;
pub const ROLL_IFRAMES: f64 = 0.3;
pub const ROLL_RECOVER: f64 = 0.15;
pub const ROLL_COOLDOWN: f64 = 0.95;
pub const LEASH: f32 = 15.0;
pub const MAX_TRAPS: usize = 3;
pub const SWORD_MAX: f32 = 3.0;
pub const GEM_WARM: C = fx::rgb(1.0, 0.8, 0.55);

/// Each wizard starts with two face elements that do not mix, a different
/// pair per player, then the other two, then LB and its elements one by one.
pub const ELEMENT_ORDER: [[El; 4]; 4] = [
    [El::Fire, El::Lightning, El::Water, El::Poison],
    [El::Water, El::Poison, El::Lightning, El::Fire],
    [El::Lightning, El::Water, El::Fire, El::Poison],
    [El::Poison, El::Fire, El::Water, El::Lightning],
];
pub const LB_ORDER: [El; 4] = [El::Frost, El::Earth, El::Air, El::Void];
/// The unlock ladder, one step at a level's first shrine and one at its
/// boss, so it runs over five of the six levels (Joep: not all at once).
/// By step: the mana you hold, the queue's slots, the spell forms, how many
/// of your four base elements and of the four LB elements are open.
pub const STAGES: usize = 10;
pub const STAGE_MANA: [f64; STAGES] = [50.0, 50.0, 60.0, 65.0, 70.0, 75.0, 80.0, 85.0, 90.0, 100.0];
pub const STAGE_SLOTS: [usize; STAGES] = [1, 1, 2, 2, 2, 2, 3, 3, 3, 3];
const CAST: &[Form] = &[Form::Cast];
const NOVA: &[Form] = &[Form::Cast, Form::Nova];
const PLANT: &[Form] = &[Form::Cast, Form::Nova, Form::Plant];
const ENCHANT: &[Form] = &[Form::Cast, Form::Nova, Form::Plant, Form::Enchant];
pub const STAGE_FORMS: [&[Form]; STAGES] = [CAST, CAST, NOVA, NOVA, PLANT, PLANT, PLANT, ENCHANT, ENCHANT, ENCHANT];
const STAGE_BASE: [i32; STAGES] = [2, 3, 3, 4, 4, 4, 4, 4, 4, 4];
const STAGE_LB: [i32; STAGES] = [0, 0, 0, 0, 0, 0, 0, 1, 2, 4];
pub const ALL_UNLOCKED: i32 = 99;

/// One controller, read the same way whatever it is (pad.gd).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pad {
    pub mv: Vec2,
    pub aim: Vec2,
    pub held: u32,
    pub before: u32,
    /// Shooting staff bolts: the right stick pushed far enough.
    pub fire: bool,
}

impl Pad {
    pub const A: u32 = 0;
    pub const B: u32 = 1;
    pub const X: u32 = 2;
    pub const Y: u32 = 3;
    pub const LB: u32 = 4;
    pub const RB: u32 = 5;
    pub const BACK: u32 = 6;
    pub const START: u32 = 7;
    pub const LS: u32 = 8;
    pub const RS: u32 = 9;
    pub const UP: u32 = 10;
    pub const DOWN: u32 = 11;
    pub const LEFT: u32 = 12;
    pub const RIGHT: u32 = 13;
    pub const LT: u32 = 14;
    pub const RT: u32 = 15;

    pub fn down(&self, b: u32) -> bool {
        self.held & (1 << b) != 0
    }

    pub fn pressed(&self, b: u32) -> bool {
        self.held & (1 << b) != 0 && self.before & (1 << b) == 0
    }

    pub fn released(&self, b: u32) -> bool {
        self.held & (1 << b) == 0 && self.before & (1 << b) != 0
    }

    pub fn set(&mut self, b: u32, on: bool) {
        if on {
            self.held |= 1 << b;
        } else {
            self.held &= !(1 << b);
        }
    }

    /// The stick dead zone (Pad._dead).
    pub fn dead(v: Vec2) -> Vec2 {
        let l = v.length();
        if l < 0.22 {
            return Vec2::ZERO;
        }
        v / l * ((l - 0.22) / 0.7).clamp(0.0, 1.0)
    }
}

/// The face buttons' pairs of elements.
pub fn pair(button: u32) -> [El; 2] {
    match button {
        Pad::A => crate::el::PAIR_A,
        Pad::B => crate::el::PAIR_B,
        Pad::X => crate::el::PAIR_X,
        _ => crate::el::PAIR_Y,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Wizard {
    pub seat: usize,
    pub name: String,
    pub robe: C,
    pub player_id: String,
    /// Unlocks earned (-1: not even the practice strip), and which element
    /// order is theirs.
    pub unlocks: i32,
    pub element_order: usize,
    /// Just learned, for the HUD to make their buttons pulse.
    pub learned: Vec<El>,
    pub learned_t: f64,
    /// What the controller says this tick, and what it said before.
    pub pad: Pad,
    /// The app's latest reading of a human player's controller.
    pub input: Pad,
    pub bot: Option<Bot>,
    pub queue: Queue,
    pub aim_dir: Vec3,
    pub relics: Vec<usize>,
    pub saved: [Vec<El>; 2],
    pub last_cast: Vec<El>,
    pub bolt_el: Option<El>,
    pub ammo: i32,
    pub mana: f64,
    pub no_mana: f64,
    /// The stream running from the staff.
    pub channel: Option<Id>,
    /// The spell winding up on RT, how long so far and how long it needs.
    pub charging: Option<Box<Spell>>,
    pub charge_t: f64,
    pub charge_need: f64,
    /// Where a winding-up spell would land (the charge mark).
    pub mark: Option<Vec3>,
    bolt_t: f64,
    pub roll_t: f64,
    roll_cd: f64,
    pub roll_dir: Vec3,
    roll_recover: f64,
    pub rb_t: f64,
    save_t: [f64; 2],
    pub revive: f64,
    pub swing_t: f64,
    pub swing_cd: f64,
    pub swing_dir: Vec3,
    pub cast_anim: f64,
    pub enchant: Option<Box<Spell>>,
    pub enchant_time: f64,
    pub enchant_store: f64,
    enchant_full: f64,
    pub enchant_power: f64,
    traps: Vec<Id>,
    sentries: Vec<Id>,
    /// The grave, once fallen, and how long it has stood.
    pub tomb: Option<Vec3>,
    pub tomb_t: f64,
    /// Shrines this wizard took a tome from ("biome_k").
    pub took: Vec<String>,
    /// For the practice strip: rolled and knocked a bullet back.
    pub rolled: bool,
    pub deflected: bool,
    /// The group's stage and this wizard's unlock level, kept in step by
    /// World::unlocks_changed.
    pub stage: usize,
    pub level: i32,
    /// The pose that places the staff's gem: body turn, staff tilt, reach.
    pub body_yaw: f32,
    pub staff_rx: f32,
    pub staff_z: f32,
    /// The gem's bloom from the last shot (1 just fired) and its colour.
    pub bloom: f32,
    pub flash_c: C,
}

impl Wizard {
    pub fn make(i: usize, s: &Seat, robe: C, settings: &Settings) -> (Actor, Wizard) {
        let mut a = Actor::new(WIZARDS, Vec3::ZERO, 100.0, 0.45, crate::knobs::wizard_speed());
        a.wizard = true;
        a.freeze_time = 0.9;
        a.freeze_immunity = 4.0;
        let w = Wizard {
            seat: i,
            name: s.name.clone(),
            robe,
            player_id: s.player_id.clone(),
            unlocks: s.unlocks,
            element_order: i,
            bot: if s.bot {
                Some(Bot::new(settings.recipe))
            } else {
                None
            },
            queue: Queue::new(),
            aim_dir: Vec3::NEG_Z,
            mana: b::MANA_MAX,
            charge_need: 0.5,
            rb_t: -1.0,
            stage: 3,
            level: ALL_UNLOCKED,
            staff_z: 0.18,
            flash_c: GEM_WARM,
            swing_dir: Vec3::NEG_Z,
            ..Default::default()
        };
        (a, w)
    }

    pub fn mods(&self) -> Mods {
        Mods::of(&self.relics)
    }

    pub fn mana_max(&self) -> f64 {
        STAGE_MANA[self.stage] + self.mods().mana
    }

    pub fn form_open(&self, f: Form) -> bool {
        STAGE_FORMS[self.stage].contains(&f)
    }

    pub fn lb_open(&self) -> bool {
        self.level >= 0 && STAGE_LB[(self.level as usize).min(STAGES - 1)] > 0
    }

    pub fn element_open(&self, e: El) -> bool {
        let u = self.level;
        if u < 0 {
            return false;
        }
        let order = ELEMENT_ORDER[self.element_order % ELEMENT_ORDER.len()];
        let step = (u as usize).min(STAGES - 1);
        if let Some(i) = order.iter().position(|&x| x == e) {
            return (i as i32) < STAGE_BASE[step];
        }
        LB_ORDER
            .iter()
            .position(|&x| x == e)
            .is_some_and(|i| (i as i32) < STAGE_LB[step])
    }

    pub fn open_elements(&self) -> Vec<El> {
        crate::el::BASE
            .iter()
            .copied()
            .filter(|&e| self.element_open(e))
            .collect()
    }

    /// The queue and mana follow the stage when it changes.
    pub fn refresh(&mut self, group_stage: usize, s: &Settings) {
        self.level = if s.forced_unlocks >= 0 {
            s.forced_unlocks
        } else if self.bot.is_some() || s.unlock_all {
            ALL_UNLOCKED
        } else {
            self.unlocks
        };
        self.stage = if self.bot.is_some() && s.forced_unlocks < 0 {
            STAGES - 1
        } else {
            group_stage
        };
        self.queue.size = STAGE_SLOTS[self.stage];
        if self.queue.slots.len() > self.queue.size {
            self.queue.slots.truncate(self.queue.size);
            self.ammo = self.full_ammo();
        }
        self.mana = self.mana.min(self.mana_max());
    }

    pub fn add_mana(&mut self, amount: f64) {
        self.mana = self.mana_max().min(self.mana + amount);
    }

    /// What a spell costs: per element in it, an overload a fixed sum.
    pub fn cost(s: &Spell) -> f64 {
        match s.shape {
            Shape::Overload => b::OVERLOAD_COST,
            Shape::Sentry => b::SENTRY_COST,
            Shape::Staff | Shape::Stomp | Shape::Fizzle => 0.0,
            _ => b::MANA_PER_ELEMENT * s.power as f64,
        }
    }

    /// What one bolt carries: every loaded element, a little stronger for
    /// each copy, split across the mix.
    pub fn bolt_spell(&self) -> BoltSpell {
        if self.queue.is_empty() {
            return BoltSpell::default();
        }
        let mut counts: Vec<(El, u8)> = Vec::new();
        for &e in &self.queue.slots {
            match counts.iter_mut().find(|c| c.0 == e) {
                Some(c) => c.1 += 1,
                None => counts.push((e, 1)),
            }
        }
        let front = self.queue.slots[0];
        let scales = b::bolt_scales(&counts, front, self.mods().bolt);
        BoltSpell {
            counts,
            primary: Some(front),
            scales,
        }
    }

    pub fn shot_rate(&self) -> f64 {
        if self.enchant_time > 0.0 {
            if let Some(e) = &self.enchant {
                return b::flight(e.primary).rate;
            }
        }
        b::flight(self.queue.slots.first().copied()).rate
    }

    pub fn full_ammo(&self) -> i32 {
        self.queue.slots.first().map(|&e| b::ammo(e)).unwrap_or(0)
    }

    pub fn running(&self, w: &World) -> bool {
        self.channel.is_some_and(|c| w.ecs.contains(c))
    }

    /// Channelling a stream or winding up keeps the staff busy.
    pub fn busy(&self, w: &World) -> bool {
        self.running(w) || self.charging.is_some() || self.roll_t > 0.0
    }

    /// How far an enchanted swing reaches: the blade grows with every
    /// element laid on it.
    pub fn enchant_reach(&self) -> f32 {
        let n: u32 = self
            .enchant
            .as_ref()
            .map(|e| e.counts.iter().map(|c| c.1 as u32).sum())
            .unwrap_or(0);
        if n == 0 {
            return swing_reach();
        }
        swing_reach()
            + (SWORD_MAX - swing_reach()) * (n.min(queue::SIZE as u32) as f32) / queue::SIZE as f32
    }

    /// How much of the enchantment is left, 1 to 0.
    pub fn enchant_left(&self) -> f64 {
        let Some(e) = &self.enchant else { return 0.0 };
        if self.enchant_time <= 0.0 {
            return 0.0;
        }
        let by_time = self.enchant_time / e.n.duration.max(0.1);
        by_time.min(self.enchant_store / self.enchant_full.max(0.001))
    }

    /// How many of the queued elements, from the front, a `form` cast can
    /// pay for now.
    pub fn affordable(&self, form: Form) -> usize {
        let mods = self.mods();
        let slots = &self.queue.slots;
        for n in (1..=slots.len()).rev() {
            let s = compose(&slots[..n], form, &mods);
            if self.mana >= Wizard::cost(&s) * mods.cost - 0.01 {
                return n;
            }
        }
        0
    }

    /// What `form` would cast now: short of mana, the biggest spell from the
    /// front of the queue you can pay for.
    pub fn preview(&self, form: Form) -> Spell {
        let n = self.affordable(form);
        let slots = &self.queue.slots;
        compose(if n > 0 { &slots[..n] } else { slots }, form, &self.mods())
    }

    /// Unlocks earned up to `n` (the caller tells the group).
    pub fn learn(&mut self, n: i32) -> Vec<El> {
        if n <= self.unlocks {
            return Vec::new();
        }
        let before = self.open_elements();
        self.unlocks = n;
        Vec::from_iter(before)
    }

    pub fn enchant_end(&mut self) {
        self.enchant = None;
        self.enchant_time = 0.0;
    }

    fn spend_enchant(&mut self, dmg: f64) {
        self.enchant_store -= dmg;
        if self.enchant_store <= 0.0 {
            self.enchant_end();
        }
    }
}

/// Where the gem sits: the staff turns about the hand at the end of the
/// right sleeve, the body turns to the aim.
pub fn staff_tip(pos: Vec3, w: &Wizard) -> Vec3 {
    let staff = Vec3::new(0.42, 0.75, w.staff_z)
        + Mat3::from_rotation_x(w.staff_rx) * Vec3::new(0.0, 1.12, 0.0);
    pos + Mat3::from_rotation_y(w.body_yaw) * staff
}

/// How fast the roll goes `k` of the way through it: a quick push off,
/// then an even ease to a stop.
pub fn roll_speed(k: f64) -> f64 {
    if k < 0.15 {
        return 0.4 + 0.6 * k / 0.15;
    }
    (1.0 - (k - 0.15) / 0.85).clamp(0.0, 1.0)
}

impl World {
    pub fn staff_tip(&self, id: Id) -> Vec3 {
        let w = self.wiz(id);
        staff_tip(self.pos(id), &w)
    }

    /// A wizard's tick (Wizard._physics_process). The Wizard component is
    /// lifted out while it runs; the Actor stays in the world.
    pub fn wizard_step(&mut self, id: Id, dt: f64) {
        let mut w = std::mem::take(&mut *self.wiz_mut(id));
        self.wizard_tick(id, &mut w, dt);
        *self.wiz_mut(id) = w;
    }

    fn wizard_tick(&mut self, id: Id, w: &mut Wizard, dt: f64) {
        self.tick_status(id, dt);
        w.pad.before = w.pad.held;
        if w.bot.is_some() {
            self.bot_think(id, w, dt);
        } else {
            let i = w.input.clone();
            w.pad.held = i.held;
            w.pad.mv = i.mv;
            w.pad.aim = i.aim;
            w.pad.fire = i.fire;
        }
        if w.swing_t > 0.0 {
            self.deflect_sweep(id, w);
        }
        w.swing_t = (w.swing_t - dt).max(0.0);
        if w.swing_cd > 0.0 {
            w.swing_cd -= dt;
            if w.swing_cd <= 0.0 {
                let at = staff_tip(self.pos(id), w);
                self.burst(at, fx::rgb(1.0, 0.9, 0.6), 8, 1.5, 0.12, None);
            }
        }
        w.cast_anim = (w.cast_anim - dt).max(0.0);
        w.roll_cd = (w.roll_cd - dt).max(0.0);
        w.no_mana = (w.no_mana - dt).max(0.0);
        if !self.alive(id) {
            self.drive(id, Vec3::ZERO, dt);
            self.raise(id, w, dt);
            return;
        }
        w.tomb_t = 0.0;
        w.mana = w.mana_max().min(w.mana + crate::knobs::mana_regen() * w.mods().regen * dt);
        w.learned_t = (w.learned_t - dt).max(0.0);
        if w.enchant_time > 0.0 {
            w.enchant_time -= dt;
            if w.enchant_time <= 0.0 {
                w.enchant_end();
            }
        }
        w.bolt_el = w.queue.slots.first().copied();
        self.read_input(id, w, dt);
        if w.roll_t > 0.0 {
            w.roll_t -= dt;
            let v = w.roll_dir * (crate::knobs::roll_speed() * roll_speed(1.0 - w.roll_t / ROLL_TIME)) as f32;
            self.drive(id, v, dt);
            if w.roll_t <= 0.0 {
                w.roll_recover = ROLL_RECOVER;
                self.am(id).blocks = true;
            }
            self.pose(w, dt);
            return;
        }
        w.roll_recover = (w.roll_recover - dt).max(0.0);
        let mv = Vec3::new(w.pad.mv.x, 0.0, w.pad.mv.y);
        let mut spd = self.a(id).move_speed();
        if w.running(self) {
            spd *= 0.55;
        } else if w.charging.is_some() {
            spd *= 0.6;
        }
        if w.roll_recover > 0.0 {
            spd *= 0.7;
        }
        let v = self.leash(id, mv) * spd as f32;
        self.drive(id, v, dt);
        self.pose(w, dt);
    }

    /// Couch co-op shares one screen: nobody walks further than LEASH from
    /// another wizard.
    fn leash(&self, id: Id, mut mv: Vec3) -> Vec3 {
        let p = self.pos(id);
        for &o in &self.wizards {
            if o == id || !self.alive(o) {
                continue;
            }
            let away = flat(p - self.pos(o));
            if away.length() > LEASH && mv.dot(away) > 0.0 {
                let n = away.normalize();
                mv -= n * mv.dot(n);
            }
        }
        mv
    }

    /// The parts of Wizard._visuals that place the staff.
    fn pose(&mut self, w: &mut Wizard, dt: f64) {
        let dt = dt as f32;
        let target = w.aim_dir.x.atan2(w.aim_dir.z);
        w.body_yaw = lerp_angle(w.body_yaw, target, 14.0 * dt);
        w.bloom = (w.bloom - dt * 7.0).max(0.0);
        let lift = if w.cast_anim > 0.0
            || w.channel.is_some_and(|c| self.ecs.contains(c))
            || w.charging.is_some()
        {
            1.0
        } else {
            0.0
        };
        let point = if w.charging.is_some() { 1.45 } else { 0.95 };
        w.staff_rx += (point * lift - w.staff_rx) * 12.0 * dt;
        w.staff_z += (0.18 + 0.25 * lift - w.staff_z) * 12.0 * dt;
        if w.swing_t > 0.0 {
            let sweep = 1.0 - (w.swing_t / SWING_COOLDOWN) as f32;
            let e = 1.0 - (1.0 - (sweep * 1.6).min(1.0)).powi(3);
            w.body_yaw = target + (-0.6 + 1.2 * e);
        }
    }

    fn read_input(&mut self, id: Id, w: &mut Wizard, dt: f64) {
        if w.pad.aim.length() > 0.3 {
            w.aim_dir = Vec3::new(w.pad.aim.x, 0.0, w.pad.aim.y).normalize();
        } else if w.pad.mv.length() > 0.3
            && !w.channel.is_some_and(|c| self.ecs.contains(c))
            && w.charging.is_none()
        {
            w.aim_dir = Vec3::new(w.pad.mv.x, 0.0, w.pad.mv.y).normalize();
        }
        if !self.a(id).can_act() {
            return;
        }
        // Locked elements, and LB before it is learned, do nothing.
        let flip = w.pad.down(Pad::LB) && w.lb_open();
        for bt in [Pad::A, Pad::B, Pad::X, Pad::Y] {
            if w.pad.pressed(bt) {
                let e = pair(bt)[flip as usize];
                if w.element_open(e) {
                    self.conjure(id, w, e);
                }
            }
        }
        if w.pad.pressed(Pad::LT) {
            self.roll(id, w);
        }
        if w.roll_t > 0.0 {
            return;
        }
        // A spell winds up, then goes once it is ready and RT is let go.
        if w.charging.is_some() {
            w.charge_t += dt;
            if w.charge_t >= w.charge_need && !w.pad.down(Pad::RT) {
                self.release(id, w);
            } else if let Some(s) = w.charging.clone() {
                w.mark = self.cast_mark(id, w, &s);
            }
        }
        if w.pad.pressed(Pad::RT) {
            self.cast(id, w, Form::Cast);
        }
        if w.pad.pressed(Pad::RB) {
            self.melee(id, w);
            w.rb_t = 0.0;
        } else if w.pad.down(Pad::RB) && w.rb_t >= 0.0 {
            w.rb_t += dt;
        }
        if w.pad.released(Pad::RB) {
            if w.rb_t >= NOVA_HOLD && w.form_open(Form::Nova) {
                self.cast(id, w, Form::Nova);
            }
            w.rb_t = -1.0;
        }
        if w.pad.pressed(Pad::LS) && w.form_open(Form::Plant) {
            self.cast(id, w, Form::Plant);
        }
        if w.pad.pressed(Pad::RS) && w.form_open(Form::Enchant) {
            self.cast(id, w, Form::Enchant);
        }
        w.bolt_t -= dt;
        // Swinging the sword stops the bolts for a moment.
        if w.pad.fire && w.bolt_t <= 0.0 && !w.busy(self) && w.swing_t <= 0.0 {
            w.bolt_t = 1.0 / w.shot_rate();
            self.shoot(id, w);
        }
        if w.pad.pressed(Pad::UP) && !w.last_cast.is_empty() {
            let s = w.last_cast.clone();
            self.load_slots(id, w, &s);
        }
        for (k, bt) in [Pad::LEFT, Pad::RIGHT].into_iter().enumerate() {
            if w.pad.down(bt) {
                w.save_t[k] += dt;
                if w.save_t[k] >= SAVE_HOLD && w.save_t[k] - dt < SAVE_HOLD && !w.queue.is_empty() {
                    w.saved[k] = w.queue.slots.clone();
                    // Saved: a little gold sparkle off the gem, no words.
                    let at = staff_tip(self.pos(id), w);
                    self.burst(at, fx::rgb(1.0, 0.85, 0.4), 10, 2.0, 0.12, None);
                }
            } else if w.pad.released(bt) {
                if w.save_t[k] < SAVE_HOLD && !w.saved[k].is_empty() {
                    let s = w.saved[k].clone();
                    self.load_slots(id, w, &s);
                }
                w.save_t[k] = 0.0;
            }
        }
    }

    fn muzzle(w: &mut Wizard, c: C) {
        w.flash_c = c;
        w.bloom = 1.0;
    }

    fn shoot(&mut self, id: Id, w: &mut Wizard) {
        let dir = yaw_rot(w.aim_dir, self.rng.range(-0.03, 0.03));
        // Fired steadily, the staff stays up instead of bobbing.
        w.cast_anim = w.cast_anim.max(1.0 / w.shot_rate() + 0.1);
        if w.enchant_time > 0.0 && w.enchant.is_some() {
            self.enchanted_shot(id, w, dir);
            return;
        }
        let bs = w.bolt_spell();
        let at = staff_tip(self.pos(id), w) + dir * 0.1;
        let c = bs.primary.map(fx::el_c).unwrap_or(fx::WHITE);
        self.fire_bolt(Bolt::new(id, bs, dir, at, Vec::new(), 1.0, true));
        World::muzzle(w, c);
        if !w.queue.is_empty() {
            w.ammo -= 1;
            if w.ammo <= 0 {
                let gone = w.queue.slots.remove(0);
                w.ammo = w.full_ammo();
                let at = self.pos(id) + Vec3::Y * 2.1;
                self.burst(at, fx::el_c(gone), 8, 2.0, 0.2, None);
            }
        }
        self.count("shots");
    }

    /// An enchanted staff shoots its element's own way: fire bolts burst,
    /// frost shards pierce, water sprays three, lightning jumps on.
    fn enchanted_shot(&mut self, id: Id, w: &mut Wizard, dir: Vec3) {
        let en = w.enchant.clone().unwrap();
        let primary = en.primary.unwrap_or(El::Fire);
        let mut s = BoltSpell {
            counts: en.counts.clone(),
            primary: en.primary,
            scales: b::bolt_scales(&en.counts, primary, b::ENCHANT_BOLT),
        };
        let spend = b::enchant_shot(&en, &s.scales);
        let bs = bases(&en.counts);
        let mut dirs = vec![dir];
        if bs.contains(&El::Water) {
            dirs = vec![yaw_rot(dir, -0.13), dir, yaw_rot(dir, 0.13)];
            // Flying bolts at half: about two of the three land. A spark
            // never misses, so all three land: a third each.
            let share = if b::flight(s.primary).zap {
                1.0 / 3.0
            } else {
                0.5
            };
            for sc in s.scales.iter_mut() {
                sc.1 *= share;
            }
        }
        let tip = staff_tip(self.pos(id), w);
        let n = dirs.len();
        for &d in &dirs {
            let bolt = Bolt::new(
                id,
                s.clone(),
                d,
                tip + d * 0.1,
                bs.clone(),
                1.0 / n as f64,
                d == dir,
            );
            self.fire_bolt(bolt);
        }
        World::muzzle(w, s.primary.map(fx::el_c).unwrap_or(fx::WHITE));
        self.count("shots");
        w.spend_enchant(spend);
    }

    /// The sword swing: hits monsters in front with your element and knocks
    /// their bullets back the way you aim.
    fn melee(&mut self, id: Id, w: &mut Wizard) {
        if w.swing_t > 0.0 || w.swing_cd > 0.0 || w.charging.is_some() || w.roll_t > 0.0 {
            return;
        }
        w.swing_t = SWING_COOLDOWN;
        w.swing_cd = SWING_COOLDOWN + crate::knobs::swing_recover() * w.mods().swing;
        self.count("swings");
        let touch = w.bolt_spell();
        let enchanted = w.enchant_time > 0.0 && w.enchant.is_some();
        if enchanted {
            self.enchanted_swing(id, w);
        }
        let pos = self.pos(id);
        let whet = self.mods(id).sword;
        for a in self.living_enemies() {
            let (ap, ar) = {
                let ac = self.a(a);
                (ac.pos, ac.radius)
            };
            let to = flat(ap - pos);
            if to.length() > swing_reach() + ar
                || signed_angle(w.aim_dir, to).abs() > SWING_ARC + ar.atan2(to.length().max(0.1))
            {
                continue;
            }
            self.hit(
                a,
                &[(El::Earth, 1.0)],
                0.4 * whet,
                Some(id),
                to.normalize_or_zero(),
                1.0,
            );
            if !enchanted {
                self.strike(a, &touch, 2.5 * whet, id, to.normalize_or_zero());
            }
            w.add_mana(b::MANA_PER_SWING);
        }
        // The sword smashes what stands in the arc too.
        let cut = if enchanted {
            w.enchant_reach()
        } else {
            swing_reach()
        };
        for i in 0..self.level.props.len() {
            let p = &self.level.props[i];
            if p.broken || !p.breakable {
                continue;
            }
            let tp = flat(p.pos - pos);
            if tp.length() < cut + 0.6 && signed_angle(w.aim_dir, tp).abs() < SWING_ARC + 0.3 {
                self.damage_prop(i, 4.0, w.aim_dir, None);
            }
        }
        w.swing_dir = w.aim_dir;
        self.deflect_sweep(id, w);
        let sc = w.bolt_el.map(fx::el_c).unwrap_or(fx::rgb(1.0, 0.85, 0.5));
        let ep = w.enchant.as_ref().and_then(|e| e.primary);
        let at = pos + w.aim_dir * 1.4 + Vec3::Y;
        self.burst(
            at,
            fx::mix(sc, fx::rgb(0.95, 0.9, 0.75), 0.4),
            if w.enchant.is_none() { 10 } else { 22 },
            3.5,
            0.22,
            ep,
        );
        if enchanted {
            let cost = b::enchant_swing(w.enchant.as_ref().unwrap());
            w.spend_enchant(cost);
        }
    }

    /// Knock back every enemy bullet inside the swing's arc. Runs on every
    /// tick of the swing, so one that flies in mid-sweep goes back too.
    fn deflect_sweep(&mut self, id: Id, w: &mut Wizard) {
        let pos = self.pos(id);
        let team = self.team(id);
        for bid in self.bullets() {
            let (bp, br, bt) = {
                let bu = self.ecs.get::<&crate::pickups::Bullet>(bid).unwrap();
                (bu.pos, bu.radius, bu.team)
            };
            if bt == team {
                continue;
            }
            let to2 = flat(bp - pos);
            let d = to2.length();
            if d > swing_reach() + 0.3 + br {
                continue;
            }
            if d > 0.6 && signed_angle(w.swing_dir, to2).abs() > SWING_ARC + br.atan2(d) {
                continue;
            }
            self.deflect(bid, id, w.aim_dir);
            w.add_mana(b::MANA_PER_DEFLECT);
            self.count("deflects");
            w.deflected = true;
        }
    }

    /// The enchanted swing: the blade of elements lands on everything it
    /// sweeps through, and each element adds its trick.
    fn enchanted_swing(&mut self, id: Id, w: &mut Wizard) {
        let s = w.enchant.clone().unwrap();
        let bs = bases(&s.counts);
        let reach = w.enchant_reach();
        let pos = self.pos(id);
        let c = crate::spells::color(&s);
        if bs.contains(&El::Void) {
            // Void drags everything near into the arc first.
            for a in self.living_enemies() {
                let mut ac = self.am(a);
                if ac.pos.distance(pos) < reach + 3.0 {
                    let d = flat(pos + w.aim_dir * 1.4 - ac.pos);
                    ac.push += d.normalize_or_zero() * 9.0;
                }
            }
        }
        let mix = crate::world::mix_of(&s.counts);
        let mut hits = 0;
        for a in self.living_enemies() {
            let (ap, ar) = {
                let ac = self.a(a);
                (ac.pos, ac.radius)
            };
            let to = flat(ap - pos);
            let ang = signed_angle(w.aim_dir, to).abs();
            if to.length() > reach + ar || ang > SWING_ARC + 0.2 + ar.atan2(to.length().max(0.1)) {
                continue;
            }
            hits += 1;
            let dir = to.normalize_or_zero();
            self.hit(a, &mix, b::ENCHANT_SWING, Some(id), dir, 1.0);
            {
                let mut ac = self.am(a);
                if bs.contains(&El::Earth) {
                    ac.stun = ac.stun.max(0.8);
                }
                if bs.contains(&El::Air) || bs.contains(&El::Water) {
                    ac.push += dir * if bs.contains(&El::Air) { 13.0 } else { 8.0 };
                }
            }
            if bs.contains(&El::Lightning) {
                // The hit jumps on to whoever stands near.
                for o in self.living_enemies() {
                    if o != a && self.pos(o).distance(self.pos(a)) < 3.0 {
                        self.hit(o, &[(El::Lightning, 1.0)], 1.2, Some(id), dir, 1.0);
                        let at = self.pos(o) + Vec3::Y;
                        self.burst(
                            at,
                            fx::el_c(El::Lightning),
                            8,
                            5.0,
                            0.15,
                            Some(El::Lightning),
                        );
                    }
                }
            }
        }
        if bs.contains(&El::Fire) {
            let at = self.level.clamp_inside(pos + w.aim_dir * 2.2);
            let f = self.add_field(at, El::Fire, 1.3, 3.0, Some(id));
            self.field_mut(f).friendly = true;
        }
        if bs.contains(&El::Frost) {
            self.touch_ground(pos + w.aim_dir * 2.0, 2.0, El::Frost, Some(id));
        }
        // The blade's path: the element sprays off along the arc it cut.
        for i in 0..7 {
            let k = (i as f32 - 3.0) / 3.0;
            let at = pos + yaw_rot(w.aim_dir, k * SWING_ARC) * reach * 0.85 + Vec3::Y * 0.9;
            self.burst(at, c, 10, 4.0, 0.25, s.primary);
        }
        self.ev(Ev::LightPop {
            at: pos + w.aim_dir * 2.0,
            c,
            energy: 2.5,
            range: 6.0,
            time: 0.2,
        });
        if hits > 0 {
            self.shake(0.15);
        }
    }

    /// A dodge roll the way you walk, untouchable at its start.
    fn roll(&mut self, id: Id, w: &mut Wizard) {
        if w.roll_cd > 0.0 || w.roll_t > 0.0 || !self.a(id).can_act() {
            return;
        }
        let mut d = Vec3::new(w.pad.mv.x, 0.0, w.pad.mv.y);
        if d.length() < 0.2 {
            d = w.aim_dir;
        }
        w.roll_dir = d.normalize();
        w.roll_t = ROLL_TIME;
        w.roll_cd = crate::knobs::roll_cooldown();
        {
            let mut a = self.am(id);
            a.invuln = ROLL_IFRAMES;
            // Through monsters, not into them: only the world stops a roll.
            a.blocks = false;
        }
        if w.channel.is_some() {
            w.channel = None;
        }
        // Rolling out of a windup calls the spell off: mana and elements back.
        if let Some(s) = w.charging.take() {
            w.mark = None;
            w.mana = w.mana_max().min(w.mana + Wizard::cost(&s) * w.mods().cost);
            let mut back = s.slots.clone();
            back.extend(w.queue.slots.iter().copied());
            back.truncate(w.queue.size);
            w.queue.slots = back;
            w.ammo = w.full_ammo();
        }
        w.rolled = true;
        self.count("rolls");
    }

    /// Load one element for mana.
    fn conjure(&mut self, id: Id, w: &mut Wizard, e: El) {
        let mut trial = w.queue.clone();
        if trial.add(e) == Added::Full {
            return;
        }
        let price = b::MANA_PER_LOAD * w.mods().cost;
        if w.mana < price - 0.01 {
            self.no_mana(id, w);
            return;
        }
        w.mana -= price;
        let front = w.queue.slots.first().copied();
        let what = w.queue.add(e);
        // A new or changed front element starts with a full charge.
        if w.queue.is_empty() {
            w.ammo = 0;
        } else if w.queue.slots.first().copied() != front {
            w.ammo = w.full_ammo();
        }
        if what == Added::Fuse {
            let at = self.pos(id) + Vec3::Y * 1.9;
            self.burst(at, fx::el_c(e), 12, 2.5, 0.25, None);
        }
    }

    /// Load a remembered queue (D-pad recast or a saved spell), paying for
    /// each slot.
    fn load_slots(&mut self, id: Id, w: &mut Wizard, slots: &[El]) {
        let slots = &slots[..slots.len().min(w.queue.size)];
        let price = b::MANA_PER_LOAD * slots.len() as f64 * w.mods().cost;
        if w.mana < price - 0.01 {
            self.no_mana(id, w);
            return;
        }
        w.mana -= price;
        w.queue.slots = slots.to_vec();
        w.ammo = w.full_ammo();
    }

    fn no_mana(&mut self, id: Id, w: &mut Wizard) {
        w.no_mana = 0.6;
        // The gem fizzles: a dull blue puff, and the mana orb flashes.
        let at = staff_tip(self.pos(id), w);
        self.ev(Ev::Puff {
            at,
            c: fx::rgb(0.3, 0.4, 0.75),
            size: 0.35,
        });
        self.count("no_mana");
    }

    /// Cast what is queued in `form`.
    fn cast(&mut self, id: Id, w: &mut Wizard, form: Form) {
        if w.swing_t > 0.0 {
            return;
        }
        if w.running(self) {
            // RT again ends the stream, and casts what you loaded since if any.
            if form != Form::Cast || w.queue.is_empty() {
                if form == Form::Cast {
                    w.channel = None;
                }
                return;
            }
            w.channel = None;
        }
        if w.charging.is_some() {
            return;
        }
        let s = w.preview(form);
        if matches!(s.shape, Shape::Fizzle | Shape::Staff) {
            return;
        }
        let price = Wizard::cost(&s) * w.mods().cost;
        if w.mana < price - 0.01 {
            self.no_mana(id, w);
            return;
        }
        w.mana -= price;
        w.ammo = 0;
        // What the spell did not use stays loaded, front first.
        let used = s.slots.len();
        let rest: Vec<El> = w.queue.slots[used.min(w.queue.slots.len())..].to_vec();
        if used > 0 {
            w.last_cast = w.queue.slots[..used].to_vec();
        }
        w.queue.clear();
        if !rest.is_empty() {
            w.queue.slots = rest;
            w.ammo = w.full_ammo();
        }
        w.cast_anim = 0.3;
        self.on_cast(&s);
        let pos = self.pos(id);
        let mods = self.mods(id);
        // Wind-up spells ask the Luau hooks when they are let go.
        let s = if crate::spells::winds_up(s.shape) {
            s
        } else {
            match crate::rules::cast(self, id, w.seat, w.aim_dir, &s) {
                crate::rules::Cast::Keep => s,
                crate::rules::Cast::Skip => return,
                crate::rules::Cast::Instead(t) => *t,
            }
        };
        match s.shape {
            Shape::Spray | Shape::Beam | Shape::Strike | Shape::Projectile | Shape::Overload => {
                w.charge_t = 0.0;
                w.charge_need = b::windup(s.shape) * mods.charge;
                w.mark = self.cast_mark(id, w, &s);
                w.charging = Some(Box::new(s));
            }
            Shape::Nova => {
                self.nova(id, &s, nova_n(&s.n), pos, true, None, 1.0);
                if mods.echo {
                    self.later(
                        0.35,
                        crate::world::Later::Nova {
                            caster: id,
                            spell: Box::new(s),
                        },
                    );
                }
            }
            Shape::Trap => {
                let most = MAX_TRAPS + if mods.planter { 2 } else { 0 };
                w.traps.retain(|&t| self.ecs.contains(t));
                while w.traps.len() >= most {
                    let old = w.traps.remove(0);
                    let _ = self.ecs.despawn(old);
                }
                let at = self.level.clamp_inside(pos);
                let t = self.plant_trap(id, &s, at);
                w.traps.push(t);
            }
            Shape::Sentry => {
                let most = if mods.planter { 2 } else { 1 };
                w.sentries.retain(|&t| self.ecs.contains(t));
                while w.sentries.len() >= most {
                    let old = w.sentries.remove(0);
                    let at = self
                        .ecs
                        .get::<&crate::spells::Sentry>(old)
                        .map(|s| s.pos)
                        .unwrap_or(pos);
                    self.burst(
                        at + Vec3::Y * 1.65,
                        crate::spells::color(&s),
                        20,
                        4.0,
                        0.35,
                        None,
                    );
                    let _ = self.ecs.despawn(old);
                }
                let at = self.level.clamp_inside(pos);
                let t = self.plant_sentry(id, &s, at);
                w.sentries.push(t);
            }
            Shape::Enchant => self.enchant(id, w, s),
            Shape::Stomp => self.stomp(id, w),
            _ => {}
        }
    }

    /// The wound-up spell goes, the way you aim now.
    fn release(&mut self, id: Id, w: &mut Wizard) {
        let s = *w.charging.take().unwrap();
        w.mark = None;
        w.cast_anim = 0.3;
        let s = match crate::rules::cast(self, id, w.seat, w.aim_dir, &s) {
            crate::rules::Cast::Keep => s,
            crate::rules::Cast::Skip => return,
            crate::rules::Cast::Instead(t) => *t,
        };
        let tip = staff_tip(self.pos(id), w);
        // The spell leaves the staff with a flash.
        self.burst(tip, crate::spells::color(&s), 14, 4.0, 0.14, None);
        match s.shape {
            Shape::Spray => w.channel = Some(self.start_spray(id, &s, tip)),
            Shape::Beam => w.channel = Some(self.start_beam(id, &s, tip)),
            Shape::Strike => {
                let at = self.cast_point(id, w, &s);
                self.thunder(id, &s, at);
            }
            Shape::Projectile => {
                let count = s.n.pieces.max(1);
                for i in 0..count {
                    let spread = if count == 1 {
                        0.0
                    } else {
                        (s.n.spread as f32).to_radians() * (i as f32 / (count - 1) as f32 - 0.5)
                    };
                    let dir = yaw_rot(w.aim_dir, spread);
                    self.throw(id, &s, dir, tip + dir * 0.1);
                }
                w.cast_anim = 0.3;
            }
            Shape::Overload => {
                let pos = self.pos(id);
                let at = if s.primary == Some(El::Water) {
                    // The way it will roll: its start in front of the caster.
                    pos + w.aim_dir * WAVE_START
                } else {
                    pos + w.aim_dir * OVERLOAD_REACH
                };
                let at = self.level.clamp_inside(at);
                self.overload(id, &s, at);
            }
            _ => {}
        }
    }

    fn cast_mark(&self, id: Id, w: &Wizard, s: &Spell) -> Option<Vec3> {
        if matches!(
            s.shape,
            Shape::Projectile | Shape::Beam | Shape::Strike | Shape::Overload
        ) {
            Some(self.cast_point(id, w, s))
        } else {
            None
        }
    }

    /// Where the charging spell would strike if let go now: an overload's
    /// spot, otherwise the first monster in line, a wall, or its reach.
    pub fn cast_point(&self, id: Id, w: &Wizard, s: &Spell) -> Vec3 {
        let pos = self.pos(id);
        if s.shape == Shape::Overload {
            if s.primary == Some(El::Water) {
                // The middle of the strip the wave will sweep.
                return flat(pos + w.aim_dir * (WAVE_START + WAVE_RUN * 0.5));
            }
            return flat(self.level.clamp_inside(pos + w.aim_dir * OVERLOAD_REACH));
        }
        let reach = if s.shape == Shape::Projectile {
            12.0
        } else if s.n.range > 0.0 {
            s.n.range as f32
        } else {
            10.0
        };
        let from = pos + Vec3::Y * 1.1;
        let mut best = reach;
        for a in self.living_enemies() {
            let ac = self.a(a);
            let to = flat(ac.pos - pos);
            let along = to.dot(w.aim_dir);
            if along > 0.0 && along < best && (to - w.aim_dir * along).length() < ac.radius + 0.5 {
                best = along;
            }
        }
        if let Some((at, _)) = self.ray(from, from + w.aim_dir * best) {
            best = best.min((at - from).length() - 0.3);
        }
        flat(pos + w.aim_dir * best.max(1.0))
    }

    /// An enchantment lays the queue on your sword for a while.
    fn enchant(&mut self, id: Id, w: &mut Wizard, s: Spell) {
        w.enchant_end();
        w.enchant_time = s.n.duration;
        w.enchant_store = b::enchant_store(&s);
        w.enchant_full = w.enchant_store;
        let n: u32 = s.counts.iter().map(|c| c.1 as u32).sum();
        w.enchant_power = n.min(queue::SIZE as u32) as f64 / queue::SIZE as f64;
        let at = self.pos(id) + Vec3::new(-0.36, 1.28, 0.24);
        self.burst(at, crate::spells::color(&s), 20, 4.0, 0.2, s.primary);
        w.enchant = Some(Box::new(s));
    }

    /// A burst of the empty queue: a stomp that shoves whoever stands near.
    fn stomp(&mut self, id: Id, w: &mut Wizard) {
        if w.swing_t > 0.0 {
            return;
        }
        w.swing_t = SWING_COOLDOWN;
        let pos = self.pos(id);
        let ff = self.ff();
        for a in self.living() {
            if a == id {
                continue;
            }
            let to = flat(self.pos(a) - pos);
            if to.length() > 2.6 + self.a(a).radius {
                continue;
            }
            self.hit(
                a,
                &[(El::Earth, 1.0)],
                0.55,
                Some(id),
                to.normalize_or_zero(),
                ff,
            );
        }
        self.ev(Ev::Ring {
            at: pos,
            c: fx::rgb(0.7, 0.6, 0.5),
            r: 2.6,
            time: 0.3,
        });
    }

    /// A wizard fell (Wizard.die): the grave rises beside the body.
    pub fn wizard_died(&mut self, id: Id) {
        let (how, dir, pos) = {
            let a = self.a(id);
            (a.death_style(), a.last_dir, a.pos)
        };
        {
            let mut a = self.am(id);
            a.burn = 0.0;
            a.poison = 0.0;
        }
        let mut w = self.wiz_mut(id);
        w.charging = None;
        w.mark = None;
        w.enchant_end();
        w.channel = None;
        w.queue.clear();
        w.revive = 0.0;
        let mut away = flat(dir);
        if away.length() < 0.05 {
            away = -Vec3::new(w.body_yaw.sin(), 0.0, w.body_yaw.cos());
        }
        let away = away.normalize();
        w.tomb = Some(pos - away * 0.9);
        w.tomb_t = 0.0;
        let seat = w.seat;
        drop(w);
        self.ev(Ev::WizardDown {
            seat,
            how,
            dir: away,
        });
    }

    /// A friend standing at your grave raises you: REVIVE_NEED in about
    /// three seconds with one, faster with more.
    fn raise(&mut self, id: Id, w: &mut Wizard, dt: f64) {
        w.tomb_t += dt;
        let mut n = 0.0;
        for &o in &self.wizards {
            if o != id && self.alive(o) && self.grave_dist(id, w, self.pos(o)) < REVIVE_REACH {
                n += self.a(o).mods.raise;
            }
        }
        if n == 0.0 {
            w.revive = (w.revive - crate::knobs::revive_rate() * 0.5 * dt).max(0.0);
        } else {
            w.revive += crate::knobs::revive_rate() * n * dt;
            if w.revive >= crate::knobs::revive_need() {
                self.revive(id, w, 0.4);
            }
        }
    }

    /// How far from the grave: the stone or the body, whichever is nearer.
    pub fn grave_dist(&self, id: Id, w: &Wizard, at: Vec3) -> f32 {
        let mut d = at.distance(self.pos(id));
        if let Some(t) = w.tomb {
            d = d.min(at.distance(t));
        }
        d
    }

    pub fn revive(&mut self, id: Id, w: &mut Wizard, frac: f64) {
        if self.alive(id) {
            return;
        }
        {
            let mut a = self.am(id);
            a.alive = true;
            a.hp = a.max_hp * frac;
            a.frozen = 0.0;
            a.chill = 0.0;
            a.stun = 0.0;
            a.blocks = true;
            // A moment to stand up while the pieces fly back together.
            a.invuln = a.invuln.max(1.2);
        }
        if let Some(t) = w.tomb.take() {
            self.ev(Ev::LightPop {
                at: t,
                c: fx::rgb(1.0, 0.82, 0.45),
                energy: 4.0,
                range: 8.0,
                time: 0.6,
            });
        }
        self.ev(Ev::WizardUp { seat: w.seat });
    }

    /// revive() for a wizard that is not mid-step.
    pub fn revive_id(&mut self, id: Id, frac: f64) {
        let mut w = std::mem::take(&mut *self.wiz_mut(id));
        self.revive(id, &mut w, frac);
        *self.wiz_mut(id) = w;
    }

    /// A tome taken: its powers go to the wizard.
    pub fn give_relic(&mut self, id: Id, relic: usize) {
        let mods = {
            let mut w = self.wiz_mut(id);
            w.relics.push(relic);
            w.mods()
        };
        let mut a = self.am(id);
        a.mods = mods;
        a.speed = crate::knobs::wizard_speed() * mods.speed;
        let old = a.max_hp;
        a.max_hp = 100.0 + mods.hp;
        a.winds = mods.wind as i32;
        a.hp += a.max_hp - old;
        drop(a);
        self.unlocks_changed();
    }

    /// Earn unlocks up to `n`; a newly learned element flares on the staff.
    pub fn unlock_to(&mut self, id: Id, n: i32) {
        let before = self.wiz_mut(id).learn(n);
        if self.wiz(id).unlocks != n {
            return;
        }
        self.unlocks_changed();
        let new: Vec<El> = self
            .wiz(id)
            .open_elements()
            .into_iter()
            .filter(|e| !before.contains(e))
            .collect();
        if !new.is_empty() {
            let mut w = self.wiz_mut(id);
            w.learned = new.clone();
            w.learned_t = 6.0;
        }
        let tip = self.staff_tip(id);
        let pos = self.pos(id);
        for e in new {
            self.burst(tip, fx::el_c(e), 26, 3.5, 0.2, None);
            self.ev(Ev::Ring {
                at: pos,
                c: fx::el_c(e),
                r: 2.2,
                time: 0.5,
            });
        }
    }
}
