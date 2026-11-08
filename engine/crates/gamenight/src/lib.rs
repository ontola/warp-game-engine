//! A native GameNight client for Warp games, matching the Godot SDK 6 wire
//! contract (extracted from Sideways).
//! Transport runs on its own thread; no socket wait blocks the renderer. Merely
//! launching a standalone playtest does not connect it to the household party.
//! Back belongs to the host. Controller identities come from seat tokens, never
//! native device indices. Launch tokens are never included in logs or errors.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use tungstenite::{Message, stream::MaybeTlsStream};

pub mod lobby;
pub mod artwork;

const STALE: Duration = Duration::from_millis(250);
const BUTTON_MASK: u16 = 0x3fff;

#[derive(Clone)]
pub struct Config {
    pub url: String,
    pub game_id: String,
    pub token: String,
    pub launched_by_daemon: bool,
}

impl Config {
    /// From GAMENIGHT, GAMENIGHT_ADDR, GAMENIGHT_GAME_ID (else `game_id`) and
    /// GAMENIGHT_TOKEN. None when neither the daemon nor an address is set.
    pub fn from_env(game_id: &str) -> Result<Option<Self>, String> {
        let launched_by_daemon = std::env::var("GAMENIGHT").as_deref() == Ok("1");
        let addr = std::env::var("GAMENIGHT_ADDR").unwrap_or_default();
        if !launched_by_daemon && addr.is_empty() { return Ok(None); }
        let addr = if addr.is_empty() { "127.0.0.1:7912" } else { &addr };
        let url = if addr.contains("://") { addr.into() } else { format!("ws://{addr}") };
        Ok(Some(Self { url, game_id: std::env::var("GAMENIGHT_GAME_ID").unwrap_or_else(|_| game_id.into()),
            token: std::env::var("GAMENIGHT_TOKEN").unwrap_or_default(), launched_by_daemon }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub index: usize,
    pub player_id: String,
    pub name: String,
    pub color: String,
    pub controller: String,
    pub ai: bool,
    pub couch: Option<i64>,
}

#[derive(Clone, Debug)]
pub enum Command {
    Connected,
    Prepare { session: String, seats: Vec<Seat> },
    Start { session: String },
    Pause { session: String },
    Resume { session: String },
    Dispose { session: String },
    Roster { seats: Vec<Seat> },
    PlayersUpdated { seats: Vec<Seat> },
    Setting { key: String, value: Value },
    PlayerSettings { player_id: String, values: Value },
    Action { key: String, player_id: String },
    Disconnected(String),
    Error(String),
}

/// Button bits of a controller frame (Back belongs to GameNight).
pub mod button {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const X: u16 = 1 << 2;
    pub const Y: u16 = 1 << 3;
    pub const LB: u16 = 1 << 4;
    pub const RB: u16 = 1 << 5;
    pub const BACK: u16 = 1 << 6;
    pub const START: u16 = 1 << 7;
    pub const LS: u16 = 1 << 8;
    pub const RS: u16 = 1 << 9;
    pub const UP: u16 = 1 << 10;
    pub const DOWN: u16 = 1 << 11;
    pub const LEFT: u16 = 1 << 12;
    pub const RIGHT: u16 = 1 << 13;
}

/// One seat's controller this tick: buttons held (minus any still held from
/// the lobby), pressed since last tick, and axes in -1..1 (left x, left y,
/// right x, right y, left trigger, right trigger; up is negative y).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    pub buttons: u16,
    pub pressed: u16,
    pub axes: [f64; 6],
}

impl Pad {
    /// A stick with a radial deadzone (0.15), rescaled from 0. `side` 0 is left.
    pub fn stick(&self, side: usize) -> [f64; 2] { radial([self.axes[side * 2], self.axes[side * 2 + 1]], 0.15) }
    /// A trigger 0..1 with a 0.06 deadzone. `side` 0 is left.
    pub fn trigger(&self, side: usize) -> f64 { let x = self.axes[4 + side].max(0.0); if x <= 0.06 { 0.0 } else { (x - 0.06) / 0.94 } }
    pub fn held(&self, b: u16) -> bool { self.buttons & b != 0 }
    pub fn just(&self, b: u16) -> bool { self.pressed & b != 0 }
}

#[derive(Default)]
struct ButtonEdges { previous: u16, blocked: u16 }

pub struct State {
    pub game_id: String,
    pub session: String,
    pub phase: String,
    pub connected: bool,
    host_sdk: Option<u64>,
    party: Value,
    frames: BTreeMap<String, Value>,
    frame_at: Option<Instant>,
    edges: BTreeMap<usize, ButtonEdges>,
    activity_at: BTreeMap<String, Instant>,
}

impl State {
    pub fn new(game_id: String) -> Self {
        Self { game_id, session: String::new(), phase: "idle".into(), connected: false, host_sdk: None,
            party: json!({}), frames: BTreeMap::new(), frame_at: None, edges: BTreeMap::new(), activity_at: BTreeMap::new() }
    }

    pub fn is_running(&self) -> bool { self.connected && self.phase == "running" }

    /// Optional SDK6 messages are not understood by the installed SDK5 host.
    /// No advertisement means unavailable; it never implies support.
    fn has_sdk6_options(&self) -> bool { self.host_sdk.is_some_and(|v| v >= 6) }

    /// Safe diagnostic state: canonical routing and lifecycle, never the
    /// launch token or private profile payloads. Input age uses receipt time.
    pub fn snapshot(&self) -> Value {
        json!({"connected":self.connected, "game":self.game_id, "session":self.session, "phase":self.phase,
            "godot_sdk":self.host_sdk, "capabilities":{"actions":self.has_sdk6_options(),"room_log":self.has_sdk6_options()},
            "controller_age_ms":self.frame_at.map(|at| at.elapsed().as_millis() as u64),
            "controller_tokens":self.frames.keys().collect::<Vec<_>>(),
            "seats":self.seats().iter().map(|s| json!({"index":s.index,"player_id":s.player_id,"name":s.name,"color":s.color,"controller":s.controller,"ai":s.ai,"couch":s.couch})).collect::<Vec<_>>()})
    }

    /// Profile by stable player ID; artwork must never be logged as diagnostics.
    pub fn player(&self, id: &str) -> Option<&Value> {
        self.party["players"].as_array()?.iter().find(|p| p["id"].as_str() == Some(id))
    }

    pub fn seats(&self) -> Vec<Seat> {
        let mut seats: Vec<_> = self.party["seats"].as_array().into_iter().flatten().filter_map(|seat| {
            let kind = seat["occupant"]["kind"].as_str()?;
            if kind != "local" && kind != "ai" { return None; }
            let index = seat["index"].as_u64()? as usize;
            let id = text(&seat["occupant"], "player_id");
            let profile = self.party["players"].as_array().into_iter().flatten().find(|p| text(p, "id") == id && !id.is_empty());
            Some(Seat { index, player_id: id, name: profile.map(|p| text(p, "name")).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Player {}", index + 1)),
                color: profile.map(|p| text(p, "color")).unwrap_or_default(), controller: text(seat, "controller"), ai: kind == "ai", couch: profile.and_then(|p| p["couch"].as_i64()) })
        }).collect();
        seats.sort_by_key(|s| (s.couch.unwrap_or(1000), s.index));
        seats.truncate(4);
        seats
    }

    fn neutralize(&mut self) {
        self.frames.clear();
        self.frame_at = None;
        self.edges.clear();
    }

    fn hold_buttons(&mut self) {
        for seat in self.seats() { self.edges.insert(seat.index, ButtonEdges { previous: 0, blocked: BUTTON_MASK }); }
    }

    pub fn suppress_buttons(&mut self) { self.hold_buttons(); }

    /// All genuinely active host devices, including unassigned controllers.
    /// The caller additionally gates this on foreground, non-scripted play.
    fn activity_due(&mut self, now: Instant) -> Vec<String> {
        if !self.is_running() || !self.frame_at.is_some_and(|at| now.saturating_duration_since(at) < STALE) { return vec![]; }
        let mut due = vec![];
        for (token, frame) in &self.frames {
            let active = frame["buttons"].as_u64().unwrap_or(0) & BUTTON_MASK as u64 != 0
                || (0..6).any(|i| axis(frame, i).abs() > 0.25);
            if active && self.activity_at.get(token).is_none_or(|at| now.saturating_duration_since(*at) >= Duration::from_secs(1)) {
                self.activity_at.insert(token.clone(), now);
                due.push(token.clone());
            }
        }
        due
    }

    /// Called once per human seat per simulation tick (or menu input tick).
    /// Stale, disconnected or missing-controller input is neutral. `any_phase`
    /// permits the game's menu while paused, but never an inactive session.
    pub fn pad_for_seat(&mut self, index: usize, now: Instant, any_phase: bool) -> Pad {
        let available = self.connected && !self.session.is_empty()
            && (self.phase == "running" || (any_phase && self.phase == "paused"))
            && self.frame_at.is_some_and(|at| now.saturating_duration_since(at) < STALE);
        let controller = self.seats().iter().find(|s| s.index == index && !s.ai).map(|s| s.controller.clone());
        let frame = if available { controller.and_then(|c| self.frames.get(&c)) } else { None };
        let Some(frame) = frame else {
            self.edges.insert(index, ButtonEdges { previous: 0, blocked: BUTTON_MASK });
            return Pad::default();
        };
        let held = (frame["buttons"].as_u64().unwrap_or(0) as u16) & BUTTON_MASK & !button::BACK;
        let edge = self.edges.entry(index).or_default();
        edge.blocked &= held;
        let buttons = held & !edge.blocked;
        let pressed = buttons & !edge.previous;
        edge.previous = buttons;
        Pad { buttons, pressed, axes: std::array::from_fn(|i| axis(frame, i)) }
    }

    fn ready(&mut self, session: &str) -> bool {
        if self.session != session || self.phase != "preparing" { return false; }
        self.phase = "ready".into();
        true
    }

    fn receive(&mut self, msg: Value, now: Instant) -> Vec<Command> {
        let kind = text(&msg, "type");
        let session = text(&msg, "session");
        if matches!(kind.as_str(), "start" | "pause" | "resume" | "dispose" | "party_updated" | "players_updated")
            && (self.session.is_empty() || session != self.session) { return vec![]; }
        match kind.as_str() {
            "welcome" => {
                if msg["protocol_version"].as_u64() != Some(1) { return vec![Command::Error("Unsupported or missing Gamepad Party protocol_version".into())]; }
                self.connected = true;
                self.host_sdk = msg["godot_sdk"].as_u64();
                self.party = msg.get("party").cloned().unwrap_or_else(|| json!({}));
                vec![Command::Connected]
            }
            "party_state" => { self.party = msg.get("party").cloned().unwrap_or_else(|| json!({})); vec![] }
            "controller_frame" => {
                self.frames.clear();
                self.frame_at = Some(now);
                for frame in msg["controllers"].as_array().into_iter().flatten() {
                    let token = text(frame, "controller");
                    if !token.is_empty() { self.frames.entry(token).or_insert_with(|| frame.clone()); }
                }
                vec![]
            }
            "prepare" => {
                if text(&msg, "game") != self.game_id || session.is_empty() || session == self.session { return vec![]; }
                let mut commands = vec![];
                if !self.session.is_empty() { commands.push(Command::Dispose { session: self.session.clone() }); }
                self.session = session.clone();
                self.phase = "preparing".into();
                self.neutralize();
                self.activity_at.clear();
                self.party["seats"] = msg.get("seats").cloned().unwrap_or_else(|| json!([]));
                self.party["players"] = msg.get("players").cloned().unwrap_or_else(|| json!([]));
                commands.push(Command::Prepare { session, seats: self.seats() });
                commands
            }
            "start" if self.phase == "ready" => { self.phase = "running".into(); self.hold_buttons(); vec![Command::Start { session }] }
            "pause" if self.phase == "running" => { self.phase = "paused".into(); self.hold_buttons(); vec![Command::Pause { session }] }
            "resume" if self.phase == "paused" => { self.phase = "running".into(); self.hold_buttons(); vec![Command::Resume { session }] }
            "dispose" => { self.phase = "idle".into(); self.session.clear(); self.neutralize(); vec![Command::Dispose { session }] }
            "party_updated" => {
                for key in ["seats", "players", "presence"] { self.party[key] = msg.get(key).cloned().unwrap_or_else(|| json!([])); }
                self.neutralize();
                self.hold_buttons();
                vec![Command::Roster { seats: self.seats() }]
            }
            "players_updated" => {
                let mut players = self.party["players"].as_array().cloned().unwrap_or_default();
                for player in msg["players"].as_array().into_iter().flatten() {
                    if let Some(old) = players.iter_mut().find(|p| p["id"] == player["id"]) { *old = player.clone(); } else { players.push(player.clone()); }
                }
                self.party["players"] = json!(players);
                vec![Command::PlayersUpdated { seats: self.seats() }]
            }
            "setting_changed" => vec![Command::Setting { key: text(&msg, "key"), value: msg.get("value").cloned().unwrap_or(Value::Null) }],
            "player_settings" => vec![Command::PlayerSettings { player_id: text(&msg, "player_id"), values: msg.get("values").cloned().unwrap_or_else(|| json!({})) }],
            "action_triggered" if !self.session.is_empty() => vec![Command::Action { key: text(&msg, "key"), player_id: text(&msg, "player_id") }],
            "error" => vec![Command::Error(text(&msg, "message"))],
            _ => vec![],
        }
    }
}

fn text(v: &Value, key: &str) -> String { v[key].as_str().unwrap_or_default().to_owned() }
fn axis(frame: &Value, i: usize) -> f64 { frame["axes"].get(i).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| (v / 32767.0).clamp(-1.0, 1.0)).unwrap_or(0.0) }
fn radial(v: [f64; 2], deadzone: f64) -> [f64; 2] {
    let length = v[0].hypot(v[1]);
    if length <= deadzone { return [0.0, 0.0]; }
    let scale = ((length - deadzone) / (1.0 - deadzone)).min(1.0) / length;
    [v[0] * scale, v[1] * scale]
}

enum Incoming { Json(Value, Instant), Disconnected(String) }

pub struct Client {
    pub state: State,
    pub launched_by_daemon: bool,
    /// Per seat: whether the last rumble sent was a stop.
    rumble_quiet: BTreeMap<usize, bool>,
    outgoing: Sender<Value>,
    incoming: Receiver<Incoming>,
    /// Read ahead by [`Client::loading`], still to be handled by `poll`.
    held: std::collections::VecDeque<Incoming>,
    /// A `prepare` seen ahead by [`Client::loading`]: whose progress to report.
    loading_session: Option<String>,
}

impl Client {
    pub fn connect_from_env(game_id: &str) -> Result<Option<Self>, String> { Config::from_env(game_id)?.map(Self::connect).transpose() }

    pub fn connect(config: Config) -> Result<Self, String> {
        if !config.url.starts_with("ws://") { return Err("Gamepad Party requires a ws:// host address".into()); }
        let (outgoing, send_rx) = mpsc::channel();
        let (recv_tx, incoming) = mpsc::channel();
        let state = State::new(config.game_id.clone());
        let launched_by_daemon = config.launched_by_daemon;
        std::thread::Builder::new().name("gamenight".into()).spawn(move || {
            if let Err(error) = transport(config, send_rx, &recv_tx) { let _ = recv_tx.send(Incoming::Disconnected(error)); }
        }).map_err(|e| format!("Cannot start Gamepad Party connection thread: {e}"))?;
        Ok(Self { state, launched_by_daemon, rumble_quiet: BTreeMap::new(), outgoing, incoming, held: Default::default(), loading_session: None })
    }

    pub fn poll(&mut self) -> Vec<Command> {
        let mut commands = vec![];
        while let Some(item) = self.held.pop_front().or_else(|| self.incoming.try_recv().ok()) {
            match item {
                Incoming::Json(value, received_at) => commands.extend(self.state.receive(value, received_at)),
                Incoming::Disconnected(error) => {
                    self.state.connected = false;
                    self.state.phase = "idle".into();
                    self.state.neutralize();
                    commands.push(Command::Disconnected(error));
                }
            }
            // Let the caller complete this transaction (especially building a
            // Prepare and replying Ready) before the next wire message changes
            // the authoritative session. Otherwise Prepare(s1), Prepare(s2)
            // drained together made Ready(s1) fail against state already at s2.
            // Input-only messages still coalesce, using their receipt times.
            if !commands.is_empty() { break; }
        }
        commands
    }

    /// Startup progress for the host's loading screen, before the game
    /// handles its `prepare` (the window, the GPU, the scene). Reads ahead
    /// for that `prepare` without handling it; `poll` still does.
    pub fn loading(&mut self, percent: u32, text: &str) {
        while let Ok(item) = self.incoming.try_recv() {
            if let Incoming::Json(msg, _) = &item
                && msg["type"] == "prepare" && msg["game"] == self.state.game_id.as_str()
                && let Some(session) = msg["session"].as_str().filter(|s| !s.is_empty()) {
                self.loading_session = Some(session.to_string());
            }
            self.held.push_back(item);
        }
        let session = if self.state.phase == "preparing" { Some(self.state.session.clone()) } else { self.loading_session.clone() };
        if let Some(session) = session { let _ = self.notify_progress(&session, percent, text); }
    }

    pub fn send_json(&self, value: Value) -> Result<(), String> { self.outgoing.send(value).map_err(|_| "Gamepad Party is disconnected".into()) }
    pub fn declare_settings(&self, settings: Value) -> Result<(), String> { self.send_json(json!({"type":"declare_settings", "settings":settings})) }
    pub fn declare_actions(&self, actions: Value) -> Result<(), String> {
        if !self.state.has_sdk6_options() {
            eprintln!("Gamepad Party: host SDK {:?} does not advertise SDK6; optional menu actions are unavailable", self.state.host_sdk);
            return Ok(());
        }
        self.send_json(json!({"type":"declare_actions", "actions":actions}))
    }
    /// Plain-text requests from players and the session assistant. With
    /// `mcp` (a live-control token file) the host's assistant changes the
    /// game through its MCP tools instead of sending text.
    pub fn declare_commands(&self, description: &str, examples: &[&str], mcp: Option<&std::path::Path>) -> Result<(), String> {
        let mut msg = json!({"type":"declare_commands", "description":description, "examples":examples});
        if let Some(path) = mcp { msg["mcp"] = json!(path.to_string_lossy()); }
        self.send_json(msg)
    }
    pub fn notify_participation(&self, session: &str, instant_join: bool) -> Result<(), String> { self.send_json(json!({"type":"participation", "session":session, "instant_join":instant_join})) }
    pub fn notify_ready(&mut self, session: &str) -> Result<(), String> {
        if !self.state.ready(session) { return Err("Ready does not match a preparing Gamepad Party session".into()); }
        self.send_json(json!({"type":"ready", "session":session}))
    }
    /// Loading progress shown in the lobby while a session prepares.
    pub fn notify_progress(&self, session: &str, percent: u32, text: &str) -> Result<(), String> {
        self.send_json(json!({"type":"progress", "session":session, "percent":percent.min(100), "text":text}))
    }
    pub fn notify_finished(&self, session: &str) -> Result<(), String> { self.send_json(json!({"type":"finished", "session":session})) }
    pub fn request_overlay(&self) -> Result<(), String> { self.send_json(json!({"type":"request_overlay"})) }
    pub fn request_start(&self) -> Result<(), String> { self.send_json(json!({"type":"request_start"})) }
    /// Only call from real foreground play, never from a hidden/scripted test.
    pub fn report_activity(&mut self) -> Result<(), String> {
        for controller in self.state.activity_due(Instant::now()) {
            self.send_json(json!({"type":"controller_input", "session":self.state.session, "controller":controller}))?;
        }
        Ok(())
    }
    /// Vibrate the controller on seat `seat`: `weak` is the light buzzy motor,
    /// `strong` the heavy one, both 0..1, for `seconds` (at most 5). Each call
    /// replaces the last for that seat; call it a few times a second while the
    /// effect lasts and with zeros to stop. Only while the session runs in
    /// front (the host ignores it otherwise); repeated stops are sent once.
    pub fn rumble(&mut self, seat: usize, weak: f64, strong: f64, seconds: f64) -> Result<(), String> {
        if !self.state.is_running() || seat > 255 { return Ok(()); }
        let (weak, strong) = (weak.clamp(0.0, 1.0), strong.clamp(0.0, 1.0));
        let quiet = weak == 0.0 && strong == 0.0;
        if quiet && *self.rumble_quiet.get(&seat).unwrap_or(&true) { return Ok(()); }
        self.rumble_quiet.insert(seat, quiet);
        self.send_json(json!({"type":"rumble", "session":self.state.session, "seat":seat,
            "weak":(weak * 65535.0) as u32, "strong":(strong * 65535.0) as u32, "ms":((seconds * 1000.0) as i64).clamp(0, 5000)}))
    }
    pub fn room_log(&self, text: &str) -> Result<(), String> {
        if !self.state.is_running() || !self.state.has_sdk6_options() { return Ok(()); }
        self.send_json(json!({"type":"room_log", "session":self.state.session, "text":text.chars().take(120).collect::<String>()}))
    }
}

fn transport(config: Config, outgoing: Receiver<Value>, incoming: &Sender<Incoming>) -> Result<(), String> {
    let mut hello = json!({"type":"hello", "role":"game", "game":config.game_id});
    if !config.token.is_empty() { hello["token"] = json!(config.token); }
    transport_with(&config.url, hello, outgoing, incoming)
}

/// One websocket session: send `hello`, then pump both directions until the
/// caller drops its sender (Ok) or the connection fails (Err, redacted).
fn transport_with(url: &str, hello: Value, outgoing: Receiver<Value>, incoming: &Sender<Incoming>) -> Result<(), String> {
    let mut diagnostics = TransportDiagnostics::new();
    let mut ws_config = tungstenite::protocol::WebSocketConfig::default();
    ws_config.max_message_size = Some(16 * 1024 * 1024); // SDK6: four avatar/profile payloads may exceed 1 MiB.
    let (mut socket, _) = tungstenite::client::connect_with_config(url, Some(ws_config), 3)
        .map_err(|error| diagnostics.failure("connect", websocket_error(&error)))?;
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => configure_stream(stream)?,
        _ => return Err("Unexpected Gamepad Party transport".into()),
    }
    match socket.send(Message::Text(hello.to_string().into())) {
        Err(error) if !would_block(&error) => return Err(diagnostics.failure("hello", websocket_error(&error))),
        _ => {}
    }
    diagnostics.sent_messages += 1;
    loop {
        let mut busy = false;
        loop {
            match outgoing.try_recv() {
                Ok(value) => {
                    // Non-blocking: a full socket buffer keeps the frame queued for the next flush.
                    match socket.send(Message::Text(value.to_string().into())) {
                        Err(error) if !would_block(&error) => return Err(diagnostics.failure("send", websocket_error(&error))),
                        _ => {}
                    }
                    diagnostics.sent_messages += 1;
                    busy = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => { let _ = socket.close(None); let _ = socket.flush(); return Ok(()); }
            }
        }
        match socket.flush() {
            Err(error) if !would_block(&error) => return Err(diagnostics.failure("flush", websocket_error(&error))),
            _ => {}
        }
        match socket.read() {
            Ok(Message::Text(raw)) => {
                diagnostics.received_frames += 1;
                diagnostics.last_received = Some(Instant::now());
                let value: Value = serde_json::from_str(raw.as_str()).map_err(|error| diagnostics.failure("decode",
                    format!("json={:?}, line={}, column={}", error.classify(), error.line(), error.column())))?;
                if !value.is_object() { continue; }
                if incoming.send(Incoming::Json(value, Instant::now())).is_err() { return Ok(()); }
                busy = true;
            }
            Ok(Message::Close(frame)) => return Err(diagnostics.failure("receive close",
                format!("code={}", frame.map(|frame| u16::from(frame.code).to_string()).unwrap_or_else(|| "none".into())))),
            Ok(_) => {
                diagnostics.received_frames += 1;
                diagnostics.last_received = Some(Instant::now());
                let _ = socket.flush();
            }
            // Nothing to read: wait a moment rather than spin.
            Err(error) if would_block(&error) => if !busy { std::thread::sleep(IDLE_WAIT); },
            Err(error) => return Err(diagnostics.failure("read", websocket_error(&error))),
        }
    }
}

// Report only when this transport exits, never once per idle read timeout.
// Error Display/Debug can contain URLs, headers or buffered messages: retain
// structural causes and OS codes without printing any peer-controlled text.
struct TransportDiagnostics {
    started: Instant,
    last_received: Option<Instant>,
    received_frames: u64,
    sent_messages: u64,
}

impl TransportDiagnostics {
    fn new() -> Self {
        Self { started: Instant::now(), last_received: None, received_frames: 0, sent_messages: 0 }
    }

    fn failure(&self, operation: &str, cause: String) -> String {
        let last_received_ms = self.last_received.map(|at| at.elapsed().as_millis().to_string()).unwrap_or_else(|| "none".into());
        format!("Gamepad Party transport {operation} failed: {cause}; elapsed_ms={}, received_frames={}, sent_messages={}, last_received_ms={last_received_ms}",
            self.started.elapsed().as_millis(), self.received_frames, self.sent_messages)
    }
}

/// How long the transport sleeps when nothing came in or went out.
const IDLE_WAIT: Duration = Duration::from_millis(2);

/// Nothing to do yet on the non-blocking socket. Windows error 997
/// (ERROR_IO_PENDING) is the same thing: it ended a 40-minute Spellstorm run
/// when the socket still used a read timeout, so it is never fatal.
fn would_block(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(error) if io_would_block(error))
}

fn io_would_block(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted)
        || (cfg!(windows) && error.raw_os_error() == Some(997))
}

fn io_error(error: &std::io::Error) -> String {
    format!("io={:?}, os_code={}", error.kind(), error.raw_os_error().map(|code| code.to_string()).unwrap_or_else(|| "none".into()))
}

fn websocket_error(error: &tungstenite::Error) -> String {
    use tungstenite::{Error, error::ProtocolError};
    match error {
        Error::Io(error) => io_error(error),
        Error::ConnectionClosed => "connection_closed".into(),
        Error::AlreadyClosed => "already_closed".into(),
        Error::Capacity(error) => format!("capacity={error:?}"), // Only limits/counts.
        Error::Protocol(ProtocolError::InvalidHeader(_)) => "protocol=InvalidHeader".into(),
        Error::Protocol(error) => format!("protocol={error:?}"), // Remaining 0.28 variants contain only enums/numbers.
        Error::Http(response) => format!("http_status={}", response.status().as_u16()),
        Error::HttpFormat(_) => "http_format".into(),
        Error::WriteBufferFull(_) => "write_buffer_full".into(),
        Error::Utf8(_) => "invalid_utf8".into(),
        Error::Url(_) => "invalid_url_or_connect_target".into(),
        Error::Tls(_) => "tls".into(),
        Error::AttackAttempt => "attack_attempt".into(),
    }
}

/// Non-blocking, not a read timeout: on Windows a timed-out blocking read
/// leaves the socket in an undefined state (and surfaced as error 997).
fn configure_stream(stream: &TcpStream) -> Result<(), String> {
    stream.set_nonblocking(true).map_err(|e| format!("Gamepad Party set_nonblocking failed: {}", io_error(&e)))?;
    stream.set_nodelay(true).map_err(|e| format!("Gamepad Party set_nodelay failed: {}", io_error(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_diagnostics_redact_peer_text_and_keep_structural_causes() {
        use tungstenite::{Error, error::{ProtocolError, UrlError}};
        let secret = "private-token-and-payload";
        let errors = [
            Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionReset, secret)),
            Error::Utf8(secret.into()),
            Error::Url(UrlError::UnableToConnect(format!("ws://host/?token={secret}"))),
            Error::WriteBufferFull(Box::new(Message::Text(secret.into()))),
            Error::Protocol(ProtocolError::InvalidHeader(Box::new(tungstenite::http::HeaderName::from_static("private-token-and-payload")))),
            Error::Http(Box::new(tungstenite::http::Response::builder().status(403).header("x-private", secret).body(Some(secret.as_bytes().to_vec())).unwrap())),
        ];
        let diagnostics = TransportDiagnostics::new();
        for error in errors {
            let report = diagnostics.failure("read", websocket_error(&error));
            assert!(!report.contains(secret), "peer data leaked: {report}");
            assert!(report.contains("received_frames=0") && report.contains("last_received_ms=none"));
            assert!(report.len() < 512);
        }
        assert!(websocket_error(&Error::Io(std::io::Error::from_raw_os_error(10054))).contains("os_code=10054"));
        assert_eq!(websocket_error(&Error::Protocol(ProtocolError::ResetWithoutClosingHandshake)), "protocol=ResetWithoutClosingHandshake");
        for kind in [std::io::ErrorKind::WouldBlock, std::io::ErrorKind::TimedOut] {
            assert!(would_block(&Error::Io(std::io::Error::from(kind))));
        }
        // Windows ERROR_IO_PENDING ended a Spellstorm run: never fatal there.
        assert_eq!(would_block(&Error::Io(std::io::Error::from_raw_os_error(997))), cfg!(windows));
        assert!(!would_block(&Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset))));
        assert!(!would_block(&Error::ConnectionClosed));
    }

    /// The non-blocking socket loses nothing under a burst both ways, a large
    /// frame included, and ends cleanly when the game drops its sender.
    #[test]
    fn non_blocking_transport_keeps_every_frame_of_a_burst() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut config = tungstenite::protocol::WebSocketConfig::default();
            config.max_message_size = Some(16 * 1024 * 1024);
            let mut socket = tungstenite::accept_with_config(stream, Some(config)).unwrap();
            let _hello = socket.read().unwrap();
            for i in 0..3000 { socket.send(Message::Text(json!({"type":"n","i":i}).to_string().into())).unwrap(); }
            socket.send(Message::Text(json!({"type":"big","pad":"x".repeat(1 << 20)}).to_string().into())).unwrap();
            let mut got = 0;
            while got < 3000 { if let Message::Text(_) = socket.read().unwrap() { got += 1; } }
            // Then the client's close.
            while let Ok(m) = socket.read() { if m.is_close() { break; } }
            got
        });
        let (out_tx, out_rx) = mpsc::channel();
        let (in_tx, in_rx) = mpsc::channel();
        let url = format!("ws://{address}");
        let client = std::thread::spawn(move || transport_with(&url, json!({"type":"hello"}), out_rx, &in_tx));
        for i in 0..3000 { out_tx.send(json!({"type":"input","i":i})).unwrap(); }
        let mut seen = 0;
        let mut big = false;
        while !big {
            match in_rx.recv_timeout(Duration::from_secs(10)).expect("frames keep coming") {
                Incoming::Json(v, _) if v["type"] == "n" => { assert_eq!(v["i"], seen); seen += 1; }
                Incoming::Json(v, _) if v["type"] == "big" => big = true,
                other => panic!("unexpected {:?}", matches!(other, Incoming::Disconnected(_))),
            }
        }
        assert_eq!(seen, 3000);
        drop(out_tx);
        assert!(client.join().unwrap().is_ok(), "dropping the sender ends the transport cleanly");
        assert_eq!(server.join().unwrap(), 3000);
    }

    #[test]
    fn transport_close_report_keeps_code_without_reason_token_or_query() {
        use std::net::TcpListener;
        use tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        let secret = "private-token-and-payload";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let _hello = socket.read().unwrap();
            socket.close(Some(CloseFrame { code: CloseCode::Policy, reason: secret.into() })).unwrap();
        });
        let mut client = Client::connect(Config { url: format!("ws://{address}/?private={secret}"), game_id:"sideways".into(), token:secret.into(), launched_by_daemon:true }).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut report = None;
        while Instant::now() < deadline && report.is_none() {
            for event in client.poll() { if let Command::Disconnected(error) = event { report = Some(error); } }
            if report.is_none() { std::thread::sleep(Duration::from_millis(2)); }
        }
        server.join().unwrap();
        let report = report.expect("host close must report a disconnect");
        assert!(report.contains("receive close") && report.contains("code=1008"), "{report}");
        assert!(report.contains("sent_messages=1"), "{report}");
        assert!(!report.contains(secret), "{report}");
    }

    fn prepare() -> Value { json!({"type":"prepare", "game":"sideways", "session":"s1",
        "seats":[{"index":3,"controller":"pad-b","occupant":{"kind":"local","player_id":"p2"}}, {"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}}],
        "players":[{"id":"p1","name":"Joep","couch":0},{"id":"p2","name":"Two","couch":1}]}) }
    fn running() -> State {
        let mut state = State::new("sideways".into());
        let now = Instant::now();
        state.receive(json!({"type":"welcome","protocol_version":1,"godot_sdk":6,"party":{}}), now);
        state.receive(prepare(), now);
        assert!(state.ready("s1"));
        state.receive(json!({"type":"start","session":"s1"}), now);
        state
    }

    #[test]
    fn lifecycle_requires_matching_session_and_ready() {
        let mut state = State::new("sideways".into());
        let now = Instant::now();
        state.receive(prepare(), now);
        assert!(state.receive(json!({"type":"start","session":"s1"}), now).is_empty());
        assert!(!state.ready("other")); assert!(state.ready("s1"));
        assert!(state.receive(json!({"type":"start","session":"other"}), now).is_empty());
        state.receive(json!({"type":"start","session":"s1"}), now);
        assert_eq!(state.phase, "running");
        state.receive(json!({"type":"pause","session":"s1"}), now); assert_eq!(state.phase, "paused");
        state.receive(json!({"type":"resume","session":"s1"}), now); assert_eq!(state.phase, "running");
        state.receive(json!({"type":"dispose","session":"s1"}), now);
        assert!(state.session.is_empty()); assert_eq!(state.phase, "idle");
    }

    #[test]
    fn welcome_validates_protocol_and_negotiates_optional_sdk6_messages() {
        let now = Instant::now();
        for version in [Value::Null, json!(0), json!(2), json!("1")] {
            let mut state = State::new("sideways".into());
            let events = state.receive(json!({"type":"welcome","protocol_version":version,"godot_sdk":6,"party":{}}), now);
            assert!(matches!(events.as_slice(), [Command::Error(_)]));
            assert!(!state.connected);
        }
        for (sdk, supported) in [(Value::Null,false),(json!(5),false),(json!(6),true)] {
            let mut state = State::new("sideways".into());
            let events = state.receive(json!({"type":"welcome","protocol_version":1,"godot_sdk":sdk,"party":{}}), now);
            assert!(matches!(events.as_slice(), [Command::Connected]));
            assert_eq!(state.has_sdk6_options(), supported);
            assert_eq!(state.snapshot()["capabilities"]["actions"], supported);
        }
    }

    #[test]
    fn canonical_controller_tokens_staleness_and_button_edges() {
        let mut state = running(); let now = Instant::now();
        assert_eq!(state.seats().iter().map(|s| s.index).collect::<Vec<_>>(), [0,3]);
        state.receive(json!({"type":"controller_frame","controllers":[
            {"controller":"pad-b","buttons":0,"axes":[-32768,0,0,0,0,0]},
            {"controller":"pad-a","buttons":128,"axes":[32767,0,0,0,0,32767]}]}), now);
        let first = state.pad_for_seat(0, now, false);
        assert_eq!(first.trigger(1), 1.0); assert_eq!(first.stick(0)[0], 1.0);
        assert!(!first.just(button::START), "held lobby button must be released first");
        assert_eq!(state.pad_for_seat(3, now, false).stick(0)[0], -1.0);
        assert_eq!(state.pad_for_seat(2, now, false), Pad::default());
        assert_eq!(state.pad_for_seat(0, now + STALE, false), Pad::default());
        state.receive(json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":0,"axes":[]}]}), now);
        state.pad_for_seat(0, now, false);
        state.receive(json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":196,"axes":[]}]}), now);
        let press = state.pad_for_seat(0, now, false);
        assert!(press.just(button::START) && press.just(button::X));
        assert_eq!(press.pressed & button::BACK, 0, "Back must never reach gameplay");
        assert!(!state.pad_for_seat(0, now, false).just(button::START));
        state.receive(json!({"type":"controller_frame","controllers":[]}), now);
        assert_eq!(state.pad_for_seat(0, now, false), Pad::default());
    }

    #[test]
    fn profiles_do_not_restart_and_rosters_clear_old_input() {
        let mut state = running(); let now = Instant::now();
        let commands = state.receive(json!({"type":"players_updated","session":"s1","players":[{"id":"p1","name":"New name","couch":2,"avatar":"test-art","skin_color":"#aabbcc"}]}), now);
        assert!(matches!(commands[0], Command::PlayersUpdated { .. }));
        assert_eq!(state.phase, "running"); assert_eq!(state.seats()[1].name, "New name");
        assert_eq!(state.player("p1").unwrap()["avatar"], "test-art");
        assert_eq!(state.player("p1").unwrap()["skin_color"], "#aabbcc");
        assert!(state.player("absent").is_none());
        state.receive(json!({"type":"party_updated","session":"s1","seats":[],"players":[]}), now);
        assert!(state.seats().is_empty());
        assert_eq!(state.pad_for_seat(0, now, false), Pad::default());
    }

    #[test]
    fn activity_reports_real_devices_with_deadzone_staleness_and_one_second_limit() {
        let mut state = running(); let now = Instant::now();
        let frames = json!({"type":"controller_frame", "controllers":[
            {"controller":"unassigned", "axes":[0,0,0,0,0,32767], "buttons":0},
            {"controller":"pad-a", "axes":[6000,0,0,0,0,0], "buttons":0},
            {"controller":"pad-b", "axes":[], "buttons":1}]});
        state.receive(frames.clone(), now);
        assert_eq!(state.activity_due(now), ["pad-b", "unassigned"]);
        assert!(state.activity_due(now + Duration::from_millis(100)).is_empty());
        assert!(state.activity_due(now + Duration::from_secs(1)).is_empty(), "stale stream is not human activity");
        let later = now + Duration::from_secs(1);
        state.receive(frames.clone(), later);
        assert_eq!(state.activity_due(later), ["pad-b", "unassigned"]);
        state.receive(json!({"type":"pause", "session":"s1"}), later);
        state.receive(frames, later + Duration::from_secs(1));
        assert!(state.activity_due(later + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn rumble_runs_only_in_a_running_session_and_sends_one_stop() {
        let (outgoing, sent) = mpsc::channel();
        let (_receive, incoming) = mpsc::channel();
        let mut client = Client { state: State::new("sideways".into()), launched_by_daemon:true, rumble_quiet: BTreeMap::new(), outgoing, incoming, held: Default::default(), loading_session: None };
        client.rumble(0, 1.0, 0.5, 0.2).unwrap();
        assert!(sent.try_recv().is_err(), "not running: nothing sent");
        client.state = running();
        client.rumble(1, 1.0, 0.5, 9.0).unwrap();
        assert_eq!(sent.try_recv().unwrap(), json!({"type":"rumble","session":"s1","seat":1,"weak":65535,"strong":32767,"ms":5000}));
        client.rumble(1, 0.0, 0.0, 0.0).unwrap();
        client.rumble(1, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(sent.try_recv().unwrap()["weak"], 0);
        assert!(sent.try_recv().is_err(), "one stop only");
        client.rumble(2, 0.0, 0.0, 0.0).unwrap();
        assert!(sent.try_recv().is_err(), "a seat that never rumbled needs no stop");
    }

    #[test]
    fn startup_progress_reports_against_a_prepare_read_ahead() {
        let (outgoing, sent) = mpsc::channel();
        let (receive, incoming) = mpsc::channel();
        let mut client = Client { state: State::new("sideways".into()), launched_by_daemon:true, rumble_quiet: BTreeMap::new(), outgoing, incoming, held: Default::default(), loading_session: None };
        client.loading(20, "opening the window");
        assert!(sent.try_recv().is_err(), "no session yet: nothing to report against");
        receive.send(Incoming::Json(json!({"type":"welcome","protocol_version":1}), Instant::now())).unwrap();
        receive.send(Incoming::Json(prepare(), Instant::now())).unwrap();
        client.loading(35, "starting the graphics card");
        assert_eq!(sent.try_recv().unwrap(), json!({"type":"progress","session":"s1","percent":35,"text":"starting the graphics card"}));
        assert_eq!(client.state.phase, "idle", "read ahead, not handled");
        let mut prepared = false;
        for _ in 0..4 { prepared |= client.poll().iter().any(|c| matches!(c, Command::Prepare { .. })); }
        assert!(prepared, "poll still handles the prepare");
    }

    #[test]
    fn live_control_is_offered_with_its_token_file() {
        let (outgoing, sent) = mpsc::channel();
        let (_receive, incoming) = mpsc::channel();
        let client = Client { state: State::new("neon-siege".into()), launched_by_daemon:true, rumble_quiet: BTreeMap::new(), outgoing, incoming, held: Default::default(), loading_session: None };
        client.declare_commands("Change it live", &[], Some(std::path::Path::new("/tmp/warp-live/neon-siege.json"))).unwrap();
        assert_eq!(sent.try_recv().unwrap(), json!({"type":"declare_commands","description":"Change it live","examples":[],"mcp":"/tmp/warp-live/neon-siege.json"}));
        client.declare_commands("Items", &["give me 1 bread"], None).unwrap();
        assert!(sent.try_recv().unwrap().get("mcp").is_none());
    }

    #[test]
    fn queued_prepares_and_dispose_do_not_overtake_ready_handling() {
        let (outgoing, sent) = mpsc::channel();
        let (receive, incoming) = mpsc::channel();
        let mut client = Client { state: State::new("sideways".into()), launched_by_daemon:true, rumble_quiet: BTreeMap::new(), outgoing, incoming, held: Default::default(), loading_session: None };
        let now=Instant::now();
        let mut second=prepare(); second["session"]=json!("s2");
        for msg in [json!({"type":"welcome","protocol_version":1,"party":{}}),prepare(),second,json!({"type":"dispose","session":"s2"})] {
            receive.send(Incoming::Json(msg,now)).unwrap();
        }
        assert!(matches!(client.poll().as_slice(),[Command::Connected]));
        assert!(matches!(client.poll().as_slice(),[Command::Prepare{session,..}] if session=="s1"));
        client.notify_ready("s1").expect("queued second Prepare must not invalidate current Ready");
        assert_eq!(sent.try_recv().unwrap()["session"],"s1");
        assert!(matches!(client.poll().as_slice(),[Command::Dispose{session:old},Command::Prepare{session:new,..}] if old=="s1" && new=="s2"));
        client.notify_ready("s2").expect("queued Dispose must not overtake preparation");
        assert_eq!(sent.try_recv().unwrap()["session"],"s2");
        assert!(matches!(client.poll().as_slice(),[Command::Dispose{session}] if session=="s2"));
        assert!(client.state.session.is_empty());
        assert!(client.poll().is_empty());
    }

    #[test]
    fn loopback_websocket_handshake_ready_and_controller_frame() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let hello: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(hello, json!({"type":"hello","role":"game","game":"sideways","token":"test-token"}));
            for msg in [json!({"type":"welcome","protocol_version":1,"party":{}}), prepare()] { socket.send(Message::Text(msg.to_string().into())).unwrap(); }
            let ready: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(ready, json!({"type":"ready","session":"s1"}));
            socket.send(Message::Text(json!({"type":"start","session":"s1"}).to_string().into())).unwrap();
            socket.send(Message::Text(json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":0,"axes":[0,0,0,0,0,32767]}]}).to_string().into())).unwrap();
            let done: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(done["type"], "finished");
        });
        let mut client = Client::connect(Config { url: format!("ws://{address}"), game_id:"sideways".into(), token:"test-token".into(), launched_by_daemon:true }).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut throttle = false;
        while Instant::now() < deadline {
            for event in client.poll() { if let Command::Prepare { session, .. } = event { client.notify_ready(&session).unwrap(); } }
            if client.state.is_running() && client.state.pad_for_seat(0, Instant::now(), false).trigger(1) == 1.0 { throttle = true; break; }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(throttle, "host controller frame did not reach canonical seat");
        client.notify_finished("s1").unwrap();
        server.join().unwrap();
    }
}
