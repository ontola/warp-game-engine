//! Tyres on the stage: rolling roar, gravel crunch and stones. Gravel is
//! made of grains (short random bursts, more of them the faster the tyres
//! roll and slide), not a steady hiss: steady band-passed noise reads as
//! wind or static, not as stones.

use crate::dsp::{Biquad, Kind, Noise, OnePole, approach, smoothstep};

/// What the wheels are doing, summed over the wheels in contact.
#[derive(Clone, Copy, Debug, Default)]
pub struct TyreControls {
    /// Road speed (m/s).
    pub speed: f32,
    /// Sliding speed at the contact patches (m/s), summed.
    pub slip: f32,
    /// Fraction of wheels in contact, 0..1.
    pub contact: f32,
    /// Share of loose ground (gravel, loose, dirt, mud) vs. firm (asphalt).
    pub loose: f32,
    /// Share of grass and water.
    pub soft: f32,
    pub wet: f32,
}

/// Tyres' level in the mix next to an engine whose leveler aims at 0.9:
/// rolling sits about 20 dB under full throttle, a slide about 15 dB.
pub const MIX_LEVEL: f32 = 0.25;

pub struct TyreVoice {
    sr: f32,
    target: TyreControls,
    c: TyreControls,
    noise: Noise,
    roar: Biquad,
    crunch: Biquad,
    crunch_lp: Biquad,
    stones: Biquad,
    hiss: Biquad,
    splash: Biquad,
    body: OnePole,
    stone: f32,
    grain: f32,
}

impl TyreVoice {
    pub fn new(sr: f32, seed: u32) -> TyreVoice {
        TyreVoice {
            sr,
            target: TyreControls::default(),
            c: TyreControls::default(),
            noise: Noise::new(seed ^ 0x5157),
            roar: Biquad::new(Kind::LowPass, 300.0, 0.7, sr),
            crunch: Biquad::new(Kind::BandPass, 750.0, 0.9, sr),
            // Grains start with a click; keep it a crunch, not a hiss.
            crunch_lp: Biquad::new(Kind::LowPass, 1300.0, 0.7, sr),
            stones: Biquad::new(Kind::BandPass, 1800.0, 1.2, sr),
            hiss: Biquad::new(Kind::BandPass, 1400.0, 0.7, sr),
            splash: Biquad::new(Kind::BandPass, 900.0, 0.7, sr),
            body: OnePole::new(120.0, sr),
            stone: 0.0,
            grain: 0.0,
        }
    }

    pub fn set(&mut self, c: TyreControls) {
        self.target = c;
    }

    pub fn render_add(&mut self, out: &mut [f32], gain: f32) {
        let dt = 1.0 / self.sr;
        self.roar.set(Kind::LowPass, 120.0 + self.c.speed * 9.0, 0.7, self.sr);
        for o in out.iter_mut() {
            let t = self.target;
            self.c.speed = approach(self.c.speed, t.speed.abs(), 0.03, dt);
            self.c.slip = approach(self.c.slip, t.slip, 0.03, dt);
            self.c.contact = approach(self.c.contact, t.contact, 0.02, dt);
            self.c.loose = approach(self.c.loose, t.loose, 0.05, dt);
            self.c.soft = approach(self.c.soft, t.soft, 0.05, dt);
            let c = self.c;
            let v = smoothstep(0.3, 40.0, c.speed);
            let w = self.noise.white();
            let firm = (1.0 - c.loose - c.soft).max(0.0);
            // Rolling: a low roar on every surface.
            let mut y = self.roar.run(w) * v * (0.6 + 0.6 * firm) * 1.6;
            // Gravel: grains crushed under the tyres; a slide crushes more,
            // lower and louder. Normal cornering slip stays quiet.
            let slide = smoothstep(1.5, 10.0, c.slip) * c.contact;
            let grains = c.loose * c.contact * (c.speed * 8.0 + slide * 300.0);
            if self.noise.unit() < grains * dt {
                self.grain = 0.4 + 0.6 * self.noise.unit();
            }
            self.grain *= (-dt / 0.004).exp();
            let g = self.crunch.run(w * self.grain);
            y += self.crunch_lp.run(g) * (0.6 + 0.6 * slide) * 1.4;
            // Now and then a stone flicked into the arch.
            let rate = c.loose * c.contact * (c.speed * 0.8 + slide * 15.0);
            if self.noise.unit() < rate * dt {
                self.stone = 0.6 + self.noise.unit();
            }
            y += self.stones.run(self.noise.white() * self.stone) * 0.5;
            self.stone *= (-dt / 0.003).exp();
            // Sliding on firm ground: a rubbery scrub (asphalt only).
            y += self.hiss.run(w) * slide * firm * 0.2;
            // Grass swish, water splash.
            y += self.splash.run(w) * c.soft * v * 0.4;
            // The body: thumps of the road through the shell.
            y += self.body.run(w) * v * 0.6;
            *o += y * c.contact * gain;
        }
    }
}
