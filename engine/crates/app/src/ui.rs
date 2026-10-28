//! Menus for keyboard, mouse-free and gamepad play: a focus that moves on
//! the D-pad, arrows or left stick (with key repeat), South/Enter selects,
//! East backs out. Drawing goes into the HUD list, so it shows in every
//! view and in headless captures. Immediate mode: keep a `Menu` per local
//! player and call `update` with that player's controls each tick.
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
}
