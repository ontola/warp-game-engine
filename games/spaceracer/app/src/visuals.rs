use glam::{Affine3A, Mat3, Quat, Vec3};
use render::{
    Lighting, Scene,
    cluster::Vertex,
    primitives::{self, Primitive},
    scene::{EMISSIVE, Material, SURFACE, THIN_FOLIAGE},
    slots::{SlotId, Slots},
};
use serde_json::Value;
use spaceracer::race::{Race, Weapon};
use std::{io::Read, path::Path};

pub const COLORS: [[f32; 3]; 6] = [
    [0.7, 0.035, 0.055],
    [0.025, 0.5, 0.7],
    [0.8, 0.45, 0.015],
    [0.25, 0.05, 0.7],
    [0.08, 0.55, 0.18],
    [0.6, 0.2, 0.03],
];
pub struct ShipPart {
    pub mesh: usize,
    pub slot: SlotId,
    pub chain: Vec<(String, Affine3A)>,
}
pub struct Visuals {
    pub scene: Scene,
    pub lighting: Lighting,
    pub lamps: Vec<render::headlights::LocalLight>,
    pub obstacles: Vec<spaceracer::environment::Obstacle>,
    pub slots: Slots,
    pub ships: Vec<Vec<ShipPart>>,
    pub jets: Vec<(usize, SlotId)>,
    pub pickup_mesh: usize,
    pub pickup_slots: Vec<SlotId>,
    pub projectile_mesh: usize,
    pub projectiles: Vec<SlotId>,
    pub mounts: Vec<SlotId>,
    pub mount_meshes: Vec<usize>,
    pub uploaded: bool,
    pub fx: render::fx::Effects,
    pub crashes: Vec<u64>,
    pub blast_stamp: f64,
    pub fx_stamp: f32,
}
fn material(scene: &mut Scene, tint: [f32; 3], rough: f32, emission: f32) -> u32 {
    scene.add_material(Material {
        tint,
        layer: -1,
        roughness: rough,
        metal: if rough < 0.5 { 0.5 } else { 0. },
        flags: if emission > 0. { EMISSIVE } else { 0 },
        scale: emission.max(1.),
    })
}
fn box_at(scene: &mut Scene, mesh: usize, pos: Vec3, size: Vec3, rotation: Quat) {
    scene.add_instance(
        mesh,
        Affine3A::from_scale_rotation_translation(size, rotation, pos),
    );
}
fn quad(p: &mut Primitive, points: [Vec3; 4], normal: [Vec3; 4], uv: [[f32; 2]; 4]) {
    let k = p.vertices.len() as u32;
    for i in 0..4 {
        p.vertices.push(Vertex {
            pos: points[i].to_array(),
            normal: normal[i].to_array(),
            uv: uv[i],
        });
    }
    let mut ids = [k, k + 1, k + 2, k, k + 2, k + 3];
    if (points[1] - points[0])
        .cross(points[2] - points[0])
        .dot(normal[0])
        < 0.
    {
        ids = [k, k + 2, k + 1, k, k + 3, k + 2];
    }
    p.indices.extend(ids);
}
fn surface(scene: &mut Scene, kind: i32, tint: [f32; 3], rough: f32, scale: f32) -> u32 {
    scene.add_material(Material {
        tint,
        layer: -2 - kind,
        roughness: rough,
        metal: 0.0,
        flags: SURFACE,
        scale,
    })
}
fn jet_mesh(scene: &mut Scene, night: bool) -> usize {
    let mask = image::RgbaImage::from_fn(64, 256, |x, y| {
        let u = x as f32 / 63. * 2. - 1.;
        let v = ((y as f32 - 12.) / 227.).clamp(0., 1.);
        let width = (1. - v).powf(0.6) * (0.85 + 0.025 * (v * 32.).sin());
        let a = (1. - (u / width.max(0.01)).powi(2)).max(0.).powf(2.) * (1. - v).powf(1.2);
        image::Rgba([
            155,
            210,
            255,
            if y < 12 || y > 239 {
                0
            } else {
                (a * 200.) as u8
            },
        ])
    });
    let layer = scene.add_image("spaceracer-plasma-profile", mask);
    let mat = scene.add_material(Material {
        tint: [0.2, 0.5, 1.],
        layer,
        roughness: 1.,
        metal: 0.,
        flags: EMISSIVE,
        scale: if night { 650. } else { 35000. },
    });
    scene.set_material_additive(mat, 0.65, true);
    let mut p = Primitive {
        material: mat,
        ..Default::default()
    };
    for k in 0..3 {
        let a = k as f32 * std::f32::consts::PI / 3.;
        let r = Vec3::new(a.cos(), a.sin(), 0.) * 0.7;
        let tail = -Vec3::Z;
        quad(
            &mut p,
            [-r, r, r * 0.005 + tail, -r * 0.005 + tail],
            [Vec3::new(-a.sin(), a.cos(), 0.); 4],
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
        );
    }
    scene.add_mesh(&p.mesh())
}
fn track_lights(scene: &mut Scene, race: &Race) -> Vec<render::headlights::LocalLight> {
    let mut lights = Vec::new();
    if race.biome != "city" {
        return lights;
    }
    let steel = material(scene, [0.055, 0.065, 0.08], 0.65, 0.);
    let glow = material(scene, [0.65, 0.8, 1.], 0.5, 90.);
    let pole = scene.add_mesh(&primitives::cube(steel).mesh());
    let lamp = scene.add_mesh(&primitives::cube(glow).mesh());
    for d in (0..race.world.track.length as usize).step_by(90) {
        let n = race.world.track.sample(d as f64);
        if n.air_gap || !n.rails || n.geometry.shape_angle > 1. {
            continue;
        }
        for side in [-1., 1.] {
            let x = side * (n.geometry.width + 0.8);
            let basis = n.frame(x);
            let rot = Quat::from_mat3(&basis);
            box_at(scene, pole, n.point(x, 4.), Vec3::new(0.28, 8., 0.28), rot);
            box_at(
                scene,
                pole,
                n.point(x - side * 2., 8.),
                Vec3::new(4.3, 0.24, 0.4),
                rot,
            );
            let at = n.point(x - side * 3.5, 7.8);
            box_at(scene, lamp, at, Vec3::new(1.7, 0.12, 0.7), rot);
            lights.push(render::headlights::LocalLight {
                position: at - basis.y_axis * 0.2,
                direction: -basis.y_axis,
                intensity_cd: Vec3::new(0.65, 0.8, 1.) * 2200.,
                range_m: 65.,
                inner_cos: 0.8,
                outer_cos: 0.05,
                source_radius_m: 0.8,
                falloff: 2.,
            });
        }
    }
    lights
}
fn road(scene: &mut Scene, race: &Race) {
    let asphalt = surface(scene, 0, [0.035, 0.045, 0.055], 0.35, 1.);
    let edge = material(scene, [0.07, 0.65, 0.95], 0.3, 140.);
    let rail = material(scene, [0.08, 0.1, 0.14], 0.4, 0.);
    let boost = material(scene, [0.1, 0.8, 0.9], 0.3, 80.);
    let charge = material(scene, [0.2, 0.9, 0.48], 0.3, 60.);
    let nodes = &race.world.track.nodes;
    for start in (0..nodes.len()).step_by(64) {
        let mut parts: Vec<_> = [asphalt, edge, rail, boost, charge]
            .map(|material| Primitive {
                material,
                ..Default::default()
            })
            .into();
        for i in start..(start + 64).min(nodes.len()) {
            let a = &nodes[i];
            let b = &nodes[(i + 1) % nodes.len()];
            if a.air_gap || b.air_gap {
                continue;
            }
            for lane in 0..16 {
                let u = -1. + lane as f64 / 8.;
                let v = u + 0.125;
                let x = (u + v) * 0.5 * a.geometry.width;
                if !a.supported(x, 0.) {
                    continue;
                }
                let ax = u * a.geometry.width;
                let bx = v * a.geometry.width;
                let cx = v * b.geometry.width;
                let dx = u * b.geometry.width;
                let top = [
                    a.point(ax, 0.),
                    a.point(bx, 0.),
                    b.point(cx, 0.),
                    b.point(dx, 0.),
                ];
                let norms = [
                    a.frame(ax).y_axis,
                    a.frame(bx).y_axis,
                    b.frame(cx).y_axis,
                    b.frame(dx).y_axis,
                ];
                quad(
                    &mut parts[0],
                    top,
                    norms,
                    [
                        [
                            u as f32,
                            i as f32 * (race.world.track.length / nodes.len() as f64) as f32,
                        ],
                        [
                            v as f32,
                            i as f32 * (race.world.track.length / nodes.len() as f64) as f32,
                        ],
                        [
                            v as f32,
                            (i + 1) as f32 * (race.world.track.length / nodes.len() as f64) as f32,
                        ],
                        [
                            u as f32,
                            (i + 1) as f32 * (race.world.track.length / nodes.len() as f64) as f32,
                        ],
                    ],
                );
                if a.zone == "repair" && u >= -0.88 && v <= 0.3 && i % 3 == 0 {
                    quad(
                        &mut parts[4],
                        std::array::from_fn(|j| top[j] + norms[j] * 0.035),
                        norms,
                        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
                    );
                }
            }
            if a.zone == "boost" && i % 3 == 0 {
                let d = (i as f64) * race.world.track.length / nodes.len() as f64;
                for side in [-1., 1.] {
                    let points = [
                        (side * a.geometry.width * 0.3, d),
                        (0., d + 6.),
                        (0., d + 8.),
                        (side * a.geometry.width * 0.3, d + 2.),
                    ]
                    .map(|(x, d)| race.world.track.sample(d).point(x, 0.05));
                    quad(
                        &mut parts[3],
                        points,
                        [a.geometry.basis().y_axis; 4],
                        [[0.; 2]; 4],
                    );
                }
            }
            for side in [-1., 1.] {
                let x = side * a.geometry.width;
                let y = side * b.geometry.width;
                let normal = [
                    a.frame(x).y_axis,
                    a.frame(x).y_axis,
                    b.frame(y).y_axis,
                    b.frame(y).y_axis,
                ];
                quad(
                    &mut parts[1],
                    [
                        a.point(x, 0.08),
                        a.point(x - side * 0.35, 0.08),
                        b.point(y - side * 0.35, 0.08),
                        b.point(y, 0.08),
                    ],
                    normal,
                    [[0.; 2]; 4],
                );
                quad(
                    &mut parts[2],
                    [
                        a.point(x, -2.5),
                        a.point(x, 0.),
                        b.point(y, 0.),
                        b.point(y, -2.5),
                    ],
                    [a.frame(x).x_axis * side as f32; 4],
                    [[0.; 2]; 4],
                );
                if a.rails && a.geometry.shape_angle < 3. {
                    quad(
                        &mut parts[2],
                        [
                            a.point(x, 0.),
                            a.point(x, 2.4),
                            b.point(y, 2.4),
                            b.point(y, 0.),
                        ],
                        [-a.frame(x).x_axis * side as f32; 4],
                        [[0.; 2]; 4],
                    );
                    quad(
                        &mut parts[1],
                        [
                            a.point(x, 2.45),
                            a.point(x - side * 0.3, 2.45),
                            b.point(y - side * 0.3, 2.45),
                            b.point(y, 2.45),
                        ],
                        normal,
                        [[0.; 2]; 4],
                    );
                }
            }
            if a.tunnel && a.geometry.shape_angle < 0.1 {
                for k in 0..12 {
                    let x = k as f32 * std::f32::consts::PI / 12.;
                    let y = (k + 1) as f32 * std::f32::consts::PI / 12.;
                    let point = |n: &spaceracer::track::Node, t: f32| {
                        n.geometry.p
                            + n.geometry.basis()
                                * Vec3::new(
                                    t.cos() * (n.geometry.width as f32 + 4.),
                                    t.sin() * 30.,
                                    0.,
                                )
                    };
                    let center = (a.geometry.p + b.geometry.p) * 0.5;
                    let pos = [point(a, x), point(a, y), point(b, y), point(b, x)];
                    let normals = std::array::from_fn(|j| (center - pos[j]).normalize_or_zero());
                    quad(&mut parts[2], pos, normals, [[0.; 2]; 4]);
                    if i % 12 == 0 {
                        quad(
                            &mut parts[1],
                            pos.map(|v| v + (center - v).normalize_or_zero() * 0.08),
                            normals,
                            [[0.; 2]; 4],
                        );
                    }
                }
            }
        }
        for p in parts {
            if !p.indices.is_empty() {
                let mesh = scene.add_mesh(&p.mesh());
                scene.add_instance(mesh, Affine3A::IDENTITY);
            }
        }
    }
}
fn rock_mesh(scene: &mut Scene, material: u32, variant: usize) -> usize {
    let mut p = primitives::sphere(material, 12, 18);
    for v in &mut p.vertices {
        let mut q = Vec3::from(v.pos);
        let band = 1. + 0.12 * (q.y * 8. + variant as f32).sin();
        let wobble = 1. + 0.12 * (q.x * 11. + q.z * 7. + variant as f32 * 2.).sin();
        q.x = q.x.signum() * q.x.abs().powf(0.65) * band * wobble;
        q.z = q.z.signum() * q.z.abs().powf(0.75) * band;
        q.y = q.y.clamp(-0.8, 0.65 + variant as f32 * 0.05) + q.x * 0.12;
        v.pos = q.to_array();
    }
    let mut out = Primitive {
        material,
        ..Default::default()
    };
    for tri in p.indices.chunks_exact(3) {
        let a = Vec3::from(p.vertices[tri[0] as usize].pos);
        let b = Vec3::from(p.vertices[tri[1] as usize].pos);
        let c = Vec3::from(p.vertices[tri[2] as usize].pos);
        let n = (b - a).cross(c - a).normalize_or_zero();
        for &k in tri {
            let mut v = p.vertices[k as usize];
            v.normal = n.to_array();
            out.indices.push(out.vertices.len() as u32);
            out.vertices.push(v);
        }
    }
    scene.add_mesh(&out.mesh())
}
fn scenery(
    scene: &mut Scene,
    race: &Race,
    root: &Path,
) -> Result<Vec<spaceracer::environment::Obstacle>, String> {
    use spaceracer::environment::Obstacle;
    let e = &race.environment;
    let city = race.biome == "city";
    let forest = race.biome == "forest";
    let color = match race.biome.as_str() {
        "forest" => [0.065, 0.16, 0.045],
        "desert" => [0.55, 0.32, 0.13],
        "ocean" => [0.12, 0.22, 0.21],
        "cell" => [0.17, 0.045, 0.15],
        _ => [0.015, 0.02, 0.035],
    };
    let mat = surface(scene, 6, color, 0.85, 1.);
    let mut terrain = Primitive {
        material: mat,
        ..Default::default()
    };
    let at = |x: f32, z: f32| Vec3::new(x, e.height(x, z), z);
    for x in -50..50 {
        for z in -50..50 {
            let x = x as f32 * 80.;
            let z = z as f32 * 80.;
            let q = [
                at(x, z),
                at(x + 80., z),
                at(x + 80., z + 80.),
                at(x, z + 80.),
            ];
            let n = std::array::from_fn(|i| {
                let p = q[i];
                Vec3::new(
                    e.height(p.x - 1., p.z) - e.height(p.x + 1., p.z),
                    2.,
                    e.height(p.x, p.z - 1.) - e.height(p.x, p.z + 1.),
                )
                .normalize()
            });
            quad(&mut terrain, q, n, [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]);
        }
    }
    let mesh = scene.add_mesh(&terrain.mesh());
    scene.add_instance(mesh, Affine3A::IDENTITY);
    if forest || race.biome == "ocean" {
        let water = surface(scene, 3, [0.008, 0.065, 0.09], 0.16, 1.);
        let mesh = scene.add_mesh(&primitives::cube(water).mesh());
        box_at(
            scene,
            mesh,
            Vec3::new(0., e.ground - e.hills * 0.25, 0.),
            Vec3::new(8000., 0.5, 8000.),
            Quat::IDENTITY,
        );
    }
    let cladding = material(scene, [0.09, 0.12, 0.16], 0.5, 0.);
    let trim = scene.add_mesh(&primitives::cube(cladding).mesh());
    let rim_material = material(scene, [0.1, 0.5, 0.7], 0.5, 18.);
    let rim = scene.add_mesh(&primitives::cube(rim_material).mesh());
    let mut models = vec![];
    if forest {
        for name in ["pine-tree-1", "birch-tree-1"] {
            models.push(assets::gltf::load(
                scene,
                &root.join(format!("source/assets/forest/authored/{name}/scene.gltf")),
            )?);
        }
    }
    for model in &models {
        for &id in &model.materials {
            let m = &mut scene.materials[id as usize];
            if m.flags & render::scene::ALPHA_TEST != 0 {
                m.flags |= THIN_FOLIAGE;
                m.roughness = 0.9;
                m.metal = 0.;
                m.tint = [0.65, 0.78, 0.56];
            }
        }
    }
    let building = if city {
        surface(scene, 1, [0.045, 0.065, 0.085], 0.4, 1.)
    } else {
        surface(scene, 4, color, 0.8, 1.)
    };
    let mut towers = vec![];
    for k in 0..8 {
        let m = surface(
            scene,
            1,
            [0.035 + k as f32 * 0.002, 0.05, 0.065],
            0.4,
            k as f32 + 1.,
        );
        towers.push(scene.add_mesh(&primitives::cube(m).mesh()));
    }

    let rocks: Vec<_> = (0..4).map(|v| rock_mesh(scene, building, v)).collect();
    let organelle = surface(scene, 5, [0.08, 0.6, 0.54], 0.4, 1.);
    let sphere = scene.add_mesh(&primitives::sphere(organelle, 16, 24).mesh());
    let ad = if city {
        let layer = scene.add_texture(&root.join("source/assets/corporate-posters.png"))?;
        Some(scene.add_material(Material {
            tint: [1.; 3],
            layer,
            roughness: 0.5,
            metal: 0.,
            flags: EMISSIVE,
            scale: 260.,
        }))
    } else {
        None
    };
    let mut rng = race.world.track.seed as u64 + 73;
    let mut rand = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((rng >> 32) as u32) as f32 / u32::MAX as f32
    };
    let mut obstacles = vec![];
    let radius = (2600. / e.spacing) as i32;
    for gx in -radius..=radius {
        for gz in -radius..=radius {
            if rand() < e.skip {
                continue;
            }
            let x = gx as f32 * e.spacing + rand() * e.spacing * 0.28;
            let z = gz as f32 * e.spacing + rand() * e.spacing * 0.28;
            let w = 35. + rand() * 55.;
            let depth = 40. + rand() * 45.;
            let h = e.height[0] + rand() * (e.height[1] - e.height[0]);
            let y = e.height(x, z);
            let clearance = if forest {
                h * 0.22 + 12.
            } else {
                w.max(depth) * 0.75 + 25.
            };
            if race.world.track.nodes.iter().step_by(3).any(|n| {
                let d = n.geometry.p - Vec3::new(x, n.geometry.p.y, z);
                d.length() < n.geometry.width as f32 + clearance
            }) {
                continue;
            }
            if city {
                let cube = towers[((x.abs() + z.abs()) as usize) % towers.len()];
                let pos = Vec3::new(x, y + h * 0.5, z);
                let size = Vec3::new(w, h, depth);
                box_at(scene, cube, pos, size, Quat::IDENTITY);
                obstacles.push(Obstacle {
                    min: pos - size * 0.5,
                    max: pos + size * 0.5,
                });
                let style = ((x.abs() * 0.7 + z.abs()) as usize) % 4;
                // Solid parapets and buttresses anchor the glass curtain wall.
                for side in [-1., 1.] {
                    box_at(
                        scene,
                        trim,
                        Vec3::new(x, y + h + 0.8, z + side * depth * 0.5),
                        Vec3::new(w + 1., 1.6, 1.2),
                        Quat::IDENTITY,
                    );
                    if style != 0 {
                        box_at(
                            scene,
                            rim,
                            Vec3::new(x, y + h - 3., z + side * (depth * 0.5 + 0.18)),
                            Vec3::new(w, 0.25, 0.35),
                            Quat::IDENTITY,
                        );
                    }
                    for k in 0..=if style == 2 { 4 } else { 2 } {
                        let divisions = if style == 2 { 4. } else { 2. };
                        box_at(
                            scene,
                            trim,
                            Vec3::new(
                                x - w * 0.5 + w * k as f32 / divisions,
                                y + h * 0.5,
                                z + side * (depth * 0.5 + 0.15),
                            ),
                            Vec3::new(0.6, h, 0.6),
                            Quat::IDENTITY,
                        );
                    }
                }

                if rand() > 0.5 {
                    let pos = Vec3::new(x, y + h + 15., z);
                    let size = Vec3::new(w * 0.65, 30., depth * 0.6);
                    box_at(scene, cube, pos, size, Quat::IDENTITY);
                    obstacles.push(Obstacle {
                        min: pos - size * 0.5,
                        max: pos + size * 0.5,
                    });
                }
                if rand() > 0.6 {
                    box_at(
                        scene,
                        cube,
                        Vec3::new(x, y + h + 38., z),
                        Vec3::new(1.4, 50., 1.4),
                        Quat::IDENTITY,
                    );
                }
                if let Some(mat) = ad {
                    if rand() > 0.42 {
                        let tile = (rand() * 6.) as u32;
                        let u = (tile % 3) as f32 / 3.;
                        let v = (tile / 3) as f32 / 2.;
                        for side in [-1., 1.] {
                            let mut poster = Primitive {
                                material: mat,
                                ..Default::default()
                            };
                            let p = Vec3::new(x, y + h * 0.58, z + side * (depth * 0.5 + 0.2));
                            let a = (w * 0.42).min(h * 0.20);
                            let b = a * 2.;
                            quad(
                                &mut poster,
                                [
                                    p + Vec3::new(-a * side, -b, 0.),
                                    p + Vec3::new(a * side, -b, 0.),
                                    p + Vec3::new(a * side, b, 0.),
                                    p + Vec3::new(-a * side, b, 0.),
                                ],
                                [Vec3::Z * side; 4],
                                [
                                    [u, v + 0.5],
                                    [u + 1. / 3., v + 0.5],
                                    [u + 1. / 3., v],
                                    [u, v],
                                ],
                            );
                            let mesh = scene.add_mesh(&poster.mesh());
                            scene.add_instance(mesh, Affine3A::IDENTITY);
                        }
                    }
                }
            } else if forest {
                let tree = &models[(rand() * 2.) as usize % 2];
                let scale = h / (tree.max.y - tree.min.y).max(0.01);
                let rot = Quat::from_rotation_y(rand() * std::f32::consts::TAU);
                let center = (tree.min + tree.max) * 0.5;
                let offset = rot * Vec3::new(center.x, tree.min.y, center.z) * scale;
                box_at(
                    scene,
                    tree.mesh,
                    Vec3::new(x, y, z) - offset,
                    Vec3::splat(scale),
                    rot,
                );
                obstacles.push(Obstacle {
                    min: Vec3::new(x - 3., y, z - 3.),
                    max: Vec3::new(x + 3., y + h * 0.8, z + 3.),
                });
            } else if race.biome == "cell" {
                box_at(
                    scene,
                    sphere,
                    Vec3::new(x, y + h * 0.7, z),
                    Vec3::new(w * 0.55, h * 0.8, depth * 0.55),
                    Quat::from_rotation_y(rand() * 6.28),
                );
                // Helical protein strands are shared sphere instances, not unique meshes.
                for k in 0..24 {
                    let t = k as f32 * 0.48;
                    for side in [-1., 1.] {
                        box_at(
                            scene,
                            sphere,
                            Vec3::new(
                                x + side * t.cos() * 18.,
                                y + k as f32 * 5.,
                                z + side * t.sin() * 18.,
                            ),
                            Vec3::splat(4.),
                            Quat::IDENTITY,
                        );
                    }
                }
                obstacles.push(Obstacle {
                    min: Vec3::new(x - w * 0.25, y, z - depth * 0.25),
                    max: Vec3::new(x + w * 0.25, y + h, z + depth * 0.25),
                });
            } else {
                let pos = Vec3::new(x, y + h * 0.25, z);
                let size = Vec3::new(w, h, depth);
                box_at(
                    scene,
                    rocks[((x.abs() + z.abs()) as usize) % 4],
                    pos,
                    size,
                    Quat::from_rotation_y(rand() * 6.28),
                );
                obstacles.push(Obstacle {
                    min: pos - Vec3::new(w * 0.25, h * 0.4, depth * 0.25),
                    max: pos + Vec3::new(w * 0.25, h * 0.4, depth * 0.25),
                });
            }
        }
    }
    Ok(obstacles)
}
fn pickup_mesh(scene: &mut Scene, material: u32) -> usize {
    let mut p = Primitive {
        material,
        ..Default::default()
    };
    for axis in 0..3 {
        for a in [-0.5, 0.5] {
            for b in [-0.5, 0.5] {
                let mut scale = Vec3::splat(0.055);
                scale[axis] = 1.;
                let mut pos = Vec3::ZERO;
                pos[(axis + 1) % 3] = a;
                pos[(axis + 2) % 3] = b;
                let shape = primitives::cube(material);
                let offset = p.vertices.len() as u32;
                for mut v in shape.vertices {
                    v.pos = (Vec3::from(v.pos) * scale + pos).to_array();
                    p.vertices.push(v);
                }
                p.indices
                    .extend(shape.indices.into_iter().map(|i| i + offset));
            }
        }
    }
    scene.add_mesh(&p.mesh())
}
fn weapon_mesh(scene: &mut Scene, kind: usize, metal: u32, glow: u32) -> usize {
    let mut p = Primitive {
        material: metal,
        ..Default::default()
    };
    let mut add = |shape: Primitive, scale: Vec3, rot: Quat, pos: Vec3| {
        let offset = p.vertices.len() as u32;
        let xf = Affine3A::from_scale_rotation_translation(scale, rot, pos);
        for mut v in shape.vertices {
            v.pos = xf.transform_point3(Vec3::from(v.pos)).to_array();
            v.normal = (rot * Vec3::from(v.normal)).to_array();
            p.vertices.push(v);
        }
        p.indices
            .extend(shape.indices.into_iter().map(|i| i + offset));
    };
    match kind {
        0 | 5 => {
            add(
                primitives::cylinder(metal, 0.15, 0.46, 12),
                Vec3::new(1., 5.4, 1.),
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                Vec3::ZERO,
            );
            for side in [-1., 1.] {
                add(
                    primitives::cube(metal),
                    Vec3::new(1.5, 0.08, 1.3),
                    Quat::from_rotation_y(side * 0.35),
                    Vec3::new(side * 0.75, 0., -1.),
                );
            }
        }
        2 => {
            add(
                primitives::cylinder(metal, 0.7, 0.7, 12),
                Vec3::new(1., 0.7, 1.),
                Quat::IDENTITY,
                Vec3::new(0., -0.2, 0.),
            );
            add(
                primitives::cube(metal),
                Vec3::new(1.1, 0.8, 1.1),
                Quat::IDENTITY,
                Vec3::new(0., 0.2, -0.3),
            );
            for j in 0..6 {
                let a = j as f32 * std::f32::consts::TAU / 6.;
                add(
                    primitives::cylinder(metal, 0.12, 0.12, 6),
                    Vec3::new(1., 2.8, 1.),
                    Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                    Vec3::new(a.cos() * 0.32, 0.25 + a.sin() * 0.32, 1.),
                );
            }
            add(
                primitives::cube(metal),
                Vec3::new(0.6, 0.65, 1.4),
                Quat::IDENTITY,
                Vec3::new(-0.8, 0.1, -0.25),
            );
        }
        4 => {
            for side in [-1., 1.] {
                add(
                    primitives::cube(metal),
                    Vec3::new(0.3, 0.45, 3.5),
                    Quat::IDENTITY,
                    Vec3::new(side * 0.38, 0.1, 0.5),
                );
            }
            add(
                primitives::cube(metal),
                Vec3::new(1.2, 0.7, 1.1),
                Quat::IDENTITY,
                Vec3::new(0., 0., -1.),
            );
        }
        _ => {
            add(
                primitives::sphere(metal, 8, 12),
                Vec3::new(1.1, 0.55, 1.6),
                Quat::IDENTITY,
                Vec3::ZERO,
            );
            for z in [-0.7, 0., 0.7] {
                add(
                    primitives::cube(metal),
                    Vec3::new(1.25, 0.15, 0.15),
                    Quat::IDENTITY,
                    Vec3::new(0., 0.5, z),
                );
            }
        }
    }
    let _ = glow;
    scene.add_mesh(&p.mesh())
}
fn transform(v: &Value) -> Affine3A {
    let origin: [f32; 3] = serde_json::from_value(v["origin"].clone()).unwrap_or([0.; 3]);
    let basis: [[f32; 3]; 3] = serde_json::from_value(v["basis"].clone()).unwrap_or([
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
    ]);
    Affine3A::from_mat3_translation(Mat3::from_cols_array_2d(&basis), Vec3::from(origin))
}
impl Visuals {
    pub fn new(race: &Race, root: &Path) -> Result<Self, String> {
        let mut scene = Scene::default();
        let mut lighting = Lighting::default();
        if race.biome == "city" || race.biome == "cell" {
            scene.sky = Some(primitives::gradient_sky(
                [0.025, 0.045, 0.08],
                [0.12, 0.16, 0.21],
                [0.035, 0.045, 0.055],
            ));
        } else {
            let sky = render::sky::bake_live(
                0,
                render::sky::LiveSettings::preset(
                    render::sky::DayTime::Noon,
                    render::sky::WeatherLook::Clear,
                ),
                &mut Default::default(),
                Default::default(),
            )?;
            sky.apply_scene(&mut scene, &mut lighting);
        }
        if race.biome == "cell" {
            scene.sky = Some(primitives::gradient_sky(
                [0.10, 0.12, 0.15],
                [0.18, 0.14, 0.20],
                [0.06, 0.04, 0.065],
            ));
        }
        lighting.sun_lux = race.environment.sun;
        lighting.sky_lux = race.environment.sky;
        lighting.ev100 = race.environment.exposure;
        if race.biome == "city" {
            lighting.sun_color = Vec3::new(0.6, 0.72, 1.);
        }
        lighting.fog_height_m = 450.;
        lighting.fog_density = if race.biome == "city" {
            0.00015
        } else {
            0.000015
        };
        lighting.fog_start = 800.;
        lighting.fog_end = 10000.;
        lighting.fog_phase_g = 0.4;
        lighting.bloom_boost = 0.65;
        lighting.bloom_threshold = 1.4;
        lighting.fx_white_floor = 0.25;
        lighting.lod_pixels = 0.7;
        lighting.vignette = 0.12;
        lighting.sharpen = 0.15;
        road(&mut scene, race);
        let obstacles = scenery(&mut scene, race, root)?;
        let lamps = track_lights(&mut scene, race);
        let mut raw = String::new();
        flate2::read::GzDecoder::new(include_bytes!("../../assets/ships.json.gz").as_slice())
            .read_to_string(&mut raw)
            .map_err(|e| e.to_string())?;
        let data: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let mut all_parts = vec![];
        for color in COLORS {
            let mut materials = vec![];
            for source in data["materials"].as_array().ok_or("ship materials")? {
                let hull = source["shader"]
                    .as_str()
                    .is_some_and(|v| v.ends_with("ship.gdshader"));
                let tint = if hull {
                    color
                } else {
                    serde_json::from_value::<[f32; 4]>(source["albedo_color"].clone())
                        .unwrap_or([0.1, 0.3, 0.5, 1.])[..3]
                        .try_into()
                        .unwrap()
                };
                materials.push(scene.add_material(Material {
                    tint,
                    layer: if hull { -12 } else { -1 },
                    roughness: source["roughness"].as_f64().unwrap_or(0.31) as f32,
                    metal: if hull {
                        0.32
                    } else {
                        source["metallic"].as_f64().unwrap_or(0.) as f32
                    },
                    flags: if hull {
                        SURFACE
                    } else if source["emission_enabled"].as_bool().unwrap_or(false) {
                        EMISSIVE
                    } else {
                        0
                    },
                    scale: if hull {
                        1.
                    } else {
                        source["emission_energy_multiplier"].as_f64().unwrap_or(0.) as f32
                            * if race.biome == "city" || race.biome == "cell" {
                                100.
                            } else {
                                12000.
                            }
                    },
                }));
            }
            let mut parts = vec![];
            load_node(
                &mut scene,
                &data,
                &data["models"][0],
                &materials,
                &mut vec![],
                true,
                &mut parts,
            )?;
            all_parts.push(parts);
        }
        let pickup_mat = material(&mut scene, [0.6, 0.15, 0.95], 0.3, 100.);
        let pickup_mesh = pickup_mesh(&mut scene, pickup_mat);
        let missile_mat = material(&mut scene, [0.32, 0.36, 0.4], 0.25, 0.);
        let mount_meshes: Vec<_> = (0..6)
            .map(|k| weapon_mesh(&mut scene, k, missile_mat, pickup_mat))
            .collect();
        let projectile_mesh = mount_meshes[0];
        let mount_mesh = mount_meshes[2];
        let mut slots = Slots::reserve(&mut scene, 1024);
        let ships = all_parts
            .into_iter()
            .map(|parts: Vec<(usize, Vec<(String, Affine3A)>)>| {
                parts
                    .into_iter()
                    .map(|(mesh, chain)| ShipPart {
                        mesh,
                        chain,
                        slot: slots.add(&mut scene, mesh, Affine3A::IDENTITY).unwrap(),
                    })
                    .collect()
            })
            .collect();
        let pickup_slots = race
            .pickups
            .iter()
            .map(|_| {
                slots
                    .add(&mut scene, pickup_mesh, Affine3A::IDENTITY)
                    .unwrap()
            })
            .collect();
        let projectiles = (0..32)
            .map(|_| {
                slots
                    .add(
                        &mut scene,
                        projectile_mesh,
                        Affine3A::from_scale(Vec3::splat(0.001)),
                    )
                    .unwrap()
            })
            .collect();
        let mounts = (0..6)
            .map(|_| {
                slots
                    .add(
                        &mut scene,
                        mount_mesh,
                        Affine3A::from_scale(Vec3::splat(0.001)),
                    )
                    .unwrap()
            })
            .collect();
        let mut jets = vec![];
        let mesh = jet_mesh(&mut scene, race.biome == "city" || race.biome == "cell");
        for _ in 0..12 {
            let id = slots.add(&mut scene, mesh, Affine3A::IDENTITY).unwrap();
            jets.push((mesh, id));
        }
        Ok(Self {
            scene,
            lighting,
            lamps,
            obstacles,
            slots,
            ships,
            jets,
            pickup_mesh,
            pickup_slots,
            projectile_mesh,
            projectiles,
            mounts,
            mount_meshes,
            uploaded: false,
            fx: Default::default(),
            crashes: vec![0; 6],
            blast_stamp: -1.,
            fx_stamp: -1.,
        })
    }
    pub fn update(&mut self, race: &Race, previous: &[spaceracer::race_rules::Pose], alpha: f32) {
        let time = race.tick as f32 / 120.;
        let fresh = time > self.fx_stamp;
        self.fx_stamp = time;
        // Explicit interpolation in draw handles >4 m/tick boosts; generic Slots
        // would classify that speed as a teleport.
        for i in 0..race.pilots.len() {
            let mut pose = race.pose(i);
            if let Some(prev) = previous.get(i) {
                pose.origin = prev.origin.lerp(pose.origin, alpha);
                pose.basis = Mat3::from_quat(
                    Quat::from_mat3(&prev.basis).slerp(Quat::from_mat3(&pose.basis), alpha),
                );
            }
            let p = &race.pilots[i];
            let input = race.drivers[i].input.drive;
            let base = Affine3A::from_mat3_translation(pose.basis, pose.origin);
            for part in &self.ships[i] {
                let mut xf = base;
                for (name, local) in &part.chain {
                    let mut local = *local;
                    let rot = if name.starts_with("WingControl") {
                        let side = if name.ends_with('L') { -1. } else { 1. };
                        Quat::from_rotation_x(
                            (-input.trim * 0.55 - input.strafe * side * 0.5 + input.brake * 0.6)
                                as f32,
                        ) * Quat::from_rotation_z((-input.strafe * 0.16) as f32)
                    } else if name.starts_with("Rudder") {
                        Quat::from_rotation_y(
                            (input.steer.signum() * input.steer.abs().powf(0.72) * 0.7) as f32,
                        )
                    } else if name.starts_with("Airbrake") {
                        Quat::from_rotation_x(input.brake as f32 * 1.15)
                    } else {
                        Quat::IDENTITY
                    };
                    local.matrix3 *= glam::Mat3A::from_quat(rot);
                    xf *= local;
                }
                if p.crashed {
                    xf *= Affine3A::from_scale(Vec3::splat(0.001));
                }
                self.slots.set(&mut self.scene, part.slot, part.mesh, xf);
            }
            let mounting = race.drivers[i].active.or(race.drivers[i].item);
            self.slots.set(
                &mut self.scene,
                self.mounts[i],
                self.mount_meshes[mounting.map_or(0, |w| w as usize)],
                base * Affine3A::from_scale_rotation_translation(
                    if mounting.is_some() && !p.crashed {
                        Vec3::ONE
                    } else {
                        Vec3::splat(0.001)
                    },
                    Quat::IDENTITY,
                    Vec3::new(0., 1.5, -1.),
                ),
            );
            for side in 0..2 {
                let (mesh, id) = self.jets[i * 2 + side];
                let power = if p.crashed || p.emp_time > 0. {
                    0.
                } else {
                    input.throttle as f32
                };
                let boosted = p.boost > 0. || p.on_pad;
                let local = Vec3::new(if side == 0 { -2.45 } else { 2.45 }, -0.12, -3.8);
                let pulse = 1. + 0.06 * (time * 91. + i as f32 * 3.).sin();
                let length = (1.5 + power * 2. + if boosted { 4. } else { 0. }) * power;
                self.slots.set(
                    &mut self.scene,
                    id,
                    mesh,
                    base * Affine3A::from_scale_rotation_translation(
                        Vec3::new(power * 1.15, power * 1.15, length * pulse)
                            .max(Vec3::splat(0.001)),
                        Quat::IDENTITY,
                        local,
                    ),
                );
            }
            if race.drivers[i].motion.crash_id > self.crashes[i] {
                self.crashes[i] = race.drivers[i].motion.crash_id;
                self.fx
                    .explosion(time, pose.origin, 12., Vec3::new(1., 0.3, 0.05));
            }
        }
        for (k, p) in race.pickups.iter().enumerate() {
            let n = race.world.track.sample(p.distance);
            let size = if p.cooldown > 0. { 0.001 } else { 3. };
            self.slots.set(
                &mut self.scene,
                self.pickup_slots[k],
                self.pickup_mesh,
                Affine3A::from_scale_rotation_translation(
                    Vec3::splat(size),
                    Quat::from_rotation_y(race.clock as f32) * Quat::from_rotation_z(0.7),
                    n.point(p.x, 4.),
                ),
            );
        }
        for (k, id) in self.projectiles.iter().enumerate() {
            let xf = if let Some(p) = race.projectiles.get(k) {
                Affine3A::from_scale_rotation_translation(
                    Vec3::splat(1.3),
                    Quat::from_rotation_arc(Vec3::Z, p.velocity.normalize_or_zero()),
                    p.position,
                )
            } else {
                Affine3A::from_scale(Vec3::splat(0.001))
            };
            self.slots
                .set(&mut self.scene, *id, self.projectile_mesh, xf);
        }
        if fresh {
            for b in &race.beams {
                if b.age <= spaceracer::handling::DT * 1.1 {
                    let d = b.to - b.from;
                    let _ = self.fx.streak(render::fx::AdditiveStreak {
                        position: (b.from + b.to) * 0.5,
                        travel: d.normalize_or_zero() * 0.001,
                        birth: time - 0.001,
                        life: if b.rail { 0.14 } else { 1. / 120. },
                        radius: if b.rail { [0.16, 0.03] } else { [0.035, 0.015] },
                        length: [d.length(); 2],
                        color: if b.rail {
                            Vec3::new(0.3, 0.8, 1.)
                        } else {
                            Vec3::new(1., 0.7, 0.2)
                        },
                        radiance: 15000.,
                        opacity: 0.8,
                        fade_in: 0.,
                        fade_out: 1.,
                        ..Default::default()
                    });
                    let _ = self.fx.mote(render::fx::AdditiveMote {
                        position: b.to,
                        birth: time - 0.001,
                        life: 0.1,
                        radius: 1.4,
                        color: Vec3::new(1., 0.7, 0.3),
                        radiance: 12000.,
                        fade_in: 0.,
                        fade_out: 1.,
                        ..Default::default()
                    });
                }
            }
            for p in &race.projectiles {
                let back = -p.velocity.normalize_or_zero();
                let _ = self.fx.mote(render::fx::AdditiveMote {
                    position: p.position + back * 3.,
                    travel: back * 20.,
                    birth: time - 0.001,
                    life: 0.09,
                    radius: 1.5,
                    color: Vec3::new(0.2, 0.65, 1.),
                    radiance: 35000.,
                    fade_in: 0.,
                    fade_out: 1.,
                    motion_power: 1.,
                    ..Default::default()
                });
                if p.kind == Weapon::Missile {
                    if let Some(target) = p.target {
                        let goal = race.pose(target).origin;
                        let distance = goal.distance(p.position);
                        if distance < 160. {
                            let _ = self.fx.streak(render::fx::AdditiveStreak {
                                position: (goal + p.position) * 0.5,
                                travel: (goal - p.position).normalize_or_zero() * 0.001,
                                birth: time - 0.001,
                                life: 1. / 60.,
                                radius: [0.025; 2],
                                length: [distance; 2],
                                color: Vec3::new(1., 0.2, 0.12),
                                radiance: 3000. * (1. - distance / 160.),
                                opacity: 0.4,
                                fade_in: 0.,
                                fade_out: 1.,
                                ..Default::default()
                            });
                            let _ = self.fx.mote(render::fx::AdditiveMote {
                                position: goal,
                                birth: time - 0.001,
                                life: 1. / 60.,
                                radius: 0.5,
                                color: Vec3::ONE,
                                radiance: 20000. * (1. - distance / 160.),
                                fade_in: 0.,
                                fade_out: 1.,
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
        for b in &race.blasts {
            let birth = race.clock - b.age;
            if birth > self.blast_stamp {
                self.fx.explosion(
                    time - b.age as f32,
                    b.position,
                    b.radius * 0.3,
                    if b.kind == Weapon::Emp {
                        Vec3::new(0.1, 0.7, 1.)
                    } else {
                        Vec3::new(1., 0.3, 0.05)
                    },
                );
            }
        }
        self.blast_stamp = race.clock;
    }
    pub fn light(&self, race: &Race) -> Lighting {
        let mut light = self.lighting.clone();
        light.time = race.tick as f32 / 120.;
        light
    }
}
fn load_node(
    scene: &mut Scene,
    data: &Value,
    node: &Value,
    mats: &[u32],
    chain: &mut Vec<(String, Affine3A)>,
    visible: bool,
    out: &mut Vec<(usize, Vec<(String, Affine3A)>)>,
) -> Result<(), String> {
    let name = node["name"].as_str().unwrap_or("");
    let visible = visible && node["visible"].as_bool().unwrap_or(true);
    chain.push((name.into(), transform(&node["transform"])));
    let optical = [
        "Exhaust",
        "BoostArcs",
        "Reverse",
        "EngineHalo",
        "EngineFlare",
        "EngineHeat",
        "EngineCore",
    ]
    .iter()
    .any(|s| name.starts_with(s));
    if visible && !optical {
        if let Some(mesh) = node["mesh"].as_u64() {
            for surface in data["meshes"][mesh as usize]["surfaces"]
                .as_array()
                .ok_or("ship surfaces")?
            {
                let positions: Vec<[f32; 3]> = serde_json::from_value(surface["positions"].clone())
                    .map_err(|e| e.to_string())?;
                let normals: Vec<[f32; 3]> =
                    serde_json::from_value(surface["normals"].clone()).unwrap_or_default();
                let uv: Vec<[f32; 2]> =
                    serde_json::from_value(surface["uv"].clone()).unwrap_or_default();
                let mut indices: Vec<u32> =
                    serde_json::from_value(surface["indices"].clone()).unwrap_or_default();
                if indices.is_empty() {
                    indices = (0..positions.len() as u32).collect();
                }
                for t in indices.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
                let override_mat = node["material"].as_i64().unwrap_or(-1);
                let m = if override_mat >= 0 {
                    override_mat
                } else {
                    surface["material"].as_i64().unwrap_or(0)
                }
                .max(0) as usize;
                let mut mat = mats[m];
                if scene.materials[mat as usize].flags & SURFACE != 0 {
                    let mut copy = scene.materials[mat as usize];
                    let style = node["instance_parameters"]["marking_style"]
                        .as_i64()
                        .unwrap_or(0)
                        .clamp(0, 4);
                    copy.layer = -12 - style as i32;
                    copy.scale = node["instance_parameters"]["face_side"]
                        .as_f64()
                        .unwrap_or(1.) as f32;
                    mat = scene.add_material(copy);
                }
                let p = Primitive {
                    material: mat,
                    vertices: positions
                        .into_iter()
                        .enumerate()
                        .map(|(i, pos)| Vertex {
                            pos,
                            normal: normals.get(i).copied().unwrap_or([0., 1., 0.]),
                            uv: uv.get(i).copied().unwrap_or([0.; 2]),
                        })
                        .collect(),
                    indices,
                };
                let mesh = scene.add_mesh(&p.mesh());
                out.push((mesh, chain.clone()));
            }
        }
    }
    for child in node["children"].as_array().into_iter().flatten() {
        load_node(scene, data, child, mats, chain, visible, out)?;
    }
    chain.pop();
    Ok(())
}
