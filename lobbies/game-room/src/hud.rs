//! The lobby's screen overlay (main.gd `_draw_hud`): names over heads, open
//! Start menus, pickup badges over items, players without a controller,
//! toasts and the connection and update banners.

use crate::fonts::faces;
use crate::godot::Color;
use crate::lobby::{short, AMBER, DIM, GREEN, TEXT, VIOLET};
use crate::live::Lobby;
use crate::paint::Canvas;
use glam::{Mat4, Vec3};
use render::hud::{FontFace, HudDrawList, TextAlign};

const BG: Color = crate::room::BG;

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

/// The room camera for a screen of `size`: on screens narrower than 16:9
/// the field of view grows, so the room keeps its 16:9 width and the door
/// and the QR code at the sides stay in view. It grows mostly upwards: the
/// camera tilts up, so the room sits low with more ceiling above it and
/// only a little more floor below.
pub fn fitted(camera: &render::Camera, size: [u32; 2]) -> render::Camera {
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    let wide = 16.0 / 9.0;
    let mut camera = *camera;
    if aspect < wide {
        let fov = 2.0 * ((camera.fov_y / 2.0).tan() * wide / aspect).atan();
        // Most of the new room goes above: a little floor stays in front.
        let tilt = (fov - camera.fov_y) / 2.0 * 0.6;
        let forward = camera.target - camera.pos;
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        camera.target = camera.pos + glam::Quat::from_axis_angle(right, tilt) * forward;
        camera.fov_y = fov;
    }
    camera
}

/// The room camera's view-projection for a screen of `size`.
pub fn view_proj(camera: &render::Camera, size: [u32; 2]) -> Mat4 {
    let camera = &fitted(camera, size);
    let (w, h) = (size[0] as f32, size[1] as f32);
    let view = Mat4::look_at_rh(camera.pos, camera.target, Vec3::Y);
    let proj = Mat4::perspective_rh(camera.fov_y, w / h.max(1.0), camera.near.max(0.05), 4000.0);
    proj * view
}

/// Screen position of a world point, for the mouse.
pub fn to_screen(view_proj: Mat4, size: [u32; 2], p: Vec3) -> Option<[f32; 2]> { project(view_proj, size, p) }

static UI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// How much bigger than on a TV the overlay draws: on a phone the screen
/// is small in the hand, so the host asks for more (see [`ui_for_dp`]).
pub fn set_ui_scale(k: f32) { UI_SCALE.store(k.clamp(0.5, 3.0).to_bits(), std::sync::atomic::Ordering::Relaxed); }

/// The overlay's scale on a screen `h` pixels high: 1 at 1080p on a TV.
pub fn ui(h: f32) -> f32 { h / 1080.0 * f32::from_bits(UI_SCALE.load(std::sync::atomic::Ordering::Relaxed)) }

/// The UI scale for a screen whose short side is `dp` device-independent
/// pixels (CSS px): a phone (about 400) gets 1.7x, a tablet or laptop
/// (700 and up) 1x.
pub fn ui_for_dp(dp: f32) -> f32 { (700.0 / dp.max(1.0)).clamp(1.0, 1.7) }

/// Where the mouse's Start menu stands: the middle of the screen.
fn mouse_menu_at(size: [u32; 2]) -> ([f32; 2], f32) {
    let scale = ui(size[1] as f32);
    ([size[0] as f32 / 2.0, size[1] as f32 * 0.62], (20.0 * scale).floor())
}

/// The mouse menu's item rows on screen, for pointing at them.
pub fn mouse_menu_rows(lobby: &Lobby, size: [u32; 2]) -> Option<([f32; 4], Vec<[f32; 4]>)> {
    let (at, fs) = mouse_menu_at(size);
    menu_layout(lobby, crate::mouse::MOUSE, at, fs, size)
}

pub fn draw(lobby: &Lobby, size: [u32; 2], _alpha: f32) -> HudDrawList {
    let mut list = HudDrawList::new(size);
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut c = Canvas::new(&mut list, [0, 0, size[0], size[1]]);
    let scale = ui(h);
    let view_proj = view_proj(&lobby.stage.room.camera, size);

    let error = crate::host::get(&lobby.host.party, "onboarding_error", "");
    if !error.is_empty() && error != "<null>" {
        let bw = (w * 0.9).min(1080.0 * scale);
        let r = [(w - bw) / 2.0, 24.0 * scale, bw, 112.0 * scale];
        crate::paint::panel_with(&mut c, r, scale, crate::room::PANEL_SOLID, crate::room::RED.with_alpha(0.6));
        text(&mut c, "COULD NOT PREPARE YOUR GAME", [r[0] + 24.0 * scale, r[1] + 34.0 * scale], (16.0 * scale).floor(), AMBER);
        text(&mut c, &short(&error, 120), [r[0] + 24.0 * scale, r[1] + 66.0 * scale], (21.0 * scale).floor(), TEXT);
        c.over();
    }

    crate::catalog::shelf_sides(lobby, &mut c, view_proj, size);
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
    // The catalog view covers the room; Start menus and the mouse's go over it.
    if lobby.catalog_fade() > 0.0 { crate::catalog::draw(lobby, &mut c, size); }
    let mut corner = 0;
    for p in &lobby.world.players {
        if !lobby.menus.contains_key(&p.id) { continue; }
        let fs = (20.0 * scale).floor();
        let at = if lobby.catalog_open() {
            // Over the catalog the players aren't seen: menus go to the
            // corners, clear of the covers' middle (bottom left first).
            let mh = menu_layout(lobby, &p.id, [0.0, h], fs, size).map_or(0.0, |(r, _)| r[3]);
            let (right, top) = [(false, false), (false, true), (true, true), (true, false)][corner % 4];
            corner += 1;
            let y = if top { 110.0 * scale + mh } else if right { h * 0.66 } else { h - 24.0 * scale };
            [if right { w } else { 0.0 }, y]
        } else {
            let head = Vec3::new(p.x as f32, p.y as f32 + if p.sleeping { 1.3 } else { 2.25 }, lobby.actor_depth(&p.id));
            let Some(at) = project(view_proj, size, head) else { continue };
            [at[0], at[1] - (20.0 * scale).floor() - 10.0 * scale]
        };
        c.over();
        menu(lobby, &mut c, &p.id, at, fs);
    }
    if lobby.menus.contains_key(crate::mouse::MOUSE) {
        c.over();
        let (at, fs) = mouse_menu_at(size);
        menu(lobby, &mut c, crate::mouse::MOUSE, at, fs);
    }
    c.over();
    back_panel(lobby, &mut c, view_proj, size, scale);
    c.over();
    bench(lobby, &mut c, scale);

    // Toasts, bottom centre; in the catalog view top centre, clear of its
    // buttons and hints.
    let catalog = lobby.catalog_open();
    let mut y = if catalog { 40.0 * scale } else { h - 40.0 * scale };
    for t in lobby.toasts.iter().rev() {
        let fs = (22.0 * scale).floor();
        let tw = width(&t.text, fs) + 62.0 * scale;
        let r = if catalog { [w / 2.0 - tw / 2.0, y, tw, fs + 24.0] } else { [w / 2.0 - tw / 2.0, y - fs - 20.0, tw, fs + 24.0] };
        crate::paint::panel(&mut c, r, scale);
        // Its colour as a small dot, not a bar (DESIGN.md "Toast").
        c.circle([r[0] + 24.0 * scale, r[1] + r[3] / 2.0], 4.0 * scale, t.color);
        text(&mut c, &t.text, [r[0] + 38.0 * scale, r[1] + r[3] / 2.0 + fs * 0.35], fs, TEXT);
        y += if catalog { r[3] + 10.0 } else { -(r[3] + 10.0) };
    }
    if !lobby.host.update_ready().is_empty() {
        let fs = (18.0 * scale).floor();
        let msg = "Update ready  ·  Start to install";
        let bw = width(msg, fs) + 58.0 * scale;
        let r = [w - bw - 32.0 * scale, 32.0 * scale, bw, fs + 22.0];
        crate::paint::panel(&mut c, r, scale);
        c.circle([r[0] + 22.0 * scale, r[1] + r[3] / 2.0], 4.0 * scale, GREEN);
        text(&mut c, msg, [r[0] + 36.0 * scale, r[1] + r[3] / 2.0 + fs * 0.35], fs, TEXT);
    }
    if !lobby.host.demo && !lobby.host.connected {
        let fs = (24.0 * scale).floor();
        let msg = "Connecting to Splitscreen Club…";
        let bw = width(msg, fs) + 48.0;
        crate::paint::panel(&mut c, [w / 2.0 - bw / 2.0, 24.0, bw, fs + 28.0], scale);
        text(&mut c, msg, [w / 2.0 - bw / 2.0 + 24.0, 24.0 + fs + 6.0], fs, AMBER);
    }
    list
}

/// One B badge per reachable item, sized to the item on screen.
fn item_hints(lobby: &Lobby, c: &mut Canvas, view_proj: Mat4, size: [u32; 2]) {
    let mut targets: Vec<(u32, game_room_sim::world::Kind, Option<Vec3>, f32, Option<u8>)> = lobby.pickup_hint_items().into_iter().map(|(id, kind, half)| (id, kind, None, 0.0, half)).collect();
    // Just picked up: the badge presses, with the keys it showed (or all
    // keys, when everyone plays on the keyboard).
    let keys = (!lobby.world.players.is_empty() && lobby.world.players.iter().all(|p| lobby.keyboard_half(&p.id) == Some(0))).then_some(0);
    for (&id, &(kind, pos, at)) in &lobby.item_presses {
        let pressed = (1.0 - (lobby.time - at) / 0.2).max(0.0);
        match targets.iter_mut().find(|t| t.0 == id) { Some(t) => { t.2 = Some(pos); t.3 = pressed; } None => targets.push((id, kind, Some(pos), pressed, keys)) }
    }
    for (id, kind, pressed_at, pressed, half) in targets {
        let top = match pressed_at.or_else(|| lobby.world.items.iter().find(|i| i.id == id).map(|i| Vec3::new(i.x as f32, i.y as f32, 0.05))) { Some(p) => p, None => continue };
        let top = top + Vec3::new(0.0, 0.95, 0.0);
        let (Some(at), Some(above)) = (project(view_proj, size, top), project(view_proj, size, top + Vec3::new(0.0, 0.4032, 0.0))) else { continue };
        let factor = ((at[0] - above[0]).powi(2) + (at[1] - above[1]).powi(2)).sqrt() / 96.0;
        let label = if kind == game_room_sim::world::Kind::Ball { "Kick" } else { "Pick up" };
        badge(c, at, factor, "B", half.and_then(|h| crate::paint::key_cap_on(h, "B")), label, crate::room::PAD_B, pressed, size);
    }
    // Someone carrying a player: B throws them, out of the party when the
    // exit door is close in the way they face.
    for p in &lobby.world.players {
        if p.carrying.is_none() { continue; }
        let door = game_room_sim::layout::EXIT_DOOR;
        let towards = (door - p.x) * p.facing > 0.0 && (p.x - door).abs() < 6.0;
        let label = if towards { "Throw out of party" } else { "Throw" };
        let top = Vec3::new(p.x as f32, p.y as f32 + 3.3, lobby.actor_depth(&p.id));
        let (Some(at), Some(above)) = (project(view_proj, size, top), project(view_proj, size, top + Vec3::new(0.0, 0.4032, 0.0))) else { continue };
        let factor = ((at[0] - above[0]).powi(2) + (at[1] - above[1]).powi(2)).sqrt() / 96.0;
        let cap = lobby.keyboard_half(&p.id).and_then(|h| crate::paint::key_cap_on(h, "B"));
        badge(c, at, factor, "B", cap, label, crate::room::PAD_B, 0.0, size);
    }
    c.over();
}

/// room.gd `draw_prompt` for one button, scaled by `k` and centred on `at`.
/// `cap`: a keyboard player's key, drawn as its keycap instead of the disc.
#[allow(clippy::too_many_arguments)]
fn badge(c: &mut Canvas, at: [f32; 2], k: f32, key: &str, cap: Option<&str>, label: &str, color: Color, pressed: f32, size: [u32; 2]) {
    let h = 96.0 * k;
    let tw = render::hud::measure(label, bold(), 44.0 * k)[0];
    let glyph = cap.map_or(96.0, |cap| crate::paint::key_cap_width(cap) + 24.0) * k;
    let full = [40.0 * k + glyph + 36.0 * k + tw, h];
    let x0 = (at[0] - full[0] / 2.0).clamp(4.0, size[0] as f32 - full[0] - 4.0);
    let y0 = (at[1] - full[1] / 2.0).clamp(4.0, size[1] as f32 - full[1] - 4.0);
    let x = x0 + 20.0 * k;
    c.style_box([x, y0 + 6.0 * k, glyph + tw + 16.0 * k, h - 12.0 * k], [h / 2.0 - 6.0 * k; 4], crate::room::PANEL, None);
    if let Some(cap) = cap {
        crate::paint::keycap(c, [x + 12.0 * k, y0 + 20.0 * k, glyph - 24.0 * k, 56.0 * k], cap, pressed, true);
        c.text(bold(), [x + glyph - 6.0 * k, y0 + h / 2.0 + 16.0 * k], label, TextAlign::Left, -1.0, 44.0 * k, TEXT);
        return;
    }
    let center = [x + h / 2.0 - 4.0 * k, y0 + h / 2.0];
    c.circle(center, h / 2.0 - 13.0 * k, color.darkened(0.65));
    let r = h / 2.0 - 14.0 * k;
    let squash = 1.0 - pressed * 0.18;
    let face = [center[0], center[1] + pressed * 10.0 * k];
    let points: Vec<[f32; 2]> = (0..32).map(|i| { let a = i as f32 / 32.0 * std::f32::consts::TAU; [face[0] + a.cos() * r, face[1] + a.sin() * r * squash] }).collect();
    c.polygon(&points, color.darkened(pressed * 0.12));
    c.text(bold(), [x + 10.0 * k, y0 + h / 2.0 + (17.0 + pressed * 10.0) * k], key, TextAlign::Center, h - 28.0 * k, 48.0 * k, BG);
    c.text(bold(), [x + 90.0 * k, y0 + h / 2.0 + 16.0 * k], label, TextAlign::Left, -1.0, 44.0 * k, TEXT);
}

/// A Start menu's box and its item rows (main.gd `_draw_menu` layout),
/// standing on `at` (bottom centre).
const MOUSE_HEADING: &str = "KEYBOARD & MOUSE";

fn menu_layout(lobby: &Lobby, id: &str, at: [f32; 2], fs: f32, size: [u32; 2]) -> Option<([f32; 4], Vec<[f32; 4]>)> {
    lobby.menus.get(id)?;
    let items = lobby.menu_items(id);
    let (line, inset) = menu_spacing(fs);
    let block = fs * 0.5;
    let gap = fs * 0.22;
    let row = block + 12.0;
    let mut mw: f32 = 0.0;
    let mut extra = 0.0;
    for item in &items {
        mw = mw.max(width(&item.label, fs));
        if item.action == "couch" { mw = mw.max(lobby.profiles.len() as f32 * (block + gap) - gap); extra += row; }
    }
    // The label's inset on both sides, plus room to breathe.
    mw += 2.0 * (inset + menu_text_x(fs)) + fs;
    // The keyboard and mouse menu says what it is and how to leave it.
    if id == crate::mouse::MOUSE { mw = mw.max(width(MOUSE_HEADING, (fs * 0.7).floor()) + 2.0 * (menu_spacing(fs).1 + menu_text_x(fs))); }
    // 12 under the last row, as at its sides (rows are `line - 4` tall).
    let mh = line * items.len() as f32 + extra + menu_head(fs) + inset;
    let x = (at[0] - mw / 2.0).clamp(16.0, (size[0] as f32 - mw - 16.0).max(16.0));
    let y0 = (at[1] - mh).max(16.0);
    let mut rows = vec![];
    let mut y = y0 + menu_head(fs);
    for item in &items {
        let couch = if item.action == "couch" { row } else { 0.0 };
        rows.push([x + inset, y, mw - 2.0 * inset, line - 4.0 + couch]);
        y += line + couch;
    }
    Some(([x, y0, mw, mh], rows))
}

/// The menu's heading band: the first row starts below it. A fixed 50 px
/// let large menus (a 4K HUD, a browser canvas) draw the heading over the
/// first row.
fn menu_head(fs: f32) -> f32 { menu_spacing(fs).1 * 1.6 + (fs * 0.7).floor() + menu_spacing(fs).1 }

/// A menu row's height and the panel's inset around the rows, from the
/// label size: rows with air above and below the words (Joep: the old
/// ones felt cramped).
fn menu_spacing(fs: f32) -> (f32, f32) { ((fs * 2.0).round(), (fs * 0.7).round()) }

/// Where a row's label starts, from the row's left edge.
fn menu_text_x(fs: f32) -> f32 { (fs * 0.8).round() }

/// main.gd `_draw_menu`: a player's Start menu over their head.
/// The rows of the host's Back panel (`back_menu` in the party): the
/// player it is for, then per chord its button, label and whether it is
/// the leave waiting for a second B.
pub fn back_rows(lobby: &Lobby) -> Option<(Option<String>, Vec<(String, String, bool)>)> {
    let menu = &lobby.host.party["back_menu"];
    let chords = menu["chords"].as_array().filter(|c| !c.is_empty())?;
    let player = menu["player"].as_str().filter(|p| !p.is_empty()).map(String::from)
        .or_else(|| menu["controller"].as_str().and_then(|c| lobby.controllers.get(c).cloned()));
    let confirm = menu["confirm_leave"].as_bool().unwrap_or(false);
    let rows = chords.iter().map(|chord| {
        let button = chord["button"].as_str().unwrap_or("").to_uppercase();
        let action = chord["action"].as_str().unwrap_or("");
        let fallback = match action { "next" => "Play the next game", "previous" => "Play the previous game", "leave" => "Leave the party", "report_bug" => "Report a bug", other => other };
        let label = chord["label"].as_str().filter(|l| !l.is_empty()).unwrap_or(fallback).to_string();
        let waiting = confirm && action == "leave";
        (button, if waiting { "Press B again to leave".into() } else { label }, waiting)
    }).collect();
    Some((player, rows))
}

/// While a player holds Back: what Back plus another button does, over
/// their head (or bottom centre for someone not in the room).
fn back_panel(lobby: &Lobby, c: &mut Canvas, view_proj: Mat4, size: [u32; 2], scale: f32) {
    let Some((player, rows)) = back_rows(lobby) else { return };
    let (w, h) = (size[0] as f32, size[1] as f32);
    let fs = (24.0 * scale).floor();
    let line = fs * 2.0;
    let key_w = fs * 2.6;
    let head = fs * 1.9;
    let pad = fs * 0.8;
    let text_w = rows.iter().map(|r| width(&r.1, fs)).fold(width("Hold Back and press", (fs * 0.8).floor()), f32::max);
    let (pw, ph) = (pad * 2.0 + key_w + fs * 0.7 + text_w, pad * 2.0 + head + line * rows.len() as f32);
    let anchor = player.as_ref().and_then(|id| {
        let p = lobby.world.player(id)?;
        project(view_proj, size, Vec3::new(p.x as f32, p.y as f32 + if p.sleeping { 1.3 } else { 2.25 }, lobby.actor_depth(id)))
    });
    let (x, y) = match anchor {
        Some(at) => (at[0] - pw / 2.0, at[1] - ph - 34.0 * scale),
        None => (w / 2.0 - pw / 2.0, h - ph - 120.0 * scale),
    };
    let (x, y) = (x.clamp(8.0, w - pw - 8.0), y.clamp(8.0, h - ph - 8.0));
    crate::paint::panel_solid(c, [x, y, pw, ph], scale);
    let name = player.as_deref().map(|id| lobby.name(id)).unwrap_or_default();
    let title = if name.is_empty() { "BACK".to_string() } else { format!("BACK  ·  {}", short(&name, 14).to_uppercase()) };
    c.text(bold(), [x + pad, y + pad + fs * 0.9], &title, TextAlign::Left, -1.0, (fs * 0.7).floor(), AMBER);
    let half = player.as_deref().and_then(|id| lobby.keyboard_half(id));
    let back = half.and_then(|h| crate::paint::key_cap_on(h, "BACK")).unwrap_or("Back");
    let hold = format!("Hold {back} and press");
    c.text(regular(), [x + pad, y + pad + fs * 1.75], &hold, TextAlign::Left, -1.0, (fs * 0.8).floor(), DIM);
    for (i, (button, label, waiting)) in rows.iter().enumerate() {
        let top = y + pad + head + line * i as f32 + fs * 0.25;
        let r = [x + pad, top, key_w, line - fs * 0.5];
        if let Some(cap) = half.and_then(|h| crate::paint::key_cap_on(h, button)) {
            crate::paint::keycap(c, r, cap, 0.0, true);
        } else if button == "B" || button == "Y" {
            let center = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
            c.circle(center, r[3] / 2.0, crate::paint::pad_color(button).unwrap_or(TEXT));
            c.text(bold(), [r[0], center[1] + fs * 0.36], button, TextAlign::Center, r[2], fs, BG);
        } else {
            crate::paint::bumper(c, r, button, 0, 0.0);
        }
        let color = if *waiting { AMBER } else { TEXT };
        c.text(regular(), [r[0] + key_w + fs * 0.7, r[1] + r[3] / 2.0 + fs * 0.36], label, TextAlign::Left, -1.0, fs, color);
    }
}

fn menu(lobby: &Lobby, c: &mut Canvas, id: &str, at: [f32; 2], fs: f32) {
    let items = lobby.menu_items(id);
    let Some(state) = lobby.menus.get(id) else { return };
    let Some(([x, y0, mw, mh], _)) = menu_layout(lobby, id, at, fs, [c.size[0] as u32, c.size[1] as u32]) else { return };
    let (line, inset) = menu_spacing(fs);
    let block = fs * 0.5;
    let gap = fs * 0.22;
    let row = block + 12.0;
    let mouse = id == crate::mouse::MOUSE;
    // One panel style (DESIGN.md "Menu"): solid, so the room doesn't read through.
    let k = fs / 22.0;
    crate::paint::panel_solid(c, [x, y0, mw, mh], k);
    let heading = match state.page.as_str() { "lobbies" => "CHOOSE A LOBBY".to_string(), "remove" => "REMOVE A PLAYER".into(), page if page.starts_with("player:") => lobby.name(&page["player:".len()..]).to_uppercase(), _ if mouse => MOUSE_HEADING.into(), _ => lobby.name(id).to_uppercase() };
    text(c, &heading, [x + inset + menu_text_x(fs), y0 + inset * 1.6 + (fs * 0.7).floor()], (fs * 0.7).floor(), AMBER);
    let mut y = y0 + menu_head(fs);
    for (i, item) in items.iter().enumerate() {
        let selected = i == state.index;
        let couch = item.action == "couch";
        let mut label = item.label.clone();
        let a = lobby.button_name(id, "A");
        if item.action == "quit" && state.confirm && selected { label = if mouse { "Click again to quit".into() } else { format!("Press {a} again to quit") }; }
        if item.action == "remove" && state.confirm && selected { label = if mouse { format!("Click again to remove {}", short(&item.name, 14)) } else { format!("Press {a} again to remove {}", short(&item.name, 14)) }; }
        if selected { c.style_box([x + inset, y, mw - 2.0 * inset, line - 4.0 + if couch { row } else { 0.0 }], [12.0 * k; 4], VIOLET.with_alpha(0.35), None); }
        // Caps centred in the row's `line - 4` (Outfit's cap height is 0.7 em).
        text(c, &label, [x + inset + menu_text_x(fs), y + (line - 4.0 + fs * 0.7) / 2.0], fs, if selected { TEXT } else { DIM });
        if couch {
            // Yours is filled.
            let mine = lobby.couch_place(id);
            let colour = Color::from_string(lobby.profile(id).map_or("", |p| p.color.as_str()), AMBER);
            for place in 0..lobby.profiles.len() {
                let cell = [x + inset + menu_text_x(fs) + place as f32 * (block + gap), y + line - 2.0, block, block];
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
    let r = [32.0 * scale, c.size[1] - bh - 32.0 * scale, bw, bh];
    crate::paint::panel(c, r, scale);
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
