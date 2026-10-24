//! Bounded source/voxel selection for the SAME branch group and wind rig.
//!
//! This is a per-view CPU reference/diagnostic planner, not world streaming or
//! a full Nanite hierarchy. It never changes plant membership, colour, size,
//! source IDs or branch poses. Production defaults require optical evidence;
//! small projected cells alone cannot establish opacity/lighting equivalence.

use crate::foliage_voxel::VoxelAsset;
use glam::{Mat4, Vec3, Vec4};
use std::collections::{BTreeMap,BTreeSet};

pub const MAX_GROUPS:usize=256;
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
}
