//! Warp Humans going limp (`humans.ragdoll`): eleven capsules from the rig's
//! joints as they stood at the blow, thrown along it, with loose anatomical
//! joint limits (after Growing Guns' corpse rig). Steps with the simulation,
//! so replays match; a settled body freezes and leaves the physics world.
use body::Bone;
use glam::{Affine3A, Mat4, Quat, Vec3};
use kern::collide::YawBox;
use physics::ragdoll::{AngularProfile, Ragdoll, Segment};
use physics::{Body, Physics, Static};
use std::collections::BTreeMap;

pub const PARTS: usize = 11;
/// At most this many move at once; the oldest freezes to make room.
const ACTIVE: usize = 24;
/// Seconds before a body freezes where it lies, settled or not.
const SETTLE: f32 = 4.0;
const HZ: f32 = 60.0;

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
    age: f32,
    /// The palette at the fall (model space).
    pub palette: Vec<Mat4>,
}

/// A solid moving box shared by every struck person, keyed by game identity.
#[derive(Clone, Copy)]
pub struct Obstacle {
    pub shape: YawBox,
    pub previous: Vec3,
    pub previous_yaw: f32,
}

pub const MAX_OBSTACLES: usize = 64;

#[derive(Default)]
pub struct Dolls {
    phys: Option<Physics>,
    acc: f32,
    pub dolls: BTreeMap<u32, Doll>,
    pub obstacles: BTreeMap<u32, Obstacle>,
    colliders: BTreeMap<u32, (Body, Vec3)>,
}

impl Dolls {
    /// Person `id` goes limp: `palette` (model space, rig joints) is how it
    /// stood, `model` where; `push` the blow (world, m/s).
    pub fn fall(&mut self, id: u32, joint: &[usize], bind: &[Vec3], palette: &[Mat4], model: Affine3A, push: Vec3) -> Result<(), String> {
        if !push.is_finite() { return Err("ragdoll push must be finite".into()); }
        self.remove(id);
        let active: Vec<u32> = self.dolls.iter().filter(|(_, d)| d.rig.is_some()).map(|(k, _)| *k).collect();
        if active.len() >= ACTIVE {
            if let Some(old) = active.iter().min_by(|a, b| self.dolls[*b].age.total_cmp(&self.dolls[*a].age)) {
                let old = *old;
                self.freeze(old);
            }
        }
        let used: u32 = self.dolls.values().filter(|d| d.rig.is_some()).fold(0, |m, d| m | d.bit);
        let bit = 1u32 << (0..32).find(|i| used & (1 << i) == 0).unwrap_or(0);
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
            segs.push(seg(at(b), at(c), 0.075, Some(upper), at(b)));
        }
        let ground = model.translation.y;
        let floor = phys.add_floor_for(ground, bit);
        let flat = push.with_y(0.0);
        let force = flat.length();
        let dir = flat.normalize_or(Vec3::Z);
        // Struck high: the chest goes first, the feet trail.
        let mut rig = match Ragdoll::spawn(phys, &segs, flat * 0.8 + Vec3::Y * (0.6 + force * 0.15) + Vec3::Y * push.y, up.cross(dir) * (1.2 + force * 0.45), bit) {
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
            if let Err(e) = rig.set_angular_profile(phys, i, frames[segs[i].parent.unwrap()], frames[i], profile(i)) {
                rig.remove(phys);
                phys.remove_static(floor);
                return Err(e);
            }
        }
        rig.update(phys);
        rig.update(phys);
        self.dolls.insert(id, Doll { rig: Some(rig), floor: Some(floor), bit, fall: world, fall_inverse: world.inverse(), frozen: None, age: 0.0, palette: palette.to_vec() });
        Ok(())
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
        self.colliders.retain(|id, (body, half)| {
            let keep = self.obstacles.get(id).is_some_and(|b| b.shape.half == *half && nearby(b));
            if !keep { phys.remove(*body); }
            keep
        });
        for (&id, b) in &self.obstacles {
            if self.colliders.contains_key(&id) || !nearby(b) { continue; }
            let previous = YawBox { center: b.previous, yaw: b.previous_yaw, half: b.shape.half };
            self.colliders.insert(id, (phys.add_kinematic_box(&previous), b.shape.half));
        }
    }

    #[cfg(test)]
    pub(crate) fn obstacle_count(&self) -> usize { self.colliders.len() }

    pub fn settled(&self, id: u32) -> bool {
        self.dolls.get(&id).is_some_and(|d| d.frozen.is_some())
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
            let fraction = 1.0 / remaining as f32;
            for (id, (body, _)) in &self.colliders {
                let target = self.obstacles[id].shape;
                if let Some((pos, rot)) = phys.pose(*body) {
                    phys.move_kinematic(*body, pos.lerp(target.center, fraction),
                        rot.slerp(Quat::from_rotation_y(target.yaw), fraction));
                }
            }
            phys.step();
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
            let still = d.age > 0.8 && (0..rig.len()).all(|i| phys.sleeping(rig.body(i)) || phys.velocity(rig.body(i)).is_some_and(|v| v.length() < 0.05));
            if still || d.age > SETTLE { done.push(*id); }
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
