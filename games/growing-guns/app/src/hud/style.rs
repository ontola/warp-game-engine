//! The "Arena" look from Godot's `ui_style.gd`: fight-poster type (Anton),
//! slanted plates with a red edge, bone white on arterial red. Colours are
//! straight-alpha sRGB, as the HUD list takes them. Vector icons from
//! `ui_style.gd` and `card_face.gd`, drawn into a square.
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use std::f32::consts::{PI, TAU};

pub type C = [f32; 4];

pub const RED: C = [0.843, 0.149, 0.239, 1.0];
pub const BONE: C = [0.965, 0.933, 0.886, 1.0];
pub const BONE_DIM: C = [0.965, 0.933, 0.886, 0.72];
pub const PLATE: C = [0.07, 0.035, 0.04, 0.8];
pub const INK: C = [0.11, 0.065, 0.07, 1.0];
pub const BLACK: C = [0.0, 0.0, 0.0, 1.0];
pub const GOLD: C = [1.0, 0.78, 0.25, 1.0];
pub const GOOD: C = [0.55, 1.0, 0.45, 1.0];
pub const BAD: C = [1.0, 0.38, 0.36, 1.0];
/// Plates lean like the type: x shifts by this fraction of the height.
pub const SKEW: f32 = 0.21;

static ANTON: &[u8] = include_bytes!("../../../assets/fonts/Anton-Regular.ttf");
static BARLOW_EXTRA_BOLD: &[u8] = include_bytes!("../../../assets/fonts/BarlowCondensed-ExtraBold.ttf");
/// Both fonts ship under the SIL OFL 1.1; keep these with distributions.
#[allow(dead_code)]
pub const FONT_LICENSES: [&str; 2] = [include_str!("../../../assets/fonts/OFL-Anton.txt"), include_str!("../../../assets/fonts/OFL-BarlowCondensed.txt")];

/// The game's type, three roles (keep to them; Joep asked for one system):
///
/// - Headline: Anton in bone white with a hard red shadow (`Label::poster`).
///   Everything that announces: banners ("JOEP DIED", "AIR STRIKE
///   INCOMING"), round results, "PICK A CARD", menu and board titles.
///   Always capitals. Sizes `size::XL` (match over), `L` (screen banner),
///   `M` (a view's headline, the card picker's title).
/// - Label: Barlow Condensed ExtraBold (`Label::label`), capitals, outlined
///   on the scene: buttons, card bands, toasts, prompts, the kill feed.
///   Sizes `S` and `XS`.
/// - Body: Barlow Condensed SemiBold (`BODY`): descriptions, small print,
///   settings rows. `XS` and smaller.
///
/// One headline at a time per place: a new one fades the old out first
/// (the banner gives way to the card picker's title).
pub mod size {
    pub const XL: f32 = 92.0;
    pub const L: f32 = 44.0;
    pub const M: f32 = 30.0;
    pub const S: f32 = 22.0;
    pub const XS: f32 = 16.0;
}

/// Poster face (Anton): HUD numbers, titles, banners.
pub fn display() -> FontFace { render::hud::register_font(ANTON) }
/// Barlow Condensed ExtraBold: buttons, card bands, prompts.
pub fn bold() -> FontFace { render::hud::register_font(BARLOW_EXTRA_BOLD) }
/// Barlow Condensed SemiBold: labels and body text (the engine's body face).
pub const BODY: FontFace = FontFace::Body;

pub fn with_alpha(c: C, a: f32) -> C { [c[0], c[1], c[2], c[3] * a] }
pub fn lerp(a: C, b: C, t: f32) -> C { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t] }
pub fn darkened(c: C, k: f32) -> C { [c[0] * (1.0 - k), c[1] * (1.0 - k), c[2] * (1.0 - k), c[3]] }
pub fn rgb(c: [f32; 3]) -> C { [c[0], c[1], c[2], 1.0] }
pub fn bytes(c: C) -> [u8; 4] { c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8) }

/// Text ids: one per run in draw order, so a frame laid out like the last
/// reuses its shaped text.
pub(crate) fn next_id(d: &HudDrawList) -> u64 { (0x6767 << 32) + d.text.len() as u64 }

/// One line of text. `x` is the left edge, centre or right edge by `align`;
/// `y` is the vertical centre of the line.
#[derive(Clone, Copy)]
pub struct Label { pub font: FontFace, pub size: f32, pub color: C, pub align: TextAlign, pub shadow: Option<(f32, C)>, pub outline: Option<(f32, C)> }

impl Label {
    pub fn new(font: FontFace, size: f32, color: C) -> Self { Label { font, size, color, align: TextAlign::Left, shadow: None, outline: None } }
    pub fn center(mut self) -> Self { self.align = TextAlign::Center; self }
    pub fn right(mut self) -> Self { self.align = TextAlign::Right; self }
    /// A hard drop shadow down and right (Godot's `display_label`).
    pub fn shadow(mut self, px: f32, c: C) -> Self { self.shadow = Some((px, c)); self }
    /// A dark edge all round (Godot's `outline_size`), drawn as offset copies.
    pub fn outline(mut self, px: f32, c: C) -> Self { self.outline = Some((px, c)); self }
    /// The label role: Barlow ExtraBold with a dark outline (`size` px).
    pub fn label(size: f32, color: C) -> Self { Label::new(bold(), size, color).outline((size * 0.2).max(2.0), with_alpha(BLACK, 0.9)) }
    /// Poster display text: bone white with a hard red shadow of 7% of the size.
    pub fn poster(size: f32) -> Self { Label::new(display(), size, BONE).shadow((size * 0.07).max(2.0), RED) }

    pub fn draw(&self, d: &mut HudDrawList, text: &str, x: f32, y: f32, clip: [f32; 4]) {
        if text.is_empty() || self.size < 1.0 { return; }
        let lines = text.lines().count().max(1) as f32;
        let h = self.size * 1.2 * lines;
        let w = render::hud::measure(text, self.font, self.size)[0] + 4.0;
        let left = match self.align { TextAlign::Left => x, TextAlign::Center => x - w * 0.5, TextAlign::Right => x - w };
        // glyphon lines sit a little low for these faces; lift to centre.
        let top = y - h * 0.5 - self.size * 0.04;
        let run = |d: &mut HudDrawList, dx: f32, dy: f32, c: C| {
            if c[3] <= 0.003 { return; }
            d.text.push(TextRun { id: next_id(d), text: text.into(), position: [left + dx, top + dy], bounds: clip, font_size: self.size, color: bytes(c), font: self.font, align: self.align, width: w });
        };
        if let Some((px, c)) = self.outline {
            let a = with_alpha(c, self.color[3]);
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, -1.4), (0.0, 1.4), (-1.4, 0.0), (1.4, 0.0)] { run(d, dx * px * 0.5, dy * px * 0.5, a); }
        }
        if let Some((px, c)) = self.shadow { run(d, px, px, with_alpha(c, self.color[3])); }
        run(d, 0.0, 0.0, self.color);
    }
    pub fn width(&self, text: &str) -> f32 { render::hud::measure(text, self.font, self.size)[0] }
}

/// The largest size up to `size` at which `text` fits in `width`.
pub fn fit(text: &str, font: FontFace, size: f32, width: f32) -> f32 {
    let w = render::hud::measure(text, font, size)[0];
    if w <= width || w <= 0.0 { size } else { (size * width / w).max(6.0) }
}

/// A slanted plate (Godot `UiStyle.plate`): the top leans right by SKEW of
/// the height; `accent` paints the red edge on the left.
pub fn plate(d: &mut HudDrawList, r: [f32; 4], bg: C, accent: Option<(f32, C)>, clip: [f32; 4]) {
    let [x, y, w, h] = r;
    let k = SKEW * h * 0.5;
    d.polygon(&[[x + k, y], [x + w + k, y], [x + w - k, y + h], [x - k, y + h]], bg, clip);
    if let Some((e, c)) = accent { d.polygon(&[[x + k, y], [x + k + e, y], [x - k + e, y + h], [x - k, y + h]], c, clip); }
}

/// The big framed panel behind a menu (Godot `UiStyle.menu_panel`).
pub fn menu_panel(d: &mut HudDrawList, r: [f32; 4], u: f32, clip: [f32; 4]) {
    d.rect(r, [0.06, 0.03, 0.035, 0.94], clip);
    d.rect([r[0], r[1], 6.0 * u, r[3]], RED, clip);
}

pub fn frame_rect(d: &mut HudDrawList, r: [f32; 4], w: f32, c: C, clip: [f32; 4]) {
    let [x, y, rw, rh] = r;
    d.rect([x, y, rw, w], c, clip);
    d.rect([x, y + rh - w, rw, w], c, clip);
    d.rect([x, y, w, rh], c, clip);
    d.rect([x + rw - w, y, w, rh], c, clip);
}

fn at(c: [f32; 2], v: [f32; 2]) -> [f32; 2] { [c[0] + v[0], c[1] + v[1]] }

pub fn heart(d: &mut HudDrawList, r: [f32; 4], color: C, plus: bool, clip: [f32; 4]) {
    let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
    let rad = r[2].min(r[3]) * 0.5;
    let lobe = rad * 0.52;
    // Two lobes and the point, as Godot's outline; filled as two lobes and a
    // wedge so the concave notch stays open.
    for side in [-1.0f32, 1.0] { d.circle(at(c, [side * lobe * 0.92, -rad * 0.28]), lobe, color, clip); }
    let y0 = c[1] - rad * 0.28;
    d.polygon(&[[c[0] - lobe * 1.85, y0 + lobe * 0.18], [c[0] + lobe * 1.85, y0 + lobe * 0.18], [c[0], c[1] + rad * 0.95]], color, clip);
    d.triangle([[c[0] - lobe * 1.6, y0 + lobe * 0.55], [c[0] + lobe * 1.6, y0 + lobe * 0.55], [c[0], y0]], color, clip);
    if plus {
        let (t, l) = (rad * 0.26, rad * 0.62);
        let pc = [c[0], c[1] - rad * 0.05];
        d.rect([pc[0] - l * 0.5, pc[1] - t * 0.5, l, t], BONE, clip);
        d.rect([pc[0] - t * 0.5, pc[1] - l * 0.5, t, l], BONE, clip);
    }
}

pub fn bullet(d: &mut HudDrawList, r: [f32; 4], color: C, clip: [f32; 4]) {
    let w = r[2] * 0.36;
    let x = r[0] + r[2] * 0.5 - w * 0.5;
    let top = r[1] + r[3] * 0.06;
    let bottom = r[1] + r[3] - r[3] * 0.04;
    let tip = top + w * 1.15;
    let mut pts = vec![[x, bottom], [x, tip]];
    for i in 0..9 {
        let a = PI + PI * i as f32 / 8.0;
        pts.push([x + w * 0.5 + a.cos() * w * 0.5, tip + a.sin() * (tip - top)]);
    }
    pts.push([x + w, bottom]);
    d.polygon(&pts, color, clip);
    d.rect([x - w * 0.12, bottom - r[3] * 0.16, w * 1.24, r[3] * 0.16], with_alpha(color, 0.55), clip);
}

pub fn grenade(d: &mut HudDrawList, r: [f32; 4], color: C, clip: [f32; 4]) {
    let s = r[2].min(r[3]);
    let c = [r[0] + r[2] * 0.5 - s * 0.04, r[1] + r[3] * 0.5 + s * 0.1];
    d.circle(c, s * 0.33, color, clip);
    d.rect([c[0] - s * 0.12, c[1] - s * 0.46, s * 0.24, s * 0.14], color, clip);
    d.arc(at(c, [s * 0.2, -s * 0.36]), s * 0.14, PI, TAU * 0.95, (s * 0.07).max(1.5), color, clip);
    let groove = [0.0, 0.0, 0.0, 0.32];
    d.rect([c[0] - s * 0.26, c[1] - s * 0.08, s * 0.52, s * 0.06], groove, clip);
    d.rect([c[0] - s * 0.26, c[1] + s * 0.08, s * 0.52, s * 0.06], groove, clip);
}

/// A small button prompt ("LMB", "RT") hanging off an icon's bottom-right.
pub fn prompt(d: &mut HudDrawList, r: [f32; 4], text: &str, clip: [f32; 4]) {
    let size = (r[3] * 0.42).round().max(9.0);
    let tw = render::hud::measure(text, bold(), size)[0];
    let th = size * 1.0;
    let (bw, bh) = (tw + 6.0, th + 2.0);
    let (bx, by) = (r[0] + r[2] + 3.0 - bw, r[1] + r[3] + 2.0 - bh);
    d.rect([bx, by, bw, bh], RED, clip);
    Label::new(bold(), size, BONE).draw(d, text, bx + 3.0, by + bh * 0.5, clip);
}

/// Card art icons (`card_face.gd`), id to shape.
pub fn card_icon(id: &str) -> &'static str {
    match id {
        "dmg_up" | "headhunter" | "glass_cannon" => "skull",
        "haymaker" => "fist",
        "healthy" | "lifesteal" => "heart",
        "steady_hands" | "precision" | "hitscan" | "sniper" => "crosshair",
        "homing" => "homing",
        "sneakers" | "acrobat" | "dodge" => "wing",
        "rapid_fire" | "uzi" | "minigun" | "spray" | "extra_barrel" | "shotgun" => "bullets",
        "bullet_speed" | "heavy_rounds" | "last_bullet" | "wind_up" | "grow" | "drill_rounds" | "careful_planning" => "bullet",
        "big_mag" | "scavenger" | "tactical_reload" => "mag",
        "quick_reload" | "refreshed" | "echo" => "reload",
        "ricochet" => "bounce",
        "silencer" => "silencer",
        "explosive" | "landmine_rounds" | "backblast" | "bazooka" | "cluster_grenade" | "air_strike" => "bomb",
        "poison" => "drop",
        "chilling_rounds" => "snow",
        "big_head" | "chonky" | "slenderman" | "flatfish" => "body",
        "phoenix" => "flame",
        "teleport" => "swirl",
        "sword" | "ninja" => "sword",
        "ion_cannon" => "beam",
        _ => "star",
    }
}

pub const BODY_CARDS: [&str; 7] = ["big_head", "chonky", "slenderman", "flatfish", "healthy", "glass_cannon", "phoenix"];

fn polyline(d: &mut HudDrawList, pts: &[[f32; 2]], w: f32, c: C, clip: [f32; 4]) {
    for p in pts.windows(2) { d.line(p[0], p[1], w, c, clip); }
    for p in &pts[1..pts.len().saturating_sub(1)] { d.circle(*p, w * 0.5, c, clip); }
}

/// Flat icon shapes in one colour, sized to the square `r`.
pub fn draw_icon(d: &mut HudDrawList, kind: &str, r: [f32; 4], color: C, clip: [f32; 4]) {
    let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
    let s = r[2];
    let line = (s * 0.09).max(2.0);
    let p = |x: f32, y: f32| [c[0] + x * s, c[1] + y * s];
    match kind {
        "skull" => {
            d.circle(p(0.0, -0.08), s * 0.34, color, clip);
            d.rect([c[0] - s * 0.2, c[1] + s * 0.12, s * 0.4, s * 0.24], color, clip);
            let hole = INK;
            d.circle(p(-0.13, -0.06), s * 0.1, hole, clip);
            d.circle(p(0.13, -0.06), s * 0.1, hole, clip);
            d.triangle([p(0.0, 0.06), p(-0.05, 0.15), p(0.05, 0.15)], hole, clip);
            for i in 0..3 { d.rect([c[0] + s * (-0.13 + i as f32 * 0.1), c[1] + s * 0.26, s * 0.04, s * 0.1], hole, clip); }
        }
        "heart" => heart(d, r, color, false, clip),
        "crosshair" => {
            d.arc(c, s * 0.34, 0.0, TAU, line, color, clip);
            for v in [[0.0, -1.0], [0.0, 1.0], [-1.0, 0.0], [1.0, 0.0f32]] { d.line(p(v[0] * 0.16, v[1] * 0.16), p(v[0] * 0.5, v[1] * 0.5), line, color, clip); }
            d.circle(c, s * 0.06, color, clip);
        }
        "homing" => {
            let o = p(-0.05, 0.1);
            d.arc(o, s * 0.3, PI * 0.9, PI * 1.9, line, color, clip);
            let a = PI * 1.9;
            let tip = [o[0] + a.cos() * s * 0.3, o[1] + a.sin() * s * 0.3];
            d.triangle([at(tip, [s * 0.14, s * 0.02]), at(tip, [-s * 0.06, -s * 0.12]), at(tip, [-s * 0.04, s * 0.12])], color, clip);
            d.arc(p(0.28, -0.28), s * 0.12, 0.0, TAU, line * 0.7, color, clip);
        }
        "wing" => {
            for i in 0..4 {
                let y = c[1] - s * 0.3 + i as f32 * s * 0.15;
                let w = s * (0.75 - i as f32 * 0.14);
                let x0 = c[0] - s * 0.38;
                d.polygon(&[[x0, y], [x0 + w, y - s * 0.06], [x0 + w * 0.9, y + s * 0.07], [x0, y + s * 0.1]], color, clip);
            }
        }
        "bullets" => for i in 0..3 {
            bullet(d, [r[0] + s * (0.05 + i as f32 * 0.25), r[1] + s * 0.05 * (1 - i % 2) as f32, s * 0.42, s * 0.9], color, clip);
        },
        "bullet" => {
            // Tilted 0.6 rad, as Godot's transform.
            let (sn, cs) = 0.6f32.sin_cos();
            let mut tmp = HudDrawList::new([1, 1]);
            bullet(&mut tmp, [-s * 0.4, -s * 0.5, s * 0.8, s], color, clip);
            for t in tmp.vertices.chunks(3) {
                let q = |v: [f32; 2]| [c[0] + v[0] * cs - v[1] * sn, c[1] + v[0] * sn + v[1] * cs];
                d.triangle([q(t[0].position), q(t[1].position), q(t[2].position)], t[0].color, clip);
            }
            for i in 0..3 {
                let y = c[1] + s * (0.05 + i as f32 * 0.12);
                d.line([r[0] + s * 0.02, y], [r[0] + s * (0.3 - i as f32 * 0.06), y], line * 0.6, color, clip);
            }
        }
        "mag" => {
            d.polygon(&[p(-0.22, -0.4), p(0.12, -0.4), p(0.3, 0.42), p(-0.04, 0.42)], color, clip);
            for i in 0..4 { d.rect([c[0] + s * (-0.12 + i as f32 * 0.04), c[1] + s * (-0.28 + i as f32 * 0.17), s * 0.22, s * 0.05], INK, clip); }
        }
        "reload" => {
            d.arc(c, s * 0.32, PI * 0.15, PI * 0.95, line * 1.2, color, clip);
            d.arc(c, s * 0.32, PI * 1.15, PI * 1.95, line * 1.2, color, clip);
            for a in [PI * 0.95, PI * 1.95] {
                let q = [c[0] + a.cos() * s * 0.32, c[1] + a.sin() * s * 0.32];
                let (t, n) = ([-a.sin(), a.cos()], [a.cos(), a.sin()]);
                d.triangle([at(q, [t[0] * s * 0.16, t[1] * s * 0.16]), at(q, [n[0] * s * 0.13, n[1] * s * 0.13]), at(q, [-n[0] * s * 0.13, -n[1] * s * 0.13])], color, clip);
            }
        }
        "bounce" => {
            let pts = [p(-0.45, -0.3), p(-0.1, 0.3), p(0.2, -0.15), p(0.42, 0.2)];
            polyline(d, &pts, line, color, clip);
            d.rect([c[0] - s * 0.5, c[1] + s * 0.34, s, s * 0.08], color, clip);
            d.circle(pts[3], s * 0.08, color, clip);
        }
        "silencer" => {
            d.rect([c[0] - s * 0.48, c[1] - s * 0.12, s * 0.3, s * 0.16], color, clip);
            d.rect([c[0] - s * 0.2, c[1] - s * 0.2, s * 0.66, s * 0.32], color, clip);
            for i in 0..4 { d.rect([c[0] + s * (-0.1 + i as f32 * 0.14), c[1] - s * 0.2, s * 0.03, s * 0.32], INK, clip); }
            d.rect([c[0] - s * 0.46, c[1] + s * 0.04, s * 0.12, s * 0.26], color, clip);
        }
        "bomb" => {
            d.circle(p(-0.05, 0.08), s * 0.3, color, clip);
            d.rect([c[0] + s * 0.05, c[1] - s * 0.3, s * 0.16, s * 0.14], color, clip);
            d.line(p(0.14, -0.3), p(0.3, -0.42), line * 0.7, color, clip);
            for i in 0..6 {
                let a = TAU * i as f32 / 6.0;
                let o = p(0.36, -0.44);
                d.line(at(o, [a.cos() * s * 0.05, a.sin() * s * 0.05]), at(o, [a.cos() * s * 0.13, a.sin() * s * 0.13]), line * 0.5, color, clip);
            }
        }
        "drop" => {
            let mut pts = vec![p(0.0, -0.46)];
            for i in 0..15 {
                let a = -PI * 0.15 + PI * 1.3 * i as f32 / 14.0;
                pts.push(at(p(0.0, 0.12), [a.cos() * s * 0.3, a.sin() * s * 0.3]));
            }
            d.polygon(&pts, color, clip);
        }
        "snow" => for i in 0..3 {
            let a = PI * i as f32 / 3.0;
            let v = [a.cos() * s * 0.46, a.sin() * s * 0.46];
            d.line([c[0] - v[0], c[1] - v[1]], [c[0] + v[0], c[1] + v[1]], line, color, clip);
            for e in [[v[0] * 0.6, v[1] * 0.6], [-v[0] * 0.6, -v[1] * 0.6]] {
                let len = (e[0] * e[0] + e[1] * e[1]).sqrt().max(1e-3);
                let n = [-e[1] / len * s * 0.1, e[0] / len * s * 0.1];
                let base = at(c, e);
                let tip = at(c, [e[0] * 1.35, e[1] * 1.35]);
                d.line(base, at(tip, n), line * 0.7, color, clip);
                d.line(base, at(tip, [-n[0], -n[1]]), line * 0.7, color, clip);
            }
        },
        "body" => {
            d.circle(p(0.0, -0.28), s * 0.15, color, clip);
            d.polygon(&[p(-0.3, -0.08), p(0.3, -0.08), p(0.22, 0.46), p(-0.22, 0.46)], color, clip);
        }
        "flame" => for layer in 0..2 {
            let k = 1.0 - layer as f32 * 0.45;
            let base = p(0.0, 0.16);
            let mut pts = Vec::new();
            for i in 0..13 { let a = PI * i as f32 / 12.0; pts.push(at(base, [a.cos() * s * 0.28 * k, a.sin() * s * 0.28 * k])); }
            for v in [[-0.3, -0.2], [-0.12, -0.42], [0.08, -0.62], [0.06, -0.38], [0.24, -0.3f32]] { pts.push(at(base, [v[0] * s * k, v[1] * s * k])); }
            d.polygon(&pts, if layer == 0 { color } else { with_alpha(INK, 0.55) }, clip);
        },
        "swirl" => {
            let pts: Vec<[f32; 2]> = (0..50).map(|i| { let t = i as f32 / 49.0; let a = t * TAU * 2.2; at(c, [a.cos() * s * (0.04 + 0.42 * t), a.sin() * s * (0.04 + 0.42 * t)]) }).collect();
            polyline(d, &pts, line, color, clip);
        }
        "sword" => {
            d.polygon(&[p(0.42, -0.42), p(0.3, -0.12), p(-0.12, 0.2), p(-0.2, 0.12)], color, clip);
            d.line(p(-0.3, 0.06), p(-0.06, 0.3), line * 1.3, color, clip);
            d.line(p(-0.16, 0.2), p(-0.38, 0.42), line * 1.5, color, clip);
        }
        "beam" => {
            d.rect([c[0] - s * 0.1, c[1] - s * 0.5, s * 0.2, s * 0.8], color, clip);
            d.arc(p(0.0, 0.34), s * 0.22, PI, TAU, line, color, clip);
            d.arc(p(0.0, 0.34), s * 0.36, PI * 1.1, PI * 1.9, line * 0.7, with_alpha(color, 0.6), clip);
        }
        "fist" => {
            d.rect([c[0] - s * 0.3, c[1] - s * 0.2, s * 0.56, s * 0.44], color, clip);
            for i in 0..4 { d.circle(p(-0.22 + i as f32 * 0.14, -0.2), s * 0.08, color, clip); }
            d.rect([c[0] - s * 0.2, c[1] + s * 0.24, s * 0.36, s * 0.2], color, clip);
            d.line(p(-0.24, 0.02), p(0.1, 0.02), line * 0.5, INK, clip);
        }
        _ => {
            let pts: Vec<[f32; 2]> = (0..10).map(|i| { let a = -PI * 0.5 + TAU * i as f32 / 10.0; let k = if i % 2 == 0 { 0.46 } else { 0.2 }; at(c, [a.cos() * s * k, a.sin() * s * k]) }).collect();
            d.polygon(&pts, color, clip);
        }
    }
}
