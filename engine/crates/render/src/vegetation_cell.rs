//! Bounded spatial-cell authoring for the common vegetation assembly cooker.
//! Copies placement/branch records, never expanded vertices per world member.
use crate::foliage_assembly::{Assembly, BranchGroup, Limits};
use glam::{Mat3, Mat4};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Stable local identity, independent of input enumeration order.
    pub id: u64,
    /// Rigid cell-local placement, including terrain height and orientation.
    pub rest: Mat4,
}
fn identity(cell: u64, member: u64, local: u64) -> u64 {
    let mut h=cell ^ member.wrapping_mul(0x9e3779b97f4a7c15) ^ local.wrapping_mul(0xd1b54a32d192ed03);
    h=(h^(h>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
    h=(h^(h>>27)).wrapping_mul(0x94d049bb133111eb);h^(h>>31)
}
/// Compose repeated patches/trees into one root with articulated descendants.
/// The resulting CookedPlant can have a whole-cell voxel fallback, replacing
/// per-member world roots at distance. This does not choose visible cells.
/// Different species can use different cell catalogs; optical semantics remain
/// those of the source, never an inferred density multiplier.
pub fn compose(seed:u64, source:&Assembly, placements:&[Placement], limits:Limits)->Result<Assembly,String> {
    source.validate()?;
    if placements.is_empty() || source.instances.is_empty() {return Err("vegetation cell requires nonempty source and members".into());}
    let count=placements.len();
    let branches=(source.branches.len()-1).checked_mul(count).and_then(|n|n.checked_add(1)).ok_or("cell branch overflow")?;
    let instances=source.instances.len().checked_mul(count).ok_or("cell placement overflow")?;
    let vertices=source.instances.iter().try_fold(0usize,|n,p|n.checked_add(source.parts[p.part as usize].vertices.len())).ok_or("cell vertex overflow")?;
    let indices=source.instances.iter().try_fold(0usize,|n,p|n.checked_add(source.parts[p.part as usize].indices.len())).ok_or("cell index overflow")?;
    let vertices=vertices.checked_mul(count).ok_or("cell vertex overflow")?;
    let indices=indices.checked_mul(count).ok_or("cell index overflow")?;
    let bytes=vertices.checked_mul(std::mem::size_of::<crate::cluster::Vertex>()).and_then(|n|indices.checked_mul(4).and_then(|i|n.checked_add(i))).ok_or("cell source bytes overflow")?;
    if branches>u32::MAX as usize || instances>u32::MAX as usize || branches>limits.max_branches || instances>limits.max_instances || source.parts.len()>limits.max_parts
        || vertices>limits.max_vertices || indices>limits.max_indices || bytes>limits.max_source_bytes {
        return Err("vegetation cell exceeds explicit assembly budget".into());
    }
    let mut ids=BTreeSet::new();
    for p in placements {
        let r=Mat3::from_mat4(p.rest);
        if !ids.insert(p.id) || !p.rest.is_finite() || p.rest.row(3)!=glam::Vec4::W
            || r.determinant()<=0. || !(r.transpose()*r).abs_diff_eq(Mat3::IDENTITY,1e-4) {
            return Err("invalid vegetation cell identity or rigid placement".into());
        }
    }
    let mut ordered=placements.to_vec();ordered.sort_by_key(|p|p.id);
    let mut out_branches=Vec::with_capacity(branches);
    out_branches.push(BranchGroup {id:identity(seed,0,0),parent:None,rest:Mat4::IDENTITY,stiffness:1.});
    let mut out_instances=Vec::with_capacity(instances);
    for p in ordered {
        let base=out_branches.len() as u32;
        let remap=|b:u32|if b==0 {0} else {base+b-1};
        // Merge each motionless source root into the cell root. Adding a new
        // animated ancestor would move basal anchors and inflate pose demand.
        for b in source.branches.iter().skip(1) {
            let parent=b.parent.unwrap();
            out_branches.push(BranchGroup {id:identity(seed,p.id,b.id),parent:Some(remap(parent)),
                rest:if parent==0 {p.rest*b.rest} else {b.rest},stiffness:b.stiffness});
        }
        for instance in &source.instances {
            let mut item=instance.clone();item.id=identity(seed,p.id,item.id);
            if item.branch==0 {item.rest=p.rest*item.rest;}
            item.branch=remap(item.branch);out_instances.push(item);
        }
    }
    let mut parameters=source.parameters;parameters.seed=seed;
    Assembly::from_parts(parameters,source.parts.clone(),out_branches,out_instances,source.materials.clone(),limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_assembly::{generate_meadow_patch_budget,MeadowBladeBudget,Parameters};
    use glam::{Quat,Vec3};
    fn limits()->Limits {Limits {max_branches:64,max_instances:2048,max_vertices:100_000,max_indices:300_000,max_source_bytes:8*1024*1024,..Default::default()}}
    #[test]
    fn cells_share_geometry_and_preserve_every_transformed_source_vertex() {
        let source=generate_meadow_patch_budget(Parameters::meadow_patch(47),MeadowBladeBudget::Blades120,Limits::meadow_patch()).unwrap();
        let members=[Placement {id:20,rest:Mat4::from_rotation_translation(Quat::from_rotation_y(0.7),Vec3::new(1.2,0.4,0.3))},
            Placement {id:10,rest:Mat4::from_rotation_translation(Quat::from_rotation_x(0.2),Vec3::new(-0.5,0.1,0.2))}];
        let cell=compose(123,&source,&members,limits()).unwrap();
        assert_eq!(cell.parts.len(),3);assert_eq!(cell.instances.len(),240);assert_eq!(cell.branches.len(),9);
        let original=source.flatten_rest().unwrap();let expanded=cell.flatten_rest().unwrap();
        for (member,meshes) in [members[1],members[0]].iter().zip(expanded.chunks(120)) {
            for (actual,reference) in meshes.iter().zip(&original) {
                assert_eq!(actual.indices,reference.indices);assert_eq!(actual.material,reference.material);
                for (a,b) in actual.vertices.iter().zip(&reference.vertices) {
                    assert!(Vec3::from(a.pos).abs_diff_eq(member.rest.transform_point3(Vec3::from(b.pos)),1e-5));
                    assert!(Vec3::from(a.normal).abs_diff_eq(member.rest.transform_vector3(Vec3::from(b.normal)),1e-5));
                }
            }
        }
        let reversed=compose(123,&source,&[members[1],members[0]],limits()).unwrap();
        assert_eq!(cell.instances.iter().map(|p|p.id).collect::<Vec<_>>(),reversed.instances.iter().map(|p|p.id).collect::<Vec<_>>());
        assert_eq!(cell.branches.iter().map(|p|p.id).collect::<Vec<_>>(),reversed.branches.iter().map(|p|p.id).collect::<Vec<_>>());
        let config=crate::vegetation_aggregate::Config {reference_cell_size:0.05,..Default::default()};
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&cell,config).unwrap();
        cooked.cook_proxy(&cell,config,5).unwrap();
        assert_eq!(cooked.stats().primitive_meshes,9);assert_eq!(cooked.stats().placements,240);
        let mut scene=crate::Scene::default();
        crate::foliage_runtime::plant_species_population(&mut scene,
            &[crate::foliage_runtime::Species {plant:&cooked,category:crate::scene::GeometryCategory::Grass,deformation:crate::foliage_runtime::Deformation::BasalShear}],
            &[crate::foliage_population::WorldRoot {id:123,placement:crate::foliage_runtime::RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}}],
            1,16,crate::aggregate_lod::Policy {allow_sampled_optics:true,..Default::default()}).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();
        assert_eq!(spec.roots.len(),1);assert_eq!(scene.voxel_proxies,2,"camera and shadow root records share one cell asset");
        assert_eq!(cooked.stats().branches,9);assert!(scene.meadow.is_none());
    }
    #[test]
    fn rejects_duplicate_ids_nonrigid_placements_and_overbudget_cells() {
        let source=generate_meadow_patch_budget(Parameters::meadow_patch(47),MeadowBladeBudget::Blades120,Limits::meadow_patch()).unwrap();
        let p=Placement {id:1,rest:Mat4::IDENTITY};
        assert!(compose(1,&source,&[],limits()).is_err());
        assert!(compose(1,&source,&[p,p],limits()).is_err());
        assert!(compose(1,&source,&[Placement {rest:Mat4::from_scale(Vec3::splat(2.)),..p}],limits()).is_err());
        assert!(compose(1,&source,&[p],Limits {max_instances:119,..limits()}).is_err());
        assert!(compose(1,&source,&[p],Limits {max_branches:4,..limits()}).is_err());
    }
}
