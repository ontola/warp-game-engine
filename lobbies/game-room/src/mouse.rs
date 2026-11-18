//! The mouse in the room. It is nobody: no character, no seat. Pointing at
//! furniture lights that station's buttons where players see them, clicking
//! a button does what that button does (anywhere else on the furniture is
//! Y), the wheel browses the shelf or skips tracks, and Escape opens a Start
//! menu of the room's own, from which GameNight can be quit.

use crate::host::Station;
use crate::live::Lobby;
use crate::lobby::Menu;
use app::Pointer;
use game_room_sim::layout as l;
use game_room_sim::world::BTN_A;
use glam::{Vec3, Vec4Swizzles};

/// The mouse's key in `Lobby::menus`; never a player id.
pub const MOUSE: &str = "mouse";
/// Seconds without movement after which the mouse lets go of what it points at.
const IDLE: f32 = 5.0;
/// World metres per prompt texture pixel (room.rs `prompt`).
const PROMPT_PX: f32 = 0.0042;
/// Stations the mouse can work. The doors are somebody's: picking up a
/// profile needs a controller. The exit quits Splitscreen Club for the mouse.
const STATIONS: [&str; 5] = ["tv", "shelf", "queue", "music", "exit"];
/// Seconds a first click on the exit's Quit waits for the second.
const QUIT_CONFIRM: f32 = 3.0;

#[derive(Default)]
pub struct Mouse {
    pub at: Option<[f32; 2]>,
    pub size: [u32; 2],
    moved_at: f32,
    /// What it points at, and which of that station's buttons.
    pub hover: Option<(Station, Option<String>)>,
    /// Until when a second click on the exit's Quit quits.
    pub quit_until: f32,
}

impl Lobby {
    /// Where a world point shows on a screen of `size` (tests point with it).
    pub fn screen_point(&self, p: Vec3, size: [u32; 2]) -> Option<[f32; 2]> {
        crate::hud::to_screen(crate::hud::view_proj(&self.stage.room.camera, size), size, p)
    }

    pub fn mouse_menu_open(&self) -> bool { self.menus.contains_key(MOUSE) }

    /// The button the mouse hovers on prompt `name`, if any.
    pub fn mouse_hover_key(&self, name: &str) -> Option<String> {
        let (station, key) = self.mouse.hover.as_ref()?;
        if self.mouse_station().is_none() || prompt_name(station) != name { return None; }
        key.clone()
    }

    /// Whether the exit's Quit waits for its second click.
    pub fn mouse_quit_armed(&self) -> bool { self.time < self.mouse.quit_until }

    /// The station the mouse points at, while it is being used.
    pub fn mouse_station(&self) -> Option<Station> {
        if self.time - self.mouse.moved_at > IDLE || self.menus.contains_key(MOUSE) { return None; }
        self.mouse.hover.as_ref().map(|h| h.0.clone())
    }

    /// Escape: the mouse's Start menu opens, or closes (a page goes back).
    pub fn mouse_escape(&mut self) {
        match self.menus.get(MOUSE) {
            Some(m) if !m.page.is_empty() => self.menu_page(MOUSE, ""),
            Some(_) => { self.menus.remove(MOUSE); }
            None => { self.menus.insert(MOUSE.into(), Menu::default()); }
        }
        self.screens_dirty = true;
    }

    /// One tick of free cursor input.
    pub fn mouse_input(&mut self, p: Pointer, size: [u32; 2]) {
        self.mouse.size = size;
        if p.moved { self.mouse.moved_at = self.time; }
        self.mouse.at = p.position;
        let used = p.moved || p.pressed || p.wheel != 0.0;
        if !self.accepts_input() {
            self.mouse.hover = None;
            return;
        }
        if self.menus.contains_key(MOUSE) {
            self.mouse.hover = None;
            self.mouse_menu(p, size);
            return;
        }
        let Some(at) = p.position else { self.mouse.hover = None; return };
        if used { self.mouse.hover = self.pointed(at, size); }
        let Some((station, key)) = self.mouse.hover.clone() else { return };
        if p.pressed { self.mouse_press(station.clone(), key.as_deref().unwrap_or("Y")); }
        if p.wheel != 0.0 {
            // Wheel up is back through the shelf, like LB.
            let key = if p.wheel > 0.0 { "LB" } else { "RB" };
            if station.kind == "shelf" || station.kind == "music" { self.mouse_press(station, key); }
        }
    }

    /// A button of `station`, pressed by the mouse.
    fn mouse_press(&mut self, station: Station, key: &str) {
        match (station.kind.as_str(), key) {
            ("shelf", "LB" | "RB") => {
                if self.host.shelf_games().len() < 2 { return; }
                self.press_prompt("shelf", key);
                self.browse(if key == "LB" { -1 } else { 1 });
            }
            ("music", "LB" | "RB") => self.skip_music_as(None, if key == "LB" { -1 } else { 1 }),
            ("exit", _) => {
                self.press_prompt("exit", "Y");
                if self.mouse_quit_armed() {
                    self.mouse.quit_until = 0.0;
                    if self.client.as_ref().is_some_and(|c| c.connected) { self.command(serde_json::json!({"type": "quit_party"})); } else { self.quit = true; }
                } else {
                    self.mouse.quit_until = self.time + QUIT_CONFIRM;
                }
                self.screens_dirty = true;
            }
            (_, "X") => self.use_station_as(None, station, true),
            (_, "Y") => self.use_station_as(None, station, false),
            _ => {}
        }
    }

    /// Hover picks the item under the cursor; a click works it, and a
    /// click outside the menu closes it.
    fn mouse_menu(&mut self, p: Pointer, size: [u32; 2]) {
        let Some(at) = p.position else { return };
        let Some((frame, rows)) = crate::hud::mouse_menu_rows(self, size) else { return };
        let row = rows.iter().position(|r| inside(*r, at));
        if let (Some(i), true) = (row, p.moved || p.pressed) {
            let menu = self.menus.get_mut(MOUSE).unwrap();
            if menu.index != i { menu.index = i; menu.confirm = false; }
        }
        if !p.pressed { return; }
        if row.is_some() { self.menu_input(MOUSE, BTN_A, [0.0; 2]); }
        else if !inside(frame, at) { self.menus.remove(MOUSE); }
    }

    /// The station, and the button of it, under the cursor at `at`.
    fn pointed(&self, at: [f32; 2], size: [u32; 2]) -> Option<(Station, Option<String>)> {
        let view_proj = crate::hud::view_proj(&self.stage.room.camera, size);
        // Shown buttons first: they float in front of the furniture.
        for name in STATIONS {
            if !self.prompt_active.get(name).copied().unwrap_or(false) { continue; }
            if let Some(key) = self.pill_at(name, view_proj, size, at) {
                let station = self.mouse_station_named(name)?;
                return Some((station, Some(key)));
            }
        }
        // Then the furniture: where the cursor's ray meets the plane players
        // use it on, below the ceiling.
        let ndc = [at[0] / size[0] as f32 * 2.0 - 1.0, 1.0 - at[1] / size[1] as f32 * 2.0];
        let inv = view_proj.inverse();
        let near = inv * glam::Vec4::new(ndc[0], ndc[1], 0.0, 1.0);
        let far = inv * glam::Vec4::new(ndc[0], ndc[1], 1.0, 1.0);
        let (near, far) = (near.xyz() / near.w, far.xyz() / far.w);
        let dir = far - near;
        if dir.z.abs() < 1e-6 { return None; }
        let t = (l::OBJECTS_Z as f32 - near.z) / dir.z;
        if t <= 0.0 { return None; }
        let hit = near + dir * t;
        if !(-0.2..=6.4).contains(&hit.y) { return None; }
        self.host.station_at(hit.x as f64, None).or_else(|| ((hit.x as f64 - l::EXIT_DOOR).abs() <= 0.6).then(exit_station)).map(|s| (s, None))
    }

    fn mouse_station_named(&self, name: &str) -> Option<Station> {
        let x = match name {
            "tv" => 0.0,
            "shelf" => (l::GAME_SHELF.0 + l::GAME_SHELF.1) / 2.0,
            "queue" => (l::UP_NEXT.0 + l::UP_NEXT.1) / 2.0,
            "exit" => return Some(exit_station()),
            _ => l::JUKEBOX,
        };
        self.host.station_at(x, None)
    }

    /// The button of prompt `name` under `at` (paint.rs `prompt` layout on
    /// the live lobby's wide prompt screens).
    fn pill_at(&self, name: &str, view_proj: glam::Mat4, size: [u32; 2], at: [f32; 2]) -> Option<String> {
        let def = self.build_prompts().iter().find(|p| p.name == name)?;
        let prompt = self.prompts.get(name)?;
        let items: Vec<(String, String)> = prompt.items.iter().map(|(k, t, _)| (k.clone(), t.clone())).collect();
        let width = crate::live::PROMPT_WIDTH as f32;
        let mut x = crate::paint::prompt_left(name, width, crate::room::prompt_size(&items)[0] as f32 - prompt.concealed_width()) + 20.0;
        let center = def.pos + Vec3::new(0.0, self.prompt_bob(name, def.pos.x), 0.0);
        let world = |tx: f32, ty: f32| center + Vec3::new((tx - width / 2.0) * PROMPT_PX, (48.0 - ty) * PROMPT_PX, 0.0);
        for (key, label) in &items {
            let badge = if key == "LB" || key == "RB" { 150.0 } else { 96.0 };
            let tw = crate::fonts::measure_bold(label, 44.0);
            let (a, b) = (world(x, 6.0), world(x + badge + tw + 16.0, 90.0));
            x += badge + tw + 36.0;
            let (Some(a), Some(b)) = (crate::hud::to_screen(view_proj, size, a), crate::hud::to_screen(view_proj, size, b)) else { continue };
            if inside([a[0], a[1], b[0] - a[0], b[1] - a[1]], at) { return Some(key.clone()); }
        }
        None
    }
}

fn exit_station() -> Station {
    Station { id: "exit".into(), kind: "exit".into(), label: "Quit Splitscreen Club".into(), enabled: true, ..Default::default() }
}

/// The prompt a station's buttons float on.
pub fn prompt_name(station: &Station) -> &str { if station.kind == "music" { "music" } else { &station.id } }

fn inside(r: [f32; 4], p: [f32; 2]) -> bool { p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3] }
