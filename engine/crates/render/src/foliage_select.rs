//! Experimental GPU forest selection, independent of the CPU near-tree pool.
//!
//! Immutable preorder recipe trees are shared by any number of instances.
//! Phase one uses historical depth only to defer work. Phase two retests that
//! work against CURRENT depth. No readback or CPU tree shortlist is required.
//! This is a selection fixture/API, not yet connected to source/voxel raster.
//! Consumers must discard records whose instance fallback flag is nonzero and
//! render that instance's resident root instead. This avoids holes AND duplicate
//! partial subtrees on overflow. Each view needs its own Selector/scratch.
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
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {vp:[f32;16],previous_vp:[f32;16],pixels:[f32;4],counts:[u32;4]}

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
    _nodes:wgpu::Buffer,_instances:wgpu::Buffer,uniform:wgpu::Buffer,
    /// vec4(instance ID, node ID, representation ID, generation).
    pub records:wgpu::Buffer,
    deferred:wgpu::Buffer,
    /// count, deferred count, deferred overflow, output overflow, tested,
    /// frustum rejected, current-depth rejected, reserved; indirect args at 32/48.
    pub counters:wgpu::Buffer,
    /// Separate indirect arguments: deferred dispatch at 0, consumer dispatch at 12.
    pub arguments:wgpu::Buffer,
    /// One u32 per instance, reset each frame. See module consumer contract.
    pub fallback:wgpu::Buffer,
    layout:wgpu::BindGroupLayout,first:wgpu::ComputePipeline,second:wgpu::ComputePipeline,finish:wgpu::ComputePipeline,
    count:u32,capacity:u32,deferred_capacity:u32,
    binds:std::sync::Mutex<crate::frame::resource_cache::BindGroups<2>>,
}
impl Selector {
    pub fn new(gpu:&Gpu,nodes:&[Node],instances:&[Instance],capacity:u32,deferred_capacity:u32)->Result<Self,String> {
        validate(nodes,instances,capacity)?;
        if deferred_capacity==0 || instances.len().div_ceil(64)>gpu.device.limits().max_compute_workgroups_per_dimension as usize
            || deferred_capacity.div_ceil(64)>gpu.device.limits().max_compute_workgroups_per_dimension {
            return Err("foliage dispatch capacity exceeds device".into());
        }
        for bytes in [std::mem::size_of_val(nodes)as u64,std::mem::size_of_val(instances)as u64,capacity as u64*16,deferred_capacity as u64*8,instances.len()as u64*4] {
            if bytes>gpu.device.limits().max_storage_buffer_binding_size as u64 {return Err("foliage buffer exceeds device binding limit".into());}
        }
        crate::shaders::checked(gpu,|| {
            let upload=|label,data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:Some(label),contents:data,usage:wgpu::BufferUsages::STORAGE});
            let storage=|label,size|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,
                usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::INDIRECT,mapped_at_creation:false});
            let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage selector view"),size:std::mem::size_of::<Uniform>()as u64,
                usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let entries=(0..8).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,
                ty:if binding==7 {wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}}
                else {wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform} else {wgpu::BufferBindingType::Storage {read_only:binding<3}},has_dynamic_offset:false,min_binding_size:None}}}).collect::<Vec<_>>();
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("foliage GPU selection"),entries:&entries});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("foliage GPU selection"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("foliage GPU selection"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_select.wgsl").into())});
            let make=|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            Self {_nodes:upload("shared foliage hierarchy",bytemuck::cast_slice(nodes)),_instances:upload("persistent foliage instances",bytemuck::cast_slice(instances)),uniform,
                records:storage("selected foliage records",capacity as u64*16),deferred:storage("deferred foliage subtrees",deferred_capacity as u64*8),
                counters:storage("foliage selection counters",64),arguments:storage("foliage indirect arguments",24),fallback:storage("foliage root fallbacks",instances.len()as u64*4),
                first:make("first"),second:make("second"),finish:make("finish"),layout,count:instances.len()as u32,capacity,deferred_capacity,binds:Default::default()}
        })
    }
    fn bind(&self,gpu:&Gpu,hzb:&wgpu::TextureView)->wgpu::BindGroup {
        let buffers=[&self.uniform,&self._nodes,&self._instances,&self.records,&self.deferred,&self.counters,&self.fallback];
        let mut entries=buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>();
        entries.push(wgpu::BindGroupEntry {binding:7,resource:wgpu::BindingResource::TextureView(hzb)});
        self.binds.lock().unwrap().entries(gpu,&self.layout,"foliage selector depth",&entries)
    }
    /// Call once per view/frame; current depth MUST be rebuilt after consuming
    /// first-phase records, before second(). Historical depth must describe the
    /// supplied previous matrix; invalidate it on camera cuts or moved occluders.
    pub fn first(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,vp:Mat4,previous_vp:Mat4,size:[u32;2],error:f32,history_valid:bool,hzb:&wgpu::TextureView)->Result<(),String> {
        if !vp.is_finite() || !previous_vp.is_finite() || size.contains(&0) || !error.is_finite() || error<0. {return Err("invalid foliage selection view".into());}
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&Uniform {vp:vp.to_cols_array(),previous_vp:previous_vp.to_cols_array(),
            pixels:[size[0]as f32,size[1]as f32,error,0.],counts:[self.count,self.capacity,self.deferred_capacity,u32::from(history_valid)]}));
        enc.clear_buffer(&self.counters,0,None);enc.clear_buffer(&self.fallback,0,None);
        let bind=self.bind(gpu,hzb);let mut pass=enc.begin_compute_pass(&Default::default());
        pass.set_bind_group(0,&bind,&[]);pass.set_pipeline(&self.first);pass.dispatch_workgroups(self.count.div_ceil(64),1,1);
        pass.set_pipeline(&self.finish);pass.dispatch_workgroups(1,1,1);drop(pass);
        enc.copy_buffer_to_buffer(&self.counters,32,&self.arguments,0,12);
        enc.copy_buffer_to_buffer(&self.counters,48,&self.arguments,12,12);Ok(())
    }
    /// Appends recovered records. This prototype requires final consumption after
    /// both phases: phase-one depth may seed HZB, but final color selection must
    /// honor fallback flags that phase two can set (discard partial subtrees).
    pub fn second(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,current_hzb:&wgpu::TextureView) {
        let bind=self.bind(gpu,current_hzb);let mut pass=enc.begin_compute_pass(&Default::default());
        pass.set_bind_group(0,&bind,&[]);pass.set_pipeline(&self.second);pass.dispatch_workgroups_indirect(&self.arguments,0);
        pass.set_pipeline(&self.finish);pass.dispatch_workgroups(1,1,1);drop(pass);
        enc.copy_buffer_to_buffer(&self.counters,48,&self.arguments,12,12);
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
   selector.second(&gpu,&mut enc,if current_blocked {&blocked}else{&empty});gpu.queue.submit([enc.finish()]);
   let c=read(&gpu,&selector.counters);let f=read(&gpu,&selector.fallback);let records=read(&gpu,&selector.records);
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
