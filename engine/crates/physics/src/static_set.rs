//! Incremental static colliders for levels with stable piece indices.
use crate::{Physics, Static};
use kern::collide::YawBox;

/// Owns a collection of static collider handles in one `Physics` world.
/// Slot indices are keys: keep holes for destroyed pieces. An exact unchanged
/// box retains its collider and broadphase entry. Changed/missing boxes are
/// replaced; unrelated colliders and dynamic bodies are never cleared.
///
/// Like `Body`/`Static`, this collection belongs to the Physics world in which
/// it was created. Do not reuse it with a different/replaced world. `clear`
/// removes its colliders explicitly; dropping this handle collection does not.
/// Supply geometry changes through `sync`; directly mutating these colliders
/// through `Physics::world` bypasses the cached source shapes. Removing a
/// collider externally is supported and detected on the next synchronization.
#[derive(Default)]
pub struct StaticSet {
    shapes: Vec<Option<YawBox>>,
    handles: Vec<Option<Static>>,
}

fn same_box(a: Option<YawBox>, b: Option<YawBox>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.center.to_array().map(f32::to_bits) == b.center.to_array().map(f32::to_bits)
            && a.half.to_array().map(f32::to_bits) == b.half.to_array().map(f32::to_bits)
            && a.yaw.to_bits() == b.yaw.to_bits(),
        _ => false,
    }
}

impl StaticSet {
    /// Synchronize a complete keyed collection. Previously removed handles
    /// (e.g. destruction via `Physics::remove_static`) are recreated when the
    /// corresponding box returns. No epsilon comparison hides geometry edits.
    pub fn sync(&mut self, physics: &mut Physics, boxes: &[Option<YawBox>]) -> &[Option<Static>] {
        for handle in self.handles.drain(boxes.len().min(self.handles.len())..) {
            if let Some(handle) = handle { physics.remove_static(handle); }
        }
        self.handles.resize(boxes.len(), None);
        self.shapes.resize(boxes.len(), None);
        for (i, &shape) in boxes.iter().enumerate() {
            let live = self.handles[i].is_some_and(|h| physics.world.colliders.get(h.0).is_some());
            if same_box(self.shapes[i], shape) && live == shape.is_some() { continue; }
            if let Some(handle) = self.handles[i].take().filter(|_| live) { physics.remove_static(handle); }
            self.handles[i] = shape.as_ref().map(|b| physics.add_static(b));
            self.shapes[i] = shape;
        }
        &self.handles
    }

    pub fn clear(&mut self, physics: &mut Physics) { self.sync(physics, &[]); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BodyDesc, Shape};
    use glam::Vec3;

    fn block(x: f32) -> YawBox { YawBox::new(Vec3::new(x, 0.0, 0.0), Vec3::splat(2.0), 0.0) }

    #[test]
    fn unchanged_handles_survive_and_only_changed_slots_are_replaced() {
        let mut p = Physics::new(Vec3::NEG_Y * 9.8, 60.0);
        let mut set = StaticSet::default();
        let mut boxes = [Some(block(0.0)), None, Some(block(4.0))];
        let original = set.sync(&mut p, &boxes).to_vec();
        p.step();
        assert_eq!(set.sync(&mut p, &boxes), original);
        boxes[2].as_mut().unwrap().yaw = 0.3;
        let changed = set.sync(&mut p, &boxes);
        assert_eq!(changed[0], original[0]);
        assert_eq!(changed[1], None);
        assert_ne!(changed[2], original[2]);
        assert_eq!(p.world.colliders.len(), 2);
    }

    #[test]
    fn destruction_restore_shrink_and_clear_preserve_foreign_geometry_and_bodies() {
        let mut p = Physics::new(Vec3::NEG_Y * 9.8, 60.0);
        let foreign = p.add_static(&block(20.0));
        let body = p.add(&BodyDesc::new(Shape::Sphere(0.2), Vec3::new(20.0, 4.0, 0.0)));
        let mut set = StaticSet::default();
        let boxes = [Some(block(0.0)), Some(block(4.0))];
        let original = set.sync(&mut p, &boxes).to_vec();
        p.remove_static(original[0].unwrap());
        p.step();
        let restored = set.sync(&mut p, &boxes).to_vec();
        assert_ne!(restored[0], original[0]);
        assert_eq!(restored[1], original[1]);
        set.sync(&mut p, &[None]);
        p.step();
        assert_eq!(p.world.colliders.len(), 2); // unrelated static + sphere
        assert!(p.world.colliders.get(foreign.0).is_some());
        assert!(p.pose(body).is_some());
        set.sync(&mut p, &boxes);
        set.clear(&mut p);
        p.step();
        assert_eq!(p.world.colliders.len(), 2);
        assert_eq!(p.body_count(), 1);
    }

    #[test]
    fn updated_queries_and_contacts_work_and_repeat_deterministically() {
        let run = || {
            let mut p = Physics::new(Vec3::NEG_Y * 9.8, 60.0);
            let mut set = StaticSet::default();
            set.sync(&mut p, &[Some(block(0.0)), Some(block(5.0))]);
            p.step();
            assert!(p.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 8.0).is_some());
            set.sync(&mut p, &[None, Some(block(5.0))]);
            p.step();
            assert!(p.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 8.0).is_none());
            let mut changed = block(5.0);
            changed.center.y = -0.5;
            changed.half.y = 0.5;
            set.sync(&mut p, &[Some(block(-5.0)), Some(changed)]);
            let body = p.add(&BodyDesc::new(Shape::Sphere(0.25), Vec3::new(5.0, 3.0, 0.0)));
            for _ in 0..180 { p.step(); }
            let pos = p.pose(body).unwrap().0;
            assert!((0.20..0.30).contains(&pos.y), "sphere settled on updated box: {pos}");
            pos.to_array().map(f32::to_bits)
        };
        assert_eq!(run(), run());
    }
}
