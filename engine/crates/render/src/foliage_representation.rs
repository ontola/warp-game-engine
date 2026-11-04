//! Bounded source/voxel selection for the SAME branch group and wind rig.
//!
//! This is a bounded per-view CPU planner, not world streaming or
//! a full Nanite hierarchy. It never changes plant membership, colour, size,
//! source IDs or branch poses. Production defaults require optical evidence;
//! small projected cells alone cannot establish opacity/lighting equivalence.

use crate::foliage_voxel::VoxelAsset;
use glam::{Mat4, Vec3, Vec4};
use std::collections::{BTreeMap,BTreeSet};

pub const MAX_GROUPS:usize=crate::voxel_forest::MAX_VIEW_GROUPS;
pub const MAX_LEVELS:usize=16;

#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub struct GroupKey {
    pub instance:u64,
    pub branch:u64,
    /// Increment when rest geometry/season/placement changes, not for wind.
    pub generation:u32,
}

#[derive(Clone,Copy,Debug,PartialEq)]
pub enum OpticalEvidence {
    Unvalidated,
    /// Caller-provided maximum absolute coverage error from its accepted
    /// source/volume oracle. This module does not manufacture such evidence.
    CoverageBound {max_absolute_error:f32},
}

#[derive(Clone,Copy,Debug)]
pub struct Candidate {
    pub level:u32,
    /// Conservative spatial reconstruction footprint in world metres. This
    /// describes positional support, not a bound on Beer-opacity error.
    pub error_radius:f32,
    pub optics:OpticalEvidence,
}

#[derive(Clone,Debug)]
pub struct Group {
    pub key:GroupKey,
    /// Conservative world-space motion envelope shared by source and volume.
    pub center:Vec3,
    pub radius:f32,
    pub candidates:Vec<Candidate>,
}

#[derive(Clone,Copy,Debug)]
pub struct View {
    /// Unjittered matrix. Near/far decisions must not oscillate with TAA jitter.
    pub view_projection:Mat4,
    pub width:u32,
    pub height:u32,
}

#[derive(Clone,Copy,Debug)]
pub struct Policy {
    pub max_error_pixels:f32,
    pub hysteresis_fraction:f32,
    pub max_coverage_error:f32,
    /// Explicit experiments only. Current procedural voxels have not passed
    /// optical equivalence and therefore keep source geometry by default.
    pub allow_unvalidated_optics:bool,
}
impl Default for Policy {
    fn default()->Self{Self {max_error_pixels:1.0,hysteresis_fraction:0.15,
        max_coverage_error:0.05,allow_unvalidated_optics:false}}
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Representation {Source,Voxel {level:u32}}

#[derive(Clone,Copy,Debug)]
pub struct Decision {
    pub key:GroupKey,
    pub representation:Representation,
    pub projected_error_pixels:f32,
    pub optics_validated:bool,
}

/// One source branch and the volume baked from that exact branch. The source
/// remains available for shadow passes regardless of its camera selection.
#[derive(Clone,Debug)]
pub struct Binding {
    pub owner:u32,
    pub volume:u32,
    pub group:Group,
}

/// Shared primitives belonging to one branch-local source representation.
/// `owner` is a non-rendering (zero-cluster) branch proxy used for the volume
/// transform and one wind pose. Members retain their own immutable meshes and
/// local transforms; they are never duplicated into an expanded branch mesh.
#[derive(Clone,Debug)]
pub struct SourceGroup {
    pub owner:u32,
    pub members:Vec<u32>,
}

fn validate_source_groups(scene:&crate::scene::Scene)->Result<(),String> {
    if scene.foliage_source_groups.len()>MAX_GROUPS {
        return Err("shared foliage groups exceed the representation budget".into());
    }
    let binding_owners:BTreeSet<_>=scene.foliage_representations.iter().map(|b|b.owner).collect();
    let mut owners=BTreeSet::new();let mut members=BTreeSet::new();
    for group in &scene.foliage_source_groups {
        if !owners.insert(group.owner)||!binding_owners.contains(&group.owner)||group.members.is_empty() {
            return Err("shared foliage group needs a unique bound proxy and nonempty members".into());
        }
        let proxy=scene.instances.get(group.owner as usize).ok_or("shared foliage proxy out of range")?;
        if proxy.cluster_count!=0||proxy.flags&crate::scene::PART_WIND==0 {
            return Err("shared foliage proxy must be a zero-cluster animated branch".into());
        }
        let poses:Vec<_>=scene.part_wind.iter().enumerate().filter(|(_,p)|p.owner==Some(group.owner)).collect();
        if poses.len()!=1 {return Err("shared foliage proxy needs exactly one branch pose".into());}
        let pose=poses[0].0 as u32;
        let expected:BTreeSet<_>=scene.part_attachments.iter().filter(|a|a.pose==pose).map(|a|a.owner).collect();
        let actual:BTreeSet<_>=group.members.iter().copied().collect();
        if actual.len()!=group.members.len()||actual!=expected {
            return Err("shared foliage members must exactly cover the branch attachments".into());
        }
        for &member in &group.members {
            let source=scene.instances.get(member as usize).ok_or("shared foliage member out of range")?;
            if !members.insert(member)||binding_owners.contains(&member)||source.cluster_count==0
                ||source.flags&crate::scene::PART_WIND==0||source.category()!=proxy.category() {
                return Err("shared foliage members must be unique nonempty primitives of the same category".into());
            }
        }
    }
    Ok(())
}

#[derive(Clone,Copy,Debug)]
pub struct PlannedGroup {
    pub key:GroupKey,
    pub owner:u32,
    pub volume:u32,
    pub representation:Representation,
    /// The renderer must not choose a coarser mip than this decision allows.
    pub projected_error_pixels:f32,
    pub optics_validated:bool,
    /// Currently exactly 0 or 1: no noisy screen-pixel crossfade. A branch
    /// switches only inside its subpixel budget. This does NOT prove equal
    /// optical coverage; that is a separate evidence gate above.
    pub volume_weight:f32,
}

/// Keep one planner per stable viewport, never one history shared by all four
/// players. Planning uses only immutable bounds and the unjittered camera; it
/// needs no GPU readback and does not evaluate branch animation on the CPU.
#[derive(Default)]
pub struct ViewPlanner {history:BTreeMap<GroupKey,(u32,u32,Decision)>}

fn threshold_fraction(key:GroupKey)->f32 {
    // Geometry generation and input ordering do not move the threshold.
    // Independent branches stagger naturally instead of one 50m LOD ring.
    let mut hash=key.instance^key.branch.rotate_left(23);
    hash=(hash^(hash>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
    hash=(hash^(hash>>27)).wrapping_mul(0x94d049bb133111eb);
    hash^=hash>>31;
    0.8+0.2*((hash>>40) as f32/16_777_215.0)
}

impl ViewPlanner {
    pub fn clear(&mut self) {self.history.clear();}

    /// Source/voxel exclusivity is decided per whole branch; the renderer must
    /// apply the same decision to its source raster and volume dispatch, only
    /// in this camera's colour pass. It must not apply it to sun/headlight
    /// shadow casters. Invalid requests leave the previous history untouched.
    pub fn plan(&mut self,bindings:&[Binding],view:View,policy:Policy)->Result<Vec<PlannedGroup>,String> {
        if bindings.len()>MAX_GROUPS {return Err("too many foliage representation bindings".into());}
        select(&[],view,policy,&[])?;
        let mut owners=BTreeSet::new();let mut volumes=BTreeSet::new();
        let mut next=BTreeMap::new();let mut output=Vec::with_capacity(bindings.len());
        for binding in bindings {
            if !owners.insert(binding.owner)||!volumes.insert(binding.volume)||next.contains_key(&binding.group.key) {
                return Err("foliage representation identities/owners/volumes must be unique".into());
            }
            let old=self.history.get(&binding.group.key)
                .filter(|(owner,volume,_)|*owner==binding.owner&&*volume==binding.volume)
                .map(|(_,_,decision)|*decision);
            // select() has symmetric hysteresis. Compensate its upper bound
            // so even a retained volume never exceeds the requested budget.
            let staggered=Policy {max_error_pixels:policy.max_error_pixels*threshold_fraction(binding.group.key)
                /(1.0+policy.hysteresis_fraction),..policy};
            let decision=select(std::slice::from_ref(&binding.group),view,staggered,
                old.as_slice())?[0];
            output.push(PlannedGroup {key:decision.key,owner:binding.owner,volume:binding.volume,
                representation:decision.representation,projected_error_pixels:decision.projected_error_pixels,
                optics_validated:decision.optics_validated,
                volume_weight:if matches!(decision.representation,Representation::Voxel {..}) {1.0}else{0.0}});
            next.insert(decision.key,(binding.owner,binding.volume,decision));
        }
        output.sort_unstable_by_key(|plan|plan.key);
        self.history=next;Ok(output)
    }
}

fn affine(rows:[[f32;4];3])->Mat4 {
    Mat4::from_cols(Vec4::new(rows[0][0],rows[1][0],rows[2][0],0.0),
        Vec4::new(rows[0][1],rows[1][1],rows[2][1],0.0),
        Vec4::new(rows[0][2],rows[1][2],rows[2][2],0.0),
        Vec4::new(rows[0][3],rows[1][3],rows[2][3],1.0))
}

/// Validate the actual scene mapping before uploading it. A mismatched source
/// owner, volume inverse, mip list or wind branch must fail, not silently hide
/// an unrelated plant. The group envelope must contain all packed mip support
/// at rest; the caller also supplies the full supported animation margin.
pub fn validate_bindings(scene:&crate::scene::Scene)->Result<(),String> {
    validate_source_groups(scene)?;
    let bindings=&scene.foliage_representations;
    // Validates all policy, identity, candidate and envelope fields too.
    ViewPlanner::default().plan(bindings,View {view_projection:Mat4::IDENTITY,width:1,height:1},
        scene.foliage_representation_policy)?;
    if bindings.is_empty() {return Ok(());}
    let forest=scene.voxel_forest.as_ref().ok_or("foliage representations require their packed volumes")?;
    if forest.instances.len()-(scene.voxel_proxies as usize).min(forest.instances.len())>MAX_GROUPS {
        return Err("foliage representation forest exceeds the per-view GPU table".into());
    }
    for binding in bindings {
        if binding.volume as usize>=MAX_GROUPS {
            return Err("foliage representation volume exceeds the per-view GPU table".into());
        }
        let source=scene.instances.get(binding.owner as usize).ok_or("foliage representation owner is out of range")?;
        let volume=forest.instances.get(binding.volume as usize).ok_or("foliage representation volume is out of range")?;
        // A virtual branch owner (vegetation_asset virtual members) draws its
        // templates itself: first_cluster packs them.
        let virtual_owner=source.cluster_count==0&&source.first_cluster!=0&&source.flags&crate::scene::PART_WIND!=0;
        let shared=virtual_owner||scene.foliage_source_groups.iter().any(|group|group.owner==binding.owner);
        // An aggregate-only articulation root replaces its descendant groups,
        // not nonexistent root triangles. Exact mode expands those children.
        // Ordinary orphan/empty branch bindings remain invalid.
        let subtree_root=scene.aggregate_lod.as_ref().is_some_and(|selection|
            selection.groups.iter().enumerate().any(|(index,g)|
                g.volume==binding.volume && g.parent.is_none()
                && selection.groups.iter().any(|child|child.parent==Some(index as u32))))
            && scene.part_wind.iter().any(|pose|pose.owner==Some(binding.owner)
                && pose.volume==Some(binding.volume) && pose.parent.is_none());
        if (source.cluster_count==0&&!shared&&!subtree_root)||volume.meta[0]!=binding.owner {
            return Err("foliage representation source/volume owner mismatch or empty source".into());
        }
        if volume.meta[3]!=crate::foliage_volume_data::ALL_MATERIALS {
            return Err("dynamic foliage representation requires an exact whole-branch source replacement".into());
        }
        let source_pose=affine(source.rows);let inverse=affine(volume.inverse);
        if !source_pose.is_finite()||!inverse.is_finite()||!source_pose.inverse().is_finite()
            ||!source_pose.inverse().abs_diff_eq(inverse,2e-4) {
            return Err("foliage representation source/volume bind poses differ".into());
        }
        let first=volume.meta[1] as usize;let count=volume.meta[2] as usize;
        let levels=first.checked_add(count).and_then(|end|forest.levels.get(first..end))
            .filter(|levels|!levels.is_empty()).ok_or("foliage representation packed mip range is invalid")?;
        if binding.group.candidates.iter().any(|c|c.level as usize>=levels.len()) {
            return Err("foliage representation candidate has no packed mip".into());
        }
        let axes=[source_pose.x_axis.truncate(),source_pose.y_axis.truncate(),source_pose.z_axis.truncate()];
        let stretch=(axes.iter().map(|a|a.abs().element_sum()).fold(0.0,f32::max)
            *(axes[0].abs()+axes[1].abs()+axes[2].abs()).max_element()).sqrt();
        for candidate in &binding.group.candidates {
            let required=1.5*3.0f32.sqrt()*levels[candidate.level as usize].origin_cell[3]*stretch;
            if candidate.error_radius+1e-6<required {
                return Err("foliage representation understates packed cell reconstruction error".into());
            }
        }
        for level in levels {
            let origin=Vec3::from_slice(&level.origin_cell[..3]);let cell=level.origin_cell[3];
            let dims=Vec3::new(level.dims_offset[0] as f32,level.dims_offset[1] as f32,level.dims_offset[2] as f32);
            if !origin.is_finite()||!cell.is_finite()||cell<=0.0||level.dims_offset[..3].contains(&0) {
                return Err("foliage representation mip support is invalid".into());
            }
            let lo=origin-Vec3::splat(cell*0.5);let hi=origin+(dims+Vec3::splat(0.5))*cell;
            for corner in 0..8 {
                let local=Vec3::new(if corner&1==0 {lo.x}else{hi.x},if corner&2==0 {lo.y}else{hi.y},if corner&4==0 {lo.z}else{hi.z});
                if source_pose.transform_point3(local).distance(binding.group.center)>binding.group.radius+0.002 {
                    return Err("foliage representation envelope does not contain packed mip support".into());
                }
            }
        }
        let parts:Vec<_>=scene.part_wind.iter().filter(|part|part.owner==Some(binding.owner)
            ||part.volume==Some(binding.volume)).collect();
        if source.flags&crate::scene::PART_WIND!=0 {
            if parts.len()!=1||parts[0].owner!=Some(binding.owner)||parts[0].volume!=Some(binding.volume) {
                return Err("foliage representation source/volume must share one wind branch".into());
            }
        } else if !parts.is_empty() {
            return Err("animated foliage representation source must carry PART_WIND".into());
        }
    }
    Ok(())
}

impl Group {
    /// Derive one motion envelope and error radii from the exact voxel source.
    /// The maximum wind displacement must cover the supported wind settings.
    /// No camera, seed or random replacement geometry enters this operation.
    pub fn from_voxels(key:GroupKey,asset:&VoxelAsset,rest:Mat4,wind_margin:f32,
        optics:OpticalEvidence)->Result<Self,String> {
        if !rest.is_finite()||rest.x_axis.w!=0.0||rest.y_axis.w!=0.0||rest.z_axis.w!=0.0||rest.w_axis.w!=1.0
            ||!rest.determinant().is_finite()||rest.determinant()<=0.0
            ||!wind_margin.is_finite()||wind_margin<0.0||asset.levels.is_empty()||asset.levels.len()>MAX_LEVELS {
            return Err("invalid foliage representation source/transform/wind envelope".into());
        }
        let matrix=[rest.x_axis.truncate(),rest.y_axis.truncate(),rest.z_axis.truncate()];
        let norm_one=matrix.iter().map(|c|c.abs().element_sum()).fold(0.0,f32::max);
        let row_sums=matrix[0].abs()+matrix[1].abs()+matrix[2].abs();
        // sqrt(||A||1 ||A||inf) is an upper bound on every vector stretch,
        // including nonuniform scale followed by branch rotation.
        let stretch=(norm_one*row_sums.max_element()).sqrt();
        let mut lo=Vec3::from(asset.source_bounds[0]);let mut hi=Vec3::from(asset.source_bounds[1]);
        if !lo.is_finite()||!hi.is_finite()||lo.cmpgt(hi).any(){return Err("invalid foliage source bounds".into());}
        let mut candidates=Vec::new();let mut previous=0.0;
        for (level,mip) in asset.levels.iter().enumerate() {
            if !mip.cell_size.is_finite()||mip.cell_size<=previous||mip.dimensions.contains(&0) {
                return Err("invalid foliage representation mip hierarchy".into());
            }
            previous=mip.cell_size;
            let bounds=mip.local_bounds(asset.origin);let support=Vec3::splat(mip.cell_size*0.5);
            lo=lo.min(Vec3::from(bounds[0])-support);hi=hi.max(Vec3::from(bounds[1])+support);
            // A quadrature sample can be half a diagonal from its cell centre;
            // the linear reconstruction tent extends a full diagonal further.
            candidates.push(Candidate {level:level as u32,error_radius:1.5*3.0f32.sqrt()*mip.cell_size*stretch,optics});
        }
        let center=rest.transform_point3((lo+hi)*0.5);
        let radius=(hi-lo).length()*0.5*stretch+wind_margin;
        if !center.is_finite()||!radius.is_finite()||candidates.iter().any(|c|!c.error_radius.is_finite()) {
            return Err("foliage representation world bounds overflow".into());
        }
        Ok(Self {key,center,radius,candidates})
    }
}

/// Bound screen displacement over a sphere, including off-axis perspective.
/// A centre-only distance test misses the projective derivative near the view
/// edge. Overlapping the camera/near plane keeps source geometry conservatively.
fn pixels_per_metre(group:&Group,view:View)->Option<f32> {
    let m=view.view_projection.transpose();
    let [x,y,z,w]=[m.x_axis,m.y_axis,m.z_axis,m.w_axis];
    let p=group.center.extend(1.0);
    let w_min=w.dot(p)-group.radius*w.truncate().length();
    let z_min=z.dot(p)-group.radius*z.truncate().length();
    // Covers ordinary and reversed-Z 0..w clip space without assuming which
    // depth plane is the near plane.
    let wz_min=(w-z).dot(p)-group.radius*(w-z).truncate().length();
    if w_min<=1e-6||z_min<=0.0||wz_min<=0.0{return None;}
    let row_bound=|row:Vec4,pixels:u32| {
        let magnitude=row.truncate().length();
        let numerator=row.dot(p).abs()+group.radius*magnitude;
        (magnitude/w_min+numerator*w.truncate().length()/(w_min*w_min))*pixels as f32*0.5
    };
    let derivative=Vec3::new(row_bound(x,view.width),row_bound(y,view.height),0.0).length();
    if derivative.is_finite(){Some(derivative)}else{None}
}

pub fn select(groups:&[Group],view:View,policy:Policy,previous:&[Decision])->Result<Vec<Decision>,String> {
    if groups.len()>MAX_GROUPS||previous.len()>MAX_GROUPS||view.width==0||view.height==0||!view.view_projection.is_finite()
        ||!policy.max_error_pixels.is_finite()||policy.max_error_pixels<=0.0
        ||!policy.hysteresis_fraction.is_finite()||!(0.0..0.5).contains(&policy.hysteresis_fraction)
        ||!policy.max_coverage_error.is_finite()||!(0.0..=1.0).contains(&policy.max_coverage_error) {
        return Err("invalid or over-budget foliage representation request".into());
    }
    let mut history=BTreeMap::new();
    for d in previous {if history.insert(d.key,d.representation).is_some(){return Err("duplicate foliage selection history identity".into());}}
    let mut keys=BTreeSet::new();let mut decisions=Vec::with_capacity(groups.len());
    for group in groups {
        if !keys.insert(group.key)||!group.center.is_finite()||!group.radius.is_finite()||group.radius<0.0
            ||group.candidates.len()>MAX_LEVELS {return Err("invalid or duplicate foliage representation group".into());}
        let mut last_level=None;let mut last_error=0.0;
        for c in &group.candidates {
            if c.level>=MAX_LEVELS as u32||!c.error_radius.is_finite()||c.error_radius<=last_error
                ||last_level.is_some_and(|l|c.level<=l) {return Err("unordered/invalid foliage representation candidates".into());}
            if let OpticalEvidence::CoverageBound {max_absolute_error:e}=c.optics {
                if !e.is_finite()||!(0.0..=1.0).contains(&e){return Err("invalid optical evidence bound".into());}
            }
            last_level=Some(c.level);last_error=c.error_radius;
        }
        let mut decision=Decision {key:group.key,representation:Representation::Source,projected_error_pixels:0.0,optics_validated:true};
        if let Some(scale)=pixels_per_metre(group,view) {
            let old=history.get(&group.key);
            for c in group.candidates.iter().rev() {
                let validated=matches!(c.optics,OpticalEvidence::CoverageBound {max_absolute_error:e} if e<=policy.max_coverage_error);
                if !validated&&!(policy.allow_unvalidated_optics&&c.optics==OpticalEvidence::Unvalidated){continue;}
                let representation=Representation::Voxel {level:c.level};
                let threshold=policy.max_error_pixels*(1.0+if old==Some(&representation){policy.hysteresis_fraction}else{-policy.hysteresis_fraction});
                let error=c.error_radius*scale;
                if error<=threshold {
                    decision=Decision {key:group.key,representation,projected_error_pixels:error,optics_validated:validated};break;
                }
            }
        }
        decisions.push(decision);
    }
    decisions.sort_unstable_by_key(|d|d.key);Ok(decisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(instance:u64)->GroupKey {GroupKey {instance,branch:71,generation:0}}
    fn group(instance:u64,z:f32)->Group {Group {key:key(instance),center:Vec3::new(0.0,0.0,z),radius:0.25,
        candidates:vec![Candidate {level:0,error_radius:0.02,optics:OpticalEvidence::Unvalidated},
            Candidate {level:1,error_radius:0.04,optics:OpticalEvidence::Unvalidated}]}}
    fn view(width:u32,height:u32)->View{View {view_projection:Mat4::perspective_rh(60f32.to_radians(),width as f32/height as f32,0.1,1000.0),width,height}}
    fn experimental()->Policy{Policy {allow_unvalidated_optics:true,..Default::default()}}
    #[test]
    fn optical_evidence_is_required_by_default_and_bad_evidence_cannot_be_overridden() {
        let mut g=group(1,-100.0);let v=view(1920,1080);
        assert_eq!(select(&[g.clone()],v,Policy::default(),&[]).unwrap()[0].representation,Representation::Source);
        assert!(matches!(select(&[g.clone()],v,experimental(),&[]).unwrap()[0].representation,Representation::Voxel{..}));
        for c in &mut g.candidates {c.optics=OpticalEvidence::CoverageBound {max_absolute_error:0.7};}
        assert_eq!(select(&[g.clone()],v,experimental(),&[]).unwrap()[0].representation,Representation::Source);
        for c in &mut g.candidates {c.optics=OpticalEvidence::CoverageBound {max_absolute_error:0.02};}
        assert!(select(&[g],v,Policy::default(),&[]).unwrap()[0].optics_validated);
    }
    #[test]
    fn closer_views_and_larger_viewports_keep_source_independently() {
        let v=view(1920,1080);
        assert_eq!(select(&[group(1,-4.0)],v,experimental(),&[]).unwrap()[0].representation,Representation::Source);
        assert!(matches!(select(&[group(1,-100.0)],v,experimental(),&[]).unwrap()[0].representation,Representation::Voxel{..}));
        let small=select(&[group(1,-45.0)],view(960,540),experimental(),&[]).unwrap();
        let large=select(&[group(1,-45.0)],view(3840,2160),experimental(),&[]).unwrap();
        assert!(matches!(small[0].representation,Representation::Voxel{..}));
        assert_eq!(large[0].representation,Representation::Source);
    }
    #[test]
    fn hysteresis_uses_stable_identity_and_resets_when_geometry_generation_changes() {
        let v=view(1920,1080);let p=experimental();let mut g=group(1,-40.0);g.candidates.truncate(1);
        // Choose a world error whose screen-space estimate is inside the band.
        g.candidates[0].error_radius=1.0/pixels_per_metre(&g,v).unwrap();
        assert_eq!(select(&[g.clone()],v,p,&[]).unwrap()[0].representation,Representation::Source);
        let previous=[Decision {key:g.key,representation:Representation::Voxel {level:0},projected_error_pixels:0.8,optics_validated:false}];
        assert_eq!(select(&[g.clone()],v,p,&previous).unwrap()[0].representation,Representation::Voxel {level:0});
        g.key.generation+=1;
        assert_eq!(select(&[g],v,p,&previous).unwrap()[0].representation,Representation::Source);
    }
    #[test]
    fn input_order_and_wind_phase_do_not_reseed_groups_and_near_plane_is_safe() {
        let mut groups=vec![group(3,-50.0),group(1,-80.0),group(2,-0.2)];let v=view(960,540);
        let a=select(&groups,v,experimental(),&[]).unwrap();groups.reverse();
        let b=select(&groups,v,experimental(),&[]).unwrap();
        assert_eq!(a.iter().map(|d|(d.key,d.representation)).collect::<Vec<_>>(),b.iter().map(|d|(d.key,d.representation)).collect::<Vec<_>>());
        assert_eq!(a.iter().find(|d|d.key.instance==2).unwrap().representation,Representation::Source);
        assert!(select(&[group(1,-40.0),group(1,-40.0)],v,experimental(),&[]).is_err());
        assert!(select(&vec![group(1,-40.0);MAX_GROUPS+1],v,experimental(),&[]).is_err());
    }
    #[test]
    fn off_axis_derivative_and_motion_envelope_are_conservative() {
        let v=view(1920,1080);let mut g=group(1,-50.0);
        let centre=pixels_per_metre(&g,v).unwrap();g.center.x=45.0;
        assert!(pixels_per_metre(&g,v).unwrap()>centre);
        let small=pixels_per_metre(&g,v).unwrap();g.radius=10.0;
        assert!(pixels_per_metre(&g,v).unwrap()>small);
    }

    fn binding(instance:u64,z:f32)->Binding {
        Binding {owner:instance as u32,volume:instance as u32,group:group(instance,z)}
    }
    #[test]
    fn runtime_planners_are_per_view_staggered_and_never_exceed_pixel_budget() {
        let bindings:Vec<_>=(0..4).map(|id|binding(id,-45.0)).collect();
        let mut wide=ViewPlanner::default();let mut split=ViewPlanner::default();
        let close=wide.plan(&bindings,view(3840,2160),experimental()).unwrap();
        let far=split.plan(&bindings,view(960,540),experimental()).unwrap();
        assert!(close.iter().all(|p|p.representation==Representation::Source));
        assert!(far.iter().all(|p|matches!(p.representation,Representation::Voxel {..})));
        // Reusing the distant player's history must never affect player one.
        assert!(wide.plan(&bindings,view(3840,2160),experimental()).unwrap().iter()
            .all(|p|p.representation==Representation::Source));
        let mut selected_at=BTreeMap::new();let mut planner=ViewPlanner::default();
        for step in 1..1800 {
            let distance=5.0+step as f32*0.1;
            let bindings:Vec<_>=(0..4).map(|id|binding(id,-distance)).collect();
            for p in planner.plan(&bindings,view(1920,1080),experimental()).unwrap() {
                if p.volume_weight==1.0 {selected_at.entry(p.key).or_insert(step);}
                assert!(p.volume_weight==0.0||p.volume_weight==1.0);
                assert!(p.projected_error_pixels<=1.0+1e-6);
            }
        }
        assert_eq!(selected_at.len(),4);
        assert!(selected_at.values().collect::<BTreeSet<_>>().len()>1,"branches must not share one radial transition ring");
        // Approach again: the upper hysteresis threshold also respects 1px.
        for step in (1..1800).rev() {
            for p in planner.plan(&[binding(0,-(5.0+step as f32*0.1))],view(1920,1080),experimental()).unwrap() {
                assert!(p.projected_error_pixels<=1.0+1e-6);
            }
        }
    }
    #[test]
    fn runtime_history_resets_on_mapping_generation_and_rejects_duplicates_atomically() {
        let mut planner=ViewPlanner::default();let v=view(1920,1080);let p=experimental();
        let mut b=binding(9,-80.0);b.group.candidates.truncate(1);
        planner.plan(&[b.clone()],v,p).unwrap();
        // Put the finest candidate inside this group's hysteresis band.
        b.group.candidates[0].error_radius=p.max_error_pixels*threshold_fraction(b.group.key)
            /(1.0+p.hysteresis_fraction)/pixels_per_metre(&b.group,v).unwrap();
        assert_eq!(planner.plan(&[b.clone()],v,p).unwrap()[0].volume_weight,1.0);
        assert!(planner.plan(&[b.clone(),b.clone()],v,p).is_err());
        assert_eq!(planner.plan(&[b.clone()],v,p).unwrap()[0].volume_weight,1.0);
        b.owner+=1;
        assert_eq!(planner.plan(&[b.clone()],v,p).unwrap()[0].volume_weight,0.0);
        b.group.candidates[0].error_radius*=0.5;
        assert_eq!(planner.plan(&[b.clone()],v,p).unwrap()[0].volume_weight,1.0);
        b.group.candidates[0].error_radius*=2.0;b.group.key.generation+=1;
        assert_eq!(planner.plan(&[b],v,p).unwrap()[0].volume_weight,0.0);
    }
    fn mapped_scene()->crate::scene::Scene {
        use crate::{foliage_volume_data::{GpuForest,GpuLevel,GpuTree,ALL_MATERIALS},scene::{GpuInstance,PART_WIND}};
        let rest=Mat4::from_translation(Vec3::new(0.0,0.0,-50.0));
        let rows=|m:Mat4|[m.row(0).to_array(),m.row(1).to_array(),m.row(2).to_array()];
        let mut scene=crate::scene::Scene::default();
        scene.instances.push(GpuInstance {rows:rows(rest),sphere:[0.0,0.0,-50.0,0.1],
            first_cluster:0,cluster_count:1,scale:1.0,flags:PART_WIND});
        scene.voxel_forest=Some(GpuForest {instances:vec![GpuTree {inverse:rows(rest.inverse()),
            bounds_min:[-0.1,-0.1,-50.1,0.0],bounds_max:[0.1,0.1,-49.9,0.0],meta:[0,0,2,ALL_MATERIALS]}],
            levels:vec![GpuLevel {origin_cell:[-0.02,-0.02,-0.02,0.002],dims_offset:[20,20,20,0]},
                GpuLevel {origin_cell:[-0.02,-0.02,-0.02,0.004],dims_offset:[10,10,10,0]}],..Default::default()});
        scene.part_wind.push(crate::part_wind::PartWind {owner:Some(0),volume:Some(0),rest,..Default::default()});
        scene.foliage_representations.push(binding(0,-50.0));scene
    }
    #[test]
    fn runtime_binding_validation_checks_actual_pose_material_mips_and_shared_wind_owner() {
        let mut scene=mapped_scene();assert!(validate_bindings(&scene).is_ok());
        scene.voxel_forest.as_mut().unwrap().instances[0].meta[0]=1;
        assert!(validate_bindings(&scene).is_err());
        scene.voxel_forest.as_mut().unwrap().instances[0].meta[0]=0;
        scene.voxel_forest.as_mut().unwrap().instances[0].meta[3]=0;
        assert!(validate_bindings(&scene).is_err());
        scene.voxel_forest.as_mut().unwrap().instances[0].meta[3]=crate::foliage_volume_data::ALL_MATERIALS;
        scene.voxel_forest.as_mut().unwrap().instances[0].inverse[0][3]+=0.2;
        assert!(validate_bindings(&scene).is_err());
        scene=mapped_scene();scene.part_wind[0].volume=None;
        assert!(validate_bindings(&scene).is_err());
        scene=mapped_scene();scene.foliage_representations[0].group.radius=0.001;
        assert!(validate_bindings(&scene).is_err());
        scene=mapped_scene();scene.foliage_representations[0].group.candidates[0].error_radius=0.0001;
        assert!(validate_bindings(&scene).is_err());
        scene=mapped_scene();scene.foliage_representations[0].group.candidates[1].level=3;
        assert!(validate_bindings(&scene).is_err());
    }

    fn shared_scene()->crate::scene::Scene {
        let mut scene=mapped_scene();
        let primitive=scene.instances[0];
        scene.instances[0].cluster_count=0;
        scene.instances.push(primitive);
        scene.part_attachments.push(crate::part_wind::PartAttachment {pose:0,owner:1,rest:Mat4::IDENTITY});
        scene.foliage_source_groups.push(SourceGroup {owner:0,members:vec![1]});
        scene
    }

    #[test]
    fn shared_branch_replaces_all_members_with_one_volume_decision() {
        let scene=shared_scene();validate_bindings(&scene).unwrap();
        let replacements=crate::voxel_forest::replacement_materials(&scene);
        assert_eq!(replacements,vec![0x8000_0000,0x8000_0000]);
        // The proxy contributes no clusters; the real primitive remains a
        // caster and can independently render whenever this camera picks source.
        assert_eq!(scene.instances[0].cluster_count,0);
        assert_eq!(scene.instances[1].cluster_count,1);
        assert_eq!(scene.instances[1].flags&crate::scene::NO_SHADOW,0);
    }

    #[test]
    fn shared_branch_rejects_missing_duplicate_cross_branch_and_wrong_category_members() {
        let mut scene=shared_scene();scene.foliage_source_groups[0].members.clear();
        assert!(validate_bindings(&scene).is_err());
        let mut scene=shared_scene();scene.foliage_source_groups[0].members.push(1);
        assert!(validate_bindings(&scene).is_err());
        let mut scene=shared_scene();scene.part_attachments[0].pose=3;
        assert!(validate_bindings(&scene).is_err());
        let mut scene=shared_scene();scene.instances[1].set_category(crate::scene::GeometryCategory::Grass);
        assert!(validate_bindings(&scene).is_err());
        let mut scene=shared_scene();scene.foliage_representations.clear();
        assert!(validate_bindings(&scene).is_err());
        let mut scene=shared_scene();scene.instances[0].cluster_count=1;
        assert!(validate_bindings(&scene).is_err());
        let mut scene=shared_scene();scene.foliage_source_groups[0].members=vec![0];
        scene.part_attachments[0].owner=0;
        assert!(validate_bindings(&scene).is_err());
    }
}
