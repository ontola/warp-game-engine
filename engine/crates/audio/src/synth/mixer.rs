//! The live side: voices placed in the speaker layout, a room reverb and a
//! distance-independent hall ("big tail") with as many channels as the
//! layout, loudness-priority culling and a master chain (compressors and a
//! limiter). The game thread builds voices (allocation happens there) and
//! sends them; the audio thread only mixes and hands finished voices back to
//! be dropped off the audio thread.
use super::patch::{CompiledPatch, PatchSource, Voice};
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
    /// Playback rate: 2 plays an octave up and twice as fast (resampling).
    pub pitch: f32,
    /// Share sent to the room reverb.
    pub reverb: f32,
    /// Share sent to the hall, before distance (its level does not fall off).
    pub tail: f32,
    /// LFE send (0..1).
    pub lfe: f32,
    /// 0 clear .. 1 fully behind walls.
    pub occlusion: f32,
    /// Arrive after distance / 343 m/s.
    pub sound_delay: bool,
    /// Loudness priority (dB SPL): with culling on, a sound more than the
    /// window below the loudest recent one is not played at all. None: always.
    pub spl: Option<f32>,
    pub seed: u32,
}

impl Default for Play {
    fn default() -> Self {
        Play { place: Place::Head, falloff: Falloff::default(), spread: 0.0, gain: 1.0, pitch: 1.0, reverb: 0.0, tail: 0.0, lfe: 0.0, occlusion: 0.0, sound_delay: false, spl: None, seed: 1 }
    }
}

impl Play {
    pub fn at(p: [f32; 3]) -> Self { Play { place: Place::At(p), ..Default::default() } }
    pub fn db(mut self, db: f32) -> Self { self.gain *= 10f32.powf(db / 20.0); self }
    pub fn seed(mut self, s: u32) -> Self { self.seed = s; self }
    pub fn pitch(mut self, p: f32) -> Self { self.pitch = p; self }
    pub fn spl(mut self, spl: f32) -> Self { self.spl = Some(spl); self }
}

/// A playing sound, to move it, change its parameters or stop it.
#[derive(Clone)]
pub struct Handle { id: u32, patch: CompiledPatch }

impl Handle {
    pub fn id(&self) -> u32 { self.id }
}

/// A compressor on the master: above `threshold` (dB) the level rises
/// 1/`ratio` as fast; gain reduction recovers with `release` (s). Attack is
/// immediate (peak).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compressor { pub threshold: f32, pub ratio: f32, pub release: f32 }

/// The master chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Master {
    pub compressors: Vec<Compressor>,
    /// Peak limiter ceiling (dBFS).
    pub ceiling: f32,
}

impl Default for Master {
    fn default() -> Self { Master { compressors: Vec::new(), ceiling: -1.0 } }
}

impl Master {
    /// Growing Guns' Godot master bus: 3:1 at -3 dB (80 ms), 2:1 at -1 dB
    /// (400 ms), limiter at -0.3 dB.
    pub fn godot() -> Self {
        Master { compressors: vec![Compressor { threshold: -3.0, ratio: 3.0, release: 0.08 }, Compressor { threshold: -1.0, ratio: 2.0, release: 0.4 }], ceiling: -0.3 }
    }
}

/// The hall: a long, dark reverb whose level follows how loud things were
/// recently. Each placed sound with a tail send and an SPL lifts the
/// intensity to `(spl - 60) / 100`; it decays by `fall` per second, and the
/// hall's level runs from `quiet` to `loud` dB by its square root.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hall { pub decay: f32, pub damping: f32, pub predelay: f32, pub wet: f32, pub fall: f32, pub quiet: f32, pub loud: f32 }

impl Default for Hall {
    /// Growing Guns' BigTailReverb.
    fn default() -> Self { Hall { decay: 4.5, damping: 0.85, predelay: 0.06, wet: 0.55, fall: 0.22, quiet: -50.0, loud: -22.0 } }
}

/// Loudness-priority culling (Godot sfx.gd's HDR window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cull { pub window: f32, pub release: f32, pub floor: f32 }

impl Default for Cull {
    fn default() -> Self { Cull { window: 35.0, release: 110.0, floor: 60.0 } }
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
    /// Current and target low-pass (glides in log frequency).
    lp_hz: f32,
    lp_target: f32,
    shelf_lp: f32,
    shelf: f32,
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
    Hall(Hall),
    Lift(f32),
    Cull(Option<Cull>),
    Master(Master),
}

/// Game side. Cheap to clone.
#[derive(Clone)]
pub struct SoundSender { tx: Sender<Cmd>, next: Arc<AtomicU32>, sr: f32, loudest: Arc<AtomicU32> }

/// What the audio thread hands back to be dropped elsewhere.
pub struct Garbage(#[allow(dead_code)] Trash);
enum Trash { Voice(#[allow(dead_code)] Box<Playing>), Ears(#[allow(dead_code)] Vec<Ear>), Layout(#[allow(dead_code)] Layout), Master(#[allow(dead_code)] Master) }

/// Audio side: lives in the device callback (or a test).
pub struct Mixer {
    rx: Receiver<Cmd>,
    trash: Sender<Garbage>,
    trash_rx: Option<Receiver<Garbage>>,
    voices: Vec<Box<Playing>>,
    layout: Layout,
    ears: Vec<Ear>,
    reverb: Reverb,
    hall: Reverb,
    hall_cfg: Hall,
    intensity: f32,
    hall_pre: Vec<f32>,
    hall_pos: usize,
    cull: Option<Cull>,
    loudest: f32,
    loudest_out: Arc<AtomicU32>,
    master_cfg: Master,
    comp_gain: Vec<f32>,
    volume: f32,
    limiter: f32,
    sr: f32,
    /// Output frames rendered but not yet taken (interleaved).
    pending: Vec<f32>,
    taken: usize,
    block: Vec<f32>,
    wet: Vec<f32>,
    hall_wet: Vec<f32>,
    send: [f32; BLOCK],
    hall_send: [f32; BLOCK],
    /// Voices dropped because the pool was full.
    pub stolen: u64,
    /// Sounds not played because louder ones masked them.
    pub culled: u64,
    max_voices: usize,
}

/// A connected sender and mixer for `layout` at `sr`.
pub fn mixer(layout: Layout, max_voices: usize, sr: f32) -> (SoundSender, Mixer) {
    let (tx, rx) = channel();
    let (trash, trash_rx) = channel();
    let ch = layout.channels();
    let loudest = Arc::new(AtomicU32::new(60f32.to_bits()));
    let hall_cfg = Hall::default();
    let mut hall = Reverb::new(ch, sr);
    hall.set(hall_cfg.decay, hall_cfg.damping, hall_cfg.wet);
    (SoundSender { tx, next: Arc::new(AtomicU32::new(1)), sr, loudest: loudest.clone() },
     Mixer { rx, trash, trash_rx: Some(trash_rx), voices: Vec::with_capacity(max_voices), reverb: Reverb::new(ch, sr), hall, hall_cfg, intensity: 0.0,
        hall_pre: vec![0.0; (0.5 * sr) as usize], hall_pos: 0, cull: None, loudest: 60.0, loudest_out: loudest, master_cfg: Master::default(), comp_gain: Vec::new(),
        layout, ears: Vec::new(), volume: 1.0, limiter: 1.0, sr,
        pending: Vec::new(), taken: 0, block: vec![0.0; BLOCK * ch], wet: vec![0.0; BLOCK * ch], hall_wet: vec![0.0; BLOCK * ch], send: [0.0; BLOCK], hall_send: [0.0; BLOCK],
        stolen: 0, culled: 0, max_voices: max_voices.max(1) })
}

impl SoundSender {
    /// Start a patch. The voice is built here, on the calling thread.
    pub fn play(&self, patch: &Arc<impl PatchSource>, params: &[(&str, f32)], play: Play) -> Handle {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        // Resampling by rate: the voice runs at sr / pitch and plays at sr.
        let voice = patch.voice(params, play.seed, self.sr / play.pitch.clamp(0.05, 20.0));
        let patch = voice.patch.clone();
        let p = Box::new(Playing { id, voice, play, wait: 0, gains: [0.0; MAX_SPEAKERS], prev: [0.0; MAX_SPEAKERS], lp: 0.0, lp_hz: 20_000.0, lp_target: 20_000.0,
            shelf_lp: 0.0, shelf: 1.0, stopping: false });
        let _ = self.tx.send(Cmd::Play(p));
        Handle { id, patch }
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
    /// The room reverb: decay (s, to -60 dB), damping 0..1, return level.
    pub fn room(&self, decay: f32, damping: f32, level: f32) { let _ = self.tx.send(Cmd::Room { decay, damping, level }); }
    pub fn hall(&self, h: Hall) { let _ = self.tx.send(Cmd::Hall(h)); }
    /// Lift the hall's intensity to at least `i` (0..1).
    pub fn lift(&self, i: f32) { let _ = self.tx.send(Cmd::Lift(i)); }
    /// Loudness-priority culling on (Some) or off.
    pub fn cull(&self, c: Option<Cull>) { let _ = self.tx.send(Cmd::Cull(c)); }
    pub fn master(&self, m: Master) { let _ = self.tx.send(Cmd::Master(m)); }
    /// The loudest recent SPL (for ducking beds and music).
    pub fn loudest(&self) -> f32 { f32::from_bits(self.loudest.load(Ordering::Relaxed)) }
    pub fn sample_rate(&self) -> f32 { self.sr }
}

impl Mixer {
    pub fn channels(&self) -> usize { self.layout.channels() }
    pub fn active(&self) -> usize { self.voices.len() }
    pub fn sample_rate(&self) -> f32 { self.sr }
    /// The receiver for finished voices: drop them on a non-audio thread
    /// (taking it out; without it they are dropped here).
    pub fn take_trash(&mut self) -> Option<Receiver<Garbage>> { self.trash_rx.take() }

    fn bin(&self, g: Trash) { if self.trash_rx.is_none() { let _ = self.trash.send(Garbage(g)); } }

    fn drain(&mut self) {
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Play(mut p) => {
                    if let Some(spl) = p.play.spl {
                        if let Some(c) = self.cull { if spl < self.loudest - c.window { self.culled += 1; self.bin(Trash::Voice(p)); continue; } }
                        self.loudest = self.loudest.max(spl);
                        if p.play.tail > 0.0 && matches!(p.play.place, Place::At(_)) { self.intensity = self.intensity.max(((spl - 60.0) / 100.0).clamp(0.0, 1.0)); }
                    }
                    if self.voices.len() >= self.max_voices {
                        // Steal the quietest-placed voice.
                        let k = (0..self.voices.len()).min_by(|&a, &b| self.voices[a].play.gain.total_cmp(&self.voices[b].play.gain)).unwrap();
                        let v = self.voices.swap_remove(k);
                        self.bin(Trash::Voice(v));
                        self.stolen += 1;
                    }
                    place_voice(&self.layout, &self.ears, &mut p);
                    p.prev = p.gains;
                    p.lp_hz = p.lp_target;
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
                    self.hall_wet = vec![0.0; BLOCK * ch];
                    self.reverb = Reverb::new(ch, self.sr);
                    self.hall = Reverb::new(ch, self.sr);
                    self.hall.set(self.hall_cfg.decay, self.hall_cfg.damping, self.hall_cfg.wet);
                    self.pending.clear();
                    self.taken = 0;
                    for p in self.voices.iter_mut() { place_voice(&self.layout, &self.ears, p); }
                }
                Cmd::Volume(g) => self.volume = g,
                Cmd::Room { decay, damping, level } => self.reverb.set(decay, damping, level),
                Cmd::Hall(h) => { self.hall_cfg = h; self.hall.set(h.decay, h.damping, h.wet); }
                Cmd::Lift(i) => self.intensity = self.intensity.max(i.clamp(0.0, 1.0)),
                Cmd::Cull(c) => self.cull = c,
                Cmd::Master(m) => {
                    self.comp_gain = vec![1.0; m.compressors.len()];
                    let old = std::mem::replace(&mut self.master_cfg, m);
                    self.bin(Trash::Master(old));
                }
            }
        }
    }

    fn dist(&self, p: &Playing) -> f32 {
        match p.play.place {
            Place::Head | Place::Pan(_) => 0.0,
            Place::At(x) => self.ears.iter().map(|e| ((x[0] - e.pos[0]).powi(2) + (x[1] - e.pos[1]).powi(2) + (x[2] - e.pos[2]).powi(2)).sqrt()).fold(f32::INFINITY, f32::min).min(1000.0),
        }
    }

    /// Render one block into `self.block` (interleaved).
    fn block(&mut self) {
        let ch = self.layout.channels();
        self.block.iter_mut().for_each(|v| *v = 0.0);
        self.send.iter_mut().for_each(|v| *v = 0.0);
        self.hall_send.iter_mut().for_each(|v| *v = 0.0);
        let mut mono = [0.0f32; BLOCK];
        let sr = self.sr;
        let dt = BLOCK as f32 / sr;
        // Low-pass glide: Godot's raytraced muffle moves 1 - e^(-5 dt) in log2 Hz.
        let glide = 1.0 - (-5.0 * dt).exp();
        let shelf_a = 1.0 - (-std::f32::consts::TAU * 8000.0 / sr).exp();
        let mut k = 0;
        while k < self.voices.len() {
            let p = &mut self.voices[k];
            if p.wait >= BLOCK { p.wait -= BLOCK; k += 1; continue; }
            let alive = p.voice.render(&mut mono) && !p.stopping;
            let skip = std::mem::take(&mut p.wait);
            if (p.lp_hz - p.lp_target).abs() > 1.0 { p.lp_hz = 2f32.powf(p.lp_hz.log2() + (p.lp_target.log2() - p.lp_hz.log2()) * glide); }
            let filtered = p.lp_hz < 19_000.0;
            let a = 1.0 - (-std::f32::consts::TAU * p.lp_hz / sr).exp();
            let (g, rv, tail, shelf) = (p.play.gain, p.play.reverb, p.play.tail, p.shelf);
            // The room send follows the voice's loudest speaker gain (distance).
            let near = p.gains[..ch.min(MAX_SPEAKERS)].iter().fold(0.0f32, |m, g| m.max(*g));
            for (s, x) in mono.iter().enumerate().skip(skip) {
                let pre = *x * g;
                let mut v = *x;
                if filtered { p.lp += a * (v - p.lp); v = p.lp; }
                if shelf < 0.999 { p.shelf_lp += shelf_a * (v - p.shelf_lp); v = p.shelf_lp + shelf * (v - p.shelf_lp); }
                let v = v * g;
                let t = s as f32 / BLOCK as f32;
                for c in 0..ch.min(MAX_SPEAKERS) {
                    let gc = p.prev[c] + (p.gains[c] - p.prev[c]) * t;
                    self.block[s * ch + c] += v * gc;
                }
                self.send[s] += v * rv * near;
                self.hall_send[s] += pre * tail;
            }
            p.prev = p.gains;
            if alive { k += 1; } else { let v = self.voices.swap_remove(k); self.bin(Trash::Voice(v)); }
        }
        self.reverb.run(&self.send, &mut self.wet, ch);
        // Hall: predelay, then the reverb at the intensity's level.
        let level = 10f32.powf((self.hall_cfg.quiet + (self.hall_cfg.loud - self.hall_cfg.quiet) * self.intensity.sqrt()) / 20.0);
        let n = self.hall_pre.len();
        let pre = ((self.hall_cfg.predelay * sr) as usize).min(n - 1);
        for s in 0..BLOCK {
            self.hall_pre[self.hall_pos] = self.hall_send[s];
            self.hall_send[s] = self.hall_pre[(self.hall_pos + n - pre) % n] * level;
            self.hall_pos = (self.hall_pos + 1) % n;
        }
        self.hall.run(&self.hall_send, &mut self.hall_wet, ch);
        self.intensity *= (-self.hall_cfg.fall * dt).exp();
        if self.intensity < 0.002 { self.intensity = 0.0; }
        let c = self.cull.unwrap_or_default();
        self.loudest = (self.loudest - c.release * dt).max(c.floor);
        self.loudest_out.store(self.loudest.to_bits(), Ordering::Relaxed);
        // Master: compressors (instant attack, exponential release), then a
        // peak limiter at the ceiling.
        let ceiling = 10f32.powf(self.master_cfg.ceiling / 20.0);
        let release = (-1.0 / (0.1 * sr)).exp();
        if self.comp_gain.len() != self.master_cfg.compressors.len() { self.comp_gain = vec![1.0; self.master_cfg.compressors.len()]; }
        for f in 0..BLOCK {
            let mut peak = 0.0f32;
            for c in 0..ch {
                let v = (self.block[f * ch + c] + self.wet[f * ch + c] + self.hall_wet[f * ch + c]) * self.volume;
                self.block[f * ch + c] = v;
                peak = peak.max(v.abs());
            }
            let mut gain = 1.0f32;
            for (comp, cg) in self.master_cfg.compressors.iter().zip(self.comp_gain.iter_mut()) {
                let level = 20.0 * (peak * gain).max(1e-9).log10();
                let over = level - comp.threshold;
                let target = if over > 0.0 { 10f32.powf(-over * (1.0 - 1.0 / comp.ratio.max(1.0)) / 20.0) } else { 1.0 };
                let rel = (-1.0 / (comp.release.max(1e-3) * sr)).exp();
                *cg = if target < *cg { target } else { target + (*cg - target) * rel };
                gain *= *cg;
            }
            let peak = peak * gain;
            // Release toward unity, never above what keeps this frame under the ceiling.
            let released = 1.0 + (self.limiter - 1.0) * release;
            self.limiter = if peak > 0.0 { released.min(ceiling / peak) } else { released };
            let g = gain * self.limiter;
            for c in 0..ch { let v = self.block[f * ch + c] * g; self.block[f * ch + c] = v.clamp(-1.0, 1.0); }
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

impl crate::Source for Mixer {
    fn render(&mut self, out: &mut [f32], channels: usize) { Mixer::render(self, out, channels) }
}

fn place_voice(layout: &Layout, ears: &[Ear], p: &mut Playing) {
    let ch = layout.channels().min(MAX_SPEAKERS);
    let (lp, d) = place_into(layout, ears, p.play.place, &p.play.falloff, p.play.spread, p.play.lfe, p.play.occlusion, &mut p.gains[..ch]);
    p.lp_target = lp;
    p.shelf = 10f32.powf(p.play.falloff.shelf_db(d) / 20.0);
}
