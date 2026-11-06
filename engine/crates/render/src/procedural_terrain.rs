//! Procedural terrain: no heightmap, no voxels, no stored mesh. The ground
//! is a function (`terrain_field`), and every frame the cameras pick a
//! quadtree of patches (CDLOD) whose cells project to a few pixels. Each
//! patch is one 128-triangle cluster in a reserved pool; a compute pass
//! samples the function into a patch when it first appears, then the
//! normal cluster pipeline culls, rasters, shades and shadows it.
//!
//! ```ignore
//! let pool = procedural_terrain::reserve(&mut scene, 16384, terrain_material, seed, Style::default());
//! let renderer = Renderer::new(&gpu, &scene, &light);
//! let mut terrain = ProceduralTerrain::new(pool, Settings::default());
//! terrain.update(&gpu, &renderer, &[TerrainView::new(&camera, height_px)])?; // per frame
//! ```
use crate::cluster::{GpuCluster, Vertex};
use crate::scene::{GeometryCategory, GpuInstance, Material, Scene, INSTANCE_CATEGORY_SHIFT};
use crate::{forest_field, terrain_field, vegetation_style, world_constraints, Camera, Gpu, Renderer};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;
use std::sync::Mutex;

/// Cells per patch side. 8x8 cells = 128 triangles = one cluster.
pub const CELLS: i32 = 8;
const VERTS: u32 = 81;
/// Instance flag: geometry rewritten at runtime (a dynamic shadow caster).
pub const PROCEDURAL: u32 = 128;

const SHADER: &str = include_str!("shaders/procedural_terrain.wgsl");

/// The reserved pool, made by [`reserve`] before the renderer.
#[derive(Clone, Debug)]
pub struct TerrainSpec {
    pub first_instance: u32,
    pub first_cluster: u32,
    pub first_vertex: u32,
    pub capacity: u32,
    pub seed: u32,
    pub style: terrain_field::Style,
    /// Roads and pads the land bends to (none: the bare function).
    pub constraints: Option<std::sync::Arc<world_constraints::World>>,
}

impl TerrainSpec {
    /// The ground at (x, z) for this terrain, constraints included.
    pub fn eval(&self, x: f32, z: f32, cut: f32) -> terrain_field::Sample {
        match &self.constraints {
            Some(w) => w.eval(x, z, cut, self.seed, &self.style),
            None => terrain_field::eval(x, z, cut, self.seed, &self.style),
        }
    }

    /// Ground height under world (x, z) at full detail, for physics.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        match &self.constraints {
            Some(w) => w.height(x, z, self.seed, &self.style),
            None => terrain_field::height(x, z, self.seed, &self.style),
        }
    }
}

/// Bend the scene's procedural terrain to `world`'s roads and pads. Call
/// before the renderer is made, with the spec `reserve` returned.
pub fn constrain(scene: &mut Scene, world: std::sync::Arc<world_constraints::World>) -> TerrainSpec {
    let spec = scene.procedural_terrain.as_mut().expect("reserve the procedural terrain first");
    spec.constraints = Some(world);
    spec.clone()
}

/// Sun visibility baked for `size` metres square around `centre` at `cell`
/// spacing, for terrain shadows past the shadow maps: returns the grid for
/// `Scene::heightfield` and the visibility for `Scene::terrain_sun`. The
/// grid's heights are the "unknown" sentinel: the coarse samples would lie
/// above the finer mesh in places, and the heightfield's ground tests would
/// then black out the ground under them.
/// `constraints` bend the land and give it its regions, as the mesh does.
pub fn sun_heightfield(seed: u32, style: &terrain_field::Style, constraints: Option<&world_constraints::World>, centre: [f32; 2], size: f32, cell: f32, sun: Vec3)
    -> (crate::scene::Heightfield, Vec<f32>) {
    let n = (size / cell).ceil() as usize + 1;
    let origin = [(centre[0] / cell).floor() * cell - (n / 2) as f32 * cell, (centre[1] / cell).floor() * cell - (n / 2) as f32 * cell];
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
    let rows = n.div_ceil(threads);
    let mut raw = vec![0.0f32; n * n];
    crate::par::scope(|s| {
        for (k, chunk) in raw.chunks_mut(rows * n).enumerate() {
            s.spawn(move || {
                for (i, h) in chunk.iter_mut().enumerate() {
                    let (x, z) = ((i % n) as f32, (k * rows + i / n) as f32);
                    let (u, v) = (origin[0] + x * cell, origin[1] + z * cell);
                    *h = match constraints {
                        Some(w) => w.eval(u, v, cell, seed, style).pos.y,
                        None => terrain_field::eval(u, v, cell, seed, style).pos.y,
                    };
                }
            });
        }
    });
    let sun_vis = bake_sun(&raw, n, cell, sun);
    (crate::scene::Heightfield { origin, cell, w: n as u32, h: n as u32, heights: vec![-1e9; n * n] }, sun_vis)
}

/// Sun visibility per sample of an n x n height grid: march toward the sun
/// and compare the steepest rise on the way with the sun's elevation, with
/// a few degrees of soft edge so shadows past the shadow maps are not
/// stair-stepped by the grid.
fn bake_sun(raw: &[f32], n: usize, cell: f32, sun: Vec3) -> Vec<f32> {
    let flat = (sun.x * sun.x + sun.z * sun.z).sqrt();
    if sun.y <= 0.0 { return vec![0.0; raw.len()]; }
    if flat < 1e-4 { return vec![1.0; raw.len()]; }
    let (dx, dz, rise) = (sun.x / flat, sun.z / flat, sun.y / flat);
    let top = raw.iter().copied().fold(f32::MIN, f32::max);
    let soft = 0.05f32;
    let sample = |x: f32, z: f32| -> Option<f32> {
        if x < 0.0 || z < 0.0 || x >= (n - 1) as f32 || z >= (n - 1) as f32 { return None; }
        let (ix, iz) = (x as usize, z as usize);
        let (fx, fz) = (x - ix as f32, z - iz as f32);
        let r = |i: usize, j: usize| raw[j * n + i];
        Some((r(ix, iz) * (1.0 - fx) + r(ix + 1, iz) * fx) * (1.0 - fz) + (r(ix, iz + 1) * (1.0 - fx) + r(ix + 1, iz + 1) * fx) * fz)
    };
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
    let rows = n.div_ceil(threads);
    let mut vis = vec![1.0f32; n * n];
    crate::par::scope(|s| {
        for (k, chunk) in vis.chunks_mut(rows * n).enumerate() {
            s.spawn(move || {
                for (i, v) in chunk.iter_mut().enumerate() {
                    let (x, z) = ((i % n) as f32, (k * rows + i / n) as f32);
                    let h0 = raw[(k * rows + i / n) * n + i % n];
                    let mut steepest = f32::MIN;
                    // Steps in grid cells; past `top` nothing can block.
                    let mut t = 1.5f32;
                    loop {
                        let Some(h) = sample(x + dx * t, z + dz * t) else { break };
                        steepest = steepest.max((h - h0) / (t * cell));
                        if steepest > rise + soft || h0 + t * cell * (rise - soft) > top { break; }
                        t += 1.0;
                    }
                    *v = 1.0 - smooth(rise - soft, rise + soft, steepest);
                }
            });
        }
    });
    vis
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Data materials the terrain material's scale points at, -(first + 1):
/// the ForestStyle (2), far forest start and density (1), the GroundStyle
/// (4), terrain shape params (1: tint.x the ditch amount);
/// terrain_surface.wgsl far_forest_params, field_ground and field_ditch
/// read them.
/// Created with the defaults (no far forest) on first use.
fn style_block(scene: &mut Scene, material: u32) -> usize {
    let scale = scene.materials[material as usize].scale;
    if scale < 0.0 { return (-scale - 1.0).round() as usize; }
    let first = scene.materials.len();
    assert!(first < 1 << 24, "terrain style material index out of range");
    let forest: [Material; 2] = bytemuck::cast(forest_field::ForestStyle::default());
    let ground: [Material; 4] = bytemuck::cast(vegetation_style::GroundStyle::default());
    scene.materials.extend(forest);
    scene.materials.push(Material { tint: [0.0; 3], layer: -1, roughness: 1.0, flags: 0, scale: 1.0, metal: 0.0 });
    scene.materials.extend(ground);
    scene.materials.push(Material { tint: [0.0; 3], layer: -1, roughness: 1.0, flags: 0, scale: 1.0, metal: 0.0 });
    scene.materials[material as usize].scale = -(first as f32 + 1.0);
    first
}

/// Draw the terrain in `style`'s palette: its ground tints, and its forest
/// for far_forest (call that with `style.forest`). The meadow's season is the
/// caller's, for its meadow cook.
pub fn vegetation_style(scene: &mut Scene, material: u32, style: &vegetation_style::VegetationStyle) {
    let first = style_block(scene, material);
    let forest: [Material; 2] = bytemuck::cast(style.forest);
    let ground: [Material; 4] = bytemuck::cast(style.ground);
    // The region palette pointer (region_ground) lives in the ground's
    // reserved word; a new palette keeps it.
    let regions = scene.materials[first + 6].roughness;
    scene.materials[first..first + 2].copy_from_slice(&forest);
    scene.materials[first + 3..first + 7].copy_from_slice(&ground);
    scene.materials[first + 6].roughness = regions;
}

/// Ground colours per region (crate::regions): region kind k is drawn in
/// `grounds[k]` (its biome's palette at the season), blended across
/// borders the way the terrain's shape is, per pixel
/// (terrain_surface.wgsl region_ground). Replaces the single palette of
/// `vegetation_style` wherever the regions' grid reaches (its edge holds
/// beyond). The data goes into materials after the style block; the
/// GroundStyle's reserved.x points at it (first + 1, 0 for none).
pub fn region_ground(scene: &mut Scene, material: u32, regions: &crate::regions::Regions, grounds: &[vegetation_style::GroundStyle]) {
    let first = style_block(scene, material);
    let words = regions.surface_words(bytemuck::cast_slice(grounds), 32);
    let at = scene.materials.len();
    assert!(at + words.len() / 8 < 1 << 24, "region ground material index out of range");
    for chunk in words.chunks(8) {
        let mut w = [0.0f32; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        scene.materials.push(Material { tint: [w[0], w[1], w[2]], layer: w[3].to_bits() as i32, roughness: w[4], flags: w[5].to_bits(), scale: w[6], metal: w[7] });
    }
    scene.materials[first + 6].roughness = at as f32 + 1.0;
}

/// Past `radius_m` from the camera, where the drawn grass cells end, colour
/// grass ground by `tint` (multiplying the ground): a near-free far proxy for
/// the blades (terrain_surface.wgsl grass_proxy). Radius 0 turns it off.
pub fn grass_proxy(scene: &mut Scene, material: u32, radius_m: f32, tint: [f32; 3]) {
    let first = style_block(scene, material);
    let m = &mut scene.materials[first + 7];
    m.tint[1] = radius_m.max(0.0);
    m.roughness = tint[0]; m.scale = tint[1]; m.metal = tint[2];
}

/// Draw the forest of `forest_field` into the terrain from `start_m` from
/// the camera on, where single trees end: its canopy from above in `style`,
/// no geometry. `density` as forest_field's (terrain_surface.wgsl far_forest).
pub fn far_forest(scene: &mut Scene, material: u32, start_m: f32, density: f32, style: &forest_field::ForestStyle) {
    let first = style_block(scene, material);
    let forest: [Material; 2] = bytemuck::cast(*style);
    scene.materials[first..first + 2].copy_from_slice(&forest);
    scene.materials[first + 2] = Material { tint: [(start_m - 150.0).max(0.0), density.clamp(0.0, 1.0), 0.0],
        layer: -1, roughness: 0.0, flags: 0, scale: start_m.max(0.0), metal: 0.0 };
}

/// far_forest for trees placed around `centre` (x, z) and handed over in the
/// band `start_m..end_m` (forest_field::handover_keep drops the objects the
/// layer takes over), so drawn and painted forest sum to one share at every
/// distance, also while the camera moves away from where the trees were
/// placed.
pub fn far_forest_handover(scene: &mut Scene, material: u32, centre: [f32; 2], start_m: f32, end_m: f32, density: f32,
    style: &forest_field::ForestStyle) {
    let first = style_block(scene, material);
    let forest: [Material; 2] = bytemuck::cast(*style);
    scene.materials[first..first + 2].copy_from_slice(&forest);
    scene.materials[first + 2] = Material { tint: [start_m.max(0.0), density.clamp(0.0, 1.0), centre[0]],
        layer: -1, roughness: centre[1], flags: 1, scale: end_m.max(start_m + 1e-3), metal: scene.materials[first + 2].metal };
}

/// Draw `world`'s roads into the terrain's pixels: gravel with two worn
/// wheel lines, a ragged edge and a shoulder of loose stone, sharp at any
/// mesh detail (terrain_surface.wgsl road_at). The roads go into data
/// materials after the style block; its far forest params' metal points
/// at them (first + 1, 0 for none).
pub fn road_surface(scene: &mut Scene, material: u32, world: &world_constraints::World) {
    let first = style_block(scene, material);
    let words = world.surface_words();
    if words.is_empty() { scene.materials[first + 2].metal = 0.0; return; }
    let at = scene.materials.len();
    assert!(at < 1 << 24, "road data material index out of range");
    for chunk in words.chunks(8) {
        let mut w = [0.0f32; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        scene.materials.push(Material { tint: [w[0], w[1], w[2]], layer: w[3].to_bits() as i32, roughness: w[4], flags: w[5].to_bits(), scale: w[6], metal: w[7] });
    }
    scene.materials[first + 2].metal = at as f32 + 1.0;
}

/// Reserve `capacity` patch slots that draw with `material`.
pub fn reserve(scene: &mut Scene, capacity: u32, material: u32, seed: u32, style: terrain_field::Style) -> TerrainSpec {
    assert!(scene.procedural_terrain.is_none(), "one procedural terrain per scene");
    // terrain_surface.wgsl reads the seed from the material's metalness
    // (terrain is never metal) for detail finer than the mesh.
    scene.materials[material as usize].metal = seed as f32;
    // Ditches are drawn into the pixels too (terrain_surface.wgsl field_ditch).
    if style.ditches > 0.0 {
        let first = style_block(scene, material);
        scene.materials[first + 7].tint[0] = style.ditches;
    }
    let first_vertex = scene.vertices.len() as u32;
    let first_tri = scene.tris.len() as u32;
    let first_cluster = scene.clusters.len() as u32;
    let first_instance = scene.instances.len() as u32;
    scene.vertices.resize(scene.vertices.len() + (capacity * VERTS) as usize, Vertex::zeroed());
    // One shared topology; diagonals alternate so slopes do not all lean one way.
    let at = |i: i32, j: i32| (j * (CELLS + 1) + i) as u32;
    for j in 0..CELLS {
        for i in 0..CELLS {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
            let pack = |x: u32, y: u32, z: u32| x | y << 8 | z << 16;
            if (i + j) & 1 == 0 {
                scene.tris.push(pack(a, c, b));
                scene.tris.push(pack(b, c, d));
            } else {
                scene.tris.push(pack(a, c, d));
                scene.tris.push(pack(a, d, b));
            }
        }
    }
    let hidden = [0.0, -1.0e7, 0.0, 0.0];
    for slot in 0..capacity {
        let first_vert = scene.cluster_verts.len() as u32;
        scene.cluster_verts.extend((0..VERTS).map(|k| first_vertex + slot * VERTS + k));
        scene.clusters.push(GpuCluster {
            cull: hidden, parent: hidden, own: hidden,
            parent_error: f32::MAX, own_error: 0.0,
            first_tri, tri_count: (CELLS * CELLS * 2) as u32, first_vert, material, depth: 0, _pad: 0,
        });
        scene.instances.push(GpuInstance {
            rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            sphere: hidden,
            first_cluster: first_cluster + slot,
            cluster_count: 0,
            scale: 1.0,
            flags: PROCEDURAL | (GeometryCategory::Terrain as u32) << INSTANCE_CATEGORY_SHIFT,
        });
    }
    let spec = TerrainSpec { first_instance, first_cluster, first_vertex, capacity, seed, style, constraints: None };
    scene.procedural_terrain = Some(spec.clone());
    spec
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Largest projected length of one cell, in pixels.
    pub cell_pixels: f32,
    /// Largest projected geometric error, in pixels.
    pub error_pixels: f32,
    /// Patches farther than this from every camera are not drawn (metres).
    pub view_distance: f32,
    /// Cell size of the finest patches (metres, a power of two).
    pub finest: f32,
    /// Level of the root patches (cell = finest * 2^level).
    pub root_level: u32,
    /// Cell target multiplier for patches outside every view.
    pub offscreen_factor: f32,
    /// Degrees added around the view cone before a patch counts as outside:
    /// a small turn does not refine what was just outside.
    pub cone_margin_deg: f32,
    /// New patches built per frame. A turn that wants more builds them
    /// hidden over the next frames and shows them all at once, so the frame
    /// never stalls and the ground never has holes.
    pub build_budget: usize,
    /// The selection reruns only after the camera moves this far (metres)
    /// or turns this much (degrees).
    pub reselect_m: f32,
    pub reselect_deg: f32,
    /// Seconds a patch takes to slide from the shape it replaced into its
    /// own after a change of detail (0: swap at once).
    pub morph_s: f32,
    /// Only patches with cells up to this long (metres) slide: farther
    /// ones change less than a pixel, and every sliding patch is rebuilt
    /// each frame.
    pub morph_max_cell: f32,
    /// At most this many patches slide at once (the finest first); the rest
    /// swap at once. A fast flight changes thousands a second.
    pub morph_max_patches: usize,
}

impl Default for Settings {
    // offscreen_factor 8 swung a 5K view between 4k and 12k patches as it
    // turned (6.9k new in one 30 degree step): mouse turns hitched.
    fn default() -> Self { Settings { cell_pixels: 24.0, error_pixels: 1.5, view_distance: 20000.0, finest: 1.0 / 16.0, root_level: 13,
        offscreen_factor: 2.5, cone_margin_deg: 15.0, build_budget: 1024, reselect_m: 0.5, reselect_deg: 1.0, morph_s: 0.35, morph_max_cell: 2.0, morph_max_patches: 768 } }
}

/// One camera as the LOD sees it.
#[derive(Clone, Copy, Debug)]
pub struct TerrainView {
    pub pos: Vec3,
    /// Pixels per radian at the screen centre: (height / 2) / tan(fov / 2).
    pub pixels_per_radian: f32,
    /// Unit view direction.
    pub forward: Vec3,
    /// Cosine of the half angle of a cone around `forward` that holds the
    /// whole view (its corners included).
    pub cos_half_cone: f32,
}

/// Pixel thresholds count pixels of a view at most this tall. Patches grow
/// with the square of the height, so a 5K Retina view wanted four times the
/// geometry of 1440p (and ran at a quarter of the frame rate) for detail
/// finer than its pixels show; shading stays at full resolution.
pub const LOD_HEIGHT_PX: u32 = 1440;

impl TerrainView {
    pub fn new(camera: &Camera, width_px: u32, height_px: u32) -> Self {
        let tan_y = (camera.fov_y * 0.5).tan();
        let tan_x = tan_y * width_px as f32 / height_px.max(1) as f32;
        let corner = (tan_x * tan_x + tan_y * tan_y).sqrt();
        TerrainView { pos: camera.pos, pixels_per_radian: height_px.min(LOD_HEIGHT_PX) as f32 * 0.5 / tan_y,
            forward: (camera.target - camera.pos).normalize_or(Vec3::NEG_Z), cos_half_cone: 1.0 / (1.0 + corner * corner).sqrt() }
    }

    /// Whether a sphere can be inside the view cone.
    #[cfg(test)]
    fn sees(&self, centre: Vec3, radius: f32) -> bool { self.sees_within(centre, radius, 0.0) }

    /// [`Self::sees`] with the cone widened by `margin` radians.
    fn sees_within(&self, centre: Vec3, radius: f32, margin: f32) -> bool {
        let to = centre - self.pos;
        let d = to.length();
        if d <= radius { return true; }
        // Cone/sphere test: angle to the centre minus the sphere's angular size.
        let cos_a = to.dot(self.forward) / d;
        let sin_r = (radius / d).min(1.0);
        let cos_r = (1.0 - sin_r * sin_r).sqrt();
        let cone = (self.cos_half_cone.clamp(-1.0, 1.0).acos() + margin).min(std::f32::consts::PI);
        let (sin_c, cos_c) = cone.sin_cos();
        // cos(cone + r) <= cos_a
        cos_a >= cos_c * cos_r - sin_c * sin_r
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Job {
    origin: [f32; 2],
    spacing: f32,
    mask: u32,
    slot: u32,
    /// Geomorph: bits 0..1 [`MORPH_FROM_COARSER`] or [`MORPH_FROM_FINER`],
    /// bits 2..5 the edges (west, east, north, south) that slide too.
    morph: u32,
    /// 0 the old shape, 1 the patch's own.
    morph_t: f32,
    _pad: u32,
}

/// Job.morph: the patch replaces a coarser one (a split).
const MORPH_FROM_COARSER: u32 = 1;
/// Job.morph: the patch replaces finer ones (a merge).
const MORPH_FROM_FINER: u32 = 2;

/// A shown patch sliding from the shape it replaced into its own
/// (procedural_terrain.wgsl build): rebuilt every frame until it is there.
#[derive(Clone, Copy, Debug)]
struct Morph {
    job: Job,
    started_s: f64,
    /// The same-level neighbour each sliding edge slides with: an edge stops
    /// sliding (takes its own shape) once that neighbour is gone, because
    /// the patch that replaced it does not slide along it.
    with: [Option<(Node, u32)>; 4],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub patches: usize,
    pub built: usize,
    pub freed: usize,
    /// Patches dropped because the pool was full.
    pub overflow: usize,
    /// Wanted patches still to build before the shown set changes.
    pub pending: usize,
    pub finest_level: u32,
    /// Pixel thresholds were multiplied by this to fit the pool (1 = as set).
    pub lod_scale: f32,
}

/// Quadtree keys are a few small integers, looked up millions of times per
/// second while the camera moves: the default SipHash was most of that cost.
#[derive(Default)]
struct NodeHash(u64);
impl Hasher for NodeHash {
    fn write(&mut self, bytes: &[u8]) { for &b in bytes { self.write_u32(u32::from(b)); } }
    fn write_u32(&mut self, n: u32) { self.0 = (self.0.rotate_left(5) ^ u64::from(n)).wrapping_mul(0xf135_7aea_2e62_a9c5); }
    fn write_i32(&mut self, n: i32) { self.write_u32(n as u32); }
    fn finish(&self) -> u64 { self.0.rotate_left(26) }
}
type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<NodeHash>>;
type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<NodeHash>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Node { level: u32, ix: i32, iz: i32 }

impl Node {
    fn parent(self) -> Node { Node { level: self.level + 1, ix: self.ix >> 1, iz: self.iz >> 1 } }
    fn children(self) -> [Node; 4] {
        let (l, x, z) = (self.level - 1, self.ix * 2, self.iz * 2);
        [Node { level: l, ix: x, iz: z }, Node { level: l, ix: x + 1, iz: z }, Node { level: l, ix: x, iz: z + 1 }, Node { level: l, ix: x + 1, iz: z + 1 }]
    }
}

/// The CPU half: patch selection and slot bookkeeping.
pub struct ProceduralTerrain {
    pub spec: TerrainSpec,
    pub settings: Settings,
    slots: HashMap<(Node, u32), u32>,
    free: Vec<u32>,
    /// Multiplies the pixel thresholds when the views want more patches than
    /// the pool holds; eases back towards 1 when there is room again.
    lod: f32,
    /// Centre height and geometric error per node: the field never changes,
    /// and evaluating it was most of the per-frame selection time.
    samples: std::cell::RefCell<HashMap<Node, (f32, f32)>>,
    /// Patches built but not shown yet: slot and the instance that shows it.
    hidden: HashMap<(Node, u32), (u32, GpuInstance)>,
    /// The last selection, its views, patch count and finest level.
    selection: Option<(Vec<TerrainView>, Arc<HashSet<(Node, u32)>>, usize, u32)>,
    /// The selection whose patches are all built and shown: while it stands,
    /// a frame has nothing to plan.
    settled: Option<Arc<HashSet<(Node, u32)>>>,
    /// What each slot's instance holds on the GPU.
    instances: Vec<GpuInstance>,
    /// Shown patches still sliding into their own shape, by slot.
    morphs: HashMap<u32, Morph>,
    clock: web_time::Instant,
}

/// One frame of terrain changes: instance writes and patch builds.
pub struct Plan {
    pub stats: Stats,
    changed: Vec<(u32, GpuInstance)>,
    jobs: Vec<Job>,
    /// Rebuilds of sliding patches (Settings::morph_s), outside the budget.
    morph_jobs: Vec<Job>,
}

/// Cached node samples before the cache starts over.
const SAMPLE_CACHE: usize = 1 << 20;

/// Share of the pool the selection may fill; the rest absorbs the patches
/// that balancing and the next frame's movement add.
const BUDGET: f32 = 0.9;

impl ProceduralTerrain {
    pub fn new(spec: TerrainSpec, settings: Settings) -> Self {
        let instances = (0..spec.capacity).map(|slot| hidden_instance(&spec, slot)).collect();
        ProceduralTerrain { free: (0..spec.capacity).rev().collect(), instances, spec, settings, slots: HashMap::default(), lod: 1.0, samples: Default::default(),
            hidden: HashMap::default(), selection: None, settled: None, morphs: HashMap::default(), clock: web_time::Instant::now() }
    }

    fn cell(&self, level: u32) -> f32 { self.settings.finest * (1u32 << level) as f32 }
    fn size(&self, level: u32) -> f32 { self.cell(level) * CELLS as f32 }

    /// A node's centre height and geometric error.
    fn sample(&self, n: Node) -> (f32, f32) {
        let mut cache = self.samples.borrow_mut();
        if let Some(&s) = cache.get(&n) { return s; }
        if cache.len() >= SAMPLE_CACHE { cache.clear(); }
        let (size, cell) = (self.size(n.level), self.cell(n.level));
        let (x, z) = ((n.ix as f32 + 0.5) * size, (n.iz as f32 + 0.5) * size);
        let e = self.spec.eval(x, z, cell * CELLS as f32);
        let mut error = e.error(cell);
        // A road's edge and banks are sharper than the land: refine there.
        if let Some(w) = &self.spec.constraints {
            if w.clearance(x, z) < size { error = error.max(0.2 * cell); }
        }
        let s = (e.pos.y, error);
        cache.insert(n, s);
        s
    }

    fn wants_split(&self, n: Node, views: &[TerrainView]) -> bool {
        if n.level == 0 { return false; }
        let size = self.size(n.level);
        let cell = self.cell(n.level);
        let (x0, z0) = (n.ix as f32 * size, n.iz as f32 * size);
        let (centre_y, error) = self.sample(n);
        let reach = size * 0.75 + 8.0;
        let centre = Vec3::new(x0 + size * 0.5, centre_y, z0 + size * 0.5);
        let radius = (size * size * 0.5 + reach * reach).sqrt();
        views.iter().any(|v| {
            let dx = (v.pos.x - centre.x).abs() - size * 0.5;
            let dz = (v.pos.z - centre.z).abs() - size * 0.5;
            let dy = (v.pos.y - centre_y).abs() - reach;
            let d = (dx.max(0.0).powi(2) + dz.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt().max(0.05);
            // Out of view, patches only cast shadows: much coarser is fine.
            let scale = self.lod * if v.sees_within(centre, radius, self.settings.cone_margin_deg.to_radians()) { 1.0 } else { self.settings.offscreen_factor };
            let px = v.pixels_per_radian / d;
            cell * px > self.settings.cell_pixels * scale || error * px > self.settings.error_pixels * scale
        })
    }

    /// Leaves of the patch quadtree for these views, balanced so that
    /// neighbours (diagonals too) differ by at most one level.
    fn select(&self, views: &[TerrainView]) -> HashSet<Node> {
        let root = self.settings.root_level;
        let rs = self.size(root);
        let vd = self.settings.view_distance;
        let mut roots = HashSet::default();
        for v in views {
            let (a, b) = (((v.pos.x - vd) / rs).floor() as i32, ((v.pos.x + vd) / rs).floor() as i32);
            let (c, d) = (((v.pos.z - vd) / rs).floor() as i32, ((v.pos.z + vd) / rs).floor() as i32);
            for ix in a..=b {
                for iz in c..=d {
                    let cx = ((ix as f32 + 0.5) * rs - v.pos.x).abs() - rs * 0.5;
                    let cz = ((iz as f32 + 0.5) * rs - v.pos.z).abs() - rs * 0.5;
                    if cx.max(0.0).hypot(cz.max(0.0)) <= vd { roots.insert(Node { level: root, ix, iz }); }
                }
            }
        }
        let mut leaves = HashSet::default();
        let mut stack: Vec<Node> = roots.into_iter().collect();
        while let Some(n) = stack.pop() {
            if self.wants_split(n, views) { stack.extend(n.children()); } else { leaves.insert(n); }
        }
        balance(&mut leaves, root);
        leaves
    }

    /// [`Self::select`], coarsened until it fits the pool. Patches fall
    /// roughly with the square of the thresholds, so each retry scales them
    /// by the square root of the excess. Between frames the scale relaxes
    /// slowly, so it does not flicker between two levels of detail.
    fn select_within_budget(&mut self, views: &[TerrainView]) -> HashSet<Node> {
        let budget = (self.spec.capacity as f32 * BUDGET) as usize;
        let mut leaves = self.select(views);
        for _ in 0..8 {
            if leaves.len() <= budget { break; }
            self.lod *= (leaves.len() as f32 / budget as f32).sqrt().max(1.05) * 1.05;
            leaves = self.select(views);
        }
        if leaves.len() < budget * 2 / 3 { self.lod = (self.lod * 0.98).max(1.0); }
        leaves
    }

    /// The wanted patches (with their stitch masks) for these views; the
    /// last selection while the views stay within the reselect thresholds.
    fn wanted(&mut self, views: &[TerrainView]) -> (Arc<HashSet<(Node, u32)>>, usize, u32) {
        let cos_turn = self.settings.reselect_deg.to_radians().cos();
        if let Some((last, keys, patches, finest)) = &self.selection {
            let same = last.len() == views.len() && last.iter().zip(views).all(|(a, b)| a.pos.distance(b.pos) <= self.settings.reselect_m
                && a.forward.dot(b.forward) >= cos_turn && a.pixels_per_radian == b.pixels_per_radian && a.cos_half_cone == b.cos_half_cone);
            if same { return (keys.clone(), *patches, *finest); }
        }
        let leaves = self.select_within_budget(views);
        let topology = TerrainTopology::new(&leaves, self.settings.root_level);
        let keys: Arc<HashSet<(Node, u32)>> = Arc::new(leaves.iter().map(|&n| (n, mask(&topology, n, self.settings.root_level))).collect());
        let finest = leaves.iter().map(|n| n.level).min().unwrap_or(u32::MAX);
        self.selection = Some((views.to_vec(), keys.clone(), leaves.len(), finest));
        (keys, leaves.len(), finest)
    }

    /// The instance that shows a built patch.
    fn shown_instance(&self, n: Node, slot: u32) -> GpuInstance {
        let size = self.size(n.level);
        let origin = [n.ix as f32 * size, n.iz as f32 * size];
        let centre_y = self.sample(n).0;
        let half = size * 0.5;
        let radius = (2.0 * half * half).sqrt() + size + 8.0 + terrain_field::MAX_PUSH;
        let mut inst = hidden_instance(&self.spec, slot);
        inst.sphere = [origin[0] + half, centre_y, origin[1] + half, radius];
        inst.cluster_count = 1;
        inst
    }

    fn job(&self, (n, m): (Node, u32), slot: u32) -> Job {
        let size = self.size(n.level);
        Job { origin: [n.ix as f32 * size, n.iz as f32 * size], spacing: self.cell(n.level), mask: m, slot, morph: 0, morph_t: 1.0, _pad: 0 }
    }

    /// This frame's changes, CPU only. Up to `build_budget` new patches are
    /// built; when the views want more, they are built hidden and the shown
    /// set changes only once all of them exist (no holes, no stall).
    pub fn plan(&mut self, views: &[TerrainView]) -> Plan {
        let (wanted, patches, finest_level) = self.wanted(views);
        let mut stats = Stats { patches, finest_level, lod_scale: self.lod, ..Default::default() };
        if self.settled.as_ref().is_some_and(|s| Arc::ptr_eq(s, &wanted)) {
            let morph_jobs = self.morph_jobs();
            return Plan { stats, changed: Vec::new(), jobs: Vec::new(), morph_jobs };
        }
        let mut changed: Vec<(u32, GpuInstance)> = Vec::new();
        let mut jobs = Vec::new();
        // Hidden patches no longer wanted go back to the pool (still hidden).
        let free = &mut self.free;
        self.hidden.retain(|key, (slot, _)| { if wanted.contains(key) { return true; } free.push(*slot); stats.freed += 1; false });
        let mut new: Vec<(Node, u32)> = wanted.iter().filter(|k| !self.slots.contains_key(k) && !self.hidden.contains_key(k)).copied().collect();
        // Coarse first: if the pool still runs out, far ground stays and only
        // some detail goes missing.
        new.sort_by_key(|(n, m)| (std::cmp::Reverse(n.level), n.ix, n.iz, *m));
        let budget = self.settings.build_budget.max(1);
        // With nothing shown yet (the first frame) there is nothing to keep.
        if new.len() > budget && new.len() <= self.free.len() && !self.slots.is_empty() {
            // Build ahead, hidden; the shown set stays as it is.
            for key in new.into_iter().take(budget) {
                let slot = self.free.pop().unwrap();
                jobs.push(self.job(key, slot));
                let inst = self.shown_instance(key.0, slot);
                self.hidden.insert(key, (slot, inst));
                stats.built += 1;
            }
            stats.pending = wanted.len() - self.slots.keys().filter(|k| wanted.contains(*k)).count() - self.hidden.len();
            let morph_jobs = self.morph_jobs();
            return Plan { stats, changed, jobs, morph_jobs };
        }
        // Swap: everything wanted is built by the end of this frame (or the
        // pool has no room to build ahead, which is the old behaviour).
        let old: HashSet<Node> = self.slots.keys().map(|k| k.0).collect();
        let root = self.settings.root_level;
        let mut fresh: Vec<((Node, u32), u32)> = Vec::new();
        let morphs = &mut self.morphs;
        self.slots.retain(|key, slot| {
            if wanted.contains(key) { return true; }
            morphs.remove(slot);
            self.free.push(*slot);
            changed.push((*slot, hidden_instance(&self.spec, *slot)));
            stats.freed += 1;
            false
        });
        for (key, (slot, inst)) in self.hidden.drain() {
            changed.push((slot, inst));
            self.slots.insert(key, slot);
            fresh.push((key, slot));
        }
        for key in new {
            let Some(slot) = self.free.pop() else { stats.overflow += 1; continue; };
            jobs.push(self.job(key, slot));
            changed.push((slot, self.shown_instance(key.0, slot)));
            self.slots.insert(key, slot);
            fresh.push((key, slot));
            stats.built += 1;
        }
        if self.settings.morph_s > 0.0 && !old.is_empty() { self.start_morphs(&old, &fresh, root); }
        changed.sort_by_key(|c| c.0);
        if stats.overflow == 0 { self.settled = Some(wanted); }
        let morph_jobs = self.morph_jobs();
        Plan { stats, changed, jobs, morph_jobs }
    }

    /// Where each patch shown by a swap comes from (`old`: the nodes shown
    /// before it), and which of its edges slide with a neighbour that comes
    /// from the same side; a patch with nothing to slide from swaps at once.
    fn start_morphs(&mut self, old: &HashSet<Node>, fresh: &[((Node, u32), u32)], root: u32) {
        let mut below: HashSet<Node> = HashSet::default();
        for &n in old {
            let mut a = n;
            while a.level < root { a = a.parent(); if !below.insert(a) { break; } }
        }
        let from = |n: Node| {
            if old.contains(&n) { return 0; }
            if below.contains(&n) { return MORPH_FROM_FINER; }
            let mut a = n;
            while a.level < root { a = a.parent(); if old.contains(&a) { return MORPH_FROM_COARSER; } }
            0
        };
        let mut sliding: Vec<((Node, u32), u32, u32)> = fresh.iter().map(|&(key, slot)| (key, slot, from(key.0)))
            .filter(|&(key, _, dir)| dir != 0 && self.cell(key.0.level) <= self.settings.morph_max_cell).collect();
        sliding.sort_by_key(|&(key, _, _)| (key.0.level, key.0.ix, key.0.iz));
        sliding.truncate(self.settings.morph_max_patches.saturating_sub(self.morphs.len()));
        // Only patches that slide count as sliding neighbours.
        let dirs: HashMap<Node, (u32, (Node, u32))> = sliding.iter().map(|&(key, _, dir)| (key.0, (dir, key))).collect();
        let started_s = self.clock.elapsed().as_secs_f64();
        for &(key, slot, dir) in &sliding {
            let n = key.0;
            let mut with = [None; 4];
            let mut morph = dir;
            for (e, (dx, dz)) in [(-1, 0), (1, 0), (0, -1), (0, 1)].into_iter().enumerate() {
                let nb = Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz };
                if let Some(&(d, nb_key)) = dirs.get(&nb) {
                    if d == dir { with[e] = Some(nb_key); morph |= 4 << e; }
                }
            }
            let mut job = self.job(key, slot);
            job.morph = morph;
            job.morph_t = 0.0;
            self.morphs.insert(slot, Morph { job, started_s, with });
        }
    }

    /// This frame's rebuilds of the sliding patches; the last one of each
    /// builds its own shape.
    fn morph_jobs(&mut self) -> Vec<Job> {
        if self.morphs.is_empty() { return Vec::new(); }
        let now = self.clock.elapsed().as_secs_f64();
        let span = f64::from(self.settings.morph_s.max(1e-3));
        let slots = &self.slots;
        let mut jobs = Vec::with_capacity(self.morphs.len());
        self.morphs.retain(|_, m| {
            for (e, w) in m.with.iter_mut().enumerate() {
                if w.is_some_and(|k| !slots.contains_key(&k)) { *w = None; m.job.morph &= !(4 << e); }
            }
            let t = ((now - m.started_s) / span) as f32;
            let mut job = m.job;
            if t >= 1.0 { job.morph = 0; job.morph_t = 1.0; } else { job.morph_t = t.max(0.0); }
            jobs.push(job);
            t < 1.0
        });
        jobs
    }

    /// Choose this frame's patches, build the new ones on the GPU and hide
    /// the ones no longer needed. Call once per frame before rendering.
    pub fn update(&mut self, gpu: &Gpu, renderer: &Renderer, views: &[TerrainView]) -> Result<Stats, String> {
        let Plan { stats, changed, mut jobs, morph_jobs } = self.plan(views);
        // One job per slot: a sliding patch's rebuild replaces its plain build.
        if !morph_jobs.is_empty() {
            let sliding: HashSet<u32> = morph_jobs.iter().map(|j| j.slot).collect();
            jobs.retain(|j| !sliding.contains(&j.slot));
            jobs.extend(morph_jobs);
        }
        // One upload from the first changed slot to the last: freed slots are
        // scattered, and every separate write costs more than the bytes between.
        if let (Some(&(first, _)), Some(&(last, _))) = (changed.first(), changed.last()) {
            for &(slot, instance) in &changed { self.instances[slot as usize] = instance; }
            renderer.write_procedural_instances(gpu, (self.spec.first_instance + first) as usize, &self.instances[first as usize..=last as usize])?;
        }
        if !jobs.is_empty() { renderer.queue_procedural_terrain(jobs)?; }
        Ok(stats)
    }
}

fn hidden_instance(spec: &TerrainSpec, slot: u32) -> GpuInstance {
    GpuInstance {
        rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        sphere: [0.0, -1.0e7, 0.0, 0.0],
        first_cluster: spec.first_cluster + slot,
        cluster_count: 0,
        scale: 1.0,
        flags: PROCEDURAL | (GeometryCategory::Terrain as u32) << INSTANCE_CATEGORY_SHIFT,
    }
}

/// The leaf covering node `n`'s area at `n.level` or coarser, if any.
#[cfg(test)]
fn covering(leaves: &HashSet<Node>, mut n: Node, root: u32) -> Option<Node> {
    loop {
        if leaves.contains(&n) { return Some(n); }
        if n.level >= root { return None; }
        n = n.parent();
    }
}

/// Explicit quadtree occupancy: branches terminate unsuccessful neighbour
/// queries immediately. Splits update the index rather than rebuilding it.
struct TerrainTopology {
    nodes: HashMap<Node, bool>, // true: leaf; false: branch
    indexed: bool,
}
impl TerrainTopology {
    fn new(leaves: &HashSet<Node>, root: u32) -> Self {
        Self::with_index(leaves, root, std::env::var("WARP_TERRAIN_TOPOLOGY_INDEX").is_ok_and(|v| v == "1"))
    }
    fn with_index(leaves: &HashSet<Node>, root: u32, indexed: bool) -> Self {
        let mut nodes = HashMap::with_capacity_and_hasher(leaves.len() * 2, Default::default());
        for &leaf in leaves {
            nodes.insert(leaf, true);
            let mut n = leaf;
            while indexed && n.level < root {
                n = n.parent();
                if nodes.insert(n, false).is_some() { break; }
            }
        }
        Self { nodes, indexed }
    }
    fn covering(&self, mut n: Node, root: u32) -> Option<Node> {
        loop {
            if let Some(&leaf) = self.nodes.get(&n) {
                if leaf { return Some(n); }
                if self.indexed { return None; }
            }
            if n.level >= root { return None; }
            n = n.parent();
        }
    }
    fn split(&mut self, n: Node) {
        if self.indexed { self.nodes.insert(n, false); } else { self.nodes.remove(&n); }
        for child in n.children() { self.nodes.insert(child, true); }
    }
}

fn balance(leaves: &mut HashSet<Node>, root: u32) {
    let mut topology = TerrainTopology::new(leaves, root);
    let mut queue: Vec<Node> = leaves.iter().copied().collect();
    queue.sort_by_key(|n| (n.level, n.ix, n.iz));
    while let Some(n) = queue.pop() {
        if !leaves.contains(&n) { continue; }
        for dz in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dz == 0 { continue; }
                let m = Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz };
                if let Some(c) = topology.covering(m, root) {
                    if c.level > n.level + 1 {
                        leaves.remove(&c);
                        topology.split(c);
                        for k in c.children() { leaves.insert(k); queue.push(k); }
                        queue.push(n);
                    }
                }
            }
        }
    }
}

/// Edge (bits 0..3: -x, +x, -z, +z) and corner (bits 4..7) flags for
/// neighbours one level coarser.
fn mask(topology: &TerrainTopology, n: Node, root: u32) -> u32 {
    let coarser = |dx: i32, dz: i32| {
        topology.covering(Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz }, root).is_some_and(|c| c.level > n.level)
    };
    let mut m = 0;
    if coarser(-1, 0) { m |= 1; }
    if coarser(1, 0) { m |= 2; }
    if coarser(0, -1) { m |= 4; }
    if coarser(0, 1) { m |= 8; }
    for (bit, (cx, cz)) in [(-1, -1), (1, -1), (-1, 1), (1, 1)].into_iter().enumerate() {
        if coarser(cx, 0) || coarser(0, cz) || coarser(cx, cz) { m |= 16 << bit; }
    }
    m
}

/// The GPU half, owned by the renderer.
pub(crate) struct TerrainRenderer {
    spec: TerrainSpec,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    jobs: Mutex<(wgpu::Buffer, wgpu::BindGroup)>,
    pending: Mutex<Vec<Job>>,
    vertices: crate::topology::Topology,
    clusters: wgpu::Buffer,
    world: world_constraints::GpuWorld,
}

impl TerrainRenderer {
    pub fn new(gpu: &Gpu, spec: TerrainSpec, vertices: &crate::topology::Topology, clusters: &wgpu::Buffer) -> Self {
        let source = format!("{}\n{}\n{}", terrain_field::WGSL, world_constraints::wgsl(0, 4), SHADER);
        let world = world_constraints::GpuWorld::new(gpu, spec.constraints.as_deref().unwrap_or(&world_constraints::World::empty()));
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("procedural terrain"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let st = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("procedural terrain"), entries: &[
            entry(0, wgpu::BufferBindingType::Uniform), entry(1, st(true)), entry(2, st(false)), entry(3, st(false)),
            entry(4, st(true)), entry(5, st(true)), entry(6, st(true))] });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("procedural terrain"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("procedural terrain build"), layout: Some(&pipeline_layout),
            module: &module, entry_point: Some("build"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let params = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("procedural terrain params"), size: 16 + std::mem::size_of::<terrain_field::Style>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let jobs = Self::job_buffer(gpu, 1024);
        let bind = Self::bind(gpu, &layout, &params, &jobs, vertices, clusters, &world);
        TerrainRenderer { spec, pipeline, layout, params, jobs: Mutex::new((jobs, bind)), pending: Mutex::new(Vec::new()),
            vertices: vertices.clone(), clusters: clusters.clone(), world }
    }

    fn job_buffer(gpu: &Gpu, count: usize) -> wgpu::Buffer {
        gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("procedural terrain jobs"), size: (count * std::mem::size_of::<Job>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    fn bind(gpu: &Gpu, layout: &wgpu::BindGroupLayout, params: &wgpu::Buffer, jobs: &wgpu::Buffer, vertices: &crate::topology::Topology, clusters: &wgpu::Buffer,
        world: &world_constraints::GpuWorld) -> wgpu::BindGroup {
        let [w0, w1, w2] = world.entries(4);
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("procedural terrain"), layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: jobs.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: vertices.vertices() },
            wgpu::BindGroupEntry { binding: 3, resource: clusters.as_entire_binding() }, w0, w1, w2] })
    }

    /// Validation readback: slot `slot`'s 81 vertices and its cluster words.
    pub fn debug_slot(&self, gpu: &Gpu, slot: u32, instances: &wgpu::Buffer) -> (Vec<f32>, Vec<u32>, Vec<f32>) {
        let read = |src: &wgpu::Buffer, offset: u64, bytes: u64| -> Vec<u8> {
            let dst = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(src, offset, &dst, 0, bytes);
            gpu.queue.submit([enc.finish()]);
            dst.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            let v = dst.slice(..).get_mapped_range().expect("mapped").to_vec();
            v
        };
        let v = read(&self.vertices.buffer, ((self.spec.first_vertex + slot * VERTS) * 32) as u64, (VERTS * 32) as u64);
        let c = read(&self.clusters, ((self.spec.first_cluster + slot) * 80) as u64, 80);
        let i = read(instances, ((self.spec.first_instance + slot) * 80) as u64, 80);
        (bytemuck::cast_slice(&v).to_vec(), bytemuck::cast_slice(&c).to_vec(), bytemuck::cast_slice(&i).to_vec())
    }

    pub fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        let (a, b) = (self.spec.first_instance as usize, (self.spec.first_instance + self.spec.capacity) as usize);
        first < b && first.saturating_add(count) > a
    }

    pub fn queue(&self, jobs: Vec<Job>) {
        let mut pending = self.pending.lock().unwrap();
        // A slot rebuilt twice before a frame keeps only its last job.
        let mut seen = HashSet::default();
        for j in jobs.iter() { seen.insert(j.slot); }
        pending.retain(|j| !seen.contains(&j.slot));
        pending.extend(jobs);
    }

    /// Build queued patches. Runs before every view and shadow pass.
    pub fn encode(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder) -> bool {
        let jobs = std::mem::take(&mut *self.pending.lock().unwrap());
        if jobs.is_empty() { return false; }
        let mut held = self.jobs.lock().unwrap();
        let bytes = (jobs.len() * std::mem::size_of::<Job>()) as u64;
        if held.0.size() < bytes {
            let buffer = Self::job_buffer(gpu, jobs.len().next_power_of_two());
            let bind = Self::bind(gpu, &self.layout, &self.params, &buffer, &self.vertices, &self.clusters, &self.world);
            *held = (buffer, bind);
        }
        gpu.queue.write_buffer(&held.0, 0, bytemuck::cast_slice(&jobs));
        let params = [self.spec.first_vertex, self.spec.first_cluster, jobs.len() as u32, self.spec.seed];
        gpu.queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params));
        gpu.queue.write_buffer(&self.params, 16, bytemuck::bytes_of(&self.spec.style));
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("procedural terrain"), timestamp_writes: None });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &held.1, &[]);
        let n = jobs.len() as u32;
        pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain() -> ProceduralTerrain {
        ProceduralTerrain::new(TerrainSpec { first_instance: 0, first_cluster: 0, first_vertex: 0, capacity: 1 << 16, seed: 1, style: terrain_field::Style::default(), constraints: None }, Settings::default())
    }

    #[test]
    fn topology_queries_match_reference_after_splits() {
        let root = 7;
        let mut leaves: HashSet<Node> = (-1..=1).flat_map(|ix| (-1..=1).map(move |iz| Node { level: root, ix, iz })).collect();
        let mut topology = TerrainTopology::with_index(&leaves, root, true);
        let mut seed = 13u32;
        for _ in 0..250 {
            let mut ordered: Vec<_> = leaves.iter().copied().filter(|n| n.level > 0).collect();
            ordered.sort_by_key(|n| (n.level, n.ix, n.iz));
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = ordered[seed as usize % ordered.len()];
            leaves.remove(&n);
            leaves.extend(n.children());
            topology.split(n);
            for &leaf in &leaves {
                for dz in -1..=1 { for dx in -1..=1 {
                    let q = Node { level: leaf.level, ix: leaf.ix + dx, iz: leaf.iz + dz };
                    assert_eq!(topology.covering(q, root), covering(&leaves, q, root));
                }}
            }
        }
        assert_eq!(topology.nodes, TerrainTopology::with_index(&leaves, root, true).nodes);
    }

    #[test]
    fn selection_is_balanced_and_detailed_near_the_camera() {
        let t = terrain();
        let y = terrain_field::height(10.0, 20.0, 1, &terrain_field::Style::default());
        let pos = Vec3::new(10.0, y + 1.7, 20.0);
        let camera = Camera { pos, target: pos + Vec3::new(1.0, -0.1, 0.3), fov_y: 1.0, near: 0.1 };
        let views = [TerrainView::new(&camera, 1920, 1080)];
        let leaves = t.select(&views);
        let finest = leaves.iter().map(|n| n.level).min().unwrap();
        assert!(finest <= 2, "fine patches near the camera: finest {finest}, camera {pos:?}");
        for &n in &leaves {
            for dz in -1..=1 { for dx in -1..=1 {
                if let Some(c) = covering(&leaves, Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz }, t.settings.root_level) {
                    assert!(c.level <= n.level + 1, "unbalanced {n:?} next to {c:?}");
                }
            }}
        }
        let mut hist = std::collections::BTreeMap::new();
        for n in &leaves { *hist.entry(n.level).or_insert(0) += 1; }
        let mut raw = HashSet::default();
        let mut stack: Vec<Node> = Vec::new();
        let rs = t.size(t.settings.root_level);
        for ix in -3..=3 { for iz in -3..=3 { stack.push(Node { level: t.settings.root_level, ix, iz }); } }
        while let Some(n) = stack.pop() { if t.wants_split(n, &views) { stack.extend(n.children()); } else { raw.insert(n); } }
        let mut seen = 0; let mut far = Vec::new();
        for n in leaves.iter().filter(|n| n.level == 6) {
            let size = t.size(6);
            let c = Vec3::new((n.ix as f32 + 0.5) * size, views[0].pos.y, (n.iz as f32 + 0.5) * size);
            if views[0].sees(c, size) { seen += 1; }
            far.push((c - views[0].pos).length() as i32);
        }
        far.sort();
        eprintln!("cam {:?} level6 seen {seen} dist p10 {} p50 {} p90 {}", views[0].pos, far[far.len()/10], far[far.len()/2], far[far.len()*9/10]);
        eprintln!("{} patches {:?}, unbalanced {} root size {rs}", leaves.len(), hist, raw.len());
        assert!(leaves.len() < 20000, "{} patches", leaves.len());
    }

    /// Joep's playtest spot (x 164, z -163, 46 m up), a 5K view at `yaw_deg`.
    fn turn_view(t: &ProceduralTerrain, yaw_deg: f32) -> TerrainView {
        let probe = Node { level: 2, ix: (164.0 / t.size(2)).floor() as i32, iz: (-163.0 / t.size(2)).floor() as i32 };
        let pos = Vec3::new(164.0, t.sample(probe).0 + 46.0, -163.0);
        let yaw = yaw_deg.to_radians();
        TerrainView::new(&Camera { pos, target: pos + Vec3::new(-yaw.sin(), -0.03, -yaw.cos()), fov_y: 65f32.to_radians(), near: 0.05 }, 5120, 2880)
    }

    #[test]
    fn turns_build_within_budget_and_show_whole_selections() {
        let mut t = terrain();
        let budget = t.settings.build_budget;
        t.plan(&[turn_view(&t, 0.0)]);
        while t.plan(&[turn_view(&t, 0.0)]).stats.pending > 0 {}
        let mut shown: HashSet<(Node, u32)> = t.slots.keys().copied().collect();
        let mut deferred = 0;
        for step in 1..=12 {
            let view = turn_view(&t, step as f32 * 30.0);
            for _ in 0..64 {
                let plan = t.plan(&[view]);
                assert!(plan.jobs.len() <= budget, "{} builds in one frame", plan.jobs.len());
                let now: HashSet<(Node, u32)> = t.slots.keys().copied().collect();
                let wanted = (*t.selection.as_ref().unwrap().1).clone();
                // The shown set is the previous selection or the new one,
                // never a mix with holes or overlaps.
                assert!(now == shown || now == wanted, "partial swap at step {step}");
                if now != shown { shown = now; }
                if plan.stats.pending == 0 && shown == wanted { break; }
                deferred += 1;
            }
            assert_eq!(shown, *t.selection.as_ref().unwrap().1, "step {step} never completed");
        }
        assert!(deferred > 0, "no turn needed more than one frame's budget");
    }

    #[test]
    fn turns_change_far_fewer_patches_than_with_eightfold_offscreen() {
        let worst = |settings: Settings| {
            let mut t = ProceduralTerrain::new(TerrainSpec { first_instance: 0, first_cluster: 0, first_vertex: 0, capacity: 1 << 16, seed: 1,
                style: terrain_field::Style::default(), constraints: None }, settings);
            let mut prev = HashSet::default();
            let mut worst = 0;
            for step in 0..=24 {
                let view = turn_view(&t, step as f32 * 30.0);
                t.wanted(&[view]);
                let keys = (*t.selection.as_ref().unwrap().1).clone();
                if step > 12 { worst = worst.max(keys.difference(&prev).count()); }
                prev = keys;
            }
            worst
        };
        let old = worst(Settings { offscreen_factor: 8.0, cone_margin_deg: 0.0, ..Settings::default() });
        let new = worst(Settings::default());
        assert!(new * 10 <= old * 8, "worst new patches per 30 degree turn: {new}, was {old}");
    }

    #[test]
    fn build_shader_validates() {
        let source = format!("{}\n{}\n{}", terrain_field::WGSL, world_constraints::wgsl(0, 4), SHADER);
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        assert_eq!(std::mem::size_of::<Job>(), 32);
    }

    #[test]
    fn walking_slides_new_detail_in_and_ends_on_the_patches_own_shape() {
        let mut t = terrain();
        t.settings.morph_s = 1e6;
        let mut view = turn_view(&t, 40.0);
        while t.plan(&[view]).stats.pending > 0 {}
        let mut splits = 0;
        let mut merges = 0;
        for step in 0..40 {
            view.pos += Vec3::new(0.0, 0.0, -1.0);
            let plan = t.plan(&[view]);
            for j in &plan.morph_jobs {
                assert!(j.morph & 3 != 0 && j.morph_t < 1.0, "step {step}: {j:?}");
                assert!(t.slots.values().any(|&s| s == j.slot), "a sliding slot is shown");
                splits += usize::from(j.morph & 3 == MORPH_FROM_COARSER);
                merges += usize::from(j.morph & 3 == MORPH_FROM_FINER);
            }
            // Every sliding edge slides with a shown same-level neighbour
            // that slides along it the same way.
            for m in t.morphs.values() {
                for (e, w) in m.with.iter().enumerate() {
                    let Some(k) = w else { assert_eq!(m.job.morph & (4 << e), 0); continue };
                    let other = t.morphs.get(&t.slots[k]).expect("neighbour still sliding");
                    assert_eq!(other.job.morph & 3, m.job.morph & 3);
                    assert_ne!(other.job.morph & (4 << (e ^ 1)), 0, "edge slides on one side only");
                }
            }
        }
        assert!(splits > 0 && merges > 0, "splits {splits}, merges {merges}");
        // Finished slides rebuild the patch in its own shape once.
        t.settings.morph_s = 1e-9;
        let last = t.plan(&[view]).morph_jobs;
        assert!(!last.is_empty() && last.iter().all(|j| j.morph == 0 && j.morph_t == 1.0));
        assert!(t.plan(&[view]).morph_jobs.is_empty());
    }

    #[test]
    fn a_still_camera_plans_nothing() {
        let mut t = terrain();
        let view = turn_view(&t, 40.0);
        while t.plan(&[view]).stats.pending > 0 {}
        let plan = t.plan(&[view]);
        assert!(plan.jobs.is_empty() && plan.changed.is_empty());
    }

    #[test]
    fn alpine_views_fit_the_pool() {
        let style = terrain_field::Style::named("alpine").unwrap();
        let mut t = ProceduralTerrain::new(TerrainSpec { first_instance: 0, first_cluster: 0, first_vertex: 0, capacity: 16384, seed: 1, style, constraints: None }, Settings::default());
        // (x, z, metres above ground, yaw, pitch in degrees); the last two
        // are F12 screenshots that were mostly holes.
        for (x, z, above, yaw, pitch) in [(0.0f32, 0.0f32, 60.0, 35.0f32, -9.0f32), (-350.0, -742.0, 3.0, 35.0, -6.0), (542.0, -375.0, 257.0, 35.0, -17.0),
            (2000.0, 1500.0, 900.0, 35.0, -27.0), (-32.7, -28.1, 61.9, 86.6, -13.0), (-63.5, -77.1, 45.6, 108.0, -10.2)] {
            let pos = Vec3::new(x, terrain_field::height(x, z, 1, &style) + above, z);
            let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
            let forward = Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
            let camera = Camera { pos, target: pos + forward, fov_y: 65f32.to_radians(), near: 0.05 };
            // A Retina window at 5K renders at 5120x2880.
            let views = [TerrainView::new(&camera, 5120, 2880)];
            let raw = t.select(&views).len();
            let started = web_time::Instant::now();
            let fit = t.select_within_budget(&views).len();
            let first = started.elapsed().as_secs_f64() * 1e3;
            let started = web_time::Instant::now();
            t.select_within_budget(&views);
            eprintln!("at {pos:?}: wanted {raw}, fits {fit} at lod {:.2} in {first:.1} ms, next frame {:.1} ms", t.lod, started.elapsed().as_secs_f64() * 1e3);
            assert!(fit <= 16384 * 9 / 10, "{fit} patches");
        }
    }
}
