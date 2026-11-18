//! GPU shadow tile damage from exact source/voxel caster snapshots.
//! The normal shared GPU-forest sun map uses this producer and masked raster.
//! The consumer must clear and redraw every marked tile, retaining the others.
use crate::Gpu;
use glam::Mat4;

pub(crate) fn mask_layout(gpu:&Gpu)->wgpu::BindGroupLayout {
    gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow tile raster mask"),entries:&[wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT|wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}}]})
}
pub(crate) const MASK_BYTES:u64=512;
/// An exact texel permutation only: changed basis, scale or Z must redraw.
fn scroll_offset(old:[u32;16],new:[u32;16],size:u32)->Option<[i32;2]> {
    if size==0 || (0..16).any(|i|i!=12 && i!=13 && old[i]!=new[i]) {return None;}
    let mut shift=[0;2];
    for axis in 0..2 {
        let delta=(f64::from(f32::from_bits(new[12+axis]))-f64::from(f32::from_bits(old[12+axis])))
            *f64::from(size)*if axis==0 {0.5}else{-0.5};
        if !delta.is_finite() || delta!=delta.round() || delta.abs()>=f64::from(size) {return None;}
        shift[axis]=delta as i32;
    }
    Some(shift)
}
/// Cached sun maps sharing one caster history: the shared map (slot 0) and
/// up to eight clipmap levels for each of four views.
pub(crate) const MAX_MAPS:usize=33;
const STATE_BYTES:usize=32+MAX_MAPS*96;
const SCROLL_STRIDE:u64=256;
/// Changed-caster counters after the masks (source rows 0..8, voxel rows 0..8).
const COUNTER_BYTES:u64=64;

/// One cached map this frame. `depth` (texture, array layer) lets an exact
/// texel translation scroll the retained depth instead of redrawing it.
pub(crate) struct MapRequest<'a> {
    pub slot:usize,
    pub vp:Mat4,
    pub size:u32,
    pub depth:Option<(&'a wgpu::Texture,u32)>,
    /// false: keep the retained depth and only collect this frame's damage
    /// (a level redrawn on a stagger). Granted only while the projection is
    /// unchanged and the slot holds valid depth; encode_maps says which
    /// requests it turned into draws.
    pub defer:bool,
}

struct Slot {
    key:Option<([u32;16],u64,u32)>,
    /// Damage collected over deferred frames, in the key's texel grid.
    pending:bool,
    raster_bind:wgpu::BindGroup,
    clear_bind:wgpu::BindGroup,
}

pub(crate) struct Damage {
    /// MAX_MAPS masks of MASK_BYTES each, slot order.
    pub mask:wgpu::Buffer,
    uniform:wgpu::Buffer,
    scroll_uniform:wgpu::Buffer,
    bind:wgpu::BindGroup,
    reset:wgpu::ComputePipeline,
    track:wgpu::ComputePipeline,
    clear:wgpu::RenderPipeline,
    scroll:wgpu::RenderPipeline,
    scroll_layout:wgpu::BindGroupLayout,
    /// Per map size: the scratch copy of the layer being scrolled.
    scroll_scratch:Vec<(u32,wgpu::Texture,wgpu::TextureView)>,
    slots:Vec<Slot>,
    counts:[u32;4],
    /// KERN_SUN_DAMAGE_REPORT=1: last frame's masks, printed as dirty tile
    /// counts per slot on the next frame (blocking; diagnostics only).
    report:Option<(wgpu::Buffer,Vec<usize>)>,
    /// Last encode_maps: per request, whether it redrew in full and its scroll (diagnostic).
    pub last_force:Vec<(bool,[i32;2])>,
}
impl Damage {
    /// Counts: source instances, voxel instances, forest branch slots, first
    /// source branch owner. Scene bindings stay stable across asset relocation.
    pub fn new(gpu:&Gpu,sources:&wgpu::Buffer,volumes:&wgpu::Buffer,choices:&wgpu::Buffer,counts:[u32;4])->Result<Self,String> {
        let total=u64::from(counts[0])+u64::from(counts[1]);
        if total==0 || total.div_ceil(64)>u64::from(gpu.limits.max_compute_workgroups_per_dimension)
            || total*128>u64::from(gpu.limits.max_storage_buffer_binding_size)
            || u64::from(counts[0])*80>sources.size() || u64::from(counts[1])*96>volumes.size()
            || counts[2]>counts[1]
            || counts[3].checked_add(counts[2]).is_none_or(|end|end>counts[0])
            || (counts[2]>0 && u64::from(counts[2])*8>choices.size()) {
            return Err("shadow damage capacity or caster bindings invalid".into());
        }
        crate::shaders::checked(gpu,|| {
            let buffer=|label,size,usage|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage,mapped_at_creation:false});
            let uniform=buffer("shadow damage state",STATE_BYTES as u64,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
            let scroll_uniform=buffer("shadow scroll state",SCROLL_STRIDE*MAX_MAPS as u64,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST);
            let mask=buffer("shadow damage tiles",MASK_BYTES*MAX_MAPS as u64+COUNTER_BYTES,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST);
            let history=buffer("shadow caster history",total*128,wgpu::BufferUsages::STORAGE);
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow damage"),entries:&(0..6).map(|binding|wgpu::BindGroupLayoutEntry {
                binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding<4}},has_dynamic_offset:false,min_binding_size:None},
            }).collect::<Vec<_>>()});
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow damage"),layout:&layout,entries:&[&uniform,sources,volumes,choices,&history,&mask].iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>()});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("shadow damage"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("shadow damage"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow_damage.wgsl").into())});
            let terrain_lod=[("TERRAIN_LOD_DAMAGE",if crate::switches::SUN_TERRAIN_LOD_DAMAGE.on() {1.0} else {0.0})];
            let pipeline=|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:wgpu::PipelineCompilationOptions {constants:&terrain_lod,..Default::default()},cache:gpu.pipeline_cache()});
            let clear_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow damage clear"),entries:&[wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::VERTEX,count:None,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}}]});
            let clear_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("shadow damage clear"),bind_group_layouts:&[Some(&clear_layout)],immediate_size:0});
            let clear_module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("shadow damage clear"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow_damage_clear.wgsl").into())});
            let clear=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:Some("shadow damage clear"),layout:Some(&clear_pl),vertex:wgpu::VertexState {module:&clear_module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},primitive:Default::default(),
                depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Always),stencil:Default::default(),bias:Default::default()}),
                multisample:Default::default(),fragment:None,multiview_mask:None,cache:gpu.pipeline_cache(),
            });
            let raster_layout=mask_layout(gpu);
            let slice=|slot:usize|wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer:&mask,offset:slot as u64*MASK_BYTES,size:std::num::NonZeroU64::new(MASK_BYTES)});
            let slots=(0..MAX_MAPS).map(|slot|Slot {key:None,pending:false,
                raster_bind:gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow tile raster mask"),layout:&raster_layout,entries:&[wgpu::BindGroupEntry {binding:0,resource:slice(slot)}]}),
                clear_bind:gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow damage clear"),layout:&clear_layout,entries:&[wgpu::BindGroupEntry {binding:0,resource:slice(slot)}]}),
            }).collect();
            let scroll_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("shadow scroll"),entries:&[
                wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
                wgpu::BindGroupLayoutEntry {binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            ]});
            let scroll_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("shadow scroll"),bind_group_layouts:&[Some(&scroll_layout)],immediate_size:0});
            let scroll_module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("shadow scroll"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow_damage_scroll.wgsl").into())});
            let scroll=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:Some("shadow scroll"),layout:Some(&scroll_pl),vertex:wgpu::VertexState {module:&scroll_module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},primitive:Default::default(),
                depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Always),stencil:Default::default(),bias:Default::default()}),
                multisample:Default::default(),fragment:Some(wgpu::FragmentState {module:&scroll_module,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[]}),multiview_mask:None,cache:gpu.pipeline_cache(),
            });
            Self {scroll,scroll_layout,scroll_scratch:Vec::new(),scroll_uniform,clear,mask,uniform,bind,reset:pipeline("reset"),track:pipeline("track"),slots,counts,report:None,last_force:Vec::new()}
        })
    }
    /// Run after source/voxel materialization. `epoch` covers asset/material,
    /// alpha/LOD policy, non-instance deformation and target recreation. Wind
    /// poses themselves are detected on GPU, not via this coarse invalidation.
    /// The owner must invalidate after an abandoned submission / failed redraw.
    #[cfg(test)]
    pub fn encode(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,vp:Mat4,epoch:u64,size:u32)->Result<(),String> {
        self.encode_maps(gpu,enc,epoch,&[MapRequest {slot:0,vp,size,depth:None,defer:false}]).map(|_|())
    }
    #[cfg(test)]
    pub fn encode_scrolled(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,vp:Mat4,epoch:u64,size:u32,depth:&wgpu::Texture)->Result<(),String> {
        self.encode_maps(gpu,enc,epoch,&[MapRequest {slot:0,vp,size,depth:Some((depth,0)),defer:false}]).map(|_|())
    }
    /// Track every caster once and mark damaged tiles in each requested map.
    /// A slot left out of a frame loses its retained depth (the history moves
    /// on without it) and redraws in full when it returns. A deferred map
    /// keeps collecting damage; returns, per request, whether it draws now
    /// (its masked tiles must be cleared and redrawn this frame).
    pub fn encode_maps(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,epoch:u64,maps:&[MapRequest<'_>])->Result<Vec<bool>,String> {
        static REPORT:std::sync::OnceLock<bool>=std::sync::OnceLock::new();
        let report=*REPORT.get_or_init(||crate::switches::SUN_DAMAGE_REPORT.on());
        if let Some((staging,slots))=self.report.take() {
            let (tx,rx)=std::sync::mpsc::channel();
            staging.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});
            gpu.wait();
            if rx.recv().is_ok_and(|r|r.is_ok()) {
                let words:Vec<u32>=bytemuck::cast_slice(&staging.slice(..).get_mapped_range().unwrap()).to_vec();
                let dirty:Vec<String>=slots.iter().map(|&s|format!("{s}:{}",words[s*128..s*128+128].iter().map(|w|w.count_ones()).sum::<u32>())).collect();
                let counters=&words[MAX_MAPS*128..MAX_MAPS*128+16];
                eprintln!("sun damage tiles of 4096 per slot: {}; changed sources by row {:?}, voxels by row {:?}",dirty.join(" "),&counters[..8],&counters[8..]);
            }
        }
        let mut state=vec![0u8;STATE_BYTES];
        let mut write=|at:usize,words:&[u32]|state[at..at+4*words.len()].copy_from_slice(bytemuck::cast_slice(words));
        let mut requested=[false;MAX_MAPS];
        let mut any_force=false;let mut scrolls=Vec::new();let mut drawn=Vec::with_capacity(maps.len());self.last_force.clear();
        for map in maps {
            let (vp,size)=(map.vp,map.size);
            if map.slot>=MAX_MAPS || requested[map.slot] {return Err("invalid shadow damage slot".into());}
            if size<64 || size%64!=0 || !vp.is_finite() || vp.determinant().abs()<1e-20 {return Err("invalid shadow damage projection".into());}
            requested[map.slot]=true;
            let key=(vp.to_cols_array().map(f32::to_bits),epoch,size);
            let offset=self.slots[map.slot].key.and_then(|old|if old.1==epoch && old.2==size {scroll_offset(old.0,key.0,size)}else{None});
            let offset=offset.filter(|s|*s==[0,0] || map.depth.is_some());
            let draw=!map.defer || offset!=Some([0,0]);
            // Collected damage is in the old grid: a scroll would misplace it.
            let offset=offset.filter(|s|*s==[0,0] || !self.slots[map.slot].pending);
            let force=offset.is_none();let shift=offset.unwrap_or([0,0]);
            any_force|=force;
            drawn.push(draw);self.last_force.push((force,shift));
            self.slots[map.slot].pending=!draw;
            let control=[u32::from(force),size,shift[0]as u32,shift[1]as u32];
            let at=32+map.slot*96;
            write(at,&vp.to_cols_array().map(f32::to_bits));write(at+64,&control);write(at+80,&[1,u32::from(draw),0,0]);
            gpu.queue.write_buffer(&self.scroll_uniform,map.slot as u64*SCROLL_STRIDE,bytemuck::cast_slice(&control));
            if shift!=[0,0] {scrolls.push((map.slot,map.depth.unwrap(),size));}
            self.slots[map.slot].key=Some(key);
        }
        for (slot,done) in requested.iter().enumerate() {if !done {self.slots[slot].key=None;self.slots[slot].pending=false;}}
        let count=maps.iter().map(|m|m.slot+1).max().unwrap_or(0);
        write(0,&self.counts);write(16,&[count as u32,u32::from(any_force),0,0]);
        gpu.queue.write_buffer(&self.uniform,0,&state);
        if report {enc.clear_buffer(&self.mask,MASK_BYTES*MAX_MAPS as u64,Some(COUNTER_BYTES));}
        let mut pass=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("shadow damage"),..Default::default()});
        pass.set_bind_group(0,&self.bind,&[]);pass.set_pipeline(&self.reset);pass.dispatch_workgroups((count as u32*128).div_ceil(64).max(1),1,1);
        pass.set_pipeline(&self.track);pass.dispatch_workgroups((self.counts[0]+self.counts[1]).div_ceil(64),1,1);
        drop(pass);
        if report {
            let staging=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("shadow damage report"),size:self.mask.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            enc.copy_buffer_to_buffer(&self.mask,0,&staging,0,self.mask.size());
            self.report=Some((staging,maps.iter().map(|m|m.slot).collect()));
        }
        for (slot,(depth,layer),size) in scrolls {
            if depth.width()!=size || depth.height()!=size || layer>=depth.depth_or_array_layers() || depth.format()!=wgpu::TextureFormat::Depth32Float {return Err("invalid shadow scroll target".into());}
            let scratch=match self.scroll_scratch.iter().position(|s|s.0==size) {
                Some(i)=>i,
                None=>{
                    let texture=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("shadow scroll scratch"),size:wgpu::Extent3d {width:size,height:size,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
                    let view=texture.create_view(&Default::default());
                    self.scroll_scratch.push((size,texture,view));self.scroll_scratch.len()-1
                }
            };
            let (_,texture,view)=&self.scroll_scratch[scratch];
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("shadow scroll"),layout:&self.scroll_layout,entries:&[
                wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer:&self.scroll_uniform,offset:slot as u64*SCROLL_STRIDE,size:std::num::NonZeroU64::new(16)})},
                wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(view)},
            ]});
            enc.copy_texture_to_texture(wgpu::TexelCopyTextureInfo {texture:depth,mip_level:0,origin:wgpu::Origin3d {x:0,y:0,z:layer},aspect:wgpu::TextureAspect::All},
                texture.as_image_copy(),wgpu::Extent3d {width:size,height:size,depth_or_array_layers:1});
            let target=depth.create_view(&wgpu::TextureViewDescriptor {label:Some("shadow scroll layer"),dimension:Some(wgpu::TextureViewDimension::D2),base_array_layer:layer,array_layer_count:Some(1),..Default::default()});
            let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("shadow depth scroll"),color_attachments:&[],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&target,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
            pass.set_pipeline(&self.scroll);pass.set_bind_group(0,&bind,&[]);pass.draw(0..3,0..1);
        }
        Ok(drawn)
    }
    /// The tile mask of `slot` for masked cull and raster consumers.
    pub fn raster_bind(&self,slot:usize)->&wgpu::BindGroup {&self.slots[slot].raster_bind}
    /// Clear only damaged tiles before re-rasterizing them. The caller must
    /// ensure source AND voxel raster are masked to those tiles; clearing alone
    /// does not make a complete cache without both masked raster consumers.
    pub fn clear_slot(&self,enc:&mut wgpu::CommandEncoder,slot:usize,depth:&wgpu::TextureView) {
        let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("shadow damaged tile clear"),color_attachments:&[],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
            view:depth,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}),stencil_ops:None,
        }),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
        pass.set_pipeline(&self.clear);pass.set_bind_group(0,&self.slots[slot].clear_bind,&[]);pass.draw(0..6,0..4096);
    }
    pub fn clear_tiles(&self,enc:&mut wgpu::CommandEncoder,depth:&wgpu::TextureView) {self.clear_slot(enc,0,depth);}
    #[cfg(test)]
    pub fn invalidate(&mut self){for slot in &mut self.slots {slot.key=None;slot.pending=false;}}
    pub fn invalidate_slot(&mut self,slot:usize){if let Some(s)=self.slots.get_mut(slot) {s.key=None;s.pending=false;}}
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn shader_validates() {
        let raster=crate::grass::raster_source();
        let cull=crate::frame::cull_source();let voxel=crate::voxel_forest::shader_source();
        for src in [include_str!("shaders/shadow_damage.wgsl"),include_str!("shaders/shadow_damage_clear.wgsl"),include_str!("shaders/shadow_damage_scroll.wgsl"),include_str!("shaders/voxel_splat_raster.wgsl"),raster.as_str(),cull.as_str(),voxel.as_str()] {
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
        let sources=upload(bytemuck::bytes_of(&source));let volumes=upload(bytemuck::bytes_of(&volume));let choices=upload(&[0;8]);
        let mut damage=Damage::new(&gpu,&sources,&volumes,&choices,[1,1,1,0]).unwrap();
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
        gpu.queue.write_buffer(&choices,0,bytemuck::bytes_of(&1u32));
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==0),"camera choice must not damage shadow tiles");
        gpu.queue.write_buffer(&choices,4,bytemuck::bytes_of(&1u32));
        let mask=run(Mat4::IDENTITY,1);assert!(dirty(&mask,16,32)&&dirty(&mask,48,32),"shadow choice uses actual one-slot stride");
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==0));
        source.sphere[0]=0.;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        let mask=run(Mat4::IDENTITY,1);assert!(dirty(&mask,16,32)&&dirty(&mask,32,32));assert!(!dirty(&mask,48,32));
        volume.bounds_min[3]=1.;gpu.queue.write_buffer(&volumes,0,bytemuck::bytes_of(&volume));
        let mask=run(Mat4::IDENTITY,1);assert!(dirty(&mask,48,32));assert!(!dirty(&mask,16,32));
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==0));
        for (mode,changed) in [(2.,true),(3.,true),(3.,false),(2.,true),(1.,true)] {
            volume.bounds_min[3]=mode;gpu.queue.write_buffer(&volumes,0,bytemuck::bytes_of(&volume));
            assert_eq!(dirty(&run(Mat4::IDENTITY,1),48,32),changed,"shadow-only roots must invalidate old/new cached tiles; camera-only roots are not casters");
        }
        // A procedural terrain patch re-tessellated by camera LOD is the same
        // surface: no damage. Becoming a terrain patch is still a change.
        source.flags=crate::procedural_terrain::PROCEDURAL|(crate::scene::GeometryCategory::Terrain as u32)<<crate::scene::INSTANCE_CATEGORY_SHIFT;
        gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));assert!(dirty(&run(Mat4::IDENTITY,1),32,32));
        source.sphere[0]=-0.5;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        assert!(run(Mat4::IDENTITY,1).iter().all(|&w|w==0),"terrain LOD patch moves must not damage cached sun tiles");
        source.sphere[0]=0.;source.flags=0;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));run(Mat4::IDENTITY,1);
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
    #[test]
    #[ignore="multi-map shadow damage GPU fixture; lavapipe supported"]
    fn one_history_marks_every_map_and_scrolls_array_layers() {
        use wgpu::util::DeviceExt;
        let gpu=Gpu::new().unwrap();
        let upload=|data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("damage fixture"),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
        let mut source=crate::scene::GpuInstance::zeroed();source.sphere=[-0.5,0.,0.5,0.04];source.cluster_count=1;
        let sources=upload(bytemuck::bytes_of(&source));let volumes=upload(&[0;96]);let choices=upload(&[0;8]);
        let mut damage=Damage::new(&gpu,&sources,&volumes,&choices,[1,1,0,0]).unwrap();
        // A clipmap-like array: level 1 of two 128-texel layers, half the scale.
        let array=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("clipmap oracle"),size:wgpu::Extent3d {width:128,height:128,depth_or_array_layers:2},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let layer=array.create_view(&wgpu::TextureViewDescriptor {dimension:Some(wgpu::TextureViewDimension::D2),base_array_layer:1,array_layer_count:Some(1),..Default::default()});
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("damage read"),size:MASK_BYTES*3,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let texels=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("layer read"),size:128*128*4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let half=Mat4::from_scale(glam::Vec3::new(0.5,0.5,1.));
        let run=|damage:&mut Damage,second:Option<Mat4>,seed:Option<f32>|->(Vec<u32>,Vec<f32>) {
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            let mut maps=vec![MapRequest {slot:0,vp:Mat4::IDENTITY,size:64,depth:None,defer:false}];
            if let Some(vp)=second {maps.push(MapRequest {slot:2,vp,size:128,depth:Some((&array,1)),defer:false});}
            damage.encode_maps(&gpu,&mut enc,1,&maps).unwrap();
            if let Some(depth)=seed {let _pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("seed layer"),color_attachments:&[],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&layer,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(depth),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});}
            enc.copy_buffer_to_buffer(&damage.mask,0,&read,0,MASK_BYTES*3);
            enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&array,mip_level:0,origin:wgpu::Origin3d {x:0,y:0,z:1},aspect:wgpu::TextureAspect::DepthOnly},wgpu::TexelCopyBufferInfo {buffer:&texels,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(512),rows_per_image:Some(128)}},wgpu::Extent3d {width:128,height:128,depth_or_array_layers:1});
            gpu.queue.submit([enc.finish()]);
            let (tx,rx)=std::sync::mpsc::channel();let tx2=tx.clone();
            read.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx.send(r).unwrap();});texels.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx2.send(r).unwrap();});
            gpu.wait();rx.recv().unwrap().unwrap();rx.recv().unwrap().unwrap();
            let words=bytemuck::cast_slice::<u8,u32>(&read.slice(..).get_mapped_range().unwrap()).to_vec();read.unmap();
            let depth=bytemuck::cast_slice::<u8,f32>(&texels.slice(..).get_mapped_range().unwrap()).to_vec();texels.unmap();
            (words,depth)
        };
        let slot=|w:&[u32],s:usize|w[s*128..s*128+128].to_vec();
        let dirty=|mask:&[u32],x:u32,y:u32|mask[((y*64+x)>>5)as usize]&(1<<((y*64+x)&31))!=0;
        let (w,_)=run(&mut damage,Some(half),Some(0.5));
        assert!(slot(&w,0).iter().all(|&v|v==u32::MAX)&&slot(&w,2).iter().all(|&v|v==u32::MAX),"first frame redraws every map");
        let (w,_)=run(&mut damage,Some(half),None);
        assert!(slot(&w,0).iter().all(|&v|v==0)&&slot(&w,2).iter().all(|&v|v==0),"unchanged casters keep every map");
        source.sphere[0]=0.;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        let (w,_)=run(&mut damage,Some(half),None);
        let (a,b)=(slot(&w,0),slot(&w,2));
        assert!(dirty(&a,16,32)&&dirty(&a,32,32)&&!dirty(&a,48,32),"shared map: old and new bounds");
        assert!(dirty(&b,24,32)&&dirty(&b,32,32)&&!dirty(&b,16,32)&&!dirty(&b,48,32),"clipmap level: the same caster at its own scale");
        // One texel right in a 128 map: the layer scrolls, the exposed column clears.
        let moved=Mat4::from_translation(glam::Vec3::new(2./128.,0.,0.))*half;
        let (w,depth)=run(&mut damage,Some(moved),None);
        assert!(slot(&w,0).iter().all(|&v|v==0));
        let b=slot(&w,2);assert!(dirty(&b,0,10)&&!dirty(&b,10,10),"only the exposed edge tiles");
        assert_eq!(depth[10*128],1.,"exposed texel cleared");assert_eq!(depth[10*128+5],0.5,"retained texel kept");
        // A map left out for a frame lost track of casters: it redraws in full.
        run(&mut damage,None,None);
        let (w,_)=run(&mut damage,Some(moved),None);
        assert!(slot(&w,2).iter().all(|&v|v==u32::MAX)&&slot(&w,0).iter().all(|&v|v==0));
    }
    #[test]
    #[ignore="deferred shadow damage GPU fixture; lavapipe supported"]
    fn deferred_maps_collect_damage_until_drawn() {
        use wgpu::util::DeviceExt;
        let gpu=Gpu::new().unwrap();
        let upload=|data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("damage fixture"),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
        let mut source=crate::scene::GpuInstance::zeroed();source.sphere=[-0.5,0.,0.5,0.04];source.cluster_count=1;
        let sources=upload(bytemuck::bytes_of(&source));let volumes=upload(&[0;96]);let choices=upload(&[0;8]);
        let mut damage=Damage::new(&gpu,&sources,&volumes,&choices,[1,1,0,0]).unwrap();
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("damage read"),size:MASK_BYTES*2,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut run=|vp:Mat4,defer:bool|->(bool,Vec<u32>) {
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            let drawn=damage.encode_maps(&gpu,&mut enc,1,&[MapRequest {slot:1,vp,size:64,depth:None,defer}]).unwrap();
            enc.copy_buffer_to_buffer(&damage.mask,0,&read,0,MASK_BYTES*2);gpu.queue.submit([enc.finish()]);
            let(tx,rx)=std::sync::mpsc::channel();read.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx.send(r).unwrap();});gpu.wait();rx.recv().unwrap().unwrap();
            let words=bytemuck::cast_slice::<u8,u32>(&read.slice(..).get_mapped_range().unwrap())[128..].to_vec();read.unmap();
            (drawn[0],words)
        };
        let dirty=|mask:&[u32],x:u32,y:u32|mask[((y*64+x)>>5)as usize]&(1<<((y*64+x)&31))!=0;
        assert!(run(Mat4::IDENTITY,true).0,"no valid depth yet: a deferral becomes a draw");
        assert_eq!(run(Mat4::IDENTITY,false),(true,vec![0;128]));
        source.sphere[0]=0.;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        let (drawn,mask)=run(Mat4::IDENTITY,true);assert!(!drawn);assert!(dirty(&mask,16,32)&&dirty(&mask,32,32));
        source.sphere[0]=0.5;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        let (drawn,mask)=run(Mat4::IDENTITY,true);assert!(!drawn);
        assert!(dirty(&mask,16,32)&&dirty(&mask,32,32)&&dirty(&mask,48,32),"deferred frames accumulate");
        let (drawn,mask)=run(Mat4::IDENTITY,false);assert!(drawn);assert_eq!(mask,vec![0;128],"a draw restarts the mask");
        source.sphere[0]=0.;gpu.queue.write_buffer(&sources,0,bytemuck::bytes_of(&source));
        assert!(!run(Mat4::IDENTITY,true).0);
        let moved=Mat4::from_translation(glam::Vec3::new(2./64.,0.,0.));
        let (drawn,mask)=run(moved,true);
        assert!(drawn&&mask.iter().all(|&w|w==u32::MAX),"a moved map draws; damage collected in the old grid forces a full redraw");
    }
    #[test]
    fn scroll_requires_exact_xy_and_unchanged_depth() {
        let bits=|m:Mat4|m.to_cols_array().map(f32::to_bits);
        let old=bits(Mat4::IDENTITY);
        assert_eq!(scroll_offset(old,old,4096),Some([0,0]));
        assert_eq!(scroll_offset(old,bits(Mat4::from_translation(glam::Vec3::new(2./4096.,-4./4096.,0.))),4096),Some([1,2]));
        for delta in [glam::Vec3::new(1./4096.,0.,0.),glam::Vec3::new(0.,0.,0.01),glam::Vec3::new(2.,0.,0.)] {
            assert_eq!(scroll_offset(old,bits(Mat4::from_translation(delta)),4096),None);
        }
        assert_eq!(scroll_offset(old,bits(Mat4::from_rotation_y(0.01)),4096),None);
        let sun=glam::Vec3::new(0.3,0.7,0.2).normalize();
        let a=crate::stability::stable_shadow_depth(glam::Vec3::ZERO,sun,192.,4096,[-512.,512.]);
        for x in [0.1,1.,8.,30.] {
            let b=crate::stability::stable_shadow_depth(glam::Vec3::X*x,sun,192.,4096,[-512.,512.]);
            assert!(scroll_offset(bits(a),bits(b),4096).is_some(),"canonical moving projection");
        }
    }
    #[test]
    fn animated_root_envelope_is_world_space_and_inactive_roots_are_ignored() {
        let root=crate::foliage_select::Instance {rows:[[0.,0.,-2.,10.],[0.,2.,0.,20.],[2.,0.,0.,30.]],scale:[2.,3.,0.,0.],info:[0;4]};
        assert_eq!(crate::foliage_runtime::root_shadow_sphere(root,[1.,2.,3.,4.]),Some([4.,24.,32.,11.]));
        let mut inactive=root;inactive.info[3]=1;
        assert_eq!(crate::foliage_runtime::root_shadow_sphere(inactive,[1.,2.,3.,4.]),None);
    }
    #[test]
    #[ignore="small exact depth scroll GPU oracle; software DX12 supported"]
    fn scrolled_depth_preserves_exact_samples_and_clears_exposed_edges() {
        use wgpu::util::DeviceExt;
        let gpu=Gpu::new().unwrap();
        let upload=|bytes:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("scroll fixture"),contents:bytes,usage:wgpu::BufferUsages::STORAGE});
        let sources=upload(&[0;80]);let volumes=upload(&[0;96]);let choices=upload(&[0;4]);
        let mut damage=Damage::new(&gpu,&sources,&volumes,&choices,[1,1,0,0]).unwrap();
        let depth=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("scroll oracle"),size:wgpu::Extent3d {width:64,height:64,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
        let target=depth.create_view(&Default::default());
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("scroll read"),size:64*64*4,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let src=r#"
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
 let p=array<vec2<f32>,3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));return vec4<f32>(p[i],0.,1.);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {return f32(u32(p.y)*64u+u32(p.x)+1u)/8192.;}
"#;
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("distinct depth samples"),source:wgpu::ShaderSource::Wgsl(src.into())});
        let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {label:Some("distinct depth samples"),layout:None,vertex:wgpu::VertexState {module:&module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},primitive:Default::default(),depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Always),stencil:Default::default(),bias:Default::default()}),multisample:Default::default(),fragment:Some(wgpu::FragmentState {module:&module,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[]}),multiview_mask:None,cache:gpu.pipeline_cache()});
        for [dx,dy] in [[0,0],[1,0],[-1,0],[0,1],[0,-1],[3,-5],[-7,13],[32,32],[-63,-63]] {
            damage.invalidate();
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            damage.encode_scrolled(&gpu,&mut enc,Mat4::IDENTITY,1,64,&depth).unwrap();
            {let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("seed depth pattern"),color_attachments:&[],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&target,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(1.),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});pass.set_pipeline(&pipeline);pass.draw(0..3,0..1);}
            gpu.queue.submit([enc.finish()]);gpu.wait();
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            let vp=Mat4::from_translation(glam::Vec3::new(dx as f32/32.,-dy as f32/32.,0.));
            damage.encode_scrolled(&gpu,&mut enc,vp,1,64,&depth).unwrap();damage.clear_tiles(&mut enc,&target);
            enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&depth,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::DepthOnly},wgpu::TexelCopyBufferInfo {buffer:&read,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(256),rows_per_image:Some(64)}},depth.size());
            gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::channel();read.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx.send(r).unwrap();});gpu.wait();rx.recv().unwrap().unwrap();
            {let data=read.slice(..).get_mapped_range().unwrap();let values:&[f32]=bytemuck::cast_slice(&data);
            for y in 0..64i32 {for x in 0..64i32 {
                let (ox,oy)=(x-dx,y-dy);let expected=if (0..64).contains(&ox)&&(0..64).contains(&oy){(oy*64+ox+1)as f32/8192.}else{1.};
                assert_eq!(values[(y*64+x)as usize],expected,"shift {dx},{dy}, pixel {x},{y}");
            }}}read.unmap();
        }
    }
    use bytemuck::Zeroable;
}
