//! Realtime audition of the same pulse engine and physical exhaust used by Warp games.

use audio::{Controls, EngineSound, EngineVoice, MufflerGeometry, Sounds, Source};
use std::sync::{Arc, Mutex};

const ROAD_MODELS: [&str; 3] = ["inline_four", "inline_five", "boxer_four"];
const ROAD_BANK: &str = include_str!("../../../crates/audio/src/road-engines.toml");
const BLOCK: usize = 256;
const SCOPE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineConfig {
    pub model: usize,
    pub rpm: f32,
    pub load: f32,
    pub chamber_length: f32,
    pub chamber_diameter: f32,
    pub packing: f32,
    pub volume: f32,
    pub enabled: bool,
}

pub type Settings = EngineConfig;

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            model: 0,
            rpm: 900.0,
            load: 0.08,
            chamber_length: 0.46,
            chamber_diameter: 0.165,
            packing: 0.78,
            volume: 0.65,
            enabled: false,
        }
    }
}

impl EngineConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model >= ROAD_MODELS.len() {
            return Err(format!("Unknown engine model {}", self.model));
        }
        for (name, value, min, max) in [
            ("RPM", self.rpm, 650.0, 8000.0),
            ("load", self.load, -1.0, 1.0),
            ("chamber length", self.chamber_length, 0.10, 1.50),
            ("chamber diameter", self.chamber_diameter, 0.04, 0.35),
            ("packing", self.packing, 0.0, 1.0),
            ("volume", self.volume, 0.0, 1.0),
        ] {
            if !(min..=max).contains(&value) {
                return Err(format!("{name} must be between {min} and {max}"));
            }
        }
        Ok(())
    }

    fn geometry(self) -> MufflerGeometry {
        MufflerGeometry {
            inlet_length: 1.85,
            inlet_diameter: 0.050,
            chamber_length: self.chamber_length,
            chamber_diameter: self.chamber_diameter,
            outlet_length: 0.66,
            outlet_diameter: 0.045,
            packing: self.packing,
        }
    }

    fn controls(self) -> Controls {
        Controls {
            rpm: self.rpm,
            throttle: self.load.max(0.0),
            load: self.load,
            gear: 1,
            ..Default::default()
        }
    }

    /// A full spatial-ready Warp patch. Solver parameters can still be
    /// changed on a playing voice; their defaults reproduce this audition.
    pub fn luau(&self) -> String {
        let model = ROAD_MODELS[self.model];
        let g = self.geometry();
        let c = self.controls();
        format!(
            r#"-- Warp Engine Generator. Preview at {rpm} RPM and load {load}.
local car = {{
    model = "{model}",
    muffler = {{
        inlet_length = {il}, inlet_diameter = {id},
        chamber_length = {cl}, chamber_diameter = {cd},
        outlet_length = {ol}, outlet_diameter = {od},
        packing = {pack},
    }},
}}
loop()
local rpm = param("rpm", {rpm})
local speed = param("speed", 0)
local load = param("load", {load})
local exhaust = engine(car, rpm, param("throttle", {throttle}), load, param("cut", 0), param("gear", 1), speed)
local road = tyres(speed, param("slip", 0), param("contact", 0), param("loose", 0), param("soft", 0), 0)
return (exhaust + road * 0.28) * adsr(0.05, 0.1, 1, 0.12)
"#,
            rpm = c.rpm,
            load = c.load,
            throttle = c.throttle,
            il = g.inlet_length,
            id = g.inlet_diameter,
            cl = g.chamber_length,
            cd = g.chamber_diameter,
            ol = g.outlet_length,
            od = g.outlet_diameter,
            pack = g.packing
        )
    }
}

struct Engines([EngineSound; 3]);

impl Engines {
    fn load() -> Result<Self, String> {
        let bank = Sounds::parse(ROAD_BANK)?;
        let [a, b, c] = ROAD_MODELS;
        Ok(Self([bank.get(a)?, bank.get(b)?, bank.get(c)?]))
    }

    /// Prepare the incoming engine away from the audio callback. Settling
    /// it at the current RPM avoids a false startup sweep on each slider edit.
    fn prepare(&self, s: Settings, rate: f32) -> Box<EngineVoice> {
        let mut spec = self.0[s.model].clone();
        spec.muffler = Some(s.geometry());
        let mut voice = Box::new(EngineVoice::new(spec, rate, 7));
        voice.set(s.controls());
        let mut warmup = [0.0; BLOCK];
        for _ in 0..((rate * 0.12) as usize).div_ceil(BLOCK) {
            warmup.fill(0.0);
            voice.render_add(&mut warmup, 1.0);
        }
        voice
    }
}

struct Pending {
    settings: Settings,
    next: Option<Box<EngineVoice>>,
}

struct Trace {
    samples: [f32; SCOPE],
    cursor: usize,
}

impl Trace {
    fn write(&mut self, output: &[f32], channels: usize) {
        for frame in output.chunks_exact(channels) {
            self.samples[self.cursor] = frame[0];
            self.cursor = (self.cursor + 1) % SCOPE;
        }
    }

    fn read(&self, out: &mut [f32]) {
        let len = out.len();
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.samples[(self.cursor + i * SCOPE / len) % SCOPE];
        }
    }
}

struct Shared {
    pending: Mutex<Pending>,
    trace: Mutex<Trace>,
}

impl Shared {
    fn new(s: Settings) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Pending {
                settings: s,
                next: None,
            }),
            trace: Mutex::new(Trace {
                samples: [0.0; SCOPE],
                cursor: 0,
            }),
        })
    }
}

pub struct Controller {
    speaker: audio::device::Speaker,
    engines: Engines,
    shared: Arc<Shared>,
    settings: Settings,
}

impl Controller {
    /// The speaker tracks and reopens a lost output device.
    pub fn open() -> Result<Self, String> {
        let engines = Engines::load()?;
        let settings = Settings::default();
        let shared = Shared::new(settings);
        let speaker = audio::device::open(|rate, _| {
            Box::new(Audition::new(
                engines.prepare(settings, rate),
                settings,
                rate,
                shared.clone(),
            ))
        })?;
        Ok(Self {
            speaker,
            engines,
            shared,
            settings,
        })
    }

    pub fn output_name(&self) -> &str {
        &self.speaker.output_name
    }

    pub fn update(&mut self, settings: Settings) {
        if self.settings == settings || settings.validate().is_err() {
            return;
        }
        let changed = self.settings.model != settings.model
            || self.settings.geometry() != settings.geometry();
        let next = changed.then(|| self.engines.prepare(settings, self.speaker.rate));
        if let Ok(mut p) = self.shared.pending.lock() {
            p.settings = settings;
            if next.is_some() {
                p.next = next;
            }
            self.settings = settings;
        }
    }

    /// Read the latest 4096 mono output frames, sampled across the UI width.
    pub fn waveform(&self, out: &mut [f32]) {
        if let Ok(trace) = self.shared.trace.lock() {
            trace.read(out);
        }
    }
}

struct Audition {
    current: Box<EngineVoice>,
    next: Option<Box<EngineVoice>>,
    shared: Arc<Shared>,
    settings: Settings,
    fade_frame: usize,
    fade_length: usize,
    gain: f32,
    gain_step: f32,
}

impl Audition {
    fn new(voice: Box<EngineVoice>, settings: Settings, rate: f32, shared: Arc<Shared>) -> Self {
        Self {
            current: voice,
            next: None,
            shared,
            settings,
            fade_frame: 0,
            fade_length: (rate * 0.045).round().max(1.0) as usize,
            gain: 0.0,
            gain_step: 1.0 / (rate * 0.02).max(1.0),
        }
    }
}

impl Source for Audition {
    fn render(&mut self, out: &mut [f32], channels: usize) {
        if channels == 0 {
            out.fill(0.0);
            return;
        }
        if let Ok(mut p) = self.shared.pending.try_lock() {
            self.settings = p.settings;
            // Let an audible crossfade finish. The mailbox retains the newest
            // replacement if the user drags a slider again in those 45 ms.
            if self.next.is_none() {
                if let Some(voice) = p.next.take() {
                    if self.gain == 0.0 {
                        self.current = voice;
                    } else {
                        self.next = Some(voice);
                        self.fade_frame = 0;
                    }
                }
            }
        }
        let controls = self.settings.controls();
        self.current.set(controls);
        if let Some(v) = &mut self.next {
            v.set(controls);
        }

        let target = if self.settings.enabled {
            self.settings.volume
        } else {
            0.0
        };
        out.fill(0.0);
        if target > 0.0 || self.gain > 0.0 {
            let mut a = [0.0; BLOCK];
            let mut b = [0.0; BLOCK];
            for chunk in out.chunks_mut(BLOCK * channels) {
                let n = chunk.len() / channels;
                a[..n].fill(0.0);
                self.current.render_add(&mut a[..n], 1.0);
                if let Some(next) = &mut self.next {
                    b[..n].fill(0.0);
                    next.render_add(&mut b[..n], 1.0);
                    for (x, &y) in a[..n].iter_mut().zip(&b) {
                        self.fade_frame = (self.fade_frame + 1).min(self.fade_length);
                        *x += (y - *x) * (self.fade_frame as f32 / self.fade_length as f32);
                    }
                    if self.fade_frame == self.fade_length {
                        self.current = self.next.take().unwrap();
                    }
                }
                for (frame, sample) in chunk.chunks_exact_mut(channels).zip(&a) {
                    self.gain += (target - self.gain).clamp(-self.gain_step, self.gain_step);
                    let mono = sample * self.gain;
                    frame[0] = mono;
                    if frame.len() > 1 {
                        frame[1] = mono;
                    }
                }
            }
        } else if let Some(voice) = self.next.take() {
            self.current = voice;
        }
        if let Ok(mut trace) = self.shared.trace.try_lock() {
            trace.write(out, channels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_models_render_different_voices_and_the_actual_scope_samples() {
        let engines = Engines::load().unwrap();
        let mut rendered = Vec::new();
        for model in 0..3 {
            let s = Settings {
                model,
                rpm: 3000.0,
                load: 0.6,
                enabled: true,
                ..Default::default()
            };
            s.validate().unwrap();
            let shared = Shared::new(s);
            let mut output =
                Audition::new(engines.prepare(s, 48_000.0), s, 48_000.0, shared.clone());
            let mut pcm = vec![0.0; 48_000];
            output.render(&mut pcm, 2);
            assert!(pcm.iter().all(|x| x.is_finite() && x.abs() <= s.volume));
            assert!(pcm.chunks_exact(2).all(|f| f[0] == f[1]));
            assert!(pcm.iter().any(|x| x.abs() > 0.02));
            let mut scope = [0.0; SCOPE];
            shared.trace.lock().unwrap().read(&mut scope);
            assert!(
                scope.iter().eq(pcm
                    .chunks_exact(2)
                    .skip(pcm.len() / 2 - SCOPE)
                    .map(|f| &f[0]))
            );
            let mut surround = [0.0; 480 * 6];
            output.render(&mut surround, 6);
            assert!(
                surround
                    .chunks_exact(6)
                    .all(|frame| frame[0] == frame[1] && frame[2..].iter().all(|x| *x == 0.0))
            );
            rendered.push(pcm);
        }
        assert!(rendered[0] != rendered[1] && rendered[0] != rendered[2]);
        assert!(
            Settings {
                rpm: f32::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings {
                model: 3,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn exported_luau_compiles_and_stock_geometry_renders_the_shipping_patch() {
        use audio::synth::{Unit, lua};
        let load_patch = |source: &str| lua::load("motor", source).unwrap().remove(0);
        let stock = Settings::default();
        let shipped = load_patch(include_str!(
            "../../../../games/warp-theft-auto/sounds/motor.luau"
        ));
        let params = [
            ("rpm", 3270.0),
            ("throttle", 0.8),
            ("load", 0.61),
            ("speed", 19.0),
            ("gear", 2.0),
            ("contact", 1.0),
        ];
        let render = |p: &audio::synth::Patch| {
            p.compile()
                .unwrap()
                .voice(&params, 11, 48_000.0)
                .render_all(0.25)
        };
        let reference = render(&shipped);
        assert_eq!(render(&load_patch(&stock.luau())), reference);
        for model in 0..3 {
            let custom = Settings {
                model,
                rpm: 2170.0,
                load: -0.2,
                chamber_length: 0.83,
                chamber_diameter: 0.21,
                packing: 0.37,
                ..stock
            };
            let patch = load_patch(&custom.luau());
            assert!(
                patch.looping
                    && patch.params.contains(&("rpm".into(), custom.rpm))
                    && patch.params.contains(&("load".into(), custom.load))
                    && patch.params.contains(&("throttle".into(), 0.0))
            );
            assert!(patch.nodes.iter().any(
                |n| matches!(&n.unit, Unit::Engine {car, fluid: false, muffler: Some(g), ..}
                if car == ROAD_MODELS[model] && *g == custom.geometry())
            ));
            let samples = render(&patch);
            assert!(samples != reference && samples.iter().any(|v| v.abs() > 0.02));
        }
    }

    #[test]
    fn rapid_shape_edits_complete_and_stop_reaches_silence() {
        let engines = Engines::load().unwrap();
        let s = Settings {
            rpm: 2200.0,
            load: 0.4,
            enabled: true,
            ..Default::default()
        };
        let shared = Shared::new(s);
        let mut output = Audition::new(engines.prepare(s, 48_000.0), s, 48_000.0, shared.clone());
        let mut pcm = [0.0; 480];
        output.render(&mut pcm, 1);
        for diameter in [0.08, 0.20, 0.28] {
            let edit = Settings {
                chamber_diameter: diameter,
                ..s
            };
            let voice = engines.prepare(edit, 48_000.0);
            let mut p = shared.pending.lock().unwrap();
            p.settings = edit;
            p.next = Some(voice);
            drop(p);
            output.render(&mut pcm, 1);
        }
        for _ in 0..20 {
            output.render(&mut pcm, 1);
        }
        assert!(output.next.is_none());
        assert_eq!(output.current.spec.muffler.unwrap().chamber_diameter, 0.28);
        shared.pending.lock().unwrap().settings.enabled = false;
        for _ in 0..5 {
            output.render(&mut pcm, 1);
        }
        assert!(pcm.iter().all(|&x| x == 0.0));
        shared.pending.lock().unwrap().settings.enabled = true;
        output.render(&mut pcm, 1);
        assert!(pcm[0].abs() <= output.gain_step && pcm.iter().any(|x| x.abs() > 0.01));
    }
}
