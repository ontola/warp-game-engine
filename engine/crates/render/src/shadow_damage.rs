//! GPU shadow tile damage from exact source/voxel caster snapshots.
//! The normal shared GPU-forest sun map uses this producer and masked raster.
//! The consumer must clear and redraw every marked tile, retaining the others.
use crate::Gpu;
use glam::Mat4;

pub(crate) fn mask_layout(gpu:&Gpu)->wgpu::BindGroupLayout {
    gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow tile raster mask"),entries:&[wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT|wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}}]})
}
pub(crate) const MASK_BYTES:u64=512;
#[repr(C)]
#[derive(Clone,Copy,bytemuck::Pod,bytemuck::Zeroable)]
struct State {vp:[f32;16],counts:[u32;4],control:[u32;4]}

pub(crate) struct Damage {
    pub mask:wgpu::Buffer,
    pub raster_bind:wgpu::BindGroup,
    uniform:wgpu::Buffer,
    bind:wgpu::BindGroup,
    reset:wgpu::ComputePipeline,
    track:wgpu::ComputePipeline,
    clear:wgpu::RenderPipeline,
    clear_bind:wgpu::BindGroup,
    counts:[u32;4],
    key:Option<([u32;16],u64,u32)>,
}
impl Damage {
    /// Counts: source instances, voxel instances, forest branch slots, first
    /// source branch owner. Scene bindings stay stable across asset relocation.
    pub fn new(gpu:&Gpu,sources:&wgpu::Buffer,volumes:&wgpu::Buffer,choices:&wgpu::Buffer,counts:[u32;4])->Result<Self,String> {
        let total=u64::from(counts[0])+u64::from(counts[1]);
        if total==0 || total.div_ceil(64)>u64::from(gpu.limits.max_compute_workgroups_per_dimension)
            || total*128>u64::from(gpu.limits.max_storage_buffer_binding_size)
            || u64::from(counts[0])*80>sources.size() || u64::from(counts[1])*96>volumes.size()
            || counts[2]>16384 || counts[2]>counts[1]
            || counts[3].checked_add(counts[2]).is_none_or(|end|end>counts[0])
            || (counts[2]>0 && (16384+u64::from(counts[2]))*4>choices.size()) {
            return Err("shadow damage capacity or caster bindings invalid".into());
        }
        crate::shaders::checked(gpu,|| {
            let buffer=|label,size,usage|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage,mapped_at_creation:false});
            let uniform=buffer("shadow damage state",96,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
            let mask=buffer("shadow damage tiles",MASK_BYTES,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_SRC);
            let history=buffer("shadow caster history",total*128,wgpu::BufferUsages::STORAGE);
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow damage"),entries:&(0..6).map(|binding|wgpu::BindGroupLayoutEntry {
                binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding<4}},has_dynamic_offset:false,min_binding_size:None},
            }).collect::<Vec<_>>()});
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow damage"),layout:&layout,entries:&[&uniform,sources,volumes,choices,&history,&mask].iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>()});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("shadow damage"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("shadow damage"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow_damage.wgsl").into())});
            let pipeline=|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            let clear_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow damage clear"),entries:&[wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::VERTEX,count:None,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}}]});
            let clear_bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow damage clear"),layout:&clear_layout,entries:&[wgpu::BindGroupEntry {binding:0,resource:mask.as_entire_binding()}]});
            let clear_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("shadow damage clear"),bind_group_layouts:&[Some(&clear_layout)],immediate_size:0});
            let clear_module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("shadow damage clear"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow_damage_clear.wgsl").into())});
            let clear=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:Some("shadow damage clear"),layout:Some(&clear_pl),vertex:wgpu::VertexState {module:&clear_module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},primitive:Default::default(),
                depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Always),stencil:Default::default(),bias:Default::default()}),
                multisample:Default::default(),fragment:None,multiview_mask:None,cache:gpu.pipeline_cache(),
            });
            let raster_bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow tile raster mask"),layout:&mask_layout(gpu),entries:&[wgpu::BindGroupEntry {binding:0,resource:mask.as_entire_binding()}]});
            Self {raster_bind,clear,clear_bind,mask,uniform,bind,reset:pipeline("reset"),track:pipeline("track"),counts,key:None}
        })
    }
    /// Run after source/voxel materialization. `epoch` covers asset/material,
    /// alpha/LOD policy, non-instance deformation and target recreation. Wind
    /// poses themselves are detected on GPU, not via this coarse invalidation.
    /// The owner must invalidate after an abandoned submission / failed redraw.
    pub fn encode(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,vp:Mat4,epoch:u64,size:u32)->Result<(),String> {
        if size<64 || size%64!=0 || !vp.is_finite() || vp.determinant().abs()<1e-20 {return Err("invalid shadow damage projection".into());}
        let key=(vp.to_cols_array().map(f32::to_bits),epoch,size);
        let force=self.key!=Some(key);
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&State {vp:vp.to_cols_array(),counts:self.counts,control:[u32::from(force),size,0,0]}));
        let mut pass=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("shadow damage"),..Default::default()});
        pass.set_bind_group(0,&self.bind,&[]);pass.set_pipeline(&self.reset);pass.dispatch_workgroups(2,1,1);
        pass.set_pipeline(&self.track);pass.dispatch_workgroups((self.counts[0]+self.counts[1]).div_ceil(64),1,1);
        self.key=Some(key);Ok(())
    }
    /// Clear only damaged tiles before re-rasterizing them. The caller must
    /// ensure source AND voxel raster are masked to those tiles; clearing alone
    /// does not make a complete cache without both masked raster consumers.
    pub fn clear_tiles(&self,enc:&mut wgpu::CommandEncoder,depth:&wgpu::TextureView) {
        let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("shadow damaged tile clear"),color_attachments:&[],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
            view:depth,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}),stencil_ops:None,
        }),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
        pass.set_pipeline(&self.clear);pass.set_bind_group(0,&self.clear_bind,&[]);pass.draw(0..6,0..4096);
    }
    pub fn invalidate(&mut self){self.key=None;}
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn shader_validates() {
        let raster=crate::grass::raster_source();
        let cull=crate::frame::cull_source();let voxel=crate::voxel_forest::shader_source();
        for src in [include_str!("shaders/shadow_damage.wgsl"),include_str!("shaders/shadow_damage_clear.wgsl"),include_str!("shaders/voxel_splat_raster.wgsl"),raster.as_str(),cull.as_str(),voxel.as_str()] {
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();}
    }
    #[test]
    #[ignore="exclusive shadow damage GPU fixture; also runs on software DX12"]
    fn tracks_old_and_new_tiles_and_epoch_without_cpu_caster_selection() {
        use wgpu::util::DeviceExt;
        let gpu=Gpu::new().unwrap();
        let upload=|data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("damage fixture"),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
        let mut source=crate::scene::GpuInstance::zeroed();source.sphere=[-0.5,0.,0.5,0.04];source.cluster_count=1;
        let mut volume=crate::foliage_volume_data::GpuTree::zeroed();volume.bounds_min=[0.45,-0.04,0.45,0.];volume.bounds_max=[0.55,0.04,0.55,0.];volume.meta[2]=1;
        let sources=upload(bytemuck::bytes_of(&source));let volumes=upload(bytemuck::bytes_of(&volume));let choices=upload(&[0;4]);
        let mut damage=Damage::new(&gpu,&sources,&volumes,&choices,[1,1,0,0]).unwrap();
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("damage read"),size:MASK_BYTES,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let depth=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("damage depth oracle"),size:wgpu::Extent3d {width:64,height:64,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
        let depth_view=depth.create_view(&Default::default());
        let depth_read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("damage depth read"),size:64*64*4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut run=|vp,epoch| {
            let mut enc=gpu.device.create_command_encoder(&Default::default());damage.encode(&gpu,&mut enc,vp,epoch,64).unwrap();enc.copy_buffer_to_buffer(&damage.mask,0,&read,0,MASK_BYTES);
            {let _pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("oracle retained depth"),color_attachments:&[],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&depth_view,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(0.25),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});}
            damage.clear_tiles(&mut enc,&depth_view);
            enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&depth,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::DepthOnly},wgpu::TexelCopyBufferInfo {buffer:&depth_read,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(256),rows_per_image:Some(64)}},wgpu::Extent3d {width:64,height:64,depth_or_array_layers:1});
            let submission=gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::channel();read.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx.send(r).unwrap();});
            gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).unwrap();rx.recv().unwrap().unwrap();
            let words=bytemuck::cast_slice::<u8,u32>(&read.slice(..).get_mapped_range().unwrap()).to_vec();read.unmap();
            let(tx,rx)=std::sync::mpsc::channel();depth_read.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx.send(r).unwrap();});gpu.wait();rx.recv().unwrap().unwrap();
            {let bytes=depth_read.slice(..).get_mapped_range().unwrap();let depths:&[f32]=bytemuck::cast_slice(&bytes);for (i,&z) in depths.iter().enumerate(){assert_eq!(z,if words[i>>5]&(1u32<<(i&31))!=0 {1.}else{0.25},"only dirty tile depth may be cleared");}}
            depth_read.unmap();words
        };
        let dirty=|mask:&[u32],x:u32,y:u32|mask[((y*64+x)>>5)as usize]&(1<<((y*64+x)&31))!=0;
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==u32::MAX));
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==0));
        source.sphere[0]=0.;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        let mask=run(Mat4::IDENTITY,1);assert!(dirty(&mask,16,32)&&dirty(&mask,32,32));assert!(!dirty(&mask,48,32));
        volume.bounds_min[3]=1.;gpu.queue.write_buffer(&volumes,0,bytemuck::bytes_of(&volume));
        let mask=run(Mat4::IDENTITY,1);assert!(dirty(&mask,48,32));assert!(!dirty(&mask,16,32));
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==0));
        source.sphere[3]=-1.;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));assert!(dirty(&run(Mat4::IDENTITY,1),32,32));
        assert!(run(Mat4::IDENTITY,2).iter().all(|&w|w==u32::MAX));
        assert!(run(Mat4::from_translation(glam::Vec3::X*0.1),2).iter().all(|&w|w==u32::MAX));
    }
    #[test]
    #[ignore="small GPU dirty-bound oracle; software DX12 supported"]
    fn dirty_bounds_match_brute_force_tiles() {
        use wgpu::util::DeviceExt;
        let gpu=Gpu::new().unwrap();
        let mut rects=Vec::<[u32;4]>::new();
        for y in 0..64 {for x in 0..64 {rects.push([x,y,x,y]);}}
        let mut seed=42u32;
        for _ in 0..4096 {
            let mut next=||{seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);(seed>>16)&63};
            let (x,y,z,w)=(next(),next(),next(),next());rects.push([x.min(z),y.min(w),x.max(z),y.max(w)]);
        }
        let src=format!("{}\n{}",include_str!("shaders/shadow_damage_query.wgsl"),r#"
@group(0) @binding(0) var<uniform> shadow_damage_words:array<vec4<u32>,32>;
@group(0) @binding(1) var<storage,read> rects:array<vec4<u32>>;
@group(0) @binding(2) var<storage,read_write> result:array<vec2<u32>>;
@compute @workgroup_size(64) fn check(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&rects) {return;}
    let r=rects[id.x];
    let low=vec3<f32>((f32(r.x)+.5)/32.-1.,1.-(f32(r.w)+.5)/32.,.25);
    let high=vec3<f32>((f32(r.z)+.5)/32.-1.,1.-(f32(r.y)+.5)/32.,.75);
    let identity=mat4x4<f32>(vec4<f32>(1.,0.,0.,0.),vec4<f32>(0.,1.,0.,0.),vec4<f32>(0.,0.,1.,0.),vec4<f32>(0.,0.,0.,1.));
    result[id.x]=vec2<u32>(u32(shadow_damage_rect(r.xy,r.zw)),u32(shadow_damage_bounds(identity,4096.,low,high)));
}
"#);
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("dirty bounds oracle"),source:wgpu::ShaderSource::Wgsl(src.into())});
        let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("dirty bounds oracle"),layout:None,module:&module,entry_point:Some("check"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
        let buffer=|label,size,usage|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage,mapped_at_creation:false});
        let mask=buffer("query mask",512,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
        let input=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("query rectangles"),contents:bytemuck::cast_slice(&rects),usage:wgpu::BufferUsages::STORAGE});
        let output=buffer("query output",rects.len()as u64*8,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC);
        let read=buffer("query read",output.size(),wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST);
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("query oracle"),layout:&pipeline.get_bind_group_layout(0),entries:&[&mask,&input,&output].iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>()});
        for pattern in 0..4 {
            let words:Vec<u32>=(0..128).map(|i|match pattern {0=>0,1=>u32::MAX,2=>if i%13==0 {1u32<<(i%32)}else{0},_=>if i%2==0 {0x80000001}else{0}}).collect();
            gpu.queue.write_buffer(&mask,0,bytemuck::cast_slice(&words));
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups((rects.len()as u32).div_ceil(64),1,1);}
            enc.copy_buffer_to_buffer(&output,0,&read,0,read.size());gpu.queue.submit([enc.finish()]);
            let(tx,rx)=std::sync::mpsc::channel();read.slice(..).map_async(wgpu::MapMode::Read,move|v|{tx.send(v).unwrap();});gpu.wait();rx.recv().unwrap().unwrap();
            {let data=read.slice(..).get_mapped_range().unwrap();let actual:&[[u32;2]]=bytemuck::cast_slice(&data);
            for (i,r) in rects.iter().enumerate() {
                let expected=(r[1]..=r[3]).any(|y|(r[0]..=r[2]).any(|x|words[(y*2+x/32)as usize]&(1u32<<(x%32))!=0))as u32;
                assert_eq!(actual[i],[expected;2],"pattern {pattern}, rectangle {r:?}");
            }}read.unmap();
        }
    }
    use bytemuck::Zeroable;
}
