//! Source flight.gd aerodynamic integrator. No road attraction during free flight.
use crate::handling::{Input, smooth};
use glam::{Mat3, Quat, Vec3};
#[derive(Clone, Debug)]
pub struct Flight {
    pub position: Vec3,
    pub velocity: Vec3,
    pub frame: Mat3,
    pub rates: Vec3,
    pub time: f64,
    pub roll: f64,
    pub handoff: f64,
    pub entry_rates: Vec3,
    pub entry_acceleration: Vec3,
    pub entry_speed: f64,
    pub soft_takeoff: bool,
    pub emp: bool,
}
impl Default for Flight {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 500.0, 0.0),
            velocity: Vec3::Z * 200.0,
            frame: Mat3::IDENTITY,
            rates: Vec3::ZERO,
            time: 0.0,
            roll: 0.0,
            handoff: 0.0,
            entry_rates: Vec3::ZERO,
            entry_acceleration: Vec3::ZERO,
            entry_speed: 200.0,
            soft_takeoff: false,
            emp: false,
        }
    }
}
fn orthonormal(m: Mat3) -> Mat3 {
    let x = m.x_axis.normalize();
    let y = (m.y_axis - x * x.dot(m.y_axis)).normalize();
    let z = (m.z_axis - x * x.dot(m.z_axis) - y * y.dot(m.z_axis)).normalize();
    Mat3::from_cols(x, y, z)
}
impl Flight {
    pub fn step(&mut self, c: Input, dt: f64) {
        self.time += dt;
        let throttle = if self.emp { 0.0 } else { c.throttle };
        let mut frame = self.frame;
        let mut v = self.velocity;
        let speed = v.length() as f64;
        let authority = 0.25 + 0.75 * smooth(65.0, 200.0, speed);
        let remaining = self.handoff;
        let handoff = smooth(0.0, 0.65, 0.65 - remaining + dt * 0.5);
        self.handoff = (remaining - dt).max(0.0);
        let mut desired = Vec3::new(
            (c.trim * 1.8) as f32,
            (-c.steer * 1.65) as f32,
            (c.strafe * 3.8) as f32,
        ) * authority as f32;
        if remaining > 0.0 {
            desired = self.entry_rates.lerp(desired, handoff as f32);
        }
        self.rates = self.rates.lerp(desired, (1.0 - (-dt * 24.0).exp()) as f32);
        let forward = v.dot(frame.z_axis).max(0.0) as f64;
        let alignment = if speed > 1.0 {
            frame.z_axis.dot(v.normalize()) as f64
        } else {
            0.0
        };
        let grip = smooth(80.0, 180.0, forward) * smooth(0.25, 0.8, alignment) * 0.95 * handoff;
        let path = frame * Vec3::new(self.rates.x, self.rates.y, 0.0) * dt as f32 * grip as f32;
        if path.length_squared() > 0.000000001 {
            v = Quat::from_axis_angle(path.normalize(), path.length()) * v;
        }
        let rotation = self.rates * dt as f32;
        if rotation.length_squared() > 0.000000001 {
            frame = orthonormal(
                frame
                    * Mat3::from_quat(Quat::from_axis_angle(
                        rotation.normalize(),
                        rotation.length(),
                    )),
            );
        }
        self.frame = frame;
        self.roll += self.rates.z as f64 * dt;
        let airflow = if speed > 1.0 {
            v.normalize()
        } else {
            frame.z_axis
        };
        let attack = (-v.dot(frame.y_axis) as f64).atan2((v.dot(frame.z_axis) as f64).max(1.0));
        let mut lift = frame.y_axis - airflow * frame.y_axis.dot(airflow);
        if lift.length_squared() > 0.001 {
            lift = lift.normalize();
        }
        let stall = 1.0 - smooth(0.48, 1.05, attack.abs());
        let forward = (v.dot(frame.z_axis) as f64).max(0.0);
        let attached = smooth(75.0, 155.0, forward);
        let camber = 0.22 + 0.58 * smooth(100.0, 210.0, forward);
        let coefficient = (camber + attack * 7.0).clamp(-2.0, 4.0) * stall;
        let lift_force = (forward * forward * 0.0011 * coefficient * attached).clamp(-260.0, 350.0);
        let thrust = 140.0 + (44.0 - 140.0) * smooth(90.0, 210.0, forward);
        let mut force = frame.z_axis * throttle as f32 * thrust as f32 + lift * lift_force as f32
            - Vec3::Y * 48.0;
        force -= v
            * (0.035
                + speed * 0.00075
                + c.brake * 0.65
                + attack.abs() * 0.12
                + (1.0 - attached) * 0.12) as f32;
        force -= frame.x_axis * v.dot(frame.x_axis) * 3.2;
        if remaining > 0.0 {
            force = self.entry_acceleration.lerp(force, handoff as f32);
        }
        let mut slowing = self.time;
        if self.soft_takeoff {
            slowing -= 0.35 * (1.0 - (-self.time / 0.35).exp());
        }
        let limit = 235.0_f64.max(self.entry_speed - 70.0 * slowing);
        v = (v + force * dt as f32).clamp_length_max(limit as f32);
        self.velocity = v;
        self.position += v * dt as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_airborne_traces() {
        let cases: serde_json::Value = serde_json::from_str(&crate::oracle("flight")).unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mut flight = Flight::default();
            if name == "stall" {
                flight.velocity = Vec3::Z * 40.0;
                flight.entry_speed = 40.0;
            }
            for (tick, f) in case["frames"].as_array().unwrap().iter().enumerate() {
                let input: Input = serde_json::from_value(f["input"].clone()).unwrap();
                flight.step(input, 1.0 / 120.0);
                for (key, actual) in [
                    ("position", flight.position),
                    ("velocity", flight.velocity),
                    ("rates", flight.rates),
                ] {
                    let expected: Vec3 = serde_json::from_value(f[key].clone()).unwrap();
                    assert!(
                        actual.distance(expected) < 0.02,
                        "{name} tick {tick} {key}: {actual:?} != {expected:?}"
                    );
                }
                let expected: [[f32; 3]; 3] = serde_json::from_value(f["frame"].clone()).unwrap();
                assert!(
                    (flight.frame.z_axis - Mat3::from_cols_array_2d(&expected).z_axis).length()
                        < 0.0001,
                    "{name}:{tick} attitude"
                );
            }
        }
    }
}
