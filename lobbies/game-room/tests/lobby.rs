//! The live lobby against a fake Splitscreen Club host: party snapshots seat
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
    let config = splitscreen_club::lobby::Config { url, lobby_id: "game-room".into(), token: "secret".into(), links_base: String::new() };
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

#[test]
fn the_mouse_is_nobody_but_works_the_room() {
    let party = json!({
        "players": [],
        "seats": [],
        "library": [{"id": "spaceracer", "title": "Space Racer", "color": "#38d6ff"}, {"id": "neon-siege", "title": "Neon Siege", "color": "#ff3df0"}],
        "playlist": {"entries": [{"game": "spaceracer", "title": "Space Racer"}], "current": null},
    });
    let (url, _to_lobby, from_lobby) = fake_host(party);
    let config = splitscreen_club::lobby::Config { url, lobby_id: "game-room".into(), token: "secret".into(), links_base: String::new() };
    let mut lobby = Lobby::connect_with(options(), config).unwrap();
    run_until(&mut lobby, |l| l.accepts_input());
    let size = [1920, 1080];
    let point = |lobby: &mut Lobby, at: [f32; 2], pressed: bool, wheel: f32| {
        app::Game::pointer(lobby, app::Pointer { position: Some(at), moved: true, pressed, held: false, wheel, ..Default::default() }, size);
        lobby.update(1.0 / 60.0);
    };
    let sent = |from: &mpsc::Receiver<Value>, kind: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(m) = from.recv_timeout(Duration::from_millis(50)) && m["type"] == kind { return m; }
        }
        panic!("no {kind} sent");
    };

    // Pointing at the TV lights its buttons; nobody joins.
    let tv = lobby.screen_point(glam::Vec3::new(0.0, 3.95, -2.0), size).unwrap();
    point(&mut lobby, tv, false, 0.0);
    assert!(lobby.prompt_active["tv"], "the TV's buttons show for the mouse");
    assert!(lobby.world.players.is_empty() && lobby.profiles.is_empty(), "the mouse is not a player");
    // A click does Y: start the next game.
    point(&mut lobby, tv, true, 0.0);
    sent(&from_lobby, "next");
    assert!(lobby.prompts["tv"].mouse, "for the mouse the buttons show a mouse, not Y/X");

    // The wheel over the shelf browses it.
    let shelf_x = ((game_room_sim::layout::GAME_SHELF.0 + game_room_sim::layout::GAME_SHELF.1) / 2.0) as f32;
    let shelf = lobby.screen_point(glam::Vec3::new(shelf_x, 1.5, -2.0), size).unwrap();
    point(&mut lobby, shelf, false, 0.0);
    assert!(lobby.prompt_active["shelf"]);
    let offset = lobby.host.shelf_offset;
    point(&mut lobby, shelf, false, -1.0);
    assert_ne!(lobby.host.shelf_offset, offset, "the wheel browses the shelf");

    // The exit door quits for the mouse, on a second click.
    let door = lobby.screen_point(glam::Vec3::new(game_room_sim::layout::EXIT_DOOR as f32, 1.5, -2.0), size).unwrap();
    point(&mut lobby, door, false, 0.0);
    assert!(lobby.prompt_active["exit"] && lobby.prompts["exit"].mouse);
    assert_eq!(lobby.prompts["exit"].items[0].1, "Quit Splitscreen Club");
    point(&mut lobby, door, true, 0.0);
    assert_eq!(lobby.prompts["exit"].items[0].1, "Click again to quit");
    point(&mut lobby, door, true, 0.0);
    sent(&from_lobby, "quit_party");
    point(&mut lobby, tv, false, 0.0);

    // Escape opens the room's own menu; clicking Quit twice quits Splitscreen Club.
    // (Not in the first moments after taking the screen: that Escape may
    // be the one that closed the game in front.)
    std::thread::sleep(Duration::from_millis(800));
    assert!(app::Game::escape(&mut lobby), "Escape never closes the room");
    assert!(lobby.mouse_menu_open());
    let (_, rows) = game_room::hud::mouse_menu_rows(&lobby, size).unwrap();
    let quit = rows.last().unwrap();
    let at = [quit[0] + quit[2] / 2.0, quit[1] + quit[3] / 2.0];
    point(&mut lobby, at, true, 0.0);
    point(&mut lobby, at, true, 0.0);
    sent(&from_lobby, "quit_party");
}

#[test]
fn new_rooms_show_where_to_start_and_where_scanned_profiles_wait() {
    let party = json!({
        "players": [],
        "seats": [],
        "library": [{"id": "spaceracer", "title": "Space Racer", "color": "#38d6ff"}],
        "playlist": {"entries": [{"game": "spaceracer", "title": "Space Racer"}], "current": null},
    });
    let (url, to_lobby, _from_lobby) = fake_host(party);
    let config = splitscreen_club::lobby::Config { url, lobby_id: "game-room".into(), token: "secret".into(), links_base: String::new() };
    let mut lobby = Lobby::connect_with(options(), config).unwrap();
    run_until(&mut lobby, |l| l.accepts_input());
    // An empty room nudges nobody, even with a profile waiting.
    lobby.set_links(json!({"room": {"pending": [{"id": "w1", "profile": {"display_name": "Sanne"}}]}}));
    for _ in 0..10 { lobby.update(1.0 / 60.0); }
    assert!(lobby.nudges.is_empty() && !lobby.prompt_active["tv"], "nothing hops before someone joins");

    // Someone joins and stands away from the TV and the doors.
    to_lobby.send(json!({"type": "party_state", "party": {
        "players": [{"id": "p0", "name": "Joep", "color": "#e36c76", "couch": 0}],
        "seats": [{"index": 0, "occupant": {"kind": "local", "player_id": "p0"}, "controller": "pad-a"}],
        "library": [{"id": "spaceracer", "title": "Space Racer", "color": "#38d6ff"}],
        "playlist": {"entries": [{"game": "spaceracer", "title": "Space Racer"}], "current": null}}})).unwrap();
    run_until(&mut lobby, |l| l.world.player("p0").is_some());
    let p = lobby.world.player_mut("p0").unwrap();
    p.x = -9.0;
    for _ in 0..30 { tick(&mut lobby); }
    run_until(&mut lobby, |l| l.nudges.contains("tv") && l.nudges.contains("door0"));
    assert_eq!(lobby.prompts["tv"].items[0].1, "Play");
    assert!(lobby.prompt_active["tv"], "before the first game, Play hops at the TV for nobody in particular");
    assert!(lobby.prompt_active["door0"] && lobby.prompts["door0"].enabled, "a scanned profile's door shows Pick up");
    assert!(!lobby.nudges.contains("door1"), "only doors with a profile waiting");
    assert_eq!(lobby.prompts["tv"].conceal, 1.0, "from afar the nudge shows only its words");

    // Walking up to the TV pops its button in and stops the hop.
    lobby.world.player_mut("p0").unwrap().x = 0.0;
    run_until(&mut lobby, |l| { tick(l); l.prompts["tv"].conceal == 0.0 });
    assert!(!lobby.nudges.contains("tv") && lobby.prompt_active["tv"]);

    // Once everyone has a profile, the doors stop hopping.
    lobby.set_links(json!({"room": {"pending": [{"id": "w1", "profile": {"display_name": "Sanne"}}]}, "linked": ["p0"]}));
    lobby.update(1.0 / 60.0);
    assert!(!lobby.nudges.contains("door0"), "nobody left to pick a profile up");

    // Once a game has run, the TV is no longer nudged.
    to_lobby.send(json!({"type": "party_state", "party": {"players": [], "seats": [],
        "library": [{"id": "spaceracer", "title": "Space Racer"}],
        "active_session": {"game": "spaceracer", "phase": "running"},
        "playlist": {"entries": [], "current": null}}})).unwrap();
    run_until(&mut lobby, |l| l.started_once);
    to_lobby.send(json!({"type": "party_state", "party": {"players": [], "seats": [],
        "library": [{"id": "spaceracer", "title": "Space Racer"}],
        "playlist": {"entries": [{"game": "spaceracer", "title": "Space Racer"}], "current": null}}})).unwrap();
    run_until(&mut lobby, |l| l.host.current_game().is_none());
    lobby.update(1.0 / 60.0);
    assert!(!lobby.nudges.contains("tv"));
}
