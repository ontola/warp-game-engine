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
        Self { first, used: vec![false; n], free: (0..n).rev().collect(), dirty: None }
    }

    pub fn capacity(&self) -> usize { self.used.len() }
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
        let i = self.first + id.0;
        scene.instances[i] = empty(scene.instances[i]);
        self.free.push(id.0);
        self.touch(id.0);
    }

    pub fn clear(&mut self, scene: &mut Scene) {
        for k in 0..self.used.len() { if self.used[k] { self.remove(scene, SlotId(k)); } }
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
}
