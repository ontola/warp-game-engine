//! The live side: voices placed in the speaker layout, a reverb bus with
//! as many channels as the layout, a limiter. The game thread builds voices
//! (allocation happens there) and sends them; the audio thread only mixes
//! and hands finished voices back to be dropped off the audio thread.
use super::patch::{Patch, Voice};
use super::reverb::Reverb;
use super::spatial::{place_into, Ear, Falloff, Layout, Place, MAX_SPEAKERS};
use super::BLOCK;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// How to play a patch.
#[derive(Clone, Debug)]
pub struct Play {
    pub place: Place,
    pub falloff: Falloff,
    /// 0 a point .. 1 all around.
    pub spread: f32,
    /// Linear gain.
    pub gain: f32,
    /// Share sent to the reverb bus.
    pub reverb: f32,
    /// LFE send (0..1).
    pub lfe: f32,
    /// 0 clear .. 1 fully behind walls.
    pub occlusion: f32,
    /// Arrive after distance / 343 m/s.
    pub sound_delay: bool,
    pub seed: u32,
}

impl Default for Play {
    fn default() -> Self { Play { place: Place::Head, falloff: Falloff::default(), spread: 0.0, gain: 1.0, reverb: 0.0, lfe: 0.0, occlusion: 0.0, sound_delay: false, seed: 1 } }
}

impl Play {
    pub fn at(p: [f32; 3]) -> Self { Play { place: Place::At(p), ..Default::default() } }
    pub fn db(mut self, db: f32) -> Self { self.gain *= 10f32.powf(db / 20.0); self }
    pub fn seed(mut self, s: u32) -> Self { self.seed = s; self }
}

/// A playing sound, to move it, change its parameters or stop it.
#[derive(Clone)]
pub struct Handle { id: u32, patch: Arc<Patch> }

impl Handle {
    pub fn id(&self) -> u32 { self.id }
}

struct Playing {
    id: u32,
    voice: Voice,
    play: Play,
    /// Samples to wait before starting (sound travel).
    wait: usize,
    gains: [f32; MAX_SPEAKERS],
    prev: [f32; MAX_SPEAKERS],
    lp: f32,
    lp_hz: f32,
    stopping: bool,
}

enum Cmd {
    Play(Box<Playing>),
    Param(u32, u16, f32),
    Move(u32, Place, f32),
    Release(u32),
    Stop(u32),
    StopAll,
    Ears(Vec<Ear>),
    Layout(Layout),
    Volume(f32),
    Room { decay: f32, damping: f32, level: f32 },
}

/// Game side. Cheap to clone.
#[derive(Clone)]
pub struct SoundSender { tx: Sender<Cmd>, next: Arc<AtomicU32>, sr: f32 }

/// What the audio thread hands back to be dropped elsewhere.
pub struct Garbage(#[allow(dead_code)] Trash);
enum Trash { Voice(#[allow(dead_code)] Box<Playing>), Ears(#[allow(dead_code)] Vec<Ear>), Layout(#[allow(dead_code)] Layout) }

/// Audio side: lives in the device callback (or a test).
pub struct Mixer {
    rx: Receiver<Cmd>,
    trash: Sender<Garbage>,
    trash_rx: Option<Receiver<Garbage>>,
    voices: Vec<Box<Playing>>,
    layout: Layout,
    ears: Vec<Ear>,
    reverb: Reverb,
    master: f32,
    limiter: f32,
    sr: f32,
    /// Output frames rendered but not yet taken (interleaved).
    pending: Vec<f32>,
    taken: usize,
    block: Vec<f32>,
    wet: Vec<f32>,
    send: [f32; BLOCK],
    /// Voices dropped because the pool was full.
    pub stolen: u64,
    max_voices: usize,
}

/// A connected sender and mixer for `layout` at `sr`.
pub fn mixer(layout: Layout, max_voices: usize, sr: f32) -> (SoundSender, Mixer) {
    let (tx, rx) = channel();
    let (trash, trash_rx) = channel();
    let ch = layout.channels();
    (SoundSender { tx, next: Arc::new(AtomicU32::new(1)), sr },
     Mixer { rx, trash, trash_rx: Some(trash_rx), voices: Vec::with_capacity(max_voices), reverb: Reverb::new(ch, sr), layout, ears: Vec::new(), master: 1.0, limiter: 1.0, sr,
        pending: Vec::new(), taken: 0, block: vec![0.0; BLOCK * ch], wet: vec![0.0; BLOCK * ch], send: [0.0; BLOCK], stolen: 0, max_voices: max_voices.max(1) })
}

impl SoundSender {
    /// Start a patch. The voice is built here, on the calling thread.
    pub fn play(&self, patch: &Arc<Patch>, params: &[(&str, f32)], play: Play) -> Handle {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let voice = patch.voice(params, play.seed, self.sr);
        let p = Box::new(Playing { id, voice, play, wait: 0, gains: [0.0; MAX_SPEAKERS], prev: [0.0; MAX_SPEAKERS], lp: 0.0, lp_hz: 20_000.0, stopping: false });
        let _ = self.tx.send(Cmd::Play(p));
        Handle { id, patch: patch.clone() }
    }
    /// Glide a parameter to `value` over the next block.
    pub fn set(&self, h: &Handle, name: &str, value: f32) {
        if let Some(k) = h.patch.params.iter().position(|p| p.0 == name) { let _ = self.tx.send(Cmd::Param(h.id, k as u16, value)); }
    }
    pub fn move_to(&self, h: &Handle, place: Place, occlusion: f32) { let _ = self.tx.send(Cmd::Move(h.id, place, occlusion)); }
    /// Gate off: ADSRs release, loops end after their tails.
    pub fn release(&self, h: &Handle) { let _ = self.tx.send(Cmd::Release(h.id)); }
    pub fn stop(&self, h: &Handle) { let _ = self.tx.send(Cmd::Stop(h.id)); }
    pub fn stop_all(&self) { let _ = self.tx.send(Cmd::StopAll); }
    pub fn ears(&self, ears: Vec<Ear>) { let _ = self.tx.send(Cmd::Ears(ears)); }
    pub fn layout(&self, l: Layout) { let _ = self.tx.send(Cmd::Layout(l)); }
    pub fn volume(&self, g: f32) { let _ = self.tx.send(Cmd::Volume(g)); }
    /// The reverb bus: decay (s, to -60 dB), damping 0..1, return level.
    pub fn room(&self, decay: f32, damping: f32, level: f32) { let _ = self.tx.send(Cmd::Room { decay, damping, level }); }
}

impl Mixer {
    pub fn channels(&self) -> usize { self.layout.channels() }
    pub fn active(&self) -> usize { self.voices.len() }
    /// The receiver for finished voices: drop them on a non-audio thread
    /// (taking it out; without it they are dropped here).
    pub fn take_trash(&mut self) -> Option<Receiver<Garbage>> { self.trash_rx.take() }

    fn bin(&self, g: Trash) { if self.trash_rx.is_none() { let _ = self.trash.send(Garbage(g)); } }

    fn drain(&mut self) {
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Play(mut p) => {
                    if self.voices.len() >= self.max_voices {
                        // Steal the quietest-placed voice.
                        let k = (0..self.voices.len()).min_by(|&a, &b| self.voices[a].play.gain.total_cmp(&self.voices[b].play.gain)).unwrap();
                        let v = self.voices.swap_remove(k);
                        self.bin(Trash::Voice(v));
                        self.stolen += 1;
                    }
                    self.place(&mut p);
                    p.prev = p.gains;
                    if p.play.sound_delay { p.wait = (self.dist(&p) / 343.0 * self.sr) as usize; }
                    p.lp = 0.0;
                    self.voices.push(p);
                }
                Cmd::Param(id, k, v) => if let Some(p) = self.voices.iter_mut().find(|p| p.id == id) { p.voice.set_index(k as usize, v) },
                Cmd::Move(id, at, occ) => if let Some(p) = self.voices.iter_mut().find(|p| p.id == id) {
                    p.play.place = at;
                    p.play.occlusion = occ;
                    place_voice(&self.layout, &self.ears, p);
                },
                Cmd::Release(id) => if let Some(p) = self.voices.iter_mut().find(|p| p.id == id) { p.voice.release() },
                Cmd::Stop(id) => if let Some(p) = self.voices.iter_mut().find(|p| p.id == id) { p.stopping = true },
                Cmd::StopAll => for p in &mut self.voices { p.stopping = true },
                Cmd::Ears(e) => {
                    let old = std::mem::replace(&mut self.ears, e);
                    self.bin(Trash::Ears(old));
                    for p in self.voices.iter_mut() { place_voice(&self.layout, &self.ears, p); }
                }
                Cmd::Layout(l) => {
                    let ch = l.channels();
                    let old = std::mem::replace(&mut self.layout, l);
                    self.bin(Trash::Layout(old));
                    self.block = vec![0.0; BLOCK * ch];
                    self.wet = vec![0.0; BLOCK * ch];
                    self.reverb = Reverb::new(ch, self.sr);
                    self.pending.clear();
                    self.taken = 0;
                }
                Cmd::Volume(g) => self.master = g,
                Cmd::Room { decay, damping, level } => self.reverb.set(decay, damping, level),
            }
        }
    }

    fn dist(&self, p: &Playing) -> f32 {
        match p.play.place {
            Place::Head => 0.0,
            Place::At(x) => self.ears.iter().map(|e| ((x[0] - e.pos[0]).powi(2) + (x[1] - e.pos[1]).powi(2) + (x[2] - e.pos[2]).powi(2)).sqrt()).fold(f32::INFINITY, f32::min).min(1000.0),
        }
    }

    fn place(&self, p: &mut Playing) { place_voice(&self.layout, &self.ears, p); }

    /// Render one block into `self.block` (interleaved).
    fn block(&mut self) {
        let ch = self.layout.channels();
        self.block.iter_mut().for_each(|v| *v = 0.0);
        self.send.iter_mut().for_each(|v| *v = 0.0);
        let mut mono = [0.0f32; BLOCK];
        let sr = self.sr;
        let mut k = 0;
        while k < self.voices.len() {
            let p = &mut self.voices[k];
            if p.wait >= BLOCK { p.wait -= BLOCK; k += 1; continue; }
            let alive = p.voice.render(&mut mono) && !p.stopping;
            let skip = std::mem::take(&mut p.wait);
            let a = 1.0 - (-std::f32::consts::TAU * p.lp_hz / sr).exp();
            let g = p.play.gain;
            for (s, x) in mono.iter().enumerate().skip(skip) {
                p.lp += a * (x - p.lp);
                let v = if p.lp_hz >= 19_000.0 { *x } else { p.lp } * g;
                let t = s as f32 / BLOCK as f32;
                for c in 0..ch.min(MAX_SPEAKERS) {
                    let gc = p.prev[c] + (p.gains[c] - p.prev[c]) * t;
                    self.block[s * ch + c] += v * gc;
                }
                self.send[s] += v * p.play.reverb;
            }
            p.prev = p.gains;
            if alive { k += 1; } else { let v = self.voices.swap_remove(k); self.bin(Trash::Voice(v)); }
        }
        self.reverb.run(&self.send, &mut self.wet, ch);
        // Master and a peak limiter at -1 dBFS.
        let ceiling = 0.891;
        let release = (-1.0 / (0.1 * sr)).exp();
        for f in 0..BLOCK {
            let mut peak = 0.0f32;
            for c in 0..ch {
                let v = (self.block[f * ch + c] + self.wet[f * ch + c]) * self.master;
                self.block[f * ch + c] = v;
                peak = peak.max(v.abs());
            }
            // Release toward unity, but never above what keeps this frame under the ceiling.
            let released = 1.0 + (self.limiter - 1.0) * release;
            self.limiter = if peak > 0.0 { released.min(ceiling / peak) } else { released };
            for c in 0..ch { let v = self.block[f * ch + c] * self.limiter; self.block[f * ch + c] = v.clamp(-1.0, 1.0); }
        }
    }

    /// Fill an interleaved device buffer with `channels` channels. A device
    /// with more channels than the layout gets silence on the rest; with
    /// fewer, the extra layout channels fold into the first ones.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let ch = self.layout.channels();
        let frames = out.len() / channels.max(1);
        for f in 0..frames {
            if self.taken >= self.pending.len() / ch.max(1) {
                self.block();
                self.pending.clear();
                self.pending.extend_from_slice(&self.block);
                self.taken = 0;
            }
            let src = &self.pending[self.taken * ch..self.taken * ch + ch];
            let dst = &mut out[f * channels..f * channels + channels];
            dst.iter_mut().for_each(|v| *v = 0.0);
            for (c, v) in src.iter().enumerate() { dst[c % channels] += v; }
            if channels < ch { dst.iter_mut().for_each(|v| *v = v.clamp(-1.0, 1.0)); }
            self.taken += 1;
        }
    }

    /// Offline: `seconds` of interleaved output in the mixer's layout.
    pub fn render_seconds(&mut self, seconds: f32) -> Vec<f32> {
        let ch = self.layout.channels();
        let mut out = vec![0.0; (seconds * self.sr) as usize * ch];
        self.render(&mut out, ch);
        out
    }
}

fn place_voice(layout: &Layout, ears: &[Ear], p: &mut Playing) {
    let ch = layout.channels().min(MAX_SPEAKERS);
    let (lp, _) = place_into(layout, ears, p.play.place, &p.play.falloff, p.play.spread, p.play.lfe, p.play.occlusion, &mut p.gains[..ch]);
    p.lp_hz = lp;
}
