//! warp-audio: render and inspect procedural sounds without a game.
//!
//!   warp-audio render sounds/shot.luau [--sound NAME] [--set size=2] [--seed 3]
//!       [--seconds 2] [--layout 5.1] [--at x,y,z] [--reverb 0.3] [--room 1.5,0.4,0.6]
//!       [--repeat 4 --every 0.12] [--out shot.wav] [--json]
//!   warp-audio check sounds/*.luau
//!   warp-audio units
//!   (any: --engines sounds.toml for engine(...) units)
//!
//! Render prints the length, peak, loudness, spectral centroid, per-speaker
//! levels and a text spectrogram, so a sound can be judged (by a person or a
//! model) without listening.
use audio::analysis::{downmix, levels, text_spectrogram, trim_silence, write_wav};
use audio::synth::{self, Ear, Layout, Place, Play};
use std::path::PathBuf;
use std::sync::Arc;

const SR: f32 = 48_000.0;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Car engines for `engine(...)` units: --engines FILE (a sounds.toml),
    // WARP_ENGINES, or Sideways' table when run from the repository.
    let engines = args.iter().position(|a| a == "--engines").map(|k| { args.remove(k); if k < args.len() { args.remove(k) } else { String::new() } })
        .or_else(|| std::env::var("WARP_ENGINES").ok()).unwrap_or_else(|| "games/sideways/data/sounds.toml".into());
    if let Ok(s) = audio::Sounds::load(&engines) { synth::register_engines(&s); }
    let r = match args.first().map(String::as_str) {
        Some("render") => render(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("units") => { units(); Ok(()) }
        _ => Err("usage: warp-audio render FILE.luau [options] | check FILES | units".into()),
    };
    if let Err(e) = r { eprintln!("{e}"); std::process::exit(1); }
}

fn units() {
    println!("Units (inputs in order; leave trailing ones off for defaults):");
    for (name, ins, doc) in synth::units::UNITS { println!("  {name}({})  {doc}", ins.join(", ")); }
    println!("\nLuau helpers: sine tri saw square steps (osc shapes), osc(shape, ...), noise(color), env{{ {{time, level, curve}} }},");
    println!("  lowpass highpass bandpass notch peak (x, cutoff, q), filter(mode, ...), decay(dur, power), sweep(f0, f1, k),");
    println!("  hump(dur), after(delay, x), expdecay(tau), expr(src, {{name = sig}}), mix(...), db(v), param(name, default),");
    println!("  length(s or param), loop(), sound(name, fn), and for Godot ports white44(), godot_lp(x, k), godot_hp(x, k).");
    println!("  Arithmetic + - * / ^ and unary - work on signals; expr has fmod.");
}

fn check(files: &[String]) -> Result<(), String> {
    let mut bad = 0;
    for f in files {
        match synth::lua::load_file(f.as_ref()) {
            Ok(ps) => for p in ps {
                let params: Vec<String> = p.params.iter().map(|(n, v)| format!("{n}={v}")).collect();
                println!("ok  {f}: {} ({} nodes{}{})", p.name, p.nodes.len(), if params.is_empty() { String::new() } else { format!(", {}", params.join(" ")) }, if p.looping { ", loops" } else { "" });
            },
            Err(e) => { bad += 1; println!("ERR {f}: {e}"); }
        }
    }
    if bad > 0 { Err(format!("{bad} file(s) failed")) } else { Ok(()) }
}

struct Opts {
    file: PathBuf, sound: Option<String>, set: Vec<(String, f32)>, seed: u32, seconds: Option<f32>, layout: Layout,
    at: Option<[f32; 3]>, reverb: f32, room: [f32; 3], repeat: u32, every: f32, out: Option<PathBuf>, json: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { file: PathBuf::new(), sound: None, set: Vec::new(), seed: 1, seconds: None, layout: Layout::mono(), at: None, reverb: 0.0, room: [1.5, 0.4, 0.6], repeat: 1, every: 0.25, out: None, json: false };
    let mut k = 0;
    let num = |s: &str| s.parse::<f32>().map_err(|_| format!("not a number: {s}"));
    let vec3 = |s: &str| -> Result<[f32; 3], String> {
        let v: Vec<f32> = s.split(',').map(|x| x.trim().parse::<f32>().map_err(|_| format!("not x,y,z: {s}"))).collect::<Result<_, _>>()?;
        if v.len() != 3 { return Err(format!("not x,y,z: {s}")); }
        Ok([v[0], v[1], v[2]])
    };
    while k < args.len() {
        let a = args[k].as_str();
        let mut val = || -> Result<String, String> { k += 1; args.get(k).cloned().ok_or(format!("{a} needs a value")) };
        match a {
            "--sound" => o.sound = Some(val()?),
            "--set" => { let v = val()?; let (n, x) = v.split_once('=').ok_or(format!("--set name=value, got {v}"))?; o.set.push((n.into(), num(x)?)); }
            "--seed" => o.seed = val()?.parse().map_err(|_| "seed: an integer")?,
            "--seconds" => o.seconds = Some(num(&val()?)?),
            "--layout" => { let v = val()?; o.layout = Layout::named(&v).ok_or(format!("unknown layout {v} (mono, stereo, quad, 5.1, 7.1)"))?; }
            "--at" => o.at = Some(vec3(&val()?)?),
            "--reverb" => o.reverb = num(&val()?)?,
            "--room" => o.room = vec3(&val()?)?,
            "--repeat" => o.repeat = val()?.parse().map_err(|_| "repeat: an integer")?,
            "--every" => o.every = num(&val()?)?,
            "--out" => o.out = Some(val()?.into()),
            "--json" => o.json = true,
            f if !f.starts_with("--") && o.file.as_os_str().is_empty() => o.file = f.into(),
            f => return Err(format!("unknown option {f}")),
        }
        k += 1;
    }
    if o.file.as_os_str().is_empty() { return Err("render: which .luau file?".into()); }
    Ok(o)
}

fn render(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let patches = synth::lua::load_file(&o.file)?;
    let patch = match &o.sound {
        Some(n) => patches.iter().find(|p| &p.name == n).ok_or_else(|| format!("no sound {n} (has {})", patches.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))?,
        None => &patches[0],
    };
    let patch = Arc::new(patch.compile()?);
    let params: Vec<(&str, f32)> = o.set.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    for (n, _) in &params { if !patch.params.iter().any(|p| p.0 == *n) { return Err(format!("{} has no parameter {n} (has {})", patch.name, patch.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", "))); } }
    let span = o.every * (o.repeat.max(1) - 1) as f32;
    let seconds = o.seconds.unwrap_or(if patch.max_length > 0.0 { patch.max_length } else { 4.0 } + span + if o.reverb > 0.0 { o.room[0] } else { 0.0 });
    let ch = o.layout.channels();
    let t0 = std::time::Instant::now();
    let (tx, mut mix) = synth::mixer(o.layout.clone(), 256, SR);
    tx.ears(vec![Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
    tx.room(o.room[0], o.room[1], o.room[2]);
    let mut out = Vec::with_capacity((seconds * SR) as usize * ch);
    let place = o.at.map_or(Place::Head, Place::At);
    let block = 0.01;
    let mut t = 0.0f32;
    let mut fired = 0u32;
    while t < seconds {
        while fired < o.repeat.max(1) && fired as f32 * o.every <= t + 1e-6 {
            tx.play(&patch, &params, Play { place, reverb: o.reverb, sound_delay: o.at.is_some(), ..Default::default() }.seed(o.seed.wrapping_add(fired)));
            fired += 1;
        }
        out.extend(mix.render_seconds(block));
        t += block;
    }
    let cost = t0.elapsed().as_secs_f32();
    trim_silence(&mut out, ch, SR);
    let l = levels(&out, ch, SR);
    if let Some(path) = &o.out { write_wav(path, SR as u32, ch as u16, &out).map_err(|e| format!("{}: {e}", path.display()))?; }
    if o.json {
        let names: Vec<String> = l.channels_db.iter().map(|v| format!("{v:.1}")).collect();
        println!("{{\"sound\":\"{}\",\"seconds\":{:.3},\"peak_db\":{:.1},\"rms_db\":{:.1},\"punch_db\":{:.1},\"centroid_hz\":{:.0},\"channels_db\":[{}],\"render_ms\":{:.1}}}",
            patch.name, l.seconds, l.peak_db, l.rms_db, l.punch_db, l.centroid_hz, names.join(","), cost * 1000.0);
        return Ok(());
    }
    println!("{} ({} nodes) on {} at seed {}: {:.3} s, rendered in {:.1} ms", patch.name, patch.nodes.len(), o.layout.name, o.seed, l.seconds, cost * 1000.0);
    println!("peak {:.1} dBFS, rms {:.1} dB, loudest 50 ms {:.1} dB, centroid {:.0} Hz", l.peak_db, l.rms_db, l.punch_db, l.centroid_hz);
    if ch > 1 { println!("speakers: {}", l.channels_db.iter().enumerate().map(|(c, v)| format!("{}:{v:.1}", speaker_name(&o.layout.name, c))).collect::<Vec<_>>().join("  ")); }
    print!("{}", text_spectrogram(&downmix(&out, ch), SR));
    if let Some(p) = &o.out { println!("wrote {}", p.display()); }
    Ok(())
}

fn speaker_name(layout: &str, c: usize) -> String {
    let names: &[&str] = match layout { "stereo" => &["L", "R"], "quad" => &["L", "R", "Ls", "Rs"], "5.1" => &["L", "R", "C", "LFE", "Ls", "Rs"], "7.1" => &["L", "R", "C", "LFE", "Lb", "Rb", "Ls", "Rs"], _ => &[] };
    names.get(c).map_or(format!("{c}"), |s| s.to_string())
}
