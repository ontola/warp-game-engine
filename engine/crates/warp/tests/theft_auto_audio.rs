//! Check the shipped tailpipe against the raw four-cylinder synth at the same load.
use audio::{analysis, synth};
use std::sync::Arc;

const RATE: usize = 48_000;
const WINDOW: usize = 4096;
const MOTOR: &str = include_str!("../../../../games/warp-theft-auto/sounds/motor.luau");
const RAW: &str = r#"
    loop()
    return engine("inline_four", param("rpm", 900), param("throttle", 0), param("load", 0), 0, 1, 0)
"#;

fn exhaust_bands(source: &str, rpm: f32, throttle: f32, load: f32) -> (f32, f32) {
    let patch = synth::lua::load("motor", source)
        .unwrap()
        .remove(0)
        .compile()
        .unwrap();
    let (tx, mut mix) = synth::mixer(synth::Layout::mono(), 1, RATE as f32);
    tx.play(
        &Arc::new(patch),
        &[("rpm", rpm), ("throttle", throttle), ("load", load)],
        synth::Play::default(),
    );
    let output = mix.render_seconds(1.0);

    let mut body = 0.0;
    let mut hiss = 0.0;
    // Skip startup and compare band power over several complete windows.
    for samples in output[RATE / 2..].chunks_exact(WINDOW) {
        let spectrum = analysis::spectrum(samples, WINDOW);
        let power = |lo: usize, hi: usize| {
            spectrum[lo * WINDOW / RATE..hi * WINDOW / RATE]
                .iter()
                .map(|magnitude| magnitude * magnitude)
                .sum::<f32>()
        };
        body += power(35, 500);
        hiss += power(3500, 9000);
    }
    (body, hiss)
}

#[test]
fn street_muffler_preserves_engine_body_and_attenuates_hiss() {
    for (rpm, throttle, load) in [(900.0, 0.0, 0.08), (4200.0, 0.9, 0.8), (3200.0, 0.0, -0.4)] {
        let (raw_body, raw_hiss) = exhaust_bands(RAW, rpm, throttle, load);
        let (body, hiss) = exhaust_bands(MOTOR, rpm, throttle, load);
        assert!(raw_body > 0.0 && raw_hiss > 0.0);
        assert!(body > raw_body * 0.25, "lost low engine notes at {rpm} RPM");
        assert!(
            hiss / body < 0.25 * raw_hiss / raw_body,
            "excessive treble at {rpm} RPM"
        );
    }
}
