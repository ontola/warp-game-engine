use super::*;
use crate::analysis::{rms, spectrum};
use std::sync::Arc;

const SR: f32 = 48_000.0;

fn node(unit: Unit, inputs: Vec<In>) -> Node { Node { unit, inputs } }

/// A plucked metal hit: noise burst into a modal bank, with a size parameter.
fn bell() -> Patch {
    Patch {
        name: "bell".into(),
        params: vec![("freq".into(), 600.0)],
        nodes: vec![
            node(Unit::Noise { color: NoiseColor::White }, vec![]),
            node(Unit::Env { points: vec![Point { time: 0.001, level: 1.0, curve: 0.0 }, Point { time: 0.01, level: 0.0, curve: 4.0 }] }, vec![]),
            node(Unit::Mul, vec![In::N(0), In::N(1)]),
            node(Unit::Modal { modes: vec![Mode { ratio: 1.0, decay: 0.6, gain: 1.0 }, Mode { ratio: 2.76, decay: 0.3, gain: 0.5 }] }, vec![In::N(2), In::P("freq".into())]),
        ],
        out: 3,
        ..Default::default()
    }
}

fn peak_hz(x: &[f32]) -> f32 {
    let n = 8192;
    let s = spectrum(&x[..n.min(x.len())], n);
    let k = (1..s.len()).max_by(|&a, &b| s[a].total_cmp(&s[b])).unwrap();
    k as f32 * SR / n as f32
}

#[test]
fn oscillators_hit_their_frequency_and_saw_is_band_limited() {
    let p = Patch { name: "o".into(), nodes: vec![node(Unit::Osc { shape: Shape::Sine }, vec![In::K(440.0)])], out: 0, max_length: 0.5, ..Default::default() };
    p.validate().unwrap();
    let x = p.voice(&[], 1, SR).render_all(0.5);
    assert!((peak_hz(&x) - 440.0).abs() < 6.0);
    assert!((rms(&x) - 0.707).abs() < 0.01);
    // A saw at 5 kHz: little energy folded back below 5 kHz (aliasing).
    let p = Patch { name: "s".into(), nodes: vec![node(Unit::Osc { shape: Shape::Saw }, vec![In::K(5000.0)])], out: 0, max_length: 0.2, ..Default::default() };
    let x = p.voice(&[], 1, SR).render_all(0.2);
    let s = spectrum(&x[..8192], 8192);
    let below: f32 = s[10..800].iter().sum();
    let total: f32 = s[10..].iter().sum();
    assert!(below / total < 0.05, "{}", below / total);
}

#[test]
fn struck_resonators_ring_at_their_modes_and_voices_end() {
    // Struck by a smooth click (a noise burst's spectrum is random).
    let mut p = bell();
    p.nodes[2].inputs = vec![In::K(1.0), In::N(1)];
    p.validate().unwrap();
    let mut v = p.voice(&[("freq", 800.0)], 3, SR);
    let x = v.render_all(5.0);
    assert!((peak_hz(&x) - 800.0).abs() < 8.0, "{}", peak_hz(&x));
    assert!(x.len() < (3.0 * SR) as usize, "ended after the ring died: {} s", x.len() as f32 / SR);
    // Same seed, same sound; another seed, another burst.
    let p = bell();
    let a = p.voice(&[], 3, SR).render_all(0.3);
    assert_eq!(a, p.voice(&[], 3, SR).render_all(0.3));
    assert_ne!(a, p.voice(&[], 4, SR).render_all(0.3));
}

#[test]
fn filters_cut_and_parameters_glide() {
    let p = Patch {
        name: "f".into(), params: vec![("cut".into(), 200.0)], max_length: 1.0,
        nodes: vec![node(Unit::Noise { color: NoiseColor::White }, vec![]), node(Unit::Filter { mode: FilterMode::Low }, vec![In::N(0), In::P("cut".into())])],
        out: 1, ..Default::default()
    };
    let dark = rms(&p.voice(&[], 1, SR).render_all(0.5));
    let bright = rms(&p.voice(&[("cut", 8000.0)], 1, SR).render_all(0.5));
    assert!(bright > dark * 3.0, "{dark} {bright}");
    let mut v = p.voice(&[], 1, SR);
    let mut b = [0.0; BLOCK];
    v.render(&mut b);
    v.set("cut", 8000.0);
    v.render(&mut b);
    assert!(b.iter().all(|s| s.is_finite()));
}

#[test]
fn validation_catches_cycles_and_unknown_params() {
    let mut p = bell();
    p.nodes[2].inputs = vec![In::N(0), In::N(3)];
    assert!(p.validate().unwrap_err().contains("comes later"));
    let mut p = bell();
    p.nodes[3].inputs[1] = In::P("nope".into());
    assert!(p.validate().unwrap_err().contains("unknown parameter"));
}

#[test]
fn the_mixer_places_voices_in_51_and_the_reverb_fills_every_speaker() {
    let (tx, mut mix) = mixer(Layout::surround51(), 32, SR);
    tx.ears(vec![Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
    // Behind and to the right: the right surround.
    let p = Arc::new(bell());
    tx.play(&p, &[], Play { place: Place::At([4.0, 0.0, 1.5]), ..Default::default() });
    let out = mix.render_seconds(0.5);
    let ch = |c: usize| rms(&out.iter().skip(c).step_by(6).copied().collect::<Vec<_>>());
    let e: Vec<f32> = (0..6).map(ch).collect();
    assert!(e[5] > 5.0 * e[0].max(e[1]).max(e[2]), "right surround leads: {e:?}");
    assert_eq!(e[3], 0.0, "no LFE send");
    // With the room on, every speaker gets the tail.
    tx.room(2.0, 0.3, 0.8);
    tx.play(&p, &[], Play { place: Place::At([0.0, 0.0, -3.0]), reverb: 1.0, ..Default::default() });
    let out = mix.render_seconds(0.6);
    for c in [0usize, 1, 2, 4, 5] {
        let tail: Vec<f32> = out.iter().skip(c).step_by(6).skip((0.4 * SR) as usize).copied().collect();
        assert!(rms(&tail) > 1e-4, "channel {c} silent");
    }
    // Stereo device on a 5.1 mix: channels fold, nothing lost.
    let mut stereo = vec![0.0; 2 * 4800];
    mix.render(&mut stereo, 2);
    assert!(stereo.iter().all(|v| v.abs() <= 1.0));
}

#[test]
fn sound_travels_at_343_and_the_limiter_holds() {
    let (tx, mut mix) = mixer(Layout::stereo(), 64, SR);
    tx.ears(vec![Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
    let p = Arc::new(bell());
    tx.play(&p, &[], Play { place: Place::At([0.0, 0.0, -68.6]), sound_delay: true, gain: 20.0, ..Default::default() });
    let out = mix.render_seconds(0.4);
    let first = out.iter().position(|v| v.abs() > 1e-6).unwrap() / 2;
    assert!((first as f32 / SR - 0.2).abs() < 0.003, "{} s", first as f32 / SR);
    for _ in 0..30 { tx.play(&p, &[], Play { gain: 5.0, ..Default::default() }); }
    let out = mix.render_seconds(0.3);
    let bad = out.iter().filter(|v| !(v.abs() <= 0.95)).count();
    assert!(bad == 0, "{bad} samples over, first {:?}", out.iter().find(|v| !(v.abs() <= 0.95)));
}

/// Cost: many struck-metal voices at once, rendered in real time?
#[test]
fn a_hundred_voices_render_far_faster_than_real_time() {
    let (tx, mut mix) = mixer(Layout::surround71(), 256, SR);
    tx.ears(vec![Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
    let p = Arc::new(bell());
    for k in 0..100 { tx.play(&p, &[("freq", 300.0 + k as f32 * 7.0)], Play { place: Place::At([k as f32 % 20.0 - 10.0, 0.0, -5.0]), reverb: 0.2, seed: k, ..Default::default() }); }
    tx.room(1.5, 0.4, 0.5);
    let t = std::time::Instant::now();
    mix.render_seconds(1.0);
    let secs = t.elapsed().as_secs_f32();
    eprintln!("100 voices, 7.1, 1 s: {:.1} ms", secs * 1000.0);
    assert!(secs < 1.0, "{secs} s for one second of audio");
}

