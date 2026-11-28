//! Handling lab: the standard objective vehicle tests, run headless on flat
//! ground, so the tyre and car model can be checked against measured cars.
//!
//! - skidpad: constant radius 30.5 m (the 200 ft circle road tests use),
//!   pure-pursuit driver, speed ramped slowly: steady lateral g and the
//!   understeer gradient (ISO 4138 constant radius).
//! - braking 100-0 km/h: wheels locked, and an ideal threshold driver that
//!   holds the slip ratio at the tyre's peak (what ABS approximates).
//! - step steer at 80 km/h to 4 m/s^2 (ISO 7401): yaw-rate response time
//!   and overshoot.
//! - sine with dwell at 80 km/h (FMVSS 126 / UN R13-H): yaw rate 1.0 s and
//!   1.75 s after the steering ends, as a fraction of the peak, i.e. whether
//!   the car stops sliding by itself.
//!
//! cargo run --release -p physics --example handling_lab -- [out.json]

use physics::vehicle::surfaces::{Surface, Surfaces};
use physics::vehicle::{Car, CarSpec, Ground, HZ, add_car};
use rapier3d::glamx::{Pose3, Quat, Vec3};
use rapier3d::prelude::*;
use serde_json::{Value, json};

struct Flat(Surface);

impl Ground for Flat {
    fn road_surface(&self, _p: Vec3, hint: i64) -> (Surface, i64) {
        (self.0, hint)
    }
    fn water_level(&self, _x: f64, _z: f64) -> f64 {
        f64::NEG_INFINITY
    }
}

const G: f64 = 9.81;
const DT: f64 = 1.0 / HZ;

struct Rig {
    world: PhysicsWorld,
    car: Car,
    ground: Flat,
    surfaces: Surfaces,
}

impl Rig {
    fn new(spec: &CarSpec, surface: Surface) -> Rig {
        let mut world = PhysicsWorld::default();
        world.gravity = Vec3::new(0., -G as f32, 0.);
        world.integration_parameters.dt = DT as f32;
        world.colliders.insert(
            ColliderBuilder::new(SharedShape::halfspace(Vec3::Y))
                .user_data(surface as u128)
                .collision_groups(InteractionGroups::new(physics::vehicle::GROUP_WORLD, Group::ALL, InteractionTestMode::And))
                .build(),
        );
        let pose = Pose3::from_translation(Vec3::new(0., spec.wheel_radius as f32 + 0.05, 0.));
        let body = add_car(&mut world, spec, pose, true);
        let mut car = Car::new(spec.clone(), body);
        car.steer_assist = false;
        let mut rig = Rig { world, car, ground: Flat(surface), surfaces: Surfaces::default() };
        rig.car.hold = true;
        for _ in 0..(2.0 * HZ) as usize {
            rig.step();
        }
        rig.car.hold = false;
        rig
    }

    fn step(&mut self) {
        self.car.integrate(&mut self.world, &self.ground, &self.surfaces, DT);
        self.world.step();
    }

    /// Roll at `v` m/s straight ahead, wheels rolling, a gear that fits.
    fn launch(&mut self, v: f64) {
        let b = &mut self.world.bodies[self.car.body];
        let fwd = b.position().rotation * Vec3::NEG_Z;
        b.set_linvel(fwd * v as f32, true);
        b.set_angvel(Vec3::ZERO, true);
        for w in &mut self.car.wheels {
            w.omega = v / w.radius;
            w.kappa = 0.0;
        }
        let spec = &self.car.spec;
        let mut gear = 1;
        for g in 1..spec.gears.len() {
            let rpm = v / spec.wheel_radius * spec.gears[g] * spec.final_drive * 60.0 / std::f64::consts::TAU;
            gear = g;
            if rpm < spec.upshift_rpm * 0.85 {
                break;
            }
        }
        let rpm = v / spec.wheel_radius * spec.gears[gear] * spec.final_drive * 60.0 / std::f64::consts::TAU;
        self.car.gear = gear as i32;
        self.car.shift_target = gear as i32;
        self.car.engine_omega = rpm.max(spec.idle_rpm) / (60.0 / std::f64::consts::TAU);
    }

    fn pose(&self) -> Pose3 {
        *self.world.bodies[self.car.body].position()
    }
    fn lin(&self) -> Vec3 {
        self.world.bodies[self.car.body].linvel()
    }
    fn yaw_rate(&self) -> f64 {
        self.world.bodies[self.car.body].angvel().y as f64
    }
    fn fwd(&self) -> Vec3 {
        self.pose().rotation * Vec3::NEG_Z
    }
    fn right(&self) -> Vec3 {
        self.pose().rotation * Vec3::X
    }
    fn speed(&self) -> f64 {
        let l = self.lin();
        (l.x as f64).hypot(l.z as f64)
    }
    /// Body slip angle, degrees, positive when the velocity points right of the nose.
    fn beta_deg(&self) -> f64 {
        let l = self.lin();
        (l.dot(self.right()) as f64).atan2(l.dot(self.fwd()) as f64).to_degrees()
    }
    /// Road-wheel steer, radians, positive to the left.
    fn steer(&self) -> f64 {
        self.car.wheels[0].steer
    }
    /// Command a road-wheel steer angle (radians, positive left).
    fn set_steer(&mut self, delta: f64) {
        self.car.steer_input = (-delta / self.car.spec.max_steer()).clamp(-1.0, 1.0);
    }
    /// Hold speed with throttle (and brake when far above).
    fn hold_speed(&mut self, want: f64) {
        let e = want - self.speed();
        self.car.throttle = (e * 0.6 + 0.12).clamp(0.0, 1.0);
        self.car.brake = (-e * 0.3 - 0.1).clamp(0.0, 1.0);
    }
    /// Steady-state steer for lateral acceleration `ay` at `v`: bisect the
    /// constant-steer response (used to size the step and sine inputs).
    fn steer_for(spec: &CarSpec, surface: Surface, v: f64, ay: f64) -> f64 {
        let (mut lo, mut hi) = (0.0f64, 0.2f64);
        for _ in 0..14 {
            let mid = 0.5 * (lo + hi);
            let mut rig = Rig::new(spec, surface);
            rig.launch(v);
            let mut acc = 0.0;
            let n = (1.5 * HZ) as usize;
            for i in 0..(4.0 * HZ) as usize {
                rig.set_steer(mid);
                rig.hold_speed(v);
                rig.step();
                if i >= (4.0 * HZ) as usize - n {
                    acc += rig.yaw_rate() * rig.speed();
                }
            }
            if acc / n as f64 > ay { hi = mid } else { lo = mid }
        }
        0.5 * (lo + hi)
    }
}

fn skidpad(spec: &CarSpec, surface: Surface) -> Value {
    let radius = 30.5;
    let mut rig = Rig::new(spec, surface);
    let p0 = rig.pose().translation;
    let left = -rig.right();
    let centre = p0 + left * radius as f32;
    let v0 = 8.0;
    rig.launch(v0);
    let mut want = v0;
    let mut trace = Vec::new();
    let mut window: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut best = 0.0f64;
    let mut lost_at = None;
    let mut t = 0.0;
    let mut integral = 0.0f64;
    while t < 240.0 {
        // Pure pursuit to a point 0.35 rad ahead on the circle.
        let p = rig.pose().translation;
        let r = p - centre;
        let ang = (r.z as f64).atan2(r.x as f64);
        // Counter-clockwise seen from above (+Y): angle decreases in x-z with -Z forward? use tangent check.
        let tangent = Vec3::new(-r.z, 0.0, r.x).normalize();
        let dir = if tangent.dot(rig.fwd()) > 0.0 { 1.0 } else { -1.0 };
        let a2 = ang + dir * 0.35;
        let target = centre + Vec3::new(a2.cos() as f32, 0.0, a2.sin() as f32) * radius as f32;
        let to = target - p;
        let ld = (to.x as f64).hypot(to.z as f64);
        let eta = (to.dot(-rig.right()) as f64).atan2(to.dot(rig.fwd()) as f64);
        let err_now = (r.x as f64).hypot(r.z as f64) - radius;
        // Pure pursuit plus integral action on the radial error, so the
        // driver keeps the circle as the car understeers.
        integral = (integral + err_now * DT).clamp(-20.0, 20.0);
        let delta = (2.0 * spec.wheelbase * eta.sin() / ld).atan() + 0.02 * err_now + 0.01 * integral;
        rig.set_steer(delta);
        want += 0.08 * DT;
        rig.hold_speed(want);
        rig.step();
        t += DT;
        let err = (r.x as f64).hypot(r.z as f64) - radius;
        let v = rig.speed();
        let ay = rig.yaw_rate().abs() * v;
        window.push((ay, err.abs(), rig.steer().abs(), v));
        if window.len() > HZ as usize {
            window.remove(0);
        }
        if window.len() == HZ as usize && t > 5.0 {
            let n = window.len() as f64;
            let ay_m = window.iter().map(|w| w.0).sum::<f64>() / n;
            let e_m = window.iter().map(|w| w.1).fold(0.0, f64::max);
            let d_m = window.iter().map(|w| w.2).sum::<f64>() / n;
            let v_m = window.iter().map(|w| w.3).sum::<f64>() / n;
            if (t * 4.0).fract() < DT * 4.0 {
                let w = &rig.car.wheels;
                let slip = |i: usize| w[i].tan_alpha.atan().abs().to_degrees();
                let roll = ((w[1].compression - w[0].compression) / spec.track).to_degrees().abs();
                trace.push(json!({"t": t, "ay_g": ay_m / G, "steer_deg": d_m.to_degrees(), "v": v_m, "err": e_m, "beta": rig.beta_deg(),
                    "front_slip_deg": (slip(0) + slip(1)) / 2.0, "rear_slip_deg": (slip(2) + slip(3)) / 2.0, "roll_deg": roll,
                    "front_toe_deg": (w[0].toe + w[1].toe).to_degrees() / 2.0, "front_camber_deg": (w[0].camber + w[1].camber).to_degrees() / 2.0}));
            }
            if e_m < 0.5 {
                best = best.max(ay_m);
            }
            if e_m > 2.0 && lost_at.is_none() {
                lost_at = Some(t);
            }
        }
        if lost_at.is_some_and(|l| t > l + 2.0) {
            break;
        }
    }
    // Understeer gradient: slope of (steer - L/R) against ay over 0.1..0.4 g.
    let pts: Vec<(f64, f64)> = trace
        .iter()
        .filter_map(|p| {
            let g = p["ay_g"].as_f64()?;
            (g > 0.1 && g < 0.4 && p["err"].as_f64()? < 0.5).then(|| (g, p["steer_deg"].as_f64().unwrap()))
        })
        .collect();
    let k = slope(&pts);
    // The same fit for the parts of the budget: axle slip angles and roll.
    let fit = |key: &str| {
        let pts: Vec<(f64, f64)> = trace.iter().filter_map(|p| {
            let g = p["ay_g"].as_f64()?;
            (g > 0.1 && g < 0.4 && p["err"].as_f64()? < 0.5).then(|| (g, p[key].as_f64().unwrap()))
        }).collect();
        slope(&pts)
    };
    let budget = json!({"front_slip": fit("front_slip_deg"), "rear_slip": fit("rear_slip_deg"), "roll": fit("roll_deg"), "front_toe": fit("front_toe_deg"), "front_camber": fit("front_camber_deg")});
    eprintln!("  skidpad budget (deg/g): {budget}");
    json!({"radius_m": radius, "max_lat_g": best / G, "understeer_deg_per_g": k, "budget": budget,
           "ackermann_deg": (spec.wheelbase / radius).to_degrees(), "trace": trace})
}

fn slope(p: &[(f64, f64)]) -> f64 {
    let n = p.len() as f64;
    if n < 2.0 {
        return f64::NAN;
    }
    let mx = p.iter().map(|q| q.0).sum::<f64>() / n;
    let my = p.iter().map(|q| q.1).sum::<f64>() / n;
    let sxy: f64 = p.iter().map(|q| (q.0 - mx) * (q.1 - my)).sum();
    let sxx: f64 = p.iter().map(|q| (q.0 - mx).powi(2)).sum();
    sxy / sxx
}

fn braking(spec: &CarSpec, surface: Surface, threshold: bool) -> Value {
    let mut rig = Rig::new(spec, surface);
    let v0 = 100.0 / 3.6;
    rig.launch(v0);
    let start = rig.pose().translation;
    let mut t = 0.0;
    let mut brake = 1.0f64;
    let mut trace = Vec::new();
    while rig.speed() > 0.3 && t < 20.0 {
        rig.car.throttle = 0.0;
        rig.set_steer(0.0);
        if threshold {
            // Ideal driver: one pedal for all four wheels, keep the worst
            // wheel's slip ratio near the tyre's longitudinal peak.
            let v = rig.speed().max(1.0);
            let worst = rig.car.wheels.iter().map(|w| (v - w.omega * w.radius) / v).fold(0.0, f64::max);
            brake = (brake + (0.10 - worst) * 8.0 * DT * 10.0).clamp(0.0, 1.0);
        }
        rig.car.brake = brake;
        rig.step();
        t += DT;

        if (t * 20.0).fract() < DT * 20.0 {
            trace.push(json!({"t": t, "v": rig.speed()}));
        }
    }
    let d = rig.pose().translation - start;
    json!({"distance_m": (d.x as f64).hypot(d.z as f64), "time_s": t, "trace": trace})
}

/// Standing start, full throttle: 0-100 km/h time (s).
fn launch(spec: &CarSpec, surface: Surface) -> Value {
    let mut rig = Rig::new(spec, surface);
    let mut t = 0.0;
    let mut trace = Vec::new();
    let mut t100 = f64::NAN;
    while t < 30.0 {
        rig.car.throttle = 1.0;
        rig.set_steer(0.0);
        rig.step();
        t += DT;
        if (t * 10.0).fract() < DT * 10.0 {
            trace.push(json!({"t": t, "kmh": rig.speed() * 3.6, "gear": rig.car.gear}));
        }
        if rig.speed() * 3.6 >= 100.0 {
            t100 = t;
            break;
        }
    }
    json!({"t_0_100_s": t100, "trace": trace})
}

fn step_steer(spec: &CarSpec, surface: Surface) -> Value {
    let v = 80.0 / 3.6;
    let delta = Rig::steer_for(spec, surface, v, 4.0);
    let mut rig = Rig::new(spec, surface);
    rig.launch(v);
    let mut trace = Vec::new();
    let mut t = 0.0;
    let t0 = 1.0;
    while t < 6.0 {
        // Ramp in 0.15 s (ISO 7401 asks for 0.1-0.2 s), then hold.
        let u = ((t - t0) / 0.15).clamp(0.0, 1.0);
        rig.set_steer(delta * u);
        rig.hold_speed(v);
        rig.step();
        t += DT;
        trace.push((t, rig.steer(), rig.yaw_rate(), rig.yaw_rate() * rig.speed(), rig.beta_deg()));
    }
    let steady = trace.iter().filter(|p| p.0 > 5.0).map(|p| p.2).sum::<f64>() / trace.iter().filter(|p| p.0 > 5.0).count() as f64;
    let t50 = trace.iter().find(|p| p.1 >= delta * 0.5).map(|p| p.0).unwrap_or(f64::NAN);
    let t90 = trace.iter().find(|p| p.0 > t50 && p.2 >= steady * 0.9).map(|p| p.0).unwrap_or(f64::NAN);
    let (tp, peak) = trace.iter().fold((0.0, 0.0f64), |a, p| if p.2 > a.1 { (p.0, p.2) } else { a });
    json!({"steer_deg": delta.to_degrees(), "steady_yaw_rate_dps": steady.to_degrees(),
           "response_time_s": t90 - t50, "peak_time_s": tp - t50, "overshoot_pct": (peak / steady - 1.0) * 100.0,
           "trace": trace.iter().step_by(3).map(|p| json!({"t": p.0 - t50, "steer_deg": p.1.to_degrees(), "yaw_dps": p.2.to_degrees(), "ay_g": p.3 / G, "beta": p.4})).collect::<Vec<_>>()})
}

fn sine_dwell(spec: &CarSpec, surface: Surface) -> Value {
    let v = 80.0 / 3.6;
    let a = Rig::steer_for(spec, surface, v, 0.3 * G);
    let f = 0.7;
    let mut runs = Vec::new();
    for scale in [1.5, 2.5, 3.5, 4.5, 5.5, 6.5] {
        let amp = (a * scale).min(spec.max_steer());
        let mut rig = Rig::new(spec, surface);
        rig.launch(v);
        let mut t = 0.0;
        let mut trace = Vec::new();
        let (p0, right0) = (rig.pose().translation, rig.right());
        // FMVSS 126 responsiveness: lateral displacement 1.07 s after the steer starts.
        let mut lateral = f64::NAN;
        let period = 1.0 / f;
        // 3/4 period, 500 ms dwell at the second peak, last quarter.
        let end = 0.75 * period + 0.5 + 0.25 * period;
        while t < end + 3.0 {
            let s = if t < 0.75 * period {
                (std::f64::consts::TAU * f * t).sin()
            } else if t < 0.75 * period + 0.5 {
                -1.0
            } else if t < end {
                (std::f64::consts::TAU * f * (t - 0.5)).sin()
            } else {
                0.0
            };
            // Left first (positive yaw), as the regulation's left-steer run.
            rig.set_steer(amp * s);
            rig.car.throttle = 0.0;
            rig.car.brake = 0.0;
            rig.step();
            t += DT;
            if lateral.is_nan() && t >= 1.07 {
                lateral = -((rig.pose().translation - p0).dot(right0) as f64);
            }
            trace.push((t, rig.steer(), rig.yaw_rate(), rig.beta_deg()));
        }
        // Peak yaw rate of the second (opposite) lobe, then the ratios.
        let peak = trace.iter().filter(|p| p.0 > 0.5 * period).map(|p| -p.2).fold(0.0f64, f64::max);
        let at = |tt: f64| trace.iter().find(|p| p.0 >= tt).map(|p| -p.2).unwrap_or(f64::NAN);
        let max_beta = trace.iter().map(|p| p.3.abs()).fold(0.0, f64::max);
        runs.push(json!({"scale": scale, "amp_deg": amp.to_degrees(), "peak_dps": peak.to_degrees(),
            "ratio_1s": at(end + 1.0) / peak, "ratio_175s": at(end + 1.75) / peak, "max_beta_deg": max_beta, "lateral_m": lateral,
            "spun": max_beta > 30.0,
            "trace": trace.iter().step_by(3).map(|p| json!({"t": p.0, "steer_deg": p.1.to_degrees(), "yaw_dps": p.2.to_degrees(), "beta": p.3})).collect::<Vec<_>>()}));
    }
    json!({"A_deg": a.to_degrees(), "runs": runs})
}

fn rally2() -> CarSpec {
    let mut s = CarSpec::compact();
    s.name = "Rally2".into();
    s.abs = false;
    s.traction_control = false;
    s.esc = false;
    // Rose-jointed rally suspension: no measured K&C, kept rigid like Sideways' cars.
    s.force_steer = [0.0; 2];
    s.torque_steer = [0.0; 2];
    s.roll_steer = [0.0; 2];
    s.roll_camber = [0.0; 2];
    s.force_camber = [0.0; 2];
    s.drive = [0.5, 0.5];
    s.centre_lock = 900.0;
    s.diff_preload = [80.0, 120.0];
    s.diff_ramp = [0.45, 0.65];
    s.max_steer_deg = 30.0;
    s.limit_rpm = 7600.0;
    s.torque = vec![[1000., 160.], [2000., 250.], [3000., 390.], [4000., 440.], [5000., 430.], [6000., 385.], [7000., 330.], [7600., 290.], [8200., 0.]];
    s.shift_time = 0.08;
    s.upshift_rpm = 7150.0;
    s.downshift_rpm = 3600.0;
    s.drag = 0.42;
    s.downforce = 0.12;
    s.boxes = vec![[1.75, 0.7, 4.0, 0.0, 0.45, 0.0]];
    s
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "build/handling_lab.json".into());
    let mut all = serde_json::Map::new();
    for (name, spec, surface) in [
        ("compact_asphalt", CarSpec::compact(), Surface::Asphalt),
        ("rally2_gravel", rally2(), Surface::Gravel),
        ("rally2_asphalt", rally2(), Surface::Asphalt),
        ("compact_gravel", CarSpec::compact(), Surface::Gravel),
        ("compact_asphalt_no_esc", CarSpec { esc: false, ..CarSpec::compact() }, Surface::Asphalt),
        ("compact_gravel_no_esc", CarSpec { esc: false, ..CarSpec::compact() }, Surface::Gravel),
    ] {
        let r = json!({
            "skidpad": skidpad(&spec, surface),
            "brake_locked": braking(&spec, surface, false),
            "brake_threshold": braking(&spec, surface, true),
            "step_steer": step_steer(&spec, surface),
            "launch": launch(&spec, surface),
            "sine_dwell": sine_dwell(&spec, surface),
        });
        eprintln!(
            "{name}: skidpad {:.2} g, understeer {:.2} deg/g, brake locked {:.1} m, threshold {:.1} m, step response {:.3} s, overshoot {:.0}%",
            r["skidpad"]["max_lat_g"].as_f64().unwrap_or(f64::NAN),
            r["skidpad"]["understeer_deg_per_g"].as_f64().unwrap_or(f64::NAN),
            r["brake_locked"]["distance_m"].as_f64().unwrap_or(f64::NAN),
            r["brake_threshold"]["distance_m"].as_f64().unwrap_or(f64::NAN),
            r["step_steer"]["response_time_s"].as_f64().unwrap_or(f64::NAN),
            r["step_steer"]["overshoot_pct"].as_f64().unwrap_or(f64::NAN),
        );
        eprintln!("  0-100 {:.2} s", r["launch"]["t_0_100_s"].as_f64().unwrap_or(f64::NAN));
        for run in r["sine_dwell"]["runs"].as_array().unwrap() {
            eprintln!(
                "  sine-dwell x{}: {:.1} deg, yaw ratio 1s {:.2} 1.75s {:.2}, max beta {:.1}, lateral {:.2} m",
                run["scale"], run["amp_deg"].as_f64().unwrap(), run["ratio_1s"].as_f64().unwrap(), run["ratio_175s"].as_f64().unwrap(), run["max_beta_deg"].as_f64().unwrap(), run["lateral_m"].as_f64().unwrap_or(f64::NAN)
            );
        }
        all.insert(name.into(), r);
    }
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&out, serde_json::to_string(&Value::Object(all)).unwrap()).unwrap();
    eprintln!("wrote {out}");
    let _ = Quat::IDENTITY;
    // Keyboard-player manoeuvres at 80 km/h: body slip over time.
    let mut players = serde_json::Map::new();
    for (name, spec, surface) in [("compact_asphalt", CarSpec::compact(), Surface::Asphalt), ("rally2_gravel", rally2(), Surface::Gravel)] {
        let pick = |tr: Vec<(f64, f64, f64, f64)>| tr.iter().step_by(6).map(|p| json!({"t": p.0, "beta": p.1, "yaw_dps": p.2})).collect::<Vec<_>>();
        let lift = player(&spec, surface, 0.0, 80.0 / 3.6);
        let power = player(&spec, surface, 1.0, 80.0 / 3.6);
        let brake = brake_turn(&spec, surface, 80.0 / 3.6);
        let max = |tr: &[(f64, f64, f64, f64)]| tr.iter().map(|p| p.1.abs()).fold(0.0, f64::max);
        eprintln!("player {name}: max body slip, lift {:.1}, full throttle {:.1}, full brake {:.1} deg", max(&lift), max(&power), max(&brake));
        players.insert(name.into(), json!({"lift": pick(lift), "throttle": pick(power), "brake": pick(brake)}));
    }
    std::fs::write(out.replace(".json", "_player.json"), serde_json::to_string(&Value::Object(players)).unwrap()).unwrap();
}

/// What a keyboard player does: full lock with the game's steering assist,
/// on or off the throttle, then let go. Body slip and yaw over time.
#[allow(dead_code)]
pub fn player(spec: &CarSpec, surface: Surface, throttle: f64, v: f64) -> Vec<(f64, f64, f64, f64)> {
    let mut rig = Rig::new(spec, surface);
    rig.car.steer_assist = true;
    rig.launch(v);
    let mut t = 0.0;
    let mut out = Vec::new();
    while t < 5.0 {
        rig.car.steer_input = if t < 1.5 { -1.0 } else { 0.0 };
        rig.car.throttle = throttle;
        rig.car.brake = 0.0;
        rig.step();
        t += DT;
        out.push((t, rig.beta_deg(), rig.yaw_rate().to_degrees(), rig.speed() * 3.6));
    }
    out
}

/// Keyboard player braking into a turn: full brake and full steer together.
#[allow(dead_code)]
pub fn brake_turn(spec: &CarSpec, surface: Surface, v: f64) -> Vec<(f64, f64, f64, f64)> {
    let mut rig = Rig::new(spec, surface);
    rig.car.steer_assist = true;
    rig.launch(v);
    let mut t = 0.0;
    let mut out = Vec::new();
    while t < 4.0 {
        rig.car.steer_input = if t < 2.0 { -1.0 } else { 0.0 };
        rig.car.throttle = 0.0;
        rig.car.brake = if t < 1.2 { 1.0 } else { 0.0 };
        rig.step();
        t += DT;
        let locked = rig.car.wheels.iter().filter(|w| w.omega.abs() < 0.5).count() as f64;
        out.push((t, rig.beta_deg(), rig.yaw_rate().to_degrees(), locked));
    }
    out
}
