//! Distant woody silhouettes for recipe plants. Keep the existing
//! leaf/twig card DAG. Only the solid wood, whose individual cylinders barely
//! simplify together, gets a whole-plant cut.
use glam::{Mat3, Mat4, Quat, Vec3};
use crate::{
    cluster::ClusterMesh,
    foliage_assembly::Assembly,
    plant_mesh_lod::PlantCards,
    plant_recipe::{self, BuildOptions, WoodyRecipe},
    primitives::{self, Primitive},
};

impl PlantCards {
    /// Card mesh with a distant replacement for its wood. Supply the recipe
    /// and seed used to cook these cards; material indices come from `materials`
    /// just as for `mesh_with_materials`. All original canopy cuts are retained.
    pub fn woody_mesh(&self, materials: &[u32], recipe: &WoodyRecipe, seed: u64) -> Result<ClusterMesh, String> {
        if materials.len() < self.wood.len() + self.cards.len() {
            return Err("woody card mesh is missing scene materials".into());
        }
        if self.mesh.clusters.iter().any(|c| c.material as usize >= self.wood.len() + self.cards.len()) {
            return Err("woody card mesh has an unknown material".into());
        }
        // Identify the wood in local material space *before* remapping:
        // callers can legitimately reuse the same scene material index.
        if self.wood.is_empty() || !self.mesh.clusters.iter().any(|c| c.material == 0) {
            return Ok(self.mesh_with_materials(materials));
        }
        let assembly = plant_recipe::build(recipe, BuildOptions::new(seed), plant_recipe::recipe_limits())?;
        let local: Vec<_> = (0..(self.wood.len() + self.cards.len()) as u32).collect();
        let mut mesh = add_distant_wood(self.mesh_with_materials(&local), &assembly, 0)?;
        for c in &mut mesh.clusters { c.material = materials[c.material as usize]; }
        Ok(mesh)
    }
}

fn add_distant_wood(mut mesh: ClusterMesh, assembly: &Assembly, material: u32) -> Result<ClusterMesh, String> {
    let (coarse, error) = wood(assembly, material)?;
    // Cluster references are offsets into the shared index/vertex arrays.
    // LOD relations use error thresholds and spheres, not cluster indices.
    let (wood_clusters, card_clusters): (Vec<_>, Vec<_>) =
        mesh.clusters.into_iter().partition(|c| c.material == material);
    mesh.clusters = wood_clusters;
    let coarse = ClusterMesh::build_fixed(&[coarse.part()], mesh.sphere);
    mesh = mesh.with_coarse(coarse, error);
    mesh.clusters.extend(card_clusters);
    Ok(mesh)
}

fn append_cylinder(wood: &mut Primitive, start: Vec3, end: Vec3, base: f32, tip: f32, sides: u32) {
    let axis = end - start;
    let pose = Mat4::from_scale_rotation_translation(
        Vec3::new(1.0, axis.length(), 1.0), Quat::from_rotation_arc(Vec3::Y, axis.normalize()), (start + end) * 0.5,
    );
    let normal = Mat3::from_mat4(pose).inverse().transpose();
    let cylinder = primitives::cylinder(wood.material, tip, base, sides);
    let offset = wood.vertices.len() as u32;
    wood.vertices.extend(cylinder.vertices.into_iter().map(|mut v| {
        v.pos = pose.transform_point3(Vec3::from(v.pos)).to_array();
        v.normal = (normal * Vec3::from(v.normal)).normalize().to_array();
        v
    }));
    wood.indices.extend(cylinder.indices.into_iter().map(|i| offset + i));
}

// A source section runs along local +Y. Its tapered cylinder has radius
// one at the base. Keep the recipe's actual seed, bends and branch angles.
#[derive(Clone, Copy)]
struct Section { start: Vec3, end: Vec3, radius: f32 }
impl Section {
    fn at(pose: Mat4) -> Self {
        Self { start: pose.transform_point3(Vec3::ZERO), end: pose.transform_point3(Vec3::Y), radius: pose.x_axis.truncate().length() }
    }
}

fn wood(assembly: &Assembly, material: u32) -> Result<(Primitive, f32), String> {
    let poses = assembly.branch_transforms()?;
    let mut wood = Primitive { material, ..Default::default() };
    // First section is the lower trunk of a woody recipe. Copy its original
    // vertices: the bark at ground level and the full collision-bearing waist
    // retain their exact radius, collar, embedding and material.
    let stem = assembly.instances.iter().find(|i| i.part == plant_recipe::STEM)
        .ok_or("woody recipe has no lower trunk")?;
    let base = &assembly.parts[stem.part as usize];
    let pose = poses[stem.branch as usize] * stem.rest;
    let normal = Mat3::from_mat4(pose).inverse().transpose();
    wood.vertices.extend(base.vertices.iter().copied().map(|mut v| {
        v.pos = pose.transform_point3(Vec3::from(v.pos)).to_array();
        v.normal = (normal * Vec3::from(v.normal)).normalize().to_array();
        v
    }));
    wood.indices.extend_from_slice(&base.indices);

    // Identify the solid upper trunk/limb segment by its assembly part; the
    // other woody parts are twigs already represented by the retained cards.
    let mut limbs = vec![Vec::new(); poses.len()];
    for instance in assembly.instances.iter().filter(|i| i.part == plant_recipe::SEGMENT) {
        // Primary limbs and their wind-articulated tips share a silhouette.
        let b = instance.branch as usize;
        let limb = match assembly.branches[b].parent {
            Some(parent) if parent != stem.branch => parent as usize,
            _ => b,
        };
        limbs[limb].push(Section::at(poses[b] * instance.rest));
    }
    let mut error = 0.0f32;
    for (i, sections) in limbs.iter().enumerate().filter(|(_, v)| !v.is_empty()) {
        // Two straight runs keep the bends in the exposed upper trunk. A
        // single tapered limb reaches the original tip of every primary
        // branch, including in bare sky gaps between sprays.
        let chunk = if i == stem.branch as usize { sections.len().div_ceil(2) } else { sections.len() };
        for run in sections.chunks(chunk) {
            let first = run[0];
            let last = run[run.len() - 1];
            let delta = last.end - first.start;
            let tip_radius = last.radius * plant_recipe::SEGMENT_TIP;
            append_cylinder(&mut wood, first.start, last.end, first.radius, tip_radius, 4);
            // Conservative silhouette displacement in metres: centreline sag
            // to the replacement chord plus the widest cross section's radius.
            // The original piece's axis is linear, so its furthest offset from
            // the chord is at an endpoint; the radius also bounds faceting and
            // taper. The usual per-view pixel threshold sets the switch.
            for section in run {
                for p in [section.start, section.end] {
                    let t = ((p - first.start).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
                    let distance = p.distance(first.start + delta * t);
                    error = error.max(distance + first.radius.max(section.radius));
                }
            }
        }
    }
    Ok((wood, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{biome::Biome, cluster::GpuCluster, recipe_forest};

    fn bounds(mesh: &ClusterMesh, choose: impl Fn(&GpuCluster) -> bool) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for cluster in mesh.clusters.iter().filter(|c| choose(c)) {
            for packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri + cluster.tri_count) as usize] {
                for shift in [0, 8, 16] {
                    let index = mesh.cluster_verts[(cluster.first_vert + ((packed >> shift) & 255)) as usize];
                    let p = Vec3::from(mesh.vertices[index as usize].pos);
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
        }
        (lo, hi)
    }

    #[test]
    fn biome_wood_cuts_keep_the_canopy_waist_and_height_at_lower_cost() {
        let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
        for biome in ["desert", "norway", "rainforest"] {
            for (name, recipe) in Biome::named(biome).unwrap().species() {
                let seed = 1;
                let cards = recipe_forest::cook_cards(&[(recipe, seed)], Some(&cache)).unwrap().pop().unwrap();
                let materials: Vec<_> = (30..).take(cards.materials(0).len()).collect();
                let source = cards.mesh_with_materials(&materials);
                let result = cards.woody_mesh(&materials, &recipe, seed).unwrap();
                let is_card = |c: &GpuCluster| c.material != materials[0];
                assert_eq!(result.leaf_triangles(), source.leaf_triangles(), "{name}: close geometry");
                let far_card = |c: &GpuCluster| c.parent_error > 1e30 && is_card(c);
                let old_cards = bounds(&source, far_card);
                assert!(old_cards.0.is_finite() && old_cards.1.y > old_cards.0.y);
                assert_eq!(bounds(&result, far_card), old_cards, "{name}: far canopy bounds");
                // Leaf/twig clusters retain their original geometry indices and LOD.
                let leaf_cuts = |mesh: &ClusterMesh| mesh.clusters.iter().filter(|c| is_card(c))
                    .map(|c| (c.first_tri, c.tri_count, c.first_vert, c.material,
                        c.own_error.to_bits(), c.parent_error.to_bits())).collect::<Vec<_>>();
                assert_eq!(leaf_cuts(&result), leaf_cuts(&source), "{name}: same card cut");
                // The previous terminal wood cut sometimes discarded the complete
                // lower stem. Compare the new far trunk against the authored foot.
                let old_wood = bounds(&source, |c| c.own_error == 0.0 && !is_card(c));
                let new_wood = bounds(&result, |c| c.parent_error > 1e30 && !is_card(c));
                assert!((new_wood.0.y-old_wood.0.y).abs() <= recipe.trunk_radius, "{name}: foot");
                assert!((new_wood.1.y-old_wood.1.y).abs() <= recipe.trunk_radius, "{name}: height");
                if recipe.protected_trunk_height > 0.0 {
                    let assembly = plant_recipe::build(&recipe, BuildOptions::new(seed), plant_recipe::recipe_limits()).unwrap();
                    let (coarse, _) = wood(&assembly, materials[0]).unwrap();
                    let waist_radius = |y: f32| coarse.vertices.iter().map(|v| Vec3::from(v.pos))
                        .filter(|p| (p.y-y).abs() < 0.02)
                        .map(|p| p.x.hypot(p.z)).fold(0.0, f32::max);
                    for y in [-0.5, recipe.protected_trunk_height] {
                        let r = waist_radius(y);
                        assert!((recipe.trunk_radius..=recipe.trunk_radius*1.04).contains(&r), "{name}: trunk waist at {y}");
                    }
                }
                assert!(result.root_triangles() * 4 < source.root_triangles(), "{name}: distant wood budget");
                eprintln!("{name}: {} -> {} coarsest triangles", source.root_triangles(), result.root_triangles());
            }
        }
    }
}
