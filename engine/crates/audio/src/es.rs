//! A port of engine-sim's synthesizer (Ange Yaghi, MIT; src/synthesizer.cpp)
//! and its filters. The exhaust runner pressures, delayed along their pipes,
//! go through:
//!
//! 1. an anti-aliasing low-pass at 1900 Hz;
//! 2. a jitter filter (reads its last 10 samples at a random, smoothed
//!    offset);
//! 3. DC removal;
//! 4. multiplication by band-limited noise ("air noise"), mixed with a
//!    little of the signal's derivative (the high-frequency gain);
//! 5. convolution with a recorded exhaust impulse response;
//! 6. a fast peak leveler.
//!
//! The pressure modulating noise through a real exhaust's response is what
//! makes engine-sim sound like an engine rather than a synthesizer.

use crate::dsp::Noise;
use std::f32::consts::PI;

/// engine-sim's ButterworthLowPassFilter: 4th order, bilinear.
#[derive(Clone, Debug)]
pub struct Butter4 {
    a: [f32; 5],
    f4: f32,
    x: [f32; 4],
    y: [f32; 4],
}

impl Butter4 {
    pub fn new(fc: f32, sr: f32) -> Butter4 {
        let f = (PI * fc.min(sr * 0.49) / sr).tan();
        let (f2, f3, f4) = (f * f, f * f * f, f * f * f * f);
        let m = -2.0 * (5.0 * PI / 8.0).cos();
        let n = -2.0 * (7.0 * PI / 8.0).cos();
        let a0 = 1.0 + (m + n) * f + (2.0 + n * m) * f2 + (m + n) * f3 + f4;
        let a = [
            a0,
            (-4.0 - 2.0 * (n + m) * f + 2.0 * (m + n) * f3 + 4.0 * f4) / a0,
            (6.0 - 2.0 * (2.0 + m * n) * f2 + 6.0 * f4) / a0,
            (-4.0 + 2.0 * (m + n) * f - 2.0 * (m + n) * f3 + 4.0 * f4) / a0,
            (1.0 - (n + m) * f + (2.0 + m * n) * f2 - (m + n) * f3 + f4) / a0,
        ];
        Butter4 { a, f4, x: [0.0; 4], y: [0.0; 4] }
    }

    /// x[0] and y[0] are the most recent.
    #[inline]
    pub fn run(&mut self, s: f32) -> f32 {
        let (x, y, a) = (&self.x, &self.y, &self.a);
        let num = self.f4 / a[0] * (s + 4.0 * x[0] + 6.0 * x[1] + 4.0 * x[2] + x[3]);
        let den = -a[1] * y[0] - a[2] * y[1] - a[3] * y[2] - a[4] * y[3];
        let out = num + den;
        self.x = [s, x[0], x[1], x[2]];
        self.y = [out, y[0], y[1], y[2]];
        out
    }
}

/// engine-sim's LowPassFilter (RC).
#[derive(Clone, Debug, Default)]
struct Rc {
    y: f32,
    alpha: f32,
}

impl Rc {
    fn new(fc: f32, sr: f32) -> Rc {
        let rc = 1.0 / (2.0 * PI * fc);
        let dt = 1.0 / sr;
        Rc { y: 0.0, alpha: dt / (rc + dt) }
    }

    #[inline]
    fn run(&mut self, s: f32) -> f32 {
        self.y = self.alpha * s + (1.0 - self.alpha) * self.y;
        self.y
    }
}

/// engine-sim's JitterFilter: a sample from the recent past, at a random
/// offset that wanders (uniform noise through a low-pass).
#[derive(Clone, Debug)]
struct Jitter {
    history: [f32; 10],
    offset: usize,
    noise: Rc,
}

impl Jitter {
    fn run(&mut self, s: f32, scale: f32, rng: &mut Noise) -> f32 {
        let n = self.history.len();
        self.history[self.offset] = s;
        self.offset = (self.offset + 1) % n;
        let t = self.noise.run(rng.unit() * (n - 1) as f32 * scale);
        let i0 = t.floor().clamp(0.0, (n - 1) as f32);
        let i1 = t.ceil().clamp(0.0, (n - 1) as f32);
        let frac = t - i0;
        let v0 = self.history[(i0 as usize + self.offset) % n];
        let v1 = self.history[(i1 as usize + self.offset) % n];
        v1 * frac + v0 * (1.0 - frac)
    }
}

/// Uniformly partitioned FFT convolution (overlap-save), block `B`:
/// the same result as engine-sim's direct convolution, at a fraction of
/// the cost, with B samples of latency.
pub struct Convolver {
    b: usize,
    fft: Fft,
    /// Spectra of the IR partitions.
    parts: Vec<Vec<(f32, f32)>>,
    /// Spectra of past input blocks, newest first (ring).
    hist: Vec<Vec<(f32, f32)>>,
    head: usize,
    /// Last 2B input samples.
    window: Vec<f32>,
    inbuf: Vec<f32>,
    outbuf: std::collections::VecDeque<f32>,
}

impl Convolver {
    pub fn new(ir: &[f32], b: usize) -> Convolver {
        let n = 2 * b;
        let fft = Fft::new(n);
        let parts: Vec<Vec<(f32, f32)>> = ir
            .chunks(b)
            .map(|c| {
                let mut re = vec![0.0; n];
                re[..c.len()].copy_from_slice(c);
                let mut im = vec![0.0; n];
                fft.forward(&mut re, &mut im);
                re.into_iter().zip(im).collect()
            })
            .collect();
        let k = parts.len().max(1);
        Convolver {
            b,
            fft,
            parts,
            hist: vec![vec![(0.0, 0.0); n]; k],
            head: 0,
            window: vec![0.0; n],
            inbuf: Vec::with_capacity(b),
            outbuf: std::collections::VecDeque::from(vec![0.0; b]),
        }
    }

    /// One sample in, one out (delayed by B).
    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        self.inbuf.push(x);
        if self.inbuf.len() == self.b {
            self.block();
        }
        self.outbuf.pop_front().unwrap_or(0.0)
    }

    fn block(&mut self) {
        let (b, n) = (self.b, 2 * self.b);
        self.window.copy_within(b.., 0);
        self.window[b..].copy_from_slice(&self.inbuf);
        self.inbuf.clear();
        let mut re = self.window.clone();
        let mut im = vec![0.0; n];
        self.fft.forward(&mut re, &mut im);
        let k = self.parts.len();
        self.head = (self.head + k - 1) % k;
        for (d, (r, i)) in self.hist[self.head].iter_mut().zip(re.iter().zip(&im)) {
            *d = (*r, *i);
        }
        let mut acc_re = vec![0.0f32; n];
        let mut acc_im = vec![0.0f32; n];
        for (p, part) in self.parts.iter().enumerate() {
            let h = &self.hist[(self.head + p) % k];
            for j in 0..n {
                let (xr, xi) = h[j];
                let (hr, hi) = part[j];
                acc_re[j] += xr * hr - xi * hi;
                acc_im[j] += xr * hi + xi * hr;
            }
        }
        self.fft.inverse(&mut acc_re, &mut acc_im);
        self.outbuf.extend(acc_re[b..].iter().copied());
    }
}

/// Complex radix-2 FFT with precomputed twiddles.
struct Fft {
    n: usize,
    rev: Vec<usize>,
    tw: Vec<(f32, f32)>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let tw = (0..n / 2).map(|k| (-2.0 * PI * k as f32 / n as f32).sin_cos()).map(|(s, c)| (c, s)).collect();
        Fft { n, rev, tw }
    }

    fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        self.run(re, im, false);
    }

    fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        self.run(re, im, true);
        let s = 1.0 / self.n as f32;
        re.iter_mut().for_each(|v| *v *= s);
        im.iter_mut().for_each(|v| *v *= s);
    }

    fn run(&self, re: &mut [f32], im: &mut [f32], inv: bool) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (c, s) = self.tw[k * step];
                    let s = if inv { -s } else { s };
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

/// engine-sim's LevelingFilter: a fast peak follower that pulls the level
/// towards `target`, never amplifying beyond `max_gain`.
#[derive(Clone, Debug)]
struct Leveler {
    peak: f32,
    gain: f32,
    decay: f32,
}

/// Synthesizer settings (engine-sim's AudioParameters), per engine.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SynthParams {
    /// Impulse response file in assets/sound/ir.
    pub impulse_response: String,
    /// Its volume (engine-sim's ir_lib volumes: 0.001 smooth, 0.01 new).
    pub ir_volume: f32,
    /// Wet share of the convolution, 0..1.
    pub convolution: f32,
    /// Derivative ("high frequency gain") share.
    pub hf_gain: f32,
    /// Air noise: how much the pressure modulates noise (0: none).
    pub noise: f32,
    pub noise_cutoff: f32,
    /// Input jitter, 0..1.
    pub jitter: f32,
    /// Leveler target (0..1 full scale) and its largest gain.
    pub level: f32,
    pub max_gain: f32,
    /// Pressure (bar) to synth units: sets where the leveler starts acting.
    pub input_scale: f32,
}

impl Default for SynthParams {
    fn default() -> Self {
        SynthParams {
            impulse_response: "mild_exhaust.wav".into(),
            ir_volume: 0.01,
            convolution: 1.0,
            hf_gain: 0.01,
            noise: 1.0,
            noise_cutoff: 2000.0,
            jitter: 0.5,
            level: 0.9,
            max_gain: 1.9,
            input_scale: 20.0,
        }
    }
}

pub struct Synth {
    p: SynthParams,
    sr: f32,
    antialias_in: Butter4,
    jitter: Jitter,
    dc: Rc,
    prev: f32,
    air: Butter4,
    conv: Option<Convolver>,
    antialias_out: Butter4,
    lev: Leveler,
    rng: Noise,
}

/// The impulse responses ship in engine/assets/sound/ir.
pub fn ir_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sound/ir")
}

/// Read a 16-bit PCM mono WAV and resample it (linear) to `sr`.
pub fn load_ir(path: &std::path::Path, sr: f32) -> Result<Vec<f32>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut i = 12;
    let (mut rate, mut ch, mut bits, mut data) = (44_100u32, 1u16, 16u16, None);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &b[i + 8..(i + 8 + len).min(b.len())];
        if id == b"fmt " {
            ch = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = Some(body);
        }
        i += 8 + len + (len & 1);
    }
    let data = data.ok_or_else(|| format!("{}: no data", path.display()))?;
    if bits != 16 {
        return Err(format!("{}: {bits}-bit, expected 16", path.display()));
    }
    let src: Vec<f32> = data.chunks(2 * ch as usize).map(|f| i16::from_le_bytes([f[0], f[1]]) as f32 / i16::MAX as f32).collect();
    // engine-sim clips the response where it falls below 100/32767.
    let end = src.iter().rposition(|v| v.abs() > 100.0 / i16::MAX as f32).map(|k| k + 1).unwrap_or(0).min(10_000);
    let src = &src[..end];
    let ratio = rate as f32 / sr;
    let n = (src.len() as f32 / ratio) as usize;
    Ok((0..n)
        .map(|k| {
            let t = k as f32 * ratio;
            let j = t as usize;
            let f = t - j as f32;
            let a = src[j.min(src.len() - 1)];
            let b = src[(j + 1).min(src.len() - 1)];
            // Keep the response's energy per output sample, as at 44.1 kHz.
            (a + (b - a) * f) * ratio
        })
        .collect())
}

impl Synth {
    pub fn new(p: SynthParams, sr: f32, seed: u32) -> Synth {
        let conv = load_ir(&ir_dir().join(&p.impulse_response), sr)
            .map(|ir| Convolver::new(&ir.iter().map(|v| v * p.ir_volume).collect::<Vec<_>>(), 128))
            .map_err(|e| eprintln!("audio: {e}; no convolution"))
            .ok();
        Synth {
            sr,
            antialias_in: Butter4::new(1900.0, sr),
            jitter: Jitter { history: [0.0; 10], offset: 0, noise: Rc::new(10_000.0, sr) },
            dc: Rc::new(10.0, sr),
            prev: 0.0,
            air: Butter4::new(p.noise_cutoff, sr),
            conv,
            antialias_out: Butter4::new(sr * 0.45, sr),
            lev: Leveler { peak: p.level, gain: 1.0, decay: 0.999f32.powf(44_100.0 / sr) },
            rng: Noise::new(seed ^ 0xE5),
            p,
        }
    }

    /// One sample of summed, delayed runner pressure (bar) in, audio out.
    #[inline]
    pub fn run(&mut self, pressure: f32) -> f32 {
        let p = &self.p;
        let x = self.antialias_in.run(pressure * p.input_scale);
        let j = self.jitter.run(x, p.jitter, &mut self.rng);
        let f = j - self.dc.run(j);
        let d = (j - self.prev) * self.sr / 44_100.0; // per 44.1 kHz sample, as tuned there
        self.prev = j;
        let r = self.air.run(self.rng.white());
        let mixed = p.noise * r + (1.0 - p.noise);
        let v_in = d * p.hf_gain * 441.0 + f * mixed * (1.0 - p.hf_gain);
        let v = match self.conv.as_mut() {
            Some(c) => p.convolution * c.run(v_in) + (1.0 - p.convolution) * v_in,
            None => v_in,
        };
        let s = self.antialias_out.run(v);
        // Leveler.
        let l = &mut self.lev;
        l.peak = (l.peak * l.decay).max(s.abs());
        let want = if l.peak > 0.0 { (p.level / l.peak).clamp(1e-5, p.max_gain) } else { 1.0 };
        l.gain = 0.9 * l.gain + 0.1 * want;
        s * l.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitioned_convolution_matches_direct() {
        let mut rng = Noise::new(5);
        let ir: Vec<f32> = (0..1000).map(|i| rng.white() * (-(i as f32) / 300.0).exp()).collect();
        let x: Vec<f32> = (0..3000).map(|_| rng.white()).collect();
        let b = 128;
        let mut c = Convolver::new(&ir, b);
        let y: Vec<f32> = x.iter().map(|v| c.run(*v)).collect();
        for n in (b..3000).step_by(37) {
            let direct: f32 = (0..ir.len()).filter(|k| *k <= n - b).map(|k| ir[k] * x[n - b - k]).sum();
            assert!((y[n] - direct).abs() < 1e-3, "sample {n}: {} vs {direct}", y[n]);
        }
    }

    #[test]
    fn impulse_responses_load() {
        for f in ["mild_exhaust.wav", "minimal_muffling_01.wav", "minimal_muffling_03.wav", "smooth_39.wav"] {
            let ir = load_ir(&ir_dir().join(f), 48_000.0).unwrap();
            assert!(ir.len() > 1000 && ir.iter().any(|v| v.abs() > 0.01), "{f}");
        }
    }
}
