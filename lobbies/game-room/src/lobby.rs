//! The lobby's logic (main.gd): the party snapshot onto the platformer
//! world, controller frames into player input, the stations, the Start
//! menu, toasts, and the host commands they send. Drawing stays in live.rs
//! and hud.rs.

use crate::godot::Color;
use crate::host::{get, s, Station};
use crate::live::{Lobby, Profile};
use game_room_sim::world::{self, BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_LB, BTN_RB, BTN_RIGHT, BTN_RT, BTN_START, BTN_UP, BTN_X, BTN_Y};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const AMBER: Color = crate::room::AMBER;
pub const VIOLET: Color = crate::room::VIOLET;
pub const GREEN: Color = crate::room::GREEN;
pub const PINK: Color = crate::room::PINK;
pub const TEXT: Color = crate::room::TEXT;
pub const DIM: Color = crate::godot::rgb(0.603_921_6, 0.596_078_46, 0.678_431_4);

/// A player without a controller leaves after this long.
pub const LOST_SECONDS: f32 = 30.0;
/// How long Y is held at a door to swap profiles.
pub const HOLD_SECONDS: f32 = 0.8;

/// One player's open Start menu.
#[derive(Clone, Debug, Default)]
pub struct Menu { pub index: usize, pub confirm: bool, pub page: String, stick: bool, stick_x: bool }

#[derive(Clone, Debug)]
pub struct MenuItem { pub label: String, pub action: String, pub player: String, pub name: String, pub lobby: String, pub title: String, pub current: bool }

impl MenuItem {
    fn new(label: &str, action: &str) -> Self {
        Self { label: label.into(), action: action.into(), player: String::new(), name: String::new(), lobby: String::new(), title: String::new(), current: false }
    }
}

#[derive(Clone, Debug)]
pub struct Toast { pub text: String, pub color: Color, pub until: f32 }

/// What came back from the host's web port on a worker thread.
pub enum Web { Links(Value), PickupRefused(String) }

fn monotonic() -> f64 {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_secs_f64()
}

impl Profile {
    pub fn from_player(p: &Value) -> Self {
        Self { id: get(p, "id", ""), name: get(p, "name", "Player"), color: get(p, "color", ""), skin: get(p, "skin_color", ""),
            avatar: get(p, "avatar", ""), couch: p.get("couch").and_then(Value::as_f64).map(|c| c as i64) }
    }
}

impl Lobby {
    pub fn profile(&self, id: &str) -> Option<&Profile> { self.profiles.iter().find(|p| p.id == id) }

    pub fn name(&self, id: &str) -> String { self.profile(id).map_or_else(|| "Player".into(), |p| p.name.clone()) }

    pub fn toast(&mut self, message: impl Into<String>, color: Color) {
        let text = message.into();
        // A reconnect loop repeats the same refusal; keep one, refreshed.
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast { text, color, until: self.time + 3.5 });
        if self.toasts.len() > 3 { self.toasts.remove(0); }
    }

    /// Send a host command; a refusal while reconnecting shows as a toast.
    pub fn command(&mut self, message: Value) {
        let Some(client) = &self.client else { if let Some(out) = &mut self.outbox { out.push(message); } return };
        if let Err(e) = client.command(message) { self.toast(e, PINK); }
    }

    pub fn accepts_input(&self) -> bool { self.host.demo || self.client.as_ref().is_some_and(|c| c.connected && c.active) }

    // ------------------------------------------------------------ party

    pub fn set_party(&mut self, snapshot: Value) {
        let was_ready = self.host.update_ready();
        self.host.party = snapshot;
        if let Some((game, _)) = &self.host.quitting {
            let active = &self.host.party["active_session"];
            if !(active.is_object() && get(active, "game", "") == *game) { self.host.quitting = None; }
        }
        if was_ready.is_empty() && !self.host.update_ready().is_empty() {
            self.toast("New Splitscreen Club version ready. It installs when the room is quiet, or press Start.", GREEN);
        }
        self.profiles = self.host.party["players"].as_array().map(|a| a.iter().map(Profile::from_player).collect()).unwrap_or_default();
        self.apply_claims();
        self.controllers.clear();
        let mut slots = HashMap::new();
        for seat in self.host.party["seats"].as_array().cloned().unwrap_or_default() {
            let Some(id) = seat["occupant"].get("player_id").filter(|v| !v.is_null()).map(s) else { continue };
            slots.insert(id.clone(), seat.get("index").and_then(Value::as_f64).unwrap_or(0.0) as usize);
            if let Some(c) = seat.get("controller").filter(|c| !c.is_null()) { self.controllers.insert(s(c), id); }
        }
        let walkers: Vec<String> = self.controllers.values().cloned().collect();
        self.phones = self.host.party["phones"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default();
        // A character whose controller went keeps standing, asleep and marked,
        // until it comes back or the player is taken out. A phone player sits.
        self.bench = self.profiles.iter().map(|p| p.id.clone())
            .filter(|id| !walkers.contains(id) && (self.phones.contains(id) || self.world.player(id).is_none())).collect();
        let ids: Vec<String> = self.world.players.iter().map(|p| p.id.clone()).collect();
        for id in ids {
            if self.profile(&id).is_none() || self.bench.contains(&id) {
                self.world.remove_player(&id);
                self.menus.remove(&id);
            }
        }
        for p in self.profiles.clone() {
            if walkers.contains(&p.id) || self.phones.contains(&p.id) { self.lost_since.remove(&p.id); }
            else { self.lost_since.entry(p.id.clone()).or_insert(self.time); }
        }
        let known: Vec<String> = self.profiles.iter().map(|p| p.id.clone()).collect();
        self.lost_since.retain(|id, _| known.contains(id));
        for (i, id) in known.iter().enumerate() {
            if walkers.contains(id) { self.world.ensure_player(id, *slots.get(id).unwrap_or(&i)); }
        }
        self.update_sleep();
        let shelf = self.host.shelf_games().len();
        if shelf > 0 { self.host.shelf_offset %= shelf; }
        self.screens_dirty = true;
    }

    pub fn update_sleep(&mut self) {
        let sleeping: Vec<String> = self.host.party["presence"].as_array().map(|a| a.iter()
            .filter(|p| p["state"] == "sleeping").map(|p| get(p, "player_id", "")).collect()).unwrap_or_default();
        let time = self.world.time;
        for p in &mut self.world.players {
            let quiet = time - p.last_input > 1.0;
            p.sleeping = (sleeping.contains(&p.id) && quiet) || self.lost_since.contains_key(&p.id);
        }
    }

    /// Takes out players whose controller (and phone) stayed gone. Only
    /// counts while the room is on screen.
    pub fn remove_lost(&mut self, delta: f32) {
        let accepts = self.accepts_input();
        for id in self.lost_since.keys().cloned().collect::<Vec<_>>() {
            if !accepts { *self.lost_since.get_mut(&id).unwrap() += delta; continue; }
            if self.time - self.lost_since[&id] < LOST_SECONDS { continue; }
            self.lost_since.remove(&id);
            let name = self.name(&id);
            self.toast(format!("{name} left the party: no controller."), DIM);
            self.command(json!({"type": "leave_party", "player_id": id}));
        }
    }

    pub fn lost_left(&self, id: &str) -> Option<i64> {
        self.lost_since.get(id).map(|since| (LOST_SECONDS - (self.time - since)).ceil().max(0.0) as i64)
    }

    // ------------------------------------------------------------ input

    fn clear_player_input(&mut self, id: &str) {
        if let Some(p) = self.world.player_mut(id) { p.held = 0; p.pressed = 0; p.stick = [0.0; 2]; p.buffer = 0.0; p.vx = 0.0; }
    }

    pub fn suspend_input(&mut self) {
        self.menus.clear();
        let ids: Vec<String> = self.world.players.iter().map(|p| p.id.clone()).collect();
        for id in ids { self.input_blocked.insert(id.clone()); self.clear_player_input(&id); }
    }

    /// Raw controller frames from the host.
    pub fn controllers_changed(&mut self, frames: &[Value]) {
        for frame in frames {
            let token = get(frame, "controller", "");
            let mut buttons = frame.get("buttons").and_then(Value::as_f64).unwrap_or(0.0) as u32;
            let axes: Vec<f64> = frame["axes"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_else(|| vec![0.0; 6]);
            let Some(id) = self.controllers.get(&token).cloned() else {
                // A controller that hasn't joined: show it's heard on the table.
                let pushed = axes.iter().any(|a| a.abs() > 12000.0);
                let (before, was_pushed) = self.unjoined.get(&token).copied().unwrap_or((0, false));
                let fresh = buttons & !before & !BTN_A != 0 || (pushed && !was_pushed);
                self.unjoined.insert(token, (buttons, pushed));
                if self.accepts_input() && fresh { self.mood.pad_activity = 2.0; self.mood.pad_hop = 0.0; }
                continue;
            };
            let stick = if axes.len() >= 2 { [(axes[0] / 32767.0) as f32, (axes[1] / 32767.0) as f32] } else { [0.0; 2] };
            // Items are used with the right trigger, so it becomes a button here.
            if axes.len() >= 6 && axes[5] / 32767.0 > 0.5 { buttons |= BTN_RT; }
            self.input(&id, buttons, stick);
        }
    }

    /// One player's controller state; demo bots and local keys use it too.
    pub fn input(&mut self, id: &str, buttons: u32, stick: [f32; 2]) {
        if self.world.player(id).is_none() { return; }
        let length = (stick[0] * stick[0] + stick[1] * stick[1]).sqrt();
        if !self.accepts_input() {
            self.held.insert(id.into(), buttons);
            self.input_blocked.insert(id.into());
            self.clear_player_input(id);
            return;
        }
        if self.input_blocked.contains(id) {
            self.held.insert(id.into(), buttons);
            self.clear_player_input(id);
            if buttons == 0 && length <= 0.25 { self.input_blocked.remove(id); }
            return;
        }
        let before = self.held.get(id).copied().unwrap_or(0);
        let pressed = buttons & !before;
        self.held.insert(id.into(), buttons);
        if buttons != 0 || length > 0.3 {
            let time = self.world.time;
            let p = self.world.player_mut(id).unwrap();
            p.last_input = time;
            p.sleeping = false;
        }
        if pressed & BTN_START != 0 {
            if self.menus.remove(id).is_none() { self.menus.insert(id.into(), Menu::default()); }
        }
        if self.menus.contains_key(id) {
            self.menu_input(id, pressed, stick);
            self.world.set_input(id, 0, [0.0; 2]);
            return;
        }
        if pressed & BTN_B != 0 {
            let i = self.world.players.iter().position(|p| p.id == id).unwrap();
            if let Some(k) = self.world.pickup_target(i) {
                let item = &self.world.items[k];
                self.item_presses.insert(item.id, (item.kind, glam::Vec3::new(item.x as f32, item.y as f32, 0.05), self.time));
            }
        }
        let station = self.station_for(id);
        let kind = station.as_ref().map_or("", |s| s.kind.as_str()).to_string();
        let station_x = matches!(kind.as_str(), "shelf" | "queue") || (kind == "tv" && station.as_ref().is_some_and(|s| s.quit));
        // X only works the room's buttons; the world never sees it.
        self.world.set_input(id, buttons & !BTN_X, stick);
        if station.as_ref().is_some_and(|s| s.hold) {
            if pressed & BTN_Y != 0 { self.holds.insert(id.into(), (station.as_ref().unwrap().id.clone(), self.time)); }
        } else if pressed & BTN_Y != 0 { self.use_station(id, false); }
        if station_x && pressed & BTN_X != 0 { self.use_station(id, true); }
        if kind == "shelf" && pressed & (BTN_LB | BTN_RB) != 0 {
            let key = if pressed & BTN_LB != 0 { "LB" } else { "RB" };
            self.press_prompt("shelf", key);
            self.browse(if pressed & BTN_LB != 0 { -1 } else { 1 });
        } else if pressed & (BTN_LB | BTN_RB) != 0 {
            self.skip_music(id, if pressed & BTN_LB != 0 { -1 } else { 1 });
        }
    }

    pub fn station_for(&self, id: &str) -> Option<Station> { self.world.player(id).and_then(|p| self.host.station_for(p)) }

    fn cooldown(&mut self, key: String) -> bool {
        let now = monotonic();
        if now < *self.station_cooldown.get(&key).unwrap_or(&0.0) { return false; }
        self.station_cooldown.insert(key, now + 0.4);
        true
    }

    fn skip_music(&mut self, id: &str, direction: i32) {
        if self.station_for(id).is_none_or(|s| s.kind != "music") { return; }
        self.skip_music_as(Some(id), direction);
    }

    /// LB/RB at the jukebox, for a player or (None) the mouse.
    pub(crate) fn skip_music_as(&mut self, id: Option<&str>, direction: i32) {
        if !self.host.has_track() { return; }
        let action = if direction < 0 { "previous_track" } else { "next_track" };
        let station = if direction < 0 { "music0" } else { "music2" };
        if !self.cooldown(format!("{station}:{action}")) { return; }
        self.press_prompt("music", if direction < 0 { "LB" } else { "RB" });
        self.command(json!({"type": "media_control", "action": action}));
        self.celebrate_as(id, station);
    }

    pub(crate) fn browse(&mut self, step: i64) {
        let n = self.host.shelf_games().len() as i64;
        if n == 0 { return; }
        self.host.shelf_offset = (self.host.shelf_offset as i64 + step).rem_euclid(n) as usize;
        self.pop_box = Some((step as f32, self.time));
        self.screens_dirty = true;
    }

    /// Y uses the station; X is its second button (queue, skip, quit).
    pub fn use_station(&mut self, id: &str, queue: bool) {
        let Some(station) = self.station_for(id) else { return };
        self.use_station_as(Some(id), station, queue);
    }

    /// Use `station` for a player, or for the mouse (None), which is nobody:
    /// it never leaves the party or picks up a profile.
    pub(crate) fn use_station_as(&mut self, id: Option<&str>, station: Station, queue: bool) {
        if !station.enabled { return; }
        // Y on a game still loading starts it once it has loaded.
        if !queue && station.loading && matches!(station.kind.as_str(), "tv" | "queue") && station.action == "next" {
            if !self.cooldown(format!("{}:wait", station.id)) { return; }
            self.press_prompt(&station.id, "Y");
            self.command(json!({"type": "next"}));
            let title = if station.title.is_empty() { self.host.cabinet_queue().first().map(|e| e.title.clone()).unwrap_or_default() } else { station.title.clone() };
            self.toast(format!("{} starts as soon as it has loaded.", if title.is_empty() { "The game".into() } else { title }), GREEN);
            return;
        }
        if !self.cooldown(format!("{}:{}:{}", station.id, station.action, if queue { "X" } else { "Y" })) { return; }
        self.press_prompt(if station.kind == "music" { "music" } else { &station.id }, if queue { "X" } else { "Y" });
        match station.kind.as_str() {
            "shelf" => {
                if queue {
                    self.command(json!({"type": "queue_game", "game": station.game, "first": false}));
                    self.toast(format!("{} is in the queue.", station.title), GREEN);
                } else if self.host.current_game().is_some_and(|g| get(&g, "id", "") == station.game) {
                    self.command(json!({"type": "close_overlay"}));
                } else {
                    // Play now: on top of the queue, then switch once it has loaded.
                    self.command(json!({"type": "queue_next", "game": station.game}));
                    self.command(json!({"type": "next"}));
                    self.toast(format!("{} starts as soon as it has loaded.", station.title), GREEN);
                }
            }
            "queue" => {
                let entries = self.host.cabinet_queue();
                let Some(first) = entries.first() else { return };
                if queue {
                    let expected = self.host.party["playlist"].clone();
                    self.command(json!({"type": "remove_playlist_entry", "expected": expected, "index": first.index}));
                    self.toast(format!("Skipped {}.", first.title), GREEN);
                } else if self.host.upcoming().first() != Some(first) {
                    // The cabinet's game is not the TV's: put it first, then start it.
                    self.command(json!({"type": "queue_next", "game": first.game}));
                    self.command(json!({"type": "next"}));
                } else {
                    self.command(json!({"type": "next"}));
                }
            }
            "tv" => {
                if queue {
                    if !station.quit { return; }
                    let quit = self.host.current_game().unwrap_or_default();
                    self.toast(format!("Quit {}. The next game is up.", get(&quit, "title", "the game")), GREEN);
                    self.command(json!({"type": "next"}));
                    // The TV shows the next game now, while the host closes this one.
                    self.host.quitting = Some((get(&quit, "id", ""), self.time + 15.0));
                    self.screens_dirty = true;
                    return;
                }
                match station.action.as_str() {
                    "resume" => self.command(json!({"type": "close_overlay"})),
                    "next" => self.command(json!({"type": "next"})),
                    _ => {}
                }
            }
            "music" => self.command(json!({"type": "media_control", "action": station.action})),
            "exit" => {
                let Some(id) = id else { return };
                self.command(json!({"type": "leave_party", "player_id": id}));
                let name = self.name(id);
                self.toast(format!("{name} left. Press a button to join again."), DIM);
            }
            "door" => {
                let Some(id) = id else { return };
                let base = self.client.as_ref().map(|c| c.config.links_base.clone()).unwrap_or_default();
                let now = monotonic();
                // Without a host the program around the lobby picks up (the browser build).
                let outbox = self.client.is_none() && self.outbox.is_some();
                if (base.is_empty() && !outbox) || now < self.pickup_cooldown { return; }
                self.pickup_cooldown = now + 5.0;
                let (pending, player, swap, tx) = (station.pending.clone(), id.to_string(), station.hold, self.web_tx.clone());
                if outbox {
                    if let Some(out) = &mut self.outbox { out.push(json!({"type": "room_pickup", "pending": pending, "player_id": player, "swap": swap})); }
                } else { std::thread::spawn(move || {
                    let mut headers = vec!["X-GameNight-Local-Pickup: 1"];
                    if swap { headers.push("X-GameNight-Swap: 1"); }
                    let path = format!("/api/room-pickup/{}/{}", path_part(&pending), path_part(&player));
                    if let Err(e) = splitscreen_club::lobby::http(&base, "POST", &path, &headers) {
                        eprintln!("game-room: pickup refused: {e}");
                        let _ = tx.send(Web::PickupRefused(e));
                    }
                }); }
                // Instant: the character becomes the profile and the door empties now.
                self.claim(id, &station);
                let name = self.name(id);
                self.toast(format!("Welcome in, {name}!"), GREEN);
            }
            _ => {}
        }
        self.celebrate_as(id, &station.id);
    }

    fn celebrate_as(&mut self, id: Option<&str>, station: &str) {
        match id {
            Some(id) => self.celebrate(id, station),
            None => { self.pressed_flash.insert(station.into(), self.time); }
        }
    }

    pub fn celebrate(&mut self, id: &str, station: &str) {
        self.pressed_flash.insert(station.into(), self.time);
        let Some(p) = self.world.player_mut(id) else { return };
        p.cheer = 0.45;
        let (x, y) = (p.x as f32, p.y as f32);
        self.props.confetti(glam::Vec3::new(x, y + 1.6, 0.3));
        self.props.flash_at(glam::Vec3::new(x, 1.2, 1.0), AMBER, 4.0, 4.0);
    }

    // ------------------------------------------------------------ menu

    pub fn menu_items(&self, id: &str) -> Vec<MenuItem> {
        let page = self.menus.get(id).map(|m| m.page.as_str()).unwrap_or("");
        if page == "lobbies" { return self.lobby_items(); }
        if page == "remove" { return self.remove_items(id); }
        if id == crate::mouse::MOUSE { return self.mouse_items(); }
        let mut items = vec![MenuItem::new("Close menu", "close"), MenuItem::new("Leave the party", "leave")];
        // Hosts that predate couch positions don't send them.
        if self.profile(id).is_some_and(|p| p.couch.is_some()) { items.insert(1, MenuItem::new("Couch position", "couch")); }
        if !self.host.update_ready().is_empty() { items.insert(0, MenuItem::new("Update Splitscreen Club now", "update")); }
        if self.host.is_linked(id) { items.insert(1, MenuItem::new("Unlink my phone profile", "unlink")); }
        if self.profiles.len() > 1 { items.push(MenuItem::new("Remove a player", "remove_page")); }
        if self.host.party["lobbies"].as_array().is_some_and(|l| l.len() > 1) { items.push(MenuItem::new("Switch lobby", "lobbies")); }
        items.push(MenuItem::new("Quit Splitscreen Club", "quit"));
        items
    }

    pub fn couch_place(&self, id: &str) -> i64 { self.profile(id).and_then(|p| p.couch).unwrap_or(0) }

    fn move_on_couch(&mut self, id: &str, step: i64) {
        let from = self.couch_place(id);
        let to = from + step;
        if to < 0 || to >= self.profiles.len() as i64 { return; }
        for p in &mut self.profiles {
            if p.id != id && p.couch == Some(to) { p.couch = Some(from); }
            if p.id == id { p.couch = Some(to); }
        }
        self.command(json!({"type": "set_couch_position", "player_id": id, "position": to}));
    }

    /// The mouse's menu: nobody's, so only what the room itself can do.
    fn mouse_items(&self) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::new("Close menu", "close")];
        if !self.host.update_ready().is_empty() { items.push(MenuItem::new("Update Splitscreen Club now", "update")); }
        if self.host.party["lobbies"].as_array().is_some_and(|l| l.len() > 1) { items.push(MenuItem::new("Switch lobby", "lobbies")); }
        items.push(MenuItem::new("Quit Splitscreen Club", "quit"));
        items
    }

    fn remove_items(&self, id: &str) -> Vec<MenuItem> {
        let mut items = vec![];
        let others: Vec<String> = self.bench.iter().cloned().chain(self.world.players.iter().map(|p| p.id.clone())).collect();
        for other in others {
            if other == id || self.profile(&other).is_none() { continue; }
            let mut label = short(&self.name(&other), 18);
            if self.bench.contains(&other) { label += if self.phones.contains(&other) { "  (phone)" } else { "  (no controller)" }; }
            else if self.lost_since.contains_key(&other) { label += "  (no controller)"; }
            else if self.world.player(&other).is_some_and(|p| p.sleeping) { label += "  (away)"; }
            items.push(MenuItem { player: other.clone(), name: self.name(&other), ..MenuItem::new(&label, "remove") });
        }
        items.push(MenuItem::new("Back", "back"));
        items
    }

    fn lobby_items(&self) -> Vec<MenuItem> {
        let mut items = vec![];
        for lobby in self.host.party["lobbies"].as_array().cloned().unwrap_or_default() {
            let current = lobby["current"].as_bool().unwrap_or(false);
            let title = lobby.get("title").map(s).unwrap_or_else(|| get(&lobby, "id", ""));
            let label = format!("{title}{}", if current { "  (here now)" } else { "" });
            items.push(MenuItem { lobby: get(&lobby, "id", ""), title, current, ..MenuItem::new(&label, "lobby") });
        }
        items.push(MenuItem::new("Back", "back"));
        items
    }

    pub(crate) fn menu_page(&mut self, id: &str, page: &str) {
        let current = self.host.party["lobbies"].as_array().and_then(|l| l.iter().position(|x| x["current"].as_bool().unwrap_or(false)));
        let Some(menu) = self.menus.get_mut(id) else { return };
        menu.page = page.into();
        menu.confirm = false;
        menu.index = 0;
        if page == "lobbies" { if let Some(i) = current { menu.index = i; } }
    }

    pub(crate) fn menu_input(&mut self, id: &str, pressed: u32, stick: [f32; 2]) {
        let items = self.menu_items(id);
        let menu = self.menus.get_mut(id).unwrap();
        menu.index = menu.index.min(items.len() - 1);
        let up = pressed & BTN_UP != 0 || (stick[1] < -0.6 && !menu.stick);
        let down = pressed & BTN_DOWN != 0 || (stick[1] > 0.6 && !menu.stick);
        let left = pressed & BTN_LEFT != 0 || (stick[0] < -0.6 && !menu.stick_x);
        let right = pressed & BTN_RIGHT != 0 || (stick[0] > 0.6 && !menu.stick_x);
        menu.stick = stick[1].abs() > 0.6;
        menu.stick_x = stick[0].abs() > 0.6;
        let n = items.len();
        if up { menu.index = (menu.index + n - 1) % n; menu.confirm = false; }
        if down { menu.index = (menu.index + 1) % n; menu.confirm = false; }
        let (index, confirm, page) = (menu.index, menu.confirm, menu.page.clone());
        if (left || right) && !up && !down && items[index].action == "couch" { self.move_on_couch(id, if left { -1 } else { 1 }); }
        if pressed & BTN_B != 0 {
            if !page.is_empty() { self.menu_page(id, ""); } else { self.menus.remove(id); }
            return;
        }
        if pressed & BTN_A == 0 { return; }
        let item = items[index].clone();
        let set_confirm = |lobby: &mut Lobby| if let Some(m) = lobby.menus.get_mut(id) { m.confirm = true; };
        match item.action.as_str() {
            "close" => { self.menus.remove(id); }
            "lobbies" => self.menu_page(id, "lobbies"),
            "remove_page" => self.menu_page(id, "remove"),
            "remove" => if confirm {
                self.menu_page(id, "");
                self.toast(format!("{} left the party.", item.name), DIM);
                self.command(json!({"type": "leave_party", "player_id": item.player}));
            } else { set_confirm(self) },
            "back" => self.menu_page(id, ""),
            "lobby" => if item.current {
                self.toast(format!("You're already in the {}.", item.title), DIM);
            } else {
                self.menus.clear();
                self.toast(format!("Opening the {}… everyone stays in the party.", item.title), GREEN);
                self.command(json!({"type": "switch_lobby", "lobby": item.lobby}));
            },
            "update" => {
                self.menus.remove(id);
                self.toast("Updating Splitscreen Club… everyone stays in the party.", GREEN);
                self.command(json!({"type": "apply_update"}));
            }
            "leave" => { self.menus.remove(id); self.command(json!({"type": "leave_party", "player_id": id})); }
            "unlink" => {
                self.menus.remove(id);
                let base = self.client.as_ref().map(|c| c.config.links_base.clone()).unwrap_or_default();
                if !base.is_empty() {
                    let path = format!("/api/player-links/{}/unlink", path_part(id));
                    std::thread::spawn(move || { let _ = splitscreen_club::lobby::http(&base, "POST", &path, &["X-GameNight-Local-Pickup: 1"]); });
                }
            }
            "quit" => if confirm {
                if self.client.as_ref().is_some_and(|c| c.connected) { self.command(json!({"type": "quit_party"})); } else { self.quit = true; }
            } else { set_confirm(self) },
            _ => {}
        }
    }

    // ------------------------------------------------------------ phone links

    /// Show a pickup straight away, before the host has done it.
    fn claim(&mut self, id: &str, station: &Station) {
        for entry in self.host.pending_profiles() {
            if get(&entry, "id", "") != station.pending { continue; }
            self.claims.insert(id.into(), (entry["profile"].clone(), self.time + 8.0));
            // The door list lags behind the party, so keep the door empty on its own.
            self.host.claimed_doors.insert(get(&entry, "id", ""), self.time + 8.0);
            self.apply_claims();
            return;
        }
    }

    /// Lay unconfirmed pickups over the host's players.
    fn apply_claims(&mut self) {
        let time = self.time;
        let claims: Vec<(String, (Value, f32))> = self.claims.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (id, (profile, until)) in claims {
            let name = get(&profile, "display_name", "");
            let avatar = get(&profile, "avatar", "");
            let Some(player) = self.profiles.iter_mut().find(|p| p.id == id) else { self.claims.remove(&id); continue };
            if time > until || (player.name == name && player.avatar == avatar) { self.claims.remove(&id); continue; }
            if !name.is_empty() { player.name = name; }
            if !avatar.is_empty() { player.avatar = avatar; }
            if profile.get("skin_color").is_some_and(|v| !v.is_null()) { player.skin = get(&profile, "skin_color", ""); }
            if profile.get("preferred_color").is_some_and(|v| !v.is_null()) { player.color = get(&profile, "preferred_color", ""); }
        }
    }

    pub fn drop_claims(&mut self) {
        if self.claims.is_empty() && self.host.claimed_doors.is_empty() { return; }
        self.claims.clear();
        self.host.claimed_doors.clear();
        let party = self.host.party.clone();
        self.set_party(party);
    }

    pub fn set_links(&mut self, links: Value) {
        self.host.links = links;
        self.show.room_code = {
            let room = &self.host.links["room"];
            let code = room.get("room_code").or_else(|| room.get("code")).map(s).unwrap_or_default();
            if code == "<null>" { String::new() } else { code }
        };
        let svg = get(&self.host.links["room"], "qr_svg", "");
        self.show.qr = if svg.len() < 200_000 { crate::qr::modules(&svg) } else { vec![] };
        self.screens_dirty = true;
    }

    // ------------------------------------------------------------ prompts

    pub fn set_prompt(&mut self, name: &str, items: &[(&str, &str, bool)], enabled: bool, active: bool) {
        let Some(p) = self.prompts.get_mut(name) else { return };
        self.prompt_active.insert(name.into(), active);
        let items: Vec<(String, String, bool)> = items.iter().map(|(k, t, on)| (k.to_string(), t.to_string(), *on)).collect();
        if p.items != items || p.enabled != enabled {
            p.items = items;
            p.enabled = enabled;
            self.screens_dirty = true;
        }
    }

    /// A nudge from afar shows only its words; the button pops in when
    /// someone (or the mouse) is there.
    /// `was_active`: whether it showed before this update; one that just
    /// appears takes its look at once instead of animating into it.
    fn tuck_prompt(&mut self, name: &str, tucked: bool, was_active: bool) {
        let Some(p) = self.prompts.get_mut(name) else { return };
        p.conceal_to = if tucked { 1.0 } else { 0.0 };
        if !was_active || !self.prompt_active.get(name).copied().unwrap_or(false) { p.conceal = p.conceal_to; }
    }

    pub fn press_prompt(&mut self, name: &str, key: &str) {
        if let Some(p) = self.prompts.get_mut(name) { p.pressed = Some((key.into(), 1.0)); self.screens_dirty = true; }
        if name == "shelf" && (key == "LB" || key == "RB") { self.show.bumper_presses[(key == "RB") as usize] = 1.0; }
    }

    fn hold_prompt(&mut self, name: &str, key: &str, amount: f32) {
        let Some(p) = self.prompts.get_mut(name) else { return };
        let current = p.hold.as_ref().filter(|h| h.0 == key).map_or(0.0, |h| h.1);
        if (current - amount).abs() < 1e-4 { return; }
        p.hold = (amount > 0.0).then(|| (key.into(), amount));
        self.screens_dirty = true;
    }

    /// The floating buttons light up while someone stands where they work.
    pub fn update_prompts(&mut self) {
        let mut occupied: HashMap<String, Station> = HashMap::new();
        let ids: Vec<String> = self.world.players.iter().map(|p| p.id.clone()).collect();
        for id in &ids {
            if self.menus.contains_key(id) { continue; }
            if let Some(station) = self.station_for(id) {
                if !occupied.contains_key(&station.id) || station.enabled { occupied.insert(station.id.clone(), station); }
            }
        }
        // The mouse lights what it points at, as if someone stood there;
        // where no player stands, the buttons show the mouse instead of Y/X.
        let mut mouse_only = None;
        if let Some(station) = self.mouse_station() {
            if !occupied.contains_key(&station.id) { mouse_only = Some(crate::mouse::prompt_name(&station).to_string()); }
            if !occupied.contains_key(&station.id) || station.enabled { occupied.insert(station.id.clone(), station); }
        }
        // Onboarding, once someone has joined: until the first game, the TV's
        // Play hops for everyone to see where a night starts; a profile
        // someone scanned in hops at its door until it is picked up.
        self.nudges.clear();
        let joined = !self.world.players.is_empty();
        // Pick up only nudges while someone still plays without a profile.
        let unlinked = self.world.players.iter().any(|p| !self.host.is_linked(&p.id) && !self.claims.contains_key(&p.id));
        if self.host.current_game().is_some() { self.started_once = true; }
        let tv = self.host.tv_station();
        let tv_items: Vec<(&str, &str, bool)> = if tv.quit { vec![("Y", &tv.short, true), ("X", "Quit", true)] } else { vec![("Y", &tv.short, true)] };
        let tv_nudge = joined && !self.started_once && tv.enabled && !occupied.contains_key("tv");
        if tv_nudge { self.nudges.insert("tv".into()); }
        let was = self.prompt_active.get("tv").copied().unwrap_or(false);
        self.set_prompt("tv", &tv_items, tv.enabled, occupied.contains_key("tv") || tv_nudge);
        self.tuck_prompt("tv", tv_nudge, was);
        let q = self.host.queue_station();
        self.set_prompt("queue", &[("Y", &q.short, !q.loading), ("X", "Skip next", true)], q.enabled, occupied.contains_key("queue"));
        let shelf_live = !self.host.shelf_games().is_empty();
        self.featured_visible = shelf_live && occupied.contains_key("shelf");
        let shelf_enabled = occupied.get("shelf").map_or(shelf_live, |s| s.enabled);
        self.set_prompt("shelf", &[("Y", "Play now", true), ("X", "Queue", true)], shelf_enabled, occupied.contains_key("shelf"));
        self.bumpers_visible = occupied.contains_key("shelf") && self.host.shelf_games().len() > 1;
        let playing = self.host.party["now_playing"]["playing"].as_bool().unwrap_or(false);
        let music: Vec<(&str, &str, bool)> = if self.host.has_track() {
            vec![("LB", "Previous", true), ("Y", if playing { "Pause" } else { "Play" }, true), ("RB", "Next", true)]
        } else { vec![("Y", "Play", true)] };
        self.set_prompt("music", &music, true, occupied.contains_key("music1"));
        let exit = if mouse_only.as_deref() == Some("exit") { if self.mouse_quit_armed() { "Click again to quit" } else { "Quit Splitscreen Club" } } else { "Leave" };
        self.set_prompt("exit", &[("Y", exit, true)], true, occupied.contains_key("exit"));
        for (name, p) in self.prompts.iter_mut() {
            let mouse = mouse_only.as_deref() == Some(name.as_str());
            let hover = if mouse { self.mouse.hover.as_ref().and_then(|h| h.1.clone()) } else { None };
            if p.mouse != mouse || p.hover != hover { p.mouse = mouse; p.hover = hover; self.screens_dirty = true; }
        }
        let waiting = self.host.pending_profiles().len();
        for i in 0..game_room_sim::layout::PROFILE_DOORS.len() {
            let key = format!("door{i}");
            let nudge = unlinked && i < waiting && !occupied.contains_key(&key);
            if nudge { self.nudges.insert(key.clone()); }
            let enabled = nudge || occupied.get(&key).is_some_and(|s| s.enabled);
            let swap = occupied.get(&key).is_some_and(|s| s.hold);
            let was = self.prompt_active.get(&key).copied().unwrap_or(false);
            self.set_prompt(&key, &[("Y", if swap { "Hold to swap" } else { "Pick up" }, true)], enabled, occupied.contains_key(&key) || nudge);
            self.tuck_prompt(&key, nudge, was);
            let charge = self.holds.iter().filter(|(_, h)| h.0 == key).map(|(id, _)| self.hold_charge(id)).fold(0.0, f32::max);
            self.hold_prompt(&key, "Y", charge);
        }
        let join_hint = self.profiles.is_empty() && (self.host.demo || self.client.as_ref().is_some_and(|c| c.connected));
        let join = join_hint || self.mood.pad_activity > 0.0;
        self.set_prompt("join", &[("A", "Join", true)], true, join);
    }

    /// Fire a hold once Y has stayed down long enough at the same door.
    pub fn update_holds(&mut self) {
        for id in self.holds.keys().cloned().collect::<Vec<_>>() {
            let station = self.station_for(&id);
            let held = self.held.get(&id).copied().unwrap_or(0) & BTN_Y != 0;
            let same = station.as_ref().is_some_and(|s| s.id == self.holds[&id].0 && s.hold);
            if self.world.player(&id).is_none() || !held || !same { self.holds.remove(&id); }
            else if self.hold_charge(&id) >= 1.0 { self.holds.remove(&id); self.use_station(&id, false); }
        }
    }

    pub fn hold_charge(&self, id: &str) -> f32 { self.holds.get(id).map_or(0.0, |h| ((self.time - h.1) / HOLD_SECONDS).clamp(0.0, 1.0)) }

    /// Badges over items someone can pick up (and ones just picked up).
    pub fn pickup_hint_items(&self) -> Vec<(u32, world::Kind)> {
        let mut out: Vec<(u32, world::Kind)> = vec![];
        for (i, p) in self.world.players.iter().enumerate() {
            if self.menus.contains_key(&p.id) { continue; }
            if let Some(k) = self.world.pickup_target(i) {
                let item = &self.world.items[k];
                if !out.iter().any(|o| o.0 == item.id) { out.push((item.id, item.kind)); }
            }
        }
        out
    }
}

pub fn short(value: &str, length: usize) -> String {
    if value.chars().count() <= length { value.into() } else { value.chars().take(length - 1).collect::<String>() + "…" }
}

fn path_part(s: &str) -> String { s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect() }

/// demo.gd's offline party: six players, a library, a queue, music, doors.
pub fn demo_party(count: usize, names: &[&str], colors: &[&str], skins: &[&str]) -> (Value, Value) {
    let players: Vec<Value> = (0..count).map(|i| json!({"id": format!("p{i}"), "name": names[i % names.len()], "color": colors[i % colors.len()],
        "skin_color": skins[i % skins.len()], "avatar": crate::face::demo(i), "couch": i})).collect();
    let seats: Vec<Value> = (0..count).map(|i| json!({"index": i, "occupant": {"kind": "local", "player_id": format!("p{i}")}, "controller": format!("demo:{i}")})).collect();
    let library = json!([
        {"id": "spaceracer", "title": "Space Racer", "color": "#38d6ff", "players": "1-8", "max_players": 8},
        {"id": "ballkickers", "title": "Ball Kickers", "color": "#3ddc97", "players": "2-8", "max_players": 8},
        {"id": "downhill-rush", "title": "Downhill Rush", "color": "#ffb454", "players": "1-4", "max_players": 4},
        {"id": "god-game", "title": "God Game", "color": "#ff4f8b", "players": "2-6", "max_players": 6, "play_with": "controller_or_phone", "min_phones": 1},
        {"id": "hexstead", "title": "Hexstead", "color": "#e8c14a", "players": "2-6", "min_players": 2, "max_players": 6, "play_with": "phone"},
        {"id": "blast-party", "title": "Blast Party", "color": "#a78bfa", "players": "2-4", "max_players": 4},
    ]);
    let phones: Vec<&str> = ["p0", "p2", "p3"].into_iter().filter(|id| players.iter().any(|p| p["id"] == *id)).collect();
    let party = json!({"players": players, "seats": seats, "library": library,
        "playlist": {"entries": if count == 0 { json!([]) } else { json!([{"game": "spaceracer", "title": "Space Racer"}, {"game": "ballkickers", "title": "Ball Kickers"},
            {"game": "god-game", "title": "God Game"}, {"game": "blast-party", "title": "Blast Party"}]) }},
        "presence": [], "phones": phones,
        "warm_session": {"game": "spaceracer", "phase": "preparing", "progress": 64, "progress_label": "Loading"},
        "now_playing": {"title": "Midnight City", "artist": "M83", "playing": true, "source": "Spotify"}});
    let links = json!({"room": {"room_code": "KJX4", "pending": [
        {"id": "pend1", "profile": {"display_name": "Sanne", "skin_color": "#edc59a", "avatar": crate::face::demo(8)}},
        {"id": "pend2", "profile": {"display_name": "Thijs", "skin_color": "#b77b50", "avatar": crate::face::demo(9)}},
        {"id": "pend3", "profile": {"display_name": "Lotte", "skin_color": "#f0d2b4", "avatar": crate::face::demo(10)}}]}, "linked": {}});
    (party, links)
}
