//! Live engine sound for `play`: the audio device (cpal) pulls from
//! `audio::stream::LiveMix`; each frame the game places every car for the
//! listeners (the chase cameras) and hands over what its engine does.
//!
//! The cars nearest a listener get the gas-dynamics exhaust (`--sound
//! fluid`, the default), the rest the cheap pulse model. Without an audio
//! device the game runs silently.

use audio::stream::{CarInput, Feeder, LiveMix};
use audio::{Model, Placement, Sounds};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use glam::Vec3;
use render::Camera;
use sideways::race::Race;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Pulse,
    Fluid,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s {
            "off" => Ok(Mode::Off),
            "pulse" => Ok(Mode::Pulse),
            "fluid" => Ok(Mode::Fluid),
            _ => Err(format!("--sound must be off, pulse or fluid, not {s}")),
        }
    }
}

pub struct Sound {
    _stream: cpal::Stream,
    feeder: Feeder,
    cars: usize,
    ears: Vec<Vec3>,
}

impl Sound {
    /// Open the default output device with a voice per car of `race`;
    /// `listeners` cars get the fluid model when `mode` is Fluid.
    pub fn start(race: &Race, listeners: usize, mode: Mode) -> Result<Sound, String> {
        let sounds = Sounds::load(None)?;
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| format!("audio config: {e}"))?;
        let config = supported.config();
        let sr = config.sample_rate as f32;
        let channels = config.channels as usize;
        let cars = race
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let model = if mode == Mode::Fluid && i < listeners { Model::Fluid } else { Model::Pulse };
                sounds.get(crate::sound::sound_id(&p.car)).map(|s| (s, model))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (mut mix, feeder) = LiveMix::start(cars, sr);
        let err = |e| eprintln!("audio: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(config, move |out: &mut [f32], _: &_| mix.fill(out, channels), err, None),
            cpal::SampleFormat::I16 => {
                let mut buf = Vec::new();
                device.build_output_stream(
                    config,
                    move |out: &mut [i16], _: &_| {
                        buf.resize(out.len(), 0.0);
                        mix.fill(&mut buf, channels);
                        for (o, s) in out.iter_mut().zip(&buf) {
                            *o = (s * 32767.0) as i16;
                        }
                    },
                    err,
                    None,
                )
            }
            f => return Err(format!("audio: unsupported sample format {f:?}")),
        }
        .map_err(|e| format!("audio stream: {e}"))?;
        stream.play().map_err(|e| format!("audio play: {e}"))?;
        eprintln!("audio: {} car(s) at {sr} Hz, {channels} channels, fluid exhaust for {}", race.players.len(), if mode == Mode::Fluid { listeners.min(race.players.len()) } else { 0 });
        Ok(Sound { _stream: stream, feeder, cars: race.players.len(), ears: Vec::new() })
    }

    pub fn cars(&self) -> usize {
        self.cars
    }

    pub fn pause(&self, paused: bool) {
        self.feeder.pause(paused);
    }

    /// (blocks mixed, late blocks)
    pub fn stats(&self) -> (u64, u64) {
        self.feeder.stats()
    }

    /// Every car's engine and tyres, placed for the nearest listener.
    pub fn update(&mut self, race: &Race, cams: &[Camera], dt: f32) {
        let ears: Vec<Vec3> = cams.iter().map(|c| c.pos).collect();
        let vel: Vec<Vec3> = ears.iter().enumerate().map(|(i, e)| self.ears.get(i).map(|p| (*e - *p) / dt.max(1e-3)).unwrap_or(Vec3::ZERO)).collect();
        let split = cams.len() > 1;
        let inputs: Vec<CarInput> = race
            .players
            .iter()
            .map(|p| {
                let (engine, tyres) = crate::sound::controls(&p.car, &race.phys);
                let t = p.car.pose(&race.phys).translation;
                let v = p.car.linvel(&race.phys);
                let pos = Vec3::new(t.x, t.y, t.z);
                let v = Vec3::new(v.x, v.y, v.z);
                let mut best = Placement { gain: 0.0, ..Default::default() };
                for (k, c) in cams.iter().enumerate() {
                    let fwd = (c.target - c.pos).normalize_or(Vec3::NEG_Z);
                    let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
                    let rel = pos - c.pos;
                    let closing = (vel[k] - v).dot(rel.normalize_or_zero());
                    let mut pl = audio::mix::place(rel.to_array(), right.to_array(), closing);
                    if split {
                        // One speaker pair for several views: keep it central.
                        pl.pan *= 0.3;
                    }
                    if pl.gain > best.gain {
                        best = pl;
                    }
                }
                CarInput { engine, tyres, place: best }
            })
            .collect();
        self.feeder.set(&inputs);
        self.ears = ears;
    }
}
