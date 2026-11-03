//! The card pick (Godot `render_player.gd` card overlay and `card_face.gd`):
//! a dark backdrop, PICK A CARD in poster type, and each card drawn as a fight
//! poster: a category band, an art panel with a glow and the card's icon, the
//! name shrunk to fit, the description, and stat changes as coloured chips.
//! Cards flip open one after another, bob while idle, and the focused one
//! lifts and grows; after a pick it flies up and fades while the rest shrink.
use super::style::*;
use super::{Feedback, Layout};
use growing_guns::cards::{Card, CardDef};
use growing_guns::{Weapon, World};
use render::hud::HudDrawList;

const BASE_W: f32 = 220.0;
const BASE_H: f32 = 300.0;
const SEP: f32 = 10.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct Hit { pub player: usize, pub index: usize, pub card: Card, pub rect: [f32; 4], pub clip: [f32; 4] }

fn cubic_out(t: f32) -> f32 { 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3) }
fn cubic_in(t: f32) -> f32 { t.clamp(0.0, 1.0).powi(3) }
fn back_out(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); let (c1, c3) = (1.70158, 2.70158); 1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2) }

/// Godot `_card_stat_diff`: what the card changes on this player's gun.
pub fn stat_diff(world: &World, player: usize, card: Card) -> Vec<String> {
    let base = &world.players[player].weapon;
    let mut next: Weapon = base.clone();
    world.cards.apply(card, &mut next);
    if let (Some(h), Some(def)) = (world.hooks.0.clone(), world.cards.get(card)) { h.apply_card(def, &mut next); }
    let mut out = Vec::new();
    let pct = |a: f32, b: f32| ((b / a - 1.0) * 100.0).round() as i32;
    let diff = |a: f32, b: f32| (a - b).abs() > 0.001;
    if diff(base.damage_mult, next.damage_mult) { out.push(format!("{:+}% Damage", pct(base.damage_mult, next.damage_mult))); }
    if diff(base.fire_rate_mult, next.fire_rate_mult) { out.push(format!("{:+}% Fire Rate", pct(base.fire_rate_mult, next.fire_rate_mult))); }
    if base.mag_size_bonus != next.mag_size_bonus { out.push(format!("{:+} Ammo Capacity", next.mag_size_bonus - base.mag_size_bonus)); }
    if diff(base.reload_mult, next.reload_mult) { out.push(format!("{:+}% Reload Speed", pct(base.reload_mult, next.reload_mult))); }
    if diff(base.special_cooldown_mult, next.special_cooldown_mult) {
        let r = ((1.0 - next.special_cooldown_mult / base.special_cooldown_mult.max(0.001)) * 100.0).round() as i32;
        if r != 0 { out.push(format!("{r:+}% RMB Recharge")); }
    }
    if base.extra_projectiles != next.extra_projectiles { out.push(format!("{:+} Projectiles", next.extra_projectiles - base.extra_projectiles)); }
    if base.ricochet_count != next.ricochet_count { out.push(format!("{:+} Bounces", next.ricochet_count - base.ricochet_count)); }
    if diff(base.move_speed_mult, next.move_speed_mult) { out.push(format!("{:+}% Move Speed", pct(base.move_speed_mult, next.move_speed_mult))); }
    if (base.spread - next.spread).abs() > 0.00001 {
        let a = ((1.0 - next.spread / base.spread.max(0.00001)) * 100.0).round() as i32;
        if a != 0 { out.push(format!("{a:+}% Accuracy")); }
    }
    if (base.recoil_per_shot - next.recoil_per_shot).abs() > 0.00001 {
        let a = ((1.0 - next.recoil_per_shot / base.recoil_per_shot.max(0.00001)) * 100.0).round() as i32;
        if a != 0 { out.push(format!("{a:+}% Stability")); }
    }
    if diff(base.bullet_speed_mult, next.bullet_speed_mult) { out.push(format!("{:+}% Bullet Speed", pct(base.bullet_speed_mult, next.bullet_speed_mult))); }
    if next.explosive_radius > base.explosive_radius { out.push("Explosive Payload".into()); }
    if base.max_hp_bonus != next.max_hp_bonus { out.push(format!("{:+} Max HP", next.max_hp_bonus - base.max_hp_bonus)); }
    if base.extra_jumps != next.extra_jumps { out.push(format!("{:+} Extra Jumps", next.extra_jumps - base.extra_jumps)); }
    out
}

fn category(def: &CardDef) -> &'static str {
    if def.desc.starts_with("RMB") { "SPECIAL" } else if BODY_CARDS.contains(&def.id.as_str()) { "BODY" } else if def.firepower { "FIREPOWER" } else { "PERK" }
}

/// Greedy word wrap with measured widths.
pub fn wrap(text: &str, font: render::hud::FontFace, size: f32, width: f32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let trial = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && render::hud::measure(&trial, font, size)[0] > width { lines.push(std::mem::replace(&mut line, word.to_string())); } else { line = trial; }
        }
        lines.push(line);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(l) = lines.last_mut() { l.push('…'); }
    }
    lines
}

/// One card face into `r` (x, y, w, h): `alpha` fades it.
pub fn face(d: &mut HudDrawList, def: &CardDef, desc: &str, stats: &[String], r: [f32; 4], alpha: f32, bright: f32, clip: [f32; 4]) {
    let [x, y, w, h] = r;
    let k = w / 200.0;
    if k <= 0.0 { return; }
    let a = |c: C| { let c = [c[0] * bright, c[1] * bright, c[2] * bright, c[3]]; with_alpha(c, alpha) };
    let col = rgb(def.color);
    let edge = if def.rare { GOLD } else { col };
    d.rect(r, a(lerp(INK, col, 0.06)), clip);
    // Art panel with a soft glow behind the icon.
    let art_h = ((112.0 - (stats.len() as f32 - 2.0).max(0.0) * 15.0) * k).max(70.0 * k);
    let art = [x + 10.0 * k, y + 32.0 * k, w - 18.0 * k, art_h];
    d.rect(art, a(darkened(INK, 0.35)), clip);
    let c = [art[0] + art[2] * 0.5, art[1] + art[3] * 0.5];
    for i in 0..6 { d.circle(c, art[3] * (0.5 + (0.38 - 0.5) * i as f32 / 5.0), a(with_alpha(edge, 0.045)), clip); }
    d.circle(c, art[3] * 0.36, a(with_alpha(edge, 0.18)), clip);
    let is = (art[3] * 0.72).min(80.0 * k);
    let icon = card_icon(&def.id);
    draw_icon(d, icon, [c[0] - is * 0.5 + 3.0 * k, c[1] - is * 0.5 + 3.0 * k, is, is], a([0.0, 0.0, 0.0, 0.55]), clip);
    draw_icon(d, icon, [c[0] - is * 0.5, c[1] - is * 0.5, is, is], a(lerp(edge, BONE, 0.25)), clip);
    frame_rect(d, art, k.max(1.0), a(with_alpha(BONE, 0.18)), clip);
    // Category band, slanted like the HUD plates.
    let (bh, bw) = (20.0 * k, 96.0 * k);
    d.polygon(&[[x, y + 6.0 * k], [x + bw + bh * SKEW, y + 6.0 * k], [x + bw, y + 6.0 * k + bh], [x, y + 6.0 * k + bh]], a(edge), clip);
    Label::new(bold(), 13.0 * k, a(INK)).draw(d, category(def), x + 12.0 * k, y + 6.0 * k + bh * 0.5, clip);
    if def.rare { Label::new(display(), 14.0 * k, a(GOLD)).right().draw(d, "RARE", x + w - 10.0 * k, y + 6.0 * k + bh * 0.5, clip); }
    // Name: poster type, shrunk until it fits on one line.
    let name = def.name.to_uppercase();
    let max_w = w - 22.0 * k;
    let size = fit(&name, display(), 30.0 * k, max_w);
    let name_mid = art[1] + art[3] + 10.0 * k + size * 0.62;
    Label::new(display(), size, a(BONE)).shadow((2.0 * k).max(1.0), a(RED)).center().draw(d, &name, x + 11.0 * k + max_w * 0.5, name_mid, clip);
    // Description, up to three lines (two when there are three stat chips).
    let mut ty = name_mid + size * 0.62 + 4.0 * k;
    if !desc.is_empty() {
        let ds = 14.0 * k;
        for line in wrap(desc, BODY, ds, w - 28.0 * k, if stats.len() < 3 { 3 } else { 2 }) {
            Label::new(BODY, ds, a(BONE_DIM)).center().draw(d, &line, x + w * 0.5, ty + ds * 0.6, clip);
            ty += ds * 1.15;
        }
    }
    // Stat chips, stacked up from the bottom edge.
    let row = 15.0 * k;
    let mut by = y + h - 10.0 * k - row * stats.len() as f32;
    for line in stats {
        let (value, label) = match line.split_once(' ') { Some((v, l)) if v.starts_with('+') || v.starts_with('-') => (v, l), _ => ("", line.as_str()) };
        let tint = if value.starts_with('-') { BAD } else { GOOD };
        let cr = [x + 12.0 * k, by, w - 22.0 * k, row - 2.0 * k];
        d.rect(cr, a(with_alpha(tint, 0.12)), clip);
        d.rect([cr[0], cr[1], 3.0 * k, cr[3]], a(tint), clip);
        let mid = cr[1] + cr[3] * 0.5;
        if !value.is_empty() { Label::new(display(), 13.0 * k, a(tint)).right().draw(d, value, cr[0] + 44.0 * k, mid, clip); }
        let ls = fit(&label.to_uppercase(), bold(), 12.0 * k, cr[2] - 52.0 * k);
        Label::new(bold(), ls, a(BONE_DIM)).draw(d, &label.to_uppercase(), cr[0] + 50.0 * k, mid, clip);
        by += row;
    }
    // Thick colour edge on the left and a fine inner frame.
    d.rect([x, y, 6.0 * k, h], a(edge), clip);
    frame_rect(d, [x + 3.0 * k, y + 3.0 * k, w - 6.0 * k, h - 6.0 * k], k.max(1.0), a(with_alpha(BONE, 0.12)), clip);
    if def.rare { frame_rect(d, r, (2.5 * k).max(2.0), a(GOLD), clip); }
}

/// A soft drop shadow: rings of falling alpha around `r`.
fn shadow(d: &mut HudDrawList, r: [f32; 4], size: f32, c: C, clip: [f32; 4]) {
    let steps = 6;
    for s in (1..=steps).rev() {
        let g = size * s as f32 / steps as f32;
        d.rect([r[0] - g, r[1] - g, r[2] + 2.0 * g, r[3] + 2.0 * g], with_alpha(c, 1.0 / steps as f32 * (1.0 - s as f32 / (steps as f32 + 1.0)) * 1.6), clip);
    }
}

/// The pick overlay for player `i` in its view.
pub(super) fn pick(d: &mut HudDrawList, world: &World, i: usize, l: &Layout, f: &Feedback, focus: usize, hits: &mut Vec<Hit>) {
    if f.lava_death && f.burned.is_some_and(|t|t<2.4) { return; }
    let p = &world.players[i];
    let (cards, age, exit): (Vec<Card>, f32, Option<(usize, f32)>) = match (&p.offer, &f.exit) {
        (Some(o), _) => (o.clone(), p.offer_age as f32, None),
        (None, Some(e)) => (e.cards.clone(), 9.0, Some((e.picked, e.age))),
        _ => return,
    };
    let clip = l.rect;
    let u = l.u;
    let n = cards.len().max(1) as f32;
    let (vw, vh) = (l.rect[2] / u, l.rect[3] / u);
    let total = BASE_W * n + SEP * (n - 1.0);
    let scale = ((vw * 0.92) / total).min((vh - 100.0).max(0.0) / BASE_H).clamp(0.7, 1.5);
    let (slot_w, slot_h, sep) = (BASE_W * scale * u, BASE_H * scale * u, (SEP * scale).round().max(6.0) * u);
    let row_w = slot_w * n + sep * (n - 1.0);
    // The same headline as the banner it follows ("P1 DIED", size::L),
    // smaller only when the cards have to shrink.
    let title_size = (size::L * scale.min(1.0)).round().max(24.0) * u;
    let block_h = title_size * 1.2 + 12.0 * u + slot_h;
    let top = l.rect[1] + (l.rect[3] - block_h) * 0.5;
    let x0 = l.rect[0] + (l.rect[2] - row_w) * 0.5;
    // Title: fades 0.12 s after a pick (and lifts as the overlay closes).
    // It fades in once the banner it replaces has faded out (0.15 s).
    let title_a = match exit { Some((_, t)) => (1.0 - t / 0.12).max(0.0), None => ((age - 0.15) / 0.2).clamp(0.0, 1.0) };
    let title = if world.coop { "WAVE COMPLETE - PICK A CARD" } else { "PICK A CARD" };
    if title_a > 0.0 {
        let lab = Label::poster(title_size).center();
        Label { color: with_alpha(lab.color, title_a), ..lab }.draw(d, title, l.rect[0] + l.rect[2] * 0.5, top + title_size * 0.6, clip);
    }
    let row_y = top + title_size * 1.2 + 12.0 * u;
    let locked = age < 0.75;
    let pad = l.controller;
    for (k, &card) in cards.iter().enumerate() {
        let Some(def) = world.cards.get(card) else { continue };
        let kf = k as f32;
        // Reveal: hidden until the flip opens at 0.5 + 0.19 k s (0.15 s, back out).
        let open = 0.5 + 0.19 * kf;
        let flip = if age < open { 0.0 } else { back_out((age - open) / 0.15) };
        if flip <= 0.001 { continue; }
        // Idle bob (±4 over 2.2 s) unless focused; focus lifts 10 and grows 12%.
        let focused = !locked && exit.is_none() && k == focus.min(cards.len() - 1);
        let hl = f.highlight.get(k).copied().unwrap_or(if focused { 1.0 } else { 0.0 });
        let bob = (age / 2.2 * std::f32::consts::PI + kf).sin() * 4.0 * (1.0 - hl);
        let mut sc = 1.0 + 0.12 * hl;
        let mut lift = -10.0 * hl + bob;
        let mut alpha = 1.0;
        if let Some((picked, t)) = exit {
            if k == picked {
                sc = 1.0 + 0.5 * back_out(t / 0.42);
                lift = -100.0 * back_out(t / 0.42);
                alpha = 1.0 - cubic_in(t / 0.34);
            } else {
                sc = 1.0 - 0.5 * cubic_out(t / 0.35);
                alpha = 1.0 - (t / 0.28).min(1.0);
            }
        }
        if alpha <= 0.003 { continue; }
        let (bw, bh) = (200.0 * scale * u * sc, 280.0 * scale * u * sc);
        let cx = x0 + kf * (slot_w + sep) + slot_w * 0.5;
        let cy = row_y + slot_h * 0.5 + lift * scale * u;
        let w = bw * flip;
        let r = [cx - w * 0.5, cy - bh * 0.5, w, bh];
        if !locked && exit.is_none() { hits.push(Hit { player: i, index: k, card, rect: r, clip }); }
        let (sz, sc_col) = if def.rare { (if focused { 36.0 } else { 25.0 }, with_alpha(GOLD, if focused { 0.72 } else { 0.4 })) } else { (if focused { 36.0 } else { 8.0 }, [0.0, 0.0, 0.0, if focused { 0.72 } else { 0.3 }]) };
        shadow(d, r, sz * scale * u * 0.5, with_alpha(sc_col, alpha), clip);
        let desc = super::prompt_text(&def.desc, pad);
        let stats = f.stats.get(k).cloned().unwrap_or_default();
        face(d, def, &desc, &stats, r, alpha, if focused { 1.12 } else { 1.0 }, clip);
        if def.rare {
            // The holo sheen: a rainbow band sweeping across.
            let t = (age * 0.5).fract();
            let hues = [[1.0, 0.3, 0.3], [1.0, 0.9, 0.3], [0.3, 1.0, 0.3], [0.3, 0.6, 1.0], [1.0, 0.3, 1.0]];
            let band = hues[((t * 5.0) as usize) % 5];
            let bx = r[0] + r[2] * t;
            d.polygon(&[[bx, r[1]], [(bx + r[2] * 0.25).min(r[0] + r[2]), r[1]], [(bx + r[2] * 0.05).min(r[0] + r[2]), r[1] + r[3]], [(bx - r[2] * 0.2).max(r[0]), r[1] + r[3]]], [band[0], band[1], band[2], 0.08 * alpha], clip);
        }
    }
}

/// Ease card highlights toward the focus, as Godot's 0.18/0.22 s tweens.
pub fn ease_highlight(f: &mut Feedback, n: usize, focus: Option<usize>, dt: f32) {
    f.highlight.resize(n, 0.0);
    for (k, h) in f.highlight.iter_mut().enumerate() {
        let target = if Some(k) == focus { 1.0 } else { 0.0 };
        let rate = if target > *h { dt / 0.18 } else { dt / 0.22 };
        // Clamp to the target so a fully highlighted card stays there instead
        // of taking a shrink step every other tick while focus is unchanged.
        *h = if target > *h { (*h + rate).min(target) } else { (*h - rate).max(target) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_highlight_stays_settled_until_focus_changes() {
        let mut feedback = Feedback::default();
        for (focus, expected) in [
            (Some(1), [0.0, 1.0, 0.0]),
            (Some(2), [0.0, 0.0, 1.0]),
            (None, [0.0; 3]),
        ] {
            // Hold each focus for two seconds, well past the rise/fall tween.
            // Checking every settled tick catches alternating size/lift jitter.
            for tick in 0..240 {
                ease_highlight(&mut feedback, 3, focus, 1.0 / 120.0);
                if tick >= 30 {
                    assert_eq!(feedback.highlight.as_slice(), &expected, "focus {focus:?}, tick {tick}");
                }
            }
        }
    }
}
