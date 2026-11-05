//! An honest frame counter.
//!
//! Averaging `1 / dt` lies: frames of 2 ms and 200 ms in turn average to
//! 252 "fps" while the eye sees 10. This averages the frame *time* and keeps
//! the worst frame of the last second, so a hitch shows as a hitch.

use std::time::Duration;

use web_time::Instant;

#[derive(Debug, Clone)]
pub struct FrameRate {
    last: Option<Instant>,
    mean_s: f32,
    worst_s: f32,
    window_worst_s: f32,
    window_start: Option<Instant>,
}

impl Default for FrameRate {
    fn default() -> Self { Self::new() }
}

impl FrameRate {
    pub const fn new() -> Self { Self { last: None, mean_s: 0.0, worst_s: 0.0, window_worst_s: 0.0, window_start: None } }

    /// Call once per drawn frame.
    pub fn tick(&mut self, now: Instant) {
        if let Some(last) = self.last { self.push((now - last).as_secs_f32(), now); }
        self.last = Some(now);
    }

    /// Adds one frame time in seconds (unclamped wall time).
    pub fn push(&mut self, dt: f32, now: Instant) {
        if dt <= 0.0 { return; }
        self.mean_s = if self.mean_s == 0.0 { dt } else { self.mean_s + (dt - self.mean_s) * 0.1 };
        self.window_worst_s = self.window_worst_s.max(dt);
        let start = *self.window_start.get_or_insert(now);
        if now - start >= Duration::from_secs(1) {
            self.worst_s = self.window_worst_s;
            self.window_worst_s = 0.0;
            self.window_start = Some(now);
        }
    }

    /// Frames per second from the mean frame time.
    pub fn fps(&self) -> f32 { if self.mean_s > 0.0 { 1.0 / self.mean_s } else { 0.0 } }

    /// The longest frame of the last full second (or of this one, if worse), in ms.
    pub fn worst_ms(&self) -> f32 { self.worst_s.max(self.window_worst_s) * 1000.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternating_hitches_read_as_the_rate_the_eye_sees() {
        let mut r = FrameRate::new();
        let mut now = Instant::now();
        for i in 0..400 {
            let dt = if i % 2 == 0 { 0.002 } else { 0.198 };
            now += Duration::from_secs_f32(dt);
            r.push(dt, now);
        }
        assert!((r.fps() - 10.0).abs() < 2.0, "fps {}", r.fps());
        assert!((r.worst_ms() - 198.0).abs() < 1.0, "worst {}", r.worst_ms());
    }
}
