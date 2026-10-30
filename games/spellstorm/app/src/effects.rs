//! The looks of the effects in the game's own fx.wgsl (Godot's spell
//! shaders, ported): every particle cell can be drawn by it, and some
//! effects (the mist walls, the ground fields) only are. The file reloads
//! while the game runs: save it and the next frame draws with it; a file
//! that does not compile keeps the last good one and says why. The
//! signature spells (tidal wave, black hole, meteor) are in spells.wgsl.
use crate::fx::{cell_at, Cell};
use render::sprites::Sprite;
use render::Gpu;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

const FX: &str = include_str!("fx.wgsl");
const SPELLS: &str = include_str!("spells.wgsl");

/// fx.wgsl and spells.wgsl as one shader: fx.wgsl's game_sprite becomes
/// fx_sprite, which spells.wgsl's own game_sprite hands the rest to.
fn combine(fx: &str, spells: &str) -> String {
    format!("{}\n{spells}", fx.replacen("fn game_sprite(", "fn fx_sprite(", 1))
}

fn builtin() -> String {
    combine(FX, SPELLS)
}

/// A cell's own look: effect 1 + the cell's index (fx.wgsl `cell_look`).
pub fn of_cell(c: Cell) -> u32 {
    1 + c as u32
}
/// A wall of mist: params (width, height, open, time).
pub const WARD: u32 = 64;
/// A field on the ground: params (kind, radius, seed, time); kind is the
/// element's index (El).
pub const GROUND: u32 = 65;
/// The tidal wave's wall of water: params (time, sink, 0, 0).
pub const WAVE: u32 = 66;
/// The black hole: params (open, time, 0, 0).
pub const BLACK_HOLE: u32 = 67;
/// A molten rock (the meteor): params (seed, time, 0, 0).
pub const MOLTEN: u32 = 68;
/// A pool of blood: params (to.x, to.z of the nearest wizard, pull +
/// floor(suck * 1000), time).
pub const BLOOD: u32 = 69;
/// A jet of water along a shaft: params (time, seed, 0, 0).
pub const JET: u32 = 70;
/// The water a tidal wave pushes over the floor: params (time, sink, seed, 0).
pub const SURGE: u32 = 71;
/// The lane a rolling spell will sweep, marked while it charges: params
/// (fill, time, length in metres, 0).
pub const LANE: u32 = 72;
/// A trap's circle of runes: params (armed, seed, time, 0).
pub const TRAP: u32 = 73;

struct State {
    /// The game's app/src, where fx.wgsl and spells.wgsl are.
    path: Option<PathBuf>,
    seen: Option<SystemTime>,
    good: Option<String>,
    pending: Option<String>,
    lines: Vec<String>,
    /// The first install was tried (a broken one is not retried every frame).
    started: bool,
}

static STATE: Mutex<State> = Mutex::new(State { path: None, seen: None, good: None, pending: None, lines: Vec::new(), started: false });

fn stamp(dir: &std::path::Path) -> Option<SystemTime> {
    ["fx.wgsl", "spells.wgsl"].iter().filter_map(|n| std::fs::metadata(dir.join(n)).and_then(|m| m.modified()).ok()).max()
}

fn read(dir: &std::path::Path) -> Option<String> {
    Some(combine(&std::fs::read_to_string(dir.join("fx.wgsl")).ok()?, &std::fs::read_to_string(dir.join("spells.wgsl")).ok()?))
}

/// Watch `dir`/app/src/fx.wgsl and spells.wgsl (the repository's), else
/// use the built-in ones.
pub fn watch(dir: Option<&std::path::Path>) {
    let mut s = STATE.lock().unwrap();
    s.path = dir.map(|d| d.join("app/src"));
    s.seen = s.path.as_deref().and_then(stamp);
    s.pending = Some(s.path.as_deref().and_then(read).unwrap_or_else(builtin));
}

/// Has fx.wgsl or spells.wgsl changed on disk? Then they compile on the next frame.
pub fn scan() -> Option<String> {
    let mut s = STATE.lock().unwrap();
    let p = s.path.clone()?;
    let st = stamp(&p);
    if st == s.seen {
        return None;
    }
    s.seen = st;
    let text = read(&p)?;
    if Some(&text) == s.good.as_ref() {
        return None;
    }
    s.pending = Some(text);
    Some("fx.wgsl: compiles on the next frame".into())
}

/// On a frame with the GPU: install what is pending.
pub fn sync(gpu: &Gpu) {
    let mut s = STATE.lock().unwrap();
    if !s.started && s.pending.is_none() {
        s.pending = Some(builtin());
    }
    s.started = true;
    let Some(new) = s.pending.take() else { return };
    match render::sprites::set_game_sprite(gpu, &new) {
        Ok(()) => {
            if s.good.is_some() {
                s.lines.push("fx.wgsl: compiled".into());
            }
            s.good = Some(new);
        }
        Err(e) => {
            s.lines.push(format!("fx.wgsl/spells.wgsl: {e} (the last good version keeps running)"));
            if s.good.is_none() && new != builtin() {
                s.pending = Some(builtin());
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
