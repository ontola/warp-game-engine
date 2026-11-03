//! Vehicle hulls. EMP disables engines, never the physical hull.
use crate::{handling::Pilot, race_rules::Pose, track::Node};
use glam::{DVec2, Vec3};

pub fn solid(p: &Pilot) -> bool {
    !p.crashed && !p.finished && p.recovery <= 0. && p.warp_time <= 0.
}

/// Retained Godot oriented footprint, including the nose-centred offset and
/// tube seam. Coordinates here are lateral/along-track, not world X/Z.
pub fn ground(a: &Pilot, b: &Pilot, length: f64, n: &Node) -> DVec2 {
    let gap = (a.distance-b.distance+length*0.5).rem_euclid(length)-length*0.5;
    let mut lateral = a.x-b.x;
    if n.geometry.shape_angle > std::f64::consts::PI-0.001 {
        lateral = (lateral+n.geometry.width).rem_euclid(n.geometry.width*2.)-n.geometry.width;
    }
    if gap.abs()>15. || lateral.abs()>15. {return DVec2::ZERO;}
    let af=DVec2::new(a.heading.sin(),a.heading.cos());
    let bf=DVec2::new(b.heading.sin(),b.heading.cos());
    let ar=DVec2::new(af.y,-af.x);let br=DVec2::new(bf.y,-bf.x);
    let delta=DVec2::new(lateral,gap)+(af-bf)*0.65;
    let mut depth=f64::INFINITY;let mut normal=DVec2::ZERO;
    for axis in [ar,af,br,bf] {
        let extent=4.7*(ar.dot(axis).abs()+br.dot(axis).abs())
            +4.45*(af.dot(axis).abs()+bf.dot(axis).abs());
        let overlap=extent-delta.dot(axis).abs();
        if overlap<=0. {return DVec2::ZERO;}
        if overlap<depth {depth=overlap;normal=axis*if delta.dot(axis)>=0. {1.}else{-1.};}
    }
    normal*depth
}

/// World-space oriented hulls for air/air and air/road contacts. Unlike the
/// previous ten-metre sphere, flying over a car is not itself a collision.
pub fn air(a: Pose, b: Pose) -> Vec3 {
    let delta=a.origin+a.basis.z_axis*0.65-b.origin-b.basis.z_axis*0.65;
    if delta.length_squared()>180. {return Vec3::ZERO;}
    let aa=[a.basis.x_axis,a.basis.y_axis,a.basis.z_axis];
    let bb=[b.basis.x_axis,b.basis.y_axis,b.basis.z_axis];
    let extents=[4.7,1.15,4.45];
    let mut depth=f32::INFINITY;let mut normal=Vec3::ZERO;
    let axes=aa.into_iter().chain(bb).chain(aa.into_iter().flat_map(|x|bb.map(|y|x.cross(y))));
    for axis in axes {
        if axis.length_squared()<1e-6 {continue;}
        let axis=axis.normalize();
        let extent=(0..3).map(|i|extents[i]*(aa[i].dot(axis).abs()+bb[i].dot(axis).abs())).sum::<f32>();
        let overlap=extent-delta.dot(axis).abs();
        if overlap<=0. {return Vec3::ZERO;}
        if overlap<depth {depth=overlap;normal=axis*if delta.dot(axis)>=0. {1.}else{-1.};}
    }
    normal*depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat3;
    #[test]
    fn air_hulls_allow_overflight_but_block_rotated_impacts() {
        let a=Pose{origin:Vec3::ZERO,basis:Mat3::IDENTITY};
        assert_eq!(air(a,Pose{origin:Vec3::Y*3.,..a}),Vec3::ZERO);
        let b=Pose{origin:Vec3::new(3.,0.,2.),basis:Mat3::from_rotation_y(0.7)};
        let separation=air(a,b);assert!(separation.length()>1.);
        assert_eq!(air(Pose{origin:a.origin+separation*1.001,..a},b),Vec3::ZERO);
    }
}
