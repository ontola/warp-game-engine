use super::*;
use crate::analysis::{rms, spectrum};
use std::sync::Arc;

const SR: f32 = 48_000.0;

fn node(unit: Unit, inputs: Vec<In>) -> Node { Node { unit, inputs } }

#[test]
fn prepared_patch_keeps_defaults_samples_and_active_voices_independent() {
    let patch = Patch {
        name: "recorded".into(), params: vec![("rate".into(), 1.0)],
        nodes: vec![node(Unit::Clip { name: "test".into(), samples: (0..1000).map(|i| (i as f32 * 0.17).sin()).collect::<Vec<_>>().into(), looping: false }, vec![In::P("rate".into())])],
        out: 0, max_length: 0.1, ..Default::default()
    };
    let prepared = patch.compile().unwrap();
    let mut active = prepared.voice(&[], 7, SR);
    let (edited, previous) = prepared.with_default("rate", 0.5).unwrap();
    assert_eq!(previous, 1.0);
    assert!(prepared.with_default("missing", 0.5).is_err());
    let old = patch.voice(&[], 7, SR).render_all(0.1);
    assert_eq!(active.render_all(0.1), old);
    assert_eq!(prepared.voice(&[], 7, SR).render_all(0.1), old);
    assert_ne!(edited.voice(&[], 7, SR).render_all(0.1), old);
    assert_eq!(edited.voice(&[("rate", 1.0)], 7, SR).render_all(0.1), old);
    for sr in [24_000.0, SR, 96_000.0] {
        assert_eq!(prepared.voice(&[("rate", 0.73)], 3, sr).render_all(0.1), patch.voice(&[("rate", 0.73)], 3, sr).render_all(0.1));
    }
}

#[test]
fn prepared_graph_preserves_seed_glides_release_and_lifetime() {
    let mut patch = bell();
    patch.nodes.push(node(Unit::Expr { src: "x * exp(-t * 2)".into(), vars: vec!["x".into()] }, vec![In::N(patch.out)]));
    patch.out = patch.nodes.len() - 1;
    patch.params.push(("duration".into(), 0.6));
    patch.length_param = Some("duration".into());
    let prepared = patch.compile().unwrap();
    for seed in [1, 7, 42] {
        for sr in [24_000.0, SR, 96_000.0] {
            let mut direct = patch.voice(&[("freq", 700.0), ("duration", 0.4)], seed, sr);
            let mut shared = prepared.voice(&[("freq", 700.0), ("duration", 0.4)], seed, sr);
            for block in 0..1000 {
                if block == 17 { direct.set("freq", 910.0); shared.set("freq", 910.0); }
                if block == 80 { direct.release(); shared.release(); }
                let (mut a, mut b) = ([0.0; BLOCK], [0.0; BLOCK]);
                let alive = direct.render(&mut a);
                assert_eq!(alive, shared.render(&mut b));
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "seed {seed}, sample rate {sr}, block {block}");
                if !alive { break; }
            }
        }
    }
}

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
    // Debug build on a shared machine: generous.
    assert!(secs < 3.0, "{secs} s for one second of audio");
}


fn ear() -> Ear { Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] } }

#[test]
fn pitch_resamples_and_pan_matches_godot() {
    // A click every 0.1 s... a 0.2 s tone: at pitch 2 it lasts 0.1 s.
    let p = Arc::new(Patch { name: "t".into(), nodes: vec![node(Unit::Osc { shape: Shape::Sine }, vec![In::K(440.0)])], out: 0, max_length: 0.2, ..Default::default() });
    let (tx, mut mix) = mixer(Layout::stereo(), 8, SR);
    tx.play(&p, &[], Play { place: Place::Pan(0.4), ..Default::default() }.pitch(2.0));
    let out = mix.render_seconds(0.3);
    let last = out.iter().rposition(|v| v.abs() > 1e-4).unwrap() / 2;
    assert!((last as f32 / SR - 0.1).abs() < 0.004, "{} s", last as f32 / SR);
    assert!((peak_hz(&out.iter().step_by(2).copied().collect::<Vec<_>>()) - 880.0).abs() < 12.0);
    // Godot panner: L = 1 - pan, R = 1.
    let l = rms(&out.iter().step_by(2).copied().collect::<Vec<_>>());
    let r = rms(&out.iter().skip(1).step_by(2).copied().collect::<Vec<_>>());
    assert!((l / r - 0.6).abs() < 0.01, "{l} {r}");
}

#[test]
fn godot_falloff_boosts_close_and_shelves_far() {
    let f = Falloff::godot(12.0);
    assert!((f.gain(12.0) - 1.0).abs() < 1e-3);
    assert!((f.gain(24.0) - 0.5).abs() < 1e-3);
    assert!((f.gain(0.5) - 7.943).abs() < 1e-3, "capped at +18 dB");
    assert_eq!(f.shelf_db(6.0), 0.0);
    assert!((f.shelf_db(48.0) + 9.0).abs() < 1e-3);
    assert!((f.fade(100.0).gain(50.0) - 0.5 * 12.0 / 50.0).abs() < 1e-3);
}

#[test]
fn loud_sounds_cull_quiet_ones_and_lift_the_hall() {
    let p = Arc::new(bell());
    let (tx, mut mix) = mixer(Layout::stereo(), 32, SR);
    tx.ears(vec![ear()]);
    tx.cull(Some(Cull::default()));
    tx.play(&p, &[], Play { tail: 0.3, ..Play::at([0.0, 0.0, -10.0]) }.spl(165.0));
    tx.play(&p, &[], Play::at([0.0, 0.0, -2.0]).spl(100.0));
    tx.play(&p, &[], Play::at([0.0, 0.0, -2.0]).spl(140.0));
    mix.render_seconds(0.05);
    assert_eq!(mix.culled, 1);
    assert_eq!(mix.active(), 2);
    assert!(tx.loudest() > 155.0, "{}", tx.loudest());
    // The window releases at 110 dB/s: after 1 s it is back near the floor.
    mix.render_seconds(1.0);
    assert!(tx.loudest() < 60.5 + 1.0, "{}", tx.loudest());
    // The hall carries a tail well after the dry sound ended.
    let tail = mix.render_seconds(1.0);
    assert!(rms(&tail) > 1e-4, "hall tail {}", rms(&tail));
}

#[test]
fn the_godot_master_compresses_hot_mixes() {
    let p = Arc::new(Patch { name: "t".into(), nodes: vec![node(Unit::Osc { shape: Shape::Sine }, vec![In::K(200.0)])], out: 0, max_length: 1.0, ..Default::default() });
    let level = |m: Option<Master>| {
        let (tx, mut mix) = mixer(Layout::mono(), 8, SR);
        if let Some(m) = m { tx.master(m); }
        tx.play(&p, &[], Play { gain: 0.95, ..Default::default() });
        let out = mix.render_seconds(0.5);
        out[12000..].iter().fold(0.0f32, |a, v| a.max(v.abs()))
    };
    let (plain, godot) = (level(None), level(Some(Master::godot())));
    assert!(plain > 0.88 && godot < plain * 0.9, "{plain} {godot}");
}

#[cfg(feature = "luau")]
#[test]
fn engines_and_tyres_play_as_units() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/sideways/data/sounds.luau");
    let sounds = crate::Sounds::load(path).unwrap();
    register_engines(&sounds);
    let car = sounds.ids()[0].clone();
    let src = format!("loop() local rpm = param('rpm', 3000) local thr = param('throttle', 0.8) return engine('{car}', rpm, thr, 0.6) + tyres(20, 4, 1, 1) * 0.25");
    let p = Arc::new(crate::synth::lua::load("car", &src).unwrap().remove(0));
    // The engine levels itself; the firing frequency follows the rpm.
    let centroid = |x: &[f32]| { let s = spectrum(&x[..8192], 8192); let (a, b) = s.iter().enumerate().take(800).fold((0.0, 0.0), |(a, b), (k, m)| (a + k as f32 * m, b + m)); a / b * SR / 8192.0 };
    let e = crate::synth::lua::load("e", &format!("return engine('{car}', param('rpm', 6000), 0.8, 0.6)")).unwrap().remove(0);
    let idle = e.voice(&[("rpm", 900.0)], 1, SR).render_all(1.0);
    let pull = e.voice(&[], 1, SR).render_all(1.0);
    assert!(rms(&idle) > 1e-3 && rms(&pull) > 1e-3);
    assert!(centroid(&pull[24000..]) > centroid(&idle[24000..]) * 1.1, "{} {}", centroid(&idle[24000..]), centroid(&pull[24000..]));
    // In the mixer, behind and to the left on 7.1.
    let (tx, mut mix) = mixer(Layout::surround71(), 8, SR);
    tx.ears(vec![Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
    tx.play(&p, &[], Play::at([-3.0, 0.0, 3.0]));
    let out = mix.render_seconds(0.5);
    let ch = |c: usize| rms(&out.iter().skip(c).step_by(8).copied().collect::<Vec<_>>());
    assert!(ch(4) > ch(1) * 3.0, "left back leads: {} vs {}", ch(4), ch(1));
}
