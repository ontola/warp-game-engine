//! Simple, deterministic particles on [`Slots`]: sparks, dust, blood,
//! casings, debris. Each particle is one instance of a mesh already in the
//! scene (a small cube or sphere with an emissive or plain material), so it
//! lights, shadows and culls like everything else. Simulation is on the CPU
//! with an optional collision callback (e.g. the game's raycast).
//!
//! ```ignore
//! let mut fx = Particles::reserve(&mut scene, 512, 7);
//! fx.burst(&mut scene, at, normal, &Burst { mesh: spark, count: 12, ..Burst::sparks() });
//! fx.update(&mut scene, dt, |from, to| game_raycast(from, to));   // per tick
//! fx.flush(&gpu, &renderer, &scene)?;                             // per frame
//! ```
use crate::scene::Scene;
use crate::slots::{SlotId, Slots};
use glam::{Affine3A, Quat, Vec3};

/// How a burst of particles leaves its origin.
#[derive(Clone, Copy, Debug)]
pub struct Burst {
    /// Scene mesh for every particle (its material gives the colour).
    pub mesh: usize,
    pub count: u32,
    /// Initial speed range (m/s) along directions within `spread` radians
    /// of the burst direction (PI: a full sphere).
    pub speed: [f32; 2],
    pub spread: f32,
    /// Seconds to live, random within the range.
    pub life: [f32; 2],
    /// Edge length at birth and death (metres); scaled linearly over life.
    pub size: [f32; 2],
    /// Metres per second squared downwards (negative rises).
    pub gravity: f32,
    /// Fraction of velocity lost per second.
    pub drag: f32,
    /// On collision: 0 sticks, 0..1 bounces with that restitution.
    pub bounce: f32,
    /// Random tumble, radians per second.
    pub spin: f32,
    /// Stretch along velocity (tracer-like streaks): length = size * (1 + stretch * speed).
    pub stretch: f32,
}

impl Burst {
    /// Fast, short, glowing: use an EMISSIVE mesh.
    pub fn sparks() -> Self { Burst { mesh: 0, count: 10, speed: [4.0, 12.0], spread: 0.9, life: [0.08, 0.25], size: [0.05, 0.01], gravity: 9.8, drag: 2.0, bounce: 0.3, spin: 0.0, stretch: 0.02 } }
    /// Slow puffs that hang and shrink.
    pub fn dust() -> Self { Burst { mesh: 0, count: 6, speed: [0.5, 2.0], spread: 1.2, life: [0.4, 0.9], size: [0.12, 0.02], gravity: -0.3, drag: 3.0, bounce: 0.0, spin: 2.0, stretch: 0.0 } }
    /// Heavy drops that fall and stick.
    pub fn drops() -> Self { Burst { mesh: 0, count: 14, speed: [1.5, 5.0], spread: 0.8, life: [1.5, 3.0], size: [0.07, 0.05], gravity: 14.0, drag: 0.5, bounce: 0.0, spin: 0.0, stretch: 0.0 } }
    /// Tumbling pieces that bounce (casings, debris).
    pub fn debris() -> Self { Burst { mesh: 0, count: 1, speed: [2.0, 3.5], spread: 0.4, life: [1.5, 2.5], size: [0.06, 0.06], gravity: 14.0, drag: 0.2, bounce: 0.35, spin: 14.0, stretch: 0.0 } }
}

#[derive(Clone, Debug)]
struct Particle { slot: SlotId, mesh: usize, pos: Vec3, vel: Vec3, rot: Quat, spin: Vec3, age: f32, life: f32, size: [f32; 2], gravity: f32, drag: f32, bounce: f32, stretch: f32, resting: bool }

/// Particles in one slot block.
pub struct Particles {
    slots: Slots,
    live: Vec<Particle>,
    rng: u64,
    /// Bursts that found the pool full (oldest particles are recycled).
    pub recycled: u64,
}

impl Particles {
    /// Reserve `capacity` instances. Call before the renderer exists.
    pub fn reserve(scene: &mut Scene, capacity: usize, seed: u64) -> Self {
        let mut slots = Slots::reserve(scene, capacity);
        slots.mark_no_shadow(scene);
        Self { slots, live: Vec::new(), rng: seed | 1, recycled: 0 }
    }

    pub fn len(&self) -> usize { self.live.len() }
    pub fn is_empty(&self) -> bool { self.live.is_empty() }

    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, r: [f32; 2]) -> f32 { r[0] + (r[1] - r[0]) * self.f() }

    /// A random direction within `spread` of `dir` (uniform over the cap).
    fn cone(&mut self, dir: Vec3, spread: f32) -> Vec3 {
        let d = dir.normalize_or(Vec3::Y);
        let cos = 1.0 - self.f() * (1.0 - spread.min(std::f32::consts::PI).cos());
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        let a = self.f() * std::f32::consts::TAU;
        let (u, v) = d.any_orthonormal_pair();
        (d * cos + (u * a.cos() + v * a.sin()) * sin).normalize()
    }

    /// Emit a burst at `at` towards `dir`, inheriting `base_vel`.
    pub fn burst_with(&mut self, scene: &mut Scene, at: Vec3, dir: Vec3, base_vel: Vec3, b: &Burst) {
        for _ in 0..b.count {
            if self.slots.len() == self.slots.capacity() && !self.live.is_empty() {
                let old = self.live.remove(0);
                self.slots.remove(scene, old.slot);
                self.recycled += 1;
            }
            let Some(slot) = self.slots.add(scene, b.mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(1e-4), Quat::IDENTITY, at)) else { return };
            let vel = self.cone(dir, b.spread) * self.range(b.speed) + base_vel;
            let spin = Vec3::new(self.f() - 0.5, self.f() - 0.5, self.f() - 0.5) * 2.0 * b.spin;
            let life = self.range(b.life);
            self.live.push(Particle { slot, mesh: b.mesh, pos: at, vel, rot: Quat::IDENTITY, spin, age: 0.0, life, size: b.size, gravity: b.gravity, drag: b.drag, bounce: b.bounce, stretch: b.stretch, resting: false });
        }
    }

    pub fn burst(&mut self, scene: &mut Scene, at: Vec3, dir: Vec3, b: &Burst) { self.burst_with(scene, at, dir, Vec3::ZERO, b) }

    /// Advance `dt` seconds. `hit(from, to)` returns the surface normal when
    /// the segment is blocked (the particle stops there, then sticks or
    /// bounces); pass `|_, _| None` for no collision.
    pub fn update(&mut self, scene: &mut Scene, dt: f32, mut hit: impl FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)>) {
        let slots = &mut self.slots;
        self.live.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life { slots.remove(scene, p.slot); return false; }
            if !p.resting {
                p.vel.y -= p.gravity * dt;
                p.vel *= (1.0 - p.drag * dt).max(0.0);
                let next = p.pos + p.vel * dt;
                match hit(p.pos, next) {
                    Some((at, n)) => {
                        p.pos = at + n * 0.01;
                        if p.bounce > 0.0 && p.vel.length() > 1.0 {
                            p.vel = (p.vel - 2.0 * p.vel.dot(n) * n) * p.bounce;
                        } else {
                            p.vel = Vec3::ZERO;
                            p.resting = true;
                            p.spin = Vec3::ZERO;
                        }
                    }
                    None => p.pos = next,
                }
                let w = p.spin * dt;
                if w != Vec3::ZERO { p.rot = (Quat::from_scaled_axis(w) * p.rot).normalize(); }
            }
            let t = p.age / p.life;
            let s = p.size[0] + (p.size[1] - p.size[0]) * t;
            let speed = p.vel.length();
            let (scale, rot) = if p.stretch > 0.0 && speed > 0.1 {
                (Vec3::new(s, s, s * (1.0 + p.stretch * speed)), Quat::from_rotation_arc(Vec3::Z, p.vel / speed))
            } else { (Vec3::splat(s), p.rot) };
            slots.set(scene, p.slot, p.mesh, Affine3A::from_scale_rotation_translation(scale.max(Vec3::splat(1e-4)), rot, p.pos));
            true
        });
    }

    pub fn clear(&mut self, scene: &mut Scene) {
        self.slots.clear(scene);
        self.live.clear();
    }

    /// Upload changed particles (once per frame).
    pub fn flush(&mut self, gpu: &crate::Gpu, renderer: &crate::Renderer, scene: &Scene) -> Result<(), String> {
        self.slots.flush(gpu, renderer, scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Material;

    fn scene() -> (Scene, usize) {
        let mut scene = Scene::default();
        let m = scene.add_material(Material { tint: [1.0; 3], layer: -1, roughness: 0.5, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&crate::primitives::cube(m).mesh());
        (scene, cube)
    }

    #[test]
    fn bursts_fly_land_and_expire_deterministically() {
        let run = || {
            let (mut scene, cube) = scene();
            let mut fx = Particles::reserve(&mut scene, 64, 3);
            fx.burst(&mut scene, Vec3::Y, Vec3::Y, &Burst { mesh: cube, count: 20, ..Burst::drops() });
            assert_eq!(fx.len(), 20);
            let floor = |a: Vec3, b: Vec3| (b.y < 0.0 && a.y >= 0.0).then(|| (a.lerp(b, a.y / (a.y - b.y)), Vec3::Y));
            for _ in 0..60 { fx.update(&mut scene, 1.0 / 60.0, floor); }
            let resting = fx.live.iter().filter(|p| p.resting).count();
            assert!(fx.live.iter().all(|p| p.pos.y >= 0.0), "nothing falls through the floor");
            let sum: f32 = fx.live.iter().map(|p| p.pos.x + p.pos.z).sum();
            for _ in 0..240 { fx.update(&mut scene, 1.0 / 60.0, floor); }
            (resting, sum.to_bits(), fx.len(), scene.instances[fx.slots.first..].iter().all(|i| i.cluster_count == 0))
        };
        let a = run();
        assert!(a.0 > 0, "some drops landed and stuck");
        assert_eq!(a.2, 0, "all expired");
        assert!(a.3, "slots emptied");
        assert_eq!(a, run(), "deterministic");
    }

    #[test]
    fn full_pool_recycles_the_oldest() {
        let (mut scene, cube) = scene();
        let mut fx = Particles::reserve(&mut scene, 8, 1);
        fx.burst(&mut scene, Vec3::ZERO, Vec3::Y, &Burst { mesh: cube, count: 12, ..Burst::sparks() });
        assert_eq!((fx.len(), fx.recycled), (8, 4));
    }
}
