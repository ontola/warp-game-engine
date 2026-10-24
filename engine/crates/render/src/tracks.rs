//! Persistent, lit tyre marks in a bounded sparse world atlas. Stamping is
//! CPU-only; dirty rectangles upload once for all views, without GPU readback.
//! Marks are material data, never floating geometry or post-light black paint.
use crate::Gpu;
use glam::{Vec2,Vec3};
use std::collections::{BTreeSet,HashMap};

pub const TEXEL_M:f32=0.08;
pub const TILE_PIXELS:u32=128;
pub const TILE_M:f32=TEXEL_M*TILE_PIXELS as f32;
pub const MAX_TILES:usize=512;
pub const ATLAS_WIDTH:u32=4096;
pub const ATLAS_HEIGHT:u32=2048;
const LOOKUP_SIZE:u32=256;
const LOOKUP_PROBES:usize=8;
pub const MAX_STAMPS_PER_UPDATE:usize=8192;
const HEIGHT_RANGE:f32=64.;
pub const HEIGHT_TOLERANCE:f32=0.16;
pub const GPU_BYTES:usize=(ATLAS_WIDTH*ATLAS_HEIGHT*4+LOOKUP_SIZE*LOOKUP_SIZE*16)as usize;

#[derive(Clone,Copy,Debug,PartialEq)]
pub struct Stamp {
    pub start:[f32;3],pub end:[f32;3],pub start_normal:[f32;3],pub end_normal:[f32;3],
    pub width:f32,pub compaction:f32,pub rubber:f32,
}
impl Stamp {
    fn valid(self)->bool {
        [self.start,self.end,self.start_normal,self.end_normal].into_iter().flatten().all(f32::is_finite)
            &&[self.width,self.compaction,self.rubber].into_iter().all(f32::is_finite)
            &&(0.1..=0.8).contains(&self.width)&&(0.0..=1.0).contains(&self.compaction)&&(0.0..=1.0).contains(&self.rubber)
            &&self.start_normal[1]>0.3&&self.end_normal[1]>0.3
            &&Vec3::from(self.start).abs().max_element()<1e6&&Vec3::from(self.end).abs().max_element()<1e6
            &&Vec3::from(self.start).distance_squared(Vec3::from(self.end))<=16.
    }
}
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub struct Stats {
    pub stamps:u64,pub rejected:u64,pub tiles_resident:usize,pub tiles_evicted:u64,
    pub texels_written:u64,pub last_upload_bytes:usize,pub gpu_bytes:usize,pub cpu_tile_bytes:usize,
}
#[derive(Clone,Copy)]
struct Rect {x0:u32,y0:u32,x1:u32,y1:u32}
impl Rect {
    fn full()->Self {Self{x0:0,y0:0,x1:TILE_PIXELS,y1:TILE_PIXELS}}
    fn include(&mut self,x:u32,y:u32) {self.x0=self.x0.min(x);self.y0=self.y0.min(y);self.x1=self.x1.max(x+1);self.y1=self.y1.max(y+1);}
}
struct Tile {key:(i32,i32),base_y:f32,pixels:Vec<[u8;4]>,lookup:usize,last_use:u64,dirty:Option<Rect>}
pub struct Atlas {
    tiles:Vec<Option<Tile>>,by_key:HashMap<(i32,i32),usize>,lookup:Vec<[i32;4]>,
    dirty_lookup:BTreeSet<usize>,serial:u64,stats:Stats,
}
impl Default for Atlas {fn default()->Self {Self::with_capacity(MAX_TILES)}}
fn hash(key:(i32,i32))->usize {
    let mut h=(key.0 as u32).wrapping_mul(0x9e3779b9)^(key.1 as u32).wrapping_mul(0x85ebca6b);
    h^=h>>16;h=h.wrapping_mul(0x7feb352d);h^=h>>15;
    (h&0xffff)as usize
}
fn height_encode(y:f32,base:f32)->Option<u16> {
    let f=(y-base+HEIGHT_RANGE)/(2.*HEIGHT_RANGE);
    (0.0..=1.0).contains(&f).then(||(f*65535.).round()as u16)
}
fn height_decode(pixel:[u8;4],base:f32)->f32 {base+(u16::from_le_bytes([pixel[2],pixel[3]])as f32/65535.*2.-1.)*HEIGHT_RANGE}
impl Atlas {
    pub fn with_capacity(capacity:usize)->Self {
        let capacity=capacity.clamp(1,MAX_TILES);
        Self {tiles:(0..capacity).map(|_|None).collect(),by_key:HashMap::new(),lookup:vec![[0;4];(LOOKUP_SIZE*LOOKUP_SIZE)as usize],
            dirty_lookup:BTreeSet::new(),serial:0,stats:Stats{gpu_bytes:GPU_BYTES,..Default::default()}}
    }
    pub fn stats(&self)->Stats {self.stats}
    fn tile(&mut self,key:(i32,i32),height:f32)->Option<usize> {
        self.serial=self.serial.wrapping_add(1);
        if let Some(&slot)=self.by_key.get(&key) {self.tiles[slot].as_mut().unwrap().last_use=self.serial;return Some(slot);}
        let lookup=(0..LOOKUP_PROBES).map(|i|(hash(key)+i)&0xffff).find(|i|self.lookup[*i][2]<=0)?;
        let slot=self.tiles.iter().position(Option::is_none).unwrap_or_else(||
            self.tiles.iter().enumerate().min_by_key(|(_,t)|t.as_ref().unwrap().last_use).unwrap().0);
        if let Some(old)=self.tiles[slot].take() {
            self.by_key.remove(&old.key);self.lookup[old.lookup]=[0,0,-1,0];self.dirty_lookup.insert(old.lookup);self.stats.tiles_evicted+=1;
        }
        let base_y=(height/16.).floor()*16.;
        self.tiles[slot]=Some(Tile{key,base_y,pixels:vec![[0;4];(TILE_PIXELS*TILE_PIXELS)as usize],lookup,last_use:self.serial,dirty:Some(Rect::full())});
        self.by_key.insert(key,slot);self.lookup[lookup]=[key.0,key.1,slot as i32+1,base_y.to_bits()as i32];self.dirty_lookup.insert(lookup);
        self.stats.tiles_resident=self.by_key.len();self.stats.cpu_tile_bytes=self.by_key.len()*(TILE_PIXELS*TILE_PIXELS*4)as usize;
        Some(slot)
    }
    pub fn stamp(&mut self,stamps:&[Stamp])->Stats {
        self.stats.last_upload_bytes=0;
        self.stats.rejected+=stamps.len().saturating_sub(MAX_STAMPS_PER_UPDATE)as u64;
        for &stamp in stamps.iter().take(MAX_STAMPS_PER_UPDATE) {
            if !stamp.valid() {self.stats.rejected+=1;continue;}
            if stamp.compaction==0.&&stamp.rubber==0. {continue;}
            self.stats.stamps+=1;
            let a=Vec3::from(stamp.start);let b=Vec3::from(stamp.end);
            let a2=Vec2::new(a.x,a.z);let b2=Vec2::new(b.x,b.z);let delta=b2-a2;
            let radius=stamp.width*0.5;let pad=radius+TEXEL_M*0.5;
            let lo=((a2.min(b2)-Vec2::splat(pad))/TEXEL_M).floor().as_ivec2();
            let hi=((a2.max(b2)+Vec2::splat(pad))/TEXEL_M).floor().as_ivec2();
            // Segment length/width validation bounds every stamp to <6400
            // candidate texels. Capsule coverage is spatial, never per-frame.
            for tz in lo.y.div_euclid(TILE_PIXELS as i32)..=hi.y.div_euclid(TILE_PIXELS as i32) {
                for tx in lo.x.div_euclid(TILE_PIXELS as i32)..=hi.x.div_euclid(TILE_PIXELS as i32) {
                    let Some(slot)=self.tile((tx,tz),(a.y+b.y)*0.5) else {self.stats.rejected+=1;continue};
                    let tile=self.tiles[slot].as_mut().unwrap();
                    let ox=tx*TILE_PIXELS as i32;let oz=tz*TILE_PIXELS as i32;
                    for z in lo.y.max(oz)..=hi.y.min(oz+TILE_PIXELS as i32-1) {for x in lo.x.max(ox)..=hi.x.min(ox+TILE_PIXELS as i32-1) {
                        let q=(Vec2::new(x as f32,z as f32)+Vec2::splat(0.5))*TEXEL_M;
                        let t=((q-a2).dot(delta)/delta.length_squared().max(1e-10)).clamp(0.,1.);
                        let centre=a.lerp(b,t);let closest=a2.lerp(b2,t);
                        let coverage=((radius+TEXEL_M*0.5-q.distance(closest))/TEXEL_M).clamp(0.,1.);
                        if coverage<=0. {continue;}
                        let n=Vec3::from(stamp.start_normal).lerp(Vec3::from(stamp.end_normal),t);
                        let height=centre.y-(n.x*(q.x-centre.x)+n.z*(q.y-centre.z))/n.y.max(0.3);
                        let Some(encoded)=height_encode(height,tile.base_y) else {continue};
                        let index=((z-oz)as u32*TILE_PIXELS+(x-ox)as u32)as usize;
                        let old=tile.pixels[index];let old_height=height_decode(old,tile.base_y);
                        // One height layer per texel. A later crossing at a
                        // different bridge height replaces that texel rather
                        // than projecting either mark onto both surfaces.
                        let old=if old[0].max(old[1])>0&&(old_height-height).abs()<=HEIGHT_TOLERANCE {old}else{[0;4]};
                        let bytes=encoded.to_le_bytes();
                        let pixel=[old[0].max((stamp.compaction*coverage*255.).round()as u8),old[1].max((stamp.rubber*coverage*255.).round()as u8),bytes[0],bytes[1]];
                        if tile.pixels[index]!=pixel {
                            tile.pixels[index]=pixel;self.stats.texels_written+=1;
                            let (x,z)=((x-ox)as u32,(z-oz)as u32);
                            if let Some(r)=tile.dirty.as_mut(){r.include(x,z)}else{tile.dirty=Some(Rect{x0:x,y0:z,x1:x+1,y1:z+1});}
                        }
                    }}
                }
            }
        }
        self.stats
    }
    /// CPU oracle for identity/height tests; shader manually filters four
    /// spatial neighbours with independent height rejection.
    pub fn sample(&self,p:[f32;3])->[f32;2] {
        let q=Vec2::new(p[0],p[2])/TEXEL_M-Vec2::splat(0.5);let base=q.floor().as_ivec2();let f=q-q.floor();
        let mut result=[0.;2];
        for y in 0..2 {for x in 0..2 {
            let pixel=base+glam::IVec2::new(x,y);let key=(pixel.x.div_euclid(TILE_PIXELS as i32),pixel.y.div_euclid(TILE_PIXELS as i32));
            let Some(&slot)=self.by_key.get(&key) else {continue};let tile=self.tiles[slot].as_ref().unwrap();
            let texel=tile.pixels[(pixel.y.rem_euclid(TILE_PIXELS as i32)as u32*TILE_PIXELS+pixel.x.rem_euclid(TILE_PIXELS as i32)as u32)as usize];
            let reject=(1.-((p[1]-height_decode(texel,tile.base_y)).abs()-0.08)/0.08).clamp(0.,1.);
            let weight=if x==0 {1.-f.x}else{f.x}*if y==0 {1.-f.y}else{f.y}*reject;
            result[0]+=texel[0]as f32/255.*weight;result[1]+=texel[1]as f32/255.*weight;
        }}result
    }
}

pub(crate) struct Resources {
    atlas:wgpu::Texture,lookup:wgpu::Texture,pub atlas_view:wgpu::TextureView,pub lookup_view:wgpu::TextureView,
    state:std::sync::Mutex<Atlas>,
}
impl Resources {
    pub fn new(gpu:&Gpu)->Self {
        let texture=|label,w,h,format|gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some(label),size:wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
        let atlas=texture("persistent tyre material atlas",ATLAS_WIDTH,ATLAS_HEIGHT,wgpu::TextureFormat::Rgba8Unorm);
        let lookup=texture("tyre tile identities",LOOKUP_SIZE,LOOKUP_SIZE,wgpu::TextureFormat::Rgba32Sint);
        let atlas_view=atlas.create_view(&Default::default());let lookup_view=lookup.create_view(&Default::default());
        Self{atlas,lookup,atlas_view,lookup_view,state:std::sync::Mutex::new(Atlas::default())}
    }
    pub fn stats(&self)->Stats {self.state.lock().unwrap().stats()}
    pub fn clear(&self,gpu:&Gpu) {
        let mut state=self.state.lock().unwrap();*state=Atlas::default();
        gpu.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&self.lookup,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
            bytemuck::cast_slice(&state.lookup),wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(LOOKUP_SIZE*16),rows_per_image:Some(LOOKUP_SIZE)},
            wgpu::Extent3d{width:LOOKUP_SIZE,height:LOOKUP_SIZE,depth_or_array_layers:1});
        state.stats.last_upload_bytes=state.lookup.len()*16;
    }
    pub fn update(&self,gpu:&Gpu,stamps:&[Stamp])->Stats {
        let mut state=self.state.lock().unwrap();state.stamp(stamps);let mut bytes=0;
        // Atlas payload is committed before identities; rendering is submitted
        // afterwards. Reused pages never expose the previous world's marks.
        for (slot,tile) in state.tiles.iter_mut().enumerate() {let Some(tile)=tile else {continue};let Some(rect)=tile.dirty.take() else {continue};
            let width=rect.x1-rect.x0;let height=rect.y1-rect.y0;
            let data:Vec<[u8;4]>=(rect.y0..rect.y1).flat_map(|y|tile.pixels[(y*TILE_PIXELS+rect.x0)as usize..(y*TILE_PIXELS+rect.x1)as usize].iter().copied()).collect();
            gpu.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&self.atlas,mip_level:0,origin:wgpu::Origin3d{x:slot as u32%32*TILE_PIXELS+rect.x0,y:slot as u32/32*TILE_PIXELS+rect.y0,z:0},aspect:wgpu::TextureAspect::All},
                bytemuck::cast_slice(&data),wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(width*4),rows_per_image:Some(height)},wgpu::Extent3d{width,height,depth_or_array_layers:1});bytes+=data.len()*4;
        }
        let dirty=std::mem::take(&mut state.dirty_lookup);
        for index in dirty {gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&self.lookup,mip_level:0,origin:wgpu::Origin3d{x:index as u32%LOOKUP_SIZE,y:index as u32/LOOKUP_SIZE,z:0},aspect:wgpu::TextureAspect::All},
            bytemuck::bytes_of(&state.lookup[index]),wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(16),rows_per_image:Some(1)},wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1});bytes+=16;}
        state.stats.last_upload_bytes=bytes;state.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stroke(x:f32,z:f32)->Stamp {Stamp{start:[x,1.,z],end:[x+1.,1.,z],start_normal:[0.,1.,0.],end_normal:[0.,1.,0.],width:0.24,compaction:0.7,rubber:0.}}
    #[test] fn continuous_cross_tile_negative_world_and_height_rejection() {
        let mut atlas=Atlas::default();atlas.stamp(&[stroke(-0.5,-0.04)]);
        for i in 0..20 {let x=-0.45+i as f32*0.045;assert!(atlas.sample([x,1.,-0.04])[0]>0.6,"missing mark at{x}");assert_eq!(atlas.sample([x,4.,-0.04]),[0.;2]);}
        assert_eq!(atlas.sample([0.,1.,1.]),[0.;2]);
        let s=atlas.stats();assert_eq!(s.stamps,1);assert!(s.tiles_resident<=4);assert_eq!(s.gpu_bytes,33*1024*1024);
    }
    #[test] fn least_recently_stamped_eviction_removes_identity_and_stale_pixels() {
        let mut atlas=Atlas::with_capacity(2);atlas.stamp(&[stroke(1.,1.),stroke(31.,1.)]);
        atlas.stamp(&[stroke(1.,1.)]);atlas.stamp(&[stroke(65.,1.)]);
        assert!(atlas.sample([1.5,1.,1.])[0]>0.5);assert_eq!(atlas.sample([31.5,1.,1.]),[0.;2]);assert!(atlas.sample([65.5,1.,1.])[0]>0.5);
        assert_eq!(atlas.stats().tiles_resident,2);assert_eq!(atlas.stats().tiles_evicted,1);
        for tile in atlas.tiles.iter().flatten(){let entry=atlas.lookup[tile.lookup];assert_eq!((entry[0],entry[1]),tile.key);}
    }
    #[test] fn stamp_frequency_does_not_turn_material_black_and_layers_do_not_leak() {
        let mut atlas=Atlas::default();let s=stroke(1.,1.);atlas.stamp(&[s]);let once=atlas.sample([1.5,1.,1.]);
        atlas.stamp(&[s;120]);assert_eq!(atlas.sample([1.5,1.,1.]),once);
        let upper=Stamp{start:[1.,8.,1.],end:[2.,8.,1.],rubber:0.9,compaction:0.,..s};atlas.stamp(&[upper]);
        assert_eq!(atlas.sample([1.5,1.,1.]),[0.;2]);assert!(atlas.sample([1.5,8.,1.])[1]>0.8);
        let normal=Vec3::new(-0.3,1.,0.).normalize().to_array();let slope=Stamp{start:[21.,1.,1.],end:[22.,1.3,1.],start_normal:normal,end_normal:normal,..s};atlas.stamp(&[slope]);
        assert!(atlas.sample([21.5,1.15,1.])[0]>0.6);
    }
    #[test] fn invalid_segments_are_bounded_and_do_not_allocate() {
        let mut atlas=Atlas::with_capacity(4);let s=stroke(1.,1.);
        for bad in [Stamp{end:[900.,0.,0.],..s},Stamp{width:f32::NAN,..s},Stamp{start_normal:[0.,0.,1.],..s}] {atlas.stamp(&[bad]);}
        assert_eq!(atlas.stats().tiles_resident,0);assert_eq!(atlas.stats().rejected,3);
        for i in 0..20 {atlas.stamp(&[stroke(i as f32*30.+1.,1.)]);}assert_eq!(atlas.stats().tiles_resident,4);
        assert!(atlas.stats().cpu_tile_bytes<=4*128*128*4);
    }
    #[test] fn hash_collision_survives_deleted_predecessor_without_stale_identity() {
        let mut seen=HashMap::new();let mut collision=None;
        for x in 0..5000 {let key=(x,0);if let Some(previous)=seen.insert(hash(key),key) {collision=Some((previous,key));break;}}
        let (a,b)=collision.expect("fixture must find a real hash collision");let mut atlas=Atlas::with_capacity(2);
        atlas.stamp(&[stroke(a.0 as f32*TILE_M+1.,1.),stroke(b.0 as f32*TILE_M+1.,1.)]);
        atlas.stamp(&[stroke(b.0 as f32*TILE_M+1.,1.),stroke(-300.,1.)]);
        assert_eq!(atlas.lookup[hash(a)][2],-1,"eviction must retain a probe-chain tombstone");
        let found=(0..LOOKUP_PROBES).map(|i|atlas.lookup[(hash(b)+i)&0xffff]).take_while(|v|v[2]!=0).find(|v|v[2]>0&&(v[0],v[1])==b);
        assert!(found.is_some(),"shader must still reach the second colliding world identity");
        assert!(!atlas.by_key.contains_key(&a));assert!(atlas.by_key.contains_key(&b));
    }
}
