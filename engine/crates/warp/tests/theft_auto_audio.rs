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
fn authored_muffler_geometry_survives_patch_serialization_and_edits() {
    let load = |source: &str| synth::lua::load("motor", source).unwrap().remove(0);
    let production = load(MOTOR);
    let encoded = serde_json::to_string(&production).unwrap();
    let restored: synth::Patch = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, production);

    let longer = MOTOR.replace("chamber_length = 0.46", "chamber_length = 0.64");
    assert_ne!(longer, MOTOR);
    let params = [("rpm", 3200.0), ("throttle", 0.8), ("load", 0.7)];
    let render = |p: &synth::Patch| p.voice(&params, 41, RATE as f32).render_all(0.4);
    let original = render(&production);
    let modified = render(&load(&longer));
    assert!(analysis::rms(&original) > 0.001 && analysis::rms(&modified) > 0.001);
    let difference = original
        .iter()
        .zip(&modified)
        .map(|(a, b)| a - b)
        .collect::<Vec<_>>();
    assert!(analysis::rms(&difference) > 0.03 * analysis::rms(&original));
    assert_eq!(original, render(&restored));

    for invalid in [
        MOTOR.replace("chamber_length = 0.46", "chamber_length = -1"),
        MOTOR.replace("packing = 0.78", "packing = 0/0"),
        MOTOR.replace("chamber_diameter", "chamber_radius"),
        MOTOR.replace("inline_four", "nonexistent_car"),
    ] {
        assert!(synth::lua::load("motor", &invalid).is_err());
    }
}

#[test]
fn street_muffler_preserves_engine_body_and_attenuates_hiss() {
    for (rpm, throttle, load) in [(900.0, 0.0, 0.08), (4200.0, 0.9, 0.8), (3200.0, 0.0, -0.4)] {
        let (raw_body, raw_hiss) = exhaust_bands(RAW, rpm, throttle, load);
        let (body, hiss) = exhaust_bands(MOTOR, rpm, throttle, load);
        assert!(raw_body > 0.0 && raw_hiss > 0.0);
        if load >= 0.0 {
            assert!(body > raw_body * 0.25, "lost low engine notes at {rpm} RPM");
        } else {
            // At fixed RPM, lifting off leaves audible pumping pulses while
            // the combustion component falls with the available torque.
            let (under_load, _) = exhaust_bands(MOTOR, rpm, 0.9, 0.8);
            assert!(
                body > under_load * 0.003 && body < under_load * 0.5,
                "overrun band power {body}, under load {under_load}"
            );
        }
        assert!(
            hiss / body < 0.25 * raw_hiss / raw_body,
            "excessive treble at {rpm} RPM"
        );
    }
}
