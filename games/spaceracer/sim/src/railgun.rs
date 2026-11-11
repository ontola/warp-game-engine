//! Straight, collision-aware hitscan with the original camera-matched guide.
//! The view is explicit serializable player input; bots use the fixed aim cone.
use glam::{Affine3A, Vec2, Vec3};
use crate::{environment::Obstacle, race::{Race,Weapon}};
pub const MUZZLE:Vec3=Vec3::new(0.,1.47,1.9);
/// Metres the road may bulge over the shot line before it blocks it.
pub const ROAD_CLEARANCE:f32=3.;
#[derive(Clone,Copy,Debug,serde::Serialize,serde::Deserialize)]
pub struct Aim {pub eye:[f32;3],pub forward:[f32;3],pub right:[f32;3],pub up:[f32;3],pub radius:f32}
impl Aim {
    pub fn camera(eye:Vec3,forward:Vec3,up:Vec3,fov:f32,radius:f32)->Self {
        let forward=forward.normalize_or_zero();let right=forward.cross(up).normalize_or_zero();
        Self{eye:eye.to_array(),forward:forward.to_array(),right:right.to_array(),up:right.cross(forward).to_array(),radius:2.*(fov*0.5).tan()*radius}
    }
    pub fn project(&self,point:Vec3)->Option<Vec2> {
        let d=point-Vec3::from(self.eye);let depth=d.dot(self.forward.into());
        (depth>0.01 && depth.is_finite() && self.radius.is_finite() && self.radius>0.).then(||Vec2::new(d.dot(self.right.into()),d.dot(self.up.into()))/depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::race::Controls;
    fn race()->Race {
        let mut r=Race::new(35,"normal","city",1,3);r.countdown=0.;
        for i in 0..6 {
            r.drivers[i].bot=false;r.pilots[i].airborne=true;r.pilots[i].recovery=0.;r.pilots[i].crashed=i>2;
            r.drivers[i].motion.air.position=Vec3::new(0.,10000.,i as f32*100.);
            r.drivers[i].motion.air.frame=glam::Mat3::IDENTITY;
        }
        r.drivers[0].item=Some(Weapon::Railgun);r
    }
    #[test]
    fn closest_to_circle_centre_not_nearest_distance_and_range_is_bounded() {
        let mut r=race();r.drivers[1].motion.air.position.x=11.;
        assert_eq!(r.rail_trace(0,None,true).target,Some(2));
        r.drivers[1].motion.air.position.x=40.;r.drivers[2].motion.air.position.z=700.;
        let shot=r.rail_trace(0,None,true);assert_eq!(shot.target,None);
        assert!((shot.from.distance(shot.to)-650.).abs()<0.01);
    }
    #[test]
    fn intervening_hull_and_scenery_block_the_straight_shot() {
        let mut r=race();assert_eq!(r.rail_trace(0,None,true).target,Some(1));
        r.obstacles.push(Obstacle::bounds(Vec3::new(-10.,9990.,45.),Vec3::new(10.,10010.,55.)));
        let shot=r.rail_trace(0,None,true);assert_eq!(shot.target,None);assert!((shot.to.z-45.).abs()<0.01);
        assert_eq!(r.rail_trace(0,None,false).target,Some(1));
    }
    #[test]
    fn camera_circle_works_with_lens_changes_and_serialized_input() {
        let mut r=race();r.drivers[1].motion.air.position.x=18.;r.pilots[2].crashed=true;
        assert_eq!(r.rail_trace(0,None,false).target,None);
        let aim=Aim::camera(Vec3::new(0.,10004.,-16.),Vec3::Z,Vec3::Y,90_f32.to_radians(),0.11);
        assert_eq!(r.rail_trace(0,Some(aim),false).target,Some(1));
        let input=Controls{rail_aim:Some(aim),fire:true,..Default::default()};
        let decoded:Controls=serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
        assert_eq!(r.rail_trace(0,decoded.rail_aim,false).target,Some(1));
        let tight=Aim::camera(Vec3::new(0.,10004.,-16.),Vec3::Z,Vec3::Y,45_f32.to_radians(),0.11);
        assert_eq!(r.rail_trace(0,Some(tight),false).target,None);
    }
    #[test]
    fn tap_fires_one_fifty_damage_round_from_the_muzzle() {
        let mut r=race();let before=r.pilots[1].energy;
        r.step(&[Controls{fire:true,..Default::default()}]);
        assert_eq!(r.drivers[0].item,None);assert_eq!(r.beams.len(),1);
        assert_eq!(r.beams[0].target,Some(1));assert!((r.pilots[1].energy-before+50.).abs()<0.1);
        assert!(r.beams[0].from.y>10001.);
        r.step(&[Controls{fire:true,..Default::default()}]);assert_eq!(r.beams.len(),1);
    }
}
#[derive(Clone,Copy,Debug)]
pub struct Trace {pub from:Vec3,pub to:Vec3,pub target:Option<usize>}
impl Race {
    pub fn rail_ready(&self,i:usize)->bool {
        let p=&self.pilots[i];let d=&self.drivers[i];
        d.item==Some(Weapon::Railgun) && d.active.is_none() && !p.crashed && !p.finished
            && p.recovery<=0. && p.emp_time<=0. && p.warp_time<=0. && self.countdown<=0. && !self.over
    }
    /// Select an aim point within the guide. Obstructions are resolved along
    /// the resulting straight ray, so a closer hull can intercept the shot.
    fn rail_aim_point(&self, shooter: usize, from: Vec3, basis: glam::Mat3, view: Option<Aim>) -> Vec3 {
        let ahead = from + basis.z_axis * 200.0;
        let guide = view.and_then(|aim| aim.project(ahead).map(|center| (aim, center)));
        let mut best_error = 1.000001;
        let mut end = from + basis.z_axis * self.combat.rail_range;
        for (index, pilot) in self.pilots.iter().enumerate() {
            if index == shooter || pilot.crashed || pilot.finished || pilot.recovery > 0.0 || pilot.warp_time > 0.0 {
                continue;
            }
            let body = self.pose(index);
            let point = body.origin + body.basis.y_axis * 0.6;
            let local = basis.transpose() * (point - from);
            if local.z <= 0.01 || point.distance(from) > self.combat.rail_range {
                continue;
            }
            let error = match guide {
                Some((aim, center)) => aim.project(point)
                    .map_or(f32::INFINITY, |projected| projected.distance(center) / aim.radius),
                None => local.truncate().length() / (local.z * self.combat.rail_tangent),
            };
            if error < best_error {
                best_error = error;
                end = point;
            }
        }
        end
    }

    pub fn rail_trace(&self, shooter: usize, view: Option<Aim>, scenery: bool) -> Trace {
        let pose = self.pose(shooter);
        let from = pose.origin + pose.basis * MUZZLE;
        let end = self.rail_aim_point(shooter, from, pose.basis, view);
        let mut shot = Trace { from, to: end, target: None };
        let mut closest = 1.00001;
        for (index, pilot) in self.pilots.iter().enumerate() {
            if index == shooter || pilot.crashed || pilot.finished {
                continue;
            }
            let body = self.pose(index);
            let hull = Obstacle::oriented(
                Vec3::new(-4.5, -0.7, -4.0),
                Vec3::new(4.5, 2.1, 4.0),
                Affine3A::from_mat3_translation(body.basis, body.origin),
            );
            if let Some((fraction, _)) = hull.trace(from, end) {
                if fraction < closest {
                    closest = fraction;
                    shot.to = from.lerp(end, fraction);
                    shot.target = (pilot.warp_time <= 0.0 && pilot.recovery <= 0.0).then_some(index);
                }
            }
        }
        if scenery {
            let wall = self.obstacles.iter()
                .filter_map(|obstacle| obstacle.trace(from, shot.to).map(|hit| hit.0))
                .min_by(f32::total_cmp);
            // The road only blocks when it rises well over the line, so a gentle
            // crest between two hover racers does not eat a shot inside the circle.
            let lift = pose.basis.y_axis * ROAD_CLEARANCE;
            let road = self.world.track.sweep_road(from + lift, shot.to + lift).map(|hit| hit.t);
            if let Some(fraction) = wall.into_iter().chain(road).min_by(f32::total_cmp) {
                shot.to = from.lerp(shot.to, fraction);
                shot.target = None;
            }
        }
        shot
    }
}
