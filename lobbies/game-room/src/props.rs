//! Items, shots and effects in the live lobby (view.gd's `_update_items`,
//! `_update_shots`, `_event`, `burst` and `_beam`, room.gd's `flash`,
//! `fireball` and camera shake). The props come from `kit`; each frame
//! they are laid out again into two blocks of instance slots.

use crate::dump::{billboard, linear, Room};
use crate::godot::Color;
use crate::kit::{self, Kit, FIREBALLS, SABER_BLADES};
use crate::room::{CAMERA_BASE, CAMERA_TARGET};
use game_room_sim::world::{self, Ev, Event, Kind, World, ITEM_LIFE};
use gdcompat::Pcg;
use glam::{Affine3A, Mat4, Quat, Vec3};
use render::headlights::LocalLight;
use render::scene::{Scene, NO_SHADOW};
use render::slots::Slots;
use std::collections::HashMap;
use std::f32::consts::PI;

const ITEM_SLOTS: usize = 320;
const FX_SLOTS: usize = 640;

struct Spark { quad: usize, pos: Vec3, vel: Vec3, damping: f32, gravity: f32, size: f32, age: f32, life: f32 }
struct Flash { pos: Vec3, color: Color, energy: f32, start: f32, range: f32 }
struct Fireball { pos: Vec3, age: f32 }
struct Beam { x0: f32, x1: f32, y: f32, age: f32 }

pub struct Props {
    kit: Kit,
    items: Slots,
    fx: Slots,
    sparks: Vec<Spark>,
    flashes: Vec<Flash>,
    /// Per fireball kind, per pool member.
    fireballs: Vec<Vec<Option<Fireball>>>,
    beam: Option<Beam>,
    /// Item positions at the last tick, for interpolation.
    pub prev: HashMap<u32, [f64; 2]>,
    /// Each saber's blade angle.
    blades: HashMap<u32, f32>,
    shake: f32,
    rng: Pcg,
    /// Kit instances moved this frame (fireballs, beam).
    moved: Vec<usize>,
    faded: Vec<usize>,
}

/// An omni light as room.gd makes it.
fn omni(pos: Vec3, color: Color, energy: f32, range: f32, light: f32) -> LocalLight {
    LocalLight { position: pos, direction: Vec3::NEG_Y, intensity_cd: linear(&serde_json::json!(color.array())) * energy * light,
        range_m: range, inner_cos: -0.999, outer_cos: -1.0, source_radius_m: 0.05, falloff: 1.0 }
}

/// Godot's `ease(t, curve)` for 0 < curve < 1.
fn ease(t: f32, curve: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if curve > 0.0 && curve < 1.0 { 1.0 - (1.0 - t).powf(1.0 / curve) } else { t.powf(curve) }
}

impl Props {
    /// The kit is already in the room; reserve slots (call from Stage's extend).
    pub fn reserve(kit: Kit, scene: &mut Scene, room: &Room) -> Self {
        let items = Slots::reserve(scene, ITEM_SLOTS);
        let fx = Slots::reserve(scene, FX_SLOTS);
        for i in &mut scene.instances[fx.first..fx.first + FX_SLOTS] { i.flags |= NO_SHADOW; }
        for pool in &kit.fireballs {
            for &(instance, material) in pool {
                if let Some(&id) = room.material_ids.get(&material) { scene.set_material_additive(id, 1.0, false); }
                if let Some(i) = room.scene_index[instance] { scene.instances[i].flags |= NO_SHADOW; }
            }
        }
        if let Some(i) = room.scene_index[kit.beam] { scene.instances[i].flags |= NO_SHADOW; }
        let fireballs = kit.fireballs.iter().map(|p| p.iter().map(|_| None).collect()).collect();
        Self { kit, items, fx, sparks: vec![], flashes: vec![], fireballs, beam: None, prev: HashMap::new(), blades: HashMap::new(),
            shake: 0.0, rng: Pcg::new(9), moved: vec![], faded: vec![] }
    }

    fn randf(&mut self) -> f32 { self.rng.randf() as f32 }

    /// view.gd `burst`: a one-shot puff of `amount` quads.
    fn burst(&mut self, pos: Vec3, amount: usize, palette: &str, speed: f32, life: f32, size: f32) {
        let quads = self.kit.sparks[palette].clone();
        for _ in 0..amount {
            // Spread 180: any direction on the sphere.
            let z = self.randf() * 2.0 - 1.0;
            let a = self.randf() * PI * 2.0;
            let dir = Vec3::new((1.0 - z * z).sqrt() * a.cos(), z, (1.0 - z * z).sqrt() * a.sin());
            let v = speed * (0.4 + 0.6 * self.randf());
            let quad = quads[((self.randf() * quads.len() as f32) as usize).min(quads.len() - 1)];
            // Explosiveness 0.95: all within the first 5% of the lifetime.
            let age = -self.randf() * life * 0.05;
            let damping = 1.0 + self.randf() * 2.0;
            let scale = 0.6 + self.randf() * 0.6;
            self.sparks.push(Spark { quad, pos, vel: dir * v, damping, gravity: -3.0, size: size * scale, age, life });
        }
    }

    fn flash(&mut self, pos: Vec3, color: &str, energy: f32, range: f32) { self.flash_color(pos, Color::hex(color), energy, range); }
    fn flash_color(&mut self, pos: Vec3, color: Color, energy: f32, range: f32) {
        self.flashes.push(Flash { pos, color, energy, start: energy, range });
    }

    fn fireball(&mut self, kind: usize, pos: Vec3) {
        let pool = &mut self.fireballs[kind];
        // A free member, or the oldest.
        let k = pool.iter().position(Option::is_none).unwrap_or_else(|| {
            (0..FIREBALLS).max_by(|&a, &b| pool[a].as_ref().unwrap().age.total_cmp(&pool[b].as_ref().unwrap().age)).unwrap()
        });
        pool[k] = Some(Fireball { pos, age: 0.0 });
    }

    fn shake(&mut self, amount: f32) { self.shake = self.shake.max(amount); }

    /// view.gd `_event`.
    pub fn event(&mut self, e: &Event) {
        let pos = Vec3::new(e.x as f32, e.y as f32, 0.2);
        match e.ev {
            Ev::Explode => {
                self.flash(pos, "ffb454", 16.0, 9.0);
                self.fireball(0, pos);
                self.fireball(1, pos);
                self.shake(0.9);
                self.burst(pos, 60, "fire", 9.0, 0.7, 0.09);
                self.burst(pos, 26, "smoke", 3.0, 1.4, 0.22);
            }
            Ev::Hit => { self.burst(pos, 14, "hit", 5.0, 0.35, 0.06); self.shake(0.25); }
            Ev::Fire => match e.kind {
                Some(Kind::Popcorn) => self.burst(pos, 6, "popcorn", 2.0, 0.3, 0.05),
                Some(Kind::Bazooka) => {
                    self.flash(pos, "ff7a3c", 6.0, 4.0);
                    self.burst(pos, 18, "puff", 2.5, 0.8, 0.14);
                    self.shake(0.3);
                }
                Some(Kind::Minigun) => self.flash(pos, "ffd27a", 1.5, 2.0),
                _ => { let h = self.randf(); self.flash_color(pos, Color::from_hsv(h, 0.7, 1.0), 2.5, 2.5); }
            },
            Ev::Laser => {
                self.beam = Some(Beam { x0: e.x as f32, x1: e.value as f32, y: e.y as f32, age: 0.0 });
                self.flash(pos, "38d6ff", 6.0, 4.0);
                self.shake(0.35);
            }
            Ev::Slash => { self.burst(pos, 22, "slash", 6.0, 0.25, 0.05); self.flash(pos, "38d6ff", 3.0, 3.0); }
            Ev::Land => if e.value > 14.0 { self.burst(pos, 10, "dust", 2.0, 0.4, 0.06) },
            Ev::WallJump | Ev::Jump => self.burst(pos, 5, "dust", 1.4, 0.3, 0.05),
            Ev::Spawn => {
                let up = pos + Vec3::new(0.0, 0.6, 0.0);
                self.burst(up, 24, "spawn", 3.5, 0.6, 0.06);
                self.flash(up, "7c5cff", 5.0, 3.0);
            }
            _ => {}
        }
    }

    /// The camera's sway and shake (room.gd `_process`).
    pub fn camera(&mut self, camera: &mut render::Camera, time: f32, delta: f32) {
        self.shake = (self.shake - delta * 1.8).max(0.0);
        let sway = Vec3::new((time * 0.21).sin() * 0.25, (time * 0.17).sin() * 0.12, 0.0);
        let jitter = Vec3::new(self.randf() * 2.0 - 1.0, self.randf() * 2.0 - 1.0, 0.0) * self.shake * 0.25;
        camera.pos = CAMERA_BASE + sway + jitter;
        camera.target = CAMERA_TARGET + sway * 0.5;
    }

    /// Lay out every prop for this frame; returns the lights they cast:
    /// flashes and items, then the shots'.
    pub fn update(&mut self, world: &World, room: &mut Room, scene: &mut Scene, alpha: f32, delta: f32, time: f32, light: f32) -> (Vec<(LocalLight, f32)>, Vec<(LocalLight, f32)>) {
        let mut lights = vec![];
        self.items.clear(scene);
        self.fx.clear(scene);
        self.moved.clear();
        self.faded.clear();
        let mesh = |room: &Room, instance: usize| room.scene_mesh[instance].expect("kit mesh");

        // Flashes fade at 3.5 times their energy per second.
        for f in &mut self.flashes { f.energy = (f.energy - f.start * delta * 3.5).max(0.0); }
        self.flashes.retain(|f| f.energy > 0.01);
        for f in &self.flashes { lights.push((omni(f.pos, f.color, f.energy, f.range, light), 2.0)); }

        // Items.
        let mut item_lights = vec![];
        self.blades.retain(|id, _| world.items.iter().any(|i| i.id == *id));
        for item in &world.items {
            let [px, py] = self.prev.get(&item.id).copied().unwrap_or([item.x, item.y]);
            let x = px + (item.x - px) * alpha as f64;
            let y = py + (item.y - py) * alpha as f64;
            let holder = item.holder.as_ref().and_then(|h| world.player(h));
            if item.holder.is_none() && item.age > ITEM_LIFE - 3.0 && (time * 8.0) as i64 % 2 == 0 && item.kind != Kind::Ball { continue; }
            let key = kit::item_key(item.kind, item.id);
            let mut yaw = 0.0;
            let mut scale = 1.0;
            let mut turn = 0.0;
            match item.kind {
                Kind::Ball => turn = -item.spin as f32,
                Kind::Bomb if item.fuse >= 0.0 => {
                    let period = if item.fuse > 1.0 { 0.4 } else { 0.15 };
                    if item.fuse.rem_euclid(period) < 0.07 { scale = 1.12; }
                }
                _ => {}
            }
            let weapon = world::WEAPONS.iter().any(|(k, _)| *k == item.kind);
            if weapon {
                let facing = holder.map_or(1.0, |p| p.facing);
                yaw = if facing > 0.0 { 0.0 } else { PI };
            }
            if item.kind == Kind::Saber {
                // Swing over the top while the slash lasts, then ease back.
                let swing = holder.map_or(0.0, |p| p.punch) as f32;
                let blade = self.blades.entry(item.id).or_insert(-0.5);
                if swing > 0.0 {
                    let t = 1.0 - swing / 0.24;
                    *blade = 1.4 + (-2.3 - 1.4) * ease(t, 0.4);
                } else {
                    *blade += (-0.5 - *blade) * (1.0 - (-delta * 14.0).exp());
                }
                turn = *blade;
            }
            let node = Affine3A::from_scale_rotation_translation(Vec3::splat(scale), Quat::from_rotation_y(yaw), Vec3::new(x as f32, y as f32, 0.05));
            let pivot = node * Affine3A::from_translation(self.kit.pivots.get(&key).copied().unwrap_or(Vec3::ZERO)) * Affine3A::from_rotation_z(turn);
            for part in &self.kit.items[&key] {
                let at = if part.pivot { pivot } else { node } * part.local;
                if self.items.add(scene, mesh(room, part.instance), at).is_none() { break; }
            }
            match item.kind {
                Kind::Bomb if item.fuse >= 0.0 => {
                    let energy = 1.5 + self.randf() * 2.0;
                    item_lights.push((omni(node.transform_point3(Vec3::new(0.05, 0.8, 0.1)), Color::hex("ffb454"), energy, 1.8, light), 1.0));
                }
                Kind::Saber => {
                    let blade = Color::hex(SABER_BLADES[item.id as usize % 4]);
                    item_lights.push((omni(pivot.transform_point3(Vec3::new(0.0, 0.65, 0.1)), blade, 1.2, 1.8, light), 1.0));
                }
                _ => {}
            }
        }
        lights.extend(item_lights);

        // Shots, eased back along their path to where they were at alpha.
        let back = (1.0 - alpha as f64) * world::DT;
        let mut shot_lights = vec![];
        for shot in &world.shots {
            let pos = Vec3::new((shot.x - shot.vx * back) as f32, (shot.y - shot.vy * back) as f32, 0.1);
            let core = self.kit.shots[&kit::shot_key(shot.kind, shot.hue)];
            let s = match shot.kind {
                Kind::Minigun => Vec3::new(2.4, 0.4, 0.4),
                Kind::Bazooka => Vec3::new(3.2, 1.3, 1.3),
                Kind::Popcorn => Vec3::splat(0.75),
                _ => Vec3::new(2.2, 0.8, 0.8),
            };
            self.fx.add(scene, mesh(room, core), Affine3A::from_scale_rotation_translation(s, Quat::IDENTITY, pos));
            if shot.kind != Kind::Popcorn {
                let c = match shot.kind {
                    Kind::Minigun => Color::hex("ffd27a"),
                    Kind::Bazooka => Color::hex("ff7a3c"),
                    _ => Color::from_hsv(shot.hue as f32, 0.8, 1.0),
                };
                shot_lights.push((omni(pos, c, 2.0, 2.0, light), 1.0));
            }
        }

        // Particles: damped, falling, facing the camera.
        for s in &mut self.sparks {
            s.age += delta;
            if s.age < 0.0 { continue; }
            let speed = s.vel.length();
            if speed > 0.0 { s.vel *= (speed - s.damping * delta).max(0.0) / speed; }
            s.vel.y += s.gravity * delta;
            s.pos += s.vel * delta;
        }
        self.sparks.retain(|s| s.age < s.life);
        for s in &self.sparks {
            if s.age < 0.0 { continue; }
            let xf = billboard(Mat4::from_scale_rotation_translation(Vec3::new(s.size, s.size, 1.0), Quat::IDENTITY, s.pos), &room.camera);
            let (sc, r, t) = xf.to_scale_rotation_translation();
            if self.fx.add(scene, mesh(room, s.quad), Affine3A::from_scale_rotation_translation(sc, r, t)).is_none() { break; }
        }

        // Fireballs grow fast (cubic ease out over 0.22 s) and fade from 0.08 s.
        for (kind, pool) in self.fireballs.iter_mut().enumerate() {
            for (k, slot) in pool.iter_mut().enumerate() {
                let (instance, material) = self.kit.fireballs[kind][k];
                let Some(f) = slot else { continue };
                f.age += delta;
                let t = (f.age / 0.22).min(1.0);
                let grow = 0.2 + 0.8 * (1.0 - (1.0 - t).powi(3));
                let fade = 1.0 - ((f.age - 0.08) / 0.35).clamp(0.0, 1.0);
                let done = f.age >= 0.43;
                let s = if done { 0.0 } else { grow };
                set_transform(room, instance, Affine3A::from_scale_rotation_translation(Vec3::splat(s.max(1e-4)), Quat::IDENTITY, f.pos));
                room.dump.materials[material]["albedo"][3] = serde_json::json!(0.75 * fade);
                self.moved.push(instance);
                self.faded.push(material);
                if done { *slot = None; }
            }
        }

        // The laser beam thins out over 0.22 s.
        if let Some(b) = &mut self.beam {
            b.age += delta;
            let t = (b.age / 0.22).min(1.0);
            let thin = 1.0 + (0.05 - 1.0) * t;
            let done = b.age >= 0.22;
            let s = if done { Vec3::splat(1e-4) } else { Vec3::new((b.x1 - b.x0).abs().max(0.01), thin, thin) };
            set_transform(room, self.kit.beam, Affine3A::from_scale_rotation_translation(s, Quat::IDENTITY, Vec3::new((b.x0 + b.x1) / 2.0, b.y, 0.15)));
            self.moved.push(self.kit.beam);
            if done { self.beam = None; }
        }
        (lights, shot_lights)
    }

    /// Send this frame's changes to the renderer.
    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, room: &Room, scene: &mut Scene) -> Result<(), String> {
        self.items.flush(gpu, renderer, scene)?;
        self.fx.flush(gpu, renderer, scene)?;
        for &k in &self.moved {
            if let Some(i) = room.sync_instance(scene, k, true) { renderer.update_instances(gpu, i, &scene.instances[i..i + 1])?; }
        }
        for &m in &self.faded {
            if let Some((id, mut material)) = room.material_now(m) {
                // Additive: the albedo's alpha scales what it adds.
                material.scale *= room.dump.materials[m]["albedo"][3].as_f64().unwrap_or(1.0) as f32;
                renderer.update_materials(gpu, id as usize, &[material])?;
            }
        }
        Ok(())
    }
}

fn set_transform(room: &mut Room, instance: usize, at: Affine3A) {
    room.dump.instances[instance].transform = Mat4::from(at).to_cols_array();
}
