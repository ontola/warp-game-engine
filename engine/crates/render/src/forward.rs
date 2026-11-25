//! Minimal forward renderer for plain mesh scenes. CPU owner/frustum selection,
//! coarsest authored mesh LOD, indexed instancing, vertex lighting, no postprocess.
use crate::{Gpu,Scene,Lighting,ViewRect,FrameStats,scene::GpuInstance};
use glam::{Mat4,Vec3,Vec4};
use bytemuck::{Pod,Zeroable};
use wgpu::util::DeviceExt;
use std::{collections::{HashMap,BTreeMap},ops::Range};
#[repr(C)] #[derive(Clone,Copy,Pod,Zeroable)]
struct Instance {rows:[[f32;4];3],material:u32,flags:u32,pad:[u32;2]}
#[repr(C)] #[derive(Clone,Copy,Pod,Zeroable)]
struct Spot {position:[f32;4],direction:[f32;4],intensity:[f32;4]}
#[repr(C)] #[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {vp:[[f32;4];4],camera:[f32;4],sun:[f32;4],sun_color:[f32;4],ambient:[f32;4],fog:[f32;4],spots:[Spot;4],
    /// The low tier's sky pass: view rays from screen positions.
    inv_vp:[[f32;4];4]}
struct Part {indices:Range<u32>,material:u32,lava:bool,blend:bool}
struct RigidDraw {model:usize,rows:[[f32;4];3],base:u32,hidden_views:u8,centre:Vec3}
/// Joint rows the palette buffer holds: 16 characters of 64 joints.
const PALETTE_ROWS:usize=16*64*3;
struct Mesh {parts:Vec<Part>}
struct Target {size:[u32;2],color:wgpu::Texture,depth:wgpu::Texture,instances:wgpu::Buffer,capacity:usize,uniform:wgpu::Buffer,bind:wgpu::BindGroup}
/// What the low tier (`Lit`) reads from the main renderer.
pub(crate) struct LitInputs<'a> {pub albedo:&'a wgpu::TextureView,pub headlights:&'a wgpu::Buffer,pub scene_tables:&'a wgpu::Buffer,pub instances:&'a wgpu::Buffer,pub source:String}
/// forward_surface.wgsl `View` and `Light`: what game surfaces read.
#[repr(C)] #[derive(Clone,Copy,Pod,Zeroable)]
struct SurfaceUniform {cam:[f32;4],sun_dir:[f32;4],sun_color:[f32;4],anim:[f32;4]}
#[repr(C)] #[derive(Clone,Copy,Pod,Zeroable)]
struct LitUniform {tone:[f32;4],sky:[f32;4],gi_origin:[f32;4],gi_dims:[f32;4],medium:[f32;4],tint:[f32;4],ambient:[f32;4],sh:[[f32;4];9],beams:[crate::volumetric::GpuLight;MAX_BEAMS]}
/// Scattering lights the low tier draws as beams, nearest first.
const MAX_BEAMS:usize=16;
/// The low tier: per-pixel light (forward_lit.wgsl) instead of the vertex
/// light of the minimum profile. Opaque and blended pipelines per format.
struct Lit {
    opaque:wgpu::RenderPipeline,opaque_srgb:wgpu::RenderPipeline,blend:wgpu::RenderPipeline,blend_srgb:wgpu::RenderPipeline,
    /// The sky where nothing was drawn (forward_lit.wgsl `fs_sky`).
    sky:wgpu::RenderPipeline,sky_srgb:wgpu::RenderPipeline,
    layout:wgpu::BindGroupLayout,uniform:wgpu::Buffer,bind:wgpu::BindGroup,albedo:wgpu::TextureView,samp:wgpu::Sampler,headlights:wgpu::Buffer,
    probe_samp:wgpu::Sampler,probes:wgpu::TextureView,blends:wgpu::Buffer,grid:Option<crate::irradiance_volume::Grid>,
    bloom:Bloom,
    /// Group 2: game surfaces' view, light, scene tables and instances.
    surface:wgpu::Buffer,surface_bind:wgpu::BindGroup,
    /// Glow strength (0 off) and display threshold, from the renderer's options each frame.
    pub(crate) glow:(f32,f32),
    /// The sky's irradiance as nine SH coefficients (the renderer's sky_sh).
    sky_sh:[[f32;4];9],
}
pub(crate) struct Forward {
    lit:Option<Lit>,
    pipeline:wgpu::RenderPipeline,pipeline_srgb:wgpu::RenderPipeline,layout:wgpu::BindGroupLayout,vertices:wgpu::Buffer,indices:wgpu::Buffer,materials:wgpu::Buffer,
    lava_view:wgpu::TextureView,lava_sampler:wgpu::Sampler,stone_view:wgpu::TextureView,stone_detail:bool,
    /// `Graphics::minimal_draw_distance` and `minimal_min_angle`.
    draw_distance:f32,min_angle:f32,
    /// Index ranges of `Scene::rigid_models`, this frame's characters and
    /// their joints: three rows per joint, one draw per character.
    rigid_models:Vec<Range<u32>>,rigid_actors:Vec<RigidDraw>,palette:wgpu::Buffer,
    pub(crate) profiler:crate::gpu_profile::Profiler, pub(crate) profiling:bool, query:Option<(wgpu::QuerySet,wgpu::Buffer)>, sequence:u64, pub(crate) profile_frame_id:Option<u64>,
    meshes:Vec<Mesh>,ranges:HashMap<(u32,u32),usize>,owners:Vec<GpuInstance>,targets:Vec<Target>,
}
impl Forward {
    pub fn new(gpu:&Gpu,scene:&Scene,materials:&wgpu::Buffer,lit:Option<LitInputs>)->Self {
        let mut indices=Vec::new();let mut meshes=Vec::new();let mut ranges=HashMap::new();
        for mesh in &scene.meshes {
            let mut by_material:BTreeMap<u32,Vec<u32>>=BTreeMap::new();
            for c in &scene.clusters[mesh.first_cluster as usize..(mesh.first_cluster+mesh.cluster_count)as usize] {
                if c.parent_error<=1e30 {continue;}
                let out=by_material.entry(c.material).or_default();
                for &tri in &scene.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize] {
                    for k in 0..3 {out.push(scene.cluster_verts[c.first_vert as usize+((tri>>(k*8))&255)as usize]);}
                }
            }
            let mut parts=Vec::new();
            for (material,list) in by_material {let start=indices.len()as u32;indices.extend(list);let flags=scene.materials[material as usize].flags;parts.push(Part{indices:start..indices.len()as u32,material,lava:flags&crate::scene::LAVA!=0,blend:flags&crate::scene::BLEND!=0});}
            ranges.insert((mesh.first_cluster,mesh.cluster_count),meshes.len());meshes.push(Mesh{parts});
        }
        // Characters go after the scene's vertices, outside the cluster data.
        let mut all_vertices=scene.vertices.clone();let mut rigid_models=Vec::new();
        for model in &scene.rigid_models {
            let (base,start)=(all_vertices.len()as u32,indices.len()as u32);
            all_vertices.extend_from_slice(&model.vertices);
            indices.extend(model.indices.iter().map(|i|i+base));
            rigid_models.push(start..indices.len()as u32);
        }
        let vertices=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("minimal vertices"),contents:bytemuck::cast_slice(&all_vertices),usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST});
        let palette=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("minimal joints"),size:(PALETTE_ROWS*16)as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        if indices.is_empty(){indices.push(0);}
        let indices=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("minimal indices"),contents:bytemuck::cast_slice(&indices),usage:wgpu::BufferUsages::INDEX});
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("minimal lighting"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry{binding:3,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
            wgpu::BindGroupLayoutEntry{binding:4,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry{binding:5,visibility:wgpu::ShaderStages::VERTEX,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None}]});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("minimal forward"),source:wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/lod_coverage.wgsl"), "\n", include_str!("shaders/forward.wgsl")).into())});
        let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(&layout)],immediate_size:0});
        // Blended materials' alpha and mode (Scene::set_material_blend), per material.
        let mut blend_table=vec![[1.0f32,0.0,0.0,0.0];scene.materials.len().max(1)];
        for (&m,b) in &scene.blended_materials {if let Some(row)=blend_table.get_mut(m as usize) {*row=[b.alpha,if b.additive {1.0} else {0.0},0.0,0.0];}}
        let lit=lit.map(|inputs|Lit::new(gpu,&layout,inputs,&blend_table));
        let pipeline_for=|format,entry|gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("minimal forward"),layout:Some(&pl),
            vertex:wgpu::VertexState{module:&module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[
                Some(wgpu::VertexBufferLayout{array_stride:32,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2]}),
                Some(wgpu::VertexBufferLayout{array_stride:64,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Uint32,7=>Uint32,8=>Uint32]})]},
            primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState{format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),fragment:Some(wgpu::FragmentState{module:&module,entry_point:Some(entry),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:gpu.pipeline_cache()});
        let proxies=scene.materials.iter().any(|m|m.layer<crate::scene::LOD_PROXY_LAYER_BIAS);
        let pipeline=pipeline_for(wgpu::TextureFormat::Rgba8Unorm,if proxies {"fs_proxy"} else {"fs"});
        let pipeline_srgb=pipeline_for(wgpu::TextureFormat::Rgba8UnormSrgb,if proxies {"fs_proxy_srgb"} else {"fs_srgb"});
        let (lava_view,lava_sampler)=crate::minimum_lava::texture(gpu);
        let stone_view=crate::minimum_stone::texture(gpu);
        let supported=gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY|wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
        let query=supported.then(||(gpu.device.create_query_set(&wgpu::QuerySetDescriptor{label:Some("minimum timestamps"),ty:wgpu::QueryType::Timestamp,count:48}),gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("minimum timestamps resolve"),size:48*8,usage:wgpu::BufferUsages::QUERY_RESOLVE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false})));
        Self{lit,profiler:crate::gpu_profile::Profiler::new(supported,48),profiling:false,query,sequence:0,profile_frame_id:None,pipeline,pipeline_srgb,layout,lava_view,lava_sampler,stone_view,stone_detail:scene.graphics.stone_detail,draw_distance:scene.graphics.minimal_draw_distance,min_angle:scene.graphics.minimal_min_angle,rigid_models,rigid_actors:Vec::new(),palette,vertices,indices,materials:materials.clone(),meshes,ranges,owners:scene.instances.clone(),targets:Vec::new()}
    }
    /// The low tier's baked room light (`Renderer::set_irradiance_volume`).
    pub fn set_probes(&mut self,gpu:&Gpu,grid:crate::irradiance_volume::Grid,probes:&[crate::irradiance_volume::Probe]) {
        if let Some(lit)=&mut self.lit {lit.set_probes(gpu,grid,probes);}
    }
    pub fn lit(&self)->bool {self.lit.is_some()}
    /// The low tier's glow: strength (0 off) and the display value it starts at.
    pub fn set_sky_sh(&mut self,sh:[[f32;4];9]) {if let Some(l)=&mut self.lit {l.sky_sh=sh;}}
    pub fn set_glow(&mut self,strength:f32,threshold:f32) {if let Some(l)=&mut self.lit {l.glow=(strength,threshold);}}
    pub fn update_vertices(&self,gpu:&Gpu,offset:u64,bytes:&[u8]) { if !bytes.is_empty() { gpu.queue.write_buffer(&self.vertices,offset,bytes); } }
    pub fn update(&mut self,first:usize,instances:&[GpuInstance])->Result<(),String>{
        if first.checked_add(instances.len()).is_none_or(|n|n>self.owners.len()){return Err("minimal owner update exceeds scene".into());}
        for i in instances {if i.cluster_count>0&&!self.ranges.contains_key(&(i.first_cluster,i.cluster_count)){return Err("minimal renderer requires a registered mesh".into());}}
        self.owners[first..first+instances.len()].copy_from_slice(instances);Ok(())
    }
    /// This frame's characters; their joints go up once for both eyes.
    pub fn set_rigid_actors(&mut self,gpu:&Gpu,actors:&[crate::scene::RigidActor]) {
        self.rigid_actors.clear();let mut rows:Vec<[f32;4]>=Vec::new();
        for actor in actors {
            if actor.model>=self.rigid_models.len()||rows.len()+actor.palette.len()*3>PALETTE_ROWS {continue;}
            let base=rows.len()as u32;
            for joint in &actor.palette {rows.extend([joint.row(0).to_array(),joint.row(1).to_array(),joint.row(2).to_array()]);}
            let m=glam::Mat4::from(actor.transform);
            self.rigid_actors.push(RigidDraw{model:actor.model,rows:[m.row(0).to_array(),m.row(1).to_array(),m.row(2).to_array()],base,hidden_views:actor.hidden_views,centre:Vec3::from(actor.transform.translation)});
        }
        if !rows.is_empty(){gpu.queue.write_buffer(&self.palette,0,bytemuck::cast_slice(&rows));}
    }
    fn target(&self,gpu:&Gpu,size:[u32;2])->Target {
        let texture=|label,format,usage|gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some(label),size:wgpu::Extent3d{width:size[0],height:size[1],depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage,view_formats:&[]});
        let color=texture("minimal eye",wgpu::TextureFormat::Rgba8Unorm,wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::TEXTURE_BINDING);
        let depth=texture("minimal depth",wgpu::TextureFormat::Depth32Float,wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING);
        let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:std::mem::size_of::<Uniform>()as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&self.layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:self.materials.as_entire_binding()},wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::TextureView(&self.lava_view)},wgpu::BindGroupEntry{binding:3,resource:wgpu::BindingResource::Sampler(&self.lava_sampler)},wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(&self.stone_view)},wgpu::BindGroupEntry{binding:5,resource:self.palette.as_entire_binding()}]});
        let instances=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:64,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        Target{size,color,depth,instances,capacity:1,uniform,bind}
    }
    pub fn render(&mut self,gpu:&Gpu,views:&[ViewRect],matrices:&[Mat4],camera_views:&[Mat4],sprites:&crate::sprites::SpriteSlot,fx:&crate::fx::FxSlot,light:&Lighting,spots:&[crate::headlights::SpotLight],beams:&[crate::volumetric::GpuLight],output:Option<&wgpu::Texture>,external:Option<&[&wgpu::Texture]>)->Result<FrameStats,String> {
        let ticket=if self.profiling {
        let names:Vec<_>=(0..views.len()).flat_map(|_|["minimum_geometry","minimum_fx","minimum_sprites"]).collect();
        let ranges:Vec<_>=(0..views.len()).map(|i|(i as u32,i*3..i*3+3)).collect();
        let specs=crate::gpu_profile::describe_passes(&names,&ranges,&[]).map_err(|_|"invalid minimum profiling layout")?;
        let context=crate::gpu_profile::Context{frame_id:self.profile_frame_id.take().unwrap_or(self.sequence),render_sequence:self.sequence,output_size:[views.iter().map(|v|v.w).sum(),views.iter().map(|v|v.h).max().unwrap_or(0)],views:views.len() as u32};
        self.profiler.reserve(context,&specs,(views.len()*6) as u32,gpu.queue.get_timestamp_period() as f64)
        }else{None};
        self.sequence+=1;
        let query=ticket.as_ref().and(self.query.as_ref()).map(|q|&q.0);
        let mut stats=FrameStats::default();let mut enc=gpu.device.create_command_encoder(&Default::default());
        for (vi,view) in views.iter().enumerate(){
            let vp=matrices[vi];let rows:[Vec4;4]=std::array::from_fn(|i|vp.row(i));
            let planes=[rows[3]+rows[0],rows[3]-rows[0],rows[3]+rows[1],rows[3]-rows[1],rows[3]-rows[2]];
            let mut groups:Vec<Vec<usize>>=vec![Vec::new();self.meshes.len()];
            for (owner,i) in self.owners.iter().enumerate(){
                if i.cluster_count==0 || (i.flags>>(8+vi))&1!=0 {continue;}
                let c=Vec3::from_slice(&i.sphere[..3]);let r=i.sphere[3];
                if planes.iter().any(|p|p.dot(c.extend(1.0)) < -r*p.truncate().length()){continue;}
                if self.draw_distance>0.0||self.min_angle>0.0 {
                    let d=c.distance(view.cam.pos);
                    if self.draw_distance>0.0 && d-r>self.draw_distance {continue;}
                    if r<d*self.min_angle {continue;}
                }
                if let Some(&mesh)=self.ranges.get(&(i.first_cluster,i.cluster_count)){groups[mesh].push(owner);}
            }
            let mut packed=Vec::new();let mut draws=Vec::new();
            for (mesh,owners) in self.meshes.iter().zip(groups){if owners.is_empty(){continue;}
                for part in &mesh.parts {let start=packed.len()as u32;
                    for &owner in &owners {let i=self.owners[owner];packed.push(Instance{rows:i.rows,material:part.material,flags:i.flags,pad:[0,owner as u32]});}
                    let end=packed.len()as u32;stats.triangles+=(part.indices.len()/3)as u32*(end-start);stats.submitted_vertices+=part.indices.len()as u64*(end-start)as u64;
                    draws.push((part.lava,part.blend,part.indices.clone(),start..end));
                }
            }
            for actor in &self.rigid_actors {
                if (actor.hidden_views>>vi)&1!=0 || planes.iter().any(|p|p.dot(actor.centre.extend(1.0)) < -3.0*p.truncate().length()) {continue;}
                let start=packed.len()as u32;
                packed.push(Instance{rows:actor.rows,material:0,flags:0,pad:[actor.base+1,0]});
                let indices=self.rigid_models[actor.model].clone();
                stats.triangles+=(indices.len()/3)as u32;
                draws.push((false,false,indices,start..start+1));
            }
            // Fill depth with solid blocks first; hidden lava needs no texture reads.
            draws.sort_by_key(|draw|(draw.1,draw.0));
            let size=[view.w,view.h];
            if self.targets.len()<=vi {self.targets.push(self.target(gpu,size));}
            if self.targets[vi].size!=size {self.targets[vi]=self.target(gpu,size);}
            let t=&mut self.targets[vi];
            if packed.len()>t.capacity {t.capacity=packed.len().next_power_of_two();t.instances=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("minimal visible owners"),size:(t.capacity*64)as u64,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});}
            if !packed.is_empty(){gpu.queue.write_buffer(&t.instances,0,bytemuck::cast_slice(&packed));}
            let exposure=1.0/(1.2*2f32.powf(light.ev100));
            let mut lamps=[Spot::zeroed();4];
            for (out,l) in lamps.iter_mut().zip(spots){*out=Spot{position:l.position.extend(l.range_m).to_array(),direction:l.direction.extend(l.outer_angle_radians.cos()).to_array(),intensity:(l.intensity_cd*exposure).extend(l.inner_angle_radians.cos()).to_array()};}
            let uniform=Uniform{vp:vp.to_cols_array_2d(),camera:view.cam.pos.extend(exposure).to_array(),sun:light.sun_dir.extend(light.sun_lux*exposure).to_array(),sun_color:light.sun_color.extend(light.emissive_scale()).to_array(),ambient:[light.sky_lux*exposure,spots.len().min(4)as f32,light.debug,light.time],fog:[light.fog_start,light.fog_end,0.,if self.stone_detail {1.} else {0.}],spots:lamps,inv_vp:vp.inverse().to_cols_array_2d()};
            gpu.queue.write_buffer(&t.uniform,0,bytemuck::bytes_of(&uniform));
            if let Some(l)=&self.lit {l.write(gpu,light,exposure,beams,view.cam.pos);}
            let target=external.map_or(&t.color,|targets|targets[vi]);
            let color=target.create_view(&Default::default());let depth=t.depth.create_view(&Default::default());
            let sky=(light.sky_lux*exposure*0.25).max(0.0);let sky=(sky/(1.0+sky)).sqrt()as f64;
            let clear=|c:f64|if target.format().is_srgb(){if c<=0.04045{c/12.92}else{((c+0.055)/1.055).powf(2.4)}}else{c};
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6);}
            {let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("minimal forward eye"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&color,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color{r:clear(sky*0.75),g:clear(sky*0.85),b:clear(sky),a:1.}),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment{view:&depth,depth_ops:Some(wgpu::Operations{load:wgpu::LoadOp::Clear(0.0),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
                let srgb=target.format().is_srgb();
                pass.set_pipeline(match &self.lit {Some(l)=>if srgb {&l.opaque_srgb} else {&l.opaque},None=>if srgb {&self.pipeline_srgb} else {&self.pipeline}});
                pass.set_bind_group(0,&t.bind,&[]);pass.set_vertex_buffer(0,self.vertices.slice(..));pass.set_vertex_buffer(1,t.instances.slice(..));pass.set_index_buffer(self.indices.slice(..),wgpu::IndexFormat::Uint32);
                if let Some(l)=&self.lit {pass.set_bind_group(1,&l.bind,&[]);pass.set_bind_group(2,&l.surface_bind,&[]);}
                let mut blending=false;
                // The sky goes after the opaque meshes (only where they left the
                // far depth) and before the blended ones.
                let sky=|pass:&mut wgpu::RenderPass,l:&Lit|{pass.set_pipeline(if srgb {&l.sky_srgb} else {&l.sky});pass.draw(0..3,0..1);};
                for (_,blend,indices,instances) in draws {
                    if blend && !blending && let Some(l)=&self.lit {blending=true;sky(&mut pass,l);pass.set_pipeline(if srgb {&l.blend_srgb} else {&l.blend});}
                    pass.draw_indexed(indices,0,instances);
                }
                if !blending && let Some(l)=&self.lit {sky(&mut pass,l);}
            }
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6+1);enc.write_timestamp(q,vi as u32*6+2);}
            if let Some(fx)=fx.0.lock().unwrap().as_mut() {
                let white=((light.sun_lux*light.sun_dir.y.max(0.0)+light.sky_lux)/std::f32::consts::PI)
                    .max(light.fx_white_floor.max(0.0)/exposure);
                fx.render_output(gpu,&mut enc,vi,target,&depth,view.cam,camera_views[vi],vp,vp,white,exposure,light,Vec3::splat(light.sky_lux/std::f32::consts::PI),Vec3::splat(light.sky_lux/std::f32::consts::PI),1.0,None,None)?;
            }
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6+3);enc.write_timestamp(q,vi as u32*6+4);}
            if let Some(sprites)=sprites.0.lock().unwrap().as_mut() {
                sprites.render_output(gpu,&mut enc,vi,target,&depth,view.cam,camera_views[vi],vp,vp,exposure)?;
            }
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6+5);}
            if external.is_none() && let Some(l)=&mut self.lit && l.glow.0>0.0 {let (strength,threshold)=l.glow;l.bloom.apply(gpu,&mut enc,vi,&t.color,strength,threshold);}
            if let Some(output)=output {enc.copy_texture_to_texture(t.color.as_image_copy(),wgpu::TexelCopyTextureInfo{texture:output,mip_level:0,origin:wgpu::Origin3d{x:view.x,y:view.y,z:0},aspect:wgpu::TextureAspect::All},t.color.size());}
        }
        if let (Some(ticket),Some((query,resolve)))=(&ticket,&self.query) {enc.resolve_query_set(query,0..ticket.query_count,resolve,0);enc.copy_buffer_to_buffer(resolve,0,&ticket.read,0,ticket.query_count as u64*8);}
        gpu.queue.submit([enc.finish()]);if let Some(ticket)=ticket {ticket.submitted();}Ok(stats)
    }
}

#[repr(C)] #[derive(Clone,Copy,Pod,Zeroable)]
struct BloomParams {texel:[f32;2],direction:[f32;2],strength:f32,threshold:f32,pad:[f32;2]}
/// Eighth-size images of one view: the bright pass and the two blur halves.
struct BloomTarget {size:[u32;2],a:wgpu::TextureView,b:wgpu::TextureView,binds:[wgpu::BindGroup;4]}
/// The low tier's glow (forward_bloom.wgsl): from the finished 8-bit
/// image at an eighth of its size, screened over it.
struct Bloom {bright:wgpu::RenderPipeline,blur:wgpu::RenderPipeline,add:wgpu::RenderPipeline,layout:wgpu::BindGroupLayout,samp:wgpu::Sampler,params:[wgpu::Buffer;4],targets:Vec<BloomTarget>}
impl Bloom {
    fn new(gpu:&Gpu)->Self {
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("forward bloom"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
            wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None}]});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("forward bloom"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/forward_bloom.wgsl").into())});
        let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let pipeline=|entry,blend:Option<wgpu::BlendState>|gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("forward bloom"),layout:Some(&pl),
            vertex:wgpu::VertexState{module:&module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},
            primitive:Default::default(),depth_stencil:None,multisample:Default::default(),
            fragment:Some(wgpu::FragmentState{module:&module,entry_point:Some(entry),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format:wgpu::TextureFormat::Rgba8Unorm,blend,write_mask:wgpu::ColorWrites::COLOR})]}),multiview_mask:None,cache:gpu.pipeline_cache()});
        let screen=wgpu::BlendState{color:wgpu::BlendComponent{src_factor:wgpu::BlendFactor::OneMinusDst,dst_factor:wgpu::BlendFactor::One,operation:wgpu::BlendOperation::Add},alpha:wgpu::BlendComponent::REPLACE};
        let samp=gpu.device.create_sampler(&wgpu::SamplerDescriptor{label:Some("forward bloom"),mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,..Default::default()});
        let params=std::array::from_fn(|_|gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("forward bloom"),size:std::mem::size_of::<BloomParams>()as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}));
        Self{bright:pipeline("bright",None),blur:pipeline("blur",None),add:pipeline("add",Some(screen)),layout,samp,params,targets:Vec::new()}
    }
    /// Glow `color` (one view's finished image) with `strength`, from what is brighter than `threshold` (display value).
    fn apply(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,vi:usize,color:&wgpu::Texture,strength:f32,threshold:f32) {
        let full=[color.width(),color.height()];let size=[full[0].div_ceil(8).max(1),full[1].div_ceil(8).max(1)];
        let source=color.create_view(&Default::default());
        let make=|gpu:&Gpu|{
            let tex=|label|gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some(label),size:wgpu::Extent3d{width:size[0],height:size[1],depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
                format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]}).create_view(&Default::default());
            let (a,b)=(tex("forward bloom a"),tex("forward bloom b"));
            let bind=|view:&wgpu::TextureView,i:usize|gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&self.layout,entries:&[
                wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(view)},wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::Sampler(&self.samp)},wgpu::BindGroupEntry{binding:2,resource:self.params[i].as_entire_binding()}]});
            let binds=[bind(&source,0),bind(&a,1),bind(&b,2),bind(&a,3)];
            BloomTarget{size:full,a,b,binds}
        };
        while self.targets.len()<=vi {let t=make(gpu);self.targets.push(t);}
        // The source view is part of the bind group: remake it when the image changes size.
        if self.targets[vi].size!=full {self.targets[vi]=make(gpu);}
        let t=&self.targets[vi];
        let quarter=[1.0/size[0]as f32,1.0/size[1]as f32];
        let p=|texel,direction|BloomParams{texel,direction,strength,threshold,pad:[0.0;2]};
        for (i,params) in [p([1.0/full[0]as f32,1.0/full[1]as f32],[0.0,0.0]),p(quarter,[1.0,0.0]),p(quarter,[0.0,1.0]),p(quarter,[0.0,0.0])].iter().enumerate() {
            gpu.queue.write_buffer(&self.params[i],0,bytemuck::bytes_of(params));
        }
        let passes:[(&wgpu::RenderPipeline,usize,&wgpu::TextureView,bool);4]=[(&self.bright,0,&t.a,false),(&self.blur,1,&t.b,false),(&self.blur,2,&t.a,false),(&self.add,3,&source,true)];
        for (pipeline,bind,out,load) in passes {
            let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("forward bloom"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:out,depth_slice:None,resolve_target:None,
                ops:wgpu::Operations{load:if load {wgpu::LoadOp::Load} else {wgpu::LoadOp::Clear(wgpu::Color::BLACK)},store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
            pass.set_pipeline(pipeline);pass.set_bind_group(0,&t.binds[bind],&[]);pass.draw(0..3,0..1);
        }
    }
}

impl Lit {
    fn new(gpu:&Gpu,layout0:&wgpu::BindGroupLayout,inputs:LitInputs,blend_table:&[[f32;4]])->Self {
        let entry=|binding,ty|wgpu::BindGroupLayoutEntry{binding,visibility:wgpu::ShaderStages::FRAGMENT,ty,count:None};
        let uniform_ty=wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None};
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("forward lit"),entries:&[
            entry(0,uniform_ty),entry(1,uniform_ty),
            entry(2,wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false}),
            entry(3,wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            entry(4,wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D3,multisampled:false}),
            entry(5,wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            entry(6,wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None})]});
        let storage=|binding|wgpu::BindGroupLayoutEntry{binding,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None};
        let blends=wgpu::util::DeviceExt::create_buffer_init(&gpu.device,&wgpu::util::BufferInitDescriptor{label:Some("forward lit blends"),contents:bytemuck::cast_slice(blend_table),usage:wgpu::BufferUsages::STORAGE});
        let surface_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("forward lit surfaces"),entries:&[entry(0,uniform_ty),entry(1,uniform_ty),storage(2),storage(3)]});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("forward lit"),source:wgpu::ShaderSource::Wgsl(inputs.source.as_str().into())});
        let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(layout0),Some(&layout),Some(&surface_layout)],immediate_size:0});
        let pipeline=|format,entry,blend:bool|gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("forward lit"),layout:Some(&pl),
            vertex:wgpu::VertexState{module:&module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[
                Some(wgpu::VertexBufferLayout{array_stride:32,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2]}),
                Some(wgpu::VertexBufferLayout{array_stride:64,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Uint32,7=>Uint32,8=>Uint32,9=>Uint32]})]},
            primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState{format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(!blend),depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),fragment:Some(wgpu::FragmentState{module:&module,entry_point:Some(entry),compilation_options:Default::default(),
                targets:&[Some(wgpu::ColorTargetState{format,blend:blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:gpu.pipeline_cache()});
        let sky_pipeline=|format,entry|gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("forward lit sky"),layout:Some(&pl),
            vertex:wgpu::VertexState{module:&module,entry_point:Some("vs_sky"),compilation_options:Default::default(),buffers:&[]},
            primitive:Default::default(),
            depth_stencil:Some(wgpu::DepthStencilState{format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(false),depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),fragment:Some(wgpu::FragmentState{module:&module,entry_point:Some(entry),compilation_options:Default::default(),
                targets:&[Some(wgpu::ColorTargetState{format,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:gpu.pipeline_cache()});
        let (unorm,srgb)=(wgpu::TextureFormat::Rgba8Unorm,wgpu::TextureFormat::Rgba8UnormSrgb);
        // No anisotropy: on an Adreno 740 anisotropic taps cost the shade
        // pass 3.4x (2026-11-25).
        let samp=gpu.device.create_sampler(&wgpu::SamplerDescriptor{label:Some("forward lit albedo"),address_mode_u:wgpu::AddressMode::Repeat,address_mode_v:wgpu::AddressMode::Repeat,
            mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,mipmap_filter:wgpu::MipmapFilterMode::Linear,..Default::default()});
        let probe_samp=gpu.device.create_sampler(&wgpu::SamplerDescriptor{label:Some("forward lit probes"),mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,..Default::default()});
        let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("forward lit"),size:std::mem::size_of::<LitUniform>()as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let probes=Self::probe_texture(gpu,[1,1,1],&[0u16;4]);
        let bind=Self::bind(gpu,&layout,&uniform,inputs.headlights,inputs.albedo,&samp,&probes,&probe_samp,&blends);
        let surface=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("forward lit surfaces"),size:std::mem::size_of::<SurfaceUniform>()as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let surface_bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("forward lit surfaces"),layout:&surface_layout,entries:&[
            wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::Buffer(wgpu::BufferBinding{buffer:&surface,offset:0,size:wgpu::BufferSize::new(16)})},
            wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::Buffer(wgpu::BufferBinding{buffer:&surface,offset:0,size:wgpu::BufferSize::new(64)})},
            wgpu::BindGroupEntry{binding:2,resource:inputs.scene_tables.as_entire_binding()},wgpu::BindGroupEntry{binding:3,resource:inputs.instances.as_entire_binding()}]});
        Self{opaque:pipeline(unorm,"fs",false),opaque_srgb:pipeline(srgb,"fs_srgb",false),blend:pipeline(unorm,"fs",true),blend_srgb:pipeline(srgb,"fs_srgb",true),sky:sky_pipeline(unorm,"fs_sky"),sky_srgb:sky_pipeline(srgb,"fs_sky_srgb"),
            layout,uniform,bind,albedo:inputs.albedo.clone(),samp,headlights:inputs.headlights.clone(),probe_samp,probes,blends,grid:None,bloom:Bloom::new(gpu),glow:(0.0,0.75),sky_sh:[[0.0;4];9],surface,surface_bind}
    }
    #[allow(clippy::too_many_arguments)]
    fn bind(gpu:&Gpu,layout:&wgpu::BindGroupLayout,uniform:&wgpu::Buffer,headlights:&wgpu::Buffer,albedo:&wgpu::TextureView,samp:&wgpu::Sampler,probes:&wgpu::TextureView,probe_samp:&wgpu::Sampler,blends:&wgpu::Buffer)->wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("forward lit"),layout,entries:&[
            wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:headlights.as_entire_binding()},
            wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::TextureView(albedo)},wgpu::BindGroupEntry{binding:3,resource:wgpu::BindingResource::Sampler(samp)},
            wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(probes)},wgpu::BindGroupEntry{binding:5,resource:wgpu::BindingResource::Sampler(probe_samp)},
            wgpu::BindGroupEntry{binding:6,resource:blends.as_entire_binding()}]})
    }
    /// The probes' six axes side by side along x (forward_lit.wgsl `probe_axis`), half floats.
    fn probe_texture(gpu:&Gpu,size:[u32;3],texels:&[u16])->wgpu::TextureView {
        let extent=wgpu::Extent3d{width:size[0],height:size[1],depth_or_array_layers:size[2]};
        let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("forward lit probes"),size:extent,mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D3,
            format:wgpu::TextureFormat::Rgba16Float,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
        gpu.queue.write_texture(texture.as_image_copy(),bytemuck::cast_slice(texels),wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(size[0]*8),rows_per_image:Some(size[1])},extent);
        texture.create_view(&Default::default())
    }
    fn set_probes(&mut self,gpu:&Gpu,grid:crate::irradiance_volume::Grid,probes:&[crate::irradiance_volume::Probe]) {
        let [dx,dy,dz]=grid.dims;
        let mut texels=vec![0u16;(dx*6*dy*dz*4)as usize];
        for z in 0..dz {for y in 0..dy {for x in 0..dx {
            let probe=&probes[(x+y*dx+z*dx*dy)as usize];
            for axis in 0..6u32 {
                let at=(((z*dy+y)*dx*6+axis*dx+x)*4)as usize;let c=probe.axes[axis as usize];
                for (k,v) in [c.x,c.y,c.z,1.0].into_iter().enumerate() {texels[at+k]=crate::noise_field::f16_bits(v.min(65000.0));}
            }
        }}}
        self.probes=Self::probe_texture(gpu,[dx*6,dy,dz],&texels);
        self.bind=Self::bind(gpu,&self.layout,&self.uniform,&self.headlights,&self.albedo,&self.samp,&self.probes,&self.probe_samp,&self.blends);
        self.grid=Some(grid);
    }
    fn write(&self,gpu:&Gpu,light:&Lighting,exposure:f32,beams:&[crate::volumetric::GpuLight],camera:Vec3) {
        let (gi_origin,gi_dims,on)=match self.grid {
            Some(g)=>(g.origin.extend(g.cell).to_array(),[g.dims[0]as f32,g.dims[1]as f32,g.dims[2]as f32,0.0],1.0),
            None=>([0.0,0.0,0.0,1.0],[1.0;4],0.0),
        };
        let fill=light.indirect_fill_lux.max(0.0);
        let mut sh=self.sky_sh;sh.iter_mut().for_each(|c|c[3]=0.0);
        let mut list=[crate::volumetric::GpuLight::zeroed();MAX_BEAMS];let mut count=0;
        let (mut medium,mut tint,mut ambient)=([0.0;4],[0.0;4],[0.0;4]);
        if let Some(v)=light.volumetric.filter(|v|v.density>0.0&&v.intensity>0.0) {
            let mut sorted:Vec<_>=beams.iter().filter(|b|Vec3::from_slice(&b.position_range[..3]).distance(camera)<b.position_range[3]+40.0).collect();
            sorted.sort_by(|a,b|Vec3::from_slice(&a.position_range[..3]).distance(camera).total_cmp(&Vec3::from_slice(&b.position_range[..3]).distance(camera)));
            for (out,b) in list.iter_mut().zip(sorted) {*out=*b;count+=1;}
            medium=[v.density,v.anisotropy,v.intensity,count as f32];tint=v.color.extend(v.extinction).to_array();ambient=v.ambient.extend(v.length).to_array();
        }
        gpu.queue.write_buffer(&self.surface,0,bytemuck::bytes_of(&SurfaceUniform{cam:camera.extend(0.0).to_array(),sun_dir:light.sun_dir.extend(light.sun_lux).to_array(),
            sun_color:light.sun_color.extend(1.0).to_array(),anim:[light.time,light.emissive_scale(),1.2*2f32.powf(light.ev100),0.0]}));
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&LitUniform{tone:[exposure,light.emissive_scale(),light.contrast,light.saturation],sky:[fill,fill*0.97,fill*0.92,on],gi_origin,gi_dims,medium,tint,ambient,sh,beams:list}));
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn minimum_preserves_per_eye_hiding_and_reactivated_owner_geometry() {
        let gpu=Gpu::new().unwrap();let mut scene=Scene::default();scene.graphics.minimal_forward=true;
        let m=scene.add_material(crate::scene::Material{tint:[1.,0.,0.],layer:-1,roughness:1.,flags:crate::scene::EMISSIVE,scale:30000.,metal:0.});
        let mesh=scene.add_mesh(&crate::primitives::quad(m).mesh());scene.add_instance(mesh,glam::Affine3A::IDENTITY);
        scene.instances[0].hide_in_views(1);
        let light=Lighting{sun_lux:0.,sky_lux:0.,..Default::default()};
        let renderer=crate::Renderer::new(&gpu,&scene,&light);
        let camera=crate::Camera{pos:Vec3::new(0.,0.,3.),target:Vec3::ZERO,fov_y:1.,near:0.05};
        let views=[ViewRect{cam:camera,x:0,y:0,w:64,h:64},ViewRect{cam:camera,x:64,y:0,w:64,h:64}];
        let draw=||renderer.try_render_views(&gpu,&views,&light,128,64,&[]).unwrap().0;
        let pixel=|image:&[u8],x:usize|image[(32*128+x)*4];
        let initial=draw();assert_eq!(pixel(&initial,32),0);assert!(pixel(&initial,96)>100);
        let eye=||gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("direct SRGB test eye"),size:wgpu::Extent3d{width:64,height:64,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
        let left=eye();let right=eye();
        renderer.try_render_eye_targets(&gpu,&views,&light,&[&left,&right]).unwrap();
        let direct=[gpu.read_rgba8(&left),gpu.read_rgba8(&right)];
        for (eye,image) in direct.iter().enumerate() {for y in 0..64 {for x in 0..64 {for c in 0..4 {
            assert!(image[(y*64+x)*4+c].abs_diff(initial[(y*128+x+eye*64)*4+c])<=1,"direct eye differs from offscreen output");
        }}}}
        scene.instances[0].cluster_count=0;renderer.update_instances(&gpu,0,&scene.instances).unwrap();
        let hidden=draw();assert_eq!(pixel(&hidden,32),0);assert_eq!(pixel(&hidden,96),0);
        scene.update_instance(0,mesh,glam::Affine3A::IDENTITY);scene.instances[0].hide_in_views(0);
        renderer.update_instances(&gpu,0,&scene.instances).unwrap();let shown=draw();assert!(pixel(&shown,32)>100&&pixel(&shown,96)>100);
    }
    #[test] fn minimum_skips_owners_beyond_the_draw_distance_or_too_small_to_see() {
        let gpu=Gpu::new().unwrap();
        let draw=|distance:f32,angle:f32,z:f32|{
            let mut scene=Scene::default();scene.graphics.minimal_forward=true;
            scene.graphics.minimal_draw_distance=distance;scene.graphics.minimal_min_angle=angle;
            let m=scene.add_material(crate::scene::Material{tint:[1.,0.,0.],layer:-1,roughness:1.,flags:crate::scene::EMISSIVE,scale:30000.,metal:0.});
            let mesh=scene.add_mesh(&crate::primitives::quad(m).mesh());scene.add_instance(mesh,glam::Affine3A::from_translation(Vec3::new(0.,0.,z)));
            let light=Lighting{sun_lux:0.,sky_lux:0.,..Default::default()};
            let renderer=crate::Renderer::new(&gpu,&scene,&light);
            let camera=crate::Camera{pos:Vec3::new(0.,0.,3.),target:Vec3::new(0.,0.,z),fov_y:1.,near:0.05};
            let image=renderer.try_render_views(&gpu,&[ViewRect{cam:camera,x:0,y:0,w:64,h:64}],&light,64,64,&[]).unwrap().0;
            image[(32*64+32)*4]
        };
        assert!(draw(0.,0.,0.)>100,"no limits: drawn");
        assert!(draw(50.,0.,0.)>100,"within the draw distance");
        assert_eq!(draw(50.,0.,-60.),0,"beyond the draw distance");
        assert!(draw(0.,0.01,0.)>100,"large enough");
        assert_eq!(draw(0.,0.5,0.),0,"smaller than the minimum angle");
    }
    #[test] fn minimum_keeps_explosion_and_ion_column_visible_until_their_expiry() {
        let gpu=Gpu::new().unwrap();let mut scene=Scene::default();scene.graphics.minimal_forward=true;
        let m=scene.add_material(crate::scene::Material{tint:[0.2;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
        let mesh=scene.add_mesh(&crate::primitives::quad(m).mesh());scene.add_instance(mesh,glam::Affine3A::from_translation(Vec3::new(0.,0.,-100.)));
        let light=Lighting{sun_lux:0.,sky_lux:0.,fx_white_floor:1.,..Default::default()};
        let renderer=crate::Renderer::new(&gpu,&scene,&light);
        let camera=crate::Camera{pos:Vec3::new(0.,1.,8.),target:Vec3::new(0.,1.,0.),fov_y:1.,near:0.05};
        let views=[ViewRect{cam:camera,x:0,y:0,w:256,h:160}];
        let draw=||renderer.try_render_views(&gpu,&views,&light,256,160,&[]).unwrap().0;
        let baseline=draw();let mut fx=crate::fx::Effects::default();
        fx.explosion(0.,Vec3::new(-2.,0.7,0.),1.5,Vec3::new(1.,0.5,0.1));
        fx.column(crate::fx::AdditiveColumn{position:Vec3::new(2.,-2.,0.),height:8.,radius:[0.3;2],color:[Vec3::new(0.05,0.5,1.);2],opacity:[0.8;2],radiance:[50000.;2],..Default::default()}).unwrap();
        fx.flush(&gpu,&renderer,0.2).unwrap();let active=draw();
        let mut changed=[0usize;2];
        for (index,(a,b)) in active.chunks_exact(4).zip(baseline.chunks_exact(4)).enumerate() {
            if a[..3].iter().zip(b).any(|(x,y)|x.abs_diff(*y)>20) {changed[(index%256)/128]+=1;}
        }
        assert!(changed[0]>30,"explosion disappeared: {changed:?}");
        assert!(changed[1]>30,"ion column disappeared: {changed:?}");
        if let Ok(path)=std::env::var("WARP_FX_CAPTURE") {crate::write_png(std::path::Path::new(&path),256,160,&active).unwrap();}
        let eye=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("direct FX eye"),size:wgpu::Extent3d{width:256,height:160,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
        renderer.try_render_eye_targets(&gpu,&views,&light,&[&eye]).unwrap();
        let direct=gpu.read_rgba8(&eye);let mut visible=[0;2];
        for (i,pixel) in direct.chunks_exact(4).enumerate() {if pixel[..3].iter().any(|v|*v>20){visible[(i%256)/128]+=1;}}
        assert!(visible.iter().all(|v|*v>30),"gameplay FX disappeared from direct sRGB attachment: {visible:?}");
        fx.flush(&gpu,&renderer,10.).unwrap();assert_eq!(draw(),baseline,"expired gameplay effects remain visible");
    }

}
