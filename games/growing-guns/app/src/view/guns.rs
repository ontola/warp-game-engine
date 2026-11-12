//! The growing gun on screen (crate::gun builds it): one rig per player,
//! drawn in first person in that player's own view and in third person in
//! everyone else's, with Godot's first-person gun feel.
use super::effects::EffectsView;
use super::{glow, material, srgb_to_linear};
use crate::gun::{self, Rig, Stats};
use glam::{Affine3A, Quat, Vec3};
use growing_guns::World;
use render::primitives;
use render::scene::Material;
use render::slots::Slots;
use render::Scene;

/// First-person gun feel (Godot `_update_gun_feel`): walk bob, jump bump,
/// strafe tilt, recoil kick, the card-growth inspect blend.
#[derive(Clone, Copy, Default)]
struct GunFeel { phase: f32, bob: f32, bump: f32, tilt: f32, kick: f32, inspect: f32, previous_inspect: f32 }

// Finish lifting the unmodified rifle and returning the camera to aim before inspection.
const PICKUP_LIFT_SECONDS: f32 = 0.40;
const PICKUP_READY_SECONDS: f32 = 0.46;

pub struct Guns {
    pub(crate) detail: gun::Detail,
    parts: usize,
    arms: Vec<super::pov_arms::Arms>,
    spectators: Vec<Option<render::humans::StandingPose>>,
    /// Mesh per shape, then per material (barrel: one per player, each
    /// with its own material that follows the heat every frame).
    meshes: Vec<Vec<usize>>,
    /// First of the players' barrel materials (consecutive) and their values.
    barrel_first: u32,
    lamp_mesh: usize,
    /// Lens transform for the flashlight actually drawn on each rifle.
    pub(super) lamps: Vec<Option<Affine3A>>,
    barrels: Vec<Material>,
    /// Each player's barrel exit this frame: first person, third person.
    muzzles: Vec<Option<[Vec3; 2]>>,
    /// Players seen in first person by a local view.
    local: Vec<bool>,
    rigs: Vec<Rig>,
    feel: Vec<GunFeel>,
    pickups: Vec<Option<(u64, Vec3)>>,
    /// First instance of each player's first-person and third-person block.
    fp: Vec<usize>,
    tp: Vec<usize>,
    /// Settings: movement tilt (the strafe lean) on.
    pub tilt: bool,
    tracked: Vec<Option<Affine3A>>,
    tracking: Vec<bool>,
    own: Vec<Option<Stats>>,
    round: Option<u32>,
}

impl Guns {
    pub fn reserve(scene: &mut Scene, players: usize) -> Self {
        let detail = if crate::graphics::get().simple_guns { gun::Detail::Low } else { gun::Detail::Full };
        Self::reserve_with_detail(scene, players, detail)
    }

    fn reserve_with_detail(scene: &mut Scene, players: usize, detail: gun::Detail) -> Self {
        let parts = detail.part_capacity() + 1; // blackout lamp
        let barrels: Vec<Material> = (0..players).map(|_| barrel_material(0.0)).collect();
        let barrel_first = scene.materials.len() as u32;
        for &m in &barrels { scene.add_material(m); }
        // Share each material across shapes; low guns never register glass blending.
        let mut palette = Vec::new();
        let mut dark = 0;
        for m in gun::MATS {
            if m == gun::Mat::Barrel { palette.extend((0..players).map(|i| barrel_first + i as u32)); }
            else {
                let mat = if detail == gun::Detail::Low && m == gun::Mat::Glass { dark } else { gun_material(scene, m) };
                if m == gun::Mat::Dark { dark = mat; }
                palette.push(mat);
            }
        }
        let meshes = gun::SHAPES.into_iter().map(|shape| palette.iter().map(|&mat|
            scene.add_mesh(&gun_primitive(shape, mat, detail).mesh())).collect()).collect();
        let (mut fp, mut tp) = (Vec::new(), Vec::new());
        for _ in 0..players {
            let a = Slots::reserve(scene, parts).first;
            // Nobody sees a first-person gun until its player is local.
            for k in 0..parts { let i = &mut scene.instances[a + k]; i.hide_in_views(0xff); i.flags |= render::scene::NO_SHADOW; }
            fp.push(a);
            tp.push(Slots::reserve(scene, parts).first);
        }
        let lamp_mesh = flashlight_mesh(scene);
        Guns { spectators: vec![None; players], pickups: vec![None; players], lamp_mesh, lamps: vec![None; players], detail, parts, arms: Vec::new(), meshes, barrel_first, barrels, muzzles: vec![None; players], local: vec![false; players], rigs: vec![Rig::with_detail(detail); players], feel: vec![GunFeel::default(); players], fp, tp, tilt: true, tracked: vec![None; players], tracking: vec![false; players], own: vec![None; players], round: None }
    }

    pub(super) fn reserve_arms(&mut self, scene: &mut Scene, seed: u64, colors: &[crate::party::PlayerColors]) {
        self.arms = (0..self.rigs.len().min(8)).map(|i|
            super::pov_arms::Arms::reserve(scene, seed, i, colors.get(i).copied().unwrap_or_default())
        ).collect();
    }

    pub fn track(&mut self, player: usize, pose: Option<Affine3A>) {
        if let Some(p) = self.tracked.get_mut(player) { *p = pose; self.tracking[player] = true; }
    }
    pub fn is_tracked(&self, player: usize) -> bool { self.tracked.get(player).is_some_and(Option::is_some) }
    pub fn tracked_muzzle(&self, player: usize, pose: Affine3A) -> Vec3 {
        pose.transform_point3(self.rigs[player].muzzle_at(1.0))
    }

    // Both the weapon mesh and the support wrist use the same placement.
    pub(crate) fn held_transform(hand: Vec3, yaw: f32, pitch: f32, primary: Vec3) -> Affine3A {
        let rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
        Affine3A::from_rotation_translation(rotation, hand - rotation * primary)
    }

    pub(super) fn contacts(&self, player: usize, alpha: f32) -> Option<gun::Contacts> {
        Some(self.rigs.get(player)?.contacts_at(alpha))
    }

    /// A card pick: the gun grows into the new stats.
    pub fn grow(&mut self, player: usize, world: &World) {
        let (Some(rig), Some(p)) = (self.rigs.get_mut(player), world.players.get(player)) else { return };
        if world.awaiting_rifle(player) { return; }
        self.own[player] = Some(Stats::weapon(&world.owned_weapon(player)));
        if self.pickups[player].is_none() { rig.grow_to(Stats::of(p)); }
    }

    pub fn jump(&mut self, player: usize) { if let Some(f) = self.feel.get_mut(player) { f.bump = 0.06; } }

    /// New arena, including restarting a match at the same round number.
    pub fn reset_heat(&mut self) {
        for rig in &mut self.rigs { rig.heat = 0.0; }
        self.barrels.fill(barrel_material(0.0));
    }

    fn view_frame(cam: render::Camera) -> Affine3A {
        let fwd = (cam.target - cam.pos).normalize_or(Vec3::NEG_Z);
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(fwd);
        Affine3A::from_cols(right.into(), up.into(), (-fwd).into(), cam.pos.into())
    }

    /// Standard first-person mount before inspection or the ground pickup is applied.
    fn firing_mount(rig: &Rig, f: &GunFeel, reload: &gun::Reload) -> Affine3A {
        let pull = ((rig.build.barrel_length - 0.5) * 0.3).clamp(0.0, 0.3);
        let at = Vec3::new(
            0.3 + (f.phase * 0.5).sin() * 0.007 * f.bob,
            -0.3 + f.phase.sin() * 0.012 * f.bob - f.bump,
            -0.8 + f.kick + pull,
        ) + reload.lift;
        Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::YXZ, 0.0, reload.tilt, f.tilt), at)
    }

    /// Firing cancels inspection on this frame; the weapon can keep growing in the aiming pose.
    pub fn shot(&mut self, player: usize, world: &World, fx: &EffectsView) -> Option<Vec3> {
        let p = world.players.get(player)?;
        let rig = self.rigs.get_mut(player)?;
        let lifting = self.pickups[player].take().is_some();
        if lifting { rig.grow_to(Stats::of(p)); }
        rig.fire(&Stats::of(p));
        let f = &mut self.feel[player];
        let interrupt = lifting || f.inspect > 0. || f.previous_inspect > 0.;
        f.inspect = 0.;
        f.previous_inspect = 0.;
        f.kick = f.kick.max(0.08 * (p.weapon.recoil() / p.weapon.gun.recoil).clamp(0.2, 2.5));
        if interrupt && self.local[player] && !self.tracking[player] {
            // The last rendered muzzle may still point sideways or be on the floor.
            let mount = Self::firing_mount(rig, f, &gun::Reload::sample(p, 1.));
            let muzzle = mount.transform_point3(rig.muzzle_at(1.));
            let cam = fx.punched(world, player, self.camera(world, player, &[], 1.));
            return Some(Self::view_frame(cam).transform_point3(muzzle));
        }
        self.muzzle(player)
    }

    /// One tick of gun state: rigs follow their stats (morphs, heat, bolt,
    /// spin) and the first-person feel eases (bob, bump, tilt, kick).
    pub fn tick(&mut self, world: &World, dt: f32) {
        let new_round = self.round != Some(world.round);
        if new_round { self.reset_heat(); self.pickups.fill(None); }
        for event in &world.events {
            if let growing_guns::Event::RifleCollected { player } = *event {
                if let Some(pickup) = self.pickups.get_mut(player) {
                    *pickup = Some((world.tick, world.players[player].pos - Vec3::Y * growing_guns::HALF_HEIGHT));
                    self.own[player] = Some(Stats::weapon(&world.owned_weapon(player)));
                }
            }
        }
        for (i, p) in world.players.iter().enumerate().take(self.rigs.len()) {
            let rig = &mut self.rigs[i];
            let waiting = world.awaiting_rifle(i);
            let target = Stats::of(p);
            if new_round || self.own[i].is_none() {
                let owned = Stats::weapon(&world.owned_weapon(i));
                let previous = self.own[i].replace(owned);
                // Leave the owner's last rifle on the floor, even if the
                // previous round lent them another player's gun.
                rig.set(previous.unwrap_or(if waiting { owned } else { target }));
                if !waiting && previous.is_some() { rig.grow_to(target); }
            }
            if !waiting {
                if let Some((tick, _)) = self.pickups[i] {
                    let age = world.tick.saturating_sub(tick) as f32 * growing_guns::DT;
                    if age >= PICKUP_READY_SECONDS {
                        self.pickups[i] = None;
                        rig.grow_to(target);
                    }
                } else {
                    rig.follow(target);
                }
            }
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
            f.previous_inspect = if new_round { 0. } else { f.inspect };
            f.inspect = rig.inspection();
        }
        for own in self.own.iter_mut().skip(world.players.len()) { *own = None; }
        self.round = Some(world.round);
    }

    pub(super) fn sample_audience(&mut self, stadium: &crate::stadium::StadiumVisuals, world: &World, time: f32) {
        for (i, pose) in self.spectators.iter_mut().enumerate() {
            *pose = if world.awaiting_rifle(i) { stadium.participant_pose(i, time) } else { None };
        }
    }

    pub(super) fn spectator(&self, i: usize) -> Option<render::humans::StandingPose> {
        self.spectators.get(i).copied().flatten()
    }

    pub(super) fn crowd_lift(&self, i: usize, flight: f32) -> f32 {
        self.spectator(i).map_or(0., |pose|
            pose.lift * (1. - super::pov_arms::ease(0., 0.16, flight)))
    }

    pub(super) fn entrance_wrists(&self, world: &World, i: usize, flight: f32) -> [Affine3A; 2] {
        let seated = self.spectator(i);
        [0, 1].map(|side| super::pov_arms::flight_wrist(i, side, flight,
            seated.map(|pose| pose.wrists[side]), world.players[i].eye_height() + growing_guns::HALF_HEIGHT))
    }

    fn pickup(&self, world: &World, i: usize, alpha: f32) -> Option<(f32, Vec3)> {
        let (tick, floor) = self.pickups.get(i).copied().flatten()?;
        let ticks = world.tick.checked_sub(tick)?;
        let age = (ticks as f32 + alpha - 1.).max(0.) * growing_guns::DT;
        (age < PICKUP_READY_SECONDS && world.players[i].alive && !self.tracking[i]).then_some((age, floor))
    }

    pub(super) fn camera(&self, world: &World, i: usize, prev: &[crate::view::Pose], alpha: f32) -> render::Camera {
        let dip = self.pickup(world, i, alpha).map_or(0., |(age, _)| 1. - super::pov_arms::ease(0.10, 0.45, age));
        let mut camera = super::entrance_camera(world, i, prev, alpha, dip);
        let lift = self.crowd_lift(i, world.entrance_flight_at(alpha));
        camera.pos.y += lift;
        camera.target.y += lift;
        camera
    }

    /// Every rig into its two blocks: first person from the (punched)
    /// camera, third person in the right hand, aimed with the head.
    pub fn pose(&mut self, scene: &mut Scene, world: &World, prev: &[crate::view::Pose], alpha: f32, fx: &EffectsView, hands: &[Option<Affine3A>]) {
        for i in 0..self.rigs.len() {
            self.lamps[i] = None;
            if let Some(arms) = self.arms.get(i) { arms.clear(scene); }
            // Slots past the players (a coop wave's enemies gone): empty.
            let Some(p) = world.players.get(i) else {
                self.muzzles[i] = None;
                for first in [self.fp[i], self.tp[i]] { empty(scene, first, 0, self.parts); }
                continue;
            };
            if p.howitzer.is_some() && p.ghost && p.offer.is_none() {
                self.pose_howitzer(scene, world, prev, alpha, i);
                continue;
            }
            let rig = &self.rigs[i];
            self.barrels[i] = barrel_material(rig.heat_glow_t());
            let f = &self.feel[i];
            let reload = gun::Reload::sample(p, alpha);
            let cam = fx.punched(world, i, self.camera(world, i, prev, alpha));
            let view = Self::view_frame(cam);
            let s = f.previous_inspect + (f.inspect - f.previous_inspect) * alpha;
            let fp = view * Self::firing_mount(rig, f, &reload)
                * Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::YXZ, s, 0.12 * s, 0.08 * s), Vec3::new(-0.18, 0.17, 0.19) * s);
            let fp = self.tracked[i].unwrap_or(fp);
            let drawn = crate::view::pose(world, i, prev, alpha);
            let (pos, rot) = (drawn.pos, Quat::from_rotation_y(drawn.yaw));
            let tp = if let Some(mount) = hands.get(i).copied().flatten() {
                mount
            } else {
                Affine3A::from_rotation_translation(rot * Quat::from_rotation_x(drawn.pitch),
                    pos + Vec3::Y * 0.05 + rot * Vec3::new(0.45, 0.0, -0.12))
            };
            let ground = world.ground_rifle(i);
            let held = p.alive && !world.awaiting_rifle(i) && (!self.tracking[i] || self.tracked[i].is_some());
            let on_floor = |floor: Vec3| Affine3A::from_rotation_translation(
                Quat::from_rotation_y(0.75) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                floor + Vec3::Y * rig.build.resting_height().max(0.03),
            );
            let ground_pose = ground.map(on_floor);
            let pickup = self.pickup(world, i, alpha).map(|(age, floor)| (age, on_floor(floor)));
            let fp = if let Some((age, floor)) = pickup {
                let lift = super::pov_arms::ease(0.055, PICKUP_LIFT_SECONDS, age);
                super::pov_arms::blend_pose(floor, fp, lift)
            } else { fp };
            if self.local[i] && p.alive && !p.ghost && !self.tracking[i] {
                if let Some(arms) = self.arms.get(i) {
                    if held {
                        arms.hold(scene, fp, rig.contacts_at(alpha), &reload, pickup.map(|(age, _)| (age, view)));
                    } else {
                        // With hands by the hips, looking down reveals them. The
                        // flailing and reaching poses follow the view during flight.
                        let upright = Affine3A::from_rotation_translation(rot, cam.pos);
                        let flight = world.entrance_flight_at(alpha);
                        arms.fly(scene, upright, view, flight, self.entrance_wrists(world, i, flight), self.spectator(i).is_some());
                    }
                }
            }
            let exit = rig.muzzle_at(alpha);
            self.muzzles[i] = held.then(|| [fp.transform_point3(exit), tp.transform_point3(exit)]);
            let visible = held || ground_pose.is_some();
            // The clamp attaches to the front underside of the growing receiver.
            let lamp = (visible && world.modifier == Some(growing_guns::Modifier::Blackout)).then(|| {
                let (_, receiver) = rig.pose_at(0., alpha).next().expect("rifle receiver");
                Vec3::new(0., -0.5 * receiver.matrix3.y_axis.length(), -0.5 * receiver.matrix3.z_axis.length() + 0.012)
            });
            self.lamps[i] = lamp.map(|mount| {
                let weapon = ground_pose.unwrap_or(if self.local[i] { fp } else { tp });
                weapon * Affine3A::from_translation(mount + LAMP_LENS)
            });
            for (first, xf, drop) in [(self.fp[i], fp, reload.view_drop), (self.tp[i], tp, reload.hand_drop)] {
                let xf = ground_pose.unwrap_or(xf);
                let children = rig.pose_at(drop, alpha).take(if visible { self.parts - 1 } else { 0 })
                    .map(|(part, local)| (self.mesh(part.shape, part.mat, i), local))
                    .chain(lamp.map(|mount| (self.lamp_mesh, Affine3A::from_translation(mount))));
                let n = scene.update_children(first, xf, children);
                empty(scene, first, n, self.parts);
            }
        }
    }

    fn pose_howitzer(&mut self, scene: &mut Scene, world: &World, prev: &[crate::view::Pose], alpha: f32, i: usize) {
        use gun::{Shape::*, Mat::*};
        let p=&world.players[i];
        let drawn=crate::view::pose(world,i,prev,alpha);
        let yaw=Quat::from_rotation_y(drawn.yaw);
        let elevation=p.artillery_elevation(drawn.pitch);
        let aim=yaw*Quat::from_rotation_x(elevation);
        let pivot=drawn.pos+Vec3::Y*(p.eye_height()-0.45);
        let recoil=0.24*(-p.since_shot*9.0).exp();
        let tube=Affine3A::from_rotation_translation(aim,pivot+aim*Vec3::Z*recoil);
        let base=Affine3A::from_rotation_translation(yaw,drawn.pos-Vec3::Y*0.65);
        let mut pieces=Vec::new();
        let mut part=|shape,mat,xf:Affine3A,size:Vec3| pieces.push((shape,mat,xf*Affine3A::from_scale(size)));
        let cylinder=Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        part(Box,Dark,base,Vec3::new(1.8,0.25,2.3));
        part(Cyl,Metal,base*Affine3A::from_translation(Vec3::Y*0.35),Vec3::new(0.55,0.7,0.55));
        if !p.siege_crossbow {
        part(Box,Metal,tube*Affine3A::from_translation(Vec3::new(0.0,0.0,-0.5)),Vec3::new(0.85,0.65,1.2));
        part(Cyl,Barrel,tube*Affine3A::from_rotation_translation(cylinder,Vec3::new(0.0,0.0,-1.85)),Vec3::new(0.28,2.3,0.28));
        part(Ring25,Metal,tube*Affine3A::from_rotation_translation(cylinder,Vec3::new(0.0,0.0,-3.03)),Vec3::splat(0.36));
        part(Cyl,Recess,tube*Affine3A::from_rotation_translation(cylinder,Vec3::new(0.0,0.0,-3.01)),Vec3::new(0.25,0.015,0.25));
        } else {
            part(Box,Dark,tube*Affine3A::from_translation(Vec3::new(0.0,0.0,-1.3)),Vec3::new(0.35,0.3,3.0));
            for side in [-1.0,1.0] {
                part(Box,Metal,tube*Affine3A::from_rotation_translation(Quat::from_rotation_y(side*0.3),Vec3::new(side*0.9,0.0,-2.1)),Vec3::new(1.9,0.15,0.22));
                part(Box,Recess,tube*Affine3A::from_rotation_translation(Quat::from_rotation_y(side*0.72),Vec3::new(side*0.9,0.05,-1.59)),Vec3::new(2.4,0.04,0.04));
            }
            if p.special_cd < 0.2 {
                part(Cyl,Metal,tube*Affine3A::from_rotation_translation(cylinder,Vec3::new(0.0,0.25,-1.6)),Vec3::new(0.09,2.4,0.09));
                part(Box,Metal,tube*Affine3A::from_rotation_translation(Quat::from_rotation_y(0.785),Vec3::new(0.0,0.25,-2.85)),Vec3::new(0.32,0.1,0.32));
            }
        }
        for side in [-1.0,1.0] {
            part(Cyl,Dark,base*Affine3A::from_rotation_translation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),Vec3::new(side*0.9,0.15,0.2)),Vec3::new(0.6,0.3,0.6));
            part(Box,Metal,tube*Affine3A::from_translation(Vec3::new(side*0.35,-0.25,-1.4)),Vec3::new(0.15,0.15,1.6));
        }
        for first in [self.fp[i],self.tp[i]] {
            for (n,(shape,mat,xf)) in pieces.iter().enumerate() { scene.update_instance(first+n,self.mesh(*shape,*mat,i),*xf); }
            empty(scene,first,pieces.len(),self.parts);
        }
        let muzzle=pivot+aim*Vec3::NEG_Z*3.2;
        self.muzzles[i]=Some([muzzle,muzzle]);
    }

    pub fn capacity(&self) -> usize { self.rigs.len() }

    /// Headless growth review uses the same reserved slots and rig poses.
    pub(crate) fn review_pose(&self, scene: &mut Scene, rig: &Rig) {
        self.review_mounted(scene, rig, Affine3A::IDENTITY);
    }

    pub(crate) fn review_mounted(&self, scene: &mut Scene, rig: &Rig, mount: Affine3A) {
        let mut n = 0;
        for (part, xf) in rig.pose(0.0) {
            assert!(n < self.parts, "gun morph exceeds reserved visual capacity");
            scene.update_instance(self.tp[0] + n, self.mesh(part.shape, part.mat, 0), mount * xf);
            n += 1;
        }
        empty(scene, self.tp[0], n, self.parts);
    }

    /// Each local player sees only their own first-person gun, and not
    /// their own third-person one.
    pub fn hide_own(&mut self, scene: &mut Scene, slots: &[usize]) {
        for (player, (&fp, &tp)) in self.fp.iter().zip(&self.tp).enumerate() {
            let views = render::scene::PlayerViews::for_player(slots, player);
            self.local[player] = views.hide_from_owner() != 0;
            if let Some(arms) = self.arms.get(player) { arms.mask(scene, views.hide_from_others()); }
            for part in &mut scene.instances[fp..fp + self.parts] {
                part.hide_in_views(views.hide_from_others());
            }
            for part in &mut scene.instances[tp..tp + self.parts] {
                part.hide_in_views(views.hide_from_owner());
            }
        }
    }

    /// Current tracked barrel end in VR; otherwise the last drawn barrel end
    /// in the local first-person view or the remote player's hand.
    pub fn muzzle(&self, i: usize) -> Option<Vec3> {
        let [fp, tp] = self.muzzles.get(i).copied().flatten()?;
        if self.tracking[i] {
            return self.tracked[i].map(|pose|self.tracked_muzzle(i,pose));
        }
        Some(if self.local[i] { fp } else { tp })
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

// Lens centre in attachment space. Put the shadow camera just beyond the bezel.
const LAMP_LENS: Vec3 = Vec3::new(0., -0.035, -0.120);

fn flashlight_mesh(scene: &mut Scene) -> usize {
    let metal = gun_material(scene, gun::Mat::Dark);
    let bezel = gun_material(scene, gun::Mat::Bolt);
    let lens = glow(scene, [1., 0.97, 0.88], 6000.);
    let barrel = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let mut pieces = [
        (primitives::cube(metal), Vec3::new(0.032, 0.025, 0.054), Quat::IDENTITY, Vec3::new(0., -0.009, 0.)),
        (primitives::cylinder(metal, 1., 1., 16), Vec3::new(0.020, 0.14, 0.020), barrel, Vec3::new(0., -0.035, -0.024)),
        (primitives::cylinder(bezel, 0.8, 1., 16), Vec3::new(0.029, 0.039, 0.029), barrel, Vec3::new(0., -0.035, -0.097)),
        (primitives::cylinder(lens, 1., 1., 16), Vec3::new(0.023, 0.002, 0.023), barrel, LAMP_LENS + 0.003 * Vec3::Z),
    ];
    for (primitive, size, rotation, centre) in &mut pieces {
        for v in &mut primitive.vertices {
            v.pos = (*rotation * (*size * Vec3::from(v.pos)) + *centre).into();
            v.normal = (*rotation * (Vec3::from(v.normal) / *size).normalize()).into();
        }
    }
    scene.add_mesh(&render::cluster::ClusterMesh::build(&pieces.each_ref().map(|piece| piece.0.part())))
}

/// Same dimensions at both detail levels, so no joint or animation changes.
fn gun_primitive(shape: gun::Shape, mat: u32, detail: gun::Detail) -> primitives::Primitive {
    use gun::Shape::*;
    let low = detail == gun::Detail::Low;
    // Eight sides keep the cardinal mounting surfaces under the sights.
    let sides = 8;
    match shape {
        Box => if low { primitives::cube(mat) } else { gun_box(mat) },
        VentedTube => vented_tube(mat),
        Cyl => primitives::cylinder(mat, 1.0, 1.0, sides),
        Taper85 => primitives::cylinder(mat, 1.0, 0.85, sides),
        Taper105 => primitives::cylinder(mat, 1.0, 1.05, sides),
        Ring18 | Ring25 => primitives::torus(mat, if shape == Ring18 { 0.099 } else { 0.143 }, if low { 8 } else { 10 }, if low { 3 } else { 6 }),
        Grip => grip_mesh(mat, false),
        GripPanel => grip_mesh(mat, true),
        Drum => if low { primitives::cylinder(mat, 1.0, 1.0, 16) } else { drum_mesh(mat) },
        Vial => primitives::cylinder(mat, 1.0, 1.0, if low { 8 } else { 24 }),
    }
}

/// Four open longitudinal vents with actual wall thickness and intact end bands.
fn vented_tube(material: u32) -> primitives::Primitive {
    let mut mesh = primitives::Primitive { material, ..Default::default() };
    let mut quad = |p: [Vec3; 4], normals: [Vec3; 4]| {
        let start = mesh.vertices.len() as u32;
        for (v, n) in p.into_iter().zip(normals) {
            mesh.vertices.push(render::cluster::Vertex { pos: v.to_array(), normal: n.to_array(), uv: [0.0; 2] });
        }
        for mut tri in [[0, 1, 2], [0, 2, 3]] {
            if (p[tri[1]] - p[tri[0]]).cross(p[tri[2]] - p[tri[0]]).dot(normals[0]) < 0.0 { tri.swap(1, 2); }
            mesh.indices.extend(tri.map(|i| start + i as u32));
        }
    };
    // Match the drum's 48-sided silhouette. Radial vertex normals make the
    // curved shell continuous; vent walls and end faces retain sharp edges.
    const N: usize = 48;
    for i in 0..N {
        let a = i as f32 * std::f32::consts::TAU / N as f32;
        let b = (i + 1) as f32 * std::f32::consts::TAU / N as f32;
        let u = Vec3::new(a.cos(), 0.0, a.sin());
        let v = Vec3::new(b.cos(), 0.0, b.sin());
        for (band, (lo, hi, r0, r1)) in [(-0.5,-0.46,0.96,1.0),(-0.46,-0.34,1.0,1.0),
            (-0.34,0.34,1.0,1.0),(0.34,0.46,1.0,1.0),(0.46,0.5,1.0,0.96)].into_iter().enumerate() {
            if band == 2 && i % 12 < 3 { continue; }
            let p = |axis: Vec3, r: f32, y: f32| axis * r + Vec3::Y * y;
            let slope = (r0 - r1) / (hi - lo);
            let nu = (u + Vec3::Y * slope).normalize();
            let nv = (v + Vec3::Y * slope).normalize();
            quad([p(u,r0,lo),p(v,r0,lo),p(v,r1,hi),p(u,r1,hi)], [nu,nv,nv,nu]);
            quad([p(u,0.85,lo),p(v,0.85,lo),p(v,0.85,hi),p(u,0.85,hi)], [-u,-v,-v,-u]);
            for (y,r,n) in [(lo,r0,-Vec3::Y),(hi,r1,Vec3::Y)] {
                quad([p(u,0.85,y),p(v,0.85,y),p(v,r,y),p(u,r,y)], [n;4]);
            }
            for (axis,n) in [(u,Vec3::new(a.sin(),0.0,-a.cos())),(v,Vec3::new(-b.sin(),0.0,b.cos()))] {
                quad([p(axis,0.85,lo),p(axis,r0,lo),p(axis,r1,hi),p(axis,0.85,hi)], [n;4]);
            }
        }
    }
    mesh
}

/// Machined unit box: broad flat faces at exactly +/-0.5, with separate
/// chamfers. A one-quad hewn_block only samples its inset corners and shrinks
/// the entire box, opening every joint whose placement assumes a unit cube.
fn gun_box(material: u32) -> primitives::Primitive {
    let mut mesh = primitives::Primitive { material, ..Default::default() };
    let inner = 0.475;
    let mut face = |points: &[Vec3], normal: Vec3| {
        let first = mesh.vertices.len() as u32;
        for &p in points {
            mesh.vertices.push(render::cluster::Vertex { pos: p.to_array(), normal: normal.to_array(), uv: [0.0; 2] });
        }
        for i in 1..points.len() - 1 {
            let mut tri = [first, first + i as u32, first + i as u32 + 1];
            if (points[i] - points[0]).cross(points[i + 1] - points[0]).dot(normal) < 0.0 { tri.swap(1, 2); }
            mesh.indices.extend(tri);
        }
    };
    for n in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
        let u = if n.y.abs() > 0.5 { Vec3::X } else { Vec3::Y }.cross(n);
        let v = n.cross(u);
        face(&[n * 0.5 - u * inner - v * inner, n * 0.5 + u * inner - v * inner,
            n * 0.5 + u * inner + v * inner, n * 0.5 - u * inner + v * inner], n);
    }
    for (a, b, c) in [(Vec3::X, Vec3::Y, Vec3::Z), (Vec3::Y, Vec3::Z, Vec3::X), (Vec3::Z, Vec3::X, Vec3::Y)] {
        for sa in [-1.0, 1.0] { for sb in [-1.0, 1.0] {
            let (a, b) = (a * sa, b * sb);
            face(&[a * 0.5 + b * inner - c * inner, a * inner + b * 0.5 - c * inner,
                a * inner + b * 0.5 + c * inner, a * 0.5 + b * inner + c * inner], (a + b).normalize());
        } }
    }
    for x in [-1.0, 1.0] { for y in [-1.0, 1.0] { for z in [-1.0, 1.0] {
        face(&[Vec3::new(x * 0.5, y * inner, z * inner), Vec3::new(x * inner, y * 0.5, z * inner),
            Vec3::new(x * inner, y * inner, z * 0.5)], Vec3::new(x, y, z).normalize());
    } } }
    mesh
}

/// Round magazine, radius 1 and depth 1 along Y. Forty-eight radial segments
/// and a rounded rim keep the large silhouette smooth, independently of the
/// low-sided barrels. Its mounting envelope matches the original cylinder.
fn drum_mesh(material: u32) -> primitives::Primitive {
    let mut mesh = primitives::Primitive { material, ..Default::default() };
    const N: u32 = 48;
    for side in [-1.0f32, 1.0] {
        for k in 0..4 {
            let step = if side < 0.0 { 3 - k } else { k };
            let angle = step as f32 * std::f32::consts::FRAC_PI_2 / 3.0;
            let (sn, cs) = angle.sin_cos();
            let y = side * (0.36 + 0.14 * sn);
            let radius = 0.94 + 0.06 * cs;
            for i in 0..N {
                let a = i as f32 * std::f32::consts::TAU / N as f32;
                let radial = Vec3::new(a.cos(), 0.0, a.sin());
                let normal = (radial * (cs / 0.06) + Vec3::Y * (side * sn / 0.14)).normalize();
                mesh.vertices.push(render::cluster::Vertex { pos: (radial * radius + Vec3::Y * y).to_array(), normal: normal.to_array(), uv: [0.0; 2] });
            }
        }
    }
    let cap = mesh.vertices.len() as u32;
    for side in [-1.0f32, 1.0] {
        mesh.vertices.push(render::cluster::Vertex { pos: [0.0, side * 0.5, 0.0], normal: [0.0, side, 0.0], uv: [0.0; 2] });
    }
    let mut triangle = |a: u32, b: u32, c: u32| {
        let v = [a, b, c].map(|i| mesh.vertices[i as usize]);
        let p = v.map(|v| Vec3::from(v.pos));
        let normal: Vec3 = v.iter().map(|v| Vec3::from(v.normal)).sum();
        mesh.indices.extend(if (p[1] - p[0]).cross(p[2] - p[0]).dot(normal) < 0.0 { [a, c, b] } else { [a, b, c] });
    };
    for row in 0..7 {
        for i in 0..N {
            let (a, b) = (row * N + i, row * N + (i + 1) % N);
            triangle(a, b, a + N); triangle(b, b + N, a + N);
        }
    }
    for i in 0..N {
        triangle(cap, i, (i + 1) % N);
        triangle(cap + 1, 7 * N + i, 7 * N + (i + 1) % N);
    }
    mesh
}

/// Chamfered cross sections create an ergonomic grip with a neck, palm swell
/// and heel. The rubber insert follows the same contour and sits on both sides.
fn grip_mesh(material: u32, panel: bool) -> primitives::Primitive {
    let mut mesh = primitives::Primitive { material, ..Default::default() };
    let sections = if panel { &gun::GRIP_SECTIONS[2..=4] } else { &gun::GRIP_SECTIONS[..] };
    let rings: Vec<[Vec3; 8]> = sections.iter().map(|&(y, width, front, back)| {
        let (w, f, b) = if panel { (width + 0.0006, front + 0.007, back - 0.007) } else { (width, front, back) };
        let bevel = if panel { 0.001 } else { 0.004 };
        [(-w + bevel, f), (w - bevel, f), (w, f + bevel), (w, b - bevel),
            (w - bevel, b), (-w + bevel, b), (-w, b - bevel), (-w, f + bevel)]
            .map(|(x, z)| Vec3::new(x, y, z))
    }).collect();
    let mut face = |points: &[Vec3]| {
        for i in 1..points.len() - 1 {
            let tri = [points[0], points[i], points[i + 1]];
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
            let first = mesh.vertices.len() as u32;
            for p in tri { mesh.vertices.push(render::cluster::Vertex { pos: p.to_array(), normal: normal.to_array(), uv: [0.0; 2] }); }
            mesh.indices.extend([first, first + 1, first + 2]);
        }
    };
    for pair in rings.windows(2) {
        for i in 0..8 {
            let j = (i + 1) % 8;
            face(&[pair[0][i], pair[0][j], pair[1][j], pair[1][i]]);
        }
    }
    let mut top = rings[0]; top.reverse(); face(&top);
    face(rings.last().unwrap());
    mesh
}

/// Isolated review scene using the exact gameplay meshes and materials.
/// Empty reserved slots remain hidden; only the settled gun is instantiated.
pub(crate) fn review_scene(stats: Stats) -> (Scene, Vec3, Vec3, usize, usize) {
    let mut scene = Scene::default();
    let guns = Guns::reserve(&mut scene, 1);
    let mut rig = Rig::with_detail(guns.detail);
    rig.set(stats);
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for (part, xf) in rig.pose(0.0) {
        scene.add_instance(guns.mesh(part.shape, part.mat, 0), xf);
        let (min, max) = match part.shape {
            gun::Shape::Grip | gun::Shape::GripPanel => (Vec3::new(-0.023, -0.135, -0.028), Vec3::new(0.023, 0.009, 0.063)),
            gun::Shape::Box => (Vec3::splat(-0.5), Vec3::splat(0.5)),
            _ => (Vec3::new(-1.15, -0.5, -1.15), Vec3::new(1.15, 0.5, 1.15)),
        };
        for x in [min.x, max.x] { for y in [min.y, max.y] { for z in [min.z, max.z] {
            let p = xf.transform_point3(Vec3::new(x, y, z));
            lo = lo.min(p); hi = hi.max(p);
        } } }
    }
    let triangles = rig.build.parts.iter().map(|p| gun_primitive(p.shape, 0, guns.detail).indices.len() / 3).sum();
    (scene, lo, hi, rig.build.parts.len(), triangles)
}

/// Empty a gun block from part `n` on, up to the first already empty slot.
fn empty(scene: &mut Scene, first: usize, n: usize, capacity: usize) {
    for inst in scene.instances[first + n..first + capacity].iter_mut().take_while(|i| i.cluster_count != 0) {
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
        Fire => scene.add_material(Material { tint: [1.0, 0.40, 0.015], layer: -2,
            roughness: 1.0, flags: render::scene::LAVA, scale: 18000.0, metal: 0.0 }),
        Barrel => unreachable!("per-player barrel_material"),
        Polymer => material(scene, [0.3, 0.25, 0.18], 0.7),
        Pad => material(scene, [0.07, 0.07, 0.075], 0.7),
        Recess => metal(scene, k(0.2), 0.3, 0.8),
        Slot => metal(scene, k(0.25), 0.4, 0.85),
        Bolt => metal(scene, [0.55, 0.57, 0.62], 0.95, 0.18),
        Lens => glow(scene, [0.35, 0.7, 1.0], 7000.0),
        Indicator => glow(scene, [0.4, 1.0, 0.95], 10000.0),
        Poison => glow(scene, [0.06, 1.0, 0.004], 2800.0),
        Cooling => glow(scene, [0.012, 0.38, 1.0], 4200.0),
        Glass => {
            let mat = material(scene, [0.65, 0.82, 0.9], 0.16);
            scene.set_material_blend(mat, 0.18, false);
            mat
        }
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

#[cfg(test)]
mod xr_tests {
    use super::*;
    #[test]
    fn next_round_uses_owned_gun_and_invisible_cards_do_not_inspect() {
        for detail in [gun::Detail::Full, gun::Detail::Low] {
        let mut world = World::new(7, &[false, false]);
        let _scripts = crate::scripts::attach(&mut world).unwrap();
        world.release_cage();
        for (i, card) in [(0, "sniper"), (1, "uzi")] { let c = world.cards.find(card).unwrap(); world.give(i, c); }
        let own = Stats::weapon(&world.owned_weapon(0));
        let borrowed = world.owned_weapon(1);
        let mut scene = Scene::default();
        let mut guns = Guns::reserve_with_detail(&mut scene, 2, detail);
        guns.tick(&world, 0.0);
        world.modifier = Some(growing_guns::Modifier::GunSwap);
        world.players[0].weapon = borrowed.clone();
        guns.tick(&world, 0.0);
        assert_ne!(guns.rigs[0].shown, own);
        // A short previous round may still have had its inspection raised.
        guns.feel[0].inspect = 0.8;
        guns.feel[0].previous_inspect = 0.7;
        let card = world.cards.find("acrobat").unwrap(); world.give(0, card);
        guns.rigs[0].heat = 1.5; guns.barrels[0] = barrel_material(1.0);
        world.round += 1; world.modifier = None; world.rebuild_weapon(0);
        guns.tick(&world, 0.0);
        assert_eq!(guns.rigs[0].shown, own);
        assert_eq!(guns.rigs[0].heat, 0.0);
        assert_eq!(guns.barrels[0].flags, barrel_material(0.0).flags);
        guns.rigs[0].heat = 1.5; guns.reset_heat();
        assert_eq!(guns.rigs[0].heat, 0.0);
        assert!(!guns.rigs[0].is_growing());
        assert_eq!(guns.feel[0].inspect, 0.0);
        assert_eq!(guns.feel[0].previous_inspect, 0.0);
        world.players[0].weapon = borrowed;
        guns.tick(&world, 0.0);
        let card = world.cards.find("heavy_rounds").unwrap(); world.give(0, card);
        world.round += 1; world.rebuild_weapon(0);
        guns.tick(&world, 0.0);
        assert!(guns.rigs[0].is_growing());
        assert_eq!(guns.rigs[0].shown, own, "the previous round's borrowed Uzi must not be the source");
        }
    }

    #[test]
    fn entrance_tracks_the_seat_phase_until_takeoff() {
        use render::{crowd::Seat, humans::Instance};
        let world = World::new(3, &[false]);
        let mut scene = Scene::default();
        let mut guns = Guns::reserve_with_detail(&mut scene, 1, gun::Detail::Low);
        let seat = Seat { pos: Vec3::ZERO, yaw: 0., scale: 1.8, width: 0.9,
            phase: 0.25, tempo: 0.75, hop: 0.4, body: 0, arms: 0 };
        let person = Instance::spectator(3, 0, &seat);
        let airborne = guns.entrance_wrists(&world, 0, 0.5);
        let mut raised = [f32::MIN; 2];
        let mut lowered = [f32::MAX; 2];
        for frame in 0..240 {
            let pose = person.standing_pose(&seat, frame as f32 / 30., 1., 0., true);
            guns.spectators[0] = Some(pose);
            assert_eq!(guns.crowd_lift(0, 0.), pose.lift);
            assert_eq!(guns.crowd_lift(0, 0.5), 0.);
            let seated = guns.entrance_wrists(&world, 0, 0.);
            for side in 0..2 {
                let height = pose.wrists[side].translation.y;
                raised[side] = raised[side].max(height);
                lowered[side] = lowered[side].min(height);
                let expected = height * 1.8 - world.players[0].eye_height() - growing_guns::HALF_HEIGHT;
                assert!((seated[side].translation.y - expected).abs() < 1e-5);
                let flying = guns.entrance_wrists(&world, 0, 0.5)[side];
                assert!(flying.abs_diff_eq(airborne[side], 1e-5));
            }
        }
        assert!((0..2).any(|side| raised[side] > 0.82 && lowered[side] < 0.62));
    }

    #[test]
    fn only_visible_upgrades_inspect_after_the_rifle_is_in_hand() {
        for detail in [gun::Detail::Full, gun::Detail::Low] {
            let mut world = World::new_with(7, &[false, false], |w| w.modifier_chance = 0.);
            let _scripts = crate::scripts::attach(&mut world).unwrap();
            let mut guns = Guns::reserve_with_detail(&mut Scene::default(), 2, detail);
            world.release_cage();
            guns.tick(&world, 0.);
            let original = guns.rigs[0].build.barrel_length;
            let source = guns.rigs[0].shown;
            for (i, id) in [(0, "sniper"), (1, "acrobat")] {
                let card = world.cards.find(id).unwrap();
                world.give(i, card);
            }
            world.next_round();
            guns.tick(&world, 0.);
            guns.grow(0, &world);
            let target = Stats::of(&world.players[0]);
            while world.awaiting_rifle(0) {
                assert!(!guns.rigs[0].is_growing());
                assert_eq!(guns.rigs[0].shown, source);
                world.step(&[]);
                guns.tick(&world, growing_guns::DT);
            }
            // The floor pickup and the return to aim show the old shape.
            for _ in 0..28 {
                assert!(!guns.rigs[0].is_growing());
                assert_eq!(guns.rigs[0].shown, source);
                assert_eq!(guns.feel[0].inspect, 0.);
                world.step(&[]); guns.tick(&world, growing_guns::DT);
            }
            assert!(guns.rigs[0].is_growing());
            for _ in 0..60 { world.step(&[]); guns.tick(&world, growing_guns::DT); }
            let midway = guns.rigs[0].build.barrel_length;
            assert!(guns.feel[0].inspect > 0.95);
            assert!(!guns.rigs[1].is_growing() && guns.feel[1].inspect == 0.);
            for _ in 0..60 { world.step(&[]); guns.tick(&world, growing_guns::DT); }
            assert!(original < midway && midway < guns.rigs[0].build.barrel_length);
            assert!(!guns.rigs[0].is_growing());
            assert_eq!(guns.rigs[0].shown, target);
            assert_eq!(guns.own[0], Some(Stats::weapon(&world.owned_weapon(0))));
        }
    }

    #[test]
    fn a_shot_cancels_the_lift_or_inspection_on_the_same_frame() {
        for during_lift in [true, false] {
            let mut world = World::new(3, &[false]);
            world.release_cage();
            let mut scene = Scene::default();
            let mut guns = Guns::reserve_with_detail(&mut scene, 1, gun::Detail::Full);
            let fx = EffectsView::reserve(&mut scene, 3);
            guns.hide_own(&mut scene, &[0]);
            guns.tick(&world, 0.);
            world.players[0].weapon.damage_mult = 2.;
            if during_lift {
                guns.pickups[0] = Some((world.tick, world.players[0].pos - Vec3::Y * growing_guns::HALF_HEIGHT));
            } else {
                guns.grow(0, &world);
                for _ in 0..30 { guns.tick(&world, growing_guns::DT); }
                assert_eq!(guns.feel[0].inspect, 1.);
            }
            guns.pose(&mut scene, &world, &[], 1., &fx, &[]);
            let before = guns.muzzle(0).unwrap();
            world.players[0].recoil_pitch = 0.08;
            let flash = guns.shot(0, &world, &fx).unwrap();
            assert!(flash.distance(before) > 0.1);
            for alpha in [0., 0.5, 1.] {
                guns.pose(&mut scene, &world, &[], alpha, &fx, &[]);
                assert!(flash.distance(guns.muzzle(0).unwrap()) < 0.015);
            }
            assert!(guns.rigs[0].is_growing());
            for _ in 0..95 {
                guns.tick(&world, growing_guns::DT);
                assert_eq!((guns.feel[0].previous_inspect, guns.feel[0].inspect), (0., 0.));
            }
            assert!(!guns.rigs[0].is_growing());
            world.players[0].weapon.damage_mult = 1.;
            guns.grow(0, &world);
            guns.tick(&world, growing_guns::DT);
            assert!(guns.feel[0].inspect > 0.);
        }
    }

    #[test]
    fn ground_rifle_becomes_held_in_both_views_on_touchdown() {
        let mut world = World::new(3, &[false, false]);
        let mut scene = Scene::default();
        let mut guns = Guns::reserve_with_detail(&mut scene, 2, gun::Detail::Low);
        let arm_start = scene.instances.len();
        guns.reserve_arms(&mut scene, 3, &[]);
        world.modifier = Some(growing_guns::Modifier::Blackout);
        let fx = EffectsView::reserve(&mut scene, 3);
        guns.hide_own(&mut scene, &[0, 1]);
        guns.tick(&world, 0.0);
        for _ in 0..164 { world.step(&[]); }
        let mut open_hands = [0; 4];
        for (ticks, armed, raised) in [(0, false, false), (1, true, false), (26, true, true)] {
            for _ in 0..ticks {
                world.step(&[]);
                guns.tick(&world, growing_guns::DT);
            }
            guns.pose(&mut scene, &world, &[], 1.0, &fx, &[]);
            for i in 0..2 {
                for first in [guns.fp[i], guns.tp[i]] {
                    assert!(scene.instances[first].cluster_count > 0);
                    if !armed {
                        let centre = Vec3::from_slice(&scene.instances[first].sphere[..3]);
                        assert!(centre.distance(world.ground_rifle(i).unwrap()) < 0.4);
                    }
                }
                let lamp = guns.lamps[i].expect("equipped blackout lamp");
                let near_floor = lamp.translation.y < world.players[i].pos.y - 0.5;
                assert_eq!(near_floor, !raised);
                for side in 0..2 {
                    let k = 2 * i + side;
                    let hand = &scene.instances[arm_start + k];
                    assert!(hand.cluster_count > 0);
                    assert_eq!((hand.flags >> render::scene::HIDE_IN_VIEW_SHIFT) as u8, !(1 << i));
                    if raised { assert_ne!(hand.first_cluster, open_hands[k]); }
                    else if !armed { open_hands[k] = hand.first_cluster; }
                    else { assert_eq!(hand.first_cluster, open_hands[k]); }
                }
                assert_eq!(guns.muzzle(i).is_some(), armed);
            }
        }
        world.modifier = None;
        guns.pose(&mut scene, &world, &[], 1., &fx, &[]);
        assert!(guns.lamps.iter().all(Option::is_none));
    }

    #[test]
    fn gun_box_preserves_mounting_dimensions_and_is_closed() {
        let mesh = gun_box(0);
        let mut edges = std::collections::BTreeMap::new();
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let lo = mesh.vertices.iter().map(|v| Vec3::from(v.pos).dot(axis)).fold(f32::INFINITY, f32::min);
            let hi = mesh.vertices.iter().map(|v| Vec3::from(v.pos).dot(axis)).fold(f32::NEG_INFINITY, f32::max);
            assert_eq!((lo, hi), (-0.5, 0.5), "mounting dimensions must match the part layout");
        }
        for t in mesh.indices.chunks_exact(3) {
            let v = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
            let p = v.map(|v| Vec3::from(v.pos));
            assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(Vec3::from(v[0].normal)) > 0.0);
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let mut edge = [v[a].pos.map(f32::to_bits), v[b].pos.map(f32::to_bits)];
                edge.sort();
                *edges.entry(edge).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&count| count == 2), "every chamfer edge must be sealed");
    }

    #[test]
    fn stereo_eyes_share_the_local_gun_and_hide_its_third_person_copy() {
        let mut scene=Scene::default();
        let mut guns=Guns::reserve(&mut scene,2);
        guns.hide_own(&mut scene,&[0,0]);
        let hidden=|index:usize| (scene.instances[index].flags & render::scene::HIDE_IN_VIEW_MASK)>>render::scene::HIDE_IN_VIEW_SHIFT;
        assert_eq!(hidden(guns.fp[0]) & 3,0);
        assert_eq!(hidden(guns.tp[0]) & 3,3);
        assert_eq!(hidden(guns.fp[1]) & 3,3);
        assert_eq!(hidden(guns.tp[1]) & 3,0);
    }
    #[test]
    fn split_screen_reassignment_updates_every_gun_part() {
        let mut scene = Scene::default();
        let mut guns = Guns::reserve_with_detail(&mut scene, 3, gun::Detail::Low);
        for (slots, owners) in [
            (&[2, 0][..], [2u8, 0, 1]),
            (&[1, 1, 0][..], [4, 3, 0]),
            (&[][..], [0, 0, 0]),
        ] {
            guns.hide_own(&mut scene, slots);
            for (player, mask) in owners.into_iter().enumerate() {
                assert_eq!(guns.local[player], mask != 0);
                for (first, hidden) in [(guns.fp[player], !mask), (guns.tp[player], mask)] {
                    for instance in &scene.instances[first..first + guns.parts] {
                        assert_eq!((instance.flags >> render::scene::HIDE_IN_VIEW_SHIFT) as u8, hidden);
                    }
                }
            }
        }
    }

    #[test]
    fn low_guns_reduce_geometry_without_glass_or_mounting_changes() {
        use gun::{Detail, Look, Mat};
        let cases = [Stats::default(), Stats { poison: 0.7, cooling: 0.52, ..Default::default() },
            Stats { mag: 60, fire_rate_mult: 8.0, ..Default::default() },
            Stats { shots: 16, damage_mult: 5.0, spread: 0.0, mag: 60, poison: 0.7, cooling: 0.52, ..Default::default() }];
        for stats in cases {
            let full = gun::build_with_detail(&Look::of(&stats), Detail::Full);
            let low = gun::build_with_detail(&Look::of(&stats), Detail::Low);
            let triangles = |b: &gun::Build, detail| b.parts.iter().map(|p| gun_primitive(p.shape, 0, detail).indices.len() / 3).sum::<usize>();
            assert!(triangles(&low, Detail::Low) * 10 < triangles(&full, Detail::Full) * 7);
            assert!(low.parts.len() < full.parts.len());
            assert_eq!(full.muzzle, low.muzzle);
            assert_eq!(full.barrel_length, low.barrel_length);
            assert!(low.parts.iter().all(|p| p.mat != Mat::Glass));
            for part in full.parts.iter().filter(|p| p.key / 1024 == 7 && p.key % 1024 >= 6) {
                assert_eq!(part.xf, low.parts.iter().find(|q| q.key == part.key).unwrap().xf, "trigger and guard must stay intact");
            }
            let top = |b: &gun::Build| b.parts.iter().filter(|p| p.key / 1024 == 2)
                .map(|p| p.xf.translation.y + p.xf.matrix3.y_axis.length() * 0.5).fold(0.0, f32::max);
            assert!((top(&full) - top(&low)).abs() < 1e-5);
        }
        let mut scene = Scene::default();
        let guns = Guns::reserve_with_detail(&mut scene, 2, Detail::Low);
        assert!(scene.blended_materials.is_empty());
        assert_eq!(scene.instances.len(), 4 * (Detail::Low.part_capacity() + 1), "each view reserves one blackout lamp per rifle");
        guns.review_pose(&mut scene, &Rig::with_detail(Detail::Low));
        // Cylinder extrema still meet the unchanged sights and magazine mount.
        for shape in [gun::Shape::Box, gun::Shape::Cyl, gun::Shape::Drum] {
            let a = gun_primitive(shape, 0, Detail::Full);
            let b = gun_primitive(shape, 0, Detail::Low);
            for axis in 0..3 {
                for sign in [-1.0, 1.0] {
                    let extent = |p: &primitives::Primitive| p.vertices.iter().map(|v| sign * v.pos[axis]).fold(0.0, f32::max);
                    assert!((extent(&a) - extent(&b)).abs() < 1e-5);
                }
            }
        }
    }

}

#[cfg(test)] mod tracked_flash_tests {
    use super::*;
    #[test] fn shot_muzzle_uses_current_tracking_before_a_render_pose() {
        let mut scene=Scene::default();let mut guns=Guns::reserve_with_detail(&mut scene,1,gun::Detail::Low);
        guns.muzzles[0]=Some([Vec3::splat(99.);2]);
        for (pos,angle) in [(Vec3::new(0.3,1.2,-0.4),0.1),(Vec3::new(-0.5,1.6,-0.9),0.8)] {
            let pose=Affine3A::from_rotation_translation(Quat::from_rotation_y(angle),pos);
            guns.track(0,Some(pose));
            assert!(guns.muzzle(0).unwrap().abs_diff_eq(guns.tracked_muzzle(0,pose),1e-6));
        }
        guns.track(0,None);assert!(guns.muzzle(0).is_none(),"lost tracking must not reuse a stale muzzle");
    }
}
