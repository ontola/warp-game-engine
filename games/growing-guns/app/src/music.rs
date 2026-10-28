//! The Growing Guns soundtrack, ported from Godot's `procedural_music.gd`.
//!
//! Despite the Godot file's name, the soundtrack is not synthesised: it is
//! two loop sets of three pre-recorded stems each (`5-low/mid/high.wav`,
//! `6-low/mid/high.wav`; 44.1 kHz stereo), all six playing in lock-step,
//! with the audible mix chosen per stem from an energy level of 0..=3:
//!
//! * 1 = round started, nobody shot yet: the low stem;
//! * 2 = a bullet passed close to someone: the mid stem;
//! * 3 = a player was hit, or two remain and one is at half health: high.
//!
//! Energy changes land on the next bar (2 s at 120 BPM) unless immediate.
//! `rebake` switches to the other loop set from its downbeat; the old set
//! holds its mix and fades out over 3 s, muffled. During card pick
//! (`set_muffle(true)`) the current set is low-passed down to 700 Hz over
//! 1 s and the other set's low stem creeps in over 3 s as a preview.
//! Gunshots and explosions duck the music by `-18 * duck` dB; the duck
//! decays linearly at 2.5 per second.
//!
//! Each loop set has its own bus as in Godot: low-pass (Godot's RBJ filter
//! at Q 0.5), compressor (-9 dB, 4:1, 2.5 ms / 140 ms) and limiter
//! (-2 dB threshold, -0.5 dB ceiling, 1.5 dB soft clip), all ported from
//! Godot's effect code. Stems play with linear interpolation like Godot's
//! `AudioStreamWAV`, and volume changes ramp over one Godot mix buffer.
//!
//! The control methods mirror Godot's `ProceduralMusic` (`set_energy`,
//! `duck`, `rebake`, `set_muffle`, ...), and [`Music::start_round`],
//! [`Music::escalate`] and [`Music::set_music_db`] carry the music logic
//! of `game.gd` (track changes only after `MIN_TRACK_SECONDS`, the round
//! level only rising, the volume setting muting at its minimum).
//!
//! Call [`Music::update`] once per frame, also while paused (Godot runs it
//! with `PROCESS_MODE_ALWAYS`), and pull stereo with [`Music::render`] or
//! the [`audio::Source`] implementation (any channel count). [`Shared`] and
//! [`Player`] put it in the speakers' mix while the game thread controls
//! it; [`Director`] follows a `World` the way `game.gd` does. Without stems
//! the music renders silence.
//!
//! Why a direct port and not synth patches: the soundtrack is recordings
//! whose sound is defined by sample-accurate lock-step playback, per-set
//! buses (filter, compressor, limiter) and bar-aligned gain changes. The
//! synth's voices would need six clip voices kept in phase, per-set bus
//! effects the synth does not have, and gain commands timed to the bar from
//! another thread. A dedicated player is shorter, exact, and cheap: silent
//! stems only advance their playheads and silent sets skip their bus.
// The whole ProceduralMusic surface is kept; the volume setting and the
// GameNight host-music hook have no callers in the Rust app yet.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};

/// Stem files per loop set, in order low, mid, high (`LOOP_SETS`).
pub const STEM_FILES: [[&str; 3]; 2] = [
    ["5-low.wav", "5-mid.wav", "5-high.wav"],
    ["6-low.wav", "6-mid.wav", "6-high.wav"],
];
/// Energy at which each stem plays at full gain.
const STEM_PEAKS: [f32; 3] = [1.0, 2.0, 3.0];
/// A stem fades out over this many energy units away from its peak.
const STEM_SPACING: f32 = 1.0;
pub const MAX_ENERGY: i32 = 3;
const MUFFLE_CUTOFF_OPEN: f32 = 18_000.0;
const MUFFLE_CUTOFF_CLOSED: f32 = 700.0;
const MUFFLE_TRANSITION_SECONDS: f32 = 1.0;
const RETIRE_FADE_SECONDS: f32 = 3.0;
const PREVIEW_FADE_SECONDS: f32 = 3.0;
const MUSIC_BPM: f32 = 120.0;
const BEATS_PER_BAR: f32 = 4.0;
pub const BAR_SECONDS: f32 = 60.0 / MUSIC_BPM * BEATS_PER_BAR;
/// Duck decay per second and the attenuation at a duck of 1.
const DUCK_DECAY_PER_SECOND: f32 = 2.5;
const DUCK_DB: f32 = -18.0;
/// The music volume setting (`MenuHelpers.music_db`): default and minimum;
/// at the minimum the music is off.
pub const MUSIC_DB_DEFAULT: f32 = -16.0;
pub const MUSIC_DB_MIN: f32 = -45.0;
pub const MUSIC_DB_MAX: f32 = 0.0;
/// A new track starts at a round boundary only after the current one has
/// played this long (`game.gd`).
pub const MIN_TRACK_SECONDS: f32 = 120.0;
/// Godot applies a volume change as a ramp across one 512-frame mix buffer
/// at 44.1 kHz.
const GAIN_RAMP_SECONDS: f32 = 512.0 / 44_100.0;
/// Frames per control block (gains, filter cutoffs, bar check).
const BLOCK: usize = 64;
/// Gains below this are Godot's -80 dB floor: silent.
const SILENT_GAIN: f32 = 0.0001;
/// `Weapon.BASE_DAMAGE`, for [`shot_duck`].
const BASE_DAMAGE: f32 = 22.0;

/// Duck for a gunshot of `damage` (`sfx.gd`): 0.18 for a base round, up to
/// 0.5 for heavy ones.
pub fn shot_duck(damage: f32) -> f32 {
    let dmg_log = (damage / BASE_DAMAGE).max(1.0).log2();
    0.18 + (dmg_log * 0.08).clamp(0.0, 0.32)
}

/// Duck for an explosion of `radius` metres (`sfx.gd`).
pub fn explosion_duck(radius: f32) -> f32 { (0.55 + radius * 0.025).clamp(0.55, 0.95) }

/// `game.gd`'s late-round heuristic: exactly two players alive and one of
/// them at or below half health. Each entry is (health, max health).
pub fn late_round(alive: &[(i32, i32)]) -> bool {
    alive.len() == 2 && alive.iter().any(|&(hp, max)| hp > 0 && hp <= (max as f32 * 0.5).ceil() as i32)
}

fn db_to_linear(db: f32) -> f32 { (db * (std::f32::consts::LN_10 / 20.0)).exp() }
fn linear_to_db(x: f32) -> f32 { x.ln() * (20.0 / std::f32::consts::LN_10) }
fn move_toward(from: f32, to: f32, delta: f32) -> f32 {
    if (to - from).abs() <= delta { to } else { from + (to - from).signum() * delta }
}

/// One looping stem: 16-bit interleaved stereo at its own sample rate.
#[derive(Clone)]
pub struct Stem {
    rate: f32,
    frames: usize,
    data: Arc<[i16]>,
}

impl Stem {
    /// Build a stem from a function of the frame index, for tests or
    /// placeholder music.
    pub fn from_fn(rate: f32, frames: usize, mut f: impl FnMut(usize) -> (f32, f32)) -> Stem {
        let q = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0) as i16;
        let mut data = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let (l, r) = f(i);
            data.push(q(l));
            data.push(q(r));
        }
        Stem { rate, frames: frames.max(1), data: if frames == 0 { vec![0, 0].into() } else { data.into() } }
    }

    /// Decode a RIFF WAV: PCM 8/16/24/32-bit or 32-bit float, mono or
    /// stereo (more channels keep the first two).
    pub fn from_wav(b: &[u8]) -> Result<Stem, String> {
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" { return Err("not a RIFF WAVE file".into()); }
        let (mut fmt, mut data) = (None, None);
        let mut i = 12;
        while i + 8 <= b.len() {
            let size = u32_at(i + 4) as usize;
            let body = i + 8;
            let end = body.saturating_add(size).min(b.len());
            match &b[i..i + 4] {
                b"fmt " if size >= 16 && body + 16 <= b.len() => {
                    let sub = if size >= 26 && body + 26 <= b.len() { u16_at(body + 24) } else { 0 };
                    fmt = Some((u16_at(body), u16_at(body + 2) as usize, u32_at(body + 4), u16_at(body + 14) as usize, sub));
                }
                b"data" => data = Some(&b[body.min(b.len())..end]),
                _ => {}
            }
            i = body.saturating_add(size + (size & 1));
        }
        let (mut format, channels, rate, bits, sub) = fmt.ok_or("missing fmt chunk")?;
        let data = data.ok_or("missing data chunk")?;
        if format == 0xFFFE { format = sub; }
        if channels == 0 || rate == 0 { return Err("bad fmt chunk".into()); }
        let width = bits / 8;
        let frame = width * channels;
        if frame == 0 { return Err(format!("unsupported {bits}-bit samples")); }
        let sample: fn(&[u8]) -> i16 = match (format, bits) {
            (1, 8) => |s| ((s[0] as i16) - 128) << 8,
            (1, 16) => |s| i16::from_le_bytes([s[0], s[1]]),
            (1, 24) => |s| i16::from_le_bytes([s[1], s[2]]),
            (1, 32) => |s| i16::from_le_bytes([s[2], s[3]]),
            (3, 32) => |s| (f32::from_le_bytes([s[0], s[1], s[2], s[3]]).clamp(-1.0, 1.0) * 32767.0) as i16,
            _ => return Err(format!("unsupported WAV format {format} with {bits} bits")),
        };
        let mut out = Vec::with_capacity(data.len() / frame * 2);
        for f in data.chunks_exact(frame) {
            let l = sample(&f[..width]);
            let r = if channels > 1 { sample(&f[width..2 * width]) } else { l };
            out.push(l);
            out.push(r);
        }
        if out.is_empty() { return Err("no samples".into()); }
        Ok(Stem { rate: rate as f32, frames: out.len() / 2, data: out.into() })
    }

    pub fn load(path: &Path) -> Result<Stem, String> {
        let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Stem::from_wav(&b).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn seconds(&self) -> f32 { self.frames as f32 / self.rate }

    fn frame(&self, i: usize) -> (f32, f32) {
        (self.data[2 * i] as f32 / 32768.0, self.data[2 * i + 1] as f32 / 32768.0)
    }
}

/// One loop set: low, mid and high stems.
pub type StemSet = [Stem; 3];

/// Load the bundled loop sets ([`STEM_FILES`]) from `dir`, such as the Godot
/// project's `assets/music`. About 60 MB; load off the main thread.
pub fn load_stem_sets(dir: &Path) -> Result<Vec<StemSet>, String> {
    STEM_FILES.iter().map(|names| {
        Ok([Stem::load(&dir.join(names[0]))?, Stem::load(&dir.join(names[1]))?, Stem::load(&dir.join(names[2]))?])
    }).collect()
}

/// Triangle gain per stem: 1 at its peak, 0 one spacing away; energies
/// below 1 also fade the whole mix (`_stem_gains`).
fn stem_gains(energy: f32) -> [f32; 3] {
    if energy <= 0.0 { return [0.0; 3]; }
    let fade_in = energy.clamp(0.0, 1.0);
    STEM_PEAKS.map(|p| (1.0 - (energy - p).abs() / STEM_SPACING).clamp(0.0, 1.0) * fade_in)
}

/// Godot's `AudioEffectLowPassFilter` with default resonance 0.5: one RBJ
/// biquad stage, direct form I, per channel.
#[derive(Clone, Default)]
struct LowPass {
    cutoff: f32,
    b: [f32; 3],
    a: [f32; 2],
    x: [[f32; 2]; 2],
    y: [[f32; 2]; 2],
}

impl LowPass {
    fn set(&mut self, cutoff: f32, sr: f32) {
        if cutoff == self.cutoff { return; }
        self.cutoff = cutoff;
        let fc = if cutoff > sr / 2.0 { sr / 2.0 - 100.0 } else { cutoff }.max(1.0);
        let w = std::f32::consts::TAU * fc / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * 0.5);
        let a0 = 1.0 + alpha;
        self.b = [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0];
        self.a = [-2.0 * c / a0, (1.0 - alpha) / a0];
    }

    /// No signal left in the filter's memory.
    fn quiet(&self) -> bool {
        self.x.iter().chain(&self.y).flatten().all(|v| v.abs() < 1e-9)
    }

    fn reset(&mut self) { self.x = [[0.0; 2]; 2]; self.y = [[0.0; 2]; 2]; }

    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let [x1, x2] = self.x[ch];
        let [y1, y2] = self.y[ch];
        let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
        self.x[ch] = [x, x1];
        self.y[ch] = [y, y1];
        y
    }
}

/// Godot's `AudioEffectCompressor` (threshold -9 dB, ratio 4, attack
/// 2500 us, release 140 ms, no makeup), peak-detecting on both channels.
#[derive(Clone)]
struct Compressor {
    threshold: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    run_db: f32,
}

impl Compressor {
    fn new(sr: f32) -> Compressor {
        Compressor {
            threshold: db_to_linear(-9.0),
            ratio: 4.0,
            attack: (-1.0 / (0.0025 * sr)).exp(),
            release: (-1.0 / (0.140 * sr)).exp(),
            run_db: 0.0,
        }
    }

    fn gain(&mut self, l: f32, r: f32) -> f32 {
        let peak = l.abs().max(r.abs());
        // Godot's detector scales the overshoot by this constant (from SC4).
        let over_db = if peak > 0.0 { (2.081_369 * linear_to_db(peak / self.threshold)).max(0.0) } else { 0.0 };
        let coef = if over_db > self.run_db { self.attack } else { self.release };
        self.run_db = over_db + coef * (self.run_db - over_db);
        if self.run_db < 1e-9 { return 1.0; }
        db_to_linear(-self.run_db * (self.ratio - 1.0) / self.ratio)
    }
}

/// Godot's `AudioEffectLimiter` (threshold -2 dB, ceiling -0.5 dB, soft
/// clip 1.5 dB): makeup from threshold to ceiling, then a soft-clip curve
/// above -1.5 dB and a hard ceiling.
#[derive(Clone, Copy)]
struct Limiter {
    makeup: f32,
    ceiling: f32,
    ceiling_db: f32,
    scv: f32,
    sc_mult: f32,
}

impl Limiter {
    fn new() -> Limiter {
        let (threshold_db, ceiling_db, soft_clip_db) = (-2.0f32, -0.5f32, 1.5f32);
        let sc = -soft_clip_db;
        let peak_db = ceiling_db + 25.0;
        Limiter {
            makeup: db_to_linear(ceiling_db - threshold_db),
            ceiling: db_to_linear(ceiling_db),
            ceiling_db,
            scv: db_to_linear(sc),
            sc_mult: ((ceiling_db - sc) / (peak_db - sc)).abs(),
        }
    }

    fn process(&self, x: f32) -> f32 {
        let x = x * self.makeup;
        let a = x.abs();
        let mut y = a;
        if a > self.scv {
            let over_db = linear_to_db(a) - self.ceiling_db;
            y = self.scv + db_to_linear(over_db * self.sc_mult);
        }
        y.min(self.ceiling).copysign(x)
    }
}

/// One loop set: its stems, playheads, gains and bus.
struct Set {
    stems: [Stem; 3],
    pos: [f64; 3],
    gain: [f32; 3],
    target: [f32; 3],
    step: [f32; 3],
    retire_fade: f32,
    frozen_energy: f32,
    muffle: f32,
    lp: LowPass,
    comp: Compressor,
}

impl Set {
    fn new(stems: StemSet, sr: f32) -> Set {
        let mut lp = LowPass::default();
        lp.set(MUFFLE_CUTOFF_OPEN, sr);
        Set {
            stems,
            pos: [0.0; 3],
            gain: [0.0; 3],
            target: [0.0; 3],
            step: [0.0; 3],
            retire_fade: 0.0,
            frozen_energy: 0.0,
            muffle: 0.0,
            lp,
            comp: Compressor::new(sr),
        }
    }

    fn seek_start(&mut self) { self.pos = [0.0; 3]; }
}

pub struct Music {
    sr: f32,
    seed: u64,
    sets: Vec<Set>,
    limiter: Limiter,
    ramp_frames: f32,
    /// `enabled`: off silences the music and drops the target energy to 0.
    enabled: bool,
    music_db: f32,
    host_music: bool,
    target_energy: f32,
    energy: f32,
    duck: f32,
    current: usize,
    muffle_active: bool,
    preview: f32,
    preview_target: f32,
    /// Bar of the current set last seen; -1 before the first.
    last_bar: i64,
    /// `game.gd`'s `_round_music_level`.
    round_level: i32,
    /// Seconds the current track has played; None before the first track of
    /// a match (`_music_track_started_ms == 0`).
    track_age: Option<f32>,
    /// Stereo scratch for [`audio::Source::render`] at other channel counts.
    scratch: Vec<f32>,
}

impl Music {
    /// Silent until stems are given with [`Music::set_stems`]. The seed
    /// picks the starting loop set, as `randi()` did in `_ready`.
    pub fn new(sample_rate: f32, seed: u64) -> Music {
        Music {
            sr: sample_rate,
            seed,
            sets: Vec::new(),
            limiter: Limiter::new(),
            ramp_frames: (GAIN_RAMP_SECONDS * sample_rate).max(1.0),
            enabled: true,
            music_db: MUSIC_DB_DEFAULT,
            host_music: false,
            target_energy: 0.0,
            energy: 0.0,
            duck: 0.0,
            current: 0,
            muffle_active: false,
            preview: 0.0,
            preview_target: 0.0,
            last_bar: -1,
            round_level: 1,
            track_age: None,
            scratch: Vec::new(),
        }
    }

    /// Output rate (the device's); filters and the compressor follow.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate == self.sr { return; }
        self.sr = sample_rate;
        self.ramp_frames = (GAIN_RAMP_SECONDS * sample_rate).max(1.0);
        for set in &mut self.sets {
            set.lp.cutoff = 0.0;
            set.lp.reset();
            set.comp = Compressor::new(sample_rate);
        }
    }

    pub fn sample_rate(&self) -> f32 { self.sr }

    /// Whether loop sets are loaded.
    pub fn has_stems(&self) -> bool { !self.sets.is_empty() }

    pub fn with_stems(sample_rate: f32, seed: u64, sets: Vec<StemSet>) -> Music {
        let mut m = Music::new(sample_rate, seed);
        m.set_stems(sets);
        m
    }

    /// Replace the loop sets and start them all together from the top.
    pub fn set_stems(&mut self, sets: Vec<StemSet>) {
        self.replace_sets(sets.into_iter().map(|s| Set::new(s, self.sr)).collect());
    }

    fn replace_sets(&mut self, sets: Vec<Set>) {
        self.sets = sets;
        let mut z = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        self.current = if self.sets.is_empty() { 0 } else { (z % self.sets.len() as u64) as usize };
        self.last_bar = -1;
    }

    // ---- ProceduralMusic's public surface ---------------------------------

    /// Target energy 0..=3. Takes effect at the next bar, or now if
    /// `immediate`.
    pub fn set_energy(&mut self, level: i32, immediate: bool) {
        self.target_energy = level.clamp(0, MAX_ENERGY) as f32;
        if immediate { self.energy = self.target_energy; }
    }

    pub fn energy(&self) -> i32 { self.target_energy.round() as i32 }

    /// Duck the music by up to 18 dB; the strongest duck wins and decays at
    /// 2.5 per second. See [`shot_duck`] and [`explosion_duck`].
    pub fn duck(&mut self, amount: f32) { self.duck = self.duck.max(amount.clamp(0.0, 1.0)); }

    /// Switch to the next loop set from its downbeat; the previous one holds
    /// its mix, muffles and fades out over 3 s.
    pub fn rebake(&mut self) {
        if self.sets.is_empty() { return; }
        let old = self.current;
        self.sets[old].retire_fade = 1.0;
        self.sets[old].frozen_energy = self.energy;
        self.current = (old + 1) % self.sets.len();
        let cur = &mut self.sets[self.current];
        cur.retire_fade = 0.0;
        cur.seek_start();
        self.last_bar = 0;
    }

    /// Card pick: muffle the current set and fade in the next set's low
    /// stem as a preview.
    pub fn set_muffle(&mut self, active: bool) {
        self.muffle_active = active;
        self.preview_target = if active { 1.0 } else { 0.0 };
    }

    /// `enabled`: off silences the music.
    pub fn set_enabled(&mut self, enabled: bool) { self.enabled = enabled; }

    /// Raw music volume in dB (`music_db`); see [`Music::set_music_db`] for
    /// the settings behaviour.
    pub fn set_volume_db(&mut self, db: f32) { self.music_db = db; }

    /// Host (GameNight) music takes priority: mute while it plays.
    pub fn set_host_music_playing(&mut self, playing: bool) { self.host_music = playing; }

    // ---- game.gd's music logic ---------------------------------------------

    fn muted_by_setting(&self) -> bool { self.music_db <= MUSIC_DB_MIN + 0.5 }

    /// `_set_music_energy`: remember the round level and set the energy,
    /// or 0 while the volume setting is at its minimum.
    pub fn set_level(&mut self, level: i32, immediate: bool) {
        if level > 0 { self.round_level = level; }
        if self.muted_by_setting() {
            self.set_energy(0, true);
        } else {
            self.set_energy(level, immediate);
        }
    }

    /// `_set_round_music_level`: raise the round level (2 for a near miss,
    /// 3 for a hit or [`late_round`]); never lowers it. Lands on the bar.
    pub fn escalate(&mut self, level: i32) {
        if level <= self.round_level { return; }
        self.round_level = level;
        self.set_level(level, false);
    }

    pub fn round_level(&self) -> i32 { self.round_level }

    /// A new match: the first round starts a fresh track.
    pub fn start_match(&mut self) { self.track_age = None; }

    /// Round start: back to low. A new track (rebake, snap to low) only if
    /// the current one has played [`MIN_TRACK_SECONDS`] or this is the
    /// match's first round; otherwise ease to low on the next bar. Returns
    /// whether the track changed.
    pub fn start_round(&mut self) -> bool {
        self.round_level = 1;
        if self.track_age.is_none_or(|t| t >= MIN_TRACK_SECONDS) {
            self.track_age = Some(0.0);
            self.rebake();
            self.set_level(1, true);
            true
        } else {
            self.set_level(1, false);
            false
        }
    }

    /// The music volume setting (`_apply_settings`): the minimum turns the
    /// music off; otherwise it plays at the round level.
    pub fn set_music_db(&mut self, db: f32) {
        self.music_db = db;
        let muted = self.muted_by_setting();
        self.enabled = !muted;
        if muted {
            self.set_energy(0, true);
        } else {
            self.set_energy(self.round_level, true);
        }
    }

    // ---- per frame ---------------------------------------------------------

    /// Advance the slow controls (`_process`): duck decay, retire, preview
    /// and muffle fades, and the track clock.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        if !self.enabled { self.target_energy = 0.0; }
        self.duck = (self.duck - dt * DUCK_DECAY_PER_SECOND).max(0.0);
        if let Some(t) = &mut self.track_age { *t += dt; }
        let retire_step = dt / RETIRE_FADE_SECONDS.max(0.01);
        self.preview = move_toward(self.preview, self.preview_target, dt / PREVIEW_FADE_SECONDS.max(0.01));
        let muffle_step = dt / MUFFLE_TRANSITION_SECONDS.max(0.01);
        for (k, set) in self.sets.iter_mut().enumerate() {
            let muffle = if k == self.current {
                self.muffle_active
            } else {
                set.retire_fade = move_toward(set.retire_fade, 0.0, retire_step);
                set.retire_fade > 0.0
            };
            set.muffle = move_toward(set.muffle, if muffle { 1.0 } else { 0.0 }, muffle_step);
        }
    }

    /// Mix into `out` (interleaved stereo, overwritten) at the sample rate
    /// given to [`Music::new`].
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if self.sets.is_empty() { return; }
        let frames = out.len() / 2;
        let mut i = 0;
        while i < frames {
            self.check_bar();
            let n = (frames - i).min(BLOCK).min(self.frames_to_bar());
            self.set_targets();
            self.render_block(&mut out[2 * i..2 * (i + n)]);
            i += n;
        }
    }

    /// `_apply_bar_aligned_energy`: on entering a new bar of the current
    /// set's playhead (a loop wrap counts), the energy takes its target.
    fn check_bar(&mut self) {
        let set = &self.sets[self.current];
        let bar = (set.pos[0] / set.stems[0].rate as f64 / BAR_SECONDS as f64).floor() as i64;
        if bar != self.last_bar {
            self.last_bar = bar;
            self.energy = self.target_energy;
        }
    }

    /// Output frames until the current set's next bar line or loop end.
    fn frames_to_bar(&self) -> usize {
        let set = &self.sets[self.current];
        let stem = &set.stems[0];
        let bar = BAR_SECONDS as f64 * stem.rate as f64;
        let next = (((set.pos[0] / bar).floor() + 1.0) * bar).min(stem.frames as f64);
        let step = stem.rate as f64 / self.sr as f64;
        (((next - set.pos[0]) / step).ceil() as usize).max(1)
    }

    /// `_apply_volumes` and the per-set filter cutoff (`_update_muffle`).
    fn set_targets(&mut self) {
        let master = if self.host_music { 0.0 } else { db_to_linear(self.music_db + DUCK_DB * self.duck.clamp(0.0, 1.0)) };
        let active = self.energy > 0.001 && self.enabled;
        let current = stem_gains(self.energy);
        let preview = stem_gains(1.0);
        let (open, closed) = (MUFFLE_CUTOFF_OPEN.ln(), MUFFLE_CUTOFF_CLOSED.ln());
        for (k, set) in self.sets.iter_mut().enumerate() {
            let (gains, visibility) = if k == self.current {
                (current, 1.0)
            } else if set.retire_fade > 0.0 {
                (stem_gains(set.frozen_energy), set.retire_fade)
            } else {
                (preview, self.preview)
            };
            for s in 0..3 {
                let g = gains[s] * visibility;
                let target = if !active || g <= SILENT_GAIN { 0.0 } else { g * master };
                if target != set.target[s] || set.step[s] == 0.0 {
                    set.target[s] = target;
                    set.step[s] = ((target - set.gain[s]).abs() / self.ramp_frames).max(1e-9);
                }
            }
            set.lp.set((open + (closed - open) * set.muffle).exp(), self.sr);
        }
    }

    fn render_block(&mut self, out: &mut [f32]) {
        let n = out.len() / 2;
        let sr = self.sr as f64;
        let limiter = self.limiter;
        for set in &mut self.sets {
            // A silent set whose filter has rung out only moves its playheads.
            let silent = set.gain.iter().chain(&set.target).all(|&g| g == 0.0);
            if silent && set.lp.quiet() {
                for s in 0..3 {
                    let stem = &set.stems[s];
                    set.pos[s] = (set.pos[s] + stem.rate as f64 / sr * n as f64) % stem.frames as f64;
                }
                set.comp.run_db = 0.0;
                continue;
            }
            let mut buf = [[0.0f32; 2]; BLOCK];
            for s in 0..3 {
                let stem = &set.stems[s];
                let len = stem.frames as f64;
                let step = stem.rate as f64 / sr;
                let mut pos = set.pos[s];
                if set.gain[s] == 0.0 && set.target[s] == 0.0 {
                    pos += step * n as f64;
                } else {
                    let (mut g, target, dg) = (set.gain[s], set.target[s], set.step[s]);
                    for frame in buf.iter_mut().take(n) {
                        let i = pos as usize;
                        let t = (pos - i as f64) as f32;
                        let (l0, r0) = stem.frame(i);
                        let (l1, r1) = stem.frame(if i + 1 >= stem.frames { 0 } else { i + 1 });
                        g = move_toward(g, target, dg);
                        frame[0] += (l0 + (l1 - l0) * t) * g;
                        frame[1] += (r0 + (r1 - r0) * t) * g;
                        pos += step;
                        if pos >= len { pos -= len; }
                    }
                    set.gain[s] = g;
                }
                set.pos[s] = pos % len;
            }
            for (o, frame) in out.chunks_exact_mut(2).zip(buf.iter()) {
                let l = set.lp.process(0, frame[0]);
                let r = set.lp.process(1, frame[1]);
                let g = set.comp.gain(l, r);
                o[0] += limiter.process(l * g);
                o[1] += limiter.process(r * g);
            }
        }
    }
}

impl audio::Source for Music {
    /// Stereo on the first two channels (mono: their average), silence on
    /// the rest.
    fn render(&mut self, out: &mut [f32], channels: usize) {
        if channels == 2 { Music::render(self, out); return; }
        out.fill(0.0);
        if channels == 0 { return; }
        let frames = out.len() / channels;
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.resize(frames * 2, 0.0);
        Music::render(self, &mut scratch);
        for (o, s) in out.chunks_exact_mut(channels).zip(scratch.chunks_exact(2)) {
            if channels == 1 { o[0] = 0.5 * (s[0] + s[1]); } else { o[0] = s[0]; o[1] = s[1]; }
        }
        self.scratch = scratch;
    }
}

/// Ordered game-time controls. Updates carry the original frame delta; the
/// audio device never substitutes its own clock for gameplay's music clock.
#[derive(Clone, Copy)]
enum ControlCommand {
    SampleRate(f32), Energy(i32, bool), Duck(f32), Rebake, Muffle(bool),
    Enabled(bool), Volume(f32), HostMusic(bool), Level(i32, bool),
    Escalate(i32), StartMatch, StartRound, MusicDb(f32), Update(f32),
}

impl ControlCommand {
    fn apply(self, m: &mut Music) -> bool {
        match self {
            Self::SampleRate(rate) => m.set_sample_rate(rate),
            Self::Energy(level, immediate) => m.set_energy(level, immediate),
            Self::Duck(amount) => m.duck(amount),
            Self::Rebake => m.rebake(),
            Self::Muffle(active) => m.set_muffle(active),
            Self::Enabled(enabled) => m.set_enabled(enabled),
            Self::Volume(db) => m.set_volume_db(db),
            Self::HostMusic(playing) => m.set_host_music_playing(playing),
            Self::Level(level, immediate) => m.set_level(level, immediate),
            Self::Escalate(level) => m.escalate(level),
            Self::StartMatch => m.start_match(),
            Self::StartRound => return m.start_round(),
            Self::MusicDb(db) => m.set_music_db(db),
            Self::Update(dt) => m.update(dt),
        }
        false
    }
}

enum Command { Control(ControlCommand), Stems(Vec<Set>) }

/// Control producers share only this small game-side object. Its mutex is
/// never acquired by the audio callback, whose Music is owned by [`Player`].
pub type Shared = Arc<Mutex<Controls>>;

pub struct Controls {
    tx: Sender<Command>,
    retired: Receiver<Vec<Set>>,
    /// No stems or DSP buffers: synchronous queries follow ordered controls
    /// even if the device has not pulled the next audio buffer yet.
    shadow: Music,
    has_stems: bool,
    player: Option<Player>,
}

pub fn shared(music: Music) -> Shared {
    let (tx, rx) = channel();
    let (retire, retired) = channel();
    let mut shadow = Music::new(music.sr, music.seed);
    shadow.enabled = music.enabled;
    shadow.music_db = music.music_db;
    shadow.host_music = music.host_music;
    shadow.target_energy = music.target_energy;
    shadow.energy = music.energy;
    shadow.round_level = music.round_level;
    shadow.track_age = music.track_age;
    let has_stems = music.has_stems();
    Arc::new(Mutex::new(Controls { tx, retired, shadow, has_stems, player: Some(Player { music, rx, retire }) }))
}

/// Serialize control producers (game and background loader), never rendering.
pub fn lock(m: &Shared) -> std::sync::MutexGuard<'_, Controls> { m.lock().unwrap_or_else(|e| e.into_inner()) }

impl Controls {
    fn send(&mut self, command: ControlCommand) -> bool {
        // A replaced stem bank can own tens of MB. Reclaim it on a control
        // producer, after the audio callback has relinquished it.
        while self.retired.try_recv().is_ok() {}
        let result = command.apply(&mut self.shadow);
        let _ = self.tx.send(Command::Control(command));
        result
    }
    pub fn set_sample_rate(&mut self, rate: f32) { self.send(ControlCommand::SampleRate(rate)); }
    pub fn sample_rate(&self) -> f32 { self.shadow.sample_rate() }
    pub fn has_stems(&self) -> bool { self.has_stems }
    pub fn set_stems(&mut self, stems: Vec<StemSet>) {
        self.has_stems = !stems.is_empty();
        // Allocate buses off the audio thread as well as decoding samples there.
        let sets = stems.into_iter().map(|s| Set::new(s, self.shadow.sr)).collect();
        let _ = self.tx.send(Command::Stems(sets));
    }
    pub fn set_energy(&mut self, level: i32, immediate: bool) { self.send(ControlCommand::Energy(level, immediate)); }
    pub fn energy(&self) -> i32 { self.shadow.energy() }
    pub fn duck(&mut self, amount: f32) { self.send(ControlCommand::Duck(amount)); }
    pub fn rebake(&mut self) { self.send(ControlCommand::Rebake); }
    pub fn set_muffle(&mut self, active: bool) { self.send(ControlCommand::Muffle(active)); }
    pub fn set_enabled(&mut self, enabled: bool) { self.send(ControlCommand::Enabled(enabled)); }
    pub fn set_volume_db(&mut self, db: f32) { self.send(ControlCommand::Volume(db)); }
    pub fn set_host_music_playing(&mut self, playing: bool) { self.send(ControlCommand::HostMusic(playing)); }
    pub fn set_level(&mut self, level: i32, immediate: bool) { self.send(ControlCommand::Level(level, immediate)); }
    pub fn escalate(&mut self, level: i32) { self.send(ControlCommand::Escalate(level)); }
    pub fn round_level(&self) -> i32 { self.shadow.round_level() }
    pub fn start_match(&mut self) { self.send(ControlCommand::StartMatch); }
    pub fn start_round(&mut self) -> bool { self.send(ControlCommand::StartRound) }
    pub fn set_music_db(&mut self, db: f32) { self.send(ControlCommand::MusicDb(db)); }
    pub fn update(&mut self, dt: f32) { self.send(ControlCommand::Update(dt)); }
}

/// Audio-owned player. Taking it out of Shared is a one-time setup operation;
/// rendering drains commands without borrowing or locking game-side state.
pub struct Player { music: Music, rx: Receiver<Command>, retire: Sender<Vec<Set>> }

impl Player {
    pub fn new(shared: &Shared) -> Player { lock(shared).player.take().expect("music player already attached") }

    fn drain(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            match command {
                Command::Control(command) => { command.apply(&mut self.music); }
                Command::Stems(sets) => {
                    let previous = std::mem::take(&mut self.music.sets);
                    self.music.replace_sets(sets);
                    let _ = self.retire.send(previous);
                }
            }
        }
    }
}

impl audio::Source for Player {
    fn render(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        audio::Source::render(&mut self.music, out, channels);
    }
}

/// The Director can drive either an offline Music or the queued live controls.
pub trait DirectedMusic {
    fn start_match(&mut self);
    fn start_round(&mut self) -> bool;
    fn escalate(&mut self, level: i32);
    fn round_level(&self) -> i32;
    fn set_muffle(&mut self, active: bool);
}

macro_rules! directed_music {
    ($ty:ty) => {
        impl DirectedMusic for $ty {
            fn start_match(&mut self) { <$ty>::start_match(self); }
            fn start_round(&mut self) -> bool { <$ty>::start_round(self) }
            fn escalate(&mut self, level: i32) { <$ty>::escalate(self, level); }
            fn round_level(&self) -> i32 { <$ty>::round_level(self) }
            fn set_muffle(&mut self, active: bool) { <$ty>::set_muffle(self, active); }
        }
    };
}
directed_music!(Music);
directed_music!(Controls);

/// The stem folder: `--music DIR`, `GG_MUSIC`, `music` beside the
/// executable, `music` beside the sound folder (the Godot project's
/// `assets`), or a sibling checkout's `../growing-guns/assets/music`.
pub fn find_dir(arg: Option<&str>, sounds: Option<&Path>) -> Option<PathBuf> {
    let candidates = [arg.map(PathBuf::from), std::env::var("GG_MUSIC").ok().map(PathBuf::from), crate::sounds::beside_exe("music"),
        sounds.and_then(|d| d.parent()).map(|d| d.join("music")), Some(PathBuf::from("../growing-guns/assets/music"))];
    candidates.into_iter().flatten().find(|p| p.join(STEM_FILES[0][0]).exists())
}

/// Load the stems from `dir` into `music`, on a thread when `background`
/// (the music starts once they are in; about 60 MB).
pub fn load_into(music: &Shared, dir: PathBuf, background: bool) {
    let m = music.clone();
    let job = move || match load_stem_sets(&dir) {
        Ok(sets) => { lock(&m).set_stems(sets); println!("music: {} loop sets from {}", STEM_FILES.len(), dir.display()); }
        Err(e) => eprintln!("silent music: {e}"),
    };
    if background {
        if let Err(e) = std::thread::Builder::new().name("music-load".into()).spawn(job) { eprintln!("silent music: {e}"); }
    } else {
        job();
    }
}

/// `game.gd`'s music logic, driven by a `World` each tick: a new match and
/// each round start ([`Music::start_round`]), a near miss past a local
/// player (mid), damage to anyone from someone else or the late-round
/// condition (high), and the card-pick muffle once fewer than two players
/// are left standing.
#[derive(Default)]
pub struct Director { started: bool }

/// Within this distance of a local player's ear a bullet zips by (`bullet.gd`).
const ZIP_RADIUS: f32 = 12.0;

impl Director {
    /// After a tick (or a client snapshot); `slots` are the local players.
    pub fn follow(&mut self, m: &mut impl DirectedMusic, world: &growing_guns::World, slots: &[usize]) {
        use growing_guns::{Event, Phase};
        if !std::mem::replace(&mut self.started, true) { m.start_match(); m.start_round(); }
        let playing = world.phase == Phase::Playing;
        for e in &world.events {
            match *e {
                Event::NewArena => {
                    if world.round <= 1 { m.start_match(); }
                    m.start_round();
                }
                Event::Hit { victim, attacker, damage, .. } if playing && damage > 0 && attacker != victim => m.escalate(3),
                Event::Shot { player, from, dir } if playing && !slots.contains(&player) => {
                    let near = slots.iter().filter_map(|&s| world.players.get(s)).any(|p| {
                        let ear = p.pos + glam::Vec3::Y * growing_guns::EYE;
                        let t = (ear - from).dot(dir).clamp(0.0, 400.0);
                        (from + dir * t).distance(ear) <= ZIP_RADIUS
                    });
                    if near { m.escalate(2); }
                }
                _ => {}
            }
        }
        if playing && m.round_level() < 3 {
            let alive: Vec<(i32, i32)> = world.players.iter().filter(|p| p.alive).map(|p| (p.health, p.max_health())).collect();
            if late_round(&alive) { m.escalate(3); }
        }
        let standing = world.players.iter().filter(|p| p.alive).count();
        m.set_muffle(world.phase == Phase::Picking && standing < 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: f32 = 48_000.0;
    /// Stem tone frequencies per (set, stem).
    const TONES: [[f32; 3]; 2] = [[110.0, 330.0, 990.0], [146.83, 440.0, 1320.0]];

    /// Two sets of sine stems at 44.1 kHz with loop lengths that are not a
    /// whole number of bars, like the real ones (57.3 s and 55.7 s).
    fn test_sets() -> Vec<StemSet> {
        let rate = 44_100.0;
        TONES.iter().enumerate().map(|(k, tones)| {
            let frames = (rate * if k == 0 { 5.3 } else { 4.7 }) as usize;
            tones.map(|f| Stem::from_fn(rate, frames, |i| {
                let x = 0.5 * (TAU * f * i as f32 / rate).sin();
                (x, x * 0.9)
            }))
        }).collect()
    }

    fn music(seed: u64) -> Music { Music::with_stems(SR, seed, test_sets()) }

    /// Run for `seconds` at 60 Hz updates, returning the interleaved output.
    fn run(m: &mut Music, seconds: f32) -> Vec<f32> {
        let per = (SR / 60.0) as usize;
        let mut out = vec![0.0; 2 * (SR * seconds) as usize];
        for chunk in out.chunks_mut(2 * per) {
            m.update(chunk.len() as f32 / 2.0 / SR);
            m.render(chunk);
        }
        out
    }

    fn left(x: &[f32]) -> Vec<f32> { x.iter().step_by(2).copied().collect() }
    fn rms(x: &[f32]) -> f32 { audio::analysis::rms(x) }

    /// Amplitude of `f` in mono `x` (Goertzel).
    fn tone(x: &[f32], f: f32) -> f32 {
        let w = TAU * f / SR;
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        let c = 2.0 * (w as f64).cos();
        for &v in x {
            let s = v as f64 + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        let p = s1 * s1 + s2 * s2 - c * s1 * s2;
        (2.0 * p.max(0.0).sqrt() / x.len() as f64) as f32
    }

    /// The seconds of the current set's next bar line, from its playhead.
    fn to_next_bar(m: &Music) -> f32 {
        m.frames_to_bar() as f32 / SR
    }

    #[test]
    fn deterministic_per_seed() {
        let script = |m: &mut Music| {
            m.set_energy(1, true);
            let mut a = run(m, 1.0);
            m.set_energy(3, false);
            m.duck(0.6);
            a.extend(run(m, 3.0));
            m.set_muffle(true);
            a.extend(run(m, 1.5));
            m.rebake();
            m.set_energy(1, true);
            a.extend(run(m, 1.0));
            a
        };
        assert_eq!(script(&mut music(7)), script(&mut music(7)));
        // The seed picks the starting loop set; both occur.
        let picks: Vec<usize> = (0..16).map(|s| music(s).current).collect();
        assert!(picks.contains(&0) && picks.contains(&1), "{picks:?}");
        let (a, b) = (picks.iter().position(|&p| p == 0).unwrap(), picks.iter().position(|&p| p == 1).unwrap());
        assert_ne!(script(&mut music(a as u64)), script(&mut music(b as u64)));
    }

    #[test]
    fn sane_levels() {
        let mut m = music(1);
        let silent = run(&mut m, 0.5);
        assert!(silent.iter().all(|&x| x == 0.0), "energy 0 is silent");
        for level in 1..=3 {
            m.set_energy(level, true);
            let x = run(&mut m, 1.0);
            let r = rms(&left(&x[4800..]));
            // One 0.5-amplitude sine at -16 dB: about 0.056 RMS.
            assert!(x.iter().all(|v| v.is_finite() && v.abs() <= db_to_linear(-0.5) + 1e-6));
            assert!((0.04..0.08).contains(&r), "level {level}: rms {r}");
        }
        // Full volume compresses and limits but never passes the ceiling.
        m.set_music_db(MUSIC_DB_MAX);
        let x = run(&mut m, 1.0);
        let peak = x.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak <= db_to_linear(-0.5) + 1e-6 && peak > 0.3, "peak {peak}");
        // The volume minimum turns the music off.
        m.set_music_db(MUSIC_DB_MIN);
        let x = run(&mut m, 0.2);
        assert!(rms(&x[4800..]) < 1e-6);
    }

    #[test]
    fn energy_changes_on_the_bar() {
        let mut m = music(0);
        let tones = TONES[m.current];
        m.set_energy(1, true);
        run(&mut m, 0.5);
        let x = left(&run(&mut m, 0.5));
        assert!(tone(&x, tones[0]) > 0.05 && tone(&x, tones[2]) < 0.001, "low stem only");
        m.set_energy(3, false);
        assert_eq!(m.energy(), 3);
        // Not before the bar line...
        let wait = to_next_bar(&m);
        assert!(wait > 0.3, "test needs room before the bar: {wait}");
        let x = left(&run(&mut m, wait - 0.25));
        assert!(tone(&x, tones[2]) < 0.001 && tone(&x, tones[0]) > 0.05, "change waits for the bar");
        // ...and after it, the high stem alone.
        run(&mut m, 0.3);
        let x = left(&run(&mut m, 0.5));
        assert!(tone(&x, tones[2]) > 0.05 && tone(&x, tones[0]) < 0.001, "high stem after the bar");
        // Immediate changes land within the gain ramp.
        m.set_energy(2, true);
        run(&mut m, 0.02);
        let x = left(&run(&mut m, 0.3));
        assert!(tone(&x, tones[1]) > 0.05 && tone(&x, tones[2]) < 0.001, "immediate mid");
    }

    #[test]
    fn duck_attenuates_and_recovers() {
        let mut m = music(2);
        m.set_energy(1, true);
        run(&mut m, 0.5);
        let base = rms(&run(&mut m, 0.25));
        m.duck(1.0);
        // 50 ms after a duck, with no update in between so it hasn't decayed.
        m.update(0.0);
        let mut x = vec![0.0; 2 * 2400];
        m.render(&mut x);
        let ducked = rms(&x[2 * 600..]);
        let ratio = ducked / base;
        assert!((db_to_linear(-18.5)..db_to_linear(-17.0)).contains(&ratio), "ducked {} dB", linear_to_db(ratio));
        // Weaker ducks don't override a stronger one.
        m.duck(0.2);
        assert_eq!(m.duck, 1.0);
        // Gone after 1 / 2.5 s.
        run(&mut m, 0.45);
        assert_eq!(m.duck, 0.0);
        let back = rms(&run(&mut m, 0.25));
        assert!((back / base - 1.0).abs() < 0.02, "recovered to {}", back / base);
        // sfx.gd's amounts.
        assert!((shot_duck(22.0) - 0.18).abs() < 1e-6 && (shot_duck(88.0) - 0.34).abs() < 1e-6 && shot_duck(1e6) == 0.5);
        assert_eq!(explosion_duck(0.0), 0.55);
        assert!((explosion_duck(8.0) - 0.75).abs() < 1e-6 && explosion_duck(30.0) == 0.95);
    }

    #[test]
    fn muffle_preview_and_rebake() {
        let mut m = music(3);
        let (cur, next) = (m.current, 1 - m.current);
        m.set_energy(3, true);
        run(&mut m, 0.5);
        let open = tone(&left(&run(&mut m, 0.5)), TONES[cur][2]);
        // Card pick: the current track low-passes to 700 Hz, the next track's
        // low stem fades in over 3 s.
        m.set_muffle(true);
        run(&mut m, 3.2);
        let x = left(&run(&mut m, 0.5));
        assert!(tone(&x, TONES[cur][2]) < open * 0.5, "muffled: {} vs {open}", tone(&x, TONES[cur][2]));
        assert!(tone(&x, TONES[next][0]) > 0.05, "preview of the next set's low stem");
        // New round with a new track: the other set from its downbeat at low,
        // the old one fading out over 3 s while holding high.
        m.set_muffle(false);
        m.start_match();
        assert!(m.start_round());
        assert_eq!(m.current, next);
        assert_eq!(m.sets[next].pos[0], 0.0);
        let x = left(&run(&mut m, 1.0));
        assert!(tone(&x, TONES[next][0]) > 0.05);
        assert!(tone(&x, TONES[cur][2]) > 0.001, "retiring set still audible");
        run(&mut m, 2.2);
        let x = left(&run(&mut m, 0.5));
        assert!(tone(&x, TONES[cur][2]) < 1e-4, "retired");
        // The new current set is never muffled.
        assert!(tone(&x, TONES[next][0]) > 0.05);
        assert_eq!(m.sets[next].muffle, 0.0);
    }

    #[test]
    fn round_logic_follows_game_gd() {
        let mut m = music(4);
        m.start_match();
        assert!(m.start_round(), "first round of a match starts a track");
        let first = m.current;
        m.escalate(2);
        m.escalate(3);
        m.escalate(2);
        assert_eq!((m.round_level(), m.energy()), (3, 3), "the round level only rises");
        // Past the bar line the energy is high. A minute in, the same track
        // carries over into the next round, easing back to low on the bar.
        run(&mut m, 2.2);
        assert_eq!(m.energy, 3.0);
        m.track_age = Some(60.0);
        assert!(!m.start_round());
        assert_eq!((m.current, m.energy(), m.energy), (first, 1, 3.0));
        // After two minutes the next round changes track and snaps to low.
        m.track_age = Some(MIN_TRACK_SECONDS);
        assert!(m.start_round());
        assert_ne!(m.current, first);
        assert_eq!(m.energy, 1.0);
        assert!(late_round(&[(50, 100), (100, 100)]) && !late_round(&[(51, 100), (100, 100)]) && !late_round(&[(10, 100)]));
        // Host music mutes without forgetting the state.
        m.set_host_music_playing(true);
        run(&mut m, 0.1);
        assert!(rms(&run(&mut m, 0.2)) < 1e-6);
        m.set_host_music_playing(false);
        assert!(rms(&run(&mut m, 0.2)) > 0.03);
    }

    #[test]
    fn source_any_channel_count() {
        let script = |channels: usize| {
            let mut m = music(5);
            m.set_energy(2, true);
            let mut out = vec![0.0; channels * 4800];
            for chunk in out.chunks_mut(channels * 480) { m.update(0.01); audio::Source::render(&mut m, chunk, channels); }
            out
        };
        let stereo = script(2);
        assert!(rms(&stereo) > 0.03);
        let six = script(6);
        for (f, s) in six.chunks_exact(6).zip(stereo.chunks_exact(2)) {
            assert_eq!((f[0], f[1]), (s[0], s[1]));
            assert!(f[2..].iter().all(|&v| v == 0.0));
        }
        let mono = script(1);
        for (m, s) in mono.iter().zip(stereo.chunks_exact(2)) { assert!((m - 0.5 * (s[0] + s[1])).abs() < 1e-7); }
        // Through the shared handle, as the speakers pull it.
        let shared = shared(music(5));
        lock(&shared).set_energy(2, true);
        let mut p = Player::new(&shared);
        let mut out = vec![0.0; 2 * 4800];
        for chunk in out.chunks_mut(2 * 480) { lock(&shared).update(0.01); audio::Source::render(&mut p, chunk, 2); }
        assert_eq!(out, stereo);
    }

    #[test]
    fn queued_controls_match_direct_audio_and_queries_in_order() {
        let mut direct = music(5);
        let shared = shared(music(5));
        let mut player = Player::new(&shared);
        let scripts = [
            vec![ControlCommand::StartMatch, ControlCommand::StartRound, ControlCommand::Escalate(3), ControlCommand::Duck(0.8), ControlCommand::Update(0.1), ControlCommand::Duck(0.2)],
            vec![ControlCommand::Muffle(true), ControlCommand::Update(0.7), ControlCommand::Rebake, ControlCommand::Level(2, true)],
            vec![ControlCommand::MusicDb(MUSIC_DB_MIN), ControlCommand::Escalate(3), ControlCommand::Update(0.02), ControlCommand::MusicDb(-10.0)],
            vec![ControlCommand::Update(MIN_TRACK_SECONDS), ControlCommand::StartRound, ControlCommand::Muffle(false), ControlCommand::SampleRate(44_100.0)],
        ];
        for commands in scripts {
            for command in commands {
                let expected = command.apply(&mut direct);
                let mut controls = lock(&shared);
                assert_eq!(controls.send(command), expected);
                assert_eq!((controls.energy(), controls.round_level(), controls.sample_rate()), (direct.energy(), direct.round_level(), direct.sample_rate()));
            }
            let (mut a, mut b) = (vec![0.0; 4096], vec![0.0; 4096]);
            audio::Source::render(&mut direct, &mut a, 2);
            audio::Source::render(&mut player, &mut b, 2);
            assert_eq!(a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), b.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
        }
        // Loading/replacing stems is ordered with controls, at the device rate.
        direct.set_stems(test_sets());
        direct.set_energy(3, true);
        {
            let mut controls = lock(&shared);
            controls.set_stems(test_sets());
            controls.set_energy(3, true);
            assert!(controls.has_stems());
        }
        let (mut a, mut b) = ([0.0; 1024], [0.0; 1024]);
        audio::Source::render(&mut direct, &mut a, 2);
        audio::Source::render(&mut player, &mut b, 2);
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
    }

    #[test]
    fn rendering_does_not_wait_for_the_game_controls_lock() {
        let shared = shared(music(5));
        lock(&shared).set_energy(2, true);
        let mut player = Player::new(&shared);
        let controls = lock(&shared);
        let (tx, rx) = channel();
        let render = std::thread::spawn(move || {
            let mut out = [0.0; 1024];
            audio::Source::render(&mut player, &mut out, 2);
            tx.send(out.iter().any(|x| *x != 0.0)).unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(controls);
        render.join().unwrap();
        assert_eq!(result, Ok(true), "audio callback must not acquire the controls mutex");
    }

    #[test]
    fn director_follows_the_world() {
        use growing_guns::{Event, World};
        let mut w = World::new(11, &[false, true, true, true]);
        let (mut m, mut d) = (music(6), Director::default());
        d.follow(&mut m, &w, &[0]);
        assert_eq!((m.round_level(), m.energy()), (1, 1), "a match starts at low");
        assert_eq!(m.track_age, Some(0.0));
        // Bots fight until someone is hurt by someone else: high.
        let mut hit = false;
        for _ in 0..60 * growing_guns::TICK_HZ {
            w.step(&[]);
            d.follow(&mut m, &w, &[0]);
            m.update(growing_guns::DT);
            if w.events.iter().any(|e| matches!(*e, Event::Hit { victim, attacker, damage, .. } if victim != attacker && damage > 0)) && w.phase == growing_guns::Phase::Playing { hit = true; break; }
        }
        assert!(hit, "no hit in a minute");
        assert_eq!(m.round_level(), 3);
        // A new round eases back to low on the same track.
        let current = m.current;
        w.next_round();
        d.follow(&mut m, &w, &[0]);
        assert_eq!((m.round_level(), m.energy(), m.current), (1, 1, current));
    }

    /// Offline render of a scripted round to a WAV for listening:
    /// `MUSIC_WAV_DIR=DIR cargo test -p growing-guns-app music::tests::render_wav -- --ignored`.
    /// Uses the real stems from `GG_MUSIC` (the Godot project's
    /// `assets/music`) when set, else the test tones; writes to
    /// `MUSIC_WAV_DIR` or the temp dir.
    #[test]
    #[ignore]
    fn render_wav() {
        let sets = match find_dir(None, None) {
            Some(dir) => load_stem_sets(&dir).expect("stems"),
            None => test_sets(),
        };
        let mut m = Music::with_stems(SR, 1, sets);
        m.set_energy(1, true);
        m.start_match();
        m.start_round();
        let mut out = run(&mut m, 6.0);
        m.escalate(2); // near miss
        out.extend(run(&mut m, 6.0));
        m.escalate(3); // hit
        for _ in 0..8 {
            m.duck(shot_duck(22.0));
            out.extend(run(&mut m, 0.25));
        }
        m.duck(explosion_duck(8.0));
        out.extend(run(&mut m, 4.0));
        m.set_muffle(true); // card pick
        out.extend(run(&mut m, 5.0));
        m.set_muffle(false);
        m.track_age = Some(MIN_TRACK_SECONDS);
        m.start_round(); // new track
        out.extend(run(&mut m, 8.0));
        let dir = std::env::var_os("MUSIC_WAV_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let path = dir.join("growing-guns-music.wav");
        audio::analysis::write_wav(&path, SR as u32, 2, &out).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
