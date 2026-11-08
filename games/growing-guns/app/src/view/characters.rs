//! The original roster and locomotion clips. Simulation still uses its capsule;
//! only small skin palettes are uploaded, once per actor rather than per view.
use assets::skinned::{ActorHandle, Character};
use glam::{Affine3A, Mat4, Quat, Vec3};
use growing_guns::{Event, World, DT, WALK_SPEED};
use render::{Gpu, Renderer, Scene};
use std::path::PathBuf;
use super::character_ragdoll::Corpse;
use physics::Physics;

const ROSTER: [&str; 3] = ["knight", "paladin", "ch10"];
const IDLE: &str = "pistol_idle";
const RUN: &str = "pistol_run";
const JUMP: &str = "pistol_jump";

#[derive(Default)]
struct Motion { idle: f32, run: f32, blend: f32, jump: Option<f32> }
struct KnightHold {
    editor: body::pose_editor::PoseEditor,
    authored: crate::foregrip_pose::ForegripPose,
    mount: body::reach::JointMount,
}
impl KnightHold {
    fn bind(model: &Character, reference: &[Mat4]) -> Result<Option<Self>, String> {
        if model.name != "knight" { return Ok(None); }
        let editor = body::pose_editor::PoseEditor::bind(model.joints(), crate::foregrip_pose::parent)?;
        let authored = crate::foregrip_pose::ForegripPose::bind(model, &editor)?;
        let (i, (_, bind)) = model.joints().enumerate().find(|(_, (n, _))| n.ends_with("RightHand")).ok_or("missing right hand")?;
        Ok(Some(Self { editor, authored, mount: body::reach::JointMount::new(i, bind, reference) }))
    }
}
struct Actor { knight: Option<KnightHold>, arms: Option<body::reach::TwoHandHold>, handle: ActorHandle, palette: Vec<Mat4>, run_palette: Vec<Mat4>, motion: Motion,
    corpse: Option<Corpse>, corpse_transform: Option<Affine3A>, velocity: Vec3 }

/// A player's body turned to lava (Godot `_apply_lava_body`): the same
/// character drawn in the lava material, posed like the corpse.
struct LavaTwin { handle: ActorHandle, palette: Vec<Mat4> }

/// Merge distance for the rigid pieces, metres.
const RIGID_CELL: f32 = 0.035;

pub struct Characters {
    animation: crate::hand_animation::HandAnimation,
    models: Vec<Character>,
    actors: Vec<Actor>,
    /// Each actor's `Scene::rigid_models` index, when drawn as rigid pieces.
    pieces: Vec<usize>,
    /// Each model's index there, built once.
    rigid_meshes: Vec<Option<usize>>,
    /// This frame's characters for the renderer.
    rigid_frame: Vec<render::scene::RigidActor>,
    lava: Vec<Option<LavaTwin>>,
    pub hands: Vec<Option<Affine3A>>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn knight_extended_pose_reproduces_saved_arm_positions() {
        use crate::gun::{Rig, Stats};
        let model = Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/characters/knight.json")).unwrap();
        let mut rig = Rig::default();
        let mut world = growing_guns::World::new(3, &[false]);
        let _scripts = crate::scripts::attach(&mut world).unwrap();
        let id = world.cards.find("sniper").unwrap();
        world.give(0, id);
        rig.set(Stats::of(&world.players[0]));
        let c = rig.contacts_at(1.0);
        assert!((c.extension - 1.0).abs() < 1e-5);
        let t = 2312.0 / 60.0;
        let mut pose = model.sample(IDLE, t, true).unwrap();
        let mut saved = KnightHold::bind(&model, &model.sample(IDLE, 0.0, true).unwrap()).unwrap().unwrap();
        let arms = grip_arms(&model).unwrap();
        let hand = model.sample_socket(IDLE, "RightHand", t, true).unwrap().w_axis.truncate();
        let (support, roll) = crate::hand_animation::HandAnimation::load().unwrap().pose(c, t);
        let aim = model_transform(Vec3::ZERO, 0.0).inverse().to_scale_rotation_translation().1;
        let xf = saved.mount.place(&pose, aim, c.primary);
        arms.apply(&mut pose, hand, hand + xf.transform_vector3(support - c.primary));
        arms.orient_support(&mut pose, xf.to_scale_rotation_translation().1 * aim.inverse() * grip_rotation(0.0) * Quat::from_rotation_x(roll));
        saved.authored.set_extension(&mut saved.editor, 1.0);
        saved.editor.apply(&mut pose);
        for (name, expected) in [
            ("RightArm", [-0.28063533, 1.47436571, 0.06693695]),
            ("RightForeArm", [-0.21969306, 1.44296432, 0.21131268]),
            ("RightHand", [-0.12265244, 1.42794120, 0.39420366]),
            ("LeftArm", [0.22642264, 1.42708194, 0.23549058]),
            ("LeftForeArm", [0.08812520, 1.39920485, 0.53796691]),
            ("LeftHand", [0.00424939, 1.40211701, 0.68414390])
        ] {
            let actual = model.joint_position(name, &pose).unwrap();
            assert!(actual.distance(Vec3::from_array(expected)) < 0.001, "{name}: {actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn support_wrist_reaches_magazine_and_sniper_contacts() {
        use crate::gun::{Rig, Stats};
        use super::super::guns::Guns;
        for name in ROSTER {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/characters").join(format!("{name}.json"));
            let model = Character::load(&path).unwrap();
            let arms = grip_arms(&model).unwrap();
            let wrist = model.joints().find(|(n, _)| n.ends_with("LeftHand")).unwrap().0;
            let hand = model.sample_socket(IDLE, "RightHand", 0.25, true).unwrap().w_axis.truncate();
            let body = model_transform(Vec3::ZERO, 0.0);
            for spread in [Stats::default().spread, 0.0] {
                let mut rig = Rig::default();
                rig.set(Stats { spread, ..Stats::default() });
                let contacts = rig.contacts_at(1.0);
                let grip = contacts.primary;
                let (support, _) = crate::hand_animation::HandAnimation::load().unwrap().pose(contacts, 0.0);
                let offset = body.inverse().transform_vector3(Guns::held_transform(Vec3::ZERO, 0.0, 0.0, grip).transform_vector3(support - grip));
                let mut palette = model.sample(IDLE, 0.25, true).unwrap();
                let upper_length = |pose: &[Mat4]| {
                    let pos = |suffix: &str| model.joints().find(|(n, _)| n.ends_with(suffix))
                        .and_then(|(n, _)| model.joint_position(n, pose)).unwrap();
                    pos("LeftArm").distance(pos("LeftForeArm"))
                };
                let original = upper_length(&palette);
                let primary = arms.apply(&mut palette, hand, hand + offset);
                let length = upper_length(&palette);
                eprintln!("{name} upper arm {original:.3} -> {length:.3} m");
                assert!((length - original * if name == "knight" { 1.24 } else { 1.0 }).abs() < 0.001);
                let error = model.joint_position(wrist, &palette).unwrap().distance(primary + offset);
                eprintln!("{name} spread {spread} support error {error:.4}m");
                assert!(error < 0.015, "{name} wrist missed contact by {error}m");
            }
        }
    }

    #[test]
    fn rigid_pieces_are_a_fraction_of_the_model_and_follow_real_joints() {
        for name in super::ROSTER {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/characters").join(format!("{name}.json"));
            let model = assets::skinned::Character::load(&path).unwrap();
            let parts = model.rigid_parts(super::RIGID_CELL);
            let triangles: usize = parts.iter().map(|p| p.indices.len() / 3).sum();
            println!("{name}: {} parts, {triangles} of {} triangles", parts.len(), model.triangles);
            assert!(triangles > 500 && triangles * 2 < model.triangles, "{name}: {triangles} of {}", model.triangles);
            assert!(parts.iter().all(|p| p.joint < model.joint_count && p.indices.iter().all(|i| (*i as usize) < p.positions.len())));
        }
    }

    use super::*;

    #[test]
    fn reused_enemy_and_round_reset_release_the_old_skin_rig() {
        let model = Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/characters/knight.json")).unwrap();
        let mut c = Characters { animation: crate::hand_animation::HandAnimation::load().unwrap(), rigid_meshes: vec![None], pieces: Vec::new(), rigid_frame: Vec::new(), models: vec![model], actors: Vec::new(), lava: Vec::new(), hands: Vec::new() };
        let mut scene = Scene::default();
        c.reserve_actor(&mut scene, 0).unwrap();
        let mut world = World::new(3, &[false, true, true]);
        let mut physics = Physics::new(Vec3::new(0.0, -20.0, 0.0), 60.0);
        world.players[0].alive = false;
        c.kill(&mut physics, 0, &world.players[0], Vec3::X, false).unwrap();
        physics.step(); c.follow_physics(&physics);
        c.pose(&world, &[], 1.0, None);
        assert!(c.corpse_transform(0).is_some());
        assert_eq!(physics.body_count(), 11);
        world.players[0].alive = true;
        c.tick(&world, &mut physics);
        c.pose(&world, &[], 1.0, None);
        assert_eq!(physics.body_count(), 0);
        assert!(c.corpse_transform(0).is_none() && c.hands[0].is_some());
        world.players[0].alive = false;
        c.kill(&mut physics, 0, &world.players[0], Vec3::X, true).unwrap();
        c.reset(&mut physics);
        assert_eq!(physics.body_count(), 0);
        assert_eq!(physics.world.impulse_joints.len(), 0);
        assert_eq!(scene.skinning.actors.len(), 1, "reusing a player never aliases or allocates another skin");
    }
}

pub(crate) fn grip_arms(model: &Character) -> Option<body::reach::TwoHandHold> {
    let arms = body::reach::TwoHandHold::bind(model.joints())?;
    Some(if model.name == "knight" { arms.with_arm_proportions(1.24, 1.20) } else { arms })
}

pub fn model_transform(pos: Vec3, yaw: f32) -> Affine3A {
    Affine3A::from_rotation_translation(
        Quat::from_rotation_y(yaw + std::f32::consts::PI), pos - Vec3::Y * 0.9)
}

// Roster bind pose: left fingers extend along +X, the back of the
// hand faces +Y. Turn the palm inward against the side of the attachment.
pub(crate) fn grip_rotation(pitch: f32) -> Quat {
    Quat::from_rotation_x(-pitch) * Quat::from_mat3(&glam::Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y))
}

/// Skin and hair lighting on the characters: opt-in (`GG_SKIN_HAIR=1`)
/// until it has been checked on a real GPU.
pub fn skin_hair_enabled() -> bool {
    std::env::var("GG_SKIN_HAIR").is_ok_and(|v| v == "1")
}

/// Which surfaces are skin or hair: the paladin's face, scalp and ears by
/// `paladin_skin.png` (tools/skin-masks.py), the zombie's bare body.
pub fn apply_skin_hair(model: &mut Character, dir: &std::path::Path) -> Result<(), String> {
    use assets::skinned::SkinHair;
    match model.name.as_str() {
        "paladin" => {
            let path = dir.join("paladin_skin.png");
            let mask = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?.into_luma8();
            // Both surfaces share the atlas: the mask leaves the armour plain.
            for surface in 0..2 { model.set_skin_hair(surface, SkinHair::Mask(mask.clone()))?; }
        }
        "ch10" => model.set_skin_hair(0, SkinHair::Skin)?,
        _ => {}
    }
    Ok(())
}

fn mix(a: Mat4, b: Mat4, t: f32) -> Mat4 {
    let a = a.to_cols_array(); let b = b.to_cols_array();
    Mat4::from_cols_array(&std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t))
}

impl Characters {
    pub fn load() -> Result<Self, String> {
        let dir = if let Some(path) = std::env::var_os("GG_CHARACTERS") {
            if path == "off" { return Err("GG_CHARACTERS=off: explicit primitive roster".into()); }
            PathBuf::from(path)
        } else {
            let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.join("assets/characters")));
            beside.filter(|p| p.join("knight.json").is_file())
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/characters"))
        };
        let mut models = ROSTER.iter().map(|name| Character::load(&dir.join(format!("{name}.json"))))
            .collect::<Result<Vec<_>, _>>()?;
        if skin_hair_enabled() {
            for model in &mut models { apply_skin_hair(model, &dir)?; }
        }
        // Fail before reserving scene slots if a required clip/socket is absent.
        for model in &models {
            for clip in [IDLE, RUN, JUMP] {
                model.sample(clip, 0.0, clip != JUMP)?;
                model.sample_socket(clip, "RightHand", 0.0, clip != JUMP)?;
            }
        }
        println!("characters: original knight, paladin, ch10 with idle/run/jump ({})", dir.display());
        Ok(Self { animation: crate::hand_animation::HandAnimation::load()?, rigid_meshes: models.iter().map(|_| None).collect(), pieces: Vec::new(), rigid_frame: Vec::new(), models, actors: Vec::new(), lava: Vec::new(), hands: Vec::new() })
    }

    pub fn models(&self) -> &[Character] { &self.models }

    pub fn reserve_actor(&mut self, scene: &mut Scene, player: usize) -> Result<usize, String> {
        let model = &self.models[player % self.models.len()];
        // Articulated poses extend outside the authored locomotion bounds.
        let handle = model.add_actor_with_bounds(scene, model_transform(Vec3::new(0.0, -500.0, 0.0), 0.0),
            model.min.min(Vec3::splat(-4.0)), model.max.max(Vec3::splat(4.0)))?;
        let palette = model.sample(IDLE, 0.0, true)?;
        let mesh = handle.mesh;
        let run_palette = vec![Mat4::IDENTITY; palette.len()];
        self.actors.push(Actor { knight: KnightHold::bind(model, &palette)?, arms: grip_arms(model), handle, palette, run_palette, motion: Motion::default(),
            corpse: None, corpse_transform: None, velocity: Vec3::ZERO });
        self.hands.push(None);
        Ok(mesh)
    }

    /// Player `player` as rigid pieces, one draw in the minimum renderer.
    pub fn reserve_rigid(&mut self, scene: &mut Scene, player: usize) -> Result<(), String> {
        use render::{cluster::Vertex, scene::RigidModel};
        let k = player % self.models.len();
        let model = &self.models[k];
        if self.rigid_meshes[k].is_none() {
            let mut whole = RigidModel::default();
            for part in model.rigid_parts(RIGID_CELL) {
                let material = scene.add_material(render::scene::Material { tint: part.tint, layer: -1, roughness: 0.7, flags: 0, scale: 1.0, metal: 0.0 });
                let base = whole.vertices.len() as u32;
                whole.vertices.extend(part.positions.iter().zip(&part.normals).map(|(p, n)| Vertex { pos: p.to_array(), normal: n.to_array(), uv: [part.joint as f32, material as f32] }));
                whole.indices.extend(part.indices.iter().map(|i| i + base));
            }
            scene.rigid_models.push(whole);
            self.rigid_meshes[k] = Some(scene.rigid_models.len() - 1);
        }
        let palette = model.sample(IDLE, 0.0, true)?;
        let run_palette = vec![Mat4::IDENTITY; palette.len()];
        let handle = ActorHandle { actor: 0, instance: 0, mesh: 0 };
        self.actors.push(Actor { knight: KnightHold::bind(model, &palette)?, arms: grip_arms(model), handle, palette, run_palette, motion: Motion::default(), corpse: None, corpse_transform: None, velocity: Vec3::ZERO });
        self.pieces.push(self.rigid_meshes[k].unwrap());
        self.hands.push(None);
        Ok(())
    }

    /// This frame's rigid characters, after `pose`: `hide` holds each player's
    /// views to hide them in (their own body).
    pub fn pose_rigid(&mut self, world: &World, prev: &[crate::view::Pose], alpha: f32, hide: &[u8]) {
        self.rigid_frame.clear();
        for (i, (model, actor)) in self.pieces.iter().zip(&self.actors).enumerate() {
            let body = world.players.get(i).and_then(|p| if p.alive && !p.ghost {
                let drawn = crate::view::pose(world, i, prev, alpha);
                Some(model_transform(drawn.pos, drawn.yaw))
            } else { actor.corpse_transform });
            let Some(transform) = body else { continue };
            self.rigid_frame.push(render::scene::RigidActor { model: *model, transform, palette: actor.palette.clone(), hidden_views: hide.get(i).copied().unwrap_or(0) });
        }
    }

    /// A lava twin per actor while the renderer has skinning room left
    /// (after every player has its own actor). Their instances start hidden.
    pub fn reserve_lava(&mut self, scene: &mut Scene, material: u32) {
        if !self.pieces.is_empty() { return; }
        self.lava = (0..self.actors.len()).map(|i| {
            let model = &self.models[i % self.models.len()];
            let handle = model.add_actor_with_material(scene, model_transform(Vec3::new(0.0, -500.0, 0.0), 0.0), Some(material)).ok()?;
            scene.instances[handle.instance].cluster_count=0;
            Some(LavaTwin { handle, palette: model.sample(IDLE, 0.0, true).ok()? })
        }).collect();
    }

    /// Turn player `i`'s corpse to lava: its twin takes the death pose.
    /// Returns the twin's instance and mesh (to place), None without room
    /// for one.
    pub fn lava_twin(&mut self, i: usize) -> Option<(usize, usize)> {
        let actor = self.actors.get(i)?;
        let twin = self.lava.get_mut(i)?.as_mut()?;
        // The pose of the current simulated animation (as `kill`), even
        // when no frame was drawn since the last tick.
        let model = &self.models[i % self.models.len()];
        let m = &actor.motion;
        if let Some(age) = m.jump {
            model.sample_into(JUMP, age, false, &mut twin.palette).ok()?;
        } else {
            model.sample_into(IDLE, m.idle, true, &mut twin.palette).ok()?;
            let run = model.sample(RUN, m.run, true).ok()?;
            for (idle, run) in twin.palette.iter_mut().zip(run) { *idle = mix(*idle, run, m.blend); }
        }
        self.hands[i] = None;
        Some((twin.handle.instance, twin.handle.mesh))
    }

    /// Instance and mesh of every lava twin (to hide).
    pub fn lava_instances(&self) -> Vec<(usize, usize)> { self.lava.iter().flatten().map(|t| (t.handle.instance, t.handle.mesh)).collect() }

    pub fn reset(&mut self, physics: &mut Physics) {
        for actor in &mut self.actors {
            if let Some(mut corpse) = actor.corpse.take() { corpse.rig.remove(physics); }
            actor.corpse_transform = None;
            actor.motion = Motion::default();
            actor.velocity = Vec3::ZERO;
        }
        self.hands.fill(None);
    }

    pub fn tick(&mut self, world: &World, physics: &mut Physics) {
        for (i, (actor, p)) in self.actors.iter_mut().zip(&world.players).enumerate() {
            if !p.alive { continue; }
            // A reused co-op enemy seat gets a fresh animated body, not a
            // second instance sharing its old corpse's deforming vertex range.
            if let Some(mut corpse) = actor.corpse.take() {
                corpse.rig.remove(physics);
                actor.corpse_transform = None;
                actor.motion = Motion::default();
            }
            actor.velocity = p.vel;
            let m = &mut actor.motion;
            let speed = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
            let local = Quat::from_rotation_y(-p.yaw) * p.vel;
            let direction = if local.z > speed * 0.2 { -1.0 } else { 1.0 };
            m.idle += DT;
            m.run += DT * (speed / WALK_SPEED).clamp(0.75, 1.5) * direction;
            let target = if p.on_floor { (speed / WALK_SPEED).clamp(0.0, 1.0) } else { 0.0 };
            m.blend += (target - m.blend) * (DT * 10.0).min(1.0);
            if world.events.iter().any(|e| matches!(e, Event::Jump { player } if *player == i)) {
                m.jump = Some(0.0);
            } else if p.on_floor {
                m.jump = None;
            } else if let Some(age) = &mut m.jump { *age += DT; }
        }
    }

    pub fn pose(&mut self, world: &World, prev: &[crate::view::Pose], alpha: f32, guns: Option<&super::guns::Guns>) {
        for (i, actor) in self.actors.iter_mut().enumerate() {
            let Some(p) = world.players.get(i) else {
                actor.corpse_transform = None;
                self.hands[i] = None;
                continue;
            };
            if !p.alive {
                actor.corpse_transform = actor.corpse.as_ref().map(|c| c.pose(alpha, &mut actor.palette));
                self.hands[i] = None;
                continue;
            }
            let model = &self.models[i % self.models.len()];
            let m = &actor.motion;
            let interpolate = (alpha.clamp(0.0, 1.0) - 1.0) * DT;
            let socket = if let Some(age) = m.jump {
                let time = (age + interpolate).max(0.0);
                model.sample_into(JUMP, time, false, &mut actor.palette).expect("validated jump clip");
                model.sample_socket(JUMP, "RightHand", time, false).expect("validated jump socket")
            } else {
                let idle_time = (m.idle + interpolate).max(0.0);
                model.sample_into(IDLE, idle_time, true, &mut actor.palette).expect("validated idle clip");
                model.sample_into(RUN, m.run, true, &mut actor.run_palette).expect("validated run clip");
                for (idle, run) in actor.palette.iter_mut().zip(&actor.run_palette) { *idle = mix(*idle, *run, m.blend); }
                mix(model.sample_socket(IDLE, "RightHand", idle_time, true).expect("validated idle socket"),
                    model.sample_socket(RUN, "RightHand", m.run, true).expect("validated run socket"), m.blend)
            };
            let drawn = crate::view::pose(world, i, prev, alpha);
            let body = model_transform(drawn.pos, drawn.yaw);
            let hand = body.transform_point3(socket.w_axis.truncate());
            self.hands[i] = Some(if let (Some(arms), Some(contacts)) = (&actor.arms, guns.and_then(|g| g.contacts(i, alpha))) {
                let (support, roll) = self.animation.pose(contacts, (world.tick as f32 + alpha) * DT);
                let base = super::guns::Guns::held_transform(hand, drawn.yaw, drawn.pitch, contacts.primary);
                let aim = (body.inverse() * base).to_scale_rotation_translation().1;
                let sampled = actor.knight.as_ref().map(|k| k.mount.place(&actor.palette, aim, contacts.primary));
                let target = sampled.map_or(base.transform_point3(support), |xf| hand + body.transform_vector3(xf.transform_vector3(support - contacts.primary)));
                let wrist = arms.apply(&mut actor.palette, socket.w_axis.truncate(), body.inverse().transform_point3(target));
                arms.orient_support(&mut actor.palette, sampled.map_or(Quat::IDENTITY, |xf| xf.to_scale_rotation_translation().1 * aim.inverse()) * grip_rotation(drawn.pitch) * Quat::from_rotation_x(roll));
                if let Some(k) = &mut actor.knight {
                    k.authored.set_extension(&mut k.editor, contacts.extension);
                    k.editor.apply(&mut actor.palette);
                    body * k.mount.place(&actor.palette, aim, contacts.primary)
                } else { super::guns::Guns::held_transform(body.transform_point3(wrist), drawn.yaw, drawn.pitch, contacts.primary) }
            } else { super::guns::Guns::held_transform(hand, drawn.yaw, drawn.pitch, guns.and_then(|g| g.contacts(i, alpha)).map_or(Vec3::ZERO, |c| c.primary)) });
        }
    }

    pub fn kill(&mut self, physics: &mut Physics, player: usize, p: &growing_guns::Player, direction: Vec3, headless: bool) -> Result<(), String> {
        let actor = self.actors.get_mut(player).ok_or("missing skinned actor")?;
        if let Some(mut corpse) = actor.corpse.take() { corpse.rig.remove(physics); }
        let model = &self.models[player % self.models.len()];
        // Use the current simulated animation even if no frame was drawn since
        // the last tick (headless runs and multiple simulation ticks per draw).
        if let Some(age) = actor.motion.jump {
            model.sample_into(JUMP, age, false, &mut actor.palette)?;
        } else {
            model.sample_into(IDLE, actor.motion.idle, true, &mut actor.palette)?;
            model.sample_into(RUN, actor.motion.run, true, &mut actor.run_palette)?;
            for (idle, run) in actor.palette.iter_mut().zip(&actor.run_palette) { *idle = mix(*idle, *run, actor.motion.blend); }
        }
        actor.corpse = Some(Corpse::spawn(model, &actor.palette, model_transform(p.pos, p.yaw), actor.velocity, direction, player, headless, physics)?);
        self.hands[player] = None;
        Ok(())
    }

    pub fn follow_physics(&mut self, physics: &Physics) {
        for actor in &mut self.actors { if let Some(c) = &mut actor.corpse { c.rig.update(physics); } }
    }

    pub fn corpse_transform(&self, player: usize) -> Option<Affine3A> {
        self.actors.get(player).and_then(|a| a.corpse_transform)
    }

    pub fn corpse_bodies(&self, bodies: &mut Vec<physics::Body>) {
        for actor in &self.actors {
            if let Some(c) = &actor.corpse {
                bodies.extend((0..c.rig.len()).map(|i| c.rig.body(i)));
            }
        }
    }

    pub fn ragdolls(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.actors.iter().filter_map(|a| a.corpse.as_ref()).map(|c| c.rig.root_position(1.0).to_array())
    }

    pub fn flush(&self, gpu: &Gpu, renderer: &Renderer) -> Result<(), String> {
        if !self.pieces.is_empty() { renderer.set_rigid_actors(gpu, &self.rigid_frame); return Ok(()); }
        // Actors are reserved consecutively into the scene's skinning table:
        // the players', then their lava twins.
        debug_assert!(self.actors.iter().map(|a| a.handle.actor).chain(self.lava.iter().flatten().map(|t| t.handle.actor)).enumerate().all(|(i, actor)| actor == i));
        renderer.set_skinning_palettes(gpu, self.actors.iter().map(|actor| actor.palette.as_slice())
            .chain(self.lava.iter().flatten().map(|twin| twin.palette.as_slice())).collect::<Vec<_>>().into_iter())
    }
}
