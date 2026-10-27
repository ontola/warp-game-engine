//! Turns the simulation into a Warp scene: arena blocks as instanced unit
//! cubes with the castle palette, lava, players, bullet tracers, sparks and
//! lights, plus one camera per local player.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::shape::Kind;
use growing_guns::{Event, World, EYE};
use render::primitives;
use render::headlights::LocalLight;
use render::scene::{GpuInstance, Material, EMISSIVE};
use render::{Camera, Lighting, Scene};

pub const TRACERS: usize = 64;
pub const SPARKS: usize = 48;
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
    spark: usize,
    sparks: Vec<(Vec3, f32)>,
    next_spark: usize,
    flashes: Vec<(Vec3, f32)>,
    lights: Vec<LocalLight>,
}


impl Visuals {
    pub fn build(world: &World) -> Self {
        let mut scene = Scene::default();
        scene.sky = Some(sky());
        let kind_mesh = |scene: &mut Scene, tint, rough| { let m = material(scene, tint, rough); cube(scene, m) };
        let floor = kind_mesh(&mut scene, palette::FLOOR, 0.9);
        let wall = kind_mesh(&mut scene, palette::WALL, 0.85);
        let building = kind_mesh(&mut scene, palette::BUILDING, 0.85);
        let dark = kind_mesh(&mut scene, palette::DARK, 0.9);
        let accent_a = kind_mesh(&mut scene, palette::ACCENT_A, 0.7);
        let accent_b = kind_mesh(&mut scene, palette::ACCENT_B, 0.7);
        for b in &world.arena.blocks {
            let m = match b.kind {
                Kind::Floor => floor,
                Kind::Wall => wall,
                Kind::Tower | Kind::Pillar => dark,
                Kind::Building => building,
                Kind::Ledge => accent_b,
                Kind::Cover => accent_a,
                Kind::Platform => accent_b,
            };
            scene.add_instance(m, Affine3A::from_scale_rotation_translation(b.half * 2.0, Quat::from_rotation_y(b.yaw), b.center));
            scene.mark_solid();
        }
        // Lava: a glowing sheet under the floor, visible through holes and
        // past the edge of open arenas.
        let lava_m = glow(&mut scene, [1.0, 0.28, 0.04], 2600.0);
        let lava = cube(&mut scene, lava_m);
        scene.add_instance(lava, Affine3A::from_scale_rotation_translation(Vec3::new(900.0, 0.2, 900.0), Quat::IDENTITY, Vec3::new(0.0, -2.3, 0.0)));
        scene.mark_no_shadow();
        // Roof beacons as small glowing cubes where the Godot lights hang.
        for &(p, c) in &world.arena.lights {
            let m = glow(&mut scene, c, 6000.0);
            let beacon = cube(&mut scene, m);
            scene.add_instance(beacon, Affine3A::from_scale_rotation_translation(Vec3::splat(0.5), Quat::IDENTITY, p));
            scene.mark_no_shadow();
        }

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
        let spark_m = glow(&mut scene, [1.0, 0.6, 0.2], 20000.0);
        let spark = cube(&mut scene, spark_m);
        for _ in 0..SPARKS { scene.add_instance(spark, Affine3A::from_translation(HIDDEN)); scene.mark_no_shadow(); }

        let lights = world.arena.lights.iter().take(12).map(|&(p, c)| omni(p, Vec3::from(c) * 900.0, 20.0)).collect();
        Visuals { scene, first_dynamic, players, tracer, spark, sparks: vec![(HIDDEN, 0.0); SPARKS], next_spark: 0, flashes: Vec::new(), lights }
    }

    /// Advance effects from this tick's events.
    pub fn absorb(&mut self, world: &World) {
        for e in &world.events {
            match *e {
                Event::Impact { pos, normal } => self.spark(pos + normal * 0.05),
                Event::Hit { pos, .. } => self.spark(pos),
                Event::Shot { from, dir, .. } => self.flashes.push((from + dir * 0.8, 0.06)),
                _ => {}
            }
        }
    }

    fn spark(&mut self, p: Vec3) {
        self.sparks[self.next_spark] = (p, 0.15);
        self.next_spark = (self.next_spark + 1) % SPARKS;
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
        for s in &mut self.sparks {
            s.1 -= dt;
            let xf = if s.1 > 0.0 { Affine3A::from_scale_rotation_translation(Vec3::splat(0.12 + s.1), Quat::IDENTITY, s.0) } else { Affine3A::from_translation(HIDDEN) };
            self.scene.update_instance(slot, self.spark, xf);
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

/// Equirectangular gradient: Godot's sky_top to horizon, ground below.
fn sky() -> (u32, u32, Vec<[f32; 4]>) {
    let (w, h) = (64u32, 32u32);
    let top = Vec3::from(srgb_to_linear(palette::SKY_TOP));
    let hor = Vec3::from(srgb_to_linear(palette::HORIZON));
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let el = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * (y as f32 + 0.5) / h as f32;
        let c = if el > 0.0 { hor.lerp(top, (el / 1.2).min(1.0).powf(0.6)) } else { hor * 0.6 };
        for _ in 0..w { px.push([c.x, c.y, c.z, 1.0]); }
    }
    (w, h, px)
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
