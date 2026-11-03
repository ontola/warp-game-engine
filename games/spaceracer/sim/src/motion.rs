//! Flight transitions and physical landing rules. World collision is supplied by the caller.
use crate::{
    flight::Flight,
    handling::{Input, Pilot},
    race_rules::{Pose, ground_pose},
    track::Track,
};
use glam::{Mat3, Vec3};
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub position: Vec3,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AirEvent {
    None,
    Landed { damage: f64 },
    Crashed { normal: Vec3 },
    LapGate { fraction: f64 },
}
#[derive(Clone, Debug)]
pub struct Motion {
    pub air: Flight,
    pub ground_velocity: Vec3,
    pub ground_acceleration: Vec3,
    pub landing_frame: Mat3,
    pub landing_blend: f64,
    pub air_travel: f64,
    pub crash_id: u64,
    pub crash_velocity: Vec3,
    pub crash_normal: Vec3,
}
impl Default for Motion {
    fn default() -> Self {
        Self {
            air: Flight::default(),
            ground_velocity: Vec3::ZERO,
            ground_acceleration: Vec3::ZERO,
            landing_frame: Mat3::IDENTITY,
            landing_blend: 0.0,
            air_travel: 0.0,
            crash_id: 0,
            crash_velocity: Vec3::ZERO,
            crash_normal: Vec3::ZERO,
        }
    }
}
pub fn touchdown_damage(frame: Mat3, surface: Mat3, velocity: Vec3) -> f64 {
    let impact = (-velocity.dot(surface.y_axis) as f64 - 18.0).max(0.0);
    let angle = (frame.y_axis.dot(surface.y_axis) as f64)
        .clamp(-1.0, 1.0)
        .acos()
        .max(
            (frame.z_axis.dot(surface.z_axis) as f64)
                .clamp(-1.0, 1.0)
                .acos(),
        );
    let tilt = (angle - 12.0_f64.to_radians()).max(0.0) / 40.0_f64.to_radians();
    let sideways = (velocity.dot(surface.x_axis).abs() as f64 - 18.0).max(0.0);
    (impact * 0.10 + tilt.powf(1.35) * 5.0 + sideways * 0.035).min(18.0)
}
impl Motion {
    pub fn pose(&self, p: &Pilot, track: &Track, clock: f64) -> Pose {
        if p.airborne || p.crashed {
            Pose {
                origin: self.air.position,
                basis: self.air.frame,
            }
        } else {
            ground_pose(
                p,
                &track.sample(p.distance),
                clock,
                Some((self.landing_frame, self.landing_blend as f32)),
            )
        }
    }
    pub fn launch(&mut self, p: &mut Pilot, pose: Pose) {
        p.airborne = true;
        self.air.time = 0.0;
        self.air_travel = 0.0;
        self.air.roll = 0.0;
        self.air.frame = pose.basis;
        self.air.position = pose.origin;
        self.air.velocity = self.ground_velocity;
        self.air.entry_speed = self.air.velocity.length() as f64;
        self.air.handoff = if p.unload > 0.35 { 0.65 } else { 0.0 };
        self.air.soft_takeoff = p.unload > 0.35;
        self.air.entry_rates = self.air.rates;
        self.air.entry_acceleration = self.ground_acceleration.clamp_length_max(180.0);
        p.on_pad = false;
        p.slide = 0.0;
        p.drifting = false;
    }
    pub fn crash(&mut self, p: &mut Pilot, normal: Vec3) -> AirEvent {
        if p.crashed {
            return AirEvent::None;
        }
        self.crash_velocity = if p.airborne {
            self.air.velocity
        } else {
            self.ground_velocity
        };
        self.crash_normal = normal;
        self.crash_id += 1;
        p.crashed = true;
        p.airborne = false;
        p.recovery = 0.0;
        p.speed = 0.0;
        p.boost = 0.0;
        p.on_pad = false;
        p.energy = (p.energy - 25.0).max(0.0);
        p.lift = 0.0;
        p.lift_speed = 0.0;
        p.unload = 0.0;
        p.acceleration = 0.0;
        p.emp_time = 0.0;
        AirEvent::Crashed { normal }
    }
    pub fn step_air(
        &mut self,
        p: &mut Pilot,
        track: &Track,
        input: Input,
        dt: f64,
        biome: &str,
        water_level: f32,
        mut trace: impl FnMut(Vec3, Vec3) -> Option<Contact>,
    ) -> AirEvent {
        let previous = self.air.position;
        self.air.emp = p.emp_time > 0.0;
        self.air.step(input, dt);
        p.speed = self.air.velocity.length() as f64;
        let position = self.air.position;
        if let Some(hit) = trace(previous, position) {
            self.air.position = hit.position;
            return self.crash(p, hit.normal);
        }
        let velocity = self.air.velocity;
        let frame = self.air.frame;
        self.air_travel += previous.distance(position) as f64;
        let contact = if self.air.time > 0.10 { track.sweep_road(previous, position) } else { None };
        if let Some(hit) = contact.filter(|h| !h.top) {
            self.air.position = previous.lerp(position,hit.t);
            return self.crash(p,hit.normal);
        }
        let (mut distance, mut n, mut x) = track.project(position, p.distance, track.length);
        if let Some(hit) = contact.filter(|h| ((h.distance-distance+track.length*0.5).rem_euclid(track.length)-track.length*0.5).abs()>track.step*2.) {
            distance = p.distance + (hit.distance-p.distance+track.length*0.5).rem_euclid(track.length)-track.length*0.5;
            n = track.sample(distance);
            x = n.lateral(position);
        }

        let surface = n.frame(x);
        let surface_position = n.point(x, 0.0);
        let after = (position - surface_position).dot(surface.y_axis) as f64 - 1.3;
        p.lift = after;
        p.lift_speed = velocity.dot(surface.y_axis) as f64;
        let inside = n.supported(x, 0.);
        let mut event = AirEvent::None;
        if let Some(hit) = contact {
            if !inside { self.air.position=previous.lerp(position,hit.t); return self.crash(p,hit.normal); }
            let nose = frame.z_axis.dot(surface.z_axis);
            let upright = frame.y_axis.dot(surface.y_axis);
            let approach = velocity.dot(surface.z_axis) as f64;
            let limit = if ["jump", "flight"].contains(&n.feature.as_str()) {
                140.0
            } else {
                115.0
            };
            if nose > 0.4 && upright > 0.4 && p.lift_speed > -limit && approach > 45.0 {
                let damage = touchdown_damage(frame, surface, velocity);
                p.energy = (p.energy - damage).max(0.0);
                p.flash = p.flash.max((damage * 0.015).min(0.4));
                if p.energy <= 0.0 {
                    return self.crash(p, surface.y_axis);
                }
                p.airborne = false;
                p.distance = distance;
                p.x = x;
                p.route = if n.split_gap > 0.01 {
                    p.x.signum()
                } else {
                    0.0
                };
                p.heading = (-frame.z_axis.dot(surface.x_axis) as f64)
                    .atan2(frame.z_axis.dot(surface.z_axis) as f64)
                    .clamp(-0.7, 0.7);
                p.slip = (-velocity.dot(surface.x_axis) as f64).clamp(-70.0, 70.0);
                p.speed = (approach / p.heading.cos().max(0.55)).clamp(65.0, 440.0)
                    * (1.0 - damage * 0.002);
                p.lift = after.max(0.0);
                p.lift_speed = 0.0;
                p.unload = 0.0;
                self.landing_frame = frame;
                self.landing_blend = 1.0;
                p.launch_cooldown = 0.7;
                event = AirEvent::Landed { damage };
            } else {
                return self.crash(p, surface.y_axis);
            }
        } else if (biome == "city"
            && position.x.abs() < 6000.0
            && position.z.abs() < 6000.0
            && position.y < -179.5)
            || (biome == "forest" && position.y <= water_level + 0.6)
        {
            return self.crash(p, Vec3::ZERO);
        }
        if p.airborne && !p.crashed {
            let distance = p.lap as f64 * track.length;
            let gate = track.sample(distance);
            let basis = gate.geometry.basis();
            let before = (previous - gate.geometry.p).dot(basis.z_axis);
            let after = (position - gate.geometry.p).dot(basis.z_axis);
            if before < 0.0 && after >= 0.0 {
                let fraction = (-before / (after - before)).clamp(0.0, 1.0);
                let crossing = previous.lerp(position, fraction) - gate.geometry.p;
                let height = crossing.dot(basis.y_axis);
                if crossing.dot(basis.x_axis).abs() as f64 <= gate.geometry.width + 5.0
                    && height >= 0.0
                    && height <= 120.0
                {
                    p.distance = distance + after.max(0.0) as f64;
                    self.air_travel = position.distance(previous.lerp(position, fraction)) as f64;
                    return AirEvent::LapGate {
                        fraction: fraction as f64,
                    };
                }
            }
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_landing_and_underside_crashes() {
        let tracks: serde_json::Value = serde_json::from_str(&crate::oracle("tracks")).unwrap();
        let track: Track = serde_json::from_value(tracks[0].clone()).unwrap();
        let cases: serde_json::Value = serde_json::from_str(&crate::oracle("landing")).unwrap();
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let mut p = Pilot {
                distance: 1000.0,
                airborne: true,
                ..Default::default()
            };
            let mut m = Motion::default();
            m.air.position = serde_json::from_value(c["initial"]["position"].clone()).unwrap();
            m.air.velocity = serde_json::from_value(c["initial"]["velocity"].clone()).unwrap();
            m.air.frame = Mat3::from_cols_array_2d(
                &serde_json::from_value(c["initial"]["frame"].clone()).unwrap(),
            );
            m.air.entry_speed = m.air.velocity.length() as f64;
            for (tick, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                m.step_air(
                    &mut p,
                    &track,
                    Input {
                        throttle: 1.0,
                        ..Default::default()
                    },
                    1.0 / 120.0,
                    "city",
                    f32::NEG_INFINITY,
                    |_, _| None,
                );
                assert_eq!(
                    p.airborne,
                    f["airborne"].as_bool().unwrap(),
                    "{name} tick {tick} airborne"
                );
                assert_eq!(
                    p.crashed,
                    f["crashed"].as_bool().unwrap(),
                    "{name} tick {tick} crash"
                );
                let expected: Vec3 = serde_json::from_value(f["position"].clone()).unwrap();
                // Swept impacts stop at the first contact rather than the old
                // end-of-tick penetration. Free-flight/landing traces stay exact.
                let tolerance=if p.crashed {m.crash_velocity.length()/120.+0.02}else{0.02};
                assert!(m.air.position.distance(expected)<tolerance,"{name} tick {tick} position");
                for (key, v) in [
                    ("energy", p.energy),
                    ("speed", p.speed),
                    ("distance", p.distance),
                    ("x", p.x),
                    ("lift", p.lift),
                ] {
                    assert!(
                        (v - f[key].as_f64().unwrap()).abs() < 0.03,
                        "{name} tick {tick} {key} {v} expected {}",
                        f[key]
                    );
                }
            }
        }
    }
}
