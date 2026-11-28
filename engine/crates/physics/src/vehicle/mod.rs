//! Four-wheel vehicles on Rapier. Games supply the car spec, ground and surface tuning.
mod car;
pub mod fleet;
mod spec;
pub mod tyre;
pub mod wheel_fx;
pub mod surfaces;
pub use car::{Car, CarStats, Ground, RoadTelemetry, SUBSTEPS, Wheel};
pub use spec::CarSpec;
use rapier3d::glamx::{Pose3, Vec3};
use rapier3d::prelude::*;

pub const ROAD: u128 = 255;
pub const GROUP_WORLD: Group = Group::GROUP_1;
pub const GROUP_CAR: Group = Group::GROUP_2;
pub const GROUP_GHOST: Group = Group::GROUP_3;
pub const HZ: f64 = 120.0;

/// A car body: mass, inertia and centre of mass from the spec, boxes
/// for collision. `contact` lets it hit other cars.
pub fn add_car(world: &mut PhysicsWorld, spec: &CarSpec, pose: Pose3, contact: bool) -> RigidBodyHandle {
    let mp = MassProperties::new(
        Vec3::new(spec.cg[0] as f32, spec.cg[1] as f32, spec.cg[2] as f32),
        spec.mass as f32,
        Vec3::new(spec.inertia[0] as f32, spec.inertia[1] as f32, spec.inertia[2] as f32),
    );
    let rb = RigidBodyBuilder::dynamic()
        .pose(pose)
        .additional_mass_properties(mp)
        .angular_damping(0.05)
        .linear_damping(0.0)
        .can_sleep(false)
        .ccd_enabled(true)
        .build();
    let h = world.bodies.insert(rb);
    for b in &spec.boxes {
        let c = ColliderBuilder::cuboid(b[0] as f32 * 0.5, b[1] as f32 * 0.5, b[2] as f32 * 0.5)
            .translation(Vec3::new(b[3] as f32, b[4] as f32, b[5] as f32))
            .density(0.0)
            .friction(0.45)
            .friction_combine_rule(CoefficientCombineRule::Min)
            .restitution(0.05)
            .collision_groups(car_groups(contact))
            .build();
        world.colliders.insert_with_parent(c, h, &mut world.bodies);
    }
    h
}

/// Cars collide with the world, and with each other unless ghosted.
pub fn car_groups(contact: bool) -> InteractionGroups {
    if contact {
        InteractionGroups::new(GROUP_CAR, GROUP_WORLD | GROUP_CAR, InteractionTestMode::And)
    } else {
        InteractionGroups::new(GROUP_GHOST, GROUP_WORLD, InteractionTestMode::And)
    }
}
