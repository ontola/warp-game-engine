//! Rigid bodies for game prototypes on Rapier: static level boxes (the
//! same `kern::collide::YawBox` the character mover uses), dynamic boxes,
//! spheres and capsules, impulses, radial blasts, joints between bodies and
//! ray casts. Fixed steps and Rapier's enhanced determinism, so replays
//! match. Uses `glam` 0.31 types like the rest of the engine.
use glam::{Quat, Vec3};
use kern::collide::YawBox;
use rapier3d::glamx;
use rapier3d::prelude::*;

pub mod ragdoll;
mod static_set;
pub use static_set::StaticSet;

fn to(v: Vec3) -> glamx::Vec3 { glamx::Vec3::from_array(v.to_array()) }
fn from(v: glamx::Vec3) -> Vec3 { Vec3::from_array(v.to_array()) }
fn to_q(q: Quat) -> glamx::Quat { glamx::Quat::from_array(q.to_array()) }
fn from_q(q: glamx::Quat) -> Quat { Quat::from_array(q.to_array()) }

/// A dynamic body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Body(RigidBodyHandle);

/// A static collider (level geometry).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Static(ColliderHandle);

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// Half extents.
    Box(Vec3),
    Sphere(f32),
    /// Half height of the straight part, radius; along local Y.
    Capsule(f32, f32),
}

/// How a dynamic body is made.
#[derive(Clone, Copy, Debug)]
pub struct BodyDesc {
    pub shape: Shape,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub spin: Vec3,
    /// kg per cubic metre.
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    /// Fraction of velocity lost per second (air).
    pub damping: f32,
    /// Continuous collision for small fast bodies.
    pub ccd: bool,
}

impl BodyDesc {
    pub fn new(shape: Shape, pos: Vec3) -> Self {
        BodyDesc { shape, pos, rot: Quat::IDENTITY, vel: Vec3::ZERO, spin: Vec3::ZERO, density: 500.0, friction: 0.7, restitution: 0.2, damping: 0.05, ccd: false }
    }
}

pub struct Physics {
    pub world: PhysicsWorld,
    /// Simulation steps per `step` call (smaller steps, same dt total).
    pub substeps: u32,
}

impl Physics {
    /// `hz`: the fixed rate `step` is called at.
    pub fn new(gravity: Vec3, hz: f32) -> Self {
        let mut world = PhysicsWorld::default();
        world.gravity = to(gravity);
        world.integration_parameters.dt = 1.0 / hz;
        Physics { world, substeps: 1 }
    }

    pub fn add_static(&mut self, b: &YawBox) -> Static {
        let c = ColliderBuilder::cuboid(b.half.x, b.half.y, b.half.z)
            .position(Pose3::from_parts(to(b.center), glamx::Quat::from_rotation_y(b.yaw)))
            .friction(0.8)
            .build();
        Static(self.world.colliders.insert(c))
    }

    /// Remove one static collider (a destroyed piece of level).
    pub fn remove_static(&mut self, s: Static) { self.world.remove_collider(s.0); }

    /// Replace every static collider (a new level) with these boxes.
    pub fn set_statics(&mut self, boxes: &[YawBox]) -> Vec<Static> {
        let old: Vec<ColliderHandle> = self.world.colliders.iter().filter(|(_, c)| c.parent().is_none()).map(|(h, _)| h).collect();
        for h in old { self.world.remove_collider(h); }
        boxes.iter().map(|b| self.add_static(b)).collect()
    }

    pub fn add(&mut self, d: &BodyDesc) -> Body {
        let rb = RigidBodyBuilder::dynamic()
            .pose(Pose3::from_parts(to(d.pos), to_q(d.rot)))
            .linvel(to(d.vel))
            .angvel(to(d.spin))
            .linear_damping(d.damping)
            .angular_damping(d.damping)
            .ccd_enabled(d.ccd)
            .build();
        let h = self.world.bodies.insert(rb);
        let shape = match d.shape {
            Shape::Box(h) => ColliderBuilder::cuboid(h.x, h.y, h.z),
            Shape::Sphere(r) => ColliderBuilder::ball(r),
            Shape::Capsule(hh, r) => ColliderBuilder::capsule_y(hh, r),
        };
        let c = shape.density(d.density).friction(d.friction).restitution(d.restitution).build();
        self.world.colliders.insert_with_parent(c, h, &mut self.world.bodies);
        Body(h)
    }

    pub fn remove(&mut self, b: Body) {
        self.world.remove_body_with_colliders(b.0, true);
    }

    /// Remove every dynamic body (a new round).
    pub fn clear_bodies(&mut self) {
        let all: Vec<RigidBodyHandle> = self.world.bodies.iter().map(|(h, _)| h).collect();
        for h in all { self.remove(Body(h)); }
    }

    /// A ball joint: `a` and `b` stay connected at the given local anchors
    /// (ragdoll necks, chains).
    pub fn ball_joint(&mut self, a: Body, b: Body, anchor_a: Vec3, anchor_b: Vec3) {
        let j = SphericalJointBuilder::new().local_anchor1(to(anchor_a)).local_anchor2(to(anchor_b));
        self.world.impulse_joints.insert(a.0, b.0, j, true);
    }

    pub fn impulse(&mut self, b: Body, impulse: Vec3, at: Option<Vec3>) {
        if let Some(rb) = self.world.bodies.get_mut(b.0) {
            match at {
                Some(p) => rb.apply_impulse_at_point(to(impulse), to(p), true),
                None => rb.apply_impulse(to(impulse), true),
            }
        }
    }

    /// Push every body within `radius` of `center` away: `speed` m/s added
    /// at the centre (whatever the mass), falling linearly to zero at the
    /// edge, with a little lift. Returns how many bodies it moved.
    pub fn blast(&mut self, center: Vec3, radius: f32, speed: f32) -> usize {
        let mut n = 0;
        for (_, rb) in self.world.bodies.iter_mut() {
            if !rb.is_dynamic() { continue; }
            let p = from(rb.translation());
            let d = p - center;
            let dist = d.length();
            if dist >= radius { continue; }
            let dir = (d / dist.max(1e-3) + Vec3::Y * 0.3).normalize();
            let dv = dir * speed * (1.0 - dist / radius);
            rb.apply_impulse(to(dv * rb.mass()), true);
            n += 1;
        }
        n
    }

    /// One fixed step (split into `substeps`).
    pub fn step(&mut self) {
        let dt = self.world.integration_parameters.dt;
        if self.substeps > 1 { self.world.integration_parameters.dt = dt / self.substeps as f32; }
        for _ in 0..self.substeps.max(1) { self.world.step(); }
        self.world.integration_parameters.dt = dt;
    }

    pub fn pose(&self, b: Body) -> Option<(Vec3, Quat)> {
        self.world.bodies.get(b.0).map(|rb| (from(rb.translation()), from_q(*rb.rotation())))
    }

    pub fn velocity(&self, b: Body) -> Option<Vec3> { self.world.bodies.get(b.0).map(|rb| from(rb.linvel())) }

    pub fn sleeping(&self, b: Body) -> bool { self.world.bodies.get(b.0).is_none_or(|rb| rb.is_sleeping()) }

    pub fn body_count(&self) -> usize { self.world.bodies.len() }

    /// First hit along a ray: distance, normal and the body if it was one.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, Option<Body>)> {
        let ray = Ray::new(to(origin), to(dir));
        let (c, hit) = self.world.cast_ray_and_get_normal(&ray, max, true, QueryFilter::default())?;
        let body = self.world.colliders.get(c).and_then(|c| c.parent()).map(Body);
        Some((hit.time_of_impact, from(hit.normal), body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> YawBox { YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 1.0, 40.0), 0.0) }

    #[test]
    fn boxes_fall_settle_and_replay_identically() {
        let run = || {
            let mut p = Physics::new(Vec3::new(0.0, -9.8, 0.0), 60.0);
            p.add_static(&floor());
            let bodies: Vec<Body> = (0..5).map(|i| p.add(&BodyDesc { rot: Quat::from_rotation_z(0.3 * i as f32), ..BodyDesc::new(Shape::Box(Vec3::splat(0.25)), Vec3::new(i as f32 * 0.1, 1.0 + i as f32 * 0.6, 0.0)) })).collect();
            for _ in 0..240 { p.step(); }
            bodies.iter().map(|&b| p.pose(b).unwrap().0.to_array().map(f32::to_bits)).collect::<Vec<_>>()
        };
        let a = run();
        for pos in &a {
            let y = f32::from_bits(pos[1]);
            assert!((0.2..3.0).contains(&y), "resting on the floor or the stack: {y}");
        }
        assert_eq!(a, run(), "deterministic");
    }

    #[test]
    fn blast_pushes_away_and_rays_find_bodies() {
        let mut p = Physics::new(Vec3::new(0.0, -9.8, 0.0), 60.0);
        p.set_statics(&[floor()]);
        let b = p.add(&BodyDesc::new(Shape::Sphere(0.3), Vec3::new(2.0, 0.3, 0.0)));
        for _ in 0..30 { p.step(); }
        assert_eq!(p.blast(Vec3::ZERO, 5.0, 10.0), 1);
        p.step();
        let v = p.velocity(b).unwrap();
        assert!(v.x > 1.0 && v.y > 0.0, "{v}");
        let (t, n, hit) = p.raycast(Vec3::new(-5.0, 0.3, 0.0), Vec3::X, 20.0).unwrap();
        assert!(hit == Some(b) && t > 5.0 && n.x < -0.5, "{t} {n} {hit:?}");
        assert!(p.raycast(Vec3::new(0.0, 5.0, 10.0), Vec3::NEG_Y, 20.0).unwrap().2.is_none(), "the floor is static");
        p.clear_bodies();
        assert_eq!(p.body_count(), 0);
    }

    #[test]
    fn joints_hold_bodies_together() {
        let mut p = Physics::new(Vec3::new(0.0, -9.8, 0.0), 60.0);
        p.add_static(&floor());
        let torso = p.add(&BodyDesc::new(Shape::Capsule(0.3, 0.35), Vec3::new(0.0, 2.0, 0.0)));
        let head = p.add(&BodyDesc::new(Shape::Sphere(0.25), Vec3::new(0.0, 2.9, 0.0)));
        p.ball_joint(torso, head, Vec3::new(0.0, 0.65, 0.0), Vec3::new(0.0, -0.25, 0.0));
        p.impulse(head, Vec3::new(30.0, 0.0, 0.0), None);
        for _ in 0..180 { p.step(); }
        let (a, b) = (p.pose(torso).unwrap().0, p.pose(head).unwrap().0);
        assert!(a.distance(b) < 1.3, "still attached: {}", a.distance(b));
    }
}
