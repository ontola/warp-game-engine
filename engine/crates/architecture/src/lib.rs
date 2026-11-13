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
pub mod baked;
pub mod sun;
pub mod surfaces;

use glam::{Affine3A, Quat, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex};
use render::irradiance_volume::Grid;
use render::room_bake::Occluders;
use render::scene::{Material, Scene, TWO_SIDED};
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
    /// A space's height (`extras.height`), when the file gives it.
    pub height: Option<f32>,
}

pub struct House {
    pub elements: Vec<Element>,
    /// Bounds of everything drawn.
    pub min: Vec3,
    pub max: Vec3,
    pub triangles: usize,
    /// The opaque confirmed geometry, for ray tests.
    pub solid: Occluders,
}

/// Eye height of a standing viewer (metres above the floor).
pub const EYE: f32 = 1.6;

/// IFC classes drawn as furniture.
const FURNITURE: [&str; 4] = ["IfcFurnishingElement", "IfcFurniture", "IfcSystemFurnitureElement", "IfcSanitaryTerminal"];

/// Rooms whose floor is tiled rather than oak, matched in their use or name
/// (English and Dutch).
const TILED: [&str; 12] = ["hall", "entr", "kitchen", "bath", "toilet", "wc", "utility", "gang", "hal", "keuken", "badkamer", "berging"];

impl House {
    /// Load a glTF building into `scene`, one mesh and instance per node,
    /// where the file puts it. Rejected elements are left out.
    pub fn load(scene: &mut Scene, path: &Path) -> Result<House, String> { Self::load_at(scene, path, None, 0.0) }

    /// Like [`House::load`], turned by `yaw` (radians, about +y) around the
    /// middle of its footprint and, with `at`, moved so that middle is at
    /// `at` (x, z). Heights are kept: files put the ground floor at 0.
    pub fn load_at(scene: &mut Scene, path: &Path, at: Option<Vec2>, yaw: f32) -> Result<House, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_glb(scene, &bytes, path, at, yaw)
    }

    /// [`House::load_at`] from a GLB's bytes (a web page fetches them);
    /// `path` names the file.
    pub fn from_glb(scene: &mut Scene, bytes: &[u8], path: &Path, at: Option<Vec2>, yaw: f32) -> Result<House, String> {
        let mut doc = assets::gltf::read_bytes(scene, bytes, path)?;
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in doc.nodes.iter().flat_map(|n| &n.primitives).flat_map(|p| &p.vertices) { lo = lo.min(Vec3::from(v.pos)); hi = hi.max(Vec3::from(v.pos)); }
        let mid = Vec3::new((lo.x + hi.x) * 0.5, 0.0, (lo.z + hi.z) * 0.5);
        let to = at.map_or(mid, |a| Vec3::new(a.x, 0.0, a.y));
        let place = Affine3A::from_translation(to) * Affine3A::from_rotation_y(yaw) * Affine3A::from_translation(-mid);
        if place != Affine3A::IDENTITY {
            for v in doc.nodes.iter_mut().flat_map(|n| &mut n.primitives).flat_map(|p| &mut p.vertices) {
                v.pos = place.transform_point3(Vec3::from(v.pos)).to_array();
                v.normal = place.transform_vector3(Vec3::from(v.normal)).to_array();
            }
        }
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
        // Room volumes, where the file gives a space's height: a face whose
        // front lies in a room is inside however its rays leave (a hall
        // wall facing the front door is not a facade).
        let rooms: Vec<(Vec<[Vec2; 3]>, f32, f32)> = doc.nodes.iter().zip(&tags).filter_map(|(node, t)| {
            let height = t["height"].as_f64()? as f32;
            if t["ifcClass"].as_str() != Some("IfcSpace") { return None; }
            let tris: Vec<[Vec3; 3]> = node.primitives.iter().flat_map(|p| p.indices.as_chunks::<3>().0.iter().map(|tri| tri.map(|i| Vec3::from(p.vertices[i as usize].pos)))).collect();
            let y = tris.iter().flatten().map(|p| p.y).fold(f32::INFINITY, f32::min);
            Some((tris.iter().map(|t| t.map(|p| Vec2::new(p.x, p.z))).collect(), y - 0.01, y + height))
        }).collect();
        let in_room = |p: Vec3| rooms.iter().any(|(tris, lo, hi)| (*lo..=*hi).contains(&p.y) && tris.iter().any(|t| in_triangle(Vec2::new(p.x, p.z), t)));
        let mut materials: HashMap<(Surface, bool, bool), u32> = HashMap::new();
        let ghost = scene.add_material(Material { tint: [0.35, 0.6, 0.95], layer: -1, roughness: 0.3, flags: 0, scale: 1.0, metal: 0.0 });
        scene.set_material_blend(ghost, 0.35, true);
        let (mut min, mut max, mut triangles) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY), 0);
        let mut elements = Vec::new();
        for (node, t) in doc.nodes.iter().zip(&tags) {
            let ifc_class = t["ifcClass"].as_str().unwrap_or("").to_string();
            let name = node.name.clone().unwrap_or_else(|| ifc_class.clone());
            let usage = t["usage"].as_str().map(str::to_string);
            let finish = Finish::from_tags(t);
            let review = review(t);
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            // Triangles regrouped by Warp material, with metre-scale UVs.
            // Surfaces are chosen per patch (triangles joined by edges with
            // close normals), not per triangle: a wall face cut around a
            // window is split into long slivers, and per-triangle choices
            // made some of them brick where their rays left by the window.
            let mut tris: Vec<(usize, [Vertex; 3], [Vec3; 3], Vec3, f32)> = Vec::new();
            for (k, p) in node.primitives.iter().enumerate() {
                for tri in p.indices.as_chunks::<3>().0 {
                    let v = tri.map(|i| p.vertices[i as usize]);
                    let pos = v.map(|v| Vec3::from(v.pos));
                    for q in pos { lo = lo.min(q); hi = hi.max(q); }
                    let c = (pos[1] - pos[0]).cross(pos[2] - pos[0]);
                    tris.push((k, v, pos, c.normalize_or(Vec3::Y), c.length() * 0.5));
                }
            }
            let patch = patches(&tris.iter().map(|t| (t.0, t.2, t.3)).collect::<Vec<_>>());
            // Per patch: area-weighted normal, outward area, total area, curved?
            let mut info: HashMap<usize, (Vec3, f32, f32, bool)> = HashMap::new();
            for (i, (k, _, pos, n, area)) in tris.iter().enumerate() {
                let e = info.entry(patch[i]).or_insert((Vec3::ZERO, 0.0, 0.0, false));
                e.0 += *n * *area;
                e.2 += *area;
                let kind = node.primitives[*k].material_name.as_deref().unwrap_or("");
                let walls = !matches!(kind, "glass" | "frame" | "furniture" | "floor")
                    && (finish.layered.is_some() || (!FURNITURE.contains(&ifc_class.as_str()) && ifc_class != "IfcWindow" && ifc_class != "IfcDoor"));
                let c = (pos[0] + pos[1] + pos[2]) / 3.0;
                if walls && !in_room(c + *n * 0.1) && faces_outside(&outside, c, *n) { e.1 += *area; }
            }
            for (i, (_, _, _, n, _)) in tris.iter().enumerate() {
                let e = info.get_mut(&patch[i]).unwrap();
                if n.dot(e.0.normalize_or(*n)) < 0.985 { e.3 = true; }
            }
            let smooth = smooth_normals(&tris.iter().map(|t| (t.2, t.3)).collect::<Vec<_>>(), &patch);
            let mut parts: HashMap<u32, (Vec<Vertex>, Vec<u32>, HashMap<[u32; 8], u32>)> = HashMap::new();
            for (i, (k, v, pos, n, _)) in tris.into_iter().enumerate() {
                let p = &node.primitives[k];
                let kind = p.material_name.as_deref().unwrap_or("");
                let &(sum, out, area, curved) = &info[&patch[i]];
                let pn = sum.normalize_or(n);
                let surface = match kind {
                    "glass" => None,
                    "frame" => Some(Surface::Frame),
                    "furniture" => Some(Surface::Furniture),
                    _ if FURNITURE.contains(&ifc_class.as_str()) => Some(Surface::Furniture),
                    "floor" => {
                        let label = format!("{} {}", usage.as_deref().unwrap_or(""), name).to_lowercase();
                        Some(if TILED.iter().any(|w| label.contains(w)) { Surface::Tile } else { Surface::Oak })
                    }
                    // What the file says it is made of: a layered build-up's
                    // outer finish outside and inner finish inside, else the
                    // surface style's name, else the element's materials.
                    _ if finish.layered.is_some() => {
                        let (outer, inner) = finish.layered.unwrap();
                        Some(if out * 2.0 > area { outer } else { inner })
                    }
                    _ if Surface::from_material_name(kind).is_some() => Surface::from_material_name(kind),
                    _ if finish.single.is_some() => finish.single,
                    _ if ifc_class == "IfcWindow" || ifc_class == "IfcDoor" => Some(Surface::Frame),
                    _ => Some(if out * 2.0 <= area { Surface::Plaster }
                        else if !curved && pn.y > 0.7 { Surface::Roof }
                        else if !curved && pn.y < -0.7 { Surface::Plaster }
                        else { Surface::Brick }),
                };
                let triplanar = curved && surface.is_some_and(Surface::textured);
                let material = match (review, surface) {
                    (Review::Proposed, _) => ghost,
                    (_, None) => p.material,
                    (_, Some(s)) => {
                        let two_sided = scene.materials.get(p.material as usize).is_some_and(|m| m.flags & TWO_SIDED != 0);
                        *materials.entry((s, triplanar, two_sided)).or_insert_with(|| s.add_mapped(scene, triplanar, two_sided))
                    }
                };
                let tile = surface.map_or(1.0, Surface::tile_metres);
                let (verts, idx, seen) = parts.entry(material).or_default();
                for (j, mut vert) in v.into_iter().enumerate() {
                    // One plane per flat patch, so the pattern runs on
                    // across its triangles.
                    vert.uv = (planar(pos[j], if curved { n } else { pn }) / tile).to_array();
                    if surface.is_some() { vert.normal = if curved { smooth[i][j] } else { n }.to_array(); }
                    // Shared vertices: the LOD simplifier only collapses
                    // edges between connected triangles; a soup of loose
                    // triangles fell back to sloppy clustering and broke a
                    // round window frame into shards at a few metres.
                    let bits: [u32; 8] = bytemuck_bits(&vert);
                    let at = *seen.entry(bits).or_insert_with(|| { verts.push(vert); verts.len() as u32 - 1 });
                    idx.push(at);
                }
            }
            let height = t["height"].as_f64().map(|h| h as f32);
            let mut element = Element { name, ifc_class, review, usage, min: lo, max: hi, mesh: None, instance: None, height };
            if review != Review::Rejected && !parts.is_empty() {
                let list: Vec<Part> = parts.iter().map(|(m, (v, i, _))| Part { vertices: v, indices: i, material: *m }).collect();
                triangles += list.iter().map(|p| p.indices.len() / 3).sum::<usize>();
                // Buildings keep their normals, uv seams and creases through LOD.
                let mesh = scene.add_mesh(&ClusterMesh::build_with_policy(&list, |_| SimplificationPolicy::TexturedGeometry));
                element.instance = Some(scene.instances.len());
                scene.add_instance(mesh, Affine3A::IDENTITY);
                element.mesh = Some(mesh);
                min = min.min(lo);
                max = max.max(hi);
            }
            elements.push(element);
        }
        if triangles == 0 { return Err(format!("{}: nothing to draw", path.display())); }
        Ok(House { elements, min, max, triangles, solid: outside })
    }

    pub fn spaces(&self) -> impl Iterator<Item = &Element> { self.elements.iter().filter(|e| e.ifc_class == "IfcSpace") }

    /// Probes every half metre over the building and a cell around it,
    /// a quarter cell off whole-metre walls so no probe sits in a wall face.
    pub fn probe_grid(&self) -> Grid { probe_grid(self.min, self.max) }

    /// A grass-coloured ground under the house, 400 m across.
    pub fn add_ground(&self, scene: &mut Scene) { add_ground(scene, self.center(), self.grade()); }

    /// The outside ground: the underside of the slab the ground floor
    /// stands on, so the ground never shares a plane with a floor (it
    /// showed through the wood). Found down from the ground-floor spaces;
    /// 15 cm below the floor (a usual build-up) when nothing is under them.
    pub fn grade(&self) -> f32 {
        let level = self.ground_level();
        let down = Vec3::NEG_Y;
        self.spaces().filter(|s| (s.max.y - level).abs() < 0.05).find_map(|s| {
            let c = (s.min + s.max) * 0.5;
            let top = Vec3::new(c.x, level + 0.3, c.z);
            let (t, _) = self.solid.hit(top, down).filter(|(t, _)| *t < 0.6)?;
            let inside = top + down * (t + 1e-3);
            let (u, _) = self.solid.hit(inside, down).filter(|(u, _)| *u < 1.5)?;
            Some(inside.y - u)
        }).unwrap_or(level - 0.15)
    }

    /// The ground floor: the floor of the space nearest height 0 (files put
    /// the ground floor at 0; a basement is lower), else the model's base.
    pub fn ground_level(&self) -> f32 {
        let floor = self.spaces().map(|s| s.max.y).min_by(|a, b| a.abs().total_cmp(&b.abs()));
        floor.unwrap_or(self.min.y)
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
    /// (about 20 mm on full frame at 16:9). Spaces where a standing eye
    /// has no ceiling within 6 m (roof terraces, low attics where the eye
    /// ends up above the roof) get no shot.
    pub fn room_shots(&self) -> Vec<(String, Camera)> {
        let c = self.center();
        self.spaces().filter(|s| s.mesh.is_some()).filter_map(|s| {
            let floor = s.max.y;
            let inset = 0.35;
            let (lo, hi) = (s.min + Vec3::splat(inset), s.max - Vec3::splat(inset));
            let x = if (lo.x - c.x).abs() < (hi.x - c.x).abs() { (lo.x, hi.x) } else { (hi.x, lo.x) };
            let z = if (lo.z - c.z).abs() < (hi.z - c.z).abs() { (lo.z, hi.z) } else { (hi.z, lo.z) };
            let pos = Vec3::new(x.0, floor + EYE, z.0);
            self.solid.hit(pos, Vec3::Y).filter(|(t, _)| *t < 6.0)?;
            Some((s.name.clone(), Camera { pos, target: Vec3::new(x.1, floor + EYE, z.1), fov_y: 60f32.to_radians(), near: 0.05 }))
        }).collect()
    }
}

impl House {
    /// One reflection box per room (`Renderer::set_reflection_probes`):
    /// the space's footprint bounds from its floor to its height (or the
    /// ceiling straight above its middle), captured from the middle at
    /// eye height. The renderer takes at most
    /// `render::reflection_probes::MAX_PROBES`; a viewer picks the nearest.
    pub fn reflection_boxes(&self) -> Vec<render::reflection_probes::ProbeBox> {
        self.spaces().filter(|s| s.mesh.is_some()).filter_map(|s| {
            let floor = s.max.y;
            let mid = Vec3::new((s.min.x + s.max.x) * 0.5, floor + 0.1, (s.min.z + s.max.z) * 0.5);
            let height = s.height.filter(|h| *h > 0.5).or_else(|| self.solid.hit(mid, Vec3::Y).map(|(t, _)| t + 0.1).filter(|h| *h < 6.0))?;
            let (mut min, mut max) = (Vec3::new(s.min.x, floor, s.min.z), Vec3::new(s.max.x, floor + height, s.max.z));
            if (max - min).min_element() < 0.3 { return None; }
            // A space ends at its walls' faces, so a door's reveal (jambs,
            // head, sill) belongs to no box, and a point there took the
            // probe of whatever room its reflected ray heads into: the
            // bright room behind a closed door. The room reaches through its
            // door openings up to the leaf (the door's middle).
            for d in self.elements.iter().filter(|e| e.ifc_class == "IfcDoor") {
                let size = d.max - d.min;
                let (a, b) = if size.x < size.z { (0, 2) } else { (2, 0) };
                if d.max[b] <= min[b] || d.min[b] >= max[b] || d.min.y >= max.y || d.max.y <= min.y { continue; }
                let mid = (d.min[a] + d.max[a]) * 0.5;
                if mid < min[a] && min[a] - mid < 0.4 { min[a] = mid; }
                if mid > max[a] && mid - max[a] < 0.4 { max[a] = mid; }
            }
            let capture = Vec3::new(mid.x, floor + EYE.min(height * 0.6), mid.z);
            Some(render::reflection_probes::ProbeBox { capture, min, max })
        }).collect()
    }
}

/// Probes every half metre over `min..max` and a cell around it, a quarter
/// cell off whole-metre walls so no probe sits in a wall face. A scene has
/// one probe grid of at most 65536 probes: for several buildings, pass the
/// bounds of all of them, and the cell grows (0.75 m, 1 m, ...) until the
/// grid fits.
pub fn probe_grid(min: Vec3, max: Vec3) -> Grid {
    let mut cell = 0.5;
    loop {
        let lo = min - Vec3::splat(cell * 1.75);
        let hi = max + Vec3::splat(cell * 1.5);
        let lo = (lo / cell).floor() * cell + Vec3::splat(cell * 0.25);
        let dims = ((hi - lo) / cell).ceil().max(Vec3::splat(2.0));
        let grid = Grid { origin: lo, cell, dims: [dims.x as u32, dims.y as u32, dims.z as u32] };
        if grid.count() <= 65536 { return grid; }
        cell += 0.25;
    }
}

/// A grass-coloured ground with its top at `level`, 400 m across, centred
/// under `center`.
pub fn add_ground(scene: &mut Scene, center: Vec3, level: f32) {
    let m = scene.add_material(Material { tint: [0.16, 0.24, 0.09], layer: -1, roughness: 0.95, flags: 0, scale: 1.0, metal: 0.0 });
    let ground = scene.add_mesh(&primitives::cube(m).mesh());
    scene.add_instance(ground, Affine3A::from_scale_rotation_translation(Vec3::new(400.0, 0.2, 400.0), Quat::IDENTITY, Vec3::new(center.x, level - 0.1, center.z)));
}

/// The finishes an element's IFC materials name (extras `layers` and
/// `materials`, from tools/ifc_to_glb.py).
struct Finish {
    /// (outer, inner) of a layered build-up: the first nameable layer from
    /// each side. IFC orders layers from a reference side that is not
    /// always the outside, so an inside-only finish (plaster, tiles, wood
    /// flooring) found on the first side swaps them.
    layered: Option<(Surface, Surface)>,
    /// The first nameable material of anything else.
    single: Option<Surface>,
}

impl Finish {
    fn from_tags(t: &serde_json::Value) -> Finish {
        let names = |k: &str| -> Vec<String> { t[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let layers: Vec<Surface> = names("layers").iter().filter_map(|n| Surface::from_material_name(n)).collect();
        let layered = match (layers.first(), layers.last()) {
            (Some(&a), Some(&b)) if layers.len() >= 2 => {
                let inside_only = |s: Surface| matches!(s, Surface::Plaster | Surface::Tile | Surface::Oak);
                Some(if inside_only(a) && !inside_only(b) { (b, a) } else { (a, b) })
            }
            _ => None,
        };
        let single = names("materials").iter().chain(&names("layers")).find_map(|n| Surface::from_material_name(n));
        Finish { layered, single }
    }
}

fn in_triangle(p: Vec2, t: &[Vec2; 3]) -> bool {
    let side = |a: Vec2, b: Vec2| (b - a).perp_dot(p - a);
    let (d0, d1, d2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
}

fn bytemuck_bits(v: &Vertex) -> [u32; 8] {
    let f = [v.pos[0], v.pos[1], v.pos[2], v.normal[0], v.normal[1], v.normal[2], v.uv[0], v.uv[1]];
    f.map(f32::to_bits)
}

/// Millimetre key of a position, for finding shared vertices.
fn key(p: Vec3) -> [i32; 3] { (p * 1000.0).round().as_ivec3().to_array() }

/// Patch id per triangle `(primitive, corners, normal)`: triangles of one
/// primitive sharing an edge join when their normals are within 30
/// degrees, so a flat face is one patch and a round reveal another.
fn patches(tris: &[(usize, [Vec3; 3], Vec3)]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i { parent[i] = parent[parent[i]]; i = parent[i]; }
        i
    }
    let mut edges: HashMap<(usize, [i32; 3], [i32; 3]), Vec<usize>> = HashMap::new();
    for (i, (k, pos, _)) in tris.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (key(pos[e]), key(pos[(e + 1) % 3]));
            edges.entry((*k, a.min(b), a.max(b))).or_default().push(i);
        }
    }
    for list in edges.values() {
        for w in list.windows(2) {
            if tris[w[0]].2.dot(tris[w[1]].2) > 0.866 {
                let (a, b) = (root(&mut parent, w[0]), root(&mut parent, w[1]));
                parent[a] = b;
            }
        }
    }
    (0..tris.len()).map(|i| root(&mut parent, i)).collect()
}

/// Vertex normals averaged (by area) over the patch's triangles sharing
/// the vertex: curved patches shade round instead of faceted.
fn smooth_normals(tris: &[([Vec3; 3], Vec3)], patch: &[usize]) -> Vec<[Vec3; 3]> {
    let mut sum: HashMap<(usize, [i32; 3]), Vec3> = HashMap::new();
    for (i, (pos, n)) in tris.iter().enumerate() {
        let area = (pos[1] - pos[0]).cross(pos[2] - pos[0]).length();
        for p in pos { *sum.entry((patch[i], key(*p))).or_default() += *n * area; }
    }
    tris.iter().enumerate().map(|(i, (pos, n))| pos.map(|p| sum[&(patch[i], key(p))].normalize_or(*n))).collect()
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
