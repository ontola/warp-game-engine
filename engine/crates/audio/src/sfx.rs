//! Sample playback for game sound effects: decoded clips, a fixed voice
//! pool with gain, equal-power pan, pitch and a distance low-pass, and
//! placement for one or more listeners (split screen hears each sound from
//! the nearest player). The game sends `Play` commands through a channel;
//! the mixer runs in the audio callback, or offline for tests.
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::SAMPLE_RATE;

/// Mono samples at `SAMPLE_RATE`.
#[derive(Clone, Debug)]
pub struct Clip(pub Arc<[f32]>);

impl Clip {
    pub fn seconds(&self) -> f32 { self.0.len() as f32 / SAMPLE_RATE }
}

fn le_u16(b: &[u8], i: usize) -> u16 { u16::from_le_bytes([b[i], b[i + 1]]) }
fn le_u32(b: &[u8], i: usize) -> u32 { u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) }

/// Decode a RIFF WAV (PCM 8/16/24/32-bit or 32-bit float, any channel
/// count and rate) to a mono clip at `SAMPLE_RATE`.
pub fn decode_wav(b: &[u8]) -> Result<Clip, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" { return Err("not a RIFF WAVE file".into()); }
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let size = le_u32(b, i + 4) as usize;
        let body = i + 8;
        let end = (body + size).min(b.len());
        match &b[i..i + 4] {
            b"fmt " if size >= 16 => fmt = Some((le_u16(b, body), le_u16(b, body + 2) as usize, le_u32(b, body + 4), le_u16(b, body + 14) as usize, if size >= 26 { le_u16(b, body + 24) } else { 0 })),
            b"data" => data = Some(&b[body..end]),
            _ => {}
        }
        i = body + size + (size & 1);
    }
    let (mut format, channels, rate, bits, sub) = fmt.ok_or("missing fmt chunk")?;
    let data = data.ok_or("missing data chunk")?;
    if format == 0xFFFE { format = sub; } // WAVE_FORMAT_EXTENSIBLE: the subformat's first word
    if channels == 0 || rate == 0 { return Err("bad fmt chunk".into()); }
    let width = bits / 8;
    let frame = width * channels;
    if frame == 0 { return Err(format!("unsupported {bits}-bit samples")); }
    let sample = |s: &[u8]| -> Result<f32, String> {
        Ok(match (format, bits) {
            (1, 8) => (s[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            _ => return Err(format!("unsupported WAV format {format} with {bits} bits")),
        })
    };
    let mut mono = Vec::with_capacity(data.len() / frame);
    for f in data.chunks_exact(frame) {
        let mut sum = 0.0;
        for c in 0..channels { sum += sample(&f[c * width..(c + 1) * width])?; }
        mono.push(sum / channels as f32);
    }
    Ok(Clip(resample(&mono, rate as f32, SAMPLE_RATE).into()))
}

pub fn load_wav(path: &std::path::Path) -> Result<Clip, String> {
    decode_wav(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
}

/// Linear resampling (enough for effects; clips are short).
fn resample(x: &[f32], from: f32, to: f32) -> Vec<f32> {
    if (from - to).abs() < 0.5 || x.is_empty() { return x.to_vec(); }
    let n = ((x.len() as f64) * to as f64 / from as f64).floor() as usize;
    let step = from as f64 / to as f64;
    (0..n).map(|k| {
        let p = k as f64 * step;
        let i = p as usize;
        let t = (p - i as f64) as f32;
        let a = x[i.min(x.len() - 1)];
        let b = x[(i + 1).min(x.len() - 1)];
        a + (b - a) * t
    }).collect()
}

/// How one voice sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    /// Linear gain.
    pub gain: f32,
    /// -1 left .. 1 right (equal power).
    pub pan: f32,
    /// Playback rate: 2 is an octave up.
    pub pitch: f32,
    /// One-pole low-pass cutoff; distance dulls a sound.
    pub lowpass_hz: f32,
}

impl Default for Play {
    fn default() -> Self { Play { gain: 1.0, pan: 0.0, pitch: 1.0, lowpass_hz: 20_000.0 } }
}

impl Play {
    pub fn db(mut self, db: f32) -> Self { self.gain *= 10f32.powf(db / 20.0); self }
    pub fn pitch(mut self, p: f32) -> Self { self.pitch = p; self }
}

/// An ear: position and the unit vector to its right.
#[derive(Clone, Copy, Debug)]
pub struct Listener { pub pos: [f32; 3], pub right: [f32; 3] }

/// Place a sound at `at` for the nearest listener. `unit` is the distance
/// within which it plays at full gain; beyond that it falls as unit/d
/// (Godot's inverse-distance model) and loses highs towards 4 kHz.
pub fn spatial(at: [f32; 3], listeners: &[Listener], unit: f32) -> Play {
    let Some((d, l)) = listeners.iter().map(|l| {
        let r = [at[0] - l.pos[0], at[1] - l.pos[1], at[2] - l.pos[2]];
        ((r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt(), (l, r))
    }).min_by(|a, b| a.0.total_cmp(&b.0)) else { return Play::default() };
    let (l, r) = l;
    let gain = if d <= unit { 1.0 } else { unit / d };
    let pan = if d > 1e-3 { (r[0] * l.right[0] + r[1] * l.right[1] + r[2] * l.right[2]) / d } else { 0.0 };
    // Several listeners on one speaker pair: keep the pan modest.
    let pan = pan * if listeners.len() > 1 { 0.4 } else { 0.8 };
    let lowpass_hz = 20_000.0 - (16_000.0 * (1.0 - gain)).max(0.0);
    Play { gain, pan, pitch: 1.0, lowpass_hz }
}

struct Voice { clip: Clip, pos: f64, play: Play, lp: f32, coef: f32 }

/// What the game sends to the mixer.
pub enum Command {
    Play(Clip, Play),
    StopAll,
    /// Master gain.
    Volume(f32),
}

/// Cheap to clone; send from the game thread.
#[derive(Clone)]
pub struct SfxSender(Sender<Command>);

impl SfxSender {
    pub fn play(&self, clip: &Clip, play: Play) { let _ = self.0.send(Command::Play(clip.clone(), play)); }
    pub fn stop_all(&self) { let _ = self.0.send(Command::StopAll); }
    pub fn volume(&self, gain: f32) { let _ = self.0.send(Command::Volume(gain)); }
}

/// The mixing side: lives in the audio callback (or a test).
pub struct SfxMixer {
    rx: Receiver<Command>,
    voices: Vec<Voice>,
    max_voices: usize,
    master: f32,
    sr: f32,
    /// Voices dropped because the pool was full (oldest is replaced).
    pub stolen: u64,
}

/// A connected sender and mixer. `sr` is the output rate.
pub fn sfx(max_voices: usize, sr: f32) -> (SfxSender, SfxMixer) {
    let (tx, rx) = channel();
    (SfxSender(tx), SfxMixer { rx, voices: Vec::with_capacity(max_voices), max_voices: max_voices.max(1), master: 1.0, sr, stolen: 0 })
}

impl SfxMixer {
    pub fn active(&self) -> usize { self.voices.len() }

    fn drain(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Command::Play(clip, play) => {
                    if self.voices.len() >= self.max_voices {
                        // Steal the voice nearest its end.
                        let k = (0..self.voices.len()).max_by(|&a, &b| {
                            let fa = self.voices[a].pos / self.voices[a].clip.0.len().max(1) as f64;
                            let fb = self.voices[b].pos / self.voices[b].clip.0.len().max(1) as f64;
                            fa.total_cmp(&fb)
                        }).unwrap();
                        self.voices.swap_remove(k);
                        self.stolen += 1;
                    }
                    let coef = 1.0 - (-2.0 * std::f32::consts::PI * play.lowpass_hz.min(self.sr * 0.45) / self.sr).exp();
                    self.voices.push(Voice { clip, pos: 0.0, play, lp: 0.0, coef });
                }
                Command::StopAll => self.voices.clear(),
                Command::Volume(g) => self.master = g,
            }
        }
    }

    /// Add into an interleaved buffer with `channels` channels (1 or more;
    /// the first two get the stereo image, others are left untouched).
    pub fn render_add(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        let rate = SAMPLE_RATE as f64 / self.sr as f64;
        let master = self.master;
        self.voices.retain_mut(|v| {
            let x = &v.clip.0;
            let (l, r) = if channels == 1 { (1.0, 0.0) } else {
                let a = (v.play.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
                (a.cos(), a.sin())
            };
            let g = v.play.gain * master;
            let step = rate * v.play.pitch.max(0.01) as f64;
            for f in out.chunks_exact_mut(channels) {
                let i = v.pos as usize;
                if i + 1 >= x.len() { return false; }
                let t = (v.pos - i as f64) as f32;
                let s = x[i] + (x[i + 1] - x[i]) * t;
                v.lp += (s - v.lp) * v.coef;
                let s = v.lp * g;
                if channels == 1 { f[0] += s; } else { f[0] += s * l; f[1] += s * r; }
                v.pos += step;
            }
            true
        });
    }

    /// Offline: mix `seconds` of stereo output (for tests and agents).
    pub fn render_stereo(&mut self, seconds: f32) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * self.sr) as usize * 2];
        for chunk in out.chunks_mut(1024) { self.render_add(chunk, 2); }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, rate: u32, bits: u16, format: u16, data: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data.len() as u32).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(format.to_le_bytes());
        b.extend(channels.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        b.extend((channels * bits / 8).to_le_bytes());
        b.extend(bits.to_le_bytes());
        b.extend(b"data");
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        b
    }

    #[test]
    fn decodes_pcm16_pcm24_and_float_and_resamples() {
        let pcm16: Vec<u8> = [16384i16, -16384].iter().cycle().take(4800).flat_map(|s| s.to_le_bytes()).collect();
        let c = decode_wav(&wav(1, 48_000, 16, 1, &pcm16)).unwrap();
        assert_eq!(c.0.len(), 4800);
        assert!((c.0[0] - 0.5).abs() < 1e-4 && (c.0[1] + 0.5).abs() < 1e-4);
        // 24-bit stereo at 44.1 kHz: L = +0.25, R = -0.25 averages to 0.
        let frame = [0x00u8, 0x00, 0x20, 0x00, 0x00, 0xE0];
        let pcm24: Vec<u8> = frame.iter().cycle().take(6 * 4410).copied().collect();
        let c = decode_wav(&wav(2, 44_100, 24, 1, &pcm24)).unwrap();
        assert_eq!(c.0.len(), 4800);
        assert!(c.0.iter().all(|s| s.abs() < 1e-6));
        let f: Vec<u8> = [0.75f32; 480].iter().flat_map(|s| s.to_le_bytes()).collect();
        let c = decode_wav(&wav(1, 48_000, 32, 3, &f)).unwrap();
        assert!((c.0[10] - 0.75).abs() < 1e-6);
        assert!(decode_wav(b"nope").is_err());
    }

    #[test]
    fn mixer_pans_pitches_ends_and_steals() {
        let clip = Clip(vec![0.5f32; 4800].into());
        let (tx, mut mix) = sfx(2, SAMPLE_RATE);
        tx.play(&clip, Play { pan: 1.0, ..Default::default() });
        let out = mix.render_stereo(0.05);
        let (l, r): (f32, f32) = (out.iter().step_by(2).map(|s| s.abs()).sum(), out.iter().skip(1).step_by(2).map(|s| s.abs()).sum());
        assert!(r > 100.0 * l.max(1e-6), "hard right: {l} {r}");
        // A 0.1 s clip at double pitch ends in 0.05 s.
        tx.play(&clip, Play::default().pitch(2.0));
        mix.render_stereo(0.06);
        assert_eq!(mix.active(), 0);
        for _ in 0..3 { tx.play(&clip, Play::default()); }
        mix.render_stereo(0.001);
        assert_eq!((mix.active(), mix.stolen), (2, 1));
    }

    #[test]
    fn spatial_uses_the_nearest_listener() {
        let ls = [Listener { pos: [0.0; 3], right: [1.0, 0.0, 0.0] }, Listener { pos: [100.0, 0.0, 0.0], right: [1.0, 0.0, 0.0] }];
        let near = spatial([95.0, 0.0, 0.0], &ls, 10.0);
        assert_eq!(near.gain, 1.0);
        assert!(near.pan < 0.0, "left of the second listener");
        let far = spatial([0.0, 0.0, 40.0], &ls[..1], 10.0);
        assert!((far.gain - 0.25).abs() < 1e-6 && far.lowpass_hz < 20_000.0);
    }
}
