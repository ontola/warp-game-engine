//! The growing gun on screen (crate::gun builds it): one rig per player,
//! drawn in first person in that player's own view and in third person in
//! everyone else's, with Godot's first-person gun feel.
use super::effects::EffectsView;
use super::{camera, glow, material, srgb_to_linear};
use crate::gun::{self, Rig, Stats};
use glam::{Affine3A, Quat, Vec3};
use growing_guns::{Player, World};
use render::primitives;
use render::scene::Material;
use render::slots::Slots;
use render::Scene;

/// Instances per gun block (the largest build is about 160 parts).
const GUN_PARTS: usize = 192;

/// First-person gun feel (Godot `_update_gun_feel`): walk bob, jump bump,
/// strafe tilt, recoil kick, the card-growth inspect blend.
#[derive(Clone, Copy, Default)]
struct GunFeel { phase: f32, bob: f32, bump: f32, tilt: f32, kick: f32, inspect: f32 }

pub struct Guns {
    /// Mesh per shape, then per material (barrel: one per player, each
    /// with its own material that follows the heat every frame).
    meshes: Vec<Vec<usize>>,
    /// First of the players' barrel materials (consecutive) and their values.
    barrel_first: u32,
    barrels: Vec<Material>,
    /// Each player's barrel exit this frame: first person, third person.
    muzzles: Vec<Option<[Vec3; 2]>>,
    /// Players seen in first person by a local view.
    local: Vec<bool>,
    rigs: Vec<Rig>,
    feel: Vec<GunFeel>,
    /// First instance of each player's first-person and third-person block.
    fp: Vec<usize>,
    tp: Vec<usize>,
    /// Settings: movement tilt (the strafe lean) on.
    pub tilt: bool,
}

impl Guns {
    pub fn reserve(scene: &mut Scene, players: usize) -> Self {
        let mut meshes = Vec::new();
        let barrels: Vec<Material> = (0..players).map(|_| barrel_material(0.0)).collect();
        let barrel_first = scene.materials.len() as u32;
        for &m in &barrels { scene.add_material(m); }
        for shape in gun::SHAPES {
            let mut row = Vec::new();
            for m in gun::MATS {
                let levels = if m == gun::Mat::Barrel { players } else { 1 };
                for h in 0..levels {
                    let mat = if m == gun::Mat::Barrel { barrel_first + h as u32 } else { gun_material(scene, m) };
                    let prim = match shape {
                        gun::Shape::Box => primitives::hewn_block(mat, 1, 0.1, 0.0, 0),
                        gun::Shape::Cyl => primitives::cylinder(mat, 1.0, 1.0, 8),
                        gun::Shape::Taper85 => primitives::cylinder(mat, 1.0, 0.85, 8),
                        gun::Shape::Taper105 => primitives::cylinder(mat, 1.0, 1.05, 8),
                        gun::Shape::Ring18 => primitives::torus(mat, 0.099, 10, 6),
                        gun::Shape::Ring25 => primitives::torus(mat, 0.143, 10, 6),
                    };
                    row.push(scene.add_mesh(&prim.mesh()));
                }
            }
            meshes.push(row);
        }
        let (mut fp, mut tp) = (Vec::new(), Vec::new());
        for _ in 0..players {
            let a = Slots::reserve(scene, GUN_PARTS).first;
            // Nobody sees a first-person gun until its player is local.
            for k in 0..GUN_PARTS { let i = &mut scene.instances[a + k]; i.hide_in_views(0xff); i.flags |= render::scene::NO_SHADOW; }
            fp.push(a);
            tp.push(Slots::reserve(scene, GUN_PARTS).first);
        }
        Guns { meshes, barrel_first, barrels, muzzles: vec![None; players], local: vec![false; players], rigs: vec![Rig::default(); players], feel: vec![GunFeel::default(); players], fp, tp, tilt: true }
    }

    /// A card pick: the gun grows into the new stats.
    pub fn grow(&mut self, player: usize, p: &Player) { if let Some(r) = self.rigs.get_mut(player) { r.grow_to(Stats::of(p)); } }

    pub fn jump(&mut self, player: usize) { if let Some(f) = self.feel.get_mut(player) { f.bump = 0.06; } }

    /// A shot: bolt, spin and heat on the rig, recoil kick on the view model.
    pub fn shot(&mut self, player: usize, p: &Player) {
        let Some(rig) = self.rigs.get_mut(player) else { return };
        rig.fire(&Stats::of(p));
        let f = &mut self.feel[player];
        f.kick = f.kick.max(0.08 * (p.weapon.recoil() / growing_guns::weapon::BASE_RECOIL).clamp(0.2, 2.5));
    }

    /// One tick of gun state: rigs follow their stats (morphs, heat, bolt,
    /// spin) and the first-person feel eases (bob, bump, tilt, kick).
    pub fn tick(&mut self, world: &World, dt: f32) {
        for (i, p) in world.players.iter().enumerate().take(self.rigs.len()) {
            let rig = &mut self.rigs[i];
            rig.follow(Stats::of(p));
            rig.update(dt);
            let f = &mut self.feel[i];
            let horiz = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
            f.bob = if p.on_floor && horiz > 0.5 { (horiz / growing_guns::WALK_SPEED).clamp(0.0, 1.4) } else { 0.0 };
            if f.bob > 0.0 { f.phase += horiz * dt / 2.2 * std::f32::consts::PI; }
            let k = (dt * 8.0).min(1.0);
            f.bump -= f.bump * k;
            f.kick -= f.kick * (dt * 14.0).min(1.0);
            let (sy, cy) = p.yaw.sin_cos();
            let lateral = if self.tilt { (p.vel.dot(Vec3::new(cy, 0.0, -sy)) / growing_guns::WALK_SPEED).clamp(-1.0, 1.0) } else { 0.0 };
            f.tilt += ((lateral * 3.5f32).to_radians() - f.tilt) * k;
            f.inspect += (if rig.is_growing() { 1.0 } else { 0.0 } - f.inspect) * (dt * 5.0).min(1.0);
        }
    }

    /// Every rig into its two blocks: first person from the (punched)
    /// camera, third person in the right hand, aimed with the head.
    pub fn pose(&mut self, scene: &mut Scene, world: &World, prev: &[crate::view::Pose], alpha: f32, fx: &EffectsView, hands: &[Option<Vec3>]) {
        for i in 0..self.rigs.len() {
            // Slots past the players (a coop wave's enemies gone): empty.
            let Some(p) = world.players.get(i) else {
                for first in [self.fp[i], self.tp[i]] { empty(scene, first, 0); }
                continue;
            };
            let rig = &self.rigs[i];
            self.barrels[i] = barrel_material(rig.heat_glow_t());
            let f = &self.feel[i];
            let (r_off, r_tilt, mag_drop) = if p.reloading() {
                gun::reload_pose(p.reload_progress())
            } else { (Vec3::ZERO, 0.0, 0.0) };
            // Long barrels pull the gun back against the shoulder.
            let pull = ((rig.build.barrel_length - 0.5) * 0.3).clamp(0.0, 0.3);
            let cam = fx.punched(world, i, camera(world, i, prev, alpha));
            let fwd = (cam.target - cam.pos).normalize_or(Vec3::NEG_Z);
            let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
            let up = right.cross(fwd);
            let view = Affine3A::from_cols(right.into(), up.into(), (-fwd).into(), cam.pos.into());
            let muzzle = Vec3::new(0.3 + (f.phase * 0.5).sin() * 0.007 * f.bob, -0.3 + f.phase.sin() * 0.012 * f.bob - f.bump, -0.8 + f.kick + pull) + r_off;
            let s = f.inspect;
            let fp = view * Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::YXZ, 0.0, r_tilt, f.tilt), muzzle)
                * Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::YXZ, s, 0.12 * s, 0.08 * s), Vec3::new(-0.14, 0.04, 0.14) * s);
            let drawn = crate::view::pose(world, i, prev, alpha);
            let (pos, rot) = (drawn.pos, Quat::from_rotation_y(drawn.yaw));
            let hand = hands.get(i).copied().flatten().map_or(
                pos + Vec3::Y * 0.05 + rot * Vec3::new(0.45, 0.0, -0.12),
                |hand| hand + rot * Vec3::new(0.0, 0.085, -0.143));
            let tp = Affine3A::from_rotation_translation(rot * Quat::from_rotation_x(drawn.pitch), hand);
            self.muzzles[i] = p.alive.then(|| [fp.transform_point3(rig.build.muzzle), tp.transform_point3(rig.build.muzzle)]);
            for (first, xf, drop) in [(self.fp[i], fp, mag_drop), (self.tp[i], tp, 0.0)] {
                let mut n = 0;
                if p.alive {
                    for (part, local) in rig.pose(drop).take(GUN_PARTS) {
                        scene.update_instance(first + n, self.mesh(part.shape, part.mat, i), xf * local);
                        n += 1;
                    }
                }
                empty(scene, first, n);
            }
        }
    }

    pub fn capacity(&self) -> usize { self.rigs.len() }

    /// Each local player sees only their own first-person gun, and not
    /// their own third-person one.
    pub fn hide_own(&mut self, scene: &mut Scene, slots: &[usize]) {
        self.local.fill(false);
        for (&tp, &fp) in self.tp.iter().zip(&self.fp) {
            for n in 0..GUN_PARTS {
                scene.instances[tp + n].hide_in_views(0);
                scene.instances[fp + n].hide_in_views(0xff);
            }
        }
        for &s in slots { if let Some(l) = self.local.get_mut(s) { *l = true; } }
        for (k, &s) in slots.iter().enumerate().take(8) {
            if s >= self.tp.len() { continue; }
            for n in 0..GUN_PARTS {
                scene.instances[self.tp[s] + n].hide_in_views(1 << k);
                scene.instances[self.fp[s] + n].hide_in_views(!(1u8 << k));
            }
        }
    }

    /// Where player `i`'s gun barrel ends as drawn last frame: in their own
    /// first-person view when local, else in their hand.
    pub fn muzzle(&self, i: usize) -> Option<Vec3> {
        self.muzzles.get(i).copied().flatten().map(|[fp, tp]| if self.local[i] { fp } else { tp })
    }

    /// Upload the barrels' heat glow.
    pub fn flush(&self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        renderer.update_materials(gpu, self.barrel_first as usize, &self.barrels)
    }

    fn mesh(&self, shape: gun::Shape, mat: gun::Mat, player: usize) -> usize {
        let s = gun::SHAPES.iter().position(|&x| x == shape).expect("listed shape");
        let mut k = 0;
        for m in gun::MATS {
            if m == mat { return self.meshes[s][k + if m == gun::Mat::Barrel { player } else { 0 }]; }
            k += if m == gun::Mat::Barrel { self.rigs.len() } else { 1 };
        }
        unreachable!("listed material")
    }
}

/// Empty a gun block from part `n` on, up to the first already empty slot.
fn empty(scene: &mut Scene, first: usize, n: usize) {
    for inst in scene.instances[first + n..first + GUN_PARTS].iter_mut().take_while(|i| i.cluster_count != 0) {
        inst.cluster_count = 0;
        inst.sphere = [0.0; 4];
    }
}

/// Godot `_make_material` and `_make_polymer`: machined metal lifted toward
/// gunmetal, matte polymer; the barrel glows with heat.
fn gun_material(scene: &mut Scene, m: gun::Mat) -> u32 {
    use gun::Mat::*;
    let rc = [0.13f32, 0.14, 0.17];
    let metal = |scene: &mut Scene, col: [f32; 3], metallic: f32, rough: f32| {
        let lift = 0.06 + (0.3 - 0.06) * metallic;
        let gm = [0.62, 0.64, 0.68];
        let tint = [0, 1, 2].map(|i| col[i] + (gm[i] - col[i]) * lift);
        scene.add_material(Material { tint: srgb_to_linear(tint), layer: -1, roughness: (rough + 0.1).min(1.0), flags: 0, scale: 1.0, metal: metallic })
    };
    let k = |f: f32| rc.map(|c| c * f);
    match m {
        Metal => metal(scene, rc, 0.85, 0.32),
        Dark => metal(scene, k(0.6), 0.95, 0.22),
        Barrel => unreachable!("per-player barrel_material"),
        Polymer => material(scene, [0.3, 0.25, 0.18], 0.7),
        Pad => material(scene, [0.07, 0.07, 0.075], 0.7),
        Recess => metal(scene, k(0.2), 0.3, 0.8),
        Slot => metal(scene, k(0.25), 0.4, 0.85),
        Bolt => metal(scene, [0.55, 0.57, 0.62], 0.95, 0.18),
        Lens => glow(scene, [0.35, 0.7, 1.0], 7000.0),
        Indicator => glow(scene, [0.4, 1.0, 0.95], 10000.0),
    }
}

/// The barrel at heat glow `t`: cold machined steel, then Godot's glow
/// (emission only) once it starts to show, continuous from there.
fn barrel_material(t: f32) -> Material {
    if t <= 0.02 {
        let col = [0.13f32, 0.14, 0.17].map(|c| c * 0.6);
        let (gm, lift) = ([0.62, 0.64, 0.68], 0.06 + (0.3 - 0.06) * 0.95);
        let tint = [0, 1, 2].map(|i| col[i] + (gm[i] - col[i]) * lift);
        return Material { tint: srgb_to_linear(tint), layer: -1, roughness: 0.32, flags: 0, scale: 1.0, metal: 0.95 };
    }
    let (c, e) = gun::heat_glow(t);
    Material { tint: c, layer: -1, roughness: 0.8, flags: render::scene::EMISSIVE, scale: e * 4000.0, metal: 0.0 }
}
