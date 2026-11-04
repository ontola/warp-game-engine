//! Monsters with real skeletons (Joep: proper 3D animation): skinned bodies
//! from `app/assets/characters` (see its README for where they come from).
//! A level reserves a pool of actors per body; a monster takes one while it
//! is around, and one that finds the pool empty keeps its old rigid body.
//! The clips cross-fade: idle, walk, attack (held on its wind-up, then
//! followed through), hit and die. The view's model transform stays on top,
//! so swelling tells and the element deaths work on these bodies too.
use assets::skinned::{ActorHandle, Character};
use glam::{Affine3A, Mat4, Vec3};
use render::skinning::blend_palette_into;
use render::{Gpu, Renderer, Scene};
use spellstorm::actor::{Actor, HURT_TIME};
use spellstorm::enemy::{Enemy, Kind};
use spellstorm::world::Id;
use std::collections::HashMap;
use std::path::PathBuf;

/// The bodies there are, and how many of each a level reserves.
const BODIES: [(&str, usize); 4] = [("skeleton", 16), ("bone_archer", 8), ("necromancer", 4), ("gravekeeper", 1)];
/// Seconds a change of clip fades over.
const FADE: f32 = 0.14;
/// How far into the attack clip the blow lands: its wind-up runs to here.
const STRIKE: f32 = 0.45;
/// Where an actor waits when nobody wears it.
const AWAY: Vec3 = Vec3::new(0.0, -500.0, 0.0);

#[derive(Clone, Copy, PartialEq)]
enum Clip {
    Idle,
    Walk,
    Attack,
    Hit,
    Die,
}

impl Clip {
    fn name(self) -> &'static str {
        match self {
            Clip::Idle => "idle",
            Clip::Walk => "walk",
            Clip::Attack => "attack",
            Clip::Hit => "hit",
            Clip::Die => "die",
        }
    }
    fn looping(self) -> bool {
        matches!(self, Clip::Idle | Clip::Walk)
    }
}

struct Slot {
    body: usize,
    handle: ActorHandle,
    owner: Option<Id>,
    palette: Vec<Mat4>,
    now: Vec<Mat4>,
    from: Vec<Mat4>,
    fade: f32,
    clip: Clip,
    t: f32,
    /// Following through after the wind-up ended.
    strike: Option<f32>,
    was_winding: bool,
}

pub struct Skins {
    bodies: Vec<(&'static str, Character)>,
    slots: Vec<Slot>,
    worn: HashMap<Id, usize>,
    /// The instances the slots hold, first and how many (they are in a row).
    pub instances: std::ops::Range<usize>,
}

fn dir() -> PathBuf {
    crate::kits::default_dir().join("../characters")
}

impl Skins {
    pub fn load() -> Skins {
        let mut bodies = Vec::new();
        for (name, _) in BODIES {
            match Character::load(&dir().join(format!("{name}.json"))) {
                Ok(c) => bodies.push((name, c)),
                Err(e) => eprintln!("skins: {name}: {e}"),
            }
        }
        Skins { bodies, slots: Vec::new(), worn: HashMap::new(), instances: 0..0 }
    }

    /// A new scene: every pool afresh.
    pub fn reserve(&mut self, scene: &mut Scene) {
        self.slots.clear();
        self.worn.clear();
        let first = scene.instances.len();
        for (body, (name, c)) in self.bodies.iter().enumerate() {
            let n = BODIES.iter().find(|b| b.0 == *name).map_or(0, |b| b.1);
            for _ in 0..n {
                // Room for the view's swelling and tumbling on top.
                let handle = match c.add_actor_with_bounds(scene, Affine3A::from_translation(AWAY), c.min.min(Vec3::splat(-4.0)), c.max.max(Vec3::splat(4.0))) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("skins: {name}: {e}");
                        break;
                    }
                };
                scene.instances[handle.instance].cluster_count = 0;
                let palette = c.sample("idle", 0.0, true).unwrap_or_else(|_| vec![Mat4::IDENTITY; c.joint_count]);
                self.slots.push(Slot { body, handle, owner: None, now: palette.clone(), from: palette.clone(), palette, fade: 1.0, clip: Clip::Idle, t: 0.0, strike: None, was_winding: false });
            }
        }
        self.instances = first..scene.instances.len();
    }

    /// This kind has a skinned body.
    pub fn has(&self, kind: &Kind) -> bool {
        self.body_of(kind).is_some()
    }

    /// The body a kind wears: its own, else the one it borrows.
    fn body_of(&self, kind: &Kind) -> Option<usize> {
        let model = kind.model.unwrap_or(kind.key);
        let of = |name: &str| self.bodies.iter().position(|b| b.0 == name);
        of(kind.key).or_else(|| of(model))
    }

    /// Pose monster `id` (at `xf`) in its skin. False: it has none, draw it
    /// the old way.
    #[allow(clippy::too_many_arguments)]
    pub fn pose(&mut self, scene: &mut Scene, id: Id, kind: &Kind, a: &Actor, e: &Enemy, dead_t: Option<f32>, xf: Mat4, flash: f32, dt: f32) -> bool {
        let Some(body) = self.body_of(kind) else { return false };
        let i = match self.worn.get(&id) {
            Some(&i) => i,
            None => {
                let Some(i) = self.slots.iter().position(|s| s.body == body && s.owner.is_none()) else { return false };
                let s = &mut self.slots[i];
                s.owner = Some(id);
                s.clip = Clip::Idle;
                s.t = id.id() as f32 * 0.37;
                s.fade = 1.0;
                s.strike = None;
                s.was_winding = false;
                self.worn.insert(id, i);
                i
            }
        };
        let c = &self.bodies[body].1;
        let s = &mut self.slots[i];
        let winding = e.wind > 0.0;
        if s.was_winding && !winding && a.alive {
            s.strike = Some(STRIKE);
        }
        s.was_winding = winding;
        let duration = |clip: Clip| c.clips.iter().find(|x| x.name == clip.name()).map_or(1.0, |x| x.duration);
        // What it should be doing, and how far in.
        let (clip, time) = if let Some(t) = dead_t {
            (Clip::Die, t)
        } else if winding {
            (Clip::Attack, (1.0 - (e.wind / e.wind_time().max(1e-3)) as f32) * STRIKE * duration(Clip::Attack))
        } else if let Some(k) = s.strike.filter(|k| *k < 1.0) {
            s.strike = Some(k + dt / duration(Clip::Attack));
            (Clip::Attack, k * duration(Clip::Attack))
        } else if a.hurt_t > 0.0 {
            s.strike = None;
            (Clip::Hit, (1.0 - (a.hurt_t / HURT_TIME) as f32) * 0.35 * duration(Clip::Hit))
        } else {
            s.strike = None;
            let speed = Vec3::new(a.vel.x, 0.0, a.vel.z).length();
            if speed > 0.4 {
                s.t += dt * (speed / 2.2).clamp(0.6, 1.8);
                (Clip::Walk, s.t)
            } else {
                s.t += dt;
                (Clip::Idle, s.t)
            }
        };
        if clip != s.clip {
            s.from.copy_from_slice(&s.palette);
            s.fade = 0.0;
            s.clip = clip;
        }
        if c.sample_into(clip.name(), time, clip.looping(), &mut s.now).is_err() {
            return false;
        }
        s.fade = (s.fade + dt / FADE).min(1.0);
        if s.fade < 1.0 {
            let _ = blend_palette_into(&s.from, &s.now, s.fade, &mut s.palette);
        } else {
            s.palette.copy_from_slice(&s.now);
        }
        // The dead sink away at the end instead of vanishing.
        let sink = dead_t.map_or(0.0, |t| ((t - 4.4) * 1.1).max(0.0));
        let xf = Mat4::from_translation(Vec3::NEG_Y * sink) * xf;
        let flags = scene.instances[s.handle.instance].flags;
        scene.update_instance(s.handle.instance, s.handle.mesh, Affine3A::from_mat4(xf));
        scene.instances[s.handle.instance].flags = flags;
        scene.instances[s.handle.instance].set_hit_flash(flash);
        true
    }

    /// Whoever is gone (`alive` says who is still around) gives its body back.
    pub fn release(&mut self, scene: &mut Scene, seen: &[Id]) {
        let slots = &mut self.slots;
        self.worn.retain(|id, i| {
            if seen.contains(id) {
                return true;
            }
            let s = &mut slots[*i];
            s.owner = None;
            scene.instances[s.handle.instance].cluster_count = 0;
            false
        });
    }

    /// `id` left its body for good (it shattered into chunks).
    pub fn shed(&mut self, scene: &mut Scene, id: Id) {
        if let Some(i) = self.worn.get(&id) {
            scene.instances[self.slots[*i].handle.instance].cluster_count = 0;
        }
    }

    pub fn flush(&self, gpu: &Gpu, renderer: &Renderer) -> Result<(), String> {
        if self.slots.is_empty() {
            return Ok(());
        }
        renderer.set_skinning_palettes(gpu, self.slots.iter().map(|s| s.palette.as_slice()).collect::<Vec<_>>().into_iter())?;
        Ok(())
    }
}
