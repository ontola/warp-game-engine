//! Per-view HUD: crosshair, hit marker, health, dash charges, ammo, a 2D
//! first-person rifle, damage flash; plus the shared scoreboard and round
//! banner. All shapes, no textures.
use crate::view::PLAYER_COLORS_U8;
use growing_guns::{World, MAX_DASH_CHARGES};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};

/// Per local player feedback timers kept by the app.
#[derive(Clone, Copy, Default)]
pub struct Feedback { pub hit: f32, pub headshot: bool, pub hurt: f32, pub bob: f32 }

const WHITE: [u8; 4] = [240, 240, 235, 255];
const MUTED: [u8; 4] = [200, 200, 195, 200];

fn text(d: &mut HudDrawList, id: u64, s: String, pos: [f32; 2], size: f32, color: [u8; 4], align: TextAlign, width: f32, clip: [f32; 4]) {
    let x = match align { TextAlign::Left => pos[0], TextAlign::Center => pos[0] - width * 0.5, TextAlign::Right => pos[0] - width };
    d.text.push(TextRun { id, text: s, position: [x, pos[1]], bounds: clip, font_size: size, color, font: FontFace::Body, align, width });
}

pub fn view(d: &mut HudDrawList, world: &World, i: usize, rect: [f32; 4], fb: &Feedback, card_focus: usize) {
    let [x, y, w, h] = rect;
    let clip = rect;
    let s = (h / 1080.0).max(0.35);
    let p = &world.players[i];
    let id = 1000 * (i as u64 + 1);
    let (cx, cy) = (x + w * 0.5, y + h * 0.5);
    if let Some(offer) = &p.offer {
        let cards: Vec<app::ui::Card> = offer.iter().map(|&c| match world.cards.get(c) {
            Some(d) => app::ui::Card { title: &d.name, body: &d.desc, color: d.color },
            None => app::ui::Card { title: "?", body: "", color: [1.0; 3] },
        }).collect();
        let humans = world.players.iter().filter(|p| !p.bot).count();
        let footer = if humans > 1 {
            format!("{:.0} s   (stick or arrows, A or Space to take)", (growing_guns::CARD_PICK_TIMEOUT - p.offer_age).max(0.0).ceil())
        } else { "stick or arrows, A or Space to take".into() };
        app::ui::cards(d, id + 200, rect, "PICK A CARD", &cards, card_focus, &footer, &app::ui::Style::for_height(h));
        return;
    }
    if world.coop && p.downed {
        text(d, id, "DOWN: hold near to revive".into(), [cx, cy - 40.0 * s], 48.0 * s, [255, 90, 70, 255], TextAlign::Center, w, clip);
        if let Some(r) = world.waves.downed(i).filter(|r| r.reviver.is_some()) {
            let f = (r.progress / growing_guns::coop::REVIVE_SECONDS).clamp(0.0, 1.0);
            text(d, id + 1, format!("REVIVING  {:.0}%", f * 100.0), [cx, cy + 24.0 * s], 30.0 * s, [140, 220, 255, 255], TextAlign::Center, w, clip);
        }
        return;
    }
    if !p.alive {
        let until = if world.coop { "spectating until the wave ends" } else { "spectating until the round ends" };
        text(d, id, "ELIMINATED".into(), [cx, cy - 40.0 * s], 54.0 * s, [255, 90, 70, 255], TextAlign::Center, w, clip);
        text(d, id + 1, until.into(), [cx, cy + 24.0 * s], 26.0 * s, MUTED, TextAlign::Center, w, clip);
        return;
    }
    // Coop: reviving a teammate, or one waiting for it.
    if world.coop {
        let line = match world.waves.reviving(i) {
            Some((v, f)) => Some((format!("REVIVING P{}  {:.0}%", v + 1, f * 100.0), [140, 220, 255, 255])),
            None => world.waves.downed.first().map(|v| (format!("P{} DOWN: hold near to revive", v.player + 1), [255, 150, 120, 255])),
        };
        if let Some((line, c)) = line { text(d, id + 7, line, [cx, cy + 70.0 * s], 28.0 * s, c, TextAlign::Center, w, clip); }
    }
    // Damage flash: red edges.
    if fb.hurt > 0.0 {
        let a = (fb.hurt * 2.0).min(0.45);
        let e = 60.0 * s;
        let red = [0.8, 0.05, 0.02, a];
        d.rect([x, y, w, e], red, clip);
        d.rect([x, y + h - e, w, e], red, clip);
        d.rect([x, y, e, h], red, clip);
        d.rect([x + w - e, y, e, h], red, clip);
    }
    // Crosshair, opening with recoil bloom.
    let gap = (6.0 + p.bloom * 900.0) * s;
    let len = 12.0 * s;
    let cw = 2.5 * s.max(0.6);
    let cc = [1.0, 1.0, 1.0, 0.9];
    d.line([cx - gap - len, cy], [cx - gap, cy], cw, cc, clip);
    d.line([cx + gap, cy], [cx + gap + len, cy], cw, cc, clip);
    d.line([cx, cy - gap - len], [cx, cy - gap], cw, cc, clip);
    d.line([cx, cy + gap], [cx, cy + gap + len], cw, cc, clip);
    d.circle([cx, cy], 1.8 * s.max(0.6), cc, clip);
    if fb.hit > 0.0 {
        let a = (fb.hit / 0.25).min(1.0);
        let c = if fb.headshot { [1.0, 0.3, 0.2, a] } else { [1.0, 1.0, 1.0, a] };
        let (r0, r1) = (10.0 * s, 22.0 * s);
        for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            d.line([cx + dx * r0, cy + dy * r0], [cx + dx * r1, cy + dy * r1], 3.0 * s.max(0.6), c, clip);
        }
    }
    // Health bar and dash pips, bottom left.
    let (bx, by, bw, bh) = (x + 36.0 * s, y + h - 70.0 * s, 300.0 * s, 22.0 * s);
    d.rect([bx - 3.0 * s, by - 3.0 * s, bw + 6.0 * s, bh + 6.0 * s], [0.0, 0.0, 0.0, 0.55], clip);
    // Coop humans have triple health.
    let full = if world.coop && !p.bot { (p.max_health() as f64 * growing_guns::coop::HUMAN_HEALTH_MULT).ceil() as i32 } else { p.max_health() };
    let frac = (p.health.max(0) as f32 / full.max(1) as f32).clamp(0.0, 1.0);
    let hc = if frac > 0.5 { [0.35, 0.85, 0.35, 0.95] } else if frac > 0.25 { [0.95, 0.75, 0.2, 0.95] } else { [0.95, 0.25, 0.2, 0.95] };
    d.rect([bx, by, bw * frac, bh], hc, clip);
    text(d, id + 2, format!("{}", p.health.max(0)), [bx + 8.0 * s, by - 2.0 * s], 22.0 * s, WHITE, TextAlign::Left, 100.0 * s, clip);
    for k in 0..MAX_DASH_CHARGES {
        let c = if k < p.dash_charges { [0.45, 0.8, 1.0, 0.95] } else { [0.2, 0.25, 0.3, 0.7] };
        d.circle([bx + 10.0 * s + k as f32 * 26.0 * s, by - 22.0 * s], 8.0 * s, c, clip);
    }
    // Ammo, bottom right.
    let (ax, ay) = (x + w - 40.0 * s, y + h - 60.0 * s);
    if p.reloading() {
        text(d, id + 3, "RELOADING".into(), [ax, ay - 10.0 * s], 28.0 * s, [255, 210, 120, 255], TextAlign::Right, 300.0 * s, clip);
    } else {
        let mag = p.mag_size();
        let pip = if mag > 10 { 6.0 * s } else { 16.0 * s };
        for k in 0..mag {
            let c = if k < p.ammo { [1.0, 0.85, 0.35, 0.95] } else { [0.25, 0.22, 0.18, 0.7] };
            d.rect([ax - (k + 1) as f32 * pip, ay, pip * 0.56, 30.0 * s], c, clip);
        }
    }
    // Special (RMB), bottom left: name and cooldown.
    let sp = if p.air_strike_charges > 0 { format!("AIR STRIKE x{}", p.air_strike_charges) } else { p.weapon.special.name().replace('_', " ").to_uppercase() };
    let ready = p.special_cd <= 0.0;
    let line = if ready { sp } else { format!("{sp}  {:.1}", p.special_cd) };
    text(d, id + 5, line, [x + 40.0 * s, y + h - 60.0 * s], 26.0 * s, if ready { [140, 220, 255, 255] } else { MUTED }, TextAlign::Left, 400.0 * s, clip);
    text(d, id + 6, format!("{} HP", p.health), [x + 40.0 * s, y + h - 96.0 * s], 30.0 * s, WHITE, TextAlign::Left, 300.0 * s, clip);
    // Player tag, top left of the view.
    text(d, id + 4, format!("P{}", i + 1), [x + 20.0 * s, y + 16.0 * s], 30.0 * s, PLAYER_COLORS_U8[i % 4], TextAlign::Left, 100.0 * s, clip);
}

/// Shared overlay: scores across the top, the round banner during intermission.
pub fn global(d: &mut HudDrawList, world: &World, net: Option<&str>) {
    let [w, h] = d.size.map(|v| v as f32);
    let s = (h / 1080.0).max(0.4);
    let clip = [0.0, 0.0, w, h];
    // Coop: the humans only (enemies come and go), and the wave.
    let shown: Vec<usize> = (0..world.players.len()).filter(|&i| !world.coop || (i < world.waves.seats && !world.players[i].bot)).collect();
    let n = shown.len() as f32;
    let cell = 92.0 * s;
    let x0 = w * 0.5 - n * cell * 0.5;
    d.rect([x0 - 8.0 * s, 0.0, n * cell + 16.0 * s, 50.0 * s], [0.0, 0.0, 0.0, 0.5], clip);
    for (k, &i) in shown.iter().enumerate() {
        let p = &world.players[i];
        let mut c = PLAYER_COLORS_U8[i % 4];
        if !p.alive || p.downed { c[3] = 110; }
        let tag = if p.bot { "B" } else { "P" };
        let label = if world.coop { format!("{tag}{}", i + 1) } else { format!("{tag}{} {}", i + 1, p.score) };
        text(d, 10 + k as u64, label, [x0 + cell * (k as f32 + 0.5), 8.0 * s], 28.0 * s, c, TextAlign::Center, cell, clip);
    }
    let line = if world.coop { format!("WAVE {}  killed {}/{}", world.waves.wave, world.waves.kills, world.waves.total) } else { format!("ROUND {}", world.round) };
    text(d, 20, line, [w * 0.5, 54.0 * s], 20.0 * s, MUTED, TextAlign::Center, 300.0 * s, clip);
    if world.phase == growing_guns::Phase::Picking && world.players.iter().any(|p| p.offer.is_some()) {
        text(d, 23, "waiting for card picks".into(), [w * 0.5, 80.0 * s], 24.0 * s, MUTED, TextAlign::Center, w, clip);
    }
    if let Some(t) = world.cage_left() {
        let line = match world.modifier { Some(m) => format!("{}  {}", m.title(), m.subtitle()), None => format!("{:.0}", t.ceil()) };
        text(d, 24, line, [w * 0.5, h * 0.22], 40.0 * s, WHITE, TextAlign::Center, w, clip);
    }
    let coop_banner = match world.phase {
        growing_guns::Phase::MatchOver if world.coop => Some((format!("WAVE {} FAILED", world.waves.wave), [255, 90, 70, 255])),
        growing_guns::Phase::Picking if world.coop => Some((format!("WAVE {} CLEARED", world.waves.wave), WHITE)),
        _ => None,
    };
    if let Some((msg, c)) = coop_banner {
        d.rect([0.0, h * 0.12 - 10.0 * s, w, 100.0 * s], [0.0, 0.0, 0.0, 0.55], clip);
        text(d, 21, msg, [w * 0.5, h * 0.12], 52.0 * s, c, TextAlign::Center, w, clip);
    } else if world.round_over() {
        let winner = world.round_winner();
        let msg = match winner {
            Some(i) if world.phase == growing_guns::Phase::MatchOver => format!("P{} WINS THE MATCH", i + 1),
            Some(i) => format!("P{} WINS THE ROUND", i + 1),
            None => "DRAW".into(),
        };
        let c = winner.map_or(WHITE, |i| PLAYER_COLORS_U8[i % 4]);
        d.rect([0.0, h * 0.12 - 10.0 * s, w, 100.0 * s], [0.0, 0.0, 0.0, 0.55], clip);
        text(d, 21, msg, [w * 0.5, h * 0.12], 52.0 * s, c, TextAlign::Center, w, clip);
    }
    if let Some(net) = net {
        text(d, 22, net.into(), [w - 16.0 * s, h - 34.0 * s], 18.0 * s, MUTED, TextAlign::Right, w * 0.6, clip);
    }
}
