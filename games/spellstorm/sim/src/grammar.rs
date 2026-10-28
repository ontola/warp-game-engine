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

use crate::balance;
use crate::el::{El, El::*, Kind};
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

fn cast_noun(e: El) -> &'static str {
    match e {
        Earth => "Boulder",
        Ice => "Ice Shards",
        Mud => "Mud Ball",
        Magma => "Lava Bomb",
        Lightning => "Thunderbolt",
        Storm => "Thunderclap",
        Poison => "Venom Beam",
        Void => "Void Ray",
        Fire => "Flamethrower",
        Frost => "Frost Breath",
        Water => "Water Jet",
        Steam => "Steam Jet",
        Air => "Gale",
    }
}

fn nova_noun(e: El) -> &'static str {
    match e {
        Poison => "Toxic Burst",
        Void => "Void Burst",
        Fire => "Fire Nova",
        Frost => "Frost Nova",
        Water => "Splash",
        Lightning => "Thunder Ring",
        Earth => "Shockwave",
        Air => "Gust Ring",
        Steam => "Steam Burst",
        Ice => "Ice Ring",
        Mud => "Mudslide",
        Magma => "Eruption",
        Storm => "Tempest",
    }
}

fn plant_noun(e: El) -> &'static str {
    match e {
        Earth => "Spike Trap",
        Ice => "Ice Spikes",
        Mud => "Mud Trap",
        Magma => "Lava Trap",
        Lightning => "Shock Trap",
        Storm => "Thunder Trap",
        Poison => "Poison Trap",
        Void => "Void Mine",
        Fire => "Fire Trap",
        Frost => "Frost Trap",
        Water => "Puddle Trap",
        Steam => "Steam Vent",
        Air => "Gust Trap",
    }
}

fn sentry_noun(e: El) -> &'static str {
    match e {
        Earth => "Stone Sentry",
        Ice => "Ice Sentry",
        Mud => "Mud Sentry",
        Magma => "Lava Sentry",
        Lightning => "Tesla Sentry",
        Storm => "Storm Sentry",
        Poison => "Venom Sentry",
        Void => "Void Sentry",
        Fire => "Fire Sentry",
        Frost => "Frost Sentry",
        Water => "Water Sentry",
        Steam => "Steam Sentry",
        Air => "Wind Sentry",
    }
}

fn overload_noun(e: El) -> &'static str {
    match e {
        Fire => "Meteor",
        Frost => "Blizzard",
        Water => "Tidal Wave",
        Lightning => "Thunderstorm",
        Earth => "Earthquake",
        Air => "Tornado",
        Poison => "Plague",
        Void => "Black Hole",
        _ => "Cataclysm",
    }
}

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

fn cataclysm_name(lead: El, rider: El) -> Option<&'static str> {
    Some(match (lead, rider) {
        (Lightning, Water) | (Water, Lightning) => "Tempest",
        (Fire, Air) | (Air, Fire) => "Firestorm",
        (Fire, Earth) => "Meteor Shower",
        (Earth, Fire) => "Eruption",
        (Frost, Air) => "Whiteout",
        (Water, Frost) => "Frozen Sea",
        (Void, Lightning) => "Event Horizon",
        (Poison, Water) => "Blight Tide",
        _ => return None,
    })
}

fn adj(e: El) -> &'static str {
    match e {
        Poison => "Venomous",
        Void => "Cursed",
        Fire => "Burning",
        Frost => "Freezing",
        Water => "Soaking",
        Lightning => "Shocking",
        Earth => "Crushing",
        Air => "Gusting",
        Steam => "Scalding",
        Ice => "Glacial",
        Mud => "Muddy",
        Magma => "Molten",
        Storm => "Thundering",
    }
}

fn enchant_adj(e: El) -> &'static str {
    match e {
        Earth => "Stone",
        Air => "Wind",
        e => e.name(),
    }
}

fn cast_shape(k: Kind) -> Shape {
    match k {
        Kind::Solid => Shape::Projectile,
        Kind::Energy => Shape::Strike,
        Kind::Spirit => Shape::Beam,
        Kind::Flow => Shape::Spray,
    }
}

fn first(slots: &[El], e: El) -> usize {
    slots.iter().position(|s| *s == e).unwrap_or(usize::MAX)
}

/// The element with the highest count among `pool`; ties go to the one
/// conjured first.
fn most(slots: &[El], spell: &Spell, pool: &[El]) -> El {
    let mut best = pool[0];
    for &e in pool {
        let (ce, cb) = (spell.count(e), spell.count(best));
        if ce > cb || (ce == cb && first(slots, e) < first(slots, best)) {
            best = e;
        }
    }
    best
}

fn adjectives(payload: &[El]) -> String {
    payload
        .iter()
        .take(2)
        .map(|e| format!("{} ", adj(*e)))
        .collect()
}

/// `slots` in queue order. `mods` are the caster's tomes; they change
/// numbers, never the shape or the name.
pub fn compose(slots: &[El], form: Form, mods: &Mods) -> Spell {
    let mut counts: Vec<(El, u8)> = Vec::new();
    for &e in slots {
        match counts.iter_mut().find(|c| c.0 == e) {
            Some(c) => c.1 += 1,
            None => counts.push((e, 1)),
        }
    }
    let mut spell = Spell {
        form,
        counts,
        slots: slots.to_vec(),
        power: slots.len(),
        primary: None,
        shape: Shape::Fizzle,
        name: String::new(),
        payload: Vec::new(),
        riders: Vec::new(),
        key: String::new(),
        n: Numbers::default(),
    };
    if slots.is_empty() {
        (spell.shape, spell.name) = match form {
            Form::Cast | Form::Plant => (Shape::Staff, "Staff Swing".into()),
            Form::Nova => (Shape::Stomp, "Stomp".into()),
            Form::Enchant => (Shape::Fizzle, "Fizzle".into()),
        };
        spell.key = format!("{}|{}", form.name(), spell.shape.name());
        numbers(&mut spell, mods);
        return spell;
    }

    let primary = if matches!(form, Form::Nova | Form::Enchant) {
        most(slots, &spell, slots)
    } else {
        let best_kind = slots.iter().map(|e| e.kind()).min().unwrap();
        let heavy: Vec<El> = slots
            .iter()
            .copied()
            .filter(|e| e.kind() == best_kind)
            .collect();
        let p = most(slots, &spell, &heavy);
        spell.shape = if form == Form::Cast {
            cast_shape(best_kind)
        } else if slots.len() >= SENTRY_SLOTS {
            Shape::Sentry
        } else {
            Shape::Trap
        };
        p
    };
    spell.primary = Some(primary);

    let mut payload: Vec<El> = Vec::new();
    for &e in slots {
        if e != primary && !payload.contains(&e) {
            payload.push(e);
        }
    }
    payload.sort_by(|a, b| {
        spell
            .count(*b)
            .cmp(&spell.count(*a))
            .then(first(slots, *a).cmp(&first(slots, *b)))
    });
    spell.payload = payload.clone();

    match form {
        Form::Cast => {
            if slots.len() >= queue::SIZE {
                cataclysm(&mut spell, slots);
            } else {
                spell.name = adjectives(&payload) + cast_noun(primary);
            }
        }
        Form::Nova => {
            spell.shape = Shape::Nova;
            spell.name = adjectives(&payload) + nova_noun(primary);
        }
        Form::Plant => {
            let noun = if spell.shape == Shape::Sentry {
                sentry_noun(primary)
            } else {
                plant_noun(primary)
            };
            spell.name = adjectives(&payload) + noun;
        }
        Form::Enchant => {
            spell.shape = Shape::Enchant;
            let words: Vec<&str> = std::iter::once(primary)
                .chain(payload.iter().copied())
                .take(2)
                .map(enchant_adj)
                .collect();
            spell.name = words.join(" ") + " Enchantment";
        }
    }
    let mut sorted = payload;
    sorted.sort();
    let list: Vec<String> = sorted.iter().map(|e| e.idx().to_string()).collect();
    spell.key = format!(
        "{}|{}|{}|{}",
        form.name(),
        spell.shape.name(),
        primary.idx(),
        list.join(",")
    );
    numbers(&mut spell, mods);
    spell
}

/// A full queue cast ahead: the base element with the most slots leads
/// (fused slots count as their `fused_base`), the rest ride.
fn cataclysm(spell: &mut Spell, slots: &[El]) {
    let mut base: Vec<(El, u32)> = Vec::new();
    let mut bump = |b: El, n: u32| match base.iter_mut().find(|c| c.0 == b) {
        Some(c) => c.1 += n,
        None => base.push((b, n)),
    };
    for &e in slots {
        let b = fused_base(e);
        bump(b, 2);
        for part in e.parts() {
            if part != b {
                bump(part, 1);
            }
        }
    }
    let mut lead: Option<(El, u32)> = None;
    for &(b, c) in &base {
        match lead {
            Some((l, lc)) if !(c > lc || (c == lc && b.kind() < l.kind())) => {}
            _ => lead = Some((b, c)),
        }
    }
    let lead = lead.unwrap().0;
    let mut riders: Vec<(El, u32)> = base.iter().copied().filter(|c| c.0 != lead).collect();
    riders.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let riders: Vec<El> = riders.into_iter().map(|c| c.0).collect();
    spell.shape = Shape::Overload;
    spell.primary = Some(lead);
    spell.name = match riders.first().and_then(|r| cataclysm_name(lead, *r)) {
        Some(n) => n.into(),
        None => adjectives(&riders) + overload_noun(lead),
    };
    spell.riders = riders;
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

fn numbers(spell: &mut Spell, mods: &Mods) {
    spell.n = raw_numbers(spell, mods);
    if !spell.n.has_rate || !balance::measured(spell) {
        return;
    }
    // The factor comes from the spell without tomes, so a tome still makes
    // you stronger.
    let mut base = spell.clone();
    base.n = raw_numbers(&base, &Mods::default());
    let f = balance::fit(&mut base);
    spell.n.rate *= f;
}

pub fn raw_numbers(spell: &Spell, mods: &Mods) -> Numbers {
    let p = spell.power as f64 * STEP;
    let lead = spell.primary.map_or(0.0, |e| spell.count(e) as f64) * STEP;
    let n = Numbers {
        has_rate: true,
        ..Default::default()
    };
    match spell.shape {
        Shape::Spray => Numbers {
            range: (3.2 + 0.9 * lead) * mods.reach,
            angle: 26.0 + 4.0 * p,
            duration: 2.5 + 0.5 * p,
            rate: 1.0,
            ..n
        },
        Shape::Beam => Numbers {
            range: 15.0 * mods.reach,
            width: 0.25 + 0.08 * p,
            duration: 3.0 + 0.5 * p,
            rate: 1.3,
            ..n
        },
        Shape::Strike => Numbers {
            range: 8.0 + 0.5 * p,
            radius: 1.0 + 0.2 * p + 0.1 * lead,
            rate: 1.0,
            ..n
        },
        Shape::Projectile => {
            let ice = spell.primary == Some(Ice);
            let pieces = if ice {
                (3.0 + 2.0 * lead).round() as i32
            } else {
                1
            };
            Numbers {
                pieces,
                spread: 8.0 + 3.0 * pieces as f64,
                speed: 16.0,
                radius: 0.25 + 0.12 * lead,
                charge_time: mods.charge,
                rate: 1.0 / pieces as f64 * if pieces > 1 { 1.4 } else { 1.0 },
                splash: if matches!(spell.primary, Some(Magma | Mud)) {
                    2.2 + 0.4 * lead
                } else {
                    0.0
                },
                pierce: ice,
                ..n
            }
        }
        Shape::Nova => Numbers {
            radius: 2.4 + 0.65 * p,
            rate: 1.25,
            ..n
        },
        Shape::Trap => Numbers {
            radius: 2.2 + 0.35 * p,
            field: 1.4 + 0.35 * p,
            field_time: 5.0 + p,
            stun: 0.8 + 0.25 * lead,
            duration: 30.0,
            rate: 1.6,
            ..n
        },
        Shape::Sentry => Numbers {
            range: 9.0,
            interval: 0.3,
            duration: 14.0,
            rate: 0.5,
            ..n
        },
        Shape::Enchant => Numbers {
            has_rate: false,
            duration: balance::ENCHANT_TIME * mods.enchant_time,
            ..Default::default()
        },
        Shape::Overload => Numbers {
            radius: 5.0,
            rate: 4.0,
            ..n
        },
        Shape::Staff => Numbers {
            range: 1.8,
            rate: 1.0,
            ..n
        },
        Shape::Stomp => Numbers {
            radius: 2.5,
            rate: 1.0,
            ..n
        },
        Shape::Fizzle => Numbers::default(),
    }
}
