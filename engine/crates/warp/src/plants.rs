//! Grown plants a Luau game places (`plants.grow`): a species' growth rules
//! and an age, cooked on background threads into Nanite-style vegetation
//! (render::plant_species, render::recipe_forest::cook_catalog) and attached to
//! the world's scene as one GPU forest.
//!
//! Two qualities. `preview` (the default) grows and assembles the plant and
//! draws its source parts as plain meshes: a fraction of a second, for
//! tuning. `full` cooks the voxel levels and proxies as well (minutes for an
//! old tree), so LOD, voxels and far views can be judged.
//!
//! Every distinct (preset, rules, look, age, seed, quality) is one cooked variant,
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
use render::plant_species::GrownSpecies;
use render::vegetation_asset::CookedPlant;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, OnceLock};

/// One placed plant.
#[derive(Clone, Copy, Debug)]
pub struct Plant {
    /// Catalogue entry supplying the foliage parts. Custom pine rules use `pine`.
    pub preset: &'static str,
    pub species: Species,
    pub look: Look,
    pub age: u32,
    pub seed: u64,
    pub pos: Vec3,
    pub yaw: f32,
    /// Cook voxels and proxies too, not only the source meshes. The preview
    /// shows until the full cook is done, then the plant swaps to it.
    pub full: bool,
}

impl Plant {
    fn key(&self) -> String {
        serde_json::json!({ "preset": self.preset, "species": self.species, "look": self.look, "age": self.age, "seed": self.seed, "full": self.full }).to_string()
    }

    /// The preview variant: what shows at once, and stands in for a full cook.
    fn preview_key(&self) -> String {
        Plant { full: false, ..*self }.key()
    }

    fn form(&self) -> Result<GrownSpecies, String> {
        let mut form = render::plant_species::grown(self.preset)
            .ok_or_else(|| format!("unknown grown plant {:?}", self.preset))?;
        form.rules = self.species;
        form.look = self.look;
        Ok(form)
    }
}

/// Full cooks take seconds to minutes and gigabytes each, so they run one at
/// a time on one worker, oldest request first; a request whose plant is gone
/// by its turn is skipped.
struct FullJob { key: String, plant: Plant, done: std::sync::mpsc::Sender<Done> }

/// What the full-cook worker sends for a request it skipped.
const SKIPPED: &str = "skipped: no longer placed";

fn wanted() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static WANTED: OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    WANTED.get_or_init(Default::default)
}

fn full_worker() -> &'static std::sync::Mutex<std::sync::mpsc::Sender<FullJob>> {
    static WORKER: OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<FullJob>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<FullJob>();
        std::thread::spawn(move || {
            for job in rx {
                let still = wanted().lock().unwrap_or_else(|p| p.into_inner()).contains(&job.key);
                let done = if still { cook(job.plant) } else { Err(SKIPPED.into()) };
                let _ = job.done.send(done);
            }
        });
        std::sync::Mutex::new(tx)
    })
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

fn preview(form: &GrownSpecies, parts: &Parts, age: u32, seed: u64) -> Result<Preview, String> {
    let (assembly, _) = form.assemble(parts, age, seed, preview_limits())?;
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

fn cook(plant: Plant) -> Done {
    let started = std::time::Instant::now();
    let form = plant.form()?;
    let parts = form.parts();
    if !plant.full {
        let p = preview(&form, &parts, plant.age, plant.seed)?;
        let what = format!("{} parts, {} placed, {} triangles", p.meshes.len(), p.instances.len(), p.triangles);
        return Ok((Cooked::Preview(Arc::new(p)), false, started.elapsed().as_secs_f64() * 1000.0, what));
    }
    let cache = cache_dir();
    let c = render::recipe_forest::cook_catalog(&form, plant.age, plant.seed, &parts, Some(&cache))?;
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

fn cache_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache") }

impl Plants {
    pub fn grow(&mut self, p: Plant) -> u32 {
        self.next += 1;
        // The preview always: it shows at once (and stands in for a full cook).
        let key = p.preview_key();
        if !self.cooked.contains_key(&key) && !self.pending.contains_key(&key) && !self.failed.contains_key(&key) {
            let (tx, rx) = channel();
            std::thread::spawn(move || { let _ = tx.send(cook(Plant { full: false, ..p })); });
            self.pending.insert(key, rx);
        }
        if p.full {
            let key = p.key();
            wanted().lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone());
            if !self.cooked.contains_key(&key) && !self.pending.contains_key(&key) && !self.failed.contains_key(&key) {
                let (tx, rx) = channel();
                let job = FullJob { key: key.clone(), plant: p, done: tx };
                let _ = full_worker().lock().unwrap_or_else(|e| e.into_inner()).send(job);
                self.pending.insert(key, rx);
            }
        }
        self.plants.insert(self.next, p);
        self.dirty = true;
        self.next
    }

    /// Full cooks still wanted: those of placed plants.
    fn rewant(&self) {
        let mut w = wanted().lock().unwrap_or_else(|e| e.into_inner());
        w.clear();
        w.extend(self.plants.values().filter(|p| p.full).map(Plant::key));
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let gone = self.plants.remove(&id).is_some();
        self.dirty |= gone;
        self.rewant();
        gone
    }

    pub fn clear(&mut self) {
        self.dirty |= !self.plants.is_empty();
        self.plants.clear();
        self.rewant();
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
            let age = self.plants.values().find(|p| p.key() == key || p.preview_key() == key).map(|p| p.age);
            match got {
                Err(e) if e == SKIPPED => continue,
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
            let live: std::collections::HashSet<String> = self.plants.values().flat_map(|p| [p.key(), p.preview_key()]).collect();
            self.cooked.retain(|k, _| live.contains(k));
        }
        landed
    }

    /// (placed, shown, still cooking with nothing to show, failed). A full
    /// plant shows as its preview until its full cook is done.
    pub fn status(&self) -> (usize, usize, usize, usize) {
        let mut out = (self.plants.len(), 0, 0, 0);
        for p in self.plants.values() {
            let (k, v) = (p.key(), p.preview_key());
            if self.cooked.contains_key(&k) || self.cooked.contains_key(&v) { out.1 += 1; }
            else if self.pending.contains_key(&k) || self.pending.contains_key(&v) { out.2 += 1; }
            else { out.3 += 1; }
        }
        out
    }

    /// Full plants still showing their preview while the full cook runs.
    pub fn upgrading(&self) -> usize {
        self.plants.values().filter(|p| p.full && !self.cooked.contains_key(&p.key()) && self.pending.contains_key(&p.key())).count()
    }

    /// The last failure, if any placed plant failed to cook.
    pub fn error(&self) -> Option<&str> {
        self.plants.values().find_map(|p| self.failed.get(&p.key()).or_else(|| self.failed.get(&p.preview_key())).map(String::as_str))
    }

    /// Attach every cooked placed plant to `scene` (a fresh scene with no
    /// vegetation yet): previews as plain meshes, full cooks as one GPU forest.
    pub fn attach(&self, scene: &mut render::scene::Scene) -> Result<usize, String> {
        let mut shown = 0;
        // Previews: each part mesh once for all plants, an instance per placement.
        let mut meshes: HashMap<(u64, Vec<u8>), usize> = HashMap::new();
        let mut materials: HashMap<Vec<u8>, u32> = HashMap::new();
        for p in self.plants.values() {
            // A full plant shows as its preview until its full cook lands.
            if p.full && matches!(self.cooked.get(&p.key()), Some(Cooked::Full(_))) { continue; }
            let Some(Cooked::Preview(v)) = self.cooked.get(&p.preview_key()) else { continue };
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
        Plant { preset: "pine", species: Species::jeffrey_pine(), look: Look::jeffrey_pine(), age, seed: 1, pos: Vec3::new(x, 0.0, 0.0), yaw: 0.0, full: false }
    }

    #[test]
    fn preset_placements_share_the_catalogue_source_geometry() {
        let form = render::plant_species::grown("mountain_birch").unwrap();
        let p = Plant { preset: "mountain_birch", species: form.rules, look: form.look, age: 5, seed: form.seed, ..pine(5, 0.0) };
        let expected = form.parts();
        let other_root = Plant { pos: Vec3::X, yaw: 1.0, full: true, ..p };
        assert_eq!(p.key(), other_root.preview_key());
        assert_ne!(p.key(), other_root.key());
        assert_ne!(p.key(), Plant { preset: "pine", ..p }.key());
        let (Cooked::Preview(preview), ..) = cook(p).unwrap() else { panic!("expected source meshes") };
        let foliage: Vec<_> = preview.meshes.iter().filter(|(_, m, _)| m.flags & render::scene::THIN_FOLIAGE != 0).collect();
        assert!(!foliage.is_empty());
        for (id, material, _) in foliage {
            assert!(expected.tufts.iter().any(|part| part.id == *id));
            assert_eq!(material.tint, form.look.needles);
        }
        let mut pine = pine(6, 0.0);
        let ordinary = pine.form().unwrap().parts().tufts[0].id;
        pine.look.twigs += 1;
        assert_ne!(ordinary, pine.form().unwrap().parts().tufts[0].id);
    }

    #[test]
    fn custom_growth_previews_preserve_catalogue_placements_with_game_overrides() {
        for name in ["dune_grass", "forest_fern", "dryas", "juniper"] {
            let mut expected = render::plant_species::grown(name).unwrap();
            expected.rules.height *= 1.6;
            expected.look.needles = [0.16, 0.24, 0.13];
            let p = Plant { preset: name, species: expected.rules, look: expected.look, age: expected.age,
                seed: 24, ..pine(5, 0.0) };
            let parts = expected.parts();
            let (source, _) = expected.assemble(&parts, p.age, p.seed, preview_limits()).unwrap();
            let branches = source.branch_transforms().unwrap();
            let (Cooked::Preview(preview), ..) = cook(p).unwrap() else { panic!("expected source meshes") };
            assert_eq!(preview.instances.len(), source.instances.len(), "{name}");
            assert!(preview.meshes.iter().any(|(_, m, _)| m.tint == p.look.needles), "{name}");
            if name == "juniper" {
                // Each needle-bearing tip also draws its woody axis and pale stomatal bands.
                let needle = preview.instances.iter().find(|(mesh, _)| preview.meshes[*mesh].1.tint == p.look.needles).unwrap();
                let companions: Vec<_> = preview.instances.iter().filter(|(_, world)| *world == needle.1)
                    .map(|(mesh, _)| &preview.meshes[*mesh].1).collect();
                assert!(companions.iter().any(|m| m.flags & render::scene::THIN_FOLIAGE == 0));
                assert!(companions.iter().any(|m| m.flags & render::scene::THIN_FOLIAGE != 0
                    && m.tint.into_iter().zip(p.look.needles).all(|(band, needle)| band > needle)));
            }
            for ((mesh, world), instance) in preview.instances.iter().zip(&source.instances) {
                let (part_id, material, _) = &preview.meshes[*mesh];
                assert_eq!(*part_id, source.parts[instance.part as usize].id);
                assert_eq!(material.tint, source.materials[instance.material as usize].tint);
                assert_eq!(*world, branches[instance.branch as usize] * instance.rest);
            }
        }
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
        let started = std::time::Instant::now();
        let mut form = render::plant_species::grown("pine").unwrap();
        form.rules.crowding = 60;
        form.rules.needle_years = 7;
        let p = preview(&form, &form.parts(), 31, 4).unwrap();
        println!("crowded 31 y: {} placed, {} triangles, {} ms", p.instances.len(), p.triangles, started.elapsed().as_millis());
    }
}
