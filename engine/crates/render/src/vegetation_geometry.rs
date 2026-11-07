//! Species-local geometry hierarchy, independent of the articulated pose tree.
//! Leaves refer to exact branch-own member sets; an internal node is their
//! disjoint union. No world placements or per-frame CPU selection live here.
use super::*;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct GeometryGroup {
    /// Children in this array; absent for an exact source branch leaf.
    pub children: Option<[u32; 2]>,
    pub source_branch: Option<u32>,
    /// Lowest shared articulated ancestor. Aggregate coordinates use its frame.
    pub frame_branch: u32,
    /// Source placement indices, sorted and unique. Stored once per group at
    /// cook time; the runtime need only upload its representation hierarchy.
    pub members: Vec<u32>,
    /// Internal-group representation; leaves reuse branch-owned assets and
    /// group zero reuses the existing whole-root asset.
    pub volume: Option<VoxelAsset>,
    pub aggregate: Vec<crate::aggregate_lod::Candidate>,
    pub motion_error: f32,
}

pub(super) fn build(plant: &CookedPlant) -> Result<Vec<GeometryGroup>, String> {
    let mut leaves = Vec::new();
    for (branch, b) in plant.branches.iter().enumerate() {
        let members: Vec<u32> = plant.placements.iter().enumerate()
            .filter(|(_, p)| p.branch == branch).map(|(i, _)| i as u32).collect();
        if members.is_empty() { continue; }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &i in &members {
            let p = &plant.placements[i as usize];
            let s = plant.primitives[p.primitive].mesh.sphere;
            let m = b.assembly_rest * p.rest;
            let c = m.transform_point3(Vec3::from_slice(&s));
            // Only used to choose a deterministic split, never for culling.
            let r = Vec3::splat(s[3] * max_axis(m));
            lo = lo.min(c-r); hi = hi.max(c+r);
        }
        let center = (lo+hi)*0.5;
        if !center.is_finite() { return Err("non-finite vegetation group center".into()); }
        leaves.push((branch as u32, center, members));
    }
    if leaves.is_empty() { return Err("vegetation geometry has no source members".into()); }
    fn ancestor(plant: &CookedPlant, mut a: u32, b: u32) -> Result<u32, String> {
        loop {
            let mut at = Some(b);
            while let Some(i) = at {
                if i == a { return Ok(a); }
                at = plant.branches[i as usize].parent;
            }
            a = plant.branches[a as usize].parent.ok_or("geometry group spans separate articulation roots")?;
        }
    }
    fn partition(plant: &CookedPlant, leaves: &mut [(u32, Vec3, Vec<u32>)], out: &mut Vec<GeometryGroup>) -> Result<u32, String> {
        let index = out.len() as u32;
        let mut members: Vec<u32> = leaves.iter().flat_map(|l|l.2.iter().copied()).collect();
        members.sort_unstable();
        let frame_branch = leaves.iter().skip(1).try_fold(leaves[0].0, |a,l|ancestor(plant,a,l.0))?;
        out.push(GeometryGroup {children:None,source_branch:None,frame_branch,members,
            volume:None,aggregate:Vec::new(),motion_error:0.0});
        if leaves.len()==1 {out[index as usize].source_branch=Some(leaves[0].0);return Ok(index);}
        let lo=leaves.iter().fold(Vec3::splat(f32::INFINITY),|a,l|a.min(l.1));
        let hi=leaves.iter().fold(Vec3::splat(f32::NEG_INFINITY),|a,l|a.max(l.1));
        let extent=hi-lo;
        let axis=if extent.x>=extent.y && extent.x>=extent.z {0}else if extent.y>=extent.z {1}else{2};
        leaves.sort_by(|a,b|a.1[axis].total_cmp(&b.1[axis]).then(a.0.cmp(&b.0)));
        let middle=leaves.len()/2;let (left,right)=leaves.split_at_mut(middle);
        let children=[partition(plant,left,out)?,partition(plant,right,out)?];
        out[index as usize].children=Some(children);Ok(index)
    }
    // Binary balanced hierarchy: at most 2 * articulated branches - 1 nodes,
    // independent of how many instances of the species exist in the world.
    let mut groups=Vec::with_capacity(leaves.len()*2-1);
    partition(plant,&mut leaves,&mut groups)?;
    Ok(groups)
}

pub(super) fn cook(plant: &mut CookedPlant, source: &Assembly, config: crate::vegetation_aggregate::Config) -> Result<(), String> {
    let cooker=crate::vegetation_aggregate::Cooker::new(source,config)?;
    // Groups cook independently: in parallel, applied in order.
    let assets=crate::par::map(plant.geometry.len(),|index| {
        let group=&plant.geometry[index];
        if index==0 || group.children.is_none() {return Ok(None);}
        cooker.cook_static_subtree(&group.members,group.frame_branch,plant.branches[group.frame_branch as usize].assembly_rest)
            .map(Some).map_err(|e|format!("geometry group {index}: {e}"))
    });
    for (index,asset) in assets.into_iter().enumerate() {
        let Some(asset)=asset? else {continue};
        let frame=plant.geometry[index].frame_branch as usize;
        let group=&plant.geometry[index];
        let mut motion=0.0f32;
        for &member in &group.members {
            let p=&plant.placements[member as usize];
            let sphere=plant.primitives[p.primitive].mesh.sphere;
            let c=p.rest.transform_point3(Vec3::from_slice(&sphere));
            let mut lever=c.length()+sphere[3]*max_axis(p.rest);
            let mut branch=p.branch;let mut displacement=0.0;
            while branch!=frame {
                let b=&plant.branches[branch];
                displacement+=2.0*(b.max_angle*0.5).sin()*lever;
                lever+=b.rest.w_axis.truncate().length();
                branch=b.parent.ok_or("group member has no shared animation frame")? as usize;
            }
            motion=motion.max(displacement);
        }
        let group=&mut plant.geometry[index];
        group.aggregate=aggregate_candidates(&asset,motion);
        group.motion_error=motion;
        group.volume=Some(asset.volume);
    }
    Ok(())
}
