//! Dynamic objects without a renderer rebuild: reserve a block of instance
//! slots once, then add, move, re-mesh and remove objects in it at runtime.
//! An empty slot has no clusters, so culling skips it for free. Any mesh
//! already in the scene can go in any slot; new geometry or materials still
//! need a new renderer.
//!
//! ```ignore
//! let mut props = Slots::reserve(&mut scene, 256);
//! let renderer = Renderer::new(&gpu, &scene, &light);
//! let crate_id = props.add(&mut scene, crate_mesh, xf).expect("pool full");
//! props.set(&mut scene, crate_id, crate_mesh, moved);
//! props.remove(&mut scene, crate_id);
//! props.flush(&gpu, &renderer, &scene)?;   // once per frame
//! ```
//!
//! Smooth motion when the display outruns the simulation (120 Hz on a 60 Hz
//! tick): call [`Slots::tick`] at the start of every simulation tick and
//! [`Slots::flush_at`] with the frame's alpha instead of `flush`. Moving
//! slots are drawn between their previous and current tick; the scene keeps
//! the current one.
use crate::scene::{GpuInstance, Scene};
use glam::Affine3A;

/// A slot index inside a [`Slots`] block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotId(pub usize);

#[derive(Clone, Debug)]
pub struct Slots {
    /// First scene instance of the block.
    pub first: usize,
    used: Vec<bool>,
    free: Vec<usize>,
    dirty: Option<(usize, usize)>,
    /// Each slot's instance at the start of the current tick ([`Slots::tick`]).
    prev: Vec<Option<GpuInstance>>,
    /// Slots drawn between ticks last frame, rewritten with their current
    /// instance once they stop.
    blended: Vec<usize>,
}

/// Farther than this in one tick is a teleport, not motion: no blending.
const SNAP: f32 = 4.0;

fn affine(rows: &[[f32; 4]; 3]) -> Affine3A {
    let c = |j: usize| glam::Vec3A::new(rows[0][j], rows[1][j], rows[2][j]);
    Affine3A::from_cols(c(0), c(1), c(2), c(3))
}

/// `cur` drawn `alpha` of the way from `prev`; None when there is nothing to
/// blend (same place, another mesh, or a teleport).
fn between(prev: &GpuInstance, cur: &GpuInstance, alpha: f32) -> Option<GpuInstance> {
    if prev.rows == cur.rows || prev.first_cluster != cur.first_cluster || cur.cluster_count == 0 { return None; }
    let (s0, r0, t0) = affine(&prev.rows).to_scale_rotation_translation();
    let (s1, r1, t1) = affine(&cur.rows).to_scale_rotation_translation();
    if t0.distance(t1) > SNAP { return None; }
    let t = t0.lerp(t1, alpha);
    let m = glam::Mat4::from_scale_rotation_translation(s0.lerp(s1, alpha), r0.slerp(r1, alpha), t);
    let mut out = *cur;
    for (i, row) in out.rows.iter_mut().enumerate() { *row = m.row(i).to_array(); }
    let shift = t - t1;
    for k in 0..3 { out.sphere[k] += shift[k]; }
    Some(out)
}

fn empty(mut i: GpuInstance) -> GpuInstance {
    i.cluster_count = 0;
    i.sphere = [0.0; 4];
    i
}

impl Slots {
    /// Append `n` empty instances to the scene. Call before `Renderer::new`.
    pub fn reserve(scene: &mut Scene, n: usize) -> Self {
        assert!(!scene.meshes.is_empty(), "add at least one mesh before reserving slots");
        let first = scene.instances.len();
        for _ in 0..n {
            scene.add_instance(0, Affine3A::IDENTITY);
            let last = scene.instances.len() - 1;
            scene.instances[last] = empty(scene.instances[last]);
        }
        Self { first, used: vec![false; n], free: (0..n).rev().collect(), dirty: None, prev: vec![None; n], blended: Vec::new() }
    }

    pub fn capacity(&self) -> usize { self.used.len() }
    pub fn is_used(&self, id: SlotId) -> bool { self.used.get(id.0).copied().unwrap_or(false) }
    pub fn len(&self) -> usize { self.used.len() - self.free.len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    /// Scene instance index of a slot (for flags such as `hide_in_views`).
    pub fn instance(&self, id: SlotId) -> usize { self.first + id.0 }

    fn touch(&mut self, k: usize) {
        self.dirty = Some(self.dirty.map_or((k, k + 1), |(a, b)| (a.min(k), b.max(k + 1))));
    }

    /// Place a mesh in a free slot. None when the block is full.
    pub fn add(&mut self, scene: &mut Scene, mesh: usize, xf: Affine3A) -> Option<SlotId> {
        let k = self.free.pop()?;
        self.used[k] = true;
        // New this tick: it appears where it is.
        self.prev[k] = None;
        let flags = scene.instances[self.first + k].flags;
        scene.update_instance(self.first + k, mesh, xf);
        scene.instances[self.first + k].flags = flags;
        self.touch(k);
        Some(SlotId(k))
    }

    /// Move or re-mesh an occupied slot.
    pub fn set(&mut self, scene: &mut Scene, id: SlotId, mesh: usize, xf: Affine3A) {
        assert!(self.used[id.0], "slot {} is empty", id.0);
        scene.update_instance(self.first + id.0, mesh, xf);
        self.touch(id.0);
    }

    pub fn remove(&mut self, scene: &mut Scene, id: SlotId) {
        if !std::mem::replace(&mut self.used[id.0], false) { return; }
        self.prev[id.0] = None;
        let i = self.first + id.0;
        scene.instances[i] = empty(scene.instances[i]);
        self.free.push(id.0);
        self.touch(id.0);
    }

    pub fn clear(&mut self, scene: &mut Scene) {
        for k in 0..self.used.len() { if self.used[k] { self.remove(scene, SlotId(k)); } }
    }

    /// A simulation tick starts: what the slots show now is where moving
    /// slots are drawn from until the next tick.
    pub fn tick(&mut self, scene: &Scene) {
        for k in 0..self.used.len() {
            self.prev[k] = self.used[k].then(|| scene.instances[self.first + k]);
        }
    }

    /// [`Slots::flush`], drawing moving slots `alpha` (0..1) of the way from
    /// their previous tick to the current one.
    pub fn flush_at(&mut self, gpu: &crate::Gpu, renderer: &crate::Renderer, scene: &Scene, alpha: f32) -> Result<(), String> {
        // Last frame's blended slots get their current instance back first.
        for k in std::mem::take(&mut self.blended) { self.touch(k); }
        let mut out = Vec::new();
        for k in 0..self.used.len() {
            let Some(prev) = self.prev[k].as_ref().filter(|_| self.used[k]) else { continue };
            if let Some(i) = between(prev, &scene.instances[self.first + k], alpha.clamp(0.0, 1.0)) {
                out.push((k, i));
                self.blended.push(k);
                self.touch(k);
            }
        }
        let Some((a, b)) = self.dirty.take() else { return Ok(()) };
        if out.is_empty() { return renderer.update_instances(gpu, self.first + a, &scene.instances[self.first + a..self.first + b]); }
        let mut range = scene.instances[self.first + a..self.first + b].to_vec();
        for (k, i) in out { range[k - a] = i; }
        renderer.update_instances(gpu, self.first + a, &range)
    }

    /// Upload the changed range (one contiguous write) to the renderer.
    pub fn flush(&mut self, gpu: &crate::Gpu, renderer: &crate::Renderer, scene: &Scene) -> Result<(), String> {
        let Some((a, b)) = self.dirty.take() else { return Ok(()) };
        renderer.update_instances(gpu, self.first + a, &scene.instances[self.first + a..self.first + b])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Material;

    #[test]
    fn slots_add_move_remove_and_track_the_dirty_range() {
        let mut scene = Scene::default();
        let m = scene.add_material(Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&crate::primitives::cube(m).mesh());
        let mut slots = Slots::reserve(&mut scene, 4);
        assert!(scene.instances[slots.first..].iter().all(|i| i.cluster_count == 0));
        let a = slots.add(&mut scene, cube, Affine3A::from_translation(glam::Vec3::X)).unwrap();
        let b = slots.add(&mut scene, cube, Affine3A::IDENTITY).unwrap();
        assert_eq!((a, b, slots.len()), (SlotId(0), SlotId(1), 2));
        assert!(scene.instances[slots.instance(a)].cluster_count > 0);
        assert_eq!(slots.dirty, Some((0, 2)));
        slots.remove(&mut scene, a);
        assert_eq!(scene.instances[slots.instance(a)].cluster_count, 0);
        assert_eq!(slots.add(&mut scene, cube, Affine3A::IDENTITY), Some(a), "freed slots are reused");
        for _ in 0..2 { slots.add(&mut scene, cube, Affine3A::IDENTITY).unwrap(); }
        assert!(slots.add(&mut scene, cube, Affine3A::IDENTITY).is_none(), "full");
        slots.clear(&mut scene);
        assert!(slots.is_empty());
    }

    #[test]
    fn a_moving_slot_is_drawn_between_its_ticks_and_a_teleport_is_not() {
        let mut scene = Scene::default();
        let m = scene.add_material(Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&crate::primitives::cube(m).mesh());
        let mut slots = Slots::reserve(&mut scene, 2);
        let a = slots.add(&mut scene, cube, Affine3A::IDENTITY).unwrap();
        slots.tick(&scene);
        slots.set(&mut scene, a, cube, Affine3A::from_translation(glam::Vec3::new(1.0, 0.0, 0.0)));
        let prev = slots.prev[a.0].unwrap();
        let cur = scene.instances[slots.instance(a)];
        let half = between(&prev, &cur, 0.5).expect("moving");
        assert!((half.rows[0][3] - 0.5).abs() < 1e-5, "{:?}", half.rows);
        assert!((half.sphere[0] - (cur.sphere[0] - 0.5)).abs() < 1e-5);
        slots.tick(&scene);
        assert!(between(&slots.prev[a.0].unwrap(), &scene.instances[slots.instance(a)], 0.5).is_none(), "standing still");
        slots.set(&mut scene, a, cube, Affine3A::from_translation(glam::Vec3::new(50.0, 0.0, 0.0)));
        assert!(between(&slots.prev[a.0].unwrap(), &scene.instances[slots.instance(a)], 0.5).is_none(), "a teleport snaps");
    }
}
