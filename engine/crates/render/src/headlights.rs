//! Bounded dynamic lighting: shadowed spots and short-range unshadowed lights.
//! Only shadowed spots own a shared perspective depth layer; those layers
//! scale with lights, not split-screen cameras. No ambient fill.
use crate::Gpu;
use bytemuck::{Pod,Zeroable};
use glam::{Mat4,Vec3};
use wgpu::util::DeviceExt;
use sha2::{Digest,Sha256};

pub const MAX_LIGHTS:usize=8;
pub const MAX_LOCAL_LIGHTS:usize=16;
pub const SHADOW_SIZE:u32=1024;
pub const SHADOW_NEAR:f32=0.05;

/// Bounded direct illumination for small emissive effects. This is not GI:
/// no bounce, atmospheric scattering or occluder test is performed. A rearward
/// cone can keep a car lamp from illuminating through its own body, but cannot
/// replace shadows from arbitrary walls. Units are linear RGB candela/metres.
#[derive(Clone,Copy,Debug,PartialEq)]
pub struct LocalLight {
    pub position:Vec3,
    pub direction:Vec3,
    pub intensity_cd:Vec3,
    pub range_m:f32,
    /// Cosines of cone half-angles; outer must be smaller than inner.
    pub inner_cos:f32,
    pub outer_cos:f32,
    /// Finite source radius prevents a singular highlight at the emitter.
    pub source_radius_m:f32,
}
impl LocalLight {
    pub fn validated(self)->Result<Self,String> {
        if ![self.position,self.direction,self.intensity_cd].iter().all(|v|v.is_finite())
            || self.direction.length_squared()<1e-8
            || !self.direction.length_squared().is_finite()
            || self.intensity_cd.min_element()<0.0 || self.intensity_cd.max_element()>1_000_000.0
            || !self.range_m.is_finite() || !(0.1..=20.0).contains(&self.range_m)
            || !self.source_radius_m.is_finite() || !(0.01..=1.0).contains(&self.source_radius_m)
            || self.source_radius_m>=self.range_m
            || !self.inner_cos.is_finite() || !self.outer_cos.is_finite()
            || !(-1.0..=1.0).contains(&self.outer_cos) || !(-1.0..=1.0).contains(&self.inner_cos)
            || self.outer_cos>=self.inner_cos
        {return Err("local light requires finite pose/intensity, range0.1..20m, source radius0.01..1m smaller than range, and -1<=outer_cos<inner_cos<=1".into());}
        Ok(Self{direction:self.direction.normalize(),..self})
    }
}
pub fn validate_local(lights:&[LocalLight])->Result<Vec<LocalLight>,String> {
    if lights.len()>MAX_LOCAL_LIGHTS {return Err(format!("at most {MAX_LOCAL_LIGHTS} local spill lights are supported"));}
    lights.iter().map(|l|l.validated()).collect()
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuLocal {
    position_range:[f32;4],direction_outer:[f32;4],intensity_inner:[f32;4],source:[f32;4],
}

/// Linear RGB intensity in candela. Direction points away from the lamp.
/// A game may represent a headlamp pair with one combined beam to bound cost.
#[derive(Clone,Copy,Debug,PartialEq)]
pub struct SpotLight {
    pub position:Vec3,pub direction:Vec3,pub up:Vec3,
    pub intensity_cd:Vec3,pub range_m:f32,
    pub inner_angle_radians:f32,pub outer_angle_radians:f32,
}
impl SpotLight {
    pub fn validated(self)->Result<Self,String> {
        if ![self.position,self.direction,self.up,self.intensity_cd].iter().all(|v|v.is_finite())
            || self.direction.length_squared()<1e-8 || self.up.length_squared()<1e-8
            || self.direction.normalize().cross(self.up.normalize()).length_squared()<1e-5
            || self.intensity_cd.min_element()<0.0 || self.intensity_cd.max_element()>1_000_000.0
            || !self.range_m.is_finite() || !(1.0..=200.0).contains(&self.range_m)
            || !self.inner_angle_radians.is_finite() || !self.outer_angle_radians.is_finite()
            || self.inner_angle_radians<0.0 || self.inner_angle_radians>=self.outer_angle_radians
            || !(0.03..=1.3).contains(&self.outer_angle_radians) {
            return Err("spot light requires finite pose/intensity, independent up/direction, range1..200m, and 0<=inner<outer<=1.3rad".into());
        }
        Ok(Self {direction:self.direction.normalize(),up:self.up.normalize(),..self})
    }
    pub fn shadow_matrix(self)->Mat4 {
        Mat4::perspective_rh(self.outer_angle_radians*2.0,1.0,SHADOW_NEAR,self.range_m)
            *Mat4::look_at_rh(self.position,self.position+self.direction,self.up)
    }
    pub fn focal_pixels(self)->f32 {SHADOW_SIZE as f32/(2.0*self.outer_angle_radians.tan())}
}
pub fn validate(lights:&[SpotLight])->Result<Vec<SpotLight>,String> {
    if lights.len()>MAX_LIGHTS {return Err(format!("at most {MAX_LIGHTS} shadowed spot lights are supported"));}
    lights.iter().map(|l|l.validated()).collect()
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuSpot {
    view_proj:[[f32;4];4],position_range:[f32;4],direction_outer:[f32;4],intensity_inner:[f32;4],
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {lights:[GpuSpot;MAX_LIGHTS],count:[u32;4],local:[GpuLocal;MAX_LOCAL_LIGHTS],local_min:[f32;4],local_max:[f32;4]}
impl Uniform {
    fn from_lights(lights:&[SpotLight],local:&[LocalLight])->Self {
        let mut result=Self::zeroed();result.count[0]=lights.len()as u32;
        for (dst,src) in result.lights.iter_mut().zip(lights) {
            *dst=GpuSpot {view_proj:src.shadow_matrix().to_cols_array_2d(),
                position_range:src.position.extend(src.range_m).to_array(),
                direction_outer:src.direction.extend(src.outer_angle_radians.cos()).to_array(),
                intensity_inner:src.intensity_cd.extend(src.inner_angle_radians.cos()).to_array()};
        }
        result.count[1]=local.len()as u32;
        let mut lower=Vec3::splat(f32::INFINITY);let mut upper=Vec3::splat(f32::NEG_INFINITY);
        for(dst,src)in result.local.iter_mut().zip(local) {
            lower=lower.min(src.position-Vec3::splat(src.range_m));
            upper=upper.max(src.position+Vec3::splat(src.range_m));
            *dst=GpuLocal {position_range:src.position.extend(src.range_m).to_array(),
                direction_outer:src.direction.extend(src.outer_cos).to_array(),
                intensity_inner:src.intensity_cd.extend(src.inner_cos).to_array(),
                source:[src.source_radius_m*src.source_radius_m,0.0,0.0,0.0]};
        }
        if !local.is_empty() {result.local_min=lower.extend(0.0).to_array();result.local_max=upper.extend(0.0).to_array();}
        result
    }
}
pub(crate) struct Resources {
    pub uniform:wgpu::Buffer,pub texture:wgpu::Texture,pub sampled:wgpu::TextureView,
    // Shadow raster binds this different texture to avoid read/write aliasing
    // even on backends which validate every resource in a bound group.
    pub empty:wgpu::TextureView,
}
impl Resources {
    pub fn new(gpu:&Gpu)->Self {
        let texture=|label,size|gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some(label),
            size:wgpu::Extent3d {width:size,height:size,depth_or_array_layers:MAX_LIGHTS as u32},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
        let tex=texture("headlight shadows",SHADOW_SIZE);
        let desc=wgpu::TextureViewDescriptor {dimension:Some(wgpu::TextureViewDimension::D2Array),..Default::default()};
        let sampled=tex.create_view(&desc);let empty=texture("unused headlight shadows",1).create_view(&desc);
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("headlights"),
            contents:bytemuck::bytes_of(&Uniform::zeroed()),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
        Self {uniform,texture:tex,sampled,empty}
    }
    pub fn upload(&self,gpu:&Gpu,lights:&[SpotLight],local:&[LocalLight]) {gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&Uniform::from_lights(lights,local)));}
    pub fn layer(&self,index:u32)->wgpu::TextureView {self.texture.create_view(&wgpu::TextureViewDescriptor {
        dimension:Some(wgpu::TextureViewDimension::D2),base_array_layer:index,array_layer_count:Some(1),..Default::default()})}

    /// Capture-only exact depth evidence, not an ordinary frame readback.
    pub fn depth_hashes(&self,gpu:&Gpu,count:usize)->Result<Vec<String>,String> {
        if count>MAX_LIGHTS{return Err("invalid active headlight layer count".into());}
        if count==0{return Ok(Vec::new());}
        let row=SHADOW_SIZE*4;let layer=u64::from(row)*u64::from(SHADOW_SIZE);
        let buffer=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("headlight depth capture"),
            size:layer*count as u64,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&self.texture,mip_level:0,
            origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::DepthOnly},
            wgpu::TexelCopyBufferInfo {buffer:&buffer,layout:wgpu::TexelCopyBufferLayout {
                offset:0,bytes_per_row:Some(row),rows_per_image:Some(SHADOW_SIZE)}},
            wgpu::Extent3d {width:SHADOW_SIZE,height:SHADOW_SIZE,depth_or_array_layers:count as u32});
        gpu.queue.submit([encoder.finish()]);
        let(tx,rx)=std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read,move|result|{let _=tx.send(result);});gpu.wait();
        rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        let hashes={let bytes=buffer.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
            bytes.chunks_exact(layer as usize).map(|data|format!("{:x}",Sha256::digest(data))).collect()};
        buffer.unmap();Ok(hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lamp()->SpotLight {SpotLight {position:Vec3::new(2.0,1.0,3.0),direction:Vec3::NEG_Z,up:Vec3::Y,
        intensity_cd:Vec3::splat(60_000.0),range_m:90.0,inner_angle_radians:0.25,outer_angle_radians:0.55}}
    #[test] fn bounded_validated_lights_and_gpu_layout() {
        assert_eq!(validate(&[lamp();8]).unwrap().len(),8);assert!(validate(&[lamp();9]).is_err());
        for bad in [SpotLight {range_m:f32::NAN,..lamp()},SpotLight {up:Vec3::NEG_Z,..lamp()},
            SpotLight {inner_angle_radians:0.7,..lamp()},SpotLight {intensity_cd:Vec3::splat(-1.0),..lamp()}] {assert!(bad.validated().is_err());}
        assert_eq!(std::mem::size_of::<GpuSpot>(),112);assert_eq!(std::mem::size_of::<GpuLocal>(),64);
        assert_eq!(std::mem::size_of::<Uniform>(),1968);
        assert_eq!(Uniform::from_lights(&[],&[]).count,[0;4]);
    }
    #[test] fn local_spill_has_an_independent_budget_and_zeroes_expired_lights() {
        let light=LocalLight{position:Vec3::ZERO,direction:Vec3::Z*3.0,intensity_cd:Vec3::new(80.0,4.0,1.0),
            range_m:3.0,inner_cos:0.8,outer_cos:0.3,source_radius_m:0.06};
        let local=validate_local(&[light;MAX_LOCAL_LIGHTS]).unwrap();
        assert_eq!(local[0].direction,Vec3::Z);
        assert!(validate_local(&[light;MAX_LOCAL_LIGHTS+1]).is_err());
        for bad in [LocalLight{range_m:f32::NAN,..light},LocalLight{intensity_cd:Vec3::splat(-1.0),..light},
            LocalLight{source_radius_m:4.0,..light},LocalLight{inner_cos:0.1,..light},LocalLight{direction:Vec3::ZERO,..light},
            LocalLight{direction:Vec3::splat(f32::MAX),..light}] {
            assert!(bad.validated().is_err());
        }
        let before=Uniform::from_lights(&[lamp();MAX_LIGHTS],&local);
        assert_eq!(before.count,[8,16,0,0]);
        assert_eq!(before.local[0].source[0],0.06*0.06);
        assert_eq!(before.local_min,[-3.0,-3.0,-3.0,0.0]);assert_eq!(before.local_max,[3.0,3.0,3.0,0.0]);
        let after=Uniform::from_lights(&[lamp();MAX_LIGHTS],&[]);
        assert_eq!(after.count,[8,0,0,0]);
        assert!(bytemuck::bytes_of(&after.local).iter().all(|v|*v==0));
        assert_eq!(bytemuck::bytes_of(&before.lights),bytemuck::bytes_of(&after.lights));
    }
    #[test] fn perspective_depth_and_rotated_beam_are_consistent() {
        let l=lamp().validated().unwrap();let vp=l.shadow_matrix();
        let n=vp.project_point3(l.position+l.direction*SHADOW_NEAR);
        let f=vp.project_point3(l.position+l.direction*l.range_m);
        assert!(n.z.abs()<1e-5);assert!((f.z-1.0).abs()<1e-5);
        let p=l.position+l.direction*10.0+Vec3::X*(10.0*l.outer_angle_radians.tan());
        assert!((vp.project_point3(p).x-1.0).abs()<1e-5);
        let q=glam::Quat::from_euler(glam::EulerRot::YXZ,0.9,0.2,0.3);
        let rotated=SpotLight {position:q*l.position,direction:q*l.direction,up:q*l.up,..l};
        assert!(vp.project_point3(p).abs_diff_eq(rotated.shadow_matrix().project_point3(q*p),1e-5));
    }
}
