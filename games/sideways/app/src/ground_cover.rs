//! The first ground-cover port: photographed CC0 tufts rooted in the terrain.
//! Positions, size and rejection are keyed by world cell, never camera angle.
//! Frozen shots use this scatter; live_ground_cover shares its assets and
//! streams candidates into a fixed GPU instance pool for moving playtests.

use glam::{Affine3A, Quat, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{ALPHA_TEST, GRASS, Material, Scene, TWO_SIDED};
use sideways::builder::{Chunk, N};
use sideways::stage::{CHUNK, GRID};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

const CELL: f32 = 0.45;
pub(crate) const HEIGHT: f32 = 0.50;
pub(crate) const WIDTH: f32 = 0.36 * (257.0 / 135.0);
pub const MAX_TUFTS: usize = 60_000;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tuft {
    id: (i32, i32),
    pos: Vec3,
    scale: f32,
    yaw: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct Ground {
    pub(crate) height: f32,
    road_d: f32,
    pub(crate) normal: Vec3,
}

pub(crate) struct Field<'a> {
    chunks: HashMap<(i32, i32), &'a Chunk>,
}

impl<'a> Field<'a> {
    pub(crate) fn new(chunks: &'a [Chunk]) -> Self {
        Self { chunks: chunks.iter().map(|c| (c.cell, c)).collect() }
    }

    pub(crate) fn sample(&self, p: Vec2) -> Option<Ground> {
        let cell = ((p.x / CHUNK as f32).floor() as i32, (p.y / CHUNK as f32).floor() as i32);
        let c = self.chunks.get(&cell)?;
        let q = (p - Vec2::new(c.x0 as f32, c.z0 as f32)) / GRID as f32;
        let (i, j) = (q.x.floor() as usize, q.y.floor() as usize);
        if i >= N - 1 || j >= N - 1 { return None; }
        let (x, z) = (q.x - i as f32, q.y - j as f32);
        let a = j * N + i;
        // Same diagonal and barycentrics as chunk_mesh(), not a bilinear
        // surface floating above or below its two actual rendered triangles.
        let (indices, weights) = if x + z <= 1.0 {
            ([a, a + 1, a + N], [1.0 - x - z, x, z])
        } else {
            ([a + N + 1, a + N, a + 1], [x + z - 1.0, 1.0 - x, 1.0 - z])
        };
        let height = (0..3).map(|k| c.heights[indices[k]] * weights[k]).sum();
        let road_d = (0..3).map(|k| c.road_d[indices[k]] * weights[k]).sum();
        let dx = if x + z <= 1.0 { c.heights[a + 1] - c.heights[a] } else { c.heights[a + N + 1] - c.heights[a + N] } / GRID as f32;
        let dz = if x + z <= 1.0 { c.heights[a + N] - c.heights[a] } else { c.heights[a + N + 1] - c.heights[a + 1] } / GRID as f32;
        let up = 1.0 / (1.0 + dx * dx + dz * dz).sqrt();
        Some(Ground { height, road_d, normal: Vec3::new(-dx * up, up, -dz * up) })
    }
}

// An integer avalanche avoids platform-sensitive sin/fract hashes at large
// coordinates. It is visual-only and never consumes the simulation RNG.
fn hash(id: (i32, i32), salt: u32) -> u32 {
    let mut x = (id.0 as u32).wrapping_mul(0x9e3779b1) ^ (id.1 as u32).wrapping_mul(0x85ebca77) ^ salt.wrapping_mul(0xc2b2ae3d);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}

fn random(id: (i32, i32), salt: u32) -> f32 { (hash(id, salt) >> 8) as f32 / 16_777_216.0 }
fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn noise(p: Vec2) -> f32 {
    let id = (p.x.floor() as i32, p.y.floor() as i32);
    let x = smooth(0.0, 1.0, p.x - p.x.floor());
    let z = smooth(0.0, 1.0, p.y - p.y.floor());
    let a = random(id, 103);
    let b = random((id.0 + 1, id.1), 103);
    let c = random((id.0, id.1 + 1), 103);
    let d = random((id.0 + 1, id.1 + 1), 103);
    (a + (b - a) * x) * (1.0 - z) + (c + (d - c) * x) * z
}

fn shape(id: (i32, i32)) -> Tuft {
    let jitter = Vec2::new(random(id, 0) - 0.5, random(id, 17) - 0.5) * 1.6;
    let xz = (Vec2::new(id.0 as f32, id.1 as f32) + Vec2::splat(0.5) + jitter) * CELL;
    Tuft { id, pos: Vec3::new(xz.x, 0.0, xz.y), scale: 0.7 + 0.6 * random(id, 9), yaw: random(id, 3) * std::f32::consts::TAU }
}

fn radius(views: usize) -> f32 { if views <= 1 { 80.0 } else if views == 2 { 65.0 } else { 52.0 } }

fn scatter(field: &Field<'_>, cameras: &[Vec3]) -> (Vec<Tuft>, usize) {
    let views = cameras.len().clamp(1, 4);
    let radius = radius(views);
    let density = match views { 1 => 1.0, 2 => 0.68, _ => 0.42 };
    let mut cells = BTreeSet::new();
    for cam in cameras.iter().take(4) {
        // One common grid in every view: overlapping cameras share a tuft.
        for z in ((cam.z - radius) / CELL).floor() as i32 - 1..=((cam.z + radius) / CELL).ceil() as i32 + 1 {
            for x in ((cam.x - radius) / CELL).floor() as i32 - 1..=((cam.x + radius) / CELL).ceil() as i32 + 1 {
                cells.insert((x, z));
            }
        }
    }
    let mut tufts = Vec::new();
    for id in cells {
        let mut tuft = shape(id);
        let xz = Vec2::new(tuft.pos.x, tuft.pos.z);
        let d = cameras.iter().take(4).map(|c| xz.distance(Vec2::new(c.x, c.z))).fold(f32::INFINITY, f32::min);
        if d >= radius { continue; }
        let Some(g) = field.sample(xz) else { continue; };
        // Include the photographed card's footprint and the road's feathered
        // shoulder. Check its surrounding footprint as well, so a centre on
        // the verge cannot leave blades sticking through the racing surface.
        let clearance = WIDTH * 0.5 * tuft.scale;
        if g.road_d < 0.9 + clearance || g.normal.y <= 0.7 { continue; }
        let safe = [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y].iter().all(|axis| field.sample(xz + *axis * clearance).is_some_and(|s| s.road_d >= 0.9));
        if !safe { continue; }
        let verge = smooth(0.5, 1.4, g.road_d) * (1.0 - smooth(10.0, 22.0, g.road_d));
        let wood = smooth(0.3, 0.6, noise(xz / 7.0)) * 0.85 * smooth(1.0, 3.0, g.road_d);
        // A single broad falloff has no concentric mesh/scale switch lines.
        // Whole tufts are selected by a fixed world hash, never pixel dither.
        let range = (1.0 - smooth(12.0, radius, d)).powi(2);
        let coverage = verge.max(wood) * smooth(0.7, 0.82, g.normal.y) * range * density;
        if random(id, 43) >= coverage { continue; }
        tuft.pos.y = g.height - 0.025;
        tufts.push(tuft);
    }
    let requested = tufts.len();
    if tufts.len() > MAX_TUFTS {
        // Bounded even for four completely separated, fully fertile views.
        // Stable priority avoids truncation along a visible world-grid line.
        tufts.sort_by_key(|t| (hash(t.id, 71), t.id));
        tufts.truncate(MAX_TUFTS);
        tufts.sort_by_key(|t| t.id);
    }
    (tufts, requested)
}

/// Shared authored clump for near rendering and offline derived volumes.
/// Its UP vertex normals are legacy lighting proxies, not blade normals.
/// The current renderer applies its common statistical leaf BRDF instead.
pub(crate) fn source_geometry() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for card in 0..3 {
        // The live grass renderer uses this same fixed template for its
        // opaque/opacity handoff: the first card lies in local Z=0, and the
        // remaining two cards keep their authored angle and physical size.
        let angle = std::f32::consts::PI * card as f32 / 3.0;
        let side = Vec3::new(angle.cos(), 0.0, angle.sin()) * WIDTH * 0.5;
        let base = vertices.len() as u32;
        for (p, uv) in [(-side, [0.0, 1.0]), (side, [1.0, 1.0]), (-side + Vec3::Y * HEIGHT, [0.0, 0.0]), (side + Vec3::Y * HEIGHT, [1.0, 0.0])] {
            vertices.push(Vertex { pos: p.to_array(), normal: Vec3::Y.to_array(), uv });
        }
        indices.extend([0, 2, 1, 1, 2, 3].map(|i| base + i));
    }
    (vertices, indices)
}

pub(crate) fn mesh(material: u32) -> ClusterMesh {
    let (vertices, indices) = source_geometry();
    ClusterMesh::build(&[Part { vertices: &vertices, indices: &indices, material }])
}

pub(crate) fn photo(root: &Path) -> Result<image::RgbaImage, String> {
    let color_path = root.join("assets/grass/grass_medium_01_diff_2k.jpg");
    let alpha_path = root.join("assets/grass/grass_medium_01_alpha_2k.png");
    let color = image::open(&color_path).map_err(|e| format!("{}: {e}", color_path.display()))?.to_rgba8();
    let alpha = image::open(&alpha_path).map_err(|e| format!("{}: {e}", alpha_path.display()))?.to_luma8();
    if color.dimensions() != alpha.dimensions() || color.width() != 2048 || color.height() != 2048 {
        return Err("grass photo expects matching original 2048x2048 diffuse/alpha atlases".into());
    }
    // The same clump as grass_field.gd, cropped before mip generation so
    // alpha coverage belongs to this clump, not the other atlas plants.
    // Original CC0 files and credits remain unchanged; no new asset license.
    Ok(image::RgbaImage::from_fn(514, 270, |x, y| {
        let mut p = *color.get_pixel(x + 1160, y + 1778);
        p[3] = alpha.get_pixel(x + 1160, y + 1778)[0];
        p
    }))
}

pub fn add(scene: &mut Scene, chunks: &[Chunk], cameras: &[Vec3], root: &Path, mesh_names: &mut Vec<String>) -> Result<String, String> {
    add_with_vegetation(scene, chunks, cameras, root, mesh_names, Default::default())
}

pub fn add_with_vegetation(scene: &mut Scene, chunks: &[Chunk], cameras: &[Vec3], root: &Path, mesh_names: &mut Vec<String>, vegetation: sideways::vegetation::Vegetation) -> Result<String, String> {
    add_with_exclusion(scene,chunks,cameras,root,mesh_names,vegetation,None)
}

pub(crate) fn add_with_exclusion(scene: &mut Scene, chunks: &[Chunk], cameras: &[Vec3], root: &Path, mesh_names: &mut Vec<String>, vegetation: sideways::vegetation::Vegetation, excluded: Option<crate::live_ground_cover::CellRect>) -> Result<String, String> {
    let t = std::time::Instant::now();
    let (mut tufts, requested) = scatter(&Field::new(chunks), cameras);
    if let Some(rect)=excluded {tufts.retain(|tuft|!rect.contains(tuft.id));}
    let layer = scene.add_image("Poly Haven grass_medium_01 selected CC0 clump", photo(root)?);
    let material = scene.add_material(Material { tint: vegetation.grass_tint(), layer, roughness: 0.85, flags: ALPHA_TEST | TWO_SIDED | GRASS, scale: 1.0, metal: 0.0 });
    let id = scene.add_mesh(&mesh(material));
    mesh_names.push("photographed grass tuft (CC0)".into());
    for tuft in &tufts {
        scene.add_classified_instance(id, Affine3A::from_scale_rotation_translation(Vec3::splat(tuft.scale * vegetation.grass_scale()), Quat::from_rotation_y(tuft.yaw), tuft.pos), render::scene::GeometryCategory::Grass);
        scene.mark_no_shadow();
        // Frozen shot grass sways too, without opting into live streaming's
        // distance-opacity policy or range descriptors.
        scene.instances.last_mut().unwrap().flags |= render::scene::WIND_GRASS;
    }
    Ok(format!("ground cover: {} tufts / {} requested, budget {}, {} views, radius {:.0} m, {:.0} ms", tufts.len(), requested, MAX_TUFTS, cameras.len(), radius(cameras.len()), t.elapsed().as_secs_f64() * 1e3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(cell: (i32, i32), road_d: f32) -> Chunk {
        Chunk { cell, x0: cell.0 as f64 * CHUNK, z0: cell.1 as f64 * CHUNK, heights: vec![0.0; N * N], road_d: vec![road_d; N * N], trees: vec![], rocks: vec![], grass: vec![], ferns: vec![], stumps: vec![] }
    }

    #[test]
    fn ground_cover_roots_match_both_rendered_triangles_and_missing_chunks() {
        let mut c = chunk((0, 0), 5.0);
        c.heights[1] = 2.0;
        c.heights[N] = 4.0;
        c.heights[N + 1] = 10.0;
        let chunks = [c];
        let f = Field::new(&chunks);
        let first = f.sample(Vec2::new(0.5, 0.5)).unwrap();
        let second = f.sample(Vec2::new(1.5, 1.5)).unwrap();
        assert!((first.height - 1.5).abs() < 1e-6);
        assert!((second.height - 6.5).abs() < 1e-6);
        assert!(first.normal.abs_diff_eq(Vec3::new(-1.0, 1.0, -2.0).normalize(), 1e-6));
        assert!(second.normal.abs_diff_eq(Vec3::new(-3.0, 1.0, -4.0).normalize(), 1e-6));
        assert!(f.sample(Vec2::new(-0.1, 1.0)).is_none());
    }

    #[test]
    fn ground_cover_is_deterministic_keeps_scale_and_rejects_roads() {
        let chunks = [chunk((0, 0), 5.0)];
        let f = Field::new(&chunks);
        let a = scatter(&f, &[Vec3::new(25.0, 2.0, 25.0)]).0;
        let same = scatter(&f, &[Vec3::new(25.0, 2.0, 25.0)]).0;
        let moved = scatter(&f, &[Vec3::new(25.2, 2.0, 25.0)]).0;
        assert!(!a.is_empty());
        assert_eq!(a, same);
        let moved: HashMap<_, _> = moved.iter().map(|t| (t.id, *t)).collect();
        for tuft in &a { if let Some(other) = moved.get(&tuft.id) { assert_eq!(tuft, other); } }
        let road = [chunk((0, 0), 0.3)];
        assert!(scatter(&Field::new(&road), &[Vec3::new(25.0, 2.0, 25.0)]).0.is_empty());
    }

    #[test]
    fn ground_cover_four_views_share_tufts_and_respect_global_budget() {
        let chunks: Vec<_> = (-2..=3).flat_map(|z| (-2..=3).map(move |x| chunk((x, z), 5.0))).collect();
        let f = Field::new(&chunks);
        let cameras = [Vec3::ZERO, Vec3::new(95.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 95.0), Vec3::new(95.0, 0.0, 95.0)];
        let (tufts, _) = scatter(&f, &cameras);
        assert!(!tufts.is_empty() && tufts.len() <= MAX_TUFTS);
        let ids: BTreeSet<_> = tufts.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), tufts.len());
        let mut reverse = cameras;
        reverse.reverse();
        assert_eq!(tufts, scatter(&f, &reverse).0);
        let repeated = [Vec3::ZERO; 4];
        let (shared, _) = scatter(&f, &repeated);
        assert!(shared.len() < tufts.len());
    }
}
