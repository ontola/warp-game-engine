//! Procedural sounds ported from Godot's `sfx.gd`: bullet impacts on stone
//! and explosions (a saturated bang plus a brown-noise rumble). Clips are
//! cached per variant and size bucket, as in Godot.
use audio::sfx::Clip;
use audio::SAMPLE_RATE;
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// Small seeded generator (variants must sound the same every run).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1) }
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

/// Bullet hitting stone: a highpassed noise click plus, for heavy rounds, a
/// swept-sine thump (`_synth_impact`).
pub fn impact(dmg_ratio: f32, variant: u32) -> Vec<f32> {
    let mut rng = Rng::new((dmg_ratio * 4.0).round() as u64 * 131 + variant as u64 * 7 + 1);
    let click_freq = rng.range(2200.0, 4500.0);
    let click_decay = rng.range(180.0, 320.0);
    let dmg_norm = ((dmg_ratio - 1.0) / 4.0).clamp(0.0, 1.0);
    let thump_f0 = rng.range(160.0, 260.0);
    let thump_f1 = rng.range(45.0, 80.0) * lerp(1.0, 0.7, dmg_norm);
    let thump_sweep = rng.range(60.0, 100.0);
    let thump_decay = rng.range(28.0, 42.0);
    let thump_gain = lerp(0.0, 0.65, dmg_norm);
    let dur = lerp(0.030, 0.080, dmg_norm) * rng.range(0.92, 1.10);
    let n = (dur * SAMPLE_RATE) as usize;
    let (mut lp, mut phase) = (0.0f32, 0.0f32);
    let hp_k = (click_freq * TAU * 0.4 / SAMPLE_RATE).clamp(0.05, 0.6);
    (0..n).map(|i| {
        let t = i as f32 / SAMPLE_RATE;
        let noise = rng.range(-1.0, 1.0);
        lp = lerp(lp, noise, hp_k);
        let click = (noise - lp) * (-t * click_decay).exp();
        let f = thump_f1 + (thump_f0 - thump_f1) * (-t * thump_sweep).exp();
        phase += TAU * f / SAMPLE_RATE;
        let thump = phase.sin() * (-t * thump_decay).exp() * thump_gain;
        ((click + thump) * 1.4).tanh()
    }).collect()
}

/// The transient: thwack, swept noise, kick and sub rumble through one
/// tanh (`_synth_explosion`).
pub fn bang(radius: f32, variant: u32) -> Vec<f32> {
    let mut rng = Rng::new(radius.round() as u64 * 977 + variant as u64 * 13 + 2);
    let bright = rng.range(0.85, 1.15);
    let thwack_decay = rng.range(180.0, 280.0);
    let thwack_drive = rng.range(4.0, 8.0);
    let dur_jit = rng.range(0.92, 1.10);
    let r = (radius / 6.0).clamp(0.3, 4.0);
    let kick_f0 = 220.0 * rng.range(0.88, 1.12);
    let kick_f1 = (55.0 / r.sqrt()).clamp(28.0, 80.0) * rng.range(0.92, 1.10);
    let kick_sweep = rng.range(70.0, 95.0);
    let kick_decay = (35.0 / r.sqrt()).clamp(10.0, 40.0);
    let kick_gain = lerp(0.55, 0.95, ((r - 1.0) / 3.0).clamp(0.0, 1.0));
    let sub_k = rng.range(0.005, 0.010);
    let sub_decay = (3.5 / r.sqrt()).clamp(1.2, 4.0);
    let dur = ((0.4 + radius * 0.08) * dur_jit).clamp(0.4, 1.6);
    let n = (dur * SAMPLE_RATE) as usize;
    let thwack_k = rng.range(0.10, 0.16);
    let brightness = (0.45 * bright).clamp(0.05, 0.95);
    let noise_decay = rng.range(8.0, 15.0);
    let (mut lp, mut tlp, mut slp, mut kphase, mut klp) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    (0..n).map(|i| {
        let t = i as f32 / SAMPLE_RATE;
        let env = (-t * 1.4).exp() * (1.0 - t / dur).max(0.0).powf(0.6);
        let noise = rng.range(-1.0, 1.0);
        tlp = lerp(tlp, rng.range(-1.0, 1.0), thwack_k);
        let thwack = (tlp * thwack_drive).tanh() * 1.5 * (-t * thwack_decay).exp();
        lp = lerp(lp, noise, brightness);
        let f = kick_f1 + (kick_f0 - kick_f1) * (-t * kick_sweep).exp();
        kphase += 2.0 * PI * f / SAMPLE_RATE;
        klp = lerp(klp, kphase.sin(), 0.30);
        let kick = klp * (-t * kick_decay).exp();
        slp = lerp(slp, rng.range(-1.0, 1.0), sub_k);
        let sub = slp * 3.5 * (-t * sub_decay).exp();
        ((thwack + lp * 0.5 * (-t * noise_decay).exp() + kick * kick_gain + sub * 0.7) * env * 1.4).tanh()
    }).collect()
}

/// The body: lowpassed brown noise, longer and deeper for bigger blasts
/// (`_synth_explosion_rumble`).
pub fn rumble(radius: f32, variant: u32) -> Vec<f32> {
    let mut rng = Rng::new(radius.round() as u64 * 389 + variant as u64 * 17 + 3);
    let rn = radius.clamp(2.0, 24.0) / 24.0;
    let dur = (0.3 + rn * rn * 4.7).clamp(0.3, 5.0);
    let n = (dur * SAMPLE_RATE) as usize;
    let cutoff = (220.0 - radius * 7.0).clamp(55.0, 200.0) + rng.range(-15.0, 15.0);
    let k = (TAU * cutoff / SAMPLE_RATE).clamp(0.005, 0.4);
    let attack = rng.range(0.05, 0.12);
    let ease = rng.range(2.4, 3.0);
    let gain = 1.5 * (6.0 / radius.max(6.0)).sqrt();
    let body = (dur - attack).max(1e-4);
    let (mut brown, mut lp) = (0.0f32, 0.0f32);
    (0..n).map(|i| {
        let t = i as f32 / SAMPLE_RATE;
        let a = (t / attack).clamp(0.0, 1.0);
        let d = (1.0 - ((t - attack) / body).clamp(0.0, 1.0)).powf(ease);
        brown = brown * 0.998 + rng.range(-1.0, 1.0) * 0.03;
        lp = lerp(lp, brown, k);
        (lp * a * d * gain).tanh()
    }).collect()
}

/// Synthesised clips, built on first use.
#[derive(Default)]
pub struct Bank { clips: HashMap<(u8, u32, u32), Clip> }

impl Bank {
    fn get(&mut self, kind: u8, bucket: u32, variant: u32, make: impl FnOnce() -> Vec<f32>) -> Clip {
        self.clips.entry((kind, bucket, variant)).or_insert_with(|| Clip(make().into())).clone()
    }
    pub fn impact(&mut self, dmg_ratio: f32, variant: u32) -> Clip {
        let b = (dmg_ratio.clamp(0.5, 6.0) * 4.0).round() as u32;
        self.get(0, b, variant % 4, || impact(b as f32 / 4.0, variant % 4))
    }
    pub fn bang(&mut self, radius: f32, variant: u32) -> Clip {
        let b = radius.clamp(2.0, 24.0).round() as u32;
        self.get(1, b, variant % 3, || bang(b as f32, variant % 3))
    }
    pub fn rumble(&mut self, radius: f32, variant: u32) -> Clip {
        let b = radius.clamp(2.0, 24.0).round() as u32;
        self.get(2, b, variant % 3, || rumble(b as f32, variant % 3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesised_clips_have_godot_lengths_and_stay_in_range() {
        for (clip, lo, hi) in [(impact(1.0, 0), 0.025, 0.04), (impact(5.0, 1), 0.07, 0.09), (bang(9.0, 0), 1.0, 1.25), (rumble(9.0, 2), 0.9, 1.1)] {
            let secs = clip.len() as f32 / SAMPLE_RATE;
            assert!((lo..hi).contains(&secs), "{secs}");
            let peak = clip.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05 && peak <= 1.0, "{peak}");
        }
        assert_eq!(bang(6.0, 1), bang(6.0, 1), "deterministic");
    }
}
