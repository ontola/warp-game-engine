//! The ion cannon's game-specific recipe, after `scripts/ion_cannon.gd`.
//! Spawn one soft charge column and its future motes once; Warp evaluates
//! their animation on the GPU. The CPU keeps only strike identity and lights.
use glam::Vec3;
use growing_guns::{Strike, StrikeKind, World, ION_CHARGE, TICK_HZ};
use render::fx::{AdditiveColumn, AdditiveMote, AdditiveStreak, Effects};
use render::headlights::LocalLight;

const CHARGE: f32 = ION_CHARGE as f32;
const SKY_HEIGHT: f32 = 2200.0;
const BOTTOM_DEPTH: f32 = 55.0;
const START_RADIUS: f32 = 30.0;
const END_RADIUS: f32 = 1.5;
const MOTE_INTERVAL: f32 = 0.065;
const MAX_CHARGES: usize = 16;
const MAX_DETONATIONS: usize = 32;
const MAX_LIGHTS: usize = 16;
const MAX_CD: f32 = 100_000_000.0;
const MOTE_NITS: f32 = 1600.0;
const DETONATION_COLUMN_NITS: f32 = 2.0;
const DETONATION_STREAK_NITS: f32 = 30.0;
const ION_COLOR: Vec3 = Vec3::new(0.38, 0.78, 1.0);
const ION_CORE: Vec3 = Vec3::new(0.92, 0.97, 1.0);
const FLASH_SUSTAIN: f32 = 0.05;
const FLASH_RELEASE: f32 = 0.30;

#[derive(Clone, Copy, PartialEq, Eq)]
struct StrikeKey { round: u32, owner: Option<usize>, target: [u32; 3], birth_tick: i128 }

fn strike_key(tick: u64, round: u32, strike: &Strike) -> StrikeKey {
    StrikeKey { round, owner: strike.owner, target: strike.target.to_array().map(f32::to_bits),
        birth_tick: tick as i128 - (strike.age * TICK_HZ as f64).round() as i128 }
}

struct Charge { key: StrikeKey, position: Vec3, birth: f32, light_scale: f32 }
struct Detonation { position: Vec3, birth: f32, radius: f32, light_scale: f32 }

#[derive(Default)]
pub struct IonFx { charges: Vec<Charge>, detonations: Vec<Detonation> }

impl IonFx {
    /// Call alongside Effects::clear when changing arenas or resetting time.
    pub fn reset(&mut self) { self.charges.clear(); self.detonations.clear(); }

    /// Safe to call more than once per simulation tick, or after a snapshot.
    /// Matching uses simulation birth ticks, not a render-frame float clock.
    pub fn sync(&mut self, world: &World, effects: &mut Effects, now: f32) {
        if !now.is_finite() { return; }
        self.detonations.retain(|d| now < d.birth + FLASH_SUSTAIN + FLASH_RELEASE);
        self.charges.retain(|c| now < c.birth + CHARGE && world.strikes.iter().any(|s|
            s.kind == StrikeKind::IonCannon && s.age.is_finite() && strike_key(world.tick, world.round, s) == c.key));
        for strike in &world.strikes {
            if strike.kind != StrikeKind::IonCannon || !strike.target.is_finite()
                || !strike.age.is_finite() || !(0.0..ION_CHARGE).contains(&strike.age) { continue; }
            let key = strike_key(world.tick, world.round, strike);
            if self.charges.iter().any(|c| c.key == key) || self.charges.len() >= MAX_CHARGES { continue; }
            let birth = now - strike.age as f32;
            let position = strike.target;
            for column in charge_columns(position, birth) {
                effects.column(column).expect("valid ion charge recipe");
            }
            // Past motes are also registered when joining a charge midway;
            // the GPU samples their current age, rather than restarting them.
            for mote in charge_motes(position, birth, seed(key)) {
                if mote.birth + mote.life > now {
                    effects.mote(mote).expect("valid ion mote recipe");
                }
            }
            self.charges.push(Charge { key, position, birth, light_scale: light_scale(effects) });
        }
    }

    /// `lava_y`: the lava's surface; the column reaches 20 m below it.
    pub fn detonate(&mut self, effects: &mut Effects, now: f32, position: Vec3, radius: f32, lava_y: f32) {
        if !now.is_finite() || !position.is_finite() || !radius.is_finite() || radius <= 0.0 { return; }
        self.detonations.retain(|d| now < d.birth + FLASH_SUSTAIN + FLASH_RELEASE);
        if self.detonations.iter().any(|d| d.birth == now && d.position == position) { return; }
        if self.detonations.len() >= MAX_DETONATIONS { self.detonations.remove(0); }
        self.charges.retain(|c| c.position != position);
        let radius = radius.min(200.0);
        let bottom = lava_y - 20.0;
        let tint = ION_CORE.lerp(ION_COLOR, 0.18);
        // Three contiguous phases preserve the original flash's rise, short
        // hot peak and release instead of an ordinary orange explosion.
        for (delay, life, opacity, radiance, power) in [
            (0.0, 0.012, [0.0, 0.72], [0.0, 14_000.0], 0.2),
            (0.012, 0.018, [0.72, 0.45], [14_000.0, 5200.0], 0.7),
            (0.030, 0.170, [0.45, 0.0], [5200.0, 0.0], 1.0),
        ] {
            effects.column(AdditiveColumn {
                position: Vec3::new(position.x, bottom, position.z), birth: now + delay, life,
                height: (position.y + 880.0 - bottom).max(12.0), radius: [radius; 2],
                color: [tint; 2], opacity, radiance: radiance.map(|v| v * DETONATION_COLUMN_NITS),
                opacity_power: power, radiance_power: power, ..Default::default()
            }).expect("valid ion detonation volume recipe");
        }
        let mut rng = Rng(position.to_array().iter().fold(0x2545_f491_4f6c_dd1du64,
            |s, v| s.rotate_left(13) ^ v.to_bits() as u64) ^ radius.to_bits() as u64);
        for _ in 0..40 {
            let length = rng.range(16.0, 30.0);
            let width = rng.range(0.09, 0.22);
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let dist = rng.unit().sqrt() * radius * 0.88;
            let start = position + Vec3::new(angle.cos() * dist, rng.range(-20.0, 20.0), angle.sin() * dist);
            let life = rng.range(0.28, 0.62);
            let travel = Vec3::new(rng.range(-1.5, 1.5), rng.range(70.0, 120.0) * life, rng.range(-1.5, 1.5));
            effects.streak(AdditiveStreak { position: start, travel, birth: now, life,
                radius: [width; 2], length: [length; 2], color: Vec3::ONE,
                radiance: rng.range(420.0, 900.0) * DETONATION_STREAK_NITS,
                opacity: rng.range(0.7, 1.0), fade_in: 0.0, fade_out: 1.0, motion_power: 1.0,
            }).expect("valid ion detonation streak recipe");
        }
        self.detonations.push(Detonation { position, birth: now, radius, light_scale: light_scale(effects) });
    }

    pub fn lights(&self, clock: f32) -> Vec<LocalLight> {
        let mut lights = Vec::with_capacity(MAX_LIGHTS);
        if !clock.is_finite() { return lights; }
        // Detonation lights take priority over charging markers in a crowd.
        for d in self.detonations.iter().rev() {
            let age = clock - d.birth;
            let envelope = flash_envelope(age);
            if envelope <= 0.0 { continue; }
            push_light(&mut lights, d.position + Vec3::Y * 2.0,
                Vec3::new(0.96, 0.99, 1.0) * (22_000.0 * d.light_scale * envelope), d.radius * 6.0);
            for i in 0..4 {
                let duration = 0.17 * (0.75 + i as f32 * 0.05);
                let strength = if age < 0.012 { ease_out((age / 0.012).clamp(0.0, 1.0)) }
                    else { (1.0 - (age - 0.012) / duration).clamp(0.0, 1.0) };
                if strength > 0.0 {
                    push_light(&mut lights, d.position + Vec3::Y * (4.0 + i as f32 * 6.0),
                        ION_CORE * ((5200.0 - i as f32 * 650.0) * d.light_scale * strength), d.radius * 3.2);
                }
            }
        }
        for c in &self.charges {
            let age = clock - c.birth;
            if !(0.0..CHARGE).contains(&age) { continue; }
            let u = age / CHARGE;
            let power = u.powf(3.0);
            let ignite = smoothstep(0.48, 0.985, u);
            let tint = ION_COLOR.lerp(ION_CORE, ignite * 0.6);
            // Sample the emitting column above the ground. A lamp only 40 cm
            // high grazed the floor, illuminating walls but barely the paving.
            // The three heights give a bright foot and broad blue-white spill.
            for (height,energy) in [(4.0,650.0),(12.0,500.0),(24.0,350.0)] {
                push_light(&mut lights, c.position + Vec3::Y * height,
                    tint * ((0.3 + energy * power) * c.light_scale), 58.0);
            }
        }
        lights
    }

    /// Optical bloom follows the charge and brief impact; no permanent change
    /// to the scene's look after the Ion effect has finished.
    pub fn glow(&self, clock: f32) -> f32 {
        if !clock.is_finite() { return 0.0; }
        let charge=self.charges.iter().filter_map(|c| {
            let age=clock-c.birth;
            (0.0..CHARGE).contains(&age).then(||smoothstep(0.4,0.9,age/CHARGE))
        }).fold(0.0,f32::max);
        self.detonations.iter().map(|d|flash_envelope(clock-d.birth)).fold(charge,f32::max)
    }

    /// A brief, distance-weighted exposure flash; it never becomes a lasting
    /// full-screen white layer and each split-screen camera gets its own value.
    pub fn flash(&self, clock: f32, eye: Vec3) -> f32 {
        if !clock.is_finite() || !eye.is_finite() { return 0.0; }
        self.detonations.iter().map(|d| {
            let distance = eye.distance(d.position);
            let weight = 1.0 - smoothstep(d.radius * 0.5, d.radius * 4.0, distance);
            0.72 * weight * flash_envelope(clock - d.birth)
        }).fold(0.0, f32::max)
    }
}

fn charge_columns(position: Vec3, birth: f32) -> [AdditiveColumn; 1] {
    [AdditiveColumn { position: position - Vec3::Y * BOTTOM_DEPTH, birth, life: CHARGE,
        height: SKY_HEIGHT + BOTTOM_DEPTH, radius: [START_RADIUS, END_RADIUS],
        radius_power: 2.0, color: [ION_COLOR, ION_CORE], color_transition: [0.45, 0.98],
        // One continuous volume: barely visible at first, then a smooth white-blue ignition.
        opacity: [0.008, 0.95], opacity_power: 1.6,
        radiance: [600.0, 20_000.0], radiance_power: 1.8,
        pulse_radians_per_second: 0.0, pulse_radiance: 0.0,
        bottom_radius_scale: 1.0 }]
}

fn charge_motes(position: Vec3, birth: f32, seed: u64) -> Vec<AdditiveMote> {
    let mut rng = Rng(seed);
    let mut motes = Vec::with_capacity(400);
    for tick in 0..=(CHARGE / MOTE_INTERVAL) as u32 {
        // Uniform by area across the full footprint, with independent births
        // rather than visible waves. The initial field fades in asynchronously.
        let count = if tick == 0 { 48 } else { 6 };
        for _ in 0..count {
            let at = tick as f32 * MOTE_INTERVAL + rng.range(0.0, if tick == 0 { 0.45 } else { MOTE_INTERVAL });
            if CHARGE - at < 0.8 { continue; }
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let dist = rng.unit().sqrt() * growing_guns::ION_RADIUS as f32;
            let start = position + Vec3::new(angle.cos() * dist, rng.range(0.12, 0.9), angle.sin() * dist);
            let nominal_life = rng.range(2.4, 3.6);
            let life = nominal_life.min(CHARGE - at);
            let power = rng.range(2.4, 3.6);
            // Truncate the original trajectory at impact: late births must not
            // compress a full rise into their shorter remaining lifetime.
            let rise = rng.range(1.2, 2.6) * nominal_life / power * (life / nominal_life).powf(power);
            let travel = Vec3::new(rng.range(-0.06, 0.06), 1.0, rng.range(-0.06, 0.06)) * rise;
            let fade_in = (rng.range(0.45, 0.95) / life).min(0.8);
            motes.push(AdditiveMote { position: start, travel, birth: birth + at, life,
                radius: rng.range(0.025, 0.045),
                color: Vec3::new(0.12, rng.range(0.42, 0.65), 1.0),
                radiance: rng.range(22.0, 36.0) * MOTE_NITS,
                opacity: rng.range(0.50, 0.80), fade_in,
                fade_out: rng.range(0.22, 0.35).min(1.0 - fade_in), motion_power: power });
        }
    }
    motes
}

fn light_scale(effects: &Effects) -> f32 {
    if effects.light_scale.is_finite() { effects.light_scale.clamp(0.0, 1_000_000.0) } else { 37_000.0 }
}

fn push_light(lights: &mut Vec<LocalLight>, position: Vec3, intensity: Vec3, range: f32) {
    if lights.len() >= MAX_LIGHTS { return; }
    lights.push(LocalLight { position, direction: Vec3::NEG_Y,
        intensity_cd: intensity.clamp(Vec3::ZERO, Vec3::splat(MAX_CD)), range_m: range.clamp(0.1, 200.0),
        inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.35, falloff: 2.0 });
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 { let t = ((x - a) / (b - a)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
fn ease_out(t: f32) -> f32 { if t >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) } }
fn flash_envelope(age: f32) -> f32 {
    if !(0.0..FLASH_SUSTAIN + FLASH_RELEASE).contains(&age) { return 0.0; }
    if age <= FLASH_SUSTAIN { return 1.0; }
    let release = (age - FLASH_SUSTAIN) / FLASH_RELEASE;
    1.0 - 2f32.powf(10.0 * (release - 1.0))
}

fn seed(key: StrikeKey) -> u64 {
    let mut value = key.birth_tick as u64 ^ (key.round as u64).rotate_left(31) ^ key.owner.unwrap_or(usize::MAX) as u64;
    for part in key.target { value = value.rotate_left(17) ^ part as u64; }
    value ^ 0x2545_f491_4f6c_dd1d
}

struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f32 {
        if self.0 == 0 { self.0 = 0x2545_f491_4f6c_dd1d; }
        self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 { lo + (hi - lo) * self.unit() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_identity_survives_ticks_and_float_age_accumulation() {
        let mut strike = Strike { kind: StrikeKind::IonCannon, target: Vec3::new(4.0, 0.0, 6.0), owner: Some(0), age: 0.0 };
        let first = strike_key(200, 1, &strike);
        for tick in 1..200 {
            strike.age += growing_guns::DT64;
            assert!(strike_key(200 + tick, 1, &strike) == first);
        }
        assert!(strike_key(400, 2, &strike) != first);
    }

    #[test]
    fn charge_motes_fade_in_accelerate_and_end_without_rushing_late_births() {
        let motes = charge_motes(Vec3::ZERO, 10.0, 5);
        assert!((300..450).contains(&motes.len()));
        let outer=motes.iter().filter(|m| m.position.length()>growing_guns::ION_RADIUS as f32*0.7).count();
        assert!(outer>motes.len()/3, "the whole disc receives motes");
        assert!(motes.windows(2).any(|m| (m[0].birth - m[1].birth).abs() > 0.01));
        let mut effects = Effects::default();
        for mote in motes {
            effects.mote(mote).expect("valid soft-dot recipe");
            assert!(mote.birth >= 10.0);
            assert!(mote.birth + mote.life <= 10.0 + CHARGE + 0.00001);
            // First fifth of life covers under 3% of the rise; speed increases.
            assert!(0.2_f32.powf(mote.motion_power) < 0.03);
            let speed = |age: f32| mote.travel.y / mote.life * mote.motion_power * age.powf(mote.motion_power - 1.0);
            assert!(speed(0.1) < 0.11);
            assert!(speed(0.75) > speed(0.25) * 4.0);
            assert!(speed(1.0) <= 2.601, "late births keep the same speed ceiling");
            assert!(mote.fade_in * mote.life >= 0.449);
            assert!((0.025..=0.045).contains(&mote.radius));
            assert!(mote.travel.x.abs().max(mote.travel.z.abs()) <= mote.travel.y * 0.061);
        }
    }

    #[test]
    fn charge_energy_fits_hdr_and_builds_toward_detonation() {
        let columns = charge_columns(Vec3::ZERO, 0.0);
        assert_eq!(columns.len(), 1, "no nested charge shells");
        // The volume response saturates at 3.8, then the GPU bounds HDR.
        let radiance = |u: f32| columns.iter().map(|c| {
            let power = u.powf(c.radiance_power);
            let alpha = c.opacity[0] + (c.opacity[1] - c.opacity[0]) * u.powf(c.opacity_power);
            let light = c.radiance[0] + (c.radiance[1] - c.radiance[0]) * power
                + (u * CHARGE * c.pulse_radians_per_second).sin() * c.pulse_radiance * power;
            light * alpha * 3.8
        }).sum::<f32>();
        for step in 0..=1000 {
            let u = step as f32 / 1000.0;
            assert!(radiance(u) < 80_000.0);
            if step > 0 { assert!(radiance(u) >= radiance(u - 0.001)); }
        }
        assert!(radiance(1.0) > radiance(0.0) * 20.0);
        assert!(radiance(0.25) < 3000.0, "the first second stays transparent");
    }

    #[test]
    fn charge_lights_illuminate_ground_at_a_distance_and_build_up() {
        let mut fx=IonFx::default();
        fx.charges.push(Charge {key:StrikeKey {round:1,owner:Some(0),target:[0;3],birth_tick:0},position:Vec3::ZERO,birth:0.0,light_scale:37_000.0});
        let floor=Vec3::new(10.0,0.0,0.0);
        let floor_lux=|time| fx.lights(time).iter().map(|l| {
            let to=l.position-floor;
            let d2=to.length_squared();
            let fade=(1.0-(d2/(l.range_m*l.range_m)).powi(2)).max(0.0);
            l.intensity_cd.y/d2 * to.normalize().y.max(0.0)*fade*fade
        }).sum::<f32>();
        assert!(floor_lux(CHARGE*0.95)>60_000.0,"late beam clearly lights the paving even in daylight");
        assert!(floor_lux(CHARGE*0.15)<floor_lux(CHARGE*0.95)*0.01);
        assert_eq!(fx.lights(CHARGE).len(),0);
        assert_eq!(fx.glow(0.0),0.0);
        assert_eq!(fx.glow(CHARGE*0.95),1.0);
        assert_eq!(fx.glow(CHARGE),0.0);
    }

    #[test]
    fn recipes_are_valid_and_flash_expires() {
        let mut effects = Effects::default();
        for column in charge_columns(Vec3::ZERO, 0.0) { effects.column(column).unwrap(); }
        let mut ion = IonFx::default();
        ion.detonate(&mut effects, 4.0, Vec3::ZERO, 38.0, growing_guns::arena::LAVA_KILL_Y);
        assert_eq!(effects.len(), 1 + 3 + 40);
        ion.detonate(&mut effects, 4.0, Vec3::ZERO, 38.0, growing_guns::arena::LAVA_KILL_Y);
        assert_eq!(effects.len(), 1 + 3 + 40);
        assert_eq!(ion.flash(3.99, Vec3::ZERO), 0.0);
        assert!(ion.flash(4.01, Vec3::ZERO) > 0.7);
        assert_eq!(ion.flash(4.4, Vec3::ZERO), 0.0);
        for light in ion.lights(4.02) {
            assert!(light.intensity_cd.max_element() <= MAX_CD);
            assert!(light.range_m <= 200.0);
        }
        ion.reset();
        assert!(ion.lights(4.02).is_empty());
        assert_eq!(ion.flash(4.02, Vec3::ZERO), 0.0);
    }
}
