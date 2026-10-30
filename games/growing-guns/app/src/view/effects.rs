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

/// Occupancy bounds: the largest arena (95 m) plus margin, lava to roofs.
const OCC_HALF: f32 = 52.0;
pub const OCC_VOXEL: f32 = 0.5;
/// Bullet colour with explosive rounds (Godot: base yellow toward orange).
pub const BLAST_TINT: Vec3 = Vec3::new(1.0, 0.70, 0.20);

/// Godot light energy to candela (as the air strike's lights).
const CD_PER_ENERGY: f32 = 37_000.0;
/// Seconds a muzzle flash shows (Godot tweens 0.045 to 0.08 s).
const FLASH_LIFE: f32 = 0.07;
pub const FLASH_IMAGE: render::sprites::SpriteImage = render::sprites::SpriteImage(0x666c_6173);
/// Muzzle-flash nits per Godot emission energy (as the bullet streaks).
const FLASH_NITS: f32 = 15_000.0;

/// A soft glow, white hot in the middle: the muzzle flash's star arms,
/// plume and core.
pub fn flash_picture() -> render::picture::Picture {
    let n = 64u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (u, v) = ([x, y].map(|c| (c as f32 + 0.5) / n as f32 * 2.0 - 1.0)).into();
            let r2: f32 = u * u + v * v;
            let a = ((-4.0 * r2).exp() - (-4.0f32).exp()).max(0.0) / (1.0 - (-4.0f32).exp());
            px.extend([255, 255, 255, (a * 255.0 + 0.5) as u8]);
        }
    }
    render::picture::Picture::new(n, n, px).expect("fixed size")
}

/// Godot `_spawn_muzzle_flash`: a starburst across the barrel and a flame
/// plume out of it in the bullet's colour, a bright short light ahead and
/// a small one on the gun.
#[derive(Clone, Copy)]
struct Flash { at: Vec3, dir: Vec3, color: Vec3, scale: f32, bright: f32, spin: f32, born: f32 }

#[derive(Clone, Copy)]
pub struct FxMeshes { pub spark: usize, pub dust: usize, pub blood: usize, pub casing: usize }

pub struct EffectsView {
    pub particles: Particles,
    pub meshes: FxMeshes,
    /// GPU rubble colliding with the arena's occupancy grid.
    pub debris: Debris,
    /// Explosions: fire, smoke, embers, heat shell, lights.
    pub blasts: Effects,
    flashes: Vec<Flash>,
    /// The effects clock at the last `update`.
    clock: f32,
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
        let particles = Particles::reserve(scene, crate::graphics::get().particles, seed | 1);
        let occ = Occupancy::new(Vec3::new(-OCC_HALF, -4.0, -OCC_HALF), Vec3::new(OCC_HALF, 30.0, OCC_HALF), OCC_VOXEL);
        let debris = Debris::reserve(scene, crate::graphics::get().debris_capacity, occ, false);
        EffectsView { particles, meshes, debris, blasts: Effects::default(), flashes: Vec::new(), clock: 0.0, punch: Vec::new(), pending_punch: Vec::new() }
    }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene, clock: f32) -> Result<(), String> {
        self.debris.flush(gpu, renderer)?;
        self.blasts.flush(gpu, renderer, clock)?;
        self.particles.flush(gpu, renderer, scene)
    }

    /// A shot's flash at the barrel exit `at` along `dir`; `scale` the
    /// bullet's size, `bright` Godot's flash brightness (0.85 to 8).
    pub fn flash(&mut self, now: f32, at: Vec3, dir: Vec3, color: [f32; 3], scale: f32, bright: f32) {
        let spin = (at.x * 91.7 + at.z * 47.3 + self.flashes.len() as f32).sin() * std::f32::consts::PI;
        self.flashes.push(Flash { at, dir: dir.normalize_or(Vec3::NEG_Z), color: Vec3::from(color), scale: scale.clamp(0.3, 2.2), bright: bright.max(0.1), spin, born: now });
        // A minigun keeps a handful, not hundreds.
        if self.flashes.len() > 16 { self.flashes.remove(0); }
    }

    /// The flashes' star, plume and core (additive, all views).
    pub fn flash_sprites(&self, out: &mut Vec<render::sprites::Sprite>) {
        use render::sprites::Sprite;
        for f in &self.flashes {
            let u = ((self.clock - f.born) / FLASH_LIFE).clamp(0.0, 1.0);
            // Godot: scale out to (1.35, 0.82, 1.8) and forward 13 cm in
            // 45 ms (expo out) while the emission fades over 60 ms.
            let grow = 1.0 - (1.0 - (u * FLASH_LIFE / 0.045).min(1.0)).powi(4);
            let fade = (1.0 - u * FLASH_LIFE / 0.06).max(0.0);
            if fade <= 0.0 { continue; }
            let s = f.scale;
            let at = f.at + f.dir * 0.13 * s * grow;
            let side = f.dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
            let up = side.cross(f.dir);
            let (sn, cs) = f.spin.sin_cos();
            let (a, b) = (side * cs + up * sn, up * cs - side * sn);
            let hot = Vec3::from(f.color).lerp(Vec3::ONE, 0.35).powf(2.2) * (6.0 * f.bright.powf(1.18) * FLASH_NITS * fade);
            let arm = |r: Vec3, u: Vec3| {
                let mut sp = Sprite::fixed(at, [0.07 * s, 0.34 * s * (0.82 + 0.5 * grow)], r, u, FLASH_IMAGE, [0.0, 0.0, 1.0, 1.0], hot);
                sp.additive = true; sp.alpha_cut = 0.0; sp.depth_bias = 0.0; sp.min_pixels = 2.0;
                sp
            };
            out.push(arm(a, b));
            out.push(arm(b, -a));
            let len = 0.34 * s * (1.0 + 0.8 * grow);
            let mut plume = Sprite::shaft(at + f.dir * len * 0.5, [0.12 * s, len * 1.3], f.dir, FLASH_IMAGE, [0.0, 0.0, 1.0, 1.0], hot);
            plume.depth_bias = 0.0; plume.min_pixels = 2.0;
            out.push(plume);
            let mut core = Sprite::billboard(at, [0.14 * s, 0.14 * s], FLASH_IMAGE, [0.0, 0.0, 1.0, 1.0], hot * 1.5);
            core.additive = true; core.alpha_cut = 0.0; core.depth_bias = 0.0; core.min_pixels = 2.0;
            out.push(core);
        }
    }

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
        self.clock = clock;
        self.flashes.retain(|f| clock - f.born < FLASH_LIFE);
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

    /// Up to `room` muzzle-flash lights, newest first: Godot's flash light
    /// 42 cm ahead (fading over 70 ms), then the gun's own warm light
    /// just behind the exit (80 ms) while there is room.
    pub fn flash_lights(&self, room: usize) -> Vec<LocalLight> {
        let mut out = Vec::new();
        for gun in [false, true] {
            for f in self.flashes.iter().rev() {
                if out.len() >= room { return out; }
                let u = ((self.clock - f.born) / FLASH_LIFE).clamp(0.0, 1.0);
                let (energy, range, warm, at, life) = if gun {
                    (1.6 * f.bright.powf(1.08), 2.0 * f.scale * f.bright.powf(0.5), Vec3::new(1.0, 0.92, 0.8).lerp(f.color, 0.4), f.at - f.dir * 0.06, 0.08)
                } else {
                    (2.75 * f.bright.powf(1.12), 5.0 * f.scale * f.bright.powf(0.55), Vec3::new(1.0, 0.95, 0.82).lerp(f.color, 0.55), f.at + f.dir * 0.42, 0.07)
                };
                let k = (1.0 - u * FLASH_LIFE / life).max(0.0);
                if k > 0.0 { out.push(omni(at, warm.powf(2.2) * energy * k * CD_PER_ENERGY, range.clamp(1.0, 20.0))); }
            }
        }
        out
    }
}
