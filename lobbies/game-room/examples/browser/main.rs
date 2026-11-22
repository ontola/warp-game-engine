//! Splitscreen Club in a browser: the Game Room first, then the Luau games
//! on its shelf, all on one engine download (`engine/tools/build-web.py
//! --warp`). A small host lives in the page: the room's commands (Play,
//! queue, skip, resume, door pickups) go to it instead of a Splitscreen
//! Club host, it loads the game with warp::web and switches to it, and Back
//! (or the game's own Quit) brings the room back without reloading it. Back
//! pauses the game: the TV offers Resume, as at home. Through the cloud
//! room (cloud.rs) phones join with the room code or QR code, wait at the
//! doors with their profile, and pick the next game.
//!
//! A device that presses a button takes a free seat or joins as a new
//! player; a pad that stays gone leaves after the room's 30 s.
//!
//! `?players=N` seats at the start (default 2), `?game=NAME` starts in that game instead of the room,
//! `?cloud=URL` the Splitscreen Club cloud (default: the page's own origin;
//! `?cloud=off` plays without one). See engine/docs/browser.md.

#[cfg(target_arch = "wasm32")]
mod cloud;

#[cfg(target_arch = "wasm32")]
mod page {
    use app::{Config, Controls, Device, Draw, Frame, Game};
    use game_room::live::Lobby;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::wasm_bindgen;

    /// Shelf colours for the games, in library order.
    const COLORS: [&str; 6] = ["#38d6ff", "#3ddc97", "#ffb454", "#ff4f8b", "#e8c14a", "#a78bfa"];
    /// Colours for players who join with a pad, by seat.
    const PLAYER_COLORS: [&str; 8] = ["#e36c76", "#49b8bc", "#d7ad43", "#a67bdd", "#4daf7c", "#cf7aaf", "#5b8def", "#f08c4a"];
    /// The most seats a table has.
    const MAX_SEATS: usize = 8;

    /// The page's host: the party the room shows, and the screens not on screen.
    #[derive(Default)]
    pub(crate) struct Host {
        pub(crate) party: Value,
        /// The party changed since the room last saw it.
        pub(crate) dirty: bool,
        /// The room's links (room code, QR code, profiles at the doors) and
        /// whether the room has seen them.
        pub(crate) links: Value,
        pub(crate) links_dirty: bool,
        /// Each player's look before a phone profile replaced it.
        pub(crate) own_looks: Vec<Value>,
        /// The room, while a game has the screen.
        room: Option<app::web::Parked>,
        /// A game Back paused, while the room has the screen.
        paused: Option<(String, app::web::Parked)>,
        /// The game on screen (its store id); None: the room.
        pub(crate) on_screen: Option<String>,
        /// A game being fetched and loaded.
        pub(crate) loading: Option<String>,
        pub(crate) players: usize,
        /// The players `?players=` asked for: they stay when their pad goes.
        base: usize,
        /// The device on each seat; None: free (the next device pressing a
        /// button takes it).
        bound: Vec<Option<Device>>,
        pub(crate) cloud: crate::cloud::Cloud,
    }

    thread_local! { static HOST: RefCell<Host> = RefCell::new(Host::default()); }

    pub(crate) fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R { HOST.with(|h| f(&mut h.borrow_mut())) }

    pub(crate) fn game_meta<'a>(party: &'a Value, game: &str) -> Option<&'a Value> {
        party["library"].as_array()?.iter().find(|g| g["id"] == game || g["folder"] == game)
    }

    fn title(party: &Value, game: &str) -> String {
        game_meta(party, game).and_then(|g| g["title"].as_str()).unwrap_or(game).to_string()
    }

    fn set_session(h: &mut Host, game: Option<&str>, phase: &str) {
        // One id per session, kept over pause and resume (the cloud's insights count it).
        let id = match &h.party["active_session"] { s if game.is_some_and(|g| s["game"] == g) => s["id"].clone(), _ => json!(crate::cloud::uuid()) };
        h.party["active_session"] = match game { Some(g) => json!({"id": id, "game": g, "phase": phase}), None => Value::Null };
        h.dirty = true;
        h.cloud.refresh = true;
    }

    /// Put `game` first in the queue (or last, unless `first`).
    pub(crate) fn queue(h: &mut Host, game: &str, first: bool) {
        let entry = json!({"game": game, "title": title(&h.party, game)});
        let Some(entries) = h.party["playlist"]["entries"].as_array_mut() else { return };
        entries.retain(|e| e["game"] != game);
        if first { entries.insert(0, entry) } else { entries.push(entry) }
        h.dirty = true;
        h.cloud.refresh = true;
    }

    /// Every device's input before each tick, in the room and in games. A
    /// device that presses something and drives nobody takes the first free
    /// seat, or joins as a new player. A seat whose pad disconnects loses its
    /// controller in the party: the room puts that player to sleep and after
    /// its usual 30 s sends `leave_party` (see [`leave_party`]). The same pad
    /// coming back wakes its player. Returns the devices of each seat.
    fn devices(h: &mut Host, devices: &[(Device, Controls)]) -> Option<Vec<Vec<Device>>> {
        for (dev, c) in devices {
            let active = c.pressed.0 != 0 || c.mv[0].abs().max(c.mv[1].abs()) > 0.6;
            if !active || h.bound.contains(&Some(*dev)) { continue; }
            let seat = match h.bound.iter().position(Option::is_none) {
                Some(i) => i,
                None if h.bound.len() < MAX_SEATS => add_player(h),
                None => continue,
            };
            h.bound[seat] = Some(*dev);
            app::web::log(&format!("{dev} plays as {}", h.party["players"][seat]["name"].as_str().unwrap_or("?")));
        }
        for (i, b) in h.bound.iter().enumerate() {
            let controller = match b {
                Some(d) if devices.iter().any(|(x, _)| x == d) => json!(format!("web:{d}")),
                Some(_) => Value::Null,
                None => json!(format!("demo:{i}")),
            };
            if h.party["seats"][i]["controller"] != controller {
                h.party["seats"][i]["controller"] = controller;
                h.dirty = true;
            }
        }
        Some(h.bound.iter().map(|b| b.iter().copied().collect()).collect())
    }

    /// A new player on a new seat at the end; returns the seat.
    fn add_player(h: &mut Host) -> usize {
        let i = h.bound.len();
        let id = format!("p{i}");
        let mut look = h.own_looks.get(i % h.base.max(1)).cloned().unwrap_or_else(|| json!({}));
        look["id"] = json!(id);
        look["name"] = json!(format!("Player {}", i + 1));
        look["color"] = json!(PLAYER_COLORS[i % PLAYER_COLORS.len()]);
        look["avatar"] = json!(game_room::face::demo(i));
        look["couch"] = json!(i);
        h.own_looks.push(look.clone());
        if let Some(list) = h.party["players"].as_array_mut() { list.push(look); }
        if let Some(seats) = h.party["seats"].as_array_mut() { seats.push(json!({"index": i, "occupant": {"kind": "local", "player_id": id}, "controller": null})); }
        h.bound.push(None);
        h.players = h.bound.len();
        h.cloud.add_seat();
        h.dirty = true;
        i
    }

    /// The room took a player out (their pad stayed gone). A player who
    /// joined leaves the table when theirs is the last seat; one of
    /// `?players=` (or a joined one between others) stays as a free seat for
    /// the next pad, in their own look again.
    fn leave_party(h: &mut Host, id: &str) {
        let Some(i) = id.strip_prefix('p').and_then(|i| i.parse::<usize>().ok()).filter(|i| *i < h.bound.len()) else { return };
        h.bound[i] = None;
        h.cloud.free_seat(i);
        if let Some(own) = h.own_looks.get(i) { h.party["players"][i] = own.clone(); }
        while h.bound.len() > h.base && h.bound.last() == Some(&None) {
            h.bound.pop();
            h.own_looks.pop();
            if let Some(list) = h.party["players"].as_array_mut() { list.pop(); }
            if let Some(seats) = h.party["seats"].as_array_mut() { seats.pop(); }
            h.cloud.drop_seat();
        }
        h.players = h.bound.len();
        h.dirty = true;
    }

    /// The room on screen: its lobby, and the page's host answering it.
    struct Room { lobby: Lobby }

    impl Room {
        fn command(&mut self, msg: &Value) {
            let game = msg["game"].as_str().unwrap_or_default().to_string();
            match msg["type"].as_str().unwrap_or_default() {
                "queue_next" => with(|h| queue(h, &game, true)),
                "queue_game" => with(|h| queue(h, &game, msg["first"] == true)),
                "remove_playlist_entry" => with(|h| {
                    let i = msg["index"].as_u64().unwrap_or(0) as usize;
                    let Some(entries) = h.party["playlist"]["entries"].as_array_mut() else { return };
                    // Skipped games go to the back: the queue never runs dry.
                    if i < entries.len() { let e = entries.remove(i); entries.push(e); }
                    h.dirty = true;
                }),
                "next" => {
                    let first = with(|h| {
                        let entries = h.party["playlist"]["entries"].as_array_mut()?;
                        if entries.is_empty() { return None; }
                        let e = entries.remove(0);
                        entries.push(e.clone());
                        h.dirty = true;
                        e["game"].as_str().map(String::from)
                    });
                    if let Some(game) = first { start(game); }
                }
                "close_overlay" => resume(),
                "leave_party" => with(|h| leave_party(h, msg["player_id"].as_str().unwrap_or_default())),
                "room_pickup" => {
                    let seat = msg["player_id"].as_str().and_then(|p| p.strip_prefix('p')).and_then(|i| i.parse().ok());
                    if let (Some(pending), Some(seat)) = (msg["pending"].as_str(), seat) { crate::cloud::pickup(pending.to_string(), seat); }
                }
                _ => {}
            }
        }
    }

    impl Game for Room {
        fn local_players(&self) -> usize { self.lobby.local_players() }
        fn tick(&mut self, controls: &[Controls]) {
            if let Some((party, players)) = with(|h| std::mem::take(&mut h.dirty).then(|| (h.party.clone(), h.players))) {
                self.lobby.set_local((0..players).map(|i| format!("p{i}")).collect());
                self.lobby.set_party(party);
            }
            if let Some(links) = with(|h| std::mem::take(&mut h.links_dirty).then(|| h.links.clone())) { self.lobby.set_links(links); }
            self.lobby.tick(controls);
            for msg in self.lobby.outbox.as_mut().map(std::mem::take).unwrap_or_default() { self.command(&msg); }
        }
        fn pointer(&mut self, pointer: app::Pointer, size: [u32; 2]) { self.lobby.pointer(pointer, size); }
        // The cursor points and clicks in the room: never captured.
        fn mouse_capture(&self) -> Option<bool> { Some(false) }
        fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> { self.lobby.draw(frame) }
        fn title(&self) -> Option<String> { self.lobby.title() }
        fn escape(&mut self) -> bool { self.lobby.escape() }
        fn hold_quality(&self) -> bool { self.lobby.hold_quality() }
    }

    fn room_config() -> Config {
        Config { title: "Splitscreen Club".into(), size: [1600, 900], keyboard: true, pointer: true, ..Default::default() }
    }

    /// Load `game` (its store id or folder; fetching its files the first
    /// time) and switch to it; the room waits behind it. A paused game is
    /// dropped, unless it is `game`: that one resumes.
    pub(crate) fn start(game: String) {
        // The paused game itself (the page's button, a phone's pick): resume it.
        let paused = with(|h| {
            let id = game_meta(&h.party, &game).and_then(|m| m["id"].as_str()).map(String::from)?;
            (h.on_screen.is_none() && h.paused.as_ref().is_some_and(|(p, _)| *p == id)).then_some(())
        });
        if paused.is_some() { return resume(); }
        let found = with(|h| {
            let meta = game_meta(&h.party, &game)?;
            let (id, folder) = (meta["id"].as_str()?.to_string(), meta["folder"].as_str()?.to_string());
            if h.loading.is_some() { return None; }
            h.loading = Some(id.clone());
            h.party["warm_session"] = json!({"game": id, "phase": "preparing", "progress": 50, "progress_label": "Loading"});
            h.dirty = true;
            h.cloud.refresh = true;
            Some((id, folder, h.players))
        });
        let Some((game, folder, players)) = found else { app::web::log(&format!("{game}: not on this page, or a game is loading")); return };
        wasm_bindgen_futures::spawn_local(async move {
            let t0 = web_time::Instant::now();
            let loaded = warp::web::load(&folder, Some(players)).await;
            with(|h| {
                h.loading = None;
                h.party["warm_session"] = Value::Null;
                h.dirty = true;
                let (cfg, lua) = match loaded {
                    Ok(l) => l,
                    Err(e) => { app::web::log(&format!("{folder}: {e}")); return; }
                };
                app::web::log(&format!("{folder}: ready {:.0} ms after Play", t0.elapsed().as_secs_f32() * 1000.0));
                h.paused = None;
                // From the room: keep it. From another game (the page's
                // buttons): that game goes, the room is already kept.
                if h.on_screen.is_none() { app::web::switch_keeping(cfg, Box::new(lua), |p| with(|h| h.room = Some(p))); } else { app::web::switch(cfg, Box::new(lua)); }
                h.on_screen = Some(game.clone());
                set_session(h, Some(&game), "running");
            });
        });
    }

    /// TV Resume: the paused game comes back where it was.
    fn resume() {
        with(|h| {
            let Some((game, paused)) = h.paused.take() else { return };
            app::web::resume(paused, Some(Box::new(|p| with(|h| h.room = Some(p)))));
            h.on_screen = Some(game.clone());
            set_session(h, Some(&game), "running");
        });
    }

    /// Back or Quit in a game: the room comes back; Back keeps the game paused.
    fn leave(why: app::web::Exit) -> bool {
        with(|h| {
            if h.on_screen.is_none() { return false; }
            let Some(room) = h.room.take() else { return false };
            let game = h.on_screen.take().unwrap_or_default();
            if why == app::web::Exit::Back {
                let name = game.clone();
                app::web::resume(room, Some(Box::new(move |p| with(|h| h.paused = Some((name, p))))));
                set_session(h, Some(&game), "paused");
            } else {
                app::web::resume(room, None);
                set_session(h, None, "");
            }
            true
        })
    }

    pub fn main() {
        script::web::init();
        app::web::install_panic_hook();
        wasm_bindgen_futures::spawn_local(async {
            if let Err(e) = run().await { app::web::progress(Some(0.3), &format!("Could not start: {e}")); app::web::log(&e); }
        });
    }

    async fn run() -> Result<(), String> {
        let library: Vec<Value> = serde_json::from_slice(&app::web::fetch("games/library.json").await?).map_err(|e| format!("games/library.json: {e}"))?;
        let players = warp::web::param("players").and_then(|p| p.parse().ok()).unwrap_or(2usize).clamp(1, 8);
        app::web::progress(Some(0.2), "Building the Game Room");
        let t0 = web_time::Instant::now();
        let mut options = game_room::stage::Options::default();
        // The light baked ahead (game-room bake-light): a page bakes on one
        // thread, which takes most of a minute. Without it, fewer rays.
        match app::web::fetch("room-light.bin").await {
            Ok(bytes) => options.light_cache = Some(bytes),
            Err(e) => { app::web::log(&format!("room-light.bin: {e}; baking the light here")); options.rays = 16; }
        }
        let mut lobby = Lobby::new(options, players, 0)?;
        lobby.outbox = Some(vec![]);
        app::web::log(&format!("Game Room built in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0));
        // The demo party, with this page's games, no phones and nothing loading.
        let mut party = lobby.host.party.clone();
        let library: Vec<Value> = library.into_iter().enumerate().map(|(i, mut g)| { if g["color"].is_null() { g["color"] = json!(COLORS[i % COLORS.len()]); } g }).collect();
        party["playlist"] = json!({"entries": library.iter().map(|g| json!({"game": g["id"], "title": g["title"]})).collect::<Vec<_>>(), "current": null});
        party["library"] = Value::Array(library);
        party["phones"] = json!([]);
        party["warm_session"] = Value::Null;
        party["active_session"] = Value::Null;
        if let Some(o) = party.as_object_mut() { o.remove("now_playing"); }
        if let Some(list) = party["players"].as_array_mut() {
            for (i, p) in list.iter_mut().enumerate() { p["name"] = json!(format!("Player {}", i + 1)); }
        }
        let own_looks = party["players"].as_array().cloned().unwrap_or_default();
        lobby.set_party(party.clone());
        // Nobody at the doors until the cloud room reports someone.
        let links = json!({"room": {"pending": []}, "linked": {}});
        lobby.set_links(links.clone());
        with(|h| { h.party = party; h.players = players; h.base = players; h.bound = vec![None; players]; h.links = links; h.own_looks = own_looks; });
        app::web::on_exit(leave);
        app::web::on_devices(|d| with(|h| devices(h, d)));
        app::web::run(room_config(), Box::new(Room { lobby }));
        crate::cloud::start(players);
        if let Some(game) = warp::web::param("game") { start(game); }
        Ok(())
    }

    /// Start game `name` (folder or store id) from the page (its buttons), as the shelf's Play does.
    #[wasm_bindgen]
    pub fn switch_game(name: String) { start(name); }

    /// Claim waiting profile `pending` (its id) for seat `seat`, as a player
    /// at a door does (for tests, and pages with their own controls).
    #[wasm_bindgen]
    pub fn pick_up(pending: String, seat: usize) { crate::cloud::pickup(pending, seat); }

    /// Back to the room, as Back on a pad does.
    #[wasm_bindgen]
    pub fn back_to_room() { leave(app::web::Exit::Back); }
}

#[cfg(target_arch = "wasm32")]
fn main() { page::main(); }

#[cfg(not(target_arch = "wasm32"))]
fn main() { eprintln!("browser is the Game Room's browser build: engine/tools/build-web.py --warp (engine/docs/browser.md)"); }
