//! What lives in the world besides players and the arena: grenades,
//! cluster fragments, mines and dash bombs, pickups, air strike rockets and
//! markers and the lava leak. The ion cannon uses GPU effects in `ion`.
use super::{cube, glow, material, sphere};
use glam::{Affine3A, Quat, Vec3};
use glam::Vec2;
use growing_guns::{ProjectileKind, StrikeKind, World, AIR_STRIKE_TRAVEL, PICKUPS};
use render::scene::NO_SHADOW;
use render::slots::Slots;
use render::Scene;
use render::headlights::LocalLight;

#[path = "airstrike.rs"]
mod airstrike;
#[path = "dropbeam.rs"]
mod dropbeam;

const SLOTS: usize = 192;

pub struct Things {
    first: usize,
    grenade: usize,
    cluster: usize,
    fragment: usize,
    mine: usize,
    mine_light: usize,
    bomb: usize,
    beacon: usize,
    pickups: [usize; 7],
    marker: usize,
    rocket: usize,
    lava: usize,
    lasers: airstrike::Lasers,
    beams: dropbeam::Beams,
}

impl Things {
    /// The pickups' ground lights (Godot `DropLight`).
    pub fn pickup_lights(&self) -> &[LocalLight] { self.beams.lights() }
    /// The pickups' scatter-only haze over their beams (night air).
    pub fn pickup_haze(&self) -> &[LocalLight] { self.beams.haze() }

    pub fn reserve(scene: &mut Scene) -> Self {
        let dark = material(scene, [0.12, 0.14, 0.12], 0.5);
        let orange = material(scene, [0.55, 0.25, 0.08], 0.5);
        let mine_m = material(scene, [0.08, 0.1, 0.1], 0.4);
        let red = glow(scene, [1.0, 0.15, 0.05], 30000.0);
        let teal = glow(scene, [0.0, 0.95, 0.75], 20000.0);
        let lava_m = super::arena::lava_material(scene);
        let first = Slots::reserve(scene, SLOTS).first;
        for k in 0..SLOTS { scene.instances[first + k].cluster_count = 0; }
        Things {
            first,
            grenade: sphere(scene, dark, 6, 10),
            cluster: sphere(scene, orange, 6, 10),
            fragment: sphere(scene, orange, 4, 8),
            mine: sphere(scene, mine_m, 4, 10),
            mine_light: sphere(scene, teal, 4, 8),
            bomb: sphere(scene, red, 6, 10),
            beacon: sphere(scene, red, 4, 8),
            pickups: PICKUPS.map(|k| dropbeam::pickup_mesh(scene, k)),
            marker: { let m = glow(scene, [1.0, 0.1, 0.05], 12000.0); scene.add_mesh(&render::primitives::torus(m, 0.04, 24, 6).mesh()) },
            rocket: { let m = glow(scene, [1.0, 0.55, 0.2], 40000.0); cube(scene, m) },
            lava: cube(scene, lava_m),
            lasers: airstrike::Lasers::reserve(scene),
            beams: dropbeam::Beams::reserve(scene),
        }
    }

    pub fn pose(&mut self, scene: &mut Scene, world: &World, clock: f32) {
        let mut n = 0;
        let mut put = |scene: &mut Scene, mesh: usize, xf: Affine3A, shadow: bool| {
            if n >= SLOTS { return; }
            scene.update_instance(self.first + n, mesh, xf);
            let i = &mut scene.instances[self.first + n];
            if shadow { i.flags &= !NO_SHADOW; } else { i.flags |= NO_SHADOW; }
            n += 1;
        };
        let at = |p: Vec3, s: Vec3| Affine3A::from_scale_rotation_translation(s, Quat::IDENTITY, p);
        for g in &world.projectiles {
            let blink = (g.age as f32 * 8.0).fract() < 0.5;
            match g.kind {
                ProjectileKind::Grenade => { put(scene, self.grenade, at(g.pos, Vec3::splat(0.2)), true); if blink { put(scene, self.beacon, at(g.pos + Vec3::Y * 0.17, Vec3::splat(0.05)), false); } }
                ProjectileKind::Cluster => { put(scene, self.cluster, at(g.pos, Vec3::splat(0.244)), true); if blink { put(scene, self.beacon, at(g.pos + Vec3::Y * 0.2, Vec3::splat(0.06)), false); } }
                ProjectileKind::Fragment => put(scene, self.fragment, at(g.pos, Vec3::splat(0.11)), true),
                ProjectileKind::Mine => {
                    put(scene, self.mine, at(g.pos, Vec3::new(0.27, 0.07, 0.27)), true);
                    if g.age >= 0.7 { put(scene, self.mine_light, at(g.pos + Vec3::Y * 0.06, Vec3::splat(if blink { 0.06 } else { 0.04 })), false); }
                }
                ProjectileKind::DashBomb => put(scene, self.bomb, at(g.pos, Vec3::splat(0.2 + 0.05 * (g.age as f32 * 12.0).sin().abs())), false),
            }
        }
        for p in &world.pickups {
            let k = PICKUPS.iter().position(|&q| q == p.kind).unwrap_or(0);
            let bob = (clock * 4.2 + p.pos.x).sin() * 0.08; // Godot BOB_SPEED, BOB_AMOUNT
            let xf = Affine3A::from_scale_rotation_translation(Vec3::ONE, Quat::from_rotation_y(clock * 1.8), p.pos + Vec3::Y * bob);
            put(scene, self.pickups[k], xf, true);
        }
        for s in &world.strikes {
            match s.kind {
                StrikeKind::AirStrike => {
                    if !airstrike::active(s) { continue; }
                    let t = (s.age / AIR_STRIKE_TRAVEL) as f32;
                    let sky = s.target + airstrike::SKY_OFFSET;
                    let pos = sky.lerp(s.target, t * t);
                    let dir = (s.target - sky).normalize();
                    put(scene, self.rocket, Affine3A::from_scale_rotation_translation(Vec3::new(0.35, 2.2, 0.35), Quat::from_rotation_arc(Vec3::Y, -dir), pos), false);
                }
                StrikeKind::IonCannon => {}
            }
        }
        // Coop: where the next enemy rises (Godot's pentagram), and the
        // revive circle where a downed teammate fell.
        if let Some(inc) = world.waves.incoming.as_ref().filter(|_| world.coop) {
            let grow = (1.0 - inc.left as f32 / growing_guns::coop::TELEGRAPH as f32).clamp(0.2, 1.0);
            let r = 1.6 * grow * (1.0 + 0.12 * (clock * 14.0).sin());
            for k in [1.0, 0.7, 0.4] { put(scene, self.marker, at(inc.ground + Vec3::Y * 0.06, Vec3::new(r * k, 1.0, r * k)), false); }
            // The beam over the spot (Godot's pentagram column).
            put(scene, self.rocket, at(inc.ground + Vec3::Y * 6.0, Vec3::new(0.12 * grow, 12.0, 0.12 * grow)), false);
        }
        for d in world.waves.downed.iter().filter(|_| world.coop) {
            let r = growing_guns::coop::REVIVE_RADIUS * (1.0 + 0.05 * (clock * 4.0).sin());
            put(scene, self.marker, at(d.corpse - Vec3::Y * (growing_guns::HALF_HEIGHT - 0.06), Vec3::new(r, 1.0, r)), false);
        }
        // The leak as Godot builds it (`arena_procedural.gd`): four 4.5 cm
        // floor fronts centred on the surface, creeping in from the edges
        // (8 cm wide as soon as they show), and 45 cm sheets down the inside
        // of the walls, growing from the top.
        let l = &world.lava;
        // An island arena's floor: Godot's LavaFloorSurface, a plane the size
        // of the arena at 0.04 m. Godot leaves it there while the leak rises
        // (and hides the leak's fronts), so the rising lava that kills is
        // invisible; here the floor rises with it.
        if world.arena.variant == growing_guns::arena::Variant::Island {
            let y = floor_lava_y(world);
            let s = world.arena.size;
            put(scene, self.lava, at(Vec3::new(0.0, y - 0.05, 0.0), Vec3::new(s, 0.1, s)), false);
        }
        if l.active && !l.full {
            let (half, y) = (l.half, l.surface());
            let shown = l.floor_t() > 0.0 || !l.has_walls;
            if shown {
                for (c, s) in lava_fronts(half, l.inset().max(0.08).min(half)) {
                    if s.x > 0.01 && s.y > 0.01 { put(scene, self.lava, at(Vec3::new(c.x, y, c.y), Vec3::new(s.x, LAVA_FRONT_THICKNESS, s.y)), false); }
                }
            }
            if l.has_walls {
                let top = growing_guns::arena::WALL_HEIGHT;
                let h = 0.05 + (top - 0.05) * l.wall_t();
                for (c, s) in wall_sheets(half) { put(scene, self.lava, at(Vec3::new(c.x, top - h * 0.5, c.y), Vec3::new(s.x, h, s.y)), false); }
            }
        }
        for k in n..SLOTS {
            let i = &mut scene.instances[self.first + k];
            if i.cluster_count == 0 { break; }
            i.cluster_count = 0;
            i.sphere = [0.0; 4];
        }
        self.lasers.pose(scene, &world.strikes);
        self.beams.pose(scene, world, clock);
    }
}

const LAVA_FRONT_THICKNESS: f32 = 0.045;

/// The drawn surface of an island arena's lava floor: 0.04 m, or the leak's
/// surface once it has risen past that.
pub fn floor_lava_y(world: &World) -> f32 {
    let l = &world.lava;
    let leak = if l.active { l.surface() } else { f32::MIN };
    growing_guns::arena::LAVA_FLOOR_Y.max(leak)
}

/// The four floor fronts the leak has covered, `inset` in from each edge:
/// centre and size in XZ (north and south span the side, east and west the
/// rest, as Godot's `_set_front_rect`).
fn lava_fronts(half: f32, inset: f32) -> [(Vec2, Vec2); 4] {
    let side = half * 2.0;
    let centre = (side - inset * 2.0).max(0.0);
    let mid = half - inset * 0.5;
    [
        (Vec2::new(0.0, -mid), Vec2::new(side, inset)), (Vec2::new(0.0, mid), Vec2::new(side, inset)),
        (Vec2::new(-mid, 0.0), Vec2::new(inset, centre)), (Vec2::new(mid, 0.0), Vec2::new(inset, centre)),
    ]
}

/// Lava running down the inside of the four walls (Godot `LavaWallFlow`).
fn wall_sheets(half: f32) -> [(Vec2, Vec2); 4] {
    let d = half - 0.62;
    [
        (Vec2::new(d, 0.0), Vec2::new(0.45, half * 2.0)), (Vec2::new(-d, 0.0), Vec2::new(0.45, half * 2.0)),
        (Vec2::new(0.0, d), Vec2::new(half * 2.0, 0.45)), (Vec2::new(0.0, -d), Vec2::new(half * 2.0, 0.45)),
    ]
}
