//! Tyre voice alone, per situation: level and how much of it is above 2 kHz.
use audio::analysis::{rms, spectrum};
use audio::tyres::{TyreControls, TyreVoice};
fn main() {
    let sr = 48_000.0;
    for (name, speed, slip) in [("rolling 20 m/s", 20.0, 0.3), ("cornering 20 m/s", 20.0, 2.0), ("sliding 20 m/s", 20.0, 6.0), ("braking hard 15 m/s", 15.0, 9.0), ("standing", 0.0, 0.0)] {
        let mut t = TyreVoice::new(sr, 1);
        t.set(TyreControls { speed, slip, contact: 1.0, loose: 1.0, soft: 0.0, wet: 0.0 });
        let mut out = vec![0.0; 48_000];
        for c in out.chunks_mut(240) {
            t.render_add(c, 0.5);
        }
        let x = &out[16_384..];
        let s = spectrum(x, 32_768);
        let df = sr / 32_768.0;
        let hi: f32 = s.iter().enumerate().filter(|(k, _)| *k as f32 * df > 2000.0).map(|(_, v)| v * v).sum();
        let all: f32 = s.iter().map(|v| v * v).sum::<f32>().max(1e-20);
        println!("{name:22} rms {:.3}  >2kHz {:5.1} dB", rms(x), 10.0 * (hi / all).max(1e-12).log10());
    }
}
