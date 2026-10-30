//! Menus after Godot's `start_screen.gd` (with `gothic_menu_ui.gd`'s stone
//! and gold frame), `game.gd`'s pause menu, settings panel and match-over
//! buttons, and its Tab scoreboard. Focus moves with the D-pad, arrows,
//! WASD or stick; A/Space/Enter takes, B/Esc backs out; left and right move
//! sliders. Mouse hover focuses, clicks activate and dragging adjusts sliders.
//! The join field and the Player name take typed text and Ctrl+V while
//! focused (`app::TextField`); a pad opens an on-screen keyboard on them
//! with A (`app::ui::CharPicker`, with PASTE). A valid pasted ID joins at
//! once, as Godot's field does. COPY puts the match ID on the clipboard.
//! Splitscreen (`splitscreen_manager.gd`): the pause menu's SPLITSCREEN, the
//! "PRESS X TO JOIN" prompt, the hint and the per-player PLAYER MENU are
//! drawn here; play.rs seats the pads. Under GameNight the pause
//! menu is Godot's `_build_gamenight_pause_menu`: a slab down the left with
//! the logo, what is being played, and RESUME, NEW MATCH, the other mode,
//! AI PLAYERS, ADVANCED (the party's match settings) and SETTINGS.
use super::style::*;
use super::{Hud, DESIGN_H};
use app::{Buttons, Controls, FieldEvent, TextEvent, TextField};
use growing_guns::{Phase, World};
use render::hud::HudDrawList;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen { Play, Start, Pause, Settings { from_start: bool }, Advanced }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { None, Resume, Versus, Wave, Settings, Back, Host, Copy, Quit, Rematch, Extend, AddBot, RemoveBot, Advanced,
    /// Join the match in the join field (`Menus::join_ticket`).
    Join,
    /// Turn splitscreen on or off (Godot `_pause_toggle_splitscreen`).
    Splitscreen,
    /// A party setting (index into `Ctx::party`) changed to this value.
    Party(usize, i64) }

#[derive(Clone, Copy, Debug, PartialEq)]
enum Item { Text(&'static str, C), Note(&'static str), Field(Field), Button(&'static str, Action, bool), Row(&'static [(&'static str, Action)]), Slider(Slider), Gap(f32), Rule,
    /// Movement tilt on or off.
    Tilt,
    /// Bots: - count +, with this label on the left (empty: none).
    Bots(&'static str),
    /// A GameNight setting slider (index into `Ctx::party`).
    Party(usize) }
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field { Ticket, Join, /** Settings: Player name. */ Name }
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slider { Music, Mouse, Stick, /** One per GameNight seat ("Aim: NAME"). */ Aim(usize) }

#[derive(Clone, Copy, Debug)]
struct Hit { rect: [f32; 4], item: usize, entry: usize }

/// What the menus need to know about the session.
pub struct Ctx<'a> {
    pub ticket: Option<&'a str>, pub client: bool, pub seed: u64, pub bots: usize,
    /// Everyone in the match, bots too.
    pub players: usize,
    pub coop: bool,
    /// Under GameNight: the party's settings (label, value, min, max) and
    /// the status line under the logo.
    pub party: Option<(Vec<(&'static str, i64, i64, i64)>, String)>,
    /// Names of the GameNight seats here (their own aim sliders).
    pub seats: Vec<String>,
}

/// What the join field holds (Godot `_update_join_form`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinState { Empty, Invalid, Own, Valid, Connecting }

/// Godot `extract_iroh_node_id`: the first piece of the pasted text that is
/// a ticket (even inside a pasted shell command or quotes), else the text
/// itself (trimmed).
pub fn extract_ticket(text: &str) -> String {
    let t = text.trim();
    if let Some(p) = t
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|p| p.len() >= 64 && net::parse_ticket(p).is_ok())
    {
        return p.to_string();
    }
    t.split_whitespace().find(|p| net::parse_ticket(p).is_ok()).unwrap_or(t).to_string()
}

#[derive(Clone, Debug)]
pub struct Menus {
    pub screen: Screen,
    pub menu: app::ui::Menu,
    pub rematch: app::ui::Menu,
    /// Look sensitivity multipliers (Godot settings: mouse 0.3..3, stick 0.3..2.5).
    pub mouse: f32,
    pub stick: f32,
    /// Music volume, dB (Godot -45 = off .. 0; default -16).
    pub music_db: f32,
    /// Movement tilt (the gun's strafe lean).
    pub tilt: bool,
    /// Stick sensitivity per GameNight player, by name.
    pub aim: Vec<(String, f32)>,
    /// Seconds the screen has been open (fades and the start orbit).
    pub age: f32,
    /// The Tab / Back scoreboard is held.
    pub scoreboard: bool,
    held_x: i32,
    repeat: f32,
    /// The match-over buttons are unlocked and took their first focus.
    rematch_armed: bool,
    /// Entry within a focused row.
    col: usize,
    /// A short line under the menu ("ticket written to ...").
    pub notice: Option<String>,
    /// The last network error, for the HUD's red panel.
    pub error: Option<String>,
    hits: RefCell<Vec<Hit>>,
    hit_size: Cell<[u32; 2]>,
    drag: Option<Hit>,
    /// "Or join a friend": the pasted match ID.
    pub join: TextField,
    /// A join is connecting (play.rs runs it).
    pub joining: bool,
    /// Settings: the Player name being edited, and the committed one.
    pub name: TextField,
    pub player_name: String,
    /// A pad typing into the focused field.
    pub picker: Option<app::ui::CharPicker>,
    /// Seconds left showing COPIED! on the COPY button.
    pub copied: f32,
}

impl Default for Menus {
    fn default() -> Self {
        Menus { screen: Screen::Play, menu: Default::default(), rematch: Default::default(), mouse: 1.0, stick: 1.0, music_db: -16.0, tilt: true, aim: Vec::new(), age: 0.0, scoreboard: false, held_x: 0, repeat: 0.0, rematch_armed: false, col: 0, notice: None, error: None, hits: Default::default(), hit_size: Cell::new([0; 2]), drag: None,
            join: TextField::new(4096), joining: false, name: TextField::new(20), player_name: String::new(), picker: None, copied: 0.0 }
    }
}

const SHARE: &[(&str, Action)] = &[("HOST ONLINE", Action::Host), ("COPY", Action::Copy)];

fn items(screen: Screen, ctx: &Ctx) -> Vec<Item> {
    let label = [0.7, 0.7, 0.85, 1.0];
    match screen {
        Screen::Start => vec![
            Item::Button("VERSUS", Action::Versus, true), Item::Button("WAVE SURVIVAL", Action::Wave, true), Item::Gap(4.0),
            Item::Button("SETTINGS", Action::Settings, true), Item::Rule,
            Item::Text("Share your Match ID:", label), Item::Field(Field::Ticket), Item::Row(SHARE),
            Item::Text("Or join a friend's match:", label), Item::Field(Field::Join),
            Item::Text("Bots:", label), Item::Bots(""),
            Item::Note("Click or arrows + Enter · Esc exits"),
        ],
        Screen::Pause if ctx.party.is_some() => vec![
            Item::Button("RESUME", Action::Resume, true), Item::Button("NEW MATCH", Action::Rematch, !ctx.client),
            if ctx.coop { Item::Button("SWITCH TO VERSUS", Action::Versus, !ctx.client) } else { Item::Button("SWITCH TO WAVE SURVIVAL", Action::Wave, !ctx.client) },
            Item::Bots("AI PLAYERS"), Item::Button("ADVANCED", Action::Advanced, true), Item::Button("SETTINGS", Action::Settings, true),
        ],
        Screen::Advanced => {
            let n = ctx.party.as_ref().map_or(0, |p| p.0.len());
            let mut v: Vec<Item> = (0..n).map(Item::Party).collect();
            v.push(Item::Button("BACK", Action::Back, true));
            v
        }
        Screen::Pause => vec![
            Item::Text("Share your match with friends:", label), Item::Field(Field::Ticket), Item::Row(SHARE),
            Item::Text("Or join a friend:", label), Item::Field(Field::Join), Item::Note("Paste an iroh game ID"),
            Item::Text("Bots:", label), Item::Bots(""), Item::Gap(4.0),
            Item::Button("VERSUS", Action::Versus, !ctx.client), Item::Button("WAVE SURVIVAL", Action::Wave, !ctx.client),
            Item::Button("SPLITSCREEN", Action::Splitscreen, !ctx.client),
            Item::Button("SETTINGS", Action::Settings, true), Item::Button("RESUME", Action::Resume, true), Item::Button("EXIT GAME", Action::Quit, true),
        ],
        Screen::Settings { .. } => settings_items(ctx),
        Screen::Play => vec![],
    }
}

/// Godot `build_settings_panel`: Player name, music, mouse, one stick
/// slider (or one "Aim" slider per GameNight seat), movement tilt, BACK.
/// (The retro shader has no counterpart here; under GameNight the party
/// names the players, so there is no Player name.)
fn settings_items(ctx: &Ctx) -> Vec<Item> {
    let mut v = if ctx.seats.is_empty() { vec![Item::Field(Field::Name)] } else { vec![] };
    v.extend([Item::Slider(Slider::Music), Item::Slider(Slider::Mouse)]);
    if ctx.seats.is_empty() { v.push(Item::Slider(Slider::Stick)); } else { v.extend((0..ctx.seats.len()).map(|k| Item::Slider(Slider::Aim(k)))); }
    v.extend([Item::Tilt, Item::Gap(8.0), Item::Button("BACK", Action::Back, true)]);
    v
}

/// Godot `music_label_for`: Off at the bottom, else 0..100 %.
fn music_label(db: f32) -> String { if db <= MUSIC_DB_MIN + 0.5 { "Off".into() } else { format!("{}%", ((db - MUSIC_DB_MIN) / -MUSIC_DB_MIN * 100.0).round()) } }
pub const MUSIC_DB_MIN: f32 = -45.0;

/// The bot stepper's - (0) and + (1): host or offline, not in coop (the
/// waves bring their own), 0..=6 bots and 2..=8 total players.
fn bots_enabled(j: usize, ctx: &Ctx) -> bool { !ctx.client && !ctx.coop && if j == 0 { ctx.bots > 0 && ctx.players > 2 } else { ctx.players < 8 } }

fn row_enabled(t: &str, ctx: &Ctx) -> bool { !(ctx.client || (t == "COPY" && ctx.ticket.is_none()) || (t == "HOST ONLINE" && ctx.ticket.is_some())) }

/// Focusable lines: item index and the enabled entries in it (a row has
/// several; the join field's JOIN button is entry 1, when it can join).
fn lines(items: &[Item], ctx: &Ctx, join_ok: bool) -> Vec<(usize, Vec<usize>)> {
    let mut out = Vec::new();
    for (k, it) in items.iter().enumerate() {
        match it {
            Item::Field(Field::Join) if !ctx.client => out.push((k, if join_ok { vec![0, 1] } else { vec![0] })),
            Item::Field(Field::Name) => out.push((k, vec![0])),
            Item::Button(_, _, true) | Item::Slider(_) | Item::Party(_) | Item::Tilt => out.push((k, vec![0])),
            Item::Bots(_) => {
                let on: Vec<usize> = [0, 1].into_iter().filter(|&j| bots_enabled(j, ctx)).collect();
                if !on.is_empty() { out.push((k, on)); }
            }
            Item::Row(r) => {
                let on: Vec<usize> = (0..r.len()).filter(|&j| row_enabled(r[j].0, ctx)).collect();
                if !on.is_empty() { out.push((k, on)); }
            }
            _ => {}
        }
    }
    out
}

fn match_buttons(world: &World) -> Vec<(&'static str, Action)> {
    if world.coop { vec![("TRY AGAIN FROM WAVE 1", Action::Rematch), ("EXIT GAME", Action::Quit)] } else { vec![("REMATCH", Action::Rematch), ("5 MORE ROUNDS", Action::Extend), ("EXIT GAME", Action::Quit)] }
}

impl Menus {
    pub fn open(&mut self, s: Screen) {
        self.commit_name();
        if matches!(s, Screen::Settings { .. }) { let n = self.player_name.clone(); self.name.set(&n); }
        self.screen = s; self.menu = Default::default(); self.col = 0; self.age = 0.0; self.notice = None; self.hits.get_mut().clear(); self.drag = None; self.picker = None;
    }

    /// What the join field holds.
    pub fn join_state(&self, ctx: &Ctx) -> JoinState {
        let t = self.join.text().trim();
        if self.joining { JoinState::Connecting } else if t.is_empty() { JoinState::Empty } else if ctx.ticket == Some(t) { JoinState::Own }
        else if net::parse_ticket(t).is_ok() { JoinState::Valid } else { JoinState::Invalid }
    }
    fn join_ok(&self, ctx: &Ctx) -> bool { !ctx.client && self.join_state(ctx) == JoinState::Valid }
    /// The match ID to join.
    pub fn join_ticket(&self) -> String { self.join.text().trim().to_string() }

    /// Godot `commit_name`: a non-empty, changed name is kept (the field
    /// shows the kept one again otherwise).
    fn commit_name(&mut self) {
        let entered = self.name.text().trim().to_string();
        if !matches!(self.screen, Screen::Settings { .. }) { return; }
        if entered.is_empty() { let n = self.player_name.clone(); self.name.set(&n); } else { self.player_name = entered; }
    }

    /// Put the focus on a text field (scripts' `focus` command).
    pub fn focus_field(&mut self, f: Field, ctx: &Ctx) -> bool {
        let list = items(self.screen, ctx);
        let Some(n) = lines(&list, ctx, self.join_ok(ctx)).iter().position(|(k, _)| list[*k] == Item::Field(f)) else { return false };
        self.menu.focus = n;
        self.col = 0;
        true
    }

    /// The text field with the focus, if any: the keyboard types into it.
    pub fn text_focus(&self, ctx: &Ctx) -> Option<Field> {
        if !self.is_open() { return None; }
        let list = items(self.screen, ctx);
        match self.focused(&list, ctx) { Some((k, 0)) => match list[k] { Item::Field(f @ (Field::Join | Field::Name)) => Some(f), _ => None }, _ => None }
    }

    /// Typed text for the focused field (keyboard, IME, paste, a pad's picker).
    pub fn text(&mut self, ev: TextEvent, ctx: &Ctx) -> Action {
        let f = match self.text_focus(ctx) {
            Some(f) => f,
            None if matches!(ev, TextEvent::Paste(_)) && matches!(self.screen, Screen::Start | Screen::Pause) && !ctx.client => {
                self.focus_field(Field::Join, ctx);
                Field::Join
            }
            None => return Action::None,
        };
        if f == Field::Join && let TextEvent::Paste(s) = &ev {
            let id = extract_ticket(s);
            if net::parse_ticket(&id).is_ok() {
                self.join.set(&id);
                return if self.join_ok(ctx) { Action::Join } else { Action::None };
            }
        }
        let field = if f == Field::Join { &mut self.join } else { &mut self.name };
        match field.apply(&ev) {
            FieldEvent::Changed if f == Field::Join => {
                // Godot `_on_join_input_changed`: keep the ID out of a pasted
                // line, and connect as soon as it is a valid one.
                let id = extract_ticket(self.join.text());
                if id != self.join.text() { self.join.set(&id); }
                if self.join_ok(ctx) { return Action::Join; }
                Action::None
            }
            FieldEvent::Submit if f == Field::Join => if self.join_ok(ctx) { Action::Join } else { Action::None },
            FieldEvent::Submit => { self.commit_name(); Action::None }
            FieldEvent::Cancel => Action::Back,
            _ => Action::None,
        }
    }
    pub fn is_open(&self) -> bool { self.screen != Screen::Play }

    fn hit(&self, rect: [f32; 4], item: usize, entry: usize) {
        self.hits.borrow_mut().push(Hit { rect, item, entry });
    }

    /// Targets come from the last displayed frame, so scale, layout and clipping
    /// match the visible controls. Recheck enabled state before activation.
    pub fn pointer(&mut self, p: app::Pointer, size: [u32; 2], world: &World, match_over: Option<f32>, ctx: &Ctx) -> Action {
        if !p.held { self.drag = None; }
        if self.hit_size.get() != size { self.drag = None; return Action::None; }
        if !p.moved && !p.pressed { return Action::None; }
        let Some(pos) = p.position else { return Action::None };
        let inside = app::ui::contains([0.0, 0.0, size[0] as f32, size[1] as f32], pos);
        let hit = self.drag.or_else(|| inside.then(|| self.hits.borrow().iter().rev().find(|h| app::ui::contains(h.rect, pos)).copied()).flatten());
        let Some(hit) = hit else { return Action::None };
        if !self.is_open() {
            if match_over.is_none_or(|t| t < 2.4) { return Action::None; }
            self.rematch.focus = hit.item;
            return if p.pressed { match_buttons(world).get(hit.item).map_or(Action::None, |b| b.1) } else { Action::None };
        }
        let list = items(self.screen, ctx);
        let foci = lines(&list, ctx, self.join_ok(ctx));
        let Some((focus, (_, entries))) = foci.iter().enumerate().find(|(_, (k, entries))| *k == hit.item && entries.contains(&hit.entry)) else { return Action::None };
        if self.text_focus(ctx) == Some(Field::Name) && !matches!(list[hit.item], Item::Field(Field::Name)) && p.pressed { self.commit_name(); }
        self.menu.focus = focus;
        self.col = entries.iter().position(|&j| j == hit.entry).unwrap_or(0);
        let item = list[hit.item];
        let sliding = matches!(item, Item::Slider(_) | Item::Party(_));
        if p.pressed && sliding { self.drag = p.held.then_some(hit); }
        if !(p.pressed || sliding && p.held && self.drag.is_some()) { return Action::None; }
        let t = ((pos[0] - hit.rect[0]) / hit.rect[2].max(1.0)).clamp(0.0, 1.0);
        match item {
            Item::Button(_, a, true) => a,
            Item::Field(Field::Join) if hit.entry == 1 => Action::Join,
            Item::Row(row) => row[hit.entry].1,
            Item::Bots(_) => if hit.entry == 0 { Action::RemoveBot } else { Action::AddBot },
            Item::Tilt => { self.tilt = !self.tilt; Action::None }
            Item::Slider(s) => {
                if s == Slider::Music { self.music_db = (MUSIC_DB_MIN * (1.0 - t)).round(); }
                else {
                    let hi = if s == Slider::Mouse { 3.0 } else { 2.5 };
                    let v = ((0.3 + (hi - 0.3) * t) * 20.0).round() / 20.0;
                    match s { Slider::Mouse => self.mouse = v, Slider::Stick => self.stick = v,
                        Slider::Aim(i) => if let Some(name) = ctx.seats.get(i) { self.set_aim(name, v); }, Slider::Music => {} }
                }
                Action::None
            }
            Item::Party(i) => ctx.party.as_ref().and_then(|p| p.0.get(i)).map_or(Action::None, |&(_, v, lo, hi)| {
                let next = lo + ((hi - lo) as f32 * t).round() as i64;
                if next == v { Action::None } else { Action::Party(i, next) }
            }),
            _ => Action::None,
        }
    }

    /// One tick of a menu with this player's controls.
    pub fn update(&mut self, c: &Controls, world: &World, match_over: Option<f32>, ctx: &Ctx, dt: f32) -> Action {
        self.age += dt;
        self.scoreboard = c.held.has(Buttons::SELECT) && !self.is_open();
        if !self.is_open() {
            // The match-over buttons: shown 1.4 s after the banner, usable 1 s later.
            if let Some(t) = match_over && t >= 2.4 {
                let b = match_buttons(world);
                // Unlocked: focus goes to 5 MORE ROUNDS, else REMATCH (Godot `_grab_rematch_overlay_focus`).
                if !self.rematch_armed { self.rematch_armed = true; self.rematch.focus = if world.coop { 0 } else { 1 }; }
                if let app::ui::MenuEvent::Select(k) = self.rematch.update(c, b.len(), 1, dt) { return b[k].1; }
            } else { self.rematch = Default::default(); self.rematch_armed = false; }
            if c.pressed.has(Buttons::START) { self.open(Screen::Pause); }
            return Action::None;
        }
        self.copied = (self.copied - dt).max(0.0);
        self.join.tick(dt);
        self.name.tick(dt);
        // A pad typing: the picker takes the controls until done or cancelled.
        if let Some(p) = &mut self.picker {
            let Some(ev) = p.update(c, dt) else { return Action::None };
            if matches!(ev, TextEvent::Enter | TextEvent::Escape) { self.picker = None; }
            if ev == TextEvent::Escape { return Action::None; }
            return self.text(ev, ctx);
        }
        if self.screen == Screen::Pause && c.pressed.has(Buttons::START) { return Action::Resume; }
        let list = items(self.screen, ctx);
        let foci = lines(&list, ctx, self.join_ok(ctx));
        let was_name = self.text_focus(ctx) == Some(Field::Name);
        // Sliders: left and right, with repeat.
        let dx = (c.held.has(Buttons::RIGHT) as i32 - c.held.has(Buttons::LEFT) as i32) + if c.mv[0] > 0.6 { 1 } else if c.mv[0] < -0.6 { -1 } else { 0 };
        let dx = dx.signum();
        let step = if dx == 0 { self.held_x = 0; 0 } else if dx != self.held_x { self.held_x = dx; self.repeat = 0.35; dx } else { self.repeat -= dt; if self.repeat <= 0.0 { self.repeat = 0.08; dx } else { 0 } };
        let focus = foci.get(self.menu.focus.min(foci.len().saturating_sub(1))).cloned();
        if let Some((k, _)) = focus && let Item::Slider(s) = list[k] && step != 0 {
            if s == Slider::Music { self.music_db = (self.music_db + step as f32).clamp(MUSIC_DB_MIN, 0.0); return Action::None; }
            let (lo, hi) = if s == Slider::Mouse { (0.3, 3.0) } else { (0.3, 2.5) };
            let v = self.slider(s, ctx);
            let v = (((v + 0.05 * step as f32).clamp(lo, hi)) * 20.0).round() / 20.0;
            match s {
                Slider::Mouse => self.mouse = v,
                Slider::Stick => self.stick = v,
                Slider::Aim(i) => if let Some(name) = ctx.seats.get(i) { self.set_aim(name, v) },
                Slider::Music => {}
            }
            return Action::None;
        }
        if let Some((k, _)) = focus && list[k] == Item::Tilt && (step != 0 || c.pressed.has(Buttons::SOUTH)) { self.tilt = !self.tilt; return Action::None; }
        if let Some((k, _)) = focus && let Item::Party(i) = list[k] && step != 0 && let Some((v, lo, hi)) = ctx.party.as_ref().and_then(|p| p.0.get(i)).map(|e| (e.1, e.2, e.3)) {
            let next = (v + step as i64).clamp(lo, hi);
            return if next != v { Action::Party(i, next) } else { Action::None };
        }
        let ev = self.menu.update(&Controls { mv: [0.0, c.mv[1]], held: Buttons(c.held.0 & !(Buttons::LEFT | Buttons::RIGHT)), pressed: Buttons(c.pressed.0 & !(Buttons::LEFT | Buttons::RIGHT)), ..*c }, foci.len(), 1, dt);
        // Rows move left and right within themselves.
        if let Some((_, on)) = &focus && step != 0 { self.col = (self.col as i32 + step).clamp(0, on.len() as i32 - 1) as usize; }
        if ev == app::ui::MenuEvent::Moved { self.col = 0; }
        if was_name && self.text_focus(ctx) != Some(Field::Name) { self.commit_name(); }
        match ev {
            app::ui::MenuEvent::Select(n) => {
                let (k, on) = &foci[n];
                let j = on[self.col.min(on.len() - 1)];
                match list[*k] {
                    Item::Button(_, a, _) => a, Item::Row(r) => r[j].1, Item::Bots(_) => if j == 0 { Action::RemoveBot } else { Action::AddBot },
                    Item::Field(Field::Join) if j == 1 => Action::Join,
                    // A on a text field: the on-screen keyboard (a keyboard types directly).
                    Item::Field(Field::Join | Field::Name) => { self.picker = Some(Default::default()); Action::None }
                    _ => Action::None,
                }
            }
            app::ui::MenuEvent::Back => self.back(),
            _ => Action::None,
        }
    }

    fn slider(&self, s: Slider, ctx: &Ctx) -> f32 {
        match s { Slider::Music => self.music_db, Slider::Mouse => self.mouse, Slider::Stick => self.stick, Slider::Aim(i) => ctx.seats.get(i).map_or(self.stick, |n| self.aim_for(n)) }
    }
    /// A GameNight player's stick sensitivity (Godot `stick_sens_for`: the shared one until set).
    pub fn aim_for(&self, name: &str) -> f32 { self.aim.iter().find(|a| a.0 == name).map_or(self.stick, |a| a.1) }
    pub fn set_aim(&mut self, name: &str, v: f32) {
        match self.aim.iter_mut().find(|a| a.0 == name) { Some(a) => a.1 = v, None => self.aim.push((name.to_string(), v)) }
    }

    /// The settings as JSON (Godot keeps them in `user://settings.cfg`).
    pub fn settings_json(&self) -> serde_json::Value {
        serde_json::json!({ "name": self.player_name, "music_db": self.music_db, "mouse": self.mouse, "stick": self.stick, "tilt": self.tilt,
            "aim": self.aim.iter().map(|(n, v)| (n.clone(), serde_json::json!(v))).collect::<serde_json::Map<_, _>>() })
    }
    pub fn load_settings(&mut self, v: &serde_json::Value) {
        let f = |k: &str, lo: f32, hi: f32, d: f32| v.get(k).and_then(|x| x.as_f64()).map_or(d, |x| (x as f32).clamp(lo, hi));
        self.music_db = f("music_db", MUSIC_DB_MIN, 0.0, self.music_db);
        self.mouse = f("mouse", 0.3, 3.0, self.mouse);
        self.stick = f("stick", 0.3, 2.5, self.stick);
        self.tilt = v.get("tilt").and_then(|x| x.as_bool()).unwrap_or(self.tilt);
        if let Some(n) = v.get("name").and_then(|x| x.as_str()) { self.player_name = n.chars().take(20).collect::<String>().trim().to_string(); }
        if let Some(m) = v.get("aim").and_then(|x| x.as_object()) { self.aim = m.iter().filter_map(|(n, x)| Some((n.clone(), (x.as_f64()? as f32).clamp(0.3, 2.5)))).collect(); }
    }

    /// Esc or B: close one level.
    pub fn back(&mut self) -> Action {
        match self.screen {
            Screen::Settings { from_start } => { self.open(if from_start { Screen::Start } else { Screen::Pause }); Action::None }
            Screen::Advanced => { self.open(Screen::Pause); Action::None }
            Screen::Pause => Action::Resume,
            Screen::Start => Action::Quit,
            Screen::Play => { self.open(Screen::Pause); Action::None }
        }
    }

    pub fn draw(&self, d: &mut HudDrawList, world: &World, hud: &Hud, ctx: &Ctx) {
        self.hits.borrow_mut().clear();
        self.hit_size.set(d.size);
        let [w, h] = d.size.map(|v| v as f32);
        let u = h / DESIGN_H;
        let clip = [0.0, 0.0, w, h];
        if self.scoreboard { scoreboard(d, world, hud, u, clip); }
        if let Some(t) = hud.match_over && t >= 1.4 && !self.is_open() {
            let a = ((t - 1.4) / 0.35).min(1.0);
            let enabled = t >= 2.4;
            let b = match_buttons(world);
            let (bw, bh) = (260.0 * u, 46.0 * u);
            let y0 = h * 0.5 - (b.len() as f32 * (bh + 14.0 * u)) * 0.5 + 30.0 * u;
            for (k, (text, _)) in b.iter().enumerate() {
                let r = [w * 0.5 - bw * 0.5, y0 + k as f32 * (bh + 14.0 * u), bw, bh];
                if enabled { self.hit(r, k, 0); }
                arena_button(d, r, text, enabled && k == self.rematch.focus, enabled, a, u, clip);
            }
        }
        match self.screen {
            Screen::Play => {}
            Screen::Start => self.start(d, ctx, u, clip),
            Screen::Pause if ctx.party.is_some() => self.party_menu(d, ctx, "", u, clip),
            Screen::Advanced => self.party_menu(d, ctx, "ADVANCED", u, clip),
            Screen::Pause => self.pause(d, world, ctx, u, clip),
            Screen::Settings { from_start } => self.settings(d, ctx, from_start, u, clip),
        }
        if let Some(p) = &self.picker {
            // The on-screen keyboard over the lower half, in the menus' colours.
            let st = app::ui::Style { panel: [0.06, 0.03, 0.035, 1.0], item: [0.13, 0.11, 0.10, 0.98], focus: RED, text: bytes(BONE), muted: bytes(BONE_DIM), scale: u * 0.75 };
            p.draw(d, 0x6767_5000, [0.0, h * 0.42, w, h * 0.58], &st);
        }
    }

    fn focused(&self, list: &[Item], ctx: &Ctx) -> Option<(usize, usize)> {
        let f = lines(list, ctx, self.join_ok(ctx));
        f.get(self.menu.focus.min(f.len().saturating_sub(1))).map(|(k, on)| (*k, on[self.col.min(on.len() - 1)]))
    }

    fn start(&self, d: &mut HudDrawList, ctx: &Ctx, u: f32, clip: [f32; 4]) {
        // Godot's Dim (black 0.32) and the logo, 440 x 190 from y 20.
        d.rect(clip, [0.0, 0.0, 0.0, 0.32], clip);
        logo(d, [clip[2] * 0.5 - 220.0 * u, 20.0 * u, 440.0 * u, 190.0 * u], clip);
        let list = items(Screen::Start, ctx);
        let (pw, ph) = (350.0 * u, 446.0 * u);
        let panel = [52.0 * u, clip[3] * 0.5 - ph * 0.5, pw, ph];
        gothic_frame(d, panel, u, 424242, clip);
        // The VBox centres its content between the crest and the bottom trim.
        let ch = column_height(&list, u);
        let top = panel[1] + 42.0 * u + ((ph - 60.0 * u - ch) * 0.5).max(0.0);
        self.column(d, &list, [panel[0] + 22.0 * u, top, pw - 44.0 * u, ch], ctx, true, u, clip);
    }

    fn pause(&self, d: &mut HudDrawList, _world: &World, ctx: &Ctx, u: f32, clip: [f32; 4]) {
        d.rect(clip, [0.0, 0.0, 0.0, 0.55], clip);
        logo(d, [clip[2] * 0.5 - 250.0 * u, 15.0 * u, 500.0 * u, 180.0 * u], clip);
        let list = items(Screen::Pause, ctx);
        let inner = column_height(&list, u);
        let pw = 350.0 * u + 52.0 * u;
        let ph = inner + 44.0 * u;
        let panel = [clip[2] * 0.5 - pw * 0.5, (clip[3] * 0.5 - ph * 0.5).max(200.0 * u), pw, ph];
        menu_panel(d, panel, u, clip);
        self.column(d, &list, [panel[0] + 26.0 * u, panel[1] + 22.0 * u, pw - 52.0 * u, inner], ctx, false, u, clip);
        // The arena seed, bottom right.
        Label::new(BODY, 13.0 * u, [0.72, 0.72, 0.85, 0.85]).outline(3.0 * u, [0.0, 0.0, 0.0, 0.85]).right()
            .draw(d, &format!("arena seed {}", ctx.seed), clip[2] - 12.0 * u, clip[3] - 18.0 * u, clip);
    }

    /// Godot `_gn_scaffold`: the slab, the logo, the status line, an
    /// optional heading and the item column (400 wide from x 38).
    fn party_menu(&self, d: &mut HudDrawList, ctx: &Ctx, heading: &str, u: f32, clip: [f32; 4]) {
        slab(d, u, clip);
        logo(d, [40.0 * u, 28.0 * u, 408.0 * u, 170.0 * u], clip);
        let status = ctx.party.as_ref().map_or("", |p| p.1.as_str());
        Label::new(bold(), 20.0 * u, RED).draw(d, status, 56.0 * u, 224.0 * u, clip);
        let mut top = 254.0 * u;
        if !heading.is_empty() {
            Label::poster(34.0 * u).shadow(3.0 * u, RED).draw(d, heading, 52.0 * u, top + 24.0 * u, clip);
            top += 54.0 * u;
        }
        let list = items(self.screen, ctx);
        self.column(d, &list, [38.0 * u, top, 400.0 * u, clip[3] - top], ctx, false, u, clip);
    }

    fn settings(&self, d: &mut HudDrawList, ctx: &Ctx, from_start: bool, u: f32, clip: [f32; 4]) {
        d.rect(clip, [0.0, 0.0, 0.0, 0.55], clip);
        let list = items(self.screen, ctx);
        let inner = column_height(&list, u) + 60.0 * u;
        let pw = 350.0 * u;
        if from_start {
            let ph = inner + 84.0 * u;
            let panel = [62.0 * u, clip[3] * 0.5 - ph * 0.5, pw, ph];
            gothic_frame(d, panel, u, 424242, clip);
            Label::poster(size::M * u).center().draw(d, "SETTINGS", panel[0] + pw * 0.5, panel[1] + 70.0 * u, clip);
            self.column(d, &list, [panel[0] + 30.0 * u, panel[1] + 110.0 * u, pw - 60.0 * u, inner], ctx, true, u, clip);
        } else {
            let ph = inner + 40.0 * u;
            let panel = [clip[2] * 0.5 - pw * 0.5 - 20.0 * u, clip[3] * 0.5 - ph * 0.5, pw + 40.0 * u, ph];
            menu_panel(d, panel, u, clip);
            Label::poster(size::M * u).center().draw(d, "SETTINGS", panel[0] + panel[2] * 0.5, panel[1] + 38.0 * u, clip);
            self.column(d, &list, [panel[0] + 40.0 * u, panel[1] + 80.0 * u, pw - 40.0 * u, inner], ctx, false, u, clip);
        }
    }

    /// A column of items into `r`; `gothic` styles buttons as the start screen does.
    fn column(&self, d: &mut HudDrawList, list: &[Item], r: [f32; 4], ctx: &Ctx, gothic: bool, u: f32, clip: [f32; 4]) {
        let focus = self.focused(list, ctx);
        let mut y = r[1];
        // The GameNight menu: big left-set buttons 46 tall, 4 apart.
        let gn = ctx.party.is_some() && matches!(self.screen, Screen::Pause | Screen::Advanced);
        let sep = if gn { 4.0 } else { 8.0 } * u;
        let cream = [0.94, 0.84, 0.64, 1.0];
        for (k, it) in list.iter().enumerate() {
            let fk = |j: usize| focus == Some((k, j));
            match *it {
                Item::Text(t, c) => {
                    let c = if gothic { cream } else { c };
                    Label::new(BODY, 14.0 * u, c).draw(d, t, r[0], y + 10.0 * u, clip);
                    y += 20.0 * u;
                }
                Item::Note(t) => {
                    let (text, c) = match t {
                        "BOTS" => (format!("{}", ctx.bots), [1.0, 0.95, 0.78, 1.0]),
                        "Paste an iroh game ID" => {
                            // Godot `_update_join_form`'s notice and colours.
                            let (t, c) = match self.join_state(ctx) {
                                JoinState::Connecting => ("Connecting...", [0.76, 0.92, 1.0, 1.0]),
                                JoinState::Empty => ("Paste an iroh game ID", [0.55, 0.6, 0.72, 1.0]),
                                JoinState::Own => ("That's your own game ID — share it with someone else", [1.0, 0.72, 0.42, 1.0]),
                                JoinState::Valid => ("Valid ID - connecting automatically", [0.58, 1.0, 0.65, 1.0]),
                                JoinState::Invalid => ("Paste the full iroh game ID", [1.0, 0.62, 0.5, 1.0]),
                            };
                            (t.to_string(), c)
                        }
                        _ => (t.to_string(), [0.57, 0.52, 0.44, 1.0]),
                    };
                    let size = if t == "BOTS" { 24.0 } else if gothic { 11.0 } else { 12.0 } * u;
                    let lab = if t == "BOTS" { Label::poster(size).center() } else { Label::new(BODY, size, c).center() };
                    let lab = if t == "BOTS" { lab } else { Label { color: c, ..lab } };
                    lab.draw(d, &text, r[0] + r[2] * 0.5, y + size * 0.65, clip);
                    y += size * 1.3;
                }
                Item::Field(Field::Name) => {
                    // Godot's name row: the label (128 wide), then the LineEdit (34 high).
                    let rh = 36.0 * u;
                    let mid = y + rh * 0.5;
                    Label::new(BODY, 14.0 * u, if fk(0) { BONE } else { [0.85, 0.85, 0.95, 1.0] }).draw(d, "Player name", r[0], mid, clip);
                    let fr = [r[0] + 142.0 * u, y + u, r[2] - 142.0 * u, 34.0 * u];
                    self.hit(fr, k, 0);
                    self.field_box(d, fr, gothic, fk(0), u, clip);
                    let inner = [fr[0] + 10.0 * u, fr[1], fr[2] - 20.0 * u, fr[3]];
                    self.name.draw(d, super::style::next_id(d), inner, BODY, 14.0 * u, bytes(cream), ("Your callsign…", bytes([0.55, 0.5, 0.42, 1.0])), fk(0).then_some(BONE));
                    y += rh + 14.0 * u;
                }
                Item::Field(Field::Join) => {
                    // Godot's join row: the field, then JOIN (70 wide) 8 to its right.
                    let fh = 34.0 * u;
                    let bw = 70.0 * u;
                    let fr = [r[0], y, r[2] - bw - 8.0 * u, fh];
                    if !ctx.client { self.hit(fr, k, 0); }
                    self.field_box(d, fr, gothic, fk(0), u, clip);
                    let inner = [fr[0] + 10.0 * u, fr[1], fr[2] - 20.0 * u, fr[3]];
                    let placeholder = if gothic { "Paste friend's Game ID..." } else { "Paste a Game ID…" };
                    self.join.draw(d, super::style::next_id(d), inner, BODY, 13.0 * u, bytes(cream), (placeholder, bytes([0.55, 0.5, 0.42, 1.0])), fk(0).then_some(BONE));
                    let br = [fr[0] + fr[2] + 8.0 * u, y, bw, fh];
                    let ok = self.join_ok(ctx);
                    if ok { self.hit(br, k, 1); }
                    let label = if self.joining { "..." } else { "JOIN" };
                    if gothic { gothic_button(d, br, label, fk(1), ok, u, clip) } else { arena_button(d, br, label, fk(1), ok, 1.0, u, clip) }
                    y += fh + sep;
                }
                Item::Field(f) => {
                    let fh = 34.0 * u;
                    let _ = f;
                    let (text, placeholder) = (ctx.ticket.map(|t| t.to_string()), if gothic { "Offline — click Host online" } else { "(offline — not hosting)" });
                    let fr = [r[0], y, r[2], fh];
                    if gothic {
                        d.rect(fr, [0.07, 0.06, 0.055, 0.98], clip);
                        frame_rect(d, fr, u.max(1.0), darkened([0.48, 0.34, 0.14, 1.0], 0.15), clip);
                    } else {
                        d.rect(fr, [0.1, 0.08, 0.09, 0.9], clip);
                        frame_rect(d, fr, u.max(1.0), [0.25, 0.2, 0.22, 1.0], clip);
                    }
                    let inner = [fr[0] + 10.0 * u, fr[1], fr[2] - 20.0 * u, fr[3]];
                    match text {
                        Some(t) => { let s = fit(&t, BODY, 13.0 * u, inner[2]); Label::new(BODY, s, cream).draw(d, &t, inner[0], fr[1] + fh * 0.5, inner); }
                        None => Label::new(BODY, 13.0 * u, [0.55, 0.5, 0.42, 1.0]).draw(d, placeholder, inner[0], fr[1] + fh * 0.5, inner),
                    }
                    y += fh + sep;
                }
                Item::Button(t, _, enabled) => {
                    let bh = if gn { 46.0 } else { 32.0 } * u;
                    let br = [r[0], y, r[2], bh];
                    if enabled { self.hit(br, k, 0); }
                    if gothic { gothic_button(d, br, t, fk(0), enabled, u, clip) } else if gn { gn_button(d, br, t, fk(0), enabled, u, clip) } else { arena_button(d, br, t, fk(0), enabled, 1.0, u, clip) }
                    y += bh + sep;
                }
                Item::Bots(label) => {
                    // Godot: "-" 46 x 46, the count (display 24) in 44, "+" 46 x 46.
                    let bh = if gn { 46.0 } else { 34.0 } * u;
                    let bw = bh;
                    let cw = 44.0 * u;
                    let x0 = if label.is_empty() { r[0] + r[2] * 0.5 - (bw * 2.0 + cw + 16.0 * u) * 0.5 } else { r[0] + r[2] - bw * 2.0 - cw - 16.0 * u };
                    if !label.is_empty() { Label::new(bold(), 28.0 * u, BONE_DIM).draw(d, label, r[0] + 14.0 * u, y + bh * 0.5, clip); }
                    for (j, t) in ["-", "+"].iter().enumerate() {
                        let bx = if j == 0 { x0 } else { x0 + bw + cw + 16.0 * u };
                        let on = bots_enabled(j, ctx);
                        if on { self.hit([bx, y, bw, bh], k, j); }
                        if gothic { gothic_button(d, [bx, y, bw, bh], t, fk(j), on, u, clip) } else if gn { gn_button(d, [bx, y, bw, bh], t, fk(j), on, u, clip) } else { arena_button(d, [bx, y, bw, bh], t, fk(j), on, 1.0, u, clip) }
                    }
                    Label::poster(24.0 * u).shadow(2.0 * u, RED).center().draw(d, &ctx.bots.to_string(), x0 + bw + 8.0 * u + cw * 0.5, y + bh * 0.5, clip);
                    y += bh + sep;
                }
                Item::Party(i) => {
                    let Some(&(name, v, lo, hi)) = ctx.party.as_ref().and_then(|p| p.0.get(i)) else { continue };
                    let rh = 36.0 * u;
                    let mid = y + rh * 0.5;
                    Label::new(bold(), 17.0 * u, if fk(0) { BONE } else { BONE_DIM }).draw(d, name, r[0] + 14.0 * u, mid, clip);
                    let (sx, sw) = (r[0] + 250.0 * u, r[2] - 250.0 * u - 52.0 * u);
                    self.hit([sx, y, sw, rh], k, 0);
                    d.rect([sx, mid - 2.0 * u, sw, 4.0 * u], [0.25, 0.22, 0.24, 1.0], clip);
                    let k = ((v - lo) as f32 / (hi - lo).max(1) as f32).clamp(0.0, 1.0);
                    d.rect([sx, mid - 2.0 * u, sw * k, 4.0 * u], if fk(0) { RED } else { [0.6, 0.55, 0.55, 1.0] }, clip);
                    d.circle([sx + sw * k, mid], 7.0 * u, if fk(0) { BONE } else { [0.8, 0.78, 0.75, 1.0] }, clip);
                    Label::new(bold(), 17.0 * u, BONE).right().draw(d, &v.to_string(), r[0] + r[2], mid, clip);
                    y += rh + sep;
                }
                Item::Row(row) => {
                    let bh = 32.0 * u;
                    let gap = 8.0 * u;
                    let n = row.len() as f32;
                    let bw = (r[2] - gap * (n - 1.0)) / n;
                    for (j, (t, _)) in row.iter().enumerate() {
                        let br = [r[0] + j as f32 * (bw + gap), y, bw, bh];
                        let enabled = row_enabled(t, ctx);
                        let t = if *t == "COPY" && self.copied > 0.0 { "COPIED!" } else { *t };
                        if enabled { self.hit(br, k, j); }
                        if gothic { gothic_button(d, br, t, fk(j), enabled, u, clip) } else { arena_button(d, br, t, fk(j), enabled, 1.0, u, clip) }
                    }
                    y += bh + sep;
                }
                Item::Slider(s) => {
                    let v = self.slider(s, ctx);
                    let (name, lo, hi) = match s {
                        Slider::Music => ("Music".to_string(), MUSIC_DB_MIN, 0.0),
                        Slider::Mouse => ("Mouse sensitivity".to_string(), 0.3, 3.0),
                        Slider::Stick => ("Stick sensitivity".to_string(), 0.3, 2.5),
                        Slider::Aim(i) => (format!("Aim: {}", ctx.seats.get(i).map_or("", |n| n.as_str())), 0.3, 2.5),
                    };
                    let value = if s == Slider::Music { music_label(v) } else { format!("{v:.2}x") };
                    let rh = 36.0 * u;
                    let mid = y + rh * 0.5;
                    let lc = if fk(0) { BONE } else { [0.85, 0.85, 0.95, 1.0] };
                    Label::new(BODY, fit(&name, BODY, 14.0 * u, 136.0 * u), lc).draw(d, &name, r[0], mid, clip);
                    let (sx, sw) = (r[0] + 142.0 * u, r[2] - 142.0 * u - 59.0 * u);
                    self.hit([sx, y, sw, rh], k, 0);
                    d.rect([sx, mid - 2.0 * u, sw, 4.0 * u], [0.25, 0.22, 0.24, 1.0], clip);
                    let k = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                    d.rect([sx, mid - 2.0 * u, sw * k, 4.0 * u], if fk(0) { RED } else { [0.6, 0.55, 0.55, 1.0] }, clip);
                    d.circle([sx + sw * k, mid], 7.0 * u, if fk(0) { BONE } else { [0.8, 0.78, 0.75, 1.0] }, clip);
                    Label::new(BODY, 14.0 * u, [0.74, 0.78, 0.9, 1.0]).right().draw(d, &value, r[0] + r[2], mid, clip);
                    y += rh + 14.0 * u;
                }
                Item::Tilt => {
                    // Godot's CheckButton: the text, and a switch on the right.
                    let rh = 30.0 * u;
                    self.hit([r[0], y, r[2], rh], k, 0);
                    let mid = y + rh * 0.5;
                    Label::new(BODY, 14.0 * u, if fk(0) { BONE } else { [0.85, 0.85, 0.95, 1.0] }).draw(d, "Movement tilt", r[0], mid, clip);
                    let (sw, sh) = (40.0 * u, 20.0 * u);
                    let sx = r[0] + r[2] - sw;
                    let on = self.tilt;
                    let track = if on { RED } else { [0.25, 0.22, 0.24, 1.0] };
                    d.rect([sx + sh * 0.5, mid - sh * 0.5, sw - sh, sh], track, clip);
                    d.circle([sx + sh * 0.5, mid], sh * 0.5, track, clip);
                    d.circle([sx + sw - sh * 0.5, mid], sh * 0.5, track, clip);
                    d.circle([if on { sx + sw - sh * 0.5 } else { sx + sh * 0.5 }, mid], sh * 0.38, if fk(0) { BONE } else { [0.8, 0.78, 0.75, 1.0] }, clip);
                    y += rh + 14.0 * u;
                }
                Item::Gap(g) => y += g * u,
                Item::Rule => { d.rect([r[0] - 2.0 * u, y + 3.0 * u, r[2] + 4.0 * u, u.max(1.0)], [0.48, 0.34, 0.14, 1.0], clip); y += 8.0 * u + sep; }
            }
        }
        if let Some(n) = &self.notice { Label::new(BODY, 12.0 * u, [1.0, 0.45, 0.35, 1.0]).outline(3.0 * u, [0.0, 0.0, 0.0, 0.85]).center().draw(d, n, r[0] + r[2] * 0.5, y + 8.0 * u, clip); }
    }
}

impl Menus {
    /// A text field's box: the start screen's stone with a gold edge, or the
    /// pause menu's dark plate; brighter edged while it has the focus.
    fn field_box(&self, d: &mut HudDrawList, fr: [f32; 4], gothic: bool, focused: bool, u: f32, clip: [f32; 4]) {
        if gothic {
            d.rect(fr, [0.07, 0.06, 0.055, 0.98], clip);
            frame_rect(d, fr, if focused { 2.0 * u } else { u.max(1.0) }, if focused { [0.95, 0.72, 0.28, 1.0] } else { darkened([0.48, 0.34, 0.14, 1.0], 0.15) }, clip);
        } else {
            d.rect(fr, [0.1, 0.08, 0.09, 0.9], clip);
            frame_rect(d, fr, if focused { 2.0 * u } else { u.max(1.0) }, if focused { RED } else { [0.25, 0.2, 0.22, 1.0] }, clip);
        }
    }
}

fn column_height(list: &[Item], u: f32) -> f32 {
    let sep = 8.0;
    list.iter().map(|it| match it {
        Item::Bots(_) => 34.0 + sep, Item::Party(_) => 36.0 + sep,
        Item::Text(..) => 20.0, Item::Note(t) => if *t == "BOTS" { 24.0 * 1.3 } else { 12.0 * 1.3 },
        Item::Field(Field::Name) => 50.0, Item::Field(_) => 34.0 + sep, Item::Button(..) | Item::Row(_) => 32.0 + sep, Item::Slider(_) => 50.0, Item::Tilt => 44.0, Item::Gap(g) => *g, Item::Rule => 8.0 + sep,
    }).sum::<f32>() * u
}

/// The GameNight menu's buttons: Barlow ExtraBold 28, left-set 14 in,
/// a slanted red plate when focused.
fn gn_button(d: &mut HudDrawList, r: [f32; 4], text: &str, focused: bool, enabled: bool, u: f32, clip: [f32; 4]) {
    if focused && enabled { plate(d, r, RED, None, clip); }
    let c = if !enabled { with_alpha(BONE, 0.3) } else if focused { BONE } else { BONE_DIM };
    let lab = Label::new(bold(), fit(text, bold(), 28.0 * u, r[2] - 20.0 * u), c);
    if text.len() <= 1 { lab.center().draw(d, text, r[0] + r[2] * 0.5, r[1] + r[3] * 0.5, clip) } else { lab.draw(d, text, r[0] + 14.0 * u, r[1] + r[3] * 0.5, clip) }
}

/// `arena_menu_backdrop.gd`: the match dimmed, a dark slab down the left
/// 42 % wide with a slanted red edge and faint red hatching.
fn slab(d: &mut HudDrawList, u: f32, clip: [f32; 4]) {
    let [w, h] = [clip[2], clip[3]];
    d.rect(clip, [0.0, 0.0, 0.0, 0.42], clip);
    let top = w * 0.42;
    let bottom = top - h * SKEW * 0.5;
    d.polygon(&[[0.0, 0.0], [top, 0.0], [bottom, h], [0.0, h]], with_alpha(darkened(INK, 0.3), 0.94), clip);
    let mut x = -h;
    while x < top {
        // Hatching from bottom left to top right, kept inside the slab's narrower edge.
        let (a, b) = ([x, h], [x + h, 0.0]);
        let inside = [0.0, 0.0, bottom, h];
        d.line(a, b, 3.0 * u, with_alpha(RED, 0.05), inside);
        x += 14.0 * u;
    }
    let edge = 8.0 * u;
    d.polygon(&[[top, 0.0], [top + edge, 0.0], [bottom + edge, h], [bottom, h]], RED, clip);
    d.line([top - 14.0 * u, 0.0], [bottom - 14.0 * u, h], 2.0 * u, with_alpha(BONE, 0.12), clip);
}

/// Godot's Arena button: no plate until focused, then a slanted red plate.
fn arena_button(d: &mut HudDrawList, r: [f32; 4], text: &str, focused: bool, enabled: bool, alpha: f32, u: f32, clip: [f32; 4]) {
    if focused { plate(d, r, with_alpha(RED, alpha), None, clip); }
    let c = if !enabled { with_alpha(BONE, 0.3) } else if focused { BONE } else { BONE_DIM };
    let size = fit(text, bold(), 22.0 * u, r[2] - 12.0 * u);
    Label::new(bold(), size, with_alpha(c, alpha)).center().draw(d, text, r[0] + r[2] * 0.5, r[1] + r[3] * 0.5, clip);
}

const GOLD_DIM: C = [0.48, 0.34, 0.14, 1.0];
const GOTHIC_GOLD: C = [0.78, 0.58, 0.22, 1.0];

/// The start screen's stone buttons with a gold edge.
fn gothic_button(d: &mut HudDrawList, r: [f32; 4], text: &str, focused: bool, enabled: bool, u: f32, clip: [f32; 4]) {
    let (bg, edge, ew) = if !enabled { ([0.10, 0.09, 0.08, 0.75], darkened(GOLD_DIM, 0.35), 1.0) } else if focused { ([0.18, 0.14, 0.11, 0.98], [0.95, 0.72, 0.28, 1.0], 2.0) } else { ([0.13, 0.11, 0.10, 0.98], GOLD_DIM, 1.0) };
    d.rect(r, bg, clip);
    frame_rect(d, r, (ew * u).max(1.0), edge, clip);
    let c = if !enabled { darkened([0.62, 0.56, 0.48, 1.0], 0.25) } else if focused { [1.0, 0.94, 0.78, 1.0] } else { [0.92, 0.82, 0.62, 1.0] };
    let size = fit(text, bold(), 15.0 * u, r[2] - 8.0 * u);
    Label::new(bold(), size, c).center().draw(d, text, r[0] + r[2] * 0.5, r[1] + r[3] * 0.5, clip);
}

/// A tiny deterministic random sequence for the stone texture.
struct Lcg(u32);
impl Lcg { fn f(&mut self) -> f32 { self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223); (self.0 >> 8) as f32 / 16777216.0 } }

/// `gothic_menu_ui.draw_frame`: stone tiles, iron edge with crown spikes,
/// gold inner trim, corner brackets and a skull crest.
pub fn gothic_frame(d: &mut HudDrawList, rect: [f32; 4], u: f32, seed: u32, clip: [f32; 4]) {
    let mut rng = Lcg(seed);
    let spike_h = 16.0 * u;
    let outer = [rect[0] - 2.0 * u, rect[1] - 2.0 * u - spike_h, rect[2] + 4.0 * u, rect[3] + 4.0 * u + spike_h];
    let base = [0.11, 0.10, 0.095, 0.97];
    d.rect(outer, base, clip);
    let cell = 14.0 * u;
    let (cols, rows) = ((outer[2] / cell).ceil() as usize, (outer[3] / cell).ceil() as usize);
    for yy in 0..rows {
        for xx in 0..cols {
            let n = rng.f();
            let c = if n < 0.22 { [0.06, 0.055, 0.05, 1.0] } else if n > 0.82 { [0.20, 0.18, 0.16, 1.0] } else { base };
            let (tx, ty) = (outer[0] + xx as f32 * cell, outer[1] + yy as f32 * cell);
            let (tw, th) = ((cell - u).min(outer[0] + outer[2] - tx), (cell - u).min(outer[1] + outer[3] - ty));
            if c != base && tw > 0.0 && th > 0.0 { d.rect([tx, ty, tw, th], c, clip); }
            if rng.f() > 0.72 { d.rect([tx + 2.0 * u + rng.f() * (cell - 5.0 * u), ty + 2.0 * u + rng.f() * (cell - 5.0 * u), 2.0 * u, u], [0.24, 0.22, 0.2, 1.0], clip); }
        }
    }
    let iron = [0.04, 0.035, 0.04, 1.0];
    frame_rect(d, outer, 3.0 * u, iron, clip);
    let count = ((outer[2] / (18.0 * u)).floor() as usize).max(3);
    let span = outer[2] / count as f32;
    for i in 0..count {
        let cx = outer[0] + span * (i as f32 + 0.5);
        let hh = spike_h * (0.75 + rng.f() * 0.3);
        d.triangle([[cx - span * 0.34, outer[1] + 2.0 * u], [cx, outer[1] - hh], [cx + span * 0.34, outer[1] + 2.0 * u]], iron, clip);
    }
    for side in [-1.0f32, 1.0] {
        let x = if side < 0.0 { outer[0] } else { outer[0] + outer[2] };
        d.triangle([[x, outer[1] + outer[3] * 0.18], [x + 8.0 * u * side, outer[1] + outer[3] * 0.28], [x, outer[1] + outer[3] * 0.38]], iron, clip);
    }
    let gi = [rect[0] + 5.0 * u, rect[1] + 5.0 * u, rect[2] - 10.0 * u, rect[3] - 10.0 * u];
    frame_rect(d, gi, u.max(1.0), GOLD_DIM, clip);
    frame_rect(d, [gi[0] + u, gi[1] + u, gi[2] - 2.0 * u, gi[3] - 2.0 * u], u.max(1.0), darkened(GOTHIC_GOLD, 0.25), clip);
    let b = [rect[0] + 7.0 * u, rect[1] + 7.0 * u, rect[2] - 14.0 * u, rect[3] - 14.0 * u];
    let len = 16.0 * u;
    for (cx, cy, dx, dy) in [(b[0], b[1], 1.0, 1.0), (b[0] + b[2], b[1], -1.0, 1.0), (b[0], b[1] + b[3], 1.0, -1.0), (b[0] + b[2], b[1] + b[3], -1.0, -1.0)] {
        d.rect([cx.min(cx + dx * len), cy - u, len, 2.0 * u], GOTHIC_GOLD, clip);
        d.rect([cx - u, cy.min(cy + dy * len), 2.0 * u, len], GOTHIC_GOLD, clip);
    }
    // Skull crest under the spikes.
    let (cx, cy) = (rect[0] + rect[2] * 0.5, rect[1] + spike_h + 18.0 * u);
    d.circle([cx, cy - 2.0 * u], 12.0 * u, [0.72, 0.68, 0.62, 1.0], clip);
    d.circle([cx, cy - 2.0 * u], 11.0 * u, [0.58, 0.54, 0.5, 1.0], clip);
    d.rect([cx - 7.0 * u, cy + 4.0 * u, 14.0 * u, 7.0 * u], [0.58, 0.54, 0.5, 1.0], clip);
    d.circle([cx - 4.5 * u, cy - u], 2.6 * u, [0.08, 0.06, 0.05, 1.0], clip);
    d.circle([cx + 4.5 * u, cy - u], 2.6 * u, [0.08, 0.06, 0.05, 1.0], clip);
    d.triangle([[cx, cy + 2.0 * u], [cx - 2.5 * u, cy + 7.0 * u], [cx + 2.5 * u, cy + 7.0 * u]], [0.1, 0.08, 0.07, 1.0], clip);
    d.arc([cx, cy - 2.0 * u], 13.0 * u, std::f32::consts::PI * 1.08, std::f32::consts::PI * 1.92, 1.5 * u, GOLD_DIM, clip);
}

static LOGO_PNG: &[u8] = include_bytes!("../../../assets/branding/logo.png");
const LOGO: render::hud::HudImage = render::hud::HudImage(0x6767_0001);

fn logo_picture() -> Option<&'static std::sync::Arc<render::picture::Picture>> {
    static P: std::sync::OnceLock<Option<std::sync::Arc<render::picture::Picture>>> = std::sync::OnceLock::new();
    P.get_or_init(|| render::picture::decode(LOGO_PNG, 1.0).map_err(|e| eprintln!("logo: {e}")).ok().map(std::sync::Arc::new)).as_ref()
}

/// The cover-art logo (`assets/branding/logo.png`) kept to its aspect and
/// centred in `r` (Godot `STRETCH_KEEP_ASPECT_CENTERED`).
pub fn logo(d: &mut HudDrawList, r: [f32; 4], clip: [f32; 4]) {
    let Some(p) = logo_picture() else { return logo_text(d, [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5], r[3] * 0.9, clip) };
    let aspect = p.width as f32 / p.height.max(1) as f32;
    let (w, h) = if r[2] / r[3] > aspect { (r[3] * aspect, r[3]) } else { (r[2], r[2] / aspect) };
    d.provide_image(LOGO, p, render::hud::ImageFilter::Linear);
    d.image([r[0] + (r[2] - w) * 0.5, r[1] + (r[3] - h) * 0.5, w, h], LOGO, [0.0, 0.0, 1.0, 1.0], [1.0; 4], clip);
}

/// The logo in poster type, should the picture not decode.
fn logo_text(d: &mut HudDrawList, center: [f32; 2], height: f32, clip: [f32; 4]) {
    let size = height * 0.5;
    for (k, word) in ["GROWING", "GUNS"].iter().enumerate() {
        let y = center[1] - height * 0.24 + k as f32 * height * 0.48;
        let lab = Label::new(display(), size, [0.98, 0.93, 0.84, 1.0]).center();
        for s in (1..=5).rev() {
            let o = s as f32 * size * 0.022;
            let c = if s > 3 { [0.32, 0.06, 0.05, 1.0] } else { [0.93, 0.33, 0.12, 1.0] };
            Label { color: c, ..lab }.draw(d, word, center[0] + o * 0.6, y + o, clip);
        }
        lab.draw(d, word, center[0], y, clip);
    }
}

/// The Tab scoreboard (Godot `_build_tab_overlay`).
fn scoreboard(d: &mut HudDrawList, world: &World, hud: &Hud, u: f32, clip: [f32; 4]) {
    let [w, h] = [clip[2], clip[3]];
    let rows: Vec<usize> = (0..world.players.len()).filter(|&i| !world.coop || i < world.waves.seats || world.players[i].alive).collect();
    let pr = [w * 0.5 - 360.0 * u, h * 0.12, 720.0 * u, h * 0.76];
    d.rect(pr, [0.04, 0.04, 0.07, 0.93], clip);
    frame_rect(d, pr, 2.0 * u, [1.0, 0.85, 0.4, 0.8], clip);
    let x0 = pr[0] + 20.0 * u;
    let x1 = pr[0] + pr[2] - 20.0 * u;
    let mut y = pr[1] + 16.0 * u;
    let head = if world.coop { format!("CO-OP WAVE {}", world.waves.wave) } else { "SCOREBOARD".into() };
    Label::poster(size::M * u).center().draw(d, &head, w * 0.5, y + 17.0 * u, clip);
    y += 38.0 * u;
    let sub = if world.coop { "clear the wave, choose cards, survive".to_string() } else { format!("first to {} rounds wins", world.rounds_to_win) };
    Label::new(BODY, 13.0 * u, [0.65, 0.65, 0.8, 1.0]).center().draw(d, &sub, w * 0.5, y + 8.0 * u, clip);
    y += 22.0 * u;
    d.rect([x0, y, x1 - x0, u.max(1.0)], [0.4, 0.4, 0.5, 0.6], clip);
    y += 10.0 * u;
    for i in rows {
        if y > pr[1] + pr[3] - 30.0 * u { break; }
        let p = &world.players[i];
        Label::new(BODY, 20.0 * u, [1.0, 1.0, 1.0, 1.0]).draw(d, hud.name(i), x0, y + 12.0 * u, clip);
        let score = if world.coop { if world.waves.is_enemy(i) { "ENEMY".into() } else { "ALLY".into() } } else { format!("{} / {}", p.score, world.rounds_to_win) };
        Label::new(BODY, 20.0 * u, [1.0, 0.85, 0.4, 1.0]).right().draw(d, &score, x1 - 92.0 * u, y + 12.0 * u, clip);
        let (ping, pc) = if p.bot { ("BOT", [0.55, 0.55, 0.65, 1.0]) } else { ("-- ms", [0.65, 0.65, 0.75, 1.0]) };
        Label::new(BODY, 16.0 * u, pc).right().draw(d, ping, x1, y + 12.0 * u, clip);
        y += 28.0 * u;
        if p.cards.is_empty() {
            Label::new(BODY, 12.0 * u, [0.5, 0.5, 0.6, 1.0]).draw(d, "    (no cards)", x0, y + 7.0 * u, clip);
            y += 18.0 * u;
        } else {
            let mut x = x0;
            for &c in &p.cards {
                let Some(def) = world.cards.get(c) else { continue };
                let col = rgb(def.color);
                let lab = Label::new(BODY, 12.0 * u, col);
                let pw = lab.width(&def.name) + 14.0 * u;
                if x + pw > x1 { x = x0; y += 22.0 * u; }
                let r = [x, y, pw, 18.0 * u];
                d.rect(r, [col[0] * 0.22, col[1] * 0.22, col[2] * 0.22, 0.95], clip);
                frame_rect(d, r, u.max(1.0), col, clip);
                lab.draw(d, &def.name, x + 7.0 * u, y + 9.0 * u, clip);
                x += pw + 6.0 * u;
            }
            y += 26.0 * u;
        }
        y += 4.0 * u;
    }
    let _ = Phase::Playing;
}

/// Godot `splitscreen_manager.gd` `_join_label`: "PRESS X TO JOIN" across
/// the top (22, gold, black outline 8) while seats are open.
pub fn join_prompt(d: &mut HudDrawList) {
    let [w, h] = d.size.map(|v| v as f32);
    let u = h / DESIGN_H;
    Label::label(size::S * u, [1.0, 0.9, 0.45, 1.0]).center().draw(d, "PRESS X TO JOIN", w * 0.5, 33.0 * u, [0.0, 0.0, w, h]);
}

/// Godot `_build_splitscreen_hint`: top left, 14, gold, outline 4.
pub fn splitscreen_hint(d: &mut HudDrawList) {
    let [w, h] = d.size.map(|v| v as f32);
    let u = h / DESIGN_H;
    Label::new(BODY, 14.0 * u, [1.0, 0.9, 0.45, 1.0]).outline(4.0 * u, [0.0, 0.0, 0.0, 0.85]).draw(d, "2-PLAYER: open menu (Esc) → SPLITSCREEN", 16.0 * u, 24.0 * u, [0.0, 0.0, w, h]);
}

/// The per-player PLAYER MENU (Godot `_build_pause_menu` in
/// splitscreen_manager.gd): a 340 x 184 panel in the screen's centre with
/// the title, "A SELECT   B RESUME" and RESUME / LEAVE, `focus` marked as
/// "> RESUME <". `who`: the player's name.
pub fn player_menu(d: &mut HudDrawList, who: &str, focus: usize) {
    let [w, h] = d.size.map(|v| v as f32);
    let u = h / DESIGN_H;
    let clip = [0.0, 0.0, w, h];
    let r = [w * 0.5 - 170.0 * u, h * 0.5 - 92.0 * u, 340.0 * u, 184.0 * u];
    menu_panel(d, r, u, clip);
    let cx = r[0] + r[2] * 0.5;
    Label::new(BODY, 18.0 * u, BONE).center().draw(d, &format!("PLAYER MENU  ·  {who}"), cx, r[1] + 26.0 * u, clip);
    Label::new(BODY, 13.0 * u, BONE_DIM).center().draw(d, "A SELECT   B RESUME", cx, r[1] + 50.0 * u, clip);
    for (k, t) in PLAYER_MENU.iter().enumerate() {
        let br = [cx - 105.0 * u, r[1] + 70.0 * u + k as f32 * 44.0 * u, 210.0 * u, 34.0 * u];
        let text = if k == focus { format!("> {t} <") } else { t.to_string() };
        arena_button(d, br, &text, k == focus, true, 1.0, u, clip);
    }
}

/// The PLAYER MENU's entries.
pub const PLAYER_MENU: [&str; 2] = ["RESUME", "LEAVE"];

#[cfg(test)]
mod pointer_tests {
    use super::*;
    const SIZE: [u32; 2] = [1600, 900];
    fn ctx() -> Ctx<'static> { Ctx { ticket: None, client: false, seed: 1, bots: 1, players: 2, coop: false, party: None, seats: vec![] } }
    fn at(r: [f32; 4], click: bool) -> app::Pointer {
        app::Pointer { position: Some([r[0] + r[2] * 0.5, r[1] + r[3] * 0.5]), moved: true, pressed: click, held: click, wheel: 0.0 }
    }
    fn draw(m: &Menus, w: &World, h: &Hud, c: &Ctx) { m.draw(&mut HudDrawList::new(SIZE), w, h, c); }

    #[test]
    fn painted_buttons_hover_click_and_reject_stale_targets() {
        let w = World::new(1, &[false, true]);
        let h = Hud::new(&w);
        let c = ctx();
        let mut m = Menus::default();
        m.open(Screen::Start);
        draw(&m, &w, &h, &c);
        let r = m.hits.borrow().iter().find(|h| h.item == 3).unwrap().rect;
        assert_eq!(m.pointer(at(r, false), SIZE, &w, None, &c), Action::None);
        assert_eq!(m.menu.focus, 2);
        m.menu.focus = 0;
        let mut stationary = at(r, false); stationary.moved = false;
        m.pointer(stationary, SIZE, &w, None, &c);
        assert_eq!(m.menu.focus, 0, "stationary mouse preserves keyboard focus");
        assert_eq!(m.pointer(at(r, true), SIZE, &w, None, &c), Action::Settings);
        assert_eq!(m.pointer(at(r, true), [800, 600], &w, None, &c), Action::None);
        m.open(Screen::Pause);
        assert_eq!(m.pointer(at(r, true), SIZE, &w, None, &c), Action::None);
    }

    #[test]
    fn disabled_buttons_and_match_reveal_cannot_be_clicked() {
        let w = World::new(1, &[false, true]);
        let mut h = Hud::new(&w);
        let mut c = ctx();
        let mut m = Menus::default();
        m.open(Screen::Start);
        draw(&m, &w, &h, &c);
        let host = m.hits.borrow().iter().find(|h| h.item == 7 && h.entry == 0).unwrap().rect;
        assert!(!m.hits.borrow().iter().any(|h| h.item == 7 && h.entry == 1));
        c.client = true;
        assert_eq!(m.pointer(at(host, true), SIZE, &w, None, &c), Action::None);
        m.open(Screen::Play);
        h.match_over = Some(2.0);
        draw(&m, &w, &h, &c);
        assert!(m.hits.borrow().is_empty());
        h.match_over = Some(2.5);
        draw(&m, &w, &h, &c);
        let r = m.hits.borrow()[1].rect;
        assert_eq!(m.pointer(at(r, true), SIZE, &w, h.match_over, &c), Action::Extend);
        assert_eq!(m.pointer(at(r, true), SIZE, &w, Some(1.0), &c), Action::None);
    }

    /// A ticket that parses (the ed25519 base point as the key, no addresses).
    pub(crate) fn fake_ticket() -> String {
        let key = format!("58{}", "66".repeat(31));
        [format!("{key}00"), format!("20{key}00"), format!("{key}0000")].into_iter().find(|t| net::parse_ticket(t).is_ok()).expect("a ticket encoding")
    }

    fn typed(m: &mut Menus, c: &Ctx, s: &str) -> Action { m.text(TextEvent::Insert(s.into()), c) }

    #[test]
    fn join_field_types_extracts_pasted_ids_and_joins_valid_ones() {
        let w = World::new(1, &[false, true]);
        let h = Hud::new(&w);
        let mut c = ctx();
        let mut m = Menus::default();
        m.open(Screen::Pause);
        assert_eq!(m.text_focus(&c), None, "the first focus is a button");
        assert!(m.focus_field(Field::Join, &c));
        assert_eq!(m.text_focus(&c), Some(Field::Join));
        assert_eq!(typed(&mut m, &c, "abc"), Action::None);
        assert_eq!(m.join_state(&c), JoinState::Invalid);
        assert_eq!(m.text(TextEvent::Enter, &c), Action::None, "no join with a bad ID");
        for _ in 0..3 { m.text(TextEvent::Backspace, &c); }
        assert_eq!(m.join_state(&c), JoinState::Empty);
        // A pasted block with the ID on its own line: the ID is kept and joins.
        let t = fake_ticket();
        assert_eq!(m.text(TextEvent::Paste(format!("Join me!\n{t}\n")), &c), Action::Join);
        assert_eq!(m.join.text(), t);
        assert_eq!(m.join_ticket(), t);
        assert_eq!(m.text(TextEvent::Enter, &c), Action::Join);
        // JOIN is entry 1 of the field's line, drawn and clickable.
        draw(&m, &w, &h, &c);
        let k = items(Screen::Pause, &c).iter().position(|i| *i == Item::Field(Field::Join)).unwrap();
        let b = m.hits.borrow().iter().find(|h| h.item == k && h.entry == 1).unwrap().rect;
        assert_eq!(m.pointer(at(b, true), SIZE, &w, None, &c), Action::Join);
        // Our own ID does not join.
        c.ticket = Some(Box::leak(t.clone().into_boxed_str()));
        assert_eq!(m.join_state(&c), JoinState::Own);
        assert_eq!(m.text(TextEvent::Enter, &c), Action::None);
        c.ticket = None;
        m.joining = true;
        assert_eq!(m.join_state(&c), JoinState::Connecting);
        m.joining = false;
        m.focus_field(Field::Join, &c);
        assert_eq!(m.text(TextEvent::Escape, &c), Action::Back);
        // Real 272-char multi-interface ticket and 500-char CLI one-liner pasted on the Start menu (even without focusing the field first).
        let long_ticket = "168ea06eeabb0bb330288edac13a55e25067fae474c43bff0de51fb693e44c3007002368747470733a2f2f657563312d312e72656c61792e6e302e69726f682e6c696e6b2e2f010052acbff0d9e5030100c0a8004cd9e5030100c0a88b03d9e5030100c0a8c200d9e5030100c0a8d700d9e503010102003798bf4c7b4ba3b6c672798b4d908ef403";
        assert!(long_ticket.len() > 256);
        m.open(Screen::Start);
        assert_eq!(m.text_focus(&c), None);
        let oneliner = format!("curl -fL http://192.168.0.76:8787/Growing-Guns-macOS.zip -o /tmp/gg.zip && \"/tmp/Growing Guns.app/Contents/MacOS/growing-guns\" play --join {long_ticket}");
        assert_eq!(m.text(TextEvent::Paste(oneliner), &c), Action::Join);
        assert_eq!(m.join_ticket(), long_ticket);
        assert_eq!(m.join_state(&c), JoinState::Valid);
        draw(&m, &w, &h, &c);
        let k = items(Screen::Start, &c).iter().position(|i| *i == Item::Field(Field::Join)).unwrap();
        let b = m.hits.borrow().iter().find(|h| h.item == k && h.entry == 1).unwrap().rect;
        assert_eq!(m.pointer(at(b, true), SIZE, &w, None, &c), Action::Join);
    }

    #[test]
    fn player_name_commits_trimmed_and_restores_empty_names() {
        let w = World::new(1, &[false, true]);
        let c = ctx();
        let mut m = Menus::default();
        m.player_name = "Joep".into();
        m.open(Screen::Settings { from_start: false });
        assert_eq!(m.name.text(), "Joep");
        assert_eq!(m.text_focus(&c), Some(Field::Name), "the name is the first line");
        for _ in 0..4 { m.text(TextEvent::Backspace, &c); }
        assert_eq!(m.text(TextEvent::Enter, &c), Action::None);
        assert_eq!((m.player_name.as_str(), m.name.text()), ("Joep", "Joep"), "an empty name is not kept");
        m.text(TextEvent::Insert("Ace ".into()), &c);
        // Moving off the field keeps the name, as Godot's focus_exited does.
        let down = Controls { held: Buttons(Buttons::DOWN), pressed: Buttons(Buttons::DOWN), ..Default::default() };
        m.update(&down, &w, None, &c, 0.016);
        assert_eq!(m.player_name, "JoepAce", "trimmed");
        assert_eq!(m.text_focus(&c), None);
        assert_eq!(m.settings_json()["name"], "JoepAce");
        let mut n = Menus::default();
        n.load_settings(&m.settings_json());
        assert_eq!(n.player_name, "JoepAce");
        // A pad: A on the field opens the on-screen keyboard; DONE closes it.
        m.menu.focus = 0;
        let press = |b: u32| Controls { held: Buttons(b), pressed: Buttons(b), ..Default::default() };
        m.update(&press(Buttons::SOUTH), &w, None, &c, 0.016);
        assert!(m.picker.is_some());
        m.update(&press(Buttons::SOUTH), &w, None, &c, 0.016);
        assert_eq!(m.name.text(), "JoepAce a");
        m.update(&press(Buttons::START), &w, None, &c, 0.016);
        assert!(m.picker.is_none());
        assert_eq!(m.player_name, "JoepAce a");
        // East on the picker cancels it without leaving the menu.
        m.update(&press(Buttons::SOUTH), &w, None, &c, 0.016);
        m.update(&press(Buttons::EAST), &w, None, &c, 0.016);
        assert!(m.picker.is_none() && matches!(m.screen, Screen::Settings { .. }));
    }

    #[test]
    fn pause_offers_splitscreen_to_hosts_only() {
        let mut c = ctx();
        let has = |c: &Ctx| items(Screen::Pause, c).contains(&Item::Button("SPLITSCREEN", Action::Splitscreen, true));
        assert!(has(&c));
        c.client = true;
        assert!(!has(&c));
        c.client = false;
        c.party = Some((vec![], String::new()));
        assert!(!items(Screen::Pause, &c).iter().any(|i| matches!(i, Item::Button("SPLITSCREEN", ..))), "GameNight's menu has none");
    }

    #[test]
    fn slider_drag_clamps_and_release_ends_drag() {
        let w = World::new(1, &[false, true]);
        let h = Hud::new(&w);
        let c = ctx();
        let mut m = Menus::default();
        m.open(Screen::Settings { from_start: false });
        draw(&m, &w, &h, &c);
        let r = m.hits.borrow().iter().find(|h| h.item == 1).unwrap().rect;
        m.pointer(at(r, true), SIZE, &w, None, &c);
        let drag = app::Pointer { position: Some([r[0] + r[2] + 100.0, r[1]]), moved: true, pressed: false, held: true, wheel: 0.0 };
        m.pointer(drag, SIZE, &w, None, &c);
        assert_eq!(m.music_db, 0.0);
        m.pointer(app::Pointer::default(), SIZE, &w, None, &c);
        assert!(m.drag.is_none());
        let before = m.music_db;
        m.pointer(app::Pointer { position: Some([r[0], r[1]]), ..drag }, SIZE, &w, None, &c);
        assert_eq!(m.music_db, before, "holding from elsewhere does not start a drag");
    }
}
