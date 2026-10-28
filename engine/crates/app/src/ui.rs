//! Menus for keyboard, mouse-free and gamepad play: a focus that moves on
//! the D-pad, arrows or left stick (with key repeat), South/Enter selects,
//! East backs out. Drawing goes into the HUD list, so it shows in every
//! view and in headless captures. Immediate mode: keep a `Menu` per local
//! player and call `update` with that player's controls each tick.
//! [`text_field`] draws a [`crate::text::TextField`] in the same style, and
//! [`CharPicker`] lets a pad type into one.
use crate::text::{TextEvent, TextField};
use crate::{Buttons, Controls};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEvent { None, Moved, Select(usize), Back }

#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub focus: usize,
    held_dir: (i32, i32),
    repeat: f32,
}

const FIRST_REPEAT: f32 = 0.35;
const NEXT_REPEAT: f32 = 0.12;

impl Menu {
    /// Move and select among `count` items laid out in `columns` columns.
    pub fn update(&mut self, c: &Controls, count: usize, columns: usize, dt: f32) -> MenuEvent {
        if count == 0 { return MenuEvent::None; }
        let columns = columns.max(1);
        self.focus = self.focus.min(count - 1);
        if c.pressed.has(Buttons::SOUTH | Buttons::START) { return MenuEvent::Select(self.focus); }
        if c.pressed.has(Buttons::EAST | Buttons::SELECT) { return MenuEvent::Back; }
        // Direction from buttons or the stick; edges move at once, holds repeat.
        let b = |m| c.held.has(m) as i32;
        let mut dir = (b(Buttons::RIGHT) - b(Buttons::LEFT), b(Buttons::DOWN) - b(Buttons::UP));
        if dir == (0, 0) {
            let (x, y) = (c.mv[0], c.mv[1]);
            if x.abs() > 0.6 || y.abs() > 0.6 { dir = if x.abs() > y.abs() { (x.signum() as i32, 0) } else { (0, -y.signum() as i32) }; }
        }
        let pressed = (c.pressed.has(Buttons::RIGHT) as i32 - c.pressed.has(Buttons::LEFT) as i32, c.pressed.has(Buttons::DOWN) as i32 - c.pressed.has(Buttons::UP) as i32);
        let step = if pressed != (0, 0) { self.held_dir = pressed; self.repeat = FIRST_REPEAT; Some(pressed) }
            else if dir == (0, 0) { self.held_dir = (0, 0); None }
            else if dir != self.held_dir { self.held_dir = dir; self.repeat = FIRST_REPEAT; Some(dir) }
            else { self.repeat -= dt; if self.repeat <= 0.0 { self.repeat = NEXT_REPEAT; Some(dir) } else { None } };
        let Some((dx, dy)) = step else { return MenuEvent::None };
        let (col, row) = ((self.focus % columns) as i32, (self.focus / columns) as i32);
        let rows = count.div_ceil(columns) as i32;
        let (col, row) = ((col + dx).rem_euclid(columns as i32), (row + dy).rem_euclid(rows));
        let next = (row as usize * columns + col as usize).min(count - 1);
        if next == self.focus { return MenuEvent::None; }
        self.focus = next;
        MenuEvent::Moved
    }
}

/// Colours and scale for menus (straight-alpha linear RGBA for shapes).
#[derive(Clone, Copy, Debug)]
pub struct Style { pub panel: [f32; 4], pub item: [f32; 4], pub focus: [f32; 4], pub text: [u8; 4], pub muted: [u8; 4], pub scale: f32 }

impl Style {
    pub fn for_height(h: f32) -> Self {
        Style { panel: [0.02, 0.025, 0.035, 0.82], item: [0.12, 0.13, 0.16, 0.9], focus: [0.95, 0.7, 0.25, 0.95], text: [240, 240, 235, 255], muted: [190, 190, 185, 220], scale: (h / 1080.0).max(0.35) }
    }
}

fn text(d: &mut HudDrawList, id: u64, s: &str, x: f32, y: f32, size: f32, color: [u8; 4], width: f32, clip: [f32; 4]) {
    d.text.push(TextRun { id, text: s.into(), position: [x, y], bounds: clip, font_size: size, color, font: FontFace::Body, align: TextAlign::Center, width });
}

/// A vertical list, centred in `rect` (x, y, w, h).
pub fn list(d: &mut HudDrawList, id: u64, rect: [f32; 4], title: &str, items: &[&str], focus: usize, st: &Style) {
    let s = st.scale;
    let (iw, ih, gap) = (rect[2].min(520.0 * s), 54.0 * s, 10.0 * s);
    let h = 70.0 * s + items.len() as f32 * (ih + gap);
    let (x, y) = (rect[0] + (rect[2] - iw) * 0.5, rect[1] + (rect[3] - h) * 0.5);
    d.rect([x - 20.0 * s, y - 20.0 * s, iw + 40.0 * s, h + 40.0 * s], st.panel, rect);
    text(d, id, title, x, y, 40.0 * s, st.text, iw, rect);
    for (i, it) in items.iter().enumerate() {
        let iy = y + 70.0 * s + i as f32 * (ih + gap);
        if i == focus { d.rect([x - 4.0 * s, iy - 4.0 * s, iw + 8.0 * s, ih + 8.0 * s], st.focus, rect); }
        d.rect([x, iy, iw, ih], st.item, rect);
        text(d, id + 1 + i as u64, it, x, iy + 10.0 * s, 30.0 * s, st.text, iw, rect);
    }
}

/// A card for `cards`: title, body text and an accent colour (sRGB 0..1).
pub struct Card<'a> { pub title: &'a str, pub body: &'a str, pub color: [f32; 3] }

/// Cards in a row, centred in `rect`, with a footer line (e.g. a countdown).
pub fn cards(d: &mut HudDrawList, id: u64, rect: [f32; 4], title: &str, cards: &[Card], focus: usize, footer: &str, st: &Style) {
    let s = st.scale;
    let n = cards.len().max(1) as f32;
    let gap = 24.0 * s;
    let cw = ((rect[2] - 80.0 * s - gap * (n - 1.0)) / n).min(300.0 * s);
    let ch = cw * 1.35;
    let total = cw * n + gap * (n - 1.0);
    let (x0, y0) = (rect[0] + (rect[2] - total) * 0.5, rect[1] + (rect[3] - ch) * 0.5 + 20.0 * s);
    d.rect([rect[0], rect[1], rect[2], rect[3]], [0.0, 0.0, 0.0, 0.45], rect);
    text(d, id, title, rect[0], y0 - 80.0 * s, 44.0 * s, st.text, rect[2], rect);
    for (i, c) in cards.iter().enumerate() {
        let x = x0 + i as f32 * (cw + gap);
        let lift = if i == focus { 18.0 * s } else { 0.0 };
        let y = y0 - lift;
        let lin = c.color.map(|v| v.powf(2.2));
        if i == focus { d.rect([x - 6.0 * s, y - 6.0 * s, cw + 12.0 * s, ch + 12.0 * s], st.focus, rect); }
        d.rect([x, y, cw, ch], [0.08, 0.08, 0.1, 0.96], rect);
        d.rect([x, y, cw, ch * 0.42], [lin[0], lin[1], lin[2], 0.9], rect);
        text(d, id + 1 + 2 * i as u64, c.title, x, y + ch * 0.47, 30.0 * s, st.text, cw, rect);
        text(d, id + 2 + 2 * i as u64, c.body, x, y + ch * 0.62, 22.0 * s, st.muted, cw, rect);
    }
    if !footer.is_empty() { text(d, id + 99, footer, rect[0], y0 + ch + 30.0 * s, 24.0 * s, st.muted, rect[2], rect); }
}

/// A text field in the menu style: the item plate, the focus frame when
/// focused, the text (or the muted placeholder) and a caret.
pub fn text_field(d: &mut HudDrawList, id: u64, rect: [f32; 4], field: &TextField, placeholder: &str, focused: bool, st: &Style) {
    let s = st.scale;
    if focused { d.rect([rect[0] - 4.0 * s, rect[1] - 4.0 * s, rect[2] + 8.0 * s, rect[3] + 8.0 * s], st.focus, rect_grow(rect, 4.0 * s)); }
    d.rect(rect, st.item, rect);
    let inner = [rect[0] + 14.0 * s, rect[1], rect[2] - 28.0 * s, rect[3]];
    let caret = focused.then(|| [st.text[0] as f32 / 255.0, st.text[1] as f32 / 255.0, st.text[2] as f32 / 255.0, 1.0]);
    field.draw(d, id, inner, FontFace::Body, 26.0 * s, st.text, (placeholder, st.muted), caret);
}

fn rect_grow(r: [f32; 4], by: f32) -> [f32; 4] { [r[0] - by, r[1] - by, r[2] + 2.0 * by, r[3] + 2.0 * by] }

/// An on-screen keyboard for pads: a grid of letters, digits and a few
/// marks, then SHIFT, DEL, PASTE and DONE. The focus moves like a [`Menu`];
/// South types the focused key, West deletes, Start is DONE, East or Select
/// cancels. Each update yields at most one [`TextEvent`].
#[derive(Clone, Debug, Default)]
pub struct CharPicker {
    pub menu: Menu,
    pub upper: bool,
}

const PICKER_COLUMNS: usize = 10;
const PICKER_CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789-_.";
const PICKER_KEYS: [&str; 5] = ["SPACE", "SHIFT", "DEL", "PASTE", "DONE"];

impl CharPicker {
    fn count() -> usize { PICKER_CHARS.chars().count() + PICKER_KEYS.len() }

    /// The label of key `k` as drawn.
    pub fn label(&self, k: usize) -> String {
        match PICKER_CHARS.chars().nth(k) {
            Some(c) if self.upper => c.to_ascii_uppercase().to_string(),
            Some(c) => c.to_string(),
            None => PICKER_KEYS.get(k - PICKER_CHARS.chars().count()).map_or(String::new(), |s| s.to_string()),
        }
    }

    pub fn update(&mut self, c: &Controls, dt: f32) -> Option<TextEvent> {
        if c.pressed.has(Buttons::START) { return Some(TextEvent::Enter); }
        if c.pressed.has(Buttons::WEST) { return Some(TextEvent::Backspace); }
        let ev = self.menu.update(&Controls { pressed: Buttons(c.pressed.0 & !Buttons::START), held: Buttons(c.held.0 & !Buttons::START), ..*c }, Self::count(), PICKER_COLUMNS, dt);
        match ev {
            MenuEvent::Back => Some(TextEvent::Escape),
            MenuEvent::Select(k) => match self.label(k).as_str() {
                "SPACE" => Some(TextEvent::Insert(" ".into())),
                "SHIFT" => { self.upper = !self.upper; None }
                "DEL" => Some(TextEvent::Backspace),
                "PASTE" => Some(TextEvent::Paste(crate::clipboard::get())),
                "DONE" => Some(TextEvent::Enter),
                ch => Some(TextEvent::Insert(ch.into())),
            },
            _ => None,
        }
    }

    /// The grid, centred in `rect`, with a hint line under it.
    pub fn draw(&self, d: &mut HudDrawList, id: u64, rect: [f32; 4], st: &Style) {
        let s = st.scale;
        let (kw, kh, gap) = (56.0 * s, 48.0 * s, 6.0 * s);
        let rows = Self::count().div_ceil(PICKER_COLUMNS);
        let (w, h) = (PICKER_COLUMNS as f32 * (kw + gap) - gap, rows as f32 * (kh + gap) - gap + 40.0 * s);
        let (x0, y0) = (rect[0] + (rect[2] - w) * 0.5, rect[1] + (rect[3] - h) * 0.5);
        d.rect([x0 - 16.0 * s, y0 - 16.0 * s, w + 32.0 * s, h + 32.0 * s], st.panel, rect);
        for k in 0..Self::count() {
            let (col, row) = (k % PICKER_COLUMNS, k / PICKER_COLUMNS);
            let r = [x0 + col as f32 * (kw + gap), y0 + row as f32 * (kh + gap), kw, kh];
            if k == self.menu.focus { d.rect([r[0] - 3.0 * s, r[1] - 3.0 * s, r[2] + 6.0 * s, r[3] + 6.0 * s], st.focus, rect); }
            d.rect(r, st.item, rect);
            let label = self.label(k);
            let size = if label.len() > 1 { 15.0 } else { 26.0 } * s;
            text(d, id + 1 + k as u64, &label, r[0], r[1] + (kh - size * 1.2) * 0.5, size, if label == "SHIFT" && self.upper { [255, 200, 90, 255] } else { st.text }, kw, rect);
        }
        text(d, id, "A type   X delete   START done   B cancel", x0, y0 + h - 26.0 * s, 20.0 * s, st.muted, w, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(b: u32) -> Controls { Controls { held: Buttons(b), pressed: Buttons(b), ..Default::default() } }

    #[test]
    fn focus_moves_wraps_repeats_and_selects() {
        let mut m = Menu::default();
        assert_eq!(m.update(&press(Buttons::RIGHT), 3, 3, 0.016), MenuEvent::Moved);
        assert_eq!(m.focus, 1);
        assert_eq!(m.update(&press(Buttons::LEFT), 3, 3, 0.016), MenuEvent::Moved);
        assert_eq!(m.update(&press(Buttons::LEFT), 3, 3, 0.016), MenuEvent::Moved);
        assert_eq!(m.focus, 2, "wraps");
        // Holding the stick right repeats after the first delay.
        let stick = Controls { mv: [1.0, 0.0], ..Default::default() };
        let moves = (0..60).filter(|_| m.update(&stick, 3, 3, 1.0 / 60.0) == MenuEvent::Moved).count();
        assert!((3..=8).contains(&moves), "{moves}");
        assert_eq!(m.update(&press(Buttons::SOUTH), 3, 3, 0.016), MenuEvent::Select(m.focus));
        assert_eq!(m.update(&press(Buttons::EAST), 3, 3, 0.016), MenuEvent::Back);
        let mut l = Menu::default();
        l.update(&press(Buttons::DOWN), 4, 1, 0.016);
        assert_eq!(l.focus, 1, "a list moves down");
    }

    #[test]
    fn char_picker_types_deletes_shifts_and_cancels() {
        let mut p = CharPicker::default();
        assert_eq!(p.update(&press(Buttons::SOUTH), 0.016), Some(TextEvent::Insert("a".into())));
        p.update(&press(Buttons::RIGHT), 0.016);
        assert_eq!(p.update(&press(Buttons::SOUTH), 0.016), Some(TextEvent::Insert("b".into())));
        assert_eq!(p.update(&press(Buttons::WEST), 0.016), Some(TextEvent::Backspace));
        assert_eq!(p.update(&press(Buttons::START), 0.016), Some(TextEvent::Enter));
        // Down five rows reaches the last row's keys (SHIFT is its second).
        p.menu.focus = 39 + 1;
        assert_eq!(p.label(p.menu.focus), "SHIFT");
        assert_eq!(p.update(&press(Buttons::SOUTH), 0.016), None);
        assert!(p.upper);
        p.menu.focus = 2;
        assert_eq!(p.update(&press(Buttons::SOUTH), 0.016), Some(TextEvent::Insert("C".into())));
        assert_eq!(p.update(&press(Buttons::EAST), 0.016), Some(TextEvent::Escape));
        let mut d = HudDrawList::new([1280, 720]);
        p.draw(&mut d, 10, [0.0, 0.0, 1280.0, 720.0], &Style::for_height(720.0));
        assert_eq!(d.text.len(), CharPicker::count() + 1);
        let mut d = HudDrawList::new([1280, 720]);
        let mut f = TextField::new(0);
        f.set("abc");
        text_field(&mut d, 1, [10.0, 10.0, 300.0, 40.0], &f, "", true, &Style::for_height(720.0));
        assert_eq!(d.text[0].text, "abc");
    }
}
/// Half-open pixel bounds, shared by drawn UI hit targets.
pub fn contains(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[1] >= r[1] && p[0] < r[0] + r[2] && p[1] < r[1] + r[3]
}
