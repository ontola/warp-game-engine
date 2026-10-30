//! GameNight: a party host runs Growing Guns, as Godot's
//! `gamenight_bridge.gd` does. The host prepares a session with its seats;
//! the game builds the match (local seats are players, AI seats bots, at
//! least two in all), reports progress and waits, minimised and frozen,
//! until `start`. Then it takes the screen and the party's controllers drive
//! the seats; pause freezes everyone, dispose goes back to waiting. The
//! party's settings (Godot's `gamenight_settings.gd`) set the match and the
//! next round's rules.
use app::{Buttons, Controls};
use gamenight::{button, Client, Command, Pad, Seat};
use growing_guns::{Modifier, Rules, World, MODIFIERS};
use serde_json::{json, Value};
use std::time::Instant;

/// Godot `gamenight_settings.gd` SPECS.
fn specs() -> Value {
    json!([
        {"key":"rounds_to_win", "label":"Rounds to win (next match)", "kind":"number", "default":10, "min":1, "max":30},
        {"key":"modifier_chance", "label":"Modifiers % (next round)", "kind":"number", "default":30, "min":0, "max":100},
        {"key":"card_pick_seconds", "label":"Card time, s (next pick)", "kind":"number", "default":10, "min":3, "max":30},
        {"key":"gravity", "label":"Gravity % (next round)", "kind":"number", "default":100, "min":25, "max":175},
        {"key":"bullet_drop", "label":"Bullet drop % (next round)", "kind":"number", "default":100, "min":0, "max":200},
        {"key":"body_damage", "label":"Body shot damage % (next round)", "kind":"number", "default":100, "min":50, "max":200},
        {"key":"pickup_rate", "label":"Random pickup rate % (next round)", "kind":"number", "default":100, "min":0, "max":200}
    ])
}

/// The "Next round" pick: a modifier for one round, as buttons in the app.
pub const NEXT_MODIFIER: &str = "next_modifier";
const RANDOM: &str = "Random";
const NO_MODIFIER: &str = "None";

/// "LOW GRAVITY" as the app's button: "Low gravity".
fn modifier_option(m: Modifier) -> String {
    let t = m.title().to_lowercase();
    let mut c = t.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

fn next_modifier_spec() -> Value {
    let mut options = vec![RANDOM.to_string(), NO_MODIFIER.to_string()];
    options.extend(MODIFIERS.iter().map(|m| modifier_option(m.0)));
    json!({"key": NEXT_MODIFIER, "label": "Next round", "kind": "choice", "default": RANDOM, "options": options,
        "description": "A modifier for the next round only. Random rolls as usual."})
}

/// An option of [`next_modifier_spec`]: None for Random, `Some(None)` for no modifier.
pub fn parse_next_modifier(option: &str) -> Result<Option<Option<Modifier>>, String> {
    if option == RANDOM { return Ok(None); }
    if option == NO_MODIFIER { return Ok(Some(None)); }
    MODIFIERS.iter().find(|m| modifier_option(m.0) == option).map(|m| Some(Some(m.0))).ok_or_else(|| format!("unknown modifier {option:?}"))
}

/// Each player's own stick sensitivity, in percent (scope player: saved
/// with their profile, set in the app's "My settings").
pub const STICK: &str = "stick_sensitivity";

fn stick_spec() -> Value {
    json!({"key": STICK, "label": "Stick sensitivity %", "kind": "number", "default": 100, "min": 30, "max": 250, "scope": "player",
        "description": "How fast your controller's right stick turns your view."})
}

/// The spec labels, in `specs()` order.
const LABELS: [&str; 7] = ["Rounds to win (next match)", "Modifiers % (next round)", "Card time, s (next pick)", "Gravity % (next round)",
    "Bullet drop % (next round)", "Body shot damage % (next round)", "Random pickup rate % (next round)"];

/// The party's settings: whole numbers within each spec's range.
#[derive(Clone, Debug)]
pub struct Settings(Vec<(String, i64, i64, i64)>);

impl Default for Settings {
    fn default() -> Self {
        Settings(specs().as_array().unwrap().iter().map(|s| (s["key"].as_str().unwrap().to_string(), s["default"].as_i64().unwrap(), s["min"].as_i64().unwrap(), s["max"].as_i64().unwrap())).collect())
    }
}

impl Settings {
    /// Godot `change`: whole numbers in range only.
    pub fn change(&mut self, key: &str, value: &Value) -> bool {
        let Some(v) = value.as_f64() else { return false };
        let Some(e) = self.0.iter_mut().find(|e| e.0 == key) else { return false };
        if !v.is_finite() || v.fract() != 0.0 || (v as i64) < e.2 || (v as i64) > e.3 { return false; }
        e.1 = v as i64;
        true
    }
    pub fn value(&self, key: &str) -> i64 { self.0.iter().find(|e| e.0 == key).map_or(0, |e| e.1) }
    pub fn key(&self, k: usize) -> Option<String> { self.0.get(k).map(|e| e.0.clone()) }
    /// Every setting as the ADVANCED menu shows it: label, value, min, max.
    pub fn entries(&self) -> Vec<(&'static str, i64, i64, i64)> { self.0.iter().zip(LABELS).map(|(e, l)| (l, e.1, e.2, e.3)).collect() }
    /// Match settings (a new match).
    pub fn apply_match(&self, w: &mut World) { w.rounds_to_win = self.value("rounds_to_win") as u32; self.apply_round(w); }
    /// Next-round settings.
    pub fn apply_round(&self, w: &mut World) {
        let pct = |k: &str| self.value(k) as f32 / 100.0;
        w.modifier_chance = pct("modifier_chance");
        w.card_pick_seconds = self.value("card_pick_seconds") as f64;
        w.rules = Rules { gravity: pct("gravity"), bullet_drop: pct("bullet_drop"), body_damage: pct("body_damage"), pickup_rate: pct("pickup_rate") };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Idle, Warm, Running, Paused }

/// What the game should do after a poll.
pub enum Ev {
    /// Build this match: `humans` seated players (names), `bots` more.
    Prepare { humans: Vec<String>, bots: usize },
    Names(Vec<String>),
    /// Seated players' own stick sensitivity (seat order, multiplier).
    Sticks(Vec<(usize, f32)>),
    /// The app picked the next round's modifier (None: back to random).
    NextModifier(Option<Option<Modifier>>),
    Dispose,
}

pub struct Party {
    client: Client,
    pub settings: Settings,
    pub session: String,
    pub phase: Phase,
    /// The seated humans, in slot order.
    seats: Vec<Seat>,
    finished: bool,
    triggers: Vec<u32>,
    /// Live control for the session assistant: its description and token file.
    live: Option<(String, std::path::PathBuf)>,
    /// The app's "Next round" pick, until that round starts.
    pub next_modifier: Option<Option<Modifier>>,
    /// Players' own stick sensitivity (multiplier), by player id.
    sticks: std::collections::HashMap<String, f32>,
}

impl Party {
    /// When launched by a party host (GAMENIGHT=1) or pointed at one (GAMENIGHT_ADDR).
    pub fn connect() -> Option<Party> {
        match Client::connect_from_env("growing-guns") {
            Ok(Some(client)) => Some(Party { client, settings: Settings::default(), session: String::new(), phase: Phase::Idle, seats: Vec::new(), finished: false, triggers: Vec::new(), live: None, next_modifier: None, sticks: Default::default() }),
            Ok(None) => None,
            Err(e) => { eprintln!("gamenight: {e}"); None }
        }
    }

    /// Under a daemon the window stays minimised until a match starts.
    pub fn screen(&self) -> app::Screen {
        if !self.client.launched_by_daemon { return app::Screen::Normal; }
        if self.phase == Phase::Running { app::Screen::Fullscreen } else { app::Screen::Hidden }
    }

    /// Startup progress for the host's loading screen ([`app::Game::loading`]).
    pub fn loading(&mut self, percent: u32, step: &str) { self.client.loading(percent, step); }

    pub fn running(&self) -> bool { self.phase == Phase::Running }

    fn humans(seats: &[Seat]) -> Vec<Seat> { seats.iter().filter(|s| !s.ai).take(4).cloned().collect() }

    pub fn poll(&mut self) -> Vec<Ev> {
        let mut out = Vec::new();
        loop {
            let cmds = self.client.poll();
            if cmds.is_empty() { break; }
            for c in cmds {
                match c {
                    Command::Connected => { self.declare(); }
                    Command::Setting { key, value } if key == NEXT_MODIFIER => match parse_next_modifier(value.as_str().unwrap_or("")) {
                        Ok(m) => { self.next_modifier = m; out.push(Ev::NextModifier(m)); }
                        Err(e) => eprintln!("gamenight: {e}"),
                    },
                    Command::Setting { key, value } => match crate::knobs::from_setting(&key, &value) {
                        // A knob: written into its mod, which reloads by itself.
                        Some(Ok(file)) => println!("[gamenight] {key} = {value} (mods/{file})"),
                        Some(Err(e)) => eprintln!("gamenight: {e}"),
                        None => if !self.settings.change(&key, &value) { eprintln!("gamenight: ignored setting {key} = {value}"); },
                    },
                    Command::PlayerSettings { player_id, values } => {
                        if let Some(v) = values.get(STICK).and_then(Value::as_f64).filter(|v| v.is_finite()) {
                            self.sticks.insert(player_id, (v as f32 / 100.0).clamp(0.3, 2.5));
                            out.push(self.sticks_ev());
                        }
                    }
                    Command::Prepare { session, seats } => {
                        println!("[gamenight] prepare {session} ({} seats)", seats.len());
                        let _ = self.client.notify_progress(&session, 85, "setting up the match");
                        self.seats = Self::humans(&seats);
                        // Versus needs two: bots fill the gap (Godot `_configure_meta_from_seats`).
                        let ai = seats.iter().filter(|s| s.ai).count();
                        let bots = ai.max(2usize.saturating_sub(self.seats.len()));
                        out.push(Ev::Prepare { humans: self.seats.iter().map(|s| s.name.clone()).collect(), bots });
                        out.push(self.sticks_ev());
                        self.session = session;
                        self.phase = Phase::Warm;
                        self.finished = false;
                    }
                    Command::Start { session } if session == self.session => { println!("[gamenight] start {session}"); self.phase = Phase::Running; }
                    Command::Pause { session } if session == self.session => self.phase = Phase::Paused,
                    Command::Resume { session } if session == self.session => self.phase = Phase::Running,
                    Command::Dispose { session } if session == self.session => {
                        self.session.clear();
                        self.phase = Phase::Idle;
                        self.seats.clear();
                        out.push(Ev::Dispose);
                    }
                    Command::Roster { seats } | Command::PlayersUpdated { seats } => {
                        // Names and colours follow the lobby; the seats driving the match stay.
                        let named = Self::humans(&seats);
                        for s in &mut self.seats { if let Some(n) = named.iter().find(|n| n.index == s.index) { s.name = n.name.clone(); s.color = n.color.clone(); } }
                        out.push(Ev::Names(self.seats.iter().map(|s| s.name.clone()).collect()));
                    }
                    Command::Disconnected(e) => { eprintln!("gamenight: {e}"); self.phase = Phase::Idle; }
                    Command::Error(e) => eprintln!("gamenight: {e}"),
                    _ => {}
                }
            }
        }
        out
    }

    /// The settings (the party's and the knobs as percent sliders) and,
    /// with live control on, the session assistant's way in.
    fn declare(&self) {
        let mut all = vec![next_modifier_spec(), stick_spec()];
        all.extend(specs().as_array().cloned().unwrap_or_default());
        for k in crate::knobs::settings() {
            all.push(json!({"key": k.key, "label": k.label, "kind": "number", "default": k.value, "min": k.min, "max": k.max}));
        }
        let _ = self.client.declare_settings(Value::Array(all));
        if let Some((description, file)) = &self.live && let Err(e) = self.client.declare_commands(description, &[], Some(file)) { eprintln!("gamenight: {e}"); }
    }

    /// Let the session assistant change the game through live control.
    pub fn offer_live(&mut self, description: String, token_file: std::path::PathBuf) {
        self.live = Some((description, token_file));
        if self.client.state.connected { self.declare(); }
    }

    /// A knob changed in the game (live tools, console): the app's slider follows.
    pub fn knob_changed(&mut self, name: &str, value: &str) {
        let Ok(v) = value.trim().parse::<f64>() else { return };
        let key = format!("{}{name}", crate::knobs::SETTING_PREFIX);
        if let Err(e) = self.client.send_json(json!({"type": "set_setting", "key": key, "value": (v * 100.0).round() as i64})) { eprintln!("gamenight: {e}"); }
    }

    fn sticks_ev(&self) -> Ev {
        Ev::Sticks(self.seats.iter().enumerate().filter_map(|(k, s)| self.sticks.get(&s.player_id).map(|&v| (k, v))).collect())
    }

    /// Seat `k` changed their stick sensitivity in the game's menu: saved
    /// with their profile, and the app shows it.
    pub fn set_stick(&mut self, k: usize, v: f32) {
        let Some(seat) = self.seats.get(k) else { return };
        if self.sticks.get(&seat.player_id).is_some_and(|&old| (old - v).abs() < 0.005) { return; }
        self.sticks.insert(seat.player_id.clone(), v);
        let msg = json!({"type": "set_player_setting", "player_id": seat.player_id, "key": STICK, "value": (v * 100.0).round() as i64});
        if let Err(e) = self.client.send_json(msg) { eprintln!("gamenight: {e}"); }
    }

    /// The picked round has started: the app's buttons go back to Random.
    pub fn next_modifier_used(&mut self) {
        if self.next_modifier.take().is_some() && let Err(e) = self.client.send_json(json!({"type": "set_setting", "key": NEXT_MODIFIER, "value": RANDOM})) { eprintln!("gamenight: {e}"); }
    }

    /// Change setting `k` from the game's own menu (Godot `_gn_open_advanced`):
    /// here at once, and through GameNight so the phones and lobby show it.
    pub fn set(&mut self, k: usize, value: i64) {
        let Some(key) = self.settings.0.get(k).map(|e| e.0.clone()) else { return };
        if self.settings.change(&key, &json!(value)) && let Err(e) = self.client.send_json(json!({"type": "set_setting", "key": key, "value": value})) { eprintln!("gamenight: {e}"); }
    }

    /// The match is built: tell the host, then wait frozen for `start`.
    pub fn ready(&mut self) {
        let session = self.session.clone();
        let _ = self.client.notify_progress(&session, 100, "ready");
        if let Err(e) = self.client.notify_ready(&session) { eprintln!("gamenight: {e}"); }
    }

    /// The match is over (once per session).
    pub fn finished(&mut self) {
        if self.finished || self.session.is_empty() { return; }
        self.finished = true;
        let _ = self.client.notify_finished(&self.session);
    }

    /// Controls for each seated player this tick, from the party's controllers.
    pub fn controls(&mut self, players: usize, dt: f32) -> Vec<Controls> {
        let now = Instant::now();
        let _ = self.client.report_activity();
        self.triggers.resize(players, 0);
        (0..players).map(|k| {
            let Some(seat) = self.seats.get(k) else { return Controls::default() };
            let pad = self.client.state.pad_for_seat(seat.index, now, false);
            let c = to_controls(&pad, self.triggers[k], dt);
            self.triggers[k] = c.held.0 & (Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
            c
        }).collect()
    }
}

/// A controller frame as the shell's pad input (same buttons, sticks and
/// look rate as a local pad: 3 rad/s, response curve 1.5).
pub fn to_controls(pad: &Pad, prev_triggers: u32, dt: f32) -> Controls {
    let map = [(button::A, Buttons::SOUTH), (button::B, Buttons::EAST), (button::X, Buttons::WEST), (button::Y, Buttons::NORTH),
        (button::LB, Buttons::LEFT_SHOULDER), (button::RB, Buttons::RIGHT_SHOULDER), (button::START, Buttons::START),
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
    fn stick_sensitivity_is_each_players_own_setting() {
        let spec = stick_spec();
        assert_eq!((spec["scope"].as_str(), spec["default"].as_i64(), spec["min"].as_i64(), spec["max"].as_i64()), (Some("player"), Some(100), Some(30), Some(250)));
    }

    #[test]
    fn the_next_round_pick_is_one_round_of_that_modifier() {
        let spec = next_modifier_spec();
        let options: Vec<&str> = spec["options"].as_array().unwrap().iter().map(|o| o.as_str().unwrap()).collect();
        assert_eq!(&options[..3], &["Random", "None", "Low gravity"]);
        assert!(options.contains(&"Blackout"));
        for o in &options { parse_next_modifier(o).expect(o); }
        assert_eq!(parse_next_modifier("Blackout"), Ok(Some(Some(Modifier::Blackout))));
        let mut w = World::new_with(5, &[true; 2], |w| w.modifier_chance = 0.0);
        w.next_modifier = parse_next_modifier("Blackout").unwrap();
        w.next_round();
        assert_eq!((w.modifier, w.next_modifier), (Some(Modifier::Blackout), None));
        w.next_round();
        assert_eq!(w.modifier, None);
    }

    #[test]
    fn settings_take_whole_numbers_in_range_and_set_the_world() {
        let mut s = Settings::default();
        assert!(s.change("gravity", &json!(50)));
        assert!(!s.change("gravity", &json!(10)));
        assert!(!s.change("gravity", &json!(50.5)));
        assert!(!s.change("nope", &json!(1)));
        assert!(s.change("rounds_to_win", &json!(3)));
        let mut w = World::new(1, &[false, true]);
        s.apply_match(&mut w);
        assert_eq!(w.rounds_to_win, 3);
        assert_eq!(w.rules.gravity, 0.5);
        assert_eq!(w.card_pick_seconds, 10.0);
        assert!((w.modifier_chance - 0.3).abs() < 1e-6);
    }

    /// A fake party host on loopback: prepare one couch seat and one AI seat,
    /// expect `ready`, start, then a controller frame firing the gun.
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
            let declared = read(&mut ws);
            assert_eq!(declared["type"], "declare_settings");
            assert!(declared["settings"].as_array().unwrap().iter().any(|s| s["key"] == STICK && s["scope"] == "player"));
            send(&mut ws, json!({"type":"setting_changed","key":"rounds_to_win","value":4}));
            send(&mut ws, json!({"type":"prepare","game":"growing-guns","session":"s1",
                "seats":[{"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}},{"index":1,"occupant":{"kind":"ai"}}],
                "players":[{"id":"p1","name":"Joep","couch":0}]}));
            let mut got = vec![];
            while !got.contains(&"ready".to_string()) { got.push(read(&mut ws)["type"].as_str().unwrap().to_string()); }
            assert!(got.contains(&"progress".to_string()));
            send(&mut ws, json!({"type":"player_settings","player_id":"p1","values":{"stick_sensitivity":150}}));
            send(&mut ws, json!({"type":"start","session":"s1"}));
            for _ in 0..200 {
                send(&mut ws, json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":0,"axes":[0,-32767,0,0,0,32767]}]}));
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // The game's own menu changed it: back into the profile.
            loop {
                let m = read(&mut ws);
                if m["type"] == "set_player_setting" { assert_eq!((m["player_id"].as_str(), m["key"].as_str(), m["value"].as_i64()), (Some("p1"), Some(STICK), Some(80))); break; }
            }
        });
        let client = Client::connect(gamenight::Config { url: format!("ws://{address}"), game_id: "growing-guns".into(), token: String::new(), launched_by_daemon: true }).unwrap();
        let mut party = Party { client, settings: Settings::default(), session: String::new(), phase: Phase::Idle, seats: Vec::new(), finished: false, triggers: Vec::new(), live: None, next_modifier: None, sticks: Default::default() };
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let (mut prepared, mut fired, mut stick) = (None, false, None);
        assert_eq!(party.screen(), app::Screen::Hidden);
        while Instant::now() < deadline && !(fired && stick.is_some()) {
            for ev in party.poll() {
                match ev {
                    Ev::Prepare { humans, bots } => { prepared = Some((humans, bots)); party.ready(); }
                    Ev::Sticks(s) if !s.is_empty() => stick = Some(s),
                    _ => {}
                }
            }
            if party.running() {
                let c = party.controls(1, 1.0 / 60.0);
                fired = c[0].held.has(Buttons::RIGHT_TRIGGER) && c[0].mv[1] > 0.9;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(prepared, Some((vec!["Joep".to_string()], 1)));
        assert_eq!(party.settings.value("rounds_to_win"), 4);
        assert!(fired, "the seat's controller drives the player");
        assert_eq!(stick, Some(vec![(0, 1.5)]));
        party.set_stick(0, 0.8);
        assert_eq!(party.screen(), app::Screen::Fullscreen);
        host.join().unwrap();
    }

    #[test]
    fn pads_map_to_the_shells_buttons_and_sticks() {
        let pad = Pad { buttons: button::A | button::RB, pressed: button::A, axes: [1.0, -1.0, 0.0, -1.0, 0.0, 1.0] };
        let c = to_controls(&pad, 0, 1.0 / 60.0);
        assert!(c.held.has(Buttons::SOUTH) && c.held.has(Buttons::RIGHT_SHOULDER) && c.held.has(Buttons::RIGHT_TRIGGER));
        assert!(c.pressed.has(Buttons::SOUTH) && c.pressed.has(Buttons::RIGHT_TRIGGER) && !c.pressed.has(Buttons::RIGHT_SHOULDER));
        assert!(c.mv[0] > 0.6 && c.mv[1] > 0.6, "up on the stick is forward: {:?}", c.mv);
        assert!((c.look[1] - 3.0 / 60.0).abs() < 1e-4, "full up looks up at 3 rad/s: {:?}", c.look);
        // A held trigger is pressed once.
        assert!(!to_controls(&pad, Buttons::RIGHT_TRIGGER, 1.0 / 60.0).pressed.has(Buttons::RIGHT_TRIGGER));
    }
}
