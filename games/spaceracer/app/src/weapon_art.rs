//! Source weapon mounts, with only the actual joints kept as dynamic meshes.
use super::visuals::material;
use glam::{Affine3A, Quat, Vec3};
use render::{
    Scene,
    cluster::ClusterMesh,
    primitives::{self, Primitive},
    slots::{SlotId, Slots},
};
use spaceracer::{
    race::{Race, Weapon},
    race_rules::Pose,
};
#[derive(Default)]
struct Build(Vec<Primitive>);
impl Build {
    fn add(&mut self, mut p: Primitive, size: Vec3, rot: Quat, at: Vec3) {
        let xf = Affine3A::from_scale_rotation_translation(size, rot, at);
        for v in &mut p.vertices {
            v.pos = xf.transform_point3(Vec3::from(v.pos)).to_array();
            v.normal = (rot * (Vec3::from(v.normal) / size))
                .normalize_or_zero()
                .to_array();
        }
        self.0.push(p);
    }
    fn cube(&mut self, m: u32, size: [f32; 3], at: [f32; 3]) {
        self.add(primitives::cube(m), size.into(), Quat::IDENTITY, at.into());
    }
    fn cyl(&mut self, m: u32, r: f32, length: f32, at: [f32; 3], forward: bool) {
        self.add(
            primitives::cylinder(m, r, r, 8),
            Vec3::new(1., length, 1.),
            if forward {
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
            } else {
                Quat::IDENTITY
            },
            at.into(),
        );
    }
    fn sphere(&mut self, m: u32, r: f32, at: [f32; 3]) {
        self.add(
            primitives::sphere(m, 4, 8),
            Vec3::splat(r),
            Quat::IDENTITY,
            at.into(),
        );
    }
    fn ring(&mut self, m: u32, inner: f32, outer: f32, at: Vec3, forward: bool) {
        let mut p = Primitive {
            material: m,
            ..Default::default()
        };
        let radius = (inner + outer) * 0.5;
        let tube = (outer - inner) * 0.5;
        for j in 0..12 {
            for k in 0..3 {
                let point = |a: usize, b: usize| {
                    let u = a as f32 * std::f32::consts::TAU / 12.;
                    let v = b as f32 * std::f32::consts::TAU / 3.;
                    let n = Vec3::new(u.cos() * v.cos(), v.sin(), u.sin() * v.cos());
                    (
                        Vec3::new(u.cos() * radius, 0., u.sin() * radius) + n * tube,
                        n,
                    )
                };
                let q = [
                    point(j, k),
                    point(j + 1, k),
                    point(j + 1, k + 1),
                    point(j, k + 1),
                ];
                super::visuals::quad(&mut p, q.map(|x| x.0), q.map(|x| x.1), [[0.; 2]; 4]);
            }
        }
        self.add(
            p,
            Vec3::ONE,
            if forward {
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
            } else {
                Quat::IDENTITY
            },
            at,
        );
    }
    fn finish(self, scene: &mut Scene) -> usize {
        scene.add_mesh(&ClusterMesh::build(
            &self.0.iter().map(Primitive::part).collect::<Vec<_>>(),
        ))
    }
}
pub struct Art {
    pub base: usize,
    pub bomb: usize,
    pub coil: usize,
    head: usize,
    barrels: usize,
    rotor: usize,
    led: usize,
    core: usize,
    wings: usize,
    parts: Vec<[SlotId; 6]>,
    payload_wings: Vec<SlotId>,
    spins: [f32; 6],
    last_time: f32,
}
impl Art {
    pub fn new(scene: &mut Scene) -> Self {
        let steel = material(scene, [0.22, 0.27, 0.33], 0.34, 0.);
        let armor = material(scene, [0.35, 0.43, 0.52], 0.43, 0.);
        let black = material(scene, [0.014, 0.019, 0.025], 0.62, 0.);
        let edge = material(scene, [0.29, 0.36, 0.38], 0.3, 0.);
        let copper = material(scene, [0.28, 0.17, 0.075], 0.35, 0.);
        let mint = material(scene, [0.44, 1., 0.83], 0.35, 20.);
        let amber = material(scene, [0.87, 0.39, 0.06], 0.45, 0.);
        let violet = material(scene, [0.42, 0.2, 1.], 0.3, 15.);
        let mut b = Build::default();
        b.cube(steel, [0.88, 0.34, 1.1], [0., -0.22, -0.1]);
        b.cube(steel, [1.6, 0.25, 1.7], [0.; 3]);
        b.cyl(steel, 0.68, 0.6, [0., 0.3, 0.], false);
        b.ring(mint, 0.57, 0.69, Vec3::new(0., 0.58, 0.), false);
        let base = b.finish(scene);
        let mut b = Build::default();
        b.0.push(super::missile_mesh::loft(
            armor,
            &[
                [-0.8, 0.48, 0.6],
                [-0.5, 0.8, 0.85],
                [0.45, 0.7, 0.65],
                [0.75, 0.5, 0.45],
            ],
        ));
        b.cyl(steel, 0.34, 1.32, [-0.97, 0.02, -0.22], true);
        b.cube(steel, [0.42, 0.36, 0.64], [-0.66, -0.13, -0.1]);
        for z in [-0.70, -0.38, -0.06, 0.26] {
            b.cyl(edge, 0.39, 0.065, [-0.97, 0.02, z], true);
        }
        b.cyl(black, 0.3, 0.1, [-0.97, 0.02, -0.92], true);
        b.ring(copper, 0.24, 0.3, Vec3::new(-0.97, 0.02, -0.99), true);
        b.cyl(steel, 0.1, 0.18, [-0.97, 0.02, -1.], true);
        b.cube(black, [0.68, 0.88, 1.22], [1.02, 0.06, -0.04]);
        b.cube(steel, [0.4, 0.045, 0.88], [1.02, 0.495, -0.04]);
        b.cube(steel, [0.32, 0.27, 0.62], [0.72, -0.06, -0.05]);
        let path = [
            Vec3::new(1.02, 0.06, -0.65),
            Vec3::new(1.03, 0.06, -0.85),
            Vec3::new(0.98, 0.08, -1.08),
            Vec3::new(0.80, 0.10, -1.23),
            Vec3::new(0.56, 0.10, -1.25),
            Vec3::new(0.37, 0.10, -1.09),
            Vec3::new(0.34, 0.10, -0.82),
        ];
        for pair in path.windows(2) {
            let a = pair[0];
            let c = pair[1];
            let rot = Quat::from_rotation_arc(Vec3::Z, (c - a).normalize());
            b.add(
                primitives::cube(black),
                Vec3::new(0.27, 0.24, a.distance(c) + 0.035),
                rot,
                (a + c) * 0.5,
            );
            b.add(primitives::cube(steel), Vec3::new(0.29, 0.27, 0.04), rot, a);
        }
        b.cube(black, [0.82, 0.37, 0.09], [0., 0.035, -0.815]);
        for y in [-0.08, 0.02, 0.12] {
            b.cube(edge, [0.4, 0.03, 0.045], [-0.1, y, -0.875]);
        }
        b.sphere(mint, 0.18, [0., 0.18, 0.7]);
        b.cyl(steel, 0.2, 0.12, [0., 0.53, -0.35], false);
        let head = b.finish(scene);
        let mut b = Build::default();
        for j in 0..6 {
            let a = j as f32 * std::f32::consts::TAU / 6.;
            for (r, h, z) in [(0.105, 2.2, 1.5), (0.14, 0.25, 2.45)] {
                b.cyl(steel, r, h, [a.cos() * 0.31, a.sin() * 0.31, z], true);
            }
            b.cyl(
                black,
                0.087,
                0.018,
                [a.cos() * 0.31, a.sin() * 0.31, 2.585],
                true,
            );
        }
        b.ring(steel, 0.38, 0.49, Vec3::new(0., 0., 2.1), true);
        let barrels = b.finish(scene);
        let mut b = Build::default();
        b.cyl(copper, 0.09, 0.05, [0.; 3], true);
        for a in [0., 2.094, 4.189] {
            b.add(
                primitives::cube(edge),
                Vec3::new(0.065, 0.4, 0.04),
                Quat::from_rotation_z(a),
                Vec3::ZERO,
            );
        }
        let rotor = b.finish(scene);
        let mut b = Build::default();
        b.sphere(mint, 0.14, [0.; 3]);
        let led = b.finish(scene);
        let mut b = Build::default();
        b.cyl(steel, 0.72, 3.1, [0.; 3], true);
        b.sphere(armor, 0.72, [0., 0., 1.5]);
        b.cyl(amber, 0.75, 0.42, [0., 0., 0.85], true);
        for a in [0., std::f32::consts::FRAC_PI_2] {
            b.add(
                primitives::cube(steel),
                Vec3::new(2.5, 0.12, 1.15),
                Quat::from_rotation_z(a),
                Vec3::new(0., 0., -1.45),
            );
        }
        b.cyl(black, 0.35, 0.1, [0., 0., -1.6], true);
        b.sphere(amber, 0.16, [0., 0.73, 0.8]);
        let bomb = b.finish(scene);
        let mut b = Build::default();
        for side in [-1., 1.] {
            let rot = Quat::from_rotation_y(side * 0.24);
            let at = Vec3::new(side * 2., 0.12, -0.2);
            b.add(primitives::cube(steel), Vec3::new(3.4, 0.13, 1.05), rot, at);
            b.add(
                primitives::cube(amber),
                Vec3::new(0.12, 0.15, 0.6),
                rot,
                at + rot * Vec3::new(side * 1.55, 0., 0.),
            );
        }
        let wings = b.finish(scene);
        let mut b = Build::default();
        b.cube(steel, [1.5, 0.35, 2.2], [0.; 3]);
        for z in [-0.66, 0., 0.66] {
            b.ring(steel, 0.68, 0.83, Vec3::new(0., 0.65, z), true);
            b.ring(violet, 0.58, 0.64, Vec3::new(0., 0.65, z), true);
        }
        for side in [-1., 1.] {
            b.cube(steel, [0.16, 0.75, 1.8], [side * 0.67, 0.4, 0.]);
        }
        let coil = b.finish(scene);
        let mut b = Build::default();
        b.sphere(violet, 0.55, [0.; 3]);
        let core = b.finish(scene);
        Self {
            base,
            bomb,
            coil,
            head,
            barrels,
            rotor,
            led,
            core,
            wings,
            parts: vec![],
            payload_wings: vec![],
            spins: [0.; 6],
            last_time: 0.,
        }
    }
    pub fn reserve(&mut self, scene: &mut Scene, slots: &mut Slots) {
        let hidden = Affine3A::from_scale(Vec3::splat(0.001));
        self.parts = (0..6)
            .map(|_| {
                [
                    self.head,
                    self.barrels,
                    self.rotor,
                    self.led,
                    self.core,
                    self.wings,
                ]
                .map(|m| slots.add(scene, m, hidden).unwrap())
            })
            .collect();
        self.payload_wings = (0..32)
            .map(|_| slots.add(scene, self.wings, hidden).unwrap())
            .collect();
    }
    pub fn update(
        &mut self,
        scene: &mut Scene,
        slots: &mut Slots,
        race: &Race,
        poses: &[Pose],
        time: f32,
        alpha: f32,
    ) {
        let dt = (time - self.last_time).clamp(0., 0.1);
        self.last_time = time;
        let hidden = Affine3A::from_scale(Vec3::splat(0.001));
        for (i, pose) in poses.iter().enumerate() {
            let d = &race.drivers[i];
            let p = &race.pilots[i];
            let kind = d.active.or(d.item);
            let mount = Affine3A::from_mat3_translation(
                pose.basis,
                pose.origin + pose.basis * Vec3::new(0., 1.15, -1.8),
            );
            let mut transforms = [hidden; 6];
            if !p.crashed && p.recovery<=0. {
                match kind {
                    Some(Weapon::Sentry) => {
                        let active = d.active == Some(Weapon::Sentry) && p.emp_time <= 0.;
                        self.spins[i] += dt
                            * if active {
                                if d.sentry_target.is_some() { 38. } else { 9. }
                            } else {
                                0.
                            };
                        let local = race.pose(i).basis.transpose() * race.sentry_basis(i);
                        let head =
                            mount * Affine3A::from_mat3_translation(local, Vec3::new(0., 0.8, 0.));
                        transforms[0] = head;
                        let recoil = if race.clock - d.shot_time + self::sentry_interval(race)
                            < 0.018
                            && d.sentry_target.is_some()
                        {
                            -0.07
                        } else {
                            0.
                        };
                        transforms[1] = head
                            * Affine3A::from_rotation_translation(
                                Quat::from_rotation_z(self.spins[i]),
                                Vec3::new(0., 0., recoil),
                            );
                        transforms[2] = head
                            * Affine3A::from_rotation_translation(
                                Quat::from_rotation_z(-self.spins[i] * 1.8),
                                Vec3::new(-0.97, 0.02, -1.025),
                            );
                        if active && (time + i as f32 * 0.11).rem_euclid(1.2) < 0.18 {
                            transforms[3] =
                                head * Affine3A::from_translation(Vec3::new(0., 0.63, -0.35));
                        }
                    }
                    Some(Weapon::Warp) => {
                        transforms[4] = mount
                            * Affine3A::from_scale_rotation_translation(
                                Vec3::splat(
                                    1. + (time * if d.warp_fx > 0.01 { 10. } else { 2.5 }).sin()
                                        * 0.08,
                                ),
                                Quat::IDENTITY,
                                Vec3::new(0., 0.6, 0.),
                            );
                    }
                    Some(Weapon::GlideBomb) => {
                        transforms[5] = mount * Affine3A::from_scale(Vec3::new(0.22, 1., 1.));
                    }
                    _ => {}
                }
            }
            for (k, m) in [
                self.head,
                self.barrels,
                self.rotor,
                self.led,
                self.core,
                self.wings,
            ]
            .into_iter()
            .enumerate()
            {
                slots.set(scene, self.parts[i][k], m, transforms[k]);
            }
        }
        for (k, slot) in self.payload_wings.iter().enumerate() {
            let xf = race
                .projectiles
                .get(k)
                .filter(|p| p.kind == Weapon::GlideBomb)
                .map_or(hidden, |p| {
                    let t = ((p.time as f32 - 0.05) / 0.33).clamp(0., 1.);
                    Affine3A::from_scale_rotation_translation(
                        Vec3::new(0.22 + 0.78 * t * t * (3. - 2. * t), 1., 1.),
                        Quat::from_rotation_arc(Vec3::Z, p.velocity.normalize_or_zero()),
                        p.previous.lerp(p.position, alpha),
                    )
                });
            slots.set(scene, *slot, self.wings, xf);
        }
    }
}
fn sentry_interval(race: &Race) -> f64 {
    race.balance.sentry_interval
}
