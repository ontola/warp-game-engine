//! Shared, stage-static source-alpha sun shadow. This is deliberately a
//! rest-pose cache: moving cars, streamed grass and animated parts stay in the
//! existing dynamic maps. Camera motion and ordinary wind cannot invalidate it.
use crate::{Gpu, scene::{Scene,GpuInstance,FOLIAGE,SOLID,NO_SHADOW,LIVE_GRASS,WIND_GRASS,PART_WIND,SKINNED}};
use glam::{Mat4,Vec3};
use bytemuck::{Pod,Zeroable};
use wgpu::util::DeviceExt;

pub(crate) const SIZE:u32=4096;
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
pub(crate) struct Uniform {pub view_proj:[[f32;4];4],pub params:[f32;4]}
#[derive(Clone,Copy)]
pub(crate) struct Projection {pub vp:Mat4,pub eye:Vec3,pub extent:f32}
struct State {
    sources:Vec<GpuInstance>,
    owners:Vec<bool>,
    dirty:bool,
    key:Option<[u32;7]>,
    projection:Projection,
}
pub(crate) struct Resources {
    pub enabled:bool,
    /// Follows the camera in steps of a quarter of `reach` instead of fitting
    /// a fixed stage: a streamed world's distant shadows, redrawn only when
    /// the camera has moved that far or the sun changed.
    pub anchored:bool,
    /// Half width (metres) of the anchored map.
    pub reach:f32,
    pub lod_pixels:f32,
    pub stage:bool,
    min_extent:f32,
    pub uniform:wgpu::Buffer,
    pub instances:wgpu::Buffer,
    pub wind:wgpu::Buffer,
    _texture:wgpu::Texture,
    pub view:wgpu::TextureView,
    pub target:wgpu::TextureView,
    pub empty:wgpu::TextureView,
    pub capacity:u32,
    state:std::sync::Mutex<State>,
}
fn eligible(i:&GpuInstance,stage:bool)->bool {
    let excluded=NO_SHADOW|LIVE_GRASS|WIND_GRASS|PART_WIND|SKINNED|if stage {0}else{SOLID};
    i.flags&excluded==0 && i.category()!=crate::scene::GeometryCategory::Cars
}
/// Camera-anchored map (streamed GPU forest worlds): static props and
/// buildings. The land has its baked heightfield sun, and the voxel forest
/// draws itself into this map (frame.rs); everything that moves or sways
/// stays in the per-frame maps.
fn eligible_anchored(i:&GpuInstance)->bool {
    use crate::scene::GeometryCategory as C;
    i.flags&(NO_SHADOW|LIVE_GRASS|WIND_GRASS|PART_WIND|SKINNED|FOLIAGE)==0
        && !matches!(i.category(),C::Cars|C::Terrain|C::Trees|C::Grass)
}
fn owners(scene:&Scene,stage:bool,anchored:bool)->Vec<bool> {
    let mut result:Vec<_>=scene.instances.iter().map(|i|if anchored {eligible_anchored(i)} else {eligible(i,stage)}).collect();
    for range in &scene.dynamic_instances {for owner in range.clone(){if let Some(v)=result.get_mut(owner){*v=false;}}}
    for part in &scene.part_wind {if let Some(owner)=part.owner {result[owner as usize]=false;}}
    for part in &scene.part_attachments {result[part.owner as usize]=false;}
    if let Some(a)=&scene.baked_animations {for p in &a.parts{result[p.owner as usize]=false;}}
    result
}
fn hidden(mut i:GpuInstance)->GpuInstance {i.cluster_count=0;i.flags|=NO_SHADOW;i}
fn configured_lod(value:Option<&str>)->f32 {
    value.and_then(|s|s.parse::<f32>().ok()).filter(|v|v.is_finite()&&*v>=0.0&&*v<=8.0).unwrap_or(0.5)
}
pub(crate) fn effective_lod(configured:f32,requested:f32)->f32 {
    // Preserve an explicit source-geometry diagnostic and stricter callers;
    // ordinary distant maps no longer simplify at several world metres.
    if requested.is_finite()&&requested>=0.0 {configured.min(requested*2.0)}else{configured}
}
fn fit(instances:&[GpuInstance],sun:Vec3,stage:bool)->Projection {fit_min(instances,sun,stage,1024.)}
fn fit_min(instances:&[GpuInstance],sun:Vec3,stage:bool,min_extent:f32)->Projection {
    let z=sun.normalize_or(Vec3::Y);
    let up=if z.y.abs()>0.99{Vec3::Z}else{Vec3::Y};
    let x=up.cross(z).normalize();let y=z.cross(x);
    let has_foliage=!stage && instances.iter().any(|i|i.cluster_count>0 && i.flags&FOLIAGE!=0);
    let mut lo=glam::Vec2::splat(f32::INFINITY);let mut hi=-lo;
    for i in instances.iter().filter(|i|i.cluster_count>0 && (!has_foliage || i.flags&FOLIAGE!=0)) {
        let p=Vec3::from_slice(&i.sphere);let r=i.sphere[3];
        let q=glam::Vec2::new(x.dot(p),y.dot(p));lo=lo.min(q-glam::Vec2::splat(r));hi=hi.max(q+glam::Vec2::splat(r));
    }
    if !lo.is_finite() || !hi.is_finite(){lo=glam::Vec2::splat(-1.);hi=-lo;}
    let centre=(lo+hi)*0.5;
    // Every source crown stays inside the fully weighted 0.8 NDC region.
    // Scene fit also means four views share exactly the same coverage/cache.
    let extent=(((hi-lo).max_element()*0.5+16.)/0.8/128.).ceil().max(min_extent/128.)*128.;
    let focus=x*centre.x+y*centre.y;
    let mut bounds=crate::stability::SunSceneBounds::default();bounds.extend_instances(instances);
    let raw=bounds.depth_range(sun,&[focus]);
    // The shader fades the outer five percent in Z too. Fit the complete
    // static caster bounds inside that fully weighted interval, not on it.
    let mid=(raw[0]+raw[1])*0.5;let half=(raw[1]-raw[0])*0.5/0.9+16.;
    let range=[mid-half,mid+half];
    Projection{vp:crate::stability::stable_shadow_depth(focus,sun,extent,SIZE,range),
        eye:crate::stability::shadow_eye(focus,sun,range),extent}
}
/// A square map `reach` metres each way around `centre`, deep enough for
/// every caster `range` reports there. The shader fades its outer 20 %.
fn anchored(centre:Vec3,sun:Vec3,reach:f32,range:&dyn Fn(Vec3,f32)->[f32;2])->Projection {
    let raw=range(centre,reach);
    let mid=(raw[0]+raw[1])*0.5;let half=(raw[1]-raw[0])*0.5/0.9+16.;
    let range=[mid-half,mid+half];
    Projection{vp:crate::stability::stable_shadow_depth(centre,sun,reach,SIZE,range),eye:crate::stability::shadow_eye(centre,sun,range),extent:reach}
}
impl Resources {
    pub fn new(gpu:&Gpu,scene:&Scene)->Self {
        let explicit=!scene.static_sun_instances.is_empty();
        let stage=scene.stage_sun || explicit;
        let min_extent=if explicit{128.}else{1024.};
        let paged=crate::switches::PAGED_FOLIAGE_SUN.on();
        let setting=crate::switches::FAR_SUN_SHADOW.var();
        // A streamed GPU forest world gets a camera-anchored map by default.
        // KERN_SUN_CLIPMAP: the clipmap's outer levels cover this reach.
        let anchored=!stage && scene.gpu_forest.is_some() && !paged && setting.as_deref()!=Ok("0") && !crate::frame::sun_clipmap_requested();
        let enabled=anchored || match setting.as_deref(){Ok("0")=>false,Ok("1")=>true,_=>stage && !paged};
        let reach=crate::switches::FAR_SUN_REACH.parse::<f32>().filter(|v|v.is_finite()&&*v>=256.).unwrap_or(1024.);
        let lod_pixels=configured_lod(crate::switches::FAR_SUN_LOD.get().as_deref());
        let mut owners=if enabled {owners(scene,stage,anchored)}else{Vec::new()};
        if enabled && explicit {owners.fill(false);for &id in &scene.static_sun_instances {if let Some(owner)=owners.get_mut(id as usize){*owner=eligible(&scene.instances[id as usize],true);}}}
        let sources:Vec<_>=scene.instances.iter().zip(&owners).map(|(&i,&owner)|if owner{i}else{hidden(i)}).collect();
        let capacity=sources.iter().map(|i|i.cluster_count as u64).sum::<u64>().max(if anchored {65536} else {1}).clamp(1,crate::frame::MAX_VISIBLE as u64) as u32;
        let data=if enabled && !sources.is_empty(){bytemuck::cast_slice(&sources)}else{&[0u8;80]};
        let instances=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("far sun static sources"),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
        let wind=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("far sun rest pose"),size:if enabled{(sources.len().max(1)*16)as u64}else{16},usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("far sun lighting"),contents:bytemuck::bytes_of(&Uniform::zeroed()),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
        let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("far sun static depth"),size:wgpu::Extent3d{width:if enabled{SIZE}else{1},height:if enabled{SIZE}else{1},depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let empty_texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("far sun caster placeholder"),size:wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let target=texture.create_view(&Default::default());
        let array_view=wgpu::TextureViewDescriptor{dimension:Some(wgpu::TextureViewDimension::D2Array),..Default::default()};
        let view=texture.create_view(&array_view);let empty=empty_texture.create_view(&array_view);
        Self{enabled,anchored,reach,lod_pixels,stage,min_extent,uniform,instances,wind,_texture:texture,view,target,empty,capacity,state:std::sync::Mutex::new(State{sources,owners,dirty:true,key:None,projection:Projection{vp:Mat4::IDENTITY,eye:Vec3::ZERO,extent:1024.}})}
    }
    pub fn update(&self,gpu:&Gpu,first:usize,input:&[GpuInstance]) {
        if !self.enabled{return;}
        let mut s=self.state.lock().unwrap();
        let (mut from,mut to)=(usize::MAX,0);
        for (n,&i) in input.iter().enumerate(){let index=first+n;
            if !s.owners[index]{continue;}
            let keep=if self.anchored {eligible_anchored(&i)} else {eligible(&i,self.stage)};
            let next=if keep{i}else{hidden(i)};
            if bytemuck::bytes_of(&next)!=bytemuck::bytes_of(&s.sources[index]){
                s.sources[index]=next;s.dirty=true;
                from=from.min(index);to=index+1;
            }
        }
        // One write for the changed range: a write per instance took tens of
        // milliseconds per frame while terrain patches streamed.
        if from<to {gpu.queue.write_buffer(&self.instances,(from*std::mem::size_of::<GpuInstance>())as u64,bytemuck::cast_slice(&s.sources[from..to]));}
    }
    /// Returns whether the persistent map needs one shared redraw. The caller
    /// must encode that redraw before any view samples the new uniform.
    /// `focus`: where the camera's lighting is centred, and the sun-axis
    /// depth range of everything that may cast within `reach` of it (anchored
    /// maps only).
    pub fn prepare(&self,gpu:&Gpu,sun:Vec3,alpha:f32,lod:f32,focus:Vec3,range:impl Fn(Vec3,f32)->[f32;2])->Option<(Projection,bool)> {
        if !self.enabled{return None;}
        let step=self.reach*0.25;
        let cell=if self.anchored {[(focus.x/step).round() as i32,(focus.z/step).round() as i32]} else {[0,0]};
        let key=[sun.x.to_bits(),sun.y.to_bits(),sun.z.to_bits(),alpha.to_bits(),lod.to_bits(),cell[0] as u32,cell[1] as u32];
        let mut s=self.state.lock().unwrap();let changed=s.dirty||s.key!=Some(key);
        if changed{
            s.projection=if self.anchored {anchored(Vec3::new(cell[0] as f32*step,focus.y,cell[1] as f32*step),sun,self.reach,&range)}
                else {fit_min(&s.sources,sun,self.stage,self.min_extent)};
            s.key=Some(key);s.dirty=true;
            gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&Uniform{view_proj:s.projection.vp.to_cols_array_2d(),params:[2.*s.projection.extent/SIZE as f32,0.8,1.,if self.stage {1.+lod}else{0.}]}));}
        Some((s.projection,changed))
    }
    /// Only submission makes the pending cache contents reusable. A frame
    /// which fails validation earlier must rebuild on its next attempt.
    pub fn submitted(&self) {if self.enabled{self.state.lock().unwrap().dirty=false;}}
}

#[cfg(test)]mod tests{
    use super::*;
    fn source(p:Vec3,r:f32,flags:u32)->GpuInstance {let mut i=GpuInstance::zeroed();i.rows[0][0]=1.;i.rows[1][1]=1.;i.rows[2][2]=1.;i.scale=1.;i.sphere=[p.x,p.y,p.z,r];i.cluster_count=1;i.flags=flags;i}
    #[test]fn stage_sun_owns_static_solids_but_never_live_slots_or_skinning(){
        let mut scene=Scene::default();scene.stage_sun=true;
        scene.instances=vec![source(Vec3::ZERO,1.,SOLID),source(Vec3::ZERO,1.,FOLIAGE),source(Vec3::ZERO,1.,0),source(Vec3::ZERO,1.,SKINNED)];
        scene.dynamic_instances.push(2..3);
        assert_eq!(owners(&scene,true,false),vec![true,true,false,false]);
        // Empty at level creation still remains dynamic when filled later.
        scene.instances[2].cluster_count=0;assert!(!owners(&scene,true,false)[2]);
    }
    #[test]fn stage_fit_includes_buildings_and_roads_outside_the_forest(){
        let sources=[source(Vec3::ZERO,20.,FOLIAGE),source(Vec3::new(6000.,200.,0.),300.,SOLID)];
        let sun=Vec3::new(0.3,0.8,0.4).normalize();let p=fit(&sources,sun,true);
        for source in sources {let q=p.vp.project_point3(Vec3::from_slice(&source.sphere));assert!(q.x.abs()<0.8 && q.y.abs()<0.8);}
    }
    #[test]fn far_sun_fit_covers_all_crowns_and_remote_casters(){
        let sources=[source(Vec3::new(-1800.,120.,350.),15.,FOLIAGE),source(Vec3::new(2400.,600.,-2600.),25.,FOLIAGE),source(Vec3::new(0.,900.,5000.),600.,0)];
        for sun in [Vec3::Y,Vec3::new(0.9,0.035,0.3).normalize(),Vec3::new(0.3,0.8,0.4).normalize()]{let p=fit(&sources,sun,false);
            for i in &sources[..2]{let q=p.vp.project_point3(Vec3::from_slice(&i.sphere));assert!(q.x.abs()<0.8&&q.y.abs()<0.8);assert!(q.z>0.&&q.z<1.);}
            assert!(p.extent>=1024.);assert!(p.vp.is_finite());
            let q=p.vp.project_point3(Vec3::from_slice(&sources[2].sphere));assert!(q.z>0.&&q.z<1.);
            for i in &sources {for bits in 0..8 {let d=Vec3::new(if bits&1==0{-1.}else{1.},if bits&2==0{-1.}else{1.},if bits&4==0{-1.}else{1.});
                let q=p.vp.project_point3(Vec3::from_slice(&i.sphere)+d*i.sphere[3]);assert!(q.z>0.05&&q.z<0.95,"static caster outside full-weight depth: {q:?}");}}
        }
    }
    #[test]fn far_sun_static_ownership_excludes_every_animated_class(){for f in [SOLID,NO_SHADOW,LIVE_GRASS,WIND_GRASS,PART_WIND]{assert!(!eligible(&source(Vec3::ZERO,1.,f),false));}assert!(eligible(&source(Vec3::ZERO,1.,FOLIAGE),false));assert!(eligible(&source(Vec3::ZERO,1.,0),false));assert_eq!(std::mem::size_of::<Uniform>(),80);}
    #[test]fn far_sun_fit_is_bit_exact_across_repeated_views(){let s=[source(Vec3::new(450.,20.,-900.),12.,FOLIAGE)];let a=fit(&s,Vec3::new(0.4,0.8,0.2).normalize(),false);for _ in 0..4{let b=fit(&s,Vec3::new(0.4,0.8,0.2).normalize(),false);assert_eq!(a.vp.to_cols_array(),b.vp.to_cols_array());}}
    #[test]
    #[ignore = "requires GPU; run serially without a game"]
    fn explicit_cache_reuses_and_only_tracks_selected_sources(){
        let gpu=Gpu::new().unwrap();let mut scene=Scene::default();
        scene.instances=vec![source(Vec3::ZERO,20.,0),source(Vec3::X*40.,1.,0)];
        scene.static_sun_instances=vec![0];
        let cache=Resources::new(&gpu,&scene);
        assert_eq!(cache.state.lock().unwrap().owners,vec![true,false]);
        let sun=Vec3::new(0.3,0.8,0.4).normalize();
        assert!(cache.prepare(&gpu,sun,0.5,0.5,Vec3::ZERO,|_,_|[-512.,512.]).unwrap().1);
        cache.submitted();
        assert!(!cache.prepare(&gpu,sun,0.5,0.5,Vec3::ZERO,|_,_|[-512.,512.]).unwrap().1);
        scene.instances[1].rows[0][3]+=10.;
        cache.update(&gpu,1,&scene.instances[1..]);
        assert!(!cache.prepare(&gpu,sun,0.5,0.5,Vec3::ZERO,|_,_|[-512.,512.]).unwrap().1);
        scene.instances[0].rows[0][3]+=10.;scene.instances[0].sphere[0]+=10.;
        cache.update(&gpu,0,&scene.instances[..1]);
        assert!(cache.prepare(&gpu,sun,0.5,0.5,Vec3::ZERO,|_,_|[-512.,512.]).unwrap().1);
        cache.submitted();
        assert!(cache.prepare(&gpu,Vec3::Y,0.5,0.5,Vec3::ZERO,|_,_|[-512.,512.]).unwrap().1);
    }
    #[test]fn explicit_stadium_cache_fits_small_scene_without_camera_dependence(){
        let sources=[source(Vec3::new(-120.,30.,0.),40.,0),source(Vec3::new(120.,40.,0.),40.,0)];
        for sun in [Vec3::Y,Vec3::new(0.9,0.1,0.3).normalize()] {
            let p=fit_min(&sources,sun,true,128.);
            assert!(p.extent<1024.);
            for i in &sources {for bits in 0..8 {
                let d=Vec3::new(if bits&1==0{-1.}else{1.},if bits&2==0{-1.}else{1.},if bits&4==0{-1.}else{1.});
                let q=p.vp.project_point3(Vec3::from_slice(&i.sphere)+d*i.sphere[3]);
                assert!(q.x.abs()<0.8 && q.y.abs()<0.8 && q.z>0.05 && q.z<0.95);
            }}
        }
    }
    #[test]fn far_sun_lod_is_bounded_without_losing_source_reference(){
        for v in [None,Some("bad"),Some("NaN"),Some("inf"),Some("-1"),Some("20")]{assert_eq!(configured_lod(v),0.5);}
        assert_eq!(configured_lod(Some("0")),0.0);assert_eq!(configured_lod(Some("0.25")),0.25);
        assert_eq!(effective_lod(0.5,1.0),0.5);assert_eq!(effective_lod(0.5,0.0),0.0);
        assert_eq!(effective_lod(0.5,0.1),0.2);assert_eq!(effective_lod(0.0,1.0),0.0);
    }
}
