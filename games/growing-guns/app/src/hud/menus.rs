//! Menus after Godot's `start_screen.gd` (with `gothic_menu_ui.gd`'s stone
//! and gold frame), `game.gd`'s pause menu, settings panel and match-over
//! buttons, and its Tab scoreboard. Focus moves with the D-pad, arrows,
//! WASD or stick; A/Space/Enter takes, B/Esc backs out; left and right move
//! sliders. Online joining by pasted ID has no text entry here: join with
//! `play --join TICKET` (the menu says so).
use super::style::*;
use super::{Hud, DESIGN_H};
use app::{Buttons, Controls};
use growing_guns::{Phase, World};
use render::hud::HudDrawList;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen { Play, Start, Pause, Settings { from_start: bool } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { None, Resume, Versus, Wave, Settings, Back, Host, Copy, Quit, Rematch, Extend }

#[derive(Clone, Copy, Debug, PartialEq)]
enum Item { Text(&'static str, C), Note(&'static str), Field(Field), Button(&'static str, Action, bool), Row(&'static [(&'static str, Action)]), Slider(Slider), Gap(f32), Rule }
#[derive(Clone, Copy, Debug, PartialEq)]
enum Field { Ticket, Join }
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slider { Mouse, Stick }

/// What the menus need to know about the session.
pub struct Ctx<'a> { pub ticket: Option<&'a str>, pub client: bool, pub seed: u64, pub bots: usize }

#[derive(Clone, Debug)]
pub struct Menus {
    pub screen: Screen,
    pub menu: app::ui::Menu,
    pub rematch: app::ui::Menu,
    /// Look sensitivity multipliers (Godot settings: mouse 0.3..3, stick 0.3..2.5).
    pub mouse: f32,
    pub stick: f32,
    /// Seconds the screen has been open (fades and the start orbit).
    pub age: f32,
    /// The Tab / Back scoreboard is held.
    pub scoreboard: bool,
    held_x: i32,
    repeat: f32,
    /// Entry within a focused row.
    col: usize,
    /// A short line under the menu ("ticket written to ...").
    pub notice: Option<String>,
}

impl Default for Menus {
    fn default() -> Self {
        Menus { screen: Screen::Play, menu: Default::default(), rematch: Default::default(), mouse: 1.0, stick: 1.0, age: 0.0, scoreboard: false, held_x: 0, repeat: 0.0, col: 0, notice: None }
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
        Screen::Pause => vec![
            Item::Text("Share your match with friends:", label), Item::Field(Field::Ticket), Item::Row(SHARE),
            Item::Text("Or join a friend:", label), Item::Field(Field::Join), Item::Note("Paste an iroh game ID"),
            Item::Text("Bots:", label), Item::Note("BOTS"), Item::Gap(4.0),
            Item::Button("VERSUS", Action::Versus, !ctx.client), Item::Button("WAVE SURVIVAL", Action::Wave, !ctx.client),
            Item::Button("SETTINGS", Action::Settings, true), Item::Button("RESUME", Action::Resume, true), Item::Button("EXIT GAME", Action::Quit, true),
        ],
        Screen::Settings { .. } => vec![
            Item::Slider(Slider::Mouse), Item::Slider(Slider::Stick), Item::Gap(8.0), Item::Button("BACK", Action::Back, true),
        ],
        Screen::Play => vec![],
    }
}

fn row_enabled(t: &str, ctx: &Ctx) -> bool { !(ctx.client || (t == "COPY" && ctx.ticket.is_none()) || (t == "HOST ONLINE" && ctx.ticket.is_some())) }

/// Focusable lines: item index and the enabled entries in it (a row has several).
fn lines(items: &[Item], ctx: &Ctx) -> Vec<(usize, Vec<usize>)> {
    let mut out = Vec::new();
    for (k, it) in items.iter().enumerate() {
        match it {
            Item::Button(_, _, true) | Item::Slider(_) => out.push((k, vec![0])),
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
                if let app::ui::MenuEvent::Select(k) = self.rematch.update(c, b.len(), 1, dt) { return b[k].1; }
            } else { self.rematch = Default::default(); }
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
            let v = match s { Slider::Mouse => &mut self.mouse, Slider::Stick => &mut self.stick };
            let (lo, hi) = match s { Slider::Mouse => (0.3, 3.0), Slider::Stick => (0.3, 2.5) };
            *v = (*v + 0.05 * step as f32).clamp(lo, hi);
            *v = (*v * 20.0).round() / 20.0;
            return Action::None;
        }
        let ev = self.menu.update(&Controls { mv: [0.0, c.mv[1]], held: Buttons(c.held.0 & !(Buttons::LEFT | Buttons::RIGHT)), pressed: Buttons(c.pressed.0 & !(Buttons::LEFT | Buttons::RIGHT)), ..*c }, foci.len(), 1, dt);
        // Rows move left and right within themselves.
        if let Some((_, on)) = &focus && step != 0 { self.col = (self.col as i32 + step).clamp(0, on.len() as i32 - 1) as usize; }
        if ev == app::ui::MenuEvent::Moved { self.col = 0; }
        match ev {
            app::ui::MenuEvent::Select(n) => {
                let (k, on) = &foci[n];
                let j = on[self.col.min(on.len() - 1)];
                match list[*k] { Item::Button(_, a, _) => a, Item::Row(r) => r[j].1, _ => Action::None }
            }
            app::ui::MenuEvent::Back => self.back(),
            _ => Action::None,
        }
    }

    /// Esc or B: close one level.
    pub fn back(&mut self) -> Action {
        match self.screen {
            Screen::Settings { from_start } => { self.open(if from_start { Screen::Start } else { Screen::Pause }); Action::None }
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
            Screen::Pause => self.pause(d, world, ctx, u, clip),
            Screen::Settings { from_start } => self.settings(d, ctx, from_start, u, clip),
        }
    }

    fn focused(&self, list: &[Item], ctx: &Ctx) -> Option<(usize, usize)> {
        let f = lines(list, ctx);
        f.get(self.menu.focus.min(f.len().saturating_sub(1))).map(|(k, on)| (*k, on[self.col.min(on.len() - 1)]))
    }

    fn start(&self, d: &mut HudDrawList, ctx: &Ctx, u: f32, clip: [f32; 4]) {
        d.rect(clip, [0.02, 0.01, 0.015, 0.22], clip);
        logo(d, [clip[2] * 0.5, 115.0 * u], 190.0 * u, clip);
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
        logo(d, [clip[2] * 0.5, 105.0 * u], 180.0 * u, clip);
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
        let sep = 8.0 * u;
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
                    let bh = 32.0 * u;
                    let br = [r[0], y, r[2], bh];
                    if gothic { gothic_button(d, br, t, fk(0), enabled, u, clip) } else { arena_button(d, br, t, fk(0), enabled, 1.0, u, clip) }
                    y += bh + sep;
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
                    let (name, v, lo, hi) = match s { Slider::Mouse => ("Mouse sensitivity", self.mouse, 0.3, 3.0), Slider::Stick => ("Stick sensitivity", self.stick, 0.3, 2.5) };
                    let rh = 36.0 * u;
                    let mid = y + rh * 0.5;
                    let lc = if fk(0) { BONE } else { [0.85, 0.85, 0.95, 1.0] };
                    Label::new(BODY, 14.0 * u, lc).draw(d, name, r[0], mid, clip);
                    let (sx, sw) = (r[0] + 142.0 * u, r[2] - 142.0 * u - 59.0 * u);
                    d.rect([sx, mid - 2.0 * u, sw, 4.0 * u], [0.25, 0.22, 0.24, 1.0], clip);
                    let k = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                    d.rect([sx, mid - 2.0 * u, sw * k, 4.0 * u], if fk(0) { RED } else { [0.6, 0.55, 0.55, 1.0] }, clip);
                    d.circle([sx + sw * k, mid], 7.0 * u, if fk(0) { BONE } else { [0.8, 0.78, 0.75, 1.0] }, clip);
                    Label::new(BODY, 14.0 * u, [0.74, 0.78, 0.9, 1.0]).right().draw(d, &format!("{v:.2}x"), r[0] + r[2], mid, clip);
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
        Item::Text(..) => 20.0, Item::Note(t) => if *t == "BOTS" { 24.0 * 1.3 } else { 12.0 * 1.3 },
        Item::Field(_) => 34.0 + sep, Item::Button(..) | Item::Row(_) => 32.0 + sep, Item::Slider(_) => 50.0, Item::Gap(g) => *g, Item::Rule => 8.0 + sep,
    }).sum::<f32>() * u
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

/// The GROWING GUNS logo in poster type (the original is a texture: bone
/// letters with a red-orange block shadow).
pub fn logo(d: &mut HudDrawList, center: [f32; 2], height: f32, clip: [f32; 4]) {
    let size = height * 0.5;
    for (k, word) in ["GROWING", "GUNS"].iter().enumerate() {
        let y = center[1] - height * 0.24 + k as f32 * height * 0.48;
        let lab = Label::new(display(), size, [0.98, 0.93, 0.84, 1.0]).center();
        // Extruded shadow: a stack of offset copies, dark red under orange.
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
