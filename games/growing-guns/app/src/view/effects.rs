//! Short-lived effects: engine particles (sparks, dust, blood drops,
//! casings), GPU debris (chips and rubble colliding with the arena's
//! occupancy grid), explosions (render::fx), muzzle flashes and the
//! explosion view punch.
use super::{cube, glow, material, omni, sphere, Camera};
use super::arena::palette;
use glam::{Quat, Vec2, Vec3};
use growing_guns::World;
use render::debris::{Debris, Occupancy};
use render::fx::Effects;
use render::headlights::LocalLight;
use render::particles::Particles;
use render::scene::Material;
use render::Scene;

/// Godot: MAX_LIVE_PARTICLES 3600 at full quality.
const DEBRIS: usize = 4096;
/// Occupancy bounds: the largest arena (95 m) plus margin, lava to roofs.
const OCC_HALF: f32 = 52.0;
pub const OCC_VOXEL: f32 = 0.5;
/// Bullet colour with explosive rounds (Godot: base yellow toward orange).
pub const BLAST_TINT: Vec3 = Vec3::new(1.0, 0.70, 0.20);

#[derive(Clone, Copy)]
pub struct FxMeshes { pub spark: usize, pub dust: usize, pub blood: usize, pub casing: usize }

pub struct EffectsView {
    pub particles: Particles,
    pub meshes: FxMeshes,
    /// GPU rubble colliding with the arena's occupancy grid.
    pub debris: Debris,
    /// Explosions: fire, smoke, embers, heat shell, lights.
    pub blasts: Effects,
    /// Muzzle flashes: position and seconds left.
    flashes: Vec<(Vec3, f32)>,
    /// Explosion view punch per player: position offset (camera local) and
    /// pitch/yaw kick, plus punches still travelling at the speed of sound.
    punch: Vec<(Vec3, Vec2)>,
    pending_punch: Vec<(f32, Vec3, f32)>,
}

impl EffectsView {
    pub fn reserve(scene: &mut Scene, seed: u64) -> Self {
        let meshes = FxMeshes {
            spark: { let m = glow(scene, [1.0, 0.6, 0.2], 20000.0); cube(scene, m) },
            dust: { let m = material(scene, palette::DARK, 0.95); cube(scene, m) },
            blood: { let m = material(scene, [0.45, 0.02, 0.02], 0.35); sphere(scene, m, 4, 6) },
            casing: { let m = scene.add_material(Material { tint: [0.75, 0.55, 0.2], layer: -1, roughness: 0.35, flags: 0, scale: 1.0, metal: 1.0 }); cube(scene, m) },
        };
        let particles = Particles::reserve(scene, 768, seed | 1);
        let occ = Occupancy::new(Vec3::new(-OCC_HALF, -4.0, -OCC_HALF), Vec3::new(OCC_HALF, 30.0, OCC_HALF), OCC_VOXEL);
        let debris = Debris::reserve(scene, DEBRIS, occ, false);
        EffectsView { particles, meshes, debris, blasts: Effects::default(), flashes: Vec::new(), punch: Vec::new(), pending_punch: Vec::new() }
    }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene, clock: f32) -> Result<(), String> {
        self.debris.flush(gpu, renderer)?;
        self.blasts.flush(gpu, renderer, clock)?;
        self.particles.flush(gpu, renderer, scene)
    }

    pub fn flash(&mut self, at: Vec3) { self.flashes.push((at, 0.06)); }

    /// Schedule the view punch for everyone within five radii of a blast,
    /// landing when the sound gets there.
    pub fn blast_punch(&mut self, world: &World, now: f32, pos: Vec3, radius: f32) {
        for p in &world.players {
            let d = p.pos.distance(pos);
            if p.alive && d < (radius * 5.0).max(radius + 1.0) { self.pending_punch.push((now + d / 343.0, pos, radius)); }
        }
    }

    /// Per frame: flashes fade, due punches land and every punch eases back.
    pub fn update(&mut self, world: &World, clock: f32, dt: f32) {
        self.flashes.retain_mut(|f| { f.1 -= dt; f.1 > 0.0 });
        self.update_punch(world, clock, dt);
    }

    /// Explosion view punch (Godot `do_explosion_view_punch`): pushed away
    /// from the blast, kicked in pitch and yaw, easing back at rate 12.
    fn update_punch(&mut self, world: &World, now: f32, dt: f32) {
        self.punch.resize(world.players.len(), (Vec3::ZERO, Vec2::ZERO));
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
                self.punch[i].1 += Vec2::new(-local.y * (0.045 + 0.165 * strength), local.x * (0.018 + 0.084 * strength));
            }
        }
        let k = (dt * 12.0).min(1.0);
        for p in &mut self.punch { p.0 = p.0.lerp(Vec3::ZERO, k); p.1 = p.1.lerp(Vec2::ZERO, k); }
    }

    /// A player's camera with the explosion punch applied.
    pub fn punched(&self, world: &World, i: usize, mut cam: Camera) -> Camera {
        let Some(&(off, rot)) = self.punch.get(i) else { return cam };
        if !world.players[i].alive { return cam; }
        let yaw = Quat::from_rotation_y(world.players[i].yaw);
        let fwd = cam.target - cam.pos;
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        let turned = Quat::from_axis_angle(Vec3::Y, rot.y) * Quat::from_axis_angle(right, rot.x) * fwd;
        cam.pos += yaw * off;
        cam.target = cam.pos + turned;
        cam
    }

    /// Up to `room` muzzle-flash lights, newest first.
    pub fn flash_lights(&self, room: usize) -> impl Iterator<Item = LocalLight> + '_ {
        self.flashes.iter().rev().take(room).map(|&(p, _)| omni(p, Vec3::new(1.0, 0.75, 0.35) * 600.0, 8.0))
    }
}
