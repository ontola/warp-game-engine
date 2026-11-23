//! Warp Humans going limp (`humans.ragdoll`): eleven capsules from the rig's
//! joints as they stood at the blow, thrown along it, with loose anatomical
//! joint limits (after Growing Guns' corpse rig). Steps with the simulation,
//! so replays match; a settled body freezes and leaves the physics world.
mod brace;

use body::Bone;
use glam::{Affine3A, Mat4, Quat, Vec3};
use kern::collide::YawBox;
use physics::ragdoll::{AngularProfile, Ragdoll, Segment};
use physics::{Body, Physics, Static, Support, vehicle::fleet::ChassisContact};
use std::collections::BTreeMap;

pub const PARTS: usize = 11;
/// At most this many move at once; the oldest freezes to make room.
const ACTIVE: usize = 24;
/// After this time, tolerate the last small movements of a supported resting rig.
const SETTLE: f32 = 4.0;
const HZ: f32 = 60.0;
const SHIN_RADIUS: f32 = 0.075;

/// The part each capture bone rides: hips, chest, head, the arms (upper,
/// lower with the hand), the legs (thigh, shin with the foot).
pub fn part(b: Bone) -> usize {
    match b {
        Bone::Pelvis => 0,
        Bone::Neck | Bone::Head => 2,
        Bone::UpperArmL => 3,
        Bone::LowerArmL | Bone::HandL => 4,
        Bone::UpperArmR => 5,
        Bone::LowerArmR | Bone::HandR => 6,
        Bone::ThighL => 7,
        Bone::CalfL | Bone::FootL | Bone::BallL => 8,
        Bone::ThighR => 9,
        Bone::CalfR | Bone::FootR | Bone::BallR => 10,
        _ => 1, // the spine and the clavicles go with the chest
    }
}

/// The bone whose turn gives each part's anatomical frame.
const FRAME: [Bone; PARTS] = [Bone::Pelvis, Bone::Spine3, Bone::Head, Bone::UpperArmL, Bone::LowerArmL, Bone::UpperArmR, Bone::LowerArmR, Bone::ThighL, Bone::CalfL, Bone::ThighR, Bone::CalfR];

// Model coordinates: +y up, +z ahead. Degrees from the bind skeleton (a
// T-pose); game tuning, not anatomy.
fn profile(part: usize) -> AngularProfile {
    let (min, max, neutral) = match part {
        1 => ([-25., -25., -20.], [35., 25., 20.], [5., 0., 0.]),
        2 => ([-40., -65., -35.], [50., 65., 35.], [8., 0., 0.]),
        3 => ([-80., -80., -100.], [90., 80., 50.], [10., 0., -65.]),
        5 => ([-80., -80., -50.], [90., 80., 100.], [10., 0., 65.]),
        4 => ([-12., -145., -12.], [12., 5., 12.], [0., -35., 0.]),
        6 => ([-12., -5., -12.], [12., 145., 12.], [0., 35., 0.]),
        7 | 9 => ([-110., -35., -45.], [30., 35., 45.], [-15., 0., 0.]),
        _ => ([-5., -10., -10.], [145., 10., 10.], [25., 0., 0.]),
    };
    let radians = |v: [f32; 3]| Vec3::from_array(v.map(f32::to_radians));
    AngularProfile { min: radians(min), max: radians(max), neutral: radians(neutral), soft_zone: 4.0_f32.to_radians(), stiffness: 0.35, limit_stiffness: 18.0, damping: 0.25, max_torque: 12.0 }
}

pub struct Doll {
    rig: Option<Ragdoll>,
    floor: Option<Static>,
    bit: u32,
    /// The model transform at the fall: palettes are kept in that space.
    fall: Mat4,
    fall_inverse: Mat4,
    /// Where each part is now relative to the fall, once it froze.
    frozen: Option<[Mat4; PARTS]>,
    /// Small reconstruction recipe shared by collision queries and waking.
    segments: Vec<Segment>,
    frames: [Quat; PARTS],
    age: f32,
    resting_on_obstacle: bool,
    /// The palette at the fall (model space).
    pub palette: Vec<Mat4>,
}

impl Doll {
    fn world_delta(&self, i: usize) -> Mat4 {
        match &self.rig {
            Some(rig) => rig.delta(i, 1.),
            None => self.frozen.as_ref().expect("inactive ragdoll has a frozen pose")[i],
        }
    }
}

/// A shared moving obstacle. Its bounds also reserve space for standing back up.
#[derive(Clone)]
pub struct Obstacle {
    pub shape: YawBox,
    pub previous: Vec3,
    pub previous_yaw: f32,
    pub chassis: Option<ChassisContact>,
}

impl Obstacle {
    fn same_geometry(&self, other: &Self) -> bool {
        match (&self.chassis, &other.chassis) {
            (Some(a), Some(b)) => a.same_hull(b),
            (None, None) => self.shape.half == other.shape.half,
            _ => false,
        }
    }

    fn add_kinematic(&self, physics: &mut Physics) -> Body {
        if let Some(chassis) = &self.chassis { return chassis.add_kinematic(physics); }
        physics.add_kinematic_box(&YawBox { center: self.previous, yaw: self.previous_yaw, half: self.shape.half })
    }

    fn target(&self) -> (Vec3, Quat) {
        self.chassis.as_ref().map_or_else(|| (self.shape.center, Quat::from_rotation_y(self.shape.yaw)), |chassis| chassis.current)
    }
}

pub const MAX_OBSTACLES: usize = 64;

#[derive(Default)]
pub struct Dolls {
    phys: Option<Physics>,
    acc: f32,
    pub dolls: BTreeMap<u32, Doll>,
    pub obstacles: BTreeMap<u32, Obstacle>,
    colliders: BTreeMap<u32, (Body, Obstacle)>,
}

impl Dolls {
    fn reserve_bit(&mut self) -> u32 {
        if self.active() >= ACTIVE {
            let oldest = self.dolls.iter().filter(|(_, d)| d.rig.is_some())
                .max_by(|(ia, a), (ib, b)| a.age.total_cmp(&b.age).then_with(|| ib.cmp(ia))).map(|(&id, _)| id);
            if let Some(id) = oldest { self.freeze(id); }
        }
        let used = self.dolls.values().filter(|d| d.rig.is_some()).fold(0u32, |m, d| m | d.bit);
        1u32 << used.trailing_ones()
    }

    /// Person `id` goes limp: `palette` (model space, rig joints) is how it
    /// stood, `model` where; `push` the blow (world, m/s).
    pub fn fall(&mut self, id: u32, joint: &[usize], bind: &[Vec3], palette: &[Mat4], model: Affine3A, push: Vec3, spin: Option<Vec3>) -> Result<(), String> {
        if !push.is_finite() || spin.is_some_and(|w| !w.is_finite()) { return Err("ragdoll motion must be finite".into()); }
        self.remove(id);
        let bit = self.reserve_bit();
        let phys = self.phys.get_or_insert_with(|| Physics::new(Vec3::new(0.0, -20.0, 0.0), HZ));
        let world = Mat4::from(model);
        let at = |b: Bone| (world * palette[joint[b as usize]]).transform_point3(bind[b as usize]);
        let (hips, chest, neck, head) = (at(Bone::Pelvis), at(Bone::Spine3), at(Bone::Neck), at(Bone::Head));
        let up = (neck - hips).normalize_or(Vec3::Y);
        let seg = |start, end, radius, parent, anchor| Segment { start, end, radius, parent, anchor };
        let mut segs = vec![
            seg(hips - up * 0.09, hips + up * 0.09, 0.14, None, hips),
            seg(hips, chest + up * 0.04, 0.16, Some(0), hips),
            seg(neck, head + up * 0.16, 0.105, Some(1), neck),
        ];
        for (a, b, c) in [(Bone::UpperArmL, Bone::LowerArmL, Bone::HandL), (Bone::UpperArmR, Bone::LowerArmR, Bone::HandR)] {
            let upper = segs.len();
            segs.push(seg(at(a), at(b), 0.065, Some(1), at(a)));
            segs.push(seg(at(b), at(c), 0.055, Some(upper), at(b)));
        }
        for (a, b, c) in [(Bone::ThighL, Bone::CalfL, Bone::FootL), (Bone::ThighR, Bone::CalfR, Bone::FootR)] {
            let upper = segs.len();
            segs.push(seg(at(a), at(b), 0.09, Some(0), at(a)));
            segs.push(seg(at(b), at(c), SHIN_RADIUS, Some(upper), at(b)));
        }
        let ground = model.translation.y;
        let floor = phys.add_floor_for(ground, bit);
        let (velocity, angular) = if let Some(w) = spin {
            // Passive handoff: subsequent acceleration comes from actual contacts.
            (push, w)
        } else {
            let flat = push.with_y(0.0);
            let force = flat.length();
            let dir = flat.normalize_or(Vec3::Z);
            (flat * 0.8 + Vec3::Y * (0.6 + force * 0.15 + push.y), up.cross(dir) * (1.2 + force * 0.45))
        };
        let mut rig = match Ragdoll::spawn(phys, &segs, velocity, angular, bit) {
            Ok(r) => r,
            Err(e) => { phys.remove_static(floor); return Err(e); }
        };
        let turn = model.to_scale_rotation_translation().1;
        let mut frames = [Quat::IDENTITY; PARTS];
        for (i, b) in FRAME.iter().enumerate() {
            let bone = (turn * Quat::from_mat4(&palette[joint[*b as usize]])).normalize();
            let body = phys.pose(rig.body(i)).ok_or("ragdoll: a new body is missing")?.1;
            frames[i] = (body.inverse() * bone).normalize();
        }
        for i in 1..PARTS {
            if let Err(e) = rig.set_angular_profile_with_brace(phys, i, frames[segs[i].parent.unwrap()], frames[i], profile(i), Some(brace::fall(i))) {
                rig.remove(phys);
                phys.remove_static(floor);
                return Err(e);
            }
        }
        rig.update(phys);
        rig.update(phys);
        self.dolls.insert(id, Doll { rig: Some(rig), floor: Some(floor), bit, fall: world, fall_inverse: world.inverse(), frozen: None, segments: segs, frames, age: 0.0, resting_on_obstacle: false, palette: palette.to_vec() });
        Ok(())
    }

    pub fn hips(&self, id: u32) -> Option<Vec3> {
        let d = self.dolls.get(&id)?;
        let hips = &d.segments[0];
        Some(d.world_delta(0).transform_point3((hips.start + hips.end) * 0.5))
    }

    pub fn raycast(&self, id: u32, origin: Vec3, dir: Vec3, limit: f32) -> Option<f32> {
        let d = self.dolls.get(&id)?;
        let dir = dir.try_normalize()?;
        d.segments.iter().enumerate().filter_map(|(i, s)| s.raycast(d.world_delta(i), origin, dir, limit)).min_by(f32::total_cmp)
    }

    pub fn closest(&self, id: u32, point: Vec3) -> Option<(usize, Vec3)> {
        let d = self.dolls.get(&id)?;
        d.segments.iter().enumerate().map(|(i, s)| (i, s.closest(d.world_delta(i), point)))
            .min_by(|a, b| a.1.distance_squared(point).total_cmp(&b.1.distance_squared(point)))
    }

    /// Recreate the same joint anchors and passive limits, then put the limbs
    /// back in the stored world pose before the first solver or render sample.
    fn wake(&mut self, id: u32) -> Result<(), String> {
        let bit = self.reserve_bit();
        let phys = self.phys.as_mut().expect("a frozen ragdoll has a world");
        let d = self.dolls.get_mut(&id).expect("ragdoll to wake");
        let mut rig = Ragdoll::spawn(phys, &d.segments, Vec3::ZERO, Vec3::ZERO, bit)?;
        let result = (|| {
            for i in 1..PARTS {
                let parent = d.segments[i].parent.unwrap();
                rig.set_angular_profile(phys, i, d.frames[parent], d.frames[i], profile(i))?;
            }
            rig.restore_deltas(phys, d.frozen.as_ref().expect("frozen pose"))
        })();
        if let Err(e) = result { rig.remove(phys); return Err(e); }
        d.floor = Some(phys.add_floor_for(d.fall.w_axis.y, bit));
        d.bit = bit;
        d.frozen = None;
        d.rig = Some(rig);
        d.age = 0.;
        d.resting_on_obstacle = false;
        Ok(())
    }

    pub fn hit(&mut self, id: u32, point: Vec3, velocity: Vec3) -> Result<bool, String> {
        if !point.is_finite() || !velocity.length_squared().is_finite() { return Err("invalid ragdoll hit".into()); }
        let Some((index, contact)) = self.closest(id, point) else { return Ok(false); };
        if velocity == Vec3::ZERO { return Ok(true); }
        if self.dolls[&id].rig.is_none() { self.wake(id)?; }
        let rig = self.dolls[&id].rig.as_ref().unwrap();
        rig.impulse_at(self.phys.as_mut().unwrap(), index, contact, velocity);
        Ok(true)
    }

    /// The body stays where it lies and leaves the physics world.
    fn freeze(&mut self, id: u32) {
        let Some(d) = self.dolls.get_mut(&id) else { return };
        let Some(mut rig) = d.rig.take() else { return };
        d.frozen = Some(std::array::from_fn(|i| rig.delta(i, 1.0)));
        if let Some(phys) = self.phys.as_mut() {
            rig.remove(phys);
            if let Some(f) = d.floor.take() { phys.remove_static(f); }
        }
    }

    pub fn remove(&mut self, id: u32) {
        self.freeze(id);
        self.dolls.remove(&id);
    }

    pub fn clear(&mut self) {
        self.dolls.clear();
        self.obstacles.clear();
        self.colliders.clear();
        self.phys = None;
        self.acc = 0.0;
    }

    /// Keep a chassis in the physics world only near a moving ragdoll. Check
    /// the swept segment so even the first contact arrives with car velocity.
    fn sync_obstacles(&mut self) {
        let Some(phys) = &mut self.phys else { return };
        let roots: Vec<_> = self.dolls.values().filter_map(|d| Some(d.rig.as_ref()?.root_position(1.0))).collect();
        let nearby = |b: &Obstacle| {
            let delta = b.shape.center - b.previous;
            let radius = 8.0 + b.shape.half.length();
            roots.iter().any(|p| {
                let t = ((*p - b.previous).dot(delta) / delta.length_squared().max(1e-8)).clamp(0., 1.);
                p.distance_squared(b.previous + delta * t) <= radius * radius
            })
        };
        self.colliders.retain(|id, (body, previous)| {
            let keep = self.obstacles.get(id).is_some_and(|b| previous.same_geometry(b) && nearby(b));
            if !keep { phys.remove(*body); }
            keep
        });
        for (&id, b) in &self.obstacles {
            if self.colliders.contains_key(&id) || !nearby(b) { continue; }
            self.colliders.insert(id, (b.add_kinematic(phys), b.clone()));
        }
    }

    #[cfg(test)]
    pub(crate) fn obstacle_count(&self) -> usize { self.colliders.len() }

    pub fn settled(&self, id: u32) -> bool {
        self.dolls.get(&id).is_some_and(|d| d.frozen.is_some() || d.resting_on_obstacle)
    }

    /// A person getting up next to a car needs room for their standing body.
    /// Project sideways onto the nearest clear side; try again next tick if
    /// several tightly parked obstacles leave no clear spot.
    pub fn standing_space(&self, mut feet: Vec3) -> Option<Vec3> {
        let overlaps = |b: &YawBox, p: Vec3| {
            if b.top() <= p.y || b.center.y - b.half.y >= p.y + 1.8 { return None; }
            b.push_out(Vec3::new(p.x, b.center.y, p.z), 0.34, b.half.max_element() + 1.)
        };
        for _ in 0..4 {
            let Some(out) = self.obstacles.values().find_map(|b| overlaps(&b.shape, feet)) else { return Some(feet) };
            feet += out + out.normalize_or_zero() * 0.005;
        }
        self.obstacles.values().all(|b| overlaps(&b.shape, feet).is_none()).then_some(feet)
    }

    /// A simulation tick: fixed physics steps; settled bodies freeze.
    pub fn tick(&mut self, dt: f32) {
        if self.phys.is_none() { return; }
        self.sync_obstacles();
        let phys = self.phys.as_mut().unwrap();
        if self.dolls.values().all(|d| d.rig.is_none()) { self.acc = 0.0; return; }
        self.acc += dt;
        let steps = ((self.acc + 1e-6) * HZ).floor().clamp(0., 4.) as usize;
        for remaining in (1..=steps).rev() {
            // At city-road speeds a chassis can travel several shin radii in
            // one simulation tick. Advance the contact skin and the limbs together
            // so the solver sees the rising bonnet before the windscreen.
            let mut travel = 0.0_f32;
            for (id, (body, _)) in &self.colliders {
                let obstacle = &self.obstacles[id];
                let (target, turn) = obstacle.target();
                if let Some((pos, rot)) = phys.pose(*body) {
                    let arc = rot.angle_between(turn) * obstacle.shape.half.length();
                    travel = travel.max((pos.distance(target) + arc) / remaining as f32);
                }
            }
            let substeps = (travel / SHIN_RADIUS).ceil().clamp(1., 4.) as usize;
            let fixed_dt = phys.world.integration_parameters.dt;
            phys.world.integration_parameters.dt = fixed_dt / substeps as f32;
            for substep in (1..=substeps).rev() {
                let fraction = 1. / ((remaining - 1) * substeps + substep) as f32;
                for (id, (body, _)) in &self.colliders {
                    let (target, rotation) = self.obstacles[id].target();
                    if let Some((pos, rot)) = phys.pose(*body) {
                        phys.move_kinematic(*body, pos.lerp(target, fraction), rot.slerp(rotation, fraction));
                    }
                }
                phys.step();
            }
            phys.world.integration_parameters.dt = fixed_dt;
            for d in self.dolls.values_mut() {
                if let Some(rig) = d.rig.as_mut() { rig.update(phys); }
            }
            self.acc -= 1.0 / HZ;
        }
        self.acc = self.acc.clamp(0.0, 1.0 / HZ);
        let mut done = Vec::new();
        for (id, d) in self.dolls.iter_mut() {
            d.age += dt;
            let Some(rig) = &d.rig else { continue };
            d.resting_on_obstacle = false;
            // A limb can retain small axial twists against its joint spring on
            // the ground. Keep tracking travel for every limb and rotation for
            // the torso and pelvis after the settling grace period.
            if d.age < 0.8 || !(0..rig.len()).all(|i| {
                let (linear, angular) = if d.age <= SETTLE { (0.05, 0.25) }
                    else if i < 2 { (0.18, 0.6) } else { (0.18, f32::INFINITY) };
                phys.motion_below(rig.body(i), linear, angular)
            }) { continue; }
            match (0..rig.len()).filter_map(|i| phys.support(rig.body(i))).max() {
                Some(Support::Ground) => done.push(*id),
                // A parked car may start again before the game requests recovery.
                Some(Support::Parked) => d.resting_on_obstacle = true,
                _ => {}
            }
        }
        for id in done { self.freeze(id); }
    }

    pub fn has(&self, id: u32) -> bool { self.dolls.contains_key(&id) }

    /// Moving bodies (not yet frozen).
    pub fn active(&self) -> usize { self.dolls.values().filter(|d| d.rig.is_some()).count() }

    /// Each part's move since the fall in the fall's model space, `alpha` of
    /// the way through the last step.
    pub fn deltas(&self, id: u32, alpha: f32) -> Option<[Mat4; PARTS]> {
        let d = self.dolls.get(&id)?;
        let world: [Mat4; PARTS] = match (&d.rig, &d.frozen) {
            (Some(rig), _) => std::array::from_fn(|i| rig.delta(i, alpha)),
            (None, Some(f)) => *f,
            _ => return None,
        };
        Some(world.map(|w| d.fall_inverse * w * d.fall))
    }
}
