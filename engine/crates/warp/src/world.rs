//! The entities a Luau game spawns, their collision boxes and the scene that
//! shows them. Simulation data only: no GPU here, so it runs in tests and
//! `warp check`. The scene is uploaded by the game's `draw`.
//!
//! Every entity sits in a [`Slots`] block reserved up front, so spawning,
//! moving and removing never rebuild the renderer. A shape and look (colour,
//! emissive, roughness, metal) seen for the first time adds a material and a
//! mesh, which needs a new renderer on the next frame (`rebuild`): free
//! during `init`, a short stall later.
use glam::{Affine3A, EulerRot, Quat, Vec3};
use kern::collide::{Mover, YawBox};
use render::primitives;
use render::scene::{Material, Scene, EMISSIVE, SURFACE};
use render::slots::{SlotId, Slots};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

/// Emissive 1 means this many nits: a clear glow in daylight.
pub const EMISSIVE_NITS: f32 = 4000.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Cube,
    Sphere,
    Capsule,
    /// A hollow regular polygon in the XZ plane (`outline6`, `outline4:0.05`):
    /// corners on a 1 m circle, the edge `width` thousandths of a metre at size 1.
    Outline { sides: u32, width: u32 },
    /// A glTF file inside the game folder (`assets/crate.glb`).
    Model(String),
    /// Glowing edges between points (the entity's `wire`): one thin box per
    /// edge, moved by the engine whenever the entity or its points move.
    Wire,
}

/// A wire frame in the entity's own space (scaled by its size, turned by
/// its rotation): points, edges between them (0-based), and the edge width
/// in metres at size 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub points: Vec<Vec3>,
    pub edges: Vec<[u16; 2]>,
    pub width: f32,
}

impl Wire {
    /// The box around the points, as a centre and half extents.
    fn bounds(&self) -> (Vec3, Vec3) {
        if self.points.is_empty() { return (Vec3::ZERO, Vec3::splat(0.5)); }
        let (lo, hi) = self.points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        ((lo + hi) * 0.5, ((hi - lo) * 0.5 + self.width * 0.5).max(Vec3::splat(1e-3)))
    }
}

impl Shape {
    pub fn name(&self) -> String {
        match self { Shape::Cube => "cube".into(), Shape::Sphere => "sphere".into(), Shape::Capsule => "capsule".into(), Shape::Outline { sides, width } => format!("outline{sides}:{}", *width as f32 / 1000.0), Shape::Model(p) => p.clone(), Shape::Wire => "wire".into() }
    }
}

/// Surface: colour in sRGB 0..1 (as a colour picker shows it), emissive
/// glow (0 off, 1 bright in daylight), roughness and metalness 0..1.
/// `surface` picks one of the game's own surfaces in `surface.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub color: [f32; 3],
    pub emissive: f32,
    pub roughness: f32,
    pub metal: f32,
    pub surface: Option<u8>,
}

impl Default for Look {
    fn default() -> Self { Look { color: [0.8, 0.8, 0.8], emissive: 0.0, roughness: 0.7, metal: 0.0, surface: None } }
}

impl Look {
    fn key(&self) -> [i32; 7] {
        let q = |v: f32| (v.clamp(0.0, 1000.0) * 1000.0).round() as i32;
        [q(self.color[0]), q(self.color[1]), q(self.color[2]), q(self.emissive), q(self.roughness), q(self.metal), self.surface.map_or(-1, i32::from)]
    }
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub shape: Shape,
    pub look: Look,
    pub pos: Vec3,
    /// Euler angles in radians: x pitch, y yaw, z roll (applied yaw, pitch, roll).
    pub rot: Vec3,
    /// Size in metres along each axis. Every shape fills a 1 m box at size 1,
    /// except the capsule, which is 2 m tall.
    pub size: Vec3,
    /// Solid entities block `world.move` and are hit by `world.raycast`.
    pub solid: bool,
    pub visible: bool,
    pub tag: String,
    /// Shape::Wire: its points and edges.
    pub wire: Option<Wire>,
    parent: Option<u32>,
    children: Vec<u32>,
    // The parent's resolved rotation and position. Size is a mesh dimension in metres.
    frame: Option<(Quat, Vec3)>,
    slot: Option<SlotId>,
    /// A wire's slots, one per edge.
    edge_slots: Vec<SlotId>,
    mesh: usize,
    /// Collision box at size 1: centre offset and half extents.
    local: (Vec3, Vec3),
}

impl Entity {
    /// The collision box: rotated about Y only (pitch and roll are ignored).
    pub fn bounds(&self) -> YawBox {
        let (offset, half) = self.local;
        let (pos, yaw) = match self.frame {
            None => (self.pos, self.rot.y),
            Some(_) => {
                let (rot, pos) = self.world_pose();
                (pos, rot.to_euler(EulerRot::YXZ).0)
            }
        };
        let center = pos + Quat::from_rotation_y(yaw) * (offset * self.size);
        YawBox { center, half: half * self.size.abs(), yaw }
    }

    pub fn parent(&self) -> Option<u32> { self.parent }

    pub fn world_pose(&self) -> (Quat, Vec3) {
        let local = Quat::from_euler(EulerRot::YXZ, self.rot.y, self.rot.x, self.rot.z);
        match self.frame {
            Some((rot, pos)) => ((rot * local).normalize(), pos + rot * self.pos),
            None => (local, self.pos),
        }
    }

    pub fn local_delta(&self, world: Vec3) -> Vec3 {
        self.frame.map_or(world, |(rot, _)| rot.conjugate() * world)
    }

    fn transform(&self) -> Affine3A {
        let (rotation, position) = self.world_pose();
        Affine3A::from_scale_rotation_translation(self.size, rotation, position)
    }
}

/// What `world.spawn` asks for.
#[derive(Clone, Debug)]
pub struct Spawn {
    pub parent: Option<u32>,
    pub shape: Shape,
    pub look: Look,
    pub pos: Vec3,
    pub rot: Vec3,
    pub size: Vec3,
    pub solid: bool,
    pub visible: bool,
    pub tag: String,
    pub wire: Option<Wire>,
}

impl Default for Spawn {
    fn default() -> Self { Spawn { parent: None, shape: Shape::Cube, look: Look::default(), pos: Vec3::ZERO, rot: Vec3::ZERO, size: Vec3::ONE, solid: true, visible: true, tag: String::new(), wire: None } }
}

/// Put an entity's slots where it is: one for a shape, one per edge for a
/// wire (as many as fit), none while hidden.
fn show(slots: &mut Slots, scene: &mut Scene, e: &mut Entity) {
    let Some(w) = &e.wire else {
        match (e.visible, e.slot) {
            (true, Some(slot)) => slots.set(scene, slot, e.mesh, e.transform()),
            (true, None) => e.slot = slots.add(scene, e.mesh, e.transform()),
            (false, Some(slot)) => { slots.remove(scene, slot); e.slot = None; }
            (false, None) => {}
        }
        return;
    };
    let want = if e.visible { w.edges.len() } else { 0 };
    while e.edge_slots.len() > want { slots.remove(scene, e.edge_slots.pop().unwrap()); }
    let xf = e.transform();
    let width = w.width * e.size.abs().max_element();
    for (i, [a, b]) in w.edges.iter().take(want).enumerate() {
        let (Some(a), Some(b)) = (w.points.get(*a as usize), w.points.get(*b as usize)) else { continue };
        let (a, b) = (xf.transform_point3(*a), xf.transform_point3(*b));
        let d = b - a;
        let length = d.length();
        let turn = if length > 1e-6 { Quat::from_rotation_arc(Vec3::Z, d / length) } else { Quat::IDENTITY };
        let edge = Affine3A::from_scale_rotation_translation(Vec3::new(width, width, length.max(1e-4)), turn, (a + b) * 0.5);
        match e.edge_slots.get(i) {
            Some(&slot) => slots.set(scene, slot, e.mesh, edge),
            None => match slots.add(scene, e.mesh, edge) { Some(slot) => e.edge_slots.push(slot), None => break },
        }
    }
}

/// A ray's first hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit { pub id: u32, pub distance: f32, pub point: Vec3, pub normal: Vec3 }

/// Which entities a query sees.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub tag: Option<String>,
    pub ignore: Vec<u32>,
    /// Include entities that are not solid.
    pub all: bool,
}

impl Filter {
    fn wants(&self, id: u32, e: &Entity, solid_only: bool) -> bool {
        (!solid_only || self.all || e.solid) && !self.ignore.contains(&id) && self.tag.as_ref().is_none_or(|t| &e.tag == t)
    }
}

struct ModelInfo { mesh: usize, min: Vec3, max: Vec3 }

pub struct World {
    pub entities: BTreeMap<u32, Entity>,
    next: u32,
    pub scene: Scene,
    slots: Slots,
    materials: HashMap<[i32; 7], u32>,
    meshes: HashMap<(usize, u32), usize>,
    models: HashMap<String, ModelInfo>,
    /// Meshes or materials were added since the renderer was made.
    pub rebuild: bool,
    /// The shapes and looks that caused it (for the log).
    pub added: Vec<String>,
    dir: PathBuf,
    fx: Fx,
}

/// Effects the engine animates on its own (`fx.burst`, `fx.ring`): plain
/// entities tagged `fx`, moved every tick and removed when spent. Looks
/// only; they never collide. When the world is full, effects are skipped.
#[derive(Default)]
struct Fx { parts: Vec<Part>, rings: Vec<Ring>, rng: u64 }

struct Part { id: u32, vel: Vec3, gravity: f32, age: f32, life: f32, size: Vec3, stretch: f32 }

struct Ring { ids: Vec<u32>, center: Vec3, radius: f32, width: f32, age: f32, life: f32 }

/// A particle burst: `count` pieces flying out from `pos`.
#[derive(Clone, Debug)]
pub struct Burst {
    pub shape: Shape,
    pub look: Look,
    pub count: usize,
    /// Fastest speed in m/s; each piece gets 25 % to 100 % of it.
    pub speed: f32,
    /// Longest life in seconds; each piece lives 50 % to 100 % of it.
    pub life: f32,
    pub size: f32,
    /// Seconds of velocity as extra length along the flight (streaks).
    pub stretch: f32,
    pub gravity: f32,
    /// Fly out in the ground plane only.
    pub flat: bool,
}

impl Default for Burst {
    fn default() -> Self { Burst { shape: Shape::Cube, look: Look { emissive: 0.06, ..Look::default() }, count: 12, speed: 4.0, life: 0.4, size: 0.12, stretch: 0.0, gravity: 0.0, flat: false } }
}

/// A ring that grows to `radius` in the ground plane and thins out.
#[derive(Clone, Debug)]
pub struct RingFx { pub look: Look, pub radius: f32, pub life: f32, pub width: f32, pub segments: usize }

impl Default for RingFx {
    fn default() -> Self { RingFx { look: Look { emissive: 0.06, ..Look::default() }, radius: 2.0, life: 0.35, width: 0.08, segments: 24 } }
}

impl World {
    /// `dir` is the game folder (models load from inside it).
    pub fn new(dir: &Path, max_entities: usize) -> Self {
        let mut scene = Scene::default();
        let m = scene.add_material(material(&Look::default()));
        let cube = scene.add_mesh(&primitives::cube(m).mesh());
        let slots = Slots::reserve(&mut scene, max_entities);
        let mut w = World { entities: BTreeMap::new(), next: 1, scene, slots, materials: HashMap::new(), meshes: HashMap::new(), models: HashMap::new(), rebuild: true, added: Vec::new(), dir: dir.to_path_buf(), fx: Fx::default() };
        w.materials.insert(Look::default().key(), m);
        w.meshes.insert((0, m), cube);
        w
    }

    pub fn capacity(&self) -> usize { self.slots.capacity() }

    /// The game folder.
    pub fn dir(&self) -> &Path { &self.dir }

    /// Build the scene again from the entities alone: what attaching
    /// vegetation needs (it must come last, and a scene cannot drop it).
    /// Ids, effects and every entity stay; the renderer is rebuilt.
    pub fn fresh_scene(&mut self) -> Result<(), String> {
        let mut w = World::new(&self.dir, self.slots.capacity());
        w.scene.style = self.scene.style.clone();
        for (&id, e) in &self.entities {
            let (mesh, _) = w.mesh(&e.shape, &e.look)?;
            let mut e = e.clone();
            e.mesh = mesh;
            e.slot = None;
            e.edge_slots.clear();
            show(&mut w.slots, &mut w.scene, &mut e);
            w.entities.insert(id, e);
        }
        self.entities = std::mem::take(&mut w.entities);
        self.scene = w.scene;
        self.slots = w.slots;
        self.materials = w.materials;
        self.meshes = w.meshes;
        self.models = w.models;
        self.rebuild = true;
        Ok(())
    }

    fn material(&mut self, look: &Look) -> u32 {
        if let Some(&m) = self.materials.get(&look.key()) { return m; }
        let m = self.scene.add_material(material(look));
        self.materials.insert(look.key(), m);
        self.rebuild = true;
        m
    }

    /// The mesh and unit collision box for a shape and look.
    fn mesh(&mut self, shape: &Shape, look: &Look) -> Result<(usize, (Vec3, Vec3)), String> {
        let half = Vec3::splat(0.5);
        let kind = match shape {
            Shape::Cube | Shape::Wire => 0,
            Shape::Sphere => 1,
            Shape::Capsule => 2,
            Shape::Outline { sides, width } => 3 + (*sides as usize) * 1000 + *width as usize,
            Shape::Model(path) => {
                if !self.models.contains_key(path) {
                    let file = self.asset(path)?;
                    let model = assets::gltf::load(&mut self.scene, &file)?;
                    self.models.insert(path.clone(), ModelInfo { mesh: model.mesh, min: model.min, max: model.max });
                    self.rebuild = true;
                }
                let m = &self.models[path];
                return Ok((m.mesh, ((m.min + m.max) * 0.5, ((m.max - m.min) * 0.5).max(Vec3::splat(1e-3)))));
            }
        };
        let local = (Vec3::ZERO, if kind == 2 { Vec3::new(0.5, 1.0, 0.5) } else { half });
        let m = self.material(look);
        if let Some(&mesh) = self.meshes.get(&(kind, m)) { return Ok((mesh, local)); }
        let c = look.color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        self.added.push(format!("{} #{:02x}{:02x}{:02x} emissive {}", shape.name(), c[0], c[1], c[2], look.emissive));
        let mut p = match kind {
            0 => primitives::cube(m),
            1 => primitives::sphere(m, 16, 24),
            // Radius 1, 2 m straight middle: halved below to 1 m wide, 2 m tall.
            2 => primitives::capsule(m, 2.0, 12, 20),
            _ => match shape {
                Shape::Outline { sides, width } => primitives::outline(m, *sides, *width as f32 / 1000.0),
                _ => unreachable!(),
            },
        };
        if kind == 1 || kind == 2 { for v in &mut p.vertices { v.pos = v.pos.map(|c| c * 0.5); } }
        let mesh = self.scene.add_mesh(&p.mesh());
        self.meshes.insert((kind, m), mesh);
        self.rebuild = true;
        Ok((mesh, local))
    }

    /// A path inside the game folder: relative, no `..`.
    fn asset(&self, path: &str) -> Result<PathBuf, String> {
        let rel = Path::new(path);
        if rel.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
            return Err(format!("{path}: model paths are relative to the game folder, without '..'"));
        }
        let file = self.dir.join(rel);
        if !file.exists() { return Err(format!("{path}: no such file in the game folder")); }
        Ok(file)
    }

    pub fn spawn(&mut self, s: Spawn) -> Result<u32, String> {
        if self.entities.len() >= self.slots.capacity() { return Err(format!("too many entities: {} at most (max_entities in game.toml)", self.slots.capacity())); }
        if s.shape == Shape::Wire && s.wire.is_none() { return Err("wire: needs points and edges".into()); }
        let frame = s.parent.map(|id| self.entities.get(&id).map(Entity::world_pose)
            .ok_or_else(|| format!("no parent entity {id}"))).transpose()?;
        let (mesh, mut local) = self.mesh(&s.shape, &s.look)?;
        if let Some(w) = &s.wire { local = w.bounds(); }
        let id = self.next;
        self.next += 1;
        let mut e = Entity { parent: s.parent, frame, children: Vec::new(), shape: s.shape, look: s.look, pos: s.pos, rot: s.rot, size: s.size, solid: s.solid, visible: s.visible, tag: s.tag, wire: s.wire, slot: None, edge_slots: Vec::new(), mesh, local };
        show(&mut self.slots, &mut self.scene, &mut e);
        if let Some(parent) = s.parent { self.entities.get_mut(&parent).unwrap().children.push(id); }
        self.entities.insert(id, e);
        Ok(id)
    }

    pub fn get(&self, id: u32) -> Option<&Entity> { self.entities.get(&id) }

    /// Change an entity; `f` edits it, then its mesh and slot follow.
    pub fn update(&mut self, id: u32, f: impl FnOnce(&mut Entity)) -> Result<(), String> {
        self.try_update(id, |e| { f(e); Ok(()) })
    }

    /// Stage fallible edits before publishing a changed transform to its children.
    pub fn try_update(&mut self, id: u32, f: impl FnOnce(&mut Entity) -> Result<(), String>) -> Result<(), String> {
        let old = self.entities.get(&id).ok_or_else(|| format!("no entity {id} (removed, or never spawned)"))?;
        let mut e = old.clone();
        f(&mut e)?;
        if (e.shape == Shape::Wire) != (old.shape == Shape::Wire) {
            return Err("shape: a wire stays a wire, and other shapes cannot become one (spawn a new entity)".into());
        }
        let moved_children = !e.children.is_empty() && (e.pos != old.pos || e.rot != old.rot);
        if e.shape != old.shape || e.look != old.look {
            (e.mesh, e.local) = self.mesh(&e.shape, &e.look)?;
        }
        if let Some(w) = &e.wire { e.local = w.bounds(); }
        show(&mut self.slots, &mut self.scene, &mut e);
        self.entities.insert(id, e);
        if moved_children { self.refresh_children(id); }
        Ok(())
    }

    /// Keep local position and rotation when changing parent. Pass None to detach.
    pub fn set_parent(&mut self, id: u32, parent: Option<u32>) -> Result<(), String> {
        let old = self.entities.get(&id).ok_or_else(|| format!("no entity {id}"))?.parent;
        if old == parent { return Ok(()); }
        let frame = if let Some(p) = parent {
            let e = self.entities.get(&p).ok_or_else(|| format!("no parent entity {p}"))?;
            if kern::parenting::ancestors(p, |i| self.entities[&i].parent).any(|i| i == id) {
                return Err("an entity cannot parent itself or an ancestor".into());
            }
            Some(e.world_pose())
        } else { None };
        if let Some(p) = old { self.entities.get_mut(&p).unwrap().children.retain(|&i| i != id); }
        if let Some(p) = parent { self.entities.get_mut(&p).unwrap().children.push(id); }
        let e = self.entities.get_mut(&id).unwrap();
        e.parent = parent;
        e.frame = frame;
        show(&mut self.slots, &mut self.scene, e);
        if !e.children.is_empty() { self.refresh_children(id); }
        Ok(())
    }

    fn refresh_children(&mut self, id: u32) {
        let mut pending = vec![id];
        while let Some(i) = pending.pop() {
            let e = &self.entities[&i];
            if e.children.is_empty() { continue; }
            let frame = e.world_pose();
            let start = pending.len();
            pending.extend_from_slice(&e.children);
            for child in &pending[start..] {
                let e = self.entities.get_mut(child).unwrap();
                e.frame = Some(frame);
                show(&mut self.slots, &mut self.scene, e);
            }
        }
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let Some(root) = self.entities.remove(&id) else { return false };
        if let Some(p) = root.parent { self.entities.get_mut(&p).unwrap().children.retain(|&i| i != id); }
        let mut pending = Vec::new();
        let mut next = Some(root);
        while let Some(e) = next {
            pending.extend(e.children);
            if let Some(slot) = e.slot { self.slots.remove(&mut self.scene, slot); }
            for slot in e.edge_slots { self.slots.remove(&mut self.scene, slot); }
            next = pending.pop().and_then(|i| self.entities.remove(&i));
        }
        true
    }

    pub fn clear(&mut self) {
        let ids: Vec<u32> = self.entities.keys().copied().collect();
        for id in ids { self.remove(id); }
        self.fx = Fx::default();
    }

    /// Add a shape and look's mesh now (in `init`), so spawning it later
    /// never stalls on a new renderer.
    pub fn warm(&mut self, shape: &Shape, look: &Look) -> Result<(), String> { self.mesh(shape, look).map(|_| ()) }

    /// Move an entity without touching anything else (the hot path).
    pub fn place(&mut self, id: u32, pos: Vec3, yaw: Option<f32>) -> Result<(), String> {
        self.transform(id, |e| { e.pos = pos; if let Some(y) = yaw { e.rot.y = y; } })
    }

    /// Change only where an entity is, how it is turned and its size.
    fn transform(&mut self, id: u32, f: impl FnOnce(&mut Entity)) -> Result<(), String> {
        let e = self.entities.get_mut(&id).ok_or_else(|| format!("no entity {id} (removed, or never spawned)"))?;
        f(e);
        if e.wire.is_some() { show(&mut self.slots, &mut self.scene, e); }
        else if let Some(slot) = e.slot { let (mesh, t) = (e.mesh, e.transform()); self.slots.set(&mut self.scene, slot, mesh, t); }
        if !e.children.is_empty() { self.refresh_children(id); }
        Ok(())
    }

    fn fx_float(&mut self) -> f32 {
        let mut r = crate::api::Rng(self.fx.rng);
        let x = r.float() as f32;
        self.fx.rng = r.0;
        x
    }

    fn room(&self) -> usize { self.slots.capacity().saturating_sub(self.entities.len()) }

    pub fn burst(&mut self, pos: Vec3, b: &Burst) -> Result<(), String> {
        self.warm(&b.shape, &b.look)?;
        for _ in 0..b.count.min(self.room()) {
            let yaw = self.fx_float() * std::f32::consts::TAU;
            let up = if b.flat { 0.0 } else { self.fx_float() * 2.0 - 1.0 };
            let flat = (1.0 - up * up).sqrt();
            let speed = b.speed * (0.25 + 0.75 * self.fx_float());
            let vel = Vec3::new(yaw.cos() * flat, up, yaw.sin() * flat) * speed;
            let life = b.life * (0.5 + 0.5 * self.fx_float());
            let size = Vec3::splat(b.size);
            let id = self.spawn(Spawn { shape: b.shape.clone(), look: b.look, pos, size, solid: false, tag: "fx".into(), ..Default::default() })?;
            self.fx.parts.push(Part { id, vel, gravity: b.gravity, age: 0.0, life, size, stretch: b.stretch });
        }
        Ok(())
    }

    pub fn ring(&mut self, center: Vec3, r: &RingFx) -> Result<(), String> {
        let n = r.segments.clamp(3, 96);
        if self.room() < n { return Ok(()); }
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n { ids.push(self.spawn(Spawn { look: r.look, pos: center, size: Vec3::ZERO, solid: false, tag: "fx".into(), ..Default::default() })?); }
        self.fx.rings.push(Ring { ids, center, radius: r.radius, width: r.width, age: 0.0, life: r.life.max(0.01) });
        self.place_ring(self.fx.rings.len() - 1);
        Ok(())
    }

    /// LÖVE Neon Siege's ring: from an eighth of the radius out to all of it.
    fn place_ring(&mut self, i: usize) {
        let ring = &self.fx.rings[i];
        let k = (ring.age / ring.life).min(1.0);
        let radius = ring.radius * (0.125 + 0.875 * k);
        let n = ring.ids.len();
        let len = std::f32::consts::TAU * radius / n as f32 * 1.08;
        let width = ring.width * (1.0 - k * 0.8);
        let (center, ids) = (ring.center, ring.ids.clone());
        for (j, id) in ids.into_iter().enumerate() {
            let a = j as f32 / n as f32 * std::f32::consts::TAU;
            let pos = center + Vec3::new(a.cos(), 0.0, a.sin()) * radius;
            // A segment's length runs along the tangent (its local X turned by -a).
            let _ = self.transform(id, |e| { e.pos = pos; e.rot.y = -a + std::f32::consts::FRAC_PI_2; e.size = Vec3::new(len, width, width); });
        }
    }

    /// Animate the effects one tick.
    pub fn step_fx(&mut self, dt: f32) {
        let mut parts = std::mem::take(&mut self.fx.parts);
        parts.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life || !self.entities.contains_key(&p.id) { self.remove(p.id); return false; }
            p.vel.y -= p.gravity * dt;
            let k = 1.0 - p.age / p.life;
            let (vel, size, stretch) = (p.vel, p.size * k, p.stretch);
            let _ = self.transform(p.id, |e| {
                e.pos += vel * dt;
                e.size = size;
                if stretch > 0.0 {
                    let flat = Vec3::new(vel.x, 0.0, vel.z);
                    e.size.z += flat.length() * stretch;
                    e.rot.y = (-vel.x).atan2(-vel.z);
                }
            });
            true
        });
        self.fx.parts = parts;
        let mut i = 0;
        while i < self.fx.rings.len() {
            self.fx.rings[i].age += dt;
            if self.fx.rings[i].age >= self.fx.rings[i].life {
                for id in self.fx.rings.remove(i).ids { self.remove(id); }
            } else {
                self.place_ring(i);
                i += 1;
            }
        }
    }

    /// Effects alive now: (particles, rings).
    pub fn fx_count(&self) -> (usize, usize) { (self.fx.parts.len(), self.fx.rings.len()) }

    /// Entities with this tag, in id order.
    pub fn find(&self, tag: &str) -> Vec<u32> { self.entities.iter().filter(|(_, e)| e.tag == tag).map(|(&id, _)| id).collect() }

    /// The first solid entity along a ray (`dir` need not be unit length).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32, filter: &Filter) -> Option<Hit> {
        let dir = dir.try_normalize()?;
        let mut best: Option<Hit> = None;
        for (&id, e) in &self.entities {
            if !filter.wants(id, e, true) { continue; }
            if let Some((t, n)) = e.bounds().ray(origin, dir, best.map_or(max, |h| h.distance)) {
                if best.is_none_or(|h| t < h.distance) { best = Some(Hit { id, distance: t, point: origin + dir * t, normal: n }); }
            }
        }
        best
    }

    /// Entities (solid or not) whose box touches a sphere, in id order.
    pub fn overlap(&self, center: Vec3, radius: f32, filter: &Filter) -> Vec<u32> {
        self.entities.iter().filter(|&(&id, e)| filter.wants(id, e, false) && sphere_touches(&e.bounds(), center, radius)).map(|(&id, _)| id).collect()
    }

    /// Move an upright cylinder (`center`, `radius`, `height`) by `vel * dt`,
    /// sliding along solid boxes. Returns the new position and velocity and
    /// whether it stands on something.
    pub fn move_body(&self, center: Vec3, vel: Vec3, dt: f32, radius: f32, height: f32, filter: &Filter) -> (Vec3, Vec3, bool) {
        let boxes: Vec<YawBox> = self.entities.iter().filter(|&(&id, e)| filter.wants(id, e, true)).map(|(_, e)| e.bounds()).collect();
        let (mut pos, mut vel) = (center, vel);
        let mover = Mover { radius: radius.max(0.01), half_height: (height * 0.5).max(0.01) };
        let contact = mover.slide(&boxes, &mut pos, &mut vel, dt, (radius * 0.5).clamp(0.05, 0.5));
        (pos, vel, contact.on_floor)
    }

    /// Upload moved slots to a renderer made from this scene.
    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> { self.slots.flush(gpu, renderer, &self.scene) }

    /// A simulation tick starts (see [`World::flush_at`]).
    pub fn tick_start(&mut self) { self.slots.tick(&self.scene); }

    /// [`World::flush`], with moving entities drawn `alpha` of the way
    /// between their previous and current tick.
    pub fn flush_at(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, alpha: f32) -> Result<(), String> { self.slots.flush_at(gpu, renderer, &self.scene, alpha) }
}

fn material(look: &Look) -> Material {
    let tint = look.color.map(|c| c.clamp(0.0, 1.0).powf(2.2));
    let (flags, scale) = if look.emissive > 0.0 { (EMISSIVE, look.emissive * EMISSIVE_NITS) } else { (0, 1.0) };
    // The game's own surface decides its glow itself: `scale` hands it the
    // emissive in nits, and the EMISSIVE flag would add the tint on top.
    if let Some(kind) = look.surface {
        let scale = if look.emissive > 0.0 { look.emissive * EMISSIVE_NITS } else { 1.0 };
        return Material { tint, layer: -2 - i32::from(kind), roughness: look.roughness.clamp(0.02, 1.0), flags: SURFACE, scale, metal: look.metal.clamp(0.0, 1.0) };
    }
    Material { tint, layer: -1, roughness: look.roughness.clamp(0.02, 1.0), flags, scale, metal: look.metal.clamp(0.0, 1.0) }
}

/// A sphere against a box rotated about Y.
fn sphere_touches(b: &YawBox, p: Vec3, r: f32) -> bool {
    let d = p - b.center;
    let (s, c) = b.yaw.sin_cos();
    // Into the box's frame (kern's convention: positive yaw turns +X to -Z).
    let local = Vec3::new(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    let nearest = local.clamp(-b.half, b.half);
    (local - nearest).length_squared() <= r * r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World { World::new(Path::new("."), 16) }

    #[test]
    fn a_wire_takes_one_slot_per_edge_and_follows_its_points() {
        let mut w = World::new(Path::new("."), 16);
        let wire = Wire { points: vec![Vec3::ZERO, Vec3::X, Vec3::Z], edges: vec![[0, 1], [1, 2], [2, 0]], width: 0.1 };
        let id = w.spawn(Spawn { shape: Shape::Wire, wire: Some(wire), solid: false, ..Default::default() }).unwrap();
        assert_eq!(w.slots.len(), 3);
        w.update(id, |e| { e.wire.as_mut().unwrap().edges.pop(); }).unwrap();
        assert_eq!(w.slots.len(), 2);
        w.update(id, |e| e.visible = false).unwrap();
        assert_eq!(w.slots.len(), 0);
        w.update(id, |e| e.visible = true).unwrap();
        w.place(id, Vec3::Y, Some(1.0)).unwrap();
        assert_eq!(w.slots.len(), 2);
        assert!(w.update(id, |e| e.shape = Shape::Cube).is_err(), "a wire stays a wire");
        assert!(w.spawn(Spawn { shape: Shape::Wire, ..Default::default() }).is_err(), "a wire needs points");
        w.remove(id);
        assert_eq!(w.slots.len(), 0);
    }

    #[test]
    fn spawn_move_remove_and_reuse_meshes() {
        let mut w = world();
        let a = w.spawn(Spawn { pos: Vec3::new(0.0, -0.5, 0.0), size: Vec3::new(10.0, 1.0, 10.0), ..Default::default() }).unwrap();
        let meshes = w.scene.meshes.len();
        let b = w.spawn(Spawn { pos: Vec3::Y * 3.0, ..Default::default() }).unwrap();
        assert_eq!(w.scene.meshes.len(), meshes, "the same shape and look share a mesh");
        w.spawn(Spawn { shape: Shape::Sphere, look: Look { color: [1.0, 0.0, 0.0], emissive: 2.0, ..Default::default() }, ..Default::default() }).unwrap();
        assert_eq!(w.scene.meshes.len(), meshes + 1);
        assert_eq!(w.scene.materials.last().unwrap().flags, EMISSIVE);
        w.update(b, |e| e.pos.x = 2.0).unwrap();
        assert_eq!(w.get(b).unwrap().pos.x, 2.0);
        assert!(w.remove(a) && !w.remove(a));
        assert!(w.update(a, |_| {}).is_err());
        for _ in 0..14 { w.spawn(Spawn::default()).unwrap(); }
        assert!(w.spawn(Spawn::default()).unwrap_err().contains("max_entities"));
        assert!(w.spawn(Spawn { shape: Shape::Model("../x.glb".into()), ..Default::default() }).is_err());
    }

    #[test]
    fn rays_overlaps_and_moves() {
        let mut w = world();
        let floor = w.spawn(Spawn { pos: Vec3::new(0.0, -0.5, 0.0), size: Vec3::new(20.0, 1.0, 20.0), ..Default::default() }).unwrap();
        let wall = w.spawn(Spawn { pos: Vec3::new(0.0, 1.0, -5.0), size: Vec3::new(4.0, 2.0, 1.0), tag: "wall".into(), ..Default::default() }).unwrap();
        let orb = w.spawn(Spawn { shape: Shape::Sphere, pos: Vec3::new(2.0, 1.0, 0.0), size: Vec3::splat(0.5), solid: false, tag: "orb".into(), ..Default::default() }).unwrap();
        let hit = w.raycast(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, 100.0, &Filter::default()).unwrap();
        assert_eq!(hit.id, wall);
        assert!((hit.distance - 4.5).abs() < 1e-4 && hit.normal == Vec3::Z, "{hit:?}");
        assert_eq!(w.raycast(Vec3::Y, Vec3::NEG_Y, 10.0, &Filter::default()).unwrap().id, floor);
        assert!(w.raycast(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0, &Filter::default()).is_none(), "the orb is not solid");
        assert_eq!(w.raycast(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0, &Filter { all: true, ..Default::default() }).unwrap().id, orb);
        assert_eq!(w.overlap(Vec3::new(1.5, 1.0, 0.0), 0.3, &Filter { tag: Some("orb".into()), ..Default::default() }), vec![orb]);
        assert!(w.overlap(Vec3::new(0.5, 1.0, 0.0), 0.3, &Filter { tag: Some("orb".into()), ..Default::default() }).is_empty());
        // A body falls onto the floor and stops at the wall.
        let (mut p, mut v) = (Vec3::new(0.0, 3.0, 0.0), Vec3::ZERO);
        let mut grounded = false;
        for _ in 0..120 { v.y -= 20.0 / 60.0; (p, v, grounded) = w.move_body(p, v, 1.0 / 60.0, 0.4, 1.8, &Filter::default()); }
        assert!(grounded && (p.y - 0.9).abs() < 0.02, "{p}");
        for _ in 0..120 { v = Vec3::new(0.0, v.y - 20.0 / 60.0, -6.0); (p, v, _) = w.move_body(p, v, 1.0 / 60.0, 0.4, 1.8, &Filter::default()); }
        assert!((p.z - (-4.5 + 0.4)).abs() < 0.05, "stopped by the wall: {p}");
    }
}
