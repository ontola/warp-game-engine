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

fn limits(variants: &[CookedPlant], full: usize) -> AttachLimits {
    let branches = variants.iter().map(|c| c.stats().branches).max().unwrap_or(64);
    let mut limits = AttachLimits { max_poses: branches.max(64) * full.max(4),
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

/// Full trees that follow the cameras, the way Nanite streams detail: every
/// tree is its whole-plant proxy, and a fixed pool of full assemblies (`slots`
/// per variant) moves to the trees nearest the cameras, hiding their proxies
/// (Renderer::move_plant, set_plant_proxies). A full tree still picks
/// per frame on the GPU between its branches and its own voxels.
pub struct Pool {
    trees: Vec<Tree>,
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
fn nearest(trees: &[Tree], members: &[usize], points: &[Vec3], count: usize, radius: f32, skip: &std::collections::HashSet<usize>) -> Vec<usize> {
    let mut near: Vec<(f32, usize)> = members.iter().filter(|i| !skip.contains(i)).map(|&i| (points.iter()
        .map(|p| p.distance_squared(trees[i].pos)).fold(f32::INFINITY, f32::min), i)).filter(|&(d, _)| d <= radius * radius).collect();
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
    while total(&per, &|c| c.stats().placements) > crate::part_wind::MAX_ATTACHMENTS
        || total(&per, &|c| c.stats().volume_assets) > crate::voxel_forest::MAX_VIEW_GROUPS {
        let worst = (0..per.len()).filter(|&v| per[v] > 0)
            .max_by_key(|&v| per[v] * variants[v].stats().placements).ok_or("not one full plant fits the renderer")?;
        per[worst] -= 1;
    }
    let mut built = Vec::new();
    for (variant, &n) in per.iter().enumerate() {
        let Some(first) = trees.iter().find(|t| t.variant == variant) else { continue; };
        let scale = trees.iter().filter(|t| t.variant == variant).map(|t| t.scale).fold(0.0, f32::max);
        for _ in 0..n { built.push(Tree { scale, ..*first }); }
    }
    let mut vegetation = SceneVegetation::new(scene, limits(variants, built.len()))?;
    let mut pool = Vec::with_capacity(built.len());
    for (k, tree) in built.iter().enumerate() {
        let handle = vegetation.append(&variants[tree.variant], instance(trees.len() + k, tree))?;
        pool.push(Slot { variant: tree.variant, plant: handle.movable, tree: None });
    }
    for (i, tree) in trees.iter().enumerate() { vegetation.append_proxy(&variants[tree.variant], instance(i, tree))?; }
    let report = vegetation.finish_aggregate(policy)?;
    let first_proxy = report.volume_instances as u32;
    Ok((report, Pool { trees: trees.to_vec(), slots: pool, first_proxy, last: Vec::new(), cards: None, hidden: Default::default(),
        by_variant: (0..variants.len()).map(|v| (0..trees.len()).filter(|&i| trees[i].variant == v).collect()).collect() }))
}

impl Pool {
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
            // O(trees): the nearest few by selection, not a full sort.
            let mut near: Vec<(f32, usize)> = members.iter().map(|&i| (points.iter()
                .map(|p| p.distance_squared(self.trees[i].pos)).fold(f32::INFINITY, f32::min), i)).collect();
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
                let near = nearest(&self.trees, members, points, mine.len(), cards.radius, &full);
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
    // Branch groups may take their voxels at up to 8 px of spatial error: at
    // 1 px their coarsest cells (0.39 m error) fitted only past ~400 m, and
    // between that and 40 m the screen-error triangle cut thinned the sprays
    // to a third of their cover (forest_ladder: spruce 33 % at 150-600 m, now
    // 104-113 %, at the same cost). The voxels keep their measured coverage.
    Policy { max_error_pixels: 8.0, allow_sampled_optics: true, measured_source_cuts: false, ..Default::default() }
}
