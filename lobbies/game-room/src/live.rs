//! The live lobby: the room built in Rust, its animation (neon, moving
//! heads, disco ball, EQ, pads), the platformer simulation and everyone in
//! it as pixel-art sprites (view.gd), on the app shell.

use crate::character::{self, Look, FRAMES};
use crate::dump::{linear, Room};
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
use std::collections::HashMap;
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

/// A profile as the lobby shows it.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub color: String,
    pub skin: String,
}

/// demo.gd's bots: wander to random spots, hop now and then, sometimes
/// punch or fire.
struct Bot { target: f64, next: f64, jump: f64 }

pub struct Lobby {
    build: RoomBuild,
    stage: Stage,
    actors_view: ActorMeshes,
    pub world: World,
    pub profiles: HashMap<String, Profile>,
    actors: HashMap<String, Actor>,
    next_order: usize,
    sheets_dirty: bool,
    screen: Option<Screen>,
    pub show: Show,
    pub prompts: HashMap<String, Prompt>,
    pub mood: Mood,
    /// Dump instances and materials RoomBuild::pose changes.
    animated: (Vec<usize>, Vec<usize>),
    time: f32,
    since_paint: f32,
    local: Vec<String>,
    bots: HashMap<String, Bot>,
    rng: Pcg,
    uploaded: bool,
    props: Props,
    events: Vec<world::Event>,
}

const NAMES: [&str; 6] = ["Joep", "Mango", "Disco", "Waffle", "Pickle", "Rocket"];
/// The host's guest colours and skins (demo.gd).
const COLORS: [&str; 6] = ["#e36c76", "#49b8bc", "#d7ad43", "#a67bdd", "#4daf7c", "#cf7aaf"];
const SKINS: [&str; 6] = ["#f5e9be", "#d99b72", "#efc39a", "#905c40", "#bf855b", "#654332"];

impl Lobby {
    /// The room with `local` keyboard/pad players and `bots` demo bots.
    pub fn new(options: Options, local: usize, bots: usize) -> Result<Self, String> {
        let mut build = room::build();
        let kit = crate::kit::add(&mut build);
        let animated = build.animated();
        let mut room = Room::from_build(build.clone());
        room.live = true;
        let (mut actors_view, mut props) = (None, None);
        let stage = Stage::new(room, options, |scene, room| {
            actors_view = Some(ActorMeshes::reserve(scene));
            props = Some(Props::reserve(kit, scene, room));
        })?;
        let mut lobby = Self {
            build, stage, actors_view: actors_view.expect("actor meshes"), world: World::new(7), profiles: HashMap::new(), actors: HashMap::new(),
            next_order: 0, sheets_dirty: true, screen: None, show: Show::default(), prompts: HashMap::new(), mood: Mood::default(),
            animated, time: 0.0, since_paint: 0.0, local: vec![], bots: HashMap::new(), rng: Pcg::new(42), uploaded: false,
            props: props.expect("props"), events: vec![],
        };
        lobby.prompts = lobby.build.prompts.iter().map(|p| (p.name.clone(), Prompt {
            items: p.items.iter().map(|(k, t)| (k.clone(), t.clone(), true)).collect(), enabled: true, ..Default::default() })).collect();
        for i in 0..(local + bots).min(MAX_ACTORS) {
            let id = format!("p{i}");
            lobby.profiles.insert(id.clone(), Profile { color: COLORS[i % 6].into(), skin: SKINS[i % 6].into() });
            lobby.world.ensure_player(&id, i);
            if i < local { lobby.local.push(id.clone()); } else {
                let x = lobby.world.player(&id).map_or(0.0, |p| p.x);
                let next = lobby.rng.randf_range_f32(0.5, 2.0) as f64;
                lobby.bots.insert(id.clone(), Bot { target: x, next, jump: 0.0 });
            }
        }
        lobby.show.humans = local + bots;
        lobby.show.profile_colors = (0..local + bots).map(|i| COLORS[i % 6].to_string()).collect();
        Ok(lobby)
    }

    pub fn names() -> &'static [&'static str] { &NAMES }

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
            self.world.set_input(&id, buttons, stick);
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
            let profile = self.profiles.get(id).cloned().unwrap_or_default();
            let key = format!("{}|{}|{}", profile.color, profile.skin, p.hat);
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
        let screen = self.screen.as_mut().unwrap();
        let mut list = HudDrawList::new([render::screen::SIZE; 2]);
        let side = render::screen::SIZE as f32;
        for (id, actor) in &self.actors {
            let p = self.world.player(id);
            let profile = self.profiles.get(id).cloned().unwrap_or_default();
            let mut sheet = character::build(&Look { color: &profile.color, skin: &profile.skin, hat: p.map_or("none", |p| p.hat.as_str()), ..Default::default() });
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
        screen.draw(gpu, renderer, self.actors_view.layer, [0.0; 4], &list)
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

impl Game for Lobby {
    fn local_players(&self) -> usize { self.local.len() }

    fn tick(&mut self, controls: &[Controls]) {
        for (id, c) in self.local.clone().iter().zip(controls) {
            let (held, stick) = buttons(c);
            self.world.set_input(id, held, stick);
        }
        self.bot_inputs();
        for p in &self.world.players { if let Some(a) = self.actors.get_mut(&p.id) { a.prev = [p.x, p.y]; } }
        self.props.prev = self.world.items.iter().map(|i| (i.id, [i.x, i.y])).collect();
        self.world.step();
        let events = self.world.take_events();
        self.events.extend(events);
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let delta = frame.dt;
        self.time += delta;
        if !self.uploaded {
            frame.set_scene(&self.stage.scene, &self.stage.light);
            let mut screen = Screen::new(frame.gpu)?;
            self.stage.upload(frame.gpu, frame.renderer().unwrap(), &mut screen)?;
            self.screen = Some(screen);
            self.uploaded = true;
            self.since_paint = 1.0;
        }
        let glows = self.place_actors(frame.alpha, delta);
        for e in std::mem::take(&mut self.events) { self.props.event(&e); }
        self.props.camera(&mut self.stage.room.camera, self.time, delta);
        let light = self.stage.options.tuning.light;
        let (mut lights, shots) = self.props.update(&self.world, &mut self.stage.room, &mut self.stage.scene, frame.alpha, delta, self.time, light);
        lights.extend(glows);
        lights.extend(shots);
        let (gpu, renderer) = (frame.gpu, frame.renderer().ok_or("no renderer")?);
        self.animate(gpu, renderer, delta)?;
        self.actors_view.slots.flush(gpu, renderer, &self.stage.scene)?;
        self.actors_view.bodies.flush(gpu, renderer, &self.stage.scene)?;
        self.props.flush(gpu, renderer, &self.stage.room, &mut self.stage.scene)?;
        self.stage.relight_with(renderer, &lights)?;
        if std::mem::take(&mut self.sheets_dirty) { self.paint_sheets(gpu, renderer)?; }
        // The idle TV cycles its colours: repaint the screens ten times a second.
        self.since_paint += delta;
        if self.since_paint >= 0.1 {
            self.since_paint = 0.0;
            self.show.time = self.time;
            let screen = self.screen.as_mut().unwrap();
            self.stage.paint(gpu, renderer, screen, &self.show, &self.prompts, None)?;
        }
        Ok(Draw { cameras: vec![self.stage.room.camera], light: self.stage.light.clone(), hud: HudDrawList::new(frame.size) })
    }

    fn title(&self) -> Option<String> { Some("GameNight Game Room".into()) }
}
