//! The arena on screen: every chunk a hewn brick with Godot's rock surface
//! (worn bricks crack and sit askew, broken ones vanish with their collider),
//! roof beacons and their lights, scorch and blood marks on the stone, and
//! the debris occupancy grid kept in step with what still stands.
use super::effects::{EffectsView, OCC_VOXEL};
use super::{cube, glow, material, omni, srgb_to_linear};
use glam::{Affine3A, Quat, Vec3};
use growing_guns::shape::Kind;
use growing_guns::World;
use physics::{Physics, Static};
use render::debris::BurstSpec;
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

struct KindMeshes { floor: Stone, wall: Stone, building: Stone, dark: Stone, accent_a: Stone, accent_b: Stone, beacons: [usize; 3] }

impl KindMeshes {
    fn stone(&self, k: Kind) -> Stone {
        match k {
            Kind::Floor => self.floor,
            Kind::Wall => self.wall,
            Kind::Tower | Kind::Pillar => self.dark,
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
    /// Slot and collider of every arena chunk (None once broken).
    chunk_slots: Vec<Option<SlotId>>,
    statics: Vec<Option<Static>>,
    /// Wear level shown per chunk (0 intact, 1 cracked, 2 crumbling).
    shown_wear: Vec<u8>,
    marks: Marks,
    lights: Vec<LocalLight>,
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
        let kinds = KindMeshes {
            floor: stone(scene, palette::FLOOR, 0.9),
            wall: stone(scene, palette::WALL, 0.85),
            building: stone(scene, palette::BUILDING, 0.85),
            dark: stone(scene, palette::DARK, 0.9),
            accent_a: stone(scene, palette::ACCENT_A, 0.7),
            accent_b: stone(scene, palette::ACCENT_B, 0.7),
            beacons: BEACON_COLORS.map(|c| { let m = glow(scene, c, 6000.0); cube(scene, m) }),
        };
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
        ArenaView { slots, kinds, chunk_slots: Vec::new(), statics: Vec::new(), shown_wear: Vec::new(), marks, lights: Vec::new(), lava: Default::default() }
    }

    /// Fill the slots, colliders and debris grid from the world's arena (a
    /// new round). Physics bodies must already be cleared.
    pub fn set(&mut self, scene: &mut Scene, phys: &mut Physics, fx: &mut EffectsView, world: &World) {
        self.slots.clear(scene);
        self.marks.slots.clear(scene);
        self.marks.ring.clear();
        let a = &world.arena;
        // Chunks first (their handles are kept), then the stands.
        let mut alive: Vec<_> = a.chunks.iter().filter(|c| c.alive).map(|c| c.shape()).collect();
        alive.extend(a.stadium.collision());
        let mut handles = phys.set_statics(&alive).into_iter();
        self.statics = a.chunks.iter().map(|c| if c.alive { handles.next() } else { None }).collect();
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

    /// The stone of the chunk at a grid index (the floor's for the stands).
    pub fn stone_at(&self, world: &World, grid_index: Option<usize>) -> Stone {
        grid_index.and_then(|c| world.arena.chunk(c)).map_or(self.kinds.floor, |ch| self.kinds.stone(ch.kind))
    }

    /// A scorch mark on the stone a bullet hit.
    pub fn scorch(&mut self, scene: &mut Scene, xf: Affine3A, chunk: u32) { self.marks.place(scene, self.marks.scorch, xf, chunk) }

    /// A blood splat on the stone.
    pub fn blood(&mut self, scene: &mut Scene, xf: Affine3A, chunk: u32) { self.marks.place(scene, self.marks.blood, xf, chunk) }

    /// Mesh and transform for a chunk at a wear level.
    fn chunk_look(&self, c: &growing_guns::Chunk, wear: u8) -> (usize, Affine3A) {
        let st = self.kinds.stone(c.kind);
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
            // Carve the chunk out of the debris grid, then restore voxels its
            // neighbours still fill.
            let c = world.arena.chunks[i as usize];
            let occ = fx.debris.occupancy_mut();
            occ.fill_box(c.center, c.half, c.yaw, false);
            let r = c.shape().reach() + OCC_VOXEL;
            for j in world.arena.grid.query(glam::Vec2::new(c.center.x - r, c.center.z - r), glam::Vec2::new(c.center.x + r, c.center.z + r)) {
                let Some(n) = world.arena.chunk(j) else { continue };
                if (n.center.y - c.center.y).abs() <= n.half.y + c.half.y + OCC_VOXEL { occ.fill_box(n.center, n.half, n.yaw, true); }
            }
        }
        // Rubble from each blast (Godot `request_burst`): 70 shards per
        // broken chunk, 128 to 900, thrown from the blast centre.
        for burst in &b.bursts {
            let st = self.kinds.stone(burst.kind);
            let count = (burst.count * 70).clamp(128, 900);
            fx.debris.burst(scene, &BurstSpec::rubble(burst.center, burst.radius.max(1.0), count, &st.shards));
            fx.particles.burst(scene, burst.center, Vec3::Y, &Burst { mesh: fx.meshes.dust, count: 12, spread: 1.5, speed: [0.5, 3.0], size: [0.9, 0.3], life: [0.8, 1.6], ..Burst::dust() });
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

/// Godot's rock_surface on a brick: procedural pits, cracks and moss;
/// `damage` 0..1 brings out more and darker cracks.
fn rock(scene: &mut Scene, tint: [f32; 3], rough: f32, damage: f32) -> u32 {
    scene.add_material(Material { tint: srgb_to_linear(tint), layer: -1, roughness: rough, flags: render::scene::ROCK, scale: damage, metal: 0.0 })
}

fn stone(scene: &mut Scene, tint: [f32; 3], rough: f32) -> Stone {
    // Per-brick brightness variation (Godot's INSTANCE_CUSTOM) as four tints.
    let intact = [0.88, 0.96, 1.04, 1.12].map(|k| {
        let m = rock(scene, tint.map(|c| (c * k).min(1.0)), rough, 0.0);
        scene.add_mesh(&primitives::hewn_block(m, 3, 0.09, 0.0, 0).mesh())
    });
    let dark = |k: f32| tint.map(|c| c * k);
    let cm = rock(scene, dark(0.9), rough, 0.5);
    let cracked = scene.add_mesh(&primitives::hewn_block(cm, 4, 0.11, 0.07, 1).mesh());
    let km = rock(scene, dark(0.8), rough, 1.0);
    let crumbling = scene.add_mesh(&primitives::hewn_block(km, 4, 0.14, 0.16, 2).mesh());
    // Rubble: rough shards in three shades (Godot: base colour x 0.72..1.17).
    let shards = [0.72, 0.92, 1.12].map(|k| {
        let m = material(scene, tint.map(|c| (c * k).min(1.0)), rough);
        scene.add_mesh(&primitives::hewn_block(m, 2, 0.15, 0.22, (k * 100.0) as u32).mesh())
    });
    Stone { intact, cracked, crumbling, shards }
}
