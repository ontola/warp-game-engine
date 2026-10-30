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
pub struct FrameOutput { pub color: wgpu::Texture, pub hud: render::hud::HudDrawList }
pub trait Game {
    /// Startup render budget per eye; the runtime recommendation remains the ceiling.
    fn eye_limit(&self) -> u32 { 1024 }
    /// Optional explicit developer diagnostics before starting a session.
    fn diagnose(&mut self, _gpu: &render::Gpu) -> Result<(), String> { Ok(()) }
    fn quit(&self) -> bool { false }
    /// Draw both eyes side by side into an RGBA8 texture, with no readback.
    fn frame(&mut self, gpu: &render::Gpu, tracking: &Tracking, eye_size: [u32; 2], dt: f32) -> Result<FrameOutput, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
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
