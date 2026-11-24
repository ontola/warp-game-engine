//! Kern audio. Engine sound is synthesised from the running engine rather
//! than played back, after Ange Yaghi's engine-sim: `engine` is a cheap
//! pulse-and-waveguide voice for every car, `fluid` (ES3D-style) solves the
//! gas dynamics in the headers and pipe for the cars closest to a listener.
//! Everything is deterministic and runs offline too. Applications provide
//! their voice presets and can render WAVs and spectrograms for testing.

pub mod analysis;
#[cfg(feature = "luau")]
pub mod cli;
#[cfg(feature = "device")]
pub mod device;
pub mod dsp;
pub mod engine;
pub mod es;
pub mod fluid;
pub mod mix;
pub mod muffler;
pub mod sfx;
pub mod sounds;
pub mod stream;
pub mod synth;
pub mod tyres;

pub use engine::{Controls, EngineSound, EngineVoice, Model};
pub use mix::{Mixer, Placement};
pub use muffler::MufflerGeometry;
pub use sounds::Sounds;

pub const SAMPLE_RATE: f32 = 48_000.0;

/// Something that fills interleaved output buffers on the audio thread: a
/// mixer, a music bed, several of them summed.
pub trait Source: Send {
    /// Overwrite `out` (interleaved, `channels` per frame).
    fn render(&mut self, out: &mut [f32], channels: usize);
}

impl Source for sfx::SfxMixer {
    fn render(&mut self, out: &mut [f32], channels: usize) { out.fill(0.0); self.render_add(out, channels); }
}

/// Several sources mixed, optionally through a peak limiter.
pub struct Sum { sources: Vec<Box<dyn Source>>, buf: Vec<f32>, ceiling: Option<f32>, gain: f32 }

impl Sum {
    /// Summed as they are.
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self { Sum { sources, buf: Vec::new(), ceiling: None, gain: 1.0 } }
    /// Summed, then held under `ceiling_db` dBFS (instant attack, 100 ms release).
    pub fn limited(sources: Vec<Box<dyn Source>>, ceiling_db: f32) -> Self { Sum { ceiling: Some(10f32.powf(ceiling_db / 20.0)), ..Sum::new(sources) } }
}

impl Source for Sum {
    fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        self.buf.resize(out.len(), 0.0);
        for s in &mut self.sources {
            s.render(&mut self.buf, channels);
            for (o, v) in out.iter_mut().zip(&self.buf) { *o += v; }
        }
        let Some(ceiling) = self.ceiling else { return };
        let release = (-1.0f32 / (0.1 * SAMPLE_RATE)).exp();
        for frame in out.chunks_mut(channels.max(1)) {
            let peak = frame.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let released = 1.0 + (self.gain - 1.0) * release;
            self.gain = if peak > 0.0 { released.min(ceiling / peak) } else { released };
            for v in frame.iter_mut() { *v = (*v * self.gain).clamp(-1.0, 1.0); }
        }
    }
}

#[cfg(test)]
mod sum_tests {
    use super::*;
    struct Tone(f32);
    impl Source for Tone { fn render(&mut self, out: &mut [f32], _: usize) { for (i, o) in out.iter_mut().enumerate() { *o = self.0 * (i as f32 * 0.05).sin(); } } }

    #[test]
    fn a_limited_sum_stays_under_its_ceiling() {
        let mut s = Sum::limited(vec![Box::new(Tone(0.8)), Box::new(Tone(0.8))], -0.3);
        let mut out = vec![0.0; 9600];
        s.render(&mut out, 2);
        assert!(out.iter().all(|v| v.abs() <= 0.967), "{}", out.iter().fold(0.0f32, |m, v| m.max(v.abs())));
        let mut plain = Sum::new(vec![Box::new(Tone(0.8)), Box::new(Tone(0.8))]);
        plain.render(&mut out, 2);
        assert!(out.iter().any(|v| v.abs() > 1.5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::rms;

    const VOICES: &str = include_str!("road-engines.toml");

    fn steady(id: &str, rpm: f32, secs: f32) -> (Vec<f32>, f32) {
        let sounds = Sounds::parse(VOICES).unwrap();
        let mut v = EngineVoice::new(sounds.get(id).unwrap(), SAMPLE_RATE, 7);
        v.set(Controls { rpm, throttle: 1.0, load: 0.9, cut: false, gear: 3, speed: 20.0 });
        let mut out = vec![0.0; (SAMPLE_RATE * secs) as usize];
        for chunk in out.chunks_mut(400) {
            v.render_add(chunk, 1.0);
        }
        let hz = v.firing_hz(1.0);
        (out[(SAMPLE_RATE * 0.5) as usize..].to_vec(), hz)
    }

    /// Amplitude at `f` (single-bin DFT).
    fn amp(x: &[f32], f: f32) -> f32 {
        let w = 2.0 * std::f32::consts::PI * f / SAMPLE_RATE;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, v) in x.iter().enumerate() {
            re += v * (w * i as f32).cos();
            im += v * (w * i as f32).sin();
        }
        (re * re + im * im).sqrt() / x.len() as f32
    }

    #[test]
    fn every_fixture_voice_loads() {
        let s = Sounds::parse(VOICES).unwrap();
        for id in s.ids() {
            let e = s.get(&id).unwrap();
            assert_eq!(e.firing_order.len(), e.cylinders, "{id}");
            assert_eq!(e.headers.len(), e.cylinders, "{id}");
        }
    }

    #[test]
    fn firing_order_shapes_the_spectrum() {
        // An inline four at 3000 rpm fires at 100 Hz: order 2 dominates order 1.
        let (x, per_rpm) = steady("inline_four", 3000.0, 1.5);
        let crank = 50.0;
        assert!((per_rpm * 3000.0 - 100.0).abs() < 1e-3);
        assert!(amp(&x, 2.0 * crank) > 8.0 * amp(&x, crank), "four: order 2 over 1");
        // A five fires 2.5 times a revolution.
        let (x, _) = steady("inline_five", 3000.0, 1.5);
        assert!(amp(&x, 2.5 * crank) > 8.0 * amp(&x, 2.0 * crank), "five: order 2.5 over 2");
        // Unequal headers on the boxer bring out the odd orders (the burble).
        let (boxer, _) = steady("boxer_four", 3000.0, 1.5);
        let (inline, _) = steady("inline_four", 3000.0, 1.5);
        let odd = |x: &[f32]| amp(x, crank) / amp(x, 2.0 * crank);
        assert!(odd(&boxer) > 4.0 * odd(&inline), "boxer odd orders");
    }

    #[test]
    fn deterministic_and_bounded() {
        let (a, _) = steady("inline_five", 6500.0, 1.0);
        let (b, _) = steady("inline_five", 6500.0, 1.0);
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        assert!(rms(&a) > 0.01);
    }

    #[test]
    fn fluid_exhaust_keeps_the_firing_order_and_stays_finite() {
        let sounds = Sounds::parse(VOICES).unwrap();
        for (id, order, under) in [("inline_four", 2.0, 1.0), ("inline_five", 2.5, 2.0)] {
            let mut v = EngineVoice::with_model(sounds.get(id).unwrap(), SAMPLE_RATE, 3, Model::Fluid);
            v.set(Controls { rpm: 3000.0, throttle: 1.0, load: 0.9, cut: false, gear: 3, speed: 20.0 });
            let mut out = vec![0.0; (SAMPLE_RATE * 1.2) as usize];
            for chunk in out.chunks_mut(400) {
                v.render_add(chunk, 1.0);
            }
            let x = &out[(SAMPLE_RATE * 0.4) as usize..];
            assert!(x.iter().all(|s| s.is_finite()), "{id}: finite");
            // No numerical clicks: peaks stay within a sane crest factor.
            let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak < 8.0 * rms(x), "{id}: crest {}", peak / rms(x));
            assert!(amp(x, order * 50.0) > 4.0 * amp(x, under * 50.0), "{id}: order {order} over {under}");
        }
    }

    #[test]
    fn live_mix_streams_without_blocking() {
        let sounds = Sounds::parse(VOICES).unwrap();
        let (mut mix, feeder) = stream::LiveMix::start(vec![(sounds.get("inline_four").unwrap(), Model::Pulse), (sounds.get("inline_five").unwrap(), Model::Pulse)], SAMPLE_RATE);
        let input = stream::CarInput { engine: Controls { rpm: 4000.0, throttle: 0.6, load: 0.5, cut: false, gear: 2, speed: 15.0 }, ..Default::default() };
        feeder.set(&[input, input]);
        let mut out = vec![0.0; 960];
        let mut energy = 0.0;
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            mix.fill(&mut out, 2);
            energy += rms(&out);
        }
        assert!(energy > 0.1, "sound came through");
        feeder.pause(true);
        mix.fill(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0), "paused is silent");
        drop(mix); // joins the workers
    }

    #[test]
    fn silent_wheels_in_the_air() {
        let mut t = tyres::TyreVoice::new(SAMPLE_RATE, 1);
        t.set(tyres::TyreControls { speed: 30.0, contact: 0.0, loose: 1.0, ..Default::default() });
        let mut out = vec![0.0; 4800];
        t.render_add(&mut out, 1.0);
        assert!(rms(&out) < 1e-4);
    }
}
