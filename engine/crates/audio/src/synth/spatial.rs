//! Placing a mono voice in an N-speaker layout: stereo, quad, 5.1, 7.1 or
//! any ring of speakers. Pairwise amplitude panning (VBAP on the
//! horizontal ring) with spread toward all speakers, a low-frequency send,
//! inverse-distance gain and air absorption, for the nearest of several
//! listeners (split screen).
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    /// Degrees clockwise from straight ahead (right is +90).
    pub azimuth: f32,
    /// The low-frequency effects channel (gets the LFE send, no panning).
    #[serde(default)]
    pub lfe: bool,
}

/// Output channels in device order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout { pub name: String, pub speakers: Vec<Speaker> }

const fn sp(azimuth: f32) -> Speaker { Speaker { azimuth, lfe: false } }
const LFE: Speaker = Speaker { azimuth: 0.0, lfe: true };

impl Layout {
    pub fn mono() -> Self { Layout { name: "mono".into(), speakers: vec![sp(0.0)] } }
    pub fn stereo() -> Self { Layout { name: "stereo".into(), speakers: vec![sp(-30.0), sp(30.0)] } }
    pub fn quad() -> Self { Layout { name: "quad".into(), speakers: vec![sp(-45.0), sp(45.0), sp(-135.0), sp(135.0)] } }
    /// L R C LFE Ls Rs (the WAVE / SMPTE order).
    pub fn surround51() -> Self { Layout { name: "5.1".into(), speakers: vec![sp(-30.0), sp(30.0), sp(0.0), LFE, sp(-110.0), sp(110.0)] } }
    /// L R C LFE Lb Rb Ls Rs.
    pub fn surround71() -> Self { Layout { name: "7.1".into(), speakers: vec![sp(-30.0), sp(30.0), sp(0.0), LFE, sp(-150.0), sp(150.0), sp(-90.0), sp(90.0)] } }

    pub fn named(n: &str) -> Option<Self> {
        match n { "mono" => Some(Self::mono()), "stereo" => Some(Self::stereo()), "quad" => Some(Self::quad()), "5.1" => Some(Self::surround51()), "7.1" => Some(Self::surround71()), _ => None }
    }
    /// The usual layout for a device with this many channels.
    pub fn for_channels(n: usize) -> Self {
        match n { 0 | 1 => Self::mono(), 2 | 3 => Self::stereo(), 4 | 5 => Self::quad(), 6 | 7 => Self::surround51(), _ => Self::surround71() }
    }
    pub fn channels(&self) -> usize { self.speakers.len() }

    /// Gains per channel for a sound from `azimuth` degrees, `spread` 0
    /// (a point) .. 1 (all around), with `lfe` the LFE send. Power sums to 1
    /// over the full-range speakers.
    pub fn gains(&self, azimuth: f32, spread: f32, lfe: f32, out: &mut [f32]) {
        out.iter_mut().for_each(|g| *g = 0.0);
        // Up to 16 full-range speakers, sorted by azimuth, without allocating
        // (this runs per voice per block on the audio thread).
        let mut ring = [(0usize, 0.0f32); MAX_SPEAKERS];
        let mut n = 0;
        for (k, s) in self.speakers.iter().enumerate() {
            if s.lfe { if let Some(o) = out.get_mut(k) { *o = lfe; } continue; }
            if n < MAX_SPEAKERS { ring[n] = (k, s.azimuth); n += 1; }
        }
        let sorted = &mut ring[..n];
        sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
        match n {
            0 => {}
            1 => out[sorted[0].0] = 1.0,
            _ => {
                let mut az = wrap(azimuth);
                let span = sorted[n - 1].1 - sorted[0].1;
                // A frontal pair (stereo): fold sounds behind onto the front.
                if span < 180.0 {
                    if az > 90.0 { az = 180.0 - az; } else if az < -90.0 { az = -180.0 - az; }
                    az = az.clamp(sorted[0].1, sorted[n - 1].1);
                }
                // The neighbouring pair around az (wrapping round the back).
                let mut pair = (n - 1, 0);
                for k in 0..n {
                    let (a, b) = (sorted[k].1, sorted[(k + 1) % n].1);
                    let b = if k + 1 == n { b + 360.0 } else { b };
                    let x = if az < a { az + 360.0 } else { az };
                    if x >= a && x <= b { pair = (k, (k + 1) % n); break; }
                }
                let (a, b) = (sorted[pair.0].1, sorted[pair.1].1);
                let width = { let w = b - a; if w <= 0.0 { w + 360.0 } else { w } };
                let along = { let d = az - a; let d = if d < 0.0 { d + 360.0 } else { d }; (d / width).clamp(0.0, 1.0) };
                // Constant-power pan between the pair, blended toward all by spread.
                let t = along * std::f32::consts::FRAC_PI_2;
                let spread = spread.clamp(0.0, 1.0);
                let even = (1.0 / n as f32).sqrt();
                let mut power = 0.0;
                for (j, &(k, _)) in sorted.iter().enumerate() {
                    let g = if j == pair.0 { t.cos() } else if j == pair.1 { t.sin() } else { 0.0 };
                    let v = g * (1.0 - spread) + even * spread;
                    out[k] = v;
                    power += v * v;
                }
                let norm = power.sqrt().max(1e-6);
                for &(k, _) in sorted.iter() { out[k] /= norm; }
            }
        }
    }
}

/// Speakers a layout may pan across (the LFE does not count).
pub const MAX_SPEAKERS: usize = 16;

fn wrap(a: f32) -> f32 { (a + 180.0).rem_euclid(360.0) - 180.0 }

/// An ear: where it is and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ear { pub pos: [f32; 3], pub forward: [f32; 3], pub right: [f32; 3] }

/// How a placed sound behaves with distance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Falloff {
    /// Gain is reference / d beyond this distance (m).
    pub reference: f32,
    /// Silent beyond this.
    pub max: f32,
    /// Highs lost per 100 m (fraction of 20 kHz, air absorption as a low-pass).
    pub air: f32,
    /// Highest gain close in (1: full gain inside `reference`; Godot's
    /// inverse-distance model rises to +18 dB, 7.94).
    #[serde(default = "one")]
    pub near_max: f32,
    /// A linear fade to silence at this distance on top (0: none).
    #[serde(default)]
    pub fade: f32,
    /// Air absorption as an 8 kHz high shelf: this many dB cut far away,
    /// `-shelf * (1 - min(1, reference / d))` (0: none).
    #[serde(default)]
    pub shelf: f32,
    /// Gain lost when fully occluded (besides the low-pass).
    #[serde(default = "occlusion_cut")]
    pub occlusion_cut: f32,
}

fn one() -> f32 { 1.0 }
fn occlusion_cut() -> f32 { 0.6 }

impl Default for Falloff {
    fn default() -> Self { Falloff { reference: 4.0, max: 400.0, air: 0.8, near_max: 1.0, fade: 0.0, shelf: 0.0, occlusion_cut: 0.6 } }
}

impl Falloff {
    /// Godot 4's AudioStreamPlayer3D: inverse distance from `unit` m, capped
    /// at +18 dB, an 8 kHz shelf down to -12 dB far away, no distance
    /// low-pass, occlusion as a low-pass only.
    pub fn godot(unit: f32) -> Self {
        Falloff { reference: unit.max(0.01), max: 4000.0, air: 0.0, near_max: 7.943, fade: 0.0, shelf: 12.0, occlusion_cut: 0.0 }
    }
    /// With a linear fade to silence at `fade` m.
    pub fn fade(mut self, fade: f32) -> Self { self.fade = fade; self }
    /// Gain at distance d (before occlusion).
    pub fn gain(&self, d: f32) -> f32 {
        if d >= self.max { return 0.0; }
        let g = if self.near_max <= 1.0 { if d <= self.reference { 1.0 } else { self.reference / d } } else { (self.reference / (d + 1e-4 * self.reference)).min(self.near_max) };
        if self.fade > 0.0 { g * (1.0 - d / self.fade).max(0.0) } else { g }
    }
    /// The high-shelf gain in dB at distance d.
    pub fn shelf_db(&self, d: f32) -> f32 { if self.shelf <= 0.0 || d <= 0.0 { 0.0 } else { -self.shelf * (1.0 - (self.reference / d).min(1.0)) } }
}

/// Where a voice plays from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Place {
    /// Not positioned (music, UI): straight ahead, all speakers by `spread`.
    #[default]
    Head,
    /// At a point in the world.
    At([f32; 3]),
    /// Not positioned, on the front pair like a stereo panner (Godot 2D
    /// players): left `min(1, 1 - pan)`, right `min(1, 1 + pan)`, so 0 plays
    /// both at full gain.
    Pan(f32),
}

/// The result for one voice: channel gains, a low-pass for distance and
/// occlusion, and the delay sound takes to arrive.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement { pub gains: Vec<f32>, pub lowpass: f32, pub distance: f32 }

/// Place relative to the nearest ear. With several ears (split screen on one
/// set of speakers) directions mean little, so the image widens.
pub fn place(layout: &Layout, ears: &[Ear], at: Place, f: &Falloff, spread: f32, lfe: f32, occlusion: f32) -> Placement {
    let mut gains = vec![0.0; layout.channels()];
    let (lowpass, distance) = place_into(layout, ears, at, f, spread, lfe, occlusion, &mut gains);
    Placement { gains, lowpass, distance }
}

/// [`place`] into `gains` (one per channel) without allocating; returns
/// (low-pass Hz, distance m).
#[allow(clippy::too_many_arguments)]
pub fn place_into(layout: &Layout, ears: &[Ear], at: Place, f: &Falloff, spread: f32, lfe: f32, occlusion: f32, gains: &mut [f32]) -> (f32, f32) {
    let (az, d) = match at {
        Place::Head => (0.0, 0.0),
        Place::Pan(p) => {
            gains.iter_mut().for_each(|g| *g = 0.0);
            if gains.len() == 1 { gains[0] = 1.0; } else { gains[0] = (1.0 - p).clamp(0.0, 1.0); gains[1] = (1.0 + p).clamp(0.0, 1.0); }
            return (20_000.0, 0.0);
        }
        Place::At(p) => {
            let Some((d, e)) = ears.iter().map(|e| (dist(p, e.pos), e)).min_by(|a, b| a.0.total_cmp(&b.0)) else {
                layout.gains(0.0, 1.0, lfe, gains);
                return (20_000.0, 0.0);
            };
            let r = [p[0] - e.pos[0], p[1] - e.pos[1], p[2] - e.pos[2]];
            (dot(r, e.right).atan2(dot(r, e.forward)).to_degrees(), d)
        }
    };
    let spread = if ears.len() > 1 && matches!(at, Place::At(_)) { spread.max(0.5) } else { spread };
    // Very close sounds wrap around the head.
    let spread = if matches!(at, Place::At(_)) { spread.max((1.0 - d / 1.5).clamp(0.0, 1.0) * 0.8) } else { spread };
    layout.gains(az, spread, lfe, gains);
    let occ = occlusion.clamp(0.0, 1.0);
    let g = f.gain(d) * (1.0 - f.occlusion_cut * occ);
    for x in gains.iter_mut() { *x *= g; }
    // Occlusion: 20 kHz clear to 250 Hz behind walls, in log frequency.
    let occluded = 2f32.powf(20_000f32.log2() + (250f32.log2() - 20_000f32.log2()) * occ);
    let lowpass = (20_000.0 * (1.0 - f.air * d / 100.0).clamp(0.05, 1.0)).min(occluded);
    (lowpass.max(250.0), d)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 { ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() }
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

#[cfg(test)]
mod tests {
    use super::*;

    fn power(g: &[f32], l: &Layout) -> f32 { g.iter().zip(&l.speakers).filter(|(_, s)| !s.lfe).map(|(g, _)| g * g).sum() }

    #[test]
    fn panning_finds_the_right_speakers_at_constant_power() {
        let l = Layout::surround51();
        let mut g = vec![0.0; 6];
        l.gains(0.0, 0.0, 0.0, &mut g);
        assert!((g[2] - 1.0).abs() < 1e-5, "centre: {g:?}");
        l.gains(110.0, 0.0, 0.5, &mut g);
        assert!((g[5] - 1.0).abs() < 1e-5 && g[3] == 0.5, "right surround + LFE: {g:?}");
        l.gains(180.0, 0.0, 0.0, &mut g);
        assert!((g[4] - g[5]).abs() < 1e-5 && g[4] > 0.6, "behind: between the surrounds {g:?}");
        for az in (-180..180).step_by(7) {
            l.gains(az as f32, 0.3, 0.0, &mut g);
            assert!((power(&g, &l) - 1.0).abs() < 1e-4, "{az}");
        }
        let s = Layout::stereo();
        let mut g = vec![0.0; 2];
        s.gains(90.0, 0.0, 0.0, &mut g);
        assert!(g[1] > 0.99 && g[0] < 0.01, "hard right: {g:?}");
        s.gains(150.0, 0.0, 0.0, &mut g);
        assert!(g[1] > g[0], "behind-right folds to the right: {g:?}");
    }

    #[test]
    fn placement_follows_the_nearest_ear_and_distance() {
        let ear = |x: f32| Ear { pos: [x, 0.0, 0.0], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] };
        let l = Layout::quad();
        let p = place(&l, &[ear(0.0)], Place::At([0.0, 0.0, 10.0]), &Falloff::default(), 0.0, 0.0, 0.0);
        // Behind the listener: the rear pair, at 4/10 gain.
        assert!(p.gains[2] > 0.2 && p.gains[3] > 0.2 && p.gains[0] < 0.01, "{:?}", p.gains);
        assert!((power(&p.gains, &l).sqrt() - 0.4).abs() < 1e-3);
        let near = place(&l, &[ear(0.0), ear(100.0)], Place::At([99.0, 0.0, -1.0]), &Falloff::default(), 0.0, 0.0, 0.0);
        assert!(near.distance < 2.0);
        let occl = place(&l, &[ear(0.0)], Place::At([0.0, 0.0, -10.0]), &Falloff::default(), 0.0, 0.0, 1.0);
        assert!(occl.lowpass < 4000.0);
    }
}
