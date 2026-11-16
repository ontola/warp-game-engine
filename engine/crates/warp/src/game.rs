//! A game folder as an `app::Game`: the Luau modules' hooks at a fixed
//! tick, the world drawn from the entities, hot reload of scripts and
//! sound patches, debug commands, live control over MCP, the players (from
//! a GameNight party host when one launched the game) and the start menu.
use crate::api::{self, Cam, Core, HudItem, Phase, Player, Requests, Rng, Shared};
use crate::party::{self, Party, SeatInfo};
use crate::config::GameToml;
use crate::{knobs, luajson};
use crate::sky::SkyChoice;
use crate::sounds::{check_params, Bank, SoundOut};
use crate::surface::GameSurface;
use crate::world::World;
use app::{Buttons, Controls, Draw, Frame, Game};
use audio::synth::{Ear, SoundSender};
use glam::Vec3;
use render::atmosphere::{AtmosphereCache, BakeQuality};
use render::hud::{FontFace, HudDrawList, TextRun};
use render::sky::{bake_live, PreparedSky};
use render::{primitives, Camera, Lighting};
use script::mlua::{self, Function, Table, Value};
use script::{Host, Limits};
use serde_json::{json, Value as Json};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Top-level folders of a game that hold no Luau modules.
pub const NOT_MODULES: &[&str] = &["sounds", "assets", "scripts"];

/// How often (in ticks) files are checked for edits: about 4 times a second at 60 Hz.
const POLL_TICKS: u64 = 15;
/// Seconds of play before Start opens the menu (under a party host).
const START_GRACE: f32 = 1.0;

/// Options from the command line that override `game.toml`.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub players: Option<usize>,
    pub bots: Option<usize>,
    pub seed: Option<u64>,
    pub sky: Option<String>,
}

pub struct LuaGame {
    pub dir: PathBuf,
    /// A one-file game: its file name in `dir` (the only module).
    pub single: Option<String>,
    pub cfg: GameToml,
    host: Host,
    core: Shared,
    /// Modules whose `init` has run (by name), so a reload does not re-run it.
    inited: BTreeSet<String>,
    /// The light of the applied sky, before `light.set` overrides.
    light: Lighting,
    applied_sky: Option<SkyChoice>,
    sky_upload: Option<PreparedSky>,
    log: VecDeque<String>,
    pub live: Option<live::Live>,
    /// The `logs` tool's buffer (usable while `live` is out serving).
    live_logs: Option<live::Logs>,
    seed: u64,
    default_sky: SkyChoice,
    /// Print log lines to stdout (off for tests that read `logs()`).
    pub echo: bool,
    /// The GameNight party host that launched the game, if any.
    pub party: Option<Party>,
    /// The start menu, when open: the selected item.
    menu: Option<usize>,
    /// Stick position in the menu last tick, for one step per push.
    menu_stick: f32,
    /// Ticks the party has been running since its start or resume.
    running_for: u32,
    /// A `prepare` was handled; tell the host ready after the next frame.
    ready_due: bool,
    /// A window shell reported startup ([`app::Game::loading`]).
    windowed: bool,
    /// Ticks seen, for the file checks (the game's own tick stops while paused).
    polls: u64,
    /// The next player id: ids are never reused in a session.
    next_id: u32,
    quit: bool,
    /// The game's own surface shader (surface.wgsl).
    surface: GameSurface,
    /// main.luau as it runs after temporary knob changes (not saved yet).
    trial: Option<String>,
    editor_pages: (Vec<render::debug_ui::DebugMenu>, Vec<render::debug_ui::DebugControl>),
}

/// The start menu's items: Joep's convention for every GameNight game
/// (the title on top, Resume, then New round; no button hints).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem { Resume, NewRound, Quit }

impl MenuItem {
    pub fn label(self) -> &'static str { match self { MenuItem::Resume => "Resume", MenuItem::NewRound => "New round", MenuItem::Quit => "Quit" } }
}

const LOG_KEEP: usize = 200;

impl LuaGame {
    /// Load a game folder, or a one-file game (a `.luau` file whose `--!`
    /// header holds its settings; `sounds/` beside it still holds patches).
    /// `sound` is where sounds go (the speakers, an offline mixer, or None
    /// for silence).
    pub fn load(path: &Path, over: &Overrides, sound: Option<SoundSender>) -> Result<Self, String> {
        Self::load_with_storage(path, over, sound, crate::storage::Location::Memory)
    }

    pub fn load_with_storage(path: &Path, over: &Overrides, sound: Option<SoundSender>, location: crate::storage::Location<'_>) -> Result<Self, String> {
        // A folder whose only script is its game (`my-game/my-game.luau`).
        let found = if path.is_dir() { crate::config::one_file_in(path) } else { None };
        let path = found.as_deref().unwrap_or(path);
        let single = path.is_file().then(|| path.file_name().map(|n| n.to_string_lossy().into_owned())).flatten();
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let dir = if single.is_some() { parent } else { path };
        let mut cfg = if single.is_some() { GameToml::load_file(path)? } else { GameToml::load(dir)? };
        if let Some(p) = over.players {
            if !(1..=4).contains(&p) { return Err(format!("--players {p}: 1 to 4")); }
            cfg.players = p;
        }
        if let Some(b) = over.bots {
            if b > 7 { return Err(format!("--bots {b}: 0 to 7")); }
            cfg.bots = b;
        }
        if let Some(s) = over.seed { cfg.seed = s; }
        let default_sky = SkyChoice::parse(over.sky.as_deref().unwrap_or(&cfg.sky))?;
        let sounds_dir = dir.join("sounds");
        let mut bank = Bank::new(sounds_dir.is_dir().then_some(sounds_dir));
        let lines = bank.scan();
        let mut world = World::new(dir, cfg.max_entities);
        world.scene.style = crate::surface::load_style(dir)?;
        let core: Shared = Arc::new(Mutex::new(Core {
            storage: if cfg.storage { Some(crate::storage::Storage::open(dir, &cfg.id, location).map_err(|e| format!("storage: {e}"))?) } else { None },
            world,
            plants: Default::default(),
            sounds: SoundOut::new(bank, sound),
            roster: Vec::new(),
            hud: Vec::new(),
            screen: cfg.size,
            views: Vec::new(),
            rng: Rng(cfg.seed),
            sky: default_sky,
            light: Default::default(),
            post: Default::default(),
            lamps: Vec::new(),
            spots: Vec::new(),
            humans: crate::humans::Humans::new(&cfg.humans),
            tick: 0,
            tick_rate: cfg.tick_rate,
            title: cfg.title.clone(),
            requests: Requests::default(),
            shared_view: !cfg.split,
            phase: Phase::Sim,
        }));
        let shared = core.clone();
        let mut host = Host::new(Limits { memory: cfg.memory_mb << 20, steps: cfg.steps }, move |lua, base| api::register(lua, base, shared))?;
        host.ignore(NOT_MODULES);
        if single.is_none() { host.enable_libraries(); }
        let mut g = LuaGame { dir: dir.to_path_buf(), single: single.clone(), seed: cfg.seed, cfg, host, core, inited: BTreeSet::new(), light: Lighting::default(), applied_sky: None, sky_upload: None,
            log: VecDeque::new(), live: None, live_logs: None, default_sky, echo: true, party: None, menu: None, menu_stick: 0.0, running_for: 0, ready_due: false, windowed: false, polls: 0, next_id: 1, quit: false, surface: GameSurface::new(dir), trial: None, editor_pages: Default::default() };
        g.surface.install();
        let people: Vec<SeatInfo> = (0..g.cfg.players).map(|i| SeatInfo { seat: i, name: format!("Player {}", i + 1), color: None, bot: false, couch: None })
            .chain((0..g.cfg.bots).map(|i| SeatInfo { seat: 100 + i, name: format!("Bot {}", i + 1), color: None, bot: true, couch: None })).collect();
        g.seat_all(&people, false);
        for l in lines { g.say(l); }
        if let Some(name) = &single { g.host.only(name); }
        g.host.watch(dir.to_path_buf());
        g.flush_log();
        g.inline_sounds();
        g.start_new();
        g.load_editor_pages();
        Ok(g)
    }

    pub fn core(&self) -> std::sync::MutexGuard<'_, Core> { api::lock(&self.core) }

    fn say(&mut self, line: String) {
        if self.echo { println!("{line}"); }
        self.log.push_back(line.clone());
        while self.log.len() > LOG_KEEP { self.log.pop_front(); }
        if let Some(l) = &self.live_logs { l.push(line); }
    }

    fn flush_log(&mut self) { for l in self.host.drain_log() { self.say(l); } }

    /// Recent log lines: prints, loads, errors, reloads.
    pub fn logs(&self) -> Vec<String> { self.log.iter().cloned().collect() }

    /// Run `hook` in every module that exports it, in path order.
    fn hook(&mut self, hook: &str, args: impl Fn() -> mlua::Result<mlua::MultiValue>) {
        for (m, f) in self.host.each(hook) { self.call_one(m, hook, &f, &args); }
    }

    fn call_one(&mut self, m: usize, hook: &str, f: &Function, args: &impl Fn() -> mlua::Result<mlua::MultiValue>) {
        self.host.run(m, hook, |_| f.call::<()>(args()?));
        self.flush_log();
    }

    /// Sound patches the modules export as source (`sounds = { name = [[...]] }`).
    fn inline_sounds(&mut self) {
        let found: Vec<(String, String)> = self.host.modules().iter().filter_map(|m| m.exports.as_ref())
            .filter_map(|e| e.raw_get::<Option<mlua::Table>>("sounds").ok().flatten())
            .flat_map(|t| t.pairs::<String, String>().filter_map(|kv| kv.ok()).collect::<Vec<_>>()).collect();
        let lines: Vec<String> = { let mut c = self.core(); found.iter().filter_map(|(n, src)| c.sounds.bank.add_source(n, src)).collect() };
        for l in lines { self.say(l); }
    }

    /// `init` for every loaded module that has not had it yet.
    fn start_new(&mut self) {
        let fresh: Vec<(usize, String)> = self.host.modules().iter().enumerate()
            .filter(|(_, m)| m.exports.is_some() && !self.inited.contains(&m.name)).map(|(i, m)| (i, m.name.clone())).collect();
        for (i, name) in fresh {
            self.inited.insert(name);
            let f = self.host.modules()[i].exports.as_ref().and_then(|e| e.raw_get::<Option<Function>>("init").ok().flatten());
            if let Some(f) = f { self.call_one(i, "init", &f, &|| Ok(mlua::MultiValue::new())); }
        }
    }

    /// Reload edited scripts and sound patches now. Returns what changed.
    pub fn reload(&mut self) -> Vec<String> {
        let mut lines = self.core().sounds.bank.scan();
        lines.extend(self.surface.scan());
        let changed = self.host.poll();
        self.flush_log();
        if !changed.is_empty() { self.inline_sounds(); }
        // An edit on disk wins over temporary changes.
        if changed.iter().any(|n| *n == self.main_name()) { self.trial = None; }
        // Changed modules that already ran init get `reload()` instead.
        for name in &changed {
            let Some(i) = self.host.modules().iter().position(|m| &m.name == name) else { continue };
            let m = &self.host.modules()[i];
            if m.error.is_some() || !self.inited.contains(name) { continue; }
            if let Some(f) = m.exports.as_ref().and_then(|e| e.raw_get::<Option<Function>>("reload").ok().flatten()) {
                self.call_one(i, "reload", &f, &|| Ok(mlua::MultiValue::new()));
            }
        }
        self.inited.retain(|n| self.host.modules().iter().any(|m| &m.name == n));
        self.start_new();
        if changed.iter().any(|n| *n == self.main_name()) { self.load_editor_pages(); }
        lines.extend(changed.iter().map(|n| match self.host.modules().iter().find(|m| &m.name == n) {
            Some(m) => match &m.error { Some(e) => format!("{n}: {e}"), None => format!("{n}: reloaded") },
            None => format!("{n}"),
        }));
        for l in &lines { if l.starts_with("sounds/") { self.say(l.clone()); } }
        lines
    }

    /// Start over: clear the world and every module's `state`, reseed, run `init`.
    pub fn restart(&mut self) {
        {
            let mut c = self.core();
            c.world.clear();
            c.plants.clear();
            c.sounds.stop_all();
            c.rng = Rng(self.seed);
            c.tick = 0;
            c.sky = self.default_sky;
            c.light = Default::default();
            c.post = Default::default();
            c.lamps.clear();
            c.spots.clear();
            c.humans.clear();
            for p in &mut c.roster { p.cam = Cam::default(); }
        }
        self.menu = None;
        for m in self.host.modules() { let _ = m.state().clear(); }
        self.inited.clear();
        self.say("restarted".into());
        self.start_new();
    }

    /// The GameNight game id: game.toml's `id`, else the folder name.
    pub fn game_id(&self) -> String { if self.cfg.id.is_empty() { self.live_name() } else { self.cfg.id.clone() } }

    /// Connect to the party host that launched the game (GAMENIGHT=1 or
    /// GAMENIGHT_ADDR). The game then waits for the host's `prepare`.
    pub fn connect_party(&mut self) -> bool {
        self.party = Party::connect(&self.game_id());
        if self.party.is_some() { self.say(format!("gamenight: waiting for the party host ({})", self.game_id())); }
        self.party.is_some()
    }

    /// Players for these seats (people first, then bots), filled up with
    /// bots to `min_players`. `seated` keeps the GameNight seat numbers.
    fn seat_all(&mut self, seats: &[SeatInfo], seated: bool) {
        // A new session numbers its players from 1 again.
        self.next_id = 1;
        let mut roster = Vec::new();
        let mut order: Vec<&SeatInfo> = seats.iter().filter(|s| !s.bot).take(4).collect();
        order.extend(seats.iter().filter(|s| s.bot));
        for s in order {
            let mut p = Player::new(self.next_id, s.name.clone(), s.color, s.bot);
            self.next_id += 1;
            if seated { p.seat = Some(s.seat); }
            p.couch = s.couch;
            roster.push(p);
        }
        let mut n = roster.iter().filter(|p| p.bot).count();
        while roster.len() < self.cfg.min_players {
            n += 1;
            roster.push(Player::new(self.next_id, format!("Bot {n}"), None, true));
            self.next_id += 1;
        }
        roster.truncate(8);
        self.core().roster = roster;
        self.assign_views();
    }

    /// People look through the split-screen views in roster order. Without
    /// people, the first player's camera fills the screen (a bots-only demo).
    fn assign_views(&mut self) {
        let mut c = self.core();
        let mut k = 0;
        let split = !c.shared_view;
        for p in &mut c.roster { p.view = (!p.bot && (split || k == 0)).then(|| { k += 1; k - 1 }); }
        if k == 0 && let Some(p) = c.roster.first_mut() { p.view = Some(0); }
    }

    fn views(&self) -> usize { self.core().roster.iter().filter(|p| p.view.is_some()).count().max(1) }

    /// The party's seats changed mid-session: names and colours follow,
    /// a new seat joins (`player_joined(p)`), a gone seat leaves
    /// (`player_left(p)`), and `players_changed()` runs after any change.
    fn reseat(&mut self, seats: &[SeatInfo]) {
        let (mut joined, mut left) = (Vec::new(), Vec::new());
        let mut changed = false;
        {
            let mut c = api::lock(&self.core);
            for p in &mut c.roster {
                let Some(seat) = p.seat else { continue };
                if let Some(s) = seats.iter().find(|s| s.seat == seat) {
                    let color = s.color.unwrap_or(p.color);
                    if p.name != s.name || p.color != color || p.couch != s.couch { changed = true; }
                    p.name = s.name.clone();
                    p.color = color;
                    p.couch = s.couch;
                }
            }
            let gone: Vec<usize> = c.roster.iter().enumerate().filter(|(_, p)| p.seat.is_some_and(|seat| !seats.iter().any(|s| s.seat == seat))).map(|(i, _)| i).collect();
            for i in gone.into_iter().rev() { left.push(c.roster.remove(i)); }
            let people = c.roster.iter().filter(|p| !p.bot).count();
            for s in seats.iter().filter(|s| !c.roster.iter().any(|p| p.seat == Some(s.seat))) {
                if !s.bot && people + joined.iter().filter(|p: &&Player| !p.bot).count() >= 4 { continue; }
                let mut p = Player::new(self.next_id, s.name.clone(), s.color, s.bot);
                self.next_id += 1;
                p.seat = Some(s.seat);
                p.couch = s.couch;
                joined.push(p);
            }
            for p in &joined {
                let at = if p.bot { c.roster.len() } else { c.roster.iter().position(|q| q.bot).unwrap_or(c.roster.len()) };
                c.roster.insert(at, p.clone());
            }
        }
        self.assign_views();
        let lua = self.host.lua().clone();
        for p in &left {
            if let Ok(t) = api::player_table(&lua, p) { self.hook("player_left", || Ok(mlua::MultiValue::from_vec(vec![Value::Table(t.clone())]))); }
        }
        for p in &joined {
            if let Ok(t) = api::player_table(&lua, p) { self.hook("player_joined", || Ok(mlua::MultiValue::from_vec(vec![Value::Table(t.clone())]))); }
        }
        if changed || !joined.is_empty() || !left.is_empty() { self.hook("players_changed", || Ok(mlua::MultiValue::new())); }
    }

    /// GameNight's lifecycle. None without a party; Some(None) while the
    /// game waits or is paused by the host; else the seats' controls.
    fn party_tick(&mut self, dt: f32) -> Option<Option<Vec<Controls>>> {
        let evs = self.party.as_mut()?.poll();
        for ev in evs {
            match ev {
                party::Ev::Prepare(seats) => {
                    self.seat_all(&seats, true);
                    self.restart();
                    // In a window, ready once a frame of the new match is
                    // drawn: the scene is built then, so the host never shows
                    // a game still loading. Headless, at once.
                    if self.windowed { self.ready_due = true; } else { self.party.as_mut().expect("party").ready(); }
                }
                party::Ev::Seats(seats) => self.reseat(&seats),
                party::Ev::Dispose => {
                    self.menu = None;
                    let mut c = self.core();
                    c.world.clear();
                    c.sounds.stop_all();
                }
            }
        }
        let seats: Vec<usize> = self.core().roster.iter().filter(|p| !p.bot).map(|p| p.seat.unwrap_or(usize::MAX)).collect();
        let party = self.party.as_mut()?;
        self.running_for = if party.running() { self.running_for.saturating_add(1) } else { 0 };
        Some(party.running().then(|| party.controls(&seats, dt, false)))
    }

    pub fn menu_items(&self) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::Resume, MenuItem::NewRound];
        // Under GameNight the host's overlay (Back) leaves the game.
        if self.party.is_none() { items.push(MenuItem::Quit); }
        items
    }

    pub fn menu_open(&self) -> bool { self.menu.is_some() }

    pub fn open_menu(&mut self) { self.menu = Some(0); self.menu_stick = 0.0; }

    fn close_menu(&mut self) { self.menu = None; }

    fn choose(&mut self, item: MenuItem) {
        self.close_menu();
        match item {
            MenuItem::Resume => {}
            MenuItem::NewRound => self.restart(),
            MenuItem::Quit => self.quit = true,
        }
    }

    /// Start opens the menu; in the menu the game is frozen and any
    /// person's controller moves the selection. True while the menu has
    /// the controls (the game does not tick).
    fn menu_tick(&mut self, controls: &[Controls]) -> bool {
        let pressed = |b: u32| controls.iter().any(|c| c.pressed.has(b));
        let Some(sel) = self.menu else {
            // Not in the first moments of play: a Start tapped in the lobby
            // or while the game was loading must not open the game on its menu.
            let settled = self.party.is_none() || self.running_for as f32 >= START_GRACE * self.cfg.tick_rate;
            if pressed(Buttons::START) && settled {
                let who = controls.iter().position(|c| c.pressed.has(Buttons::START)).unwrap_or(0);
                self.say(format!("{} menu: opened by Start (player {})", app::manners::stamp(), who + 1));
                self.open_menu();
                return true;
            }
            return false;
        };
        let items = self.menu_items();
        let stick = controls.iter().map(|c| c.mv[1]).fold(0.0f32, |a, y| if y.abs() > a.abs() { y } else { a });
        let (up, down) = (pressed(Buttons::UP) || (stick > 0.5 && self.menu_stick <= 0.5), pressed(Buttons::DOWN) || (stick < -0.5 && self.menu_stick >= -0.5));
        self.menu_stick = stick;
        let n = items.len();
        if up { self.menu = Some((sel + n - 1) % n); }
        if down { self.menu = Some((sel + 1) % n); }
        if pressed(Buttons::SOUTH) { self.choose(items[self.menu.unwrap_or(0)]); }
        else if pressed(Buttons::EAST) || pressed(Buttons::START) { self.close_menu(); }
        true
    }

    /// Run a menu item by label (`menu resume`, `menu new round`) for scripts and agents.
    fn menu_command(&mut self, what: &str) -> Result<String, String> {
        match what.trim() {
            "" | "open" => { self.say(format!("{} menu: opened by a command", app::manners::stamp())); self.open_menu(); Ok("menu open".into()) }
            "close" => { self.close_menu(); Ok("menu closed".into()) }
            w => {
                let item = self.menu_items().into_iter().find(|i| i.label().eq_ignore_ascii_case(w)).ok_or_else(|| format!("menu {w}: one of open, close, {}", self.menu_items().iter().map(|i| i.label().to_lowercase()).collect::<Vec<_>>().join(", ")))?;
                self.choose(item);
                Ok(format!("{}", item.label()))
            }
        }
    }

    /// Hand the scripts' requests to the party host (or the log without one).
    fn send_requests(&mut self) {
        let r = std::mem::take(&mut self.core().requests);
        if r == Requests::default() { return; }
        let seats: Vec<(u32, Option<usize>)> = self.core().roster.iter().map(|p| (p.id, p.seat)).collect();
        for l in &r.logs { self.say(format!("room log: {l}")); }
        if r.finish { self.say("game.finish: the match is over".into()); }
        let Some(party) = self.party.as_mut() else { return };
        for l in &r.logs { party.room_log(l); }
        for (id, weak, strong, secs) in r.rumble {
            if let Some((_, Some(seat))) = seats.iter().find(|(i, _)| *i == id) { party.rumble(*seat, weak, strong, secs); }
        }
        if r.finish { party.finished(); }
    }

    fn main_state(&self) -> Option<Table> {
        let m = self.host.modules();
        m.iter().find(|m| m.name == "main.luau").or(m.first()).map(|m| m.state().clone())
    }

    /// Errors to show: load errors and modules whose hooks are off.
    pub fn errors(&self) -> Vec<String> {
        let mut out: Vec<String> = self.host.modules().iter().filter_map(|m| m.error.clone().or_else(|| m.broken.then(|| format!("{}: a hook failed; its hooks are off until the file changes (see logs)", m.name)))).collect();
        out.extend(self.core().sounds.bank.errors.iter().map(|(f, e)| match f.strip_prefix("inline:") { Some(n) => format!("sound {n}: {e}"), None => format!("sounds/{f}: {e}") }));
        out.extend(self.surface.error.clone());
        if self.host.modules().is_empty() { out.push("no Luau modules loaded".into()); }
        out
    }

    pub fn state_json(&self) -> Json {
        let errors = self.errors();
        let game = self.main_state().map(|t| luajson::to_json(&Value::Table(t))).unwrap_or(Json::Null);
        let modules: Vec<Json> = self.host.modules().iter().map(|m| json!({ "name": m.name, "error": m.error, "broken": m.broken })).collect();
        let c = self.core();
        let mut tags = std::collections::BTreeMap::<&str, u32>::new();
        for e in c.world.entities.values() { *tags.entry(e.tag.as_str()).or_default() += 1; }
        let cams: Vec<Json> = c.roster.iter().filter(|p| p.view.is_some()).map(|p| p.cam).map(|k| json!({ "pos": k.pos.to_array(), "target": k.target.to_array(), "fov": k.fov })).collect();
        let players: Vec<Json> = c.roster.iter().map(|p| json!({ "id": p.id, "name": p.name, "color": p.color, "bot": p.bot, "seat": p.seat, "couch": p.couch, "view": p.view })).collect();
        let phase = match self.party.as_ref().map(|p| p.phase) {
            None => "standalone", Some(party::Phase::Idle) => "waiting", Some(party::Phase::Warm) => "ready", Some(party::Phase::Running) => "running", Some(party::Phase::Paused) => "paused",
        };
        let menu = self.menu.map(|i| json!({ "items": self.menu_items().iter().map(|m| m.label()).collect::<Vec<_>>(), "selected": i }));
        let sounds = json!({ "loaded": c.sounds.bank.patches.keys().collect::<Vec<_>>(), "played": c.sounds.played,
            "recent": c.sounds.recent.iter().map(|(n, t)| json!({ "sound": n, "tick": t })).collect::<Vec<_>>() });
        json!({
            "title": c.title, "tick": c.tick, "time": c.tick as f64 / c.tick_rate as f64, "players": players, "phase": phase, "menu": menu,
            "sky": c.sky.name(), "entities": c.world.entities.len(), "max_entities": c.world.capacity(), "tags": tags,
            "cameras": cams, "sounds": sounds, "modules": modules, "errors": errors, "state": game,
        })
    }

    /// One Luau chunk in the console, with `state` bound to main.luau's.
    pub fn luau(&mut self, code: &str) -> Result<String, String> {
        if let Some(s) = self.main_state() { let _ = self.host.console().raw_set("state", s); }
        let main = { let m = self.host.modules(); m.iter().position(|m| m.name == "main.luau").or((!m.is_empty()).then_some(0)) };
        self.host.console_in(main).map_err(|e| script::short(&e))?;
        let f = self.host.console_line(code)?;
        let r = f.call::<mlua::MultiValue>(()).map_err(|e| script::short(&e));
        self.flush_log();
        let vals = r?;
        Ok(if vals.is_empty() { "ok".into() } else { vals.iter().map(luajson::show).collect::<Vec<_>>().join("\t") })
    }

    fn load_editor_pages(&mut self) {
        use mlua::LuaSerdeExt;
        let name = self.main_name();
        let result = (|| {
            let Some(exports) = self.host.modules().iter().find(|m| m.name == name).and_then(|m| m.exports.as_ref()) else { return Ok(Default::default()); };
            let value = exports.raw_get::<mlua::Value>("editor_pages").map_err(|e| e.to_string())?;
            if value.is_nil() { return Ok(Default::default()); }
            let pages: Vec<crate::editor_menus::Page> = self.host.lua().from_value(value).map_err(|e| e.to_string())?;
            if exports.raw_get::<Option<Function>>("editor_action").map_err(|e|e.to_string())?.is_none() {
                return Err("export an editor_action(id) handler alongside editor_pages".into());
            }
            crate::editor_menus::build(pages)
        })();
        self.editor_pages = result.unwrap_or_else(|e| { self.say(format!("editor_pages: {e}")); Default::default() });
    }

    /// The main script file (a one-file game's own file).
    fn main_file(&self) -> PathBuf { self.dir.join(self.single.as_deref().unwrap_or("main.luau")) }

    /// The game's knobs (see [`crate::knobs`]) with their current values.
    fn main_name(&self) -> String { self.single.clone().unwrap_or_else(|| "main.luau".into()) }

    /// main.luau as it runs: with temporary changes, else the file.
    fn main_source(&self) -> Result<String, String> {
        match &self.trial { Some(t) => Ok(t.clone()), None => std::fs::read_to_string(self.main_file()).map_err(|e| e.to_string()) }
    }

    pub fn knobs(&self) -> Result<String, String> {
        let list = knobs::list(&knobs::parse(&self.main_source()?));
        Ok(if self.trial.is_some() { format!("{list}\n(some values are temporary: `keep` saves them in {}, `revert` undoes them)", self.main_name()) } else { list })
    }

    /// Run a module's `reload()` hook (after a load that did not come from
    /// a file poll).
    fn reloaded(&mut self, name: &str) {
        let Some(i) = self.host.modules().iter().position(|m| m.name == name) else { return };
        let m = &self.host.modules()[i];
        if m.error.is_some() || !self.inited.contains(name) { return; }
        if let Some(f) = m.exports.as_ref().and_then(|e| e.raw_get::<Option<Function>>("reload").ok().flatten()) {
            self.call_one(i, "reload", &f, &|| Ok(mlua::MultiValue::new()));
        }
    }

    /// Change a knob and reload. `persist` writes it into the file (ready
    /// to commit); otherwise only the running game changes, until `keep`
    /// saves or `revert` undoes it. A value that does not load is put back.
    pub fn set_knob(&mut self, name: &str, value: &str, persist: bool) -> Result<String, String> {
        let main = self.main_name();
        let before = self.main_source()?;
        let after = knobs::set(&before, name, value)?;
        if !persist {
            if let Err(err) = self.host.load_text(&main, &after) {
                let _ = self.host.load_text(&main, &before);
                self.flush_log();
                return Err(format!("{name} = {value} does not load, kept the old value: {err}"));
            }
            self.flush_log();
            self.trial = Some(after);
            self.reloaded(&main);
            self.load_editor_pages();
            let broken = self.errors();
            return Ok(format!("{name} = {} (temporary: the game runs it, {main} is unchanged; `keep` saves, `revert` undoes){}", value.trim(), if broken.is_empty() { String::new() } else { format!("; {}", broken.join("; ")) }));
        }
        let file = self.main_file();
        let on_disk = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        std::fs::write(&file, &after).map_err(|e| e.to_string())?;
        // 0.55 -> 0.28 keeps the size, and the time may not have moved yet.
        self.host.forget(&main);
        let lines = self.reload();
        if let Some(err) = self.host.modules().iter().find(|m| m.name == main).and_then(|m| m.error.clone()) {
            std::fs::write(&file, &on_disk).map_err(|e| e.to_string())?;
            self.host.forget(&main);
            self.reload();
            return Err(format!("{name} = {value} does not load, kept the old value: {err}"));
        }
        let broken = self.errors();
        Ok(format!("{name} = {} (saved in {main}; {})", value.trim(), if broken.is_empty() { lines.join("; ") } else { broken.join("; ") }))
    }

    /// Save temporary changes into the file.
    pub fn keep(&mut self) -> Result<String, String> {
        let Some(t) = self.trial.take() else { return Ok("nothing temporary to keep".into()) };
        std::fs::write(self.main_file(), &t).map_err(|e| e.to_string())?;
        let main = self.main_name();
        self.host.forget(&main);
        self.reload();
        Ok(format!("saved the temporary changes in {main}"))
    }

    /// Undo temporary changes: run the file again.
    pub fn revert(&mut self) -> Result<String, String> {
        if self.trial.take().is_none() { return Ok("nothing temporary to undo".into()); }
        let main = self.main_name();
        self.host.forget(&main);
        self.reload();
        Ok(format!("back to {main} as saved"))
    }

    pub fn help() -> &'static str {
        "commands: lua CODE (run Luau against the game; `state` is main.luau's, and main.luau's globals and exports are in scope) | reload (scripts and sounds now) | restart (clear the world and state, run init) | sky NAME | sounds | entities [TAG] | menu [open|close|resume|new round] | knobs | knob NAME VALUE (saved in main.luau) | trial NAME VALUE (only the running game) | keep | revert | logs | help"
    }

    fn input(&self, lua: &mlua::Lua, controls: &[Controls]) -> mlua::Result<Table> {
        const NAMES: &[(&str, u32)] = &[
            ("south", Buttons::SOUTH), ("east", Buttons::EAST), ("west", Buttons::WEST), ("north", Buttons::NORTH),
            ("left_shoulder", Buttons::LEFT_SHOULDER), ("right_shoulder", Buttons::RIGHT_SHOULDER), ("left_trigger", Buttons::LEFT_TRIGGER),
            ("right_trigger", Buttons::RIGHT_TRIGGER), ("select", Buttons::SELECT), ("start", Buttons::START),
            ("up", Buttons::UP), ("down", Buttons::DOWN), ("left", Buttons::LEFT), ("right", Buttons::RIGHT),
            ("jump", Buttons::SOUTH), ("fire", Buttons::RIGHT_TRIGGER), ("aim", Buttons::LEFT_TRIGGER),
        ];
        let all = lua.create_table()?;
        let none = Controls::default();
        let roster: Vec<(u32, bool)> = self.core().roster.iter().map(|p| (p.id, p.bot)).collect();
        let mut person = 0;
        for (id, bot) in roster {
            // People's controls in roster order; bots get a neutral input.
            let c = if bot { &none } else { person += 1; controls.get(person - 1).unwrap_or(&none) };
            let p = lua.create_table()?;
            p.raw_set("move_x", c.mv[0])?;
            p.raw_set("move_y", c.mv[1])?;
            p.raw_set("look_x", c.look[0])?;
            p.raw_set("look_y", c.look[1])?;
            p.raw_set("aim_x", c.aim[0])?;
            p.raw_set("aim_y", c.aim[1])?;
            let (held, pressed) = (lua.create_table()?, lua.create_table()?);
            for &(n, b) in NAMES { held.raw_set(n, c.held.has(b))?; pressed.raw_set(n, c.pressed.has(b))?; }
            p.raw_set("held", held)?;
            p.raw_set("pressed", pressed)?;
            all.raw_set(id, p)?;
        }
        Ok(all)
    }

    /// Grown plants that changed or finished cooking go into a fresh scene
    /// (vegetation is attached last and cannot be taken out again), and the
    /// sky is applied to it again.
    fn attach_plants(&mut self) {
        let mut lines = Vec::new();
        let mut sky_again = false;
        {
            let mut c = api::lock(&self.core);
            c.plants.poll();
            lines.append(&mut c.plants.log);
            let has_plants = !c.plants.plants.is_empty();
            // Keep showing the old plants until every new one has cooked, so
            // a regrown row swaps at once instead of blinking out.
            let cooking = c.plants.status().2 > 0;
            if (c.plants.dirty && !cooking) || (c.world.rebuild && has_plants && c.world.scene.gpu_forest.is_some()) {
                c.plants.dirty = false;
                let c = &mut *c;
                let started = std::time::Instant::now();
                let shown = c.world.fresh_scene().and_then(|_| { c.humans.detach(); c.plants.attach(&mut c.world.scene) });
                match shown {
                    Ok(n) => lines.push(format!("plants: {n} attached in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0)),
                    Err(e) => lines.push(format!("plants: not shown: {e}")),
                }
                sky_again = true;
            }
        }
        if sky_again { self.applied_sky = None; }
        for l in lines { self.say(l); }
    }

    /// The sky the scripts asked for, applied to the scene and the light.
    fn update_sky(&mut self) {
        let want = self.core().sky;
        if self.applied_sky == Some(want) { return; }
        self.applied_sky = Some(want);
        let mut c = api::lock(&self.core);
        match want {
            SkyChoice::Gradient => {
                c.world.scene.sky = Some(primitives::gradient_sky([0.25, 0.45, 0.8], [0.75, 0.82, 0.9], [0.35, 0.33, 0.3]));
                self.light = Lighting::default();
                c.world.rebuild = true;
                self.sky_upload = None;
            }
            SkyChoice::Preset(settings) => {
                static CACHE: std::sync::OnceLock<Mutex<AtmosphereCache>> = std::sync::OnceLock::new();
                let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
                match bake_live(0, settings, &mut cache, BakeQuality::default()) {
                    Ok(p) => {
                        let mut light = Lighting::default();
                        p.apply_scene(&mut c.world.scene, &mut light);
                        self.light = light;
                        self.sky_upload = Some(p);
                    }
                    Err(e) => { drop(c); self.say(format!("sky {}: {e}", want.name())); }
                }
            }
        }
    }

    fn lighting(&self) -> Lighting {
        let mut l = self.light.clone();
        let o = self.core().light;
        if let Some(s) = o.sun { l.sun_dir = s; }
        if let Some(v) = o.sun_lux { l.sun_lux = v; }
        if let Some(v) = o.sky_lux { l.sky_lux = v; }
        if let Some(v) = o.ev100 { l.ev100 = v; }
        if let Some(a) = o.auto { l.auto_exposure = Some(a); }
        if let Some(v) = o.fog[0] { l.fog_start = v; }
        if let Some(v) = o.fog[1] { l.fog_end = v; }
        if let Some(v) = o.fog[2] { l.fog_density = v; }
        let c = self.core();
        let p = c.post;
        l.contrast = p.contrast;
        l.saturation = p.saturation;
        l.vignette = p.vignette;
        l.grain = p.grain;
        l.aberration = p.aberration;
        l.time = c.tick as f32 / c.tick_rate;
        l
    }

    fn hud(&self, size: [u32; 2]) -> HudDrawList {
        let mut list = HudDrawList::new(size);
        let full = [0.0, 0.0, size[0] as f32, size[1] as f32];
        let rgba = |c: [f32; 4]| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        // The menu hides the game's HUD so nothing shows through it.
        let items = if self.menu.is_some() { Vec::new() } else { self.core().hud.clone() };
        for (k, item) in items.iter().enumerate() {
            match item {
                HudItem::Box { rect, color } => list.rect(*rect, *color, full),
                HudItem::Text { text, pos, size: fs, color, align, width } => list.text.push(TextRun {
                    id: k as u64 + 1, text: text.clone(), position: *pos, bounds: full, font_size: *fs, color: rgba(*color), font: FontFace::Body, align: *align,
                    width: if *width > 0.0 { *width } else { (size[0] as f32 - pos[0]).max(fs * 4.0) },
                }),
            }
        }
        let s = (size[1] as f32 / 720.0).max(0.5);
        // The start menu over the whole screen: the title, then the items.
        if let Some(sel) = self.menu {
            let (w, h) = (size[0] as f32, size[1] as f32);
            list.rect(full, [0.02, 0.02, 0.05, 0.72], full);
            let items = self.menu_items();
            let top = h * 0.5 - (64.0 + 56.0 * items.len() as f32) * s * 0.5;
            list.text.push(TextRun { id: 20_000, text: self.cfg.title.clone(), position: [0.0, top], bounds: full, font_size: 56.0 * s, color: [255, 255, 255, 255],
                font: FontFace::Title, align: render::hud::TextAlign::Center, width: w });
            for (i, item) in items.iter().enumerate() {
                let y = top + (96.0 + 56.0 * i as f32) * s;
                let on = i == sel;
                if on { list.rect([w * 0.5 - 170.0 * s, y - 8.0 * s, 340.0 * s, 48.0 * s], [1.0, 1.0, 1.0, 0.16], full); }
                list.text.push(TextRun { id: 20_001 + i as u64, text: item.label().into(), position: [0.0, y], bounds: full, font_size: 30.0 * s,
                    color: if on { [255, 214, 90, 255] } else { [230, 230, 240, 255] }, font: FontFace::Body, align: render::hud::TextAlign::Center, width: w });
            }
        }
        // Script errors stay on screen until fixed: live iteration needs to see them.
        let errors = self.errors();
        for (i, e) in errors.iter().take(6).enumerate() {
            list.text.push(TextRun { id: 10_000 + i as u64, text: e.chars().take(200).collect(), position: [12.0 * s, size[1] as f32 - (24.0 + 22.0 * (errors.len().min(6) - i) as f32) * s],
                bounds: full, font_size: 16.0 * s, color: [255, 110, 90, 255], font: FontFace::Body, align: render::hud::TextAlign::Left, width: size[0] as f32 - 24.0 * s });
        }
        list
    }

    /// Listeners at the cameras.
    fn ears(&self) {
        let mut c = self.core();
        let ears = c.roster.iter().filter(|p| p.view.is_some()).map(|p| p.cam).map(|k| {
            let f = (k.target - k.pos).try_normalize().unwrap_or(Vec3::NEG_Z);
            let r = f.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
            Ear { pos: k.pos.to_array(), forward: f.to_array(), right: r.to_array() }
        }).collect();
        c.sounds.ears(ears);
    }

    pub fn command_line(&mut self, cmd: &str) -> Result<String, String> {
        let (head, rest) = cmd.trim().split_once(' ').map_or((cmd.trim(), ""), |(a, b)| (a, b.trim()));
        match head {
            "help" | "" => Ok(Self::help().into()),
            "lua" => self.luau(rest),
            "reload" => { let l = self.reload(); Ok(if l.is_empty() { "nothing changed on disk".into() } else { l.join("\n") }) }
            "restart" => { self.restart(); Ok(format!("restarted at seed {}", self.seed)) }
            "sky" => { let s = SkyChoice::parse(rest)?; self.core().sky = s; Ok(format!("sky {}", s.name())) }
            "sounds" => Ok(self.core().sounds.bank.patches.values().map(|p| format!("{} ({})", p.name, p.params.iter().map(|(n, d)| format!("{n}={d}")).collect::<Vec<_>>().join(" "))).collect::<Vec<_>>().join("\n")),
            "entities" => {
                let c = self.core();
                let lines: Vec<String> = c.world.entities.iter().filter(|(_, e)| rest.is_empty() || e.tag == rest)
                    .map(|(id, e)| {
                        let pos = e.world_pose().1;
                        format!("{id} {} {} pos {:.2} {:.2} {:.2}", e.shape.name(), if e.tag.is_empty() { "-" } else { &e.tag }, pos.x, pos.y, pos.z)
                    }).collect();
                Ok(if lines.is_empty() { "no entities".into() } else { lines.join("\n") })
            }
            "logs" => Ok(self.logs().join("\n")),
            "menu" => self.menu_command(rest),
            "knobs" => self.knobs(),
            "knob" => { let (name, value) = rest.split_once(' ').ok_or("knob NAME VALUE")?; self.set_knob(name.trim(), value, true) }
            "trial" => { let (name, value) = rest.split_once(' ').ok_or("trial NAME VALUE")?; self.set_knob(name.trim(), value, false) }
            "keep" => self.keep(),
            "revert" => self.revert(),
            other => Err(format!("unknown command {other:?}; {}", Self::help())),
        }
    }

    /// Start live control: the generic MCP tools, plus `reload`, `restart` and `command`.
    pub fn start_live(&mut self, cfg: live::Config) -> Result<(), String> {
        let tools = vec![
            live::ToolSpec::new("reload", "Reload edited Luau modules and sound patches now (they also reload by themselves within a quarter second of a save). Returns what changed and any load error. A reload keeps the world and each module's `state` table and calls the module's `reload()` hook; `init` does not run again.", json!({ "type": "object", "properties": {} })),
            live::ToolSpec::new("restart", "Start the game over: remove every entity, clear every module's `state`, reseed the random generator and run `init` again.", json!({ "type": "object", "properties": {} })),
            live::ToolSpec::new("command", "One debug console line. `help` lists them: lua CODE, reload, restart, sky NAME, sounds, entities [TAG], knobs, knob NAME VALUE, logs.", json!({ "type": "object", "properties": { "line": { "type": "string" } }, "required": ["line"] })),
            live::ToolSpec::new("knobs", "The game's tweakable values (its `-- @knobs` block in main.luau) with their current values and what each does. Start here to change the look or feel.", json!({ "type": "object", "properties": {} })).read_only(),
            live::ToolSpec::new("set_knob", "Set one knob and hot-reload; the game's reload() hook applies it at once. With persist (the default) the value is written into main.luau, ready to commit. With persist false only the running game changes: `keep` saves such changes, `revert` undoes them. A value that does not load is put back.", json!({ "type": "object", "properties": { "name": { "type": "string" }, "value": { "type": "string", "description": "one Luau value, such as 0.5, true or \"#ff00ff\"" }, "persist": { "type": "boolean", "description": "write into main.luau (default true); false changes only the running game" } }, "required": ["name", "value"] })),
            live::ToolSpec::new("keep", "Save the temporary knob changes (set_knob with persist false) into main.luau.", json!({ "type": "object", "properties": {} })),
            live::ToolSpec::new("revert", "Undo the temporary knob changes: run main.luau as saved again.", json!({ "type": "object", "properties": {} })),
        ];
        let manifest = live::Manifest {
            name: self.live_name(),
            version: env!("CARGO_PKG_VERSION").into(),
            about: format!("\"{}\", a Warp game made of Luau scripts (main.luau), synth patches (sounds/*.luau) and assets, run by the `warp` executable.", self.cfg.title),
            guide: "Call `knobs` first: the values the game marks as tweakable, set with `set_knob` (live, and saved in main.luau; `persist: false` changes only the running game until `keep` or `revert`). For anything else, edit the game with write_file in the `game` folder: main.luau holds init(), tick(dt, input) and draw(); sounds/NAME.luau is the patch `NAME`. A saved file reloads at once: the world and the `state` table stay, so change code without losing the play session; call `restart` to run init again. The Luau API is in the folder's types.d.luau. `state` is the game's JSON (its `state` field is main.luau's state table); `run_luau` runs Luau against the live game, with main.luau's globals and exports in scope (call its functions: export a `debug` table of helpers); `screenshot` shows the frame; `render_sound` measures a patch without speakers. Errors appear in `state.errors`, `logs` and on screen.".into(),
            tools,
            roots: vec![live::Root::new("game", &self.dir, live::Reload::Game, "The game folder: main.luau and other modules, sounds/*.luau patches, game.toml, types.d.luau. A write reloads the scripts and sounds and reports load errors.").extensions(&["luau", "toml", "md", "json", "wgsl"])],
            features: live::Features { state: true, screenshot: true, luau: true, shaders: false, sound: true },
        };
        let live = live::start(cfg, manifest)?;
        if let (Some(party), Some(file)) = (self.party.as_mut(), live.token_file()) {
            party.offer_live(format!("{} can be changed live while you play: its speed, difficulty, enemies, rules, colours, look and sounds, by editing the running game. Send any request to change the game as it is said.", self.cfg.title), file.to_path_buf());
        }
        for l in &self.log { live.log(l.clone()); }
        self.live_logs = Some(live.logs().clone());
        self.live = Some(live);
        Ok(())
    }

    pub fn live_name(&self) -> String {
        let stem = self.single.as_deref().and_then(|f| Path::new(f).file_stem()).map(|s| s.to_string_lossy().into_owned());
        let name = stem.or_else(|| self.dir.canonicalize().ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))).unwrap_or_else(|| self.cfg.title.clone());
        let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        slug.trim_matches('-').to_string()
    }
}

/// Bloom and the scripts' lamps onto the renderer (cheap when unchanged).
fn post(r: &render::Renderer, p: &api::Post, lamps: &[api::Lamp], spots: &[api::Spot]) -> Result<(), String> {
    let mut o = r.debug_options();
    let before = o;
    if let Some(b) = p.bloom {
        o.bloom_enabled = b > 0.0;
        o.bloom_strength = b;
        o.bloom_rounds = p.bloom_radius;
        o.bloom_threshold = p.bloom_threshold;
        o.bloom_screen = p.bloom_wide;
    }
    if o != before { r.set_debug_options(o)?; }
    let lights: Vec<render::headlights::LocalLight> = lamps.iter().map(|l| {
        let lin = l.color.map(|c| c.clamp(0.0, 1.0).powf(2.2));
        render::headlights::LocalLight {
            position: l.pos,
            direction: glam::Vec3::NEG_Y,
            intensity_cd: glam::Vec3::from(lin) * l.intensity,
            range_m: l.range,
            inner_cos: 1.0,
            outer_cos: -1.0,
            source_radius_m: 0.1_f32.min(l.range * 0.5),
            falloff: 1.5,
        }
    }).collect();
    r.set_local_lights(&lights)?;
    let spots: Vec<render::headlights::SpotLight> = spots.iter().map(|l| {
        let lin = l.color.map(|c| c.clamp(0.0, 1.0).powf(2.2));
        // Any up that is not along the beam.
        let up = if l.dir.y.abs() > 0.9 { glam::Vec3::Z } else { glam::Vec3::Y };
        render::headlights::SpotLight {
            position: l.pos,
            direction: l.dir,
            up,
            intensity_cd: glam::Vec3::from(lin) * l.intensity,
            range_m: l.range,
            inner_angle_radians: l.inner,
            outer_angle_radians: l.angle,
        }
    }).collect();
    r.set_headlights(&spots)
}

impl Game for LuaGame {
    fn debug(&self) -> (Vec<render::debug_ui::DebugMenu>, Vec<render::debug_ui::DebugControl>) {
        self.editor_pages.clone()
    }

    fn debug_action(&mut self, action: render::debug_ui::DebugAction) {
        let render::debug_ui::DebugAction::Activate { id } = action else { return };
        if !self.editor_pages.1.iter().any(|b| b.id == id && b.enabled) { return; }
        let name = self.main_name();
        if let Some((i, f)) = self.host.each("editor_action").into_iter().find(|(i,_)| self.host.modules()[*i].name == name) {
            self.call_one(i, "editor_action", &f, &|| Ok(mlua::MultiValue::from_vec(vec![Value::Number(id as f64)])));
        }
    }

    /// The people; bots need no controls.
    fn local_players(&self) -> usize { self.core().roster.iter().filter(|p| !p.bot).count().max(1) }

    /// Under GameNight the party's controllers play: never grab the mouse
    /// (and so no click-to-capture hint on the TV).
    fn mouse_capture(&self) -> Option<bool> { self.party.as_ref().map(|_| false) }

    fn tick(&mut self, controls: &[Controls]) {
        // Frames until the next tick are drawn from here (Frame::alpha).
        {
            let mut c = self.core();
            c.world.tick_start();
            for p in &mut c.roster { p.cam_prev = p.cam; }
        }
        if let Some(mut l) = self.live.take() { l.serve(self); self.live = Some(l); }
        if self.polls % POLL_TICKS == 0 && self.polls > 0 { self.reload(); }
        self.polls += 1;
        let dt = 1.0 / self.cfg.tick_rate;
        // Under GameNight the party's controllers drive the seats; the game
        // only runs while the host says so.
        let controls = match self.party_tick(dt) {
            None => controls.to_vec(),
            Some(None) => { self.send_requests(); return; }
            Some(Some(c)) => c,
        };
        if self.menu_tick(&controls) { return; }
        let input = match self.input(self.host.lua(), &controls) { Ok(t) => t, Err(e) => { self.say(format!("input: {e}")); return; } };
        self.hook("tick", || Ok(mlua::MultiValue::from_vec(vec![Value::Number(dt as f64), Value::Table(input.clone())])));
        {
            let mut c = self.core();
            c.world.step_fx(dt);
            c.humans.tick(dt);
            c.tick += 1;
        }
        self.ears();
        self.send_requests();
    }

    fn screen(&self) -> app::Screen { self.party.as_ref().map_or(app::Screen::Normal, |p| p.screen()) }

    fn hidden_frame_due(&self) -> bool { self.ready_due }

    fn loading(&mut self, percent: u32, step: &str) {
        self.windowed = true;
        if let Some(p) = self.party.as_mut() { p.loading(percent, step); }
    }

    /// Escape opens and closes the start menu.
    fn escape(&mut self) -> bool {
        if self.menu.is_some() { self.close_menu(); } else { self.say(format!("{} menu: opened by Escape", app::manners::stamp())); self.open_menu(); }
        true
    }

    fn quit(&self) -> bool { self.quit }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let views = frame.rects(self.views());
        {
            let mut c = self.core();
            c.screen = frame.size;
            c.views = views;
            c.hud.clear();
            c.phase = Phase::Draw;
        }
        self.hook("draw", || Ok(mlua::MultiValue::new()));
        self.core().phase = Phase::Sim;
        self.attach_plants();
        self.update_sky();
        let light = self.lighting();
        let mut c = api::lock(&self.core);
        let mut surface_line = None;
        let added = std::mem::take(&mut c.world.added);
        let mut late = None;
        // Warp Humans join the scene before its first renderer.
        if !c.humans.is_empty() && !c.humans.attached() {
            let c = &mut *c;
            if let Err(e) = c.humans.attach(&mut c.world.scene) { late = Some(format!("humans: {e}")); }
            c.world.rebuild = true;
        }
        if std::mem::take(&mut c.world.rebuild) || frame.renderer().is_none() {
            // A look first seen after the start stalls the game while the
            // renderer is rebuilt: name it so it can be warmed in init.
            if frame.renderer().is_some() && !added.is_empty() {
                let more = if added.len() > 4 { format!(" and {} more", added.len() - 4) } else { String::new() };
                late = Some(format!("renderer rebuilt mid-game for new looks: {}{more} (warm them in init with world.warm, or reuse a look)", added[..added.len().min(4)].join(", ")));
            }
            if self.surface.pending() {
                let gpu = frame.gpu;
                surface_line = self.surface.apply(|| render::validated(gpu, || frame.set_scene(&c.world.scene, &light)));
            } else {
                frame.set_scene(&c.world.scene, &light);
            }
            self.sky_upload = None;
        } else {
            if self.surface.pending() {
                let gpu = frame.gpu;
                surface_line = self.surface.apply(|| frame.renderer_mut().ok_or("no renderer".to_string())?.reload_shaders(gpu).map(|_| ()));
            }
            if let Some(p) = self.sky_upload.take() {
                let gpu = frame.gpu;
                let mut l = light.clone();
                if let Some(r) = frame.renderer_mut() && let Err(e) = r.update_sky(gpu, &p.sky, &mut l) { eprintln!("sky unchanged on the GPU: {e}"); }
            }
        }
        let r = frame.renderer().ok_or("no renderer")?;
        c.world.flush_at(frame.gpu, r, frame.alpha)?;
        post(r, &c.post, &c.lamps, &c.spots)?;
        {
            let c = &mut *c;
            c.humans.draw(frame.gpu, r, &mut c.world.scene, frame.alpha)?;
        }
        let mut seen: Vec<(usize, Cam)> = c.roster.iter().filter_map(|p| p.view.map(|v| (v, p.cam_prev.towards(&p.cam, frame.alpha)))).collect();
        seen.sort_by_key(|(v, _)| *v);
        if seen.is_empty() { seen.push((0, Cam::default())); }
        let cameras = seen.into_iter().map(|(_, k)| k).map(|k| Camera { pos: k.pos, target: k.target, fov_y: k.fov.to_radians(), near: 0.05 }).collect();
        drop(c);
        if let Some(line) = surface_line { self.say(line); }
        if let Some(line) = late { self.say(line); }
        let hud = self.hud(frame.size);
        if std::mem::take(&mut self.ready_due) && let Some(p) = self.party.as_mut() { p.ready(); }
        Ok(Draw { cameras, light, hud })
    }

    fn title(&self) -> Option<String> {
        let e = self.errors();
        Some(if e.is_empty() { format!("tick {}", self.core().tick) } else { format!("{} error(s): {}", e.len(), e[0]) })
    }

    fn state(&self) -> Option<String> { Some(self.state_json().to_string()) }

    fn command(&mut self, cmd: &str) -> Result<String, String> { self.command_line(cmd) }

    /// A script's `knob NAME VALUE` changes only the running game (like
    /// `trial`): a scripted or headless run never edits main.luau.
    fn script_command(&mut self, cmd: &str) -> Result<String, String> {
        match cmd.trim_start().strip_prefix("knob ") {
            Some(rest) => self.command_line(&format!("trial {rest}")),
            None => self.command_line(cmd),
        }
    }

    fn wants_capture(&self) -> bool { self.live.as_ref().is_some_and(|l| l.wants_capture()) }

    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) { if let Some(l) = &mut self.live { l.captured(size, rgba); } }
}

impl live::LiveGame for LuaGame {
    fn state(&mut self) -> Result<Json, String> { Ok(self.state_json()) }

    fn call(&mut self, tool: &str, args: &Json) -> live::Reply {
        let text = match tool {
            "reload" => { let l = self.reload(); if l.is_empty() { "nothing changed on disk".to_string() } else { l.join("\n") } }
            "restart" => { self.restart(); format!("restarted at seed {}", self.seed) }
            "command" => self.command_line(args["line"].as_str().ok_or("command needs `line`")?)?,
            "knobs" => self.knobs()?,
            "set_knob" => {
                let value = match &args["value"] { Json::String(v) => v.clone(), Json::Null => return Err("set_knob needs `value`".into()), v => v.to_string() };
                self.set_knob(args["name"].as_str().ok_or("set_knob needs `name`")?, &value, args["persist"].as_bool().unwrap_or(true))?
            }
            "keep" => self.keep()?,
            "revert" => self.revert()?,
            other => return Err(format!("no tool {other}")),
        };
        let errors = self.errors();
        Ok(vec![live::Content::Text(if errors.is_empty() { text } else { format!("{text}\nerrors:\n{}", errors.join("\n")) })])
    }

    fn luau(&mut self, code: &str) -> Result<String, String> { LuaGame::luau(self, code) }

    fn file_written(&mut self, _root: &str, path: &str) -> Option<String> {
        self.host.forget(path);
        let lines = self.reload();
        let mine: Vec<&String> = lines.iter().filter(|l| l.starts_with(path) || l.contains(path)).collect();
        Some(if mine.is_empty() { format!("{path}: written (no module or patch changed: {})", if lines.is_empty() { "nothing reloaded".into() } else { lines.join("; ") }) } else { mine.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n") })
    }

    fn sound(&mut self) -> Option<&mut dyn live::SoundHook> { Some(self) }
}

impl live::SoundHook for LuaGame {
    fn sounds(&mut self) -> Vec<live::SoundInfo> {
        self.reload();
        self.core().sounds.bank.patches.values().map(|p| live::SoundInfo { name: p.name.clone(), params: p.params.clone(),
            about: format!("{}{}", if p.max_length > 0.0 { format!("up to {:.2} s", p.max_length) } else { "no fixed length".into() }, if p.looping { ", loops" } else { "" }) }).collect()
    }

    fn play(&mut self, name: &str, params: &[(String, f32)], at: Option<[f32; 3]>) -> Result<String, String> {
        let mut c = self.core();
        let tick = c.tick;
        let id = c.sounds.play(name, params, at, 0.0, 1.0, tick)?;
        Ok(format!("playing {name} (handle {id}){}{}", at.map_or(" at the listener".into(), |p| format!(" at {p:?}")), if c.sounds.has_output() { "" } else { "; this run has no audio output, render_sound measures it" }))
    }

    fn set_param(&mut self, sound: &str, param: &str, value: f32) -> Result<String, String> {
        let old = self.core().sounds.bank.set_default(sound, param, value)?;
        Ok(format!("{sound}.{param}: {old} -> {value} for every later play (values a script passes still win; saving the file resets it)"))
    }

    fn render(&mut self, name: &str, params: &[(String, f32)], seconds: Option<f32>) -> Result<live::Work, String> {
        let patch = self.core().sounds.bank.find(name)?;
        check_params(&patch, params)?;
        let opts = audio::cli::Opts { set: params.to_vec(), seconds, ..Default::default() };
        Ok(Box::new(move || audio::cli::report(&patch, &opts).map(|t| vec![live::Content::Text(t)])))
    }
}
