//! Renderer-independent wheel debris emission, sampled once per physics tick.
//!
//! This owns only visual credits/seeds. It never touches physics, the race RNG
//! or its digest. A renderer can upload the returned bounded births to one GPU
//! particle pool shared by all split-screen views.

use crate::{car::Wheel, surfaces::Surface};
use gdcompat::math::{V3, v3};

pub const MAX_WHEELS: usize = 16;
pub const MAX_SPAWNS_PER_TICK: usize = 96;
const DT: f32 = 1.0 / 120.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Stone,
    SoilClod,
    GrassBlade,
    MudClod,
    Dust,
    TyreSmoke,
    WaterDrop,
}

/// One real wheel contact. IDs should be stable: car_slot * 4 + wheel_index.
/// The caller may override `surface` with a per-contact water query. Existing
/// vehicle handling classifies water using the car-centre water level.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub id: u8,
    pub grounded: bool,
    pub surface: Surface,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub inherited_velocity: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub forward_speed: f32,
    pub lateral_speed: f32,
    pub tread_speed: f32,
    pub load: f32,
    pub slide: f32,
}

impl Contact {
    /// `point_velocity` is body linear velocity + angular_velocity × (contact
    /// point - world centre of mass), in metres/second.
    pub fn from_wheel(id: u8, wheel: &Wheel, point_velocity: [f32; 3]) -> Self {
        let a = |v: rapier3d::glamx::Vec3| [v.x, v.y, v.z];
        Self {
            id,
            grounded: wheel.contact,
            surface: wheel.surface,
            point: a(wheel.point),
            normal: a(wheel.normal),
            inherited_velocity: point_velocity,
            forward: a(wheel.wf),
            right: a(wheel.wr),
            forward_speed: wheel.vx as f32,
            lateral_speed: wheel.vy as f32,
            tread_speed: (wheel.omega * wheel.radius) as f32,
            load: wheel.load as f32,
            slide: wheel.slide as f32,
        }
    }

    fn valid(self) -> bool {
        self.grounded
            && self.load > 1.0
            && self.load.is_finite()
            && [
                self.point,
                self.normal,
                self.inherited_velocity,
                self.forward,
                self.right,
            ]
            .into_iter()
            .flatten()
            .all(f32::is_finite)
            && [
                self.forward_speed,
                self.lateral_speed,
                self.tread_speed,
                self.slide,
            ]
            .into_iter()
            .all(f32::is_finite)
            && vec(self.normal).length_squared() > 0.1
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawn {
    pub wheel_id: u8,
    pub serial: u64,
    pub kind: Kind,
    pub surface: Surface,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Unscaled physical point velocity, retained for renderer effects.
    pub inherited_velocity: [f32; 3],
    /// Direction of the tyre surface relative to the ground; backward for
    /// forward wheelspin, forward under braking, lateral while sliding.
    pub slip_direction: [f32; 3],
    pub velocity: [f32; 3],
    pub radius: f32,
    pub lifetime: f32,
    pub wetness: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub emitted: usize,
    pub budget_dropped: usize,
    pub invalid_contacts: usize,
}

/// Fixed memory and birth budget. Per-wheel serials make another car joining
/// unable to change this wheel's random variations. Input order is irrelevant.
pub struct Emitter {
    seed: u64,
    credits: [[f32; 2]; MAX_WHEELS],
    serials: [u64; MAX_WHEELS],
    last_tick: Option<u64>,
    births: Vec<Spawn>,
    budget: usize,
    pub stats: Stats,
}

impl Emitter {
    pub fn new(seed: u64) -> Self {
        Self::with_budget(seed, MAX_SPAWNS_PER_TICK)
    }

    /// A smaller explicit budget is useful for lower quality settings/tests.
    pub fn with_budget(seed: u64, budget: usize) -> Self {
        Self {
            seed,
            credits: [[0.0; 2]; MAX_WHEELS],
            serials: [0; MAX_WHEELS],
            last_tick: None,
            births: Vec::with_capacity(MAX_SPAWNS_PER_TICK),
            budget: budget.min(MAX_SPAWNS_PER_TICK),
            stats: Stats::default(),
        }
    }

    /// Call on stage replacement. Call `reset_wheel` for every wheel of a
    /// recovered car, and omit those contacts for its teleport tick: contact
    /// data in the vehicle can still describe the previous position then.
    pub fn reset(&mut self, seed: u64) {
        self.seed = seed;
        self.credits.fill([0.0; 2]);
        self.serials.fill(0);
        self.last_tick = None;
        self.births.clear();
        self.stats = Stats::default();
    }

    pub fn reset_wheel(&mut self, id: u8) {
        if let Some(credit) = self.credits.get_mut(id as usize) {
            *credit = [0.0; 2];
        }
    }

    /// Call once immediately after each 120 Hz simulation tick, never once
    /// per rendered view. Repeated tick IDs emit nothing; skipped/backwards
    /// ticks discard credits instead of inventing a long trail across a cut.
    pub fn tick(&mut self, tick: u64, contacts: &[Contact], wetness: f32) -> &[Spawn] {
        self.births.clear();
        self.stats = Stats::default();
        if self.last_tick == Some(tick) {
            return &self.births;
        }
        if self
            .last_tick
            .is_some_and(|last| last.checked_add(1) != Some(tick))
        {
            self.credits.fill([0.0; 2]);
        }
        self.last_tick = Some(tick);
        let wet = if wetness.is_finite() {
            wetness.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let mut samples: [Option<Contact>; MAX_WHEELS] = [None; MAX_WHEELS];
        for &contact in contacts {
            let id = contact.id as usize;
            if id >= MAX_WHEELS || samples[id].is_some() {
                self.stats.invalid_contacts += 1;
                continue;
            }
            samples[id] = Some(contact);
        }
        // Rotate priority when explicitly budget-limited, so all four cars
        // receive effects rather than starving the final seat indefinitely.
        for offset in 0..MAX_WHEELS {
            let id = (tick as usize % MAX_WHEELS + offset) % MAX_WHEELS;
            let Some(c) = samples[id].filter(|c| c.valid()) else {
                self.credits[id] = [0.0; 2];
                continue;
            };
            let (rates, kinds, activity) = emission(c, wet);
            for channel in 0..2 {
                self.credits[id][channel] += rates[channel] * DT;
                let count = self.credits[id][channel].floor() as usize;
                self.credits[id][channel] -= count as f32;
                for _ in 0..count {
                    let serial = self.serials[id];
                    self.serials[id] = serial.wrapping_add(1);
                    if self.births.len() >= self.budget {
                        self.stats.budget_dropped += 1;
                        continue;
                    }
                    self.births
                        .push(spawn(self.seed, c, kinds[channel], serial, activity, wet));
                }
            }
        }
        self.stats.emitted = self.births.len();
        &self.births
    }
}

fn emission(c: Contact, wet: f32) -> ([f32; 2], [Kind; 2], f32) {
    let slip = (c.tread_speed - c.forward_speed).hypot(c.lateral_speed);
    let speed = c.forward_speed.hypot(c.lateral_speed);
    let load = (c.load / 2000.0).clamp(0.0, 1.0);
    let activity = (((speed - 1.5).max(0.0) * 0.14 + (slip - 0.5).max(0.0) * 0.7) * load).min(15.0);
    let dry_dust = (1.0 - wet).powi(2);
    match c.surface {
        Surface::Gravel => (
            [activity * 2.0, activity * 3.0 * dry_dust],
            [Kind::Stone, Kind::Dust],
            activity,
        ),
        Surface::Loose | Surface::Dirt => (
            [activity * 1.5, activity * 3.5 * dry_dust],
            [Kind::SoilClod, Kind::Dust],
            activity,
        ),
        Surface::Grass => (
            [activity * 2.5, activity * 0.6 * dry_dust],
            [Kind::GrassBlade, Kind::Dust],
            activity,
        ),
        Surface::Mud => ([activity * 2.2, 0.0], [Kind::MudClod, Kind::Dust], activity),
        Surface::Water => (
            [
                ((speed - 0.5).max(0.0) * 0.6 + slip * 0.5).min(20.0) * load * 4.0,
                0.0,
            ],
            [Kind::WaterDrop, Kind::Dust],
            activity,
        ),
        Surface::Asphalt => (
            [
                ((slip - 3.0).max(0.0) * c.slide.clamp(0.0, 1.0)).min(15.0) * load * 2.0 * dry_dust,
                0.0,
            ],
            [Kind::TyreSmoke, Kind::Dust],
            activity,
        ),
    }
}

fn vec(a: [f32; 3]) -> V3 {
    v3(a[0], a[1], a[2])
}
fn array(v: V3) -> [f32; 3] {
    [v.x, v.y, v.z]
}
fn random(seed: u64, id: u8, serial: u64, lane: u64) -> f32 {
    let mut n = seed
        ^ (id as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ serial.wrapping_mul(0xbf58476d1ce4e5b9)
        ^ lane.wrapping_mul(0x94d049bb133111eb);
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((n ^ (n >> 31)) >> 40) as f32 / 16_777_216.0
}

fn spawn(seed: u64, c: Contact, mut kind: Kind, serial: u64, activity: f32, wet: f32) -> Spawn {
    let r = |lane| random(seed, c.id, serial, lane);
    // Grass releases both recognisable blades and occasional earthy roots.
    if kind == Kind::GrassBlade && r(0) < 0.25 {
        kind = Kind::SoilClod;
    }
    let normal = vec(c.normal).normalized();
    let forward = vec(c.forward);
    let right = vec(c.right);
    let shear = forward * (c.forward_speed - c.tread_speed) + right * c.lateral_speed;
    let slip_direction = if shear.length_squared() > 1e-8 {
        shear.normalized()
    } else {
        V3::ZERO
    };
    let (radius, life, lift, inherited, kick) = match kind {
        Kind::Stone => (0.012 + r(1) * 0.025, 1.0 + r(2), 1.1, 0.22, 0.32),
        Kind::SoilClod => (0.015 + r(1) * 0.03, 0.9 + r(2), 0.9, 0.18, 0.25),
        Kind::GrassBlade => (0.018 + r(1) * 0.025, 1.0 + r(2), 1.3, 0.18, 0.25),
        Kind::MudClod => (0.02 + r(1) * 0.04, 0.7 + r(2) * 0.7, 0.7, 0.16, 0.20),
        Kind::Dust => (0.08 + r(1) * 0.14, 1.0 + r(2) * 1.5, 0.45, 0.10, 0.08),
        Kind::TyreSmoke => (0.09 + r(1) * 0.12, 1.3 + r(2) * 1.3, 0.5, 0.08, 0.06),
        Kind::WaterDrop => (0.008 + r(1) * 0.015, 0.45 + r(2) * 0.75, 1.0, 0.3, 0.30),
    };
    let shear = if shear.length() > 35.0 {
        shear.normalized() * 35.0
    } else {
        shear
    };
    let velocity = vec(c.inherited_velocity) * inherited + shear * kick
        - forward * c.forward_speed.clamp(-60.0, 60.0) * 0.035
        + normal * (lift + activity.sqrt() * (0.1 + r(3) * 0.2))
        + right * ((r(4) - 0.5) * 1.3);
    Spawn {
        wheel_id: c.id,
        serial,
        kind,
        surface: c.surface,
        position: array(vec(c.point) + normal * radius.max(0.02)),
        normal: array(normal),
        inherited_velocity: c.inherited_velocity,
        slip_direction: array(slip_direction),
        velocity: array(velocity),
        radius,
        lifetime: life,
        wetness: wet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contact(id: u8, surface: Surface) -> Contact {
        Contact {
            id,
            grounded: true,
            surface,
            point: [1.0, 2.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            inherited_velocity: [0.0, 0.0, -15.0],
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            forward_speed: 15.0,
            lateral_speed: 0.0,
            tread_speed: 25.0,
            load: 3000.0,
            slide: 0.8,
        }
    }
    fn collect(c: &[Contact], wet: f32) -> Vec<Spawn> {
        let mut emitter = Emitter::new(42);
        (0..240)
            .flat_map(|tick| emitter.tick(tick, c, wet).to_vec())
            .collect()
    }

    #[test]
    fn airborne_unloaded_stationary_and_invalid_contacts_emit_nothing() {
        for change in 0..4 {
            let mut c = contact(0, Surface::Gravel);
            match change {
                0 => c.grounded = false,
                1 => c.load = 0.0,
                2 => {
                    c.forward_speed = 0.0;
                    c.tread_speed = 0.0;
                    c.lateral_speed = 0.0;
                }
                _ => c.point[0] = f32::NAN,
            }
            assert!(collect(&[c], 0.0).is_empty());
        }
    }

    #[test]
    fn surface_materials_wet_dust_and_asphalt_are_distinct() {
        let dry = collect(&[contact(0, Surface::Gravel)], 0.0);
        let wet = collect(&[contact(0, Surface::Gravel)], 1.0);
        assert!(dry.iter().any(|p| p.kind == Kind::Dust));
        assert!(wet.iter().all(|p| p.kind == Kind::Stone));
        let grass = collect(&[contact(0, Surface::Grass)], 0.0);
        assert!(grass.iter().any(|p| p.kind == Kind::GrassBlade));
        assert!(grass.iter().any(|p| p.kind == Kind::SoilClod));
        for (surface, kind) in [
            (Surface::Mud, Kind::MudClod),
            (Surface::Water, Kind::WaterDrop),
            (Surface::Asphalt, Kind::TyreSmoke),
        ] {
            let particles = collect(&[contact(0, surface)], 0.0);
            assert!(!particles.is_empty());
            assert!(particles.iter().all(|p| p.kind == kind));
        }
        let mut asphalt = contact(0, Surface::Asphalt);
        asphalt.tread_speed = asphalt.forward_speed;
        assert!(
            collect(&[asphalt], 0.0).is_empty(),
            "rolling asphalt must not emit dirt or tyre smoke"
        );
        assert!(
            collect(&[contact(0, Surface::Asphalt)], 1.0).is_empty(),
            "rain suppresses dry smoke, not invented dirt"
        );
    }

    #[test]
    fn wheelspin_braking_and_sideways_slip_eject_in_physical_directions() {
        let spinning = collect(&[contact(0, Surface::Gravel)], 0.0);
        assert!(spinning.iter().all(|p| p.slip_direction[2] > 0.99));
        let mut brake = contact(0, Surface::Gravel);
        brake.tread_speed = 0.0;
        assert!(
            collect(&[brake], 0.0)
                .iter()
                .all(|p| p.slip_direction[2] < -0.99)
        );
        let mut sideways = contact(0, Surface::Gravel);
        sideways.tread_speed = sideways.forward_speed;
        sideways.lateral_speed = 8.0;
        let particles = collect(&[sideways], 0.0);
        assert!(!particles.is_empty());
        assert!(particles.iter().all(|p| p.slip_direction[0] > 0.99));
    }

    #[test]
    fn four_player_budget_is_bounded_fair_and_deterministic() {
        let contacts: Vec<_> = (0..16).map(|id| contact(id, Surface::Gravel)).collect();
        let mut a = Emitter::with_budget(43, 2);
        let mut b = Emitter::with_budget(43, 2);
        let reversed: Vec<_> = contacts.iter().copied().rev().collect();
        let mut seen = [false; 16];
        let mut dropped = 0;
        for tick in 0..1200 {
            let first = a.tick(tick, &contacts, 0.0).to_vec();
            assert_eq!(first, b.tick(tick, &reversed, 0.0));
            assert!(first.len() <= 2);
            for p in first {
                seen[p.wheel_id as usize] = true;
                assert!(p.velocity.into_iter().all(f32::is_finite));
            }
            dropped += a.stats.budget_dropped;
            assert!(
                a.tick(tick, &contacts, 0.0).is_empty(),
                "second view must not duplicate emissions"
            );
        }
        assert!(seen.into_iter().all(|v| v));
        assert!(dropped > 0);
    }

    #[test]
    fn reset_and_missing_contacts_cannot_release_stale_emission_credit() {
        let c = contact(0, Surface::Gravel);
        let mut emitter = Emitter::new(42);
        emitter.tick(0, &[c], 0.0);
        assert!(emitter.tick(1, &[], 0.0).is_empty());
        assert!(emitter.tick(2, &[c], 0.0).is_empty());
        emitter.reset_wheel(0);
        assert!(emitter.tick(3, &[c], 0.0).is_empty());
        emitter.reset(42);
        assert_eq!(
            emitter.tick(0, &[c], 0.0),
            Emitter::new(42).tick(0, &[c], 0.0)
        );
    }
}
