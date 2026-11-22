//! Warp Humans for Luau games (`humans.spawn`, `humans.set`, `humans.remove`).
//!
//! game.toml reserves pools (`[humans.zombie] count = 40`). Each pool fits a
//! few seeded people (`variety`) to the shared animation rig once, then adds
//! `count` skinned actors to the scene before the renderer is built, so
//! spawning never rebuilds it: an unused actor waits far below the world.
//!
//! The game moves a human by setting its feet position and facing each tick;
//! the legs follow from how far it moved (the `body` crate's walk, jog and
//! idle captures, the cycle fitted to the rig's stride). A `pose` lays an
//! action over that: attacks, hits, falls, arms reaching or raised. The
//! "zombie" style stoops, drags its steps and holds its arms out.
//!
//! Simulation stays GPU-free: without a renderer nothing is fitted and only
//! the positions are kept (`warp check`, tests).
use assets::skinned::{ActorHandle, Character};
use body::{Bone, Bound, Layer, Library, BONES};
use glam::{Affine3A, Mat4, Quat, Vec2, Vec3};
use render::Scene;
use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

/// The most different people a pool fits in a browser.
const BROWSER_VARIETY: usize = 2;

use crate::config::HumanPool;

const RIG_JSON: &[u8] = include_bytes!("../../../assets/humans/rig/player-rig.json");
const RIG_ANIM: &[u8] = include_bytes!("../../../assets/humans/rig/player.anim.bin");

/// Where unused actors wait.
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);

/// The capture skeleton's bones and the rig's joint names.
const JOINTS: [(Bone, &str); BONES] = [
    (Bone::Pelvis, "Hips"),
    (Bone::Spine1, "Spine"),
    (Bone::Spine2, "Spine1"),
    (Bone::Spine3, "Spine2"),
    (Bone::Neck, "Neck"),
    (Bone::Head, "Head"),
    (Bone::ClavicleL, "LeftShoulder"),
    (Bone::UpperArmL, "LeftArm"),
    (Bone::LowerArmL, "LeftForeArm"),
    (Bone::HandL, "LeftHand"),
    (Bone::ClavicleR, "RightShoulder"),
    (Bone::UpperArmR, "RightArm"),
    (Bone::LowerArmR, "RightForeArm"),
    (Bone::HandR, "RightHand"),
    (Bone::ThighL, "LeftUpLeg"),
    (Bone::CalfL, "LeftLeg"),
    (Bone::FootL, "LeftFoot"),
    (Bone::BallL, "LeftToeBase"),
    (Bone::ThighR, "RightUpLeg"),
    (Bone::CalfR, "RightLeg"),
    (Bone::FootR, "RightFoot"),
    (Bone::BallR, "RightToeBase"),
];

const UAL: &str = include_str!("../../body/assets/ual.json");
/// The library's bone names, in `Bone` order.
const UAL_NAMES: [&str; BONES] = ["pelvis", "spine_01", "spine_02", "spine_03", "neck_01", "Head", "clavicle_l", "upperarm_l", "lowerarm_l", "hand_l", "clavicle_r", "upperarm_r", "lowerarm_r", "hand_r", "thigh_l", "calf_l", "foot_l", "ball_l", "thigh_r", "calf_r", "foot_r", "ball_r"];

/// The next bone down a chain (None at a hand, foot tip or the head).
fn child(b: Bone) -> Option<Bone> {
    use Bone::*;
    Some(match b {
        Pelvis => Spine1, Spine1 => Spine2, Spine2 => Spine3, Spine3 => Neck, Neck => Head,
        ClavicleL => UpperArmL, UpperArmL => LowerArmL, LowerArmL => HandL,
        ClavicleR => UpperArmR, UpperArmR => LowerArmR, LowerArmR => HandR,
        ThighL => CalfL, CalfL => FootL, FootL => BallL,
        ThighR => CalfR, CalfR => FootR, FootR => BallR,
        Head | HandL | HandR | BallL | BallR => return None,
    })
}

/// `#rrggbb` to sRGB 0..1.
pub fn parse_rgb(text: &str) -> Option<[f32; 3]> {
    let h = text.strip_prefix('#')?;
    if h.len() != 6 { return None; }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0])
}

/// One walking loop, aligned so phase 0 is the left foot's midstance.
struct Cycle { clip: usize, length: f32, contact: f32, metres: f32 }

impl Cycle {
    fn fit(lib: &Library, rig: &Bound, clip: usize) -> Result<Self, String> {
        let length = lib.get(clip).length;
        let toe = Bone::BallL as usize;
        let pelvis = Bone::Pelvis as usize;
        let foot = |phase: f32| {
            let pose = rig.pose(lib, &[Layer { clip, t: phase * length, looped: true, w: 1.0 }]);
            pose[toe].transform_point3(rig.rest[toe].unwrap()).z - pose[pelvis].transform_point3(rig.rest[pelvis].unwrap()).z
        };
        let mut prev = foot(0.0);
        for i in 1..=96 {
            let next = foot(i as f32 / 96.0);
            if prev >= 0.0 && next < 0.0 {
                let contact = ((i - 1) as f32 + prev / (prev - next)) / 96.0;
                let width = 1.0 / 48.0;
                let metres = (foot(contact - width) - foot(contact + width)) / (2.0 * width);
                if metres > 0.1 { return Ok(Self { clip, length, contact, metres }); }
            }
            prev = next;
        }
        Err("no support stroke in a walking capture".into())
    }
    fn layer(&self, phase: f32, w: f32) -> Layer { Layer { clip: self.clip, t: (self.contact + phase) * self.length, looped: true, w } }
}

fn ramp(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The skeleton and captures every human shares.
pub struct Rig {
    pub character: Character,
    lib: Library,
    /// Rig joint index of each capture bone.
    joint: [usize; BONES],
    /// The rig's own skeleton (a broad-shouldered T-pose): the legs every
    /// person shares, and the cycles fitted to them.
    skeleton: Skeleton,
    /// The captures' rest pose, from the library's own skeleton.
    capture: [Vec3; BONES],
    walk: Cycle,
    jog: Cycle,
    /// Flat out: above jogging speed the legs and arms pump.
    sprint: Cycle,
    idle: usize,
    /// Crouched: walking low, and waiting low (kneeling by someone).
    crouch: Cycle,
    crouch_idle: usize,
    clips: BTreeMap<&'static str, usize>,
}

impl Rig {
    pub fn shared() -> Result<&'static Rig, String> {
        static RIG: OnceLock<Result<Rig, String>> = OnceLock::new();
        RIG.get_or_init(Rig::load).as_ref().map_err(|e| e.clone())
    }

    fn load() -> Result<Rig, String> {
        let character = Character::animation_rig_from_bytes(RIG_JSON, RIG_ANIM)?;
        let lib = Library::builtin()?;
        let mut joint = [0; BONES];
        let mut bind = [Vec3::ZERO; BONES];
        for (bone, name) in JOINTS {
            let (i, (_, m)) = character.joints().enumerate().find(|(_, (n, _))| *n == name).ok_or_else(|| format!("humans: the rig has no joint {name}"))?;
            joint[bone as usize] = i;
            bind[bone as usize] = m.w_axis.truncate();
        }
        // The captures' rest pose, from the library's own skeleton.
        let ual: serde_json::Value = serde_json::from_str(UAL).map_err(|e| format!("humans: ual.json: {e}"))?;
        let rest_of = |b: Bone| -> Result<Vec3, String> {
            let name = UAL_NAMES[b as usize];
            let a = ual["joints"][name].as_array().ok_or_else(|| format!("humans: ual.json has no joint {name}"))?;
            Ok(Vec3::new(a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32))
        };
        let mut capture = [Vec3::ZERO; BONES];
        for (bone, _) in JOINTS { capture[bone as usize] = rest_of(bone)?; }
        let skeleton = Skeleton::new(bind, &capture);
        let bound = &skeleton.bound;
        let find = |n: &str| lib.clip(n).ok_or_else(|| format!("humans: no capture {n}"));
        let walk = Cycle::fit(&lib, bound, find("Walk_Loop")?)?;
        let jog = Cycle::fit(&lib, bound, find("Jog_Fwd_Loop")?)?;
        let sprint = Cycle::fit(&lib, bound, find("Sprint_Loop")?)?;
        let idle = find("Idle_Loop")?;
        let crouch = Cycle::fit(&lib, bound, find("Crouch_Fwd_Loop")?)?;
        let crouch_idle = find("Crouch_Idle_Loop")?;
        let mut clips = BTreeMap::new();
        for (pose, clip) in [("attack", "Sword_Attack"), ("punch", "Punch_Cross"), ("jab", "Punch_Jab"), ("hit", "Hit_Chest"), ("hit_head", "Hit_Head"), ("dead", "Death01"), ("down", "Death01"), ("cast", "Spell_Simple_Shoot"), ("roll", "Roll")] {
            clips.insert(pose, find(clip)?);
        }
        Ok(Rig { character, lib, joint, skeleton, capture, walk, jog, sprint, idle, crouch, crouch_idle, clips })
    }

    /// A person's skeleton: the rig's with the spine, head and arms where
    /// that person's body has them (`Character::warp_human_joints`).
    fn skeleton_of(&self, person: &render::humans::Instance) -> Result<Skeleton, String> {
        let joints = self.character.warp_human_joints(person)?;
        Ok(Skeleton::new(std::array::from_fn(|b| joints[self.joint[b]].1.w_axis.truncate()), &self.capture))
    }
}

/// One body's joints under the shared captures.
struct Skeleton {
    /// Each bone's bind position (what the palette moves), and the move from
    /// the bind (arms out or half down) to the captures' rest (arms down)
    /// that the poses start from.
    bind: [Vec3; BONES],
    to_rest: [Mat4; BONES],
    bound: Bound,
}

impl Skeleton {
    fn new(bind: [Vec3; BONES], capture: &[Vec3; BONES]) -> Self {
        // Turn each arm and leg segment onto the capture's direction, keeping
        // its length. The trunk, neck and collarbones stand as the body has
        // them: both rests are upright, and where one skeleton puts a joint
        // along the spine or how its collarbone points says nothing about
        // the pose (turning them hunched the back and raised the shoulders).
        let limb = |b: Bone| !matches!(b, Bone::Pelvis | Bone::Spine1 | Bone::Spine2 | Bone::Spine3 | Bone::Neck | Bone::Head | Bone::ClavicleL | Bone::ClavicleR);
        let mut turn = [Quat::IDENTITY; BONES];
        for (bone, _) in JOINTS {
            let b = bone as usize;
            turn[b] = match child(bone) {
                _ if !limb(bone) => Quat::IDENTITY,
                Some(c) => {
                    let rig = (bind[c as usize] - bind[b]).normalize_or_zero();
                    let cap = (capture[c as usize] - capture[b]).normalize_or_zero();
                    if rig == Vec3::ZERO || cap == Vec3::ZERO { Quat::IDENTITY } else { Quat::from_rotation_arc(rig, cap) }
                }
                _ => bone.parent().map_or(Quat::IDENTITY, |p| turn[p as usize]),
            };
        }
        let mut rest = [Vec3::ZERO; BONES];
        for (bone, _) in JOINTS {
            let b = bone as usize;
            rest[b] = match bone.parent() {
                Some(p) => rest[p as usize] + turn[p as usize] * (bind[b] - bind[p as usize]),
                None => bind[b],
            };
        }
        let to_rest: [Mat4; BONES] = std::array::from_fn(|b| Mat4::from_translation(rest[b]) * Mat4::from_quat(turn[b]) * Mat4::from_translation(-bind[b]));
        let spots: Vec<(Bone, [f32; 3])> = JOINTS.iter().map(|(bone, _)| (*bone, rest[*bone as usize].to_array())).collect();
        Skeleton { bind, to_rest, bound: Bound::from_joints(&spots) }
    }

    fn rest(&self, b: Bone) -> Vec3 { self.bind[b as usize] }
}

/// The joints a game can ask where they are (`humans.joint`).
pub const JOINT_NAMES: &[(&str, Bone)] = &[("head", Bone::Head), ("neck", Bone::Neck), ("chest", Bone::Spine3), ("hips", Bone::Pelvis),
    ("shoulder_l", Bone::UpperArmL), ("shoulder_r", Bone::UpperArmR), ("elbow_l", Bone::LowerArmL), ("elbow_r", Bone::LowerArmR),
    ("hand_l", Bone::HandL), ("hand_r", Bone::HandR), ("knee_l", Bone::CalfL), ("knee_r", Bone::CalfR), ("foot_l", Bone::FootL), ("foot_r", Bone::FootR)];

/// The poses a game can ask for (`humans.set(id, {pose = ...})`).
pub const POSES: &[&str] = &["walk", "idle", "crouch", "feed", "rise", "wave", "reach", "raise", "strike", "aim", "shove", "attack", "punch", "jab", "hit", "hit_head", "dead", "down", "cast", "roll"];

/// A person's gait between two ticks.
#[derive(Clone, Copy, Default, Debug)]
struct Gait { phase: f32, speed: f32 }

#[derive(Clone, Debug)]
pub struct Person {
    pub pool: String,
    slot: usize,
    pub pos: Vec3,
    pub yaw: f32,
    visible: bool,
    prev_pos: Vec3,
    prev_yaw: f32,
    /// The legs' turn from the facing towards where they walk (radians):
    /// strafing walks the legs sideways under a body that keeps aiming.
    legs: f32,
    prev_legs: f32,
    from: Gait,
    to: Gait,
    pub pose: String,
    /// Progress through the pose, 0..1 (an attack's swing, a fall).
    pub t: f32,
    prev_t: f32,
    /// Out of breath, 0..1, over any pose: bent over, the chest heaving,
    /// the arms hanging (`humans.set(id, {tired = ...})`).
    pub tired: f32,
    prev_tired: f32,
    /// Wound up for a heavy blow, -1..1: the chest turns the right shoulder
    /// back and leans away (1), or follows through past the front (< 0)
    /// (`humans.set(id, {coil = ...})`).
    pub coil: f32,
    prev_coil: f32,
    /// A blow's jolt on the upper body (pitch, roll in radians) and its
    /// spring's speed: `humans.hit` kicks it, it swings back by itself.
    jolt: Vec2,
    prev_jolt: Vec2,
    jolt_v: Vec2,
    /// Seconds alive (idle sway, the zombie lurch).
    age: f32,
    /// Moved since the last tick.
    fresh: bool,
}

impl Person {
    /// A hidden actor keeps its model and slot, e.g. while riding in a vehicle.
    pub fn set_visible(&mut self, visible: bool) {
        self.fresh |= visible && !self.visible;
        self.visible = visible;
    }
}

struct Pool {
    cfg: HumanPool,
    zombie: bool,
    /// Fitted people (empty without a renderer).
    models: Vec<Character>,
    /// Each of the pool's people's skeleton, made when first asked (the
    /// simulation poses the same bodies with or without a renderer).
    skeletons: OnceLock<Vec<Skeleton>>,
    /// One actor per slot, in the scene's skinning order.
    actors: Vec<ActorHandle>,
    used: Vec<bool>,
    /// Slots that have an actor: all of them, unless the device could not
    /// hold that many (then the first `cap`).
    cap: usize,
}

#[derive(Default)]
pub struct Humans {
    pools: BTreeMap<String, Pool>,
    pub people: BTreeMap<u32, Person>,
    next: u32,
    /// Actors are in the scene (fitted and added once a renderer is wanted).
    attached: bool,
    palettes: Vec<Vec<Mat4>>,
    /// The first actor's skinning index (0 unless the scene had others).
    first_actor: usize,
    error: Option<String>,
    /// The limp (`humans.ragdoll`), by person.
    dolls: crate::ragdoll::Dolls,
}

/// The facing of a human with `yaw`: 0 faces +z, a quarter turn faces +x.
fn model_transform(pos: Vec3, yaw: f32) -> Affine3A { Affine3A::from_rotation_translation(Quat::from_rotation_y(yaw), pos) }

fn hash(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9e3779b97f4a7c15) ^ b.wrapping_add(0x632be59bd9b4e019);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^ (x >> 29)
}

impl Pool {
    /// The pool's `k`th person, from its seed and name.
    fn person(&self, name: &str, k: usize) -> render::humans::Instance {
        let seed = hash(hash(self.cfg.seed, name.bytes().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))), k as u64);
        let [lo, hi] = self.cfg.height;
        let height = lo + (hi - lo) * ((seed >> 11) % 1000) as f32 / 1000.0;
        let shirt = self.cfg.shirt.as_deref().and_then(parse_rgb);
        render::humans::Instance::person(seed, Vec3::ZERO, 0.0, &render::humans::Constraints { height_m: Some(height), shirt })
    }

    /// The skeleton of the person in `slot`.
    fn skeleton<'a>(&'a self, rig: &'a Rig, name: &str, slot: usize) -> &'a Skeleton {
        let all = self.skeletons.get_or_init(|| (0..self.cfg.variety.max(1)).filter_map(|k| rig.skeleton_of(&self.person(name, k)).ok()).collect());
        all.get(slot % self.cfg.variety.max(1)).unwrap_or(&rig.skeleton)
    }
}

impl Humans {
    /// The skeleton `p` is posed on.
    fn skeleton<'a>(&'a self, rig: &'a Rig, p: &Person) -> &'a Skeleton {
        self.pools.get(&p.pool).map_or(&rig.skeleton, |pool| pool.skeleton(rig, &p.pool, p.slot))
    }

    pub fn new(cfg: &BTreeMap<String, HumanPool>) -> Self {
        // A browser fits each person on its one page thread before the
        // first frame (25 people took 150 s in Dead Miles): fewer there.
        let fitted = |c: &HumanPool| HumanPool { variety: if cfg!(target_arch = "wasm32") { c.variety.min(BROWSER_VARIETY) } else { c.variety }, ..c.clone() };
        let pools = cfg.iter().map(|(name, c)| (name.clone(), Pool { zombie: c.style == "zombie", cfg: fitted(c), models: Vec::new(), skeletons: OnceLock::new(), actors: Vec::new(), used: vec![false; c.count], cap: c.count })).collect();
        Humans { pools, next: 1, ..Default::default() }
    }

    pub fn is_empty(&self) -> bool { self.pools.is_empty() }

    pub fn spawn(&mut self, pool: &str, pos: Vec3, yaw: f32) -> Result<Option<u32>, String> {
        if !self.pools.contains_key(pool) {
            let names: Vec<_> = self.pools.keys().cloned().collect();
            return Err(format!("humans.spawn: no pool {pool:?} (game.toml has {})", if names.is_empty() { "none: add [humans.NAME]".into() } else { names.join(", ") }));
        }
        let p = self.pools.get_mut(pool).unwrap();
        let Some(slot) = p.used[..p.cap].iter().position(|u| !u) else { return Ok(None) };
        p.used[slot] = true;
        let id = self.next;
        self.next += 1;
        self.people.insert(id, Person { pool: pool.to_string(), slot, pos, yaw, visible: true, prev_pos: pos, prev_yaw: yaw, legs: 0.0, prev_legs: 0.0, from: Gait::default(), to: Gait::default(), pose: "walk".into(), t: 0.0, prev_t: 0.0, tired: 0.0, prev_tired: 0.0, coil: 0.0, prev_coil: 0.0, jolt: Vec2::ZERO, prev_jolt: Vec2::ZERO, jolt_v: Vec2::ZERO, age: (hash(id as u64, 7) % 1000) as f32 * 0.01, fresh: true });
        Ok(Some(id))
    }

    pub fn remove(&mut self, id: u32) -> bool {
        self.dolls.remove(id);
        let Some(p) = self.people.remove(&id) else { return false };
        if let Some(pool) = self.pools.get_mut(&p.pool) { pool.used[p.slot] = false; }
        true
    }

    pub fn clear(&mut self) {
        for p in self.pools.values_mut() { p.used.fill(false); }
        self.people.clear();
        self.dolls.clear();
    }

    /// Free and total slots of a pool.
    pub fn room(&self, pool: &str) -> Option<(usize, usize)> {
        self.pools.get(pool).map(|p| (p.used[..p.cap].iter().filter(|u| !**u).count(), p.cap))
    }

    /// Save the previous tick before game scripts change positions and poses.
    pub fn tick_start(&mut self) {
        for p in self.people.values_mut() {
            p.prev_pos = p.pos;
            p.prev_yaw = p.yaw;
            p.prev_t = p.t;
            p.prev_tired = p.tired;
            p.prev_coil = p.coil;
        }
    }

    /// A simulation tick has run: the legs advance by how far each person moved.
    pub fn tick(&mut self, dt: f32) {
        let Ok(rig) = Rig::shared() else { return };
        for p in self.people.values_mut() {
            let zombie = self.pools.get(&p.pool).is_some_and(|pool| pool.zombie);
            let moved = (p.pos - p.prev_pos).with_y(0.0);
            let teleport = p.fresh || p.pos.distance_squared(p.prev_pos) > 9.0;
            if teleport {
                p.prev_pos = p.pos;
                p.prev_yaw = p.yaw;
            }
            p.from = p.to;
            p.from.phase = p.from.phase.rem_euclid(1.0);
            p.to.phase = p.from.phase;
            let distance = if teleport { 0.0 } else { moved.length() };
            let speed = distance / dt.max(1e-4);
            p.to.speed += (speed - p.to.speed) * (1.0 - (-dt / 0.08).exp());
            // Where the legs go, against where the body faces: sideways turns
            // the hips and legs that way (up to 75 degrees), further round
            // walks backwards with the legs turned the other way.
            let ahead = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
            let right = Vec3::new(p.yaw.cos(), 0.0, -p.yaw.sin());
            let a = if distance > 1e-4 && speed > 0.3 { moved.dot(right).atan2(moved.dot(ahead)) } else { 0.0 };
            let back = a.abs() > 1.75;
            let want = if back { a - PI * a.signum() } else { a }.clamp(-1.3, 1.3);
            p.prev_legs = p.legs;
            p.legs += (want - p.legs) * (1.0 - (-dt / 0.12).exp());
            if distance > 1e-5 {
                // Walking backwards plays the cycle backwards.
                let sign = if back { -1.0 } else { 1.0 };
                let [w, j, r] = blend(p.to.speed);
                let metres = if matches!(p.pose.as_str(), "crouch" | "feed") { rig.crouch.metres } else { rig.walk.metres * w + rig.jog.metres * j + rig.sprint.metres * r };
                // A zombie drags shorter steps at the same pace.
                let stride = if zombie { 0.7 } else { 1.0 };
                p.to.phase += sign * distance / (metres * stride * (p.to.speed / 0.85).clamp(0.2, 1.0).sqrt()).max(0.05);
            }
            // The jolt springs back, overshooting once: a sway, not a snap.
            p.prev_jolt = p.jolt;
            p.jolt_v += (-p.jolt * 140.0 - p.jolt_v * 11.0) * dt;
            p.jolt = (p.jolt + p.jolt_v * dt).clamp(Vec2::splat(-0.9), Vec2::splat(0.9));
            p.age += dt;
            p.fresh = false;
        }
        self.dolls.tick(dt);
    }

    /// Person `id` goes limp as it stands, thrown by `push` (world, m/s):
    /// a ragdoll until it gets up (any pose but `dead` or `down`) or goes.
    pub fn ragdoll(&mut self, id: u32, push: Vec3) -> Result<bool, String> {
        let Some(p) = self.people.get(&id) else { return Ok(false) };
        let rig = Rig::shared()?;
        let zombie = self.pools.get(&p.pool).is_some_and(|pool| pool.zombie);
        let mut palette = vec![Mat4::IDENTITY; rig.character.joint_count];
        let bind = self.skeleton(rig, p).bind;
        pose(rig, self.skeleton(rig, p), p, zombie, 1.0, &mut palette);
        self.dolls.fall(id, &rig.joint, &bind, &palette, model_transform(p.pos, p.yaw), push)?;
        Ok(true)
    }

    /// Up again: the ragdoll goes, the poses take over.
    pub fn stand(&mut self, id: u32) { self.dolls.remove(id); }

    /// Ragdolls still moving.
    pub fn falling(&self) -> usize { self.dolls.active() }

    /// Fit the people and add every actor to `scene` (once; again after the
    /// scene is rebuilt from scratch). `budget` is the most skinned-vertex
    /// bytes the device binds at once; pools shrink evenly to fit it, and the
    /// answer then says so.
    pub fn attach(&mut self, scene: &mut Scene, budget: u64) -> Result<Option<String>, String> {
        if self.pools.is_empty() { return Ok(None); }
        let rig = Rig::shared()?;
        self.first_actor = scene.skinning.actors.len();
        let first_instance = scene.instances.len();
        let mut palettes = Vec::new();
        for (name, pool) in &mut self.pools {
            if pool.models.is_empty() {
                let skin = pool.cfg.skin.as_deref().and_then(parse_rgb);
                for k in 0..pool.cfg.variety.max(1) {
                    // Spread over the pool's age range, in no order.
                    let age = pool.cfg.age.map(|[lo, hi]| lo + (hi - lo) * (hash(pool.cfg.seed ^ 0xa9e, k as u64) % 1000) as f32 / 999.);
                    pool.models.push(rig.character.warp_human_person(&pool.person(name, k), skin, age)?);
                }
            }
        }
        // What every slot would cost, against what the device holds.
        let bytes = |n: usize| (n * render::skinning::VERTEX_BYTES) as u64;
        let taken: u64 = scene.skinning.actors.iter().map(|a| bytes(a.influences.len())).sum();
        let per: BTreeMap<String, u64> = self.pools.iter().map(|(name, p)| {
            let verts: usize = p.models.iter().map(|m| m.vertex_count()).sum::<usize>() / p.models.len().max(1);
            (name.clone(), bytes(verts))
        }).collect();
        let wanted: u64 = self.pools.iter().map(|(name, p)| per[name] * p.cfg.count as u64).sum();
        let room = (budget / 100 * 95).saturating_sub(taken);
        // The renderer's own limits on skinned actors and vertices count too.
        let actors_free = render::skinning::MAX_ACTORS.saturating_sub(scene.skinning.actors.len()) as u64;
        let verts_free = (render::skinning::MAX_VERTICES.saturating_sub(taken as usize / render::skinning::VERTEX_BYTES)) as u64;
        let slots: u64 = self.pools.values().map(|p| p.cfg.count as u64).sum();
        // The share of every pool that fits, in thousandths.
        let mut share = 1000u64;
        if wanted > room { share = share.min(room * 1000 / wanted.max(1)); }
        if slots > actors_free { share = share.min(actors_free * 1000 / slots.max(1)); }
        let wanted_verts = wanted / render::skinning::VERTEX_BYTES as u64;
        if wanted_verts > verts_free { share = share.min(verts_free * 1000 / wanted_verts.max(1)); }
        let mut note = None;
        let mut total = 0u64;
        for (name, pool) in &mut self.pools {
            pool.cap = if share >= 1000 { pool.cfg.count } else { ((pool.cfg.count as u64 * share / 1000) as usize).max(1) };
            total += pool.cap as u64;
            if pool.cap < pool.cfg.count {
                note = Some(format!("humans: this device or renderer holds fewer skinned people than the pools ask ({} MB, {} actors), so every pool is cut to fit (e.g. {name}: {} of {})", room >> 20, actors_free, pool.cap, pool.cfg.count));
            }
        }
        if total > actors_free { return Err(format!("{total} people do not fit in the renderer's {actors_free} free skinned actors")); }
        for pool in self.pools.values_mut() {
            pool.actors.clear();
            for k in 0..pool.cap {
                let model = &pool.models[k % pool.models.len()];
                // Falls and reaches go outside the walking bounds.
                let handle = model.add_actor_with_bounds(scene, model_transform(HIDDEN, 0.0), model.min.min(Vec3::new(-2.2, -0.3, -2.2)), model.max.max(Vec3::new(2.2, 2.4, 2.2)))?;
                pool.actors.push(handle);
                palettes.push(vec![Mat4::IDENTITY; model.joint_count]);
            }
        }
        let end = scene.instances.len();
        if end > first_instance { scene.dynamic_instances.push(first_instance..end); }
        self.palettes = palettes;
        self.attached = true;
        Ok(note)
    }

    pub fn attached(&self) -> bool { self.attached }

    /// Why the people could not join the scene, if they could not.
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }

    pub fn set_error(&mut self, e: Option<String>) { self.error = e; }

    /// The scene was rebuilt without the actors: add them again.
    pub fn detach(&mut self) { self.attached = false; self.error = None; }

    /// Pose everyone `alpha` of the way from the last tick to this one and
    /// move their actors; then the palettes go to the renderer.
    pub fn draw(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &mut Scene, alpha: f32) -> Result<(), String> {
        if !self.attached { return Ok(()); }
        let rig = Rig::shared()?;
        let mut shown = vec![false; self.palettes.len()];
        let mut index = BTreeMap::new();
        let mut at = 0;
        for (name, pool) in &self.pools {
            index.insert(name.clone(), at);
            at += pool.actors.len();
        }
        for (id, p) in &self.people {
            if !p.visible { continue; }
            let Some(pool) = self.pools.get(&p.pool) else { continue };
            if p.slot >= pool.actors.len() { continue; }
            let k = index[&p.pool] + p.slot;
            let model = &pool.models[p.slot % pool.models.len()];
            let pos = p.prev_pos.lerp(p.pos, alpha);
            let turn = (p.yaw - p.prev_yaw + PI).rem_euclid(TAU) - PI;
            let yaw = p.prev_yaw + turn * alpha;
            let limp = doll_pose(rig, &self.dolls, *id, alpha, &mut self.palettes[k]);
            if limp.is_none() { pose(rig, pool.skeleton(rig, &p.pool, p.slot), p, pool.zombie, alpha, &mut self.palettes[k]); }
            model.pose_face(&mut self.palettes[k], p.age);
            let h = pool.actors[p.slot];
            let at = model_transform(pos, yaw) * Affine3A::from_translation(limp.unwrap_or(Vec3::ZERO));
            scene.update_instance(h.instance, h.mesh, at);
            shown[k] = true;
        }
        // Unused actors wait below the world.
        for (pool, first) in self.pools.values().zip(index.values()) {
            for (slot, h) in pool.actors.iter().enumerate() {
                if !shown[first + slot] {
                    scene.update_instance(h.instance, h.mesh, model_transform(HIDDEN, 0.0));
                    self.palettes[first + slot].fill(Mat4::IDENTITY);
                }
            }
        }
        let instances: Vec<usize> = self.pools.values().flat_map(|p| p.actors.iter().map(|h| h.instance)).collect();
        if let (Some(&a), Some(&b)) = (instances.iter().min(), instances.iter().max()) {
            renderer.update_instances(gpu, a, &scene.instances[a..=b])?;
        }
        if self.first_actor == 0 && scene.skinning.actors.len() == self.palettes.len() {
            renderer.set_skinning_palettes(gpu, self.palettes.iter().map(|p| p.as_slice()).collect::<Vec<_>>().into_iter())?;
        } else {
            for (i, p) in self.palettes.iter().enumerate() { renderer.set_skinning_palette(gpu, self.first_actor + i, p)?; }
        }
        Ok(())
    }

    /// A blow from `dir` (world, its length the strength, ~1 a heavy hit):
    /// the upper body jolts that way and swings back.
    pub fn hit(&mut self, id: u32, dir: Vec3) -> bool {
        let Some(p) = self.people.get_mut(&id) else { return false };
        let ahead = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
        let right = Vec3::new(p.yaw.cos(), 0.0, -p.yaw.sin());
        // Pushed ahead tips the chest forward (+pitch); pushed right rolls it right (-z).
        p.jolt_v += Vec2::new(dir.dot(ahead), -dir.dot(right)) * 9.0;
        true
    }

    /// Where a person's head is now, from the rig alone (no renderer needed).
    pub fn head(&self, id: u32) -> Option<Vec3> { self.joint(id, "head") }

    /// Where one of a person's joints is now (`JOINT_NAMES`), in the world.
    pub fn joint(&self, id: u32, name: &str) -> Option<Vec3> {
        let bone = JOINT_NAMES.iter().find(|(n, _)| *n == name)?.1;
        let p = self.people.get(&id)?;
        let rig = Rig::shared().ok()?;
        let zombie = self.pools.get(&p.pool).is_some_and(|pool| pool.zombie);
        let mut palette = vec![Mat4::IDENTITY; rig.character.joint_count];
        let limp = doll_pose(rig, &self.dolls, id, 1.0, &mut palette);
        let skeleton = self.skeleton(rig, p);
        if limp.is_none() { pose(rig, skeleton, p, zombie, 1.0, &mut palette); }
        let j = rig.joint[bone as usize];
        let lift = if bone == Bone::Head { 0.08 } else { 0.0 };
        let local = palette[j].transform_point3(skeleton.rest(bone) + Vec3::new(0.0, lift, 0.0)) + limp.unwrap_or(Vec3::ZERO);
        Some(model_transform(p.pos, p.yaw).transform_point3(local))
    }
}

/// A ragdoll's palette, centred under its hips (for the culling bounds):
/// the answer is that centre, in model space. None when standing.
fn doll_pose(rig: &Rig, dolls: &crate::ragdoll::Dolls, id: u32, alpha: f32, palette: &mut [Mat4]) -> Option<Vec3> {
    let deltas = dolls.deltas(id, alpha)?;
    let doll = dolls.dolls.get(&id)?;
    let hips = rig.joint[Bone::Pelvis as usize];
    let centre = (deltas[0] * doll.palette[hips]).transform_point3(rig.skeleton.rest(Bone::Pelvis)).with_y(0.0);
    let back = Mat4::from_translation(-centre);
    for (bone, _) in JOINTS {
        let j = rig.joint[bone as usize];
        palette[j] = back * deltas[crate::ragdoll::part(bone)] * doll.palette[j];
    }
    Some(centre)
}

/// Walk, jog and sprint weights for a speed in m/s.
fn blend(speed: f32) -> [f32; 3] {
    let jog = ramp(1.6, 4.6, speed);
    let sprint = ramp(4.8, 6.2, speed);
    [1.0 - jog, jog * (1.0 - sprint), sprint]
}

/// Turn `joints` of `palette` by `q` about `at` (model space).
fn turn(palette: &mut [Mat4], joints: &[usize], at: Vec3, q: Quat) {
    let m = Mat4::from_rotation_translation(q, at - q * at);
    for &j in joints { palette[j] = m * palette[j]; }
}

fn pose(rig: &Rig, body: &Skeleton, p: &Person, zombie: bool, alpha: f32, palette: &mut [Mat4]) {
    let g = Gait { phase: p.from.phase + (p.to.phase - p.from.phase) * alpha, speed: p.from.speed + (p.to.speed - p.from.speed) * alpha };
    let t = (p.prev_t + (p.t - p.prev_t) * alpha).clamp(0.0, 1.0);
    let moving = (g.speed / 0.6).clamp(0.0, 1.0);
    let [w, j, r] = blend(g.speed);
    let mut layers = if matches!(p.pose.as_str(), "crouch" | "feed") {
        vec![Layer { clip: rig.crouch_idle, t: p.age, looped: true, w: 1.0 - moving }, rig.crouch.layer(g.phase, moving)]
    } else if p.pose == "rise" {
        // Getting up off the ground over t 0..1: from where the fall
        // ended, up onto the knees, then upright.
        let fall = rig.clips["dead"];
        let knees = ramp(0.0, 0.6, t);
        let up = ramp(0.45, 1.0, t);
        vec![
            Layer { clip: fall, t: rig.lib.get(fall).length, looped: false, w: 1.0 - knees },
            Layer { clip: rig.crouch_idle, t: p.age, looped: true, w: knees * (1.0 - up) },
            Layer { clip: if zombie { rig.idle } else { body::REST }, t: p.age, looped: true, w: up },
        ]
    } else {
        vec![
            // The capture's idle is a fighter's stance (feet wide, chest low):
            // right for the stooped dead, not for someone standing around.
            Layer { clip: if zombie { rig.idle } else { body::REST }, t: p.age, looped: true, w: 1.0 - moving },
            rig.walk.layer(g.phase, moving * w),
            rig.jog.layer(g.phase, moving * j),
            rig.sprint.layer(g.phase, moving * r),
        ]
    };
    // A clip pose takes over the whole body: attacks, hits, falls.
    if let Some(&clip) = rig.clips.get(p.pose.as_str()) {
        let length = rig.lib.get(clip).length;
        let at = if p.pose == "down" { length } else { t * length };
        let weight = if matches!(p.pose.as_str(), "dead" | "down") { 1.0 } else { 0.85 };
        for l in &mut layers { l.w *= 1.0 - weight; }
        layers.push(Layer { clip, t: at, looped: false, w: weight });
    }
    let pose = body.bound.pose(&rig.lib, &layers);
    for (bone, _) in JOINTS { palette[rig.joint[bone as usize]] = pose[bone as usize] * body.to_rest[bone as usize]; }
    let legs = p.prev_legs + (p.legs - p.prev_legs) * alpha;
    if legs.abs() > 0.01 && !matches!(p.pose.as_str(), "dead" | "down") {
        // Strafing: the hips and legs turn towards the walk, the waist
        // takes part of the twist, the chest keeps facing ahead.
        let hips = palette[rig.joint[Bone::Pelvis as usize]].transform_point3(body.rest(Bone::Pelvis));
        let lower: Vec<usize> = [Bone::Pelvis, Bone::ThighL, Bone::CalfL, Bone::FootL, Bone::BallL, Bone::ThighR, Bone::CalfR, Bone::FootR, Bone::BallR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &lower, hips, Quat::from_rotation_y(legs));
        let waist = palette[rig.joint[Bone::Spine1 as usize]].transform_point3(body.rest(Bone::Spine1));
        turn(palette, &[rig.joint[Bone::Spine1 as usize]], waist, Quat::from_rotation_y(legs * 0.45));
    }

    let at = |b: Bone, palette: &[Mat4]| palette[rig.joint[b as usize]].transform_point3(body.rest(b));
    let arm = |left: bool| if left { [Bone::UpperArmL, Bone::LowerArmL, Bone::HandL] } else { [Bone::UpperArmR, Bone::LowerArmR, Bone::HandR] }.map(|b| rig.joint[b as usize]);
    let lying = matches!(p.pose.as_str(), "dead" | "down") || (p.pose == "rise" && t < 0.35);

    // Standing relaxed: nobody is a statue. The chest rises with each
    // breath, the weight drifts from one leg to the other, the head looks
    // about, and the arms hang with the elbows a little bent. Each person
    // keeps their own time.
    let still = (1.0 - moving) * if rig.clips.contains_key(p.pose.as_str()) { 0.15 } else { 1.0 };
    if !zombie && !matches!(p.pose.as_str(), "crouch" | "feed" | "rise") && still > 0.01 {
        let own = p.slot as f32 * 2.399;
        let breath = (p.age * (1.25 + 0.2 * own.sin()) + own).sin();
        let drift = (p.age * 0.31 + own * 1.7).sin();
        let waist = at(Bone::Spine1, palette);
        let upper: Vec<usize> = [Bone::Spine1, Bone::Spine2, Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &upper, waist, Quat::from_rotation_z(still * 0.02 * drift));
        let chest = at(Bone::Spine3, palette);
        let ribs: Vec<usize> = [Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &ribs, chest, Quat::from_rotation_x(-still * 0.012 * breath));
        let neck = at(Bone::Neck, palette);
        let glance = (p.age * 0.23 + own).sin() * (p.age * 0.071 + own * 0.6).sin();
        turn(palette, &[rig.joint[Bone::Neck as usize], rig.joint[Bone::Head as usize]], neck, Quat::from_rotation_y(still * 0.22 * glance) * Quat::from_rotation_x(still * 0.012 * breath));
        for left in [true, false] {
            let side = if left { 1.0 } else { -1.0 };
            let [upper, fore, hand] = arm(left);
            let shoulder = at(if left { Bone::UpperArmL } else { Bone::UpperArmR }, palette);
            // The capture's rest holds the arms a hand away from the hips.
            turn(palette, &[upper, fore, hand], shoulder, Quat::from_rotation_z(-side * still * 0.10) * Quat::from_rotation_x(still * 0.04));
            let elbow = at(if left { Bone::LowerArmL } else { Bone::LowerArmR }, palette);
            turn(palette, &[fore, hand], elbow, Quat::from_rotation_x(-still * (0.22 + 0.02 * breath)));
        }
    }

    if zombie && !lying {
        // Stooped: the chest and head pitch forward over the hips, the head
        // lolls to one side and the shoulders lurch with each step.
        let hips = at(Bone::Spine1, palette);
        let lurch = (g.phase * TAU).sin() * 0.12 * moving;
        let upper: Vec<usize> = [Bone::Spine2, Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &upper, hips, Quat::from_rotation_x(0.28) * Quat::from_rotation_z(lurch));
        let neck = at(Bone::Neck, palette);
        let sway = (p.age * 0.7).sin() * 0.15;
        turn(palette, &[rig.joint[Bone::Neck as usize], rig.joint[Bone::Head as usize]], neck, Quat::from_rotation_z(0.25 + sway) * Quat::from_rotation_x(0.15));
    }

    // Feeding: down on the haunches and doubled over what lies in front,
    // the face in it.
    if p.pose == "feed" {
        let hips = at(Bone::Spine1, palette);
        let upper: Vec<usize> = [Bone::Spine2, Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &upper, hips, Quat::from_rotation_x(0.95));
        let neck = at(Bone::Neck, palette);
        turn(palette, &[rig.joint[Bone::Neck as usize], rig.joint[Bone::Head as usize]], neck, Quat::from_rotation_x(0.35));
    }

    // Out of breath: bent forward from the waist, the chest heaving with
    // each breath, the head up to see ahead, the arms heavy.
    let tired = (p.prev_tired + (p.tired - p.prev_tired) * alpha).clamp(0.0, 1.0);
    if tired > 0.01 && !lying {
        let waist = at(Bone::Spine1, palette);
        let breath = (p.age * 5.5).sin();
        let upper: Vec<usize> = [Bone::Spine2, Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &upper, waist, Quat::from_rotation_x(tired * (0.42 + 0.05 * breath)));
        let chest = at(Bone::Spine3, palette);
        let shoulders: Vec<usize> = [Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &shoulders, chest, Quat::from_rotation_x(-tired * 0.06 * breath));
        let neck = at(Bone::Neck, palette);
        turn(palette, &[rig.joint[Bone::Neck as usize], rig.joint[Bone::Head as usize]], neck, Quat::from_rotation_x(-tired * 0.3));
    }

    // Wound up: the chest turns the right shoulder back about the waist and
    // leans away from the blow to come; negative swings through past front.
    let coil = p.prev_coil + (p.coil - p.prev_coil) * alpha;
    if coil.abs() > 0.01 && !lying {
        let waist = at(Bone::Spine1, palette);
        let upper: Vec<usize> = [Bone::Spine2, Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &upper, waist, Quat::from_rotation_y(-coil * 0.8) * Quat::from_rotation_x(-coil.max(0.0) * 0.15));
        // The head keeps looking at the target.
        let neck = at(Bone::Neck, palette);
        turn(palette, &[rig.joint[Bone::Neck as usize], rig.joint[Bone::Head as usize]], neck, Quat::from_rotation_y(coil * 0.6));
    }

    // A blow: the upper body jolts along it from the waist, the head
    // whips further, and both swing back.
    let jolt = p.prev_jolt + (p.jolt - p.prev_jolt) * alpha;
    if jolt.length_squared() > 1e-5 && !lying {
        let waist = at(Bone::Spine1, palette);
        let upper: Vec<usize> = [Bone::Spine2, Bone::Spine3, Bone::Neck, Bone::Head, Bone::ClavicleL, Bone::UpperArmL, Bone::LowerArmL, Bone::HandL, Bone::ClavicleR, Bone::UpperArmR, Bone::LowerArmR, Bone::HandR].iter().map(|b| rig.joint[*b as usize]).collect();
        turn(palette, &upper, waist, Quat::from_rotation_x(jolt.x) * Quat::from_rotation_z(jolt.y));
        let neck = at(Bone::Neck, palette);
        turn(palette, &[rig.joint[Bone::Neck as usize], rig.joint[Bone::Head as usize]], neck, Quat::from_rotation_x(jolt.x * 0.6) * Quat::from_rotation_z(jolt.y * 0.6));
    }

    // Arm poses: the arms swing up about the shoulders (model +z is ahead).
    let raise = |palette: &mut [Mat4], left: bool, pitch: f32, spread: f32| {
        let shoulder = at(if left { Bone::UpperArmL } else { Bone::UpperArmR }, palette);
        let side = if left { 1.0 } else { -1.0 };
        turn(palette, &arm(left), shoulder, Quat::from_rotation_z(side * spread) * Quat::from_rotation_x(-pitch));
    };
    let reach = if zombie && !lying && !rig.clips.contains_key(p.pose.as_str()) { 1.25 } else { 0.0 };
    match p.pose.as_str() {
        // Wind-up: both arms high over the head.
        "raise" => { raise(palette, true, 2.5, 0.1); raise(palette, false, 2.5, 0.1); }
        // The blow: from high to low ahead over t.
        "strike" => { let a = 2.5 - 1.9 * t; raise(palette, true, a, 0.05); raise(palette, false, a, 0.05); }
        "reach" => { raise(palette, true, 1.35, 0.05); raise(palette, false, 1.35, 0.05); }
        "shove" => { let a = 1.2 + 0.3 * (t * PI).sin(); raise(palette, true, a, -0.1); raise(palette, false, a, -0.1); }
        "aim" => raise(palette, false, 1.45, -0.05),
        // Over here: the right hand high, waving side to side.
        "wave" => raise(palette, false, 2.6, 0.15 + 0.35 * (p.age * 7.0).sin()),
        _ if reach > 0.0 => {
            // The zombie's arms hang forward, swaying out of step.
            let s = (g.phase * TAU).sin() * 0.12;
            raise(palette, true, reach + s, 0.08);
            raise(palette, false, reach - s, 0.08);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crowd_fits_the_renderer() {
        // Dead Miles asks for 66; more than the old 64-actor limit.
        let mut cfg = BTreeMap::new();
        cfg.insert("a".to_string(), HumanPool { count: 50, variety: 1, style: "zombie".into(), ..Default::default() });
        cfg.insert("b".to_string(), HumanPool { count: 16, variety: 1, ..Default::default() });
        let mut h = Humans::new(&cfg);
        let mut scene = render::Scene::default();
        assert_eq!(h.attach(&mut scene, 1 << 31).unwrap(), None);
        assert_eq!(scene.skinning.actors.len(), 66);
        // The face and eyes of every instance share one recipe per identity.
        // A CPU scene owns only previews, even after all 66 actors are attached.
        assert_eq!(scene.generated_textures.len(), 2);
        for &layer in scene.generated_textures.keys() {
            let face = &scene.textures[layer as usize];
            assert_eq!(face.rgba.dimensions(), (64, 64));
            assert_eq!(face.normal.as_ref().unwrap().dimensions(), (64, 64));
        }
        // A small device cuts every pool instead of failing.
        let mut h = Humans::new(&cfg);
        let mut scene = render::Scene::default();
        let note = h.attach(&mut scene, 64 << 20).unwrap().expect("a note that pools were cut");
        assert!(note.contains("cut"), "{note}");
        let (free, total) = h.room("a").unwrap();
        assert!(total < 50 && free == total, "{free} of {total}");
        assert!(scene.skinning.actors.len() < 66);
    }

    #[test]
    fn a_ragdoll_falls_where_it_stood_along_the_blow_and_settles() {
        let mut cfg = BTreeMap::new();
        cfg.insert("z".to_string(), HumanPool { count: 2, style: "zombie".into(), ..Default::default() });
        let mut h = Humans::new(&cfg);
        let a = h.spawn("z", Vec3::new(3.0, 0.0, 2.0), 0.6).unwrap().unwrap();
        h.tick(1.0 / 60.0);
        let names = ["head", "hips", "hand_l", "foot_r", "knee_l"];
        let before: Vec<Vec3> = names.iter().map(|n| h.joint(a, n).unwrap()).collect();
        assert!(h.ragdoll(a, Vec3::new(4.0, 0.0, 0.0)).unwrap());
        // The fall starts from the pose it stood in.
        for (n, b) in names.iter().zip(&before) {
            let now = h.joint(a, n).unwrap();
            assert!(now.distance(*b) < 0.02, "{n} jumped from {b} to {now}");
        }
        for _ in 0..30 { h.tick(1.0 / 60.0); }
        assert!(h.falling() == 1);
        for _ in 0..330 { h.tick(1.0 / 60.0); }
        assert_eq!(h.falling(), 0, "settled and frozen");
        let head = h.joint(a, "head").unwrap();
        let hips = h.joint(a, "hips").unwrap();
        assert!(head.y < 0.5 && head.y > -0.1, "the head lies on the ground: {head}");
        assert!(hips.y < 0.45 && hips.y > -0.1, "hips {hips}");
        assert!(head.x > before[0].x + 0.3, "fell along the blow: {head} from {}", before[0]);
        for n in names { assert!(h.joint(a, n).unwrap().is_finite()); }
        // Getting up (any other pose) ends it; removing frees everything.
        h.stand(a);
        assert!(h.joint(a, "head").unwrap().y > 1.2);
        h.ragdoll(a, Vec3::Z).unwrap();
        assert!(h.remove(a));
        assert_eq!(h.falling(), 0);
    }

    #[test]
    fn strafing_turns_the_legs_not_the_body() {
        let mut cfg = BTreeMap::new();
        cfg.insert("p".to_string(), HumanPool { count: 1, ..Default::default() });
        let mut h = Humans::new(&cfg);
        let a = h.spawn("p", Vec3::ZERO, 0.0).unwrap().unwrap();
        let walk = |h: &mut Humans, step: Vec3| for _ in 0..60 { h.tick_start(); let p = h.people.get_mut(&a).unwrap(); p.pos += step / 60.0; h.tick(1.0 / 60.0); };
        // Facing +z, walking +x: the legs turn right, the cycle runs forward.
        walk(&mut h, Vec3::X * 1.3);
        let p = &h.people[&a];
        assert!(p.legs > 1.0, "legs {}", p.legs);
        let phase = p.to.phase;
        // Straight back: no turn, the cycle runs backwards.
        walk(&mut h, -Vec3::Z * 1.3);
        let p = &h.people[&a];
        assert!(p.legs.abs() < 0.1, "legs {}", p.legs);
        assert!(p.to.phase < phase, "backwards: {} after {}", p.to.phase, phase);
        // Standing still: the legs come back under the body.
        walk(&mut h, Vec3::ZERO);
        assert!(h.people[&a].legs.abs() < 0.05);
    }

    #[test]
    fn a_person_stands_upright_on_their_own_shoulders() {
        let rig = Rig::shared().unwrap();
        let mut cfg = BTreeMap::new();
        cfg.insert("p".to_string(), HumanPool { count: 1, variety: 1, height: [1.8, 1.8], ..Default::default() });
        let mut h = Humans::new(&cfg);
        let a = h.spawn("p", Vec3::ZERO, 0.0).unwrap().unwrap();
        for _ in 0..30 { h.tick(1.0 / 60.0); }
        let at = |n: &str| h.joint(a, n).unwrap();
        // The rig's own shoulders are 0.57 m apart; an adult's about 0.36 m.
        let width = at("shoulder_l").distance(at("shoulder_r"));
        assert!(width > 0.30 && width < 0.42, "shoulders {width} m apart");
        // Head over the hips, hands by the thighs, feet under the hips.
        let (head, hips) = (at("head"), at("hips"));
        assert!((head.z - hips.z).abs() < 0.06, "head {head} over hips {hips}");
        for hand in ["hand_l", "hand_r"] {
            let p = at(hand);
            assert!(p.y < hips.y && p.y > 0.6 && p.x.abs() < 0.36, "{hand} hangs by the thigh: {p}");
        }
        assert!(at("foot_l").distance(at("foot_r")) < 0.30, "feet under the hips");
        // Standing still is not a statue: the chest breathes.
        let mut palette = vec![Mat4::IDENTITY; rig.character.joint_count];
        let mut p = h.people[&a].clone();
        let chest = |p: &Person, palette: &mut Vec<Mat4>| { pose(rig, h.skeleton(rig, p), p, false, 1.0, palette); palette[rig.joint[Bone::Spine3 as usize]] };
        let now = chest(&p, &mut palette);
        p.age += 1.3;
        assert!(chest(&p, &mut palette) != now);
    }

    #[test]
    fn the_rig_loads_and_walks() {
        let rig = Rig::shared().unwrap();
        assert!(rig.walk.metres > 0.5 && rig.walk.metres < 3.0, "a walk cycle covers {} m", rig.walk.metres);
        let mut cfg = BTreeMap::new();
        cfg.insert("zombie".to_string(), HumanPool { count: 2, style: "zombie".into(), ..Default::default() });
        let mut h = Humans::new(&cfg);
        let a = h.spawn("zombie", Vec3::ZERO, 0.0).unwrap().unwrap();
        let b = h.spawn("zombie", Vec3::X, 0.0).unwrap().unwrap();
        assert_eq!(h.spawn("zombie", Vec3::X, 0.0).unwrap(), None, "the pool is full");
        assert!(h.spawn("nobody", Vec3::X, 0.0).is_err());
        for i in 1..=60 {
            h.tick_start();
            h.people.get_mut(&a).unwrap().pos = Vec3::new(0.0, 0.0, i as f32 * 1.3 / 60.0);
            h.tick(1.0 / 60.0);
        }
        let p = &h.people[&a];
        assert!(p.to.speed > 1.0 && p.to.speed < 1.6, "speed {}", p.to.speed);
        assert!(p.prev_pos.z < p.pos.z, "rendering needs both tick endpoints");
        assert!(p.to.phase > 0.3, "the legs moved: phase {}", p.to.phase);
        let head = h.head(a).unwrap();
        assert!(head.y > 1.2 && head.y < 2.1, "head at {head}");
        assert!(h.remove(b));
        assert_eq!(h.room("zombie"), Some((1, 2)));
        for pose in POSES {
            let mut p = h.people[&a].clone();
            p.pose = pose.to_string();
            p.t = 0.5;
            let mut palette = vec![Mat4::IDENTITY; rig.character.joint_count];
            super::pose(rig, &rig.skeleton, &p, true, 0.5, &mut palette);
            assert!(palette.iter().all(|m| m.is_finite()), "{pose}");
        }
    }
}
