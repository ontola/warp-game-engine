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
//!     { "until": "/round", "min": 2, "max_ticks": 4000 }
//!   ] }
//! ```
//!
//! A step with `ticks` (default 1 when it has controls) runs that many ticks;
//! `press` applies on its first tick only, `hold`, `mv` and `look` on every
//! tick (look is per tick, x right and y up, radians). Other players idle.
//! `capture` and `state` write into the output directory. `expect` reads a
//! JSON pointer (RFC 6901) from the game's state and compares it with
//! `equals`, or checks it within `min`/`max`, or (with neither) that it
//! exists; `until` ticks until such a check passes. A failed expectation
//! fails the run. A `capture` with `compare` checks the frame against a reference PNG
//! (written on first run). `audio` writes the sound since the previous
//! audio step as a WAV with its peak and RMS. Every step prints one JSON line.
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
    /// Free text shown in the log line.
    pub note: Option<String>,
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
    let players = game.local_players();
    let mut tick = 0u64;
    let mut failed = 0;
    for (i, step) in script.steps.iter().enumerate() {
        let has_controls = step.mv.is_some() || step.look.is_some() || !step.hold.is_empty() || !step.press.is_empty();
        let ticks = if step.until.is_some() { step.max_ticks.unwrap_or(3600) } else { step.ticks.unwrap_or(u32::from(has_controls)) };
        let mut reached = None;
        let (hold, press) = (mask(&step.hold)?, mask(&step.press)?);
        if step.player >= players.max(1) && has_controls { return Err(format!("step {i}: player {} of {players}", step.player)); }
        for t in 0..ticks {
            let mut c = vec![Controls::default(); players];
            if let Some(p) = c.get_mut(step.player) {
                p.mv = step.mv.unwrap_or_default();
                p.look = step.look.unwrap_or_default();
                p.held = Buttons(hold | if t == 0 { press } else { 0 });
                p.pressed = Buttons(if t == 0 { press } else { 0 });
            }
            game.tick(&c);
            tick += 1;
            if let Some(mix) = &mut headless.audio {
                let n = (audio::SAMPLE_RATE / headless.tick_hz).round() as usize * 2;
                let start = headless.recorded.len();
                headless.recorded.resize(start + n, 0.0);
                mix.render_add(&mut headless.recorded[start..], 2);
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
        if let Some(name) = &step.capture {
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
pub fn run_file(game: &mut dyn Game, gpu: &render::Gpu, tick_hz: f32, file: &Path, out: &Path, audio: Option<audio::sfx::SfxMixer>) -> Result<usize, String> {
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
    }
}
