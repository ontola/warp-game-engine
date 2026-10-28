//! The TV overlay (src/hud/hud.gd): one panel per wizard in its corner with
//! the health orb, the queue, mana and the face buttons; each wizard's name
//! under them on screen; the level's name card; banners, toasts, the fade
//! and the death words. Laid out on Godot's 1920x1080 canvas and scaled.
use glam::{Mat4, Vec2, Vec3, Vec4};
use render::hud::{FontFace, HudDrawList, HudImage, ImageFilter, TextAlign, TextRun};
use render::picture::Picture;
use render::Camera;
use spellstorm::el::{self, El};
use spellstorm::fx::{Ev, C};
use spellstorm::grammar::Form;
use spellstorm::wizard::Pad;
use spellstorm::World;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

static CINZEL: &[u8] = include_bytes!("../assets/fonts/Cinzel.ttf");
static ALEGREYA: &[u8] = include_bytes!("../assets/fonts/Alegreya.ttf");
/// Both ship under the SIL OFL 1.1; keep these with distributions.
#[allow(dead_code)]
pub const FONT_LICENSES: [&str; 2] = [include_str!("../assets/fonts/OFL-Cinzel.txt"), include_str!("../assets/fonts/OFL-Alegreya.txt")];

/// Art.title_font (Cinzel) and Art.body_font (Alegreya).
pub fn title() -> FontFace {
    render::hud::register_font(CINZEL)
}
pub fn body() -> FontFace {
    render::hud::register_font(ALEGREYA)
}

const VIGNETTE: HudImage = HudImage(0x5ee1_0002);
const PANEL: Vec2 = Vec2::new(312.0, 108.0);
const MARGIN: f32 = 18.0;
const NO_CLIP: [f32; 4] = [-1e5, -1e5, 2e5, 2e5];
const DEATH_WORDS: [&str; 4] = ["YOU DONE GOOFED", "RIP IN PEACE", "YOU DIED TO DEATH", "MISSION FAILED\nWE'LL GET 'EM NEXT TIME"];

fn lighten(c: C, t: f32) -> C {
    [c[0] + (1.0 - c[0]) * t, c[1] + (1.0 - c[1]) * t, c[2] + (1.0 - c[2]) * t, c[3]]
}
fn darken(c: C, t: f32) -> C {
    [c[0] * (1.0 - t), c[1] * (1.0 - t), c[2] * (1.0 - t), c[3]]
}
fn alpha(c: C, a: f32) -> C {
    [c[0], c[1], c[2], c[3] * a]
}
fn el_c(e: El) -> C {
    let c = e.color();
    [c[0], c[1], c[2], 1.0]
}
fn bytes(c: C) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// A timed fade: value now, where it goes and how fast.
#[derive(Clone, Copy, Default)]
struct Tween {
    from: f32,
    to: f32,
    t: f32,
    time: f32,
    delay: f32,
}

impl Tween {
    fn go(&mut self, to: f32, time: f32, delay: f32) {
        self.from = self.now();
        self.to = to;
        self.t = 0.0;
        self.time = time.max(1e-3);
        self.delay = delay;
    }
    fn step(&mut self, dt: f32) {
        self.t += dt;
    }
    fn now(&self) -> f32 {
        if self.time == 0.0 {
            return self.to;
        }
        let k = ((self.t - self.delay) / self.time).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * k
    }
}

pub struct Hud {
    banner: (String, String, C),
    banner_t: f32,
    toasts: Vec<(String, C, f32)>,
    fade: Tween,
    card: String,
    card_t: f32,
    death: Option<(String, f32)>,
    death_end: Option<f32>,
    vignette: Arc<Picture>,
    words: usize,
}

impl Default for Hud {
    fn default() -> Self {
        Hud {
            banner: Default::default(),
            banner_t: 0.0,
            toasts: vec![],
            fade: Tween::default(),
            card: String::new(),
            card_t: 99.0,
            death: None,
            death_end: None,
            vignette: Arc::new(vignette()),
            words: 0,
        }
    }
}

/// The vignette's darkening (hud.gd VIGNETTE), as alpha.
fn vignette() -> Picture {
    let (w, h) = (192u32, 108u32);
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let u = ((x as f32 + 0.5) / w as f32 - 0.5) * 1.6;
            let v = (y as f32 + 0.5) / h as f32 - 0.5;
            let d = (u * u + v * v).sqrt();
            let t = ((d - 0.45) / 0.6).clamp(0.0, 1.0);
            let a = t * t * (3.0 - 2.0 * t);
            rgba[((y * w + x) * 4 + 3) as usize] = (a * 255.0) as u8;
        }
    }
    Picture { width: w, height: h, rgba }
}

impl Hud {
    /// The level's name card (Hud.level, _level_card).
    pub fn level(&mut self, w: &World) {
        let b = w.level.biome();
        self.card = format!("{}: {}", w.run.biome_index + 1, b.name);
        self.card_t = 0.0;
    }

    pub fn absorb(&mut self, w: &World) {
        for e in &w.events {
            match e {
                Ev::Banner { title, sub, c } => {
                    self.banner = (title.clone(), sub.clone(), *c);
                    self.banner_t = 4.0;
                }
                Ev::Toast { text, c } => {
                    self.toasts.push((text.clone(), *c, 0.0));
                    if self.toasts.len() > 4 {
                        self.toasts.remove(0);
                    }
                }
                Ev::Fade { to, time } => self.fade.go(*to, *time, 0.0),
                Ev::YouDied => {
                    self.words += 1;
                    self.death = Some((DEATH_WORDS[(w.tick as usize + self.words) % DEATH_WORDS.len()].to_string(), 0.0));
                    self.death_end = None;
                }
                Ev::YouDiedEnd => self.death_end = Some(0.0),
                Ev::Level { .. } => self.level(w),
                _ => {}
            }
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.banner_t = (self.banner_t - dt).max(0.0);
        for t in &mut self.toasts {
            t.2 += dt;
        }
        self.toasts.retain(|t| t.2 < 5.0);
        self.fade.step(dt);
        self.card_t += dt;
        if let Some(d) = &mut self.death {
            d.1 += dt;
        }
        if let Some(e) = &mut self.death_end {
            *e += dt;
            if *e > 0.8 {
                self.death = None;
                self.death_end = None;
            }
        }
    }

    pub fn draw(&self, d: &mut HudDrawList, w: &World, cam: &Camera, size: [u32; 2]) {
        let s = size[1] as f32 / 1080.0;
        let sw = size[0] as f32 / s;
        let mut p = Painter { d, s };
        p.d.provide_image(VIGNETTE, &self.vignette, ImageFilter::Linear);
        // Hurt: the edge reddens as anyone runs low.
        let mut hurt: f32 = 0.0;
        for &id in &w.wizards {
            let a = w.a(id);
            if a.alive {
                hurt = hurt.max(1.0 - (a.hp / a.max_hp) as f32 * 2.5);
            }
        }
        let hurt = hurt.clamp(0.0, 1.0) * 0.6;
        p.d.image([0.0, 0.0, size[0] as f32, size[1] as f32], VIGNETTE, [0.0, 0.0, 1.0, 1.0], [0.35 * hurt, 0.0, 0.0, 0.85 + hurt * 0.3], NO_CLIP);
        // Names under the wizards' feet, under the rest of the HUD.
        let vp = view_proj(cam, size);
        for &id in &w.wizards {
            let a = w.a(id);
            let wz = w.wiz(id);
            let at = a.pos + Vec3::new(0.0, 0.0, 0.6);
            let c = vp * Vec4::new(at.x, at.y, at.z, 1.0);
            if c.w <= 0.0 {
                continue;
            }
            let x = (c.x / c.w * 0.5 + 0.5) * size[0] as f32 / s;
            let y = (0.5 - c.y / c.w * 0.5) * size[1] as f32 / s;
            p.text(&wz.name, x, y + 14.0, body(), 26.0, lighten(wz.robe, 0.4), TextAlign::Center, 9.0);
        }
        p.d.begin_overlay();
        for (i, &id) in w.wizards.iter().enumerate() {
            let right = i % 2 == 1;
            let top = i >= 2;
            let x = if right { sw - MARGIN - PANEL.x } else { MARGIN };
            let y = if top { MARGIN } else { 1080.0 - MARGIN - PANEL.y };
            panel(&mut p, w, id, Vec2::new(x, y), right);
        }
        // Toasts at the top in the middle.
        for (k, (text, c, t)) in self.toasts.iter().enumerate() {
            let a = if *t < 4.0 { 1.0 } else { 1.0 - (t - 4.0) };
            p.text(text, sw * 0.5, 20.0 + 18.0 + k as f32 * 34.0, body(), 26.0, alpha(*c, a), TextAlign::Center, 8.0);
        }
        // Banner.
        if self.banner_t > 0.0 {
            let bt = self.banner_t;
            let a = if bt < 0.8 { bt / 0.8 } else { ((4.0 - bt) / 0.4).clamp(0.0, 1.0) };
            p.text(&self.banner.0, sw * 0.5, 360.0, title(), 96.0, alpha(self.banner.2, a), TextAlign::Center, 18.0);
            p.text(&self.banner.1, sw * 0.5, 450.0, body(), 40.0, alpha([0.85, 0.8, 0.7, 1.0], a), TextAlign::Center, 10.0);
        }
        // The level's name: in over the dark, a hold, out while it grows.
        let t = self.card_t;
        if t < 5.1 && !self.card.is_empty() {
            let a = if t < 0.9 { 0.0 } else if t < 2.1 { (t - 0.9) / 1.2 } else if t < 3.7 { 1.0 } else { 1.0 - (t - 3.7) / 1.4 };
            let a = a.clamp(0.0, 1.0);
            let grow = 0.97 + 0.07 * ((t - 0.9) / 4.2).clamp(0.0, 1.0);
            let cy = 540.0;
            let col = [0.86, 0.8, 0.68, a];
            p.shadow_text(&self.card, sw * 0.5, cy - 20.0, title(), 96.0 * grow, col);
            p.rect([sw * 0.5 - 380.0 * grow, cy + 44.0 * grow, 760.0 * grow, 2.0], [0.86, 0.8, 0.68, 0.55 * a]);
        }
        // Black for portals and for the end of a run, over everything.
        p.d.begin_overlay();
        let f = self.fade.now();
        if f > 0.0 {
            p.rect([0.0, 0.0, sw, 1080.0], [0.0, 0.0, 0.0, f]);
        }
        if let Some((words, t)) = &self.death {
            let out = self.death_end.map(|e| 1.0 - e / 0.8).unwrap_or(1.0).clamp(0.0, 1.0);
            let black = (t / 1.4).min(1.0) * out;
            p.rect([0.0, 0.0, sw, 1080.0], [0.0, 0.0, 0.0, black]);
            let a = ((t - 0.5) / 1.6).clamp(0.0, 1.0) * out;
            let grow = 0.94 + 0.18 * (t / 6.0).min(1.0);
            let size = if words.len() < 20 { 140.0 } else { 100.0 } * grow;
            let lines: Vec<&str> = words.lines().collect();
            for (k, l) in lines.iter().enumerate() {
                let y = 540.0 + (k as f32 - (lines.len() as f32 - 1.0) * 0.5) * size * 1.1;
                p.text(l, sw * 0.5, y, title(), size, [0.62, 0.04, 0.03, a], TextAlign::Center, 0.0);
            }
        }
    }
}

/// The camera's view and projection, as the renderer builds them.
fn view_proj(cam: &Camera, size: [u32; 2]) -> Mat4 {
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    Mat4::perspective_rh(cam.fov_y, aspect, cam.near, 1000.0) * Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y)
}

/// Draws on Godot's canvas: positions in 1080-high units, scaled by `s`.
pub(crate) struct Painter<'a> {
    pub d: &'a mut HudDrawList,
    pub s: f32,
}

impl Painter<'_> {
    fn v(&self, p: Vec2) -> [f32; 2] {
        [p.x * self.s, p.y * self.s]
    }
    fn rect(&mut self, r: [f32; 4], c: C) {
        let s = self.s;
        self.d.rect([r[0] * s, r[1] * s, r[2] * s, r[3] * s], c, NO_CLIP);
    }
    fn round_rect(&mut self, r: [f32; 4], radius: f32, c: C) {
        let [x, y, w, h] = r;
        let k = radius.min(w * 0.5).min(h * 0.5);
        self.rect([x + k, y, w - 2.0 * k, h], c);
        self.rect([x, y + k, k, h - 2.0 * k], c);
        self.rect([x + w - k, y + k, k, h - 2.0 * k], c);
        for (cx, cy, a0) in [(x + k, y + k, PI), (x + w - k, y + k, PI * 1.5), (x + w - k, y + h - k, 0.0), (x + k, y + h - k, PI * 0.5)] {
            let mut pts = vec![self.v(Vec2::new(cx, cy))];
            for i in 0..=8 {
                let a = a0 + i as f32 / 8.0 * PI * 0.5;
                pts.push(self.v(Vec2::new(cx + a.cos() * k, cy + a.sin() * k)));
            }
            self.d.polygon(&pts, c, NO_CLIP);
        }
    }
    fn circle(&mut self, c: Vec2, r: f32, col: C) {
        let s = self.s;
        self.d.circle(self.v(c), r * s, col, NO_CLIP);
    }
    fn arc(&mut self, c: Vec2, r: f32, from: f32, to: f32, width: f32, col: C) {
        let s = self.s;
        self.d.arc(self.v(c), r * s, from, to, width * s, col, NO_CLIP);
    }
    fn line(&mut self, a: Vec2, b: Vec2, width: f32, col: C) {
        let s = self.s;
        self.d.line(self.v(a), self.v(b), (width * s).max(1.0), col, NO_CLIP);
    }
    fn poly(&mut self, pts: &[Vec2], col: C) {
        let pts: Vec<[f32; 2]> = pts.iter().map(|&p| self.v(p)).collect();
        self.d.polygon(&pts, col, NO_CLIP);
    }
    /// One line centred on `y`, with Godot's dark outline of `outline` px.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, text: &str, x: f32, y: f32, font: FontFace, size: f32, c: C, align: TextAlign, outline: f32) {
        if text.is_empty() || c[3] <= 0.003 {
            return;
        }
        let s = self.s;
        let px = size * s;
        let w = render::hud::measure(text, font, px)[0] + 4.0;
        let h = px * 1.2;
        let left = match align {
            TextAlign::Left => x * s,
            TextAlign::Center => x * s - w * 0.5,
            TextAlign::Right => x * s - w,
        };
        let top = y * s - h * 0.5;
        let run = |d: &mut HudDrawList, dx: f32, dy: f32, c: C| {
            d.text.push(TextRun { id: 0x5ee1_0000_0000 + d.text.len() as u64, text: text.into(), position: [left + dx, top + dy], bounds: NO_CLIP, font_size: px, color: bytes(c), font, align, width: w });
        };
        if outline > 0.0 {
            let o = outline * s * 0.5;
            let oc = [0.0, 0.0, 0.0, 0.9 * c[3]];
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, -1.4), (0.0, 1.4), (-1.4, 0.0), (1.4, 0.0)] {
                run(self.d, dx * o, dy * o, oc);
            }
        }
        run(self.d, 0.0, 0.0, c);
    }
    /// Centred, with a soft shadow below (the level card).
    fn shadow_text(&mut self, text: &str, x: f32, y: f32, font: FontFace, size: f32, c: C) {
        for (k, a) in [(6.0, 0.3), (4.0, 0.5)] {
            self.text(text, x, y + k, font, size, [0.0, 0.0, 0.0, a * c[3]], TextAlign::Center, 12.0);
        }
        self.text(text, x, y, font, size, c, TextAlign::Center, 0.0);
    }
}

/// An element as a little ball (ElementBall: its colour, lit from the top
/// left, with its icon).
fn ball(p: &mut Painter, e: El, c: Vec2, r: f32, dim: f32) {
    let col = el_c(e);
    let base = alpha(darken(col, 0.25), dim.min(1.0));
    p.circle(c, r, alpha(darken(col, 0.55), dim.min(1.0)));
    p.circle(c, r * 0.92, base);
    p.circle(c + Vec2::new(-r * 0.15, -r * 0.18), r * 0.62, alpha(lighten(col, 0.1 * dim), dim.min(1.0)));
    p.circle(c + Vec2::new(-r * 0.32, -r * 0.36), r * 0.22, [1.0, 1.0, 1.0, 0.25 * dim.min(1.0)]);
    icon(p, e, c, r * 0.6, [0.05, 0.03, 0.03, 0.85 * dim.min(1.0)]);
}

/// Icons.draw: each element as simple shapes.
pub(crate) fn icon(p: &mut Painter, e: El, c: Vec2, r: f32, col: C) {
    let w = (r * 0.16).max(1.5);
    let v = |x: f32, y: f32| c + Vec2::new(x, y);
    match e {
        El::Fire => {
            let (mut outer, mut inner) = (vec![], vec![]);
            for i in 0..24 {
                let t = i as f32 / 24.0 * TAU;
                let (x, y) = (t.sin(), t.cos());
                let pinch = 1.0 - 0.55 * (-y).max(0.0);
                outer.push(v(x * r * 0.62 * pinch + (-y).max(0.0) * r * 0.12, -y * r * 0.95 + r * 0.1));
                inner.push(v(x * r * 0.3 * pinch, -y * r * 0.5 + r * 0.38));
            }
            p.poly(&outer, col);
            p.poly(&inner, lighten(col, 0.55));
        }
        El::Frost => {
            for i in 0..6 {
                let a = i as f32 * TAU / 6.0;
                let d = Vec2::new(a.sin(), -a.cos());
                p.line(c, c + d * r, w, col);
                let mid = c + d * r * 0.55;
                for s in [-1.0f32, 1.0] {
                    p.line(mid, mid + Vec2::from_angle(s * 0.8).rotate(d) * r * 0.32, w * 0.8, col);
                }
            }
        }
        El::Water => {
            let mut pts: Vec<Vec2> = (0..20).map(|i| {
                let t = i as f32 / 19.0 * PI;
                v((t + PI).cos() * r * 0.62, r * 0.3 + t.sin() * r * 0.62)
            }).collect();
            pts.push(v(0.0, -r));
            p.poly(&pts, col);
            p.circle(v(-r * 0.22, r * 0.25), r * 0.13, lighten(col, 0.6));
        }
        El::Lightning => bolt_icon(p, c, r, col),
        El::Earth => rock_icon(p, c, r, col),
        El::Air => {
            for (i, k) in [0.9f32, 1.2, 0.7].iter().enumerate() {
                let y = c.y + (i as f32 - 1.0) * r * 0.55;
                let x0 = c.x - r * 0.85;
                let x1 = x0 + r * 1.3 * k;
                p.line(Vec2::new(x0, y), Vec2::new(x1, y), w, col);
                p.arc(Vec2::new(x1, y - r * 0.18), r * 0.18, PI * 0.5, PI * 2.0, w, col);
            }
        }
        El::Poison => {
            p.circle(v(0.0, -r * 0.12), r * 0.62, col);
            p.rect([c.x - r * 0.36, c.y + r * 0.2, r * 0.72, r * 0.5], col);
            let hole = darken(col, 0.8);
            p.circle(v(-r * 0.24, -r * 0.1), r * 0.17, hole);
            p.circle(v(r * 0.24, -r * 0.1), r * 0.17, hole);
            for i in 0..3 {
                let x = (i as f32 - 1.0) * r * 0.2;
                p.line(v(x, r * 0.42), v(x, r * 0.7), (r * 0.07).max(1.0), hole);
            }
        }
        El::Void => {
            let mut pts = vec![];
            for i in 0..17 {
                let t = i as f32 / 16.0;
                pts.push(v((t - 0.5) * r * 1.9, -(t * PI).sin() * r * 0.62));
            }
            for i in 0..17 {
                let t = 1.0 - i as f32 / 16.0;
                pts.push(v((t - 0.5) * r * 1.9, (t * PI).sin() * r * 0.62));
            }
            p.poly(&pts, col);
            p.circle(c, r * 0.36, darken(col, 0.8));
            p.circle(c, r * 0.14, lighten(col, 0.5));
        }
        El::Steam => {
            for i in 0..3 {
                let x = c.x + (i as f32 - 1.0) * r * 0.55;
                let pts: Vec<Vec2> = (0..9).map(|k| {
                    let t = k as f32 / 8.0;
                    Vec2::new(x + (t * TAU).sin() * r * 0.16, c.y + r * 0.8 - t * r * 1.6)
                }).collect();
                for s in pts.windows(2) {
                    p.line(s[0], s[1], w, col);
                }
            }
        }
        El::Ice => {
            p.poly(&[v(0.0, -r), v(r * 0.6, 0.0), v(0.0, r), v(-r * 0.6, 0.0)], col);
            p.line(v(0.0, -r * 0.7), v(0.0, r * 0.7), w * 0.6, darken(col, 0.5));
        }
        El::Mud => {
            p.circle(v(0.0, r * 0.15), r * 0.62, col);
            p.circle(v(-r * 0.55, r * 0.55), r * 0.22, col);
            p.circle(v(r * 0.6, r * 0.6), r * 0.16, col);
        }
        El::Magma => {
            rock_icon(p, c, r, col);
            p.line(v(-r * 0.3, -r * 0.4), v(r * 0.05, r * 0.05), w, [1.0, 0.55, 0.1, col[3]]);
            p.line(v(r * 0.05, r * 0.05), v(r * 0.35, r * 0.45), w, [1.0, 0.55, 0.1, col[3]]);
        }
        El::Storm => {
            p.circle(v(-r * 0.35, -r * 0.25), r * 0.38, col);
            p.circle(v(r * 0.25, -r * 0.35), r * 0.45, col);
            p.rect([c.x - r * 0.7, c.y - r * 0.3, r * 1.4, r * 0.35], col);
            bolt_icon(p, c + Vec2::new(0.0, r * 0.45), r * 0.55, col);
        }
    }
}

fn bolt_icon(p: &mut Painter, c: Vec2, r: f32, col: C) {
    // Two convex halves, so the fan fill keeps the zigzag.
    let v = |x: f32, y: f32| c + Vec2::new(x * r, y * r);
    p.poly(&[v(0.25, -1.0), v(-0.5, 0.12), v(-0.02, 0.12), v(0.05, -0.2)], col);
    p.poly(&[v(-0.02, 0.12), v(-0.3, 1.0), v(0.52, -0.2), v(0.05, -0.2)], col);
}

fn rock_icon(p: &mut Painter, c: Vec2, r: f32, col: C) {
    let v = |x: f32, y: f32| c + Vec2::new(x * r, y * r);
    p.poly(&[v(-0.85, 0.55), v(-0.7, -0.2), v(-0.2, -0.75), v(0.45, -0.6), v(0.85, -0.05), v(0.75, 0.55)], col);
    let dk = darken(col, 0.45);
    p.line(v(-0.2, -0.75), v(-0.05, 0.1), (r * 0.1).max(1.0), dk);
    p.line(v(-0.05, 0.1), v(0.4, 0.3), (r * 0.1).max(1.0), dk);
}

/// Health orb, Diablo style: a glass globe that drains.
fn orb(p: &mut Painter, c: Vec2, r: f32, frac: f32, fill: C, rim: C, bg: C) {
    p.circle(c, r + 3.0, rim);
    p.circle(c, r, bg);
    let frac = frac.clamp(0.0, 1.0);
    if frac > 0.0 {
        let top = c.y + r - 2.0 * r * frac;
        let pts: Vec<Vec2> = (0..33).map(|i| {
            let a = PI * 0.5 + (i as f32 / 32.0 - 0.5) * TAU;
            let q = c + Vec2::new(a.cos(), a.sin()) * r;
            Vec2::new(q.x, q.y.max(top))
        }).collect();
        p.poly(&pts, fill);
    }
    p.circle(c + Vec2::new(-r * 0.3, -r * 0.35), r * 0.25, [1.0, 1.0, 1.0, 0.12]);
}

const FORMS: [(&str, Form); 4] = [("RT", Form::Cast), ("RB", Form::Nova), ("LS", Form::Plant), ("RS", Form::Enchant)];

/// One wizard's panel (Hud.WizardPanel._draw). Mirrored on the right, so
/// the orb is always on the outside.
fn panel(p: &mut Painter, w: &World, id: spellstorm::world::Id, at: Vec2, mirrored: bool) {
    let a = w.a(id);
    let wz = w.wiz(id);
    let x = |v: f32| at.x + if mirrored { PANEL.x - v } else { v };
    let y = |v: f32| at.y + v;
    let tf = title();
    let bf = body();
    p.round_rect([at.x, at.y, PANEL.x, PANEL.y], 12.0, [0.03, 0.02, 0.02, 0.5]);
    let frac = if a.alive { (a.hp / a.max_hp).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let hc = Vec2::new(x(32.0), y(36.0));
    orb(p, hc, 22.0, frac, [0.62, 0.04, 0.04, 1.0], [0.3, 0.24, 0.16, 1.0], [0.08, 0.02, 0.02, 1.0]);
    if wz.enchant_time > 0.0 {
        if let Some(e) = wz.enchant.as_ref().and_then(|s| s.primary) {
            let sk = wz.enchant_left() as f32;
            p.arc(hc, 27.0, -PI * 0.5, -PI * 0.5 + TAU * sk, 3.0, lighten(el_c(e), 0.4));
        }
    }
    let hp = if a.alive { format!("{}", a.hp.ceil() as i64) } else { "✝".into() };
    p.text(&hp, hc.x, hc.y + 1.0, tf, 15.0, [1.0, 0.92, 0.85, 1.0], TextAlign::Center, 0.0);
    // Name, then the queue: three sockets, filled in order.
    let sx0 = x(74.0);
    let align = if mirrored { TextAlign::Right } else { TextAlign::Left };
    let nx = if mirrored { sx0 + 9.0 } else { sx0 - 9.0 };
    p.text(&wz.name, nx, y(13.0), tf, 15.0, lighten(wz.robe, 0.45), align, 0.0);
    let step = if mirrored { -22.0 } else { 22.0 };
    let slots = &wz.queue.slots;
    let pays = wz.affordable(Form::Cast);
    for i in 0..spellstorm::queue::SIZE {
        let oc = Vec2::new(sx0 + i as f32 * step, y(36.0));
        if i < wz.queue.size {
            p.circle(oc, 9.5, [0.2, 0.16, 0.12, 0.8]);
        }
        if let Some(&e) = slots.get(i) {
            let left_out = pays > 0 && i >= pays;
            ball(p, e, oc, 8.5, if left_out { 0.45 } else { 1.0 });
            if e.is_fused() {
                p.arc(oc, 10.5, 0.0, TAU, 1.2, [1.0, 0.9, 0.6, 1.0]);
            }
        }
    }
    if !slots.is_empty() {
        let k = (wz.ammo as f32 / (wz.full_ammo() as f32).max(1.0)).clamp(0.0, 1.0);
        p.arc(Vec2::new(sx0, y(36.0)), 11.5, -PI * 0.5, -PI * 0.5 + TAU * k, 1.6, [1.0, 0.88, 0.5, 1.0]);
    }
    let bw = (spellstorm::queue::SIZE as f32 - 1.0) * 22.0 + 20.0;
    let bx = if mirrored { sx0 - bw + 10.0 } else { sx0 - 10.0 };
    // A running stream drains a bar over the sockets until it ends.
    if let Some((left, col)) = channel_left(w, wz.channel) {
        p.rect([bx, y(30.0), bw, 12.0], [0.12, 0.09, 0.07, 1.0]);
        let fw = bw * left;
        let fx = if mirrored { bx + bw - fw } else { bx };
        p.rect([fx, y(30.0), fw, 12.0], col);
        p.rect([fx, y(30.0), fw, 4.0], lighten(col, 0.45));
    }
    // Mana: a thin bar under the queue, as long as the mana you can hold.
    let mlen = bw * (wz.mana_max() / spellstorm::balance::MANA_MAX) as f32;
    let mx0 = bx + if mirrored { bw - mlen } else { 0.0 };
    let mk = (wz.mana / wz.mana_max()).clamp(0.0, 1.0) as f32;
    p.rect([mx0, y(52.0), mlen, 5.0], if wz.no_mana > 0.0 { [0.5, 0.65, 1.0, 0.5] } else { [0.02, 0.02, 0.08, 0.9] });
    let mw = mlen * mk;
    let mx = if mirrored { mx0 + mlen - mw } else { mx0 };
    p.rect([mx, y(52.0), mw, 5.0], [0.2, 0.38, 1.0, 1.0]);
    if !a.alive {
        p.text("Stand at the grave", if mirrored { x(16.0) } else { x(16.0) }, y(80.0), bf, 13.0, [0.85, 0.3, 0.25, 1.0], align, 0.0);
        return;
    }
    buttons(p, &wz, Vec2::new(x(222.0), y(36.0)));
    // What each cast button makes of the queue, two by two.
    if !slots.is_empty() {
        let names: Vec<(&str, String)> = FORMS.iter().filter(|f| wz.form_open(f.1)).map(|f| (f.0, wz.preview(f.1).name)).collect();
        for (i, (b, name)) in names.iter().enumerate() {
            let cx = 14.0 + (i % 2) as f32 * 146.0;
            let cy = 70.0 + (i / 2) as f32 * 19.0;
            let chip_x = x(cx) - if mirrored { 22.0 } else { 0.0 };
            p.round_rect([chip_x, y(cy), 22.0, 14.0], 4.0, [0.85, 0.75, 0.55, 0.85]);
            p.text(b, chip_x + 11.0, y(cy + 7.0), bf, 10.0, [0.08, 0.05, 0.03, 1.0], TextAlign::Center, 0.0);
            let tx = if mirrored { chip_x - 4.0 } else { chip_x + 27.0 };
            p.text(name, tx, y(cy + 7.0), bf, 11.0, [0.92, 0.86, 0.74, 0.9], align, 0.0);
        }
    }
}

/// How much of a running stream is left, and its colour.
fn channel_left(w: &World, ch: Option<spellstorm::world::Id>) -> Option<(f32, C)> {
    use spellstorm::spells::{Beam, Spray};
    let id = ch?;
    if let Ok(s) = w.ecs.get::<&Spray>(id) {
        return Some(((1.0 - s.t / s.spell.n.duration.max(0.01)).clamp(0.0, 1.0) as f32, spellstorm::spells::color(&s.spell)));
    }
    if let Ok(b) = w.ecs.get::<&Beam>(id) {
        return Some(((1.0 - b.t / b.spell.n.duration.max(0.01)).clamp(0.0, 1.0) as f32, spellstorm::spells::color(&b.spell)));
    }
    None
}

/// The four face buttons where they sit on the pad, each with the element
/// it conjures; holding LB flips them to their pairs.
fn buttons(p: &mut Painter, wz: &spellstorm::wizard::Wizard, c: Vec2) {
    let flip = wz.pad.down(Pad::LB) && wz.lb_open();
    let places = [(el::PAIR_Y, Pad::Y, Vec2::new(0.0, -15.0)), (el::PAIR_X, Pad::X, Vec2::new(-15.0, 0.0)), (el::PAIR_B, Pad::B, Vec2::new(15.0, 0.0)), (el::PAIR_A, Pad::A, Vec2::new(0.0, 15.0))];
    for (pair, bit, off) in places {
        let e = pair[if flip { 1 } else { 0 }];
        let q = c + off;
        let held = wz.pad.down(bit);
        let r = if held { 9.0 } else { 7.5 };
        p.circle(q, r + 1.5, [0.04, 0.03, 0.03, 1.0]);
        if !wz.element_open(e) {
            p.circle(q, 6.0, [0.22, 0.2, 0.19, 1.0]);
            continue;
        }
        ball(p, e, q, r, if held { 1.15 } else { 0.85 });
        if wz.learned.contains(&e) && wz.learned_t > 0.0 {
            let pulse = 0.5 + 0.5 * (wz.learned_t as f32 * 7.0).sin();
            p.arc(q, r + 3.5 + pulse * 2.0, 0.0, TAU, 1.6, lighten(el_c(e), 0.4));
        }
        if wz.bolt_el == Some(e) {
            p.arc(q, r + 2.5, 0.0, TAU, 1.3, [1.0, 0.85, 0.4, 1.0]);
        }
    }
}
