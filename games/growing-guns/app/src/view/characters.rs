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
struct Actor { handle: ActorHandle, palette: Vec<Mat4>, run_palette: Vec<Mat4>, motion: Motion,
    corpse: Option<Corpse>, corpse_transform: Option<Affine3A>, velocity: Vec3 }

/// A player's body turned to lava (Godot `_apply_lava_body`): the same
/// character drawn in the lava material, posed like the corpse.
struct LavaTwin { handle: ActorHandle, palette: Vec<Mat4> }

pub struct Characters {
    models: Vec<Character>,
    actors: Vec<Actor>,
    lava: Vec<Option<LavaTwin>>,
    pub hands: Vec<Option<Vec3>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_enemy_and_round_reset_release_the_old_skin_rig() {
        let model = Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/characters/knight.json")).unwrap();
        let mut c = Characters { models: vec![model], actors: Vec::new(), lava: Vec::new(), hands: Vec::new() };
        let mut scene = Scene::default();
        c.reserve_actor(&mut scene, 0).unwrap();
        let mut world = World::new(3, &[false, true, true]);
        let mut physics = Physics::new(Vec3::new(0.0, -20.0, 0.0), 60.0);
        world.players[0].alive = false;
        c.kill(&mut physics, 0, &world.players[0], Vec3::X).unwrap();
        physics.step(); c.follow_physics(&physics);
        c.pose(&world, &[], 1.0);
        assert!(c.corpse_transform(0).is_some());
        assert_eq!(physics.body_count(), 11);
        world.players[0].alive = true;
        c.tick(&world, &mut physics);
        c.pose(&world, &[], 1.0);
        assert_eq!(physics.body_count(), 0);
        assert!(c.corpse_transform(0).is_none() && c.hands[0].is_some());
        world.players[0].alive = false;
        c.kill(&mut physics, 0, &world.players[0], Vec3::X).unwrap();
        c.reset(&mut physics);
        assert_eq!(physics.body_count(), 0);
        assert_eq!(physics.world.impulse_joints.len(), 0);
        assert_eq!(scene.skinning.actors.len(), 1, "reusing a player never aliases or allocates another skin");
    }
}

pub fn model_transform(pos: Vec3, yaw: f32) -> Affine3A {
    Affine3A::from_rotation_translation(
        Quat::from_rotation_y(yaw + std::f32::consts::PI), pos - Vec3::Y * 0.9)
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
        let models = ROSTER.iter().map(|name| Character::load(&dir.join(format!("{name}.json"))))
            .collect::<Result<Vec<_>, _>>()?;
        // Fail before reserving scene slots if a required clip/socket is absent.
        for model in &models {
            for clip in [IDLE, RUN, JUMP] {
                model.sample(clip, 0.0, clip != JUMP)?;
                model.sample_socket(clip, "RightHand", 0.0, clip != JUMP)?;
            }
        }
        println!("characters: original knight, paladin, ch10 with idle/run/jump ({})", dir.display());
        Ok(Self { models, actors: Vec::new(), lava: Vec::new(), hands: Vec::new() })
    }

    pub fn reserve_actor(&mut self, scene: &mut Scene, player: usize) -> Result<usize, String> {
        let model = &self.models[player % self.models.len()];
        // Articulated poses extend outside the authored locomotion bounds.
        let handle = model.add_actor_with_bounds(scene, model_transform(Vec3::new(0.0, -500.0, 0.0), 0.0),
            model.min.min(Vec3::splat(-4.0)), model.max.max(Vec3::splat(4.0)))?;
        let palette = model.sample(IDLE, 0.0, true)?;
        let mesh = handle.mesh;
        let run_palette = vec![Mat4::IDENTITY; palette.len()];
        self.actors.push(Actor { handle, palette, run_palette, motion: Motion::default(),
            corpse: None, corpse_transform: None, velocity: Vec3::ZERO });
        self.hands.push(None);
        Ok(mesh)
    }

    /// A lava twin per actor while the renderer has skinning room left
    /// (after every player has its own actor). Their instances start hidden.
    pub fn reserve_lava(&mut self, scene: &mut Scene, material: u32) {
        self.lava = (0..self.actors.len()).map(|i| {
            let model = &self.models[i % self.models.len()];
            let handle = model.add_actor_with_material(scene, model_transform(Vec3::new(0.0, -500.0, 0.0), 0.0), Some(material)).ok()?;
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

    pub fn pose(&mut self, world: &World, prev: &[Vec3], alpha: f32) {
        for (i, (actor, p)) in self.actors.iter_mut().zip(&world.players).enumerate() {
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
            let pos = prev.get(i).map_or(p.pos, |&q| q.lerp(p.pos, alpha));
            self.hands[i] = Some(model_transform(pos,p.yaw).transform_point3(socket.w_axis.truncate()));
        }
    }

    pub fn kill(&mut self, physics: &mut Physics, player: usize, p: &growing_guns::Player, direction: Vec3) -> Result<(), String> {
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
        actor.corpse = Some(Corpse::spawn(model, &actor.palette, model_transform(p.pos, p.yaw), actor.velocity, direction, player, physics)?);
        self.hands[player] = None;
        Ok(())
    }

    pub fn follow_physics(&mut self, physics: &Physics) {
        for actor in &mut self.actors { if let Some(c) = &mut actor.corpse { c.rig.update(physics); } }
    }

    pub fn corpse_transform(&self, player: usize) -> Option<Affine3A> {
        self.actors.get(player).and_then(|a| a.corpse_transform)
    }

    pub fn ragdolls(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.actors.iter().filter_map(|a| a.corpse.as_ref()).map(|c| c.rig.root_position(1.0).to_array())
    }

    pub fn flush(&self, gpu: &Gpu, renderer: &Renderer) -> Result<(), String> {
        // Actors are reserved consecutively into the scene's skinning table:
        // the players', then their lava twins.
        debug_assert!(self.actors.iter().map(|a| a.handle.actor).chain(self.lava.iter().flatten().map(|t| t.handle.actor)).enumerate().all(|(i, actor)| actor == i));
        renderer.set_skinning_palettes(gpu, self.actors.iter().map(|actor| actor.palette.as_slice())
            .chain(self.lava.iter().flatten().map(|twin| twin.palette.as_slice())).collect::<Vec<_>>().into_iter())
    }
}
