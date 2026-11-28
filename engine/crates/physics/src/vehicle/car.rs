//! Four-wheel vehicle dynamics: a rigid body with our own forces. Per wheel a
//! cylinder cast finds the ground; spring, damper, bump stop and anti-roll
//! bar give the load; a combined-slip Pacejka tyre with relaxation-length
//! slip states gives the force. Engine, automatic clutch, sequential box,
//! centre coupling (opened by the handbrake) and plated diffs run four
//! substeps per tick, couplings solved implicitly.

use super::{GROUP_WORLD, ROAD, CarSpec};
use super::surfaces::{Surface, Surfaces};
use super::tyre::{ALPHA_PEAK, KAPPA_PEAK, Scale};
use rapier3d::glamx::{Pose3, Quat, Vec3};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::parry::shape::Cylinder;
use rapier3d::prelude::*;
use serde::Serialize;
use std::f64::consts::{PI, TAU};

pub const SUBSTEPS: usize = 4;
const V_LOW: f64 = 1.2;
const RELAX: f64 = 0.25;
const RPM: f64 = 60.0 / TAU;
const OVER_BUMP: f64 = 0.25;
/// ABS thresholds around the reference tyre's longitudinal peak (slip 0.144).
const ABS_RELEASE: f64 = 0.17;
const ABS_APPLY: f64 = 0.11;
const ABS_DUMP_TIME: f64 = 0.04;
const ABS_BUILD_TIME: f64 = 0.15;
const ABS_MIN_SPEED: f64 = 1.5;
/// Traction control: on above 20% driving slip, off below 10%, measured
/// against at least 3 m/s so a car can pull away.
const TCS_ON: f64 = 0.2;
const TCS_OFF: f64 = 0.1;
const TCS_BUILD_TIME: f64 = 0.1;
const TCS_MIN_SPEED: f64 = 3.0;
// ESC (yaw control as in Bosch ESP, van Zanten 2000, SAE 2000-01-1633): the
// reference yaw rate of a linear car with this characteristic speed (the
// compact's 1.2 deg/g understeer gives 34 m/s), limited by the friction.
const ESC_CHAR_SPEED: f64 = 34.0;
const ESC_MIN_SPEED: f64 = 5.0;
/// Yaw-rate error (rad/s) before ESC brakes, plus a share of the reference.
const ESC_DEADBAND: f64 = 0.05;
const ESC_DEADBAND_SHARE: f64 = 0.1;
/// Yaw moment (Nm) per rad/s of error past the deadband.
const ESC_GAIN: f64 = 15000.0;
/// Time for the pump to build or drop full brake pressure.
const ESC_BUILD_TIME: f64 = 0.15;

/// What a car needs to know about the world beyond its colliders.
pub trait Ground {
    /// Surface of the road mesh at `p`, with a hint for the next lookup.
    fn road_surface(&self, p: Vec3, hint: i64) -> (Surface, i64);
    /// Water surface height at (x, z), -inf where dry.
    fn water_level(&self, x: f64, z: f64) -> f64;
}

#[derive(Clone, Debug, Serialize)]
pub struct Wheel {
    #[serde(skip)]
    pub anchor: Vec3,
    pub axle: usize,
    pub side: f64,
    pub radius: f64,
    pub inertia: f64,
    pub omega: f64,
    pub spin: f64,
    pub steer: f64,
    pub compression: f64,
    pub last_compression: f64,
    pub load: f64,
    pub kappa: f64,
    pub tan_alpha: f64,
    pub contact: bool,
    #[serde(skip)]
    pub point: Vec3,
    #[serde(skip)]
    pub normal: Vec3,
    pub surface: Surface,
    pub road_hint: i64,
    pub fx: f64,
    pub fy: f64,
    pub vx: f64,
    pub vy: f64,
    #[serde(skip)]
    pub wf: Vec3,
    #[serde(skip)]
    pub wr: Vec3,
    pub slide: f64,
    /// ABS: the share of brake pressure the modulator lets through.
    pub abs: f64,
    /// Traction control: brake torque (Nm) on a spinning driven wheel.
    pub tcs: f64,
    /// Compliance and roll steer (rad), added to `steer`.
    pub toe: f64,
    /// ESC: brake torque (Nm) the stability control puts on this wheel.
    pub esc: f64,
}

/// Engine and contact-patch signals for road sounds. Slip is the sum of
/// longitudinal and transverse sliding speeds over wheels touching ground.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoadTelemetry {
    pub load: f32,
    pub slip: f32,
    pub contact: f32,
    pub loose: f32,
    pub soft: f32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CarStats {
    pub shifts: u32,
    pub air_time: f64,
    pub max_slip_angle: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Car {
    #[serde(skip)]
    pub spec: CarSpec,
    #[serde(skip)]
    pub body: RigidBodyHandle,
    pub wheels: Vec<Wheel>,
    // Driver inputs.
    pub throttle: f64,
    pub brake: f64,
    pub steer_input: f64,
    pub handbrake: f64,
    pub manual: bool,
    pub steer_assist: bool,
    pub hold: bool,
    // State.
    pub gear: i32,
    pub engine_omega: f64,
    pub shift_timer: f64,
    pub shift_target: i32,
    pub speed: f64,
    pub on_ground: u32,
    pub wading: f64,
    pub grip_scale: f64,
    pub stats: CarStats,
    reverse_hold: f64,
    shift_request: i32,
    #[serde(skip)]
    teleport: Option<Pose3>,
    engine_torque: f64,
    esc_cut: f64,
}

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x as f32, y as f32, z as f32)
}

impl Car {
    pub fn new(spec: CarSpec, body: RigidBodyHandle) -> Car {
        let half_wb = spec.wheelbase * 0.5;
        let mut wheels = Vec::new();
        for axle in 0..2 {
            let half_tr = if axle == 0 { spec.track * 0.5 } else { spec.track_rear() * 0.5 };
            for side in [-1.0, 1.0] {
                wheels.push(Wheel {
                    anchor: v(side * half_tr, spec.droop_y, if axle == 0 { -half_wb } else { half_wb }),
                    axle,
                    side,
                    radius: spec.wheel_radius,
                    inertia: spec.wheel_inertia,
                    omega: 0.0,
                    spin: 0.0,
                    steer: 0.0,
                    compression: 0.0,
                    last_compression: 0.0,
                    load: 0.0,
                    kappa: 0.0,
                    tan_alpha: 0.0,
                    contact: false,
                    point: Vec3::ZERO,
                    normal: Vec3::Y,
                    surface: Surface::Gravel,
                    road_hint: -1,
                    fx: 0.0,
                    fy: 0.0,
                    vx: 0.0,
                    vy: 0.0,
                    wf: Vec3::NEG_Z,
                    wr: Vec3::X,
                    slide: 0.0,
                    abs: 1.0,
                    tcs: 0.0,
                    toe: 0.0,
                    esc: 0.0,
                });
            }
        }
        let engine_omega = spec.idle_rpm / RPM;
        let grip_scale = spec.tyre_grip_scale;
        Car {
            spec,
            body,
            wheels,
            throttle: 0.0,
            brake: 0.0,
            steer_input: 0.0,
            handbrake: 0.0,
            manual: false,
            steer_assist: true,
            hold: false,
            gear: 1,
            engine_omega,
            shift_timer: 0.0,
            shift_target: 1,
            speed: 0.0,
            on_ground: 0,
            wading: 0.0,
            grip_scale,
            stats: CarStats::default(),
            reverse_hold: 0.0,
            shift_request: 0,
            teleport: None,
            engine_torque: 0.0,
            esc_cut: 0.0,
        }
    }

    /// Put the car down at `pose`, at rest, before the next step.
    pub fn teleport(&mut self, phys: &mut PhysicsWorld, pose: Pose3) {
        self.teleport = Some(pose);
        if let Some(b) = phys.bodies.get_mut(self.body) {
            b.set_position(pose, true);
            b.set_linvel(Vec3::ZERO, true);
            b.set_angvel(Vec3::ZERO, true);
        }
        for w in &mut self.wheels {
            w.omega = 0.0;
            w.kappa = 0.0;
            w.tan_alpha = 0.0;
            w.road_hint = -1;
        }
        self.gear = 1;
        self.shift_timer = 0.0;
    }

    pub fn request_shift(&mut self, dir: i32) {
        self.shift_request = dir;
    }

    pub fn rpm(&self) -> f64 {
        self.engine_omega * RPM
    }

    /// Net engine torque (Nm) of the last substep: combustion minus friction.
    pub fn engine_torque(&self) -> f64 {
        self.engine_torque
    }

    pub fn road_telemetry(&self) -> RoadTelemetry {
        let peak = self.spec.torque.iter().map(|p| p[1]).fold(1.0f64, f64::max);
        let mut t = RoadTelemetry { load: (self.engine_torque() / peak) as f32, ..Default::default() };
        let mut on = 0.0;
        for w in &self.wheels {
            if !w.contact { continue; }
            on += 1.0;
            t.slip += ((w.omega * w.radius - w.vx).abs() + w.vy.abs()) as f32;
            match w.surface {
                Surface::Asphalt => {}
                Surface::Grass | Surface::Water => t.soft += 1.0,
                _ => t.loose += 1.0,
            }
        }
        if on > 0.0 {
            t.loose /= on;
            t.soft /= on;
        }
        t.contact = on / self.wheels.len().max(1) as f32;
        t
    }

    /// Ignition cut: mid-shift, or on the limiter.
    pub fn cut(&self) -> bool {
        self.shift_timer > 0.0 || self.rpm() > self.spec.limit_rpm
    }

    pub fn pose(&self, phys: &PhysicsWorld) -> Pose3 {
        *phys.bodies[self.body].position()
    }

    pub fn linvel(&self, phys: &PhysicsWorld) -> Vec3 {
        phys.bodies[self.body].linvel()
    }

    pub fn angvel(&self, phys: &PhysicsWorld) -> Vec3 {
        phys.bodies[self.body].angvel()
    }

    /// `_integrate_forces`: everything this car does to its body this tick.
    pub fn integrate(&mut self, phys: &mut PhysicsWorld, ground: &dyn Ground, surfaces: &Surfaces, dt: f64) {
        if let Some(pose) = self.teleport.take() {
            let b = &mut phys.bodies[self.body];
            b.set_position(pose, true);
            b.set_linvel(Vec3::ZERO, true);
            b.set_angvel(Vec3::ZERO, true);
            b.reset_forces(true);
            b.reset_torques(true);
            return;
        }
        let (xf, lin, ang, com) = {
            let b = &phys.bodies[self.body];
            (*b.position(), b.linvel(), b.angvel(), b.center_of_mass())
        };
        let up = xf.rotation * Vec3::Y;
        let fwd = xf.rotation * Vec3::NEG_Z;
        let right_axis = xf.rotation * Vec3::X;
        self.speed = lin.dot(fwd) as f64;
        let front_mid = xf.transform_point((self.wheels[0].anchor + self.wheels[1].anchor) * 0.5);
        self.steering(dt, lin, lin + ang.cross(front_mid - com), right_axis, fwd);
        self.gearbox(dt);
        let water = ground.water_level(xf.translation.x as f64, xf.translation.z as f64);
        let mut force = Vec3::ZERO;
        let mut torque = Vec3::ZERO;
        let mut apply = |f: Vec3, at: Vec3| {
            force += f;
            torque += (at - com).cross(f);
        };
        self.suspension(phys, ground, &xf, up, dt, water, &mut apply);
        self.compliance();
        self.stability(dt, lin, ang, up, fwd, surfaces);

        for w in self.wheels.iter_mut() {
            if !w.contact {
                continue;
            }
            let mut wf = Quat::from_axis_angle(up, (w.steer + w.toe) as f32) * fwd;
            wf = (wf - w.normal * wf.dot(w.normal)).normalize_or_zero();
            let wr = wf.cross(w.normal);
            let vp = lin + ang.cross(w.point - com);
            w.vx = vp.dot(wf) as f64;
            w.vy = vp.dot(wr) as f64;
            w.wf = wf;
            w.wr = wr;
        }

        let h = dt / SUBSTEPS as f64;
        let mut sum_fx = [0.0f64; 4];
        let mut sum_fy = [0.0f64; 4];
        for _ in 0..SUBSTEPS {
            for i in 0..4 {
                let (fx, fy) = tyre(&self.wheels[i], surfaces, self.grip_scale, self.spec.lateral_peak_scale);
                self.wheels[i].fx = fx;
                self.wheels[i].fy = fy;
                sum_fx[i] += fx;
                sum_fy[i] += fy;
            }
            self.drivetrain(h, surfaces);
            for w in self.wheels.iter_mut() {
                let denom = w.vx.abs() + V_LOW;
                if w.contact {
                    w.kappa += ((w.omega * w.radius - w.vx) - denom * w.kappa) * h / RELAX;
                    w.tan_alpha += (w.vy - denom * w.tan_alpha) * h / RELAX;
                } else {
                    w.kappa = move_toward(w.kappa, 0.0, h * 8.0);
                    w.tan_alpha = move_toward(w.tan_alpha, 0.0, h * 8.0);
                }
            }
        }

        self.on_ground = 0;
        let speed_len = lin.length() as f64;
        let vdir = lin.normalize_or_zero();
        for i in 0..4 {
            let w = &mut self.wheels[i];
            w.spin = fposmod(w.spin + w.omega * dt, TAU);
            if !w.contact {
                w.slide = move_toward(w.slide, 0.0, dt * 4.0);
                continue;
            }
            self.on_ground += 1;
            let f = w.wf * (sum_fx[i] / SUBSTEPS as f64) as f32 + w.wr * (sum_fy[i] / SUBSTEPS as f64) as f32;
            apply(f, w.point);
            let surf = surfaces.get(w.surface);
            w.slide = slide(w, self.spec.lateral_peak_scale);
            // Grass and dirt drag at the car a little.
            let drag = surf.drag * speed_len.min(10.0) * 0.1 * w.load / 3000.0;
            apply(-vdir * drag as f32, w.point);
        }
        // Wading: water pushes back on the body.
        self.wading = 0.0;
        let bottom = xf.translation.y as f64 - self.spec.wheel_radius;
        if water > bottom {
            self.wading = water - bottom;
            let hv = Vec3::new(lin.x, 0.0, lin.z);
            let d = self.wading.min(0.6) as f32;
            force += -hv * hv.length() * 420.0 * d - hv * 900.0 * d;
        }
        if self.on_ground == 0 {
            self.stats.air_time += dt;
        }
        // Air.
        force += -lin * lin.length() * self.spec.drag as f32 - up * (self.spec.downforce as f32 * lin.length_squared());
        if speed_len > 5.0 && self.on_ground > 0 {
            let slip_angle = (lin.dot(right_axis) as f64).atan2(self.speed.abs()).abs();
            self.stats.max_slip_angle = self.stats.max_slip_angle.max(slip_angle.to_degrees());
        }
        let b = &mut phys.bodies[self.body];
        b.reset_forces(true);
        b.reset_torques(true);
        b.add_force(force, true);
        b.add_torque(torque, true);
    }

    fn suspension(&mut self, phys: &PhysicsWorld, ground: &dyn Ground, xf: &Pose3, up: Vec3, dt: f64, level: f64, apply: &mut impl FnMut(Vec3, Vec3)) {
        let travel = self.spec.travel;
        let reach = travel + OVER_BUMP;
        let shape = Cylinder::new(0.1, self.spec.wheel_radius as f32);
        let filter = QueryFilter::default()
            .exclude_rigid_body(self.body)
            .groups(InteractionGroups::new(Group::ALL, GROUP_WORLD, InteractionTestMode::And));
        let qp = phys.query_pipeline_with_filter(filter);
        let lay = Quat::from_axis_angle(Vec3::Z, (PI / 2.0) as f32);
        for w in self.wheels.iter_mut() {
            let top = xf.transform_point(w.anchor + Vec3::Y * reach as f32);
            let rot = Quat::from_axis_angle(up, w.steer as f32) * xf.rotation * lay;
            let pose = Pose3::from_parts(top, rot);
            let motion = -up * reach as f32;
            w.last_compression = w.compression;
            let opts = ShapeCastOptions { max_time_of_impact: 1.0, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: true };
            let found = qp.cast_shape(&pose, motion, &shape, opts).map(|(h, hit)| (h, hit.time_of_impact, hit.witness1, hit.normal1));
            let Some((handle, toi, point, n)) = found else {
                w.contact = false;
                w.compression = 0.0;
                w.load = 0.0;
                continue;
            };
            w.contact = true;
            let t = toi.max(0.0) as f64;
            w.compression = (reach * (1.0 - t)).clamp(0.0, reach);
            w.point = point;
            w.normal = if n.dot(up) > 0.5 { n } else { up };
            let tag = phys.colliders[handle].user_data;
            w.surface = if level > w.point.y as f64 + 0.08 {
                Surface::Water
            } else if tag == ROAD {
                let (s, hint) = ground.road_surface(w.point, w.road_hint);
                w.road_hint = hint;
                s
            } else {
                Surface::from_u8(tag as u8)
            };
        }
        for axle in 0..2 {
            let (l, r) = (axle * 2, axle * 2 + 1);
            let roll = (self.wheels[l].compression - self.wheels[r].compression) * self.spec.arb[axle];
            for (k, sign) in [(l, 1.0), (r, -1.0)] {
                let w = &mut self.wheels[k];
                if !w.contact {
                    continue;
                }
                let rate = ((w.compression - w.last_compression) / dt).clamp(-4.0, 4.0);
                let damper = if rate > 0.0 { self.spec.bump[axle] } else { self.spec.rebound[axle] } * rate;
                let mut force = self.spec.spring[axle] * w.compression + damper + roll * sign;
                let stop = travel * 0.88;
                if w.compression > stop {
                    force += (w.compression - stop) * self.spec.spring[axle] * 14.0;
                }
                w.load = force.clamp(0.0, 24000.0);
                apply(up * w.load as f32, w.point);
            }
        }
    }

    fn drivetrain(&mut self, h: f64, surfaces: &Surfaces) {
        // Wheel spin as a traction control would measure it: each wheel's
        // speed at the start of the substep against the ground speed at that
        // wheel (from the car's speed and yaw rate, as real systems estimate
        // it), so the outer wheels of a tight turn do not count as spinning.
        let spin: [f64; 4] = std::array::from_fn(|i| {
            let w = &self.wheels[i];
            (w.omega * w.radius - w.vx) / w.vx.abs().max(TCS_MIN_SPEED)
        });
        for w in self.wheels.iter_mut() {
            w.omega += -w.fx * w.radius * h / w.inertia;
        }
        let spec = &self.spec;
        let rpm = self.engine_omega * RPM;
        let cut = self.shift_timer > 0.0 || rpm > spec.limit_rpm;
        let mut t = self.throttle;
        if self.gear < 0 && !self.manual {
            t = self.brake;
        }
        let idle_assist = ((spec.idle_rpm - rpm) / 250.0).clamp(0.0, 0.35);
        t = t.max(idle_assist);
        if cut {
            t = 0.0;
        }
        // ESC takes engine torque away while it brakes a wheel.
        t *= 1.0 - self.esc_cut;
        self.engine_torque = torque_curve(&spec.torque, rpm) * t - (spec.friction[0] + spec.friction[1] * rpm) * (1.0 - t);
        self.engine_omega += self.engine_torque * h / spec.engine_inertia;
        self.engine_omega = self.engine_omega.max(300.0 / RPM);

        let centre_open = self.handbrake > 0.1;
        let shares: [f64; 4] = if centre_open {
            [0.5, 0.5, 0.0, 0.0]
        } else {
            [spec.drive[0] * 0.5, spec.drive[0] * 0.5, spec.drive[1] * 0.5, spec.drive[1] * 0.5]
        };
        let mut axle_torque = [0.0f64; 2];
        if self.shift_timer <= 0.0 && !self.hold {
            let mut g = if self.gear < 0 { spec.gears[0] * -1.0 } else { spec.gears[self.gear as usize] };
            g *= spec.final_drive;
            let mut w_t = 0.0;
            let mut k = 0.0;
            for i in 0..4 {
                w_t += shares[i] * self.wheels[i].omega;
                k += shares[i] * shares[i] / self.wheels[i].inertia;
            }
            k *= g * g;
            let slip = self.engine_omega - g * w_t;
            let rpm = self.engine_omega * RPM;
            let mut engage = smoothstep(spec.idle_rpm + 150.0, 3600.0, rpm).max(smoothstep(spec.idle_rpm + 300.0, 2600.0, (g * w_t).abs() * RPM));
            if slip.abs() < 30.0 && rpm > spec.idle_rpm + 600.0 {
                engage = 1.0;
            }
            let cap = spec.clutch * engage;
            let tau = (slip / (h * (1.0 / spec.engine_inertia + k))).clamp(-cap, cap);
            self.engine_omega -= tau * h / spec.engine_inertia;
            for i in 0..4 {
                let wt = shares[i] * tau * g;
                self.wheels[i].omega += wt * h / self.wheels[i].inertia;
                axle_torque[i / 2] += wt.abs();
            }
        }
        if !centre_open {
            let front = (self.wheels[0].omega + self.wheels[1].omega) * 0.5;
            let rear = (self.wheels[2].omega + self.wheels[3].omega) * 0.5;
            let i_w = self.wheels[0].inertia;
            let tau_c = ((front - rear) * i_w / h).clamp(-spec.centre_lock, spec.centre_lock);
            for i in 0..4 {
                self.wheels[i].omega += if i < 2 { -0.5 } else { 0.5 } * tau_c * h / self.wheels[i].inertia;
            }
        }
        for axle in 0..2 {
            let cap_d = spec.diff_preload[axle] + spec.diff_ramp[axle] * axle_torque[axle];
            let (li, ri) = (self.wheels[axle * 2].inertia, self.wheels[axle * 2 + 1].inertia);
            let tau_d = ((self.wheels[axle * 2].omega - self.wheels[axle * 2 + 1].omega) / (h * (1.0 / li + 1.0 / ri))).clamp(-cap_d, cap_d);
            self.wheels[axle * 2].omega -= tau_d * h / li;
            self.wheels[axle * 2 + 1].omega += tau_d * h / ri;
        }
        let mut foot = if self.gear > 0 || self.manual { self.brake } else { self.throttle };
        if self.hold {
            foot = 1.0;
        }
        // EBD, part of every ABS: the rear wheels may not slip more than the
        // fronts under braking, so the rear never locks first.
        let front_slip = self.wheels[..2].iter().map(|w| -w.kappa * w.vx.signum()).fold(0.0f64, f64::max);
        for (w, spin) in self.wheels.iter_mut().zip(spin) {
            let release = if w.axle == 1 { ABS_RELEASE.min(front_slip.max(ABS_APPLY + 0.02)) } else { ABS_RELEASE };
            // ABS (road cars): drop pressure when the wheel's braking slip
            // passes the tyre's peak, build it again below it. A pump cycle
            // of tens of milliseconds, as on real modulators.
            let pedal = foot * spec.brake[w.axle] + w.esc;
            if spec.abs && pedal > 0.0 && w.contact && w.vx.abs() > ABS_MIN_SPEED {
                let braking_slip = -w.kappa * w.vx.signum();
                if braking_slip > release {
                    w.abs = (w.abs - h / ABS_DUMP_TIME).max(0.0);
                } else if braking_slip < ABS_APPLY {
                    w.abs = (w.abs + h / ABS_BUILD_TIME).min(1.0);
                }
            } else {
                w.abs = 1.0;
            }
            let mut t_b = pedal * w.abs;
            // Traction control (road cars, part of ESC): brake a driven wheel
            // that spins well past the car's speed, e.g. lifted off the ground
            // behind an open differential, so the other wheel gets the torque.
            if spec.traction_control && spec.drive[w.axle] > 0.0 {
                if spin > TCS_ON {
                    w.tcs = (w.tcs + spec.brake[w.axle] * h / TCS_BUILD_TIME).min(spec.brake[w.axle]);
                } else if spin < TCS_OFF {
                    w.tcs = (w.tcs - spec.brake[w.axle] * h / TCS_BUILD_TIME).max(0.0);
                }
                t_b += w.tcs;
            } else {
                w.tcs = 0.0;
            }
            if w.axle == 1 {
                t_b += self.handbrake * spec.handbrake;
            }
            if w.contact {
                t_b += surfaces.get(w.surface).rolling * w.load * w.radius;
            }
            let stop = t_b * h / w.inertia;
            w.omega = if w.omega.abs() <= stop { 0.0 } else { w.omega - w.omega.signum() * stop };
        }
    }

    /// Toe change from the suspension's kinematics and compliance (K&C): the
    /// lateral force and aligning torque bend the bushings and steering, and
    /// body roll steers the wheels. Coefficients are per wheel in the K&C
    /// convention, positive toward understeer: a front wheel turns out of
    /// the corner, a rear wheel into it. Uses the forces of the last step.
    fn compliance(&mut self) {
        let s = &self.spec;
        for axle in 0..2 {
            let (l, r) = (axle * 2, axle * 2 + 1);
            let track = if axle == 0 { s.track } else { s.track_rear() };
            let both = self.wheels[l].contact && self.wheels[r].contact;
            // Body roll (rad), positive when the right side is compressed
            // (a left-hand corner).
            let roll = if both { (self.wheels[r].compression - self.wheels[l].compression) / track } else { 0.0 };
            // Positive toe along the tyre's lateral force aligns the wheel
            // with its motion (understeer at the front, oversteer at the rear).
            let align = if axle == 0 { 1.0 } else { -1.0 };
            for k in [l, r] {
                let w = &mut self.wheels[k];
                if !w.contact {
                    w.toe = 0.0;
                    continue;
                }
                let mz = super::tyre::trail(w.tan_alpha.atan(), w.load) * w.fy;
                let deg = s.force_steer[axle] * w.fy / 1000.0 + s.torque_steer[axle] * mz / 100.0 - s.roll_steer[axle] * roll.to_degrees();
                w.toe = align * deg.to_radians();
            }
        }
    }

    /// ESC: compare the yaw rate with what the driver's steering asks of a
    /// linear car, and brake one wheel to make up the difference: the outer
    /// front when the car turns more than asked (oversteer), the inner rear
    /// when it turns less (understeer), cutting engine torque meanwhile.
    fn stability(&mut self, dt: f64, lin: Vec3, ang: Vec3, up: Vec3, fwd: Vec3, surfaces: &Surfaces) {
        let v = lin.dot(fwd) as f64;
        let mut target = [0.0f64; 4];
        let on_ground = self.wheels.iter().filter(|w| w.contact).count();
        if self.spec.esc && v > ESC_MIN_SPEED && on_ground >= 3 && self.handbrake < 0.1 && self.gear >= 0 {
            let l = self.spec.wheelbase;
            let delta = self.wheels[0].steer;
            let mu = self.wheels.iter().filter(|w| w.contact).map(|w| surfaces.get(w.surface).mu).sum::<f64>() / on_ground as f64;
            // Friction limit: a peak of about 1.1 x the surface's share of dry asphalt.
            let r_max = 1.1 * mu * 9.81 / v;
            let r_ref = (v * delta / (l * (1.0 + (v / ESC_CHAR_SPEED).powi(2)))).clamp(-r_max, r_max);
            let r = ang.dot(up) as f64;
            let err = r - r_ref;
            let band = ESC_DEADBAND + ESC_DEADBAND_SHARE * r_ref.abs();
            if err.abs() > band {
                // Yaw moment wanted, positive to the left (about up).
                let m = -(err - band * err.signum()) * ESC_GAIN;
                let oversteer = r.abs() > r_ref.abs() || r * r_ref < 0.0;
                let axle = if oversteer { 0 } else { 1 };
                // Braking a left wheel (index 0 or 2) yaws the car left.
                let k = axle * 2 + if m > 0.0 { 0 } else { 1 };
                let track = if axle == 0 { self.spec.track } else { self.spec.track_rear() };
                let w = &self.wheels[k];
                target[k] = (m.abs() / (0.5 * track) * w.radius).min(self.spec.brake[axle]);
            }
        }
        let mut cut = 0.0f64;
        for (i, w) in self.wheels.iter_mut().enumerate() {
            let step = self.spec.brake[w.axle] * dt / ESC_BUILD_TIME;
            w.esc = move_toward(w.esc, target[i], step);
            cut = cut.max(w.esc / self.spec.brake[w.axle]);
        }
        self.esc_cut = cut;
    }

    /// Steering assist: like a driver at speed, steer the front wheels no
    /// further than the tyre's peak slip angle past the direction the front
    /// axle is actually moving (so a sliding car can still be caught).
    /// `front_vel` is the velocity at the middle of the front axle.
    fn steering(&mut self, dt: f64, lin: Vec3, front_vel: Vec3, right_axis: Vec3, fwd: Vec3) {
        let mut lock = self.spec.max_steer();
        let ground_speed = lin.length() as f64;
        let ahead = front_vel.dot(fwd) as f64;
        if self.steer_assist && ground_speed > 6.0 && ahead > 0.0 {
            // Front-axle velocity angle, positive to the right of the nose.
            let theta = (front_vel.dot(right_axis) as f64).atan2(ahead);
            let side = if self.steer_input >= 0.0 { theta } else { -theta };
            let peak = ALPHA_PEAK * self.spec.lateral_peak_scale;
            let useful = (side + peak).max(peak);
            lock += (lock.min(useful) - lock) * ((ground_speed - 6.0) / 22.0).clamp(0.0, 1.0);
        }
        let target = -self.steer_input * lock;
        let current = self.wheels[0].steer;
        let rate = if target.abs() > current.abs() { 3.2 } else { 4.5 };
        let next = move_toward(current, target, rate * dt);
        for w in self.wheels.iter_mut() {
            w.steer = if w.axle == 0 { next } else { 0.0 };
        }
    }

    fn gearbox(&mut self, dt: f64) {
        if self.hold {
            self.gear = 1;
            self.shift_timer = 0.0;
            self.shift_request = 0;
            return;
        }
        if self.shift_timer > 0.0 {
            self.shift_timer -= dt;
            if self.shift_timer <= 0.0 {
                self.gear = self.shift_target;
            }
            return;
        }
        let top = self.spec.gears.len() as i32 - 1;
        if self.shift_request != 0 {
            let mut want = self.gear + self.shift_request;
            self.shift_request = 0;
            if self.gear == 1 && want == 0 {
                want = if self.speed < 2.5 { -1 } else { 1 };
            } else if self.gear == -1 && want == 0 {
                want = 1;
            }
            want = want.clamp(-1, top);
            if want != self.gear && (want < 0 || (want < self.gear && self.rpm_in(want) < self.spec.limit_rpm + 200.0) || want > self.gear) {
                self.shift(want);
            }
            return;
        }
        if self.manual {
            return;
        }
        let rpm = self.rpm();
        if self.gear > 0 {
            if self.brake > 0.5 && self.throttle < 0.1 && self.speed < 0.6 {
                self.reverse_hold += dt;
                if self.reverse_hold > 0.35 {
                    self.reverse_hold = 0.0;
                    self.shift(-1);
                    return;
                }
            } else {
                self.reverse_hold = 0.0;
            }
            let slipping = self.wheels.iter().any(|w| w.slide > 0.4);
            if self.gear < top && rpm > self.spec.upshift_rpm && self.throttle > 0.2 && self.on_ground >= 2 && !(slipping && self.speed < 12.0) {
                self.shift(self.gear + 1);
            } else if self.gear > 1 {
                let down = self.spec.downshift_rpm + 1400.0 * self.brake - 900.0 * (1.0 - self.throttle);
                if rpm < down && self.rpm_in(self.gear - 1) < self.spec.upshift_rpm - 500.0 {
                    self.shift(self.gear - 1);
                }
            }
        } else if self.gear < 0 && self.throttle > 0.4 && self.speed > -1.0 {
            self.shift(1);
        }
    }

    fn rpm_in(&self, g: i32) -> f64 {
        let ratio = if g < 0 { self.spec.gears[0] } else { self.spec.gears[g as usize] } * self.spec.final_drive;
        let mut w_t: f64 = self.wheels.iter().map(|w| w.omega.abs() * 0.25).sum();
        w_t = w_t.max(self.speed.abs() / self.spec.wheel_radius);
        w_t * ratio * RPM
    }

    fn shift(&mut self, to: i32) {
        self.shift_target = to;
        self.shift_timer = self.spec.shift_time;
        self.stats.shifts += 1;
    }
}

/// Slip to force through the measured tyre (`tyre.rs`), scaled by the
/// surface's friction and the car's tyre scale factors.
fn tyre(w: &Wheel, surfaces: &Surfaces, grip_scale: f64, lateral_peak_scale: f64) -> (f64, f64) {
    if !w.contact || w.load <= 0.0 {
        return (0.0, 0.0);
    }
    let surf = surfaces.get(w.surface);
    let scale = Scale { mu: surf.mu * grip_scale, sliding: surf.sliding, kx: 1.0, ky: 1.0 / lateral_peak_scale };
    let (fx, fy) = super::tyre::force(w.kappa, w.tan_alpha.atan(), w.load, scale);
    (fx, -fy)
}

/// How far past the grip peak a wheel slides (0 at the peak, 1 at three
/// times it), for wheel effects and the gearbox.
fn slide(w: &Wheel, lateral_peak_scale: f64) -> f64 {
    let s = (w.kappa / KAPPA_PEAK).hypot(w.tan_alpha.atan() / (ALPHA_PEAK * lateral_peak_scale));
    ((s - 0.8) / 2.0).clamp(0.0, 1.0)
}

fn torque_curve(pts: &[[f64; 2]], r: f64) -> f64 {
    if r <= pts[0][0] {
        return pts[0][1];
    }
    for i in 1..pts.len() {
        if r <= pts[i][0] {
            let (a, b) = (pts[i - 1], pts[i]);
            return a[1] + (b[1] - a[1]) * (r - a[0]) / (b[0] - a[0]);
        }
    }
    0.0
}

fn move_toward(from: f64, to: f64, delta: f64) -> f64 {
    if (to - from).abs() <= delta {
        to
    } else {
        from + (to - from).signum() * delta
    }
}

fn fposmod(x: f64, y: f64) -> f64 {
    let mut v = x % y;
    if (v < 0.0 && y > 0.0) || (v > 0.0 && y < 0.0) {
        v += y;
    }
    v
}

fn smoothstep(from: f64, to: f64, x: f64) -> f64 {
    if from == to {
        return if x < from { 0.0 } else { 1.0 };
    }
    let s = ((x - from) / (to - from)).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_telemetry_preserves_engine_braking_and_sums_only_contacting_slip() {
        let mut spec = CarSpec::compact();
        spec.torque = vec![[1000., 200.], [3500., 420.], [6000., 250.]];
        let mut world = PhysicsWorld::default();
        let body = super::super::add_car(&mut world, &spec, Pose3::IDENTITY, false);
        let mut car = Car::new(spec, body);
        assert_eq!(car.road_telemetry(), RoadTelemetry::default());
        car.engine_torque = -105.;
        for (i,w) in car.wheels.iter_mut().enumerate() {
            w.contact = i < 2;
            w.radius = 0.5;
            w.omega = 12.;
            w.vx = 9.;
            w.vy = -2.;
            w.surface = if i == 0 { Surface::Grass } else { Surface::Gravel };
        }
        assert_eq!(car.road_telemetry(), RoadTelemetry {
            load: -0.25, slip: 10., contact: 0.5, loose: 0.5, soft: 0.5,
        });
    }

    fn loaded_wheel() -> Wheel {
        let spec = CarSpec::compact();
        let mut physics = PhysicsWorld::default();
        let body = super::super::add_car(&mut physics, &spec, Pose3::IDENTITY, false);
        let car = Car::new(spec, body);
        assert_eq!(car.grip_scale, 1.0);
        let mut wheel = car.wheels[0].clone();
        wheel.contact = true;
        wheel.load = 3000.0;
        wheel
    }

    fn peak_lateral(wheel: &mut Wheel, surfaces: &Surfaces) -> f64 {
        (1..400).map(|i| {
            wheel.tan_alpha = (i as f64 * 0.001).tan();
            tyre(wheel, surfaces, 1.0, 1.0).1.abs()
        }).fold(0.0, f64::max)
    }

    #[test]
    fn surface_and_weather_scale_the_peak_and_keep_cornering_stiffness() {
        let mut wheel = loaded_wheel();
        let dry = Surfaces::default();
        let wet = Surfaces::with_wetness(&dry, 1.0);
        for surface in [Surface::Asphalt, Surface::Mud, Surface::Gravel] {
            wheel.surface = surface;
            let d = peak_lateral(&mut wheel, &dry);
            let w = peak_lateral(&mut wheel, &wet);
            let want = wet.get(surface).mu / dry.get(surface).mu;
            assert!((w / d - want).abs() < 0.005, "{surface:?}: {} vs {want}", w / d);
        }
        // Small slip angles: the force is the cornering stiffness times the angle
        // on every surface (MF scales friction, not stiffness).
        wheel.tan_alpha = 0.002;
        wheel.surface = Surface::Asphalt;
        let asphalt = tyre(&wheel, &dry, 1.0, 1.0).1;
        wheel.surface = Surface::Gravel;
        assert!((tyre(&wheel, &dry, 1.0, 1.0).1 / asphalt - 1.0).abs() < 0.01);
        // A lower lateral peak scale bites earlier.
        wheel.tan_alpha = 2.0f64.to_radians().tan();
        assert!(tyre(&wheel, &dry, 1.0, 0.8).1.abs() > tyre(&wheel, &dry, 1.0, 1.0).1.abs());
    }

    #[test]
    fn combined_tyre_force_stays_bounded_and_contact_gated() {
        let mut wheel = loaded_wheel();
        let surfaces = Surfaces::default();
        for surface in Surface::ALL {
            wheel.surface = surface;
            for slip in [0.0, 0.01, 0.1, 1.0, 10.0] {
                wheel.kappa = slip;
                wheel.tan_alpha = -slip;
                let (fx, fy) = tyre(&wheel, &surfaces, 1.0, 1.0);
                assert!(fx.is_finite() && fy.is_finite() && fx >= 0.0 && fy >= 0.0);
                // The reference tyre's highest friction is 1.214 (longitudinal, 3000 N).
                assert!(fx.hypot(fy) <= surfaces.get(surface).mu * 1.215 * wheel.load);
            }
        }
        wheel.contact = false;
        assert_eq!(tyre(&wheel, &surfaces, 1.0, 1.0), (0.0, 0.0));
        wheel.contact = true;
        wheel.load = 0.0;
        assert_eq!(tyre(&wheel, &surfaces, 1.0, 1.0), (0.0, 0.0));
    }

    /// Left-hand corner: tyre forces point left (negative along the wheel's
    /// right axis) and the right side is compressed. Understeer K&C turns the
    /// front wheels out of the corner (right, negative steer) and the rear
    /// wheels into it (left, positive steer), by the rig coefficients.
    #[test]
    fn compliance_steers_toward_understeer_in_a_corner() {
        let spec = CarSpec::compact();
        let mut physics = PhysicsWorld::default();
        let body = super::super::add_car(&mut physics, &spec, Pose3::IDENTITY, false);
        let mut car = Car::new(spec, body);
        for (i, w) in car.wheels.iter_mut().enumerate() {
            w.contact = true;
            w.load = 3000.0;
            w.fy = -2000.0;
            w.tan_alpha = 0.0;
            w.compression = if i % 2 == 1 { 0.1 } else { 0.06 };
        }
        car.compliance();
        let s = &car.spec;
        let roll = (0.04 / s.track).to_degrees();
        let front = -(s.force_steer[0] * 2.0 + s.torque_steer[0] * super::super::tyre::trail(0.0, 3000.0) * 20.0 + s.roll_steer[0] * roll);
        assert!(front < 0.0 && (car.wheels[0].toe.to_degrees() - front).abs() < 1e-9, "{} {front}", car.wheels[0].toe.to_degrees());
        assert_eq!(car.wheels[0].toe, car.wheels[1].toe);
        let rear = s.force_steer[1] * 2.0 + s.torque_steer[1] * super::super::tyre::trail(0.0, 3000.0) * 20.0 + s.roll_steer[1] * (0.04 / s.track_rear()).to_degrees();
        assert!((car.wheels[2].toe.to_degrees() - rear).abs() < 1e-9, "{} {rear}", car.wheels[2].toe.to_degrees());
        // The compact's rear compliance is the typical oversteer kind (out of
        // the corner); an understeer coefficient turns the rear wheels in.
        assert!(car.wheels[2].toe < 0.0);
        car.spec.force_steer[1] = 0.3;
        car.compliance();
        assert!(car.wheels[2].toe > 0.0);
        // A rigid suspension does not steer.
        car.spec.force_steer = [0.0; 2];
        car.spec.torque_steer = [0.0; 2];
        car.spec.roll_steer = [0.0; 2];
        car.compliance();
        assert!(car.wheels.iter().all(|w| w.toe == 0.0));
    }
}
