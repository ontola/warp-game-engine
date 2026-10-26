//! Bounded, shared world-light-space pages for static foliage sunlight.
//! This is CPU-requested source-alpha shadow paging, not UE VSM feedback.
//! Wind uses the source rest pose; cars, grass and articulated actors remain
//! in the existing dynamic maps. Missing pages are explicitly unresolved.
use crate::{Camera,Gpu,scene::{Scene,GpuInstance,FOLIAGE,SOLID,NO_SHADOW,LIVE_GRASS,WIND_GRASS,PART_WIND}};
use bytemuck::{Pod,Zeroable};
use glam::{Mat4,Vec3};
use std::collections::{BTreeMap,HashMap,HashSet};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

pub(crate) const PAGE_SIZE:u32=512;
pub(crate) const SIZE:u32=PAGE_SIZE;
pub(crate) const GUARD_TEXELS:u32=4;
pub(crate) const PAGE_INTERIOR:u32=PAGE_SIZE-2*GUARD_TEXELS;
pub(crate) const PAGE_WORLD:f32=64.;
pub(crate) const WORLD_TEXEL:f32=PAGE_WORLD/PAGE_INTERIOR as f32;
pub(crate) const MAX_PAGES:usize=256;
const MAX_VIRTUAL_TILES:usize=65_536;
const MAX_INDEX_TOUCHES:usize=8_000_000;

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
pub(crate) struct Header {
    /// XY are absolute world metres in the sun basis; Z is shared 0..1 depth.
    pub world_to_light:[[f32;4];4],
    /// World page width, inner UV fraction, guard UV fraction, inverse size.
    pub geometry:[f32;4],
    /// Published entries, revision, enabled, physical array layers.
    pub counts:[u32;4],
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable,Debug)]
pub(crate) struct PageEntry {
    /// Sorted (x,y), physical layer or -1 for known empty, revision.
    pub key_layer:[i32;4],
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Hash,PartialOrd,Ord)]
struct Key(i32,i32);
#[derive(Clone,Copy)]
pub(crate) struct ReceiverView {pub camera:Camera,pub aspect:f32}
#[derive(Clone,Copy,Debug)]
pub(crate) struct PageUpdate {
    pub layer:u32,pub vp:Mat4,pub eye:Vec3,pub extent:f32,pub capacity:u32,pub key:[i32;2],
}
pub(crate) struct Plan {
    pub updates:Vec<PageUpdate>,
    pub requested:u32,pub resident:u32,pub missing:u32,pub known_empty:u32,
    pub cache_hits:u32,pub virtual_pages:u32,pub revision:u32,pub texel_m:f32,
    pub cpu_index_rebuilt:bool,pub request_candidates:u32,
    serial:u64,
}
impl Plan {pub(crate) fn disabled()->Self {Self{updates:Vec::new(),requested:0,resident:0,missing:0,
    known_empty:0,cache_hits:0,virtual_pages:0,revision:0,texel_m:WORLD_TEXEL,
    cpu_index_rebuilt:false,request_candidates:0,serial:0}}}

#[derive(Clone,Copy,Debug)]
struct Bounds {lo:Vec3,hi:Vec3}
impl Default for Bounds {fn default()->Self {Self{lo:Vec3::splat(f32::INFINITY),hi:Vec3::splat(f32::NEG_INFINITY)}}}
impl Bounds {
    fn add(&mut self,p:Vec3,r:f32){self.lo=self.lo.min(p-Vec3::splat(r));self.hi=self.hi.max(p+Vec3::splat(r));}
    fn sphere(&self)->Option<(Vec3,f32)>{self.lo.is_finite().then(||((self.lo+self.hi)*0.5,(self.hi-self.lo).length()*0.5))}
}
#[derive(Default)]
struct Tile {foliage:Bounds,other:Bounds,clusters:u64}
#[derive(Clone,Copy,PartialEq)]
struct Receiver {sphere:[f32;4],foliage:bool}
#[derive(Clone,Copy,Default)]
struct Slot {key:Option<Key>,valid:bool,empty:bool,last_used:u64,pending:u64}
#[derive(Clone,Copy)]
struct Request {key:Key,crown:bool,score:f32}

struct State {
    sources:Vec<GpuInstance>,owners:Vec<bool>,receiver_owners:Vec<bool>,receivers:Vec<Option<Receiver>>,
    // Receiver-only objects may move without changing a single cached caster.
    // In particular, wheel instances need not carry the body's SOLID flag.
    dynamic_receivers:HashSet<usize>,
    page_world:f32,
    dirty:bool,key:Option<[u32;5]>,revision:u32,serial:u64,world_to_light:Mat4,sun:Vec3,depth:[f32;2],
    tiles:BTreeMap<Key,Tile>,slots:Vec<Slot>,
    last_views:Vec<u32>,requests:Vec<Request>,
}
pub(crate) struct Resources {
    pub enabled:bool,pub lod_pixels:f32,
    pub metadata:wgpu::Buffer,pub instances:wgpu::Buffer,pub wind:wgpu::Buffer,
    pub sampled:wgpu::TextureView,pub empty:wgpu::TextureView,pub capacity:u32,
    texture:wgpu::Texture,state:Mutex<State>,
}

fn caster(i:&GpuInstance)->bool {
    i.cluster_count>0 && i.flags&FOLIAGE!=0
        && i.flags&(SOLID|NO_SHADOW|LIVE_GRASS|WIND_GRASS|PART_WIND)==0
}
fn receiver(i:&GpuInstance)->bool {
    i.cluster_count>0 && i.flags&(SOLID|LIVE_GRASS|WIND_GRASS|PART_WIND)==0
}
fn valid_sphere(i:&GpuInstance)->bool {
    i.sphere.iter().all(|v|v.is_finite()) && i.sphere[3]>=0.
}
fn receiver_record(i:&GpuInstance)->Option<Receiver>{
    if !receiver(i)||!valid_sphere(i){return None;}
    let mut sphere=i.sphere;
    // Requests contain any permitted affine tree wind, although the cached
    // caster itself is deliberately evaluated at rest pose.
    if i.flags&FOLIAGE!=0 {sphere[3]+=0.08*((i.sphere[1]-i.rows[1][3]).abs()+i.sphere[3]);}
    Some(Receiver{sphere,foliage:i.flags&FOLIAGE!=0})
}
fn hidden(mut i:GpuInstance)->GpuInstance {i.cluster_count=0;i.flags|=NO_SHADOW;i}
fn page_world_setting(value:Option<&str>)->f32 {
    match value {Some("128")=>128.,Some("256")=>256.,_=>PAGE_WORLD}
}
fn range_keys(q:Vec3,r:f32,page_world:f32)->Result<(Key,Key,usize),String>{
    let lo=((q.truncate()-glam::Vec2::splat(r))/page_world).floor();
    let hi=((q.truncate()+glam::Vec2::splat(r))/page_world).floor();
    if !lo.is_finite()||!hi.is_finite()||lo.min_element()<i32::MIN as f32+1024.||hi.max_element()>i32::MAX as f32-1024.{return Err("foliage sun page coordinates exceed finite world bounds".into());}
    let a=Key(lo.x as i32,lo.y as i32);let b=Key(hi.x as i32,hi.y as i32);
    let count=(b.0 as i64-a.0 as i64+1).checked_mul(b.1 as i64-a.1 as i64+1).ok_or("foliage page range overflow")?;
    if count<=0||count as u64>MAX_INDEX_TOUCHES as u64{return Err("foliage page sphere exceeds bounded indexing work".into());}
    Ok((a,b,count as usize))
}
fn matrix_for_page(world:Mat4,key:Key,page_world:f32)->Mat4 {
    let extent=page_world*0.5+GUARD_TEXELS as f32*page_world/PAGE_INTERIOR as f32;
    let centre=glam::Vec2::new(key.0 as f32+0.5,key.1 as f32+0.5)*page_world;
    let mut m=world;
    for axis in [&mut m.x_axis,&mut m.y_axis,&mut m.z_axis]{axis.x/=extent;axis.y/=extent;}
    m.w_axis.x=(m.w_axis.x-centre.x)/extent;m.w_axis.y=(m.w_axis.y-centre.y)/extent;m
}
fn view_bits(views:&[ReceiverView])->Vec<u32>{
    views.iter().flat_map(|v|v.camera.pos.to_array().into_iter().chain(v.camera.target.to_array())
        .chain([v.camera.fov_y,v.camera.near,v.aspect]).map(f32::to_bits)).collect()
}
fn score(bounds:Bounds,views:&[ReceiverView])->Option<f32>{
    let (p,r)=bounds.sphere()?;let mut result:Option<f32>=None;
    for view in views {
        let c=view.camera;let f=(c.target-c.pos).normalize();
        let up=if f.y.abs()>0.99 {Vec3::Z}else{Vec3::Y};let right=f.cross(up).normalize();let up=right.cross(f);
        let q=p-c.pos;let z=f.dot(q);let ty=(c.fov_y*0.5).tan();let tx=ty*view.aspect;
        if z+r<c.near || (f*tx+right).normalize().dot(q)< -r || (f*tx-right).normalize().dot(q)< -r
            || (f*ty+up).normalize().dot(q)< -r || (f*ty-up).normalize().dot(q)< -r {continue;}
        let projected=(r/(z.max(c.near)*ty)).powi(2).min(16.);
        result=Some(result.unwrap_or(0.).max(projected));
    }result
}

impl State {
    /// Returns whether the static GPU caster record must be uploaded. Moving
    /// receiver-only geometry changes requests, never the cached Z transform
    /// or page contents. The old static union remains conservative until the
    /// next genuine caster/light rebuild; it can overrequest but cannot omit.
    fn update_source(&mut self,id:usize,i:GpuInstance)->bool {
        if !self.owners[id]&&!self.receiver_owners[id]{return false;}
        let next=if self.owners[id]&&caster(&i){i}else{hidden(i)};
        let next_receiver=if self.receiver_owners[id]{receiver_record(&i)}else{None};
        let old=self.sources[id];
        let changed=bytemuck::bytes_of(&next)!=bytemuck::bytes_of(&old);
        let caster_changed=changed&&(old.cluster_count>0||next.cluster_count>0);
        self.sources[id]=next;
        if self.receivers[id]!=next_receiver {
            self.receivers[id]=next_receiver;
            self.last_views.clear();
            if !caster_changed{self.dynamic_receivers.insert(id);}
        }
        if caster_changed{self.dirty=true;}
        caster_changed
    }
    fn rebuild(&mut self,sun:Vec3,alpha:f32,lod:f32)->Result<(),String>{
        if !sun.is_finite()||!sun.length_squared().is_finite()||sun.length_squared()<1e-12||!alpha.is_finite()||!lod.is_finite()||lod<0. {return Err("invalid foliage sun page light parameters".into());}
        self.sun=sun.normalize();
        let axis_z=self.sun;let axis_up=if axis_z.y.abs()>0.99{Vec3::Z}else{Vec3::Y};
        let axis_x=axis_up.cross(axis_z).normalize();let axis_y=axis_z.cross(axis_x);
        let mut low=f32::INFINITY;let mut high=f32::NEG_INFINITY;
        for r in self.receivers.iter().flatten(){let d=axis_z.dot(Vec3::from_slice(&r.sphere));low=low.min(d-r.sphere[3]);high=high.max(d+r.sphere[3]);}
        for i in self.sources.iter().filter(|i|i.cluster_count>0){let d=axis_z.dot(Vec3::from_slice(&i.sphere));low=low.min(d-i.sphere[3]);high=high.max(d+i.sphere[3]);}
        if !low.is_finite()||!high.is_finite(){low=-16.;high=16.;}
        let margin=1.+8.*f32::EPSILON*low.abs().max(high.abs());
        self.depth=[((low-margin)/16.).floor()*16.,((high+margin)/16.).ceil()*16.];
        self.world_to_light=crate::stability::stable_shadow_depth(Vec3::ZERO,self.sun,1.,1,self.depth);
        let mut tiles=BTreeMap::<Key,Tile>::new();let mut touches=0usize;
        for (id,entry) in self.receivers.iter().enumerate(){
            if self.dynamic_receivers.contains(&id){continue;}
            let Some(item)=entry else{continue;};
            let p=Vec3::from_slice(&item.sphere);let r=item.sphere[3];
            let q=Vec3::new(axis_x.dot(p),axis_y.dot(p),0.);let (lo,hi,count)=range_keys(q,r,self.page_world)?;
            touches=touches.checked_add(count).ok_or("foliage page index overflow")?;
            if touches>MAX_INDEX_TOUCHES{return Err("foliage receiver page indexing exceeds work budget".into());}
            for y in lo.1..=hi.1{for x in lo.0..=hi.0{
                let t=tiles.entry(Key(x,y)).or_default();
                if item.foliage{t.foliage.add(p,r);}else{t.other.add(p,r);}
            }}
            if tiles.len()>MAX_VIRTUAL_TILES{return Err("foliage receiver virtual page budget exceeded".into());}
        }
        for i in self.sources.iter().filter(|i|i.cluster_count>0){
            let p=Vec3::from_slice(&i.sphere);let r=i.sphere[3]+GUARD_TEXELS as f32*self.page_world/PAGE_INTERIOR as f32;
            let q=Vec3::new(axis_x.dot(p),axis_y.dot(p),0.);let (lo,hi,count)=range_keys(q,r,self.page_world)?;
            touches=touches.checked_add(count).ok_or("foliage caster page index overflow")?;
            if touches>MAX_INDEX_TOUCHES{return Err("foliage caster page indexing exceeds work budget".into());}
            for y in lo.1..=hi.1{for x in lo.0..=hi.0{
                // Complete caster-column occupancy is needed even where no
                // static receiver currently lives: a moving receiver can
                // request that column later without rebuilding all casters.
                let tile=tiles.entry(Key(x,y)).or_default();
                tile.clusters+=i.cluster_count as u64;
                if tile.clusters>crate::frame::MAX_VISIBLE as u64{return Err("foliage page exceeds visible-cluster capacity; refusing truncated shadows".into());}
            }}
            if tiles.len()>MAX_VIRTUAL_TILES{return Err("foliage caster virtual page budget exceeded".into());}
        }
        self.tiles=tiles;self.key=Some([sun.x.to_bits(),sun.y.to_bits(),sun.z.to_bits(),alpha.to_bits(),lod.to_bits()]);
        self.revision=self.revision.wrapping_add(1).max(1);self.dirty=false;
        self.slots.fill(Slot::default());self.last_views.clear();self.requests.clear();Ok(())
    }
    fn refresh_requests(&mut self,views:&[ReceiverView])->Result<(),String>{
        if views.is_empty()||views.len()>4{return Err("foliage sun pages require one to four receiver views".into());}
        for v in views{let c=v.camera;
            if !c.pos.is_finite()||!c.target.is_finite()||c.pos.distance_squared(c.target)<1e-12
                ||!c.fov_y.is_finite()||!(0.01..3.13).contains(&c.fov_y)||!c.near.is_finite()||c.near<=0.
                ||!v.aspect.is_finite()||v.aspect<=0.{return Err("invalid foliage page receiver frustum".into());}
        }
        let key=view_bits(views);if key==self.last_views{return Ok(());}
        self.requests.clear();
        for (&key,tile) in &self.tiles {
            if let Some(s)=score(tile.foliage,views){self.requests.push(Request{key,crown:true,score:s});}
            else if let Some(s)=score(tile.other,views){self.requests.push(Request{key,crown:false,score:s});}
        }
        if !self.dynamic_receivers.is_empty(){
            let axis_x=self.world_to_light.row(0).truncate();let axis_y=self.world_to_light.row(1).truncate();
            let mut dynamic=HashMap::<Key,Request>::new();let mut touches=0usize;
            for id in &self.dynamic_receivers {
                let Some(receiver)=self.receivers[*id] else{continue;};
                let p=Vec3::from_slice(&receiver.sphere);let radius=receiver.sphere[3];
                let mut bounds=Bounds::default();bounds.add(p,radius);
                let Some(value)=score(bounds,views) else{continue;};
                let (lo,hi,count)=range_keys(Vec3::new(axis_x.dot(p),axis_y.dot(p),0.),radius,self.page_world)?;
                touches=touches.checked_add(count).ok_or("dynamic foliage receiver request overflow")?;
                if touches>MAX_INDEX_TOUCHES{return Err("dynamic foliage receiver requests exceed bounded work".into());}
                for y in lo.1..=hi.1{for x in lo.0..=hi.0{
                    let key=Key(x,y);let r=dynamic.entry(key).or_insert(Request{key,crown:receiver.foliage,score:value});
                    r.crown|=receiver.foliage;r.score=r.score.max(value);
                }}
            }
            for r in &mut self.requests{if let Some(other)=dynamic.remove(&r.key){r.crown|=other.crown;r.score=r.score.max(other.score);}}
            self.requests.extend(dynamic.into_values());
        }
        self.requests.sort_by(|a,b|b.crown.cmp(&a.crown).then_with(||b.score.total_cmp(&a.score)).then_with(||a.key.cmp(&b.key)));
        self.last_views=key;Ok(())
    }
    fn plan(&mut self,views:&[ReceiverView],max_updates:usize,rebuilt:bool)->Result<Plan,String>{
        self.refresh_requests(views)?;self.serial=self.serial.wrapping_add(1).max(1);
        let chosen:Vec<_>=self.requests.iter().take(self.slots.len()).map(|r|r.key).collect();
        let selected:HashSet<_>=chosen.iter().copied().collect();
        let mut positions:HashMap<_,_>=self.slots.iter().enumerate().filter_map(|(i,s)|s.key.map(|k|(k,i))).collect();
        let mut updates=Vec::new();let mut hits=0;
        // Old pending work never becomes sampleable merely because another
        // frame started. It must be explicitly scheduled and encoded again.
        for s in &mut self.slots{s.pending=0;}
        for key in chosen {
            // No entry is known empty only because rebuild indexed ALL
            // guarded static caster columns, including off-camera casters.
            let clusters=self.tiles.get(&key).map_or(0,|t|t.clusters) as u32;let empty=clusters==0;
            if let Some(&index)=positions.get(&key){
                let s=&mut self.slots[index];s.last_used=self.serial;
                if s.valid{hits+=1;continue;}
                if !empty&&updates.len()>=max_updates{continue;}
            }else if !empty&&updates.len()>=max_updates{continue;}
            let index=if let Some(&index)=positions.get(&key){index}else{
                let candidate=self.slots.iter().enumerate().filter(|(_,s)|s.key.is_none_or(|k|!selected.contains(&k)))
                    .min_by_key(|(_,s)|(s.key.is_some(),s.last_used)).map(|(i,_)|i);
                let Some(index)=candidate else{continue;};
                if let Some(old)=self.slots[index].key{positions.remove(&old);}
                positions.insert(key,index);index
            };
            self.slots[index]=Slot{key:Some(key),valid:empty,empty,last_used:self.serial,pending:if empty{0}else{self.serial}};
            if !empty {
                let axis_x=self.world_to_light.row(0).truncate();let axis_y=self.world_to_light.row(1).truncate();
                let centre=axis_x*((key.0 as f32+0.5)*self.page_world)+axis_y*((key.1 as f32+0.5)*self.page_world);
                updates.push(PageUpdate{layer:index as u32,vp:matrix_for_page(self.world_to_light,key,self.page_world),
                    eye:crate::stability::shadow_eye(centre,self.sun,self.depth),extent:self.page_world*0.5+GUARD_TEXELS as f32*self.page_world/PAGE_INTERIOR as f32,
                    capacity:clusters.max(1),key:[key.0,key.1]});
            }
        }
        let available:HashSet<_>=self.slots.iter().filter(|s|s.valid||s.pending==self.serial).filter_map(|s|s.key).collect();
        let resident=self.requests.iter().filter(|r|available.contains(&r.key)).count();
        let known_empty=self.requests.iter().filter(|r|positions.get(&r.key).is_some_and(|&i|self.slots[i].valid&&self.slots[i].empty)).count();
        Ok(Plan{updates,requested:self.requests.len()as u32,resident:resident as u32,
            missing:(self.requests.len()-resident)as u32,known_empty:known_empty as u32,cache_hits:hits,
            virtual_pages:self.tiles.len()as u32,revision:self.revision,texel_m:self.page_world/PAGE_INTERIOR as f32,
            cpu_index_rebuilt:rebuilt,request_candidates:self.tiles.len()as u32,serial:self.serial})
    }
    fn table(&self)->Vec<PageEntry>{
        let mut entries:Vec<_>=self.slots.iter().enumerate().filter(|(_,s)|s.valid||s.pending==self.serial)
            .filter_map(|(layer,s)|s.key.map(|key|PageEntry{key_layer:[key.0,key.1,if s.empty{-1}else{layer as i32},self.revision as i32]})).collect();
        entries.sort_by_key(|e|(e.key_layer[0],e.key_layer[1]));entries
    }
    fn submitted(&mut self,plan:&Plan){
        if plan.revision!=self.revision||plan.serial!=self.serial{return;}
        for u in &plan.updates{let s=&mut self.slots[u.layer as usize];
            if s.key==Some(Key(u.key[0],u.key[1]))&&s.pending==plan.serial{s.valid=true;s.pending=0;}
        }
    }
}

impl Resources {
    pub fn new(gpu:&Gpu,scene:&Scene)->Result<Self,String>{
        let enabled=std::env::var("KERN_PAGED_FOLIAGE_SUN").as_deref()==Ok("1");
        // Diagnostic coverage/precision tradeoff; physical memory and update
        // budget do not change. Keep the proven fine-page default until the
        // actual distant-crown comparison accepts a coarser world footprint.
        let page_world=page_world_setting(std::env::var("KERN_FOLIAGE_SUN_PAGE_METRES").ok().as_deref());
        let lod_pixels=std::env::var("KERN_FOLIAGE_SUN_LOD").ok().and_then(|s|s.parse::<f32>().ok())
            .filter(|x|x.is_finite()&&*x>=0.&&*x<=2.).unwrap_or(0.5);
        if enabled&&(gpu.limits.max_texture_array_layers<MAX_PAGES as u32||gpu.limits.max_texture_dimension_2d<PAGE_SIZE){return Err("device cannot allocate bounded foliage sun page array".into());}
        let mut owners:Vec<_>=if enabled{scene.instances.iter().map(caster).collect()}else{Vec::new()};
        let mut receiver_owners:Vec<_>=if enabled{scene.instances.iter().map(receiver).collect()}else{Vec::new()};
        if enabled{if let Some(animation)=&scene.baked_animations{for p in &animation.parts{owners[p.owner as usize]=false;receiver_owners[p.owner as usize]=false;}}}
        if enabled&&scene.instances.iter().enumerate().any(|(i,s)|(owners[i]||receiver_owners[i])&&!valid_sphere(s)){return Err("invalid foliage sun source bounds".into());}
        let sources:Vec<_>=scene.instances.iter().zip(&owners).map(|(&i,&own)|if own{i}else{hidden(i)}).collect();
        let receivers:Vec<_>=scene.instances.iter().zip(&receiver_owners).map(|(i,own)|if *own{receiver_record(i)}else{None}).collect();
        let capacity=sources.iter().map(|i|i.cluster_count as u64).sum::<u64>().clamp(1,crate::frame::MAX_VISIBLE as u64)as u32;
        let data=if enabled&&!sources.is_empty(){bytemuck::cast_slice(&sources)}else{&[0u8;80]};
        let instances=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("foliage sun static sources"),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
        let wind=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("foliage sun rest pose"),size:if enabled{(sources.len().max(1)*16)as u64}else{16},usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
        let metadata=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("foliage sun page table"),size:(std::mem::size_of::<Header>()+MAX_PAGES*std::mem::size_of::<PageEntry>())as u64,
            usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("foliage sun bounded pages"),size:wgpu::Extent3d{width:if enabled{PAGE_SIZE}else{1},height:if enabled{PAGE_SIZE}else{1},depth_or_array_layers:if enabled{MAX_PAGES as u32}else{1}},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let empty_texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("foliage sun empty array binding"),size:wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
            dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let sampled=texture.create_view(&wgpu::TextureViewDescriptor{dimension:Some(wgpu::TextureViewDimension::D2Array),..Default::default()});
        let empty=empty_texture.create_view(&wgpu::TextureViewDescriptor{dimension:Some(wgpu::TextureViewDimension::D2Array),..Default::default()});
        Ok(Self{enabled,lod_pixels,metadata,instances,wind,sampled,empty,capacity,texture,
            state:Mutex::new(State{sources,owners,receiver_owners,receivers,dynamic_receivers:HashSet::new(),page_world,dirty:true,key:None,revision:0,serial:0,
                world_to_light:Mat4::IDENTITY,sun:Vec3::Y,depth:[-16.,16.],tiles:BTreeMap::new(),
                slots:vec![Slot::default();if enabled{MAX_PAGES}else{0}],last_views:Vec::new(),requests:Vec::new()})})
    }
    pub fn layer_view(&self,layer:u32)->wgpu::TextureView{
        assert!(self.enabled&&layer<MAX_PAGES as u32);
        self.texture.create_view(&wgpu::TextureViewDescriptor{label:Some("foliage sun render page"),dimension:Some(wgpu::TextureViewDimension::D2),base_array_layer:layer,array_layer_count:Some(1),..Default::default()})
    }
    pub fn bytes(&self)->u64{if self.enabled{PAGE_SIZE as u64*PAGE_SIZE as u64*4*MAX_PAGES as u64}else{4}}
    /// Caster ownership is declared at construction. Caster transforms/flags
    /// invalidate pages; moving receivers only update their page requests.
    pub fn update(&self,gpu:&Gpu,first:usize,input:&[GpuInstance]){
        if !self.enabled{return;}let mut state=self.state.lock().unwrap();
        assert!(first+input.len()<=state.sources.len());
        for (n,&i) in input.iter().enumerate(){let id=first+n;
            if state.update_source(id,i){
                gpu.queue.write_buffer(&self.instances,(id*std::mem::size_of::<GpuInstance>())as u64,bytemuck::bytes_of(&state.sources[id]));}
        }
    }
    /// All returned updates MUST be cleared/rastered before any view samples
    /// the published table in this submission. Call submitted only after the
    /// queue accepts that submission. On error the next prepare reschedules.
    pub fn prepare(&self,gpu:&Gpu,sun:Vec3,alpha:f32,lod:f32,views:&[ReceiverView],max_updates:usize)->Result<Plan,String>{
        if !self.enabled{return Ok(Plan::disabled());}
        let mut state=self.state.lock().unwrap();let key=[sun.x.to_bits(),sun.y.to_bits(),sun.z.to_bits(),alpha.to_bits(),lod.to_bits()];
        let rebuilt=state.dirty||state.key!=Some(key);if rebuilt{state.rebuild(sun,alpha,lod)?;}
        let plan=state.plan(views,max_updates.min(MAX_PAGES),rebuilt)?;let table=state.table();
        let header=Header{world_to_light:state.world_to_light.to_cols_array_2d(),
            geometry:[state.page_world,PAGE_INTERIOR as f32/PAGE_SIZE as f32,GUARD_TEXELS as f32/PAGE_SIZE as f32,1./PAGE_SIZE as f32],
            counts:[table.len()as u32,state.revision,1,MAX_PAGES as u32]};
        let mut bytes=bytemuck::bytes_of(&header).to_vec();bytes.extend_from_slice(bytemuck::cast_slice(&table));
        gpu.queue.write_buffer(&self.metadata,0,&bytes);Ok(plan)
    }
    pub fn submitted(&self,plan:&Plan){if self.enabled{self.state.lock().unwrap().submitted(plan);}}
}

#[cfg(test)]mod tests{
    use super::*;
    fn instance(p:Vec3,r:f32,flags:u32)->GpuInstance{let mut i=GpuInstance::zeroed();i.rows[0][0]=1.;i.rows[1][1]=1.;i.rows[2][2]=1.;i.rows[0][3]=p.x;i.rows[1][3]=p.y;i.rows[2][3]=p.z;i.sphere=[p.x,p.y,p.z,r];i.scale=1.;i.cluster_count=24;i.flags=flags;i}
    fn state(instances:&[GpuInstance],pages:usize)->State{State{sources:instances.iter().copied().map(|i|if caster(&i){i}else{hidden(i)}).collect(),owners:instances.iter().map(caster).collect(),receiver_owners:instances.iter().map(receiver).collect(),receivers:instances.iter().map(receiver_record).collect(),dynamic_receivers:HashSet::new(),page_world:PAGE_WORLD,dirty:true,key:None,revision:0,serial:0,world_to_light:Mat4::IDENTITY,sun:Vec3::Y,depth:[-16.,16.],tiles:BTreeMap::new(),slots:vec![Slot::default();pages],last_views:Vec::new(),requests:Vec::new()}}
    fn view()->ReceiverView{ReceiverView{camera:Camera{pos:Vec3::new(0.,25.,150.),target:Vec3::ZERO,fov_y:1.3,near:0.1},aspect:16./9.}}
    #[test]fn foliage_pages_abi_memory_guard_and_projection_agree(){
        assert_eq!(std::mem::size_of::<Header>(),96);assert_eq!(std::mem::size_of::<PageEntry>(),16);
        assert_eq!(PAGE_SIZE as usize*PAGE_SIZE as usize*4*MAX_PAGES,256*1024*1024);
        let mut s=state(&[instance(Vec3::new(500.,80.,-900.),12.,FOLIAGE)],8);s.rebuild(Vec3::new(0.4,0.8,0.2).normalize(),0.42,0.5).unwrap();
        for page_world in [64.,128.,256.] {for key in [Key(-18,-5),Key(0,0),Key(7,12)]{let vp=matrix_for_page(s.world_to_light,key,page_world);
            let x=s.world_to_light.row(0).truncate();let y=s.world_to_light.row(1).truncate();
            for local in [glam::Vec2::ZERO,glam::Vec2::splat(page_world*0.5),glam::Vec2::splat(page_world)]{
                let p=x*(key.0 as f32*page_world+local.x)+y*(key.1 as f32*page_world+local.y);
                let c=vp.project_point3(p);let uv=glam::Vec2::new(c.x*0.5+0.5,0.5-c.y*0.5);
                let expected=glam::Vec2::new(local.x/page_world,1.-local.y/page_world)*(504./512.)+glam::Vec2::splat(4./512.);
                assert!(uv.abs_diff_eq(expected,2e-5));assert!((c.z-s.world_to_light.project_point3(p).z).abs()<1e-6);
            }
        }}
    }
    #[test]fn foliage_pages_only_static_foliage_casts(){
        assert!(caster(&instance(Vec3::ZERO,1.,FOLIAGE)));assert!(!caster(&instance(Vec3::ZERO,1.,0)));
        for flag in [SOLID,NO_SHADOW,LIVE_GRASS,WIND_GRASS,PART_WIND]{assert!(!caster(&instance(Vec3::ZERO,1.,FOLIAGE|flag)));}
        assert!(receiver(&instance(Vec3::ZERO,1.,NO_SHADOW)));
    }
    #[test]fn foliage_pages_requests_and_cache_are_shared_and_submission_safe(){
        let mut s=state(&[instance(Vec3::new(0.,8.,0.),8.,FOLIAGE),instance(Vec3::new(0.,-3.,0.),90.,0)],32);
        s.rebuild(Vec3::new(0.4,0.8,0.2).normalize(),0.42,0.5).unwrap();let views=[view();4];
        let a=s.plan(&views,1,true).unwrap();assert_eq!(a.updates.len(),1);assert!(a.missing>0);assert!(!s.table().is_empty());
        let key=a.updates[0].key;let b=s.plan(&views,1,false).unwrap();assert_eq!(b.updates[0].key,key,"unsubmitted work cannot become a cache hit");
        s.submitted(&a);assert!(!s.slots[b.updates[0].layer as usize].valid,"stale submission cannot commit a newer plan");
        s.submitted(&b);let c=s.plan(&[view()],32,false).unwrap();assert_eq!(b.requested,c.requested,"identical four views do not multiply world requests");
        assert!(c.cache_hits>=1);s.submitted(&c);let d=s.plan(&views,32,false).unwrap();assert!(d.updates.is_empty());
        assert_eq!(d.requested,d.resident+d.missing);assert!(s.table().windows(2).all(|v|v[0].key_layer[..2]<v[1].key_layer[..2]));
    }
    #[test]fn foliage_pages_budget_reports_missing_and_empty_is_explicit(){
        let instances:Vec<_>=(-10..=10).map(|x|instance(Vec3::new(x as f32*32.,5.,0.),4.,FOLIAGE)).chain([instance(Vec3::new(0.,-5.,0.),450.,0)]).collect();
        let mut s=state(&instances,2);s.rebuild(Vec3::Y,0.42,0.5).unwrap();let a=s.plan(&[view()],2,true).unwrap();
        assert!(a.requested>2);assert!(a.missing>0);assert!(s.table().len()<=2);assert_eq!(a.requested,a.resident+a.missing);
        let mut empty=state(&[instance(Vec3::ZERO,100.,0)],32);empty.rebuild(Vec3::Y,0.42,0.5).unwrap();let e=empty.plan(&[view()],0,true).unwrap();
        assert!(e.updates.is_empty());assert!(e.known_empty>0);assert!(empty.table().iter().all(|e|e.key_layer[2]==-1));
    }
    #[test]fn foliage_pages_sun_or_source_revision_invalidates_and_bounds_fail_closed(){
        let mut s=state(&[instance(Vec3::ZERO,5.,FOLIAGE)],8);s.rebuild(Vec3::Y,0.42,0.5).unwrap();let a=s.plan(&[view()],8,true).unwrap();s.submitted(&a);
        s.rebuild(Vec3::new(0.2,0.8,0.1).normalize(),0.42,0.5).unwrap();assert!(s.table().is_empty());assert!(s.revision>a.revision);
        assert!(range_keys(Vec3::ZERO,1e9,PAGE_WORLD).is_err());assert!(s.plan(&[],8,false).is_err());
        assert!(s.rebuild(Vec3::ZERO,0.42,0.5).is_err());
    }
    #[test]fn foliage_pages_world_footprint_changes_precision_not_physical_budget(){
        assert_eq!(page_world_setting(None),64.);assert_eq!(page_world_setting(Some("64")),64.);
        assert_eq!(page_world_setting(Some("128")),128.);assert_eq!(page_world_setting(Some("256")),256.);
        for v in ["0","65","512","NaN"]{assert_eq!(page_world_setting(Some(v)),64.);}
        let instances:Vec<_>=(-8..=8).flat_map(|z|(-8..=8).map(move|x|instance(Vec3::new(x as f32*64.,5.,z as f32*64.),4.,FOLIAGE))).collect();
        let mut counts=Vec::new();
        for width in [64.,128.,256.] {
            let mut s=state(&instances,MAX_PAGES);s.page_world=width;s.rebuild(Vec3::Y,0.42,0.5).unwrap();
            let p=s.plan(&[view()],MAX_PAGES,true).unwrap();
            assert_eq!(p.texel_m,width/504.);assert_eq!(s.slots.len(),MAX_PAGES);
            assert!(p.updates.iter().all(|u|(u.extent-(width*0.5+4.*width/504.)).abs()<1e-5));
            counts.push(s.tiles.len());
            let (low,high,n)=range_keys(Vec3::new(-width*0.5,width*0.5,0.),width*0.1,width).unwrap();
            assert_eq!((low,high,n),(Key(-1,0),Key(-1,0),1));
        }
        assert!(counts[0]>counts[1]&&counts[1]>counts[2],"larger world pages cover the identical receiver forest with fewer virtual entries");
    }
    #[test]fn foliage_pages_spinning_moving_wheel_does_not_rebuild_casters(){
        let tree=instance(Vec3::new(0.,8.,0.),8.,FOLIAGE);
        let floor=instance(Vec3::new(0.,-3.,0.),150.,0);
        let wheel=instance(Vec3::new(2.,1.,2.),0.5,0); // Actual car wheels have flags0.
        let mut s=state(&[tree,floor,wheel],MAX_PAGES);
        s.rebuild(Vec3::Y,0.42,0.5).unwrap();let p=s.plan(&[view()],MAX_PAGES,true).unwrap();s.submitted(&p);
        let original_projection=s.world_to_light;let original_depth=s.depth;let revision=s.revision;
        // Rotation can alter the source transform without altering its sphere.
        let mut rotating=wheel;rotating.rows[0][0]=0.;rotating.rows[0][2]=1.;
        let cached_views=s.last_views.clone();assert!(!s.update_source(2,rotating));
        assert_eq!(s.last_views,cached_views,"pure wheel spin needs no receiver request refresh");
        for frame in 1..=120 {
            let mut moving=instance(Vec3::new(2.+frame as f32*0.01,1.,2.),0.5,0);
            moving.rows[0][0]=(frame as f32*0.1).cos();moving.rows[0][2]=(frame as f32*0.1).sin();
            assert!(!s.update_source(2,moving));assert!(!s.dirty);
            let p=s.plan(&[view()],8,false).unwrap();
            assert!(p.updates.is_empty(),"receiver movement must not re-render unchanged tree pages");
            assert_eq!(s.revision,revision);assert_eq!(s.world_to_light,original_projection);assert_eq!(s.depth,original_depth);
            s.submitted(&p);
        }
        let mut moved_tree=tree;moved_tree.rows[0][3]+=1.;moved_tree.sphere[0]+=1.;
        assert!(s.update_source(0,moved_tree));assert!(s.dirty,"real caster changes still invalidate");
    }
    #[test]fn foliage_pages_moving_receiver_requests_new_columns_without_stale_clear(){
        let tree=instance(Vec3::new(190.,8.,0.),8.,FOLIAGE);
        let floor=instance(Vec3::new(0.,-8.,0.),1.,0);
        let mut s=state(&[tree,floor],MAX_PAGES);s.rebuild(Vec3::Y,0.42,0.5).unwrap();
        // Static caster occupancy is complete even when the requested receiver
        // moves beyond its original indexed world column.
        assert!(!s.update_source(1,instance(Vec3::new(190.,-8.,0.),1.,0)));assert!(!s.dirty);
        let cam=ReceiverView{camera:Camera{pos:Vec3::new(190.,20.,150.),target:Vec3::new(190.,0.,0.),fov_y:1.3,near:0.1},aspect:16./9.};
        let p=s.plan(&[cam],MAX_PAGES,false).unwrap();
        let q=s.world_to_light.transform_point3(Vec3::new(190.,-8.,0.));let key=Key((q.x/64.).floor()as i32,(q.y/64.).floor()as i32);
        assert!(s.tiles[&key].clusters>0);assert!(s.table().iter().any(|e|e.key_layer[0]==key.0&&e.key_layer[1]==key.1&&e.key_layer[2]>=0));s.submitted(&p);
        // Moving to a genuinely empty column produces explicit empty metadata,
        // without clearing existing cached caster pages or altering their Z.
        let depth=s.depth;assert!(!s.update_source(1,instance(Vec3::new(220.,-8.,100.),1.,0)));assert!(!s.dirty);
        let cam=ReceiverView{camera:Camera{pos:Vec3::new(220.,20.,250.),target:Vec3::new(220.,0.,100.),fov_y:1.3,near:0.1},aspect:16./9.};
        s.plan(&[cam],MAX_PAGES,false).unwrap();let q=s.world_to_light.transform_point3(Vec3::new(220.,-8.,100.));let key=Key((q.x/64.).floor()as i32,(q.y/64.).floor()as i32);
        assert!(s.table().iter().any(|e|e.key_layer[0]==key.0&&e.key_layer[1]==key.1&&e.key_layer[2]==-1));assert_eq!(s.depth,depth);
    }
}
