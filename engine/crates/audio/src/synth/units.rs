//! The building blocks. Each unit reads its inputs (constants, parameters
//! or earlier units' outputs) and writes one block of mono samples. State
//! lives in the unit; nothing allocates while rendering.
use super::BLOCK;
use crate::dsp::Noise;
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// One point of an envelope: reach `level` after `time` seconds from the
/// previous point, along `curve` (0 linear, > 0 fast start, < 0 slow start).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point { pub time: f32, pub level: f32, #[serde(default)] pub curve: f32 }

/// One resonant mode: frequency as a ratio of the base, decay time (s, to
/// -60 dB) and gain.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mode { pub ratio: f32, pub decay: f32, pub gain: f32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode { Low, High, Band, Notch, Peak }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseColor { White, Pink, Brown }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape { Sine, Triangle, Saw, Square, Random }

/// What a node does. Settings that never change while it plays are here;
/// signals arrive through its inputs (see [`Unit::inputs`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Unit {
    /// Band-limited oscillator. Inputs: freq (Hz), pm (phase offset, cycles), width (square duty).
    Osc { shape: Shape },
    /// Inputs: none (gain 1).
    Noise { color: NoiseColor },
    /// Random impulses (crackle, rain, gravel). Inputs: density (per second), gain.
    Dust,
    /// A recorded sound (48 kHz mono). Inputs: rate (1 = original pitch).
    Clip { name: String, #[serde(skip)] samples: std::sync::Arc<[f32]>, looping: bool },
    /// Multi-segment envelope from 0, starting when the voice does. Inputs: none.
    Env { points: Vec<Point> },
    /// Attack-decay-sustain-release on the voice's gate. Inputs: attack, decay, sustain, release.
    Adsr,
    /// Slow modulator. Inputs: rate (Hz), depth.
    Lfo { shape: Shape },
    /// Smooth random wander in [-1, 1]. Inputs: rate (Hz).
    Wander,
    /// Inputs: a, b.
    Mul,
    /// Inputs: a, b.
    Add,
    /// Inputs: x, lo, hi.
    Clamp,
    /// Map x from [in_lo, in_hi] to [out_lo, out_hi], clamped; `exp` maps
    /// exponentially (frequencies). Inputs: x, in_lo, in_hi, out_lo, out_hi.
    Map { exp: bool },
    /// State-variable filter. Inputs: x, cutoff (Hz), q.
    Filter { mode: FilterMode },
    /// One-pole low-pass. Inputs: x, cutoff.
    LowPass1,
    /// One-pole high-pass. Inputs: x, cutoff.
    HighPass1,
    /// Feedback comb (metal, tubes). Inputs: x, freq (Hz), feedback.
    Comb,
    /// Delay with feedback. Inputs: x, time (s), feedback, mix.
    Delay { max: f32 },
    /// tanh saturation. Inputs: x, drive.
    Drive,
    /// Wave folding. Inputs: x, amount.
    Fold,
    /// Bit and rate reduction. Inputs: x, bits, hold (samples).
    Crush,
    /// A bank of damped resonators excited by x (struck objects). Inputs: x, freq (base Hz), damping (decay scale).
    Modal { modes: Vec<Mode> },
    /// Karplus-Strong string excited by x. Inputs: x, freq, decay (s), brightness 0..1.
    Pluck,
    /// Holds x whenever trig crosses up through 0. Inputs: x, trig.
    Hold,
    /// Per-sample formula (see [`super::expr`]): `t`, `sr` and up to six
    /// inputs named by `vars`, e.g. `f1 + (f0 - f1) * exp(-t * k)`.
    Expr { src: String, #[serde(default)] vars: Vec<String> },
    /// One random value per voice (from its seed), between lo and hi. Inputs: lo, hi.
    Rand,
    /// A car engine (the engine-sim model): `car` names its table in the
    /// sounds registered with [`register_engines`] unless `spec` is set.
    /// Inputs: rpm, throttle, load (-1..1), cut (> 0.5), gear, speed (m/s).
    Engine { car: String, #[serde(default)] fluid: bool, #[serde(skip)] spec: EngineSpec },
    /// Tyres on the stage: roll, slide, gravel, grass, splash. Inputs:
    /// speed, slip (m/s), contact, loose, soft, wet (0..1).
    Tyres,
}

/// An engine table carried by a patch built in code (compared by identity).
#[derive(Clone, Default)]
pub struct EngineSpec(pub Option<std::sync::Arc<crate::EngineSound>>);
impl PartialEq for EngineSpec { fn eq(&self, o: &Self) -> bool { match (&self.0, &o.0) { (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b), (None, None) => true, _ => false } } }
impl std::fmt::Debug for EngineSpec { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}", self.0.as_ref().map_or("registry", |s| s.name.as_str())) } }

static ENGINES: std::sync::RwLock<Vec<(String, std::sync::Arc<crate::EngineSound>)>> = std::sync::RwLock::new(Vec::new());

/// Make every car in `sounds` available to `engine` units by id.
pub fn register_engines(sounds: &crate::Sounds) {
    let mut e = ENGINES.write().unwrap();
    for id in sounds.ids() {
        if let Ok(spec) = sounds.get(&id) {
            e.retain(|(k, _)| *k != id);
            e.push((id, std::sync::Arc::new(spec)));
        }
    }
}

fn engine_spec(car: &str) -> Option<std::sync::Arc<crate::EngineSound>> { ENGINES.read().unwrap().iter().find(|(k, _)| k == car).map(|e| e.1.clone()) }

/// What the docs and the Luau front end call each unit and its inputs.
pub const UNITS: &[(&str, &[&str], &str)] = &[
    ("osc", &["freq", "pm", "width"], "Band-limited oscillator: sine, triangle, saw, square, random (stepped)."),
    ("noise", &[], "Noise: white, pink or brown."),
    ("dust", &["density", "gain"], "Random clicks, `density` per second."),
    ("clip", &["rate"], "A recorded sound at `rate`."),
    ("env", &[], "Envelope through {time, level, curve} points from 0."),
    ("adsr", &["attack", "decay", "sustain", "release"], "Envelope on the voice's gate (release when stopped)."),
    ("lfo", &["rate", "depth"], "Low-frequency modulator."),
    ("wander", &["rate"], "Smooth random wander in -1..1."),
    ("mul", &["a", "b"], "a * b."),
    ("add", &["a", "b"], "a + b."),
    ("clamp", &["x", "lo", "hi"], "Clamp."),
    ("map", &["x", "in_lo", "in_hi", "out_lo", "out_hi"], "Map a range onto another (exp for frequencies)."),
    ("filter", &["x", "cutoff", "q"], "State-variable filter: low, high, band, notch, peak."),
    ("lowpass1", &["x", "cutoff"], "One-pole low-pass."),
    ("highpass1", &["x", "cutoff"], "One-pole high-pass."),
    ("comb", &["x", "freq", "feedback"], "Feedback comb filter."),
    ("delay", &["x", "time", "feedback", "mix"], "Delay line with feedback."),
    ("drive", &["x", "drive"], "tanh saturation."),
    ("fold", &["x", "amount"], "Wave folder."),
    ("crush", &["x", "bits", "hold"], "Bit crusher and sample-rate reducer."),
    ("modal", &["x", "freq", "damping"], "Resonator bank (struck metal, wood, stone)."),
    ("pluck", &["x", "freq", "decay", "brightness"], "Karplus-Strong string."),
    ("hold", &["x", "trig"], "Sample and hold on a rising trigger."),
    ("rand", &["lo", "hi"], "A random value per voice (per-shot variation)."),
    ("engine", &["rpm", "throttle", "load", "cut", "gear", "speed"], "A car engine (engine-sim model) by car id."),
    ("tyres", &["speed", "slip", "contact", "loose", "soft", "wet"], "Tyres: rolling, sliding, gravel, grass and splash."),
    ("expr", &["a", "b", "c", "d", "e", "f"], "Per-sample formula of t, sr and its named inputs."),
];

impl Unit {
    pub fn name(&self) -> &'static str {
        match self {
            Unit::Osc { .. } => "osc", Unit::Noise { .. } => "noise", Unit::Dust => "dust", Unit::Clip { .. } => "clip",
            Unit::Env { .. } => "env", Unit::Adsr => "adsr", Unit::Lfo { .. } => "lfo", Unit::Wander => "wander",
            Unit::Mul => "mul", Unit::Add => "add", Unit::Clamp => "clamp", Unit::Map { .. } => "map",
            Unit::Filter { .. } => "filter", Unit::LowPass1 => "lowpass1", Unit::HighPass1 => "highpass1", Unit::Comb => "comb",
            Unit::Delay { .. } => "delay", Unit::Drive => "drive", Unit::Fold => "fold", Unit::Crush => "crush",
            Unit::Modal { .. } => "modal", Unit::Pluck => "pluck", Unit::Hold => "hold", Unit::Expr { .. } => "expr", Unit::Rand => "rand", Unit::Engine { .. } => "engine", Unit::Tyres => "tyres",
        }
    }
    /// Input names, in order.
    pub fn inputs(&self) -> &'static [&'static str] { UNITS.iter().find(|u| u.0 == self.name()).map_or(&[], |u| u.1) }
    /// Defaults for inputs left unconnected.
    pub fn default_input(&self, k: usize) -> f32 {
        match (self, k) {
            (Unit::Osc { .. }, 0) => 440.0, (Unit::Osc { .. }, 2) => 0.5,
            (Unit::Dust, 0) => 10.0, (Unit::Dust, 1) => 1.0, (Unit::Clip { .. }, 0) => 1.0,
            (Unit::Adsr, 0) => 0.01, (Unit::Adsr, 1) => 0.1, (Unit::Adsr, 2) => 0.7, (Unit::Adsr, 3) => 0.2,
            (Unit::Lfo { .. }, 0) => 1.0, (Unit::Lfo { .. }, 1) => 1.0, (Unit::Wander, 0) => 1.0,
            (Unit::Mul, _) => 1.0, (Unit::Clamp, 1) => -1.0, (Unit::Clamp, 2) => 1.0,
            (Unit::Map { .. }, 2) | (Unit::Map { .. }, 4) => 1.0,
            (Unit::Filter { .. }, 1) | (Unit::LowPass1, 1) | (Unit::HighPass1, 1) => 1000.0, (Unit::Filter { .. }, 2) => 0.707,
            (Unit::Comb, 1) => 200.0, (Unit::Comb, 2) => 0.7,
            (Unit::Delay { .. }, 1) => 0.25, (Unit::Delay { .. }, 2) => 0.3, (Unit::Delay { .. }, 3) => 0.5,
            (Unit::Drive, 1) | (Unit::Fold, 1) => 1.0, (Unit::Crush, 1) => 8.0, (Unit::Crush, 2) => 1.0,
            (Unit::Modal { .. }, 1) => 440.0, (Unit::Modal { .. }, 2) => 1.0,
            (Unit::Pluck, 1) => 220.0, (Unit::Pluck, 2) => 1.0, (Unit::Pluck, 3) => 0.5, (Unit::Rand, 1) => 1.0, (Unit::Engine { .. }, 0) => 900.0, (Unit::Tyres, 2) => 1.0,
            _ => 0.0,
        }
    }
}

/// An input for one block: a constant, a ramp (a parameter gliding) or a buffer.
#[derive(Clone, Copy)]
pub enum Sig<'a> { K(f32), Ramp(f32, f32), B(&'a [f32]) }

impl Sig<'_> {
    #[inline]
    pub fn at(&self, i: usize) -> f32 {
        match *self {
            Sig::K(v) => v,
            Sig::Ramp(a, b) => a + (b - a) * (i as f32 / BLOCK as f32),
            Sig::B(b) => b[i],
        }
    }
    /// The first value (settings read once per block).
    pub fn first(&self) -> f32 { self.at(0) }
}

/// A unit's running state.
pub(crate) enum State {
    Osc { shape: Shape, phase: f32, last: f32, held: f32, rng: Noise },
    Noise { color: NoiseColor, rng: Noise, b: [f32; 7] },
    Dust { rng: Noise },
    Clip { samples: std::sync::Arc<[f32]>, looping: bool, pos: f64 },
    Env { t: f32, k: usize, from: f32 },
    Adsr { level: f32, stage: u8 },
    Lfo { shape: Shape, phase: f32, held: f32, rng: Noise },
    Wander { rng: Noise, a: f32, b: f32, t: f32 },
    Math,
    Map { exp: bool },
    Svf { mode: FilterMode, ic1: f32, ic2: f32 },
    OnePole { y: f32, x1: f32, high: bool },
    Comb { buf: Vec<f32>, pos: usize },
    Delay { buf: Vec<f32>, pos: usize },
    Crush { held: f32, n: f32 },
    Modal { s: Vec<[f32; 2]> },
    Pluck { buf: Vec<f32>, pos: usize, lp: f32 },
    Hold { held: f32, last: f32 },
    Expr { e: std::sync::Arc<super::expr::Expr>, t: f32, slots: Vec<super::expr::Slot> },
    Rand { u: f32 },
    Engine(Option<Box<crate::engine::EngineVoice>>),
    Tyres(Box<crate::tyres::TyreVoice>),
}

/// Longest comb / string period: 20 Hz.
const MAX_PERIOD: f32 = 1.0 / 20.0;

impl State {
    pub(crate) fn new(u: &Unit, sr: f32, seed: u32, expr: Option<&std::sync::Arc<super::expr::Expr>>) -> State {
        match u {
            Unit::Osc { shape } => State::Osc { shape: *shape, phase: 0.0, last: 0.0, held: 0.0, rng: Noise::new(seed) },
            Unit::Noise { color } => State::Noise { color: *color, rng: Noise::new(seed), b: [0.0; 7] },
            Unit::Dust => State::Dust { rng: Noise::new(seed) },
            Unit::Clip { samples, looping, .. } => State::Clip { samples: samples.clone(), looping: *looping, pos: 0.0 },
            Unit::Env { .. } => State::Env { t: 0.0, k: 0, from: 0.0 },
            Unit::Adsr => State::Adsr { level: 0.0, stage: 0 },
            Unit::Lfo { shape } => State::Lfo { shape: *shape, phase: 0.0, held: 0.0, rng: Noise::new(seed) },
            Unit::Wander => { let mut rng = Noise::new(seed); let a = rng.white(); let b = rng.white(); State::Wander { rng, a, b, t: 0.0 } }
            Unit::Mul | Unit::Add | Unit::Clamp | Unit::Drive | Unit::Fold => State::Math,
            Unit::Map { exp } => State::Map { exp: *exp },
            Unit::Filter { mode } => State::Svf { mode: *mode, ic1: 0.0, ic2: 0.0 },
            Unit::LowPass1 => State::OnePole { y: 0.0, x1: 0.0, high: false },
            Unit::HighPass1 => State::OnePole { y: 0.0, x1: 0.0, high: true },
            Unit::Comb => State::Comb { buf: vec![0.0; (MAX_PERIOD * sr) as usize + 2], pos: 0 },
            Unit::Delay { max } => State::Delay { buf: vec![0.0; (max.clamp(0.001, 10.0) * sr) as usize + 2], pos: 0 },
            Unit::Crush => State::Crush { held: 0.0, n: 0.0 },
            Unit::Modal { modes } => State::Modal { s: vec![[0.0; 2]; modes.len()] },
            Unit::Pluck => State::Pluck { buf: vec![0.0; (MAX_PERIOD * sr) as usize + 2], pos: 0, lp: 0.0 },
            Unit::Hold => State::Hold { held: 0.0, last: 0.0 },
            Unit::Expr { .. } => {
                let e = expr.expect("expression prepared with the graph").clone();
                State::Expr { slots: e.slots(), e, t: 0.0 }
            }
            Unit::Rand => { let mut r = Noise::new(seed); r.unit(); State::Rand { u: r.unit() } }
            Unit::Engine { car, fluid, spec } => {
                let spec = spec.0.clone().or_else(|| engine_spec(car));
                let model = if *fluid { crate::Model::Fluid } else { crate::Model::Pulse };
                State::Engine(spec.map(|s| Box::new(crate::engine::EngineVoice::with_model((*s).clone(), sr, seed, model))))
            }
            Unit::Tyres => State::Tyres(Box::new(crate::tyres::TyreVoice::new(sr, seed))),
        }
    }

    /// Whether this unit has run its course (envelopes; voices end when
    /// every envelope is done and the output is quiet).
    pub(crate) fn finished(&self, unit: &Unit) -> Option<bool> {
        match self {
            State::Env { k, .. } => if let Unit::Env { points } = unit { Some(*k >= points.len()) } else { unreachable!() },
            State::Adsr { stage, level } => Some(*stage == 3 && *level <= 1e-5),
            State::Clip { samples, looping, pos } => Some(!looping && *pos as usize + 1 >= samples.len()),
            _ => None,
        }
    }

    /// One block. `ins` has an entry per input (missing ones are defaults).
    pub(crate) fn run(&mut self, ins: &[Sig], out: &mut [f32], sr: f32, gate: bool, unit: &Unit) {
        let n = out.len();
        let dt = 1.0 / sr;
        let i = |k: usize| ins.get(k).copied().unwrap_or_else(|| Sig::K(unit.default_input(k)));
        match self {
            State::Osc { shape, phase, last, held, rng } => {
                let (f, pm, w) = (i(0), i(1), i(2));
                for (s, o) in out.iter_mut().enumerate() {
                    let inc = (f.at(s) * dt).clamp(0.0, 0.5);
                    let p = (*phase + pm.at(s)).rem_euclid(1.0);
                    *o = match shape {
                        Shape::Sine => super::expr::sin_turns(p),
                        Shape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
                        Shape::Saw => 2.0 * p - 1.0 - poly_blep(p, inc),
                        Shape::Square => {
                            let duty = w.at(s).clamp(0.01, 0.99);
                            let mut v = if p < duty { 1.0 } else { -1.0 };
                            v += poly_blep(p, inc);
                            v -= poly_blep((p - duty).rem_euclid(1.0), inc);
                            v
                        }
                        Shape::Random => { if p < *last { *held = rng.white(); } *held }
                    };
                    *last = p;
                    *phase = (*phase + inc).fract();
                }
            }
            State::Noise { color, rng, b } => {
                for o in out.iter_mut() {
                    let w = rng.white();
                    *o = match color {
                        NoiseColor::White => w,
                        NoiseColor::Pink => {
                            // Paul Kellet's economy pink filter.
                            b[0] = 0.99886 * b[0] + w * 0.0555179;
                            b[1] = 0.99332 * b[1] + w * 0.0750759;
                            b[2] = 0.96900 * b[2] + w * 0.1538520;
                            b[3] = 0.86650 * b[3] + w * 0.3104856;
                            b[4] = 0.55000 * b[4] + w * 0.5329522;
                            b[5] = -0.7616 * b[5] - w * 0.0168980;
                            let p = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                            b[6] = w * 0.115926;
                            p * 0.11
                        }
                        NoiseColor::Brown => { b[0] = (b[0] + w * 0.02).clamp(-1.0, 1.0) * 0.998; b[0] * 3.5 }
                    };
                }
            }
            State::Dust { rng } => {
                let (d, g) = (i(0), i(1));
                for (s, o) in out.iter_mut().enumerate() {
                    *o = if rng.unit() < d.at(s) * dt { g.at(s) * rng.white().signum() } else { 0.0 };
                }
            }
            State::Clip { samples, looping, pos } => {
                let r = i(0);
                let len = samples.len();
                for (s, o) in out.iter_mut().enumerate() {
                    if len < 2 { *o = 0.0; continue; }
                    let k = *pos as usize;
                    if k + 1 >= len {
                        if *looping { *pos -= (len - 1) as f64; } else { *o = 0.0; continue; }
                    }
                    let k = *pos as usize;
                    let t = (*pos - k as f64) as f32;
                    *o = samples[k] + (samples[(k + 1).min(len - 1)] - samples[k]) * t;
                    *pos += r.at(s).max(0.0) as f64 * (crate::SAMPLE_RATE / sr) as f64;
                }
            }
            State::Env { t, k, from } => {
                let Unit::Env { points } = unit else { unreachable!() };
                for o in out.iter_mut() {
                    *o = loop {
                        let Some(p) = points.get(*k) else { break points.last().map_or(0.0, |p| p.level) };
                        if *t >= p.time { *t -= p.time.max(0.0); *from = p.level; *k += 1; continue; }
                        let u = *t / p.time.max(1e-6);
                        break *from + (p.level - *from) * curve(u, p.curve);
                    };
                    *t += dt;
                }
            }
            State::Adsr { level, stage } => {
                let (a, d, su, r) = (i(0).first().max(1e-4), i(1).first().max(1e-4), i(2).first().clamp(0.0, 1.0), i(3).first().max(1e-4));
                if !gate && *stage < 3 { *stage = 3; }
                for o in out.iter_mut() {
                    match *stage {
                        0 => { *level += dt / a; if *level >= 1.0 { *level = 1.0; *stage = 1; } }
                        1 => { *level -= dt / d * (1.0 - su); if *level <= su { *level = su; *stage = 2; } }
                        2 => {}
                        _ => { *level = (*level - dt / r).max(0.0); }
                    }
                    *o = *level;
                }
            }
            State::Lfo { shape, phase, held, rng } => {
                let (f, dep) = (i(0), i(1));
                for (s, o) in out.iter_mut().enumerate() {
                    let p = *phase;
                    let v = match shape {
                        Shape::Sine => super::expr::sin_turns(p),
                        Shape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
                        Shape::Saw => 2.0 * p - 1.0,
                        Shape::Square => if p < 0.5 { 1.0 } else { -1.0 },
                        Shape::Random => *held,
                    };
                    *phase += f.at(s) * dt;
                    if *phase >= 1.0 { *phase = phase.fract(); *held = rng.white(); }
                    *o = v * dep.at(s);
                }
            }
            State::Wander { rng, a, b, t } => {
                let f = i(0);
                for (s, o) in out.iter_mut().enumerate() {
                    *t += f.at(s).max(0.0) * dt;
                    if *t >= 1.0 { *t = t.fract(); *a = *b; *b = rng.white(); }
                    let u = *t * *t * (3.0 - 2.0 * *t);
                    *o = *a + (*b - *a) * u;
                }
            }
            State::Math => {
                let (x, y, z) = (i(0), i(1), i(2));
                match unit {
                    Unit::Mul => for (s, o) in out.iter_mut().enumerate() { *o = x.at(s) * y.at(s) },
                    Unit::Add => for (s, o) in out.iter_mut().enumerate() { *o = x.at(s) + y.at(s) },
                    Unit::Clamp => for (s, o) in out.iter_mut().enumerate() { *o = x.at(s).clamp(y.at(s), z.at(s).max(y.at(s))) },
                    Unit::Drive => for (s, o) in out.iter_mut().enumerate() { *o = (x.at(s) * y.at(s).max(0.0)).tanh() },
                    Unit::Fold => for (s, o) in out.iter_mut().enumerate() { *o = fold(x.at(s) * y.at(s).max(0.0)) },
                    _ => out.fill(0.0),
                }
            }
            State::Map { exp } => {
                let (x, a, b, c, d) = (i(0), i(1), i(2), i(3), i(4));
                for (s, o) in out.iter_mut().enumerate() {
                    let u = ((x.at(s) - a.at(s)) / (b.at(s) - a.at(s)).abs().max(1e-9).copysign(b.at(s) - a.at(s))).clamp(0.0, 1.0);
                    let (lo, hi) = (c.at(s), d.at(s));
                    *o = if *exp && lo > 0.0 && hi > 0.0 { lo * (hi / lo).powf(u) } else { lo + (hi - lo) * u };
                }
            }
            State::Svf { mode, ic1, ic2 } => {
                // Cytomic (Andrew Simper) trapezoidal SVF: stable under fast cutoff changes.
                let (x, fc, q) = (i(0), i(1), i(2));
                let coef = |c: f32, q: f32| {
                    let g = (PI * (c.clamp(10.0, sr * 0.49) / sr)).tan();
                    let k = 1.0 / q.max(0.05);
                    let a1 = 1.0 / (1.0 + g * (g + k));
                    (g, k, a1, g * a1, g * g * a1)
                };
                let static_coef = !matches!(fc, Sig::B(_)) && !matches!(q, Sig::B(_));
                let mut c = coef(fc.first(), q.first());
                for (s, o) in out.iter_mut().enumerate() {
                    if !static_coef || matches!(fc, Sig::Ramp(..)) { c = coef(fc.at(s), q.at(s)); }
                    let (_g, k, a1, a2, a3) = c;
                    let v0 = x.at(s);
                    let v3 = v0 - *ic2;
                    let v1 = a1 * *ic1 + a2 * v3;
                    let v2 = *ic2 + a2 * *ic1 + a3 * v3;
                    *ic1 = 2.0 * v1 - *ic1;
                    *ic2 = 2.0 * v2 - *ic2;
                    *o = match mode {
                        FilterMode::Low => v2,
                        FilterMode::Band => v1,
                        FilterMode::High => v0 - k * v1 - v2,
                        FilterMode::Notch => v0 - k * v1,
                        FilterMode::Peak => v0 - k * v1 - 2.0 * v2,
                    };
                }
            }
            State::OnePole { y, x1, high } => {
                let (x, fc) = (i(0), i(1));
                let coef = |f: f32| 1.0 - (-TAU * f.clamp(1.0, sr * 0.49) / sr).exp();
                let k = if let Sig::K(f) = fc { Some(coef(f)) } else { None };
                for (s, o) in out.iter_mut().enumerate() {
                    let a = k.unwrap_or_else(|| coef(fc.at(s)));
                    let v = x.at(s);
                    *y += a * (v - *y);
                    *o = if *high { v - *y } else { *y };
                    *x1 = v;
                }
            }
            State::Comb { buf, pos } => {
                let (x, f, fb) = (i(0), i(1), i(2));
                let len = buf.len();
                for (s, o) in out.iter_mut().enumerate() {
                    let d = (sr / f.at(s).max(20.0)).min((len - 2) as f32);
                    let y = read_frac(buf, *pos, d);
                    let v = x.at(s) + y * fb.at(s).clamp(-0.999, 0.999);
                    buf[*pos] = v;
                    *pos = (*pos + 1) % len;
                    *o = v;
                }
            }
            State::Delay { buf, pos } => {
                let (x, t, fb, mix) = (i(0), i(1), i(2), i(3));
                let len = buf.len();
                for (s, o) in out.iter_mut().enumerate() {
                    let d = (t.at(s) * sr).clamp(1.0, (len - 2) as f32);
                    let y = read_frac(buf, *pos, d);
                    let v = x.at(s);
                    buf[*pos] = v + y * fb.at(s).clamp(-0.999, 0.999);
                    *pos = (*pos + 1) % len;
                    let m = mix.at(s);
                    *o = v * (1.0 - m) + y * m;
                }
            }
            State::Crush { held, n } => {
                let (x, bits, hold) = (i(0), i(1), i(2));
                for (s, o) in out.iter_mut().enumerate() {
                    *n += 1.0;
                    if *n >= hold.at(s).max(1.0) {
                        *n = 0.0;
                        let q = 2f32.powf(bits.at(s).clamp(1.0, 24.0) - 1.0);
                        *held = (x.at(s) * q).round() / q;
                    }
                    *o = *held;
                }
            }
            State::Modal { s: st } => {
                let Unit::Modal { modes } = unit else { unreachable!() };
                // Each mode: a two-pole resonator, r^2 per sample from the decay.
                let (x, f, damp) = (i(0), i(1), i(2));
                let base = f.first();
                let damping = damp.first().max(1e-3);
                out.fill(0.0);
                for (m, z) in modes.iter().zip(st.iter_mut()) {
                    let w = TAU * (base * m.ratio).clamp(1.0, sr * 0.45) / sr;
                    let r = (-6.9078 / (m.decay.max(1e-3) * damping * sr)).exp();
                    let (c1, c2) = (2.0 * r * w.cos(), -r * r);
                    let g = m.gain * w.sin();
                    for (k, o) in out.iter_mut().enumerate() {
                        let y = c1 * z[0] + c2 * z[1] + x.at(k) * g;
                        z[1] = z[0];
                        z[0] = y;
                        *o += y;
                    }
                }
            }
            State::Pluck { buf, pos, lp } => {
                let (x, f, decay, bright) = (i(0), i(1), i(2), i(3));
                let len = buf.len();
                for (s, o) in out.iter_mut().enumerate() {
                    let period = (sr / f.at(s).max(20.0)).min((len - 2) as f32);
                    let y = read_frac(buf, *pos, period);
                    let fb = (-6.9078 * period / (decay.at(s).max(1e-3) * sr)).exp();
                    let b = bright.at(s).clamp(0.0, 1.0);
                    *lp += (y - *lp) * (0.05 + 0.95 * b);
                    let v = x.at(s) + *lp * fb;
                    buf[*pos] = v;
                    *pos = (*pos + 1) % len;
                    *o = v;
                }
            }
            State::Hold { held, last } => {
                let (x, t) = (i(0), i(1));
                for (s, o) in out.iter_mut().enumerate() {
                    let tr = t.at(s);
                    if *last <= 0.0 && tr > 0.0 { *held = x.at(s); }
                    *last = tr;
                    *o = *held;
                }
            }
            State::Engine(v) => {
                out.fill(0.0);
                if let Some(v) = v {
                    v.set(crate::Controls { rpm: i(0).first(), throttle: i(1).first(), load: i(2).first(), cut: i(3).first() > 0.5, gear: i(4).first().round() as i32, speed: i(5).first() });
                    v.render_add(out, 1.0);
                }
            }
            State::Tyres(v) => {
                out.fill(0.0);
                v.set(crate::tyres::TyreControls { speed: i(0).first(), slip: i(1).first(), contact: i(2).first(), loose: i(3).first(), soft: i(4).first(), wet: i(5).first() });
                v.render_add(out, 1.0);
            }
            State::Rand { u } => {
                let (lo, hi) = (i(0), i(1));
                for (s, o) in out.iter_mut().enumerate() { *o = lo.at(s) + (hi.at(s) - lo.at(s)) * *u; }
            }
            State::Expr { e, t, slots } => {
                let v: [Sig; 6] = std::array::from_fn(|k| i(k));
                e.eval_block(*t, dt, sr, &v, slots, out);
                *t += dt * out.len() as f32;
            }
        }
        debug_assert!(n == out.len());
    }
}

/// Correction for the discontinuities of saw and square (aliasing).
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 { return 0.0; }
    if t < dt { let t = t / dt; t + t - t * t - 1.0 }
    else if t > 1.0 - dt { let t = (t - 1.0) / dt; t * t + t + t + 1.0 }
    else { 0.0 }
}

#[inline]
fn fold(x: f32) -> f32 {
    let mut y = x;
    for _ in 0..8 {
        if y > 1.0 { y = 2.0 - y; } else if y < -1.0 { y = -2.0 - y; } else { break; }
    }
    y.clamp(-1.0, 1.0)
}

/// Envelope shape: 0 linear; positive bends toward a fast start
/// (exponential decay feel), negative a slow start.
#[inline]
fn curve(u: f32, c: f32) -> f32 {
    if c.abs() < 1e-4 { return u; }
    (1.0 - (-c * u).exp()) / (1.0 - (-c).exp())
}

#[inline]
fn read_frac(buf: &[f32], pos: usize, d: f32) -> f32 {
    let len = buf.len();
    let di = d.floor() as usize;
    let t = d - di as f32;
    let a = buf[(pos + len - di % len) % len];
    let b = buf[(pos + len - (di + 1) % len) % len];
    a + (b - a) * t
}
