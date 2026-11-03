//! Turns the simulation into a Warp scene, one module per part:
//! `arena` (bricks, roof lights, marks), `effects` (particles, debris,
//! explosions, flashes, view punch), `players` (players, tracers, corpses,
//! gibs via `gore`), `guns` (the growing gun) and the colosseum
//! (crate::stadium). This module owns the scene and the physics they share,
//! routes each tick's events, and sets the lighting and cameras.
mod dust;
mod arena;
mod cages;
mod characters;
mod character_ragdoll;
mod effects;
mod gore;
pub(crate) mod guns;
mod ion;
mod lava;
mod players;
mod things;

use crate::stadium::StadiumVisuals;
use arena::{palette, ArenaView};
use audio::sfx::Listener;
use effects::{EffectsView, BLAST_TINT};
use glam::Vec3;
use growing_guns::shape::Boxes;
use growing_guns::{Event, ExplosionKind, World};
use guns::Guns;
use physics::Physics;
use players::PlayersView;
pub use players::TRACERS;
use render::headlights::LocalLight;
use render::particles::Burst;
use render::primitives;
use render::scene::{GpuInstance, Material, EMISSIVE};
use render::{Camera, Lighting, Scene};

pub const FOV_Y: f32 = 75.0 * std::f32::consts::PI / 180.0;
pub const PLAYER_COLORS: [[f32; 3]; 4] = [[0.85, 0.22, 0.12], [0.15, 0.38, 0.85], [0.20, 0.70, 0.25], [0.90, 0.75, 0.12]];
pub const PLAYER_COLORS_U8: [[u8; 4]; 4] = [[255, 110, 80, 255], [110, 160, 255, 255], [120, 225, 130, 255], [255, 220, 90, 255]];
/// Players drawn in a coop match: the seats and the wave's enemies (players
/// past this are not drawn; waves reach it around wave 20).
pub const COOP_VIEW_PLAYERS: usize = 24;
/// The renderer's limit on local lights per frame.
const MAX_LIGHTS: usize = render::headlights::MAX_LOCAL_LIGHTS;
/// Of those, the most that ion strikes and blasts take, so the floodlights
/// and the lava keep theirs.
/// The flashlight's place from the eye: left and down (Godot `FLASHLIGHT_OFFSET`).
const FLASHLIGHT_OFFSET: [f32; 2] = [0.32, -0.18];
/// Flashlight strength, candela (`GG_FLASHLIGHT` overrides).
const FLASHLIGHT_CD: f32 = 60_000.0;
fn flashlight_cd() -> f32 { std::env::var("GG_FLASHLIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(FLASHLIGHT_CD) }

/// Share of the floods' intensity that comes back off the field as fill
/// (`flood_bounce`; `GG_BOUNCE` overrides).
const FLOOD_BOUNCE: f32 = 0.0;
const MAX_TRANSIENT_LIGHTS: usize = MAX_LIGHTS - 4 - lava::MAX_LAVA_LIGHTS;
/// Scatter weights for the night's volumetric light: the wide floodlights,
/// their narrow beams, the lava's glow and the roof beacons. Tuned on the
/// night captures. At most 32 lights scatter (the renderer's limit; local
/// lights before beams): 4 floods + 14 lava + 12 beams leaves the beacons out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scatter { pub flood: f32, pub beam: f32, pub lava: f32, pub beacon: f32, pub pickup: f32 }
impl Default for Scatter { fn default() -> Self { Self { flood: 0.2, beam: 1.0, lava: 1.0, beacon: 0.0, pickup: 4.0 } } }

fn srgb_to_linear(c: [f32; 3]) -> [f32; 3] { c.map(|v| v.powf(2.2)) }
fn cube(scene: &mut Scene, material: u32) -> usize { scene.add_mesh(&primitives::cube(material).mesh()) }
fn sphere(scene: &mut Scene, material: u32, rings: u32, segments: u32) -> usize { scene.add_mesh(&primitives::sphere(material, rings, segments).mesh()) }

fn material(scene: &mut Scene, tint: [f32; 3], roughness: f32) -> u32 {
    scene.add_material(Material { tint: srgb_to_linear(tint), layer: -1, roughness, flags: 0, scale: 1.0, metal: 0.0 })
}

/// Shared quarry surface, cooked once and cached by the engine. The legacy
/// switch is for same-executable visual/performance comparisons only.
pub(crate) fn stone_layer(scene: &mut Scene) -> i32 {
    if std::env::var("GG_STONE_SURFACE").as_deref() == Ok("legacy") { -1 }
    else { render::stone_surface::add_to_scene(scene) }
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
    /// The sprite images (the drop beams' shaft) are on this renderer.
    sprite_images: bool,
    /// Everything from here on is uploaded every frame (`dynamic`).
    pub first_dynamic: usize,
    phys: Physics,
    arena: ArenaView,
    fx: EffectsView,
    players: PlayersView,
    cages: cages::Cages,
    guns: Guns,
    things: things::Things,
    ion: ion::IonFx,
    /// The colosseum, its crowd and their voice.
    pub stadium: StadiumVisuals,
    /// Seconds on the effects clock for the frame being drawn.
    clock: f32,
    /// Floodlight strength against Godot's (crate::sky::floodlights).
    flood: f32,
    /// A blackout round: the floods, lamps and lava glow are out; only the
    /// players' flashlights (`flashlights`) and short flashes light the arena.
    pub blackout: bool,
    /// Air strike targets in flight last tick, and when the last one hit.
    air_targets: Vec<Vec3>,
    air_impact: f32,
    artillery_seats: Vec<Option<Vec3>>,
    /// World sprites: the light shafts, then the bullet streaks.
    sprites: Vec<render::sprites::Sprite>,
}

impl Visuals {
    pub fn build(world: &World) -> Self {
        let mut scene = Scene::default();
        scene.graphics.minimal_forward=crate::graphics::get().minimal_forward;
        scene.graphics.simple_lighting=crate::graphics::get().simple_lighting;
        scene.graphics.simple_mesh_culling=crate::graphics::get().simple_blocks;
        scene.sky = Some(sky());
        // Godot's lava noise texture, for the lava shader.
        scene.noise = Some(render::noise_field::NoiseField { size: lava::NOISE_SIZE, values: lava::noise_texture() });
        // Static range first, then the dynamic one; the order is the scene's
        // instance order.
        let arena = ArenaView::reserve(&mut scene);
        let fx = EffectsView::reserve(&mut scene, world.arena.seed);
        let mut players = PlayersView::reserve_corpses(&mut scene);
        let phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), growing_guns::TICK_HZ as f32);
        let stadium = StadiumVisuals::reserve(&mut scene);
        let first_dynamic = scene.instances.len();
        // Coop waves add players as they come; versus reserves all 8 seats up front
        // so players joining or bot counts changing never rebuild the GPU renderer.
        let drawn = world.players.len().max(if world.coop { COOP_VIEW_PLAYERS } else { 8 });
        players.reserve_dynamic(&mut scene, drawn);
        let cages = cages::Cages::reserve(&mut scene, drawn);
        let guns = Guns::reserve(&mut scene, drawn);
        let things = things::Things::reserve(&mut scene);
        // Gore lives in the dynamic range too: uploaded with it every frame.
        players.reserve_gore(&mut scene, world.arena.seed);
        // Shared geometry cache, reserved after all game-owned aux data.
        if crate::graphics::get().sky_occlusion && std::env::var("GG_SKY_OCCLUSION").as_deref() != Ok("0") {
            scene.reserve_irradiance_volume(render::irradiance_volume::Grid {
                origin: Vec3::new(-192.0, -6.0, -192.0), cell: 6.0, dims: [65, 13, 65],
            }).expect("stadium sky grid");
        }
        let mut v = Visuals { scene, first_dynamic, phys, arena, fx, players, cages, guns, things, ion: Default::default(), stadium, clock: 0.0, flood: 1.0, blackout: false, sprite_images: false, air_targets: Vec::new(), air_impact: f32::NEG_INFINITY, artillery_seats: Vec::new(), sprites: Vec::new() };
        v.set_arena(world);
        v
    }

    /// Fill the arena from the world's current arena (a new round). Call
    /// `flush` afterwards to upload; the renderer stays.
    pub fn set_arena(&mut self, world: &World) {
        self.artillery_seats.clear();
        self.guns.reset_heat();
        self.ion.reset();
        self.fx.blasts.clear();
        self.players.clear(&mut self.scene, &mut self.phys);
        self.cages.reset(&mut self.scene, world);
        self.phys.clear_bodies();
        let t = std::time::Instant::now();
        self.arena.set(&mut self.scene, &mut self.phys, &mut self.fx, world);
        crate::play::hitch("set_arena: arena", t);
        let t = std::time::Instant::now();
        self.stadium.set_arena(&mut self.scene, world);
        crate::play::hitch("set_arena: stadium", t);
    }

    /// The renderer was rebuilt: restore sprite images and the debris
    /// collision grid/metadata that lived on the previous GPU renderer.
    pub fn invalidate_sprites(&mut self) {
        self.sprite_images = false;
        self.stadium.invalidate_lighting();
        self.fx.debris.invalidate_renderer();
    }

    /// World sprites besides the name tags: the light shafts and bullet streaks.
    pub fn sprites(&self) -> &[render::sprites::Sprite] { &self.sprites }

    /// Upload changed slots, debris, effects and the crowd.
    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, light: &Lighting) -> Result<(), String> {
        if !std::mem::replace(&mut self.sprite_images, true) {
            for (id, picture) in things::Things::sprite_pictures().into_iter().chain([(players::TRAIL_IMAGE, players::trail_picture()), (effects::FLASH_IMAGE, effects::flash_picture())]) {
                renderer.set_sprite_image(gpu, id, &picture, render::hud::ImageFilter::Linear);
            }
        }
        self.arena.flush(gpu, renderer, &self.scene)?;
        self.players.flush(gpu, renderer, &self.scene)?;
        self.guns.flush(gpu, renderer)?;
        self.stadium.poll_sky(&mut self.scene);
        self.stadium.flush(gpu, renderer, &self.scene, self.clock, light)?;
        self.fx.flush(gpu, renderer, &self.scene, self.clock)
    }

    /// Reconstructing a recording needs collision/spawn ordering on the GPU,
    /// but static lighting, materials and ordinary instances only at paint time.
    pub fn flush_replay_debris(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        self.fx.debris.flush(gpu, renderer)
    }

    /// Effects from this tick's events, then one tick of physics and
    /// particle motion (colliding with the arena). `ears` place the crowd's
    /// reactions.
    pub fn absorb(&mut self, world: &World, ears: &[Listener]) {
        self.artillery_seats.resize(world.players.len(), None);
        for (i,p) in world.players.iter().enumerate() {
            if self.artillery_seats[i] != p.howitzer {
                if let Some(at)=p.howitzer {
                    // Reserve space for the gun/operator without gore or a crowd hit.
                    self.stadium.crowd.kill_near(at, 4.5, usize::MAX);
                }
                self.artillery_seats[i]=p.howitzer;
            }
        }
        let now = world.tick as f32 * growing_guns::DT;
        self.cages.sync(world);
        self.ion.sync(world, &mut self.fx.blasts, now);
        // Minimum keeps the rocket and its plume, not the lingering trail smoke.
        if !crate::graphics::get().minimal_forward { things::airstrike::smoke(&world.strikes, world.tick, now, &mut self.fx.blasts); }
        let lava = &world.lava;
        self.fx.debris.tuning.lava = Some(render::debris::LavaSurface {
            base: world.arena.lava_surface(), raised: if lava.active {lava.surface()}else{-1e20},
            half: lava.half, inset: lava.inset(), full: lava.full || lava.floor_t() >= 1.0,
        });
        self.fx.debris.advance(growing_guns::DT);
        self.arena.break_chunks(&mut self.scene, &mut self.phys, &mut self.fx, world);
        let s = &mut self.scene;
        let m = self.fx.meshes;
        for e in &world.events {
            match *e {
                Event::BulletPath { from, to, damage } => {
                    if let Some((pos, normal)) = self.players.shoot_corpses(&mut self.phys, from, to, damage) {
                        self.fx.particles.burst(s, pos + normal * 0.02, normal + Vec3::Y * 0.2,
                            &Burst { mesh: m.blood, count: 6, ..Burst::drops() });
                    }
                }
                Event::Impact { pos, normal, damage, strong } => {
                    self.fx.particles.burst(s, pos + normal * 0.03, normal, &Burst { mesh: m.spark, count: 8, ..Burst::sparks() });
                    // Chips fly off every hit, more and bigger for heavy rounds
                    // (Godot `request_chips`), as GPU debris.
                    let chunk = world.arena.grid.ray(pos + normal * 0.05, -normal, 0.3).map(|h| h.2);
                    let stone = self.arena.stone_at(world, chunk);
                    let scale = (0.06 + damage * 0.008).clamp(0.06, 1.0);
                    let amount = ((16.0 * scale).round() as u32).clamp(2, 20);
                    // A base round (22 damage) chips about 14 cm, a heavy one
                    // up to the old size; Godot's 0.05 floor read too chunky
                    // for small guns (Joep).
                    let size = 2.4 * (0.004 + damage * 0.0024).clamp(0.03, 0.35);
                    let radius = growing_guns::destruct::rifle_chip(damage / 16.0).1;
                    self.fx.debris.burst(s, &render::debris::BurstSpec::chips(pos + normal * 0.15, radius * 0.3, crate::graphics::get().debris_count(amount), size * crate::graphics::get().debris_size, &stone.shards));
                    // A bullet hole where the chips came off, bigger for strong rounds.
                    if let Some(c) = chunk {
                        let r = if strong { 0.24 } else { 0.15 };
                        self.arena.bullet_hole(pos, r, c as u32);
                    }
                }
                Event::Explosion { pos, radius, kind, .. } => {
                    // Godot's spawn_bullet_blast: billowing fire and smoke,
                    // flame tongues, embers, heat shell, flash and glow lights.
                    if kind == ExplosionKind::Ion {
                        self.ion.detonate(&mut self.fx.blasts, now, pos, radius, world.arena.lava_surface());
                    } else {
                        // An air strike's rocket hit (its blast is 10 cm over the target).
                        if self.air_targets.iter().any(|t| t.distance(pos - Vec3::Y * 0.1) < 0.05) { self.air_impact = now; }
                        self.fx.blasts.explosion(now, pos, radius, BLAST_TINT);
                        dust::emit(&mut self.fx.blasts,world,now,pos,radius);
                    }
                    self.fx.blast_punch(world, now, pos, radius);
                    if kind != ExplosionKind::Ion { self.phys.blast(pos, radius, 9.0); }
                }
                Event::Hit { victim, attacker, pos, head, .. } => {
                    let dir = (world.players[victim].pos - world.players[attacker].pos).normalize_or(Vec3::Y);
                    self.fx.particles.burst(s, pos, dir + Vec3::Y * 0.3, &Burst { mesh: m.blood, count: if head { 22 } else { 12 }, ..Burst::drops() });
                }
                Event::Kill { victim, attacker, gib, dir } => {
                    self.players.kill(s, &mut self.phys, &mut self.fx, &mut self.arena, world, victim, attacker, gib, dir, now);
                }
                // Picks can happen while dead, before the round finishes.
                // Guns starts the visible morph on the next round boundary.
                Event::Teleport { from, to, .. } => {
                    for at in [from, to] { self.fx.particles.burst(s, at, Vec3::Y, &Burst { mesh: m.spark, count: 24, ..Burst::sparks() }); }
                }
                Event::Slash { from, dir, range, .. } => {
                    for k in 0..6 {
                        let at = from + dir * (range * (k as f32 + 0.5) / 6.0);
                        self.fx.particles.burst(s, at, dir, &Burst { mesh: m.spark, count: 3, ..Burst::sparks() });
                    }
                }
                Event::Phoenix { player } => {
                    let at = world.players[player].pos;
                    self.fx.blasts.explosion(now, at, 3.0, BLAST_TINT);
                }
                Event::Jump { player } => self.guns.jump(player),
                Event::Shot { player, from, dir } => {
                    let p = &world.players[player];
                    self.guns.shot(player, p);
                    // Bullets and flash leave the barrel as drawn (the sim's
                    // muzzle is the receiver): the bullets' offset fades out
                    // over their first metres (Godot aims muzzle to crosshair).
                    let exit = self.guns.muzzle(player).unwrap_or(from + dir * 0.8);
                    self.players.shot_from(player, exit - from);
                    let w = &p.weapon;
                    let last = p.ammo == 0;
                    if w.silencer_stacks <= 0 {
                        let bright = ((w.damage_for_shot(last) / growing_guns::weapon::BASE_DAMAGE).powf(0.88) * 1.5).clamp(0.85, 8.0);
                        self.fx.flash(now - growing_guns::DT, exit, dir, w.bullet_color, w.bullet_scale_for_shot(last), bright);
                    }
                    let (sy, cy) = p.yaw.sin_cos();
                    let right = Vec3::new(cy, 0.0, -sy);
                    self.fx.particles.burst_with(s, from + right * 0.35 - Vec3::Y * 0.25 + dir * 0.3, right + Vec3::Y * 0.6, p.vel, &Burst { mesh: m.casing, ..Burst::debris() });
                }
                _ => {}
            }
        }
        // Include bodies created by this tick's Kill events, after the explosion.
        for e in &world.events {
            if let Event::Explosion { pos, radius, kind: ExplosionKind::Ion, .. } = e {
                self.phys.lift(*pos, *radius, 26.0);
            }
        }
        self.air_targets = world.strikes.iter().filter(|s| s.kind == growing_guns::StrikeKind::AirStrike).map(|s| s.target).collect();
        self.stadium.absorb(s, &mut self.fx.particles, &mut self.fx.blasts, world, ears, now, growing_guns::DT);
        self.guns.tick(world, growing_guns::DT);
        self.players.tick(world, &mut self.phys);
        self.players.burn_lava_bodies(world, &mut self.fx.blasts, now);
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
    pub fn pose(&mut self, world: &World, prev: &[Pose], alpha: f32, dt: f32) {
        self.clock = (world.tick as f32 - 1.0 + alpha) * growing_guns::DT;
        self.fx.update(world, self.clock, dt);
        self.players.pose(&mut self.scene, world, prev, alpha);
        self.cages.pose(&mut self.scene, world, prev, alpha, self.clock);
        self.guns.pose(&mut self.scene, world, prev, alpha, &self.fx, self.players.hands());
        self.things.pose(&mut self.scene, world, self.clock);
        self.sprites.clear();
        self.sprites.extend_from_slice(self.things.sprites());
        self.sprites.extend_from_slice(&self.players.trails);
        self.fx.flash_sprites(&mut self.sprites);
        self.arena.lava.pose(world);
    }

    /// The movement-tilt setting (the first-person gun's strafe lean).
    pub fn set_tilt(&mut self, on: bool) { self.guns.tilt = on; }

    /// Morph a player's gun into their current stats (a card was given).
    pub fn track_gun(&mut self, player: usize, pose: Option<glam::Affine3A>) { self.guns.track(player, pose); }
    pub fn tracked_muzzle(&self, player: usize, pose: glam::Affine3A) -> Vec3 { self.guns.tracked_muzzle(player, pose) }

    pub fn grow_gun(&mut self, i: usize, p: &growing_guns::Player) { self.guns.grow(i, p); }

    pub fn player_capacity(&self) -> usize { self.guns.capacity() }

    /// First person: in view k, local player `slots[k]` sees neither their
    /// own body nor their third-person gun, and only their first-person gun.
    pub fn hide_own_bodies(&mut self, slots: &[usize]) {
        self.players.hide_own(&mut self.scene, slots);
        self.guns.hide_own(&mut self.scene, slots);
    }

    /// Ragdoll torso positions (for state dumps).
    pub fn corpse_hits(&self) -> u64 { self.players.corpse_hits }

    pub fn ragdolls(&self) -> Vec<[f32; 3]> { self.players.ragdolls(&self.phys) }

    pub fn gore_len(&self) -> usize { self.players.gore_len() }

    /// Read-only diagnostics: CPU particles, live GPU debris, uploaded effect
    /// puffs (including delayed puffs), and cumulative debris spawns.
    /// Sample after `flush` so finished puffs have been removed.
    pub fn effect_counts(&self) -> (usize, usize, usize, u64) {
        (self.fx.particles.len(), self.fx.debris.live(), self.fx.blasts.len(), self.fx.debris.spawned)
    }

    pub fn dynamic(&self) -> &[GpuInstance] { &self.scene.instances[self.first_dynamic..] }

    pub fn ion_glow(&self) -> f32 { self.ion.glow(self.clock) }

    /// A player's camera with the explosion punch applied.
    pub fn lava_look(&mut self, player: usize, look: [f32; 2]) { self.players.lava_look(player, look); }

    pub fn punched(&self, world: &World, i: usize, cam: Camera) -> Camera {
        self.players.lava_camera(world, i, self.fx.punched(world, i, cam), self.clock)
    }

    /// Blast lights first (they matter most while they last), then the
    /// floodlights, the lava's glow, the roof beacons and muzzle flashes.
    pub fn local_lights(&self) -> Vec<LocalLight> { self.local_lights_scattering(Scatter::default()).0 }

    /// `local_lights` with a scatter weight per light for the renderer's
    /// volumetric light (`Lighting::volumetric`, only on at night): the
    /// floodlights, the lava's glow and the roof beacons may scatter, short
    /// flashes do not.
    pub fn local_lights_scattering(&self, scatter: Scatter) -> (Vec<LocalLight>, Vec<f32>) {
        let mut l = self.ion.lights(self.clock);
        l.extend(self.fx.blasts.lights(self.clock));
        l.extend(self.things.strike_lights().iter().copied());
        // Muzzle flashes before the steady lights: a shot must light up.
        l.extend(self.fx.flash_lights(4));
        l.truncate(MAX_TRANSIENT_LIGHTS);
        let mut w = vec![0.0; l.len()];
        let flood = self.flood;
        if self.blackout {
            l.extend(self.things.pickup_lights().iter().take(MAX_LIGHTS - l.len()).copied());
            w.resize(l.len(), 0.0);
            return (l, w);
        }
        // Legacy unoccluded flood fill is a diagnostic opt-in only: these
        // synthetic lamps can sit behind walls and defeat the real shadows.
        if self.flood_shadows() {
            l.extend(self.flood_bounce().into_iter().take(MAX_LIGHTS - l.len()));
            w.resize(l.len(), 0.0);
        }
        if !self.flood_shadows() {
            l.extend(self.stadium.lights().iter().take(MAX_LIGHTS - l.len()).map(|f| LocalLight { intensity_cd: f.intensity_cd * flood, ..*f }));
            w.resize(l.len(), scatter.flood);
        }
        l.extend(self.arena.lava.lights().iter().take(MAX_LIGHTS - l.len()).copied());
        w.resize(l.len(), scatter.lava);
        l.extend(self.arena.lights().iter().take(MAX_LIGHTS - l.len()).copied());
        w.resize(l.len(), scatter.beacon);
        l.extend(self.things.pickup_lights().iter().take(MAX_LIGHTS - l.len()).copied());
        w.resize(l.len(), scatter.pickup);
        (l, w)
    }

    /// At night the floodlights cast shadows (the walls shade the field):
    /// they are the renderer's shadowed spots then, not local lights. Off
    /// with `GG_FLOOD_SHADOWS=0`; by day they stay unshadowed, as in Godot.
    pub fn flood_shadows(&self) -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        self.flood > 1.01 && *ON.get_or_init(|| std::env::var("GG_FLOOD_SHADOWS").as_deref() != Ok("0"))
    }

    /// The floodlights as shadowed spots (empty unless `flood_shadows`).
    pub fn flood_spots(&self) -> Vec<render::headlights::SpotLight> {
        if !self.flood_shadows() || self.blackout { return Vec::new(); }
        self.stadium.lights().iter().take(render::headlights::MAX_LIGHTS).map(|f| {
            let side = f.direction.cross(Vec3::Y).normalize_or(Vec3::X);
            render::headlights::SpotLight {
                position: f.position, direction: f.direction, up: side.cross(f.direction),
                intensity_cd: (f.intensity_cd * self.flood).min(Vec3::splat(1.0e8)), range_m: f.range_m.clamp(1.0, 500.0),
                inner_angle_radians: f.inner_cos.clamp(-1.0, 1.0).acos(), outer_angle_radians: f.outer_cos.clamp(-1.0, 1.0).acos().min(1.3),
            }
        }).collect()
    }

    pub fn flashlight_exclusions(&self, world: &World) -> Vec<(u32,u32)> {
        flashlight_owner_ranges(world, self.players.first)
    }

    /// Every living player's flashlight in a blackout round (Godot
    /// `player.gd _build_flashlight`): a narrow shadowed beam from the off
    /// hand, low and to the left of the eye, along the aim.
    pub fn flashlights(world: &World, prev: &[Pose], alpha: f32) -> Vec<render::headlights::SpotLight> {
        world.players.iter().enumerate().filter(|(_, p)| p.alive).take(render::headlights::MAX_LIGHTS).map(|(i, p)| {
            let at = pose(world, i, prev, alpha).pos + Vec3::Y * p.eye_height();
            let dir = growing_guns::look_dir(p.yaw, p.pitch);
            let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
            let up = side.cross(dir);
            render::headlights::SpotLight {
                position: at - side * FLASHLIGHT_OFFSET[0] + up * FLASHLIGHT_OFFSET[1], direction: dir, up,
                intensity_cd: Vec3::new(1.0, 0.97, 0.88) * flashlight_cd(), range_m: 40.0,
                inner_angle_radians: 12f32.to_radians(), outer_angle_radians: 20f32.to_radians(),
            }
        }).collect()
    }

    /// The floods' light off the field (cheap GI for `flood_shadows`): one
    /// soft light high over the centre and one toward each tower, together
    /// a share of what the floods put on the field.
    fn flood_bounce(&self) -> Vec<LocalLight> {
        let floods = self.stadium.lights();
        if floods.is_empty() { return Vec::new(); }
        let k: f32 = std::env::var("GG_BOUNCE").ok().and_then(|v| v.parse().ok()).unwrap_or(FLOOD_BOUNCE);
        if !k.is_finite() || k <= 0.0 { return Vec::new(); }
        // Diagnostic approximation only: these lights have no occlusion.
        let tint = Vec3::new(1.0, 0.93, 0.84);
        let total: Vec3 = floods.iter().map(|f| f.intensity_cd).sum::<Vec3>() * self.flood * k * tint;
        let spots: Vec<Vec3> = std::iter::once(Vec3::ZERO).chain(floods.iter().map(|f| f.position.with_y(0.0) * 0.35)).collect();
        let reach = floods.iter().map(|f| f.position.with_y(0.0).length()).fold(0.0, f32::max);
        spots.iter().map(|&p| LocalLight {
            position: p + Vec3::Y * 4.0, direction: Vec3::NEG_Y, intensity_cd: total / spots.len() as f32,
            range_m: (reach * 0.8).clamp(10.0, 120.0), inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 1.0, falloff: 2.0,
        }).collect()
    }

    /// The floodlight beams and the pickups' drop-beam searchlights, which
    /// only scatter (night volumetric light).
    pub fn volumetric_beams(&self, scatter: Scatter) -> Vec<LocalLight> {
        if self.blackout { return Vec::new(); }
        let flood = self.flood * scatter.beam;
        // The pickups' haze last: the renderer takes at most 32 scattering
        // lights, so when the lava glows fill the budget the floods keep theirs.
        let haze = self.things.pickup_haze().iter().map(|b| LocalLight { intensity_cd: (b.intensity_cd * scatter.pickup).min(Vec3::splat(1.0e8)), ..*b });
        // Shadowed floodlights still light the haze, as when they were local lights.
        let floods = self.stadium.lights().iter().filter(|_| self.flood_shadows()).map(|f| LocalLight { intensity_cd: f.intensity_cd * self.flood * scatter.flood, ..*f });
        self.stadium.beams().iter().map(|b| LocalLight { intensity_cd: b.intensity_cd * flood, ..*b }).chain(floods).chain(haze).collect()
    }

    /// Floodlight strength for the sky (1 for Godot's day).
    /// The lava's night glows fade in with them.
    pub fn set_floodlights(&mut self, flood: f32) {
        self.flood = flood;
        self.arena.lava.night = ((flood - 1.0) / (crate::sky::NIGHT_FLOODLIGHTS - 1.0)).clamp(0.0, 1.0);
        self.things.set_night(self.arena.lava.night);
    }

    /// Short, distance-attenuated ion impact flash, separately per viewport.
    pub fn ion_flash(&self, eye: Vec3) -> f32 { self.ion.flash(self.clock, eye) }
    /// The incoming air-strike rocket's lens flare over one view.
    pub fn airstrike_flare(&self, list: &mut render::hud::HudDrawList, rect: [f32; 4], cam: &Camera, world: &World) { things::Things::airstrike_flare(list, rect, cam, world, self.clock) }

    /// The air strike impact's white-out (Godot `trigger_impact_flash`: two
    /// frames of an additive wash over every view), as an overlay alpha.
    pub fn air_impact_flash(&self) -> f32 {
        let t = self.clock - self.air_impact;
        if !(t > -growing_guns::DT) { return 0.0; }
        let t = t.max(0.0);
        if t < 1.0 / 30.0 { 1.0 } else { (1.0 - (t - 1.0 / 30.0) / 0.06).clamp(0.0, 1.0) }
    }
}

/// Godot's sky_top to horizon; the ground colour is the darkened horizon.
pub fn sky() -> (u32, u32, Vec<[f32; 4]>) { sky_for(false) }

/// Godot's sky, or (`inferno`) the island arenas' red-orange one.
pub fn sky_for(inferno: bool) -> (u32, u32, Vec<[f32; 4]>) {
    let (top, horizon) = if inferno { (palette::INFERNO_SKY_TOP, palette::INFERNO_HORIZON) } else { (palette::SKY_TOP, palette::HORIZON) };
    primitives::gradient_sky(top, horizon, horizon.map(|c| c * 0.79))
}

/// `lighting`, with the island arenas' warmer, stronger sun if `inferno`.
pub fn lighting_for(inferno: bool) -> Lighting {
    let mut l = lighting();
    if inferno {
        l.sun_color = Vec3::from(palette::INFERNO_SUN);
        l.sun_lux *= palette::INFERNO_SUN_ENERGY;
    }
    l
}

/// How much emission follows the exposure (render::Lighting): 1 shows it
/// as at Godot's EV 13.6 under every sky.
pub const EMISSIVE_COMPENSATION: f32 = 1.0;

pub fn lighting() -> Lighting {
    let (el, az) = (38f32.to_radians(), 140f32.to_radians());
    Lighting {
        sun_dir: Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()),
        sun_lux: 60_000.0,
        sun_disc: true,
        sun_radius: 0.0093,
        sun_color: Vec3::from(palette::SUN),
        sky_lux: 14_000.0,
        lod_pixels: crate::graphics::get().lod_pixels,
        gi: if crate::graphics::get().sky_occlusion { 1.0 } else { 0.0 },
        near_shadows: if crate::graphics::get().simple_blocks { 0.0 } else { 1.0 },
        ev100: 13.6,
        // Emission (lava, tracers, beacons) is tuned at Godot's exposure; a
        // darker sky lowers the exposure, and the glow would clip to white.
        emissive_compensation: EMISSIVE_COMPENSATION,
        emissive_ev100: 13.6,
        // Godot's blast fire and smoke are unshaded: as bright at night
        // (exposed for the floodlights) as by day, never black.
        fx_white_floor: 1.0,
        fog_start: 40.0,
        fog_end: 320.0,
        ..Default::default()
    }
}

/// A player's drawn state: position and look (recoil included). Drawing
/// blends the previous tick's into the current one, so a 120 Hz screen sees
/// turning as smooth as moving (the 60 Hz look used to judder).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose { pub pos: Vec3, pub yaw: f32, pub pitch: f32 }

impl Pose {
    pub fn of(p: &growing_guns::Player) -> Self { Pose { pos: p.pos, yaw: p.yaw, pitch: p.pitch + p.recoil_pitch } }
    pub fn all(world: &World) -> Vec<Self> { world.players.iter().map(Self::of).collect() }
}

/// Player `i` blended from `prev` (the previous tick) by `alpha`.
pub fn pose(world: &World, i: usize, prev: &[Pose], alpha: f32) -> Pose {
    let now = Pose::of(&world.players[i]);
    let Some(q) = prev.get(i).filter(|q| q.pos.distance_squared(now.pos) < 36.0) else { return now };
    let turn = (now.yaw - q.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    Pose { pos: q.pos.lerp(now.pos, alpha), yaw: q.yaw + turn * alpha, pitch: q.pitch + (now.pitch - q.pitch) * alpha }
}

pub fn camera(world: &World, i: usize, prev: &[Pose], alpha: f32) -> Camera {
    let p = &world.players[i];
    let drawn = pose(world, i, prev, alpha);
    let pos = drawn.pos;
    if !p.alive && !p.ghost {
        // Spectate from above the arena centre.
        let a = (world.tick as f32 - 1.0 + alpha) * 0.003 + i as f32;
        let r = world.arena.size * 0.55;
        return Camera { pos: Vec3::new(a.cos() * r, 22.0, a.sin() * r), target: Vec3::ZERO, fov_y: FOV_Y, near: 0.05 };
    }
    // The player's own model is hidden from this view (hide_own_bodies).
    let eye = pos + Vec3::Y * p.eye_height();
    let eye = if p.howitzer.is_some() {
        let forward=growing_guns::look_dir(drawn.yaw,0.0);
        eye - forward*2.0 - forward.cross(Vec3::Y)*1.35 + Vec3::Y*0.8
    } else { eye };
    if p.howitzer.is_some() && p.siege_crossbow && p.zooming {
        let dir=growing_guns::look_dir(drawn.yaw,p.artillery_elevation(drawn.pitch));
        let muzzle=pos+Vec3::Y*(p.eye_height()-0.45)+dir*3.3;
        return Camera { pos:muzzle, target:muzzle+dir, fov_y:30f32.to_radians(), near:0.05 };
    }
    // Sniper zoom (Godot FOV 30 instead of 75).
    let fov_y = if p.zooming { 30f32.to_radians() } else { FOV_Y };
    Camera { pos: eye, target: eye + growing_guns::look_dir(drawn.yaw, drawn.pitch), fov_y, near: 0.05 }
}

fn flashlight_owner_ranges(world: &World, first: usize) -> Vec<(u32, u32)> {
    world.players.iter().enumerate().filter(|(_, p)| p.alive)
        .take(render::headlights::MAX_LIGHTS)
        .map(|(i, _)| ((first + i * players::PARTS) as u32, players::PARTS as u32)).collect()
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    #[test]
    fn flashlight_owners_keep_player_indices_when_dead_players_are_skipped() {
        let mut world = World::new(3, &[false; 4]);
        for p in &mut world.players { p.alive = true; }
        world.players[1].alive = false;
        world.players[1].ghost = true;
        let ranges = flashlight_owner_ranges(&world, 100);
        let part_count = players::PARTS as u32;
        assert_eq!(ranges, vec![(100, part_count), (100 + 2 * part_count, part_count), (100 + 3 * part_count, part_count)]);
        assert_eq!(ranges.len(), Visuals::flashlights(&world, &[], 1.0).len());
        for p in &mut world.players { p.alive = false; }
        assert!(flashlight_owner_ranges(&world, 100).is_empty());
    }

    #[test]
    fn turning_is_blended_between_ticks_the_short_way_round() {
        let mut world = World::new(3, &[false; 2]);
        world.players[0].yaw = -3.0;
        let before = [Pose { yaw: 3.0, ..Pose::of(&world.players[0]) }];
        let half = pose(&world, 0, &before, 0.5).yaw;
        // From 3.0 to -3.0 passes through pi, not through zero.
        assert!((half.rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs() < 1e-4, "{half}");
        assert!((pose(&world, 0, &before, 1.0).yaw.sin() - (-3.0f32).sin()).abs() < 1e-5);
    }

    #[test]
    fn ghost_keeps_the_players_interpolated_eye_and_look() {
        let mut world = World::new(3, &[false; 3]);
        let p = &mut world.players[0];
        p.alive = false;
        p.ghost = true;
        p.pos = Vec3::new(4.0, 2.0, -6.0);
        p.yaw = 0.7;
        p.pitch = -0.3;
        let previous = p.pos - Vec3::X;
        let eye = previous.lerp(p.pos, 0.5) + Vec3::Y * p.eye_height();
        let direction = p.forward();
        let before = [Pose { pos: previous, ..Pose::of(&world.players[0]) }];
        let cam = camera(&world, 0, &before, 0.5);
        assert_eq!(cam.pos, eye);
        assert!((cam.target - cam.pos - direction).length() < 1e-6);
        world.players[0].ghost = false;
        assert_ne!(camera(&world, 0, &before, 0.5).pos, eye);
    }
}
