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

/// Two small textured quads: a round hot head and a subdued short wake.
/// The head faces each eye, so a shot travelling away stays a soft point.
/// No bloom, blur pass, extra light or particle simulation is required.
fn bullet_trail(head: Vec3, tail: Vec3, scale: f32, color: [f32; 3]) -> [render::sprites::Sprite;2] {
    use render::sprites::{Sprite,Facing};
    let distance=head.distance(tail);
    let dir=(head-tail).normalize_or(Vec3::NEG_Z);
    let len=distance.min(0.7*scale.sqrt().clamp(0.65,2.));
    let tint=Vec3::from(color.map(|c|(c+(1.-c)*0.35).powf(2.2)));
    let light=tint*(3.2*scale).clamp(2.4,9.)*10.*TRAIL_NITS;
    let mut wake=Sprite::shaft(head-dir*len*0.5,[0.10*scale,len],dir,TRAIL_IMAGE,[0.,0.,1.,1.],light*0.18);
    wake.depth_bias=0.;wake.min_pixels=2.;
    let diameter=0.16*scale.sqrt();
    let mut point=Sprite::billboard(head,[diameter;2],super::effects::FLASH_IMAGE,[0.,0.,1.,1.],light);
    point.facing=Facing::Screen;point.additive=true;point.alpha_cut=0.;point.depth_bias=0.;point.min_pixels=7.;
    [wake,point]
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
const SINK_FALL: f32 = 1.6;
const SINK_STAND: f32 = 1.6;
const SINK_S_FALL: f32 = 2.15;
const SINK_S_STAND: f32 = 2.15;
const SPLASH_HEIGHT: f32 = 0.35;
const SPLASH_RADIUS: f32 = 0.55;
const SPLASH_RISE_S: f32 = 0.18;
const SPLASH_SINK_S: f32 = 0.55;

/// A body sinking into the lava: a lava twin of the character (an instance
/// in the dynamic range) or lava blobs in corpse slots, and the splash.
/// `surface` is where Godot's `_lava_surface_world_y` puts it: the pool
/// under the floor (LAVA_POOL_Y) for every lava death, the leak's too; the
/// lava floor (0.04 m) in an island arena, even under a risen leak.
struct LavaCorpse { victim: usize, pos: Vec3, yaw: f32, pitch: f32, camera_look: [f32; 2], camera_prev: [f32; 2], birth: f32, fall: bool, surface: f32, twin: Option<(usize, usize)>, hands: [Option<SlotId>; 2], blobs: Vec<(SlotId, usize, Vec3, f32)>, splash: Option<SlotId> }

impl LavaCorpse {
    /// Wrists in world space, shared by the victim's view and the sinking character.
    fn wrists(&self, age: f32) -> [Affine3A; 2] {
        let eye = self.pos.with_y(self.y(age) + growing_guns::EYE);
        let body = Affine3A::from_rotation_translation(Quat::from_rotation_y(self.yaw), eye);
        std::array::from_fn(|side| {
            let sign = if side == 0 { 1. } else { -1. };
            let phase = age * 8.5 + 2.3 * side as f32 + self.victim as f32;
            let lift = super::pov_arms::ease(0.03 * side as f32, 0.34 + 0.06 * side as f32, age);
            let at = Vec3::new(sign * (0.24 + 0.07 * lift * phase.cos()),
                -0.20 + lift * (0.43 + 0.10 * phase.sin()), -0.42 + 0.06 * lift * phase.cos());
            let palm = Quat::from_euler(glam::EulerRot::YXZ,
                sign * (0.26 + 0.24 * phase.sin()), 0.65 + 0.26 * phase.cos(), sign * 0.32);
            body * Affine3A::from_rotation_translation(palm, at)
        })
    }

    fn camera_direction(&self, t: f32, alpha: f32) -> Vec3 {
            let u = (t / 2.4).clamp(0.0, 1.0);
            let look = std::array::from_fn::<_, 2, _>(|i| self.camera_prev[i] + (self.camera_look[i] - self.camera_prev[i]) * alpha);
            let tremor = 0.012 * (t * 5.0).min(1.0) * (0.5 + u);
            let yaw = self.yaw + look[0] + tremor * ((t*19.0).sin() + 0.45*(t*31.7).sin());
            let pitch = (self.pitch + (1.1-self.pitch) * u * u + look[1]
                + tremor * ((t*23.3).sin() + 0.35*(t*37.0).sin())).clamp(-1.35, 1.35);
            growing_guns::look_dir(yaw, pitch)

    }

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
    pub corpse_hits: u64,
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
        PlayersView { corpse_hits: 0, first: 0, meshes: Vec::new(), bullet: [0; 2], trails: Vec::new(), shot_offset: Vec::new(), corpses: Slots::reserve(scene, 32), ragdolls: Vec::new(), sources: Vec::new(), gore: None, characters: None, lava: Vec::new(), lava_meshes: LavaMeshes::default() }
    }

    /// Player and tracer instances, rewritten every frame.
    pub(crate) fn reserve_dynamic(&mut self, scene: &mut Scene, players: usize, seed: u64, colors: &[crate::party::PlayerColors]) {
        let rigid = crate::graphics::get().rigid_characters;
        self.characters = if !crate::graphics::get().characters && !rigid {None}else{match Characters::load_roster(seed, players, colors) {
            Ok(characters) => Some(characters),
            Err(error) => { eprintln!("characters: using primitive fallback: {error}"); None }
        }};
        self.first = scene.instances.len();
        let eye_m = material(scene, [0.05, 0.05, 0.06], 0.3);
        for i in 0..players {
            let profile = colors.get(i).copied().unwrap_or_default();
            let body_m = material(scene, profile.shirt.unwrap_or(PLAYER_COLORS[i % 4]), 0.55);
            let head_m = profile.skin.map_or(body_m, |skin| material(scene, skin, 0.55));
            let prims = if crate::graphics::get().minimal_forward {[primitives::sphere(body_m,4,6),primitives::sphere(head_m,4,6),primitives::sphere(eye_m,3,4)]}else{[primitives::sphere(body_m, 10, 16), primitives::sphere(head_m, 8, 14), primitives::sphere(eye_m, 6, 8)]};
            let [proxy_body, head, eyes] = std::array::from_fn(|k| scene.add_mesh(&prims[k].mesh()));
            let actor = if rigid { None } else { self.characters.as_mut().and_then(|c| c.reserve_actor(scene, i).ok()) };
            // Rigid pieces share the skinned path but for how they are drawn.
            let skinned = actor.is_some() || (rigid && self.characters.is_some());
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
        let lava_m = scene.add_material(render::scene::Material {
            tint: [1.0, 0.20, 0.02], layer: -2, roughness: 1.0,
            flags: render::scene::LAVA, scale: 5000.0, metal: 0.0,
        });
        if let Some(characters) = &mut self.characters {
            if rigid { for i in 0..players { if let Err(e) = characters.reserve_rigid(scene, i) { eprintln!("characters: {e}"); } } }
            characters.reserve_lava(scene, lava_m);
        }
        self.lava_meshes = LavaMeshes {
            body: scene.add_mesh(&primitives::sphere(lava_m, 10, 16).mesh()),
            head: scene.add_mesh(&primitives::sphere(lava_m, 8, 14).mesh()),
            splash: scene.add_mesh(&primitives::cylinder(lava_m, 0.42, 1.0, 10).mesh()),
        };
    }

    /// Gib pieces shattered from the players' own meshes.
    pub fn reserve_gore(&mut self, scene: &mut Scene, seed: u64, cut: Option<[usize; 3]>) {
        // The minimum renderer's rigid pieces use the plain gib meshes.
        let models = self.characters.as_ref().filter(|_| !crate::graphics::get().rigid_characters).map_or(&[][..], |c| c.models());
        self.gore = Some(Gore::reserve(scene, &std::mem::take(&mut self.sources), models, seed, cut));
    }

    fn gore(&mut self) -> &mut Gore { self.gore.as_mut().expect("reserve_gore") }

    /// A new round: no corpses, no gore. Call before clearing the physics.
    pub fn clear(&mut self, scene: &mut Scene, phys: &mut Physics) {
        self.corpses.clear(scene);
        self.ragdolls.clear();
        self.corpse_hits = 0;
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

    pub fn hands(&self) -> &[Option<glam::Affine3A>] {
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
            if victim < self.meshes.len() { self.lava_death(scene, victim, v, fall, now, world.lava_surface_at(v.pos)); }
            return;
        }
        let blood = fx.meshes.blood;
        // Past the view's player capacity (a big coop wave): blood only.
        let Some(skinned) = self.meshes.get(victim).map(|m| m.skinned) else {
            fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..crate::blood::spray() });
            return;
        };
        let Some(a) = attacker else {
            fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..crate::blood::spray() });
            return;
        };
        let push = if dir.length_squared() > 1e-4 { dir } else { (v.pos - world.players[a].pos).with_y(0.0).normalize_or(Vec3::Y) };
        if matches!(gib, Gib::Head) {
            let head = v.hitboxes().into_iter().find(|(_, _, head)| *head).expect("head hitbox").0;
            self.gore().headshot(scene, phys, head, v.vel, push.normalize_or(Vec3::Y), now);
            fx.blasts.gib_mist(now, head, 0.42);
            fx.particles.burst(scene, head, push + Vec3::Y * 0.3, &Burst { mesh: blood, count: 9, spread: 1.1, speed: [2.0, 5.0], ..crate::blood::spray() });
            blood_splats(scene, arena, world, head, push, 3, 1.2);
            if skinned {
                if let Some(characters) = &mut self.characters {
                    if let Err(error) = characters.kill(phys, victim, v, push, true) { eprintln!("ragdoll: {error}"); }
                }
            } else { self.ragdoll(scene, phys, victim, v, push, false); }
            return;
        }
        // A burst (overkill, explosions) shatters skinned bodies as well: no
        // corpse, so the character stays hidden while the chunks fly.
        // Rigid characters have no shards of their own: the old primitive
        // pieces would fly out of a real body, so every death is a ragdoll.
        let rigid = crate::graphics::get().rigid_characters;
        if skinned && (rigid || !matches!(gib, Gib::Burst { .. })) {
            // Drive the original textured skin from articulated limb bodies.
            if let Some(characters) = &mut self.characters {
                if let Err(error) = characters.kill(phys, victim, v, push, false) { eprintln!("ragdoll: {error}"); }
            }
            fx.blasts.gib_mist(now, v.pos + Vec3::Y * 0.3, 0.8);
            fx.particles.burst(scene, v.pos, push + Vec3::Y * 0.5, &Burst { mesh: blood, count: 28, spread: 1.6, speed: [2.0, 8.0], ..crate::blood::spray() });
            blood_splats(scene, arena, world, v.pos, push, 4, 1.3);
            return;
        }
        match gib {
            Gib::Burst { severity, blast } => {
                // Godot: chaos from the severity, the whole body in chunks,
                // red spray, blood drops and splats on the ground around.
                let chaos = (severity * if blast { 0.75 } else { 0.82 }).clamp(0.35, 1.1);
                self.gore().burst(scene, phys, victim, &[gore::Part::Body, gore::Part::Head, gore::Part::Eyes], v.pos, v.yaw, v.vel, push, chaos, now);
                fx.blasts.gib_mist(now, v.pos + Vec3::Y * 0.2, 1.0 + chaos * 1.5);
                fx.particles.burst(scene, v.pos, push + Vec3::Y * 0.5, &Burst { mesh: blood, count: 28, spread: 1.8, speed: [3.0, 10.0], ..crate::blood::spray() });
                blood_splats(scene, arena, world, v.pos, push, (6.0 + chaos * 8.0) as usize, 1.6 + chaos * 1.4);
            }
            Gib::Head | Gib::Lava { .. } => unreachable!("handled above"),
            Gib::Ragdoll => {
                fx.particles.burst(scene, v.pos, Vec3::Y, &Burst { mesh: blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..crate::blood::spray() });
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
        let hands = std::array::from_fn(|_| self.corpses.add(scene, self.lava_meshes.head, Affine3A::from_translation(HIDDEN)));
        let c = LavaCorpse { victim, pos: v.pos.with_y(surface + growing_guns::HALF_HEIGHT * 0.65), yaw: v.yaw, pitch: v.pitch, camera_look: [0.;2], camera_prev: [0.;2], birth: now, fall, surface, twin, hands, blobs, splash };
        self.lava.push(c);
    }

    pub fn lava_look(&mut self, victim: usize, look: [f32; 2]) {
        if let Some(c) = self.lava.iter_mut().find(|c| c.victim == victim) {
            c.camera_prev = c.camera_look;
            c.camera_look[0] += look[0] * 0.3;
            c.camera_look[1] = (c.camera_look[1] + look[1] * 0.3).clamp(-0.65, 0.65);
        }
    }

    /// The victim follows exactly the same slow sink as the visible corpse.
    pub fn lava_camera(&self, world: &World, victim: usize, mut camera: render::Camera, clock: f32) -> render::Camera {
        if world.players[victim].alive || world.players[victim].howitzer.is_some() { return camera; }
        if let Some(c)=self.lava.iter().find(|c|c.victim==victim && clock-c.birth<2.4) {
            let t = (clock-c.birth).max(0.0);
            let alpha = (clock / growing_guns::DT - (world.tick as f32 - 1.0)).clamp(0.0, 1.0);
            let direction = c.camera_direction(t, alpha);
            camera.pos=Vec3::new(c.pos.x,c.y((clock-c.birth).max(0.0))+growing_guns::EYE,c.pos.z);
            camera.target=camera.pos+direction;
        }
        camera
    }

    pub fn burn_lava_bodies(&self, world: &World, fire: &mut render::fx::Effects, now: f32) {
        if world.tick % 5 != 0 || crate::graphics::get().minimal_forward { return; }
        for c in &self.lava {
            let age=now-c.birth;
            if !(0.0..2.4).contains(&age) || world.players[c.victim].alive {continue;}
            let center=Vec3::new(c.pos.x,c.surface,c.pos.z);
            for k in 0..6 {
                let a=k as f32*2.399963+(world.tick/5) as f32*1.37;
                let radius=0.45+0.12*((k as f32*7.1+age*11.0).sin());
                fire.burning_body(now,center+Vec3::new(a.cos()*radius,0.0,a.sin()*radius));
            }
        }
    }

    /// Godot `end_lava_death`: player `victim`'s lava body goes (a revive).
    fn end_lava(&mut self, scene: &mut Scene, victim: usize) {
        let Some(k) = self.lava.iter().position(|c| c.victim == victim) else { return };
        let c = self.lava.swap_remove(k);
        if let Some((instance, mesh)) = c.twin { pose_instance(scene, instance, mesh, Affine3A::from_translation(HIDDEN)); }
        for slot in c.hands.into_iter().flatten() { self.corpses.remove(scene, slot); }
        for (slot, ..) in c.blobs { self.corpses.remove(scene, slot); }
        if let Some(slot) = c.splash { self.corpses.remove(scene, slot); }
    }

    /// Sinking lava bodies and their splashes at `clock` seconds.
    fn pose_lava(&mut self, scene: &mut Scene, world: &World, clock: f32, guns: &super::guns::Guns) {
        let finished: Vec<_> = self.lava.iter().filter(|c| clock - c.birth >= 2.4 || world.players.get(c.victim).is_none_or(|p| p.alive)).map(|c| c.victim).collect();
        for i in finished { self.end_lava(scene, i); }
        let rot = Quat::from_rotation_y(0.0);
        for c in &mut self.lava {
            let t = (clock - c.birth).max(0.0);
            let at = Vec3::new(c.pos.x, c.y(t), c.pos.z);
            let wrists = c.wrists(t);
            let owner = if world.players[c.victim].howitzer.is_none() { self.meshes[c.victim].own_views } else { 0 };
            if let Some(meshes) = guns.open_hand_meshes(c.victim) {
                // Other views see the articulated body, or the same open hands on the simple body.
                let hidden = if c.twin.is_some() { !owner } else { 0 };
                for ((slot, mesh), wrist) in c.hands.iter().zip(meshes).zip(wrists) {
                    if let Some(slot) = *slot {
                        self.corpses.set(scene, slot, mesh, wrist);
                        scene.instances[self.corpses.instance(slot)].hide_in_views(hidden);
                    }
                }
            }
            if let Some((instance, mesh)) = c.twin {
                let body = characters::model_transform(at, c.yaw);
                if let Some(characters) = &mut self.characters {
                    characters.pose_lava_twin(c.victim, wrists.map(|w| body.inverse().transform_point3(w.translation.into())));
                }
                pose_instance(scene, instance, mesh, body);
                scene.instances[instance].hide_in_views(owner);
            }
            for &(slot, mesh, scale, offset) in &c.blobs {
                self.corpses.set(scene, slot, mesh, Affine3A::from_scale_rotation_translation(scale, Quat::from_rotation_y(c.yaw), at + Vec3::Y * offset));
            }
            for &(slot, ..) in &c.blobs {
                scene.instances[self.corpses.instance(slot)].hide_in_views(owner);
            }
            if let Some(slot) = c.splash {
                scene.instances[self.corpses.instance(slot)].hide_in_views(owner);
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
        for &(_, slot, ..) in &parts { scene.instances[self.corpses.instance(slot)].hide_in_views(0); }
        self.ragdolls.push(Ragdoll { torso, parts });
    }

    /// One tick after the physics step: corpses and gore follow their bodies.
    pub fn follow_physics(&mut self, scene: &mut Scene, phys: &mut Physics, now: f32, splat:impl FnMut(&mut Scene,Vec3,Vec3,Vec3)) {
        self.gore().update(scene, phys, now,splat);
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
    pub fn pose(&mut self, scene: &mut Scene, world: &World, prev: &[crate::view::Pose], alpha: f32, guns: &super::guns::Guns) {
        if let Some(characters) = &mut self.characters { characters.pose(world, prev, alpha, Some(guns)); }
        let mut slot = self.first;
        for (i, m) in self.meshes.iter().enumerate() {
            let Some(_p) = world.players.get(i).filter(|p| p.alive) else {
                // Rigid characters draw their corpse themselves.
                let corpse = world.players.get(i).filter(|_| !crate::graphics::get().rigid_characters).and_then(|_| self.characters.as_ref().and_then(|c| c.corpse_transform(i)));
                for (part, mesh) in [m.body, m.head, m.eyes].into_iter().enumerate() {
                    let xf = if part == 0 { corpse.unwrap_or(Affine3A::from_translation(HIDDEN)) } else { Affine3A::from_translation(HIDDEN) };
                    pose_instance(scene, slot, mesh, xf);
                    scene.instances[slot].hide_in_views(0);
                    slot += 1;
                }
                continue;
            };
            let drawn = crate::view::pose(world, i, prev, alpha);
            let pos = drawn.pos + Vec3::Y * guns.crowd_lift(i, world.entrance_flight_at(alpha));
            let rot = Quat::from_rotation_y(drawn.yaw);
            let (body, head, eyes) = if m.skinned {
                (if crate::graphics::get().rigid_characters {Affine3A::from_translation(HIDDEN)} else {characters::model_transform(pos, drawn.yaw)}, Affine3A::from_translation(HIDDEN), Affine3A::from_translation(HIDDEN))
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
        if crate::graphics::get().rigid_characters && let Some(characters) = &mut self.characters {
            let hide: Vec<u8> = self.meshes.iter().map(|m| m.own_views).collect();
            characters.pose_rigid(world, prev, alpha, &hide, guns);
        }
        self.pose_lava(scene, world, (world.tick as f32 - 1.0 + alpha) * growing_guns::DT, guns);
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
                    if head.distance(tail) > 0.05 { self.trails.extend(bullet_trail(head, tail, scale, w.bullet_color)); }
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
            m.own_views = render::scene::PlayerViews::for_player(slots, i).hide_from_owner();
            let first = self.first + i * PARTS;
            for part in &mut scene.instances[first..first + PARTS] {
                part.hide_in_views(m.own_views);
            }
        }
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn shoot_corpses(&mut self, phys: &mut Physics, from: Vec3, to: Vec3, damage: f32) -> Option<(Vec3, Vec3)> {
        let mut bodies = Vec::new();
        if let Some(characters) = &self.characters { characters.corpse_bodies(&mut bodies); }
        for doll in &self.ragdolls { bodies.extend(doll.parts.iter().map(|p| p.0)); }
        let (body, point, normal) = phys.raycast_bodies(from, to, &bodies)?;
        self.corpse_hits += 1;
        // Contact impulses spin struck limbs and pull the connected body.
        let strength = (damage.max(0.0) * 0.8).clamp(0.5, 40.0);
        phys.impulse(body, (to-from).normalize_or(Vec3::ZERO)*strength, Some(point));
        Some((point, normal))
    }

    pub fn ragdolls(&self, phys: &Physics) -> Vec<[f32; 3]> {
        let mut positions: Vec<_> = self.ragdolls.iter().filter_map(|r| phys.pose(r.torso)).map(|(p, _)| p.to_array()).collect();
        if let Some(c) = &self.characters { positions.extend(c.ragdolls()); }
        positions
    }

    pub fn gore_len(&self) -> usize { self.gore.as_ref().map_or(0, |g| g.len()) }
}

/// Blood splats on the ground around a burst body (Godot
/// `spawn_overkill_blood_splats_after_destruction`).
fn blood_splats(
    scene: &mut Scene,
    arena: &mut ArenaView,
    world: &World,
    at: Vec3,
    travel: Vec3,
    n: usize,
    reach: f32,
) {
    let mut bits = (world.tick as u32) ^ at.x.to_bits().rotate_left(7) ^ at.z.to_bits();
    let mut unit = || {
        bits = bits.wrapping_mul(1664525).wrapping_add(1013904223);
        (bits >> 8) as f32 / 16777216.
    };
    let axis = travel.normalize_or(Vec3::Y);
    for k in 0..n {
        let angle = unit() * std::f32::consts::TAU;
        let scatter = Vec3::new(angle.cos(), 0., angle.sin()) * unit();
        let direction =
            (axis * unit() + scatter - Vec3::Y * (0.22 + 1.4 * unit())).normalize_or(Vec3::NEG_Y);
        let from = at + Vec3::Y * 0.15;
        let Some((t, normal, c)) =
            growing_guns::shape::raycast(&world.arena.grid, from, direction, reach + 3.)
        else {
            continue;
        };
        let size = (0.18 + 0.48 * unit()) * if k == 0 { 1.45 } else { 1. };
        arena.blood(
            scene,
            &world.arena.grid.boxes[c],
            from + direction * t,
            normal,
            direction,
            size,
            c as u32,
        );
    }
}

#[cfg(test)] mod trail_tests {
    use super::*;
    #[test] fn minimum_trail_texture_is_visible_in_both_direct_srgb_eyes() {
        let gpu=render::Gpu::new().unwrap();let mut scene=Scene::default();scene.graphics.minimal_forward=true;
        let material=scene.add_material(render::scene::Material{tint:[0.2;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
        let wall=scene.add_mesh(&render::primitives::quad(material).mesh());
        scene.add_instance(wall,Affine3A::from_scale_rotation_translation(Vec3::splat(50.),Quat::IDENTITY,Vec3::new(0.,0.,-40.)));
        let light=render::Lighting{sun_lux:0.,sky_lux:500.,ev100:12.,..Default::default()};
        let renderer=render::Renderer::new(&gpu,&scene,&light);
        renderer.set_sprite_image(&gpu,TRAIL_IMAGE,&trail_picture(),render::hud::ImageFilter::Linear);
        let trails=[bullet_trail(Vec3::new(-2.,1.8,-12.),Vec3::new(3.,0.5,-8.),1.,[1.,0.8,0.3]),
            bullet_trail(Vec3::new(0.3,1.2,-16.),Vec3::new(0.3,1.2,-3.),1.,[0.3,0.8,1.])];
        let trails:Vec<_>=trails.into_iter().flatten().collect();
        renderer.set_sprite_image(&gpu,super::super::effects::FLASH_IMAGE,&super::super::effects::flash_picture(),render::hud::ImageFilter::Linear);
        let eyes:[wgpu::Texture;2]=std::array::from_fn(|_|gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("trail stereo fixture"),size:wgpu::Extent3d{width:640,height:640,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]}));
        let views=[-0.032,0.032].map(|x|render::ViewRect{cam:render::Camera{pos:Vec3::new(x,1.6,0.),target:Vec3::new(x,1.6,-10.),fov_y:1.1,near:0.05},x:0,y:0,w:640,h:640});
        renderer.try_render_eye_targets(&gpu,&views,&light,&[&eyes[0],&eyes[1]]).unwrap();
        let baseline=eyes.each_ref().map(|eye|gpu.read_rgba8(eye));
        renderer.set_sprites(&gpu,&trails).unwrap();
        renderer.try_render_eye_targets(&gpu,&views,&light,&[&eyes[0],&eyes[1]]).unwrap();
        let images=eyes.each_ref().map(|eye|gpu.read_rgba8(eye));
        for i in 0..2 {
            let bright=images[i].chunks_exact(4).zip(baseline[i].chunks_exact(4)).filter(|(a,b)|a[..3].iter().zip(*b).any(|(a,b)|*a as i32-*b as i32>60)).count();
            assert!(bright>40,"bullet trails disappear in eye {i}: {bright}");
        }
        if let Ok(path)=std::env::var("WARP_TRAIL_CAPTURE") {
            let mut stereo=Vec::new();for y in 0..640 {for image in &images{stereo.extend_from_slice(&image[y*640*4..(y+1)*640*4]);}}
            render::write_png(std::path::Path::new(&path),1280,640,&stereo).unwrap();
        }

        // End-on distant shots remain round in both eyes, also looking up/down.
        // Old width-only sprites became flat lines as their height shrank.
        for pitch in [-0.7f32,0.,0.7] {
            let direction=Vec3::new(0.,pitch.sin(),-pitch.cos());
            let mut point=bullet_trail(Vec3::new(0.,1.6,0.)+direction*30.,Vec3::ZERO,1.,[1.,0.8,0.3])[1];
            point.size=[0.025;2]; // force minimum pixel size
            let tilted=views.map(|mut v|{v.cam.target=v.cam.pos+direction;v});
            renderer.set_sprites(&gpu,&[]).unwrap();renderer.try_render_eye_targets(&gpu,&tilted,&light,&[&eyes[0],&eyes[1]]).unwrap();
            let empty=eyes.each_ref().map(|eye|gpu.read_rgba8(eye));
            renderer.set_sprites(&gpu,&[point]).unwrap();renderer.try_render_eye_targets(&gpu,&tilted,&light,&[&eyes[0],&eyes[1]]).unwrap();
            for i in 0..2 {
                let image=gpu.read_rgba8(&eyes[i]);let mut bounds=[640usize,640,0,0];let mut count=0;
                for (at,(a,b)) in image.chunks_exact(4).zip(empty[i].chunks_exact(4)).enumerate() {
                    if a[..3].iter().zip(b).any(|(a,b)|*a as i32-*b as i32>40){let x=at%640;let y=at/640;bounds[0]=bounds[0].min(x);bounds[1]=bounds[1].min(y);bounds[2]=bounds[2].max(x);bounds[3]=bounds[3].max(y);count+=1;}
                }
                assert!(count>=12,"end-on glow missing at pitch {pitch}, eye {i}: {count}");
                let w=bounds[2]-bounds[0]+1;let h=bounds[3]-bounds[1]+1;
                assert!(w.abs_diff(h)<=1 && w>=4 && h>=4,"flattened glow {w}x{h} at pitch {pitch}");
            }
        }
    }
}

#[cfg(test)]
mod lava_sink_tests {
    use super::*;
    #[test]
    fn full_reserved_roster_keeps_actual_character_lava_twins() {
        let mut scene=Scene::default();let mut characters=Characters::load_roster(3, 8, &[]).unwrap();
        for i in 0..8 {characters.reserve_actor(&mut scene,i).unwrap();}
        let material=super::super::arena::lava_material(&mut scene);
        characters.reserve_lava(&mut scene,material);
        assert_eq!(characters.lava_instances().len(),8);
        for i in 0..8 {assert!(characters.lava_twin(i).is_some());}
    }

    #[test]
    fn lava_body_sinks_slowly_from_the_surface_then_fully_submerges() {
        let c=LavaCorpse {victim:0,pos:Vec3::new(2.0,-3.0+growing_guns::HALF_HEIGHT*0.65,4.0),
            yaw:0.0,pitch:0.0,camera_look:[0.;2],camera_prev:[0.;2],birth:0.0,fall:true,surface:-3.0,twin:None,hands:[None;2],blobs:vec![],splash:None};
        let start = c.wrists(0.);
        let raised = c.wrists(0.65);
        for side in 0..2 {
            assert!(raised[side].translation.y - c.y(0.65) > start[side].translation.y - c.y(0.) + 0.25);
        }
        assert!((raised[0].translation.y - raised[1].translation.y).abs() > 0.05);
        assert!((c.camera_direction(0.0, 1.0) - growing_guns::look_dir(c.yaw, c.pitch)).length() < 1e-6);
        assert!(c.camera_direction(2.2, 1.0).y > 0.7, "gaze rises during sinking");
        let mut controlled = LavaCorpse { camera_look: [0.3, -0.2], camera_prev: [0.0; 2], ..c };
        assert!((controlled.camera_direction(0.0, 0.0) - Vec3::NEG_Z).length() < 1e-6);
        assert!((controlled.camera_direction(0.0, 1.0) - growing_guns::look_dir(0.3, -0.2)).length() < 1e-6);
        controlled.camera_look = [0.; 2];
        let c = controlled;
        assert!(c.y(0.0)>c.surface);
        assert!(c.y(0.5)>c.surface,"slow sink, not gravity");
        let mut previous=c.y(0.0);
        for i in 1..=144 {let y=c.y(i as f32/60.0);assert!(y<=previous+1e-6);previous=y;}
        assert!(c.y(2.4)+growing_guns::EYE<c.surface);
    }
}
