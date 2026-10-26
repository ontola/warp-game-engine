//! `sideways sound`: engine sound without speakers. Renders a WAV and a
//! spectrogram PNG (with the expected firing frequency drawn over it), so
//! sound can be checked and tuned the same way as frames.
//!
//!     sideways sound out.wav --car mv6 --sweep 1000:7600 --seconds 6
//!     sideways sound out.wav --race --seed 1 --ticks 2400 --from 400

use audio::analysis::{rms, spectrogram, write_wav};
use audio::engine::Controls;
use audio::mix::{Mixer, place};
use audio::tyres::TyreControls;
use audio::{SAMPLE_RATE, Sounds};
use glam::{Quat, Vec3};
use sideways::car::Car;
use sideways::physics::Physics;
use sideways::surfaces::Surface;

/// Game presets belong to Sideways; the audio engine only parses supplied data.
pub fn load_sounds(path: Option<&str>) -> Result<Sounds, String> {
    match path {
        Some(path) => Sounds::load(path),
        None => Sounds::parse(&sideways::game_assets::read_data("sounds.toml", include_str!("../../data/sounds.toml"))?),
    }
}

/// The sound model id of a car spec (cars.toml model ids).
pub fn sound_id(car: &Car) -> &'static str {
    let m = &car.spec.model;
    sideways::spec::GRID.iter().copied().find(|g| m.contains(g)).unwrap_or("base")
}

/// What the engine and tyres are doing, for the synth.
pub fn controls(car: &Car, phys: &Physics) -> (Controls, TyreControls) {
    let peak = car.spec.torque.iter().map(|p| p[1]).fold(1.0f64, f64::max);
    let engine = Controls {
        rpm: car.rpm() as f32,
        throttle: car.throttle as f32,
        load: (car.engine_torque() / peak) as f32,
        cut: car.cut(),
        gear: car.gear,
        speed: car.speed as f32,
    };
    let mut t = TyreControls { speed: car.speed.abs() as f32, ..Default::default() };
    let n = car.wheels.len().max(1) as f32;
    let mut on = 0.0;
    for w in &car.wheels {
        if !w.contact {
            continue;
        }
        on += 1.0;
        // Sliding speed at the patch: longitudinal slip plus sideways.
        t.slip += ((w.omega * w.radius - w.vx).abs() + w.vy.abs()) as f32;
        match w.surface {
            Surface::Asphalt => {}
            Surface::Grass | Surface::Water => t.soft += 1.0,
            _ => t.loose += 1.0,
        }
    }
    if on > 0.0 {
        t.loose /= on;
        t.soft /= on;
    }
    t.contact = on / n;
    let _ = phys;
    (engine, t)
}

fn opt(args: &[String], name: &str, default: &str) -> String {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or(default.to_string())
}

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("sound: {e}");
            2
        }
    }
}

fn run_inner(args: &[String]) -> Result<(), String> {
    let out = args.get(1).cloned().filter(|a| !a.starts_with("--")).unwrap_or("sound.wav".into());
    let mut sounds = load_sounds(args.iter().position(|a| a == "--sounds").map(|i| args[i + 1].as_str()))?;
    for (i, a) in args.iter().enumerate() {
        if a == "--set" {
            sounds.set(&args[i + 1])?;
        }
    }
    let sr = SAMPLE_RATE;
    let block = (sr / 120.0) as usize; // one physics tick
    let mut mixer = Mixer::new(sr);
    if args.iter().any(|a| a == "--no-tyres") {
        mixer.tyres = 0.0;
    }
    let mut stereo = Vec::new();
    let mut watch_rpm = Vec::new(); // per block, for the spectrogram overlay
    let t0 = std::time::Instant::now();
    let model = if opt(args, "--model", "pulse") == "fluid" { audio::Model::Fluid } else { audio::Model::Pulse };
    let firing: f32;
    if args.iter().any(|a| a == "--race") {
        use kern::Sim;
        let mut settings = sideways::race::Settings::default();
        settings.stage_km = opt(args, "--km", "3").parse().unwrap();
        settings.rivals = opt(args, "--rivals", "1").parse().unwrap();
        settings.bots = 1;
        let cars = sideways::spec::Cars::load(None)?;
        let mut timings = kern::Timings::default();
        let mut race = sideways::race::Race::new(opt(args, "--seed", "1").parse().unwrap(), settings, cars);
        let ticks: u64 = opt(args, "--ticks", "2400").parse().unwrap();
        let from: u64 = opt(args, "--from", "400").parse().unwrap();
        let listen: usize = opt(args, "--player", "0").parse().unwrap();
        let _ = listen;
        for p in &race.players {
            // The listener's own car gets `--model`; the others the pulse model.
            let m = if mixer.cars.len() == listen { model } else { audio::Model::Pulse };
            mixer.add_car_model(sounds.for_car(sound_id(&p.car))?, mixer.cars.len() as u32 + 1, m);
        }
        firing = mixer.cars[listen].engine.firing_hz(1.0);
        let mut buf = vec![0.0; block * 2];
        let mut prev_ear: Option<Vec3> = None;
        for tick in 0..ticks {
            race.step(&mut timings);
            if tick < from {
                continue;
            }
            // The listener rides with the chase camera of `listen`.
            let me = &race.players[listen].car;
            let pose = me.pose(&race.phys);
            let rot = Quat::from_xyzw(pose.rotation.x, pose.rotation.y, pose.rotation.z, pose.rotation.w);
            let pos = Vec3::new(pose.translation.x, pose.translation.y, pose.translation.z);
            let fwd = rot * Vec3::NEG_Z;
            let heading = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
            let ear = pos - heading * 5.2 + Vec3::Y * 2.05;
            let ear_vel = prev_ear.map(|p| (ear - p) * 120.0).unwrap_or(Vec3::ZERO);
            prev_ear = Some(ear);
            let right = heading.cross(Vec3::Y);
            for (i, p) in race.players.iter().enumerate() {
                let (c, t) = controls(&p.car, &race.phys);
                let cp = p.car.pose(&race.phys).translation;
                let v = p.car.linvel(&race.phys);
                let rel = Vec3::new(cp.x, cp.y, cp.z) - ear;
                let dir = rel.normalize_or_zero();
                let closing = (ear_vel - Vec3::new(v.x, v.y, v.z)).dot(dir);
                let car = &mut mixer.cars[i];
                car.controls = c;
                car.tyre_controls = t;
                car.place = place(rel.to_array(), right.to_array(), closing);
            }
            mixer.render(&mut buf);
            stereo.extend_from_slice(&buf);
            watch_rpm.push(race.players[listen].car.rpm() as f32);
        }
    } else {
        let id = opt(args, "--car", "base");
        mixer.add_car_model(sounds.get(&id)?, 1, model);
        firing = mixer.cars[0].engine.firing_hz(1.0);
        let sweep: Vec<f32> = opt(args, "--sweep", "1000:7400").split(':').map(|x| x.parse().unwrap()).collect();
        let mut seconds: f32 = opt(args, "--seconds", "6").parse().unwrap();
        let throttle: f32 = opt(args, "--throttle", "1").parse().unwrap();
        // --controls file.json: [[t, rpm, throttle], ...] (e.g. read off a
        // reference recording); interpolated, rpm 0/null is the engine off.
        let controls: Vec<[f32; 3]> = match args.iter().position(|a| a == "--controls") {
            Some(i) => {
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&args[i + 1]).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                v.as_array().ok_or("controls: expected an array")?.iter().map(|r| [r[0].as_f64().unwrap_or(0.0) as f32, r[1].as_f64().unwrap_or(0.0) as f32, r[2].as_f64().unwrap_or(0.0) as f32]).collect()
            }
            None => Vec::new(),
        };
        if let Some(last) = controls.last() {
            seconds = last[0];
        }
        let blocks = (seconds * 120.0) as usize;
        let speed_pin: Option<f32> = args.iter().position(|a| a == "--speed").map(|i| args[i + 1].parse().unwrap());
        let gear_pin: Option<i32> = args.iter().position(|a| a == "--gear").map(|i| args[i + 1].parse().unwrap());
        let mut buf = vec![0.0; block * 2];
        for b in 0..blocks {
            let x = b as f32 / blocks.max(1) as f32;
            let mut rpm = sweep[0] + (sweep[sweep.len() - 1] - sweep[0]) * x;
            let mut throttle = throttle;
            if !controls.is_empty() {
                let t = b as f32 / 120.0;
                let k = controls.partition_point(|c| c[0] <= t).clamp(1, controls.len() - 1);
                let (c0, c1) = (controls[k - 1], controls[k]);
                let f = ((t - c0[0]) / (c1[0] - c0[0]).max(1e-6)).clamp(0.0, 1.0);
                rpm = c0[1] + (c1[1] - c0[1]) * f;
                throttle = c0[2] + (c1[2] - c0[2]) * f;
            }
            let car = &mut mixer.cars[0];
            // --speed and --gear pin the road state (e.g. standing still).
            let speed = speed_pin.unwrap_or(rpm / 7600.0 * 40.0);
            car.controls = Controls { rpm, throttle, load: throttle * 0.9, cut: false, gear: gear_pin.unwrap_or(3), speed };
            car.place = audio::Placement { gain: 1.0, pan: 0.0, air_hz: 18_000.0, doppler: 1.0 };
            mixer.render(&mut buf);
            stereo.extend_from_slice(&buf);
            watch_rpm.push(rpm);
        }
    }
    let secs = stereo.len() as f32 / 2.0 / sr;
    let took = t0.elapsed().as_secs_f32();
    write_wav(std::path::Path::new(&out), sr as u32, 2, &stereo).map_err(|e| format!("{out}: {e}"))?;
    let mono: Vec<f32> = stereo.chunks(2).map(|f| 0.5 * (f[0] + f[1])).collect();
    let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let work: u64 = mixer.cars.iter().map(|c| c.engine.work()).sum();
    if work > 0 {
        println!("fluid: {:.1} M cell updates per second of sound", work as f32 / secs / 1e6);
    }
    println!("sound: {out}: {secs:.1} s, rms {:.3}, peak {peak:.3}, {} car(s), rendered in {took:.2} s with the sim ({:.0}x realtime)", rms(&mono), mixer.cars.len(), secs / took.max(1e-6));
    if args.iter().any(|a| a == "--orders") {
        // Order analysis of the last second: amplitude (dB) at multiples of
        // half the crank frequency, where an engine's sound lives.
        let r = *watch_rpm.last().unwrap_or(&1.0);
        let tail = &mono[mono.len().saturating_sub(sr as usize)..];
        let crank = r / 60.0;
        let mut line = String::new();
        let peak = (1..=24).map(|k| goertzel(tail, crank * k as f32 * 0.5, sr)).fold(1e-9f32, f32::max);
        for k in 1..=24 {
            let a = goertzel(tail, crank * k as f32 * 0.5, sr);
            line += &format!(" {:.1}:{:.0}", k as f32 * 0.5, 20.0 * (a / peak).max(1e-6).log10());
        }
        println!("orders at {r:.0} rpm (order:dB):{line}");
    }
    let png = opt(args, "--spectrogram", &out.replace(".wav", ".png"));
    spectrogram_png(&png, &mono, sr, &watch_rpm, firing, block)?;
    println!("spectrogram: {png} (red: firing frequency, rpm/60 * cylinders/2)");
    Ok(())
}

/// Spectrogram on a log frequency axis (30 Hz..8 kHz), log magnitude; the
/// firing frequency in red and its 2nd harmonic in orange.
/// Amplitude of frequency `f` in `x` (Goertzel, Hann window).
fn goertzel(x: &[f32], f: f32, sr: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * f / sr;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    let n = x.len();
    for (i, v) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos();
        let s0 = v * win + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / n as f32
}

fn spectrogram_png(path: &str, x: &[f32], sr: f32, rpm: &[f32], firing_per_rpm: f32, block: usize) -> Result<(), String> {
    let n = 4096;
    let hop = 480;
    let cols = spectrogram(x, n, hop);
    let (fmin, fmax) = (30.0f32, 8000.0f32);
    let bins = (fmax / (sr / n as f32)) as usize;
    let h_px = 400u32;
    let (w, h) = (cols.len().max(1) as u32, h_px);
    let y_of = |f: f32| (h_px as f32 - 1.0) * (1.0 - (f / fmin).ln() / (fmax / fmin).ln());

    let mut img = image::RgbImage::new(w, h);
    let max = cols.iter().flat_map(|c| c[..bins].iter()).fold(1e-9f32, |m, v| m.max(*v));
    for (cx, col) in cols.iter().enumerate() {
        for y in 0..h {
            let f = fmin * (fmax / fmin).powf((h - 1 - y) as f32 / (h - 1) as f32);
            let k = ((f / (sr / n as f32)) as usize).min(col.len() - 1);
            let db = 20.0 * (col[k] / max).max(1e-6).log10();
            let v = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let c = [(v * 255.0) as u8, (v.powf(0.7) * 230.0) as u8, (v.powf(0.4) * 160.0) as u8];
            img.put_pixel(cx as u32, y, image::Rgb([c[2] / 2 + c[0] / 2, c[1], c[2]]));
        }
        let t = (cx * hop + n / 2) / block;
        if let Some(r) = rpm.get(t) {
            for (mult, col) in [(1.0, [255, 40, 40]), (2.0, [255, 160, 0])] {
                let f = r * firing_per_rpm * mult;
                let y = y_of(f);
                if y >= 0.0 && y < h as f32 && f > fmin && cx % 3 == 0 {
                    img.put_pixel(cx as u32, y as u32, image::Rgb(col));
                }
            }
        }
    }
    img.save(path).map_err(|e| format!("{path}: {e}"))
}
