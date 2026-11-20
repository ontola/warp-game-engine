//! Splitscreen Club in a browser: the Game Room first, then the Luau games
//! on its shelf, all on one engine download (`engine/tools/build-web.py
//! --warp`). A small host lives in the page: the room's commands (Play,
//! queue, skip, resume) go to it instead of a Splitscreen Club host, it
//! loads the game with warp::web and switches to it, and Back (or the
//! game's own Quit) brings the room back without reloading it. Back pauses
//! the game: the TV offers Resume, as at home.
//!
//! `?players=N` local players (default 2: keyboard, then pads in the order
//! they wake up), `?game=NAME` starts in that game instead of the room.
//! See engine/docs/browser.md.

#[cfg(target_arch = "wasm32")]
mod page {
    use app::{Config, Controls, Draw, Frame, Game};
    use game_room::live::Lobby;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::wasm_bindgen;

    /// Shelf colours for the games, in library order.
    const COLORS: [&str; 6] = ["#38d6ff", "#3ddc97", "#ffb454", "#ff4f8b", "#e8c14a", "#a78bfa"];

    /// The page's host: the party the room shows, and the screens not on screen.
    #[derive(Default)]
    struct Host {
        party: Value,
        /// The party changed since the room last saw it.
        dirty: bool,
        /// The room, while a game has the screen.
        room: Option<(Config, Box<dyn Game>)>,
        /// A game Back paused, while the room has the screen.
        paused: Option<(String, Config, Box<dyn Game>)>,
        /// The game on screen; None: the room.
        on_screen: Option<String>,
        /// A game being fetched and loaded.
        loading: Option<String>,
        players: usize,
    }

    thread_local! { static HOST: RefCell<Host> = RefCell::new(Host::default()); }

    fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R { HOST.with(|h| f(&mut h.borrow_mut())) }

    fn title(party: &Value, game: &str) -> String {
        party["library"].as_array().and_then(|l| l.iter().find(|g| g["id"] == game)).and_then(|g| g["title"].as_str()).unwrap_or(game).to_string()
    }

    fn set_session(h: &mut Host, game: Option<&str>, phase: &str) {
        h.party["active_session"] = match game { Some(g) => json!({"game": g, "phase": phase}), None => Value::Null };
        h.dirty = true;
    }

    /// The room on screen: its lobby, and the page's host answering it.
    struct Room { lobby: Lobby }

    impl Room {
        fn command(&mut self, msg: &Value) {
            let game = msg["game"].as_str().unwrap_or_default().to_string();
            match msg["type"].as_str().unwrap_or_default() {
                "queue_next" | "queue_game" => with(|h| {
                    let entry = json!({"game": game, "title": title(&h.party, &game)});
                    let Some(entries) = h.party["playlist"]["entries"].as_array_mut() else { return };
                    entries.retain(|e| e["game"] != game.as_str());
                    if msg["type"] == "queue_next" || msg["first"] == true { entries.insert(0, entry) } else { entries.push(entry) }
                    h.dirty = true;
                }),
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
                _ => {}
            }
        }
    }

    impl Game for Room {
        fn local_players(&self) -> usize { self.lobby.local_players() }
        fn tick(&mut self, controls: &[Controls]) {
            if let Some(party) = with(|h| std::mem::take(&mut h.dirty).then(|| h.party.clone())) { self.lobby.set_party(party); }
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

    /// Load `game` (fetching its files the first time) and switch to it;
    /// the room waits behind it, a paused game is dropped.
    fn start(game: String) {
        let players = with(|h| {
            if h.loading.is_some() { return None; }
            h.loading = Some(game.clone());
            h.party["warm_session"] = json!({"game": game, "phase": "preparing", "progress": 50, "progress_label": "Loading"});
            h.dirty = true;
            Some(h.players)
        });
        let Some(players) = players else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let t0 = web_time::Instant::now();
            let loaded = warp::web::load(&game, Some(players)).await;
            with(|h| {
                h.loading = None;
                h.party["warm_session"] = Value::Null;
                h.dirty = true;
                let (cfg, lua) = match loaded {
                    Ok(l) => l,
                    Err(e) => { app::web::log(&format!("{game}: {e}")); return; }
                };
                app::web::log(&format!("{game}: ready {:.0} ms after Play", t0.elapsed().as_secs_f32() * 1000.0));
                h.paused = None;
                // From the room: keep it. From another game (the page's
                // buttons): that game goes, the room is already kept.
                if h.on_screen.is_none() { app::web::switch_keeping(cfg, Box::new(lua), |c, g| with(|h| h.room = Some((c, g)))); } else { app::web::switch(cfg, Box::new(lua)); }
                h.on_screen = Some(game.clone());
                set_session(h, Some(&game), "playing");
            });
        });
    }

    /// TV Resume: the paused game comes back where it was.
    fn resume() {
        with(|h| {
            let Some((game, cfg, paused)) = h.paused.take() else { return };
            app::web::switch_keeping(cfg, paused, |c, g| with(|h| h.room = Some((c, g))));
            h.on_screen = Some(game.clone());
            set_session(h, Some(&game), "playing");
        });
    }

    /// Back or Quit in a game: the room comes back; Back keeps the game paused.
    fn leave(why: app::web::Exit) -> bool {
        with(|h| {
            if h.on_screen.is_none() { return false; }
            let Some((cfg, room)) = h.room.take() else { return false };
            let game = h.on_screen.take().unwrap_or_default();
            if why == app::web::Exit::Back {
                let name = game.clone();
                app::web::switch_keeping(cfg, room, move |c, g| with(|h| h.paused = Some((name, c, g))));
                set_session(h, Some(&game), "paused");
            } else {
                app::web::switch(cfg, room);
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
        lobby.set_party(party.clone());
        // Nobody waits at the doors: phones join a real host, not this page.
        lobby.set_links(json!({"room": {"pending": []}, "linked": {}}));
        with(|h| { h.party = party; h.players = players; });
        app::web::on_exit(leave);
        app::web::run(room_config(), Box::new(Room { lobby }));
        if let Some(game) = warp::web::param("game") { start(game); }
        Ok(())
    }

    /// Start game `name` from the page (its buttons), as the shelf's Play does.
    #[wasm_bindgen]
    pub fn switch_game(name: String) { start(name); }

    /// Back to the room, as Back on a pad does.
    #[wasm_bindgen]
    pub fn back_to_room() { leave(app::web::Exit::Back); }
}

#[cfg(target_arch = "wasm32")]
fn main() { page::main(); }

#[cfg(not(target_arch = "wasm32"))]
fn main() { eprintln!("browser is the Game Room's browser build: engine/tools/build-web.py --warp (engine/docs/browser.md)"); }
