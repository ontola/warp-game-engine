//! Original vehicle-relative chase camera. Render integration supplies this up axis.
use crate::{handling::smooth, race_rules::Pose};
use glam::{Mat3, Quat, Vec3};
#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct Tuning {
    pub distance: f64, pub height: f64, pub speed_pullback: f64, pub boost_pullback: f64,
    pub fov_base: f64, pub fov_speed: f64, pub fov_surge: f64, pub fov_boost: f64, pub shake: f64,
}
impl Default for Tuning {
    fn default()->Self {Self{distance:18.,height:7.,speed_pullback:0.008,boost_pullback:1.3,
        fov_base:72.,fov_speed:10.,fov_surge:5.,fov_boost:9.,shake:1.}}
}
impl Tuning {
    pub fn valid(self)->bool {
        [self.distance,self.height,self.speed_pullback,self.boost_pullback,self.fov_base,self.fov_speed,self.fov_surge,self.fov_boost,self.shake].iter().all(|v|v.is_finite())
            && (12.0..=24.).contains(&self.distance) && (3.0..=9.).contains(&self.height)
            && (0.0..=0.02).contains(&self.speed_pullback) && (0.0..=3.).contains(&self.boost_pullback)
            && (60.0..=80.).contains(&self.fov_base) && [self.fov_speed,self.fov_surge,self.fov_boost].iter().all(|v|(0.0..=15.).contains(v))
            && (0.0..=2.).contains(&self.shake)
    }
}
#[derive(Clone, Debug)]
pub struct Chase {
    pub position: Vec3,
    pub frame: Mat3,
    pub rotation: Quat,
    pub distance: f64,
    pub fov: f64,
    pub boost: f64,
    pub surge: f64,
    pub rush: f64,
    pub clock: f64,
    pub velocity: Vec3,
    initialized: bool,
}
impl Default for Chase {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            frame: Mat3::IDENTITY,
            rotation: Quat::IDENTITY,
            distance: 0.0,
            fov: 75.0,
            boost: 0.0,
            surge: 0.0,
            rush: 0.0,
            clock: 0.0,
            velocity: Vec3::ZERO,
            initialized: false,
        }
    }
}
fn blend(a: f64, b: f64, dt: f64, rate: f64) -> f64 {
    a + (b - a) * (1.0 - (-dt * rate).exp())
}
impl Chase {
    pub fn update(
        &mut self,
        pose: Pose,
        speed: f64,
        boosting: bool,
        acceleration: f64,
        dt: f64,
        snap: bool,
        effects: bool,
        aspect: f64,
    ) {
        self.update_tuned(pose,speed,boosting,acceleration,dt,snap,effects,aspect,Tuning::default());
    }
    pub fn update_tuned(&mut self, pose:Pose, speed:f64, boosting:bool, acceleration:f64,
        dt:f64, snap:bool, effects:bool, aspect:f64, tuning:Tuning) {
        let previous = self.position;
        let rush = if effects {
            smooth(80.0, 465.0, speed)
        } else {
            0.0
        };
        let target_boost = if boosting && effects {
            smooth(180.0, 350.0, speed)
        } else {
            0.0
        };
        let target_surge = if effects {
            smooth(15.0, 180.0, acceleration) * smooth(0.0, 90.0, speed)
        } else {
            0.0
        };
        self.boost = if snap {
            target_boost
        } else {
            blend(
                self.boost,
                target_boost,
                dt,
                if target_boost > self.boost { 10.0 } else { 3.0 },
            )
        };
        self.surge = if snap {
            target_surge
        } else {
            blend(
                self.surge,
                target_surge,
                dt,
                if target_surge > self.surge { 8.0 } else { 2.5 },
            )
        };
        self.rush = rush;
        let wanted = Quat::from_mat3(&pose.basis);
        if snap || !self.initialized {
            self.rotation = wanted;
        } else {
            self.rotation = self
                .rotation
                .slerp(wanted, (1.0 - (-dt * 6.5).exp()) as f32)
                .normalize();
        }
        let frame = Mat3::from_quat(self.rotation);
        let wanted_distance = tuning.distance + speed * tuning.speed_pullback + self.boost * tuning.boost_pullback + self.surge * 0.6;
        self.distance = if snap || !self.initialized {
            wanted_distance
        } else {
            blend(self.distance, wanted_distance, dt, 5.0)
        };
        self.position = pose.origin - frame.z_axis * self.distance as f32 + frame.y_axis * tuning.height as f32;
        let target = pose.origin + frame.z_axis * 24.0 + frame.y_axis * 1.5;
        let z = (self.position - target).normalize();
        let x = frame.y_axis.cross(z).normalize();
        let y = z.cross(x);
        self.frame = Mat3::from_cols(x, y, z);
        let extreme = if effects {
            smooth(465.0, 605.0, speed)
        } else {
            0.0
        };
        let reference = (tuning.fov_base + rush*tuning.fov_speed + self.surge*tuning.fov_surge + self.boost*tuning.fov_boost + extreme*4.)
            .clamp(tuning.fov_base, 100.0);
        let fov = (2.0
            * ((reference.to_radians() * 0.5).tan() * (16.0 / 9.0 / aspect).min(1.0)).atan())
        .to_degrees();
        self.fov = if snap {
            fov
        } else {
            blend(self.fov, fov, dt, 5.0)
        };
        if effects {
            self.clock += dt.min(0.05);
        }
        let shake = if effects && !snap {
            self.boost * (0.4 + 0.6 * rush) * tuning.shake
        } else {
            0.0
        };
        self.position += frame.x_axis * ((self.clock * 73.0).sin() * 0.035 * shake) as f32
            + frame.y_axis * ((self.clock * 91.0).sin() * 0.025 * shake) as f32;
        self.frame = self.frame
            * Mat3::from_rotation_x(
                (0.16_f64.to_radians()
                    * shake
                    * ((self.clock * 67.0).sin() + 0.3 * (self.clock * 109.0).sin()))
                    as f32,
            )
            * Mat3::from_rotation_y(
                (0.12_f64.to_radians() * shake * (self.clock * 83.0).sin()) as f32,
            )
            * Mat3::from_rotation_z(
                (0.08_f64.to_radians() * shake * (self.clock * 57.0).sin()) as f32,
            );
        if dt > 0.0 {
            self.velocity = if snap {
                Vec3::ZERO
            } else {
                (self.position - previous) / dt as f32
            };
        }
        self.initialized = true;
    }
    pub fn forward(&self) -> Vec3 {
        -self.frame.z_axis
    }
    pub fn up(&self) -> Vec3 {
        self.frame.y_axis
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_chase_camera_through_rolls_and_split_aspects() {
        let cases: serde_json::Value = serde_json::from_str(&crate::oracle("chase")).unwrap();
        for c in cases.as_array().unwrap() {
            let aspect = c["aspect"].as_f64().unwrap();
            let mut chase = Chase::default();
            for (tick, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                let t = tick as f64 / 120.0;
                let basis = Mat3::from_rotation_y((t * 0.2) as f32)
                    * Mat3::from_rotation_x((t * 0.5) as f32)
                    * Mat3::from_rotation_z((t * 0.3) as f32);
                let pose = Pose {
                    basis,
                    origin: Vec3::new((t * 20.0) as f32, (t * 3.0) as f32, (t * 100.0) as f32),
                };
                chase.update(
                    pose,
                    300.0 + t * 20.0,
                    tick > 100,
                    50.0,
                    1.0 / 120.0,
                    tick == 0,
                    true,
                    aspect,
                );
                for (key, v) in [
                    ("position", chase.position),
                    ("up", chase.up()),
                    ("forward", chase.forward()),
                ] {
                    let expected: Vec3 = serde_json::from_value(f[key].clone()).unwrap();
                    assert!(
                        v.distance(expected) < 0.002,
                        "aspect {aspect} tick {tick} {key} {v:?} expected {expected:?}"
                    );
                }
                assert!(
                    (chase.fov - f["fov"].as_f64().unwrap()).abs() < 0.0001,
                    "aspect {aspect} tick {tick} fov"
                );
            }
        }
    }
}
