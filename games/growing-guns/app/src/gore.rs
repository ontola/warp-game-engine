//! Gibs, after Growing Guns' `violence.gd`: a body killed by an explosion
//! or by overkill bursts into Voronoi chunks of its own meshes (flesh caps
//! on every cut, `render::primitives::shatter`) plus flying blood and meat
//! blobs; a headshot bursts the head and the torso tumbles on. Pieces are
//! rigid bodies that bounce through the arena, then go after their time or
//! when newer gore needs the room (Godot caps live chunks the same way).
use glam::{Affine3A, Quat, Vec3};
use physics::{Body, BodyDesc, Physics, Shape};
use render::primitives::{self, Primitive};
use render::scene::{Material, NO_SHADOW};
use render::slots::{SlotId, Slots};
use render::Scene;

/// Live gore pieces at once (Godot: 32 rigid chunks; Rapier takes more).
pub const MAX_PIECES: usize = 160;
const VARIANTS: usize = 3;
/// Godot GIB_CHUNK_COUNT for the blob's primitive meshes.
const CELLS: usize = 3;
const LIFETIME: f32 = 12.0;

/// A body part that can burst: where it sits on the player (scale, offset
/// from the player position in player yaw space) and its cached pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part { Body, Head, Eyes }

#[derive(Clone, Debug)]
struct ShardMesh { mesh: usize, centroid: Vec3, half: Vec3 }

struct Piece { body: Body, slot: SlotId, mesh: usize, scale: Vec3, born: f32 }

pub struct Gore {
    /// Per player, per part, a few shatter layouts.
    shards: Vec<[Vec<Vec<ShardMesh>>; 3]>,
    blobs: [usize; 4],
    slots: Slots,
    pieces: Vec<Piece>,
    rng: u64,
}

/// The source part's transform on a player at `pos` facing `yaw`.
pub fn part_xf(part: Part, pos: Vec3, yaw: f32) -> Affine3A {
    let rot = Quat::from_rotation_y(yaw);
    match part {
        Part::Body => Affine3A::from_scale_rotation_translation(Vec3::new(0.42, 0.5, 0.42), rot, pos + Vec3::Y * -0.25),
        Part::Head => Affine3A::from_scale_rotation_translation(Vec3::splat(0.3), rot, pos + Vec3::Y * 0.55),
        Part::Eyes => Affine3A::from_scale_rotation_translation(Vec3::new(0.2, 0.07, 0.05), rot, pos + Vec3::Y * 0.6 + rot * Vec3::new(0.0, 0.0, -0.27)),
    }
}

impl Gore {
    /// `parts[p]`: player p's body, head and eye primitives (its materials).
    pub fn reserve(scene: &mut Scene, parts: &[[Primitive; 3]], seed: u64) -> Self {
        // Flesh: Godot's cap colours run from deep to raw red.
        let flesh: Vec<u32> = [[0.45, 0.06, 0.05], [0.62, 0.11, 0.08], [0.82, 0.16, 0.11]]
            .map(|c| scene.add_material(Material { tint: c.map(|v: f32| v.powf(2.2)), layer: -1, roughness: 0.9, flags: 0, scale: 1.0, metal: 0.0 })).to_vec();
        let mut shards = Vec::new();
        for (p, prims) in parts.iter().enumerate() {
            let per: [Vec<Vec<ShardMesh>>; 3] = std::array::from_fn(|k| (0..VARIANTS).map(|v| {
                primitives::shatter(&prims[k], CELLS, flesh[(p + k + v) % flesh.len()], seed ^ ((p * 31 + k * 7 + v) as u64 + 1))
                    .into_iter().map(|s| ShardMesh { mesh: scene.add_mesh(&s.mesh()), centroid: s.centroid, half: s.half }).collect()
            }).collect());
            shards.push(per);
        }
        // Blood and meat blobs (Godot spawn_blood_gib_blobs): dark blood and flesh.
        let blobs = [[0.30, 0.018, 0.012], [0.20, 0.016, 0.012], [0.92, 0.18, 0.12], [0.55, 0.09, 0.07]].map(|c| {
            let m = scene.add_material(Material { tint: c.map(|v: f32| v.powf(2.2)), layer: -1, roughness: 0.4, flags: 0, scale: 1.0, metal: 0.0 });
            scene.add_mesh(&primitives::sphere(m, 3, 5).mesh())
        });
        let slots = Slots::reserve(scene, MAX_PIECES);
        Gore { shards, blobs, slots, pieces: Vec::new(), rng: seed | 1 }
    }

    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }

    pub fn len(&self) -> usize { self.pieces.len() }

    fn spawn(&mut self, scene: &mut Scene, phys: &mut Physics, mesh: usize, scale: Vec3, shape: Shape, pos: Vec3, rot: Quat, vel: Vec3, now: f32) {
        if self.pieces.len() >= MAX_PIECES {
            let old = self.pieces.remove(0);
            phys.remove(old.body);
            self.slots.remove(scene, old.slot);
        }
        let spin = Vec3::new(self.range(-10.0, 10.0), self.range(-10.0, 10.0), self.range(-10.0, 10.0));
        let body = phys.add(&BodyDesc { rot, vel, spin, density: 900.0, restitution: 0.15, friction: 0.8, damping: 0.1, ccd: true, ..BodyDesc::new(shape, pos) });
        let Some(slot) = self.slots.add(scene, mesh, Affine3A::from_scale_rotation_translation(scale, rot, pos)) else { phys.remove(body); return };
        let i = self.slots.instance(slot);
        scene.instances[i].flags |= NO_SHADOW;
        self.pieces.push(Piece { body, slot, mesh, scale, born: now });
    }

    /// Burst `parts` of player `p` (at `pos`, facing `yaw`). `dir` is the
    /// killing push, `chaos` 0..1.1 how violent (Godot `spawn_ragdoll`).
    #[allow(clippy::too_many_arguments)]
    pub fn burst(&mut self, scene: &mut Scene, phys: &mut Physics, p: usize, parts: &[Part], pos: Vec3, yaw: f32, vel: Vec3, dir: Vec3, chaos: f32, now: f32) {
        let kb = 1.6;
        let dir = dir.try_normalize().unwrap_or(Vec3::Y);
        let base = vel * 0.5 + dir * self.range(2.2, 3.9) * kb * (1.0 + chaos * 0.18) + Vec3::Y * self.range(1.2, 2.8) * kb * (1.2 + chaos * 0.06);
        let strength = 1.45 * kb * (1.0 + chaos * 0.2);
        let rot = Quat::from_rotation_y(yaw);
        for &part in parts {
            let k = part as usize;
            let v = (self.f() * VARIANTS as f32) as usize % VARIANTS;
            let src = part_xf(part, pos, yaw);
            let scale = Vec3::new(src.matrix3.x_axis.length(), src.matrix3.y_axis.length(), src.matrix3.z_axis.length());
            let shards = self.shards[p.min(self.shards.len() - 1)][k][v].clone();
            for s in shards {
                let at = src.transform_point3(s.centroid);
                let out = (at - pos - Vec3::Y * 0.1).try_normalize().unwrap_or(Vec3::Y);
                let jitter = Vec3::new(self.range(-1.5, 1.5), 0.0, self.range(-1.5, 1.5));
                let vel = base + jitter + out * strength + Vec3::Y * strength * 0.2;
                let half = (s.half * scale).max(Vec3::splat(0.025));
                self.spawn(scene, phys, s.mesh, scale, Shape::Box(half), at, rot, vel, now);
            }
        }
        // Blood and meat blobs, more for worse deaths.
        let sev = (0.9 + chaos * 1.6).clamp(0.35, 2.8);
        let n = ((5.0 + 15.0 * sev / 2.5 + chaos * 6.0).round() as usize).clamp(4, 24);
        for _ in 0..n {
            let r = self.range(0.035, 0.13) * (0.85 + 0.7 * sev / 2.5);
            let at = pos + Vec3::new(self.range(-0.35, 0.35), self.range(-0.15, 0.45), self.range(-0.35, 0.35));
            let out = (at - pos).try_normalize().unwrap_or(dir);
            let b = strength * self.range(0.85, 1.35) * (1.0 + chaos * 0.22);
            let vel = base + out * b + Vec3::Y * b * self.range(0.15, 0.35);
            let mesh = self.blobs[if self.f() < 0.38 { 2 + (self.f() * 2.0) as usize % 2 } else { (self.f() * 2.0) as usize % 2 }];
            let scale = Vec3::new(self.range(0.75, 1.25), self.range(0.65, 1.05), self.range(0.75, 1.25)) * r;
            self.spawn(scene, phys, mesh, scale, Shape::Sphere(r), at, rot, vel, now);
        }
    }

    /// Follow the physics; retire pieces past their time.
    pub fn update(&mut self, scene: &mut Scene, phys: &mut Physics, now: f32) {
        let mut k = 0;
        while k < self.pieces.len() {
            let pc = &self.pieces[k];
            if now - pc.born > LIFETIME {
                let pc = self.pieces.remove(k);
                phys.remove(pc.body);
                self.slots.remove(scene, pc.slot);
                continue;
            }
            if let Some((pos, rot)) = phys.pose(pc.body) {
                // Shrink away over the last second.
                let fade = ((LIFETIME - (now - pc.born)) / 1.0).clamp(0.0, 1.0);
                self.slots.set(scene, pc.slot, pc.mesh, Affine3A::from_scale_rotation_translation(pc.scale * fade.max(0.01), rot, pos));
            }
            k += 1;
        }
    }

    pub fn clear(&mut self, scene: &mut Scene, phys: &mut Physics) {
        for pc in self.pieces.drain(..) { phys.remove(pc.body); self.slots.remove(scene, pc.slot); }
    }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene) -> Result<(), String> { self.slots.flush(gpu, renderer, scene) }
}
