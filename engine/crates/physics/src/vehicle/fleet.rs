//! A shared fixed-step road world for games with multiple drivers. The caller
//! supplies static collision boxes once and the elapsed simulation time per tick.
use super::{Car, CarSpec, Ground, RoadTelemetry, HZ, add_car, surfaces::{Surface, Surfaces}};
use glam::{EulerRot, Quat as Rotation, Vec3};
use kern::collide::YawBox;
use rapier3d::{glamx::{Pose3, Quat, Vec3 as V3}, prelude::*};
use std::{collections::BTreeMap, sync::{Arc, OnceLock}};

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

/// The road solver's collision shape and full poses before and after its last game tick.
#[derive(Clone)]
pub struct ChassisContact {
    hull: SharedShape,
    pub previous: (Vec3, Rotation),
    pub current: (Vec3, Rotation),
}

impl ChassisContact {
    pub fn same_hull(&self, other: &Self) -> bool { Arc::ptr_eq(&self.hull.0, &other.hull.0) }

    /// Mirror the very same shapes in a secondary simulation (e.g. nearby ragdolls).
    pub fn add_kinematic(&self, physics: &mut crate::Physics) -> crate::Body {
        physics.add_kinematic_shape(self.previous.0, self.previous.1, self.hull.clone())
    }
}

/// Cross-section of a vehicle's contact skin in chassis-local coordinates.
pub struct HullSection { pub z: f32, pub width: f32, pub top: f32, pub roof_width: f32 }

struct FleetCar {
    car: Car,
    previous: Pose3,
    // The compact's lower body and cabin are already cooked by the road solver.
    // Assemble their shared handles only for consumers that need the whole hull.
    hull: OnceLock<SharedShape>,
}

pub struct Fleet {
    world: PhysicsWorld,
    cars: BTreeMap<u32, FleetCar>,
    contact_profiles: BTreeMap<String, SharedShape>,
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
        Self { world, cars: BTreeMap::new(), contact_profiles: BTreeMap::new(), surfaces: Surfaces::default(), next: 1, acc: 0. }
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
        let previous = car.pose(&self.world);
        self.cars.insert(id, FleetCar { car, previous, hull: OnceLock::new() });
        id
    }

    pub fn count(&self) -> usize { self.cars.len() }

    /// Release this chassis and all of its Rapier colliders. Surviving
    /// drivers keep their original handles, and retired IDs stay retired.
    pub fn remove(&mut self, id: u32) -> bool {
        let Some(entry) = self.cars.remove(&id) else { return false; };
        self.world.remove_body_with_colliders(entry.car.body, true);
        true
    }

    /// A missing driver engages the parking brake and freezes the car once it settles.
    pub fn drive(&mut self, id: u32, input: Option<Pedals>) -> bool {
        let Some(entry) = self.cars.get_mut(&id) else { return false };
        let car = &mut entry.car;
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
        for entry in self.cars.values_mut() { entry.previous = entry.car.pose(&self.world); }
        self.acc += dt;
        let step = 1. / HZ;
        while self.acc + 1e-10 >= step {
            for entry in self.cars.values_mut() {
                let car = &mut entry.car;
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

    /// An optional low-poly secondary contact skin, shared by every instance.
    /// The road solver keeps the vehicle spec's chassis mass and driving contacts.
    pub fn set_contact_profile(&mut self, name: String, bottom: f32, sections: &[HullSection]) -> Result<(), String> {
        if name.is_empty() || name.len() > 64 || (!self.contact_profiles.contains_key(&name) && self.contact_profiles.len() >= 64) {
            return Err("vehicles.contact_profile needs a name up to 64 bytes and at most 64 profiles".into());
        }
        if !bottom.is_finite() || !(2..=20).contains(&sections.len()) ||
            sections.iter().any(|s| !s.z.is_finite() || !s.top.is_finite() || !s.width.is_finite() || !s.roof_width.is_finite()
                || s.top <= bottom || s.width <= 0. || s.roof_width <= 0.) ||
            sections.windows(2).any(|w| w[1].z - w[0].z < 0.001) {
            return Err("vehicles.contact_profile needs 2..20 finite ordered sections with positive widths and top above bottom".into());
        }
        // Build a single watertight surface. Interior faces between adjoining
        // hull sections would catch capsules sliding from the nose up the hood.
        let mut vertices = Vec::with_capacity(sections.len() * 4);
        let mut triangles = vec![[0,2,1],[0,3,2]];
        for (i,s) in sections.iter().enumerate() {
            let (w,r) = (s.width * 0.5, s.roof_width * 0.5);
            vertices.extend([V3::new(-w,bottom,s.z), V3::new(w,bottom,s.z), V3::new(r,s.top,s.z), V3::new(-r,s.top,s.z)]);
            let rear = i as u32 * 4;
            if i > 0 {
                let front = rear - 4;
                for j in 0..4 { let k = (j + 1) % 4; triangles.extend([[front+j,front+k,rear+k],[front+j,rear+k,rear+j]]); }
            }
            if i + 1 == sections.len() { triangles.extend([[rear,rear+1,rear+2],[rear,rear+2,rear+3]]); }
        }
        let hull = SharedShape::trimesh_with_flags(vertices, triangles, TriMeshFlags::FIX_INTERNAL_EDGES)
            .map_err(|e| format!("vehicle contact mesh: {e}"))?;
        self.contact_profiles.insert(name, hull);
        Ok(())
    }

    pub fn chassis_contact(&self, id: u32, profile: Option<&str>) -> Option<ChassisContact> {
        let entry = self.cars.get(&id)?;
        let attached = self.world.bodies[entry.car.body].colliders();
        if attached.is_empty() { return None; }
        let hull = match profile {
            Some(name) => self.contact_profiles.get(name)?,
            None => entry.hull.get_or_init(|| SharedShape::compound(attached.iter().map(|handle| {
                let c = &self.world.colliders[*handle];
                (*c.position_wrt_parent().expect("chassis collider"), c.shared_shape().clone())
            }).collect())),
        }.clone();
        let pose = |p: Pose3| (crate::from(p.translation), crate::from_q(p.rotation));
        Some(ChassisContact { hull, previous: pose(entry.previous), current: pose(entry.car.pose(&self.world)) })
    }

    pub fn pose(&self, id: u32) -> Option<CarPose> {
        let car = &self.cars.get(&id)?.car;
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
    fn a_named_contact_skin_is_shared_slopes_between_sections_and_updates_atomically() {
        let mut fleet = Fleet::new(std::iter::empty());
        let car = fleet.spawn(CarSpec::compact(), Vec3::ZERO, 0.);
        let other = fleet.spawn(CarSpec::compact(), Vec3::X * 10., 0.);
        let section = |z, top| HullSection { z, top, width: 1.6, roof_width: 1.4 };
        let mesh = [section(-2.2, 0.2), section(-1.4, 0.6), section(1.0, 1.)];
        fleet.set_contact_profile("compact".into(), -0.1, &mesh).unwrap();
        let contact = fleet.chassis_contact(car, Some("compact")).unwrap();
        assert!(contact.same_hull(&fleet.chassis_contact(other, Some("compact")).unwrap()));
        assert!(!contact.same_hull(&fleet.chassis_contact(car, None).unwrap()));
        let mut secondary = crate::Physics::new(Vec3::ZERO, 60.);
        let body = contact.add_kinematic(&mut secondary);
        let (_, point, normal) = secondary.raycast_bodies(Vec3::new(0., 3., -1.8), Vec3::new(0., 0., -1.8), &[body]).unwrap();
        assert!((point.y - 0.73).abs() < 0.001 && normal.y > 0.8 && normal.z < -0.3);
        let bad = [section(1.0, 0.6), section(0.0, 0.5)];
        assert!(fleet.set_contact_profile("compact".into(), -0.1, &bad).is_err());
        assert!(contact.same_hull(&fleet.chassis_contact(car, Some("compact")).unwrap()));
        assert!(fleet.chassis_contact(car, Some("missing")).is_none());
    }

    #[test]
    fn shared_chassis_keeps_the_hood_below_the_cabin_and_tracks_road_poses() {
        let road = YawBox::new(Vec3::new(0., -0.5, 0.), Vec3::new(40., 1., 200.), 0.);
        let mut fleet = Fleet::new([road].into_iter());
        let id = fleet.spawn(CarSpec::compact(), Vec3::ZERO, 0.);
        let initial = fleet.chassis_contact(id, None).unwrap();
        let mut secondary = crate::Physics::new(Vec3::ZERO, 60.);
        let body = initial.add_kinematic(&mut secondary);
        let height = |z| secondary.raycast_bodies(Vec3::new(0., 3., z), Vec3::new(0., 0., z), &[body]).unwrap().1.y;
        assert!((height(-1.6) - 0.94).abs() < 0.01);
        assert!((height(0.0) - 1.47).abs() < 0.01);
        fleet.drive(id, Some(Pedals { throttle: 1., ..Default::default() }));
        let mut previous = initial.current;
        for _ in 0..90 {
            fleet.step(1. / 60.);
            let next = fleet.chassis_contact(id, None).unwrap();
            assert!(initial.same_hull(&next));
            assert_eq!(next.previous, previous);
            previous = next.current;
        }
        assert!(previous.0.z < -1.0);
        fleet.remove(id);
        assert!(fleet.chassis_contact(id, None).is_none());
    }

    #[test]
    fn removed_chassis_leaves_no_body_or_collider_and_ids_stay_stable() {
        let mut fleet = Fleet::new(std::iter::empty());
        let first = fleet.spawn(CarSpec::compact(), Vec3::ZERO, 0.);
        let body = fleet.cars[&first].car.body;
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
