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
//!
//! The same host runs natively in the Android app (lobbies/game-room/android,
//! engine/docs/android-host.md): app::web is the Android shell there, the
//! page's HTML hints are lines over the picture (app::android::banner) and
//! the address's query is the intent's `query` extra.

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
mod builtin;
#[cfg(target_arch = "wasm32")]
mod html_game;
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
mod cloud;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub(crate) mod page {
    use app::{Buttons, Config, Controls, Device, Draw, Frame, Game};
    use game_room::live::Lobby;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    #[cfg(target_arch = "wasm32")]
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
        /// A game picked while nobody had joined: its files are fetched
        /// ahead, and it starts once there are players enough.
        waiting: Option<String>,
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
        pub(crate) cloud: super::cloud::Cloud,
        /// A key pressed since the room last ticked (its buttons).
        key: Option<u32>,
        /// The room log for the cloud room's phones, oldest first (its
        /// `activity`), and the next line's id.
        pub(crate) activity: Vec<Value>,
        activity_id: u64,
        /// Back held in the room: since when (or the last chord), and whether B armed leaving.
        backs: Vec<(Device, web_time::Instant, bool)>,
        /// The last bug report (Back + Y, F4): one per press.
        bug_at: Option<web_time::Instant>,
        /// Pads not playing with their stick pushed, and whether one was
        /// heard since the room last ticked (the table's pads jiggle).
        pushed: Vec<Device>,
        pad_heard: bool,
        /// Pads this page has heard from: a new one in a game gets the join hint.
        seen: Vec<Device>,
        /// Enter held on the keyboard while nobody plays on it.
        kb_enter: bool,
        /// The game whose keyboard hint is due, and when ([`keyboard_game_hint`]).
        kb_hint: Option<(String, web_time::Instant)>,
        /// The keyboard (or keyboard half) that last pressed a key: the one
        /// a new pad takes over when two share the keyboard.
        last_keys: Option<Device>,
        /// A chord to run when the room ticks.
        chord: Option<Chord>,
        /// Games started, oldest first (Back + LB plays the last again).
        played: Vec<String>,
        /// Covers fetched for the room's catalog: (game id, PNG).
        covers: Vec<(String, Vec<u8>)>,
        /// Gameplay screenshots fetched for the room's TV: (game id, PNG).
        shots: Vec<(String, Vec<u8>)>,
        /// Pads connected that play for nobody: on the room's table.
        idle_pads: usize,
    }

    thread_local! { static HOST: RefCell<Host> = RefCell::new(Host::default()); }

    pub(crate) fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R { HOST.with(|h| f(&mut h.borrow_mut())) }

    pub(crate) fn game_meta<'a>(party: &'a Value, game: &str) -> Option<&'a Value> {
        party["library"].as_array()?.iter().find(|g| g["id"] == game || g["folder"] == game)
    }

    fn min_players(party: &Value, game: &str) -> usize {
        game_meta(party, game).and_then(|g| g["min_players"].as_u64()).unwrap_or(1) as usize
    }

    /// Whether `players` people are enough for `game`, with the bots it
    /// brings (Stack Together: one person and its bot).
    pub(crate) fn fits(party: &Value, game: &str, players: usize) -> bool {
        let bots = game_meta(party, game).and_then(|g| g["bots"].as_u64()).unwrap_or(0) as usize;
        players >= 1 && players + bots >= min_players(party, game) && input_fits(party, game)
    }

    /// Whether the input at hand plays `game` (as the room's host.rs
    /// input_fits): with only keyboards seated, the game must take keys,
    /// and two on one keyboard only a game with `shared_keyboard` 2.
    fn input_fits(party: &Value, game: &str) -> bool {
        let controllers: Vec<&str> = party["seats"].as_array().into_iter().flatten()
            .filter(|s| s["occupant"].get("player_id").is_some_and(|v| !v.is_null())).map(|s| s["controller"].as_str().unwrap_or("")).collect();
        if controllers.is_empty() || !controllers.iter().all(|c| c.contains(":keyboard")) { return true; }
        let Some(g) = game_meta(party, game) else { return true };
        let shared = g["shared_keyboard"].as_u64().unwrap_or(1) as usize;
        if controllers.len() >= 2 { shared >= controllers.len() } else { g["keyboard"] == true || g["mouse"] == true || shared >= 2 }
    }

    fn title(party: &Value, game: &str) -> String {
        game_meta(party, game).and_then(|g| g["title"].as_str()).unwrap_or(game).to_string()
    }

    fn set_session(h: &mut Host, game: Option<&str>, phase: &str) {
        warp::web::reset_story_edit();
        #[cfg(target_arch = "wasm32")]
        if let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
            let _ = match game.filter(|_| phase == "running") {
                Some(id) => root.set_attribute("data-playing", id),
                None => root.remove_attribute("data-playing"),
            };
        }
        // One id per session, kept over pause and resume (the cloud's insights count it).
        let id = match &h.party["active_session"] { s if game.is_some_and(|g| s["game"] == g) => s["id"].clone(), _ => json!(super::cloud::uuid()) };
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
    /// Pad `index` pressed A on the loading page (index.html
    /// `window.splitscreenEarlyPads`): true once, then it is taken off.
    fn early_pad(index: u32) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            let Some(window) = web_sys::window() else { return false };
            let Some(list) = js_sys::Reflect::get(&window, &"splitscreenEarlyPads".into()).ok().and_then(|l| l.dyn_into::<js_sys::Array>().ok()) else { return false };
            let at = list.index_of(&wasm_bindgen::JsValue::from(index), 0);
            if at < 0 { return false; }
            let rest = list.slice(0, at as u32).concat(&list.slice(at as u32 + 1, list.length()));
            let _ = js_sys::Reflect::set(&window, &"splitscreenEarlyPads".into(), &rest);
            return true;
        }
        #[allow(unreachable_code)]
        { let _ = index; false }
    }

    fn devices(h: &mut Host, devices: &[(Device, Controls)]) -> Option<Vec<Vec<Device>>> {
        for (dev, c) in devices {
            // Any key on the keyboard: the keyboard on the room's table hops.
            // (WASD and the arrows move: they count as keys too.)
            let key_action = c.pressed.0 != 0 || c.mv != [0.0; 2];
            if dev.is_keys() && key_action {
                h.last_keys = Some(*dev);
                // Any key: the keyboard on the room's table hops, as the
                // controllers there do for a button. Not once two play on
                // it: it takes nobody more, like a joined controller.
                if !app::keyboard_split() { h.key = Some(c.pressed.0 | ((c.mv[0] > 0.0) as u32) << 30 | ((c.mv[1] > 0.0) as u32) << 31); }
            }
            // G while one player has the whole keyboard: a second one joins
            // on it, and the keyboard splits in two halves.
            if *dev == Device::Keyboard && c.pressed.has(Buttons::KEYBOARD_JOIN) && h.bound.contains(&Some(Device::Keyboard)) { share_keyboard(h); continue; }
            // A solo game link already reserved one player. Their first movement,
            // action or captured mouse look can claim that seat in this tick.
            let solo_seat = h.on_screen.is_some() && h.bound.as_slice() == [None] && !h.phone[0];
            if *dev == Device::Keyboard && key_action {
                if !solo_seat && !c.pressed.has(Buttons::START) && !h.bound.contains(&Some(Device::Keyboard)) {
                    if h.on_screen.is_some() { join_hint(true, None) } else { key_hint() }
                }
            }
            // A pad joins with A. In the room or when adding another player,
            // the keyboard joins with Enter, so typing doesn't add a player.
            // The keyboard joins when Enter comes back up, so that Enter
            // does not also open the new player's Start menu.
            let enter = *dev == Device::Keyboard && (c.held.has(Buttons::START) || c.pressed.has(Buttons::START));
            let released = *dev == Device::Keyboard && h.kb_enter && !enter;
            if *dev == Device::Keyboard { h.kb_enter = enter && !h.bound.contains(&Some(Device::Keyboard)); }
            let active = match dev {
                Device::Keyboard => released || (solo_seat && !enter && (key_action || c.look != [0.0; 2])),
                // As at home: A joins; anything else only jiggles the pads on the table.
                // A pad that pressed A on the loading page joins as soon as it shows up.
                Device::Pad(i) => c.pressed.has(Buttons::SOUTH) || (!h.seen.contains(dev) && early_pad(*i)),
                // Halves are only there while both are seated ([`share_keyboard`]).
                Device::KeyHalf(_) => false,
            };
            // A pad first heard during a game (the browser shows a pad only
            // once a button is pressed): how to join, unless it pressed A.
            if let Device::Pad(_) = dev && !h.seen.contains(dev) {
                h.seen.push(*dev);
                if h.on_screen.is_some() && !active && !h.bound.contains(&Some(*dev)) { join_hint(false, keyboard_player(h)); }
            }
            if let Device::Pad(_) = dev && !active && !h.bound.contains(&Some(*dev)) {
                let pushed = c.mv[0].abs().max(c.mv[1].abs()) > 0.6;
                let was = h.pushed.contains(dev);
                if pushed && !was { h.pushed.push(*dev); } else if !pushed { h.pushed.retain(|d| d != dev); }
                if c.pressed.0 != 0 || (pushed && !was) {
                    h.pad_heard = true;
                    // In a game nobody sees the table: say how to join.
                    if h.on_screen.is_some() { join_hint(false, keyboard_player(h)); }
                }
            }
            if !active || h.bound.contains(&Some(*dev)) { continue; }
            // Splitscreen Club is played on controllers: A on a new pad while
            // someone plays on the keyboard hands that player (seat, look,
            // profile) to the pad instead of adding a second player. The
            // keyboard is free again: Enter joins it as someone new.
            // Two on the keyboard: the one who last pressed a key.
            let kb_seat = h.bound.iter().position(|b| b.is_some() && *b == h.last_keys).or_else(|| h.bound.iter().position(|b| b.is_some_and(|d| d.is_keys())));
            if let Device::Pad(_) = dev && let Some(seat) = kb_seat {
                h.bound[seat] = Some(*dev);
                h.kb_enter = false;
                let who = name(h, seat);
                show_hint("✓", &format!("{who} plays on the controller now"), "");
                note(h, "joined", &format!("{who} switched from the keyboard to a controller"), None);
                app::web::log(&format!("{dev} takes over {who} from the keyboard"));
                whole_keyboard(h);
                continue;
            }
            seat_device(h, *dev);
        }
        whole_keyboard(h);
                if h.kb_hint.as_ref().is_some_and(|(_, at)| web_time::Instant::now() >= *at) {
            let (game, _) = h.kb_hint.take().unwrap();
            if h.on_screen.as_deref() == Some(game.as_str()) && let Some(who) = keyboard_player(h) { keyboard_game_hint(h, &game, &who); }
        }
        // In the room, for as long as someone plays on the keyboard.
        controller_nudge(if h.on_screen.is_none() { keyboard_player(h) } else { None }, app::keyboard_split());
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
        h.idle_pads = devices.iter().filter(|(d, _)| matches!(d, Device::Pad(_)) && !h.bound.contains(&Some(*d))).count();
        back_chords(h, devices);
        warp::web::set_seats(seats(h));
        // Games draw each player's own buttons (hud.button): pad or keys.
        app::set_seat_devices(h.bound.clone());
        for line in warp::web::take_room_log() { let game = h.on_screen.clone(); note(h, "game", &line, game.as_deref()); }
        Some(h.bound.iter().map(|b| b.iter().copied().collect()).collect())
    }

    /// `dev` takes the first free seat, or a new player's; false: no seat left.
    fn seat_device(h: &mut Host, dev: Device) -> bool {
        let (seat, new_player) = match (0..h.bound.len()).find(|&i| h.bound[i].is_none() && !h.phone[i]) {
            Some(i) => (i, false),
            None if h.bound.len() < MAX_SEATS => (add_player(h, false), true),
            None => return false,
        };
        h.bound[seat] = Some(dev);
        // A reserved seat already has a player in the running game.
        // Only adding a party seat needs another game player.
        if new_player && h.on_screen.is_some() {
            let info = &seats(h)[seat];
            if app::web::game_takes_joins() { show_hint("✓", &format!("{} joined the game", info.name), ""); } else { next_game_hint(&info.name); }
            app::web::player_joins(&info.name, info.color);
        }
        let text = format!("{} joined with {}", name(h, seat), if dev.is_keys() { "the keyboard" } else { "a controller" });
        note(h, "joined", &text, None);
        app::web::log(&format!("{dev} plays as {}", h.party["players"][seat]["name"].as_str().unwrap_or("?")));
        // Keyboard in a game (a game link, or Play picked with the
        // mouse): the room's banner isn't on screen, so say it once here.
        if dev == Device::Keyboard && let Some(game) = h.on_screen.clone() { h.kb_hint = Some((game, web_time::Instant::now())); }
        true
    }

    /// How many players `game` takes on one keyboard (its catalog entry's
    /// `players.shared_keyboard`; 1 when it doesn't say).
    fn shared_keyboard(party: &Value, game: &str) -> usize {
        game_meta(party, game).and_then(|g| g["shared_keyboard"].as_u64()).unwrap_or(1) as usize
    }

    /// G on the keyboard while one player has it whole: a second player
    /// joins on it. The keyboard splits ([`app::split_keyboard`]): who sits
    /// left on the couch gets the left half (WASD, Space), who sits right
    /// the arrows ([`whole_keyboard`] keeps it so). In a game, only one that
    /// takes two on a keyboard.
    fn share_keyboard(h: &mut Host) {
        if app::keyboard_split() { return; }
        let Some(first) = h.bound.iter().position(|b| *b == Some(Device::Keyboard)) else { return };
        if let Some(game) = h.on_screen.clone() && shared_keyboard(&h.party, &game) < 2 {
            return show_hint("G", &format!("{} takes one player per keyboard", title(&h.party, &game)), "A second player joins with a controller, or on the keyboard in the room");
        }
        h.bound[first] = Some(Device::KeyHalf(1));
        if !seat_device(h, Device::KeyHalf(0)) { h.bound[first] = Some(Device::Keyboard); return; }
        app::split_keyboard(true);
        // Left on the couch: WASD; right: the arrows.
        whole_keyboard(h);
        show_hint("✓", "Two players on the keyboard", "Left on the couch plays on WASD, right on the arrows");
    }

    /// One player left on a split keyboard (the other left, or took a
    /// controller): they get the whole keyboard back, with the mouse. Two:
    /// the halves follow the couch (left WASD, right arrows).
    fn whole_keyboard(h: &mut Host) {
        if !app::keyboard_split() { return; }
        let mut halves: Vec<usize> = (0..h.bound.len()).filter(|&i| matches!(h.bound[i], Some(Device::KeyHalf(_)))).collect();
        if halves.len() >= 2 {
            // Fixed by couch position (Joep): who sits left plays on WASD,
            // who sits right on the arrows, also after moving seats.
            halves.sort_by_key(|&i| (h.party["players"][i]["couch"].as_i64().unwrap_or(i as i64), i));
            for (k, &i) in halves.iter().enumerate() { h.bound[i] = Some(Device::KeyHalf(k.min(1) as u8)); }
            return;
        }
        for i in halves { h.bound[i] = Some(Device::Keyboard); }
        app::split_keyboard(false);
        h.last_keys = None;
    }

    /// Whether `game` runs smoothly on this device (its measured `bench`
    /// in library.json; game_room::host::Host::hardware_note).
    fn smooth_here(party: &Value, game: &str) -> bool {
        let Some(meta) = game_meta(party, game) else { return true };
        let Some(device) = DEVICE.with(|d| d.borrow().clone()) else { return true };
        let mut h = game_room::host::Host::default();
        h.device = Some(device);
        h.runs_smoothly(meta)
    }

    thread_local! {
        /// What the room last drew on, for [`smooth_here`].
        static DEVICE: std::cell::RefCell<Option<game_room::host::HostDevice>> = const { std::cell::RefCell::new(None) };
    }

    /// The first game in the queue these players can play; the ones before
    /// it wait at the back.
    fn next_game() {
        let first = with(|h| {
            // Nobody here yet: pick as for one, so start() fetches it ahead.
            let players = h.players.max(1);
            let fits: Vec<bool> = h.party["playlist"]["entries"].as_array()?.iter().map(|e| e["game"].as_str().is_some_and(|g| fits(&h.party, g, players))).collect();
            // A game measured too heavy for this device only when nothing else fits.
            let smooth: Vec<bool> = h.party["playlist"]["entries"].as_array()?.iter().map(|e| e["game"].as_str().is_none_or(|g| smooth_here(&h.party, g))).collect();
            let at = fits.iter().zip(&smooth).position(|(f, s)| *f && *s).or_else(|| fits.iter().position(|f| *f));
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

    /// The game played before, again (Back + LB): the last one started.
    fn previous_game() {
        if let Some(game) = with(|h| h.played.last().cloned()) { start(game); }
    }

    /// Back held in the room ([`back_chords`]): what it asks for, done
    /// once the room ticks (a game starting needs the host free).
    #[derive(Clone, Copy, PartialEq)]
    enum Chord { Next, Previous, ReportBug }

    /// Holding Back in the room shows, after 0.16 s, what Back plus a
    /// button does (the room draws the party's `back_menu` over the
    /// player): RB the next game, LB the previous one, B twice leaves.
    /// Back tapped does nothing here. In a game Back goes to the room when
    /// it is let go without a chord; Back + RB / LB there go straight on to
    /// the next / previous game ([`app::back_chord`]).
    fn back_chords(h: &mut Host, devices: &[(Device, Controls)]) {
        let now = web_time::Instant::now();
        let mut menu = Value::Null;
        for (dev, c) in devices {
            let Some(seat) = h.bound.iter().position(|b| *b == Some(*dev)) else { continue };
            if h.on_screen.is_some() || !c.held.has(Buttons::SELECT) { h.backs.retain(|(d, ..)| d != dev); continue; }
            let k = match h.backs.iter().position(|(d, ..)| d == dev) {
                Some(k) => k,
                None => { h.backs.push((*dev, now, false)); h.backs.len() - 1 }
            };
            let id = h.party["players"][seat]["id"].as_str().unwrap_or_default().to_string();
            if c.pressed.has(Buttons::RIGHT_SHOULDER) { h.chord = Some(Chord::Next); h.backs[k].1 = now; }
            if c.pressed.has(Buttons::LEFT_SHOULDER) && !h.played.is_empty() { h.chord = Some(Chord::Previous); h.backs[k].1 = now; }
            if c.pressed.has(Buttons::NORTH) { h.chord = Some(Chord::ReportBug); h.backs[k].1 = now; }
            if c.pressed.has(Buttons::EAST) {
                if h.backs[k].2 { h.backs.remove(k); leave_party(h, &id); h.party["back_menu"] = Value::Null; h.dirty = true; return; }
                h.backs[k].2 = true;
            }
            if now.duration_since(h.backs[k].1).as_secs_f32() < 0.16 { continue; }
            let mut chords = vec![json!({"button": "rb", "action": "next", "label": "Play the next game"})];
            if !h.played.is_empty() { chords.push(json!({"button": "lb", "action": "previous", "label": "Play the previous game"})); }
            chords.push(json!({"button": "y", "action": "report_bug", "label": "Report a bug"}));
            chords.push(json!({"button": "b", "action": "leave", "label": "Leave the party"}));
            menu = json!({"player": id, "controller": format!("web:{dev}"), "confirm_leave": h.backs[k].2, "chords": chords});
        }
        if h.party["back_menu"] != menu { h.party["back_menu"] = menu; h.dirty = true; }
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
        #[cfg(target_os = "android")]
        {
            // The browser page's notice (index.html #club-notice), without
            // the keyboard: a phone host has none.
            const EMPTY: &str = "Connect a controller and press A\nYou can also scan the QR code and pick Play on this phone, but a controller is the best way to play";
            if empty { app::web::banner(EMPTY, None) } else { app::web::clear_banner(Some(EMPTY)) }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
            let _ = if empty { root.set_attribute("data-empty", "") } else { root.remove_attribute("data-empty") };
        }
    }

    /// Whether a controller is connected: the page shows A's glyph only
    /// then (`<html data-pad>`, see index.html `.with-pad`).
    fn pad_present(pad: bool) {
        thread_local! { static SHOWN: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
        if SHOWN.with(|s| s.replace(Some(pad))) == Some(pad) { return; }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
            let _ = if pad { root.set_attribute("data-pad", "") } else { root.remove_attribute("data-pad") };
        }
    }

    /// A key pressed while nobody plays on the keyboard: the page shows
    /// for a moment that Enter joins (`<html data-key-hint>`, see index.html).
    fn key_hint() {
        thread_local! { static LAST: std::cell::Cell<Option<web_time::Instant>> = const { std::cell::Cell::new(None) }; }
        if LAST.with(|l| l.get().is_some_and(|t| t.elapsed().as_secs_f32() < 4.0)) { return; }
        LAST.with(|l| l.set(Some(web_time::Instant::now())));
        app::web::log("keyboard: a key, nobody on it; Enter joins");
        #[cfg(target_os = "android")]
        app::web::banner("Press Enter to join", Some(4.0));
        #[cfg(target_arch = "wasm32")]
        {
            let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
            // Off, a layout, on: the animation runs again.
            let _ = root.remove_attribute("data-key-hint");
            let _ = root.get_bounding_client_rect();
            let _ = root.set_attribute("data-key-hint", "");
        }
    }

    /// In a game, a controller (or the keyboard: `keyboard`) nobody plays
    /// with was touched: "Press A to join", or, when the game takes no
    /// one mid-game, that A joins from the next game
    /// (`<html data-join-hint>`, see index.html).
    fn join_hint(keyboard: bool, keyboard_player: Option<String>) {
        thread_local! { static LAST: std::cell::Cell<Option<web_time::Instant>> = const { std::cell::Cell::new(None) }; }
        if LAST.with(|l| l.get().is_some_and(|t| t.elapsed().as_secs_f32() < 4.0)) { return; }
        LAST.with(|l| l.set(Some(web_time::Instant::now())));
        let (key, now) = (if keyboard { "Enter" } else { "A" }, app::web::game_takes_joins());
        // A pad while someone plays on the keyboard: A takes that player over.
        if let Some(who) = keyboard_player.filter(|_| !keyboard) { return show_hint(key, &format!("Play as {who} on this controller"), "Splitscreen Club is made for controllers"); }
        // The glyph says which button (DESIGN.md: glyph, then the verb).
        let text = if now { "Join this game".to_string() } else { "Join: you play from the next game".to_string() };
        show_hint(key, &text, "");
    }

    /// The player on the keyboard, if anyone plays on it: their name.
    fn keyboard_player(h: &Host) -> Option<String> {
        let names: Vec<String> = (0..h.bound.len()).filter(|&i| h.bound[i].is_some_and(|d| d.is_keys())).map(|i| name(h, i)).collect();
        (!names.is_empty()).then(|| names.join(" & "))
    }

    /// The room's banner while `who` plays on the keyboard (and mouse):
    /// Splitscreen Club is made for controllers, connect one and press A;
    /// that pad then takes `who` over ([`devices`]). `<html data-kb-nudge>`,
    /// see index.html `#kb-nudge`. None: no banner (nobody on the keyboard,
    /// or a game on screen).
    fn controller_nudge(who: Option<String>, split: bool) {
        thread_local! { static SHOWN: RefCell<Option<(Option<String>, bool)>> = const { RefCell::new(None) }; }
        if SHOWN.with(|s| s.borrow().as_ref() == Some(&(who.clone(), split))) { return; }
        SHOWN.with(|s| *s.borrow_mut() = Some((who.clone(), split)));
        #[cfg(target_os = "android")]
        {
            const TEXT: &str = "Splitscreen Club is made for controllers\nConnect one and press A to take over from the keyboard";
            if who.is_some() { app::web::banner(TEXT, None) } else { app::web::clear_banner(Some(TEXT)) }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
            if let (Some(e), Some(who)) = (doc.get_element_by_id("kb-nudge-who"), &who) {
                e.set_text_content(Some(&if split { format!("Keyboard · {who}") } else { format!("Keyboard & mouse · {who}") }));
            }
            let Some(root) = doc.document_element() else { return };
            let _ = if split { root.set_attribute("data-kb-split", "") } else { root.remove_attribute("data-kb-split") };
            // Hidden with X: back for other keyboard players.
            let _ = root.remove_attribute("data-kb-nudge-hidden");
            // An old join hint (its 4 s long over) would hold the banner back.
            if who.is_some() { let _ = root.remove_attribute("data-join-hint"); }
            let _ = if who.is_some() { root.set_attribute("data-kb-nudge", "") } else { root.remove_attribute("data-kb-nudge") };
        }
    }

    /// Whether `game` declares keyboard play: its catalog entry's
    /// `players.keyboard` (playable on keys) or `players.mouse` (made for
    /// keyboard and mouse); gamenight.json, copied into library.json by
    /// build-web.py. Neither: made for controllers.
    fn plays_on_keyboard(party: &Value, game: &str) -> bool {
        game_meta(party, game).is_some_and(|g| g["keyboard"] == true || g["mouse"] == true)
    }

    /// `who` plays on the keyboard in `game` (it started so, or the
    /// keyboard took a seat in it): a game made for controllers says so,
    /// with how to take over on one. A game that declares keyboard play
    /// says nothing.
    fn keyboard_game_hint(h: &Host, game: &str, who: &str) {
        if plays_on_keyboard(&h.party, game) { return; }
        show_hint("A", &format!("{} is made for controllers", title(&h.party, game)), &format!("Connect one and press A to play as {who} on it"));
    }

    /// Someone joined a game that takes no one mid-game.
    fn next_game_hint(name: &str) { show_hint("✓", &format!("{name} plays from the next game"), ""); }

    /// The top hint: `key` as its glyph (A: the green disc), `text`, and a
    /// muted `detail` line under it ("": none).
    fn show_hint(key: &str, text: &str, detail: &str) {
        app::web::log(&format!("hint: {text}"));
        #[cfg(target_os = "android")]
        { app::web::banner_glyph(key, &if detail.is_empty() { text.to_string() } else { format!("{text}\n{detail}") }, 4.0); }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
            if let Some(e) = doc.get_element_by_id("join-key") {
                e.set_text_content(Some(key));
                let _ = match key { "A" => e.set_attribute("class", "pad-a"), "✓" => e.set_attribute("class", "tick"), _ => e.remove_attribute("class") };
            }
            if let Some(e) = doc.get_element_by_id("join-text") { e.set_text_content(Some(text)); }
            if let Some(e) = doc.get_element_by_id("join-detail") { e.set_text_content(Some(detail)); }
            let Some(root) = doc.document_element() else { return };
            let _ = root.remove_attribute("data-join-hint");
            let _ = root.get_bounding_client_rect();
            let _ = root.set_attribute("data-join-hint", "");
        }
    }

    /// The first game this page starts: the page shows for a few seconds how
    /// to get back to the room (`<html data-back-hint>`, see index.html).
    fn back_hint() {
        #[cfg(target_os = "android")]
        {
            thread_local! { static SHOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
            if !SHOWN.with(|s| s.replace(true)) { app::web::banner("Back on a controller brings you back to the room", Some(5.0)); }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) else { return };
            if !root.has_attribute("data-back-hint") { let _ = root.set_attribute("data-back-hint", ""); }
        }
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
                "next" => next_game(),
                "previous" => previous_game(),
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
                    if let (Some(pending), Some(seat)) = (msg["pending"].as_str(), seat) { super::cloud::pickup(pending.to_string(), seat); }
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
            // A game picked before anyone joined starts once it can.
            if let Some(game) = with(|h| (h.players > 0 && h.on_screen.is_none() && h.loading.is_none()).then(|| h.waiting.clone()).flatten().filter(|g| fits(&h.party, g, h.players))) {
                with(|h| h.waiting = None);
                start(game);
            }
            if let Some(code) = with(|h| h.key.take()) { self.lobby.key_pressed(code); }
            if with(|h| std::mem::take(&mut h.pad_heard)) { self.lobby.pad_heard(); }
            self.lobby.set_idle_controllers(with(|h| h.idle_pads));
            // The table's join prompt: A only while a controller is there;
            // without one, Enter, then G for a second player on the keyboard.
            let pad = with(|h| h.idle_pads > 0 || h.bound.iter().any(|d| d.is_some_and(|d| d.is_pad())));
            pad_present(pad);
            self.lobby.join_with = with(|h| {
                if pad { game_room::live::JoinWith::Pad }
                else if h.bound.contains(&Some(Device::Keyboard)) { game_room::live::JoinWith::Key("G") }
                else if h.bound.iter().any(|d| d.is_some_and(|d| d.is_keys())) { game_room::live::JoinWith::Nobody }
                else { game_room::live::JoinWith::Key("ENTER") }
            });
            for (game, png) in with(|h| std::mem::take(&mut h.covers)) { self.lobby.set_cover(&game, &png); }
            for (game, png) in with(|h| std::mem::take(&mut h.shots)) { self.lobby.set_screenshot(&game, &png); }
            fetch_ahead();
            if app::web::take_report_key() { with(|h| h.chord = Some(Chord::ReportBug)); }
            match with(|h| h.chord.take()) {
                Some(Chord::Next) => next_game(),
                Some(Chord::Previous) => previous_game(),
                Some(Chord::ReportBug) => report_room_bug(),
                None => {}
            }
            self.lobby.tick(controls);
            for msg in self.lobby.outbox.as_mut().map(std::mem::take).unwrap_or_default() { self.command(&msg); }
        }
        fn pointer(&mut self, pointer: app::Pointer, size: [u32; 2]) { self.lobby.pointer(pointer, size); }
        // The cursor points and clicks in the room: never captured.
        fn mouse_capture(&self) -> Option<bool> { Some(false) }
        fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
            // A direct link is already loading its game. Upload the lobby the
            // first time someone visits it, including after a failed game load.
            if frame.renderer().is_none() && with(|h| h.loading.is_some()) {
                return Ok(Draw { cameras: Vec::new(), light: Default::default(), hud: render::hud::HudDrawList::new(frame.size) });
            }
            // The screen and GPU, for the catalog's "May stutter here".
            let device = game_room::host::HostDevice { gpu: frame.gpu.info.name.clone(), size: frame.size, phone: app::web::mobile() };
            if self.lobby.host.device.as_ref() != Some(&device) { DEVICE.with(|d| *d.borrow_mut() = Some(device.clone())); self.lobby.host.device = Some(device); }
            self.lobby.draw(frame)
        }
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
        // Started with nobody playing (the mouse on Play): whoever started it
        // plays. One seat waits for them, claimed by the first key or mouse
        // look in the game, or A on a pad, as a game link's seat is. A phone
        // or tablet has no keyboard: there the game waits for A.
        if !app::web::mobile() && with(|h| h.players == 0 && h.on_screen.is_none() && h.loading.is_none()) {
            app::web::log(&format!("{game}: started with nobody playing; the first key, mouse or A plays"));
            with(|h| { add_player(h, false); });
        }
        if with(|h| h.players == 0) {
            app::web::log(&format!("{game}: starts when someone joins; fetching it now"));
            prefetch(game);
            return empty_room(true);
        }
        // Not a game for fewer people than it needs (Relay Keepers with one).
        if let Some(need) = with(|h| (!fits(&h.party, &game, h.players)).then(|| min_players(&h.party, &game))) {
            app::web::log(&format!("{game}: needs {need} players; press a button on another controller to join"));
            return;
        }
        let found = with(|h| {
            let meta = game_meta(&h.party, &game)?;
            let entry = meta["web_entry"].as_str().map(String::from);
            let (id, folder) = (meta["id"].as_str()?.to_string(), meta["folder"].as_str()?.to_string());
            if h.loading.is_some() { return None; }
            h.loading = Some(id.clone());
            h.waiting = None;
            // Play next: the paused game stops now, not once the new one is
            // ready (its voices end too, before the new one starts its own).
            if h.paused.take().is_some() { warp::web::hush(None); }
            h.party["warm_session"] = json!({"game": id, "phase": "preparing", "progress": 50, "progress_label": "Loading"});
            h.dirty = true;
            h.cloud.refresh = true;
            Some((id, folder, h.players, entry))
        });
        let Some((game, folder, players, entry)) = found else { app::web::log(&format!("{game}: not on this page, or a game is loading")); return };
        app::web::progress(Some(0.05), &format!("Loading {}", with(|h| title(&h.party, &game))));
        app::web::spawn(async move {
            let t0 = web_time::Instant::now();
            #[cfg(target_arch = "wasm32")]
            let html = entry.map(|entry| super::html_game::HtmlGame::load(entry, with(|h| title(&h.party, &game)), players));
            #[cfg(not(target_arch = "wasm32"))]
            let html = { let _ = entry; None };
            let loaded = if let Some(html) = html { Ok(html) } else { match super::builtin::load(&folder, players).await {
                Some(l) => l,
                None => warp::web::load(&folder, Some(players)).await.map(|(c, g)| (c, Box::new(g) as Box<dyn Game>)),
            } };
            with(|h| {
                h.loading = None;
                h.party["warm_session"] = Value::Null;
                h.dirty = true;
                let (cfg, game_box) = match loaded {
                    Ok(l) => l,
                    Err(e) => { app::web::log(&format!("{folder}: {e}")); app::web::progress(None, "Could not load"); return; }
                };
                app::web::log(&format!("{folder}: ready {:.0} ms after Play", t0.elapsed().as_secs_f32() * 1000.0));
                h.paused = None;
                // From the room: keep it. From another game (the page's
                // buttons): that game goes, the room is already kept.
                if h.on_screen.is_none() { app::web::switch_keeping(cfg, game_box, |p| with(|h| h.room = Some(p))); } else { app::web::switch(cfg, game_box); }
                h.on_screen = Some(game.clone()); app::web::set_in_game(true);
                set_session(h, Some(&game), "running");
                h.played.retain(|g| *g != game);
                h.played.push(game.clone());
                let text = format!("Started {}", title(&h.party, &game));
                note(h, "game_started", &text, Some(&game));
                empty_room(false);
                back_hint();
                // After the Back hint (the first game's 7 s) in the same place.
                h.kb_hint = Some((game.clone(), web_time::Instant::now() + std::time::Duration::from_secs_f32(if h.played.len() == 1 { 7.5 } else { 1.0 })));
            });
        });
    }

    /// Picked with nobody here: fetch the game's files ahead (the TV shows
    /// the bar), and start it once someone joins ([`Room::tick`]).
    fn prefetch(game: String) {
        let found = with(|h| {
            let meta = game_meta(&h.party, &game)?;
            let id = meta["id"].as_str()?.to_string();
            let folder = meta["folder"].as_str()?.to_string();
            let html = meta["web_entry"].is_string();
            if h.waiting.as_deref() == Some(id.as_str()) { return None; }
            h.waiting = Some(id.clone());
            Some((id, folder, html))
        });
        let Some((id, folder, html)) = found else { return };
        app::web::clear_prefetched();
        let name = with(|h| title(&h.party, &id));
        let status = move |progress: u32, label: &str| with(|h| if h.waiting.as_deref() == Some(id.as_str()) && h.loading.is_none() {
            h.party["warm_session"] = json!({"game": id, "phase": if progress >= 100 { "ready" } else { "preparing" }, "progress": progress, "progress_label": label});
            h.dirty = true;
            // The page's card too: the bar while it loads, then out of the
            // way (the room says to press A).
            if progress >= 100 { app::web::progress(None, label) } else { app::web::progress(Some(progress as f32 / 100.0), &format!("{name}: {label}")) }
        });
        if html { return status(100, "Press A to join"); }
        status(0, "Loading");
        app::web::spawn(async move {
            let list = match app::web::fetch(&format!("games/{folder}/files.json")).await.and_then(|b| serde_json::from_slice::<Vec<String>>(&b).map_err(|e| e.to_string())) {
                Ok(l) => l,
                Err(e) => { app::web::log(&format!("{folder}: {e}")); return status(100, "Press A to join"); }
            };
            for (i, rel) in list.iter().enumerate() {
                // Picked something else meanwhile: stop.
                if with(|h| h.waiting.as_deref() != Some(folder_id(&h.party, &folder).as_str())) { return; }
                if let Err(e) = app::web::prefetch(&format!("games/{folder}/{rel}")).await { app::web::log(&e); }
                status((i + 1) as u32 * 99 / list.len().max(1) as u32, "Loading · press A to join");
            }
            status(100, "Press A to join");
        });
    }

    /// The native host on a phone downloads a game's files the first time
    /// it needs them (app::android keeps them on the phone): this fetches
    /// the next game the playlist would start for these players while the
    /// room or the current game runs, so starting it reads from the phone.
    /// Only files, no GPU work; one game at a time, checked every 2 s.
    /// The browser holds the files in memory (app::web::prefetch) for the
    /// one game up next, so starting it only builds it.
    fn fetch_ahead() {
        #[cfg(any(target_os = "android", target_arch = "wasm32"))]
        {
            thread_local! {
                static CHECKED: std::cell::Cell<Option<web_time::Instant>> = const { std::cell::Cell::new(None) };
                static FETCHED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
                static BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
            }
            if BUSY.get() || CHECKED.get().is_some_and(|t| t.elapsed().as_secs_f32() < 2.0) { return; }
            // A game picked before anyone joined is fetching its own files.
            if with(|h| h.waiting.is_some() || h.loading.is_some()) { return; }
            CHECKED.set(Some(web_time::Instant::now()));
            let next = with(|h| {
                let players = h.players.max(1);
                h.party["playlist"]["entries"].as_array()?.iter().filter_map(|e| e["game"].as_str())
                    .filter(|g| h.on_screen.as_deref() != Some(*g) && fits(&h.party, g, players))
                    .find_map(|g| {
                        let meta = game_meta(&h.party, g)?;
                        if meta["web_entry"].is_string() { return None; }
                        let folder = meta["folder"].as_str()?.to_string();
                        (!FETCHED.with(|f| f.borrow().contains(&folder))).then_some(folder)
                    })
            });
            // Only the first game up; once it is here, the next one waits
            // until it becomes first.
            let Some(folder) = next else { return };
            let first = with(|h| {
                let players = h.players.max(1);
                h.party["playlist"]["entries"].as_array().and_then(|l| l.iter().filter_map(|e| e["game"].as_str())
                    .find(|g| h.on_screen.as_deref() != Some(*g) && fits(&h.party, g, players))
                    .and_then(|g| game_meta(&h.party, g)?["folder"].as_str().map(String::from)))
            });
            if first.as_deref() != Some(folder.as_str()) { return; }
            BUSY.set(true);
            // In memory in a browser: only the one game up next.
            #[cfg(target_arch = "wasm32")]
            { app::web::clear_prefetched(); FETCHED.with(|f| f.borrow_mut().clear()); }
            app::web::spawn(async move {
                let started = web_time::Instant::now();
                let mut bytes = 0;
                match app::web::fetch(&format!("games/{folder}/files.json")).await.and_then(|b| serde_json::from_slice::<Vec<String>>(&b).map_err(|e| e.to_string())) {
                    Ok(list) => for rel in &list {
                        match app::web::prefetch(&format!("games/{folder}/{rel}")).await { Ok(n) => bytes += n, Err(e) => app::web::log(&e) }
                    },
                    Err(e) => app::web::log(&format!("{folder}: {e}")),
                }
                app::web::log(&format!("{folder}: fetched ahead, {:.1} MB in {:.1} s", bytes as f64 / 1e6, started.elapsed().as_secs_f32()));
                FETCHED.with(|f| f.borrow_mut().push(folder));
                BUSY.set(false);
            });
        }
    }

    /// The store id of the game in `folder`.
    fn folder_id(party: &Value, folder: &str) -> String {
        party["library"].as_array().into_iter().flatten().find(|g| g["folder"] == folder).and_then(|g| g["id"].as_str()).unwrap_or(folder).to_string()
    }

    /// TV Resume: the paused game comes back where it was.
    fn resume() {
        with(|h| {
            let Some((game, paused)) = h.paused.take() else { return };
            #[cfg(target_arch = "wasm32")]
            if let Some(session) = h.party["active_session"]["id"].as_str() {
                super::html_game::show_again(session);
            }
            app::web::resume(paused, Some(Box::new(|p| with(|h| h.room = Some(p)))));
            warp::web::hush(Some(false));
            h.on_screen = Some(game.clone()); app::web::set_in_game(true);
            set_session(h, Some(&game), "running");
            empty_room(false);
        });
    }

    /// One bug report per 10 seconds (a held chord, a key repeat).
    fn bug_allowed(h: &mut Host) -> bool {
        let now = web_time::Instant::now();
        if h.bug_at.is_some_and(|at| now.duration_since(at).as_secs() < 10) { return false; }
        h.bug_at = Some(now);
        true
    }

    /// Tell the table a bug was reported: the room log, and a hint on screen.
    fn bug_noted(h: &mut Host, game: Option<&str>) {
        let text = match game {
            Some(g) => format!("Bug reported in {}. Say what happened on your phone.", title(&h.party, g)),
            None => "Bug reported. Say what happened on your phone.".to_string(),
        };
        note(h, "game", &text, game);
        show_hint("✓", &text, "");
    }

    /// Back + Y or F4 in the room: report a bug with the room on screen.
    fn report_room_bug() {
        if !with(|h| { let ok = bug_allowed(h); if ok { bug_noted(h, None); } ok }) { return; }
        app::web::capture_frame(Box::new(|size, rgba| {
            let img = image::RgbaImage::from_raw(size[0], size[1], rgba);
            super::cloud::report_bug(None, None, img.as_ref());
        }));
    }

    /// Back or Quit in a game: the room comes back; Back keeps the game paused.
    fn leave(why: app::web::Exit) -> bool {
        with(|h| {
            if h.on_screen.is_none() { return false; }
            let Some(room) = h.room.take() else { return false };
            let game = h.on_screen.take().unwrap_or_default();
            app::web::set_in_game(false);
            if why != app::web::Exit::Quit {
                // Back + RB / LB in the game: straight on to another game
                // (the room plays the chord on its next tick).
                h.chord = match why {
                    app::web::Exit::Next => Some(Chord::Next),
                    app::web::Exit::Previous if !h.played.is_empty() => Some(Chord::Previous),
                    _ => None,
                };
            }
            let keep = why != app::web::Exit::Quit;
            #[cfg(target_arch = "wasm32")]
            let keep = super::html_game::pause() && keep;
            // Back + Y or F4 in the game: report a bug with its last frame.
            let bug = why == app::web::Exit::ReportBug && bug_allowed(h);
            if bug { bug_noted(h, Some(&game)); }
            let html = game_meta(&h.party, &game).is_some_and(|m| m["web_entry"].is_string());
            if bug && (html || !keep) {
                super::cloud::report_bug(Some(game.clone()), Some(title(&h.party, &game)), None);
            }
            if keep {
                // Its last frame goes on the TV while it waits (a Warp game;
                // an HTML game draws outside the engine's canvas).
                if !html {
                    let id = game.clone();
                    let name = title(&h.party, &game);
                    app::web::capture_frame(Box::new(move |size, rgba| {
                        let Some(img) = image::RgbaImage::from_raw(size[0], size[1], rgba) else { return };
                        if bug { super::cloud::report_bug(Some(id.clone()), Some(name), Some(&img)); }
                        let small = image::imageops::thumbnail(&img, 640, (640 * size[1] / size[0].max(1)).max(1));
                        let mut png = Vec::new();
                        if small.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
                            with(|h| h.shots.push((id, png)));
                        }
                    }));
                }
                let name = game.clone();
                app::web::resume(room, Some(Box::new(move |p| with(|h| h.paused = Some((name, p))))));
                // Its loops and music wait silently with it (Joep: the sound went on).
                warp::web::hush(Some(true));
                set_session(h, Some(&game), "paused");
            } else {
                app::web::resume(room, None);
                warp::web::hush(None);
                set_session(h, None, "");
            }
            true
        })
    }

    pub fn main() {
        #[cfg(target_arch = "wasm32")]
        script::web::init();
        app::web::install_panic_hook();
        // Loading a game shows the page's small card at the bottom (and the
        // TV): a pick from the catalog covers the TV, so the TV alone was
        // no feedback (2026-11-24).
        app::web::spawn(async {
            if let Err(e) = run().await { app::web::progress(Some(0.3), &format!("Could not start: {e}")); app::web::log(&e); }
        });
    }

    async fn run() -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        let bytes = super::html_game::catalog_json().await.map_err(|e| format!("Game catalog: {e:?}"))?
            .as_string().ok_or("Game catalog was not JSON")?.into_bytes();
        #[cfg(not(target_arch = "wasm32"))]
        let bytes = app::web::fetch("games/library.json").await?;
        let library: Vec<Value> = serde_json::from_slice(&bytes).map_err(|e| format!("games/library.json: {e}"))?;
        // The native phone host runs Warp games only: a web export (Godot's,
        // `web_entry`) needs a browser page, so it stays off the shelf there
        // instead of silently dropping out of the queue when picked.
        #[cfg(target_os = "android")]
        let library: Vec<Value> = library.into_iter().filter(|g| !g["web_entry"].is_string()).collect();
        // A game link (`?game=`) seats one player unless `?players=` says
        // otherwise: the first key, mouse look or A on a pad claims it.
        let players = warp::web::param("players").and_then(|p| p.parse().ok())
            .unwrap_or(if warp::web::param("game").is_some() { 1 } else { 0usize }).min(MAX_SEATS);
        // The room's QR code on the loading screen while the room is built.
        super::cloud::register_early();
        app::web::progress(Some(0.2), "Building the Game Room");
        let t0 = web_time::Instant::now();
        let mut options = game_room::stage::Options::default();
        // The room's cheap tier wherever the shell's phone preset is on: a
        // phone or tablet browser, `?quality=low` or `?quality=mobile`
        // (`?quality=full` turns both off).
        options.low = app::web::mobile();
        // The light baked ahead (game-room bake-light): a page bakes on one
        // thread, which takes most of a minute. Without it, fewer rays.
        match app::web::fetch("room-light.bin").await {
            Ok(bytes) => options.light_cache = Some(bytes),
            Err(e) => { app::web::log(&format!("room-light.bin: {e}; baking the light here")); options.rays = 16; }
        }
        let mut lobby = Lobby::new(options, players, 0)?;
        lobby.outbox = Some(vec![]);
        lobby.host.no_jukebox = true;
        lobby.remember_view();
        app::web::log(&format!("Game Room built in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0));
        // The demo party, with this page's games, no phones and nothing loading.
        let mut party = lobby.host.party.clone();
        let library: Vec<Value> = library.into_iter().enumerate().map(|(i, mut g)| { if g["color"].is_null() { g["color"] = json!(COLORS[i % COLORS.len()]); } g }).collect();
        // The queue starts in a fresh random order each time (Joep: not the
        // same first game every evening).
        let mut entries: Vec<Value> = library.iter().map(|g| json!({"game": g["id"], "title": g["title"]})).collect();
        let mut seed = getrandom::u64().unwrap_or(0x9e37_79b9_7f4a_7c15);
        for i in (1..entries.len()).rev() {
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            entries.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        party["playlist"] = json!({"entries": entries, "current": null});
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
        with(|h| {
            h.party = party; h.players = players; h.base = players; h.bound = vec![None; players];
            h.phone = vec![false; players]; h.next_id = players; h.links = links; h.own_looks = own_looks;
            // A direct game load can finish before the room's first device tick.
            warp::web::set_seats(seats(h));
        });
        // The catalog's covers, fetched after the room is up (it shows a
        // title card until a cover arrives).
        let covers: Vec<(String, String)> = with(|h| h.party["library"].as_array().into_iter().flatten()
            .filter(|g| g["has_cover"] == true)
            .filter_map(|g| {
                let folder = g["folder"].as_str()?;
                let url = g["web_cover"].as_str().map(String::from).unwrap_or_else(|| format!("games/{folder}/cover.png"));
                Some((g["id"].as_str()?.to_string(), url))
            }).collect());
        // The TV's gameplay screenshots (build-web.py copies the store's).
        let shots: Vec<(String, String)> = with(|h| h.party["library"].as_array().into_iter().flatten()
            .filter(|g| g["has_screenshot"] == true)
            .filter_map(|g| Some((g["id"].as_str()?.to_string(), format!("games/{}/screenshot.png", g["folder"].as_str()?)))).collect());
        app::web::spawn(async move {
            for (game, url) in covers {
                match app::web::fetch(&url).await {
                    Ok(png) => with(|h| h.covers.push((game, png))),
                    Err(e) => app::web::log(&format!("{url}: {e}")),
                }
            }
            for (game, url) in shots {
                match app::web::fetch(&url).await {
                    Ok(png) => with(|h| h.shots.push((game, png))),
                    Err(e) => app::web::log(&format!("{url}: {e}")),
                }
            }
        });
        app::web::on_exit(leave);
        app::web::on_devices(|d| with(|h| devices(h, d)));
        app::web::run(room_config(), Box::new(Room { lobby }));
        super::cloud::start(players);
        if let Some(game) = warp::web::param("game") { start(game); }
        Ok(())
    }

    /// Start game `name` (folder or store id) from the page (its buttons), as the shelf's Play does.
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn switch_game(name: String) { start(name); }

    /// Author code runs in a disposable Web Worker with its own VM and memory.
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen]
    pub fn evaluate_story(source: &str) -> Result<String, wasm_bindgen::JsValue> {
        if web_sys::window().is_some() { return Err("Story scripts run in the editor worker".into()); }
        warp::story::evaluate(source).map(|value| value.to_string()).map_err(Into::into)
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen]
    pub fn start_story(game: &str, data: &str) -> Result<(), wasm_bindgen::JsValue> {
        if !with(|h| h.on_screen.as_deref() == Some(game) && h.loading.is_none()) {
            return Err("Open the game before starting its story".into());
        }
        warp::web::queue_story(data).map_err(Into::into)
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen]
    pub fn story_reply() -> Option<String> { warp::web::story_reply() }

    /// Claim waiting profile `pending` (its id) for seat `seat`, as a player
    /// at a door does (for tests, and pages with their own controls).
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn pick_up(pending: String, seat: usize) { super::cloud::pickup(pending, seat); }

    /// Back to the room, as Back on a pad does.
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn back_to_room() { leave(app::web::Exit::Back); }
}

#[cfg(target_arch = "wasm32")]
fn main() { if web_sys::window().is_some() { page::main(); } }

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn main() { eprintln!("browser is the Game Room's browser build: engine/tools/build-web.py --warp (engine/docs/browser.md)"); }
