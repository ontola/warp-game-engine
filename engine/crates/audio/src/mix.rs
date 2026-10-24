//! The mix: every car's engine and tyres, placed for the listeners.

use crate::dsp::OnePole;
use crate::engine::{Controls, EngineSound, EngineVoice, Model};
use crate::tyres::{TyreControls, TyreVoice};

/// Where a car sits for the listener(s), worked out by the game per tick.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Linear gain (distance, occlusion).
    pub gain: f32,
    /// -1 left .. 1 right.
    pub pan: f32,
    /// Air absorption: a low-pass cutoff (Hz) growing shorter with distance.
    pub air_hz: f32,
    /// Doppler factor (>1 approaching): scales every pitch of the voice.
    pub doppler: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Placement { gain: 1.0, pan: 0.0, air_hz: 18_000.0, doppler: 1.0 }
    }
}

/// Distance and direction to a placement: gain falls as 1/d beyond the
/// first few metres, highs fall off with distance, approach raises pitch.
pub fn place(rel: [f32; 3], right: [f32; 3], closing_speed: f32) -> Placement {
    let d = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
    let gain = 1.0 / (1.0 + (d - 4.0).max(0.0) / 6.0);
    let pan = if d > 1e-3 { ((rel[0] * right[0] + rel[1] * right[1] + rel[2] * right[2]) / d).clamp(-1.0, 1.0) * 0.8 } else { 0.0 };
    let air_hz = (18_000.0 / (1.0 + d / 60.0)).max(1200.0);
    let doppler = (343.0 / (343.0 - closing_speed.clamp(-120.0, 120.0))).clamp(0.6, 1.6);
    Placement { gain, pan, air_hz, doppler }
}

pub struct CarVoice {
    pub engine: EngineVoice,
    pub tyres: TyreVoice,
    pub controls: Controls,
    pub tyre_controls: TyreControls,
    pub place: Placement,
    gain: f32,
    pan: f32,
    air: [OnePole; 2],
    scratch: Vec<f32>,
}

pub struct Mixer {
    pub sr: f32,
    pub cars: Vec<CarVoice>,
    pub master: f32,
    /// Tyre level in the mix (0 mutes them, for diagnosis).
    pub tyres: f32,
}

impl Mixer {
    pub fn new(sr: f32) -> Mixer {
        Mixer { sr, cars: Vec::new(), master: 0.8, tyres: crate::tyres::MIX_LEVEL }
    }

    pub fn add_car(&mut self, sound: EngineSound, seed: u32) -> usize {
        self.add_car_model(sound, seed, Model::Pulse)
    }

    pub fn add_car_model(&mut self, sound: EngineSound, seed: u32, model: Model) -> usize {
        self.cars.push(CarVoice {
            engine: EngineVoice::with_model(sound, self.sr, seed, model),
            tyres: TyreVoice::new(self.sr, seed),
            controls: Controls::default(),
            tyre_controls: TyreControls::default(),
            place: Placement::default(),
            gain: 0.0,
            pan: 0.0,
            air: [OnePole::new(18_000.0, self.sr), OnePole::new(18_000.0, self.sr)],
            scratch: Vec::new(),
        });
        self.cars.len() - 1
    }

    /// Render interleaved stereo frames.
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        out.iter_mut().for_each(|x| *x = 0.0);
        for car in &mut self.cars {
            let mut c = car.controls;
            // Doppler: the whole voice runs faster or slower.
            c.rpm *= car.place.doppler;
            car.engine.set(c);
            car.tyres.set(car.tyre_controls);
            car.scratch.clear();
            car.scratch.resize(frames, 0.0);
            car.engine.render_add(&mut car.scratch, 1.0);
            car.tyres.render_add(&mut car.scratch, self.tyres);
            car.air[0].set_hz(car.place.air_hz, self.sr);
            car.air[1].set_hz(car.place.air_hz, self.sr);
            // Glide gain and pan over the block: no clicks.
            let (g0, p0) = (car.gain, car.pan);
            let (g1, p1) = (car.place.gain, car.place.pan);
            for (i, s) in car.scratch.iter().enumerate() {
                let t = (i + 1) as f32 / frames.max(1) as f32;
                let g = g0 + (g1 - g0) * t;
                let p = p0 + (p1 - p0) * t;
                let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
                let (r, l) = a.sin_cos();
                out[2 * i] += car.air[0].run(*s) * g * l;
                out[2 * i + 1] += car.air[1].run(*s) * g * r;
            }
            car.gain = g1;
            car.pan = p1;
        }
        for x in out.iter_mut() {
            *x = (*x * self.master).tanh();
        }
    }
}
