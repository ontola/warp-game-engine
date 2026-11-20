//! Window manners under a Splitscreen Club host (the Godot SDK's `screen.gd`): get
//! off the screen on pause, take it back on start and resume, and keep
//! asking until the window server agrees. Window-mode changes are animated
//! and a request mid-animation can be dropped, so one call is not enough:
//! a game that asked once can stay on the TV after Back, or stay behind the
//! lobby after Resume.

use crate::Screen;
use std::time::Duration;
use web_time::Instant;
use winit::dpi::PhysicalPosition;
use winit::window::{Fullscreen, Window};

/// How long to let a window-mode change land before asking again.
const RETRY: Duration = Duration::from_millis(250);
/// Still not minimised after this: park the window off the display. The
/// party's lobby must never end up behind a dismissed game.
const OFF_SCREEN_AFTER: Duration = Duration::from_secs(8);
/// Stop asking for the screen after this; the window server said no.
const CLAIM_FOR: Duration = Duration::from_secs(6);
/// Where a parked window goes.
const PARKED: PhysicalPosition<i32> = PhysicalPosition::new(-30000, -30000);

/// Also used by games with their own winit loop (Sideways).
pub struct Manners {
    pub screen: Screen,
    since: Instant,
    asked: Option<Instant>,
    settled: bool,
    parked: bool,
    /// The window has been shown. It opens invisible and appears with its
    /// first frame, never as an empty (white) rectangle.
    shown: bool,
}

impl Default for Manners {
    fn default() -> Self { Self::new() }
}

impl Manners {
    pub fn new() -> Self { Self { screen: Screen::Normal, since: Instant::now(), asked: None, settled: true, parked: false, shown: false } }

    /// Window attributes for the first wish: the window opens invisible and
    /// shows itself with its first presented frame (`presented`). A game
    /// that starts hidden (a warm launch) also opens inactive, so it cannot
    /// take the focus from the game being played.
    pub fn first(&mut self, wish: Screen, attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
        let attrs = attrs.with_visible(false);
        // Loading stays in the taskbar until a correctly sized frame is ready.
        self.since = Instant::now();
        if wish != Screen::Hidden { return attrs; }
        self.screen = Screen::Hidden;
        attrs.with_fullscreen(None).with_active(false)
    }

    /// Not on screen: never shown yet, or hidden. Such a window may get no
    /// redraw events, so the shell drives its frames itself.
    pub fn offscreen(&self) -> bool { !self.shown || self.screen == Screen::Hidden }

    /// A frame of `surface` size was presented: show a window that is due
    /// to appear once its surface matches the window (no stretched or
    /// empty first frame).
    pub fn presented(&mut self, w: &Window, surface: [u32; 2]) {
        if self.shown || self.screen == Screen::Hidden { return; }
        let size = w.inner_size();
        if [size.width, size.height] != surface { return; }
        w.set_visible(true);
        crate::startup::ready();
        println!("{} window ({}): shown with its first frame, {}x{}", stamp(), process(), surface[0], surface[1]);
        if self.screen == Screen::Fullscreen { w.focus_window(); }
        self.shown = true;
        self.settled = false;
        self.asked = None;
    }

    /// The surface gave no frame because the window is occluded. A window
    /// that was never shown is occluded by being invisible (Metal reports
    /// it so), so it would wait forever for its first frame: show it now.
    pub fn occluded(&mut self, w: &Window) {
        if self.shown || self.screen == Screen::Hidden { return; }
        w.set_visible(true);
        crate::startup::ready();
        println!("{} window ({}): shown before its first frame (surface occluded while invisible)", stamp(), process());
        if self.screen == Screen::Fullscreen { w.focus_window(); }
        self.shown = true;
        self.settled = false;
        self.asked = None;
    }

    /// A new wish from the game. Returns true when it changed.
    pub fn wish(&mut self, wish: Screen, focused: bool) -> bool {
        if wish == self.screen { return false; }
        // Hand the foreground over before minimising or stepping back (a
        // lobby behind the game being resumed): afterwards this process can
        // no longer give it away.
        if wish != Screen::Fullscreen && focused { allow_foreground(); }
        println!("{} window ({}): {:?} -> {wish:?}", stamp(), process(), self.screen);
        self.screen = wish;
        self.since = Instant::now();
        self.asked = None;
        self.settled = false;
        true
    }

    /// Called every frame: ask again until the window is where it should be.
    pub fn step(&mut self, w: &Window, focused: bool, fullscreen: bool) {
        // A window shown by its owner (Sideways shows its own).
        if !self.shown && w.is_visible() == Some(true) { self.shown = true; }
        if self.settled { return; }
        let now = Instant::now();
        if self.asked.is_some_and(|t| now - t < RETRY) { return; }
        match self.screen {
            Screen::Hidden => {
                // Never shown: invisible is as off-screen as it gets.
                if !self.shown { self.settled = true; return; }
                // No answer (some window servers can't tell): asked once is all we can do.
                let minimized = w.is_minimized();
                if minimized == Some(true) || (minimized.is_none() && self.asked.is_some()) { self.settled = true; return; }
                if now - self.since >= OFF_SCREEN_AFTER {
                    println!("{} window ({}): could not minimise; parking it off-screen", stamp(), process());
                    w.set_fullscreen(None);
                    w.set_outer_position(PARKED);
                    self.parked = true;
                    self.settled = true;
                    return;
                }
                w.set_minimized(true);
            }
            Screen::Fullscreen if !self.shown => {
                // Size it while invisible; `presented` shows it.
                if w.fullscreen().is_none() { w.set_fullscreen(Some(Fullscreen::Borderless(self.monitor(w)))); }
            }
            Screen::Normal if !self.shown => {
                if w.fullscreen().is_some() != fullscreen { w.set_fullscreen(fullscreen.then(|| Fullscreen::Borderless(self.monitor(w)))); }
            }
            Screen::Fullscreen => {
                // Shown and fullscreen: one more raise, then leave it be.
                if w.is_minimized() != Some(true) && w.fullscreen().is_some() && !self.parked && fills(w) {
                    if !focused { w.focus_window(); }
                    self.settled = true;
                    return;
                }
                // Made fullscreen while invisible, some window servers keep
                // the old size: leave fullscreen and ask again.
                if w.fullscreen().is_some() && !fills(w) {
                    let (s, m) = (w.inner_size(), w.current_monitor().map(|m| m.size()).unwrap_or_default());
                    println!("{} window ({}): {}x{} on a {}x{} screen; fullscreen again", stamp(), process(), s.width, s.height, m.width, m.height);
                    w.set_fullscreen(None);
                }
                if now - self.since >= CLAIM_FOR {
                    println!("{} window ({}): the window server kept the screen from us", stamp(), process());
                    self.settled = true;
                    return;
                }
                w.set_minimized(false);
                w.set_fullscreen(Some(Fullscreen::Borderless(self.monitor(w))));
                self.parked = false;
                w.focus_window();
            }
            Screen::Normal => {
                w.set_minimized(false);
                if fullscreen && w.fullscreen().is_some() && !fills(w) { w.set_fullscreen(None); }
                w.set_fullscreen(fullscreen.then(|| Fullscreen::Borderless(self.monitor(w))));
                if self.parked && !fullscreen { w.set_outer_position(PhysicalPosition::new(64, 64)); }
                self.parked = false;
                self.settled = true;
            }
        }
        self.asked = Some(now);
    }

    /// A parked window has no monitor of its own; use the primary one.
    fn monitor(&self, w: &Window) -> Option<winit::monitor::MonitorHandle> {
        if self.parked { w.primary_monitor() } else { None }
    }
}

/// This executable's name, to tell a game's window lines from the lobby's.
pub fn process() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default())
}

/// The window covers its monitor (true when that cannot be told).
fn fills(w: &Window) -> bool {
    let Some(m) = w.current_monitor() else { return true };
    let (s, m) = (w.inner_size(), m.size());
    m.width == 0 || (s.width >= m.width && s.height >= m.height)
}

/// The time of day (UTC, as the host's log) for log lines that must line up
/// with the host's: `10:47:08.421`.
pub fn stamp() -> String {
    let ms = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) % 86_400_000;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Windows keeps a background process from taking focus unless the
/// foreground app hands it over. This process is in front, so it hands it
/// over itself, in place. It used to start the host's helper
/// (GAMENIGHT_FOREGROUND_HELPER --allow-foreground) and wait for it: on the
/// Mancave that took about a second, on every Back, with the game frozen on
/// the TV in the meantime (Godot lobbies still need the helper).
fn allow_foreground() {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn AllowSetForegroundWindow(process_id: u32) -> i32;
        }
        // ASFW_ANY: the lobby's process id is not known here.
        unsafe { AllowSetForegroundWindow(u32::MAX); }
    }
}
