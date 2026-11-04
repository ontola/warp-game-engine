//! Bodies that move without anyone animating them. A game says what a body
//! is doing (`Doing`), `Gait` turns that into blended clip layers, and those
//! pose either a rig of rigid pieces (`Bound`) or any skinned character whose
//! clips fill a `ClipSet`.
//!
//! The standard skeleton is `Bone`; `Library::builtin` holds motion-captured
//! clips for it from Quaternius' Universal Animation Library (CC0,
//! tools/ual_to_json.py -> assets/ual.json). A model built from rigid pieces
//! needs only a `Rig`: a few joint spots in its own standing pose. Each piece
//! hangs on one bone, a clip turns each bone by how far the capture's bone
//! turned from its rest pose (the T-pose with the arms let down), and the
//! joints follow down the chain. Bones a rig leaves out ride on their parent,
//! so a one-piece arm swings from the shoulder alone.
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;
use std::collections::HashMap;

pub const BONES: usize = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Bone {
    Pelvis,
    Spine1,
    Spine2,
    Spine3,
    Neck,
    Head,
    ClavicleL,
    UpperArmL,
    LowerArmL,
    HandL,
    ClavicleR,
    UpperArmR,
    LowerArmR,
    HandR,
    ThighL,
    CalfL,
    FootL,
    BallL,
    ThighR,
    CalfR,
    FootR,
    BallR,
}

use Bone::*;

const ALL: [Bone; BONES] =
    [Pelvis, Spine1, Spine2, Spine3, Neck, Head, ClavicleL, UpperArmL, LowerArmL, HandL, ClavicleR, UpperArmR, LowerArmR, HandR, ThighL, CalfL, FootL, BallL, ThighR, CalfR, FootR, BallR];

/// The names in ual.json, in `Bone` order.
const NAMES: [&str; BONES] = [
    "pelvis",
    "spine_01",
    "spine_02",
    "spine_03",
    "neck_01",
    "Head",
    "clavicle_l",
    "upperarm_l",
    "lowerarm_l",
    "hand_l",
    "clavicle_r",
    "upperarm_r",
    "lowerarm_r",
    "hand_r",
    "thigh_l",
    "calf_l",
    "foot_l",
    "ball_l",
    "thigh_r",
    "calf_r",
    "foot_r",
    "ball_r",
];

impl Bone {
    pub fn parent(self) -> Option<Bone> {
        Some(match self {
            Pelvis => return None,
            Spine1 | ThighL | ThighR => Pelvis,
            Spine2 => Spine1,
            Spine3 => Spine2,
            Neck | ClavicleL | ClavicleR => Spine3,
            Head => Neck,
            UpperArmL => ClavicleL,
            LowerArmL => UpperArmL,
            HandL => LowerArmL,
            UpperArmR => ClavicleR,
            LowerArmR => UpperArmR,
            HandR => LowerArmR,
            CalfL => ThighL,
            FootL => CalfL,
            BallL => FootL,
            CalfR => ThighR,
            FootR => CalfR,
            BallR => FootR,
        })
    }

    /// The next bone down its own chain (where its segment points).
    fn next(self) -> Option<Bone> {
        Some(match self {
            Pelvis => Spine1,
            Spine1 => Spine2,
            Spine2 => Spine3,
            Spine3 => Neck,
            Neck => Head,
            ClavicleL => UpperArmL,
            UpperArmL => LowerArmL,
            LowerArmL => HandL,
            ClavicleR => UpperArmR,
            UpperArmR => LowerArmR,
            LowerArmR => HandR,
            ThighL => CalfL,
            CalfL => FootL,
            FootL => BallL,
            ThighR => CalfR,
            CalfR => FootR,
            FootR => BallR,
            Head | HandL | HandR | BallL | BallR => return None,
        })
    }
}

pub struct Clip {
    pub length: f32,
    /// When its blow lands (the hand at its fastest).
    pub hit: f32,
    /// How fast it walks, in hip heights a second.
    pub speed: f32,
    rot: Vec<[Quat; BONES]>,
    root: Vec<Vec3>,
}

impl Clip {
    /// The bones' turns and the pelvis' move (over hip height) at `t`.
    fn sample(&self, t: f32, looped: bool, fps: f32) -> ([Quat; BONES], Vec3) {
        let n = self.rot.len();
        if n == 1 {
            return (self.rot[0], self.root[0]);
        }
        let f = if looped { (t / self.length).rem_euclid(1.0) * (n - 1) as f32 } else { (t * fps).clamp(0.0, (n - 1) as f32) };
        let a = (f.floor() as usize).min(n - 2);
        let u = f - a as f32;
        let mut q = self.rot[a];
        for (b, q) in q.iter_mut().enumerate() {
            *q = q.slerp(self.rot[a + 1][b], u);
        }
        (q, self.root[a].lerp(self.root[a + 1], u))
    }
}

pub struct Library {
    fps: f32,
    clips: Vec<Clip>,
    named: HashMap<String, usize>,
}

impl Library {
    /// The clips every game gets (see the crate docs).
    pub fn builtin() -> Result<Library, String> {
        Library::parse(include_str!("../assets/ual.json"))
    }

    /// A library in tools/ual_to_json.py's format.
    pub fn parse(text: &str) -> Result<Library, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("ual.json: {e}"))?;
        let fps = v["fps"].as_f64().unwrap_or(30.0) as f32;
        let mut lib = Library { fps, clips: Vec::new(), named: HashMap::new() };
        let floats = |v: &Value| -> Vec<f32> { v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default() };
        for (name, c) in v["clips"].as_object().ok_or("ual.json: no clips")? {
            let frames = c["frames"].as_u64().unwrap_or(0) as usize;
            let tracks: Vec<Vec<f32>> = NAMES.iter().map(|b| floats(&c["rot"][*b])).collect();
            let root = floats(&c["root"]);
            if frames == 0 || tracks.iter().any(|t| t.len() != frames * 4) || root.len() != frames * 3 {
                return Err(format!("ual.json: clip {name} is short"));
            }
            let rot = (0..frames).map(|f| std::array::from_fn(|b| Quat::from_slice(&tracks[b][f * 4..f * 4 + 4]).normalize())).collect();
            let root = (0..frames).map(|f| Vec3::from_slice(&root[f * 3..f * 3 + 3])).collect();
            lib.named.insert(name.clone(), lib.clips.len());
            lib.clips.push(Clip { length: c["length"].as_f64().unwrap_or(1.0) as f32, hit: c["hit"].as_f64().unwrap_or(0.0) as f32, speed: c["speed"].as_f64().unwrap_or(0.0) as f32, rot, root });
        }
        Ok(lib)
    }

    pub fn clip(&self, name: &str) -> Option<usize> {
        self.named.get(name).copied()
    }

    pub fn get(&self, i: usize) -> &Clip {
        &self.clips[i]
    }

    /// Every clip's length, by id (for a `ClipSet`).
    pub fn lengths(&self) -> Vec<f32> {
        self.clips.iter().map(|c| c.length).collect()
    }
}

/// A model's joints, in its own space and standing pose.
pub struct Rig {
    pub joints: &'static [(Bone, [f32; 3])],
    /// Spots that pull the nearest piece onto a bone (a sword to its hand).
    pub grips: &'static [([f32; 3], Bone)],
}

/// One clip in a blend: which, where in it, and how much of the pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub clip: usize,
    pub t: f32,
    pub looped: bool,
    pub w: f32,
}

/// A rig with its pieces hung on bones.
pub struct Bound {
    pub rest: [Option<Vec3>; BONES],
    /// Each bone's nearest kept bone up its chain (itself when kept).
    keep: [Bone; BONES],
    hip: f32,
    /// The bone of each piece, in the order given to `new`.
    pub bone_of: Vec<u8>,
}

impl Bound {
    /// Hang pieces on `rig`: each, by where its middle is, on the nearest bone.
    pub fn new(rig: &Rig, pieces: &[Vec3]) -> Bound {
        let mut rest = [None; BONES];
        for &(b, p) in rig.joints {
            rest[b as usize] = Some(Vec3::from(p));
        }
        let keep = ALL.map(|b| {
            let mut k = b;
            while rest[k as usize].is_none() {
                match k.parent() {
                    Some(p) => k = p,
                    None => break,
                }
            }
            k
        });
        // Each kept bone's segment, to its next kept bone down the chain or
        // a short stub past a leaf.
        let mut segs: Vec<(Vec3, Vec3, Bone)> = Vec::new();
        for b in ALL {
            let Some(a) = rest[b as usize] else { continue };
            let mut n = b.next();
            while let Some(c) = n.filter(|c| rest[*c as usize].is_none()) {
                n = c.next();
            }
            let end = match n.and_then(|c| rest[c as usize]) {
                Some(e) => e,
                None => {
                    let from = b.parent().map(|p| rest[keep[p as usize] as usize].unwrap_or(a)).unwrap_or(a);
                    a + (a - from).normalize_or_zero() * 0.12
                }
            };
            segs.push((a, end, b));
        }
        for &(p, b) in rig.grips {
            segs.push((Vec3::from(p), Vec3::from(p), keep[b as usize]));
        }
        let bone_of = pieces
            .iter()
            .map(|&c| segs.iter().min_by(|x, y| seg_dist(c, x.0, x.1).total_cmp(&seg_dist(c, y.0, y.1))).map_or(Pelvis as u8, |s| s.2 as u8))
            .collect();
        let hip = rest[Pelvis as usize].map(|p| p.y).unwrap_or(1.0).max(0.1);
        Bound { rest, keep, hip, bone_of }
    }

    /// Each bone's transform in model space for the blended `layers`.
    pub fn pose(&self, lib: &Library, layers: &[Layer]) -> [Mat4; BONES] {
        let mut rot = [Quat::IDENTITY; BONES];
        let mut root = Vec3::ZERO;
        let total: f32 = layers.iter().map(|l| l.w).sum();
        if total > 1e-4 {
            let mut acc = [glam::Vec4::ZERO; BONES];
            for l in layers {
                if l.w <= 0.0 {
                    continue;
                }
                let (q, r) = lib.get(l.clip).sample(l.t, l.looped, lib.fps);
                let w = l.w / total;
                root += r * w;
                for b in 0..BONES {
                    let v = glam::Vec4::from(q[b]);
                    let s = if acc[b].dot(v) < 0.0 { -w } else { w };
                    acc[b] += v * s;
                }
            }
            for b in 0..BONES {
                rot[b] = Quat::from_vec4(acc[b]).normalize();
            }
        }
        // Bones left out turn with their kept parent.
        let rot: [Quat; BONES] = std::array::from_fn(|b| rot[self.keep[b] as usize]);
        let mut at = [Vec3::ZERO; BONES];
        let mut out = [Mat4::IDENTITY; BONES];
        for b in ALL {
            let i = b as usize;
            let Some(r) = self.rest[i] else { continue };
            at[i] = match b.parent().map(|p| self.keep[p as usize]) {
                Some(p) if self.rest[p as usize].is_some() && p != b => {
                    let pr = self.rest[p as usize].unwrap_or(r);
                    at[p as usize] + rot[p as usize] * (r - pr)
                }
                _ => r + root * self.hip,
            };
            out[i] = Mat4::from_translation(at[i]) * Mat4::from_quat(rot[i]) * Mat4::from_translation(-r);
        }
        std::array::from_fn(|b| out[self.keep[b] as usize])
    }

    pub fn hip(&self) -> f32 {
        self.hip
    }
}

fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-8 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

/// The clips a gait plays, as ids a caller knows how to pose: into a
/// `Library`, or a skinned character's own list.
pub struct ClipSet {
    pub idle: usize,
    pub walk: usize,
    pub jog: usize,
    pub hit: usize,
    pub death: usize,
    /// Each clip's length in seconds, by id.
    pub lengths: Vec<f32>,
}

/// An attack, as clips: what the wind-up plays and how far, and what plays on
/// from where after the blow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Action {
    pub wind: usize,
    pub wind_to: f32,
    pub release: usize,
    pub release_from: f32,
}

/// A one-off clip over the body's motion: a blow, a flinch, a death.
#[derive(Clone, Copy, Debug)]
struct Shot {
    clip: usize,
    t: f32,
    w: f32,
    /// Its weight at full (a flinch only partly takes over).
    full: f32,
}

/// What a body is doing, as the game sees it.
pub struct Doing {
    /// Ground speed (m/s).
    pub speed: f32,
    /// Winding up: the attack and how far in (0 to 1).
    pub wind: Option<(Action, f32)>,
    /// Just got hit.
    pub hit: bool,
    pub dead: bool,
    /// Frozen or stunned: the body holds still.
    pub held: bool,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn toward(v: &mut f32, to: f32, rate: f32, dt: f32) {
    *v += (to - *v) * (rate * dt).min(1.0);
}

/// A body's animation state, frame to frame: what to blend, from what it is
/// doing and how long its legs are.
#[derive(Clone, Debug, Default)]
pub struct Gait {
    phase: f32,
    idle_t: f32,
    moving: f32,
    jog: f32,
    blow: Option<Shot>,
    flinch: Option<Shot>,
    death: Option<Shot>,
    /// The attack winding up, until it lets go.
    attack: Option<Action>,
}

impl Gait {
    /// The layers to blend this frame. `hip`: how high its hips stand, in
    /// metres: short legs step faster to keep the same speed.
    pub fn step(&mut self, c: &ClipSet, hip: f32, d: &Doing, dt: f32) -> Vec<Layer> {
        let dt = if d.held { 0.0 } else { dt };
        // A body this much bigger than the capture walks this much slower.
        let size = (hip / 0.92).max(0.3);
        let v = d.speed / size;
        toward(&mut self.moving, smooth(0.15, 0.7, v), 8.0, dt);
        toward(&mut self.jog, smooth(1.4, 3.0, v), 5.0, dt);
        // Steps a second: a stroll to a run.
        let cadence = (0.8 + 0.6 * smooth(0.8, 4.0, v)) / size.sqrt();
        self.phase = (self.phase + cadence * dt).fract();
        self.idle_t += dt;
        let len = |clip: usize| c.lengths[clip];
        let mut out = vec![
            Layer { clip: c.idle, t: self.idle_t, looped: true, w: 1.0 - self.moving },
            Layer { clip: c.walk, t: self.phase * len(c.walk), looped: true, w: self.moving * (1.0 - self.jog) },
            Layer { clip: c.jog, t: self.phase * len(c.jog), looped: true, w: self.moving * self.jog },
        ];
        // The blow: held to the wind-up so it lands as the attack does, then
        // played out.
        if let Some((action, k)) = d.wind {
            let b = self.blow.get_or_insert(Shot { clip: action.wind, t: 0.0, w: 0.0, full: 1.0 });
            b.clip = action.wind;
            b.t = k * action.wind_to;
            toward(&mut b.w, 1.0, 12.0, dt);
            self.attack = Some(action);
        } else if let Some(b) = &mut self.blow {
            if let Some(action) = self.attack.take() {
                // Let go: the release clip from where the blow lands.
                b.clip = action.release;
                b.t = action.release_from;
            }
            b.t += dt;
            let left = len(b.clip) - b.t;
            b.w = b.w.min((left / 0.25).max(0.0));
            if left <= 0.0 {
                self.blow = None;
            }
        }
        if d.hit && !d.dead {
            self.flinch = Some(Shot { clip: c.hit, t: 0.0, w: 0.7, full: 0.7 });
        }
        if let Some(f) = &mut self.flinch {
            f.t += dt;
            f.w = f.full * (1.0 - (f.t / len(f.clip))).max(0.0);
            if f.t >= len(f.clip) {
                self.flinch = None;
            }
        }
        if d.dead {
            let s = self.death.get_or_insert(Shot { clip: c.death, t: 0.0, w: 0.0, full: 1.0 });
            s.t += dt;
            toward(&mut s.w, 1.0, 14.0, dt);
            self.blow = None;
            self.flinch = None;
        }
        // Each one-off takes its share over what lies under it.
        for s in [self.blow, self.flinch, self.death].into_iter().flatten() {
            let keep = 1.0 - s.w.clamp(0.0, 1.0);
            for l in &mut out {
                l.w *= keep;
            }
            out.push(Layer { clip: s.clip, t: s.t, looped: false, w: s.w });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> ClipSet {
        ClipSet { idle: 0, walk: 1, jog: 2, hit: 3, death: 4, lengths: vec![1.0; 5] }
    }

    fn walk_phase_after(hip: f32) -> f32 {
        let mut g = Gait::default();
        let d = Doing { speed: 2.0, wind: None, hit: false, dead: false, held: false };
        for _ in 0..12 {
            g.step(&set(), hip, &d, 1.0 / 60.0);
        }
        g.phase
    }

    #[test]
    fn short_legs_step_faster_at_the_same_speed() {
        assert!(walk_phase_after(0.4) > walk_phase_after(1.8));
    }

    #[test]
    fn a_blow_lands_where_the_wind_up_ends_and_plays_out() {
        let mut g = Gait::default();
        let a = Action { wind: 3, wind_to: 0.4, release: 3, release_from: 0.4 };
        let mut d = Doing { speed: 0.0, wind: Some((a, 1.0)), hit: false, dead: false, held: false };
        for _ in 0..30 {
            g.step(&set(), 0.9, &d, 1.0 / 60.0);
        }
        d.wind = None;
        let l = g.step(&set(), 0.9, &d, 1.0 / 60.0);
        let blow = l.iter().find(|l| !l.looped).unwrap();
        assert!((blow.t - 0.4 - 1.0 / 60.0).abs() < 1e-4, "{blow:?}");
        for _ in 0..60 {
            g.step(&set(), 0.9, &d, 1.0 / 60.0);
        }
        assert!(g.step(&set(), 0.9, &d, 1.0 / 60.0).iter().all(|l| l.looped));
    }

    #[test]
    fn builtin_clips_load_and_stand_a_rig_up() {
        const RIG: Rig = Rig { joints: &[(Pelvis, [0.0, 1.0, 0.0]), (Head, [0.0, 1.7, 0.0]), (ThighL, [0.1, 1.0, 0.0]), (FootL, [0.1, 0.05, 0.0])], grips: &[] };
        let lib = Library::builtin().unwrap();
        let idle = lib.clip("Idle_Loop").unwrap();
        let b = Bound::new(&RIG, &[Vec3::new(0.0, 1.76, 0.0), Vec3::new(0.1, 0.5, 0.0)]);
        assert_eq!(b.bone_of[0], Head as u8);
        let m = b.pose(&lib, &[Layer { clip: idle, t: 0.0, looped: true, w: 1.0 }]);
        assert!(m[Head as usize].transform_point3(Vec3::new(0.0, 1.7, 0.0)).y > 1.2);
    }
}
