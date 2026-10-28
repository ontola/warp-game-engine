//! A feedback delay network reverb with one decorrelated output per
//! speaker, so the tail surrounds the listener in any layout.
const LINES: usize = 8;
/// Delay lengths in ms: mutually prime-ish, 30 to 75 ms.
const LENGTHS_MS: [f32; LINES] = [29.7, 37.1, 41.1, 43.7, 53.0, 59.9, 67.7, 73.1];

pub struct Reverb {
    lines: Vec<Vec<f32>>,
    pos: [usize; LINES],
    lp: [f32; LINES],
    g: [f32; LINES],
    damp: f32,
    level: f32,
    sr: f32,
    decay: f32,
}

impl Reverb {
    pub fn new(_channels: usize, sr: f32) -> Self {
        let lines = LENGTHS_MS.iter().map(|ms| vec![0.0; (ms * 0.001 * sr) as usize + 1]).collect();
        let mut r = Reverb { lines, pos: [0; LINES], lp: [0.0; LINES], g: [0.0; LINES], damp: 0.3, level: 0.0, sr, decay: 1.6 };
        r.set(1.6, 0.3, 0.0);
        r
    }

    /// Decay to -60 dB in seconds, high damping 0..1, return level.
    pub fn set(&mut self, decay: f32, damping: f32, level: f32) {
        self.decay = decay.max(0.05);
        self.damp = damping.clamp(0.0, 0.99);
        self.level = level.max(0.0);
        for (k, l) in self.lines.iter().enumerate() {
            self.g[k] = 10f32.powf(-3.0 * l.len() as f32 / (self.decay * self.sr));
        }
    }

    /// Mono send in, `channels` interleaved wet out (overwritten).
    pub fn run(&mut self, send: &[f32], wet: &mut [f32], channels: usize) {
        wet.iter_mut().for_each(|v| *v = 0.0);
        if self.level <= 0.0 { return; }
        let scale = self.level / (LINES as f32).sqrt();
        for (f, &x) in send.iter().enumerate() {
            let mut y = [0.0f32; LINES];
            for k in 0..LINES {
                let l = &self.lines[k];
                let v = l[self.pos[k]];
                self.lp[k] += (v - self.lp[k]) * (1.0 - self.damp);
                y[k] = self.lp[k];
            }
            // Hadamard mixing (fast Walsh-Hadamard, normalised).
            let mut h = y;
            let mut len = 1;
            while len < LINES {
                for i in (0..LINES).step_by(len * 2) {
                    for j in i..i + len { let (a, b) = (h[j], h[j + len]); h[j] = a + b; h[j + len] = a - b; }
                }
                len *= 2;
            }
            let norm = 1.0 / (LINES as f32).sqrt();
            for k in 0..LINES {
                let l = &mut self.lines[k];
                let p = self.pos[k];
                l[p] = x + h[k] * norm * self.g[k];
                self.pos[k] = (p + 1) % l.len();
            }
            // Each speaker hears its own sign pattern of the lines.
            for c in 0..channels {
                let mut v = 0.0;
                for (k, yk) in y.iter().enumerate() {
                    let sign = if ((c + 1) * (k + 3)).count_ones() % 2 == 0 { 1.0 } else { -1.0 };
                    v += yk * sign;
                }
                wet[f * channels + c] = v * scale;
            }
        }
    }
}
