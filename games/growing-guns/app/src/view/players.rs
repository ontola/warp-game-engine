//! Players on screen: body, head and eyes per player, bullets (Godot
//! `bullet.gd`: a brass body, a nose and a glowing streak behind), and
//! how bodies go when they die (Godot `do_ragdoll`): a corpse that tumbles,
//! a head that bursts, or the whole body in gibs with blood all around.
use super::arena::ArenaView;
use super::characters::{self, Characters};
use super::effects::EffectsView;
use super::gore::{self, Gore};
use super::{material, PLAYER_COLORS};
use glam::{Affine3A, Quat, Vec3};
use growing_guns::{Gib, Player, World};
use physics::{Body, BodyDesc, Physics, Shape};
use render::particles::Burst;
use render::primitives;
use render::slots::{SlotId, Slots};
use render::Scene;

pub const TRACERS: usize = 64;
/// Godot `BULLET_BODY_RADIUS`, `BULLET_BODY_LENGTH`, `BULLET_NOSE_LENGTH`.
const BULLET_RADIUS: f32 = 0.016;
const BULLET_LENGTH: f32 = 0.05;
const NOSE_LENGTH: f32 = 0.024;
pub const TRAIL_IMAGE: render::sprites::SpriteImage = render::sprites::SpriteImage(0x7472_6169);
/// Nits per unit of the streak's Godot HDR (head strength x bloom gain 10).
const TRAIL_NITS: f32 = 1_500.0;

/// The streak's texture (`bullet_trail.gdshader`): across, exp(-0.85 r²)
/// with r in streak radii (the image spans 3 radii either side); along,
/// from the bullet (v 0) (1 - v)² faded to nothing at the tail.
pub fn trail_picture() -> render::picture::Picture {
    let (w, h) = (32u32, 128u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let t = ((v - 1.0) / (0.06 - 1.0)).clamp(0.0, 1.0);
        let along = (1.0 - v).powi(2) * t * t * (3.0 - 2.0 * t);
        for x in 0..w {
            let r = (((x as f32 + 0.5) / w as f32) * 2.0 - 1.0).abs() * 3.0;
            let a = (-0.85 * r * r).exp() * along;
            px.extend([255, 255, 255, (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
        }
    }
    render::picture::Picture::new(w, h, px).expect("fixed size")
}

/// Metres over which a round's drawn path joins the sim's (barrel exit to
/// the crosshair's line).
const CONVERGE: f32 = 12.0;

/// A bullet's streak (Godot `_trail_inst`) from `tail` to `head`: 4 cm x
/// scale thick, as long as it has flown up to `speed_mult` x 6 + 1 m (a
/// bazooka keeps a stubby tail, a sniper round a long one), in the
/// bullet's colour lifted 18 % toward white, its head strength 3.2 x scale
/// (2.4 to 9).
fn bullet_trail(head: Vec3, tail: Vec3, scale: f32, color: [f32; 3]) -> render::sprites::Sprite {
    let len = head.distance(tail);
    let dir = (head - tail) / len;
    let radius = 0.02 * scale;
    let tint = Vec3::from(color.map(|c| (c + (1.0 - c) * 0.18).powf(2.2)));
    let strength = (3.2 * scale).clamp(2.4, 9.0) * 10.0;
    let mut s = render::sprites::Sprite::shaft(head - dir * len * 0.5, [radius * 6.0, len], dir, TRAIL_IMAGE, [0.0, 0.0, 1.0, 1.0], tint * strength * TRAIL_NITS);
    s.depth_bias = 0.0;
    s.min_pixels = 2.0;
    s
}
/// Instances per player in the dynamic range: body, head, eyes.
pub const PARTS: usize = 3;
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);
// Inactive slots must not keep deforming their meshes or inflate direct-draw
// budgets. update_instance restores the registered mesh when the slot returns.
fn pose_instance(scene:&mut Scene,slot:usize,mesh:usize,xf:Affine3A) {
    scene.update_instance(slot,mesh,xf);
    if Vec3::from(xf.translation)==HIDDEN {scene.instances[slot].cluster_count=0;}
}
const BODY: Vec3 = Vec3::new(0.42, 0.5, 0.42);
const HEAD: f32 = 0.3;

/// `skinned`: an animated character (the renderer has room for a few);
/// the rest are drawn as the primitive blob.
struct PlayerMeshes { body: usize, head: usize, eyes: usize, skinned: bool, own_views: u8 }

/// A corpse: jointed bodies, each drawn with one of the player's meshes.
struct Ragdoll { torso: Body, parts: Vec<(Body, SlotId, usize, Affine3A)> }

/// Godot `Violence.play_lava_death`, tuned in `lava_death_lab.tscn`: the
/// body turns to lava and sinks below the surface (a fall: a splash pillar
/// first, then deeper and slower).
const SINK_FALL: f32 = 0.75;
const SINK_STAND: f32 = 0.5;
const SINK_S_FALL: f32 = 2.15;
const SINK_S_STAND: f32 = 1.35;
const SPLASH_HEIGHT: f32 = 2.2;
const SPLASH_RADIUS: f32 = 0.55;
const SPLASH_RISE_S: f32 = 0.18;
const SPLASH_SINK_S: f32 = 0.55;

/// A body sinking into the lava: a lava twin of the character (an instance
/// in the dynamic range) or lava blobs in corpse slots, and the splash.
/// `surface` is where Godot's `_lava_surface_world_y` puts it: the pool
/// under the floor (LAVA_POOL_Y) for every lava death, the leak's too; the
/// lava floor (0.04 m) in an island arena, even under a risen leak.
struct LavaCorpse { victim: usize, pos: Vec3, yaw: f32, birth: f32, fall: bool, surface: f32, twin: Option<(usize, usize)>, blobs: Vec<(SlotId, usize, Vec3, f32)>, splash: Option<SlotId> }

impl LavaCorpse {
    /// Centre height `t` seconds in (Godot: sine ease-in, after the splash
    /// has risen when it fell in).
    fn y(&self, t: f32) -> f32 {
        let (depth, secs, delay) = if self.fall { (SINK_FALL, SINK_S_FALL, SPLASH_RISE_S) } else { (SINK_STAND, SINK_S_STAND, 0.0) };
        let u = ((t - delay) / secs).clamp(0.0, 1.0);
        self.pos.y + (self.surface - depth - self.pos.y) * (1.0 - (u * std::f32::consts::FRAC_PI_2).cos())
    }

    /// The splash pillar's centre `t` seconds in (quad ease-out up, sine
    /// ease-in down), None once it has sunk.
    fn splash_y(&self, t: f32) -> Option<f32> {
        let y = self.surface + 0.06;
        let (start, peak, end) = (y + SPLASH_HEIGHT * 0.5 + 0.08, y + SPLASH_HEIGHT * 1.05, y - SPLASH_HEIGHT * 0.7);
        if t < SPLASH_RISE_S { let u = t / SPLASH_RISE_S; return Some(start + (peak - start) * (1.0 - (1.0 - u) * (1.0 - u))); }
        let u = (t - SPLASH_RISE_S) / SPLASH_SINK_S;
        (u < 1.0).then(|| peak + (end - peak) * (1.0 - (u * std::f32::consts::FRAC_PI_2).cos()))
    }
}

/// Lava-material blob body and head (players without a character model)
/// and the splash pillar.
#[derive(Clone, Copy, Default)]
struct LavaMeshes { body: usize, head: usize, splash: usize }

pub struct PlayersView {
    /// First instance of the players' block (then the tracers).
    pub first: usize,
    meshes: Vec<PlayerMeshes>,
    /// The bullet's brass body and its nose (Godot BULLET_BODY / NOSE).
    bullet: [usize; 2],
    /// This frame's bullet streaks (additive shafts in the bullet's colour).
    pub trails: Vec<render::sprites::Sprite>,
    /// Per player: drawn barrel exit minus the sim's muzzle at the last shot.
    shot_offset: Vec<Vec3>,
    corpses: Slots,
    ragdolls: Vec<Ragdoll>,
    /// Body, head and eye primitives per player, kept until `reserve_gore`.
    sources: Vec<[primitives::Primitive; 3]>,
    pub gore: Option<Gore>,
    characters: Option<Characters>,
    lava: Vec<LavaCorpse>,
    lava_meshes: LavaMeshes,
}

impl PlayersView {
    /// Corpse slots (static range, before the dynamic instances).
    pub fn reserve_corpses(scene: &mut Scene) -> Self {
        PlayersView { first: 0, meshes: Vec::new(), bullet: [0; 2], trails: Vec::new(), shot_offset: Vec::new(), corpses: Slots::reserve(scene, 32), ragdolls: Vec::new(), sources: Vec::new(), gore: None, characters: None, lava: Vec::new(), lava_meshes: LavaMeshes::default() }
    }

    /// Player and tracer instances, rewritten every frame.
    pub fn reserve_dynamic(&mut self, scene: &mut Scene, players: usize) {
        self.characters = if !crate::graphics::get().characters {None}else{match Characters::load() {
            Ok(characters) => Some(characters),
            Err(error) => { eprintln!("characters: using primitive fallback: {error}"); None }
        }};
        self.first = scene.instances.len();
        let eye_m = material(scene, [0.05, 0.05, 0.06], 0.3);
        for i in 0..players {
            let body_m = material(scene, PLAYER_COLORS[i % 4], 0.55);
            let prims = if crate::graphics::get().minimal_forward {[primitives::sphere(body_m,4,6),primitives::sphere(body_m,4,6),primitives::sphere(eye_m,3,4)]}else{[primitives::sphere(body_m, 10, 16), primitives::sphere(body_m, 8, 14), primitives::sphere(eye_m, 6, 8)]};
            let [proxy_body, head, eyes] = std::array::from_fn(|k| scene.add_mesh(&prims[k].mesh()));
            let actor = self.characters.as_mut().and_then(|c| c.reserve_actor(scene, i).ok());
            let skinned = actor.is_some();
            let body = actor.unwrap_or_else(|| {
                scene.add_instance(proxy_body, Affine3A::from_translation(HIDDEN));
                proxy_body
            });
            for m in [head, eyes] { scene.add_instance(m, Affine3A::from_translation(HIDDEN)); }
            self.meshes.push(PlayerMeshes { body, head, eyes, skinned, own_views: 0 });
            self.sources.push(prims);
        }
        // Godot: an 8-sided brass cylinder (radius 1.6 cm, 5 cm, metallic
        // 0.88, roughness 0.32) and a 2.4 cm cone, both scaled by the shot's
        // bullet scale (a bazooka round is twice the size and more).
        let brass = scene.add_material(render::scene::Material { tint: [0.84f32, 0.70, 0.24].map(|c| c.powf(2.2)), layer: -1, roughness: 0.32, flags: 0, scale: 1.0, metal: 0.88 });
        let tip = scene.add_material(render::scene::Material { tint: [0.80f32, 0.78, 0.66].map(|c| c.powf(2.2)), layer: -1, roughness: 0.24, flags: 0, scale: 1.0, metal: 0.82 });
        self.bullet = [
            scene.add_mesh(&primitives::cylinder(brass, BULLET_RADIUS, BULLET_RADIUS * 1.04, 8).mesh()),
            scene.add_mesh(&primitives::cylinder(tip, 0.0, BULLET_RADIUS, 8).mesh()),
        ];
        for k in 0..TRACERS * 2 { scene.add_instance(self.bullet[k % 2], Affine3A::from_translation(HIDDEN)); scene.mark_no_shadow(); }
        // Lava deaths: twins of the characters after the tracers (outside
        // the players' block), blobs and the splash in the lava material.
        let lava_m = super::arena::lava_material(scene);
        if let Some(characters) = &mut self.characters { characters.reserve_lava(scene, lava_m); }
        self.lava_meshes = LavaMeshes {
            body: scene.add_mesh(&primitives::sphere(lava_m, 10, 16).mesh()),
            head: scene.add_mesh(&primitives::sphere(lava_m, 8, 14).mesh()),
            splash: scene.add_mesh(&primitives::cylinder(lava_m, 0.42, 1.0, 10).mesh()),
        };
    }

    /// Gib pieces shattered from the players' own meshes.
    pub fn reserve_gore(&mut self, scene: &mut Scene, seed: u64) {
        let models = self.characters.as_ref().map_or(&[][..], |c| c.models());
        self.gore = Some(Gore::reserve(scene, &std::mem::take(&mut self.sources), models, seed));
    }

    fn gore(&mut self) -> &mut Gore { self.gore.as_mut().expect("reserve_gore") }

    /// A new round: no corpses, no gore. Call before clearing the physics.
    pub fn clear(&mut self, scene: &mut Scene, phys: &mut Physics) {
        self.corpses.clear(scene);
        self.ragdolls.clear();
        self.lava.clear();
        if let Some(characters) = &self.characters {
            for (k, mesh) in characters.lava_instances() { pose_instance(scene, k, mesh, Affine3A::from_translation(HIDDEN)); }
        }
        self.gore().clear(scene, phys);
        if let Some(characters) = &mut self.characters { characters.reset(phys); }
    }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene) -> Result<(), String> {
        if let Some(characters) = &self.characters { characters.flush(gpu, renderer)?; }
        self.corpses.flush(gpu, renderer, scene)
    }

    pub fn tick(&mut self, world: &World, phys: &mut Physics) {
        if let Some(characters) = &mut self.characters { characters.tick(world, phys); }
    }

    pub fn hands(&self) -> &[Option<Vec3>] {
        self.characters.as_ref().map_or(&[], |c| c.hands.as_slice())
    }

    /// A death: lava takes the body whole, otherwise by the kill's gib mode.
    #[allow(clippy::too_many_arguments)]
    pub fn kill(&mut self, scene: &mut Scene, phys: &mut Physics, fx: &mut EffectsView, arena: &mut ArenaView, world: &World,
        victim: usize, attacker: Option<usize>, gib: Gib, dir: Vec3, now: f32) {
        let v = &world.players[victim];
        // Lava (Godot `_lava_death`): no ragdoll and no blood, the body
        // takes the lava's look and sinks.
        if let Gib::Lava { fall } = gib {
            if victim < self.meshes.len() { self.lava_death(scene, victim, v, fall, now, world.arena.lava_surface()); }
            return;
        }
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
        // A burst (overkill, explosions) shatters skinned bodies as well: no
        // corpse, so the character stays hidden while the chunks fly.
        if skinned && !matches!(gib, Gib::Burst { .. }) {
            // Drive the original textured skin from articulated limb bodies.
            if let Some(characters) = &mut self.characters {
                if let Err(error) = characters.kill(phys, victim, v, push, matches!(gib, Gib::Head)) { eprintln!("ragdoll: {error}"); }
            }
            if matches!(gib, Gib::Head) {
                self.gore().burst(scene, phys, victim, &[gore::Part::Head, gore::Part::Eyes], v.pos, v.yaw, v.vel, push, 0.5, now);
            }
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
            Gib::Lava { .. } => unreachable!("handled above"),
            Gib::Ragdoll => {
                fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..Burst::drops() });
                self.ragdoll(scene, phys, victim, v, push, true);
            }
        }
    }

    /// Godot `Violence.play_lava_death`: a lava twin of the character (or
    /// lava blobs) where the body was, and a splash pillar when it fell in.
    fn lava_death(&mut self, scene: &mut Scene, victim: usize, v: &Player, fall: bool, now: f32, surface: f32) {
        self.end_lava(scene, victim);
        let twin = if self.meshes[victim].skinned { self.characters.as_mut().and_then(|c| c.lava_twin(victim)) } else { None };
        let mut blobs = Vec::new();
        if twin.is_none() {
            let lm = self.lava_meshes;
            for (mesh, offset, scale) in [(lm.body, -0.25, BODY), (lm.head, 0.55, Vec3::splat(HEAD))] {
                if let Some(slot) = self.corpses.add(scene, mesh, Affine3A::from_translation(HIDDEN)) { blobs.push((slot, mesh, scale, offset)); }
            }
        }
        let splash = if fall { self.corpses.add(scene, self.lava_meshes.splash, Affine3A::from_translation(HIDDEN)) } else { None };
        let c = LavaCorpse { victim, pos: v.pos, yaw: v.yaw, birth: now, fall, surface, twin, blobs, splash };
        self.lava.push(c);
    }

    /// Godot `end_lava_death`: player `victim`'s lava body goes (a revive).
    fn end_lava(&mut self, scene: &mut Scene, victim: usize) {
        let Some(k) = self.lava.iter().position(|c| c.victim == victim) else { return };
        let c = self.lava.swap_remove(k);
        if let Some((instance, mesh)) = c.twin { pose_instance(scene, instance, mesh, Affine3A::from_translation(HIDDEN)); }
        for (slot, ..) in c.blobs { self.corpses.remove(scene, slot); }
        if let Some(slot) = c.splash { self.corpses.remove(scene, slot); }
    }

    /// Sinking lava bodies and their splashes at `clock` seconds.
    fn pose_lava(&mut self, scene: &mut Scene, world: &World, clock: f32) {
        let revived: Vec<usize> = self.lava.iter().map(|c| c.victim).filter(|&i| world.players.get(i).is_none_or(|p| p.alive)).collect();
        for i in revived { self.end_lava(scene, i); }
        let rot = Quat::from_rotation_y(0.0);
        for c in &mut self.lava {
            let t = (clock - c.birth).max(0.0);
            let at = Vec3::new(c.pos.x, c.y(t), c.pos.z);
            if let Some((instance, mesh)) = c.twin { pose_instance(scene, instance, mesh, characters::model_transform(at, c.yaw)); }
            for &(slot, mesh, scale, offset) in &c.blobs {
                self.corpses.set(scene, slot, mesh, Affine3A::from_scale_rotation_translation(scale, Quat::from_rotation_y(c.yaw), at + Vec3::Y * offset));
            }
            if let Some(slot) = c.splash {
                match c.splash_y(t) {
                    Some(y) => self.corpses.set(scene, slot, self.lava_meshes.splash, Affine3A::from_scale_rotation_translation(
                        Vec3::new(SPLASH_RADIUS, SPLASH_HEIGHT, SPLASH_RADIUS), rot, Vec3::new(c.pos.x, y, c.pos.z))),
                    None => { self.corpses.remove(scene, slot); c.splash = None; }
                }
            }
        }
    }

    /// A corpse that tumbles: torso, and the head unless it burst. Pushed
    /// away from the killing blow.
    fn ragdoll(&mut self, scene: &mut Scene, phys: &mut Physics, victim: usize, v: &Player, dir: Vec3, with_head: bool) {
        let push = dir.with_y(0.0).normalize_or_zero() * 5.0 + Vec3::Y * 2.0;
        let rot = Quat::from_rotation_y(v.yaw);
        let (b, h) = (self.meshes[victim].body, self.meshes[victim].head);
        let (shape, center, local) = (Shape::Capsule(0.12, 0.4), v.pos - Vec3::Y * 0.25, Affine3A::from_scale(BODY));
        let torso = phys.add(&BodyDesc { rot, vel: v.vel + push, density: 300.0, ..BodyDesc::new(shape, center) });
        let Some(ts) = self.corpses.add(scene, b, Affine3A::from_translation(v.pos)) else { phys.remove(torso); return };
        let mut parts = vec![(torso, ts, b, local)];
        if with_head {
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
        if let Some(characters) = &mut self.characters { characters.follow_physics(phys); }
        for r in &self.ragdolls {
            for &(body, slot, mesh, local) in &r.parts {
                if let Some((pos, rot)) = phys.pose(body) {
                    self.corpses.set(scene, slot, mesh, Affine3A::from_rotation_translation(rot, pos) * local);
                }
            }
        }
    }

    /// Players (interpolated by `alpha` from `prev`) and tracers.
    pub fn pose(&mut self, scene: &mut Scene, world: &World, prev: &[crate::view::Pose], alpha: f32) {
        if let Some(characters) = &mut self.characters { characters.pose(world, prev, alpha); }
        let mut slot = self.first;
        for (i, m) in self.meshes.iter().enumerate() {
            let Some(_p) = world.players.get(i).filter(|p| p.alive) else {
                let corpse = world.players.get(i).and_then(|_| self.characters.as_ref().and_then(|c| c.corpse_transform(i)));
                for (part, mesh) in [m.body, m.head, m.eyes].into_iter().enumerate() {
                    let xf = if part == 0 { corpse.unwrap_or(Affine3A::from_translation(HIDDEN)) } else { Affine3A::from_translation(HIDDEN) };
                    pose_instance(scene, slot, mesh, xf);
                    scene.instances[slot].hide_in_views(0);
                    slot += 1;
                }
                continue;
            };
            let drawn = crate::view::pose(world, i, prev, alpha);
            let (pos, rot) = (drawn.pos, Quat::from_rotation_y(drawn.yaw));
            let (body, head, eyes) = if m.skinned {
                (characters::model_transform(pos, drawn.yaw), Affine3A::from_translation(HIDDEN), Affine3A::from_translation(HIDDEN))
            } else {
                (Affine3A::from_scale_rotation_translation(BODY, rot, pos + Vec3::Y * -0.25),
                 Affine3A::from_scale_rotation_translation(Vec3::splat(HEAD), rot, pos + Vec3::Y * 0.55),
                 Affine3A::from_scale_rotation_translation(Vec3::new(0.2, 0.07, 0.05), rot, pos + Vec3::Y * 0.6 + rot * Vec3::new(0.0, 0.0, -0.27)))
            };
            for (mesh, xf) in [(m.body, body), (m.head, head), (m.eyes, eyes)] {
                pose_instance(scene, slot, mesh, xf);
                scene.instances[slot].hide_in_views(m.own_views);
                slot += 1;
            }
        }
        self.pose_lava(scene, world, (world.tick as f32 - 1.0 + alpha) * growing_guns::DT);
        // Bullets, drawn where they are at this frame (between ticks), nose
        // first: the body and nose once clear of the muzzle, the streak
        // from the first metre on so a player's own never fills the screen.
        self.trails.clear();
        let behind = (1.0 - alpha).clamp(0.0, 1.0) * growing_guns::DT;
        for t in 0..TRACERS {
            let shown = world.bullets.get(t).filter(|b| b.travelled > 0.05 && b.vel.length_squared() > 1e-6);
            let (body, nose) = shown.map_or((Affine3A::from_translation(HIDDEN), Affine3A::from_translation(HIDDEN)), |b| {
                let w = world.players.get(b.owner).map(|p| &p.weapon);
                let scale = w.map_or(1.0, |w| w.bullet_scale_for_shot(b.last_in_mag)).clamp(0.3, 8.0);
                let dir = b.vel.normalize();
                // Where the round is at distance `d` from the muzzle: on the
                // sim's path, pulled to the drawn barrel near its start.
                let flown = (b.travelled - b.vel.length() * behind).max(0.0);
                let offset = self.shot_offset.get(b.owner).copied().unwrap_or(Vec3::ZERO);
                let sim_pos = b.pos - b.vel * behind;
                let at = |d: f32| sim_pos - dir * (flown - d) + offset * (1.0 - d / CONVERGE).max(0.0);
                let pos = at(flown);
                let (body_len, nose_len) = (BULLET_LENGTH * scale, NOSE_LENGTH * scale);
                let dir = (pos - at((flown - 0.5).max(0.0))).normalize_or(dir);
                let rot = Quat::from_rotation_arc(Vec3::Y, dir);
                if let Some(w) = w {
                    let len = (w.bullet_speed_mult * 6.0 + 1.0).clamp(1.0, 28.0);
                    let head = pos - dir * body_len;
                    let tail = at((flown - len).max(0.0));
                    if head.distance(tail) > 0.05 { self.trails.push(bullet_trail(head, tail, scale, w.bullet_color)); }
                }
                (Affine3A::from_scale_rotation_translation(Vec3::new(scale, body_len, scale), rot, pos - dir * body_len * 0.5),
                 Affine3A::from_scale_rotation_translation(Vec3::new(scale, nose_len, scale), rot, pos + dir * nose_len * 0.5))
            });
            pose_instance(scene, slot, self.bullet[0], body);
            pose_instance(scene, slot + 1, self.bullet[1], nose);
            slot += 2;
        }
    }

    /// A shot left player `p`'s drawn barrel `offset` from the sim's muzzle.
    pub fn shot_from(&mut self, p: usize, offset: Vec3) {
        if self.shot_offset.len() <= p { self.shot_offset.resize(p + 1, Vec3::ZERO); }
        // A wild offset (a teleport, a respawn between frames) is ignored.
        self.shot_offset[p] = if offset.length() < 4.0 { offset } else { Vec3::ZERO };
    }

    /// First person: hide each local player's own model from their view
    /// (`slots[k]` is drawn in view k). It still casts its shadow.
    pub fn hide_own(&mut self, scene: &mut Scene, slots: &[usize]) {
        for (i, m) in self.meshes.iter_mut().enumerate() {
            m.own_views = 0;
            for part in 0..PARTS { scene.instances[self.first + i * PARTS + part].hide_in_views(0); }
        }
        for (k, &s) in slots.iter().enumerate().take(8) {
            let Some(m) = self.meshes.get_mut(s) else { continue };
            m.own_views |= 1 << k;
            for part in 0..PARTS { scene.instances[self.first + s * PARTS + part].hide_in_views(m.own_views); }
        }
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn ragdolls(&self, phys: &Physics) -> Vec<[f32; 3]> {
        let mut positions: Vec<_> = self.ragdolls.iter().filter_map(|r| phys.pose(r.torso)).map(|(p, _)| p.to_array()).collect();
        if let Some(c) = &self.characters { positions.extend(c.ragdolls()); }
        positions
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
