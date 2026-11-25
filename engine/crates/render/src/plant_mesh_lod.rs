//! A recipe plant as one rigid cluster-LOD mesh of cards, for the middle
//! distance: nearer than its voxel proxy, further than the few full
//! assemblies with branch wind. Like Nanite foliage: geometry first, voxels
//! only far away.
//!
//! The expanded plant is far too big to draw (a 16 m spruce is ~30M
//! triangles in ~5000 placements of a handful of parts), and geometric
//! simplification cannot merge separate sprays (a spruce stays ~176k
//! triangles at any cut). Each foliage placement becomes one or two quads: its
//! part baked once into a card (an alpha silhouette in one cell of the
//! plant's atlas), placed where the part was. Wood keeps a geometric cut of
//! its own cluster DAG. The merged mesh gets an alpha-card cluster DAG, so
//! the renderer's screen-error cut thins it further with distance. One
//! instance per tree, shared by every tree of the variant.
//!
//! ```ignore
//! let cards = PlantCards::cook(&assembly, 0.03)?;
//! let layer = scene.add_image("spruce cards", cards.atlas.clone());
//! let materials: Vec<u32> = cards.materials(layer).into_iter().map(|m| scene.add_material(m)).collect();
//! let mesh = scene.add_mesh(&cards.mesh_with_materials(&materials));
//! ```
use crate::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex};
use crate::foliage_assembly::Assembly;
use crate::scene::{Material, ALPHA_TEST, THIN_FOLIAGE, TWO_SIDED};
use glam::{Mat3, Vec2, Vec3};
use std::collections::HashMap;

/// The atlas: one square layer of the scene's texture array.
pub const ATLAS_SIZE: u32 = 2048;
/// Parts smaller than this (part space, metres) become cards whatever their
/// material: sprays and their twigs, not trunks and branches.
const CARD_MAX_M: f32 = 2.0;
/// Darkening per layer of needles stacked over a card texel past the first:
/// the source shades a needle spray's inner layers in their own shadow, a
/// flat card would not. Leaves lie in fewer, flatter layers and keep their
/// tone (forest_ladder measures both against the source).
const OVERLAP_SHADE: f32 = 0.6;
/// Parts whose triangles are thinner than this (area over longest edge
/// squared) are needles.
const NEEDLE_THINNESS: f32 = 0.1;

/// The merged plant: its mesh, the atlas its cards sample, and its
/// materials: the assembly's (wood), then one card material per foliage
/// material (`materials(layer)`). Cluster materials index that list.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PlantCards {
    pub mesh: ClusterMesh,
    pub wood: Vec<Material>,
    /// Card materials' source materials (tint, roughness), by card index.
    pub cards: Vec<Material>,
    #[serde(with = "rgba")]
    pub atlas: image::RgbaImage,
    pub quads: usize,
    pub wood_triangles: usize,
}

mod rgba {
    pub fn serialize<S: serde::Serializer>(image: &image::RgbaImage, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&(image.width(), image.height(), image.as_raw()), s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<image::RgbaImage, D::Error> {
        let (w, h, raw): (u32, u32, Vec<u8>) = serde::Deserialize::deserialize(d)?;
        image::RgbaImage::from_raw(w, h, raw).ok_or_else(|| serde::de::Error::custom("atlas size"))
    }
}

fn foliage(material: &Material) -> bool {
    material.flags & (ALPHA_TEST | THIN_FOLIAGE) != 0
}

/// Fit each card to the part itself so a tilted leaf projects face-on. A
/// volumetric spray also needs a side silhouette when viewed edge-on.
#[derive(Clone, Copy)]
struct Plane { x: Vec3, y: Vec3 }

impl Plane {
    fn normal(self) -> Vec3 { self.x.cross(self.y) }
    fn project(self, p: Vec3) -> Vec2 { Vec2::new(p.dot(self.x), p.dot(self.y)) }
}

/// The principal direction of a positive semidefinite covariance matrix.
/// Start on its largest column so we do not miss an axis-aligned part.
fn principal(c: Mat3) -> Vec3 {
    let mut v = c.y_axis;
    for col in [c.x_axis, c.z_axis] {
        if col.length_squared() > v.length_squared() { v = col; }
    }
    v = v.normalize_or_zero();
    for _ in 0..10 { v = (c * v).normalize_or_zero(); }
    v
}

fn planes(vertices: &[Vertex], indices: &[u32], allow_side: bool) -> Vec<Plane> {
    let triangle = |ids: &[u32]| [0, 1, 2].map(|k| Vec3::from(vertices[ids[k] as usize].pos));
    let area = |[a, b, c]: [Vec3; 3]| (b - a).cross(c - a).length();
    let mut sum = Vec3::ZERO;
    let mut weight = 0.0;
    for ids in indices.chunks_exact(3) {
        let pts = triangle(ids);
        let w = area(pts);
        sum += (pts[0] + pts[1] + pts[2]) * w;
        weight += w;
    }
    if weight == 0.0 { return vec![Plane { x: Vec3::X, y: Vec3::Y }]; }
    let center = sum / (3.0 * weight);
    let outer = |p: Vec3| Mat3::from_cols(p * p.x, p * p.y, p * p.z);
    let mut cov = Mat3::ZERO;
    for ids in indices.chunks_exact(3) {
        let pts = triangle(ids);
        let [a, b, c] = pts.map(|p| p - center);
        // Integral of p pᵀ over a triangle, common denominator omitted.
        cov += (outer(a) + outer(b) + outer(c) + outer(a + b + c)) * area(pts);
    }
    cov *= 1.0 / weight;
    let y = principal(cov);
    let projected = Mat3::IDENTITY - outer(y);
    let x = projected * principal(projected * cov * projected);
    let x = x.try_normalize().unwrap_or_else(|| y.any_orthonormal_vector());
    let front = Plane { x, y };
    let n = front.normal();
    // RMS depth at least a third of the smaller in-plane radius. A tilted,
    // slightly cupped leaf remains one card; a round spray gets two.
    if allow_side && 9.0 * n.dot(cov * n) > x.dot(cov * x) {
        vec![front, Plane { x: n, y }]
    } else {
        vec![front]
    }
}

#[derive(Clone, Copy)]
struct Card { source_normals: Option<[Vec3; 4]>, plane: Plane, lo: Vec2, hi: Vec2, depth: f32, uv0: Vec2, uv1: Vec2 }

/// Bake one orthographic silhouette and its local-space depth.
fn bake_card(vertices: &[Vertex], indices: &[u32], atlas: &mut image::RgbaImage, cell: (u32, u32, u32), plane: Plane) -> Card {
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for v in vertices { lo = lo.min(plane.project(Vec3::from(v.pos))); hi = hi.max(plane.project(Vec3::from(v.pos))); }
    let size = (hi - lo).max(Vec2::splat(1e-4));
    // Fit the card into the square cell, keeping its aspect.
    let (cx, cy, side) = cell;
    let fit = side as f32 / size.x.max(size.y);
    let (w, h) = (((size.x * fit) as u32).clamp(1, side), ((size.y * fit) as u32).clamp(1, side));
    const SS: u32 = 4;
    let (mut area, mut edge2) = (0.0f32, 0.0f32);
    for t in indices.chunks_exact(3) {
        let p: [Vec3; 3] = std::array::from_fn(|k| Vec3::from(vertices[t[k] as usize].pos));
        area += (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
        edge2 += (p[1] - p[0]).length_squared().max((p[2] - p[1]).length_squared()).max((p[0] - p[2]).length_squared());
    }
    let needles = area < NEEDLE_THINNESS * edge2;
    let needle_spray = vertices.iter().any(|v| v.uv[1] < -2.5);
    let mut front_depth = needle_spray.then(|| vec![f32::NEG_INFINITY; (w * h) as usize]);
    // Integrate the actual spray's normals, weighted by triangle area and
    // screen quadrant. Four samples can travel in the existing card vertices.
    let mut normals = [Vec3::ZERO; 4];
    let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
    let mut cover = vec![0u16; (w * h) as usize];
    for t in indices.chunks_exact(3) {
        let p: [Vec2; 3] = std::array::from_fn(|k| (plane.project(Vec3::from(vertices[t[k] as usize].pos)) - lo) / size * Vec2::new(w as f32, h as f32) * SS as f32);
        let (bmin, bmax) = (p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO), p[0].max(p[1]).max(p[2]).ceil().min(Vec2::new((w * SS) as f32, (h * SS) as f32)));
        let edge = |a: Vec2, b: Vec2, q: Vec2| (b - a).perp_dot(q - a);
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-9 { continue; }
        let xyz = [0, 1, 2].map(|k| Vec3::from_array(vertices[t[k] as usize].pos));
        let depths = xyz.map(|v| v.dot(plane.normal()));
        if needle_spray {
            let mid = ((plane.project((xyz[0] + xyz[1] + xyz[2]) / 3.0) - lo) / size).clamp(Vec2::ZERO, Vec2::ONE);
            let n = t.iter().map(|&i| Vec3::from_array(vertices[i as usize].normal)).sum::<Vec3>();
            let surface = (xyz[1] - xyz[0]).cross(xyz[2] - xyz[0]).length();
            for (sum, corner) in normals.iter_mut().zip(corners) {
                let weight = Vec2::ONE - (mid - corner).abs();
                *sum += n * surface * (weight.x * weight.y).powi(2);
            }
        }
        // Thin needles cover few samples: count each sample once per pixel.
        let mut hit = std::collections::HashSet::new();
        for sy in bmin.y as u32..bmax.y as u32 {
            for sx in bmin.x as u32..bmax.x as u32 {
                let q = Vec2::new(sx as f32 + 0.5, sy as f32 + 0.5);
                let (e0, e1, e2) = (edge(p[1], p[2], q) / area, edge(p[2], p[0], q) / area, edge(p[0], p[1], q) / area);
                if e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0 {
                    hit.insert((sx, sy));
                    if let Some(front) = &mut front_depth {
                        let i = ((sy / SS) * w + sx / SS) as usize;
                        front[i] = front[i].max(e0 * depths[0] + e1 * depths[1] + e2 * depths[2]);
                    }
                }
            }
        }
        for (sx, sy) in hit { cover[((sy / SS) * w + sx / SS) as usize] += 1; }
    }
    // Overlapping triangles count twice; coverage saturates. Fattened a little
    // so the alpha test keeps thin sprays' density at mip levels.
    for py in 0..h {
        for px in 0..w {
            let raw = cover[(py * w + px) as usize] as f32 / (SS * SS) as f32;
            let c = (raw * 1.6).min(1.0);
            // Where the part's own triangles stack, the source shades its
            // inner layers in their own shadow; a flat card would not.
            let mut shade = if needles { 1.0 / (1.0 + OVERLAP_SHADE * (raw - 1.0).max(0.0)) } else { 1.0 };
            if let Some(front) = &front_depth {
                // A neighbouring shoot above this surface blocks a fraction of
                // the local sky. Empty texels leave tips exposed. Directional
                // light and cast shadows are still evaluated at draw time.
                let depth = front[(py * w + px) as usize];
                if depth.is_finite() {
                    let radius = (w.min(h) / 12).max(1) as i32;
                    let mut blocked = 0.0;
                    for (dx, dy) in [(1,0),(-1,0),(0,1),(0,-1),(1,1),(-1,1),(-1,-1),(1,-1)] {
                        let (x, y) = (px as i32 + dx * radius, py as i32 + dy * radius);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 { continue; }
                        let rise = (front[(y as u32 * w + x as u32) as usize] - depth).max(0.0);
                        let run = (Vec2::new(dx as f32, dy as f32) * radius as f32 / fit).length();
                        blocked += rise / (rise + run);
                    }
                    shade *= 1.0 - 0.50 * blocked / 8.0;
                }
            }
            let v = (shade.powf(1.0 / 2.2) * 255.0) as u8;
            // Image rows run top down; the card's +y runs up.
            atlas.put_pixel(cx + px, cy + (h - 1 - py), image::Rgba([v, v, v, (c * 255.0) as u8]));
        }
    }
    let depth = vertices.iter().map(|v| Vec3::from(v.pos).dot(plane.normal())).sum::<f32>() / vertices.len().max(1) as f32;
    let at = atlas.width() as f32;
    Card { source_normals: needle_spray.then(|| normals.map(|n| n.normalize_or(plane.normal()))), plane, lo, hi, depth, uv0: Vec2::new(cx as f32, cy as f32) / at, uv1: Vec2::new((cx + w) as f32, (cy + h) as f32) / at }
}

fn part_mesh(vertices: &[Vertex], indices: &[u32]) -> ClusterMesh {
    ClusterMesh::build_with_policy(&[Part { vertices, indices, material: 0 }], |_| SimplificationPolicy::TexturedGeometry)
}

impl PlantCards {
    /// `assembly` at rest: foliage as cards, wood cut at `wood_error_m` metres.
    pub fn cook(assembly: &Assembly, wood_error_m: f32) -> Result<Self, String> {
        Self::cook_with_atlas(assembly, wood_error_m, ATLAS_SIZE)
    }

    /// The same bake for a smaller transfer or GPU texture budget.
    pub fn cook_with_atlas(assembly: &Assembly, wood_error_m: f32, atlas_size: u32) -> Result<Self, String> {
        if !atlas_size.is_power_of_two() || !(64..=ATLAS_SIZE).contains(&atlas_size) {
            return Err("plant cards: atlas side must be a power of two from 64 to 2048".into());
        }
        if !(wood_error_m > 0.0) { return Err("plant cards: wood error must be positive".into()); }
        let branches = assembly.branch_transforms()?;
        let wood = assembly.materials.clone();
        // Cards per small (part, material): foliage, and the twigs it grows
        // on; trunk and branches stay geometry. Card materials per material.
        let small = |part: u32| assembly.parts.get(part as usize).is_some_and(|g| {
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for v in &g.vertices { lo = lo.min(Vec3::from(v.pos)); hi = hi.max(Vec3::from(v.pos)); }
            (hi - lo).max_element() < CARD_MAX_M
        });
        let leafy: Vec<(u32, u32)> = {
            let mut seen: Vec<(u32, u32)> = assembly.instances.iter()
                .filter(|i| assembly.materials.get(i.material as usize).is_some_and(foliage) || small(i.part))
                .map(|i| (i.part, i.material)).collect();
            seen.sort_unstable();
            seen.dedup();
            seen
        };
        let mut layouts = Vec::with_capacity(leafy.len());
        for &(part, material) in &leafy {
            let g = assembly.parts.get(part as usize).ok_or("plant cards: part out of range")?;
            let m = assembly.materials.get(material as usize).ok_or("plant cards: material out of range")?;
            layouts.push(planes(&g.vertices, &g.indices, foliage(m)));
        }
        let count: usize = layouts.iter().map(Vec::len).sum();
        let columns = (count as f32).sqrt().ceil().max(1.0) as u32;
        let side = atlas_size / columns;
        if side == 0 { return Err("plant cards: too many foliage shapes for atlas".into()); }
        let mut atlas = image::RgbaImage::from_pixel(atlas_size, atlas_size, image::Rgba([255, 255, 255, 0]));
        let mut cards = HashMap::new();
        let mut slot = 0;
        for (&key, layout) in leafy.iter().zip(layouts) {
            let g = &assembly.parts[key.0 as usize];
            let projections = layout.into_iter().map(|plane| {
                let cell = ((slot % columns) * side, (slot / columns) * side, side);
                slot += 1;
                bake_card(&g.vertices, &g.indices, &mut atlas, cell, plane)
            }).collect::<Vec<_>>();
            cards.insert(key, projections);
        }
        let mut card_materials: Vec<u32> = leafy.iter().map(|&(_, m)| m).collect();
        card_materials.sort_unstable();
        card_materials.dedup();
        let card_index = |m: u32| wood.len() as u32 + card_materials.iter().position(|&c| c == m).unwrap() as u32;
        let mut merged: Vec<(Vec<Vertex>, Vec<u32>)> = vec![(Vec::new(), Vec::new()); wood.len() + card_materials.len()];
        let mut woods: HashMap<u32, ClusterMesh> = HashMap::new();
        let (mut quads, mut wood_triangles) = (0usize, 0usize);
        for instance in &assembly.instances {
            let material = assembly.materials.get(instance.material as usize).ok_or("plant cards: material out of range")?;
            let transform = *branches.get(instance.branch as usize).ok_or("plant cards: branch out of range")? * instance.rest;
            let normals = Mat3::from_mat4(transform).inverse().transpose();
            if let Some(projections) = cards.get(&(instance.part, instance.material)) {
                let (vertices, indices) = &mut merged[card_index(instance.material) as usize];
                for card in projections {
                    let base = vertices.len() as u32;
                    let normal = (normals * card.plane.normal()).normalize_or_zero();
                    for (corner, (cx, cy, u, v)) in [(card.lo.x, card.lo.y, card.uv0.x, card.uv1.y), (card.hi.x, card.lo.y, card.uv1.x, card.uv1.y),
                        (card.hi.x, card.hi.y, card.uv1.x, card.uv0.y), (card.lo.x, card.hi.y, card.uv0.x, card.uv0.y)].into_iter().enumerate() {
                        let local = card.plane.x * cx + card.plane.y * cy + card.plane.normal() * card.depth;
                        let normal = card.source_normals.map_or(normal, |n| (normals * n[corner]).normalize_or(normal));
                        vertices.push(Vertex { pos: transform.transform_point3(local).to_array(), normal: normal.to_array(), uv: [u, v] });
                    }
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                    quads += 1;
                }
                continue;
            }
            // Wood: the part DAG's cut at the error, welded per placement.
            let g = assembly.parts.get(instance.part as usize).ok_or("plant cards: part out of range")?;
            let mesh = woods.entry(instance.part).or_insert_with(|| part_mesh(&g.vertices, &g.indices));
            let scale = transform.x_axis.truncate().length().max(transform.y_axis.truncate().length()).max(transform.z_axis.truncate().length());
            let (vertices, indices) = &mut merged[instance.material as usize];
            let mut remap: HashMap<u32, u32> = HashMap::new();
            for c in &mesh.clusters {
                let parent = if c.parent_error.is_finite() { c.parent_error * scale } else { f32::INFINITY };
                if c.own_error * scale > wood_error_m || parent <= wood_error_m { continue; }
                for t in c.first_tri..c.first_tri + c.tri_count {
                    let tri = mesh.tris[t as usize];
                    for corner in 0..3 {
                        let source = mesh.cluster_verts[(c.first_vert + ((tri >> (corner * 8)) & 255)) as usize];
                        let index = *remap.entry(source).or_insert_with(|| {
                            let v = mesh.vertices[source as usize];
                            vertices.push(Vertex { pos: transform.transform_point3(Vec3::from(v.pos)).to_array(),
                                normal: (normals * Vec3::from(v.normal)).normalize_or_zero().to_array(), uv: v.uv });
                            vertices.len() as u32 - 1
                        });
                        indices.push(index);
                    }
                    wood_triangles += 1;
                }
            }
            let _ = material;
        }
        let parts: Vec<Part> = merged.iter().enumerate().filter(|(_, (_, i))| !i.is_empty())
            .map(|(m, (v, i))| Part { vertices: v, indices: i, material: m as u32 }).collect();
        if parts.is_empty() { return Err("plant cards: no geometry".into()); }
        let n_wood = wood.len() as u32;
        let mesh = ClusterMesh::build_with_policy(&parts, |p| {
            if p.material >= n_wood { SimplificationPolicy::AlphaCards } else { SimplificationPolicy::TexturedGeometry }
        });
        let cards = card_materials.iter().map(|&m| assembly.materials[m as usize]).collect();
        Ok(Self { mesh, wood, cards, atlas, quads, wood_triangles })
    }

    /// The cards `what` describes (recipe, seed, error: Debug text will do)
    /// from `dir`, or from `cook`, saving them there; and whether they came
    /// from disk. The key also hashes this file, so a changed card cook
    /// never loads old cards; a damaged file just cooks again.
    pub fn cached(dir: &std::path::Path, what: &str, cook: impl FnOnce() -> Result<Self, String>) -> Result<(Self, bool), String> {
        let code = include_str!("plant_mesh_lod.rs").bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
        let key = crate::vegetation_asset::cook_key(&format!("plant cards {code:016x} {what}"));
        let path = dir.join(format!("{key:016x}.cards"));
        let load = || -> Option<Self> {
            let bytes = std::fs::read(&path).ok()?;
            let body = bytes.strip_prefix(b"WARPCARD".as_slice())?.strip_prefix(key.to_le_bytes().as_slice())?;
            postcard::from_bytes(body).ok()
        };
        if let Some(cards) = load() { return Ok((cards, true)); }
        let cards = cook()?;
        if let Ok(body) = postcard::to_allocvec(&cards) {
            let _ = std::fs::create_dir_all(dir);
            let partial = path.with_extension(format!("partial{}", std::process::id()));
            let bytes = [b"WARPCARD".as_slice(), &key.to_le_bytes(), &body].concat();
            if std::fs::write(&partial, bytes).is_ok() { let _ = std::fs::rename(&partial, &path); }
        }
        Ok((cards, false))
    }

    /// The materials the mesh's clusters index, cards sampling `layer`.
    pub fn materials(&self, layer: i32) -> Vec<Material> {
        let mut all = self.wood.clone();
        all.extend(self.cards.iter().map(|m| Material {
            layer, flags: ALPHA_TEST | TWO_SIDED | (m.flags & THIN_FOLIAGE), ..*m
        }));
        all
    }

    /// The mesh with cluster materials turned into the scene's: `scene[m]`
    /// is where material `m` of `materials()` went (Scene::add_material).
    pub fn mesh_with_materials(&self, scene: &[u32]) -> ClusterMesh {
        let mut mesh = ClusterMesh { vertices: self.mesh.vertices.clone(), cluster_verts: self.mesh.cluster_verts.clone(),
            tris: self.mesh.tris.clone(), clusters: self.mesh.clusters.clone(), sphere: self.mesh.sphere, levels: self.mesh.levels };
        for c in &mut mesh.clusters { c.material = scene[c.material as usize]; }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plant_recipe::{self, BuildOptions, CrownProfile, FoliageKind, WoodyRecipe};

    /// terrain_fly's spruce (engine examples copy game recipes, as does this).
    const SPRUCE: WoodyRecipe = WoodyRecipe {
        id: 0x5350_5255_4345, height: 16.0, trunk_radius: 0.22, crown_base: 0.18, crown_radius: 2.9, crown_profile: CrownProfile::Conical,
        tiers: 10, branches_per_tier: 3, branch_rise: 0.04, branch_radius: 0.065, stiffness: 0.72, foliage: FoliageKind::Needle,
        sprays_per_branch: 84, spray_scale: 1.15, spray_axial_bias: 0.40, foliage_colour: [0.065, 0.135, 0.045], wood_colour: [0.16, 0.105, 0.062],
        protected_trunk_height: 5.5,
    };

    #[test]
    fn spruce_card_retains_source_normal_variation_and_shaded_inner_needles() {
        use crate::plant_growth_assembly::Parts;
        let look = crate::plant_growth_assembly::Look::norway_spruce();
        let brush = &Parts::for_look(&look).tufts[0];
        let plane = planes(&brush.vertices, &brush.indices, true)[0];
        let mut atlas = image::RgbaImage::new(128, 128);
        let card = bake_card(&brush.vertices, &brush.indices, &mut atlas, (0, 0, 128), plane);
        let normals = card.source_normals.unwrap();
        assert!(normals.iter().all(|n| n.is_normalized()));
        // Give the same spruce two known lighting directions, one on each side
        // of the shoot. Each corner should favour needles on its own side.
        let mut divided = brush.vertices.clone();
        let split = divided.iter().map(|v| plane.project(Vec3::from(v.pos)).x).sum::<f32>()
            / divided.len() as f32;
        for vertex in &mut divided {
            let left = plane.project(Vec3::from(vertex.pos)).x < split;
            vertex.normal = if left { Vec3::X } else { Vec3::Y }.to_array();
        }
        let divided_card = bake_card(&divided, &brush.indices, &mut atlas, (0, 0, 128), plane);
        let [bottom_left, bottom_right, top_right, top_left] = divided_card.source_normals.unwrap();
        assert!(bottom_left.x > bottom_right.x && top_left.x > top_right.x);
        assert!(bottom_right.y > bottom_left.y && top_right.y > top_left.y);
        assert!(atlas.pixels().any(|p| p[3] >= 128 && p[0] < 195));
        assert!(atlas.pixels().any(|p| p[3] >= 128 && p[0] > 245));
    }

    #[test]
    fn fitted_planes_follow_the_surface_after_rotation_and_translation() {
        let rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, 0.8, -0.4, 0.7);
        let points = [[-0.4, 0.0, 0.0], [0.4, 0.0, 0.0], [0.3, 1.0, 0.03], [-0.3, 1.0, 0.0]];
        let vertices: Vec<_> = points.into_iter().map(|p| Vertex {
            pos: (rotation * Vec3::from(p) + Vec3::new(2.0, -1.0, 0.5)).to_array(),
            normal: [0.0; 3], uv: [0.0; 2],
        }).collect();
        let layout = planes(&vertices, &[0, 1, 2, 0, 2, 3], true);
        assert_eq!(layout.len(), 1, "a cupped leaf should stay cheap");
        assert!(layout[0].normal().dot(rotation * Vec3::Z).abs() > 0.99);
    }

    #[test]
    fn a_volumetric_spray_has_two_orthogonal_silhouettes() {
        let rotation = glam::Quat::from_rotation_z(0.68);
        let points = [[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0], [0.0, 2.0, 0.0],
                      [0.0, 0.0, -0.7], [0.0, 0.0, 0.7], [0.0, 2.0, 0.0]];
        let vertices: Vec<_> = points.into_iter().map(|p| Vertex {
            pos: (rotation * Vec3::from(p)).to_array(), normal: [0.0; 3], uv: [0.0; 2],
        }).collect();
        let indices = [0, 1, 2, 3, 4, 5];
        let layout = planes(&vertices, &indices, true);
        assert_eq!(layout.len(), 2);
        assert!(layout[0].normal().dot(layout[1].normal()).abs() < 1e-5);
        assert!(layout[0].y.dot(rotation * Vec3::Y).abs() > 0.99);
        assert_eq!(planes(&vertices, &indices, false).len(), 1, "a small woody attachment stays one card");
        assert_eq!(planes(&vertices, &[0, 0, 0], true).len(), 1);
    }

    #[test]
    fn a_spruce_becomes_a_few_thousand_cards() {
        let assembly = plant_recipe::build(&SPRUCE, BuildOptions::new(1), plant_recipe::recipe_limits()).unwrap();
        let started = web_time::Instant::now();
        let cards = PlantCards::cook(&assembly, 0.03).unwrap();
        let materials = cards.materials(3);
        assert!(cards.cards.iter().any(|m| m.flags & THIN_FOLIAGE != 0));
        assert!(cards.cards.iter().any(|m| m.flags & THIN_FOLIAGE == 0));
        for (source, card) in cards.cards.iter().zip(&materials[cards.wood.len()..]) {
            assert_eq!(card.flags, ALPHA_TEST | TWO_SIDED | (source.flags & THIN_FOLIAGE));
        }
        let roots: usize = cards.mesh.clusters.iter().filter(|c| !c.parent_error.is_finite()).map(|c| c.tri_count as usize).sum();
        eprintln!("PLANTCARDS {} quads, {} wood tris -> {} leaf tris, {} levels, coarsest {} tris, {:.1} s",
            cards.quads, cards.wood_triangles, cards.mesh.leaf_triangles(), cards.mesh.levels, roots, started.elapsed().as_secs_f64());
        if let Ok(path) = std::env::var("PLANT_CARDS_ATLAS") { cards.atlas.save(path).unwrap(); }
        assert!(cards.quads > 1000 && cards.mesh.leaf_triangles() < 60_000, "{} quads, {} tris", cards.quads, cards.mesh.leaf_triangles());
        assert!(cards.atlas.pixels().any(|p| p[3] > 128), "cards baked");
    }
}
