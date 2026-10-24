//! Where do the fluid model's output spikes come from? Prints the largest
//! output samples with the exit state around them.
use audio::fluid::{FluidEngine, Geometry};
fn main() {
    let rpm: f32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1500.0);
    let sr = 48_000.0;
    let mut e = FluidEngine::new(Geometry::default(), &[0.0, 180.0, 540.0, 360.0], &[0.62, 0.6, 0.6, 0.62], 3.3, sr, 1);
    let mut ys = Vec::new();
    for i in 0..(sr * 1.0) as usize {
        let y = e.sample(rpm, 1.0, 0.0, true, 0.05);
        let p = e.probe();
        if i > 4800 {
            ys.push((y.abs(), i, p));
        }
    }
    let rms = (ys.iter().map(|v| v.0 * v.0).sum::<f32>() / ys.len() as f32).sqrt();
    ys.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!("rms {rms:.4}");
    for (y, i, p) in ys.iter().take(8) {
        println!("|y| {y:.3} at {i}: crank {:.0} exit p {:.0} u {:.1}", p[0], p[6], p[7]);
    }
}
