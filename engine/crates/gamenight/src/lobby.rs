//! The lobby role: a Warp program that is the room between games (like the
//! Godot Game Room). Matches `sdk/godot/addons/gamenight/lobby.gd`.
//!
//! The host owns players, sessions and input routing. A lobby receives the
//! whole party snapshot, every controller frame, and screen ownership
//! (`lobby_focus`); it sends plain commands back. The transport reconnects on
//! its own (1.5 s), and a reconnect never carries the old focus grant over.
//! Phone links and room pickups go through the host's local web port.

use crate::{transport_with, Incoming};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const RETRY: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug)]
pub struct Config {
    pub url: String,
    pub lobby_id: String,
    pub token: String,
    /// `http://127.0.0.1:<port>` of the host's web server, or empty when
    /// links and pickups are unavailable (anything but loopback is refused).
    pub links_base: String,
}

impl Config {
    /// From GAMENIGHT_ADDR (default 127.0.0.1:7912), GAMENIGHT_GAME_ID (else
    /// `lobby_id`), GAMENIGHT_TOKEN and GAMENIGHT_LINKS_URL (default
    /// http://127.0.0.1:7913). A lobby always connects: the host starts it.
    pub fn from_env(lobby_id: &str) -> Self {
        let addr = std::env::var("GAMENIGHT_ADDR").ok().filter(|a| !a.is_empty()).unwrap_or_else(|| "127.0.0.1:7912".into());
        let url = if addr.contains("://") { addr } else { format!("ws://{addr}") };
        let links = std::env::var("GAMENIGHT_LINKS_URL").ok().filter(|l| !l.is_empty()).unwrap_or_else(|| "http://127.0.0.1:7913".into());
        Self { url, lobby_id: std::env::var("GAMENIGHT_GAME_ID").ok().filter(|g| !g.is_empty()).unwrap_or_else(|| lobby_id.into()),
            token: std::env::var("GAMENIGHT_TOKEN").unwrap_or_default(), links_base: loopback_only(links) }
    }
}

fn loopback_only(base: String) -> String { if base.starts_with("http://127.0.0.1:") { base.trim_end_matches('/').into() } else { String::new() } }

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Welcome: the first party snapshot. `lobby_ready` is sent on the next
    /// `poll`, after the caller had a chance to build its first frame.
    Connected,
    Party,
    /// Screen ownership: draw and take input only while active.
    Focus(bool),
    /// Raw controller frames (all controllers, also unassigned ones).
    Controllers(Vec<Value>),
    /// The host refused a command; show the message.
    Rejected(String),
    Disconnected(String),
}

pub struct Lobby {
    pub config: Config,
    pub party: Value,
    pub active: bool,
    pub connected: bool,
    outgoing: Option<Sender<Value>>,
    incoming: Option<Receiver<Incoming>>,
    retry_at: Option<Instant>,
    ready_due: bool,
}

impl Lobby {
    pub fn connect_from_env(lobby_id: &str) -> Self { Self::connect(Config::from_env(lobby_id)) }

    pub fn connect(config: Config) -> Self {
        let mut lobby = Self { config, party: json!({}), active: false, connected: false, outgoing: None, incoming: None, retry_at: None, ready_due: false };
        lobby.open();
        lobby
    }

    fn open(&mut self) {
        let (outgoing, send_rx) = mpsc::channel();
        let (recv_tx, incoming) = mpsc::channel();
        let url = self.config.url.clone();
        let mut hello = json!({"type":"hello", "role":"lobby", "game":self.config.lobby_id});
        if !self.config.token.is_empty() { hello["token"] = json!(self.config.token); }
        let spawned = std::thread::Builder::new().name("gamenight-lobby".into()).spawn(move || {
            if !url.starts_with("ws://") { let _ = recv_tx.send(Incoming::Disconnected("GameNight requires a ws:// host address".into())); return; }
            match transport_with(&url, hello, send_rx, &recv_tx) {
                Err(error) => { let _ = recv_tx.send(Incoming::Disconnected(error)); }
                Ok(()) => {}
            }
        });
        if spawned.is_ok() { self.outgoing = Some(outgoing); self.incoming = Some(incoming); self.retry_at = None; }
        else { self.retry_at = Some(Instant::now() + RETRY); }
    }

    /// Call once per frame. Reconnects after a failure.
    pub fn poll(&mut self) -> Vec<Event> {
        if self.ready_due { self.ready_due = false; self.send(json!({"type":"lobby_ready"})); }
        if self.incoming.is_none() {
            if self.retry_at.is_some_and(|at| Instant::now() >= at) { self.open(); }
            return vec![];
        }
        let mut events = vec![];
        while let Some(item) = self.incoming.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match item {
                Incoming::Json(message, _) => events.extend(self.receive(message)),
                Incoming::Disconnected(error) => {
                    self.outgoing = None;
                    self.incoming = None;
                    self.retry_at = Some(Instant::now() + RETRY);
                    self.ready_due = false;
                    if self.connected || self.active { events.push(Event::Disconnected(error)); }
                    self.connected = false;
                    self.active = false;
                    break;
                }
            }
        }
        events
    }

    fn receive(&mut self, message: Value) -> Vec<Event> {
        match message["type"].as_str().unwrap_or_default() {
            "welcome" => {
                self.connected = true;
                self.party = message.get("party").cloned().unwrap_or_else(|| json!({}));
                self.ready_due = true;
                vec![Event::Connected, Event::Party]
            }
            "party_state" => { self.party = message.get("party").cloned().unwrap_or_else(|| json!({})); vec![Event::Party] }
            "lobby_focus" => { self.active = message["active"].as_bool().unwrap_or(false); vec![Event::Focus(self.active)] }
            "controller_frame" => vec![Event::Controllers(message["controllers"].as_array().cloned().unwrap_or_default())],
            "error" => vec![Event::Rejected(message["message"].as_str().unwrap_or("The request could not be completed.").into())],
            _ => vec![],
        }
    }

    fn send(&self, message: Value) -> bool { self.outgoing.as_ref().is_some_and(|tx| tx.send(message).is_ok()) }

    /// A host command. Err (with a message to show) while reconnecting.
    pub fn command(&self, mut message: Value) -> Result<(), String> {
        if !self.connected { return Err("Reconnecting to GameNight. Please try again when connected.".into()); }
        // The queue commands echo the host's integer cursor and length.
        if let Some(expected) = message.get_mut("expected").and_then(Value::as_object_mut) {
            for key in ["current", "queued"] {
                if let Some(n) = expected.get(key).and_then(Value::as_f64) { expected.insert(key.into(), json!(n as i64)); }
            }
        }
        if matches!(message["type"].as_str(), Some("next" | "play_next" | "close_overlay")) { allow_game_foreground(); }
        if self.send(message) { Ok(()) } else { Err("GameNight is disconnected".into()) }
    }

    pub fn join_guest(&self, name: &str) -> Result<(), String> { self.command(json!({"type":"join_party", "name":name})) }
    pub fn leave(&self, player_id: &str) -> Result<(), String> { self.command(json!({"type":"leave_party", "player_id":player_id})) }
    /// Sit `player_id` down at `position` on the couch, counted from the left.
    pub fn set_couch_position(&self, player_id: &str, position: u32) -> Result<(), String> {
        self.command(json!({"type":"set_couch_position", "player_id":player_id, "position":position}))
    }
    /// Start `game` now (starts an idle night).
    pub fn play(&self, game: &str) -> Result<(), String> { self.command(json!({"type":"play_next", "game":game})) }
    /// Put `game` on top of the queue without starting it.
    pub fn queue_next(&self, game: &str) -> Result<(), String> { self.command(json!({"type":"queue_next", "game":game})) }
    /// Add `game` to the players' queue (after earlier picks), or first.
    pub fn queue_game(&self, game: &str, first: bool) -> Result<(), String> { self.command(json!({"type":"queue_game", "game":game, "first":first})) }
    pub fn start_next(&self) -> Result<(), String> { self.command(json!({"type":"next"})) }
    /// Close only the pause that opening the lobby caused.
    pub fn resume_game(&self) -> Result<(), String> { self.command(json!({"type":"close_overlay"})) }
    pub fn open_lobby(&self) -> Result<(), String> { self.command(json!({"type":"open_overlay"})) }
    pub fn quit_party(&self) -> Result<(), String> { self.command(json!({"type":"quit_party"})) }
    pub fn set_setting(&self, game: &str, key: &str, value: Value) -> Result<(), String> {
        self.command(json!({"type":"set_setting", "game":game, "key":key, "value":value}))
    }
    /// `play_pause`, `previous_track`, `next_track` or `resume`.
    pub fn media_control(&self, action: &str) -> Result<(), String> {
        if !matches!(action, "play_pause" | "previous_track" | "next_track" | "resume") { return Err(format!("Unknown media action {action}")); }
        self.command(json!({"type":"media_control", "action":action}))
    }
    pub fn remove_from_queue(&self, index: usize) -> Result<(), String> {
        self.command(json!({"type":"remove_playlist_entry", "expected":self.party["playlist"], "index":index}))
    }
    pub fn move_in_queue(&self, from: usize, to: usize) -> Result<(), String> {
        self.command(json!({"type":"move_playlist_entry", "expected":self.party["playlist"], "from":from, "to":to}))
    }
    /// Choose another lobby (`party.lobbies`); the host restarts into it.
    pub fn switch_lobby(&self, lobby: &str) -> Result<(), String> { self.command(json!({"type":"switch_lobby", "lobby":lobby})) }
    pub fn apply_update(&self) -> Result<(), String> { self.command(json!({"type":"apply_update"})) }

    /// GET /api/player-links: the room QR (`room.qr_svg`), code and doors.
    pub fn player_links(&self) -> Result<Value, String> {
        let body = http(&self.config.links_base, "GET", "/api/player-links", &[])?;
        serde_json::from_slice(&body).map_err(|e| format!("player-links: {e}"))
    }
    /// Pick up the waiting phone profile `pending` for the room player `player_id`.
    /// `swap` replaces a profile the player already has ("Hold to swap").
    pub fn room_pickup(&self, pending: &str, player_id: &str, swap: bool) -> Result<(), String> {
        let mut headers = vec!["X-GameNight-Local-Pickup: 1"];
        if swap { headers.push("X-GameNight-Swap: 1"); }
        http(&self.config.links_base, "POST", &format!("/api/room-pickup/{}/{}", path_part(pending), path_part(player_id)), &headers).map(|_| ())
    }
    pub fn unlink(&self, player_id: &str) -> Result<(), String> {
        http(&self.config.links_base, "POST", &format!("/api/player-links/{}/unlink", path_part(player_id)), &["X-GameNight-Local-Pickup: 1"]).map(|_| ())
    }
}

fn path_part(s: &str) -> String { s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect() }

/// Windows keeps a background process from taking focus unless the foreground
/// app hands it over; the host ships a helper that does
/// (GAMENIGHT_FOREGROUND_HELPER --allow-foreground).
fn allow_game_foreground() {
    if !cfg!(windows) { return; }
    let Ok(helper) = std::env::var("GAMENIGHT_FOREGROUND_HELPER") else { return };
    if helper.is_empty() || !std::path::Path::new(&helper).is_file() { return; }
    let _ = std::process::Command::new(helper).arg("--allow-foreground").status();
}

/// A tiny HTTP/1.1 client for the host's loopback web port (3 s timeout).
/// Blocking: call from a worker thread if a frame must not wait.
pub fn http(base: &str, method: &str, path: &str, headers: &[&str]) -> Result<Vec<u8>, String> {
    let host = base.strip_prefix("http://").filter(|h| h.starts_with("127.0.0.1:")).ok_or("GameNight links are unavailable")?;
    let addr: std::net::SocketAddr = host.parse().map_err(|_| "Bad GameNight links address".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| format!("links connect: {:?}", e.kind()))?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: 0\r\n");
    for header in headers { request.push_str(header); request.push_str("\r\n"); }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).map_err(|e| format!("links send: {:?}", e.kind()))?;
    let mut response = vec![];
    stream.read_to_end(&mut response).map_err(|e| format!("links read: {:?}", e.kind()))?;
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").ok_or("links: no header end")?;
    let head = String::from_utf8_lossy(&response[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("links: no status")?;
    let mut body = response[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { body = dechunk(&body)?; }
    if status >= 300 { return Err(format!("links status {status}")); }
    Ok(body)
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    loop {
        let line = data.windows(2).position(|w| w == b"\r\n").ok_or("links: bad chunk")?;
        let size = usize::from_str_radix(String::from_utf8_lossy(&data[..line]).split(';').next().unwrap_or("").trim(), 16).map_err(|_| "links: bad chunk size")?;
        data = &data[line + 2..];
        if size == 0 { return Ok(out); }
        if data.len() < size + 2 { return Err("links: short chunk".into()); }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn wait<F: FnMut(&mut Lobby) -> bool>(lobby: &mut Lobby, mut done: F) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline { if done(lobby) { return; } std::thread::sleep(Duration::from_millis(5)); }
        panic!("timed out");
    }

    #[test]
    fn lobby_says_hello_readies_after_welcome_and_tracks_focus() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let hello: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            socket.send(tungstenite::Message::Text(json!({"type":"welcome","protocol_version":1,"party":{"playlist":{"current":1,"queued":2}}}).to_string().into())).unwrap();
            let ready: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            socket.send(tungstenite::Message::Text(json!({"type":"lobby_focus","active":true}).to_string().into())).unwrap();
            let command: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            socket.send(tungstenite::Message::Text(json!({"type":"error","message":"no"}).to_string().into())).unwrap();
            let _ = socket.read();
            (hello, ready, command)
        });
        let mut lobby = Lobby::connect(Config { url: format!("ws://{address}"), lobby_id: "warp-room".into(), token: "t".into(), links_base: String::new() });
        assert!(lobby.command(json!({"type":"next"})).is_err(), "commands wait for the welcome");
        let mut seen = vec![];
        wait(&mut lobby, |l| { seen.extend(l.poll()); l.active });
        assert_eq!(seen, vec![Event::Connected, Event::Party, Event::Focus(true)]);
        lobby.remove_from_queue(0).unwrap();
        wait(&mut lobby, |l| l.poll().contains(&Event::Rejected("no".into())));
        drop(lobby);
        let (hello, ready, command) = server.join().unwrap();
        assert_eq!(hello, json!({"type":"hello","role":"lobby","game":"warp-room","token":"t"}));
        assert_eq!(ready, json!({"type":"lobby_ready"}));
        assert_eq!(command, json!({"type":"remove_playlist_entry","expected":{"current":1,"queued":2},"index":0}));
    }

    #[test]
    fn links_are_loopback_only_and_http_reads_plain_and_chunked_bodies() {
        assert_eq!(loopback_only("http://example.com:7913".into()), "");
        assert_eq!(loopback_only("http://127.0.0.1:7913/".into()), "http://127.0.0.1:7913");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut requests = vec![];
            for reply in ["HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"room\":{}}",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n",
                "HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\n\r\n"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap();
                requests.push(String::from_utf8_lossy(&buf[..n]).to_string());
                stream.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        let lobby = Lobby { config: Config { url: "ws://x".into(), lobby_id: "l".into(), token: String::new(), links_base: base },
            party: json!({}), active: false, connected: false, outgoing: None, incoming: None, retry_at: None, ready_due: false };
        assert_eq!(lobby.player_links().unwrap(), json!({"room":{}}));
        assert_eq!(serde_json::from_slice::<Value>(&http(&lobby.config.links_base, "GET", "/x", &[]).unwrap()).unwrap(), json!({"a":1}));
        assert!(lobby.room_pickup("p/../1", "me", true).unwrap_err().contains("409"));
        let requests = server.join().unwrap();
        assert!(requests[2].starts_with("POST /api/room-pickup/p..1/me HTTP/1.1"));
        assert!(requests[2].contains("X-GameNight-Swap: 1"));
    }
}
