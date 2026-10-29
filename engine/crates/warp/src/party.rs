//! GameNight: a party host runs a Luau game. The host prepares a session
//! with its seats; the game starts over with those players (local seats are
//! people, AI seats bots, and `min_players` fills up with bots), reports
//! ready and waits hidden until `start`. Then it takes the screen and the
//! party's controllers drive the players. Pause freezes the game, dispose
//! goes back to waiting. Names and colours follow the lobby live
//! (`players_updated`); a seat that joins or leaves mid-session joins or
//! leaves the game without a restart.
use app::{Buttons, Controls};
use gamenight::{button, Client, Command, Pad, Seat};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Idle, Warm, Running, Paused }

/// A seated person or AI seat, as the game sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct SeatInfo { pub seat: usize, pub name: String, pub color: Option<[f32; 3]>, pub bot: bool, pub couch: Option<i64> }

impl SeatInfo {
    fn from(s: &Seat) -> Self { SeatInfo { seat: s.index, name: s.name.clone(), color: hex(&s.color), bot: s.ai, couch: s.couch } }
}

/// What the game should do after a poll.
#[derive(Debug, PartialEq)]
pub enum Ev {
    /// Start over with these seats.
    Prepare(Vec<SeatInfo>),
    /// The seats now (names, colours, joins and leaves), mid-session.
    Seats(Vec<SeatInfo>),
    Dispose,
}

pub struct Party {
    client: Client,
    pub session: String,
    pub phase: Phase,
    finished: bool,
    triggers: Vec<u32>,
    /// Live control to offer the session assistant: what it can change and
    /// the token file. Declared again after every (re)connect.
    live: Option<(String, PathBuf)>,
}

impl Party {
    /// When launched by a party host (GAMENIGHT=1) or pointed at one (GAMENIGHT_ADDR).
    pub fn connect(game_id: &str) -> Option<Party> {
        match Client::connect_from_env(game_id) {
            Ok(Some(client)) => Some(Party::new(client)),
            Ok(None) => None,
            Err(e) => { eprintln!("gamenight: {e}"); None }
        }
    }

    pub fn new(client: Client) -> Party { Party { client, session: String::new(), phase: Phase::Idle, finished: false, triggers: Vec::new(), live: None } }

    /// Under a daemon the window is on screen only while the match runs: a
    /// pause from the host (Back, Alt+Tab) hands the screen to the lobby.
    /// The game's own Start menu is not a host pause and stays on screen.
    pub fn screen(&self) -> app::Screen {
        if !self.client.launched_by_daemon { return app::Screen::Normal; }
        if self.phase == Phase::Running { app::Screen::Fullscreen } else { app::Screen::Hidden }
    }

    pub fn running(&self) -> bool { self.phase == Phase::Running }

    fn seats(seats: &[Seat]) -> Vec<SeatInfo> { seats.iter().map(SeatInfo::from).collect() }

    pub fn poll(&mut self) -> Vec<Ev> {
        let mut out = Vec::new();
        loop {
            let cmds = self.client.poll();
            if cmds.is_empty() { break; }
            for c in cmds {
                match c {
                    Command::Prepare { session, seats } => {
                        println!("{} [gamenight] prepare {session} ({} seats)", app::manners::stamp(), seats.len());
                        let _ = self.client.notify_progress(&session, 5, "starting the game");
                        out.push(Ev::Prepare(Self::seats(&seats)));
                        self.session = session;
                        self.phase = Phase::Warm;
                        self.finished = false;
                    }
                    Command::Start { session } if session == self.session => { println!("{} [gamenight] start {session}", app::manners::stamp()); self.phase = Phase::Running; }
                    Command::Pause { session } if session == self.session => { println!("{} [gamenight] pause", app::manners::stamp()); self.phase = Phase::Paused; }
                    Command::Resume { session } if session == self.session => { println!("{} [gamenight] resume", app::manners::stamp()); self.phase = Phase::Running; }
                    Command::Dispose { session } if session == self.session => {
                        println!("{} [gamenight] dispose", app::manners::stamp());
                        self.session.clear();
                        self.phase = Phase::Idle;
                        out.push(Ev::Dispose);
                    }
                    Command::Roster { seats } | Command::PlayersUpdated { seats } if !self.session.is_empty() => out.push(Ev::Seats(Self::seats(&seats))),
                    Command::Connected => self.declare_live(),
                    Command::Disconnected(e) => { eprintln!("gamenight: {e}"); self.phase = Phase::Idle; }
                    Command::Error(e) => eprintln!("gamenight: {e}"),
                    _ => {}
                }
            }
        }
        out
    }

    /// The game is built: tell the host, then wait frozen for `start`.
    pub fn ready(&mut self) {
        let session = self.session.clone();
        let _ = self.client.notify_progress(&session, 100, "ready");
        if let Err(e) = self.client.notify_ready(&session) { eprintln!("gamenight: {e}"); }
    }

    /// The match is over (once per session): the host moves on.
    pub fn finished(&mut self) {
        if self.finished || self.session.is_empty() { return; }
        self.finished = true;
        let _ = self.client.notify_finished(&self.session);
    }

    /// Let the session assistant change the game through live control.
    pub fn offer_live(&mut self, description: String, token_file: PathBuf) {
        self.live = Some((description, token_file));
        if self.client.state.connected { self.declare_live(); }
    }

    fn declare_live(&self) {
        let Some((description, file)) = &self.live else { return };
        if let Err(e) = self.client.declare_commands(description, &[], Some(file)) { eprintln!("gamenight: {e}"); }
    }

    pub fn room_log(&self, text: &str) { let _ = self.client.room_log(text); }

    pub fn rumble(&mut self, seat: usize, weak: f64, strong: f64, seconds: f64) { let _ = self.client.rumble(seat, weak, strong, seconds); }

    /// Controls for each seat in `seats` this tick. `any_phase` also reads
    /// them while paused (the game's own menu).
    pub fn controls(&mut self, seats: &[usize], dt: f32, any_phase: bool) -> Vec<Controls> {
        let now = Instant::now();
        if self.running() { let _ = self.client.report_activity(); }
        self.triggers.resize(seats.len(), 0);
        seats.iter().enumerate().map(|(k, &seat)| {
            let pad = self.client.state.pad_for_seat(seat, now, any_phase);
            let c = to_controls(&pad, self.triggers[k], dt);
            self.triggers[k] = c.held.0 & (Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
            c
        }).collect()
    }
}

/// `#rrggbb` as sRGB 0..1.
fn hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 { return None; }
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    Some([b(0)?, b(2)?, b(4)?])
}

/// A controller frame as the shell's pad input (same buttons, sticks and
/// look rate as a local pad: 3 rad/s, response curve 1.5), as Growing Guns.
pub fn to_controls(pad: &Pad, prev_triggers: u32, dt: f32) -> Controls {
    let map = [(button::A, Buttons::SOUTH), (button::B, Buttons::EAST), (button::X, Buttons::WEST), (button::Y, Buttons::NORTH),
        (button::LB, Buttons::LEFT_SHOULDER), (button::RB, Buttons::RIGHT_SHOULDER), (button::START, Buttons::START), (button::BACK, Buttons::SELECT),
        (button::LS, Buttons::LEFT_STICK), (button::RS, Buttons::RIGHT_STICK),
        (button::UP, Buttons::UP), (button::DOWN, Buttons::DOWN), (button::LEFT, Buttons::LEFT), (button::RIGHT, Buttons::RIGHT)];
    let bits = |b: u16| map.iter().filter(|(g, _)| b & g != 0).fold(0u32, |a, (_, o)| a | o);
    let mut held = bits(pad.buttons);
    if pad.trigger(0) > 0.5 { held |= Buttons::LEFT_TRIGGER; }
    if pad.trigger(1) > 0.5 { held |= Buttons::RIGHT_TRIGGER; }
    let pressed = bits(pad.pressed) | (held & (Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER) & !prev_triggers);
    let l = pad.stick(0);
    let r = pad.stick(1);
    let m = r[0].hypot(r[1]);
    let k = if m > 0.0 { m.powf(1.5) / m * 3.0 * dt as f64 } else { 0.0 };
    Controls { mv: [l[0] as f32, -l[1] as f32], look: [(r[0] * k) as f32, (-r[1] * k) as f32], aim: [r[0] as f32, -r[1] as f32], held: Buttons(held), pressed: Buttons(pressed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_and_pads() {
        assert_eq!(hex("#ff0080"), Some([1.0, 0.0, 128.0 / 255.0]));
        assert_eq!(hex(""), None);
        let pad = Pad { buttons: button::A | button::START, pressed: button::START, axes: [0.0, -1.0, 0.0, 0.0, 0.0, 1.0] };
        let c = to_controls(&pad, 0, 1.0 / 60.0);
        assert!(c.held.has(Buttons::SOUTH) && c.pressed.has(Buttons::START) && c.held.has(Buttons::RIGHT_TRIGGER));
        assert!(c.mv[1] > 0.9, "up on the stick is forward");
        let up = Pad { axes: [0.0, 0.0, 0.0, -1.0, 0.0, 0.0], ..pad };
        assert!(to_controls(&up, 0, 1.0 / 60.0).aim[1] > 0.99, "the right stick's position, y up");
    }
}
