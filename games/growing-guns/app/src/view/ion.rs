//! The ion cannon's game-specific recipe, after `scripts/ion_cannon.gd`.
//! Spawn the nested charge shells and their future motes once; Warp evaluates
//! their animation on the GPU. The CPU keeps only strike identity and lights.
use glam::Vec3;
use growing_guns::{Strike, StrikeKind, World, ION_CHARGE, TICK_HZ};
use render::fx::{AdditiveColumn, AdditiveStreak, Effects};
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
// Photometric calibration for Warp's absolute-nit HDR target. Godot's very
// large late emission values cannot be multiplied by the exposure white:
// overlapping shells would saturate fp16 long before the intended ignition.
// Keep the source's timings/curves and fit their combined energy to the target.
const CHARGE_START_NITS: f32 = 1000.0;
const CHARGE_END_NITS: f32 = 2.0;
const MOTE_NITS: f32 = 1000.0;
const DETONATION_SHELL_NITS: f32 = 2.0;
const DETONATION_STREAK_NITS: f32 = 30.0;
const ION_COLOR: Vec3 = Vec3::new(0.38, 0.78, 1.0);
const ION_CORE: Vec3 = Vec3::new(0.92, 0.97, 1.0);
const FLASH_SUSTAIN: f32 = 0.05;
const FLASH_RELEASE: f32 = 0.30;

#[derive(Clone, Copy)]
struct Layer { radius: f32, alpha: [f32; 2], emission: [f32; 2], color: Vec3 }

const LAYERS: [Layer; 8] = [
    Layer { radius: 0.08, alpha: [0.035, 1.00], emission: [5.0, 5200.0], color: Vec3::new(0.42, 0.72, 0.96) },
    Layer { radius: 0.15, alpha: [0.028, 0.92], emission: [4.0, 3400.0], color: Vec3::new(0.46, 0.76, 0.98) },
    Layer { radius: 0.25, alpha: [0.022, 0.78], emission: [3.2, 2100.0], color: Vec3::new(0.40, 0.74, 0.98) },
    Layer { radius: 0.38, alpha: [0.018, 0.58], emission: [2.6, 1200.0], color: Vec3::new(0.36, 0.70, 0.98) },
    Layer { radius: 0.52, alpha: [0.014, 0.42], emission: [2.0, 680.0], color: Vec3::new(0.32, 0.66, 0.96) },
    Layer { radius: 0.68, alpha: [0.010, 0.28], emission: [1.5, 360.0], color: Vec3::new(0.28, 0.62, 0.94) },
    Layer { radius: 0.84, alpha: [0.008, 0.18], emission: [1.1, 190.0], color: Vec3::new(0.24, 0.58, 0.90) },
    Layer { radius: 1.00, alpha: [0.006, 0.12], emission: [0.8, 95.0], color: Vec3::new(0.20, 0.54, 0.86) },
];

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
                    effects.streak(mote).expect("valid ion mote recipe");
                }
            }
            self.charges.push(Charge { key, position, birth, light_scale: light_scale(effects) });
        }
    }

    pub fn detonate(&mut self, effects: &mut Effects, now: f32, position: Vec3, radius: f32) {
        if !now.is_finite() || !position.is_finite() || !radius.is_finite() || radius <= 0.0 { return; }
        self.detonations.retain(|d| now < d.birth + FLASH_SUSTAIN + FLASH_RELEASE);
        if self.detonations.iter().any(|d| d.birth == now && d.position == position) { return; }
        if self.detonations.len() >= MAX_DETONATIONS { self.detonations.remove(0); }
        self.charges.retain(|c| c.position != position);
        let radius = radius.min(200.0);
        let bottom = growing_guns::arena::LAVA_KILL_Y - 20.0;
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
                color: [tint; 2], opacity, radiance: radiance.map(|v| v * DETONATION_SHELL_NITS),
                opacity_power: power, radiance_power: power, ..Default::default()
            }).expect("valid ion detonation shell recipe");
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
            let power = u.powf(2.35);
            let focus = u * u;
            let ignite = smoothstep(0.48, 0.985, u);
            let energy = 3.0 + (240.0 - 3.0) * power + (age * 9.0).sin() * 10.0 * power;
            push_light(&mut lights, c.position + Vec3::Y * 0.4,
                ION_COLOR.lerp(Vec3::ONE, ignite * 0.75) * energy * c.light_scale, 30.0 - 8.0 * focus);
        }
        lights
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

fn charge_columns(position: Vec3, birth: f32) -> [AdditiveColumn; 8] {
    std::array::from_fn(|i| {
        let layer = LAYERS[i];
        AdditiveColumn { position: position - Vec3::Y * BOTTOM_DEPTH, birth, life: CHARGE,
            height: SKY_HEIGHT + BOTTOM_DEPTH, radius: [START_RADIUS * layer.radius, END_RADIUS * layer.radius],
            radius_power: 2.0, color: [layer.color, Vec3::ONE], color_transition: [0.48, 0.985],
            opacity: layer.alpha, opacity_power: 4.2,
            radiance: [layer.emission[0] * CHARGE_START_NITS, layer.emission[1] * CHARGE_END_NITS], radiance_power: 2.35,
            pulse_radians_per_second: 9.0, pulse_radiance: 14.0 * (1.0 - i as f32 * 0.1) * CHARGE_END_NITS,
            bottom_radius_scale: 1.03 }
    })
}

fn charge_motes(position: Vec3, birth: f32, seed: u64) -> Vec<AdditiveStreak> {
    let mut rng = Rng(seed);
    let mut motes = Vec::with_capacity(96);
    for tick in 1..=(CHARGE / MOTE_INTERVAL) as u32 {
        let at = tick as f32 * MOTE_INTERVAL;
        let u = at / CHARGE;
        let power = u.powf(2.35);
        if power < 0.12 { continue; }
        let radius = START_RADIUS + (END_RADIUS - START_RADIUS) * u * u;
        let count = if power < 0.42 { 1 } else if power < 0.78 { 2 } else { 3 };
        for _ in 0..count {
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let dist = rng.unit() * radius * 0.82;
            let start = position + Vec3::new(angle.cos() * dist, rng.range(0.15, 2.5), angle.sin() * dist);
            let width = rng.range(0.04, 0.11);
            let full_life = rng.range(2.2, 4.2);
            let life = full_life.min(CHARGE - at);
            if life <= 0.0 { continue; }
            let fraction = life / full_life;
            // The original kills charge motes on detonation. Shortening life
            // must shorten travel too, otherwise the final motes shoot upward.
            let travel = Vec3::new(rng.range(-1.2, 1.2), rng.range(12.0, 34.0), rng.range(-1.2, 1.2)) * fraction;
            let end_width = width * (1.0 + (rng.range(1.4, 2.4) - 1.0) * fraction);
            motes.push(AdditiveStreak { position: start, travel, birth: birth + at, life,
                radius: [width, end_width], length: [width * 2.0, end_width * 2.0],
                color: Vec3::new(0.82, 0.94, 1.0), radiance: rng.range(32.0, 58.0) * MOTE_NITS,
                opacity: rng.range(0.32, 0.62), fade_in: 0.28, fade_out: 0.72, motion_power: 1.0 });
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
    fn charge_motes_end_at_detonation_without_speeding_up() {
        let motes = charge_motes(Vec3::ZERO, 10.0, 5);
        assert!((40..100).contains(&motes.len()));
        for mote in motes {
            assert!(mote.birth >= 10.0 + CHARGE * 0.12f32.powf(1.0 / 2.35));
            assert!(mote.birth + mote.life <= 10.0 + CHARGE + 0.00001);
            assert!((12.0 / 4.2..=34.0 / 2.2).contains(&(mote.travel.y / mote.life)));
            assert_eq!(mote.motion_power, 1.0);
        }
    }

    #[test]
    fn charge_energy_fits_hdr_and_builds_toward_detonation() {
        let columns = charge_columns(Vec3::ZERO, 0.0);
        // A line through the axis can see both faces of every nested shell.
        // This conservative scalar sum bounds all RGB components, including
        // the pulse, before the fragment shader's final fp16 guard.
        let radiance = |u: f32| columns.iter().map(|c| {
            let power = u.powf(c.radiance_power);
            let alpha = c.opacity[0] + (c.opacity[1] - c.opacity[0]) * u.powf(c.opacity_power);
            let light = c.radiance[0] + (c.radiance[1] - c.radiance[0]) * power
                + (u * CHARGE * c.pulse_radians_per_second).sin() * c.pulse_radiance * power;
            light * alpha * 2.0
        }).sum::<f32>();
        for step in 0..=1000 { assert!(radiance(step as f32 / 1000.0) < 55_000.0); }
        assert!(radiance(1.0) > radiance(0.0) * 20.0);
        assert!(radiance(0.25) < 2000.0, "the first second stays transparent");
    }

    #[test]
    fn recipes_are_valid_and_flash_expires() {
        let mut effects = Effects::default();
        for column in charge_columns(Vec3::ZERO, 0.0) { effects.column(column).unwrap(); }
        let mut ion = IonFx::default();
        ion.detonate(&mut effects, 4.0, Vec3::ZERO, 38.0);
        assert_eq!(effects.len(), 8 + 3 + 40);
        ion.detonate(&mut effects, 4.0, Vec3::ZERO, 38.0);
        assert_eq!(effects.len(), 8 + 3 + 40);
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
