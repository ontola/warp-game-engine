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
struct Uniform {vp:[[f32;4];4],camera:[f32;4],sun:[f32;4],sun_color:[f32;4],ambient:[f32;4],fog:[f32;4],spots:[Spot;4]}
struct Part {indices:Range<u32>,material:u32,lava:bool}
struct Mesh {parts:Vec<Part>}
struct Target {size:[u32;2],color:wgpu::Texture,depth:wgpu::Texture,instances:wgpu::Buffer,capacity:usize,uniform:wgpu::Buffer,bind:wgpu::BindGroup}
pub(crate) struct Forward {
    pipeline:wgpu::RenderPipeline,pipeline_srgb:wgpu::RenderPipeline,layout:wgpu::BindGroupLayout,vertices:wgpu::Buffer,indices:wgpu::Buffer,materials:wgpu::Buffer,
    lava_view:wgpu::TextureView,lava_sampler:wgpu::Sampler,
    pub(crate) profiler:crate::gpu_profile::Profiler, pub(crate) profiling:bool, query:Option<(wgpu::QuerySet,wgpu::Buffer)>, sequence:u64, pub(crate) profile_frame_id:Option<u64>,
    meshes:Vec<Mesh>,ranges:HashMap<(u32,u32),usize>,owners:Vec<GpuInstance>,targets:Vec<Target>,
}
impl Forward {
    pub fn new(gpu:&Gpu,scene:&Scene,materials:&wgpu::Buffer)->Self {
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
            for (material,list) in by_material {let start=indices.len()as u32;indices.extend(list);parts.push(Part{indices:start..indices.len()as u32,material,lava:scene.materials[material as usize].flags&crate::scene::LAVA!=0});}
            ranges.insert((mesh.first_cluster,mesh.cluster_count),meshes.len());meshes.push(Mesh{parts});
        }
        let vertices=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("minimal vertices"),contents:bytemuck::cast_slice(&scene.vertices),usage:wgpu::BufferUsages::VERTEX});
        if indices.is_empty(){indices.push(0);}
        let indices=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("minimal indices"),contents:bytemuck::cast_slice(&indices),usage:wgpu::BufferUsages::INDEX});
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("minimal lighting"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::VERTEX,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry{binding:3,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None}]});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("minimal forward"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/forward.wgsl").into())});
        let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let pipeline_for=|format,entry|gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("minimal forward"),layout:Some(&pl),
            vertex:wgpu::VertexState{module:&module,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[
                Some(wgpu::VertexBufferLayout{array_stride:32,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2]}),
                Some(wgpu::VertexBufferLayout{array_stride:64,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Uint32,7=>Uint32]})]},
            primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState{format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),fragment:Some(wgpu::FragmentState{module:&module,entry_point:Some(entry),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:gpu.pipeline_cache()});
        let pipeline=pipeline_for(wgpu::TextureFormat::Rgba8Unorm,"fs");
        let pipeline_srgb=pipeline_for(wgpu::TextureFormat::Rgba8UnormSrgb,"fs_srgb");
        let (lava_view,lava_sampler)=crate::minimum_lava::texture(gpu);
        let supported=gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY|wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
        let query=supported.then(||(gpu.device.create_query_set(&wgpu::QuerySetDescriptor{label:Some("minimum timestamps"),ty:wgpu::QueryType::Timestamp,count:48}),gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("minimum timestamps resolve"),size:48*8,usage:wgpu::BufferUsages::QUERY_RESOLVE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false})));
        Self{profiler:crate::gpu_profile::Profiler::new(supported,48),profiling:false,query,sequence:0,profile_frame_id:None,pipeline,pipeline_srgb,layout,lava_view,lava_sampler,vertices,indices,materials:materials.clone(),meshes,ranges,owners:scene.instances.clone(),targets:Vec::new()}
    }
    pub fn update(&mut self,first:usize,instances:&[GpuInstance])->Result<(),String>{
        if first.checked_add(instances.len()).is_none_or(|n|n>self.owners.len()){return Err("minimal owner update exceeds scene".into());}
        for i in instances {if i.cluster_count>0&&!self.ranges.contains_key(&(i.first_cluster,i.cluster_count)){return Err("minimal renderer requires a registered mesh".into());}}
        self.owners[first..first+instances.len()].copy_from_slice(instances);Ok(())
    }
    fn target(&self,gpu:&Gpu,size:[u32;2])->Target {
        let texture=|label,format,usage|gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some(label),size:wgpu::Extent3d{width:size[0],height:size[1],depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage,view_formats:&[]});
        let color=texture("minimal eye",wgpu::TextureFormat::Rgba8Unorm,wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::TEXTURE_BINDING);
        let depth=texture("minimal depth",wgpu::TextureFormat::Depth32Float,wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING);
        let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:std::mem::size_of::<Uniform>()as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&self.layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:self.materials.as_entire_binding()},wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::TextureView(&self.lava_view)},wgpu::BindGroupEntry{binding:3,resource:wgpu::BindingResource::Sampler(&self.lava_sampler)}]});
        let instances=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:64,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        Target{size,color,depth,instances,capacity:1,uniform,bind}
    }
    pub fn render(&mut self,gpu:&Gpu,views:&[ViewRect],matrices:&[Mat4],camera_views:&[Mat4],sprites:&crate::sprites::SpriteSlot,fx:&crate::fx::FxSlot,light:&Lighting,spots:&[crate::headlights::SpotLight],output:Option<&wgpu::Texture>,external:Option<&[&wgpu::Texture]>)->Result<FrameStats,String> {
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
                if let Some(&mesh)=self.ranges.get(&(i.first_cluster,i.cluster_count)){groups[mesh].push(owner);}
            }
            let mut packed=Vec::new();let mut draws=Vec::new();
            for (mesh,owners) in self.meshes.iter().zip(groups){if owners.is_empty(){continue;}
                for part in &mesh.parts {let start=packed.len()as u32;
                    for &owner in &owners {let i=self.owners[owner];packed.push(Instance{rows:i.rows,material:part.material,flags:i.flags,pad:[0;2]});}
                    let end=packed.len()as u32;stats.triangles+=(part.indices.len()/3)as u32*(end-start);stats.submitted_vertices+=part.indices.len()as u64*(end-start)as u64;
                    draws.push((part.lava,part.indices.clone(),start..end));
                }
            }
            // Fill depth with solid blocks first; hidden lava needs no texture reads.
            draws.sort_by_key(|draw|draw.0);
            let size=[view.w,view.h];
            if self.targets.len()<=vi {self.targets.push(self.target(gpu,size));}
            if self.targets[vi].size!=size {self.targets[vi]=self.target(gpu,size);}
            let t=&mut self.targets[vi];
            if packed.len()>t.capacity {t.capacity=packed.len().next_power_of_two();t.instances=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("minimal visible owners"),size:(t.capacity*64)as u64,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});}
            if !packed.is_empty(){gpu.queue.write_buffer(&t.instances,0,bytemuck::cast_slice(&packed));}
            let exposure=1.0/(1.2*2f32.powf(light.ev100));
            let mut lamps=[Spot::zeroed();4];
            for (out,l) in lamps.iter_mut().zip(spots){*out=Spot{position:l.position.extend(l.range_m).to_array(),direction:l.direction.extend(l.outer_angle_radians.cos()).to_array(),intensity:(l.intensity_cd*exposure).extend(l.inner_angle_radians.cos()).to_array()};}
            let uniform=Uniform{vp:vp.to_cols_array_2d(),camera:view.cam.pos.extend(exposure).to_array(),sun:light.sun_dir.extend(light.sun_lux*exposure).to_array(),sun_color:light.sun_color.extend(light.emissive_scale()).to_array(),ambient:[light.sky_lux*exposure,spots.len().min(4)as f32,light.debug,light.time],fog:[light.fog_start,light.fog_end,0.,0.],spots:lamps};
            gpu.queue.write_buffer(&t.uniform,0,bytemuck::bytes_of(&uniform));
            let target=external.map_or(&t.color,|targets|targets[vi]);
            let color=target.create_view(&Default::default());let depth=t.depth.create_view(&Default::default());
            let sky=(light.sky_lux*exposure*0.25).max(0.0);let sky=(sky/(1.0+sky)).sqrt()as f64;
            let clear=|c:f64|if target.format().is_srgb(){if c<=0.04045{c/12.92}else{((c+0.055)/1.055).powf(2.4)}}else{c};
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6);}
            {let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("minimal forward eye"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&color,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color{r:clear(sky*0.75),g:clear(sky*0.85),b:clear(sky),a:1.}),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment{view:&depth,depth_ops:Some(wgpu::Operations{load:wgpu::LoadOp::Clear(0.0),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
                pass.set_pipeline(if target.format().is_srgb(){&self.pipeline_srgb}else{&self.pipeline});pass.set_bind_group(0,&t.bind,&[]);pass.set_vertex_buffer(0,self.vertices.slice(..));pass.set_vertex_buffer(1,t.instances.slice(..));pass.set_index_buffer(self.indices.slice(..),wgpu::IndexFormat::Uint32);
                for (_,indices,instances) in draws {pass.draw_indexed(indices,0,instances);}
            }
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6+1);enc.write_timestamp(q,vi as u32*6+2);}
            if let Some(fx)=fx.0.lock().unwrap().as_mut() {
                let white=((light.sun_lux*light.sun_dir.y.max(0.0)+light.sky_lux)/std::f32::consts::PI)
                    .max(light.fx_white_floor.max(0.0)/exposure);
                fx.render_output(gpu,&mut enc,vi,target,&depth,view.cam,camera_views[vi],vp,vp,white,exposure,light,Vec3::splat(light.sky_lux/std::f32::consts::PI),Vec3::splat(light.sky_lux/std::f32::consts::PI),None)?;
            }
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6+3);enc.write_timestamp(q,vi as u32*6+4);}
            if let Some(sprites)=sprites.0.lock().unwrap().as_mut() {
                sprites.render_output(gpu,&mut enc,vi,target,&depth,view.cam,camera_views[vi],vp,vp,exposure)?;
            }
            if let Some(q)=query {enc.write_timestamp(q,vi as u32*6+5);}
            if let Some(output)=output {enc.copy_texture_to_texture(t.color.as_image_copy(),wgpu::TexelCopyTextureInfo{texture:output,mip_level:0,origin:wgpu::Origin3d{x:view.x,y:view.y,z:0},aspect:wgpu::TextureAspect::All},t.color.size());}
        }
        if let (Some(ticket),Some((query,resolve)))=(&ticket,&self.query) {enc.resolve_query_set(query,0..ticket.query_count,resolve,0);enc.copy_buffer_to_buffer(resolve,0,&ticket.read,0,ticket.query_count as u64*8);}
        gpu.queue.submit([enc.finish()]);if let Some(ticket)=ticket {ticket.submitted();}Ok(stats)
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
