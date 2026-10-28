//! The drop beam over a sky-drop pickup, from `scripts/pickup_item.gd`
//! (`_build_drop_beam`): a tapered, see-through emissive column in the
//! pickup's colour from where it was dropped (52 m up) down to where it
//! lands, pulsing, with a coloured omni light on the landing spot. It stays
//! for the pickup's whole life, so players see where the next one falls.
//!
//! Volumetric look: no hard tube but a light shaft, an additive sprite
//! turned about the vertical towards each camera whose texture falls off
//! softly from a bright core into a wide glow (as a beam through hazy air
//! looks), tapering to the top. At night, when the arena air scatters
//! (`Lighting::volumetric`), a scatter-only searchlight from the landing
//! spot also lights the air along it and the ground light glows around
//! the pickup.
use glam::Vec3;
use growing_guns::{Pickup, PickupKind, World};
use render::headlights::LocalLight;
use render::picture::Picture;
use render::sprites::{Sprite, SpriteImage};
use render::Scene;

/// The shaft's texture.
pub(super) const IMAGE: SpriteImage = SpriteImage(0x6265_616d);
/// Half-width of the glow at the ground (the texture's edge), metres.
const HALF_WIDTH: f32 = 1.1;
/// Godot `CylinderMesh` radii: the core's taper from ground to sky.
const TOP_RADIUS: f32 = 0.12;
const BOTTOM_RADIUS: f32 = 0.36;
/// The core's and the glow's spread, in metres at the ground (scaled by
/// the taper higher up), and the glow's strength relative to the core.
const CORE: f32 = 0.24;
const GLOW: f32 = 0.55;
const GLOW_SHARE: f32 = 0.4;
/// Scatter-only searchlight per beam, up from the landing spot: half
/// angles of its soft (inner) and outer edge, candela (unshadowed; tuned
/// at `sky night`) and reach.
const HAZE_INNER_DEG: f32 = 0.6;
const HAZE_OUTER_DEG: f32 = 2.2;
const HAZE_CD: f32 = 2.0e7;
const HAZE_RANGE: f32 = 60.0;
/// Nits of the core for Godot's `emission_energy_multiplier` 13 at alpha
/// 0.78 (its pulse 11 + 2.5 sin(4.2 t)).
const NITS: f32 = 5_000.0;
/// Shaft brightness in full daylight relative to the night (the floodlit
/// night is exposed brighter, the beam must hold up against the day sky).
pub(super) const DAY_GAIN: f32 = 2.5;
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

/// The shaft's texture, white with the falloff in alpha: across, a core
/// and a wide glow (both Gaussian, windowed to zero at the edge); along
/// (v = 0 at the sky), Godot's taper and a fade into the sky.
pub(super) fn picture() -> Picture {
    let (w, h) = (128u32, 256u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let taper = (TOP_RADIUS + (BOTTOM_RADIUS - TOP_RADIUS) * v) / BOTTOM_RADIUS;
        let sky_fade = (v / 0.12).min(1.0);
        for x in 0..w {
            let d = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * HALF_WIDTH;
            let g = |s: f32| (-(d / (s * taper)).powi(2)).exp();
            let window = (1.0 - (d / HALF_WIDTH).powi(2)).max(0.0).powi(2);
            let a = ((1.0 - GLOW_SHARE) * g(CORE) + GLOW_SHARE * g(GLOW)) * window * sky_fade * sky_fade;
            rgba.extend([255, 255, 255, (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
        }
    }
    Picture::new(w, h, rgba).expect("fixed size")
}

#[derive(Default)]
pub(super) struct Beams {
    lights: Vec<LocalLight>,
    haze: Vec<LocalLight>,
    sprites: Vec<Sprite>,
    /// Extra shaft brightness against the bright day sky (1 at night).
    pub gain: f32,
}

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
    pub fn reserve(_scene: &mut Scene) -> Self { Self { gain: DAY_GAIN, ..Self::default() } }

    pub fn pose(&mut self, _scene: &mut Scene, world: &World, clock: f32) {
        self.lights.clear();
        self.haze.clear();
        self.sprites.clear();
        let pulse = (clock * 4.2).sin();
        for p in &world.pickups {
            let land = landing(world, p);
            let color = linear(beam_color(p.kind));
            self.lights.push(LocalLight {
                position: land + Vec3::Y * 0.08, direction: Vec3::NEG_Y,
                intensity_cd: color * (LIGHT_ENERGY * ENERGY_CD * (1.0 + 0.15 * pulse)),
                range_m: LIGHT_RANGE, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.2, falloff: 1.0,
            });
            self.haze.push(LocalLight {
                position: land + Vec3::Y * 0.3, direction: Vec3::Y,
                intensity_cd: color * (HAZE_CD * (1.0 + 0.2 * pulse)),
                range_m: HAZE_RANGE, inner_cos: HAZE_INNER_DEG.to_radians().cos(), outer_cos: HAZE_OUTER_DEG.to_radians().cos(),
                source_radius_m: 0.2, falloff: 1.0,
            });
            // Godot's pulse: emission 11 + 2.5 sin(4.2 t) around 13.
            let span = (SKY_Y - land.y).max(0.5);
            let nits = NITS * self.gain * (11.0 + 2.5 * pulse) / 13.0;
            // Sunk 0.3 m so the floor cuts it, not the texture's bottom row.
            let centre = Vec3::new(land.x, land.y + span * 0.5 - 0.3, land.z);
            let mut s = Sprite::shaft(centre, [HALF_WIDTH * 2.0, span + 0.6], Vec3::Y, IMAGE, [0.0, 0.0, 1.0, 1.0], color * nits);
            s.depth_bias = 0.0;
            self.sprites.push(s);
        }
    }

    /// The ground lights, one per pickup.
    pub fn lights(&self) -> &[LocalLight] { &self.lights }
    /// The scatter-only haze spots, one per pickup.
    pub fn haze(&self) -> &[LocalLight] { &self.haze }
    /// The light shafts, one per pickup.
    pub fn sprites(&self) -> &[Sprite] { &self.sprites }
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
