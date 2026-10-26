//! Whole live evergreen sources for the experimental shared world-volume path.
//! The renderer owns selection/traversal. This adapter only cooks the exact
//! three game assets and binds their existing whole source owners, once.
use render::{Scene, cluster::Part,
    foliage_volume_data::{VolumeBinding, ALL_MATERIALS},
    foliage_voxel::{bake, BakeConfig, ProjectionMeasure, VoxelAsset},
    foliage_world::{WorldConfig, WorldField},
    scene::{ALPHA_TEST, BLEND, FOLIAGE, GeometryCategory, Material, THIN_FOLIAGE, TWO_SIDED},
};
use std::{collections::HashSet, path::Path, time::Instant};

#[derive(Clone, Copy, Debug)]
pub struct TreeMaterials { pub trunk_a:u32, pub trunk_b:u32, pub bark:u32, pub crown:u32 }

impl TreeMaterials {
    pub fn surface(self, name:&str) -> u32 {
        if name.contains("trunk_b") { self.trunk_b }
        else if name.contains("trunk") { self.trunk_a }
        else if name.contains("bark") { self.bark }
        else { self.crown }
    }

    fn bake_materials(self, source:&[Material]) -> Result<Vec<Material>,String> {
        let ids=[self.trunk_a,self.trunk_b,self.bark,self.crown];
        if ids.iter().copied().collect::<HashSet<_>>().len()!=4 {
            return Err("whole-tree cook requires distinct crown and wood materials".into());
        }
        for id in ids {
            let m=source.get(id as usize).ok_or("whole-tree material out of bounds")?;
            if m.flags&BLEND!=0 || m.metal!=0.0 {
                return Err("whole-tree volume currently supports opaque/cutout dielectric source materials".into());
            }
        }
        let crown=source[self.crown as usize];
        if crown.flags&(ALPHA_TEST|TWO_SIDED)!=(ALPHA_TEST|TWO_SIDED) {
            return Err("live whole-tree cook expects the authored two-sided cutout crown".into());
        }
        let mut materials=source.to_vec();
        // Source shading recognizes these needles using ALPHA_TEST|TWO_SIDED
        // plus the FOLIAGE owner flag. The general baker has no owner context:
        // annotate its material copy to preserve the leaf-vs-bark moment. In
        // the baker this flag changes only leaf_area, not source optics/data.
        // Never set it on Scene.materials: that would change source lighting.
        materials[self.crown as usize].flags|=THIN_FOLIAGE;
        Ok(materials)
    }
}

/// Includes bark, trunk and every crown triangle, after the same fullness and
/// archetype deformation as visuals.rs. No opaque density/alpha/tint boost.
/// The mixed open-card/wood source uses SheetCrossings, not a blanket closed
/// surface area halving. Correlated opaque coverage remains an approximation.
pub fn cook_assets(scene:&Scene, root:&Path, fullness:f32, materials:TreeMaterials,
    config:BakeConfig) -> Result<Vec<VoxelAsset>,String> {
    if config.projection_measure!=ProjectionMeasure::SheetCrossings {
        return Err("whole live trees require SheetCrossings for their cutout leaf sheets".into());
    }
    let bake_materials=materials.bake_materials(&scene.materials)?;
    sideways::vegetation::TreeArchetype::ALL.into_iter().map(|archetype| {
        let surfaces=crate::tree_assets::load_with_fullness(root,archetype,fullness)?;
        let parts:Vec<_>=surfaces.iter().map(|(name,vertices,indices)|Part {
            vertices, indices, material:materials.surface(name),
        }).collect();
        let asset=bake(&parts,&bake_materials,&scene.textures,config)?;
        if asset.levels.is_empty() || asset.levels[0].voxels.is_empty() {
            return Err(format!("empty whole-tree optical asset: {}",archetype.label()));
        }
        Ok(asset)
    }).collect()
}

pub struct BuiltWorld {
    pub field:WorldField,
    pub metadata:serde_json::Value,
    pub summary:String,
}

/// Builds a complete provider transaction without mutating source geometry,
/// wind flags, physics, old voxel masks, or the Scene. The caller publishes the
/// returned field only after successful construction/renderer validation.
pub fn build(scene:&Scene, tree_meshes:&[usize], root:&Path, fullness:f32,
    materials:TreeMaterials, bake_config:BakeConfig, config:WorldConfig) -> Result<BuiltWorld,String> {
    let started=Instant::now();
    if tree_meshes.len()!=3 { return Err("whole-tree world requires the three live evergreen source meshes".into()); }
    let mut ranges=Vec::new();
    for &mesh_id in tree_meshes {
        let mesh=scene.meshes.get(mesh_id).ok_or("whole-tree mesh out of bounds")?;
        let range=(mesh.first_cluster,mesh.cluster_count);
        if mesh.cluster_count==0 || ranges.contains(&range) { return Err("invalid or duplicate whole-tree source mesh".into()); }
        let end=mesh.first_cluster.checked_add(mesh.cluster_count).ok_or("whole-tree cluster range overflow")?;
        let clusters=scene.clusters.get(mesh.first_cluster as usize..end as usize).ok_or("whole-tree cluster range out of bounds")?;
        let allowed=[materials.trunk_a,materials.trunk_b,materials.bark,materials.crown];
        if clusters.iter().any(|c|!allowed.contains(&c.material)) {
            return Err("whole-tree source mesh uses material absent from its optical recipe".into());
        }
        ranges.push(range);
    }
    let mut bindings=Vec::new();
    let mut family_counts=[0usize;3];
    for (owner,i) in scene.instances.iter().enumerate() {
        if i.category()!=GeometryCategory::Trees || i.cluster_count==0 {continue;}
        let Some(asset)=ranges.iter().position(|r|*r==(i.first_cluster,i.cluster_count)) else {continue;};
        if i.flags&FOLIAGE==0 {return Err("live tree source lacks its foliage material/wind identity".into());}
        bindings.push(VolumeBinding{owner:u32::try_from(owner).map_err(|_|"whole-tree owner overflow")?,
            asset:asset as u32,crown_material:ALL_MATERIALS});
        family_counts[asset]+=1;
    }
    let assets=cook_assets(scene,root,fullness,materials,bake_config)?;
    let field=WorldField::build(&assets,&bindings,&scene.instances,config)?;
    let asset_metadata:Vec<_>=assets.iter().enumerate().map(|(index,a)|serde_json::json!({
        "archetype":sideways::vegetation::TreeArchetype::ALL[index].label(),
        "source_triangles":a.stats.triangles,"quadrature_samples":a.stats.samples,
        "cell_metres":a.levels[0].cell_size,"levels":a.levels.len(),
        "occupied_cells":a.levels.iter().map(|l|l.voxels.len()).collect::<Vec<_>>(),
        "source_bounds":a.source_bounds,"projection_measure":"sheet_crossings",
    })).collect();
    let grid_metadata=serde_json::json!({"origin":field.grid.origin,"dimensions":field.grid.dimensions,
        "cell_metres":field.grid.cell_metres,"cells":field.grid.spans.len(),"references":field.grid.references.len(),
        "bytes":field.grid.byte_len(),"maximum_references_per_cell":field.grid.spans.iter().map(|s|s[1]).max().unwrap_or(0)});
    let elapsed_ms=started.elapsed().as_secs_f64()*1000.0;
    let metadata=serde_json::json!({"provider":"shared_whole_tree_voxels_world_grid",
        "fullness":fullness,"tree_count":bindings.len(),"family_counts":family_counts,
        "groups":field.groups.len(),"assets":asset_metadata,"grid":grid_metadata,
        "provider_bytes":field.byte_len(),"maximum_wind_shear":field.max_wind_shear,
        "source_material_ids":[materials.trunk_a,materials.trunk_b,materials.bark,materials.crown],
        "whole_owner":true,"source_wind_and_shadow_owners_retained":true,"construction_ms":elapsed_ms,
        "optical_equivalence_validated":false,"persistent_asset_cache":false});
    let summary=format!("EXPERIMENTAL world foliage: {} complete tree owners, {} canonical groups, 3 shared optical assets, {} spatial references, {} provider bytes; {:.0} ms cook/pack; source-derived density/material/normal mips, source wind and shadow owners retained; optical acceptance pending",
        bindings.len(),field.groups.len(),field.grid.references.len(),field.byte_len(),elapsed_ms);
    Ok(BuiltWorld{field,metadata,summary})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn materials()->Vec<Material> {(0..4).map(|i|Material{tint:[0.2,0.5,0.1],layer:-1,
        roughness:0.9,flags:if i==3 {ALPHA_TEST|TWO_SIDED}else{0},scale:1.0,metal:0.0}).collect()}
    fn ids()->TreeMaterials {TreeMaterials{trunk_a:0,trunk_b:1,bark:2,crown:3}}
    #[test]
    fn exact_surface_material_mapping_and_bake_only_leaf_semantics() {
        assert_eq!(ids().surface("trunk_b.001"),1); assert_eq!(ids().surface("trunk_a"),0);
        assert_eq!(ids().surface("stout_bark"),2); assert_eq!(ids().surface("pine_crown"),3);
        let source=materials();let cooked=ids().bake_materials(&source).unwrap();
        for i in 0..4 {
            assert_eq!(source[i].tint,cooked[i].tint); assert_eq!(source[i].roughness,cooked[i].roughness);
            assert_eq!(source[i].layer,cooked[i].layer);
            assert_eq!(source[i].flags,cooked[i].flags&!THIN_FOLIAGE);
        }
        assert_eq!(cooked[3].flags,ALPHA_TEST|TWO_SIDED|THIN_FOLIAGE);
        assert_eq!(source[3].flags,ALPHA_TEST|TWO_SIDED);
    }
    #[test]
    fn cannot_silently_bake_blended_or_aliased_crown_materials() {
        let mut source=materials();source[0].flags=BLEND;
        assert!(ids().bake_materials(&source).is_err());
        assert!(TreeMaterials{crown:0,..ids()}.bake_materials(&materials()).is_err());
    }
}
