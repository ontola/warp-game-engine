//! The controllers on the coffee table: one for each connected controller
//! that hasn't joined, each dropping in from above as it connects.
use std::f32::consts::PI;

/// The most controllers the table has room for.
pub const MAX: usize = 4;
/// Seconds of the fall, then of the little bounce on landing.
const FALL: f32 = 0.7;
const BOUNCE: f32 = 0.3;
/// How high above the table a new controller starts: above the picture.
const HEIGHT: f32 = 10.0;
/// Between controllers that connect at once.
const STAGGER: f32 = 0.15;

#[derive(Default, Debug)]
pub struct TablePads {
    /// Controllers on the table; None until the host has said.
    count: Option<usize>,
    /// Seconds since each pad started falling (negative: still waiting to);
    /// None: not on the table.
    fall: [Option<f32>; MAX],
    /// The host counts them itself (the browser) instead of the lobby
    /// counting the unjoined tokens in controller frames.
    pub external: bool,
}

impl TablePads {
    /// `n` controllers not joined are connected. The first count puts them
    /// down as they are; later ones drop the new pads from above.
    pub fn set(&mut self, n: usize) {
        let n = n.min(MAX);
        let old = match self.count {
            Some(old) => old,
            None => {
                for (i, f) in self.fall.iter_mut().enumerate() { *f = (i < n).then_some(FALL + BOUNCE); }
                self.count = Some(n);
                return;
            }
        };
        if n == old { return; }
        for (i, f) in self.fall.iter_mut().enumerate() {
            if i >= n { *f = None; } else if i >= old { *f = Some(-((i - old) as f32) * STAGGER); }
        }
        self.count = Some(n);
    }

    /// Advances the falls; true when a pad lands this frame (the table thumps).
    pub fn update(&mut self, delta: f32) -> bool {
        let mut landed = false;
        for t in self.fall.iter_mut().flatten() {
            if *t >= FALL + BOUNCE { continue; }
            landed |= *t < FALL && *t + delta >= FALL;
            *t += delta;
        }
        landed
    }

    /// Each pad's height above its spot on the table; None: not there. The
    /// room's default (two pads down) until the host has said.
    pub fn heights(&self) -> Option<[Option<f32>; MAX]> {
        self.count?;
        Some(self.fall.map(|f| f.and_then(height)))
    }
}

fn height(t: f32) -> Option<f32> {
    if t < 0.0 { return None; }
    Some(if t < FALL {
        HEIGHT * (1.0 - (t / FALL).powi(2))
    } else if t < FALL + BOUNCE {
        0.1 * (PI * (t - FALL) / BOUNCE).sin()
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_follow_the_count_and_new_ones_drop_in() {
        let mut t = TablePads::default();
        assert_eq!(t.heights(), None);
        t.set(1);
        assert_eq!(t.heights().unwrap(), [Some(0.0), None, None, None]);
        t.set(3);
        let h = t.heights().unwrap();
        assert_eq!(h[0], Some(0.0));
        assert!(h[1].unwrap() > 3.0, "the new one starts high: {h:?}");
        assert_eq!(h[2], None, "the second new one waits a moment");
        let mut landed = false;
        for _ in 0..90 { landed |= t.update(1.0 / 60.0); }
        assert!(landed);
        assert_eq!(t.heights().unwrap(), [Some(0.0), Some(0.0), Some(0.0), None]);
        t.set(9);
        assert!(t.heights().unwrap()[3].is_some());
        t.set(0);
        assert_eq!(t.heights().unwrap(), [None; MAX]);
    }
}
