//! Bounded moving ground cover using the existing photographed tuft material.
//! World tiles cache candidates, while a fixed instance pool keeps GPU buffers
//! and every surviving plant's position/size stable. This first live pass uses
//! deterministic whole-plant density falloff, not opacity blending or dither.

use glam::{Affine3A, Quat, Vec2, Vec3};
use render::scene::{ALPHA_TEST, GRASS, Material, Scene, TWO_SIDED};
use sideways::builder::{Chunk, N};
use sideways::stage::{CHUNK, GRID};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, ops::Range, path::Path, time::Instant};

const CELL: f32 = 0.45;
const TILE_CELLS: i32 = 8;
const TILE: f32 = CELL * TILE_CELLS as f32;
const TUFT_WIDTH: f32 = 0.36 * (257.0 / 135.0);
type Cell = (i32, i32);

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub capacity: usize,
    pub active: usize,
    pub cached_tiles: usize,
    pub updates: u64,
    pub last_update_ms: f64,
    pub ground_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tuft { id: Cell, position: Vec3, scale: f32, yaw: f32, coverage: f32 }
struct GroundChunk { x0: f32, z0: f32, heights: Box<[f32]>, road: Box<[f32]> }
struct Ground(HashMap<Cell, GroundChunk>);
#[derive(Clone, Copy)]
struct Sample { height: f32, road: f32, up: f32 }

impl Ground {
    fn new(chunks: &[Chunk]) -> Self {
        Self(chunks.iter().map(|c| (c.cell, GroundChunk { x0: c.x0 as f32, z0: c.z0 as f32,
            heights: c.heights.clone().into_boxed_slice(), road: c.road_d.clone().into_boxed_slice() })).collect())
    }
    fn sample(&self, p: Vec2) -> Option<Sample> {
        let cell = ((p.x / CHUNK as f32).floor() as i32, (p.y / CHUNK as f32).floor() as i32);
        for dz in [0, -1] { for dx in [0, -1] {
            let Some(c) = self.0.get(&(cell.0 + dx, cell.1 + dz)) else { continue; };
            let q = (p - Vec2::new(c.x0, c.z0)) / GRID as f32;
            if !(0.0..=(N - 1) as f32).contains(&q.x) || !(0.0..=(N - 1) as f32).contains(&q.y) { continue; }
            let i = (q.x.floor() as usize).min(N - 2); let j = (q.y.floor() as usize).min(N - 2);
            let x = q.x - i as f32; let z = q.y - j as f32; let a = j * N + i;
            let (indices, weights) = if x + z <= 1.0 { ([a, a + 1, a + N], [1.0 - x - z, x, z]) }
                else { ([a + N + 1, a + N, a + 1], [x + z - 1.0, 1.0 - x, 1.0 - z]) };
            let interpolate = |values: &[f32]| (0..3).map(|k| values[indices[k]] * weights[k]).sum::<f32>();
            let dx = if x + z <= 1.0 { c.heights[a + 1] - c.heights[a] } else { c.heights[a + N + 1] - c.heights[a + N] } / GRID as f32;
            let dz = if x + z <= 1.0 { c.heights[a + N] - c.heights[a] } else { c.heights[a + N + 1] - c.heights[a + 1] } / GRID as f32;
            return Some(Sample { height: interpolate(&c.heights), road: interpolate(&c.road), up: 1.0 / (1.0 + dx * dx + dz * dz).sqrt() });
        }}
        None
    }
}

fn hash(id: Cell, salt: u32) -> u32 {
    let mut x = (id.0 as u32).wrapping_mul(0x9e3779b1) ^ (id.1 as u32).wrapping_mul(0x85ebca77) ^ salt.wrapping_mul(0xc2b2ae3d);
    x ^= x >> 16; x = x.wrapping_mul(0x7feb352d); x ^= x >> 15; x = x.wrapping_mul(0x846ca68b); x ^ (x >> 16)
}
fn random(id: Cell, salt: u32) -> f32 { (hash(id, salt) >> 8) as f32 / 16_777_216.0 }
fn smooth(lo: f32, hi: f32, x: f32) -> f32 { let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
fn noise(p: Vec2) -> f32 {
    let id = (p.x.floor() as i32, p.y.floor() as i32);
    let x = smooth(0.0, 1.0, p.x - p.x.floor()); let y = smooth(0.0, 1.0, p.y - p.y.floor());
    let a = random(id, 103); let b = random((id.0 + 1, id.1), 103);
    let c = random((id.0, id.1 + 1), 103); let d = random((id.0 + 1, id.1 + 1), 103);
    (a + (b - a) * x) * (1.0 - y) + (c + (d - c) * x) * y
}
fn radius(views: usize) -> f32 { match views { 0 | 1 => 32.0, 2 => 26.0, _ => 22.0 } }

fn candidate(ground: &Ground, id: Cell) -> Option<Tuft> {
    let jitter = Vec2::new(random(id, 0) - 0.5, random(id, 17) - 0.5) * 1.6;
    let xz = (Vec2::new(id.0 as f32, id.1 as f32) + Vec2::splat(0.5) + jitter) * CELL;
    let scale = 0.7 + 0.6 * random(id, 9);
    let root = ground.sample(xz)?;
    let clearance = TUFT_WIDTH * 0.5 * scale;
    if root.road < 0.9 + clearance || root.up <= 0.7 { return None; }
    if ![Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y].iter().all(|axis|
        ground.sample(xz + *axis * clearance).is_some_and(|sample| sample.road >= 0.9)) { return None; }
    let verge = smooth(0.5, 1.4, root.road) * (1.0 - smooth(10.0, 22.0, root.road));
    let wood = smooth(0.3, 0.6, noise(xz / 7.0)) * 0.85 * smooth(1.0, 3.0, root.road);
    let coverage = verge.max(wood) * smooth(0.7, 0.82, root.up);
    if coverage <= 0.0 { return None; }
    Some(Tuft { id, position: Vec3::new(xz.x, root.height - 0.025, xz.y), scale,
        yaw: random(id, 3) * std::f32::consts::TAU, coverage })
}

pub struct LiveGroundCover {
    ground: Ground,
    mesh: usize,
    first: usize,
    capacity: usize,
    anchors: Option<(usize, Vec<Cell>)>,
    tiles: BTreeMap<Cell, Vec<Tuft>>,
    active: BTreeMap<Cell, usize>,
    free: BTreeSet<usize>,
    stats: Stats,
}

impl LiveGroundCover {
    /// Reserve before the renderer uploads the scene and before cars are added.
    /// Disabled pool slots have zero clusters, not zero-sized moving plants.
    pub fn reserve(scene: &mut Scene, chunks: &[Chunk], cameras: &[Vec3], root: &Path,
        mesh_names: &mut Vec<String>) -> Result<Self, String> {
        let layer = scene.add_image("Poly Haven grass_medium_01 selected CC0 live clump", crate::ground_cover::photo(root)?);
        let material = scene.add_material(Material { tint: [1.0; 3], layer, roughness: 0.85,
            flags: ALPHA_TEST | TWO_SIDED | GRASS, scale: 1.0, metal: 0.0 });
        let mesh = scene.add_mesh(&crate::ground_cover::mesh(material));
        mesh_names.push("photographed grass tuft (CC0 live pool)".into());
        let capacity = if cameras.len() <= 1 { 16_000 } else { 24_000 };
        let mut stream = Self::with_mesh(scene, chunks, mesh, capacity);
        stream.update(scene, cameras);
        Ok(stream)
    }

    fn with_mesh(scene: &mut Scene, chunks: &[Chunk], mesh: usize, capacity: usize) -> Self {
        let first = scene.instances.len();
        for _ in 0..capacity {
            scene.add_instance(mesh, Affine3A::IDENTITY); scene.mark_no_shadow();
            let instance = scene.instances.last_mut().unwrap(); instance.cluster_count = 0; instance.sphere[3] = 0.0;
        }
        let ground_bytes = chunks.iter().map(|c| (c.heights.len() + c.road_d.len()) * std::mem::size_of::<f32>()).sum();
        Self { ground: Ground::new(chunks), mesh, first, capacity, anchors: None, tiles: BTreeMap::new(),
            active: BTreeMap::new(), free: (0..capacity).collect(), stats: Stats { capacity, ground_bytes, ..Default::default() } }
    }

    pub fn stats(&self) -> Stats { self.stats }
    pub fn summary(&self) -> String {
        format!("live ground cover: {} active / {} fixed slots, {} cached world tiles, {:.1} MiB native ground, {:.2} ms last update; stable sizes, photo cutout, no casting shadows",
            self.stats.active, self.capacity, self.tiles.len(), self.stats.ground_bytes as f64 / 1_048_576.0, self.stats.last_update_ms)
    }

    /// Returns the contiguous union of changed slots. An unchanged camera tile
    /// does no candidate generation, transform writes or GPU upload work.
    pub fn update(&mut self, scene: &mut Scene, cameras: &[Vec3]) -> Option<Range<usize>> {
        let views = cameras.len().clamp(1, 4);
        let mut anchors: Vec<Cell> = cameras.iter().take(4)
            .filter(|c| c.is_finite() && c.x.abs() < 10_000_000.0 && c.z.abs() < 10_000_000.0)
            .map(|c| ((c.x / TILE).floor() as i32, (c.z / TILE).floor() as i32)).collect();
        anchors.sort_unstable(); anchors.dedup();
        let key = (views, anchors);
        if self.anchors.as_ref() == Some(&key) { return None; }
        let started = Instant::now();
        let centres: Vec<Vec2> = key.1.iter().map(|&(x, z)| (Vec2::new(x as f32, z as f32) + Vec2::splat(0.5)) * TILE).collect();
        let distance = |p: Vec2| centres.iter().map(|c| p.distance(*c)).fold(f32::INFINITY, f32::min);
        let range = radius(views);
        // Include all candidate jitter/card extents around the visible circle;
        // tiles are only an update/cache granularity, never a square cut line.
        let margin = TILE * std::f32::consts::FRAC_1_SQRT_2 + CELL;
        let reach = ((range + margin) / TILE).ceil() as i32;
        let mut wanted_tiles = BTreeSet::new();
        for &(x, z) in &key.1 { for tz in z - reach..=z + reach { for tx in x - reach..=x + reach {
            let centre = (Vec2::new(tx as f32, tz as f32) + Vec2::splat(0.5)) * TILE;
            if distance(centre) <= range + margin { wanted_tiles.insert((tx, tz)); }
        }}}
        self.tiles.retain(|id, _| wanted_tiles.contains(id));
        for tile in wanted_tiles {
            self.tiles.entry(tile).or_insert_with(|| {
                let mut tufts = Vec::new();
                for z in 0..TILE_CELLS { for x in 0..TILE_CELLS {
                    if let Some(tuft) = candidate(&self.ground, (tile.0 * TILE_CELLS + x, tile.1 * TILE_CELLS + z)) { tufts.push(tuft); }
                }}
                tufts
            });
        }
        let mut selected: Vec<Tuft> = self.tiles.values().flatten().copied().filter(|tuft| {
            let d = distance(Vec2::new(tuft.position.x, tuft.position.z));
            let falloff = (1.0 - smooth(8.0, range, d)).powi(2);
            random(tuft.id, 43) < tuft.coverage * falloff
        }).collect();
        if selected.len() > self.capacity {
            // The explicit hard cap has a stable world priority, rather than
            // dropping all grass from one view or slicing along a grid row.
            selected.sort_unstable_by_key(|tuft| (hash(tuft.id, 71), tuft.id)); selected.truncate(self.capacity);
        }
        let desired: BTreeMap<Cell, Tuft> = selected.into_iter().map(|tuft| (tuft.id, tuft)).collect();
        let removed: Vec<_> = self.active.iter().filter(|(id, _)| !desired.contains_key(*id)).map(|(&id, &slot)| (id, slot)).collect();
        let mut dirty_lo = usize::MAX; let mut dirty_hi = 0;
        for (id, slot) in removed {
            self.active.remove(&id); self.free.insert(slot);
            let instance = &mut scene.instances[self.first + slot]; instance.cluster_count = 0; instance.sphere[3] = 0.0;
            dirty_lo = dirty_lo.min(slot); dirty_hi = dirty_hi.max(slot + 1);
        }
        for (id, tuft) in desired {
            if self.active.contains_key(&id) { continue; }
            let slot = self.free.pop_first().expect("selected grass count respects fixed pool capacity");
            scene.update_instance(self.first + slot, self.mesh, Affine3A::from_scale_rotation_translation(
                Vec3::splat(tuft.scale), Quat::from_rotation_y(tuft.yaw), tuft.position));
            self.active.insert(id, slot); dirty_lo = dirty_lo.min(slot); dirty_hi = dirty_hi.max(slot + 1);
        }
        self.anchors = Some(key);
        self.stats.active = self.active.len(); self.stats.cached_tiles = self.tiles.len();
        self.stats.updates += 1; self.stats.last_update_ms = started.elapsed().as_secs_f64() * 1000.0;
        if dirty_hi > 0 { Some(self.first + dirty_lo..self.first + dirty_hi) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(cell: Cell, road: f32) -> Chunk {
        Chunk { cell, x0: cell.0 as f64 * CHUNK, z0: cell.1 as f64 * CHUNK, heights: vec![0.0; N * N],
            road_d: vec![road; N * N], trees: vec![], rocks: vec![], grass: vec![], ferns: vec![], stumps: vec![] }
    }
    fn setup(chunks: &[Chunk], capacity: usize) -> (Scene, LiveGroundCover) {
        let mut scene = Scene::default();
        let mesh = scene.add_mesh(&crate::ground_cover::mesh(0));
        // Real scenes already contain terrain/trees before the pool.
        scene.add_instance(mesh, Affine3A::from_translation(Vec3::new(500.0, 0.0, 500.0)));
        let stream = LiveGroundCover::with_mesh(&mut scene, chunks, mesh, capacity);
        (scene, stream)
    }
    #[test]
    fn exact_native_roots_and_road_exclusion() {
        let mut c = chunk((0, 0), 5.0); c.heights[1] = 2.0; c.heights[N] = 4.0; c.heights[N + 1] = 10.0;
        let ground = Ground::new(&[c]);
        assert!((ground.sample(Vec2::new(0.5, 0.5)).unwrap().height - 1.5).abs() < 1e-6);
        assert!((ground.sample(Vec2::new(1.5, 1.5)).unwrap().height - 6.5).abs() < 1e-6);
        let road = Ground::new(&[chunk((0, 0), 0.2)]);
        assert!((0..100).all(|i| candidate(&road, (i, 30)).is_none()));
    }
    #[test]
    fn moving_pool_keeps_survivor_poses_slots_and_car_tail_unchanged() {
        let chunks: Vec<_> = (-1..=1).flat_map(|z| (-1..=1).map(move |x| chunk((x, z), 5.0))).collect();
        let (mut scene, mut stream) = setup(&chunks, 16_000);
        let prefix = scene.instances[0].rows;
        let car_start = scene.instances.len(); scene.add_instance(stream.mesh, Affine3A::from_translation(Vec3::new(10.0, 2.0, 10.0)));
        let car = scene.instances[car_start];
        assert!(stream.update(&mut scene, &[Vec3::new(1.0, 2.0, 1.0)]).is_some());
        let before: BTreeMap<_, _> = stream.active.iter().map(|(&id, &slot)| (id, (slot, scene.instances[stream.first + slot].rows))).collect();
        let updates = stream.stats.updates;
        assert!(stream.update(&mut scene, &[Vec3::new(1.1, 2.0, 1.1)]).is_none());
        assert_eq!(stream.stats.updates, updates);
        let dirty = stream.update(&mut scene, &[Vec3::new(5.0, 2.0, 1.0)]).unwrap();
        assert!(dirty.end <= car_start);
        let mut survivors = 0;
        for (&id, &slot) in &stream.active { if let Some(&(old_slot, rows)) = before.get(&id) {
            assert_eq!(slot, old_slot); assert_eq!(scene.instances[stream.first + slot].rows, rows); survivors += 1;
        }}
        assert!(survivors > 100);
        assert_eq!(scene.instances[car_start].rows, car.rows);
        assert_eq!(scene.instances[0].rows, prefix);
        assert!(scene.instances[stream.first..car_start].iter().all(|instance| instance.flags & render::scene::NO_SHADOW != 0));
        stream.update(&mut scene, &[]);
        assert_eq!(stream.stats.active, 0);
        assert!(scene.instances[stream.first..car_start].iter().all(|instance| instance.cluster_count == 0));
    }
    #[test]
    fn four_views_share_world_cells_and_hard_cap_without_resizing() {
        let chunks: Vec<_> = (-1..=2).flat_map(|z| (-1..=2).map(move |x| chunk((x, z), 5.0))).collect();
        let (mut scene, mut stream) = setup(&chunks, 500);
        let cameras = [Vec3::ZERO, Vec3::new(45.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 45.0), Vec3::new(45.0, 0.0, 45.0)];
        stream.update(&mut scene, &cameras);
        assert_eq!(stream.stats.active, 500); assert_eq!(scene.instances.len(), stream.first + 500);
        let positions: Vec<_> = scene.instances.iter().map(|i| i.rows).collect();
        let mut reversed = cameras; reversed.reverse();
        assert!(stream.update(&mut scene, &reversed).is_none());
        assert_eq!(positions, scene.instances.iter().map(|i| i.rows).collect::<Vec<_>>());
    }
}
