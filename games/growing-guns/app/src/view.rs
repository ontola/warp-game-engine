//! Turns the simulation into a Warp scene: arena chunks as hewn stone
//! bricks with the castle palette (worn bricks crack, broken ones vanish),
//! lava, players, bullet tracers, sparks, scorch marks, rubble and lights,
//! plus one camera per local player.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::shape::{Boxes, Kind};
use growing_guns::{Event, World, EYE};
use render::primitives;
use render::debris::{BurstSpec, Debris, Occupancy};
use render::fx::Effects;
use render::particles::{Burst, Particles};
use render::slots::{SlotId, Slots};
use physics::{Body, BodyDesc, Physics, Shape, Static};
use render::headlights::LocalLight;
use render::scene::{GpuInstance, Material, EMISSIVE};
use render::{Camera, Lighting, Scene};
use crate::stadium::StadiumVisuals;
use crate::gun::{self, Rig, Stats};
use audio::sfx::Listener;

pub const TRACERS: usize = 64;
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);
pub const FOV_Y: f32 = 75.0 * std::f32::consts::PI / 180.0;

/// Godot palette "castle_grey" (linear-ish albedo, as authored there).
mod palette {
    pub const FLOOR: [f32; 3] = [0.49, 0.47, 0.44];
    pub const WALL: [f32; 3] = [0.56, 0.54, 0.50];
    pub const BUILDING: [f32; 3] = [0.60, 0.57, 0.53];
    pub const DARK: [f32; 3] = [0.42, 0.40, 0.38];
    pub const ACCENT_A: [f32; 3] = [0.95, 0.55, 0.22];
    pub const ACCENT_B: [f32; 3] = [0.82, 0.72, 0.38];
    pub const SKY_TOP: [f32; 3] = [0.22, 0.24, 0.28];
    pub const HORIZON: [f32; 3] = [0.48, 0.46, 0.44];
    pub const SUN: [f32; 3] = [1.0, 0.82, 0.58];
}

pub const PLAYER_COLORS: [[f32; 3]; 4] = [[0.85, 0.22, 0.12], [0.15, 0.38, 0.85], [0.20, 0.70, 0.25], [0.90, 0.75, 0.12]];
pub const PLAYER_COLORS_U8: [[u8; 4]; 4] = [[255, 110, 80, 255], [110, 160, 255, 255], [120, 225, 130, 255], [255, 220, 90, 255]];

fn srgb_to_linear(c: [f32; 3]) -> [f32; 3] { c.map(|v| v.powf(2.2)) }

fn cube(scene: &mut Scene, material: u32) -> usize { scene.add_mesh(&primitives::cube(material).mesh()) }
fn sphere(scene: &mut Scene, material: u32, rings: u32, segments: u32) -> usize { scene.add_mesh(&primitives::sphere(material, rings, segments).mesh()) }

fn material(scene: &mut Scene, tint: [f32; 3], roughness: f32) -> u32 {
    scene.add_material(Material { tint: srgb_to_linear(tint), layer: -1, roughness, flags: 0, scale: 1.0, metal: 0.0 })
}

fn glow(scene: &mut Scene, tint: [f32; 3], nits: f32) -> u32 {
    scene.add_material(Material { tint, layer: -1, roughness: 0.8, flags: EMISSIVE, scale: nits, metal: 0.0 })
}

struct PlayerMeshes { body: usize, head: usize, eyes: usize, gun: usize }

/// Scene plus the instance ranges the frame loop rewrites.
pub struct Visuals {
    pub scene: Scene,
    pub first_dynamic: usize,
    players: Vec<PlayerMeshes>,
    tracer: usize,
    fx: Particles,
    fxm: FxMeshes,
    corpses: Slots,
    phys: Physics,
    ragdolls: Vec<Ragdoll>,
    flashes: Vec<(Vec3, f32)>,
    lights: Vec<LocalLight>,
    arena: Slots,
    kinds: KindMeshes,
    /// Slot and collider of every arena chunk (None once broken).
    chunk_slots: Vec<Option<SlotId>>,
    statics: Vec<Option<Static>>,
    /// Wear level shown per chunk (0 intact, 1 cracked, 2 crumbling).
    shown_wear: Vec<u8>,
    marks: Marks,
    /// GPU rubble colliding with the arena's occupancy grid.
    debris: Debris,
    /// Explosions: fire, smoke, embers, heat shell, lights (render::fx).
    blasts: Effects,
    /// Seconds on the effects clock for the frame being drawn.
    clock: f32,
    /// Explosion view punch per player: position offset (camera local) and
    /// pitch/yaw kick, plus punches still travelling at the speed of sound.
    punch: Vec<(Vec3, glam::Vec2)>,
    pending_punch: Vec<(f32, Vec3, f32)>,
    /// The colosseum, its crowd and their voice.
    pub stadium: StadiumVisuals,
    /// The growing gun per player, its meshes and instance blocks.
    guns: Guns,
}

/// Instance blocks per player: the first-person gun (seen only in that
/// player's own view) and the third-person gun (seen by everyone else).
const GUN_PARTS: usize = 192;

struct Guns {
    /// Mesh per shape, then per material (barrel: one per heat level).
    meshes: Vec<Vec<usize>>,
    rigs: Vec<Rig>,
    feel: Vec<GunFeel>,
    /// First instance of each player's first-person and third-person block.
    fp: Vec<usize>,
    tp: Vec<usize>,
}

/// First-person gun feel (Godot `_update_gun_feel`): walk bob, jump bump,
/// strafe tilt, recoil kick, the card-growth inspect blend.
#[derive(Clone, Copy, Default)]
struct GunFeel { phase: f32, bob: f32, bump: f32, tilt: f32, kick: f32, inspect: f32 }

/// Bullet colour with explosive rounds (Godot: base yellow toward orange).
const BLAST_TINT: Vec3 = Vec3::new(1.0, 0.70, 0.20);

/// Bullet scorch marks: a ring of slots, oldest reused first; each knows
/// the chunk it sits on and goes when that chunk breaks.
struct Marks { slots: Slots, ring: Vec<(SlotId, u32)>, next: usize, mesh: usize }

struct Ragdoll { torso: Body, head: Body, slots: [SlotId; 2], meshes: [usize; 2] }

#[derive(Clone, Copy)]
struct FxMeshes { spark: usize, dust: usize, blood: usize, casing: usize, fire: usize, smoke: usize }

/// Stone per palette colour: four tint variants of the intact brick, then
/// cracked and crumbling bricks for worn chunks.
#[derive(Clone, Copy)]
struct Stone { intact: [usize; 4], cracked: usize, crumbling: usize, chip: usize, shards: [usize; 3] }

struct KindMeshes { floor: Stone, wall: Stone, building: Stone, dark: Stone, accent_a: Stone, accent_b: Stone, beacons: [usize; 3] }

impl KindMeshes {
    fn stone(&self, k: Kind) -> Stone {
        match k {
            Kind::Floor => self.floor,
            Kind::Wall => self.wall,
            Kind::Tower | Kind::Pillar => self.dark,
            Kind::Building => self.building,
            Kind::Ledge | Kind::Platform => self.accent_b,
            Kind::Cover => self.accent_a,
        }
    }
}

/// Godot's hairline mortar gap between bricks.
const VISUAL_CHUNK_INSET: f32 = 0.992;
const MARKS: usize = 384;
/// Godot: MAX_LIVE_PARTICLES 3600 at full quality.
const DEBRIS: usize = 4096;
/// Occupancy bounds: the largest arena (95 m) plus margin, lava to roofs.
const OCC_HALF: f32 = 52.0;
const OCC_VOXEL: f32 = 0.5;

/// The largest generated arena has about 9400 chunks.
const ARENA_SLOTS: usize = 9800;
/// The Godot arena's light colours (towers and building roofs).
const BEACON_COLORS: [[f32; 3]; 3] = [[1.0, 0.4, 0.3], [1.0, 0.35, 0.2], [1.0, 0.7, 0.2]];
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|i| (a[i] - b[i]).powi(2)).sum() }


impl Visuals {
    pub fn build(world: &World) -> Self {
        let mut scene = Scene::default();
        scene.sky = Some(sky());
        let kinds = KindMeshes {
            floor: stone(&mut scene, palette::FLOOR, 0.9),
            wall: stone(&mut scene, palette::WALL, 0.85),
            building: stone(&mut scene, palette::BUILDING, 0.85),
            dark: stone(&mut scene, palette::DARK, 0.9),
            accent_a: stone(&mut scene, palette::ACCENT_A, 0.7),
            accent_b: stone(&mut scene, palette::ACCENT_B, 0.7),
            beacons: BEACON_COLORS.map(|c| { let m = glow(&mut scene, c, 6000.0); cube(&mut scene, m) }),
        };
        // Lava: a glowing sheet under the floor, visible through holes and
        // past the edge of open arenas.
        let lava_m = glow(&mut scene, [1.0, 0.28, 0.04], 2600.0);
        let lava = cube(&mut scene, lava_m);
        scene.add_instance(lava, Affine3A::from_scale_rotation_translation(Vec3::new(900.0, 0.2, 900.0), Quat::IDENTITY, Vec3::new(0.0, -2.3, 0.0)));
        scene.mark_no_shadow();
        // The arena lives in slots: a new round refills them, no rebuild.
        let arena = Slots::reserve(&mut scene, ARENA_SLOTS);
        let mark_m = material(&mut scene, [0.06, 0.055, 0.05], 0.95);
        let marks = Marks { slots: Slots::reserve(&mut scene, MARKS), ring: Vec::new(), next: 0, mesh: sphere(&mut scene, mark_m, 4, 8) };
        // Effects: engine particles with their own meshes.
        let fxm = FxMeshes {
            spark: { let m = glow(&mut scene, [1.0, 0.6, 0.2], 20000.0); cube(&mut scene, m) },
            dust: { let m = material(&mut scene, palette::DARK, 0.95); cube(&mut scene, m) },
            blood: { let m = material(&mut scene, [0.45, 0.02, 0.02], 0.35); sphere(&mut scene, m, 4, 6) },
            casing: { let m = scene.add_material(Material { tint: [0.75, 0.55, 0.2], layer: -1, roughness: 0.35, flags: 0, scale: 1.0, metal: 1.0 }); cube(&mut scene, m) },
            fire: { let m = glow(&mut scene, [1.0, 0.45, 0.12], 9000.0); sphere(&mut scene, m, 6, 10) },
            smoke: { let m = material(&mut scene, [0.16, 0.15, 0.14], 1.0); sphere(&mut scene, m, 6, 10) },
        };
        let fx = Particles::reserve(&mut scene, 768, world.arena.seed | 1);
        let occ = Occupancy::new(Vec3::new(-OCC_HALF, -4.0, -OCC_HALF), Vec3::new(OCC_HALF, 30.0, OCC_HALF), OCC_VOXEL);
        let debris = Debris::reserve(&mut scene, DEBRIS, occ, false);
        // Ragdolls: two jointed bodies per corpse, drawn with the player's meshes.
        let corpses = Slots::reserve(&mut scene, 32);
        let phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), growing_guns::TICK_HZ as f32);
        let stadium = StadiumVisuals::reserve(&mut scene);

        let first_dynamic = scene.instances.len();
        let eye_m = material(&mut scene, [0.05, 0.05, 0.06], 0.3);
        let gun_m = material(&mut scene, [0.12, 0.12, 0.13], 0.4);
        let mut players = Vec::new();
        for i in 0..world.players.len() {
            let body_m = material(&mut scene, PLAYER_COLORS[i % 4], 0.55);
            let body = sphere(&mut scene, body_m, 10, 16);
            let head = sphere(&mut scene, body_m, 8, 14);
            let eyes = sphere(&mut scene, eye_m, 6, 8);
            let gun = cube(&mut scene, gun_m);
            for m in [body, head, eyes, gun] { scene.add_instance(m, Affine3A::from_translation(HIDDEN)); }
            players.push(PlayerMeshes { body, head, eyes, gun });
        }
        let tracer_m = glow(&mut scene, [1.0, 0.9, 0.3], 30000.0);
        let tracer = cube(&mut scene, tracer_m);
        for _ in 0..TRACERS { scene.add_instance(tracer, Affine3A::from_translation(HIDDEN)); scene.mark_no_shadow(); }
        let guns = Guns::reserve(&mut scene, world.players.len());

        let mut v = Visuals { scene, first_dynamic, players, tracer, fx, fxm, flashes: Vec::new(), lights: Vec::new(), arena, kinds,
            corpses, phys, ragdolls: Vec::new(), chunk_slots: Vec::new(), statics: Vec::new(), shown_wear: Vec::new(), marks, debris,
            blasts: Effects::default(), clock: 0.0, punch: Vec::new(), pending_punch: Vec::new(), stadium, guns };
        v.set_arena(world);
        v
    }

    /// Fill the arena slots from the world's current arena (a new round).
    /// Call `flush` afterwards to upload; the renderer stays.
    pub fn set_arena(&mut self, world: &World) {
        self.arena.clear(&mut self.scene);
        self.corpses.clear(&mut self.scene);
        self.ragdolls.clear();
        self.phys.clear_bodies();
        self.marks.slots.clear(&mut self.scene);
        self.marks.ring.clear();
        let a = &world.arena;
        // Chunks first (their handles are kept), then the stands.
        let mut alive: Vec<_> = a.chunks.iter().filter(|c| c.alive).map(|c| c.shape()).collect();
        alive.extend(a.stadium.collision());
        let mut handles = self.phys.set_statics(&alive).into_iter();
        self.statics = a.chunks.iter().map(|c| if c.alive { handles.next() } else { None }).collect();
        self.shown_wear = a.chunks.iter().map(|c| wear_level(c.wear())).collect();
        self.debris.clear();
        let occ = self.debris.occupancy_mut();
        occ.clear();
        for c in a.chunks.iter().filter(|c| c.alive) { occ.fill_box(c.center, c.half, c.yaw, true); }
        self.chunk_slots = Vec::with_capacity(a.chunks.len());
        for (i, c) in a.chunks.iter().enumerate() {
            let slot = c.alive.then(|| {
                let (mesh, xf) = self.chunk_look(c, self.shown_wear[i]);
                self.arena.add(&mut self.scene, mesh, xf).expect("arena slots")
            });
            self.chunk_slots.push(slot);
        }
        let k = &self.kinds;
        // Roof beacons where the Godot lights hang, in the nearest colour.
        for &(p, c) in &world.arena.lights {
            let nearest = (0..BEACON_COLORS.len()).min_by(|&a, &b| dist(BEACON_COLORS[a], c).total_cmp(&dist(BEACON_COLORS[b], c))).unwrap();
            self.arena.add(&mut self.scene, k.beacons[nearest], Affine3A::from_scale_rotation_translation(Vec3::splat(0.5), Quat::IDENTITY, p)).expect("arena slots");
        }
        self.stadium.set_arena(&mut self.scene, world);
        self.lights = world.arena.lights.iter().take(12).map(|&(p, c)| omni(p, Vec3::from(c) * 900.0, 20.0)).collect();
    }

    /// Upload changed arena slots and particles.
    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        self.arena.flush(gpu, renderer, &self.scene)?;
        self.corpses.flush(gpu, renderer, &self.scene)?;
        self.marks.slots.flush(gpu, renderer, &self.scene)?;
        self.debris.flush(gpu, renderer)?;
        self.blasts.flush(gpu, renderer, self.clock)?;
        self.stadium.flush(gpu, renderer, &self.scene, self.clock)?;
        self.fx.flush(gpu, renderer, &self.scene)
    }

    /// Effects from this tick's events, then one tick of particle motion
    /// (colliding with the arena). `ears` place the crowd's reactions.
    pub fn absorb(&mut self, world: &World, ears: &[Listener]) {
        self.break_chunks(world);
        let (s, m) = (&mut self.scene, self.fxm);
        for e in &world.events {
            match *e {
                Event::Impact { pos, normal, damage, strong } => {
                    self.fx.burst(s, pos + normal * 0.03, normal, &Burst { mesh: m.spark, count: 8, ..Burst::sparks() });
                    self.fx.burst(s, pos + normal * 0.05, normal, &Burst { mesh: m.dust, count: 5, ..Burst::dust() });
                    // Chips fly off every hit, more and bigger for heavy rounds
                    // (Godot `request_chips`), as GPU debris.
                    let chunk = world.arena.grid.ray(pos + normal * 0.05, -normal, 0.3).map(|h| h.2);
                    let stone = chunk.map_or(self.kinds.floor, |c| world.arena.chunk(c).map_or(self.kinds.floor, |ch| self.kinds.stone(ch.kind)));
                    let scale = (0.06 + damage * 0.008).clamp(0.06, 1.0);
                    let amount = ((16.0 * scale).round() as u32).clamp(2, 20);
                    let size = 2.4 * (0.05 + damage * 0.0022).clamp(0.05, 0.35);
                    let radius = growing_guns::destruct::rifle_chip(damage / 16.0).1;
                    self.debris.burst(s, &BurstSpec::chips(pos + normal * 0.15, radius * 0.3, amount, size, &stone.shards));
                    // A scorch mark on the stone (decal), bigger for strong rounds.
                    if let Some(c) = chunk {
                        let r = if strong { 0.32 } else { 0.22 };
                        let rot = Quat::from_rotation_arc(Vec3::Y, normal);
                        let xf = Affine3A::from_scale_rotation_translation(Vec3::new(r, 0.02, r), rot, pos);
                        self.marks.place(s, xf, c as u32);
                    }
                }
                Event::Explosion { pos, radius, .. } => {
                    // Godot's spawn_bullet_blast: billowing fire and smoke,
                    // flame tongues, embers, heat shell, flash and glow lights.
                    let now = world.tick as f32 * growing_guns::DT;
                    self.blasts.explosion(now, pos, radius, BLAST_TINT);
                    // View punch for everyone within five radii, after the
                    // sound has travelled.
                    for (i, p) in world.players.iter().enumerate() {
                        let d = p.pos.distance(pos);
                        if p.alive && d < (radius * 5.0).max(radius + 1.0) { self.pending_punch.push((now + d / 343.0, pos, radius)); let _ = i; }
                    }
                    self.phys.blast(pos, radius, 9.0);
                }
                Event::Hit { victim, attacker, pos, head, .. } => {
                    let dir = (world.players[victim].pos - world.players[attacker].pos).normalize_or(Vec3::Y);
                    self.fx.burst(s, pos, dir + Vec3::Y * 0.3, &Burst { mesh: m.blood, count: if head { 22 } else { 12 }, ..Burst::drops() });
                }
                Event::Kill { victim, attacker, .. } => {
                    let v = &world.players[victim];
                    self.fx.burst(s, v.pos, Vec3::Y, &Burst { mesh: m.blood, count: 40, spread: 1.6, speed: [2.0, 7.0], ..Burst::drops() });
                    // Shot dead: pushed away from the shooter. Lava: no body.
                    if let Some(a) = attacker {
                        let push = (v.pos - world.players[a].pos).with_y(0.0).normalize_or_zero() * 5.0 + Vec3::Y * 2.0;
                        let rot = Quat::from_rotation_y(v.yaw);
                        let torso = self.phys.add(&BodyDesc { rot, vel: v.vel + push, density: 300.0, ..BodyDesc::new(Shape::Capsule(0.12, 0.4), v.pos - Vec3::Y * 0.25) });
                        let head = self.phys.add(&BodyDesc { rot, vel: v.vel + push * 1.4, density: 300.0, ..BodyDesc::new(Shape::Sphere(0.28), v.pos + Vec3::Y * 0.55) });
                        self.phys.ball_joint(torso, head, Vec3::Y * 0.52, Vec3::Y * -0.28);
                        let pm = &self.players[victim];
                        let (b, h) = (pm.body, pm.head);
                        let ts = self.corpses.add(s, b, Affine3A::from_translation(v.pos));
                        let hs = self.corpses.add(s, h, Affine3A::from_translation(v.pos));
                        if let (Some(ts), Some(hs)) = (ts, hs) { self.ragdolls.push(Ragdoll { torso, head, slots: [ts, hs], meshes: [b, h] }); }
                    }
                }
                Event::Picked { player, .. } => self.guns.rigs[player].grow_to(Stats::of(&world.players[player])),
                Event::Jump { player } => self.guns.feel[player].bump = 0.06,
                Event::Shot { player, from, dir } => {
                    let stats = Stats::of(&world.players[player]);
                    self.guns.rigs[player].fire(&stats);
                    let f = &mut self.guns.feel[player];
                    f.kick = f.kick.max(0.08 * world.players[player].mods.recoil.clamp(0.2, 2.5));
                    self.flashes.push((from + dir * 0.8, 0.06));
                    let p = &world.players[player];
                    let (sy, cy) = p.yaw.sin_cos();
                    let right = Vec3::new(cy, 0.0, -sy);
                    self.fx.burst_with(s, from + right * 0.35 - Vec3::Y * 0.25 + dir * 0.3, right + Vec3::Y * 0.6, p.vel, &Burst { mesh: m.casing, ..Burst::debris() });
                }
                _ => {}
            }
        }
        let now = world.tick as f32 * growing_guns::DT;
        self.stadium.absorb(&mut self.scene, &mut self.fx, &mut self.blasts, world, ears, now, growing_guns::DT);
        let s = &mut self.scene;
        self.guns.tick(world, growing_guns::DT);
        self.phys.step();
        for r in &self.ragdolls {
            for (body, slot, mesh, scale) in [(r.torso, r.slots[0], r.meshes[0], Vec3::new(0.42, 0.5, 0.42)), (r.head, r.slots[1], r.meshes[1], Vec3::splat(0.3))] {
                if let Some((pos, rot)) = self.phys.pose(body) {
                    self.corpses.set(s, slot, mesh, Affine3A::from_scale_rotation_translation(scale, rot, pos));
                }
            }
        }
        let shapes = &world.arena.grid;
        self.fx.update(s, growing_guns::DT, |a, b| {
            let d = b - a;
            let len = d.length();
            if len < 1e-6 { return None; }
            growing_guns::shape::raycast(shapes, a, d / len, len).map(|(t, n, _)| (a + d / len * t, n))
        });
    }

    /// Rewrite the dynamic instances. `alpha` blends from the previous tick.
    pub fn pose(&mut self, world: &World, prev: &[Vec3], alpha: f32, dt: f32) {
        self.clock = (world.tick as f32 - 1.0 + alpha) * growing_guns::DT;
        self.update_punch(world, dt);
        let mut slot = self.first_dynamic;
        for (i, p) in world.players.iter().enumerate() {
            let m = &self.players[i];
            if !p.alive {
                for mesh in [m.body, m.head, m.eyes, m.gun] { self.scene.update_instance(slot, mesh, Affine3A::from_translation(HIDDEN)); slot += 1; }
                continue;
            }
            let pos = prev.get(i).map_or(p.pos, |&q| q.lerp(p.pos, alpha));
            let rot = Quat::from_rotation_y(p.yaw);
            let body = Affine3A::from_scale_rotation_translation(Vec3::new(0.42, 0.5, 0.42), rot, pos + Vec3::Y * -0.25);
            let head = Affine3A::from_scale_rotation_translation(Vec3::splat(0.3), rot, pos + Vec3::Y * 0.55);
            let eyes = Affine3A::from_scale_rotation_translation(Vec3::new(0.2, 0.07, 0.05), rot, pos + Vec3::Y * 0.6 + rot * Vec3::new(0.0, 0.0, -0.27));
            let aim = rot * Quat::from_rotation_x(p.pitch);
            let gun = Affine3A::from_scale_rotation_translation(Vec3::new(0.09, 0.12, 0.7), aim, pos + Vec3::Y * 0.05 + rot * Vec3::new(0.38, 0.0, -0.3));
            let _ = gun;
            for (mesh, xf) in [(m.body, body), (m.head, head), (m.eyes, eyes), (m.gun, Affine3A::from_translation(HIDDEN))] { self.scene.update_instance(slot, mesh, xf); slot += 1; }
        }
        // Tracers: a short glowing streak behind each live bullet.
        for t in 0..TRACERS {
            // Skip the first metres so a player's own tracer never fills
            // their screen; the streak then grows to four metres.
            let xf = world.bullets.get(t).filter(|b| b.travelled > 1.0).map_or(Affine3A::from_translation(HIDDEN), |b| {
                let dir = b.vel.normalize();
                let len = (b.travelled - 1.0).clamp(0.3, 4.0);
                let rot = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
                Affine3A::from_scale_rotation_translation(Vec3::new(0.035, 0.035, len), rot, b.pos - dir * len * 0.5)
            });
            self.scene.update_instance(slot, self.tracer, xf);
            slot += 1;
        }
        self.flashes.retain_mut(|f| { f.1 -= dt; f.1 > 0.0 });
        self.pose_guns(world, prev, alpha, dt);
    }

    /// Morph a player's gun into their current stats (a card was given).
    pub fn grow_gun(&mut self, i: usize, p: &growing_guns::Player) { self.guns.rigs[i].grow_to(Stats::of(p)); }

    /// The growing guns: each player's rig follows their stats, then is drawn
    /// in first person (own view) and third person (everyone else's).
    fn pose_guns(&mut self, world: &World, prev: &[Vec3], alpha: f32, dt: f32) {
        let Guns { meshes, rigs, feel, fp: fps, tp: tps } = &self.guns;
        let _ = dt;
        for (i, p) in world.players.iter().enumerate() {
            let rig = &rigs[i];
            let heat = rig.heat_level();
            let f = &feel[i];
            let bob = f.bob;
            let reload = if p.reloading() { 1.0 - p.reload_timer * p.mods.reload / growing_guns::RELOAD_TIME } else { 0.0 };
            let (r_off, r_tilt, mag_drop) = if p.reloading() { gun::reload_pose(reload) } else { (Vec3::ZERO, 0.0, 0.0) };
            let pull = ((rig.build.barrel_length - 0.5) * 0.3).clamp(0.0, 0.3);
            let cam = punched_camera(world, i, prev, alpha, &self.punch);
            let fwd = (cam.target - cam.pos).normalize_or(Vec3::NEG_Z);
            let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
            let up = right.cross(fwd);
            let view = Affine3A::from_cols(right.into(), up.into(), (-fwd).into(), cam.pos.into());
            let muzzle = Vec3::new(0.3 + (f.phase * 0.5).sin() * 0.007 * bob, -0.3 + f.phase.sin() * 0.012 * bob - f.bump, -0.8 + f.kick + pull) + r_off;
            let s = f.inspect;
            let fp = view * Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::YXZ, 0.0, r_tilt, f.tilt), muzzle)
                * Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::YXZ, 1.0 * s, 0.12 * s, 0.08 * s), Vec3::new(-0.14, 0.04, 0.14) * s);
            // Third person: in the right hand, aimed with the head.
            let pos = prev.get(i).map_or(p.pos, |&q| q.lerp(p.pos, alpha));
            let rot = Quat::from_rotation_y(p.yaw);
            let tp = Affine3A::from_rotation_translation(rot * Quat::from_rotation_x(p.pitch), pos + Vec3::Y * 0.05 + rot * Vec3::new(0.45, 0.0, -0.12));
            for (first, xf, drop) in [(fps[i], fp, mag_drop), (tps[i], tp, 0.0)] {
                let mut n = 0;
                if p.alive {
                    for (part, local) in rig.pose(drop) {
                        if n == GUN_PARTS { break; }
                        self.scene.update_instance(first + n, gun_mesh(meshes, part.shape, part.mat, heat), xf * local);
                        n += 1;
                    }
                }
                for k in n..GUN_PARTS {
                    let inst = &mut self.scene.instances[first + k];
                    if inst.cluster_count == 0 { break; }
                    inst.cluster_count = 0;
                    inst.sphere = [0.0; 4];
                }
            }
        }
    }

    /// First person: hide each local player's own model from their view
    /// (`slots[k]` is drawn in view k). It still casts its shadow.
    pub fn hide_own_bodies(&mut self, slots: &[usize]) {
        for (k, &s) in slots.iter().enumerate().take(8) {
            for part in 0..4 { self.scene.instances[self.first_dynamic + s * 4 + part].hide_in_views(1 << k); }
            for n in 0..GUN_PARTS {
                self.scene.instances[self.guns.tp[s] + n].hide_in_views(1 << k);
                self.scene.instances[self.guns.fp[s] + n].hide_in_views(!(1u8 << k));
            }
        }
    }

    /// Mesh and transform for a chunk at a wear level.
    fn chunk_look(&self, c: &growing_guns::Chunk, wear: u8) -> (usize, Affine3A) {
        let st = self.kinds.stone(c.kind);
        let mesh = match wear { 0 => st.intact[(c.seed % 4) as usize], 1 => st.cracked, _ => st.crumbling };
        // Worn bricks shrink a little and sit askew, like Godot's deform carve.
        let shrink = [1.0, 0.97, 0.92][wear as usize];
        let tilt = if wear == 0 { Quat::IDENTITY } else {
            let r = |k: u32| ((c.seed >> k) & 0xff) as f32 / 255.0 - 0.5;
            Quat::from_euler(glam::EulerRot::XYZ, r(0) * 0.05 * wear as f32, r(8) * 0.05 * wear as f32, r(16) * 0.05 * wear as f32)
        };
        // The mortar gap is horizontal only: courses sit flush, so the bright
        // lava never shows through a seam under a wall.
        (mesh, Affine3A::from_scale_rotation_translation(c.half * 2.0 * Vec3::new(VISUAL_CHUNK_INSET, 1.0, VISUAL_CHUNK_INSET) * shrink, Quat::from_rotation_y(c.yaw) * tilt, c.center))
    }

    /// This tick's breakage: worn bricks change look, broken ones and their
    /// marks and colliders go, rubble flies.
    fn break_chunks(&mut self, world: &World) {
        self.debris.advance(growing_guns::DT);
        let b = &world.breakage;
        if b.is_empty() { return; }
        for &(i, _) in &b.worn {
            let c = &world.arena.chunks[i as usize];
            let lvl = wear_level(c.wear());
            if lvl != self.shown_wear[i as usize] {
                self.shown_wear[i as usize] = lvl;
                if let Some(slot) = self.chunk_slots[i as usize] {
                    let (mesh, xf) = self.chunk_look(c, lvl);
                    self.arena.set(&mut self.scene, slot, mesh, xf);
                }
            }
        }
        for &i in &b.removed {
            if let Some(slot) = self.chunk_slots[i as usize].take() { self.arena.remove(&mut self.scene, slot); }
            if let Some(h) = self.statics[i as usize].take() { self.phys.remove_static(h); }
            self.marks.drop_on(&mut self.scene, i);
            // Carve the chunk out of the debris grid, then restore voxels its
            // neighbours still fill.
            let c = world.arena.chunks[i as usize];
            let occ = self.debris.occupancy_mut();
            occ.fill_box(c.center, c.half, c.yaw, false);
            let r = c.shape().reach() + OCC_VOXEL;
            for j in world.arena.grid.query(glam::Vec2::new(c.center.x - r, c.center.z - r), glam::Vec2::new(c.center.x + r, c.center.z + r)) {
                let Some(n) = world.arena.chunk(j) else { continue };
                if (n.center.y - c.center.y).abs() <= n.half.y + c.half.y + OCC_VOXEL { occ.fill_box(n.center, n.half, n.yaw, true); }
            }
        }
        // Rubble from each blast (Godot `request_burst`): 70 shards per
        // broken chunk, 128 to 900, thrown from the blast centre.
        for burst in &b.bursts {
            let st = self.kinds.stone(burst.kind);
            let count = (burst.count * 70).clamp(128, 900);
            self.debris.burst(&self.scene, &BurstSpec::rubble(burst.center, burst.radius.max(1.0), count, &st.shards));
            self.fx.burst(&mut self.scene, burst.center, Vec3::Y, &Burst { mesh: self.fxm.dust, count: 12, spread: 1.5, speed: [0.5, 3.0], size: [0.9, 0.3], life: [0.8, 1.6], ..Burst::dust() });
        }
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn ragdolls(&self) -> Vec<[f32; 3]> {
        self.ragdolls.iter().filter_map(|r| self.phys.pose(r.torso)).map(|(p, _)| p.to_array()).collect()
    }

    pub fn dynamic(&self) -> &[GpuInstance] { &self.scene.instances[self.first_dynamic..] }

    /// Explosion view punch (Godot `do_explosion_view_punch`): pushed away
    /// from the blast, kicked in pitch and yaw, easing back at rate 12.
    fn update_punch(&mut self, world: &World, dt: f32) {
        self.punch.resize(world.players.len(), (Vec3::ZERO, glam::Vec2::ZERO));
        let now = self.clock;
        let due: Vec<_> = self.pending_punch.iter().filter(|p| p.0 <= now).copied().collect();
        self.pending_punch.retain(|p| p.0 > now);
        for (_, pos, radius) in due {
            for (i, p) in world.players.iter().enumerate() {
                let to = p.pos - pos;
                let dist = to.length();
                let reach = (radius * 5.0).max(radius + 1.0);
                if !p.alive || dist >= reach { continue; }
                let strength = (1.0 - dist / reach).clamp(0.0, 1.0) * (radius / 5.0).clamp(0.45, 1.6);
                let away = if dist > 1e-3 { to / dist } else { Vec3::Y };
                let local = Quat::from_rotation_y(p.yaw).inverse() * away;
                self.punch[i].0 += local * (0.09 + 0.33 * strength);
                self.punch[i].1 += glam::Vec2::new(-local.y * (0.045 + 0.165 * strength), local.x * (0.018 + 0.084 * strength));
            }
        }
        let k = (dt * 12.0).min(1.0);
        for p in &mut self.punch { p.0 = p.0.lerp(Vec3::ZERO, k); p.1 = p.1.lerp(glam::Vec2::ZERO, k); }
    }

    /// A player's camera with the explosion punch applied.
    pub fn punched(&self, world: &World, i: usize, cam: Camera) -> Camera { apply_punch(world, i, cam, &self.punch) }

    pub fn local_lights(&self) -> Vec<LocalLight> {
        // Blast lights first: they matter most while they last.
        let mut l = self.blasts.lights(self.clock);
        l.truncate(16);
        l.extend(self.stadium.lights().iter().take(16 - l.len()).copied());
        l.extend(self.lights.iter().take(16 - l.len()).copied());
        for &(p, _) in self.flashes.iter().rev().take(16 - l.len()) { l.push(omni(p, Vec3::new(1.0, 0.75, 0.35) * 600.0, 8.0)); }
        l
    }
}

impl Guns {
    /// One tick of gun state: rigs follow their stats (morphs, heat, bolt,
    /// spin) and the first-person feel eases (bob, bump, tilt, kick).
    fn tick(&mut self, world: &World, dt: f32) {
        for (i, p) in world.players.iter().enumerate() {
            let rig = &mut self.rigs[i];
            rig.follow(Stats::of(p));
            rig.update(dt);
            let f = &mut self.feel[i];
            let horiz = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
            f.bob = if p.on_floor && horiz > 0.5 { (horiz / growing_guns::WALK_SPEED).clamp(0.0, 1.4) } else { 0.0 };
            if f.bob > 0.0 { f.phase += horiz * dt / 2.2 * std::f32::consts::PI; }
            let k = (dt * 8.0).min(1.0);
            f.bump -= f.bump * k;
            f.kick -= f.kick * (dt * 14.0).min(1.0);
            let (sy, cy) = p.yaw.sin_cos();
            let lateral = (p.vel.dot(Vec3::new(cy, 0.0, -sy)) / growing_guns::WALK_SPEED).clamp(-1.0, 1.0);
            f.tilt += ((lateral * 3.5f32).to_radians() - f.tilt) * k;
            f.inspect += (if rig.is_growing() { 1.0 } else { 0.0 } - f.inspect) * (dt * 5.0).min(1.0);
        }
    }

    fn reserve(scene: &mut Scene, players: usize) -> Self {
        let mut meshes = Vec::new();
        for shape in gun::SHAPES {
            let mut row = Vec::new();
            for m in gun::MATS {
                let levels = if m == gun::Mat::Barrel { gun::HEAT_LEVELS } else { 1 };
                for h in 0..levels {
                    let mat = gun_material(scene, m, h);
                    let prim = match shape {
                        gun::Shape::Box => primitives::hewn_block(mat, 1, 0.1, 0.0, 0),
                        gun::Shape::Cyl => primitives::cylinder(mat, 1.0, 1.0, 8),
                        gun::Shape::Taper85 => primitives::cylinder(mat, 1.0, 0.85, 8),
                        gun::Shape::Taper105 => primitives::cylinder(mat, 1.0, 1.05, 8),
                        gun::Shape::Ring18 => primitives::torus(mat, 0.099, 10, 6),
                        gun::Shape::Ring25 => primitives::torus(mat, 0.143, 10, 6),
                    };
                    row.push(scene.add_mesh(&prim.mesh()));
                }
            }
            meshes.push(row);
        }
        let (mut fp, mut tp) = (Vec::new(), Vec::new());
        for _ in 0..players {
            let a = Slots::reserve(scene, GUN_PARTS).first;
            // Nobody sees a first-person gun until its player is local.
            for k in 0..GUN_PARTS { let i = &mut scene.instances[a + k]; i.hide_in_views(0xff); i.flags |= render::scene::NO_SHADOW; }
            fp.push(a);
            tp.push(Slots::reserve(scene, GUN_PARTS).first);
        }
        Guns { meshes, rigs: vec![Rig::default(); players], feel: vec![GunFeel::default(); players], fp, tp }
    }

}

fn gun_mesh(meshes: &[Vec<usize>], shape: gun::Shape, mat: gun::Mat, heat: usize) -> usize {
    let s = gun::SHAPES.iter().position(|&x| x == shape).unwrap();
    let mut k = 0;
    for m in gun::MATS {
        if m == mat { return meshes[s][k + if m == gun::Mat::Barrel { heat } else { 0 }]; }
        k += if m == gun::Mat::Barrel { gun::HEAT_LEVELS } else { 1 };
    }
    unreachable!()
}

/// Godot `_make_material` and `_make_polymer`: machined metal lifted toward
/// gunmetal, matte polymer; the barrel glows with heat.
fn gun_material(scene: &mut Scene, m: gun::Mat, heat: usize) -> u32 {
    use gun::Mat::*;
    let rc = [0.13f32, 0.14, 0.17];
    let metal = |scene: &mut Scene, col: [f32; 3], metallic: f32, rough: f32| {
        let lift = 0.06 + (0.3 - 0.06) * metallic;
        let gm = [0.62, 0.64, 0.68];
        let tint = [0, 1, 2].map(|i| col[i] + (gm[i] - col[i]) * lift);
        scene.add_material(Material { tint: srgb_to_linear(tint), layer: -1, roughness: (rough + 0.1).min(1.0), flags: 0, scale: 1.0, metal: metallic })
    };
    let k = |f: f32| rc.map(|c| c * f);
    match m {
        Metal => metal(scene, rc, 0.85, 0.32),
        Dark => metal(scene, k(0.6), 0.95, 0.22),
        Barrel if heat == 0 => metal(scene, k(0.6), 0.95, 0.22),
        Barrel => { let (c, e) = gun::heat_glow(heat); glow(scene, c, e * 4000.0) }
        Polymer => material(scene, [0.3, 0.25, 0.18], 0.7),
        Pad => material(scene, [0.07, 0.07, 0.075], 0.7),
        Recess => metal(scene, k(0.2), 0.3, 0.8),
        Slot => metal(scene, k(0.25), 0.4, 0.85),
        Bolt => metal(scene, [0.55, 0.57, 0.62], 0.95, 0.18),
        Lens => glow(scene, [0.35, 0.7, 1.0], 7000.0),
        Indicator => glow(scene, [0.4, 1.0, 0.95], 10000.0),
    }
}

/// A player's camera with the view punch, for the first-person gun.
fn punched_camera(world: &World, i: usize, prev: &[Vec3], alpha: f32, punch: &[(Vec3, glam::Vec2)]) -> Camera {
    apply_punch(world, i, camera(world, i, prev, alpha), punch)
}

fn apply_punch(world: &World, i: usize, mut cam: Camera, punch: &[(Vec3, glam::Vec2)]) -> Camera {
    let Some(&(off, rot)) = punch.get(i) else { return cam };
    if !world.players[i].alive { return cam; }
    let yaw = Quat::from_rotation_y(world.players[i].yaw);
    let fwd = cam.target - cam.pos;
    let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
    let turned = Quat::from_axis_angle(Vec3::Y, rot.y) * Quat::from_axis_angle(right, rot.x) * fwd;
    cam.pos += yaw * off;
    cam.target = cam.pos + turned;
    cam
}

impl Marks {
    fn place(&mut self, scene: &mut Scene, xf: Affine3A, chunk: u32) {
        if self.ring.len() < self.slots.capacity() {
            if let Some(id) = self.slots.add(scene, self.mesh, xf) { self.ring.push((id, chunk)); }
            return;
        }
        let k = self.next % self.ring.len();
        self.next += 1;
        let id = self.ring[k].0;
        if self.slots.is_used(id) { self.slots.set(scene, id, self.mesh, xf); } else if let Some(n) = self.slots.add(scene, self.mesh, xf) { self.ring[k].0 = n; }
        self.ring[k].1 = chunk;
    }

    fn drop_on(&mut self, scene: &mut Scene, chunk: u32) {
        for (id, c) in &mut self.ring {
            if *c == chunk && self.slots.is_used(*id) { self.slots.remove(scene, *id); *c = u32::MAX; }
        }
    }
}

fn wear_level(w: f32) -> u8 { if w <= 0.08 { 0 } else if w < 0.5 { 1 } else { 2 } }

fn stone(scene: &mut Scene, tint: [f32; 3], rough: f32) -> Stone {
    // Per-brick brightness variation (Godot's INSTANCE_CUSTOM) as four tints.
    let intact = [0.88, 0.96, 1.04, 1.12].map(|k| {
        let m = material(scene, tint.map(|c| (c * k).min(1.0)), rough);
        scene.add_mesh(&primitives::hewn_block(m, 3, 0.09, 0.0, 0).mesh())
    });
    let dark = |k: f32| tint.map(|c| c * k);
    let cm = material(scene, dark(0.8), rough);
    let cracked = scene.add_mesh(&primitives::hewn_block(cm, 4, 0.11, 0.07, 1).mesh());
    let km = material(scene, dark(0.62), rough);
    let crumbling = scene.add_mesh(&primitives::hewn_block(km, 4, 0.14, 0.16, 2).mesh());
    let chm = material(scene, dark(0.9), rough);
    let chip = scene.add_mesh(&primitives::hewn_block(chm, 1, 0.12, 0.2, 3).mesh());
    // Rubble: rough shards in three shades (Godot: base colour x 0.72..1.17).
    let shards = [0.72, 0.92, 1.12].map(|k| {
        let m = material(scene, tint.map(|c| (c * k).min(1.0)), rough);
        scene.add_mesh(&primitives::hewn_block(m, 2, 0.15, 0.22, (k * 100.0) as u32).mesh())
    });
    Stone { intact, cracked, crumbling, chip, shards }
}

fn omni(p: Vec3, cd: Vec3, range: f32) -> LocalLight {
    LocalLight { position: p, direction: Vec3::NEG_Y, intensity_cd: cd, range_m: range, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.15, falloff: 2.0 }
}

/// Godot's sky_top to horizon; the ground colour is the darkened horizon.
fn sky() -> (u32, u32, Vec<[f32; 4]>) {
    primitives::gradient_sky(palette::SKY_TOP, palette::HORIZON, palette::HORIZON.map(|c| c * 0.79))
}

pub fn lighting() -> Lighting {
    let (el, az) = (38f32.to_radians(), 140f32.to_radians());
    Lighting {
        sun_dir: Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()),
        sun_lux: 60_000.0,
        sun_color: Vec3::from(palette::SUN),
        sky_lux: 14_000.0,
        ev100: 13.6,
        fog_start: 40.0,
        fog_end: 320.0,
        ..Default::default()
    }
}

pub fn camera(world: &World, i: usize, prev: &[Vec3], alpha: f32) -> Camera {
    let p = &world.players[i];
    let pos = prev.get(i).map_or(p.pos, |&q| q.lerp(p.pos, alpha));
    if !p.alive {
        // Spectate from above the arena centre.
        let a = world.tick as f32 * 0.003 + i as f32;
        let r = world.arena.size * 0.55;
        return Camera { pos: Vec3::new(a.cos() * r, 22.0, a.sin() * r), target: Vec3::ZERO, fov_y: FOV_Y, near: 0.05 };
    }
    // The player's own model is hidden from this view (hide_own_bodies).
    let eye = pos + Vec3::Y * EYE;
    Camera { pos: eye, target: eye + p.forward(), fov_y: FOV_Y, near: 0.05 }
}
