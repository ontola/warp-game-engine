//! Bounded shared rigid-part animation for source geometry and derived volumes.
//!
//! This is an opt-in assembly prototype, not a whole-forest skeletal system.
//! Each small assembly is evaluated parent-first by one GPU invocation. Source
//! instance transforms, volume inverse rays and bounds use exactly that pose.
//! Previous matrices are retained, but existing voxel/DLSS rejection remains:
//! retaining a previous matrix alone is not a correct fractional-volume MV.

use crate::{Gpu, foliage_volume_data::{GpuForest, GpuTree}, scene::{GpuInstance, PART_WIND}, wind::WindState};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3, Vec4};
use std::{collections::HashSet, sync::Mutex};
use wgpu::util::DeviceExt;

pub const MAX_PARTS: usize = 256;
pub const MAX_ASSEMBLY_PARTS: usize = 64;
const PERIOD: f64 = 120.0;
const NONE: u32 = u32::MAX;

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
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniform { field:[f32;4], counts:[u32;4] }
struct Packed {parts:Vec<Descriptor>,assemblies:Vec<[u32;4]>,owners:Vec<usize>}
struct State {field:[f32;4],initialized:bool}

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
    if parts.is_empty() || parts.len()>MAX_PARTS {return Err(format!("part wind requires 1..={MAX_PARTS} branches"));}
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

fn pack(parts:&[PartWind],instances:&[GpuInstance],forest:Option<&GpuForest>)->Result<Packed,String> {
    let (bind,assemblies)=bind_poses(parts)?;let mut descriptors=Vec::with_capacity(parts.len());
    let mut owners=HashSet::new();let mut volumes=HashSet::new();
    let mut root_scale=1.0f32;
    for (index,part) in parts.iter().enumerate() {
        if part.parent.is_none() {
            root_scale=[part.rest.x_axis,part.rest.y_axis,part.rest.z_axis].iter()
                .map(|v|v.truncate().length()).fold(0.0,f32::max);
        }
        let mut d=Descriptor::zeroed();d.rest=rows(part.rest);
        d.animation=[part.max_angle,part.harmonic as f32,part.phase.rem_euclid(std::f32::consts::TAU),0.0];
        d.identity=[part.parent.unwrap_or(NONE),part.owner.unwrap_or(NONE),part.volume.unwrap_or(NONE),0];
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
    let mut owners:Vec<_>=owners.into_iter().collect();owners.sort_unstable();
    Ok(Packed {parts:descriptors,assemblies,owners})
}

fn field(state:&WindState)->Result<[f32;4],String> {
    let state=state.validated()?;
    if state.strength==0.0 {return Ok([0.0;4]);}
    Ok([state.direction.x,state.direction.y,state.strength,
        (state.time_seconds.rem_euclid(PERIOD)*std::f64::consts::TAU/PERIOD) as f32])
}

/// Independent CPU pose reference for fixtures; production updates stay on the
/// GPU. Uses quaternion composition instead of the shader's Rodrigues rows.
pub fn sample_poses(parts:&[PartWind],state:&WindState)->Result<Vec<Mat4>,String> {
    bind_poses(parts)?;let field=field(state)?;let mut result:Vec<Mat4>=Vec::with_capacity(parts.len());
    let axis=Vec3::new(-field[1],0.0,field[0]).try_normalize().unwrap_or(Vec3::X);
    for part in parts {
        let base=part.parent.map(|p|result[p as usize]).unwrap_or(Mat4::IDENTITY)*part.rest;
        let angle=part.max_angle*field[2]*(field[3]*part.harmonic as f32+part.phase.rem_euclid(std::f32::consts::TAU)).sin();
        let rotation=Quat::from_axis_angle(axis,angle);let pivot=base.w_axis.truncate();
        result.push(Mat4::from_translation(pivot)*Mat4::from_quat(rotation)*Mat4::from_translation(-pivot)*base);
    }
    Ok(result)
}

pub struct PartWindRenderer {
    pipeline:wgpu::ComputePipeline,bind:wgpu::BindGroup,uniform:wgpu::Buffer,poses:wgpu::Buffer,
    assembly_count:u32,part_count:u32,owners:Vec<usize>,state:Mutex<State>,
}
impl PartWindRenderer {
    pub fn new(gpu:&Gpu,parts:&[PartWind],scene_instances:&[GpuInstance],instance_buffer:&wgpu::Buffer,
        forest:Option<(&GpuForest,&wgpu::Buffer)>)->Result<Self,String> {
        let packed=pack(parts,scene_instances,forest.map(|(f,_)|f))?;
        if instance_buffer.size()<(scene_instances.len()*std::mem::size_of::<GpuInstance>()) as u64
            || !instance_buffer.usage().contains(wgpu::BufferUsages::STORAGE) {return Err("part wind requires the writable live source instance storage buffer".into());}
        if let Some((forest,buffer))=forest {
            if buffer.size()<(forest.instances.len()*std::mem::size_of::<GpuTree>()) as u64
                || !buffer.usage().contains(wgpu::BufferUsages::STORAGE) {return Err("part wind requires the live volume instance storage buffer".into());}
        }
        let upload=|label,bytes:&[u8],usage|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some(label),contents:bytes,usage});
        let descriptors=upload("part wind bind poses",bytemuck::cast_slice(&packed.parts),wgpu::BufferUsages::STORAGE);
        let assemblies=upload("part wind assemblies",bytemuck::cast_slice(&packed.assemblies),wgpu::BufferUsages::STORAGE);
        let uniform=upload("part wind state",bytemuck::bytes_of(&Uniform::zeroed()),wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
        let poses=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("part wind current previous poses"),
            size:(parts.len()*std::mem::size_of::<GpuPartPose>()) as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let dummy;
        let volume_buffer=if let Some((_,buffer))=forest {buffer} else {
            dummy=upload("empty part wind volumes",bytemuck::bytes_of(&GpuTree::zeroed()),wgpu::BufferUsages::STORAGE);&dummy
        };
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("rigid assembly part wind"),source:wgpu::ShaderSource::Wgsl(SHADER.into())});
        let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("rigid assembly part wind"),layout:None,
            module:&module,entry_point:Some("update_parts"),compilation_options:Default::default(),cache:None});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("rigid assembly part wind"),layout:&pipeline.get_bind_group_layout(0),
            entries:&[wgpu::BindGroupEntry {binding:0,resource:uniform.as_entire_binding()},
                wgpu::BindGroupEntry {binding:1,resource:descriptors.as_entire_binding()},
                wgpu::BindGroupEntry {binding:2,resource:assemblies.as_entire_binding()},
                wgpu::BindGroupEntry {binding:3,resource:poses.as_entire_binding()},
                wgpu::BindGroupEntry {binding:4,resource:instance_buffer.as_entire_binding()},
                wgpu::BindGroupEntry {binding:5,resource:volume_buffer.as_entire_binding()}]});
        Ok(Self {pipeline,bind,uniform,poses,assembly_count:packed.assemblies.len() as u32,part_count:parts.len() as u32,
            owners:packed.owners,state:Mutex::new(State {field:[0.0;4],initialized:false})})
    }

    pub fn poses(&self)->&wgpu::Buffer {&self.poses}

    pub fn contains_owner_range(&self,first:usize,count:usize)->bool {
        let end=first.saturating_add(count);self.owners.iter().any(|owner|*owner>=first&&*owner<end)
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
        let mut state=self.state.lock().unwrap();
        let uniform=Uniform {field:state.field,counts:[self.assembly_count,self.part_count,u32::from(!state.initialized),0]};
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&uniform));state.initialized=true;
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("rigid assembly part wind"),
            timestamp_writes:timestamp.map(|(query_set,index)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(index),end_of_pass_write_index:Some(index+1)})});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);pass.dispatch_workgroups(self.assembly_count.div_ceil(32),1,1);true
    }
}

pub const SHADER:&str=include_str!("shaders/part_wind.wgsl");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rigid_part_compute_shader_validates() {
        let module=naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e|panic!("{}",e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(SHADER)));
    }
    fn assembly()->Vec<PartWind> {
        vec![PartWind {rest:Mat4::from_scale_rotation_translation(Vec3::new(1.2,0.8,1.4),Quat::from_rotation_y(0.3),Vec3::new(12.0,2.0,-8.0)),..Default::default()},
            PartWind {parent:Some(0),rest:Mat4::from_translation(Vec3::new(0.0,2.0,0.0)),max_angle:0.15,phase:0.7,..Default::default()},
            PartWind {parent:Some(1),rest:Mat4::from_rotation_translation(Quat::from_rotation_z(0.2),Vec3::new(0.0,1.0,0.0)),max_angle:0.08,harmonic:17,phase:1.1,..Default::default()}]
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
        assert!(pack(&[p.clone(),p],&[source],None).is_err());
        assert_eq!(std::mem::size_of::<GpuPartPose>(),96);
        assert_eq!(std::mem::size_of::<Descriptor>(),304);
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
        let packed=pack(&[root,child],&[source],None).unwrap();
        assert_eq!(packed.parts[1].source.scale,2.0);
        assert!((packed.parts[1].local_sphere[3]-2.0).abs()<1e-6);
        let direction=Vec3::new(std::f32::consts::FRAC_1_SQRT_2,-std::f32::consts::FRAC_1_SQRT_2,0.0);
        assert!(world.transform_vector3(direction).length()<=packed.parts[1].local_sphere[3]+1e-6);
    }
}
