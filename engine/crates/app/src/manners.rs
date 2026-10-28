//! Window manners under a GameNight host (the Godot SDK's `screen.gd`): get
//! off the screen on pause, take it back on start and resume, and keep
//! asking until the window server agrees. Window-mode changes are animated
//! and a request mid-animation can be dropped, so one call is not enough:
//! a game that asked once can stay on the TV after Back, or stay behind the
//! lobby after Resume.

use crate::Screen;
use std::time::{Duration, Instant};
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

pub(crate) struct Manners {
    pub screen: Screen,
    since: Instant,
    asked: Option<Instant>,
    settled: bool,
    parked: bool,
}

impl Manners {
    pub fn new() -> Self { Self { screen: Screen::Normal, since: Instant::now(), asked: None, settled: true, parked: false } }

    /// Window attributes for the first wish: a game that starts hidden (a
    /// warm launch) opens parked and inactive, so it cannot flash over the
    /// game being played or take its focus.
    pub fn first(&mut self, wish: Screen, attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
        if wish != Screen::Hidden { return attrs; }
        self.screen = Screen::Hidden;
        self.settled = false;
        self.parked = true;
        attrs.with_fullscreen(None).with_active(false).with_position(PARKED)
    }

    /// A new wish from the game. Returns true when it changed.
    pub fn wish(&mut self, wish: Screen, focused: bool) -> bool {
        if wish == self.screen { return false; }
        // Hand the foreground over before minimising: afterwards this
        // process can no longer give it to the lobby.
        if wish == Screen::Hidden && focused { allow_foreground(); }
        println!("window: {:?} -> {wish:?}", self.screen);
        self.screen = wish;
        self.since = Instant::now();
        self.asked = None;
        self.settled = false;
        true
    }

    /// Called every frame: ask again until the window is where it should be.
    pub fn step(&mut self, w: &Window, focused: bool, fullscreen: bool) {
        if self.settled { return; }
        let now = Instant::now();
        if self.asked.is_some_and(|t| now - t < RETRY) { return; }
        match self.screen {
            Screen::Hidden => {
                // No answer (some window servers can't tell): asked once is all we can do.
                let minimized = w.is_minimized();
                if minimized == Some(true) || (minimized.is_none() && self.asked.is_some()) { self.settled = true; return; }
                if now - self.since >= OFF_SCREEN_AFTER {
                    println!("window: could not minimise; parking it off-screen");
                    w.set_fullscreen(None);
                    w.set_outer_position(PARKED);
                    self.parked = true;
                    self.settled = true;
                    return;
                }
                w.set_minimized(true);
            }
            Screen::Fullscreen => {
                // Shown and fullscreen: one more raise, then leave it be.
                if w.is_minimized() != Some(true) && w.fullscreen().is_some() && !self.parked {
                    if !focused { w.focus_window(); }
                    self.settled = true;
                    return;
                }
                if now - self.since >= CLAIM_FOR {
                    println!("window: the window server kept the screen from us");
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

/// Windows keeps a background process from taking focus unless the
/// foreground app hands it over; the host ships a helper that does
/// (GAMENIGHT_FOREGROUND_HELPER --allow-foreground).
fn allow_foreground() {
    if !cfg!(windows) { return; }
    let Ok(helper) = std::env::var("GAMENIGHT_FOREGROUND_HELPER") else { return };
    if helper.is_empty() || !std::path::Path::new(&helper).is_file() { return; }
    let _ = std::process::Command::new(helper).arg("--allow-foreground").status();
}
