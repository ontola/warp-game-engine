//! The start and pause menu (src/hud/menu.gd): one screen. Start (Esc)
//! opens it and pauses; up and down pick a row, left and right change it,
//! A confirms, B or Start closes. The game's logo on top, Resume and Start
//! new run first, the controls on the left (only in this menu), the wizards
//! and their tomes on the right.
use crate::hud::{body, title};
use app::{Buttons, Controls};
use glam::Vec2;
use render::hud::{FontFace, HudDrawList, HudImage, ImageFilter, TextAlign, TextRun};
use render::picture::Picture;
use spellstorm::biome::biomes;
use spellstorm::el::{self, El};
use spellstorm::grammar::Form;
use spellstorm::World;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

static LOGO_PNG: &[u8] = include_bytes!("../assets/branding/logo.png");
const LOGO: HudImage = HudImage(0x5ee1_0003);
const NO_CLIP: [f32; 4] = [-1e5, -1e5, 2e5, 2e5];
const REPEAT: f32 = 0.22;
const PAD_DARK: [f32; 4] = [0.16, 0.14, 0.13, 1.0];
const PAD_EDGE: [f32; 4] = [0.55, 0.5, 0.42, 1.0];
const PAD_LIT: [f32; 4] = [1.0, 0.82, 0.4, 1.0];
const INK: [f32; 4] = [0.05, 0.04, 0.03, 1.0];
const TOME_TITLE: [f32; 4] = [1.0, 0.8, 0.42, 1.0];
const TOME_TEXT: [f32; 4] = [0.86 * 0.88, 0.84 * 0.88, 0.8 * 0.88, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Start,
    Resume,
    Restart,
    Lobby,
    Level,
    FriendlyFire,
    Difficulty,
    Unlocks,
    Quit,
}

/// What the menu asks of the game.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    StartAt(i32),
    /// Back to the Splitscreen Club room (the host's overlay).
    Lobby,
    Quit,
}

pub struct Menu {
    pub open: bool,
    title_mode: bool,
    rows: Vec<Row>,
    sel: usize,
    biome: i32,
    nav_t: f32,
    logo: Option<Arc<Picture>>,
    standalone: bool,
}

const CONTROLS: [(&str, &str); 14] = [
    ("lstick", "Walk"),
    ("rstick", "Aim and shoot bolts"),
    ("faces", "Load an element"),
    ("lb", "Hold: each button's second element"),
    ("rt", "Cast ahead: winds up, hold to aim"),
    ("rb", "Sword swing, knocks bullets back"),
    ("rb_hold", "Hold and let go: burst around you"),
    ("lt", "Dodge roll"),
    ("rs", "Click: enchant your bolts and sword"),
    ("rs_hold", "Hold: shake your staff empty"),
    ("ls", "Plant it at your feet"),
    ("dpad_up", "Your last spell again"),
    ("dpad_side", "Saved spells (hold to save)"),
    ("start", "This menu"),
];

fn ff_name(w: &World) -> &'static str {
    match w.settings.ff {
        f if f <= 0.0 => "off",
        f if f < 0.375 => "quarter",
        f if f < 0.75 => "half",
        _ => "full",
    }
}

fn difficulty_name(w: &World) -> &'static str {
    match w.settings.monster_hp {
        h if h < 0.9 => "easy",
        h if h > 1.1 => "hard",
        _ => "normal",
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn step<'a>(list: &[&'a str], now: &str, dx: i32) -> &'a str {
    let i = list.iter().position(|&x| x == now).unwrap_or(0) as i32;
    list[(i + dx).rem_euclid(list.len() as i32) as usize]
}

impl Menu {
    /// `standalone`: not in a Splitscreen Club session, so it offers Quit.
    pub fn new(standalone: bool) -> Menu {
        let logo = render::picture::decode(LOGO_PNG, 1.0).ok().map(Arc::new);
        Menu { open: false, title_mode: false, rows: vec![], sel: 0, biome: 0, nav_t: 0.0, logo, standalone }
    }

    /// `title`: the start menu (Start instead of Resume, no restart).
    pub fn show(&mut self, title: bool, w: &World) {
        self.title_mode = title;
        self.biome = if title { 0 } else { w.run.biome_index as i32 };
        // A new run sits right under Resume, as every Splitscreen Club game has it.
        self.rows = if title { vec![Row::Start] } else { vec![Row::Resume, Row::Restart] };
        if !self.standalone {
            self.rows.push(Row::Lobby);
        }
        self.rows.extend([Row::Level, Row::FriendlyFire, Row::Difficulty, Row::Unlocks]);
        if self.standalone {
            self.rows.push(Row::Quit);
        }
        self.sel = 0;
        self.open = true;
    }

    /// One tick of the humans' controls. Returns what the game should do.
    pub fn update(&mut self, w: &mut World, pads: &[Controls], dt: f32) -> Action {
        if !self.open {
            if pads.iter().any(|c| c.pressed.has(Buttons::START)) {
                self.show(false, w);
            }
            return Action::None;
        }
        self.nav_t -= dt;
        let (mut dx, mut dy) = (0i32, 0i32);
        let (mut ok, mut back) = (false, false);
        for c in pads {
            let p = c.pressed;
            if p.has(Buttons::UP) {
                dy = -1;
            }
            if p.has(Buttons::DOWN) {
                dy = 1;
            }
            if p.has(Buttons::LEFT) {
                dx = -1;
            }
            if p.has(Buttons::RIGHT) {
                dx = 1;
            }
            // The stick, with a repeat; the shell's y is up.
            if self.nav_t <= 0.0 && dx == 0 && dy == 0 {
                if c.mv[1] > 0.6 {
                    dy = -1;
                } else if c.mv[1] < -0.6 {
                    dy = 1;
                } else if c.mv[0] < -0.6 {
                    dx = -1;
                } else if c.mv[0] > 0.6 {
                    dx = 1;
                }
            }
            ok |= p.has(Buttons::SOUTH);
            back |= p.has(Buttons::EAST) || (p.has(Buttons::START) && !self.title_mode);
            if self.title_mode && p.has(Buttons::START) {
                ok = true;
            }
        }
        if dx != 0 || dy != 0 {
            self.nav_t = REPEAT;
        }
        if dy != 0 {
            self.sel = (self.sel as i32 + dy).rem_euclid(self.rows.len() as i32) as usize;
        }
        if dx != 0 {
            self.change(w, self.rows[self.sel], dx);
        }
        if ok {
            return self.confirm(w, self.rows[self.sel]);
        }
        if back && !self.title_mode {
            self.open = false;
            return Action::Close;
        }
        Action::None
    }

    /// Esc on the keyboard: open, or close (never the start menu).
    pub fn escape(&mut self, w: &World) {
        if !self.open {
            self.show(false, w);
        } else if !self.title_mode {
            self.open = false;
        }
    }

    fn change(&mut self, w: &mut World, row: Row, dx: i32) {
        match row {
            Row::Level => self.biome = (self.biome + 1 + dx).rem_euclid(biomes().len() as i32 + 1) - 1,
            Row::FriendlyFire => {
                let v = step(&["off", "quarter", "half", "full"], ff_name(w), dx);
                w.set_option("friendly_fire", v);
            }
            Row::Difficulty => {
                let v = step(&["easy", "normal", "hard"], difficulty_name(w), dx);
                w.set_option("difficulty", v);
            }
            Row::Unlocks => {
                let v = if w.settings.unlock_all { "earned" } else { "all" };
                w.set_option("unlocks", v);
            }
            _ => {}
        }
    }

    fn confirm(&mut self, w: &mut World, row: Row) -> Action {
        match row {
            Row::Resume => {
                self.open = false;
                Action::Close
            }
            Row::Start | Row::Level | Row::Restart => {
                self.open = false;
                Action::StartAt(self.biome)
            }
            Row::FriendlyFire | Row::Difficulty | Row::Unlocks => {
                self.change(w, row, 1);
                Action::None
            }
            Row::Lobby => {
                self.open = false;
                Action::Lobby
            }
            Row::Quit => Action::Quit,
        }
    }

    fn label(&self, w: &World, row: Row) -> (&'static str, String) {
        match row {
            Row::Start => ("Start", String::new()),
            Row::Resume => ("Resume", String::new()),
            Row::Restart => ("Start new run", String::new()),
            Row::Level => ("Level", if self.biome < 0 { "Random".into() } else { biomes()[self.biome as usize].name.into() }),
            Row::FriendlyFire => ("Friendly fire", cap(ff_name(w))),
            Row::Difficulty => ("Difficulty", cap(difficulty_name(w))),
            Row::Unlocks => ("Unlocks", if w.settings.unlock_all { "All".into() } else { "Earned".into() }),
            Row::Lobby => ("Back to lobby", String::new()),
            Row::Quit => ("Quit", String::new()),
        }
    }

    pub fn draw(&self, d: &mut HudDrawList, w: &World, size: [u32; 2]) {
        if !self.open {
            return;
        }
        // Laid out for 16:9: controls, menu and tomes side by side. A squarer
        // window (a browser tab, a portrait screen) stacks the controls and
        // tomes under the menu; either way it scales to fit and centres, or
        // the side panels squeeze and their words pile up (Joep).
        let stacked = (size[0] as f32) < size[1] as f32 * 1.45;
        let (dw, dh) = if stacked { (1240.0, 1260.0) } else { (1920.0, 1080.0) };
        let s = (size[1] as f32 / dh).min(size[0] as f32 / dw);
        let vw = size[0] as f32 / s;
        let vh = size[1] as f32 / s;
        let oy = (vh - dh) * 0.5;
        d.begin_overlay();
        let mut p = P { d, s };
        p.rect([0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, if self.title_mode { 0.8 } else { 0.66 }]);
        let top = oy + 1080.0 * 0.15;
        if let Some(logo) = &self.logo {
            p.d.provide_image(LOGO, logo, ImageFilter::Linear);
            let (bx, by, bw, bh) = (vw * 0.2, oy + 1080.0 * 0.025, vw * 0.6, 1080.0 * 0.15);
            let k = (bw / logo.width as f32).min(bh / logo.height as f32);
            let (fw, fh) = (logo.width as f32 * k, logo.height as f32 * k);
            let r = [(bx + (bw - fw) * 0.5) * s, (by + (bh - fh) * 0.5) * s, fw * s, fh * s];
            p.d.image(r, LOGO, [0.0, 0.0, 1.0, 1.0], [1.0; 4], NO_CLIP);
        }
        let sub = if self.title_mode { "A couch co-op roguelite for wizards" } else { "Paused" };
        p.text(sub, vw * 0.5, top + 46.0, body(), 32.0, [0.75, 0.68, 0.56, 1.0], TextAlign::Center);
        if !self.title_mode {
            let b = w.level.biome();
            let found = w.grimoire.len();
            let line = format!("{}   ·   Level {} of {}   ·   Grimoire: {} of {} spells discovered", b.name, w.run.biome_index + 1, biomes().len(), found, spellstorm::grammar::TOTAL);
            p.text(&line, vw * 0.5, top + 88.0, body(), 25.0, [0.9, 0.78, 0.45, 1.0], TextAlign::Center);
        }
        // Three panels: controls left, the menu in the middle, tomes right.
        let y = top + 170.0;
        let pw = 560.0;
        let cx = vw * 0.5;
        let side = (cx - pw * 0.5 - 60.0).min(620.0);
        // Stacked: the controls (wide enough for two columns) and the tomes
        // share a row under the menu.
        let below = y - 50.0 + self.rows.len() as f32 * 64.0 + 40.0 + 24.0;
        let (side, kx, ky, tomes_at) = if stacked {
            let (kw, tw) = (740.0, 420.0);
            let x0 = cx - (kw + 30.0 + tw) * 0.5;
            (kw, x0, below, [x0 + kw + 30.0, below, tw, vh - below - 10.0])
        } else {
            (side, cx - pw * 0.5 - 30.0 - side, y - 50.0, [cx + pw * 0.5 + 30.0, y - 50.0, side, vh - y - 10.0])
        };
        p.frame([cx - pw * 0.5, y - 50.0, pw, self.rows.len() as f32 * 64.0 + 40.0]);
        for (i, &row) in self.rows.iter().enumerate() {
            let (name, value) = self.label(w, row);
            let on = i == self.sel;
            let col = if on { [1.0, 0.82, 0.4, 1.0] } else { [0.72, 0.66, 0.56, 1.0] };
            // Godot draws at the baseline; these centre on the line.
            let yy = y + i as f32 * 64.0;
            if on {
                p.rect([cx - pw * 0.5 + 10.0, yy - 38.0, pw - 20.0, 54.0], [0.35, 0.08, 0.05, 0.55]);
            }
            if value.is_empty() {
                p.text(name, cx, yy - 11.0, title(), 36.0, col, TextAlign::Center);
            } else {
                p.text(name, cx - pw * 0.5 + 30.0, yy - 10.0, title(), 30.0, col, TextAlign::Left);
                let v = if on { format!("‹  {value}  ›") } else { value };
                p.text(&v, cx - pw * 0.05 + pw * 0.26, yy - 10.0, body(), 28.0, col, TextAlign::Center);
            }
        }
        if !w.wizards.is_empty() {
            self.tomes(&mut p, w, tomes_at);
        }
        // The controls, two columns, each by a drawing of its part of the pad.
        let row_h = 56.0;
        let controls = controls(w);
        let per = controls.len().div_ceil(2);
        let colw = side * 0.5;
        p.frame([kx, ky, side, per as f32 * row_h + 24.0]);
        for (i, (kind, what)) in controls.iter().enumerate() {
            let c = Vec2::new(kx + colw * (i / per) as f32 + 44.0, ky + 40.0 + (i % per) as f32 * row_h);
            glyph(&mut p, kind, c);
            p.wrapped(what, c.x + 46.0, c.y, colw - 100.0, 19.0, [0.85, 0.79, 0.68, 1.0]);
        }
    }

    /// The wizards and the tomes each has taken this run.
    fn tomes(&self, p: &mut P, w: &World, r: [f32; 4]) {
        let x = r[0] + 26.0;
        let tw = r[2] - 52.0;
        let bottom = r[1] + r[3] - 12.0;
        enum Line {
            Name(String, [f32; 4]),
            Plain(&'static str),
            Tome(String),
            Text(String),
            Gap,
        }
        let mut lines = vec![];
        for &id in &w.wizards {
            let wz = w.wiz(id);
            let c = wz.robe;
            lines.push(Line::Name(wz.name.clone(), [c[0] + (1.0 - c[0]) * 0.3, c[1] + (1.0 - c[1]) * 0.3, c[2] + (1.0 - c[2]) * 0.3, 1.0]));
            if wz.relics.is_empty() {
                lines.push(Line::Plain("No tomes yet"));
            }
            // Tomes stack: one line each, with how many.
            let mut seen: Vec<usize> = vec![];
            for &t in &wz.relics {
                if seen.contains(&t) {
                    continue;
                }
                seen.push(t);
                let n = wz.relics.iter().filter(|&&r| r == t).count();
                if let Some(rel) = spellstorm::relics::all().get(t) {
                    lines.push(Line::Tome(if n > 1 { format!("{} x{n}", rel.name) } else { rel.name.clone() }));
                    lines.push(Line::Text(rel.text.clone()));
                }
            }
            lines.push(Line::Gap);
        }
        let text_lines = |t: &str| (render::hud::measure(t, body(), 17.0 * p.s)[0] / ((tw - 18.0) * p.s).max(1.0)).ceil().clamp(1.0, 3.0);
        let mut used = 40.0;
        for l in &lines {
            used += match l {
                Line::Name(..) => 34.0,
                Line::Plain(_) => 30.0,
                Line::Tome(_) => 25.0,
                Line::Text(t) => text_lines(t) * 22.0 + 8.0,
                Line::Gap => 14.0,
            };
        }
        p.frame([r[0], r[1], r[2], r[3].min(used)]);
        let mut yy = r[1] + 40.0;
        for l in &lines {
            if yy > bottom {
                break;
            }
            match l {
                Line::Name(n, c) => {
                    p.text(n, x, yy - 9.0, title(), 26.0, *c, TextAlign::Left);
                    yy += 34.0;
                }
                Line::Plain(t) => {
                    p.text(t, x, yy - 7.0, body(), 19.0, [0.5, 0.46, 0.4, 1.0], TextAlign::Left);
                    yy += 30.0;
                }
                Line::Tome(t) => {
                    p.text(t, x + 18.0, yy - 7.0, title(), 19.0, TOME_TITLE, TextAlign::Left);
                    yy += 25.0;
                }
                Line::Text(t) => {
                    let n = p.wrapped(t, x + 18.0, yy - 4.0, tw - 18.0, 17.0, TOME_TEXT);
                    yy += n * 22.0 + 8.0;
                }
                Line::Gap => yy += 14.0,
            }
        }
    }
}

/// Only the controls someone at the couch has learned.
fn controls(w: &World) -> Vec<(&'static str, &'static str)> {
    let humans: Vec<_> = w.wizards.iter().filter(|&&id| w.wiz(id).bot.is_none()).copied().collect();
    let Some(&first) = humans.first() else { return CONTROLS.to_vec() };
    let wz = w.wiz(first);
    let lb = humans.iter().any(|&id| w.wiz(id).lb_open());
    CONTROLS
        .iter()
        .filter(|c| match c.0 {
            "lb" => lb,
            "rb_hold" => wz.form_open(Form::Nova),
            "ls" => wz.form_open(Form::Plant),
            "rs" => wz.form_open(Form::Enchant),
            _ => true,
        })
        .copied()
        .collect()
}

struct P<'a> {
    d: &'a mut HudDrawList,
    s: f32,
}

impl P<'_> {
    fn v(&self, q: Vec2) -> [f32; 2] {
        [q.x * self.s, q.y * self.s]
    }
    fn rect(&mut self, r: [f32; 4], c: [f32; 4]) {
        let s = self.s;
        self.d.rect([r[0] * s, r[1] * s, r[2] * s, r[3] * s], c, NO_CLIP);
    }
    /// Menu._frame: a dark panel with a thin bronze edge.
    fn frame(&mut self, r: [f32; 4]) {
        self.rect(r, [0.06, 0.04, 0.03, 0.92]);
        let e = [0.45, 0.35, 0.22, 0.9];
        self.rect([r[0], r[1], r[2], 2.0], e);
        self.rect([r[0], r[1] + r[3] - 2.0, r[2], 2.0], e);
        self.rect([r[0], r[1], 2.0, r[3]], e);
        self.rect([r[0] + r[2] - 2.0, r[1], 2.0, r[3]], e);
    }
    fn circle(&mut self, c: Vec2, r: f32, col: [f32; 4]) {
        let s = self.s;
        self.d.circle(self.v(c), r * s, col, NO_CLIP);
    }
    fn arc(&mut self, c: Vec2, r: f32, a: f32, b: f32, width: f32, col: [f32; 4]) {
        let s = self.s;
        self.d.arc(self.v(c), r * s, a, b, width * s, col, NO_CLIP);
    }
    fn line(&mut self, a: Vec2, b: Vec2, width: f32, col: [f32; 4]) {
        let s = self.s;
        self.d.line(self.v(a), self.v(b), width * s, col, NO_CLIP);
    }
    fn poly(&mut self, pts: &[Vec2], col: [f32; 4]) {
        let pts: Vec<[f32; 2]> = pts.iter().map(|&q| self.v(q)).collect();
        self.d.polygon(&pts, col, NO_CLIP);
    }
    /// One line centred on `y`.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, text: &str, x: f32, y: f32, font: FontFace, size: f32, c: [f32; 4], align: TextAlign) {
        let s = self.s;
        let px = size * s;
        let w = render::hud::measure(text, font, px)[0] + 4.0;
        let left = match align {
            TextAlign::Left => x * s,
            TextAlign::Center => x * s - w * 0.5,
            TextAlign::Right => x * s - w,
        };
        let id = 0x5ee2_0000_0000 + self.d.text.len() as u64;
        self.d.text.push(TextRun { id, text: text.into(), position: [left, y * s - px * 0.6], bounds: NO_CLIP, font_size: px, color: c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8), font, align, width: w });
    }
    /// Up to three lines in `width`, the first centred on `y`. Returns the
    /// lines used.
    fn wrapped(&mut self, text: &str, x: f32, y: f32, width: f32, size: f32, c: [f32; 4]) -> f32 {
        let mut lines: Vec<String> = vec![String::new()];
        for word in text.split(' ') {
            let cur = lines.last().unwrap();
            let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if !cur.is_empty() && render::hud::measure(&next, body(), size * self.s)[0] > width * self.s && lines.len() < 3 {
                lines.push(word.to_string());
            } else {
                *lines.last_mut().unwrap() = next;
            }
        }
        for (k, l) in lines.iter().enumerate() {
            self.text(l, x, y + k as f32 * size * 1.15, body(), size, c, TextAlign::Left);
        }
        lines.len() as f32
    }
}

/// What an unlock card shows on its left: an element, or a part of the pad
/// (a `glyph` kind).
pub enum CardIcon {
    Element(El),
    Pad(&'static str),
    /// The queue's sockets, the newest of `n` lit.
    Slots(usize),
}

/// Something just learned, in the menu's own look: its framed panel, the
/// element as on the controls' face buttons or the pad part drawn large,
/// a soft line saying what it is, the name in gold and how to use it.
/// Centred on `cx`, its top at `y`, all faded by `a`.
#[allow(clippy::too_many_arguments)]
pub fn unlock_card(d: &mut HudDrawList, s: f32, cx: f32, y: f32, icon: CardIcon, kicker: &str, name: &str, hint: &str, a: f32) {
    let (v0, t0) = (d.vertices.len(), d.text.len());
    let (w, h) = (680.0, 128.0);
    let x0 = cx - w * 0.5;
    let mut p = P { d: &mut *d, s };
    p.frame([x0, y, w, h]);
    let ic = Vec2::new(x0 + 76.0, y + h * 0.5);
    match icon {
        CardIcon::Element(e) => {
            let col = e.color();
            p.circle(ic, 36.0, PAD_EDGE);
            p.circle(ic, 34.0, [col[0], col[1], col[2], 1.0]);
            crate::hud::icon(&mut crate::hud::Painter { d: &mut *p.d, s }, e, ic, 22.0, [0.06, 0.04, 0.04, 1.0]);
        }
        CardIcon::Pad(kind) => {
            let k = 1.35;
            glyph(&mut P { d: &mut *p.d, s: s * k }, kind, ic / k);
        }
        CardIcon::Slots(n) => {
            let r = 15.0;
            let gap = 36.0;
            for i in 0..n {
                let c = ic + Vec2::new((i as f32 - (n as f32 - 1.0) * 0.5) * gap, 0.0);
                let new = i + 1 == n;
                if new {
                    p.circle(c, r + 7.0, [1.0, 0.82, 0.4, 0.25]);
                }
                p.circle(c, r + 2.0, PAD_EDGE);
                p.circle(c, r, if new { [1.0, 0.82, 0.4, 1.0] } else { PAD_DARK });
            }
        }
    }
    let tx = x0 + 168.0;
    p.text(kicker, tx, y + 30.0, body(), 24.0, [0.75, 0.68, 0.56, 1.0], TextAlign::Left);
    p.text(name, tx, y + 66.0, title(), 40.0, [1.0, 0.82, 0.4, 1.0], TextAlign::Left);
    p.text(hint, tx, y + 102.0, body(), 24.0, TOME_TEXT, TextAlign::Left);
    for v in &mut d.vertices[v0..] {
        v.color[3] *= a;
    }
    for t in &mut d.text[t0..] {
        t.color[3] = (t.color[3] as f32 * a).round() as u8;
    }
}

/// One controller part (glyph) drawn at scale `s` round `c`, for text
/// that names buttons (the HUD's hints): also "a", "b", "x", "y".
pub fn pad_icon(d: &mut HudDrawList, s: f32, kind: &str, c: Vec2) {
    glyph(&mut P { d, s }, kind, c);
}

/// One controller part, about 80 by 40 around `c` (Menu._glyph).
fn glyph(p: &mut P, kind: &str, c: Vec2) {
    let f = title();
    match kind {
        "lstick" | "rstick" | "rs" | "rs_hold" | "ls" => {
            // One plain disc with its letter (Joep: no layers of borders):
            // lit when it is clicked, with arrows round it when pushed.
            let click = kind == "rs" || kind == "rs_hold" || kind == "ls";
            p.circle(c, 17.0, if click { PAD_LIT } else { [0.9, 0.86, 0.78, 1.0] });
            let l = if kind == "lstick" || kind == "ls" { "L" } else { "R" };
            p.text(l, c.x, c.y + 1.0, f, 22.0, INK, TextAlign::Center);
            if !click {
                for a in 0..4 {
                    let d = Vec2::from_angle(a as f32 * PI * 0.5).rotate(Vec2::NEG_Y);
                    let o = Vec2::new(-d.y, d.x);
                    let tip = c + d * 27.0;
                    p.poly(&[tip, tip - d * 6.0 + o * 4.0, tip - d * 6.0 - o * 4.0], PAD_LIT);
                }
            }
        }
        "faces" => {
            for (pair, off) in [(el::PAIR_Y, Vec2::new(0.0, -12.0)), (el::PAIR_X, Vec2::new(-12.0, 0.0)), (el::PAIR_B, Vec2::new(12.0, 0.0)), (el::PAIR_A, Vec2::new(0.0, 12.0))] {
                let e: El = pair[0];
                let col = e.color();
                let q = c + off * 1.25;
                p.circle(q, 8.0, [col[0], col[1], col[2], 1.0]);
                crate::hud::icon(&mut crate::hud::Painter { d: &mut *p.d, s: p.s }, e, q, 5.0, [0.06, 0.04, 0.04, 1.0]);
            }
        }
        "a" | "b" | "x" | "y" => {
            // A face button in the pad's own colour, its letter on it.
            let col = match kind {
                "a" => [0.36, 0.72, 0.28, 1.0],
                "b" => [0.86, 0.26, 0.2, 1.0],
                "x" => [0.26, 0.5, 0.95, 1.0],
                _ => [0.95, 0.78, 0.2, 1.0],
            };
            p.circle(c, 20.0, PAD_EDGE);
            p.circle(c, 18.0, col);
            p.text(&kind.to_uppercase(), c.x, c.y + 1.0, f, 24.0, [1.0, 1.0, 1.0, 1.0], TextAlign::Center);
        }
        "lb" | "rb" | "rb_hold" => {
            p.rect([c.x - 28.0, c.y - 10.0, 56.0, 20.0], if kind != "rb_hold" { PAD_LIT } else { PAD_EDGE });
            p.text(if kind == "lb" { "LB" } else { "RB" }, c.x, c.y, f, 17.0, INK, TextAlign::Center);
            if kind == "rb_hold" {
                let t = c + Vec2::new(40.0, 0.0);
                p.arc(t, 8.0, 0.0, TAU, 2.0, PAD_EDGE);
                p.arc(t, 8.0, -PI * 0.5, PI, 3.0, PAD_LIT);
            }
        }
        "lt" | "rt" => {
            let v = |x: f32, y: f32| c + Vec2::new(x, y);
            p.poly(&[v(-18.0, 18.0), v(-18.0, -6.0), v(-10.0, -18.0), v(18.0, -18.0), v(18.0, 18.0)], PAD_LIT);
            p.text(&kind.to_uppercase(), c.x, c.y + 2.0, f, 16.0, INK, TextAlign::Center);
        }
        "dpad_up" | "dpad_side" => {
            for d in [Vec2::NEG_Y, Vec2::Y, Vec2::NEG_X, Vec2::X] {
                let lit = (kind == "dpad_up" && d == Vec2::NEG_Y) || (kind == "dpad_side" && d.y == 0.0);
                let q = c + d * 12.0 - Vec2::splat(7.0);
                p.rect([q.x, q.y, 14.0, 14.0], if lit { PAD_LIT } else { PAD_EDGE });
            }
            p.rect([c.x - 7.0, c.y - 7.0, 14.0, 14.0], PAD_EDGE);
        }
        "start" => {
            p.rect([c.x - 18.0, c.y - 8.0, 36.0, 16.0], PAD_LIT);
            for k in 0..3 {
                let y = c.y - 4.0 + k as f32 * 4.0;
                p.line(Vec2::new(c.x - 9.0, y), Vec2::new(c.x + 9.0, y), 2.0, INK);
            }
        }
        _ => {}
    }
}
