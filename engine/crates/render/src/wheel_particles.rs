//! Shared, bounded wheel ejecta. Births are uploaded once; age, drag and gravity
//! are evaluated on the GPU from simulation time, identically in all four views.
use crate::{Camera, Gpu, Lighting};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Mutex;

pub const MAX_PARTICLES: usize = 4096;
pub const MAX_VISIBLE_PER_VIEW: usize = 2048;
/// Conservative sum of projected billboard/mesh bounding boxes. Dust cannot
/// consume unbounded fill at a stationary burnout or with four overlapping cars.
const MAX_SCREEN_COVERAGE: f32 = 2.0;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind { Stone, SoilClod, GrassBlade, MudClod, Dust, TyreSmoke, WaterDrop, Sand }
#[derive(Clone, Copy, Debug)]
pub struct Birth {
    pub position: Vec3, pub velocity: Vec3, pub normal: Vec3,
    pub radius: f32, pub lifetime: f32, pub wetness: f32,
    pub kind: Kind, pub seed: u32, pub birth_time: f64,
}
impl Birth {
    fn valid(&self, now: f64) -> bool {
        self.position.is_finite() && self.velocity.is_finite() && self.normal.is_finite()
            && (0.25..=4.0).contains(&self.normal.length_squared())
            && self.velocity.length_squared() <= 160_000.0
            && (0.001..=0.5).contains(&self.radius) && (0.02..=8.0).contains(&self.lifetime)
            && (0.0..=1.0).contains(&self.wetness) && self.birth_time.is_finite()
            && self.birth_time >= 0.0 && self.birth_time <= now + 1e-6
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Particle {
    position_radius: [f32;4], velocity_born: [f32;4], normal_life: [f32;4],
    color_kind: [f32;4], params: [f32;4], // seed angle, gravity, drag, opacity
}
fn particle(b: &Birth, epoch: f64) -> Particle {
    let (color, gravity, drag, opacity) = match b.kind {
        // Gravel is a mix of rock types: grey, buff, brown and near-black pieces.
        Kind::Stone => ([Vec3::new(0.36,0.35,0.33),Vec3::new(0.45,0.40,0.32),Vec3::new(0.30,0.24,0.18),Vec3::new(0.16,0.16,0.16)][(b.seed>>3) as usize%4],9.81,0.28,1.0),
        Kind::SoilClod => (Vec3::new(0.14,0.085,0.041),9.81,0.45,1.0),
        // Fresh blades, some yellowed (albedo of grass leaves, about 0.1-0.25).
        Kind::GrassBlade => ([Vec3::new(0.11,0.24,0.04),Vec3::new(0.15,0.28,0.05),Vec3::new(0.24,0.26,0.08)][(b.seed>>3) as usize%3],5.5,2.0,1.0),
        Kind::MudClod => (Vec3::new(0.065,0.040,0.023),9.81,0.65,1.0),
        Kind::Dust => (Vec3::new(0.34,0.26,0.17),-0.16,1.5,0.21),
        Kind::TyreSmoke => (Vec3::splat(0.35),-0.32,1.8,0.19),
        Kind::WaterDrop => (Vec3::new(0.28,0.34,0.38),9.81,0.8,0.55),
        // Dry quartz sand, pale buff to golden (linear albedo about 0.3-0.45). Clumps
        // of fine grains feel more air drag than a stone of the same size.
        Kind::Sand => ([Vec3::new(0.40,0.33,0.22),Vec3::new(0.45,0.37,0.25),Vec3::new(0.34,0.27,0.18)][(b.seed>>3) as usize%3],9.81,0.9,1.0),
    };
    let phase=(b.seed as f64 / u32::MAX as f64 * std::f64::consts::TAU) as f32;
    let variation=0.85+0.30*((b.seed.wrapping_mul(747796405)>>8) as f32 / 16777215.0);
    Particle { position_radius:b.position.extend(b.radius).to_array(),
        velocity_born:b.velocity.extend((b.birth_time-epoch) as f32).to_array(),
        normal_life:b.normal.normalize().extend(b.lifetime).to_array(),
        color_kind:(color*variation*(1.0-0.4*b.wetness)).extend(b.kind as u32 as f32).to_array(),
        params:[phase,gravity,drag,opacity] }
}
fn position(p: &Particle, age:f32) -> Vec3 {
    let drag=p.params[2];let travel=(1.0-(-drag*age).exp())/drag;
    Vec3::from_slice(&p.position_radius[..3])+Vec3::from_slice(&p.velocity_born[..3])*travel
        -Vec3::Y*p.params[1]*(age/drag-travel/drag)
}
fn radius(p:&Particle,age:f32)->f32 {
    p.position_radius[3] * if matches!(p.color_kind[3] as u32,4|5) {1.0+age*2.2} else {1.0}
}
struct Pool { particles:Vec<Particle>, next:usize, now:f64, epoch:f64 }
impl Default for Pool {fn default()->Self{Self {particles:Vec::new(),next:0,now:0.0,epoch:0.0}}}
impl Pool {
    fn update(&mut self, now:f64, births:&[Birth])->Result<Vec<(usize,Particle)>,String> {
        if !now.is_finite() || now<0.0 || births.len()>MAX_PARTICLES || births.iter().any(|b|!b.valid(now)) {
            return Err("invalid wheel particle time/births or birth batch exceeds 4096".into());
        }
        if now<self.now {return Err("wheel particle simulation time moved backwards; reset at stage change".into());}
        if self.particles.is_empty(){self.epoch=now;}
        self.now=now;
        let mut writes=Vec::with_capacity(births.len());
        for b in births {
            if now-b.birth_time>=b.lifetime as f64{continue;}
            let p=particle(b,self.epoch);let slot=self.next;
            if slot==self.particles.len(){self.particles.push(p);}else{self.particles[slot]=p;}
            writes.push((slot,p));self.next=(slot+1)%MAX_PARTICLES;
        }
        Ok(writes)
    }
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {
    output_vp:[[f32;4];4], depth_vp:[[f32;4];4], shadow_vp:[[f32;4];4], near_vp:[[f32;4];4],
    camera_time:[f32;4], right_near:[f32;4], up:[f32;4], sun_dir:[f32;4], sun_color:[f32;4],
    ambient:[f32;4], shadow:[f32;4], size:[f32;4],
}
pub struct Shadows<'a> {
    pub far:&'a wgpu::TextureView, pub near:&'a wgpu::TextureView, pub sampler:&'a wgpu::Sampler,
    pub far_vp:Mat4, pub near_vp:Mat4, pub far_texel:f32, pub near_texel:f32, pub near_enabled:bool,
}
struct Target { size:(u32,u32), output:Option<wgpu::Texture>, uniform:wgpu::Buffer, order:wgpu::Buffer }
pub struct WheelParticles {
    pipeline:wgpu::RenderPipeline,layout:wgpu::BindGroupLayout,buffer:wgpu::Buffer,
    pool:Mutex<Pool>,targets:Mutex<Vec<Option<Target>>>,
}
pub fn shader_source()->&'static str {include_str!("shaders/wheel_particles.wgsl")}
impl WheelParticles {
    pub fn new(gpu:&Gpu)->Self {
        let entries=(0..7).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,count:None,
            ty:match binding {
                0=>wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},
                1|2=>wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},
                3..=5=>wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},
                _=>wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            }}).collect::<Vec<_>>();
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("wheel particles"),entries:&entries});
        let pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("wheel particles"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("wheel particles"),source:wgpu::ShaderSource::Wgsl(shader_source().into())});
        let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("wheel dust and debris"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState{module:&shader,entry_point:Some("particle_vs"),compilation_options:Default::default(),buffers:&[]},
            primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},depth_stencil:None,multisample:Default::default(),
            fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("particle_fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format:wgpu::TextureFormat::Rgba16Float,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache: gpu.pipeline_cache()});
        let buffer=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("shared wheel particle births"),size:(MAX_PARTICLES*std::mem::size_of::<Particle>())as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        Self{pipeline,layout,buffer,pool:Mutex::new(Pool::default()),targets:Mutex::new(Vec::new())}
    }
    pub fn reset(&self){*self.pool.lock().unwrap_or_else(|e|e.into_inner())=Pool::default();}
    /// Same clock/analytical position as the draw. All live supports are kept;
    /// camera sorting or particle coverage caps never certify shadow demand.
    pub(crate) fn shadow_receiver_bounds(&self,map_mask:u32)->Vec<crate::receiver_shadow_demand::ReceiverBounds> {
        let pool=self.pool.lock().unwrap_or_else(|e|e.into_inner());let now=(pool.now-pool.epoch)as f32;
        pool.particles.iter().filter_map(|p| {
            let age=now-p.velocity_born[3];if age<0.0||age>=p.normal_life[3] {return None;}
            let center=position(p,age);let reach=Vec3::splat(radius(p,age)+0.01);
            Some(crate::receiver_shadow_demand::ReceiverBounds {min:center-reach,max:center+reach,map_mask})
        }).collect()
    }
    pub fn update(&self,gpu:&Gpu,now:f64,births:&[Birth])->Result<(),String>{
        let writes=self.pool.lock().unwrap_or_else(|e|e.into_inner()).update(now,births)?;
        // At most two contiguous writes for a ring wrap, rather than one queue
        // allocation per particle or re-uploading every surviving particle.
        let mut at=0;
        while at<writes.len(){let first=at;at+=1;while at<writes.len()&&writes[at].0==writes[at-1].0+1{at+=1;}
            let data:Vec<_>=writes[first..at].iter().map(|x|x.1).collect();
            gpu.queue.write_buffer(&self.buffer,(writes[first].0*std::mem::size_of::<Particle>())as u64,bytemuck::cast_slice(&data));}
        Ok(())
    }
    /// `reuse_output` must only be true for a disposable post-temporal target
    /// such as exhaust output. Native/DLSS history is always immutable.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,index:usize,target:&wgpu::Texture,reuse_output:bool,
        depth:&wgpu::TextureView,camera:Camera,output_vp:Mat4,depth_vp:Mat4,light:&Lighting,shadows:Shadows<'_>)->Result<Option<wgpu::Texture>,String>{
        let pool=self.pool.lock().unwrap_or_else(|e|e.into_inner());let now=(pool.now-pool.epoch)as f32;
        let mut order:Vec<_>=pool.particles.iter().enumerate().filter_map(|(i,p)|{
            let age=now-p.velocity_born[3];if age<0.0||age>=p.normal_life[3]{return None;}
            let pos=position(p,age);let r=radius(p,age);let clip=output_vp*pos.extend(1.0);
            let scale=output_vp.y_axis.truncate().length().max(output_vp.x_axis.truncate().length());let margin=r*scale;
            let dist=pos.distance(camera.pos);let projected=margin/clip.w.max(camera.near);
            (dist<100.0&&clip.w>camera.near+r&&clip.x.abs()<clip.w+margin&&clip.y.abs()<clip.w+margin&&projected*target.height()as f32>0.4)
                .then_some((i as u32,dist,projected*projected))
        }).collect();
        if order.is_empty(){return Ok(None);}if index>=4{return Err("wheel particles support at most four views".into());}
        // Prefer the closest ejecta if saturated; the selected set is then
        // back-to-front for transparent dust. Stable IDs break distance ties.
        order.sort_by(|a,b|a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let mut coverage=0.0;order.retain(|p|{if coverage+p.2>MAX_SCREEN_COVERAGE{return false;}coverage+=p.2;true});order.truncate(MAX_VISIBLE_PER_VIEW);
        if order.is_empty(){return Ok(None);}order.reverse();let ids:Vec<_>=order.iter().map(|p|p.0).collect();drop(pool);
        if target.format()!=wgpu::TextureFormat::Rgba16Float{return Err("wheel particles require HDR input".into());}
        if !target.usage().contains(if reuse_output{wgpu::TextureUsages::RENDER_ATTACHMENT}else{wgpu::TextureUsages::COPY_SRC}){return Err("wheel particle target has incompatible usage".into());}
        let size=(target.width(),target.height());let mut targets=self.targets.lock().unwrap_or_else(|e|e.into_inner());if targets.len()<=index{targets.resize_with(index+1,||None);}
        if targets[index].as_ref().is_none_or(|t|t.size!=size){
            let buffer=|label,size,usage|gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some(label),size,usage,mapped_at_creation:false});
            targets[index]=Some(Target{size,output:None,uniform:buffer("wheel particle view",std::mem::size_of::<Uniform>()as u64,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST),order:buffer("wheel particle view order",(MAX_VISIBLE_PER_VIEW*4)as u64,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST)});
        }
        let t=targets[index].as_mut().unwrap();
        if !reuse_output&&t.output.is_none(){t.output=Some(gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("wheel particle output, never temporal history"),size:target.size(),mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:target.format(),usage:wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[]}));}
        let forward=(camera.target-camera.pos).normalize_or(Vec3::NEG_Z);let right=forward.cross(Vec3::Y).normalize_or(Vec3::X);let up=right.cross(forward);
        let uniform=Uniform{output_vp:output_vp.to_cols_array_2d(),depth_vp:depth_vp.to_cols_array_2d(),shadow_vp:shadows.far_vp.to_cols_array_2d(),near_vp:shadows.near_vp.to_cols_array_2d(),
            camera_time:camera.pos.extend(now).to_array(),right_near:right.extend(camera.near).to_array(),up:up.extend(0.0).to_array(),sun_dir:light.sun_dir.extend(light.shadow_strength.clamp(0.0,1.0)).to_array(),sun_color:(light.sun_color*light.sun_lux.max(0.0)).extend(0.0).to_array(),
            ambient:(Vec3::new(0.78,0.86,1.0)*light.sky_lux.max(0.0)).extend(0.0).to_array(),shadow:[shadows.far_texel,shadows.near_texel,if shadows.near_enabled{1.0}else{0.0},0.0],size:[size.0 as f32,size.1 as f32,1.0/size.0 as f32,1.0/size.1 as f32]};
        gpu.queue.write_buffer(&t.uniform,0,bytemuck::bytes_of(&uniform));gpu.queue.write_buffer(&t.order,0,bytemuck::cast_slice(&ids));
        let output=if reuse_output{target}else{let output=t.output.as_ref().unwrap();enc.copy_texture_to_texture(target.as_image_copy(),output.as_image_copy(),target.size());output};
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("wheel particle view"),layout:&self.layout,entries:&[
            wgpu::BindGroupEntry{binding:0,resource:t.uniform.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:self.buffer.as_entire_binding()},wgpu::BindGroupEntry{binding:2,resource:t.order.as_entire_binding()},
            wgpu::BindGroupEntry{binding:3,resource:wgpu::BindingResource::TextureView(depth)},wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(shadows.far)},wgpu::BindGroupEntry{binding:5,resource:wgpu::BindingResource::TextureView(shadows.near)},wgpu::BindGroupEntry{binding:6,resource:wgpu::BindingResource::Sampler(shadows.sampler)}]});
        let view=output.create_view(&Default::default());let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("wheel particles"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&bind,&[]);pass.draw(0..24,0..ids.len()as u32);drop(pass);Ok(Some(output.clone()))
    }
}

#[cfg(test)]mod tests{
    use super::*;
    fn birth()->Birth{Birth{position:Vec3::ZERO,velocity:Vec3::new(4.,3.,1.),normal:Vec3::Y,radius:0.03,lifetime:1.5,wetness:0.,kind:Kind::Stone,seed:23,birth_time:0.}}
    #[test]fn shader_and_layout_validate(){assert_eq!(std::mem::size_of::<Particle>(),80);assert_eq!(std::mem::size_of::<Uniform>(),384);let m=naga::front::wgsl::parse_str(shader_source()).unwrap();naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&m).unwrap();}
    #[test]fn shared_pool_bound_and_expiry(){let mut p=Pool::default();let births=vec![birth();MAX_PARTICLES];assert_eq!(p.update(0.,&births).unwrap().len(),MAX_PARTICLES);assert_eq!(p.particles.len(),MAX_PARTICLES);p.update(0.1,&[Birth{birth_time:0.1,..birth()}]).unwrap();assert_eq!(p.particles.len(),MAX_PARTICLES);assert_eq!(p.next,1);assert!(p.update(4.,&births).unwrap().is_empty());}
    #[test]fn pause_ages_no_particles_and_rewind_requires_reset(){let mut p=Pool::default();p.update(0.,&[birth()]).unwrap();let initial=p.particles[0];p.update(0.,&[]).unwrap();assert_eq!(bytemuck::bytes_of(&initial),bytemuck::bytes_of(&p.particles[0]));p.update(1.,&[]).unwrap();assert!(p.update(0.,&[]).is_err());p=Pool::default();assert!(p.update(0.,&[birth()]).is_ok());}
    #[test]fn delayed_birth_keeps_age_and_invalid_batch_is_atomic(){let mut p=Pool::default();p.update(0.5,&[birth()]).unwrap();assert!((p.now as f32-p.epoch as f32-p.particles[0].velocity_born[3]-0.5).abs()<1e-6);let n=p.particles.len();assert!(p.update(1.,&[Birth{normal:Vec3::ZERO,..birth()}]).is_err());assert_eq!(p.particles.len(),n);assert_eq!(p.now,0.5);}
    #[test]fn analytical_motion_and_kinds_are_distinct(){let b=birth();let stone=particle(&b,0.);let dust=particle(&Birth{kind:Kind::Dust,..b},0.);let grass=particle(&Birth{kind:Kind::GrassBlade,..b},0.);assert_eq!(position(&stone,0.),b.position);assert!(position(&stone,1.).y<position(&dust,1.).y);assert!(radius(&dust,1.)>radius(&stone,1.));assert_ne!(grass.color_kind,stone.color_kind);}
}
