//! Opt-in CPU wall-time attribution inside `Renderer` frames
//! (`KERN_CPU_SPANS=1`): named marks between phases of `frame_output`,
//! averaged per frame until read. No cost when off beyond one branch.
use std::sync::Mutex;
use web_time::Instant;

#[derive(Default)]
pub struct Collector { acc: Mutex<(Vec<(&'static str, f64)>, u64)> }

impl Collector {
    pub fn from_env() -> Option<Self> { (crate::switches::CPU_SPANS.on()).then(Self::default) }

    /// Average milliseconds per frame for each phase since the last call.
    pub fn take(&self) -> (Vec<(&'static str, f64)>, u64) {
        let mut a = self.acc.lock().unwrap();
        let frames = a.1.max(1);
        let out = a.0.iter().map(|&(n, t)| (n, t / frames as f64)).collect();
        let n = a.1;
        *a = (Vec::new(), 0);
        (out, n)
    }
}

pub struct Spans<'a> { c: Option<&'a Collector>, last: Instant, marks: Vec<(&'static str, f64)> }

impl<'a> Spans<'a> {
    pub fn start(c: Option<&'a Collector>) -> Self { Spans { c, last: Instant::now(), marks: Vec::new() } }
    /// Time since the previous mark goes to `name`.
    pub fn mark(&mut self, name: &'static str) {
        if self.c.is_none() { return; }
        let now = Instant::now();
        self.marks.push((name, (now - self.last).as_secs_f64() * 1000.0));
        self.last = now;
    }
}

impl Drop for Spans<'_> {
    fn drop(&mut self) {
        let Some(c) = self.c else { return };
        self.mark("tail");
        let mut a = c.acc.lock().unwrap();
        for &(n, t) in &self.marks {
            match a.0.iter_mut().find(|e| e.0 == n) { Some(e) => e.1 += t, None => a.0.push((n, t)) }
        }
        a.1 += 1;
    }
}
