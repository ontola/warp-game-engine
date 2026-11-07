//! The F3 debug overlay: a panel down the right over the running match
//! (it is no pause) with four tabs. CARDS gives any card to the target
//! player (type to filter), ITEMS sets the special, drops pickups and refills
//! health and ammo, SKY picks the time of day and weather, MODES forces a
//! round modifier (Blackout, Fog, ...) and starts it at once, and runs the
//! arena and match tools (lava leak, island arena, bots, next round, cage).
//! Every entry runs the game's own debug commands (`play.rs`, `help`), so the
//! overlay adds no game logic of its own.
//!
//! Keys: F3 (or Back + Start on a pad) opens and closes it, Esc or B closes
//! it. Up and down move, left and right move within a row and pick the
//! target player on the TARGET row, Q/E or LB/RB change the tab, Space,
//! Enter or A take. On the CARDS search field the keyboard types (Enter gives
//! the first match); a pad opens the on-screen keyboard with A. The mouse
//! hovers and clicks, the wheel scrolls.
use super::style::*;
use super::DESIGN_H;
use app::{Buttons, Controls, FieldEvent, TextEvent, TextField};
use render::hud::HudDrawList;
use std::cell::{Cell, RefCell};

pub const TABS: [&str; 4] = ["CARDS", "ITEMS", "SKY", "MODES"];
const TIMES: [&str; 6] = ["sunrise", "morning", "noon", "afternoon", "sunset", "night"];
const WEATHERS: [&str; 3] = ["clear", "haze", "mist"];

/// What the overlay shows of the match; play.rs fills it.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// Enabled cards: id, name, colour, rare.
    pub cards: Vec<(String, String, C, bool)>,
    /// Everyone: name, position, bot.
    pub players: Vec<(String, [f32; 3], bool)>,
    /// The target player's cards (ids), special, max health and magazine.
    pub held: Vec<String>,
    pub special: &'static str,
    pub max_health: i32,
    pub mag: u32,
    /// `sky` choice name (`godot`, `noon:clear`, `rounds:clear`, ..).
    pub sky: String,
    /// This round's modifier and the forced one, by id.
    pub modifier: Option<&'static str>,
    pub forced: Option<&'static str>,
    pub bots_paused: bool,
    pub round: u32,
    pub seed: u64,
    pub lava: bool,
    pub island: bool,
}

/// What an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act { Tab(usize), Player(usize), Search, Card(usize, String), Run(Vec<String>) }

#[derive(Clone, Debug)]
struct Entry { text: String, sub: String, act: Act, on: bool, color: Option<C> }

impl Entry {
    fn run(text: impl Into<String>, cmds: &[String], on: bool) -> Self { Entry { text: text.into(), sub: String::new(), act: Act::Run(cmds.to_vec()), on, color: None } }
}

#[derive(Clone, Debug)]
enum Row { Head(String), Search, Line(String, Vec<Entry>) }

#[derive(Clone, Copy, Debug)]
struct Hit { rect: [f32; 4], row: usize, entry: usize }

#[derive(Clone, Debug)]
pub struct DebugMenu {
    window: render::editor_window::EditorWindow,
    pub open: bool,
    pub tab: usize,
    /// The player the cards, specials and pickups go to.
    pub player: usize,
    pub search: TextField,
    /// A pad typing into the search field.
    pub picker: Option<app::ui::CharPicker>,
    menu: app::ui::Menu,
    col: usize,
    held_x: i32,
    repeat: f32,
    /// Content scroll in pixels; `follow` keeps the focus in view after a key move.
    scroll: Cell<f32>,
    follow: Cell<bool>,
    hits: RefCell<Vec<Hit>>,
    hit_size: Cell<[u32; 2]>,
    /// The last command's reply (ok?) and its age.
    pub notice: Option<(String, bool)>,
    notice_age: f32,
}

impl Default for DebugMenu {
    fn default() -> Self {
        DebugMenu { window:Default::default(),open: false, tab: 0, player: 0, search: TextField::new(32), picker: None, menu: Default::default(), col: 0, held_x: 0, repeat: 0.0,
            scroll: Cell::new(0.0), follow: Cell::new(false), hits: Default::default(), hit_size: Cell::new([0; 2]), notice: None, notice_age: 0.0 }
    }
}

/// `cmds` with `{p}` for the target player.
fn cmds(list: &[&str], p: usize) -> Vec<String> { list.iter().map(|c| c.replace("{p}", &p.to_string())).collect() }

fn chunked(entries: Vec<Entry>, n: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut it = entries.into_iter().peekable();
    while it.peek().is_some() { rows.push(Row::Line(String::new(), it.by_ref().take(n).collect())); }
    rows
}

impl DebugMenu {
    pub fn set_open(&mut self, on: bool) {
        self.open = on;
        self.window.cancel();
        self.picker = None;
        self.follow.set(true);
    }
    pub fn toggle(&mut self) { let on = !self.open; self.set_open(on); }

    pub fn set_tab(&mut self, t: usize) {
        self.tab = t % TABS.len();
        self.scroll.set(0.0);
        self.picker = None;
        // Stay on the tab bar when it had the focus, else go to the first content line.
        if self.menu.focus != 0 { self.menu.focus = 2; self.col = 0; }
        else { self.col = self.tab; }
        self.follow.set(true);
    }

    /// The search filter matches this card (id or name, any case).
    fn matches(&self, id: &str, name: &str) -> bool {
        let q = self.search.text().trim().to_lowercase();
        q.is_empty() || id.to_lowercase().contains(&q) || name.to_lowercase().contains(&q)
    }

    fn rows(&self, ctx: &Ctx) -> Vec<Row> {
        let p = self.player.min(ctx.players.len().saturating_sub(1));
        let tabs = TABS.iter().enumerate().map(|(k, t)| Entry { text: t.to_string(), sub: String::new(), act: Act::Tab(k), on: k == self.tab, color: None }).collect();
        let players = ctx.players.iter().enumerate().map(|(k, (_, _, bot))| Entry { text: format!("P{}", k + 1), sub: if *bot { "bot".into() } else { String::new() }, act: Act::Player(k), on: k == p, color: None }).collect();
        let mut v = vec![Row::Line(String::new(), tabs), Row::Line("TARGET".into(), players)];
        let pos = ctx.players.get(p).map_or([0.0; 3], |q| q.1);
        match self.tab {
            0 => {
                v.push(Row::Search);
                v.push(Row::Head("LMB: add card   RMB: remove one".into()));
                let held = if ctx.held.is_empty() { "no cards".to_string() } else { ctx.held.join(", ") };
                v.push(Row::Head(format!("HAS: {held}")));
                let cards: Vec<Entry> = ctx.cards.iter().filter(|c| self.matches(&c.0, &c.1)).map(|(id, name, color, rare)| Entry {
                    text: name.clone(), sub: if *rare { format!("{id} · rare") } else { id.clone() }, act: Act::Card(p, id.clone()), on: ctx.held.contains(id), color: Some(*color),
                }).collect();
                if cards.is_empty() { v.push(Row::Head("no card matches".into())); }
                v.extend(chunked(cards, 2));
            }
            1 => {
                v.push(Row::Head("SPECIAL (RMB / LT)".into()));
                let specials = growing_guns::weapon::SPECIALS.iter().map(|(_, n)| Entry::run(n.replace('_', " ").to_uppercase(), &[format!("special {p} {n}")], ctx.special == *n)).collect();
                v.extend(chunked(specials, 3));
                v.push(Row::Head("DROP A PICKUP ON THE TARGET".into()));
                let pickups = growing_guns::PICKUPS.iter().map(|k| {
                    let name = pickup_name(*k);
                    Entry::run(name.replace('_', " ").to_uppercase(), &[format!("pickup {name} {:.2} {:.2} {:.2}", pos[0], pos[2], pos[1] + 9.0)], false)
                }).collect();
                v.extend(chunked(pickups, 3));
                v.push(Row::Head("PLAYER".into()));
                v.push(Row::Line(String::new(), vec![
                    Entry::run("FULL HEALTH", &[format!("health {p} {}", ctx.max_health)], false),
                    Entry::run("REFILL AMMO", &[format!("ammo {p} {}", ctx.mag)], false),
                ]));
            }
            2 => {
                v.push(Row::Head(format!("SKY: {}", ctx.sky)));
                let head = ctx.sky.split(':').next().unwrap_or("");
                v.push(Row::Line(String::new(), vec![
                    Entry::run("RANDOM", &cmds(&["sky random"], p), head == "random"),
                    Entry::run("PER ROUND", &cmds(&["sky rounds"], p), head == "rounds"),
                    Entry::run("DAY CYCLE", &cmds(&["sky cycle"], p), head == "cycle"),
                ]));
                for t in TIMES {
                    let entries = WEATHERS.iter().map(|w| Entry::run(w.to_uppercase(), &[format!("sky {t}:{w}")], ctx.sky == format!("{t}:{w}"))).collect();
                    v.push(Row::Line(t.to_uppercase(), entries));
                }
            }
            _ => {
                let now = ctx.modifier.map_or("none".to_string(), |m| growing_guns::Modifier::named(m).map_or(m.to_string(), |m| m.title().to_string()));
                v.push(Row::Head(format!("ROUND {}  ·  MODIFIER: {now}{}", ctx.round, ctx.forced.map_or(String::new(), |f| format!("  ·  forced {f}")))));
                let mut mods: Vec<Entry> = growing_guns::MODIFIERS.iter().map(|(_, id, title, _)| Entry::run(*title, &[format!("modifier {id}"), "round".into()], ctx.modifier == Some(*id))).collect();
                mods.push(Entry::run("NONE", &cmds(&["modifier none", "round"], p), ctx.modifier.is_none()));
                v.extend(chunked(mods, 2));
                v.push(Row::Head("ARENA".into()));
                v.push(Row::Line(String::new(), vec![
                    Entry::run("LAVA LEAK", &cmds(&["lava"], p), ctx.lava),
                    Entry::run("ISLAND ARENA", &[format!("arena island {}", ctx.seed.wrapping_add(1) % 100_000)], ctx.island),
                    Entry::run("NEW ARENA", &[format!("arena {}", ctx.seed.wrapping_add(1) % 100_000)], false),
                ]));
                v.push(Row::Head("MATCH".into()));
                v.push(Row::Line(String::new(), vec![
                    Entry::run("NEXT ROUND", &cmds(&["round"], p), false),
                    Entry::run("OPEN CAGE", &cmds(&["cage"], p), false),
                    Entry::run("REMATCH", &cmds(&["rematch"], p), false),
                ]));
                v.push(Row::Line(String::new(), vec![
                    if ctx.bots_paused { Entry::run("BOTS PAUSED", &cmds(&["bots resume"], p), true) } else { Entry::run("PAUSE BOTS", &cmds(&["bots pause"], p), false) },
                    Entry::run("+5 ROUNDS", &cmds(&["extend"], p), false),
                ]));
            }
        }
        v
    }

    /// Focusable rows: row index and entry count (the search field has one).
    fn foci(rows: &[Row]) -> Vec<(usize, usize)> {
        rows.iter().enumerate().filter_map(|(k, r)| match r { Row::Search => Some((k, 1)), Row::Line(_, e) if !e.is_empty() => Some((k, e.len())), _ => None }).collect()
    }

    fn focused(&self, rows: &[Row]) -> Option<(usize, usize)> {
        let f = Self::foci(rows);
        f.get(self.menu.focus.min(f.len().saturating_sub(1))).map(|&(k, n)| (k, self.col.min(n - 1)))
    }

    /// The search field has the focus: the keyboard types into it.
    pub fn text_focus(&self, ctx: &Ctx) -> bool {
        let rows = self.rows(ctx);
        self.open && self.focused(&rows).is_some_and(|(k, _)| matches!(rows[k], Row::Search))
    }

    /// Put the focus on a line of the open tab (`debug focus`): `search`, or
    /// the n-th focusable line (0 the tabs, 1 the target).
    pub fn focus(&mut self, n: usize, ctx: &Ctx) -> bool {
        let rows = self.rows(ctx);
        let f = Self::foci(&rows);
        if n >= f.len() { return false; }
        self.menu.focus = n;
        self.col = if n == 0 { self.tab } else if n == 1 { self.player } else { 0 };
        self.follow.set(true);
        true
    }
    pub fn focus_search(&mut self, ctx: &Ctx) -> bool {
        let rows = self.rows(ctx);
        match Self::foci(&rows).iter().position(|(k, _)| matches!(rows[*k], Row::Search)) { Some(n) => self.focus(n, ctx), None => false }
    }

    /// The focused line's entry and what it does, for the state dump.
    pub fn describe(&self, ctx: &Ctx) -> serde_json::Value {
        let rows = self.rows(ctx);
        let (text, act) = match self.focused(&rows) {
            Some((k, j)) => match &rows[k] { Row::Line(_, e) => (Some(e[j].text.clone()), Some(format!("{:?}", e[j].act))), Row::Search => (Some("search".to_string()), None), Row::Head(_) => (None, None) },
            None => (None, None),
        };
        serde_json::json!({ "open": self.open, "tab": TABS[self.tab], "player": self.player, "search": self.search.text(), "focus": self.menu.focus, "focused": text, "action": act,
            "picker": self.picker.is_some(), "notice": self.notice.as_ref().map(|n| &n.0) })
    }

    /// Run an entry: tabs, target and search here; commands go back to play.rs.
    fn activate(&mut self, act: Act, ctx: &Ctx) -> Vec<String> {
        match act {
            Act::Tab(t) => { self.set_tab(t); Vec::new() }
            Act::Player(k) => { self.player = k.min(ctx.players.len().saturating_sub(1)); Vec::new() }
            Act::Search => { self.picker = Some(Default::default()); Vec::new() }
            Act::Card(p, id) => vec![format!("give {p} {id}")],
            Act::Run(c) => c,
        }
    }

    /// Typed text for the search field. Enter gives the first match; Escape
    /// clears the search, or closes the overlay when it is empty.
    pub fn text(&mut self, ev: TextEvent, ctx: &Ctx) -> Vec<String> {
        if !self.text_focus(ctx) { return Vec::new(); }
        match self.search.apply(&ev) {
            FieldEvent::Changed => { self.scroll.set(0.0); Vec::new() }
            FieldEvent::Submit => {
                let p = self.player.min(ctx.players.len().saturating_sub(1));
                ctx.cards.iter().find(|c| self.matches(&c.0, &c.1)).map_or(Vec::new(), |c| vec![format!("give {p} {}", c.0)])
            }
            FieldEvent::Cancel => { if self.search.is_empty() { self.set_open(false); } else { self.search.set(""); } Vec::new() }
            _ => Vec::new(),
        }
    }

    /// A command's reply, shown at the bottom for a few seconds.
    pub fn reply(&mut self, r: Result<String, String>) {
        self.notice = Some(match r { Ok(t) => (t, true), Err(e) => (e, false) });
        self.notice_age = 0.0;
    }

    /// One tick with the local players' merged controls; returns commands to run.
    pub fn update(&mut self, c: &Controls, ctx: &Ctx, dt: f32) -> Vec<String> {
        self.notice_age += dt;
        if self.notice_age > 5.0 { self.notice = None; }
        self.search.tick(dt);
        if let Some(p) = &mut self.picker {
            let Some(ev) = p.update(c, dt) else { return Vec::new() };
            if matches!(ev, TextEvent::Enter | TextEvent::Escape) { self.picker = None; }
            if ev == TextEvent::Escape { return Vec::new(); }
            return self.text(ev, ctx);
        }
        // LB / RB (Q / E): the tabs.
        if c.pressed.has(Buttons::LEFT_SHOULDER) { self.set_tab(self.tab + TABS.len() - 1); return Vec::new(); }
        if c.pressed.has(Buttons::RIGHT_SHOULDER) { self.set_tab(self.tab + 1); return Vec::new(); }
        let rows = self.rows(ctx);
        let foci = Self::foci(&rows);
        if foci.is_empty() { return Vec::new(); }
        let dx = ((c.held.has(Buttons::RIGHT) as i32 - c.held.has(Buttons::LEFT) as i32) + if c.mv[0] > 0.6 { 1 } else if c.mv[0] < -0.6 { -1 } else { 0 }).signum();
        let step = if dx == 0 { self.held_x = 0; 0 } else if dx != self.held_x { self.held_x = dx; self.repeat = 0.35; dx } else { self.repeat -= dt; if self.repeat <= 0.0 { self.repeat = 0.1; dx } else { 0 } };
        self.menu.focus = self.menu.focus.min(foci.len() - 1);
        let (_, n) = foci[self.menu.focus];
        if step != 0 {
            self.col = (self.col.min(n - 1) as i32 + step).clamp(0, n as i32 - 1) as usize;
            // The tab bar and the target row follow left and right at once.
            match self.menu.focus { 0 => self.set_tab(self.col), 1 => self.player = self.col, _ => {} }
            return Vec::new();
        }
        // Only B (and Esc, F3) closes: Back is half of the pad's open chord.
        let strip = Buttons::LEFT | Buttons::RIGHT | Buttons::SELECT;
        let before = self.menu.focus;
        let ev = self.menu.update(&Controls { mv: [0.0, c.mv[1]], held: Buttons(c.held.0 & !strip), pressed: Buttons(c.pressed.0 & !strip), ..*c }, foci.len(), 1, dt);
        if self.menu.focus != before {
            self.follow.set(true);
            self.col = match self.menu.focus { 0 => self.tab, 1 => self.player, _ => self.col.min(foci[self.menu.focus].1 - 1) };
        }
        match ev {
            app::ui::MenuEvent::Select(f) => {
                let (k, n) = foci[f];
                match &rows[k] { Row::Line(_, e) => { let a = e[self.col.min(n - 1)].act.clone(); self.activate(a, ctx) } Row::Search => self.activate(Act::Search, ctx), Row::Head(_) => Vec::new() }
            }
            app::ui::MenuEvent::Back => { self.set_open(false); Vec::new() }
            _ => Vec::new(),
        }
    }

    fn layout(&self,size:[u32;2])->render::editor_window::WindowFrame {
        let [w,h]=size.map(|n|n as f32);
        let u=(h/DESIGN_H).min(w/410.).max(0.01);
        self.window.frame(size,[w-478.*u,8.*u,470.*u,h-16.*u],[390.*u,240.*u],u)
    }
    /// Mouse: hover focuses, a click takes, the wheel scrolls. Targets are
    /// the last drawn frame's.
    pub fn pointer(&mut self, p: app::Pointer, size: [u32; 2], ctx: &Ctx) -> Vec<String> {
        let frame=self.layout(size);
        if self.picker.is_none() {
            match self.window.pointer(size,frame,[390.*frame.scale,240.*frame.scale],p.position,p.pressed,p.held) {
                render::editor_window::WindowEvent::Close=>{self.set_open(false);return Vec::new();},
                render::editor_window::WindowEvent::Handled=>return Vec::new(),
                _=>{}
            }
            if !p.position.is_some_and(|xy|app::ui::contains(frame.body(),xy)) {return Vec::new();}
        }
        if self.hit_size.get() != size { return Vec::new(); }
        if p.wheel != 0.0 { self.scroll.set((self.scroll.get() - p.wheel * 60.0).max(0.0)); }
        if !p.moved && !p.pressed && !p.right_pressed { return Vec::new(); }
        let Some(pos) = p.position else { return Vec::new() };
        let Some(hit) = self.hits.borrow().iter().rev().find(|h| app::ui::contains(h.rect, pos)).copied() else { return Vec::new() };
        let rows = self.rows(ctx);
        let foci = Self::foci(&rows);
        let Some(f) = foci.iter().position(|(k, _)| *k == hit.row) else { return Vec::new() };
        self.menu.focus = f;
        self.col = hit.entry;
        if p.right_pressed {
            return match &rows[hit.row] {
                Row::Line(_, entries) => match entries.get(hit.entry).map(|e| &e.act) {
                    Some(Act::Card(player, id)) => vec![format!("remove {player} {id}")],
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
        }
        if !p.pressed { return Vec::new(); }
        match &rows[hit.row] {
            Row::Line(_, e) => e.get(hit.entry).map_or(Vec::new(), |e| { let a = e.act.clone(); self.activate(a, ctx) }),
            // A click on the field: the keyboard types (the focus is enough).
            _ => Vec::new(),
        }
    }

    pub fn draw(&self, d: &mut HudDrawList, ctx: &Ctx) {
        self.hits.borrow_mut().clear();
        self.hit_size.set(d.size);
        if !self.open { return; }
        d.begin_overlay();
        let [w, h] = d.size.map(|v| v as f32);
        let frame=self.layout(d.size);
        frame.draw(d,0x6767_5220,3,"Match tools");
        let u=frame.scale;
        let panel=frame.body();
        let clip=panel;
        let x0 = panel[0] + 18.0 * u;
        let iw = panel[2] - 30.0 * u;

        Label::new(BODY, 11.0 * u, BONE_DIM).right().draw(d, "F3 / ESC close  ·  Q E tabs  ·  type to search", panel[0] + panel[2] - 12.0 * u, panel[1] + 26.0 * u, clip);
        let rows = self.rows(ctx);
        let focus = self.focused(&rows);
        let rh = 30.0 * u;
        let gap = 5.0 * u;
        // The tab bar and the target are fixed; the rest scrolls.
        let mut y = panel[1] + 46.0 * u;
        for k in 0..2 { self.row(d, &rows[k], k, [x0, y, iw, rh], focus, u, clip); y += rh + gap + if k == 0 { 4.0 * u } else { 0.0 }; }
        d.rect([x0, y + 2.0 * u, iw, u.max(1.0)], with_alpha(RED, 0.5), clip);
        y += 10.0 * u;
        let notice_h = 34.0 * u;
        let area = [x0 - 4.0 * u, y, iw + 8.0 * u, panel[1] + panel[3] - notice_h - y];
        let height = |r: &Row| if matches!(r, Row::Head(_)) { 22.0 * u } else { rh + gap };
        let tops: Vec<f32> = rows[2..].iter().scan(0.0, |acc, r| { let t = *acc; *acc += height(r); Some(t) }).collect();
        let total: f32 = rows[2..].iter().map(height).sum();
        let mut scroll = self.scroll.get().min((total - area[3]).max(0.0));
        if self.follow.get() && let Some((k, _)) = focus && k >= 2 {
            let (top, bottom) = (tops[k - 2], tops[k - 2] + rh);
            if top < scroll { scroll = top; }
            if bottom > scroll + area[3] { scroll = bottom - area[3]; }
            if k == 2 { scroll = 0.0; }
        }
        self.follow.set(false);
        self.scroll.set(scroll);
        for (i, r) in rows[2..].iter().enumerate() {
            let ry = area[1] + tops[i] - scroll;
            if ry + height(r) < area[1] || ry > area[1] + area[3] { continue; }
            self.row(d, r, i + 2, [x0, ry, iw, rh], focus, u, area);
        }
        if total > area[3] {
            // A scroll bar on the right edge.
            let (bx, bh) = (panel[0] + panel[2] - 7.0 * u, area[3] * (area[3] / total));
            d.rect([bx, area[1], 3.0 * u, area[3]], [0.25, 0.22, 0.24, 0.8], clip);
            d.rect([bx, area[1] + (area[3] - bh) * scroll / (total - area[3]).max(1.0), 3.0 * u, bh], RED, clip);
        }
        if let Some((t, ok)) = &self.notice {
            let c = if *ok { GOOD } else { BAD };
            let ny = panel[1] + panel[3] - notice_h * 0.5;
            Label::new(BODY, fit(t, BODY, 12.0 * u, iw), c).draw(d, t, x0, ny, clip);
        }
        if let Some(p) = &self.picker {
            let st = app::ui::Style { panel: [0.06, 0.03, 0.035, 1.0], item: [0.13, 0.11, 0.10, 0.98], focus: RED, text: bytes(BONE), muted: bytes(BONE_DIM), scale: u * 0.75 };
            p.draw(d, 0x6767_5100, [0.0, h * 0.42, w, h * 0.58], &st);
        }
    }

    fn row(&self, d: &mut HudDrawList, row: &Row, k: usize, r: [f32; 4], focus: Option<(usize, usize)>, u: f32, clip: [f32; 4]) {
        let mid = r[1] + r[3] * 0.5;
        match row {
            Row::Head(t) => {
                let s = fit(t, bold(), 15.0 * u, r[2]);
                Label::new(bold(), s, GOLD).draw(d, t, r[0], r[1] + 12.0 * u, clip);
            }
            Row::Search => {
                let fr = [r[0], r[1], r[2], r[3]];
                let on = focus.is_some_and(|f| f.0 == k);
                d.rect(fr, [0.1, 0.08, 0.09, 0.95], clip);
                frame_rect(d, fr, if on { 2.0 * u } else { u.max(1.0) }, if on { RED } else { [0.35, 0.28, 0.3, 1.0] }, clip);
                self.hits.borrow_mut().push(Hit { rect: fr, row: k, entry: 0 });
                let inner = [fr[0] + 10.0 * u, fr[1], fr[2] - 20.0 * u, fr[3]];
                self.search.draw(d, next_id(d), inner, BODY, 14.0 * u, bytes(BONE), ("Search cards...", bytes([0.55, 0.5, 0.42, 1.0])), on.then_some(BONE));
            }
            Row::Line(label, entries) => {
                let lw = if label.is_empty() { 0.0 } else { 100.0 * u };
                if !label.is_empty() {
                    let c = if focus.is_some_and(|f| f.0 == k) { BONE } else { BONE_DIM };
                    Label::new(bold(), fit(label, bold(), 16.0 * u, lw - 6.0 * u), c).draw(d, label, r[0], mid, clip);
                }
                let n = entries.len().max(1) as f32;
                let gap = 6.0 * u;
                let bw = (r[2] - lw - gap * (n - 1.0)) / n;
                for (j, e) in entries.iter().enumerate() {
                    let br = [r[0] + lw + j as f32 * (bw + gap), r[1], bw, r[3]];
                    self.hits.borrow_mut().push(Hit { rect: br, row: k, entry: j });
                    entry(d, br, e, focus == Some((k, j)), u, clip);
                }
            }
        }
    }
}

/// One button: dark until focused (red), a gold edge when it is the current
/// choice, a colour stripe for cards.
fn entry(d: &mut HudDrawList, r: [f32; 4], e: &Entry, focused: bool, u: f32, clip: [f32; 4]) {
    let bg = if focused { RED } else { [0.13, 0.11, 0.10, 0.92] };
    d.rect(r, bg, clip);
    if e.on { frame_rect(d, r, 2.0 * u, GOLD, clip); }
    let mut x = r[0] + 8.0 * u;
    if let Some(c) = e.color {
        d.rect([r[0], r[1], 5.0 * u, r[3]], c, clip);
        x += 3.0 * u;
    }
    let tc = if focused { BONE } else if e.on { GOLD } else { BONE_DIM };
    let inner = [r[0], r[1], r[2], r[3]];
    let clip = intersect(clip, inner);
    if e.sub.is_empty() || e.color.is_none() {
        let room = r[2] - 12.0 * u;
        let size = fit(&e.text, bold(), 16.0 * u, room - if e.sub.is_empty() { 0.0 } else { 26.0 * u });
        if e.color.is_some() { Label::new(bold(), size, tc).draw(d, &e.text, x, r[1] + r[3] * 0.5, clip); }
        else { Label::new(bold(), size, tc).center().draw(d, &e.text, r[0] + r[2] * 0.5, r[1] + r[3] * 0.5, clip); }
        if !e.sub.is_empty() { Label::new(BODY, 10.0 * u, with_alpha(tc, 0.7)).right().draw(d, &e.sub, r[0] + r[2] - 5.0 * u, r[1] + r[3] - 7.0 * u, clip); }
    } else {
        // Cards: the name, the id under it.
        let size = fit(&e.text, bold(), 15.0 * u, r[2] - 16.0 * u);
        Label::new(bold(), size, tc).draw(d, &e.text, x, r[1] + r[3] * 0.36, clip);
        Label::new(BODY, 10.0 * u, with_alpha(tc, 0.65)).draw(d, &e.sub, x, r[1] + r[3] * 0.78, clip);
    }
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// The `pickup` command's name for a kind (`plus_one`, `air_strike`).
pub fn pickup_name(k: growing_guns::PickupKind) -> String {
    let mut s = String::new();
    for (i, ch) in format!("{k:?}").chars().enumerate() {
        if ch.is_uppercase() && i > 0 { s.push('_'); }
        s.push(ch.to_ascii_lowercase());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx { cards: vec![("explosive".into(), "Explosive Rounds".into(), [1.0, 0.5, 0.2, 1.0], false), ("rapid_fire".into(), "Rapid Fire".into(), [0.4, 0.8, 1.0, 1.0], true)],
            players: vec![("Joep".into(), [1.0, 2.0, 3.0], false), ("Bot".into(), [0.0; 3], true)], sky: "godot".into(), max_health: 100, mag: 8, special: "grenade", ..Default::default() }
    }
    fn press(b: u32) -> Controls { Controls { held: Buttons(b), pressed: Buttons(b), ..Default::default() } }

    #[test]
    fn right_click_removes_one_card_only_from_hovered_target() {
        let c = ctx();
        let mut m = DebugMenu::default();
        m.set_open(true);
        m.player = 1;
        let size = [1600, 900];
        m.draw(&mut HudDrawList::new(size), &c);
        let rows = m.rows(&c);
        let hit = *m.hits.borrow().iter().find(|h| matches!(&rows[h.row], Row::Line(_, e) if matches!(e[h.entry].act, Act::Card(_, _)))).unwrap();
        let pointer = app::Pointer { position: Some([hit.rect[0]+4.0, hit.rect[1]+4.0]), right_pressed: true, ..Default::default() };
        assert_eq!(m.pointer(pointer, size, &c), vec!["remove 1 explosive"]);
        assert!(m.pointer(app::Pointer { right_pressed: false, ..pointer }, size, &c).is_empty());
        assert!(m.pointer(pointer, [800,600], &c).is_empty());
        assert_eq!(m.pointer(app::Pointer { pressed: true, right_pressed: false, ..pointer }, size, &c), vec!["give 1 explosive"]);
    }

    #[test]
    fn search_filters_and_enter_gives_the_first_match_to_the_target() {
        let c = ctx();
        let mut m = DebugMenu::default();
        m.set_open(true);
        assert!(m.focus_search(&c));
        assert!(m.text_focus(&c));
        m.text(TextEvent::Insert("rapid".into()), &c);
        assert_eq!(m.text(TextEvent::Enter, &c), vec!["give 0 rapid_fire".to_string()]);
        // The target row: right picks player 2; a card then goes to them.
        m.focus(1, &c);
        m.update(&press(Buttons::RIGHT), &c, 0.016);
        assert_eq!(m.player, 1);
        m.update(&Controls::default(), &c, 0.016);
        m.update(&press(Buttons::DOWN), &c, 0.016);
        m.update(&Controls::default(), &c, 0.016);
        m.update(&press(Buttons::DOWN), &c, 0.016);
        assert_eq!(m.update(&press(Buttons::SOUTH), &c, 0.016), vec!["give 1 rapid_fire".to_string()]);
        // Escape clears the search, then closes.
        m.focus_search(&c);
        m.text(TextEvent::Escape, &c);
        assert!(m.search.is_empty() && m.open);
        m.text(TextEvent::Escape, &c);
        assert!(!m.open);
    }

    #[test]
    fn tabs_switch_on_the_shoulders_and_modes_start_the_modifier_now() {
        let c = ctx();
        let mut m = DebugMenu::default();
        m.set_open(true);
        m.update(&press(Buttons::LEFT_SHOULDER), &c, 0.016);
        assert_eq!(TABS[m.tab], "MODES");
        let rows = m.rows(&c);
        let blackout = rows.iter().find_map(|r| match r { Row::Line(_, e) => e.iter().find(|e| e.text == "BLACKOUT").cloned(), _ => None }).expect("blackout");
        assert_eq!(blackout.act, Act::Run(vec!["modifier blackout".into(), "round".into()]));
        m.update(&press(Buttons::RIGHT_SHOULDER), &c, 0.016);
        m.update(&press(Buttons::RIGHT_SHOULDER), &c, 0.016);
        assert_eq!(TABS[m.tab], "ITEMS");
        assert_eq!(pickup_name(growing_guns::PickupKind::AirStrike), "air_strike");
        // B closes; Back alone does not (it is half of the pad's chord).
        m.update(&press(Buttons::SELECT), &c, 0.016);
        assert!(m.open);
        m.update(&press(Buttons::EAST), &c, 0.016);
        assert!(!m.open);
    }
}
