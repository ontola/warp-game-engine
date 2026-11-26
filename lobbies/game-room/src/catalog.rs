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
pub use crate::paint::Cover;

#[derive(Default)]
pub struct Catalog {
    /// Open; the first row shown (eased towards `scroll`, in rows).
    pub open: bool,
    pub shown: f32,
    scroll: f32,
    /// The choice the scroll last followed: the wheel scrolls freely until
    /// the choice changes again.
    followed: Option<usize>,
    /// When it opened, for the fade in, and when it closed, for the fade out.
    pub opened_at: f32,
    pub closed_at: Option<f32>,
    /// The covers decoded so far (None: none or undecodable).
    pub covers: std::collections::HashMap<String, Option<Cover>>,
    /// Gameplay screenshots for the TV, by game id: the store's, or the
    /// host's capture of a game left paused ([`Lobby::set_screenshot`]).
    pub shots: std::collections::HashMap<String, Option<Cover>>,
    /// Which data URI each screenshot came from (a cheap hash).
    shot_src: std::collections::HashMap<String, u64>,
    /// Each player's stick last frame, so a push browses once.
    pub sticks: std::collections::HashMap<String, [f32; 2]>,
    next_image: u64,
    /// Remember the view on this device (real runs, not tests or shots).
    pub remember: bool,
}

/// What the catalog draws where: per visible game, its shelf index and
/// rectangle, the grid's area (cards clip to it), and the play, queue and
/// close buttons.
pub struct Layout { pub cards: Vec<(usize, [f32; 4])>, pub area: [f32; 4], pub buttons: Vec<(&'static str, [f32; 4])> }

/// The grid on a screen of `size` for `n` games: columns, card size, gap,
/// the area it scrolls in and how many rows fit there.
struct Grid { cols: usize, card: [f32; 2], gap: f32, area: [f32; 4], rows: usize }

fn grid(size: [u32; 2], n: usize) -> Grid {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let scale = crate::hud::ui(h);
    let area = [48.0 * scale, 112.0 * scale, w - 96.0 * scale, h - 112.0 * scale - 250.0 * scale];
    let gap = 28.0 * scale;
    // Two rows of covers, up to 30% of the screen high each, never so
    // wide that fewer than three fit across.
    let card_h = (h * 0.3).min((area[3] - gap) / 2.0);
    let mut card = [card_h * 0.75, card_h];
    let mut cols = (((area[2] + gap) / (card[0] + gap)).floor() as usize).max(1);
    if cols < 3 { cols = 3; card[0] = (area[2] - gap * 2.0) / 3.0; card[1] = card[0] / 0.75; }
    let rows = (((area[3] + gap) / (card[1] + gap)).floor() as usize).max(1);
    // Games that fit on one screen spread over its rows (8 on a wide
    // phone: 2x4, not one row of 8).
    cols = if n <= cols * rows { n.div_ceil(rows).max(3) } else { cols }.min(cols).min(n.max(1));
    Grid { cols, card, gap, area, rows }
}

/// Seconds the catalog fades in and out.
const FADE: f32 = 0.2;

impl Lobby {
    /// How much of the catalog shows, 0..1: fading in after it opens, out
    /// after it closes.
    pub fn catalog_fade(&self) -> f32 {
        let n = !self.host.shelf_games().is_empty();
        match self.catalog.closed_at {
            Some(t) if !self.catalog.open && n => (1.0 - (self.time - t) / FADE).clamp(0.0, 1.0),
            _ if self.catalog_open() => ((self.time - self.catalog.opened_at) / FADE).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// The catalog view is showing (chosen, and there are games for it).
    pub fn catalog_open(&self) -> bool { self.catalog.open && !self.host.shelf_games().is_empty() }

    pub fn open_catalog(&mut self) {
        if self.host.shelf_games().is_empty() { self.toast("No games on the shelf yet.", crate::lobby::DIM); return; }
        self.show_catalog(true);
    }

    /// Back to the room view.
    pub fn close_catalog(&mut self) { self.show_catalog(false); }

    fn show_catalog(&mut self, open: bool) {
        if !open && self.catalog_open() { self.catalog.closed_at = Some(self.time); }
        if open && !self.catalog.open {
            // Reopened mid fade out: fade in from where it was.
            let shown = self.catalog_fade();
            self.catalog.opened_at = self.time - shown * FADE;
            self.catalog.closed_at = None;
            let cols = grid(self.hud_size, self.host.shelf_games().len()).cols;
            self.catalog.scroll = (self.host.shelf_offset / cols) as f32;
            self.catalog.shown = self.catalog.scroll;
            self.catalog.sticks.clear();
            self.menus.remove(crate::mouse::MOUSE);
        }
        // The order stays put while the view shows; once it has faded out,
        // the games that don't fit go last again (update_catalog).
        if open { self.host.shelf_order = None; self.host.shelf_order = Some(self.host.shelf_games().iter().map(|g| get(g, "id", "")).collect()); }
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

    /// A gameplay screenshot (PNG/JPEG) for the TV: the store's, or the
    /// current state of a game the host just paused.
    pub fn set_screenshot(&mut self, game: &str, png: &[u8]) {
        let shot = self.decode_cover(png);
        self.catalog.shots.insert(game.to_string(), shot);
        self.screens_dirty = true;
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
        // Covers decode whether or not the catalog shows: the shelf shows
        // its neighbours' covers too.
        self.decode_covers();
        if !self.catalog.open && self.catalog_fade() == 0.0 { self.host.shelf_order = None; }
        if !self.catalog_open() { return; }
        // Scroll (in rows) so the chosen game's row shows, eased.
        let n = self.host.shelf_games().len();
        let g = grid(self.hud_size, n);
        let row = (self.host.shelf_offset / g.cols) as f32;
        let last = (n.div_ceil(g.cols).saturating_sub(g.rows)) as f32;
        let mut target = self.catalog.scroll;
        if self.catalog.followed != Some(self.host.shelf_offset) {
            self.catalog.followed = Some(self.host.shelf_offset);
            target = target.clamp(row - g.rows as f32 + 1.0, row);
        }
        let target = target.clamp(0.0, last);
        self.catalog.scroll = target;
        let d = target - self.catalog.shown;
        self.catalog.shown = if d.abs() < 0.002 { target } else { self.catalog.shown + d * (1.0 - (-delta * 14.0).exp()) };
    }

    fn decode_covers(&mut self) {
        let wanted: Vec<(String, String)> = self.host.shelf_games().iter()
            .map(|g| (get(g, "id", ""), [get(g, "cover", ""), get(g, "screenshot", "")].into_iter().find(|v| v.starts_with("data:")).unwrap_or_default()))
            .filter(|(id, _)| !self.catalog.covers.contains_key(id))
            .take(2).collect();
        for (id, uri) in wanted {
            let cover = self.decode_uri(&uri);
            self.catalog.covers.insert(id, cover);
        }
        // The TV's screenshots, from the library's data URIs: a host's
        // capture of the game as it was left (`live_screenshot`, replaced
        // whenever it changes) before the store's.
        let wanted: Vec<(String, String)> = self.host.library().iter()
            .map(|g| (get(g, "id", ""), [get(g, "live_screenshot", ""), get(g, "gameplay", ""), get(g, "screenshot", "")].into_iter().find(|v| v.starts_with("data:")).unwrap_or_default()))
            .filter(|(id, uri)| !uri.is_empty() && self.catalog.shot_src.get(id) != Some(&quick_hash(uri)))
            .take(1).collect();
        for (id, uri) in wanted {
            self.catalog.shot_src.insert(id.clone(), quick_hash(&uri));
            let shot = self.decode_uri(&uri);
            self.catalog.shots.insert(id, shot);
        }
    }

    fn decode_uri(&mut self, uri: &str) -> Option<Cover> {
        let bytes = uri.split_once(";base64,").and_then(|(_, b)| base64::engine::general_purpose::STANDARD.decode(b.trim()).ok())?;
        self.decode_cover(&bytes)
    }

    /// A player's buttons while the catalog is open.
    pub(crate) fn catalog_input(&mut self, id: Option<&str>, pressed: u32, stick: [f32; 2]) {
        use game_room_sim::world::*;
        let stick_was = id.and_then(|id| self.catalog.sticks.insert(id.into(), stick)).unwrap_or([0.0; 2]);
        let left = pressed & (BTN_LB | BTN_LEFT) != 0 || (stick[0] < -0.6 && stick_was[0] >= -0.6);
        let right = pressed & (BTN_RB | BTN_RIGHT) != 0 || (stick[0] > 0.6 && stick_was[0] <= 0.6);
        let up = pressed & BTN_UP != 0 || (stick[1] < -0.6 && stick_was[1] >= -0.6);
        let down = pressed & BTN_DOWN != 0 || (stick[1] > 0.6 && stick_was[1] <= 0.6);
        if left { self.browse(-1); }
        if right { self.browse(1); }
        // Up and down a row; not past the first or last one.
        let n = self.host.shelf_games().len();
        let cols = grid(self.hud_size, n).cols;
        let at = self.host.shelf_offset;
        if up && at >= cols { self.browse(-(cols as i64)); }
        if down && at / cols < n.saturating_sub(1) / cols { self.browse(((at + cols).min(n - 1) - at) as i64); }
        if pressed & BTN_B != 0 { self.close_catalog(); return; }
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

    /// The mouse in the open catalog: the wheel scrolls, a click on a cover
    /// chooses it, on the chosen one (or Play) plays it.
    pub(crate) fn catalog_pointer(&mut self, p: Pointer, size: [u32; 2]) {
        if p.wheel != 0.0 {
            let n = self.host.shelf_games().len();
            let g = grid(size, n);
            let last = n.div_ceil(g.cols).saturating_sub(g.rows) as f32;
            // Half a row per wheel line: a notch moves, a trackpad glides.
            self.catalog.scroll = (self.catalog.scroll - p.wheel * 0.5).clamp(0.0, last);
        }
        let (Some(at), true) = (p.position, p.pressed) else { return };
        let layout = catalog_layout(self, size);
        if let Some((name, _)) = layout.buttons.iter().find(|(_, r)| inside(*r, at)) {
            match *name { "play" => self.catalog_use(None, false), "queue" => self.catalog_use(None, true), _ => self.close_catalog() }
            return;
        }
        if !inside(layout.area, at) { return; }
        // Play or Queue on the hovered card: that game, wherever the choice was.
        let scale = crate::hud::ui(size[1] as f32);
        let hit = layout.cards.iter().find_map(|&(i, r)| {
            if !self.host.shelf_games().get(i).is_some_and(|g| self.host.fits_party(g)) { return None; }
            let buttons = card_buttons(r, layout.area, scale)?;
            buttons.iter().find(|(_, b)| inside(*b, at)).map(|(name, _)| (i, *name))
        });
        if let Some((i, name)) = hit {
            let scroll = self.catalog.scroll;
            self.browse(i as i64 - self.host.shelf_offset as i64);
            self.catalog.scroll = scroll;
            self.catalog_use(None, name == "queue");
            return;
        }
        match layout.cards.iter().find(|(_, r)| inside(*r, at)) {
            Some(&(i, _)) if i == self.host.shelf_offset => self.catalog_use(None, false),
            Some(&(i, _)) => {
                // Choosing by mouse leaves the scroll where it is.
                let scroll = self.catalog.scroll;
                self.browse(i as i64 - self.host.shelf_offset as i64);
                self.catalog.scroll = scroll;
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

/// Length and a sample of a long string: enough to see a data URI change.
fn quick_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.len().hash(&mut h);
    let b = s.as_bytes();
    for i in (0..b.len()).step_by((b.len() / 4096).max(1)) { b[i].hash(&mut h); }
    b[b.len().saturating_sub(256)..].hash(&mut h);
    h.finish()
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

fn inside(r: [f32; 4], p: [f32; 2]) -> bool { p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3] }

/// The mouse's Play and Queue buttons on a hovered card, at its foot
/// (Joep, 2026-11-25). None when that foot is scrolled out of the grid.
pub fn card_buttons(r: [f32; 4], area: [f32; 4], scale: f32) -> Option<[(&'static str, [f32; 4]); 2]> {
    let (m, gap) = (12.0 * scale, 8.0 * scale);
    let bh = (44.0 * scale).max(28.0);
    let bw = (r[2] - 2.0 * m - gap) / 2.0;
    let y = r[1] + r[3] - m - bh;
    if y < area[1] || y + bh > area[1] + area[3] { return None; }
    Some([("play", [r[0] + m, y, bw, bh]), ("queue", [r[0] + m + bw + gap, y, bw, bh])])
}

/// Where the cards and buttons go on a screen of `size`: the games in a
/// grid (scrolled by rows), the buttons under it.
pub fn catalog_layout(lobby: &Lobby, size: [u32; 2]) -> Layout {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let n = lobby.host.shelf_games().len();
    let g = grid(size, n);
    let mut cards = vec![];
    if n == 0 { return Layout { cards, area: g.area, buttons: vec![] }; }
    let width = g.cols as f32 * g.card[0] + (g.cols as f32 - 1.0) * g.gap;
    let x0 = g.area[0] + (g.area[2] - width) / 2.0;
    // Fewer rows than fit: centred in the area.
    let rows = n.div_ceil(g.cols);
    let height = rows as f32 * g.card[1] + (rows as f32 - 1.0) * g.gap;
    let y0 = g.area[1] + ((g.area[3] - height) / 2.0).max(0.0) - lobby.catalog.shown * (g.card[1] + g.gap);
    for i in 0..n {
        let (col, row) = ((i % g.cols) as f32, (i / g.cols) as f32);
        let r = [x0 + col * (g.card[0] + g.gap), y0 + row * (g.card[1] + g.gap), g.card[0], g.card[1]];
        // Down to the screen's bottom: the next row shows through the panel.
        if r[1] + r[3] > g.area[1] - g.gap && r[1] < h { cards.push((i, r)); }
    }
    let bh = (54.0 * crate::hud::ui(h)).max(34.0);
    let y = g.area[1] + g.area[3] + h * 0.1;
    let bw = bh * 3.6;
    let gap = bh * 0.4;
    let bx = w / 2.0 - (bw * 3.0 + gap * 2.0) / 2.0;
    let buttons = vec![("play", [bx, y, bw, bh]), ("queue", [bx + bw + gap, y, bw, bh]), ("close", [bx + 2.0 * (bw + gap), y, bw, bh])];
    Layout { cards, area: g.area, buttons }
}

use crate::paint::fill_uv;

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

/// At the game shelf, under its LB and RB: the covers of the games they
/// browse to, small and dimmed, like the catalog's neighbours.
pub fn shelf_sides(lobby: &Lobby, c: &mut Canvas, view_proj: glam::Mat4, size: [u32; 2]) {
    use crate::godot::rgba;
    if !lobby.bumpers_visible { return; }
    let shelf = &lobby.show.shelf;
    let n = shelf.len();
    if n < 2 { return; }
    let bx = ((game_room_sim::layout::BOOKCASE.0 + game_room_sim::layout::BOOKCASE.1) / 2.0) as f32;
    let z = game_room_sim::layout::BACK as f32 + 0.7;
    for side in [-1i64, 1] {
        let i = (lobby.host.shelf_offset as i64 + side).rem_euclid(n as i64) as usize;
        use crate::room::{SIDE_COVER_HALF as H, SIDE_COVER_X, SIDE_COVER_Y};
        let center = glam::Vec3::new(bx + side as f32 * SIDE_COVER_X, SIDE_COVER_Y, z);
        let (Some(a), Some(b)) = (crate::hud::to_screen(view_proj, size, center + glam::Vec3::new(-H[0], H[1], 0.0)),
            crate::hud::to_screen(view_proj, size, center + glam::Vec3::new(H[0], -H[1], 0.0))) else { continue };
        let r = [a[0], a[1], b[0] - a[0], b[1] - a[1]];
        if r[2] <= 2.0 || r[3] <= 2.0 { continue; }
        c.rect([r[0] - 2.0, r[1] - 2.0, r[2] + 4.0, r[3] + 4.0], rgba(0.0, 0.0, 0.0, 0.5));
        match lobby.catalog.covers.get(&shelf[i].id).and_then(|c| c.as_ref()) {
            Some(art) => { c.provide(art.image, &art.picture); c.image(r, art.image, fill_uv(art.size, r), rgba(0.75, 0.75, 0.75, 1.0)); }
            None => title_card(c, &shelf[i], r, r, 0.75, 1.0),
        }
    }
    c.over();
}

/// The open catalog over the whole screen.
pub fn draw(lobby: &Lobby, c: &mut Canvas, size: [u32; 2]) {
    use crate::godot::{rgba, Color};
    use crate::lobby::{DIM, TEXT, VIOLET};
    use render::hud::TextAlign;
    let (w, h) = (size[0] as f32, size[1] as f32);
    let scale = crate::hud::ui(h);
    let shelf = &lobby.show.shelf;
    if shelf.is_empty() { return; }
    let fade = lobby.catalog_fade();
    let middle = lobby.host.shelf_offset % shelf.len();
    let cover = |i: usize| shelf.get(i).and_then(|g| lobby.catalog.covers.get(&g.id)).and_then(|c| c.as_ref());
    let faces = crate::fonts::faces();
    c.over();
    // The middle game's cover, dark and big, behind everything.
    c.rect([0.0, 0.0, w, h], rgba(0.02, 0.02, 0.04, fade));
    // The middle game's colour as a soft glow from the top (its cover is
    // already the big card: not again behind it).
    let color = Color::from_string(&shelf[middle].color, VIOLET);
    // One colour per two pixel rows: smooth, no bands. The glow fades
    // out from the top, the bottom darkens for the title and buttons.
    let glow = color.darkened(0.55);
    let base = rgba(0.02, 0.02, 0.04, 1.0);
    let mut y = 0.0;
    while y < h {
        let t = y / h;
        let g = (1.0 - t / 0.75).max(0.0);
        let g = 0.45 * g * g;
        let dark = 1.0 - 0.55 * ((t - 0.5) / 0.5).clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| (a * (1.0 - g) + b * g) * dark;
        c.rect([0.0, y, w, 2.0], Color { r: mix(base.r, glow.r), g: mix(base.g, glow.g), b: mix(base.b, glow.b), a: fade });
        y += 2.0;
    }
    // An amber kicker, the size of the count opposite (DESIGN.md "Type").
    c.text(faces.bold, [48.0 * scale, 72.0 * scale], "ALL GAMES", TextAlign::Left, -1.0, (24.0 * scale).floor(), crate::lobby::AMBER.with_alpha(fade));
    // Only the keyboard here: say what a controller adds (Joep, 2026-11-25).
    let hidden = lobby.host.hidden_for_keyboard();
    let count = if hidden > 0 { format!("{} more with a controller   {} of {}", hidden, middle + 1, shelf.len()) } else { format!("{} of {}", middle + 1, shelf.len()) };
    let cs = (24.0 * scale).floor();
    let cw = render::hud::measure(&count, faces.regular, cs)[0];
    c.text(faces.regular, [w - 48.0 * scale - cw, 72.0 * scale], &count, TextAlign::Left, -1.0, cs, DIM.with_alpha(fade));

    let layout = catalog_layout(lobby, size);
    let games = lobby.host.shelf_games();
    let mouse = lobby.mouse.at;
    let mut opens: Vec<(usize, [f32; 4], [f32; 4])> = vec![];
    let queue = lobby.host.upcoming();
    let current = lobby.host.current_game().map(|g| (get(&g, "id", ""), g["phase"] == "paused"));
    // A little past the area, so the chosen card's frame is not cut off.
    let pad = 8.0 * scale;
    let area = [layout.area[0] - pad, layout.area[1] - pad, layout.area[2] + 2.0 * pad, h - layout.area[1] + pad];
    // All cards in one layer: they never overlap (the HUD has few layers).
    c.over();
    c.clip_to(Some(area));
    for &(i, r) in &layout.cards {
        let mid = i == middle;
        let hover = mouse.is_some_and(|m| inside(r, m) && inside(layout.area, m));
        c.style_box([r[0] - 4.0 * scale, r[1] + 8.0 * scale, r[2] + 8.0 * scale, r[3] + 4.0 * scale], [14.0 * scale; 4], rgba(0.0, 0.0, 0.0, 0.45 * fade), None);
        if mid || hover {
            let b = 5.0 * scale;
            c.style_box([r[0] - b, r[1] - b, r[2] + 2.0 * b, r[3] + 2.0 * b], [12.0 * scale; 4], if mid { TEXT } else { DIM }.with_alpha(fade), None);
        }
        // A game this party can't play yet: dimmed, its reason on it.
        let fits = games.get(i).is_none_or(|g| lobby.host.fits_party(g));
        // Too heavy for this device: a little dimmed, still playable.
        let smooth = games.get(i).is_none_or(|g| lobby.host.runs_smoothly(g));
        let k = (if mid { 1.0 } else if hover { 0.9 } else { 0.72 }) * if !fits { 0.4 } else if !smooth { 0.65 } else { 1.0 };
        let open = intersect(r, area);
        opens.push((i, r, open));
        match cover(i) {
            Some(art) => { c.provide(art.image, &art.picture); c.image(r, art.image, fill_uv(art.size, r), rgba(k, k, k, fade)); }
            None => title_card(c, &shelf[i], r, r, k, fade),
        }
    }
    c.clip_to(None);
    c.over();
    // Over the cards, each only where its card shows: a loading game's
    // progress, and a queued game's place in the queue.
    for &(i, r, open) in &opens {
        let game = &shelf[i].id;
        c.clip_to(Some(open));
        let k = 0.8;
        let loading = lobby.host.loading_shown(game);
        if let Some(percent) = loading {
            let bh = (34.0 * scale * k).floor();
            let band = [r[0], r[1] + r[3] - bh * 1.9, r[2], bh * 1.9];
            c.rect(band, rgba(0.03, 0.03, 0.06, 0.85 * fade));
            let label = lobby.host.loading_kicker(game);
            let fs = (bh * 0.5).floor();
            c.text(faces.bold, [open[0], band[1] + bh * 0.75], &label, TextAlign::Center, open[2], fs, crate::lobby::AMBER.with_alpha(fade));
            let bar = [r[0] + bh * 0.5, band[1] + bh * 1.15, r[2] - bh, bh * 0.28];
            crate::paint::progress(c, bar, percent / 100.0, fade);
        }
        let reason = games.get(i).map(|g| Some(lobby.host.shortfall(g)).filter(|r| !r.is_empty()).unwrap_or_else(|| lobby.host.hardware_note(g))).unwrap_or_default();
        if !reason.is_empty() {
            let fs = (22.0 * scale).floor();
            // At the top: a card in the last row shows only its top half
            // above the panel, and its reason must still read.
            let band = [r[0], r[1], r[2], fs * 2.4];
            c.rect(band, crate::room::PANEL_SOLID.with_alpha(0.85 * fade));
            c.text(faces.bold, [open[0], band[1] + band[3] / 2.0 + fs * 0.36], &reason, TextAlign::Center, open[2], fs, TEXT.with_alpha(fade));
        }
        // A quiet tag top right (DESIGN.md "Playlist"): the game on now gets
        // a pause or play sign, a queued game its place, "02" as in the list.
        let place = queue.iter().position(|e| &e.game == game);
        let on_now = current.as_ref().is_some_and(|(id, _)| id == game);
        if on_now || place.is_some() {
            let fs = (20.0 * scale).floor();
            let label = place.map(|p| format!("{:02}", p + 1)).unwrap_or_default();
            let tw = if on_now { fs * 1.6 } else { render::hud::measure(&label, faces.bold, fs)[0] + fs * 0.9 };
            let tag = [open[0] + open[2] - tw - 10.0 * scale, r[1] + 10.0 * scale, tw, fs * 1.5];
            c.style_box(tag, [8.0 * scale; 4], crate::room::PANEL_SOLID.with_alpha(0.78 * fade), None);
            if on_now {
                let paused = current.as_ref().is_some_and(|c| c.1);
                let center = [tag[0] + tag[2] / 2.0, tag[1] + tag[3] / 2.0];
                let u = fs * 0.3;
                let ink = crate::lobby::AMBER.with_alpha(fade);
                if paused {
                    c.rect([center[0] - u * 0.85, center[1] - u, u * 0.6, u * 2.0], ink);
                    c.rect([center[0] + u * 0.25, center[1] - u, u * 0.6, u * 2.0], ink);
                } else {
                    c.polygon(&[[center[0] - u * 0.7, center[1] - u], [center[0] + u * 1.0, center[1]], [center[0] - u * 0.7, center[1] + u]], ink);
                }
            } else {
                c.text(faces.bold, [tag[0], tag[1] + tag[3] / 2.0 + fs * 0.36], &label, TextAlign::Center, tag[2], fs, TEXT.with_alpha(0.85 * fade));
            }
        }
    }
    c.clip_to(None);
    // The mouse's Play and Queue on the hovered card (DESIGN.md "Button").
    if let Some(m) = mouse {
        if let Some(&(_, r, _)) = opens.iter().find(|(i, r, _)| inside(*r, m) && inside(layout.area, m) && games.get(*i).is_some_and(|g| lobby.host.fits_party(g))) {
            if let Some(buttons) = card_buttons(r, layout.area, scale) {
                c.over();
                for (name, b) in buttons {
                    let on = inside(b, m);
                    let fill = if name == "play" { crate::room::VIOLET_DEEP } else { crate::room::PANEL_SOLID };
                    let fill = if on { fill.lightened(0.15) } else { fill };
                    c.style_box(b, [12.0 * scale; 4], fill.with_alpha(fade), Some(((1.0 * scale).max(1.0), crate::room::LINE.with_alpha(0.1 * fade))));
                    let label = if name == "play" { "Play" } else { "Queue" };
                    let fs = (b[3] * 0.42).floor();
                    c.text(faces.bold, [b[0], b[1] + b[3] / 2.0 + fs * 0.36], label, TextAlign::Center, b[2], fs, TEXT.with_alpha(fade));
                }
            }
        }
    }
    // The panel under the grid: see-through at its top edge, so the games
    // further down show; darker towards the buttons.
    c.over();
    let top = layout.area[1] + layout.area[3] + 4.0 * scale;
    let mut y = top;
    while y < h {
        let t = ((y - top) / (60.0 * scale)).clamp(0.0, 1.0);
        c.rect([0.0, y, w, 2.0], rgba(0.02, 0.02, 0.04, (0.72 + 0.24 * t) * fade));
        y += 2.0;
    }
    let station = lobby.host.station_at((game_room_sim::layout::GAME_SHELF.0 + game_room_sim::layout::GAME_SHELF.1) / 2.0, None);
    let card_bottom = layout.area[1] + layout.area[3];
    let title = if shelf[middle].title.is_empty() { "Game" } else { &shelf[middle].title };
    c.text(faces.bold, [0.0, card_bottom + 58.0 * scale], title, TextAlign::Center, w, (44.0 * scale).floor(), TEXT.with_alpha(fade));
    if let Some(s) = station.as_ref().filter(|s| !s.enabled) {
        c.text(faces.regular, [0.0, card_bottom + 92.0 * scale], &s.label, TextAlign::Center, w, (22.0 * scale).floor(), crate::lobby::AMBER.with_alpha(fade));
    }
    let enabled = station.as_ref().is_some_and(|s| s.enabled);
    for &(name, r) in &layout.buttons {
        let hover = mouse.is_some_and(|m| inside(r, m));
        let (label, key, accent) = match name { "play" => ("Play now", "A", crate::lobby::GREEN), "queue" => ("Queue", "X", crate::room::PAD_X), _ => ("Room", "B", crate::room::PAD_B) };
        let live = enabled || name == "close";
        // Only the button is coloured: a disc with its letter, then the
        // label, centred together. No pill; a mouse over it gets a faint one.
        if hover && live { c.style_box(r, [r[3] / 2.0; 4], rgba(1.0, 1.0, 1.0, 0.08 * fade), None); }
        let fs = (r[3] * 0.42).floor();
        let radius = r[3] * 0.32;
        let gap = r[3] * 0.25;
        let tw = render::hud::measure(label, faces.bold, fs)[0];
        // The room is also R3 (as it opened): B / R3.
        let slash = if name == "close" { render::hud::measure(" / ", faces.bold, fs)[0] } else { 0.0 };
        let extra = if name == "close" { slash + 2.0 * radius } else { 0.0 };
        let x = r[0] + (r[2] - (2.0 * radius + extra + gap + tw)) / 2.0;
        let disc = [x + radius, r[1] + r[3] / 2.0];
        c.circle(disc, radius, accent.with_alpha(if live { fade } else { 0.35 * fade }));
        c.text(faces.bold, [disc[0] - radius, disc[1] + fs * 0.33], key, TextAlign::Center, 2.0 * radius, (fs * 0.9).floor(), Color::hex("0d1020").with_alpha(fade));
        if name == "close" {
            c.text(faces.bold, [x + 2.0 * radius, disc[1] + fs * 0.36], " / ", TextAlign::Left, -1.0, fs, DIM.with_alpha(fade));
            crate::paint::stick(c, [x + 3.0 * radius + slash, disc[1]], radius, "R3", true);
        }
        c.text(faces.bold, [x + 2.0 * radius + extra + gap, r[1] + r[3] / 2.0 + fs * 0.36], label, TextAlign::Left, -1.0, fs, TEXT.with_alpha(if live { fade } else { 0.4 * fade }));
    }
    // The queue (DESIGN.md "Playlist"), bottom right: green UP NEXT,
    // numbered rows with a small cover, "+N more" muted. Clear of the
    // buttons: where it would reach them (screens narrower than 16:9) it
    // shrinks to one line under them.
    if !queue.is_empty() {
        let fs = (19.0 * scale).floor();
        let line = fs * 1.9;
        let margin = 32.0 * scale;
        let pad = 20.0 * scale;
        let buttons_right = layout.buttons.iter().map(|(_, r)| r[0] + r[2]).fold(0.0, f32::max);
        let buttons_bottom = layout.buttons.iter().map(|(_, r)| r[1] + r[3]).fold(0.0, f32::max);
        let shown = queue.len().min(4);
        let title_of = |e: &crate::host::Entry| match lobby.host.loading_shown(&e.game) { Some(_) => format!("{}  {}%", e.title, lobby.host.loading_percent(&e.game)), None => e.title.clone() };
        let thumb = [line * 0.62, line * 0.82];
        let rows_w = queue.iter().take(shown).map(|e| render::hud::measure(&title_of(e), faces.regular, fs)[0]).fold(160.0 * scale, f32::max);
        let qw = pad * 2.0 + fs * 1.6 + thumb[0] + fs * 0.6 + rows_w;
        let full_h = pad * 2.0 + fs * 1.2 + line * shown as f32 + if queue.len() > shown { fs * 1.4 } else { 0.0 };
        let compact = w - qw - margin < buttons_right + margin;
        c.over();
        if compact {
            let first = &queue[0];
            let more = if queue.len() > 1 { format!("  +{}", queue.len() - 1) } else { String::new() };
            let kicker = "UP NEXT  ";
            let ks = (fs * 0.75).floor();
            let tw = render::hud::measure(kicker, faces.bold, ks)[0] + render::hud::measure(&title_of(first), faces.regular, fs)[0] + render::hud::measure(&more, faces.regular, fs)[0];
            let (qw, qh) = (tw + pad * 2.0, fs * 2.4);
            let (qx, qy) = (w - qw - margin, (h - qh - margin).max(buttons_bottom + 16.0 * scale));
            crate::paint::panel_with(c, [qx, qy, qw, qh], scale, Color { a: crate::room::PANEL.a * fade, ..crate::room::PANEL }, crate::room::LINE.with_alpha(0.1 * fade));
            let y = qy + qh / 2.0 + fs * 0.36;
            c.text(faces.bold, [qx + pad, y], kicker, TextAlign::Left, -1.0, ks, crate::lobby::GREEN.with_alpha(fade));
            let x = qx + pad + render::hud::measure(kicker, faces.bold, ks)[0];
            c.text(faces.regular, [x, y], &title_of(first), TextAlign::Left, -1.0, fs, TEXT.with_alpha(fade));
            let x = x + render::hud::measure(&title_of(first), faces.regular, fs)[0];
            c.text(faces.regular, [x, y], &more, TextAlign::Left, -1.0, fs, DIM.with_alpha(fade));
        } else {
            let (qx, qy) = (w - qw - margin, h - full_h - margin);
            crate::paint::panel_with(c, [qx, qy, qw, full_h], scale, Color { a: crate::room::PANEL.a * fade, ..crate::room::PANEL }, crate::room::LINE.with_alpha(0.1 * fade));
            c.text(faces.bold, [qx + pad, qy + pad + fs * 0.8], "UP NEXT", TextAlign::Left, -1.0, (fs * 0.75).floor(), crate::lobby::GREEN.with_alpha(fade));
            for (i, e) in queue.iter().take(shown).enumerate() {
                let top = qy + pad + fs * 1.2 + line * i as f32;
                let mid = top + line / 2.0;
                c.text(faces.regular, [qx + pad, mid + fs * 0.3], &format!("{:02}", i + 1), TextAlign::Left, -1.0, (fs * 0.8).floor(), DIM.with_alpha(fade));
                let tr = [qx + pad + fs * 1.6, mid - thumb[1] / 2.0, thumb[0], thumb[1]];
                match lobby.catalog.covers.get(&e.game).and_then(|c| c.as_ref()) {
                    Some(art) => { c.provide(art.image, &art.picture); c.image(tr, art.image, fill_uv(art.size, tr), rgba(1.0, 1.0, 1.0, fade)); }
                    None => {
                        let color = shelf.iter().find(|g| g.id == e.game).map_or(VIOLET, |g| Color::from_string(&g.color, VIOLET));
                        c.rect(tr, color.darkened(0.4).with_alpha(fade));
                    }
                }
                let loading = lobby.host.loading_shown(&e.game).is_some();
                let x = tr[0] + tr[2] + fs * 0.6;
                c.text(faces.regular, [x, mid + fs * 0.36], &e.title, TextAlign::Left, -1.0, fs, TEXT.with_alpha(fade));
                if loading {
                    let x = x + render::hud::measure(&format!("{}  ", e.title), faces.regular, fs)[0];
                    c.text(faces.regular, [x, mid + fs * 0.36], &format!("{}%", lobby.host.loading_percent(&e.game)), TextAlign::Left, -1.0, fs, crate::lobby::AMBER.with_alpha(fade));
                }
            }
            if queue.len() > shown {
                c.text(faces.regular, [qx + pad + fs * 1.6, qy + full_h - pad - fs * 0.2], &format!("+{} more", queue.len() - shown), TextAlign::Left, -1.0, (fs * 0.85).floor(), DIM.with_alpha(fade));
            }
        }
    }
}
