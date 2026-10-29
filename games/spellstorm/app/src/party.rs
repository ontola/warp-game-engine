//! GameNight, as main.gd's bridge: the host prepares a session with its
//! seats, the game seats a wizard for each (couch seats are players, AI seats
//! bots), starts a run, reports ready and waits frozen until `start`. The
//! party's controllers drive the wizards; pause freezes them, dispose waits
//! again. Names follow the lobby live, the party's settings set friendly
//! fire, difficulty and unlocks, and a phone's "New run" starts one.
use app::{Buttons, Controls};
use gamenight::{button, Client, Command, Pad, Seat};
use serde_json::json;
use std::time::Instant;

pub const GAME_ID: &str = "spellstorm";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Warm,
    Running,
    Paused,
}

/// What the game should do after a poll.
#[derive(Debug, PartialEq)]
pub enum Ev {
    /// Seat these and start a run: (party seat, name, colour, player id, AI).
    Prepare(Vec<Seated>),
    /// Names picked up in the lobby: player id and name.
    Names(Vec<(String, String)>),
    Setting(String, String),
    NewRun,
    Dispose,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seated {
    pub seat: usize,
    pub name: String,
    pub color: Option<[f32; 3]>,
    pub player_id: String,
    pub bot: bool,
}

pub struct Party {
    client: Client,
    pub session: String,
    pub phase: Phase,
    finished: bool,
    triggers: Vec<u32>,
}

/// `#rrggbb` as sRGB 0..1.
fn hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    Some([b(0)?, b(2)?, b(4)?])
}

fn seated(seats: &[Seat]) -> Vec<Seated> {
    seats.iter().map(|s| Seated { seat: s.index, name: s.name.clone(), color: hex(&s.color), player_id: s.player_id.clone(), bot: s.ai }).collect()
}

impl Party {
    /// When launched by a party host (GAMENIGHT=1) or pointed at one
    /// (GAMENIGHT_ADDR).
    pub fn connect() -> Option<Party> {
        match Client::connect_from_env(GAME_ID) {
            Ok(Some(client)) => Some(Party::new(client)),
            Ok(None) => None,
            Err(e) => {
                eprintln!("gamenight: {e}");
                None
            }
        }
    }

    pub fn new(client: Client) -> Party {
        Party { client, session: String::new(), phase: Phase::Idle, finished: false, triggers: vec![] }
    }

    /// Under a daemon the window stays minimised until a run starts.
    pub fn screen(&self) -> app::Screen {
        if !self.client.launched_by_daemon {
            return app::Screen::Normal;
        }
        // A host pause (Back, Alt+Tab) hands the screen to the lobby; staying
        // fullscreen made the lobby and the game fight over the screen.
        if self.phase == Phase::Running { app::Screen::Fullscreen } else { app::Screen::Hidden }
    }

    pub fn running(&self) -> bool {
        self.phase == Phase::Running
    }

    /// main.gd _declare.
    fn declare(&self) {
        let _ = self.client.declare_settings(json!([
            {"key": "friendly_fire", "label": "Friendly fire", "kind": "choice", "default": "half", "options": ["off", "half", "full"]},
            {"key": "difficulty", "label": "Difficulty", "kind": "choice", "default": "normal", "options": ["easy", "normal", "hard"]},
            {"key": "unlocks", "label": "Unlocks", "kind": "choice", "default": "earned", "options": ["earned", "all"]},
        ]));
        let _ = self.client.declare_actions(json!([
            {"key": "new_run", "label": "New run", "description": "Ends this run for everyone and starts a fresh one.", "confirm": true},
        ]));
    }

    pub fn poll(&mut self) -> Vec<Ev> {
        let mut out = vec![];
        loop {
            let cmds = self.client.poll();
            if cmds.is_empty() {
                break;
            }
            for c in cmds {
                match c {
                    Command::Connected => self.declare(),
                    Command::Prepare { session, seats } => {
                        println!("[gamenight] prepare {session} ({} seats)", seats.len());
                        let _ = self.client.notify_progress(&session, 5, "starting the game");
                        out.push(Ev::Prepare(seated(&seats)));
                        self.session = session;
                        self.phase = Phase::Warm;
                        self.finished = false;
                    }
                    Command::Start { session } if session == self.session => {
                        println!("[gamenight] start {session}");
                        self.phase = Phase::Running;
                    }
                    Command::Resume { session } if session == self.session => {
                        println!("[gamenight] resume {session}");
                        self.phase = Phase::Running;
                    }
                    Command::Pause { session } if session == self.session => {
                        println!("[gamenight] pause {session}");
                        self.phase = Phase::Paused;
                    }
                    Command::Dispose { session } if session == self.session => {
                        println!("[gamenight] dispose {session}");
                        self.session.clear();
                        self.phase = Phase::Idle;
                        out.push(Ev::Dispose);
                    }
                    Command::PlayersUpdated { seats } | Command::Roster { seats } if !self.session.is_empty() => {
                        out.push(Ev::Names(seats.iter().filter(|s| !s.player_id.is_empty()).map(|s| (s.player_id.clone(), s.name.clone())).collect()));
                    }
                    Command::Setting { key, value } => {
                        if let Some(v) = value.as_str() {
                            out.push(Ev::Setting(key, v.to_string()));
                        }
                    }
                    Command::Action { key, .. } if key == "new_run" => out.push(Ev::NewRun),
                    Command::Disconnected(e) => {
                        eprintln!("gamenight: {e}");
                        self.phase = Phase::Idle;
                    }
                    Command::Error(e) => eprintln!("gamenight: {e}"),
                    _ => {}
                }
            }
        }
        out
    }

    /// The run is built: tell the host, then wait frozen for `start`.
    pub fn ready(&mut self) {
        let s = self.session.clone();
        let _ = self.client.notify_progress(&s, 100, "ready");
        if let Err(e) = self.client.notify_ready(&s) {
            eprintln!("gamenight: {e}");
        }
    }

    /// The run is over (once per session).
    pub fn finished(&mut self) {
        if self.finished || self.session.is_empty() {
            return;
        }
        self.finished = true;
        let _ = self.client.notify_finished(&self.session);
    }

    pub fn room_log(&self, text: &str) {
        if !self.session.is_empty() {
            let _ = self.client.room_log(text);
        }
    }

    /// Esc: the host's overlay.
    pub fn overlay(&self) {
        let _ = self.client.request_overlay();
    }

    /// Controls for each party seat this tick; `any_phase` reads them while
    /// paused too.
    pub fn controls(&mut self, seats: &[usize], dt: f32, any_phase: bool) -> Vec<Controls> {
        let now = Instant::now();
        if self.running() {
            let _ = self.client.report_activity();
        }
        self.triggers.resize(seats.len(), 0);
        seats
            .iter()
            .enumerate()
            .map(|(k, &seat)| {
                let pad = self.client.state.pad_for_seat(seat, now, any_phase);
                let c = to_controls(&pad, self.triggers[k], dt);
                self.triggers[k] = c.held.0 & (Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
                c
            })
            .collect()
    }
}

/// A controller frame as the shell's pad input: the same buttons and sticks
/// as a local pad, the triggers as buttons past half way.
pub fn to_controls(pad: &Pad, prev_triggers: u32, _dt: f32) -> Controls {
    let map = [
        (button::A, Buttons::SOUTH),
        (button::B, Buttons::EAST),
        (button::X, Buttons::WEST),
        (button::Y, Buttons::NORTH),
        (button::LB, Buttons::LEFT_SHOULDER),
        (button::RB, Buttons::RIGHT_SHOULDER),
        (button::START, Buttons::START),
        (button::LS, Buttons::LEFT_STICK),
        (button::RS, Buttons::RIGHT_STICK),
        (button::UP, Buttons::UP),
        (button::DOWN, Buttons::DOWN),
        (button::LEFT, Buttons::LEFT),
        (button::RIGHT, Buttons::RIGHT),
    ];
    let bits = |b: u16| map.iter().filter(|(g, _)| b & g != 0).fold(0u32, |a, (_, o)| a | o);
    let mut held = bits(pad.buttons);
    if pad.trigger(0) > 0.5 {
        held |= Buttons::LEFT_TRIGGER;
    }
    if pad.trigger(1) > 0.5 {
        held |= Buttons::RIGHT_TRIGGER;
    }
    let pressed = bits(pad.pressed) | (held & (Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER) & !prev_triggers);
    // Raw sticks: pad.gd's own dead zone shapes them (play::pad).
    let l = [pad.axes[0], pad.axes[1]];
    let r = [pad.axes[2], pad.axes[3]];
    Controls { mv: [l[0] as f32, -l[1] as f32], look: [0.0; 2], aim: [r[0] as f32, -r[1] as f32], held: Buttons(held), pressed: Buttons(pressed) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// A fake party host on loopback: settings, prepare one couch seat and
    /// one AI seat, expect `ready`, start, then a controller frame aiming.
    #[test]
    fn a_party_prepares_starts_and_drives_its_seat() {
        use std::net::TcpListener;
        use tungstenite::Message;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let read = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>| -> Value { loop { if let Message::Text(t) = ws.read().unwrap() { return serde_json::from_str(t.as_str()).unwrap(); } } };
            assert_eq!(read(&mut ws)["type"], "hello");
            let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, v: Value| ws.send(Message::Text(v.to_string().into())).unwrap();
            send(&mut ws, json!({"type":"welcome","protocol_version":1,"godot_sdk":6,"party":{}}));
            assert_eq!(read(&mut ws)["type"], "declare_settings");
            assert_eq!(read(&mut ws)["type"], "declare_actions");
            send(&mut ws, json!({"type":"setting_changed","key":"difficulty","value":"hard"}));
            send(&mut ws, json!({"type":"prepare","game":GAME_ID,"session":"s1",
                "seats":[{"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}},{"index":1,"occupant":{"kind":"ai"}}],
                "players":[{"id":"p1","name":"Joep","couch":0}]}));
            let mut got = vec![];
            while !got.contains(&"ready".to_string()) { got.push(read(&mut ws)["type"].as_str().unwrap().to_string()); }
            assert!(got.contains(&"progress".to_string()));
            send(&mut ws, json!({"type":"start","session":"s1"}));
            for _ in 0..200 {
                send(&mut ws, json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":0,"axes":[0,0,32767,0,0,0]}]}));
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        let client = Client::connect(gamenight::Config { url: format!("ws://{address}"), game_id: GAME_ID.into(), token: String::new(), launched_by_daemon: true }).unwrap();
        let mut party = Party::new(client);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let (mut prepared, mut setting, mut aimed) = (None, None, false);
        assert_eq!(party.screen(), app::Screen::Hidden);
        while Instant::now() < deadline && !aimed {
            for ev in party.poll() {
                match ev {
                    Ev::Prepare(s) => {
                        prepared = Some(s);
                        party.ready();
                    }
                    Ev::Setting(k, v) => setting = Some((k, v)),
                    _ => {}
                }
            }
            if party.running() {
                aimed = party.controls(&[0], 1.0 / 60.0, false)[0].aim[0] > 0.9;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let seats = prepared.expect("prepared");
        assert_eq!(seats.len(), 2);
        assert_eq!((seats[0].name.as_str(), seats[0].player_id.as_str(), seats[0].bot), ("Joep", "p1", false));
        assert!(seats[1].bot);
        assert_eq!(setting, Some(("difficulty".into(), "hard".into())));
        assert!(aimed, "the seat's controller aims the wizard");
        assert_eq!(party.screen(), app::Screen::Fullscreen);
        host.join().unwrap();
    }
}
