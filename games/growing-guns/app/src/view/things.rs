//! What lives in the world besides players and the arena: grenades,
//! cluster fragments, mines and dash bombs, pickups, air strike rockets and
//! markers, the ion cannon column and the lava leak.
use super::{cube, glow, material, sphere};
use glam::{Affine3A, Quat, Vec3};
use glam::Vec2;
use growing_guns::{PickupKind, ProjectileKind, StrikeKind, World, AIR_STRIKE_TRAVEL, ION_CHARGE, PICKUPS};
use render::scene::{Material, NO_SHADOW};
use render::slots::Slots;
use render::Scene;

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
    ion: usize,
    ion_core: usize,
    lava: usize,
}

impl Things {
    pub fn reserve(scene: &mut Scene) -> Self {
        let dark = material(scene, [0.12, 0.14, 0.12], 0.5);
        let orange = material(scene, [0.55, 0.25, 0.08], 0.5);
        let mine_m = material(scene, [0.08, 0.1, 0.1], 0.4);
        let red = glow(scene, [1.0, 0.15, 0.05], 30000.0);
        let teal = glow(scene, [0.0, 0.95, 0.75], 20000.0);
        let pick = |scene: &mut Scene, c: [f32; 3]| { let m = glow(scene, c, 9000.0); cube(scene, m) };
        let lava_m = scene.add_material(Material { tint: [1.0; 3], layer: -1, roughness: 1.0, flags: render::scene::LAVA, scale: 12000.0, metal: 0.0 });
        let ion_m = glow(scene, [0.35, 0.72, 1.0], 6000.0);
        let ion_core_m = glow(scene, [0.8, 0.95, 1.0], 60000.0);
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
            pickups: [
                pick(scene, [1.0, 0.2, 0.25]), pick(scene, [0.9, 0.3, 0.9]), pick(scene, [1.0, 0.45, 0.08]), pick(scene, [1.0, 0.8, 0.2]),
                pick(scene, [0.3, 0.9, 1.0]), pick(scene, [1.0, 1.0, 1.0]), pick(scene, [1.0, 0.28, 0.12]),
            ],
            marker: { let m = glow(scene, [1.0, 0.1, 0.05], 12000.0); scene.add_mesh(&render::primitives::torus(m, 0.04, 24, 6).mesh()) },
            rocket: { let m = glow(scene, [1.0, 0.55, 0.2], 40000.0); cube(scene, m) },
            ion: scene.add_mesh(&render::primitives::cylinder(ion_m, 1.0, 1.0, 24).mesh()),
            ion_core: scene.add_mesh(&render::primitives::cylinder(ion_core_m, 1.0, 1.0, 12).mesh()),
            lava: cube(scene, lava_m),
        }
    }

    pub fn pose(&self, scene: &mut Scene, world: &World, clock: f32) {
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
            let bob = (clock * 2.4 + p.pos.x).sin() * 0.12;
            let xf = Affine3A::from_scale_rotation_translation(Vec3::splat(if p.kind == PickupKind::Mushroom { 0.6 } else { 0.45 }), Quat::from_rotation_y(clock * 1.8), p.pos + Vec3::Y * bob);
            put(scene, self.pickups[k], xf, true);
        }
        for s in &world.strikes {
            match s.kind {
                StrikeKind::AirStrike => {
                    let t = (s.age / AIR_STRIKE_TRAVEL) as f32;
                    let pulse = 1.0 + 0.15 * (clock * 10.0).sin();
                    put(scene, self.marker, at(s.target + Vec3::Y * 0.1, Vec3::new(3.0 * pulse, 1.0, 3.0 * pulse)), false);
                    let sky = s.target + Vec3::new(0.0, 263.0, 93.0);
                    let pos = sky.lerp(s.target, t * t);
                    let dir = (s.target - sky).normalize();
                    put(scene, self.rocket, Affine3A::from_scale_rotation_translation(Vec3::new(0.35, 2.2, 0.35), Quat::from_rotation_arc(Vec3::Y, -dir), pos), false);
                }
                StrikeKind::IonCannon => {
                    let t = (s.age / ION_CHARGE) as f32;
                    let r = 30.0 + (1.5 - 30.0) * t;
                    put(scene, self.ion, at(s.target + Vec3::Y * 200.0, Vec3::new(r, 400.0, r)), false);
                    put(scene, self.ion_core, at(s.target + Vec3::Y * 200.0, Vec3::new(0.3 + t, 400.0, 0.3 + t)), false);
                }
            }
        }
        let l = &world.lava;
        if l.active {
            let (half, inset, y) = (l.half, l.inset(), l.surface());
            if inset > 0.01 {
                for (c, s) in lava_strips(half, inset) { put(scene, self.lava, at(Vec3::new(c.x, y - 0.1, c.y), Vec3::new(s.x, 0.2, s.y)), false); }
            }
            if l.rise > 0.0 { put(scene, self.lava, at(Vec3::new(0.0, y - 0.1, 0.0), Vec3::new(half * 2.0, 0.2, half * 2.0)), false); }
            if l.has_walls {
                let h = growing_guns::arena::WALL_HEIGHT * l.wall_t();
                let top = growing_guns::arena::WALL_HEIGHT;
                for (c, s) in wall_sheets(half) { put(scene, self.lava, at(Vec3::new(c.x, top - h * 0.5, c.y), Vec3::new(s.x, h.max(0.01), s.y)), false); }
            }
        }
        for k in n..SLOTS {
            let i = &mut scene.instances[self.first + k];
            if i.cluster_count == 0 { break; }
            i.cluster_count = 0;
            i.sphere = [0.0; 4];
        }
    }
}

/// The four floor bands the leak has covered: centre and size in XZ.
fn lava_strips(half: f32, inset: f32) -> [(Vec2, Vec2); 4] {
    let mid = half - inset * 0.5;
    [
        (Vec2::new(mid, 0.0), Vec2::new(inset, half * 2.0)), (Vec2::new(-mid, 0.0), Vec2::new(inset, half * 2.0)),
        (Vec2::new(0.0, mid), Vec2::new(half * 2.0, inset)), (Vec2::new(0.0, -mid), Vec2::new(half * 2.0, inset)),
    ]
}

/// Lava running down the inside of the four walls.
fn wall_sheets(half: f32) -> [(Vec2, Vec2); 4] {
    let d = half - 0.55;
    [
        (Vec2::new(d, 0.0), Vec2::new(0.1, half * 2.0)), (Vec2::new(-d, 0.0), Vec2::new(0.1, half * 2.0)),
        (Vec2::new(0.0, d), Vec2::new(half * 2.0, 0.1)), (Vec2::new(0.0, -d), Vec2::new(half * 2.0, 0.1)),
    ]
}
