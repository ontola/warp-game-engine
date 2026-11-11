//! Spellstorm's monsters on the engine's `body` crate: which joints each
//! model has, and which clips its attacks play.
use crate::kits::{Kit, Kits};
use body::Bone::*;
use body::{Action, Bone, Bound, ClipSet, Library, Rig};
use spellstorm::enemy::Attack;

/// Hang a kit's nodes on `rig`: `bone_of` goes by node index.
pub fn bind(rig: &Rig, kit: &Kit, kits: &Kits) -> Bound {
    let pieces: Vec<_> = kit
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let xf = kit.model_xf(i);
            n.mesh.and_then(|m| kits.mesh_centre(m)).map(|c| xf.transform_point3(c)).unwrap_or(xf.w_axis.truncate())
        })
        .collect();
    Bound::new(rig, &pieces)
}

/// The clips the monsters use, looked up once.
pub struct Clips {
    pub set: ClipSet,
    sword: usize,
    punch: usize,
    raise: usize,
    cast: usize,
}

impl Clips {
    pub fn new(lib: &Library) -> Option<Clips> {
        Some(Clips {
            set: ClipSet {
                idle: lib.clip("Idle_Loop")?,
                walk: lib.clip("Walk_Loop")?,
                jog: lib.clip("Jog_Fwd_Loop")?,
                hit: lib.clip("Hit_Chest")?,
                death: lib.clip("Death01")?,
                lengths: lib.lengths(),
            },
            sword: lib.clip("Sword_Attack")?,
            punch: lib.clip("Punch_Cross")?,
            raise: lib.clip("Spell_Simple_Enter")?,
            cast: lib.clip("Spell_Simple_Shoot")?,
        })
    }

    /// What an attack plays.
    pub fn action(&self, lib: &Library, attack: Attack) -> Action {
        let blow = |clip: usize| Action { wind: clip, wind_to: lib.get(clip).hit, release: clip, release_from: lib.get(clip).hit };
        match attack {
            // A caster draws its hand up, then throws.
            Attack::Ranged | Attack::Lob => Action { wind: self.raise, wind_to: lib.get(self.raise).length, release: self.cast, release_from: 0.0 },
            Attack::Dash => blow(self.punch),
            Attack::Melee | Attack::Burrow | Attack::Slam => blow(self.sword),
        }
    }
}

/// The rig of each humanoid monster kit, from its Godot builder
/// (src/actors/enemy.gd). Others keep their hand-made motion.
pub fn rig(kit: &str) -> Option<&'static Rig> {
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

const SKELETON: Rig = Rig {
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

const BONE_ARCHER: Rig = Rig {
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

const GHOUL: Rig = Rig {
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
const CULTIST: Rig = Rig {
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

const NECROMANCER: Rig = Rig {
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
const DROWNED: Rig = Rig {
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

const SHIELDBEARER: Rig = Rig {
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

const BRUTE: Rig = Rig {
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

const DEMON: Rig = Rig {
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

const IMP: Rig = Rig {
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
    use body::Layer;
    use glam::Vec3;

    fn setup(kit: &str) -> (Library, Clips, Bound, Kits) {
        let lib = Library::builtin().unwrap();
        let clips = Clips::new(&lib).unwrap();
        let kits = Kits::load(None).unwrap();
        let b = bind(rig(kit).unwrap(), kits.kit(&format!("enemy/{kit}")).unwrap(), &kits);
        (lib, clips, b, kits)
    }

    /// Where a kept bone's joint goes under one clip at `t`.
    fn joint(b: &Bound, lib: &Library, clip: usize, t: f32, bone: Bone) -> Vec3 {
        let m = b.pose(lib, &[Layer { clip, t, looped: true, w: 1.0 }]);
        m[bone as usize].transform_point3(b.rest[bone as usize].unwrap())
    }

    #[test]
    fn skeleton_stands_and_walks_on_its_feet() {
        let (lib, c, b, _) = setup("skeleton");
        for clip in [c.set.idle, c.set.walk, c.set.jog] {
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
        let head = joint(&b, &lib, c.set.idle, 0.0, Head);
        assert!(head.distance(Vec3::new(0.0, 1.5, 0.06)) < 0.15, "{head}");
    }

    #[test]
    fn every_rigged_kit_stands_up() {
        let lib = Library::builtin().unwrap();
        let c = Clips::new(&lib).unwrap();
        let kits = Kits::load(None).unwrap();
        for k in spellstorm::enemy::kinds().iter().filter(|k| rig(k.key).is_some()) {
            let kit = kits.kit(&format!("enemy/{}", k.key)).unwrap_or_else(|| panic!("no kit {}", k.key));
            let b = bind(rig(k.key).unwrap(), kit, &kits);
            let m = b.pose(&lib, &[Layer { clip: c.set.idle, t: 0.0, looped: true, w: 1.0 }]);
            let head = m[Head as usize].transform_point3(b.rest[Head as usize].unwrap());
            assert!(head.y > b.hip() * 1.2, "{}: head at {head}", k.key);
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
                let m = b.pose(&lib, &[Layer { clip: c, t, looped: false, w: 1.0 }]);
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
