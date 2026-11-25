//! Shared rigid-part animation for source geometry and derived volumes.
//!
//! Buffers scale to the submitted population within explicit device/byte limits.
//! Optional GPU scheduling considers every supplied camera and shadow consumer.
//! Each active assembly is evaluated parent-first by one GPU invocation. Source
//! instance transforms, volume inverse rays and bounds use exactly that pose.
//! Source motion uses the rendered primitive matrices. Fractional volumes
//! integrate previous/current branch motion using their extinction weights;
//! a single centroid remains an approximation for overlapping motions.

use crate::{Gpu, foliage_volume_data::{GpuForest, GpuTree}, scene::{GpuInstance, PART_WIND}, wind::WindState};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use std::{collections::HashSet, sync::Mutex};
use wgpu::util::DeviceExt;

pub const MAX_PARTS: usize = 256;
pub const MAX_ASSEMBLY_PARTS: usize = 64;
/// Compatibility defaults for older scene builders, not runtime buffer sizes.
pub const MAX_ATTACHMENTS: usize = 65_536;
/// Toggle on waking a sleeping assembly. Motion's instance identity check
/// rejects this one-frame epoch change; bits 8..15 remain camera visibility.
pub const HISTORY_EPOCH: u32 = 1 << 7;
pub const MAX_ANIMATION_VIEWS: usize = 32;
const ATTACHMENT_PAGE: usize = 256;
use crate::wind::{gust_at, PERIOD};
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub struct RuntimeLimits {
    pub max_poses: usize,
    pub max_attachments: usize,
    pub max_gpu_bytes: u64,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self { max_poses: 262_144, max_attachments: 1_048_576, max_gpu_bytes: 512 * 1024 * 1024 }
    }
}
impl RuntimeLimits {
    pub fn validate_counts(self, poses: usize, attachments: usize) -> Result<(), String> {
        if poses == 0 || poses > self.max_poses || poses > u32::MAX as usize
            || attachments > self.max_attachments || attachments > u32::MAX as usize
            || self.max_gpu_bytes == 0 {
            return Err("part wind exceeds explicit pose/attachment/byte limits".into());
        }
        Ok(())
    }
}

/// One actual render consumer. Include every player view and every shadow map
/// which can observe these plants; omitting a shadow view is not conservative.
/// Empty view lists retain full animation. Matrices use WebGPU's 0..w depth.
#[derive(Clone, Copy, Debug)]
pub struct AnimationView { pub view_projection: Mat4, pub viewport: [u32; 2] }

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
struct GpuAnimationView { rows: [[f32; 4]; 4], viewport: [f32; 4] }
impl AnimationView {
    fn packed(self) -> Result<GpuAnimationView, String> {
        if !self.view_projection.is_finite() || !self.view_projection.inverse().is_finite()
            || self.viewport.contains(&0) {
            return Err("part wind animation view requires a finite invertible projection and nonzero viewport".into());
        }
        Ok(GpuAnimationView { rows: std::array::from_fn(|i| self.view_projection.row(i).to_array()),
            viewport: [self.viewport[0] as f32 * 0.5, self.viewport[1] as f32 * 0.5, 0.0, 0.0] })
    }
}

/// Stable branch ordering is supplied by the source assembly. Indices address
/// this descriptor array, not a per-frame compacted visibility list. A root
/// begins a contiguous assembly; children reference an earlier node in it.
#[derive(Clone, Debug)]
pub struct PartWind {
    pub parent: Option<u32>,
    /// Empty rig roots may have no source geometry or corresponding volume.
    pub owner: Option<u32>,
    pub volume: Option<u32>,
    /// Root: assembly-to-world. Child: rigid parent-local bind transform.
    pub rest: Mat4,
    /// Maximum authored angular deflection in radians at strength one.
    pub max_angle: f32,
    /// Integer temporal harmonic over 120 seconds, preserving wrap continuity.
    pub harmonic: u32,
    /// Authored stable phase, normally derived from the source branch ID.
    pub phase: f32,
}
impl Default for PartWind {
    fn default() -> Self {
        Self {parent:None,owner:None,volume:None,rest:Mat4::IDENTITY,max_angle:0.0,harmonic:9,phase:0.0}
    }
}

/// Growth-assembly angular response, shared by resident and streamed trees.
pub fn growth_angle(parent: Option<u32>, stiffness: f32) -> f32 {
    if parent.is_none() { 0. } else { 0.6 * (1. - stiffness).clamp(0., 1.) }
}

/// Stable, periodic oscillator of a particular growth branch in a particular plant.
pub fn growth_oscillator(plant: u64, branch: u64) -> (u32, f32) {
    let mut x = plant ^ branch;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    let phase = ((x ^ (x >> 31)) >> 40) as f32 / 16_777_216.0 * std::f32::consts::TAU;
    (7 + (branch % 13) as u32, phase)
}

/// One reused primitive attached to a shared branch pose. Its local transform
/// may contain a proper rotation and positive nonuniform scale. Wind never
/// changes this rest transform or adds another sine evaluation per primitive.
#[derive(Clone, Debug)]
pub struct PartAttachment {
    pub pose: u32,
    pub owner: u32,
    pub rest: Mat4,
}

/// Public GPU layout: six vec4 rows, 96 bytes per source branch. The first
/// encoded frame has previous=current. Snapshot once before ALL view passes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuPartPose {
    pub current: [[f32;4];3],
    pub previous: [[f32;4];3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Descriptor {
    rest: [[f32;4];3],
    source: GpuInstance,
    volume: GpuTree,
    local_min: [f32;4],
    local_max: [f32;4],
    local_sphere: [f32;4],
    animation: [f32;4],
    identity: [u32;4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AttachmentDescriptor {
    rest: [[f32;4];3],
    source: GpuInstance,
    /// Primitive-local centre, conservative WORLD-space radius. Singular-value
    /// bounds of the root and attachment keep the radius valid at every pose.
    local_sphere: [f32;4],
    identity: [u32;4],
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniform { field:[f32;4], counts:[u32;4], schedule:[u32;4], lod:[f32;4] }
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct AssemblyDescriptor { range:[u32;4], sphere:[f32;4], motion:[f32;4] }
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct AssemblyState { state:[u32;4], animation:[f32;4] }
struct Packed {parts:Vec<Descriptor>,attachments:Vec<AttachmentDescriptor>,assemblies:Vec<AssemblyDescriptor>,
    pages:Vec<[u32;4]>,owners:Vec<usize>}
struct State {field:[f32;4],initialized:bool,scheduled:bool,views:Vec<GpuAnimationView>}

fn rows(matrix:Mat4)->[[f32;4];3] {
    [matrix.row(0).to_array(),matrix.row(1).to_array(),matrix.row(2).to_array()]
}
fn matrix(rows:[[f32;4];3])->Mat4 {
    Mat4::from_cols(Vec4::new(rows[0][0],rows[1][0],rows[2][0],0.0),
        Vec4::new(rows[0][1],rows[1][1],rows[2][1],0.0),
        Vec4::new(rows[0][2],rows[1][2],rows[2][2],0.0),
        Vec4::new(rows[0][3],rows[1][3],rows[2][3],1.0))
}
fn matrix_close(a:Mat4,b:Mat4)->bool {
    a.to_cols_array().into_iter().zip(b.to_cols_array()).all(|(a,b)|(a-b).abs()<=2e-4*(1.0+a.abs().max(b.abs())))
}

fn bind_poses(parts:&[PartWind])->Result<(Vec<Mat4>,Vec<[u32;4]>),String> {
    if parts.is_empty() || parts.len()>u32::MAX as usize {return Err("part wind requires a nonempty u32-addressable rig".into());}
    let mut result:Vec<Mat4>=Vec::with_capacity(parts.len());let mut assemblies:Vec<[u32;4]>=Vec::new();
    let mut root=0usize;
    for (index,part) in parts.iter().enumerate() {
        if !part.rest.is_finite() || !part.rest.row(3).abs_diff_eq(Vec4::W,1e-6)
            || !part.max_angle.is_finite() || !(0.0..=std::f32::consts::FRAC_PI_4).contains(&part.max_angle)
            || !(1..=120).contains(&part.harmonic) || !part.phase.is_finite() {
            return Err(format!("part {index} has invalid affine bind pose or bounded animation parameters"));
        }
        let axes=[part.rest.x_axis.truncate(),part.rest.y_axis.truncate(),part.rest.z_axis.truncate()];
        let lengths=axes.map(Vec3::length);
        if lengths.iter().any(|n|!n.is_finite() || *n<1e-6 || *n>1e6) {return Err(format!("part {index} has invalid scale"));}
        let unit=[axes[0]/lengths[0],axes[1]/lengths[1],axes[2]/lengths[2]];
        if unit[0].dot(unit[1]).abs()>1e-4 || unit[1].dot(unit[2]).abs()>1e-4 || unit[2].dot(unit[0]).abs()>1e-4
            || unit[0].dot(unit[1].cross(unit[2]))<0.9999 {return Err(format!("part {index} bind axes must be a proper rotation without shear/reflection"));}
        let world=if let Some(parent)=part.parent {
            let parent=parent as usize;
            if parent<root || parent>=index {return Err(format!("part {index} parent must precede it in the same contiguous assembly"));}
            // This keeps inverse-ray stretch and the source sphere radius
            // invariant under animation. All size variation is on the root.
            if lengths.iter().any(|n|(*n-1.0).abs()>1e-4) {return Err(format!("part {index} child bind transform must be rigid; put scale on its assembly root"));}
            assemblies.last_mut().unwrap()[1]+=1;
            result[parent]*part.rest
        } else {root=index;assemblies.push([index as u32,1,0,0]);part.rest};
        if assemblies.last().unwrap()[1] as usize>MAX_ASSEMBLY_PARTS {return Err(format!("part assembly exceeds {MAX_ASSEMBLY_PARTS} branches"));}
        if !world.is_finite() || !world.inverse().is_finite() {return Err(format!("part {index} world bind pose is singular/non-finite"));}
        result.push(world);
    }
    Ok((result,assemblies))
}

/// Gershgorin upper bound on the largest eigenvalue of A^T A. Tight for
/// orthogonal scaled axes, and conservative despite tolerated roundoff in an
/// authored TRS. Max column length alone is not safe for rotated scale chains.
fn linear_scale_bound(rest:Mat4)->f32 {
    let axes=[rest.x_axis.truncate(),rest.y_axis.truncate(),rest.z_axis.truncate()];
    axes.iter().map(|a|axes.iter().map(|b|a.dot(*b).abs()).sum::<f32>())
        .fold(0.0,f32::max).sqrt()*(1.0+8.0*f32::EPSILON)
}

fn attachment_scale(rest:Mat4)->Result<f32,String> {
    if !rest.is_finite() || !rest.row(3).abs_diff_eq(Vec4::W,1e-6) {
        return Err("primitive attachment needs a finite affine local transform".into());
    }
    let axes=[rest.x_axis.truncate(),rest.y_axis.truncate(),rest.z_axis.truncate()];
    let lengths=axes.map(Vec3::length);
    if lengths.iter().any(|n|!n.is_finite() || *n<1e-6 || *n>1e6) {
        return Err("primitive attachment scale is outside its positive finite bounds".into());
    }
    let unit=[axes[0]/lengths[0],axes[1]/lengths[1],axes[2]/lengths[2]];
    if unit[0].dot(unit[1]).abs()>1e-4 || unit[1].dot(unit[2]).abs()>1e-4 || unit[2].dot(unit[0]).abs()>1e-4
        || unit[0].dot(unit[1].cross(unit[2]))<0.9999 {
        return Err("primitive attachment axes must be a proper rotation with positive scale, without shear/reflection".into());
    }
    Ok(linear_scale_bound(rest))
}

fn pack(parts:&[PartWind],attachments:&[PartAttachment],instances:&[GpuInstance],forest:Option<&GpuForest>)->Result<Packed,String> {
    pack_limited(parts,attachments,instances,forest,RuntimeLimits::default())
}

fn validate_allocation(sizes:&[u64],device:&wgpu::Limits,max_bytes:u64)->Result<u64,String> {
    let cap=device.max_buffer_size.min(u64::from(device.max_storage_buffer_binding_size));
    let mut total=std::mem::size_of::<Uniform>() as u64;
    for &size in sizes {
        if size>cap {return Err(format!("part wind buffer {size} exceeds device storage/buffer limit {cap}"));}
        total=total.checked_add(size).ok_or("part wind buffer byte count overflow")?;
    }
    if total>max_bytes || device.max_storage_buffers_per_shader_stage<7 {
        return Err("part wind exceeds explicit GPU byte budget or device storage binding count".into());
    }
    Ok(total)
}

fn pack_limited(parts:&[PartWind],attachments:&[PartAttachment],instances:&[GpuInstance],forest:Option<&GpuForest>,limits:RuntimeLimits)->Result<Packed,String> {
    limits.validate_counts(parts.len(),attachments.len())?;
    let (bind,assemblies)=bind_poses(parts)?;let mut descriptors=Vec::with_capacity(parts.len());
    let mut assembly_for_pose=vec![0u32;parts.len()];
    for (index,range) in assemblies.iter().enumerate() {
        assembly_for_pose[range[0] as usize..(range[0]+range[1]) as usize].fill(index as u32);
    }
    let mut owners=HashSet::new();let mut volumes=HashSet::new();
    let mut root_scale=1.0f32;let mut attachment_root_scale=1.0f32;let mut root_scales=Vec::with_capacity(parts.len());
    for (index,part) in parts.iter().enumerate() {
        if part.parent.is_none() {
            root_scale=[part.rest.x_axis,part.rest.y_axis,part.rest.z_axis].iter()
                .map(|v|v.truncate().length()).fold(0.0,f32::max);
            // Attachment chains add a second nonuniform transform. Bound root
            // stretch explicitly, with roundoff allowance for the bounded rig.
            attachment_root_scale=linear_scale_bound(part.rest)*(1.0+MAX_ASSEMBLY_PARTS as f32*8.0*f32::EPSILON);
        }
        root_scales.push(attachment_root_scale);
        let mut d=Descriptor::zeroed();d.rest=rows(part.rest);
        d.animation=[part.max_angle,part.harmonic as f32,part.phase.rem_euclid(std::f32::consts::TAU),0.0];
        d.identity=[part.parent.unwrap_or(NONE),part.owner.unwrap_or(NONE),part.volume.unwrap_or(NONE),assembly_for_pose[index]];
        if let Some(owner)=part.owner {
            let source=*instances.get(owner as usize).ok_or_else(||format!("part {index} source owner is out of range"))?;
            if !owners.insert(owner as usize) {return Err("each animated source owner must belong to exactly one part".into());}
            if source.flags&PART_WIND==0 {return Err(format!("part {index} source must carry PART_WIND to prevent duplicate whole-plant shear"));}
            if !matrix_close(matrix(source.rows),bind[index]) {return Err(format!("part {index} source transform differs from its assembly bind pose"));}
            let sphere=Vec4::from_array(source.sphere);
            if !sphere.is_finite() || sphere.w<0.0 || !source.scale.is_finite() || source.scale<=0.0 {return Err(format!("part {index} source bounds/scale are invalid"));}
            d.source=source;
            // Scene instances use their largest column length. A rotated
            // child under a nonuniform root can have nonorthogonal columns,
            // so use the invariant root singular-value bound instead.
            let bound_scale=root_scale.max(source.scale);
            d.source.scale=bound_scale;
            d.local_sphere=bind[index].inverse().transform_point3(sphere.truncate())
                .extend(sphere.w*(bound_scale/source.scale)).to_array();
        }
        if let Some(volume)=part.volume {
            let owner=part.owner.ok_or("a volume part must have a matching source owner")?;
            if !volumes.insert(volume) {return Err("each animated volume must belong to exactly one part".into());}
            let forest=forest.ok_or("part volume has no uploaded forest")?;
            let volume=*forest.instances.get(volume as usize).ok_or("part volume index out of range")?;
            if volume.meta[0]!=owner || !matrix_close(matrix(volume.inverse),bind[index].inverse()) {return Err(format!("part {index} volume owner/inverse differs from its source bind pose"));}
            let first=volume.meta[1] as usize;let count=volume.meta[2] as usize;
            let levels=first.checked_add(count).and_then(|end|forest.levels.get(first..end)).filter(|levels|!levels.is_empty())
                .ok_or("part volume has invalid packed levels")?;
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for level in levels {
                let origin=Vec3::from_slice(&level.origin_cell[..3]);let h=level.origin_cell[3];
                if !origin.is_finite() || !h.is_finite() || h<=0.0 || level.dims_offset[..3].contains(&0) {return Err("part volume has invalid level support".into());}
                let dims=Vec3::new(level.dims_offset[0] as f32,level.dims_offset[1] as f32,level.dims_offset[2] as f32);
                lo=lo.min(origin-Vec3::splat(h*0.5));hi=hi.max(origin+(dims+Vec3::splat(0.5))*h);
            }
            if !lo.is_finite() || !hi.is_finite() {return Err("part volume support overflows".into());}
            d.volume=volume;d.local_min=lo.extend(0.0).to_array();d.local_max=hi.extend(0.0).to_array();
        }
        descriptors.push(d);
    }
    let mut attached=Vec::with_capacity(attachments.len());
    for (index,attachment) in attachments.iter().enumerate() {
        let pose=attachment.pose as usize;
        let bind=*bind.get(pose).ok_or_else(||format!("attachment {index} branch pose is out of range"))?;
        let local_scale=attachment_scale(attachment.rest)?;
        let world=bind*attachment.rest;
        if !world.is_finite() || !world.inverse().is_finite() {
            return Err(format!("attachment {index} world bind transform is singular/non-finite"));
        }
        let source=*instances.get(attachment.owner as usize).ok_or_else(||format!("attachment {index} source owner is out of range"))?;
        if !owners.insert(attachment.owner as usize) {
            return Err("each animated source owner must belong to exactly one branch OR primitive attachment".into());
        }
        if source.flags&PART_WIND==0 {return Err(format!("attachment {index} source must carry PART_WIND to prevent duplicate whole-plant shear"));}
        if !matrix_close(matrix(source.rows),world) {return Err(format!("attachment {index} source transform differs from branch bind times primitive rest"));}
        let sphere=Vec4::from_array(source.sphere);
        if !sphere.is_finite() || sphere.w<0.0 || !source.scale.is_finite() || source.scale<=0.0 {
            return Err(format!("attachment {index} source bounds/scale are invalid"));
        }
        let expected_scale=[world.x_axis,world.y_axis,world.z_axis].iter().map(|a|a.truncate().length()).fold(0.0,f32::max);
        if (source.scale-expected_scale).abs()>2e-4*source.scale.max(expected_scale) {
            return Err(format!("attachment {index} source scale does not match the Scene::add_instance bounds convention"));
        }
        // Scene::add_instance uses max column length for scale and multiplies
        // its mesh-local radius by that scale. Recover that local radius, then
        // use ||root||2 * ||attachment||2, valid for ALL intervening rotations.
        let bound_scale=root_scales[pose]*local_scale;
        let local_center=world.inverse().transform_point3(sphere.truncate());
        let radius=(sphere.w/source.scale)*bound_scale;
        if !bound_scale.is_finite() || bound_scale<=0.0 || !local_center.is_finite() || !radius.is_finite() {
            return Err(format!("attachment {index} conservative bounds overflow"));
        }
        let mut d=AttachmentDescriptor::zeroed();d.rest=rows(attachment.rest);d.source=source;
        d.source.scale=bound_scale;d.local_sphere=local_center.extend(radius).to_array();
        d.identity=[attachment.pose,attachment.owner,assembly_for_pose[pose],0];attached.push(d);
    }
    // Stable packing gives each assembly contiguous attachment pages. Runtime
    // wakes pages on the GPU instead of visiting every sleeping primitive.
    attached.sort_by_key(|a|a.identity[2]);
    let mut assembly_descriptors:Vec<_>=assemblies.iter().map(|range|AssemblyDescriptor {
        range:[range[0],range[1],0,0],sphere:bind[range[0] as usize].w_axis.to_array(),motion:[0.0;4]
    }).collect();
    for assembly in &mut assembly_descriptors {assembly.sphere[3]=0.0;}
    let mut include=|pose:usize,mut lever:f32|->Result<(),String> {
        let assembly=&mut assembly_descriptors[assembly_for_pose[pose] as usize];
        let mut motion=0.0;let mut index=pose;
        loop {
            // Arc length is linear in wind strength/LOD amplitude. A chord
            // bound multiplied by strength would underestimate smaller angles.
            motion+=parts[index].max_angle*lever;
            let Some(parent)=parts[index].parent else {break;};
            lever+=parts[index].rest.w_axis.truncate().length()*root_scales[pose];index=parent as usize;
        }
        if !lever.is_finite() || !motion.is_finite() {return Err("part wind assembly envelope overflows".into());}
        assembly.sphere[3]=assembly.sphere[3].max(lever);
        assembly.motion[0]=assembly.motion[0].max(motion);
        Ok(())
    };
    for (index,d) in descriptors.iter().enumerate() {
        let mut lever=0.0f32;
        if d.identity[1]!=NONE {lever=Vec3::from_slice(&d.local_sphere[..3]).length()*root_scales[index]+d.local_sphere[3];}
        if d.identity[2]!=NONE {lever=lever.max(Vec3::from_slice(&d.local_min[..3]).abs()
            .max(Vec3::from_slice(&d.local_max[..3]).abs()).length()*root_scales[index]);}
        include(index,lever)?;
    }
    for d in &attached {
        let pose=d.identity[0] as usize;
        let lever=matrix(d.rest).transform_point3(Vec3::from_slice(&d.local_sphere[..3])).length()*root_scales[pose]
            +d.local_sphere[3];
        include(pose,lever)?;
    }
    let mut pages=Vec::new();let mut at=0;
    for (index,assembly) in assembly_descriptors.iter_mut().enumerate() {
        assembly.range[2]=pages.len() as u32;
        while at<attached.len() && attached[at].identity[2]==index as u32 {
            let start=at;
            while at<attached.len() && at-start<ATTACHMENT_PAGE && attached[at].identity[2]==index as u32 {at+=1;}
            pages.push([start as u32,(at-start) as u32,index as u32,0]);
        }
        assembly.range[3]=pages.len() as u32-assembly.range[2];
    }
    let mut owners:Vec<_>=owners.into_iter().collect();owners.sort_unstable();
    Ok(Packed {parts:descriptors,attachments:attached,assemblies:assembly_descriptors,pages,owners})
}

/// Validate the exact hierarchy, attachments and conservative bounds used by
/// GPU upload before committing a scene-construction transaction. This only
/// prepares CPU descriptors; it creates no GPU resources or animated poses.
pub(crate) fn validate_bindings(parts:&[PartWind],attachments:&[PartAttachment],instances:&[GpuInstance],
    forest:Option<&GpuForest>)->Result<(),String> {
    // No rig at all (an all-proxy forest): nothing is uploaded (frame.rs).
    if parts.is_empty() && attachments.is_empty() {return Ok(());}
    pack(parts,attachments,instances,forest).map(|_| ())
}

fn field(state:&WindState)->Result<[f32;4],String> {
    let state=state.validated()?;
    if state.strength==0.0 {return Ok([0.0;4]);}
    Ok([state.direction.x,state.direction.y,state.strength,
        (state.time_seconds.rem_euclid(PERIOD)*std::f64::consts::TAU/PERIOD) as f32])
}

/// Small streamed rigs can retain their validated rest hierarchy and evaluate
/// the same oscillator as the GPU into a caller-owned pose buffer.
pub struct PartWindRig {
    parts: Vec<PartWind>,
}
impl PartWindRig {
    pub fn new(parts: Vec<PartWind>) -> Result<Self, String> {
        bind_poses(&parts)?;
        Ok(Self { parts })
    }

    pub fn sample_into(&self, state: &WindState, poses: &mut Vec<Mat4>) -> Result<(), String> {
        evaluate_poses(&self.parts, state, poses)
    }
}

/// One-off CPU pose sample using the same validated hierarchy as streamed rigs.
pub fn sample_poses(parts: &[PartWind], state: &WindState) -> Result<Vec<Mat4>, String> {
    bind_poses(parts)?;
    let mut poses = Vec::with_capacity(parts.len());
    evaluate_poses(parts, state, &mut poses)?;
    Ok(poses)
}

fn evaluate_poses(parts: &[PartWind], state: &WindState, result: &mut Vec<Mat4>) -> Result<(), String> {
    let field = field(state)?;
    result.clear();
    let direction = Vec2::new(field[0], field[1]);
    let crosswind = Vec2::new(-direction.y, direction.x);
    let flutter_dir = direction * 0.4 + crosswind * 0.9165;
    let mut gust = Vec2::ZERO;
    let mut flutter = 0.0;
    for part in parts {
        let base = if let Some(parent) = part.parent {
            result[parent as usize] * part.rest
        } else {
            let root = part.rest.w_axis;
            gust = gust_at(Vec2::new(root.x, root.z), direction, field[3]);
            flutter = 0.30 * (0.35 + 0.65 * gust.dot(direction).abs());
            part.rest
        };
        let phase = field[3] * part.harmonic as f32 + part.phase.rem_euclid(std::f32::consts::TAU);
        let force = 0.70 * gust + flutter_dir * (flutter * phase.sin());
        // The long axis of a growth limb is local +Y. Transverse load
        // bends it around its attachment; a wind-aligned limb feels less.
        let torque = base.y_axis.truncate().normalize().cross(Vec3::new(force.x, 0.0, force.y));
        let magnitude = torque.length();
        let angle = part.max_angle * field[2] * magnitude;
        let rotation = if magnitude > 1e-6 {
            Quat::from_axis_angle(torque / magnitude, angle)
        } else {
            Quat::IDENTITY
        };
        // Rotate the three axes in place to preserve the exact world pivot.
        result.push(Mat4::from_cols(
            (rotation * base.x_axis.truncate()).extend(0.0),
            (rotation * base.y_axis.truncate()).extend(0.0),
            (rotation * base.z_axis.truncate()).extend(0.0),
            base.w_axis,
        ));
    }
    Ok(())
}

pub struct PartWindRenderer {
    pipeline:wgpu::ComputePipeline,bind:wgpu::BindGroup,uniform:wgpu::Buffer,poses:wgpu::Buffer,
    schedule_pass:(wgpu::ComputePipeline,wgpu::BindGroup),dispatch_pass:(wgpu::ComputePipeline,wgpu::BindGroup),
    views:wgpu::Buffer,schedules:wgpu::Buffer,counter:wgpu::Buffer,dispatch:wgpu::Buffer,dispatch_width:u32,page_count:u32,gpu_bytes:u64,
    attachment_pass:Option<(wgpu::ComputePipeline,wgpu::BindGroup)>,
    assembly_count:u32,part_count:u32,attachment_count:u32,owners:Vec<usize>,state:Mutex<State>,
    descriptors:wgpu::Buffer,assemblies:wgpu::Buffer,
    /// Root pose -> its assembly and the root's built rest and envelope.
    roots:std::collections::HashMap<u32,(u32,Mat4,AssemblyDescriptor)>,
}
impl PartWindRenderer {
    pub fn new(gpu:&Gpu,parts:&[PartWind],attachments:&[PartAttachment],scene_instances:&[GpuInstance],instance_buffer:&wgpu::Buffer,
        forest:Option<(&GpuForest,&wgpu::Buffer)>)->Result<Self,String> {
        Self::new_with_limits(gpu,parts,attachments,scene_instances,instance_buffer,forest,RuntimeLimits::default())
    }

    pub fn new_with_limits(gpu:&Gpu,parts:&[PartWind],attachments:&[PartAttachment],scene_instances:&[GpuInstance],instance_buffer:&wgpu::Buffer,
        forest:Option<(&GpuForest,&wgpu::Buffer)>,limits:RuntimeLimits)->Result<Self,String> {
        let packed=pack_limited(parts,attachments,scene_instances,forest.map(|(f,_)|f),limits)?;
        if instance_buffer.size()<(scene_instances.len()*std::mem::size_of::<GpuInstance>()) as u64
            || !instance_buffer.usage().contains(wgpu::BufferUsages::STORAGE) {return Err("part wind requires the writable live source instance storage buffer".into());}
        if let Some((forest,buffer))=forest {
            if buffer.size()<(forest.instances.len()*std::mem::size_of::<GpuTree>()) as u64
                || !buffer.usage().contains(wgpu::BufferUsages::STORAGE) {return Err("part wind requires the live volume instance storage buffer".into());}
        }
        let device_limits=gpu.device.limits();
        let sizes=[packed.parts.len() as u64*std::mem::size_of::<Descriptor>() as u64,
            packed.assemblies.len() as u64*std::mem::size_of::<AssemblyDescriptor>() as u64,
            parts.len() as u64*std::mem::size_of::<GpuPartPose>() as u64,
            packed.attachments.len() as u64*std::mem::size_of::<AttachmentDescriptor>() as u64,
            packed.assemblies.len() as u64*std::mem::size_of::<AssemblyState>() as u64,
            packed.pages.len() as u64*16,packed.pages.len().max(1) as u64*4,
            MAX_ANIMATION_VIEWS as u64*std::mem::size_of::<GpuAnimationView>() as u64,16,4,
            if forest.is_none(){std::mem::size_of::<GpuTree>() as u64}else{0}];
        let gpu_bytes=validate_allocation(&sizes,&device_limits,limits.max_gpu_bytes)?;
        let dispatch_width=device_limits.max_compute_workgroups_per_dimension.min(65_535);
        let assembly_groups=(packed.assemblies.len() as u64).div_ceil(32);
        if dispatch_width==0 || assembly_groups>u64::from(dispatch_width).pow(2)
            || packed.pages.len() as u64>u64::from(dispatch_width).pow(2) {
            return Err("part wind exceeds device dispatch dimensions".into());
        }
        let upload=|label,bytes:&[u8],usage|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some(label),contents:bytes,usage});
        let storage=|label,size,extra|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,
            usage:wgpu::BufferUsages::STORAGE|extra,mapped_at_creation:false});
        let descriptors=upload("part wind bind poses",bytemuck::cast_slice(&packed.parts),wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST);
        let assemblies=upload("part wind assemblies",bytemuck::cast_slice(&packed.assemblies),wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST);
        let roots=packed.assemblies.iter().enumerate().map(|(index,a)|
            (a.range[0],(index as u32,parts[a.range[0] as usize].rest,*a))).collect();
        let uniform=upload("part wind state",bytemuck::bytes_of(&Uniform::zeroed()),wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
        let poses=storage("part wind current previous poses",sizes[2],wgpu::BufferUsages::COPY_SRC);
        let schedules=storage("part wind assembly schedules",sizes[4],wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST);
        let active_pages=storage("part wind active attachment pages",sizes[6],wgpu::BufferUsages::empty());
        let views=storage("part wind animation consumers",sizes[7],wgpu::BufferUsages::COPY_DST);
        let counter=storage("part wind active attachment count",4,wgpu::BufferUsages::COPY_DST);
        let dispatch=storage("part wind indirect attachment dispatch",16,wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::INDIRECT);
        let dummy;
        let volume_buffer=if let Some((_,buffer))=forest {buffer} else {
            dummy=upload("empty part wind volumes",bytemuck::bytes_of(&GpuTree::zeroed()),wgpu::BufferUsages::STORAGE);&dummy
        };
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("rigid assembly part wind"),source:wgpu::ShaderSource::Wgsl(SHADER.into())});
        let pipeline_for=|entry,label|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(label),layout:None,
            module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
        let group_for=|pipeline:&wgpu::ComputePipeline,label,bindings:&[(u32,&wgpu::Buffer)]| {
            let entries:Vec<_>=bindings.iter().map(|&(binding,buffer)|wgpu::BindGroupEntry {binding,resource:buffer.as_entire_binding()}).collect();
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some(label),layout:&pipeline.get_bind_group_layout(0),entries:&entries})
        };
        let scheduler=pipeline_for("schedule_assemblies","part wind GPU assembly scheduler");
        let scheduler_bind=group_for(&scheduler,"part wind GPU assembly scheduler",&[(0,&uniform),(2,&assemblies),
            (7,&schedules),(8,&views),(9,&counter),(10,&active_pages)]);
        let dispatch_pipeline=pipeline_for("finish_dispatch","part wind GPU indirect arguments");
        let dispatch_bind=group_for(&dispatch_pipeline,"part wind GPU indirect arguments",&[(0,&uniform),(9,&counter),(12,&dispatch)]);
        let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("rigid assembly part wind"),layout:None,
            module:&module,entry_point:Some("update_parts"),compilation_options:Default::default(),cache: gpu.pipeline_cache()});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("rigid assembly part wind"),layout:&pipeline.get_bind_group_layout(0),
            entries:&[wgpu::BindGroupEntry {binding:0,resource:uniform.as_entire_binding()},
                wgpu::BindGroupEntry {binding:1,resource:descriptors.as_entire_binding()},
                wgpu::BindGroupEntry {binding:2,resource:assemblies.as_entire_binding()},
                wgpu::BindGroupEntry {binding:3,resource:poses.as_entire_binding()},
                wgpu::BindGroupEntry {binding:4,resource:instance_buffer.as_entire_binding()},
                wgpu::BindGroupEntry {binding:5,resource:volume_buffer.as_entire_binding()},
                wgpu::BindGroupEntry {binding:7,resource:schedules.as_entire_binding()}]});
        let attachment_pass=if packed.attachments.is_empty() {None}else{
            let attached=upload("part wind primitive attachments",bytemuck::cast_slice(&packed.attachments),wgpu::BufferUsages::STORAGE);
            let pages=upload("part wind attachment page descriptors",bytemuck::cast_slice(&packed.pages),wgpu::BufferUsages::STORAGE);
            let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("shared branch primitive scatter"),layout:None,
                module:&module,entry_point:Some("update_attachments"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shared branch primitive scatter"),layout:&pipeline.get_bind_group_layout(0),
                entries:&[wgpu::BindGroupEntry {binding:0,resource:uniform.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:3,resource:poses.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:4,resource:instance_buffer.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:6,resource:attached.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:7,resource:schedules.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:9,resource:counter.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:10,resource:active_pages.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:11,resource:pages.as_entire_binding()}]});
            Some((pipeline,bind))
        };
        Ok(Self {pipeline,bind,uniform,poses,attachment_pass,schedule_pass:(scheduler,scheduler_bind),
            dispatch_pass:(dispatch_pipeline,dispatch_bind),views,schedules,counter,dispatch,dispatch_width,
            page_count:packed.pages.len() as u32,gpu_bytes,assembly_count:packed.assemblies.len() as u32,
            part_count:parts.len() as u32,attachment_count:packed.attachments.len() as u32,
            owners:packed.owners,state:Mutex::new(State {field:[0.0;4],initialized:false,scheduled:false,views:Vec::new()}),
            descriptors,assemblies,roots})
    }

    /// Move the assembly rooted at pose `root`: `delta` maps its built root
    /// onto the new one, `ratio` (at most 1: bounds were built for the built
    /// scale) is the scale change. Children follow their parents; the next
    /// encode rewrites every pose, member and volume once, without motion.
    pub fn move_assembly(&self,gpu:&Gpu,root:u32,delta:Mat4,ratio:f32)->Result<(),String> {
        let &(index,rest,built)=self.roots.get(&root).ok_or("moved part is not an assembly root")?;
        if !delta.is_finite() || !(ratio>0.0 && ratio<=1.0+1e-4) {return Err("an assembly moves rigidly and may only shrink".into());}
        let rest=delta*rest;
        gpu.queue.write_buffer(&self.descriptors,(root as usize*std::mem::size_of::<Descriptor>()) as u64,bytemuck::cast_slice(&rows(rest)));
        let mut moved=built;
        moved.sphere=delta.transform_point3(Vec3::from_slice(&built.sphere[..3])).extend(built.sphere[3]*ratio).to_array();
        moved.motion[0]=built.motion[0]*ratio;
        let at=index as u64;
        gpu.queue.write_buffer(&self.assemblies,at*std::mem::size_of::<AssemblyDescriptor>() as u64,bytemuck::bytes_of(&moved));
        // animation.y marks the move for the scheduler (see schedule_assemblies).
        gpu.queue.write_buffer(&self.schedules,at*std::mem::size_of::<AssemblyState>() as u64+16,bytemuck::cast_slice(&[1.0f32,1.0]));
        Ok(())
    }

    pub fn poses(&self)->&wgpu::Buffer {&self.poses}
    pub fn gpu_bytes(&self)->u64 {self.gpu_bytes}
    /// Assembly state and `[active_page_count, dispatch_x, dispatch_y, 1]`.
    /// Diagnostic readback only; no frame-time CPU selection consumes these.
    pub fn schedule_debug_buffers(&self)->(&wgpu::Buffer,&wgpu::Buffer) {(&self.schedules,&self.dispatch)}

    /// Compare with the last encoded animation consumers before deciding to
    /// reuse a shadow cache. Camera-driven LOD changes poses even when time and
    /// wind strength are frozen. This does not select or iterate any plants.
    pub fn animation_views_changed(&self,views:&[AnimationView])->Result<bool,String> {
        let views=Self::pack_views(views)?;let state=self.state.lock().unwrap();
        Ok(!state.initialized || !state.scheduled || state.views!=views)
    }

    fn pack_views(views:&[AnimationView])->Result<Vec<GpuAnimationView>,String> {
        if views.len()>MAX_ANIMATION_VIEWS {return Err(format!("part wind supports at most {MAX_ANIMATION_VIEWS} animation consumers"));}
        views.iter().map(|v|v.packed()).collect()
    }

    pub fn contains_owner_range(&self,first:usize,count:usize)->bool {
        crate::owner_ranges::contains(&self.owners,first,count)
    }

    /// Returns true for a changed pose field, including enabled -> disabled.
    /// The caller must invalidate direct source-geometry shadow caches then.
    pub fn update(&self,_gpu:&Gpu,wind:&WindState)->Result<bool,String> {
        let field=field(wind)?;let mut state=self.state.lock().unwrap();
        let changed=state.field!=field;state.field=field;Ok(changed)
    }

    /// Call once before culling, shadows and all camera passes. Even a fixed
    /// phase copies current to previous so later temporal consumers cannot
    /// accidentally keep an old displacement. No CPU per-part update occurs.
    pub fn encode(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,timestamp:Option<(&wgpu::QuerySet,u32)>)->bool {
        self.encode_internal(gpu,encoder,&[],false,timestamp);
        true
    }

    /// Shared GPU animation LOD, evaluated before every consuming render pass.
    /// The largest demand across all supplied consumers wins. No CPU assembly
    /// list or per-plant update is required; empty lists retain full animation.
    pub fn encode_with_views(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,views:&[AnimationView],
        timestamp:Option<(&wgpu::QuerySet,u32)>)->Result<bool,String> {
        let views=Self::pack_views(views)?;
        self.encode_internal(gpu,encoder,&views,true,timestamp);
        Ok(true)
    }

    fn encode_internal(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,views:&[GpuAnimationView],scheduled:bool,
        timestamp:Option<(&wgpu::QuerySet,u32)>) {
        let mut state=self.state.lock().unwrap();
        let uniform=Uniform {field:state.field,counts:[self.assembly_count,self.part_count,u32::from(!state.initialized),self.attachment_count],
            schedule:[views.len() as u32,u32::from(scheduled),self.dispatch_width,self.page_count],lod:[0.25,1.0,0.0,0.0]};
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&uniform));state.initialized=true;
        state.scheduled=scheduled;state.views.clear();state.views.extend_from_slice(views);
        if !views.is_empty() {gpu.queue.write_buffer(&self.views,0,bytemuck::cast_slice(views));}
        encoder.clear_buffer(&self.counter,0,None);
        let groups=self.assembly_count.div_ceil(32);
        let dispatch=(groups.min(self.dispatch_width),groups.div_ceil(self.dispatch_width));
        {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("part wind GPU assembly scheduler"),
                timestamp_writes:timestamp.map(|(query_set,index)|wgpu::ComputePassTimestampWrites {query_set,
                    beginning_of_pass_write_index:Some(index),end_of_pass_write_index:None})});
            pass.set_pipeline(&self.schedule_pass.0);pass.set_bind_group(0,&self.schedule_pass.1,&[]);
            pass.dispatch_workgroups(dispatch.0,dispatch.1,1);
        }
        if self.attachment_pass.is_some() {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("part wind GPU indirect arguments"),timestamp_writes:None});
            pass.set_pipeline(&self.dispatch_pass.0);pass.set_bind_group(0,&self.dispatch_pass.1,&[]);pass.dispatch_workgroups(1,1,1);
        }
        {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("rigid assembly part wind"),
                timestamp_writes:timestamp.filter(|_|self.attachment_pass.is_none()).map(|(query_set,index)|wgpu::ComputePassTimestampWrites {query_set,
                    beginning_of_pass_write_index:None,end_of_pass_write_index:Some(index+1)})});
            pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);pass.dispatch_workgroups(dispatch.0,dispatch.1,1);
        }
        // A separate pass guarantees all assemblies have written their poses
        // before any primitive reads one, including across workgroups.
        if let Some((pipeline,bind))=&self.attachment_pass {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("shared branch primitive scatter"),
                timestamp_writes:timestamp.map(|(query_set,index)|wgpu::ComputePassTimestampWrites {query_set,
                    beginning_of_pass_write_index:None,end_of_pass_write_index:Some(index+1)})});
            pass.set_pipeline(pipeline);pass.set_bind_group(0,bind,&[]);pass.dispatch_workgroups_indirect(&self.dispatch,4);
        }
    }
}

pub const SHADER:&str=concat!(include_str!("shaders/wind_field.wgsl"), "\n", include_str!("shaders/part_wind.wgsl"));

#[cfg(test)]
mod tests {
    use super::*;
    /// An all-proxy forest has no rig at all; frame.rs then uploads none.
    #[test]
    fn no_rig_is_valid() {
        assert!(validate_bindings(&[], &[], &[], None).is_ok());
    }
    #[test]
    fn rigid_part_compute_shader_validates() {
        let module=naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e|panic!("{}",e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(SHADER)));
        assert!(module.entry_points.iter().any(|e|e.name=="update_parts"&&e.workgroup_size==[32,1,1]));
        assert!(module.entry_points.iter().any(|e|e.name=="update_attachments"&&e.workgroup_size==[64,1,1]));
        assert!(module.entry_points.iter().any(|e|e.name=="schedule_assemblies"&&e.workgroup_size==[32,1,1]));
        assert!(module.entry_points.iter().any(|e|e.name=="finish_dispatch"&&e.workgroup_size==[1,1,1]));
    }

    #[test]
    fn dynamic_population_pages_and_device_budgets_are_explicit() {
        let mut parts=Vec::new();let mut attachments=Vec::new();let mut sources=Vec::new();
        for plant in 0..300 {
            let root=parts.len() as u32;
            let transform=Mat4::from_translation(Vec3::new(plant as f32*3.0,0.0,-50.0));
            parts.push(PartWind {rest:transform,..Default::default()});
            parts.push(PartWind {parent:Some(root),rest:Mat4::from_translation(Vec3::Y*2.0),max_angle:0.2,..Default::default()});
            let owner=sources.len() as u32;
            let rest=Mat4::from_scale(Vec3::new(0.2,0.8,0.3));
            attachments.push(PartAttachment {pose:root+1,owner,rest});
            sources.push(primitive_source(transform*parts[root as usize+1].rest*rest,Vec3::Y*0.5,0.6,0));
        }
        attachments.reverse();
        let packed=pack(&parts,&attachments,&sources,None).unwrap();
        assert_eq!(packed.parts.len(),600);assert!(packed.parts.len()>MAX_PARTS);
        assert_eq!(packed.assemblies.len(),300);assert_eq!(packed.pages.len(),300);
        let mut seen=HashSet::new();
        for (index,assembly) in packed.assemblies.iter().enumerate() {
            assert_eq!(assembly.range[0],index as u32*2);assert_eq!(assembly.range[1],2);
            for page in &packed.pages[assembly.range[2] as usize..][..assembly.range[3] as usize] {
                assert_eq!(page[2],index as u32);
                for attachment in &packed.attachments[page[0] as usize..][..page[1] as usize] {
                    assert_eq!(attachment.identity[2],index as u32);assert!(seen.insert(attachment.identity[1]));
                }
            }
        }
        assert_eq!(seen.len(),sources.len());
        assert!(pack_limited(&parts,&attachments,&sources,None,RuntimeLimits {max_poses:599,..Default::default()}).is_err());
        let device=wgpu::Limits::default();
        let bytes=validate_allocation(&[1024,2048],&device,4096).unwrap();
        assert_eq!(bytes,3072+std::mem::size_of::<Uniform>() as u64);
        assert!(validate_allocation(&[4096],&device,4096).is_err());
        assert!(validate_allocation(&[u64::from(device.max_storage_buffer_binding_size)+1],&device,u64::MAX).is_err());
        assert!(AnimationView {view_projection:Mat4::IDENTITY,viewport:[0,1080]}.packed().is_err());
        assert!(AnimationView {view_projection:Mat4::ZERO,viewport:[1920,1080]}.packed().is_err());
        assert_eq!(std::mem::size_of::<Uniform>(),64);
        assert_eq!(std::mem::size_of::<GpuAnimationView>(),80);
        assert_eq!(std::mem::size_of::<AssemblyDescriptor>(),48);
        assert_eq!(std::mem::size_of::<AssemblyState>(),32);
    }

    #[test]
    fn assembly_envelope_contains_nested_scaled_motion_and_bounds_linear_strength() {
        let mut parts=assembly();parts[0].max_angle=0.3;
        let (bind,_)=bind_poses(&parts).unwrap();
        let local=Mat4::from_scale_rotation_translation(Vec3::new(0.1,1.7,0.3),
            Quat::from_rotation_y(0.4),Vec3::new(0.2,0.6,0.1));
        let attachment=PartAttachment {pose:2,owner:0,rest:local};
        let center=Vec3::new(0.2,0.5,-0.1);let radius=0.8;
        let source=primitive_source(bind[2]*local,center,radius,17);
        let packed=pack(&parts,&[attachment],&[source],None).unwrap();let envelope=packed.assemblies[0];
        for strength in [0.0,0.1,0.4,0.8,1.0] {for time_seconds in [0.0,0.37,2.1,30.0,119.9] {
            let poses=sample_poses(&parts,&WindState {strength,time_seconds,..Default::default()}).unwrap();
            for sample in 0..128 {
                let y=1.0-2.0*(sample as f32+0.5)/128.0;let angle=sample as f32*2.3999631;
                let direction=Vec3::new((1.0-y*y).sqrt()*angle.cos(),y,(1.0-y*y).sqrt()*angle.sin());
                let p=local.transform_point3(center+direction*radius);
                let current=poses[2].transform_point3(p);let rest=bind[2].transform_point3(p);
                assert!(current.distance(Vec3::from_slice(&envelope.sphere[..3]))<=envelope.sphere[3]+1e-5);
                assert!(current.distance(rest)<=envelope.motion[0]*strength+1e-5);
            }
        }}
    }

    /// Exercises the real GPU compaction and history transition, including
    /// populations beyond the former fixed pose/assembly limit. Run serially
    /// with other GPU fixtures; there is no readback in the production path.
    #[test]
    #[ignore = "requires an exclusive GPU fixture slot"]
    fn gpu_scheduler_compacts_pages_unions_views_and_resets_reentry_history() {
        fn read<T:Pod>(gpu:&Gpu,buffer:&wgpu::Buffer)->Vec<T> {
            let output=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("part scheduler oracle readback"),
                size:buffer.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut encoder=gpu.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(buffer,0,&output,0,buffer.size());
            let submission=gpu.queue.submit([encoder.finish()]);let (send,receive)=std::sync::mpsc::sync_channel(1);
            output.slice(..).map_async(wgpu::MapMode::Read,move |result|{let _=send.send(result);});
            gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),
                timeout:Some(std::time::Duration::from_secs(30))}).unwrap();
            receive.try_recv().unwrap().unwrap();
            let bytes=output.slice(..).get_mapped_range().unwrap();
            bytes.chunks_exact(std::mem::size_of::<T>()).map(bytemuck::pod_read_unaligned).collect()
        }
        let gpu=Gpu::new().unwrap();let mut parts=Vec::new();let mut attachments=Vec::new();let mut sources=Vec::new();
        for plant in 0..300 {
            let root=parts.len() as u32;
            let origin=Mat4::from_translation(Vec3::new((plant/100) as f32*1000.0,0.0,-20.0));
            parts.push(PartWind {rest:origin,..Default::default()});
            parts.push(PartWind {parent:Some(root),rest:Mat4::from_translation(Vec3::Y*2.0),max_angle:0.3,phase:0.4,..Default::default()});
            // The first assembly spans two pages, exposing page-boundary bugs
            // that one-attachment-per-assembly fixtures cannot detect.
            for primitive in 0..if plant==0 {257}else{1} {
                let rest=Mat4::from_translation(Vec3::X*(primitive as f32/512.0));
                let owner=sources.len() as u32;attachments.push(PartAttachment {pose:root+1,owner,rest});
                sources.push(primitive_source(origin*parts[root as usize+1].rest*rest,Vec3::Y*0.5,0.6,19));
            }
        }
        let instances=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("part scheduler oracle instances"),
            contents:bytemuck::cast_slice(&sources),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC});
        let renderer=PartWindRenderer::new(&gpu,&parts,&attachments,&sources,&instances,None).unwrap();
        let mut wind=WindState {strength:0.8,time_seconds:3.1,..Default::default()};
        renderer.update(&gpu,&wind).unwrap();
        let timestamp=gpu.summary().timestamps.then(||gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label:Some("part wind timestamp regression"),ty:wgpu::QueryType::Timestamp,count:2 }));
        let frame=|views:Option<&[AnimationView]>| {
            let mut encoder=gpu.device.create_command_encoder(&Default::default());
            if let Some(views)=views {renderer.encode_with_views(&gpu,&mut encoder,views,timestamp.as_ref().map(|q|(q,0))).unwrap();}
                else {renderer.encode(&gpu,&mut encoder,timestamp.as_ref().map(|q|(q,0)));}
            gpu.queue.submit([encoder.finish()]);
            (read::<AssemblyState>(&gpu,renderer.schedule_debug_buffers().0),
                read::<u32>(&gpu,renderer.schedule_debug_buffers().1))
        };
        let camera=|x:f32|AnimationView {
            view_projection:Mat4::perspective_infinite_reverse_rh(1.0,1.0,0.1)
                *Mat4::look_at_rh(Vec3::new(x,2.0,0.0),Vec3::new(x,2.0,-20.0),Vec3::Y),viewport:[1024;2],
        };
        let absent=[camera(-1000.0)];
        let (all,dispatch)=frame(None);
        assert_eq!(dispatch[0],301);assert!(all.iter().all(|s|s.state[0]==2&&s.animation[0]==1.0));
        let expected=sample_poses(&parts,&wind).unwrap();
        for (pose,expected) in read::<GpuPartPose>(&gpu,renderer.poses()).iter().zip(&expected) {
            assert!(matrix_close(matrix(pose.current),*expected));assert_eq!(pose.current,pose.previous);
        }
        assert!(renderer.animation_views_changed(&absent).unwrap());
        let (enter,dispatch)=frame(Some(&absent));
        assert_eq!(dispatch[0],301);assert!(enter.iter().all(|s|s.state[0]==2&&s.animation[0]==0.0));
        assert!(!renderer.animation_views_changed(&absent).unwrap());
        let (settle,dispatch)=frame(Some(&absent));
        assert_eq!(dispatch[0],0);assert!(settle.iter().all(|s|s.state[0]==1));
        let (sleep,dispatch)=frame(Some(&absent));
        assert_eq!(dispatch[0],0);assert!(sleep.iter().all(|s|s.state[0]==0));
        for pose in read::<GpuPartPose>(&gpu,renderer.poses()) {assert_eq!(pose.current,pose.previous);}
        let (partial,dispatch)=frame(Some(&[camera(0.0)]));
        assert_eq!(dispatch[0],101);
        assert!(partial[..100].iter().all(|s|s.animation[0]==1.0&&s.state[2]==HISTORY_EPOCH));
        assert!(partial[100..].iter().all(|s|s.state[0]==0&&s.animation[0]==0.0));
        // A different player and an orthographic shadow map can each wake
        // plants outside player one's frustum. All four consumers are unioned.
        let shadow=AnimationView {view_projection:Mat4::orthographic_rh(-10.0,10.0,-10.0,10.0,0.1,100.0)
            *Mat4::look_at_rh(Vec3::new(1000.0,2.0,0.0),Vec3::new(1000.0,2.0,-20.0),Vec3::Y),viewport:[2048;2]};
        let union=[camera(0.0),shadow,camera(2000.0),camera(-1000.0)];
        let (wake,dispatch)=frame(Some(&union));
        assert_eq!(dispatch[0],301);assert!(wake.iter().all(|s|s.state[0]==2&&s.state[2]==HISTORY_EPOCH&&s.animation[0]==1.0));
        let resumed=read::<GpuPartPose>(&gpu,renderer.poses());
        for (pose,expected) in resumed.iter().zip(&expected) {
            assert!(matrix_close(matrix(pose.current),*expected));assert_eq!(pose.current,pose.previous);
        }
        for (source,attachment) in read::<GpuInstance>(&gpu,&instances).iter().zip(&attachments) {
            assert_eq!(source.flags,sources[attachment.owner as usize].flags^HISTORY_EPOCH);
            assert_eq!(source.first_cluster,19);assert_eq!(source.cluster_count,7);
            assert!(matrix_close(matrix(source.rows),expected[attachment.pose as usize]*attachment.rest));
        }
        wind.time_seconds+=0.25;renderer.update(&gpu,&wind).unwrap();frame(Some(&union));
        let next=sample_poses(&parts,&wind).unwrap();
        for ((pose,previous),expected) in read::<GpuPartPose>(&gpu,renderer.poses()).iter().zip(&resumed).zip(next) {
            assert_eq!(pose.previous,previous.current);assert!(matrix_close(matrix(pose.current),expected));
        }
    }

    fn assembly()->Vec<PartWind> {
        vec![PartWind {rest:Mat4::from_scale_rotation_translation(Vec3::new(1.2,0.8,1.4),Quat::from_rotation_y(0.3),Vec3::new(12.0,2.0,-8.0)),..Default::default()},
            PartWind {parent:Some(0),rest:Mat4::from_translation(Vec3::new(0.0,2.0,0.0)),max_angle:0.15,phase:0.7,..Default::default()},
            PartWind {parent:Some(1),rest:Mat4::from_rotation_translation(Quat::from_rotation_z(0.2),Vec3::new(0.0,1.0,0.0)),max_angle:0.08,harmonic:17,phase:1.1,..Default::default()}]
    }
    #[test]
    fn cached_growth_branch_poses_use_the_resident_plant_oscillator() {
        let mut parts = assembly();
        for (i, part) in parts.iter_mut().enumerate() {
            part.max_angle = growth_angle(part.parent, 0.7);
            (part.harmonic, part.phase) = growth_oscillator(47, 731+i as u64);
        }
        assert_eq!(parts[0].max_angle, 0.0);
        assert_eq!(growth_angle(Some(0), 0.0), 0.6);
        assert!(growth_angle(Some(0), 0.8) < growth_angle(Some(0), 0.2));
        assert_ne!(growth_oscillator(47, 731), growth_oscillator(48, 731));
        let rig = PartWindRig::new(parts.clone()).unwrap();
        let mut poses = Vec::with_capacity(parts.len());
        let buffer = poses.as_ptr();
        for (strength, time_seconds) in [(0.0, 17.0), (0.8, 0.2), (0.8, 120.2), (0.5, 21.0)] {
            let state = WindState { strength, time_seconds, ..Default::default() };
            rig.sample_into(&state, &mut poses).unwrap();
            assert_eq!(poses, sample_poses(&parts, &state).unwrap());
            assert_eq!(poses.as_ptr(), buffer);
        }
        parts[1].parent = Some(2);
        assert!(PartWindRig::new(parts).is_err());
    }

    #[test]
    fn a_shared_gust_deflects_flexible_limbs_individually_around_their_pivots() {
        let make_limb = |x, stiffness, phase| PartWind {
            parent: Some(0), rest: Mat4::from_translation(Vec3::new(x, 4.0, 0.0)),
            max_angle: growth_angle(Some(0), stiffness), harmonic: 17, phase, ..Default::default()
        };
        let parts = [PartWind::default(), make_limb(-1.0, 0.3, 0.0),
            make_limb(1.0, 0.3, std::f32::consts::PI), make_limb(0.0, 0.9, 0.0)];
        let mut max_separation = 0.0f32;
        let mut max_soft = 0.0f32;
        let mut max_stiff = 0.0f32;
        for i in 0..120 {
            let state = WindState { time_seconds: i as f64, direction: Vec2::X, strength: 1.0, ..Default::default() };
            let pose = sample_poses(&parts, &state).unwrap();
            assert_eq!(pose[0], Mat4::IDENTITY);
            for j in 1..4 { assert_eq!(pose[j].w_axis, parts[j].rest.w_axis); }
            let deflection = |j:usize| (pose[j].y_axis.truncate() - Vec3::Y).length();
            max_soft = max_soft.max(deflection(1));
            max_stiff = max_stiff.max(deflection(3));
            max_separation = max_separation.max((pose[1].y_axis - pose[2].y_axis).length());
        }
        assert!(max_separation > 0.12, "independent limb displacement {max_separation}");
        assert!(max_soft > 0.2 && max_soft > 6.0 * max_stiff, "{max_soft}, {max_stiff}");
    }

    #[test]
    fn parent_attachment_root_and_inverse_ray_stretch_are_preserved() {
        let parts=assembly();let (rest,_)=bind_poses(&parts).unwrap();
        let wind=WindState {strength:0.8,time_seconds:2.3,..Default::default()};let poses=sample_poses(&parts,&wind).unwrap();
        assert!(matrix_close(poses[0],rest[0]));
        for i in 1..parts.len() {
            let parent=parts[i].parent.unwrap() as usize;
            assert!(poses[i].w_axis.truncate().distance(poses[parent].transform_point3(parts[i].rest.w_axis.truncate()))<2e-6);
            // Frobenius norm is unchanged by any left/right rigid rotations.
            let norm=|m:Mat4|[m.x_axis,m.y_axis,m.z_axis].iter().map(|v|v.truncate().length_squared()).sum::<f32>();
            assert!((norm(poses[i].inverse())-norm(rest[i].inverse())).abs()<2e-5);
            let local=Vec3::new(0.17,0.6,-0.21);let world=poses[i].transform_point3(local);
            assert!(poses[i].inverse().transform_point3(world).distance(local)<3e-6);
            let normal=Vec3::new(0.4,0.3,0.8).normalize();let tangent=normal.any_orthonormal_vector();
            assert!(poses[i].inverse().transpose().transform_vector3(normal).dot(poses[i].transform_vector3(tangent)).abs()<1e-5);
        }
        assert!(poses[2].transform_point3(Vec3::Y).distance(rest[2].transform_point3(Vec3::Y))>0.01);
    }
    #[test]
    fn disable_and_period_wrap_preserve_the_bind_pose_contract() {
        let parts=assembly();let (rest,_)=bind_poses(&parts).unwrap();
        let disabled=sample_poses(&parts,&WindState::default()).unwrap();
        assert!(rest.iter().zip(disabled).all(|(a,b)|matrix_close(*a,b)));
        let a=sample_poses(&parts,&WindState {strength:0.6,time_seconds:0.125,..Default::default()}).unwrap();
        let b=sample_poses(&parts,&WindState {strength:0.6,time_seconds:120.125,..Default::default()}).unwrap();
        assert!(a.iter().zip(b).all(|(a,b)|matrix_close(*a,b)));
    }
    #[test]
    fn invalid_hierarchy_scale_and_duplicate_owner_are_rejected() {
        let parts=assembly();let mut bad=parts.clone();bad[1].parent=Some(2);assert!(bind_poses(&bad).is_err());
        let mut bad=parts.clone();bad[2].rest*=Mat4::from_scale(Vec3::new(1.0,2.0,1.0));assert!(bind_poses(&bad).is_err());
        let mut bad=parts.clone();bad[0].rest.x_axis.y=0.4;assert!(bind_poses(&bad).is_err());
        let mut bad=parts.clone();bad[1].max_angle=f32::NAN;assert!(bind_poses(&bad).is_err());
        let source=GpuInstance {rows:rows(Mat4::IDENTITY),sphere:[0.0,0.0,0.0,1.0],first_cluster:0,cluster_count:1,scale:1.0,flags:PART_WIND};
        let p=PartWind {owner:Some(0),..Default::default()};
        assert!(pack(&[p.clone(),p],&[],&[source],None).is_err());
        assert_eq!(std::mem::size_of::<GpuPartPose>(),96);
        assert_eq!(std::mem::size_of::<Descriptor>(),304);
        assert_eq!(std::mem::size_of::<AttachmentDescriptor>(),160);
        assert_eq!(std::mem::offset_of!(AttachmentDescriptor,source),48);
        assert_eq!(std::mem::offset_of!(AttachmentDescriptor,local_sphere),128);
        assert_eq!(std::mem::offset_of!(AttachmentDescriptor,identity),144);
    }
    #[test]
    fn rotated_children_retain_a_conservative_sphere_under_nonuniform_root_scale() {
        let root=PartWind {rest:Mat4::from_scale(Vec3::new(2.0,1.0,0.5)),..Default::default()};
        let child=PartWind {parent:Some(0),owner:Some(0),rest:Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4),..Default::default()};
        let world=root.rest*child.rest;
        let columns=[world.x_axis,world.y_axis,world.z_axis];
        let old_scale=columns.iter().map(|v|v.truncate().length()).fold(0.0,f32::max);
        assert!(old_scale<2.0);
        let source=GpuInstance {rows:rows(world),sphere:[0.0,0.0,0.0,old_scale],
            first_cluster:0,cluster_count:1,scale:old_scale,flags:PART_WIND};
        let packed=pack(&[root,child],&[],&[source],None).unwrap();
        assert_eq!(packed.parts[1].source.scale,2.0);
        assert!((packed.parts[1].local_sphere[3]-2.0).abs()<1e-6);
        let direction=Vec3::new(std::f32::consts::FRAC_1_SQRT_2,-std::f32::consts::FRAC_1_SQRT_2,0.0);
        assert!(world.transform_vector3(direction).length()<=packed.parts[1].local_sphere[3]+1e-6);
    }

    fn primitive_source(world:Mat4,local_center:Vec3,local_radius:f32,first_cluster:u32)->GpuInstance {
        let scale=[world.x_axis,world.y_axis,world.z_axis].iter().map(|c|c.truncate().length()).fold(0.0,f32::max);
        GpuInstance {rows:rows(world),sphere:world.transform_point3(local_center).extend(local_radius*scale).to_array(),
            first_cluster,cluster_count:7,scale,flags:PART_WIND|1}
    }

    #[test]
    fn primitive_attachments_share_branch_poses_and_preserve_mesh_metadata() {
        let parts=assembly();let (bind,_)=bind_poses(&parts).unwrap();
        let attachments=vec![
            PartAttachment {pose:1,owner:0,rest:Mat4::from_scale_rotation_translation(Vec3::new(0.03,0.7,0.012),
                Quat::from_rotation_z(0.4),Vec3::new(-0.1,0.3,0.06))},
            PartAttachment {pose:1,owner:1,rest:Mat4::from_scale_rotation_translation(Vec3::new(0.045,0.4,0.015),
                Quat::from_rotation_y(-1.2),Vec3::new(0.1,0.5,-0.04))},
            PartAttachment {pose:2,owner:2,rest:Mat4::from_scale_rotation_translation(Vec3::new(0.02,0.6,0.018),
                Quat::from_rotation_x(0.8),Vec3::new(0.03,0.2,-0.02))},
        ];
        let center=Vec3::new(0.05,0.45,-0.03);let radius=0.65;
        // Several instances reference the SAME primitive cluster range.
        let sources:Vec<_>=attachments.iter().map(|a|primitive_source(bind[a.pose as usize]*a.rest,center,radius,41)).collect();
        let packed=pack(&parts,&attachments,&sources,None).unwrap();
        assert_eq!(packed.parts.len(),parts.len());assert_eq!(packed.assemblies.len(),1);
        assert_eq!(packed.attachments.len(),attachments.len());assert_eq!(packed.owners,[0,1,2]);
        for ((d,a),original) in packed.attachments.iter().zip(&attachments).zip(&sources) {
            assert_eq!(d.identity,[a.pose,a.owner,0,0]);assert!(matrix_close(matrix(d.rest),a.rest));
            assert_eq!(d.source.first_cluster,original.first_cluster);assert_eq!(d.source.cluster_count,original.cluster_count);
            assert_eq!(d.source.flags,original.flags);assert_eq!(d.source.rows,original.rows);
            assert!(Vec3::from_slice(&d.local_sphere[..3]).distance(center)<2e-4);
            assert!(d.source.scale>=original.scale);
        }
        let poses=sample_poses(&parts,&WindState {strength:0.8,time_seconds:3.1,..Default::default()}).unwrap();
        assert_eq!(poses.len(),parts.len());
        // Shared branch identity does not collapse the primitive-local offsets
        // or scales. They continue following the same parent-animated pivot.
        let a=poses[1]*attachments[0].rest;let b=poses[1]*attachments[1].rest;
        assert!(a.transform_point3(center).distance(b.transform_point3(center))>0.1);
        assert!((poses[1].inverse()*a).abs_diff_eq(attachments[0].rest,2e-5));
        assert!((poses[1].inverse()*b).abs_diff_eq(attachments[1].rest,2e-5));
    }

    #[test]
    fn primitive_spheres_bound_nonuniform_scale_chains_at_every_sampled_wind_pose() {
        let mut parts=assembly();parts[0].max_angle=0.3;
        let (bind,_)=bind_poses(&parts).unwrap();let center=Vec3::new(0.07,0.51,-0.04);let radius=0.73;
        let attachments=[PartAttachment {pose:2,owner:0,rest:Mat4::from_scale_rotation_translation(
            Vec3::new(0.02,1.7,0.11),Quat::from_euler(glam::EulerRot::XYZ,0.3,0.9,-0.7),Vec3::new(0.08,0.6,-0.1))}];
        let source=primitive_source(bind[2]*attachments[0].rest,center,radius,10);
        let packed=pack(&parts,&attachments,&[source],None).unwrap();let descriptor=&packed.attachments[0];
        for strength in [0.0,0.4,1.0] {for time_seconds in [0.0,0.37,2.1,30.0,119.9] {
            let poses=sample_poses(&parts,&WindState {strength,time_seconds,..Default::default()}).unwrap();
            let current=poses[2]*attachments[0].rest;
            let uploaded_center=current.transform_point3(Vec3::from_slice(&descriptor.local_sphere[..3]));
            let exact_center=current.transform_point3(center);
            assert!(uploaded_center.distance(exact_center)<2e-5);
            for sample in 0..512 {
                let y=1.0-2.0*(sample as f32+0.5)/512.0;let angle=sample as f32*2.3999631;
                let radial=(1.0-y*y).sqrt();let direction=Vec3::new(radial*angle.cos(),y,radial*angle.sin());
                let point=current.transform_point3(center+direction*radius);
                assert!(point.distance(uploaded_center)<=descriptor.local_sphere[3]+2e-5);
                assert!(current.transform_vector3(direction).length()<=descriptor.source.scale+1e-5);
            }
        }}
    }

    #[test]
    fn primitive_attachments_reject_duplicate_owners_invalid_binds_and_pool_overflow() {
        let parts=[PartWind::default()];let source=primitive_source(Mat4::IDENTITY,Vec3::Y*0.5,0.6,0);
        let attachment=PartAttachment {pose:0,owner:0,rest:Mat4::IDENTITY};
        let valid=pack(&parts,std::slice::from_ref(&attachment),&[source],None).unwrap();
        assert_eq!(valid.attachments.len(),1);
        assert!(pack(&parts,&[attachment.clone(),attachment.clone()],&[source],None).is_err());
        assert!(pack(&[PartWind {owner:Some(0),..Default::default()}],std::slice::from_ref(&attachment),&[source],None).is_err());
        for bad in [PartAttachment {pose:1,..attachment.clone()},PartAttachment {owner:1,..attachment.clone()},
            PartAttachment {rest:Mat4::from_scale(Vec3::new(-1.0,1.0,1.0)),..attachment.clone()},
            PartAttachment {rest:Mat4::from_scale(Vec3::new(1.0,0.0,1.0)),..attachment.clone()},
            PartAttachment {rest:Mat4::from_translation(Vec3::X),..attachment.clone()},
            PartAttachment {rest:Mat4::from_cols(Vec4::new(1.0,0.3,0.0,0.0),Vec4::Y,Vec4::Z,Vec4::W),..attachment.clone()}] {
            assert!(pack(&parts,&[bad],&[source],None).is_err());
        }
        for bad in [GpuInstance {flags:0,..source},GpuInstance {scale:2.0,..source},
            GpuInstance {sphere:[0.0,0.0,0.0,-1.0],..source},GpuInstance {scale:f32::NAN,..source}] {
            assert!(pack(&parts,std::slice::from_ref(&attachment),&[bad],None).is_err());
        }
        assert!(pack(&parts,&vec![attachment;MAX_ATTACHMENTS+1],&[source],None).is_err());
        let old=pack(&parts,&[],&[],None).unwrap();assert!(old.attachments.is_empty());assert!(old.owners.is_empty());
    }

    #[test]
    fn zero_cluster_proxy_can_own_volume_while_primitive_instances_share_its_pose() {
        let parts=[PartWind {owner:Some(0),volume:Some(0),max_angle:0.2,..Default::default()}];
        let attachment=PartAttachment {pose:0,owner:1,rest:Mat4::from_scale_rotation_translation(
            Vec3::new(0.02,0.6,0.01),Quat::from_rotation_z(0.2),Vec3::new(0.0,0.3,0.0))};
        let proxy=GpuInstance {rows:rows(Mat4::IDENTITY),sphere:[0.0;4],first_cluster:0,cluster_count:0,scale:1.0,flags:PART_WIND};
        let primitive=primitive_source(attachment.rest,Vec3::Y*0.5,0.6,19);
        let volume=GpuTree {inverse:rows(Mat4::IDENTITY),bounds_min:[-0.05,-0.05,-0.05,0.0],
            bounds_max:[0.45,0.45,0.45,0.0],meta:[0,0,1,u32::MAX]};
        let forest=GpuForest {levels:vec![crate::foliage_volume_data::GpuLevel {origin_cell:[0.0,0.0,0.0,0.1],dims_offset:[4,4,4,0]}],
            indices:Vec::new(),cells:Vec::new(),instances:vec![volume]};
        let packed=pack(&parts,&[attachment],&[proxy,primitive],Some(&forest)).unwrap();
        assert_eq!(packed.owners,[0,1]);assert_eq!(packed.parts[0].source.cluster_count,0);
        assert_eq!(packed.parts[0].identity,[NONE,0,0,0]);assert_eq!(packed.parts[0].volume.meta,volume.meta);
        assert_eq!(packed.attachments[0].identity,[0,1,0,0]);assert_eq!(packed.attachments[0].source.first_cluster,19);
    }
}
