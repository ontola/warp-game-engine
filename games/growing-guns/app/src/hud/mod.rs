//! The HUD and screens, after Godot's `render_player.gd` (per view: the
//! bottom row of plates, crosshair, hit marker, damage arcs, death and ghost
//! tints, toasts, the round-win strip, the card pick) and `game.gd` (shared:
//! the round banner, kill feed, coop kill count, the match-over banner and
//! its menu, the pause and settings menus, the scoreboard). All vector
//! shapes and text; no textures. Sizes are Godot design units (a 1281x720
//! canvas stretched to the window) times the view's layout scale.
pub mod cards;
pub mod menus;
pub mod plates;
pub mod style;

use crate::view::PLAYER_COLORS_U8;
use growing_guns::{Event, PickupKind, Phase, World, DT, MAX_DASH_CHARGES};
use render::hud::HudDrawList;
use style::*;

/// Godot design height: the canvas is 1281x720 stretched to the window.
pub const DESIGN_H: f32 = 720.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum HitKind { #[default] Body, Head, Kill }

/// Floating damage number over a hit (Godot `_show_hit_damage_number`).
#[derive(Clone, Copy, Debug)]
pub struct Number { pub pos: glam::Vec3, pub damage: i32, pub head: bool, pub age: f32 }

/// The card offer as last shown, kept for the exit animation after a pick.
#[derive(Clone, Debug)]
pub struct Exit { pub cards: Vec<growing_guns::cards::Card>, pub picked: usize, pub age: f32 }

/// Per player feedback timers kept by the app.
#[derive(Clone, Debug)]
pub struct Feedback {
    /// Seconds since the last hit-marker flash, and its kind.
    pub hit: f32,
    pub hit_kind: HitKind,
    /// Damage direction arcs: angle from facing (0 forward, +right), life left.
    pub arcs: Vec<(f32, f32)>,
    /// Seconds since a lava or fall death (the red screen).
    pub burned: Option<f32>,
    pub bob: f32,
    /// The special's cooldown when it was last set, for the progress bar.
    pub special_max: f64,
    last_special: f64,
    /// Pickup toast: title, subtitle, colour, seconds shown.
    pub toast: Option<(String, String, C, f32)>,
    pub numbers: Vec<Number>,
    pub exit: Option<Exit>,
    /// Seconds since the card pick closed: the dark backdrop fades out.
    pub pick_closed: f32,
    /// Card highlight 0..1 per offered card (eases like Godot's tweens).
    pub highlight: Vec<f32>,
    /// Stat chips per offered card, worked out when the offer opens.
    pub stats: Vec<Vec<String>>,
    /// The explosion flash (Godot `_explosion_flash_overlay`): alpha now,
    /// the target it rises to, and how fast the target falls.
    pub flash: [f32; 3],
    last_offer: Option<Vec<growing_guns::cards::Card>>,
}

impl Default for Feedback {
    fn default() -> Self {
        Feedback { hit: 9.0, hit_kind: HitKind::Body, arcs: Vec::new(), burned: None, bob: 0.0, special_max: 0.0, last_special: 0.0, toast: None,
            numbers: Vec::new(), exit: None, pick_closed: 9.0, highlight: Vec::new(), stats: Vec::new(), flash: [0.0; 3], last_offer: None }
    }
}

/// The shared round banner (Godot `RoundBanner`): text, size, seconds left.
#[derive(Clone, Debug)]
pub struct Banner { pub text: String, pub size: f32, pub left: f32 }

/// Everything the HUD remembers between ticks.
#[derive(Clone, Debug, Default)]
pub struct Hud {
    pub fb: Vec<Feedback>,
    pub banner: Option<Banner>,
    /// Kill feed lines with their age (Godot: 3+ humans only).
    pub feed: Vec<(String, f32)>,
    /// Seconds since the match ended.
    pub match_over: Option<f32>,
    /// Seconds since a new round began (the white flash and modifier toast).
    pub round_age: f32,
    /// Display names: humans P1..P4, bots Godot-style ("Blitz Byte 23").
    pub names: Vec<String>,
    bots: Vec<bool>,
    /// Names given from outside (a GameNight party's seats), by world player.
    given: Vec<Option<String>>,
    /// The lava leak was running last tick (its "LAVA LEAK" banner).
    lava_was: bool,
    /// World players with a view on this machine.
    pub locals: Vec<usize>,
}

const BOT_PREFIXES: [&str; 16] = ["Blitz", "Crank", "Dash", "Fang", "Fuse", "Glitch", "Hex", "Jolt", "Knock", "Pixel", "Quake", "Rift", "Scrap", "Spark", "Vex", "Zip"];
const BOT_SUFFIXES: [&str; 16] = ["Byte", "Core", "Drift", "Flux", "Kick", "Loop", "Nail", "Patch", "Pulse", "Rush", "Scope", "Shift", "Slug", "Snap", "Volt", "Wire"];

pub fn bot_name(i: usize) -> String {
    let h = (i as u32).wrapping_mul(2654435761).rotate_left(7) ^ 0x9e37;
    format!("{} {} {:02}", BOT_PREFIXES[(h % 16) as usize], BOT_SUFFIXES[((h >> 4) % 16) as usize], 10 + (h >> 8) % 90)
}

pub fn pickup_info(k: PickupKind) -> (&'static str, &'static str, C) {
    match k {
        PickupKind::Heart => ("HEART", "+50 HP (overheal)", [1.0, 0.25, 0.45, 1.0]),
        PickupKind::Mushroom => ("MUSHROOM", "+100 HP, big body", [1.0, 0.35, 0.2, 1.0]),
        PickupKind::Bomb => ("BOMB", "2× explosive rounds", [1.0, 0.72, 0.15, 1.0]),
        PickupKind::PlusOne => ("+1 UP", "Revive once this round", [0.35, 1.0, 0.45, 1.0]),
        PickupKind::Laser => ("LASER RIFLE", "A lot of crazy fast pew pews", [0.25, 0.95, 1.0, 1.0]),
        PickupKind::Dice => ("DICE", "Random stat ×2 and ×½", [0.95, 0.85, 1.0, 1.0]),
        PickupKind::AirStrike => ("AIR STRIKE", "Next RMB: rocket on crosshair", [1.0, 0.45, 0.12, 1.0]),
    }
}

/// Godot `round_modifiers.gd display_info`: title, subtitle, colour.
pub fn modifier_info(m: growing_guns::Modifier) -> (&'static str, &'static str, C) {
    use growing_guns::Modifier::*;
    match m {
        LowGravity => ("LOW GRAVITY", "Floatier jumps & bullets", [0.55, 0.85, 1.0, 1.0]),
        Fog => ("FOG", "Short view distance", [0.72, 0.78, 0.88, 1.0]),
        SupplyStorm => ("SUPPLY STORM", "Sky drops spawn 3× faster", [0.45, 1.0, 0.65, 1.0]),
        SuddenDeath => ("SUDDEN DEATH", "Lava rises immediately", [1.0, 0.42, 0.18, 1.0]),
        HeadshotsOnly => ("HEADSHOTS ONLY", "Body damage ×¼", [1.0, 0.55, 0.55, 1.0]),
        BoomBoom => ("BOOM BOOM", "Explosive rounds for everyone", [1.0, 0.62, 0.15, 1.0]),
        Brrrrt => ("BRRRRT", "10× fire rate & ammo for everyone", [1.0, 0.78, 0.35, 1.0]),
        GunSwap => ("GUN SWAP", "You spawn with someone else's loadout", [0.82, 0.48, 1.0, 1.0]),
        SniperFight => ("SNIPER FIGHT", "RMB zoom · 10× precision · 10× bullet speed", [0.55, 0.88, 1.0, 1.0]),
        AirStrikes => ("AIR STRIKES", "Random rockets from the sky", [1.0, 0.28, 0.12, 1.0]),
        IonCannon => ("ION CANNON", "Orbital beams from above", [0.35, 0.72, 1.0, 1.0]),
        Blackout => ("BLACKOUT", "Lights out — use your flashlight", [0.3, 0.32, 0.45, 1.0]),
    }
}

impl Hud {
    pub fn new(world: &World) -> Self {
        let mut h = Hud { round_age: 9.0, ..Default::default() };
        h.sync(world);
        h
    }

    pub fn sync(&mut self, world: &World) {
        let n = world.players.len();
        self.fb.resize_with(n, Feedback::default);
        self.bots = world.players.iter().map(|p| p.bot).collect();
        self.names = (0..n).map(|i| match self.given.get(i) {
            Some(Some(g)) if !world.players[i].bot => g.clone(),
            _ if world.players[i].bot => bot_name(i),
            _ => format!("P{}", i + 1),
        }).collect();
    }

    /// Party names for the seated players: `names[k]` belongs to world
    /// player `slots[k]` (GameNight's `Gg::names`); empty restores P1..P4.
    pub fn set_names(&mut self, world: &World, slots: &[usize], names: &[String]) {
        let mut given = vec![None; world.players.len()];
        for (&s, n) in slots.iter().zip(names) { if let Some(g) = given.get_mut(s) && !n.trim().is_empty() { *g = Some(n.trim().to_string()); } }
        if given != self.given { self.given = given; self.sync(world); }
    }

    pub fn name(&self, i: usize) -> &str { self.names.get(i).map_or("Player", |s| s.as_str()) }

    pub fn announce(&mut self, text: impl Into<String>, seconds: f32, size: f32) { self.banner = Some(Banner { text: text.into(), size, left: seconds }); }

    /// One tick: advance timers and read this tick's events.
    pub fn tick(&mut self, world: &World, dt: f32) {
        if self.bots.len() != world.players.len() || self.bots.iter().zip(&world.players).any(|(&b, p)| b != p.bot) {
            // A remote player took a seat or left it (Godot "%s joined" / "%s left").
            let flips: Vec<(usize, bool)> = self.bots.iter().zip(&world.players).enumerate().filter(|(i, (b, p))| **b != p.bot && !self.locals.contains(i)).map(|(i, (_, p))| (i, p.bot)).collect();
            self.sync(world);
            for (i, bot) in flips { let n = self.name(i).to_string(); self.announce(if bot { format!("{n} left") } else { format!("{n} joined") }, 2.0, 0.0); }
        }
        let humans = world.players.iter().filter(|p| !p.bot).count();
        let mut clear_banner = false;
        for (i, f) in self.fb.iter_mut().enumerate() {
            let p = &world.players[i];
            f.hit += dt;
            for a in &mut f.arcs { a.1 -= dt; }
            f.arcs.retain(|a| a.1 > 0.0);
            if let Some(b) = &mut f.burned { *b += dt; }
            if p.alive { f.burned = None; }
            let speed = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt();
            f.bob += speed * dt * 0.75;
            if p.special_cd > f.last_special + 1e-6 { f.special_max = p.special_cd; }
            f.last_special = p.special_cd;
            if let Some(t) = &mut f.toast { t.3 += dt; if t.3 > 1.35 { f.toast = None; } }
            // Godot `_update_explosion_sidechain`: a quick rise, then the target decays.
            let [a, target, vel] = &mut f.flash;
            if *target > 0.0 || *a > 0.001 {
                *a += (*target - *a) * (dt * 25.0).clamp(0.0, 1.0);
                *target = (*target - *vel * dt).max(0.0);
            } else { *a = 0.0; *vel = 0.0; }
            for n in &mut f.numbers { n.age += dt; }
            f.numbers.retain(|n| n.age < 1.0);
            // Card pick: keep the offer for the exit animation, ease highlights.
            if let Some(e) = &mut f.exit { e.age += dt; if e.age > 0.48 { f.exit = None; } }
            match (&p.offer, &f.last_offer) {
                (None, Some(prev)) => {
                    let picked = world.events.iter().find_map(|e| match *e { Event::Picked { player, card } if player == i => prev.iter().position(|&c| c == card), _ => None });
                    // Picked: the chosen card flies up. Taken away (round
                    // restart, match end): everything just fades.
                    if let Some(k) = picked { f.exit = Some(Exit { cards: prev.clone(), picked: k, age: 0.0 }); }
                    f.pick_closed = 0.0;
                }
                (Some(o), prev) => {
                    if prev.as_ref() != Some(o) {
                        f.stats = o.iter().map(|&c| cards::stat_diff(world, i, c)).collect();
                        // The picker's banner clears as the cards arrive (Godot `_show_card_pick`).
                        if self.locals.contains(&i) { clear_banner = true; }
                    }
                    f.pick_closed += dt;
                }
                _ => f.pick_closed += dt,
            }
            f.last_offer = p.offer.clone();
        }
        // Coop enemies go by their archetype (Godot `_coop_enemy_name`).
        if world.coop {
            for i in world.waves.seats..world.players.len().min(self.names.len()) {
                if let Some(a) = world.waves.archetype(i) {
                    let want = format!("{} {:02}", a.name().to_uppercase(), i - world.waves.seats + 1);
                    if self.names[i] != want { self.names[i] = want; }
                }
            }
        }
        // Sudden death: the leak's banner as it starts (Godot `_start_lava_leak`).
        if world.lava.active && !self.lava_was && world.lava.elapsed <= 0.1 { self.announce("LAVA LEAK", 1.5, 0.0); }
        self.lava_was = world.lava.active;
        for (_, age) in &mut self.feed { *age += dt; }
        self.feed.retain(|f| f.1 < 3.35);
        if let Some(b) = &mut self.banner { b.left -= dt; if b.left <= 0.0 { self.banner = None; } }
        self.round_age += dt;
        if world.phase == Phase::MatchOver { *self.match_over.get_or_insert(0.0) += dt; } else { self.match_over = None; }
        let new_strikes = world.strikes.iter().filter(|s| s.owner.is_none() && s.age <= DT as f64 * 1.5).map(|s| s.kind).collect::<Vec<_>>();
        for k in new_strikes {
            self.announce(match k { growing_guns::StrikeKind::AirStrike => "AIR STRIKE INCOMING", growing_guns::StrikeKind::IonCannon => "ION CANNON INCOMING" }, 1.2, 0.0);
        }
        for e in &world.events {
            match *e {
                Event::Hit { victim, attacker, head, damage, pos } => {
                    if let Some(f) = self.fb.get_mut(attacker) && attacker != victim {
                        f.hit = 0.0;
                        f.hit_kind = if head { HitKind::Head } else { HitKind::Body };
                        if damage > 0 { f.numbers.push(Number { pos, damage, head, age: 0.0 }); }
                    }
                    if let (Some(v), Some(a)) = (world.players.get(victim), world.players.get(attacker)) && attacker != victim {
                        let dx = a.pos - v.pos;
                        let (s, c) = v.yaw.sin_cos();
                        let (fwd, right) = (dx.x * -s + dx.z * -c, dx.x * c - dx.z * s);
                        if fwd * fwd + right * right > 1e-4 && let Some(f) = self.fb.get_mut(victim) { f.arcs.push((right.atan2(fwd), 0.28)); }
                    }
                }
                Event::Kill { victim, attacker, .. } => {
                    if let Some(a) = attacker.filter(|&a| a != victim) {
                        if let Some(f) = self.fb.get_mut(a) { f.hit = 0.0; f.hit_kind = HitKind::Kill; }
                        if humans >= 3 { let line = format!("{} => {}", self.name(a), self.name(victim)); self.feed.push((line, 0.0)); }
                        while self.feed.len() > 6 { self.feed.remove(0); }
                    } else if let Some(f) = self.fb.get_mut(victim) && !world.players[victim].bot {
                        // Lava and falls: the red screen (Godot `_lava_death`).
                        f.burned = Some(0.0);
                    }
                    if world.players.get(victim).is_some_and(|p| !p.bot) && !(world.coop && world.waves.is_enemy(victim)) {
                        let line = if world.coop && world.players[victim].downed { format!("{} is down", self.name(victim)) } else { format!("{} died", self.name(victim)) };
                        self.announce(line, 2.0, 0.0);
                    }
                }
                // Godot `trigger_explosion_sidechain` (bullet blasts: peak r / 5),
                // for every player from their own distance; ion strikes flash on their own.
                Event::Explosion { pos, radius, kind: growing_guns::ExplosionKind::Conventional, .. } => {
                    let peak = (radius / 5.0).clamp(0.35, 3.0);
                    for (f, p) in self.fb.iter_mut().zip(&world.players) {
                        let reach = (radius * 6.0).max(12.0);
                        let amount = ((1.0 - pos.distance(p.pos) / reach) * peak).clamp(0.0, peak.max(3.0));
                        if amount <= 0.0 { continue; }
                        f.flash[1] = f.flash[1].max((amount * 0.62).min(0.96));
                        f.flash[2] = f.flash[2].max(7.0 + amount * 4.0);
                    }
                }
                Event::PickedUp { player, kind } => {
                    let (t, s, c) = pickup_info(kind);
                    if let Some(f) = self.fb.get_mut(player) { f.toast = Some((t.into(), s.into(), c, 0.0)); }
                }
                Event::NewArena => {
                    self.round_age = 0.0;
                    if world.coop {
                        let mut parts = Vec::new();
                        for a in ["grunt", "grenadier", "fragger", "sniper", "bomber"] {
                            let n = world.waves.queue.iter().filter(|q| q.name() == a).count();
                            if n > 0 { parts.push(format!("{n}x {a}")); }
                        }
                        self.announce(format!("WAVE {}\n{}", world.waves.wave, parts.join(", ")), 3.0, 40.0);
                    }
                }
                Event::RoundOver { .. } if world.coop && world.phase == Phase::Picking => {
                    self.announce(format!("WAVE {} CLEARED", world.waves.wave), 1.4, 0.0);
                }
                _ => {}
            }
        }
        if clear_banner { self.banner = None; }
    }

    /// The HUD for local view `k` (world player `i`).
    pub fn view(&self, d: &mut HudDrawList, world: &World, i: usize, rect: [f32; 4], focus: usize, flash: f32, pad: bool) {
        let mut l = Layout::new(rect, d.size[1] as f32);
        l.controller = pad;
        let p = &world.players[i];
        let f = &self.fb[i];
        let clip = rect;
        let [x, y, w, h] = rect;
        let (cx, cy) = (x + w * 0.5, y + h * 0.5);
        if flash > 0.0 { d.rect(rect, [0.96, 0.99, 1.0, flash], clip); }
        let picking = p.offer.is_some() || f.exit.is_some();
        // Ghost tint (spectating after a 3+ player death).
        if p.ghost && !picking { d.rect(rect, [0.25, 0.55, 0.85, 0.28], clip); }
        if !picking {
            self.damage_numbers(d, world, i, rect);
            damage_arcs(d, f, &l);
            if p.alive && !p.downed { crosshair(d, world, i, &l); hit_marker(d, f, &l); }
            if p.alive || p.ghost { self.bottom_row(d, world, i, &l); }
        }
        // Coop: reviving a teammate.
        if world.coop && world.waves.reviving(i).is_some() && !picking {
            let size = (28.0 * l.s).round().max(16.0) * l.u;
            Label::new(BODY, size, [0.72, 0.92, 1.0, 1.0]).outline(5.0 * l.u, [0.0, 0.0, 0.0, 0.9]).center().draw(d, "REVIVING...", cx, y + h - 98.0 * l.u, clip);
        }
        if world.coop && p.downed && !picking {
            Label::new(BODY, 24.0 * l.u, [1.0, 0.55, 0.45, 1.0]).outline(5.0 * l.u, [0.0, 0.0, 0.0, 0.9]).center().draw(d, "DOWN — A TEAMMATE CAN REVIVE YOU", cx, y + h - 98.0 * l.u, clip);
        }
        self.toasts(d, world, i, &l);
        self.round_strip(d, world, i, &l);
        // The death overlay: red after a lava death, then the card pick's
        // dark backdrop (Godot `_blend_death_into_card_pick`).
        let death = death_color(world, p, f);
        // Above the HUD's text too (Godot z 25 over the panels' 10).
        if death[3] > 0.003 { d.begin_overlay(); d.rect(rect, death, clip); }
        if picking {
            d.begin_overlay();
            cards::pick(d, world, i, &l, f, focus);
        }
        // The explosion flash sits above everything (Godot canvas layer 50).
        if f.flash[0] > 0.003 { d.begin_overlay(); d.rect(rect, [1.0, 0.96, 0.88, f.flash[0]], clip); }
        let _ = cy;
    }

    fn bottom_row(&self, d: &mut HudDrawList, world: &World, i: usize, l: &Layout) {
        let p = &world.players[i];
        let f = &self.fb[i];
        let s = l.s;
        let font_big = (28.0 * s).round().max(16.0) * l.u;
        let font_small = (19.0 * s).round().max(13.0) * l.u;
        let panel_h = 36.0 * s * l.u;
        let y = l.rect[1] + l.rect[3] - (14.0 * s).round() * l.u - 38.0 * s * l.u;
        let cx = l.rect[0] + l.rect[2] * 0.5;
        let clip = l.rect;
        let ghost = p.ghost;
        let pad = l.controller;
        let panels: [(f32, f32); 4] = [(-290.0, font_big), (-140.0, font_big), (10.0, font_small), (160.0, font_small)];
        for (k, (px, font)) in panels.into_iter().enumerate() {
            let r = [cx + px * s * l.u, y, 130.0 * s * l.u, panel_h];
            // Each panel stands on its own slanted plate with the red edge.
            plate(d, [r[0] - 10.0 * l.u, r[1] - 4.0 * l.u, r[2] + 16.0 * l.u, r[3] + 8.0 * l.u], PLATE, Some((5.0 * l.u, RED)), clip);
            let label = Label::new(display(), font, BONE).outline(3.0 * l.u, [0.0, 0.0, 0.0, 0.9]);
            let mid = r[1] + r[3] * 0.5;
            match k {
                0 => {
                    let cs = r[3] * 0.85;
                    heart(d, [r[0], r[1] + (r[3] - cs) * 0.5, cs, cs], if ghost { [0.65, 0.85, 1.0, 1.0] } else { RED }, true, clip);
                    let text = if ghost { "GHOST".to_string() } else { p.health.max(0).to_string() };
                    label.draw(d, &text, r[0] + cs + 8.0 * l.u, mid, clip);
                }
                1 | 2 => {
                    let is = r[3] * 0.95;
                    let ir = [r[0], r[1] + (r[3] - is) * 0.5, is, is];
                    if k == 1 { bullet(d, ir, BONE, clip) } else { grenade(d, ir, BONE, clip) }
                    prompt(d, ir, match (k, pad) { (1, false) => "LMB", (1, true) => "RT", (_, false) => "RMB", _ => "LT" }, clip);
                    let (text, progress, fill) = if k == 1 {
                        let prog = if !ghost && p.reloading() { (1.0 - p.rifle_cd / p.weapon.reload_time().max(0.01)).clamp(0.0, 1.0) as f32 } else { 0.0 };
                        (if ghost { "GHOST".to_string() } else { format!("{} / {}", p.ammo, p.mag_size()) }, prog, BONE)
                    } else {
                        let name = if ghost { "MINE".to_string() } else if p.air_strike_charges > 0 {
                            if p.air_strike_charges == 1 { "AIR STRIKE".into() } else { format!("AIR STRIKE x{}", p.air_strike_charges) }
                        } else {
                            match p.weapon.special.name() { "cluster_grenade" => "CLUSTER".into(), n => n.replace('_', " ").to_uppercase() }
                        };
                        let prog = if !ghost && p.special_cd > 0.0 && f.special_max > 0.0 { (1.0 - p.special_cd / f.special_max).clamp(0.0, 1.0) as f32 } else { 0.0 };
                        (name, prog, RED)
                    };
                    let cx0 = r[0] + is + 8.0 * l.u;
                    let cw = (r[0] + r[2] - cx0).max(0.0);
                    if progress > 0.0 {
                        let (bh, by) = (r[3] * 0.5, r[1] + r[3] * 0.25);
                        d.rect([cx0, by, cw, bh], [0.0, 0.0, 0.0, 0.55], clip);
                        d.rect([cx0, by, cw * progress, bh], fill, clip);
                    } else {
                        let size = fit(&text, display(), font, cw.max(1.0) * 1.15);
                        Label { size, ..label }.draw(d, &text, cx0, mid, clip);
                    }
                }
                _ => {
                    let is = r[3] * 0.95;
                    let ir = [r[0], r[1] + (r[3] - is) * 0.5, is, is];
                    dash(d, ir, BONE, clip);
                    prompt(d, ir, if pad { "RB" } else { "SHIFT" }, clip);
                    let sx = r[0] + is + 8.0 * l.u;
                    let total = (r[0] + r[2] - sx).max(0.0);
                    let gap = 6.0 * l.u;
                    let sw = (total - gap) * 0.5;
                    let (bh, by) = (r[3] * 0.5, r[1] + r[3] * 0.25);
                    for b in 0..MAX_DASH_CHARGES {
                        let fill = if b < p.dash_charges { 1.0 } else if b == p.dash_charges { (p.dash_recharge / growing_guns::DASH_RECHARGE_TIME).clamp(0.0, 1.0) as f32 } else { 0.0 };
                        let bx = sx + b as f32 * (sw + gap);
                        d.rect([bx, by, sw, bh], [0.0, 0.0, 0.0, 0.55], clip);
                        d.rect([bx, by, sw * fill, bh], BONE, clip);
                    }
                }
            }
        }
    }

    fn damage_numbers(&self, d: &mut HudDrawList, world: &World, i: usize, rect: [f32; 4]) {
        let f = &self.fb[i];
        if f.numbers.is_empty() { return; }
        let cam = crate::view::camera(world, i, &[], 1.0);
        for (k, n) in f.numbers.iter().enumerate() {
            // Rises 0.9 m with a cubic ease-out and fades after 0.45 s, from a
            // jittered start 0.4 m above the hit.
            let j = (k as u32).wrapping_mul(2654435761) ^ (n.damage as u32).wrapping_mul(40503);
            let jitter = glam::Vec3::new(((j & 255) as f32 / 255.0 - 0.5) * 0.7, 0.05 + ((j >> 8 & 255) as f32 / 255.0) * 0.3, ((j >> 16 & 255) as f32 / 255.0 - 0.5) * 0.7);
            let t = n.age.clamp(0.0, 1.0);
            let rise = 1.0 - (1.0 - t).powi(3);
            let pos = n.pos + glam::Vec3::Y * 0.4 + jitter + glam::Vec3::Y * 0.9 * rise;
            let Some((sx, sy, dist)) = project(&cam, rect, pos) else { continue };
            let a = if n.age < 0.45 { 1.0 } else { 1.0 - ((n.age - 0.45) / 0.55 * std::f32::consts::FRAC_PI_2).sin().powi(1).min(1.0) * 1.0 };
            let a = a.clamp(0.0, 1.0);
            // 64 px font at 0.004 m per pixel: 0.256 m tall in the world.
            let px = 0.256 / (dist * (cam.fov_y * 0.5).tan() * 2.0) * rect[3];
            if px < 4.0 { continue; }
            let c = if n.head { [1.0, 0.55, 0.45, a] } else { [1.0, 0.95, 0.75, a] };
            Label::new(BODY, px.min(160.0), c).outline(px * 0.25, [0.0, 0.0, 0.0, 1.0]).center().draw(d, &n.damage.to_string(), sx, sy, rect);
        }
    }

    fn toasts(&self, d: &mut HudDrawList, world: &World, i: usize, l: &Layout) {
        let f = &self.fb[i];
        let cx = l.rect[0] + l.rect[2] * 0.5;
        let top = l.rect[1];
        let clip = l.rect;
        // Round modifier: 2.5 s at the top of each view.
        if let Some(m) = world.modifier && self.round_age < 2.5 && !world.coop {
            let (t, s, c) = modifier_info(m);
            let lab = Label::new(BODY, 24.0 * l.u, c).outline(5.0 * l.u, [0.0, 0.0, 0.0, 0.92]).center();
            lab.draw(d, &format!("{t}\n{s}"), cx, top + 76.0 * l.u, clip);
        }
        if let Some((t, s, c, _)) = &f.toast {
            let text = if s.is_empty() { t.clone() } else { format!("{t}\n{}", prompt_text(s, l.controller)) };
            let lab = Label::new(BODY, 22.0 * l.u, *c).outline(5.0 * l.u, [0.0, 0.0, 0.0, 0.92]).center();
            let w = lab.width(&text).max(360.0 * l.u - 36.0 * l.u) + 36.0 * l.u;
            let (th, ty) = (60.0 * l.u, top + 72.0 * l.u);
            plate(d, [cx - w * 0.5, ty, w, th], PLATE, Some((5.0 * l.u, RED)), clip);
            lab.draw(d, &text, cx, ty + th * 0.5, clip);
        }
    }

    /// The per-view strip at the top: who won the round, then who we wait for.
    fn round_strip(&self, d: &mut HudDrawList, world: &World, i: usize, l: &Layout) {
        let p = &world.players[i];
        if p.offer.is_some() || world.phase == Phase::MatchOver { return; }
        let text = if world.coop {
            if world.phase == Phase::Picking && p.offer.is_none() {
                let names = self.pickers(world);
                if names.is_empty() { None } else { Some(format!("WAVE CLEARED — WAITING FOR {}…", names.join(", "))) }
            } else { None }
        } else if world.round_over() && world.phase != Phase::Picking {
            let won = world.round_winner();
            match won {
                Some(w) if w == i => Some("YOU WIN THE ROUND".to_string()),
                Some(w) if p.alive => Some(format!("{} WINS THE ROUND", self.name(w))),
                _ => None,
            }
        } else if world.phase == Phase::Picking {
            let names = self.pickers(world);
            if world.round_winner() == Some(i) {
                Some(if names.is_empty() { "ROUND WON — WAITING FOR PICKS…".to_string() } else { format!("ROUND WON — WAITING FOR {}…", names.join(", ")) })
            } else { None }
        } else { None };
        if let Some(t) = text {
            let lab = Label::new(display(), 30.0 * l.u, BONE).shadow(3.0 * l.u, RED).center();
            let size = fit(&t, display(), lab.size, l.rect[2] * 0.96);
            Label { size, ..lab }.draw(d, &t, l.rect[0] + l.rect[2] * 0.5, l.rect[1] + 32.0 * l.u, l.rect);
        }
    }

    /// Humans still owing a card pick.
    pub fn pickers(&self, world: &World) -> Vec<String> {
        world.players.iter().enumerate().filter(|(_, p)| !p.bot && p.offer.is_some()).map(|(k, _)| self.name(k).to_string()).collect()
    }

    /// Shared overlay: the round banner, kill feed, coop count, match over.
    /// `net`: a network error for Godot's red panel at the top right.
    pub fn global(&self, d: &mut HudDrawList, world: &World, net: Option<&str>) {
        let [w, h] = d.size.map(|v| v as f32);
        let u = h / DESIGN_H;
        let clip = [0.0, 0.0, w, h];
        // Round start: a white flash, held 0.25 s then faded over 0.45 s.
        if self.round_age < 0.7 && world.round > 1 {
            let a = if self.round_age < 0.25 { 1.0 } else { 1.0 - ((self.round_age - 0.25) / 0.45).clamp(0.0, 1.0).powi(2) };
            d.rect(clip, [1.0, 0.98, 0.94, a * 0.9], clip);
        }
        // Coop: KILLED n/m, top right (Godot's scoreboard label).
        if world.coop {
            let lab = Label::new(BODY, 24.0 * u, [1.0, 0.8, 0.4, 1.0]).outline(4.0 * u, BLACK).right();
            lab.draw(d, &format!("KILLED   {}/{}", world.waves.kills, world.waves.total), w - 20.0 * u, 34.0 * u, clip);
        }
        // Kill feed: top right from y 96, newest last, fading after 3 s.
        for (k, (line, age)) in self.feed.iter().enumerate() {
            let a = if *age < 3.0 { 1.0 } else { 1.0 - (age - 3.0) / 0.35 };
            let lab = Label::new(BODY, 16.0 * u, with_alpha([1.0, 0.94, 0.8, 1.0], a)).outline(4.0 * u, with_alpha(BLACK, 0.95)).right();
            let tw = lab.width(line) + 20.0 * u;
            let (ex, ey, eh) = (w - 18.0 * u - tw, 96.0 * u + k as f32 * 36.0 * u, 30.0 * u);
            d.rect([ex, ey, tw, eh], [0.02, 0.025, 0.03, 0.78 * a], clip);
            frame_rect(d, [ex, ey, tw, eh], u.max(1.0), [1.0, 0.78, 0.35, 0.7 * a], clip);
            lab.draw(d, line, w - 28.0 * u, ey + eh * 0.5, clip);
        }
        let mut banner = self.banner.clone();
        let mut pop = 1.0;
        if let Some(t) = self.match_over {
            let text = if world.coop { format!("WAVE {} FAILED", world.waves.wave) } else {
                match world.round_winner().or_else(|| world.players.iter().position(|p| p.score >= world.rounds_to_win)) {
                    Some(k) if !world.players[k].bot && world.players.iter().filter(|p| !p.bot).count() == 1 => "YOU'RE THE WINNER".to_string(),
                    Some(k) => format!("{} WINS THE MATCH", self.name(k)),
                    None => "MATCH OVER".to_string(),
                }
            };
            banner = Some(Banner { text, size: 92.0, left: 99.0 });
            // Pop in from 0.4 with an overshoot over 0.55 s, then breathe.
            pop = if t < 0.55 { let k = t / 0.55; 0.4 + 0.6 * back_out(k) } else { 1.0 + 0.05 * (0.5 - 0.5 * ((t - 0.55) / 0.9 * std::f32::consts::PI).cos()) };
            if t < 0.3 { pop *= 1.0; }
        }
        if let Some(b) = banner {
            let size = if b.size > 0.0 { b.size } else { 56.0 } * u * pop;
            let alpha = self.match_over.map_or(1.0, |t| (t / 0.3).min(1.0));
            let lab = Label::new(BODY, size, [1.0, 0.95, 0.5, alpha]).outline(6.0 * u * pop, BLACK).center();
            let lines = b.text.lines().count().max(1) as f32;
            let size = fit(b.text.lines().max_by_key(|l| l.len()).unwrap_or(""), BODY, size, w * 0.98);
            Label { size, ..lab }.draw(d, &b.text, w * 0.5, 115.0 * u + (lines - 1.0) * size * 0.6, clip);
        }
        if let Some(net) = net {
            // 422 wide from 18 in at the top right, 14 x 10 margins, wrapped 15 px text.
            let pw = 422.0 * u;
            let lines = cards::wrap(net, BODY, 15.0 * u, pw - 28.0 * u, 6);
            let ph = lines.len() as f32 * 18.0 * u + 20.0 * u;
            let pr = [w - 18.0 * u - pw, 18.0 * u, pw, ph];
            d.rect(pr, [0.22, 0.03, 0.035, 0.94], clip);
            frame_rect(d, pr, 2.0 * u, [1.0, 0.35, 0.28, 0.9], clip);
            let lab = Label::new(BODY, 15.0 * u, [1.0, 0.84, 0.78, 1.0]).outline(4.0 * u, with_alpha(BLACK, 0.95));
            for (k, line) in lines.iter().enumerate() { lab.draw(d, line, pr[0] + 14.0 * u, pr[1] + 10.0 * u + (k as f32 + 0.5) * 18.0 * u, clip); }
        }
    }
}

fn back_out(t: f32) -> f32 { let (c1, c3) = (1.70158, 2.70158); 1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2) }

/// Mouse names in prompts rewritten for a pad (Godot `hud_icon.gd`).
pub fn prompt_text(s: &str, pad: bool) -> String {
    if !pad { return s.to_string(); }
    s.replace("RMB", "LT").replace("LMB", "RT").replace("Shift", "RB").replace("SHIFT", "RB")
}

/// A view's layout: its rectangle, pixels per design unit and Godot's
/// layout scale (clamp(min(w/960, h/540), 0.58, 1)).
pub struct Layout { pub rect: [f32; 4], pub u: f32, pub s: f32, pub controller: bool }

impl Layout {
    pub fn new(rect: [f32; 4], window_h: f32) -> Self {
        let u = (window_h / DESIGN_H).max(0.1);
        let (vw, vh) = (rect[2] / u, rect[3] / u);
        Layout { rect, u, s: (vw / 960.0).min(vh / 540.0).clamp(0.58, 1.0), controller: false }
    }
}

/// Screen position of a world point in a view, with its distance.
pub fn project(cam: &render::Camera, rect: [f32; 4], p: glam::Vec3) -> Option<(f32, f32, f32)> {
    let fwd = (cam.target - cam.pos).normalize_or_zero();
    let right = fwd.cross(glam::Vec3::Y).normalize_or_zero();
    let up = right.cross(fwd);
    let v = p - cam.pos;
    let z = v.dot(fwd);
    if z < 0.1 { return None; }
    let t = (cam.fov_y * 0.5).tan();
    let aspect = rect[2] / rect[3].max(1.0);
    let (nx, ny) = (v.dot(right) / (z * t * aspect), v.dot(up) / (z * t));
    if nx.abs() > 1.2 || ny.abs() > 1.2 { return None; }
    Some((rect[0] + (nx * 0.5 + 0.5) * rect[2], rect[1] + (0.5 - ny * 0.5) * rect[3], v.length()))
}

/// Godot crosshair: four yellow strokes opening with the effective spread.
fn crosshair(d: &mut HudDrawList, world: &World, i: usize, l: &Layout) {
    let (cx, cy) = (l.rect[0] + l.rect[2] * 0.5, l.rect[1] + l.rect[3] * 0.5);
    let s = l.s;
    let width = (1.5 * s).max(1.0) * l.u;
    let len = (8.0 * s).max(5.0) * l.u;
    let gap = ((4.0 * s).max(3.0) + world.effective_spread(i) * 400.0) * l.u;
    let c = [1.0, 1.0, 0.4, 0.9];
    let clip = l.rect;
    d.line([cx, cy - gap], [cx, cy - gap - len], width, c, clip);
    d.line([cx, cy + gap], [cx, cy + gap + len], width, c, clip);
    d.line([cx - gap, cy], [cx - gap - len, cy], width, c, clip);
    d.line([cx + gap, cy], [cx + gap + len, cy], width, c, clip);
}

/// Hit marker: four diagonal strokes that scale out and fade (0.13/0.16 s).
fn hit_marker(d: &mut HudDrawList, f: &Feedback, l: &Layout) {
    let t = f.hit;
    if t >= 0.16 { return; }
    let k = (t / 0.16).clamp(0.0, 1.0);
    let alpha = 1.0 - k.powi(3);
    let e = (t / 0.13).clamp(0.0, 1.0);
    let spread = 0.7 + 0.65 * if e >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * e) };
    let col = match f.hit_kind { HitKind::Head => [1.0, 0.9, 0.25], HitKind::Kill => [1.0, 0.25, 0.22], HitKind::Body => [1.0, 1.0, 1.0] };
    let (cx, cy) = (l.rect[0] + l.rect[2] * 0.5, l.rect[1] + l.rect[3] * 0.5);
    let (inner, outer) = (5.0 * spread * l.u, 13.0 * spread * l.u);
    for pass in 0..2 {
        for (dx, dy) in [(0.7071f32, 0.7071f32), (-0.7071, 0.7071), (-0.7071, -0.7071), (0.7071, -0.7071)] {
            let (c, w) = if pass == 0 { ([0.0, 0.0, 0.0, (alpha * 1.1).min(1.0)], (2.4 + 1.6) * l.u) } else { ([col[0], col[1], col[2], alpha], 2.4 * l.u) };
            d.line([cx + dx * inner, cy + dy * inner], [cx + dx * outer, cy + dy * outer], w, c, l.rect);
        }
    }
}

/// Red arcs toward whoever hit us (Godot `damage_indicator.gd`).
fn damage_arcs(d: &mut HudDrawList, f: &Feedback, l: &Layout) {
    let (cx, cy) = (l.rect[0] + l.rect[2] * 0.5, l.rect[1] + l.rect[3] * 0.5);
    let radius = l.rect[2].min(l.rect[3]) * 0.42;
    for &(angle, life) in &f.arcs {
        let t = (life / 0.28).clamp(0.0, 1.0);
        let a = angle - std::f32::consts::FRAC_PI_2;
        d.arc([cx, cy], radius, a - 0.38, a + 0.38, 26.0 * l.u, [1.0, 0.12, 0.14, t * t * 0.65], l.rect);
    }
}

/// The death overlay's colour now.
fn death_color(world: &World, p: &growing_guns::Player, f: &Feedback) -> C {
    let ease = |t: f32| 0.5 - 0.5 * (t.clamp(0.0, 1.0) * std::f32::consts::PI).cos();
    let mut c = [0.8, 0.0, 0.0, 0.0];
    if let Some(t) = f.burned { c = lerp([0.8, 0.0, 0.0, 0.6], [0.65, 0.0, 0.0, 1.0], ease(t / 0.4)); }
    if p.offer.is_some() {
        let t = p.offer_age as f32;
        if world.coop { c = lerp(c, [0.05, 0.05, 0.08, 0.5], ease(t / 0.6)); } else {
            c = lerp(c, [0.30, 0.0, 0.0, 1.0], ease(t / 0.55));
            if t > 0.55 { c = lerp([0.30, 0.0, 0.0, 1.0], [0.09, 0.0, 0.0, 1.0], ease((t - 0.55) / 0.65)); }
        }
    } else if f.pick_closed < 0.18 {
        let from = if world.coop { [0.05, 0.05, 0.08, 0.5] } else { [0.09, 0.0, 0.0, 1.0] };
        let k = 1.0 - (1.0 - f.pick_closed / 0.18).powi(3);
        c = with_alpha(from, 1.0 - k);
    } else if f.burned.is_none() { c[3] = 0.0; }
    let _ = PLAYER_COLORS_U8;
    c
}
