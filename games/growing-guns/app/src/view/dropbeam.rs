//! The drop beam over a sky-drop pickup, from `scripts/pickup_item.gd`
//! (`_build_drop_beam`): a tapered, see-through emissive column in the
//! pickup's colour from where it was dropped (52 m up) down to where it
//! lands, pulsing, with a coloured omni light on the landing spot. It stays
//! for the pickup's whole life, so players see where the next one falls.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::{Pickup, PickupKind, World, PICKUPS};
use render::headlights::LocalLight;
use render::scene::NO_SHADOW;
use render::Scene;

/// Beams at once per kind (pickups seldom overlap; extras go without).
const PER_KIND: usize = 4;
/// Godot `CylinderMesh` radii (height is the sky-to-ground distance).
const TOP_RADIUS: f32 = 0.12;
const BOTTOM_RADIUS: f32 = 0.36;
/// Godot albedo alpha.
const ALPHA: f32 = 0.78;
/// Nits for Godot's `emission_energy_multiplier` 13 (its pulse is
/// 11 + 2.5 sin(4.2 t), carried here by the light and a slight swell).
const NITS: f32 = 6_000.0;
/// Godot `pickup_item.gd` drop height (flow.rs `PICKUP_HEIGHT`).
const SKY_Y: f32 = 52.0;
/// Godot `_compute_landing_pos`: ray hit plus 0.42 m.
const LAND_LIFT: f32 = 0.42;
/// Godot ground light: energy 8, range 5.5, 8 cm over the landing spot.
const LIGHT_ENERGY: f32 = 8.0;
const LIGHT_RANGE: f32 = 5.5;
/// Candela per unit of Godot light energy (as the blast and lava lights).
const ENERGY_CD: f32 = 37_000.0;

/// Godot `beam_color` (sRGB), in `PICKUPS` order.
pub(super) fn beam_color(kind: PickupKind) -> [f32; 3] {
    match kind {
        PickupKind::Heart => [1.0, 0.25, 0.45],
        PickupKind::Mushroom => [1.0, 0.35, 0.2],
        PickupKind::Bomb => [1.0, 0.72, 0.15],
        PickupKind::PlusOne => [0.35, 1.0, 0.45],
        PickupKind::Laser => [0.25, 0.95, 1.0],
        PickupKind::AirStrike => [1.0, 0.45, 0.12],
        PickupKind::Dice => [0.85, 0.95, 1.0],
    }
}

fn linear(c: [f32; 3]) -> Vec3 { Vec3::from(c.map(|v| v.powf(2.2))) }

pub(super) struct Beams { first: usize, meshes: [usize; 7], lights: Vec<LocalLight> }

/// Where a pickup comes down: straight below its drop point.
fn landing(world: &World, p: &Pickup) -> Vec3 {
    if p.grounded.is_some() { return p.pos; }
    let sky = Vec3::new(p.pos.x, SKY_Y, p.pos.z);
    match growing_guns::shape::raycast(&world.arena.grid, sky, Vec3::NEG_Y, 500.0) {
        Some((t, _, _)) => sky + Vec3::NEG_Y * t + Vec3::Y * LAND_LIFT,
        None => sky + Vec3::NEG_Y * 48.0 + Vec3::Y * LAND_LIFT,
    }
}

impl Beams {
    pub fn reserve(scene: &mut Scene) -> Self {
        let meshes = PICKUPS.map(|kind| {
            let m = super::super::glow(scene, linear(beam_color(kind)).to_array(), NITS);
            // The column is seen from inside and out as it towers over the
            // arena; additive so it never darkens what is behind it.
            scene.set_material_additive(m, ALPHA, true);
            scene.add_mesh(&render::primitives::cylinder(m, TOP_RADIUS, BOTTOM_RADIUS, 12).mesh())
        });
        let first = scene.instances.len();
        for mesh in meshes {
            for _ in 0..PER_KIND {
                // Transparency registers triangle owners once at startup:
                // dormant slots keep their mesh, zero scale hides them.
                scene.add_instance(mesh, Affine3A::from_scale(Vec3::ZERO));
                scene.instances.last_mut().unwrap().flags |= NO_SHADOW;
            }
        }
        Self { first, meshes, lights: Vec::new() }
    }

    pub fn pose(&mut self, scene: &mut Scene, world: &World, clock: f32) {
        let mut used = [0usize; 7];
        self.lights.clear();
        let pulse = (clock * 4.2).sin();
        for p in &world.pickups {
            let k = PICKUPS.iter().position(|&q| q == p.kind).unwrap_or(0);
            let land = landing(world, p);
            let color = linear(beam_color(p.kind));
            self.lights.push(LocalLight {
                position: land + Vec3::Y * 0.08, direction: Vec3::NEG_Y,
                intensity_cd: color * (LIGHT_ENERGY * ENERGY_CD * (1.0 + 0.15 * pulse)),
                range_m: LIGHT_RANGE, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.2, falloff: 1.0,
            });
            if used[k] >= PER_KIND { continue; }
            let sky = Vec3::new(land.x, SKY_Y, land.z);
            let height = (sky.y - land.y).max(0.5);
            let swell = 1.0 + 0.08 * pulse;
            let xf = Affine3A::from_scale_rotation_translation(Vec3::new(swell, height, swell), Quat::IDENTITY, (sky + land) * 0.5);
            scene.update_instance(self.first + k * PER_KIND + used[k], self.meshes[k], xf);
            used[k] += 1;
        }
        for (k, &n) in used.iter().enumerate() {
            for slot in n..PER_KIND {
                let i = self.first + k * PER_KIND + slot;
                if scene.instances[i].scale > 0.0 { scene.update_instance(i, self.meshes[k], Affine3A::from_scale(Vec3::ZERO)); }
            }
        }
    }

    /// The ground lights, one per pickup.
    pub fn lights(&self) -> &[LocalLight] { &self.lights }
}

/// Godot's per-kind pickup visual (`_build_visual`): unshaded emissive
/// spheres, cylinders and boxes at their own size, as one mesh.
pub(super) fn pickup_mesh(scene: &mut Scene, kind: PickupKind) -> usize {
    use render::primitives::{cube, cylinder, sphere, Primitive};
    enum S { Ball(f32), Can(f32, f32, f32), Box(Vec3) }
    let parts: &[(S, [f32; 3], f32, Vec3)] = match kind {
        PickupKind::Heart => &[(S::Ball(0.28), [1.0, 0.18, 0.28], 3.5, Vec3::ZERO)],
        PickupKind::Mushroom => &[
            (S::Ball(0.22), [0.95, 0.2, 0.18], 2.5, Vec3::new(0.0, 0.24, 0.0)),
            (S::Can(0.16, 0.14, 0.18), [0.95, 0.88, 0.72], 1.2, Vec3::ZERO),
        ],
        PickupKind::Bomb => &[
            (S::Ball(0.26), [0.12, 0.12, 0.14], 2.0, Vec3::ZERO),
            (S::Ball(0.06), [1.0, 0.85, 0.2], 4.0, Vec3::new(0.16, 0.2, 0.0)),
        ],
        PickupKind::PlusOne => &[(S::Box(Vec3::new(0.42, 0.42, 0.12)), [0.25, 0.95, 0.35], 3.0, Vec3::ZERO)],
        PickupKind::Laser => &[
            (S::Can(0.08, 0.08, 0.36), [0.2, 0.95, 1.0], 5.0, Vec3::ZERO),
            (S::Ball(0.05), [1.0, 0.2, 0.2], 6.0, Vec3::new(0.0, 0.0, -0.22)),
        ],
        PickupKind::Dice => &[
            (S::Box(Vec3::splat(0.34)), [0.95, 0.92, 1.0], 4.5, Vec3::ZERO),
            (S::Ball(0.05), [0.15, 0.15, 0.2], 3.0, Vec3::new(0.1, 0.12, 0.18)),
        ],
        // `_build_air_strike_rocket`.
        PickupKind::AirStrike => &[
            (S::Can(0.06, 0.075, 0.34), [0.95, 0.72, 0.38], 4.2, Vec3::ZERO),
            (S::Can(0.02, 0.06, 0.11), [1.0, 0.96, 0.88], 5.5, Vec3::new(0.0, 0.225, 0.0)),
            (S::Can(0.095, 0.045, 0.11), [1.0, 0.55, 0.12], 7.0, Vec3::new(0.0, -0.225, 0.0)),
            (S::Box(Vec3::new(0.16, 0.02, 0.04)), [0.95, 0.72, 0.38], 2.8, Vec3::new(0.0, -0.1, 0.0)),
            (S::Box(Vec3::new(0.04, 0.02, 0.16)), [0.95, 0.72, 0.38], 2.8, Vec3::new(0.0, -0.1, 0.0)),
        ],
    };
    let prims: Vec<Primitive> = parts.iter().map(|(shape, color, energy, at)| {
        let m = super::super::glow(scene, linear(*color).to_array(), energy * NITS_PER_ENERGY);
        let (mut p, size) = match *shape {
            S::Ball(r) => (sphere(m, 4, 7), Vec3::splat(r)),
            S::Can(top, bottom, h) => (cylinder(m, top, bottom, 6), Vec3::new(1.0, h, 1.0)),
            S::Box(s) => (cube(m), s),
        };
        for v in &mut p.vertices { v.pos = (Vec3::from(v.pos) * size + *at).to_array(); }
        p
    }).collect();
    let parts: Vec<_> = prims.iter().map(|p| p.part()).collect();
    scene.add_mesh(&render::cluster::ClusterMesh::build(&parts))
}

/// Nits per unit of Godot `emission_energy_multiplier` for the small
/// unshaded pickup shapes.
const NITS_PER_ENERGY: f32 = 2_500.0;
