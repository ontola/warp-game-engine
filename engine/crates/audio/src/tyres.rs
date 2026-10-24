//! Tyres on the stage: rolling roar, gravel crunch and stones, sliding hiss.

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

pub struct TyreVoice {
    sr: f32,
    target: TyreControls,
    c: TyreControls,
    noise: Noise,
    roar: Biquad,
    crunch: Biquad,
    stones: Biquad,
    hiss: Biquad,
    splash: Biquad,
    body: OnePole,
    stone: f32,
}

impl TyreVoice {
    pub fn new(sr: f32, seed: u32) -> TyreVoice {
        TyreVoice {
            sr,
            target: TyreControls::default(),
            c: TyreControls::default(),
            noise: Noise::new(seed ^ 0x5157),
            roar: Biquad::new(Kind::LowPass, 300.0, 0.7, sr),
            crunch: Biquad::new(Kind::BandPass, 1800.0, 0.8, sr),
            stones: Biquad::new(Kind::BandPass, 3200.0, 3.0, sr),
            hiss: Biquad::new(Kind::BandPass, 2400.0, 0.6, sr),
            splash: Biquad::new(Kind::BandPass, 900.0, 0.7, sr),
            body: OnePole::new(120.0, sr),
            stone: 0.0,
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
            let mut y = self.roar.run(w) * v * (0.25 + 0.5 * firm) * 0.6;
            // Gravel: crunch, and stones flicked into the arches.
            let crunch_amt = c.loose * smoothstep(0.5, 25.0, c.speed + c.slip * 2.0);
            y += self.crunch.run(w) * crunch_amt * 0.5;
            let rate = c.loose * (c.speed * 3.0 + c.slip * 12.0) * c.contact;
            if self.noise.unit() < rate * dt {
                self.stone = 0.6 + self.noise.unit();
            }
            y += self.stones.run(self.noise.white() * self.stone) * 1.4;
            self.stone *= (-dt / 0.006).exp();
            // Sliding: hiss on gravel, a squeal on asphalt.
            let slide = smoothstep(0.5, 8.0, c.slip) * c.contact;
            y += self.hiss.run(w) * slide * (0.3 + 0.6 * c.loose);
            // Grass swish, water splash.
            y += self.splash.run(w) * c.soft * v * 0.4;
            // The body: thumps of the road through the shell.
            y += self.body.run(w) * v * 0.3;
            *o += y * c.contact * gain;
        }
    }
}
