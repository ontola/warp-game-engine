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
//! `?players=N` seats at the start (default none: the room asks for a
//! controller or the QR code until someone joins), `?game=NAME` starts in that game instead of the room,
//! `?cloud=URL` the Splitscreen Club cloud (default: the page's own origin;
//! `?cloud=off` plays without one). See engine/docs/browser.md.

#[cfg(target_arch = "wasm32")]
mod cloud;

#[cfg(target_arch = "wasm32")]
mod page {
    use app::{Buttons, Config, Controls, Device, Draw, Frame, Game};
    use game_room::live::Lobby;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::wasm_bindgen;

    /// Shelf colours for the games, in library order.
    const COLORS: [&str; 6] = ["#38d6ff", "#3ddc97", "#ffb454", "#ff4f8b", "#e8c14a", "#a78bfa"];
    /// Colours for players who join with a pad, by seat.
    const PLAYER_COLORS: [&str; 8] = ["#e36c76", "#49b8bc", "#d7ad43", "#a67bdd", "#4daf7c", "#cf7aaf", "#5b8def", "#f08c4a"];
    /// Skins for players who join, by seat (the demo party's).
    const SKINS: [&str; 6] = ["#f5e9be", "#d99b72", "#efc39a", "#905c40", "#bf855b", "#654332"];
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
        /// Seats of players who play on their phone alone (the cloud's
        /// "phone seat"): no device takes them.
        pub(crate) phone: Vec<bool>,
        /// The next new player's id number (ids stay when others leave).
        next_id: usize,
        pub(crate) cloud: crate::cloud::Cloud,
        /// A key pressed since the room last ticked (its buttons).
        key: Option<u32>,
        /// The room log for the cloud room's phones, oldest first (its
        /// `activity`), and the next line's id.
        pub(crate) activity: Vec<Value>,
        activity_id: u64,
        /// Covers fetched for the room's catalog: (game id, PNG).
        covers: Vec<(String, Vec<u8>)>,
    }

    thread_local! { static HOST: RefCell<Host> = RefCell::new(Host::default()); }

    pub(crate) fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R { HOST.with(|h| f(&mut h.borrow_mut())) }

    pub(crate) fn game_meta<'a>(party: &'a Value, game: &str) -> Option<&'a Value> {
        party["library"].as_array()?.iter().find(|g| g["id"] == game || g["folder"] == game)
    }

    fn min_players(party: &Value, game: &str) -> usize {
        game_meta(party, game).and_then(|g| g["min_players"].as_u64()).unwrap_or(1) as usize
    }

    /// Whether `players` people are enough for `game`.
    pub(crate) fn fits(party: &Value, game: &str, players: usize) -> bool { players >= min_players(party, game) }

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

    /// A line in the room log (the phones' Room log): `kind` as the
    /// party protocol's ActivityKind (joined, left, game_started, game...).
    pub(crate) fn note(h: &mut Host, kind: &str, text: &str, game: Option<&str>) {
        h.activity_id += 1;
        let at = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut line = json!({"id": h.activity_id, "at": at, "kind": kind, "text": text.chars().take(300).collect::<String>()});
        if let Some(g) = game { line["game"] = json!(g); }
        h.activity.push(line);
        let skip = h.activity.len().saturating_sub(100);
        h.activity.drain(..skip);
        h.cloud.refresh = true;
    }

    fn name(h: &Host, seat: usize) -> String { h.party["players"][seat]["name"].as_str().unwrap_or("Someone").to_string() }

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
    /// device that presses something (the keyboard: Enter) and drives nobody takes the first free
    /// seat, or joins as a new player. A seat whose pad disconnects loses its
    /// controller in the party: the room puts that player to sleep and after
    /// its usual 30 s sends `leave_party` (see [`leave_party`]). The same pad
    /// coming back wakes its player. Returns the devices of each seat.
    fn devices(h: &mut Host, devices: &[(Device, Controls)]) -> Option<Vec<Vec<Device>>> {
        for (dev, c) in devices {
            // Any key on the keyboard: the keyboard on the room's table hops.
            // (WASD and the arrows move: they count as keys too.)
            if *dev == Device::Keyboard && (c.pressed.0 != 0 || c.mv[0].abs().max(c.mv[1].abs()) > 0.5) {
                h.key = Some(c.pressed.0 | ((c.mv[0] > 0.0) as u32) << 30 | ((c.mv[1] > 0.0) as u32) << 31);
                if !c.pressed.has(Buttons::START) && !h.bound.contains(&Some(Device::Keyboard)) { key_hint(); }
            }
            // A pad joins with any button or its stick; the keyboard only
            // with Enter, so typing or a stray key adds nobody.
            let active = match dev {
                Device::Keyboard => c.pressed.has(Buttons::START),
                Device::Pad(_) => c.pressed.0 != 0 || c.mv[0].abs().max(c.mv[1].abs()) > 0.6,
            };
            if !active || h.bound.contains(&Some(*dev)) { continue; }
            let seat = match (0..h.bound.len()).find(|&i| h.bound[i].is_none() && !h.phone[i]) {
                Some(i) => i,
                None if h.bound.len() < MAX_SEATS => add_player(h, false),
                None => continue,
            };
            h.bound[seat] = Some(*dev);
            let text = format!("{} joined with {}", name(h, seat), if *dev == Device::Keyboard { "the keyboard" } else { "a controller" });
            note(h, "joined", &text, None);
            app::web::log(&format!("{dev} plays as {}", h.party["players"][seat]["name"].as_str().unwrap_or("?")));
        }
        for (i, b) in h.bound.iter().enumerate() {
            let controller = match b {
                Some(d) if devices.iter().any(|(x, _)| x == d) => json!(format!("web:{d}")),
                Some(_) => Value::Null,
                // A phone player has no controller; the party lists its phone.
                None if h.phone[i] => Value::Null,
                None => json!(format!("demo:{i}")),
            };
            if h.party["seats"][i]["controller"] != controller {
                h.party["seats"][i]["controller"] = controller;
                h.dirty = true;
            }
        }
        warp::web::set_seats(seats(h));
        for line in warp::web::take_room_log() { let game = h.on_screen.clone(); note(h, "game", &line, game.as_deref()); }
        Some(h.bound.iter().map(|b| b.iter().copied().collect()).collect())
    }

    /// The people on the seats, as a game sees them: a running game takes
    /// in a player who joins (a pad that wakes up) and drops one who
    /// leaves; a paused game does when it resumes.
    fn seats(h: &Host) -> Vec<warp::party::SeatInfo> {
        (0..h.bound.len()).map(|i| {
            let p = &h.party["players"][i];
            let text = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            warp::party::SeatInfo { seat: i, name: p["name"].as_str().map_or_else(|| format!("Player {}", i + 1), String::from), color: warp::party::hex(&text("color")), bot: false, couch: p["couch"].as_i64(), skin: text("skin_color"), avatar: text("avatar") }
        }).collect()
    }

    /// The seat of player `id`.
    pub(crate) fn seat_of(h: &Host, id: &str) -> Option<usize> {
        h.party["players"].as_array()?.iter().position(|p| p["id"] == id)
    }

    /// A new player on a new seat at the end (`phone`: one who plays on
    /// their phone alone); returns the seat.
    pub(crate) fn add_player(h: &mut Host, phone: bool) -> usize {
        let i = h.bound.len();
        let id = format!("p{}", h.next_id);
        h.next_id += 1;
        let mut look = json!({});
        look["id"] = json!(id);
        look["name"] = json!(format!("Player {}", i + 1));
        look["color"] = json!(PLAYER_COLORS[i % PLAYER_COLORS.len()]);
        look["skin_color"] = json!(SKINS[i % SKINS.len()]);
        look["avatar"] = json!(game_room::face::demo(i));
        look["couch"] = json!(i);
        h.own_looks.push(look.clone());
        if let Some(list) = h.party["players"].as_array_mut() { list.push(look); }
        if let Some(seats) = h.party["seats"].as_array_mut() { seats.push(json!({"index": i, "occupant": {"kind": "local", "player_id": id}, "controller": null})); }
        if phone { if let Some(phones) = h.party["phones"].as_array_mut() { phones.push(json!(id)); } }
        h.bound.push(None);
        h.phone.push(phone);
        h.players = h.bound.len();
        h.cloud.add_seat();
        h.dirty = true;
        i
    }

    /// The room took player `id` out (their pad stayed gone, or their phone
    /// left). One of the `?players=` seats stays as a free seat for the next
    /// pad, in its own look again; any other player leaves the table.
    pub(crate) fn leave_party(h: &mut Host, id: &str) {
        let Some(i) = seat_of(h, id) else { return };
        let text = format!("{} left", name(h, i));
        note(h, "left", &text, None);
        if let Some(phones) = h.party["phones"].as_array_mut() { phones.retain(|p| p != id); }
        if i < h.base {
            h.bound[i] = None;
            h.phone[i] = false;
            h.cloud.free_seat(i);
            if let Some(own) = h.own_looks.get(i) { h.party["players"][i] = own.clone(); }
        } else {
            h.bound.remove(i);
            h.phone.remove(i);
            h.own_looks.remove(i);
            if let Some(list) = h.party["players"].as_array_mut() { list.remove(i); }
            if let Some(seats) = h.party["seats"].as_array_mut() {
                seats.remove(i);
                for (k, seat) in seats.iter_mut().enumerate() { seat["index"] = json!(k); }
            }
            h.cloud.remove_seat(i);
        }
        h.players = h.bound.len();
        h.dirty = true;
    }

    /// Tell the page whether the room is on screen with nobody in it: it
    /// shows "press a button on a controller, or scan the code" then
    /// (`<html data-empty>`, see index.html).
    fn empty_room(empty: bool) {
        thread_local! { static SHOWN: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
        if SHOWN.with(|s| s.replace(Some(empty))) == Some(empty) { return; }
        let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
        let _ = if empty { root.set_attribute("data-empty", "") } else { root.remove_attribute("data-empty") };
    }

    /// A key pressed while nobody plays on the keyboard: the page shows
    /// for a moment that Enter joins (`<html data-key-hint>`, see index.html).
    fn key_hint() {
        thread_local! { static LAST: std::cell::Cell<Option<web_time::Instant>> = const { std::cell::Cell::new(None) }; }
        if LAST.with(|l| l.get().is_some_and(|t| t.elapsed().as_secs_f32() < 4.0)) { return; }
        LAST.with(|l| l.set(Some(web_time::Instant::now())));
        app::web::log("keyboard: a key, nobody on it; Enter joins");
        let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
        // Off, a layout, on: the animation runs again.
        let _ = root.remove_attribute("data-key-hint");
        let _ = root.get_bounding_client_rect();
        let _ = root.set_attribute("data-key-hint", "");
    }

    /// The first game this page starts: the page shows for a few seconds how
    /// to get back to the room (`<html data-back-hint>`, see index.html).
    fn back_hint() {
        let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
        if !root.has_attribute("data-back-hint") { let _ = root.set_attribute("data-back-hint", ""); }
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
                    // The first game in the queue these players can play;
                    // the ones before it wait at the back.
                    let first = with(|h| {
                        let players = h.players;
                        let fits: Vec<bool> = h.party["playlist"]["entries"].as_array()?.iter().map(|e| e["game"].as_str().is_some_and(|g| fits(&h.party, g, players))).collect();
                        let at = fits.iter().position(|f| *f);
                        let entries = h.party["playlist"]["entries"].as_array_mut()?;
                        let Some(at) = at else {
                            app::web::log(&format!("nothing in the queue for {players} player(s)"));
                            return None;
                        };
                        let e = entries.remove(at);
                        entries.push(e.clone());
                        h.dirty = true;
                        e["game"].as_str().map(String::from)
                    });
                    if let Some(game) = first { start(game); }
                }
                "close_overlay" => resume(),
                "leave_party" => with(|h| leave_party(h, msg["player_id"].as_str().unwrap_or_default())),
                // Moved on the couch (the room's mouse menu): whoever sat there swaps.
                "set_couch_position" => with(|h| {
                    let Some(i) = msg["player_id"].as_str().and_then(|p| seat_of(h, p)) else { return };
                    let (from, to) = (h.party["players"][i]["couch"].clone(), msg["position"].clone());
                    let Some(list) = h.party["players"].as_array_mut() else { return };
                    for p in list.iter_mut() { if p["couch"] == to { p["couch"] = from.clone(); } }
                    list[i]["couch"] = to;
                    h.dirty = true;
                }),
                "room_pickup" => {
                    let seat = msg["player_id"].as_str().and_then(|p| with(|h| seat_of(h, p)));
                    if let (Some(pending), Some(seat)) = (msg["pending"].as_str(), seat) { crate::cloud::pickup(pending.to_string(), seat); }
                }
                _ => {}
            }
        }
    }

    impl Game for Room {
        fn local_players(&self) -> usize { self.lobby.local_players() }
        fn tick(&mut self, controls: &[Controls]) {
            if let Some(party) = with(|h| std::mem::take(&mut h.dirty).then(|| h.party.clone())) {
                let ids = party["players"].as_array().map(|l| l.iter().filter_map(|p| p["id"].as_str().map(String::from)).collect()).unwrap_or_default();
                self.lobby.set_local(ids);
                self.lobby.set_party(party);
            }
            if let Some(links) = with(|h| std::mem::take(&mut h.links_dirty).then(|| h.links.clone())) { self.lobby.set_links(links); }
            empty_room(with(|h| h.players == 0 && h.on_screen.is_none()));
            if let Some(code) = with(|h| h.key.take()) { self.lobby.key_pressed(code); }
            for (game, png) in with(|h| std::mem::take(&mut h.covers)) { self.lobby.set_cover(&game, &png); }
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
        // A game needs a player: nobody here yet, so the room asks for one.
        if with(|h| h.players == 0) {
            app::web::log(&format!("{game}: nobody to play it yet"));
            return empty_room(true);
        }
        // Not a game for fewer people than it needs (Relay Keepers with one).
        if let Some(need) = with(|h| (!fits(&h.party, &game, h.players)).then(|| min_players(&h.party, &game))) {
            app::web::log(&format!("{game}: needs {need} players; press a button on another controller to join"));
            return;
        }
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
                let text = format!("Started {}", title(&h.party, &game));
                note(h, "game_started", &text, Some(&game));
                empty_room(false);
                back_hint();
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
            empty_room(false);
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
        // After the room is up, loading a game shows on its TV only.
        app::web::quiet_progress();
        wasm_bindgen_futures::spawn_local(async {
            if let Err(e) = run().await { app::web::progress(Some(0.3), &format!("Could not start: {e}")); app::web::log(&e); }
        });
    }

    async fn run() -> Result<(), String> {
        let library: Vec<Value> = serde_json::from_slice(&app::web::fetch("games/library.json").await?).map_err(|e| format!("games/library.json: {e}"))?;
        let players = warp::web::param("players").and_then(|p| p.parse().ok()).unwrap_or(0usize).min(MAX_SEATS);
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
        lobby.remember_view();
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
        with(|h| { h.party = party; h.players = players; h.base = players; h.bound = vec![None; players]; h.phone = vec![false; players]; h.next_id = players; h.links = links; h.own_looks = own_looks; });
        // The catalog's covers, fetched after the room is up (it shows a
        // title card until a cover arrives).
        let covers: Vec<(String, String)> = with(|h| h.party["library"].as_array().into_iter().flatten()
            .filter(|g| g["has_cover"] == true)
            .filter_map(|g| Some((g["id"].as_str()?.to_string(), format!("games/{}/cover.png", g["folder"].as_str()?)))).collect());
        wasm_bindgen_futures::spawn_local(async move {
            for (game, url) in covers {
                match app::web::fetch(&url).await {
                    Ok(png) => with(|h| h.covers.push((game, png))),
                    Err(e) => app::web::log(&format!("{url}: {e}")),
                }
            }
        });
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
