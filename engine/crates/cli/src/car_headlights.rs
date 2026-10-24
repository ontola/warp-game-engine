//! First bounded car-only night lights: one combined pair per human car.
//! Visual light intensity is a tuning value, not a homologated beam model.
use glam::Vec3;
use render::{Lighting,headlights::SpotLight};
use crate::visuals::CarPose;

pub fn from_poses(poses:&[CarPose],human_indices:&[usize],light:&Lighting)->Vec<SpotLight> {
    // Activate at twilight/night without raising ambient exposure or adding
    // scene lamps. Daytime has no headlight shadow-pass cost.
    if light.sun_dir.y>5.0f32.to_radians().sin() && light.sun_lux>=5000.0 {return Vec::new();}
    human_indices.iter().take(render::headlights::MAX_LIGHTS).filter_map(|&i|poses.get(i)).map(|p| {
        let front=p.boxes.iter().map(|b|(b[5]-b[2]*0.5)as f32).fold(-1.8,f32::min);
        let height=p.boxes.first().map(|b|b[4]as f32).unwrap_or(0.35).clamp(0.2,0.8)+p.ride;
        // The whole emitter starts beyond the body's frontmost bound, so its
        // own grille/bonnet cannot bury the spotlight. Follow full body pose.
        SpotLight {position:p.pos+p.rot*Vec3::new(0.0,height,front-0.12),
            direction:p.rot*Vec3::new(0.0,-0.045,-1.0).normalize(),up:p.rot*Vec3::Y,
            intensity_cd:Vec3::new(80_000.0,76_000.0,68_000.0),range_m:90.0,
            inner_angle_radians:17.0f32.to_radians(),outer_angle_radians:32.0f32.to_radians()}
    }).collect()
}
pub fn metadata(lights:&[SpotLight])->serde_json::Value {serde_json::json!({
    "mode":"one shadowed combined headlamp pair per human car","enabled":!lights.is_empty(),
    "lights":lights.iter().map(|l|serde_json::json!({"position":l.position.to_array(),"direction":l.direction.to_array(),
        "up":l.up.to_array(),"intensity_cd":l.intensity_cd.to_array(),"range_m":l.range_m,
        "inner_angle_radians":l.inner_angle_radians,"outer_angle_radians":l.outer_angle_radians})).collect::<Vec<_>>(),
    "shadow_size":render::headlights::SHADOW_SIZE,
    "limits":["combined pair, not two individual lamps","four human cars maximum; no AI lamps","existing low-grass no-shadow policy unchanged","no atmospheric beam volume or lens-emitter glow"]})}

#[cfg(test)]mod tests {
    use super::*;use glam::Quat;
    fn pose()->CarPose {CarPose {pos:Vec3::new(1.0,2.0,3.0),rot:Quat::IDENTITY,color:Vec3::ONE,
        boxes:vec![[1.8,0.6,4.0,0.0,0.4,0.0]],model:String::new(),ride:0.03,wheels:vec![]}}
    #[test]fn night_lamps_follow_body_and_stay_outside_front_bound() {
        let night=Lighting {sun_lux:0.0,sun_dir:Vec3::NEG_Y,..Default::default()};
        let p=pose();let a=from_poses(&[p],&[0],&night)[0];assert!((a.position.z-0.88).abs()<1e-5);
        let q=Quat::from_euler(glam::EulerRot::YXZ,0.7,0.2,-0.15);let mut p=pose();p.rot=q;
        let b=from_poses(&[p],&[0],&night)[0];
        assert!(b.direction.abs_diff_eq(q*a.direction,1e-6));assert!(b.up.abs_diff_eq(q*Vec3::Y,1e-6));
        assert!(b.position.abs_diff_eq(Vec3::new(1.0,2.0,3.0)+q*(a.position-Vec3::new(1.0,2.0,3.0)),1e-5));
        assert!(from_poses(&[pose()],&[0],&Lighting::default()).is_empty());
        assert_eq!(from_poses(&(0..7).map(|_|pose()).collect::<Vec<_>>(),&[0,1,2,3,4,5,6],&night).len(),4);
    }
}
