//! Turns the simulation into a Warp scene, one module per part:
//! `arena` (bricks, beacons, marks), `effects` (particles, debris,
//! explosions, flashes, view punch), `players` (players, tracers, corpses,
//! gibs via `gore`), `guns` (the growing gun) and the colosseum
//! (crate::stadium). This module owns the scene and the physics they share,
//! routes each tick's events, and sets the lighting and cameras.
mod arena;
mod effects;
mod gore;
mod guns;
mod players;

use crate::stadium::StadiumVisuals;
use arena::{palette, ArenaView};
use audio::sfx::Listener;
use effects::{EffectsView, BLAST_TINT};
use glam::{Affine3A, Quat, Vec3};
use growing_guns::shape::Boxes;
use growing_guns::{Event, World, EYE};
use guns::Guns;
use physics::Physics;
use players::PlayersView;
use render::headlights::LocalLight;
use render::particles::Burst;
use render::primitives;
use render::scene::{GpuInstance, Material, EMISSIVE};
use render::{Camera, Lighting, Scene};

pub const FOV_Y: f32 = 75.0 * std::f32::consts::PI / 180.0;
pub const PLAYER_COLORS: [[f32; 3]; 4] = [[0.85, 0.22, 0.12], [0.15, 0.38, 0.85], [0.20, 0.70, 0.25], [0.90, 0.75, 0.12]];
pub const PLAYER_COLORS_U8: [[u8; 4]; 4] = [[255, 110, 80, 255], [110, 160, 255, 255], [120, 225, 130, 255], [255, 220, 90, 255]];
/// The renderer's limit on local lights per frame.
const MAX_LIGHTS: usize = 16;

fn srgb_to_linear(c: [f32; 3]) -> [f32; 3] { c.map(|v| v.powf(2.2)) }
fn cube(scene: &mut Scene, material: u32) -> usize { scene.add_mesh(&primitives::cube(material).mesh()) }
fn sphere(scene: &mut Scene, material: u32, rings: u32, segments: u32) -> usize { scene.add_mesh(&primitives::sphere(material, rings, segments).mesh()) }

fn material(scene: &mut Scene, tint: [f32; 3], roughness: f32) -> u32 {
    scene.add_material(Material { tint: srgb_to_linear(tint), layer: -1, roughness, flags: 0, scale: 1.0, metal: 0.0 })
}

fn glow(scene: &mut Scene, tint: [f32; 3], nits: f32) -> u32 {
    scene.add_material(Material { tint, layer: -1, roughness: 0.8, flags: EMISSIVE, scale: nits, metal: 0.0 })
}

fn omni(p: Vec3, cd: Vec3, range: f32) -> LocalLight {
    LocalLight { position: p, direction: Vec3::NEG_Y, intensity_cd: cd, range_m: range, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.15, falloff: 2.0 }
}

/// Scene plus the instance ranges the frame loop rewrites.
pub struct Visuals {
    pub scene: Scene,
    /// Everything from here on is uploaded every frame (`dynamic`).
    pub first_dynamic: usize,
    phys: Physics,
    arena: ArenaView,
    fx: EffectsView,
    players: PlayersView,
    guns: Guns,
    /// The colosseum, its crowd and their voice.
    pub stadium: StadiumVisuals,
    /// Seconds on the effects clock for the frame being drawn.
    clock: f32,
}

impl Visuals {
    pub fn build(world: &World) -> Self {
        let mut scene = Scene::default();
        scene.sky = Some(sky());
        // Static range first, then the dynamic one; the order is the scene's
        // instance order.
        let arena = ArenaView::reserve(&mut scene);
        let fx = EffectsView::reserve(&mut scene, world.arena.seed);
        let mut players = PlayersView::reserve_corpses(&mut scene);
        let phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), growing_guns::TICK_HZ as f32);
        let stadium = StadiumVisuals::reserve(&mut scene);
        let first_dynamic = scene.instances.len();
        players.reserve_dynamic(&mut scene, world.players.len());
        let guns = Guns::reserve(&mut scene, world.players.len());
        // Gore lives in the dynamic range too: uploaded with it every frame.
        players.reserve_gore(&mut scene, world.arena.seed);
        let mut v = Visuals { scene, first_dynamic, phys, arena, fx, players, guns, stadium, clock: 0.0 };
        v.set_arena(world);
        v
    }

    /// Fill the arena from the world's current arena (a new round). Call
    /// `flush` afterwards to upload; the renderer stays.
    pub fn set_arena(&mut self, world: &World) {
        self.players.clear(&mut self.scene, &mut self.phys);
        self.phys.clear_bodies();
        self.arena.set(&mut self.scene, &mut self.phys, &mut self.fx, world);
        self.stadium.set_arena(&mut self.scene, world);
    }

    /// Upload changed slots, debris, effects and the crowd.
    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        self.arena.flush(gpu, renderer, &self.scene)?;
        self.players.flush(gpu, renderer, &self.scene)?;
        self.stadium.flush(gpu, renderer, &self.scene, self.clock)?;
        self.fx.flush(gpu, renderer, &self.scene, self.clock)
    }

    /// Effects from this tick's events, then one tick of physics and
    /// particle motion (colliding with the arena). `ears` place the crowd's
    /// reactions.
    pub fn absorb(&mut self, world: &World, ears: &[Listener]) {
        let now = world.tick as f32 * growing_guns::DT;
        self.fx.debris.advance(growing_guns::DT);
        self.arena.break_chunks(&mut self.scene, &mut self.phys, &mut self.fx, world);
        let s = &mut self.scene;
        let m = self.fx.meshes;
        for e in &world.events {
            match *e {
                Event::Impact { pos, normal, damage, strong } => {
                    self.fx.particles.burst(s, pos + normal * 0.03, normal, &Burst { mesh: m.spark, count: 8, ..Burst::sparks() });
                    self.fx.particles.burst(s, pos + normal * 0.05, normal, &Burst { mesh: m.dust, count: 5, ..Burst::dust() });
                    // Chips fly off every hit, more and bigger for heavy rounds
                    // (Godot `request_chips`), as GPU debris.
                    let chunk = world.arena.grid.ray(pos + normal * 0.05, -normal, 0.3).map(|h| h.2);
                    let stone = self.arena.stone_at(world, chunk);
                    let scale = (0.06 + damage * 0.008).clamp(0.06, 1.0);
                    let amount = ((16.0 * scale).round() as u32).clamp(2, 20);
                    let size = 2.4 * (0.05 + damage * 0.0022).clamp(0.05, 0.35);
                    let radius = growing_guns::destruct::rifle_chip(damage / 16.0).1;
                    self.fx.debris.burst(s, &render::debris::BurstSpec::chips(pos + normal * 0.15, radius * 0.3, amount, size, &stone.shards));
                    // A scorch mark on the stone, bigger for strong rounds.
                    if let Some(c) = chunk {
                        let r = if strong { 0.32 } else { 0.22 };
                        let xf = Affine3A::from_scale_rotation_translation(Vec3::new(r, 0.02, r), Quat::from_rotation_arc(Vec3::Y, normal), pos);
                        self.arena.scorch(s, xf, c as u32);
                    }
                }
                Event::Explosion { pos, radius, .. } => {
                    // Godot's spawn_bullet_blast: billowing fire and smoke,
                    // flame tongues, embers, heat shell, flash and glow lights.
                    self.fx.blasts.explosion(now, pos, radius, BLAST_TINT);
                    self.fx.blast_punch(world, now, pos, radius);
                    self.phys.blast(pos, radius, 9.0);
                }
                Event::Hit { victim, attacker, pos, head, .. } => {
                    let dir = (world.players[victim].pos - world.players[attacker].pos).normalize_or(Vec3::Y);
                    self.fx.particles.burst(s, pos, dir + Vec3::Y * 0.3, &Burst { mesh: m.blood, count: if head { 22 } else { 12 }, ..Burst::drops() });
                }
                Event::Kill { victim, attacker, gib, dir } => {
                    self.players.kill(s, &mut self.phys, &mut self.fx, &mut self.arena, world, victim, attacker, gib, dir, now);
                }
                Event::Picked { player, .. } => self.guns.grow(player, &world.players[player]),
                Event::Jump { player } => self.guns.jump(player),
                Event::Shot { player, from, dir } => {
                    let p = &world.players[player];
                    self.guns.shot(player, p);
                    self.fx.flash(from + dir * 0.8);
                    let (sy, cy) = p.yaw.sin_cos();
                    let right = Vec3::new(cy, 0.0, -sy);
                    self.fx.particles.burst_with(s, from + right * 0.35 - Vec3::Y * 0.25 + dir * 0.3, right + Vec3::Y * 0.6, p.vel, &Burst { mesh: m.casing, ..Burst::debris() });
                }
                _ => {}
            }
        }
        self.stadium.absorb(s, &mut self.fx.particles, &mut self.fx.blasts, world, ears, now, growing_guns::DT);
        self.guns.tick(world, growing_guns::DT);
        self.phys.step();
        self.players.follow_physics(s, &mut self.phys, now);
        let shapes = &world.arena.grid;
        self.fx.particles.update(s, growing_guns::DT, |a, b| {
            let d = b - a;
            let len = d.length();
            if len < 1e-6 { return None; }
            growing_guns::shape::raycast(shapes, a, d / len, len).map(|(t, n, _)| (a + d / len * t, n))
        });
    }

    /// Rewrite the dynamic instances. `alpha` blends from the previous tick.
    pub fn pose(&mut self, world: &World, prev: &[Vec3], alpha: f32, dt: f32) {
        self.clock = (world.tick as f32 - 1.0 + alpha) * growing_guns::DT;
        self.fx.update(world, self.clock, dt);
        self.players.pose(&mut self.scene, world, prev, alpha);
        self.guns.pose(&mut self.scene, world, prev, alpha, &self.fx);
    }

    /// Morph a player's gun into their current stats (a card was given).
    pub fn grow_gun(&mut self, i: usize, p: &growing_guns::Player) { self.guns.grow(i, p); }

    /// First person: in view k, local player `slots[k]` sees neither their
    /// own body nor their third-person gun, and only their first-person gun.
    pub fn hide_own_bodies(&mut self, slots: &[usize]) {
        self.players.hide_own(&mut self.scene, slots);
        self.guns.hide_own(&mut self.scene, slots);
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn ragdolls(&self) -> Vec<[f32; 3]> { self.players.ragdolls(&self.phys) }

    pub fn gore_len(&self) -> usize { self.players.gore_len() }

    pub fn dynamic(&self) -> &[GpuInstance] { &self.scene.instances[self.first_dynamic..] }

    /// A player's camera with the explosion punch applied.
    pub fn punched(&self, world: &World, i: usize, cam: Camera) -> Camera { self.fx.punched(world, i, cam) }

    /// Blast lights first (they matter most while they last), then the
    /// floodlights, the roof beacons and muzzle flashes.
    pub fn local_lights(&self) -> Vec<LocalLight> {
        let mut l = self.fx.blasts.lights(self.clock);
        l.truncate(MAX_LIGHTS);
        l.extend(self.stadium.lights().iter().take(MAX_LIGHTS - l.len()).copied());
        l.extend(self.arena.lights().iter().take(MAX_LIGHTS - l.len()).copied());
        let room = MAX_LIGHTS - l.len();
        l.extend(self.fx.flash_lights(room));
        l
    }
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
