//! OpenXR feeds predicted poses into Warp's existing offscreen renderer.
//! Game coordinates and locomotion remain game-owned; the engine owns sessions,
//! Vulkan interop, Touch bindings and compositor submission.
use glam::{Mat4, Quat, Vec3, Vec4};

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::run;

#[derive(Clone, Copy, Debug)]
pub struct Pose { pub position: Vec3, pub orientation: Quat }
impl Default for Pose { fn default() -> Self { Self { position: Vec3::ZERO, orientation: Quat::IDENTITY } } }
impl Pose {
    pub fn transformed(self, origin: Vec3, yaw: f32) -> Self {
        let rotation = Quat::from_rotation_y(yaw);
        Self { position: origin + rotation * self.position, orientation: rotation * self.orientation }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Eye { pub pose: Pose, pub fov: [f32; 4] }
impl Eye {
    pub fn camera(self) -> render::Camera {
        render::Camera { pos: self.pose.position, target: self.pose.position + self.pose.orientation * Vec3::NEG_Z,
            fov_y: self.fov[2] - self.fov[3], near: 0.05 }
    }
    pub fn projection(self) -> Mat4 { projection(self.fov, 0.05) }
}

/// OpenXR angles: left, right, up, down. Vulkan/wgpu clip depth is reversed
/// 0..1 (near=1, infinity=0), while the renderer handles its normal Y convention.
pub fn projection(fov: [f32; 4], near: f32) -> Mat4 {
    let [l, r, u, d] = fov.map(f32::tan);
    Mat4::from_cols(Vec4::new(2.0 / (r-l), 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / (u-d), 0.0, 0.0),
        Vec4::new((r+l)/(r-l), (u+d)/(u-d), 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0))
}

#[derive(Clone, Debug)]
pub struct Tracking {
    pub eyes: [Eye; 2], pub head: Pose, pub aim: Option<Pose>,
    pub controls: app::Controls, pub turn: f32, pub focused: bool,
    /// Runtime recenter: reset the game-owned tracking origin on this frame.
    pub recentered: bool,
}
pub struct FrameOutput { pub color: Option<wgpu::Texture>, pub hud: render::hud::HudDrawList }
pub trait Game {
    /// Preferred display frequency; actual support is queried from the runtime.
    fn refresh_rate(&self) -> Option<f32> { None }
    /// Optional explicit developer diagnostics before starting a session.
    fn diagnose(&mut self, _gpu: &render::Gpu) -> Result<(), String> { Ok(()) }
    fn quit(&self) -> bool { false }
    /// Draw directly into supplied per-eye attachments, or return side-by-side RGBA8 for diagnostics.
    fn frame(&mut self, gpu: &render::Gpu, tracking: &Tracking, eye_size: [u32; 2], dt: f32, targets: Option<&[&wgpu::Texture]>) -> Result<FrameOutput, String>;
}

/// Collapse full-field display tints into a premultiplied linear colour.
/// A one-pixel projection layer covers both complete eye frusta, including
/// the HUD, without a screen-sized render target or another scene copy.
pub fn vision_color(tints:&[[f32;4]])->wgpu::Color {
    let mut out=[0.0f64;4];
    for tint in tints {
        let a=(tint[3] as f64).clamp(0.0,1.0);
        for i in 0..3 {let c=(tint[i] as f64).clamp(0.0,1.0);let linear=if c<=0.04045{c/12.92}else{((c+0.055)/1.055).powf(2.4)};
            out[i]=linear*a+out[i]*(1.0-a);}
        out[3]=a+out[3]*(1.0-a);
    }
    wgpu::Color{r:out[0],g:out[1],b:out[2],a:out[3]}
}

pub fn draw_vision(gpu:&render::Gpu,tints:&[[f32;4]],target:&wgpu::Texture) {
    let view=target.create_view(&Default::default());
    let mut enc=gpu.device.create_command_encoder(&Default::default());
    {let _pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("full-vision tint"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{
        view:&view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(vision_color(tints)),store:wgpu::StoreOp::Store}})],..Default::default()});}
    gpu.queue.submit([enc.finish()]);
}

/// Shared HUD submission for native presentation and headless GPU measurements.
pub fn draw_hud(gpu:&render::Gpu,hud:&mut render::hud::HudRenderer,list:&render::hud::HudDrawList,target:&wgpu::Texture)->Result<(),String> {
    draw_hud_timed(gpu,hud,list,target).map(|_|())
}
/// CPU wall phases: text/geometry preparation, encoding, queue submission.
pub fn draw_hud_timed(gpu:&render::Gpu,hud:&mut render::hud::HudRenderer,list:&render::hud::HudDrawList,target:&wgpu::Texture)->Result<([f64;3],render::hud::HudStats),String> {
    let start=std::time::Instant::now();
    let stats=hud.prepare(gpu,list)?;let prepared=std::time::Instant::now();
    let mut enc=gpu.device.create_command_encoder(&Default::default());
    let view=target.create_view(&Default::default());
    {let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("XR HUD"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{
        view:&view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})],..Default::default()});hud.render(&mut pass)?;}
    let commands=enc.finish();let encoded=std::time::Instant::now();
    gpu.queue.submit([commands]);Ok(([(prepared-start).as_secs_f64()*1000.,(encoded-prepared).as_secs_f64()*1000.,encoded.elapsed().as_secs_f64()*1000.],stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vision_tints_compose_in_linear_premultiplied_order() {
        assert_eq!(vision_color(&[]),wgpu::Color::TRANSPARENT);
        let tint=vision_color(&[[1.,0.,0.,0.5],[0.,0.,1.,0.5]]);
        assert_eq!(tint,wgpu::Color{r:0.25,g:0.,b:0.5,a:0.75});
        assert_eq!(vision_color(&[[0.2,0.4,0.8,0.5],[1.,1.,1.,1.]]),wgpu::Color::WHITE);
    }
    #[test]
    fn asymmetric_frustum_maps_all_edges_and_reverse_depth() {
        let fov = [-0.8, 0.65, 0.9, -0.7];
        let p = projection(fov, 0.05);
        for (angle, axis, edge) in [(fov[0],0,-1.0),(fov[1],0,1.0),(fov[2],1,1.0),(fov[3],1,-1.0)] {
            let mut v = Vec4::new(0.0,0.0,-2.0,1.0); v[axis] = angle.tan()*2.0;
            let clip=p*v; assert!((clip[axis]/clip.w-edge).abs()<1e-5);
        }
        let n=p*Vec4::new(0.0,0.0,-0.05,1.0); assert!((n.z/n.w-1.0).abs()<1e-6);
        let far=p*Vec4::new(0.0,0.0,-10000.0,1.0); assert!(far.z/far.w<0.00001);
    }
    #[test]
    fn rigid_rebase_preserves_ipd_and_head_roll() {
        let q=Quat::from_rotation_z(0.3);
        let a=Pose {position:q*Vec3::new(-0.032,1.6,0.0),orientation:q};
        let b=Pose {position:q*Vec3::new(0.032,1.6,0.0),orientation:q};
        let a=a.transformed(Vec3::new(5.0,2.0,3.0),1.1); let b=b.transformed(Vec3::new(5.0,2.0,3.0),1.1);
        assert!((a.position.distance(b.position)-0.064).abs()<1e-5);
        assert!((a.orientation*Vec3::Y-Quat::from_rotation_y(1.1)*q*Vec3::Y).length()<1e-5);
    }
}

#[cfg(target_os = "android")]
mod vulkan;
#[cfg(target_os = "android")]
mod actions;
