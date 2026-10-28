//! Arena geometry: boxes rotated about Y (Godot's `_add_static_box` with
//! `rotation_y`) tagged with what they are. Collision and rays come from
//! the engine's `kern::collide`.
use glam::Vec3;
pub use kern::collide::{ray_sphere, BoxGrid, Boxes, YawBox};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Floor,
    Wall,
    Tower,
    Building,
    Ledge,
    Pillar,
    Cover,
    Platform,
    /// Lava-island rock: the island columns, their cover and planks
    /// (Godot `_mat_dark` in the all-floor-lava layout).
    Island,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub center: Vec3,
    pub half: Vec3,
    /// Rotation about +Y in radians (Godot convention: positive turns +X towards -Z).
    pub yaw: f32,
    pub kind: Kind,
    /// Split into 8 m tiles before chunking (Godot's
    /// `_add_destructible_region`: floor, perimeter walls, tower shafts).
    #[serde(default)]
    pub tiled: bool,
}

impl Block {
    pub fn new(center: Vec3, size: Vec3, yaw: f32, kind: Kind) -> Self {
        Self { center, half: size * 0.5, yaw, kind, tiled: false }
    }
    pub fn shape(&self) -> YawBox { YawBox { center: self.center, half: self.half, yaw: self.yaw } }
    pub fn top(&self) -> f32 { self.center.y + self.half.y }
    pub fn tiled(mut self) -> Self { self.tiled = true; self }
}

/// Closest hit: distance, normal, box index.
pub fn raycast<B: Boxes + ?Sized>(boxes: &B, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
    boxes.ray(origin, dir, max)
}
