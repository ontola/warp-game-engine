//! Adaptive quality: hold the display's frame rate by trading render
//! resolution and grass reach, one rung of a fixed ladder at a time.
//!
//! The only signal is the presented frame interval. With vsync a frame that
//! fits shows up at the refresh period, so there is no measure of headroom:
//! the controller steps up after a calm spell and, when that step misses,
//! waits twice as long before trying again.
use std::time::{Duration, Instant};

/// Best first. Render scale applies to both axes of the output; the grass
/// factors multiply the debug menu's own distance and patch-count values.
const LADDER: [Rung; 9] = [
    Rung { scale: 1.00, grass_distance: 1.00, grass_count: 1.00 },
    Rung { scale: 0.90, grass_distance: 1.00, grass_count: 1.00 },
    Rung { scale: 0.80, grass_distance: 0.90, grass_count: 1.00 },
    Rung { scale: 0.75, grass_distance: 0.80, grass_count: 0.90 },
    Rung { scale: 0.70, grass_distance: 0.70, grass_count: 0.80 },
    Rung { scale: 0.65, grass_distance: 0.60, grass_count: 0.70 },
    Rung { scale: 0.60, grass_distance: 0.50, grass_count: 0.60 },
    Rung { scale: 0.55, grass_distance: 0.40, grass_count: 0.55 },
    Rung { scale: 0.50, grass_distance: 0.30, grass_count: 0.50 },
];
const WINDOW: usize = 24;
/// A median this far over the period means frames are being missed.
const SLOW: f64 = 1.12;
/// A median this close to the period counts as calm.
const CALM: f64 = 1.04;
const SETTLE: Duration = Duration::from_millis(600);
const FIRST_TRY_UP: Duration = Duration::from_secs(3);
const MAX_TRY_UP: Duration = Duration::from_secs(40);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rung { pub scale: f32, pub grass_distance: f32, pub grass_count: f32 }

pub struct AdaptiveQuality {
    target_ms: f64,
    level: usize,
    samples: Vec<f64>,
    changed: Instant,
    calm_since: Option<Instant>,
    try_up: Duration,
    /// The last step up, while a miss right after it still counts against it.
    probing: Option<Instant>,
}

impl AdaptiveQuality {
    /// `SIDEWAYS_FPS_TARGET` overrides the refresh rate; 0 turns this off.
    pub fn new(refresh_hz: Option<f64>, now: Instant) -> Option<Self> {
        let fps = match std::env::var("SIDEWAYS_FPS_TARGET").ok().and_then(|v| v.parse::<f64>().ok()) {
            Some(v) if v <= 0.0 => return None,
            Some(v) => v,
            None => refresh_hz.filter(|hz| hz.is_finite() && *hz >= 24.0).unwrap_or(60.0),
        };
        Some(Self { target_ms: 1000.0 / fps, level: 0, samples: Vec::with_capacity(WINDOW), changed: now,
            calm_since: None, try_up: FIRST_TRY_UP, probing: None })
    }

    pub fn rung(&self) -> Rung { LADDER[self.level] }

    /// Feed one presented frame interval. Returns true when the rung changed.
    pub fn frame(&mut self, dt_ms: f64, now: Instant) -> bool {
        if !dt_ms.is_finite() || dt_ms <= 0.0 { return false; }
        // A changed rung reallocates targets: skip its own hitch.
        if now.duration_since(self.changed) < SETTLE { self.samples.clear(); return false; }
        if self.samples.len() == WINDOW { self.samples.remove(0); }
        self.samples.push(dt_ms);
        if self.samples.len() < WINDOW / 2 { return false; }
        let mut sorted = self.samples.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        if median > self.target_ms * SLOW {
            self.calm_since = None;
            if self.probing.take().is_some_and(|t| now.duration_since(t) < self.try_up) {
                self.try_up = (self.try_up * 2).min(MAX_TRY_UP);
            }
            return self.step(1, now);
        }
        if self.probing.is_some_and(|t| now.duration_since(t) >= self.try_up) {
            self.probing = None;
            self.try_up = FIRST_TRY_UP;
        }
        if median <= self.target_ms * CALM {
            let since = *self.calm_since.get_or_insert(now);
            if self.level > 0 && now.duration_since(since) >= self.try_up {
                self.probing = Some(now);
                return self.step(-1, now);
            }
        } else {
            self.calm_since = None;
        }
        false
    }

    fn step(&mut self, by: isize, now: Instant) -> bool {
        let next = (self.level as isize + by).clamp(0, LADDER.len() as isize - 1) as usize;
        if next == self.level { return false; }
        self.level = next;
        self.changed = now;
        self.calm_since = None;
        self.samples.clear();
        true
    }

    /// The render size for an output size, rounded to even pixels.
    pub fn render_size(&self, width: u32, height: u32) -> (u32, u32) {
        let s = self.rung().scale;
        let even = |v: u32| (((v as f32 * s) as u32) & !1).max(2);
        if s >= 1.0 { (width, height) } else { (even(width), even(height)) }
    }

    /// The debug menu's grass settings scaled down by the current rung.
    pub fn apply(&self, mut options: render::debug_settings::DebugRenderOptions) -> render::debug_settings::DebugRenderOptions {
        let r = self.rung();
        options.meadow_distance_scale = (options.meadow_distance_scale * r.grass_distance).clamp(0.25, 1.0);
        options.meadow_density = (options.meadow_density * r.grass_count).clamp(0.0, 1.0);
        options
    }

    pub fn metadata(&self) -> serde_json::Value {
        let r = self.rung();
        serde_json::json!({"target_fps": 1000.0 / self.target_ms, "level": self.level, "levels": LADDER.len(),
            "render_scale": r.scale, "grass_distance": r.grass_distance, "grass_count": r.grass_count})
    }

    pub fn label(&self) -> String {
        let r = self.rung();
        format!("{:.0} fps target: render {:.0}%, grass reach {:.0}%, grass count {:.0}%",
            1000.0 / self.target_ms, r.scale * 100.0, r.grass_distance * 100.0, r.grass_count * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(q: &mut AdaptiveQuality, start: &mut Instant, dt: f64, frames: usize) {
        for _ in 0..frames { *start += Duration::from_secs_f64(dt / 1000.0); q.frame(dt, *start); }
    }

    fn quality(now: Instant) -> AdaptiveQuality {
        AdaptiveQuality { target_ms: 1000.0 / 120.0, level: 0, samples: vec![], changed: now - SETTLE,
            calm_since: None, try_up: FIRST_TRY_UP, probing: None }
    }

    #[test]
    fn slow_frames_step_down_and_calm_frames_step_back_up() {
        let mut now = Instant::now();
        let mut q = quality(now);
        run(&mut q, &mut now, 16.0, 200);
        assert!(q.level >= 2, "level {}", q.level);
        let low = q.level;
        run(&mut q, &mut now, 8.3, 120 * 4);
        assert!(q.level < low);
    }

    #[test]
    fn calm_frames_at_the_top_stay_there() {
        let mut now = Instant::now();
        let mut q = quality(now);
        run(&mut q, &mut now, 8.33, 1200);
        assert_eq!(q.level, 0);
    }

    #[test]
    fn a_failed_step_up_waits_longer_before_the_next() {
        let mut now = Instant::now();
        let mut q = quality(now);
        q.level = 3;
        run(&mut q, &mut now, 8.3, 120 * 4);
        assert_eq!(q.level, 2);
        run(&mut q, &mut now, 12.0, 40);
        assert_eq!(q.level, 3);
        assert_eq!(q.try_up, FIRST_TRY_UP * 2);
    }

    #[test]
    fn render_size_keeps_even_pixels_and_native_at_the_top() {
        let now = Instant::now();
        let mut q = quality(now);
        assert_eq!(q.render_size(3840, 2160), (3840, 2160));
        q.level = LADDER.len() - 1;
        assert_eq!(q.render_size(3840, 2160), (1920, 1080));
        q.level = 3;
        let (w, h) = q.render_size(3841, 2161);
        assert!(w % 2 == 0 && h % 2 == 0);
    }
}
