//! Source-based stage composition. Luau owns placement; Rust handles shared
//! geometry, spatial clearance, instancing and per-frame analytic transforms.
use super::{
    stage_mesh, stage_terrain,
    visuals::{material, quad, surface},
};
use glam::{Affine3A, Mat3, Quat, Vec3};
use render::{
    Scene,
    headlights::LocalLight,
    primitives::Primitive,
    scene::{ALPHA_TEST, EMISSIVE, Material, THIN_FOLIAGE},
    slots::{SlotId, Slots},
};
use script::{
    Host, Limits,
    mlua::{self, LuaSerdeExt},
};
use spaceracer::{
    environment::{Landscape, Obstacle},
    race::Race,
    track::Node,
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
struct Request {
    mesh: String,
    mat: String,
    xf: Affine3A,
    solid: bool,
    motion: Option<(String, f32, f32, f32)>,
}
#[derive(Clone, Copy)]
struct Bounds {
    lo: Vec3,
    hi: Vec3,
}
// Shared planting probes from the rendered geometry, including overhanging bulbs
// and broad leaves. Evaluated only during generation, never during a frame.
struct PlantShape {
    bounds: Bounds,
    probes: Vec<Vec3>,
}
#[derive(Clone, Copy, serde::Deserialize)]
struct PlantClearance {
    root_band: f32,
    max_slope_degrees: f32,
    clearance: f32,
    road_margin: f32,
}
impl PlantClearance {
    fn valid(self) -> bool {
        (0.0..=0.1).contains(&self.root_band)
            && (0.0..=80.).contains(&self.max_slope_degrees)
            && (0.0..=5.).contains(&self.clearance)
            && (0.0..=100.).contains(&self.road_margin)
    }
}
impl PlantShape {
    fn new(name: &str) -> Self {
        let mut scratch = Scene::default();
        let mut probes = Vec::new();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for part in stage_mesh::geometry(&mut scratch, name, 0) {
            for v in &part.vertices {
                let p = Vec3::from(v.pos);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            for triangle in part.indices.chunks_exact(3) {
                let a = Vec3::from(part.vertices[triangle[0] as usize].pos);
                let b = Vec3::from(part.vertices[triangle[1] as usize].pos);
                let c = Vec3::from(part.vertices[triangle[2] as usize].pos);
                let steps = (a.distance(b).max(b.distance(c)).max(c.distance(a)) / 0.035)
                    .ceil()
                    .max(1.) as usize;
                for i in 0..=steps {
                    for j in 0..=steps - i {
                        probes.push(
                            a + (b - a) * (i as f32 / steps as f32)
                                + (c - a) * (j as f32 / steps as f32),
                        );
                    }
                }
            }
        }
        Self {
            bounds: Bounds { lo, hi },
            probes,
        }
    }
    fn clears_ground(
        &self,
        terrain: &Landscape,
        transform: Affine3A,
        rules: PlantClearance,
    ) -> bool {
        let root = transform.translation;
        // Do not plant upright stalks into a cliff face. The small buried root
        // band anchors them naturally; the rest of the geometry must be exposed.
        if terrain.normal(root.x, root.z).y < rules.max_slope_degrees.to_radians().cos() {
            return false;
        }
        self.probes
            .iter()
            .filter(|p| p.y > rules.root_band)
            .all(|&p| {
                let p = transform.transform_point3(p);
                p.y > terrain.height(p.x, p.z) + rules.clearance
            })
    }
}
impl Bounds {
    fn center(c: Vec3, h: Vec3) -> Self {
        Self {
            lo: c - h,
            hi: c + h,
        }
    }
    fn intersects(self, b: Self) -> bool {
        self.lo.cmple(b.hi).all() && self.hi.cmpge(b.lo).all()
    }
    fn transformed(self, xf: Affine3A) -> Self {
        let center = xf.transform_point3((self.lo + self.hi) * 0.5);
        let h = Mat3::from(xf.matrix3).abs() * ((self.hi - self.lo) * 0.5);
        Self::center(center, h)
    }
}
#[derive(Clone)]
struct Animated {
    mesh: usize,
    slot: SlotId,
    xf: Affine3A,
    kind: String,
    phase: f32,
    speed: f32,
    length: f32,
}
struct Walker {
    center: Vec3,
    size: f32,
    phase: f32,
    parts: Vec<(usize, SlotId)>,
}
pub struct Stage {
    pub landscape: Arc<Landscape>,
    pub obstacles: Vec<Obstacle>,
    pub lamps: Vec<LocalLight>,
    dynamic: Vec<(usize, Request)>,
    animated: Vec<Animated>,
    walkers: Vec<Walker>,
    walker_data: Vec<(Vec3, f32, f32)>,
    walker_meshes: [usize; 3],
    pub counts: HashMap<String, usize>,
    octopuses: Vec<super::ocean_life::Octopus>,
}
fn vec(v: mlua::Vector) -> Vec3 {
    Vec3::new(v.x(), v.y(), v.z())
}
fn lv(v: Vec3) -> mlua::Vector {
    mlua::Vector::new(v.x, v.y, v.z)
}
fn xf(p: mlua::Vector, s: mlua::Vector, r: mlua::Vector) -> Affine3A {
    let r = vec(r);
    Affine3A::from_scale_rotation_translation(
        vec(s),
        Quat::from_euler(glam::EulerRot::YXZ, r.y, r.x, r.z),
        vec(p),
    )
}
fn link(a: Vec3, b: Vec3, width: f32) -> Affine3A {
    Affine3A::from_scale_rotation_translation(
        Vec3::new(width, a.distance(b), width),
        Quat::from_rotation_arc(Vec3::Y, (b - a).normalize_or_zero()),
        (a + b) * 0.5,
    )
}
fn node_table(lua: &mlua::Lua, n: &Node) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    t.set("p", lv(n.geometry.p))?;
    t.set("r", lv(n.geometry.basis().x_axis))?;
    t.set("up", lv(n.geometry.basis().y_axis))?;
    t.set("w", n.geometry.width)?;
    Ok(t)
}
fn palette(hex: &str) -> [f32; 3] {
    let v = u32::from_str_radix(hex, 16).unwrap();
    [
        ((v >> 16) & 255) as f32 / 255.,
        ((v >> 8) & 255) as f32 / 255.,
        (v & 255) as f32 / 255.,
    ]
}
fn mat(scene: &mut Scene, name: &str, cache: &mut HashMap<String, u32>) -> u32 {
    if let Some(&m) = cache.get(name) {
        return m;
    }
    let night = 90.;
    let m = if let Some(n) = name.strip_prefix("facade") {
        let k = n.parse::<u64>().unwrap_or(0);
        let mut rng = gdcompat::Pcg::new(k * 129 + 31);
        surface(scene, 1, [rng.randf(), rng.randf(), rng.randf()], 0.55, 1.)
    } else if let Some(n) = name.strip_prefix("neon") {
        let k = n.parse::<usize>().unwrap_or(0) % 6;
        material(
            scene,
            palette(["b6d4f0", "f89466", "ecd378", "76d6c9", "bd82f2", "699ddd"][k]),
            0.5,
            42.,
        )
    } else if name.starts_with("traffic") {
        let k = name[7..].parse::<usize>().unwrap_or(0) % 6;
        material(
            scene,
            palette(["8996a1", "3d5268", "753b40", "ad9560", "ced3cc", "344641"][k]),
            0.45,
            0.,
        )
    } else if name.starts_with("fish") {
        material(
            scene,
            palette(
                ["bfc6cb", "bfa833", "b76833", "679cc7", "b57eaf"]
                    [name[4..].parse::<usize>().unwrap_or(0) % 5],
            ),
            0.5,
            0.,
        )
    } else {
        match name {
            "membrane" => surface(scene, 9, [0.1, 0.3, 0.26], 1., 1.),
            "metal" => material(scene, [0.035, 0.05, 0.075], 0.55, 0.),
            "beacon" => material(scene, [1., 0.06, 0.025], 0.5, night),
            "fern" => material(scene, [0.075, 0.20, 0.035], 0.95, 0.),
            "moss" => surface(scene, 18, [0.12, 0.15, 0.095], 0.9, 1.),
            "bubble" => surface(scene, 22, [0.06, 0.22, 0.25], 0.12, 1.),
            "cactus" => material(scene, palette("6c8a4f"), 0.95, 0.),
            "sandstone" => surface(scene, 4, [0.55, 0.32, 0.15], 0.85, 1.),
            "reef" => surface(scene, 16, [0.16, 0.26, 0.23], 0.85, 1.),
            "kelp" => material(scene, [0.22, 0.26, 0.07], 0.95, 0.),
            "lantern" => material(scene, [0.20, 0.10, 0.24], 0.7, 0.),
            "fan" => material(scene, [0.65, 0.12, 0.22], 0.7, 0.),
            "sponge" => material(scene, [0.65, 0.24, 0.09], 0.8, 0.),
            "jelly" => surface(scene, 5, [0.35, 0.65, 0.7], 0.4, 18.),
            "manta" => material(scene, [0.10, 0.15, 0.2], 0.6, 0.),
            "tissue0" => surface(scene, 5, palette("d986a2"), 0.48, 1.),
            "tissue1" => surface(scene, 5, palette("d99766"), 0.48, 1.),
            "tissue2" => surface(scene, 5, palette("6cc5b4"), 0.48, 1.),
            "tissue3" => surface(scene, 5, palette("ad8ad1"), 0.48, 1.),
            "nucleus" => surface(scene, 5, palette("8d78ac"), 0.45, 2.),
            "fold" => surface(scene, 5, palette("ffd49a"), 0.5, 2.),
            "microtubule" => surface(scene, 5, palette("61afa8"), 0.5, 1.),
            _ => material(scene, [0.3, 0.4, 0.35], 0.7, 0.),
        }
    };
    if name == "membrane" {
        scene.materials[m as usize].flags |= render::scene::NO_AMBIENT_OCCLUSION;
    }
    cache.insert(name.into(), m);
    m
}
impl Stage {
    pub fn build(scene: &mut Scene, race: &Race, root: &Path) -> Result<Self, String> {
        // Stable texture slots for the faithful facade and road-reflection shader.
        for name in [
            "office-facade.png",
            "corporate-posters.png",
            "city-billboards.png",
        ] {
            scene.add_texture(&root.join(format!("source/assets/{name}")))?;
        }
        let landscape = stage_terrain::generate(race);
        let water = race
            .world
            .track
            .nodes
            .iter()
            .map(|n| n.geometry.p.y - 24.)
            .fold(f32::INFINITY, f32::min);
        let ground_mat = match race.biome.as_str() {
            "city" => surface(scene, 7, [0.025, 0.03, 0.04], 0.8, 1.),
            "forest" => surface(scene, 6, [0.13, 0.24, 0.075], 0.95, water),
            "desert" => surface(scene, 8, [0.55, 0.36, 0.18], 0.95, 1.),
            "ocean" => surface(scene, 15, [0.16, 0.29, 0.27], 0.9, 1.),
            _ => surface(scene, 6, [0.04, 0.1, 0.09], 0.8, 1.),
        };
        stage_terrain::mesh(scene, &landscape, ground_mat);
        if race.biome == "forest" {
            let shore = image::RgbaImage::from_fn(256, 256, |x, z| {
                let h = landscape.height(
                    x as f32 / 255. * 7680. - 3840.,
                    z as f32 / 255. * 7680. - 3840.,
                );
                let v = ((-(water - h).max(0.) / 14.).exp() * 255.) as u8;
                image::Rgba([v, v, v, 255])
            });
            scene.add_image("spaceracer-shore", shore);
        }
        if race.biome == "forest" {
            let m = surface(scene, 3, [0.008, 0.065, 0.09], 0.16, 1.);
            let mut p = Primitive {
                material: m,
                ..Default::default()
            };
            quad(
                &mut p,
                [
                    Vec3::new(-6000., water, -6000.),
                    Vec3::new(6000., water, -6000.),
                    Vec3::new(6000., water, 6000.),
                    Vec3::new(-6000., water, 6000.),
                ],
                [Vec3::Y; 4],
                [[0.; 2]; 4],
            );
            let mesh = scene.add_mesh(&p.mesh());
            scene.add_instance(mesh, Affine3A::IDENTITY);
        }
        let mut models = Vec::new();
        let mut tree_xf = Vec::new();
        if race.biome == "forest" {
            for name in ["pine-tree-1", "birch-tree-1"] {
                let model = assets::gltf::load(
                    scene,
                    &root.join(format!("source/assets/forest/authored/{name}/scene.gltf")),
                )?;
                let h = (model.max.y - model.min.y).max(0.01);
                let center = (model.min + model.max) * 0.5;
                tree_xf.push(
                    Affine3A::from_scale(Vec3::splat(1. / h))
                        * Affine3A::from_translation(-Vec3::new(center.x, model.min.y, center.z)),
                );
                for &id in &model.materials {
                    let m = &mut scene.materials[id as usize];
                    m.metal = 0.;
                    m.roughness = 0.95;
                    if m.flags & ALPHA_TEST != 0 {
                        m.flags |= THIN_FOLIAGE;
                        if name.starts_with("pine") {
                            m.tint = [0.55, 0.72, 0.48];
                        }
                    }
                }
                models.push(model);
            }
        }
        let nodes = Arc::new(race.world.track.nodes.clone());
        let road: Arc<Vec<Bounds>> = Arc::new(
            (0..nodes.len())
                .map(|i| {
                    let a = &nodes[i];
                    let b = &nodes[(i + 1) % nodes.len()];
                    let mut pad = Vec3::splat(a.geometry.width.max(b.geometry.width) as f32 + 24.);
                    let mut hi = a.geometry.p.max(b.geometry.p);
                    if a.air_gap || a.feature == "flight" || a.feature == "jump" {
                        pad += Vec3::splat(35.);
                        hi = hi.max(a.geometry.p + Vec3::Y * 140.);
                    }
                    Bounds {
                        lo: a.geometry.p.min(b.geometry.p) - pad,
                        hi: hi + pad,
                    }
                })
                .collect(),
        );
        let requests = Arc::new(Mutex::new(Vec::<Request>::new()));
        let plants: HashMap<_, _> = if race.biome == "ocean" {
            ["kelp", "lantern", "fan", "sponge"]
                .into_iter()
                .map(|name| (name.to_owned(), PlantShape::new(name)))
                .collect()
        } else {
            HashMap::new()
        };
        let occupied = Arc::new(Mutex::new(Vec::<Bounds>::new()));
        let lamps = Arc::new(Mutex::new(Vec::<LocalLight>::new()));
        let walkers = Arc::new(Mutex::new(Vec::<(Vec3, f32, f32)>::new()));
        let octopuses = Arc::new(Mutex::new(Vec::<super::ocean_life::Site>::new()));
        let mut host = Host::new(
            Limits {
                memory: 96 << 20,
                steps: 8_000_000,
            },
            |lua, api| {
                let rng = Arc::new(Mutex::new(gdcompat::Pcg::new(1)));
                let r = rng.clone();
                api.set(
                    "seed",
                    lua.create_function(move |_, s: u64| {
                        r.lock().unwrap().seed(s);
                        Ok(())
                    })?,
                )?;
                let r = rng.clone();
                api.set(
                    "random",
                    lua.create_function(move |_, ()| Ok(r.lock().unwrap().randf()))?,
                )?;
                api.set(
                    "integer",
                    lua.create_function(move |_, (a, b): (i32, i32)| {
                        Ok(rng.lock().unwrap().randi_range(a, b))
                    })?,
                )?;
                let r = road.clone();
                api.set(
                    "clear",
                    lua.create_function(move |_, (p, h): (mlua::Vector, mlua::Vector)| {
                        Ok(!r
                            .iter()
                            .any(|b| b.intersects(Bounds::center(vec(p), vec(h)))))
                    })?,
                )?;
                let r = road.clone();
                let o = occupied.clone();
                api.set(
                    "free",
                    lua.create_function(move |_, (p, h): (mlua::Vector, mlua::Vector)| {
                        let b = Bounds::center(vec(p), vec(h));
                        Ok(!r.iter().any(|x| x.intersects(b))
                            && !o.lock().unwrap().iter().any(|x| x.intersects(b)))
                    })?,
                )?;
                let o = occupied.clone();
                api.set(
                    "occupy",
                    lua.create_function(move |_, (p, h): (mlua::Vector, mlua::Vector)| {
                        o.lock().unwrap().push(Bounds::center(vec(p), vec(h)));
                        Ok(())
                    })?,
                )?;
                let n = nodes.clone();
                api.set(
                    "node",
                    lua.create_function(move |lua, i: usize| node_table(lua, &n[i % n.len()]))?,
                )?;
                let n = nodes.clone();
                api.set(
                    "sample",
                    lua.create_function(move |lua, u: f64| {
                        node_table(
                            lua,
                            &n[((u.rem_euclid(1.) * n.len() as f64) as usize) % n.len()],
                        )
                    })?,
                )?;
                let n = nodes.clone();
                api.set(
                    "nearest",
                    lua.create_function(move |lua, p: mlua::Vector| {
                        let p = vec(p);
                        let nearest = n
                            .iter()
                            .min_by(|a, b| {
                                let da = a.geometry.p - p;
                                let db = b.geometry.p - p;
                                (da.x * da.x + da.z * da.z).total_cmp(&(db.x * db.x + db.z * db.z))
                            })
                            .unwrap();
                        node_table(lua, nearest)
                    })?,
                )?;
                let t = landscape.clone();
                api.set(
                    "ground",
                    lua.create_function(move |_, (x, z): (f32, f32)| Ok(t.height(x, z)))?,
                )?;
                let r = road.clone();
                api.set(
                    "tree_clear",
                    lua.create_function(
                        move |_,
                              (_variant, p, s, rot): (
                            usize,
                            mlua::Vector,
                            mlua::Vector,
                            mlua::Vector,
                        )| {
                            let transform = xf(p, s, rot);
                            let crown =
                                Bounds::center(Vec3::new(0., 0.7, 0.), Vec3::new(0.22, 0.3, 0.22))
                                    .transformed(transform);
                            let trunk = Bounds::center(
                                Vec3::new(0., 0.3, 0.),
                                Vec3::new(0.025, 0.3, 0.025),
                            )
                            .transformed(transform);
                            Ok(!r.iter().any(|b| b.intersects(crown) || b.intersects(trunk)))
                        },
                    )?,
                )?;
                let r = road.clone();
                let terrain = landscape.clone();
                api.set(
                    "mesh_clear",
                    lua.create_function(
                        move |lua,
                              (name, p, s, rot, options): (
                            String,
                            mlua::Vector,
                            mlua::Vector,
                            mlua::Vector,
                            Option<mlua::Table>,
                        )| {
                            let transform = xf(p, s, rot);
                            if let Some(plant) = plants.get(&name) {
                                let rules: PlantClearance =
                                    lua.from_value(mlua::Value::Table(options.ok_or_else(
                                        || mlua::Error::runtime("plant clearance rules required"),
                                    )?))?;
                                if !rules.valid() {
                                    return Err(mlua::Error::runtime(
                                        "invalid plant clearance rules",
                                    ));
                                }
                                let mut bounds = plant.bounds.transformed(transform);
                                bounds.lo -= Vec3::splat(rules.road_margin);
                                bounds.hi += Vec3::splat(rules.road_margin);
                                return Ok(!r.iter().any(|b| b.intersects(bounds))
                                    && plant.clears_ground(&terrain, transform, rules));
                            }
                            let (c, h) = match name.as_str() {
                                "dna" => (Vec3::ZERO, Vec3::new(0.14, 0.52, 0.14)),
                                "protein" => (Vec3::ZERO, Vec3::new(0.45, 0.3, 0.45)),
                                "reticulum" => (Vec3::ZERO, Vec3::new(0.45, 0.17, 0.45)),
                                "arch" => (Vec3::new(0., 0.55, 0.), Vec3::new(0.6, 0.65, 0.1)),
                                "kelp" | "lantern" | "fan" | "sponge" => {
                                    (Vec3::new(0., 0.5, 0.), Vec3::new(0.3, 0.6, 0.3))
                                }
                                _ if name.starts_with("rock") => {
                                    (Vec3::new(0., 0.5, 0.), Vec3::new(0.4, 0.6, 0.4))
                                }
                                _ => (Vec3::ZERO, Vec3::splat(0.5)),
                            };
                            let mut b = Bounds::center(c, h).transformed(xf(p, s, rot));
                            b.lo -= Vec3::splat(8.);
                            b.hi += Vec3::splat(8.);
                            Ok(!r.iter().any(|x| x.intersects(b)))
                        },
                    )?,
                )?;
                let q = requests.clone();
                api.set(
                    "emit",
                    lua.create_function(
                        move |_,
                              (mesh, mat, p, s, r, solid): (
                            String,
                            String,
                            mlua::Vector,
                            mlua::Vector,
                            mlua::Vector,
                            bool,
                        )| {
                            if q.lock().unwrap().len() > 100_000 {
                                return Err(mlua::Error::runtime("stage instance budget"));
                            }
                            q.lock().unwrap().push(Request {
                                mesh,
                                mat,
                                xf: xf(p, s, r),
                                solid,
                                motion: None,
                            });
                            Ok(())
                        },
                    )?,
                )?;
                let q = requests.clone();
                api.set(
                    "moving",
                    lua.create_function(
                        move |_,
                              (mesh, mat, p, s, r, kind, phase, speed, length): (
                            String,
                            String,
                            mlua::Vector,
                            mlua::Vector,
                            mlua::Vector,
                            String,
                            f32,
                            f32,
                            f32,
                        )| {
                            q.lock().unwrap().push(Request {
                                mesh,
                                mat,
                                xf: xf(p, s, r),
                                solid: false,
                                motion: Some((kind, phase, speed, length)),
                            });
                            Ok(())
                        },
                    )?,
                )?;
                let q = requests.clone();
                api.set(
                    "beam",
                    lua.create_function(
                        move |_, (mat, a, b, width): (String, mlua::Vector, mlua::Vector, f32)| {
                            q.lock().unwrap().push(Request {
                                mesh: "rod".into(),
                                mat,
                                xf: link(vec(a), vec(b), width),
                                solid: false,
                                motion: None,
                            });
                            Ok(())
                        },
                    )?,
                )?;
                let q = requests.clone();
                api.set(
                    "pore",
                    lua.create_function(move |_, (p, n): (mlua::Vector, mlua::Vector)| {
                        q.lock().unwrap().push(Request {
                            mesh: "pore".into(),
                            mat: "tissue0".into(),
                            xf: Affine3A::from_scale_rotation_translation(
                                Vec3::splat(13.),
                                Quat::from_rotation_arc(Vec3::Y, vec(n)),
                                vec(p),
                            ),
                            solid: false,
                            motion: None,
                        });
                        Ok(())
                    })?,
                )?;
                let w = walkers.clone();
                api.set(
                    "walker",
                    lua.create_function(move |_, (p, size, phase): (mlua::Vector, f32, f32)| {
                        w.lock().unwrap().push((vec(p), size, phase));
                        Ok(())
                    })?,
                )?;
                let animals = octopuses.clone();
                api.set(
                    "octopus",
                    lua.create_function(
                        move |_, (p, size, yaw, phase): (mlua::Vector, f32, f32, f32)| {
                            let mut sites = animals.lock().unwrap();
                            if sites.len() >= 4
                                || !(16.0..=40.0).contains(&size)
                                || !vec(p).is_finite()
                                || !yaw.is_finite()
                                || !phase.is_finite()
                            {
                                return Err(mlua::Error::external("invalid octopus site"));
                            }
                            sites.push(super::ocean_life::Site {
                                position: vec(p),
                                size,
                                yaw,
                                phase,
                            });
                            Ok(())
                        },
                    )?,
                )?;
                let l = lamps.clone();
                api.set("lamp",lua.create_function(move|_,(p,color,power,range):(mlua::Vector,mlua::Vector,f32,f32)|{l.lock().unwrap().push(LocalLight{position:vec(p),direction:Vec3::Y,intensity_cd:vec(color)*power,range_m:range,inner_cos:1.,outer_cos:-1.,source_radius_m:1.,falloff:1.8});Ok(())})?)?;
                Ok(())
            },
        )?;
        let source = std::fs::read_to_string(root.join("../rules/stages.luau"))
            .map_err(|e| e.to_string())?;
        host.add_source("stages.luau", &source)?;
        let function = host.modules()[0]
            .exports
            .as_ref()
            .unwrap()
            .get::<mlua::Function>("build")
            .map_err(|e| e.to_string())?;
        host.run(0,"build",|lua|function.call::<()>(lua.to_value(&serde_json::json!({"seed":race.world.track.seed,"biome":race.biome,"nodes":nodes.len(),"water":water}))?)).ok_or_else(||host.drain_log().join("\n"))?;
        let mut materials = HashMap::new();
        let mut meshes = HashMap::<(String, String), (usize, Vec3, Vec3)>::new();
        let mut obstacles = Vec::new();
        let mut dynamic = Vec::new();
        let mut counts = HashMap::new();
        let mut posters = Vec::new();
        let mut buildings = Vec::new();
        for r in requests.lock().unwrap().iter() {
            *counts.entry(r.mesh.clone()).or_insert(0) += 1;
            let (mesh, lo, hi) = if r.mesh.starts_with("tree") {
                let k = r.mesh[4..].parse::<usize>().unwrap_or(0);
                let model = &models[k];
                (
                    model.mesh,
                    Vec3::new(-0.025, 0., -0.025),
                    Vec3::new(0.025, 0.5, 0.025),
                )
            } else {
                let key = (r.mesh.clone(), r.mat.clone());
                if let Some(v) = meshes.get(&key) {
                    *v
                } else {
                    let v = if let Some(n) = r.mesh.strip_prefix("poster") {
                        let tile = n.parse::<usize>().unwrap();
                        let (layer, u, v, w, h) = if tile < 6 {
                            (
                                1,
                                (tile % 3) as f32 / 3.,
                                (tile / 3) as f32 / 2.,
                                1. / 3.,
                                0.5,
                            )
                        } else {
                            (2, (tile - 6) as f32 / 4., 0., 0.25, 1.)
                        };
                        let m = scene.add_material(Material {
                            tint: [1.; 3],
                            layer,
                            roughness: 0.5,
                            metal: 0.,
                            flags: EMISSIVE,
                            scale: 145.,
                        });
                        let mut p = Primitive {
                            material: m,
                            ..Default::default()
                        };
                        quad(
                            &mut p,
                            [
                                Vec3::new(-0.5, -0.5, 0.),
                                Vec3::new(0.5, -0.5, 0.),
                                Vec3::new(0.5, 0.5, 0.),
                                Vec3::new(-0.5, 0.5, 0.),
                            ],
                            [Vec3::Z; 4],
                            [[u, v + h], [u + w, v + h], [u + w, v], [u, v]],
                        );
                        (
                            scene.add_mesh(&p.mesh()),
                            Vec3::new(-0.5, -0.5, -0.01),
                            Vec3::new(0.5, 0.5, 0.01),
                        )
                    } else {
                        let m = mat(scene, &r.mat, &mut materials);
                        stage_mesh::build(scene, &r.mesh, m)
                    };
                    meshes.insert(key, v);
                    v
                }
            };
            if r.motion.is_some() {
                dynamic.push((mesh, r.clone()));
                continue;
            }
            let transform = if r.mesh.starts_with("tree") {
                r.xf * tree_xf[r.mesh[4..].parse::<usize>().unwrap_or(0)]
            } else {
                r.xf
            };
            let first = scene.instances.len();
            scene.add_instance(mesh, transform);
            if r.mesh == "membrane" {
                for i in &mut scene.instances[first..] {
                    i.flags |= render::scene::NO_SHADOW;
                }
            }
            if r.solid {
                obstacles.push(Obstacle::oriented(lo,hi,r.xf));
            }
            if r.mesh.starts_with("poster") {
                posters.push(r.clone());
            }
            if r.mat.starts_with("facade") && r.mesh == "box" {
                let b = Bounds { lo, hi }.transformed(r.xf);
                buildings.push((b, scene.materials[materials[&r.mat] as usize].tint));
            }
        }
        if race.biome == "city" {
            reflection_data(scene, &posters, &buildings);
        } else {
            scene.aux = vec![0; 8];
        }
        // Source ambient illumination is independent of visible sky brightness.
        // An authored directional ambient cube, not a claim of baked bounce GI.
        if race.environment.ambient > 0. {
            use render::irradiance_volume::{Grid, Probe, pack};
            let grid = Grid {
                origin: Vec3::splat(-8192.),
                cell: 8192.,
                dims: [4, 4, 4],
            };
            scene.reserve_irradiance_volume(grid)?;
            let level = race.environment.ambient;
            let tint = if race.biome == "city" {
                Vec3::new(0.86, 0.92, 1.)
            } else {
                Vec3::new(0.80, 1., 0.91)
            };
            let probe = Probe {
                axes: [
                    tint * level * 0.7,
                    tint * level * 0.7,
                    tint * level,
                    tint * level * 0.22,
                    tint * level * 0.7,
                    tint * level * 0.7,
                ],
            };
            let words = pack(&grid, &vec![probe; grid.count()])?;
            let offset = scene.irradiance_volume.unwrap().0;
            scene.aux[offset..offset + words.len()].copy_from_slice(&words);
        }
        let mut walker_meshes = [0; 3];
        for (i, (shape, material_name)) in [
            ("sphere", "tissue0"),
            ("rod", "fold"),
            ("sphere", "tissue2"),
        ]
        .into_iter()
        .enumerate()
        {
            let m = mat(scene, material_name, &mut materials);
            walker_meshes[i] = stage_mesh::build(scene, shape, m).0;
        }
        println!(
            "stage {}: {} static/dynamic parts, {} traffic/creatures, {} walkers",
            race.biome,
            requests.lock().unwrap().len(),
            dynamic.len(),
            walkers.lock().unwrap().len()
        );
        let animals: Vec<_> = octopuses
            .lock()
            .unwrap()
            .iter()
            .map(|&site| {
                let bounds = Bounds {
                    lo: Vec3::new(-1.2, 0., -1.2) * site.size,
                    hi: Vec3::new(1.2, 2.85, 1.4) * site.size,
                }
                .transformed(Affine3A::from_rotation_translation(
                    Quat::from_rotation_y(site.yaw),
                    site.position,
                ));
                obstacles.push(Obstacle::bounds(bounds.lo,bounds.hi));
                super::ocean_life::Octopus::add(scene, site)
            })
            .collect();
        counts.insert("octopus".into(), animals.len());
        let walker_data = walkers.lock().unwrap().clone();
        let lamps = lamps.lock().unwrap().clone();
        Ok(Self {
            landscape,
            obstacles,
            lamps,
            dynamic,
            animated: Vec::new(),
            walkers: Vec::new(),
            walker_data,
            walker_meshes,
            counts,
            octopuses: animals,
        })
    }
    pub fn capacity(&self) -> usize {
        self.dynamic.len() + self.walker_data.len() * 20
    }
    pub fn reserve(&mut self, scene: &mut Scene, slots: &mut Slots) {
        for (mesh, r) in &self.dynamic {
            let slot = slots.add(scene, *mesh, r.xf).unwrap();
            let (kind, phase, speed, length) = r.motion.as_ref().unwrap();
            self.animated.push(Animated {
                mesh: *mesh,
                slot,
                xf: r.xf,
                kind: kind.clone(),
                phase: *phase,
                speed: *speed,
                length: *length,
            });
        }
        for &(center, size, phase) in &self.walker_data {
            let mut parts = Vec::new();
            for i in 0..20 {
                let mesh = self.walker_meshes[if i < 2 {
                    if i == 0 { 0 } else { 2 }
                } else if i < 14 {
                    1
                } else {
                    2
                }];
                parts.push((mesh, slots.add(scene, mesh, Affine3A::IDENTITY).unwrap()));
            }
            self.walkers.push(Walker {
                center,
                size,
                phase,
                parts,
            });
        }
    }
    pub fn flush(&self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        for animal in &self.octopuses {
            renderer.set_skinning_palette(gpu, animal.actor, &animal.palette)?;
        }
        Ok(())
    }
    pub fn update(&mut self, scene: &mut Scene, slots: &mut Slots, time: f32) {
        for animal in &mut self.octopuses {
            animal.update(time);
        }
        for a in &self.animated {
            let mut xf = a.xf;
            let t = time * a.speed + a.phase;
            if a.kind == "traffic" {
                xf.translation.x += (t).rem_euclid(a.length);
            } else if a.kind == "bubble" {
                xf.translation.y += t.rem_euclid(a.length);
                xf.translation.x += (t * 0.13).sin() * 2.;
                xf.translation.z += (t * 0.1).cos() * 2.;
            } else if a.kind == "swim" {
                xf.translation += glam::Vec3A::new(
                    t.cos() * a.length,
                    (t * 1.3).sin() * 4.,
                    t.sin() * a.length * 0.7,
                );
                let q = Quat::from_rotation_y(-t);
                xf.matrix3 = glam::Mat3A::from_quat(q) * xf.matrix3;
            } else {
                xf.translation.y += t.sin() * a.length;
                xf.translation.x += (t * 0.3).sin() * a.length;
            }
            slots.set(scene, a.slot, a.mesh, xf);
        }
        for w in &self.walkers {
            let s = w.size;
            let gait = time * 2.5 + w.phase;
            let base = w.center + Vec3::Z * (time * 0.18 + w.phase).sin() * 38.;
            let belly = base + Vec3::Y * (s * 0.67 + (gait * 2.).sin() * s * 0.035);
            let cargo = belly + Vec3::Y * s * 0.73;
            let poses = [
                Affine3A::from_scale_rotation_translation(
                    Vec3::new(s * 0.9, s * 0.7, s * 1.25),
                    Quat::from_rotation_z(gait.sin() * 0.055),
                    belly,
                ),
                Affine3A::from_scale_rotation_translation(
                    Vec3::new(s * 1.32, s * 1.45, s * 1.25),
                    Quat::IDENTITY,
                    cargo,
                ),
            ];
            for i in 0..2 {
                slots.set(scene, w.parts[i].1, w.parts[i].0, poses[i]);
            }
            for limb in 0..6 {
                let side = if limb % 2 == 0 { 1. } else { -1. };
                let row = (limb / 2) as f32;
                let cycle =
                    gait + if limb % 2 == 0 {
                        std::f32::consts::PI
                    } else {
                        0.
                    } + row * std::f32::consts::PI * 0.7;
                let hip = belly + Vec3::new(side * s * 0.22, -s * 0.06, (row - 1.) * s * 0.34);
                let foot = base
                    + Vec3::new(
                        side * s * 0.68,
                        cycle.sin().max(0.) * s * 0.22,
                        (row - 1.) * s * 0.54 + cycle.cos() * s * 0.25,
                    );
                let knee = (hip + foot) * 0.5 + Vec3::new(side * s * 0.26, s * 0.21, 0.);
                for (i, xf) in [
                    (2 + limb * 2, link(hip, knee, s * 0.095)),
                    (3 + limb * 2, link(knee, foot, s * 0.075)),
                    (
                        14 + limb,
                        Affine3A::from_scale_rotation_translation(
                            Vec3::new(s * 0.19, s * 0.11, s * 0.28),
                            Quat::IDENTITY,
                            foot,
                        ),
                    ),
                ] {
                    slots.set(scene, w.parts[i].1, w.parts[i].0, xf);
                }
            }
        }
    }
}
fn reflection_data(scene: &mut Scene, posters: &[Request], buildings: &[(Bounds, [f32; 3])]) {
    // Packed screen planes, facade boxes and a shared balanced spatial hierarchy.
    // GPU rays use near-first traversal, independent of receiver tile boundaries.
    let mut data = vec![
        0x53524143,
        posters.len() as u32,
        buildings.len() as u32,
        0,
        0,
        0,
        0,
        0,
    ];
    let sign_start = data.len();
    for p in posters {
        let x = Vec3::from(p.xf.matrix3.x_axis);
        let y = Vec3::from(p.xf.matrix3.y_axis);
        let n = x.cross(y).normalize();
        let tile = p.mesh[6..].parse::<usize>().unwrap();
        for f in [
            p.xf.translation.x,
            p.xf.translation.y,
            p.xf.translation.z,
            x.length(),
            x.normalize().x,
            x.normalize().y,
            x.normalize().z,
            y.length(),
            y.normalize().x,
            y.normalize().y,
            y.normalize().z,
            tile as f32,
            n.x,
            n.y,
            n.z,
            0.,
        ] {
            data.push(f.to_bits());
        }
    }
    let box_start = data.len();
    for (b, tint) in buildings {
        for f in [
            b.lo.x, b.lo.y, b.lo.z, 0., b.hi.x, b.hi.y, b.hi.z, 0., tint[0], tint[1], tint[2], 0.,
        ] {
            data.push(f.to_bits());
        }
    }
    // Trace actual world-space bounds instead of changing a top-six list at
    // every 128 m boundary (which sliced reflections in the middle of a road).
    let mut leaves: Vec<(Bounds, u32)> = posters
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let at = Vec3::from(p.xf.translation);
            let extent = (Vec3::from(p.xf.matrix3.x_axis).abs()
                + Vec3::from(p.xf.matrix3.y_axis).abs())
                * 0.5
                + Vec3::splat(0.03);
            (
                Bounds {
                    lo: at - extent,
                    hi: at + extent,
                },
                0x80000000 | i as u32,
            )
        })
        .chain(
            buildings
                .iter()
                .enumerate()
                .map(|(i, (b, _))| (*b, i as u32)),
        )
        .collect();
    let tree_start = data.len();
    reflection_bvh(&mut data, tree_start, &mut leaves);
    data[3] = sign_start as u32;
    data[4] = box_start as u32;
    data[6] = tree_start as u32;
    data[7] = ((data.len() - tree_start) / 8) as u32;
    scene.aux = data;
}

fn reflection_bvh(data: &mut Vec<u32>, start: usize, leaves: &mut [(Bounds, u32)]) {
    reflection_bvh_at(data, start, leaves, 0);
}

fn reflection_bvh_at(data: &mut Vec<u32>, start: usize, leaves: &mut [(Bounds, u32)], depth: usize) {
    if leaves.is_empty() {
        return;
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for (b, _) in leaves.iter() {
        lo = lo.min(b.lo);
        hi = hi.max(b.hi);
    }
    let node = data.len();
    data.extend_from_slice(&[
        lo.x.to_bits(),
        lo.y.to_bits(),
        lo.z.to_bits(),
        0,
        hi.x.to_bits(),
        hi.y.to_bits(),
        hi.z.to_bits(),
        u32::MAX,
    ]);
    if leaves.len() == 1 {
        data[node + 3] = ((data.len() - start) / 8) as u32;
        data[node + 7] = leaves[0].1;
        return;
    }
    let extent = hi - lo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    // Build once per generated level. Median splits put tall towers and small
    // billboards into heavily overlapping boxes; millions of reflection rays
    // then walk both branches. Minimize child surface area instead, retaining
    // every proxy and the exact same nearest-hit reflection.
    let mut best = (f32::INFINITY, axis, leaves.len() / 2);
    let area = |lo: Vec3, hi: Vec3| { let d = (hi-lo).max(Vec3::ZERO); d.x*d.y+d.x*d.z+d.y*d.z };
    if depth < 16 && leaves.len() > 2 {
        let mut suffix = vec![(Vec3::ZERO, Vec3::ZERO); leaves.len()];
        for axis in 0..3 {
            leaves.sort_unstable_by(|a,b| (a.0.lo[axis]+a.0.hi[axis]).total_cmp(&(b.0.lo[axis]+b.0.hi[axis])).then(a.1.cmp(&b.1)));
            let (mut l,mut h) = (Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY));
            for i in (0..leaves.len()).rev() { l=l.min(leaves[i].0.lo); h=h.max(leaves[i].0.hi); suffix[i]=(l,h); }
            let (mut l,mut h) = (Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY));
            for i in 1..leaves.len() {
                l=l.min(leaves[i-1].0.lo); h=h.max(leaves[i-1].0.hi);
                let cost=area(l,h)*i as f32+area(suffix[i].0,suffix[i].1)*(leaves.len()-i) as f32;
                if cost < best.0 { best=(cost,axis,i); }
            }
        }
    }
    leaves.sort_unstable_by(|a,b| (a.0.lo[best.1]+a.0.hi[best.1]).total_cmp(&(b.0.lo[best.1]+b.0.hi[best.1])).then(a.1.cmp(&b.1)));
    let mid = best.2;
    reflection_bvh_at(data, start, &mut leaves[..mid], depth+1);
    reflection_bvh_at(data, start, &mut leaves[mid..], depth+1);
    data[node + 3] = ((data.len() - start) / 8) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflection_hierarchy_bounds_depth_for_coincident_proxies() {
        let bounds=Bounds {lo:Vec3::ZERO,hi:Vec3::ONE};
        let mut leaves:Vec<_>=(0..10000).map(|id|(bounds,id)).collect();
        let mut data=Vec::new();
        reflection_bvh(&mut data,0,&mut leaves);
        let mut stack=vec![(0usize,1usize)];
        let mut count=0;
        while let Some((node,depth))=stack.pop() {
            assert!(depth<32,"builder must not degenerate into deep recursion");
            assert!(data[node*8+3] as usize>node && data[node*8+3] as usize<=data.len()/8);
            if data[node*8+7]==u32::MAX {stack.push((node+1,depth+1));stack.push((data[(node+1)*8+3] as usize,depth+1));}
            else {count+=1;}
        }
        assert_eq!(count,10000);
    }
    #[test]
    fn reflection_hierarchy_keeps_all_proxies_and_bounds() {
        let mut leaves: Vec<_> = (0..1500u32)
            .map(|i| {
                let p = Vec3::new(
                    (i % 31) as f32 * 113.,
                    (i % 7) as f32 * 44.,
                    (i / 31) as f32 * 127.,
                );
                (
                    Bounds {
                        lo: p - Vec3::splat(12.),
                        hi: p + Vec3::new(20., 150., 4.),
                    },
                    i,
                )
            })
            .collect();
        let original = leaves.clone();
        let mut data = vec![0; 8];
        reflection_bvh(&mut data, 8, &mut leaves);
        let read = |at: usize| {
            Vec3::new(
                f32::from_bits(data[at]),
                f32::from_bits(data[at + 1]),
                f32::from_bits(data[at + 2]),
            )
        };
        let mut stack = vec![(0usize, 1usize)];
        let mut found = std::collections::HashSet::new();
        while let Some((node, depth)) = stack.pop() {
            assert!(depth < 32);
            let at = 8 + node * 8;
            let lo = read(at);
            let hi = read(at + 4);
            let id = data[at + 7];
            if id == u32::MAX {
                for child in [node + 1, data[8 + (node+1)*8 + 3] as usize] {
                    assert!(child > node);
                    let c = 8 + child * 8;
                    assert!(read(c).cmpge(lo).all() && read(c + 4).cmple(hi).all());
                    stack.push((child, depth + 1));
                }
            } else {
                assert!(found.insert(id));
                assert_eq!(lo, original[id as usize].0.lo);
                assert_eq!(hi, original[id as usize].0.hi);
            }
        }
        assert_eq!(found.len(), original.len());
    }
    #[test]
    fn threaded_reflection_rays_match_exhaustive_bounds() {
        let original:Vec<_>=(0..900u32).map(|i| {
            let p=Vec3::new((i%30) as f32*30.,(i%7) as f32*2.,(i/30) as f32*30.);
            (Bounds {lo:p,hi:p+Vec3::new(14.,20.+(i%17) as f32*5.,18.)},i)
        }).collect();
        let mut leaves=original.clone();let mut data=Vec::new();
        reflection_bvh(&mut data,0,&mut leaves);
        let vec3=|at:usize|Vec3::new(f32::from_bits(data[at]),f32::from_bits(data[at+1]),f32::from_bits(data[at+2]));
        for i in 0..200 {
            let p=Vec3::new(-40.,17.+i as f32*0.47,(i*37%900) as f32);
            let ray=Vec3::new(1.,(i as f32*0.37).sin()*0.07,(i as f32*0.13).cos()*0.8).normalize();
            let inv=Vec3::ONE/ray;
            let distance=|lo:Vec3,hi:Vec3| {let a=(lo-p)*inv;let b=(hi-p)*inv;let near=a.min(b).max_element().max(0.);let far=a.max(b).min_element();if far>=near && near>0. {near}else{f32::INFINITY}};
            let reference=original.iter().map(|(b,id)|(distance(b.lo,b.hi),*id)).filter(|(d,_)|*d<1600.).min_by(|a,b|a.0.total_cmp(&b.0));
            let mut nearest=1600.;let mut hit=None;let mut node=0usize;let mut visits=0;
            while node<data.len()/8 {
                visits+=1;assert!(visits<=data.len()/8);
                let current=node;let at=current*8;node=data[at+3] as usize;
                let enter=distance(vec3(at),vec3(at+4));
                if enter>=nearest {continue;}
                let payload=data[at+7];
                if payload==u32::MAX {node=current+1;}else {nearest=enter;hit=Some((enter,payload));}
            }
            assert_eq!(hit,reference,"ray {i}");
        }
    }
    #[test]
    fn reef_plants_reject_cliffs_and_overhangs_but_keep_clear_ground() {
        let rules = PlantClearance {
            root_band: 0.035,
            max_slope_degrees: 45.,
            clearance: 0.25,
            road_margin: 8.,
        };
        let mut terrain = Landscape {
            heights: vec![0.; 81 * 81],
            cells: 80,
            step: 2.,
            half: 80.,
            outside: 0.,
        };
        let upright = Affine3A::from_scale_rotation_translation(
            Vec3::splat(50.),
            Quat::IDENTITY,
            Vec3::new(0., -0.5, 0.),
        );
        for name in ["kelp", "lantern", "fan", "sponge"] {
            assert!(
                PlantShape::new(name).clears_ground(&terrain, upright, rules),
                "{name} on flat ground"
            );
        }
        let lantern = PlantShape::new("lantern");
        // Root remains on flat ground, but the bent head reaches into a wall.
        for z in 0..=80 {
            for x in 0..=80 {
                if x as f32 * 2. - 80. >= 16. {
                    terrain.heights[z * 81 + x] = 100.;
                }
            }
        }
        assert!(!lantern.clears_ground(&terrain, upright, rules));
        let away = Affine3A::from_scale_rotation_translation(
            Vec3::splat(50.),
            Quat::from_rotation_y(std::f32::consts::PI),
            Vec3::new(0., -0.5, 0.),
        );
        assert!(lantern.clears_ground(&terrain, away, rules));
        for z in 0..=80 {
            for x in 0..=80 {
                terrain.heights[z * 81 + x] = (x as f32 * 2. - 80.) * 2.;
            }
        }
        assert!(!PlantShape::new("kelp").clears_ground(&terrain, upright, rules));
    }
    #[test]
    fn source_stages_keep_distinct_components_and_road_clearance() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
        for biome in ["city", "forest", "cell", "desert", "ocean"] {
            let race = Race::new(31, "normal", biome, 1, 3);
            let mut scene = Scene::default();
            let stage = Stage::build(&mut scene, &race, &root).unwrap();
            let count = |name: &str| *stage.counts.get(name).unwrap_or(&0);
            match biome {
                "city" => {
                    assert!(count("box") > 10000);
                    assert!(stage.dynamic.len() > 300);
                    assert!(count("poster0") > 20);
                    assert_eq!(scene.aux[0], 0x53524143);
                }
                "forest" => {
                    assert!(count("tree0") + count("tree1") > 400);
                    assert!(count("fern") > 500);
                }
                "cell" => {
                    assert!(count("dna") > 5 && count("protein") > 15 && count("cristae") > 10);
                    assert!(stage.walker_data.len() > 10);
                    assert_eq!(count("membrane"), 1);
                }
                "desert" => assert!(count("cactus") > 200 && count("rock0") + count("rock1") > 100),
                "ocean" => {
                    assert!(count("kelp") > 500 && count("jelly") == 112 && count("manta") == 6);
                    assert!(count("octopus") >= 3);
                    scene
                        .skinning
                        .validate(&scene.vertices, &scene.instances)
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(stage.landscape.heights.iter().all(|h| h.is_finite()));
            for n in &race.world.track.nodes {
                assert!(
                    stage.landscape.height(n.geometry.p.x, n.geometry.p.z) < n.geometry.p.y - 3.,
                    "{biome} terrain intrudes into track"
                );
                assert!(!stage.obstacles.iter().any(|b| n.geometry.p.cmpge(b.min).all()&&n.geometry.p.cmple(b.max).all()),"{biome} obstacle on track");
            }
        }
    }
}
