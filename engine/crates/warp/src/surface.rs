//! A game's own surface shader: `surface.wgsl` in the game folder replaces
//! the renderer's game_surface.wgsl (see render::set_game_surface), and
//! entities pick one of its kinds with `surface = N`. An edited file
//! recompiles on the next frame; one that does not compile keeps the last
//! good version and reports the compiler's message.
use std::path::PathBuf;
use std::time::SystemTime;

/// The engine's copy: keeps the material as it is.
pub const BUILT_IN: &str = include_str!("../../render/src/shaders/game_surface.wgsl");
pub const FILE: &str = "surface.wgsl";

pub struct GameSurface {
    path: PathBuf,
    stamp: Option<SystemTime>,
    /// The source the renderer runs now.
    good: String,
    /// An edit waiting for the next frame.
    pending: Option<String>,
    pub error: Option<String>,
}

impl GameSurface {
    pub fn new(dir: &std::path::Path) -> Self {
        let mut s = GameSurface { path: dir.join(FILE), stamp: None, good: BUILT_IN.to_owned(), pending: None, error: None };
        s.scan();
        s
    }

    /// Notice an edited (or new, or removed) surface.wgsl.
    pub fn scan(&mut self) -> Option<String> {
        let stamp = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if stamp == self.stamp { return None; }
        self.stamp = stamp;
        let text = match stamp { Some(_) => std::fs::read_to_string(&self.path).ok()?, None => BUILT_IN.to_owned() };
        if text == self.good && self.error.is_none() { return None; }
        self.pending = Some(text);
        Some(format!("{FILE}: compiles on the next frame"))
    }

    /// Before the renderer is (re)built: the source it should compile.
    /// With a pending edit, try that one; `built` reports whether it compiled.
    pub fn pending(&self) -> bool { self.pending.is_some() }

    /// Build or reload with the pending source; on a compiler error put the
    /// last good one back and run `retry` with it. Returns a log line.
    pub fn apply(&mut self, mut build: impl FnMut() -> Result<(), String>) -> Option<String> {
        let new = self.pending.take()?;
        render::set_game_surface(&new);
        match build() {
            Ok(()) => { self.good = new; self.error = None; Some(format!("{FILE}: compiled")) }
            Err(e) => {
                render::set_game_surface(&self.good);
                let line = format!("{FILE}: {e}");
                self.error = Some(line.clone());
                if let Err(e) = build() { return Some(format!("{line}; the last good version failed too: {e}")); }
                Some(format!("{line} (the last good version keeps running)"))
            }
        }
    }

    /// Set the current source before the first renderer.
    pub fn install(&self) { render::set_game_surface(self.pending.as_deref().unwrap_or(&self.good)); }
}
