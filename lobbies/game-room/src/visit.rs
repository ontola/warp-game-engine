//! Visit another room (Joep, 26 November 2026): a code field over the
//! room's QR code. The mouse clicks it and the keyboard types the code; a
//! pad opens it from its Start menu ("Visit another room") and spells the
//! code with the stick. Enter (or A on the last letter) goes there: the
//! browser host follows that room (`/play/?follow=CODE`); the desktop host
//! has no online play of its own yet, so it opens that page in the browser.

use crate::live::Lobby;
use crate::lobby::{DIM, GREEN};
use app::TextEvent;
use game_room_sim::world::{BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_START, BTN_UP, BTN_X};
use serde_json::json;

/// Room codes are four letters and digits (KJX4).
pub const LEN: usize = 4;
/// What a pad spells with: no 0/O or 1/I mix-ups are ruled out here, the
/// host's codes use them all.
pub const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Where the desktop host sends a visit: the browser host follows the room.
fn play_url() -> String {
    std::env::var("GAMENIGHT_PLAY_URL").unwrap_or_else(|_| "https://splitscreen.club/play/".into())
}

#[derive(Clone, Debug, Default)]
pub struct Visit {
    /// The field has the focus (typing or spelling).
    pub open: bool,
    /// The code so far, upper case.
    pub code: String,
    /// The pad player spelling the code; None: the keyboard types it.
    pub owner: Option<String>,
    /// The pad's letter for the next slot (an index into [`CHARS`]).
    pub pick: usize,
    stick: bool,
    stick_x: bool,
}

impl Visit {
    /// Keep letters and digits, upper case, up to [`LEN`].
    fn push(&mut self, text: &str) {
        for ch in text.chars().filter(|c| c.is_ascii_alphanumeric()) {
            if self.code.len() >= LEN { break; }
            self.code.push(ch.to_ascii_uppercase());
        }
    }
}

impl Lobby {
    /// The field takes the keyboard: typed keys go to it, not to players.
    pub fn visit_typing(&self) -> bool { self.visit.open && self.visit.owner.is_none() }

    /// Open the field; `owner`, a pad player spelling with the stick.
    pub fn open_visit(&mut self, owner: Option<String>) {
        self.visit = Visit { open: true, owner, ..Default::default() };
        self.screens_dirty = true;
    }

    pub fn close_visit(&mut self) {
        self.visit = Visit::default();
        self.screens_dirty = true;
    }

    /// The keyboard typing into the field.
    pub fn visit_text(&mut self, event: TextEvent) {
        if !self.visit_typing() { return; }
        match event {
            TextEvent::Insert(text) | TextEvent::Paste(text) => self.visit.push(&text),
            TextEvent::Backspace | TextEvent::Delete => { self.visit.code.pop(); }
            TextEvent::Enter => self.submit_visit(),
            TextEvent::Escape => self.close_visit(),
            _ => {}
        }
    }

    /// A pad spelling the code: up and down pick the letter, A types it
    /// (and on the last one goes), B or X deletes, and with nothing typed
    /// B closes. Start goes once the code is whole.
    pub(crate) fn visit_pad(&mut self, pressed: u32, stick: [f32; 2]) {
        let v = &mut self.visit;
        let n = CHARS.len();
        let up = pressed & BTN_UP != 0 || (stick[1] < -0.6 && !v.stick);
        let down = pressed & BTN_DOWN != 0 || (stick[1] > 0.6 && !v.stick);
        let left = pressed & BTN_LEFT != 0 || (stick[0] < -0.6 && !v.stick_x);
        let right = pressed & BTN_RIGHT != 0 || (stick[0] > 0.6 && !v.stick_x);
        v.stick = stick[1].abs() > 0.6;
        v.stick_x = stick[0].abs() > 0.6;
        if up { v.pick = (v.pick + n - 1) % n; }
        if down { v.pick = (v.pick + 1) % n; }
        if left || right { v.pick = (v.pick + if left { n - 10 } else { 10 }) % n; }
        if pressed & (BTN_B | BTN_X) != 0 {
            if v.code.pop().is_none() && pressed & BTN_B != 0 { self.close_visit(); }
            return;
        }
        if pressed & BTN_A != 0 {
            if v.code.len() < LEN { v.code.push(CHARS[v.pick] as char); }
            if v.code.len() == LEN { self.submit_visit(); }
            return;
        }
        if pressed & BTN_START != 0 && v.code.len() == LEN { self.submit_visit(); }
    }

    /// Go to the room the code names.
    fn submit_visit(&mut self) {
        if self.visit.code.len() < LEN { return; }
        let code = std::mem::take(&mut self.visit.code);
        self.close_visit();
        if code == self.show.room_code {
            self.toast("That's this room's code.", DIM);
            return;
        }
        if let Some(out) = &mut self.outbox {
            out.push(json!({"type": "visit_room", "code": code}));
            self.toast(format!("Visiting room {code}…"), GREEN);
            return;
        }
        let url = format!("{}?follow={code}", play_url());
        if open_in_browser(&url) { self.toast(format!("Room {code} opens in your browser."), GREEN); }
        else { self.toast(format!("Open {url} to visit room {code}."), DIM); }
    }
}

/// The system browser on `url`; false where there is none to start.
fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "android")]
    let cmd: Result<(), ()> = { app::android::open_url(url); Ok(()) };
    #[cfg(target_arch = "wasm32")]
    let cmd: Result<(), ()> = { let _ = url; Err(()) };
    cmd.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_keeps_four_letters_and_digits_upper_case() {
        let mut v = Visit::default();
        v.push("kj-x 4z9");
        assert_eq!(v.code, "KJX4");
    }
}
