//! Per-view HUD: crosshair, hit marker, health, dash charges, ammo, a 2D
//! first-person rifle, damage flash; plus the shared scoreboard and round
//! banner. All shapes, no textures.
use crate::view::PLAYER_COLORS_U8;
use growing_guns::{World, MAX_DASH_CHARGES, RELOAD_TIME, WALK_SPEED};
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
    if let Some(offer) = p.offer {
        let cards: Vec<app::ui::Card> = offer.iter().map(|c| app::ui::Card { title: c.name(), body: c.text(), color: c.color() }).collect();
        let footer = format!("{:.0} s   (stick or arrows, A or Space to take)", world.pick_time.ceil());
        app::ui::cards(d, id + 200, rect, "PICK A CARD", &cards, card_focus, &footer, &app::ui::Style::for_height(h));
        return;
    }
    if !p.alive {
        text(d, id, "ELIMINATED".into(), [cx, cy - 40.0 * s], 54.0 * s, [255, 90, 70, 255], TextAlign::Center, w, clip);
        text(d, id + 1, "spectating until the round ends".into(), [cx, cy + 24.0 * s], 26.0 * s, MUTED, TextAlign::Center, w, clip);
        return;
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
    // 2D rifle, bottom right: kicks back on a shot, bobs while running.
    let kick = (1.0 - p.since_shot / 0.12).max(0.0);
    let speed = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt() / WALK_SPEED;
    let bob = if p.on_floor { fb.bob.sin() * 10.0 * s * speed.min(1.0) } else { -6.0 * s };
    let reload = if p.reloading() { (std::f32::consts::PI * (1.0 - p.reload_timer * p.mods.reload / RELOAD_TIME)).sin() } else { 0.0 };
    let gx = x + w * 0.66 + kick * 18.0 * s + (fb.bob * 0.5).sin() * 6.0 * s * speed.min(1.0);
    let gy = y + h * 0.78 + kick * 26.0 * s + bob + reload * 120.0 * s;
    let gun_dark = [0.10, 0.10, 0.11, 1.0];
    let gun_mid = [0.20, 0.20, 0.22, 1.0];
    let wood = [0.42, 0.26, 0.14, 1.0];
    let u = 1.15 * s;
    let poly = |d: &mut HudDrawList, pts: &[[f32; 2]], c: [f32; 4]| {
        for k in 1..pts.len() - 1 {
            d.triangle([pts[0], pts[k], pts[k + 1]].map(|q| [gx + q[0] * u, gy + q[1] * u]), c, clip);
        }
    };
    poly(d, &[[-60.0, -30.0], [50.0, 10.0], [40.0, 40.0], [-70.0, 0.0]], gun_dark); // handguard
    poly(d, &[[-175.0, -72.0], [-50.0, -22.0], [-56.0, -8.0], [-180.0, -58.0]], gun_dark); // barrel
    poly(d, &[[30.0, 0.0], [230.0, 70.0], [215.0, 115.0], [15.0, 40.0]], gun_mid); // receiver
    poly(d, &[[120.0, 50.0], [165.0, 66.0], [150.0, 170.0], [105.0, 160.0]], wood); // grip
    poly(d, &[[220.0, 70.0], [420.0, 120.0], [400.0, 230.0], [205.0, 115.0]], wood); // stock
    poly(d, &[[40.0, -8.0], [140.0, 28.0], [136.0, 40.0], [36.0, 4.0]], [0.05, 0.05, 0.05, 1.0]); // sight rail
    if kick > 0.4 {
        let mf = [1.0, 0.85, 0.35, (kick - 0.4) * 1.6];
        d.circle([gx - 182.0 * u, gy - 66.0 * u], 22.0 * u * kick, mf, clip);
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
    let frac = (p.health.max(0) as f32 / p.max_health() as f32).clamp(0.0, 1.0);
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
    // Player tag, top left of the view.
    text(d, id + 4, format!("P{}", i + 1), [x + 20.0 * s, y + 16.0 * s], 30.0 * s, PLAYER_COLORS_U8[i % 4], TextAlign::Left, 100.0 * s, clip);
}

/// Shared overlay: scores across the top, the round banner during intermission.
pub fn global(d: &mut HudDrawList, world: &World, net: Option<&str>) {
    let [w, h] = d.size.map(|v| v as f32);
    let s = (h / 1080.0).max(0.4);
    let clip = [0.0, 0.0, w, h];
    let n = world.players.len() as f32;
    let cell = 92.0 * s;
    let x0 = w * 0.5 - n * cell * 0.5;
    d.rect([x0 - 8.0 * s, 0.0, n * cell + 16.0 * s, 50.0 * s], [0.0, 0.0, 0.0, 0.5], clip);
    for (i, p) in world.players.iter().enumerate() {
        let mut c = PLAYER_COLORS_U8[i % 4];
        if !p.alive { c[3] = 110; }
        let tag = if p.bot { "B" } else { "P" };
        text(d, 10 + i as u64, format!("{tag}{} {}", i + 1, p.score), [x0 + cell * (i as f32 + 0.5), 8.0 * s], 28.0 * s, c, TextAlign::Center, cell, clip);
    }
    text(d, 20, format!("ROUND {}", world.round), [w * 0.5, 54.0 * s], 20.0 * s, MUTED, TextAlign::Center, 300.0 * s, clip);
    if world.ended && world.intermission <= 0.0 {
        d.rect([0.0, 0.0, w, 40.0 * s], [0.0, 0.0, 0.0, 0.0], clip);
        text(d, 23, format!("waiting for card picks  {:.0} s", world.pick_time.ceil()), [w * 0.5, 80.0 * s], 24.0 * s, MUTED, TextAlign::Center, w, clip);
    }
    if world.intermission > 0.0 {
        let winner = world.players.iter().position(|p| p.alive);
        let msg = match winner { Some(i) => format!("P{} WINS THE ROUND", i + 1), None => "DRAW".into() };
        let c = winner.map_or(WHITE, |i| PLAYER_COLORS_U8[i % 4]);
        d.rect([0.0, h * 0.12 - 10.0 * s, w, 100.0 * s], [0.0, 0.0, 0.0, 0.55], clip);
        text(d, 21, msg, [w * 0.5, h * 0.12], 52.0 * s, c, TextAlign::Center, w, clip);
    }
    if let Some(net) = net {
        text(d, 22, net.into(), [w - 16.0 * s, h - 34.0 * s], 18.0 * s, MUTED, TextAlign::Right, w * 0.6, clip);
    }
}
