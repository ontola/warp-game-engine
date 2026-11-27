//! The mouse in the room. It is nobody: no character, no seat. Pointing at
//! furniture lights that station's buttons where players see them, clicking
//! a button does what that button does (anywhere else on the furniture is
//! Y), the wheel browses the shelf or skips tracks, and Escape opens a Start
//! menu of the room's own, from which Splitscreen Club can be quit.

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
const PROMPT_PX: f32 = 0.0042 * crate::live::PROMPT_SCALE;
/// Stations the mouse can work. The doors are somebody's: picking up a
/// profile needs a controller. The exit quits Splitscreen Club for the mouse.
const STATIONS: [&str; 6] = ["tv", "shelf", "queue", "music", "privacy", "exit"];
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
            Some(m) if m.page.starts_with("player:") => { self.menus.remove(MOUSE); }
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
        if self.catalog_open() {
            self.mouse.hover = None;
            self.catalog_pointer(p, size);
            return;
        }
        let Some(at) = p.position else { self.mouse.hover = None; return };
        self.sync_visit();
        // A click on a player opens what the room can do with them.
        if p.pressed {
            // The visit field over the QR code: a click types into it, a
            // click anywhere else lets it go.
            if self.visit_at(at, size) { if !self.visit_typing() { self.open_visit(None); } return; }
            if self.visit_typing() { self.close_visit(); }
            if let Some(id) = self.player_at(at, size) { self.open_player_menu(&id); return; }
            // A click on the QR code copies the room's link (a browser host).
            if self.outbox.is_some() && !self.show.room_code.is_empty() && self.plaque_at(at, size) {
                let code = self.show.room_code.clone();
                let url = crate::host::get(&self.host.links["room"], "join_url", "");
                let url = if url.is_empty() || url == "<null>" { None } else { Some(url) };
                if let Some(out) = &mut self.outbox { out.push(serde_json::json!({"type": "copy_room_code", "code": code, "url": url})); }
                self.toast(if url.is_some() { "Room link copied".to_string() } else { format!("Room code {code} copied") }, crate::room::GREEN);
                return;
            }
        }
        if used {
            let pointed = self.pointed(at, size);
            // Between a station's furniture and its buttons (and the shelf's
            // side covers) the hover holds: the whole zone is the station's.
            let keep = pointed.is_none() && self.mouse.hover.as_ref().is_some_and(|h| self.in_zone(&h.0, at, size));
            if !keep { self.mouse.hover = pointed.map(|(s, k)| self.mouse_key(s, k, at, size)); }
            if let Some((s, k)) = &self.mouse.hover && !self.mouse_can(s, k.as_deref()) { self.mouse.hover = None; }
        }
        let Some((station, key)) = self.mouse.hover.clone() else { return };
        // A click on the game shelf itself opens the catalog; its buttons play.
        if p.pressed && station.kind == "shelf" && key.is_none() { self.open_catalog(); return; }
        if p.pressed { self.mouse_press(station.clone(), key.as_deref().unwrap_or("Y")); }
        if p.wheel != 0.0 {
            // Wheel up is back through the shelf, like LB.
            let key = if p.wheel > 0.0 { "LB" } else { "RB" };
            if station.kind == "shelf" || station.kind == "music" { self.mouse_press(station, key); }
        }
    }

    /// What the mouse can do with nobody in the room: the browser host
    /// starts no game without a player, so Play does not light up there.
    fn mouse_can(&self, station: &Station, key: Option<&str>) -> bool {
        if !self.world.players.is_empty() || self.outbox.is_none() { return true; }
        match station.kind.as_str() {
            "tv" | "queue" => key == Some("X") && station.kind == "queue",
            "shelf" => key != Some("Y"),
            _ => true,
        }
    }

    /// On the shelf's furniture a click opens the catalog: say so by
    /// lighting "All games"; on its side covers, LB and RB.
    fn mouse_key(&self, station: Station, key: Option<String>, at: [f32; 2], size: [u32; 2]) -> (Station, Option<String>) {
        if station.kind != "shelf" || key.is_some() { return (station, key); }
        let key = self.shelf_side_at(at, size).map_or("R3", |side| if side < 0 { "LB" } else { "RB" });
        (station, Some(key.into()))
    }

    /// The shelf's side cover (and its bumper) under `at`: -1 left, 1 right.
    fn shelf_side_at(&self, at: [f32; 2], size: [u32; 2]) -> Option<i32> {
        if self.host.shelf_games().len() < 2 { return None; }
        let view_proj = crate::hud::view_proj(&self.stage.room.camera, size);
        let bx = ((l::BOOKCASE.0 + l::BOOKCASE.1) / 2.0) as f32;
        let z = l::BACK as f32 + 0.7;
        [-1, 1].into_iter().find(|&side| {
            use crate::room::{BUMPER_Y, SIDE_COVER_HALF as H, SIDE_COVER_X, SIDE_COVER_Y};
            let c = Vec3::new(bx + side as f32 * SIDE_COVER_X, SIDE_COVER_Y, z);
            let (Some(a), Some(b)) = (crate::hud::to_screen(view_proj, size, c + Vec3::new(-H[0] - 0.05, BUMPER_Y + 0.2 - SIDE_COVER_Y, 0.0)),
                crate::hud::to_screen(view_proj, size, c + Vec3::new(H[0] + 0.05, -H[1] - 0.05, 0.0))) else { return false };
            inside([a[0], a[1], b[0] - a[0], b[1] - a[1]], at)
        })
    }

    /// Whether `at` is in `station`'s zone: from its furniture up to and
    /// around its buttons (for the shelf, its side covers too).
    fn in_zone(&self, station: &Station, at: [f32; 2], size: [u32; 2]) -> bool {
        let name = prompt_name(station);
        let Some(def) = self.build_prompts().iter().find(|p| p.name == name).map(|p| p.pos) else { return false };
        let half = crate::live::PROMPT_WIDTH as f32 * PROMPT_PX * 0.5 * 0.8 + if station.kind == "shelf" { 0.5 } else { 0.2 };
        let view_proj = crate::hud::view_proj(&self.stage.room.camera, size);
        let floor = Vec3::new(def.x, 0.0, def.z);
        let (Some(a), Some(b)) = (crate::hud::to_screen(view_proj, size, Vec3::new(def.x - half, def.y + 0.45, def.z)),
            crate::hud::to_screen(view_proj, size, floor + Vec3::new(half, 0.0, 0.0))) else { return false };
        inside([a[0], a[1], b[0] - a[0], b[1] - a[1]], at)
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
            ("tv", "LB" | "RB") => self.skip_game(None, if key == "LB" { -1 } else { 1 }),
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
            ("shelf", "R3") => { self.press_prompt("shelf", "R3"); self.open_catalog(); }
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

    /// The player under the cursor at `at`: the front-most whose body
    /// (feet to head) is there.
    pub fn player_at(&self, at: [f32; 2], size: [u32; 2]) -> Option<String> {
        let view_proj = crate::hud::view_proj(&self.stage.room.camera, size);
        let mut best: Option<(f32, String)> = None;
        for p in &self.world.players {
            let depth = self.actor_depth(&p.id);
            let tall = if p.sleeping { 1.2 } else { 2.1 };
            let feet = crate::hud::to_screen(view_proj, size, Vec3::new(p.x as f32, p.y as f32, depth));
            let head = crate::hud::to_screen(view_proj, size, Vec3::new(p.x as f32, p.y as f32 + tall, depth));
            let (Some(feet), Some(head)) = (feet, head) else { continue };
            let half = (feet[1] - head[1]).abs() * 0.28;
            let hit = at[0] >= feet[0] - half && at[0] <= feet[0] + half && at[1] >= head[1] && at[1] <= feet[1];
            if hit && best.as_ref().is_none_or(|b| depth > b.0) { best = Some((depth, p.id.clone())); }
        }
        best.map(|b| b.1)
    }

    /// The plaque with the room's QR code and code, on screen.
    pub fn plaque_rect(&self, size: [u32; 2]) -> Option<[f32; 4]> {
        let view_proj = crate::hud::view_proj(&self.stage.room.camera, size);
        let c = Vec3::new(l::PLAQUE_CENTER[0] as f32, l::PLAQUE_CENTER[1] as f32, l::PLAQUE_CENTER[2] as f32 + 0.04);
        let corner = |dx: f32, dy: f32| crate::hud::to_screen(view_proj, size, c + Vec3::new(dx * 1.1, dy * 1.315, 0.0));
        let (Some(a), Some(b)) = (corner(-1.0, 1.0), corner(1.0, -1.0)) else { return None };
        Some([a[0].min(b[0]), a[1].min(b[1]), (b[0] - a[0]).abs(), (b[1] - a[1]).abs()])
    }

    /// Whether `at` is on the plaque with the room's QR code and code.
    pub fn plaque_at(&self, at: [f32; 2], size: [u32; 2]) -> bool { self.plaque_rect(size).is_some_and(|r| inside(r, at)) }

    /// The visit-a-room screen over the plaque (visit.rs), on screen.
    pub fn visit_rect(&self, size: [u32; 2]) -> Option<[f32; 4]> {
        if self.catalog_open() { return None; }
        let view_proj = crate::hud::view_proj(&self.stage.room.camera, size);
        let c = Vec3::from(crate::room::VISIT_CENTER) + Vec3::new(0.0, 0.0, 0.04);
        let [w, h] = crate::room::VISIT_SIZE;
        let corner = |dx: f32, dy: f32| crate::hud::to_screen(view_proj, size, c + Vec3::new(dx * w / 2.0, dy * h / 2.0, 0.0));
        let (Some(a), Some(b)) = (corner(-1.0, 1.0), corner(1.0, -1.0)) else { return None };
        Some([a[0].min(b[0]), a[1].min(b[1]), (b[0] - a[0]).abs(), (b[1] - a[1]).abs()])
    }

    /// Whether `at` is on the visit field.
    pub fn visit_at(&self, at: [f32; 2], size: [u32; 2]) -> bool { self.visit_rect(size).is_some_and(|r| inside(r, at)) }

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
        self.host.station_at(hit.x as f64, None).or_else(|| (self.outbox.is_none() && (hit.x as f64 - l::EXIT_DOOR).abs() <= 0.6).then(exit_station)).map(|s| (s, None))
    }

    fn mouse_station_named(&self, name: &str) -> Option<Station> {
        let x = match name {
            "tv" => 0.0,
            "shelf" => (l::GAME_SHELF.0 + l::GAME_SHELF.1) / 2.0,
            "queue" => (l::UP_NEXT.0 + l::UP_NEXT.1) / 2.0,
            // A browser tab has nothing to quit.
            "exit" => return self.outbox.is_none().then(exit_station),
            "privacy" => l::SWITCH,
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
