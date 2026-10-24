//! Live audio: every car is synthesised on its own worker thread, a few
//! milliseconds ahead, and the device callback only places and mixes the
//! finished blocks. The game never waits on audio and a slow voice (the
//! gas-dynamics exhaust) never stalls the callback: it drops out briefly
//! and the underrun is counted.
//!
//!   game thread --Feeder::set--> shared inputs (latest wins)
//!   worker per car: inputs -> EngineVoice + TyreVoice -> 5 ms mono blocks
//!   device callback: LiveMix::fill pulls blocks, pans, filters, mixes

use crate::dsp::OnePole;
use crate::engine::{Controls, EngineSound, EngineVoice, Model};
use crate::mix::Placement;
use crate::tyres::{TyreControls, TyreVoice};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};

/// Samples per worker block (5 ms at 48 kHz).
pub const BLOCK: usize = 240;
/// Blocks a worker may run ahead: latency and slack against hiccups.
pub const AHEAD: usize = 3;

#[derive(Clone, Copy, Debug, Default)]
pub struct CarInput {
    pub engine: Controls,
    pub tyres: TyreControls,
    pub place: Placement,
}

struct Shared {
    inputs: Mutex<Vec<CarInput>>,
    running: AtomicBool,
    paused: AtomicBool,
    underruns: AtomicU64,
    blocks: AtomicU64,
}

/// The game's handle: set what each car does, pause, read statistics.
#[derive(Clone)]
pub struct Feeder {
    shared: Arc<Shared>,
}

impl Feeder {
    pub fn set(&self, inputs: &[CarInput]) {
        if let Ok(mut v) = self.shared.inputs.lock() {
            for (d, s) in v.iter_mut().zip(inputs) {
                *d = *s;
            }
        }
    }

    /// Paused: workers idle and the output is silent.
    pub fn pause(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    /// (blocks mixed, blocks a voice was late)
    pub fn stats(&self) -> (u64, u64) {
        (self.shared.blocks.load(Ordering::Relaxed), self.shared.underruns.load(Ordering::Relaxed))
    }
}

struct Lane {
    rx: Receiver<Vec<f32>>,
    pending: VecDeque<f32>,
    gain: f32,
    pan: f32,
    air: [OnePole; 2],
}

/// The callback side.
pub struct LiveMix {
    shared: Arc<Shared>,
    lanes: Vec<Lane>,
    places: Vec<Placement>,
    workers: Vec<std::thread::JoinHandle<()>>,
    sr: f32,
    pub master: f32,
}

impl LiveMix {
    /// One worker per car; `models` picks each car's exhaust model.
    pub fn start(cars: Vec<(EngineSound, Model)>, sr: f32) -> (LiveMix, Feeder) {
        let shared = Arc::new(Shared {
            inputs: Mutex::new(vec![CarInput::default(); cars.len()]),
            running: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            blocks: AtomicU64::new(0),
        });
        let mut lanes = Vec::new();
        let mut workers = Vec::new();
        for (i, (sound, model)) in cars.into_iter().enumerate() {
            let (tx, rx) = sync_channel::<Vec<f32>>(AHEAD);
            let sh = shared.clone();
            workers.push(
                std::thread::Builder::new()
                    .name(format!("audio car {i}"))
                    .spawn(move || worker(i, sound, model, sr, sh, tx))
                    .expect("audio worker thread"),
            );
            lanes.push(Lane { rx, pending: VecDeque::new(), gain: 0.0, pan: 0.0, air: [OnePole::new(18_000.0, sr), OnePole::new(18_000.0, sr)] });
        }
        let n = lanes.len();
        let feeder = Feeder { shared: shared.clone() };
        (LiveMix { shared, lanes, places: vec![Placement::default(); n], workers, sr, master: 0.8 }, feeder)
    }

    /// Fill an interleaved buffer of `channels` (the stereo mix goes to the
    /// first two, the rest stay silent). Never blocks.
    pub fn fill(&mut self, out: &mut [f32], channels: usize) {
        out.iter_mut().for_each(|x| *x = 0.0);
        let channels = channels.max(1);
        let frames = out.len() / channels;
        if self.shared.paused.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(v) = self.shared.inputs.try_lock() {
            for (p, i) in self.places.iter_mut().zip(v.iter()) {
                *p = i.place;
            }
        }
        for (k, lane) in self.lanes.iter_mut().enumerate() {
            // Gather enough samples, without waiting.
            while lane.pending.len() < frames {
                match lane.rx.try_recv() {
                    Ok(b) => lane.pending.extend(b),
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                }
            }
            if lane.pending.len() < frames {
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
            let place = self.places[k];
            for a in lane.air.iter_mut() {
                a.set_hz(place.air_hz, self.sr);
            }
            let (g0, p0, g1, p1) = (lane.gain, lane.pan, place.gain, place.pan);
            for f in 0..frames {
                let s = lane.pending.pop_front().unwrap_or(0.0);
                let t = (f + 1) as f32 / frames as f32;
                let g = g0 + (g1 - g0) * t;
                let a = (p0 + (p1 - p0) * t + 1.0) * std::f32::consts::FRAC_PI_4;
                let (r, l) = a.sin_cos();
                let left = lane.air[0].run(s) * g * l;
                let right = lane.air[1].run(s) * g * r;
                if channels == 1 {
                    out[f] += 0.5 * (left + right);
                } else {
                    out[f * channels] += left;
                    out[f * channels + 1] += right;
                }
            }
            lane.gain = g1;
            lane.pan = p1;
        }
        for x in out.iter_mut() {
            *x = (*x * self.master).tanh();
        }
        self.shared.blocks.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for LiveMix {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        // Unblock workers waiting on a full channel.
        for lane in &self.lanes {
            while lane.rx.try_recv().is_ok() {}
        }
        self.lanes.clear();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn worker(i: usize, sound: EngineSound, model: Model, sr: f32, shared: Arc<Shared>, tx: SyncSender<Vec<f32>>) {
    let mut engine = EngineVoice::with_model(sound, sr, i as u32 + 1, model);
    let mut tyres = TyreVoice::new(sr, i as u32 + 1);
    while shared.running.load(Ordering::Relaxed) {
        if shared.paused.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }
        let input = shared.inputs.lock().map(|v| v[i]).unwrap_or_default();
        let mut c = input.engine;
        c.rpm *= input.place.doppler;
        engine.set(c);
        tyres.set(input.tyres);
        let mut block = vec![0.0; BLOCK];
        engine.render_add(&mut block, 1.0);
        tyres.render_add(&mut block, 0.5);
        // Blocks while AHEAD blocks wait: that paces the worker.
        if tx.send(block).is_err() {
            break;
        }
    }
}
