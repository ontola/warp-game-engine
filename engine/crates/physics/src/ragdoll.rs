//! Articulated capsule bodies with interpolated world-space skin deltas.
//! A game supplies joint positions and maps its skin joints to these segments.
use super::{Body, BodyDesc, Physics, Shape, to};
use glam::{Mat4, Quat, Vec3};
use rapier3d::prelude::{Group, InteractionGroups, InteractionTestMode, SphericalJointBuilder};

/// One capsule in a connected, parent-before-child tree. Endpoints are world
/// joint positions and describe the capsule's outer tips; very short segments
/// become spheres. `anchor` attaches this segment to its parent in world space.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub start: Vec3,
    pub end: Vec3,
    pub radius: f32,
    pub parent: Option<usize>,
    pub anchor: Vec3,
}

#[derive(Clone, Copy)]
struct Pose { position: Vec3, rotation: Quat }
impl Pose {
    fn matrix(self) -> Mat4 { Mat4::from_rotation_translation(self.rotation, self.position) }
    fn interpolate(self, next: Self, alpha: f32) -> Self {
        let alpha = alpha.clamp(0.0, 1.0);
        Self { position: self.position.lerp(next.position, alpha), rotation: self.rotation.slerp(next.rotation, alpha) }
    }
}
struct Part { body: Body, initial_inverse: Mat4, previous: Pose, current: Pose }

/// Owns the body's handles, but not the physics world. Call [`Self::remove`]
/// before discarding it, or clear all bodies when discarding an entire level.
pub struct Ragdoll { parts: Vec<Part> }

impl Ragdoll {
    /// Spawn a connected capsule tree. `velocity` is the root center's linear
    /// velocity; `spin` is world angular velocity, also applied to limb centers.
    /// `collision_group` must contain one bit unique among active ragdolls.
    /// Its bodies ignore one another while colliding with other groups and
    /// ordinary static geometry. Invalid input creates no bodies or joints.
    pub fn spawn(phys: &mut Physics, segments: &[Segment], velocity: Vec3, spin: Vec3, collision_group: u32) -> Result<Self, String> {
        if segments.is_empty() || !collision_group.is_power_of_two() || !velocity.is_finite() || !spin.is_finite() {
            return Err("ragdoll needs segments, finite motion and one collision-group bit".into());
        }
        for (i, s) in segments.iter().enumerate() {
            if !s.start.is_finite() || !s.end.is_finite() || !s.anchor.is_finite() || !s.radius.is_finite() || s.radius <= 0.0
                || !(s.end - s.start).length().is_finite() || (s.end - s.start).length() < 1e-4
                || if i == 0 { s.parent.is_some() } else { s.parent.is_none_or(|parent| parent >= i) } {
                return Err(format!("invalid ragdoll segment {i}: finite dimensions and one parent-before-child tree required"));
            }
        }
        let groups = InteractionGroups::new(Group::from_bits_retain(collision_group), Group::from_bits_retain(!collision_group), InteractionTestMode::And);
        let root = (segments[0].start + segments[0].end) * 0.5;
        let mut parts: Vec<Part> = Vec::with_capacity(segments.len());
        for s in segments {
            let axis = s.end - s.start;
            let length = axis.length();
            let pose = Pose { position: (s.start + s.end) * 0.5, rotation: Quat::from_rotation_arc(Vec3::Y, axis / length) };
            let body = phys.add(&BodyDesc {
                rot: pose.rotation, vel: velocity + spin.cross(pose.position - root), spin,
                density: 300.0, friction: 0.85, restitution: 0.02, damping: 0.8, ccd: true,
                ..BodyDesc::new(Shape::Capsule((length * 0.5 - s.radius).max(0.0), s.radius), pose.position)
            });
            let rb = phys.world.bodies.get_mut(body.0).expect("new ragdoll body");
            rb.set_additional_solver_iterations(2);
            for &collider in rb.colliders() {
                phys.world.colliders.get_mut(collider).expect("new ragdoll collider").set_collision_groups(groups);
            }
            let initial_inverse = pose.matrix().inverse();
            if let Some(parent) = s.parent {
                let a = &parts[parent];
                let joint = SphericalJointBuilder::new()
                    .local_anchor1(to(a.initial_inverse.transform_point3(s.anchor)))
                    .local_anchor2(to(initial_inverse.transform_point3(s.anchor)))
                    .contacts_enabled(false);
                phys.world.impulse_joints.insert(a.body.0, body.0, joint, true);
            }
            parts.push(Part { body, initial_inverse, previous: pose, current: pose });
        }
        Ok(Self { parts })
    }

    pub fn len(&self) -> usize { self.parts.len() }
    pub fn is_empty(&self) -> bool { self.parts.is_empty() }
    /// Handle for optional impulses or diagnostics on a particular segment.
    pub fn body(&self, part: usize) -> Body { self.parts[part].body }

    /// Capture after each fixed physics step, preserving the preceding pose
    /// for rendering. A level reset may already have removed these handles.
    pub fn update(&mut self, phys: &Physics) {
        for part in &mut self.parts {
            part.previous = part.current;
            if let Some((position, rotation)) = phys.pose(part.body) { part.current = Pose { position, rotation }; }
        }
    }

    /// World rigid transform from a segment's initial pose to its interpolated
    /// pose. Apply it before the initial world skin transform, then transform
    /// back through the current model instance's inverse for a skin palette.
    pub fn delta(&self, part: usize, alpha: f32) -> Mat4 {
        let part = &self.parts[part];
        part.previous.interpolate(part.current, alpha).matrix() * part.initial_inverse
    }

    pub fn root_position(&self, alpha: f32) -> Vec3 {
        self.parts.first().map_or(Vec3::ZERO, |part| part.previous.interpolate(part.current, alpha).position)
    }

    /// Remove bodies, their colliders and attached joints. Safe after a global
    /// physics clear, and safe to call more than once.
    pub fn remove(&mut self, phys: &mut Physics) {
        for part in self.parts.drain(..) { phys.remove(part.body); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kern::collide::YawBox;

    fn segments() -> [Segment; 3] {
        [
            Segment { start: Vec3::new(0.0, 2.0, 0.0), end: Vec3::new(0.0, 2.6, 0.0), radius: 0.2, parent: None, anchor: Vec3::ZERO },
            Segment { start: Vec3::new(0.0, 2.6, 0.0), end: Vec3::new(0.6, 2.6, 0.0), radius: 0.1, parent: Some(0), anchor: Vec3::new(0.0, 2.6, 0.0) },
            Segment { start: Vec3::new(0.6, 2.6, 0.0), end: Vec3::new(1.1, 2.6, 0.0), radius: 0.09, parent: Some(1), anchor: Vec3::new(0.6, 2.6, 0.0) },
        ]
    }

    #[test]
    fn limbs_bend_independently_while_joint_anchors_remain_attached() {
        let mut phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), 60.0);
        let segments = segments();
        let mut doll = Ragdoll::spawn(&mut phys, &segments, Vec3::ZERO, Vec3::ZERO, 1).unwrap();
        for i in 0..doll.len() { assert!(doll.delta(i, 1.0).abs_diff_eq(Mat4::IDENTITY, 1e-5)); }
        phys.impulse(doll.body(2), Vec3::new(0.0, 0.0, 0.4), Some(segments[2].end));
        for _ in 0..30 {
            phys.step();
            doll.update(&phys);
            for (i, segment) in segments.iter().enumerate().skip(1) {
                let anchor_a = doll.delta(segment.parent.unwrap(), 1.0).transform_point3(segment.anchor);
                let anchor_b = doll.delta(i, 1.0).transform_point3(segment.anchor);
                assert!(anchor_a.distance(anchor_b) < 0.03, "detached joint {i}: {}", anchor_a.distance(anchor_b));
            }
        }
        assert!(!doll.delta(0, 1.0).abs_diff_eq(doll.delta(2, 1.0), 0.01), "limbs must not be one rigid frozen pose");
        let before = doll.root_position(1.0);
        phys.step();
        doll.update(&phys);
        assert!(doll.root_position(0.0).abs_diff_eq(before, 1e-6));
        assert!(doll.root_position(0.5).abs_diff_eq(before.lerp(doll.root_position(1.0), 0.5), 1e-6));
    }

    #[test]
    fn ragdoll_settles_on_the_floor_replays_and_removes_all_owned_physics() {
        let run = || {
            let mut phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), 60.0);
            phys.add_static(&YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(30.0, 1.0, 30.0), 0.0));
            let mut doll = Ragdoll::spawn(&mut phys, &segments(), Vec3::new(2.0, 1.0, 0.0), Vec3::new(0.5, 0.0, 1.0), 4).unwrap();
            for _ in 0..600 { phys.step(); doll.update(&phys); }
            let result: Vec<_> = (0..doll.len()).map(|i| {
                let (position, rotation) = phys.pose(doll.body(i)).unwrap();
                assert!(position.is_finite() && rotation.is_finite() && position.y > 0.04 && position.y < 1.0, "{position}");
                assert!(phys.velocity(doll.body(i)).unwrap().length() < 0.15, "ragdoll should settle");
                doll.delta(i, 1.0).to_cols_array().map(f32::to_bits)
            }).collect();
            doll.remove(&mut phys);
            doll.remove(&mut phys);
            assert!(doll.is_empty());
            assert_eq!(phys.body_count(), 0);
            assert_eq!(phys.world.impulse_joints.len(), 0);
            assert_eq!(phys.world.colliders.len(), 1, "retain the level floor");
            result
        };
        assert_eq!(run(), run(), "fixed-step replay must match exactly");
    }

    #[test]
    fn collision_groups_exclude_self_but_preserve_other_ragdolls_and_statics() {
        let mut phys = Physics::new(Vec3::ZERO, 60.0);
        let mut a = Ragdoll::spawn(&mut phys, &segments(), Vec3::ZERO, Vec3::ZERO, 1).unwrap();
        let b = Ragdoll::spawn(&mut phys, &segments(), Vec3::ZERO, Vec3::ZERO, 2).unwrap();
        let groups = |doll: &Ragdoll, i| {
            let body = phys.world.bodies.get(doll.body(i).0).unwrap();
            phys.world.colliders.get(body.colliders()[0]).unwrap().collision_groups()
        };
        assert!(!groups(&a, 0).test(groups(&a, 2)));
        assert!(groups(&a, 0).test(groups(&b, 2)));
        assert!(groups(&a, 0).test(InteractionGroups::all()));
        assert!(phys.world.impulse_joints.iter().all(|(_, joint)| !joint.data.contacts_enabled));
        phys.clear_bodies();
        a.update(&phys);
        a.remove(&mut phys);
        assert_eq!(phys.body_count(), 0);
    }

    #[test]
    fn invalid_descriptions_leave_the_world_unchanged() {
        let mut phys = Physics::new(Vec3::ZERO, 60.0);
        let mut bad = segments();
        bad[2].parent = Some(2);
        assert!(Ragdoll::spawn(&mut phys, &bad, Vec3::ZERO, Vec3::ZERO, 1).is_err());
        bad[2] = segments()[2];
        bad[2].radius = f32::NAN;
        assert!(Ragdoll::spawn(&mut phys, &bad, Vec3::ZERO, Vec3::ZERO, 1).is_err());
        assert!(Ragdoll::spawn(&mut phys, &segments(), Vec3::ZERO, Vec3::ZERO, 3).is_err());
        assert_eq!(phys.body_count(), 0);
        assert_eq!(phys.world.impulse_joints.len(), 0);
    }
}
