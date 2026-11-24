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

/// Colours for players without one (Splitscreen Club gives its own).
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
    /// The Splitscreen Club seat, when a party host seated this player.
    pub seat: Option<usize>,
    /// Couch position from the lobby (0 is leftmost), when known.
    pub couch: Option<i64>,
    pub cam: Cam,
    /// The camera at the start of the current tick: frames between ticks
    /// are drawn from here towards `cam` (smooth at 120 Hz on a 60 Hz tick).
    pub cam_prev: Cam,
    /// The split-screen view this player looks through (people only).
    pub view: Option<usize>,
    /// The Splitscreen Club face (`hud.face`): its sources and the composed art.
    pub face_src: (String, String),
    pub face: Option<std::sync::Arc<crate::face::Face>>,
}

impl Player {
    pub fn new(id: u32, name: String, color: Option<[f32; 3]>, bot: bool) -> Self {
        Player { id, name, color: color.unwrap_or(PLAYER_COLORS[(id as usize - 1) % 4]), bot, seat: None, couch: None, cam: Cam::default(), cam_prev: Cam::default(), view: None, face_src: (String::new(), String::new()), face: None }
    }

    /// Recompose the face when the profile's skin or art changed.
    pub fn set_face(&mut self, skin: &str, art: &str) {
        let src = (skin.to_string(), art.to_string());
        if self.face.is_some() && self.face_src == src { return; }
        self.face = Some(std::sync::Arc::new(crate::face::compose(art, skin, self.color)));
        self.face_src = src;
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
    /// Native aerial perspective and clouds, shared by every camera (`sky.clouds`).
    pub sky_volume: Option<render::sky_volume::SkyVolume>,
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

/// A spotlight that casts shadows (`lights.spots`): torches, headlights,
/// a lamp over a door. `angle` is the outer half-angle in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot { pub pos: Vec3, pub dir: Vec3, pub color: [f32; 3], pub intensity: f32, pub range: f32, pub angle: f32, pub inner: f32 }

pub const MAX_SPOTS: usize = render::headlights::MAX_LIGHTS;

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
    pub(crate) rooms: crate::room_scene::RoomScene,
    pub spots: Vec<Spot>,
    /// Warp Humans (game.toml `[humans.NAME]` pools).
    pub humans: crate::humans::Humans,
    pub tick: u64,
    pub tick_rate: f32,
    pub title: String,
    pub requests: Requests,
    /// One shared view (game.toml `split = false`): every camera call is the viewer's.
    pub shared_view: bool,
    pub phase: Phase,
}

impl Core {
    /// Interpolated simulation time for rendering between the last two fixed ticks.
    pub(crate) fn render_time(&self, alpha: f32) -> f64 {
        (self.tick as f64 - 1.0 + f64::from(alpha)).max(0.0) / f64::from(self.tick_rate)
    }
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
        "pane" => Shape::Pane,
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
        p if p.split_once("#part-").is_some_and(|(file,name)| (file.ends_with(".glb") || file.ends_with(".gltf")) && !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')) => Shape::Model(p.to_string()),
        p if p.ends_with(".glb") || p.ends_with(".gltf") || p.rsplit_once("#crush").is_some_and(|(file, n)|
            (file.ends_with(".glb") || file.ends_with(".gltf")) && matches!(n.as_bytes(), [b'0'..=b'8'])) => Shape::Model(p.to_string()),
        other => return Err(rt(format!("unknown shape {other:?}: cube, pane, sphere, capsule, outline6 or a model path such as \"assets/crate.glb\""))),
    })
}

const ENTITY_KEYS: &[&str] = &["shape", "pos", "rot", "size", "color", "emissive", "roughness", "metal", "surface", "glass", "coverage", "solid", "visible", "shadow_only", "hide_views", "tag", "points", "edges", "width"];

fn boolean(v: &Value, what: &str) -> mlua::Result<bool> {
    match v { Value::Boolean(b) => Ok(*b), _ => Err(rt(format!("{what}: expected true or false"))) }
}

fn number(v: &Value, what: &str) -> mlua::Result<f32> { num(v).ok_or_else(|| rt(format!("{what}: expected a number"))) }

/// A complete weather declaration. Start with the native fair-weather defaults.
fn cloud_volume(value: Value) -> mlua::Result<Option<render::sky_volume::SkyVolume>> {
    use render::sky_volume::{Clouds, SkyVolume};
    match value {
        Value::Boolean(false) => Ok(None),
        Value::Boolean(true) => Ok(Some(SkyVolume::default())),
        Value::Table(t) => {
            let mut volume = SkyVolume::default();
            let mut wind = Clouds::default().wind;
            for (key, value) in options(&t, &["coverage", "base", "thickness", "density", "wind_x", "wind_z", "shadow", "steps", "light_steps", "far", "aerial"], "sky.clouds")? {
                match key.as_str() {
                    "aerial" => volume.aerial = boolean(&value, &key)?,
                    "wind_x" => wind.x = number(&value, &key)?,
                    "wind_z" => wind.y = number(&value, &key)?,
                    _ => volume.set(&format!("cloud_{key}"), number(&value, &key)?).map_err(rt)?,
                }
            }
            // Validate the two wind components together (the limit is on speed).
            volume.clouds.as_mut().unwrap().wind = wind;
            Ok(Some(volume.validated().map_err(rt)?))
        }
        _ => Err(rt("sky.clouds expects true, false or a cloud settings table")),
    }
}

/// Apply `world.spawn`/`world.set` options to an entity's fields.
#[allow(clippy::too_many_arguments)]
/// `hide_views`: a mask, bit i hides the entity from camera view i (it
/// still casts shadows and shows in the other views).
fn hide_views(opts: &[(String, Value)]) -> mlua::Result<Option<u8>> {
    for (k, v) in opts {
        if k == "hide_views" {
            let m = number(v, "hide_views")?;
            if !(0.0..=255.0).contains(&m) || m.fract() != 0.0 { return Err(rt("hide_views: a whole number 0 to 255 (bit i hides view i)")); }
            return Ok(Some(m as u8));
        }
    }
    Ok(None)
}

fn apply(opts: &[(String, Value)], shape_: &mut Shape, look: &mut Look, pos: &mut Vec3, rot: &mut Vec3, sz: &mut Vec3, solid: &mut bool, visible: &mut bool, shadow_only: &mut bool, tag: &mut String) -> mlua::Result<()> {
    for (k, v) in opts {
        match k.as_str() {
            "shape" => *shape_ = shape(&v.to_string()?)?,
            "pos" => *pos = vec3(v, "pos")?,
            "rot" => *rot = vec3(v, "rot")?,
            "size" => *sz = size(v, "size")?,
            "color" | "emissive" | "roughness" | "metal" | "surface" | "glass" | "coverage" => { look_key(look, k, v)?; }
            "solid" => *solid = boolean(v, "solid")?,
            "visible" => *visible = boolean(v, "visible")?,
            "shadow_only" => *shadow_only = boolean(v, "shadow_only")?,
            "tag" => *tag = v.to_string()?,
            "points" | "edges" | "width" | "hide_views" => {}
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

/// Read and validate the entire geometry before touching scene storage.
fn mesh_parts(parts: Table) -> mlua::Result<Vec<crate::world::geometry::MeshPart>> {
    use crate::world::geometry::{MeshPart, MAX_VERTICES, MAX_INDICES};
    use render::cluster::Vertex;
    if !(1..=32).contains(&parts.raw_len()) { return Err(rt("world.mesh expects 1..32 parts")); }
    let mut result=Vec::new();
    let (mut nv,mut ni)=(0,0);
    for n in 1..=parts.raw_len() {
        let part:Table=parts.raw_get(n)?;
        let mut look=Look::default();
        for (k,v) in options(&part, &["positions","normals","uvs","indices","color","roughness","metal","surface"], "mesh part")? {
            look_key(&mut look,&k,&v)?;
        }
        if !look.color.iter().all(|v|v.is_finite()) || !look.roughness.is_finite() || !look.metal.is_finite() {
            return Err(rt("mesh material must be finite"));
        }
        let positions:Table=part.raw_get("positions")?;
        let normals:Table=part.raw_get("normals")?;
        let uvs:Option<Table>=part.raw_get("uvs")?;
        let indices:Table=part.raw_get("indices")?;
        let (count,faces)=(positions.raw_len(),indices.raw_len());
        if count<3 || faces<3 || faces%3!=0 || normals.raw_len()!=count || uvs.as_ref().is_some_and(|t|t.raw_len()!=count) {
            return Err(rt("mesh needs matching positions and normals (and optional UVs), and index triples"));
        }
        nv+=count; ni+=faces;
        if nv>MAX_VERTICES || ni>MAX_INDICES { return Err(rt("world.mesh geometry budget exceeded")); }
        let mut vertices=Vec::with_capacity(count);
        for i in 1..=count {
            let pos=vec3(&positions.raw_get(i)?,"mesh position")?;
            let norm=vec3(&normals.raw_get(i)?,"mesh normal")?.try_normalize().ok_or_else(||rt("invalid mesh normal"))?;
            let uv=match &uvs {
                None => [0.,0.],
                Some(t) => {
                    let v=vec3(&t.raw_get(i)?,"mesh UV vector")?;
                    [v.x,v.y]
                }
            };
            if !pos.is_finite() || pos.abs().max_element()>1_000_000. || !uv.iter().all(|v|v.is_finite() && v.abs()<=1_000_000.) {
                return Err(rt("invalid mesh position or UV"));
            }
            vertices.push(Vertex{pos:pos.to_array(),normal:norm.to_array(),uv});
        }
        let mut triangles=Vec::with_capacity(faces);
        for i in 1..=faces {
            let v:f64=indices.raw_get(i)?;
            if !(1.0..=count as f64).contains(&v) || v.fract()!=0. { return Err(rt("mesh indices must reference 1-based vertices")); }
            triangles.push(v as u32-1);
        }
        for face in triangles.chunks_exact(3) {
            let point=|i:u32| Vec3::from(vertices[i as usize].pos);
            if (point(face[1])-point(face[0])).cross(point(face[2])-point(face[0])).length_squared()==0. {
                return Err(rt("degenerate mesh triangle"));
            }
        }
        result.push(MeshPart{vertices,indices:triangles,look});
    }
    Ok(result)
}

/// A look's keys (color, emissive, roughness, metal, surface); false for any other key.
fn look_key(look: &mut Look, k: &str, v: &Value) -> mlua::Result<bool> {
    match k {
        "color" => { let c = color(v, "color")?; look.color = [c[0], c[1], c[2]]; }
        "emissive" => look.emissive = number(v, "emissive")?.max(0.0),
        "roughness" => look.roughness = number(v, "roughness")?,
        "metal" => look.metal = number(v, "metal")?,
        "coverage" => look.coverage = match v {
            Value::Boolean(false) => None,
            _ => {
                let coverage = number(v, "coverage")?;
                if !(0.0..=1.0).contains(&coverage) {
                    return Err(rt("coverage: a number from 0 to 1, or false"));
                }
                Some((coverage * 255.0).round() as u8)
            }
        },
        "glass" => look.glass = match v {
            Value::Nil | Value::Boolean(false) => None,
            _ => {
                let alpha = number(v, "glass")?;
                if !(0.0..=1.0).contains(&alpha) { return Err(rt("glass: scattering opacity from 0 to 1, or false")); }
                Some(alpha)
            }
        },
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
    if let Some(byte) = e.look.coverage { t.raw_set("coverage", f32::from(byte) / 255.0)?; }
    if let Some(alpha) = e.look.glass { t.raw_set("glass", alpha)?; }
    if let Some(k) = e.look.surface { t.raw_set("surface", k)?; }
    t.raw_set("solid", e.solid)?;
    t.raw_set("visible", e.visible)?;
    t.raw_set("shadow_only", e.shadow_only)?;
    t.raw_set("hide_views", e.hide_views)?;
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

/// The player whose camera `i` looks through: the first one in the roster
/// on the same view (players can share one, `camera.views`), with one
/// shared view the first with a view; else `i`.
fn viewer(c: &Core, i: usize) -> usize {
    if let Some(v) = c.roster[i].view { return c.roster.iter().position(|p| p.view == Some(v)).unwrap_or(i); }
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
    if let Some(view) = p.view { t.raw_set("view", view)?; }
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
    crate::vehicles::register(lua, api, core.clone())?;
    let table = |name: &str, fns: Vec<(&str, mlua::Function)>| -> mlua::Result<()> {
        let t = lua.create_table()?;
        for (k, f) in fns { t.raw_set(k, f)?; }
        t.set_readonly(true);
        api.raw_set(name, t)
    };
    macro_rules! f {
        ($core:ident, $body:expr) => {{ let $core = core.clone(); lua.create_function($body)? }};
    }

    table("grass", vec![("set", f!(core, move |_, opts: Option<Table>| {
        let mut c = lock(&core);
        sim(&c, "grass.set")?;
        if let Some(opts) = opts {
            options(&opts, &["patches", "cutouts", "seed", "growth", "distance"], "grass.set")?;
            let rectangles = |key: &str| -> mlua::Result<Vec<crate::grass::Rect>> {
                let Some(list) = opts.raw_get::<Option<Table>>(key)? else { return Ok(vec![]) };
                list.sequence_values::<Table>().map(|t| {
                    let t = t?;
                    options(&t, &["pos", "width", "depth", "yaw"], "grass rectangle")?;
                    crate::grass::Rect::new(vec3(&t.raw_get("pos")?, "grass rectangle pos")?,
                        number(&t.raw_get("width")?, "grass rectangle width")?,
                        number(&t.raw_get("depth")?, "grass rectangle depth")?,
                        t.raw_get::<Option<f32>>("yaw")?.unwrap_or(0.)).map_err(rt)
                }).collect()
            };
            let lawns = crate::grass::Lawns { patches: rectangles("patches")?, cutouts: rectangles("cutouts")?,
                seed: opts.raw_get::<Option<u32>>("seed")?.unwrap_or(1),
                growth: opts.raw_get::<Option<f32>>("growth")?.unwrap_or(0.3),
                distance: opts.raw_get::<Option<f32>>("distance")?.unwrap_or(24.) };
            let (field, footprints) = lawns.build().map_err(rt)?;
            c.world.scene.meadow = Some(field);
            c.world.scene.meadow_footprints = footprints;
            c.world.rebuild = true;
        } else if c.world.scene.meadow.take().is_some() {
            c.world.scene.meadow_footprints.clear();
            c.world.rebuild = true;
        }
        Ok(())
    }))])?;

    table("plants", vec![
        ("grow", f!(core, move |_, opts: Table| {
            let o = options(&opts, &["age", "seed", "pos", "yaw", "scale", "full", "species", "look"], "plants.grow")?;
            let get = |k: &str| o.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or(Value::Nil);
            let age = num(&get("age")).ok_or_else(|| rt("plants.grow: needs age (years)"))?;
            if !(0.0..=2000.0).contains(&age) { return Err(rt("plants.grow: age 0 to 2000 years")); }
            let pos = match get("pos") { Value::Nil => Vec3::ZERO, v => vec3(&v, "plants.grow pos")? };
            let scale = match get("scale") { Value::Nil => 1.0, v => number(&v, "plants.grow scale")? };
            if !scale.is_finite() || scale <= 0.0 { return Err(rt("plants.grow scale: expected a positive finite number")); }
            let p = crate::plants::Plant {
                species: merged(&render::plant_growth::Species::jeffrey_pine(), &get("species"), "plants.grow species")?,
                look: merged(&render::plant_growth_assembly::Look::jeffrey_pine(), &get("look"), "plants.grow look")?,
                age: age.round() as u32,
                seed: num(&get("seed")).map_or(1, |s| s.max(0.0) as u64),
                pos,
                yaw: num(&get("yaw")).unwrap_or(0.0),
                scale,
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
            vfs::write(&file, text + "\n").map_err(|e| rt(format!("{}: {e}", file.display())))
        })),
        ("load", f!(core, move |lua, name: String| {
            let file = data_file(&lock(&core), &name)?;
            let Ok(text) = vfs::read_to_string(&file) else { return Ok(Value::Nil) };
            let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| rt(format!("{}: {e}", file.display())))?;
            crate::luajson::from_json(lua, &j)
        })),
    ])?;

    table("world", vec![
        ("mesh", f!(core, move |_, (parts,opts): (Value,Value)| {
            let mut c=lock(&core);
            let spawn = || -> mlua::Result<u32> {
                sim(&c,"world.mesh")?;
                let Value::Table(parts)=parts else {return Err(rt("world.mesh expects a list of parts"));};
                let mut s=Spawn { solid:false,..Default::default() };
                if let Value::Table(t)=opts {
                    let opts=options(&t,&["pos","rot","size","solid","visible","tag"],"world.mesh")?;
                    apply(&opts,&mut s.shape,&mut s.look,&mut s.pos,&mut s.rot,&mut s.size,&mut s.solid,&mut s.visible,&mut s.shadow_only,&mut s.tag)?;
                    if !s.pos.is_finite() || !s.rot.is_finite() || !s.size.is_finite() { return Err(rt("mesh transform must be finite")); }
                } else if !opts.is_nil() {
                    return Err(rt("world.mesh expects an options table"));
                }
                c.world.add_geometry(mesh_parts(parts)?,s).map_err(rt)
            };
            Ok(match spawn() {
                Ok(id) => (Some(id),None),
                Err(err) => (None,Some(err.to_string())),
            })
        })),
        ("spawn", f!(core, move |_, (name, opts, parent): (String, Option<Table>, Option<u32>)| {
            let mut s = Spawn { parent, shape: shape(&name)?, ..Default::default() };
            if let Some(t) = opts {
                let o = options(&t, &ENTITY_KEYS[1..], "world.spawn")?;
                apply(&o, &mut s.shape, &mut s.look, &mut s.pos, &mut s.rot, &mut s.size, &mut s.solid, &mut s.visible, &mut s.shadow_only, &mut s.tag)?;
                if let Some(m) = hide_views(&o)? { s.hide_views = m; }
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
                apply(&o, &mut e.shape, &mut e.look, &mut e.pos, &mut e.rot, &mut e.size, &mut e.solid, &mut e.visible, &mut e.shadow_only, &mut e.tag)
                    .and_then(|_| apply_wire(&o, &mut e.wire))
                    .and_then(|_| { if let Some(m) = hide_views(&o)? { e.hide_views = m; } Ok(()) })
                    .map_err(|err| err.to_string())
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
                for (k, v) in options(&t, &["color", "emissive", "roughness", "metal", "surface", "glass", "coverage"], "world.warm")? { look_key(&mut look, &k, &v)?; }
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
            let removed = c.world.remove(id);
            if removed { c.rooms.check_geometry = true; }
            Ok(removed)
        })),
        ("clear", f!(core, move |_, ()| {
            let mut c = lock(&core);
            sim(&c, "world.clear")?;
            c.world.clear();
            c.rooms.clear();
            Ok(())
        })),
        ("find", f!(core, move |lua, tag: String| lua.create_sequence_from(lock(&core).world.find(&tag)))),
        ("count", f!(core, move |_, ()| Ok(lock(&core).world.entities.len()))),
        ("capacity", f!(core, move |_, ()| Ok(lock(&core).world.capacity()))),
        ("part_center", f!(core, move |_, path:String| Ok(lock(&core).world.part_center(&path).map(vector)))),
        ("warm_dents", f!(core, move |_, (list,count): (Table,usize)| {
            if list.raw_len()>16 { return Err(rt("too many dent models")); }
            let paths=list.sequence_values::<String>().collect::<mlua::Result<Vec<_>>>()?;
            let mut c=lock(&core);sim(&c,"world.warm_dents")?;
            c.world.warm_dents(paths,count).map_err(rt)
        })),
        ("dent", f!(core, move |_, (id,fields,mask): (u32,Option<Table>,Option<u32>)| {
            let fields=fields.map(crate::vehicles::read_dents).transpose()?;
            let mut c=lock(&core);sim(&c,"world.dent")?;
            c.world.dent(id,fields,mask.unwrap_or(0)).map_err(rt)
        })),
        ("heightfield", f!(core, move |_, opts: Table| {
            let origin = vec3(&opts.raw_get("origin")?, "heightfield origin")?;
            let columns = opts.raw_get("columns")?;
            let spacing = opts.raw_get("spacing")?;
            let samples: Table = opts.raw_get("heights")?;
            use physics::terrain::{HeightGrid, RoadStrip};
            use physics::vehicle::surfaces::Surface;
            if samples.raw_len() > HeightGrid::MAX_SAMPLES { return Err(rt("heightfield sample budget exceeded")); }
            let heights = samples.sequence_values::<f32>().collect::<mlua::Result<Vec<_>>>()?;
            let grip = |name: Option<String>| {
                let name = name.as_deref().unwrap_or("grass");
                Surface::ALL.into_iter().find(|s| s.name()==name).ok_or_else(|| rt(format!("unknown terrain grip {name}")))
            };
            let base = grip(opts.raw_get("grip")?)?;
            let mut roads = Vec::new();
            let mut count = 0;
            if let Some(routes) = opts.raw_get::<Option<Table>>("roads")? {
                for route in routes.sequence_values::<Table>() {
                    let route = route?;
                    let points: Table = route.raw_get("points")?;
                    count += points.raw_len();
                    if count > HeightGrid::MAX_ROAD_POINTS || points.raw_len()<2 { return Err(rt("invalid heightfield road point count")); }
                    let points = points.sequence_values::<Table>().map(|p| {
                        let p = p?;
                        Ok(glam::Vec2::new(p.raw_get::<f32>(1)?-origin.x,p.raw_get::<f32>(2)?-origin.z))
                    }).collect::<mlua::Result<Vec<_>>>()?;
                    roads.push(RoadStrip { points, width: route.raw_get("width")?, surface: grip(route.raw_get("grip")?)? });
                }
            }
            let tag = opts.raw_get::<Option<String>>("tag")?.unwrap_or_default();
            let grid = HeightGrid::new(columns,spacing,&heights,base,&roads).map_err(rt)?;
            let mut c = lock(&core);
            sim(&c,"world.heightfield")?;
            c.world.add_heightfield(grid,origin,tag).map_err(rt)
        })),
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
        ("warm_explosions", f!(core, move |_, ()| {
            let mut c = lock(&core);
            sim(&c, "fx.warm_explosions")?;
            c.world.explosions_prepared.get_or_insert(false);
            Ok(())
        })),
        ("explosion", f!(core, move |_, (pos, radius, light_range): (Value, Value, Option<f32>)| {
            let pos = vec3(&pos, "fx.explosion position")?;
            let radius = number(&radius, "fx.explosion radius")?;
            if !pos.is_finite() || !radius.is_finite() || radius <= 0.0 || radius > 200.0 {
                return Err(rt("fx.explosion needs a finite position and a radius in (0, 200] metres"));
            }
            if light_range.is_some_and(|reach| !(1.0..=500.0).contains(&reach)) {
                return Err(rt("fx.explosion light range must be 1 to 500 metres"));
            }
            let mut c = lock(&core);
            sim(&c, "fx.explosion")?;
            let time = c.tick as f32 / c.tick_rate;
            c.world.render_effects().explosion_with_light_range(time, pos, radius, Vec3::new(1.0, 0.70, 0.20), light_range);
            Ok(())
        })),
        ("flame", f!(core, move |_, (pos, width, height, life): (Value, f32, f32, f32)| {
            let pos = vec3(&pos, "fx.flame position")?;
            if !pos.is_finite() || !(0.05..=4.).contains(&width) || !(0.1..=8.).contains(&height) || !(0.1..=4.).contains(&life) {
                return Err(rt("fx.flame needs a finite position, width 0.05..4 m, height 0.1..8 m and lifetime 0.1..4 s"));
            }
            let mut c = lock(&core);
            sim(&c, "fx.flame")?;
            let time = c.tick as f32 / c.tick_rate;
            c.world.render_effects().flame(time, pos, width, height, life);
            Ok(())
        })),
        ("exhaust_smoke", f!(core, move |_, (pos, radius): (Value, f32)| {
            let pos = vec3(&pos, "fx.exhaust_smoke position")?;
            if !pos.is_finite() || !(0.1..=8.).contains(&radius) {
                return Err(rt("fx.exhaust_smoke needs a finite position and a radius in [0.1, 8] metres"));
            }
            let mut c = lock(&core);
            sim(&c, "fx.exhaust_smoke")?;
            let time = c.tick as f32 / c.tick_rate;
            c.world.render_effects().exhaust_smoke(time, pos, radius);
            Ok(())
        })),
        ("gib_mist", f!(core, move |_, (pos, intensity): (Value, Value)| {
            let pos = vec3(&pos, "fx.gib_mist position")?;
            let intensity = number(&intensity, "fx.gib_mist intensity")?;
            if !pos.is_finite() || !intensity.is_finite() || !(0.35..=2.8).contains(&intensity) {
                return Err(rt("fx.gib_mist needs a finite position and an intensity in [0.35, 2.8]"));
            }
            let mut c = lock(&core);
            sim(&c, "fx.gib_mist")?;
            let time = c.tick as f32 / c.tick_rate;
            c.world.render_effects().gib_mist(time, pos, intensity);
            Ok(())
        })),
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
        // Who looks through which part of the screen: {[player id] = view},
        // views numbered from 1. Players on the same view share its camera
        // (the first of them in players.list() is the one camera.set moves);
        // the screen splits into as many parts as there are views. Players
        // left out keep theirs. A game splits the screen when its players
        // walk apart and joins it again when they come together.
        ("views", f!(core, move |_, t: Table| {
            let mut c = lock(&core);
            for pair in t.pairs::<i64, i64>() {
                let (id, n) = pair?;
                let i = player(&c, Some(id))?;
                if n < 1 || n > 8 { return Err(rt(format!("camera.views: view {n} for player {id} (1 to 8)"))); }
                c.roster[i].view = Some(n as usize - 1);
            }
            // Number the views 0.. in order, without gaps.
            let mut used: Vec<usize> = c.roster.iter().filter_map(|p| p.view).collect();
            used.sort_unstable();
            used.dedup();
            for p in &mut c.roster { if let Some(v) = p.view { p.view = used.iter().position(|u| *u == v); } }
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
        ("move_to", f!(core, move |_, (h, at): (u32, Value)| {
            let pos = vec3(&at, "sound.move_to position")?;
            let c = lock(&core);
            sim(&c, "sound.move_to")?;
            c.sounds.move_to(h, pos.to_array());
            Ok(())
        })),
        ("stop", f!(core, move |_, h: u32| { lock(&core).sounds.stop(h); Ok(()) })),
        ("list", f!(core, move |lua, ()| { let c = lock(&core); let b = &c.sounds.bank; let mut names: Vec<String> = b.patches.keys().chain(b.sampled.keys()).cloned().collect(); names.sort(); names.dedup(); lua.create_sequence_from(names) })),
        ("has", f!(core, move |_, name: String| { let c = lock(&core); Ok(c.sounds.bank.has(&name)) })),
        ("recorded", f!(core, move |_, name: String| { let c = lock(&core); Ok(c.sounds.bank.is_sampled(&name)) })),
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
        ("face", f!(core, move |_, (id, x, y, size): (u32, f32, f32, f32)| {
            let mut c = lock(&core);
            let Some(p) = c.roster.iter_mut().find(|p| p.id == id) else { return Ok(false) };
            if p.bot { return Ok(false); }
            if p.face.is_none() { p.set_face("", ""); }
            let face = p.face.clone().expect("face");
            let [bx, by, bs, _] = face.bounds();
            let k = size / bs.max(1) as f32;
            for (r, col) in face.runs() {
                if r[0] + r[2] <= bx as f32 || r[0] >= (bx + bs) as f32 || r[1] < by as f32 || r[1] >= (by + bs) as f32 { continue; }
                let x0 = r[0].max(bx as f32);
                let x1 = (r[0] + r[2]).min((bx + bs) as f32);
                c.hud.push(HudItem::Box { rect: [x + (x0 - bx as f32) * k, y + (r[1] - by as f32) * k, (x1 - x0) * k + 0.5, k + 0.5], color: col });
            }
            Ok(true)
        })),
        ("size", f!(core, move |_, ()| { let s = lock(&core).screen; Ok((s[0], s[1])) })),
        ("measure", f!(core, move |_, (text, size): (Value, Option<f32>)| {
            let _ = &core;
            let text = match &text { Value::String(s) => s.to_string_lossy(), other => other.to_string()? };
            let [w, h] = render::hud::measure(&text, render::hud::FontFace::Body, size.unwrap_or(24.0).clamp(4.0, 400.0));
            Ok((w, h))
        })),
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
        ("set_time", f!(core, move |_, (hour, look): (f32, Option<String>)| {
            let mut c = lock(&core);
            sim(&c, "sky.set_time")?;
            if !hour.is_finite() { return Err(rt("sky.set_time needs a finite hour")); }
            let hour = hour.rem_euclid(24.);
            let weather = render::sky::WeatherLook::parse(look.as_deref().unwrap_or("clear")).map_err(rt)?;
            let previous = match c.sky {
                SkyChoice::Clock { hour: old, weather: w, .. }
                    if w == weather && ((hour-old+12.).rem_euclid(24.)-12.).abs() < 0.25 => old,
                _ => hour,
            };
            c.sky = SkyChoice::Clock { previous, hour, weather };
            Ok(())
        })),
        ("clouds", f!(core, move |_, value: Value| {
            let mut c = lock(&core);
            sim(&c, "sky.clouds")?;
            c.light.sky_volume = cloud_volume(value)?;
            Ok(())
        })),
        // Bake a sky the game will switch to later, in the background, so
        // the switch does not stall a frame (a bake is a few hundred ms).
        ("warm", f!(core, move |_, name: String| {
            let _ = &core;
            crate::game::warm_sky(SkyChoice::parse(&name).map_err(rt)?);
            Ok(())
        })),
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
        ("room", f!(core, move |_, (id, opts): (u32, Option<Table>)| {
            let mut c = lock(&core);
            sim(&c, "lights.room")?;
            if !c.rooms.enabled { return Err(rt("lights.room needs room_lighting = true in game.toml")); }
            let Some(t) = opts else { c.rooms.remove(id); return Ok(()); };
            options(&t, &["min", "max", "entities", "panels"], "lights.room")?;
            let min = vec3(&t.get("min")?, "room min")?;
            let max = vec3(&t.get("max")?, "room max")?;
            crate::room_lighting::grid(min, max).map_err(rt)?;
            let ids = t.get::<Table>("entities")?.sequence_values::<u32>()
                .take(c.world.capacity() + 1).collect::<mlua::Result<Vec<_>>>()?;
            if ids.is_empty() || ids.len() > c.world.capacity() || ids.iter().any(|id| c.world.get(*id).is_none()) {
                return Err(rt("lights.room needs live world entity IDs"));
            }
            let mut panels = Vec::new();
            if let Some(list) = t.get::<Option<Table>>("panels")? {
                for item in list.sequence_values::<Table>() {
                    let t = item?;
                    options(&t, &["pos", "color", "intensity", "range", "width", "depth"], "room panel")?;
                    let pos = vec3(&t.get("pos")?, "panel pos")?;
                    let rgb = color(&t.get("color")?, "room panel color")?;
                    let linear = Vec3::new(rgb[0],rgb[1],rgb[2]).powf(2.2);
                    let panel = crate::room_scene::panel(pos, linear, t.get("intensity")?, t.get("range")?, t.get("width")?, t.get("depth")?).map_err(rt)?;
                    panels.push(panel);
                    if panels.len() > 8 { return Err(rt("lights.room accepts at most eight ceiling panels")); }
                }
            }
            c.rooms.set(id, min, max, ids, panels).map_err(rt)
        })),
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
        ("spots", f!(core, move |_, list: Table| {
            let mut spots = Vec::new();
            for item in list.sequence_values::<Table>() {
                let t = item?;
                let mut l = Spot { pos: Vec3::ZERO, dir: Vec3::NEG_Y, color: [1.0; 3], intensity: 1000.0, range: 15.0, angle: 30f32.to_radians(), inner: -1.0 };
                for (k, v) in options(&t, &["pos", "dir", "color", "intensity", "range", "angle", "inner"], "lights.spots")? {
                    match k.as_str() {
                        "pos" => l.pos = vec3(&v, "spot pos")?,
                        "dir" => l.dir = vec3(&v, "spot dir")?,
                        "color" => { let c = color(&v, "spot color")?; l.color = [c[0], c[1], c[2]]; }
                        "intensity" => l.intensity = number(&v, "intensity")?.clamp(0.0, 1_000_000.0),
                        "range" => l.range = number(&v, "range")?.clamp(1.0, 500.0),
                        "angle" => l.angle = number(&v, "angle")?.clamp(2.0, 74.0).to_radians(),
                        "inner" => l.inner = number(&v, "inner")?.max(0.0).to_radians(),
                        _ => unreachable!(),
                    }
                }
                if l.dir.length_squared() < 1e-8 { return Err(rt("lights.spots: dir must not be zero".to_string())); }
                l.dir = l.dir.normalize();
                l.inner = if l.inner < 0.0 { l.angle * 0.6 } else { l.inner.min(l.angle * 0.98) };
                if spots.len() == MAX_SPOTS { return Err(rt(format!("lights.spots: at most {MAX_SPOTS} spots"))); }
                spots.push(l);
            }
            lock(&core).spots = spots;
            Ok(())
        })),
        ("clear", f!(core, move |_, ()| { let mut c = lock(&core); c.lamps.clear(); c.spots.clear(); Ok(()) })),
    ])?;

    table("humans", vec![
        ("spawn", f!(core, move |_, (pool, opts): (String, Option<Table>)| {
            let mut c = lock(&core);
            sim(&c, "humans.spawn")?;
            let (mut pos, mut yaw, mut look, mut resident) = (Vec3::ZERO, 0.0, None, None);
            if let Some(t) = opts {
                for (k, v) in options(&t, &["pos", "yaw", "look", "resident"], "humans.spawn")? {
                    match k.as_str() {
                        "pos" => pos = vec3(&v, "humans.spawn pos")?,
                        "yaw" => yaw = number(&v, "humans.spawn yaw")?,
                        "look" => look = Some(t.raw_get::<usize>("look")?),
                        "resident" => resident = Some(t.raw_get::<usize>("resident")?),
                        _ => unreachable!(),
                    }
                }
            }
            c.humans.spawn_with_look(&pool, pos, yaw, look, resident).map_err(rt)
        })),
        ("set", f!(core, move |_, (id, opts): (u32, Table)| {
            let mut c = lock(&core);
            sim(&c, "humans.set")?;
            let o = options(&opts, &["pos", "yaw", "pose", "t", "tired", "coil", "visible", "grip", "support", "pitch"], "humans.set")?;
            let Some(p) = c.humans.people.get_mut(&id) else { return Ok(false) };
            let mut up = false;
            for (k, v) in o {
                match k.as_str() {
                    "pos" => p.pos = vec3(&v, "humans.set pos")?,
                    "visible" => p.set_visible(boolean(&v, "humans.set visible")?),
                    "yaw" => p.yaw = number(&v, "humans.set yaw")?,
                    "t" => p.t = number(&v, "humans.set t")?.clamp(0.0, 1.0),
                    "grip" => p.hold.grip = vec3(&v, "humans.set grip")?,
                    "support" => p.hold.support = vec3(&v, "humans.set support")?,
                    "pitch" => p.hold.pitch = number(&v, "humans.set pitch")?.clamp(-1.45, 1.45),
                    "tired" => p.tired = number(&v, "humans.set tired")?.clamp(0.0, 1.0),
                    "coil" => p.coil = number(&v, "humans.set coil")?.clamp(-1.0, 1.0),
                    "pose" => {
                        let name = match &v { Value::String(s) => s.to_str()?.to_string(), _ => return Err(rt("humans.set pose: expected a string".to_string())) };
                        if !crate::humans::POSES.contains(&name.as_str()) { return Err(rt(format!("humans.set: pose {name:?} (one of {})", crate::humans::POSES.join(", ")))); }
                        up |= !matches!(name.as_str(), "dead" | "down");
                        p.pose = name;
                    }
                    _ => unreachable!(),
                }
            }
            if up { c.humans.stand(id); }
            Ok(true)
        })),
        ("obstacles", f!(core, move |_, list: Table| {
            let mut c = lock(&core);
            sim(&c, "humans.obstacles")?;
            let mut boxes = std::collections::BTreeMap::new();
            for item in list.sequence_values::<Table>() {
                let t = item?;
                let id = t.raw_get::<u32>("id")?;
                let pos = vec3(&t.raw_get("pos")?, "obstacle pos")?;
                let size = vec3(&t.raw_get("size")?, "obstacle size")?;
                let yaw = t.raw_get::<Option<f32>>("yaw")?.unwrap_or(0.);
                let previous = t.raw_get::<Option<Value>>("prevPos")?.map(|v| vec3(&v, "obstacle prevPos")).transpose()?.unwrap_or(pos);
                let previous_yaw = t.raw_get::<Option<f32>>("prevYaw")?.unwrap_or(yaw);
                if !pos.is_finite() || !previous.is_finite() || !size.is_finite() || size.min_element() <= 0.
                    || !yaw.is_finite() || !previous_yaw.is_finite() { return Err(rt("humans.obstacles needs finite poses and positive box sizes")); }
                let shape = kern::collide::YawBox::new(pos, size, yaw);
                let vehicle = t.raw_get::<Option<u32>>("vehicle")?;
                let profile = t.raw_get::<Option<String>>("profile")?;
                if vehicle.is_none() && profile.is_some() { return Err(rt("humans.obstacles profile needs a vehicle")); }
                let chassis = vehicle.map(|car| {
                    c.world.fleet.as_ref().and_then(|fleet| fleet.chassis_contact(car, profile.as_deref()))
                        .ok_or_else(|| rt(format!("humans.obstacles: unknown vehicle {car} or contact profile")))
                }).transpose()?;
                if boxes.insert(id, crate::ragdoll::Obstacle { shape, previous, previous_yaw, chassis }).is_some() {
                    return Err(rt("humans.obstacles needs a distinct id for each box"));
                }
                if boxes.len() > crate::ragdoll::MAX_OBSTACLES { return Err(rt("humans.obstacles takes up to 64 nearby boxes")); }
            }
            c.humans.set_obstacles(boxes);
            Ok(())
        })),
        ("recover", f!(core, move |_, id: u32| {
            let mut c = lock(&core);
            sim(&c, "humans.recover")?;
            Ok(c.humans.recover(id).map(vector).unwrap_or(Value::Nil))
        })),
        ("ragdoll_raycast", f!(core, move |_, (id, origin, dir, limit): (u32, Value, Value, f32)| {
            let origin = vec3(&origin, "ragdoll ray origin")?;
            let dir = vec3(&dir, "ragdoll ray direction")?;
            if !origin.is_finite() || !dir.is_finite() || !limit.is_finite() || limit < 0. {
                return Err(rt("humans.ragdoll_raycast needs a finite ray and nonnegative range"));
            }
            Ok(lock(&core).humans.ragdoll_raycast(id, origin, dir, limit))
        })),
        ("ragdoll_closest", f!(core, move |_, (id, point): (u32, Value)| {
            let point = vec3(&point, "ragdoll query position")?;
            if !point.is_finite() { return Err(rt("humans.ragdoll_closest needs a finite position")); }
            Ok(lock(&core).humans.ragdoll_closest(id, point).map(vector).unwrap_or(Value::Nil))
        })),
        ("ragdoll_hit", f!(core, move |_, (id, point, velocity): (u32, Value, Value)| {
            let point = vec3(&point, "ragdoll hit position")?;
            let velocity = vec3(&velocity, "ragdoll hit velocity")?;
            if !point.is_finite() || !velocity.length_squared().is_finite() { return Err(rt("humans.ragdoll_hit needs finite position and velocity")); }
            let mut c = lock(&core);
            sim(&c, "humans.ragdoll_hit")?;
            c.humans.ragdoll_hit(id, point, velocity).map_err(rt)
        })),
        ("ragdoll_impact", f!(core, move |_, (id, point, kick, carried): (u32, Value, Value, Option<Value>)| {
            let point = vec3(&point, "ragdoll contact")?;
            let kick = vec3(&kick, "ragdoll impact velocity")?;
            let carried = carried.map(|v| vec3(&v, "ragdoll carried velocity")).transpose()?;
            let mut c = lock(&core);
            sim(&c, "humans.ragdoll_impact")?;
            let rate = c.tick_rate;
            c.humans.ragdoll_impact(id, point, kick, carried, rate).map_err(rt)
        })),
        ("ragdoll", f!(core, move |_, (id, push, spin): (u32, Option<Value>, Option<Value>)| {
            let mut c = lock(&core);
            sim(&c, "humans.ragdoll")?;
            let push = match push { Some(v) => vec3(&v, "humans.ragdoll push")?, None => Vec3::ZERO };
            let spin = spin.map(|v| vec3(&v, "humans.ragdoll spin")).transpose()?;
            c.humans.ragdoll_with_motion(id, push, spin).map_err(rt)
        })),
        ("remove", f!(core, move |_, id: u32| { let mut c = lock(&core); sim(&c, "humans.remove")?; Ok(c.humans.remove(id)) })),
        ("hit", f!(core, move |_, (id, dir): (u32, Value)| {
            let mut c = lock(&core);
            sim(&c, "humans.hit")?;
            let d = vec3(&dir, "humans.hit dir")?;
            Ok(c.humans.hit(id, d))
        })),
        ("head", f!(core, move |_, id: u32| Ok(lock(&core).humans.head(id).map(vector).unwrap_or(Value::Nil)))),
        ("joint", f!(core, move |_, (id, name): (u32, String)| {
            if !crate::humans::JOINT_NAMES.iter().any(|(n, _)| *n == name) {
                return Err(rt(format!("humans.joint: {name:?} (one of {})", crate::humans::JOINT_NAMES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "))));
            }
            Ok(lock(&core).humans.joint(id, &name).map(vector).unwrap_or(Value::Nil))
        })),
        ("room", f!(core, move |_, (pool, body): (String, Option<usize>)| {
            let c = lock(&core);
            let (free, all) = match body { Some(body) => c.humans.body_room(&pool, body), None => c.humans.room(&pool) }
                .ok_or_else(|| rt(format!("humans.room: unknown pool or body {pool:?} {body:?}")))?;
            let (_, bodies, looks) = c.humans.distant_limits(&pool).unwrap();
            Ok((free, all, bodies, looks))
        })),
        ("hide_distant", f!(core, move |_, (pool, id): (String, usize)| {
            let mut c = lock(&core);
            sim(&c, "humans.hide_distant")?;
            Ok(c.humans.hide_distant(&pool, id))
        })),
        ("distant", f!(core, move |_, (pool, rows, fitted, seconds): (String, Table, Table, f32)| {
            let mut c = lock(&core);
            sim(&c, "humans.distant")?;
            let (limit, _, variety) = c.humans.distant_limits(&pool).ok_or_else(|| rt(format!("humans.distant: no pool {pool:?}")))?;
            let count = rows.raw_len();
            if count > limit { return Err(rt(format!("humans.distant: {count} exceeds the {limit} reserved slots"))); }
            if !seconds.is_finite() || seconds <= 0. { return Err(rt("humans.distant: expected a positive interval".to_string())); }
            let mut samples = Vec::with_capacity(count);
            for id in 1..=count {
                let row: Table = rows.raw_get(id)?;
                let pos = vec3(&row.raw_get("pos")?, "humans.distant pos")?;
                let yaw = row.raw_get::<Option<f32>>("yaw")?.unwrap_or(0.);
                let next = match row.raw_get::<Value>("nextPos")? {
                    Value::Nil => pos, v => vec3(&v, "humans.distant nextPos")?,
                };
                let look = row.raw_get::<Option<usize>>("look")?.unwrap_or((id - 1) % variety);
                if look >= variety { return Err(rt(format!("humans.distant: look outside 0..{variety}"))); }
                let next_yaw = row.raw_get::<Option<f32>>("nextYaw")?.unwrap_or(yaw);
                if !pos.is_finite() || !next.is_finite() || !yaw.is_finite() || !next_yaw.is_finite() {
                    return Err(rt(format!("humans.distant: resident {id} needs finite positions and headings")));
                }
                samples.push(crate::distant_humans::Sample {
                    pos, yaw, next, look, next_yaw,
                    active: row.raw_get::<Option<bool>>("active")?.unwrap_or(true),
                    fitted: fitted.raw_get::<Option<u32>>(id)?.is_some(),
                });
            }
            c.humans.set_distant(&pool, samples, seconds);
            Ok(())
        })),
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
