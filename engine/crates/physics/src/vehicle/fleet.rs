//! A shared fixed-step road world for games with multiple drivers. The caller
//! supplies static collision boxes once and the elapsed simulation time per tick.
use super::{Car, CarSpec, Ground, RoadTelemetry, HZ, add_car, surfaces::{Surface, Surfaces}};
use glam::{EulerRot, Quat as Rotation, Vec3};
use kern::collide::YawBox;
use rapier3d::{glamx::{Pose3, Quat, Vec3 as V3}, prelude::*};
use std::collections::BTreeMap;

struct Pavement;
impl Ground for Pavement {
    fn road_surface(&self, _: V3, hint: i64) -> (Surface, i64) { (Surface::Asphalt, hint) }
    fn water_level(&self, _: f64, _: f64) -> f64 { f64::NEG_INFINITY }
}

#[derive(Clone, Copy, Default)]
pub struct Pedals {
    pub throttle: f64,
    pub brake: f64,
    pub steer: f64,
    pub handbrake: f64,
}

pub struct CarPose {
    /// Axle-height chassis origin, Euler angles in Warp's YXZ order and wheel motion.
    pub pos: Vec3,
    pub rot: Vec3,
    pub speed: f64,
    pub rpm: f64,
    pub torque: f64,
    pub road: RoadTelemetry,
    pub gear: i32,
    pub cut: bool,
    pub parked: bool,
    pub wheels: [(f64, f64, f64); 4],
}

pub struct Fleet {
    world: PhysicsWorld,
    cars: BTreeMap<u32, Car>,
    surfaces: Surfaces,
    next: u32,
    acc: f64,
}

impl Fleet {
    pub fn new(boxes: impl Iterator<Item = YawBox>) -> Self {
        let mut world = PhysicsWorld::default();
        world.gravity = V3::new(0., -9.8, 0.);
        world.integration_parameters.dt = (1. / HZ) as f32;
        for b in boxes {
            let half = b.half;
            let c = ColliderBuilder::cuboid(half.x, half.y, half.z)
                .position(Pose3::from_parts(V3::from_array(b.center.to_array()), Quat::from_rotation_y(b.yaw)))
                .user_data(Surface::Asphalt as u128)
                .collision_groups(InteractionGroups::new(super::GROUP_WORLD, Group::ALL, InteractionTestMode::And)).build();
            world.colliders.insert(c);
        }
        Self { world, cars: BTreeMap::new(), surfaces: Surfaces::default(), next: 1, acc: 0. }
    }

    /// Park on the supplied ground point. Parked cars remain solid to traffic.
    pub fn spawn(&mut self, spec: CarSpec, feet: Vec3, yaw: f32) -> u32 {
        let origin = V3::from_array(feet.to_array()) + V3::Y * spec.wheel_radius as f32;
        let handle = add_car(&mut self.world, &spec, Pose3::from_parts(origin, Quat::from_rotation_y(yaw)), true);
        self.world.bodies[handle].set_body_type(RigidBodyType::Fixed, false);
        let mut car = Car::new(spec, handle);
        car.hold = true;
        let id = self.next;
        self.next += 1;
        self.cars.insert(id, car);
        id
    }

    pub fn count(&self) -> usize { self.cars.len() }

    /// Release this chassis and all of its Rapier colliders. Surviving
    /// drivers keep their original handles, and retired IDs stay retired.
    pub fn remove(&mut self, id: u32) -> bool {
        let Some(car) = self.cars.remove(&id) else { return false; };
        self.world.remove_body_with_colliders(car.body, true);
        true
    }

    /// A missing driver engages the parking brake and freezes the car once it settles.
    pub fn drive(&mut self, id: u32, input: Option<Pedals>) -> bool {
        let Some(car) = self.cars.get_mut(&id) else { return false };
        car.hold = input.is_none();
        let p = input.unwrap_or_default();
        car.throttle = p.throttle.clamp(0., 1.);
        car.brake = p.brake.clamp(0., 1.);
        car.steer_input = p.steer.clamp(-1., 1.);
        car.handbrake = p.handbrake.clamp(0., 1.);
        if input.is_some() && self.world.bodies[car.body].is_fixed() {
            self.world.bodies[car.body].set_body_type(RigidBodyType::Dynamic, true);
        }
        true
    }

    pub fn step(&mut self, dt: f64) {
        self.acc += dt;
        let step = 1. / HZ;
        while self.acc + 1e-10 >= step {
            for car in self.cars.values_mut() {
                if self.world.bodies[car.body].is_fixed() { continue; }
                car.integrate(&mut self.world, &Pavement, &self.surfaces, step);
                if car.hold && car.on_ground == 4 && car.linvel(&self.world).length_squared() < 0.004
                    && car.angvel(&self.world).length_squared() < 0.004 {
                    let body = &mut self.world.bodies[car.body];
                    body.set_linvel(V3::ZERO, false);
                    body.set_angvel(V3::ZERO, false);
                    body.set_body_type(RigidBodyType::Fixed, false);
                    car.speed = 0.;
                }
            }
            self.world.step();
            self.acc -= step;
        }
    }

    pub fn pose(&self, id: u32) -> Option<CarPose> {
        let car = self.cars.get(&id)?;
        let p = car.pose(&self.world);
        let (yaw, pitch, roll) = Rotation::from_array(p.rotation.to_array()).to_euler(EulerRot::YXZ);
        Some(CarPose {
            pos: Vec3::from_array(p.translation.to_array()), rot: Vec3::new(pitch, yaw, roll),
            speed: car.speed, gear: car.gear, cut: car.cut(), rpm: car.rpm(), torque: car.engine_torque(),
            parked: self.world.bodies[car.body].is_fixed(),
            road: car.road_telemetry(),
            wheels: std::array::from_fn(|i| {
                let w = &car.wheels[i];
                let height = if self.world.bodies[car.body].is_fixed() && !w.contact { 0. } else { car.spec.droop_y + w.compression };
                (height, w.steer, -w.spin)
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_chassis_leaves_no_body_or_collider_and_ids_stay_stable() {
        let mut fleet = Fleet::new(std::iter::empty());
        let first = fleet.spawn(CarSpec::compact(), Vec3::ZERO, 0.);
        let body = fleet.cars[&first].body;
        let shapes = fleet.world.bodies[body].colliders().to_vec();
        let second = fleet.spawn(CarSpec::compact(), Vec3::X * 4., 0.);
        assert!(!shapes.is_empty() && fleet.remove(first) && !fleet.remove(first));
        assert!(fleet.world.bodies.get(body).is_none());
        assert!(shapes.iter().all(|h| fleet.world.colliders.get(*h).is_none()));
        assert!(fleet.pose(first).is_none() && fleet.pose(second).is_some());
        assert!(fleet.spawn(CarSpec::compact(), Vec3::ZERO, 0.) > second);
    }

    #[test]
    fn parked_cars_stay_put_and_drivers_share_the_120_hz_world() {
        fn run(hz: u32) -> (Vec3, f64) {
            let floor = YawBox::new(Vec3::new(0., -0.5, 0.), Vec3::new(80., 1., 400.), 0.);
            let mut fleet = Fleet::new([floor].into_iter());
            let first = fleet.spawn(CarSpec::compact(), Vec3::ZERO, 0.);
            let parked = fleet.spawn(CarSpec::compact(), Vec3::X * 4., 0.);
            fleet.drive(first, Some(Pedals { throttle: 1., ..Default::default() }));
            for _ in 0..3 * hz { fleet.step(1. / hz as f64); }
            let car = fleet.pose(first).unwrap();
            assert_eq!(fleet.pose(parked).unwrap().pos.x, 4.);
            assert!(car.pos.z < -3. && car.speed > 3. && car.rpm > 1500.);
            (car.pos, car.rpm)
        }
        assert_eq!(run(60), run(120));
    }
}
