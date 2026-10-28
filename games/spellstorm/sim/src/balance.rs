//! The yardstick every spell is measured against (balance.gd, see the
//! Godot game's docs/balance.md). One number rules it: DMG_PER_MANA. A
//! spell's worth over its price lands on its target through its rate.

use std::f64::consts::PI;

use crate::el::{El, El::*};
use crate::grammar::{Shape, Spell};

pub const DMG_PER_MANA: f64 = 9.0;
pub const MANA_MAX: f64 = 100.0;
pub const MANA_REGEN: f64 = 3.0;
pub const SOUL_MANA_PER_HP: f64 = 0.15;
pub const MANA_PER_LOAD: f64 = 5.0;
pub const MANA_PER_ELEMENT: f64 = 13.0;
pub const OVERLOAD_COST: f64 = 40.0;
pub const SENTRY_COST: f64 = 50.0;
pub const MANA_PER_SWING: f64 = 3.0;
pub const MANA_PER_DEFLECT: f64 = 3.0;

pub const BOLT_DPS: f64 = 12.0;
pub const PLAIN_BOLT: f64 = 0.12;

/// How a bolt flies: shots a second, speed, reach, thrown in an arc, how
/// wide it hits, and whether it is a spark that jumps to the first in line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub rate: f64,
    pub speed: f64,
    pub range: f64,
    pub lob: bool,
    pub hit: f64,
    pub zap: bool,
}

const fn fl(rate: f64, speed: f64, range: f64, lob: bool, hit: f64) -> Flight {
    Flight {
        rate,
        speed,
        range,
        lob,
        hit,
        zap: false,
    }
}

pub const PLAIN_FLIGHT: Flight = fl(6.0, 22.0, 18.0, false, 0.18);

fn base_flight(e: El) -> Option<Flight> {
    Some(match e {
        Fire => fl(6.0, 22.0, 16.0, false, 0.2),
        Frost => fl(4.0, 36.0, 22.0, false, 0.14),
        Water => fl(9.0, 16.0, 9.0, true, 0.2),
        Lightning => Flight {
            zap: true,
            ..fl(5.0, 60.0, 11.0, false, 0.16)
        },
        Earth => fl(1.5, 15.0, 13.0, true, 0.34),
        Air => fl(7.0, 30.0, 18.0, false, 0.22),
        Poison => fl(3.0, 11.0, 10.0, true, 0.26),
        Void => fl(2.0, 9.0, 15.0, false, 0.34),
        _ => return None,
    })
}

pub const AMMO_TIME: f64 = 2.25;
pub const BURN_DPS: f64 = 7.0;
pub const BURN_MAX: f64 = 6.0;
pub const POISON_DOSE: f64 = 25.0;
pub const POISON_TIME: f64 = 5.0;
pub const POISON_CAP: f64 = 150.0;
pub const POISON_FLOOR: f64 = 2.0;
pub const PATIENCE: f64 = 4.0;
pub const BONUS_PER_SLOT: f64 = 0.1;
pub const BONUS_PER_KIND: f64 = 0.06;
pub const BAND: (f64, f64) = (0.8, 1.3);
pub const FIT_RANGE: (f64, f64) = (0.05, 30.0);
pub const DENSITY: f64 = 0.04;
pub const MAX_TARGETS: f64 = 6.0;
pub const CROWD: f64 = 0.5;
pub const FREEZE_WORTH: f64 = 12.0;
pub const FREEZE_TIME: f64 = 2.6;
pub const POISON_CREDIT: f64 = PATIENCE / (PATIENCE + POISON_TIME);
pub const RANGE_EVEN: f64 = 14.0;
pub const RANGE_WEIGHT: f64 = 0.2;
pub const ENCHANT_BOLT: f64 = 2.5;
pub const ENCHANT_SWING: f64 = 0.6;
pub const SWING_TARGETS: f64 = 2.0;
pub const ENCHANT_TIME: f64 = 20.0;
pub const SENTRY_UPTIME: f64 = 0.6;

fn ease(s: Shape) -> f64 {
    match s {
        Shape::Sentry => 0.85,
        Shape::Spray => 0.95,
        Shape::Projectile => 1.15,
        _ => 1.0,
    }
}

/// Every RT spell winds up this long before it goes.
pub fn windup(s: Shape) -> f64 {
    match s {
        Shape::Spray => 0.4,
        Shape::Beam => 0.55,
        Shape::Strike => 0.6,
        Shape::Projectile => 0.65,
        Shape::Overload => 1.0,
        _ => 0.0,
    }
}

fn held(s: Shape) -> bool {
    matches!(s, Shape::Spray | Shape::Beam)
}

fn ranged(s: Shape) -> bool {
    matches!(s, Shape::Spray | Shape::Beam | Shape::Strike)
}

/// What a shove, a soak, a slow and a shock are worth, in damage, per unit,
/// summed in balance.gd's order.
fn utility(e: &crate::el::Effect, scale: f64) -> f64 {
    let mut w = 0.0;
    w += e.push * scale * 1.0;
    w += e.wet * scale * 1.0;
    w += e.slow * scale * 3.0;
    w += e.shock * scale * 2.0;
    w
}

pub fn price(s: &Spell) -> f64 {
    let load = MANA_PER_LOAD * s.slots.len() as f64;
    match s.shape {
        Shape::Overload => load + OVERLOAD_COST,
        Shape::Sentry => load + SENTRY_COST,
        _ => load + MANA_PER_ELEMENT * s.power as f64,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Delivery {
    pub scale: f64,
    pub time: f64,
    pub targets: f64,
}

pub fn delivery(s: &Spell) -> Delivery {
    let n = &s.n;
    let d = |scale, time, targets| Delivery {
        scale,
        time,
        targets,
    };
    match s.shape {
        Shape::Spray => {
            let cone = n.angle.to_radians() * n.range * n.range * 0.5;
            d(n.rate * n.duration * 0.8, n.duration, crowd(cone))
        }
        Shape::Beam => d(n.rate * n.duration, n.duration, 1.0),
        Shape::Strike => d(
            n.rate,
            0.0,
            1.0 + (crowd(PI * n.radius * n.radius) - 1.0) * 0.6,
        ),
        Shape::Projectile => {
            let hit = 1.75 * n.rate * n.pieces as f64;
            let mut targets = 1.0;
            if n.splash > 0.0 {
                targets += (crowd(PI * n.splash * n.splash) - 1.0) * 0.6;
            }
            if n.pierce {
                targets = 1.5;
            }
            d(hit, 0.0, targets)
        }
        Shape::Nova | Shape::Trap | Shape::Overload => {
            d(n.rate * 0.8, 0.0, crowd(PI * n.radius * n.radius))
        }
        Shape::Sentry => {
            let shots = (n.duration / n.interval).floor() * SENTRY_UPTIME;
            d(n.rate * shots, n.duration * SENTRY_UPTIME, 1.0)
        }
        _ => d(0.0, 0.0, 0.0),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Worth {
    pub total: f64,
    pub hit: f64,
    pub burn: f64,
    pub poison: f64,
    pub freeze: f64,
    pub utility: f64,
    pub targets: f64,
    pub real: f64,
}

pub fn worth(s: &Spell) -> Worth {
    let d = delivery(s);
    let (mut dmg, mut poison, mut burn, mut chill, mut util, mut shock_dmg) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let mut soaks = false;
    for &(e, c) in &s.counts {
        let c = c as f64;
        let eff = e.effect();
        dmg += eff.dmg * c;
        if eff.shock > 0.0 {
            shock_dmg += eff.dmg * c;
        }
        soaks = soaks || eff.wet > 0.0;
        poison += eff.poison * c;
        burn += eff.burn * c;
        chill += eff.chill * c;
        util += utility(eff, c);
    }
    // Shock hits a wet monster twice as hard.
    if soaks && shock_dmg > 0.0 {
        dmg += shock_dmg * d.time / (d.time + 0.4);
    }
    let per_target = dmg * d.scale;
    let burn_s = (d.time + BURN_MAX).min(4.0 * burn * d.scale);
    let burn_worth = over_time(BURN_DPS, burn_s, d.time);
    let freezes = (chill * d.scale / 3.0).min(1.0 + d.time / 4.0);
    let freeze_worth = FREEZE_WORTH * freezes.floor();
    let utility_worth = util * d.scale;
    let dose = (POISON_DOSE * poison * d.scale).min(POISON_CAP * (1.0 + d.time / POISON_TIME));
    let poison_worth = dose * POISON_CREDIT;
    let total = (per_target + burn_worth + poison_worth + freeze_worth + utility_worth) * d.targets;
    let real = (per_target + BURN_DPS * burn_s + dose) * d.targets;
    Worth {
        total,
        hit: per_target,
        burn: burn_worth,
        poison: poison_worth,
        freeze: freeze_worth,
        utility: utility_worth,
        targets: d.targets,
        real,
    }
}

/// What `seconds` of damage over time at `dps` is worth: in full while the
/// spell itself runs (`kept`), credited at PATIENCE after.
pub fn over_time(dps: f64, seconds: f64, kept: f64) -> f64 {
    let now = seconds.min(kept);
    let tail = (seconds - now).max(0.0);
    dps * (now + PATIENCE * (1.0 - (-tail / PATIENCE).exp()))
}

pub fn ratio(s: &Spell) -> f64 {
    let p = price(s);
    if p <= 0.0 {
        return 0.0;
    }
    gain(s) / (p * DMG_PER_MANA)
}

/// How long the staff cannot shoot because of this spell.
pub fn busy(s: &Spell) -> f64 {
    let mut t = windup(s.shape);
    if held(s.shape) {
        t += s.n.duration;
    }
    t
}

pub fn crowd(area: f64) -> f64 {
    (1.0 + DENSITY * area).clamp(1.0, MAX_TARGETS)
}

pub fn credited(targets: f64) -> f64 {
    1.0 + (targets - 1.0).max(0.0) * CROWD
}

pub fn gain(s: &Spell) -> f64 {
    let w = worth(s);
    let per = w.total / w.targets.max(0.001);
    per * credited(w.targets) - BOLT_DPS * busy(s)
}

pub fn target(s: &Spell) -> f64 {
    let mut t = 1.0
        + BONUS_PER_SLOT * (s.slots.len() as f64 - 1.0)
        + BONUS_PER_KIND * (s.counts.len() as f64 - 1.0);
    if ranged(s.shape) {
        t *= 1.0 + RANGE_WEIGHT * ((RANGE_EVEN - s.n.range) / 10.0).clamp(-0.5, 1.0);
    }
    t * ease(s.shape)
}

pub fn measured(s: &Spell) -> bool {
    !matches!(
        s.shape,
        Shape::Staff | Shape::Stomp | Shape::Fizzle | Shape::Enchant | Shape::Overload
    )
}

/// The factor on the spell's rate that puts it on its target, found by
/// halving the gap (worth is not linear in the rate).
pub fn fit(s: &mut Spell) -> f64 {
    if !measured(s) || !s.n.has_rate {
        return 1.0;
    }
    let goal = target(s);
    let rate = s.n.rate;
    let (mut lo, mut hi) = FIT_RANGE;
    for _ in 0..24 {
        let mid = (lo * hi).sqrt();
        s.n.rate = rate * mid;
        if ratio(s) < goal {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    s.n.rate = rate;
    (lo * hi).sqrt()
}

/// How `e`'s bolts fly; a fusion averages its parts. None is the plain bolt.
pub fn flight(e: Option<El>) -> Flight {
    let Some(e) = e else { return PLAIN_FLIGHT };
    if let Some(f) = base_flight(e) {
        return f;
    }
    let p = e.fusion_parts().unwrap();
    let a = base_flight(p[0]).unwrap();
    let b = base_flight(p[1]).unwrap();
    Flight {
        rate: (a.rate * b.rate).sqrt(),
        speed: (a.speed + b.speed) * 0.5,
        range: (a.range + b.range) * 0.5,
        lob: a.lob && b.lob,
        hit: (a.hit + b.hit) * 0.5,
        zap: a.zap || b.zap,
    }
}

/// How many bolts one loaded element gives.
pub fn ammo(e: El) -> i32 {
    ((flight(Some(e)).rate * AMMO_TIME).round() as i32).max(2)
}

/// Harder bolts to land earn more.
pub fn skill(e: El) -> f64 {
    let f = flight(Some(e));
    let mut k = 1.0 + 0.3 * (1.0 - f.speed.min(22.0) / 22.0);
    k += 0.1 * (1.0 - f.rate / 6.0).max(0.0);
    if f.lob {
        k += 0.1;
    }
    k += RANGE_WEIGHT * ((RANGE_EVEN - f.range) / 10.0).clamp(-0.5, 1.0);
    k
}

/// One bolt of `e` at `scale`, as part of a stream at `rate` a second.
pub fn shot_worth(e: El, scale: f64, rate: f64) -> f64 {
    let eff = e.effect();
    let mut w = eff.dmg * scale;
    w += utility(eff, scale);
    if eff.burn > 0.0 {
        w += BURN_DPS * (eff.burn * scale * rate).min(1.0) / rate;
    }
    if eff.poison > 0.0 {
        w += (POISON_DOSE * eff.poison * scale).min(POISON_CAP / (POISON_TIME * rate))
            * POISON_CREDIT;
    }
    w += FREEZE_WORTH * (eff.chill * scale * rate / 3.0).min(1.0 / FREEZE_TIME) / rate;
    w
}

/// How hard one bolt of `e` hits, so its stream is worth BOLT_DPS times its
/// skill a second.
pub fn bolt_scale(e: El) -> f64 {
    static CACHE: std::sync::OnceLock<[f64; 13]> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        crate::el::ALL.map(|e| {
            let rate = flight(Some(e)).rate;
            let goal = BOLT_DPS * skill(e);
            let (mut lo, mut hi) = (0.001f64, 10.0f64);
            for _ in 0..40 {
                let mid = (lo * hi).sqrt();
                if shot_worth(e, mid, rate) * rate < goal {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo * hi).sqrt()
        })
    })[e.idx()]
}

/// How hard each element of a bolt hits. The front element sets how it
/// flies; each element keeps its own worth a second.
pub fn bolt_scales(counts: &[(El, u8)], front: El, mult: f64) -> Vec<(El, f64)> {
    let split = 1.0 / (counts.len() as f64).sqrt();
    let rate = flight(Some(front)).rate;
    counts
        .iter()
        .map(|&(e, c)| {
            (
                e,
                bolt_scale(e) * flight(Some(e)).rate / rate * (0.6 + 0.4 * c as f64) * split * mult,
            )
        })
        .collect()
}

pub fn bolt_dps(e: El) -> f64 {
    let rate = flight(Some(e)).rate;
    shot_worth(e, bolt_scale(e), rate) * rate
}

/// What each base element's trick on an enchanted bolt adds.
fn charm(b: El) -> f64 {
    match b {
        Earth | Air | Void => 1.5,
        Poison => 3.0,
        _ => 0.0,
    }
}

pub fn enchant_store(s: &Spell) -> f64 {
    price(s) * DMG_PER_MANA * target(s)
}

/// What one enchanted bolt is worth.
pub fn enchant_shot(s: &Spell, scales: &[(El, f64)]) -> f64 {
    let rate = flight(s.primary).rate;
    let mut bases: Vec<El> = Vec::new();
    let mut w = 0.0;
    for &(e, sc) in scales {
        w += shot_worth(e, sc, rate);
        for b in e.parts() {
            if !bases.contains(&b) {
                bases.push(b);
            }
        }
    }
    let get = |e: El| scales.iter().find(|c| c.0 == e).map(|c| c.1);
    if bases.contains(&Frost) {
        w *= credited(1.5);
    }
    let fire = get(Fire).or(get(Magma)).or(get(Steam)).unwrap_or(0.0);
    if bases.contains(&Fire) {
        w += shot_worth(Fire, fire.max(bolt_scale(Fire)) * 0.5, rate) * credited(1.5);
    }
    if bases.contains(&Lightning) {
        w += shot_worth(Lightning, bolt_scale(Lightning) * 1.2, rate) * 1.2 * CROWD;
    }
    if bases.contains(&Water) {
        for &(e, sc) in scales {
            if e.effect().shock > 0.0 {
                w += e.effect().dmg * sc;
            }
        }
        if bases.contains(&Lightning) {
            w += Lightning.effect().dmg * bolt_scale(Lightning) * 1.2 * 1.2 * CROWD;
        }
    }
    for b in [Earth, Air, Void, Poison] {
        if bases.contains(&b) {
            w += charm(b);
        }
    }
    w
}

/// What one enchanted swing is worth.
pub fn enchant_swing(s: &Spell) -> f64 {
    let mut w = 0.0;
    for &(e, c) in &s.counts {
        let eff = e.effect();
        let n = c as f64 * ENCHANT_SWING;
        w += eff.dmg * n;
        w += utility(eff, n);
        w += over_time(BURN_DPS, BURN_MAX.min(4.0 * eff.burn * n), 0.0);
        w += POISON_CAP.min(POISON_DOSE * eff.poison * n) * POISON_CREDIT;
        w += FREEZE_WORTH * (eff.chill * n / 3.0).min(1.0).floor();
    }
    if s.counts.iter().any(|c| c.0.effect().wet > 0.0) {
        for &(e, c) in &s.counts {
            if e.effect().shock > 0.0 {
                w += e.effect().dmg * c as f64 * ENCHANT_SWING;
            }
        }
    }
    w * credited(SWING_TARGETS)
}
