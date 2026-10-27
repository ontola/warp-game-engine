//! Turns the simulation into a Warp scene: arena blocks as instanced unit
//! cubes with the castle palette, lava, players, bullet tracers, sparks and
//! lights, plus one camera per local player.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::shape::Kind;
use growing_guns::{Event, World, EYE};
use render::primitives;
use render::particles::{Burst, Particles};
use render::slots::{SlotId, Slots};
use physics::{Body, BodyDesc, Physics, Shape};
use render::headlights::LocalLight;
use render::scene::{GpuInstance, Material, EMISSIVE};
use render::{Camera, Lighting, Scene};

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
}

struct Ragdoll { torso: Body, head: Body, slots: [SlotId; 2], meshes: [usize; 2] }

#[derive(Clone, Copy)]
struct FxMeshes { spark: usize, dust: usize, blood: usize, casing: usize }

struct KindMeshes { floor: usize, wall: usize, building: usize, dark: usize, accent_a: usize, accent_b: usize, beacons: [usize; 3] }

const ARENA_SLOTS: usize = 256;
/// The Godot arena's light colours (towers and building roofs).
const BEACON_COLORS: [[f32; 3]; 3] = [[1.0, 0.4, 0.3], [1.0, 0.35, 0.2], [1.0, 0.7, 0.2]];
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|i| (a[i] - b[i]).powi(2)).sum() }


impl Visuals {
    pub fn build(world: &World) -> Self {
        let mut scene = Scene::default();
        scene.sky = Some(sky());
        let kind_mesh = |scene: &mut Scene, tint, rough| { let m = material(scene, tint, rough); cube(scene, m) };
        let kinds = KindMeshes {
            floor: kind_mesh(&mut scene, palette::FLOOR, 0.9),
            wall: kind_mesh(&mut scene, palette::WALL, 0.85),
            building: kind_mesh(&mut scene, palette::BUILDING, 0.85),
            dark: kind_mesh(&mut scene, palette::DARK, 0.9),
            accent_a: kind_mesh(&mut scene, palette::ACCENT_A, 0.7),
            accent_b: kind_mesh(&mut scene, palette::ACCENT_B, 0.7),
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
        // Effects: engine particles with their own meshes.
        let fxm = FxMeshes {
            spark: { let m = glow(&mut scene, [1.0, 0.6, 0.2], 20000.0); cube(&mut scene, m) },
            dust: { let m = material(&mut scene, palette::DARK, 0.95); cube(&mut scene, m) },
            blood: { let m = material(&mut scene, [0.45, 0.02, 0.02], 0.35); sphere(&mut scene, m, 4, 6) },
            casing: { let m = scene.add_material(Material { tint: [0.75, 0.55, 0.2], layer: -1, roughness: 0.35, flags: 0, scale: 1.0, metal: 1.0 }); cube(&mut scene, m) },
        };
        let fx = Particles::reserve(&mut scene, 768, world.arena.seed | 1);
        // Ragdolls: two jointed bodies per corpse, drawn with the player's meshes.
        let corpses = Slots::reserve(&mut scene, 32);
        let phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), growing_guns::TICK_HZ as f32);

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

        let mut v = Visuals { scene, first_dynamic, players, tracer, fx, fxm, flashes: Vec::new(), lights: Vec::new(), arena, kinds,
            corpses, phys, ragdolls: Vec::new() };
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
        self.phys.set_statics(&world.arena.shapes);
        let k = &self.kinds;
        for b in &world.arena.blocks {
            let m = match b.kind {
                Kind::Floor => k.floor,
                Kind::Wall => k.wall,
                Kind::Tower | Kind::Pillar => k.dark,
                Kind::Building => k.building,
                Kind::Ledge | Kind::Platform => k.accent_b,
                Kind::Cover => k.accent_a,
            };
            let xf = Affine3A::from_scale_rotation_translation(b.half * 2.0, Quat::from_rotation_y(b.yaw), b.center);
            self.arena.add(&mut self.scene, m, xf).expect("arena slots");
        }
        // Roof beacons where the Godot lights hang, in the nearest colour.
        for &(p, c) in &world.arena.lights {
            let nearest = (0..BEACON_COLORS.len()).min_by(|&a, &b| dist(BEACON_COLORS[a], c).total_cmp(&dist(BEACON_COLORS[b], c))).unwrap();
            self.arena.add(&mut self.scene, k.beacons[nearest], Affine3A::from_scale_rotation_translation(Vec3::splat(0.5), Quat::IDENTITY, p)).expect("arena slots");
        }
        self.lights = world.arena.lights.iter().take(12).map(|&(p, c)| omni(p, Vec3::from(c) * 900.0, 20.0)).collect();
    }

    /// Upload changed arena slots and particles.
    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        self.arena.flush(gpu, renderer, &self.scene)?;
        self.corpses.flush(gpu, renderer, &self.scene)?;
        self.fx.flush(gpu, renderer, &self.scene)
    }

    /// Effects from this tick's events, then one tick of particle motion
    /// (colliding with the arena).
    pub fn absorb(&mut self, world: &World) {
        let (s, m) = (&mut self.scene, self.fxm);
        for e in &world.events {
            match *e {
                Event::Impact { pos, normal } => {
                    self.fx.burst(s, pos + normal * 0.03, normal, &Burst { mesh: m.spark, count: 8, ..Burst::sparks() });
                    self.fx.burst(s, pos + normal * 0.05, normal, &Burst { mesh: m.dust, count: 5, ..Burst::dust() });
                }
                Event::Hit { victim, attacker, pos, head, .. } => {
                    let dir = (world.players[victim].pos - world.players[attacker].pos).normalize_or(Vec3::Y);
                    self.fx.burst(s, pos, dir + Vec3::Y * 0.3, &Burst { mesh: m.blood, count: if head { 22 } else { 12 }, ..Burst::drops() });
                }
                Event::Kill { victim, attacker } => {
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
                Event::Shot { player, from, dir } => {
                    self.flashes.push((from + dir * 0.8, 0.06));
                    let p = &world.players[player];
                    let (sy, cy) = p.yaw.sin_cos();
                    let right = Vec3::new(cy, 0.0, -sy);
                    self.fx.burst_with(s, from + right * 0.35 - Vec3::Y * 0.25 + dir * 0.3, right + Vec3::Y * 0.6, p.vel, &Burst { mesh: m.casing, ..Burst::debris() });
                }
                _ => {}
            }
        }
        self.phys.step();
        for r in &self.ragdolls {
            for (body, slot, mesh, scale) in [(r.torso, r.slots[0], r.meshes[0], Vec3::new(0.42, 0.5, 0.42)), (r.head, r.slots[1], r.meshes[1], Vec3::splat(0.3))] {
                if let Some((pos, rot)) = self.phys.pose(body) {
                    self.corpses.set(s, slot, mesh, Affine3A::from_scale_rotation_translation(scale, rot, pos));
                }
            }
        }
        let shapes = &world.arena.shapes;
        self.fx.update(s, growing_guns::DT, |a, b| {
            let d = b - a;
            let len = d.length();
            if len < 1e-6 { return None; }
            growing_guns::shape::raycast(shapes, a, d / len, len).map(|(t, n, _)| (a + d / len * t, n))
        });
    }

    /// Rewrite the dynamic instances. `alpha` blends from the previous tick.
    pub fn pose(&mut self, world: &World, prev: &[Vec3], alpha: f32, dt: f32) {
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
            for (mesh, xf) in [(m.body, body), (m.head, head), (m.eyes, eyes), (m.gun, gun)] { self.scene.update_instance(slot, mesh, xf); slot += 1; }
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
    }

    /// First person: hide each local player's own model from their view
    /// (`slots[k]` is drawn in view k). It still casts its shadow.
    pub fn hide_own_bodies(&mut self, slots: &[usize]) {
        for (k, &s) in slots.iter().enumerate().take(8) {
            for part in 0..4 { self.scene.instances[self.first_dynamic + s * 4 + part].hide_in_views(1 << k); }
        }
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn ragdolls(&self) -> Vec<[f32; 3]> {
        self.ragdolls.iter().filter_map(|r| self.phys.pose(r.torso)).map(|(p, _)| p.to_array()).collect()
    }

    pub fn dynamic(&self) -> &[GpuInstance] { &self.scene.instances[self.first_dynamic..] }

    pub fn local_lights(&self) -> Vec<LocalLight> {
        let mut l = self.lights.clone();
        for &(p, _) in self.flashes.iter().rev().take(16 - l.len()) { l.push(omni(p, Vec3::new(1.0, 0.75, 0.35) * 600.0, 8.0)); }
        l
    }
}

fn omni(p: Vec3, cd: Vec3, range: f32) -> LocalLight {
    LocalLight { position: p, direction: Vec3::NEG_Y, intensity_cd: cd, range_m: range, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.15 }
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
