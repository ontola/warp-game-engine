//! Menus after Godot's `start_screen.gd` (with `gothic_menu_ui.gd`'s stone
//! and gold frame), `game.gd`'s pause menu, settings panel and match-over
//! buttons, and its Tab scoreboard. Focus moves with the D-pad, arrows,
//! WASD or stick; A/Space/Enter takes, B/Esc backs out; left and right move
//! sliders. Online joining by pasted ID has no text entry here: join with
//! `play --join TICKET` (the menu says so). Under GameNight the pause
//! menu is Godot's `_build_gamenight_pause_menu`: a slab down the left with
//! the logo, what is being played, and RESUME, NEW MATCH, the other mode,
//! AI PLAYERS, ADVANCED (the party's match settings) and SETTINGS.
use super::style::*;
use super::{Hud, DESIGN_H};
use app::{Buttons, Controls};
use growing_guns::{Phase, World};
use render::hud::HudDrawList;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen { Play, Start, Pause, Settings { from_start: bool }, Advanced }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { None, Resume, Versus, Wave, Settings, Back, Host, Copy, Quit, Rematch, Extend, AddBot, RemoveBot, Advanced,
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
enum Field { Ticket, Join }
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slider { Music, Mouse, Stick, /** One per GameNight seat ("Aim: NAME"). */ Aim(usize) }

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
}

impl Default for Menus {
    fn default() -> Self {
        Menus { screen: Screen::Play, menu: Default::default(), rematch: Default::default(), mouse: 1.0, stick: 1.0, music_db: -16.0, tilt: true, aim: Vec::new(), age: 0.0, scoreboard: false, held_x: 0, repeat: 0.0, rematch_armed: false, col: 0, notice: None, error: None }
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
            Item::Note("1 / 2 selects · Esc exits"),
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
            Item::Button("SETTINGS", Action::Settings, true), Item::Button("RESUME", Action::Resume, true), Item::Button("EXIT GAME", Action::Quit, true),
        ],
        Screen::Settings { .. } => settings_items(ctx),
        Screen::Play => vec![],
    }
}

/// Godot `build_settings_panel`: music, mouse, one stick slider (or one
/// "Aim" slider per GameNight seat), movement tilt, BACK. (Player name and
/// the retro shader have no counterpart here.)
fn settings_items(ctx: &Ctx) -> Vec<Item> {
    let mut v = vec![Item::Slider(Slider::Music), Item::Slider(Slider::Mouse)];
    if ctx.seats.is_empty() { v.push(Item::Slider(Slider::Stick)); } else { v.extend((0..ctx.seats.len()).map(|k| Item::Slider(Slider::Aim(k)))); }
    v.extend([Item::Tilt, Item::Gap(8.0), Item::Button("BACK", Action::Back, true)]);
    v
}

/// Godot `music_label_for`: Off at the bottom, else 0..100 %.
fn music_label(db: f32) -> String { if db <= MUSIC_DB_MIN + 0.5 { "Off".into() } else { format!("{}%", ((db - MUSIC_DB_MIN) / -MUSIC_DB_MIN * 100.0).round()) } }
pub const MUSIC_DB_MIN: f32 = -45.0;

/// The bot stepper's - (0) and + (1): host or offline, not in coop (the
/// waves bring their own), 0..=6 bots and 8 players at most.
fn bots_enabled(j: usize, ctx: &Ctx) -> bool { !ctx.client && !ctx.coop && if j == 0 { ctx.bots > 0 && ctx.players > 1 } else { ctx.players < 8 } }

fn row_enabled(t: &str, ctx: &Ctx) -> bool { !(ctx.client || (t == "COPY" && ctx.ticket.is_none()) || (t == "HOST ONLINE" && ctx.ticket.is_some())) }

/// Focusable lines: item index and the enabled entries in it (a row has several).
fn lines(items: &[Item], ctx: &Ctx) -> Vec<(usize, Vec<usize>)> {
    let mut out = Vec::new();
    for (k, it) in items.iter().enumerate() {
        match it {
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
    pub fn open(&mut self, s: Screen) { self.screen = s; self.menu = Default::default(); self.col = 0; self.age = 0.0; self.notice = None; }
    pub fn is_open(&self) -> bool { self.screen != Screen::Play }

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
        if self.screen == Screen::Pause && c.pressed.has(Buttons::START) { return Action::Resume; }
        let list = items(self.screen, ctx);
        let foci = lines(&list, ctx);
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
        match ev {
            app::ui::MenuEvent::Select(n) => {
                let (k, on) = &foci[n];
                let j = on[self.col.min(on.len() - 1)];
                match list[*k] { Item::Button(_, a, _) => a, Item::Row(r) => r[j].1, Item::Bots(_) => if j == 0 { Action::RemoveBot } else { Action::AddBot }, _ => Action::None }
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
    fn set_aim(&mut self, name: &str, v: f32) {
        match self.aim.iter_mut().find(|a| a.0 == name) { Some(a) => a.1 = v, None => self.aim.push((name.to_string(), v)) }
    }

    /// The settings as JSON (Godot keeps them in `user://settings.cfg`).
    pub fn settings_json(&self) -> serde_json::Value {
        serde_json::json!({ "music_db": self.music_db, "mouse": self.mouse, "stick": self.stick, "tilt": self.tilt,
            "aim": self.aim.iter().map(|(n, v)| (n.clone(), serde_json::json!(v))).collect::<serde_json::Map<_, _>>() })
    }
    pub fn load_settings(&mut self, v: &serde_json::Value) {
        let f = |k: &str, lo: f32, hi: f32, d: f32| v.get(k).and_then(|x| x.as_f64()).map_or(d, |x| (x as f32).clamp(lo, hi));
        self.music_db = f("music_db", MUSIC_DB_MIN, 0.0, self.music_db);
        self.mouse = f("mouse", 0.3, 3.0, self.mouse);
        self.stick = f("stick", 0.3, 2.5, self.stick);
        self.tilt = v.get("tilt").and_then(|x| x.as_bool()).unwrap_or(self.tilt);
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
    }

    fn focused(&self, list: &[Item], ctx: &Ctx) -> Option<(usize, usize)> {
        let f = lines(list, ctx);
        f.get(self.menu.focus.min(f.len().saturating_sub(1))).map(|(k, on)| (*k, on[self.col.min(on.len() - 1)]))
    }

    fn start(&self, d: &mut HudDrawList, ctx: &Ctx, u: f32, clip: [f32; 4]) {
        // Godot's Dim (black 0.32) and the logo, 440 x 190 from y 20.
        d.rect(clip, [0.0, 0.0, 0.0, 0.32], clip);
        logo(d, [clip[2] * 0.5 - 220.0 * u, 20.0 * u, 440.0 * u, 190.0 * u], clip);
        let list = items(Screen::Start, ctx);
        let (pw, ph) = (350.0 * u, 392.0 * u);
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
            Label::new(BODY, 26.0 * u, [0.98, 0.82, 0.38, 1.0]).outline(5.0 * u, [0.32, 0.05, 0.06, 1.0]).center().draw(d, "SETTINGS", panel[0] + pw * 0.5, panel[1] + 70.0 * u, clip);
            self.column(d, &list, [panel[0] + 30.0 * u, panel[1] + 110.0 * u, pw - 60.0 * u, inner], ctx, true, u, clip);
        } else {
            let ph = inner + 40.0 * u;
            let panel = [clip[2] * 0.5 - pw * 0.5 - 20.0 * u, clip[3] * 0.5 - ph * 0.5, pw + 40.0 * u, ph];
            menu_panel(d, panel, u, clip);
            Label::new(BODY, 28.0 * u, [1.0, 0.9, 0.45, 1.0]).outline(4.0 * u, [0.4, 0.0, 0.1, 1.0]).center().draw(d, "SETTINGS", panel[0] + panel[2] * 0.5, panel[1] + 38.0 * u, clip);
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
                        "Paste an iroh game ID" => ("No paste here: join with play --join TICKET".into(), [0.55, 0.6, 0.72, 1.0]),
                        _ => (t.to_string(), [0.57, 0.52, 0.44, 1.0]),
                    };
                    let size = if t == "BOTS" { 24.0 } else if gothic { 11.0 } else { 12.0 } * u;
                    let lab = if t == "BOTS" { Label::poster(size).center() } else { Label::new(BODY, size, c).center() };
                    let lab = if t == "BOTS" { lab } else { Label { color: c, ..lab } };
                    lab.draw(d, &text, r[0] + r[2] * 0.5, y + size * 0.65, clip);
                    y += size * 1.3;
                }
                Item::Field(f) => {
                    let fh = 34.0 * u;
                    let (text, placeholder) = match f {
                        Field::Ticket => (ctx.ticket.map(|t| t.to_string()), if gothic { "Offline — click Host online" } else { "(offline — not hosting)" }),
                        Field::Join => (None, if gothic { "Paste friend's Game ID..." } else { "Paste a Game ID…" }),
                    };
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
                        if gn { gn_button(d, [bx, y, bw, bh], t, fk(j), on, u, clip) } else { arena_button(d, [bx, y, bw, bh], t, fk(j), on, 1.0, u, clip) }
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

fn column_height(list: &[Item], u: f32) -> f32 {
    let sep = 8.0;
    list.iter().map(|it| match it {
        Item::Bots(_) => 34.0 + sep, Item::Party(_) => 36.0 + sep,
        Item::Text(..) => 20.0, Item::Note(t) => if *t == "BOTS" { 24.0 * 1.3 } else { 12.0 * 1.3 },
        Item::Field(_) => 34.0 + sep, Item::Button(..) | Item::Row(_) => 32.0 + sep, Item::Slider(_) => 50.0, Item::Tilt => 44.0, Item::Gap(g) => *g, Item::Rule => 8.0 + sep,
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
    Label::new(BODY, 28.0 * u, [1.0, 0.9, 0.5, 1.0]).center().draw(d, &head, w * 0.5, y + 17.0 * u, clip);
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
