use glam::{Affine3A, Mat3, Quat, Vec3};
use render::{
    Lighting, Scene,
    cluster::Vertex,
    primitives::{self, Primitive},
    scene::{CLEARCOAT, EMISSIVE, Material, SURFACE},
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
type LoadedPart = (usize, Option<(usize, Vec3)>, Vec<(String, Affine3A)>);
pub struct ShipPart {
    pub mesh: usize,
    pub slot: SlotId,
    pub chain: Vec<(String, Affine3A)>,
    pub wreck: Option<(usize, Vec3, SlotId)>,
}
pub struct Visuals {
    pub stage: super::stage::Stage,
    pub scene: Scene,
    pub lighting: Lighting,
    pub lamps: Vec<render::headlights::LocalLight>,
    pub obstacles: Vec<spaceracer::environment::Obstacle>,
    pub slots: Slots,
    pub ships: Vec<Vec<ShipPart>>,
    pub jets: super::jets::Jets,
    pub weapon_art: super::weapon_art::Art,
    pub faces: super::pilot_faces::Faces,
    pub crash_fx: super::crash_fx::Crashes,
    pub presentation_dt: f32,
    last_presentation: Option<f32>,
    pub pickup_mesh: usize,
    pub pickup_slots: Vec<SlotId>,
    pub projectile_mesh: usize,
    pub projectiles: Vec<SlotId>,
    pub mounts: Vec<SlotId>,
    pub mount_meshes: Vec<usize>,
    pub uploaded: bool,
    pub vehicle_aux: usize,
    pub fx: render::fx::Effects,
    pub crashes: Vec<u64>,
    pub blast_stamp: f64,
    pub fx_stamp: f32,
}
pub(super) fn material(scene: &mut Scene, tint: [f32; 3], rough: f32, emission: f32) -> u32 {
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
pub(super) fn quad(p: &mut Primitive, points: [Vec3; 4], normal: [Vec3; 4], uv: [[f32; 2]; 4]) {
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
pub(super) fn surface(scene: &mut Scene, kind: i32, tint: [f32; 3], rough: f32, scale: f32) -> u32 {
    scene.add_material(Material {
        tint,
        layer: -2 - kind,
        roughness: rough,
        metal: 0.0,
        flags: SURFACE,
        scale,
    })
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
// Two continuous ribbons, not cells discarded by their midpoint. This preserves
// the exact collision boundary even when the split expands between samples.
use spaceracer::track_collision::road_span;
fn road_edge(parts:&mut [Primitive],a:&spaceracer::track::Node,b:&spaceracer::track::Node,x:f64,y:f64,inward:f64,ha:f64,hb:f64) {
    let na=a.frame(x);let nb=b.frame(y);
    let top=[na.y_axis,na.y_axis,nb.y_axis,nb.y_axis];
    let normal=[-na.x_axis*inward as f32,-na.x_axis*inward as f32,-nb.x_axis*inward as f32,-nb.x_axis*inward as f32];
    quad(&mut parts[2],[a.point(x,-2.5),a.point(x,ha),b.point(y,hb),b.point(y,-2.5)],normal,[[0.;2];4]);
    // One continuous neon strip along each actual edge, including unguarded edges.
    quad(&mut parts[1],[a.point(x,ha+0.05),a.point(x+inward*0.3,ha+0.05),b.point(y+inward*0.3,hb+0.05),b.point(y,hb+0.05)],top,[[0.;2];4]);
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
            if a.air_gap {
                continue;
            }
            for lane in 0..16 {
                let side = if lane<8 {-1.}else{1.};
                if a.dead_gap && a.dead_side*side>0. {continue;}
                let t=(lane%8) as f64/8.;
                let [ax,bx,cx,dx]=road_span(a,b,side,t,t+0.125);
                let u=ax/a.geometry.width;let v=bx/a.geometry.width;
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
                            (cx/b.geometry.width) as f32,
                            (i + 1) as f32 * (race.world.track.length / nodes.len() as f64) as f32,
                        ],
                        [
                            (dx/b.geometry.width) as f32,
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
                    if a.dead_gap && a.dead_side*side>0. {continue;}
                    let split=a.split_gap.max(b.split_gap)>0.0001;
                    let wings:&[f64]=if split {&[-1.,1.]}else{&[1.]};
                    for &wing_side in wings {
                        let points = [(0.28,d),(0.,d+6.),(0.,d+8.),(0.28,d+2.)]
                            .map(|(wing,d)| {
                                let n=race.world.track.sample(d);
                                let center=if split {side*(n.geometry.width+n.split_gap)*0.5}else{0.};
                                let span=n.geometry.width-n.split_gap;
                                n.point(center+side*wing_side*span*wing,0.05)
                            });
                        quad(&mut parts[3],points,[a.geometry.basis().y_axis;4],[[0.;2];4]);
                    }
                }
            }
            for side in [-1.,1.] {
                if a.dead_gap && a.dead_side*side>0. {continue;}
                road_edge(&mut parts,a,b,side*a.geometry.width,side*b.geometry.width,-side,
                    if a.rails && a.geometry.shape_angle<3. {2.4}else{0.},
                    if a.rails && b.geometry.shape_angle<3. {2.4}else{0.});
                if a.split_gap.max(b.split_gap)>0.0001 {
                    // This boundary already constrains racers in Pilot::constrain.
                    // Match the source fork rail and fade its height at the nose.
                    road_edge(&mut parts,a,b,side*a.split_gap,side*b.split_gap,side,
                        2.4*spaceracer::handling::smooth(0.,3.,a.split_gap),
                        2.4*spaceracer::handling::smooth(0.,3.,b.split_gap));
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
    if kind == 0 {
        return super::missile_mesh::mesh(scene);
    }
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
        let mut scene = Scene {stage_sun:true,..Default::default()};
        scene.graphics.sharp_sprites=true;
        let mut lighting = Lighting::default();
        if race.biome == "city" || race.biome == "cell" || race.biome == "ocean" {
            scene.sky = Some(primitives::gradient_sky(
                [0.025, 0.045, 0.08],
                [0.12, 0.16, 0.21],
                [0.035, 0.045, 0.055],
            ));
        } else {
            let sky = render::sky::bake_live(
                0,
                render::sky::LiveSettings::preset(
                    if race.biome == "forest" {
                        render::sky::DayTime::Afternoon
                    } else {
                        render::sky::DayTime::Noon
                    },
                    render::sky::WeatherLook::Clear,
                ),
                &mut Default::default(),
                Default::default(),
            )?;
            sky.apply_scene(&mut scene, &mut lighting);
        }
        if race.biome == "cell" {
            scene.sky = Some(primitives::gradient_sky(
                [0.09, 0.235, 0.275],
                [0.26, 0.42, 0.38],
                [0.08, 0.19, 0.18],
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
        if race.biome == "ocean" {
            scene.sky = Some(primitives::gradient_sky(
                [0.06, 0.43, 0.50],
                [0.04, 0.25, 0.31],
                [0.025, 0.12, 0.16],
            ));
            lighting.sun_color = Vec3::new(0.72, 0.9, 1.0);
            lighting.fog_height_m = 1800.;
            lighting.fog_density = 0.00035;
            lighting.fog_start = 180.;
            lighting.fog_end = 1800.;
        }
        lighting.fog_phase_g = 0.4;
        lighting.bloom_boost = 0.8;
        lighting.contrast = 1.06;
        lighting.saturation = 1.04;
        lighting.bloom_threshold = 1.4;
        lighting.fx_white_floor = 0.25;
        lighting.lod_pixels = 0.7;
        lighting.vignette = 0.12;
        lighting.sharpen = 0.15;
        road(&mut scene, race);
        let mut stage = super::stage::Stage::build(&mut scene, race, root)?;
        let obstacles = stage.obstacles.clone();
        let mut lamps = track_lights(&mut scene, race);
        lamps.extend(stage.lamps.iter().copied());
        let mut raw = String::new();
        flate2::read::GzDecoder::new(include_bytes!("../../assets/ships.json.gz").as_slice())
            .read_to_string(&mut raw)
            .map_err(|e| e.to_string())?;
        let data: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let mut faces = super::pilot_faces::Faces::new(&mut scene);
        let mut all_parts = vec![];
        for (design, color) in COLORS.into_iter().enumerate() {
            let first_material = scene.materials.len();
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
                        SURFACE | CLEARCOAT
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
                &data["models"][design % data["models"].as_array().unwrap().len()],
                &materials,
                &mut vec![],
                true,
                &mut parts,
                faces.layer * 8 + design as i32 + 2,
            )?;
            faces.paints.push(
                (first_material..scene.materials.len())
                    .filter(|&m| {
                        let k = -2 - scene.materials[m].layer;
                        (10..=14).contains(&k) || k == 23
                    })
                    .collect(),
            );
            all_parts.push(parts);
        }
        let pickup_mat = material(&mut scene, [0.6, 0.15, 0.95], 0.3, 100.);
        let pickup_mesh = pickup_mesh(&mut scene, pickup_mat);
        let missile_mat = material(&mut scene, [0.32, 0.36, 0.4], 0.25, 0.);
        let mut weapon_art=super::weapon_art::Art::new(&mut scene,race.environment.exposure);
        let mount_meshes: Vec<_> = (0..6)
            .map(|k| match k {1=>weapon_art.coil,2=>weapon_art.base,4=>weapon_art.rail,5=>weapon_art.bomb,_=>weapon_mesh(&mut scene,k,missile_mat,pickup_mat)})
            .collect();
        let projectile_mesh = mount_meshes[0];
        let mount_mesh = mount_meshes[2];
        let mut slots = Slots::reserve(&mut scene, 1024 + stage.capacity());
        stage.reserve(&mut scene, &mut slots);
        weapon_art.reserve(&mut scene,&mut slots);
        let ships: Vec<Vec<ShipPart>> = all_parts
            .into_iter()
            .map(|parts: Vec<LoadedPart>| {
                parts
                    .into_iter()
                    .map(|(mesh, wreck, chain)| ShipPart {
                        wreck: wreck.map(|(m, c)| {
                            (
                                m,
                                c,
                                slots
                                    .add(&mut scene, m, Affine3A::from_scale(Vec3::splat(0.0001)))
                                    .unwrap(),
                            )
                        }),
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
        let wear_layer = scene.add_texture(&root.join("source/assets/paint-wear.png"))?;
        let vehicle_aux = scene.aux.len();
        scene.aux[5] = vehicle_aux as u32;
        scene.aux.push(wear_layer as u32);
        for parts in &ships {
            let mut record = [0u32;24];
            record[0] = parts.iter().map(|p| slots.instance(p.slot)).min().unwrap() as u32;
            record[1] = parts.iter().map(|p| slots.instance(p.slot)).max().unwrap() as u32 + 1;
            record[3] = 1f32.to_bits();
            scene.aux.extend(record);
        }
        let jets = super::jets::Jets::new(root)?;
        let crash_fx = super::crash_fx::Crashes::new(root)?;
        let mut fx = render::fx::Effects::default();
        fx.light_scale = crash_fx.config.explosion_light_scale;
        fx.smooth_billows = true;
        // Same shared animated density asset and adaptive renderer as Growing Guns.
        // This Effects instance handles both vehicle crashes and weapon blasts.
        fx.set_baked_smoke(crash_fx.config.baked_explosion_smoke);
        Ok(Self {
            stage,
            scene,
            lighting,
            lamps,
            obstacles,
            slots,
            ships,
            jets,
            crash_fx,
            faces,
            presentation_dt: 0.,
            last_presentation: None,
            pickup_mesh,
            pickup_slots,
            projectile_mesh,
            projectiles,
            mounts,
            mount_meshes,
            weapon_art,
            uploaded: false,
            vehicle_aux,
            fx,
            crashes: vec![0; 6],
            blast_stamp: -1.,
            fx_stamp: -1.,
        })
    }
    pub fn update(
        &mut self,
        race: &Race,
        previous: &[spaceracer::race_rules::Pose],
        alpha: f32,
    ) -> Result<(), String> {
        let time = (race.tick.saturating_sub(1) as f32 + alpha) / 120.;
        self.presentation_dt = self.last_presentation.map_or(0., |t| (time - t).max(0.));
        self.last_presentation = Some(time);
        self.stage.update(
            &mut self.scene,
            &mut self.slots,
            (race.tick as f32 - 1. + alpha).max(0.) / 120.,
        );
        // Emit discrete weapon events once per simulation tick, even when
        // several interpolated presentation frames fall between two ticks.
        let tick_time = race.tick as f32 / 120.;
        let fresh = tick_time > self.fx_stamp;
        self.fx_stamp = tick_time;
        // Explicit interpolation in draw handles >4 m/tick boosts; generic Slots
        // would classify that speed as a teleport.
        let mut poses = Vec::with_capacity(race.pilots.len());
        for i in 0..race.pilots.len() {
            let mut pose = race.pose(i);
            if let Some(prev) = previous.get(i) {
                pose.origin = prev.origin.lerp(pose.origin, alpha);
                pose.basis = Mat3::from_quat(
                    Quat::from_mat3(&prev.basis).slerp(Quat::from_mat3(&pose.basis), alpha),
                );
            }
            poses.push(pose.clone());
            let p = &race.pilots[i];
            let input = race.drivers[i].input.drive;
            let base = Affine3A::from_mat3_translation(pose.basis, pose.origin);
            let b = self.vehicle_aux + 1 + i*24;
            let record = &mut self.scene.aux[b..b+24];
            record[2] = (1.0 - (p.energy as f32 / 100.).clamp(0.,1.)).to_bits();
            record[3] = super::respawn::progress(race,i,alpha).to_bits();
            for (offset,v) in [(4,pose.origin),(8,pose.basis.x_axis),(12,pose.basis.y_axis),(16,pose.basis.z_axis)] {
                for (j,f) in v.to_array().into_iter().enumerate() {record[offset+j]=f.to_bits();}
            }
            record[20] = (2f32.powf(race.environment.exposure)*8.).to_bits();

            let new_crash = race.drivers[i].motion.crash_id > self.crashes[i];
            let mut wreck_parts = Vec::new();
            for part in &self.ships[i] {
                let mut xf = base;
                for (name, local) in &part.chain {
                    let local =
                        super::jets::control_transform(name, *local, input, input.brake as f32);
                    xf *= local;
                }
                if new_crash && let Some((mesh, center, slot)) = part.wreck {
                    wreck_parts.push((mesh, slot, xf * Affine3A::from_translation(center)));
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
                    if mounting.is_some() && !p.crashed && p.recovery<=0. {
                        Vec3::ONE
                    } else {
                        Vec3::splat(0.001)
                    },
                    Quat::IDENTITY,
                    Vec3::from_array(self.jets.config.socket),
                ),
            );
            if new_crash {
                self.crashes[i] = race.drivers[i].motion.crash_id;
                self.fx.explosion(
                    time,
                    pose.origin,
                    self.crash_fx.config.crash_radius,
                    Vec3::new(1., 0.3, 0.05),
                );
                self.crash_fx.spawn(
                    race,
                    i,
                    time,
                    &wreck_parts,
                    &mut self.scene,
                    &mut self.slots,
                )?;
            }
        }
        self.weapon_art.update(&mut self.scene,&mut self.slots,race,&poses,time,alpha);
        self.jets.update(race, &poses, time);
        for (k, p) in race.pickups.iter().enumerate() {
            let n = race.world.track.sample(p.distance);
            let size = if p.cooldown > 0. { 0.001 } else { 3. };
            self.slots.set(
                &mut self.scene,
                self.pickup_slots[k],
                self.pickup_mesh,
                Affine3A::from_scale_rotation_translation(
                    Vec3::splat(size),
                    Quat::from_rotation_y(time) * Quat::from_rotation_z(0.7),
                    n.point(p.x, 4.),
                ),
            );
        }
        for (k, id) in self.projectiles.iter().enumerate() {
            let xf = if let Some(p) = race.projectiles.get(k) {
                Affine3A::from_scale_rotation_translation(
                    Vec3::ONE,
                    Quat::from_rotation_arc(Vec3::Z, p.velocity.try_normalize().unwrap_or(Vec3::Z)),
                    p.previous.lerp(p.position, alpha),
                )
            } else {
                Affine3A::from_scale(Vec3::splat(0.001))
            };
            self.slots.set(
                &mut self.scene,
                *id,
                race.projectiles
                    .get(k)
                    .map_or(self.projectile_mesh, |p| self.mount_meshes[p.kind as usize]),
                xf,
            );
        }
        if fresh {
            for p in &race.projectiles {
                if p.kind == Weapon::Missile {
                    if let Some(target) = p.target {
                        let goal = race.pose(target).origin;
                        let distance = goal.distance(p.position);
                        let eta = distance / p.velocity.length().max(80.);
                        if p.locked && eta < 0.75 && !p.evaded {
                            let _ = self.fx.streak(render::fx::AdditiveStreak {
                                position: (goal + p.position) * 0.5,
                                travel: (goal - p.position).normalize_or_zero() * 0.001,
                                birth: time - 0.001,
                                life: 1. / 60.,
                                radius: [0.012; 2],
                                length: [distance; 2],
                                color: Vec3::new(1., 0.72, 0.52),
                                radiance: 12000. * (1. - eta / 0.75).powi(2),
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
                                radiance: 65000. * (1. - eta / 0.75).powi(2),
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
            if birth > self.blast_stamp && b.kind != Weapon::Emp {
                self.fx.explosion(
                    time - b.age as f32,
                    b.position,
                    b.radius * 0.3,
                    Vec3::new(1., 0.3, 0.05),
                );
            }
        }
        self.blast_stamp = race.clock;
        self.crash_fx
            .update(race, time, &mut self.scene, &mut self.slots, &mut self.fx);
        Ok(())
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
    out: &mut Vec<LoadedPart>,
    face_code: i32,
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
                    if node["instance_parameters"]["face_surface"].as_bool() == Some(true) {
                        copy.scale = copy.scale.signum() * face_code as f32;
                    }
                    mat = scene.add_material(copy);
                }
                if name == "Canopy" {
                    mat = scene.add_material(Material {
                        tint: [0.012, 0.035, 0.052],
                        layer: -19,
                        roughness: 0.12,
                        metal: 0.55,
                        flags: SURFACE | CLEARCOAT,
                        scale: 1.,
                    });
                }
                let mut p = Primitive {
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
                let lo = p
                    .vertices
                    .iter()
                    .fold(Vec3::splat(f32::INFINITY), |a, v| a.min(v.pos.into()));
                let hi = p
                    .vertices
                    .iter()
                    .fold(Vec3::splat(f32::NEG_INFINITY), |a, v| a.max(v.pos.into()));
                let extent = hi - lo;
                let mut edges = extent.to_array();
                edges.sort_by(f32::total_cmp);
                let wreck = if edges[2] > 0.6
                    && edges[1] > 0.2
                    && scene.materials[mat as usize].flags & EMISSIVE == 0
                {
                    let center = (lo + hi) * 0.5;
                    let tint = scene.materials[mat as usize].tint;
                    p.material = super::visuals::surface(scene, 23, tint, 0.8, 1.);
                    for v in &mut p.vertices {
                        v.pos = (Vec3::from(v.pos) - center).to_array();
                    }
                    Some((scene.add_mesh(&p.mesh()), center))
                } else {
                    None
                };
                out.push((mesh, wreck, chain.clone()));
            }
        }
    }
    for child in node["children"].as_array().into_iter().flatten() {
        load_node(scene, data, child, mats, chain, visible, out, face_code)?;
    }
    chain.pop();
    Ok(())
}

#[cfg(test)]
mod road_tests {
    use super::*;
    #[test]
    fn split_ribbons_cover_supported_surface_without_covering_the_gap() {
        for seed in [89502,35,80385] {
            let race=Race::new(seed,"hard","forest",1,3);
            let nodes=&race.world.track.nodes;
            for (i,a) in nodes.iter().enumerate() {
                let b=&nodes[(i+1)%nodes.len()];
                if a.air_gap || a.split_gap.max(b.split_gap)<0.01 {continue;}
                for t in [0.1,0.5,0.9] {
                    let node=race.world.track.sample((i as f64+t)*race.world.track.step);
                    for k in 0..101 {
                        let x=(-1.+(k as f64+0.5)*2./101.)*node.geometry.width;
                        // Collision intentionally treats sub-centimetre split openings as closed.
                        if (x.abs()-node.split_gap).abs()<0.011 {continue;}
                        let mut covers=0;
                        for side in [-1.,1.] {
                            if a.dead_gap && a.dead_side*side>0. {continue;}
                            for lane in 0..8 {
                                let q=road_span(a,b,side,lane as f64/8.,(lane+1) as f64/8.);
                                let lo=q[0]+(q[3]-q[0])*t;let hi=q[1]+(q[2]-q[1])*t;
                                if x>=lo && x<hi {covers+=1;}
                            }
                        }
                        assert_eq!(covers,usize::from(node.supported(x,0.)),"seed {seed}, node {i}, x {x}, t {t}");
                    }
                }
            }
        }
    }
    #[test]
    fn inner_guardrail_meets_exact_deck_boundary_at_both_ends() {
        let race=Race::new(89502,"hard","forest",1,3);
        let a=race.world.track.sample(4854.);let b=race.world.track.sample(4854.+race.world.track.step);
        assert!(a.split_gap>0.01);
        let mut parts:Vec<_>=(0..5).map(|_|Primitive::default()).collect();
        road_edge(&mut parts,&a,&b,a.split_gap,b.split_gap,1.,2.4,2.4);
        for (j,node) in [(0,&a),(1,&a),(2,&b),(3,&b)] {
            let p=Vec3::from(parts[2].vertices[j].pos);
            assert!((node.lateral(p)-node.split_gap).abs()<0.001);
        }
        assert_eq!(parts[1].indices.len(),6,"one unbroken neon strip per edge");
    }
}
