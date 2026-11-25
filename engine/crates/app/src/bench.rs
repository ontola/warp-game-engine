//! A short, fixed measurement of what the game on screen costs on this
//! device, for the hardware-fit table (gamenight-internal
//! docs/strategy/hardware-fit.md): `bench=1` in the browser or Android
//! host's query (`bench=540,1080` picks the two 3D heights).
//!
//! Each game (the Game Room too) runs 10 s to warm up, 20 s at the low
//! height, 2 s to settle and 20 s at the high height, with adaptive quality
//! held still. Then one `BENCH {json}` line goes to the log: frame, CPU and
//! GPU medians per height, and a straight-line fit of the GPU time through
//! the two pixel counts, which splits it into a fixed part (shadows, culls,
//! raster: does not shrink with resolution) and a part per 3D pixel. A
//! switch to another game starts again for that game.
//!
//! Where the GPU has no timestamps (the browser), the fit uses frame times
//! and says so (`"fit_on": "frame"`); that only holds while the GPU is the
//! limit.

/// Seconds of each phase.
const WARM_S: f64 = 10.0;
const MEASURE_S: f64 = 20.0;
const SETTLE_S: f64 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub frame_ms: f32,
    /// Ticks + draw + encode on the CPU, without [`Sample::wait_ms`].
    pub cpu_ms: f32,
    /// Waiting for a free swapchain image (the GPU or the display is behind).
    pub wait_ms: f32,
    /// The frame's GPU time from timestamps, when there are any.
    pub gpu_ms: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase { Warm, Low, Settle, High, Done }

#[derive(Clone, Debug, PartialEq)]
struct Level { lines: u32, px: u64, frame: f32, frame_p95: f32, cpu: f32, wait: f32, gpu: Option<f32>, frames: usize }

#[derive(Clone, Debug)]
pub struct Bench {
    asked: [u32; 2],
    phase: Phase,
    clock: f64,
    samples: Vec<Sample>,
    levels: Vec<Level>,
    /// The two heights for the current screen.
    heights: [u32; 2],
}

/// `bench=1` (540 and 1080 lines) or `bench=H1,H2`; None for anything else.
pub fn parse(q: Option<&str>) -> Option<[u32; 2]> {
    let q = q?.trim();
    if q == "1" || q == "on" { return Some([540, 1080]); }
    let (a, b) = q.split_once(',')?;
    let (a, b) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
    (a >= 90 && b > a).then_some([a, b])
}

/// The two heights on a screen `h` lines high: the high one at most the
/// screen, the low one at most half of that.
fn fit_heights([lo, hi]: [u32; 2], h: u32) -> [u32; 2] {
    let hi = hi.min(h).max(2);
    [lo.min(hi / 2).max(1), hi]
}

/// Pixels of a 3D view `lines` high on a `size` screen (same aspect).
pub fn pixels(lines: u32, size: [u32; 2]) -> u64 {
    let w = (f64::from(lines) * f64::from(size[0]) / f64::from(size[1].max(1))).round() as u64;
    w * u64::from(lines)
}

fn median(mut v: Vec<f32>, q: f32) -> f32 {
    if v.is_empty() { return 0.0; }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * q) as usize]
}

impl Bench {
    pub fn new(asked: [u32; 2]) -> Bench {
        Bench { asked, phase: Phase::Warm, clock: 0.0, samples: Vec::new(), levels: Vec::new(), heights: asked }
    }

    /// From the query's `bench` value.
    pub fn from_query(q: Option<&str>) -> Option<Bench> { parse(q).map(Bench::new) }

    /// Start over, for the game just switched to.
    pub fn restart(&mut self) { *self = Bench::new(self.asked); }

    pub fn done(&self) -> bool { self.phase == Phase::Done }

    /// The 3D height to draw this frame at (None once done: adaptive
    /// quality takes over again).
    pub fn height(&mut self, size: [u32; 2]) -> Option<u32> {
        self.heights = fit_heights(self.asked, size[1]);
        match self.phase {
            Phase::Warm | Phase::Low => Some(self.heights[0]),
            Phase::Settle | Phase::High => Some(self.heights[1]),
            Phase::Done => None,
        }
    }

    /// One drawn frame. Returns the finished `BENCH {json}` line once.
    pub fn frame(&mut self, s: Sample, size: [u32; 2], about: &About) -> Option<String> {
        if self.phase == Phase::Done { return None; }
        self.clock += f64::from(s.frame_ms) / 1000.0;
        let limit = match self.phase { Phase::Warm => WARM_S, Phase::Low | Phase::High => MEASURE_S, Phase::Settle => SETTLE_S, Phase::Done => 0.0 };
        if matches!(self.phase, Phase::Low | Phase::High) { self.samples.push(s); }
        if self.clock < limit { return None; }
        self.clock = 0.0;
        self.phase = match self.phase {
            Phase::Warm => Phase::Low,
            Phase::Low => { self.close(0, size); Phase::Settle }
            Phase::Settle => Phase::High,
            Phase::High => { self.close(1, size); Phase::Done }
            Phase::Done => Phase::Done,
        };
        (self.phase == Phase::Done).then(|| format!("BENCH {}", self.report(size, about)))
    }

    fn close(&mut self, k: usize, size: [u32; 2]) {
        let s = std::mem::take(&mut self.samples);
        let gpu: Vec<f32> = s.iter().filter_map(|s| s.gpu_ms).filter(|g| *g > 0.0).collect();
        let lines = self.heights[k];
        self.levels.push(Level {
            lines,
            px: pixels(lines, size),
            frame: median(s.iter().map(|s| s.frame_ms).collect(), 0.5),
            frame_p95: median(s.iter().map(|s| s.frame_ms).collect(), 0.95),
            cpu: median(s.iter().map(|s| s.cpu_ms).collect(), 0.5),
            wait: median(s.iter().map(|s| s.wait_ms).collect(), 0.5),
            // Half the frames at least, or the timestamps are not to be trusted.
            gpu: (gpu.len() * 2 >= s.len() && !gpu.is_empty()).then(|| median(gpu, 0.5)),
            frames: s.len(),
        });
    }

    fn report(&self, size: [u32; 2], about: &About) -> serde_json::Value {
        let (lo, hi) = (&self.levels[0], &self.levels[1]);
        let (fit_on, a, b) = match (lo.gpu, hi.gpu) {
            (Some(a), Some(b)) => ("gpu", a, b),
            _ => ("frame", lo.frame, hi.frame),
        };
        let (fixed, per_px) = fit(lo.px, a, hi.px, b);
        let r1 = |x: f32| (x * 10.0).round() / 10.0;
        let level = |l: &Level| serde_json::json!({
            "lines": l.lines, "px": l.px, "frame_ms": r1(l.frame), "frame_p95_ms": r1(l.frame_p95),
            "cpu_ms": r1(l.cpu), "wait_ms": r1(l.wait), "gpu_ms": l.gpu.map(r1), "frames": l.frames,
        });
        serde_json::json!({
            "game": about.game,
            "gpu": about.gpu,
            "platform": about.platform,
            "tier": about.tier,
            "out": size,
            "players": about.players,
            "levels": [level(lo), level(hi)],
            "fit_on": fit_on,
            "gpu_fixed_ms": r1(fixed),
            "gpu_ns_per_px": r1(per_px),
            "cpu_ms": r1(lo.cpu.max(hi.cpu)),
            "memory_mb": about.memory_mb,
        })
    }
}

/// The line through (px1, ms1) and (px2, ms2): (ms at 0 pixels, ns per
/// pixel), both at least 0 (noise can tilt a flat line below).
pub fn fit(px1: u64, ms1: f32, px2: u64, ms2: f32) -> (f32, f32) {
    let dpx = px2 as f64 - px1 as f64;
    if dpx <= 0.0 { return (ms1.max(0.0), 0.0); }
    let per_px_ms = (f64::from(ms2) - f64::from(ms1)) / dpx;
    let per_px_ms = per_px_ms.max(0.0);
    let fixed = (f64::from(ms1) - per_px_ms * px1 as f64).max(0.0);
    (fixed as f32, (per_px_ms * 1e6) as f32)
}

/// What the line says about where it ran.
#[derive(Clone, Debug, Default)]
pub struct About {
    pub game: String,
    pub gpu: String,
    pub platform: &'static str,
    /// The `tier` query (low, medium, high), else the shell's own preset.
    pub tier: String,
    pub players: usize,
    pub memory_mb: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query() {
        assert_eq!(parse(Some("1")), Some([540, 1080]));
        assert_eq!(parse(Some("432,864")), Some([432, 864]));
        assert_eq!(parse(Some("864,432")), None);
        assert_eq!(parse(Some("0")), None);
        assert_eq!(parse(None), None);
    }

    #[test]
    fn heights_fit_the_screen() {
        assert_eq!(fit_heights([540, 1080], 1752), [540, 1080]);
        assert_eq!(fit_heights([540, 1080], 800), [400, 800]);
    }

    #[test]
    fn line_fit() {
        // 2 ms fixed + 14 ns/px.
        let (a, b) = (1_000_000u64, 3_000_000u64);
        let (fixed, ns) = fit(a, 2.0 + 14.0, b, 2.0 + 42.0);
        assert!((fixed - 2.0).abs() < 1e-3 && (ns - 14.0).abs() < 1e-3, "{fixed} {ns}");
        assert_eq!(fit(a, 10.0, b, 9.0), (10.0, 0.0));
    }

    #[test]
    fn runs_through_the_phases() {
        let size = [2800, 1752];
        let about = About { game: "Room".into(), gpu: "Adreno (TM) 740".into(), platform: "android", tier: "medium".into(), players: 1, memory_mb: 850 };
        let mut b = Bench::new([540, 1080]);
        let mut out = None;
        let mut frames = 0;
        while out.is_none() {
            let h = b.height(size).unwrap();
            let px = pixels(h, size) as f32;
            let gpu = 2.0 + 14.0e-6 * px;
            out = b.frame(Sample { frame_ms: gpu.max(8.0), cpu_ms: 6.0, wait_ms: 0.0, gpu_ms: Some(gpu) }, size, &about);
            frames += 1;
            assert!(frames < 100_000);
        }
        let line = out.unwrap();
        let v: serde_json::Value = serde_json::from_str(line.strip_prefix("BENCH ").unwrap()).unwrap();
        assert_eq!(v["fit_on"], "gpu");
        assert!((v["gpu_fixed_ms"].as_f64().unwrap() - 2.0).abs() < 0.11, "{v}");
        assert!((v["gpu_ns_per_px"].as_f64().unwrap() - 14.0).abs() < 0.11, "{v}");
        assert_eq!(v["levels"][1]["lines"], 1080);
        assert!(b.done() && b.height(size).is_none());
        b.restart();
        assert_eq!(b.height(size), Some(540));
    }
}
