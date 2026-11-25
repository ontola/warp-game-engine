//! Online couches for Luau games (engine/docs/online.md).
//!
//! One host runs the game. Another couch (a follower) joins with its own
//! people, screen and pads: its people sit in the host's roster like local
//! people, its pads come over the network, and it gets the world back.
//! A follower never runs the game's Luau: it mirrors the host's entities,
//! plays the sounds the host played and shows the HUD and cameras the host
//! drew for the follower's own screen. So every Luau game plays online
//! without netcode of its own.
//!
//! This file holds the wire format and the diff; the transport is the `net`
//! crate (iroh, QUIC) on native builds.
use crate::api::{Cam, Core, HudItem};
use crate::world::{Entity, Look, Shape, Spawn, Wire};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The network protocol name. Bump it when the wire format changes.
pub const ALPN: &[u8] = b"warp-couch/1";

/// Frames a second the host sends; a follower ticks at this rate and
/// interpolates between them.
pub const SEND_HZ: f32 = 30.0;

/// Host ticks between two frames.
pub fn send_every(tick_rate: f32) -> u64 { (tick_rate / SEND_HZ).round().max(1.0) as u64 }

/// A pad's state for one tick (`app::Controls` on the wire).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pad { pub mv: [f32; 2], pub look: [f32; 2], pub aim: [f32; 2], pub triggers: [f32; 2], pub held: u32, pub pressed: u32 }

impl From<&app::Controls> for Pad {
    fn from(c: &app::Controls) -> Self { Pad { mv: c.mv, look: c.look, aim: c.aim, triggers: c.triggers, held: c.held.0, pressed: c.pressed.0 } }
}

impl Pad {
    /// Start belongs to the host's couch: a remote press never opens the host's menu.
    pub fn controls(&self) -> app::Controls {
        let strip = !app::Buttons::START;
        app::Controls { mv: self.mv, look: self.look, aim: self.aim, triggers: self.triggers, held: app::Buttons(self.held & strip), pressed: app::Buttons(self.pressed & strip) }
    }
}

/// Follower to host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Up {
    /// First message: how many people sit on this couch, its screen in pixels
    /// and the game it loaded (the folder name) and its protocol.
    Hello { people: u8, screen: [u32; 2], game: String },
    /// Every tick: this couch's pads, one per person.
    Pads(Vec<Pad>),
    /// The window changed size.
    Screen([u32; 2]),
    /// Round-trip measurement: echoed back as `Down::Pong`.
    Ping(u64),
    /// For the page around the game (the Game Room's online couches, JSON):
    /// passed to [`Shared`] and never to a game.
    Lobby(String),
}

/// Host to follower.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Down {
    /// The follower's people got these player ids in the host's roster.
    Welcome { players: Vec<u32>, tick_rate: f32 },
    /// What changed since the last frame this follower got.
    Frame(Frame),
    Pong(u64),
    /// The host refused or ended this couch.
    Bye(String),
    /// For the page around the game, as [`Up::Lobby`].
    Lobby(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Frame {
    pub tick: u64,
    /// New entities, or entities whose shape, parent, visibility or tag changed.
    pub full: Vec<(u32, Head, Pose)>,
    /// Entities that only moved, turned or resized.
    pub poses: Vec<(u32, Pose)>,
    /// Entities whose look changed (and maybe their pose: see `poses`).
    pub looks: Vec<(u32, WLook)>,
    /// Wires whose points, edges or width changed.
    pub wires: Vec<(u32, WWire)>,
    pub gone: Vec<u32>,
    /// One camera per follower person, in the follower's order.
    pub cams: Vec<WCam>,
    /// The HUD the game drew for the follower's screen, as changes to the last one.
    pub hud: Option<HudPatch>,
    pub sounds: Vec<SoundEvent>,
    /// The sky's name when it changed.
    pub sky: Option<String>,
}

/// Where an entity is (millimetres), how it is turned (1/10000 radian) and
/// its size (millimetres): whole numbers, small on the wire (varints).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose { pub pos: [i32; 3], pub rot: [i32; 3], pub size: [i32; 3] }

/// Millimetres and ten-thousandths of a radian.
const MM: f32 = 1000.0;
const ANGLE: f32 = 10000.0;
fn q(v: Vec3, k: f32) -> [i32; 3] { (v * k).round().to_array().map(|x| x.clamp(i32::MIN as f32, i32::MAX as f32) as i32) }
fn dq(v: [i32; 3], k: f32) -> Vec3 { Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32) / k }

impl Pose {
    pub fn pos(&self) -> Vec3 { dq(self.pos, MM) }
    pub fn rot(&self) -> Vec3 { dq(self.rot, ANGLE) }
    pub fn size(&self) -> Vec3 { dq(self.size, MM) }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Head {
    pub parent: Option<u32>,
    pub shape: WShape,
    pub look: WLook,
    pub visible: bool,
    pub shadow_only: bool,
    pub hide_views: u8,
    pub tag: String,
    pub wire: Option<WWire>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WShape { Cube, Pane, Sphere, Capsule, Outline { sides: u32, width: u32 }, Model(String), Wire, /// A generated mesh: not replicated yet, shown as nothing.
    Unsupported }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WLook { pub color: [f32; 3], pub emissive: f32, pub roughness: f32, pub metal: f32, pub surface: Option<u8>, pub glass: Option<f32>, pub coverage: Option<u8> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WWire { pub points: Vec<[i32; 3]>, pub edges: Vec<[u16; 2]>, pub width: f32 }

impl WWire {
    fn of(w: &Wire) -> Self { WWire { points: w.points.iter().map(|p| q(*p, MM)).collect(), edges: w.edges.clone(), width: w.width } }
    fn wire(&self) -> Wire { Wire { points: self.points.iter().map(|p| dq(*p, MM)).collect(), edges: self.edges.clone(), width: self.width } }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WCam { pub pos: [f32; 3], pub target: [f32; 3], pub fov: f32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WHud {
    Text { text: String, pos: [f32; 2], size: f32, color: [f32; 4], align: u8, width: f32 },
    Box { rect: [f32; 4], color: [f32; 4] },
    Circle { center: [f32; 2], radius: f32, color: [f32; 4] },
}

/// The HUD as changes: its new length and the items that differ by position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HudPatch { pub len: u32, pub items: Vec<(u32, WHud)> }

impl HudPatch {
    pub fn apply(&self, hud: &mut Vec<WHud>) {
        hud.truncate(self.len as usize);
        for (i, item) in &self.items {
            let i = *i as usize;
            if i < hud.len() { hud[i] = item.clone(); } else if i == hud.len() { hud.push(item.clone()); }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundEvent { pub name: String, pub params: Vec<(String, f32)>, pub at: Option<[f32; 3]>, pub db: f32, pub pitch: f32 }

fn shape_out(s: &Shape) -> WShape {
    match s {
        Shape::Cube => WShape::Cube, Shape::Pane => WShape::Pane, Shape::Sphere => WShape::Sphere, Shape::Capsule => WShape::Capsule,
        Shape::Outline { sides, width } => WShape::Outline { sides: *sides, width: *width }, Shape::Model(p) => WShape::Model(p.clone()),
        Shape::Wire => WShape::Wire, Shape::Generated(_) => WShape::Unsupported,
    }
}

fn shape_in(s: &WShape) -> Shape {
    match s {
        WShape::Cube | WShape::Unsupported => Shape::Cube, WShape::Pane => Shape::Pane, WShape::Sphere => Shape::Sphere, WShape::Capsule => Shape::Capsule,
        WShape::Outline { sides, width } => Shape::Outline { sides: *sides, width: *width }, WShape::Model(p) => Shape::Model(p.clone()), WShape::Wire => Shape::Wire,
    }
}

fn look_out(l: &Look) -> WLook { WLook { color: l.color, emissive: l.emissive, roughness: l.roughness, metal: l.metal, surface: l.surface, glass: l.glass, coverage: l.coverage } }
fn look_in(l: &WLook) -> Look { Look { color: l.color, emissive: l.emissive, roughness: l.roughness, metal: l.metal, surface: l.surface, glass: l.glass, coverage: l.coverage } }

fn head(e: &Entity) -> Head {
    Head {
        parent: e.parent(), shape: shape_out(&e.shape), look: look_out(&e.look),
        visible: e.visible && !matches!(e.shape, Shape::Generated(_)), shadow_only: e.shadow_only, hide_views: e.hide_views, tag: e.tag.clone(),
        wire: e.wire.as_ref().map(WWire::of),
    }
}

fn pose(e: &Entity) -> Pose { Pose { pos: q(e.pos, MM), rot: q(e.rot, ANGLE), size: q(e.size, MM) } }

/// Whether a follower's entity shows what the host's does, to the wire's precision.
pub fn same(a: &Entity, b: &Entity) -> bool { pose(a) == pose(b) && head(a) == head(b) }

pub fn cam_out(c: &Cam) -> WCam { WCam { pos: c.pos.to_array(), target: c.target.to_array(), fov: c.fov } }
pub fn cam_in(c: &WCam) -> Cam { Cam { pos: Vec3::from_array(c.pos), target: Vec3::from_array(c.target), fov: c.fov } }

pub fn hud_out(h: &HudItem) -> WHud {
    use render::hud::TextAlign;
    match h {
        HudItem::Text { text, pos, size, color, align, width } => WHud::Text { text: text.clone(), pos: *pos, size: *size, color: *color, align: match align { TextAlign::Left => 0, TextAlign::Center => 1, TextAlign::Right => 2 }, width: *width },
        HudItem::Box { rect, color } => WHud::Box { rect: *rect, color: *color },
        HudItem::Circle { center, radius, color } => WHud::Circle { center: *center, radius: *radius, color: *color },
    }
}

pub fn hud_in(h: &WHud) -> HudItem {
    use render::hud::TextAlign;
    match h {
        WHud::Text { text, pos, size, color, align, width } => HudItem::Text { text: text.clone(), pos: *pos, size: *size, color: *color, align: match align { 1 => TextAlign::Center, 2 => TextAlign::Right, _ => TextAlign::Left }, width: *width },
        WHud::Box { rect, color } => HudItem::Box { rect: *rect, color: *color },
        WHud::Circle { center, radius, color } => HudItem::Circle { center: *center, radius: *radius, color: *color },
    }
}

/// Whether a HUD item lies anywhere on a screen of this size (items drawn for
/// people on another couch land far outside it and are dropped).
pub fn on_screen(h: &HudItem, screen: [u32; 2]) -> bool {
    let (w, hgt) = (screen[0] as f32, screen[1] as f32);
    let inside = |x: f32, y: f32, ww: f32, hh: f32| x + ww >= 0.0 && y + hh >= 0.0 && x <= w && y <= hgt;
    match h {
        HudItem::Text { pos, size, width, .. } => inside(pos[0] - width.max(*size * 8.0), pos[1], width.max(*size * 8.0) * 2.0, *size * 2.0),
        HudItem::Box { rect, .. } => inside(rect[0], rect[1], rect[2], rect[3]),
        HudItem::Circle { center, radius, .. } => inside(center[0] - radius, center[1] - radius, radius * 2.0, radius * 2.0),
    }
}

/// A view rectangle far off any screen: the HUD a game draws for a person on
/// another couch goes there.
pub const OFFSCREEN: [f32; 4] = [-1.0e6, -1.0e6, 1280.0, 720.0];

/// What one follower already has: the entities it was sent and the sky.
#[derive(Default)]
pub struct Baseline { ents: BTreeMap<u32, (Head, Pose)>, sky: Option<String>, hud: Option<Vec<WHud>> }

impl Baseline {
    /// The changes from what this follower has to the host's world now. A new
    /// follower starts from nothing, so its first frame is the whole world.
    pub fn diff(&mut self, c: &Core, tick: u64) -> Frame {
        let mut f = Frame { tick, ..Default::default() };
        for (&id, e) in &c.world.entities {
            let (h, p) = (head(e), pose(e));
            match self.ents.get_mut(&id) {
                Some((oh, op)) => {
                    let (look, wire) = (oh.look != h.look, oh.wire != h.wire);
                    let rest = h.parent != oh.parent || h.shape != oh.shape || h.visible != oh.visible || h.shadow_only != oh.shadow_only
                        || h.hide_views != oh.hide_views || h.tag != oh.tag || h.wire.is_some() != oh.wire.is_some();
                    if rest { f.full.push((id, h.clone(), p)); }
                    else {
                        if look { f.looks.push((id, h.look)); }
                        if wire && let Some(w) = &h.wire { f.wires.push((id, w.clone())); }
                        if *op != p { f.poses.push((id, p)); }
                    }
                    *oh = h; *op = p;
                }
                None => { f.full.push((id, h.clone(), p)); self.ents.insert(id, (h, p)); }
            }
        }
        let gone: Vec<u32> = self.ents.keys().copied().filter(|id| !c.world.entities.contains_key(id)).collect();
        for id in &gone { self.ents.remove(id); }
        f.gone = gone;
        self.finish(f, c)
    }

    /// The HUD, when it differs from what this follower shows.
    pub fn hud(&mut self, hud: Vec<WHud>) -> Option<HudPatch> {
        let old = self.hud.get_or_insert_with(Vec::new);
        if *old == hud { return None; }
        let items = hud.iter().enumerate().filter(|(i, h)| old.get(*i) != Some(*h)).map(|(i, h)| (i as u32, h.clone())).collect();
        let patch = HudPatch { len: hud.len() as u32, items };
        *old = hud;
        Some(patch)
    }

    fn finish(&mut self, mut f: Frame, c: &Core) -> Frame {
        let sky = c.sky.name();
        if self.sky.as_deref() != Some(sky.as_str()) { f.sky = Some(sky.clone()); self.sky = Some(sky); }
        f
    }
}

/// Make the follower's world what the host's frame says.
pub fn apply(c: &mut Core, f: &Frame) -> Vec<String> {
    let mut errors = Vec::new();
    for id in &f.gone { c.world.remove(*id); }
    let mut parents = Vec::new();
    for (id, h, p) in &f.full {
        let spawn = Spawn {
            parent: None, shape: shape_in(&h.shape), look: look_in(&h.look), pos: p.pos(), rot: p.rot(), size: p.size(),
            solid: false, visible: h.visible && h.shape != WShape::Unsupported, shadow_only: h.shadow_only, hide_views: h.hide_views, tag: h.tag.clone(),
            wire: h.wire.as_ref().map(WWire::wire),
        };
        let r = if c.world.entities.contains_key(id) {
            c.world.update(*id, |e| {
                e.shape = spawn.shape; e.look = spawn.look; e.pos = spawn.pos; e.rot = spawn.rot; e.size = spawn.size;
                e.visible = spawn.visible; e.shadow_only = spawn.shadow_only; e.hide_views = spawn.hide_views; e.tag = spawn.tag; e.wire = spawn.wire;
            })
        } else {
            c.world.spawn_as(*id, spawn)
        };
        if let Err(e) = r { errors.push(format!("entity {id}: {e}")); }
        parents.push((*id, h.parent));
    }
    for (id, parent) in parents {
        if c.world.get(id).is_some_and(|e| e.parent() != parent)
            && let Err(e) = c.world.set_parent(id, parent.filter(|p| c.world.get(*p).is_some())) { errors.push(format!("entity {id}: {e}")); }
    }
    for (id, l) in &f.looks {
        if let Err(e) = c.world.update(*id, |e| e.look = look_in(l)) { errors.push(e); }
    }
    for (id, w) in &f.wires {
        if let Err(e) = c.world.update(*id, |e| e.wire = Some(w.wire())) { errors.push(e); }
    }
    for (id, p) in &f.poses {
        if let Err(e) = c.world.set_pose(*id, p.pos(), p.rot(), p.size()) { errors.push(e); }
    }
    if let Some(s) = &f.sky && let Ok(choice) = crate::sky::SkyChoice::parse(s) { c.sky = choice; }
    errors
}

/// Bytes a message takes on the wire (postcard), for the bandwidth line.
pub fn wire_size<T: Serialize>(m: &T) -> usize { postcard::to_allocvec(m).map_or(0, |v| v.len()) }

/// Sent bytes over a sliding few seconds, as kbit/s.
#[derive(Default)]
pub struct Meter { bytes: u64, frames: u64, since: Option<web_time::Instant> }

impl Meter {
    pub fn add(&mut self, bytes: usize) { self.bytes += bytes as u64; self.frames += 1; self.since.get_or_insert_with(web_time::Instant::now); }
    /// Some((kbit/s, bytes per frame)) once every `every` seconds.
    pub fn report(&mut self, every: f32) -> Option<(f64, f64)> {
        let t = self.since?.elapsed().as_secs_f64();
        if t < every as f64 || self.frames == 0 { return None; }
        let r = (self.bytes as f64 * 8.0 / 1000.0 / t, self.bytes as f64 / self.frames as f64);
        *self = Meter::default();
        Some(r)
    }
}

/// What a host's transport reports.
pub enum LinkEvent { Joined(u64), Message(u64, Up), Left(u64) }

/// A host's side of the connections to follower couches: iroh on native
/// builds ([`iroh_link`]), WebRTC data channels in a browser (`online_web`).
pub trait HostLink {
    fn poll(&mut self) -> Option<LinkEvent>;
    /// False when that couch can no longer be reached.
    fn send(&mut self, client: u64, msg: Down) -> bool;
    /// What another couch joins with.
    fn ticket(&self) -> String;
}

pub enum Got { Msg(Down), Empty, Closed }

/// A follower's connection to its host.
pub trait FollowLink {
    fn send(&mut self, msg: Up);
    fn poll(&mut self) -> Got;
}

/// One connection for a whole page: the Game Room talks over it
/// ([`Up::Lobby`], [`Down::Lobby`]) and hands each online game a
/// [`Shared::host_side`] or [`Shared::follow_side`], so the couches stay
/// connected between games. Lobby messages never reach a game, and game
/// messages that arrive while no game holds a side are dropped.
pub struct Shared<L: ?Sized>(std::rc::Rc<std::cell::RefCell<SharedState<L>>>);

pub struct SharedState<L: ?Sized> {
    /// The couches connected (host side), in the order they came.
    pub clients: Vec<u64>,
    /// Couches that left since the page last asked (host side).
    pub gone: Vec<u64>,
    pub lobby: std::collections::VecDeque<(u64, String)>,
    game: std::collections::VecDeque<SharedMsg>,
    in_game: bool,
    closed: bool,
    link: Box<L>,
}

enum SharedMsg { Host(LinkEvent), Follow(Down) }

impl<L: ?Sized> Clone for Shared<L> { fn clone(&self) -> Self { Shared(self.0.clone()) } }

impl<L: ?Sized> Shared<L> {
    pub fn new(link: Box<L>) -> Self {
        Shared(std::rc::Rc::new(std::cell::RefCell::new(SharedState { clients: Vec::new(), gone: Vec::new(), lobby: Default::default(), game: Default::default(), in_game: false, closed: false, link })))
    }
    pub fn state(&self) -> std::cell::RefMut<'_, SharedState<L>> { self.0.borrow_mut() }
    /// Whether the game holding a side is on screen (false: paused or gone;
    /// its messages are dropped meanwhile).
    pub fn set_in_game(&self, on: bool) { let mut s = self.0.borrow_mut(); s.in_game = on; s.game.clear(); }
}

impl Shared<dyn HostLink> {
    fn pump(s: &mut SharedState<dyn HostLink>) {
        while let Some(ev) = s.link.poll() {
            match ev {
                LinkEvent::Joined(c) => { if !s.clients.contains(&c) { s.clients.push(c); } if s.in_game { s.game.push_back(SharedMsg::Host(ev)); } }
                LinkEvent::Left(c) => { s.clients.retain(|x| *x != c); s.gone.push(c); if s.in_game { s.game.push_back(SharedMsg::Host(ev)); } }
                LinkEvent::Message(c, Up::Lobby(text)) => s.lobby.push_back((c, text)),
                other => if s.in_game { s.game.push_back(SharedMsg::Host(other)); },
            }
        }
    }
    /// The page's messages from couches, and who joined and left.
    pub fn poll_lobby(&self) -> Vec<(u64, String)> { let mut s = self.0.borrow_mut(); Self::pump(&mut s); s.lobby.drain(..).collect() }
    pub fn send(&self, client: u64, msg: Down) -> bool { self.0.borrow_mut().link.send(client, msg) }
    pub fn ticket(&self) -> String { self.0.borrow().link.ticket() }
    /// The link for a game this page hosts.
    pub fn host_side(&self) -> Box<dyn HostLink> { { let mut s = self.0.borrow_mut(); s.in_game = true; s.game.clear(); } Box::new(self.clone()) }
}

impl HostLink for Shared<dyn HostLink> {
    fn poll(&mut self) -> Option<LinkEvent> {
        let mut s = self.0.borrow_mut();
        Self::pump(&mut s);
        loop {
            match s.game.pop_front()? { SharedMsg::Host(ev) => return Some(ev), SharedMsg::Follow(_) => {} }
        }
    }
    fn send(&mut self, client: u64, msg: Down) -> bool { self.0.borrow_mut().link.send(client, msg) }
    fn ticket(&self) -> String { self.0.borrow().link.ticket() }
}

impl Shared<dyn FollowLink> {
    fn pump(s: &mut SharedState<dyn FollowLink>) {
        loop {
            match s.link.poll() {
                Got::Msg(Down::Lobby(text)) => s.lobby.push_back((0, text)),
                Got::Msg(m) => if s.in_game { s.game.push_back(SharedMsg::Follow(m)); },
                Got::Empty => break,
                Got::Closed => { s.closed = true; break; }
            }
        }
    }
    pub fn poll_lobby(&self) -> Vec<(u64, String)> { let mut s = self.0.borrow_mut(); Self::pump(&mut s); s.lobby.drain(..).collect() }
    pub fn send(&self, msg: Up) { self.0.borrow_mut().link.send(msg) }
    /// The host is gone (or never answered).
    pub fn closed(&self) -> bool { self.0.borrow().closed }
    /// The link for a game this page follows.
    pub fn follow_side(&self) -> Box<dyn FollowLink> { { let mut s = self.0.borrow_mut(); s.in_game = true; s.game.clear(); } Box::new(self.clone()) }
}

impl FollowLink for Shared<dyn FollowLink> {
    fn send(&mut self, msg: Up) { self.0.borrow_mut().link.send(msg) }
    fn poll(&mut self) -> Got {
        let mut s = self.0.borrow_mut();
        Self::pump(&mut s);
        loop {
            match s.game.pop_front() {
                Some(SharedMsg::Follow(m)) => return Got::Msg(m),
                Some(SharedMsg::Host(_)) => {}
                None => return if s.closed { Got::Closed } else { Got::Empty },
            }
        }
    }
}

/// A follower couch, as the host sees it.
pub struct Remote {
    /// Its people's player ids in the host's roster, in its order.
    pub players: Vec<u32>,
    pub screen: [u32; 2],
    /// The newest pads, and the presses since the last host tick.
    pub pads: Vec<Pad>,
    pub pressed: Vec<u32>,
    pub base: Baseline,
    pub meter: Meter,
    /// Bytes sent to it so far, and by part: full entities, poses, HUD, the rest.
    pub sent: u64,
    pub parts: [u64; 4],
}

pub struct OnlineHost {
    pub link: Box<dyn HostLink>,
    pub remotes: BTreeMap<u64, Remote>,
}

impl OnlineHost {
    pub fn new(link: Box<dyn HostLink>) -> Self { OnlineHost { link, remotes: BTreeMap::new() } }
    pub fn ticket(&self) -> String { self.link.ticket() }
}

pub struct Follower {
    pub link: Box<dyn FollowLink>,
    pub players: Vec<u32>,
    pub hud: Vec<HudItem>,
    /// The HUD as sent, which patches apply to.
    pub wire_hud: Vec<WHud>,
    pub screen: [u32; 2],
    pub meter: Meter,
    pub rtt_ms: Option<f64>,
    pub last_ping: Option<web_time::Instant>,
    pub started: web_time::Instant,
    pub host_tick: u64,
    pub frames: u64,
    pub closed: Option<String>,
    /// Frames received and not shown yet: one is shown per tick, so a
    /// late packet does not stall the picture (a small jitter buffer).
    pub queue: std::collections::VecDeque<Frame>,
    /// The hello, sent again until the host welcomes this couch (a browser
    /// link opens some time after it is made).
    pub hello: Up,
}

impl Follower {
    pub fn new(mut link: Box<dyn FollowLink>, people: u8, screen: [u32; 2], game: &str) -> Self {
        let hello = Up::Hello { people, screen, game: game.into() };
        link.send(hello.clone());
        Follower { link, players: Vec::new(), hud: Vec::new(), wire_hud: Vec::new(), screen, meter: Meter::default(), rtt_ms: None, last_ping: None, started: web_time::Instant::now(), host_tick: 0, frames: 0, closed: None, queue: Default::default(), hello }
    }
    pub fn now_us(&self) -> u64 { self.started.elapsed().as_micros() as u64 }
}

/// The native transport: the `net` crate (iroh, QUIC with hole punching).
#[cfg(not(target_arch = "wasm32"))]
pub mod iroh_link {
    use super::*;

    pub struct Host { net: net::Host<Up, Down>, txs: BTreeMap<u64, net::HostSender<Down>> }

    impl Host {
        pub fn start(lan: bool) -> Result<Self, String> {
            // Frames are deltas: never collapse them.
            Ok(Host { net: net::Host::start(ALPN, lan, |_| false)?, txs: BTreeMap::new() })
        }
    }

    impl HostLink for Host {
        fn poll(&mut self) -> Option<LinkEvent> {
            Some(match self.net.poll()? {
                net::HostEvent::Joined { client, tx } => { self.txs.insert(client, tx); LinkEvent::Joined(client) }
                net::HostEvent::Message { client, msg } => LinkEvent::Message(client, msg),
                net::HostEvent::Left { client } => { self.txs.remove(&client); LinkEvent::Left(client) }
            })
        }
        fn send(&mut self, client: u64, msg: Down) -> bool { self.txs.get(&client).is_some_and(|tx| tx.send(msg).is_ok()) }
        fn ticket(&self) -> String { self.net.ticket.clone() }
    }

    pub struct Client(net::Client<Up, Down>);

    impl Client {
        pub fn join(ticket: &str, lan: bool) -> Result<Self, String> { Ok(Client(net::Client::join(ticket, ALPN, lan)?)) }
    }

    impl FollowLink for Client {
        fn send(&mut self, msg: Up) { self.0.send(msg); }
        fn poll(&mut self) -> Got {
            match self.0.poll() { Ok(m) => Got::Msg(m), Err(net::TryRecvError::Empty) => Got::Empty, Err(_) => Got::Closed }
        }
    }
}
