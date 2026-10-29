//! The looks of the effects in the game's own fx.wgsl (Godot's spell
//! shaders, ported): every particle cell can be drawn by it, and some
//! effects (the mist walls, the ground fields) only are. The file reloads
//! while the game runs: save it and the next frame draws with it; a file
//! that does not compile keeps the last good one and says why.
use crate::fx::{cell_at, Cell};
use render::sprites::Sprite;
use render::Gpu;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

pub const SOURCE: &str = include_str!("fx.wgsl");

/// A cell's own look: effect 1 + the cell's index (fx.wgsl `cell_look`).
pub fn of_cell(c: Cell) -> u32 {
    1 + c as u32
}
/// A wall of mist: params (width, height, open, time).
pub const WARD: u32 = 64;
/// A field on the ground: params (kind, radius, seed, time); kind is the
/// element's index (El).
pub const GROUND: u32 = 65;

struct State {
    path: Option<PathBuf>,
    seen: Option<SystemTime>,
    good: Option<String>,
    pending: Option<String>,
    lines: Vec<String>,
}

static STATE: Mutex<State> = Mutex::new(State { path: None, seen: None, good: None, pending: None, lines: Vec::new() });

fn stamp(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Watch `dir`/app/src/fx.wgsl (the repository's), else use the built-in one.
pub fn watch(dir: Option<&std::path::Path>) {
    let mut s = STATE.lock().unwrap();
    s.path = dir.map(|d| d.join("app/src/fx.wgsl"));
    s.seen = s.path.as_deref().and_then(stamp);
    s.pending = Some(s.path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| SOURCE.to_string()));
}

/// Has fx.wgsl changed on disk? Then it compiles on the next frame.
pub fn scan() -> Option<String> {
    let mut s = STATE.lock().unwrap();
    let p = s.path.clone()?;
    let st = stamp(&p);
    if st == s.seen {
        return None;
    }
    s.seen = st;
    let text = std::fs::read_to_string(&p).ok()?;
    if Some(&text) == s.good.as_ref() {
        return None;
    }
    s.pending = Some(text);
    Some("fx.wgsl: compiles on the next frame".into())
}

/// On a frame with the GPU: install what is pending.
pub fn sync(gpu: &Gpu) {
    let mut s = STATE.lock().unwrap();
    if s.good.is_none() && s.pending.is_none() {
        s.pending = Some(SOURCE.to_string());
    }
    let Some(new) = s.pending.take() else { return };
    match render::sprites::set_game_sprite(gpu, &new) {
        Ok(()) => {
            if s.good.is_some() {
                s.lines.push("fx.wgsl: compiled".into());
            }
            s.good = Some(new);
        }
        Err(e) => {
            s.lines.push(format!("fx.wgsl: {e} (the last good version keeps running)"));
            if s.good.is_none() && new != SOURCE {
                s.pending = Some(SOURCE.to_string());
            }
        }
    }
}

/// What happened since the last call, for the log.
pub fn lines() -> Vec<String> {
    std::mem::take(&mut STATE.lock().unwrap().lines)
}

/// Hand every particle without an effect of its own to its cell's look in
/// fx.wgsl: params (seed from where it is, time, 0, 0).
pub fn tag(sprites: &mut [Sprite], time: f32) {
    for s in sprites {
        if s.effect != 0 {
            continue;
        }
        let Some(c) = cell_at(s.uv) else { continue };
        // A crop (ring bands) keeps the atlas: its quad is not the cell.
        if (s.uv[2].abs() - crate::fx::cell_size()).abs() > 0.01 || (s.uv[3].abs() - crate::fx::cell_size()).abs() > 0.01 {
            continue;
        }
        // Particles bring their own seed and age (params.w = 1); anything else
        // gets a seed from where it is.
        let own = s.params[3] == 1.0;
        let seed = if own { s.params[0] } else { ((s.center.x * 12.9898 + s.center.z * 78.233 + s.center.y * 37.719).sin() * 43758.547).fract().abs() * 97.0 };
        s.effect = of_cell(c);
        s.params = [seed, time, if own { s.params[2] } else { 0.0 }, 0.0];
    }
}
