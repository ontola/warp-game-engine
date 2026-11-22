//! What every game that a party host launches does the same way: the host
//! prepares a session, the game reports ready and waits until `start`,
//! pause freezes, dispose goes back to waiting. [`Party`] holds the client
//! and that lifecycle; a game wraps it with its own settings, seats and the
//! events it emits. It has no screen or input types: see
//! `app::Screen::for_party` for the window and each game's `to_controls`.
use crate::Client;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Idle, Warm, Running, Paused }

/// `#rrggbb` as sRGB 0..1. Lenient: the `#` is optional, surrounding
/// whitespace is trimmed, and anything past six characters is ignored.
pub fn hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 { return None; }
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    Some([b(0)?, b(2)?, b(4)?])
}

/// `#rrggbb` as sRGB 0..1, exactly: the `#`, then six hex digits and nothing else.
pub fn hex_strict(s: &str) -> Option<[f32; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 || !s.bytes().all(|v| v.is_ascii_hexdigit()) { return None; }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some([16, 8, 0].map(|shift| ((n >> shift) & 255u32) as f32 / 255.))
}

pub struct Party {
    pub client: Client,
    pub session: String,
    pub phase: Phase,
    /// Set by [`Party::finished`] once per session; `prepare` clears it.
    pub finished: bool,
    /// Declared as `participation` for every prepared session
    /// ([`Party::offer_joins`]).
    joins: Option<bool>,
    /// Spare controllers' last buttons and whether a stick was pushed.
    spare: std::collections::BTreeMap<String, (u16, bool)>,
    hint_at: Option<std::time::Instant>,
    /// The people seated, by player id: a new one gets "joined the game".
    people: Vec<String>,
    notice: Option<String>,
}

impl Party {
    pub fn new(client: Client) -> Party { Party { client, session: String::new(), phase: Phase::Idle, finished: false, joins: None, spare: Default::default(), hint_at: None, people: Vec::new(), notice: None } }

    /// When launched by a party host (GAMENIGHT=1) or pointed at one (GAMENIGHT_ADDR).
    pub fn connect(game_id: &str) -> Option<Party> {
        match Client::connect_from_env(game_id) {
            Ok(Some(client)) => Some(Party::new(client)),
            Ok(None) => None,
            Err(e) => { eprintln!("splitscreen-club: {e}"); None }
        }
    }

    pub fn running(&self) -> bool { self.phase == Phase::Running }

    /// Startup progress for the host's loading screen.
    pub fn loading(&mut self, percent: u32, step: &str) { self.client.loading(percent, step); }

    /// The host asked for a session: tell it the match is being set up.
    /// Call before [`Party::prepare`].
    pub fn setting_up(&self, session: &str) { let _ = self.client.notify_progress(session, 85, "setting up the match"); }

    /// A new session is prepared: warm, not finished.
    pub fn prepare(&mut self, session: String) {
        if let Some(live) = self.joins { let _ = self.client.notify_participation(&session, live); }
        self.people = self.client.state.seats().iter().filter(|s| !s.ai && !s.player_id.is_empty()).map(|s| s.player_id.clone()).collect();
        self.spare.clear();
        self.session = session;
        self.phase = Phase::Warm;
        self.finished = false;
    }

    /// Mid-game joins (call before the first `prepare`): `live` takes
    /// people into the match in progress (the host sends their seat as
    /// `party_updated`, a [`crate::Command::Roster`]); false keeps the
    /// roster and they play from the next game. Either way the host seats
    /// a spare controller in the party when it presses A.
    pub fn offer_joins(&mut self, live: bool) { self.joins = Some(live); }

    /// Once per tick while a match runs: a controller nobody plays with that
    /// is touched shows how to join (`takes`: whether the game takes them
    /// now, else from the next game), as the browser host does. Only A
    /// joins, and the host does that (Joep: anything else only jiggles).
    pub fn watch_joins(&mut self, takes: bool) {
        let now = std::time::Instant::now();
        let spare = self.client.state.unseated(now);
        self.spare.retain(|token, _| spare.iter().any(|(t, _, _)| t == token));
        let running = self.running();
        for (token, buttons, axes) in spare {
            let pushed = (0..2).any(|k| axes[k * 2].hypot(axes[k * 2 + 1]) > 0.6);
            let (before, was) = self.spare.insert(token, (buttons, pushed)).unwrap_or((0, false));
            let pressed = buttons & !before;
            if !running || (pressed == 0 && !(pushed && !was)) { continue; }
            if pressed & crate::button::A != 0 {
                if !takes { self.notice = Some("You play from the next game".into()); }
                continue;
            }
            if self.hint_at.is_some_and(|t| now.duration_since(t).as_secs_f32() < 4.0) { continue; }
            self.hint_at = Some(now);
            self.notice = Some(if takes { "Press A to join this game".into() } else { "Press A to join; you play from the next game".into() });
        }
    }

    /// The seats after a `party_updated` ([`crate::Command::Roster`]):
    /// says "<name> joined the game" for someone new.
    pub fn seats_changed(&mut self, seats: &[crate::Seat]) {
        for s in seats.iter().filter(|s| !s.ai && !s.player_id.is_empty()) {
            if !self.people.contains(&s.player_id) {
                self.people.push(s.player_id.clone());
                if self.running() { self.notice = Some(format!("{} joined the game", s.name)); }
            }
        }
    }

    /// The line to show now, once ([`Party::watch_joins`], [`Party::seats_changed`]).
    pub fn take_notice(&mut self) -> Option<String> { self.notice.take() }

    /// Each of these applies only to the current session and says whether it did.
    pub fn start(&mut self, session: &str) -> bool { self.to(session, Phase::Running) }
    pub fn resume(&mut self, session: &str) -> bool { self.to(session, Phase::Running) }
    pub fn pause(&mut self, session: &str) -> bool { self.to(session, Phase::Paused) }

    /// Back to waiting: no session, idle.
    pub fn dispose(&mut self, session: &str) -> bool {
        if session != self.session { return false; }
        self.session.clear();
        self.phase = Phase::Idle;
        true
    }

    /// The host went away.
    pub fn disconnected(&mut self) { self.phase = Phase::Idle; }

    fn to(&mut self, session: &str, phase: Phase) -> bool {
        if session != self.session { return false; }
        self.phase = phase;
        true
    }

    /// The match is built: tell the host, then wait frozen for `start`.
    pub fn ready(&mut self) {
        let session = self.session.clone();
        let _ = self.client.notify_progress(&session, 100, "ready");
        if let Err(e) = self.client.notify_ready(&session) { eprintln!("splitscreen-club: {e}"); }
    }

    /// The match is over (once per session).
    pub fn finished(&mut self) {
        if self.finished || self.session.is_empty() { return; }
        self.finished = true;
        let _ = self.client.notify_finished(&self.session);
    }

    /// A line for the room log, while a session exists.
    pub fn room_log(&self, text: &str) {
        if !self.session.is_empty() {
            let _ = self.client.room_log(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Config};
    use serde_json::{json, Value};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    use tungstenite::Message;

    #[test]
    fn hex_is_lenient_and_hex_strict_is_exact() {
        assert_eq!(hex("#ff0080"), Some([1.0, 0.0, 128.0 / 255.0]));
        assert_eq!(hex(" 00ff00 "), Some([0.0, 1.0, 0.0]));
        assert_eq!(hex("#00ff00aa"), Some([0.0, 1.0, 0.0]));
        assert_eq!(hex(""), None);
        assert_eq!(hex("#12345"), None);
        assert_eq!(hex_strict("#1a80fe"), Some([26., 128., 254.].map(|x| x / 255.)));
        for bad in ["", "1a80fe", "#1a80f", "#1a80fe0", " #1a80fe", "#+a80fe", "#gg0000"] { assert_eq!(hex_strict(bad), None, "{bad:?}"); }
    }

    /// A host on loopback: prepare, expect progress 85 (the game's), then
    /// progress 100 and ready; walk start, pause, resume, dispose; a command
    /// for another session changes nothing; `finished` goes out once, and only with a session.
    #[test]
    fn a_session_prepares_runs_pauses_and_disposes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let host = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let read = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>| -> Value { loop { if let Message::Text(t) = ws.read().unwrap() { return serde_json::from_str(t.as_str()).unwrap(); } } };
            let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, v: Value| ws.send(Message::Text(v.to_string().into())).unwrap();
            assert_eq!(read(&mut ws)["type"], "hello");
            send(&mut ws, json!({"type":"welcome","protocol_version":1,"godot_sdk":6,"party":{}}));
            send(&mut ws, json!({"type":"prepare","game":"t","session":"s1","seats":[],"players":[]}));
            let sent: Vec<Value> = (0..3).map(|_| read(&mut ws)).collect();
            let kinds: Vec<_> = sent.iter().map(|m| m["type"].as_str().unwrap()).collect();
            assert_eq!(kinds, ["progress", "progress", "ready"]);
            assert_eq!((sent[0]["percent"].as_i64(), sent[1]["percent"].as_i64()), (Some(85), Some(100)));
            for (kind, session) in [("start", "s1"), ("pause", "s1"), ("resume", "s1"), ("pause", "s1"), ("dispose", "s1")] {
                send(&mut ws, json!({"type":kind,"session":session}));
                go_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            let finished = read(&mut ws);
            assert_eq!((finished["type"].as_str(), finished["session"].as_str()), (Some("finished"), Some("s2")));
        });
        let client = Client::connect(Config { url: format!("ws://{address}"), game_id: "t".into(), token: String::new(), launched_by_daemon: true }).unwrap();
        let mut party = Party::new(client);
        assert_eq!((party.phase, party.running()), (Phase::Idle, false));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = vec![];
        // Prepare first.
        'prepare: while Instant::now() < deadline {
            for c in party.client.poll() {
                if let Command::Prepare { session, .. } = c {
                    party.setting_up(&session);
                    party.prepare(session);
                    party.ready();
                    break 'prepare;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!((party.session.as_str(), party.phase), ("s1", Phase::Warm));
        let mut steps = 0;
        while steps < 5 && Instant::now() < deadline {
            for c in party.client.poll() {
                let applied = match &c {
                    Command::Start { session } => party.start(session),
                    Command::Pause { session } => party.pause(session),
                    Command::Resume { session } => party.resume(session),
                    Command::Dispose { session } => party.dispose(session),
                    _ => continue,
                };
                seen.push((applied, party.phase, party.session.clone()));
                steps += 1;
                go_tx.send(()).unwrap();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let s = |applied, phase, session: &str| (applied, phase, session.to_string());
        assert_eq!(seen, [s(true, Phase::Running, "s1"), s(true, Phase::Paused, "s1"), s(true, Phase::Running, "s1"), s(true, Phase::Paused, "s1"), s(true, Phase::Idle, "")]);
        // Commands for another session change nothing.
        party.prepare("s2".into());
        party.start("s2");
        for applied in [party.pause("other"), party.resume("other"), party.start("other"), party.dispose("other")] { assert!(!applied); }
        assert_eq!((party.session.as_str(), party.phase), ("s2", Phase::Running));
        assert!(party.dispose("s2"));
        // No session: nothing to finish, and nothing is sent.
        party.finished();
        assert!(!party.finished);
        party.prepare("s2".into());
        party.finished();
        party.finished();
        assert!(party.finished);
        party.prepare("s3".into());
        assert!(!party.finished, "a new session starts unfinished");
        party.disconnected();
        assert_eq!(party.phase, Phase::Idle);
        host.join().unwrap();
    }

    /// A spare controller in a running match: a touch shows how to join
    /// (A, never another button, is the join, which the host does), and
    /// the seat the host then sends says who joined.
    #[test]
    fn a_spare_controller_is_told_how_to_join_and_the_joiner_is_named() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let host = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let read = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>| -> Value { loop { if let Message::Text(t) = ws.read().unwrap() { return serde_json::from_str(t.as_str()).unwrap(); } } };
            let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, v: Value| ws.send(Message::Text(v.to_string().into())).unwrap();
            assert_eq!(read(&mut ws)["type"], "hello");
            send(&mut ws, json!({"type":"welcome","protocol_version":1,"godot_sdk":6,"party":{}}));
            send(&mut ws, json!({"type":"prepare","game":"t","session":"s1",
                "seats":[{"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}}],"players":[{"id":"p1","name":"Joep"}]}));
            let p = read(&mut ws);
            assert_eq!((p["type"].as_str(), p["session"].as_str(), p["instant_join"].as_bool()), (Some("participation"), Some("s1"), Some(true)));
            while read(&mut ws)["type"] != "ready" {}
            send(&mut ws, json!({"type":"start","session":"s1"}));
            go_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let frame = |b: u16| json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":1,"axes":[0,0,0,0,0,0]},{"controller":"pad-b","buttons":b,"axes":[0,0,0,0,0,0]}]});
            for b in [2, 0, 1] {
                send(&mut ws, frame(b));
                go_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            send(&mut ws, json!({"type":"party_updated","session":"s1",
                "seats":[{"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}},{"index":1,"controller":"pad-b","occupant":{"kind":"local","player_id":"p2"}}],
                "players":[{"id":"p1","name":"Joep"},{"id":"p2","name":"Nora"}]}));
            go_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let client = Client::connect(Config { url: format!("ws://{address}"), game_id: "t".into(), token: String::new(), launched_by_daemon: true }).unwrap();
        let mut party = Party::new(client);
        party.offer_joins(true);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut notices = vec![];
        let mut wanted = 0;
        let mut step = |party: &mut Party, want: fn(&Command) -> bool| {
            while Instant::now() < deadline {
                let mut hit = false;
                for c in party.client.poll() {
                    match &c {
                        Command::Prepare { session, .. } => { party.prepare(session.clone()); party.ready(); }
                        Command::Start { session } => { party.start(session); }
                        Command::Roster { seats } => party.seats_changed(seats),
                        _ => {}
                    }
                    hit |= want(&c);
                }
                if hit { return; }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("timed out");
        };
        step(&mut party, |c| matches!(c, Command::Start { .. }));
        go_tx.send(()).unwrap();
        for _ in 0..3 {
            // Frames make no command: wait until this frame is in.
            let before = party.client.state.unseated(Instant::now());
            let t = Instant::now();
            while Instant::now() < deadline && t.elapsed() < Duration::from_millis(100) {
                party.client.poll();
                if party.client.state.unseated(Instant::now()) != before { break; }
                std::thread::sleep(Duration::from_millis(2));
            }
            party.watch_joins(true);
            notices.push(party.take_notice());
            wanted += 1;
            go_tx.send(()).unwrap();
        }
        assert_eq!(wanted, 3);
        step(&mut party, |c| matches!(c, Command::Roster { .. }));
        notices.push(party.take_notice());
        go_tx.send(()).unwrap();
        assert_eq!(notices, [Some("Press A to join this game".to_string()), None, None, Some("Nora joined the game".to_string())],
            "B shows the hint, A says nothing until the seat comes, then the joiner is named");
        host.join().unwrap();
    }
}
