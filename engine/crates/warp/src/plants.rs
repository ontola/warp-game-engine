//! Grown plants a Luau game places (`plants.grow`): a species' growth rules
//! and an age, cooked on background threads into Nanite-style vegetation
//! (render::plant_growth, render::recipe_forest::cook_grown) and attached to
//! the world's scene as one GPU forest.
//!
//! Two qualities. `preview` (the default) grows and assembles the plant and
//! draws its source parts as plain meshes: a fraction of a second, for
//! tuning. `full` cooks the voxel levels and proxies as well (minutes for an
//! old tree), so LOD, voxels and far views can be judged.
//!
//! Every distinct (rules, look, age, seed, quality) is one cooked variant,
//! kept in memory (full cooks also on disk, build/cook-cache), so changing
//! one parameter only re-cooks the plants that use it. Any change rebuilds
//! the scene and the renderer on the next frame (a short stall, fine for a
//! tuning lab).
use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, SimplificationPolicy};
use render::scene::Material;
use render::foliage_population::WorldRoot;
use render::foliage_runtime::RootPlacement;
use render::plant_growth::Species;
use render::plant_growth_assembly::{Look, Parts};
use render::vegetation_asset::CookedPlant;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, OnceLock};

/// One placed plant.
#[derive(Clone, Debug)]
pub struct Plant {
    pub species: Species,
    pub look: Look,
    pub age: u32,
    pub seed: u64,
    pub pos: Vec3,
    pub yaw: f32,
    /// Cook voxels and proxies too, not only the source meshes.
    pub full: bool,
}

impl Plant {
    fn key(&self) -> String {
        serde_json::json!({ "species": self.species, "look": self.look, "age": self.age, "seed": self.seed, "full": self.full }).to_string()
    }
}

/// A cooked variant.
enum Cooked {
    Full(Arc<CookedPlant>),
    Preview(Arc<Preview>),
}

/// What a background cook sends back: the variant, loaded from disk, milliseconds, a summary.
type Done = Result<(Cooked, bool, f64, String), String>;

/// A plant's source parts, Nanite style: each part's mesh once (per
/// material), and every placement of it as an instance in the plant's space.
struct Preview {
    meshes: Vec<(u64, Material, ClusterMesh)>,
    instances: Vec<(usize, glam::Mat4)>,
    triangles: usize,
}

fn preview(species: &Species, look: Look, age: u32, seed: u64) -> Result<Preview, String> {
    let plant = render::plant_growth::grow(species, age, seed);
    let (assembly, _) = render::plant_growth_assembly::assemble(&plant, parts(&look), look, seed, preview_limits())?;
    let branches = assembly.branch_transforms()?;
    let mut index: HashMap<(u32, u32), usize> = HashMap::new();
    let mut out = Preview { meshes: Vec::new(), instances: Vec::with_capacity(assembly.instances.len()), triangles: 0 };
    for inst in &assembly.instances {
        let at = *index.entry((inst.part, inst.material)).or_insert_with(|| {
            let part = &assembly.parts[inst.part as usize];
            let p = [render::cluster::Part { vertices: &part.vertices, indices: &part.indices, material: 0 }];
            let material = assembly.materials[inst.material as usize];
            // Needles keep every triangle at every distance: simplifying a
            // thin needle drops it, and crowns thin out a few metres away.
            // The full cook handles distance with voxels instead.
            let mesh = if material.flags & render::scene::THIN_FOLIAGE != 0 {
                ClusterMesh::build_with_constraints(&p, |_| SimplificationPolicy::ThinFoliage, |_, _| true)
            } else {
                ClusterMesh::build_with_policy(&p, |_| SimplificationPolicy::TexturedGeometry)
            };
            out.meshes.push((part.id, assembly.materials[inst.material as usize], mesh));
            out.meshes.len() - 1
        });
        out.triangles += assembly.parts[inst.part as usize].indices.len() / 3;
        out.instances.push((at, branches[inst.branch as usize] * inst.rest));
    }
    Ok(out)
}

fn cook(species: Species, look: Look, age: u32, seed: u64, full: bool) -> Done {
    let started = std::time::Instant::now();
    if !full {
        let p = preview(&species, look, age, seed)?;
        let what = format!("{} parts, {} placed, {} triangles", p.meshes.len(), p.instances.len(), p.triangles);
        return Ok((Cooked::Preview(Arc::new(p)), false, started.elapsed().as_secs_f64() * 1000.0, what));
    }
    let cache = cache_dir();
    let c = render::recipe_forest::cook_grown(&[(species, age, seed)], parts(&look), look, Some(&cache))?.pop().ok_or("nothing cooked")?;
    let s = c.plant.stats();
    let what = format!("{} branches, {} placements", s.branches, s.placements);
    Ok((Cooked::Full(Arc::new(c.plant)), c.loaded, c.millis, what))
}

#[derive(Default)]
pub struct Plants {
    next: u32,
    pub plants: BTreeMap<u32, Plant>,
    cooked: HashMap<String, Cooked>,
    failed: HashMap<String, String>,
    pending: HashMap<String, Receiver<Done>>,
    /// The plants changed since the scene last took them.
    pub dirty: bool,
    /// Log lines for the game's log (cooks finished, failures).
    pub log: Vec<String>,
}

/// The preview draws plain instances, so it has no placement budget: every
/// stem and brush of any rule set shows (the full cook keeps the grown limits).
fn preview_limits() -> render::foliage_assembly::Limits {
    render::foliage_assembly::Limits { max_instances: 1 << 20, max_vertices: u32::MAX as usize / 4, max_indices: u32::MAX as usize / 4, max_source_bytes: usize::MAX / 4, ..render::plant_growth_assembly::limits() }
}

/// The parts for a look's brushes, made once per needle count and length.
fn parts(look: &Look) -> &'static Parts {
    static PARTS: OnceLock<std::sync::Mutex<HashMap<[u32; 4], &'static Parts>>> = OnceLock::new();
    let mut map = PARTS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    *map.entry([look.brush_needles, look.needle_length.to_bits(), look.brush_spread.to_bits(), look.brush_reach.to_bits()])
        .or_insert_with(|| Box::leak(Box::new(Parts::for_look(look))))
}

fn cache_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache") }

impl Plants {
    pub fn grow(&mut self, p: Plant) -> u32 {
        self.next += 1;
        let key = p.key();
        if !self.cooked.contains_key(&key) && !self.pending.contains_key(&key) && !self.failed.contains_key(&key) {
            let (tx, rx) = channel();
            let (species, look, age, seed, full) = (p.species, p.look, p.age, p.seed, p.full);
            std::thread::spawn(move || { let _ = tx.send(cook(species, look, age, seed, full)); });
            self.pending.insert(key, rx);
        }
        self.plants.insert(self.next, p);
        self.dirty = true;
        self.next
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let gone = self.plants.remove(&id).is_some();
        self.dirty |= gone;
        gone
    }

    pub fn clear(&mut self) {
        self.dirty |= !self.plants.is_empty();
        self.plants.clear();
    }

    /// Collect finished cooks. True when one of them is in use.
    pub fn poll(&mut self) -> bool {
        let mut landed = false;
        let keys: Vec<String> = self.pending.keys().cloned().collect();
        for key in keys {
            let got = match self.pending[&key].try_recv() {
                Ok(done) => done,
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Err(_) => Err("the cook thread stopped".into()),
            };
            self.pending.remove(&key);
            let age = self.plants.values().find(|p| p.key() == key).map(|p| p.age);
            match got {
                Ok((plant, loaded, ms, what)) => {
                    let kind = if matches!(plant, Cooked::Full(_)) { "full" } else { "preview" };
                    self.log.push(format!("plant age {} ({kind}): {} in {ms:.0} ms, {what}", age.map_or("?".into(), |a| a.to_string()), if loaded { "loaded" } else { "cooked" }));
                    self.cooked.insert(key, plant);
                }
                Err(e) => { self.log.push(format!("plant cook failed: {e}")); self.failed.insert(key, e); }
            }
            landed |= age.is_some();
        }
        self.dirty |= landed;
        // Keep memory to what is placed and a little history.
        if self.cooked.len() > 256 {
            let live: std::collections::HashSet<String> = self.plants.values().map(Plant::key).collect();
            self.cooked.retain(|k, _| live.contains(k));
        }
        landed
    }

    /// (placed, cooked and shown, still cooking, failed).
    pub fn status(&self) -> (usize, usize, usize, usize) {
        let mut out = (self.plants.len(), 0, 0, 0);
        for p in self.plants.values() {
            let k = p.key();
            if self.cooked.contains_key(&k) { out.1 += 1; } else if self.pending.contains_key(&k) { out.2 += 1; } else { out.3 += 1; }
        }
        out
    }

    /// The last failure, if any placed plant failed to cook.
    pub fn error(&self) -> Option<&str> { self.plants.values().find_map(|p| self.failed.get(&p.key()).map(String::as_str)) }

    /// Attach every cooked placed plant to `scene` (a fresh scene with no
    /// vegetation yet): previews as plain meshes, full cooks as one GPU forest.
    pub fn attach(&self, scene: &mut render::scene::Scene) -> Result<usize, String> {
        let mut shown = 0;
        // Previews: each part mesh once for all plants, an instance per placement.
        let mut meshes: HashMap<(u64, Vec<u8>), usize> = HashMap::new();
        let mut materials: HashMap<Vec<u8>, u32> = HashMap::new();
        for p in self.plants.values() {
            let Some(Cooked::Preview(v)) = self.cooked.get(&p.key()) else { continue };
            let ids: Vec<usize> = v.meshes.iter().map(|(part, material, mesh)| {
                let bytes = bytemuck::bytes_of(material).to_vec();
                if let Some(&id) = meshes.get(&(*part, bytes.clone())) { return id; }
                let m = *materials.entry(bytes.clone()).or_insert_with(|| scene.add_material(*material));
                let first = scene.clusters.len();
                let id = scene.add_mesh(mesh);
                for c in &mut scene.clusters[first..] { c.material = m; }
                meshes.insert((*part, bytes), id);
                id
            }).collect();
            let root = glam::Mat4::from_rotation_translation(Quat::from_rotation_y(p.yaw), p.pos);
            for (at, m) in &v.instances { scene.add_instance(ids[*at], Affine3A::from_mat4(root * *m)); }
            shown += 1;
        }
        let mut variants: Vec<(String, Arc<CookedPlant>)> = Vec::new();
        let mut roots = Vec::new();
        for (&id, p) in &self.plants {
            let key = p.key();
            let Some(Cooked::Full(c)) = self.cooked.get(&key) else { continue };
            let variant = match variants.iter().position(|(k, _)| *k == key) { Some(i) => i, None => { variants.push((key, c.clone())); variants.len() - 1 } };
            roots.push(WorldRoot { id: u64::from(id), placement: RootPlacement { pos: p.pos, yaw: p.yaw, scale: 1.0, variant, up: Vec3::Y } });
        }
        if roots.is_empty() { return Ok(shown); }
        // Workaround: a single-branch plant (a seedling) as a GPU forest
        // variant hides every variant after it (seen on lavapipe and an RTX
        // 5070 Ti; cause not found yet). Order the variants by branch count,
        // most first, so single-branch ones come last.
        let mut order: Vec<usize> = (0..variants.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(variants[i].1.stats().branches));
        let mut new_index = vec![0; variants.len()];
        for (to, &from) in order.iter().enumerate() { new_index[from] = to; }
        let variants: Vec<_> = order.iter().map(|&i| variants[i].clone()).collect();
        for r in &mut roots { r.placement.variant = new_index[r.placement.variant]; }
        let species: Vec<_> = variants.iter().map(|(_, plant)| render::foliage_runtime::Species {
            plant: plant.as_ref(), category: render::scene::GeometryCategory::Trees, deformation: render::foliage_runtime::Deformation::Rigid,
        }).collect();
        // terrain_fly's quality policy: the pixel error never relaxes with distance.
        let policy = render::aggregate_lod::Policy { max_error_pixels: 1.0, allow_sampled_optics: true, measured_source_cuts: false, proxy_pixels: 1.0, proxy_pixels_from: 0.0, ..Default::default() };
        let capacity = (roots.len() * 2).max(64);
        render::foliage_runtime::plant_species_population(scene, &species, &roots, capacity, 32768, policy)?;
        Ok(shown + roots.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pine(age: u32, x: f32) -> Plant {
        Plant { species: Species::jeffrey_pine(), look: Look::jeffrey_pine(), age, seed: 1, pos: Vec3::new(x, 0.0, 0.0), yaw: 0.0, full: false }
    }

    #[test]
    fn previews_cook_in_the_background_and_attach_as_meshes() {
        let mut plants = Plants::default();
        plants.grow(pine(6, 0.0));
        plants.grow(pine(6, 4.0));
        plants.grow(pine(80, 12.0));
        let started = std::time::Instant::now();
        while plants.status().1 < 3 {
            plants.poll();
            assert!(plants.error().is_none(), "{:?}", plants.error());
            assert!(started.elapsed().as_secs() < 120, "preview too slow");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(plants.status(), (3, 3, 0, 0));
        let mut scene = render::scene::Scene::default();
        assert_eq!(plants.attach(&mut scene).unwrap(), 3);
        println!("{:?}", plants.log);
        let meshes = scene.meshes.len();
        assert!(meshes >= 2 && scene.instances.len() > meshes, "{meshes} meshes, {} instances", scene.instances.len());
        assert!(scene.gpu_forest.is_none());
        assert!(plants.log.iter().any(|l| l.contains("preview")), "{:?}", plants.log);
    }
}

#[cfg(test)]
mod limits {
    use super::*;

    #[test]
    fn a_crowded_preview_has_no_placement_budget() {
        let species = Species { crowding: 60, needle_years: 7, ..Species::jeffrey_pine() };
        let started = std::time::Instant::now();
        let p = preview(&species, Look::jeffrey_pine(), 31, 4).unwrap();
        println!("crowded 31 y: {} placed, {} triangles, {} ms", p.instances.len(), p.triangles, started.elapsed().as_millis());
    }
}
