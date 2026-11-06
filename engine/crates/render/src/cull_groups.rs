//! Opt-in two-level spatial instance broadphase, independent of cluster LOD.
//!
//! Owner identities never change. Morton ordering is built once; actual bounds
//! are refitted on the GPU after wind/pose updates, once for all views. Recycled
//! empty slots and moving actors remain in the ordering. Movement can reduce
//! spatial efficiency, never invalidate visibility. Camera, solar and headlight selection
//! independently traverse these bounds against their own frusta. No camera
//! visibility or occlusion result is reused to select shadow casters.
use crate::{scene::GpuInstance, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

pub const OWNERS_PER_LEAF: usize = 64;
pub const LEAVES_PER_PARENT: usize = 8;
pub(crate) const SCENE_BINDINGS: [u32; 4] = [3, 4, 5, 12];

/// Independent switches share only topology/refitted bounds, never visibility.
#[derive(Clone, Copy, Debug)]
struct Enablement { camera: bool, sun: bool, headlight: bool, replacement: bool }
impl Enablement {
    fn allocated(self)->bool { self.camera || self.sun || self.headlight || self.replacement }
    fn bits(self)->u32 { u32::from(self.camera) | (u32::from(self.sun)<<1) | (u32::from(self.headlight)<<2) | (u32::from(self.replacement)<<3) }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Group {
    sphere: [f32; 4],
    /// Leaf: first owner/count. Parent: first leaf/count. Remaining words reserved.
    span: [u32; 4],
}

/// Constructor-time ordering only: all uploaded owner slots occur exactly once.
struct Topology { owners: Vec<u32>, groups: Vec<Group>, leaves: u32, parents: u32 }

fn morton(p: Vec3, lo: Vec3, extent: Vec3) -> u32 {
    let q = ((p-lo)/extent*1023.0).clamp(Vec3::ZERO, Vec3::splat(1023.0)).as_uvec3();
    let mut key=0;
    for bit in 0..10 { for axis in 0..3 { key |= ((q[axis]>>bit)&1) << (3*bit+axis); } }
    key
}
impl Topology {
    fn new(instances: &[GpuInstance]) -> Result<Self,String> {
        let count=u32::try_from(instances.len()).map_err(|_|"spatial owner count exceeds u32")?;
        let center=|i:&GpuInstance| {let p=Vec3::from_slice(&i.sphere[..3]);if p.is_finite(){p}else{Vec3::ZERO}};
        let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
        for inst in instances {let p=center(inst);lo=lo.min(p);hi=hi.max(p);}
        if instances.is_empty(){lo=Vec3::ZERO;hi=Vec3::ONE;}
        // Isotropic quantization preserves world-space proximity. Normalizing
        // each axis separately would mix kilometre-separated X regions when
        // a scene happens to have only a few metres of Y/Z extent.
        let extent=Vec3::splat((hi-lo).max_element().max(1e-6));
        let mut keyed:Vec<_>=instances.iter().enumerate().map(|(id,inst)|(morton(center(inst),lo,extent),id as u32)).collect();
        keyed.sort_unstable(); // owner ID breaks Morton ties deterministically
        let owners=keyed.into_iter().map(|(_,id)|id).collect::<Vec<_>>();
        let leaves=count.div_ceil(OWNERS_PER_LEAF as u32);
        let parents=leaves.div_ceil(LEAVES_PER_PARENT as u32);
        let mut groups=Vec::with_capacity((leaves+parents)as usize);
        for i in 0..leaves {let first=i*OWNERS_PER_LEAF as u32;
            groups.push(Group {sphere:[0.,0.,0.,-1.],span:[first,(count-first).min(OWNERS_PER_LEAF as u32),0,0]});}
        for i in 0..parents {let first=i*LEAVES_PER_PARENT as u32;
            groups.push(Group {sphere:[0.,0.,0.,-1.],span:[first,(leaves-first).min(LEAVES_PER_PARENT as u32),0,0]});}
        Ok(Self {owners,groups,leaves,parents})
    }
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub enabled: bool,
    pub total_parent_groups: u32,
    pub total_leaf_groups: u32,
    pub owner_slots: u32,
    pub shared_bytes: u64,
    pub owner_scratch_bytes_per_view: u64,
    /// Actual GPU work for this view/light, accumulated over any camera phases
    /// (not unique groups). Solar views are selected independently.
    pub parent_groups_tested: u32,
    pub parent_groups_rejected: u32,
    pub leaf_groups_tested: u32,
    pub leaf_groups_rejected: u32,
    pub owner_tests: u32,
    pub compacted_owners: u32,
    pub resident_clusters_visited: u32,
    pub overflow_fallbacks: u32,
    /// Incomplete traversal was detected and replaced by the flat owner scan.
    pub traversal_mismatch_fallbacks: u32,
}

fn validate_counters(words:&[u32],owner_slots:u32,parents:u32)->Result<(),String> {
    if words.len()<32{return Err("spatial instance counters missing".into());}
    if words[21]>words[20] || words[23]>words[22] || words[25]>words[24] {
        return Err("inconsistent spatial instance counters".into());
    }
    // Every retained leaf visits all members, including empty/policy-rejected
    // owners. Only the final leaf may be partial, at most once per phase.
    let retained=u64::from(words[22]-words[23]);
    let phases=if parents==0{0}else{u64::from(words[20].div_ceil(parents))};
    let padding=(OWNERS_PER_LEAF as u64-u64::from(owner_slots)%OWNERS_PER_LEAF as u64)%OWNERS_PER_LEAF as u64;
    let maximum=retained*OWNERS_PER_LEAF as u64;
    let minimum=maximum.saturating_sub(phases*padding);
    if words[31]==0 && !(minimum..=maximum).contains(&u64::from(words[24])) {
        return Err(format!("incomplete spatial owner traversal: {} visits, expected {minimum}..={maximum}",words[24]));
    }
    Ok(())
}

pub(crate) struct Resources {
    /// Camera broadphase only. Sun-only allocation must not enable camera work.
    pub enabled: bool,
    pub sun_enabled: bool,
    pub headlight_enabled: bool,
    pub replacement_enabled: bool,
    pub groups: wgpu::Buffer,
    pub owners: wgpu::Buffer,
    count: u32,
    leaves: u32,
    pub parents: u32,
    replacement_scratch_bytes: u64,
    refit: Option<(wgpu::ComputePipeline,wgpu::ComputePipeline,wgpu::BindGroup)>,
}
impl Resources {
    pub fn new(gpu:&Gpu, instances:&[GpuInstance], instance_buffer:&wgpu::Buffer, wind:&wgpu::Buffer,
        replacement:Option<&crate::source_replacement::Plan>)->Result<Self,String> {
        let enablement=Enablement {
            camera:std::env::var("KERN_INSTANCE_GROUPS").as_deref()==Ok("1"),
            sun:std::env::var("KERN_SUN_INSTANCE_GROUPS").as_deref()==Ok("1"),
            headlight:std::env::var("KERN_HEADLIGHT_INSTANCE_GROUPS").as_deref()==Ok("1"),
            replacement:replacement.is_some_and(|plan|plan.group_count>0),
        };
        let enabled=enablement.camera;
        let topology=Topology::new(if enablement.allocated() {instances}else{&[]})?;
        let count=topology.owners.len()as u32;
        let mut bytes=bytemuck::cast_slice(&[topology.leaves,topology.parents,count,enablement.bits()]).to_vec();
        // A dummy element keeps the runtime-array binding valid when disabled.
        let empty_group=[Group::zeroed()];
        bytes.extend_from_slice(bytemuck::cast_slice(if topology.groups.is_empty(){&empty_group}else{&topology.groups}));
        let mut owner_words=topology.owners.clone();
        let replacement_scratch_bytes=if enablement.replacement {
            let plan=replacement.unwrap();plan.append_members(&mut owner_words)?;plan.scratch_bytes()
        }else{0};
        let owner_data=if owner_words.is_empty(){&[0u32][..]}else{&owner_words};
        let limit=u64::from(gpu.limits.max_storage_buffer_binding_size);
        if bytes.len()as u64>limit || std::mem::size_of_val(owner_data)as u64>limit || replacement_scratch_bytes>limit {
            return Err("spatial instance groups exceed storage buffer limit".into());
        }
        let groups=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("spatial instance group bounds"),contents:&bytes,
            usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC});
        let owners=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("spatial stable owner ordering"),contents:bytemuck::cast_slice(owner_data),
            usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC});
        let refit=if enablement.allocated() {
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("spatial group bounds refit"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/cull_groups.wgsl").into())});
            // Explicit common layout: parent refit consumes fewer inputs but
            // shares the leaf bind group without auto-layout incompatibility.
            let entries=(0..4).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,
                ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:binding!=3},has_dynamic_offset:false,min_binding_size:None},count:None}).collect::<Vec<_>>();
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("spatial refit"),entries:&entries});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("spatial refit"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let pipeline=|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("spatial refit"),layout:&layout,entries:&[
                wgpu::BindGroupEntry {binding:0,resource:instance_buffer.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:wind.as_entire_binding()},
                wgpu::BindGroupEntry {binding:2,resource:owners.as_entire_binding()},wgpu::BindGroupEntry {binding:3,resource:groups.as_entire_binding()}]});
            Some((pipeline("refit_leaves"),pipeline("refit_parents"),bind))
        }else{None};
        if enablement.allocated() {eprintln!("Spatial instance groups: {count} owner slots, {} leaves, {} parents; current GPU bounds, camera={enabled}, sun={}, headlights={}",topology.leaves,topology.parents,enablement.sun,enablement.headlight);}
        Ok(Self {enabled,sun_enabled:enablement.sun,headlight_enabled:enablement.headlight,replacement_enabled:enablement.replacement,
            groups,owners,count,leaves:topology.leaves,parents:topology.parents,replacement_scratch_bytes,refit})
    }
    pub fn allocated(&self)->bool { self.enabled || self.sun_enabled || self.headlight_enabled || self.replacement_enabled }
    /// Camera compact owner prefix plus optional replacement decisions.
    pub fn scratch_bytes(&self)->u64 {self.owner_scratch_bytes().max(self.replacement_scratch_bytes)}
    /// Shadows never consume or allocate camera replacement decisions.
    pub fn owner_scratch_bytes(&self)->u64 {u64::from(self.count.max(1))*4}
    pub fn encode(&self,encoder:&mut wgpu::CommandEncoder,time:Option<(&wgpu::QuerySet,u32)>)->bool {
        let Some((leaf,parent,bind))=&self.refit else{return false;};
        if self.leaves==0{return false;}
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("instance group refit"),timestamp_writes:time.map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
        pass.set_bind_group(0,bind,&[]);pass.set_pipeline(leaf);
        pass.dispatch_workgroups(self.leaves.min(65535),self.leaves.div_ceil(65535),1);
        pass.set_pipeline(parent);let n=self.parents.div_ceil(64);
        pass.dispatch_workgroups(n.min(65535),n.div_ceil(65535),1);true
    }
    pub fn diagnostics(&self,words:&[u32])->Result<Diagnostics,String> {
        let mut result=self.diagnostics_for(words,self.enabled)?;
        // A camera can retain the replacement decision tail even when its
        // instance broadphase is off. Shadow diagnostics have only N owners.
        if self.enabled || self.replacement_enabled {result.owner_scratch_bytes_per_view=self.scratch_bytes();}
        Ok(result)
    }
    /// Counters from one actual solar view, never the camera's retained list.
    pub fn sun_diagnostics(&self,words:&[u32])->Result<Diagnostics,String> {
        self.diagnostics_for(words,self.sun_enabled)
    }
    pub(crate) fn diagnostics_for(&self,words:&[u32],enabled:bool)->Result<Diagnostics,String> {
        validate_counters(words,self.count,self.parents)?;
        Ok(Diagnostics {enabled,total_parent_groups:self.parents,total_leaf_groups:self.leaves,owner_slots:self.count,
            shared_bytes:self.groups.size()+self.owners.size(),owner_scratch_bytes_per_view:if enabled{self.owner_scratch_bytes()}else{0},
            parent_groups_tested:words[20],parent_groups_rejected:words[21],leaf_groups_tested:words[22],leaf_groups_rejected:words[23],
            owner_tests:words[24],compacted_owners:words[25],resident_clusters_visited:words[26],overflow_fallbacks:words[27],
            traversal_mismatch_fallbacks:words[31]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn instance(p:Vec3)->GpuInstance {GpuInstance {sphere:p.extend(1.0).to_array(),cluster_count:1,..GpuInstance::zeroed()}}
    #[test]
    fn camera_sun_and_headlights_share_allocation_without_sharing_visibility() {
        for camera in [false,true] {for sun in [false,true] {for headlight in [false,true] {for replacement in [false,true] {
            let flags=Enablement {camera,sun,headlight,replacement};
            assert_eq!(flags.allocated(),camera||sun||headlight||replacement);
            assert_eq!(flags.bits()&1!=0,camera);
            assert_eq!(flags.bits()&2!=0,sun);
            assert_eq!(flags.bits()&4!=0,headlight);
            assert_eq!(flags.bits()&crate::source_replacement::SPATIAL_FLAG!=0,replacement);
        }}}}
    }
    #[test]
    fn perspective_headlight_groups_preserve_offscreen_wind_and_plane_crossing_casters() {
        use crate::headlights::{SpotLight,SHADOW_NEAR};
        use glam::{Mat4,Quat,Vec4};
        fn planes(vp:Mat4)->[Vec4;6] {
            let m=vp.transpose();
            [m.w_axis+m.x_axis,m.w_axis-m.x_axis,m.w_axis+m.y_axis,
                m.w_axis-m.y_axis,m.z_axis,m.w_axis-m.z_axis]
                .map(|p|p/p.truncate().length())
        }
        fn visible(s:Vec4,planes:&[Vec4;6])->bool {
            s.w>=0. && planes.iter().all(|p|p.truncate().dot(s.truncate())+p.w>=-s.w)
        }
        fn fit(spheres:impl Iterator<Item=Vec4>)->Vec4 {
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for s in spheres {lo=lo.min(s.truncate()-Vec3::splat(s.w));hi=hi.max(s.truncate()+Vec3::splat(s.w));}
            if !lo.is_finite(){return Vec4::new(0.,0.,0.,-1.);}
            let center=(lo+hi)*0.5;let radius=((hi-lo)*0.5).length();
            center.extend(radius+(center.abs().max_element()+radius)*1e-5+0.001)
        }
        let initial=(0..2051).map(|i|instance(Vec3::new(
            (i%41)as f32*17.-340.,(i%13)as f32*4.-20.,(i/41)as f32*13.-320.
        ))).collect::<Vec<_>>();
        let topology=Topology::new(&initial).unwrap();
        for (position,rotation,range) in [
            (Vec3::ZERO,Quat::IDENTITY,110.),
            (Vec3::new(70.,35.,-50.),Quat::from_euler(glam::EulerRot::YXZ,1.7,0.8,0.3),110.),
            (Vec3::new(-120.,-10.,140.),Quat::from_euler(glam::EulerRot::YXZ,-2.4,-0.7,-0.5),200.),
            (Vec3::new(200.,-20.,250.),Quat::from_euler(glam::EulerRot::YXZ,0.4,1.1,0.2),5.),
        ] {
            let lamp=SpotLight {position,direction:rotation*Vec3::NEG_Z,up:rotation*Vec3::Y,
                intensity_cd:Vec3::splat(40_000.),range_m:range,
                inner_angle_radians:17f32.to_radians(),outer_angle_radians:32f32.to_radians()}.validated().unwrap();
            let mut source=initial.clone();let mut wind=vec![0.;source.len()];
            let d=lamp.direction;let right=d.cross(lamp.up).normalize();let middle=range*0.5;
            // These owners moved after the immutable Morton order was built.
            source[0]=instance(position+d*middle);
            source[1]=instance(position+d*(SHADOW_NEAR*0.5));source[1].sphere[3]=0.04;
            source[2]=instance(position+d*(range+0.5)); // far-plane crossing sphere
            source[3]=instance(position+d*(range+3.)); // wholly beyond the finite range
            source[4]=instance(position+d*middle+right*(middle*lamp.outer_angle_radians.tan()+0.8));
            source[4].sphere[3]=0.1;wind[4]=1.; // current wind crosses the side plane
            source[5]=source[0];source[5].cluster_count=0;
            source[6]=source[0];source[6].flags|=4; // NO_SHADOW remains excluded
            source[1001]=instance(position+d*(range*0.7));
            let sphere=|id:u32| {let mut s=Vec4::from_array(source[id as usize].sphere);s.w+=wind[id as usize];s};
            let leaves=topology.groups[..topology.leaves as usize].iter().map(|leaf| {
                fit(topology.owners[leaf.span[0]as usize..(leaf.span[0]+leaf.span[1])as usize].iter().copied()
                    .filter(|&id|source[id as usize].cluster_count!=0).map(sphere))
            }).collect::<Vec<_>>();
            let parents=topology.groups[topology.leaves as usize..].iter().map(|p| {
                fit(leaves[p.span[0]as usize..(p.span[0]+p.span[1])as usize].iter().copied().filter(|s|s.w>=0.))
            }).collect::<Vec<_>>();
            let light_planes=planes(lamp.shadow_matrix());
            let owner=|id:u32|source[id as usize].cluster_count!=0&&(source[id as usize].flags&4)==0&&visible(sphere(id),&light_planes);
            let mut flat=(0..source.len()as u32).filter(|&id|owner(id)).collect::<Vec<_>>();
            let mut grouped=Vec::new();let mut rejected_groups=0;
            for (index,parent) in topology.groups[topology.leaves as usize..].iter().enumerate() {
                if !visible(parents[index],&light_planes){rejected_groups+=1;continue;}
                for leaf_id in parent.span[0]..parent.span[0]+parent.span[1] {
                    if !visible(leaves[leaf_id as usize],&light_planes){rejected_groups+=1;continue;}
                    let leaf=topology.groups[leaf_id as usize];
                    grouped.extend(topology.owners[leaf.span[0]as usize..(leaf.span[0]+leaf.span[1])as usize].iter().copied().filter(|&id|owner(id)));
                }
            }
            flat.sort_unstable();grouped.sort_unstable();assert_eq!(grouped,flat);
            assert!(rejected_groups>0,"test must exercise spatial work rejection");
            for id in [0,1,2,4,1001] {assert!(grouped.contains(&id),"current/plane-crossing caster {id} missing");}
            for id in [3,5,6] {assert!(!grouped.contains(&id),"reference policy must reject {id}");}
            assert!(!visible(Vec4::from_array(source[4].sphere),&light_planes),"wind padding must be necessary");
            let camera=SpotLight {direction:-d,..lamp};
            assert!(!visible(sphere(0),&planes(camera.shadow_matrix())),"off-camera blocker must remain a light caster");
        }
    }
    #[test]
    fn light_space_groups_match_flat_current_pose_selection_for_four_independent_views() {
        use glam::{Mat3,Quat,Vec4};
        let mut source=(0..2051).map(|i|instance(Vec3::new(
            (i%41)as f32*17.-340., (i%13)as f32*4.-20., (i/41)as f32*13.-320.
        ))).collect::<Vec<_>>();
        let topology=Topology::new(&source).unwrap();
        // Recycle an empty slot, move actors across the original Morton cells,
        // and retain the current wind sweep. Constructor positions are not a
        // valid rejection certificate after these changes.
        source[0]=instance(Vec3::new(120.,0.,0.)); // outside camera, casts into light volume
        source[63]=instance(Vec3::new(-5.,30.,5.));
        source[1001]=instance(Vec3::new(12.,-10.,-8.));
        source[42].cluster_count=0;
        source[73].flags|=4; // NO_SHADOW
        let wind=(0..source.len()).map(|id|(id%11)as f32*0.37).collect::<Vec<_>>();
        let fit=|spheres:&[Vec4]| {
            if spheres.is_empty(){return Vec4::new(0.,0.,0.,-1.);}
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for s in spheres {lo=lo.min(s.truncate()-Vec3::splat(s.w));hi=hi.max(s.truncate()+Vec3::splat(s.w));}
            let center=(lo+hi)*0.5;let radius=((hi-lo)*0.5).length();
            center.extend(radius+(center.abs().max_element()+radius)*1e-5+0.001)
        };
        let sphere=|id:usize| {let mut s=Vec4::from_array(source[id].sphere);s.w+=wind[id];s};
        let leaves=topology.groups[..topology.leaves as usize].iter().map(|leaf| {
            let members=topology.owners[leaf.span[0]as usize..(leaf.span[0]+leaf.span[1])as usize].iter()
                .filter(|&&id|source[id as usize].cluster_count!=0).map(|&id|sphere(id as usize)).collect::<Vec<_>>();
            fit(&members)
        }).collect::<Vec<_>>();
        let parents=topology.groups[topology.leaves as usize..].iter().map(|p| {
            fit(&leaves[p.span[0]as usize..(p.span[0]+p.span[1])as usize].iter().copied().filter(|s|s.w>=0.).collect::<Vec<_>>())
        }).collect::<Vec<_>>();
        for (center,rotation,extent) in [
            (Vec3::ZERO,Quat::IDENTITY,Vec3::new(150.,80.,200.)),
            (Vec3::new(70.,35.,-50.),Quat::from_rotation_x(0.8)*Quat::from_rotation_y(1.7),Vec3::new(35.,50.,400.)),
            (Vec3::new(-120.,0.,140.),Quat::from_rotation_z(0.3)*Quat::from_rotation_x(-1.1),Vec3::new(65.,40.,450.)),
            (Vec3::new(200.,-20.,250.),Quat::from_rotation_y(-2.4),Vec3::new(25.,25.,500.)),
        ] {
            let basis=Mat3::from_quat(rotation);
            let planes=(0..3).flat_map(|axis|[-1.,1.].map(|sign| {
                let n=basis.col(axis)*sign;n.extend(extent[axis]-n.dot(center))
            })).collect::<Vec<_>>();
            let visible=|s:Vec4|s.w>=0.&&planes.iter().all(|p|p.truncate().dot(s.truncate())+p.w>=-s.w);
            let owner=|id:u32|source[id as usize].cluster_count!=0&&(source[id as usize].flags&4)==0&&visible(sphere(id as usize));
            let mut flat=(0..source.len()as u32).filter(|&id|owner(id)).collect::<Vec<_>>();
            let mut selected=Vec::new();
            for (parent_index,parent) in topology.groups[topology.leaves as usize..].iter().enumerate() {
                if !visible(parents[parent_index]){continue;}
                for leaf_id in parent.span[0]..parent.span[0]+parent.span[1] {
                    if !visible(leaves[leaf_id as usize]){continue;}
                    let leaf=topology.groups[leaf_id as usize];
                    selected.extend(topology.owners[leaf.span[0]as usize..(leaf.span[0]+leaf.span[1])as usize].iter().copied().filter(|&id|owner(id)));
                }
            }
            flat.sort_unstable();selected.sort_unstable();assert_eq!(selected,flat);
            let mut unique=selected.clone();unique.dedup();assert_eq!(selected,unique);
            assert!(selected.len()<source.len(),"fixture must exercise rejection");
            if rotation==Quat::IDENTITY {
                assert!(selected.contains(&0),"off-camera light blocker must survive");
                assert!(source[0].sphere[0]-source[0].sphere[3]>20.,"outside narrow camera frustum");
                assert!(selected.contains(&63)&&selected.contains(&1001),"moved current poses must survive");
            }
        }
    }
    #[test]
    fn topology_preserves_all_owner_ids_including_empty_and_nonfinite_slots() {
        let mut input=(0..1501).map(|i|instance(Vec3::new((i%37)as f32,0.,(i/37)as f32))).collect::<Vec<_>>();
        input[9].cluster_count=0;input[99].sphere[0]=f32::NAN;
        let a=Topology::new(&input).unwrap();let b=Topology::new(&input).unwrap();assert_eq!(a.owners,b.owners);
        let mut ids=a.owners.clone();ids.sort_unstable();assert_eq!(ids,(0..input.len()as u32).collect::<Vec<_>>());
        let from_leaves=a.groups[..a.leaves as usize].iter().flat_map(|g|a.owners[g.span[0]as usize..(g.span[0]+g.span[1])as usize].iter().copied()).collect::<Vec<_>>();
        assert_eq!(from_leaves,a.owners);
        let leaf_ids=a.groups[a.leaves as usize..].iter().flat_map(|g|g.span[0]..g.span[0]+g.span[1]).collect::<Vec<_>>();
        assert_eq!(leaf_ids,(0..a.leaves).collect::<Vec<_>>());
        assert_eq!(std::mem::size_of::<Group>(),32);
    }
    #[test]
    fn spatial_order_localizes_interleaved_world_regions_without_category_assumptions() {
        let input=(0..2048).map(|i|instance(Vec3::new(if i%2==0{-1000.}else{1000.},(i%16)as f32,(i%31)as f32))).collect::<Vec<_>>();
        let t=Topology::new(&input).unwrap();
        for parent in &t.groups[t.leaves as usize..] {
            let mut signs=Vec::new();for leaf in parent.span[0]..parent.span[0]+parent.span[1] {let g=t.groups[leaf as usize];
                signs.extend(t.owners[g.span[0]as usize..(g.span[0]+g.span[1])as usize].iter().map(|&id|input[id as usize].sphere[0].signum()as i32));}
            assert!(signs.iter().all(|s|*s==signs[0]),"distant interleaved owners must become separate groups");
        }
    }
    #[test]
    fn retained_leaf_visits_cover_each_owner_once_including_partial_leaves() {
        for count in [1,63,64,65,511,512,513,1501] {
            let input=(0..count).map(|i|instance(Vec3::new(i as f32,0.,0.))).collect::<Vec<_>>();
            let topology=Topology::new(&input).unwrap();
            for pattern in 0..8u32 {
                let mut expected=Vec::new();let mut visited=Vec::new();
                for parent in &topology.groups[topology.leaves as usize..] {
                    for leaf_id in parent.span[0]..parent.span[0]+parent.span[1] {
                        if pattern!=0 && leaf_id%8!=pattern-1 {continue;}
                        let leaf=topology.groups[leaf_id as usize];
                        expected.extend_from_slice(&topology.owners[leaf.span[0]as usize..(leaf.span[0]+leaf.span[1])as usize]);
                        for lane in 0..64 {if lane<leaf.span[1] {
                            visited.push(topology.owners[(leaf.span[0]+lane)as usize]);
                        }}
                    }
                }
                assert_eq!(visited,expected);
                let mut unique=visited.clone();unique.sort_unstable();unique.dedup();assert_eq!(unique.len(),visited.len());
            }
        }
    }
    #[test]
    fn diagnostics_reject_recorded_partial_traversal_and_allow_explicit_fallback() {
        // Actual failing r45 native-a near-view counters: 1,412 retained
        // leaves but only 11,648 owner visits, rather than at least 90,306.
        let mut words=[0u32;32];words[20]=1258;words[21]=986;
        words[22]=2176;words[23]=764;words[24]=11648;words[25]=4881;
        assert!(validate_counters(&words,321825,629).unwrap_err().contains("incomplete"));
        words[31]=1;words[27]=1;assert!(validate_counters(&words,321825,629).is_ok());
        // A one-owner last leaf can be retained twice without requiring all
        // 64 lanes to have a valid owner; all other leaf visits remain exact.
        let mut tail=[0u32;32];tail[20]=2;tail[22]=4;tail[24]=130;
        assert!(validate_counters(&tail,65,1).is_ok());
        tail[24]=129;assert!(validate_counters(&tail,65,1).is_err());
    }
    #[test]
    fn refitted_group_spheres_enclose_moved_recycled_and_wind_swept_members() {
        // Independent geometric oracle over changing current spheres. Sample
        // every bounding-box corner, not just sphere centres, so an omitted
        // wind pad or stale constructor position cannot pass this contract.
        let mut input=(0..777).map(|i|instance(Vec3::new((i%37)as f32,0.,(i/37)as f32))).collect::<Vec<_>>();
        input[14].cluster_count=0;
        let topology=Topology::new(&input).unwrap();
        input[14]=instance(Vec3::new(-300.,70.,100.));
        input[99].sphere=[500.,-20.,-150.,12.];
        let fitted=|spheres:&[[f32;4]]| {
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for s in spheres {let c=Vec3::from_slice(&s[..3]);lo=lo.min(c-Vec3::splat(s[3]));hi=hi.max(c+Vec3::splat(s[3]));}
            let c=(lo+hi)*0.5;let r=((hi-lo)*0.5).length();
            c.extend(r+(c.abs().max_element()+r)*1e-5+0.001).to_array()
        };
        let mut leaves=Vec::new();
        for group in &topology.groups[..topology.leaves as usize] {
            let members=topology.owners[group.span[0]as usize..(group.span[0]+group.span[1])as usize].iter().map(|&id|{
                let mut sphere=input[id as usize].sphere;sphere[3]+=(id%9)as f32*0.31;sphere
            }).collect::<Vec<_>>();
            let sphere=fitted(&members);let center=Vec3::from_slice(&sphere[..3]);
            for member in members {for corner in 0..8 {
                let p=Vec3::from_slice(&member[..3])+Vec3::new(if corner&1==0{-1.}else{1.},if corner&2==0{-1.}else{1.},if corner&4==0{-1.}else{1.})*member[3];
                assert!(center.distance(p)<=sphere[3]);
            }}
            leaves.push(sphere);
        }
        for group in &topology.groups[topology.leaves as usize..] {
            let members=&leaves[group.span[0]as usize..(group.span[0]+group.span[1])as usize];let sphere=fitted(members);
            let center=Vec3::from_slice(&sphere[..3]);
            for member in members {assert!(center.distance(Vec3::from_slice(&member[..3]))+member[3]<=sphere[3]);}
            // Two opposing views: a group outside a plane must imply every
            // member is outside. Near-plane-intersecting groups stay eligible.
            for sign in [-1.,1.] {let n=Vec3::X*sign;for offset in [-600.,-1.,0.,50.,600.] {
                if n.dot(center)+offset < -sphere[3] {
                    assert!(members.iter().all(|m|n.dot(Vec3::from_slice(&m[..3]))+offset < -m[3]));
                }
            }}
        }
    }
    #[test]
    fn shaders_validate_and_cull_scene_layout_fits_portable_storage_budget() {
        for source in [include_str!("shaders/cull_groups.wgsl").to_owned(),crate::frame::cull_source()] {
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let scene_buffers=module.global_variables.iter().filter(|(_,g)|g.binding.as_ref().is_some_and(|b|b.group==0)&&matches!(g.space,naga::AddressSpace::Storage {..})).count();
            let view_buffers=module.global_variables.iter().filter(|(_,g)|g.binding.as_ref().is_some_and(|b|b.group==1)&&matches!(g.space,naga::AddressSpace::Storage {..})).count();
            assert!(scene_buffers+view_buffers<=16);
        }
        assert_eq!(SCENE_BINDINGS,[3,4,5,12]);
    }
    #[test]
    fn shadow_group_predicate_has_no_camera_depth_or_fog_dependencies() {
        fn calls(block:&naga::Block,out:&mut Vec<naga::Handle<naga::Function>>) {
            for statement in block {match statement {
                naga::Statement::Call {function,..}=>out.push(*function),
                naga::Statement::Block(body)=>calls(body,out),
                naga::Statement::If {accept,reject,..}=>{calls(accept,out);calls(reject,out);},
                naga::Statement::Switch {cases,..}=>{for case in cases {calls(&case.body,out);}},
                naga::Statement::Loop {body,continuing,..}=>{calls(body,out);calls(continuing,out);},
                _=>{},
            }}
        }
        let source=crate::frame::cull_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap();
        for name in ["sun_spatial_select","headlight_spatial_select"] {
            let entry=module.entry_points.iter().find(|entry|entry.name==name).unwrap();
            // Both entrypoints explicitly select the pure light-space branch,
            // never the camera branch that can use fog or a camera Hi-Z map.
            let mut entry_calls=Vec::new();calls(&entry.function.body,&mut entry_calls);
            assert_eq!(entry_calls.len(),1);
            assert_eq!(module.functions[entry_calls[0]].name.as_deref(),Some("select_spatial_parent"));
            let naga::Statement::Call {arguments,..}=entry.function.body.iter().find(|s|matches!(s,naga::Statement::Call {..})).unwrap() else{unreachable!()};
            assert!(matches!(entry.function.expressions[arguments[2]],naga::Expression::Literal(naga::Literal::Bool(true))));
        }
        let (root,_)=module.functions.iter().find(|(_,function)|function.name.as_deref()==Some("sun_group_visible")).unwrap();
        let mut pending=vec![root];let mut visited=Vec::new();
        while let Some(handle)=pending.pop() {
            if visited.contains(&handle){continue;}visited.push(handle);
            let function=&module.functions[handle];
            assert!(!matches!(function.name.as_deref(),Some("hidden_by_camera_fog"|"tree_occluded")),"shadow groups cannot borrow camera rejection");
            calls(&function.body,&mut pending);
        }
        assert!(visited.iter().any(|&handle|module.functions[handle].name.as_deref()==Some("in_frustum")));
    }
}
