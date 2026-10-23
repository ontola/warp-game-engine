//! Bounded moving ground cover using the existing photographed tuft material.
//! World tiles cache candidates, while a fixed instance pool keeps GPU buffers
//! and every surviving plant's position/size stable. CPU residency extends
//! beyond the renderer's continuous fade; moving a cache tile cannot select
//! visible plants by a camera-dependent density hash.

use glam::{Affine3A, Quat, Vec2, Vec3};
use render::scene::{ALPHA_TEST, GRASS, LIVE_GRASS, Material, Scene, TWO_SIDED};
use sideways::builder::{Chunk, N};
use sideways::stage::{CHUNK, GRID};
use std::{collections::{BTreeMap, BTreeSet, HashMap, HashSet}, ops::Range, path::Path, time::Instant};

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
    /// Requested resident plants before the explicit emergency pool cap.
    pub requested: usize,
    pub budget_clipped: usize,
    /// Clipping visible plants would be a quality failure, never normal LOD.
    pub visible_budget_clipped: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tuft { id: Cell, position: Vec3, scale: f32, yaw: f32 }
struct Tile {
    tufts: Vec<Tuft>,
    min: Vec2,
    max: Vec2,
    /// Desired circular residency, including slots omitted by an emergency
    /// pool cap. Candidate order and therefore bit positions never change.
    resident: u64,
}
impl Tile {
    fn new(ground: &Ground, tile: Cell) -> Self {
        let mut tufts = Vec::new();
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for z in 0..TILE_CELLS { for x in 0..TILE_CELLS {
            if let Some(tuft) = candidate(ground, (tile.0 * TILE_CELLS + x, tile.1 * TILE_CELLS + z)) {
                let p = Vec2::new(tuft.position.x, tuft.position.z);
                min = min.min(p); max = max.max(p); tufts.push(tuft);
            }
        }}
        Self { tufts, min, max, resident: 0 }
    }
    fn selected(&self, mask: u64) -> impl Iterator<Item = Tuft> + '_ {
        SetBits(mask).map(|index| self.tufts[index])
    }
    fn mask(&self, centres: &[Vec2], range_squared: f32) -> u64 {
        if self.tufts.is_empty() { return 0; }
        let mut intersects = false;
        for &centre in centres {
            // Actual candidate bounds include jitter and f32 world rounding.
            // The common interior needs no per-plant distance/hash lookup.
            let far = (centre - self.min).abs().max((centre - self.max).abs());
            if far.length_squared() <= range_squared {
                return u64::MAX >> (64 - self.tufts.len());
            }
            if centre.distance_squared(centre.clamp(self.min, self.max)) <= range_squared {
                intersects = true;
            }
        }
        if !intersects { return 0; }
        self.tufts.iter().enumerate().fold(0, |mask, (index, tuft)| {
            let p = Vec2::new(tuft.position.x, tuft.position.z);
            if centres.iter().any(|centre| p.distance_squared(*centre) <= range_squared) {
                mask | (1u64 << index)
            } else { mask }
        })
    }
}
struct SetBits(u64);
impl Iterator for SetBits {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.0 == 0 { return None; }
        let index = self.0.trailing_zeros() as usize;
        self.0 &= self.0 - 1;
        Some(index)
    }
}
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
// Shared with grass_common.wgsl's opacity endpoints: 40..60 m for one view,
// 20..38 m for split views. Four metres cover the maximum 2.55 m camera-to-
// cache-anchor offset, so CPU additions/removals happen at zero opacity.
fn radius(views: usize) -> f32 { if views <= 1 { 64.0 } else { 42.0 } }
fn visible_radius(views: usize) -> f32 { if views <= 1 { 60.0 } else { 38.0 } }

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
    // Keep a continuous low cover inside the woodland, with denser clearings.
    // This is a world habitat mask: camera position never changes membership.
    let wood = (0.55 + 0.45 * smooth(0.25, 0.65, noise(xz / 7.0)))
        * 0.95 * smooth(1.0, 3.0, root.road);
    let coverage = verge.max(wood) * smooth(0.7, 0.82, root.up);
    if random(id, 43) >= coverage { return None; }
    Some(Tuft { id, position: Vec3::new(xz.x, root.height - 0.025, xz.y), scale,
        yaw: random(id, 3) * std::f32::consts::TAU })
}

pub struct LiveGroundCover {
    ground: Ground,
    mesh: usize,
    first: usize,
    capacity: usize,
    anchors: Option<(usize, Vec<Cell>)>,
    tiles: BTreeMap<Cell, Tile>,
    active: HashMap<Cell, usize>,
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
        mesh_names.push("photographed grass tuft (CC0 live opacity pool)".into());
        let capacity = if cameras.len() <= 1 { 70_000 } else { 115_000 };
        let mut stream = Self::with_mesh(scene, chunks, mesh, capacity);
        scene.grass_ranges.push(render::grass::GrassRange { first: stream.first as u32,
            count: capacity as u32, material, width: crate::ground_cover::WIDTH,
            height: crate::ground_cover::HEIGHT, split: cameras.len() > 1 });
        stream.update(scene, cameras);
        Ok(stream)
    }

    fn with_mesh(scene: &mut Scene, chunks: &[Chunk], mesh: usize, capacity: usize) -> Self {
        let first = scene.instances.len();
        for _ in 0..capacity {
            scene.add_instance(mesh, Affine3A::IDENTITY); scene.mark_no_shadow();
            let instance = scene.instances.last_mut().unwrap();
            instance.flags |= LIVE_GRASS;
            instance.cluster_count = 0; instance.sphere[3] = 0.0;
        }
        let ground_bytes = chunks.iter().map(|c| (c.heights.len() + c.road_d.len()) * std::mem::size_of::<f32>()).sum();
        Self { ground: Ground::new(chunks), mesh, first, capacity, anchors: None, tiles: BTreeMap::new(),
            active: HashMap::with_capacity(capacity), free: (0..capacity).collect(), stats: Stats { capacity, ground_bytes, ..Default::default() } }
    }

    pub fn stats(&self) -> Stats { self.stats }
    pub fn summary(&self) -> String {
        format!("live ground cover: {} active / {} fixed slots, {} requested, {} clipped ({} visible), {} cached world tiles, {:.1} MiB native ground, {:.2} ms last update; same world cells, stable 3-card sizes, true opacity outer fade, no casting shadows",
            self.stats.active, self.capacity, self.stats.requested, self.stats.budget_clipped,
            self.stats.visible_budget_clipped, self.tiles.len(),
            self.stats.ground_bytes as f64 / 1_048_576.0, self.stats.last_update_ms)
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
        let distance_squared = |p: Vec2| centres.iter().map(|c| p.distance_squared(*c)).fold(f32::INFINITY, f32::min);
        let range = radius(views);
        // Include all candidate jitter/card extents around the visible circle;
        // tiles are only an update/cache granularity, never a square cut line.
        let margin = TILE * std::f32::consts::FRAC_1_SQRT_2 + CELL;
        let reach = ((range + margin) / TILE).ceil() as i32;
        let mut wanted_tiles = BTreeSet::new();
        for &(x, z) in &key.1 { for tz in z - reach..=z + reach { for tx in x - reach..=x + reach {
            let centre = (Vec2::new(tx as f32, tz as f32) + Vec2::splat(0.5)) * TILE;
            if distance_squared(centre) <= (range + margin).powi(2) { wanted_tiles.insert((tx, tz)); }
        }}}
        let mut removed = Vec::new();
        let mut added = Vec::new();
        self.tiles.retain(|id, tile| {
            if wanted_tiles.contains(id) { return true; }
            removed.extend(tile.selected(tile.resident).map(|tuft| tuft.id));
            false
        });
        for tile in wanted_tiles {
            self.tiles.entry(tile).or_insert_with(|| Tile::new(&self.ground, tile));
        }
        let was_capped = self.stats.budget_clipped > 0;
        self.stats.requested = 0;
        for tile in self.tiles.values_mut() {
            let desired = tile.mask(&centres, range * range);
            removed.extend(tile.selected(tile.resident & !desired).map(|tuft| tuft.id));
            added.extend(tile.selected(desired & !tile.resident));
            tile.resident = desired;
            self.stats.requested += desired.count_ones() as usize;
        }
        self.stats.budget_clipped = self.stats.requested.saturating_sub(self.capacity);
        self.stats.visible_budget_clipped = 0;
        if self.stats.budget_clipped > 0 || was_capped {
            // A capped frame may omit arbitrary interior plants. Reconcile
            // globally while capped and once after leaving that mode; normal
            // streaming only changes bits at the circular residency border.
            let mut selected: Vec<Tuft> = self.tiles.values()
                .flat_map(|tile| tile.selected(tile.resident)).collect();
            // Emergency bound, not a visual LOD. First drop only resident
            // plants already beyond every view's zero-opacity endpoint.
            // If even visible plants exceed the pool, report that explicitly.
            let visible = |tuft: &Tuft| cameras.iter().take(4).any(|camera| {
                camera.is_finite() && Vec2::new(camera.x, camera.z)
                    .distance_squared(Vec2::new(tuft.position.x, tuft.position.z))
                    < visible_radius(views).powi(2)
            });
            if selected.len() > self.capacity {
                selected.sort_unstable_by_key(|tuft| (!visible(tuft), hash(tuft.id, 71), tuft.id));
                self.stats.visible_budget_clipped = selected[self.capacity..].iter().filter(|tuft| visible(tuft)).count();
                selected.truncate(self.capacity);
            }
            let desired: HashSet<Cell> = selected.iter().map(|tuft| tuft.id).collect();
            removed = self.active.keys().filter(|id| !desired.contains(*id)).copied().collect();
            added = selected.into_iter().filter(|tuft| !self.active.contains_key(&tuft.id)).collect();
        }
        let mut dirty_lo = usize::MAX; let mut dirty_hi = 0;
        for id in removed {
            let slot = self.active.remove(&id).expect("resident grass has an active slot outside capped reconciliation");
            self.free.insert(slot);
            let instance = &mut scene.instances[self.first + slot]; instance.cluster_count = 0; instance.sphere[3] = 0.0;
            dirty_lo = dirty_lo.min(slot); dirty_hi = dirty_hi.max(slot + 1);
        }
        for tuft in added {
            let id = tuft.id;
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
        assert!(stream.stats.budget_clipped > 0);
        assert!(stream.stats.visible_budget_clipped > 0);
        let positions: Vec<_> = scene.instances.iter().map(|i| i.rows).collect();
        let mut reversed = cameras; reversed.reverse();
        assert!(stream.update(&mut scene, &reversed).is_none());
        assert_eq!(positions, scene.instances.iter().map(|i| i.rows).collect::<Vec<_>>());
    }

    #[test]
    fn dense_core_is_not_camera_thinned_and_tile_changes_stay_beyond_fade() {
        let chunks: Vec<_> = (-2..=2).flat_map(|z| (-2..=2).map(move |x| chunk((x, z), 5.0))).collect();
        let (mut scene, mut stream) = setup(&chunks, 70_000);
        let previous = Vec3::new(1.7, 2.0, 1.7);
        let current = Vec3::new(3.7, 2.0, 1.7);
        stream.update(&mut scene, &[previous]);
        let position = |instance: &render::scene::GpuInstance| Vec2::new(instance.rows[0][3], instance.rows[2][3]);
        let before: HashMap<_, _> = stream.active.iter().map(|(&id, &slot)|
            (id, position(&scene.instances[stream.first + slot]))).collect();
        // On this fully fertile flat fixture, essentially every 0.45 m cell
        // in the 20 m core is occupied (~6,200), rather than a camera hash.
        assert!(before.values().filter(|p| p.distance(Vec2::new(previous.x, previous.z)) < 20.0).count() > 5_500);
        assert_eq!(stream.stats.budget_clipped, 0);
        stream.update(&mut scene, &[current]);
        assert_eq!(stream.stats.budget_clipped, 0);
        for (id, p) in &before {
            if !stream.active.contains_key(id) {
                assert!(p.distance(Vec2::new(current.x, current.z)) > visible_radius(1));
            }
        }
        for (&id, &slot) in &stream.active {
            if !before.contains_key(&id) {
                assert!(position(&scene.instances[stream.first + slot]).distance(Vec2::new(previous.x, previous.z)) > visible_radius(1));
            }
        }
        assert!(scene.instances[stream.first..].iter().all(|instance|
            instance.flags & (LIVE_GRASS | render::scene::NO_SHADOW)
                == (LIVE_GRASS | render::scene::NO_SHADOW)));
    }

    #[test]
    fn residency_margin_covers_every_camera_position_inside_its_cache_tile() {
        // This guards the CPU/shader endpoint contract independent of the
        // actual path taken through a tile, including diagonal crossings.
        let maximum_anchor_error = TILE * std::f32::consts::FRAC_1_SQRT_2;
        for views in 1..=4 {
            assert!(radius(views) - maximum_anchor_error > visible_radius(views));
        }
    }

    fn reference_selection(stream: &LiveGroundCover, cameras: &[Vec3]) -> (Vec<Tuft>, usize) {
        // Deliberately retain the original full candidate scan as the oracle;
        // it does not use tile masks or their bounding-box classification.
        let views = cameras.len().clamp(1, 4);
        let centres: Vec<_> = cameras.iter().take(4)
            .filter(|c| c.is_finite() && c.x.abs() < 10_000_000.0 && c.z.abs() < 10_000_000.0)
            .map(|c| (Vec2::new((c.x / TILE).floor(), (c.z / TILE).floor()) + Vec2::splat(0.5)) * TILE).collect();
        let mut selected: Vec<_> = stream.tiles.values().flat_map(|tile| tile.tufts.iter()).copied()
            .filter(|tuft| centres.iter().any(|centre|
                Vec2::new(tuft.position.x, tuft.position.z).distance_squared(*centre) <= radius(views).powi(2))).collect();
        let requested = selected.len();
        if selected.len() > stream.capacity {
            let visible = |tuft: &Tuft| cameras.iter().take(4).any(|camera| camera.is_finite()
                && Vec2::new(camera.x, camera.z).distance_squared(Vec2::new(tuft.position.x, tuft.position.z)) < visible_radius(views).powi(2));
            selected.sort_unstable_by_key(|tuft| (!visible(tuft), hash(tuft.id, 71), tuft.id));
            selected.truncate(stream.capacity);
        }
        (selected, requested)
    }

    #[test]
    fn incremental_residency_matches_full_scan_through_view_changes_and_pool_recovery() {
        let chunks: Vec<_> = (-4..=4).flat_map(|z| (-4..=4).map(move |x| chunk((x, z), 5.0))).collect();
        let (mut scene, mut stream) = setup(&chunks, 70_000);
        let paths = [vec![Vec3::new(-2.0, 2.0, -2.0)], vec![Vec3::new(2.0, 2.0, -2.0)],
            vec![Vec3::new(6.0, 2.0, 2.0)],
            vec![Vec3::new(-70.0, 2.0, -70.0), Vec3::new(70.0, 2.0, -70.0), Vec3::new(-70.0, 2.0, 70.0), Vec3::new(70.0, 2.0, 70.0)],
            vec![Vec3::new(6.0, 2.0, 2.0)], vec![Vec3::new(6.0, 2.0, 2.0), Vec3::new(7.0, 2.0, 2.0)],
            vec![Vec3::new(130.0, 2.0, 90.0)], vec![], vec![Vec3::new(2.0, 2.0, -2.0)]];
        let mut had_cap = false;
        for cameras in paths {
            let before: HashMap<_, _> = stream.active.iter().map(|(&id, &slot)|
                (id, (slot, scene.instances[stream.first + slot].rows))).collect();
            stream.update(&mut scene, &cameras);
            let (expected, requested) = reference_selection(&stream, &cameras);
            assert_eq!(requested, stream.stats.requested);
            assert_eq!(stream.active.keys().copied().collect::<HashSet<_>>(), expected.iter().map(|tuft| tuft.id).collect());
            assert_eq!(stream.active.len() + stream.free.len(), stream.capacity);
            had_cap |= stream.stats.budget_clipped > 0;
            for (&id, &slot) in &stream.active {
                if let Some(&(previous_slot, rows)) = before.get(&id) {
                    assert_eq!(slot, previous_slot);
                    assert_eq!(scene.instances[stream.first + slot].rows, rows);
                }
            }
        }
        assert!(had_cap, "fixture must exercise the capped-to-incremental recovery path");
        assert_eq!(stream.stats.budget_clipped, 0);
    }

    #[test]
    #[ignore = "CPU timing fixture; run explicitly in release mode with --nocapture"]
    fn benchmark_incremental_residency() {
        let chunks: Vec<_> = (-2..=5).flat_map(|z| (-2..=5).map(move |x| chunk((x, z), 5.0))).collect();
        let (mut scene, mut stream) = setup(&chunks, 70_000);
        stream.update(&mut scene, &[Vec3::new(1.0, 2.0, 1.0)]);
        let mut updates = Vec::new();
        let mut mask_ms = 0.0;
        let mut full_scan_ms = 0.0;
        for step in 1..=64 {
            let camera = Vec3::new(1.0 + step as f32 * TILE, 2.0, 1.0 + (step / 8) as f32 * TILE);
            stream.update(&mut scene, &[camera]);
            assert_eq!(stream.stats.budget_clipped, 0);
            updates.push(stream.stats.last_update_ms);
            let centre = (Vec2::new((camera.x / TILE).floor(), (camera.z / TILE).floor()) + Vec2::splat(0.5)) * TILE;
            let started = Instant::now();
            let masked: usize = stream.tiles.values().map(|tile| tile.mask(&[centre], radius(1).powi(2)).count_ones() as usize).sum();
            std::hint::black_box(masked);
            mask_ms += started.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            let scanned = stream.tiles.values().flat_map(|tile| tile.tufts.iter()).filter(|tuft|
                Vec2::new(tuft.position.x, tuft.position.z).distance_squared(centre) <= radius(1).powi(2)).count();
            std::hint::black_box(scanned);
            full_scan_ms += started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(masked, scanned);
            assert_eq!(masked, stream.stats.active);
        }
        updates.sort_by(f64::total_cmp);
        println!("grass CPU 64 tile moves: active={}, update p50={:.3} ms p95={:.3} ms; selection-only mask={:.3} ms vs full-scan={:.3} ms total (no GPU/upload)",
            stream.stats.active, updates[32], updates[60], mask_ms, full_scan_ms);
    }
}
