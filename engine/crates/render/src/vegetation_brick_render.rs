//! Native renderer for disjoint world-owned vegetation optical bricks.
//!
//! Providers publish four dense VolumeCell mips and a generation status per
//! brick. Screen-tile scheduling and exact piecewise-constant cell traversal
//! replace per-pixel root scans. This bounded prototype has no DLSS motion,
//! streaming, or authority to remove source geometry for unpublished bricks.
use crate::{foliage_volume_data::{GpuCell, GpuLevel}, Gpu};
use bytemuck::{Pod, Zeroable};
use std::{sync::Mutex, time::Duration};
use wgpu::util::DeviceExt;

#[path = "vegetation_brick_lod.rs"]
mod lod;
pub use lod::{BrickMipSelection, MIP_TRANSITION_OCTAVES, brick_projected_error_bound, select_brick_mips};

pub const MAX_GROUPS: usize = 16;
pub const LEVELS_PER_GROUP: usize = 4;
pub const MAX_CELLS: u32 = 262_144;
const TILE: u32 = 16;
const COUNTER_BYTES: u64 = 176;

#[derive(Clone, Copy, Debug)]
struct Inventory { groups: u32, cells: u32, max_steps: u32 }
fn ulp(value:f32)->f32 {
    let positive=value.abs();
    f32::from_bits(positive.to_bits()+1)-positive
}
fn inventory(levels: &[GpuLevel]) -> Result<Inventory, String> {
    if levels.is_empty() || levels.len() % LEVELS_PER_GROUP != 0 || levels.len() / LEVELS_PER_GROUP > MAX_GROUPS {
        return Err("vegetation bricks require 1..16 groups with four mips each".into());
    }
    let mut cells = 0u64;
    let mut steps = 0u32;
    for (group, mips) in levels.chunks_exact(LEVELS_PER_GROUP).enumerate() {
        let fine = &mips[0];
        for (mip, level) in mips.iter().enumerate() {
            if !level.origin_cell.iter().all(|x| x.is_finite()) || level.origin_cell[3] <= 0.0
                || level.dims_offset[..3].iter().any(|&x| x == 0 || x > 256) {
                return Err("vegetation brick has invalid cell size, dimensions or origin".into());
            }
            if level.origin_cell[..3] != fine.origin_cell[..3]
                || level.origin_cell[3] != fine.origin_cell[3] * (1u32 << mip) as f32
                || (0..3).any(|axis| fine.dims_offset[axis] % (1 << mip) != 0
                    || level.dims_offset[axis] != fine.dims_offset[axis] >> mip) {
                return Err("vegetation brick mips must exactly partition the same world bounds".into());
            }
            let count = u64::from(level.dims_offset[0]) * u64::from(level.dims_offset[1]) * u64::from(level.dims_offset[2]);
            // Disjoint dense ranges make the reported allocation authoritative.
            if u64::from(level.dims_offset[3]) != cells {
                return Err("vegetation brick dense ranges must be contiguous and nonoverlapping".into());
            }
            cells += count;
            for axis in 0..3 {
                let hi = level.origin_cell[axis] + level.dims_offset[axis] as f32 * level.origin_cell[3];
                if !hi.is_finite() || hi <= level.origin_cell[axis]
                    || level.origin_cell[axis] + level.origin_cell[3] == level.origin_cell[axis] {
                    return Err("vegetation brick grid exceeds supported world precision".into());
                }
            }
        }
        // A straight ray crosses at most nx+ny+nz faces. Four extra visits
        // cover initialization/roundoff at an entering face, not lost density.
        steps += fine.dims_offset[..3].iter().sum::<u32>() + 4;
        for other in levels[..group * LEVELS_PER_GROUP].chunks_exact(LEVELS_PER_GROUP) {
            let other = &other[0];
            let overlaps = [0usize, 2].into_iter().all(|axis| {
                let hi = fine.origin_cell[axis] + fine.dims_offset[axis] as f32 * fine.origin_cell[3];
                let other_hi = other.origin_cell[axis] + other.dims_offset[axis] as f32 * other.origin_cell[3];
                let overlap=hi.min(other_hi)-fine.origin_cell[axis].max(other.origin_cell[axis]);
                // Provider ownership uses integer world-group keys. Repeated
                // origin+extent and integer*spacing can disagree by one ULP.
                // Accept only that arithmetic seam; ray intervals below are
                // clipped to their preceding exit so it cannot double mass.
                let roundoff=2.0*ulp(hi).max(ulp(other_hi));
                overlap>roundoff
            });
            if overlaps { return Err("vegetation brick XZ interiors overlap; optical mass would be integrated twice".into()); }
        }
    }
    if cells > u64::from(MAX_CELLS) { return Err("vegetation brick dense-cell budget exceeds262144".into()); }
    Ok(Inventory { groups: (levels.len() / LEVELS_PER_GROUP) as u32, cells: cells as u32, max_steps: steps })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Config { size_count: [u32; 4], limits: [u32; 4], quality: [f32; 4] }
#[derive(Clone, Debug, Default)]
pub struct VegetationBrickStats {
    pub view: usize,
    pub input_size: [u32; 2],
    pub groups: u32,
    pub expected_generation: u32,
    pub invalid_groups: u32,
    pub active_tiles: u32,
    pub scheduled_pixels: u32,
    pub traced_rays: u32,
    pub max_tile_groups: u32,
    pub max_ray_groups: u32,
    pub max_cell_steps: u32,
    pub max_selected_mip: u32,
    /// Executed selected-mip fine DDA iterations. True empty-region jumps
    /// are counted separately; this includes the old read-only bypass loops.
    pub cell_steps: u64,
    pub occupied_samples: u64,
    /// Nonzero is a failed diagnostic, never an accepted truncated integral.
    pub failed_rays: u32,
    pub step_limit_failures: u32,
    pub coordinate_failures: u32,
    pub progress_failures: u32,
    /// Constructed mode, not a claim that any empty region was encountered.
    pub empty_skip_enabled: bool,
    /// Positive-length selected-mip intervals whose cell reads/optics were
    /// bypassed using an exactly empty ancestor. Not unique world cells.
    pub skipped_empty_cells: u64,
    /// Empty ancestor regions actually traversed, aggregated over all rays.
    pub empty_skip_regions: u64,
    /// Independent true-jump mode; old read skipping retains its own flag.
    pub empty_jump_enabled: bool,
    /// Executed coarse empty-region jumps. Together with cell_steps this is
    /// the actual traversal iteration count, not a reconstructed fine count.
    pub empty_jump_regions: u64,
    /// Uncertain/nonprogressing jump candidates that used the reference DDA.
    pub empty_jump_fallbacks: u64,
    pub smooth_mips_enabled: bool,
    pub lod_pixels: f32,
    pub mip_transition_octaves: f32,
    /// Ray/brick intersections traversed at each finer integration mip; not
    /// unique bricks. A blended intersection also reads its adjacent coarse mip.
    pub traversal_mip_hits: [u64; 4],
    pub blended_brick_hits: u64,
    /// Positive-length fine-grid intervals evaluated with both optical mips.
    pub blended_cell_segments: u64,
    /// Optical-cell loads only; excludes hierarchy reads used by empty jumps.
    pub optical_cell_reads: u64,
    /// Positive-budget intersections whose finest cell is already too large.
    /// Diagnostics cannot authorize source removal for these intersections.
    pub finest_over_budget_hits: u64,
    pub max_finest_projected_pixels: f32,
    pub step_bound: u32,
    /// Shared provider buffers are reported once per field, not per camera.
    pub shared_provider_bytes: u64,
    pub target_bytes: u64,
}
struct Targets {
    size: (u32, u32), layer: wgpu::Texture, layer_view: wgpu::TextureView,
    bind: wgpu::BindGroup, composite: wgpu::BindGroup,
    work: wgpu::Buffer, indirect: wgpu::Buffer, counters: wgpu::Buffer,
    config: wgpu::Buffer, generation: u32, bytes: u64,
    empty_skip_enabled: bool, empty_jump_enabled: bool, smooth_mips_enabled: bool, lod_pixels: f32,
}
pub(crate) struct VegetationBrickRenderer {
    inventory: Inventory,
    empty_skip_enabled: bool,
    empty_jump_enabled: bool,
    smooth_mips_enabled: bool,
    provider: [wgpu::Buffer; 3],
    provider_bytes: u64,
    data_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    tiles: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    targets: Mutex<Vec<Option<Targets>>>,
}
pub(crate) fn shader_source() -> Result<String, String> {
    Ok(format!("{}\n{}", crate::shaders::Snapshot::load()?.shade(), include_str!("shaders/vegetation_bricks.wgsl")))
}
impl VegetationBrickRenderer {
    /// Reuse the reduced voxel scene layout and volume_view layout. The
    /// provider publishes current world normals; no instance transform is read.
    pub(crate) fn new(gpu: &Gpu, scene_layout: &wgpu::BindGroupLayout,
        view_layout: &wgpu::BindGroupLayout, levels: &[GpuLevel],
        level_buffer: &wgpu::Buffer, cells: &wgpu::Buffer, status: &wgpu::Buffer) -> Result<Self, String> {
        let inventory = inventory(levels)?;
        if level_buffer.size() < std::mem::size_of_val(levels) as u64
            || cells.size() < u64::from(inventory.cells) * std::mem::size_of::<GpuCell>() as u64
            || status.size() < u64::from(inventory.groups) * 16 {
            return Err("vegetation brick provider buffers are smaller than their declared grids".into());
        }
        let source = shader_source()?;
        crate::shaders::checked(gpu, || {
            let storage = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
            let data_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vegetation brick data"), entries: &[
                    (0,storage(true)),(1,storage(true)),(2,storage(true)),(3,storage(false)),(4,storage(false)),
                    (5,wgpu::BindingType::StorageTexture { access:wgpu::StorageTextureAccess::WriteOnly,format:wgpu::TextureFormat::Rgba16Float,view_dimension:wgpu::TextureViewDimension::D2 }),
                    (6,wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}),
                ].map(|(binding,ty)|wgpu::BindGroupLayoutEntry {binding,ty,visibility:wgpu::ShaderStages::COMPUTE,count:None}),
            });
            let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:Some("vegetation brick integration"),bind_group_layouts:&[Some(scene_layout),Some(view_layout),Some(&data_layout)],immediate_size:0 });
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("vegetation bricks"),source:wgpu::ShaderSource::Wgsl(source.into())});
            let pipeline = |entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label:Some(entry),layout:Some(&layout),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache() });
            let tiles=pipeline("vegetation_brick_tiles");let integrate=pipeline("vegetation_brick_integrate");
            let composite_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("vegetation brick composite"),entries:&[
                wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None}]});
            let composite_pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("vegetation brick composite"),bind_group_layouts:&[Some(&composite_layout)],immediate_size:0});
            let composite_shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("vegetation brick composite"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/voxel_composite.wgsl").into())});
            let composite=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:Some("vegetation brick composite"),layout:Some(&composite_pipeline_layout),
                vertex:wgpu::VertexState {module:&composite_shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},
                primitive:Default::default(),depth_stencil:None,multisample:Default::default(),
                fragment:Some(wgpu::FragmentState {module:&composite_shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),
                multiview_mask:None,cache:gpu.pipeline_cache(),
            });
            Self {inventory,empty_skip_enabled:std::env::var("KERN_BRICK_EMPTY_SKIP").as_deref()==Ok("1"),empty_jump_enabled:std::env::var("KERN_BRICK_EMPTY_JUMP").as_deref()==Ok("1"),smooth_mips_enabled:std::env::var("KERN_BRICK_SMOOTH_MIPS").as_deref()==Ok("1"),provider:[level_buffer.clone(),cells.clone(),status.clone()],provider_bytes:level_buffer.size()+cells.size()+status.size(),data_layout,composite_layout,tiles,integrate,composite,targets:Mutex::new(Vec::new())}
        })
    }
    /// Diagnostic controls change consumer work only, never provider buffers
    /// or publication. Each rendered target records the mode actually used.
    pub(crate) fn set_empty_skip(&mut self,enabled:bool) {self.empty_skip_enabled=enabled;}
    pub(crate) fn set_empty_jump(&mut self,enabled:bool) {self.empty_jump_enabled=enabled;}
    pub(crate) fn set_smooth_mips(&mut self,enabled:bool) {self.smooth_mips_enabled=enabled;}
    pub(crate) fn set_view_count(&self,count:usize) {
        let mut targets=self.targets.lock().unwrap();targets.truncate(count);targets.resize_with(count,||None);
    }
    fn target(&self,gpu:&Gpu,size:(u32,u32))->Targets {
        let tiles=u64::from(size.0.div_ceil(TILE))*u64::from(size.1.div_ceil(TILE));
        let layer=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("vegetation brick fractional radiance"),
            size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
            format:wgpu::TextureFormat::Rgba16Float,usage:wgpu::TextureUsages::STORAGE_BINDING|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
        let layer_view=layer.create_view(&Default::default());
        let buffer=|label,size,usage|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage,mapped_at_creation:false});
        let work=buffer("vegetation brick tile work",(4+tiles*2)*4,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST);
        let indirect=buffer("vegetation brick indirect",12,wgpu::BufferUsages::INDIRECT|wgpu::BufferUsages::COPY_DST);
        let counters=buffer("vegetation brick counters",COUNTER_BYTES,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST);
        let config=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("vegetation brick config"),contents:bytemuck::bytes_of(&Config {size_count:[size.0,size.1,size.0.div_ceil(TILE),self.inventory.groups],limits:[0,self.inventory.max_steps,tiles as u32,0],quality:[0.0;4]}),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("vegetation brick data"),layout:&self.data_layout,entries:&[
            wgpu::BindGroupEntry {binding:0,resource:self.provider[0].as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:self.provider[1].as_entire_binding()},
            wgpu::BindGroupEntry {binding:2,resource:self.provider[2].as_entire_binding()},wgpu::BindGroupEntry {binding:3,resource:work.as_entire_binding()},
            wgpu::BindGroupEntry {binding:4,resource:counters.as_entire_binding()},wgpu::BindGroupEntry {binding:5,resource:wgpu::BindingResource::TextureView(&layer_view)},
            wgpu::BindGroupEntry {binding:6,resource:config.as_entire_binding()}]});
        let composite=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("vegetation brick composite"),layout:&self.composite_layout,entries:&[wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(&layer_view)}]});
        Targets {size,layer,layer_view,bind,composite,work,indirect,counters,config,generation:0,bytes:u64::from(size.0)*u64::from(size.1)*8+(4+tiles*2)*4+12+COUNTER_BYTES+48,empty_skip_enabled:false,empty_jump_enabled:false,smooth_mips_enabled:false,lod_pixels:0.0}
    }
    /// Provider encode must precede this call in the same ordered queue. The
    /// caller retains source coverage unless publication matches generation.
    /// This method does not support temporal reconstruction or opaque depth.
    pub(crate) fn render(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,index:usize,size:(u32,u32),
        scene:&wgpu::BindGroup,view:&wgpu::BindGroup,hdr:&wgpu::TextureView,generation:u32,lod_pixels:f32,
        times:[Option<(&wgpu::QuerySet,u32)>;3])->Result<(),String> {
        if index>=4 || size.0==0 || size.1==0 || size.0>16384 || size.1>16384 || generation==0 || !lod_pixels.is_finite() || lod_pixels<0.0 {
            return Err("invalid vegetation brick view, generation or projected-cell threshold".into());
        }
        let mut targets=self.targets.lock().unwrap();
        if targets.len()<=index {targets.resize_with(index+1,||None);}
        if !targets[index].as_ref().is_some_and(|target|target.size==size) {targets[index]=Some(self.target(gpu,size));}
        let target=targets[index].as_mut().unwrap();target.generation=generation;
        target.empty_skip_enabled=self.empty_skip_enabled;target.empty_jump_enabled=self.empty_jump_enabled;
        target.smooth_mips_enabled=self.smooth_mips_enabled;target.lod_pixels=lod_pixels;
        let tile_count=size.0.div_ceil(TILE)*size.1.div_ceil(TILE);
        let mode=u32::from(self.empty_skip_enabled)|(u32::from(self.empty_jump_enabled)<<1);
        gpu.queue.write_buffer(&target.config,0,bytemuck::bytes_of(&Config {size_count:[size.0,size.1,size.0.div_ceil(TILE),self.inventory.groups],limits:[generation,self.inventory.max_steps,tile_count,mode],quality:[lod_pixels,if self.smooth_mips_enabled {MIP_TRANSITION_OCTAVES}else{0.0},0.0,0.0]}));
        encoder.clear_buffer(&target.counters,0,None);encoder.clear_buffer(&target.work,0,Some(16));
        // Attachment clear avoids a full-screen invocation of the ray kernel.
        // Its cost belongs to the tile timestamp, including the following bin.
        {let _pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("vegetation brick clear"),
            color_attachments:&[Some(wgpu::RenderPassColorAttachment {view:&target.layer_view,depth_slice:None,resolve_target:None,ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,
            timestamp_writes:times[0].map(|(query_set,i)|wgpu::RenderPassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:None}),occlusion_query_set:None,multiview_mask:None});}
        {let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("vegetation brick tiles"),timestamp_writes:times[0].map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:None,end_of_pass_write_index:Some(i+1)})});
            pass.set_pipeline(&self.tiles);pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,view,&[]);pass.set_bind_group(2,&target.bind,&[]);
            pass.dispatch_workgroups(size.0.div_ceil(TILE).div_ceil(8),size.1.div_ceil(TILE).div_ceil(8),1);}
        encoder.copy_buffer_to_buffer(&target.work,0,&target.indirect,0,12);
        {let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("vegetation brick integrate"),timestamp_writes:times[1].map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
            pass.set_pipeline(&self.integrate);pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,view,&[]);pass.set_bind_group(2,&target.bind,&[]);pass.dispatch_workgroups_indirect(&target.indirect,0);}
        {let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("vegetation brick composite"),
            color_attachments:&[Some(wgpu::RenderPassColorAttachment {view:hdr,depth_slice:None,resolve_target:None,ops:wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,
            timestamp_writes:times[2].map(|(query_set,i)|wgpu::RenderPassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)}),occlusion_query_set:None,multiview_mask:None});
            pass.set_pipeline(&self.composite);pass.set_bind_group(0,&target.composite,&[]);pass.draw(0..3,0..1);}
        Ok(())
    }
    /// Raw premultiplied RGBA16F layer for explicit evidence readback.
    pub(crate) fn layers(&self)->Vec<wgpu::Texture> {
        self.targets.lock().unwrap().iter().filter_map(|x|x.as_ref().map(|x|x.layer.clone())).collect()
    }
    pub(crate) fn diagnostics(&self,gpu:&Gpu)->Result<Vec<VegetationBrickStats>,String> {
        let targets=self.targets.lock().unwrap();let live:Vec<_>=targets.iter().enumerate().filter_map(|(i,t)|t.as_ref().map(|t|(i,t))).collect();
        if live.is_empty() {return Ok(Vec::new());}
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("vegetation brick diagnostics readback"),size:COUNTER_BYTES*live.len() as u64,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        for (slot,(_,target)) in live.iter().enumerate() {encoder.copy_buffer_to_buffer(&target.counters,0,&read,slot as u64*COUNTER_BYTES,COUNTER_BYTES);}
        let submission=gpu.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::sync_channel(1);
        read.slice(..).map_async(wgpu::MapMode::Read,move|result|{let _=tx.send(result);});
        gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(Duration::from_secs(60))}).map_err(|e|format!("vegetation brick diagnostics wait: {e:?}"))?;
        rx.try_recv().map_err(|e|format!("vegetation brick diagnostics callback: {e}"))?.map_err(|e|format!("vegetation brick diagnostics map: {e:?}"))?;
        let values={let bytes=read.slice(..).get_mapped_range().map_err(|e|format!("vegetation brick diagnostics range: {e:?}"))?;
            live.iter().zip(bytes.chunks_exact(COUNTER_BYTES as usize)).map(|((view,target),bytes)| {
                let word=|i:usize|u32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap());
                let wide=|i|u64::from(word(i))|(u64::from(word(i+1))<<32);
                VegetationBrickStats {view:*view,input_size:[target.size.0,target.size.1],groups:self.inventory.groups,expected_generation:target.generation,
                    invalid_groups:word(0),active_tiles:word(1),scheduled_pixels:word(2),traced_rays:word(3),max_tile_groups:word(4),max_ray_groups:word(5),
                    max_cell_steps:word(6),max_selected_mip:word(7),cell_steps:wide(8),occupied_samples:wide(10),failed_rays:word(12),step_bound:self.inventory.max_steps,
                    step_limit_failures:word(13),coordinate_failures:word(14),progress_failures:word(15),
                    empty_skip_enabled:target.empty_skip_enabled,skipped_empty_cells:wide(16),empty_skip_regions:wide(18),
                    empty_jump_enabled:target.empty_jump_enabled,empty_jump_regions:wide(20),empty_jump_fallbacks:wide(22),
                    smooth_mips_enabled:target.smooth_mips_enabled,lod_pixels:target.lod_pixels,
                    mip_transition_octaves:if target.smooth_mips_enabled {MIP_TRANSITION_OCTAVES}else{0.0},
                    traversal_mip_hits:std::array::from_fn(|mip|wide(24+mip*2)),
                    blended_brick_hits:wide(32),blended_cell_segments:wide(34),optical_cell_reads:wide(36),
                    finest_over_budget_hits:wide(38),max_finest_projected_pixels:f32::from_bits(word(40)),
                    shared_provider_bytes:self.provider_bytes,target_bytes:target.bytes}
            }).collect()};read.unmap();Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grids(origins:&[[f32;3]])->Vec<GpuLevel> {
        let mut result=Vec::new();let mut offset=0;
        for origin in origins {for mip in 0..4 {
            let n=8>>mip;result.push(GpuLevel {origin_cell:[origin[0],origin[1],origin[2],0.5*(1<<mip) as f32],dims_offset:[n,n,n,offset]});offset+=n*n*n;
        }}result
    }
    #[test]
    fn disjoint_ownership_and_exact_mips_are_required() {
        let valid=grids(&[[0.0;3],[4.0,0.0,0.0]]);let info=inventory(&valid).unwrap();
        assert_eq!(info.groups,2);assert_eq!(info.cells,1170);assert_eq!(info.max_steps,56);
        assert!(inventory(&grids(&[[0.0;3],[3.99,10.0,0.0]])).is_err());
        let mut wrong=valid.clone();wrong[1].dims_offset[1]=3;assert!(inventory(&wrong).is_err());
        let mut wrong=valid;wrong[4].dims_offset[3]-=1;assert!(inventory(&wrong).is_err());
    }
    #[test]
    fn group_and_cell_budgets_are_hard_limits() {
        let origins:Vec<_>=(0..17).map(|i|[i as f32*4.0,0.0,0.0]).collect();
        assert!(inventory(&grids(&origins)).is_err());
        let mut bad=grids(&[[0.0;3]]);bad[0].origin_cell[0]=f32::NAN;assert!(inventory(&bad).is_err());
        assert_eq!(std::mem::size_of::<GpuCell>(),176);assert_eq!(std::mem::size_of::<Config>(),48);
    }
    // f32 port of the actual shader's slab and DDA arithmetic, including
    // terminal-face policy. The old accumulated-next path is retained only
    // inside this regression to demonstrate the observed fixture failure.
    fn traverse_f32(origin:glam::Vec3,direction:glam::Vec3,lower:glam::Vec3,dims:glam::IVec3,
        cell:f32,limit:f32,accumulated:bool)->Option<Result<(f64,f64,u32),u32>> {
        use glam::{IVec3,Vec3};
        let upper=lower+dims.as_vec3()*cell;let(mut enter,mut exit)=(0.0f32,limit);
        for axis in 0..3 {
            if direction[axis].abs()<1e-12 {
                if origin[axis]<lower[axis] || origin[axis]>=upper[axis] {return None;}
            } else {
                let a=(lower[axis]-origin[axis])/direction[axis];let b=(upper[axis]-origin[axis])/direction[axis];
                enter=enter.max(a.min(b));exit=exit.min(a.max(b));
            }
        }
        if exit<=enter {return None;}
        let at=(origin+direction*enter-lower)/cell;
        let mut coord=at.floor().as_ivec3();
        for axis in 0..3 {if direction[axis]<0.0 && at[axis]==at[axis].floor() {coord[axis]-=1;}}
        coord=coord.clamp(IVec3::ZERO,dims-IVec3::ONE);
        let step=IVec3::from_array(direction.to_array().map(|v|if v>0.0 {1}else if v<0.0 {-1}else{0}));
        let face=|coord:IVec3,axis:usize| {
            if direction[axis].abs()<1e-12 {return 1e30;}
            let boundary=coord[axis]+i32::from(direction[axis]>0.0);
            if !accumulated && (boundary<=0 || boundary>=dims[axis]) {return exit;}
            (lower[axis]+boundary as f32*cell-origin[axis])/direction[axis]
        };
        let mut next=Vec3::from_array(std::array::from_fn(|axis|face(coord,axis)));
        let mut t=enter;let mut integral=0.0f64;let mut steps=0;
        while t<exit {
            if steps>dims.element_sum() as u32+4 {return Some(Err(13));}
            if coord.cmplt(IVec3::ZERO).any() || coord.cmpge(dims).any() {return Some(Err(14));}
            let end=exit.min(next.min_element());
            if end>t {integral+=f64::from(end)-f64::from(t);t=end;}
            let mut crossed=false;
            for axis in 0..3 {if next[axis]<=t {
                coord[axis]+=step[axis];
                next[axis]=if accumulated {next[axis]+cell/direction[axis].abs()}else{face(coord,axis)};
                crossed=true;
            }}
            steps+=1;if !crossed && t<exit {return Some(Err(15));}
        }
        Some(Ok((integral,f64::from(exit)-f64::from(enter),steps)))
    }
    #[test]
    fn actual_fixture_f32_rays_integrate_terminal_cells_through_analytic_exit() {
        use glam::{IVec3,Mat4,Vec3,Vec4};
        // Exact brick-native-a fixture coordinates, dimensions, cameras and
        // reversed-Z ray reconstruction. Both unclipped and actual floor-
        // clipped rays exercise fine and all three coarse cells.
        let cameras=[Vec3::new(0.,3.5,7.),Vec3::new(5.5,0.65,6.),Vec3::new(-10.,7.,-13.),Vec3::new(0.,9.,55.)];
        let cell=0.45f32*8.0/32.0;let edge=0.45f32*8.0;
        let origins=[Vec3::new(-edge,-1.,-edge),Vec3::new(0.,-1.,-edge),Vec3::new(-edge,-1.,0.),Vec3::new(0.,-1.,0.)];
        let mut rays=0;let mut old_failed=0;
        for origin in cameras {
            let inverse=(Mat4::perspective_infinite_reverse_rh(50f32.to_radians(),640.0/360.0,0.05)
                *Mat4::look_at_rh(origin,Vec3::new(0.,0.15,0.),Vec3::Y)).inverse();
            for y in (0..360).step_by(6) {for x in (0..640).step_by(6) {
                let far=inverse*Vec4::new((x as f32+0.5)/640.0*2.0-1.0,1.0-(y as f32+0.5)/360.0*2.0,1e-4,1.0);
                let direction=(far.truncate()/far.w-origin).normalize();
                let floor=if direction.y<0.0 {(-0.015-origin.y)/direction.y}else{1e30};
                for lower in origins {for mip in 0..4 {for limit in [1e30,floor] {
                    let dims=IVec3::new(32,24,32)/(1<<mip);let size=cell*(1<<mip) as f32;
                    let Some(result)=traverse_f32(origin,direction,lower,dims,size,limit,false) else {continue;};
                    let (integral,length,steps)=result.unwrap_or_else(|reason|panic!("fixture pixel{x},{y} camera{origin:?} mip{mip}: failure{reason}"));
                    assert!((integral-length).abs()<1e-12,"all positive tail length must retain density");
                    assert!(steps<=dims.element_sum() as u32+4);rays+=1;
                    old_failed+=usize::from(traverse_f32(origin,direction,lower,dims,size,limit,true).unwrap().is_err());
                }}}
            }}
        }
        assert!(rays>1000,"fixture must exercise real intersecting rays");
        assert!(old_failed>100,"test must reproduce the accumulated-next failure, got{old_failed}");
    }
    fn interval(origin:[f64;3],direction:[f64;3],lower:[f64;3],upper:[f64;3],limit:f64)->Option<(f64,f64)> {
        let(mut enter,mut exit)=(0.0f64,limit);
        for axis in 0..3 {
            if direction[axis].abs()<1e-12 {
                if origin[axis]<lower[axis]||origin[axis]>=upper[axis] {return None;}
            } else {
                let a=(lower[axis]-origin[axis])/direction[axis];let b=(upper[axis]-origin[axis])/direction[axis];
                enter=enter.max(a.min(b));exit=exit.min(a.max(b));
            }
        }
        (exit>enter).then_some((enter,exit))
    }
    // Independent oracle intersects every one of the512 cells; traversal
    // visits only crossed cells. Compare integrated optical depth, not pictures.
    #[test]
    fn dda_preserves_density_at_faces_negative_rays_and_opaque_clips() {
        let sigma=|c:[i32;3]|0.25+f64::from((c[0]+c[1]*3+c[2]*5)%7)*0.2;
        for (origin,direction,limit) in [
            ([-2.0,0.25,0.25],[1.0,0.0,0.0],100.0),
            ([6.0,1.25,1.25],[-1.0,0.0,0.0],100.0),
            ([-1.0,-1.0,-1.0],[1.0,1.0,1.0],100.0),
            ([2.0,2.0,2.0],[-1.0,-1.0,-1.0],0.7),
            ([-2.0,1.1,2.1],[1.0,0.2,-0.1],3.125),
            ([-2.0,1.1,2.1],[1.0,0.2,-0.1],1.0), // opaque before brick
            ([4.0,-1.0,1.0],[0.0,1.0,0.0],100.0), // half-open outer face
        ] {
            let length=direction.iter().map(|x|x*x).sum::<f64>().sqrt();
            let direction=direction.map(|x|x/length);
            let mut expected=0.0;
            for z in 0..8 {for y in 0..8 {for x in 0..8 {
                let c=[x,y,z];let lower=c.map(|v|f64::from(v)*0.5);let upper=lower.map(|v|v+0.5);
                if let Some((a,b))=interval(origin,direction,lower,upper,limit) {expected+=sigma(c)*(b-a);}
            }}}
            let mut actual=0.0;let mut steps=0;
            if let Some((enter,exit))=interval(origin,direction,[0.0;3],[4.0;3],limit) {
                let at: [f64;3]=std::array::from_fn(|axis|(origin[axis]+direction[axis]*enter)/0.5);
                let mut c: [i32;3]=std::array::from_fn(|axis|((at[axis].floor() as i32)-i32::from(direction[axis]<0.0&&at[axis]==at[axis].floor())).clamp(0,7));
                let step=direction.map(|x|if x>0.0 {1}else if x<0.0 {-1}else {0});
                let mut next:[f64;3]=std::array::from_fn(|axis|if step[axis]==0 {f64::INFINITY}else {((f64::from(c[axis]+i32::from(step[axis]>0))*0.5)-origin[axis])/direction[axis]});
                let mut t=enter;
                while t<exit {
                    assert!(steps<28);assert!(c.into_iter().all(|v|(0..8).contains(&v)));
                    let end=exit.min(next.into_iter().fold(f64::INFINITY,f64::min));
                    if end>t {actual+=sigma(c)*(end-t);t=end;}
                    for axis in 0..3 {if next[axis]<=t {c[axis]+=step[axis];next[axis]+=0.5/direction[axis].abs();}}
                    steps+=1;
                }
            }
            assert!((actual-expected).abs()<1e-10,"{origin:?} {direction:?} clip{limit}: {actual} != {expected}");
            // Beer transport is invariant to subdividing a homogeneous
            // interval and cannot gain opacity from a clipped-away cell.
            assert!(((-actual).exp()-(-expected).exp()).abs()<1e-12);
        }
    }
    #[test]
    fn generation_contract_never_accepts_stale_or_failed_publication() {
        let ready=|state:[u32;4],expected:u32|expected!=0&&state[0]==expected&&state[1]==expected&&state[2]==0;
        assert!(ready([7,7,0,123],7));
        for state in [[0,0,0,0],[8,7,0,123],[7,6,0,123],[7,7,2,123],[6,6,0,123]] {assert!(!ready(state,7));}
        assert!(!ready([0;4],0));
    }
    fn exact_empty(cell:&GpuCell)->bool {
        cell.extinction.iter().flatten().all(|v|*v==0.)
            && cell.normals.iter().all(|n|n[3]==0.)
    }
    #[test]
    fn empty_certificate_rejects_nonzero_directions_and_underflowed_mass() {
        let empty=GpuCell::zeroed();assert!(exact_empty(&empty));
        for axis in 0..13 {
            let mut cell=empty;cell.extinction[axis/4][axis%4]=f32::from_bits(1);
            assert!(!exact_empty(&cell),"no nonzero directional coefficient may be skipped");
        }
        // A positive child coefficient can disappear under mip averaging.
        // The actual producer retains its positive-area leaf marker and
        // normalized lobe mass independently of this division by eight.
        let mut parent=empty;parent.extinction[0][0]=f32::from_bits(1)*0.125;
        assert_eq!(parent.extinction[0][0],0.);
        parent.extinction[3][1]=1.;assert!(!exact_empty(&parent));
        parent.extinction[3][1]=0.;parent.normals[3][3]=1.;assert!(!exact_empty(&parent));
        parent=empty;parent.extinction[1][2]=f32::NAN;assert!(!exact_empty(&parent));
        parent=empty;parent.normals[0][3]=f32::NAN;assert!(!exact_empty(&parent));
        assert_eq!(COUNTER_BYTES,44*4,"mip-work totals follow jump counters; no reused error words");
    }
    // F32 reference traversal with a sparse field and all four mip choices.
    // Returning occupied coordinates AND exact endpoint bits makes the test
    // stricter than comparing a final Beer opacity after rounding.
    fn sparse_walk(origin:glam::Vec3,direction:glam::Vec3,limit:f32,mip:usize,
        levels:&[Vec<GpuCell>;4],skip:bool)->(Vec<(usize,u32,u32)>,u32,u32) {
        let (occupied,steps,skipped,_)=sparse_walk_mode(origin,direction,limit,mip,levels,u8::from(skip));
        (occupied,steps,skipped)
    }
    fn sparse_walk_mode(origin:glam::Vec3,direction:glam::Vec3,limit:f32,mip:usize,
        levels:&[Vec<GpuCell>;4],mode:u8)->(Vec<(usize,u32,u32)>,u32,u32,u32) {
        use glam::{IVec3,Vec3};
        let dims=IVec3::splat(8>>mip);let cell=0.1125f32*(1<<mip) as f32;
        let lower=Vec3::new(-0.9,-0.3,-0.9);let upper=lower+dims.as_vec3()*cell;
        let(mut enter,mut exit)=(0.0f32,limit);
        for axis in 0..3 {
            if direction[axis].abs()<1e-12 {
                if origin[axis]<lower[axis]||origin[axis]>=upper[axis] {return (Vec::new(),0,0,0);}
            } else {
                let a=(lower[axis]-origin[axis])/direction[axis];let b=(upper[axis]-origin[axis])/direction[axis];
                enter=enter.max(a.min(b));exit=exit.min(a.max(b));
            }
        }
        if exit<=enter {return (Vec::new(),0,0,0);}
        let at=(origin+direction*enter-lower)/cell;let mut coord=at.floor().as_ivec3();
        for axis in 0..3 {if direction[axis]<0. && at[axis]==at[axis].floor() {coord[axis]-=1;}}
        coord=coord.clamp(IVec3::ZERO,dims-IVec3::ONE);
        let step=IVec3::from_array(direction.to_array().map(|v|if v>0. {1}else if v<0. {-1}else{0}));
        let face=|coord:IVec3,axis:usize| {
            if direction[axis].abs()<1e-12 {return 1e30;}
            let boundary=coord[axis]+i32::from(direction[axis]>0.);
            if boundary<=0||boundary>=dims[axis] {return exit;}
            (lower[axis]+boundary as f32*cell-origin[axis])/direction[axis]
        };
        let index=|c:IVec3,n:i32|(c.x+n*(c.y+n*c.z)) as usize;
        let mut next=Vec3::from_array(std::array::from_fn(|a|face(coord,a)));
        let(mut t,mut steps,mut skipped,mut jumps)=(enter,0,0,0);let mut occupied=Vec::new();
        while t<exit {
            assert!(steps+jumps<(dims.element_sum()+4) as u32);
            assert!(coord.cmpge(IVec3::ZERO).all()&&coord.cmplt(dims).all());
            let ancestor=if mode!=0 {(mip+1..4).rev().find(|&m|exact_empty(&levels[m][index(coord/(1<<(m-mip)),8>>m)]))}else{None};
            if mode&2!=0 {if let Some(m)=ancestor {
                let width=1<<(m-mip);let base=(coord/width)*width;
                let mut end=exit;
                for axis in 0..3 {
                    let mut last=coord;last[axis]=base[axis]+if direction[axis]>0. {width-1}else{0};
                    end=end.min(face(last,axis));
                }
                if end>t && end<=exit {
                    if end==exit {t=end;jumps+=1;continue;}
                    let mut advanced=coord;
                    for axis in 0..3 {
                        if direction[axis].abs()<1e-12 {continue;}
                        let mut lo=0;let mut hi=if step[axis]>0 {base[axis]+width-coord[axis]}else{coord[axis]-base[axis]+1};
                        for _ in 0..4 {
                            if lo>=hi {break;}
                            let middle=(lo+hi+1)/2;let mut before=coord;before[axis]+=(middle-1)*step[axis];
                            if face(before,axis)<=end {lo=middle;}else{hi=middle-1;}
                        }
                        advanced[axis]+=lo*step[axis];
                    }
                    if advanced.cmpge(IVec3::ZERO).all()&&advanced.cmplt(dims).all()&&advanced!=coord {
                        coord=advanced;t=end;next=Vec3::from_array(std::array::from_fn(|a|face(coord,a)));
                        jumps+=1;continue;
                    }
                }
            }}
            let ancestor=if mode&1!=0 {ancestor}else{None};
            let parent=ancestor.map(|m|coord/(1<<(m-mip)));
            loop {
                if t>=exit {break;}
                assert!(steps+jumps<(dims.element_sum()+4) as u32);
                assert!(coord.cmpge(IVec3::ZERO).all()&&coord.cmplt(dims).all());
                if let Some(m)=ancestor {if Some(coord/(1<<(m-mip)))!=parent {break;}}
                let end=exit.min(next.min_element());
                if end>t {
                    if ancestor.is_some() {skipped+=1;} else {
                        let i=index(coord,dims.x);
                        if levels[mip][i].extinction[0][0]>0. {occupied.push((i,t.to_bits(),end.to_bits()));}
                    }
                    t=end;
                }
                let mut crossed=false;
                for axis in 0..3 {if next[axis]<=t {coord[axis]+=step[axis];next[axis]=face(coord,axis);crossed=true;}}
                steps+=1;assert!(crossed||t>=exit);
                if ancestor.is_none() {break;}
            }
        }
        (occupied,steps,skipped,jumps)
    }
    fn sparse_levels()->[Vec<GpuCell>;4] {
        let mut levels:[Vec<GpuCell>;4]=std::array::from_fn(|m|vec![GpuCell::zeroed();(8usize>>m).pow(3)]);
        // Include a terminal slab face and isolated interior cells. Most
        // space remains empty, while every selected mip contains real mass.
        for z in 0..8 {for y in 0..8 {for x in 0..8 {
            if (z==7&&y==3)||(x==2&&y==2&&z==2)||(x==6&&y==6&&z==5) {
                let c=&mut levels[0][x+8*(y+8*z)];
                for axis in 0..13 {c.extinction[axis/4][axis%4]=0.1+(x+y+z) as f32*0.03;}
                c.extinction[3][1]=1.;c.normals[0][3]=1.;
            }
        }}}
        for mip in 1..4 {
            let n=8usize>>mip;let child_n=n*2;
            for z in 0..n {for y in 0..n {for x in 0..n {
                let mut cell=GpuCell::zeroed();
                for dz in 0..2 {for dy in 0..2 {for dx in 0..2 {
                    let child=levels[mip-1][x*2+dx+child_n*(y*2+dy+child_n*(z*2+dz))];
                    for axis in 0..13 {cell.extinction[axis/4][axis%4]+=child.extinction[axis/4][axis%4]*0.125;}
                    if child.extinction[3][1]>0. {cell.extinction[3][1]=1.;cell.normals[0][3]=1.;}
                }}}
                levels[mip][x+n*(y+n*z)]=cell;
            }}}
        }
        levels
    }
    #[test]
    fn empty_regions_preserve_every_occupied_endpoint_at_all_mips_and_clips() {
        use glam::Vec3;
        let levels=sparse_levels();let mut skipped=0;let mut occupied=0;
        for mip in 0..4 {for side in [-1.,1.] {for i in 0..80 {
            let origin=Vec3::new(-0.9+((i*17)%79) as f32/79.*0.9,-0.3+((i*29)%79) as f32/79.*0.9,-0.45+side*2.);
            let direction=(Vec3::new(-0.45,0.15,-0.45)-origin).normalize();
            for limit in [0.1,1.7,2.125,100.] {
                let reference=sparse_walk(origin,direction,limit,mip,&levels,false);
                let candidate=sparse_walk(origin,direction,limit,mip,&levels,true);
                assert_eq!(candidate.0,reference.0,"occupied endpoints changed at mip{mip} ray{i} clip{limit}");
                assert_eq!(candidate.1,reference.1,"logical DDA/error bound remains unchanged");
                assert!(candidate.2<=candidate.1);skipped+=candidate.2;occupied+=candidate.0.len();
            }
        }}}
        // Exact simultaneous faces, reverse rays and origins on a grid face.
        for (origin,direction) in [(Vec3::splat(-2.),Vec3::ONE.normalize()),
            (Vec3::new(-0.675,-0.075,-0.675),-Vec3::ONE.normalize()),
            (Vec3::new(-0.6,0.1,-2.),Vec3::Z)] {
            let a=sparse_walk(origin,direction,100.,0,&levels,false);
            let b=sparse_walk(origin,direction,100.,0,&levels,true);assert_eq!(a.0,b.0);assert_eq!(a.1,b.1);
        }
        assert!(skipped>1000&&occupied>100,"test must actually exercise empty and occupied intervals");
    }
    #[test]
    fn coarse_jumps_preserve_occupied_endpoint_bits_and_reduce_real_iterations() {
        use glam::Vec3;
        let levels=sparse_levels();let(mut fine,mut executed,mut jumps,mut occupied)=(0,0,0,0);
        // Exercise nearby and distant origins, every optical mip, inward and
        // outward rays, opaque cuts inside empty parents and at occupied tails.
        for mip in 0..4 {for distance in [0.3f32,2.,55.,1000.] {for side in [-1.,1.] {for i in 0..80 {
            let target=Vec3::new(-0.9+((i*17)%79) as f32/79.*0.9,-0.3+((i*29)%79) as f32/79.*0.9,-0.45);
            let origin=Vec3::new(-0.45+(i%3) as f32*0.13,0.15,-0.45+side*distance);
            let direction=(target-origin).normalize();
            for limit in [distance*0.2,distance,distance+0.2,1e30] {
                let reference=sparse_walk_mode(origin,direction,limit,mip,&levels,0);
                for mode in [2,3] {
                    let candidate=sparse_walk_mode(origin,direction,limit,mip,&levels,mode);
                    assert_eq!(candidate.0,reference.0,"occupied endpoint changed mip{mip} distance{distance} side{side} ray{i} limit{limit} mode{mode}");
                    assert!(candidate.1+candidate.3<=reference.1,"jump must eliminate, not invent, selected-grid intervals");
                    if mode==2 {assert_eq!(candidate.2,0,"true jump is not read-only bypass");}
                    fine+=reference.1;executed+=candidate.1+candidate.3;jumps+=candidate.3;occupied+=candidate.0.len();
                }
            }
        }}}}
        for direction in [Vec3::X,Vec3::Y,Vec3::Z,Vec3::ONE.normalize(),-Vec3::ONE.normalize()] {
            for origin in [Vec3::new(-0.675,-0.075,-0.675),Vec3::splat(-2.),Vec3::splat(2.)] {
                let a=sparse_walk_mode(origin,direction,100.,0,&levels,0);
                let b=sparse_walk_mode(origin,direction,100.,0,&levels,2);assert_eq!(a.0,b.0);
            }
        }
        assert!(occupied>100&&jumps>1000,"must exercise occupied segments and true jumps");
        assert!(executed<fine,"must eliminate actual fine-DDA iterations");
    }
    #[test]
    fn bounded_boundary_search_matches_all_fine_faces_including_ties() {
        // Independent linear enumeration of every candidate boundary is the
        // oracle for the bounded binary search, including coalesced f32 times.
        for lower in [-1e6f32,-3.6,-0.9,0.,3.6,1e6] {
            for width in [2,4,8] {for start in 0..width {for step in [-1,1] {
                for direction in [-0.000001f32,-0.37,0.000001,0.37] {
                    if direction.signum() as i32!=step {continue;}
                    let origin=lower-11.7*direction;let cell=0.1125f32;
                    let count=if step>0 {width-start}else{start+1};
                    let face=|k:i32| {
                        let before=start+(k-1)*step;let boundary=before+i32::from(step>0);
                        (lower+boundary as f32*cell-origin)/direction
                    };
                    for k in 1..=count {for end in [face(k),face(k)-0.01,face(k)+0.01] {
                        let expected=(1..=count).take_while(|&i|face(i)<=end).count() as i32;
                        let(mut lo,mut hi)=(0,count);
                        for _ in 0..4 {if lo>=hi {break;}let middle=(lo+hi+1)/2;
                            if face(middle)<=end {lo=middle;}else{hi=middle-1;}}
                        assert_eq!(lo,expected,"lower{lower} width{width} start{start} step{step} endpoint{end}");
                    }}
                }
            }}}
        }
    }
    #[test]
    fn shared_optics_shader_validates_without_temporal_or_root_bindings() {
        let shared=crate::shaders::SHARED_NAMES.map(crate::shaders::built_in).join("\n");
        let source=format!("{shared}\n{}",include_str!("shaders/vegetation_bricks.wgsl"));
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
        let group2:Vec<_>=module.global_variables.iter().filter_map(|(_,v)|v.binding.as_ref().filter(|b|b.group==2).map(|b|b.binding)).collect();
        assert_eq!(group2,vec![0,1,2,3,4,5,6]);
        assert_eq!(module.entry_points.iter().filter(|e|e.name.starts_with("vegetation_brick_")).count(),2);
        assert!(!include_str!("shaders/vegetation_bricks.wgsl").contains("previous_world"));
        // Count actual entry-point resource dependencies, not every unused
        // declaration in the shared shade library. A future helper must not
        // silently add an unbound material/instance dependency to this path.
        let view_bindings=[0,2,4,5,6,7,10,11,13,14,15];
        for (index,entry) in module.entry_points.iter().enumerate() {
            if !entry.name.starts_with("vegetation_brick_") {continue;}
            let ep=info.get_entry_point(index);let mut storage=0;
            for (handle,global) in module.global_variables.iter() {
                if ep[handle].is_empty() {continue;}
                if matches!(global.space,naga::AddressSpace::Storage {..}) {storage+=1;}
                let Some(binding)=&global.binding else {continue;};
                match binding.group {
                    0=>assert!(crate::voxel_forest::SCENE_BINDINGS.contains(&binding.binding),"{} scene binding{}",entry.name,binding.binding),
                    1=>assert!(view_bindings.contains(&binding.binding),"{} view binding{}",entry.name,binding.binding),
                    2=>assert!(binding.binding<=6),_=>panic!("unexpected resource group"),
                }
            }
            assert!(storage<=16,"{} needs{storage} storage buffers",entry.name);
        }
    }
}
