//! Scripted playtests: drive a game headless from a JSON script, saving
//! frames and state dumps, so an agent (or CI) can play, look and check.
//!
//! ```json
//! { "size": [800, 450], "seed_note": "free text is ignored",
//!   "steps": [
//!     { "ticks": 60 },
//!     { "ticks": 30, "player": 0, "mv": [0, 1], "look": [0.02, 0], "hold": ["RIGHT_TRIGGER"] },
//!     { "press": ["SOUTH"] },
//!     { "capture": "after-jump.png" },
//!     { "state": "after-jump.json" },
//!     { "expect": "/players/0/alive", "equals": true },
//!     { "cmd": ["give 0 explosive", "face 0 0 -1.4"] },
//!     { "until": "/round", "min": 2, "max_ticks": 4000 }
//!   ] }
//! ```
//!
//! A step with `ticks` (default 1 when it has controls) runs that many ticks;
//! `press` applies on its first tick only, `hold`, `mv`, `look` and `aim` on every
//! tick (look is per tick, x right and y up, radians). Other players idle.
//! `capture` and `state` write into the output directory. `expect` reads a
//! JSON pointer (RFC 6901) from the game's state and compares it with
//! `equals`, or checks it within `min`/`max`, or (with neither) that it
//! exists; `until` ticks until such a check passes. A failed expectation
//! fails the run. A `capture` with `compare` checks the frame against a reference PNG
//! (written on first run). `audio` writes the sound since the previous
//! audio step as a WAV with its peak and RMS. `cmd` (a string or a list)
//! runs game debug commands (`Game::command`) before the step's ticks, to
//! set up a scenario: give items, teleport, aim. Every step prints one JSON line.
//!
//! Text and devices: `clipboard` sets the (in-process) clipboard, `text`
//! types a string and `keys` sends editing keys (`Backspace`, `Enter`,
//! `Escape`, `Left`, `Right`, `Home`, `End`, `Delete`, `Copy`, `Paste`)
//! through [`Game::text`], before the step's ticks. `pads: N` connects N
//! virtual pads (`pad0`..) beside the keyboard from that step on; `device:
//! "pad1"` makes the step's controls come from that device, routed through
//! [`Game::seats`] as the window does (a spare pad pressing X to join).
//! Without either, controls go straight to seat `player`. Scripts never touch
//! the system clipboard.
use crate::input::Device;
use crate::text::TextEvent;
use crate::{Buttons, Controls, Game, Headless};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Deserialize)]
pub struct Script {
    #[serde(default = "default_size")]
    pub size: [u32; 2],
    pub steps: Vec<Step>,
}

fn default_size() -> [u32; 2] { [800, 450] }

/// A field given as JSON null deserializes to Some(Null), not None.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> { Value::deserialize(d).map(Some) }

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub ticks: Option<u32>,
    #[serde(default)]
    pub player: usize,
    pub mv: Option<[f32; 2]>,
    pub look: Option<[f32; 2]>,
    /// The right stick's position (twin-stick aim), x right, y up.
    pub aim: Option<[f32; 2]>,
    #[serde(default)]
    pub hold: Vec<String>,
    #[serde(default)]
    pub press: Vec<String>,
    pub capture: Option<String>,
    pub state: Option<String>,
    pub expect: Option<String>,
    /// Tick (with this step's controls) until this pointer passes the
    /// `equals`/`min`/`max` check (or is not null), at most `max_ticks`
    /// (default 3600); not reaching it is a failure.
    pub until: Option<String>,
    pub max_ticks: Option<u32>,
    /// `null` is a real expectation (the value must be null or absent).
    #[serde(default, deserialize_with = "present")]
    pub equals: Option<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// With `capture`: a reference PNG (relative to the script) to compare
    /// against; fails when the mean absolute difference per channel (0..255)
    /// exceeds `tolerance` (default 1.0). A missing reference is written.
    pub compare: Option<String>,
    /// Write the sound mixed since the previous `audio` step (needs
    /// `Headless::audio`) as a WAV and report its peak and RMS.
    pub audio: Option<String>,
    pub tolerance: Option<f64>,
    /// Debug commands for `Game::command`, run before the ticks.
    #[serde(default, deserialize_with = "one_or_many")]
    pub cmd: Vec<String>,
    /// Free text shown in the log line.
    pub note: Option<String>,
    /// Typed into the game (`Game::text`) before the ticks.
    pub text: Option<String>,
    /// Editing keys (see `TextEvent::key`), after `text`.
    #[serde(default)]
    pub keys: Vec<String>,
    /// Set the clipboard first.
    pub clipboard: Option<String>,
    /// Connected virtual pads from this step on.
    pub pads: Option<u32>,
    /// The device this step's controls come from (`keyboard`, `pad0`..).
    pub device: Option<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum M { One(String), Many(Vec<String>) }
    Ok(match M::deserialize(d)? { M::One(s) => vec![s], M::Many(v) => v })
}

pub fn button(name: &str) -> Result<u32, String> {
    Ok(match name.to_ascii_uppercase().as_str() {
        "SOUTH" | "A" => Buttons::SOUTH,
        "EAST" | "B" => Buttons::EAST,
        "WEST" | "X" => Buttons::WEST,
        "NORTH" | "Y" => Buttons::NORTH,
        "LEFT_SHOULDER" | "LB" => Buttons::LEFT_SHOULDER,
        "RIGHT_SHOULDER" | "RB" => Buttons::RIGHT_SHOULDER,
        "LEFT_TRIGGER" | "LT" => Buttons::LEFT_TRIGGER,
        "RIGHT_TRIGGER" | "RT" => Buttons::RIGHT_TRIGGER,
        "SELECT" => Buttons::SELECT,
        "START" => Buttons::START,
        "UP" => Buttons::UP,
        "DOWN" => Buttons::DOWN,
        "LEFT" => Buttons::LEFT,
        "RIGHT" => Buttons::RIGHT,
        other => return Err(format!("unknown button {other}")),
    })
}

fn mask(names: &[String]) -> Result<u32, String> { names.iter().try_fold(0, |m, n| Ok(m | button(n)?)) }

/// Run a script. Returns the number of failed expectations.
fn read_png(path: &Path) -> Result<([u32; 2], Vec<u8>), String> {
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
    Ok(([img.width(), img.height()], img.into_raw()))
}

/// Mean absolute difference per RGB channel, 0..255.
pub fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for c in 0..3 { sum += u64::from(p[c].abs_diff(q[c])); n += 1; }
    }
    sum as f64 / n.max(1) as f64
}

fn check(state: &Value, pointer: &str, step: &Step) -> (Value, bool) {
    let got = state.pointer(pointer).cloned().unwrap_or(Value::Null);
    let ok = match (&step.equals, got.as_f64()) {
        (Some(want), _) => &got == want,
        (None, Some(v)) if step.min.is_some() || step.max.is_some() => step.min.is_none_or(|m| v >= m) && step.max.is_none_or(|m| v <= m),
        (None, _) if step.min.is_none() && step.max.is_none() => !got.is_null(),
        _ => false,
    };
    (got, ok)
}

/// Run a script; `base` resolves reference images. Returns failed checks.
pub fn run(game: &mut dyn Game, headless: &mut Headless, script: &Script, out: &Path, base: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    run_steps(game, Some(headless), script, out, base)
}

/// The runner; without `headless`, capture and audio steps are errors.
fn run_steps(game: &mut dyn Game, mut headless: Option<&mut Headless>, script: &Script, out: &Path, base: &Path) -> Result<usize, String> {
    crate::clipboard::memory_only();
    let mut pads: Option<u32> = None;
    let mut tick = 0u64;
    let mut failed = 0;
    for (i, step) in script.steps.iter().enumerate() {
        let has_controls = step.mv.is_some() || step.look.is_some() || step.aim.is_some() || !step.hold.is_empty() || !step.press.is_empty();
        let ticks = if step.until.is_some() { step.max_ticks.unwrap_or(3600) } else { step.ticks.unwrap_or(u32::from(has_controls)) };
        let mut reached = None;
        let (hold, press) = (mask(&step.hold)?, mask(&step.press)?);
        let device = step.device.as_deref().map(Device::parse).transpose().map_err(|e| format!("step {i}: {e}"))?;
        if step.pads.is_some() { pads = step.pads; }
        if device.is_some() && pads.is_none() { pads = Some(0); }
        let players = game.local_players();
        if device.is_none() && step.player >= players.max(1) && has_controls { return Err(format!("step {i}: player {} of {players}", step.player)); }
        let mut replies = Vec::new();
        for c in &step.cmd {
            match game.command(c) {
                Ok(r) => replies.push(json!({ "cmd": c, "ok": true, "reply": r })),
                Err(e) => { failed += 1; replies.push(json!({ "cmd": c, "ok": false, "error": e })); }
            }
        }
        if let Some(c) = &step.clipboard { crate::clipboard::set(c); }
        let mut typed = Vec::new();
        if let Some(t) = &step.text { typed.push(TextEvent::Insert(t.clone())); }
        for k in &step.keys { typed.push(TextEvent::key(k).map_err(|e| format!("step {i}: {e}"))?); }
        let text_events = typed.len();
        for ev in typed { game.text(ev); }
        for t in 0..ticks {
            let mine = Controls {
                mv: step.mv.unwrap_or_default(),
                look: step.look.unwrap_or_default(),
                aim: step.aim.unwrap_or_default(),
                held: Buttons(hold | if t == 0 { press } else { 0 }),
                pressed: Buttons(if t == 0 { press } else { 0 }),
            };
            let c = match pads {
                // Devices: every connected one idles but the step's, and the
                // game's seats route them.
                Some(n) => {
                    let mut devices: Vec<(Device, Controls)> = std::iter::once(Device::Keyboard).chain((0..n).map(Device::Pad)).map(|d| (d, Controls::default())).collect();
                    if let Some(dev) = device {
                        match devices.iter_mut().find(|d| d.0 == dev) { Some(d) => d.1 = mine, None => devices.push((dev, mine)) }
                    }
                    let mut c = crate::seat_controls(game, &devices);
                    if device.is_none() && has_controls && let Some(p) = c.get_mut(step.player) { *p = mine; }
                    c
                }
                None => {
                    let mut c = vec![Controls::default(); game.local_players()];
                    if let Some(p) = c.get_mut(step.player) { *p = mine; }
                    c
                }
            };
            game.tick(&c);
            tick += 1;
            // A live-control screenshot asked for during the tick.
            if game.wants_capture() && let Some(h) = headless.as_deref_mut() { let rgba = h.frame(game)?; game.captured(h.size, rgba); }
            if let Some(headless) = headless.as_deref_mut() && let Some(mix) = &mut headless.audio {
                let n = (audio::SAMPLE_RATE / headless.tick_hz).round() as usize * 2;
                let start = headless.recorded.len();
                headless.recorded.resize(start + n, 0.0);
                mix.render(&mut headless.recorded[start..], 2);
            }
            if let Some(pointer) = &step.until {
                let state: Value = game.state().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
                if check(&state, pointer, step).1 { reached = Some(t + 1); break; }
            }
        }

        let mut line = json!({ "step": i, "tick": tick });
        if let Some(pointer) = &step.until {
            if reached.is_none() { failed += 1; }
            line["until"] = json!({ "pointer": pointer, "ticks": reached, "ok": reached.is_some() });
        }
        if let Some(n) = &step.note { line["note"] = json!(n); }
        if text_events > 0 { line["text_events"] = json!(text_events); }
        if let Some(d) = device { line["device"] = json!(d.to_string()); }
        if !replies.is_empty() { line["cmd"] = json!(replies); }
        if let Some(name) = &step.capture {
            let headless = headless.as_deref_mut().ok_or("capture needs a renderer")?;
            let rgba = headless.frame(game)?;
            let path = out.join(name);
            render::write_png(&path, headless.size[0], headless.size[1], &rgba)?;
            line["capture"] = json!(path.display().to_string());
            if let Some(reference) = &step.compare {
                let reference = base.join(reference);
                match read_png(&reference) {
                    Ok((size, want)) if size == headless.size => {
                        let diff = mean_abs_diff(&rgba, &want);
                        let ok = diff <= step.tolerance.unwrap_or(1.0);
                        if !ok { failed += 1; }
                        line["compare"] = json!({ "reference": reference.display().to_string(), "mean_abs_diff": diff, "ok": ok });
                    }
                    Ok((size, _)) => {
                        failed += 1;
                        line["compare"] = json!({ "reference": reference.display().to_string(), "error": format!("size {size:?} differs"), "ok": false });
                    }
                    Err(_) => {
                        render::write_png(&reference, headless.size[0], headless.size[1], &rgba)?;
                        line["compare"] = json!({ "reference": reference.display().to_string(), "written": true });
                    }
                }
            }
        }
        let state = || -> Result<Value, String> {
            let text = game.state().ok_or("this game exposes no state")?;
            serde_json::from_str(&text).map_err(|e| format!("game state is not JSON: {e}"))
        };
        if let Some(name) = &step.audio {
            let headless = headless.as_deref_mut().ok_or("audio needs a mixer")?;
            let path = out.join(name);
            let samples = std::mem::take(&mut headless.recorded);
            audio::analysis::write_wav(&path, audio::SAMPLE_RATE as u32, 2, &samples).map_err(|e| e.to_string())?;
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt();
            line["audio"] = json!({ "path": path.display().to_string(), "seconds": samples.len() as f32 / 2.0 / audio::SAMPLE_RATE, "peak": peak, "rms": rms,
                "enabled": headless.audio.is_some() });
        }
        if let Some(name) = &step.state {
            let path = out.join(name);
            std::fs::write(&path, serde_json::to_string_pretty(&state()?).unwrap()).map_err(|e| e.to_string())?;
            line["state"] = json!(path.display().to_string());
        }
        if let Some(pointer) = &step.expect {
            let (got, ok) = check(&state()?, pointer, step);
            if !ok { failed += 1; }
            line["expect"] = json!({ "pointer": pointer, "got": got, "ok": ok });
        }
        println!("{line}");
    }
    println!("{}", json!({ "done": true, "ticks": tick, "failed": failed }));
    Ok(failed)
}

/// Load a script from a file and run it.
/// `audio`: an offline mixer whose sender the game holds; scripts can then
/// record and check its sound.
pub fn run_file(game: &mut dyn Game, gpu: &render::Gpu, tick_hz: f32, file: &Path, out: &Path, audio: Option<Box<dyn audio::Source>>) -> Result<usize, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let script: Script = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut headless = Headless::new(gpu, script.size, tick_hz)?;
    headless.audio = audio;
    run(game, &mut headless, &script, out, file.parent().unwrap_or(Path::new(".")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_parse_and_buttons_resolve() {
        let s: Script = serde_json::from_str(r#"{"steps":[{"ticks":3},{"press":["A","rt"]},{"expect":"/x","min":1}]}"#).unwrap();
        assert_eq!(s.size, [800, 450]);
        assert_eq!(s.steps.len(), 3);
        assert_eq!(mask(&s.steps[1].press).unwrap(), Buttons::SOUTH | Buttons::RIGHT_TRIGGER);
        assert!(button("Z").is_err());
        assert_eq!(mean_abs_diff(&[10, 20, 30, 255], &[13, 20, 27, 0]), 2.0);
        assert!(serde_json::from_str::<Script>(r#"{"steps":[{"tick":3}]}"#).is_err(), "typos are errors");
        let n: Script = serde_json::from_str(r#"{"steps":[{"expect":"/a","equals":null},{"expect":"/a"}]}"#).unwrap();
        assert_eq!(n.steps[0].equals, Some(Value::Null));
        assert_eq!(n.steps[1].equals, None);
        let t: Script = serde_json::from_str(r#"{"steps":[{"pads":2,"device":"pad1","press":["X"]},{"text":"abc","keys":["Backspace","Enter"],"clipboard":"t"}]}"#).unwrap();
        assert_eq!(t.steps[0].pads, Some(2));
        assert_eq!(t.steps[1].keys.len(), 2);
    }

    /// A game with seats that a spare pad joins, and a text sink.
    struct Seats { seats: Vec<Vec<Device>>, ticks: Vec<Vec<Controls>>, typed: String }
    impl Game for Seats {
        fn local_players(&self) -> usize { self.seats.len() }
        fn tick(&mut self, c: &[Controls]) { self.ticks.push(c.to_vec()); }
        fn devices(&mut self, devices: &[(Device, Controls)]) {
            for (d, c) in devices {
                if c.pressed.has(Buttons::WEST) && !self.seats.iter().any(|s| s.contains(d)) { self.seats.push(vec![*d]); }
            }
        }
        fn seats(&self) -> Option<Vec<Vec<Device>>> { Some(self.seats.clone()) }
        fn draw(&mut self, _: &mut crate::Frame) -> Result<crate::Draw, String> { Err("no drawing".into()) }
        fn text(&mut self, e: TextEvent) {
            match e { TextEvent::Insert(s) | TextEvent::Paste(s) => self.typed += &s, TextEvent::Backspace => { self.typed.pop(); } _ => {} }
        }
        fn state(&self) -> Option<String> { Some(json!({ "seats": self.seats.len(), "typed": self.typed }).to_string()) }
    }

    #[test]
    fn device_steps_route_through_the_games_seats_and_text_reaches_it() {
        let mut g = Seats { seats: vec![vec![Device::Keyboard]], ticks: vec![], typed: String::new() };
        let script: Script = serde_json::from_str(r#"{"steps":[
            {"pads":2,"ticks":2},
            {"device":"pad1","press":["X"]},
            {"device":"pad1","press":["A"]},
            {"device":"keyboard","press":["B"]},
            {"text":"hello","keys":["Backspace"]},
            {"clipboard":"xyz","keys":["Paste"]},
            {"expect":"/seats","equals":2},
            {"expect":"/typed","equals":"hellxyz"}]}"#).unwrap();
        let failed = run_steps(&mut g, None, &script, Path::new("."), Path::new(".")).unwrap();
        assert_eq!(failed, 0);
        assert_eq!(g.seats, vec![vec![Device::Keyboard], vec![Device::Pad(1)]]);
        // The seat counts from the tick it joins; the keyboard's seat never sees the pad.
        assert_eq!(g.ticks[1].len(), 1);
        assert_eq!(g.ticks[2].len(), 2);
        assert!(!g.ticks[2][0].pressed.has(Buttons::WEST));
        assert!(g.ticks[3][1].pressed.has(Buttons::SOUTH) && !g.ticks[3][0].pressed.has(Buttons::SOUTH));
        assert!(g.ticks[4][0].pressed.has(Buttons::EAST));
        assert!(run_steps(&mut g, None, &serde_json::from_str(r#"{"steps":[{"capture":"x.png"}]}"#).unwrap(), Path::new("."), Path::new(".")).is_err());
    }
}
