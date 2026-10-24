//! Small DSP building blocks: filters, delay lines, noise, smoothing.

use std::f32::consts::PI;

/// Deterministic white noise (xorshift32), uniform in [-1, 1).
#[derive(Clone, Debug)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Noise {
        Noise(seed.wrapping_mul(0x9E37_79B9) | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [-1, 1).
    pub fn white(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// Roughly normal (sum of four uniforms), unit variance.
    pub fn gauss(&mut self) -> f32 {
        (self.white() + self.white() + self.white() + self.white()) * 0.866
    }
}

/// One-pole low-pass. `set_hz` keeps state, so the cutoff can glide.
#[derive(Clone, Debug, Default)]
pub struct OnePole {
    a: f32,
    pub y: f32,
}

impl OnePole {
    pub fn new(hz: f32, sr: f32) -> OnePole {
        let mut f = OnePole::default();
        f.set_hz(hz, sr);
        f
    }

    pub fn set_hz(&mut self, hz: f32, sr: f32) {
        self.a = 1.0 - (-2.0 * PI * hz.clamp(1.0, sr * 0.49) / sr).exp();
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// RBJ biquad (Audio EQ Cookbook), transposed direct form II.
#[derive(Clone, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

pub enum Kind {
    LowPass,
    HighPass,
    /// Band-pass with 0 dB peak gain.
    BandPass,
}

impl Biquad {
    pub fn new(kind: Kind, hz: f32, q: f32, sr: f32) -> Biquad {
        let mut b = Biquad::default();
        b.set(kind, hz, q, sr);
        b
    }

    pub fn set(&mut self, kind: Kind, hz: f32, q: f32, sr: f32) {
        let w = 2.0 * PI * hz.clamp(5.0, sr * 0.45) / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.05));
        let (b0, b1, b2) = match kind {
            Kind::LowPass => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0),
            Kind::HighPass => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0),
            Kind::BandPass => (alpha, 0.0, -alpha),
        };
        let a0 = 1.0 + alpha;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = -2.0 * c / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// A delay line read at fractional delays (linear interpolation).
#[derive(Clone, Debug)]
pub struct Delay {
    buf: Vec<f32>,
    pos: usize,
}

impl Delay {
    /// Room for delays up to `max` samples.
    pub fn new(max: usize) -> Delay {
        Delay { buf: vec![0.0; max.next_power_of_two().max(4)], pos: 0 }
    }

    #[inline]
    pub fn write(&mut self, x: f32) {
        self.pos = (self.pos + 1) & (self.buf.len() - 1);
        self.buf[self.pos] = x;
    }

    /// The sample written `d` samples ago (d >= 0; 0 is the latest).
    #[inline]
    pub fn read(&self, d: f32) -> f32 {
        let mask = self.buf.len() - 1;
        let d = d.clamp(0.0, (mask - 1) as f32);
        let i = d as usize;
        let f = d - i as f32;
        let a = self.buf[(self.pos.wrapping_sub(i)) & mask];
        let b = self.buf[(self.pos.wrapping_sub(i + 1)) & mask];
        a + (b - a) * f
    }
}

/// Removes the DC offset of the unipolar exhaust pressure.
#[derive(Clone, Debug, Default)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9975 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Exponential approach to a target, `tau` seconds per e-fold.
#[inline]
pub fn approach(value: f32, target: f32, tau: f32, dt: f32) -> f32 {
    value + (target - value) * (1.0 - (-dt / tau.max(1e-5)).exp())
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
