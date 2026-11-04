//! Real body animation for the monsters: motion-captured clips from
//! Quaternius' Universal Animation Library (CC0, tools/ual_to_json.py ->
//! assets/anim/ual.json) moved onto our own models.
//!
//! The monsters are built from rigid pieces (bones, arms, a robe), so each
//! piece hangs on one bone of a rig: a few joint spots per model, in the
//! model's own standing pose. A clip turns each bone by how far the capture's
//! bone turned from its rest pose (the T-pose with the arms let down), and
//! the joints follow down the chain. Bones a model leaves out ride on their
//! parent, so a one-piece arm swings from the shoulder alone.
use crate::kits::{Kit, Kits};
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

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
    pub fn load() -> Result<Library, String> {
        let path = dir().join("ual.json");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("ual.json: {e}"))?;
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
}

fn dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent().map(|d| d.join("anim")).filter(|d| d.join("ual.json").exists()) {
            return d;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/anim")
}

/// One model's joints, in its own space and standing pose.
pub struct RigDef {
    pub joints: &'static [(Bone, [f32; 3])],
    /// Spots that pull the nearest piece onto a bone (a sword to its hand).
    pub grips: &'static [([f32; 3], Bone)],
}

/// A model's rig with its pieces hung on bones.
pub struct Bound {
    rest: [Option<Vec3>; BONES],
    /// Each bone's nearest kept bone up its chain (itself when kept).
    keep: [Bone; BONES],
    hip: f32,
    /// The bone of each node of the kit, by node index.
    pub bone_of: Vec<u8>,
}

impl Bound {
    pub fn new(def: &RigDef, kit: &Kit, kits: &Kits) -> Bound {
        let mut rest = [None; BONES];
        for &(b, p) in def.joints {
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
        for &(p, b) in def.grips {
            segs.push((Vec3::from(p), Vec3::from(p), keep[b as usize]));
        }
        let mut bone_of = vec![Pelvis as u8; kit.nodes.len()];
        for (i, n) in kit.nodes.iter().enumerate() {
            let xf = kit.model_xf(i);
            let c = n.mesh.and_then(|m| kits.mesh_centre(m)).map(|c| xf.transform_point3(c)).unwrap_or(xf.w_axis.truncate());
            let near = segs.iter().min_by(|x, y| seg_dist(c, x.0, x.1).total_cmp(&seg_dist(c, y.0, y.1)));
            if let Some(s) = near {
                bone_of[i] = s.2 as u8;
            }
        }
        let hip = rest[Pelvis as usize].map(|p| p.y).unwrap_or(1.0).max(0.1);
        Bound { rest, keep, hip, bone_of }
    }

    /// Each bone's transform in model space for the blended `layers`
    /// (clip, time, looped, weight).
    pub fn pose(&self, lib: &Library, layers: &[(usize, f32, bool, f32)]) -> [Mat4; BONES] {
        let mut rot = [Quat::IDENTITY; BONES];
        let mut root = Vec3::ZERO;
        let total: f32 = layers.iter().map(|l| l.3).sum();
        if total > 1e-4 {
            let mut acc = [glam::Vec4::ZERO; BONES];
            for &(c, t, looped, w) in layers {
                if w <= 0.0 {
                    continue;
                }
                let (q, r) = lib.get(c).sample(t, looped, lib.fps);
                let w = w / total;
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

/// The rig of each humanoid monster kit, from its Godot builder
/// (src/actors/enemy.gd). Others keep their hand-made motion.
pub fn rig(kit: &str) -> Option<&'static RigDef> {
    Some(match kit {
        "skeleton" | "gravekeeper" | "practice_archer" => &SKELETON,
        "bone_archer" => &BONE_ARCHER,
        "ghoul" => &GHOUL,
        "cultist" | "practice_caster" => &CULTIST,
        "necromancer" => &NECROMANCER,
        "drowned" | "drowned_king" => &DROWNED,
        "shieldbearer" => &SHIELDBEARER,
        "brute" | "frost_brute" | "warden" => &BRUTE,
        "demon" | "pit_lord" => &DEMON,
        "imp" => &IMP,
        _ => return None,
    })
}

macro_rules! legs {
    ($hx:expr, $hy:expr, $kx:expr, $ky:expr, $kz:expr, $ax:expr, $ay:expr, $az:expr) => {
        [(ThighL, [$hx, $hy, -0.02]), (CalfL, [$kx, $ky, $kz]), (FootL, [$ax, $ay, $az]), (ThighR, [-$hx, $hy, -0.02]), (CalfR, [-$kx, $ky, $kz]), (FootR, [-$ax, $ay, $az])]
    };
}

const SK_LEGS: [(Bone, [f32; 3]); 6] = legs!(0.11, 0.92, 0.13, 0.5, 0.08, 0.12, 0.09, -0.03);

const SKELETON: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.93, -0.01]),
        (Spine2, [0.0, 1.1, -0.02]),
        (Spine3, [0.0, 1.3, 0.0]),
        (Head, [0.0, 1.5, 0.06]),
        (UpperArmL, [0.2, 1.43, -0.01]),
        (LowerArmL, [0.26, 1.17, 0.03]),
        (HandL, [0.27, 0.96, 0.16]),
        (UpperArmR, [-0.2, 1.43, -0.01]),
        (LowerArmR, [-0.26, 1.17, 0.03]),
        (HandR, [-0.24, 0.97, 0.21]),
        SK_LEGS[0],
        SK_LEGS[1],
        SK_LEGS[2],
        SK_LEGS[3],
        SK_LEGS[4],
        SK_LEGS[5],
    ],
    // The blade in its right hand; the gravekeeper's shovel.
    grips: &[([-0.24, 1.08, 0.53], HandR), ([-0.22, 1.0, 0.22], HandR), ([-0.26, 0.38, 0.46], HandR)],
};

const BONE_ARCHER: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.93, -0.01]),
        (Spine2, [0.0, 1.1, -0.02]),
        (Spine3, [0.0, 1.3, 0.0]),
        (Head, [0.0, 1.5, 0.06]),
        (UpperArmL, [0.2, 1.43, -0.01]),
        (LowerArmL, [0.26, 1.17, 0.03]),
        (HandL, [0.27, 0.96, 0.16]),
        (UpperArmR, [-0.2, 1.43, -0.01]),
        (LowerArmR, [-0.27, 1.25, 0.12]),
        (HandR, [-0.2, 1.25, 0.4]),
        SK_LEGS[0],
        SK_LEGS[1],
        SK_LEGS[2],
        SK_LEGS[3],
        SK_LEGS[4],
        SK_LEGS[5],
    ],
    // The bow and its string in the raised hand; the quiver on its back.
    grips: &[([-0.2, 1.22, 0.45], HandR), ([-0.2, 1.22, 0.38], HandR), ([0.0, 1.3, -0.15], Spine3)],
};

/// The clips the monsters use, looked up once.
pub struct Clips {
    idle: usize,
    walk: usize,
    jog: usize,
    death: usize,
    hit: usize,
    sword: usize,
    punch: usize,
    raise: usize,
    cast: usize,
}

impl Clips {
    pub fn new(lib: &Library) -> Option<Clips> {
        Some(Clips {
            idle: lib.clip("Idle_Loop")?,
            walk: lib.clip("Walk_Loop")?,
            jog: lib.clip("Jog_Fwd_Loop")?,
            death: lib.clip("Death01")?,
            hit: lib.clip("Hit_Chest")?,
            sword: lib.clip("Sword_Attack")?,
            punch: lib.clip("Punch_Cross")?,
            raise: lib.clip("Spell_Simple_Enter")?,
            cast: lib.clip("Spell_Simple_Shoot")?,
        })
    }
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

/// A monster's body animation state, frame to frame.
#[derive(Clone, Debug, Default)]
pub struct Body {
    phase: f32,
    idle_t: f32,
    moving: f32,
    jog: f32,
    blow: Option<Shot>,
    flinch: Option<Shot>,
    death: Option<Shot>,
    /// The attack winding up, until it lets go.
    attack: Option<Attack>,
}

/// What the sim says the monster is doing, for its body.
pub struct Doing {
    /// Ground speed (m/s).
    pub speed: f32,
    /// Winding up: which clip and how far in (0 to 1).
    pub wind: Option<(Attack, f32)>,
    /// Just got hit.
    pub hit: bool,
    pub dead: bool,
    /// Frozen or stunned: the body holds still.
    pub held: bool,
}

use spellstorm::enemy::Attack;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn toward(v: &mut f32, to: f32, rate: f32, dt: f32) {
    *v += (to - *v) * (rate * dt).min(1.0);
}

impl Body {
    /// An attack's wind-up clip, how far into it the wind-up goes, and the
    /// clip and spot the release plays on from.
    fn clips_for(lib: &Library, c: &Clips, attack: Attack) -> (usize, f32, (usize, f32)) {
        match attack {
            // A caster draws its hand up, then throws.
            Attack::Ranged | Attack::Lob => (c.raise, lib.get(c.raise).length, (c.cast, 0.0)),
            Attack::Dash => (c.punch, lib.get(c.punch).hit, (c.punch, lib.get(c.punch).hit)),
            Attack::Melee | Attack::Burrow | Attack::Slam => (c.sword, lib.get(c.sword).hit, (c.sword, lib.get(c.sword).hit)),
        }
    }
    /// The layers to blend this frame (clip, time, looped, weight).
    pub fn step(&mut self, lib: &Library, c: &Clips, hip: f32, d: &Doing, dt: f32) -> Vec<(usize, f32, bool, f32)> {
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
        let mut out = vec![
            (c.idle, self.idle_t, true, 1.0 - self.moving),
            (c.walk, self.phase * lib.get(c.walk).length, true, self.moving * (1.0 - self.jog)),
            (c.jog, self.phase * lib.get(c.jog).length, true, self.moving * self.jog),
        ];
        // The blow: held to the wind-up so it lands as the attack does, then
        // played out.
        if let Some((attack, k)) = d.wind {
            // What the wind-up plays (to where in it), and what the release.
            let (wind, to, _) = Body::clips_for(lib, c, attack);
            let b = self.blow.get_or_insert(Shot { clip: wind, t: 0.0, w: 0.0, full: 1.0 });
            b.clip = wind;
            b.t = k * to;
            toward(&mut b.w, 1.0, 12.0, dt);
            self.attack = Some(attack);
        } else if let Some(b) = &mut self.blow {
            if let Some(attack) = self.attack.take() {
                // Let go: the release clip from where the blow lands.
                let (_, _, (clip, from)) = Body::clips_for(lib, c, attack);
                b.clip = clip;
                b.t = from;
            }
            b.t += dt;
            let left = lib.get(b.clip).length - b.t;
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
            let len = lib.get(f.clip).length;
            f.w = f.full * (1.0 - (f.t / len)).max(0.0);
            if f.t >= len {
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
                l.3 *= keep;
            }
            out.push((s.clip, s.t, false, s.w));
        }
        out
    }
}

const GHOUL: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.8, -0.08]),
        (Spine2, [0.0, 0.95, 0.0]),
        (Spine3, [0.0, 1.1, 0.12]),
        (Head, [0.0, 1.15, 0.32]),
        (UpperArmL, [0.22, 1.15, 0.12]),
        (LowerArmL, [0.33, 0.8, 0.2]),
        (HandL, [0.32, 0.42, 0.36]),
        (UpperArmR, [-0.22, 1.15, 0.12]),
        (LowerArmR, [-0.33, 0.8, 0.2]),
        (HandR, [-0.32, 0.42, 0.36]),
        (ThighL, [0.15, 0.8, -0.05]),
        (CalfL, [0.18, 0.45, 0.0]),
        (FootL, [0.18, 0.08, 0.0]),
        (ThighR, [-0.15, 0.8, -0.05]),
        (CalfR, [-0.18, 0.45, 0.0]),
        (FootR, [-0.18, 0.08, 0.0]),
    ],
    // Its loincloth.
    grips: &[([0.0, 0.64, -0.12], Pelvis)],
};

/// Robed: no legs to move, the robe goes with the hips.
const CULTIST: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.9, 0.0]),
        (Spine3, [0.0, 1.25, 0.0]),
        (Head, [0.0, 1.45, 0.05]),
        (UpperArmL, [0.2, 1.38, 0.0]),
        (HandL, [0.23, 1.11, 0.5]),
        (UpperArmR, [-0.2, 1.38, 0.0]),
        (HandR, [-0.38, 0.98, 0.24]),
    ],
    // The staff and its head; the robe.
    grips: &[([-0.4, 0.87, 0.24], HandR), ([-0.4, 1.78, 0.24], HandR), ([0.0, 0.74, -0.04], Pelvis)],
};

const NECROMANCER: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 1.0, 0.0]),
        (Spine3, [0.0, 1.4, 0.0]),
        (Head, [0.0, 1.62, 0.05]),
        (UpperArmL, [0.22, 1.5, 0.0]),
        (HandL, [0.3, 1.24, 0.48]),
        (UpperArmR, [-0.22, 1.5, 0.0]),
        (HandR, [-0.44, 1.11, 0.18]),
    ],
    grips: &[([-0.42, 1.06, 0.18], HandR), ([-0.42, 2.03, 0.24], HandR), ([0.0, 0.82, -0.1], Pelvis), ([0.0, 0.8, -0.02], Pelvis)],
};

/// One piece per arm and per leg.
const DROWNED: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.85, 0.0]),
        (Spine3, [0.0, 1.2, 0.05]),
        (Head, [0.0, 1.3, 0.28]),
        (UpperArmL, [0.28, 1.25, 0.1]),
        (HandL, [0.3, 0.62, 0.35]),
        (UpperArmR, [-0.28, 1.25, 0.1]),
        (HandR, [-0.3, 0.62, 0.35]),
        (ThighL, [0.12, 0.85, 0.0]),
        (FootL, [0.12, 0.08, 0.02]),
        (ThighR, [-0.12, 0.85, 0.0]),
        (FootR, [-0.12, 0.08, 0.02]),
    ],
    grips: &[([0.0, 0.94, 0.05], Pelvis)],
};

const SHIELDBEARER: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.75, 0.0]),
        (Spine3, [0.0, 1.25, 0.0]),
        (Head, [0.0, 1.55, 0.05]),
        (UpperArmL, [0.3, 1.4, 0.0]),
        (LowerArmL, [0.33, 1.15, 0.15]),
        (HandL, [0.23, 1.0, 0.3]),
        (UpperArmR, [-0.3, 1.4, 0.0]),
        (LowerArmR, [-0.38, 1.1, 0.08]),
        (HandR, [-0.42, 0.9, 0.15]),
        (ThighL, [0.13, 0.72, 0.0]),
        (FootL, [0.13, 0.08, 0.05]),
        (ThighR, [-0.13, 0.72, 0.0]),
        (FootR, [-0.13, 0.08, 0.05]),
    ],
    // The shield on its left arm, the mace in its right; skirt and tabard.
    grips: &[([0.12, 0.96, 0.4], HandL), ([0.12, 0.96, 0.42], HandL), ([-0.42, 0.7, 0.25], HandR), ([0.0, 0.72, 0.0], Pelvis), ([0.0, 0.72, 0.28], Pelvis)],
};

const BRUTE: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 1.1, 0.0]),
        (Spine3, [0.0, 1.7, 0.05]),
        (Head, [0.0, 1.85, 0.5]),
        (UpperArmL, [0.6, 1.85, 0.05]),
        (LowerArmL, [0.8, 1.35, 0.1]),
        (HandL, [0.82, 0.8, 0.3]),
        (UpperArmR, [-0.6, 1.85, 0.05]),
        (LowerArmR, [-0.8, 1.35, 0.1]),
        (HandR, [-0.82, 0.8, 0.3]),
        (ThighL, [0.27, 1.1, 0.0]),
        (CalfL, [0.33, 0.62, 0.05]),
        (FootL, [0.35, 0.08, 0.1]),
        (ThighR, [-0.27, 1.1, 0.0]),
        (CalfR, [-0.33, 0.62, 0.05]),
        (FootR, [-0.35, 0.08, 0.1]),
    ],
    // Its belt; the club across its back; the warden's ice on its shoulder.
    grips: &[([0.0, 0.79, 0.04], Pelvis), ([0.17, 1.62, 0.1], Spine3), ([-0.28, 2.03, -0.28], Spine3)],
};

const DEMON: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 1.05, 0.0]),
        (Spine3, [0.0, 1.6, 0.05]),
        (Head, [0.0, 1.75, 0.3]),
        (UpperArmL, [0.42, 1.7, 0.05]),
        (HandL, [0.55, 0.9, 0.2]),
        (UpperArmR, [-0.42, 1.7, 0.05]),
        (HandR, [-0.55, 0.9, 0.2]),
        (ThighL, [0.16, 1.05, 0.0]),
        (FootL, [0.16, 0.08, 0.05]),
        (ThighR, [-0.16, 1.05, 0.0]),
        (FootR, [-0.16, 0.08, 0.05]),
    ],
    // Wings and the spine ridge on its back, the tail at its hips.
    grips: &[([0.68, 1.65, -0.45], Spine3), ([-0.68, 1.65, -0.45], Spine3), ([0.01, 1.43, -0.25], Spine3), ([0.27, 0.62, -0.43], Pelvis)],
};

const IMP: RigDef = RigDef {
    joints: &[
        (Pelvis, [0.0, 0.55, 0.0]),
        (Spine3, [0.0, 0.85, 0.05]),
        (Head, [0.0, 0.95, 0.15]),
        (UpperArmL, [0.15, 0.85, 0.05]),
        (HandL, [0.2, 0.55, 0.25]),
        (UpperArmR, [-0.15, 0.85, 0.05]),
        (HandR, [-0.2, 0.55, 0.25]),
        (ThighL, [0.08, 0.55, 0.0]),
        (FootL, [0.08, 0.05, 0.0]),
        (ThighR, [-0.08, 0.55, 0.0]),
        (FootR, [-0.08, 0.05, 0.0]),
    ],
    grips: &[([0.34, 0.9, -0.2], Spine3), ([-0.34, 0.9, -0.2], Spine3), ([0.13, 0.43, -0.3], Pelvis)],
};

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(kit: &str) -> (Library, Clips, Bound, Kits) {
        let lib = Library::load().unwrap();
        let clips = Clips::new(&lib).unwrap();
        let kits = Kits::load(None).unwrap();
        let b = Bound::new(rig(kit).unwrap(), kits.kit(&format!("enemy/{kit}")).unwrap(), &kits);
        (lib, clips, b, kits)
    }

    /// Where a kept bone's joint goes under one clip at `t`.
    fn joint(b: &Bound, lib: &Library, clip: usize, t: f32, bone: Bone) -> Vec3 {
        let m = b.pose(lib, &[(clip, t, true, 1.0)]);
        m[bone as usize].transform_point3(b.rest[bone as usize].unwrap())
    }

    #[test]
    fn skeleton_stands_and_walks_on_its_feet() {
        let (lib, c, b, _) = setup("skeleton");
        for clip in [c.idle, c.walk, c.jog] {
            for f in 0..20 {
                let t = f as f32 / 20.0 * lib.get(clip).length;
                let head = joint(&b, &lib, clip, t, Head);
                let pelvis = joint(&b, &lib, clip, t, Pelvis);
                assert!(head.y > pelvis.y + 0.4, "head under the hips at {t}: {head} {pelvis}");
                for foot in [FootL, FootR] {
                    let p = joint(&b, &lib, clip, t, foot);
                    assert!(p.y > -0.15 && p.y < 0.75, "foot {foot:?} at {p} in clip {clip} {t}");
                }
            }
        }
        // The idle stands nearly as modelled.
        let head = joint(&b, &lib, c.idle, 0.0, Head);
        assert!(head.distance(Vec3::new(0.0, 1.5, 0.06)) < 0.15, "{head}");
    }

    #[test]
    fn every_rigged_kit_stands_up() {
        let lib = Library::load().unwrap();
        let c = Clips::new(&lib).unwrap();
        let kits = Kits::load(None).unwrap();
        for k in spellstorm::enemy::KINDS.iter().filter(|k| rig(k.key).is_some()) {
            let kit = kits.kit(&format!("enemy/{}", k.key)).unwrap_or_else(|| panic!("no kit {}", k.key));
            let b = Bound::new(rig(k.key).unwrap(), kit, &kits);
            let m = b.pose(&lib, &[(c.idle, 0.0, true, 1.0)]);
            let head = m[Head as usize].transform_point3(b.rest[Head as usize].unwrap());
            assert!(head.y > b.hip * 1.2, "{}: head at {head}", k.key);
        }
    }

    #[test]
    fn pieces_hang_on_their_bones() {
        let (_, _, b, kits) = setup("skeleton");
        let kit = kits.kit("enemy/skeleton").unwrap();
        let on = |p: Vec3| {
            let i = (0..kit.nodes.len())
                .filter(|&i| kit.nodes[i].mesh.is_some())
                .min_by(|&x, &y| {
                    let c = |i: usize| kit.model_xf(i).transform_point3(kits.mesh_centre(kit.nodes[i].mesh.unwrap()).unwrap());
                    c(x).distance(p).total_cmp(&c(y).distance(p))
                })
                .unwrap();
            b.bone_of[i]
        };
        assert_eq!(on(Vec3::new(-0.24, 1.08, 0.53)), HandR as u8, "the sword");
        assert_eq!(on(Vec3::new(0.0, 1.6, 0.09)), Head as u8, "the skull");
        assert_eq!(on(Vec3::new(0.12, 0.27, 0.02)), CalfL as u8, "a shin");
        assert_eq!(on(Vec3::new(0.23, 1.3, 0.01)), UpperArmL as u8, "an upper arm");
    }

    /// RIG_DUMP=file.json: each piece's corners through some clips, to look at.
    #[test]
    fn dump_poses() {
        let Ok(out) = std::env::var("RIG_DUMP") else {
            return;
        };
        let kit_name = std::env::var("RIG_KIT").unwrap_or("skeleton".into());
        let (lib, _, b, kits) = setup(&kit_name);
        let kit = kits.kit(&format!("enemy/{kit_name}")).unwrap();
        let mut clips = serde_json::Map::new();
        for name in ["Idle_Loop", "Walk_Loop", "Jog_Fwd_Loop", "Sword_Attack", "Spell_Simple_Shoot", "Death01", "Hit_Chest", "Punch_Cross", "Jump_Start"] {
            let c = lib.clip(name).unwrap();
            let mut frames = Vec::new();
            for f in 0..8 {
                let t = f as f32 / 8.0 * lib.get(c).length;
                let m = b.pose(&lib, &[(c, t, false, 1.0)]);
                let mut pieces = Vec::new();
                for (i, n) in kit.nodes.iter().enumerate() {
                    let Some(mesh) = n.mesh else { continue };
                    let xf = m[b.bone_of[i] as usize] * kit.model_xf(i);
                    let mut pts = Vec::new();
                    for s in &kits.file.meshes[mesh].surfaces {
                        for (k, p) in kits.f32s(s.positions).chunks_exact(3).enumerate() {
                            if k % 3 == 0 {
                                let q = xf.transform_point3(Vec3::from_slice(p));
                                pts.push(serde_json::json!([q.x, q.y, q.z]));
                            }
                        }
                    }
                    pieces.push(serde_json::json!({"bone": b.bone_of[i], "pts": pts}));
                }
                frames.push(serde_json::Value::Array(pieces));
            }
            clips.insert(name.into(), serde_json::Value::Array(frames));
        }
        std::fs::write(out, serde_json::Value::Object(clips).to_string()).unwrap();
    }
}
