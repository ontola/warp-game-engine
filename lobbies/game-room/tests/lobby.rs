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

#[test]
fn the_mouse_clicks_a_player_to_remove_them() {
    let party = json!({
        "players": [{"id": "p0", "name": "Sanne", "color": "#e36c76", "couch": 0}],
        "seats": [{"index": 0, "occupant": {"kind": "local", "player_id": "p0"}, "controller": "pad-a"}],
        "library": [], "playlist": {"entries": [], "current": null},
    });
    let (url, _to_lobby, from_lobby) = fake_host(party);
    let config = splitscreen_club::lobby::Config { url, lobby_id: "game-room".into(), token: "secret".into(), links_base: String::new() };
    let mut lobby = Lobby::connect_with(options(), config).unwrap();
    run_until(&mut lobby, |l| l.accepts_input() && l.world.player("p0").is_some());
    lobby.world.player_mut("p0").unwrap().x = -4.0;
    for _ in 0..30 { tick(&mut lobby); }
    let size = [1920, 1080];
    let p = lobby.world.player("p0").unwrap();
    let body = lobby.screen_point(glam::Vec3::new(p.x as f32, p.y as f32 + 1.0, lobby.actor_depth("p0")), size).unwrap();
    assert_eq!(lobby.player_at(body, size).as_deref(), Some("p0"));
    let click = |lobby: &mut Lobby, at: [f32; 2]| {
        app::Game::pointer(lobby, app::Pointer { position: Some(at), moved: true, pressed: true, ..Default::default() }, size);
        lobby.update(1.0 / 60.0);
    };
    click(&mut lobby, body);
    assert!(lobby.mouse_menu_open(), "a click on a player opens their menu");
    let items = lobby.menu_items(game_room::mouse::MOUSE);
    let remove = items.iter().position(|i| i.action == "remove").unwrap();
    assert!(items[remove].label.contains("Sanne"));
    let (_, rows) = game_room::hud::mouse_menu_rows(&lobby, size).unwrap();
    let row = rows[remove];
    let at = [row[0] + row[2] / 2.0, row[1] + row[3] / 2.0];
    click(&mut lobby, at);
    click(&mut lobby, at);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(m) = from_lobby.recv_timeout(Duration::from_millis(50)) && m["type"] == "leave_party" {
            assert_eq!(m["player_id"], "p0");
            assert!(!lobby.mouse_menu_open());
            return;
        }
    }
    panic!("no leave_party sent");
}

#[test]
fn the_catalog_shows_every_game_and_plays_the_middle_one() {
    use base64::Engine;
    let mut png = vec![];
    image::RgbaImage::from_pixel(6, 8, image::Rgba([200, 40, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let cover = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
    let party = json!({
        "players": [{"id": "p0", "name": "Sanne", "color": "#e36c76"}],
        "seats": [{"index": 0, "occupant": {"kind": "local", "player_id": "p0"}, "controller": "pad-a"}],
        "library": [{"id": "a", "title": "Alpha", "color": "#38d6ff", "cover": cover}, {"id": "b", "title": "Bravo", "color": "#ff3df0"}, {"id": "c", "title": "Charlie", "color": "#7cff6b"}],
        "playlist": {"entries": [], "current": null},
    });
    let (url, _to_lobby, from_lobby) = fake_host(party);
    let config = splitscreen_club::lobby::Config { url, lobby_id: "game-room".into(), token: "secret".into(), links_base: String::new() };
    let mut lobby = Lobby::connect_with(options(), config).unwrap();
    run_until(&mut lobby, |l| l.accepts_input() && l.world.player("p0").is_some());
    let press = |lobby: &mut Lobby, buttons: u32| {
        lobby.input("p0", buttons, [0.0; 2]);
        lobby.update(1.0 / 60.0);
        lobby.input("p0", 0, [0.0; 2]);
        lobby.update(1.0 / 60.0);
    };
    use game_room_sim::world::{BTN_A, BTN_B, BTN_BACK, BTN_DOWN, BTN_LEFT, BTN_RB, BTN_RS};
    // A player that joined holds nothing at first (input waits for a release).
    lobby.input("p0", 0, [0.0; 2]);
    assert_eq!(lobby.host.shelf_games().len(), 3);
    press(&mut lobby, BTN_BACK);
    assert!(!lobby.catalog_open(), "Back does nothing in the room");
    press(&mut lobby, BTN_RS);
    assert!(lobby.catalog_open(), "R3 anywhere switches to the catalog view");
    press(&mut lobby, BTN_RS);
    assert!(!lobby.catalog_open(), "and back");
    // Walk left until the shelf is in front of them.
    for _ in 0..600 {
        if lobby.station_for("p0").is_some_and(|s| s.kind == "shelf") { break; }
        lobby.input("p0", 0, [-1.0, 0.0]);
        tick(&mut lobby);
    }
    lobby.input("p0", 0, [0.0; 2]);
    for _ in 0..10 { tick(&mut lobby); }
    lobby.update(1.0 / 60.0);
    assert_eq!(lobby.station_for("p0").map(|s| s.kind), Some("shelf".into()));
    assert!(lobby.prompts["shelf"].items.iter().any(|i| i.0 == "R3"), "the shelf offers the big catalog on R3");
    press(&mut lobby, BTN_RS);
    assert!(lobby.catalog_open(), "R3 at the shelf opens the catalog");
    for _ in 0..4 { lobby.update(1.0 / 60.0); }
    assert!(lobby.catalog.covers.get("a").is_some_and(|c| c.is_some()), "the data URI cover decodes");
    assert!(lobby.catalog.covers.get("b").is_some_and(|c| c.is_none()), "no cover: a title card");
    let start = lobby.host.shelf_offset;
    press(&mut lobby, BTN_RB);
    assert_eq!(lobby.host.shelf_offset, (start + 1) % 3, "RB browses");
    press(&mut lobby, BTN_DOWN);
    assert_eq!(lobby.host.shelf_offset, (start + 1) % 3, "one row of three: down stays");
    press(&mut lobby, BTN_LEFT);
    assert_eq!(lobby.host.shelf_offset, start, "left browses back");
    let _ = game_room::hud::draw(&lobby, [1920, 1080], 1.0);

    // The mouse clicks another cover to choose it.
    let size = [1920, 1080];
    for _ in 0..60 { lobby.update(1.0 / 60.0); }
    let layout = game_room::catalog::catalog_layout(&lobby, size);
    let &(side, r) = layout.cards.iter().find(|(i, _)| *i != lobby.host.shelf_offset).unwrap();
    app::Game::pointer(&mut lobby, app::Pointer { position: Some([r[0] + 4.0, r[1] + r[3] / 2.0]), moved: true, pressed: true, ..Default::default() }, size);
    lobby.update(1.0 / 60.0);
    assert_eq!(lobby.host.shelf_offset, side, "a click on another cover chooses it");

    // A plays it now; the room stays in the catalog view for after the game.
    let game = lobby.show.shelf[side].id.clone();
    press(&mut lobby, BTN_A);
    assert!(lobby.catalog_open());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "no queue_next sent");
        if let Ok(m) = from_lobby.recv_timeout(Duration::from_millis(50)) && m["type"] == "queue_next" { assert_eq!(m["game"], game.as_str()); break; }
    }
    press(&mut lobby, BTN_B);
    assert!(!lobby.catalog_open(), "B goes back to the room view");
}

#[test]
fn the_back_panel_shows_the_hosts_chords_for_its_player() {
    let mut lobby = Lobby::new(options(), 0, 2).unwrap();
    let player = lobby.world.players[0].id.clone();
    assert!(game_room::hud::back_rows(&lobby).is_none(), "no back_menu, no panel");
    let mut party = lobby.host.party.clone();
    party["back_menu"] = json!({"player": player, "confirm_leave": false, "chords": [
        {"button": "rb", "action": "next", "label": "Play next game"}, {"button": "b", "action": "leave"}]});
    lobby.set_party(party.clone());
    let (who, rows) = game_room::hud::back_rows(&lobby).unwrap();
    assert_eq!(who.as_deref(), Some(player.as_str()));
    assert_eq!(rows, vec![("RB".to_string(), "Play next game".to_string(), false), ("B".into(), "Leave the party".into(), false)]);
    party["back_menu"]["confirm_leave"] = json!(true);
    lobby.set_party(party.clone());
    assert_eq!(game_room::hud::back_rows(&lobby).unwrap().1[1], ("B".into(), "Press B again to leave".into(), true));
    let _ = game_room::hud::draw(&lobby, [1920, 1080], 1.0);
    party["back_menu"] = Value::Null;
    lobby.set_party(party);
    assert!(game_room::hud::back_rows(&lobby).is_none(), "an empty back_menu hides it");
}

#[test]
fn a_keyboard_player_sees_keys() {
    let mut lobby = Lobby::new(options(), 0, 1).unwrap();
    let mut party = lobby.host.party.clone();
    party["seats"][0]["controller"] = json!("web:keyboard");
    lobby.set_party(party);
    let id = lobby.world.players[0].id.clone();
    assert!(lobby.on_keyboard(&id));
    assert_eq!(lobby.button_name(&id, "A"), "Space");
    assert_eq!(game_room::paint::key_cap("Y"), Some("F"));
}

#[test]
fn a_keyboard_player_walking_off_keeps_keycaps() {
    let mut lobby = Lobby::new(options(), 0, 1).unwrap();
    let mut party = lobby.host.party.clone();
    party["players"] = json!([party["players"][0].clone()]);
    party["seats"] = json!([party["seats"][0].clone()]);
    party["seats"][0]["controller"] = json!("web:keyboard");
    party["warm_session"] = json!(null);
    party["library"] = json!([{"id": "spaceracer", "title": "Space Racer", "color": "#38d6ff", "keyboard": true, "shared_keyboard": 2}]);
    party["playlist"] = json!({"entries": [{"game": "spaceracer", "title": "Space Racer"}]});
    lobby.set_party(party);
    lobby.started_once = false;
    let id = lobby.world.players[0].id.clone();
    lobby.world.player_mut(&id).unwrap().x = 0.0;
    for _ in 0..20 { tick(&mut lobby); lobby.update(1.0 / 60.0); }
    lobby.world.player_mut(&id).unwrap().x = -9.0;
    for t in 0..40 {
        tick(&mut lobby); lobby.update(1.0 / 60.0);
        assert!(lobby.prompts["tv"].keys, "the TV's nudge keeps its keycap as it tucks away (tick {t})");
    }
}
