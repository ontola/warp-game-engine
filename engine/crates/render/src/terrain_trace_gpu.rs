use super::*;
use crate::Gpu;
use bytemuck::{Pod,Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]#[derive(Clone,Copy,Pod,Zeroable)]
struct World {origin_cell:[f32;4],dims:[u32;4]}
#[repr(C)]#[derive(Clone,Copy,Pod,Zeroable)]
struct Header {world:World,levels:[[u32;4];MAX_LEVELS],info:[u32;4]}
#[repr(C)]#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuRay {origin_min:[f32;4],direction_max:[f32;4]}
#[repr(C)]#[derive(Clone,Copy,Pod,Zeroable)]
struct ResultBytes {info:[u32;4],position_t:[f32;4],normal:[f32;4]}

// WGSL TerrainHierarchy has 16-byte struct alignment even though its trailing
// runtime vec2 array has an 8-byte stride. A single cell therefore needs 320,
// not 312, bytes to satisfy the shader's minimum binding size.
fn packed_size(bounds:u32)->usize {
    (std::mem::size_of::<Header>()+bounds as usize*8).div_ceil(16)*16
}

fn header(hf:&Heightfield,coverage:Coverage)->Result<Header,String>{
    let (w,h)=dimensions(hf)?;let mut p=Header::zeroed();p.world=World{origin_cell:[hf.origin[0],hf.origin[1],hf.cell,UNKNOWN_HEIGHT],dims:[hf.w,hf.h,w,h]};
    let (mut w,mut h,mut offset,mut level)=(w,h,0,0);
    loop {p.levels[level]=[offset,w,h,0];offset+=w*h;level+=1;if w==1&&h==1{break;}w=(w/2).max(1);h=(h/2).max(1);}
    p.info=[level as u32,offset,coverage as u32,0];Ok(p)
}
impl Hierarchy {
    /// Exact GPU header + mip order. Useful for startup CPU upload/reference;
    /// the GPU builder computes the same bytes without uploading CPU bounds.
    pub fn packed_bytes(&self)->Vec<u8>{
        let h=header(&self.field,self.coverage).expect("validated hierarchy");
        let mut out=bytemuck::bytes_of(&h).to_vec();
        for l in &self.levels{out.extend_from_slice(bytemuck::cast_slice(&l.bounds));}
        out.resize(packed_size(h.info[1]),0);out
    }
}

/// One immutable world field shared by every view and light. Construct once,
/// encode_build once before querying. No sun or camera enters this resource.
pub struct GpuHierarchy {
    pub buffer:wgpu::Buffer,
    pub height_view:wgpu::TextureView,
    _owned_heights:Option<wgpu::Texture>,
    header:Header,
    build_pipeline:wgpu::ComputePipeline,
    build_groups:Vec<wgpu::BindGroup>,
    query_pipeline:wgpu::ComputePipeline,
    query_layout:wgpu::BindGroupLayout,
}
fn uniform()->wgpu::BindingType {wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}}
fn storage(read_only:bool)->wgpu::BindingType {wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only},has_dynamic_offset:false,min_binding_size:None}}
fn texture()->wgpu::BindingType {wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false}}
fn layout(gpu:&Gpu,label:&str,items:&[(u32,wgpu::BindingType)])->wgpu::BindGroupLayout {
    gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some(label),entries:&items.iter().map(|&(binding,ty)|wgpu::BindGroupLayoutEntry{binding,visibility:wgpu::ShaderStages::COMPUTE,ty,count:None}).collect::<Vec<_>>()})
}
fn bind(gpu:&Gpu,layout:&wgpu::BindGroupLayout,items:&[(u32,wgpu::BindingResource<'_>)])->wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("terrain trace"),layout,entries:&items.iter().map(|(binding,resource)|wgpu::BindGroupEntry{binding:*binding,resource:resource.clone()}).collect::<Vec<_>>()})
}
fn pipeline(gpu:&Gpu,layout:&wgpu::BindGroupLayout,source:&str,entry:&str)->wgpu::ComputePipeline{
    let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some(entry),source:wgpu::ShaderSource::Wgsl(source.into())});
    let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some(entry),bind_group_layouts:&[Some(layout)],immediate_size:0});
    gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache: gpu.pipeline_cache()})
}
impl GpuHierarchy {
    pub fn new(gpu:&Gpu,hf:&Heightfield,coverage:Coverage,existing_height_view:Option<&wgpu::TextureView>)->Result<Self,String>{
        let header=header(hf,coverage)?;let size=packed_size(header.info[1])as u64;
        if size>gpu.limits.max_storage_buffer_binding_size as u64||size>gpu.limits.max_buffer_size{return Err(format!("terrain hierarchy {size} bytes exceeds device buffer budget"));}
        if hf.w.max(hf.h)>gpu.limits.max_texture_dimension_2d{return Err("terrain heightfield exceeds device texture dimensions".into());}
        let buffer=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("terrain minmax hierarchy"),size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        gpu.queue.write_buffer(&buffer,0,bytemuck::bytes_of(&header));
        let owned=if existing_height_view.is_none(){Some(gpu.device.create_texture_with_data(&gpu.queue,&wgpu::TextureDescriptor{label:Some("terrain trace raw heights"),size:wgpu::Extent3d{width:hf.w,height:hf.h,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]},wgpu::util::TextureDataOrder::LayerMajor,bytemuck::cast_slice(&hf.heights)))}else{None};
        let height_view=existing_height_view.cloned().unwrap_or_else(||owned.as_ref().unwrap().create_view(&Default::default()));
        let build_layout=layout(gpu,"terrain hierarchy build",&[(0,uniform()),(1,texture()),(2,storage(false))]);
        let build_pipeline=pipeline(gpu,&build_layout,include_str!("shaders/terrain_trace_build.wgsl"),"build");
        let mut build_groups=Vec::new();
        for level in 0..header.info[0]{let params=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("terrain mip level"),contents:bytemuck::cast_slice(&[level,0,0,0]),usage:wgpu::BufferUsages::UNIFORM});
            build_groups.push(bind(gpu,&build_layout,&[(0,params.as_entire_binding()),(1,wgpu::BindingResource::TextureView(&height_view)),(2,buffer.as_entire_binding())]));}
        let query_layout=layout(gpu,"terrain trace query",&[(0,uniform()),(1,texture()),(3,storage(true)),(4,storage(false)),(24,storage(true))]);
        let query_pipeline=pipeline(gpu,&query_layout,&format!("{}\n{}",super::SHADER,include_str!("shaders/terrain_trace_query.wgsl")),"query_terrain");
        Ok(Self{buffer,height_view,_owned_heights:owned,header,build_pipeline,build_groups,query_pipeline,query_layout})
    }
    pub fn bytes(&self)->u64 {self.buffer.size()}
    pub fn level_count(&self)->u32 {self.header.info[0]}
    pub fn encode_build(&self,encoder:&mut wgpu::CommandEncoder){
        for (level,group) in self.build_groups.iter().enumerate(){
            // Each dispatch ends before the next level reads its output.
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("terrain minmax mip"),timestamp_writes:None});
            pass.set_pipeline(&self.build_pipeline);pass.set_bind_group(0,group,&[]);
            let m=self.header.levels[level];pass.dispatch_workgroups(m[1].div_ceil(8),m[2].div_ceil(8),1);
        }
    }
    pub fn encode_queries(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,rays:&[Ray],budget:u32,upward:bool)->Result<GpuBatch,String>{
        if rays.is_empty()||rays.len()>8192{return Err("terrain query batch must contain 1..8192 rays".into());}
        let raw:Vec<_>=rays.iter().map(|r|GpuRay{origin_min:[r.origin.x,r.origin.y,r.origin.z,r.t_min],direction_max:[r.direction.x,r.direction.y,r.direction.z,r.t_max]}).collect();
        let input=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("terrain query rays"),contents:bytemuck::cast_slice(&raw),usage:wgpu::BufferUsages::STORAGE});
        let params=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("terrain query budget"),contents:bytemuck::cast_slice(&[rays.len()as u32,budget.min(MAX_NODE_BUDGET),u32::from(upward),0]),usage:wgpu::BufferUsages::UNIFORM});
        let size=(rays.len()*std::mem::size_of::<ResultBytes>())as u64;
        let output=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("terrain query result"),size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("terrain query readback"),size,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let group=bind(gpu,&self.query_layout,&[(0,params.as_entire_binding()),(1,wgpu::BindingResource::TextureView(&self.height_view)),(3,input.as_entire_binding()),(4,output.as_entire_binding()),(24,self.buffer.as_entire_binding())]);
        {let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("terrain world queries"),timestamp_writes:None});pass.set_pipeline(&self.query_pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups((rays.len()as u32).div_ceil(32),1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&read,0,size);Ok(GpuBatch{read,count:rays.len()})
    }
    pub fn read_packed(&self,gpu:&Gpu)->Vec<u8>{
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("terrain hierarchy proof"),size:self.buffer.size(),usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(&self.buffer,0,&read,0,self.buffer.size());gpu.queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();let out=read.slice(..).get_mapped_range().unwrap().to_vec();read.unmap();out
    }
}
pub struct GpuBatch {read:wgpu::Buffer,count:usize}
impl GpuBatch {pub fn read(self,gpu:&Gpu)->Vec<TraceResult>{
    self.read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
    let results={let view=self.read.slice(..).get_mapped_range().unwrap();let bytes:&[ResultBytes]=bytemuck::cast_slice(&view);
        bytes[..self.count].iter().map(|b|TraceResult{status:match b.info[0]{0=>Status::Clear,1=>Status::Hit,3=>Status::AboveTerrainBounds,_=>Status::Unknown},reason:match b.info[1]{0=>Reason::None,1=>Reason::OutsideCoverage,2=>Reason::UnknownHeight,3=>Reason::Budget,5=>Reason::Stack,_=>Reason::InvalidRay},distance:b.position_t[3],position:Vec3::from_slice(&b.position_t),normal:Vec3::from_slice(&b.normal),visited_nodes:b.info[2],triangle_tests:b.info[3]}).collect()};
    self.read.unmap();results
}}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn packed_layout_matches_wgsl(){assert_eq!(std::mem::size_of::<Header>(),304);assert_eq!(std::mem::offset_of!(Header,levels),32);assert_eq!(std::mem::offset_of!(Header,info),288);assert_eq!(std::mem::size_of::<ResultBytes>(),48);assert_eq!(std::mem::size_of::<GpuRay>(),32);}
    #[test]fn one_cell_packed_buffer_covers_wgsl_minimum_and_zero_padding(){
        let hf=Heightfield{origin:[0.,0.],cell:1.,w:2,h:2,heights:vec![0.,1.,2.,3.]};
        let h=Hierarchy::build(&hf).unwrap();let bytes=h.packed_bytes();
        assert_eq!(bytes.len(),320);assert_eq!(bytes.len(),packed_size(1));
        assert_eq!(&bytes[304..312],bytemuck::cast_slice::<_,u8>(&[[0.0f32,3.0f32]]));
        assert_eq!(&bytes[312..],&[0;8]);
        for count in [1,2,5,21,341]{assert_eq!(packed_size(count)%16,0);assert!(packed_size(count)>=304+count as usize*8);}
    }
    #[test]fn hierarchy_build_and_trace_shaders_validate(){for source in [include_str!("shaders/terrain_trace_build.wgsl").to_string(),format!("{}\n{}",super::super::SHADER,include_str!("shaders/terrain_trace_query.wgsl"))]{let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();}}
}
