//! An engine voice in the spirit of Ange Yaghi's engine-sim: the sound is
//! not a recording but the exhaust pressure of a running engine. Each
//! cylinder blows down when its exhaust valve opens, at its own crank angle
//! (firing order, even or odd firing), with a pulse whose strength follows
//! the combustion (load, misfires, cuts) and varies a little cycle to cycle.
//! Pulses travel their own header (unequal lengths make the boxer rumble),
//! meet in the collector, pass the turbine, and ring in the main pipe, a
//! digital waveguide with reflections at both ends. Intake, valvetrain,
//! turbo, straight-cut gear whine and overrun pops are layered on top.
//!
//! engine-sim integrates a full thermodynamic model at tens of kHz; this
//! keeps its signal path but drives it from the game's own engine state
//! (rpm, throttle, torque), so four cars cost well under a millisecond per
//! 10 ms audio block.

use crate::dsp::{Biquad, DcBlock, Delay, Kind, Noise, OnePole, approach, smoothstep};
use crate::fluid::{FluidEngine, Geometry};
use serde::Deserialize;

/// Speed of sound in hot exhaust gas (m/s).
const C_EXHAUST: f32 = 480.0;
/// Crank angles (degrees after firing TDC) where valves open and close.
const EVO: f32 = 130.0;
const EXHAUST_LEN: f32 = 250.0;
const IVO: f32 = 350.0;
const INTAKE_LEN: f32 = 230.0;

/// How an engine sounds: one table per car in data/sounds.toml.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineSound {
    pub name: String,
    pub cylinders: usize,
    /// Cylinder numbers (1-based) in firing order.
    pub firing_order: Vec<usize>,
    /// Crank degrees between firings, one per firing; even when absent.
    #[serde(default)]
    pub firing_gaps: Vec<f32>,
    /// Header length per cylinder (m), in cylinder-number order.
    pub headers: Vec<f32>,
    /// Main pipe length (m), collector to tailpipe.
    pub pipe: f32,
    /// Reflection at the open tailpipe (0..1) and at the engine end.
    pub end_reflection: f32,
    pub engine_reflection: f32,
    /// Muffler body resonances: [Hz, Q, gain].
    pub resonances: Vec<[f32; 3]>,
    /// Exhaust low-pass (Hz) closed and open throttle.
    pub tone: [f32; 2],
    /// Cycle-to-cycle combustion variation at idle and at the limiter.
    pub roughness: [f32; 2],
    pub turbo: bool,
    /// Whistle pitch (Hz) at no boost and at full boost.
    pub whistle: [f32; 2],
    /// Teeth on the gearbox's constant mesh: whine = engine rev/s * teeth.
    pub gear_teeth: f32,
    /// Chance per exhaust event of a pop on the overrun (anti-lag).
    pub pops: f32,
    /// Levels.
    pub exhaust: f32,
    pub intake: f32,
    pub mechanical: f32,
    pub whine: f32,
    pub turbo_level: f32,
    pub gain: f32,
    /// Geometry for the gas-dynamics exhaust (`fluid`); defaults if absent.
    #[serde(default)]
    pub fluid: Geometry,
    /// Level of the fluid exhaust relative to the pulse model's.
    #[serde(default = "one")]
    pub fluid_level: f32,
}

fn one() -> f32 {
    1.0
}

/// Which exhaust model a voice runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// Pulses into delay-line headers and a linear waveguide pipe: cheap.
    Pulse,
    /// Gas dynamics in the cylinders, headers and pipe (`fluid`): costly.
    Fluid,
}

/// What the game tells the voice, at any rate (it is smoothed per sample).
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    pub rpm: f32,
    pub throttle: f32,
    /// Engine torque as a fraction of the curve's peak, -1..1.
    pub load: f32,
    /// Ignition cut: the limiter or a gear shift.
    pub cut: bool,
    /// Engaged gear (0 neutral, -1 reverse).
    pub gear: i32,
    /// Road speed (m/s).
    pub speed: f32,
}

struct Cylinder {
    /// Crank angle of this cylinder's firing TDC (degrees, 0..720).
    phase: f32,
    last: f32,
    amp: f32,
    header: Delay,
    delay: f32,
}

pub struct EngineVoice {
    pub spec: EngineSound,
    sr: f32,
    dt: f32,
    crank: f32,
    rpm: f32,
    throttle: f32,
    load: f32,
    target: Controls,
    cyl: Vec<Cylinder>,
    fwd: Delay,
    back: Delay,
    pipe_n: f32,
    back_out: f32,
    end_lp: OnePole,
    turbine: OnePole,
    tone: OnePole,
    res: Vec<(Biquad, f32)>,
    prev_rad: f32,
    dc: DcBlock,
    noise: Noise,
    intake_bp: Biquad,
    tick_bp: Biquad,
    tick: f32,
    whine_phase: f32,
    whine2_phase: f32,
    boost: f32,
    whistle_phase: f32,
    hiss_bp: Biquad,
    bov: f32,
    bov_bp: Biquad,
    lift_timer: f32,
    prev_throttle: f32,
    crack: f32,
    crack_bp: Biquad,
    fluid: Option<FluidEngine>,
}

impl EngineVoice {
    pub fn new(spec: EngineSound, sr: f32, seed: u32) -> EngineVoice {
        EngineVoice::with_model(spec, sr, seed, Model::Pulse)
    }

    pub fn with_model(spec: EngineSound, sr: f32, seed: u32, model: Model) -> EngineVoice {
        let n = spec.cylinders.max(1);
        let gaps: Vec<f32> = if spec.firing_gaps.len() == n { spec.firing_gaps.clone() } else { vec![720.0 / n as f32; n] };
        let mut phase_of = vec![0.0f32; n];
        let mut angle = 0.0;
        for (k, &c) in spec.firing_order.iter().enumerate().take(n) {
            phase_of[(c.max(1) - 1).min(n - 1)] = angle;
            angle += gaps[k];
        }
        let cyl = (0..n)
            .map(|i| {
                let len = spec.headers.get(i).copied().unwrap_or(0.5);
                let delay = len / C_EXHAUST * sr;
                Cylinder { phase: phase_of[i], last: 0.0, amp: 0.0, header: Delay::new(delay as usize + 4), delay }
            })
            .collect();
        let pipe_n = spec.pipe / C_EXHAUST * sr;
        let fluid = (model == Model::Fluid).then(|| FluidEngine::new(spec.fluid.clone(), &phase_of, &spec.headers, spec.pipe, sr, seed));
        let res = spec.resonances.iter().map(|r| (Biquad::new(Kind::BandPass, r[0], r[1], sr), r[2])).collect();
        EngineVoice {
            sr,
            dt: 1.0 / sr,
            crank: 0.0,
            rpm: 0.0,
            throttle: 0.0,
            load: 0.0,
            target: Controls::default(),
            cyl,
            fwd: Delay::new(pipe_n as usize + 4),
            back: Delay::new(pipe_n as usize + 4),
            pipe_n,
            back_out: 0.0,
            end_lp: OnePole::new(1800.0, sr),
            turbine: OnePole::new(1400.0, sr),
            tone: OnePole::new(spec.tone[0], sr),
            res,
            prev_rad: 0.0,
            dc: DcBlock::default(),
            noise: Noise::new(seed),
            intake_bp: Biquad::new(Kind::BandPass, 700.0, 1.1, sr),
            tick_bp: Biquad::new(Kind::BandPass, 3300.0, 9.0, sr),
            tick: 0.0,
            whine_phase: 0.0,
            whine2_phase: 0.0,
            boost: 0.0,
            whistle_phase: 0.0,
            hiss_bp: Biquad::new(Kind::BandPass, 5000.0, 2.0, sr),
            bov: 0.0,
            bov_bp: Biquad::new(Kind::BandPass, 2600.0, 0.8, sr),
            lift_timer: 10.0,
            prev_throttle: 0.0,
            crack: 0.0,
            crack_bp: Biquad::new(Kind::BandPass, 1400.0, 0.9, sr),
            fluid,
            spec,
        }
    }

    pub fn set(&mut self, c: Controls) {
        self.target = c;
    }

    pub fn model(&self) -> Model {
        if self.fluid.is_some() { Model::Fluid } else { Model::Pulse }
    }

    /// Pipe cell updates so far (fluid model), for cost reports.
    pub fn work(&self) -> u64 {
        self.fluid.as_ref().map(|f| f.work).unwrap_or(0)
    }

    /// Exhaust firing frequency (Hz) at `rpm`: cylinders/2 pulses per revolution.
    pub fn firing_hz(&self, rpm: f32) -> f32 {
        rpm / 60.0 * self.cyl.len() as f32 / 2.0
    }

    /// Add this voice into `out` (mono), `gain` times.
    pub fn render_add(&mut self, out: &mut [f32], gain: f32) {
        let sr = self.sr;
        let dt = self.dt;
        let spec_tone = self.spec.tone;
        // Per block: filters that follow slow state.
        let open = self.throttle;
        self.tone.set_hz(spec_tone[0] + (spec_tone[1] - spec_tone[0]) * open, sr);
        self.intake_bp.set(Kind::BandPass, 450.0 + self.rpm * 0.12, 1.1, sr);
        let whistle_hz = self.spec.whistle[0] + (self.spec.whistle[1] - self.spec.whistle[0]) * self.boost;
        self.hiss_bp.set(Kind::BandPass, whistle_hz * 1.4, 1.5, sr);
        for o in out.iter_mut() {
            let t = self.target;
            self.rpm = approach(self.rpm, t.rpm.max(0.0), 0.012, dt);
            self.throttle = approach(self.throttle, t.throttle.clamp(0.0, 1.0), 0.02, dt);
            self.load = approach(self.load, t.load.clamp(-1.0, 1.0), 0.03, dt);
            // Throttle lift: the turbo's blow-off valve and anti-lag pops.
            if self.prev_throttle - self.throttle > 0.004 && self.throttle < 0.35 && self.lift_timer > 0.3 {
                self.lift_timer = 0.0;
                if self.spec.turbo && self.boost > 0.35 {
                    self.bov = self.boost;
                }
            }
            self.prev_throttle = self.throttle;
            self.lift_timer += dt;
            let rpm = self.rpm;
            let running = rpm > 200.0;
            self.crank = (self.crank + rpm / 60.0 * 360.0 * dt).rem_euclid(720.0);

            // Cylinders: blowdown pulses into their headers.
            let rough = self.spec.roughness[0] + (self.spec.roughness[1] - self.spec.roughness[0]) * smoothstep(900.0, 7500.0, rpm);
            let overrun = self.throttle < 0.12 && rpm > 3800.0 && (self.lift_timer < 1.6 || t.cut);
            let mut collector = 0.0;
            let mut intake_env = 0.0;
            let mut tick = 0.0;
            for c in self.cyl.iter_mut() {
                let a = (self.crank - c.phase).rem_euclid(720.0);
                if c.last < EVO && a >= EVO {
                    // A new exhaust event: how strong was this combustion?
                    let pumping = 0.1 + 0.25 * self.throttle;
                    let burn = if t.cut || !running { 0.0 } else { 0.2 + 0.8 * self.load.max(0.0) };
                    c.amp = (pumping + burn) * (1.0 + rough * self.noise.gauss()).max(0.2);
                    if overrun && self.noise.unit() < self.spec.pops {
                        c.amp *= 3.5;
                        self.crack = 1.0;
                    }
                    tick += 1.0;
                }
                if c.last < IVO && a >= IVO {
                    tick += 0.6;
                }
                c.last = a;
                let mut p = 0.0;
                if running && a >= EVO && a < EVO + EXHAUST_LEN {
                    let x = (a - EVO) / EXHAUST_LEN;
                    let env = (-x * 7.0).exp();
                    p = c.amp * ((1.0 - (-x * 40.0).exp()) * env + 0.12 * (x * std::f32::consts::PI).sin());
                    p += c.amp * 0.22 * env * self.noise.white();
                }
                if a >= IVO && a < IVO + INTAKE_LEN {
                    intake_env += ((a - IVO) / INTAKE_LEN * std::f32::consts::PI).sin();
                }
                c.header.write(p);
                collector += c.header.read(c.delay);
            }
            if self.spec.turbo {
                // The turbine smooths the pulses and eats the highs.
                let lp = self.turbine.run(collector);
                collector = lp * 0.75 + collector * 0.25;
            }

            let rad = if let Some(f) = self.fluid.as_mut() {
                // The gas-dynamics exhaust replaces the pulse path below.
                let fuel = !(t.cut || (self.throttle < 0.03 && rpm > 1600.0));
                f.sample(rpm, self.throttle, self.boost, fuel, rough) * self.spec.fluid_level
            } else {
            // Main pipe: forward and backward travelling waves.
            let fwd_in = collector + self.spec.engine_reflection * self.back_out;
            self.fwd.write(fwd_in);
            let at_end = self.fwd.read(self.pipe_n);
            let refl = -self.spec.end_reflection * self.end_lp.run(at_end);
            self.back.write(refl);
            self.back_out = self.back.read(self.pipe_n);
            let rad = at_end + refl;
            // Radiation from the tailpipe favours higher frequencies.
            let ex = rad + 2.5 * (rad - self.prev_rad);
            self.prev_rad = rad;
            ex
            };
            let mut ex = rad;
            ex = self.tone.run(ex);
            let mut body = 0.0;
            for (bq, g) in self.res.iter_mut() {
                body += bq.run(ex) * *g;
            }
            ex += body;
            // Overrun crackle: a sharp broadband snap from the tailpipe.
            if self.crack > 1e-3 {
                ex += self.crack_bp.run(self.noise.white()) * self.crack * 2.5;
                self.crack *= (-dt / 0.012).exp();
            }
            let mut y = self.dc.run(ex) * self.spec.exhaust;

            // Intake roar, stronger with the throttle open.
            if running {
                let air = (0.08 + self.throttle * 0.9) * smoothstep(0.0, 6000.0, rpm + 1500.0);
                y += self.intake_bp.run(self.noise.white()) * intake_env * air * self.spec.intake;
            }
            // Valvetrain: a tick per valve event.
            self.tick = tick;
            y += self.tick_bp.run(self.tick) * self.spec.mechanical * (0.3 + rpm / 8000.0);

            // Straight-cut gears: whine from the constant mesh, louder under load.
            if t.gear != 0 && t.speed.abs() > 0.5 {
                let f = rpm / 60.0 * self.spec.gear_teeth;
                self.whine_phase = (self.whine_phase + f * dt).fract();
                self.whine2_phase = (self.whine2_phase + f * 2.0 * dt).fract();
                let w = (self.whine_phase * std::f32::consts::TAU).sin() + 0.35 * (self.whine2_phase * std::f32::consts::TAU).sin();
                y += w * self.spec.whine * (0.25 + self.load.abs()) * smoothstep(0.5, 6.0, t.speed.abs());
            }

            // Turbo: boost builds with throttle and revs, whistles, blows off.
            if self.spec.turbo {
                let want = self.throttle * smoothstep(2200.0, 4300.0, rpm);
                self.boost = approach(self.boost, want, if want > self.boost { 0.55 } else { 0.22 }, dt);
                self.whistle_phase = (self.whistle_phase + whistle_hz * dt).fract();
                let whistle = (self.whistle_phase * std::f32::consts::TAU).sin() * 0.6 + self.hiss_bp.run(self.noise.white()) * 0.8;
                y += whistle * self.boost * self.boost * self.spec.turbo_level;
                if self.bov > 1e-3 {
                    let flutter = 0.7 + 0.3 * (self.lift_timer * 2.0 * std::f32::consts::PI * 22.0).sin();
                    y += self.bov_bp.run(self.noise.white()) * self.bov * flutter * self.spec.turbo_level * 3.0;
                    self.bov *= (-dt / 0.28).exp();
                }
            }
            *o += (y * self.spec.gain).tanh() * gain;
        }
    }
}
