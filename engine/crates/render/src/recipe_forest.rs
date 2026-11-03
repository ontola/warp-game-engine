//! A forest of recipe plants, the way every game draws one: a few cooked
//! variants of a recipe are shared by every tree; the trees nearest the
//! camera are full assemblies (branch groups pick source or voxels by
//! projected error), every other tree is the same plant as one whole-plant
//! voxel proxy (docs/voxel-raster-plan.md); in between, card trees
//! (plant_mesh_lod: foliage as quads, wood as geometry; add_cards) take the
//! middle distance. Cooks run in parallel and are
//! cached on disk (CookedPlant::cached), so a warm start loads in
//! milliseconds.

use crate::aggregate_lod::Policy;
use crate::plant_mesh_lod::PlantCards;
use crate::plant_recipe::{self, BuildOptions, WoodyRecipe};
use crate::scene::{GeometryCategory, GpuInstance};
use crate::vegetation_aggregate::Config;
use crate::vegetation_asset::{AttachLimits, AttachmentReport, CookedPlant, MovablePlant, PlantInstance, ProxyShow, SceneVegetation};
use crate::{Gpu, Renderer, Scene};
use glam::{Affine3A, Mat4, Quat, Vec3};
use std::path::Path;

/// Levels of the whole-plant proxy: 10 cm cells, then 20 cm .. 3.2 m.
pub const PROXY_LEVELS: u32 = 6;

/// The aggregate cook every recipe tree uses: each branch group also gets
/// voxel levels.
pub fn aggregate_config() -> Config {
    Config { max_placements: 8192, max_source_triangles: 40_000_000, max_samples: 150_000_000, triangle_refinements: true,
        max_reference_voxels: 262_144, max_diagnostic_steps: 80_000_000, ..Default::default() }
}

/// The whole-plant proxy cook: 10 cm reference cells.
pub fn proxy_config() -> Config {
    Config { reference_cell_size: 0.1, max_reference_voxels: 1 << 21, max_build_bytes: 1 << 30, ..aggregate_config() }
}

/// One cooked variant and whether it came from the cache.
pub struct Cooked {
    pub plant: CookedPlant,
    pub loaded: bool,
    pub millis: f64,
}

/// Cook each (recipe, seed) variant in parallel, each with its whole-plant
/// proxy. With `cache`, a variant cooked before with the same recipe, seed,
/// configs and engine cook code loads from disk instead.
pub fn cook_variants(variants: &[(WoodyRecipe, u64)], cache: Option<&Path>) -> Result<Vec<Cooked>, String> {
    let (config, proxy) = (aggregate_config(), proxy_config());
    std::thread::scope(|scope| {
        let jobs: Vec<_> = variants.iter().map(|(recipe, seed)| scope.spawn(move || -> Result<Cooked, String> {
            let seed = *seed;
            let started = std::time::Instant::now();
            let cook = || {
                let assembly = plant_recipe::build(recipe, BuildOptions::new(seed), plant_recipe::recipe_limits())?;
                let mut plant = CookedPlant::cook_aggregate(&assembly, config)?;
                plant.cook_proxy(&assembly, proxy, PROXY_LEVELS)?;
                Ok(plant)
            };
            let (plant, loaded) = match cache {
                Some(dir) => {
                    let what = format!("{recipe:?} {:?} {:?} {config:?} {proxy:?} {PROXY_LEVELS}",
                        BuildOptions::new(seed), plant_recipe::recipe_limits());
                    CookedPlant::cached(dir, &what, cook)?
                }
                None => (cook()?, false),
            };
            Ok(Cooked { plant, loaded, millis: started.elapsed().as_secs_f64() * 1000.0 })
        })).collect();
        jobs.into_iter().map(|job| job.join().map_err(|_| "recipe cook panicked".to_string())?).collect()
    })
}

/// Wood of the card tier is cut at this error (metres); its foliage is cards.
pub const CARD_WOOD_ERROR_M: f32 = 0.03;

/// Each (recipe, seed) variant as cards (plant_mesh_lod): the middle
/// distance's mesh, between the full trees and the proxies. With `cache`,
/// loaded like the plants.
pub fn cook_cards(variants: &[(WoodyRecipe, u64)], cache: Option<&Path>) -> Result<Vec<PlantCards>, String> {
    std::thread::scope(|scope| {
        let jobs: Vec<_> = variants.iter().map(|(recipe, seed)| scope.spawn(move || -> Result<PlantCards, String> {
            let cook = || {
                let assembly = plant_recipe::build(recipe, BuildOptions::new(*seed), plant_recipe::recipe_limits())?;
                PlantCards::cook(&assembly, CARD_WOOD_ERROR_M)
            };
            match cache {
                Some(dir) => Ok(PlantCards::cached(dir, &format!("{recipe:?} {:?} {:?} {CARD_WOOD_ERROR_M}",
                    BuildOptions::new(*seed), plant_recipe::recipe_limits()), cook)?.0),
                None => cook(),
            }
        })).collect();
        jobs.into_iter().map(|job| job.join().map_err(|_| "card cook panicked".to_string())?).collect()
    })
}

/// One tree: its foot, turn about +Y, uniform scale and cooked variant.
#[derive(Clone, Copy, Debug)]
pub struct Tree {
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
    pub variant: usize,
}

/// Put `trees` into `scene`: the first `full` as full assemblies, the rest
/// as whole-plant proxies (pass them nearest first). `policy` sets the
/// screen error of both. Stable plant ids are the trees' indices + 1.
pub fn plant(scene: &mut Scene, variants: &[CookedPlant], trees: &[Tree], full: usize, policy: Policy) -> Result<AttachmentReport, String> {
    if trees.is_empty() { return Ok(AttachmentReport::default()); }
    if let Some(tree) = trees.iter().find(|t| t.variant >= variants.len()) {
        return Err(format!("recipe forest variant {} of {}", tree.variant, variants.len()));
    }
    let full = full.min(trees.len());
    let mut vegetation = SceneVegetation::new(scene, limits(variants, full))?;
    for (i, tree) in trees[..full].iter().enumerate() { vegetation.append(&variants[tree.variant], instance(i, tree))?; }
    for (i, tree) in trees[full..].iter().enumerate() {
        vegetation.append_proxy(&variants[tree.variant], instance(full + i, tree))?;
    }
    vegetation.finish_aggregate(policy)
}

static VIRTUAL_MEMBERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Full plants as virtual members (vegetation_asset AttachLimits): a game's
/// choice (set_virtual_members), or KERN_VIRTUAL_MEMBERS=1/0 to force it.
pub fn virtual_members() -> bool {
    match std::env::var("KERN_VIRTUAL_MEMBERS").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => VIRTUAL_MEMBERS.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// Turn virtual members on for plants planted from now on.
pub fn set_virtual_members(on: bool) { VIRTUAL_MEMBERS.store(on, std::sync::atomic::Ordering::Relaxed); }

fn limits(variants: &[CookedPlant], full: usize) -> AttachLimits {
    let branches = variants.iter().map(|c| c.stats().branches).max().unwrap_or(64);
    // KERN_VIRTUAL_MEMBERS=1: placements as shared templates composed with
    // their branch pose on the GPU (docs/foliage-assemblies-plan.md).
    let virtual_members = virtual_members();
    let mut limits = AttachLimits { virtual_members, max_poses: branches.max(64) * full.max(4),
        max_primitive_instances: crate::part_wind::MAX_ATTACHMENTS, ..Default::default() };
    limits.pack.max_instances = limits.max_poses.min(crate::foliage_volume_data::MAX_INSTANCES);
    // Every variant brings its branch volumes and its proxy: a mixed forest
    // of six species needs more than the default 256.
    let assets: usize = variants.iter().map(|c| c.stats().volume_assets + 1).sum();
    limits.pack.max_assets = limits.pack.max_assets.max(assets);
    limits
}

fn root(t: &Tree) -> Mat4 {
    Mat4::from_scale_rotation_translation(Vec3::splat(t.scale), Quat::from_rotation_y(t.yaw), t.pos)
}

fn instance(i: usize, t: &Tree) -> PlantInstance {
    PlantInstance { id: i as u64 + 1, generation: 0, root: root(t), category: GeometryCategory::Trees }
}

/// Legacy CPU-assigned detail pool (not GPU assembly streaming): every
/// tree is its whole-plant proxy, and a fixed pool of full assemblies (`slots`
/// per variant) moves to the trees nearest the cameras, hiding their proxies
/// (Renderer::move_plant, set_plant_proxies). A full tree still picks
/// per frame on the GPU between its branches and its own voxels.
pub struct Pool {
    trees: Vec<Tree>,
    proxy_templates: Vec<Option<(Tree, crate::foliage_volume_data::GpuTree, GpuInstance)>>,
    proxy_owners: Vec<u32>,
    slot_scales: Vec<f32>,
    slots: Vec<Slot>,
    /// Forest volume index of tree 0's proxy; the rest follow in order.
    first_proxy: u32,
    by_variant: Vec<Vec<usize>>,
    last: Vec<Vec3>,
    cards: Option<Cards>,
    /// Trees whose proxies are hidden now (a full slot or a card stands there).
    hidden: std::collections::HashMap<usize, ProxyShow>,
}

struct Slot { variant: usize, plant: MovablePlant, tree: Option<usize> }

/// The card tier (add_cards): a fixed set of card instances per variant that
/// Pool::update gives to the nearest trees past the full slots, within `radius`.
struct Cards {
    meshes: Vec<crate::scene::MeshRef>,
    first: usize,
    slots: Vec<(usize, Option<usize>)>,
    radius: f32,
}

/// Where unused card instances wait: far below any ground, outside every view.
const PARKED: Vec3 = Vec3::new(0.0, -100_000.0, 0.0);

fn card_instance(mesh: &crate::scene::MeshRef, root: Mat4) -> GpuInstance {
    let xf = Affine3A::from_mat4(root);
    let c = xf.transform_point3(Vec3::from_slice(&mesh.sphere[..3]));
    let scale = xf.matrix3.x_axis.length().max(xf.matrix3.y_axis.length()).max(xf.matrix3.z_axis.length());
    let r = root.transpose();
    let mut instance = GpuInstance { rows: [r.x_axis.to_array(), r.y_axis.to_array(), r.z_axis.to_array()],
        sphere: [c.x, c.y, c.z, mesh.sphere[3] * scale], first_cluster: mesh.first_cluster, cluster_count: mesh.cluster_count, scale,
        // The tree's proxy casts its shadow (ProxyShow::ShadowOnly).
        flags: crate::scene::NO_SHADOW };
    instance.set_category(GeometryCategory::Trees);
    instance
}

/// Add the card tier to `pool`: `cards` (one per variant, cook_cards) into
/// `scene` as `per_variant` card instances each, which Pool::update gives to
/// the trees nearest the cameras past the full slots, within `radius_m`,
/// hiding their proxies. Call before the renderer is built.
pub fn add_cards(scene: &mut Scene, pool: &mut Pool, cards: &[PlantCards], per_variant: usize, radius_m: f32) -> Result<(), String> {
    if cards.len() != pool.by_variant.len() { return Err(format!("{} card sets for {} variants", cards.len(), pool.by_variant.len())); }
    let mut meshes = Vec::new();
    for (v, c) in cards.iter().enumerate() {
        let layer = scene.add_image(&format!("recipe forest cards {v}"), c.atlas.clone());
        let materials: Vec<u32> = c.materials(layer).into_iter().map(|m| scene.add_material(m)).collect();
        let mesh = scene.add_mesh(&c.mesh_with_materials(&materials));
        let m = &scene.meshes[mesh];
        meshes.push(crate::scene::MeshRef { first_cluster: m.first_cluster, cluster_count: m.cluster_count, sphere: m.sphere });
    }
    let first = scene.instances.len();
    let mut slots = Vec::new();
    for (v, mesh) in meshes.iter().enumerate() {
        for _ in 0..per_variant.min(pool.by_variant[v].len()) {
            scene.instances.push(card_instance(mesh, Mat4::from_translation(PARKED)));
            slots.push((v, None));
        }
    }
    pool.cards = Some(Cards { meshes, first, slots, radius: radius_m });
    Ok(())
}

/// The members of `members` nearest `points`, at most `count`, within
/// `radius` and not in `skip`.
/// Distance factor a tree already in a tier gets when that tier is chosen
/// again: a newcomer must be over 15 % nearer to take its place.
const STICKY: f32 = 0.87;

fn nearest(trees: &[Tree], members: &[usize], points: &[Vec3], count: usize, radius: f32, skip: &std::collections::HashSet<usize>,
    held: &std::collections::HashSet<usize>) -> Vec<usize> {
    let mut near: Vec<(f32, usize)> = members.iter().filter(|i| !skip.contains(i)).map(|&i| (points.iter()
        .map(|p| p.distance_squared(trees[i].pos)).fold(f32::INFINITY, f32::min)
        * if held.contains(&i) { STICKY * STICKY } else { 1.0 }, i)).filter(|&(d, _)| d <= radius * radius).collect();
    let count = count.min(near.len());
    if count < near.len() { near.select_nth_unstable_by(count, |a, b| a.0.total_cmp(&b.0)); }
    near.truncate(count);
    near.into_iter().map(|(_, i)| i).collect()
}

/// Pool statistics for logs and tests. `slots` can be fewer than asked:
/// plant_streamed fits the pool to the renderer's budgets.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct PoolStats { pub slots: usize, pub placed: usize, pub moved: usize }

/// Put `trees` into `scene` as proxies, plus `slots` full assemblies per
/// variant that Pool::update gives to the trees nearest the cameras. Each
/// slot is built at its variant's largest scale (moves may
/// only shrink it) on that variant's first tree, so pass trees nearest first.
pub fn plant_streamed(scene: &mut Scene, variants: &[CookedPlant], trees: &[Tree], slots: usize,
    policy: Policy) -> Result<(AttachmentReport, Pool), String> {
    plant_streamed_reserved(scene, variants, trees, slots, policy, 0)
}

/// Reserve spare whole-plant instance slots for relocating a forest without
/// rebuilding shaders, geometry or view resources. Unused slots are parked.
pub fn plant_streamed_reserved(scene: &mut Scene, variants: &[CookedPlant], trees: &[Tree], slots: usize,
    policy: Policy, spare: usize) -> Result<(AttachmentReport, Pool), String> {
    if trees.is_empty() { return Err("a streamed recipe forest needs trees".into()); }
    if let Some(tree) = trees.iter().find(|t| t.variant >= variants.len()) {
        return Err(format!("recipe forest variant {} of {}", tree.variant, variants.len()));
    }
    // The pool's members and branch volumes share the renderer's fixed
    // budgets (part wind attachments, view groups): past them, the variant
    // with the most members gives up a slot until the pool fits.
    let counts: Vec<usize> = (0..variants.len()).map(|v| trees.iter().filter(|t| t.variant == v).count()).collect();
    let mut per: Vec<usize> = counts.iter().map(|&n| slots.min(n)).collect();
    let total = |per: &[usize], cost: &dyn Fn(&CookedPlant) -> usize| per.iter().zip(variants).map(|(&n, c)| n * cost(c)).sum::<usize>();
    // Virtual members keep no attachment per placement.
    let members = |c: &CookedPlant| if virtual_members() { 0 } else { c.stats().placements };
    while total(&per, &members) > crate::part_wind::MAX_ATTACHMENTS
        || total(&per, &|c| c.stats().volume_assets) > crate::voxel_forest::MAX_VIEW_GROUPS {
        let worst = (0..per.len()).filter(|&v| per[v] > 0)
            .max_by_key(|&v| per[v] * variants[v].stats().volume_assets.max(members(&variants[v]))).ok_or("not one full plant fits the renderer")?;
        per[worst] -= 1;
    }
    let mut slot_scales = vec![0.0; variants.len()];
    let mut built = Vec::new();
    for (variant, &n) in per.iter().enumerate() {
        let Some(first) = trees.iter().find(|t| t.variant == variant) else { continue; };
        let scale = trees.iter().filter(|t| t.variant == variant).map(|t| t.scale).fold(0.0, f32::max);
        let scale = scale * if spare > 0 { 1.25 } else { 1.0 };
        slot_scales[variant] = scale;
        for _ in 0..n { built.push(Tree { scale, ..*first }); }
    }
    let mut reserved = trees.to_vec();
    reserved.extend((0..spare).map(|_| Tree { pos: PARKED, ..trees[0] }));
    let mut vegetation = SceneVegetation::new(scene, limits(variants, built.len()))?;
    let mut pool = Vec::with_capacity(built.len());
    for (k, tree) in built.iter().enumerate() {
        let handle = vegetation.append(&variants[tree.variant], instance(reserved.len() + k, tree))?;
        pool.push(Slot { variant: tree.variant, plant: handle.movable, tree: None });
    }
    for (i, tree) in reserved.iter().enumerate() { vegetation.append_proxy(&variants[tree.variant], instance(i, tree))?; }
    let report = vegetation.finish_aggregate(policy)?;
    let first_proxy = report.volume_instances as u32;
    // Reserved slots must not enlarge the canopy's height/mass envelope.
    for v in &mut scene.voxel_forest.as_mut().ok_or("missing packed forest")?.instances[first_proxy as usize+trees.len()..] {
        v.bounds_min[3]=ProxyShow::Hidden as u32 as f32;
        scene.instances[v.meta[0] as usize].flags=0;
    }
    let proxies = &scene.voxel_forest.as_ref().ok_or("missing packed forest")?.instances[first_proxy as usize..];
    let proxy_owners = proxies.iter().map(|t| t.meta[0]).collect();
    let proxy_templates = (0..variants.len()).map(|v| trees.iter().position(|t| t.variant == v)
        .map(|i| (trees[i], proxies[i], scene.instances[proxies[i].meta[0] as usize]))).collect();
    Ok((report, Pool { trees: trees.to_vec(), proxy_templates, proxy_owners, slot_scales, slots: pool, first_proxy, last: Vec::new(), cards: None, hidden: Default::default(),
        by_variant: (0..variants.len()).map(|v| (0..trees.len()).filter(|&i| trees[i].variant == v).collect()).collect() }))
}

/// Move an immutable proxy template. The transformed AABB is conservative;
/// the inverse transform still addresses exactly the same shared voxel asset.
fn relocated_proxy(template: &(Tree, crate::foliage_volume_data::GpuTree, GpuInstance), tree: &Tree,
    owner: u32) -> (crate::foliage_volume_data::GpuTree, GpuInstance) {
    let (old, mut volume, mut source) = *template;
    let delta = root(tree) * root(&old).inverse();
    let inverse = crate::foliage_volume_data::matrix(volume.inverse) * delta.inverse();
    volume.inverse = [inverse.row(0).to_array(), inverse.row(1).to_array(), inverse.row(2).to_array()];
    let lo = Vec3::from_slice(&volume.bounds_min);
    let hi = Vec3::from_slice(&volume.bounds_max);
    let mut a = Vec3::splat(f32::INFINITY);
    let mut b = Vec3::splat(f32::NEG_INFINITY);
    for k in 0..8 {
        let p = delta.transform_point3(Vec3::new(if k&1==0 {lo.x}else{hi.x}, if k&2==0 {lo.y}else{hi.y}, if k&4==0 {lo.z}else{hi.z}));
        a = a.min(p); b = b.max(p);
    }
    volume.bounds_min = a.extend(0.0).to_array();
    volume.bounds_max = b.extend(0.0).to_array();
    volume.meta[0] = owner;
    let transform = delta * crate::foliage_volume_data::matrix(source.rows);
    source.rows = [transform.row(0).to_array(), transform.row(1).to_array(), transform.row(2).to_array()];
    source.sphere = delta.transform_point3(Vec3::from_slice(&source.sphere)).extend(source.sphere[3] * tree.scale / old.scale).to_array();
    source.scale = tree.scale;
    (volume, source)
}

impl Pool {
    /// Replace placements atomically before drawing. False means capacity,
    /// species or scale requires a real rebuild; no partial writes are made.
    pub fn replace_trees(&mut self, renderer: &Renderer, gpu: &Gpu, trees: Vec<Tree>) -> Result<bool, String> {
        if trees.len() > self.proxy_owners.len() || trees.iter().any(|t|
            !t.pos.is_finite() || !t.yaw.is_finite() || !t.scale.is_finite() || t.scale < 1e-4 ||
            self.proxy_templates.get(t.variant).is_none_or(|v| v.is_none()) ||
            t.scale > self.slot_scales.get(t.variant).copied().unwrap_or(0.0)) { return Ok(false); }
        if self.by_variant.iter().enumerate().any(|(v,_)| trees.iter().filter(|t| t.variant == v).count() < self.slots.iter().filter(|s| s.variant == v).count()) { return Ok(false); }
        let fallback = self.proxy_templates.iter().flatten().next().ok_or("no proxy template")?;
        let mut volumes = Vec::with_capacity(self.proxy_owners.len());
        let mut sources = Vec::with_capacity(self.proxy_owners.len());
        for (i, &owner) in self.proxy_owners.iter().enumerate() {
            let (mut volume, mut source) = if let Some(tree) = trees.get(i) {
                relocated_proxy(self.proxy_templates[tree.variant].as_ref().unwrap(), tree, owner)
            } else { relocated_proxy(fallback, &Tree {pos: PARKED, ..fallback.0}, owner) };
            if i >= trees.len() { volume.bounds_min[3] = ProxyShow::Hidden as u32 as f32; source.flags = 0; }
            volumes.push(volume); sources.push(source);
        }
        renderer.replace_plant_proxy_instances(gpu, self.first_proxy, &volumes, &sources)?;
        self.trees = trees;
        self.by_variant = (0..self.proxy_templates.len()).map(|v| (0..self.trees.len()).filter(|&i| self.trees[i].variant == v).collect()).collect();
        self.reset();
        Ok(true)
    }
    /// Forget where the slots stand, for a renderer built anew from the
    /// scene: the next update places every slot again.
    pub fn reset(&mut self) {
        for slot in &mut self.slots { slot.tree = None; }
        if let Some(cards) = &mut self.cards { for slot in &mut cards.slots { slot.1 = None; } }
        self.last.clear();
        self.hidden.clear();
    }

    /// Give each variant's slots to its trees nearest `points` (the cameras).
    /// Call before the first frame: until then slots stand unplaced. Slots
    /// on trees still wanted stay; a slot moves only when a nearer tree needs
    /// it. Cheap when the cameras stand still.
    pub fn update(&mut self, renderer: &Renderer, gpu: &Gpu, points: &[Vec3]) -> Result<PoolStats, String> {
        let mut stats = PoolStats { slots: self.slots.len(), placed: self.slots.iter().filter(|s| s.tree.is_some()).count(), moved: 0 };
        let still = points.len() == self.last.len() && points.iter().zip(&self.last).all(|(a, b)| a.distance(*b) < 1.0);
        if still || points.is_empty() { return Ok(stats); }
        self.last = points.to_vec();
        for (variant, members) in self.by_variant.iter().enumerate() {
            let mine: Vec<usize> = (0..self.slots.len()).filter(|&k| self.slots[k].variant == variant).collect();
            if mine.is_empty() { continue; }
            // O(trees): the nearest few by selection, not a full sort. A tree
            // already full counts as STICKY nearer, so one at the edge of the
            // set does not flip between full and card at every camera metre.
            let held: std::collections::HashSet<usize> = mine.iter().filter_map(|&k| self.slots[k].tree).collect();
            let mut near: Vec<(f32, usize)> = members.iter().map(|&i| (points.iter()
                .map(|p| p.distance_squared(self.trees[i].pos)).fold(f32::INFINITY, f32::min)
                * if held.contains(&i) { STICKY * STICKY } else { 1.0 }, i)).collect();
            let count = mine.len().min(near.len());
            if count < near.len() { near.select_nth_unstable_by(count, |a, b| a.0.total_cmp(&b.0)); }
            near.truncate(count);
            let wanted: Vec<usize> = near.iter().map(|&(_, i)| i).collect();
            let mut free: Vec<usize> = mine.iter().copied().filter(|&k| self.slots[k].tree.is_none_or(|t| !wanted.contains(&t))).collect();
            // Unplaced slots are taken first (pop from the back).
            free.sort_by_key(|&k| std::cmp::Reverse(self.slots[k].tree.is_some()));
            for tree in wanted {
                if mine.iter().any(|&k| self.slots[k].tree == Some(tree)) { continue; }
                let Some(k) = free.pop() else { break; };
                let slot = &mut self.slots[k];
                renderer.move_plant(gpu, &slot.plant, root(&self.trees[tree]))?;
                slot.tree = Some(tree);
                stats.moved += 1;
            }
        }
        let full: std::collections::HashSet<usize> = self.slots.iter().filter_map(|s| s.tree).collect();
        let mut wanted: std::collections::HashMap<usize, ProxyShow> = full.iter().map(|&t| (t, ProxyShow::Hidden)).collect();
        // Cards: the nearest trees past the full ones, within the radius. A
        // card stays on a tree still wanted; freed cards take the new ones.
        if let Some(cards) = &mut self.cards {
            let mut moved = false;
            for (variant, members) in self.by_variant.iter().enumerate() {
                let mine: Vec<usize> = (0..cards.slots.len()).filter(|&k| cards.slots[k].0 == variant).collect();
                if mine.is_empty() { continue; }
                let held: std::collections::HashSet<usize> = mine.iter().filter_map(|&k| cards.slots[k].1).collect();
                let near = nearest(&self.trees, members, points, mine.len(), cards.radius, &full, &held);
                let mut free: Vec<usize> = mine.iter().copied().filter(|&k| cards.slots[k].1.is_none_or(|t| !near.contains(&t))).collect();
                for &tree in &near {
                    if mine.iter().any(|&k| cards.slots[k].1 == Some(tree)) { continue; }
                    let Some(k) = free.pop() else { break; };
                    cards.slots[k].1 = Some(tree);
                    moved = true;
                }
                for k in free { if cards.slots[k].1.take().is_some() { moved = true; } }
            }
            // Cards cast no shadow: their proxies keep casting, as every
            // other non-full tree's does.
            wanted.extend(cards.slots.iter().filter_map(|s| s.1).map(|t| (t, ProxyShow::ShadowOnly)));
            if moved {
                let instances: Vec<GpuInstance> = cards.slots.iter().map(|&(v, tree)| card_instance(&cards.meshes[v],
                    tree.map_or(Mat4::from_translation(PARKED), |t| root(&self.trees[t])))).collect();
                renderer.update_instances(gpu, cards.first, &instances)?;
            }
        }
        // Proxies hide where a full tree or a card stands, and show again
        // where none does, in one call (no frame without either).
        let mut changes: Vec<(u32, ProxyShow)> = wanted.iter().filter(|(t, s)| self.hidden.get(t) != Some(s))
            .map(|(&t, &s)| (self.first_proxy + t as u32, s)).collect();
        changes.extend(self.hidden.keys().filter(|t| !wanted.contains_key(t)).map(|&t| (self.first_proxy + t as u32, ProxyShow::Shown)));
        renderer.set_plant_proxies(gpu, &changes)?;
        self.hidden = wanted;
        stats.placed = self.slots.iter().filter(|s| s.tree.is_some()).count();
        Ok(stats)
    }
}

/// The renderer policy for a recipe forest in a game: ordinary screen-error
/// cluster LOD, sampled optics, proxies at Policy's defaults.
pub fn game_policy() -> Policy {
    // Branch groups take their voxels at up to 32 px of spatial error. The
    // screen-error triangle cut thins sprays as they shrink (forest_ladder
    // spruce: 84 % of the source's cover at 40 m, 50 % at 80 m with 8 px);
    // the voxels keep their measured coverage, and since shade lights them
    // like the leaves (VOXEL_SURFACE) their tone too: 40-300 m within 96-105 %
    // cover and 99-106 % tone for spruce and birch, grove 51 -> 19 ms.
    Policy { max_error_pixels: 32.0, allow_sampled_optics: true, measured_source_cuts: false, ..Default::default() }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn relocated_proxy_keeps_local_samples_owner_and_asset() {
        let old = Tree {pos:Vec3::new(100.0,20.0,-30.0),yaw:0.7,scale:1.3,variant:0};
        let inv = root(&old).inverse();
        let transform = root(&old);
        let mut volume = crate::foliage_volume_data::GpuTree::zeroed();
        volume.inverse = [inv.row(0).to_array(), inv.row(1).to_array(), inv.row(2).to_array()];
        volume.meta = [9,12,6,u32::MAX];
        let mut lo=Vec3::splat(f32::INFINITY); let mut hi=Vec3::splat(f32::NEG_INFINITY);
        let points:Vec<_>=(0..8).map(|k| Vec3::new(if k&1==0 {-2.0}else{2.0}, if k&2==0 {0.0}else{14.0}, if k&4==0 {-3.0}else{3.0})).collect();
        for p in &points {let w=transform.transform_point3(*p);lo=lo.min(w);hi=hi.max(w);}
        volume.bounds_min=lo.extend(0.0).to_array();volume.bounds_max=hi.extend(0.0).to_array();
        let source=GpuInstance {rows:[transform.row(0).to_array(),transform.row(1).to_array(),transform.row(2).to_array()],sphere:old.pos.extend(10.0).to_array(),scale:old.scale,..GpuInstance::zeroed()};
        for yaw in [-2.0,0.0,1.7] {for scale in [0.2,1.0,2.0] {
            let new=Tree {pos:Vec3::new(-400.0,50.0,190.0),yaw,scale,..old};
            let (v,s)=relocated_proxy(&(old,volume,source),&new,47);
            assert_eq!(v.meta,[47,12,6,u32::MAX]);
            for p in &points {
                let w=root(&new).transform_point3(*p);
                assert!(crate::foliage_volume_data::matrix(v.inverse).transform_point3(w).distance(*p)<0.001);
                assert!((w-Vec3::from_slice(&v.bounds_min)).min_element()>-0.001);
                assert!((Vec3::from_slice(&v.bounds_max)-w).min_element()>-0.001);
                assert!(crate::foliage_volume_data::matrix(s.rows).transform_point3(*p).distance(w)<0.001);
            }
        }}
    }
}
