//! Noncompetitive finish lap. Retains hardware, animates controls and survives race completion.
use crate::{
    handling::{DT, Input, smooth, toward},
    race::Race,
    race_rules::Pose,
};
use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[derive(Clone, Debug)]
pub struct Victory {
    pub age: f64,
    pub next_boost: f64,
    pub bank: f64,
    pub pose: Pose,
    pub offset: Vec3,
    pub rotation: Quat,
    pub shot: Shot,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Shot {
    pub speed: f64,
    pub throttle: f64,
    pub acceleration: f64,
    pub brake: f64,
    pub boost: bool,
    pub boost_delay: f64,
    pub line: f64,
    pub heading: f64,
    pub bank: f64,
    pub steer: f64,
    pub strafe: f64,
    pub trim: f64,
    pub offset: [f32; 3],
    pub fov: f32,
    pub hop_height: f64,
}
impl Race {
    pub fn begin_victory(&mut self, i: usize) {
        if self.drivers[i].victory.is_some() {
            return;
        }
        let pose = self.pose(i);
        let n = self.world.track.sample(self.pilots[i].distance);
        let frame = n.frame(self.pilots[i].x);
        self.drivers[i].victory = Some(Victory {
            age: 0.,
            next_boost: 0.2+i as f64*0.37,
            bank: 0.,
            pose,
            offset: frame.inverse() * (pose.origin - n.point(self.pilots[i].x, 1.3)),
            rotation: Quat::from_mat3(&(frame.inverse() * pose.basis)).normalize(),
            shot: Shot {
                speed: self.pilots[i].speed,
                throttle: 1.,
                acceleration: 150., brake: 0., boost: false, boost_delay: 4.,
                line: 0., heading: 0., bank: 0.,
                steer: 0.,
                strafe: 0.,
                trim: 0.,
                offset: [-13., 6., -21.],
                fov: 58.,
                hop_height: 18.,
            },
        });
        let p = &mut self.pilots[i];
        p.finished = true;
        p.crashed = false;
        p.airborne = false;
        // Keep a boost already burning across the finish line.
        p.emp_time = 0.;
        p.warp_time = 0.;
        self.drivers[i].warp_fx=0.;
        p.recovery = 0.;
        p.on_pad = false;
        self.drivers[i].finish_time.get_or_insert(self.clock);
    }
    pub(crate) fn step_victory(&mut self, i: usize) {
        self.begin_victory(i);
        let p=&self.pilots[i];
        let n = self.world.track.sample(p.distance);
        let next = self.world.track.sample(p.distance+110.);
        let age = self.drivers[i].victory.as_ref().unwrap().age + DT;
        let preview:Vec<_>=[0.,40.,90.,160.,260.,380.].into_iter().map(|ahead| {
            let ahead=ahead*(p.speed/crate::handling::BOOST_SPEED).powi(2).max(1.);
            let n=self.world.track.sample(p.distance+ahead);
            json!({"ahead":ahead,"curve":n.curve,"hairpin":n.feature=="hairpin"})
        }).collect();
        let remaining=(p.boost-DT).max(0.);
        let near_jump=self.world.jump_distances.iter().any(|j|(j[0]-p.distance).rem_euclid(self.world.track.length)<350.);

        let shot: Shot = match self.rules.call(
            "victory",
            json!({"age":age,"slot":i,"curve":n.curve,"next_curve":next.curve,"crest":n.crest,"speed":p.speed,
                "preview":preview,"boost":remaining,"ready":age>=self.drivers[i].victory.as_ref().unwrap().next_boost,
                "on_pad":n.zone=="boost","near_jump":near_jump,"top_speed":crate::handling::TOP_SPEED,
                "boost_speed":crate::handling::BOOST_SPEED,"stack_speed":crate::handling::STACK_BOOST_SPEED}),
        ) {
            Ok(v) => v,
            Err(e) => {
                self.script_error = Some(e);
                return;
            }
        };
        if !shot.hop_height.is_finite()
            || !(0.0..=50.).contains(&shot.hop_height)
            || !shot.speed.is_finite()
            || !(1.0..=crate::handling::MAX_SPEED).contains(&shot.speed)
            || !shot.fov.is_finite()
            || !(20.0..=100.).contains(&shot.fov)
            || shot.offset.iter().any(|v| !v.is_finite() || v.abs() > 100.)
            || !(1.0..=600.).contains(&shot.acceleration)
            || !(0.0..=1.).contains(&shot.throttle) || !(0.0..=1.).contains(&shot.brake)
            || !(1.0..=15.).contains(&shot.boost_delay)
            || [shot.steer,shot.strafe,shot.trim,shot.line,shot.heading,shot.bank].iter().any(|v| !v.is_finite() || v.abs()>1.)
        {
            self.script_error = Some("invalid victory choreography".into());
            return;
        }
        let p = &mut self.pilots[i];
        let old_speed = p.speed;
        p.boost=remaining;
        if shot.boost && remaining<=0. {
            p.boost=p.boost_duration;
            self.drivers[i].victory.as_mut().unwrap().next_boost=age+shot.boost_delay;
        }
        p.speed = toward(p.speed, shot.speed, shot.acceleration * DT);
        p.acceleration = (p.speed - old_speed) / DT;
        p.distance += p.speed * DT;
        let n = self.world.track.sample(p.distance);
        let goal = if n.split_gap > 0.01 {
            (if n.preferred_route < 0. { -1. } else { 1. }) * (n.split_gap + 8.)
        } else if n.dead_side.abs()>0.01 {
            -n.dead_side.signum()*n.geometry.width*0.45
        } else {
            shot.line*n.geometry.width
        };
        let old_x=p.x;
        p.x += (goal - p.x) * (1. - (-DT * 2.5).exp());
        p.x = p.x.clamp(-n.geometry.width * 0.8, n.geometry.width * 0.8);
        if n.split_gap > 0.01 && p.x.abs() < n.split_gap + 5. {
            p.x = goal;
        }
        p.heading=toward(p.heading,shot.heading,1.4*DT);
        p.trim=shot.trim;
        p.slip=(p.x-old_x)/DT;
        p.slide=toward(p.slide,shot.brake*0.65,3.*DT);
        p.drifting=p.slide>0.18;
        p.on_pad=n.zone=="boost" && !n.air_gap;

        p.unload = 0.;
        p.lift = 0.;
        let d = &mut self.drivers[i];
        d.input = crate::race::Controls {
            drive: Input {
                throttle: shot.throttle,
                steer: shot.steer,
                strafe: shot.strafe,
                trim: shot.trim,
                brake: shot.brake,
                boost: p.boost>0.,
            },
            ..Default::default()
        };
        let v = d.victory.as_mut().unwrap();
        let frame = n.frame(p.x);
        let prev = v.pose.origin;
        v.age = age;
        v.bank=toward(v.bank,shot.bank,1.5*DT);
        v.shot = shot;
        let racing_rotation=Quat::from_rotation_y(-p.heading as f32)*Quat::from_rotation_z(v.bank as f32);
        let wrapped = p.distance.rem_euclid(self.world.track.length);
        let hop = self
            .world
            .jump_distances
            .iter()
            .find(|j| wrapped >= j[0] && wrapped <= j[1])
            .map_or(0., |j| {
                ((wrapped - j[0]) / (j[1] - j[0]) * std::f64::consts::PI)
                    .sin()
                    .powi(2)
                    * v.shot.hop_height
            });
        v.pose = Pose {
            origin: n.point(p.x, 1.3 + hop) + frame * v.offset * (-age as f32 * 4.).exp(),
            basis: frame
                * Mat3::from_quat(
                    v.rotation
                        .slerp(racing_rotation, smooth(0., 1.3, age) as f32),
                ),
        };
        let velocity=(v.pose.origin-prev)/DT as f32;
        d.motion.ground_acceleration=(velocity-d.motion.ground_velocity)/DT as f32;
        d.motion.ground_velocity=velocity;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finished_racers_boost_at_race_speed_and_preserve_finish_results() {
        let mut r=Race::new(35,"hard","city",1,1);
        r.countdown=0.;
        for i in 0..6 {r.pilots[i].speed=300.;r.begin_victory(i);}
        r.over=true;
        let mut max_speed=0f64;let mut boosted=[false;6];let mut first=[None;6];
        for tick in 0..2400 {
            r.step(&[]);
            assert!(r.script_error.is_none(),"{:?}",r.script_error);
            for i in 0..6 {
                let p=&r.pilots[i];
                assert!(p.finished && !p.crashed);
                assert_eq!(r.drivers[i].finish_time,Some(0.));
                assert!(r.pose(i).origin.is_finite() && r.pose(i).basis.is_finite());
                if p.boost>0. {boosted[i]=true;first[i].get_or_insert(tick);}
                max_speed=max_speed.max(p.speed);
            }
        }
        assert!(boosted.iter().all(|v|*v),"{boosted:?}");
        assert!(max_speed>440.,"boost must change actual speed: {max_speed}");
        assert_ne!(first[0],first[1],"drivers should not boost in lockstep");
        // Crossing the line does not cut off a boost already burning.
        let mut r=Race::new(35,"hard","city",1,1);r.pilots[0].boost=0.9;r.begin_victory(0);
        assert_eq!(r.pilots[0].boost,0.9);
    }
    #[test]
    fn victory_policy_brakes_for_hairpins_but_not_the_end_of_a_straight_boost() {
        let mut rules=crate::rules::Rules::new(None).unwrap();
        let mut c=json!({"age":2.,"slot":0,"curve":0.,"next_curve":0.,"crest":0.,"speed":460.,
            "boost":0.,"ready":false,"on_pad":false,"near_jump":false,
            "top_speed":315.,"boost_speed":465.,"stack_speed":565.,
            "preview":[{"ahead":0.,"curve":0.,"hairpin":false}]});
        let straight:Shot=rules.call("victory",c.clone()).unwrap();
        assert_eq!(straight.brake,0.);assert_eq!(straight.throttle,1.);assert!(straight.acceleration<50.);
        c["preview"]=json!([{"ahead":0.,"curve":0.02,"hairpin":true}]);
        let corner:Shot=rules.call("victory",c).unwrap();
        assert!(corner.speed<=110. && corner.brake>0.9 && !corner.boost);
    }
    #[test]
    fn victory_keeps_moving_after_results_without_consuming_hardware() {
        let mut r = Race::new(35, "hard", "city", 1, 1);
        r.countdown = 0.;
        r.drivers[0].item = Some(crate::race::Weapon::Missile);
        for i in 0..6 {
            r.begin_victory(i);
        }
        r.over = true;
        let start = r.pose(0).origin;
        for _ in 0..2400 {
            r.step(&[]);
        }
        assert!(r.script_error.is_none());
        assert!(r.pose(0).origin.distance(start) > 100.);
        assert!(r.drivers[0].victory.as_ref().unwrap().age > 19.9);
        assert_eq!(r.drivers[0].item, Some(crate::race::Weapon::Missile));
        assert!(!r.pilots[0].crashed);
        assert_eq!(r.drivers[0].finish_time, Some(0.));
    }
}
