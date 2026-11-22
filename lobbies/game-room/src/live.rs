//! The live lobby: the room built in Rust, its animation (neon, moving
//! heads, disco ball, EQ, pads), the platformer simulation and everyone in
//! it as pixel-art sprites (view.gd), on the app shell.

use crate::character::{self, Look, FRAMES};
use crate::dump::{linear, Room};
use crate::host::{get, Host};
use crate::lobby::{Menu, Toast, Web};
use crate::paint::{Prompt, Show};
use crate::props::Props;
use crate::room::{self, Mood, RoomBuild};
use crate::stage::{Options, Stage};
use app::{Buttons, Controls, Draw, Frame, Game};
use game_room_sim::layout;
use game_room_sim::world::{self, World};
use gdcompat::Pcg;
use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::headlights::LocalLight;
use render::hud::{HudDrawList, HudImage, ImageFilter};
use render::scene::{Material, Scene, ALPHA_TEST, PIXEL_ART, TWO_SIDED};
use render::screen::Screen;
use render::slots::{SlotId, Slots};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::f32::consts::PI;

pub const MAX_ACTORS: usize = 8;
/// Sprite3D pixel_size.
const PIXEL: f32 = 0.04;
const SHEET_W: u32 = character::W * FRAMES.len() as u32;
/// Sheets sit in the actor layer three times enlarged: the renderer samples
/// the top mip bilinearly, and at 3x that blurs only a sliver of each pixel.
const UP: u32 = 3;

/// Where slot `k`'s (enlarged) sheet sits in the actor layer.
fn sheet_origin(k: usize) -> [u32; 2] { [0, k as u32 * (character::H * UP + 8)] }

/// The sprite quads: per slot, per frame, facing right and flipped.
struct ActorMeshes { layer: i32, slots: Slots, meshes: Vec<[[usize; 2]; FRAMES.len()]>, bodies: Slots, body: usize }

impl ActorMeshes {
    fn reserve(scene: &mut Scene) -> Self {
        let layer = scene.add_screen_texture("actors");
        // Godot: shaded, alpha cut, nearest, double sided.
        let material = scene.add_material(Material { tint: [1.0; 3], layer, roughness: 1.0, flags: ALPHA_TEST | TWO_SIDED | PIXEL_ART, scale: 1.0, metal: 0.0 });
        let side = render::screen::SIZE as f32;
        let (fw, fh) = (character::W as f32 * PIXEL, character::H as f32 * PIXEL);
        // Sprite3D offset: the frame's centre sits H/2 - 2 pixels up.
        let lift = (character::H as f32 / 2.0 - 2.0) * PIXEL;
        let meshes = (0..MAX_ACTORS).map(|k| {
            let [ox, oy] = sheet_origin(k);
            std::array::from_fn(|f| std::array::from_fn(|flip| {
                let (u0, u1) = ((ox + f as u32 * character::W * UP) as f32 / side, (ox + (f as u32 + 1) * character::W * UP) as f32 / side);
                let (u0, u1) = if flip == 1 { (u1, u0) } else { (u0, u1) };
                let (v0, v1) = (oy as f32 / side, (oy + character::H * UP) as f32 / side);
                let corner = |x: f32, y: f32| Vertex { pos: [(x - 0.5) * fw, lift + (0.5 - y) * fh, 0.0], normal: [0.0, 0.0, 1.0], uv: [u0 + (u1 - u0) * x, v0 + (v1 - v0) * y] };
                let verts = [corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0), corner(0.0, 0.0)];
                scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &verts, indices: &[0, 1, 2, 0, 2, 3], material }]))
            }))
        }).collect();
        let slots = Slots::reserve(scene, MAX_ACTORS);
        // A flat sprite seen edge-on by the lights casts a hairline shadow,
        // so the sprite casts none and an invisible capsule body does.
        for i in &mut scene.instances[slots.first..slots.first + MAX_ACTORS] { i.flags |= render::scene::NO_SHADOW; }
        let grey = scene.add_material(Material { tint: [0.5; 3], layer: -1, roughness: 1.0, flags: 0, scale: 1.0, metal: 0.0 });
        // Godot's CapsuleMesh: radius 0.36, 1.7 tall.
        let body = scene.add_mesh(&render::primitives::capsule(grey, (1.7 - 0.72) / 0.36, 8, 12).mesh());
        let bodies = Slots::reserve(scene, MAX_ACTORS);
        for i in &mut scene.instances[bodies.first..bodies.first + MAX_ACTORS] { i.hide_in_views(0xff); }
        Self { layer, slots, meshes, bodies, body }
    }
}

/// One player on screen (view.gd's actor).
struct Actor {
    slot: SlotId,
    body: SlotId,
    key: String,
    color: String,
    depth: f32,
    order: usize,
    prev: [f64; 2],
}

/// A party player as the lobby shows it.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub couch: Option<i64>,
    pub color: String,
    pub skin: String,
    /// The drawn face, as Splitscreen Club sends it (face.rs).
    pub avatar: String,
}

/// demo.gd's bots: wander to random spots, hop now and then, sometimes
/// punch or fire.
struct Bot { target: f64, next: f64, jump: f64 }

pub struct Lobby {
    build: RoomBuild,
    pub(crate) stage: Stage,
    actors_view: ActorMeshes,
    pub world: World,
    /// What the host's snapshot says, and what follows from it.
    pub host: Host,
    pub(crate) client: Option<splitscreen_club::lobby::Lobby>,
    /// Party players, in the host's order.
    pub profiles: Vec<Profile>,
    actors: HashMap<String, Actor>,
    next_order: usize,
    sheets_dirty: bool,
    screen: Option<Screen>,
    pub show: Show,
    pub prompts: HashMap<String, Prompt>,
    pub prompt_active: HashMap<String, bool>,
    /// Prompts shown to nobody in particular, to show new players what to
    /// do (the TV before the first game, doors with a waiting profile):
    /// they hop instead of bobbing.
    pub nudges: std::collections::HashSet<String>,
    /// A game has run since the room opened: the TV stops nudging.
    pub started_once: bool,
    /// Each prompt's glow, easing towards lit while someone stands there.
    prompt_glow: HashMap<String, f32>,
    pub mood: Mood,
    /// Dump instances and materials RoomBuild::pose changes.
    animated: (Vec<usize>, Vec<usize>),
    pub(crate) time: f32,
    since_paint: f32,
    pub(crate) screens_dirty: bool,
    /// Keyboard and pad players of a demo (`play`).
    local: Vec<String>,
    bots: HashMap<String, Bot>,
    rng: Pcg,
    uploaded: bool,
    pub(crate) props: Props,
    board: crate::board::Board,
    events: Vec<world::Event>,
    // main.gd's state
    pub(crate) controllers: HashMap<String, String>,
    pub(crate) held: HashMap<String, u32>,
    pub(crate) input_blocked: HashSet<String>,
    pub(crate) menus: HashMap<String, Menu>,
    pub(crate) bench: Vec<String>,
    pub(crate) phones: Vec<String>,
    pub(crate) lost_since: HashMap<String, f32>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) unjoined: HashMap<String, (u32, bool)>,
    pub(crate) holds: HashMap<String, (String, f32)>,
    pub(crate) station_cooldown: HashMap<String, f64>,
    pub(crate) pickup_cooldown: f64,
    pub(crate) item_presses: HashMap<u32, (world::Kind, Vec3, f32)>,
    pub(crate) pressed_flash: HashMap<String, f32>,
    pub(crate) claims: HashMap<String, (serde_json::Value, f32)>,
    pub(crate) pop_box: Option<(f32, f32)>,
    pub(crate) featured_visible: bool,
    pub(crate) bumpers_visible: bool,
    display_active: Option<bool>,
    /// When the host last handed the screen over or took it back.
    display_at: web_time::Instant,
    pub(crate) quit: bool,
    pub(crate) web_tx: Sender<Web>,
    web_rx: Receiver<Web>,
    links_due: f32,
    last_queue: Vec<String>,
    /// Hidden-by-default room parts the lobby shows (featured box, bumpers).
    featured: Vec<usize>,
    bumpers: Vec<usize>,
    hud_size: [u32; 2],
    pub(crate) mouse: crate::mouse::Mouse,
    /// Without a host: the commands the lobby would have sent it, for a host
    /// in the same program to take (the browser build, examples/browser/main.rs).
    pub outbox: Option<Vec<serde_json::Value>>,
}

/// Width of every prompt screen in the live lobby.
pub(crate) const PROMPT_WIDTH: u32 = 1024;

const NAMES: [&str; 6] = ["Joep", "Mango", "Disco", "Waffle", "Pickle", "Rocket"];
/// The host's guest colours and skins (demo.gd).
const COLORS: [&str; 6] = ["#e36c76", "#49b8bc", "#d7ad43", "#a67bdd", "#4daf7c", "#cf7aaf"];
const SKINS: [&str; 6] = ["#f5e9be", "#d99b72", "#efc39a", "#905c40", "#bf855b", "#654332"];

impl Lobby {
    /// The demo: an offline party of `local` keyboard/pad players and
    /// `bots` demo bots (demo.gd), with a library, a queue and music.
    pub fn new(options: Options, local: usize, bots: usize) -> Result<Self, String> {
        let mut lobby = Self::build(options, None)?;
        lobby.host.demo = true;
        let count = (local + bots).min(MAX_ACTORS);
        let (party, links) = crate::lobby::demo_party(count, &NAMES, &COLORS, &SKINS);
        lobby.set_party(party);
        lobby.set_links(links);
        for i in 0..count {
            let id = format!("p{i}");
            if i < local { lobby.local.push(id); continue; }
            let x = lobby.world.player(&id).map_or(0.0, |p| p.x);
            let next = lobby.rng.randf_range_f32(0.5, 2.0) as f64;
            lobby.bots.insert(id, Bot { target: x, next, jump: 0.0 });
        }
        Ok(lobby)
    }

    /// The demo's local players, one per seat in seat order (a browser host
    /// adds one when a pad joins and drops one that left).
    pub fn set_local(&mut self, ids: Vec<String>) { self.local = ids; }

    /// The room's light bake, for [`crate::stage::Options::light_cache`].
    pub fn light_cache(&self) -> Vec<u8> { self.stage.light_cache() }

    /// The lobby of a Splitscreen Club host (GAMENIGHT_ADDR, GAMENIGHT_GAME_ID,
    /// GAMENIGHT_TOKEN, GAMENIGHT_LINKS_URL).
    pub fn connect(options: Options) -> Result<Self, String> {
        Self::connect_with(options, splitscreen_club::lobby::Config::from_env("game-room"))
    }

    pub fn connect_with(options: Options, config: splitscreen_club::lobby::Config) -> Result<Self, String> {
        Self::build(options, Some(splitscreen_club::lobby::Lobby::connect(config)))
    }

    fn build(options: Options, client: Option<splitscreen_club::lobby::Lobby>) -> Result<Self, String> {
        let mut build = room::build();
        let kit = crate::kit::add(&mut build);
        let animated = build.animated();
        let mut parts = build.clone();
        // Prompts change their buttons at run time: give each a screen wide
        // enough for any of them, and stretch its quad to match.
        for p in &build.prompts {
            if let Some(screen) = build.screens.iter().find(|s| s.name == format!("prompt_{}", p.name)) {
                parts.dump.textures[screen.texture].width = PROMPT_WIDTH;
            }
        }
        let mut room = Room::from_build(parts);
        room.live = true;
        let (mut actors_view, mut props, mut board) = (None, None, None);
        let stage = Stage::new(room, options, |scene, room| {
            actors_view = Some(ActorMeshes::reserve(scene));
            board = Some(crate::board::Board::reserve(&kit, scene, room));
            props = Some(Props::reserve(kit, scene, room));
        })?;
        let lobby_id = client.as_ref().map_or_else(|| "game-room".to_string(), |c| c.config.lobby_id.clone());
        let featured = build.dump.instances.iter().enumerate().filter(|(_, i)| i.name.starts_with("featured/")).map(|(k, _)| k).collect();
        let bumpers = build.dump.instances.iter().enumerate().filter(|(_, i)| i.name == "shelf_lb" || i.name == "shelf_rb").map(|(k, _)| k).collect();
        let (web_tx, web_rx) = std::sync::mpsc::channel();
        let mut lobby = Self {
            build, stage, actors_view: actors_view.expect("actor meshes"), world: World::new(7), host: Host::new(&lobby_id), client, profiles: vec![], actors: HashMap::new(),
            next_order: 0, sheets_dirty: true, screen: None, show: Show::default(), prompts: HashMap::new(), prompt_active: HashMap::new(), nudges: Default::default(), started_once: false, prompt_glow: HashMap::new(),
            mood: Mood::default(), animated, time: 0.0, since_paint: 0.0, screens_dirty: true, local: vec![], bots: HashMap::new(), rng: Pcg::new(42), uploaded: false,
            props: props.expect("props"), board: board.expect("board"), events: vec![], controllers: HashMap::new(), held: HashMap::new(), input_blocked: HashSet::new(), menus: HashMap::new(),
            bench: vec![], phones: vec![], lost_since: HashMap::new(), toasts: vec![], unjoined: HashMap::new(), holds: HashMap::new(), station_cooldown: HashMap::new(),
            pickup_cooldown: 0.0, item_presses: HashMap::new(), pressed_flash: HashMap::new(), claims: HashMap::new(), pop_box: None, featured_visible: false,
            bumpers_visible: false, display_active: None, display_at: web_time::Instant::now(), quit: false, web_tx, web_rx, links_due: 0.0, last_queue: vec![], featured, bumpers, hud_size: [1920, 1080], mouse: Default::default(), outbox: None,
        };
        lobby.prompts = lobby.build.prompts.iter().map(|p| (p.name.clone(), Prompt {
            items: p.items.iter().map(|(k, t)| (k.clone(), t.clone(), true)).collect(), enabled: true, ..Default::default() })).collect();
        Ok(lobby)
    }

    pub fn names() -> &'static [&'static str] { &NAMES }

    pub(crate) fn build_prompts(&self) -> &[crate::room::PromptDef] { &self.build.prompts }

    /// The z a player is drawn at (view.gd `depth`).
    pub fn actor_depth(&self, id: &str) -> f32 { self.actors.get(id).map_or(0.0, |a| a.depth + (a.order % 8) as f32 * 0.03) }

    /// Show an event as if the world had sent it.
    pub fn event(&mut self, e: world::Event) { self.events.push(e); }

    fn bot_inputs(&mut self) {
        let dt = world::DT;
        let ids: Vec<String> = self.bots.keys().cloned().collect();
        for id in ids {
            let Some(x) = self.world.player(&id).map(|p| p.x) else { continue };
            let bot = self.bots.get_mut(&id).unwrap();
            let mut buttons = 0;
            let mut stick = [0.0f32; 2];
            bot.next -= dt;
            if bot.next <= 0.0 {
                bot.next = self.rng.randf_range_f32(1.0, 3.0) as f64;
                bot.target = self.rng.randf_range_f32(-10.5, 10.5) as f64;
            }
            if (bot.target - x).abs() > 0.4 { stick[0] = (bot.target - x).signum() as f32; }
            bot.jump -= dt;
            if bot.jump <= 0.0 && self.rng.randf() < 0.03 {
                buttons |= world::BTN_A;
                bot.jump = 0.5;
            } else if bot.jump > 0.2 { buttons |= world::BTN_A; }
            if self.rng.randf() < 0.01 { buttons |= world::BTN_B; }
            if self.rng.randf() < 0.02 { buttons |= world::BTN_RT; }
            self.input(&id, buttons, stick);
        }
    }

    /// The pose's changes to the room, onto the scene and the renderer.
    fn animate(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, delta: f32) -> Result<(), String> {
        self.build.pose(self.time, delta, &self.mood);
        let room = &mut self.stage.room;
        let (instances, materials) = &self.animated;
        let mut dirty: Option<(usize, usize)> = None;
        for &k in instances {
            room.dump.instances[k].transform = self.build.dump.instances[k].transform;
            if let Some(i) = room.sync_instance(&mut self.stage.scene, k, true) {
                dirty = Some(dirty.map_or((i, i + 1), |(a, b)| (a.min(i), b.max(i + 1))));
            }
        }
        if let Some((a, b)) = dirty { renderer.update_instances(gpu, a, &self.stage.scene.instances[a..b])?; }
        for &m in materials {
            room.dump.materials[m] = self.build.dump.materials[m].clone();
            if let Some((id, material)) = room.material_now(m) { renderer.update_materials(gpu, id as usize, &[material])?; }
        }
        room.dump.lights.clone_from(&self.build.dump.lights);
        room.relight();
        Ok(())
    }

    /// view.gd `update`: place every player's sprite.
    fn place_actors(&mut self, alpha: f32, delta: f32) -> Vec<(LocalLight, f32)> {
        let ids: Vec<String> = self.world.players.iter().map(|p| p.id.clone()).collect();
        self.actors.retain(|id, a| {
            let keep = ids.contains(id);
            if !keep {
                self.actors_view.slots.remove(&mut self.stage.scene, a.slot);
                self.actors_view.bodies.remove(&mut self.stage.scene, a.body);
            }
            keep
        });
        let mut glows = vec![];
        let tuning = std::f32::consts::PI;
        for id in &ids {
            let p = self.world.player(id).unwrap().clone();
            let profile = self.profile(id).cloned().unwrap_or_default();
            let phone = self.phones.contains(id);
            let avatar = { use std::hash::{Hash, Hasher}; let mut h = std::collections::hash_map::DefaultHasher::new(); profile.avatar.hash(&mut h); h.finish() };
            let key = format!("{}|{}|{avatar}|{}|{phone}", profile.color, profile.skin, p.hat);
            if !self.actors.contains_key(id) {
                let Some(slot) = self.actors_view.slots.add(&mut self.stage.scene, self.actors_view.meshes[0][0][0], Affine3A::IDENTITY) else { continue };
                let body = self.actors_view.bodies.add(&mut self.stage.scene, self.actors_view.body, Affine3A::IDENTITY).expect("as many bodies as sprites");
                self.actors.insert(id.clone(), Actor { slot, body, key: String::new(), color: profile.color.clone(), depth: 0.0, order: self.next_order, prev: [p.x, p.y] });
                self.next_order += 1;
            }
            let carrier_depth = p.carried_by.as_ref().and_then(|c| self.actors.get(c)).map(|c| c.depth);
            let actor = self.actors.get_mut(id).unwrap();
            if actor.key != key { actor.key = key; actor.color = profile.color.clone(); self.sheets_dirty = true; }
            // Ease towards the depth of the surface below (couch, shelf).
            let mut target = layout::depth_at(p.x, p.y) as f32;
            if let Some(d) = carrier_depth { target = d + 0.06; }
            actor.depth += (target - actor.depth) * (1.0 - (-delta * 12.0).exp());
            let depth = actor.depth + (actor.order % 8) as f32 * 0.03;
            let x = actor.prev[0] + (p.x - actor.prev[0]) * alpha as f64;
            let y = actor.prev[1] + (p.y - actor.prev[1]) * alpha as f64;
            let frame = character::frame_index(&frame_name(&p, self.time));
            let mut squash = 1.0;
            if p.land > 0.0 { squash = 0.86; } else if !p.grounded && p.vy > 4.0 { squash = 1.08; }
            let mut scale = Vec3::new(1.0 / f32::sqrt(squash), squash, 1.0);
            if p.spawn > 0.0 { scale *= 1.0 - p.spawn as f32 * 0.8; }
            let mut rot = if p.stun > 0.0 { (self.time * 30.0).sin() * 0.15 } else { 0.0 };
            let mut offset = Vec3::ZERO;
            let (mut body_rot, mut body_scale, mut body_pos) = (0.0, Vec3::new(scale.x, scale.y, 1.0), Vec3::new(0.0, 0.85 * scale.y, -0.42));
            if p.sleeping {
                // The curled pose on its side, cheek on the surface, breathing.
                let side = p.facing as f32;
                let breath = ((self.time * 1.8 + actor.order as f32).sin() + 1.0) * 0.5;
                rot = side * PI * 0.5;
                scale = Vec3::new(1.0 + breath * 0.025, 1.0, 1.0);
                offset = Vec3::new(side * if p.carried_by.is_some() { 1.16 } else { 0.94 }, 0.52 + breath * 0.012, 0.0);
                (body_rot, body_scale, body_pos) = (PI * 0.5, Vec3::new(0.65, 0.9, 1.0), Vec3::new(0.0, 0.3, -0.42));
            }
            let xf = Affine3A::from_scale_rotation_translation(scale, Quat::from_rotation_z(rot), Vec3::new(x as f32, y as f32, depth) + offset);
            let mesh = self.actors_view.meshes[actor.slot.0][frame][(p.facing < 0.0) as usize];
            self.actors_view.slots.set(&mut self.stage.scene, actor.slot, mesh, xf);
            let node = Vec3::new(x as f32, y as f32, depth);
            let body = Affine3A::from_scale_rotation_translation(body_scale * 0.36, Quat::from_rotation_z(body_rot), node + body_pos);
            self.actors_view.bodies.set(&mut self.stage.scene, actor.body, self.actors_view.body, body);
            // A soft personal glow so every player pops out of the haze.
            let c = linear(&serde_json::json!(crate::godot::Color::from_string(&actor.color, crate::godot::Color::hex("7c5cff")).array()));
            glows.push((LocalLight { position: Vec3::new(x as f32, y as f32 + 0.8, depth + 0.8), direction: Vec3::NEG_Y, intensity_cd: c * 0.9 * tuning,
                range_m: 2.2, inner_cos: -0.999, outer_cos: -1.0, source_radius_m: 0.05, falloff: 1.0 }, 1.0));
        }
        glows
    }

    /// Draw every actor's sheet into the actor layer.
    fn paint_sheets(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        let mut screen = self.screen.take().unwrap();
        let mut list = HudDrawList::new([render::screen::SIZE; 2]);
        let side = render::screen::SIZE as f32;
        for (id, actor) in &self.actors {
            let p = self.world.player(id);
            let profile = self.profile(id).cloned().unwrap_or_default();
            let phone = self.phones.contains(id);
            let face = crate::face::decode(&profile.avatar);
            let mut sheet = character::build(&Look { color: &profile.color, skin: &profile.skin, hat: p.map_or("none", |p| p.hat.as_str()),
                face: face.as_ref().map(|f| character::Face { image: &f.image, center: f.center }), phone });
            // Clear pixels take the outline's colour, so filtering never
            // brings in Godot's transparent white.
            for px in sheet.pixels_mut() { if px[3] == 0 { *px = image::Rgba([11, 11, 18, 0]); } }
            let sheet = image::imageops::resize(&sheet, sheet.width() * UP, sheet.height() * UP, image::imageops::FilterType::Nearest);
            let picture = std::sync::Arc::new(render::picture::Picture::new(sheet.width(), sheet.height(), sheet.into_raw())?);
            let image = HudImage(1000 + actor.slot.0 as u64);
            screen.set_image(gpu, image, &picture, ImageFilter::Nearest);
            let [x, y] = sheet_origin(actor.slot.0);
            list.image([x as f32, y as f32, (SHEET_W * UP) as f32, (character::H * UP) as f32], image, [0.0, 0.0, 1.0, 1.0], [1.0; 4], [0.0, 0.0, side, side]);
        }
        let drawn = screen.draw(gpu, renderer, self.actors_view.layer, [0.0; 4], &list);
        self.screen = Some(screen);
        drawn
    }
}

/// view.gd `_frame`: which sheet frame a player shows.
fn frame_name(p: &world::Player, time: f32) -> String {
    if p.sleeping { return "sleep".into(); }
    if p.cheer > 0.0 { return "cheer".into(); }
    if p.punch > 0.08 { return "throw".into(); }
    if !p.grounded { return if p.vy > 0.0 { "jump" } else { "fall" }.into(); }
    if p.carrying.is_some() { return "cheer".into(); }
    if p.item.is_some() { return "carry".into(); }
    if p.vx.abs() > 0.4 { return format!("walk{}", (p.walk_phase * 2.2) as i64 % 4); }
    format!("idle{}", ((time as f64 * 1.6 + p.x) as i64).rem_euclid(2))
}

/// Pad and keyboard controls as the lobby's buttons and stick.
fn buttons(c: &Controls) -> (u32, [f32; 2]) {
    let pairs = [(Buttons::SOUTH, world::BTN_A), (Buttons::EAST, world::BTN_B), (Buttons::WEST, world::BTN_X), (Buttons::NORTH, world::BTN_Y),
        (Buttons::LEFT_SHOULDER, world::BTN_LB), (Buttons::RIGHT_SHOULDER, world::BTN_RB), (Buttons::SELECT, world::BTN_BACK), (Buttons::START, world::BTN_START),
        (Buttons::UP, world::BTN_UP), (Buttons::DOWN, world::BTN_DOWN), (Buttons::LEFT, world::BTN_LEFT), (Buttons::RIGHT, world::BTN_RIGHT),
        (Buttons::RIGHT_TRIGGER, world::BTN_RT)];
    let held = pairs.iter().filter(|(b, _)| c.held.0 & b != 0).fold(0, |a, (_, w)| a | w);
    // The world's stick is SDL style: y down.
    (held, [c.mv[0], -c.mv[1]])
}

impl Lobby {
    /// Host messages, web replies and the links poll.
    fn poll(&mut self, delta: f32) {
        let events = self.client.as_mut().map(|c| c.poll()).unwrap_or_default();
        for event in events {
            use splitscreen_club::lobby::Event;
            match event {
                Event::Connected => { self.screens_dirty = true; }
                Event::Party => { let party = self.client.as_ref().unwrap().party.clone(); self.set_party(party); }
                Event::Focus(active) => self.focus_changed(active),
                Event::Controllers(frames) => self.controllers_changed(&frames),
                Event::Rejected(message) => self.toast(message, crate::lobby::PINK),
                Event::Disconnected(_) => {
                    self.display_active = None;
                    self.suspend_input();
                    self.toast("Reconnecting… your party is still here.", crate::lobby::AMBER);
                }
            }
        }
        while let Ok(reply) = self.web_rx.try_recv() {
            match reply {
                Web::Links(links) => self.set_links(links),
                Web::PickupRefused(why) => {
                    self.drop_claims();
                    // 409: the character already is someone (hold Y to swap),
                    // or that profile sits on another character.
                    let text = if why.contains("409") { "That profile or character is already taken. Hold Y at the door to swap." }
                        else if why.contains("410") { "That profile left the door. Scan the code again." }
                        else { "That profile could not be picked up. Try again." };
                    self.toast(text, crate::lobby::PINK);
                }
            }
        }
        // The room QR code, its join code and the profiles waiting at the doors.
        let base = self.client.as_ref().map(|c| c.config.links_base.clone()).unwrap_or_default();
        self.links_due -= delta;
        if !base.is_empty() && self.links_due <= 0.0 {
            self.links_due = 1.0;
            let tx = self.web_tx.clone();
            std::thread::spawn(move || {
                let links = splitscreen_club::lobby::http(&base, "GET", "/api/player-links", &[]).ok()
                    .and_then(|body| serde_json::from_slice(&body).ok()).unwrap_or_else(|| serde_json::json!({}));
                let _ = tx.send(Web::Links(links));
            });
        }
    }

    /// Only a host ownership change may claim the screen.
    fn focus_changed(&mut self, active: bool) {
        if self.display_active == Some(active) { return; }
        self.display_active = Some(active);
        self.display_at = web_time::Instant::now();
        self.suspend_input();
    }

    /// What the screens show, from the party (main.gd's painters' inputs).
    fn refresh_show(&mut self) {
        use crate::paint::{Banner, Game as ShowGame, Track};
        let h = &self.host;
        let game_of = |meta: &serde_json::Value| ShowGame { id: get(meta, "id", ""), title: get(meta, "title", "Game"), color: get(meta, "color", "") };
        let mut tv = None;
        let upcoming = h.upcoming();
        if let Some(game) = h.current_game() {
            let id = get(&game, "id", "");
            let mut banner = Banner { kicker: if game["phase"] == "paused" { "PAUSED".into() } else { "NOW PLAYING".into() }, color: crate::lobby::GREEN,
                detail: "Press Y at the TV to resume".into(), loading: None };
            if let Some(loading) = h.loading_session(&id) {
                banner.kicker = h.loading_kicker(&id);
                banner.color = crate::lobby::AMBER;
                banner.detail = loading.label;
                banner.loading = h.loading_shown(&id);
            } else if !h.fits_party(&game) {
                let action = h.tv_station();
                banner.detail = if action.enabled { format!("Y at TV: {}", action.label) } else { "This game needs a different player count".into() };
            }
            tv = Some((game_of(&game), banner));
        } else if let Some(first) = upcoming.first() {
            let meta = h.game_meta(&first.game).cloned().unwrap_or_else(|| serde_json::json!({"id": first.game, "title": first.title}));
            let id = get(&meta, "id", "");
            let mut banner = Banner { kicker: "UP NEXT".into(), color: crate::godot::rgb(0.6, 0.75, 1.0), detail: "Press Y at the TV to start".into(), loading: None };
            let issue = h.issue(&id);
            let warm = &h.party["warm_session"];
            if !issue.is_empty() {
                banner.kicker = issue.to_uppercase();
                banner.color = crate::lobby::PINK;
                banner.detail = "Press Y at the TV to retry".into();
            } else if let Some(loading) = h.loading_session(&id) {
                banner.kicker = h.loading_kicker(&id);
                banner.color = crate::lobby::AMBER;
                banner.detail = loading.label;
                banner.loading = h.loading_shown(&id);
            } else if warm.is_object() && get(warm, "game", "") == id && warm["phase"] == "ready" {
                banner.kicker = "UP NEXT · READY".into();
                banner.color = crate::lobby::GREEN;
            }
            tv = Some((game_of(&meta), banner));
        }
        let cabinet = h.cabinet_queue();
        let queue: Vec<ShowGame> = cabinet.iter().map(|e| h.game_meta(&e.game).map(game_of).unwrap_or(ShowGame { id: e.game.clone(), title: e.title.clone(), color: String::new() })).collect();
        let first = cabinet.first().map(|e| e.game.clone());
        let show = &mut self.show;
        show.tv = tv;
        show.queue = queue;
        show.queue_status = first.as_ref().map(|g| (h.queue_status(g), !h.issue(g).is_empty())).filter(|s| !s.0.is_empty());
        show.queue_loading = first.as_ref().and_then(|g| h.loading_shown(g));
        show.tv_has_next = !upcoming.is_empty();
        show.queue_first = upcoming.len() - cabinet.len();
        show.shelf = h.shelf_games().iter().map(|g| game_of(g)).collect();
        show.shelf_offset = h.shelf_offset;
        show.humans = h.human_count();
        show.profile_colors = self.profiles.iter().map(|p| p.color.clone()).collect();
        show.now_playing = h.has_track().then(|| {
            let t = &h.party["now_playing"];
            let mut artist = get(t, "artist", "");
            if artist.is_empty() { artist = get(t, "source", ""); }
            Track { title: get(t, "title", ""), artist, playing: t["playing"].as_bool().unwrap_or(false) }
        });
        self.mood.music_playing = show.now_playing.as_ref().is_some_and(|t| t.playing);
        // The TV light leans towards the next game's colour.
        let next = h.current_game().or_else(|| upcoming.first().and_then(|e| h.game_meta(&e.game).cloned()));
        self.mood.color = next.map_or(room::VIOLET, |g| crate::godot::Color::from_string(&get(&g, "color", ""), room::VIOLET));
        let ids: Vec<String> = cabinet.iter().map(|e| e.game.clone()).collect();
        if ids != self.last_queue { self.last_queue = ids; }
    }

    /// How far a prompt floats up now: a slow bob, or a hop while it nudges.
    pub(crate) fn prompt_bob(&self, name: &str, x: f32) -> f32 {
        // The doors hop lower than the TV: picking up is a gentler hint.
        if self.nudges.contains(name) { (self.time * 3.2).sin().abs() * if name == "tv" { 0.22 } else { 0.1 } } else { (self.time * 2.2 + x).sin() * 0.05 }
    }

    /// Room parts the lobby moves: prompts (shown, bobbing, glowing), the
    /// featured box and the shelf bumpers.
    fn pose_parts(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, delta: f32) -> Result<(), String> {
        let mut changed = vec![];
        for p in &self.build.prompts {
            let active = self.prompt_active.get(&p.name).copied().unwrap_or(false);
            let screen = self.build.screens.iter().find(|s| s.name == format!("prompt_{}", p.name));
            let stretch = screen.map_or(1.0, |s| PROMPT_WIDTH as f32 / s.size[0] as f32);
            let bob = Vec3::new(0.0, self.prompt_bob(&p.name, p.pos.x), 0.0);
            let at = Affine3A::from_scale_rotation_translation(Vec3::new(stretch, 1.0, 1.0), Quat::IDENTITY, p.pos + bob);
            self.stage.room.dump.instances[p.instance].transform = glam::Mat4::from(at).to_cols_array();
            changed.push((p.instance, active));
            let glow = self.prompt_glow.entry(p.name.clone()).or_insert(0.85);
            let target = if active { 1.6 } else { 0.85 };
            let next = if (*glow - target).abs() <= delta * 3.0 { target } else { *glow + (target - *glow).signum() * delta * 3.0 };
            if next != *glow {
                *glow = next;
                let m = &mut self.stage.room.dump.materials[p.material];
                m["albedo"] = serde_json::json!([next, next, next, if active { 1.0 } else { 0.8 }]);
                if let Some((id, material)) = self.stage.room.material_now(p.material) { renderer.update_materials(gpu, id as usize, &[material])?; }
            }
        }
        // Little hop of the featured box when browsing (0.35 s, back easing).
        let pop = self.pop_box.map(|(step, at)| (step, ((self.time - at) / 0.35).min(1.0)));
        if pop.is_some_and(|(_, t)| t >= 1.0) { self.pop_box = None; }
        let back = |t: f32| { let c1 = 1.70158; let c3 = c1 + 1.0; 1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2) };
        let (yaw, scale) = pop.map_or((0.0, 1.0), |(step, t)| { let k = back(t); (-0.6 * step * (1.0 - k), 0.9 + 0.1 * k) });
        let root = self.featured_root();
        for &k in &self.featured {
            let local = root.inverse() * Affine3A::from_mat4(glam::Mat4::from_cols_array(&self.build.dump.instances[k].transform));
            let at = root * Affine3A::from_scale_rotation_translation(Vec3::splat(scale), Quat::from_rotation_y(yaw), Vec3::ZERO) * local;
            self.stage.room.dump.instances[k].transform = glam::Mat4::from(at).to_cols_array();
            changed.push((k, self.featured_visible));
        }
        for &k in &self.bumpers { changed.push((k, self.bumpers_visible)); }
        for (k, visible) in changed {
            if let Some(i) = self.stage.room.sync_instance(&mut self.stage.scene, k, visible) {
                renderer.update_instances(gpu, i, &self.stage.scene.instances[i..i + 1])?;
            }
        }
        Ok(())
    }

    /// Where the featured box stands (room.rs `game_shelf`).
    fn featured_root(&self) -> Affine3A {
        let bx = ((game_room_sim::layout::BOOKCASE.0 + game_room_sim::layout::BOOKCASE.1) / 2.0) as f32;
        Affine3A::from_rotation_translation(Quat::from_rotation_x(-0.06), Vec3::new(bx, 3.56, game_room_sim::layout::BACK as f32 + 0.5))
    }

    /// Host messages and the lobby's per-frame logic, without drawing
    /// (`draw` calls it; tests drive it directly).
    pub fn update(&mut self, delta: f32) {
        self.poll(delta);
        self.process(delta);
    }

    /// Per-frame lobby work (main.gd `_process`).
    fn process(&mut self, delta: f32) {
        self.host.time = self.time;
        self.host.connected = self.client.as_ref().is_some_and(|c| c.connected);
        self.remove_lost(delta);
        self.update_sleep();
        self.update_holds();
        self.mood.pad_activity = (self.mood.pad_activity - delta).max(0.0);
        self.mood.pad_hop += delta;
        self.update_prompts();
        self.host.update_loading(delta);
        self.refresh_show();
        let mut animating = false;
        for p in self.prompts.values_mut() {
            if let Some((_, v)) = &mut p.pressed {
                *v = (*v - delta / 0.36).max(0.0);
                animating = true;
                if *v == 0.0 { p.pressed = None; }
            }
            if p.conceal != p.conceal_to {
                let step = delta / 0.45;
                p.conceal = if (p.conceal - p.conceal_to).abs() <= step { p.conceal_to } else { p.conceal + (p.conceal_to - p.conceal).signum() * step };
                animating = true;
            }
        }
        for v in &mut self.show.bumper_presses { if *v > 0.0 { *v = (*v - delta / 0.36).max(0.0); animating = true; } }
        if animating || !self.holds.is_empty() { self.screens_dirty = true; }
        let time = self.time;
        self.toasts.retain(|t| t.until > time);
        self.item_presses.retain(|_, p| time - p.2 < 0.2);
    }
}

impl Game for Lobby {
    fn local_players(&self) -> usize { self.local.len() }

    fn tick(&mut self, controls: &[Controls]) {
        for (id, c) in self.local.clone().iter().zip(controls) {
            let (held, stick) = buttons(c);
            self.input(id, held, stick);
        }
        self.bot_inputs();
        for p in &self.world.players { if let Some(a) = self.actors.get_mut(&p.id) { a.prev = [p.x, p.y]; } }
        self.props.prev = self.world.items.iter().map(|i| (i.id, [i.x, i.y])).collect();
        // The world only runs while the room is on screen.
        if !self.accepts_input() { return; }
        self.world.step();
        let events = self.world.take_events();
        self.events.extend(events);
        // Carried to the exit and thrown out: an away player leaves the party.
        for id in std::mem::take(&mut self.world.ejected) {
            let name = self.name(&id);
            self.toast(format!("{name} was thrown out of the party."), crate::lobby::DIM);
            self.command(serde_json::json!({"type": "leave_party", "player_id": id}));
        }
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        // Behind a game: a few frames a second, the GPU is the game's. Only
        // once the game's window is really in front (this one lost focus):
        // between "Play next" and the game showing, the room is still what
        // everyone looks at and must not stutter.
        if self.display_active == Some(false) && !frame.focused { std::thread::sleep(std::time::Duration::from_millis(100)); }
        let delta = frame.dt;
        self.time += delta;
        // A fresh renderer (the browser host switched to a game and back)
        // has none of the room: upload it again.
        if !self.uploaded || frame.renderer().is_none() {
            self.sheets_dirty = true;
            frame.set_scene(&self.stage.scene, &self.stage.light);
            let mut screen = Screen::new(frame.gpu)?;
            self.stage.upload(frame.gpu, frame.renderer().unwrap(), &mut screen)?;
            self.screen = Some(screen);
            self.uploaded = true;
            self.since_paint = 1.0;
        }
        self.update(delta);
        let glows = self.place_actors(frame.alpha, delta);
        for e in std::mem::take(&mut self.events) { self.props.event(&e); }
        self.props.camera(&mut self.stage.room.camera, self.time, delta);
        let light = self.stage.options.tuning.light;
        let depths: HashMap<String, f32> = self.actors.keys().map(|id| (id.clone(), self.actor_depth(id))).collect();
        let actor_depth = |id: &str| depths.get(id).copied().unwrap_or(0.0);
        let (mut lights, shots) = self.props.update(&self.world, &mut self.stage.room, &mut self.stage.scene, frame.alpha, delta, self.time, light, &actor_depth);
        let pending = self.host.pending_profiles();
        let shelf: Vec<(String, String)> = self.show.shelf.iter().map(|g| (g.title.clone(), g.color.clone())).collect();
        self.board.update(&mut self.stage.room, &mut self.stage.scene, &pending, &shelf, self.host.shelf_offset, light);
        lights.extend(self.board.lights().iter().copied());
        lights.extend(glows);
        lights.extend(shots);
        let (gpu, renderer) = (frame.gpu, frame.renderer().ok_or("no renderer")?);
        self.animate(gpu, renderer, delta)?;
        self.actors_view.slots.flush(gpu, renderer, &self.stage.scene)?;
        self.actors_view.bodies.flush(gpu, renderer, &self.stage.scene)?;
        self.props.flush(gpu, renderer, &self.stage.room, &mut self.stage.scene)?;
        self.pose_parts(gpu, renderer, delta)?;
        let screen = self.screen.as_mut().unwrap();
        self.board.flush(gpu, renderer, &self.stage.room, &self.stage.scene, screen)?;
        self.stage.relight_with(renderer, &lights)?;
        if std::mem::take(&mut self.sheets_dirty) { self.paint_sheets(gpu, renderer)?; }
        // The idle TV cycles its colours: repaint the screens ten times a
        // second, and at once when something on them changed.
        self.since_paint += delta;
        if self.since_paint >= 0.1 || (self.screens_dirty && self.since_paint >= 1.0 / 30.0) {
            self.since_paint = 0.0;
            self.screens_dirty = false;
            self.show.time = self.time;
            let screen = self.screen.as_mut().unwrap();
            self.stage.paint(gpu, renderer, screen, &self.show, &self.prompts, None)?;
        }
        self.hud_size = frame.size;
        let hud = crate::hud::draw(self, frame.size, frame.alpha);
        Ok(Draw { cameras: vec![self.stage.room.camera], light: self.stage.light.clone(), hud })
    }

    fn pointer(&mut self, pointer: app::Pointer, size: [u32; 2]) { self.mouse_input(pointer, size); }

    /// Escape is the mouse's Start: the room never closes on it.
    /// The game handing the screen back keeps rendering for a moment
    /// (about 1.6 s before it hides): those frames are not the room's cost.
    fn hold_quality(&self) -> bool { self.display_active == Some(false) || self.display_at.elapsed().as_secs_f32() < 3.0 }

    fn escape(&mut self) -> bool {
        // Escape belongs to the game in front; the one that closed it may
        // still reach the room as it comes back, so ignore it for a moment.
        let ours = self.display_active != Some(false) && self.display_at.elapsed().as_secs_f32() > 0.75;
        if ours { self.mouse_escape(); }
        true
    }

    fn title(&self) -> Option<String> { Some("Splitscreen Club Game Room".into()) }

    fn screen(&self) -> app::Screen {
        if self.client.is_none() { return app::Screen::Normal; }
        match self.display_active {
            Some(true) if std::env::var("GAMENIGHT_LOBBY_FULLSCREEN").as_deref() != Ok("0") => app::Screen::Fullscreen,
            // A game has the screen: stay fullscreen behind it, never
            // minimised, so Back reveals the room and not the desktop, the
            // taskbar or another app while the lobby climbs back up.
            Some(false) => app::Screen::Normal,
            _ => app::Screen::Normal,
        }
    }

    fn quit(&self) -> bool { self.quit }
}
