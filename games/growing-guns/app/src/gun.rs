//! The gun that grows, after Godot's `procedural_gun.gd`: a rifle built from
//! boxes, cylinders and rings whose every part follows the weapon's stats.
//! Accuracy stretches the barrel (and mounts a scope), damage swells the
//! receiver and fattens the barrel, rounds deepen the magazine (a drum past
//! 30), fire rate adds a vented shroud and at 5x turns it into a minigun,
//! extra projectiles add barrels and explosive rounds put a launcher tube
//! where the stock was. A card pick morphs the old gun into the new one over
//! two seconds. Firing slams the bolt back, spins minigun barrels and heats
//! the barrels until they glow.
//!
//! Godot conventions: -Z forward, +Y up, the receiver centred on the origin.
use glam::{Affine3A, EulerRot, Quat, Vec3};
use growing_guns::weapon::{BASE_FIRE_INTERVAL as FIRE_INTERVAL, BASE_MAG_SIZE, BASE_SPREAD as SPREAD};
use growing_guns::Player;
const MAG_SIZE: u32 = BASE_MAG_SIZE as u32;
use std::f32::consts::{PI, TAU};

/// The weapon stats the look reads (Godot `_lerp_weapon` fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub spread: f32,
    pub bullet_scale: f32,
    pub damage_mult: f32,
    pub fire_rate_mult: f32,
    pub mag: u32,
    pub shots: u32,
    pub explosive_radius: f32,
    pub poison: f32,
    pub cooling: f32,
}

impl Default for Stats {
    fn default() -> Self { Stats { spread: SPREAD, bullet_scale: 1.0, damage_mult: 1.0, fire_rate_mult: 1.0, mag: MAG_SIZE, shots: 1, explosive_radius: 0.0, poison: 0.0, cooling: 0.0 } }
}

impl Stats {
    pub fn of(p: &Player) -> Self {
        Self::weapon(&p.weapon)
    }

    pub fn weapon(w: &growing_guns::Weapon) -> Self {
        Stats { spread: w.spread(), bullet_scale: w.bullet_scale, damage_mult: w.damage_mult, fire_rate_mult: w.fire_rate_mult, mag: w.mag_size(), shots: w.shots_per_trigger(), explosive_radius: w.explosive_radius, poison: w.damage_over_time, cooling: if w.slow_duration > 0.0 { 1.0 - w.slow_on_hit } else { 0.0 } }
    }

    pub fn lerp(a: &Stats, b: &Stats, t: f32) -> Stats {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Stats {
            spread: l(a.spread, b.spread).max(0.0),
            bullet_scale: l(a.bullet_scale, b.bullet_scale).max(0.05),
            damage_mult: l(a.damage_mult, b.damage_mult).max(0.05),
            fire_rate_mult: l(a.fire_rate_mult, b.fire_rate_mult).max(0.05),
            mag: l(a.mag as f32, b.mag as f32).round().max(1.0) as u32,
            shots: l(a.shots as f32, b.shots as f32).round().max(1.0) as u32,
            explosive_radius: l(a.explosive_radius, b.explosive_radius).max(0.0),
            poison: l(a.poison, b.poison).max(0.0),
            cooling: l(a.cooling, b.cooling).max(0.0),
        }
    }

    pub fn fire_interval(&self) -> f32 { (FIRE_INTERVAL / self.fire_rate_mult.max(0.01)).max(self.shots as f32 / 240.0) }
    fn projectiles_per_second(&self) -> f32 { self.shots as f32 / self.fire_interval().max(0.001) }
}

/// Smootherstep: monotonic, so on/off parts cross their thresholds once.
pub fn grow_ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mat { Metal, Dark, Barrel, Polymer, Recess, Slot, Bolt, Pad, Lens, Indicator, Poison, Cooling, Glass }
pub const MATS: [Mat; 13] = [Mat::Metal, Mat::Dark, Mat::Barrel, Mat::Polymer, Mat::Recess, Mat::Slot, Mat::Bolt, Mat::Pad, Mat::Lens, Mat::Indicator, Mat::Poison, Mat::Cooling, Mat::Glass];

/// Unit shapes: a box (side 1), cylinders of height 1 along Y with radius 1
/// at the top (tapering to `bottom` below), rings around Y of radius 1.
/// Grip shapes instead use the metre-sized GRIP_SECTIONS below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape { Box, Cyl, Taper85, Taper105, Ring18, Ring25, Grip, GripPanel, Drum, Vial }
pub const SHAPES: [Shape; 10] = [Shape::Box, Shape::Cyl, Shape::Taper85, Shape::Taper105, Shape::Ring18, Shape::Ring25, Shape::Grip, Shape::GripPanel, Shape::Drum, Shape::Vial];

/// Selected once at scene creation; simulation stats and attachment IDs stay shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Detail { #[default] Full, Low }
impl Detail {
    /// Includes both outgoing and incoming parts during a morph.
    pub const fn part_capacity(self) -> usize { match self { Self::Full => 320, Self::Low => 160 } }
}

/// How a part moves: bolts slide back on a shot, minigun barrels spin,
/// the magazine drops out on reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion { Fixed, Bolt, Spin, Mag }

#[derive(Clone, Copy, Debug)]
pub struct Part { pub key: u32, pub shape: Shape, pub mat: Mat, pub xf: Affine3A, pub motion: Motion }

/// The visual parameters (Godot's exports), as `apply_weapon_stats` sets them.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    receiver: Vec3,
    barrels: u32,
    barrel_length: f32,
    barrel_radius: f32,
    wide_muzzle: bool,
    minigun: bool,
    mag: Vec3,
    drum: bool,
    scope: bool,
    stock: bool,
    foregrip: bool,
    shroud: bool,
    shroud_extent: f32,
    shroud_frac: f32,
    tube: bool,
    tube_length: f32,
    rail: f32,
    poison: f32,
    cooling: f32,
}

const MUZZLE_FLARE: f32 = 1.55;
const MUZZLE_LENGTH: f32 = 0.05;
// The lower controls share a fixed layout. Magazine growth stays in front of
// this rear face; receiver/barrel growth must never squeeze the trigger space.
const MAG_REAR_Z: f32 = -0.045;
const GRIP_Z: f32 = 0.085;
/// Grip loft in metres: (height from receiver underside, half width, front Z,
/// back Z). The narrow neck, palm swell, swept backstrap and flared heel are
/// baked into the mesh instead of tilting a cylinder through the controls.
pub const GRIP_SECTIONS: [(f32, f32, f32, f32); 7] = [
    (0.009, 0.018, -0.028, 0.028),
    (-0.014, 0.016, -0.025, 0.030),
    (-0.038, 0.019, -0.017, 0.041),
    (-0.078, 0.0215, -0.003, 0.055),
    (-0.116, 0.020, 0.008, 0.063),
    (-0.128, 0.023, 0.007, 0.061),
    (-0.135, 0.021, 0.012, 0.056),
];
const FOREGRIP_LENGTH: f32 = 0.12;
const FOREGRIP_RADIUS: f32 = 0.022;
const FOREGRIP_OFFSET_Z: f32 = 0.26;
const HANDGUARD_HEIGHT: f32 = 0.025;
const RAIL_HEIGHT: f32 = 0.02;
const SIGHT_POST: f32 = 0.04;
const REAR_SIGHT_R: f32 = 0.022;
const SCOPE_LENGTH: f32 = 0.18;
const STOCK: Vec3 = Vec3::new(0.05, 0.09, 0.2);
const STOCK_TAPER: f32 = 0.7;
const BOLT_TRAVEL_FRAC: f32 = 0.8;
const BOLT_RADIUS: f32 = 0.009;
const BOLT_LENGTH: f32 = 0.04;
const INDICATOR: Vec3 = Vec3::new(0.012, 0.012, 0.05);
const SHROUD_BPS_APPEAR: f32 = 6.0;
const SHROUD_BPS_FULL: f32 = 42.0;
// Seat attached parts slightly inside their supports, including bevels.
const JOINT: f32 = 0.003;

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

impl Look {
    /// Godot `apply_weapon_stats`.
    pub fn of(w: &Stats) -> Look {
        let spread_factor = (1.0 - w.spread / 0.05).clamp(0.0, 1.0);
        let barrel_length = lerp(0.20, 1.4, spread_factor.powi(10));
        let dmg = ((w.damage_mult - 1.0) / 4.0).clamp(0.0, 1.0);
        let barrel_radius = (0.022 * w.bullet_scale.max(1.0) * lerp(1.0, 1.7, dmg)).clamp(0.012, 0.10);
        let receiver = Vec3::new(lerp(0.055, 0.075, dmg), lerp(0.072, 0.16, dmg), lerp(0.24, 0.55, dmg));
        let drum = w.mag > 30;
        let mag = if drum {
            Vec3::new(0.07, (0.18 + 0.006 * (w.mag - 30) as f32).clamp(0.18, 0.36), lerp(0.06, 0.11, dmg))
        } else {
            Vec3::new(lerp(0.044, 0.070, dmg), (0.04 + 0.020 * w.mag as f32).clamp(0.06, 0.6), lerp(0.045, 0.090, dmg))
        };
        let minigun = w.fire_rate_mult >= 5.0;
        let bps = w.projectiles_per_second();
        let bps_factor = ((bps - SHROUD_BPS_APPEAR) / (SHROUD_BPS_FULL - SHROUD_BPS_APPEAR)).clamp(0.0, 1.0);
        let n = w.shots.max(1);
        let span = barrel_radius * 2.4 * (n - 1) as f32;
        let clearance = barrel_radius + 0.012;
        let tube = w.explosive_radius > 0.0;
        let blast = if tube { ((w.explosive_radius - 4.0) / 10.0).clamp(0.0, 1.0) } else { 0.0 };
        Look {
            receiver, barrels: n, barrel_length, barrel_radius, wide_muzzle: w.damage_mult >= 3.0, minigun, mag, drum,
            scope: w.spread < 0.003, stock: barrel_length > 0.55 && !tube, foregrip: barrel_length > 0.55,
            shroud: bps >= SHROUD_BPS_APPEAR && !minigun, shroud_extent: if n > 1 { span * 0.5 + clearance } else { clearance }, shroud_frac: lerp(0.34, 0.92, bps_factor),
            tube, tube_length: lerp(0.14, 0.46, blast),
            rail: ((w.damage_mult - 1.15) / 0.35).clamp(0.0, 1.0),
            poison: (w.poison / 0.7).clamp(0.0, 1.0), cooling: (w.cooling / 0.52).clamp(0.0, 1.0),
        }
    }

}

/// A built gun: parts in gun space, how far the bolt travels, whether the
/// barrels spin, and the barrel length (the view pulls long guns back).
#[derive(Clone, Debug, Default)]
pub struct Build { pub parts: Vec<Part>, pub bolt_travel: f32, pub minigun: bool, pub barrel_length: f32, /** Gun-space barrel exit. */ pub muzzle: Vec3 }

impl Build {
    fn same_shape(&self, other: &Self) -> bool {
        self.parts.len() == other.parts.len() && self.parts.iter().zip(&other.parts).all(|(a, b)| {
            a.key == b.key && a.shape == b.shape && a.mat == b.mat && a.motion == b.motion &&
                a.xf.to_cols_array().iter().zip(b.xf.to_cols_array()).all(|(a, b)| (*a - b).abs() < 1e-5)
        })
    }

    fn sight_anchor(&self) -> Vec3 {
        if let Some(p) = self.parts.iter().find(|p| p.key == 10 * 1024) {
            return Vec3::new(0.0, p.xf.translation.y - p.xf.matrix3.y_axis.length() * 0.5 + JOINT, p.xf.translation.z);
        }
        let radius = self.parts.iter().find(|p| p.key / 1024 == 3).map_or(0.022, |p| p.xf.matrix3.x_axis.length());
        Vec3::new(0.0, radius, -self.parts[0].xf.matrix3.z_axis.length() * 0.5 - self.barrel_length * 0.85)
    }

    fn anchor(&self, part: &Part) -> Vec3 {
        let half = self.parts[0].xf.matrix3.diagonal() * 0.5;
        let p: Vec3 = part.xf.translation.into();
        match part.key / 1024 {
            // A component shares one anchor: its teeth, mounts, bells and
            // lenses must remain joined while their whole assembly grows.
            2 | 16 => Vec3::new(0.0, half.y, 0.0),
            10 if part.key % 1024 == 0 => self.sight_anchor(),
            10 => Vec3::new(0.0, half.y, half.z * 0.64),
            3 | 4 | 6 | 14 | 15 => Vec3::new(0.0, 0.0, -half.z),
            5 => Vec3::new(0.0, -half.y, MAG_REAR_Z),
            8 => Vec3::new(0.0, -half.y, half.z),
            9 => Vec3::new(0.0, 0.0, half.z),
            12 | 13 => Vec3::new(-half.x + 0.003, half.y - 0.003, if part.key / 1024 == 12 { 0.060 } else { -0.060 }),
            _ => p,
        }
    }
}

#[derive(Clone, Debug)]
struct Morph {
    from: Stats, to: Stats, elapsed: f32,
    source: Build, target: Build,
    links: Vec<(Option<usize>, Option<usize>)>,
}

impl Morph {
    fn at(&self, t: f32) -> Build {
        let mut result = Build { parts: Vec::with_capacity(self.links.len()),
            bolt_travel: lerp(self.source.bolt_travel, self.target.bolt_travel, t),
            minigun: self.target.minigun, barrel_length: lerp(self.source.barrel_length, self.target.barrel_length, t),
            muzzle: self.source.muzzle.lerp(self.target.muzzle, t) };
        let scaled = |build: &Build, index: usize, amount: f32| {
            let mut p = build.parts[index];
            // The existing front sight keeps its full size. Its new bearing
            // collar/bridge grows in thickness on the current barrel section,
            // rather than approaching from the receiver and leaving it floating.
            if matches!(p.key / 1024, 14 | 15) {
                let support = self.source.sight_anchor().lerp(self.target.sight_anchor(), t);
                let (size, rotation, _) = p.xf.to_scale_rotation_translation();
                let (size, position) = if p.key / 1024 == 14 {
                    let front = lerp(-self.source.parts[0].xf.matrix3.z_axis.length() * 0.5,
                        -self.target.parts[0].xf.matrix3.z_axis.length() * 0.5, t);
                    (Vec3::new(support.y, size.y * amount.max(0.0001), support.y),
                        Vec3::new(0.0, 0.0, if p.key % 1024 == 0 { front - 0.007 } else { support.z }))
                } else {
                    let width = |b: &Build| b.parts.iter().find(|q| q.key == p.key)
                        .map_or(b.sight_anchor().y * 2.0, |q| q.xf.matrix3.x_axis.length());
                    let size = Vec3::new(lerp(width(&self.source), width(&self.target), t),
                        size.y * amount.max(0.0001), size.z * amount.max(0.0001));
                    (size, support - Vec3::Y * (size.y * 0.5))
                };
                p.xf = Affine3A::from_scale_rotation_translation(size, rotation, position);
                return p;
            }
            let anchor = build.anchor(&p);
            let moving_anchor = self.source.anchor(&p).lerp(self.target.anchor(&p), t);
            let amount = amount.max(0.0001);
            p.xf = Affine3A::from_translation(moving_anchor) * Affine3A::from_scale(Vec3::splat(amount)) * Affine3A::from_translation(-anchor) * p.xf;
            p
        };
        for &(a, b) in &self.links {
            let p = match (a, b) {
                (Some(i), Some(j)) => {
                    let mut p = self.target.parts[j];
                    let (sa, ra, pa) = self.source.parts[i].xf.to_scale_rotation_translation();
                    let (sb, rb, pb) = p.xf.to_scale_rotation_translation();
                    p.xf = Affine3A::from_scale_rotation_translation(sa.lerp(sb, t), ra.slerp(rb, t), pa.lerp(pb, t));
                    p
                }
                (Some(i), None) if t < 1.0 => scaled(&self.source, i, 1.0 - t),
                (None, Some(j)) if t > 0.0 => scaled(&self.target, j, t),
                _ => continue,
            };
            result.parts.push(p);
        }
        result
    }
}

struct B { parts: Vec<Part>, group: u32, next: u32 }
impl B {
    fn group(&mut self, group: u32) { self.group = group; self.next = 0; }
    fn put(&mut self, shape: Shape, mat: Mat, size: Vec3, pos: Vec3, rot: Quat, motion: Motion) {
        self.parts.push(Part { key: self.group * 1024 + self.next, shape, mat, xf: Affine3A::from_scale_rotation_translation(size, rot, pos), motion });
        self.next += 1;
    }
    fn bx(&mut self, size: Vec3, pos: Vec3, mat: Mat) { self.put(Shape::Box, mat, size, pos, Quat::IDENTITY, Motion::Fixed); }
    /// Godot `_add_cylinder`: laid along Z (rotated +90 degrees about X).
    fn cyl(&mut self, shape: Shape, r: f32, h: f32, pos: Vec3, mat: Mat, motion: Motion) {
        self.put(shape, mat, Vec3::new(r, h, r), pos, Quat::from_rotation_x(PI * 0.5), motion);
    }
}

/// Godot euler angles (YXZ order).
fn euler(x: f32, y: f32, z: f32) -> Quat { Quat::from_euler(EulerRot::YXZ, y, x, z) }

/// Godot `_rebuild_impl`.
pub fn build(l: &Look) -> Build {
    use Mat::*;
    let mut b = B { parts: Vec::with_capacity(128), group: 0, next: 0 };
    let n = l.barrels.max(1);
    let pitch = l.barrel_radius * 2.4;
    let span = pitch * (n - 1) as f32;
    let muzzle_r = l.barrel_radius * MUZZLE_FLARE;
    let bundle = l.barrel_radius * 1.8;
    let min_w = if l.minigun { (bundle + l.barrel_radius) * 2.2 } else { span + l.barrel_radius * 2.0 + 0.010 };
    let rs = Vec3::new(l.receiver.x.max(min_w), l.receiver.y, l.receiver.z);
    b.bx(rs, Vec3::ZERO, Metal);
    b.bx(Vec3::new(rs.x * 1.04, rs.y * 0.34, rs.z * 0.92), Vec3::new(0.0, -rs.y * 0.34, rs.z * 0.02), Dark);
    b.bx(Vec3::new(0.004, rs.y * 0.26, rs.z * 0.3), Vec3::new(rs.x * 0.5 + 0.001, rs.y * 0.14, -rs.z * 0.12), Recess);
    b.group(1);
    // Charging handles in their slots, both sides.
    let travel = rs.z * BOLT_TRAVEL_FRAC.clamp(0.0, 0.95);
    let rest_z = -travel * 0.5;
    for side in [-1.0f32, 1.0] {
        b.bx(Vec3::new(0.012, BOLT_RADIUS * 2.4, travel + BOLT_RADIUS * 2.0), Vec3::new(side * (rs.x * 0.5 + 0.006), 0.0, 0.0), Slot);
        b.put(Shape::Cyl, Bolt, Vec3::new(BOLT_RADIUS, BOLT_LENGTH, BOLT_RADIUS), Vec3::new(side * (rs.x * 0.5 + 0.012 + BOLT_LENGTH * 0.5), 0.0, rest_z), euler(PI * 0.5, PI * 0.5, 0.0), Motion::Bolt);
    }
    let front = -rs.z * 0.5;
    let barrel_z = front - l.barrel_length * 0.5;
    let muzzle_z = front - l.barrel_length - MUZZLE_LENGTH * 0.5;
    b.group(2);
    // Picatinny rail with its teeth.
    let rail_h = RAIL_HEIGHT * l.rail;
    let rail_y = rs.y * 0.5 + rail_h * 0.5;
    let rail = Vec3::new((rs.x * 0.8).min(0.04), rail_h, rs.z * 0.85);
    let base_h = rail.y * 0.5;
    if l.rail > 0.001 {
    b.bx(Vec3::new(rail.x, base_h + JOINT * l.rail, rail.z), Vec3::new(0.0, rail_y - rail.y * 0.5 + (base_h - JOINT * l.rail) * 0.5, 0.0), Dark);
    let period = 0.022;
    let teeth = ((rail.z / period) as u32).max(1);
    for i in 0..teeth {
        let z = -((teeth - 1) as f32 * period) * 0.5 + i as f32 * period;
        b.bx(Vec3::new(rail.x * 1.2, base_h + JOINT * l.rail, 0.012), Vec3::new(0.0, rail_y + rail.y * 0.5 - (base_h + JOINT * l.rail) * 0.5, z), Dark);
    }
    }
    b.group(3);
    if l.minigun {
        for i in 0..6 {
            let a = i as f32 * TAU / 6.0;
            let (x, y) = (a.cos() * bundle, a.sin() * bundle);
            b.cyl(Shape::Cyl, l.barrel_radius, l.barrel_length, Vec3::new(x, y, barrel_z), Barrel, Motion::Spin);
            b.cyl(Shape::Taper85, muzzle_r, MUZZLE_LENGTH, Vec3::new(x, y, muzzle_z), Barrel, Motion::Spin);
        }
    } else {
        for i in 0..n {
            let x = (i as f32 - (n - 1) as f32 * 0.5) * pitch;
            b.cyl(Shape::Cyl, l.barrel_radius, l.barrel_length, Vec3::new(x, 0.0, barrel_z), Barrel, Motion::Fixed);
            if l.wide_muzzle {
                b.bx(Vec3::new(l.barrel_radius * 4.5, l.barrel_radius * 1.6, MUZZLE_LENGTH), Vec3::new(x, 0.0, muzzle_z), Barrel);
            } else {
                b.cyl(Shape::Taper85, muzzle_r, MUZZLE_LENGTH, Vec3::new(x, 0.0, muzzle_z), Barrel, Motion::Fixed);
            }
        }
    }
    b.group(4);
    if l.shroud {
        let len = l.barrel_length * l.shroud_frac.clamp(0.05, 0.95);
        shroud(&mut b, front - len * 0.5, len, l.shroud_extent.max(l.barrel_radius + 0.012), l.barrel_radius + 0.012, 5);
    }
    b.group(5);
    if l.drum {
        let r = l.mag.y * 0.5;
        let mag_z = MAG_REAR_Z - l.mag.x * 0.5;
        // A circular drum otherwise meets the receiver at a single tangent.
        b.put(Shape::Box, Dark, Vec3::new(0.045, 0.04, 0.045), Vec3::new(0.0, -rs.y * 0.5 - 0.012, mag_z), Quat::IDENTITY, Motion::Mag);
        b.put(Shape::Drum, Dark, Vec3::new(r, l.mag.x, r), Vec3::new(0.0, -rs.y * 0.5 - r, mag_z), Quat::from_rotation_x(PI * 0.5), Motion::Mag);
    } else {
        let y = -rs.y * 0.5 - l.mag.y * 0.5 + JOINT;
        let mag_z = MAG_REAR_Z - l.mag.z * 0.5;
        b.put(Shape::Box, Polymer, l.mag, Vec3::new(0.0, y, mag_z), Quat::IDENTITY, Motion::Mag);
        b.put(Shape::Box, Dark, Vec3::new(l.mag.x * 1.1, 0.012, l.mag.z * 1.15), Vec3::new(0.0, y - l.mag.y * 0.5, mag_z), Quat::IDENTITY, Motion::Mag);
    }
    b.group(6);
    if l.foregrip {
        let gz = front - FOREGRIP_OFFSET_Z;
        let hg_y = -l.barrel_radius - HANDGUARD_HEIGHT * 0.5 + JOINT;
        b.bx(Vec3::new(l.barrel_radius * 2.4, HANDGUARD_HEIGHT, front - gz), Vec3::new(0.0, hg_y, (front + gz) * 0.5), Polymer);
        let tilt = -8f32.to_radians();
        let rot = Quat::from_rotation_x(tilt);
        let anchor = Vec3::new(0.0, hg_y - HANDGUARD_HEIGHT * 0.5 + JOINT + FOREGRIP_RADIUS * tilt.sin().abs(), gz + FOREGRIP_RADIUS * 1.1);
        b.put(Shape::Taper85, Polymer, Vec3::new(FOREGRIP_RADIUS, FOREGRIP_LENGTH, FOREGRIP_RADIUS), anchor - rot * Vec3::Y * (FOREGRIP_LENGTH * 0.5), rot, Motion::Fixed);
    }
    b.group(7);
    controls(&mut b, -rs.y * 0.5);
    b.group(8);
    if l.stock {
        let (back_h, front_h, half_z) = (STOCK.y, STOCK.y * STOCK_TAPER, STOCK.z * 0.5);
        let bottom = -rs.y * 0.5;
        let back_z = rs.z * 0.5 + half_z * 1.5;
        b.bx(Vec3::new(STOCK.x, back_h, half_z + JOINT), Vec3::new(0.0, bottom + back_h * 0.5, back_z - JOINT * 0.5), Polymer);
        b.bx(Vec3::new(STOCK.x * 0.95, front_h, half_z + JOINT), Vec3::new(0.0, bottom + front_h * 0.5, rs.z * 0.5 + (half_z - JOINT) * 0.5), Polymer);
        b.bx(Vec3::new(STOCK.x * 1.04, back_h * 1.04, 0.012), Vec3::new(0.0, bottom + back_h * 0.5, back_z + half_z * 0.5 + 0.006 - JOINT), Pad);
    }
    b.group(9);
    if l.tube {
        let r = rs.y * 0.5;
        b.cyl(Shape::Cyl, r, l.tube_length, Vec3::new(0.0, 0.0, rs.z * 0.5 + l.tube_length * 0.5 - JOINT), Dark, Motion::Fixed);
    }
    b.group(10);
    if !l.scope {
        let sight_z = front - l.barrel_length * 0.85;
        let sight_base = if l.minigun {
            let r = bundle + l.barrel_radius + 0.004;
            b.group(14);
            for z in [front - 0.007, sight_z] { b.cyl(Shape::Cyl, r, 0.02, Vec3::new(0.0, 0.0, z), Dark, Motion::Fixed); }
            b.group(10);
            r
        } else if n > 1 {
            b.group(15);
            // Even barrel counts have no central barrel beneath the sight.
            b.bx(Vec3::new(span + l.barrel_radius * 2.0, 0.008, 0.018), Vec3::new(0.0, l.barrel_radius - 0.001, sight_z), Dark);
            b.group(10);
            l.barrel_radius + 0.003
        } else { l.barrel_radius };
        b.bx(Vec3::new(0.005, SIGHT_POST, 0.012), Vec3::new(0.0, sight_base + SIGHT_POST * 0.5 - JOINT, sight_z), Dark);
        b.put(Shape::Ring25, Dark, Vec3::splat(REAR_SIGHT_R * 0.875), Vec3::new(0.0, rail_y + rail_h * 0.5 + REAR_SIGHT_R * 0.4, rs.z * 0.32), Quat::from_rotation_x(PI * 0.5), Motion::Fixed);
    } else {
        b.group(16);
        let (tube_r, bell_r, bell_len) = (0.018, 0.028, 0.04);
        let top = rail_y + rail_h * 0.5;
        let sy = top + 0.025;
        let mount_bottom = top - base_h - JOINT;
        for rz in [-SCOPE_LENGTH * 0.3, SCOPE_LENGTH * 0.3] { b.bx(Vec3::new(tube_r * 2.2, sy - mount_bottom, 0.02), Vec3::new(0.0, (mount_bottom + sy) * 0.5, rz), Dark); }
        b.cyl(Shape::Cyl, tube_r, SCOPE_LENGTH, Vec3::new(0.0, sy, 0.0), Dark, Motion::Fixed);
        b.cyl(Shape::Cyl, bell_r, bell_len, Vec3::new(0.0, sy, -SCOPE_LENGTH * 0.5 - bell_len * 0.5), Dark, Motion::Fixed);
        b.cyl(Shape::Cyl, bell_r, bell_len, Vec3::new(0.0, sy, SCOPE_LENGTH * 0.5 + bell_len * 0.5), Dark, Motion::Fixed);
        b.cyl(Shape::Cyl, bell_r * 0.85, 0.005, Vec3::new(0.0, sy, -SCOPE_LENGTH * 0.5 - bell_len + 0.002), Lens, Motion::Fixed);
    }
    b.group(11);
    b.bx(INDICATOR, Vec3::new(rs.x * 0.52 + INDICATOR.x * 0.5 - 0.001, -rs.y * 0.34, 0.0), Indicator);
    for (amount, z, mat) in [(l.poison, 0.060, Poison), (l.cooling, -0.060, Cooling)] {
        b.group(if mat == Poison { 12 } else { 13 });
        if amount > 0.001 { reservoir(&mut b, rs, z, amount, mat); }
    }
    Build { parts: b.parts, bolt_travel: travel, minigun: l.minigun, barrel_length: l.barrel_length, muzzle: Vec3::new(0.0, 0.0, muzzle_z - MUZZLE_LENGTH * 0.5) }
}

/// Low hardware keeps gameplay silhouettes and mounting points, while omitting
/// tiny fasteners, rail teeth, vent ribs and the transparent reservoir shell.
/// Simplify before comparing/morphing so omitted details cannot trigger inspect.
pub fn build_with_detail(l: &Look, detail: Detail) -> Build {
    let mut built = build(l);
    if detail == Detail::Full { return built; }
    let front = -built.parts[0].xf.matrix3.z_axis.length() * 0.5;
    built.parts.retain_mut(|p| {
        let (group, n) = (p.key / 1024, p.key % 1024);
        match group {
            0 if n == 2 => return false, // Tiny ejection recess.
            1 if n % 2 == 0 => return false, // Bolt slots; moving handles stay.
            2 => {
                if n != 0 { return false; }
                // One solid rail reaches the same top as the original teeth.
                let extra = RAIL_HEIGHT * l.rail * 0.5;
                p.xf.matrix3.y_axis *= (p.xf.matrix3.y_axis.length() + extra) / p.xf.matrix3.y_axis.length();
                p.xf.translation.y += extra * 0.5;
            }
            4 => {
                if n % 8 != 0 { return false; }
                // Four continuous shroud plates instead of 32 caps/rails/ribs.
                let length = l.barrel_length * l.shroud_frac.clamp(0.05, 0.95);
                p.xf.matrix3.z_axis = Vec3::Z.into();
                p.xf.matrix3.z_axis *= length;
                p.xf.translation.z = front - length * 0.5;
            }
            7 if (2..6).contains(&n) => return false, // Grip screws only; keep trigger/guard/panels.
            12 | 13 => {
                if !matches!(n, 0 | 1 | 2 | 4) { return false; }
                if n == 2 {
                    // The opaque coloured core fills the former glass envelope.
                    p.xf.matrix3.x_axis *= 0.014 / 0.0115;
                    p.xf.matrix3.z_axis *= 0.014 / 0.0115;
                    p.xf.matrix3.y_axis *= 0.060 / 0.056;
                }
            }
            _ => {}
        }
        true
    });
    built
}

/// A coloured, glass-covered cartridge leans out from the upper camera-facing
/// edge, leaning back along the receiver. Separate mounts keep both upgrades visible when combined.
fn reservoir(b: &mut B, receiver: Vec3, z: f32, growth: f32, core: Mat) {
    let growth = growth * 0.72;
    let axis = Vec3::new(-0.28, 0.48, 0.83).normalize();
    let rot = Quat::from_rotation_arc(Vec3::Y, axis);
    let root = Vec3::new(-receiver.x * 0.5 + 0.003, receiver.y * 0.5 - 0.003, z);
    let mut cyl = |r: f32, h: f32, y: f32, mat: Mat| {
        b.put(Shape::Vial, mat, Vec3::new(r, h, r) * growth, root + axis * (y * growth), rot, Motion::Fixed);
    };
    cyl(0.020, 0.020, 0.007, Mat::Dark);
    cyl(0.017, 0.014, 0.023, Mat::Metal);
    cyl(0.0115, 0.056, 0.055, core);
    cyl(0.014, 0.060, 0.055, Mat::Glass);
    cyl(0.018, 0.012, 0.086, Mat::Metal);
    cyl(0.009, 0.003, 0.093, core);
    for x in [-0.014, 0.014] {
        b.put(Shape::Cyl, Mat::Dark, Vec3::new(0.0015, 0.062, 0.0015) * growth,
            root + rot * Vec3::new(x, 0.055, 0.0) * growth, rot, Motion::Fixed);
    }
}

/// Fixed ergonomic assembly below the growing receiver. Z distances and the
/// finger opening remain unchanged throughout upgrade morphs and reloads.
fn controls(b: &mut B, bottom: f32) {
    use Mat::*;
    let origin = Vec3::new(0.0, bottom, GRIP_Z);
    b.put(Shape::Grip, Polymer, Vec3::ONE, origin, Quat::IDENTITY, Motion::Fixed);
    b.put(Shape::GripPanel, Pad, Vec3::ONE, origin, Quat::IDENTITY, Motion::Fixed);
    for side in [-1.0, 1.0] {
        b.put(Shape::Cyl, Bolt, Vec3::new(0.0035, 0.003, 0.0035),
            Vec3::new(side * 0.0215, bottom - 0.060, GRIP_Z + 0.020), Quat::from_rotation_z(PI * 0.5), Motion::Fixed);
        b.bx(Vec3::new(0.0008, 0.004, 0.001), Vec3::new(side * 0.023, bottom - 0.060, GRIP_Z + 0.020), Recess);
    }
    let mut stroke = |points: &[(f32, f32)], width: f32, thick: f32, mat: Mat| {
        for pair in points.windows(2) {
            let a = Vec3::new(0.0, bottom + pair[0].0, pair[0].1);
            let c = Vec3::new(0.0, bottom + pair[1].0, pair[1].1);
            let d = c - a;
            b.put(Shape::Box, mat, Vec3::new(width, d.length() + JOINT, thick), (a + c) * 0.5,
                Quat::from_rotation_arc(Vec3::Y, d.normalize()), Motion::Fixed);
        }
    };
    stroke(&[(0.004, -0.027), (-0.035, -0.027), (-0.050, -0.016),
        (-0.050, 0.048), (-0.025, 0.066), (0.004, 0.066)], 0.012, 0.006, Dark);
    // Curved steel trigger, visibly separated from the front wall and floor
    // of the guard. The root is embedded in the receiver.
    stroke(&[(0.004, 0.030), (-0.014, 0.030), (-0.029, 0.024), (-0.035, 0.014)], 0.009, 0.0045, Bolt);
}

/// Godot `_add_barrel_shroud`: four slotted plates around the barrels.
fn shroud(b: &mut B, centre_z: f32, length: f32, half_x: f32, half_y: f32, slots: u32) {
    let (plate, rail_w, rib_z, cap) = (0.006, 0.022, 0.025, 0.024);
    let inner = (length - 2.0 * cap).max(0.001);
    let ribs = slots.saturating_sub(1);
    let slot_len = ((inner - ribs as f32 * rib_z) / slots as f32).max(0.001);
    let (z_back, z_front) = (centre_z + length * 0.5, centre_z - length * 0.5);
    for side in 0..4 {
        let radial = if side % 2 == 0 { half_y } else { half_x };
        let tangential = if side % 2 == 0 { half_x } else { half_y };
        let py = radial - plate * 0.5;
        let width = 2.0 * tangential;
        let rot = Quat::from_rotation_z(side as f32 * PI * 0.5);
        let mut put = |size: Vec3, pos: Vec3| b.put(Shape::Box, Mat::Dark, size, rot * pos, rot, Motion::Fixed);
        put(Vec3::new(width, plate, cap), Vec3::new(0.0, py, z_front + cap * 0.5));
        put(Vec3::new(width, plate, cap), Vec3::new(0.0, py, z_back - cap * 0.5));
        let rx = tangential - rail_w * 0.5;
        put(Vec3::new(rail_w, plate, inner), Vec3::new(-rx, py, centre_z));
        put(Vec3::new(rail_w, plate, inner), Vec3::new(rx, py, centre_z));
        for i in 0..ribs {
            let z = z_front + cap + slot_len + i as f32 * (slot_len + rib_z) + rib_z * 0.5;
            put(Vec3::new((width - 2.0 * rail_w).max(0.001), plate, rib_z), Vec3::new(0.0, py, z));
        }
    }
}

/// One gun in the world: its build, the morph in flight and the live state
/// (heat, bolt, spin) that firing drives.
#[derive(Clone, Debug)]
pub struct Rig {
    detail: Detail,
    pub shown: Stats,
    pub build: Build,
    grow: Option<Morph>,
    pub heat: f32,
    bolt_back: f32,
    bolt_cycle: f32,
    spin_speed: f32,
    spin: f32,
    previous_pose: Vec<Affine3A>,
}

impl Default for Rig {
    fn default() -> Self { Self::with_detail(Detail::Full) }
}

pub const GROW_SECONDS: f32 = 2.0;

impl Rig {
    pub fn with_detail(detail: Detail) -> Self {
        let s = Stats::default();
        Rig { detail, shown: s, build: build_with_detail(&Look::of(&s), detail), grow: None,
            heat: 0.0, bolt_back: 0.0, bolt_cycle: 0.08, spin_speed: 0.0, spin: 0.0, previous_pose: Vec::new() }
    }
    /// Snap to these stats (a new round, a respawn).
    pub fn set(&mut self, s: Stats) {
        let interrupted = self.grow.take().is_some();
        self.previous_pose.clear();
        if interrupted || s != self.shown { self.shown = s; self.build = build_with_detail(&Look::of(&s), self.detail); }
    }

    /// Morph from the currently visible model, including an interrupted morph.
    pub fn grow_to(&mut self, to: Stats) {
        if self.grow.as_ref().is_some_and(|g| g.to == to) { return; }
        let target = build_with_detail(&Look::of(&to), self.detail);
        if let Some(g) = &mut self.grow {
            if g.target.same_shape(&target) { g.to = to; return; }
        }
        if self.build.same_shape(&target) { self.set(to); return; }
        let source = self.build.clone();
        let mut links = Vec::new();
        let mut used = vec![false; target.parts.len()];
        for (i, p) in source.parts.iter().enumerate() {
            let found = target.parts.iter().enumerate().find(|(j, q)| !used[*j] && p.key == q.key && p.shape == q.shape && p.mat == q.mat).map(|(j, _)| j);
            if let Some(j) = found { used[j] = true; }
            links.push((Some(i), found));
        }
        links.extend(used.iter().enumerate().filter(|(_, used)| !**used).map(|(j, _)| (None, Some(j))));
        self.grow = Some(Morph { from: self.shown, to, elapsed: 0.0, source, target, links });
    }

    pub fn is_growing(&self) -> bool { self.grow.is_some() }

    /// Keep the target current: a morph retargets, otherwise a change snaps.
    pub fn follow(&mut self, target: Stats) {
        if let Some(g) = &self.grow {
            if g.to != target { self.grow_to(target); }
        } else { self.set(target); }
    }

    /// A shot: bolt to the rear stop, barrels spin up, heat climbs.
    pub fn fire(&mut self, stats: &Stats) {
        self.bolt_back = self.build.bolt_travel;
        self.bolt_cycle = stats.fire_interval();
        if self.build.minigun { self.spin_speed = (self.spin_speed + 12.0).min(45.0); }
        self.heat = (self.heat + stats.damage_mult * stats.shots as f32 * 0.01).clamp(0.0, 1.5);
    }

    pub fn update(&mut self, dt: f32) {
        let previous = self.grow.as_ref().map(|_| self.build.parts.iter().map(|p| ((p.key, p.shape, p.mat), p.xf)).collect::<std::collections::HashMap<_, _>>());
        if let Some(g) = &mut self.grow {
            g.elapsed += dt.min(0.05);
            let t = grow_ease(g.elapsed / GROW_SECONDS);
            self.shown = Stats::lerp(&g.from, &g.to, t);
            self.build = g.at(t);
            if g.elapsed >= GROW_SECONDS { self.shown = g.to; self.build = g.target.clone(); self.grow = None; }
        }
        self.previous_pose.clear();
        if let Some(previous) = previous {
            self.previous_pose.extend(self.build.parts.iter().map(|p| previous.get(&(p.key, p.shape, p.mat)).copied().unwrap_or(p.xf)));
        }
        self.heat *= (-0.3 * dt).exp();
        if self.heat < 0.001 { self.heat = 0.0; }
        if self.bolt_back > 0.0 { self.bolt_back = (self.bolt_back - self.build.bolt_travel / self.bolt_cycle.max(0.001) * dt).max(0.0); }
        self.spin += self.spin_speed * dt;
        self.spin_speed *= (-2.5 * dt).exp();
    }

    /// Barrel glow 0 (cold steel) to 1, continuous.
    pub fn heat_glow_t(&self) -> f32 { smoothstep(0.15, 1.0, self.heat) }

    /// Every part's gun-space transform this frame. `mag_drop` lowers the
    /// magazine (reload).
    pub fn pose(&self, mag_drop: f32) -> impl Iterator<Item = (&Part, Affine3A)> + '_ {
        self.pose_at(mag_drop, 1.0)
    }

    /// Blend the two simulation poses for screens faster than the game tick.
    pub fn pose_at(&self, mag_drop: f32, alpha: f32) -> impl Iterator<Item = (&Part, Affine3A)> + '_ {
        let spin = Affine3A::from_rotation_z(self.spin);
        self.build.parts.iter().enumerate().map(move |(i, p)| {
            let local = if let Some(previous) = self.previous_pose.get(i) {
                let (a, r, t) = previous.to_scale_rotation_translation();
                let (b, s, u) = p.xf.to_scale_rotation_translation();
                Affine3A::from_scale_rotation_translation(a.lerp(b, alpha), r.slerp(s, alpha), t.lerp(u, alpha))
            } else { p.xf };
            let xf = match p.motion {
                Motion::Fixed => local,
                Motion::Bolt => Affine3A::from_translation(Vec3::Z * self.bolt_back) * local,
                Motion::Spin => spin * local,
                Motion::Mag => Affine3A::from_translation(Vec3::NEG_Y * mag_drop) * local,
            };
            (p, xf)
        })
    }
}

pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 { let t = ((x - a) / (b - a)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// Godot's blackbody-ish heat colour and emission energy at glow `t` (0..1).
pub fn heat_glow(t: f32) -> ([f32; 3], f32) {
    ([1.0, 0.7 * t.powf(1.4), 0.3 * t.powi(3)], 6.0 * t)
}

/// The first-person reload: the gun rises and tilts, sways right and back,
/// lowers; the magazine drops out and slides back in (Godot `_animate_reload`).
/// `u` runs 0..1 over the reload. Returns (offset, tilt about X, mag drop).
pub fn reload_pose(u: f32) -> (Vec3, f32, f32) {
    let seg = |a: f32, d: f32| ((u - a) / d).clamp(0.0, 1.0);
    let cubic_out = |t: f32| 1.0 - (1.0 - t).powi(3);
    let cubic_in = |t: f32| t * t * t;
    let sine = |t: f32| 0.5 - 0.5 * (PI * t).cos();
    let up = Vec3::new(0.0, 0.14, -0.02);
    let right = Vec3::new(0.06, 0.0, 0.0);
    let rise = cubic_out(seg(0.0, 0.18));
    let lower = cubic_in(seg(0.70, 0.22));
    let sway = sine(seg(0.28, 0.17)) - sine(seg(0.45, 0.17));
    let off = up * rise * (1.0 - lower) + right * sway;
    let tilt = 22f32.to_radians() * rise * (1.0 - lower);
    let back_out = |t: f32| { let c = 1.70158; 1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2) };
    let drop = 1.2 * (cubic_in(seg(0.12, 0.16)) - back_out(seg(0.55, 0.15)) * f32::from(u >= 0.55));
    (off, tilt, drop.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_visible_changes_trigger_growth_and_retargeting_keeps_the_current_pose() {
        let mut rig = Rig::default();
        rig.grow_to(Stats { fire_rate_mult: 1.1, ..Default::default() });
        assert!(!rig.is_growing(), "a fire-rate change below the shroud threshold has no model change");
        let to = Stats { damage_mult: 5.0, mag: 45, poison: 0.7, ..Default::default() };
        rig.grow_to(to);
        for _ in 0..22 { rig.update(1.0 / 30.0); }
        let before = rig.build.clone();
        rig.grow_to(Stats { spread: 0.0, cooling: 0.52, ..to });
        rig.update(0.0);
        assert!(rig.build.same_shape(&before), "retargeting must not jump back to the original weapon");
        for _ in 0..61 { rig.update(1.0 / 30.0); assert!(rig.build.parts.len() <= 320); }
        assert!(!rig.is_growing());
    }

    #[test]
    fn base_is_slim_without_a_rail_and_both_elemental_reservoirs_are_visible() {
        let base = Look::of(&Stats::default());
        assert!(base.receiver.x < 0.06 && base.receiver.y < 0.08 && base.mag.x < 0.05);
        assert!(!build(&base).parts.iter().any(|p| p.key / 1024 == 2));
        let upgraded = build(&Look::of(&Stats { damage_mult: 1.5, poison: 0.7, cooling: 0.52, ..Default::default() }));
        assert!(upgraded.parts.iter().any(|p| p.key / 1024 == 2));
        for mat in [Mat::Poison, Mat::Cooling] { assert!(upgraded.parts.iter().any(|p| p.mat == mat)); }
    }

    #[test]
    fn grips_and_stock_remain_seated_across_receiver_sizes() {
        for damage_mult in [1.0, 2.5, 5.0] {
            let look = Look::of(&Stats { spread: 0.0, damage_mult, ..Default::default() });
            let built = build(&look);
            let grip = built.parts.iter().find(|p| p.shape == Shape::Grip).unwrap();
            let top = grip.xf.transform_point3(Vec3::new(0.0, GRIP_SECTIONS[0].0, 0.0));
            assert!(top.y > -look.receiver.y * 0.5, "upper grip cap must be embedded");
            let pad = built.parts.iter().find(|p| p.mat == Mat::Pad && p.shape == Shape::Box).unwrap();
            let pad_front = pad.xf.transform_point3(Vec3::new(0.0, 0.0, -0.5)).z;
            let stock_back = look.receiver.z * 0.5 + STOCK.z;
            assert!(pad_front < stock_back && stock_back - pad_front < 0.01, "butt pad must sit on the stock");
        }
    }

    #[test]
    fn controls_keep_their_spacing_and_magazines_leave_finger_clearance() {
        let start = Stats { spread: 0.1, ..Default::default() };
        let end = Stats { spread: 0.0, damage_mult: 5.0, fire_rate_mult: 6.0, mag: 100, shots: 8, ..Default::default() };
        let mut relative_trigger = None;
        // Includes short receivers, the box-to-drum transition, large upgrades
        // and intermediate shapes during the two-second morph.
        for i in 0..=100 {
            let look = Look::of(&Stats::lerp(&start, &end, i as f32 / 100.0));
            let built = build(&look);
            let grip = built.parts.iter().find(|p| p.shape == Shape::Grip).unwrap();
            let trigger: Vec<_> = built.parts.iter().filter(|p| p.mat == Mat::Bolt && p.motion == Motion::Fixed && p.xf.translation.x.abs() < 0.001).collect();
            assert_eq!(trigger.len(), 3, "a visible trigger must be present");
            let offset = trigger[0].xf.translation - grip.xf.translation;
            let expected = relative_trigger.get_or_insert(offset);
            assert!((offset - *expected).length() < 1e-6, "trigger/grip spacing changed during growth");
            assert_eq!(grip.xf.translation.z, GRIP_Z);
            for part in built.parts.iter().filter(|p| p.motion == Motion::Mag) {
                let half = if part.shape == Shape::Box { Vec3::splat(0.5) } else { Vec3::new(1.0, 0.5, 1.0) };
                for x in [-1.0, 1.0] { for y in [-1.0, 1.0] { for z in [-1.0, 1.0] {
                    let rear = part.xf.transform_point3(half * Vec3::new(x, y, z)).z;
                    assert!(rear < -0.035, "magazine or base plate entered the trigger-guard clearance");
                } } }
            }
            let mag = built.parts.iter().find(|p| p.motion == Motion::Mag && (p.mat == Mat::Polymer || p.shape == Shape::Drum)).unwrap();
            let rear = if mag.shape == Shape::Box { mag.xf.transform_point3(Vec3::Z * 0.5).z }
                else { mag.xf.transform_point3(Vec3::Y * 0.5).z };
            assert!((rear - MAG_REAR_Z).abs() < 1e-6, "rear mounting face must remain fixed");
        }
    }

    #[test]
    fn cards_grow_the_gun() {
        let base = Look::of(&Stats::default());
        assert!((base.barrel_length - 0.42).abs() < 0.02 && !base.stock && !base.scope && !base.minigun && !base.drum);
        let sniper = Look::of(&Stats { spread: 0.0, ..Default::default() });
        assert!(sniper.barrel_length > 1.3 && sniper.scope && sniper.stock);
        let big = Look::of(&Stats { damage_mult: 5.0, mag: 40, ..Default::default() });
        assert!(big.wide_muzzle && big.drum && big.receiver.z > 0.5);
        let mini = build(&Look::of(&Stats { fire_rate_mult: 6.0, ..Default::default() }));
        assert!(mini.minigun && mini.parts.iter().filter(|p| p.motion == Motion::Spin).count() == 12);
        let boom = build(&Look::of(&Stats { explosive_radius: 8.0, ..Default::default() }));
        assert!(boom.parts.len() > build(&base).parts.len());
        assert!(build(&Look::of(&Stats { spread: 0.0, damage_mult: 5.0, fire_rate_mult: 3.0, shots: 16, mag: 60, ..Default::default() })).parts.len() < 160);
    }

    #[test]
    fn growth_morphs_then_lands_and_firing_heats_and_cycles() {
        let mut r = Rig::default();
        let to = Stats { spread: 0.0, ..Default::default() };
        r.grow_to(to);
        for _ in 0..20 { r.update(0.05); }
        let mid = r.build.barrel_length;
        assert!(r.is_growing() && mid > 0.5 && mid < 1.3, "{mid}");
        for _ in 0..30 { r.update(0.05); }
        assert!(!r.is_growing() && r.shown == to && r.build.barrel_length > 1.3);
        for _ in 0..200 { r.fire(&Stats { damage_mult: 3.0, ..to }); }
        assert_eq!(r.heat_glow_t(), 1.0);
        assert!(r.bolt_back > 0.0);
        r.update(0.5);
        assert_eq!(r.bolt_back, 0.0);
        let (_, _, drop) = reload_pose(0.4);
        assert!(drop > 1.0 && reload_pose(1.0).2 == 0.0 && reload_pose(0.0).0 == Vec3::ZERO);
    }

    #[test]
    fn new_rail_and_scope_stay_seated_during_receiver_growth() {
        let mut rig = Rig::default();
        rig.grow_to(Stats { damage_mult: 3.0, spread: 0.0, ..Default::default() });
        for _ in 0..59 {
            rig.update(1.0 / 30.0);
            let top = rig.build.parts[0].xf.matrix3.y_axis.length() * 0.5;
            let rail = rig.build.parts.iter().find(|p| p.key == 2 * 1024).unwrap();
            let bottom = rail.xf.transform_point3(Vec3::NEG_Y * 0.5).y;
            assert!(bottom <= top + 1e-5 && top - bottom < 0.004, "rail floated off the moving receiver");
            let scope_parts: Vec<_> = rig.build.parts.iter().filter(|p| p.key / 1024 == 16).collect();
            let mount_top = scope_parts[0].xf.transform_point3(Vec3::Y * 0.5).y;
            let scope_center = scope_parts[2].xf.translation.y;
            assert!((mount_top - scope_center).abs() < 1e-5, "scope separated from its growing mounts");
        }
    }
    #[test]
    fn front_sight_stays_on_its_support_when_barrels_change() {
        let base = Stats::default();
        for other in [Stats { fire_rate_mult: 8.0, ..base }, Stats { shots: 4, ..base }] {
            for (from, to) in [(base, other), (other, base)] {
                let mut rig = Rig::default(); rig.set(from); rig.grow_to(to);
                for _ in 0..60 {
                    rig.update(1.0 / 30.0);
                    let post = rig.build.parts.iter().find(|p| p.key == 10 * 1024).unwrap();
                    let bottom = post.xf.translation.y - post.xf.matrix3.y_axis.length() * 0.5;
                    let support = rig.build.parts.iter().find(|p| p.key == 14 * 1024 + 1)
                        .map(|p| (p.xf.matrix3.x_axis.length(), p.xf.translation.z))
                        .or_else(|| rig.build.parts.iter().find(|p| p.key == 15 * 1024)
                            .map(|p| (p.xf.translation.y + p.xf.matrix3.y_axis.length() * 0.5, p.xf.translation.z)));
                    if let Some((top, z)) = support {
                        assert!((top - bottom - JOINT).abs() < 1e-5, "front sight separated from growing support");
                        assert!((post.xf.translation.z - z).abs() < 1e-5);
                    }
                }
            }
        }
    }

    #[test]
    fn low_growth_keeps_every_part_inside_the_preallocated_budget() {
        let cases = [Stats::default(),
            Stats { poison: 0.7, cooling: 0.52, ..Default::default() },
            Stats { fire_rate_mult: 4.0, spread: 0.08, mag: 30, ..Default::default() },
            Stats { fire_rate_mult: 8.0, damage_mult: 5.0, mag: 60, ..Default::default() },
            Stats { shots: 16, damage_mult: 5.0, bullet_scale: 4.0, mag: 30, poison: 0.7, cooling: 0.52, ..Default::default() },
            Stats { spread: 0.0, shots: 16, fire_rate_mult: 4.0, damage_mult: 5.0, mag: 60, explosive_radius: 8.0, poison: 0.7, cooling: 0.52, ..Default::default() }];
        for from in cases { for to in cases {
            let mut rig = Rig::with_detail(Detail::Low); rig.set(from); rig.grow_to(to);
            for _ in 0..61 {
                rig.update(1.0 / 30.0);
                assert!(rig.build.parts.len() <= Detail::Low.part_capacity());
                assert!(rig.build.parts.iter().all(|p| p.xf.is_finite() && p.mat != Mat::Glass));
            }
            assert!(!rig.is_growing());
            assert!(rig.build.same_shape(&build_with_detail(&Look::of(&to), Detail::Low)));
        } }
    }

}
