//! Print the fluid engine's state through a few cycles: the debugging view.
use audio::fluid::{FluidEngine, Geometry};
fn main() {
    let rpm: f32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(3000.0);
    let sr = 48_000.0;
    let mut e = FluidEngine::new(Geometry::default(), &[0.0, 180.0, 540.0, 360.0], &[0.6; 4], 2.9, sr, 1);
    let mut last = -1.0;
    let n = (sr * 0.2) as usize;
    for i in 0..n {
        let y = e.sample(rpm, 1.0, 0.0, true, 0.0);
        let p = e.probe();
        if i > n / 2 && (p[0] / 30.0).floor() != (last / 30.0f32).floor() {
            println!("crank {:5.0}  cyl p {:8.0} T {:6.0} m {:6.1}mg  header p {:8.0}  coll p {:8.0}  exit p {:8.0} u {:7.1}  out {:+.3}", p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], y);
        }
        last = p[0];
    }
}
