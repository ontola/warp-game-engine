//! Bounded, shared baked rigid-part animation. Not a skinned-character system.
//!
//! Clips contain model-space poses, baked once; actors share these samples.
//! One compute pass updates ordinary opaque instances before every view and
//! shadow pass. Existing rigid-instance motion reconstruction sees those same
//! matrices, then snapshots them AFTER all views. No per-camera animation.

use crate::{Gpu, scene::GpuInstance};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use std::{collections::HashSet, sync::Mutex};
use wgpu::util::DeviceExt;

pub const MAX_ACTORS: usize = 384;
pub const MAX_PARTS: usize = 8192;
pub const MAX_POSES: usize = 65536;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct BonePose {
    pub rotation: [f32; 4],
    /// Model-space position; w reserved, zero.
    pub translation: [f32; 4],
}
impl BonePose {
    pub fn new(rotation: Quat, translation: Vec3) -> Self {
        Self { rotation: rotation.to_array(), translation: translation.extend(0.0).to_array() }
    }
    pub fn matrix(self) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_array(self.rotation), Vec3::from_slice(&self.translation))
    }
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub bones: u32,
    /// Periodic samples in frame-major order, WITHOUT a duplicate last frame.
    pub frames: u32,
    pub duration: f32,
    pub poses: Vec<BonePose>,
}

#[derive(Clone, Debug)]
pub struct Actor {
    /// Uniform positive scale and rigid orientation; no sheared normal basis.
    pub root: Mat4,
    pub clip: u32,
    /// Cycle fraction, not seconds. Stable seeded variation belongs here.
    pub phase: f32,
    pub rate: f32,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub actor: u32,
    pub bone: u32,
    pub owner: u32,
    /// Actual mesh-local sphere, never the actor's rest-pose world bound.
    pub local_sphere: [f32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct BakedAnimations {
    pub clips: Vec<Clip>,
    pub actors: Vec<Actor>,
    pub parts: Vec<Part>,
}

fn root_scale(root: Mat4) -> Result<f32, String> {
    if !root.is_finite() || !root.row(3).abs_diff_eq(glam::Vec4::W, 1e-6) {
        return Err("baked animation root must be finite affine".into());
    }
    let axes = [root.x_axis.truncate(), root.y_axis.truncate(), root.z_axis.truncate()];
    let s = axes[0].length();
    if !(0.01..=100.0).contains(&s) || axes.iter().any(|a| (a.length()-s).abs()>s*1e-4)
        || axes[0].dot(axes[1]).abs()>s*s*1e-4 || axes[1].dot(axes[2]).abs()>s*s*1e-4
        || axes[0].dot(axes[2]).abs()>s*s*1e-4 || axes[0].cross(axes[1]).dot(axes[2])<s*s*s*0.999 {
        return Err("baked animation root needs a proper rotation and uniform positive scale".into());
    }
    Ok(s)
}

impl BakedAnimations {
    pub fn validate(&self, instances: &[GpuInstance]) -> Result<(), String> {
        if self.clips.is_empty() || self.clips.len()>16 || self.actors.is_empty() || self.actors.len()>MAX_ACTORS
            || self.parts.is_empty() || self.parts.len()>MAX_PARTS {
            return Err("baked animation clip/actor/part count outside bounded capacity".into());
        }
        let mut count=0usize;
        for clip in &self.clips {
            if !(1..=64).contains(&clip.bones) || !(2..=240).contains(&clip.frames)
                || !clip.duration.is_finite() || !(0.1..=120.0).contains(&clip.duration)
                || clip.poses.len()!=clip.bones as usize*clip.frames as usize {
                return Err(format!("invalid baked clip {}",clip.name));
            }
            count+=clip.poses.len();
            for pose in &clip.poses {
                let q=Quat::from_array(pose.rotation); let p=Vec3::from_slice(&pose.translation);
                if !q.is_finite() || (q.length_squared()-1.0).abs()>1e-4 || !p.is_finite()
                    || p.abs().max_element()>100.0 || pose.translation[3]!=0.0 {
                    return Err("invalid baked rigid pose".into());
                }
            }
        }
        if count>MAX_POSES {return Err("baked pose atlas exceeds capacity".into());}
        for actor in &self.actors {
            root_scale(actor.root)?;
            if actor.clip as usize>=self.clips.len() || !actor.phase.is_finite() || !(0.0..1.0).contains(&actor.phase)
                || !actor.rate.is_finite() || !(0.25..=2.0).contains(&actor.rate) {return Err("invalid baked actor phase/clip/rate".into());}
        }
        let mut owners=HashSet::new();
        for part in &self.parts {
            let actor=self.actors.get(part.actor as usize).ok_or("baked part actor out of range")?;
            let source=instances.get(part.owner as usize).ok_or("baked part owner out of range")?;
            if part.bone>=self.clips[actor.clip as usize].bones || !owners.insert(part.owner)
                || !part.local_sphere.iter().all(|v|v.is_finite()) || part.local_sphere[3]<=0.0
                || source.cluster_count==0 || source.flags & (crate::scene::FOLIAGE | crate::scene::WIND_GRASS | crate::scene::PART_WIND | crate::scene::LIVE_GRASS)!=0 {
                return Err("baked part has invalid/duplicate owner, sphere, or another deformation source".into());
            }
        }
        Ok(())
    }

    /// CPU reference and initial scene pose. Same shortest-arc normalized
    /// quaternion interpolation as the GPU; animated transforms are rigid.
    pub fn sample(&self, actor: usize, bone: u32, seconds: f32) -> Mat4 {
        let a=&self.actors[actor]; let clip=&self.clips[a.clip as usize];
        let position=(seconds*a.rate/clip.duration+a.phase).rem_euclid(1.0)*clip.frames as f32;
        let f=position.floor() as u32; let t=position-position.floor();
        let p=clip.poses[(f*clip.bones+bone) as usize];
        let n=clip.poses[(((f+1)%clip.frames)*clip.bones+bone) as usize];
        let q=Quat::from_array(p.rotation); let mut r=Quat::from_array(n.rotation);
        if q.dot(r)<0.0 {r=-r;}
        let rotation=(q*(1.0-t)+r*t).normalize();
        let translation=Vec3::from_slice(&p.translation).lerp(Vec3::from_slice(&n.translation),t);
        a.root*Mat4::from_rotation_translation(rotation,translation)
    }

    pub fn pose_bytes(&self)->usize {self.clips.iter().map(|c|c.poses.len()).sum::<usize>()*std::mem::size_of::<BonePose>()}
}

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuActor {root:[[f32;4];3],animation:[f32;4],clip:[u32;4]}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuPart {identity:[u32;4],sphere:[f32;4]}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {time:[f32;4],counts:[u32;4]}

pub(crate) struct AnimationRenderer {
    pipeline:wgpu::ComputePipeline, bind:wgpu::BindGroup, uniform:wgpu::Buffer,
    owners:Vec<u32>, time:Mutex<f32>,
}
impl AnimationRenderer {
    pub fn new(gpu:&Gpu,animation:&BakedAnimations,instances:&[GpuInstance],buffer:&wgpu::Buffer)->Result<Self,String> {
        animation.validate(instances)?;
        let mut poses=Vec::new();let mut offsets=Vec::new();
        for clip in &animation.clips {offsets.push(poses.len() as u32);poses.extend_from_slice(&clip.poses);}
        let actors:Vec<_>=animation.actors.iter().map(|a|{
            let c=&animation.clips[a.clip as usize];
            GpuActor {root:[a.root.row(0).to_array(),a.root.row(1).to_array(),a.root.row(2).to_array()],
                animation:[a.phase,a.rate,c.duration,root_scale(a.root).unwrap()],
                clip:[offsets[a.clip as usize],c.frames,c.bones,0]}
        }).collect();
        let parts:Vec<_>=animation.parts.iter().map(|p|GpuPart {identity:[p.actor,p.bone,p.owner,0],sphere:p.local_sphere}).collect();
        let upload=|name,bytes:&[u8],usage|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some(name),contents:bytes,usage});
        for bytes in [bytemuck::cast_slice::<_,u8>(&poses),bytemuck::cast_slice::<_,u8>(&actors),bytemuck::cast_slice::<_,u8>(&parts)] {
            if bytes.len() as u64>gpu.limits.max_storage_buffer_binding_size as u64 {return Err("baked animation exceeds GPU storage capacity".into());}
        }
        let atlas=upload("shared baked rigid poses",bytemuck::cast_slice(&poses),wgpu::BufferUsages::STORAGE);
        let actors=upload("baked animation actors",bytemuck::cast_slice(&actors),wgpu::BufferUsages::STORAGE);
        let parts=upload("baked animation parts",bytemuck::cast_slice(&parts),wgpu::BufferUsages::STORAGE);
        let uniform=upload("baked animation clock",bytemuck::bytes_of(&Uniform::zeroed()),wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("baked rigid animation"),source:wgpu::ShaderSource::Wgsl(SHADER.into())});
        let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("baked rigid animation"),layout:None,module:&module,
            entry_point:Some("animate_parts"),compilation_options:Default::default(),cache: gpu.pipeline_cache()});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("baked rigid animation"),layout:&pipeline.get_bind_group_layout(0),entries:&[
            wgpu::BindGroupEntry {binding:0,resource:uniform.as_entire_binding()},
            wgpu::BindGroupEntry {binding:1,resource:atlas.as_entire_binding()},
            wgpu::BindGroupEntry {binding:2,resource:actors.as_entire_binding()},
            wgpu::BindGroupEntry {binding:3,resource:parts.as_entire_binding()},
            wgpu::BindGroupEntry {binding:4,resource:buffer.as_entire_binding()}]});
        Ok(Self {pipeline,bind,uniform,owners:animation.parts.iter().map(|p|p.owner).collect(),time:Mutex::new(0.0)})
    }
    pub fn set_time(&self,seconds:f64)->Result<bool,String> {
        if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {return Err("baked animation clock must be finite, 0..86400 seconds".into());}
        let mut time=self.time.lock().unwrap();let changed=*time!=seconds as f32;*time=seconds as f32;Ok(changed)
    }
    pub fn contains_owner_range(&self,first:usize,count:usize)->bool {
        self.owners.iter().any(|owner|(*owner as usize)>=first&&(*owner as usize)<first.saturating_add(count))
    }
    pub fn encode(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,timestamp:Option<(&wgpu::QuerySet,u32)>) {
        let uniform=Uniform {time:[*self.time.lock().unwrap(),0.0,0.0,0.0],counts:[self.owners.len() as u32,0,0,0]};
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&uniform));
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("shared baked animation"),timestamp_writes:timestamp.map(|(query_set,start)|wgpu::ComputePassTimestampWrites {
            query_set,beginning_of_pass_write_index:Some(start),end_of_pass_write_index:Some(start+1)})});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);pass.dispatch_workgroups((self.owners.len() as u32).div_ceil(64),1,1);
    }
}

pub const SHADER:&str=include_str!("shaders/baked_animation.wgsl");

#[cfg(test)]
mod tests {
    use super::*;
    fn rig()->(BakedAnimations,Vec<GpuInstance>) {
        let animation=BakedAnimations {clips:vec![Clip {name:"loop".into(),bones:1,frames:4,duration:2.0,poses:(0..4).map(|i|{
            let a=i as f32*std::f32::consts::FRAC_PI_2;
            BonePose::new(Quat::from_rotation_z(a.sin()*0.6),Vec3::new(a.cos()*0.2,1.0,0.0))}).collect()}],
            actors:vec![Actor {root:Mat4::from_scale_rotation_translation(Vec3::splat(1.2),Quat::from_rotation_y(0.7),Vec3::new(5.0,2.0,-7.0)),clip:0,phase:0.3,rate:1.0}],
            parts:vec![Part {actor:0,bone:0,owner:0,local_sphere:[0.0,0.5,0.0,0.8]}]};
        let source=GpuInstance {rows:[[1.0,0.0,0.0,0.0],[0.0,1.0,0.0,0.0],[0.0,0.0,1.0,0.0]],sphere:[0.0;4],first_cluster:0,cluster_count:1,scale:1.0,flags:0};
        (animation,vec![source])
    }
    #[test]
    fn baked_animation_wgsl_and_layout_validate() {
        let module=naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e|panic!("{}",e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<BonePose>(),32);assert_eq!(std::mem::size_of::<GpuActor>(),80);
        assert_eq!(std::mem::size_of::<GpuPart>(),32);assert_eq!(std::mem::size_of::<GpuInstance>(),80);
    }
    #[test]
    fn interpolation_wrap_bounds_and_rigid_motion_correspondence() {
        let (a,instances)=rig();a.validate(&instances).unwrap();
        let local=Vec3::new(0.2,0.9,0.1);let center=Vec3::from_slice(&a.parts[0].local_sphere);let radius=a.parts[0].local_sphere[3]*1.2;
        for tick in 0..240 {
            let t=tick as f32/120.0;let now=a.sample(0,0,t);let old=a.sample(0,0,(t-1.0/120.0).max(0.0));
            assert!(now.abs_diff_eq(a.sample(0,0,t+2.0),3e-6));
            assert!(now.transform_point3(local).distance(now.transform_point3(center))<radius);
            // Existing motion shader's inverse-current / previous-owner path.
            let recovered=now.inverse().transform_point3(now.transform_point3(local));
            assert!(old.transform_point3(recovered).distance(old.transform_point3(local))<3e-6);
            for _view in 0..4 {assert_eq!(a.sample(0,0,t),now);}
        }
        assert!(a.sample(0,0,0.0).abs_diff_eq(a.sample(0,0,2.0-1e-5),3e-5));
    }
    #[test]
    fn bad_atlas_owners_and_double_deformation_are_rejected() {
        let (a,mut sources)=rig();let mut b=a.clone();b.parts.push(b.parts[0].clone());assert!(b.validate(&sources).is_err());
        sources[0].flags=crate::scene::FOLIAGE;assert!(a.validate(&sources).is_err());sources[0].flags=0;
        let mut b=a.clone();b.clips[0].poses[0].rotation=[0.0;4];assert!(b.validate(&sources).is_err());
        let mut b=a.clone();b.actors[0].root*=Mat4::from_scale(Vec3::new(1.0,2.0,1.0));assert!(b.validate(&sources).is_err());
        let mut b=a.clone();b.actors.resize(MAX_ACTORS+1,b.actors[0].clone());assert!(b.validate(&sources).is_err());
    }
}
