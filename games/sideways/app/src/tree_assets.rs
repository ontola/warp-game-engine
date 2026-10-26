//! Three shared evergreen silhouettes. Shape once before cluster/volume baking;
//! no additional runtime geometry, materials, or per-instance mesh copies.
use glam::Vec3;
use render::cluster::{Vertex, read_kmesh};
use sideways::vegetation::{TreeArchetype, TreePhenotype};
use std::path::Path;

pub type Surfaces = Vec<(String, Vec<Vertex>, Vec<u32>)>;

/// Explicit source experiment. A factor of one retains the existing source;
/// values above one enlarge each needle frond around its fixed stem anchor.
/// This is not a tree-count, alpha-cutoff, lighting, or runtime LOD setting.
pub fn configured_fullness() -> Result<f32, String> {
    match std::env::var("SIDEWAYS_TREE_FULLNESS") {
        Ok(value) => validate_fullness(
            value
                .parse::<f32>()
                .map_err(|_| "SIDEWAYS_TREE_FULLNESS must be a finite number in 1..1.35")?,
        ),
        Err(std::env::VarError::NotPresent) => Ok(1.0),
        Err(_) => Err("SIDEWAYS_TREE_FULLNESS must be valid Unicode".into()),
    }
}

fn validate_fullness(value: f32) -> Result<f32, String> {
    if !value.is_finite() || !(1.0..=1.35).contains(&value) {
        return Err("SIDEWAYS_TREE_FULLNESS must be a finite number in 1..1.35".into());
    }
    Ok(value)
}

fn is_crown(name: &str) -> bool {
    !name.contains("trunk") && !name.contains("bark")
}

#[derive(Debug)]
struct Frond {
    vertices: Vec<usize>,
    anchor: usize,
}

/// The retained source has 136 independent bent fronds, each eight vertices
/// and eight triangles. Its texture's stem begins at UV (0,0). Recognize that
/// exact contract instead of treating arbitrary connected crown meshes as
/// interchangeable leaf cards or moving the whole crown around the trunk.
fn fronds(vertices: &[Vertex], indices: &[u32]) -> Result<Vec<Frond>, String> {
    use std::collections::BTreeMap;
    if vertices.is_empty() || indices.is_empty() || indices.len() % 3 != 0 {
        return Err("crown fullness requires indexed source fronds".into());
    }
    if vertices.iter().any(|v| {
        v.pos
            .iter()
            .chain(&v.normal)
            .chain(&v.uv)
            .any(|x| !x.is_finite())
    }) || indices.iter().any(|&i| i as usize >= vertices.len())
    {
        return Err("crown fullness received invalid source geometry".into());
    }
    fn root(parents: &mut [usize], mut i: usize) -> usize {
        while parents[i] != i {
            parents[i] = parents[parents[i]];
            i = parents[i];
        }
        i
    }
    let mut parents: Vec<_> = (0..vertices.len()).collect();
    for t in indices.chunks_exact(3) {
        let a = root(&mut parents, t[0] as usize);
        let b = root(&mut parents, t[1] as usize);
        let c = root(&mut parents, t[2] as usize);
        parents[b] = a;
        parents[c] = a;
    }
    let mut groups = BTreeMap::<usize, (Vec<usize>, usize)>::new();
    for i in 0..vertices.len() {
        groups.entry(root(&mut parents, i)).or_default().0.push(i);
    }
    for t in indices.chunks_exact(3) {
        groups
            .get_mut(&root(&mut parents, t[0] as usize))
            .unwrap()
            .1 += 1;
    }
    groups
        .into_values()
        .map(|(members, triangles)| {
            if members.len() != 8 || triangles != 8 {
                return Err(
                    "crown fullness requires eight-vertex/eight-triangle source fronds".into(),
                );
            }
            let anchor = *members
                .iter()
                .min_by(|&&a, &&b| {
                    let length = |i: usize| vertices[i].uv[0].powi(2) + vertices[i].uv[1].powi(2);
                    length(a).total_cmp(&length(b)).then(a.cmp(&b))
                })
                .unwrap();
            let uv = vertices[anchor].uv;
            if uv[0].abs() > 0.02 || uv[1].abs() > 0.02 {
                return Err("crown frond has no authored stem anchor near UV (0,0)".into());
            }
            Ok(Frond {
                vertices: members,
                anchor,
            })
        })
        .collect()
}

fn expand_fronds(vertices: &mut [Vertex], indices: &[u32], fullness: f32) -> Result<(), String> {
    validate_fullness(fullness)?;
    if fullness == 1.0 {
        return Ok(());
    }
    let groups = fronds(vertices, indices)?;
    for group in groups {
        let anchor = Vec3::from(vertices[group.anchor].pos);
        for index in group.vertices {
            if index == group.anchor {
                continue;
            } // Keep exact source anchor bits.
            let position = Vec3::from(vertices[index].pos);
            vertices[index].pos = (anchor + (position - anchor) * fullness).to_array();
            // Positive uniform scaling preserves authored shading normals,
            // triangle winding and every UV. Existing taper below applies
            // its inverse-transpose using the new position when required.
        }
    }
    Ok(())
}

/// How crown needles are represented. `Cards` is the retained source: 136
/// alpha-tested fronds of eight triangles. `Opaque` puts one copy of the
/// opaque twig part (the needle texture's alpha silhouette as triangles,
/// `assets/meshes/pine_twig_part.kmesh`, baked by tools/twig_part.py) on every
/// frond card instead: no alpha test, about 240 times the crown triangles.
/// An experiment to measure cards against geometry, not a promoted default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crown {
    Cards,
    Opaque,
}

impl Crown {
    pub fn label(self) -> &'static str {
        match self {
            Crown::Cards => "cards",
            Crown::Opaque => "opaque",
        }
    }
}

pub fn configured_crown() -> Result<Crown, String> {
    match std::env::var("SIDEWAYS_TREE_CROWN").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("") | Ok("cards") => Ok(Crown::Cards),
        Ok("opaque") => Ok(Crown::Opaque),
        _ => Err("SIDEWAYS_TREE_CROWN must be cards or opaque".into()),
    }
}

pub fn load_twig_part(root: &Path) -> Result<(Vec<Vertex>, Vec<u32>), String> {
    let path = root.join("assets/meshes/pine_twig_part.kmesh");
    let mut surfaces =
        read_kmesh(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    if surfaces.len() != 1 {
        return Err(format!("{}: expected one twig surface", path.display()));
    }
    let (_, vertices, indices) = surfaces.pop().unwrap();
    if indices.is_empty()
        || indices.len() % 3 != 0
        || indices.iter().any(|&i| i as usize >= vertices.len())
        || vertices
            .iter()
            .any(|v| v.uv.iter().any(|x| !x.is_finite() || !(-0.01..=1.01).contains(x)))
    {
        return Err(format!("{}: invalid twig part", path.display()));
    }
    Ok((vertices, indices))
}

/// A point of the frond's texture square on its bent card: barycentric
/// weights in the card triangle that contains `uv`, or, outside the card's UV
/// footprint, the plane of the nearest one extended.
fn card_weights(card: &[[Vertex; 3]], uv: glam::Vec2) -> (usize, [f32; 3]) {
    let mut best = (0usize, [1.0f32, 0.0, 0.0], f32::NEG_INFINITY);
    for (k, t) in card.iter().enumerate() {
        let [a, b, c] = t.map(|v| glam::Vec2::from(v.uv));
        let area = (b - a).perp_dot(c - a);
        if area.abs() < 1e-12 {
            continue;
        }
        let w1 = (uv - a).perp_dot(c - a) / area;
        let w2 = (b - a).perp_dot(uv - a) / area;
        let w = [1.0 - w1 - w2, w1, w2];
        let inside = w.iter().copied().fold(f32::INFINITY, f32::min);
        if inside > best.2 {
            best = (k, w, inside);
        }
    }
    (best.0, best.1)
}

/// Replace every alpha-tested frond card with the opaque twig part mapped
/// onto it. Positions and normals follow the card (barycentric in UV space),
/// UVs stay the part's own, so albedo comes from the same texture. Each
/// mapped triangle is wound to face along its interpolated normal.
pub fn opaque_fronds(
    vertices: &[Vertex],
    indices: &[u32],
    part: &(Vec<Vertex>, Vec<u32>),
) -> Result<(Vec<Vertex>, Vec<u32>), String> {
    let groups = fronds(vertices, indices)?;
    let mut frond_of = vec![usize::MAX; vertices.len()];
    for (k, g) in groups.iter().enumerate() {
        for &v in &g.vertices {
            frond_of[v] = k;
        }
    }
    let mut cards = vec![Vec::<[Vertex; 3]>::new(); groups.len()];
    for t in indices.chunks_exact(3) {
        cards[frond_of[t[0] as usize]].push([0, 1, 2].map(|k| vertices[t[k] as usize]));
    }
    let (part_vertices, part_indices) = part;
    let mut out_vertices = Vec::with_capacity(groups.len() * part_vertices.len());
    let mut out_indices = Vec::with_capacity(groups.len() * part_indices.len());
    for card in &cards {
        let base = out_vertices.len() as u32;
        for pv in part_vertices {
            let (k, w) = card_weights(card, glam::Vec2::from(pv.uv));
            let t = &card[k];
            let pos: Vec3 = (0..3).map(|i| Vec3::from(t[i].pos) * w[i]).sum();
            let mut normal: Vec3 = (0..3).map(|i| Vec3::from(t[i].normal) * w[i]).sum();
            if normal.length_squared() < 1e-12 {
                normal = (Vec3::from(t[1].pos) - Vec3::from(t[0].pos))
                    .cross(Vec3::from(t[2].pos) - Vec3::from(t[0].pos));
            }
            out_vertices.push(Vertex {
                pos: pos.to_array(),
                normal: normal.normalize_or(Vec3::Y).to_array(),
                uv: pv.uv,
            });
        }
        out_indices.extend(part_indices.iter().map(|i| base + i));
    }
    orient_to_normals(&out_vertices, &mut out_indices);
    Ok((out_vertices, out_indices))
}

/// Wind each triangle to face along the sum of its vertex normals. Run again
/// after any warp that moves positions and normals differently (the taper).
fn orient_to_normals(vertices: &[Vertex], indices: &mut [u32]) {
    for t in indices.chunks_exact_mut(3) {
        let p = [t[0], t[1], t[2]].map(|i| Vec3::from(vertices[i as usize].pos));
        let n: Vec3 = t.iter().map(|&i| Vec3::from(vertices[i as usize].normal)).sum();
        if (p[1] - p[0]).cross(p[2] - p[0]).dot(n) < 0.0 {
            t.swap(1, 2);
        }
    }
}

pub fn crown_frond_count(surfaces: &Surfaces) -> Result<usize, String> {
    surfaces
        .iter()
        .filter(|(name, _, _)| is_crown(name))
        .try_fold(0, |count, (_, vertices, indices)| {
            Ok(count + fronds(vertices, indices)?.len())
        })
}

/// Native placements already carry the scale used by their trunk colliders.
/// Never reroll, reapply phenotype or recompute that authoritative value.
pub fn native_scale(spot: &sideways::builder::Spot) -> Vec3 {
    Vec3::new(spot.scale, spot.scale_y, spot.scale)
}

/// Same width/height policy as builder::terrain_chunk, driven by stateless
/// world-cell units for background instances. Height is relative to the same
/// width draw, not an independently enlarged absolute height. The multiplication
/// order matches the native f32 RNG path before the shared phenotype is applied.
pub fn scale_from_units(width_unit: f32, height_unit: f32, phenotype: TreePhenotype) -> Vec3 {
    let width = width_unit * (1.25f32 - 0.75) + 0.75;
    let height = width * (height_unit * (1.15f32 - 0.9) + 0.9);
    Vec3::new(
        width * phenotype.scale[0],
        height * phenotype.scale[1],
        width * phenotype.scale[0],
    )
}

/// Card crowns for the volume/voxel experiments; the live scene
/// (visuals.rs) honours SIDEWAYS_TREE_CROWN through load_with_crown.
pub fn load(root: &Path, archetype: TreeArchetype) -> Result<Surfaces, String> {
    load_with_fullness(root, archetype, configured_fullness()?)
}

pub fn load_with_fullness(
    root: &Path,
    archetype: TreeArchetype,
    fullness: f32,
) -> Result<Surfaces, String> {
    load_with_crown(root, archetype, fullness, Crown::Cards)
}

pub fn load_with_crown(
    root: &Path,
    archetype: TreeArchetype,
    fullness: f32,
    crown: Crown,
) -> Result<Surfaces, String> {
    validate_fullness(fullness)?;
    let twig = match crown {
        Crown::Cards => None,
        Crown::Opaque => Some(load_twig_part(root)?),
    };
    let variant = ["a", "b", "c"][archetype.index()];
    let path = root.join(format!("assets/meshes/fir_{variant}_near.kmesh"));
    let mut surfaces =
        read_kmesh(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    // Freeze the existing tree-wide warp before enlarging any leaf geometry.
    // Otherwise a taller expanded frond would indirectly retaper all wood,
    // moving the trunk/branches and invalidating the fixed-anchor promise.
    let taper_height = surfaces
        .iter()
        .flat_map(|(_, vertices, _)| vertices.iter())
        .map(|v| v.pos[1])
        .fold(0.0f32, f32::max);
    for (name, vertices, indices) in &mut surfaces {
        // The retained C trunk has no UVs at all. Do not sample an arbitrary
        // atlas corner: give just that wood a Y-axis cylinder unwrap and the
        // existing tileable bark texture. Original geometry/normals survive.
        if name.contains("trunk") && vertices.iter().all(|v| v.uv == [0.0; 2]) {
            cylinder_uvs(vertices, indices);
            *name = "bark_albedo.jpg".into();
        }
        if is_crown(name) {
            expand_fronds(vertices, indices, fullness)?;
            // Before the taper below, which then bends wood, cards and
            // twigs alike.
            if let Some(part) = &twig {
                let (v, i) = opaque_fronds(vertices, indices, part)?;
                *vertices = v;
                *indices = i;
            }
        }
    }
    // One coherent warp for wood and leaves preserves their attachment. The
    // lower trunk stays exactly as authored, including its physical radius.
    if archetype == TreeArchetype::Conifer {
        for (name, vertices, indices) in &mut surfaces {
            for v in vertices.iter_mut() {
                taper(v, taper_height);
            }
            if twig.is_some() && is_crown(name) {
                orient_to_normals(vertices, indices);
            }
        }
    }
    Ok(surfaces)
}

fn cylinder_uvs(vertices: &mut Vec<Vertex>, indices: &mut [u32]) {
    for v in vertices.iter_mut() {
        v.uv = [
            v.pos[2].atan2(v.pos[0]) / std::f32::consts::TAU + 0.5,
            v.pos[1] * 0.5,
        ];
    }
    let mut wrapped = std::collections::HashMap::<u32, u32>::new();
    for tri in indices.chunks_exact_mut(3) {
        let lo = tri
            .iter()
            .map(|i| vertices[*i as usize].uv[0])
            .fold(f32::INFINITY, f32::min);
        let hi = tri
            .iter()
            .map(|i| vertices[*i as usize].uv[0])
            .fold(f32::NEG_INFINITY, f32::max);
        if hi - lo <= 0.5 {
            continue;
        }
        for index in tri {
            if vertices[*index as usize].uv[0] >= 0.5 {
                continue;
            }
            *index = *wrapped.entry(*index).or_insert_with(|| {
                let mut vertex = vertices[*index as usize];
                vertex.uv[0] += 1.0;
                let duplicate = vertices.len() as u32;
                vertices.push(vertex);
                duplicate
            });
        }
    }
}

fn radial_scale(y: f32, height: f32) -> (f32, f32) {
    let start = (height * 0.38).max(5.5).min(height);
    let span = (height - start).max(0.001);
    let t = ((y - start) / span).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    (1.0 - 0.52 * smooth, -0.52 * 6.0 * t * (1.0 - t) / span)
}

fn taper(v: &mut Vertex, height: f32) {
    let p = Vec3::from(v.pos);
    let n = Vec3::from(v.normal);
    let (scale, derivative) = radial_scale(p.y, height);
    // Inverse transpose of x'=f(y)x, y'=y, z'=f(y)z. Copying the old normals
    // would change the good tree lighting and create false bright/dark bands.
    v.normal = Vec3::new(
        n.x / scale,
        n.y - derivative * (p.x * n.x + p.z * n.z) / scale,
        n.z / scale,
    )
    .normalize_or_zero()
    .to_array();
    v.pos = [p.x * scale, p.y, p.z * scale];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_scale_matches_native_rng_math_and_keeps_native_spots_bit_exact() {
        use sideways::{
            builder::{Spot, V3Ser},
            vegetation::{Season, Vegetation},
        };
        for seed in [0, 1, 683407368, 370671355, u64::MAX] {
            let mut native = gdcompat::Pcg::new(seed);
            let mut units = gdcompat::Pcg::new(seed);
            for i in 0..256 {
                let sc = native.randf_range_f32(0.75, 1.25);
                let yaw = (native.randf() as f64 * std::f64::consts::TAU) as f32;
                let sy = sc * native.randf_range_f32(0.9, 1.15);
                let u = units.randf();
                let observed_yaw = (units.randf() as f64 * std::f64::consts::TAU) as f32;
                let v = units.randf();
                assert_eq!(yaw.to_bits(), observed_yaw.to_bits());
                let phenotype =
                    Vegetation::new(seed, if i % 2 == 0 { 1.0 } else { 0.3 }, Season::Summer)
                        .unwrap()
                        .tree(683407368, -400.0 + i as f32 * 0.37, 17.0 - i as f32 * 0.91);
                let spot = Spot {
                    pos: V3Ser(-4.0, 12.0, -400.0),
                    yaw,
                    scale: sc * phenotype.scale[0],
                    scale_y: sy * phenotype.scale[1],
                };
                let reference = Vec3::new(spot.scale, spot.scale_y, spot.scale);
                assert_eq!(
                    native_scale(&spot).to_array().map(f32::to_bits),
                    reference.to_array().map(f32::to_bits)
                );
                assert_eq!(
                    scale_from_units(u, v, phenotype)
                        .to_array()
                        .map(f32::to_bits),
                    reference.to_array().map(f32::to_bits)
                );
            }
            assert_eq!(
                native.next_u32(),
                units.next_u32(),
                "native draw count/order must survive the shared visual policy"
            );
        }
    }

    #[test]
    fn matched_units_remove_extra_far_crown_area_and_proxy_mass() {
        let mut rng = gdcompat::Pcg::new(0x5d196342);
        let mut area = [0.0f64; 2];
        let mut volume = [0.0f64; 2];
        let mut masses = [[0.0f64; 2]; 3];
        for _ in 0..65536 {
            let u = rng.randf();
            let v = rng.randf();
            let old_w = 0.85 + 0.60 * u;
            let old_h = 0.85 + 0.55 * v;
            let new = scale_from_units(
                u,
                v,
                TreePhenotype {
                    archetype: TreeArchetype::Canopy,
                    juvenile: false,
                    scale: [1.0; 2],
                },
            );
            for (i, (w, h)) in [(old_w, old_h), (new.x, new.y)].into_iter().enumerate() {
                area[i] += f64::from(w).powi(2);
                volume[i] += f64::from(w).powi(2) * f64::from(h);
                for (family, [wide, tall]) in [[1.0f32, 1.0], [0.84, 1.20], [1.42, 0.96]]
                    .into_iter()
                    .enumerate()
                {
                    masses[family][i] += f64::from((w * wide).max(h * tall)).powi(3);
                }
            }
        }
        let n = 65536.0;
        eprintln!(
            "tree scale matched 65536 unit pairs: mean old/new crown XY area factor {:.6}/{:.6} ({:.2}% excess removed), volume {:.6}/{:.6}; legacy max-axis sphere mass old/new ratio canopy {:.5}, conifer {:.5}, stout {:.5}; source radius/mass constants cancel, geometric proxies not optical alpha coverage",
            area[0] / n,
            area[1] / n,
            (area[0] / area[1] - 1.0) * 100.0,
            volume[0] / n,
            volume[1] / n,
            masses[0][0] / masses[0][1],
            masses[1][0] / masses[1][1],
            masses[2][0] / masses[2][1]
        );
        assert!((area[0] / area[1] - 1.325).abs() < 0.01);
        assert!((volume[0] / volume[1] - 1.397).abs() < 0.015);
        assert!(masses.iter().all(|m| m[0] > m[1] * 1.25));
    }

    #[test]
    fn cylinder_unwrap_splits_only_uv_seam_without_moving_geometry() {
        let mut vertices: Vec<_> = [
            [-1.0, 0.0, 0.01],
            [-1.0, 0.0, -0.01],
            [-1.0, 1.0, 0.01],
            [-1.0, 1.0, -0.01],
        ]
        .into_iter()
        .map(|pos| Vertex {
            pos,
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0; 2],
        })
        .collect();
        let before = vertices.clone();
        let old_indices = [0, 1, 2, 1, 3, 2];
        let mut indices = old_indices;
        cylinder_uvs(&mut vertices, &mut indices);
        assert_eq!(
            vertices.len(),
            6,
            "two seam vertices are shared by both triangles"
        );
        for (new, old) in indices.iter().zip(old_indices) {
            assert_eq!(vertices[*new as usize].pos, before[old as usize].pos);
            assert_eq!(vertices[*new as usize].normal, before[old as usize].normal);
        }
        for tri in indices.chunks_exact(3) {
            let lo = tri
                .iter()
                .map(|i| vertices[*i as usize].uv[0])
                .fold(f32::INFINITY, f32::min);
            let hi = tri
                .iter()
                .map(|i| vertices[*i as usize].uv[0])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                hi - lo < 0.01,
                "seam must not stretch one texture wrap across a thin triangle"
            );
        }
    }

    #[test]
    fn taper_preserves_root_uvs_and_transforms_normals_with_the_surface() {
        let normal = Vec3::new(0.3, 0.4, 0.5).normalize();
        let mut root = Vertex {
            pos: [0.22, 2.5, -0.22],
            normal: normal.to_array(),
            uv: [0.4, 0.7],
        };
        let before = root;
        taper(&mut root, 14.0);
        assert_eq!(root.pos, before.pos);
        assert_eq!(root.uv, before.uv);
        assert!(Vec3::from(root.normal).distance(normal) < 1e-6);
        for y in [5.8, 8.0, 11.0, 13.5] {
            let position = Vec3::new(2.0, y, -1.0);
            let u = normal.any_orthonormal_vector();
            let w = normal.cross(u);
            let warp = |p: Vec3| {
                let (f, _) = radial_scale(p.y, 14.0);
                Vec3::new(p.x * f, p.y, p.z * f)
            };
            let du = warp(position + u * 0.001) - warp(position - u * 0.001);
            let dw = warp(position + w * 0.001) - warp(position - w * 0.001);
            let geometric = du.cross(dw).normalize();
            let mut vertex = Vertex {
                pos: position.to_array(),
                normal: normal.to_array(),
                uv: [0.1, 0.2],
            };
            taper(&mut vertex, 14.0);
            assert!(Vec3::from(vertex.normal).dot(geometric) > 0.9999);
            assert_eq!(vertex.uv, [0.1, 0.2]);
        }
    }

    #[test]
    fn real_source_archetypes_keep_geometry_and_repair_only_missing_trunk_uvs() {
        let root = sideways::game_assets::root();
        for archetype in TreeArchetype::ALL {
            let variant = ["a", "b", "c"][archetype.index()];
            let original = read_kmesh(
                &std::fs::read(root.join(format!("assets/meshes/fir_{variant}_near.kmesh")))
                    .unwrap(),
            )
            .unwrap();
            let shaped = load_with_fullness(&root, archetype, 1.0).unwrap();
            assert_eq!(shaped.len(), original.len());
            let height = original
                .iter()
                .flat_map(|(_, v, _)| v.iter())
                .map(|v| v.pos[1])
                .fold(0.0f32, f32::max);
            let mut changed = 0;
            for ((name, vertices, indices), (old_name, old_vertices, old_indices)) in
                shaped.iter().zip(&original)
            {
                let repaired =
                    old_name.contains("trunk") && old_vertices.iter().all(|v| v.uv == [0.0; 2]);
                assert_eq!(
                    name,
                    if repaired {
                        "bark_albedo.jpg"
                    } else {
                        old_name
                    }
                );
                assert_eq!(indices.len(), old_indices.len());
                if !repaired {
                    assert_eq!(vertices.len(), old_vertices.len());
                    assert_eq!(indices, old_indices);
                }
                for (index, old_index) in indices.iter().zip(old_indices) {
                    let v = &vertices[*index as usize];
                    let old = &old_vertices[*old_index as usize];
                    if !repaired {
                        assert_eq!(v.uv, old.uv);
                    }
                    assert_eq!(v.pos[1], old.pos[1]);
                    assert!(v.pos.iter().chain(v.normal.iter()).all(|f| f.is_finite()));
                    if archetype != TreeArchetype::Conifer || old.pos[1] <= height * 0.38 {
                        assert_eq!(v.pos, old.pos);
                    }
                    changed += usize::from(v.pos != old.pos);
                }
                if repaired {
                    let mapped = indices
                        .chunks_exact(3)
                        .filter(|tri| {
                            let u = tri
                                .iter()
                                .map(|i| glam::Vec2::from(vertices[*i as usize].uv))
                                .collect::<Vec<_>>();
                            (u[1] - u[0]).perp_dot(u[2] - u[0]).abs() > 1e-8
                        })
                        .count();
                    assert!(
                        mapped * 100 > indices.len() / 3 * 95,
                        "cylinder UVs must cover the actual trunk surface"
                    );
                    for tri in indices.chunks_exact(3) {
                        let u = tri
                            .iter()
                            .map(|i| vertices[*i as usize].uv[0])
                            .collect::<Vec<_>>();
                        // Axis-crossing tiny root triangles may exceed half a
                        // wrap, but a normal cylindrical seam must not do so.
                        assert!(u.iter().all(|value| value.is_finite()));
                    }
                }
            }
            assert_eq!(changed > 0, archetype == TreeArchetype::Conifer);
        }
    }

    #[test]
    fn fullness_keeps_exact_stems_wood_uvs_counts_and_rebuilds_source_bounds() {
        use render::cluster::{ClusterMesh, Part, SimplificationPolicy};
        let root = sideways::game_assets::root();
        for archetype in TreeArchetype::ALL {
            let reference = load_with_fullness(&root, archetype, 1.0).unwrap();
            let candidate = load_with_fullness(&root, archetype, 1.25).unwrap();
            assert_eq!(crown_frond_count(&candidate).unwrap(), 136);
            let mut changed = 0;
            for ((name, before, old_indices), (new_name, after, indices)) in
                reference.iter().zip(&candidate)
            {
                assert_eq!(name, new_name);
                assert_eq!(before.len(), after.len());
                assert_eq!(old_indices, indices);
                let anchors: std::collections::BTreeSet<_> = if is_crown(name) {
                    fronds(before, old_indices)
                        .unwrap()
                        .iter()
                        .map(|g| g.anchor)
                        .collect()
                } else {
                    (0..before.len()).collect()
                };
                for (i, (a, b)) in before.iter().zip(after).enumerate() {
                    assert_eq!(a.uv, b.uv);
                    assert!(b.pos.iter().chain(&b.normal).all(|x| x.is_finite()));
                    assert!((Vec3::from(b.normal).length_squared() - 1.0).abs() < 0.005);
                    if anchors.contains(&i) {
                        assert_eq!(
                            a.pos.map(f32::to_bits),
                            b.pos.map(f32::to_bits),
                            "stem or wood moved"
                        );
                        assert_eq!(
                            a.normal.map(f32::to_bits),
                            b.normal.map(f32::to_bits),
                            "stem or wood normal changed"
                        );
                    }
                    changed += usize::from(a.pos != b.pos);
                }
            }
            assert_eq!(
                changed,
                136 * 7,
                "only seven non-anchor vertices per frond expand"
            );
            let parts: Vec<_> = candidate
                .iter()
                .map(|(name, v, i)| Part {
                    vertices: v,
                    indices: i,
                    material: u32::from(is_crown(name)),
                })
                .collect();
            let mesh = ClusterMesh::build_with_policy(&parts, |p| {
                if p.material == 1 {
                    SimplificationPolicy::AlphaCards
                } else {
                    SimplificationPolicy::TexturedGeometry
                }
            });
            assert_eq!(
                mesh.leaf_triangles(),
                candidate.iter().map(|(_, _, i)| i.len() / 3).sum::<usize>()
            );
            let centre = Vec3::from_slice(&mesh.sphere);
            assert!(
                candidate
                    .iter()
                    .flat_map(|(_, v, _)| v)
                    .all(|v| Vec3::from(v.pos).distance(centre) <= mesh.sphere[3] + 1e-4),
                "rebuilt mesh sphere must contain enlarged source"
            );
        }
    }

    #[test]
    fn fullness_is_bounded_and_uniform_frond_scaling_preserves_winding_and_normals() {
        for invalid in [0.0, 0.99, 1.351, f32::NAN, f32::INFINITY] {
            assert!(validate_fullness(invalid).is_err());
        }
        let root = sideways::game_assets::root();
        let source =
            read_kmesh(&std::fs::read(root.join("assets/meshes/fir_a_near.kmesh")).unwrap())
                .unwrap();
        let (_, original, indices) = source.iter().find(|(name, _, _)| is_crown(name)).unwrap();
        let mut enlarged = original.clone();
        expand_fronds(&mut enlarged, indices, 1.25).unwrap();
        for (old, new) in original.iter().zip(&enlarged) {
            assert_eq!(old.normal, new.normal);
            assert_eq!(old.uv, new.uv);
        }
        for t in indices.chunks_exact(3) {
            let face = |v: &[Vertex]| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(v[i as usize].pos));
                (b - a).cross(c - a)
            };
            let old = face(original);
            let new = face(&enlarged);
            assert!(old.normalize().dot(new.normalize()) > 0.99999);
            assert!((new.length() / old.length() - 1.25f32.powi(2)).abs() < 0.001);
        }
        assert!(fronds(original, &indices[..indices.len() - 3]).is_err());
        let mut wrong_uv = original.clone();
        for v in &mut wrong_uv {
            v.uv = [0.5, 0.5];
        }
        assert!(fronds(&wrong_uv, indices).is_err());
    }

    // CPU source-union diagnostic only: orthographic silhouettes at the
    // finest geometry/texture level, with bilinear alpha and the live .42
    // cutoff. It does not certify GPU mip/LOD, lighting or wind continuity.
    fn projected_crown(
        surfaces: &Surfaces,
        right: glam::Vec3,
        up: glam::Vec3,
    ) -> Vec<([glam::Vec2; 3], [glam::Vec2; 3])> {
        surfaces
            .iter()
            .filter(|(name, _, _)| is_crown(name))
            .flat_map(|(_, vertices, indices)| {
                indices.chunks_exact(3).map(move |t| {
                    let v = [t[0], t[1], t[2]].map(|i| vertices[i as usize]);
                    (
                        v.map(|v| {
                            let p = Vec3::from(v.pos);
                            glam::Vec2::new(p.dot(right), p.dot(up))
                        }),
                        v.map(|v| glam::Vec2::from(v.uv)),
                    )
                })
            })
            .collect()
    }

    fn hull(mut points: Vec<glam::Vec2>) -> Vec<glam::Vec2> {
        points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        points.dedup();
        let mut chain = Vec::<glam::Vec2>::new();
        for &p in &points {
            while chain.len() >= 2
                && (chain[chain.len() - 1] - chain[chain.len() - 2])
                    .perp_dot(p - chain[chain.len() - 1])
                    <= 0.0
            {
                chain.pop();
            }
            chain.push(p);
        }
        let lower = chain.len();
        for &p in points.iter().rev().skip(1) {
            while chain.len() > lower
                && (chain[chain.len() - 1] - chain[chain.len() - 2])
                    .perp_dot(p - chain[chain.len() - 1])
                    <= 0.0
            {
                chain.pop();
            }
            chain.push(p);
        }
        chain.pop();
        chain
    }

    fn crown_mask(
        triangles: &[([glam::Vec2; 3], [glam::Vec2; 3])],
        texture: &image::RgbaImage,
        centre: glam::Vec2,
        extent: f32,
        size: usize,
    ) -> Vec<bool> {
        let mut mask = vec![false; size * size];
        let alpha = |uv: glam::Vec2| {
            let p = uv.clamp(glam::Vec2::ZERO, glam::Vec2::ONE)
                * glam::Vec2::new(texture.width() as f32, texture.height() as f32)
                - glam::Vec2::splat(0.5);
            let base = p.floor();
            let f = p - base;
            let texel = |x: i32, y: i32| {
                texture.get_pixel(
                    x.clamp(0, texture.width() as i32 - 1) as u32,
                    y.clamp(0, texture.height() as i32 - 1) as u32,
                )[3] as f32
                    / 255.0
            };
            let a = texel(base.x as i32, base.y as i32) * (1.0 - f.x)
                + texel(base.x as i32 + 1, base.y as i32) * f.x;
            let b = texel(base.x as i32, base.y as i32 + 1) * (1.0 - f.x)
                + texel(base.x as i32 + 1, base.y as i32 + 1) * f.x;
            a * (1.0 - f.y) + b * f.y
        };
        for &(world, uv) in triangles {
            let [a, b, c] = world.map(|p| {
                (p - centre) / extent * (size as f32 - 4.0) + glam::Vec2::splat(size as f32 * 0.5)
            });
            let area = (b - a).perp_dot(c - a);
            if area.abs() < 1e-7 {
                continue;
            }
            let lo = a.min(b).min(c).floor().max(glam::Vec2::ZERO);
            let hi = a
                .max(b)
                .max(c)
                .ceil()
                .min(glam::Vec2::splat(size as f32 - 1.0));
            for y in lo.y as usize..=hi.y as usize {
                for x in lo.x as usize..=hi.x as usize {
                    if mask[y * size + x] {
                        continue;
                    }
                    let p = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w1 = (p - a).perp_dot(c - a) / area;
                    let w2 = (b - a).perp_dot(p - a) / area;
                    let w0 = 1.0 - w1 - w2;
                    if w0 >= 0.0
                        && w1 >= 0.0
                        && w2 >= 0.0
                        && alpha(uv[0] * w0 + uv[1] * w1 + uv[2] * w2) >= 0.42
                    {
                        mask[y * size + x] = true;
                    }
                }
            }
        }
        mask
    }

    #[test]
    fn opaque_crown_places_one_twig_per_frond_on_its_card() {
        let root = sideways::game_assets::root();
        let part = load_twig_part(&root).unwrap();
        for archetype in TreeArchetype::ALL {
            let cards = load_with_crown(&root, archetype, 1.0, Crown::Cards).unwrap();
            let opaque = load_with_crown(&root, archetype, 1.0, Crown::Opaque).unwrap();
            assert_eq!(cards.len(), opaque.len());
            for ((name, cv, ci), (oname, ov, oi)) in cards.iter().zip(&opaque) {
                assert_eq!(name, oname);
                if !is_crown(name) {
                    assert_eq!((cv.len(), ci.len()), (ov.len(), oi.len()), "wood is untouched");
                    continue;
                }
                let fronds = fronds(cv, ci).unwrap().len();
                assert_eq!(fronds, 136);
                assert_eq!(oi.len(), fronds * part.1.len());
                assert_eq!(ov.len(), fronds * part.0.len());
                assert!(ov.iter().all(|v| v.pos.iter().chain(&v.normal).all(|x| x.is_finite())));
                // Every twig vertex stays near its card: extrapolation past
                // the card's UV footprint is bounded.
                let (lo, hi) = cv.iter().fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(Vec3::from(v.pos)), hi.max(Vec3::from(v.pos)))
                });
                let margin = (hi - lo).max_element() * 0.05;
                assert!(ov.iter().all(|v| {
                    let p = Vec3::from(v.pos);
                    p.cmpge(lo - margin).all() && p.cmple(hi + margin).all()
                }));
                // Triangles face along their shading normals.
                let flipped = oi.chunks_exact(3).filter(|t| {
                    let p = [t[0], t[1], t[2]].map(|i| Vec3::from(ov[i as usize].pos));
                    let n: Vec3 = t.iter().map(|&i| Vec3::from(ov[i as usize].normal)).sum();
                    let g = (p[1] - p[0]).cross(p[2] - p[0]);
                    g.dot(n) < -1e-3 * g.length() * n.length()
                }).count();
                eprintln!("{} {name}: {flipped} of {} twig triangles face against their normals", archetype.label(), oi.len() / 3);
                assert_eq!(flipped, 0);
            }
        }
        assert!(matches!(configured_crown(), Ok(Crown::Cards) | Ok(Crown::Opaque)));
    }

    /// The opaque crown must cover what the alpha-tested crown covers, seen
    /// from around the tree: CPU orthographic masks of the finest source.
    #[test]
    fn opaque_crown_silhouette_matches_alpha_tested_cards() {
        let root = sideways::game_assets::root();
        let texture = image::open(root.join("assets/trees/authored_pine_albedo.png"))
            .unwrap()
            .to_rgba8();
        let solid = image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4]));
        let size = 384usize;
        for archetype in TreeArchetype::ALL {
            let cards = load_with_crown(&root, archetype, 1.0, Crown::Cards).unwrap();
            let opaque = load_with_crown(&root, archetype, 1.0, Crown::Opaque).unwrap();
            let mut rows = Vec::new();
            let mut worst = (1.0f64, f64::INFINITY, 0.0f64);
            for elevation in [0.0f32, 30.0] {
                for angle in 0..8 {
                    let yaw = angle as f32 * std::f32::consts::TAU / 8.0;
                    let pitch = elevation.to_radians();
                    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
                    let direction =
                        Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos());
                    let up = direction.cross(right);
                    let a = projected_crown(&cards, right, up);
                    let b = projected_crown(&opaque, right, up);
                    let points: Vec<_> = a.iter().flat_map(|(p, _)| p.iter().copied()).collect();
                    let lo = points.iter().copied().fold(glam::Vec2::splat(f32::INFINITY), glam::Vec2::min);
                    let hi = points.iter().copied().fold(glam::Vec2::splat(f32::NEG_INFINITY), glam::Vec2::max);
                    let centre = (lo + hi) * 0.5;
                    let extent = (hi - lo).max_element();
                    let alpha = crown_mask(&a, &texture, centre, extent, size);
                    let geometry = crown_mask(&b, &solid, centre, extent, size);
                    let both = alpha.iter().zip(&geometry).filter(|(a, b)| **a && **b).count();
                    let either = alpha.iter().zip(&geometry).filter(|(a, b)| **a || **b).count();
                    let ratio = geometry.iter().filter(|x| **x).count() as f64
                        / alpha.iter().filter(|x| **x).count() as f64;
                    let iou = both as f64 / either as f64;
                    rows.push(serde_json::json!({"azimuth_degrees":angle*45,"elevation_degrees":elevation,"iou":iou,"coverage_ratio":ratio}));
                    worst = (worst.0.min(iou), worst.1.min(ratio), worst.2.max(ratio));
                }
            }
            eprintln!("opaque crown silhouette: {}", serde_json::json!({"archetype":archetype.label(),
                "scope":"CPU finest-source orthographic coverage, opaque twig geometry vs alpha-tested cards at cutoff 0.42; not GPU/LOD/lighting parity",
                "mask_size":size,"views":rows}));
            eprintln!("{}: worst IoU {:.3}, coverage {:.3}..{:.3}", archetype.label(), worst.0, worst.1, worst.2);
            // Needles are sub-pixel at this mask scale, so per-pixel overlap of
            // two thin-structure masks is pessimistic; total coverage is what
            // a distant viewer sees. Measured: IoU >= 0.81, coverage 0.96-0.98.
            assert!(worst.0 > 0.8 && worst.1 > 0.94 && worst.2 < 1.06, "{}: {worst:?}", archetype.label());
        }
    }

    #[test]
    fn fuller_crowns_increase_multiangle_alpha_union_at_fixed_geometry_budget() {
        let root = sideways::game_assets::root();
        let texture = image::open(root.join("assets/trees/authored_pine_albedo.png"))
            .unwrap()
            .to_rgba8();
        let size = 384usize;
        for archetype in TreeArchetype::ALL {
            let source = load_with_fullness(&root, archetype, 1.0).unwrap();
            let full = load_with_fullness(&root, archetype, 1.25).unwrap();
            let mut source_total = 0usize;
            let mut full_total = 0usize;
            let mut rows = Vec::new();
            for elevation in [0.0f32, 20.0] {
                for angle in 0..8 {
                    let yaw = angle as f32 * std::f32::consts::TAU / 8.0;
                    let pitch = elevation.to_radians();
                    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
                    let direction = Vec3::new(
                        yaw.sin() * pitch.cos(),
                        pitch.sin(),
                        yaw.cos() * pitch.cos(),
                    );
                    let up = direction.cross(right);
                    let a = projected_crown(&source, right, up);
                    let b = projected_crown(&full, right, up);
                    let points: Vec<_> = a
                        .iter()
                        .chain(&b)
                        .flat_map(|(p, _)| p.iter().copied())
                        .collect();
                    let lo = points
                        .iter()
                        .copied()
                        .fold(glam::Vec2::splat(f32::INFINITY), glam::Vec2::min);
                    let hi = points
                        .iter()
                        .copied()
                        .fold(glam::Vec2::splat(f32::NEG_INFINITY), glam::Vec2::max);
                    let centre = (lo + hi) * 0.5;
                    let extent = (hi - lo).max_element();
                    let masks = [
                        crown_mask(&a, &texture, centre, extent, size),
                        crown_mask(&b, &texture, centre, extent, size),
                    ];
                    let outline = hull(a.iter().flat_map(|(p, _)| p.iter().copied()).collect());
                    let mut count = [0usize; 2];
                    let mut interior = 0usize;
                    for y in 0..size {
                        for x in 0..size {
                            let p = (glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5)
                                - glam::Vec2::splat(size as f32 * 0.5))
                                / (size as f32 - 4.0)
                                * extent
                                + centre;
                            if (0..outline.len()).all(|i| {
                                (outline[(i + 1) % outline.len()] - outline[i])
                                    .perp_dot(p - outline[i])
                                    >= 0.0
                            }) {
                                interior += 1;
                                for k in 0..2 {
                                    count[k] += usize::from(masks[k][y * size + x]);
                                }
                            }
                        }
                    }
                    assert!(
                        count[0] > 1000 && interior > count[0],
                        "diagnostic must contain source needles and background holes"
                    );
                    assert!(
                        count[1] as f64 >= count[0] as f64 * 0.97,
                        "{} angle{angle} elevation{elevation}: interior alpha union regressed {count:?}",
                        archetype.label()
                    );
                    source_total += count[0];
                    full_total += count[1];
                    rows.push(serde_json::json!({"azimuth_degrees":angle*45,"elevation_degrees":elevation,
                    "reference_hull_pixels":interior,"source_covered_pixels":count[0],"fuller_covered_pixels":count[1],
                    "source_fill":count[0]as f64/interior as f64,"fuller_fill":count[1]as f64/interior as f64}));
                }
            }
            let gain = full_total as f64 / source_total as f64;
            eprintln!(
                "tree fullness source coverage: {}",
                serde_json::json!({"archetype":archetype.label(),"fullness":1.25,
                "fronds":136,"crown_triangles":1088,"mask_size":size,"alpha_cut":0.42,
                "scope":"CPU finest-source orthographic alpha union inside SAME reference crown convex hull; not GPU/LOD/lighting parity",
                "interior_coverage_ratio":gain,"views":rows})
            );
            assert!(
                gain > 1.05,
                "{} failed to add meaningful interior coverage: {gain}",
                archetype.label()
            );
        }
    }
}
