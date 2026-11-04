//! GPU forest selection, independent of the CPU near-tree pool.
//!
//! Immutable preorder recipe trees are shared by any number of instances.
//! Phase one uses historical depth only to defer work. Phase two retests that
//! work against CURRENT depth. No readback or CPU tree shortlist is required.
//! foliage_runtime connects a budgeted consumer union to actual source/voxel draws.
//! Consumers must discard records whose instance fallback flag is nonzero and
//! render that instance's resident root instead. This avoids holes AND duplicate
//! partial subtrees on overflow. Independent cuts need separate scratch; the
//! runtime instead shares one frame-wide camera/shadow union and residency.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;
pub const NONE:u32=u32::MAX;
#[repr(C)]
#[derive(Clone,Copy,Debug,Pod,Zeroable)]
pub struct Node {
    /// Local-space conservative animated subtree sphere.
    pub sphere:[f32;4],
    /// Geometric/aggregate error in metres. Remaining components reserved.
    pub error:[f32;4],
    /// First child (NONE for leaf), exclusive subtree end, representation ID, flags.
    /// Flag bit 0: resident overflow fallback only; always refine normally.
    /// Flag bit 1: whole-root voxel choice, emitted without a transient slot.
    pub links:[u32;4],
}
#[repr(C)]
#[derive(Clone,Copy,Debug,Pod,Zeroable)]
pub struct Instance {
    pub rows:[[f32;4];3],
    /// Conservative linear transform scale bound, world wind padding, reserved.
    pub scale:[f32;4],
    /// Root node, stable generation, reserved. Array position is the stable ID.
    pub info:[u32;4],
}
/// Visibility ownership is explicit: projection shape cannot distinguish a
/// perspective lamp from a camera, or an orthographic camera from the sun.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(u32)]
pub enum ConsumerKind { Camera=0, Shadow=1 }
#[derive(Clone,Copy)]
pub struct ConsumerView {pub projection:crate::part_wind::AnimationView,pub kind:ConsumerKind}
impl ConsumerView {
    pub fn camera(projection:crate::part_wind::AnimationView)->Self {Self {projection,kind:ConsumerKind::Camera}}
    pub fn shadow(projection:crate::part_wind::AnimationView)->Self {Self {projection,kind:ConsumerKind::Shadow}}
    fn packed(self)->Result<Consumer,String> {
        let v=self.projection;
        if !v.view_projection.is_finite() || v.viewport.contains(&0) {return Err("invalid foliage consumer projection".into());}
        Ok(Consumer {vp:v.view_projection.to_cols_array(),pixels:[v.viewport[0]as f32,v.viewport[1]as f32,self.kind as u32 as f32,0.]})
    }
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Consumer {vp:[f32;16],pixels:[f32;4]}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {consumers:[Consumer;32],consumer_count:[u32;4],vp:[f32;16],previous_vp:[f32;16],pixels:[f32;4],counts:[u32;4]}

fn validate(nodes:&[Node],instances:&[Instance],capacity:u32)->Result<(),String> {
    if nodes.is_empty() || instances.is_empty() || capacity==0 {return Err("empty foliage selection input/capacity".into());}
    if (nodes.len()as u64)*(instances.len()as u64)>u32::MAX as u64/2 {return Err("foliage traversal counter budget exceeded".into());}
    let mut ends=Vec::<usize>::new();
    for (i,n) in nodes.iter().enumerate() {
        while ends.last()==Some(&i) {ends.pop();}
        if !n.sphere.iter().chain(n.error.iter()).all(|x|x.is_finite()) || n.sphere[3]<0. || n.error[0]<0.
            || n.links[1]as usize<=i || n.links[1]as usize>nodes.len()
            || ends.last().is_some_and(|&end|n.links[1]as usize>end) {
            return Err("invalid foliage subtree bounds/escape".into());
        }
        if n.links[0]==NONE {
            if n.links[1]as usize!=i+1 {return Err("leaf must escape to next node".into());}
        }else {
            if n.links[0]as usize!=i+1 || n.links[1]as usize<=i+1 {return Err("children must be contiguous preorder".into());}
            // Every immediate child is inside the parent envelope. This is
            // required for safe subtree culling, not merely authoring advice.
            let mut child=i+1;
            while child<(n.links[1]as usize) {
                let c=&nodes[child];
                let d=glam::Vec3::from_slice(&c.sphere).distance(glam::Vec3::from_slice(&n.sphere));
                if !d.is_finite() || d+c.sphere[3]>n.sphere[3]+1e-5 {return Err("child outside foliage parent envelope".into());}
                if c.links[1]as usize<=child || c.links[1]>n.links[1] {return Err("invalid foliage child escape".into());}
                child=c.links[1]as usize;
            }
            ends.push(n.links[1]as usize);
        }
    }
    for t in instances {
        if t.info[0]as usize>=nodes.len() || !t.rows.iter().flatten().chain(t.scale.iter()).all(|v|v.is_finite())
            || t.scale[0]<=0. || t.scale[1]<0. {return Err("invalid foliage instance".into());}
        // ||A||2 <= sqrt(||A||1 * ||A||infinity); permits nonuniform scales and
        // shear without unsafe culling. A supplied larger bound is fine.
        let row=(0..3).map(|r|(0..3).map(|c|t.rows[r][c].abs()).sum::<f32>()).fold(0.,f32::max);
        let col=(0..3).map(|c|(0..3).map(|r|t.rows[r][c].abs()).sum::<f32>()).fold(0.,f32::max);
        if t.scale[0]+1e-5<(row*col).sqrt() {return Err("foliage scale bound is not conservative".into());}
    }
    Ok(())
}

pub struct Selector {
    pub(crate) _nodes:wgpu::Buffer,pub(crate) _instances:wgpu::Buffer,uniform:wgpu::Buffer,
    /// vec4(instance ID, node ID, representation ID, generation).
    pub records:wgpu::Buffer,
    pub asset_requests:wgpu::Buffer,pub(crate) asset_pages:wgpu::Buffer,asset_state:std::sync::Mutex<Vec<[u32;4]>>,pinned_pages:Vec<bool>,compact_assets:wgpu::ComputePipeline,
    deferred:wgpu::Buffer,
    /// count, deferred count, deferred overflow, output overflow, tested,
    /// frustum rejected, current-depth rejected, direct roots; indirect args at 32/48.
    /// Tested nodes counts the emitting walk, not the budget-count prepass.
    /// Word 11: frontier overflow requests; word 15: peak requested frontier.
    pub counters:wgpu::Buffer,
    /// Separate indirect arguments: deferred dispatch at 0, consumer dispatch at 12.
    pub arguments:wgpu::Buffer,
    /// One u32 per instance, reset each frame. See module consumer contract.
    pub fallback:wgpu::Buffer,
    /// Whole-tree representation + 1; zero means branches or culled. No branch slot needed.
    pub direct:wgpu::Buffer,
    children:wgpu::Buffer,work:[wgpu::Buffer;2],levels:u32,seed_demand:wgpu::ComputePipeline,seed_emit:wgpu::ComputePipeline,prepare_wave:wgpu::ComputePipeline,count_wave:wgpu::ComputePipeline,emit_wave:wgpu::ComputePipeline,
    pub(crate) demand:wgpu::Buffer,bucket_cost:wgpu::Buffer,prefix_demand:wgpu::ComputePipeline,prefix_buckets:wgpu::ComputePipeline,
    layout:wgpu::BindGroupLayout,first:wgpu::ComputePipeline,second:wgpu::ComputePipeline,finish:wgpu::ComputePipeline,
    count:u32,capacity:u32,deferred_capacity:u32,
    binds:std::sync::Mutex<crate::frame::resource_cache::BindGroups<4>>,
}
impl Selector {
    pub(crate) fn dimensions(&self)->(u32,u32) {(self.capacity,self.count)}
    pub fn new(gpu:&Gpu,nodes:&[Node],instances:&[Instance],capacity:u32,deferred_capacity:u32)->Result<Self,String> {
        let work_capacity=(instances.len()as u32).saturating_mul(8).max(capacity.saturating_mul(4)).max(4096).min(1<<20).max(instances.len()as u32);
        Self::with_work_capacity(gpu,nodes,instances,capacity,deferred_capacity,work_capacity)
    }
    /// Bounded frontier storage. Overflow publishes a complete resident root,
    /// never a partial cut. This is a memory budget, not a backend switch.
    pub fn with_work_capacity(gpu:&Gpu,nodes:&[Node],instances:&[Instance],capacity:u32,deferred_capacity:u32,work_capacity:u32)->Result<Self,String> {
        validate(nodes,instances,capacity)?;
        let limit=gpu.device.limits();
        if work_capacity<(instances.len()as u32) || work_capacity.div_ceil(64)>limit.max_compute_workgroups_per_dimension
            || limit.max_storage_buffers_per_shader_stage<14 || limit.max_compute_workgroups_per_dimension<32768 {return Err("foliage work queue exceeds device limits".into());}
        let page_count=nodes.iter().map(|n|n.links[2]).max().unwrap_or(0).checked_add(1).ok_or("asset page ID overflow")?;
        if page_count>1<<20 {return Err("asset page catalog exceeds bounded request capacity".into());}
        let mut pinned_pages=vec![false;page_count as usize];
        // Only the complete root fallback is mandatory. Whole-root detail
        // candidates also use bit 1, but must remain evictable.
        let mut root=0usize;
        while root<nodes.len() {let n=&nodes[root];if n.links[3]&2!=0 {pinned_pages[n.links[2]as usize]=true;}root=n.links[1]as usize;}
        let mut asset_state=vec![[1u32,0,1,0];page_count as usize];asset_state[0][3]=1;
        let mut ends=Vec::new();let mut levels=0;
        for (i,node) in nodes.iter().enumerate() {
            while ends.last()==Some(&i){ends.pop();}
            levels=levels.max(ends.len()as u32+1);
            if node.links[0]!=NONE {ends.push(node.links[1]as usize);}
        }

        let mut children=vec![0u32;nodes.len()*2];
        for (i,node) in nodes.iter().enumerate() {
            children[i*2]=children.len()as u32;
            if node.links[0]!=NONE {
                let mut child=node.links[0];
                while child<node.links[1] {children.push(child);children[i*2+1]+=1;child=nodes[child as usize].links[1];}
            }
        }
        if deferred_capacity==0 || instances.len().div_ceil(64)>gpu.device.limits().max_compute_workgroups_per_dimension as usize
            || deferred_capacity.div_ceil(64)>gpu.device.limits().max_compute_workgroups_per_dimension {
            return Err("foliage dispatch capacity exceeds device".into());
        }
        for bytes in [children.len()as u64*4,16+work_capacity as u64*8,instances.len()as u64*16,(instances.len().div_ceil(256)as u64+1)*128,std::mem::size_of_val(nodes)as u64,std::mem::size_of_val(instances)as u64,capacity as u64*16,deferred_capacity as u64*8,instances.len()as u64*4] {
            if bytes>gpu.device.limits().max_storage_buffer_binding_size as u64 {return Err("foliage buffer exceeds device binding limit".into());}
        }
        crate::shaders::checked(gpu,|| {
            let upload=|label,data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:Some(label),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
            let storage=|label,size|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,
                usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::INDIRECT,mapped_at_creation:false});
            let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage selector view"),size:std::mem::size_of::<Uniform>()as u64,
                usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let entries=(0..16).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,
                ty:if binding==7 {wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}}
                else {wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform} else {wgpu::BufferBindingType::Storage {read_only:binding<3 || binding==13}},has_dynamic_offset:false,min_binding_size:None}}}).collect::<Vec<_>>();
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("foliage GPU selection"),entries:&entries});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("foliage GPU selection"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("foliage GPU selection"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_select.wgsl").into())});
            let make=|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            Self {asset_pages:upload("foliage asset publication",bytemuck::cast_slice(&asset_state)),asset_requests:storage("foliage compact asset demand",16+page_count as u64*16),asset_state:std::sync::Mutex::new(asset_state),pinned_pages,compact_assets:make("compact_asset_requests"),children:upload("shared foliage child spans",bytemuck::cast_slice(&children)),work:[storage("foliage frontier A",16+work_capacity as u64*8),storage("foliage frontier B",16+work_capacity as u64*8)],levels,
                seed_demand:make("seed_demand"),seed_emit:make("seed_emit"),prepare_wave:make("prepare_wave"),count_wave:make("count_wave"),emit_wave:make("emit_wave"),_nodes:upload("shared foliage hierarchy",bytemuck::cast_slice(nodes)),_instances:upload("persistent foliage instances",bytemuck::cast_slice(instances)),uniform,
                records:storage("selected foliage records",capacity as u64*16),deferred:storage("deferred foliage subtrees",deferred_capacity as u64*8),
                counters:storage("foliage selection counters",64),arguments:storage("foliage indirect arguments",24),fallback:storage("foliage root fallbacks",instances.len()as u64*4),
                direct:storage("direct resident forest roots",instances.len()as u64*4),
                demand:storage("forest root demand",instances.len()as u64*16),bucket_cost:storage("forest priority costs",(instances.len().div_ceil(256)as u64+1)*128),prefix_demand:make("prefix_demand"),prefix_buckets:make("prefix_buckets"),first:make("first"),second:make("second"),finish:make("finish"),layout,count:instances.len()as u32,capacity,deferred_capacity,binds:Default::default()}
        })
    }
    /// Publish at a frame boundary, after the associated data upload is ordered
    /// on this queue and before encoding consumers of the new publication.
    /// Invalid batches have no GPU effects. Resident roots cannot be evicted.
    /// This controls visibility safety; it does not itself allocate asset memory.
    pub fn publish_asset_pages(&self,gpu:&Gpu,updates:&[(u32,bool,u32)])->Result<(),String> {
        let mut state=self.asset_state.lock().unwrap();let mut seen=std::collections::HashSet::new();
        for &(id,resident,generation) in updates {
            let old=state.get(id as usize).ok_or("asset page outside catalog")?;
            if !seen.insert(id) || generation==0 || generation<old[2] {return Err("duplicate or stale asset page publication".into());}
            if !resident && self.pinned_pages[id as usize] {return Err("resident root pages are pinned".into());}
        }
        if updates.is_empty(){return Ok(());}
        let epoch=state[0][3].checked_add(1).ok_or("asset publication epoch exhausted")?;
        for &(id,resident,generation) in updates {state[id as usize]=[u32::from(resident),0,generation,if id==0 {epoch}else{0}];gpu.queue.write_buffer(&self.asset_pages,id as u64*16,bytemuck::cast_slice(&state[id as usize]));}
        state[0][3]=epoch;gpu.queue.write_buffer(&self.asset_pages,12,bytemuck::bytes_of(&epoch));
        Ok(())
    }
    fn bind(&self,gpu:&Gpu,hzb:&wgpu::TextureView)->wgpu::BindGroup {self.bind_work(gpu,hzb,0)}
    fn bind_work(&self,gpu:&Gpu,hzb:&wgpu::TextureView,front:usize)->wgpu::BindGroup {
        let buffers=[&self.uniform,&self._nodes,&self._instances,&self.records,&self.deferred,&self.counters,&self.fallback];
        let mut entries=buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>();
        entries.push(wgpu::BindGroupEntry {binding:7,resource:wgpu::BindingResource::TextureView(hzb)});
        entries.push(wgpu::BindGroupEntry {binding:8,resource:self.direct.as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:9,resource:self.demand.as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:10,resource:self.bucket_cost.as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:11,resource:self.work[front].as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:12,resource:self.work[1-front].as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:13,resource:self.children.as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:14,resource:self.asset_pages.as_entire_binding()});
        entries.push(wgpu::BindGroupEntry {binding:15,resource:self.asset_requests.as_entire_binding()});
        self.binds.lock().unwrap().entries(gpu,&self.layout,"foliage selector depth",&entries)
    }
    /// Call once per view/frame; current depth MUST be rebuilt after consuming
    /// first-phase records, before second(). Historical depth must describe the
    /// supplied previous matrix; invalidate it on camera cuts or moved occluders.
    pub fn first(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,vp:Mat4,previous_vp:Mat4,size:[u32;2],error:f32,history_valid:bool,hzb:&wgpu::TextureView)->Result<(),String> {
        if !vp.is_finite() || !previous_vp.is_finite() || size.contains(&0) || !error.is_finite() || error<0. {return Err("invalid foliage selection view".into());}
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&Uniform {consumers:[Consumer::zeroed();32],consumer_count:[0;4],vp:vp.to_cols_array(),previous_vp:previous_vp.to_cols_array(),
            pixels:[size[0]as f32,size[1]as f32,error,0.],counts:[self.count,self.capacity,self.deferred_capacity,u32::from(history_valid)]}));
        enc.clear_buffer(&self.counters,0,None);enc.clear_buffer(&self.fallback,0,None);enc.clear_buffer(&self.direct,0,None);
        let bind=self.bind(gpu,hzb);let mut pass=enc.begin_compute_pass(&Default::default());
        pass.set_bind_group(0,&bind,&[]);pass.set_pipeline(&self.first);pass.dispatch_workgroups(self.count.div_ceil(64),1,1);
        pass.set_pipeline(&self.finish);pass.dispatch_workgroups(1,1,1);drop(pass);
        enc.copy_buffer_to_buffer(&self.counters,32,&self.arguments,0,12);
        enc.copy_buffer_to_buffer(&self.counters,48,&self.arguments,12,12);Ok(())
    }
    /// Conservative union across camera AND shadow consumers. This produces
    /// one shared resident cut for the frame. Per-view source/cluster HZB still
    /// runs downstream; no camera depth can reject an off-camera caster here.
    pub fn union(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[crate::part_wind::AnimationView],error:f32,branch_budget:u32,empty_depth:&wgpu::TextureView)->Result<(),String> {
        let consumers=views.iter().copied().map(ConsumerView::camera).collect::<Vec<_>>();
        self.union_consumers(gpu,enc,&consumers,error,branch_budget,empty_depth)
    }
    /// Frame-wide camera/shadow union. Shadow consumers retain demand even
    /// when cameras cannot see their casters; only cameras get budget priority.
    pub fn union_consumers(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[ConsumerView],error:f32,branch_budget:u32,empty_depth:&wgpu::TextureView)->Result<(),String> {
        if branch_budget==0 || branch_budget>self.capacity || views.is_empty() || views.len()>32 || !error.is_finite() || error<=0. {return Err("invalid foliage consumer union".into());}
        let mut consumers=[Consumer::zeroed();32];
        for (dst,v) in consumers.iter_mut().zip(views) {
            *dst=v.packed()?;
        }
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&Uniform {consumers,consumer_count:[views.len()as u32,branch_budget,0,0],
            vp:views[0].projection.view_projection.to_cols_array(),previous_vp:views[0].projection.view_projection.to_cols_array(),pixels:[0.,0.,error,0.],counts:[self.count,self.capacity,self.deferred_capacity,0]}));
        enc.clear_buffer(&self.counters,0,None);enc.clear_buffer(&self.fallback,0,None);enc.clear_buffer(&self.direct,0,None);
        let binds=[self.bind_work(gpu,empty_depth,0),self.bind_work(gpu,empty_depth,1)];
        enc.clear_buffer(&self.work[0],0,Some(16));
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&binds[0],&[]);
         pass.set_pipeline(&self.seed_demand);pass.dispatch_workgroups(self.count.div_ceil(64),1,1);}
        self.waves(enc,&binds,&self.count_wave);
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&binds[0],&[]);
         pass.set_pipeline(&self.prefix_demand);pass.dispatch_workgroups(self.count.div_ceil(256),1,1);
         pass.set_pipeline(&self.prefix_buckets);pass.dispatch_workgroups(1,1,1);}
        enc.clear_buffer(&self.work[0],0,Some(16));
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&binds[0],&[]);
         pass.set_pipeline(&self.seed_emit);pass.dispatch_workgroups(self.count.div_ceil(64),1,1);}
        self.waves(enc,&binds,&self.emit_wave);
        enc.clear_buffer(&self.asset_requests,0,Some(16));
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&binds[0],&[]);pass.set_pipeline(&self.compact_assets);pass.dispatch_workgroups((self.pinned_pages.len()as u32).div_ceil(64),1,1);}
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&binds[0],&[]);
         pass.set_pipeline(&self.finish);pass.dispatch_workgroups(1,1,1);}
        enc.copy_buffer_to_buffer(&self.counters,48,&self.arguments,12,12);Ok(())
    }
    fn waves(&self,enc:&mut wgpu::CommandEncoder,binds:&[wgpu::BindGroup;2],pipeline:&wgpu::ComputePipeline) {
        // The immutable recipe depth bounds submission, not CPU visibility.
        // Empty frontiers dispatch zero workgroups, without a GPU readback.
        for level in 0..self.levels {
            let bind=&binds[level as usize%2];
            {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,bind,&[]);
             pass.set_pipeline(&self.prepare_wave);pass.dispatch_workgroups(1,1,1);}
            enc.copy_buffer_to_buffer(&self.counters,32,&self.arguments,0,12);
            let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,bind,&[]);
            pass.set_pipeline(pipeline);pass.dispatch_workgroups_indirect(&self.arguments,0);
        }
    }
    /// Appends recovered records. This prototype requires final consumption after
    /// both phases: phase-one depth may seed HZB, but final color selection must
    /// honor fallback flags that phase two can set (discard partial subtrees).
    pub fn second(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,current_hzb:&wgpu::TextureView) {
        let bind=self.bind(gpu,current_hzb);let mut pass=enc.begin_compute_pass(&Default::default());
        pass.set_bind_group(0,&bind,&[]);pass.set_pipeline(&self.second);pass.dispatch_workgroups_indirect(&self.arguments,0);
        pass.set_pipeline(&self.finish);pass.dispatch_workgroups(1,1,1);drop(pass);
        enc.copy_buffer_to_buffer(&self.counters,48,&self.arguments,12,12);
        enc.clear_buffer(&self.asset_requests,0,Some(16));
        let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&bind,&[]);pass.set_pipeline(&self.compact_assets);pass.dispatch_workgroups((self.pinned_pages.len()as u32).div_ceil(64),1,1);
    }
}

#[cfg(test)]
mod tests {
 use super::*;
 fn fixture(count:usize)->(Vec<Node>,Vec<Instance>) {
  let nodes=vec![
   Node {sphere:[0.,0.,0.5,0.3],error:[0.1,0.,0.,0.],links:[1,3,90,0]},
   Node {sphere:[-0.12,0.,0.5,0.1],error:[0.;4],links:[NONE,2,91,0]},
   Node {sphere:[0.12,0.,0.5,0.1],error:[0.;4],links:[NONE,3,92,0]}];
  let tree=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0,17,0,0]};
  (nodes,vec![tree;count])
 }
 #[test] fn validates_shared_subtrees_and_conservative_bounds() {
  let (nodes,trees)=fixture(2049);assert!(validate(&nodes,&trees,1).is_ok());
  let mut bad=nodes.clone();bad[0].links[0]=0;assert!(validate(&bad,&trees,1).is_err());
  let mut bad=nodes.clone();bad[1].sphere[0]=2.;assert!(validate(&bad,&trees,1).is_err());
  let mut bad=nodes.clone();bad[2].links[1]=2;assert!(validate(&bad,&trees,1).is_err());
  let mut bad=trees.clone();bad[0].rows[0][0]=2.;assert!(validate(&nodes,&bad,1).is_err());
  assert_eq!(std::mem::size_of::<Node>(),48);assert_eq!(std::mem::size_of::<Instance>(),80);
 }
 #[test] fn consumer_roles_are_independent_of_projection_shape() {
  use crate::part_wind::AnimationView;
  for matrix in [Mat4::IDENTITY,Mat4::perspective_infinite_reverse_rh(1.,1.,0.1)] {
   let view=AnimationView {view_projection:matrix,viewport:[1920,1080]};
   let camera=ConsumerView::camera(view).packed().unwrap();
   let shadow=ConsumerView::shadow(view).packed().unwrap();
   assert_eq!(camera.vp,shadow.vp);assert_eq!(camera.pixels[..2],shadow.pixels[..2]);
   assert_eq!(camera.pixels[2],0.);assert_eq!(shadow.pixels[2],1.);
  }
  assert!(ConsumerView::camera(AnimationView {view_projection:Mat4::IDENTITY,viewport:[0,1080]}).packed().is_err());
  assert_eq!(std::mem::size_of::<Consumer>(),80,"keep uniform ABI stable");
 }
 #[test] fn shader_validates() {
  let module=naga::front::wgsl::parse_str(include_str!("shaders/foliage_select.wgsl")).unwrap();
  naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
 }
 fn read(gpu:&Gpu,b:&wgpu::Buffer)->Vec<u32> {
  let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("selector test readback"),size:b.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
  let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(b,0,&out,0,b.size());
  let submission=gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::sync_channel(1);
  out.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});
  gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).unwrap();rx.recv().unwrap().unwrap();
  let bytes=out.slice(..).get_mapped_range().unwrap();bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect()
 }
 fn depth(gpu:&Gpu,value:f32)->wgpu::TextureView {
  let tex=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("selector test depth"),size:wgpu::Extent3d {width:8,height:8,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
   format:wgpu::TextureFormat::R32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
  gpu.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&tex,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},bytemuck::cast_slice(&[value;64]),
   wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(32),rows_per_image:Some(8)},wgpu::Extent3d {width:8,height:8,depth_or_array_layers:1});
  tex.create_view(&Default::default())
 }
 #[test]
 #[ignore="exclusive GPU resident ancestor publication fixture"]
 fn gpu_missing_detail_keeps_nearest_complete_ancestor() {
  let gpu=Gpu::new().unwrap();let (_,mut trees)=fixture(1);trees[0].info[0]=0;
  let n=|radius,error,first,end,rep,flags|Node {sphere:[0.,0.,0.5,radius],error:[error,0.,0.,0.],links:[first,end,rep,flags]};
  let nodes=vec![n(0.3,1.,1,7,0,2),n(0.2,1.,2,4,1,0),n(0.1,0.,NONE,3,2,0),n(0.1,0.,NONE,4,3,0),
   n(0.2,1.,5,7,4,0),n(0.1,0.,NONE,6,5,0),n(0.1,0.,NONE,7,6,0)];
  let selector=Selector::new(&gpu,&nodes,&trees,16,16).unwrap();let empty=depth(&gpu,0.);
  let views=[crate::part_wind::AnimationView {view_projection:Mat4::IDENTITY,viewport:[128;2]};4];
  let draw=|budget| {
   let mut enc=gpu.device.create_command_encoder(&Default::default());selector.union(&gpu,&mut enc,&views,0.01,budget,&empty).unwrap();gpu.queue.submit([enc.finish()]);
   let count=read(&gpu,&selector.counters)[0]as usize;
   let mut cut=read(&gpu,&selector.records).chunks_exact(4).take(count).map(|r|r[1]).collect::<Vec<_>>();cut.sort();cut
  };
  selector.publish_asset_pages(&gpu,&[(2,false,1)]).unwrap();
  assert_eq!(draw(16),[1,5,6],"only the missing subtree coarsens; its sibling keeps detail");
  assert_eq!(read(&gpu,&selector.fallback),[0]);assert_eq!(read(&gpu,&selector.direct),[0]);
  let requests=read(&gpu,&selector.asset_requests);assert!(requests[4..4+requests[0]as usize*4].chunks_exact(4).any(|r|r[0]==2 && r[1]==0));
  selector.publish_asset_pages(&gpu,&[(2,true,2)]).unwrap();assert_eq!(draw(16),[2,3,5,6]);
  selector.publish_asset_pages(&gpu,&[(1,false,1)]).unwrap();assert!(draw(16).is_empty());
  assert_eq!(read(&gpu,&selector.direct),[1],"missing immediate child retains whole resident root");
  selector.publish_asset_pages(&gpu,&[(1,true,2),(3,false,1)]).unwrap();assert_eq!(draw(16),[1,5,6]);
  assert!(draw(2).is_empty());assert_eq!(read(&gpu,&selector.fallback),[2],"budget cannot publish a partial ancestor cut");
  selector.publish_asset_pages(&gpu,&[(3,true,2)]).unwrap();assert_eq!(draw(16),[2,3,5,6]);
 }
 #[test]
 #[ignore="exclusive GPU consumer ownership and budget fixture"]
 fn gpu_consumer_demand_distinguishes_camera_and_shadow_only_roots() {
  use crate::part_wind::AnimationView;
  let gpu=Gpu::new().unwrap();let (nodes,mut trees)=fixture(3);
  trees[1].rows[0][3]=4.;trees[2].rows[0][3]=8.;
  let selector=Selector::new(&gpu,&nodes,&trees,6,6).unwrap();let empty=depth(&gpu,0.);
  let at=|x:f32|AnimationView {view_projection:Mat4::from_translation(glam::Vec3::new(-x,0.,0.)),viewport:[128;2]};
  let mut consumers=[ConsumerView::camera(at(0.)),ConsumerView::shadow(at(0.)),ConsumerView::shadow(at(4.))];
  for _ in 0..2 {
   let mut enc=gpu.device.create_command_encoder(&Default::default());
   selector.union_consumers(&gpu,&mut enc,&consumers,0.01,2,&empty).unwrap();gpu.queue.submit([enc.finish()]);
   let demand=read(&gpu,&selector.demand);assert_eq!(demand[1]&768,768);assert_eq!(demand[5]&768,512);assert_eq!(demand[9]&768,0);
   assert_eq!(demand[0],2);assert_eq!(demand[4],2);assert_eq!(demand[8],0);
   assert_eq!(read(&gpu,&selector.fallback),[0,2,0],"orthographic camera retains budget priority over shadow-only root");
   assert_eq!(read(&gpu,&selector.counters)[0],2);
   consumers.reverse();
  }
 }
 #[test]
 #[ignore="exclusive GPU two-dimensional frontier fixture"]
 fn gpu_wavefront_dispatches_second_row() {
  let gpu=Gpu::new().unwrap();let (nodes,trees)=fixture(32769);let empty=depth(&gpu,0.);
  let selector=Selector::with_work_capacity(&gpu,&nodes,&trees,65538,1,32769).unwrap();
  let views=[crate::part_wind::AnimationView {view_projection:Mat4::IDENTITY,viewport:[128;2]}];
  let mut enc=gpu.device.create_command_encoder(&Default::default());
  selector.union(&gpu,&mut enc,&views,0.01,65538,&empty).unwrap();gpu.queue.submit([enc.finish()]);
  let counts=read(&gpu,&selector.counters);assert_eq!(counts[0],65538);assert_eq!(counts[11],0);
  assert!(read(&gpu,&selector.fallback).iter().all(|&f|f==0));
  let mut hits=vec![0;32769];
  for r in read(&gpu,&selector.records).chunks_exact(4) {hits[r[0]as usize]+=1;}
  assert!(hits.iter().all(|&n|n==2));
 }
 #[test]
 #[ignore="exclusive GPU wavefront fixture"]
 fn gpu_wavefront_matches_serial_cut_and_recovers_queue_overflow() {
  let gpu=Gpu::new().unwrap();let (_,trees)=fixture(193);let empty=depth(&gpu,0.);
  let mut nodes=vec![Node {sphere:[0.,0.,0.5,0.3],error:[0.3,0.,0.,0.],links:[1,21,0,2]}];
  for _ in 0..4 {
   let i=nodes.len()as u32;
   nodes.push(Node {sphere:[0.,0.,0.5,0.2],error:[0.1,0.,0.,0.],links:[i+1,i+5,i,0]});
   for j in i+1..i+5 {nodes.push(Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,j+1,j,0]});}
  }
  let views=[crate::part_wind::AnimationView {view_projection:Mat4::IDENTITY,viewport:[128;2]}];
  let serial=Selector::new(&gpu,&nodes,&trees,4096,4096).unwrap();
  let mut enc=gpu.device.create_command_encoder(&Default::default());
  serial.first(&gpu,&mut enc,Mat4::IDENTITY,Mat4::IDENTITY,[128;2],0.01,false,&empty).unwrap();
  gpu.queue.submit([enc.finish()]);
  let expected=read(&gpu,&serial.counters)[0]as usize;assert_eq!(expected,193*16);
  let mut reference=read(&gpu,&serial.records).chunks_exact(4).take(expected).map(|r|r.to_vec()).collect::<Vec<_>>();reference.sort();
  for work_capacity in [4096,193] {
   let selector=Selector::with_work_capacity(&gpu,&nodes,&trees,4096,4096,work_capacity).unwrap();
   assert_eq!(selector.levels,3);
   for iteration in 0..3 {
    let coarse=iteration==1;
    let mut enc=gpu.device.create_command_encoder(&Default::default());
    selector.union(&gpu,&mut enc,&views,if coarse{10000.}else{0.01},4096,&empty).unwrap();
    gpu.queue.submit([enc.finish()]);
    let counts=read(&gpu,&selector.counters);let fallback=read(&gpu,&selector.fallback);let direct=read(&gpu,&selector.direct);
    let mut actual=read(&gpu,&selector.records).chunks_exact(4).take(counts[0]as usize).filter(|r|fallback[r[0]as usize]==0).map(|r|r.to_vec()).collect::<Vec<_>>();actual.sort();
    if coarse {
     assert_eq!(counts[0],0);assert_eq!(counts[7],193);assert_eq!(counts[11],0);
     assert!(fallback.iter().all(|&f|f==0));assert!(direct.iter().all(|&d|d==1));
    } else {
     let valid=reference.iter().filter(|r|fallback[r[0]as usize]==0).cloned().collect::<Vec<_>>();
     assert_eq!(actual,valid);assert!(direct.iter().all(|&d|d==0));
     if work_capacity==4096 {assert_eq!(actual,reference);assert_eq!(counts[11],0);assert_eq!(counts[15],772);}
     else {assert!(counts[11]>0);assert!(fallback.contains(&3));}
    }
   }
  }
 }
 #[test]
 #[ignore="exclusive GPU budget fixture"]
 fn gpu_union_budget_prioritizes_complete_near_cuts() {
  let gpu=Gpu::new().unwrap();let (nodes,mut trees)=fixture(1025);
  // Put far trees first in storage. A queue racing to fill would favor them.
  for t in &mut trees {t.rows[2][3]=-50.;}
  trees[1024].rows[2][3]=-2.;
  let vp=Mat4::perspective_infinite_reverse_rh(1.,1.,0.05);
  let views=[crate::part_wind::AnimationView {view_projection:vp,viewport:[128;2]}];
  let selector=Selector::new(&gpu,&nodes,&trees,512,512).unwrap();let hzb=depth(&gpu,0.);
  let residency=crate::foliage_residency::Residency::new(&gpu,&selector,2).unwrap();
  for _ in 0..3 {
   let mut enc=gpu.device.create_command_encoder(&Default::default());
   selector.union(&gpu,&mut enc,&views,0.0001,2,&hzb).unwrap();
   residency.encode(&mut enc);gpu.queue.submit([enc.finish()]);
   let counts=read(&gpu,&selector.counters);let f=read(&gpu,&selector.fallback);
   assert_eq!(counts[0],2);assert!(f[..1024].iter().all(|&v|v==2));assert_eq!(f[1024],0);
   let records=read(&gpu,&selector.records);assert_eq!(records[0],1024);assert_eq!(records[4],1024);
   let stats=read(&gpu,&residency.counters);assert_eq!(stats[4],0);assert_eq!(stats[6],2);
  }
 }
 #[test]
 #[ignore="exclusive GPU fixture"]
 fn gpu_shared_selection_recovers_disocclusion_and_overflow() {
  let gpu=Gpu::new().unwrap();let(n,t)=fixture(1025);let empty=depth(&gpu,0.);let blocked=depth(&gpu,0.95);
  for (capacity,deferred_capacity,history,current_blocked,error,expected) in [
   (4096,4096,false,false,1.,2050),(4096,4096,true,false,1.,2050),
   (4096,4096,true,true,1.,0),(4096,1,true,false,1.,2050),
   (4096,4096,false,false,1000.,1025),(5,4096,true,false,1.,2050)] {
   let selector=Selector::new(&gpu,&n,&t,capacity,deferred_capacity).unwrap();
   let mut enc=gpu.device.create_command_encoder(&Default::default());
   selector.first(&gpu,&mut enc,Mat4::IDENTITY,Mat4::IDENTITY,[128;2],error,history,&blocked).unwrap();
   selector.second(&gpu,&mut enc,if current_blocked {&blocked}else{&empty});
   let residency=crate::foliage_residency::Residency::new(&gpu,&selector,4096).unwrap();
   residency.encode(&mut enc);gpu.queue.submit([enc.finish()]);
   let c=read(&gpu,&selector.counters);let f=read(&gpu,&selector.fallback);let records=read(&gpu,&selector.records);
   let slots=read(&gpu,&residency.slots);let mappings=read(&gpu,&residency.request_slots);
   for (r,&slot) in records.chunks_exact(4).take(c[0].min(capacity)as usize).zip(&mappings) {
    if f[r[0]as usize]!=0 {assert_eq!(slot,NONE);}
    else {assert!(slot<4096);assert_eq!(&slots[slot as usize*8..][..4],r);assert_eq!(slots[slot as usize*8+6],1);}
   }
   assert_eq!(c[0],expected);assert_eq!(c[12],expected.min(capacity).div_ceil(64));
   if history && deferred_capacity>1 {assert_eq!(c[1],1025);}
   if deferred_capacity==1 {assert!(c[2]>0);}
   let mut selected=vec![Vec::new();t.len()];
   for r in records.chunks_exact(4).take(c[0].min(capacity)as usize) {
    assert_eq!(r[3],17);assert_eq!(r[2],n[r[1]as usize].links[2]);selected[r[0]as usize].push(r[1]);
   }
   for (id,cut) in selected.iter_mut().enumerate() {
    cut.sort();
    if f[id]!=0 {assert!(capacity<expected);continue;}
    if current_blocked {assert!(cut.is_empty());}
    else if error>100. {assert_eq!(*cut,vec![0]);}else{assert_eq!(*cut,vec![1,2]);}
   }
   eprintln!("selection capacity={capacity} deferred={deferred_capacity} history={history} blocked={current_blocked}: records={} nodes={} fallback_instances={}",c[0],c[4],f.iter().filter(|&&v|v!=0).count());
  }
 }
}
