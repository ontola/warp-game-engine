//! Offline tools for the headless loop: WAV files and spectra.

use std::f32::consts::PI;

/// 16-bit PCM WAV, interleaved `channels`.
pub fn write_wav(path: &std::path::Path, sr: u32, channels: u16, samples: &[f32]) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    let data = (samples.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&sr.to_le_bytes());
    b.extend_from_slice(&(sr * channels as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b)
}

/// In-place radix-2 FFT of (re, im); length a power of two.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Magnitude spectrum (Hann window) of `x[..n]`, n a power of two: n/2 bins
/// of sr/n Hz.
pub fn spectrum(x: &[f32], n: usize) -> Vec<f32> {
    let mut re: Vec<f32> = (0..n).map(|i| x.get(i).copied().unwrap_or(0.0) * (0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())).collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect()
}

/// Short-time spectra: one column of `n/2` magnitudes every `hop` samples.
pub fn spectrogram(x: &[f32], n: usize, hop: usize) -> Vec<Vec<f32>> {
    (0..x.len().saturating_sub(n) / hop).map(|k| spectrum(&x[k * hop..], n)).collect()
}

/// Root mean square.
pub fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn db(v: f32) -> f32 { if v > 0.0 { 20.0 * v.log10() } else { -120.0 } }

/// How a rendered sound measures (warp-audio render, live `render_sound`).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Levels {
    pub seconds: f32,
    pub peak_db: f32,
    pub rms_db: f32,
    /// The loudest 50 ms window: how hard it hits.
    pub punch_db: f32,
    pub centroid_hz: f32,
    /// Per speaker, for interleaved input with more than one channel.
    pub channels_db: Vec<f32>,
}

/// Measure interleaved `out` (`channels` wide) at `sr`; `mono` is its
/// downmix (see [`downmix`]).
pub fn levels(out: &[f32], channels: usize, sr: f32) -> Levels {
    let ch = channels.max(1);
    let mono = downmix(out, ch);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let per: Vec<f32> = (0..ch).map(|c| db(rms(&out.iter().skip(c).step_by(ch).copied().collect::<Vec<_>>()))).collect();
    let n = 4096.min(mono.len().next_power_of_two()).max(256);
    let s = spectrum(&mono[..n.min(mono.len())], n);
    let (num, den) = s.iter().enumerate().skip(1).fold((0.0, 0.0), |(a, b), (k, m)| (a + k as f32 * m, b + m));
    let centroid = if den > 0.0 { num / den * sr / n as f32 } else { 0.0 };
    let w = (0.05 * sr) as usize;
    let punch = mono.chunks(w.max(1)).map(rms).fold(0.0f32, f32::max);
    Levels { seconds: mono.len() as f32 / sr, peak_db: db(peak), rms_db: db(rms(&mono)), punch_db: db(punch), centroid_hz: centroid, channels_db: per }
}

/// The mean of the channels of interleaved `out`.
pub fn downmix(out: &[f32], channels: usize) -> Vec<f32> {
    let ch = channels.max(1);
    out.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
}

/// Cut trailing silence (under -90 dBFS), keeping 20 ms after the last sound.
pub fn trim_silence(out: &mut Vec<f32>, channels: usize, sr: f32) {
    let ch = channels.max(1);
    let frames = out.len() / ch;
    let last = (0..frames).rev().find(|&f| out[f * ch..(f + 1) * ch].iter().any(|v| v.abs() > 3e-5)).map_or(0, |f| f + 1);
    out.truncate((last + (0.02 * sr) as usize).min(frames) * ch);
}

/// A spectrogram as text, so a sound can be judged without listening:
/// rows are log-spaced bands from 40 Hz to 20 kHz (high at the top), 72
/// columns of time, brightness relative to the loudest cell over 60 dB.
pub fn text_spectrogram(x: &[f32], sr: f32) -> String {
    if x.len() < 512 { return String::new(); }
    let n = 1024;
    let cols = 72usize;
    let hop = (x.len().saturating_sub(n) / cols).max(64);
    let frames = spectrogram(x, n, hop);
    if frames.is_empty() { return String::new(); }
    let bands: Vec<f32> = (0..=14).map(|k| 40.0 * (20000.0f32 / 40.0).powf(k as f32 / 14.0)).collect();
    let mut grid = vec![vec![0.0f32; frames.len()]; bands.len() - 1];
    for (c, f) in frames.iter().enumerate() {
        for b in 0..bands.len() - 1 {
            let (lo, hi) = ((bands[b] / sr * n as f32) as usize, ((bands[b + 1] / sr * n as f32) as usize).max((bands[b] / sr * n as f32) as usize + 1));
            grid[b][c] = f[lo.min(f.len() - 1)..hi.min(f.len())].iter().map(|m| m * m).sum::<f32>();
        }
    }
    let max = grid.iter().flatten().fold(0.0f32, |m, v| m.max(*v));
    if max <= 0.0 { return "(silent)\n".into(); }
    let ramp = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
    let mut s = String::new();
    for b in (0..grid.len()).rev() {
        let label = bands[b + 1];
        s += &format!("{:>6} |", if label >= 1000.0 { format!("{:.1}k", label / 1000.0) } else { format!("{label:.0}") });
        for v in &grid[b] {
            let d = 10.0 * (v / max).max(1e-12).log10();
            let k = (((d + 60.0) / 60.0) * (ramp.len() - 1) as f32).round().clamp(0.0, (ramp.len() - 1) as f32) as usize;
            s.push(ramp[k]);
        }
        s.push('\n');
    }
    s += &format!("       +{}\n        0 s{:>w$}\n", "-".repeat(grid[0].len()), format!("{:.2} s", x.len() as f32 / sr), w = grid[0].len().saturating_sub(3));
    s
}
