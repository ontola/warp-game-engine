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
}

impl Default for Stats {
    fn default() -> Self { Stats { spread: SPREAD, bullet_scale: 1.0, damage_mult: 1.0, fire_rate_mult: 1.0, mag: MAG_SIZE, shots: 1, explosive_radius: 0.0 } }
}

impl Stats {
    pub fn of(p: &Player) -> Self {
        let w = &p.weapon;
        Stats { spread: w.spread(), bullet_scale: w.bullet_scale, damage_mult: w.damage_mult, fire_rate_mult: w.fire_rate_mult, mag: p.mag_size(), shots: w.shots_per_trigger(), explosive_radius: w.explosive_radius }
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
pub enum Mat { Metal, Dark, Barrel, Polymer, Recess, Slot, Bolt, Pad, Lens, Indicator }
pub const MATS: [Mat; 10] = [Mat::Metal, Mat::Dark, Mat::Barrel, Mat::Polymer, Mat::Recess, Mat::Slot, Mat::Bolt, Mat::Pad, Mat::Lens, Mat::Indicator];

/// Unit shapes: a box (side 1), cylinders of height 1 along Y with radius 1
/// at the top (tapering to `bottom` below), rings around Y of radius 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape { Box, Cyl, Taper85, Taper105, Ring18, Ring25 }
pub const SHAPES: [Shape; 6] = [Shape::Box, Shape::Cyl, Shape::Taper85, Shape::Taper105, Shape::Ring18, Shape::Ring25];

/// How a part moves: bolts slide back on a shot, minigun barrels spin,
/// the magazine drops out on reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion { Fixed, Bolt, Spin, Mag }

#[derive(Clone, Copy, Debug)]
pub struct Part { pub shape: Shape, pub mat: Mat, pub xf: Affine3A, pub motion: Motion }

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
}

const MUZZLE_FLARE: f32 = 1.55;
const MUZZLE_LENGTH: f32 = 0.05;
const MAG_OFFSET_Z: f32 = 0.04;
const FOREGRIP_LENGTH: f32 = 0.12;
const FOREGRIP_RADIUS: f32 = 0.022;
const FOREGRIP_OFFSET_Z: f32 = 0.26;
const HANDGUARD_HEIGHT: f32 = 0.025;
const RAIL_HEIGHT: f32 = 0.02;
const SIGHT_POST: f32 = 0.04;
const REAR_SIGHT_R: f32 = 0.022;
const SCOPE_LENGTH: f32 = 0.18;
const GRIP_LENGTH: f32 = 0.13;
const GRIP_RADIUS: f32 = 0.024;
const GRIP_TILT: f32 = -36.0 * PI / 180.0;
const GUARD_R: f32 = 0.028;
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
        let receiver = Vec3::new(0.075, lerp(0.10, 0.16, dmg), lerp(0.26, 0.55, dmg));
        let drum = w.mag > 30;
        let mag = if drum {
            Vec3::new(0.07, (0.18 + 0.006 * (w.mag - 30) as f32).clamp(0.18, 0.36), lerp(0.06, 0.11, dmg))
        } else {
            Vec3::new(0.08, (0.04 + 0.024 * w.mag as f32).clamp(0.06, 0.6), lerp(0.06, 0.11, dmg))
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
        }
    }

}

/// A built gun: parts in gun space, how far the bolt travels, whether the
/// barrels spin, and the barrel length (the view pulls long guns back).
#[derive(Clone, Debug, Default)]
pub struct Build { pub parts: Vec<Part>, pub bolt_travel: f32, pub minigun: bool, pub barrel_length: f32, /** Gun-space barrel exit. */ pub muzzle: Vec3 }

struct B { parts: Vec<Part> }
impl B {
    fn put(&mut self, shape: Shape, mat: Mat, size: Vec3, pos: Vec3, rot: Quat, motion: Motion) {
        self.parts.push(Part { shape, mat, xf: Affine3A::from_scale_rotation_translation(size, rot, pos), motion });
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
    let mut b = B { parts: Vec::with_capacity(96) };
    let n = l.barrels.max(1);
    let pitch = l.barrel_radius * 2.4;
    let span = pitch * (n - 1) as f32;
    let muzzle_r = l.barrel_radius * MUZZLE_FLARE;
    let bundle = l.barrel_radius * 1.8;
    let min_w = if l.minigun { (bundle + l.barrel_radius) * 2.2 } else { span + l.barrel_radius * 3.0 };
    let rs = Vec3::new(l.receiver.x.max(min_w), l.receiver.y, l.receiver.z);
    b.bx(rs, Vec3::ZERO, Metal);
    b.bx(Vec3::new(rs.x * 1.04, rs.y * 0.34, rs.z * 0.92), Vec3::new(0.0, -rs.y * 0.34, rs.z * 0.02), Dark);
    b.bx(Vec3::new(0.004, rs.y * 0.26, rs.z * 0.3), Vec3::new(rs.x * 0.5 + 0.001, rs.y * 0.14, -rs.z * 0.12), Recess);
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
    // Picatinny rail with its teeth.
    let rail_y = rs.y * 0.5 + RAIL_HEIGHT * 0.5;
    let rail = Vec3::new((rs.x * 0.8).min(0.04), RAIL_HEIGHT, rs.z * 0.85);
    let base_h = rail.y * 0.5;
    b.bx(Vec3::new(rail.x, base_h + JOINT, rail.z), Vec3::new(0.0, rail_y - rail.y * 0.5 + (base_h - JOINT) * 0.5, 0.0), Dark);
    let period = 0.022;
    let teeth = ((rail.z / period) as u32).max(1);
    for i in 0..teeth {
        let z = -((teeth - 1) as f32 * period) * 0.5 + i as f32 * period;
        b.bx(Vec3::new(rail.x * 1.2, base_h + JOINT, 0.012), Vec3::new(0.0, rail_y + rail.y * 0.5 - (base_h + JOINT) * 0.5, z), Dark);
    }
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
    if l.shroud {
        let len = l.barrel_length * l.shroud_frac.clamp(0.05, 0.95);
        shroud(&mut b, front - len * 0.5, len, l.shroud_extent.max(l.barrel_radius + 0.012), l.barrel_radius + 0.012, 5);
    }
    if l.drum {
        let r = l.mag.y * 0.5;
        // A circular drum otherwise meets the receiver at a single tangent.
        b.put(Shape::Box, Dark, Vec3::new(0.045, 0.04, 0.045), Vec3::new(0.0, -rs.y * 0.5 - 0.012, MAG_OFFSET_Z), Quat::IDENTITY, Motion::Mag);
        b.put(Shape::Cyl, Dark, Vec3::new(r, l.mag.x, r), Vec3::new(0.0, -rs.y * 0.5 - r, MAG_OFFSET_Z), Quat::from_rotation_x(PI * 0.5), Motion::Mag);
    } else {
        let y = -rs.y * 0.5 - l.mag.y * 0.5 + JOINT;
        b.put(Shape::Box, Polymer, l.mag, Vec3::new(0.0, y, MAG_OFFSET_Z), Quat::IDENTITY, Motion::Mag);
        b.put(Shape::Box, Dark, Vec3::new(l.mag.x * 1.1, 0.012, l.mag.z * 1.15), Vec3::new(0.0, y - l.mag.y * 0.5, MAG_OFFSET_Z), Quat::IDENTITY, Motion::Mag);
    }
    if l.foregrip {
        let gz = front - FOREGRIP_OFFSET_Z;
        let hg_y = -l.barrel_radius - HANDGUARD_HEIGHT * 0.5 + JOINT;
        b.bx(Vec3::new(l.barrel_radius * 2.4, HANDGUARD_HEIGHT, front - gz), Vec3::new(0.0, hg_y, (front + gz) * 0.5), Polymer);
        let tilt = -8f32.to_radians();
        let rot = Quat::from_rotation_x(tilt);
        let anchor = Vec3::new(0.0, hg_y - HANDGUARD_HEIGHT * 0.5 + JOINT + FOREGRIP_RADIUS * tilt.sin().abs(), gz + FOREGRIP_RADIUS * 1.1);
        b.put(Shape::Taper85, Polymer, Vec3::new(FOREGRIP_RADIUS, FOREGRIP_LENGTH, FOREGRIP_RADIUS), anchor - rot * Vec3::Y * (FOREGRIP_LENGTH * 0.5), rot, Motion::Fixed);
    }
    let pz = rs.z * 0.32;
    // Rotate about the attachment, not the centre: the complete upper cap
    // stays seated in the receiver even on an angled grip.
    let grip_rot = Quat::from_rotation_x(GRIP_TILT);
    let grip_anchor = Vec3::new(0.0, -rs.y * 0.5 + JOINT + GRIP_RADIUS * GRIP_TILT.sin().abs(), pz);
    b.put(Shape::Taper105, Polymer, Vec3::new(GRIP_RADIUS, GRIP_LENGTH, GRIP_RADIUS), grip_anchor - grip_rot * Vec3::Y * (GRIP_LENGTH * 0.5), grip_rot, Motion::Fixed);
    // Trigger guard: a ring opening downward (axis along X).
    b.put(Shape::Ring18, Dark, Vec3::splat(GUARD_R * 0.91), Vec3::new(0.0, -rs.y * 0.5 - GUARD_R * 0.5, (MAG_OFFSET_Z + pz) * 0.5), Quat::from_rotation_z(PI * 0.5), Motion::Fixed);
    if l.stock {
        let (back_h, front_h, half_z) = (STOCK.y, STOCK.y * STOCK_TAPER, STOCK.z * 0.5);
        let bottom = -rs.y * 0.5;
        let back_z = rs.z * 0.5 + half_z * 1.5;
        b.bx(Vec3::new(STOCK.x, back_h, half_z + JOINT), Vec3::new(0.0, bottom + back_h * 0.5, back_z - JOINT * 0.5), Polymer);
        b.bx(Vec3::new(STOCK.x * 0.95, front_h, half_z + JOINT), Vec3::new(0.0, bottom + front_h * 0.5, rs.z * 0.5 + (half_z - JOINT) * 0.5), Polymer);
        b.bx(Vec3::new(STOCK.x * 1.04, back_h * 1.04, 0.012), Vec3::new(0.0, bottom + back_h * 0.5, back_z + half_z * 0.5 + 0.006 - JOINT), Pad);
    }
    if l.tube {
        let r = rs.y * 0.5;
        b.cyl(Shape::Cyl, r, l.tube_length, Vec3::new(0.0, 0.0, rs.z * 0.5 + l.tube_length * 0.5 - JOINT), Dark, Motion::Fixed);
    }
    if !l.scope {
        let sight_z = front - l.barrel_length * 0.85;
        let sight_base = if l.minigun {
            let r = bundle + l.barrel_radius + 0.004;
            for z in [front - 0.007, sight_z] { b.cyl(Shape::Cyl, r, 0.02, Vec3::new(0.0, 0.0, z), Dark, Motion::Fixed); }
            r
        } else if n > 1 {
            // Even barrel counts have no central barrel beneath the sight.
            b.bx(Vec3::new(span + l.barrel_radius * 2.0, 0.008, 0.018), Vec3::new(0.0, l.barrel_radius - 0.001, sight_z), Dark);
            l.barrel_radius + 0.003
        } else { l.barrel_radius };
        b.bx(Vec3::new(0.005, SIGHT_POST, 0.012), Vec3::new(0.0, sight_base + SIGHT_POST * 0.5 - JOINT, sight_z), Dark);
        b.put(Shape::Ring25, Dark, Vec3::splat(REAR_SIGHT_R * 0.875), Vec3::new(0.0, rail_y + RAIL_HEIGHT * 0.5 + REAR_SIGHT_R * 0.4, rs.z * 0.32), Quat::from_rotation_x(PI * 0.5), Motion::Fixed);
    } else {
        let (tube_r, bell_r, bell_len) = (0.018, 0.028, 0.04);
        let top = rail_y + RAIL_HEIGHT * 0.5;
        let sy = top + 0.025;
        let mount_bottom = top - base_h - JOINT;
        for rz in [-SCOPE_LENGTH * 0.3, SCOPE_LENGTH * 0.3] { b.bx(Vec3::new(tube_r * 2.2, sy - mount_bottom, 0.02), Vec3::new(0.0, (mount_bottom + sy) * 0.5, rz), Dark); }
        b.cyl(Shape::Cyl, tube_r, SCOPE_LENGTH, Vec3::new(0.0, sy, 0.0), Dark, Motion::Fixed);
        b.cyl(Shape::Cyl, bell_r, bell_len, Vec3::new(0.0, sy, -SCOPE_LENGTH * 0.5 - bell_len * 0.5), Dark, Motion::Fixed);
        b.cyl(Shape::Cyl, bell_r, bell_len, Vec3::new(0.0, sy, SCOPE_LENGTH * 0.5 + bell_len * 0.5), Dark, Motion::Fixed);
        b.cyl(Shape::Cyl, bell_r * 0.85, 0.005, Vec3::new(0.0, sy, -SCOPE_LENGTH * 0.5 - bell_len + 0.002), Lens, Motion::Fixed);
    }
    b.bx(INDICATOR, Vec3::new(rs.x * 0.52 + INDICATOR.x * 0.5 - 0.001, -rs.y * 0.23, 0.0), Indicator);
    Build { parts: b.parts, bolt_travel: travel, minigun: l.minigun, barrel_length: l.barrel_length, muzzle: Vec3::new(0.0, 0.0, muzzle_z - MUZZLE_LENGTH * 0.5) }
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
    pub shown: Stats,
    pub build: Build,
    grow: Option<(Stats, Stats, f32)>,
    pub heat: f32,
    bolt_back: f32,
    bolt_cycle: f32,
    spin_speed: f32,
    spin: f32,
}

impl Default for Rig {
    fn default() -> Self { let s = Stats::default(); Rig { shown: s, build: build(&Look::of(&s)), grow: None, heat: 0.0, bolt_back: 0.0, bolt_cycle: 0.08, spin_speed: 0.0, spin: 0.0 } }
}

pub const GROW_SECONDS: f32 = 2.0;

impl Rig {
    /// Snap to these stats (a new round, a respawn).
    pub fn set(&mut self, s: Stats) {
        self.grow = None;
        if s != self.shown { self.shown = s; self.build = build(&Look::of(&s)); }
    }

    /// Morph from what is shown into `to` (a card pick).
    pub fn grow_to(&mut self, to: Stats) {
        if to == self.shown && self.grow.is_none() { return; }
        let from = self.shown;
        self.grow = Some((from, to, 0.0));
    }

    pub fn is_growing(&self) -> bool { self.grow.is_some() }

    /// Keep the target current: a morph retargets, otherwise a change snaps.
    pub fn follow(&mut self, target: Stats) {
        match &mut self.grow {
            Some((_, to, _)) => *to = target,
            None => self.set(target),
        }
    }

    /// A shot: bolt to the rear stop, barrels spin up, heat climbs.
    pub fn fire(&mut self, stats: &Stats) {
        self.bolt_back = self.build.bolt_travel;
        self.bolt_cycle = stats.fire_interval();
        if self.build.minigun { self.spin_speed = (self.spin_speed + 12.0).min(45.0); }
        self.heat = (self.heat + stats.damage_mult * stats.shots as f32 * 0.01).clamp(0.0, 1.5);
    }

    pub fn update(&mut self, dt: f32) {
        if let Some((from, to, t)) = &mut self.grow {
            *t += dt.min(0.05);
            let (from, to, t) = (*from, *to, *t);
            if t >= GROW_SECONDS { self.grow = None; self.shown = to; self.build = build(&Look::of(&to)); }
            else { let s = Stats::lerp(&from, &to, grow_ease(t / GROW_SECONDS)); self.build = build(&Look::of(&s)); }
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
        let spin = Affine3A::from_rotation_z(self.spin);
        self.build.parts.iter().map(move |p| {
            let xf = match p.motion {
                Motion::Fixed => p.xf,
                Motion::Bolt => Affine3A::from_translation(Vec3::Z * self.bolt_back) * p.xf,
                Motion::Spin => spin * p.xf,
                Motion::Mag => Affine3A::from_translation(Vec3::NEG_Y * mag_drop) * p.xf,
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
    fn grips_and_stock_remain_seated_across_receiver_sizes() {
        for damage_mult in [1.0, 2.5, 5.0] {
            let look = Look::of(&Stats { spread: 0.0, damage_mult, ..Default::default() });
            let built = build(&look);
            let grip = built.parts.iter().find(|p| p.shape == Shape::Taper105).unwrap();
            for i in 0..32 {
                let a = i as f32 * TAU / 32.0;
                let top = grip.xf.transform_point3(Vec3::new(a.cos(), 0.5, a.sin()));
                assert!(top.y > -look.receiver.y * 0.5, "tilted upper grip cap must be embedded");
            }
            let pad = built.parts.iter().find(|p| p.mat == Mat::Pad).unwrap();
            let pad_front = pad.xf.transform_point3(Vec3::new(0.0, 0.0, -0.5)).z;
            let stock_back = look.receiver.z * 0.5 + STOCK.z;
            assert!(pad_front < stock_back && stock_back - pad_front < 0.01, "butt pad must sit on the stock");
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
}
