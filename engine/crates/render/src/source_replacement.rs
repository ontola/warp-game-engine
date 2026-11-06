//! Scalable, per-view whole-owner replacement decisions.
//!
//! Source IDs, meshes and wind remain untouched. The source and replacement
//! renderer must consume the same published GPU decision table. Readiness is
//! a provider contract (complete coverage/pose/capacity), not inferred from a
//! small voxel footprint. Missing readiness always retains source geometry.
use crate::{scene::{Scene,BLEND},Gpu};
use bytemuck::{Pod,Zeroable};
use glam::{Mat4,Vec3,Vec4};
use std::collections::BTreeSet;
use wgpu::util::DeviceExt;

pub const MAGIC:u32=0x53525031;
pub const VERSION:u32=1;
pub const HEADER_WORDS:u32=8;
pub const GROUP_WORDS:u32=16;
pub const SPATIAL_FLAG:u32=8;
pub const NO_GROUP:u32=u32::MAX;
pub const MAX_GROUPS:usize=65_536;
pub const MAX_MEMBERS:usize=1_048_576;

#[derive(Clone,Debug)]
pub struct Group {
    pub key:u64,
    /// Geometry/provider identity. Increment when the source is rebuilt.
    pub generation:u32,
    /// Original complete source owners; unique within and across groups.
    pub members:Vec<u32>,
    /// Immutable world envelope including source, reconstruction and every
    /// supported wind pose. A changed envelope requires rebuilding the plan.
    pub bounds:[[f32;3];2],
    /// Conservative world spatial reconstruction error, not optical error.
    pub error_radius:f32,
}

/// A producer may publish readiness only after all required coverage, pose,
/// storage and traversal budgets are guaranteed. Do not publish before a
/// fallible per-view binning/preflight step. Zero fields retain the source.
#[repr(C)]
#[derive(Clone,Copy,Debug,Default,Pod,Zeroable)]
pub struct Readiness {
    pub generation:u32,
    pub ready:u32,
    pub valid:u32,
    /// Zero for immutable providers; otherwise the current View generation.
    pub frame_generation:u32,
}

#[derive(Clone,Debug)]
pub struct Plan {
    pub owner_count:u32,
    pub group_count:u32,
    pub member_count:u32,
    /// Absolute offsets into binding9, after the N Morton owner IDs.
    pub owner_map_offset:u32,
    pub descriptor_offset:u32,
    pub members_offset:u32,
    words:Vec<u32>,
}
impl Plan {
    pub fn new(scene:&Scene,groups:&[Group])->Result<Self,String> {
        if groups.len()>MAX_GROUPS {return Err("source replacement group budget exceeded".into());}
        let owner_count=u32::try_from(scene.instances.len()).map_err(|_|"source owner count exceeds u32")?;
        let group_count=groups.len()as u32;
        let member_count=groups.iter().try_fold(0usize,|n,g|n.checked_add(g.members.len())).ok_or("source member count overflow")?;
        if member_count>MAX_MEMBERS {return Err("source replacement member budget exceeded".into());}
        let owner_map_offset=owner_count.checked_add(HEADER_WORDS).ok_or("source mapping overflow")?;
        let descriptor_offset=owner_map_offset.checked_add(owner_count).ok_or("source mapping overflow")?;
        let members_offset=descriptor_offset.checked_add(group_count*GROUP_WORDS).ok_or("source group overflow")?;
        let end=members_offset.checked_add(member_count as u32).ok_or("source members overflow")?;
        let mut words=vec![0;(end-owner_count)as usize];
        words[..8].copy_from_slice(&[MAGIC,VERSION,group_count,owner_count,owner_map_offset,descriptor_offset,members_offset,end]);
        words[HEADER_WORDS as usize..(descriptor_offset-owner_count)as usize].fill(NO_GROUP);
        let mut keys=BTreeSet::new();let mut checked_meshes=BTreeSet::new();let mut next_member=members_offset;
        for (group_id,group) in groups.iter().enumerate() {
            let [lo,hi]=group.bounds.map(Vec3::from_array);
            if !keys.insert(group.key)||group.generation==0||group.members.is_empty()
                ||!lo.is_finite()||!hi.is_finite()||lo.abs().max_element()>=1e18||hi.abs().max_element()>=1e18
                ||lo.cmpgt(hi).any()||!group.error_radius.is_finite()||group.error_radius<=0. {
                return Err("invalid source replacement identity, bounds or error".into());
            }
            let center=(lo+hi)*0.5;let radius=((hi-lo)*0.5).length();
            if !radius.is_finite() {return Err("source replacement envelope overflows".into());}
            for &owner in &group.members {
                let inst=scene.instances.get(owner as usize).ok_or("source replacement owner out of range")?;
                let map_index=(owner_map_offset-owner_count+owner)as usize;
                if words[map_index]!=NO_GROUP||inst.cluster_count==0 {return Err("source replacement members must be unique nonempty owners".into());}
                let sphere=Vec4::from_array(inst.sphere);let pad=(center.abs().max_element()+radius+1.)*(32.*f32::EPSILON);
                if !sphere.is_finite()||sphere.w<0.||
                    (sphere.truncate()-Vec3::splat(sphere.w)).cmplt(lo-Vec3::splat(pad)).any()||
                    (sphere.truncate()+Vec3::splat(sphere.w)).cmpgt(hi+Vec3::splat(pad)).any() {
                    return Err("source replacement envelope omits a source owner sphere".into());
                }
                let first=inst.first_cluster as usize;let count=inst.cluster_count as usize;
                let end=first.checked_add(count).ok_or("source cluster range overflow")?;
                let clusters=scene.clusters.get(first..end).ok_or("source cluster range out of bounds")?;
                if checked_meshes.insert((first,count)) {
                    for cluster in clusters {
                        let material=scene.materials.get(cluster.material as usize).ok_or("source replacement material missing")?;
                        if material.flags&BLEND!=0 {return Err("source replacement must not omit forward blended materials".into());}
                    }
                }
                words[map_index]=group_id as u32;words[(next_member-owner_count)as usize]=owner;next_member+=1;
            }
            let start=(descriptor_offset-owner_count+group_id as u32*GROUP_WORDS)as usize;
            let descriptor=&mut words[start..start+GROUP_WORDS as usize];
            descriptor[..4].copy_from_slice(&[group.key as u32,(group.key>>32)as u32,group.generation,group.members.len()as u32]);
            descriptor[4..8].copy_from_slice(&center.extend(radius).to_array().map(f32::to_bits));
            descriptor[8]=group.error_radius.to_bits();descriptor[9]=next_member-group.members.len()as u32;
        }
        Ok(Self {owner_count,group_count,member_count:member_count as u32,owner_map_offset,descriptor_offset,members_offset,words})
    }
    /// Append after exactly N Morton owner IDs; existing topology indices stay valid.
    pub(crate) fn append_members(&self,words:&mut Vec<u32>)->Result<(),String> {
        if words.len()!=self.owner_count as usize {return Err("source plan/spatial owner count mismatch".into());}
        words.extend_from_slice(&self.words);Ok(())
    }
    pub fn packed_words(&self)->u32 {self.owner_count+self.words.len()as u32}
    pub fn view_header_word(&self)->u32 {self.owner_count}
    pub fn decision_word(&self)->u32 {self.owner_count+HEADER_WORDS}
    pub fn scratch_bytes(&self)->u64 {u64::from(self.owner_count+HEADER_WORDS+self.group_count)*4}
    pub fn readiness_bytes(&self)->u64 {u64::from(self.group_count.max(1))*16}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cluster::GpuCluster,scene::{GpuInstance,Material,FOLIAGE}};

    fn source(count:usize)->(Scene,Vec<Group>) {
        let mut scene=Scene::default();
        scene.materials.push(Material::zeroed());scene.clusters.push(GpuCluster::zeroed());
        let mut groups=Vec::new();
        for id in 0..count {
            let p=Vec3::new(id as f32*4.,0.,-100.);
            scene.instances.push(GpuInstance {sphere:p.extend(1.).to_array(),cluster_count:1,flags:FOLIAGE,scale:1.,..GpuInstance::zeroed()});
            groups.push(Group {key:0x100000000+id as u64,generation:7,members:vec![id as u32],
                bounds:[(p-Vec3::splat(2.)).to_array(),(p+Vec3::splat(2.)).to_array()],error_radius:0.02});
        }
        (scene,groups)
    }
    fn packed(plan:&Plan)->Vec<u32> {
        let mut words=(0..plan.owner_count).rev().collect::<Vec<_>>();
        plan.append_members(&mut words).unwrap();words
    }
    // Reference of the published ABI, separate from any culling policy. These
    // malformed/stale cases must never become geometry omission.
    fn selected(plan:&Plan,words:&[u32],view:&[u32],owner:u32)->bool {
        let n=plan.owner_count as usize;
        if owner>=plan.owner_count||words.len()<n+8||view.len()<n+8{return false;}
        let h=&words[n..n+8];let v=&view[n..n+8];
        if h[0]!=MAGIC||h[1]!=VERSION||h[3]!=plan.owner_count{return false;}
        if v[0]!=MAGIC||v[1]==0||v[1]!=v[2]||v[3]!=h[2]||v[4]!=1||v[5]!=plan.owner_count{return false;}
        let Some(&g)=words.get(h[4]as usize+owner as usize)else{return false;};
        g<h[2]&&view.get(n+8+g as usize)==Some(&1)
    }
    fn published(plan:&Plan,generation:u32)->Vec<u32> {
        let n=plan.owner_count as usize;let mut out=vec![0;n+8+plan.group_count as usize];
        out[n..n+8].copy_from_slice(&[MAGIC,generation,generation,plan.group_count,1,plan.owner_count,0,0]);
        out[n+8..].fill(1);out
    }
    #[test]
    fn scalable_packing_keeps_original_owner_ids_and_wind_flags() {
        let (scene,mut groups)=source(1025);groups.reverse();
        let before=bytemuck::cast_slice::<_,u8>(&scene.instances).to_vec();
        let plan=Plan::new(&scene,&groups).unwrap();let words=packed(&plan);
        assert_eq!(plan.group_count,1025);assert_eq!(words.len(),plan.packed_words()as usize);
        assert_eq!(&words[..1025],&(0..1025).rev().collect::<Vec<_>>());
        assert_eq!(words[plan.owner_map_offset as usize],1024);
        assert_eq!(words[plan.owner_map_offset as usize+1024],0);
        for (index,group) in groups.iter().enumerate() {
            let d=plan.descriptor_offset as usize+index*GROUP_WORDS as usize;
            assert_eq!(u64::from(words[d])|(u64::from(words[d+1])<<32),group.key);
            assert_eq!(words[words[d+9]as usize],group.members[0]);
        }
        assert_eq!(before,bytemuck::cast_slice::<_,u8>(&scene.instances));
        assert_eq!(plan.scratch_bytes(),(1025+8+1025)*4);
        assert!(plan.append_members(&mut vec![0;1024]).is_err());
    }
    #[test]
    fn duplicate_members_missing_support_and_blended_materials_are_rejected() {
        let (mut scene,mut groups)=source(2);
        groups[1].members[0]=0;assert!(Plan::new(&scene,&groups).is_err());
        groups[1].members[0]=1;groups[0].bounds[1][0]=0.;assert!(Plan::new(&scene,&groups).is_err());
        groups[0].bounds[1][0]=2.;groups[0].generation=0;assert!(Plan::new(&scene,&groups).is_err());
        groups[0].generation=7;groups[0].error_radius=f32::NAN;assert!(Plan::new(&scene,&groups).is_err());
        groups[0].error_radius=0.02;scene.materials[0].flags=BLEND;assert!(Plan::new(&scene,&groups).is_err());
        scene.materials[0].flags=crate::scene::ALPHA_TEST;assert!(Plan::new(&scene,&groups).is_ok());
    }
    #[test]
    fn view_tail_never_counts_as_compaction_capacity_and_invalid_publication_keeps_source() {
        let (scene,groups)=source(3);let plan=Plan::new(&scene,&groups[..2]).unwrap();
        let words=packed(&plan);let view=published(&plan,19);
        assert!(selected(&plan,&words,&view,0));assert!(selected(&plan,&words,&view,1));
        assert!(!selected(&plan,&words,&view,2));assert!(!selected(&plan,&words,&view,3));
        for (offset,value) in [(0,0),(1,0),(2,18),(3,1),(4,0),(5,2)] {
            let mut invalid=view.clone();invalid[plan.owner_count as usize+offset]=value;
            assert!(!selected(&plan,&words,&invalid,0));
        }
        assert!(!selected(&plan,&words,&view[..plan.owner_count as usize],0));
        // Compaction writes exactly [0,N); neither decisions nor publication
        // can be clobbered, including the two-phase camera append path.
        let mut overwritten=view.clone();overwritten[..plan.owner_count as usize].fill(u32::MAX);
        assert_eq!(&view[3..],&overwritten[3..]);assert!(selected(&plan,&words,&overwritten,0));
        let cull=include_str!("shaders/cull.wgsl");
        assert!(!cull.contains("arrayLength(&spatial_owners)"));
        assert!(cull.contains("if slot<spatial_header().z {spatial_owners[slot]=id;}"));
        assert!(cull.contains("view.params.z<0.5 && view.extra.y<1.5 && (spatial_header().w&8u)!=0u"));
    }
    fn ppm(v:View,center:Vec3,radius:f32)->f32 {
        let m=v.view_projection.transpose();let p=center.extend(1.);let w=m.w_axis;let z=m.z_axis;
        let wlen=w.truncate().length();let wmin=w.dot(p)-radius*wlen;
        if !(wmin>1e-6&&z.dot(p)-radius*z.truncate().length()>0.&&(w-z).dot(p)-radius*(w-z).truncate().length()>0.) {return 1e30;}
        let axis=|row:Vec4,size:u32| {let len=row.truncate().length();
            (len/wmin+(row.dot(p).abs()+radius*len)*wlen/(wmin*wmin))*size as f32*0.5};
        glam::Vec2::new(axis(m.x_axis,v.output_size[0]),axis(m.y_axis,v.output_size[1])).length()
    }
    fn view()->View {View {view_projection:Mat4::perspective_rh(60f32.to_radians(),16./9.,0.1,5000.),
        output_size:[1920,1080],max_error_pixels:1.,generation:19,enabled:true,native_supported:true,optical_opt_in:true}}
    #[test]
    fn paired_views_use_output_error_and_near_plane_stays_source() {
        let near=view();let mut far=view();far.view_projection*=Mat4::from_translation(Vec3::new(0.,0.,-1000.));
        let p=Vec3::new(0.,0.,-10.);let radius=2.;let error=0.02;
        assert!(ppm(near,p,radius)*error>1.);assert!(ppm(far,p,radius)*error<1.);
        let mut four_k=far;four_k.output_size=[3840,2160];
        assert!((ppm(four_k,p,radius)/ppm(far,p,radius)-2.).abs()<1e-5);
        assert_eq!(ppm(near,Vec3::new(0.,0.,-0.2),1.),1e30);
        let (scene,groups)=source(1);let plan=Plan::new(&scene,&groups).unwrap();
        for (enabled,native,optics) in [(false,true,true),(true,false,true),(true,true,false)] {
            let mut invalid=near;invalid.enabled=enabled;invalid.native_supported=native;invalid.optical_opt_in=optics;
            assert_eq!(invalid.uniform(&plan).unwrap().info[3],0);
        }
        let mut bad=near;bad.generation=0;assert!(bad.uniform(&plan).is_err());
    }
    #[test]
    fn projected_error_bound_covers_finite_point_displacements_off_axis() {
        let view=view();let center=Vec3::new(35.,10.,-70.);let radius=12.;let bound=ppm(view,center,radius);
        let project=|p:Vec3| {let q=view.view_projection*p.extend(1.);q.truncate().truncate()/q.w*glam::Vec2::new(960.,540.)};
        for i in 0..64 {
            let angle=i as f32*0.37;let p=center+Vec3::new(angle.sin(),(angle*1.7).sin(),angle.cos())*4.;
            for d in [Vec3::X,Vec3::Y,Vec3::Z,Vec3::new(1.,2.,3.).normalize()] {
                let delta=d*0.05;
                assert!(project(p+delta).distance(project(p))<=bound*delta.length());
            }
        }
    }
    #[test]
    fn shaders_validate_packed_abi_without_new_cull_bindings() {
        assert_eq!(std::mem::size_of::<Readiness>(),16);assert_eq!(std::mem::size_of::<Uniform>(),96);
        let selector=include_str!("shaders/source_replacement.wgsl");
        assert!(selector.contains("ready.x!=packed[base+2u]"));
        assert!(selector.contains("ready.w!=0u && ready.w!=selection_view.info.z"));
        for source in [selector.to_owned(),crate::frame::cull_source()] {
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let storage=module.global_variables.iter().filter(|(_,g)|matches!(g.space,naga::AddressSpace::Storage {..})).count();
            assert!(storage<=16,"replacement must reuse packed bindings9/10");
        }
    }
}

#[derive(Clone,Copy,Debug)]
pub struct View {
    pub view_projection:Mat4,
    pub output_size:[u32;2],
    pub max_error_pixels:f32,
    pub generation:u32,
    pub enabled:bool,
    pub native_supported:bool,
    /// Experimental optics must be explicitly accepted by the caller.
    pub optical_opt_in:bool,
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {vp:[f32;16],output:[f32;4],info:[u32;4]}
impl View {
    fn uniform(self,plan:&Plan)->Result<Uniform,String> {
        if !self.view_projection.is_finite()||self.output_size.contains(&0)||self.generation==0
            ||!self.max_error_pixels.is_finite()||self.max_error_pixels<=0. {
            return Err("invalid source replacement view/error/generation".into());
        }
        Ok(Uniform {vp:self.view_projection.to_cols_array(),output:[self.output_size[0]as f32,self.output_size[1]as f32,self.max_error_pixels,0.],
            info:[plan.owner_count,plan.group_count,self.generation,u32::from(self.enabled&&self.native_supported&&self.optical_opt_in)]})
    }
}

pub(crate) fn query_source()->&'static str {include_str!("shaders/source_replacement_query.wgsl")}
pub struct Selector {plan:Plan,layout:wgpu::BindGroupLayout,pipelines:[wgpu::ComputePipeline;2]}
impl Selector {
    pub fn new(gpu:&Gpu,plan:&Plan)->Result<Self,String> {
        crate::shaders::checked(gpu,|| {
            let entries=(0..4).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,
                ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding!=2}},
                    has_dynamic_offset:false,min_binding_size:None},count:None}).collect::<Vec<_>>();
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("source replacement selector"),entries:&entries});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("source replacement selector"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("source replacement selector"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/source_replacement.wgsl").into())});
            let pipelines=["select_groups","publish_selection"].map(|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&shader,
                entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()}));
            Self {plan:plan.clone(),layout,pipelines}
        })
    }
    pub fn plan(&self)->&Plan {&self.plan}
    /// Scratch needs STORAGE|COPY_DST; only the decision header is cleared.
    /// Encode once per camera before either tree phase. Neither phase may
    /// overwrite the tail; shadow passes never consume its selection.
    pub fn encode(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,members:&wgpu::Buffer,scratch:&wgpu::Buffer,
        readiness:&wgpu::Buffer,view:View,time:Option<(&wgpu::QuerySet,u32)>)->Result<(),String> {
        let uniform=view.uniform(&self.plan)?;
        if members.size()<u64::from(self.plan.packed_words())*4||scratch.size()<self.plan.scratch_bytes()||readiness.size()<self.plan.readiness_bytes() {
            return Err("source replacement buffers do not cover the declared plan".into());
        }
        let config=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("source replacement view"),contents:bytemuck::bytes_of(&uniform),usage:wgpu::BufferUsages::UNIFORM});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("source replacement view"),layout:&self.layout,entries:&[
            wgpu::BindGroupEntry {binding:0,resource:config.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:members.as_entire_binding()},
            wgpu::BindGroupEntry {binding:2,resource:scratch.as_entire_binding()},wgpu::BindGroupEntry {binding:3,resource:readiness.as_entire_binding()}]});
        encoder.clear_buffer(scratch,u64::from(self.plan.view_header_word())*4,Some(u64::from(HEADER_WORDS)*4));
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("source replacement selection"),timestamp_writes:time.map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
        pass.set_bind_group(0,&bind,&[]);
        if self.plan.group_count>0 {pass.set_pipeline(&self.pipelines[0]);pass.dispatch_workgroups(self.plan.group_count.div_ceil(64),1,1);}
        pass.set_pipeline(&self.pipelines[1]);pass.dispatch_workgroups(1,1,1);Ok(())
    }
}
