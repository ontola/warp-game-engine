//! Adapt real cooked plants to shared GPU selection without cloning geometry
//! or creating scene owners per instance. Representation IDs address this table,
//! not transient scene/pool slots. Optical admission uses existing cook evidence.
use super::*;
use crate::foliage_select::{Node, NONE};

#[derive(Clone,Copy,Debug,PartialEq)]
pub enum SelectedRepresentation {
    /// Fine source members on exactly this branch, not its descendants.
    SourceBranch { branch:u32, branch_id:u64 },
    /// Branch zero's aggregate covers the whole plant; others cover their
    /// own members. Keep the original cooked volume/material/normal data.
    Voxel { branch:u32, branch_id:u64, level:u32 },
}
#[derive(Clone,Debug)]
pub struct SharedSelectionRecipe {
    pub nodes:Vec<Node>,
    pub representations:Vec<SelectedRepresentation>,
    /// Conservative entire-plant animated bound. Used even for the fallback.
    pub sphere:[f32;4],
}

fn enclosing(spheres:&[[f32;4]])->[f32;4] {
    let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
    for s in spheres {let c=Vec3::from_slice(s);lo=lo.min(c-Vec3::splat(s[3]));hi=hi.max(c+Vec3::splat(s[3]));}
    let centre=(lo+hi)*0.5;
    let radius=spheres.iter().map(|s|centre.distance(Vec3::from_slice(s))+s[3]).fold(0.,f32::max);
    centre.extend(radius*1.00001+0.00001).to_array()
}

/// Offline cook evidence, before spatial error or residency budgets apply.
#[derive(Clone,Debug,serde::Serialize)]
pub struct SelectionEvidence {
    pub branches:usize,pub root_voxel_candidates:usize,pub admitted_root_candidates:usize,
    pub voxel_candidates:usize,pub admitted_voxel_candidates:usize,
    pub root_errors:Vec<[f32;3]>,
}
fn admitted(c:&crate::aggregate_lod::Candidate,p:crate::aggregate_lod::Policy)->bool {
    p.allow_sampled_optics && !c.source_only && c.voxel_coverage_error<=p.max_coverage_error && c.voxel_color_error<=p.max_color_error
}
impl CookedPlant {
    pub fn selection_evidence(&self,policy:crate::aggregate_lod::Policy)->SelectionEvidence {
        let root=self.branches.first().map(|b|b.aggregate.as_slice()).unwrap_or(&[]);
        SelectionEvidence {
            branches:self.branches.len(),root_voxel_candidates:root.iter().filter(|c|!c.source_only).count(),
            admitted_root_candidates:root.iter().filter(|c|admitted(c,policy)).count(),
            voxel_candidates:self.branches.iter().flat_map(|b|&b.aggregate).filter(|c|!c.source_only).count(),
            admitted_voxel_candidates:self.branches.iter().flat_map(|b|&b.aggregate).filter(|c|admitted(c,policy)).count(),
            root_errors:root.iter().filter(|c|!c.source_only).map(|c|[c.spatial_error,c.voxel_coverage_error,c.voxel_color_error]).collect(),
        }
    }
    /// Build once per species/recipe. No world placements, camera or near-tree
    /// count participate. Fine branches retain exact source membership IDs.
    /// This adapter currently admits measured VOXEL candidates and exact-source
    /// leaves. It does not export measured simplified-triangle alternatives.
    pub fn shared_selection_recipe(&self,policy:crate::aggregate_lod::Policy)->Result<SharedSelectionRecipe,String> {
        if ![policy.max_coverage_error,policy.max_color_error].iter().all(|v|v.is_finite()&&*v>=0.&&*v<=1.) {
            return Err("invalid shared foliage optical policy".into());
        }
        let root=self.branches.first().ok_or("empty cooked plant")?;
        let volume=root.volume.as_ref().ok_or("shared selection requires a cooked whole-subtree root")?;
        if root.aggregate.is_empty() || volume.levels.is_empty() || root.parent.is_some() {
            return Err("shared selection requires cook_aggregate hierarchy/evidence".into());
        }
        let mut bounds=Vec::with_capacity(self.branches.len());
        for (i,b) in self.branches.iter().enumerate() {
            let mut local=Vec::new();
            for p in self.placements.iter().filter(|p|p.branch==i) {
                let sphere=self.primitives[p.primitive].mesh.sphere;
                let m=p.rest;let c=m.transform_point3(Vec3::from_slice(&sphere));
                // Conservative for authored shear as well as rotated/nonuniform scale.
                let row=(0..3).map(|r|m.row(r).truncate().abs().element_sum()).fold(0.,f32::max);
                let col=(0..3).map(|c|m.col(c).truncate().abs().element_sum()).fold(0.,f32::max);
                let scale=(row*col).sqrt();
                local.push(c.extend(sphere[3]*scale).to_array());
            }
            if let Some(asset)=&b.volume {
                let (lo,hi)=support(asset);local.push(((lo+hi)*0.5).extend((hi-lo).length()*0.5).to_array());
            }
            if local.is_empty(){bounds.push(None);continue;}
            let s=enclosing(&local);let c=b.assembly_rest.transform_point3(Vec3::from_slice(&s));
            let mut lever=Vec3::from_slice(&s).length()+s[3];let mut wind=0.;let mut k=i;
            loop {
                let branch=&self.branches[k];wind+=2.*(branch.max_angle*0.5).sin()*lever;
                if let Some(parent)=branch.parent {lever+=branch.rest.w_axis.truncate().length();k=parent as usize;}else{break;}
            }
            bounds.push(Some(c.extend(s[3]+wind).to_array()));
        }
        let sphere=enclosing(&bounds.iter().flatten().copied().collect::<Vec<_>>());
        let mut recipe=SharedSelectionRecipe {nodes:Vec::new(),representations:Vec::new(),sphere};
        // Resident fallback exists even if no root voxel meets the optical
        // policy. flags bit 0 prohibits normal selection of that fallback.
        recipe.representations.push(SelectedRepresentation::Voxel {branch:0,branch_id:root.id,level:volume.levels.len()as u32-1});
        recipe.nodes.push(Node {sphere,error:[0.;4],links:[1,0,0,1]});
        let candidates=|branch:usize| {
            let b=&self.branches[branch];
            let mut result=b.aggregate.iter().filter(|c|admitted(c,policy))
                .copied().collect::<Vec<_>>();
            result.sort_by(|a,b|b.spatial_error.total_cmp(&a.spatial_error).then(b.level.cmp(&a.level)));
            result
        };
        let push_chain=|recipe:&mut SharedSelectionRecipe,branch:usize,bound:[f32;4]|->Result<(),String> {
            for c in candidates(branch) {
                if !c.spatial_error.is_finite() || c.spatial_error<0. || self.branches[branch].volume.as_ref().is_none_or(|a|c.level as usize>=a.levels.len()) {
                    return Err("invalid cooked aggregate candidate".into());
                }
                let rep=recipe.representations.len()as u32;
                recipe.representations.push(SelectedRepresentation::Voxel {branch:branch as u32,branch_id:self.branches[branch].id,level:c.level});
                let next=recipe.nodes.len()as u32+1;
                recipe.nodes.push(Node {sphere:bound,error:[c.spatial_error,0.,0.,0.],links:[next,0,rep,0]});
            }
            Ok(())
        };
        push_chain(&mut recipe,0,sphere)?;
        let root_chain=recipe.nodes.len();
        for (branch,b) in self.branches.iter().enumerate() {
            if !self.placements.iter().any(|p|p.branch==branch){continue;}
            let bound=bounds[branch].ok_or("source branch has no bounds")?;
            let start=recipe.nodes.len();
            // Root volume represents ALL branches, so never reuse it as the
            // voxel alternative of the root's own trunk members.
            if branch!=0 {push_chain(&mut recipe,branch,bound)?;}
            let rep=recipe.representations.len()as u32;
            recipe.representations.push(SelectedRepresentation::SourceBranch {branch:branch as u32,branch_id:b.id});
            let end=recipe.nodes.len()as u32+1;
            recipe.nodes.push(Node {sphere:bound,error:[0.;4],links:[NONE,end,rep,0]});
            for node in &mut recipe.nodes[start..] {node.links[1]=end;}
        }
        let end=recipe.nodes.len()as u32;
        if end as usize==root_chain {return Err("shared plant has no source leaves".into());}
        for node in &mut recipe.nodes[..root_chain] {node.links[1]=end;}
        Ok(recipe)
    }
}
