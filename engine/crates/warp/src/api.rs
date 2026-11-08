//! The Luau game API: the `world`, `fx`, `players`, `camera`, `sound`, `hud`,
//! `sky`, `light`, `random` and `game` tables every module sees. Documented for
//! game authors in `starter/types.d.luau` and engine/docs/warp-cli.md.
//!
//! The functions share one [`Core`] (the world, sounds, cameras, HUD and
//! random generator) behind a mutex. The game never holds the lock while
//! Luau runs, so a hook can call any function.
//!
//! Determinism: everything that changes the game (spawning, moving,
//! removing, random numbers, sounds) is refused inside `draw`, which runs a
//! variable number of times per tick (never, in a headless script, unless
//! a frame is captured). Cameras, the HUD, the sky and the light may be set
//! anywhere.
use crate::sky::SkyChoice;
use crate::sounds::SoundOut;
use crate::world::{Burst, Entity, Filter, Look, RingFx, Shape, Spawn, Wire, World};
use glam::Vec3;
use render::hud::TextAlign;
use script::mlua::{self, Lua, Table, Value, Vector};
use std::sync::{Arc, Mutex, MutexGuard};

/// A local player's camera. `fov` is vertical, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam { pub pos: Vec3, pub target: Vec3, pub fov: f32 }

impl Cam {
    /// `self` drawn `alpha` (0..1) of the way to `to`; a jump snaps.
    pub fn towards(&self, to: &Cam, alpha: f32) -> Cam {
        if self.pos.distance(to.pos) > 4.0 { return *to; }
        Cam { pos: self.pos.lerp(to.pos, alpha), target: self.target.lerp(to.target, alpha), fov: self.fov + (to.fov - self.fov) * alpha }
    }
}

impl Default for Cam {
    fn default() -> Self { Cam { pos: Vec3::new(0.0, 6.0, 12.0), target: Vec3::new(0.0, 1.0, 0.0), fov: 60.0 } }
}

#[derive(Clone, Debug)]
pub enum HudItem {
    Text { text: String, pos: [f32; 2], size: f32, color: [f32; 4], align: TextAlign, width: f32 },
    Box { rect: [f32; 4], color: [f32; 4] },
}

/// Colours for players without one (GameNight gives its own).
pub const PLAYER_COLORS: [[f32; 3]; 4] = [[0.91, 0.33, 0.25], [0.25, 0.56, 0.91], [0.95, 0.76, 0.19], [0.31, 0.75, 0.35]];

/// One player of the game: a person on a controller or a bot the scripts
/// drive. Ids are stable for the session and never reused, so a player
/// who leaves does not renumber the others.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub id: u32,
    pub name: String,
    /// sRGB 0..1.
    pub color: [f32; 3],
    pub bot: bool,
    /// The GameNight seat, when a party host seated this player.
    pub seat: Option<usize>,
    /// Couch position from the lobby (0 is leftmost), when known.
    pub couch: Option<i64>,
    pub cam: Cam,
    /// The camera at the start of the current tick: frames between ticks
    /// are drawn from here towards `cam` (smooth at 120 Hz on a 60 Hz tick).
    pub cam_prev: Cam,
    /// The split-screen view this player looks through (people only).
    pub view: Option<usize>,
}

impl Player {
    pub fn new(id: u32, name: String, color: Option<[f32; 3]>, bot: bool) -> Self {
        Player { id, name, color: color.unwrap_or(PLAYER_COLORS[(id as usize - 1) % 4]), bot, seat: None, couch: None, cam: Cam::default(), cam_prev: Cam::default(), view: None }
    }
}

/// What the scripts asked of the party host this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Requests {
    pub finish: bool,
    pub logs: Vec<String>,
    /// (player id, weak, strong, seconds)
    pub rumble: Vec<(u32, f64, f64, f64)>,
}

/// What `light.set` overrides on top of the sky's light.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightOverride {
    pub sun: Option<Vec3>,
    pub sun_lux: Option<f32>,
    pub sky_lux: Option<f32>,
    pub ev100: Option<f32>,
    /// `exposure = "auto"` and its tuning: the camera adapts like an eye.
    pub auto: Option<render::AutoExposure>,
    pub fog: [Option<f32>; 3],
}

/// The look after the scene is lit (`post.set`): bloom and the tone pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Post {
    /// Glow strength, 0 (the renderer's faint default) to 2.
    pub bloom: Option<f32>,
    /// Blur rounds, 1 (a tight halo) to 6 (a wide soft glow).
    pub bloom_radius: u32,
    /// Exposed brightness where the glow starts.
    pub bloom_threshold: f32,
    /// Wide (Godot-style screen glow, from the far blur rounds) or tight
    /// (an added halo hugging what glows: neon lines).
    pub bloom_wide: bool,
    pub contrast: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub grain: f32,
    pub aberration: f32,
}

impl Default for Post {
    fn default() -> Self { Post { bloom: None, bloom_radius: 4, bloom_threshold: 1.0, bloom_wide: true, contrast: 1.0, saturation: 1.0, vignette: 0.0, grain: 0.0, aberration: 0.0 } }
}

/// A coloured point light (`lights.set`): no shadows, it lights what is near.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp { pub pos: Vec3, pub color: [f32; 3], pub intensity: f32, pub range: f32 }

pub const MAX_LAMPS: usize = render::headlights::MAX_LOCAL_LIGHTS;

/// Which hook is running: `draw` may not change the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Sim, Draw }

/// A small seeded generator (SplitMix64): the same seed gives the same
/// numbers on every machine.
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn float(&mut self) -> f64 { (self.next() >> 11) as f64 / (1u64 << 53) as f64 }
}

pub struct Core {
    pub(crate) storage: Option<crate::storage::Storage>,
    pub world: World,
    /// Grown plants (`plants.grow`), attached to the scene on the next frame.
    pub plants: crate::plants::Plants,
    pub sounds: SoundOut,
    /// Every player, people first in view order, then bots.
    pub roster: Vec<Player>,
    /// The HUD of the frame being drawn (cleared before `draw`).
    pub hud: Vec<HudItem>,
    /// Output size and each player's view rectangle (x, y, w, h), in pixels.
    pub screen: [u32; 2],
    pub views: Vec<[f32; 4]>,
    pub rng: Rng,
    pub sky: SkyChoice,
    pub light: LightOverride,
    pub post: Post,
    pub lamps: Vec<Lamp>,
    pub tick: u64,
    pub tick_rate: f32,
    pub title: String,
    pub requests: Requests,
    /// One shared view (game.toml `split = false`): every camera call is the viewer's.
    pub shared_view: bool,
    pub phase: Phase,
}

pub type Shared = Arc<Mutex<Core>>;

pub fn lock(core: &Shared) -> MutexGuard<'_, Core> { core.lock().unwrap_or_else(|p| p.into_inner()) }

fn rt(msg: impl std::fmt::Display) -> mlua::Error { mlua::Error::runtime(msg) }

fn num(v: &Value) -> Option<f32> {
    match v { Value::Integer(i) => Some(*i as f32), Value::Number(x) => Some(*x as f32), _ => None }
}

/// A vector, `{x=, y=, z=}` or `{1, 2, 3}`.
pub fn vec3(v: &Value, what: &str) -> mlua::Result<Vec3> {
    match v {
        Value::Vector(v) => Ok(Vec3::new(v.x(), v.y(), v.z())),
        Value::Table(t) => {
            let get = |k: &str, i: i64| -> mlua::Result<f32> {
                let a: Value = t.raw_get(k)?;
                let a = if a.is_nil() { t.raw_get(i)? } else { a };
                num(&a).ok_or_else(|| rt(format!("{what}: needs numbers x, y and z")))
            };
            Ok(Vec3::new(get("x", 1)?, get("y", 2)?, get("z", 3)?))
        }
        _ => Err(rt(format!("{what}: expected a vector (vector.create(x, y, z)), got a {}", v.type_name()))),
    }
}

fn vector(v: Vec3) -> Value { Value::Vector(Vector::new(v.x, v.y, v.z)) }

/// A size: a number (every axis) or a vector.
fn size(v: &Value, what: &str) -> mlua::Result<Vec3> {
    match num(v) { Some(s) => Ok(Vec3::splat(s)), None => vec3(v, what) }
}

/// sRGB colour 0..1 with alpha: `"#rrggbb"`/`"#rrggbbaa"`, a vector (rgb),
/// `{r, g, b, a}` (named or listed) or a number (grey).
pub fn color(v: &Value, what: &str) -> mlua::Result<[f32; 4]> {
    let bad = || rt(format!("{what}: a colour is \"#rrggbb\", \"#rrggbbaa\", a vector or {{r, g, b, a}} with values 0 to 1"));
    match v {
        Value::String(s) => {
            let s = s.to_string_lossy();
            let hex = s.trim_start_matches('#');
            if !(hex.len() == 6 || hex.len() == 8) { return Err(bad()); }
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map(|b| b as f32 / 255.0).map_err(|_| bad());
            Ok([byte(0)?, byte(2)?, byte(4)?, if hex.len() == 8 { byte(6)? } else { 1.0 }])
        }
        Value::Vector(c) => Ok([c.x(), c.y(), c.z(), 1.0]),
        Value::Table(t) => {
            let get = |k: &str, i: i64, d: Option<f32>| -> mlua::Result<f32> {
                let a: Value = t.raw_get(k)?;
                let a = if a.is_nil() { t.raw_get(i)? } else { a };
                if a.is_nil() { return d.ok_or_else(bad); }
                num(&a).ok_or_else(bad)
            };
            Ok([get("r", 1, None)?, get("g", 2, None)?, get("b", 3, None)?, get("a", 4, Some(1.0))?])
        }
        _ => num(v).map(|g| [g, g, g, 1.0]).ok_or_else(bad),
    }
}

/// Options tables reject unknown keys, so a typo is an error, not a no-op.
fn options(t: &Table, allowed: &[&str], what: &str) -> mlua::Result<Vec<(String, Value)>> {
    let mut out = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let Value::String(k) = k else { return Err(rt(format!("{what}: option names are strings"))) };
        let k = k.to_string_lossy();
        if !allowed.is_empty() && !allowed.contains(&k.as_str()) { return Err(rt(format!("{what}: unknown option {k:?} (options: {})", allowed.join(", ")))); }
        out.push((k, v));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub fn shape(name: &str) -> mlua::Result<Shape> {
    Ok(match name {
        "cube" | "box" => Shape::Cube,
        "sphere" | "ball" => Shape::Sphere,
        "capsule" => Shape::Capsule,
        "wire" => Shape::Wire,
        p if p.starts_with("outline") => {
            let bad = || rt(format!("{p:?}: outline<sides>[:<width>], such as \"outline6\" or \"outline4:0.05\" (3 to 64 sides, width 0.005 to 0.5)"));
            let (sides, width) = p["outline".len()..].split_once(':').map_or((&p["outline".len()..], None), |(a, b)| (a, Some(b)));
            let sides: u32 = sides.parse().map_err(|_| bad())?;
            let width: f32 = match width { Some(w) => w.parse().map_err(|_| bad())?, None => 0.06 };
            if !(3..=64).contains(&sides) || !(0.005..=0.5).contains(&width) { return Err(bad()); }
            Shape::Outline { sides, width: (width * 1000.0).round() as u32 }
        }
        p if p.ends_with(".glb") || p.ends_with(".gltf") => Shape::Model(p.to_string()),
        other => return Err(rt(format!("unknown shape {other:?}: cube, sphere, capsule, outline6 or a model path such as \"assets/crate.glb\""))),
    })
}

const ENTITY_KEYS: &[&str] = &["shape", "pos", "rot", "size", "color", "emissive", "roughness", "metal", "surface", "solid", "visible", "tag", "points", "edges", "width"];

fn boolean(v: &Value, what: &str) -> mlua::Result<bool> {
    match v { Value::Boolean(b) => Ok(*b), _ => Err(rt(format!("{what}: expected true or false"))) }
}

fn number(v: &Value, what: &str) -> mlua::Result<f32> { num(v).ok_or_else(|| rt(format!("{what}: expected a number"))) }

/// Apply `world.spawn`/`world.set` options to an entity's fields.
#[allow(clippy::too_many_arguments)]
fn apply(opts: &[(String, Value)], shape_: &mut Shape, look: &mut Look, pos: &mut Vec3, rot: &mut Vec3, sz: &mut Vec3, solid: &mut bool, visible: &mut bool, tag: &mut String) -> mlua::Result<()> {
    for (k, v) in opts {
        match k.as_str() {
            "shape" => *shape_ = shape(&v.to_string()?)?,
            "pos" => *pos = vec3(v, "pos")?,
            "rot" => *rot = vec3(v, "rot")?,
            "size" => *sz = size(v, "size")?,
            "color" | "emissive" | "roughness" | "metal" | "surface" => { look_key(look, k, v)?; }
            "solid" => *solid = boolean(v, "solid")?,
            "visible" => *visible = boolean(v, "visible")?,
            "tag" => *tag = v.to_string()?,
            "points" | "edges" | "width" => {}
            _ => unreachable!("checked by options"),
        }
    }
    Ok(())
}

/// A wire's keys: `points` (vectors in the entity's own space), `edges`
/// (pairs of 1-based point numbers, as `{ {1, 2}, {2, 3} }` or flat
/// `{ 1, 2, 2, 3 }`) and `width` (metres at size 1).
fn apply_wire(opts: &[(String, Value)], wire: &mut Option<Wire>) -> mlua::Result<()> {
    let wants = opts.iter().any(|(k, _)| matches!(k.as_str(), "points" | "edges" | "width"));
    if !wants { return Ok(()); }
    let Some(w) = wire else { return Err(rt("points, edges and width are for shape \"wire\"")) };
    for (k, v) in opts {
        match k.as_str() {
            "points" => {
                let Value::Table(t) = v else { return Err(rt("points: a list of vectors")) };
                w.points = t.sequence_values::<Value>().map(|p| p.and_then(|p| vec3(&p, "points"))).collect::<mlua::Result<_>>()?;
            }
            "edges" => {
                let Value::Table(t) = v else { return Err(rt("edges: a list of point pairs")) };
                let mut flat = Vec::new();
                for item in t.sequence_values::<Value>() {
                    match item? {
                        Value::Table(pair) => for n in pair.sequence_values::<f64>() { flat.push(n?); },
                        Value::Integer(n) => flat.push(n as f64),
                        Value::Number(n) => flat.push(n),
                        _ => return Err(rt("edges: pairs of point numbers")),
                    }
                }
                if flat.len() % 2 != 0 { return Err(rt("edges: an even number of point numbers (pairs)")) }
                w.edges = flat.chunks(2).map(|c| [c[0] as i64 - 1, c[1] as i64 - 1]).map(|[a, b]| [a.clamp(0, u16::MAX as i64) as u16, b.clamp(0, u16::MAX as i64) as u16]).collect();
            }
            "width" => w.width = number(v, "width")?.max(0.0),
            _ => {}
        }
    }
    if let Some(bad) = w.edges.iter().flatten().find(|&&i| i as usize >= w.points.len()) {
        return Err(rt(format!("edges: point {} does not exist ({} points)", bad + 1, w.points.len())));
    }
    Ok(())
}

/// A look's keys (color, emissive, roughness, metal, surface); false for any other key.
fn look_key(look: &mut Look, k: &str, v: &Value) -> mlua::Result<bool> {
    match k {
        "color" => { let c = color(v, "color")?; look.color = [c[0], c[1], c[2]]; }
        "emissive" => look.emissive = number(v, "emissive")?.max(0.0),
        "roughness" => look.roughness = number(v, "roughness")?,
        "metal" => look.metal = number(v, "metal")?,
        "surface" => look.surface = match v {
            Value::Nil | Value::Boolean(false) => None,
            _ => {
                let n = number(v, "surface")?;
                if n.fract() != 0.0 || !(0.0..=255.0).contains(&n) { return Err(rt("surface: a whole number 0 to 255 (the kind in surface.wgsl), or false")); }
                Some(n as u8)
            }
        },
        _ => return Ok(false),
    }
    Ok(true)
}

fn burst(opts: Option<Table>) -> mlua::Result<Burst> {
    let mut b = Burst::default();
    let Some(t) = opts else { return Ok(b) };
    for (k, v) in options(&t, &["shape", "color", "emissive", "roughness", "metal", "count", "speed", "life", "size", "stretch", "gravity", "flat"], "fx.burst")? {
        if look_key(&mut b.look, &k, &v)? { continue; }
        match k.as_str() {
            "shape" => b.shape = shape(&v.to_string()?)?,
            "count" => b.count = number(&v, "count")?.clamp(0.0, 256.0) as usize,
            "speed" => b.speed = number(&v, "speed")?.max(0.0),
            "life" => b.life = number(&v, "life")?.clamp(0.01, 10.0),
            "size" => b.size = number(&v, "size")?.max(0.0),
            "stretch" => b.stretch = number(&v, "stretch")?.max(0.0),
            "gravity" => b.gravity = number(&v, "gravity")?,
            "flat" => b.flat = boolean(&v, "flat")?,
            _ => unreachable!(),
        }
    }
    Ok(b)
}

fn ring(opts: Option<Table>) -> mlua::Result<RingFx> {
    let mut r = RingFx::default();
    let Some(t) = opts else { return Ok(r) };
    for (k, v) in options(&t, &["color", "emissive", "roughness", "metal", "radius", "life", "width", "segments"], "fx.ring")? {
        if look_key(&mut r.look, &k, &v)? { continue; }
        match k.as_str() {
            "radius" => r.radius = number(&v, "radius")?.max(0.0),
            "life" => r.life = number(&v, "life")?.clamp(0.01, 10.0),
            "width" => r.width = number(&v, "width")?.max(0.0),
            "segments" => r.segments = number(&v, "segments")?.clamp(3.0, 96.0) as usize,
            _ => unreachable!(),
        }
    }
    Ok(r)
}

fn entity_table(lua: &Lua, id: u32, e: &Entity) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("id", id)?;
    t.raw_set("shape", e.shape.name())?;
    t.raw_set("pos", vector(e.pos))?;
    t.raw_set("parent", e.parent())?;
    t.raw_set("world_pos", vector(e.world_pose().1))?;
    t.raw_set("rot", vector(e.rot))?;
    t.raw_set("size", vector(e.size))?;
    t.raw_set("color", Value::Vector(Vector::new(e.look.color[0], e.look.color[1], e.look.color[2])))?;
    t.raw_set("emissive", e.look.emissive)?;
    t.raw_set("roughness", e.look.roughness)?;
    t.raw_set("metal", e.look.metal)?;
    if let Some(k) = e.look.surface { t.raw_set("surface", k)?; }
    t.raw_set("solid", e.solid)?;
    t.raw_set("visible", e.visible)?;
    t.raw_set("tag", e.tag.as_str())?;
    Ok(t)
}

/// `{tag=, ignore= id or {ids}, all=}` for queries.
fn filter(opts: Option<Table>) -> mlua::Result<Filter> {
    let mut f = Filter::default();
    let Some(t) = opts else { return Ok(f) };
    for (k, v) in options(&t, &["tag", "ignore", "all"], "query")? {
        match k.as_str() {
            "tag" => f.tag = Some(v.to_string()?),
            "all" => f.all = boolean(&v, "all")?,
            "ignore" => match &v {
                Value::Table(ids) => for id in ids.sequence_values::<u32>() { f.ignore.push(id?); },
                other => f.ignore.push(num(other).ok_or_else(|| rt("ignore: an entity id or a list of ids"))? as u32),
            },
            _ => unreachable!(),
        }
    }
    Ok(f)
}

pub(crate) fn sim(c: &Core, what: &str) -> mlua::Result<()> {
    if c.phase == Phase::Draw { return Err(rt(format!("{what} changes the game, so it belongs in init or tick, not in draw (draw runs a varying number of times per tick)"))); }
    Ok(())
}

/// A player id (default: the first player) as an index into the roster.
fn player(c: &Core, p: Option<i64>) -> mlua::Result<usize> {
    let Some(p) = p else { return if c.roster.is_empty() { Err(rt("there are no players")) } else { Ok(0) } };
    c.roster.iter().position(|x| x.id as i64 == p).ok_or_else(|| {
        let ids: Vec<String> = c.roster.iter().map(|x| x.id.to_string()).collect();
        rt(format!("player {p}: no such player (the players are {}; use players.list())", ids.join(", ")))
    })
}

/// With one shared view, the player whose camera it is; else `i`.
fn viewer(c: &Core, i: usize) -> usize {
    if !c.shared_view { return i; }
    c.roster.iter().position(|p| p.view.is_some()).unwrap_or(i)
}

/// Where a world point shows in a view rectangle (pixels), or None behind the camera.
pub fn project(cam: &Cam, view: [f32; 4], at: Vec3) -> Option<[f32; 2]> {
    let aspect = (view[2] / view[3].max(1.0)).max(1e-3);
    let proj = glam::Mat4::perspective_rh(cam.fov.to_radians(), aspect, 0.05, 10_000.0);
    let look = glam::Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y);
    let clip = proj * look * at.extend(1.0);
    if clip.w <= 1e-4 { return None; }
    let ndc = clip.truncate() / clip.w;
    Some([view[0] + (ndc.x * 0.5 + 0.5) * view[2], view[1] + (0.5 - ndc.y * 0.5) * view[3]])
}

/// A player as Luau sees it.
pub fn player_table(lua: &Lua, p: &Player) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("id", p.id)?;
    t.raw_set("name", p.name.as_str())?;
    t.raw_set("color", Value::Vector(Vector::new(p.color[0], p.color[1], p.color[2])))?;
    t.raw_set("bot", p.bot)?;
    t.raw_set("human", !p.bot)?;
    if let Some(s) = p.seat { t.raw_set("seat", s)?; }
    if let Some(c) = p.couch { t.raw_set("couch", c)?; }
    Ok(t)
}

/// Add the API tables to a module base (`Host::new`'s `api`).
/// `base` with the fields a Luau table sets (unknown names are an error).
/// A struct as JSON with its f32 fields printed short (0.3, not 0.30000001).
fn short_json<T: serde::Serialize>(v: &T) -> mlua::Result<serde_json::Value> {
    serde_json::from_str(&serde_json::to_string(v).map_err(rt)?).map_err(rt)
}

fn merged<T: serde::Serialize + serde::de::DeserializeOwned>(base: &T, over: &Value, what: &str) -> mlua::Result<T> {
    let mut j = short_json(base)?;
    if !over.is_nil() {
        let serde_json::Value::Object(o) = crate::luajson::to_json(over) else { return Err(rt(format!("{what}: expected a table of fields"))) };
        let m = j.as_object_mut().expect("a struct");
        for (k, v) in o {
            if !m.contains_key(&k) { return Err(rt(format!("{what}: unknown field {k:?} (fields: {})", m.keys().cloned().collect::<Vec<_>>().join(", ")))); }
            m.insert(k, v);
        }
    }
    serde_json::from_value(j).map_err(|e| rt(format!("{what}: {e}")))
}

/// A data file name inside the game folder: letters, digits, `-` and `_`.
fn data_file(c: &Core, name: &str) -> mlua::Result<std::path::PathBuf> {
    if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') { return Err(rt(format!("{name:?}: a name of letters, digits, - and _"))); }
    Ok(c.world.dir().join(format!("{name}.json")))
}

pub fn register(lua: &Lua, api: &Table, core: Shared) -> mlua::Result<()> {
    crate::storage::register(lua, api, core.clone())?;
    let table = |name: &str, fns: Vec<(&str, mlua::Function)>| -> mlua::Result<()> {
        let t = lua.create_table()?;
        for (k, f) in fns { t.raw_set(k, f)?; }
        t.set_readonly(true);
        api.raw_set(name, t)
    };
    macro_rules! f {
        ($core:ident, $body:expr) => {{ let $core = core.clone(); lua.create_function($body)? }};
    }

    table("plants", vec![
        ("grow", f!(core, move |_, opts: Table| {
            let o = options(&opts, &["age", "seed", "pos", "yaw", "full", "species", "look"], "plants.grow")?;
            let get = |k: &str| o.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or(Value::Nil);
            let age = num(&get("age")).ok_or_else(|| rt("plants.grow: needs age (years)"))?;
            if !(0.0..=2000.0).contains(&age) { return Err(rt("plants.grow: age 0 to 2000 years")); }
            let pos = match get("pos") { Value::Nil => Vec3::ZERO, v => vec3(&v, "plants.grow pos")? };
            let p = crate::plants::Plant {
                species: merged(&render::plant_growth::Species::jeffrey_pine(), &get("species"), "plants.grow species")?,
                look: merged(&render::plant_growth_assembly::Look::jeffrey_pine(), &get("look"), "plants.grow look")?,
                age: age.round() as u32,
                seed: num(&get("seed")).map_or(1, |s| s.max(0.0) as u64),
                pos,
                yaw: num(&get("yaw")).unwrap_or(0.0),
                full: matches!(get("full"), Value::Boolean(true)),
            };
            let mut c = lock(&core);
            sim(&c, "plants.grow")?;
            Ok(c.plants.grow(p))
        })),
        ("remove", f!(core, move |_, id: u32| { let mut c = lock(&core); sim(&c, "plants.remove")?; Ok(c.plants.remove(id)) })),
        ("clear", f!(core, move |_, ()| { let mut c = lock(&core); sim(&c, "plants.clear")?; c.plants.clear(); Ok(()) })),
        ("status", f!(core, move |lua, ()| {
            let mut c = lock(&core);
            c.plants.poll();
            let (placed, shown, cooking, failed) = c.plants.status();
            let t = lua.create_table()?;
            t.raw_set("placed", placed)?; t.raw_set("shown", shown)?; t.raw_set("cooking", cooking)?; t.raw_set("failed", failed)?;
            t.raw_set("upgrading", c.plants.upgrading())?;
            if let Some(e) = c.plants.error() { t.raw_set("error", e)?; }
            Ok(t)
        })),
        ("defaults", f!(core, move |lua, ()| {
            let _ = &core;
            let t = lua.create_table()?;
            t.raw_set("species", crate::luajson::from_json(lua, &short_json(&render::plant_growth::Species::jeffrey_pine())?)?)?;
            t.raw_set("look", crate::luajson::from_json(lua, &short_json(&render::plant_growth_assembly::Look::jeffrey_pine())?)?)?;
            Ok(t)
        })),
        ("save", f!(core, move |_, (name, value): (String, Value)| {
            let file = data_file(&lock(&core), &name)?;
            let text = serde_json::to_string_pretty(&crate::luajson::to_json(&value)).map_err(rt)?;
            std::fs::write(&file, text + "\n").map_err(|e| rt(format!("{}: {e}", file.display())))
        })),
        ("load", f!(core, move |lua, name: String| {
            let file = data_file(&lock(&core), &name)?;
            let Ok(text) = std::fs::read_to_string(&file) else { return Ok(Value::Nil) };
            let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| rt(format!("{}: {e}", file.display())))?;
            crate::luajson::from_json(lua, &j)
        })),
    ])?;

    table("world", vec![
        ("spawn", f!(core, move |_, (name, opts, parent): (String, Option<Table>, Option<u32>)| {
            let mut s = Spawn { parent, shape: shape(&name)?, ..Default::default() };
            if let Some(t) = opts {
                let o = options(&t, &ENTITY_KEYS[1..], "world.spawn")?;
                apply(&o, &mut s.shape, &mut s.look, &mut s.pos, &mut s.rot, &mut s.size, &mut s.solid, &mut s.visible, &mut s.tag)?;
                if s.shape == Shape::Wire { s.wire = Some(Wire { points: Vec::new(), edges: Vec::new(), width: 0.05 }); }
                apply_wire(&o, &mut s.wire)?;
            }
            let mut c = lock(&core);
            sim(&c, "world.spawn")?;
            c.world.spawn(s).map_err(rt)
        })),
        ("set", f!(core, move |_, (id, opts): (u32, Table)| {
            let o = options(&opts, ENTITY_KEYS, "world.set")?;
            let mut c = lock(&core);
            sim(&c, "world.set")?;
            c.world.try_update(id, |e| {
                apply(&o, &mut e.shape, &mut e.look, &mut e.pos, &mut e.rot, &mut e.size, &mut e.solid, &mut e.visible, &mut e.tag)
                    .and_then(|_| apply_wire(&o, &mut e.wire)).map_err(|err| err.to_string())
            }).map_err(rt)
        })),
        ("set_parent", f!(core, move |_, (id, parent): (u32, Option<u32>)| {
            let mut c = lock(&core);
            sim(&c, "world.set_parent")?;
            c.world.set_parent(id, parent).map_err(rt)
        })),
        ("place", f!(core, move |_, (id, pos, yaw): (u32, Value, Option<f32>)| {
            let p = vec3(&pos, "world.place position")?;
            let mut c = lock(&core);
            sim(&c, "world.place")?;
            c.world.place(id, p, yaw).map_err(rt)
        })),
        ("warm", f!(core, move |_, (name, opts): (String, Option<Table>)| {
            let s = shape(&name)?;
            let mut look = Look::default();
            if let Some(t) = opts {
                for (k, v) in options(&t, &["color", "emissive", "roughness", "metal", "surface"], "world.warm")? { look_key(&mut look, &k, &v)?; }
            }
            lock(&core).world.warm(&s, &look).map_err(rt)
        })),
        ("get", f!(core, move |lua, id: u32| {
            let c = lock(&core);
            match c.world.get(id) { Some(e) => Ok(Value::Table(entity_table(lua, id, e)?)), None => Ok(Value::Nil) }
        })),
        ("exists", f!(core, move |_, id: u32| Ok(lock(&core).world.get(id).is_some()))),
        ("remove", f!(core, move |_, id: u32| {
            let mut c = lock(&core);
            sim(&c, "world.remove")?;
            Ok(c.world.remove(id))
        })),
        ("clear", f!(core, move |_, ()| {
            let mut c = lock(&core);
            sim(&c, "world.clear")?;
            c.world.clear();
            Ok(())
        })),
        ("find", f!(core, move |lua, tag: String| lua.create_sequence_from(lock(&core).world.find(&tag)))),
        ("count", f!(core, move |_, ()| Ok(lock(&core).world.entities.len()))),
        ("raycast", f!(core, move |lua, (origin, dir, max, opts): (Value, Value, Option<f32>, Option<Table>)| {
            let (o, d) = (vec3(&origin, "raycast origin")?, vec3(&dir, "raycast direction")?);
            let f = filter(opts)?;
            let c = lock(&core);
            let Some(h) = c.world.raycast(o, d, max.unwrap_or(1000.0), &f) else { return Ok(Value::Nil) };
            let t = lua.create_table()?;
            t.raw_set("id", h.id)?;
            t.raw_set("distance", h.distance)?;
            t.raw_set("point", vector(h.point))?;
            t.raw_set("normal", vector(h.normal))?;
            Ok(Value::Table(t))
        })),
        ("overlap", f!(core, move |lua, (center, radius, opts): (Value, f32, Option<Table>)| {
            let p = vec3(&center, "overlap centre")?;
            let f = filter(opts)?;
            lua.create_sequence_from(lock(&core).world.overlap(p, radius, &f))
        })),
        ("move", f!(core, move |_, (id, vel, dt, opts): (u32, Value, f32, Option<Table>)| {
            let v = vec3(&vel, "move velocity")?;
            let mut f = filter(opts)?;
            f.ignore.push(id);
            let mut c = lock(&core);
            sim(&c, "world.move")?;
            let e = c.world.get(id).ok_or_else(|| rt(format!("world.move: no entity {id}")))?;
            let b = e.bounds();
            let (radius, height) = (b.half.x.min(b.half.z), b.half.y * 2.0);
            let (center, vel, grounded) = c.world.move_body(b.center, v, dt, radius, height, &f);
            let pos = e.pos + e.local_delta(center - b.center);
            c.world.place(id, pos, None).map_err(rt)?;
            Ok((vector(vel), grounded))
        })),
    ])?;

    table("fx", vec![
        ("burst", f!(core, move |_, (pos, opts): (Value, Option<Table>)| {
            let (p, b) = (vec3(&pos, "fx.burst position")?, burst(opts)?);
            let mut c = lock(&core);
            sim(&c, "fx.burst")?;
            c.world.burst(p, &b).map_err(rt)
        })),
        ("ring", f!(core, move |_, (pos, opts): (Value, Option<Table>)| {
            let (p, r) = (vec3(&pos, "fx.ring position")?, ring(opts)?);
            let mut c = lock(&core);
            sim(&c, "fx.ring")?;
            c.world.ring(p, &r).map_err(rt)
        })),
        ("count", f!(core, move |_, ()| { let (p, r) = lock(&core).world.fx_count(); Ok((p, r)) })),
    ])?;

    table("camera", vec![
        ("set", f!(core, move |_, (p, opts): (i64, Table)| {
            let mut c = lock(&core);
            let i = player(&c, Some(p))?;
            let i = viewer(&c, i);
            let mut cam = c.roster[i].cam;
            for (k, v) in options(&opts, &["pos", "target", "fov"], "camera.set")? {
                match k.as_str() {
                    "pos" => cam.pos = vec3(&v, "camera pos")?,
                    "target" => cam.target = vec3(&v, "camera target")?,
                    "fov" => cam.fov = number(&v, "fov")?.clamp(5.0, 170.0),
                    _ => unreachable!(),
                }
            }
            c.roster[i].cam = cam;
            Ok(())
        })),
        ("get", f!(core, move |lua, p: Option<i64>| {
            let c = lock(&core);
            let cam = c.roster[viewer(&c, player(&c, p)?)].cam;
            let t = lua.create_table()?;
            t.raw_set("pos", vector(cam.pos))?;
            t.raw_set("target", vector(cam.target))?;
            t.raw_set("fov", cam.fov)?;
            Ok(t)
        })),
    ])?;

    table("sound", vec![
        ("play", f!(core, move |_, (name, opts): (String, Option<Table>)| {
            let (mut params, mut at, mut db, mut pitch) = (Vec::new(), None, 0.0, 1.0);
            if let Some(t) = opts {
                for (k, v) in options(&t, &[], "sound.play")? {
                    match k.as_str() {
                        "at" => at = Some(vec3(&v, "at")?.to_array()),
                        "db" => db = number(&v, "db")?,
                        "pitch" => pitch = number(&v, "pitch")?,
                        _ => { let x = number(&v, &k)?; params.push((k, x)); }
                    }
                }
            }
            let mut c = lock(&core);
            sim(&c, "sound.play")?;
            let tick = c.tick;
            c.sounds.play(&name, &params, at, db, pitch, tick).map_err(rt)
        })),
        ("set_param", f!(core, move |_, (h, param, value): (u32, String, f32)| { lock(&core).sounds.set(h, &param, value); Ok(()) })),
        ("stop", f!(core, move |_, h: u32| { lock(&core).sounds.stop(h); Ok(()) })),
        ("list", f!(core, move |lua, ()| lua.create_sequence_from(lock(&core).sounds.bank.patches.keys().cloned().collect::<Vec<_>>()))),
    ])?;

    table("hud", vec![
        ("text", f!(core, move |_, (text, x, y, opts): (Value, f32, f32, Option<Table>)| {
            let text = match &text { Value::String(s) => s.to_string_lossy(), other => other.to_string()? };
            let (mut size, mut col, mut align, mut width) = (24.0, [1.0; 4], TextAlign::Left, 0.0);
            if let Some(t) = opts {
                for (k, v) in options(&t, &["size", "color", "align", "width"], "hud.text")? {
                    match k.as_str() {
                        "size" => size = number(&v, "size")?.clamp(4.0, 400.0),
                        "color" => col = color(&v, "color")?,
                        "width" => width = number(&v, "width")?,
                        "align" => align = match v.to_string()?.as_str() {
                            "left" => TextAlign::Left, "center" => TextAlign::Center, "right" => TextAlign::Right,
                            a => return Err(rt(format!("align {a:?}: left, center or right"))),
                        },
                        _ => unreachable!(),
                    }
                }
            }
            lock(&core).hud.push(HudItem::Text { text, pos: [x, y], size, color: col, align, width });
            Ok(())
        })),
        ("box", f!(core, move |_, (x, y, w, h, col): (f32, f32, f32, f32, Option<Value>)| {
            let col = match col { Some(v) => color(&v, "hud.box colour")?, None => [0.0, 0.0, 0.0, 0.5] };
            lock(&core).hud.push(HudItem::Box { rect: [x, y, w, h], color: col });
            Ok(())
        })),
        ("size", f!(core, move |_, ()| { let s = lock(&core).screen; Ok((s[0], s[1])) })),
        ("project", f!(core, move |_, (pos, p): (Value, Option<i64>)| {
            let at = vec3(&pos, "hud.project position")?;
            let c = lock(&core);
            let i = viewer(&c, player(&c, p)?);
            let v = c.roster[i].view.and_then(|v| c.views.get(v)).copied().unwrap_or([0.0, 0.0, c.screen[0] as f32, c.screen[1] as f32]);
            Ok(match project(&c.roster[i].cam, v, at) { Some([x, y]) => (Value::Number(x as f64), Value::Number(y as f64)), None => (Value::Nil, Value::Nil) })
        })),
        ("view", f!(core, move |_, p: Option<i64>| {
            let c = lock(&core);
            let i = player(&c, p)?;
            let v = c.roster[viewer(&c, i)].view.and_then(|v| c.views.get(v)).copied().unwrap_or([0.0, 0.0, c.screen[0] as f32, c.screen[1] as f32]);
            Ok((v[0], v[1], v[2], v[3]))
        })),
    ])?;

    table("sky", vec![
        ("set", f!(core, move |_, name: String| {
            let choice = SkyChoice::parse(&name).map_err(rt)?;
            lock(&core).sky = choice;
            Ok(())
        })),
        ("get", f!(core, move |_, ()| Ok(lock(&core).sky.name()))),
    ])?;

    table("light", vec![
        ("set", f!(core, move |_, opts: Table| {
            let mut c = lock(&core);
            for (k, v) in options(&opts, &["sun", "sun_lux", "sky_lux", "exposure", "exposure_min", "exposure_max", "exposure_bias", "adapt_speed", "fog_start", "fog_end", "fog_density"], "light.set")? {
                let l = &mut c.light;
                match k.as_str() {
                    "sun" => l.sun = Some(vec3(&v, "sun")?.try_normalize().ok_or_else(|| rt("sun: a direction towards the sun, not zero"))?),
                    "sun_lux" => l.sun_lux = Some(number(&v, &k)?.max(0.0)),
                    "sky_lux" => l.sky_lux = Some(number(&v, &k)?.max(0.0)),
                    // A number fixes the exposure (EV100); "auto" meters it.
                    "exposure" => match &v {
                        Value::String(s) if s.to_string_lossy() == "auto" => { l.auto.get_or_insert_with(Default::default); }
                        _ => { l.ev100 = Some(number(&v, "exposure: a number (EV100) or \"auto\"")?); l.auto = None; }
                    },
                    "exposure_min" | "exposure_max" | "exposure_bias" | "adapt_speed" => {
                        let x = number(&v, &k)?;
                        let a = l.auto.get_or_insert_with(Default::default);
                        match k.as_str() {
                            "exposure_min" => a.min_ev100 = x,
                            "exposure_max" => a.max_ev100 = x,
                            "exposure_bias" => a.compensation = x.clamp(-8.0, 8.0),
                            // Eyes adapt to the dark about three times slower.
                            _ => { let x = x.max(0.0); a.speed_light = x; a.speed_dark = x / 3.0; }
                        }
                    }
                    "fog_start" => l.fog[0] = Some(number(&v, &k)?),
                    "fog_end" => l.fog[1] = Some(number(&v, &k)?),
                    "fog_density" => l.fog[2] = Some(number(&v, &k)?.max(0.0)),
                    _ => unreachable!(),
                }
            }
            Ok(())
        })),
        ("reset", f!(core, move |_, ()| { lock(&core).light = LightOverride::default(); Ok(()) })),
    ])?;

    table("post", vec![
        ("set", f!(core, move |_, opts: Table| {
            let mut c = lock(&core);
            for (k, v) in options(&opts, &["bloom", "bloom_radius", "bloom_threshold", "bloom_wide", "contrast", "saturation", "vignette", "grain", "aberration"], "post.set")? {
                if k == "bloom_wide" { c.post.bloom_wide = boolean(&v, "bloom_wide")?; continue; }
                let x = number(&v, &k)?;
                let p = &mut c.post;
                match k.as_str() {
                    "bloom" => p.bloom = Some(x.clamp(0.0, 2.0)),
                    "bloom_radius" => p.bloom_radius = x.round().clamp(1.0, render::bloom::MAX_ROUNDS as f32) as u32,
                    "bloom_threshold" => p.bloom_threshold = x.clamp(0.0, 16.0),
                    "contrast" => p.contrast = x.clamp(0.5, 2.0),
                    "saturation" => p.saturation = x.clamp(0.0, 2.0),
                    "vignette" => p.vignette = x.clamp(0.0, 1.0),
                    "grain" => p.grain = x.clamp(0.0, 1.0),
                    "aberration" => p.aberration = x.clamp(0.0, 16.0),
                    _ => unreachable!(),
                }
            }
            Ok(())
        })),
        ("reset", f!(core, move |_, ()| { lock(&core).post = Post::default(); Ok(()) })),
    ])?;

    table("lights", vec![
        ("set", f!(core, move |_, list: Table| {
            let mut lamps = Vec::new();
            for item in list.sequence_values::<Table>() {
                let t = item?;
                let mut l = Lamp { pos: Vec3::ZERO, color: [1.0; 3], intensity: 1.0, range: 6.0 };
                for (k, v) in options(&t, &["pos", "color", "intensity", "range"], "lights.set")? {
                    match k.as_str() {
                        "pos" => l.pos = vec3(&v, "light pos")?,
                        "color" => { let c = color(&v, "light color")?; l.color = [c[0], c[1], c[2]]; }
                        "intensity" => l.intensity = number(&v, "intensity")?.clamp(0.0, 10_000.0),
                        "range" => l.range = number(&v, "range")?.clamp(0.5, 200.0),
                        _ => unreachable!(),
                    }
                }
                if lamps.len() == MAX_LAMPS { return Err(rt(format!("lights.set: at most {MAX_LAMPS} lights"))); }
                lamps.push(l);
            }
            lock(&core).lamps = lamps;
            Ok(())
        })),
        ("clear", f!(core, move |_, ()| { lock(&core).lamps.clear(); Ok(()) })),
    ])?;

    table("random", vec![
        ("float", f!(core, move |_, ()| { let mut c = lock(&core); sim(&c, "random")?; Ok(c.rng.float()) })),
        ("range", f!(core, move |_, (lo, hi): (f64, f64)| { let mut c = lock(&core); sim(&c, "random")?; Ok(lo + (hi - lo) * c.rng.float()) })),
        ("int", f!(core, move |_, (lo, hi): (i64, i64)| {
            if hi < lo { return Err(rt(format!("random.int({lo}, {hi}): the second bound is below the first"))); }
            let mut c = lock(&core);
            sim(&c, "random")?;
            Ok(lo + (c.rng.next() % ((hi - lo) as u64 + 1)) as i64)
        })),
        ("chance", f!(core, move |_, p: f64| { let mut c = lock(&core); sim(&c, "random")?; Ok(c.rng.float() < p) })),
        ("seed", f!(core, move |_, s: i64| { let mut c = lock(&core); sim(&c, "random")?; c.rng = Rng(s as u64); Ok(()) })),
    ])?;

    table("game", vec![
        ("tick", f!(core, move |_, ()| Ok(lock(&core).tick))),
        ("time", f!(core, move |_, ()| { let c = lock(&core); Ok(c.tick as f64 / c.tick_rate as f64) })),
        ("dt", f!(core, move |_, ()| Ok(1.0 / lock(&core).tick_rate))),
        ("players", f!(core, move |_, ()| Ok(lock(&core).roster.len()))),
        ("title", f!(core, move |_, ()| Ok(lock(&core).title.clone()))),
        ("finish", f!(core, move |_, ()| { let mut c = lock(&core); sim(&c, "game.finish")?; c.requests.finish = true; Ok(()) })),
        ("log", f!(core, move |_, text: Value| {
            let text = match &text { Value::String(s) => s.to_string_lossy(), other => other.to_string()? };
            let mut c = lock(&core);
            sim(&c, "game.log")?;
            if c.requests.logs.len() < 8 { c.requests.logs.push(text.chars().take(120).collect()); }
            Ok(())
        })),
    ])?;

    table("players", vec![
        ("list", f!(core, move |lua, ()| {
            let c = lock(&core);
            let all = lua.create_table()?;
            for (i, p) in c.roster.iter().enumerate() { all.raw_set(i + 1, player_table(lua, p)?)?; }
            Ok(all)
        })),
        ("get", f!(core, move |lua, id: i64| {
            let c = lock(&core);
            match c.roster.iter().find(|p| p.id as i64 == id) { Some(p) => Ok(Value::Table(player_table(lua, p)?)), None => Ok(Value::Nil) }
        })),
        ("count", f!(core, move |_, ()| Ok(lock(&core).roster.len()))),
        ("rumble", f!(core, move |_, (id, strength, seconds): (i64, f64, Option<f64>)| {
            let mut c = lock(&core);
            sim(&c, "players.rumble")?;
            let i = player(&c, Some(id))?;
            let id = c.roster[i].id;
            let s = strength.clamp(0.0, 1.0);
            if c.requests.rumble.len() < 16 { c.requests.rumble.push((id, s, s, seconds.unwrap_or(0.2).clamp(0.0, 5.0))); }
            Ok(())
        })),
    ])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_vectors_and_random() {
        let lua = Lua::new();
        let v = |s: &str| -> Value { lua.load(s).eval().unwrap() };
        assert_eq!(color(&v("'#ff8000'"), "c").unwrap(), [1.0, 128.0 / 255.0, 0.0, 1.0]);
        assert_eq!(color(&v("{0.5, 0.25, 1, 0.5}"), "c").unwrap(), [0.5, 0.25, 1.0, 0.5]);
        assert_eq!(color(&v("{r = 1, g = 0, b = 0}"), "c").unwrap(), [1.0, 0.0, 0.0, 1.0]);
        assert!(color(&v("'#ff80'"), "c").is_err());
        assert_eq!(vec3(&v("vector.create(1, 2, 3)"), "p").unwrap(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(vec3(&v("{x = 1, y = 2, z = 3}"), "p").unwrap(), Vec3::new(1.0, 2.0, 3.0));
        assert!(vec3(&v("'up'"), "p").is_err());
        let (mut a, mut b) = (Rng(7), Rng(7));
        let xs: Vec<f64> = (0..100).map(|_| a.float()).collect();
        assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
        assert_eq!(xs, (0..100).map(|_| b.float()).collect::<Vec<_>>(), "the same seed, the same numbers");
        let cam = Cam { pos: Vec3::new(0.0, 10.0, 0.01), target: Vec3::ZERO, fov: 60.0 };
        let mid = project(&cam, [0.0, 0.0, 200.0, 100.0], Vec3::ZERO).unwrap();
        assert!((mid[0] - 100.0).abs() < 0.01 && (mid[1] - 50.0).abs() < 0.01, "{mid:?}");
        let right = project(&cam, [0.0, 0.0, 200.0, 100.0], Vec3::X).unwrap();
        assert!(right[0] > 100.0, "+X is to the right looking down with -Z up the screen");
        assert!(project(&cam, [0.0, 0.0, 200.0, 100.0], Vec3::new(0.0, 20.0, 0.0)).is_none(), "behind the camera");
    }
}
