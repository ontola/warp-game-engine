//! Monsters with real skeletons (Joep: proper 3D animation): skinned bodies
//! from `app/assets/characters` (see its README for where they come from).
//! A level reserves a pool of actors per body; a monster takes one while it
//! is around, and one that finds the pool empty keeps its old rigid body.
//! The engine's `body::Gait` blends the clips, as it does for rigged pieces:
//! the pace follows the length of the legs. The view's model transform stays
//! on top, so swelling tells and the element deaths work on these bodies too.
use assets::skinned::{ActorHandle, Character};
use body::{Action, ClipSet, Doing, Gait};
use glam::{Affine3A, Mat4, Vec3};
use render::{Gpu, Renderer, Scene};
use spellstorm::actor::Actor;
use spellstorm::enemy::{Enemy, Kind};
use spellstorm::world::Id;
use std::collections::HashMap;
use std::path::PathBuf;

/// The bodies in use, and how many of each a level reserves, for example
/// ("skeleton", 16). None for now: KayKit's skeletons are too cartoony for this
/// game (Joep: Diablo and Dark Souls, not big heads), so those monsters keep
/// their own models on the motion-captured rig until darker bodies are found.
const BODIES: [(&str, usize); 0] = [];
/// Every body's clips; a `ClipSet` id is the place here.
const CLIPS: [&str; 5] = ["idle", "walk", "hit", "die", "attack"];
/// How far into the attack clip the blow lands: its wind-up runs to here.
const STRIKE: f32 = 0.45;
/// Where an actor waits when nobody wears it.
const AWAY: Vec3 = Vec3::new(0.0, -500.0, 0.0);

struct Body {
    name: &'static str,
    character: Character,
    set: ClipSet,
    attack: Action,
    /// How high its hips stand, at its own scale (metres).
    hip: f32,
}

impl Body {
    fn new(name: &'static str, character: Character) -> Result<Body, String> {
        let length = |clip: &str| character.clips.iter().find(|c| c.name == clip).map(|c| c.duration).ok_or(format!("{name}: no clip {clip}"));
        let lengths = CLIPS.iter().map(|c| length(c)).collect::<Result<Vec<_>, _>>()?;
        let strike = STRIKE * lengths[4];
        let idle = character.sample("idle", 0.0, true)?;
        let hip = character.joint_position("hips", &idle).map_or(0.9, |p| p.y);
        Ok(Body {
            name,
            set: ClipSet { idle: 0, walk: 1, jog: 1, hit: 2, death: 3, lengths },
            attack: Action { wind: 4, wind_to: strike, release: 4, release_from: strike },
            hip,
            character,
        })
    }
}

struct Slot {
    body: usize,
    handle: ActorHandle,
    owner: Option<Id>,
    palette: Vec<Mat4>,
    layer: Vec<Mat4>,
    gait: Gait,
    hurt_seen: f64,
}

pub struct Skins {
    bodies: Vec<Body>,
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
            match Character::load(&dir().join(format!("{name}.json"))).and_then(|c| Body::new(name, c)) {
                Ok(b) => bodies.push(b),
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
        for (i, b) in self.bodies.iter().enumerate() {
            let c = &b.character;
            let n = BODIES.iter().find(|x| x.0 == b.name).map_or(0, |x| x.1);
            for _ in 0..n {
                // Room for the view's swelling and tumbling on top.
                let handle = match c.add_actor_with_bounds(scene, Affine3A::from_translation(AWAY), c.min.min(Vec3::splat(-4.0)), c.max.max(Vec3::splat(4.0))) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("skins: {}: {e}", b.name);
                        break;
                    }
                };
                scene.instances[handle.instance].cluster_count = 0;
                let palette = c.sample("idle", 0.0, true).unwrap_or_else(|_| vec![Mat4::IDENTITY; c.joint_count]);
                self.slots.push(Slot { body: i, handle, owner: None, layer: palette.clone(), palette, gait: Gait::default(), hurt_seen: 0.0 });
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
        let of = |name: &str| self.bodies.iter().position(|b| b.name == name);
        of(kind.key).or_else(|| of(model))
    }

    /// Pose monster `id` (at `xf`, `scale` times its body's size) in its
    /// skin. False: it has none, draw it the old way.
    #[allow(clippy::too_many_arguments)]
    pub fn pose(&mut self, scene: &mut Scene, id: Id, kind: &Kind, a: &Actor, e: &Enemy, dead_t: Option<f32>, xf: Mat4, scale: f32, flash: f32, dt: f32) -> bool {
        let Some(body) = self.body_of(kind) else { return false };
        let i = match self.worn.get(&id) {
            Some(&i) => i,
            None => {
                let Some(i) = self.slots.iter().position(|s| s.body == body && s.owner.is_none()) else { return false };
                let s = &mut self.slots[i];
                s.owner = Some(id);
                s.gait = Gait::default();
                s.hurt_seen = 0.0;
                self.worn.insert(id, i);
                i
            }
        };
        let b = &self.bodies[body];
        let s = &mut self.slots[i];
        let doing = Doing {
            speed: a.vel.with_y(0.0).length(),
            wind: (e.wind > 0.0 || e.boss_wind > 0.0).then(|| (b.attack, e.wind_k() as f32)),
            hit: a.hurt_t > s.hurt_seen + 1e-6,
            dead: dead_t.is_some(),
            held: a.frozen > 0.0 || a.stun > 0.0,
        };
        s.hurt_seen = a.hurt_t;
        let layers = s.gait.step(&b.set, b.hip * scale, &doing, dt);
        let total: f32 = layers.iter().map(|l| l.w.max(0.0)).sum();
        if total > 1e-4 {
            s.palette.fill(Mat4::ZERO);
            for l in layers.iter().filter(|l| l.w > 0.0) {
                if b.character.sample_into(CLIPS[l.clip], l.t, l.looped, &mut s.layer).is_err() {
                    return false;
                }
                for (out, m) in s.palette.iter_mut().zip(&s.layer) {
                    *out += *m * (l.w / total);
                }
            }
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

    /// Whoever is gone (not in `seen`) gives its body back.
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
