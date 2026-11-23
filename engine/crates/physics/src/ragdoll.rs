//! Articulated capsule bodies with interpolated world-space skin deltas.
//! A game supplies joint positions and maps its skin joints to these segments.
use super::{Body, BodyDesc, Physics, Shape, to, to_q};
use glam::{Mat4, Quat, Vec3};
use rapier3d::prelude::{Group, InteractionGroups, InteractionTestMode, SphericalJointBuilder, ImpulseJointHandle, JointAxis, MotorModel};

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

/// Game-authored passive response: radians and force-based spring coefficients.
/// These are approximations, not inferred anatomical measurements.
#[derive(Clone, Copy, Debug)]
pub struct AngularProfile {
    pub min: Vec3, pub max: Vec3, pub neutral: Vec3,
    pub soft_zone: f32, pub stiffness: f32, pub limit_stiffness: f32,
    pub damping: f32, pub max_torque: f32,
}
impl AngularProfile {
    fn valid(self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.neutral.is_finite()
            && self.neutral.cmpge(self.min).all() && self.neutral.cmple(self.max).all()
            && self.min.cmpgt(Vec3::splat(-std::f32::consts::PI)).all()
            && self.max.cmplt(Vec3::splat(std::f32::consts::PI)).all()
            && [self.soft_zone, self.stiffness, self.limit_stiffness, self.damping, self.max_torque]
                .iter().all(|x| x.is_finite() && *x >= 0.0)
            && self.soft_zone > 0.0
            && (self.max-self.min).cmpgt(Vec3::splat(2.0*self.soft_zone)).all()
    }
    // Sum the neutral spring and a quadratic edge spring into one solver motor.
    fn response(self, i: usize, angle: f32) -> (f32, f32) {
        let edge = angle.clamp(self.min[i]+self.soft_zone, self.max[i]-self.soft_zone);
        let ramp = ((angle-edge).abs()/self.soft_zone).clamp(0.0,1.0);
        let extra = self.limit_stiffness*ramp*ramp;
        let stiffness = self.stiffness+extra;
        let target = if stiffness > 0.0 { (self.neutral[i]*self.stiffness+edge*extra)/stiffness }
            else { self.neutral[i] };
        (target, stiffness)
    }
}
/// An optional, short-lived muscle drive in the same bind-relative joint frame
/// as the passive limits. The existing motor torque cap applies to the sum.
#[derive(Clone, Copy, Debug)]
pub struct JointBrace {
    pub target: Vec3,
    pub stiffness: f32,
    pub damping: f32,
    /// Time in seconds spent recruiting, holding and releasing the muscles.
    pub ramp: f32,
    pub hold: f32,
    pub release: f32,
}

impl JointBrace {
    fn valid(self, profile: AngularProfile) -> bool {
        self.target.is_finite() && self.target.cmpge(profile.min).all() && self.target.cmple(profile.max).all()
            && [self.stiffness,self.damping,self.ramp,self.hold,self.release].iter().all(|x| x.is_finite() && *x >= 0.)
            && self.ramp > 0. && self.release > 0. && self.stiffness > 0.
    }

    fn weight(self, age: f32) -> f32 {
        let t = (age / self.ramp).min((self.ramp + self.hold + self.release - age) / self.release).clamp(0., 1.);
        t*t*(3.-2.*t)
    }
}

pub(crate) struct PassiveJoint {
    handle: ImpulseJointHandle,
    profile: AngularProfile,
    brace: Option<(JointBrace, f32)>,
}

impl PassiveJoint {
    fn step(&mut self, dt: f32) -> AngularProfile {
        let mut profile = self.profile;
        if let Some((brace, age)) = &mut self.brace {
            *age += dt;
            if *age >= brace.ramp + brace.hold + brace.release {
                self.brace = None;
            } else {
                let weight = brace.weight(*age);
                let spring = brace.stiffness * weight;
                if spring > 0. {
                    let total = profile.stiffness + spring;
                    profile.neutral = profile.neutral.lerp(brace.target, spring / total);
                    profile.stiffness = total;
                    profile.damping += brace.damping * weight;
                }
            }
        }
        profile
    }
}

pub(crate) fn update_passive_joints(phys: &mut Physics) {
    phys.passive_joints.retain(|j| phys.world.impulse_joints.get(j.handle).is_some());
    let dt = phys.world.integration_parameters.dt;
    for passive in &mut phys.passive_joints {
        let profile = passive.step(dt);
        let Some(joint) = phys.world.impulse_joints.get_mut(passive.handle, false) else { continue };
        let Some(a) = phys.world.bodies.get(joint.body1()) else { continue };
        let Some(b) = phys.world.bodies.get(joint.body2()) else { continue };
        let qa = *a.rotation()*joint.data.local_frame1.rotation;
        let qb = *b.rotation()*joint.data.local_frame2.rotation;
        let mut q = qa.inverse()*qb;
        if q.w < 0.0 { q = -q; }
        for (i, axis) in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ].into_iter().enumerate() {
            // Same angular coordinate as Rapier's generic-joint motors.
            let angle = 2.0*q.to_array()[i].clamp(-1.0,1.0).asin();
            let (target, stiffness) = profile.response(i, angle);
            joint.data.set_motor_position(axis, target, stiffness, profile.damping);
        }
    }
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
struct Part { body: Body, initial_inverse: Mat4, previous: Pose, current: Pose, joint: Option<ImpulseJointHandle> }

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
            let joint = if let Some(parent) = s.parent {
                let a = &parts[parent];
                let joint = SphericalJointBuilder::new()
                    .local_anchor1(to(a.initial_inverse.transform_point3(s.anchor)))
                    .local_anchor2(to(initial_inverse.transform_point3(s.anchor)))
                    .contacts_enabled(false);
                Some(phys.world.impulse_joints.insert(a.body.0, body.0, joint, true))
            } else { None };
            parts.push(Part { body, initial_inverse, previous: pose, current: pose, joint });
        }
        Ok(Self { parts })
    }

    /// Local anatomical frames define zero independently of the death pose.
    pub fn set_angular_profile(&self, phys: &mut Physics, part: usize,
        parent_frame: Quat, child_frame: Quat, profile: AngularProfile) -> Result<(), String> {
        self.set_angular_profile_with_brace(phys, part, parent_frame, child_frame, profile, None)
    }

    /// Install the passive limits and optionally recruit a temporary local muscle
    /// spring. The solver reacts against the parent, with no drive on the root.
    pub fn set_angular_profile_with_brace(&self, phys: &mut Physics, part: usize,
        parent_frame: Quat, child_frame: Quat, profile: AngularProfile, brace: Option<JointBrace>) -> Result<(), String> {
        if brace.is_some_and(|drive| !drive.valid(profile)) || !profile.valid() || !parent_frame.is_finite() || !child_frame.is_finite()
            || (parent_frame.length_squared()-1.0).abs()>0.001
            || (child_frame.length_squared()-1.0).abs()>0.001 {
            return Err("invalid ragdoll angular profile or frame".into());
        }
        let handle = self.parts.get(part).and_then(|p| p.joint).ok_or("part has no parent joint")?;
        let joint = phys.world.impulse_joints.get_mut(handle, true).ok_or("ragdoll joint removed")?;
        joint.data.local_frame1.rotation = to_q(parent_frame);
        joint.data.local_frame2.rotation = to_q(child_frame);
        for (i, axis) in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ].into_iter().enumerate() {
            joint.data.set_limits(axis, [profile.min[i],profile.max[i]]);
            joint.data.set_motor_model(axis, MotorModel::ForceBased);
            joint.data.set_motor_max_force(axis, profile.max_torque);
            joint.data.set_motor_position(axis, profile.neutral[i], profile.stiffness, profile.damping);
        }
        // Default sleep thresholds cut off a deliberately weak rest spring
        // halfway through relaxation. Still allow settled corpses to sleep.
        for body in [joint.body1(), joint.body2()] {
            if let Some(rb) = phys.world.bodies.get_mut(body) {
                rb.activation_mut().normalized_linear_threshold = 0.02;
                rb.activation_mut().angular_threshold = 0.02;
            }
        }
        phys.passive_joints.retain(|p| p.handle != handle);
        phys.passive_joints.push(PassiveJoint { handle, profile, brace: brace.map(|b| (b, 0.)) });
        Ok(())
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
        phys.passive_joints.retain(|j| phys.world.impulse_joints.get(j.handle).is_some());
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

    fn passive_profile() -> AngularProfile {
        AngularProfile { min: Vec3::splat(-0.8), max: Vec3::splat(0.8),
            neutral: Vec3::new(0.25, 0.0, 0.0), soft_zone: 0.2,
            stiffness: 1.0, limit_stiffness: 20.0, damping: 0.5, max_torque: 15.0 }
    }

    #[test]
    fn passive_response_is_weak_inside_and_progressively_resists_both_edges() {
        let p = passive_profile();
        assert_eq!(p.response(0, 0.0), (0.25, 1.0));
        assert_eq!(p.response(0, 0.5), (0.25, 1.0));
        for sign in [-1.0, 1.0] {
            let mut previous = 0.0;
            for a in [0.61, 0.65, 0.7, 0.75, 0.8] {
                let angle = sign*a;
                let (target, stiffness) = p.response(0, angle);
                let restoring = sign*(angle-target)*stiffness;
                assert!(restoring > previous);
                previous = restoring;
            }
        }
    }

    #[test]
    fn passive_joint_returns_to_neutral_resists_impacts_and_cleans_up() {
        let run = || {
            let mut phys = Physics::new(Vec3::ZERO, 120.0);
            phys.substeps = 4;
            let seg = [
                Segment { start: Vec3::ZERO, end: Vec3::Y*0.6, radius: 0.12, parent: None, anchor: Vec3::ZERO },
                Segment { start: Vec3::Y*0.6, end: Vec3::Y*1.2, radius: 0.1, parent: Some(0), anchor: Vec3::Y*0.6 },
            ];
            let mut doll = Ragdoll::spawn(&mut phys, &seg, Vec3::ZERO, Vec3::ZERO, 1).unwrap();
            doll.set_angular_profile(&mut phys, 1, Quat::IDENTITY, Quat::IDENTITY, passive_profile()).unwrap();
            let relative = |p: &Physics| {
                let a = p.pose(doll.body(0)).unwrap().1;
                let b = p.pose(doll.body(1)).unwrap().1;
                let mut q = a.inverse()*b;
                if q.w < 0.0 { q = -q; }
                2.0*q.x.clamp(-1.0,1.0).asin()
            };
            for _ in 0..1200 { phys.step(); }
            assert!((relative(&phys)-0.25).abs()<0.06, "neutral angle {}", relative(&phys));
            phys.world.bodies.get_mut(doll.body(1).0).unwrap().apply_torque_impulse(to(Vec3::X*0.6), true);
            for _ in 0..1200 {
                phys.step();
                assert!(relative(&phys).abs()<0.95, "hard limit exceeded: {}", relative(&phys));
            }
            assert!((relative(&phys)-0.25).abs()<0.06);
            let bits = relative(&phys).to_bits();
            doll.remove(&mut phys);
            assert!(phys.passive_joints.is_empty());
            assert_eq!(phys.world.impulse_joints.len(), 0);
            bits
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn temporary_brace_flexes_under_torque_then_releases_without_translating_the_body() {
        let mut phys = Physics::new(Vec3::ZERO, 120.);
        phys.substeps = 4;
        let straight = [
            Segment { start: Vec3::ZERO, end: Vec3::Y*0.6, radius: 0.1, parent: None, anchor: Vec3::ZERO },
            Segment { start: Vec3::Y*0.6, end: Vec3::Y*1.2, radius: 0.1, parent: Some(0), anchor: Vec3::Y*0.6 },
        ];
        let rig = Ragdoll::spawn(&mut phys, &straight, Vec3::ZERO, Vec3::ZERO, 1).unwrap();
        let profile = AngularProfile { neutral: Vec3::ZERO, ..passive_profile() };
        let brace = JointBrace { target: Vec3::X*0.65, stiffness: 9., damping: 0.35,
            ramp: 0.15, hold: 0.35, release: 0.40 };
        assert_eq!(brace.weight(0.), 0.);
        assert_eq!(brace.weight(0.25), 1.);
        assert!(brace.weight(0.70) < 0.51);
        assert!(rig.set_angular_profile_with_brace(&mut phys, 1, Quat::IDENTITY, Quat::IDENTITY,
            profile, Some(JointBrace { target: Vec3::X*2., ..brace })).is_err());
        rig.set_angular_profile_with_brace(&mut phys, 1, Quat::IDENTITY, Quat::IDENTITY, profile, Some(brace)).unwrap();
        let rotation = |p: &Physics| {
            let a = p.pose(rig.body(0)).unwrap().1;
            let b = p.pose(rig.body(1)).unwrap().1;
            let q = a.inverse()*b;
            2.*(q.x*q.w.signum()).clamp(-1.,1.).asin()
        };
        let center = |p: &Physics| (p.pose(rig.body(0)).unwrap().0+p.pose(rig.body(1)).unwrap().0)*0.5;
        let start = center(&phys);
        for _ in 0..54 { phys.step(); }
        assert!(rotation(&phys)>0.30, "brace failed to bend the joint: {}",rotation(&phys));
        assert!(center(&phys).distance(start) < 0.002);
        for _ in 0..1000 { phys.step(); }
        assert!(phys.passive_joints[0].brace.is_none());
        assert!(rotation(&phys).abs()<0.06, "passive neutral was not restored");
        assert!(center(&phys).distance(start) < 0.003);
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
