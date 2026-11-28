//! What the lobby makes of the host's party snapshot (main.gd): which games
//! fit the party, the queue as the TV and the Up next cabinet show it,
//! loading progress, and what each station does for a player standing at
//! it. Pure: the live lobby feeds it snapshots and the clock.

use game_room_sim::layout as l;
use game_room_sim::world::Player;
use serde_json::Value;
use std::collections::HashMap;

/// GDScript's `str()` of a JSON value: strings as they are, whole floats
/// without decimals, null as `<null>`.
pub fn s(v: &Value) -> String {
    match v {
        Value::String(t) => t.clone(),
        Value::Null => "<null>".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() { Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64), _ => n.to_string() },
        other => other.to_string(),
    }
}

/// `str(d.get(key, default))`.
pub fn get(v: &Value, key: &str, default: &str) -> String {
    match v.get(key) { Some(x) => s(x), None => default.into() }
}

fn arr(v: &Value) -> &[Value] { v.as_array().map(Vec::as_slice).unwrap_or(&[]) }
fn dict(v: &Value) -> bool { v.is_object() }
fn int(v: &Value) -> i64 { v.as_f64().map(|f| f as i64).or_else(|| v.as_str().and_then(|t| t.parse().ok())).unwrap_or(0) }

/// One queued game the lobby shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry { pub index: usize, pub game: String, pub title: String }

/// What a player can use where they stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Station {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub short: String,
    pub action: String,
    pub enabled: bool,
    pub extra: String,
    /// The TV: X quits the running game.
    pub quit: bool,
    /// A door that swaps a linked profile: Y must be held.
    pub hold: bool,
    pub loading: bool,
    pub game: String,
    pub title: String,
    /// A door's waiting profile.
    pub pending: String,
}

/// Something on its way to playable (main.gd `_loading_entries`).
#[derive(Clone, Debug, PartialEq)]
pub struct Loading { pub game: String, pub stage: String, pub progress: Option<f64>, pub label: String }

/// The screen and GPU this host draws on, for [`Host::hardware_note`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostDevice {
    /// As the adapter reports it ("Adreno (TM) 740"); empty in a browser
    /// that hides it.
    pub gpu: String,
    pub size: [u32; 2],
    /// The phone preset (its 3D view starts at 1080 lines, not 1440).
    pub phone: bool,
}

/// The lowest step of the shells' resolution ladder (app::quality, 0.67 of
/// the start height, which is 1080 lines on a phone and 1440 elsewhere).
pub fn ladder_floor(d: &HostDevice) -> u32 {
    let start = d.size[1].min(if d.phone { 1080 } else { 1440 });
    (start as f32 * 0.67) as u32
}

/// The GPU time `bench` (a game's measured cost on this GPU: fixed ms + ns
/// per 3D pixel) predicts at `lines` on this screen.
pub fn predicted_gpu_ms(bench: &Value, d: &HostDevice, lines: u32) -> Option<f64> {
    let fixed = bench["gpu_fixed_ms"].as_f64()?;
    let per_px = bench["gpu_ns_per_px"].as_f64()?;
    let aspect = f64::from(d.size[0]) / f64::from(d.size[1].max(1));
    let px = f64::from(lines) * f64::from(lines) * aspect;
    Some(fixed + per_px * px / 1e6)
}

#[derive(Default)]
pub struct Host {
    pub party: Value,
    pub lobby_id: String,
    pub connected: bool,
    /// Running the demo: no host, everything works.
    pub demo: bool,
    /// No jukebox in the room (the browser can't steer the OS's music).
    pub no_jukebox: bool,
    /// The online switch (where the jukebox is not): Public, or Friends
    /// only. The browser host starts on Public.
    pub public: bool,
    pub time: f32,
    pub shelf_offset: usize,
    /// The shelf's order while the catalog is open (game ids): a game that
    /// stops fitting (someone joined to play) stays where it was instead of
    /// jumping to the end under the mouse (Joep, 2026-11-26).
    pub shelf_order: Option<Vec<String>>,
    /// /api/player-links.
    pub links: Value,
    /// A game just quit: hidden until the host has closed it.
    pub quitting: Option<(String, f32)>,
    /// Doors emptied by a pickup the host hasn't confirmed yet.
    pub claimed_doors: HashMap<String, f32>,
    load_shown: HashMap<String, f32>,
    /// Set by the shell that draws the room; None: no hardware notes.
    pub device: Option<HostDevice>,
    load_report: HashMap<String, (f64, f32, String)>,
}

impl Host {
    pub fn new(lobby_id: &str) -> Self { Self { party: Value::Object(Default::default()), lobby_id: lobby_id.into(), links: Value::Object(Default::default()), ..Default::default() } }

    pub fn library(&self) -> &[Value] { arr(&self.party["library"]) }

    /// The shelf and the catalog: the games this party can play now, then
    /// the ones it can't yet (dimmed, can't start: "Needs 2 players"). With
    /// only the keyboard, games made for controllers are left out
    /// (Joep, 2026-11-25); [`Self::hidden_for_keyboard`] counts them.
    pub fn shelf_games(&self) -> Vec<&Value> {
        let games: Vec<&Value> = self.library().iter().filter(|g| get(g, "id", "<null>") != self.lobby_id && self.input_fits(g)).collect();
        let (mut fit, rest): (Vec<&Value>, Vec<&Value>) = games.into_iter().partition(|g| self.fits_party(g));
        fit.extend(rest);
        if let Some(order) = &self.shelf_order {
            fit.sort_by_key(|g| order.iter().position(|id| *id == get(g, "id", "")).unwrap_or(usize::MAX));
        }
        fit
    }

    /// Whether everyone seated plays on the browser's keyboard.
    pub fn keyboard_only(&self) -> bool {
        let seated: Vec<&Value> = arr(&self.party["seats"]).iter().filter(|seat| seat["occupant"].get("player_id").is_some_and(|v| !v.is_null())).collect();
        !seated.is_empty() && seated.iter().all(|seat| seat["controller"].as_str().is_some_and(|c| c.contains(":keyboard")))
    }

    /// Players on the keyboard: 2 when two share it (its halves,
    /// "web:keyboard-left" / "-right").
    fn keyboard_players(&self) -> usize {
        arr(&self.party["seats"]).iter().filter(|seat| seat["controller"].as_str().is_some_and(|c| c.contains(":keyboard"))).count()
    }

    /// Whether the input at hand plays `game`: anything does, unless only
    /// the keyboard is here and the game doesn't declare keys
    /// (`players.keyboard` or `players.mouse` in its catalog entry).
    /// Two on one keyboard: the game must take that many
    /// (`players.shared_keyboard`).
    fn input_fits(&self, game: &Value) -> bool {
        if !self.keyboard_only() { return true; }
        let shared = game["shared_keyboard"].as_u64().unwrap_or(1) as usize;
        match self.keyboard_players() { 0 | 1 => game["keyboard"] == true || game["mouse"] == true || shared >= 2, n => shared >= n }
    }

    /// Games left off the shelf because only the keyboard is here.
    pub fn hidden_for_keyboard(&self) -> usize {
        if !self.keyboard_only() { return 0; }
        self.library().iter().filter(|g| get(g, "id", "<null>") != self.lobby_id && !self.input_fits(g)).count()
    }

    pub fn game_meta(&self, id: &str) -> Option<&Value> { self.library().iter().find(|g| get(g, "id", "<null>") == id) }

    fn quitting_game(&self) -> String {
        match &self.quitting { Some((g, until)) if self.time < *until => g.clone(), _ => String::new() }
    }

    pub fn upcoming(&self) -> Vec<Entry> {
        let playlist = &self.party["playlist"];
        let entries = arr(&playlist["entries"]);
        let mut start = if playlist.get("current").is_none_or(Value::is_null) { 0 } else { int(&playlist["current"]) + 1 };
        let next_index = if self.party.get("next_game_index").is_some_and(|v| !v.is_null()) { int(&self.party["next_game_index"]) } else { -1 };
        let empty = Value::Object(Default::default());
        // Nobody seated yet: as for one person, so the TV doesn't offer a
        // game for two before anyone joined (Joep, 28 November 2026).
        let alone = self.human_count() == 0;
        let fits = |game: &str| {
            let meta = self.game_meta(game).unwrap_or(&empty);
            self.fits_party(meta) && (!alone || self.fits_one(meta))
        };
        let authoritative = next_index >= 0 && (next_index as usize) < entries.len() && fits(&s(&entries[next_index as usize]["game"]));
        if authoritative { start = next_index; }
        // The host may skip an incompatible entry or honour an explicit next pick.
        let mut target = &self.party["warm_session"];
        if !dict(target) { target = &self.party["warming"]; }
        if !authoritative && dict(target) {
            let game = get(target, "game", "");
            let active = self.current_game();
            if !game.is_empty() && active.as_ref().is_none_or(|a| get(a, "id", "") != game) {
                for offset in 0..entries.len() {
                    let index = (start as usize + offset) % entries.len();
                    if s(&entries[index]["game"]) == game && fits(&game) { start = index as i64; break; }
                }
            }
        }
        if start as usize >= entries.len() { start = 0; }
        let quitting = self.quitting_game();
        let mut result = vec![];
        // From `start` round to the entries before it: a next game picked
        // from the end of the queue (the browser host moves what it starts
        // to the back) left Up next nearly empty until it had been played
        // (Joep, 28 November 2026). The playing entry itself stays out.
        let current = if playlist.get("current").is_none_or(Value::is_null) { -1 } else { int(&playlist["current"]) };
        let order = (start as usize..entries.len()).chain(0..start as usize);
        for index in order {
            let entry = &entries[index];
            let game = s(&entry["game"]);
            if index as i64 == current && index < start as usize { continue; }
            if game == self.lobby_id || game == quitting || !fits(&game) || result.iter().any(|e: &Entry| e.game == game) { continue; }
            result.push(Entry { index, game, title: get(entry, "title", "Game") });
        }
        result
    }

    /// Whether one person (with the bots `game` brings) is enough for it.
    fn fits_one(&self, game: &Value) -> bool {
        let low = if game.get("min_players").is_none_or(Value::is_null) { 1 } else { int(&game["min_players"]).max(1) };
        let bots = if game.get("bots").is_some_and(|v| !v.is_null()) { int(&game["bots"]).max(0) } else { 0 };
        1 + bots >= low
    }

    pub fn human_count(&self) -> usize {
        arr(&self.party["seats"]).iter().filter(|seat| seat["occupant"].get("player_id").is_some_and(|p| !p.is_null())).count()
    }

    fn phones(&self) -> Vec<String> { arr(&self.party["phones"]).iter().map(s).collect() }

    /// Seated players who hold what `game` is played with: (humans, able, phones).
    pub fn party_fit(&self, game: &Value) -> (i64, i64, i64) {
        let phones = self.phones();
        let play_with = get(game, "play_with", "controller");
        let mut fit = (0, 0, 0);
        for seat in arr(&self.party["seats"]) {
            let Some(id) = seat["occupant"].get("player_id").filter(|v| !v.is_null()) else { continue };
            let phone = phones.contains(&s(id));
            let pad = seat.get("controller").is_some_and(|c| !c.is_null()) || !phone;
            let able = match play_with.as_str() { "phone" => phone, "controller_and_phone" => pad && phone, "controller_or_phone" => pad || phone, _ => pad };
            fit.0 += 1;
            if able { fit.1 += 1; if phone { fit.2 += 1; } }
        }
        fit
    }

    /// Why this party can't play `game` ("" when it can).
    pub fn shortfall(&self, game: &Value) -> String {
        let (humans, able, phones) = self.party_fit(game);
        if humans == 0 { return String::new(); }
        // Only keyboards here: a game that doesn't take them (or not two on
        // one keyboard) isn't suggested on the TV or Up next either.
        if !self.input_fits(game) {
            return if self.keyboard_players() >= 2 && (game["keyboard"] == true || game["mouse"] == true) { "One player per keyboard".into() } else { "Needs controllers".into() };
        }
        if game.get("max_players").is_some_and(|v| !v.is_null()) && able > int(&game["max_players"]) { return "Too many players".into(); }
        let low = if game.get("min_players").is_none_or(Value::is_null) { 1 } else { int(&game["min_players"]).max(1) };
        let play_with = get(game, "play_with", "controller");
        // Bots the game brings fill seats (Stack Together: one person and
        // its bot), as the host counts them when it starts the game.
        let bots = if game.get("bots").is_some_and(|v| !v.is_null()) { int(&game["bots"]).max(0) } else { 0 };
        let able = able + bots;
        if able < low {
            if humans < low { return format!("Needs {low} players"); }
            if matches!(play_with.as_str(), "phone" | "controller_and_phone") { return format!("Needs {low} phones"); }
            return format!("Needs {low} controllers");
        }
        if game.get("min_phones").is_some_and(|v| !v.is_null()) && phones < int(&game["min_phones"]) {
            let n = int(&game["min_phones"]);
            return if n == 1 { "Needs a phone".into() } else { format!("Needs {n} phones") };
        }
        String::new()
    }

    pub fn fits_party(&self, game: &Value) -> bool { self.shortfall(game).is_empty() }

    /// "May stutter here" when the game's cost measured on this GPU (its
    /// `bench` in library.json, from lobbies/game-room/hardware-fit.json)
    /// misses its frame rate by more than 10% even at the ladder's lowest
    /// step; empty when it fits or nothing was measured on this GPU. The
    /// game can still be picked; autoplay leaves it out
    /// (gamenight-internal docs/strategy/hardware-fit.md).
    pub fn hardware_note(&self, game: &Value) -> String {
        let Some(d) = self.device.as_ref().filter(|d| !d.gpu.is_empty() && d.size[1] > 0) else { return String::new() };
        let bench = &game["bench"][d.gpu.as_str()];
        let hz = game["frame_target_hz"].as_f64().filter(|hz| *hz >= 20.0).unwrap_or(60.0);
        match predicted_gpu_ms(bench, d, ladder_floor(d)) {
            Some(ms) if ms > 1000.0 / hz * 1.1 => "May stutter here".into(),
            _ => String::new(),
        }
    }

    /// Whether the room picks `game` by itself (autoplay, the next game):
    /// it fits the party and runs smoothly here.
    pub fn runs_smoothly(&self, game: &Value) -> bool { self.hardware_note(game).is_empty() }

    /// The running (or paused) game with its phase, or None.
    pub fn current_game(&self) -> Option<Value> {
        let session = &self.party["active_session"];
        if !session.as_object().is_some_and(|o| !o.is_empty()) || get(session, "game", "") == self.lobby_id { return None; }
        let game = get(session, "game", "");
        if !self.quitting_game().is_empty() && game == self.quitting_game() { return None; }
        let mut meta = self.game_meta(&game).cloned().unwrap_or_else(|| serde_json::json!({"id": game, "title": game}));
        meta["phase"] = session.get("phase").cloned().unwrap_or(Value::String(String::new()));
        Some(meta)
    }

    pub fn update_ready(&self) -> String {
        match self.party.get("update_ready") { None | Some(Value::Null) => String::new(), Some(v) => s(v) }
    }

    pub fn has_track(&self) -> bool { self.party["now_playing"].as_object().is_some_and(|o| !o.is_empty()) }

    pub fn issue(&self, game: &str) -> String {
        for issue in arr(&self.party["game_issues"]) {
            if get(issue, "game", "") == game {
                return match get(issue, "kind", "").as_str() {
                    "closed" => "Game closed", "disconnected" => "Game disconnected", "exited_unexpectedly" => "Game exited unexpectedly",
                    "failed_to_start" => "Could not start game", "startup_timeout" => "Game did not connect", _ => "Game stopped",
                }.into();
            }
        }
        String::new()
    }

    // ------------------------------------------------------------ loading

    pub fn loading_entries(&self) -> Vec<Loading> {
        let mut result = vec![];
        for key in ["active_session", "warm_session"] {
            let session = &self.party[key];
            if dict(session) && matches!(get(session, "phase", "").as_str(), "created" | "preparing") {
                result.push(Loading { game: get(session, "game", ""), stage: "load".into(), progress: session.get("progress").and_then(Value::as_f64),
                    label: session.get("progress_label").filter(|v| !v.is_null()).map(s).unwrap_or_else(|| "Preparing game".into()) });
            }
        }
        for install in arr(&self.party["installs"]) {
            if !dict(install) { continue; }
            let label = match get(install, "state", "").as_str() {
                "queued" => "Waiting to download", "downloading" => "Downloading", "verifying" => "Checking the download", "extracting" => "Unpacking", _ => continue,
            };
            result.push(Loading { game: get(install, "game", ""), stage: "download".into(), progress: install.get("percent").and_then(Value::as_f64), label: label.into() });
        }
        let warming = &self.party["warming"];
        if warming.as_object().is_some_and(|o| !o.is_empty()) {
            let game = get(warming, "game", "");
            if self.issue(&game).is_empty() && !result.iter().any(|e| e.game == game) {
                result.push(Loading { game, stage: "start".into(), progress: None, label: "Starting the game".into() });
            }
        }
        result
    }

    pub fn loading_session(&self, game: &str) -> Option<Loading> { self.loading_entries().into_iter().find(|e| e.game == game) }

    /// Ease the shown percentages towards what the host reports.
    pub fn update_loading(&mut self, delta: f32) {
        let mut seen = vec![];
        for session in self.loading_entries() {
            if seen.contains(&session.game) { continue; }
            seen.push(session.game.clone());
            let reported = session.progress.map_or(0.0, |p| p.clamp(0.0, 100.0));
            let mut report = self.load_report.get(&session.game).cloned().unwrap_or((-1.0, self.time, session.stage.clone()));
            if report.2 != session.stage {
                // Downloading and loading are separate bars: each starts at 0.
                report = (-1.0, self.time, session.stage.clone());
                self.load_shown.insert(session.game.clone(), 0.0);
            }
            if reported != report.0 {
                report = (reported, self.time, session.stage.clone());
                self.load_report.insert(session.game.clone(), report.clone());
            }
            // A download reports real bytes, so follow it. Anything else creeps
            // halfway toward 95% over this phase, never reaching it.
            let mut ceiling = reported;
            if session.stage != "download" && reported < 95.0 { ceiling = reported + (95.0 - reported) * 0.5; }
            let estimate = reported + (ceiling - reported) * (1.0 - (-((self.time - report.1) as f64) / 6.0).exp());
            let now = *self.load_shown.get(&session.game).unwrap_or(&0.0) as f64;
            let shown = 99.0f64.min(now.max(now + (estimate - now) * (delta as f64 * 2.5).min(1.0)));
            self.load_shown.insert(session.game, shown as f32);
        }
        self.load_shown.retain(|g, _| seen.contains(g));
        self.load_report.retain(|g, _| seen.contains(g));
    }

    pub fn loading_percent(&self, game: &str) -> i64 { *self.load_shown.get(game).unwrap_or(&0.0) as i64 }
    pub fn loading_shown(&self, game: &str) -> Option<f32> { self.loading_session(game).map(|_| *self.load_shown.get(game).unwrap_or(&0.0)) }

    /// "LOADING 40%", or "DOWNLOADING 40%" while the game is still arriving.
    pub fn loading_kicker(&self, game: &str) -> String {
        let stage = self.loading_session(game).map(|l| l.stage).unwrap_or_default();
        format!("{} {}%", if stage == "download" { "DOWNLOADING" } else { "LOADING" }, self.loading_percent(game))
    }

    pub fn queue_status(&self, game: &str) -> String {
        let issue = self.issue(game);
        if !issue.is_empty() { return issue + " · Press Y here to retry"; }
        if let Some(loading) = self.loading_session(game) { return format!("{}% · {}", self.loading_percent(game), loading.label); }
        let warm = &self.party["warm_session"];
        if dict(warm) && get(warm, "game", "") == game && warm["phase"] == "ready" { return "Ready to play".into(); }
        String::new()
    }

    // ------------------------------------------------------------ stations

    /// What the Up next cabinet shows: with nothing running the TV shows
    /// the first queued game, so the cabinet shows the one after it.
    pub fn cabinet_queue(&self) -> Vec<Entry> {
        let queue = self.upcoming();
        if self.current_game().is_some() { queue } else { queue.into_iter().skip(1).collect() }
    }

    /// The TV: Y resumes the paused game, or starts the next one.
    pub fn tv_station(&self) -> Station {
        let queue = self.upcoming();
        if let Some(active) = self.current_game().filter(|a| self.fits_party(a)) {
            let quit = !queue.is_empty();
            return Station { label: format!("Resume {}", get(&active, "title", "game")), short: "Resume".into(), action: "resume".into(), enabled: true,
                quit, extra: if quit { "X quit".into() } else { String::new() }, ..Default::default() };
        }
        let Some(first) = queue.first() else {
            return Station { label: "No queued games for this party".into(), short: "Play".into(), action: "next".into(), enabled: false, ..Default::default() };
        };
        // Y while the first game loads is remembered: the host starts it once
        // it is ready (pending transition); the TV shows how far it is.
        if self.loading_session(&first.game).is_some() {
            return Station { label: format!("Loading {}", first.title), short: "Play".into(), action: "next".into(), enabled: true, loading: true, title: first.title.clone(), ..Default::default() };
        }
        Station { label: format!("Play {}", first.title), short: "Play".into(), action: "next".into(), enabled: true, ..Default::default() }
    }

    /// The queue cabinet always starts the next game, even while another is paused.
    pub fn queue_station(&self) -> Station {
        let entries = self.cabinet_queue();
        let loading = entries.first().is_some_and(|e| self.loading_session(&e.game).is_some());
        let label = match entries.first() {
            Some(e) if loading => format!("Loading {}", e.title),
            Some(e) => format!("Play {}", e.title),
            None => "Queue is empty".into(),
        };
        Station { id: "queue".into(), kind: "queue".into(), action: "next".into(), short: if loading { "Loading…".into() } else { "Play".into() }, loading,
            label, enabled: !entries.is_empty() && (self.connected || self.demo), ..Default::default() }
    }

    fn unavailable(&self, game: &Value) -> String {
        if !self.connected && !self.demo { return "Waiting for Splitscreen Club".into(); }
        self.shortfall(game)
    }

    /// What `p` can use where they stand, or None.
    pub fn station_for(&self, p: &Player) -> Option<Station> {
        if !p.grounded || p.sleeping { return None; }
        let on_floor = p.y.abs() <= 0.1;
        // Stations follow what the camera shows: where the player appears
        // against the furniture, not where they stand on the play plane.
        let x = l::apparent_x(p.x, 0.0);
        let at_tv_floor = on_floor && x >= l::TV_STATION.0 && x <= l::TV_STATION.1;
        let console = l::TV_CONSOLE;
        let on_tv_console = (p.y - console.top).abs() <= 0.1 && p.x >= console.x0 && p.x <= console.x1;
        if at_tv_floor || on_tv_console { return Some(Station { id: "tv".into(), kind: "tv".into(), ..self.tv_station() }); }
        if !on_floor { return None; }
        self.station_at(x, Some(&p.id))
    }

    /// The station of the furniture at `x` on the furniture plane, for
    /// `player`; None for the mouse at the exit and the doors, which only
    /// work for somebody.
    pub fn station_at(&self, x: f64, player: Option<&str>) -> Option<Station> {
        if x >= l::TV_STATION.0 && x <= l::TV_STATION.1 { return Some(Station { id: "tv".into(), kind: "tv".into(), ..self.tv_station() }); }
        let shelf = self.shelf_games();
        if x >= l::GAME_SHELF.0 && x <= l::GAME_SHELF.1 && !shelf.is_empty() {
            let game = shelf[self.shelf_offset % shelf.len()];
            let reason = self.unavailable(game);
            let title = get(game, "title", "Game");
            return Some(Station { id: "shelf".into(), kind: "shelf".into(), game: get(game, "id", ""), title: title.clone(),
                label: if reason.is_empty() { format!("Play {} now", get(game, "title", "game")) } else { reason.clone() },
                enabled: reason.is_empty(), extra: if shelf.len() > 1 { "X queue LB RB browse".into() } else { "X queue".into() }, ..Default::default() });
        }
        if x >= l::UP_NEXT.0 && x <= l::UP_NEXT.1 { return Some(self.queue_station()); }
        if !self.no_jukebox && (x - l::JUKEBOX).abs() <= 0.85 {
            // Nothing on yet: Y starts the host's last media player, like the Play key.
            if !self.has_track() {
                return Some(Station { id: "music1".into(), kind: "music".into(), label: "Play music".into(), action: "resume".into(), enabled: true, ..Default::default() });
            }
            let playing = self.party["now_playing"]["playing"].as_bool().unwrap_or(false);
            return Some(Station { id: "music1".into(), kind: "music".into(), label: if playing { "Pause music" } else { "Play music" }.into(),
                action: "play_pause".into(), enabled: true, extra: "LB previous RB next".into(), ..Default::default() });
        }
        if self.no_jukebox && (x - l::SWITCH).abs() <= 0.8 {
            return Some(Station { id: "privacy".into(), kind: "privacy".into(), label: if self.public { "Make it private" } else { "Make it public" }.into(),
                action: "toggle_public".into(), enabled: true, ..Default::default() });
        }
        let id = player?;
        if (x - l::EXIT_DOOR).abs() <= 0.6 {
            return Some(Station { id: "exit".into(), kind: "exit".into(), label: "Leave".into(), enabled: true, ..Default::default() });
        }
        let pending = self.pending_profiles();
        for (i, entry) in pending.iter().enumerate().take(l::PROFILE_DOORS.len()) {
            if (x - l::PROFILE_DOORS[i]).abs() <= l::DOOR_HALF {
                // Already someone else: swapping drops that profile, so it takes a hold.
                let linked = self.is_linked(id);
                let name = get(&entry["profile"], "display_name", "Player");
                return Some(Station { id: format!("door{i}"), kind: "door".into(), pending: get(entry, "id", ""),
                    label: if linked { format!("Hold Y to become {name}") } else { format!("Pick up {name}") }, enabled: true, hold: linked, ..Default::default() });
            }
        }
        None
    }

    /// Whether character `id` already is a phone profile. The host sends
    /// `linked` as a list of player ids (older builds: an object by id).
    pub fn is_linked(&self, id: &str) -> bool {
        match &self.links["linked"] {
            Value::Array(ids) => ids.iter().any(|v| v.as_str() == Some(id)),
            Value::Object(map) => map.contains_key(id),
            _ => false,
        }
    }

    /// Profiles waiting at the doors, in arrival order; with more waiting
    /// than doors, the newest arrivals take the doors.
    pub fn pending_profiles(&self) -> Vec<Value> {
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let waiting: Vec<Value> = arr(&self.links["room"]["pending"]).iter().filter(|p| {
            let id = get(p, "id", "");
            if self.claimed_doors.get(&id).is_some_and(|until| self.time <= *until) { return false; }
            match p.get("expires").and_then(Value::as_f64) { Some(e) => e > now || e < 1e9, None => true }
        }).cloned().collect();
        let skip = waiting.len().saturating_sub(l::PROFILE_DOORS.len());
        waiting.into_iter().skip(skip).collect()
    }

    /// Titles after the cabinet's first game: (number, title).
    pub fn queue_title_rows(&self, rows: usize) -> Vec<(usize, String)> {
        let queue = self.cabinet_queue();
        let first = self.upcoming().len() - queue.len();
        (1..queue.len().min(rows + 1)).map(|i| {
            let title = self.game_meta(&queue[i].game).map(|m| get(m, "title", &queue[i].title)).unwrap_or_else(|| queue[i].title.clone());
            (first + i + 1, title)
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn host(party: Value) -> Host { let mut h = Host::new("game-room"); h.party = party; h.connected = true; h }

    fn seats(n: usize) -> Value {
        json!((0..n).map(|i| json!({"index": i, "occupant": {"player_id": format!("p{i}")}, "controller": format!("c{i}")})).collect::<Vec<_>>())
    }

    #[test]
    fn keyboard_only_leaves_controller_games_off_the_shelf() {
        let mut seats = seats(1);
        seats[0]["controller"] = json!("web:keyboard");
        let h = host(json!({
            "seats": seats,
            "library": [{"id": "pads", "title": "Pads"}, {"id": "keys", "title": "Keys", "keyboard": true}, {"id": "mouse", "title": "Mouse", "mouse": true}],
            "playlist": {"entries": [], "current": null},
        }));
        assert!(h.keyboard_only());
        assert_eq!(h.shelf_games().iter().map(|g| get(g, "id", "")).collect::<Vec<_>>(), ["keys", "mouse"]);
        assert_eq!(h.hidden_for_keyboard(), 1);
    }

    #[test]
    fn two_on_one_keyboard_only_see_shared_keyboard_games() {
        let mut seats = seats(2);
        seats[0]["controller"] = json!("web:keyboard-right");
        seats[1]["controller"] = json!("web:keyboard-left");
        let h = host(json!({
            "seats": seats,
            "library": [{"id": "pads", "title": "Pads"}, {"id": "keys", "title": "Keys", "keyboard": true}, {"id": "two", "title": "Two", "keyboard": true, "shared_keyboard": 2}],
            "playlist": {"entries": [], "current": null},
        }));
        assert!(h.keyboard_only());
        assert_eq!(h.shelf_games().iter().map(|g| get(g, "id", "")).collect::<Vec<_>>(), ["two"]);
    }

    #[test]
    fn games_that_do_not_fit_the_party_go_last_on_the_shelf_and_stay_out_of_the_queue() {
        let h = host(json!({
            "seats": seats(3),
            "library": [{"id": "duo", "title": "Duo", "max_players": 2.0}, {"id": "party", "title": "Party"}, {"id": "game-room", "title": "Room"},
                {"id": "phones", "title": "Phones", "play_with": "phone"}],
            "playlist": {"entries": [{"game": "duo", "title": "Duo"}, {"game": "party", "title": "Party"}, {"game": "phones"}], "current": null},
        }));
        assert_eq!(h.shelf_games().iter().map(|g| get(g, "id", "")).collect::<Vec<_>>(), ["party", "duo", "phones"]);
        assert_eq!(h.upcoming(), vec![Entry { index: 1, game: "party".into(), title: "Party".into() }]);
        assert_eq!(h.shortfall(&h.library()[0]), "Too many players");
        assert_eq!(h.shortfall(&h.library()[3]), "Needs 1 phones");
        // Nothing runs: the TV starts the first game and the cabinet shows what follows.
        assert_eq!(h.tv_station().label, "Play Party");
        assert!(h.cabinet_queue().is_empty());
        assert_eq!(h.queue_station().label, "Queue is empty");
    }

    #[test]
    fn up_next_wraps_round_from_the_warming_game_and_waits_for_players_before_duos() {
        // The browser host moved the game it warms to the back: Up next showed
        // only that one until it had been played (28 November 2026).
        let lib = json!([{"id": "duo", "title": "Duo", "min_players": 2}, {"id": "a", "title": "Alpha"}, {"id": "b", "title": "Beta"}, {"id": "gg", "title": "GG"}]);
        let list = json!({"entries": [{"game": "duo"}, {"game": "a"}, {"game": "b"}, {"game": "gg"}], "current": null});
        let ids = |h: &Host| h.upcoming().into_iter().map(|e| e.game).collect::<Vec<_>>();
        // Nobody here: not the game for two.
        let h = host(json!({"seats": seats(0), "library": lib, "playlist": list, "warm_session": {"game": "gg", "phase": "preparing"}}));
        assert_eq!(ids(&h), ["gg", "a", "b"]);
        // Two people: everything, from the warming game round.
        let h = host(json!({"seats": seats(2), "library": lib, "playlist": list, "warm_session": {"game": "gg", "phase": "preparing"}}));
        assert_eq!(ids(&h), ["gg", "duo", "a", "b"]);
        // A playing entry doesn't come round again.
        let h = host(json!({"seats": seats(2), "library": lib, "playlist": {"entries": list["entries"], "current": 1}}));
        assert_eq!(ids(&h), ["b", "gg", "duo"]);
    }

    #[test]
    fn a_paused_game_resumes_at_the_tv_and_x_quits_when_something_follows() {
        let h = host(json!({
            "seats": seats(1),
            "library": [{"id": "a", "title": "Alpha"}, {"id": "b", "title": "Beta"}],
            "playlist": {"entries": [{"game": "a", "title": "Alpha"}, {"game": "b", "title": "Beta"}], "current": 0.0},
            "active_session": {"game": "a", "phase": "paused"},
        }));
        let tv = h.tv_station();
        assert_eq!((tv.label.as_str(), tv.quit, tv.extra.as_str()), ("Resume Alpha", true, "X quit"));
        assert_eq!(h.cabinet_queue()[0].game, "b");
        assert_eq!(h.queue_station().label, "Play Beta");
    }

    #[test]
    fn loading_creeps_towards_95_percent_and_downloads_follow_the_bytes() {
        let mut h = host(json!({"warm_session": {"game": "a", "phase": "preparing"}, "installs": [{"game": "b", "state": "downloading", "percent": 40.0}]}));
        for _ in 0..600 { h.time += 1.0 / 60.0; h.update_loading(1.0 / 60.0); }
        assert!(h.loading_percent("a") > 20 && h.loading_percent("a") < 48, "{}", h.loading_percent("a"));
        assert_eq!(h.loading_percent("b"), 39);
        assert_eq!(h.loading_kicker("b"), "DOWNLOADING 39%");
        assert_eq!(h.queue_status("a"), format!("{}% · Preparing game", h.loading_percent("a")));
    }

    #[test]
    fn stations_follow_where_the_player_appears() {
        let mut h = host(json!({"seats": seats(1), "library": [{"id": "a", "title": "Alpha"}]}));
        h.links = json!({"room": {"pending": [{"id": "x", "profile": {"display_name": "Sanne"}}]}, "linked": ["p0"]});
        let mut w = game_room_sim::world::World::new(1);
        w.ensure_player("p0", 0);
        let mut at = |x: f64| { let p = w.players.iter_mut().next().unwrap(); p.x = x; p.y = 0.0; p.grounded = true; h.station_for(p).map(|s| (s.id, s.label, s.hold)) };
        assert_eq!(at(0.0).unwrap().0, "tv");
        assert_eq!(at(l::stand_x(-9.6)).unwrap().1, "Play Alpha now");
        assert_eq!(at(l::stand_x(l::JUKEBOX)).unwrap().1, "Play music");
        assert_eq!(at(l::stand_x(l::EXIT_DOOR)).unwrap().0, "exit");
        assert_eq!(at(l::stand_x(l::PROFILE_DOORS[0])), Some(("door0".into(), "Hold Y to become Sanne".into(), true)));
        assert_eq!(at(l::stand_x(7.0)), None);
    }

    #[test]
    fn hardware_notes_from_the_tablet_bench() {
        // The 25 November bench on the Tab S9+ (hardware-fit.json).
        let fit: Value = serde_json::from_str(include_str!("../hardware-fit.json")).unwrap();
        let tab = &fit["measured"]["Adreno (TM) 740"]["games"];
        let game = |id: &str| json!({"id": id, "bench": {"Adreno (TM) 740": tab[id]}});
        let mut h = host(json!({}));
        assert_eq!(h.hardware_note(&game("spellstorm")), "", "no device: no note");
        h.device = Some(HostDevice { gpu: "Adreno (TM) 740".into(), size: [2800, 1752], phone: true });
        assert_eq!(ladder_floor(h.device.as_ref().unwrap()), 723);
        for light in ["splitscreen-club", "neon-siege", "stack-together"] { assert_eq!(h.hardware_note(&game(light)), "", "{light}"); }
        for heavy in ["spellstorm", "dead-miles"] { assert_eq!(h.hardware_note(&game(heavy)), "May stutter here", "{heavy}"); }
        assert!(!h.runs_smoothly(&game("spellstorm")));
        // At 30 fps Spellstorm's floor fits (18.5 + 43.8 ns x 836k px = 55 ms > 36.7: still not).
        assert_eq!(h.hardware_note(&json!({"bench": {"Adreno (TM) 740": tab["spellstorm"]}, "frame_target_hz": 30})), "May stutter here");
        // Another GPU, or a browser that hides its name: nothing measured, no note.
        h.device = Some(HostDevice { gpu: "Apple M5 Pro".into(), size: [3024, 1964], phone: false });
        assert_eq!(h.hardware_note(&game("spellstorm")), "");
        h.device = Some(HostDevice { gpu: String::new(), size: [3024, 1964], phone: false });
        assert_eq!(h.hardware_note(&game("spellstorm")), "");
    }
}
