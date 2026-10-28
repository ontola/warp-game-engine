//! The live lobby against a fake GameNight host: party snapshots seat
//! players, controller frames move them, and stations send host commands.

use game_room::live::Lobby;
use game_room::stage::Options;
use serde_json::{json, Value};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tungstenite::Message;

fn options() -> Options { Options { probes: false, fog: false, ..Default::default() } }

/// A host that welcomes the lobby, hands it the screen, then sends what the
/// test queues and reports every message the lobby sends.
fn fake_host(party: Value) -> (String, mpsc::Sender<Value>, mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (to_lobby, outbox) = mpsc::channel::<Value>();
    let (inbox, from_lobby) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        let mut welcomed = false;
        loop {
            match socket.read() {
                Ok(Message::Text(t)) => {
                    let m: Value = serde_json::from_str(&t).unwrap();
                    if m["type"] == "hello" && !welcomed {
                        welcomed = true;
                        socket.send(Message::Text(json!({"type": "welcome", "protocol_version": 1, "party": party}).to_string().into())).unwrap();
                    } else if m["type"] == "lobby_ready" {
                        socket.send(Message::Text(json!({"type": "lobby_focus", "active": true}).to_string().into())).unwrap();
                    }
                    if inbox.send(m).is_err() { return; }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => return,
            }
            while let Ok(m) = outbox.try_recv() { socket.send(Message::Text(m.to_string().into())).unwrap(); }
        }
    });
    (url, to_lobby, from_lobby)
}

fn run_until(lobby: &mut Lobby, mut done: impl FnMut(&mut Lobby) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        lobby.update(1.0 / 60.0);
        if done(lobby) { return; }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out");
}

fn tick(lobby: &mut Lobby) { app::Game::tick(lobby, &[]); }

#[test]
fn a_seated_player_starts_the_queued_game_at_the_tv() {
    let party = json!({
        "players": [{"id": "p0", "name": "Joep", "color": "#e36c76", "skin_color": "#f5e9be", "couch": 0}],
        "seats": [{"index": 0, "occupant": {"kind": "local", "player_id": "p0"}, "controller": "pad-a"}],
        "library": [{"id": "spaceracer", "title": "Space Racer", "color": "#38d6ff"}],
        "playlist": {"entries": [{"game": "spaceracer", "title": "Space Racer"}], "current": null},
    });
    let (url, to_lobby, from_lobby) = fake_host(party);
    let config = gamenight::lobby::Config { url, lobby_id: "game-room".into(), token: "secret".into(), links_base: String::new() };
    let mut lobby = Lobby::connect_with(options(), config).unwrap();
    run_until(&mut lobby, |l| l.world.player("p0").is_some() && l.host.connected);
    let hello = from_lobby.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(hello, json!({"type": "hello", "role": "lobby", "game": "game-room", "token": "secret"}));
    // Let the player land, then press Y in front of the TV.
    let p = lobby.world.player_mut("p0").unwrap();
    p.x = 0.0;
    p.y = 0.0;
    for _ in 0..30 { tick(&mut lobby); }
    run_until(&mut lobby, |l| l.accepts_input());
    for buttons in [0, 8, 0] {
        to_lobby.send(json!({"type": "controller_frame", "controllers": [{"controller": "pad-a", "buttons": buttons, "axes": [0, 0, 0, 0, 0, 0]}]})).unwrap();
        let before = lobby.world.player("p0").map(|p| p.last_input);
        run_until(&mut lobby, |l| l.world.player("p0").map(|p| p.last_input) != before || buttons == 0);
        tick(&mut lobby);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sent = vec![];
    while Instant::now() < deadline && !sent.iter().any(|m: &Value| m["type"] == "next") {
        lobby.update(1.0 / 60.0);
        while let Ok(m) = from_lobby.try_recv() { sent.push(m); }
    }
    assert!(sent.iter().any(|m| m == &json!({"type": "next"})), "the TV starts the next game: {sent:?}");
}
