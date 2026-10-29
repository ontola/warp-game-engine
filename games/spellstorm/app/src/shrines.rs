//! What stands on the way: the altars with their tomes and the ring that
//! fills while you stand in it (Run._altar, _fill_ring), the practice
//! strip's little shrine with the first two elements over it
//! (Practice._raise_shrine), and the portal on to the next biome
//! (Run._portal), and a fallen wizard's grave with its pillar of light
//! (Wizard.die, _grave_lights).
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
/// Wizard.GRAVE_CALM and GRAVE_WARM.
const CALM: [f32; 3] = [0.62, 0.8, 1.0];
const WARM: [f32; 3] = [1.0, 0.82, 0.45];
const GRAVE_BEAM: f32 = 16.0;

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
    /// Art.tombstone: slab, round top, the cross's two bars, the dirt.
    tomb: [usize; 5],
    /// SentryFx: the stone foot, the pillar and the head's barrel.
    sentry: [usize; 3],
}

#[derive(Default)]
pub struct Shrines {
    meshes: Option<Meshes>,
    /// When the practice shrine rose, for its spring up.
    shrine_t: Option<f32>,
    pub sprites: Vec<Sprite>,
    pub lights: Vec<LocalLight>,
    /// Each wizard's gem on the posed staff (by wizard index), from the view.
    pub gems: Vec<Option<Vec3>>,
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
        let grey = scene.add_material(Material { tint: linear3([0.38, 0.37, 0.4]).to_array(), layer: -1, roughness: 0.5, flags: 0, scale: 1.0, metal: 0.0 });
        let dark = solid(scene, [0.2, 0.2, 0.22]);
        let dirt = solid(scene, [0.22, 0.18, 0.12]);
        let tomb = [
            scene.add_mesh(&primitives::cube(grey).mesh()),
            scene.add_mesh(&primitives::cylinder(grey, 0.35, 0.35, 20).mesh()),
            scene.add_mesh(&primitives::cube(dark).mesh()),
            scene.add_mesh(&primitives::cube(dark).mesh()),
            scene.add_mesh(&primitives::cube(dirt).mesh()),
        ];
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
        let sentry_stone = scene.add_material(Material { tint: linear3([0.22, 0.2, 0.24]).to_array(), layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let sentry = [
            scene.add_mesh(&primitives::cylinder(sentry_stone, 0.45, 0.6, 8).mesh()),
            scene.add_mesh(&primitives::cylinder(sentry_stone, 0.14, 0.3, 6).mesh()),
            scene.add_mesh(&primitives::cylinder(sentry_stone, 0.05, 0.09, 6).mesh()),
        ];
        self.meshes = Some(Meshes { column, slab, cover, page, lit_page, stump, orbs, torus, tomb, sentry });
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
        // Sentries (SentryFx): a stone pillar with the queue's elements in a
        // ring round its foot and the lead element's orb on top, bobbing,
        // its barrel turned at what it shoots.
        for &id in &w.fx {
            let Ok(sn) = w.ecs.get::<&spellstorm::spells::Sentry>(id) else { continue };
            let orb = |e: El| m.orbs.iter().find(|o| o.0 == e).map(|o| o.1);
            let node = Mat4::from_translation(sn.pos);
            out.push((m.sentry[0], part(node, Vec3::new(0.0, 0.12, 0.0), Vec3::new(1.0, 0.25, 1.0), Vec3::ZERO), true));
            out.push((m.sentry[1], part(node, Vec3::new(0.0, 0.85, 0.0), Vec3::new(1.0, 1.3, 1.0), Vec3::ZERO), true));
            let n = sn.spell.slots.len().max(1);
            for (i, &e) in sn.spell.slots.iter().enumerate() {
                let a = i as f32 * std::f32::consts::TAU / n as f32;
                if let Some(o) = orb(e) {
                    out.push((o, part(node, Vec3::new(a.cos() * 0.36, 0.3, a.sin() * 0.36), Vec3::splat(0.1), Vec3::ZERO), false));
                }
            }
            let head = node * Mat4::from_translation(Vec3::Y * 1.65) * Mat4::from_rotation_y(sn.head_yaw);
            if let Some(o) = sn.spell.primary.and_then(orb) {
                out.push((o, part(head, Vec3::new(0.0, (sn.t as f32 * 3.0).sin() * 0.06, 0.0), Vec3::splat(0.3), Vec3::ZERO), false));
            }
            out.push((m.sentry[2], part(head, Vec3::new(0.0, 0.0, 0.35), Vec3::new(1.0, 0.45, 1.0), Vec3::new(90.0, 0.0, 0.0)), true));
        }
        // The queued elements ride on the staff: small balls circling its
        // gem, spinning faster and wider while RB winds up a burst.
        for (wi, &id) in w.wizards.iter().enumerate() {
            let Some(a) = w.actor(id) else { continue };
            if !a.alive {
                continue;
            }
            let wz = w.wiz(id);
            let slots = &wz.queue.slots;
            if slots.is_empty() {
                continue;
            }
            let nova_k = if wz.rb_t > 0.12 { (wz.rb_t / spellstorm::wizard::NOVA_HOLD).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let gem = self.gems.get(wi).copied().flatten().unwrap_or_else(|| spellstorm::wizard::staff_tip(a.pos, &wz));
            for (i, &e) in slots.iter().enumerate() {
                let ang = time * (3.0 + 9.0 * nova_k) + i as f32 * std::f32::consts::TAU / slots.len() as f32;
                let r = 0.24 + 0.14 * nova_k;
                let at = gem + Vec3::new(ang.cos() * r, (ang * 2.0).sin() * 0.03 - 0.02, ang.sin() * r);
                let big = if e.is_fused() { 1.3 } else { 1.0 };
                if let Some(&(_, mesh)) = m.orbs.iter().find(|o| o.0 == e) {
                    out.push((mesh, crate::look::affine(Mat4::from_scale_rotation_translation(Vec3::splat(0.065 * big * (1.0 + 0.4 * nova_k)), Quat::IDENTITY, at)), false));
                }
            }
        }
        // Graves: the stone rises beside the body, a pillar of light calls
        // out, and the ring fills while a friend stands there.
        for &id in &w.wizards {
            let Some(a) = w.actor(id) else { continue };
            if a.alive {
                continue;
            }
            let wz = w.wiz(id);
            let Some(at) = wz.tomb else { continue };
            let t = wz.tomb_t as f32;
            let up = ((t - 1.2) / 0.6).clamp(0.0, 1.0);
            let y = -1.2 * (1.0 - up) * (1.0 - up);
            let node = Mat4::from_translation(Vec3::new(at.x, y, at.z));
            let [slab, top, bar, cross, dirt] = m.tomb;
            out.push((slab, part(node, Vec3::new(0.0, 0.45, 0.0), Vec3::new(0.7, 0.9, 0.18), Vec3::ZERO), true));
            out.push((top, part(node, Vec3::new(0.0, 0.9, 0.0), Vec3::new(1.0, 0.18, 1.0), Vec3::new(90.0, 0.0, 0.0)), true));
            out.push((bar, part(node, Vec3::new(0.0, 0.62, 0.1), Vec3::new(0.08, 0.4, 0.04), Vec3::ZERO), true));
            out.push((cross, part(node, Vec3::new(0.0, 0.72, 0.1), Vec3::new(0.28, 0.08, 0.04), Vec3::ZERO), true));
            out.push((dirt, part(node, Vec3::new(0.0, 0.06, 0.1), Vec3::new(1.0, 0.12, 0.6), Vec3::ZERO), true));
            let need = spellstorm::knobs::revive_need().max(1.0);
            let k = (wz.revive / need).clamp(0.0, 1.0) as f32;
            let friend = w.wizards.iter().copied().find(|&o| o != id && w.alive(o) && w.grave_dist(id, &wz, w.pos(o)) < spellstorm::wizard::REVIVE_REACH);
            let near = friend.is_some();
            let warm = if near { 1.0 } else { k };
            let c = Vec3::from(CALM).lerp(Vec3::from(WARM), warm).to_array();
            // Fades in once the stone is up.
            let grave_in = ((t - 1.8) * 1.2).clamp(0.0, 1.0);
            let base = Vec3::new(at.x, 0.0, at.z);
            self.beam(base, c, grave_in * (1.0 + if near { 0.6 } else { 0.0 } + 1.6 * k), time);
            let pool = grave_in * (0.6 + if near { 0.3 } else { 0.0 } + 0.6 * k);
            let mut s = Sprite::billboard(base + Vec3::Y * 0.3, [4.5, 4.5], IMAGE, uv(Cell::HaloOut), linear3(c) * pool * 1.4);
            s.additive = true;
            s.alpha_cut = 0.0;
            self.sprites.push(s);
            self.lights.push(omni(base + Vec3::Y * 1.4, c, grave_in * (1.5 + if near { 1.5 } else { 0.0 } + 3.0 * k), 7.0));
            if near {
                self.grave_motes(base, time, 1.0 + 1.5 * k);
            }
            let fill_c = friend.map(|o| {
                let r = w.wiz(o).robe;
                [r[0] + (1.0 - r[0]) * 0.35, r[1] + (1.0 - r[1]) * 0.35, r[2] + (1.0 - r[2]) * 0.35]
            });
            rings.push((base, spellstorm::wizard::REVIVE_REACH, k, GOLD, fill_c.unwrap_or(GOLD), up));
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

    /// grave_beam.gdshader: a pillar of light up out of the grave, soft at
    /// its sides, fading out high up, breathing.
    fn beam(&mut self, base: Vec3, c: [f32; 3], strength: f32, time: f32) {
        if strength <= 0.0 {
            return;
        }
        let breathe = 0.85 + 0.15 * (time * 2.2).sin();
        let lin = linear3(Vec3::from(c).lerp(Vec3::ONE, 0.2).to_array());
        // Pieces half their height apart: their tents add up to one even
        // column, fading as it rises.
        let n = 16;
        let step = GRAVE_BEAM / n as f32;
        for i in 0..=n {
            let h = i as f32 / n as f32;
            let fade = (1.0 - smooth(0.0, 0.85, h)) * smooth(0.0, 0.03, h + 0.02);
            let width = 2.0 * (0.7 + 0.3 * h);
            let at = base + Vec3::Y * (step * i as f32);
            let mut s = Sprite::shaft(at, [width, step * 2.0], Vec3::Y, IMAGE, uv(Cell::Pillar), lin * fade * breathe * strength * 0.8 * 0.6);
            s.depth_bias = 0.3;
            self.sprites.push(s);
        }
    }

    /// Motes streaming up into the pillar while a friend raises the fallen.
    fn grave_motes(&mut self, base: Vec3, time: f32, speed: f32) {
        let col = linear3(WARM) * 1.4;
        for j in 0..24 {
            let h = j as f32 * 1.91 + 0.7;
            let ph = (time * speed / 1.6 + hash(h)).fract();
            let t = ph * 1.6;
            let a = hash(h + 0.4) * std::f32::consts::TAU;
            let r = (0.9 + 0.3 * hash(h + 0.8) - 1.3 * t * t * 0.5).max(0.1);
            let p = base + Vec3::new(a.cos() * r, 0.1 + (1.5 + 1.5 * hash(h + 1.1)) * t, a.sin() * r);
            let fade = smooth(0.0, 0.15, ph) * (1.0 - smooth(0.5, 1.0, ph));
            let mut s = Sprite::billboard(p, [0.14, 0.14], IMAGE, uv(Cell::Spark), col * fade);
            s.additive = true;
            s.alpha_cut = 0.0;
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
