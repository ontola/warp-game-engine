//! Arena geometry: boxes rotated about Y (Godot's `_add_static_box` with
//! `rotation_y`) tagged with what they are. Collision and rays come from
//! the engine's `kern::collide`.
use glam::Vec3;
pub use kern::collide::{ray_sphere, YawBox};
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
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub center: Vec3,
    pub half: Vec3,
    /// Rotation about +Y in radians (Godot convention: positive turns +X towards -Z).
    pub yaw: f32,
    pub kind: Kind,
}

impl Block {
    pub fn new(center: Vec3, size: Vec3, yaw: f32, kind: Kind) -> Self {
        Self { center, half: size * 0.5, yaw, kind }
    }
    pub fn shape(&self) -> YawBox { YawBox { center: self.center, half: self.half, yaw: self.yaw } }
    pub fn top(&self) -> f32 { self.center.y + self.half.y }
}

/// Closest hit among the arena's shapes: distance, normal, block index.
pub fn raycast(shapes: &[YawBox], origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
    kern::collide::raycast(shapes, origin, dir, max)
}
