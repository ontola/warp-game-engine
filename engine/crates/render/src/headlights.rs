//! Bounded dynamic spot lights. Each light owns one shared perspective shadow
//! layer; its cost scales with lights, not split-screen cameras. No ambient fill.
use crate::Gpu;
use bytemuck::{Pod,Zeroable};
use glam::{Mat4,Vec3};
use wgpu::util::DeviceExt;

pub const MAX_LIGHTS:usize=4;
pub const SHADOW_SIZE:u32=1024;
pub const SHADOW_NEAR:f32=0.05;

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
struct Uniform {lights:[GpuSpot;MAX_LIGHTS],count:[u32;4]}
impl Uniform {
    fn from_lights(lights:&[SpotLight])->Self {
        let mut result=Self::zeroed();result.count[0]=lights.len()as u32;
        for (dst,src) in result.lights.iter_mut().zip(lights) {
            *dst=GpuSpot {view_proj:src.shadow_matrix().to_cols_array_2d(),
                position_range:src.position.extend(src.range_m).to_array(),
                direction_outer:src.direction.extend(src.outer_angle_radians.cos()).to_array(),
                intensity_inner:src.intensity_cd.extend(src.inner_angle_radians.cos()).to_array()};
        }
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
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let tex=texture("headlight shadows",SHADOW_SIZE);
        let desc=wgpu::TextureViewDescriptor {dimension:Some(wgpu::TextureViewDimension::D2Array),..Default::default()};
        let sampled=tex.create_view(&desc);let empty=texture("unused headlight shadows",1).create_view(&desc);
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("headlights"),
            contents:bytemuck::bytes_of(&Uniform::zeroed()),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
        Self {uniform,texture:tex,sampled,empty}
    }
    pub fn upload(&self,gpu:&Gpu,lights:&[SpotLight]) {gpu.queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&Uniform::from_lights(lights)));}
    pub fn layer(&self,index:u32)->wgpu::TextureView {self.texture.create_view(&wgpu::TextureViewDescriptor {
        dimension:Some(wgpu::TextureViewDimension::D2),base_array_layer:index,array_layer_count:Some(1),..Default::default()})}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lamp()->SpotLight {SpotLight {position:Vec3::new(2.0,1.0,3.0),direction:Vec3::NEG_Z,up:Vec3::Y,
        intensity_cd:Vec3::splat(60_000.0),range_m:90.0,inner_angle_radians:0.25,outer_angle_radians:0.55}}
    #[test] fn bounded_validated_lights_and_gpu_layout() {
        assert_eq!(validate(&[lamp();4]).unwrap().len(),4);assert!(validate(&[lamp();5]).is_err());
        for bad in [SpotLight {range_m:f32::NAN,..lamp()},SpotLight {up:Vec3::NEG_Z,..lamp()},
            SpotLight {inner_angle_radians:0.7,..lamp()},SpotLight {intensity_cd:Vec3::splat(-1.0),..lamp()}] {assert!(bad.validated().is_err());}
        assert_eq!(std::mem::size_of::<GpuSpot>(),112);assert_eq!(std::mem::size_of::<Uniform>(),464);
        assert_eq!(Uniform::from_lights(&[]).count,[0;4]);
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
