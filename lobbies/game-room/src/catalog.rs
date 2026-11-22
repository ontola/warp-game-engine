//! The catalog view of the Game Room: every game on the shelf as big covers
//! over the whole screen. It is a view of the room, not a menu: R3 at the
//! game shelf (or a click on the shelf or its All games button) switches to
//! it, R3, B or the Room button back, and each device remembers the view it
//! was left in and starts in it. LB/RB (or left/right, or the wheel) browse, A or Y plays the
//! game in the middle now, X queues it, B or Back closes. The mouse clicks a
//! cover at the side to bring it to the middle, and the middle one or the
//! buttons under it to play or queue.
//!
//! Covers come from the library entries' `cover` (or `screenshot`) as a
//! `data:image/png;base64,…` URI, as the host sends them, or from
//! [`Lobby::set_cover`] for a host that has them as files or URLs (the
//! browser). Games without one get a title card in their colour.

use crate::host::get;
use crate::live::Lobby;
use app::Pointer;
use base64::Engine;
use render::hud::HudImage;
use render::picture::Picture;
use crate::paint::Canvas;
use std::sync::Arc;

/// A decoded cover: the HUD image it is drawn as, and its size.
#[derive(Clone)]
pub struct Cover { pub image: HudImage, pub picture: Arc<Picture>, pub size: [u32; 2] }

#[derive(Default)]
pub struct Catalog {
    /// Open, with the shelf offset easing towards its slot (for the slide).
    pub open: bool,
    pub shown: f32,
    /// When it opened, for the fade in.
    pub opened_at: f32,
    /// The covers decoded so far (None: none or undecodable).
    pub covers: std::collections::HashMap<String, Option<Cover>>,
    /// Each player's stick last frame, so a push browses once.
    pub sticks: std::collections::HashMap<String, [f32; 2]>,
    next_image: u64,
    /// Remember the view on this device (real runs, not tests or shots).
    pub remember: bool,
}

/// What the catalog draws where: per visible game, its shelf index and
/// rectangle (middle last), and the play, queue and close buttons.
pub struct Layout { pub cards: Vec<(usize, [f32; 4])>, pub buttons: Vec<(&'static str, [f32; 4])> }

impl Lobby {
    /// The catalog view is showing (chosen, and there are games for it).
    pub fn catalog_open(&self) -> bool { self.catalog.open && !self.host.shelf_games().is_empty() }

    pub fn open_catalog(&mut self) {
        if self.host.shelf_games().is_empty() { self.toast("No games on the shelf yet.", crate::lobby::DIM); return; }
        self.show_catalog(true);
    }

    /// Back to the room view.
    pub fn close_catalog(&mut self) { self.show_catalog(false); }

    fn show_catalog(&mut self, open: bool) {
        if open && !self.catalog.open {
            self.catalog.opened_at = self.time;
            self.catalog.shown = self.host.shelf_offset as f32;
            self.catalog.sticks.clear();
            self.menus.remove(crate::mouse::MOUSE);
        }
        self.catalog.open = open;
        self.screens_dirty = true;
        if self.catalog.remember { store_view(open); }
    }

    /// Remember the view on this device from now on, and start in the one
    /// it was left in.
    pub fn remember_view(&mut self) {
        self.catalog.remember = true;
        if stored_view() { self.catalog.open = true; self.catalog.opened_at = self.time; }
    }

    /// A host that has covers as files or URLs hands them over here.
    pub fn set_cover(&mut self, game: &str, png: &[u8]) {
        let cover = self.decode_cover(png);
        self.catalog.covers.insert(game.to_string(), cover);
    }

    fn decode_cover(&mut self, bytes: &[u8]) -> Option<Cover> {
        let art = image::load_from_memory(bytes).ok()?.to_rgba8();
        let size = [art.width(), art.height()];
        let picture = Arc::new(Picture::new(size[0], size[1], art.into_raw()).ok()?);
        self.catalog.next_image += 1;
        Some(Cover { image: HudImage(3000 + self.catalog.next_image), picture, size })
    }

    /// Decode the covers of the shelf's games, a couple per frame, and ease
    /// the slide. Called every frame.
    pub(crate) fn update_catalog(&mut self, delta: f32) {
        if !self.catalog_open() { return; }
        let n = self.host.shelf_games().len() as f32;
        // Ease towards the shelf offset the short way round.
        let target = self.host.shelf_offset as f32;
        let mut d = (target - self.catalog.shown).rem_euclid(n);
        if d > n / 2.0 { d -= n; }
        self.catalog.shown = if d.abs() < 0.002 { target } else { (self.catalog.shown + d * (1.0 - (-delta * 14.0).exp())).rem_euclid(n) };
        let wanted: Vec<(String, String)> = self.host.shelf_games().iter()
            .map(|g| (get(g, "id", ""), [get(g, "cover", ""), get(g, "screenshot", "")].into_iter().find(|v| v.starts_with("data:")).unwrap_or_default()))
            .filter(|(id, _)| !self.catalog.covers.contains_key(id))
            .take(2).collect();
        for (id, uri) in wanted {
            let bytes = uri.split_once(";base64,").and_then(|(_, b)| base64::engine::general_purpose::STANDARD.decode(b.trim()).ok());
            let cover = bytes.and_then(|b| self.decode_cover(&b));
            self.catalog.covers.insert(id, cover);
        }
    }

    /// A player's buttons while the catalog is open.
    pub(crate) fn catalog_input(&mut self, id: Option<&str>, pressed: u32, stick: [f32; 2]) {
        use game_room_sim::world::*;
        let stick_was = id.and_then(|id| self.catalog.sticks.insert(id.into(), stick)).unwrap_or([0.0; 2]);
        let left = pressed & (BTN_LB | BTN_LEFT) != 0 || (stick[0] < -0.6 && stick_was[0] >= -0.6);
        let right = pressed & (BTN_RB | BTN_RIGHT) != 0 || (stick[0] > 0.6 && stick_was[0] <= 0.6);
        if left { self.browse(-1); }
        if right { self.browse(1); }
        if pressed & (BTN_B | BTN_RS) != 0 { self.close_catalog(); return; }
        if pressed & (BTN_A | BTN_Y) != 0 { self.catalog_use(id, false); }
        if pressed & BTN_X != 0 { self.catalog_use(id, true); }
    }

    /// Play the middle game now, or queue it, as the shelf's Y and X do.
    /// The view stays: the room comes back to it after the game.
    fn catalog_use(&mut self, id: Option<&str>, queue: bool) {
        let x = (game_room_sim::layout::GAME_SHELF.0 + game_room_sim::layout::GAME_SHELF.1) / 2.0;
        let Some(station) = self.host.station_at(x, None) else { return };
        if !station.enabled { self.toast(station.label.clone(), crate::lobby::DIM); return; }
        self.use_station_as(id, station, queue);
    }

    /// The mouse in the open catalog: the wheel browses, a click on a side
    /// cover brings it to the middle, on the middle one (or Play) plays it.
    pub(crate) fn catalog_pointer(&mut self, p: Pointer, size: [u32; 2]) {
        if p.wheel != 0.0 { self.browse(if p.wheel > 0.0 { -1 } else { 1 }); }
        let (Some(at), true) = (p.position, p.pressed) else { return };
        let layout = catalog_layout(self, size);
        if let Some((name, _)) = layout.buttons.iter().find(|(_, r)| inside(*r, at)) {
            match *name { "play" => self.catalog_use(None, false), "queue" => self.catalog_use(None, true), _ => self.close_catalog() }
            return;
        }
        // The middle card is drawn last, so look from the end.
        match layout.cards.iter().rev().find(|(_, r)| inside(*r, at)) {
            Some(&(i, _)) if i == self.host.shelf_offset => self.catalog_use(None, false),
            Some(&(i, _)) => {
                let n = self.host.shelf_games().len() as i64;
                let mut step = (i as i64 - self.host.shelf_offset as i64).rem_euclid(n);
                if step > n / 2 { step -= n; }
                self.browse(step);
            }
            None => {}
        }
    }
}

const VIEW_KEY: &str = "splitscreen-club.game-room-view";

#[cfg(target_arch = "wasm32")]
fn stored_view() -> bool { app::web::stored(VIEW_KEY).as_deref() == Some("catalog") }
#[cfg(target_arch = "wasm32")]
fn store_view(catalog: bool) { app::web::store(VIEW_KEY, if catalog { "catalog" } else { "room" }); }

/// Where this machine keeps the view: the per-user app data folder.
#[cfg(not(target_arch = "wasm32"))]
fn view_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("Splitscreen Club").join(VIEW_KEY))
}
#[cfg(not(target_arch = "wasm32"))]
fn stored_view() -> bool { view_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|v| v.trim() == "catalog") }
#[cfg(not(target_arch = "wasm32"))]
fn store_view(catalog: bool) {
    let Some(file) = view_file() else { return };
    if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(file, if catalog { "catalog" } else { "room" });
}

fn inside(r: [f32; 4], p: [f32; 2]) -> bool { p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3] }

/// Where the cards and buttons go on a screen of `size`: the middle cover
/// tall, two each side smaller and further back.
pub fn catalog_layout(lobby: &Lobby, size: [u32; 2]) -> Layout {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let n = lobby.host.shelf_games().len();
    let mut cards = vec![];
    if n == 0 { return Layout { cards, buttons: vec![] }; }
    let card_h = h * 0.56;
    let card_w = card_h * 0.75;
    let (cx, cy) = (w / 2.0, h * 0.44);
    let shown = lobby.catalog.shown;
    let reach = 2.min((n as i64 - 1) / 2 + (n as i64 - 1) % 2) as i64;
    let mut order: Vec<(f32, usize)> = vec![];
    for k in -reach..=reach {
        let i = (shown.round() as i64 + k).rem_euclid(n as i64) as usize;
        let mut off = i as f32 - shown;
        if off > n as f32 / 2.0 { off -= n as f32; }
        if off < -(n as f32) / 2.0 { off += n as f32; }
        order.push((off, i));
    }
    // Far ones first, so the middle one draws over its neighbours.
    order.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    order.dedup_by_key(|o| o.1);
    for (off, i) in order {
        let scale = 1.0 - 0.2 * off.abs().min(2.5);
        let (cw, ch) = (card_w * scale, card_h * scale);
        // Each card tucks a little under the one in front of it.
        let a = off.abs();
        let along = if a <= 1.0 { 0.74 * a } else { 0.74 + 0.56 * (a - 1.0) };
        let x = cx + off.signum() * along * card_w - cw / 2.0;
        cards.push((i, [x, cy - ch / 2.0, cw, ch]));
    }
    let bh = (h * 0.06).max(36.0);
    let y = cy + card_h / 2.0 + h * 0.1;
    let bw = bh * 3.6;
    let gap = bh * 0.4;
    let x0 = cx - (bw * 3.0 + gap * 2.0) / 2.0;
    let buttons = vec![("play", [x0, y, bw, bh]), ("queue", [x0 + bw + gap, y, bw, bh]), ("close", [x0 + 2.0 * (bw + gap), y, bw, bh])];
    Layout { cards, buttons }
}

/// The part of a `size` picture that fills `r` (centre crop).
fn fill_uv(size: [u32; 2], r: [f32; 4]) -> [f32; 4] {
    let ia = size[0] as f32 / size[1].max(1) as f32;
    let ra = r[2] / r[3].max(1.0);
    if ia > ra { let uw = ra / ia; [(1.0 - uw) / 2.0, 0.0, uw, 1.0] } else { let vh = ia / ra; [0.0, (1.0 - vh) / 2.0, 1.0, vh] }
}

/// A game without a cover: its colour, top to bottom darker, and its
/// title; `k` dims it (cards further back), `fade` fades it in.
fn title_card(c: &mut Canvas, game: &crate::paint::Game, r: [f32; 4], open: [f32; 4], k: f32, fade: f32) {
    use crate::godot::Color;
    let color = Color::from_string(&game.color, crate::lobby::VIOLET);
    let bands = 32;
    for b in 0..bands {
        let (top, bottom) = ((r[3] * b as f32 / bands as f32).round(), (r[3] * (b + 1) as f32 / bands as f32).round());
        let shade = color.darkened(0.35 + 0.45 * (b as f32 + 0.5) / bands as f32);
        c.rect([r[0], r[1] + top, r[2], bottom - top], Color { r: shade.r * k, g: shade.g * k, b: shade.b * k, a: fade });
    }
    // The title, as big as fits the part that shows.
    let title = if game.title.is_empty() { "Game" } else { &game.title };
    let words: Vec<String> = title.split(' ').map(str::to_uppercase).collect();
    let face = crate::fonts::faces().bold;
    let widest = words.iter().map(|w| render::hud::measure(w, face, 100.0)[0]).fold(1.0, f32::max);
    let size = (r[2] * 0.13).min(open[2] * 0.8 * 100.0 / widest).floor().max(8.0);
    let line = size * 1.1;
    let top = r[1] + r[3] / 2.0 - (words.len() as f32 - 1.0) * line / 2.0 + size * 0.35;
    let ink = crate::lobby::TEXT;
    for (i, word) in words.iter().enumerate() {
        c.text(face, [open[0], top + i as f32 * line], word, render::hud::TextAlign::Center, open[2], size,
            Color { r: ink.r * k, g: ink.g * k, b: ink.b * k, a: fade });
    }
}

/// The open catalog over the whole screen.
pub fn draw(lobby: &Lobby, c: &mut Canvas, size: [u32; 2]) {
    use crate::godot::{rgba, Color};
    use crate::lobby::{DIM, TEXT, VIOLET};
    use render::hud::TextAlign;
    let (w, h) = (size[0] as f32, size[1] as f32);
    let scale = h / 1080.0;
    let shelf = &lobby.show.shelf;
    if shelf.is_empty() { return; }
    let fade = ((lobby.time - lobby.catalog.opened_at) / 0.2).clamp(0.0, 1.0);
    let middle = lobby.host.shelf_offset % shelf.len();
    let cover = |i: usize| shelf.get(i).and_then(|g| lobby.catalog.covers.get(&g.id)).and_then(|c| c.as_ref());
    let faces = crate::fonts::faces();
    c.over();
    // The middle game's cover, dark and big, behind everything.
    c.rect([0.0, 0.0, w, h], rgba(0.02, 0.02, 0.04, fade));
    // The middle game's colour as a soft glow from the top (its cover is
    // already the big card: not again behind it).
    let color = Color::from_string(&shelf[middle].color, VIOLET);
    let glow = 24;
    for k in 0..glow {
        let (top, bottom) = ((h * 0.7 * k as f32 / glow as f32).round(), (h * 0.7 * (k + 1) as f32 / glow as f32).round());
        let t = 1.0 - (k as f32 + 0.5) / glow as f32;
        c.rect([0.0, top, w, bottom - top], color.darkened(0.55).with_alpha(0.45 * t * t * fade));
    }
    // Darker towards the bottom, for the title and buttons.
    let bands = 24;
    for k in 0..bands {
        let (top, bottom) = ((h * (0.5 + 0.5 * k as f32 / bands as f32)).round(), (h * (0.5 + 0.5 * (k + 1) as f32 / bands as f32)).round());
        c.rect([0.0, top, w, bottom - top], rgba(0.0, 0.0, 0.0, 0.55 * (k as f32 + 0.5) / bands as f32 * fade));
    }
    c.text(faces.bold, [48.0 * scale, 72.0 * scale], "ALL GAMES", TextAlign::Left, -1.0, (34.0 * scale).floor(), TEXT.with_alpha(fade));
    let count = format!("{} of {}", middle + 1, shelf.len());
    let cs = (24.0 * scale).floor();
    let cw = render::hud::measure(&count, faces.regular, cs)[0];
    c.text(faces.regular, [w - 48.0 * scale - cw, 72.0 * scale], &count, TextAlign::Left, -1.0, cs, DIM.with_alpha(fade));

    let layout = catalog_layout(lobby, size);
    let mouse = lobby.mouse.at;
    for &(i, r) in &layout.cards {
        c.over();
        let mid = i == middle;
        let hover = mouse.is_some_and(|m| inside(r, m));
        // A soft shadow, then the cover or a title card.
        c.style_box([r[0] - 6.0 * scale, r[1] + 10.0 * scale, r[2] + 12.0 * scale, r[3] + 8.0 * scale], [18.0 * scale; 4], rgba(0.0, 0.0, 0.0, 0.45 * fade), None);
        if mid || hover {
            let b = 5.0 * scale;
            c.style_box([r[0] - b, r[1] - b, r[2] + 2.0 * b, r[3] + 2.0 * b], [14.0 * scale; 4], if mid { TEXT } else { DIM }.with_alpha(fade), None);
        }
        // Further back is darker (one layer per card: the HUD has few).
        let far = (1.0 - r[3] / (h * 0.56)) / 0.2;
        let k = if mid { 1.0 } else if hover { 0.85 } else { (0.68 - 0.18 * far).max(0.3) };
        // The part no nearer card covers: a card's title sits there, and
        // nothing of it is drawn under the cards in front.
        let (mut x0, mut x1) = (r[0], r[0] + r[2]);
        for &(j, n) in layout.cards.iter().skip_while(|c| c.0 != i).skip(1) {
            if j == i { continue; }
            if n[0] > r[0] && n[0] < x1 { x1 = n[0]; }
            if n[0] + n[2] < r[0] + r[2] && n[0] + n[2] > x0 { x0 = n[0] + n[2]; }
        }
        let open = [x0, r[1], (x1 - x0).max(1.0), r[3]];
        c.clip_to(Some(open));
        match cover(i) {
            Some(art) => { c.provide(art.image, &art.picture); c.image(r, art.image, fill_uv(art.size, r), rgba(k, k, k, fade)); }
            None => title_card(c, &shelf[i], r, open, k, fade),
        }
        c.clip_to(None);
    }
    c.over();
    let station = lobby.host.station_at((game_room_sim::layout::GAME_SHELF.0 + game_room_sim::layout::GAME_SHELF.1) / 2.0, None);
    let card_bottom = h * 0.44 + h * 0.28;
    let title = if shelf[middle].title.is_empty() { "Game" } else { &shelf[middle].title };
    c.text(faces.bold, [0.0, card_bottom + 58.0 * scale], title, TextAlign::Center, w, (44.0 * scale).floor(), TEXT.with_alpha(fade));
    if let Some(s) = station.as_ref().filter(|s| !s.enabled) {
        c.text(faces.regular, [0.0, card_bottom + 92.0 * scale], &s.label, TextAlign::Center, w, (22.0 * scale).floor(), crate::lobby::AMBER.with_alpha(fade));
    }
    let enabled = station.as_ref().is_some_and(|s| s.enabled);
    for &(name, r) in &layout.buttons {
        let hover = mouse.is_some_and(|m| inside(r, m));
        let (label, key, accent) = match name { "play" => ("Play now", "A", crate::lobby::GREEN), "queue" => ("Queue", "X", Color::hex("4a8fe7")), _ => ("Room", "B", Color::hex("e2574c")) };
        let live = enabled || name == "close";
        let bg = if hover && live { accent.darkened(0.2) } else { rgba(0.06, 0.06, 0.1, 0.92) };
        c.style_box(r, [r[3] / 2.0; 4], bg.with_alpha(bg.a * fade), Some((2.0 * scale, accent.with_alpha(if live { fade } else { 0.35 * fade }))));
        let fs = (r[3] * 0.42).floor();
        // The controller button in a little disc, then the label.
        let disc = [r[0] + r[3] * 0.55, r[1] + r[3] / 2.0];
        c.circle(disc, r[3] * 0.32, accent.with_alpha(if live { fade } else { 0.35 * fade }));
        c.text(faces.bold, [disc[0] - r[3], disc[1] + fs * 0.36], key, TextAlign::Center, 2.0 * r[3], (fs * 0.9).floor(), Color::hex("0c0a16").with_alpha(fade));
        c.text(faces.regular, [r[0] + r[3], r[1] + r[3] / 2.0 + fs * 0.36], label, TextAlign::Center, r[2] - r[3] * 1.2, fs, TEXT.with_alpha(if live { fade } else { 0.4 * fade }));
    }
    // The queue, small, bottom right.
    let queue = lobby.host.upcoming();
    if !queue.is_empty() {
        let fs = (19.0 * scale).floor();
        let line = fs * 1.55;
        let shown = queue.len().min(4);
        let qw = queue.iter().take(shown).map(|e| render::hud::measure(&e.title, faces.regular, fs)[0]).fold(160.0 * scale, f32::max) + fs * 3.2;
        let qh = fs * 2.4 + line * shown as f32 + if queue.len() > shown { line * 0.8 } else { 0.0 };
        let (qx, qy) = (w - qw - 32.0 * scale, h - qh - 72.0 * scale);
        c.over();
        c.style_box([qx, qy, qw, qh], [fs * 0.5; 4], rgba(0.03, 0.03, 0.06, 0.78 * fade), None);
        c.rect([qx, qy + fs * 0.6, 3.0 * scale, fs * 1.2], crate::lobby::GREEN.with_alpha(fade));
        c.text(faces.bold, [qx + fs, qy + fs * 1.6], "UP NEXT", TextAlign::Left, -1.0, (fs * 0.85).floor(), crate::lobby::GREEN.with_alpha(fade));
        for (i, e) in queue.iter().take(shown).enumerate() {
            let y = qy + fs * 2.4 + line * (i as f32 + 0.75);
            c.text(faces.regular, [qx + fs, y], &format!("{:02}", i + 1), TextAlign::Left, -1.0, (fs * 0.8).floor(), DIM.with_alpha(fade));
            c.text(faces.regular, [qx + fs * 2.6, y], &e.title, TextAlign::Left, -1.0, fs, TEXT.with_alpha(fade));
        }
        if queue.len() > shown {
            c.text(faces.regular, [qx + fs * 2.6, qy + qh - fs * 0.6], &format!("+{} more", queue.len() - shown), TextAlign::Left, -1.0, (fs * 0.8).floor(), DIM.with_alpha(fade));
        }
    }
    let hint = "LB / RB browse  ·  click a cover to bring it to the middle  ·  R3 or B: back to the room";
    c.text(faces.regular, [0.0, h - 36.0 * scale], hint, TextAlign::Center, w, (19.0 * scale).floor(), DIM.with_alpha(fade));
}
