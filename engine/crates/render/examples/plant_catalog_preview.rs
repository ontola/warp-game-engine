//! Render a grown species' source triangles at stand and shoot scales without a GPU.
//! Run `cargo run --release -p render --example plant_catalog_preview -- --help`.
use glam::{Mat3, Mat4, Vec2, Vec3};
use render::{
    foliage_assembly::{Assembly, PartInstance},
    plant_growth,
    plant_growth_assembly::{self as growth_assembly, Parts},
    plant_species, scene,
};
use std::{env, fs, path::Path};

const HELP: &str = "plant_catalog_preview [--out DIR] [--age YEARS] [--seed N] SPECIES...\n  --list         list species that use the growth model\n  --out DIR      default: build/plant-catalog\n  --age, --seed  default: species' stand age and seed\nWrites transparent habit/shoot PNGs and a JSON scale record for each species.";
const SIDE: usize = 720;
const SUPERSAMPLE: usize = 2;

fn view(toward_eye: Vec3) -> Mat3 {
    let toward = toward_eye.normalize();
    let right = Vec3::Y.cross(toward).normalize();
    Mat3::from_cols(right, toward.cross(right), toward).transpose()
}

struct Frame {
    view: Mat3,
    centre: Vec2,
    width: f32,
}

impl Frame {
    fn fit(geometry: &Assembly, indices: &[usize], world: &[Mat4], view: Mat3) -> Self {
        let mut lo = Vec2::splat(f32::INFINITY);
        let mut hi = -lo;
        for &index in indices {
            let instance = &geometry.instances[index];
            let transform = world[index];
            for vertex in &geometry.parts[instance.part as usize].vertices {
                let p = (view * transform.transform_point3(vertex.pos.into())).truncate();
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        Self {
            view,
            centre: (hi + lo) * 0.5,
            width: (hi - lo).max_element() * 1.12,
        }
    }

    fn project(&self, p: Vec3, side: usize) -> Vec3 {
        let projected = self.view * p;
        let xy = (projected.truncate() - self.centre) / self.width;
        ((xy * Vec2::new(1.0, -1.0) + 0.5) * side as f32).extend(projected.z)
    }
}

// Show a real attached shoot and its supporting wood. For a leafless specimen,
// show its longest woody axis. Keep both world matrices (including scale).
fn shoot_detail(assembly: &Assembly, parts: &Parts, world: &[Mat4]) -> (Vec<usize>, Mat3, bool) {
    let mut stems = vec![Vec::new(); assembly.branches.len()];
    for (i, instance) in assembly.instances.iter().enumerate() {
        let part = assembly.parts[instance.part as usize].id;
        if parts.stems.iter().any(|s| s.id == part) {
            stems[instance.branch as usize].push(i);
        }
    }
    let mut best = None;
    for (i, instance) in assembly.instances.iter().enumerate() {
        if assembly.materials[instance.material as usize].flags & scene::THIN_FOLIAGE == 0 {
            continue;
        }
        let origin = world[i].transform_point3(Vec3::ZERO);
        let axis = world[i].transform_vector3(Vec3::Y);
        for &j in &stems[instance.branch as usize] {
            let stem_axis = world[j].transform_vector3(Vec3::Y);
            let gap = origin.distance(world[j].transform_point3(Vec3::Y));
            let alignment = axis.normalize().dot(stem_axis.normalize());
            // A full-size shoot that continues the end of a stem, with ties
            // broken by assembly order. Gap is relative to the support length.
            let score = gap / stem_axis.length() + (1.0 - alignment) + 1.0 / axis.length();
            // A crown leader can carry a shoot right on top of a thick trunk.
            // Prefer a side branch for the specimen, if the plant has one.
            let leader = assembly.branches[instance.branch as usize].parent.is_none();
            let rank = (leader, score);
            if best.is_none_or(|(previous, _, _)| rank < previous) {
                best = Some((rank, i, j));
            }
        }
    }
    let (indices, is_leaf) = if let Some((_, shoot, support)) = best {
        (vec![shoot, support], true)
    } else {
        let wood = stems
            .iter()
            .flatten()
            .copied()
            .max_by(|&a, &b| {
                world[a]
                    .y_axis
                    .length_squared()
                    .total_cmp(&world[b].y_axis.length_squared())
            })
            .unwrap_or(0);
        (vec![wood], false)
    };
    let (_, rotation, _) = world[indices[0]].to_scale_rotation_translation();
    // Choose the view that exposes the greatest projected surface area.
    // A palmate leaf opens across the shoot while a palm frond opens along it.
    let part = &assembly.parts[assembly.instances[indices[0]].part as usize];
    let face_vectors = part
        .indices
        .chunks_exact(3)
        .map(|triangle| {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                .map(|i| Vec3::from(part.vertices[i as usize].pos));
            (b - a).cross(c - a)
        })
        .collect::<Vec<_>>();
    let directions = [
        Vec3::new(0.31, 0.22, 1.0),
        Vec3::new(1.0, 0.22, 0.31),
        Vec3::new(0.26, 1.0, 0.48),
        Vec3::new(0.88, 0.67, 0.34),
    ];
    let area = |dir: Vec3| face_vectors.iter().map(|n| n.dot(dir).abs()).sum::<f32>();
    let direction = directions
        .map(Vec3::normalize)
        .into_iter()
        .max_by(|a, b| area(*a).total_cmp(&area(*b)))
        .unwrap();
    let axes = view(direction) * Mat3::from_quat(rotation.inverse());
    (indices, axes, is_leaf)
}

#[derive(Clone, Copy)]
struct Vertex {
    screen: Vec3,
    world: Vec3,
    normal: Vec3,
}

struct Shadow {
    frame: Frame,
    depth: Vec<f32>,
}

impl Shadow {
    fn visibility(&self, point: Vec3, side: usize) -> f32 {
        let q = self.frame.project(point, side);
        let mut lit = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let x = (q.x.floor() as i32 + dx).clamp(0, side as i32 - 1) as usize;
                let y = (q.y.floor() as i32 + dy).clamp(0, side as i32 - 1) as usize;
                if self.depth[y * side + x] <= q.z + self.frame.width * 1.6 / side as f32 {
                    lit += 1.0;
                }
            }
        }
        lit / 9.0
    }
}

struct Specimen<'a> {
    geometry: &'a Assembly,
    indices: &'a [usize],
    world: &'a [Mat4],
}

impl Specimen<'_> {
    fn rasterize(
        &self,
        frame: &Frame,
        side: usize,
        mut shade: impl FnMut(usize, &PartInstance, [Vertex; 3], Vec3),
    ) -> Vec<f32> {
        let mut depth = vec![f32::NEG_INFINITY; side * side];
        for &index in self.indices {
            let instance = &self.geometry.instances[index];
            let part = &self.geometry.parts[instance.part as usize];
            let transform = self.world[index];
            let normals = Mat3::from_mat4(transform).inverse().transpose();
            let vertices: Vec<_> = part
                .vertices
                .iter()
                .map(|v| {
                    let world = transform.transform_point3(v.pos.into());
                    Vertex {
                        screen: frame.project(world, side),
                        world,
                        normal: (normals * Vec3::from(v.normal)).normalize_or_zero(),
                    }
                })
                .collect();
            for triangle in part.indices.chunks_exact(3) {
                let v = [
                    vertices[triangle[0] as usize],
                    vertices[triangle[1] as usize],
                    vertices[triangle[2] as usize],
                ];
                let [a, b, c] = v.map(|v| v.screen.truncate());
                let area = (b - a).perp_dot(c - a);
                if area.abs() < f32::EPSILON {
                    continue;
                }
                let lo = a
                    .min(b)
                    .min(c)
                    .floor()
                    .clamp(Vec2::ZERO, Vec2::splat(side as f32))
                    .as_uvec2();
                let hi = a
                    .max(b)
                    .max(c)
                    .ceil()
                    .clamp(Vec2::ZERO, Vec2::splat(side as f32))
                    .as_uvec2();
                for y in lo.y..hi.y {
                    for x in lo.x..hi.x {
                        let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let wa = (c - b).perp_dot(point - b) / area;
                        let wb = (a - c).perp_dot(point - c) / area;
                        let weights = Vec3::new(wa, wb, 1.0 - wa - wb);
                        if weights.min_element() < 0.0 {
                            continue;
                        }
                        let z = weights.dot(Vec3::from(v.map(|v| v.screen.z)));
                        let index = y as usize * side + x as usize;
                        if z > depth[index] {
                            depth[index] = z;
                            shade(index, instance, v, weights);
                        }
                    }
                }
            }
        }
        depth
    }

    fn save(&self, frame: &Frame, path: &Path) -> Result<(), String> {
        let toward_eye = frame.view.transpose() * Vec3::Z;
        let sun = frame.view.transpose() * Vec3::new(-0.46, 0.72, 0.48).normalize();
        let light = Frame::fit(self.geometry, self.indices, self.world, view(sun));
        let shadow = Shadow {
            depth: self.rasterize(&light, SIDE, |_, _, _, _| {}),
            frame: light,
        };
        let side = SIDE * SUPERSAMPLE;
        let mut samples = vec![[0u8; 4]; side * side];
        self.rasterize(frame, side, |i, instance, v, weights| {
            let mat = &self.geometry.materials[instance.material as usize];
            let leaf = mat.flags & scene::THIN_FOLIAGE != 0;
            let interpolate = |values: [Vec3; 3]| {
                values[0] * weights.x + values[1] * weights.y + values[2] * weights.z
            };
            let mut n = interpolate(v.map(|v| v.normal)).normalize_or_zero();
            if leaf && n.dot(toward_eye) < 0.0 {
                n = -n;
            }
            let cosine = n.dot(sun);
            let lit = shadow.visibility(interpolate(v.map(|v| v.world)), SIDE);
            let direct = cosine.max(0.0) + if leaf { 0.18 * (-cosine).max(0.0) } else { 0.0 };
            let light = 0.41
                + 0.11 * n.dot(frame.view.transpose() * Vec3::Y).max(0.0)
                + direct * lit * 0.91;
            let rgb = mat.tint.map(|c| {
                let linear = (c * light).clamp(0.0, 1.0);
                let srgb = if linear <= 0.0031308 {
                    12.92 * linear
                } else {
                    1.055 * linear.powf(1.0 / 2.4) - 0.055
                };
                (255.0 * srgb).round() as u8
            });
            samples[i] = [rgb[0], rgb[1], rgb[2], 255];
        });
        let mut pixels = Vec::with_capacity(SIDE * SIDE * 4);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let mut rgba = [0u32; 4];
                for dy in 0..SUPERSAMPLE {
                    for dx in 0..SUPERSAMPLE {
                        let p = samples[(y * SUPERSAMPLE + dy) * side + x * SUPERSAMPLE + dx];
                        for (sum, value) in rgba.iter_mut().zip(p) {
                            *sum += value as u32;
                        }
                    }
                }
                let covered = (rgba[3] / 255).max(1);
                pixels.extend(rgba[..3].iter().map(|x| (x / covered) as u8));
                pixels.push((rgba[3] / (SUPERSAMPLE * SUPERSAMPLE) as u32) as u8);
            }
        }
        render::write_png(path, SIDE as u32, SIDE as u32, &pixels)
    }
}

fn export(name: &str, age: Option<u32>, seed: Option<u64>, directory: &Path) -> Result<(), String> {
    let recipe = plant_species::grown(name)
        .ok_or_else(|| format!("unknown grown species {name:?}, see --list"))?;
    let age = age.unwrap_or(recipe.age);
    let seed = seed.unwrap_or(recipe.seed);
    let parts = recipe.parts();
    let plant = plant_growth::grow(&recipe.rules, age, seed);
    let (geometry, counts) =
        growth_assembly::assemble(&plant, &parts, recipe.look, seed, growth_assembly::limits())?;
    let bones = geometry.branch_transforms()?;
    let world: Vec<_> = geometry
        .instances
        .iter()
        .map(|i| bones[i.branch as usize] * i.rest)
        .collect();
    let all: Vec<_> = (0..world.len()).collect();
    if all.is_empty() {
        return Err(format!("{name} produced no geometry"));
    }
    let habit = Frame::fit(&geometry, &all, &world, view(Vec3::new(0.86, 0.12, 0.49)));
    let (detail, axes, leaf) = shoot_detail(&geometry, &parts, &world);
    let shoot = Frame::fit(&geometry, &detail[..1], &world, axes);
    let path = |suffix| directory.join(format!("{name}{suffix}"));
    Specimen {
        geometry: &geometry,
        indices: &all,
        world: &world,
    }
    .save(&habit, &path("-habit.png"))?;
    Specimen {
        geometry: &geometry,
        indices: &detail,
        world: &world,
    }
    .save(&shoot, &path("-shoot.png"))?;
    let [lo, hi] = geometry.bounds()?;
    let size = Vec3::from(hi) - Vec3::from(lo);
    let record = serde_json::json!({
        "species": name, "age_years": age, "seed": seed.to_string(),
        "height_m": size.y, "span_m": size.x.max(size.z), "instances": all.len(),
        "placed_tufts": counts.tufts, "dropped_tufts": counts.tufts_dropped,
        "habit_frame_m": habit.width, "shoot_frame_m": shoot.width,
        "detail": if leaf { "shoot_and_support" } else { "woody_axis" },
    });
    fs::write(path(".json"), record.to_string()).map_err(|e| e.to_string())?;
    println!(
        "{name}: {} years, {:.2} m high, {:.2} m wide",
        age,
        size.y,
        size.x.max(size.z)
    );
    Ok(())
}

fn main() -> Result<(), String> {
    let mut directory = std::path::PathBuf::from("build/plant-catalog");
    let mut age = None;
    let mut seed = None;
    let mut names = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            "--list" => {
                for name in plant_species::GROWN_NAMES {
                    println!("{name}");
                }
                return Ok(());
            }
            "--out" => directory = args.next().ok_or("--out requires a directory")?.into(),
            "--age" => {
                let years = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|&v: &u32| v > 0);
                age = Some(years.ok_or("--age requires a positive year count")?);
            }
            "--seed" => {
                seed = Some(
                    args.next()
                        .and_then(|s| s.parse().ok())
                        .ok_or("--seed requires an unsigned integer")?,
                )
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}\n{HELP}")),
            _ => names.push(arg),
        }
    }
    if names.is_empty() {
        return Err(HELP.into());
    }
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    for name in names {
        export(&name, age, seed, &directory)?;
    }
    Ok(())
}
