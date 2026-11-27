//! The painted screens: room.gd's and main.gd's canvas painters (logo, TV,
//! up next, jukebox label, plaque, shelf box, bumpers, prompts) drawn into
//! the room's screen atlas with the HUD renderer.
//!
//! `Canvas` speaks Godot's CanvasItem: text placed by its baseline, widths
//! for alignment, outlines. Painters read `Show`, what the lobby wants on
//! its screens; the lobby logic fills it from the party state.

use crate::fonts::faces;
use crate::godot::Color;
use crate::room::{AMBER, GREEN, NIGHT_LILAC, PINK, VIOLET};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use std::f32::consts::{PI, TAU};

use crate::room::{BG, DIM, TEXT};

/// A game as the screens show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Game {
    pub id: String,
    pub title: String,
    /// The store colour, "#rrggbb"; title cards fall back to violet.
    pub color: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

/// The TV's top-left capsule and the line under the title.
#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    pub kicker: String,
    pub color: Color,
    pub detail: String,
    /// Loading progress, 0 to 100, while the game is on its way.
    pub loading: Option<f32>,
}

/// A game's decoded cover: the image id it is drawn as, and its size.
#[derive(Clone)]
pub struct Cover { pub image: render::hud::HudImage, pub picture: std::sync::Arc<render::picture::Picture>, pub size: [u32; 2] }

impl std::fmt::Debug for Cover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "Cover({:?}, {:?})", self.image, self.size) }
}

/// The part of a `size` picture that fills `r` (centre crop).
pub fn fill_uv(size: [u32; 2], r: [f32; 4]) -> [f32; 4] {
    let ia = size[0] as f32 / size[1].max(1) as f32;
    let ra = r[2] / r[3].max(1.0);
    if ia > ra { let uw = ra / ia; [(1.0 - uw) / 2.0, 0.0, uw, 1.0] } else { let vh = ia / ra; [0.0, (1.0 - vh) / 2.0, 1.0, vh] }
}

/// Everything the painted screens show.
#[derive(Clone, Debug, Default)]
pub struct Show {
    pub time: f32,
    /// What the TV shows: the running game, or the first queued one.
    pub tv: Option<(Game, Banner)>,
    /// What "press Y" names: Y, or the key when everyone plays on the
    /// keyboard ("" reads as Y).
    pub y_key: String,
    /// The Up next cabinet's entries (after the TV's game).
    pub queue: Vec<Game>,
    /// The cabinet's first game is loading or broken: its status line.
    pub queue_status: Option<(String, bool)>,
    pub queue_loading: Option<f32>,
    /// Something is queued for the TV, so the cabinet says "Nothing after this".
    pub tv_has_next: bool,
    /// Numbers before the cabinet's first title (1 when the TV shows one).
    pub queue_first: usize,
    pub shelf: Vec<Game>,
    pub shelf_offset: usize,
    pub humans: usize,
    pub profile_colors: Vec<String>,
    pub now_playing: Option<Track>,
    pub room_code: String,
    /// The online switch is on Public.
    pub public: bool,
    /// The room QR code's dark modules, 0..1 (qr.rs).
    pub qr: Vec<[f32; 4]>,
    pub bumper_presses: [f32; 2],
    /// Covers by game id (the ones decoded so far).
    pub covers: std::collections::HashMap<String, Cover>,
    /// Gameplay screenshots by game id, for the TV.
    pub shots: std::collections::HashMap<String, Cover>,
    /// A bug just reported at the table (Back + Y): where to describe it.
    pub bug: Option<BugNote>,
    /// The visit-a-room screen (visit.rs).
    pub visit: VisitShow,
}

/// What the visit-a-room screen over the QR code shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisitShow {
    /// Taking the code: the next slot is lit.
    pub open: bool,
    /// The code typed so far.
    pub code: String,
    /// A pad spelling it: the letter its stick is on.
    pub pick: Option<char>,
    /// The mouse is over it.
    pub hover: bool,
}

/// The bug panel (hud.rs `bug_panel`): a bug reported at the table and
/// the QR code of its describe page, for whoever has no linked phone.
#[derive(Clone, Debug, Default)]
pub struct BugNote {
    /// The amber line above it: "BUG REPORTED" when empty, "BETA FEEDBACK"
    /// after an open beta.
    pub kicker: String,
    /// "What went wrong in Dead Miles?"
    pub title: String,
    /// What to do: scan, the app, or tap the screen.
    pub line: String,
    /// The describe link, short enough to type ("splitscreen.club/b/…").
    pub link: String,
    /// Its QR code's dark modules, 0..1 (qr.rs).
    pub qr: Vec<[f32; 4]>,
    /// Lobby time it came; it goes after [`BUG_SECONDS`].
    pub since: f32,
}

/// How long the bug panel stays in the room.
pub const BUG_SECONDS: f32 = 180.0;

/// Godot's draw calls on one screen's rectangle in an atlas draw list.
/// Painters draw in the screen's own pixels (`size`); `k` atlas pixels
/// stand for each, so a screen shown big on a 4K TV stays sharp.
pub struct Canvas<'a> {
    list: &'a mut HudDrawList,
    origin: [f32; 2],
    pub size: [f32; 2],
    k: f32,
    /// Drawing stays inside this (screen pixels), when set.
    clip_to: Option<[f32; 4]>,
}

fn srgb(c: Color) -> [f32; 4] { [c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0), c.a.clamp(0.0, 1.0)] }

impl<'a> Canvas<'a> {
    pub fn new(list: &'a mut HudDrawList, rect: [u32; 4]) -> Self { Self::scaled(list, rect, 1.0) }

    /// A screen painted at `k` atlas pixels per screen pixel into `rect`.
    pub fn scaled(list: &'a mut HudDrawList, rect: [u32; 4], k: f32) -> Self {
        Self { list, origin: [rect[0] as f32, rect[1] as f32], size: [rect[2] as f32 / k, rect[3] as f32 / k], k, clip_to: None }
    }

    fn clip(&self) -> [f32; 4] {
        let full = [self.origin[0], self.origin[1], self.size[0] * self.k, self.size[1] * self.k];
        let Some(r) = self.clip_to else { return full };
        let r = [self.origin[0] + r[0] * self.k, self.origin[1] + r[1] * self.k, r[2] * self.k, r[3] * self.k];
        let (x0, y0) = (full[0].max(r[0]), full[1].max(r[1]));
        let (x1, y1) = ((full[0] + full[2]).min(r[0] + r[2]), (full[1] + full[3]).min(r[1] + r[3]));
        [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
    }

    /// Keep later drawing inside `r` (None: the whole canvas again).
    pub fn clip_to(&mut self, r: Option<[f32; 4]>) { self.clip_to = r; }
    /// Draw `dx` further right (a prompt centred in a wider screen).
    pub fn shift(&mut self, dx: f32) { self.origin[0] += dx * self.k; self.size[0] -= dx; }

    fn at(&self, p: [f32; 2]) -> [f32; 2] { [self.origin[0] + p[0] * self.k, self.origin[1] + p[1] * self.k] }

    /// Later shapes go over the text drawn so far (text otherwise draws last).
    pub fn over(&mut self) { self.list.begin_overlay(); }

    /// A picture (handed to the list with `provide`) over `r`, `uv` its part.
    pub fn image(&mut self, r: [f32; 4], id: render::hud::HudImage, uv: [f32; 4], tint: Color) {
        let p = self.at([r[0], r[1]]);
        let clip = self.clip();
        self.list.image([p[0], p[1], r[2] * self.k, r[3] * self.k], id, uv, srgb(tint), clip);
    }

    pub fn provide(&mut self, id: render::hud::HudImage, picture: &std::sync::Arc<render::picture::Picture>) {
        self.list.provide_image(id, picture, render::hud::ImageFilter::Linear);
    }

    pub fn rect(&mut self, r: [f32; 4], c: Color) {
        let p = self.at([r[0], r[1]]);
        let clip = self.clip();
        self.list.rect([p[0], p[1], r[2] * self.k, r[3] * self.k], srgb(c), clip);
    }

    /// A one-pixel line, as Godot's draw_line with its default width.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], c: Color, width: f32) {
        let clip = self.clip();
        let (a, b) = (self.at(a), self.at(b));
        self.list.line(a, b, width * self.k, srgb(c), clip);
    }

    pub fn circle(&mut self, center: [f32; 2], radius: f32, c: Color) {
        let clip = self.clip();
        let center = self.at(center);
        self.list.circle(center, radius * self.k, srgb(c), clip);
    }

    pub fn arc(&mut self, center: [f32; 2], radius: f32, from: f32, to: f32, width: f32, c: Color) {
        let clip = self.clip();
        let center = self.at(center);
        self.list.arc(center, radius * self.k, from, to, width * self.k, srgb(c), clip);
    }

    pub fn polygon(&mut self, points: &[[f32; 2]], c: Color) {
        let clip = self.clip();
        let points: Vec<[f32; 2]> = points.iter().map(|&p| self.at(p)).collect();
        self.list.polygon(&points, srgb(c), clip);
    }

    /// StyleBoxFlat: rounded corners (top-left, top-right, bottom-right,
    /// bottom-left) and an inner border.
    pub fn style_box(&mut self, r: [f32; 4], radii: [f32; 4], bg: Color, border: Option<(f32, Color)>) {
        if let Some((width, color)) = border {
            self.polygon(&rounded(r, radii), color);
            let inner = [r[0] + width, r[1] + width, r[2] - 2.0 * width, r[3] - 2.0 * width];
            self.polygon(&rounded(inner, radii.map(|k| (k - width).max(0.0))), bg);
        } else {
            self.polygon(&rounded(r, radii), bg);
        }
    }

    /// Godot's draw_string: `pos` is the left end of the baseline; a width
    /// aligns within [x, x + width].
    pub fn text(&mut self, face: FontFace, pos: [f32; 2], text: &str, align: TextAlign, width: f32, size: f32, c: Color) {
        self.text_at(face, pos, text, align, width, size, c, [0.0, 0.0]);
    }

    /// Godot's draw_string_outline: a ring of copies `outline` wide.
    pub fn text_outline(&mut self, face: FontFace, pos: [f32; 2], text: &str, align: TextAlign, width: f32, size: f32, outline: f32, c: Color) {
        let r = outline * 0.5;
        let rings: &[f32] = if r > 6.0 { &[1.0, 0.5] } else { &[1.0] };
        let copies = 16 * rings.len();
        // Copies overlap near the glyphs; each is fainter so the stack
        // lands near Godot's single stroke.
        let a = 1.0 - (1.0 - c.a.clamp(0.0, 1.0)).powf(2.0 / copies as f32);
        for &k in rings {
            for i in 0..16 {
                let t = i as f32 / 16.0 * TAU;
                self.text_at(face, pos, text, align, width, size, c.with_alpha(a), [t.cos() * r * k, t.sin() * r * k]);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_at(&mut self, face: FontFace, pos: [f32; 2], text: &str, align: TextAlign, width: f32, size: f32, c: Color, shift: [f32; 2]) {
        if text.is_empty() || size <= 0.0 { return; }
        let width = if width > 0.0 { width } else { render::hud::measure(text, face, size)[0] + 4.0 };
        let top = pos[1] - render::hud::baseline(face, size);
        let p = self.at([pos[0] + shift[0], top + shift[1]]);
        let id = self.list.text.len() as u64 + 1;
        let to8 = |v: f32| (v * 255.0).round() as u8;
        let s = srgb(c);
        self.list.text.push(TextRun { id, text: text.into(), position: p, bounds: self.clip(), font_size: size * self.k,
            color: [to8(s[0]), to8(s[1]), to8(s[2]), to8(s[3])], font: face, align, width: width * self.k });
    }
}

/// The outline of a rounded rectangle, corners as arcs.
fn rounded(r: [f32; 4], radii: [f32; 4]) -> Vec<[f32; 2]> {
    let [x, y, w, h] = r;
    let limit = w.min(h) * 0.5;
    let corners = [([x, y], PI, 0), ([x + w, y], 1.5 * PI, 1), ([x + w, y + h], 0.0, 2), ([x, y + h], 0.5 * PI, 3)];
    let mut points = vec![];
    for (corner, start, k) in corners {
        let rad = radii[k].clamp(0.0, limit);
        let sx = if corner[0] > x { -1.0 } else { 1.0 };
        let sy = if corner[1] > y { -1.0 } else { 1.0 };
        let c = [corner[0] + sx * rad, corner[1] + sy * rad];
        let steps = if rad > 0.0 { 8 } else { 0 };
        for i in 0..=steps {
            let a = start + 0.5 * PI * i as f32 / steps.max(1) as f32;
            points.push([c[0] + a.cos() * rad, c[1] + a.sin() * rad]);
        }
    }
    points
}

fn regular() -> FontFace { faces().regular }
fn bold() -> FontFace { faces().bold }

/// main.gd's `_text`: headings (40 px and up) use the heavy weight.
fn text(c: &mut Canvas, value: &str, pos: [f32; 2], size: f32, color: Color, width: f32, align: TextAlign) {
    c.text(if size >= 40.0 { bold() } else { regular() }, pos, value, align, width, size, color);
}

fn left(c: &mut Canvas, value: &str, pos: [f32; 2], size: f32, color: Color) { text(c, value, pos, size, color, -1.0, TextAlign::Left); }

/// main.gd's `_fit`: shrink by 2 until the value fits, down to `minimum`.
fn fit(value: &str, mut size: f32, width: f32, minimum: f32) -> f32 {
    let face = |s: f32| if s >= 40.0 { bold() } else { regular() };
    while size > minimum && render::hud::measure(value, face(size), size)[0] > width { size -= 2.0; }
    size
}

/// main.gd's `_short`: at most `length` characters, with an ellipsis.
fn short(value: &str, length: usize) -> String {
    if value.chars().count() <= length { value.into() } else { value.chars().take(length - 1).collect::<String>() + "…" }
}

/// The Splitscreen Club wordmark as a neon sign (room.gd `_logo`).
pub fn logo(c: &mut Canvas) {
    let wordmark = faces().wordmark;
    // One line, written like the domain: "Splitscreen" amber, ".Club" lilac (Joep).
    let words = [("Splitscreen", AMBER), (".Club", NIGHT_LILAC)];
    // Narrow enough to clear the wall shelves on both sides (Joep).
    let mut size = 140.0;
    let widths = |size: f32| words.map(|w| render::hud::measure(w.0, wordmark, size)[0]);
    let mut w = widths(size);
    let mut total = w[0] + w[1];
    if total > 920.0 {
        size = (size * 920.0 / total).floor();
        w = widths(size);
        total = w[0] + w[1];
    }
    let mut x = (1280.0 - total) / 2.0;
    for (i, (word, color)) in words.into_iter().enumerate() {
        let at = [x, 160.0 + size * 0.36];
        for k in [30.0, 18.0, 10.0] { c.text_outline(wordmark, at, word, TextAlign::Left, -1.0, size, k, color.with_alpha(0.2)); }
        c.text_outline(wordmark, at, word, TextAlign::Left, -1.0, size, 4.0, color.lightened(0.2));
        c.text(wordmark, at, word, TextAlign::Left, -1.0, size, color);
        x += w[i];
    }
}

/// The game's cover over `r` when it has one, else its title card.
pub fn art(c: &mut Canvas, show: &Show, game: &Game, r: [f32; 4], size: f32) {
    match show.covers.get(&game.id) {
        Some(art) => {
            // A tall cover on a wide screen: keep its upper part, where the
            // title usually is, rather than the middle.
            let mut uv = fill_uv(art.size, r);
            uv[1] *= 0.35;
            c.provide(art.image, &art.picture);
            c.image(r, art.image, uv, Color::WHITE);
        }
        None => title_card(c, game, r, size),
    }
}

/// A game without artwork: its colour as a gradient, the title in caps.
pub fn title_card(c: &mut Canvas, game: &Game, r: [f32; 4], size: f32) {
    let color = Color::from_string(&game.color, VIOLET);
    for y in 0..r[3] as i32 {
        c.rect([r[0], r[1] + y as f32, r[2], 1.0], color.darkened(0.35 + 0.45 * y as f32 / r[3]));
    }
    let title = if game.title.is_empty() { "Game" } else { &game.title };
    let words: Vec<&str> = title.split(' ').collect();
    let line = size * 1.1;
    let top = r[1] + r[3] / 2.0 - (words.len() as f32 - 1.0) * line / 2.0 + size * 0.35;
    for (i, word) in words.iter().enumerate() {
        text(c, &word.to_uppercase(), [r[0], top + i as f32 * line], size, TEXT, r[2], TextAlign::Center);
    }
}

fn scanlines(c: &mut Canvas) {
    c.over();
    let mut y = 0.0;
    while y < c.size[1] {
        c.rect([0.0, y - 0.5, c.size[0], 1.0], Color::BLACK.with_alpha(0.12));
        y += 4.0;
    }
}

fn loading_bar(c: &mut Canvas, r: [f32; 4], percent: Option<f32>) {
    let Some(p) = percent else { return };
    progress(c, r, p / 100.0, 1.0);
}

/// The one progress bar (DESIGN.md "Progress"): a round track, white at
/// 8 %, filled amber; the same in the HTML loader, on cards and on the TV.
pub fn progress(c: &mut Canvas, r: [f32; 4], fraction: f32, alpha: f32) {
    let radius = r[3] / 2.0;
    c.style_box(r, [radius; 4], Color::WHITE.with_alpha(0.08 * alpha), None);
    let fill = r[2] * fraction.clamp(0.0, 1.0);
    if fill > 0.5 { c.style_box([r[0], r[1], fill.max(r[3]), r[3]], [radius; 4], AMBER.with_alpha(alpha), None); }
}

/// main.gd `_draw_tv`.
pub fn tv(c: &mut Canvas, show: &Show) {
    let [w, h] = c.size;
    c.rect([0.0, 0.0, w, h], Color::hex("07060d"));
    let Some((game, banner)) = &show.tv else { return idle_tv(c, show) };
    // A screenshot of the game (its current state once played), never
    // the cover: a tall poster cropped wide looks wrong on a TV.
    match show.shots.get(&game.id) {
        Some(shot) => { c.provide(shot.image, &shot.picture); c.image([0.0, 0.0, w, h], shot.image, fill_uv(shot.size, [0.0, 0.0, w, h]), Color::WHITE); }
        None => title_card(c, game, [0.0, 0.0, w, h], 64.0),
    }
    c.over();
    c.rect([0.0, h - 120.0, w, 120.0], crate::godot::rgba(0.02, 0.02, 0.05, 0.78));
    left(c, if game.title.is_empty() { "Game" } else { &game.title }, [36.0, h - 58.0], 48.0, TEXT);
    left(c, &banner.detail, [36.0, h - 22.0], 22.0, DIM);
    let kw = render::hud::measure(&banner.kicker, regular(), 32.0)[0] + 40.0;
    c.rect([28.0, 26.0, kw, 52.0], crate::godot::rgba(0.02, 0.02, 0.05, 0.85));
    c.rect([28.0, 26.0, 6.0, 52.0], banner.color);
    left(c, &banner.kicker, [48.0, 63.0], 32.0, banner.color);
    let mut x = w - 40.0;
    for color in &show.profile_colors {
        c.circle([x, h - 60.0], 14.0, Color::from_string(color, VIOLET));
        x -= 36.0;
    }
    loading_bar(c, [28.0, 88.0, w - 56.0, 12.0], banner.loading);
    scanlines(c);
}

fn idle_tv(c: &mut Canvas, show: &Show) {
    let [w, h] = c.size;
    for i in 0..14 {
        let hue = (show.time as f64 * 0.05 + i as f64 * 0.05).rem_euclid(1.0) as f32;
        c.rect([0.0, i as f32 * h / 14.0, w, h / 14.0 + 1.0], Color::from_hsv(hue, 0.7, 0.35));
    }
    text(c, "NOTHING QUEUED", [0.0, h / 2.0 - 10.0], 64.0, TEXT, w, TextAlign::Center);
    let y = if show.y_key.is_empty() { "Y" } else { show.y_key.as_str() };
    text(c, &format!("Walk to the game shelf and press {y} to add a game"), [0.0, h / 2.0 + 48.0], 24.0, TEXT.darkened(0.15), w, TextAlign::Center);
    scanlines(c);
}

/// main.gd `_draw_front_box`: the featured box on the shelf stand.
pub fn box_front(c: &mut Canvas, show: &Show) {
    let [w, h] = c.size;
    if show.shelf.is_empty() {
        c.rect([0.0, 0.0, w, h], Color::hex("0c0a16"));
        text(c, "NO GAMES", [0.0, h / 2.0], 32.0, DIM, w, TextAlign::Center);
        return;
    }
    let game = &show.shelf[show.shelf_offset % show.shelf.len()];
    art(c, show, game, [0.0, 0.0, w, h], if w > 220.0 { 32.0 } else { 16.0 });
}

/// main.gd `_draw_queue_preview`: the Up next cabinet.
pub fn queue_preview(c: &mut Canvas, show: &Show) {
    let [w, _] = c.size;
    c.rect([0.0, 0.0, c.size[0], c.size[1]], BG);
    left(c, "UP NEXT", [24.0, 46.0], 34.0, GREEN);
    let Some(game) = show.queue.first() else {
        let (a, b) = if show.tv_has_next { ("Nothing after this".to_string(), "Queue more at the game shelf".to_string()) }
            else if !show.shelf.is_empty() { ("Choose a game".into(), "at the game shelf".into()) }
            else { ("No matching games".into(), format!("{} players in this party", show.humans)) };
        left(c, &a, [24.0, 206.0], 44.0, TEXT);
        left(c, &b, [24.0, 256.0], 32.0, DIM);
        return;
    };
    // The cover at its own shape (3:4) on the left, big; the title, the
    // status and the loading bar beside it.
    let h = c.size[1];
    let art_r = [24.0, 66.0, (h - 84.0) * 0.75, h - 84.0];
    art(c, show, game, art_r, 32.0);
    let x = art_r[0] + art_r[2] + 24.0;
    let tw = w - x - 24.0;
    let title = short(&game.title, 32);
    let words: Vec<&str> = title.split(' ').collect();
    // Up to two lines, as big as fit.
    let lines: Vec<String> = if words.len() > 1 && fit(&title, 46.0, tw, 30.0) < 40.0 {
        let half = words.len().div_ceil(2);
        vec![words[..half].join(" "), words[half..].join(" ")]
    } else { vec![title.clone()] };
    let size = lines.iter().map(|l| fit(l, 46.0, tw, 26.0)).fold(46.0, f32::min);
    // Its number, as in the list under it and the catalog (DESIGN.md "Playlist").
    left(c, &format!("{:02}", show.queue_first + 1), [x, 98.0], 28.0, DIM);
    for (i, line) in lines.iter().enumerate() { left(c, line, [x, 140.0 + i as f32 * size * 1.15], size, TEXT); }
    let below = 140.0 + lines.len() as f32 * size * 1.15;
    if let Some((status, broken)) = &show.queue_status {
        left(c, status, [x, below + 12.0], fit(status, 24.0, tw, 17.0), if *broken { PINK } else { AMBER });
    }
    loading_bar(c, [x, below + 34.0, tw, 12.0], show.queue_loading);
}

/// main.gd `_draw_queue_titles`: the numbered list under the cabinet.
pub fn queue_titles(c: &mut Canvas, show: &Show) {
    let [w, _] = c.size;
    c.rect([0.0, 0.0, c.size[0], c.size[1]], BG);
    let rows: Vec<&Game> = show.queue.iter().skip(1).take(crate::room::QUEUE_TITLE_ROWS).collect();
    for (i, game) in rows.iter().enumerate() {
        let y = 53.0 + i as f32 * 55.0;
        left(c, &format!("{:02}", show.queue_first + i + 2), [22.0, y], 32.0, DIM);
        // A small 3:4 cover, then the title (DESIGN.md "Playlist").
        art(c, show, game, [88.0, y - 30.0, 27.0, 36.0], 8.0);
        let title = short(&game.title, 32);
        left(c, &title, [130.0, y], fit(&title, 40.0, w - 150.0, 28.0), TEXT);
        c.line([84.0, y + 14.0], [w - 24.0, y + 14.0], Color::WHITE.with_alpha(0.08), 1.0);
    }
    let more = show.queue.len().saturating_sub(1 + rows.len());
    if more > 0 { left(c, &format!("+{more} more"), [130.0, 284.0], 30.0, DIM); }
}

/// main.gd `_draw_jukebox`: dark lettering on the glowing dome.
pub fn jukebox(c: &mut Canvas, show: &Show) {
    let [w, _] = c.size;
    let ink = Color::hex("4a1428");
    let Some(track) = &show.now_playing else {
        text(c, "JUKEBOX", [0.0, 96.0], fit("JUKEBOX", 48.0, w - 40.0, 14.0), ink, w, TextAlign::Center);
        return;
    };
    let title = short(&track.title, 28);
    let artist = short(&track.artist, 32);
    text(c, if track.playing { "NOW PLAYING" } else { "PAUSED" }, [0.0, 44.0], 18.0, ink.with_alpha(0.7), w, TextAlign::Center);
    text(c, &title, [0.0, 94.0], fit(&title, 44.0, w - 40.0, 22.0), ink, w, TextAlign::Center);
    text(c, &artist, [0.0, 128.0], fit(&artist, 24.0, w - 80.0, 16.0), Color::hex("8a2e3c"), w, TextAlign::Center);
}

/// main.gd `_draw_plaque`: the room's QR code and join code.
pub fn plaque(c: &mut Canvas, show: &Show) {
    let [w, h] = c.size;
    c.rect([0.0, 0.0, w, h], Color::hex("0c0a16"));
    // As big as the plaque allows: phones read it from the couch, at an angle.
    // Even margins above the square and below the code, with a clear gap
    // between them (Joep, 22 November 2026: the code ran into the square).
    let side = w - 24.0;
    let qr = [(w - side) / 2.0, 14.0, side, side];
    let cy = qr[1] + side / 2.0;
    if show.qr.is_empty() {
        text(c, "Phone joining", [qr[0], cy - 6.0], 16.0, DIM, side, TextAlign::Center);
        text(c, "starts with Splitscreen Club", [qr[0], cy + 18.0], 16.0, DIM, side, TextAlign::Center);
    } else {
        // Dark modules on a white square, the host's SVG margin as the quiet
        // zone: Android's scanner (ML Kit) does not read inverted codes at
        // all. White for the most contrast (Joep, 25 November 2026: it read
        // grey); the plaque's screen is bright enough to show it as white.
        c.rect(qr, Color::WHITE);
        for m in &show.qr { c.rect([qr[0] + m[0] * side, qr[1] + m[1] * side, m[2] * side, m[3] * side], Color::hex("0c0a16")); }
    }
    let code = if show.room_code.is_empty() { "· · · ·" } else { &show.room_code };
    c.text(faces().wordmark, [0.0, h - 16.0], code, TextAlign::Center, w, 52.0, TEXT);
}

/// The visit-a-room screen (visit.rs): a kicker, the code's slots and
/// what to do, lit in violet (interaction, DESIGN.md) while it takes a code.
pub fn visit(c: &mut Canvas, v: &VisitShow) {
    let [w, h] = c.size;
    c.rect([0.0, 0.0, w, h], Color::hex("0c0a16"));
    let lilac = crate::room::NIGHT_LILAC;
    c.text(faces().bold, [0.0, 40.0], "VISIT A ROOM", TextAlign::Center, w, 26.0, AMBER);
    let n = crate::visit::LEN;
    let (side, gap) = (64.0, 12.0);
    let x0 = (w - (n as f32 * (side + gap) - gap)) / 2.0;
    let y = 64.0;
    for i in 0..n {
        let r = [x0 + i as f32 * (side + gap), y, side, side];
        let focus = v.open && i == v.code.len();
        let ring = if focus { (4.0, lilac) } else if v.hover { (2.0, lilac.with_alpha(0.7)) } else { (2.0, Color::hex("3a3f5c")) };
        c.style_box(r, [10.0; 4], Color::hex("1a1e36"), Some(ring));
        let (ch, color) = match v.code.chars().nth(i) {
            Some(ch) => (ch.to_string(), TEXT),
            None if focus && v.pick.is_some() => (v.pick.unwrap().to_string(), lilac),
            None => ("·".into(), DIM),
        };
        c.text(faces().bold, [r[0], r[1] + side / 2.0 + 14.0], &ch, TextAlign::Center, side, 40.0, color);
    }
    let line = if !v.open { "Click here to enter a room code" }
        else if v.pick.is_some() { "Stick picks · A types · B deletes" }
        else { "Type the code · Enter to go · Esc cancels" };
    text(c, line, [0.0, h - 20.0], 26.0, if v.open { TEXT } else { DIM }, w, TextAlign::Center);
}

/// The online switch's sign: PUBLIC on the left, FRIENDS ONLY on the right,
/// the one it is on lit in a green pill (green: state, DESIGN.md).
pub fn privacy(c: &mut Canvas, show: &Show) {
    // One word, what the room is now: green Public, amber Private.
    let [w, h] = c.size;
    c.rect([0.0, 0.0, w, h], Color::hex("0d1020"));
    let (word, color) = if show.public { ("PUBLIC", GREEN) } else { ("PRIVATE", AMBER) };
    c.style_box([8.0, 8.0, w - 16.0, h - 16.0], [22.0; 4], Color::hex("0d1020"), Some((6.0, color)));
    let size = fit(word, 96.0, w - 60.0, 30.0);
    c.text(faces().wordmark, [0.0, h / 2.0 + size * 0.36], word, TextAlign::Center, w, size, color.lightened(0.25));
}

/// The one floating panel (DESIGN.md "Panel"): bg at 88 %, 1 px edge,
/// radius 18 and a soft shadow 8 px under it, all at `scale` (1 at 1080p).
pub fn panel(c: &mut Canvas, r: [f32; 4], scale: f32) { panel_with(c, r, scale, crate::room::PANEL, crate::room::LINE); }

/// A menu's panel: 95 % so the room does not read through it.
pub fn panel_solid(c: &mut Canvas, r: [f32; 4], scale: f32) { panel_with(c, r, scale, crate::room::PANEL_SOLID, crate::room::LINE); }

/// A panel with its own edge colour (an error's red edge).
pub fn panel_with(c: &mut Canvas, r: [f32; 4], scale: f32, fill: Color, edge: Color) {
    let radius = (18.0 * scale).min(r[3] / 2.0);
    c.style_box([r[0], r[1] + 8.0 * scale, r[2], r[3]], [radius; 4], Color { r: 0.0, g: 0.0, b: 0.0, a: 0.3 * fill.a }, None);
    c.style_box(r, [radius; 4], fill, Some(((1.0 * scale).max(1.0), edge)));
}

/// A face button's colour (A green, B red, X blue, Y yellow), None for others.
pub fn pad_color(key: &str) -> Option<Color> {
    Some(match key.to_ascii_uppercase().as_str() { "A" => crate::room::PAD_A, "B" => crate::room::PAD_B, "X" => crate::room::PAD_X, "Y" => crate::room::PAD_Y, _ => return None })
}

/// Whether `key` is a stick: L3/R3 (press) or LS/RS (move).
pub fn is_stick(key: &str) -> bool { matches!(key.to_ascii_uppercase().as_str(), "L3" | "R3" | "LS" | "RS") }

/// A stick seen from above (DESIGN.md glyphs): a grey disc with a 1.5 px
/// edge, the cap's ring, L or R in it. L3/R3 (press) get a bar with a
/// down chevron under the disc; LS/RS (move) a triangle on each side.
pub fn stick(c: &mut Canvas, center: [f32; 2], radius: f32, key: &str, on: bool) {
    let key = key.to_ascii_uppercase();
    let edge = if on { Color::hex("b4b2c8") } else { Color::hex("55516a") };
    let ink = if on { TEXT } else { DIM };
    let press = key.ends_with('3');
    let lift = if press { radius * 0.22 } else { 0.0 };
    let o = [center[0], center[1] - lift];
    c.circle(o, radius, edge);
    c.circle(o, radius - (radius * 0.07).max(1.5), Color::hex("30303f"));
    c.circle(o, radius * 0.62, edge);
    c.circle(o, radius * 0.62 - (radius * 0.06).max(1.2), Color::hex("26262f"));
    let size = (radius * 0.7).floor();
    c.text(bold(), [o[0] - radius, o[1] + size * 0.36], &key[..1], TextAlign::Center, 2.0 * radius, size, ink);
    if press {
        // The press: a short bar under the stick, a chevron pointing into it.
        let y = o[1] + radius + radius * 0.2;
        c.rect([o[0] - radius * 0.5, y + radius * 0.16, radius, radius * 0.12], ink);
        c.polygon(&[[o[0] - radius * 0.22, y - radius * 0.04], [o[0] + radius * 0.22, y - radius * 0.04], [o[0], y + radius * 0.12]], ink);
    } else {
        for d in [-1.0f32, 1.0] {
            let tip = [o[0] + d * radius * 1.38, o[1]];
            let back = o[0] + d * radius * 1.12;
            c.polygon(&[tip, [back, o[1] - radius * 0.2], [back, o[1] + radius * 0.2]], ink);
        }
    }
}

/// room.gd `draw_bumper`: an Xbox-style bumper, with a chevron pointing
/// the way it browses (-1 left, 1 right, 0 none).
pub fn bumper(c: &mut Canvas, mut r: [f32; 4], key: &str, direction: i32, pressed: f32) {
    r[1] += pressed * 6.0;
    let round = (r[3] * 0.5).floor();
    let small = (round * 0.45).floor();
    let left_side = key.starts_with('L');
    let radii = [if left_side { round } else { small }, if key.starts_with('R') { round } else { small }, small, small];
    let border = (r[3] * 0.05).floor().max(1.0);
    c.style_box(r, radii, Color::hex("30303f").darkened(pressed * 0.3), Some((border, Color::hex("b4b2c8"))));
    let size = (r[3] * 0.58).floor();
    c.text(bold(), [r[0], r[1] + r[3] / 2.0 + size * 0.36], key, TextAlign::Center, r[2], size, Color::hex("f2f1f8"));
    if direction != 0 {
        let d = direction as f32;
        let tip = [if direction < 0 { r[0] - r[3] * 0.32 } else { r[0] + r[2] + r[3] * 0.32 }, r[1] + r[3] / 2.0];
        let back = tip[0] - d * r[3] * 0.24;
        let half = r[3] * 0.2;
        c.over();
        c.polygon(&[tip, [back, tip[1] - half], [back, tip[1] + half]], Color::hex("f2f1f8"));
    }
}

/// The shelf stand's bumper screens (`bumper_LB`, `bumper_RB`).
pub fn shelf_bumper(c: &mut Canvas, show: &Show, right: bool) {
    let (key, side, x) = if right { ("RB", 1, 20.0) } else { ("LB", -1, 60.0) };
    bumper(c, [x, 36.0, 320.0, 128.0], key, side, show.bumper_presses[right as usize]);
}

/// A floating row of controller buttons (room.gd `draw_prompt`).
#[derive(Clone, Debug, Default)]
pub struct Prompt {
    /// (button, label, on)
    pub items: Vec<(String, String, bool)>,
    pub enabled: bool,
    pub pressed: Option<(String, f32)>,
    pub hold: Option<(String, f32)>,
    /// How far the buttons are tucked away (0 shown, 1 label only): a
    /// nudge seen from afar shows just the words, and the button pops in
    /// once someone stands there.
    pub conceal: f32,
    pub conceal_to: f32,
    /// Shown for the mouse (nobody stands there): a mouse instead of the
    /// controller buttons, and the hovered button lit.
    pub mouse: bool,
    pub hover: Option<String>,
    /// Whoever stands there plays on the keyboard: keys instead of pad buttons.
    pub keys: bool,
    /// Which keys: 0 the whole keyboard, 1 or 2 its left or right half.
    pub half: u8,
}

impl Prompt {
    /// How much of a button shows: (width share, badge scale with overshoot).
    pub fn reveal(&self) -> (f32, f32) {
        // The word slides aside first, then the button pops into the room
        // it made (and the other way round when it tucks away), so the two
        // never overlap.
        let t = (1.0 - self.conceal).clamp(0.0, 1.0);
        let slide = (t / 0.55).min(1.0);
        let pop = ((t - 0.45) / 0.55).clamp(0.0, 1.0);
        let (c1, c3) = (1.70158, 2.70158);
        let back = if pop > 0.0 { (1.0 + c3 * (pop - 1.0).powi(3) + c1 * (pop - 1.0).powi(2)).max(0.0) } else { 0.0 };
        (slide * slide * (3.0 - 2.0 * slide), back)
    }

    /// Pixels the tucked-away buttons save on this row.
    pub fn concealed_width(&self) -> f32 {
        let (w, _) = self.reveal();
        self.items.iter().filter(|(k, _, _)| k != "LB" && k != "RB").count() as f32 * 66.0 * (1.0 - w)
    }
}

/// The key a keyboard player presses for a pad button (app::input's keys),
/// or None where the keyboard has none.
pub fn key_cap(button: &str) -> Option<&'static str> { key_cap_on(0, button) }

/// Pad button `button` on keyboard `half`: 0 the whole keyboard, 1 its left
/// half, 2 its right half when two people share it (app input.rs
/// `half_button`). None where that half has no key for it.
pub fn key_cap_on(half: u8, button: &str) -> Option<&'static str> {
    let b = button.to_ascii_uppercase();
    Some(match (half, b.as_str()) {
        (1, "A") => "Space", (1, "B") => "L Shift", (1, "X") => "Q", (1, "Y") => "E", (1, "BACK") => "Tab", (1, "START") => "1",
        (2, "A") => "R Shift", (2, "B") => "/", (2, "X") => ".", (2, "Y") => "'", (2, "BACK") => "Backspace", (2, "START") => "Enter",
        (1 | 2, _) => return None,
        (_, "A") => "Space", (_, "B") => "Backspace", (_, "X") => "R", (_, "Y") => "F", (_, "LB") => "Q", (_, "RB") => "E", (_, "BACK") => "Tab", (_, "START") => "Enter", (_, "R3") => "C", (_, "ENTER") => "Enter", (_, "G") => "G",
        _ => return None,
    })
}

/// How wide a keyboard key's cap is at prompt size (44 px labels).
pub fn key_cap_width(cap: &str) -> f32 { (render::hud::measure(cap, bold(), 44.0)[0] + 40.0).max(84.0) }

/// A keyboard key: a light cap with the key's name, `r` its rectangle.
pub fn keycap(c: &mut Canvas, r: [f32; 4], cap: &str, pressed: f32, on: bool) {
    let r = [r[0], r[1] + pressed * 6.0, r[2], r[3]];
    let radius = r[3] * 0.22;
    c.style_box([r[0], r[1] + r[3] * 0.1, r[2], r[3]], [radius; 4], Color::hex("0b0b12"), None);
    // A touch under white: in the room the glowing prompt blooms, and a
    // pure white cap bled into its letter.
    c.style_box(r, [radius; 4], if on { Color::hex("d6d4e2") } else { Color::hex("8d8aa0") }, Some(((r[3] * 0.05).max(1.5), Color::hex("0b0b12"))));
    let size = (r[3] * 0.66).floor();
    c.text(bold(), [r[0], r[1] + r[3] / 2.0 + size * 0.36], cap, TextAlign::Center, r[2], size, Color::hex("0b0b12"));
}

pub fn prompt(c: &mut Canvas, state: &Prompt) {
    let h = 96.0;
    let mut x = 20.0;
    for (key, label, item_on) in &state.items {
        let cap = if state.keys && !state.mouse { key_cap_on(state.half, key) } else { None };
        let bumper_key = cap.is_none() && (key == "LB" || key == "RB");
        let badge = match cap { Some(cap) => key_cap_width(cap) + 24.0, None if bumper_key => 150.0, None => 96.0 };
        let tw = render::hud::measure(label, bold(), 44.0)[0];
        let (w, scale) = if bumper_key || cap.is_some() { (1.0, 1.0) } else { state.reveal() };
        let label_x = x + (badge - 6.0) * w + 24.0 * (1.0 - w);
        let pill = label_x - x + tw + 22.0;
        let hovered = state.mouse && state.hover.as_deref() == Some(key.as_str());
        // The hovered button is the one a click works: lit, with a ring.
        c.style_box([x, 6.0, pill, h - 12.0], [h / 2.0 - 6.0; 4], if hovered { crate::godot::rgba(0.22, 0.16, 0.42, 0.95) } else { crate::godot::rgba(0.03, 0.03, 0.06, 0.82) },
            hovered.then(|| (4.0, Color::hex("f8cd34"))));
        let on = state.enabled && *item_on;
        let color = if on { pad_color(key).unwrap_or(TEXT) } else { Color::hex("55516a") };
        let pressed = state.pressed.as_ref().filter(|p| &p.0 == key).map_or(0.0, |p| p.1);
        if let Some(cap) = cap {
            // As tall as the pill allows: the key's letter reads at a distance.
            keycap(c, [x + 12.0, 12.0, badge - 24.0, 70.0], cap, pressed, on);
        } else if state.mouse {
            // A mouse with its left button lit (the wheel for LB/RB), and an
            // arrow for the way a bumper browses.
            let cx = if bumper_key { x + 40.0 } else { x + h / 2.0 - 4.0 };
            mouse_icon(c, [cx, h / 2.0 + pressed * 6.0], if on { Color::hex("f8cd34") } else { Color::hex("55516a") }, bumper_key);
            if bumper_key {
                let (ax, s) = (x + 102.0, if key == "LB" { -1.0 } else { 1.0 });
                c.polygon(&[[ax - 14.0 * s, h / 2.0 - 16.0], [ax - 14.0 * s, h / 2.0 + 16.0], [ax + 14.0 * s, h / 2.0]], TEXT);
            }
        } else if bumper_key {
            bumper(c, [x + 12.0, 18.0, 122.0, 60.0], key, 0, pressed);
        } else if is_stick(key) && scale > 0.02 {
            let center = [x + (h / 2.0 - 4.0) * w + 24.0 * (1.0 - w), h / 2.0 + pressed * 6.0];
            stick(c, center, (h / 2.0 - 14.0) * scale.min(1.0), key, on);
        } else if scale > 0.02 {
            let center = [x + (h / 2.0 - 4.0) * w + 24.0 * (1.0 - w), h / 2.0];
            c.circle(center, (h / 2.0 - 13.0) * scale, color.darkened(0.65));
            if let Some((_, hold)) = state.hold.as_ref().filter(|p| &p.0 == key && p.1 > 0.0) {
                c.arc(center, h / 2.0 - 4.0, -PI / 2.0, -PI / 2.0 + TAU * hold, 7.0, color.lightened(0.3));
            }
            // The badge squashes down when pressed; the label stays put.
            let squash = 1.0 - pressed * 0.18;
            let face_center = [center[0], center[1] + pressed * 10.0];
            let r = (h / 2.0 - 14.0) * scale;
            let points: Vec<[f32; 2]> = (0..48).map(|i| { let a = i as f32 / 48.0 * TAU; [face_center[0] + a.cos() * r, face_center[1] + a.sin() * r * squash] }).collect();
            c.polygon(&points, color.darkened(pressed * 0.12));
            if scale > 0.5 {
                let size = 48.0 * scale.min(1.15);
                c.text(bold(), [center[0] - (h - 28.0) / 2.0, h / 2.0 + 17.0 * size / 48.0 + pressed * 10.0], key, TextAlign::Center, h - 28.0, size, Color::hex("0b0b12"));
            }
        }
        c.text(bold(), [label_x, h / 2.0 + 16.0], label, TextAlign::Left, -1.0, 44.0, if on { TEXT } else { DIM });
        x += pill + 20.0;
    }
}

/// Where a prompt's row starts on its screen: centred, except at the exit and the shelf,
/// which sits by the screen's left edge, so its row starts over the door
/// and reads to the right.
pub fn prompt_left(name: &str, width: f32, need: f32) -> f32 {
    let centred = ((width - need) / 2.0).max(0.0).floor();
    match name {
        "exit" => centred.max((width / 2.0 - 64.0).floor()),
        // The shelf is by the screen's left edge: its row leans right.
        "shelf" => (centred + 110.0).min((width - need).max(0.0)).floor(),
        _ => centred,
    }
}

/// A mouse seen from above, centred at `at`: the left button (or the
/// wheel) in `lit`.
fn mouse_icon(c: &mut Canvas, at: [f32; 2], lit: Color, wheel: bool) {
    // High contrast on the dark pill: a white body with a dark outline,
    // dark seams, and the lit button a strong amber with its own outline.
    let (w, h) = (40.0, 60.0);
    let ink = Color::hex("0b0b12");
    let r = [at[0] - w / 2.0, at[1] - h / 2.0, w, h];
    c.style_box([r[0] - 3.0, r[1] - 3.0, w + 6.0, h + 6.0], [w / 2.0 + 3.0; 4], ink, None);
    c.style_box(r, [w / 2.0; 4], Color::hex("ffffff"), None);
    let split = r[1] + h * 0.42;
    let lit = if lit == Color::hex("f8cd34") { Color::hex("ffa600") } else { lit };
    if wheel {
        c.style_box([at[0] - 7.0, r[1] + 6.0, 14.0, 20.0], [7.0; 4], ink, None);
        c.style_box([at[0] - 4.5, r[1] + 8.5, 9.0, 15.0], [4.5; 4], lit, None);
    } else {
        let b = [r[0] + 2.0, r[1] + 2.0, w / 2.0 - 2.0, split - r[1] - 2.0];
        c.style_box(b, [w / 2.0 - 2.0, 2.0, 2.0, 2.0], ink, None);
        c.style_box([b[0] + 3.0, b[1] + 3.0, b[2] - 5.0, b[3] - 5.0], [w / 2.0 - 5.0, 2.0, 2.0, 2.0], lit, None);
        c.rect([at[0] - 2.0, r[1], 4.0, split - r[1]], ink);
    }
    c.rect([r[0] + 2.0, split - 2.0, w - 4.0, 4.0], ink);
}

/// room.gd `_draw_music_icon`: previous, play/pause, next on the pads.
pub fn music_icon(c: &mut Canvas, index: usize) {
    let ink = crate::godot::rgb(0.12, 0.03, 0.08);
    match index {
        0 => {
            c.rect([14.0, 18.0, 5.0, 28.0], ink);
            c.polygon(&[[48.0, 18.0], [48.0, 46.0], [21.0, 32.0]], ink);
        }
        1 => {
            c.polygon(&[[13.0, 17.0], [13.0, 47.0], [33.0, 32.0]], ink);
            c.rect([38.0, 18.0, 5.0, 28.0], ink);
            c.rect([47.0, 18.0, 5.0, 28.0], ink);
        }
        _ => {
            c.polygon(&[[16.0, 18.0], [16.0, 46.0], [43.0, 32.0]], ink);
            c.rect([45.0, 18.0, 5.0, 28.0], ink);
        }
    }
}

/// Paints the named screen; false for a screen this module does not know.
pub fn paint(c: &mut Canvas, name: &str, show: &Show, prompts: &std::collections::HashMap<String, Prompt>) -> bool {
    match name {
        "logo" => logo(c),
        "tv" => tv(c, show),
        "box_front" => box_front(c, show),
        "queue0" => queue_preview(c, show),
        "queue_titles" => queue_titles(c, show),
        "jukebox" => jukebox(c, show),
        "plaque" => plaque(c, show),
        "visit" => visit(c, &show.visit),
        "privacy" => privacy(c, show),
        "bumper_LB" => shelf_bumper(c, show, false),
        "bumper_RB" => shelf_bumper(c, show, true),
        _ if name.starts_with("music_icon") => music_icon(c, name["music_icon".len()..].parse().unwrap_or(0)),
        _ if name.starts_with("prompt_") => match prompts.get(&name["prompt_".len()..]) {
            Some(p) => {
                // The live lobby gives prompts a wider screen; centre the pills in it.
                let items: Vec<(String, String)> = p.items.iter().map(|(k, t, _)| (k.clone(), t.clone())).collect();
                let need = crate::room::prompt_size(&items)[0] as f32 - p.concealed_width();
                c.shift(prompt_left(&name["prompt_".len()..], c.size[0], need));
                prompt(c, p)
            }
            None => return false,
        },
        _ => return false,
    }
    true
}

#[cfg(test)]
mod key_cap_tests {
    use super::key_cap_on;

    #[test]
    fn each_keyboard_half_shows_its_own_keys() {
        assert_eq!(key_cap_on(0, "A"), Some("Space"));
        assert_eq!(key_cap_on(1, "A"), Some("Space"));
        assert_eq!(key_cap_on(2, "A"), Some("R Shift"));
        assert_eq!(key_cap_on(1, "b"), Some("L Shift"));
        assert_eq!(key_cap_on(2, "BACK"), Some("Backspace"));
        assert_eq!(key_cap_on(2, "LB"), None, "the halves have no bumpers");
    }
}
