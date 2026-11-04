//! Frame-wide GPU forest residency. CPU supplies recipes and permanent roots
//! once; every consumer's demand is combined on GPU before assigning branches.
use crate::{Gpu,Scene,foliage_select::{Node,Instance,Selector,NONE},foliage_residency::Residency,
    scene::{GpuInstance,PART_WIND},foliage_volume_data::GpuTree};
use glam::{Mat4,Vec3,Vec4};
use bytemuck::{Pod,Zeroable};
use wgpu::util::DeviceExt;

/// World placement independent of any camera, renderer tier or CPU near pool.
#[derive(Clone, Copy, Debug)]
pub struct RootPlacement {pub pos:Vec3,pub yaw:f32,pub scale:f32,pub variant:usize}

#[repr(C)]
#[derive(Clone,Copy,Debug,Pod,Zeroable)]
pub struct Branch {
    rest:[[f32;4];3], source:GpuInstance, volume:GpuTree,
    low:[f32;4], high:[f32;4], animation:[f32;4], info:[u32;4],
}
#[derive(Clone,Debug)]
pub struct Spec {
    pub nodes:Vec<Node>,pub roots:Vec<Instance>,branches:Vec<Branch>,representations:Vec<[u32;4]>,
    proxies:Vec<GpuTree>,pub first_owner:u32,pub slots:u32,pub error:f32,
}
fn rows(m:Mat4)->[[f32;4];3] {[m.row(0).to_array(),m.row(1).to_array(),m.row(2).to_array()]}
fn matrix(r:[[f32;4];3])->Mat4 {Mat4::from_cols(
    Vec4::new(r[0][0],r[1][0],r[2][0],0.),Vec4::new(r[0][1],r[1][1],r[2][1],0.),
    Vec4::new(r[0][2],r[1][2],r[2][2],0.),Vec4::new(r[0][3],r[1][3],r[2][3],1.))}
fn local_bounds(v:GpuTree)->([f32;4],[f32;4]) {
    let inverse=matrix(v.inverse);let mut low=Vec3::splat(f32::INFINITY);let mut high=-low;
    for i in 0..8 {let p=Vec3::new(if i&1==0{v.bounds_min[0]}else{v.bounds_max[0]},
        if i&2==0{v.bounds_min[1]}else{v.bounds_max[1]},if i&4==0{v.bounds_min[2]}else{v.bounds_max[2]});
        let p=inverse.transform_point3(p);low=low.min(p);high=high.max(p);}
    (low.extend(0.).to_array(),high.extend(0.).to_array())
}

/// Attach shared recipe geometry once, plus one lightweight root per tree and
/// a bounded number of branch OUTPUT slots. Never constructs recipe_forest::Pool.
pub fn plant(scene:&mut Scene,plants:&[crate::vegetation_asset::CookedPlant],trees:&[RootPlacement],slots:u32,policy:crate::aggregate_lod::Policy)->Result<(),String> {
    use crate::vegetation_asset::{SceneVegetation,AttachLimits,PlantInstance,SelectedRepresentation};
    if slots==0 || slots as usize>crate::voxel_forest::MAX_VIEW_GROUPS || trees.is_empty() || plants.is_empty()
        || trees.iter().any(|t|t.variant>=plants.len()||!t.scale.is_finite()||t.scale<=0.) {return Err("invalid GPU forest population/budget".into());}
    let poses=plants.iter().map(|p|p.stats().branches).sum::<usize>();
    let mut limits=AttachLimits {virtual_members:true,max_poses:poses.max(64),max_primitive_instances:1<<19,..Default::default()};
    limits.pack.max_assets=poses+plants.len();// The packed branch catalog is bounded independently of world roots.
    // append_proxy uses the asset-only catalog and is not a branch group.
    limits.pack.max_instances=poses;
    let mut builder=SceneVegetation::new(scene,limits)?;
    let mut handles=Vec::new();
    for (i,p) in plants.iter().enumerate() {handles.push(builder.append(p,PlantInstance {id:i as u64+1,generation:0,root:Mat4::IDENTITY,category:crate::scene::GeometryCategory::Trees})?);}
    for (i,t) in trees.iter().enumerate() {
        let root=Mat4::from_scale_rotation_translation(Vec3::splat(t.scale),glam::Quat::from_rotation_y(t.yaw),t.pos);
        builder.append_proxy(&plants[t.variant],PlantInstance {id:(plants.len()+i+1)as u64,generation:0,root,category:crate::scene::GeometryCategory::Trees})?;
    }
    builder.finish_aggregate(policy)?;
    let mut forest=scene.voxel_forest.take().ok_or("GPU forest has no volumes")?;
    let proxy_start=forest.instances.len()-trees.len();
    let proxies=forest.instances[proxy_start..].to_vec();
    let mut nodes=Vec::new();let mut representations=Vec::new();let mut root_nodes=Vec::new();
    for (variant,p) in plants.iter().enumerate() {
        let recipe=p.shared_selection_recipe(policy)?;let node_base=nodes.len()as u32;let rep_base=representations.len()as u32;
        root_nodes.push(node_base);
        for mut n in recipe.nodes {if matches!(recipe.representations[n.links[2]as usize],SelectedRepresentation::Voxel {branch:0,..}) {n.links[3]|=2;}if n.links[0]!=NONE {n.links[0]+=node_base;}n.links[1]+=node_base;n.links[2]+=rep_base;nodes.push(n);}
        for r in recipe.representations {representations.push(match r {
            SelectedRepresentation::SourceBranch {branch,..}=>[handles[variant].pose_range.start as u32+branch,0,0,0],
            SelectedRepresentation::Voxel {branch,level,..}=>[handles[variant].pose_range.start as u32+branch,1,level,0],
        });}
    }
    let mut branches=Vec::new();
    for p in &scene.part_wind {
        let source=p.owner.map(|o|scene.instances[o as usize]).unwrap_or_else(GpuInstance::zeroed);
        let volume=p.volume.map(|v|forest.instances[v as usize]).unwrap_or(forest.instances[0]);
        let (low,high)=local_bounds(volume);
        branches.push(Branch {rest:rows(p.rest),source,volume,low,high,
            animation:[p.max_angle,p.harmonic as f32,p.phase,0.],info:[p.parent.unwrap_or(NONE),u32::from(p.volume.is_some() && p.parent.is_some()),0,0]});
    }
    // Reference recipe carriers are not world instances. Keep the shared
    // negative-radius primitive templates; retire only their reference owners.
    for p in &scene.part_wind {if let Some(o)=p.owner {scene.instances[o as usize].sphere[3]=-1.;scene.instances[o as usize].first_cluster=0;}}
    scene.part_wind.clear();scene.part_attachments.clear();scene.aggregate_lod=None;
    scene.foliage_representations.clear();scene.foliage_source_groups.clear();
    let first_owner=scene.instances.len()as u32;
    if first_owner.checked_add(slots).is_none_or(|end|end>=1<<19) {return Err("GPU branch owners exceed virtual ID encoding".into());}
    let template=branches.iter().find(|b|b.source.first_cluster&4095!=0).ok_or("GPU forest has no source templates")?.source;
    let mut parked=template;parked.rows=rows(Mat4::from_translation(Vec3::new(0.,-100000.,0.)));parked.sphere=[0.,-100000.,0.,1.];
    scene.instances.extend(std::iter::repeat_n(parked,slots as usize));
    let mut placeholder=forest.instances[0];placeholder.inverse=rows(matrix(parked.rows).inverse());
    placeholder.bounds_min=[0.,-100001.,0.,0.];placeholder.bounds_max=[1.,-100000.,1.,0.];
    forest.instances=(0..slots).map(|slot|GpuTree {meta:[first_owner+slot,placeholder.meta[1],placeholder.meta[2],u32::MAX],..placeholder}).collect();
    forest.instances.extend_from_slice(&proxies);
    let roots=trees.iter().enumerate().map(|(i,t)| {
        let m=Mat4::from_scale_rotation_translation(Vec3::splat(t.scale),glam::Quat::from_rotation_y(t.yaw),t.pos);
        // sqrt(norm1*normInf) conservatively bounds rotated scales.
        let row=(0..3).map(|r|m.row(r).truncate().abs().element_sum()).fold(0.,f32::max);
        let col=(0..3).map(|c|m.col(c).truncate().abs().element_sum()).fold(0.,f32::max);
        scene.instances[proxies[i].meta[0]as usize].flags|=PART_WIND;
        Instance {rows:rows(m),scale:[(row*col).sqrt(),0.,0.,0.],info:[root_nodes[t.variant],0,handles[t.variant].pose_range.start as u32,0]}
    }).collect();
    scene.voxel_forest=Some(forest);scene.voxel_proxies=trees.len()as u32;
    scene.gpu_forest=Some(Spec {nodes,roots,branches,representations,proxies,first_owner,slots,error:policy.max_error_pixels.max(0.25)});
    Ok(())
}

pub struct Runtime {
    selector:Selector,residency:Residency,uniform:wgpu::Buffer,bind:wgpu::BindGroup,
    pipeline:wgpu::ComputePipeline,empty_depth:wgpu::TextureView,
    pub choices:wgpu::Buffer,slots:u32,trees:u32,error:f32,
    wind:std::sync::Mutex<crate::wind::WindState>,
}
#[derive(Debug,serde::Serialize)]
pub struct Diagnostics {
    pub trees:u32,pub branch_capacity:u32,pub selected_records:u32,pub tested_nodes:u32,
    pub source_slots:u32,pub voxel_slots:u32,pub shadow_voxel_slots:u32,pub direct_roots:u32,pub fallback_trees:u32,pub retained:u32,pub assigned:u32,
    pub slot_failures:u32,pub hash_failures:u32,
}
impl Runtime {
    pub fn new(gpu:&Gpu,spec:&Spec,instances:&wgpu::Buffer,volumes:&wgpu::Buffer)->Result<Self,String> {
        if gpu.device.limits().max_storage_buffers_per_shader_stage<12 {return Err("GPU forest materialization requires 12 storage bindings".into());}
        for i in 0..spec.branches.len() {
            let mut n=i;let mut depth=1;
            while spec.branches[n].info[0]!=NONE {
                let p=spec.branches[n].info[0]as usize;
                if p>=n || depth>=64 {return Err("GPU forest branch ancestry is not bounded parent-first".into());}
                n=p;depth+=1;
            }
        }
        if (spec.slots as u64+spec.roots.len()as u64).div_ceil(64)>gpu.device.limits().max_compute_workgroups_per_dimension as u64 {
            return Err("GPU forest materialization exceeds dispatch capacity".into());
        }
        let records=spec.slots.saturating_mul(4).max(spec.roots.len()as u32).max(1024);
        let selector=Selector::new(gpu,&spec.nodes,&spec.roots,records,records)?;
        let residency=Residency::new(gpu,&selector,spec.slots)?;
        crate::shaders::checked(gpu,|| {
            let upload=|label,data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some(label),contents:data,usage:wgpu::BufferUsages::STORAGE});
            let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU forest pose state"),size:32,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let choices=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU forest shared representation choices"),size:131072,
                usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            let root_history=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU forest root representation history"),size:spec.roots.len()as u64*4,usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
            let branches=upload("shared forest branch rigs",bytemuck::cast_slice(&spec.branches));
            let reps=upload("shared forest representations",bytemuck::cast_slice(&spec.representations));
            let proxies=upload("resident forest roots",bytemuck::cast_slice(&spec.proxies));
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("GPU forest materialization"),entries:&(0..13).map(|binding|wgpu::BindGroupLayoutEntry {
                binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding<8 || binding==11}},has_dynamic_offset:false,min_binding_size:None},
            }).collect::<Vec<_>>()});
            let buffers=[&uniform,&selector._instances,&selector._nodes,&branches,&reps,&residency.slots,&selector.fallback,&proxies,instances,volumes,&choices,&selector.direct,&root_history];
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("GPU forest materialization"),layout:&layout,entries:&buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>()});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("GPU forest materialization"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("GPU forest materialization"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_materialize.wgsl").into())});
            let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("GPU forest materialization"),layout:Some(&pl),module:&module,entry_point:Some("materialize"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            let empty_depth=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("GPU forest conservative depth"),size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]}).create_view(&Default::default());
            Self {selector,residency,uniform,bind,pipeline,empty_depth,choices,slots:spec.slots,trees:spec.roots.len()as u32,error:spec.error,wind:Default::default()}
        })
    }
    pub fn set_wind(&self,state:&crate::wind::WindState)->Result<(),String>{*self.wind.lock().unwrap()=state.validated()?;Ok(())}
    /// Explicit synchronous diagnostic only; never called to select or draw.
    pub fn diagnostics(&self,gpu:&Gpu)->Result<Diagnostics,String> {
        let inputs=[(&self.selector.counters,64u64),(&self.residency.counters,32),(&self.selector.fallback,self.trees as u64*4),(&self.choices,131072)];
        let size=inputs.iter().map(|(_,n)|n).sum();
        let output=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("forest diagnostic snapshot"),size,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());let mut offset=0;
        for (buffer,len) in inputs {enc.copy_buffer_to_buffer(buffer,0,&output,offset,len);offset+=len;}
        let submission=gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::sync_channel(1);
        output.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});
        gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).map_err(|e|e.to_string())?;
        rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        let bytes=output.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
        let words:Vec<u32>=bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect();
        let c=&words[..16];let r=&words[16..24];let fallback=&words[24..24+self.trees as usize];
        let choices=&words[24+self.trees as usize..];
        let voxels=choices[..self.slots as usize].iter().filter(|w|**w&3==1).count()as u32;
        let shadow_voxel_slots=choices[16384..16384+self.slots as usize].iter().filter(|w|**w&3==1).count()as u32;
        Ok(Diagnostics {trees:self.trees,branch_capacity:self.slots,selected_records:c[0],tested_nodes:c[4],source_slots:r[6].saturating_sub(voxels),voxel_slots:voxels,shadow_voxel_slots,direct_roots:c[7],
            fallback_trees:fallback.iter().filter(|f|**f!=0).count()as u32,retained:r[2],assigned:r[3],slot_failures:r[4],hash_failures:r[5]})
    }
    pub fn encode(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[crate::part_wind::AnimationView],first_owner:u32)->Result<(),String> {
        self.selector.union(gpu,enc,views,self.error,self.slots,&self.empty_depth)?;
        self.residency.encode(enc);
        let wind=*self.wind.lock().unwrap();
        let field=[wind.direction.x,wind.direction.y,wind.strength,(wind.time_seconds.rem_euclid(120.)/120.*std::f64::consts::TAU)as f32];
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::cast_slice(&field));
        gpu.queue.write_buffer(&self.uniform,16,bytemuck::cast_slice(&[self.slots,first_owner,self.trees,0]));
        let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);
        pass.dispatch_workgroups((self.slots+self.trees).div_ceil(64),1,1);Ok(())
    }
}

#[cfg(test)] mod tests {
    #[test] fn materialize_shader_validates(){let src=include_str!("shaders/foliage_materialize.wgsl");
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));}

    #[test]
    fn large_world_roots_do_not_consume_branch_pack_budget() {
        use super::*;
        let source=crate::vegetation_asset::tests::aggregate_fixture();
        let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();
        cooked.cook_proxy(&source,config,3).unwrap();
        let trees=vec![RootPlacement {pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0};20000];
        let mut scene=Scene::default();
        plant(&mut scene,&[cooked],&trees,8,crate::aggregate_lod::Policy::default()).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();
        assert_eq!(spec.roots.len(),20000);assert_eq!(spec.branches.len(),3);
        assert_eq!(scene.voxel_forest.as_ref().unwrap().instances.len(),20008);
        assert!(scene.part_wind.is_empty());
    }

    #[test]
    #[ignore="exclusive GPU forest draw fixture"]
    fn gpu_forest_voxels_overflow_split_and_teleport() {
        use super::*;
        use crate::{Camera,Lighting,Renderer,ViewRect,vegetation_asset::{CookedPlant,SceneVegetation,AttachLimits,PlantInstance}};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        // Keep GI enabled: conservative selection bounds must not inflate canopy density.
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:1.,shadow_strength:0.,..Default::default()};
        let policy=crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()};
        let cam=|x:f32|Camera {pos:Vec3::new(x+0.3,0.4,1.6),target:Vec3::new(x+0.3,0.4,0.),fov_y:1.,near:0.05};
        let mut reference=Scene::default();let mut b=SceneVegetation::new(&mut reference,AttachLimits::default()).unwrap();
        b.append_proxy(&cooked,PlantInstance {id:1,generation:0,root:Mat4::IDENTITY,category:crate::scene::GeometryCategory::Trees}).unwrap();b.finish_aggregate(policy).unwrap();
        let reference=Renderer::new(&gpu,&reference,&light);
        let view=[ViewRect {cam:cam(0.),x:0,y:0,w:128,h:128}];
        let (expected,_)=reference.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap();
        let tree=RootPlacement {pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0};
        let mut scene=Scene::default();plant(&mut scene,std::slice::from_ref(&cooked),&[tree],1,policy).unwrap();
        let renderer=Renderer::new(&gpu,&scene,&light);
        let (actual,_)=renderer.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap();
        let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        assert_eq!(d.fallback_trees,1);assert_eq!(d.source_slots+d.voxel_slots,0);
        let mae=expected.iter().zip(&actual).map(|(&a,&b)|a.abs_diff(b)as u64).sum::<u64>()as f64/expected.len()as f64;
        eprintln!("resident fallback vs proxy reference MAE={mae}");assert!(mae<2.);
        // Force regular whole-subtree voxel selection, distinct from overflow.
        let mut scene=Scene::default();let coarse=crate::aggregate_lod::Policy {allow_sampled_optics:true,max_color_error:1.,max_coverage_error:1.,max_error_pixels:10000.,..policy};
        plant(&mut scene,std::slice::from_ref(&cooked),&[tree],8,coarse).unwrap();
        let renderer=Renderer::new(&gpu,&scene,&light);let distant=[ViewRect {cam:Camera {pos:Vec3::new(0.3,0.4,20.),..cam(0.)},..view[0]}];renderer.try_render_views(&gpu,&distant,&light,128,128,&[]).unwrap();
        let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();assert_eq!(d.direct_roots,1);assert_eq!(d.voxel_slots,0);assert_eq!(d.source_slots,0);assert_eq!(d.fallback_trees,0);
        // Move between disjoint tree populations, then use four simultaneous
        // cameras. No host assignment, renderer rebuild or root upload occurs.
        let trees=(0..4).map(|i|RootPlacement {pos:Vec3::X*(i as f32*8.),..tree}).collect::<Vec<_>>();
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&trees,16,policy).unwrap();let renderer=Renderer::new(&gpu,&scene,&light);
        let night=Lighting {sun_lux:0.,..light.clone()};
        for index in [0,3,1,2,0] {
            let v=[ViewRect {cam:cam(index as f32*8.),x:0,y:0,w:128,h:128}];
            renderer.try_render_views(&gpu,&v,&night,128,128,&[]).unwrap();let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
            assert_eq!(d.fallback_trees,0);assert_eq!(d.source_slots,3);assert_eq!(d.assigned,3);assert_eq!(d.shadow_voxel_slots,2);
        }
        let views=(0..4).map(|i|ViewRect {cam:cam(i as f32*8.),x:(i%2)*128,y:(i/2)*128,w:128,h:128}).collect::<Vec<_>>();
        renderer.try_render_views(&gpu,&views,&night,256,256,&[]).unwrap();let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        assert_eq!(d.source_slots,12);assert_eq!(d.fallback_trees,0);
        renderer.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap();
        let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        assert_eq!(d.source_slots,12,"off-camera trees must remain in the independent sun-caster cut");
        eprintln!("GPU forest source/voxel draws, fallback, teleport, four cameras PASS");
    }
}
