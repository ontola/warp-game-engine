//! Players on screen: body, head and eyes per player, bullet tracers, and
//! how bodies go when they die (Godot `do_ragdoll`): a corpse that tumbles,
//! a head that bursts, or the whole body in gibs with blood all around.
use super::arena::ArenaView;
use super::characters::{self, Characters};
use super::effects::EffectsView;
use super::gore::{self, Gore};
use super::{cube, glow, material, PLAYER_COLORS};
use glam::{Affine3A, Quat, Vec3};
use growing_guns::{Gib, Player, World};
use physics::{Body, BodyDesc, Physics, Shape};
use render::particles::Burst;
use render::primitives;
use render::slots::{SlotId, Slots};
use render::Scene;

pub const TRACERS: usize = 64;
/// Instances per player in the dynamic range: body, head, eyes.
pub const PARTS: usize = 3;
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);
const BODY: Vec3 = Vec3::new(0.42, 0.5, 0.42);
const HEAD: f32 = 0.3;

/// `skinned`: an animated character (the renderer has room for a few);
/// the rest are drawn as the primitive blob.
struct PlayerMeshes { body: usize, head: usize, eyes: usize, skinned: bool }

/// A corpse: jointed bodies, each drawn with one of the player's meshes.
struct Ragdoll { torso: Body, parts: Vec<(Body, SlotId, usize, Affine3A)> }

pub struct PlayersView {
    /// First instance of the players' block (then the tracers).
    pub first: usize,
    meshes: Vec<PlayerMeshes>,
    tracer: usize,
    corpses: Slots,
    ragdolls: Vec<Ragdoll>,
    /// Body, head and eye primitives per player, kept until `reserve_gore`.
    sources: Vec<[primitives::Primitive; 3]>,
    pub gore: Option<Gore>,
    characters: Option<Characters>,
}

impl PlayersView {
    /// Corpse slots (static range, before the dynamic instances).
    pub fn reserve_corpses(scene: &mut Scene) -> Self {
        PlayersView { first: 0, meshes: Vec::new(), tracer: 0, corpses: Slots::reserve(scene, 32), ragdolls: Vec::new(), sources: Vec::new(), gore: None, characters: None }
    }

    /// Player and tracer instances, rewritten every frame.
    pub fn reserve_dynamic(&mut self, scene: &mut Scene, players: usize) {
        self.characters = match Characters::load() {
            Ok(characters) => Some(characters),
            Err(error) => { eprintln!("characters: using primitive fallback: {error}"); None }
        };
        self.first = scene.instances.len();
        let eye_m = material(scene, [0.05, 0.05, 0.06], 0.3);
        for i in 0..players {
            let body_m = material(scene, PLAYER_COLORS[i % 4], 0.55);
            let prims = [primitives::sphere(body_m, 10, 16), primitives::sphere(body_m, 8, 14), primitives::sphere(eye_m, 6, 8)];
            let [proxy_body, head, eyes] = std::array::from_fn(|k| scene.add_mesh(&prims[k].mesh()));
            let actor = self.characters.as_mut().and_then(|c| c.reserve_actor(scene, i).ok());
            let skinned = actor.is_some();
            let body = actor.unwrap_or_else(|| {
                scene.add_instance(proxy_body, Affine3A::from_translation(HIDDEN));
                proxy_body
            });
            for m in [head, eyes] { scene.add_instance(m, Affine3A::from_translation(HIDDEN)); }
            self.meshes.push(PlayerMeshes { body, head, eyes, skinned });
            self.sources.push(prims);
        }
        // Gore only for blob bodies (indexed by player).
        if self.meshes.iter().all(|m| m.skinned) { self.sources.clear(); }
        let tracer_m = glow(scene, [1.0, 0.9, 0.3], 30000.0);
        self.tracer = cube(scene, tracer_m);
        for _ in 0..TRACERS { scene.add_instance(self.tracer, Affine3A::from_translation(HIDDEN)); scene.mark_no_shadow(); }
    }

    /// Gib pieces shattered from the players' own meshes.
    pub fn reserve_gore(&mut self, scene: &mut Scene, seed: u64) {
        self.gore = Some(Gore::reserve(scene, &std::mem::take(&mut self.sources), seed));
    }

    fn gore(&mut self) -> &mut Gore { self.gore.as_mut().expect("reserve_gore") }

    /// A new round: no corpses, no gore. Call before clearing the physics.
    pub fn clear(&mut self, scene: &mut Scene, phys: &mut Physics) {
        self.corpses.clear(scene);
        self.ragdolls.clear();
        self.gore().clear(scene, phys);
        if let Some(characters) = &mut self.characters { characters.reset(); }
    }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene) -> Result<(), String> {
        if let Some(characters) = &self.characters { characters.flush(gpu, renderer)?; }
        self.corpses.flush(gpu, renderer, scene)
    }

    pub fn tick(&mut self, world: &World) {
        if let Some(characters) = &mut self.characters { characters.tick(world); }
    }

    pub fn hands(&self) -> &[Option<Vec3>] {
        self.characters.as_ref().map_or(&[], |c| c.hands.as_slice())
    }

    /// A death: lava takes the body whole, otherwise by the kill's gib mode.
    #[allow(clippy::too_many_arguments)]
    pub fn kill(&mut self, scene: &mut Scene, phys: &mut Physics, fx: &mut EffectsView, arena: &mut ArenaView, world: &World,
        victim: usize, attacker: Option<usize>, gib: Gib, dir: Vec3, now: f32) {
        let v = &world.players[victim];
        let blood = fx.meshes.blood;
        // Past the view's player capacity (a big coop wave): blood only.
        let Some(skinned) = self.meshes.get(victim).map(|m| m.skinned) else {
            fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..Burst::drops() });
            return;
        };
        let Some(a) = attacker else {
            fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..Burst::drops() });
            return;
        };
        let push = if dir.length_squared() > 1e-4 { dir } else { (v.pos - world.players[a].pos).with_y(0.0).normalize_or(Vec3::Y) };
        if skinned {
            // Keep the original textured model in its final animated pose.
            // Mesh-specific dismemberment needs a separate skin-aware bake;
            // do not replace a humanoid corpse with the old coloured blobs.
            self.ragdoll(scene, phys, victim, v, push, false);
            fx.blasts.gib_mist(now, v.pos + Vec3::Y * 0.3, 0.8);
            fx.particles.burst(scene, v.pos, push + Vec3::Y * 0.5, &Burst { mesh: blood, count: 28, spread: 1.6, speed: [2.0, 8.0], ..Burst::drops() });
            blood_splats(scene, arena, world, v.pos, 4, 1.3);
            return;
        }
        match gib {
            Gib::Burst { severity, blast } => {
                // Godot: chaos from the severity, the whole body in chunks,
                // red spray, blood drops and splats on the ground around.
                let chaos = (severity * if blast { 0.75 } else { 0.82 }).clamp(0.35, 1.1);
                self.gore().burst(scene, phys, victim, &[gore::Part::Body, gore::Part::Head, gore::Part::Eyes], v.pos, v.yaw, v.vel, push, chaos, now);
                fx.blasts.gib_mist(now, v.pos + Vec3::Y * 0.2, 1.0 + chaos * 1.5);
                fx.particles.burst(scene, v.pos, push + Vec3::Y * 0.5, &Burst { mesh: blood, count: 28, spread: 1.8, speed: [3.0, 10.0], ..Burst::drops() });
                blood_splats(scene, arena, world, v.pos, (6.0 + chaos * 8.0) as usize, 1.6 + chaos * 1.4);
            }
            Gib::Head => {
                self.gore().burst(scene, phys, victim, &[gore::Part::Head, gore::Part::Eyes], v.pos, v.yaw, v.vel, push, 0.5, now);
                fx.blasts.gib_mist(now, v.pos + Vec3::Y * 0.6, 0.8);
                fx.particles.burst(scene, v.pos + Vec3::Y * 0.55, push + Vec3::Y * 0.4, &Burst { mesh: blood, count: 20, spread: 1.4, speed: [2.0, 8.0], ..Burst::drops() });
                self.ragdoll(scene, phys, victim, v, push, false);
                blood_splats(scene, arena, world, v.pos, 3, 1.2);
            }
            Gib::Ragdoll => {
                fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..Burst::drops() });
                self.ragdoll(scene, phys, victim, v, push, true);
            }
        }
    }

    /// A corpse that tumbles: torso, and the head unless it burst. Pushed
    /// away from the killing blow.
    fn ragdoll(&mut self, scene: &mut Scene, phys: &mut Physics, victim: usize, v: &Player, dir: Vec3, with_head: bool) {
        let push = dir.with_y(0.0).normalize_or_zero() * 5.0 + Vec3::Y * 2.0;
        let rot = Quat::from_rotation_y(v.yaw);
        let (b, h) = (self.meshes[victim].body, self.meshes[victim].head);
        let skinned = self.meshes[victim].skinned;
        let (shape, center, local) = if skinned {
            (Shape::Capsule(0.32, 0.55), v.pos, characters::model_transform(Vec3::ZERO, 0.0))
        } else { (Shape::Capsule(0.12, 0.4), v.pos - Vec3::Y * 0.25, Affine3A::from_scale(BODY)) };
        let torso = phys.add(&BodyDesc { rot, vel: v.vel + push, density: 300.0, ..BodyDesc::new(shape, center) });
        let Some(ts) = self.corpses.add(scene, b, Affine3A::from_translation(v.pos)) else { phys.remove(torso); return };
        if skinned {
            scene.instances[self.corpses.instance(ts)].flags |= render::scene::SKINNED;
        }
        let mut parts = vec![(torso, ts, b, local)];
        if with_head && !skinned {
            let head = phys.add(&BodyDesc { rot, vel: v.vel + push * 1.4, density: 300.0, ..BodyDesc::new(Shape::Sphere(0.28), v.pos + Vec3::Y * 0.55) });
            phys.ball_joint(torso, head, Vec3::Y * 0.52, Vec3::Y * -0.28);
            match self.corpses.add(scene, h, Affine3A::from_translation(v.pos)) {
                Some(hs) => parts.push((head, hs, h, Affine3A::from_scale(Vec3::splat(HEAD)))),
                None => phys.remove(head),
            }
        }
        self.ragdolls.push(Ragdoll { torso, parts });
    }

    /// One tick after the physics step: corpses and gore follow their bodies.
    pub fn follow_physics(&mut self, scene: &mut Scene, phys: &mut Physics, now: f32) {
        self.gore().update(scene, phys, now);
        for r in &self.ragdolls {
            for &(body, slot, mesh, local) in &r.parts {
                if let Some((pos, rot)) = phys.pose(body) {
                    self.corpses.set(scene, slot, mesh, Affine3A::from_rotation_translation(rot, pos) * local);
                }
            }
        }
    }

    /// Players (interpolated by `alpha` from `prev`) and tracers.
    pub fn pose(&mut self, scene: &mut Scene, world: &World, prev: &[Vec3], alpha: f32) {
        let mut slot = self.first;
        for (i, m) in self.meshes.iter().enumerate() {
            let Some(p) = world.players.get(i).filter(|p| p.alive) else {
                for mesh in [m.body, m.head, m.eyes] { scene.update_instance(slot, mesh, Affine3A::from_translation(HIDDEN)); slot += 1; }
                continue;
            };
            let pos = prev.get(i).map_or(p.pos, |&q| q.lerp(p.pos, alpha));
            let rot = Quat::from_rotation_y(p.yaw);
            let (body, head, eyes) = if m.skinned {
                (characters::model_transform(pos, p.yaw), Affine3A::from_translation(HIDDEN), Affine3A::from_translation(HIDDEN))
            } else {
                (Affine3A::from_scale_rotation_translation(BODY, rot, pos + Vec3::Y * -0.25),
                 Affine3A::from_scale_rotation_translation(Vec3::splat(HEAD), rot, pos + Vec3::Y * 0.55),
                 Affine3A::from_scale_rotation_translation(Vec3::new(0.2, 0.07, 0.05), rot, pos + Vec3::Y * 0.6 + rot * Vec3::new(0.0, 0.0, -0.27)))
            };
            for (mesh, xf) in [(m.body, body), (m.head, head), (m.eyes, eyes)] { scene.update_instance(slot, mesh, xf); slot += 1; }
        }
        if let Some(characters) = &mut self.characters { characters.pose(world, prev, alpha); }
        // Tracers: a short glowing streak behind each live bullet. Skip the
        // first metre so a player's own tracer never fills their screen; the
        // streak then grows to four metres.
        for t in 0..TRACERS {
            let xf = world.bullets.get(t).filter(|b| b.travelled > 1.0).map_or(Affine3A::from_translation(HIDDEN), |b| {
                let dir = b.vel.normalize();
                let len = (b.travelled - 1.0).clamp(0.3, 4.0);
                let rot = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
                Affine3A::from_scale_rotation_translation(Vec3::new(0.035, 0.035, len), rot, b.pos - dir * len * 0.5)
            });
            scene.update_instance(slot, self.tracer, xf);
            slot += 1;
        }
    }

    /// First person: hide each local player's own model from their view
    /// (`slots[k]` is drawn in view k). It still casts its shadow.
    pub fn hide_own(&self, scene: &mut Scene, slots: &[usize]) {
        for (k, &s) in slots.iter().enumerate().take(8) {
            for part in 0..PARTS { scene.instances[self.first + s * PARTS + part].hide_in_views(1 << k); }
        }
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn ragdolls(&self, phys: &Physics) -> Vec<[f32; 3]> {
        self.ragdolls.iter().filter_map(|r| phys.pose(r.torso)).map(|(p, _)| p.to_array()).collect()
    }

    pub fn gore_len(&self) -> usize { self.gore.as_ref().map_or(0, |g| g.len()) }
}

/// Blood splats on the ground around a burst body (Godot
/// `spawn_overkill_blood_splats_after_destruction`).
fn blood_splats(scene: &mut Scene, arena: &mut ArenaView, world: &World, at: Vec3, n: usize, reach: f32) {
    for k in 0..n {
        let a = k as f32 * 2.399 + at.x * 1.7 + world.tick as f32 * 0.37;
        let r = reach * ((k as f32 + 0.5) / n as f32).sqrt();
        let from = at + Vec3::new(a.cos() * r, 0.6, a.sin() * r);
        let Some((t, normal, c)) = growing_guns::shape::raycast(&world.arena.grid, from, Vec3::NEG_Y, 6.0) else { continue };
        let size = 0.25 + 0.35 * (1.0 - r / reach.max(0.01));
        let rot = Quat::from_rotation_arc(Vec3::Y, normal) * Quat::from_rotation_y(a * 3.1);
        let xf = Affine3A::from_scale_rotation_translation(Vec3::new(size, 0.02, size * 0.8), rot, from + Vec3::NEG_Y * t);
        arena.blood(scene, xf, c as u32);
    }
}
