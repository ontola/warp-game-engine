//! Tweens for presentation: a value moving from one to another over time
//! with Godot's transitions and eases (boxes sliding onto a shelf, a cover
//! popping in), and `Follow` for values that must ease, never jump (a
//! loading bar). Simulation stays in ticks; drive these with frame time.
use glam::{Vec2, Vec3, Vec4};

/// Godot's `Tween.TRANS_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Trans { #[default] Linear, Sine, Quad, Cubic, Quart, Quint, Expo, Circ, Back, Elastic, Bounce }
/// Godot's `Tween.EASE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ease { In, #[default] Out, InOut, OutIn }

/// The eased fraction of `t` in 0..1 (Back and Elastic overshoot).
pub fn ease(t: f32, trans: Trans, ease: Ease) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let i = |t: f32| ease_in(t, trans);
    let o = |t: f32| 1.0 - ease_in(1.0 - t, trans);
    match ease {
        Ease::In => i(t),
        Ease::Out => o(t),
        Ease::InOut => if t < 0.5 { i(t * 2.0) * 0.5 } else { 0.5 + o(t * 2.0 - 1.0) * 0.5 },
        Ease::OutIn => if t < 0.5 { o(t * 2.0) * 0.5 } else { 0.5 + i(t * 2.0 - 1.0) * 0.5 },
    }
}

fn ease_in(t: f32, trans: Trans) -> f32 {
    use std::f32::consts::PI;
    match trans {
        Trans::Linear => t,
        Trans::Sine => 1.0 - (t * PI / 2.0).cos(),
        Trans::Quad => t * t,
        Trans::Cubic => t * t * t,
        Trans::Quart => t.powi(4),
        Trans::Quint => t.powi(5),
        Trans::Expo => if t <= 0.0 { 0.0 } else { 2f32.powf(10.0 * (t - 1.0)) - 0.001 * (1.0 - t) },
        Trans::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
        Trans::Back => { let s = 1.70158; t * t * ((s + 1.0) * t - s) }
        Trans::Elastic => if t <= 0.0 || t >= 1.0 { t } else { -(2f32.powf(10.0 * (t - 1.0))) * ((t - 1.075) * (2.0 * PI) / 0.3).sin() },
        Trans::Bounce => 1.0 - bounce_out(1.0 - t),
    }
}

fn bounce_out(t: f32) -> f32 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d { n * t * t }
    else if t < 2.0 / d { let t = t - 1.5 / d; n * t * t + 0.75 }
    else if t < 2.5 / d { let t = t - 2.25 / d; n * t * t + 0.9375 }
    else { let t = t - 2.625 / d; n * t * t + 0.984375 }
}

pub trait Lerp: Copy { fn lerp(self, to: Self, t: f32) -> Self; }
impl Lerp for f32 { fn lerp(self, to: Self, t: f32) -> Self { self + (to - self) * t } }
impl Lerp for Vec2 { fn lerp(self, to: Self, t: f32) -> Self { Vec2::lerp(self, to, t) } }
impl Lerp for Vec3 { fn lerp(self, to: Self, t: f32) -> Self { Vec3::lerp(self, to, t) } }
impl Lerp for Vec4 { fn lerp(self, to: Self, t: f32) -> Self { Vec4::lerp(self, to, t) } }
impl Lerp for [f32; 4] { fn lerp(self, to: Self, t: f32) -> Self { std::array::from_fn(|i| self[i] + (to[i] - self[i]) * t) } }

/// One value moving from `from` to `to` over `duration` seconds after `delay`.
#[derive(Clone, Copy, Debug)]
pub struct Tween<T: Lerp> { pub from: T, pub to: T, pub start: f32, pub delay: f32, pub duration: f32, pub trans: Trans, pub ease: Ease }

impl<T: Lerp> Tween<T> {
    /// Starts at `now` (any clock in seconds, the same one passed to `at`).
    pub fn new(from: T, to: T, now: f32, duration: f32, trans: Trans, ease: Ease) -> Self { Self { from, to, start: now, delay: 0.0, duration, trans, ease } }
    pub fn delayed(mut self, delay: f32) -> Self { self.delay = delay; self }
    /// A constant value (a finished tween).
    pub fn hold(value: T) -> Self { Self::new(value, value, 0.0, 0.0, Trans::Linear, Ease::Out) }
    pub fn progress(&self, now: f32) -> f32 { if self.duration <= 0.0 { if now >= self.start + self.delay { 1.0 } else { 0.0 } } else { ((now - self.start - self.delay) / self.duration).clamp(0.0, 1.0) } }
    pub fn at(&self, now: f32) -> T { self.from.lerp(self.to, ease(self.progress(now), self.trans, self.ease)) }
    pub fn done(&self, now: f32) -> bool { self.progress(now) >= 1.0 }
    /// Retarget from the current value, so a change mid-flight never jumps.
    pub fn retarget(&mut self, to: T, now: f32, duration: f32) { *self = Self { from: self.at(now), to, start: now, delay: 0.0, duration, ..*self }; }
}

/// A value chasing a target without jumps: exponential approach with a
/// half-life, plus an optional speed limit (units per second).
#[derive(Clone, Copy, Debug)]
pub struct Follow { pub value: f32, pub half_life: f32, pub max_speed: f32 }

impl Follow {
    pub fn new(value: f32, half_life: f32) -> Self { Self { value, half_life, max_speed: f32::INFINITY } }
    pub fn update(&mut self, target: f32, dt: f32) -> f32 {
        let k = if self.half_life <= 0.0 { 1.0 } else { 1.0 - 0.5f32.powf(dt.max(0.0) / self.half_life) };
        let step = ((target - self.value) * k).clamp(-self.max_speed * dt, self.max_speed * dt);
        self.value += step;
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_hit_their_ends_and_godot_shapes() {
        for trans in [Trans::Linear, Trans::Sine, Trans::Quad, Trans::Cubic, Trans::Quart, Trans::Quint, Trans::Expo, Trans::Circ, Trans::Back, Trans::Elastic, Trans::Bounce] {
            for e in [Ease::In, Ease::Out, Ease::InOut, Ease::OutIn] {
                assert!(ease(0.0, trans, e).abs() < 2e-3 && (ease(1.0, trans, e) - 1.0).abs() < 2e-3, "{trans:?} {e:?}");
            }
        }
        assert!((ease(0.5, Trans::Cubic, Ease::Out) - 0.875).abs() < 1e-6);
        assert!((0..100).any(|i| ease(i as f32 / 100.0, Trans::Back, Ease::Out) > 1.05), "back overshoots");
        assert_eq!(ease(0.25, Trans::Linear, Ease::InOut), 0.25);
    }

    #[test]
    fn tweens_delay_finish_and_retarget_without_jumping() {
        let mut t = Tween::new(0.0f32, 10.0, 1.0, 2.0, Trans::Linear, Ease::Out).delayed(0.5);
        assert_eq!(t.at(1.4), 0.0);
        assert_eq!(t.at(2.5), 5.0);
        assert!(t.done(3.5) && t.at(9.0) == 10.0);
        t.retarget(0.0, 2.5, 1.0);
        assert_eq!(t.at(2.5), 5.0, "continues from where it was");
        assert_eq!(t.at(3.5), 0.0);
        let mut f = Follow { max_speed: 50.0, ..Follow::new(0.0, 0.1) };
        assert_eq!(f.update(100.0, 0.1), 5.0, "speed-limited");
        for _ in 0..200 { f.update(100.0, 0.1); }
        assert!((f.value - 100.0).abs() < 1e-3);
    }
}
