//! The arena on screen: every chunk a hewn brick with Godot's rock surface
//! (worn bricks crack and sit askew, broken ones vanish with their collider),
//! roof beacons and their lights, scorch and blood marks on the stone, and
//! the debris occupancy grid kept in step with what still stands.
use super::effects::{EffectsView, OCC_VOXEL};
use super::{cube, glow, material, omni, srgb_to_linear};
use glam::{Affine3A, Quat, Vec2, Vec3};
use growing_guns::shape::Kind;
use growing_guns::World;
use kern::collide::{BoxGridScratch, YawBox};
use physics::{Physics, Static, StaticSet};
use render::debris::{BurstSpec, Occupancy};
use render::headlights::LocalLight;
use render::particles::Burst;
use render::primitives;
use render::scene::Material;
use render::slots::{SlotId, Slots};
use render::Scene;

/// Godot palette "castle_grey" (linear-ish albedo, as authored there).
pub mod palette {
    pub const FLOOR: [f32; 3] = [0.49, 0.47, 0.44];
    pub const WALL: [f32; 3] = [0.56, 0.54, 0.50];
    pub const BUILDING: [f32; 3] = [0.60, 0.57, 0.53];
    pub const DARK: [f32; 3] = [0.42, 0.40, 0.38];
    pub const ACCENT_A: [f32; 3] = [0.95, 0.55, 0.22];
    pub const ACCENT_B: [f32; 3] = [0.82, 0.72, 0.38];
    pub const SKY_TOP: [f32; 3] = [0.22, 0.24, 0.28];
    pub const HORIZON: [f32; 3] = [0.48, 0.46, 0.44];
    pub const SUN: [f32; 3] = [1.0, 0.82, 0.58];

    /// Godot `_inferno_palette`'s atmosphere for island arenas: sky, sun
    /// (energy 2.55 against the castle grey's 2.0).
    pub const INFERNO_SKY_TOP: [f32; 3] = [0.06, 0.05, 0.09];
    pub const INFERNO_HORIZON: [f32; 3] = [0.58, 0.24, 0.10];
    pub const INFERNO_SUN: [f32; 3] = [1.0, 0.72, 0.38];
    pub const INFERNO_SUN_ENERGY: f32 = 2.55 / 2.0;

    /// Godot `_inferno_palette`, the island arenas' stone: the castle grey
    /// pulled towards a dark basalt.
    pub fn inferno(base: [f32; 3]) -> [f32; 3] {
        const STONE: [f32; 3] = [0.30, 0.28, 0.26];
        let lighten = |a: f32| STONE.map(|c| c + (1.0 - c) * a);
        let (to, t) = if base == FLOOR { (STONE, 0.62) } else if base == WALL { (lighten(0.08), 0.68) }
            else if base == BUILDING { (lighten(0.12), 0.64) } else if base == DARK { ([0.20, 0.18, 0.17], 0.72) } else { return base };
        [0, 1, 2].map(|k| base[k] + (to[k] - base[k]) * t)
    }
}

/// Godot's hairline mortar gap between bricks.
const VISUAL_CHUNK_INSET: f32 = 0.992;
const MARKS: usize = 384;
/// The largest generated arena has about 9400 chunks.
const ARENA_SLOTS: usize = 9800;
/// The Godot arena's light colours (towers and building roofs).
const BEACON_COLORS: [[f32; 3]; 3] = [[1.0, 0.4, 0.3], [1.0, 0.35, 0.2], [1.0, 0.7, 0.2]];

/// Stone per palette colour: four tint variants of the intact brick, then
/// cracked and crumbling bricks for worn chunks, and rubble shards.
#[derive(Clone, Copy)]
pub struct Stone { intact: [usize; 4], cracked: usize, crumbling: usize, pub shards: [usize; 3] }

#[derive(Clone, Copy)]
struct KindMeshes { floor: Stone, wall: Stone, building: Stone, dark: Stone, accent_a: Stone, accent_b: Stone, beacons: [usize; 3] }

impl KindMeshes {
    fn stone(&self, k: Kind) -> Stone {
        match k {
            Kind::Floor => self.floor,
            Kind::Wall => self.wall,
            Kind::Tower | Kind::Pillar | Kind::Island => self.dark,
            Kind::Building => self.building,
            Kind::Ledge | Kind::Platform => self.accent_b,
            Kind::Cover => self.accent_a,
        }
    }
}

/// Scorch and blood marks: a ring of slots, oldest reused first; each knows
/// the chunk it sits on and goes when that chunk breaks.
struct Marks { slots: Slots, ring: Vec<(SlotId, u32)>, next: usize, scorch: usize, blood: usize }

pub struct ArenaView {
    slots: Slots,
    kinds: KindMeshes,
    /// The same in the inferno palette, for island arenas.
    inferno: KindMeshes,
    island: bool,
    /// Slot and collider of every arena chunk (None once broken).
    chunk_slots: Vec<Option<SlotId>>,
    statics: Vec<Option<Static>>,
    static_set: StaticSet,
    static_shapes: Vec<Option<YawBox>>,
    /// Wear level shown per chunk (0 intact, 1 cracked, 2 crumbling).
    shown_wear: Vec<u8>,
    marks: Marks,
    lights: Vec<LocalLight>,
    occupancy_repair: OccupancyRepair,
    /// The light the lava casts.
    pub lava: super::lava::LavaLight,
}

/// The lava's radiance for a white pixel, per channel. Godot shows the
/// unshaded albedo through Filmic at exposure 1.08 (and glow); under the
/// engine's AgX that saturation is out of reach, so this is the best fit to
/// the Godot captures (docs/lava-parity.md): a bright, red-leaning white
/// with no blue, which AgX's channel crosstalk fills in.
pub const LAVA_NITS: f32 = 45000.0;
pub const LAVA_TINT: [f32; 3] = [1.0, 0.92, 0.0];

/// The flowing lava (Godot `make_lava_shader_material`).
pub fn lava_material(scene: &mut Scene) -> u32 {
    scene.add_material(Material { tint: LAVA_TINT, layer: -1, roughness: 1.0, flags: render::scene::LAVA, scale: LAVA_NITS, metal: 0.0 })
}

impl ArenaView {
    /// Brick meshes, the lava sheet, the arena slots and the marks.
    pub fn reserve(scene: &mut Scene) -> Self {
        let beacons = BEACON_COLORS.map(|c| { let m = glow(scene, c, 6000.0); cube(scene, m) });
        let set = |scene: &mut Scene, tint: fn([f32; 3]) -> [f32; 3]| KindMeshes {
            floor: stone(scene, tint(palette::FLOOR), 0.9),
            wall: stone(scene, tint(palette::WALL), 0.85),
            building: stone(scene, tint(palette::BUILDING), 0.85),
            dark: stone(scene, tint(palette::DARK), 0.9),
            accent_a: stone(scene, tint(palette::ACCENT_A), 0.7),
            accent_b: stone(scene, tint(palette::ACCENT_B), 0.7),
            beacons,
        };
        let kinds = set(scene, |c| c);
        let inferno = set(scene, palette::inferno);
        // Lava: Godot's flowing lava under the floor, visible through holes
        // and past the edge of open arenas.
        // Its surface is at the kill height (Godot LAVA_POOL_Y), level with
        // the bottom of the floor.
        let lava_m = lava_material(scene);
        let lava = cube(scene, lava_m);
        scene.add_instance(lava, Affine3A::from_scale_rotation_translation(Vec3::new(900.0, 0.2, 900.0), Quat::IDENTITY, Vec3::new(0.0, growing_guns::arena::LAVA_KILL_Y - 0.1, 0.0)));
        scene.mark_no_shadow();
        // The arena lives in slots: a new round refills them, no rebuild.
        let slots = Slots::reserve(scene, ARENA_SLOTS);
        let scorch_m = material(scene, [0.06, 0.055, 0.05], 0.95);
        let blood_m = material(scene, [0.36, 0.02, 0.015], 0.35);
        let marks = Marks { slots: Slots::reserve(scene, MARKS), ring: Vec::new(), next: 0, scorch: super::sphere(scene, scorch_m, 4, 8), blood: super::sphere(scene, blood_m, 4, 10) };
        ArenaView { slots, kinds, inferno, island: false, chunk_slots: Vec::new(), statics: Vec::new(), static_set: Default::default(), static_shapes: Vec::new(), shown_wear: Vec::new(), marks, lights: Vec::new(), occupancy_repair: Default::default(), lava: Default::default() }
    }

    /// Fill the slots, colliders and debris grid from the world's arena (a
    /// new round). Physics bodies must already be cleared.
    pub fn set(&mut self, scene: &mut Scene, phys: &mut Physics, fx: &mut EffectsView, world: &World) {
        self.slots.clear(scene);
        self.marks.slots.clear(scene);
        self.marks.ring.clear();
        let a = &world.arena;
        self.island = a.variant == growing_guns::arena::Variant::Island;
        // Keep chunk indices (including holes) stable, then the stands. An
        // unchanged collider survives the reset; destroyed handles are restored.
        self.static_shapes.clear();
        self.static_shapes.extend(a.chunks.iter().map(|c| c.alive.then(|| c.shape())));
        self.static_shapes.extend(a.stadium.collision().into_iter().map(Some));
        let handles = self.static_set.sync(phys, &self.static_shapes);
        self.statics.clear();
        self.statics.extend_from_slice(&handles[..a.chunks.len()]);
        self.shown_wear = a.chunks.iter().map(|c| wear_level(c.wear())).collect();
        fx.debris.clear();
        let occ = fx.debris.occupancy_mut();
        occ.clear();
        for c in a.chunks.iter().filter(|c| c.alive) { occ.fill_box(c.center, c.half, c.yaw, true); }
        self.chunk_slots = Vec::with_capacity(a.chunks.len());
        for (i, c) in a.chunks.iter().enumerate() {
            let slot = c.alive.then(|| {
                let (mesh, xf) = self.chunk_look(c, self.shown_wear[i]);
                self.slots.add(scene, mesh, xf).expect("arena slots")
            });
            self.chunk_slots.push(slot);
        }
        // Roof beacons where the Godot lights hang, in the nearest colour.
        for &(p, c) in &a.lights {
            let nearest = (0..BEACON_COLORS.len()).min_by(|&x, &y| dist(BEACON_COLORS[x], c).total_cmp(&dist(BEACON_COLORS[y], c))).unwrap();
            self.slots.add(scene, self.kinds.beacons[nearest], Affine3A::from_scale_rotation_translation(Vec3::splat(0.5), Quat::IDENTITY, p)).expect("arena slots");
        }
        self.lights = a.lights.iter().take(12).map(|&(p, c)| omni(p, Vec3::from(c) * 900.0, 20.0)).collect();
        self.lava.set(world);
    }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene) -> Result<(), String> {
        self.slots.flush(gpu, renderer, scene)?;
        self.marks.slots.flush(gpu, renderer, scene)
    }

    pub fn lights(&self) -> &[LocalLight] { &self.lights }

    /// The stone set of this arena (the inferno one on islands).
    fn kinds(&self) -> &KindMeshes { if self.island { &self.inferno } else { &self.kinds } }

    /// The stone of the chunk at a grid index (the floor's for the stands).
    pub fn stone_at(&self, world: &World, grid_index: Option<usize>) -> Stone {
        grid_index.and_then(|c| world.arena.chunk(c)).map_or(self.kinds().floor, |ch| self.kinds().stone(ch.kind))
    }

    /// A scorch mark on the stone a bullet hit.
    pub fn scorch(&mut self, scene: &mut Scene, xf: Affine3A, chunk: u32) { self.marks.place(scene, self.marks.scorch, xf, chunk) }

    /// A blood splat on the stone.
    pub fn blood(&mut self, scene: &mut Scene, xf: Affine3A, chunk: u32) { self.marks.place(scene, self.marks.blood, xf, chunk) }

    /// Mesh and transform for a chunk at a wear level.
    fn chunk_look(&self, c: &growing_guns::Chunk, wear: u8) -> (usize, Affine3A) {
        let st = self.kinds().stone(c.kind);
        let mesh = match wear { 0 => st.intact[(c.seed % 4) as usize], 1 => st.cracked, _ => st.crumbling };
        // Worn bricks shrink a little and sit askew, like Godot's deform carve.
        let shrink = [1.0, 0.97, 0.92][wear as usize];
        let tilt = if wear == 0 { Quat::IDENTITY } else {
            let r = |k: u32| ((c.seed >> k) & 0xff) as f32 / 255.0 - 0.5;
            Quat::from_euler(glam::EulerRot::XYZ, r(0) * 0.05 * wear as f32, r(8) * 0.05 * wear as f32, r(16) * 0.05 * wear as f32)
        };
        // The mortar gap is horizontal only: courses sit flush, so the bright
        // lava never shows through a seam under a wall.
        (mesh, Affine3A::from_scale_rotation_translation(c.half * 2.0 * Vec3::new(VISUAL_CHUNK_INSET, 1.0, VISUAL_CHUNK_INSET) * shrink, Quat::from_rotation_y(c.yaw) * tilt, c.center))
    }

    /// This tick's breakage: worn bricks change look, broken ones and their
    /// marks and colliders go, rubble flies.
    pub fn break_chunks(&mut self, scene: &mut Scene, phys: &mut Physics, fx: &mut EffectsView, world: &World) {
        let b = &world.breakage;
        if b.is_empty() { return; }
        self.lava.broken(world, &b.removed);
        for &(i, _) in &b.worn {
            let c = &world.arena.chunks[i as usize];
            let lvl = wear_level(c.wear());
            if lvl != self.shown_wear[i as usize] {
                self.shown_wear[i as usize] = lvl;
                if let Some(slot) = self.chunk_slots[i as usize] {
                    let (mesh, xf) = self.chunk_look(c, lvl);
                    self.slots.set(scene, slot, mesh, xf);
                }
            }
        }
        for &i in &b.removed {
            if let Some(slot) = self.chunk_slots[i as usize].take() { self.slots.remove(scene, slot); }
            if let Some(h) = self.statics[i as usize].take() { phys.remove_static(h); }
            self.marks.drop_on(scene, i);
        }
        self.occupancy_repair.apply(fx.debris.occupancy_mut(), &world.arena, &b.removed);
        // Rubble from each blast (Godot `request_burst`): 70 shards per
        // broken chunk, 128 to 900, thrown from the blast centre.
        for burst in &b.bursts {
            let st = self.kinds().stone(burst.kind);
            let count = (burst.count * 70).clamp(128, 900);
            fx.debris.burst(scene, &BurstSpec::rubble(burst.center, burst.radius.max(1.0), count, &st.shards));
            fx.particles.burst(scene, burst.center, Vec3::Y, &Burst { mesh: fx.meshes.dust, count: 12, spread: 1.5, speed: [0.5, 3.0], size: [0.9, 0.3], life: [0.8, 1.6], ..Burst::dust() });
        }
    }
}

/// Restore each surviving neighbour once after carving all removed chunks.
/// Filling solid voxels commutes; the final occupancy is the same as restoring
/// neighbours after every carve, including boxes that overlap several holes.
#[derive(Default)]
struct OccupancyRepair {
    query: BoxGridScratch,
    seen: Vec<bool>,
    neighbours: Vec<usize>,
}

impl OccupancyRepair {
    fn apply(&mut self, occ: &mut Occupancy, arena: &growing_guns::Arena, removed: &[u32]) {
        for &i in &self.neighbours { self.seen[i] = false; }
        self.neighbours.clear();
        if removed.is_empty() { return; }
        if self.seen.len() < arena.chunks.len() { self.seen.resize(arena.chunks.len(), false); }
        for &i in removed {
            let c = &arena.chunks[i as usize];
            occ.fill_box(c.center, c.half, c.yaw, false);
            let r = c.shape().reach() + OCC_VOXEL;
            for &j in arena.grid.query_with_scratch(Vec2::new(c.center.x - r, c.center.z - r), Vec2::new(c.center.x + r, c.center.z + r), &mut self.query) {
                let Some(n) = arena.chunk(j) else { continue };
                if !self.seen[j] && (n.center.y - c.center.y).abs() <= n.half.y + c.half.y + OCC_VOXEL {
                    self.seen[j] = true;
                    self.neighbours.push(j);
                }
            }
        }
        for &i in &self.neighbours {
            let n = &arena.chunks[i];
            occ.fill_box(n.center, n.half, n.yaw, true);
        }
    }
}

impl Marks {
    fn place(&mut self, scene: &mut Scene, mesh: usize, xf: Affine3A, chunk: u32) {
        if self.ring.len() < self.slots.capacity() {
            if let Some(id) = self.slots.add(scene, mesh, xf) { self.ring.push((id, chunk)); }
            return;
        }
        let k = self.next % self.ring.len();
        self.next += 1;
        let id = self.ring[k].0;
        if self.slots.is_used(id) { self.slots.set(scene, id, mesh, xf); } else if let Some(n) = self.slots.add(scene, mesh, xf) { self.ring[k].0 = n; }
        self.ring[k].1 = chunk;
    }

    fn drop_on(&mut self, scene: &mut Scene, chunk: u32) {
        for (id, c) in &mut self.ring {
            if *c == chunk && self.slots.is_used(*id) { self.slots.remove(scene, *id); *c = u32::MAX; }
        }
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|i| (a[i] - b[i]).powi(2)).sum() }

fn wear_level(w: f32) -> u8 { if w <= 0.08 { 0 } else if w < 0.5 { 1 } else { 2 } }

/// Shared textured stone, filtered by the screen footprint. Damage remains
/// independent of the recipe and keeps the existing worn/crumbling shapes.
fn rock(scene: &mut Scene, tint: [f32; 3], rough: f32, damage: f32) -> u32 {
    let layer = super::stone_layer(scene);
    scene.add_material(Material { tint: srgb_to_linear(tint), layer, roughness: rough, flags: render::scene::ROCK, scale: damage, metal: 0.0 })
}

fn stone(scene: &mut Scene, tint: [f32; 3], rough: f32) -> Stone {
    let legacy = super::stone_layer(scene) < 0;
    // Per-brick brightness variation (Godot's INSTANCE_CUSTOM) as four tints.
    let intact = [0.88, 0.96, 1.04, 1.12].map(|k| {
        let m = rock(scene, tint.map(|c| (c * k).min(1.0)), rough, 0.0);
        let shape = if legacy { primitives::hewn_block(m, 3, 0.09, 0.0, 0) }
            else { render::quarried_block::quarried_block(m, 0.0, (k * 12.5) as u32) };
        scene.add_mesh(&shape.mesh())
    });
    let dark = |k: f32| tint.map(|c| c * k);
    let cm = rock(scene, dark(0.9), rough, 0.5);
    let cracked_shape = if legacy { primitives::hewn_block(cm, 4, 0.11, 0.07, 1) }
        else { render::quarried_block::quarried_block(cm, 0.5, 1) };
    let cracked = scene.add_mesh(&cracked_shape.mesh());
    let km = rock(scene, dark(0.8), rough, 1.0);
    let crumbling_shape = if legacy { primitives::hewn_block(km, 4, 0.14, 0.16, 2) }
        else { render::quarried_block::quarried_block(km, 1.0, 1) };
    let crumbling = scene.add_mesh(&crumbling_shape.mesh());
    // Rubble: rough shards in three shades (Godot: base colour x 0.72..1.17).
    let shards = [0.72, 0.92, 1.12].map(|k| {
        let shard_tint = tint.map(|c| (c * k).min(1.0));
        let m = if super::stone_layer(scene) < 0 { material(scene, shard_tint, rough) }
            else { rock(scene, shard_tint, rough, 0.35) };
        scene.add_mesh(&primitives::hewn_block(m, 2, 0.15, 0.22, (k * 100.0) as u32).mesh())
    });
    Stone { intact, cracked, crumbling, shards }
}

#[cfg(test)]
mod tests {
    use super::*;
    use growing_guns::{Arena, Breakage, Chunk};
    use growing_guns::destruct::Carve;
    use growing_guns::rng::Rng;
    use growing_guns::shape::{BoxGrid, YawBox};

    fn fixture(seed: u64) -> Arena {
        let mut a = Arena::empty();
        let mut rng = Rng::new(seed);
        for i in 0..96 {
            a.chunks.push(Chunk {
                center: Vec3::new(rng.range(-14.0, 14.0), rng.range(-4.0, 9.0), rng.range(-14.0, 14.0)),
                half: Vec3::new(rng.range(0.2, 2.7), rng.range(0.2, 1.5), rng.range(0.2, 2.7)),
                yaw: rng.range(-3.0, 3.0), kind: Kind::Wall, volume: i / 4,
                max_hp: 50.0, hp: 50.0, alive: true, seed: i,
            });
        }
        // Several removed boxes share the same surviving, rotated neighbour.
        for (i, c) in a.chunks.iter_mut().take(8).enumerate() {
            c.center = Vec3::new(i as f32 * 0.2, 0.0, 0.0);
            c.half = Vec3::new(2.0, 0.75, 1.5);
        }
        let mut shapes: Vec<_> = a.chunks.iter().map(Chunk::shape).collect();
        // Grid entries after the chunks are stadium colliders, not debris solids.
        shapes.push(YawBox::new(Vec3::ZERO, Vec3::splat(5.0), 0.5));
        a.grid = BoxGrid::new(shapes, 4.0);
        a
    }

    fn occupancy(a: &Arena) -> Occupancy {
        let mut o = Occupancy::new(Vec3::new(-12.0, -5.0, -12.0), Vec3::new(12.0, 12.0, 12.0), OCC_VOXEL);
        for c in a.chunks.iter().filter(|c| c.alive) { o.fill_box(c.center, c.half, c.yaw, true); }
        o
    }

    /// The pre-batching algorithm: clear and restore once per removed chunk.
    fn sequential(o: &mut Occupancy, a: &Arena, removed: &[u32]) -> usize {
        let mut fills = 0;
        for &i in removed {
            let c = &a.chunks[i as usize];
            o.fill_box(c.center, c.half, c.yaw, false);
            let r = c.shape().reach() + OCC_VOXEL;
            for j in a.grid.query(Vec2::new(c.center.x - r, c.center.z - r), Vec2::new(c.center.x + r, c.center.z + r)) {
                let Some(n) = a.chunk(j) else { continue };
                if (n.center.y - c.center.y).abs() <= n.half.y + c.half.y + OCC_VOXEL {
                    o.fill_box(n.center, n.half, n.yaw, true);
                    fills += 1;
                }
            }
        }
        fills
    }

    #[test]
    fn occupancy_batch_matches_sequential_after_overlapping_rotated_carves_and_resets() {
        let mut repair = OccupancyRepair::default();
        let mut total_removed = 0;
        for seed in [1, 19, 482] {
            let original = fixture(seed);
            let mut a = original.clone();
            let mut expected = occupancy(&a);
            let mut actual = expected.clone();
            let mut rng = Rng::new(seed ^ 0x1234);
            for tick in 0..20 {
                if tick == 10 {
                    a = original.clone();
                    expected = occupancy(&a);
                    actual = expected.clone();
                }
                let at = a.chunks[rng.below(a.chunks.len() as u32) as usize].center;
                let mut removed = a.carve(&[
                    Carve { pos: at, radius: 5.5, damage: 900.0 },
                    Carve { pos: at + Vec3::new(1.0, 0.3, -0.5), radius: 4.0, damage: 900.0 },
                ]).removed;
                if tick % 3 == 0 { removed.extend_from_within(..); removed.reverse(); }
                total_removed += removed.len();
                sequential(&mut expected, &a, &removed);
                repair.apply(&mut actual, &a, &removed);
                assert_eq!(actual.bits, expected.bits, "seed {seed}, tick {tick}");
            }
            repair.apply(&mut actual, &a, &[]);
            assert_eq!(actual.bits, expected.bits, "empty batch");
        }
        assert!(total_removed > 100, "exercise destruction, not just empty updates");
    }

    #[test]
    fn occupancy_batch_restores_shared_neighbours_once_and_reuses_capacity() {
        let mut a = fixture(4);
        let mut expected = occupancy(&a);
        let mut actual = expected.clone();
        let removed = [0, 1, 2, 0, 1];
        a.apply_breakage(&Breakage { removed: removed.to_vec(), ..Default::default() });
        let old_fills = sequential(&mut expected, &a, &removed);
        let mut repair = OccupancyRepair::default();
        repair.apply(&mut actual, &a, &removed);
        assert_eq!(actual.bits, expected.bits);
        assert!(repair.neighbours.len() * 2 < old_fills, "shared neighbours should be restored once");
        let capacities = (repair.seen.capacity(), repair.neighbours.capacity());
        for _ in 0..8 {
            repair.apply(&mut actual, &a, &removed);
            assert_eq!(actual.bits, expected.bits);
            assert_eq!((repair.seen.capacity(), repair.neighbours.capacity()), capacities);
        }
        // Reuse the same scratch after switching to an empty/smaller arena.
        let empty = Arena::empty();
        repair.apply(&mut actual, &empty, &[]);
        a.chunks.truncate(4);
        a.grid = BoxGrid::new(a.chunks.iter().map(Chunk::shape).collect(), 4.0);
        for i in [0, 1, 2] { a.grid.remove(i); }
        expected = occupancy(&a);
        actual = expected.clone();
        sequential(&mut expected, &a, &removed);
        repair.apply(&mut actual, &a, &removed);
        assert_eq!(actual.bits, expected.bits, "smaller arena");
    }
}
