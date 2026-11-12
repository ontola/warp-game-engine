//! Turns a queue of elements plus a cast form into one spell (grammar.gd).
//! The effects only carry out what this returns.
//!
//! * The form is the button: RT casts ahead, RB bursts around you, LS
//!   plants at your feet, RS enchants your bolts and staff.
//! * The heaviest class decides the shape of a cast: solid throws, energy
//!   strikes from the sky, spirit beams, flow sprays.
//! * Planting one or two elements makes a trap, a full queue a sentry.
//! * Within that class the element you have most of leads, every other one
//!   rides along as payload. A full queue cast ahead is a cataclysm.

use crate::el::{El, El::*};
use crate::queue;
use crate::relics::Mods;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Form {
    Cast,
    Nova,
    Plant,
    Enchant,
}

pub const FORMS: [Form; 4] = [Form::Cast, Form::Nova, Form::Plant, Form::Enchant];

impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Form::Cast => "cast",
            Form::Nova => "nova",
            Form::Plant => "plant",
            Form::Enchant => "enchant",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Staff,
    Stomp,
    Fizzle,
    Spray,
    Beam,
    Strike,
    Projectile,
    Nova,
    Trap,
    Sentry,
    Enchant,
    Overload,
}

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Shape::Staff => "staff",
            Shape::Stomp => "stomp",
            Shape::Fizzle => "fizzle",
            Shape::Spray => "spray",
            Shape::Beam => "beam",
            Shape::Strike => "strike",
            Shape::Projectile => "projectile",
            Shape::Nova => "nova",
            Shape::Trap => "trap",
            Shape::Sentry => "sentry",
            Shape::Enchant => "enchant",
            Shape::Overload => "overload",
        }
    }
}

/// Size, range and timing of a shape. Only the fields its shape sets are
/// meaningful; `has_rate` is false where grammar.gd leaves `rate` out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Numbers {
    pub has_rate: bool,
    pub rate: f64,
    pub range: f64,
    pub angle: f64,
    pub duration: f64,
    pub width: f64,
    pub radius: f64,
    pub pieces: i32,
    pub spread: f64,
    pub speed: f64,
    pub charge_time: f64,
    pub splash: f64,
    pub pierce: bool,
    pub field: f64,
    pub field_time: f64,
    pub stun: f64,
    pub interval: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spell {
    pub form: Form,
    /// Element counts in order of first appearance (GDScript dictionaries
    /// keep insertion order, and some sums follow it).
    pub counts: Vec<(El, u8)>,
    pub slots: Vec<El>,
    pub power: usize,
    pub primary: Option<El>,
    pub shape: Shape,
    pub name: String,
    pub payload: Vec<El>,
    pub riders: Vec<El>,
    pub key: String,
    pub n: Numbers,
}

impl Spell {
    pub fn count(&self, e: El) -> u8 {
        self.counts.iter().find(|c| c.0 == e).map_or(0, |c| c.1)
    }

    pub fn has(&self, e: El) -> bool {
        self.count(e) > 0
    }
}

pub const SENTRY_SLOTS: usize = queue::SIZE;
pub const STEP: f64 = 5.0 / queue::SIZE as f64;
pub const TOTAL: usize = 2410;

/// A fused slot counts as this base element in a cataclysm.
pub fn fused_base(e: El) -> El {
    match e {
        Steam => Water,
        Ice => Frost,
        Mud => Earth,
        Magma => Fire,
        Storm => Lightning,
        e => e,
    }
}

/// `slots` in queue order. `mods` are the caster's tomes; they change
/// numbers, never the shape or the name. The Luau rules decide when they
/// are installed (crate::rules), else the built-in grammar below.
pub fn compose(slots: &[El], form: Form, mods: &Mods) -> Spell {
    crate::rules::compose(slots, form, mods)
}

/// The summed per-unit effect of every element in the spell, scaled by the
/// caster's tomes. Fusions count once each, like a base element.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sum {
    pub dmg: f64,
    pub poison: f64,
    pub push: f64,
    pub burn: f64,
    pub chill: f64,
    pub wet: f64,
    pub shock: f64,
    pub slow: f64,
    pub curse: f64,
}

pub fn effect(spell: &Spell, mods: &Mods) -> Sum {
    let mut out = Sum::default();
    for &(e, n) in &spell.counts {
        let n = n as f64;
        let m = mods.el[e.idx()];
        let f = e.effect();
        out.dmg += f.dmg * n * m;
        out.poison += f.poison * n * m;
        out.push += f.push * n * m;
        out.burn += f.burn * n * m;
        out.chill += f.chill * n * m;
        out.wet += f.wet * n * m;
        out.shock += f.shock * n * m;
        out.slow += f.slow * n * m;
        if e == Void {
            out.curse += n;
        }
    }
    out.dmg *= mods.dmg;
    out
}
