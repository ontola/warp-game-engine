//! Dynamic graphics quality controller for Warp.
//!
//! Automatically scales render resolution, geometry LOD bias, grass reach/count,
//! shadows and post-processing across a calibrated ladder to hit and sustain
//! a target framerate (e.g. 60 or 120 FPS).
//!
//! The only input signal is the presented frame interval (dt).
//! Because vsync masks headroom, the controller steps down when frame time
//! exceeds the budget (with hysteresis and median filtering across a window),
//! and cautiously probes steps back up after extended calm periods.
use std::time::Duration;
use web_time::Instant;
use render::debug_settings::DebugRenderOptions;
use render::Lighting;

/// Fixed quality ladder: best/highest quality at index 0.
/// Rungs trade:
/// - 3D render resolution scale (0.67..1.00) while HUD remains native
/// - Grass distance and patch density (for outdoor meadow stages)
/// - Contact occlusion and bloom
///
/// Geometry LOD, near shadows and GI stay as the game set them: a coarser LOD
/// stripped card trees to their trunks and dropping near shadows drew a hard
/// shadow edge (terrain_fly playtest), which reads far worse than a softer
/// image. Below 0.67 the upscale itself turns to mush; a scene that misses
/// its target there needs cheaper content, not a blurrier frame.
pub const LADDER: [Rung; 8] = [
    Rung { scale: 1.00, lod_scale: 1.0, grass_distance: 1.00, grass_count: 1.00, near_shadows: true, contact: true, gi: true, bloom: true },
    Rung { scale: 0.95, lod_scale: 1.0, grass_distance: 1.00, grass_count: 1.00, near_shadows: true, contact: true, gi: true, bloom: true },
    Rung { scale: 0.90, lod_scale: 1.0, grass_distance: 0.80, grass_count: 1.00, near_shadows: true, contact: true, gi: true, bloom: true },
    Rung { scale: 0.85, lod_scale: 1.0, grass_distance: 0.60, grass_count: 0.90, near_shadows: true, contact: true, gi: true, bloom: true },
    Rung { scale: 0.80, lod_scale: 1.0, grass_distance: 0.45, grass_count: 0.80, near_shadows: true, contact: true, gi: true, bloom: true },
    Rung { scale: 0.75, lod_scale: 1.0, grass_distance: 0.30, grass_count: 0.60, near_shadows: true, contact: false, gi: true, bloom: true },
    Rung { scale: 0.70, lod_scale: 1.0, grass_distance: 0.20, grass_count: 0.40, near_shadows: true, contact: false, gi: true, bloom: true },
    Rung { scale: 0.67, lod_scale: 1.0, grass_distance: 0.10, grass_count: 0.20, near_shadows: true, contact: false, gi: true, bloom: false },
];

/// Targets above this are not chased: a 144 Hz display would otherwise
/// pull every scene to the bottom rung.
const MAX_TARGET_FPS: f64 = 120.0;

const WINDOW: usize = 24;
/// A median this far over the period means frames are being missed.
const SLOW: f64 = 1.12;
/// A median this close to the period counts as calm.
const CALM: f64 = 1.04;
const SETTLE: Duration = Duration::from_millis(600);
const FIRST_TRY_UP: Duration = Duration::from_secs(3);
const MAX_TRY_UP: Duration = Duration::from_secs(40);
/// Below this frame rate on the lowest rung for [`TOO_HEAVY_AFTER`], the
/// content is too heavy for this machine and the game is told
/// (`Game::content_too_heavy`). Resolution chases the display (up to 120 Hz);
/// visible content is only cut on a severe, sustained fall.
pub const CONTENT_TARGET_FPS: f64 = 45.0;
pub const TOO_HEAVY_AFTER: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rung {
    pub scale: f32,
    pub lod_scale: f32,
    pub grass_distance: f32,
    pub grass_count: f32,
    pub near_shadows: bool,
    pub contact: bool,
    pub gi: bool,
    pub bloom: bool,
}

pub struct AdaptiveQuality {
    target_ms: f64,
    level: usize,
    samples: Vec<f64>,
    changed: Instant,
    calm_since: Option<Instant>,
    try_up: Duration,
    probing: Option<Instant>,
    enabled: bool,
    /// Since when frames have been slow on the lowest rung.
    starved_since: Option<Instant>,
}

impl AdaptiveQuality {
    /// Create a new adaptive quality controller.
    /// `WARP_TARGET_FPS` overrides `target_fps` (0 turns it off).
    pub fn new(target_fps: Option<f32>, refresh_hz: Option<f64>, now: Instant) -> Option<Self> {
        let fps = match std::env::var("WARP_TARGET_FPS").ok().and_then(|v| v.parse::<f64>().ok()) {
            Some(v) if v <= 0.0 => return None,
            Some(v) => v,
            None => target_fps.map(f64::from).or(refresh_hz).filter(|hz| hz.is_finite() && *hz >= 20.0).unwrap_or(60.0).min(MAX_TARGET_FPS),
        };
        Some(Self {
            target_ms: 1000.0 / fps,
            level: 0,
            samples: Vec::with_capacity(WINDOW),
            changed: now,
            calm_since: None,
            try_up: FIRST_TRY_UP,
            probing: None,
            enabled: true,
            starved_since: None,
        })
    }

    pub fn enabled(&self) -> bool { self.enabled }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled { self.level = 0; self.starved_since = None; }
        }
    }

    /// Dynamically update the target FPS (e.g. from user settings or display change).
    pub fn set_target_fps(&mut self, target_fps: Option<f32>, refresh_hz: Option<f64>) {
        let fps = match std::env::var("WARP_TARGET_FPS").ok().and_then(|v| v.parse::<f64>().ok()) {
            Some(v) if v > 0.0 => v,
            _ => target_fps.map(f64::from).or(refresh_hz).filter(|hz| hz.is_finite() && *hz >= 20.0).unwrap_or(60.0).min(MAX_TARGET_FPS),
        };
        let new_target_ms = 1000.0 / fps;
        if (self.target_ms - new_target_ms).abs() > 0.1 {
            self.target_ms = new_target_ms;
            self.samples.clear();
            self.calm_since = None;
            self.try_up = FIRST_TRY_UP;
        }
    }

    pub fn target_fps(&self) -> f32 { (1000.0 / self.target_ms) as f32 }

    pub fn rung(&self) -> Rung {
        if self.enabled { LADDER[self.level] } else { LADDER[0] }
    }

    pub fn level(&self) -> usize { self.level }

    /// The lowest rung has run below [`CONTENT_TARGET_FPS`] for [`TOO_HEAVY_AFTER`]: true
    /// once, then the wait starts over (the game lowers its content, and the
    /// controller gets the same time to settle on the lighter scene).
    pub fn content_too_heavy(&mut self, now: Instant) -> bool {
        if !self.starved_since.is_some_and(|t| now.duration_since(t) >= TOO_HEAVY_AFTER) { return false; }
        self.starved_since = Some(now);
        true
    }

    /// Frames that are not measured (a hidden window): forget the samples
    /// and any slow streak, so they cannot lower quality later either.
    pub fn idle(&mut self) {
        self.samples.clear();
        self.starved_since = None;
        self.calm_since = None;
    }

    /// Feed one presented frame interval in milliseconds. Returns true when the rung changed.
    pub fn frame(&mut self, dt_ms: f64, now: Instant) -> bool {
        if !self.enabled || !dt_ms.is_finite() || dt_ms <= 0.0 { return false; }
        // A changed rung reallocates targets: skip its own transient hitch.
        if now.duration_since(self.changed) < SETTLE { self.samples.clear(); return false; }
        if self.samples.len() == WINDOW { self.samples.remove(0); }
        self.samples.push(dt_ms);
        if self.samples.len() < WINDOW / 2 { return false; }
        let mut sorted = self.samples.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let content_ms = self.target_ms.max(1000.0 / CONTENT_TARGET_FPS);
        if self.level == LADDER.len() - 1 && median > content_ms * SLOW { self.starved_since.get_or_insert(now); }
        else { self.starved_since = None; }
        if median > self.target_ms * SLOW {
            self.calm_since = None;
            if self.probing.take().is_some_and(|t| now.duration_since(t) < self.try_up) {
                self.try_up = (self.try_up * 2).min(MAX_TRY_UP);
            }
            // Far behind: skip a rung so heavy scenes stabilize rapidly.
            return self.step(if median > self.target_ms * 1.6 { 2 } else { 1 }, now);
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
        self.starved_since = None;
        self.changed = now;
        self.calm_since = None;
        self.samples.clear();
        true
    }

    /// The 3D render height for a given output window height, rounded to even pixels.
    pub fn render_height(&self, height: u32) -> u32 {
        let s = self.rung().scale;
        if s >= 1.0 { height } else { (((height as f32 * s) as u32) & !1).max(2) }
    }

    /// Apply geometry and lighting adjustments for this frame.
    pub fn apply_light(&self, mut light: Lighting) -> Lighting {
        let r = self.rung();
        light.lod_pixels *= r.lod_scale;
        if !r.near_shadows { light.near_shadows = 0.0; }
        if !r.contact { light.contact_occlusion = 0.0; light.reflection_occlusion = 0.0; }
        if !r.gi { light.gi = 0.0; }
        light
    }

    /// Scale render options (grass, bloom) down according to current rung.
    pub fn apply_options(&self, mut options: DebugRenderOptions) -> DebugRenderOptions {
        let r = self.rung();
        options.meadow_distance_scale = (options.meadow_distance_scale * r.grass_distance)
            .clamp(render::debug_settings::MEADOW_DISTANCE_SCALE_MIN, 1.0);
        options.meadow_density = (options.meadow_density * r.grass_count).clamp(0.0, 1.0);
        if !r.bloom {
            options.bloom_enabled = false;
        }
        options
    }

    pub fn metadata(&self) -> serde_json::Value {
        let r = self.rung();
        serde_json::json!({
            "target_fps": 1000.0 / self.target_ms,
            "level": self.level,
            "levels": LADDER.len(),
            "render_scale": r.scale,
            "lod_scale": r.lod_scale,
            "grass_distance": r.grass_distance,
            "grass_count": r.grass_count,
            "near_shadows": r.near_shadows,
            "contact": r.contact,
            "bloom": r.bloom,
        })
    }

    pub fn label(&self) -> String {
        let r = self.rung();
        format!(
            "{:.0} fps target: render {:.0}%, LOD x{:.2}, grass {:.0}%, shadows {}",
            1000.0 / self.target_ms,
            r.scale * 100.0,
            r.lod_scale,
            r.grass_distance * 100.0,
            if r.near_shadows { "full" } else { "fast" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(q: &mut AdaptiveQuality, start: &mut Instant, dt: f64, frames: usize) {
        for _ in 0..frames {
            *start += Duration::from_secs_f64(dt / 1000.0);
            q.frame(dt, *start);
        }
    }

    fn quality(now: Instant) -> AdaptiveQuality {
        AdaptiveQuality {
            target_ms: 1000.0 / 60.0,
            level: 0,
            samples: vec![],
            changed: now - SETTLE,
            calm_since: None,
            try_up: FIRST_TRY_UP,
            probing: None,
            enabled: true,
            starved_since: None,
        }
    }

    #[test]
    fn content_is_too_heavy_only_after_the_lowest_rung_keeps_missing() {
        let mut now = Instant::now();
        let mut q = quality(now);
        // 30 fps against 60: the ladder bottoms out first, then the wait runs.
        run(&mut q, &mut now, 33.0, 30 * 6);
        assert_eq!(q.level, LADDER.len() - 1);
        assert!(!q.content_too_heavy(now));
        run(&mut q, &mut now, 33.0, 30 * 6);
        assert!(q.content_too_heavy(now));
        assert!(!q.content_too_heavy(now), "once, then the wait starts over");
        // Making the target again resets it.
        run(&mut q, &mut now, 16.0, 60);
        run(&mut q, &mut now, 33.0, 30 * 2);
        assert!(!q.content_too_heavy(now));
    }

    #[test]
    fn idle_frames_of_a_hidden_window_never_count() {
        let mut now = Instant::now();
        let mut q = quality(now);
        q.target_ms = 1000.0 / 120.0;
        // Half a window of 20 fps while hidden, then shown and fast.
        run(&mut q, &mut now, 50.0, WINDOW / 2 - 1);
        q.idle();
        run(&mut q, &mut now, 8.0, WINDOW * 4);
        assert_eq!(q.level, 0);
        assert!(!q.content_too_heavy(now));
    }

    #[test]
    fn a_120_hz_target_cuts_resolution_but_content_only_below_60_fps() {
        let mut now = Instant::now();
        let mut q = quality(now);
        q.target_ms = 1000.0 / 120.0;
        // 80 fps on a 120 Hz screen: the resolution ladder bottoms out ...
        run(&mut q, &mut now, 12.5, 80 * 10);
        assert_eq!(q.level, LADDER.len() - 1);
        // ... but 80 fps (or 50) is plenty for content.
        assert!(!q.content_too_heavy(now));
        run(&mut q, &mut now, 20.0, 50 * 6);
        assert!(!q.content_too_heavy(now));
        run(&mut q, &mut now, 30.0, 33 * 6);
        assert!(q.content_too_heavy(now), "30 fps is not");
    }

    #[test]
    fn slow_frames_step_down_and_calm_frames_step_back_up() {
        let mut now = Instant::now();
        let mut q = quality(now);
        run(&mut q, &mut now, 25.0, 100);
        assert!(q.level >= 2, "level {}", q.level);
        let low = q.level;
        run(&mut q, &mut now, 15.0, 60 * 4);
        assert!(q.level < low);
    }

    #[test]
    fn calm_frames_at_the_top_stay_there() {
        let mut now = Instant::now();
        let mut q = quality(now);
        run(&mut q, &mut now, 16.0, 300);
        assert_eq!(q.level, 0);
    }

    #[test]
    fn a_failed_step_up_waits_longer_before_the_next() {
        let mut now = Instant::now();
        let mut q = quality(now);
        q.level = 3;
        run(&mut q, &mut now, 15.0, 60 * 4);
        assert_eq!(q.level, 2);
        run(&mut q, &mut now, 25.0, 40);
        assert_eq!(q.level, 3);
        assert_eq!(q.try_up, FIRST_TRY_UP * 2);
    }

    #[test]
    fn no_rung_coarsens_lod_or_drops_near_shadows_or_gi() {
        assert!(LADDER.iter().all(|r| r.lod_scale == 1.0 && r.near_shadows && r.gi && r.scale >= 0.67));
    }

    #[test]
    fn a_fast_display_is_chased_up_to_120_fps() {
        let q = AdaptiveQuality::new(None, Some(144.0), Instant::now()).unwrap();
        if std::env::var("WARP_TARGET_FPS").is_err() { assert_eq!(q.target_fps(), 120.0); }
    }

    #[test]
    fn render_height_keeps_even_pixels() {
        let now = Instant::now();
        let mut q = quality(now);
        assert_eq!(q.render_height(1080), 1080);
        q.level = LADDER.len() - 1;
        assert_eq!(q.render_height(1080), 722);
        q.level = 3;
        let h = q.render_height(1081);
        assert_eq!(h % 2, 0);
    }
}
