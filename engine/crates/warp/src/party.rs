//! Splitscreen Club: a party host runs a Luau game. The host prepares a session
//! with its seats; the game starts over with those players (local seats are
//! people, AI seats bots, and `min_players` fills up with bots), reports
//! ready and waits hidden until `start`. Then it takes the screen and the
//! party's controllers drive the players. Pause freezes the game, dispose
//! goes back to waiting. Names and colours follow the lobby live
//! (`players_updated`); a seat that joins or leaves mid-session joins or
//! leaves the game without a restart.
use app::{Buttons, Controls};
use splitscreen_club::party::hex;
use splitscreen_club::{button, Command, Pad, Seat};
use std::path::PathBuf;
use std::time::Instant;

pub use splitscreen_club::party::Phase;

/// A seated person or AI seat, as the game sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct SeatInfo { pub seat: usize, pub name: String, pub color: Option<[f32; 3]>, pub bot: bool, pub couch: Option<i64>, pub skin: String, pub avatar: String }

impl SeatInfo {
    fn from(s: &Seat) -> Self { SeatInfo { seat: s.index, name: s.name.clone(), color: hex(&s.color), bot: s.ai, couch: s.couch, skin: s.skin_color.clone(), avatar: s.avatar.clone() } }
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
    host: splitscreen_club::party::Party,
    triggers: Vec<u32>,
    /// Live control to offer the session assistant: what it can change and
    /// the token file. Declared again after every (re)connect.
    live: Option<(String, PathBuf)>,
}

impl Party {
    /// When launched by a party host (GAMENIGHT=1) or pointed at one (GAMENIGHT_ADDR).
    pub fn connect(game_id: &str) -> Option<Party> {
        splitscreen_club::party::Party::connect(game_id).map(Party::from_host)
    }

    pub fn new(client: splitscreen_club::Client) -> Party { Party::from_host(splitscreen_club::party::Party::new(client)) }

    fn from_host(host: splitscreen_club::party::Party) -> Party { Party { host, triggers: Vec::new(), live: None } }

    pub fn phase(&self) -> Phase { self.host.phase }

    /// Under a daemon the window is on screen only while the match runs: a
    /// pause from the host (Back, Alt+Tab) hands the screen to the lobby.
    /// The game's own Start menu is not a host pause and stays on screen.
    pub fn screen(&self) -> app::Screen {
        app::Screen::for_party(self.host.client.launched_by_daemon, self.host.running())
    }

    pub fn running(&self) -> bool { self.host.running() }

    /// Startup progress for the host's loading screen ([`app::Game::loading`]).
    pub fn loading(&mut self, percent: u32, step: &str) { self.host.loading(percent, step); }

    fn seats(seats: &[Seat]) -> Vec<SeatInfo> { seats.iter().map(SeatInfo::from).collect() }

    pub fn poll(&mut self) -> Vec<Ev> {
        let mut out = Vec::new();
        loop {
            let cmds = self.host.client.poll();
            if cmds.is_empty() { break; }
            for c in cmds {
                match c {
                    Command::Prepare { session, seats } => {
                        println!("{} [splitscreen-club] prepare {session} ({} seats)", app::manners::stamp(), seats.len());
                        self.host.setting_up(&session);
                        out.push(Ev::Prepare(Self::seats(&seats)));
                        self.host.prepare(session);
                    }
                    Command::Start { session } => if self.host.start(&session) { println!("{} [splitscreen-club] start {session}", app::manners::stamp()); },
                    Command::Pause { session } => if self.host.pause(&session) { println!("{} [splitscreen-club] pause", app::manners::stamp()); },
                    Command::Resume { session } => if self.host.resume(&session) { println!("{} [splitscreen-club] resume", app::manners::stamp()); },
                    Command::Dispose { session } => if self.host.dispose(&session) {
                        println!("{} [splitscreen-club] dispose", app::manners::stamp());
                        out.push(Ev::Dispose);
                    },
                    Command::Roster { seats } | Command::PlayersUpdated { seats } if !self.host.session.is_empty() => out.push(Ev::Seats(Self::seats(&seats))),
                    Command::Connected => self.declare_live(),
                    Command::Disconnected(e) => { eprintln!("splitscreen-club: {e}"); self.host.disconnected(); }
                    Command::Error(e) => eprintln!("splitscreen-club: {e}"),
                    _ => {}
                }
            }
        }
        out
    }

    /// The game is built: tell the host, then wait frozen for `start`.
    pub fn ready(&mut self) { self.host.ready(); }

    /// The match is over (once per session): the host moves on.
    pub fn finished(&mut self) { self.host.finished(); }

    /// Let the session assistant change the game through live control.
    pub fn offer_live(&mut self, description: String, token_file: PathBuf) {
        self.live = Some((description, token_file));
        if self.host.client.state.connected { self.declare_live(); }
    }

    fn declare_live(&self) {
        let Some((description, file)) = &self.live else { return };
        if let Err(e) = self.host.client.declare_commands(description, &[], Some(file)) { eprintln!("splitscreen-club: {e}"); }
    }

    pub fn room_log(&self, text: &str) { let _ = self.host.client.room_log(text); }

    pub fn rumble(&mut self, seat: usize, weak: f64, strong: f64, seconds: f64) { let _ = self.host.client.rumble(seat, weak, strong, seconds); }

    /// Controls for each seat in `seats` this tick. `any_phase` also reads
    /// them while paused (the game's own menu).
    pub fn controls(&mut self, seats: &[usize], dt: f32, any_phase: bool) -> Vec<Controls> {
        let now = Instant::now();
        if self.running() { let _ = self.host.client.report_activity(); }
        self.triggers.resize(seats.len(), 0);
        seats.iter().enumerate().map(|(k, &seat)| {
            let pad = self.host.client.state.pad_for_seat(seat, now, any_phase);
            let c = to_controls(&pad, self.triggers[k], dt);
            self.triggers[k] = c.held.0 & (Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
            c
        }).collect()
    }
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
    Controls { mv: [l[0] as f32, -l[1] as f32], look: [(r[0] * k) as f32, (-r[1] * k) as f32], aim: [r[0] as f32, -r[1] as f32], triggers: [pad.trigger(0) as f32, pad.trigger(1) as f32], held: Buttons(held), pressed: Buttons(pressed) }
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
