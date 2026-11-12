//! Warp Architecture: buildings and their interiors, lit like real rooms.
//!
//! A house arrives as glTF 2.0 (the AtomicBuildings export, or IFC turned
//! into glTF by IfcConvert). Nodes may carry `extras` with `ifcClass`
//! (`IfcWall`, `IfcWindow`, `IfcSpace`, ...) and `reviewState`
//! (`proposed`, `confirmed`). From that this crate derives what a viewer of
//! buildings needs without hand placement: the probe grid for the room bake
//! (light that stays inside the rooms, see `render::room_bake`), camera shots
//! per room and around the outside, and a ground to stand on.
//!
//! ```ignore
//! let house = architecture::House::load(&mut scene, Path::new("house.glb"))?;
//! house.add_ground(&mut scene);
//! scene.reserve_irradiance_volume(house.probe_grid())?;
//! ```
use glam::{Affine3A, Mat4, Quat, Vec3};
use render::irradiance_volume::Grid;
use render::scene::{Material, Scene};
use render::{primitives, Camera};
use std::path::Path;

/// Review state of an element, as AtomicBuildings marks AI work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Review { Confirmed, Proposed, Unknown }

/// One glTF node that has a mesh, with its world-space bounds.
#[derive(Clone, Debug)]
pub struct Element {
    pub name: String,
    /// IFC entity name (`IfcWall`, `IfcSpace`, ...), empty when unknown.
    pub ifc_class: String,
    pub review: Review,
    pub min: Vec3,
    pub max: Vec3,
}

pub struct House {
    pub model: assets::gltf::Model,
    pub elements: Vec<Element>,
}

/// Eye height of a standing viewer (metres above the floor).
pub const EYE: f32 = 1.6;

impl House {
    /// Load a glTF building into `scene` (one mesh, not yet instanced) and
    /// read its elements. Adds the model at the origin.
    pub fn load(scene: &mut Scene, path: &Path) -> Result<House, String> {
        let model = assets::gltf::load(scene, path)?;
        scene.add_instance(model.mesh, Affine3A::IDENTITY);
        let elements = read_elements(path)?;
        Ok(House { model, elements })
    }

    pub fn spaces(&self) -> impl Iterator<Item = &Element> { self.elements.iter().filter(|e| e.ifc_class == "IfcSpace") }

    /// Probes every half metre over the building and a cell around it,
    /// a quarter cell off whole-metre walls so no probe sits in a wall face.
    pub fn probe_grid(&self) -> Grid {
        let cell = 0.5;
        let lo = self.model.min - Vec3::splat(cell * 1.75);
        let hi = self.model.max + Vec3::splat(cell * 1.5);
        let lo = (lo / cell).floor() * cell + Vec3::splat(cell * 0.25);
        let dims = ((hi - lo) / cell).ceil().max(Vec3::ONE);
        Grid { origin: lo, cell, dims: [dims.x as u32, dims.y as u32, dims.z as u32] }
    }

    /// A grass-coloured ground under the house, 400 m across.
    pub fn add_ground(&self, scene: &mut Scene) {
        let m = scene.add_material(Material { tint: [0.16, 0.24, 0.09], layer: -1, roughness: 0.95, flags: 0, scale: 1.0, metal: 0.0 });
        let ground = scene.add_mesh(&primitives::cube(m).mesh());
        let top = self.ground_level();
        scene.add_instance(ground, Affine3A::from_scale_rotation_translation(Vec3::new(400.0, 0.2, 400.0), Quat::IDENTITY, Vec3::new(self.center().x, top - 0.1, self.center().z)));
    }

    /// The lowest floor: the top of the lowest space, else the model's base.
    pub fn ground_level(&self) -> f32 {
        let floor = self.spaces().map(|s| s.max.y).fold(f32::INFINITY, f32::min);
        if floor.is_finite() { floor } else { self.model.min.y }
    }

    pub fn center(&self) -> Vec3 { (self.model.min + self.model.max) * 0.5 }

    /// Four views from outside: the corners at 3/4, standing height, far
    /// enough to frame the whole building with a 50 degree lens.
    pub fn exterior_shots(&self) -> Vec<(String, Camera)> {
        let c = self.center();
        let r = (self.model.max - self.model.min).length() * 0.5;
        let d = r / (25f32.to_radians().tan()) * 1.1;
        let ground = self.ground_level();
        let target = Vec3::new(c.x, ground + (c.y - ground) * 0.8, c.z);
        // AtomicBuildings' glTF has the front (plan y = 0) facing -z; a
        // viewer in front looking at +z has +x on their left.
        [("front left", 1.0, -1.0), ("front right", -1.0, -1.0), ("back left", -1.0, 1.0), ("back right", 1.0, 1.0)].into_iter().map(|(name, x, z)| {
            let pos = Vec3::new(c.x, ground + EYE, c.z) + Vec3::new(x, 0.0, z).normalize() * d;
            (name.to_string(), Camera { pos, target, fov_y: 50f32.to_radians(), near: 0.1 })
        }).collect()
    }

    /// One view per room: standing in the corner nearest the building's
    /// middle (inset 0.35 m), looking across to the far corner, where the
    /// outside walls and their windows are.
    pub fn room_shots(&self) -> Vec<(String, Camera)> {
        let c = self.center();
        self.spaces().map(|s| {
            let floor = s.max.y;
            let inset = 0.35;
            let (lo, hi) = (s.min + Vec3::splat(inset), s.max - Vec3::splat(inset));
            let x = if (lo.x - c.x).abs() < (hi.x - c.x).abs() { (lo.x, hi.x) } else { (hi.x, lo.x) };
            let z = if (lo.z - c.z).abs() < (hi.z - c.z).abs() { (lo.z, hi.z) } else { (hi.z, lo.z) };
            let pos = Vec3::new(x.0, floor + EYE, z.0);
            let target = Vec3::new(x.1, floor + 1.2, z.1);
            (s.name.clone(), Camera { pos, target, fov_y: 70f32.to_radians(), near: 0.05 })
        }).collect()
    }
}

fn read_elements(path: &Path) -> Result<Vec<Element>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let g = gltf::Gltf::from_slice_without_validation(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    let mut stack: Vec<(gltf::Node, Mat4)> = g.default_scene().or_else(|| g.scenes().next())
        .map(|s| s.nodes().map(|n| (n, Mat4::IDENTITY)).collect()).unwrap_or_default();
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for p in mesh.primitives() {
                let b = p.bounding_box();
                for i in 0..8 {
                    let corner = Vec3::new(if i & 1 == 0 { b.min[0] } else { b.max[0] }, if i & 2 == 0 { b.min[1] } else { b.max[1] }, if i & 4 == 0 { b.min[2] } else { b.max[2] });
                    let w = world.transform_point3(corner);
                    min = min.min(w);
                    max = max.max(w);
                }
            }
            let extras: serde_json::Value = node.extras().as_ref().and_then(|r| serde_json::from_str(r.get()).ok()).unwrap_or_default();
            let ifc_class = extras["ifcClass"].as_str().unwrap_or("").to_string();
            let review = match extras["reviewState"].as_str() { Some("proposed") => Review::Proposed, Some("confirmed") => Review::Confirmed, _ => Review::Unknown };
            out.push(Element { name: node.name().unwrap_or(&ifc_class).to_string(), ifc_class, review, min, max });
        }
        for child in node.children() { stack.push((child, world)); }
    }
    Ok(out)
}
