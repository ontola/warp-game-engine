//! Ground handling equations translated from SpaceRacer src/race.gd and flight.gd.
//! This kernel is not yet the complete race/weapon/airborne state machine.
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
pub const TOP_SPEED: f64 = 315.0;
pub const BOOST_SPEED: f64 = 465.0;
pub const STACK_BOOST_SPEED: f64 = 565.0;
pub const MAX_SPEED: f64 = 620.0;
pub const DT: f64 = 1.0 / 120.0;
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
pub fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
pub fn toward(a: f64, b: f64, d: f64) -> f64 {
    if (b - a).abs() <= d {
        b
    } else {
        a + (b - a).signum() * d
    }
}
fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Input {
    pub throttle: f64,
    pub steer: f64,
    pub strafe: f64,
    pub trim: f64,
    pub brake: f64,
    pub boost: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct HandlingGains {
    pub steering: f64,
    pub thrust: f64,
    pub lateral_grip: f64,
    pub yaw_assist: f64,
    pub strafe: f64,
}
impl Default for HandlingGains {
    fn default() -> Self {
        Self { steering: 1., thrust: 1., lateral_grip: 1., yaw_assist: 1., strafe: 1. }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Zone {
    #[default]
    None,
    Boost,
    Repair,
}
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub width: f64,
    pub curve: f64,
    pub crest: f64,
    pub slope: f64,
    pub rails: bool,
    pub shape_angle: f64,
    pub split_gap: f64,
    pub zone: Zone,
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            width: 500.0,
            curve: 0.0,
            crest: 0.0,
            slope: 0.0,
            rails: true,
            shape_angle: 0.0,
            split_gap: 0.0,
            zone: Zone::None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pilot {
    pub distance: f64,
    pub x: f64,
    pub speed: f64,
    pub heading: f64,
    pub slip: f64,
    pub energy: f64,
    pub boost: f64,
    pub trim: f64,
    pub slide: f64,
    pub slide_hold: f64,
    pub lift: f64,
    pub lift_speed: f64,
    pub unload: f64,
    pub acceleration: f64,
    pub flash: f64,
    pub energy_refill_delay: f64,
    pub energy_previous: f64,
    pub on_pad: bool,
    pub drifting: bool,
    pub airborne: bool,
    pub crashed: bool,
    pub lap: u32,
    pub slot: usize,
    pub route: f64,
    pub boost_held: bool,
    pub recovery: f64,
    pub launch_cooldown: f64,
    pub emp_time: f64,
    pub warp_time: f64,
    pub slipstream: f64,
    pub bump_velocity: f64,
    pub finished: bool,
    pub boost_cost: f64,
    pub boost_duration: f64,
}
impl Default for Pilot {
    fn default() -> Self {
        Self {
            distance: 0.0,
            x: 0.0,
            speed: 0.0,
            heading: 0.0,
            slip: 0.0,
            energy: 100.0,
            boost: 0.0,
            trim: 0.0,
            slide: 0.0,
            slide_hold: 0.0,
            lift: 0.0,
            lift_speed: 0.0,
            unload: 0.0,
            acceleration: 0.0,
            flash: 0.0,
            energy_refill_delay: 2.0,
            energy_previous: 100.0,
            on_pad: false,
            drifting: false,
            airborne: false,
            crashed: false,
            lap: 1,
            slot: 0,
            route: 0.0,
            boost_held: false,
            recovery: 0.0,
            launch_cooldown: 0.0,
            emp_time: 0.0,
            warp_time: 0.0,
            slipstream: 0.0,
            bump_velocity: 0.0,
            finished: false,
            boost_cost: 22.0,
            boost_duration: 1.25,
        }
    }
}
impl Pilot {
    pub fn lateral_limit(&self, width: f64) -> f64 {
        width - 4.7 * self.heading.cos().abs() - 5.1 * self.heading.sin().abs() - 0.2
    }
    pub fn constrain(&mut self, n: Surface) {
        if n.shape_angle > PI - 0.001 {
            self.x = (self.x + n.width).rem_euclid(n.width * 2.0) - n.width;
        } else if n.rails {
            let e = self.lateral_limit(n.width);
            self.x = self.x.clamp(-e, e);
        }
        if n.split_gap > 0.01 {
            if self.route == 0.0 {
                self.route = if self.x.abs() > 0.1 {
                    self.x.signum()
                } else if self.slot % 2 == 0 {
                    1.0
                } else {
                    -1.0
                };
            }
            let margin = (n.width - self.lateral_limit(n.width)) * smooth(0.0, 3.0, n.split_gap);
            if self.x * self.route < n.split_gap + margin {
                self.x = self.route * (n.split_gap + margin);
                if self.slip * self.route < 0.0 {
                    self.slip = -self.slip * 0.25;
                    self.heading = self.route * 0.04;
                }
            }
        } else {
            self.route = 0.0;
        }
    }
    /// Updates ground motion, returning the source's magnetic-release request.
    /// Caller handles route sampling, collisions, checkpoints and flight transition.
    pub fn ground_step(&mut self, c: Input, n: Surface, dt: f64) -> bool {
        self.ground_step_tuned(c, n, dt, HandlingGains::default())
    }
    pub fn ground_step_tuned(&mut self, c: Input, n: Surface, dt: f64, gains: HandlingGains) -> bool {
        self.flash = (self.flash - dt).max(0.0);
        self.launch_cooldown = (self.launch_cooldown - dt).max(0.0);
        let steer = c.steer.clamp(-1.0, 1.0);
        let throttle = if self.emp_time > 0.0 {
            0.0
        } else {
            c.throttle.clamp(0.0, 1.0)
        };
        let brake = c.brake.clamp(0.0, 1.0);
        let strafe = if self.emp_time > 0.0 {
            0.0
        } else {
            c.strafe.clamp(-1.0, 1.0)
        };
        self.trim = toward(self.trim, c.trim.clamp(-1.0, 1.0), dt * 5.0);
        let planted = self.trim.max(0.0);
        let loose = (-self.trim).max(0.0);
        let target_slide = brake.sqrt() * smooth(45.0, 140.0, self.speed) * (1.0 - planted * 0.55);
        if target_slide > self.slide {
            self.slide = toward(self.slide, target_slide, dt * 14.0);
            self.slide_hold = 0.18;
        } else if brake > 0.08 {
            self.slide = toward(self.slide, target_slide, dt * 4.0);
            self.slide_hold = 0.18;
        } else {
            self.slide_hold = (self.slide_hold - dt).max(0.0);
            if self.slide_hold == 0.0 || planted > 0.2 {
                let counter = if steer * self.heading < -0.015 {
                    0.6
                } else {
                    0.0
                };
                self.slide = toward(self.slide, 0.0, dt * (0.65 + planted * 2.8 + counter));
            }
        }
        self.drifting = self.slide > 0.18;
        if c.boost
            && !self.boost_held
            && self.lap > 1
            && self.energy > self.boost_cost
            && self.recovery == 0.0
            && brake < 0.05
            && !self.airborne
            && self.emp_time <= 0.0
        {
            self.energy -= self.boost_cost;
            self.boost = self.boost_duration;
        }
        self.boost_held = c.boost;
        self.boost = (self.boost - dt).max(0.0);
        self.on_pad = n.zone == Zone::Boost
            && self.x.abs() < n.width * 0.35
            && brake < 0.05
            && self.lift < 1.0
            && self.emp_time <= 0.0;
        let fast = (self.boost > 0.0 || self.on_pad) && brake < 0.05;
        let stacked = fast && self.boost > 0.0 && self.on_pad;
        let target = if stacked {
            STACK_BOOST_SPEED
        } else if fast {
            BOOST_SPEED
        } else {
            TOP_SPEED
        };
        let target = target + loose * 60.0 - planted * 35.0;
        let drag = 1.0 - self.slipstream * if fast { 0.12 } else { 0.26 };
        let mut accel =
            throttle * (1.0 - brake * 0.9) * 160.0 * gains.thrust * (1.0 - (self.speed / target).powi(2) * drag);
        if fast {
            accel += (target - self.speed).max(0.0) * if stacked { 4.2 } else { 3.0 };
        }
        if throttle == 0.0 {
            accel -= 38.0 * drag;
        }
        accel -= 240.0 * brake + planted * self.speed * 0.12;
        accel -= steer.abs() * self.speed * lerp(0.045, 0.08, self.slide)
            + n.slope * 28.0 * self.heading.cos();
        self.acceleration = accel.max(0.0);
        self.speed = (self.speed + accel * dt).clamp(0.0, MAX_SPEED);
        let instability = smooth(435.0, 605.0, self.speed) * (1.0 - planted.clamp(0.0, 1.0) * 0.72);
        let yaw = steer * gains.steering * lerp(1.65, 3.8, self.slide) * (1.0 + instability * 0.22);
        let assistance = gains.yaw_assist * smooth(35.0, 130.0, self.speed)
            * lerp(3.5, 0.8, self.slide)
            * (1.0 - instability * 0.45);
        let facing = wrap(self.heading);
        let side = strafe * 46.0 * gains.strafe + self.bump_velocity;
        let advance = self.speed * facing.cos() - side * facing.sin();
        self.heading = wrap(facing + (yaw - n.curve * advance - facing.sin() * assistance) * dt);
        let lateral = self.heading.sin() * self.speed + side * self.heading.cos();
        let forward = self.speed * self.heading.cos() - side * self.heading.sin();
        self.slip -= n.curve
            * forward
            * forward
            * lerp(0.25, 1.0, self.slide)
            * (1.0 + instability * 0.5)
            * dt;
        let grip = gains.lateral_grip * lerp(14.0, 1.8, self.slide)
            * (1.0 + planted * 0.75 - loose * 0.48)
            * (1.0 - self.unload * 0.35)
            * (1.0 - instability * 0.58);
        self.slip = lerp(self.slip, lateral, 1.0 - (-grip * dt).exp());
        let crest = (-n.crest).max(0.0) * forward * forward * (265.0 / TOP_SPEED).powi(2);
        let hold = 230.0 * (1.0 - loose * 0.82) + planted * 180.0 + brake * 80.0;
        let available = self.launch_cooldown == 0.0 && brake < 0.1 && self.speed > 165.0;
        let mut demand = smooth(0.55, 1.0, loose) * smooth(120.0, 220.0, self.speed);
        if loose > 0.35 && self.speed > 260.0 {
            demand = demand.max(((crest - hold) / 180.0).clamp(0.0, 1.0));
        }
        if !available {
            demand = 0.0;
        }
        self.unload = lerp(self.unload, demand, 1.0 - (-dt * 6.5).exp());
        let target = 3.6 * self.unload * self.unload;
        self.lift_speed += ((target - self.lift) * 64.0 - self.lift_speed * 16.0) * dt;
        self.lift = (self.lift + self.lift_speed * dt).max(0.0);
        if self.lift == 0.0 {
            self.lift_speed = self.lift_speed.max(0.0);
        }
        let launch = available && self.unload > 0.68 && self.lift > 1.35 && self.lift_speed > 0.0;
        self.x += self.slip * dt;
        let edge = self.lateral_limit(n.width);
        if n.rails && self.x.abs() > edge && self.lift < 2.5 {
            let side = self.x.signum();
            self.x = side * edge;
            if self.slip * side > 0.0 {
                let hit = self.slip.abs();
                self.energy = (self.energy - hit * 0.1 - 8.0 * dt).max(0.0);
                self.speed *= 1.0 - 0.22_f64.min(hit * 0.002);
                self.slip = -side * (hit * 0.4).min(40.0);
                self.heading = -side * 0.1;
                self.flash = 0.18;
            }
        }
        self.constrain(n);
        if n.zone == Zone::Repair
            && !self.airborne
            && !self.crashed
            && !self.finished
            && self.recovery <= 0.0
            && self.warp_time <= 0.0
            && self.energy > 0.0
            && self.energy < 100.0
            && self.lift < 1.0
            && self.x >= n.width * (-0.88)
            && self.x <= n.width * 0.30
        {
            self.energy = (self.energy + 40.0 * dt).min(100.0);
        }
        self.distance += (self.speed * self.heading.cos() - side * self.heading.sin()) * dt;
        launch
    }
    pub fn refill(&mut self, dt: f64, rate: f64) {
        if self.energy < self.energy_previous
            || self.boost > 0.0
            || self.warp_time > 0.0
            || self.emp_time > 0.0
            || self.crashed
            || self.recovery > 0.0
            || self.finished
        {
            self.energy_refill_delay = 2.0;
        } else {
            let t = (dt - self.energy_refill_delay).max(0.0);
            self.energy_refill_delay = (self.energy_refill_delay - dt).max(0.0);
            if self.energy > 0.0 {
                self.energy = (self.energy + 1.5 * rate * t).min(100.0);
            }
        }
        self.energy_previous = self.energy;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steering_gain_multiplies_turn_rate_even_at_full_input() {
        let mut normal = Pilot::default();
        let mut tuned = normal.clone();
        let input = Input { steer: 1., ..Input::default() };
        normal.ground_step(input, Surface::default(), DT);
        tuned.ground_step_tuned(input, Surface::default(), DT,
            HandlingGains { steering: 6., ..HandlingGains::default() });
        assert!((tuned.heading - 6. * normal.heading).abs() < 1e-12);
    }
    #[test]
    fn original_godot_control_traces() {
        let raw = crate::oracle("handling");
        let cases: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mut p = Pilot::default();
            if name != "accelerate" {
                p.speed = 265.0;
            }
            if name == "boost" || name == "stack_boost" {
                p.lap = 2;
            }
            if name == "recharge" {
                p.energy = 35.0;
            }
            let zone = match case["zone"].as_str().unwrap() {
                "boost" => Zone::Boost,
                "repair" => Zone::Repair,
                _ => Zone::None,
            };
            let surface = Surface {
                width: case["width"].as_f64().unwrap(),
                zone,
                ..Surface::default()
            };
            for (tick, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let c: Input = serde_json::from_value(frame["input"].clone()).unwrap();
                assert!(
                    !p.ground_step(c, surface, DT),
                    "unexpected launch {name}:{tick}"
                );
                p.constrain(surface);
                p.refill(DT, 1.0);
                let actual = serde_json::to_value(&p).unwrap();
                for (key, value) in frame["state"].as_object().unwrap() {
                    if let Some(expected) = value.as_f64() {
                        let got = actual[key].as_f64().unwrap();
                        assert!(
                            (got - expected).abs() < 0.00005,
                            "{name} tick {tick} {key}: {got} != {expected}"
                        );
                    } else {
                        assert_eq!(&actual[key], value, "{name} tick {tick} {key}");
                    }
                }
            }
        }
    }
}
