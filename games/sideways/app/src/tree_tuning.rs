//! Explicit tree authoring transaction for one existing stage.
//!
//! A renderer must be rebuilt after `apply_scene`: canopy mass bounds, shadow
//! pages, wind envelopes and every source CLOD refer to the edited instances.
//! Commit matching trunk physics only after that renderer is ready. This is
//! never an update_instances shortcut or a new population generator.
use render::scene::{Scene, GpuInstance, Material, GeometryCategory, FOLIAGE, ALPHA_TEST, THIN_FOLIAGE};
use sideways::physics::{Physics, TreeColliderSource, TreeColliderBaseline};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub size_scale: f32,
    /// Stable subset of the existing forest; 1 means all authored roots.
    pub density: f32,
    /// Linear leaf pigment multipliers; bark, alpha and lighting stay physical.
    pub color_multiplier: [f32; 3],
}
impl Default for Settings {
    fn default() -> Self { Self { size_scale:1.0, density:1.0, color_multiplier:[1.0;3] } }
}
impl Settings {
    pub fn validate(self) -> Result<(), String> {
        if !self.size_scale.is_finite() || !(0.5..=1.5).contains(&self.size_scale) {
            return Err("tree size must be finite and in 0.5..=1.5".into());
        }
        if !self.density.is_finite() || !(0.0..=1.0).contains(&self.density) {
            return Err("tree population must be finite and in 0..=1 of the existing forest".into());
        }
        if self.color_multiplier.iter().any(|v| !v.is_finite() || !(0.25..=2.0).contains(v)) {
            return Err("tree leaf RGB multipliers must be finite and in 0.25..=2".into());
        }
        Ok(())
    }
}

struct Tree {
    owner: usize,
    original: GpuInstance,
    root: [f32;3],
    rank: f64,
}

pub struct Baseline {
    trees: Vec<Tree>,
    crowns: Vec<(usize,Material)>,
    physics: TreeColliderBaseline,
    instance_count: usize,
    material_count: usize,
    cluster_count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub total_trees: usize,
    pub visible_trees: usize,
    pub native_colliders: usize,
    pub crown_materials: usize,
    pub settings: Settings,
}

fn mix(mut v:u64)->u64 {
    v=(v^(v>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v=(v^(v>>27)).wrapping_mul(0x94d049bb133111eb);
    v^(v>>31)
}
fn root_rank(root:[f32;3])->f64 {
    let key=(root[0].to_bits() as u64)^((root[2].to_bits() as u64)<<32)^0x7e55_1a8d_0ea4_6c39;
    (mix(key)>>11) as f64 / 9_007_199_254_740_992.0
}
fn scale_of(instance:&GpuInstance)->Result<[f32;2],String> {
    let x=glam::Vec3::new(instance.rows[0][0],instance.rows[1][0],instance.rows[2][0]);
    let y=glam::Vec3::new(instance.rows[0][1],instance.rows[1][1],instance.rows[2][1]);
    let z=glam::Vec3::new(instance.rows[0][2],instance.rows[1][2],instance.rows[2][2]);
    let sx=x.length();let sy=y.length();let sz=z.length();
    if !sx.is_finite() || !sy.is_finite() || !sz.is_finite() || sx<=0.0 || sy<=0.0 || sz<=0.0
        || (sx-sz).abs()>sx.max(sz)*1e-5 || y.x.abs()>sy*1e-5 || y.z.abs()>sy*1e-5
        || x.y.abs()>sx*1e-5 || z.y.abs()>sz*1e-5 || y.y<=0.0
        || x.cross(y).dot(z)<=0.0
    {return Err("tree tuning requires upright source instances with positive equal horizontal scales".into());}
    Ok([sx,sy])
}

impl Baseline {
    fn unsupported_scene(scene:&Scene)->bool {
        scene.voxel_forest.is_some() || scene.foliage_world.is_some() || !scene.foliage_representations.is_empty()
            || !scene.foliage_source_groups.is_empty() || !scene.part_wind.is_empty() || !scene.part_attachments.is_empty()
    }

    /// A treeless stage has no tree settings to apply. In an experimental
    /// voxel/branch scene only a neutral preset can leave its assets untouched.
    pub fn prepare_optional_stage(scene:&mut Scene,physics:&mut Physics,settings:Settings)->Result<Option<Self>,String> {
        settings.validate()?;
        let no_trees=!scene.instances.iter().any(|i|i.category()==GeometryCategory::Trees && i.cluster_count>0);
        if Self::unsupported_scene(scene) {
            if settings == Settings::default() { return Ok(None); }
        } else if no_trees { return Ok(None); }
        Self::prepare_stage(scene,physics,settings).map(Some)
    }

    pub fn stage_metadata(baseline:Option<&Self>,settings:Settings)->serde_json::Value {
        match baseline {
            Some(baseline)=>{let mut value=baseline.metadata(settings);value["editing_available"]=true.into();value},
            None=>serde_json::json!({"editing_available":false,"size_scale":settings.size_scale,
                "population_fraction":settings.density,"leaf_rgb_multiplier":settings.color_multiplier,
                "scope":"No editable source trees in this scene."}),
        }
    }

    /// Start a newly built stage from absolute game settings before creating
    /// any renderer resources. Retain the authored source/collider baseline
    /// so subsequent Apply/Reset never compounds scale or population removal.
    pub fn prepare_stage(scene:&mut Scene,physics:&mut Physics,settings:Settings)->Result<Self,String> {
        settings.validate()?;
        let baseline=Self::capture(scene,physics)?;
        baseline.apply_scene(scene,settings)?;
        baseline.apply_physics(physics,settings)?;
        Ok(baseline)
    }

    pub fn capture(scene:&Scene,physics:&Physics)->Result<Self,String> {
        // Packed voxel transforms, branch ownership and their optical assets
        // need a different transaction. Fail explicitly rather than tune only
        // one of those representations.
        if Self::unsupported_scene(scene)
        {return Err("tree editing is unavailable for experimental packed voxel/branch/world scenes; rebuild their source and voxel assets together, or use SIDEWAYS_RENDER_BASELINE=authored to leave them unedited".into());}
        let mut trees=Vec::new();let mut sources=Vec::new();let mut crown_ids=BTreeSet::new();
        for (owner,instance) in scene.instances.iter().enumerate() {
            if instance.category()!=GeometryCategory::Trees {continue;}
            if instance.cluster_count==0 {continue;}
            if instance.rows.iter().flatten().chain(instance.sphere.iter()).any(|v|!v.is_finite())
                || !instance.scale.is_finite() || instance.scale<=0.0 || instance.sphere[3]<=0.0
            {return Err("tree source transform or culling sphere is invalid".into());}
            let root=[instance.rows[0][3],instance.rows[1][3],instance.rows[2][3]];
            let scale=scale_of(instance)?;
            let first=instance.first_cluster as usize;
            let end=first.checked_add(instance.cluster_count as usize).ok_or("tree cluster range overflow")?;
            let clusters=scene.clusters.get(first..end).ok_or("tree cluster range exceeds source scene")?;
            for cluster in clusters {
                let id=cluster.material as usize;
                let material=scene.materials.get(id).ok_or("tree source material is absent")?;
                if material.flags&(ALPHA_TEST|THIN_FOLIAGE)!=0 {crown_ids.insert(id);}
            }
            trees.push(Tree{owner,original:*instance,root,rank:root_rank(root)});
            sources.push(TreeColliderSource{root,scale});
        }
        if trees.is_empty() {return Err("this scene contains no editable source trees".into());}
        // A crown tint must never recolour unrelated cars, terrain or grass.
        for instance in &scene.instances {
            if instance.category()==GeometryCategory::Trees || instance.cluster_count==0 {continue;}
            let first=instance.first_cluster as usize;
            let end=first.checked_add(instance.cluster_count as usize).ok_or("instance cluster range overflow")?;
            for cluster in scene.clusters.get(first..end).ok_or("instance cluster range exceeds scene")? {
                if crown_ids.contains(&(cluster.material as usize)) {
                    return Err("tree crown material is shared with non-tree geometry; split that material before editing".into());
                }
            }
        }
        let physics=TreeColliderBaseline::capture(physics,&sources)?;
        let crowns=crown_ids.into_iter().map(|id|(id,scene.materials[id])).collect();
        Ok(Self{trees,crowns,physics,instance_count:scene.instances.len(),
            material_count:scene.materials.len(),cluster_count:scene.clusters.len()})
    }

    fn selected(&self,settings:Settings)->Vec<bool> {
        self.trees.iter().map(|tree| settings.density==1.0 || tree.rank<f64::from(settings.density)).collect()
    }

    /// Restore original bytes first, then apply the selected absolute settings.
    /// No allocation or instance-index changes: dynamic cars retain their IDs.
    pub fn apply_scene(&self,scene:&mut Scene,settings:Settings)->Result<Report,String> {
        settings.validate()?;
        if scene.instances.len()!=self.instance_count || scene.materials.len()!=self.material_count
            || scene.clusters.len()!=self.cluster_count
        {return Err("tree baseline belongs to a different scene; capture it again after stage construction".into());}
        for tree in &self.trees {
            let current=&scene.instances[tree.owner];
            if current.category()!=GeometryCategory::Trees || current.first_cluster!=tree.original.first_cluster
                || [current.rows[0][3],current.rows[1][3],current.rows[2][3]]!=tree.root
            {return Err("tree instance identity changed after baseline capture".into());}
        }
        let selected=self.selected(settings);
        for (tree,&enabled) in self.trees.iter().zip(&selected) {
            let mut instance=tree.original;
            if settings.size_scale!=1.0 {
                for row in &mut instance.rows {for value in &mut row[..3] {*value*=settings.size_scale;}}
                for axis in 0..3 {
                    instance.sphere[axis]=tree.root[axis]+(instance.sphere[axis]-tree.root[axis])*settings.size_scale;
                }
                instance.sphere[3]*=settings.size_scale;
                instance.scale*=settings.size_scale;
            }
            if !enabled {
                instance.cluster_count=0;
                instance.flags&=!FOLIAGE;
            }
            scene.instances[tree.owner]=instance;
        }
        for &(id,original) in &self.crowns {
            let mut material=original;
            for channel in 0..3 {material.tint[channel]*=settings.color_multiplier[channel];}
            scene.materials[id]=material;
        }
        Ok(Report{total_trees:self.trees.len(),visible_trees:selected.iter().filter(|v|**v).count(),
            native_colliders:self.physics.len(),crown_materials:self.crowns.len(),settings})
    }

    /// Commit after the replacement renderer has successfully reconstructed all
    /// foliage resources. Car bodies, state, tick frequency and road stay put.
    pub fn apply_physics(&self,physics:&mut Physics,settings:Settings)->Result<(),String> {
        settings.validate()?;
        self.physics.apply(physics,settings.size_scale,&self.selected(settings))
    }

    pub fn metadata(&self,settings:Settings)->serde_json::Value {
        serde_json::json!({"size_scale":settings.size_scale,"population_fraction":settings.density,
            "leaf_rgb_multiplier":settings.color_multiplier,"total_source_trees":self.trees.len(),
            "selected_source_trees":self.selected(settings).iter().filter(|v|**v).count(),
            "matched_native_trunks":self.physics.len(),"crown_materials":self.crowns.len(),
            "requires_renderer_rebuild":true,"physics_commit_required":true,
            "scope":"Same-stage source trees at every distance, deterministic subset of existing roots; new roots above100% are not generated. Background trees retain their original absence of driving collision."})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render::cluster::{ClusterMesh,Part,Vertex};
    use glam::{Affine3A,Quat,Vec3};

    fn fixture()->(Scene,Physics) {
        let mut scene=Scene::default();
        let leaf=scene.add_material(Material{tint:[0.4,0.6,0.3],layer:-1,roughness:0.9,
            flags:ALPHA_TEST,scale:1.0,metal:0.0});
        let wood=scene.add_material(Material{tint:[0.3,0.2,0.1],layer:-1,roughness:1.0,
            flags:0,scale:1.0,metal:0.0});
        let vertices=[[-1.,1.,0.],[1.,1.,0.],[0.,3.,0.]].map(|pos|Vertex{pos,normal:[0.,0.,1.],uv:[0.;2]});
        let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2],material:leaf}]));
        let bark=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2],material:wood}]));
        let mut physics=Physics::new();
        for i in 0..16 {
            let root=[i as f32*7.0,2.0,13.0];let scale=0.8+i as f32*0.02;
            scene.add_classified_instance(mesh,Affine3A::from_scale_rotation_translation(Vec3::splat(scale),
                Quat::from_rotation_y(i as f32*0.21),Vec3::from_array(root)),GeometryCategory::Trees);
            scene.mark_foliage();
            if i<4 {physics.add_trunk(&sideways::builder::Spot{pos:sideways::builder::V3Ser(root[0],root[1],root[2]),
                yaw:0.0,scale,scale_y:scale});}
        }
        scene.add_classified_instance(bark,Affine3A::IDENTITY,GeometryCategory::Other);
        (scene,physics)
    }

    #[test]
    fn tree_settings_are_bounded_and_default_is_exact_baseline() {
        let (mut scene,mut physics)=fixture();let baseline=Baseline::capture(&scene,&physics).unwrap();
        let original=scene.instances.clone();let original_material=scene.materials.clone();
        let changed=Settings{size_scale:1.4,density:0.4,color_multiplier:[1.2,0.8,1.1]};
        let report=baseline.apply_scene(&mut scene,changed).unwrap();
        assert_eq!(report.total_trees,16);assert!(report.visible_trees>0&&report.visible_trees<16);
        assert_eq!(report.native_colliders,4);baseline.apply_physics(&mut physics,changed).unwrap();
        baseline.apply_scene(&mut scene,Settings::default()).unwrap();
        baseline.apply_physics(&mut physics,Settings::default()).unwrap();
        for (a,b) in scene.instances.iter().zip(&original) {
            assert_eq!(a.rows,b.rows);assert_eq!(a.sphere,b.sphere);assert_eq!(a.first_cluster,b.first_cluster);
            assert_eq!(a.cluster_count,b.cluster_count);assert_eq!(a.flags,b.flags);assert_eq!(a.scale,b.scale);
        }
        for (a,b) in scene.materials.iter().zip(&original_material) {
            assert_eq!(a.tint,b.tint);assert_eq!(a.flags,b.flags);assert_eq!(a.roughness,b.roughness);
        }
        for bad in [Settings{size_scale:f32::NAN,..Default::default()},Settings{density:1.01,..Default::default()},
            Settings{color_multiplier:[1.,f32::INFINITY,1.],..Default::default()}] {
            assert!(baseline.apply_scene(&mut scene,bad).is_err());
        }
    }

    #[test]
    fn subsets_are_world_stable_monotonic_and_disabled_trees_cast_no_canopy() {
        let (mut scene,physics)=fixture();let baseline=Baseline::capture(&scene,&physics).unwrap();
        let low=baseline.selected(Settings{density:0.25,..Default::default()});
        let high=baseline.selected(Settings{density:0.75,..Default::default()});
        assert!(low.iter().zip(&high).all(|(&a,&b)|!a||b));
        let unrelated=scene.instances.last().unwrap().rows;
        baseline.apply_scene(&mut scene,Settings{density:0.0,..Default::default()}).unwrap();
        assert!(scene.instances.iter().filter(|i|i.category()==GeometryCategory::Trees)
            .all(|i|i.cluster_count==0&&i.flags&FOLIAGE==0));
        assert_eq!(scene.instances.last().unwrap().rows,unrelated);
        assert_eq!(root_rank([10.,2.,20.]),root_rank([10.,99.,20.]));
    }

    #[test]
    fn stage_baseline_is_applied_once_and_remains_an_absolute_reset_reference() {
        let settings=Settings{size_scale:1.5,density:0.45,color_multiplier:[1.0,1.15,0.9]};
        let (mut scene,mut physics)=fixture();let original=scene.instances.clone();let colors=scene.materials.clone();
        let baseline=Baseline::prepare_stage(&mut scene,&mut physics,settings).unwrap();
        let tuned=scene.instances.clone();let tuned_materials=scene.materials.clone();
        assert_eq!(baseline.metadata(settings)["total_source_trees"],16);
        for (tree,instance) in baseline.trees.iter().zip(&scene.instances) {
            assert_eq!(tree.original.rows,original[tree.owner].rows,"retain authored bytes, not scaled stage bytes");
            assert_eq!(instance.scale,tree.original.scale*1.5);
        }
        // Repeat Apply/Reset never scales already edited trees a second time.
        baseline.apply_scene(&mut scene,Settings{size_scale:0.7,density:0.2,color_multiplier:[0.8;3]}).unwrap();
        baseline.apply_scene(&mut scene,settings).unwrap();baseline.apply_physics(&mut physics,settings).unwrap();
        for (a,b) in scene.instances.iter().zip(&tuned) {
            assert_eq!(a.rows,b.rows);assert_eq!(a.sphere,b.sphere);assert_eq!(a.cluster_count,b.cluster_count);
        }
        for (a,b) in scene.materials.iter().zip(&tuned_materials) {assert_eq!(a.tint,b.tint);}
        baseline.apply_scene(&mut scene,Settings::default()).unwrap();baseline.apply_physics(&mut physics,Settings::default()).unwrap();
        for (a,b) in scene.instances.iter().zip(&original) {assert_eq!(a.rows,b.rows);assert_eq!(a.cluster_count,b.cluster_count);}
        for (a,b) in scene.materials.iter().zip(&colors) {assert_eq!(a.tint,b.tint);}
        // A newly constructed stage receives its own authored baseline.
        let (mut next_scene,mut next_physics)=fixture();
        let next=Baseline::prepare_stage(&mut next_scene,&mut next_physics,settings).unwrap();
        assert_eq!(next.metadata(settings),baseline.metadata(settings));
        for (a,b) in next_scene.instances.iter().zip(&tuned) {assert_eq!(a.rows,b.rows);assert_eq!(a.cluster_count,b.cluster_count);}
    }

    #[test]
    fn explicit_neutral_preset_leaves_unsupported_scene_untouched_without_hiding_edits() {
        let (mut scene,mut physics)=fixture();let original=scene.instances.clone();
        scene.part_attachments.push(render::part_wind::PartAttachment {pose:0,owner:0,rest:glam::Mat4::IDENTITY});
        let baseline=Baseline::prepare_optional_stage(&mut scene,&mut physics,Settings::default()).unwrap();
        assert!(baseline.is_none());
        assert_eq!(Baseline::stage_metadata(baseline.as_ref(),Settings::default())["editing_available"],false);
        assert!(Baseline::prepare_optional_stage(&mut scene,&mut physics,Settings{size_scale:1.5,..Default::default()}).is_err());
        for (a,b) in scene.instances.iter().zip(&original) {assert_eq!(a.rows,b.rows);assert_eq!(a.sphere,b.sphere);assert_eq!(a.cluster_count,b.cluster_count);}
        assert!(Baseline::prepare_optional_stage(&mut Scene::default(),&mut Physics::new(),Settings::default()).unwrap().is_none());
        let settings = Settings { size_scale: 1.5, density: 0.45, ..Settings::default() };
        assert!(Baseline::prepare_optional_stage(&mut Scene::default(),&mut Physics::new(),settings).unwrap().is_none());
    }

    #[test]
    fn uniform_root_scaling_keeps_transformed_source_inside_scaled_sphere() {
        let (mut scene,physics)=fixture();let baseline=Baseline::capture(&scene,&physics).unwrap();
        baseline.apply_scene(&mut scene,Settings{size_scale:1.5,..Default::default()}).unwrap();
        for (tree,instance) in baseline.trees.iter().zip(&scene.instances) {
            for p in [[-1.,1.,0.],[1.,1.,0.],[0.,3.,0.]] {
                let transformed=|i:&GpuInstance|Vec3::from_array(i.rows.map(|r|r[0]*p[0]+r[1]*p[1]+r[2]*p[2]+r[3]));
                let expected=Vec3::from_array(tree.root)+(transformed(&tree.original)-Vec3::from_array(tree.root))*1.5;
                assert!(transformed(instance).abs_diff_eq(expected,1e-5));
                assert!(transformed(instance).distance(Vec3::from_array(instance.sphere[..3].try_into().unwrap()))<=instance.sphere[3]+1e-4);
            }
        }
    }
}
