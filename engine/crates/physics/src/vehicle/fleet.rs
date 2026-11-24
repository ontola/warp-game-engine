//! A shared fixed-step road world for games with multiple drivers. The caller
//! supplies static collision boxes once and the elapsed simulation time per tick.
use super::{Car, CarSpec, Ground, RoadTelemetry, HZ, add_car, surfaces::{Surface, Surfaces}};
use glam::{EulerRot, Quat as Rotation, Vec3};
use kern::collide::YawBox;
use rapier3d::{glamx::{Pose3, Quat, Vec3 as V3}, prelude::*};
use std::{collections::BTreeMap, sync::{Arc, OnceLock}};

#[derive(Default)]
struct FleetGround { grids: Vec<(ColliderHandle, crate::terrain::PlacedGrid)> }
impl Ground for FleetGround {
    fn road_surface(&self, p: V3, hint: i64) -> (Surface, i64) {
        // Only terrain colliders carry the ROAD tag. Pick the support nearest the
        // contact point when two patches overlap in horizontal coordinates.
        if let [(_, grid)] = self.grids.as_slice() { return (grid.surface_at(p).unwrap_or(Surface::Asphalt), hint); }
        self.grids.iter().enumerate().filter_map(|(i, (_, g))| {
            Some(((g.floor(p.x,p.z)?-p.y).abs(),g.surface_at(p)?,i as i64))
        }).min_by(|a,b| a.0.total_cmp(&b.0)).map_or((Surface::Asphalt,hint), |(_,s,i)| (s,i))
    }
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

    fn pose(&self) -> Pose3 { Pose3::from_parts(crate::to(self.current.0), crate::to_q(self.current.1)) }

    /// Intersect the fitted skin in its current full chassis pose.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, limit: f32) -> Option<f32> {
        let dir = dir.try_normalize()?;
        self.hull.cast_ray(&self.pose(), &Ray::new(crate::to(origin), crate::to(dir)), limit, true)
    }

    pub fn closest_point(&self, point: Vec3) -> Vec3 {
        crate::from(self.hull.project_point(&self.pose(), crate::to(point), false).point)
    }

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
    query_skin: Option<(String, SharedShape)>,
}

pub struct Fleet {
    world: PhysicsWorld,
    cars: BTreeMap<u32, FleetCar>,
    contact_profiles: BTreeMap<String, SharedShape>,
    /// Model-space triangles split once per fitted body and grid, shared by all paints.
    crush_grids: BTreeMap<String, (kern::crush::Grid, Vec<(usize, [Vec3; 3])>)>,
    dent_templates: BTreeMap<String,(Vec<Vec3>,Vec<[u32;3]>)>,
    panel_surfaces: BTreeMap<String,(Vec<Vec3>,Vec<([u32;3],u8)>)>,
    surfaces: Surfaces,
    ground: FleetGround,
    static_regions: BTreeMap<String, Vec<ColliderHandle>>,
    next: u32,
    acc: f64,
    /// Strongest hard contact per vehicle across this game tick's fixed steps.
    impacts: BTreeMap<u32, (Vec3, f32)>,
}

impl Fleet {
    pub fn new(boxes: impl Iterator<Item = YawBox>) -> Self {
        let mut world = PhysicsWorld::default();
        world.gravity = V3::new(0., -9.8, 0.);
        world.integration_parameters.dt = (1. / HZ) as f32;
        for b in boxes { world.colliders.insert(Self::box_collider(b)); }
        Self { world, cars: BTreeMap::new(), contact_profiles: BTreeMap::new(), crush_grids: BTreeMap::new(), dent_templates:BTreeMap::new(), panel_surfaces:BTreeMap::new(), surfaces: Surfaces::default(), ground: FleetGround::default(), static_regions: BTreeMap::new(), next: 1, acc: 0., impacts: BTreeMap::new() }
    }

    fn box_collider(b: YawBox) -> Collider {
        ColliderBuilder::cuboid(b.half.x, b.half.y, b.half.z)
            .position(Pose3::from_parts(crate::to(b.center), Quat::from_rotation_y(b.yaw)))
            .user_data(Surface::Asphalt as u128)
            .collision_groups(InteractionGroups::new(super::GROUP_WORLD, Group::ALL, InteractionTestMode::And)).build()
    }

    fn insert_heightfield(&mut self, grid: crate::terrain::PlacedGrid) -> ColliderHandle {
        let collider = grid.collider().user_data(super::ROAD)
            .collision_groups(InteractionGroups::new(super::GROUP_WORLD, Group::ALL, InteractionTestMode::And));
        let handle = self.world.colliders.insert(collider);
        self.ground.grids.push((handle, grid));
        handle
    }

    pub fn add_heightfield(&mut self, grid: crate::terrain::PlacedGrid) {
        self.insert_heightfield(grid);
    }

    /// Replace one streamed region while keeping car bodies, their velocity, and other regions.
    /// All input comes from a validated snapshot, before removing any existing colliders.
    pub fn set_static_region(&mut self, name: &str, boxes: Vec<YawBox>, grids: Vec<crate::terrain::PlacedGrid>) -> Result<(), String> {
        let count = boxes.len() + grids.len();
        let previous = self.static_regions.get(name).map_or(0, Vec::len);
        let total = self.static_regions.values().map(Vec::len).sum::<usize>() - previous + count;
        if name.is_empty() || name.len() > 64 || total > 16_384 ||
            (count > 0 && previous == 0 && self.static_regions.len() >= 64) {
            return Err("invalid vehicle static region name or collider budget".into());
        }
        if let Some(old) = self.static_regions.remove(name) {
            for handle in old { self.world.remove_collider(handle); }
            self.ground.grids.retain(|(h, _)| self.world.colliders.contains(*h));
        }
        if count > 0 {
            let mut handles = Vec::with_capacity(count);
            handles.extend(boxes.into_iter().map(|b| self.world.colliders.insert(Self::box_collider(b))));
            handles.extend(grids.into_iter().map(|g| self.insert_heightfield(g)));
            self.static_regions.insert(name.to_owned(), handles);
        }
        Ok(())
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
        self.world.bodies[handle].user_data = id.into();
        let previous = car.pose(&self.world);
        self.cars.insert(id, FleetCar { car, previous, hull: OnceLock::new(), query_skin: None });
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

    /// Move an existing car at the simulation boundary, keeping the drivetrain and velocity.
    pub fn relocate(&mut self, id: u32, feet: Vec3, yaw: Option<f32>) -> bool {
        let Some(entry) = self.cars.get_mut(&id) else { return false };
        let body = &mut self.world.bodies[entry.car.body];
        let pos = crate::to(feet) + V3::Y * entry.car.spec.wheel_radius as f32;
        let rot = yaw.map_or(body.position().rotation, Quat::from_rotation_y);
        let pose = Pose3::from_parts(pos, rot);
        body.set_position(pose, true);
        entry.previous = pose;
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
        self.impacts.clear();
        for entry in self.cars.values_mut() { entry.previous = entry.car.pose(&self.world); }
        self.acc += dt;
        let step = 1. / HZ;
        while self.acc + 1e-10 >= step {
            for entry in self.cars.values_mut() {
                let car = &mut entry.car;
                if self.world.bodies[car.body].is_fixed() { continue; }
                car.integrate(&mut self.world, &self.ground, &self.surfaces, step);
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
            self.record_impacts();
            self.acc -= step;
        }
    }

    /// Report the shared contact anchor and normal change in speed (m/s).
    /// The resting and suspension loads are below the reporting threshold.
    fn record_impacts(&mut self) {
        for pair in self.world.narrow_phase.contact_pairs() {
            let impulse = pair.total_impulse_magnitude();
            if impulse <= 0. { continue; }
            let a = &self.world.colliders[pair.collider1];
            let b = &self.world.colliders[pair.collider2];
            for collider in [a, b] {
                let Some(body) = collider.parent() else { continue; };
                let id = self.world.bodies[body].user_data as u32;
                let Some(entry) = self.cars.get(&id) else { continue; };
                let delta_v = impulse / entry.car.spec.mass as f32;
                if delta_v <= 1.2 || self.impacts.get(&id).is_some_and(|&(_, last)| last >= delta_v) { continue; }
                let Some((_, hit)) = pair.find_deepest_contact() else { continue; };
                let point = (a.position().transform_point(hit.local_p1) + b.position().transform_point(hit.local_p2)) * 0.5;
                self.impacts.insert(id, (crate::from(point), delta_v));
            }
        }
    }

    pub fn impacts(&self) -> &BTreeMap<u32, (Vec3, f32)> { &self.impacts }

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
        self.crush_grids.remove(&name);
        self.dent_templates.remove(&name);
        for entry in self.cars.values_mut() {
            if entry.query_skin.as_ref().is_some_and(|(n, _)| n == &name) { entry.query_skin = None; }
        }
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

    /// Refit the small gameplay query skin only on a hit. Road solver mass,
    /// suspension and the chassis used to support ragdolls stay stable.
    pub fn crush_query(&mut self, id: u32, profile: &str, grid: kern::crush::Grid, scales: [f32; 4]) -> Result<(), String> {
        if !grid.valid() || !scales.iter().all(|s| (0.05..=1.).contains(s)) { return Err("invalid crush grid or scale".into()); }
        let Some(entry) = self.cars.get_mut(&id) else { return Err("unknown vehicle".into()); };
        let mesh = self.contact_profiles.get(profile).and_then(|shape| shape.as_trimesh()).ok_or("unknown contact profile")?;
        if !self.crush_grids.get(profile).is_some_and(|cached| cached.0 == grid) {
            let (mut poly, mut work, mut pieces) = (Vec::new(), Vec::new(), Vec::new());
            for indices in mesh.indices() {
                let tri = indices.map(|i| crate::from(mesh.vertices()[i as usize]));
                grid.split(tri, &mut poly, &mut work, |cell, tri| pieces.push((cell, tri)));
            }
            self.crush_grids.insert(profile.to_owned(), (grid, pieces));
        }
        let pieces = &self.crush_grids[profile].1;
        let mut vertices = Vec::with_capacity(pieces.len() * 3);
        let mut indices = Vec::with_capacity(pieces.len());
        let [left,right,front,rear] = scales;
        for &(cell, tri) in pieces {
            let scale = Vec3::new([left,1.,right][cell % 3],1.,[front,1.,rear][cell / 3]);
            let base=vertices.len() as u32;
            vertices.extend(tri.map(|p| crate::to(kern::crush::deform(p, grid.hinge(cell), scale))));
            indices.push([base,base+1,base+2]);
        }
        let hull = SharedShape::trimesh(vertices,indices).map_err(|e| format!("crushed contact mesh: {e}"))?;
        entry.query_skin=Some((profile.to_owned(),hull));
        Ok(())
    }

    pub fn has_panel_surface(&self,name:&str)->bool { self.panel_surfaces.contains_key(name) }

    pub fn set_panel_surface(&mut self,name:String,vertices:Vec<Vec3>,faces:Vec<([u32;3],u8)>) {
        self.panel_surfaces.insert(name,(vertices,faces));
    }

    pub fn cut_query(&mut self,id:u32,profile:&str,model:&str,dents:&[kern::crush::Dent],mask:u32)->Result<(),String>{
        let car=self.cars.get_mut(&id).ok_or("unknown vehicle")?;
        let (rest,faces)=self.panel_surfaces.get(model).ok_or("unknown panel model")?;
        let points=rest.iter().map(|&p|crate::to(kern::crush::dent_vertex(p,Vec3::Y,dents).0)).collect();
        let faces=faces.iter().filter(|(_,part)|*part==0 || mask&(1u32<<*part)==0).map(|&(tri,_)|tri).collect();
        let shape=SharedShape::trimesh(points,faces).map_err(|e|format!("cut contact mesh: {e}"))?;
        car.query_skin=Some((profile.to_owned(),shape));
        Ok(())
    }

    /// Dense, shared sampling of the fitted sheet. Geometry changes only when a
    /// car receives another hit; there is no per-road-tick mesh cooking.
    pub fn dent_query(&mut self,id:u32,name:&str,dents:&[kern::crush::Dent])->Result<(),String>{
        let entry=self.cars.get_mut(&id).ok_or("unknown vehicle")?;
        let original=self.contact_profiles.get(name).and_then(|s|s.as_trimesh()).ok_or("unknown contact profile")?;
        let (base,indices)=self.dent_templates.entry(name.to_owned()).or_insert_with(||{
            let mut verts=Vec::new();let mut indices=Vec::new();
            let mut unique=std::collections::HashMap::<[u32;3],u32>::new();
            let mut queue:Vec<_>=original.indices().iter().map(|tri|tri.map(|i|crate::from(original.vertices()[i as usize]))).collect();
            while let Some([a,b,c])=queue.pop(){
                let d=[a.distance_squared(b),b.distance_squared(c),c.distance_squared(a)];
                let (x,y,z)=if d[0]>=d[1] && d[0]>=d[2]{(a,b,c)}else if d[1]>=d[2]{(b,c,a)}else{(c,a,b)};
                if x.distance_squared(y)>0.24*0.24 {
                    let mid=(x+y)*0.5; queue.extend([[x,mid,z],[mid,y,z]]);
                }else{
                    indices.push([a,b,c].map(|v|*unique.entry(v.to_array().map(f32::to_bits)).or_insert_with(||{let idx=verts.len() as u32;verts.push(v);idx})));
                }
            }
            (verts,indices)
        });
        let points=base.iter().map(|&p|crate::to(kern::crush::dent_vertex(p,Vec3::Y,dents).0)).collect();
        let shape=SharedShape::trimesh(points,indices.clone()).map_err(|e|format!("dented contact mesh: {e}"))?;
        entry.query_skin=Some((name.to_owned(),shape));
        Ok(())
    }

    pub fn query_contact(&self, id: u32, name: &str) -> Option<ChassisContact> {
        let mut contact = self.chassis_contact(id, Some(name))?;
        if let Some((n,hull)) = &self.cars.get(&id)?.query_skin {
            if n == name { contact.hull = hull.clone(); }
        }
        Some(contact)
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
    fn replacing_a_region_keeps_other_roads_and_car_motion() {
        let ground = |surface| {
            let grid = crate::terrain::HeightGrid::new(2, 8., &[0.; 4], surface, &[]).unwrap();
            let center = grid.center_offset();
            crate::terrain::PlacedGrid::new(Arc::new(grid), Rotation::IDENTITY, center)
        };
        let fixed = YawBox::new(Vec3::new(0., -0.5, 0.), Vec3::new(80., 1., 400.), 0.);
        let mut fleet = Fleet::new([fixed].into_iter());
        let id = fleet.spawn(CarSpec::compact(), Vec3::X, 0.);
        fleet.drive(id, Some(Pedals {throttle: 1., ..Default::default()}));
        for _ in 0..100 { fleet.step(1./60.); }
        let before = fleet.pose(id).unwrap();
        assert!(before.speed > 1.);
        fleet.set_static_region("hill", vec![], vec![ground(Surface::Grass)]).unwrap();
        let count = fleet.world.colliders.len();
        let query = V3::new(1., 0., 1.);
        assert_eq!(fleet.ground.road_surface(query, 0).0, Surface::Grass);
        fleet.set_static_region("hill", vec![], vec![ground(Surface::Gravel)]).unwrap();
        assert_eq!(fleet.ground.road_surface(query, 0).0, Surface::Gravel);
        assert_eq!(fleet.world.colliders.len(), count);
        assert_eq!(fleet.pose(id).unwrap().pos, before.pos);
        assert!(fleet.relocate(id, Vec3::new(1., 2., -15.), None));
        assert_eq!(fleet.pose(id).unwrap().speed, before.speed);
        fleet.set_static_region("hill", vec![], vec![]).unwrap();
        assert_eq!(fleet.ground.road_surface(query, 0).0, Surface::Asphalt);
        assert_eq!(fleet.world.colliders.len(), count - 1);
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
