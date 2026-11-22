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
use spellstorm::wizard::{used, Pad};
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
    /// "Run N" after a wipe, and how long it has shown.
    run_card: (String, f32),
    death_end: Option<f32>,
    vignette: Arc<Picture>,
    words: usize,
    /// What each seat had last tick (forms, LB, learned_t), to see unlocks.
    had: std::collections::HashMap<usize, (usize, bool, f64, usize)>,
    /// What each seat was ever shown a card for: a card only for what is
    /// truly new to that player (not again after a death or a new run).
    seen: std::collections::HashSet<(usize, u32)>,
    /// Unlocks to show one after another, and how long the first has shown.
    unlocks: Vec<Unlock>,
    unlock_t: f32,
    /// Damage and healing numbers floating up: (where, text, colour,
    /// size, age, life).
    numbers: Vec<(Vec3, String, C, f32, f32, f32)>,
    /// Hints for the moves a player has not used yet (Joep), by player:
    /// what they ever used, how long they have played since the last hint
    /// was done, and the hint showing now with how long it has shown.
    hints: std::collections::HashMap<String, Hints>,
}

#[derive(Default)]
struct Hints {
    used: u32,
    used_el: u32,
    wait: f32,
    now: Option<(u32, f32)>,
    /// The words of the hint showing (they follow what is loaded).
    text: String,
    /// A hint just done: its words, ticked off and fading (Joep).
    gone: Option<(String, f32)>,
    /// How long something newly learned has waited behind a basic hint.
    learned_wait: f32,
}
/// Seconds a done hint shows its tick while it fades.
const HINT_GONE: f32 = 1.2;

/// The moves taught by a hint under a wizard until they have been used, in
/// the order they are taught, a step at a time (Joep): one element loaded,
/// cleared again, shot, cast, then the same for the other starting element;
/// dodging and the sword. New skills first once learned: two elements
/// together as soon as a second slot opens.
const HINTS: [u32; 14] = [used::DRINK, NEW_ELEMENT, used::CAST2, used::NOVA, used::PLANT, used::ENCHANT, used::LOAD, used::SHAKE, used::BOLT_EL, used::CAST, used::CAST_PAIR, used::ROLL, used::SWING, used::DEFLECT];
/// Not a move: an element learned and never loaded yet. After a new skill's
/// card, its hint stays until it has been used (Joep).
const NEW_ELEMENT: u32 = 1 << 30;

/// The first element open to this wizard that they never loaded.
fn new_element(wz: &spellstorm::wizard::Wizard, used_el: u32) -> Option<El> {
    let order = spellstorm::wizard::ELEMENT_ORDER[wz.element_order % spellstorm::wizard::ELEMENT_ORDER.len()];
    order.iter().chain(spellstorm::wizard::LB_ORDER.iter()).copied().find(|&e| wz.element_open(e) && used_el & (1 << e.idx()) == 0)
}

/// The button that loads base element `e` ("{B}"), or with LB held; in
/// braces, drawn as the pad's own button (hint_line).
fn button_of(e: El) -> String {
    [(el::PAIR_A, "A"), (el::PAIR_B, "B"), (el::PAIR_X, "X"), (el::PAIR_Y, "Y")]
        .iter()
        .find_map(|(pair, b)| pair.iter().position(|&x| x == e).map(|i| if i == 1 { format!("{{lb}} {{{b}}}") } else { format!("{{{b}}}") }))
        .unwrap_or_else(|| "{A}".into())
}

/// A hint's words for this wizard: their own first elements and buttons,
/// and the spells those make.
fn hint_text(bit: u32, wz: &spellstorm::wizard::Wizard, mods: &spellstorm::relics::Mods, used_el: u32) -> String {
    let order = spellstorm::wizard::ELEMENT_ORDER[wz.element_order % spellstorm::wizard::ELEMENT_ORDER.len()];
    let open: Vec<El> = order.iter().copied().filter(|&e| wz.element_open(e)).collect();
    let e = wz.queue.slots.first().copied().or(open.first().copied()).unwrap_or(order[0]);
    let name = |els: &[El]| spellstorm::compose(els, Form::Cast, mods).name;
    match bit {
        NEW_ELEMENT => match new_element(wz, used_el | wz.used_el) {
            Some(n) => format!("New: {} loads {}", button_of(n), n.name()),
            None => String::new(),
        },
        used::NOVA => "New: hold {rb_hold} and let go to cast a Nova round you".into(),
        used::PLANT => "New: load elements, then click {ls} to plant them".into(),
        used::ENCHANT => "New: load elements, then click {rs} to enchant your staff".into(),
        used::LOAD => format!("{} loads {}", button_of(e), e.name()),
        used::BOLT_EL if wz.queue.is_empty() => format!("{} loads {}, then {{rstick}} shoots it", button_of(e), e.name()),
        used::BOLT_EL => format!("Push {{rstick}} to shoot {}", e.name()),
        used::CAST => format!("{} loads {}, then {{rt}} casts {}", button_of(e), e.name(), name(&[e])),
        // The same lesson again for the other starting element (Joep).
        used::CAST_PAIR => {
            let e = if wz.used & used::CAST_FIRST == 0 { order[0] } else { order[1] };
            format!("{} loads {}, then {{rt}} casts {}", button_of(e), e.name(), name(&[e]))
        }
        used::ROLL => "{lt} to dodge".into(),
        used::SWING => "{rb} to swing your sword".into(),
        used::DEFLECT => "Swing {rb} into a shot to send it back".into(),
        used::CAST2 => {
            let a = open.first().copied().unwrap_or(order[0]);
            let b = open.get(1).copied().unwrap_or(order[1]);
            format!("New: {} {} and {} {}, then {{rt}} casts {}", button_of(a), a.name(), button_of(b), b.name(), name(&[a, b]))
        }
        used::SHAKE => "Hold {rs} to clear elements".into(),
        used::DRINK => "Drink blood to heal".into(),
        _ => String::new(),
    }
}
/// Seconds of play before the first hint, and between one done and the next.
const HINT_WAIT: f32 = 4.0;

/// Something a wizard just learned, shown briefly mid-screen.
#[derive(Clone, PartialEq)]
enum Learned {
    Element(El),
    Move(Form),
    Pairs,
    /// A spell holds this many elements now (a new slot in the queue).
    Slots(usize),
    /// The staff's bolts on the right stick: open from the start, so shown
    /// as the very first thing learned (Joep).
    Bolts,
}

impl Learned {
    /// One number per thing that can be learned.
    fn key(&self) -> u32 {
        match self {
            Learned::Element(e) => e.idx() as u32,
            Learned::Move(f) => 100 + *f as u32,
            Learned::Pairs => 200,
            Learned::Slots(n) => 300 + *n as u32,
            Learned::Bolts => 400,
        }
    }
}

struct Unlock {
    what: Learned,
    /// Who learned it, when not every wizard did.
    who: Vec<String>,
}

const UNLOCK_SHOW: f32 = 3.2;

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
            run_card: (String::new(), 0.0),
            death_end: None,
            vignette: Arc::new(vignette()),
            words: 0,
            had: Default::default(),
            seen: Default::default(),
            unlocks: vec![],
            unlock_t: 0.0,
            numbers: vec![],
            hints: Default::default(),
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
                Ev::YouDiedEnd => {
                    self.death_end = Some(0.0);
                    // Back on your feet: which run this is, a moment over
                    // the level's name (Joep).
                    self.run_card = (format!("Run {}", w.runs_ended + 1), 0.0);
                }
                Ev::Level { .. } => self.level(w),
                Ev::Text { at, text, c, size, time } => self.numbers.push((*at, text.clone(), *c, *size, 0.0, time.max(0.6) + 0.3)),
                _ => {}
            }
        }
        self.watch_unlocks(w);
        self.watch_hints(w);
    }

    /// Per human player, the next move they never used, once its moment
    /// comes: dodging and the sword in a fight, shaking with a staff loaded.
    fn watch_hints(&mut self, w: &World) {
        let dt = spellstorm::world::DT as f32;
        if w.paused {
            return;
        }
        let fight = !w.living_enemies().is_empty();
        let shots = !w.bullets().is_empty();
        for &id in &w.wizards {
            let wz = w.wiz(id);
            if wz.bot.is_some() || !w.alive(id) {
                continue;
            }
            let key = if wz.player_id.is_empty() { format!("seat{}", wz.seat) } else { wz.player_id.clone() };
            let h = self.hints.entry(key).or_default();
            if let Some((_, t)) = &mut h.gone {
                *t += dt;
                if *t > HINT_GONE {
                    h.gone = None;
                }
            }
            h.used |= wz.used;
            h.used_el |= wz.used_el;
            let done = |bit: u32, used: u32, used_el: u32| if bit == NEW_ELEMENT { new_element(&wz, used_el).is_none() } else { used & bit == bit };
            // A new skill's hint waits for its card to go (Joep).
            let cards = !self.unlocks.is_empty();
            let u = h.used;
            // Only while hurt (Joep); healed, the hint goes until next time.
            let a = w.a(id);
            let hurt = a.hp < a.max_hp - 0.5;
            let ready = |bit: u32| match bit {
                // Only something learned later: the first elements are the
                // basics' own hints.
                NEW_ELEMENT => !cards && u & used::CAST != 0,
                used::NOVA => !cards && wz.form_open(Form::Nova),
                used::PLANT => !cards && wz.form_open(Form::Plant),
                used::ENCHANT => !cards && wz.form_open(Form::Enchant),
                used::ROLL | used::SWING => fight,
                used::DEFLECT => shots && u & used::SWING != 0,
                used::CAST_PAIR => u & used::CAST != 0,
                // Taught as soon as a second slot opens (Joep), after its card.
                used::CAST2 => !cards && wz.queue.size >= 2 && u & used::CAST != 0,
                // One of the first steps (Joep), once something is loaded.
                used::SHAKE => !wz.queue.is_empty() && u & used::LOAD != 0,
                used::DRINK => hurt,
                _ => true,
            };
            // Something just learned (a new element, a new move) comes before
            // whatever basic is showing, a while after its card (Joep: "press
            // X to load it" under the player too, until they do).
            const NEW: [u32; 6] = [used::DRINK, NEW_ELEMENT, used::CAST2, used::NOVA, used::PLANT, used::ENCHANT];
            let learned = NEW.iter().copied().find(|&bit| !done(bit, h.used, h.used_el) && ready(bit));
            if let Some((bit, t)) = &mut h.now {
                *t += dt;
                if *bit == used::DRINK && !hurt && !done(*bit, h.used, h.used_el) {
                    h.now = None;
                    h.text.clear();
                    h.wait = 0.0;
                } else if done(*bit, h.used, h.used_el) {
                    h.gone = Some((std::mem::take(&mut h.text), 0.0));
                    h.now = None;
                    h.wait = 0.0;
                } else if let Some(l) = learned.filter(|_| !NEW.contains(bit)) {
                    h.learned_wait += dt;
                    if h.learned_wait >= HINT_WAIT {
                        h.now = Some((l, 0.0));
                        h.learned_wait = 0.0;
                        h.text = hint_text(l, &wz, &w.a(id).mods, h.used_el);
                    } else {
                        h.text = hint_text(*bit, &wz, &w.a(id).mods, h.used_el);
                    }
                } else {
                    h.learned_wait = 0.0;
                    h.text = hint_text(*bit, &wz, &w.a(id).mods, h.used_el);
                }
                continue;
            }
            h.wait += dt;
            if h.wait < HINT_WAIT {
                continue;
            }
            h.now = HINTS.iter().copied().find(|&bit| !done(bit, h.used, h.used_el) && ready(bit)).map(|bit| (bit, 0.0));
            if let Some((bit, _)) = h.now {
                h.text = hint_text(bit, &wz, &w.a(id).mods, h.used_el);
            }
        }
    }

    /// New elements, moves and the LB pairs, as they open for each human
    /// wizard; one card for what several learned at once.
    fn watch_unlocks(&mut self, w: &World) {
        let mut fresh: Vec<(Learned, String)> = vec![];
        let mut humans = 0;
        for &id in &w.wizards {
            let wz = w.wiz(id);
            if wz.bot.is_some() {
                continue;
            }
            humans += 1;
            let forms = spellstorm::wizard::STAGE_FORMS[wz.stage.min(spellstorm::wizard::STAGES - 1)].len();
            let slots = spellstorm::wizard::STAGE_SLOTS[wz.stage.min(spellstorm::wizard::STAGES - 1)];
            let now = (forms, wz.lb_open(), wz.learned_t, slots);
            // A wizard first seen (joining, a new run) shows nothing, but
            // for the bolts to someone who never shot.
            let mut new: Vec<Learned> = vec![];
            if !self.had.contains_key(&wz.seat) && wz.used & used::BOLT == 0 && !w.settings.unlock_all {
                new.push(Learned::Bolts);
            }
            if let Some(&(f0, lb0, t0, s0)) = self.had.get(&wz.seat) {
                // A new element slot is a big step: its own card, first
                // (Joep).
                if slots > s0 {
                    new.insert(0, Learned::Slots(slots));
                }
                if wz.learned_t > t0 + 1.0 {
                    new.extend(wz.learned.iter().map(|&e| Learned::Element(e)));
                }
                for &f in &spellstorm::wizard::STAGE_FORMS[wz.stage.min(spellstorm::wizard::STAGES - 1)][f0.min(forms)..] {
                    new.push(Learned::Move(f));
                }
                if now.1 && !lb0 {
                    new.push(Learned::Pairs);
                }
            }
            self.had.insert(wz.seat, now);
            // Only what this player never had a card for (not again after
            // a death or a new run); what they hold now counts as known.
            let seat = wz.seat;
            new.retain(|what| self.seen.insert((seat, what.key())));
            fresh.extend(new.into_iter().map(|what| (what, wz.name.clone())));
            for e in wz.open_elements() {
                self.seen.insert((seat, Learned::Element(e).key()));
            }
            for &f in spellstorm::wizard::STAGE_FORMS[wz.stage.min(spellstorm::wizard::STAGES - 1)] {
                self.seen.insert((seat, Learned::Move(f).key()));
            }
            if wz.lb_open() {
                self.seen.insert((seat, Learned::Pairs.key()));
            }
            for n in 2..=slots {
                self.seen.insert((seat, Learned::Slots(n).key()));
            }
        }
        // Everything unlocked at once (the menu's Unlocks: All, or forced):
        // no cards for it, and none left over (Joep).
        if w.settings.unlock_all || w.settings.forced_unlocks >= 0 {
            self.unlocks.clear();
            self.unlock_t = 0.0;
            return;
        }
        for (what, name) in fresh {
            match self.unlocks.iter_mut().find(|u| u.what == what) {
                Some(u) => u.who.push(name),
                None => self.unlocks.push(Unlock { what, who: vec![name] }),
            }
        }
        for u in &mut self.unlocks {
            if u.who.len() >= humans {
                u.who.clear();
            }
        }
    }

    pub fn update(&mut self, dt: f32) {
        if !self.unlocks.is_empty() {
            self.unlock_t += dt;
            if self.unlock_t > UNLOCK_SHOW {
                self.unlocks.remove(0);
                self.unlock_t = 0.0;
            }
        }
        self.banner_t = (self.banner_t - dt).max(0.0);
        for t in &mut self.toasts {
            t.2 += dt;
        }
        self.toasts.retain(|t| t.2 < 5.0);
        for n in &mut self.numbers {
            n.4 += dt;
        }
        self.numbers.retain(|n| n.4 < n.5);
        self.fade.step(dt);
        self.card_t += dt;
        self.run_card.1 += dt;
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
            // Smaller than they were (Joep), the bar as big as a monster's.
            p.text(&wz.name, x, y + 12.0, body(), 19.0, lighten(wz.robe, 0.4), TextAlign::Center, 7.0);
            // Health under the name (Joep): red on a dark sleeve, gone
            // while down.
            if a.alive && a.max_hp > 0.0 {
                let k = (a.hp / a.max_hp).clamp(0.0, 1.0) as f32;
                let (bw, by) = (64.0, y + 30.0);
                p.rect([x - bw * 0.5 - 2.0, by - 2.0, bw + 4.0, 9.0], [0.0, 0.0, 0.0, 0.7]);
                p.rect([x - bw * 0.5, by, bw * k, 5.0], [0.8, 0.12, 0.08, 1.0]);
            }
            let key = if wz.player_id.is_empty() { format!("seat{}", wz.seat) } else { wz.player_id.clone() };
            let shown = self.hints.get(&key).filter(|_| a.alive && wz.bot.is_none());
            if let Some(h) = shown {
                if let Some((_, t)) = h.now.filter(|_| !h.text.is_empty()) {
                    // In softly, then a slow pulse so it is noticed.
                    let a = (t / 0.4).min(1.0) * (0.8 + 0.2 * (t * 3.0).sin());
                    hint_line(&mut p, &h.text, x, y + 62.0, a, None);
                } else if let Some((text, t)) = h.gone.as_ref().filter(|g| !g.0.is_empty()) {
                    // Done (Joep): a tick in front, then it fades and rises.
                    let k = (t / HINT_GONE).clamp(0.0, 1.0);
                    let a = 1.0 - smooth01(((k - 0.35) / 0.65).clamp(0.0, 1.0));
                    hint_line(&mut p, text, x, y + 62.0 - 10.0 * k * k, a, Some((t / 0.2).min(1.0)));
                }
            }
        }
        tome_labels(&mut p, w, cam, &vp, size);
        enemy_bars(&mut p, w, &vp, size);
        self.draw_numbers(&mut p, &vp, size);
        p.d.begin_overlay();
        for (i, &id) in w.wizards.iter().enumerate() {
            let right = i % 2 == 1;
            let top = i >= 2;
            let x = if right { sw - MARGIN - PANEL.x } else { MARGIN };
            let y = if top { MARGIN } else { 1080.0 - MARGIN - PANEL.y };
            panel(&mut p, w, id, Vec2::new(x, y), right);
        }
        grave_arrows(&mut p, w, &vp, sw);
        // Toasts at the top in the middle.
        for (k, (text, c, t)) in self.toasts.iter().enumerate() {
            let a = if *t < 4.0 { 1.0 } else { 1.0 - (t - 4.0) };
            p.text(text, sw * 0.5, 20.0 + 18.0 + k as f32 * 34.0, body(), 26.0, alpha(*c, a), TextAlign::Center, 8.0);
        }
        // Close-up shots (SPELLSTORM_ZOOM) leave the middle of the screen free,
        // and the pause menu covers it (the card showed through the menu).
        if let Some(u) = self.unlocks.first().filter(|_| std::env::var_os("SPELLSTORM_ZOOM").is_none() && !w.paused) {
            unlock_card(&mut p, u, self.unlock_t, sw);
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
            // The name itself keeps one size: every new size rasterises a whole
            // alphabet into the glyph atlas, and a growing one filled it at 4K.
            p.shadow_text(&self.card, sw * 0.5, cy - 20.0, title(), 96.0, col);
            p.rect([sw * 0.5 - 380.0 * grow, cy + 44.0 * grow, 760.0 * grow, 2.0], [0.86, 0.8, 0.68, 0.55 * a]);
        }
        // The run counter after a wipe, over the card, as the card fades.
        let t = self.run_card.1;
        if t < 5.1 && !self.run_card.0.is_empty() {
            let a = if t < 0.9 { 0.0 } else if t < 2.1 { (t - 0.9) / 1.2 } else if t < 3.7 { 1.0 } else { 1.0 - (t - 3.7) / 1.4 };
            p.shadow_text(&self.run_card.0, sw * 0.5, 400.0, title(), 96.0, [0.86, 0.8, 0.68, a.clamp(0.0, 1.0)]);
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
            // One size, as the level card (the atlas).
            let size = if words.len() < 20 { 150.0 } else { 108.0 };
            let lines: Vec<&str> = words.lines().collect();
            for (k, l) in lines.iter().enumerate() {
                let y = 540.0 + (k as f32 - (lines.len() as f32 - 1.0) * 0.5) * size * 1.1;
                p.text(l, sw * 0.5, y, title(), size, [0.62, 0.04, 0.03, a], TextAlign::Center, 0.0);
            }
        }
    }

    /// The damage and healing numbers (World::numbers): over whoever took
    /// it, drifting up, fading at the end. One size each, as the atlas wants.
    fn draw_numbers(&self, p: &mut Painter, vp: &Mat4, size: [u32; 2]) {
        for (at, text, c, px, age, life) in &self.numbers {
            let k = age / life;
            let rise = 0.9 * (1.0 - (1.0 - k).powi(2));
            let Some(q) = on_screen(vp, *at + Vec3::Y * rise, size, p.s) else { continue };
            let a = 1.0 - ((k - 0.65) / 0.35).clamp(0.0, 1.0);
            let px = px * spellstorm::knobs::number_size() as f32;
            p.text(text, q.x, q.y, title(), px, alpha(*c, a), TextAlign::Center, (px * 0.27).max(2.0));
        }
    }
}

/// Where `at` lands on Godot's 1080-high canvas, if in front of the camera.
fn on_screen(vp: &Mat4, at: Vec3, size: [u32; 2], s: f32) -> Option<Vec2> {
    let c = *vp * Vec4::new(at.x, at.y, at.z, 1.0);
    if c.w <= 0.0 {
        return None;
    }
    Some(Vec2::new((c.x / c.w * 0.5 + 0.5) * size[0] as f32 / s, (0.5 - c.y / c.w * 0.5) * size[1] as f32 / s))
}

/// Joep: a fallen wizard off the screen gets an arrow at the screen's
/// edge, in their robe's colour, pointing to their grave, so a friend
/// knows where to go to raise them.
fn grave_arrows(p: &mut Painter, w: &World, vp: &Mat4, sw: f32) {
    let time = w.tick as f32 / 60.0;
    let (inset, mid) = (70.0, Vec2::new(sw * 0.5, 540.0));
    for &id in &w.wizards {
        let Some(a) = w.actor(id) else { continue };
        let wz = w.wiz(id);
        let (false, Some(at)) = (a.alive, wz.tomb) else { continue };
        let c = *vp * Vec4::new(at.x, at.y + 1.0, at.z, 1.0);
        // Behind the camera the projection flips: point the other way.
        let ndc = Vec2::new(c.x, -c.y) / c.w.abs().max(1e-4);
        let q = mid + ndc * Vec2::new(sw * 0.5, 540.0) * c.w.signum();
        let seen = c.w > 0.0 && q.x > inset * 0.5 && q.x < sw - inset * 0.5 && q.y > inset * 0.5 && q.y < 1080.0 - inset * 0.5;
        if seen {
            continue;
        }
        let dir = if c.w > 0.0 { q - mid } else { mid - q }.normalize_or(Vec2::Y);
        // Where the line from the middle meets the inset frame.
        let half = Vec2::new(sw * 0.5 - inset, 540.0 - inset);
        let t = (half.x / dir.x.abs().max(1e-4)).min(half.y / dir.y.abs().max(1e-4));
        let tip = mid + dir * t;
        let pulse = 1.0 + 0.12 * (time * 5.0).sin();
        let side = Vec2::new(-dir.y, dir.x);
        let (len, wide) = (46.0 * pulse, 26.0 * pulse);
        let base = tip - dir * len;
        let rim = [0.0, 0.0, 0.0, 0.75];
        p.poly(&[tip + dir * 5.0, base + side * (wide + 5.0) - dir * 3.0, base - side * (wide + 5.0) - dir * 3.0], rim);
        p.poly(&[tip, base + side * wide, base - side * wide], lighten(wz.robe, 0.25));
        p.circle(base - dir * 26.0, 22.0, rim);
        p.circle(base - dir * 26.0, 18.0, lighten(wz.robe, 0.1));
        let first = wz.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        p.text(&first, base.x - dir.x * 26.0, base.y - dir.y * 26.0, title(), 24.0, [1.0, 1.0, 1.0, 1.0], TextAlign::Center, 6.0);
    }
}

/// A thin health bar over every hurt monster, wider for bigger ones.
fn enemy_bars(p: &mut Painter, w: &World, vp: &Mat4, size: [u32; 2]) {
    for &id in &w.enemies {
        let Some(a) = w.actor(id) else { continue };
        // A burrower under the floor shows no bar (Enemy._go_under).
        let under = w.ecs.get::<&spellstorm::enemy::Enemy>(id).is_ok_and(|e| e.under);
        if under || !a.alive || a.hp >= a.max_hp || a.max_hp <= 0.0 {
            continue;
        }
        let r = a.radius as f32;
        let Some(q) = on_screen(vp, a.pos + Vec3::Y * (1.15 + r * 1.3), size, p.s) else { continue };
        let wd = (44.0 + 56.0 * r).min(160.0);
        let k = (a.hp / a.max_hp).clamp(0.0, 1.0) as f32;
        let x = q.x - wd * 0.5;
        p.rect([x - 2.0, q.y - 2.0, wd + 4.0, 9.0], [0.0, 0.0, 0.0, 0.7]);
        p.rect([x, q.y, wd * k, 5.0], [0.8, 0.12, 0.08, 1.0]);
    }
}

/// Over each altar its tome's name in the title face and gold, and under it
/// what it does in pale plain text (Run._altar_label: 80 and 60 px at
/// 0.0035 m a pixel, wrapped at 720 px, always on top).
fn tome_labels(p: &mut Painter, w: &World, cam: &Camera, vp: &Mat4, size: [u32; 2]) {
    let right = (cam.target - cam.pos).cross(Vec3::Y).normalize_or_zero();
    let ps = p.s;
    let screen = move |at: Vec3| {
        let c = *vp * Vec4::new(at.x, at.y, at.z, 1.0);
        (c.w > 0.0).then(|| Vec2::new((c.x / c.w * 0.5 + 0.5) * size[0] as f32 / ps, (0.5 - c.y / c.w * 0.5) * size[1] as f32 / ps))
    };
    let tomes = spellstorm::relics::all();
    for a in &w.run.altars {
        if a.rise <= 0.0 || a.taken {
            continue;
        }
        let Some(relic) = tomes.get(a.relic) else { continue };
        let k = a.rise.min(1.0);
        let base = a.pos.with_y(0.0);
        let (Some(top), Some(under), Some(side)) = (screen(base + Vec3::Y * 2.68 * k), screen(base + Vec3::Y * 2.58 * k), screen(base + Vec3::Y * 2.58 * k + right)) else { continue };
        // A fixed size on screen: scaling with the distance stepped through
        // font sizes as the camera moved (`side` only anchors the label now).
        let _ = side;
        let (title_px, text_px) = (34.0, 22.0);
        let wrap = 320.0;
        p.text(&relic.name, top.x, top.y - title_px * 0.6, title(), title_px, [1.0, 0.8, 0.42, k], TextAlign::Center, title_px * 0.225);
        for (i, line) in wrapped(&relic.text, body(), text_px * p.s, wrap * p.s).iter().enumerate() {
            p.text(line, under.x, under.y + text_px * (0.6 + 1.2 * i as f32), body(), text_px, [0.86, 0.84, 0.8, k], TextAlign::Center, text_px * 0.3);
        }
    }
}

/// `text` broken into lines no wider than `width` px at `px`.
fn wrapped(text: &str, font: FontFace, px: f32, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && render::hud::measure(&next, font, px)[0] > width {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
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
        // Sizes on a 4% grid: each new size costs the glyph atlas a whole
        // alphabet, so text that scales with the camera must not make one a frame.
        let px = (size * s).max(1.0);
        let px = if px < 16.0 { px.round() } else { 1.04f32.powf((px.ln() / 1.04f32.ln()).round()) }.min(500.0);
        let w = render::hud::measure(text, font, px)[0] + 4.0;
        let h = px * 1.2;
        let left = match align {
            TextAlign::Left => x * s,
            TextAlign::Center => x * s - w * 0.5,
            TextAlign::Right => x * s - w,
        };
        let top = y * s - h * 0.5;
        let id = 0x5ee1_0000_0000 + self.d.text.len() as u64;
        self.d.text.push(TextRun { id, text: text.into(), position: [left, top], bounds: NO_CLIP, font_size: px, color: bytes(c), font, align, width: w });
        // Godot's outline_size is the whole stroke; a real one round the glyphs.
        if outline > 0.0 {
            self.d.outline(id, outline * s * 0.5, bytes([0.0, 0.0, 0.0, 0.9 * c[3]]));
        }
    }
    /// How wide `text` is drawn by `text`, in 1080-high units.
    fn width(&self, text: &str, font: FontFace, size: f32) -> f32 {
        let s = self.s;
        let px = (size * s).max(1.0);
        let px = if px < 16.0 { px.round() } else { 1.04f32.powf((px.ln() / 1.04f32.ln()).round()) }.min(500.0);
        (render::hud::measure(text, font, px)[0] + 4.0) / s
    }
    /// Centred, with a soft shadow below (the level card).
    fn shadow_text(&mut self, text: &str, x: f32, y: f32, font: FontFace, size: f32, c: C) {
        for (k, a) in [(6.0, 0.3), (4.0, 0.5)] {
            self.text(text, x, y + k, font, size, [0.0, 0.0, 0.0, a * c[3]], TextAlign::Center, 12.0);
        }
        self.text(text, x, y, font, size, c, TextAlign::Center, 0.0);
    }
}

fn smooth01(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// A hint's line centred on `x`, its `{button}`s drawn as the pad's own
/// buttons (Joep: icons, not "RT"), at opacity `a`; `tick` (0..1, how far
/// it has popped in) puts a green tick in front for a hint just done.
fn hint_line(p: &mut Painter, text: &str, x: f32, y: f32, a: f32, tick: Option<f32>) {
    const SIZE: f32 = 24.0;
    const K: f32 = 0.62;
    if a <= 0.003 {
        return;
    }
    // Words and buttons in turn.
    let mut parts: Vec<(bool, &str)> = vec![];
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { break };
        if i > 0 {
            parts.push((false, &rest[..i]));
        }
        parts.push((true, &rest[i + 1..i + j]));
        rest = &rest[i + j + 1..];
    }
    if !rest.is_empty() {
        parts.push((false, rest));
    }
    let wide = |kind: &str| -> f32 {
        match kind.to_ascii_lowercase().as_str() {
            "lb" | "rb" | "lt" | "rt" => 38.0,
            "rb_hold" => 56.0,
            "rstick" | "lstick" => 36.0,
            _ => 30.0,
        }
    };
    let words = |p: &Painter, t: &str| p.width(t, body(), SIZE);
    let tick_w = if tick.is_some() { 34.0 } else { 0.0 };
    let total: f32 = tick_w + parts.iter().map(|&(icon, t)| if icon { wide(t) } else { words(p, t) }).sum::<f32>();
    let mut cx = x - total * 0.5;
    // The buttons are shapes: drawn first, then faded with the vertices.
    let v0 = p.d.vertices.len();
    let t0 = p.d.text.len();
    if let Some(k) = tick {
        let c = Vec2::new(cx + 13.0, y);
        let r = 12.0 * (0.6 + 0.4 * k) * (1.0 + 0.25 * (k * PI).sin());
        p.circle(c, r + 1.5, [0.02, 0.06, 0.02, 1.0]);
        p.circle(c, r, [0.3, 0.78, 0.32, 1.0]);
        let q = |dx: f32, dy: f32| c + Vec2::new(dx, dy) * (r / 12.0);
        p.line(q(-5.5, 0.5), q(-1.5, 4.5), 3.0, [1.0, 1.0, 1.0, 1.0]);
        p.line(q(-1.5, 4.5), q(6.0, -4.5), 3.0, [1.0, 1.0, 1.0, 1.0]);
        cx += tick_w;
    }
    let mut runs: Vec<(f32, &str)> = vec![];
    for &(icon, t) in &parts {
        if icon {
            let w = wide(t);
            crate::menu::pad_icon(p.d, p.s * K, &t.to_ascii_lowercase(), Vec2::new(cx + w * 0.5, y) / K);
            cx += w;
        } else {
            runs.push((cx, t));
            cx += words(p, t);
        }
    }
    for v in &mut p.d.vertices[v0..] {
        v.color[3] *= a;
    }
    for t in &mut p.d.text[t0..] {
        t.color[3] = (t.color[3] as f32 * a).round() as u8;
    }
    let col = if tick.is_some() { [0.75, 1.0, 0.75, a] } else { [1.0, 0.92, 0.7, a] };
    for (left, t) in runs {
        p.text(t, left, y, body(), SIZE, col, TextAlign::Left, 8.0);
    }
}

/// What was just learned, below the middle of the screen in the start
/// menu's look (menu::unlock_card): in and up, a hold, out.
fn unlock_card(p: &mut Painter, u: &Unlock, t: f32, sw: f32) {
    use crate::menu::CardIcon;
    let a = (t / 0.25).min(1.0) * ((UNLOCK_SHOW - t) / 0.5).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let rise = (1.0 - (t / 0.35).min(1.0)).powi(2) * 18.0;
    let (kicker, name, hint, icon) = match u.what {
        Learned::Element(e) => {
            let (pad, lb) = [(el::PAIR_A, "A"), (el::PAIR_B, "B"), (el::PAIR_X, "X"), (el::PAIR_Y, "Y")]
                .iter()
                .find_map(|(pair, b)| pair.iter().position(|&x| x == e).map(|i| (*b, i == 1)))
                .unwrap_or(("A", false));
            let hint = if lb { format!("Hold LB and press {pad}") } else { format!("Press {pad} to load it") };
            ("New element", e.name().to_string(), hint, CardIcon::Element(e))
        }
        Learned::Move(f) => {
            let (name, hint, pad) = match f {
                Form::Nova => ("Nova", "Hold RB, then let go", "rb_hold"),
                Form::Plant => ("Plant", "Click the left stick to set it down", "ls"),
                Form::Enchant => ("Enchant", "Click the right stick to enchant your staff", "rs"),
                Form::Cast => ("Cast", "Press RT", "rt"),
            };
            ("New move", name.to_string(), hint.to_string(), CardIcon::Pad(pad))
        }
        Learned::Bolts => ("New move", "Staff Bolts".to_string(), "Push the right stick to shoot".to_string(), CardIcon::Pad("rstick")),
        Learned::Pairs => ("New elements", "Element Pairs".to_string(), "Hold LB: every button gives its other element".to_string(), CardIcon::Pad("lb")),
        Learned::Slots(n) => {
            let name = if n == 2 { "Second element slot" } else { "Third element slot" };
            ("New element slot", name.to_string(), format!("Load {n} elements into one spell"), CardIcon::Slots(n))
        }
    };
    let kicker = if u.who.is_empty() { kicker.to_string() } else { format!("{kicker}   ·   {}", u.who.join(", ")) };
    crate::menu::unlock_card(p.d, p.s, sw * 0.5, 690.0 + rise, icon, &kicker, &name, &hint, a);
}

/// An element as a little ball (ElementBall: its colour, lit from the top
/// left, with its icon).
fn ball(p: &mut Painter, e: El, c: Vec2, r: f32, dim: f32) {
    let col = el_c(e);
    // Its own bright colour with a dark icon on it (Joep: more contrast).
    let k = dim.min(1.0);
    p.circle(c, r, alpha(lighten(col, 0.1 * (dim - 1.0).max(0.0) * 4.0), k));
    p.circle(c + Vec2::new(-r * 0.3, -r * 0.34), r * 0.24, [1.0, 1.0, 1.0, 0.3 * k]);
    icon(p, e, c, r * 0.66, [0.04, 0.02, 0.02, k]);
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
    buttons(p, &wz, Vec2::new(x(226.0), y(40.0)));
    // What each cast button makes of the queue, two by two.
    if !slots.is_empty() {
        let names: Vec<(&str, String)> = FORMS.iter().filter(|f| wz.form_open(f.1)).map(|f| (f.0, wz.preview(f.1).name)).collect();
        for (i, (b, name)) in names.iter().enumerate() {
            let cx = 14.0 + (i % 2) as f32 * 146.0;
            let cy = 70.0 + (i / 2) as f32 * 19.0;
            let chip_x = x(cx) - if mirrored { 22.0 } else { 0.0 };
            p.round_rect([chip_x, y(cy), 22.0, 14.0], 4.0, [1.0, 0.84, 0.48, 1.0]);
            p.text(b, chip_x + 11.0, y(cy + 7.5), title(), 11.0, [0.04, 0.03, 0.02, 1.0], TextAlign::Center, 0.0);
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
    // Bigger than they were and on a dark plate (Joep: hard to read).
    let d = 20.0;
    let places = [(el::PAIR_Y, Pad::Y, Vec2::new(0.0, -d)), (el::PAIR_X, Pad::X, Vec2::new(-d, 0.0)), (el::PAIR_B, Pad::B, Vec2::new(d, 0.0)), (el::PAIR_A, Pad::A, Vec2::new(0.0, d))];
    p.circle(c, d + 13.0, [0.02, 0.015, 0.015, 0.75]);
    for (pair, bit, off) in places {
        let e = pair[if flip { 1 } else { 0 }];
        let q = c + off;
        let held = wz.pad.down(bit);
        let r = if held { 11.5 } else { 10.0 };
        if !wz.element_open(e) {
            p.circle(q, 5.0, [0.3, 0.28, 0.26, 1.0]);
            continue;
        }
        ball(p, e, q, r, if held { 1.25 } else { 1.0 });
        if wz.learned.contains(&e) && wz.learned_t > 0.0 {
            let pulse = 0.5 + 0.5 * (wz.learned_t as f32 * 7.0).sin();
            p.arc(q, r + 3.5 + pulse * 2.0, 0.0, TAU, 1.6, lighten(el_c(e), 0.4));
        }
        if wz.bolt_el == Some(e) {
            p.arc(q, r + 2.5, 0.0, TAU, 1.3, [1.0, 0.85, 0.4, 1.0]);
        }
    }
}
