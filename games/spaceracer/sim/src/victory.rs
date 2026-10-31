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
    pub pose: Pose,
    pub offset: Vec3,
    pub rotation: Quat,
    pub shot: Shot,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Shot {
    pub speed: f64,
    pub throttle: f64,
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
            pose,
            offset: frame.inverse() * (pose.origin - n.point(self.pilots[i].x, 1.3)),
            rotation: Quat::from_mat3(&(frame.inverse() * pose.basis)).normalize(),
            shot: Shot {
                speed: 200.,
                throttle: 0.72,
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
        p.boost = 0.;
        p.emp_time = 0.;
        p.warp_time = 0.;
        self.drivers[i].warp_fx=0.;
        p.recovery = 0.;
        p.on_pad = false;
        self.drivers[i].finish_time.get_or_insert(self.clock);
    }
    pub(crate) fn step_victory(&mut self, i: usize) {
        self.begin_victory(i);
        let n = self.world.track.sample(self.pilots[i].distance + 110.);
        let age = self.drivers[i].victory.as_ref().unwrap().age + DT;
        let shot: Shot = match self.rules.call(
            "victory",
            json!({"age":age,"curve":n.curve,"crest":n.crest,"speed":self.pilots[i].speed}),
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
            || !(1.0..=400.).contains(&shot.speed)
            || !shot.fov.is_finite()
            || !(20.0..=100.).contains(&shot.fov)
            || shot.offset.iter().any(|v| !v.is_finite() || v.abs() > 100.)
        {
            self.script_error = Some("invalid victory choreography".into());
            return;
        }
        let p = &mut self.pilots[i];
        let old_speed = p.speed;
        p.speed = toward(p.speed, shot.speed, 45. * DT);
        p.acceleration = (p.speed - old_speed) / DT;
        p.distance += p.speed * DT;
        let n = self.world.track.sample(p.distance);
        let goal = if n.split_gap > 0.01 {
            (if n.preferred_route < 0. { -1. } else { 1. }) * (n.split_gap + 8.)
        } else {
            0.
        };
        p.x += (goal - p.x) * (1. - (-DT * 2.5).exp());
        p.x = p.x.clamp(-n.geometry.width * 0.8, n.geometry.width * 0.8);
        if n.split_gap > 0.01 && p.x.abs() < n.split_gap + 5. {
            p.x = goal;
        }
        p.heading *= (-DT * 3.).exp();
        p.trim = 0.;
        p.slip = 0.;
        p.slide = 0.;
        p.unload = 0.;
        p.lift = 0.;
        let d = &mut self.drivers[i];
        d.input = crate::race::Controls {
            drive: Input {
                throttle: shot.throttle,
                steer: shot.steer,
                strafe: shot.strafe,
                trim: shot.trim,
                brake: (-p.acceleration / 140.).clamp(0., 0.4),
                boost: false,
            },
            ..Default::default()
        };
        let v = d.victory.as_mut().unwrap();
        let frame = n.frame(p.x);
        let prev = v.pose.origin;
        v.age = age;
        v.shot = shot;
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
                        .slerp(Quat::IDENTITY, smooth(0., 1.3, age) as f32),
                ),
        };
        d.motion.ground_velocity = (v.pose.origin - prev) / DT as f32;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
