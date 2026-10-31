//! The colosseum crowd's sound, ported from the crowd_sound addon
//! (joepio/godot-crowd-sound) that Growing Guns uses: recorded CC0 beds and
//! reactions (cheer and panic loops, roars, goal eruptions, "ooh"s, screams,
//! chants, the kick-off surge) plus a synthesised murmur (a small crowd of
//! formant talkers through a stadium reverb) and applause.
//!
//! Two values drive the mix: enthusiasm (exciting play) and fear (the crowd
//! itself in danger). The beds crossfade on them; reactions fire on game
//! events. The same two values drive the crowd's body language
//! (`render::crowd`).
use audio::sfx::{spatial, Clip, Listener, Play, SfxSender};
use std::f32::consts::{PI, TAU};
use std::path::{Path, PathBuf};

const RATE: f32 = 22_050.0;
const LOOP_SECONDS: f32 = 6.0;
const SEAM_SECONDS: f32 = 0.4;
const ENTHUSIASM_DECAY: f32 = 0.38;
const FEAR_DECAY: f32 = 0.20;
const LAYER_DYN_RANGE_DB: f32 = 14.0;

// Mix (dB at full layer gain), as in the addon.
const MURMUR_DB: f32 = -22.0;
const CHEER_DB: f32 = -10.0;
const PANIC_DB: f32 = -9.0;
const CLAPS_DB: f32 = -12.0;
const REACTION_DB: f32 = -8.0;
const CHANT_DB: f32 = -10.0;

const BED_MURMUR: u32 = 100;
const BED_CHEER: u32 = 101;
const BED_PANIC: u32 = 102;
const BED_CLAPS: u32 = 103;
const BED_CHANT: u32 = 104;

/// The recorded set (samples/ in the addon).
#[derive(Clone)]
pub struct Samples { cheer: Clip, panic: Clip, surge: Clip, roar: Vec<Clip>, goal: Vec<Clip>, ooh: Vec<Clip>, scream: Vec<Clip>, chant: Vec<Clip> }

/// The addon's sample folder: `--crowd DIR`, `GG_CROWD`, a `crowd` folder
/// inside the sound folder, next to the Godot project's audio folder,
/// `audio/crowd` beside the executable, or a Godot checkout found by
/// `sounds::godot_checkout`.
pub fn find_dir(arg: Option<&str>, sounds: Option<&Path>) -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = [arg.map(PathBuf::from), std::env::var("GG_CROWD").ok().map(PathBuf::from)].into_iter().flatten().collect();
    if let Some(s) = sounds {
        c.push(s.join("crowd"));
        c.push(s.join("../../addons/crowd_sound/samples"));
    }
    c.extend(crate::sounds::beside_exe("audio/crowd"));
    c.extend(crate::sounds::godot_checkout("addons/crowd_sound/samples"));
    c.into_iter().find(|p| p.join("cheer_loop.ogg").exists())
}

impl Samples {
    fn silent() -> Self {
        let c = Clip(Vec::new().into());
        Samples { cheer: c.clone(), panic: c.clone(), surge: c, roar: Vec::new(), goal: Vec::new(), ooh: Vec::new(), scream: Vec::new(), chant: Vec::new() }
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let one = |n: &str| audio::sfx::load_clip(&dir.join(format!("{n}.ogg")));
        let set = |p: &str| (1..10).map_while(|i| audio::sfx::load_clip(&dir.join(format!("{p}_{i}.ogg"))).ok()).collect::<Vec<_>>();
        Ok(Samples { cheer: one("cheer_loop")?, panic: one("panic_loop")?, surge: one("surge")?, roar: set("roar"), goal: set("goal"), ooh: set("ooh"), scream: set("scream"), chant: set("chant") })
    }
}

fn db(x: f32) -> f32 { 10f32.powf(x / 20.0) }

pub struct CrowdSound {
    /// Without a speaker or the recordings the mood still runs (it drives
    /// the crowd's body language); only the sound is skipped.
    tx: Option<SfxSender>,
    s: Samples,
    pub enthusiasm: f32,
    pub fear: f32,
    pub active: bool,
    gains: [f32; 4],
    roar_cd: f32,
    ooh_cd: f32,
    scream_cd: f32,
    celebrate_hold: f32,
    chant_cd: f32,
    chant_left: f32,
    chant_fading: bool,
    chant_gain: f32,
    chant_force: f32,
    chant_bag: Vec<usize>,
    rng: u64,
    /// Loudness of the game's own sound above "loud" (dB), ducks the beds.
    pub loudness_db: f32,
}

impl CrowdSound {
    /// Replays share the already decoded clips and existing ambience beds.
    pub(crate) fn audio_bank(&self) -> Option<(SfxSender, Samples)> {
        self.tx.as_ref().map(|tx| (tx.clone(), self.s.clone()))
    }
    pub(crate) fn replay_audio(&mut self, bank: Option<&(SfxSender, Samples)>, audible: bool) {
        self.tx = bank.filter(|_| audible).map(|(tx, _)| tx.clone());
        if let Some((_, samples)) = bank { self.s = samples.clone(); }
    }

    /// Start the beds (silent until the mood lifts them). Synthesis takes a
    /// moment. `None`: mood only.
    pub fn new(audio: Option<(SfxSender, Samples)>) -> Self {
        let (tx, s) = match audio {
            Some((tx, s)) => {
                let murmur = finish_loop(reverb(&murmur((LOOP_SECONDS + SEAM_SECONDS) * RATE), 0.36, 0.88));
                let claps = finish_loop(reverb(&applause((LOOP_SECONDS + SEAM_SECONDS) * RATE), 0.1, 0.6));
                let silent = Play { gain: 0.0, ..Default::default() };
                tx.start_loop(BED_MURMUR, &to_clip(&murmur), silent);
                tx.start_loop(BED_CHEER, &s.cheer, silent);
                tx.start_loop(BED_PANIC, &s.panic, silent);
                tx.start_loop(BED_CLAPS, &to_clip(&claps), silent);
                (Some(tx), s)
            }
            None => (None, Samples::silent()),
        };
        CrowdSound { tx, s, enthusiasm: 0.0, fear: 0.0, active: true, gains: [0.0; 4], roar_cd: 0.0, ooh_cd: 0.0, scream_cd: 0.0, celebrate_hold: 0.0,
            chant_cd: 10.0, chant_left: 0.0, chant_fading: false, chant_gain: 0.0, chant_force: 0.0, chant_bag: Vec::new(), rng: 0x1234_5678_9ABC, loudness_db: 0.0 }
    }

    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
    fn pick(&mut self, set: usize) -> Option<Clip> {
        let n = [self.s.roar.len(), self.s.goal.len(), self.s.ooh.len(), self.s.scream.len()][set];
        if n == 0 { return None; }
        let k = (self.f() * n as f32) as usize % n;
        Some([&self.s.roar, &self.s.goal, &self.s.ooh, &self.s.scream][set][k].clone())
    }
    fn oneshot(&mut self, clip: &Clip, dbv: f32, at: Option<([f32; 3], &[Listener])>, unit: f32) {
        let pitch = self.range(0.94, 1.06);
        let play = match at { Some((p, ears)) => spatial(p, ears, unit), None => Play::default() };
        if let Some(tx) = &self.tx { tx.play(clip, Play { gain: play.gain * db(dbv), pitch, ..play }); }
    }

    /// Round start: anticipation surge, fear forgotten.
    pub fn kickoff(&mut self) {
        if !self.active { return; }
        self.fear = 0.0;
        self.enthusiasm = self.enthusiasm.max(0.85);
        self.gains[1] = self.gains[1].max(0.6);
        self.roar_cd = 2.0;
        let surge = self.s.surge.clone();
        if !surge.0.is_empty() { self.oneshot(&surge, REACTION_DB, None, 0.0); }
    }

    /// A round won: the loudest the crowd gets, then a victory chant.
    pub fn celebrate(&mut self) {
        if !self.active { return; }
        self.fear = 0.0;
        self.enthusiasm = 1.0;
        self.gains[1] = 1.0;
        self.celebrate_hold = 4.0;
        self.roar_cd = 2.5;
        self.chant_force = 1.8;
        if let Some(c) = self.pick(1) { self.oneshot(&c, REACTION_DB + 4.0, None, 0.0); }
    }

    /// Someone got hurt: a bump and sometimes an "ooh".
    pub fn hit(&mut self) {
        if !self.active { return; }
        self.enthusiasm = (self.enthusiasm + 0.18).clamp(0.0, 1.0);
        if self.ooh_cd <= 0.0 && self.f() < 0.4
            && let Some(c) = self.pick(2) {
                self.ooh_cd = 1.6;
                let e = self.enthusiasm;
                self.oneshot(&c, REACTION_DB - 6.0 + e * 3.0, None, 0.0);
            }
    }

    /// A kill: the bowl roars.
    pub fn roar(&mut self) {
        if !self.active { return; }
        self.enthusiasm = (self.enthusiasm + 0.6).clamp(0.0, 1.0);
        if self.roar_cd <= 0.0
            && let Some(c) = self.pick(0) {
                self.roar_cd = 1.4;
                let e = self.enthusiasm;
                self.oneshot(&c, REACTION_DB + e * 3.0, None, 0.0);
            }
    }

    pub fn excite(&mut self, amount: f32) { if self.active { self.enthusiasm = (self.enthusiasm + amount).clamp(0.0, 1.0); } }

    /// The crowd in danger: fear, cheering collapses, a scream from `at`.
    pub fn scare(&mut self, amount: f32, at: [f32; 3], ears: &[Listener], intensity: f32, cheer_keep: f32) {
        if !self.active { return; }
        self.fear = (self.fear + amount).clamp(0.0, 1.0);
        self.enthusiasm *= cheer_keep;
        if self.scream_cd <= 0.0
            && let Some(c) = self.pick(3) {
                self.scream_cd = 0.55;
                self.oneshot(&c, REACTION_DB - 4.0 + intensity * 5.0, Some((at, ears)), 35.0);
            }
    }

    /// Per frame (or tick): decay, cooldowns, chants and the bed mix.
    pub fn update(&mut self, dt: f32, chant_at: Option<([f32; 3], &[Listener])>) {
        let floor: f32 = if self.active { 0.2 } else { 0.0 };
        self.enthusiasm = floor.max(self.enthusiasm - (self.enthusiasm - floor) * ENTHUSIASM_DECAY * dt);
        self.fear = (self.fear - self.fear * FEAR_DECAY * dt).max(0.0);
        if self.celebrate_hold > 0.0 { self.celebrate_hold -= dt; self.enthusiasm = 1.0; }
        self.roar_cd = (self.roar_cd - dt).max(0.0);
        self.ooh_cd = (self.ooh_cd - dt).max(0.0);
        self.scream_cd = (self.scream_cd - dt).max(0.0);
        let duck = if self.celebrate_hold <= 0.0 { (self.loudness_db * 0.4).clamp(0.0, 10.0) } else { 0.0 };
        let chanting = self.chant_left > 0.0 && !self.chant_fading;
        self.update_chant(dt, duck, chant_at);
        let (e, f) = (self.enthusiasm, self.fear);
        let targets = if self.active {
            [(0.55 + 0.3 * e - 0.25 * f).clamp(0.15, 0.9), e * (1.0 - f * 0.75), f, ((e - 0.6) / 0.4).clamp(0.0, 1.0) * (1.0 - f * 0.75)]
        } else { [0.0; 4] };
        let (up, down) = ((dt * 5.0).min(1.0), (dt * 1.6).min(1.0));
        for (g, t) in self.gains.iter_mut().zip(targets) { *g += (t - *g) * if t > *g { up } else { down }; }
        let cd = if chanting { 2.0 } else { 0.0 };
        let base = [MURMUR_DB - duck - cd * 0.3, CHEER_DB - duck - cd, PANIC_DB - duck * 0.5, CLAPS_DB - duck - cd * 0.5];
        for (k, id) in [BED_MURMUR, BED_CHEER, BED_PANIC, BED_CLAPS].into_iter().enumerate() {
            let g = self.gains[k];
            if let Some(tx) = &self.tx { tx.loop_gain(id, if g < 0.005 { 0.0 } else { db(base[k] - (1.0 - g.clamp(0.0, 1.0)) * LAYER_DYN_RANGE_DB) }); }
        }
    }

    fn update_chant(&mut self, dt: f32, duck: f32, at: Option<([f32; 3], &[Listener])>) {
        if self.chant_left > 0.0 {
            self.chant_left -= dt;
            if self.fear > 0.5 || !self.active { self.chant_fading = true; }
            let target = if self.chant_fading { 0.0 } else { db(CHANT_DB - (self.loudness_db * 0.5).clamp(0.0, 14.0) - self.fear * 10.0) };
            // Fade out at 50 dB/s, in by a lerp, as the addon does.
            self.chant_gain = if self.chant_fading { (self.chant_gain * db(-50.0 * dt)).max(0.0) } else { self.chant_gain + (target - self.chant_gain) * (dt * 6.0).min(1.0) };
            if self.chant_left <= 0.0 || self.chant_gain < db(-55.0) && self.chant_fading {
                if let Some(tx) = &self.tx { tx.stop_loop(BED_CHANT); }
                self.chant_left = 0.0;
                self.chant_cd = if self.chant_fading { self.range(16.0, 30.0) } else { self.range(14.0, 26.0) };
                self.chant_fading = false;
            } else {
                if let Some(tx) = &self.tx { tx.loop_gain(BED_CHANT, self.chant_gain); }
            }
            let _ = duck;
            return;
        }
        if !self.active || self.s.chant.is_empty() { return; }
        if self.chant_force > 0.0 {
            self.chant_force -= dt;
            if self.chant_force <= 0.0 { self.start_chant(at); }
            return;
        }
        self.chant_cd -= dt;
        if self.chant_cd <= 0.0 && self.enthusiasm > 0.4 && self.fear < 0.25 && self.f() < dt / 4.0 { self.start_chant(at); }
    }

    fn start_chant(&mut self, at: Option<([f32; 3], &[Listener])>) {
        if self.chant_bag.is_empty() {
            self.chant_bag = (0..self.s.chant.len()).collect();
            for i in (1..self.chant_bag.len()).rev() { let j = (self.f() * (i + 1) as f32) as usize % (i + 1); self.chant_bag.swap(i, j); }
        }
        let Some(k) = self.chant_bag.pop() else { return };
        let clip = self.s.chant[k].clone();
        let mut play = at.map_or(Play::default(), |(p, ears)| spatial(p, ears, 26.0));
        play.pitch = self.range(0.97, 1.03);
        self.chant_gain = db(CHANT_DB - 8.0) * play.gain;
        play.gain = self.chant_gain;
        if let Some(tx) = &self.tx { tx.start_loop(BED_CHANT, &clip, play); }
        self.chant_left = clip.seconds() / play.pitch;
        self.chant_cd = 999.0;
    }
}

// -------------------- synthesis (boot time) --------------------

fn to_clip(x: &[f32]) -> Clip {
    // 22.05 kHz synth to the mixer rate.
    let to = audio::SAMPLE_RATE;
    let n = (x.len() as f64 * to as f64 / RATE as f64) as usize;
    let step = RATE as f64 / to as f64;
    Clip((0..n).map(|k| {
        let p = k as f64 * step;
        let i = p as usize;
        let t = (p - i as f64) as f32;
        x[i.min(x.len() - 1)] + (x[(i + 1).min(x.len() - 1)] - x[i.min(x.len() - 1)]) * t
    }).collect::<Vec<_>>().into())
}

fn normalize(x: &mut [f32], peak: f32) {
    let p = x.iter().fold(1e-4f32, |m, v| m.max(v.abs()));
    for v in x { *v *= peak / p; }
}

/// Normalise, then blend the tail into the head for a seamless loop.
fn finish_loop(mut x: Vec<f32>) -> Vec<f32> {
    normalize(&mut x, 0.7);
    let fade = (SEAM_SECONDS * RATE) as usize;
    let body = x.len() - fade;
    for k in 0..fade {
        let w = k as f32 / fade as f32;
        x[k] = x[k] * w + x[body + k] * (1.0 - w);
    }
    x.truncate(body);
    x
}

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
    fn below(&mut self, n: usize) -> usize { (self.f() * n as f32) as usize % n.max(1) }
}

const VOWELS: [[f32; 3]; 7] = [[750., 1220., 2600.], [480., 1900., 2550.], [300., 2250., 2950.], [480., 880., 2550.], [330., 900., 2350.], [650., 1700., 2500.], [600., 1150., 2450.]];

/// Idle murmur: 6 synthesised talkers, each heard three times (resampled,
/// shifted, panned, the far ones low-passed) = 18 voices.
fn murmur(n: f32) -> Vec<f32> {
    let n = n as usize;
    let mut out = vec![0.0f32; n];
    let mut rng = Rng(0x9E37_79B9_0000_0001);
    for k in 0..6 {
        let voice = talker(n, &mut rng);
        for copy in 0..3 {
            let near = copy == 0 && k < 3;
            let g = if near { rng.range(0.6, 1.0) } else { rng.range(0.15, 0.5) * 0.247 };
            let a = (-TAU * 800.0 * rng.range(0.7, 1.3) / RATE).exp();
            let rate = if copy == 0 { 1.0 } else { rng.range(0.9, 1.12) };
            let mut pos = if copy == 0 { 0.0 } else { rng.f() * n as f32 };
            let mut lp = 0.0;
            for o in out.iter_mut() {
                let s = if copy == 0 { voice[(pos as usize).min(n - 1)] } else {
                    let j = pos as usize % n;
                    let fr = pos - pos.floor();
                    voice[j] + (voice[(j + 1) % n] - voice[j]) * fr
                };
                pos += rate;
                if pos >= n as f32 { pos -= n as f32; }
                let s = if near { s } else { lp = s + (lp - s) * a; lp };
                *o += s * g;
            }
        }
    }
    out
}

/// One talker: syllables planned first (consonant, vowel, pitch with
/// phrase declination), then a glottal pulse train through three moving
/// vowel formants plus consonant hiss.
fn talker(n: usize, rng: &mut Rng) -> Vec<f32> {
    let f0 = if rng.f() < 0.5 { rng.range(114.0 * 0.8, 114.0 * 1.25) } else { rng.range(210.0 * 0.85, 210.0 * 1.2) };
    let fscale = if f0 < 160.0 { 1.0 } else { 1.15 };
    struct Syl { start: usize, cend: usize, end: usize, pitch: f32, from: [f32; 3], to: [f32; 3], kind: u8, fric: f32 }
    let mut plan = Vec::new();
    let mut t = 0usize;
    let mut cur = [600.0 * fscale, 1150.0 * fscale, 2450.0 * fscale];
    while t < n {
        let phrase = (rng.range(0.6, 1.03) * RATE) as usize;
        let (p0, p_end) = (t, (t + phrase).min(n));
        while t < p_end {
            let cl = (rng.range(0.02, 0.06) * RATE) as usize;
            let vl = (rng.range(0.5, 1.5) * 0.112 * RATE) as usize;
            let len = (cl + vl).min(n - t);
            if len == 0 { break; }
            let prog = (t - p0) as f32 / phrase as f32;
            let pitch = f0 * (1.0 + 0.245 - 0.245 * 1.3 * prog) * (1.0 + rng.range(-0.164, 0.164));
            let v = VOWELS[rng.below(VOWELS.len())];
            let tgt = [v[0] * fscale, v[1] * fscale, v[2] * fscale];
            plan.push(Syl { start: t, cend: t + cl.min(len), end: t + len, pitch, from: cur, to: tgt, kind: rng.below(3) as u8, fric: rng.range(0.3, 1.0) });
            cur = tgt;
            t += len;
        }
        t += (-(rng.f().max(1e-6)).ln() * 0.228 * RATE) as usize;
    }
    let bw = 139.0;
    let (r1, r2, r3) = ((-PI * bw / RATE).exp(), (-PI * bw * 1.4 / RATE).exp(), (-PI * bw * 2.0 / RATE).exp());
    let (mut c1, mut c2, mut c3) = (0.0f32, 0.0f32, 0.0f32);
    let mut y = [0.0f32; 6];
    let k_glide = 1.0 - (-TAU * 3.57 / RATE).exp();
    let k_tilt = 1.0 - (-TAU * 892.0 / RATE).exp();
    let a_hp = (-TAU * 3000.0 / RATE).exp();
    let k_fenv = 1.0 - (-TAU * 200.0 / RATE).exp();
    let (mut ps1, mut ps2, mut ph, mut prev_g, mut tilt, mut hp_lp, mut fenv) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let oq = 0.464;
    let (mut vpeak, mut fpeak) = (1e-4f32, 1e-4f32);
    let mut voice = vec![0.0f32; n];
    let mut fric = vec![0.0f32; n];
    let burst = (0.012 * RATE) as usize;
    let u = 1.732_050_8f32;
    let mut si = 0usize;
    for i in 0..n {
        while si < plan.len() && i >= plan[si].end { si += 1; }
        let syl = plan.get(si);
        let (mut amp, mut fr_target, mut pitch_target) = (0.0, 0.0, f0);
        if let Some(s) = syl.filter(|s| i >= s.start) {
            pitch_target = s.pitch;
            let vlen = (s.end - s.cend).max(1) as f32;
            if i < s.cend {
                match s.kind { 0 => fr_target = s.fric, 1 => fr_target = if i + burst >= s.cend { 1.0 } else { 0.0 }, _ => amp = 0.3 }
            } else {
                amp = (PI * (i - s.cend) as f32 / vlen).sin().max(0.0).powf(0.914);
            }
            if i % 64 == 0 {
                let fv = if i < s.cend { s.from.map(|v| v * 0.8) } else {
                    let w = (i - s.cend) as f32 / vlen;
                    [0, 1, 2].map(|k| s.from[k] * 0.85 + (s.to[k] - s.from[k] * 0.85) * w)
                };
                c1 = 2.0 * r1 * (TAU * fv[0] / RATE).cos();
                c2 = 2.0 * r2 * (TAU * fv[1] / RATE).cos();
                c3 = 2.0 * r3 * (TAU * fv[2] / RATE).cos();
            }
        }
        ps1 += (pitch_target - f0 - ps1) * k_glide;
        ps2 += (ps1 - ps2) * k_glide;
        let pitch = (f0 + ps2) * (1.0 + 0.021 * u * (2.0 * rng.f() - 1.0));
        ph += pitch / RATE;
        if ph >= 1.0 { ph -= 1.0; }
        let g = if ph < oq { 0.5 - 0.5 * (PI * ph / oq).cos() } else { (PI * (ph - oq) / (2.0 * (1.0 - oq))).cos().max(0.0) };
        tilt += ((g - prev_g) * 30.0 - tilt) * k_tilt;
        prev_g = g;
        let w = u * (2.0 * rng.f() - 1.0);
        let src = (tilt + w * 0.178) * amp;
        let o1 = (1.0 - r1) * src + c1 * y[0] - r1 * r1 * y[1];
        y[1] = y[0]; y[0] = o1;
        let o2 = (1.0 - r2) * src + c2 * y[2] - r2 * r2 * y[3];
        y[3] = y[2]; y[2] = o2;
        let o3 = (1.0 - r3) * src + c3 * y[4] - r3 * r3 * y[5];
        y[5] = y[4]; y[4] = o3;
        let v = o1 + 0.6 * o2 + 0.3 * o3;
        voice[i] = v;
        vpeak = vpeak.max(v.abs());
        hp_lp = w + (hp_lp - w) * a_hp;
        fenv += (fr_target - fenv) * k_fenv;
        let f = (w - hp_lp) * fenv;
        fric[i] = f;
        fpeak = fpeak.max(f.abs());
    }
    let fg = vpeak * 0.022 / fpeak;
    for (v, f) in voice.iter_mut().zip(&fric) { *v += f * fg; }
    voice
}

/// Small Schroeder reverb (four damped combs, two allpasses): the stands' echo.
fn reverb(x: &[f32], wet: f32, size: f32) -> Vec<f32> {
    let lens: Vec<usize> = [1557, 1617, 1491, 1422, 556, 225].iter().map(|&b| ((b as f32 * size) as usize).max(1)).collect();
    let mut lines: Vec<Vec<f32>> = lens.iter().map(|&l| vec![0.0; l]).collect();
    let mut p = [0usize; 6];
    let mut s = [0.0f32; 4];
    let (fb, damp) = (0.84, 0.3);
    let mut wet_buf = vec![0.0f32; x.len()];
    for (i, &v) in x.iter().enumerate() {
        let inp = v * 0.015;
        let mut acc = 0.0;
        for c in 0..4 {
            let y = lines[c][p[c]];
            s[c] = y * (1.0 - damp) + s[c] * damp;
            lines[c][p[c]] = inp + s[c] * fb;
            p[c] = (p[c] + 1) % lens[c];
            acc += y;
        }
        for a in 4..6 {
            let b = lines[a][p[a]];
            lines[a][p[a]] = acc + b * 0.5;
            p[a] = (p[a] + 1) % lens[a];
            acc = b - acc;
        }
        wet_buf[i] = acc;
    }
    let pd = x.iter().fold(1e-4f32, |m, v| m.max(v.abs()));
    let pw = wet_buf.iter().fold(1e-4f32, |m, v| m.max(v.abs()));
    let ws = pd / pw * wet;
    x.iter().zip(&wet_buf).map(|(d, w)| d * (1.0 - wet) + w * ws).collect()
}

/// Applause: twelve clappers at their own steady rates, each clap a short
/// noise burst through the clapper's band-pass.
fn applause(n: f32) -> Vec<f32> {
    let n = n as usize;
    let mut out = vec![0.0f32; n];
    let mut rng = Rng(55005 * 0x9E37_79B9 + 1);
    for _ in 0..12 {
        let rate = rng.range(2.25, 4.75);
        let f = rng.range(1814.0 * 0.7, 1814.0 * 1.3);
        let r = (-PI * 491.0 / RATE).exp();
        let c = 2.0 * r * (TAU * f / RATE).cos();
        let dec = (-1.0 / (0.0036 * RATE)).exp();
        let g = rng.range(0.3, 1.0).powf(1.5);
        let mut next = (rng.f() / rate * RATE) as usize;
        let (mut env, mut ya, mut yb) = (0.0f32, 0.0f32, 0.0f32);
        for (i, o) in out.iter_mut().enumerate() {
            if i == next {
                env += rng.range(0.5, 1.0);
                next += (RATE / rate * rng.range(0.92, 1.08)) as usize;
            }
            env *= dec;
            if env < 0.0005 && ya.abs() < 0.0005 { ya = 0.0; yb = 0.0; continue; }
            let x = 1.732_050_8 * (2.0 * rng.f() - 1.0) * env;
            let y = (1.0 - r) * x + c * ya - r * r * yb;
            yb = ya;
            ya = y;
            *o += (y + x * 0.077) * g;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesised_beds_loop_cleanly_and_are_not_silent() {
        for bed in [finish_loop(reverb(&murmur(3.0 * RATE), 0.36, 0.88)), finish_loop(reverb(&applause(3.0 * RATE), 0.1, 0.6))] {
            let rms = (bed.iter().map(|v| v * v).sum::<f32>() / bed.len() as f32).sqrt();
            assert!(rms > 0.02, "{rms}");
            assert!(bed.iter().all(|v| v.is_finite() && v.abs() <= 0.71));
            // The seam: end and start meet without a jump.
            assert!((bed[bed.len() - 1] - bed[0]).abs() < 0.2);
        }
    }

    #[test]
    fn mood_moves_the_bed_mix() {
        let dir = find_dir(None, None);
        let Some(dir) = dir else { return };
        let (tx, mut mix) = audio::sfx::sfx(32, audio::SAMPLE_RATE);
        let mut c = CrowdSound::new(Some((tx, Samples::load(&dir).unwrap())));
        for _ in 0..60 { c.update(1.0 / 60.0, None); }
        let calm = mix.render_stereo(0.5);
        c.celebrate();
        for _ in 0..60 { c.update(1.0 / 60.0, None); }
        let loud = mix.render_stereo(0.5);
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!(rms(&loud) > rms(&calm) * 1.5, "{} {}", rms(&calm), rms(&loud));
    }
}
