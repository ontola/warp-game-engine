//! The lobby's screen overlay (main.gd `_draw_hud`): names over heads, open
//! Start menus, pickup badges over items, players without a controller,
//! toasts and the connection and update banners.

use crate::fonts::faces;
use crate::godot::{rgba, Color};
use crate::lobby::{short, AMBER, DIM, GREEN, TEXT, VIOLET};
use crate::live::Lobby;
use crate::paint::Canvas;
use glam::{Mat4, Vec3};
use render::hud::{FontFace, HudDrawList, TextAlign};

const BG: Color = crate::room::BG;
const PANEL: Color = rgba(0.03, 0.03, 0.06, 0.88);

fn regular() -> FontFace { faces().regular }
fn bold() -> FontFace { faces().bold }
fn width(text: &str, size: f32) -> f32 { render::hud::measure(text, if size >= 40.0 { bold() } else { regular() }, size)[0] }

/// main.gd `_text`: headings (40 px and up) use the heavy weight.
fn text(c: &mut Canvas, value: &str, pos: [f32; 2], size: f32, color: Color) {
    c.text(if size >= 40.0 { bold() } else { regular() }, pos, value, TextAlign::Left, -1.0, size, color);
}

/// Screen position of a world point, or None behind the camera.
fn project(view_proj: Mat4, size: [u32; 2], p: Vec3) -> Option<[f32; 2]> {
    let clip = view_proj * p.extend(1.0);
    if clip.w <= 1e-4 { return None; }
    let ndc = clip.truncate() / clip.w;
    Some([(ndc.x * 0.5 + 0.5) * size[0] as f32, (0.5 - ndc.y * 0.5) * size[1] as f32])
}

pub fn draw(lobby: &Lobby, size: [u32; 2], _alpha: f32) -> HudDrawList {
    let mut list = HudDrawList::new(size);
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut c = Canvas::new(&mut list, [0, 0, size[0], size[1]]);
    let scale = h / 1080.0;
    let camera = &lobby.stage.room.camera;
    let view = Mat4::look_at_rh(camera.pos, camera.target, Vec3::Y);
    let proj = Mat4::perspective_rh(camera.fov_y, w / h.max(1.0), camera.near.max(0.05), 4000.0);
    let view_proj = proj * view;

    let error = crate::host::get(&lobby.host.party, "onboarding_error", "");
    if !error.is_empty() && error != "<null>" {
        let bw = (w * 0.9).min(1080.0 * scale);
        let r = [(w - bw) / 2.0, 24.0 * scale, bw, 112.0 * scale];
        c.rect(r, rgba(0.03, 0.03, 0.06, 0.94));
        c.rect([r[0], r[1], 5.0 * scale, r[3]], AMBER);
        text(&mut c, "Could not prepare your game", [r[0] + 22.0 * scale, r[1] + 30.0 * scale], (23.0 * scale).floor(), AMBER);
        text(&mut c, &short(&error, 120), [r[0] + 22.0 * scale, r[1] + 62.0 * scale], (21.0 * scale).floor(), TEXT);
        c.over();
    }

    item_hints(lobby, &mut c, view_proj, size);

    // Only names and open menus follow players; action buttons stay at their objects.
    for p in &lobby.world.players {
        let depth = lobby.actor_depth(&p.id);
        let head = Vec3::new(p.x as f32, p.y as f32 + if p.sleeping { 1.3 } else { 2.25 }, depth);
        let Some(at) = project(view_proj, size, head) else { continue };
        let profile = lobby.profile(&p.id);
        let color = Color::from_string(profile.map_or("", |p| p.color.as_str()), VIOLET);
        let name = short(&lobby.name(&p.id), 14);
        let fs = (20.0 * scale).floor();
        let nw = width(&name, fs);
        c.text_outline(regular(), [at[0] - nw / 2.0, at[1]], &name, TextAlign::Left, -1.0, fs, (5.0 * scale).floor().max(4.0), BG.with_alpha(0.85));
        text(&mut c, &name, [at[0] - nw / 2.0, at[1]], fs, color.lightened(0.35));
        if let Some(left) = lobby.lost_left(&p.id) {
            let lost = format!("Controller lost · leaves in {left}s");
            let small = (16.0 * scale).floor();
            let lw = width(&lost, small);
            let y = at[1] - fs - 4.0 * scale;
            c.text_outline(regular(), [at[0] - lw / 2.0, y], &lost, TextAlign::Left, -1.0, small, (5.0 * scale).floor().max(4.0), BG.with_alpha(0.85));
            text(&mut c, &lost, [at[0] - lw / 2.0, y], small, AMBER);
        }
    }
    for p in &lobby.world.players {
        if !lobby.menus.contains_key(&p.id) { continue; }
        let head = Vec3::new(p.x as f32, p.y as f32 + if p.sleeping { 1.3 } else { 2.25 }, lobby.actor_depth(&p.id));
        let Some(at) = project(view_proj, size, head) else { continue };
        c.over();
        let fs = (20.0 * scale).floor();
        menu(lobby, &mut c, &p.id, [at[0], at[1] - fs - 10.0 * scale], (22.0 * scale).floor());
    }
    c.over();
    bench(lobby, &mut c, scale);

    // Toasts, bottom centre.
    let mut y = h - 40.0 * scale;
    for t in lobby.toasts.iter().rev() {
        let fs = (22.0 * scale).floor();
        let tw = width(&t.text, fs) + 44.0;
        let r = [w / 2.0 - tw / 2.0, y - fs - 20.0, tw, fs + 24.0];
        c.rect(r, PANEL);
        c.rect([r[0], r[1], 4.0, r[3]], t.color);
        text(&mut c, &t.text, [r[0] + 24.0, r[1] + fs + 4.0], fs, TEXT);
        y -= r[3] + 10.0;
    }
    if !lobby.host.update_ready().is_empty() {
        let fs = (18.0 * scale).floor();
        let msg = "Update ready  ·  Start to install";
        let bw = width(msg, fs) + 40.0;
        let r = [w - bw - 24.0 * scale, 24.0 * scale, bw, fs + 22.0];
        c.rect(r, rgba(0.03, 0.03, 0.06, 0.85));
        c.rect([r[0], r[1], 4.0, r[3]], GREEN);
        text(&mut c, msg, [r[0] + 20.0, r[1] + fs + 4.0], fs, TEXT);
    }
    if !lobby.host.demo && !lobby.host.connected {
        let fs = (24.0 * scale).floor();
        let msg = "Connecting to GameNight…";
        let bw = width(msg, fs) + 48.0;
        c.rect([w / 2.0 - bw / 2.0, 24.0, bw, fs + 28.0], rgba(0.03, 0.03, 0.06, 0.9));
        text(&mut c, msg, [w / 2.0 - bw / 2.0 + 24.0, 24.0 + fs + 6.0], fs, AMBER);
    }
    list
}

/// One B badge per reachable item, sized to the item on screen.
fn item_hints(lobby: &Lobby, c: &mut Canvas, view_proj: Mat4, size: [u32; 2]) {
    let mut targets: Vec<(u32, game_room_sim::world::Kind, Option<Vec3>, f32)> = lobby.pickup_hint_items().into_iter().map(|(id, kind)| (id, kind, None, 0.0)).collect();
    for (&id, &(kind, pos, at)) in &lobby.item_presses {
        let pressed = (1.0 - (lobby.time - at) / 0.2).max(0.0);
        match targets.iter_mut().find(|t| t.0 == id) { Some(t) => { t.2 = Some(pos); t.3 = pressed; } None => targets.push((id, kind, Some(pos), pressed)) }
    }
    for (id, kind, pressed_at, pressed) in targets {
        let top = match pressed_at.or_else(|| lobby.world.items.iter().find(|i| i.id == id).map(|i| Vec3::new(i.x as f32, i.y as f32, 0.05))) { Some(p) => p, None => continue };
        let top = top + Vec3::new(0.0, 0.95, 0.0);
        let (Some(at), Some(above)) = (project(view_proj, size, top), project(view_proj, size, top + Vec3::new(0.0, 0.4032, 0.0))) else { continue };
        let factor = ((at[0] - above[0]).powi(2) + (at[1] - above[1]).powi(2)).sqrt() / 96.0;
        let label = if kind == game_room_sim::world::Kind::Ball { "Kick" } else { "Pick up" };
        badge(c, at, factor, "B", label, Color::hex("e2574c"), pressed, size);
    }
    c.over();
}

/// room.gd `draw_prompt` for one button, scaled by `k` and centred on `at`.
#[allow(clippy::too_many_arguments)]
fn badge(c: &mut Canvas, at: [f32; 2], k: f32, key: &str, label: &str, color: Color, pressed: f32, size: [u32; 2]) {
    let h = 96.0 * k;
    let tw = render::hud::measure(label, bold(), 44.0 * k)[0];
    let full = [(40.0 + 96.0 + 36.0) * k + tw, h];
    let x0 = (at[0] - full[0] / 2.0).clamp(4.0, size[0] as f32 - full[0] - 4.0);
    let y0 = (at[1] - full[1] / 2.0).clamp(4.0, size[1] as f32 - full[1] - 4.0);
    let x = x0 + 20.0 * k;
    c.style_box([x, y0 + 6.0 * k, 96.0 * k + tw + 16.0 * k, h - 12.0 * k], [h / 2.0 - 6.0 * k; 4], rgba(0.03, 0.03, 0.06, 0.82), None);
    let center = [x + h / 2.0 - 4.0 * k, y0 + h / 2.0];
    c.circle(center, h / 2.0 - 13.0 * k, color.darkened(0.65));
    let r = h / 2.0 - 14.0 * k;
    let squash = 1.0 - pressed * 0.18;
    let face = [center[0], center[1] + pressed * 10.0 * k];
    let points: Vec<[f32; 2]> = (0..32).map(|i| { let a = i as f32 / 32.0 * std::f32::consts::TAU; [face[0] + a.cos() * r, face[1] + a.sin() * r * squash] }).collect();
    c.polygon(&points, color.darkened(pressed * 0.12));
    c.text(bold(), [x + 10.0 * k, y0 + h / 2.0 + (17.0 + pressed * 10.0) * k], key, TextAlign::Center, h - 28.0 * k, 48.0 * k, Color::hex("0b0b12"));
    c.text(bold(), [x + 90.0 * k, y0 + h / 2.0 + 16.0 * k], label, TextAlign::Left, -1.0, 44.0 * k, TEXT);
}

/// main.gd `_draw_menu`: a player's Start menu over their head.
fn menu(lobby: &Lobby, c: &mut Canvas, id: &str, at: [f32; 2], fs: f32) {
    let items = lobby.menu_items(id);
    let Some(state) = lobby.menus.get(id) else { return };
    let line = fs + 16.0;
    let block = fs * 0.5;
    let gap = fs * 0.22;
    let row = block + 12.0;
    let mut mw: f32 = 0.0;
    let mut extra = 0.0;
    for item in &items {
        mw = mw.max(width(&item.label, fs));
        if item.action == "couch" { mw = mw.max(lobby.profiles.len() as f32 * (block + gap) - gap); extra += row; }
    }
    mw += 80.0;
    let mh = line * items.len() as f32 + extra + 70.0;
    let x = (at[0] - mw / 2.0).clamp(16.0, (c.size[0] - mw - 16.0).max(16.0));
    let y0 = (at[1] - mh).max(16.0);
    c.rect([x, y0, mw, mh], rgba(0.04, 0.035, 0.08, 0.95));
    let edge = VIOLET.with_alpha(0.9);
    for r in [[x, y0, mw, 3.0], [x, y0 + mh - 3.0, mw, 3.0], [x, y0, 3.0, mh], [x + mw - 3.0, y0, 3.0, mh]] { c.rect(r, edge); }
    let heading = match state.page.as_str() { "lobbies" => "CHOOSE A LOBBY".to_string(), "remove" => "REMOVE A PLAYER".into(), _ => lobby.name(id).to_uppercase() };
    text(c, &heading, [x + 24.0, y0 + fs + 14.0], (fs * 0.7).floor(), AMBER);
    let mut y = y0 + 50.0;
    for (i, item) in items.iter().enumerate() {
        let selected = i == state.index;
        let couch = item.action == "couch";
        let mut label = item.label.clone();
        if item.action == "quit" && state.confirm && selected { label = "Press A again to quit".into(); }
        if item.action == "remove" && state.confirm && selected { label = format!("Press A again to remove {}", short(&item.name, 14)); }
        if selected { c.rect([x + 12.0, y, mw - 24.0, line - 4.0 + if couch { row } else { 0.0 }], VIOLET.with_alpha(0.35)); }
        text(c, &label, [x + 32.0, y + fs], fs, if selected { TEXT } else { DIM });
        if couch {
            // Yours is filled.
            let mine = lobby.couch_place(id);
            let colour = Color::from_string(lobby.profile(id).map_or("", |p| p.color.as_str()), AMBER);
            for place in 0..lobby.profiles.len() {
                let cell = [x + 32.0 + place as f32 * (block + gap), y + line - 2.0, block, block];
                if place as i64 == mine { c.rect(cell, colour); } else {
                    let e = DIM.with_alpha(if selected { 0.8 } else { 0.5 });
                    for r in [[cell[0], cell[1], block, 2.0], [cell[0], cell[1] + block - 2.0, block, 2.0], [cell[0], cell[1], 2.0, block], [cell[0] + block - 2.0, cell[1], 2.0, block]] { c.rect(r, e); }
                }
            }
            y += row;
        }
        y += line;
    }
}

/// main.gd `_draw_bench`: players without a controller, bottom left.
fn bench(lobby: &Lobby, c: &mut Canvas, scale: f32) {
    if lobby.bench.is_empty() { return; }
    let fs = (19.0 * scale).floor();
    let line = fs + 14.0 * scale;
    let small = (fs * 0.75).floor();
    let mut bw = width("ON PHONES", small);
    for id in &lobby.bench { bw = bw.max(width(&(short(&lobby.name(id), 16) + "  no controller"), fs) + 44.0 * scale); }
    bw += 36.0 * scale;
    let bh = line * lobby.bench.len() as f32 + 44.0 * scale;
    let r = [24.0 * scale, c.size[1] - bh - 24.0 * scale, bw, bh];
    c.rect(r, rgba(0.03, 0.03, 0.06, 0.82));
    c.rect([r[0], r[1], 4.0 * scale, r[3]], VIOLET);
    let all_phones = lobby.bench.iter().all(|id| lobby.phones.contains(id));
    text(c, if all_phones { "ON PHONES" } else { "NO CONTROLLER" }, [r[0] + 18.0 * scale, r[1] + 26.0 * scale], small, AMBER);
    for (i, id) in lobby.bench.iter().enumerate() {
        let y = r[1] + 40.0 * scale + i as f32 * line;
        let color = Color::from_string(lobby.profile(id).map_or("", |p| p.color.as_str()), VIOLET);
        let phone = lobby.phones.contains(id);
        let icon = [r[0] + 20.0 * scale, y + 2.0 * scale, 12.0 * scale, 20.0 * scale];
        if phone {
            c.rect(icon, color.lightened(0.2));
            let g = 2.5 * scale;
            c.rect([icon[0] + g, icon[1] + g, icon[2] - 2.0 * g, icon[3] - 2.0 * g], BG.with_alpha(0.9));
        } else {
            c.circle([icon[0] + icon[2] / 2.0, icon[1] + icon[3] / 2.0], 6.0 * scale, color.with_alpha(0.45));
        }
        let label = short(&lobby.name(id), 16);
        text(c, &label, [r[0] + 44.0 * scale, y + fs], fs, if phone { color.lightened(0.35) } else { DIM });
        if !phone {
            let x = r[0] + 44.0 * scale + width(&(label.clone() + "  "), fs);
            text(c, &format!("leaves in {}s", lobby.lost_left(id).unwrap_or(0)), [x, y + fs], (fs * 0.8).floor(), AMBER);
        }
    }
}
