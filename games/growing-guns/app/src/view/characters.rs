//! The original roster and locomotion clips. Simulation still uses its capsule;
//! only small skin palettes are uploaded, once per actor rather than per view.
use assets::skinned::{ActorHandle, Character};
use glam::{Affine3A, Mat4, Quat, Vec3};
use growing_guns::{Event, World, DT, WALK_SPEED};
use render::{Gpu, Renderer, Scene};
use std::path::PathBuf;

const ROSTER: [&str; 3] = ["knight", "paladin", "ch10"];
const IDLE: &str = "pistol_idle";
const RUN: &str = "pistol_run";
const JUMP: &str = "pistol_jump";

#[derive(Default)]
struct Motion { idle: f32, run: f32, blend: f32, jump: Option<f32> }
struct Actor { handle: ActorHandle, palette: Vec<Mat4>, run_palette: Vec<Mat4>, motion: Motion }

pub struct Characters {
    models: Vec<Character>,
    actors: Vec<Actor>,
    pub hands: Vec<Option<Vec3>>,
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
        Ok(Self { models, actors: Vec::new(), hands: Vec::new() })
    }

    pub fn reserve_actor(&mut self, scene: &mut Scene, player: usize) -> Result<usize, String> {
        let model = &self.models[player % self.models.len()];
        let handle = model.add_actor(scene, model_transform(Vec3::new(0.0, -500.0, 0.0), 0.0))?;
        let palette = model.sample(IDLE, 0.0, true)?;
        let mesh = handle.mesh;
        let run_palette = vec![Mat4::IDENTITY; palette.len()];
        self.actors.push(Actor { handle, palette, run_palette, motion: Motion::default() });
        self.hands.push(None);
        Ok(mesh)
    }

    pub fn reset(&mut self) {
        for actor in &mut self.actors { actor.motion = Motion::default(); }
        self.hands.fill(None);
    }

    pub fn tick(&mut self, world: &World) {
        for (i, (actor, p)) in self.actors.iter_mut().zip(&world.players).enumerate() {
            if !p.alive { continue; }
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
            // Corpses share this actor's deformed mesh until the next round.
            // Keeping the final palette freezes the death pose without readback.
            if !p.alive { self.hands[i] = None; continue; }
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

    pub fn flush(&self, gpu: &Gpu, renderer: &Renderer) -> Result<(), String> {
        // Actors are reserved consecutively into the scene's skinning table.
        debug_assert!(self.actors.iter().enumerate().all(|(i, actor)| actor.handle.actor == i));
        renderer.set_skinning_palettes(gpu, self.actors.iter().map(|actor| actor.palette.as_slice()))
    }
}
