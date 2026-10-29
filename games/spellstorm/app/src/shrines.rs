//! What stands on the way: the altars with their tomes and the ring that
//! fills while you stand in it (Run._altar, _fill_ring), the practice
//! strip's little shrine with the first two elements over it
//! (Practice._raise_shrine), and the portal on to the next biome
//! (Run._portal).
use crate::fx::{uv, Cell, BAND, BANDS, BAND_AT, BAND_SIZE, IMAGE, LIGHT};
use crate::look::linear3;
use glam::{Affine3A, EulerRot, Mat4, Quat, Vec3};
use render::headlights::LocalLight;
use render::primitives;
use render::scene::{Material, EMISSIVE};
use render::sprites::Sprite;
use render::Scene;
use spellstorm::el::{self, El};
use spellstorm::run::{practice::SHRINE_R, ALTAR_R, GATE_BACK, PORTAL_R, PORTAL_TIME, TAKE_TIME};
use spellstorm::World;
use std::f32::consts::FRAC_PI_2;

const GOLD: [f32; 3] = [1.0, 0.8, 0.45];
const PORTAL: [f32; 3] = [0.3, 0.55, 1.0];

/// The meshes, made with the scene.
#[derive(Default)]
struct Meshes {
    column: usize,
    slab: usize,
    cover: usize,
    page: usize,
    lit_page: usize,
    stump: usize,
    orbs: Vec<(El, usize)>,
    torus: usize,
}

#[derive(Default)]
pub struct Shrines {
    meshes: Option<Meshes>,
    /// When the practice shrine rose, for its spring up.
    shrine_t: Option<f32>,
    pub sprites: Vec<Sprite>,
    pub lights: Vec<LocalLight>,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash(n: f32) -> f32 {
    ((n * 12.9898).sin() * 43758.547).fract().abs()
}

/// Tween.TRANS_BACK, EASE_OUT: past 1 and back.
fn back_out(t: f32) -> f32 {
    let (c1, t) = (1.70158, t.clamp(0.0, 1.0) - 1.0);
    1.0 + (c1 + 1.0) * t * t * t + c1 * t * t
}

fn omni(pos: Vec3, c: [f32; 3], energy: f32, range: f32) -> LocalLight {
    LocalLight { position: pos, direction: Vec3::NEG_Y, intensity_cd: linear3(c) * energy * LIGHT, range_m: range, inner_cos: -0.999, outer_cos: -1.0, source_radius_m: 0.05, falloff: 1.0 }
}

/// A part at `at` in its node: turned by degrees `rot` (Godot's order).
fn part(node: Mat4, at: Vec3, size: Vec3, rot: Vec3) -> Affine3A {
    let q = Quat::from_euler(EulerRot::YXZ, rot.y.to_radians(), rot.x.to_radians(), rot.z.to_radians());
    crate::look::affine(node * Mat4::from_scale_rotation_translation(size, q, at))
}

impl Shrines {
    /// Every mesh they may show, into the scene being built.
    pub fn build(&mut self, scene: &mut Scene) {
        let solid = |scene: &mut Scene, c: [f32; 3]| scene.add_material(Material { tint: linear3(c).to_array(), layer: -1, roughness: 0.85, flags: 0, scale: 1.0, metal: 0.0 });
        // Fx.glow: the colour, and as much again times energy glowing.
        let glow = |scene: &mut Scene, c: [f32; 3], energy: f32| scene.add_material(Material { tint: linear3(c).to_array(), layer: -1, roughness: 0.85, flags: EMISSIVE, scale: energy, metal: 0.0 });
        let stone = solid(scene, [0.3, 0.28, 0.27]);
        let cover = solid(scene, [0.4, 0.1, 0.08]);
        let paper = solid(scene, [0.85, 0.8, 0.65]);
        let lit = glow(scene, GOLD, 1.2);
        let ring = glow(scene, PORTAL, 3.0);
        let column = scene.add_mesh(&primitives::cylinder(stone, 0.35, 0.45, 8).mesh());
        let stump = scene.add_mesh(&primitives::cylinder(stone, 0.3, 0.42, 8).mesh());
        let slab = scene.add_mesh(&primitives::cube(stone).mesh());
        let cover = scene.add_mesh(&primitives::cube(cover).mesh());
        let page = scene.add_mesh(&primitives::cube(paper).mesh());
        let lit_page = scene.add_mesh(&primitives::cube(lit).mesh());
        let torus = scene.add_mesh(&primitives::torus(ring, 0.1 / 1.35, 32, 8).mesh());
        // element.gdshader at energy 1.6: the element's colour, glowing.
        let orbs = el::ALL
            .iter()
            .map(|&e| {
                let c = e.color();
                let g = linear3([c[0], c[1], c[2]]) * 1.6;
                let peak = g.max_element().max(1e-6);
                let m = scene.add_material(Material { tint: (g / peak).to_array(), layer: -1, roughness: 1.0, flags: EMISSIVE, scale: peak, metal: 0.0 });
                (e, scene.add_mesh(&primitives::sphere(m, 8, 12).mesh()))
            })
            .collect();
        self.meshes = Some(Meshes { column, slab, cover, page, lit_page, stump, orbs, torus });
        self.shrine_t = None;
    }

    /// This frame's parts into `out` (mesh, where, casts a shadow), and
    /// their rings, sparks and lights.
    pub fn update(&mut self, w: &World, time: f32, out: &mut Vec<(usize, Affine3A, bool)>) {
        self.sprites.clear();
        self.lights.clear();
        let Some(m) = self.meshes.take() else { return };
        let mut rings = vec![];
        for a in &w.run.altars {
            if a.rise <= 0.0 {
                continue;
            }
            // It springs up, and squashes flat once its tome is taken.
            let s = if a.taken { a.rise.max(0.01) } else { back_out(a.rise) };
            let node = Mat4::from_translation(Vec3::new(a.pos.x, 0.0, a.pos.z)) * Mat4::from_scale(Vec3::new(1.0, s, 1.0));
            out.push((m.column, part(node, Vec3::new(0.0, 0.45, 0.0), Vec3::new(1.0, 0.9, 1.0), Vec3::ZERO), true));
            out.push((m.slab, part(node, Vec3::new(0.0, 0.95, 0.0), Vec3::new(0.8, 0.1, 0.6), Vec3::ZERO), true));
            // Lying open on a lectern, tipped toward the camera.
            let book = node * Mat4::from_translation(Vec3::new(0.0, 1.35, 0.0)) * Mat4::from_rotation_x(35f32.to_radians());
            out.push((m.cover, part(book, Vec3::ZERO, Vec3::new(0.62, 0.04, 0.42), Vec3::ZERO), true));
            for sgn in [-1.0f32, 1.0] {
                out.push((m.page, part(book, Vec3::new(sgn * 0.145, 0.05, 0.0), Vec3::new(0.28, 0.05, 0.38), Vec3::new(0.0, 0.0, sgn * 7.0)), true));
            }
            out.push((m.lit_page, part(book, Vec3::new(-0.145, 0.078, 0.0), Vec3::new(0.24, 0.002, 0.3), Vec3::new(0.0, 0.0, -7.0)), false));
            let k = s.min(1.0);
            self.lights.push(omni(a.pos.with_y(1.8 * s), GOLD, 1.6 * k, 3.5));
            if !a.taken {
                let c = a.fill_c;
                rings.push((a.pos, ALTAR_R, (a.hold / TAKE_TIME) as f32, GOLD, [c[0], c[1], c[2]], k));
                self.sparks(a.pos.with_y(1.3 * s), 16, 0.4, 0.6, 0.5, 0.15, GOLD, time, a.relic as f32 * 7.3, k);
            }
        }
        // The practice shrine: a low stone, the first elements over it.
        match w.run.practice.as_ref().and_then(|p| p.shrine.map(|at| (p, at))) {
            Some((p, at)) => {
                let since = time - *self.shrine_t.get_or_insert(time);
                let s = back_out(since / 0.6).max(0.01);
                let node = Mat4::from_translation(Vec3::new(at.x, 0.0, at.z)) * Mat4::from_scale(Vec3::new(1.0, s, 1.0));
                out.push((m.stump, part(node, Vec3::new(0.0, 0.35, 0.0), Vec3::new(1.0, 0.7, 1.0), Vec3::ZERO), true));
                let n = p.shrine_els.len().max(1) as f32;
                for (i, e) in p.shrine_els.iter().enumerate() {
                    let a = std::f32::consts::TAU * i as f32 / n;
                    if let Some(&(_, mesh)) = m.orbs.iter().find(|o| o.0 == *e) {
                        out.push((mesh, part(node, Vec3::new(a.cos() * 0.35, 1.15, a.sin() * 0.35), Vec3::splat(0.13), Vec3::ZERO), false));
                    }
                }
                self.lights.push(omni(at.with_y(1.4 * s), GOLD, 1.4 * s.min(1.0), 3.5));
                rings.push((at, SHRINE_R, p.fill as f32, GOLD, GOLD, s.min(1.0)));
            }
            None => self.shrine_t = None,
        }
        // The portal: a glowing ring standing in the gate, the way through
        // it shimmering, and the ring to stand in before it.
        if let Some(at) = w.run.portal {
            let mid = Vec3::new(at.x, 1.6, at.z - GATE_BACK);
            let xf = Mat4::from_scale_rotation_translation(Vec3::splat(1.35), Quat::from_rotation_x(FRAC_PI_2), mid);
            out.push((m.torus, crate::look::affine(xf), false));
            let (sn, cs) = (time * 0.4).sin_cos();
            let (right, up) = (Vec3::new(cs, sn, 0.0), Vec3::new(-sn, cs, 0.0));
            let mut disc = Sprite::fixed(mid, [2.5, 2.5], right, up, IMAGE, uv(Cell::Patch), linear3([0.2, 0.4, 1.0]) * 0.6 * 1.2);
            disc.additive = true;
            disc.alpha_cut = 0.0;
            self.sprites.push(disc);
            self.portal_motes(mid, time);
            self.lights.push(omni(mid + Vec3::Z * 0.6, PORTAL, 2.5, 7.0));
            rings.push((at, PORTAL_R, (w.run.portal_t / PORTAL_TIME) as f32, [0.55, 0.75, 1.0], [0.55, 0.75, 1.0], 1.0));
        }
        // A faint guide in the ring's own colour, and the fill in its taker's.
        for (at, r, fill, guide, c, k) in rings {
            self.ring(at, r, 1.0, guide, 0.45 * k);
            self.ring(at, r, fill.clamp(0.0, 1.0), c, 1.6 * k);
        }
        self.meshes = Some(m);
    }

    /// A ring to stand in on the floor (Run._fill_ring): `progress` of it
    /// swept round, brightest at its leading edge, as slash.gdshader.
    fn ring(&mut self, at: Vec3, r: f32, progress: f32, c: [f32; 3], strength: f32) {
        if progress <= 0.0 || strength <= 0.0 {
            return;
        }
        let c = Vec3::from(c);
        let centre = Vec3::new(at.x, 0.05, at.z);
        for i in 0..BANDS {
            let u = i as f32 / BANDS as f32;
            let shown = (progress * BANDS as f32 - i as f32).clamp(0.0, 1.0);
            if shown <= 0.0 {
                break;
            }
            let lead = if progress < 0.999 { (-(u - progress).abs() * 10.0).exp() } else { 0.0 };
            // From the side away from the camera, round the way a clock goes.
            let ang = std::f32::consts::PI - i as f32 * BAND;
            let up = Vec3::new(ang.sin(), 0.0, ang.cos());
            let right = Vec3::new(up.z, 0.0, -up.x);
            let col = linear3(c.lerp(Vec3::ONE, 0.25 + lead * 0.5).to_array()) * (1.0 + lead * 1.2) * shown * strength * 0.8;
            let mut s = Sprite::fixed(centre + up * BAND_AT * r, [BAND_SIZE * r, BAND_SIZE * r], right, up, IMAGE, uv(Cell::Band), col);
            s.additive = true;
            s.alpha_cut = 0.0;
            s.depth_bias = 0.2;
            self.sprites.push(s);
        }
    }

    /// Fx.particles: `n` sparks from a ball of `spread` round `at`, rising.
    #[allow(clippy::too_many_arguments)]
    fn sparks(&mut self, at: Vec3, n: usize, spread: f32, speed: f32, lift: f32, size: f32, c: [f32; 3], time: f32, seed: f32, k: f32) {
        let col = linear3(c) * 1.4 * k;
        for j in 0..n {
            let h = seed + j as f32 * 1.37;
            let life = 1.2;
            let ph = (time / life + hash(h)).fract();
            let t = ph * life;
            let (a, b) = (hash(h + 0.3) * std::f32::consts::TAU, hash(h + 0.6) * 2.0 - 1.0);
            let dir = Vec3::new((1.0 - b * b).sqrt() * a.cos(), b, (1.0 - b * b).sqrt() * a.sin());
            let p = at + dir * spread * hash(h + 0.9).sqrt() + Vec3::Y * (speed * hash(h + 1.2) * t + 0.5 * lift * t * t);
            let fade = smooth(0.0, 0.15, ph) * (1.0 - smooth(0.5, 1.0, ph));
            let mut s = Sprite::billboard(p, [size, size], IMAGE, uv(Cell::Spark), col * fade);
            s.additive = true;
            s.alpha_cut = 0.0;
            self.sprites.push(s);
        }
    }

    /// The portal's motes: off its ring, drifting up.
    fn portal_motes(&mut self, mid: Vec3, time: f32) {
        let col = linear3(PORTAL) * 1.4;
        for j in 0..60 {
            let h = j as f32 * 2.11 + 0.5;
            let ph = (time / 1.2 + hash(h)).fract();
            let t = ph * 1.2;
            let a = hash(h + 0.4) * std::f32::consts::TAU;
            let r = 1.1 + 0.25 * hash(h + 0.8);
            let p = mid + Vec3::new(a.cos() * r, a.sin() * r, (hash(h + 1.1) - 0.5) * 0.1) + Vec3::Y * 0.5 * hash(h + 1.5) * t;
            let fade = smooth(0.0, 0.15, ph) * (1.0 - smooth(0.5, 1.0, ph));
            let mut s = Sprite::billboard(p, [0.3, 0.3], IMAGE, uv(Cell::Spark), col * fade);
            s.additive = true;
            s.alpha_cut = 0.0;
            self.sprites.push(s);
        }
    }
}
