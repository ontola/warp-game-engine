//! Warp Architecture: buildings and their interiors, lit like real rooms.
//!
//! A house arrives as glTF 2.0 (the AtomicBuildings export, or IFC turned
//! into glTF by IfcConvert). Nodes may carry `extras` with `ifcClass`
//! (`IfcWall`, `IfcWindow`, `IfcSpace`, ...), `reviewState` (`proposed`,
//! `confirmed`) and `usage` (a space's use). Each node becomes its own
//! instance, so an element can be hidden or restyled. From the tags this
//! crate derives what a viewer of buildings needs without hand placement:
//! surfaces ([`surfaces`]: brick outside, plaster inside, oak or tiles per
//! room), proposed elements as a tinted ghost, the probe grid for the room
//! bake (`render::room_bake`), camera shots per room and around the outside,
//! a ground, and the sun for a place and time ([`sun`]).
//!
//! ```ignore
//! let house = architecture::House::load(&mut scene, Path::new("house.glb"))?;
//! house.add_ground(&mut scene);
//! scene.reserve_irradiance_volume(house.probe_grid())?;
//! ```
pub mod sun;
pub mod surfaces;

use glam::{Affine3A, Quat, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::irradiance_volume::Grid;
use render::room_bake::Occluders;
use render::scene::{Material, Scene};
use render::{primitives, Camera};
use std::collections::HashMap;
use std::path::Path;
pub use surfaces::Surface;

/// Review state of an element, as AtomicBuildings marks AI work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Review { Confirmed, Proposed, Rejected, Unknown }

/// One glTF node with a mesh: an element of the building.
#[derive(Clone, Debug)]
pub struct Element {
    pub name: String,
    /// IFC entity name (`IfcWall`, `IfcSpace`, ...), empty when unknown.
    pub ifc_class: String,
    pub review: Review,
    /// A space's use (`living`, `kitchen`, ...) from `extras.usage`.
    pub usage: Option<String>,
    /// World-space bounds.
    pub min: Vec3,
    pub max: Vec3,
    /// Scene mesh and instance (rejected elements have none).
    pub mesh: Option<usize>,
    pub instance: Option<usize>,
}

pub struct House {
    pub elements: Vec<Element>,
    /// Bounds of everything drawn.
    pub min: Vec3,
    pub max: Vec3,
    pub triangles: usize,
}

/// Eye height of a standing viewer (metres above the floor).
pub const EYE: f32 = 1.6;

/// Rooms whose floor is tiled rather than oak, matched in their use or name
/// (English and Dutch).
const TILED: [&str; 12] = ["hall", "entr", "kitchen", "bath", "toilet", "wc", "utility", "gang", "hal", "keuken", "badkamer", "berging"];

impl House {
    /// Load a glTF building into `scene`, one mesh and instance per node.
    /// Rejected elements are left out.
    pub fn load(scene: &mut Scene, path: &Path) -> Result<House, String> {
        let doc = assets::gltf::read(scene, path)?;
        let tags: Vec<serde_json::Value> = doc.nodes.iter().map(|n| n.extras.as_deref().and_then(|e| serde_json::from_str(e).ok()).unwrap_or_default()).collect();
        let review = |t: &serde_json::Value| match t["reviewState"].as_str() {
            Some("proposed") => Review::Proposed, Some("confirmed") => Review::Confirmed, Some("rejected") => Review::Rejected, _ => Review::Unknown };
        // What blocks the view out: every opaque confirmed triangle.
        let mut solid = Vec::new();
        for (node, t) in doc.nodes.iter().zip(&tags) {
            if !matches!(review(t), Review::Confirmed | Review::Unknown) { continue; }
            for p in node.primitives.iter().filter(|p| p.material_name.as_deref() != Some("glass")) {
                for tri in p.indices.as_chunks::<3>().0 { solid.push(tri.map(|i| Vec3::from(p.vertices[i as usize].pos))); }
            }
        }
        let outside = Occluders::from_triangles(solid);
        let mut materials: HashMap<Surface, u32> = HashMap::new();
        let ghost = scene.add_material(Material { tint: [0.35, 0.6, 0.95], layer: -1, roughness: 0.3, flags: 0, scale: 1.0, metal: 0.0 });
        scene.set_material_blend(ghost, 0.35, true);
        let (mut min, mut max, mut triangles) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY), 0);
        let mut elements = Vec::new();
        for (node, t) in doc.nodes.iter().zip(&tags) {
            let ifc_class = t["ifcClass"].as_str().unwrap_or("").to_string();
            let name = node.name.clone().unwrap_or_else(|| ifc_class.clone());
            let usage = t["usage"].as_str().map(str::to_string);
            let review = review(t);
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            // Triangles regrouped by Warp material, with metre-scale UVs.
            let mut parts: HashMap<u32, (Vec<Vertex>, Vec<u32>)> = HashMap::new();
            for p in &node.primitives {
                let kind = p.material_name.as_deref().unwrap_or("");
                for tri in p.indices.as_chunks::<3>().0 {
                    let v = tri.map(|i| p.vertices[i as usize]);
                    let pos = v.map(|v| Vec3::from(v.pos));
                    for q in pos { lo = lo.min(q); hi = hi.max(q); }
                    let n = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or(Vec3::Y);
                    let surface = match kind {
                        "glass" => None,
                        "frame" => Some(Surface::Frame),
                        "furniture" => Some(Surface::Furniture),
                        "floor" => {
                            let label = format!("{} {}", usage.as_deref().unwrap_or(""), name).to_lowercase();
                            Some(if TILED.iter().any(|w| label.contains(w)) { Surface::Tile } else { Surface::Oak })
                        }
                        _ if ifc_class == "IfcWindow" || ifc_class == "IfcDoor" => Some(Surface::Frame),
                        _ => {
                            let c = (pos[0] + pos[1] + pos[2]) / 3.0;
                            Some(if !faces_outside(&outside, c, n) { Surface::Plaster }
                                else if n.y > 0.7 { Surface::Roof }
                                else if n.y < -0.7 { Surface::Plaster }
                                else { Surface::Brick })
                        }
                    };
                    let material = match (review, surface) {
                        (Review::Proposed, _) => ghost,
                        (_, None) => p.material,
                        (_, Some(s)) => *materials.entry(s).or_insert_with(|| s.add(scene)),
                    };
                    let tile = surface.map_or(1.0, Surface::tile_metres);
                    let (verts, idx) = parts.entry(material).or_default();
                    for (k, mut vert) in v.into_iter().enumerate() {
                        vert.uv = (planar(pos[k], n) / tile).to_array();
                        vert.normal = if surface.is_some() { n.to_array() } else { vert.normal };
                        idx.push(verts.len() as u32);
                        verts.push(vert);
                    }
                }
            }
            let mut element = Element { name, ifc_class, review, usage, min: lo, max: hi, mesh: None, instance: None };
            if review != Review::Rejected && !parts.is_empty() {
                let list: Vec<Part> = parts.iter().map(|(m, (v, i))| Part { vertices: v, indices: i, material: *m }).collect();
                triangles += list.iter().map(|p| p.indices.len() / 3).sum::<usize>();
                let mesh = scene.add_mesh(&ClusterMesh::build(&list));
                element.instance = Some(scene.instances.len());
                scene.add_instance(mesh, Affine3A::IDENTITY);
                element.mesh = Some(mesh);
                min = min.min(lo);
                max = max.max(hi);
            }
            elements.push(element);
        }
        if triangles == 0 { return Err(format!("{}: nothing to draw", path.display())); }
        Ok(House { elements, min, max, triangles })
    }

    pub fn spaces(&self) -> impl Iterator<Item = &Element> { self.elements.iter().filter(|e| e.ifc_class == "IfcSpace") }

    /// Probes every half metre over the building and a cell around it,
    /// a quarter cell off whole-metre walls so no probe sits in a wall face.
    pub fn probe_grid(&self) -> Grid {
        let cell = 0.5;
        let lo = self.min - Vec3::splat(cell * 1.75);
        let hi = self.max + Vec3::splat(cell * 1.5);
        let lo = (lo / cell).floor() * cell + Vec3::splat(cell * 0.25);
        let dims = ((hi - lo) / cell).ceil().max(Vec3::ONE);
        Grid { origin: lo, cell, dims: [dims.x as u32, dims.y as u32, dims.z as u32] }
    }

    /// A grass-coloured ground under the house, 400 m across.
    pub fn add_ground(&self, scene: &mut Scene) {
        let m = scene.add_material(Material { tint: [0.16, 0.24, 0.09], layer: -1, roughness: 0.95, flags: 0, scale: 1.0, metal: 0.0 });
        let ground = scene.add_mesh(&primitives::cube(m).mesh());
        let c = self.center();
        scene.add_instance(ground, Affine3A::from_scale_rotation_translation(Vec3::new(400.0, 0.2, 400.0), Quat::IDENTITY, Vec3::new(c.x, self.ground_level() - 0.1, c.z)));
    }

    /// The lowest floor: the top of the lowest space, else the model's base.
    pub fn ground_level(&self) -> f32 {
        let floor = self.spaces().map(|s| s.max.y).fold(f32::INFINITY, f32::min);
        if floor.is_finite() { floor } else { self.min.y }
    }

    pub fn center(&self) -> Vec3 { (self.min + self.max) * 0.5 }

    /// Four views from outside at the corners, from standing height with a
    /// level camera (vertical lines stay vertical, as in architectural
    /// photos), far enough to frame the building with a 50 degree lens.
    pub fn exterior_shots(&self) -> Vec<(String, Camera)> {
        let c = self.center();
        let ground = self.ground_level();
        let above = (self.max.y - ground - EYE).max(1.0);
        let half = (self.max - self.min) * 0.5;
        let d = (above / 22f32.to_radians().tan()).max(half.x.hypot(half.z) * 2.2);
        // AtomicBuildings' glTF has the front (plan y = 0) facing -z; a
        // viewer in front looking at +z has +x on their left.
        [("front left", 1.0, -1.0), ("front right", -1.0, -1.0), ("back left", -1.0, 1.0), ("back right", 1.0, 1.0)].into_iter().map(|(name, x, z)| {
            let pos = Vec3::new(c.x, ground + EYE, c.z) + Vec3::new(x, 0.0, z).normalize() * d;
            (name.to_string(), Camera { pos, target: Vec3::new(c.x, ground + EYE, c.z), fov_y: 50f32.to_radians(), near: 0.1 })
        }).collect()
    }

    /// One view per room: standing in the corner nearest the building's
    /// middle (inset 0.35 m), level, looking across to the far corner, where
    /// the outside walls and their windows are. Lens: 60 degrees vertical
    /// (about 20 mm on full frame at 16:9).
    pub fn room_shots(&self) -> Vec<(String, Camera)> {
        let c = self.center();
        self.spaces().filter(|s| s.mesh.is_some()).map(|s| {
            let floor = s.max.y;
            let inset = 0.35;
            let (lo, hi) = (s.min + Vec3::splat(inset), s.max - Vec3::splat(inset));
            let x = if (lo.x - c.x).abs() < (hi.x - c.x).abs() { (lo.x, hi.x) } else { (hi.x, lo.x) };
            let z = if (lo.z - c.z).abs() < (hi.z - c.z).abs() { (lo.z, hi.z) } else { (hi.z, lo.z) };
            let pos = Vec3::new(x.0, floor + EYE, z.0);
            (s.name.clone(), Camera { pos, target: Vec3::new(x.1, floor + EYE, z.1), fov_y: 60f32.to_radians(), near: 0.05 })
        }).collect()
    }
}

/// Does a face at `c` with normal `n` look out of the building? Four rays
/// (straight out, 40 degrees to each side, and up) must all leave without
/// hitting the building: an inside wall facing a window across the room
/// still sees walls on its tilted rays.
fn faces_outside(solid: &Occluders, c: Vec3, n: Vec3) -> bool {
    let side = if n.y.abs() < 0.9 { Vec3::Y.cross(n).normalize() } else { Vec3::X };
    let o = c + n * 0.02;
    let rays = [n, (n + side * 0.84).normalize(), (n - side * 0.84).normalize(), (n + Vec3::Y * 0.6).normalize()];
    rays.iter().all(|&d| solid.hit(o, d).is_none_or(|(t, _)| t > 40.0))
}

/// Metre coordinates in the face's plane: floors and ceilings in x/z, walls
/// along their own length and up, so textures keep their real size on walls
/// at any angle.
fn planar(p: Vec3, n: Vec3) -> Vec2 {
    if n.y.abs() > 0.7 { return Vec2::new(p.x, p.z); }
    let along = Vec3::Y.cross(n).normalize_or(Vec3::X);
    Vec2::new(p.dot(along), -p.y)
}
