//! Bake a species once for inexpensive static forest stands. Both budgets sample
//! the same growth, using a continuous wood sweep and volumes fitted to foliage.
use glam::Vec3;
use render::{plant_growth, plant_recipe, plant_species};
use serde::Serialize;
use std::{collections::BTreeMap, env, fs, path::Path};

#[derive(Clone, Copy)]
struct Joint {
    pos: Vec3,
    radius: f32,
}
#[derive(Clone, Copy)]
struct Leaf {
    pos: Vec3,
    radius: f32,
    axis: Vec3,
    limb: u32,
}
struct Source {
    label: &'static str,
    model: &'static str,
    stems: Vec<Vec<Joint>>,
    leaves: Vec<Leaf>,
}

fn grown(kind: &str, age: u32, seed: u64) -> Source {
    use plant_growth::Species;
    use render::plant_growth_assembly::Look;
    let (rules, label, reach) = if kind == "pine" {
        (
            Species::jeffrey_pine(),
            "Pinus jeffreyi",
            Look::jeffrey_pine().needle_length,
        )
    } else {
        (
            Species::norway_spruce(),
            "Picea abies",
            Look::norway_spruce().needle_length,
        )
    };
    let p = plant_growth::grow(&rules, age, seed);
    // Follow annual shoots in parent order; a new first-order axis starts a limb.
    let mut stems: Vec<Vec<Joint>> = Vec::new();
    let mut axes = vec![0; p.segments.len()];
    for (i, s) in p.segments.iter().enumerate() {
        if s.order > 1 {
            axes[i] = axes[s.parent.expect("lateral shoot has a parent") as usize];
            continue;
        }
        let axis = if let Some(parent) = s
            .parent
            .filter(|j| p.segments[*j as usize].order == s.order)
        {
            axes[parent as usize]
        } else {
            stems.push(vec![Joint {
                pos: s.from,
                radius: s.radius[0],
            }]);
            stems.len() - 1
        };
        axes[i] = axis;
        stems[axis].push(Joint {
            pos: s.to,
            radius: s.radius[1],
        });
    }
    let leaves = p
        .tufts
        .iter()
        .map(|t| Leaf {
            pos: t.at + t.axis * (reach * 0.45),
            axis: t.axis,
            limb: axes[t.segment as usize] as u32,
            radius: reach * (0.3 + 0.7 * t.fullness),
        })
        .collect();
    Source {
        label,
        model: "plant_growth",
        stems,
        leaves,
    }
}

fn recipe(kind: &str, age: u32, seed: u64) -> Result<Source, String> {
    let name = plant_species::NAMES
        .into_iter()
        .find(|name| *name == kind)
        .ok_or_else(|| format!("unknown Warp Vegetation species '{kind}'"))?;
    let recipe = plant_species::named(name).ok_or("missing woody recipe")?;
    let assembly = plant_recipe::build(
        &recipe,
        plant_recipe::BuildOptions {
            seed,
            growth: (age as f32 / 20.).clamp(0.25, 1.5),
            season: 0.,
        },
        plant_recipe::recipe_limits(),
    )?;
    let matrices = assembly.branch_transforms()?;
    // Sample the actual six directional extremities of each leaf spray.
    // Their template vertices are shared by many instances.
    let extrema: Vec<_> = assembly
        .parts
        .iter()
        .map(|part| {
            [
                Vec3::X,
                Vec3::NEG_X,
                Vec3::Y,
                Vec3::NEG_Y,
                Vec3::Z,
                Vec3::NEG_Z,
            ]
            .map(|d| {
                part.vertices
                    .iter()
                    .map(|v| Vec3::from_array(v.pos))
                    .max_by(|a, b| a.dot(d).total_cmp(&b.dot(d)))
                    .unwrap()
            })
        })
        .collect();
    let mut stems = BTreeMap::<u32, Vec<Joint>>::new();
    let mut leaves = Vec::new();
    for inst in &assembly.instances {
        let m = matrices[inst.branch as usize] * inst.rest;
        let mut group = inst.branch;
        while let Some(parent) = assembly.branches[group as usize].parent {
            if parent == 0 {
                break;
            }
            group = parent;
        }
        if assembly.materials[inst.material as usize].flags & render::scene::THIN_FOLIAGE != 0 {
            for p in extrema[inst.part as usize] {
                leaves.push(Leaf {
                    pos: m.transform_point3(p),
                    radius: 0.025,
                    axis: m.y_axis.truncate().normalize_or(Vec3::Y),
                    limb: group,
                });
            }
        } else if inst.part == 0 || inst.part as usize == assembly.parts.len() - 1 {
            // The recipe's structural stem templates are unit +Y tapers.
            let stem = stems.entry(group).or_default();
            let radius = m.x_axis.truncate().length();
            let base = m.transform_point3(Vec3::ZERO);
            if stem.is_empty() {
                stem.push(Joint {
                    pos: base.max(Vec3::new(f32::NEG_INFINITY, 0., f32::NEG_INFINITY)),
                    radius,
                });
            } else {
                // Overlapping primitive segments share one ring on the surface sweep.
                *stem.last_mut().unwrap() = Joint { pos: base, radius };
            }
            stem.push(Joint {
                pos: m.transform_point3(Vec3::Y),
                radius: radius * 0.7,
            });
        }
    }
    Ok(Source {
        label: match name {
            "birch" => "Birch",
            "broadleaf" => "Broadleaf",
            other => other,
        },
        model: "plant_recipe",
        stems: stems.into_values().collect(),
        leaves,
    })
}

impl Source {
    fn fit(&mut self, height: f32, root: f32) -> Result<(), String> {
        let top = self
            .leaves
            .iter()
            .map(|p| p.pos.y + p.radius)
            .chain(self.stems.iter().flatten().map(|n| n.pos.y))
            .fold(0., f32::max);
        let trunk = self
            .stems
            .first()
            .and_then(|p| p.first())
            .ok_or("empty trunk")?;
        if top <= 0. || trunk.radius <= 0. || self.leaves.is_empty() {
            return Err("empty source".into());
        }
        let size = height / top;
        let radial = root / trunk.radius;
        for j in self.stems.iter_mut().flatten() {
            j.pos *= size;
            j.radius *= radial;
        }
        for l in &mut self.leaves {
            l.pos *= size;
            l.radius *= size;
        }
        // Keep the leader first. Spend remaining bark triangles on the limbs
        // with the most projected wood area, independent of species and seed.
        self.stems[1..].sort_by(|a, b| {
            let score = |s: &[Joint]| {
                s.windows(2)
                    .map(|w| w[0].pos.distance(w[1].pos) * w[0].radius)
                    .sum::<f32>()
            };
            score(b).total_cmp(&score(a))
        });
        Ok(())
    }
}

#[derive(Default, Serialize)]
struct Mesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tones: Vec<u8>,
}

fn sweep(mesh: &mut Mesh, stem: &[Joint], spans: usize, sides: usize) {
    let base = mesh.positions.len() as u32;
    let rings: Vec<_> = (0..=spans)
        .map(|i| stem[i * (stem.len() - 1) / spans])
        .collect();
    let axis = (rings.last().unwrap().pos - rings[0].pos).normalize_or(Vec3::Y);
    let guide = if axis.x.abs() < 0.85 {
        Vec3::X
    } else {
        Vec3::Z
    };
    for (i, j) in rings.iter().enumerate() {
        let axis =
            (rings[(i + 1).min(spans)].pos - rings[i.saturating_sub(1)].pos).normalize_or(axis);
        let u = axis.cross(guide).normalize_or(Vec3::Z);
        let v = axis.cross(u);
        for n in 0..sides {
            let (s, c) = (std::f32::consts::TAU * n as f32 / sides as f32).sin_cos();
            let normal = u * c + v * s;
            let mut p = j.pos + normal * j.radius;
            if i == 0 && j.pos.y.abs() < 1e-5 {
                p.y = 0.;
            }
            mesh.positions.push(p.to_array());
            mesh.normals.push(normal.to_array());
            if i > 0 {
                let a = base + ((i - 1) * sides + n) as u32;
                let b = base + ((i - 1) * sides + (n + 1) % sides) as u32;
                mesh.indices.extend([
                    a,
                    b,
                    a + sides as u32,
                    b,
                    b + sides as u32,
                    a + sides as u32,
                ]);
            }
        }
    }
    // Only the visible tip needs a cap; roots enter either soil or parent wood.
    let tip = base + (spans * sides) as u32;
    for i in 1..sides - 1 {
        mesh.indices
            .extend([tip, tip + i as u32, tip + i as u32 + 1]);
    }
}

fn wood(s: &Source, total: usize) -> Mesh {
    let mut m = Mesh::default();
    let allowed = total * 27 / 100;
    let sides = if total >= 400 { 6 } else { 4 };
    let leader = &s.stems[0];
    let spans = (if total >= 400 { 5 } else { 2 }).min(leader.len() - 1);
    sweep(&mut m, leader, spans, sides);
    for limb in &s.stems[1..] {
        let sides = if total >= 400 { 4 } else { 3 };
        let spans = (if total >= 600 { 2 } else { 1 }).min(limb.len() - 1);
        if m.indices.len() / 3 + spans * sides * 2 + sides - 2 > allowed {
            break;
        }
        sweep(&mut m, limb, spans, sides);
    }
    m
}

fn mean(s: &[Leaf]) -> Vec3 {
    s.iter().map(|l| l.pos).sum::<Vec3>() / s.len() as f32
}

fn split(s: Vec<Leaf>) -> (Vec<Leaf>, Vec<Leaf>) {
    let center = mean(&s);
    let far = |origin: Vec3| {
        s.iter()
            .map(|l| l.pos)
            .max_by(|a, b| {
                a.distance_squared(origin)
                    .total_cmp(&b.distance_squared(origin))
            })
            .unwrap()
    };
    let mut a = far(center);
    let mut b = far(a);
    // Cluster the occupied source rather than slicing it at world-aligned cell edges.
    for _ in 0..5 {
        let (mut left, mut right) = (Vec3::ZERO, Vec3::ZERO);
        let (mut n, mut k) = (0, 0);
        for l in &s {
            if l.pos.distance_squared(a) <= l.pos.distance_squared(b) {
                left += l.pos;
                n += 1;
            } else {
                right += l.pos;
                k += 1;
            }
        }
        if n == 0 || k == 0 {
            break;
        }
        a = left / n as f32;
        b = right / k as f32;
    }
    let (mut l, mut r): (Vec<_>, Vec<_>) = s
        .into_iter()
        .partition(|p| p.pos.distance_squared(a) <= p.pos.distance_squared(b));
    if r.is_empty() {
        r = l.split_off(l.len() / 2);
    }
    (l, r)
}

fn error(s: &[Leaf]) -> f32 {
    let center = mean(s);
    s.iter()
        .map(|l| l.pos.distance_squared(center))
        .sum::<f32>()
}

fn crown(mesh: &mut Mesh, leaves: &[Leaf], sides: usize) {
    let center = mean(leaves);
    let (min, max) = leaves.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
        (lo.min(p.pos.y - p.radius), hi.max(p.pos.y + p.radius))
    });
    let h = (max - min).max(0.001);
    let tip = leaves
        .iter()
        .max_by(|a, b| a.pos.y.total_cmp(&b.pos.y))
        .unwrap()
        .pos;
    let bottom = leaves
        .iter()
        .min_by(|a, b| a.pos.y.total_cmp(&b.pos.y))
        .unwrap()
        .pos;
    let heading = center.z.atan2(center.x);
    let mut vertices = vec![
        Vec3::new(tip.x, max, tip.z),
        Vec3::new(bottom.x, min, bottom.z),
    ];
    for i in 0..sides {
        let angle = std::f32::consts::TAU * i as f32 / sides as f32 + heading;
        let (sn, cs) = angle.sin_cos();
        let d = Vec3::new(cs, 0., sn);
        let t = Vec3::new(-sn, 0., cs);
        let support = |l: &Leaf| {
            let off = l.pos - center;
            off.dot(d) + l.radius - 0.65 * off.dot(t).abs()
        };
        let l = leaves
            .iter()
            .max_by(|a, b| support(a).total_cmp(&support(b)))
            .unwrap();
        let off = l.pos - center;
        let horizontal = (off.dot(d) + l.radius).max(l.radius);
        let lateral = off.dot(t).clamp(-0.34 * horizontal, 0.34 * horizontal);
        let mut p = center + d * horizontal + t * lateral;
        p.y = l.pos.y.clamp(min + 0.12 * h, min + 0.88 * h);
        vertices.push(p);
    }
    let mut faces = Vec::new();
    for i in 0..sides {
        let j = (i + 1) % sides;
        let (lo, next) = (2 + i, 2 + j);
        faces.push([lo, next, 1]);
        faces.push([0, next, lo]);
    }
    add_crown_mesh(mesh, center, vertices, faces, false);
}

/// The richer tier represents settled growth along a limb as pointed branchlet sprays.
/// A cloud's longest axis follows the actual shoot direction at its tip, with a
/// shorter, asymmetric cross-section where the occupied foliage spreads out.
fn branchlet(mesh: &mut Mesh, leaves: &[Leaf]) {
    let center = mean(leaves);
    let growth = leaves
        .iter()
        .map(|p| p.axis)
        .sum::<Vec3>()
        .normalize_or(Vec3::Y);
    let mut axis = leaves
        .iter()
        .map(|p| p.pos - center)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .unwrap()
        .normalize_or(growth);
    // Power iteration on the occupied samples' covariance gives the direction
    // of their actual spread, including the droop of the supporting limb.
    for _ in 0..4 {
        axis = leaves
            .iter()
            .map(|p| {
                let v = p.pos - center;
                v * v.dot(axis)
            })
            .sum::<Vec3>()
            .normalize_or(axis);
    }
    if axis.dot(growth) < 0. {
        axis = -axis;
    }
    axis = (axis * 0.8 + growth * 0.2).normalize_or(growth);
    let across = Vec3::Y.cross(axis).normalize_or(Vec3::X);
    let up = axis.cross(across).normalize_or(Vec3::Y);
    let support = |d: Vec3| {
        leaves
            .iter()
            .max_by(|a, b| (a.pos.dot(d) + a.radius).total_cmp(&(b.pos.dot(d) + b.radius)))
            .unwrap()
    };
    let tip = support(axis);
    let base = support(-axis);
    let mut vertices = vec![tip.pos + axis * tip.radius, base.pos - axis * base.radius];
    for dir in [across, up, -across, -up] {
        let side = axis.cross(dir);
        let score = |p: &Leaf| {
            let v = p.pos - center;
            v.dot(dir) + p.radius - 0.28 * v.dot(axis).abs() - 0.34 * v.dot(side).abs()
        };
        let sample = leaves
            .iter()
            .max_by(|a, b| score(a).total_cmp(&score(b)))
            .unwrap();
        let v = sample.pos - center;
        let width = (v.dot(dir) + sample.radius).max(sample.radius);
        let along = axis.dot(v) * 0.55;
        let lateral = v.dot(side).clamp(-0.28 * width, 0.28 * width);
        vertices.push(center + dir * width + axis * along + side * lateral);
    }
    // The ring is counterclockwise around the shoot axis, viewed from its tip.
    let mut faces = Vec::with_capacity(8);
    for n in 0..4 {
        let a = n + 2;
        let b = (n + 1) % 4 + 2;
        faces.extend([[0, a, b], [1, b, a]]);
    }
    add_crown_mesh(mesh, center, vertices, faces, true);
}

fn add_crown_mesh(
    mesh: &mut Mesh,
    center: Vec3,
    vertices: Vec<Vec3>,
    faces: Vec<[usize; 3]>,
    detailed: bool,
) {
    let top = mesh.positions.len() as u32;
    let mut normals = vec![Vec3::ZERO; vertices.len()];
    let heading = center.z.atan2(center.x);
    let hue = ((heading * 3.1 + center.y * 0.83).sin() * 0.5 + 0.5) * 0.85;
    let shade = if hue < 0.17 {
        0
    } else if hue > 0.72 {
        2
    } else {
        1
    };
    for [a, b, c] in faces {
        let n = (vertices[b] - vertices[a]).cross(vertices[c] - vertices[a]);
        for k in [a, b, c] {
            normals[k] += n;
        }
        mesh.indices
            .extend([top + a as u32, top + b as u32, top + c as u32]);
        let tone = if detailed && n.normalize_or(Vec3::Y).y < -0.65 && shade == 1 {
            0
        } else {
            shade
        };
        mesh.tones.push(tone);
    }
    for (p, n) in vertices.into_iter().zip(normals) {
        mesh.positions.push(p.to_array());
        mesh.normals.push(
            n.normalize_or((p - center).normalize_or(Vec3::Y))
                .to_array(),
        );
    }
}

#[derive(Serialize)]
struct Tier {
    wood: Mesh,
    foliage: Mesh,
}
/// Keep foliage on its parent limb. When a specimen has more limbs than the
/// budget supports, merge small neighbouring ones before the spatial splits.
fn limb_groups(leaves: &[Leaf], target: usize) -> Vec<Vec<Leaf>> {
    let mut limbs = BTreeMap::<u32, Vec<Leaf>>::new();
    for &p in leaves {
        limbs.entry(p.limb).or_default().push(p);
    }
    let mut groups: Vec<_> = limbs.into_values().map(|p| (mean(&p), p)).collect();
    while groups.len() > target {
        let small = groups
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| p.1.len())
            .unwrap()
            .0;
        let (c, mut leaves) = groups.swap_remove(small);
        let near = groups
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.0.distance_squared(c).total_cmp(&b.0.distance_squared(c)))
            .unwrap()
            .0;
        let g = &mut groups[near];
        g.1.append(&mut leaves);
        g.0 = mean(&g.1);
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

fn tier(s: &Source, budget: usize) -> Tier {
    let wood = wood(s, budget);
    let mut foliage = Mesh::default();
    let detailed = budget >= 400;
    let groups = ((budget - wood.indices.len() / 3) / 8).max(1);
    let mut lobes: Vec<_> = if detailed {
        limb_groups(&s.leaves, (groups / 2).max(1))
    } else {
        vec![s.leaves.clone()]
    }
    .into_iter()
    .map(|leaves| {
        let e = error(&leaves);
        (leaves, e)
    })
    .collect();
    while lobes.len() < groups {
        let Some((index, _)) = lobes
            .iter()
            .enumerate()
            .filter(|(_, v)| v.0.len() > 1)
            .max_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
        else {
            break;
        };
        let (l, r) = split(lobes.swap_remove(index).0);
        let (le, re) = (error(&l), error(&r));
        lobes.extend([(l, le), (r, re)]);
    }
    for (points, _) in &lobes {
        if detailed {
            branchlet(&mut foliage, points);
        } else {
            crown(&mut foliage, points, 4);
        }
    }
    Tier { wood, foliage }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 9 {
        return Err("bake_woodland_tree SPECIES SEED AGE HEIGHT TRUNK_RADIUS STANDARD_TRIS PHONE_TRIS OUTPUT.json".into());
    }
    let species = &args[1];
    let seed = args[2].parse::<u64>().map_err(|e| e.to_string())?;
    let age = args[3].parse::<u32>().map_err(|e| e.to_string())?;
    let height = args[4].parse::<f32>().map_err(|e| e.to_string())?;
    let radius = args[5].parse::<f32>().map_err(|e| e.to_string())?;
    let regular = args[6].parse::<usize>().map_err(|e| e.to_string())?;
    let phone = args[7].parse::<usize>().map_err(|e| e.to_string())?;
    if !(3..=100).contains(&age)
        || !(0.5..=50.).contains(&height)
        || !(0.015..=1.5).contains(&radius)
        || !(48..=4000).contains(&regular)
        || !(48..=regular).contains(&phone)
    {
        return Err("age, dimensions or triangle budgets outside supported range".into());
    }
    let mut s = match species.as_str() {
        "pine" | "spruce" => grown(species, age, seed),
        _ => recipe(species, age, seed)?,
    };
    s.fit(height, radius)?;
    let output = serde_json::json!({"species":species,"botanical_name":s.label,"source":s.model,"age":age,"seed":seed,
        "height":height,"trunk_radius":radius,"source_leaf_samples":s.leaves.len(),
        "standard":tier(&s,regular),"phone":tier(&s,phone)});
    let path = Path::new(&args[8]);
    fs::write(
        path,
        serde_json::to_vec(&output).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn main() -> Result<(), String> {
    run(&env::args().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_budgets_fit_one_grown_crown_and_keep_the_root_and_coverage() {
        for kind in ["pine", "spruce", "birch", "broadleaf"] {
            let mut s = if kind == "pine" || kind == "spruce" {
                grown(kind, 14, 18)
            } else {
                recipe(kind, 14, 18).unwrap()
            };
            s.fit(10., 0.25).unwrap();
            for budget in [115, 850] {
                let a = tier(&s, budget);
                let n = (a.wood.indices.len() + a.foliage.indices.len()) / 3;
                assert!(n <= budget && n > budget * 2 / 3, "{kind} {budget} -> {n}");
                assert_eq!(a.foliage.indices.len() / 3, a.foliage.tones.len());
                for m in [&a.wood, &a.foliage] {
                    assert_eq!(m.positions.len(), m.normals.len());
                    assert!(m.indices.iter().all(|i| (*i as usize) < m.positions.len()));
                    assert!(m
                        .normals
                        .iter()
                        .all(|n| (Vec3::from_array(*n).length() - 1.).abs() < 1e-4));
                }
                let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
                for face in a.foliage.indices.chunks_exact(3) {
                    for (u, v) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
                        let edge = edges.entry((u.min(v), u.max(v))).or_default();
                        edge.0 += 1;
                        edge.1 += if u < v { 1 } else { -1 };
                    }
                }
                assert!(
                    edges.values().all(|e| *e == (2, 0)),
                    "open or miswound crown"
                );
                let root = Vec3::from_array(a.wood.positions[0]);
                assert!(root.y.abs() < 1e-4 && (root.length() - 0.25).abs() < 0.005);
                let compass = (0..8).map(|i| {
                    let (z, x) = (i as f32 * std::f32::consts::TAU / 8.).sin_cos();
                    Vec3::new(x, 0., z)
                });
                for axis in compass.chain([
                    Vec3::Y,
                    Vec3::NEG_Y,
                    Vec3::new(1., 1., 1.).normalize(),
                    Vec3::new(-1., -1., -1.).normalize(),
                ]) {
                    let source_max = s
                        .leaves
                        .iter()
                        .map(|l| l.pos.dot(axis) + l.radius)
                        .fold(f32::MIN, f32::max);
                    let mesh_max = a
                        .foliage
                        .positions
                        .iter()
                        .map(|p| Vec3::from_array(*p).dot(axis))
                        .fold(f32::MIN, f32::max);
                    assert!(
                        (source_max - mesh_max).abs() < 0.55 + 0.08 * source_max.abs(),
                        "{kind} {axis} {source_max} {mesh_max}"
                    );
                }
                assert_eq!(
                    serde_json::to_string(&a).unwrap(),
                    serde_json::to_string(&tier(&s, budget)).unwrap()
                );
            }
        }
    }
}
