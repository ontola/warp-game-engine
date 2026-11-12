//! CPU-only, deterministic tyre-tuning comparison on flat gravel.
//!
//! Run: cargo run --locked -p sideways --example handling_probe -- [cars.luau] [car ids...]
//! With no arguments, load normal game data and compare Asti and MV6.
//! Each maneuver settles an original-spec car for 360 ticks before applying
//! tuning and initializing the same body, wheel and engine speeds. No window,
//! renderer, controller, random inputs or wall-clock timing is involved.

use rapier3d::glamx::{Pose3, Vec3};
use serde::Serialize;
use serde_json::{Value, json};
use sideways::{
    bench,
    car::{Car, Ground, SUBSTEPS},
    physics::{HZ, Physics},
    spec::{CarSpec, Cars},
    surfaces::{Surface, Surfaces},
};

const DT: f64 = 1.0 / HZ;
const SETTLE_TICKS: usize = 360;
const RUN_TICKS: usize = 600;
const FORWARD_MPS: f64 = 60.0 / 3.6;
const IMPULSE_DELTA_V_MPS: f64 = 5.0;
const HOLD_TICKS: usize = 60;

struct FlatGround;

impl Ground for FlatGround {
    fn road_surface(&self, _p: Vec3, hint: i64) -> (Surface, i64) {
        (Surface::Gravel, hint)
    }

    fn water_level(&self, _x: f64, _z: f64) -> f64 {
        f64::NEG_INFINITY
    }
}

#[derive(Clone, Copy)]
struct Tuning {
    name: &'static str,
    grip: f64,
    lateral_peak: f64,
}

const ORIGINAL: Tuning = Tuning { name: "original", grip: 1.0, lateral_peak: 1.0 };
const CANDIDATE: Tuning = Tuning { name: "candidate", grip: 1.12, lateral_peak: 0.80 };

#[derive(Clone, Copy)]
enum Maneuver {
    LateralImpulse,
    SteerCountersteer,
}

impl Maneuver {
    fn name(self) -> &'static str {
        match self {
            Self::LateralImpulse => "lateral_impulse",
            Self::SteerCountersteer => "steer_countersteer",
        }
    }

    fn release_tick(self) -> usize {
        match self {
            Self::LateralImpulse => 0,
            Self::SteerCountersteer => 120,
        }
    }

    fn input(self, tick: usize) -> f64 {
        match self {
            Self::SteerCountersteer if tick < 90 => 0.35,
            Self::SteerCountersteer if tick < 120 => -0.18,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct Sample {
    speed_kmh: f64,
    forward_mps: f64,
    lateral_mps: f64,
    body_slip_deg: f64,
    yaw_rate_rad_s: f64,
    heading_rad: f64,
}

fn sample(car: &Car, physics: &Physics) -> Sample {
    let rotation = car.pose(physics).rotation;
    let velocity = car.linvel(physics);
    let forward = rotation * Vec3::NEG_Z;
    let lateral_mps = velocity.dot(rotation * Vec3::X) as f64;
    let forward_mps = velocity.dot(forward) as f64;
    Sample {
        speed_kmh: velocity.length() as f64 * 3.6,
        forward_mps,
        lateral_mps,
        body_slip_deg: lateral_mps.atan2(forward_mps.abs()).to_degrees(),
        yaw_rate_rad_s: car.angvel(physics).y as f64,
        heading_rad: (-forward.x as f64).atan2(-forward.z as f64),
    }
}

fn state_finite(car: &Car, physics: &Physics) -> bool {
    let pose = car.pose(physics);
    pose.translation.is_finite()
        && pose.rotation.is_finite()
        && car.linvel(physics).is_finite()
        && car.angvel(physics).is_finite()
        && [car.engine_omega, car.speed, car.shift_timer, car.grip_scale].iter().all(|v| v.is_finite())
        && car.wheels.iter().all(|w| {
            w.point.is_finite() && w.normal.is_finite() && w.wf.is_finite() && w.wr.is_finite()
                && [w.omega, w.spin, w.steer, w.compression, w.last_compression, w.load,
                    w.kappa, w.tan_alpha, w.fx, w.fy, w.vx, w.vy, w.slide]
                    .iter().all(|v| v.is_finite())
        })
}

fn apply_tuning(spec: &mut CarSpec, tuning: Tuning) {
    spec.tyre_grip_scale = tuning.grip;
    spec.lateral_peak_scale = tuning.lateral_peak;
}

fn fixture(source: &CarSpec, tuning: Tuning, maneuver: Maneuver) -> Result<(Physics, Car), String> {
    let mut spec = source.clone();
    apply_tuning(&mut spec, ORIGINAL);
    let mut physics = Physics::new();
    physics.add_flat_ground(Surface::Gravel);
    let body = physics.add_car(&spec, Pose3::from_translation(Vec3::new(0.0, 0.36, 0.0)), true);
    let mut car = Car::new(spec, body);
    car.manual = true;
    car.brake = 1.0;
    let surfaces = Surfaces::default();
    for _ in 0..SETTLE_TICKS {
        car.integrate(&mut physics, &FlatGround, &surfaces, DT);
        physics.step();
        if !state_finite(&car, &physics) {
            return Err("non-finite state while settling".into());
        }
    }
    if car.on_ground != 4 {
        return Err(format!("fixture did not settle on all four wheels: {} contacts", car.on_ground));
    }

    // Keep the settled pose and suspension state. Every tuning uses the same
    // original-spec settling process; only the two tyre parameters now differ.
    apply_tuning(&mut car.spec, tuning);
    car.grip_scale = tuning.grip;
    car.brake = 0.0;
    car.gear = 3;
    car.shift_target = 3;
    car.shift_timer = 0.0;
    car.speed = FORWARD_MPS;
    car.engine_omega = FORWARD_MPS / car.spec.wheel_radius * car.spec.gears[3] * car.spec.final_drive;
    for wheel in &mut car.wheels {
        wheel.omega = FORWARD_MPS / wheel.radius;
        wheel.kappa = 0.0;
        wheel.tan_alpha = 0.0;
    }
    let rotation = car.pose(&physics).rotation;
    let lateral = if matches!(maneuver, Maneuver::LateralImpulse) { IMPULSE_DELTA_V_MPS } else { 0.0 };
    let body = &mut physics.world.bodies[car.body];
    body.set_linvel(rotation * Vec3::new(lateral as f32, 0.0, -FORWARD_MPS as f32), true);
    body.set_angvel(Vec3::ZERO, true);
    body.reset_forces(true);
    body.reset_torques(true);
    Ok((physics, car))
}

fn run_maneuver(source: &CarSpec, tuning: Tuning, maneuver: Maneuver) -> Result<Value, String> {
    let (mut physics, mut car) = fixture(source, tuning, maneuver)?;
    let surfaces = Surfaces::default();
    let pose = car.pose(&physics);
    let initial = sample(&car, &physics);
    let initial_state = json!({
        "pose_translation": pose.translation.to_array(),
        "pose_rotation": pose.rotation.to_array(),
        "linear_velocity": car.linvel(&physics).to_array(),
        "angular_velocity": car.angvel(&physics).to_array(),
        "engine_omega": car.engine_omega,
        "wheel_omega": car.wheels.iter().map(|w| w.omega).collect::<Vec<_>>(),
        "wheel_compression": car.wheels.iter().map(|w| w.compression).collect::<Vec<_>>(),
        "wheel_last_compression": car.wheels.iter().map(|w| w.last_compression).collect::<Vec<_>>(),
        "wheel_load": car.wheels.iter().map(|w| w.load).collect::<Vec<_>>(),
        "sample": initial,
    });
    let mut previous = initial;
    let mut at_release = initial;
    let mut lateral_travel = 0.0;
    let mut lateral_at_release = 0.0;
    let mut heading_delta = 0.0;
    let mut heading_at_release = 0.0;
    let mut peak_slip = initial.body_slip_deg.abs();
    let mut peak_yaw = initial.yaw_rate_rad_s.abs();
    let mut peak_yaw_after_release = 0.0_f64;
    let mut reverse_yaw_after_release = 0.0_f64;
    let mut min_forward = initial.forward_mps;
    let mut min_contacts = 4;
    let mut contact_loss_ticks = 0;
    let mut non_finite_tick = None;
    let mut stable: Option<(usize, Sample, f64)> = None;
    let mut recovery = Value::Null;
    let mut completed_ticks = 0;
    for tick in 0..RUN_TICKS {
        if tick == maneuver.release_tick() {
            at_release = previous;
            lateral_at_release = lateral_travel;
            heading_at_release = heading_delta;
            if previous.body_slip_deg.abs() < 3.0 && previous.yaw_rate_rad_s.abs() < 0.1 {
                stable = Some((tick, previous, lateral_travel));
            }
        }
        car.steer_input = maneuver.input(tick);
        car.integrate(&mut physics, &FlatGround, &surfaces, DT);
        physics.step();
        completed_ticks = tick + 1;
        if !state_finite(&car, &physics) {
            non_finite_tick = Some(tick + 1);
            break;
        }
        let current = sample(&car, &physics);
        lateral_travel += (previous.lateral_mps.abs() + current.lateral_mps.abs()) * 0.5 * DT;
        let heading_step = current.heading_rad - previous.heading_rad;
        heading_delta += heading_step.sin().atan2(heading_step.cos());
        peak_slip = peak_slip.max(current.body_slip_deg.abs());
        peak_yaw = peak_yaw.max(current.yaw_rate_rad_s.abs());
        min_forward = min_forward.min(current.forward_mps);
        min_contacts = min_contacts.min(car.on_ground);
        if car.on_ground != 4 {
            contact_loss_ticks += 1;
        }
        if tick >= maneuver.release_tick() {
            peak_yaw_after_release = peak_yaw_after_release.max(current.yaw_rate_rad_s.abs());
            if at_release.yaw_rate_rad_s.abs() > 1e-6 {
                reverse_yaw_after_release = reverse_yaw_after_release.max(-current.yaw_rate_rad_s * at_release.yaw_rate_rad_s.signum());
            }
            if current.body_slip_deg.abs() < 3.0 && current.yaw_rate_rad_s.abs() < 0.1 {
                let (start, start_sample, start_travel) = *stable.get_or_insert((tick + 1, current, lateral_travel));
                if recovery.is_null() && tick + 1 - start >= HOLD_TICKS {
                    recovery = json!({
                        "stable_start_after_release_s": (start - maneuver.release_tick()) as f64 * DT,
                        "confirmed_after_release_s": (tick + 1 - maneuver.release_tick()) as f64 * DT,
                        "speed_at_stable_start_kmh": start_sample.speed_kmh,
                        "speed_at_confirmation_kmh": current.speed_kmh,
                        "body_lateral_travel_until_stable_m": start_travel - lateral_at_release,
                    });
                }
            } else {
                stable = None;
            }
        }
        previous = current;
    }
    Ok(json!({
        "initial_state": initial_state,
        "at_release": at_release,
        "final": previous,
        "recovery": recovery,
        "finite": non_finite_tick.is_none(),
        "non_finite_tick": non_finite_tick,
        "all_four_wheels_in_contact": contact_loss_ticks == 0,
        "minimum_contact_wheels": min_contacts,
        "contact_loss_ticks": contact_loss_ticks,
        "completed_ticks": completed_ticks,
        "peak_abs_body_slip_deg": peak_slip,
        "peak_abs_yaw_rate_rad_s": peak_yaw,
        "peak_abs_yaw_rate_after_release_rad_s": peak_yaw_after_release,
        "peak_reverse_yaw_rate_after_release_rad_s": reverse_yaw_after_release,
        "body_lateral_travel_m": lateral_travel,
        "body_lateral_travel_after_release_m": lateral_travel - lateral_at_release,
        "world_right_displacement_m": (car.pose(&physics).translation - pose.translation).dot(pose.rotation * Vec3::X),
        "heading_change_deg": heading_delta.to_degrees(),
        "heading_change_after_release_deg": (heading_delta - heading_at_release).to_degrees(),
        "minimum_forward_speed_kmh": min_forward * 3.6,
    }))
}

fn run() -> Result<bool, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cars = Cars::load(args.first().map(String::as_str))?;
    let ids: Vec<&str> = if args.len() > 1 {
        args[1..].iter().map(String::as_str).collect()
    } else {
        vec!["asti", "mv6"]
    };
    let mut results = serde_json::Map::new();
    let mut valid = true;
    for id in ids {
        let source = cars.get(id)?;
        let mut variants = serde_json::Map::new();
        for tuning in [ORIGINAL, CANDIDATE] {
            let mut spec = source.clone();
            apply_tuning(&mut spec, tuning);
            let bench = bench::run_spec(&spec)?;
            valid &= bench.failures.is_empty();
            let mut maneuvers = serde_json::Map::new();
            for maneuver in [Maneuver::LateralImpulse, Maneuver::SteerCountersteer] {
                let result = run_maneuver(&source, tuning, maneuver)?;
                valid &= result["finite"] == true
                    && result["all_four_wheels_in_contact"] == true
                    && !result["recovery"].is_null();
                maneuvers.insert(maneuver.name().into(), result);
            }
            variants.insert(tuning.name.into(), json!({
                "tyre_grip_scale": tuning.grip,
                "lateral_peak_scale": tuning.lateral_peak,
                "maneuvers": maneuvers,
                "existing_bench": { "results": bench.results, "failures": bench.failures, "ticks": bench.ticks },
            }));
        }
        let initial_states_match = [Maneuver::LateralImpulse, Maneuver::SteerCountersteer].iter().all(|m| {
            variants["original"]["maneuvers"][m.name()]["initial_state"]
                == variants["candidate"]["maneuvers"][m.name()]["initial_state"]
        });
        valid &= initial_states_match;
        variants.insert("initial_states_match".into(), json!(initial_states_match));
        results.insert(id.into(), Value::Object(variants));
    }
    println!("{}", serde_json::to_string_pretty(&json!({
        "fixture": {
            "surface": "flat_gravel", "hz": HZ, "tyre_substeps": SUBSTEPS,
            "settle_ticks": SETTLE_TICKS, "maneuver_ticks": RUN_TICKS,
            "initial_forward_kmh": 60.0, "manual_gear": 3, "steering_assist": true,
            "throttle": 0.0, "brake": 0.0, "handbrake": 0.0,
            "initialization": "Settle original spec with brakes for 3 s; keep pose and suspension state; set matched body/wheel/engine speeds; then apply only tyre tuning. Coast in third gear with engine braking.",
            "lateral_impulse": { "body_right_delta_v_mps": IMPULSE_DELTA_V_MPS, "impulse": "Equivalent centre-of-mass impulse mass * delta_v, no initial yaw; wheel slip states start at zero", "duration_s": 5.0 },
            "steer_countersteer": [ { "start_s": 0.0, "duration_s": 0.75, "steer": 0.35 }, { "start_s": 0.75, "duration_s": 0.25, "steer": -0.18 }, { "start_s": 1.0, "duration_s": 4.0, "steer": 0.0 } ],
            "recovery": "After input release: abs(body slip) < 3 deg and abs(yaw rate) < 0.1 rad/s continuously for 0.5 s. Null means not reached within the run; start and confirmation times are both reported.",
            "lateral_travel": "Trapezoidal integral of absolute body-relative lateral speed, not net world displacement",
            "validity": "Requires identical original/candidate initial states, finite simulation, all four wheels in contact throughout both maneuvers, sustained recovery in both maneuvers, and no existing bench failures for either tuning",
        },
        "valid": valid,
        "cars": results,
    })).map_err(|e| e.to_string())?);
    Ok(valid)
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("handling_probe: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(result: &Value, key: &str) -> f64 {
        result[key].as_f64().unwrap_or_else(|| panic!("missing numeric metric {key}"))
    }

    fn compare_car(id: &str) {
        let cars = Cars::load(Some(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/cars.luau"))).unwrap();
        let source = cars.get(id).unwrap();
        // Explicit comparisons stay independent of the current cars.luau tune.
        let original = Tuning { name: "original", grip: 1.0, lateral_peak: 1.0 };
        let candidate = Tuning { name: "candidate", grip: 1.12, lateral_peak: 0.80 };
        for maneuver in [Maneuver::LateralImpulse, Maneuver::SteerCountersteer] {
            let baseline = run_maneuver(&source, original, maneuver).unwrap();
            let tuned = run_maneuver(&source, candidate, maneuver).unwrap();
            assert_eq!(baseline["initial_state"], tuned["initial_state"], "{id} {} initial state", maneuver.name());
            for (name, result) in [("original", &baseline), ("candidate", &tuned)] {
                assert_eq!(result["finite"], true, "{id} {} {name} finite", maneuver.name());
                assert_eq!(result["all_four_wheels_in_contact"], true, "{id} {} {name} wheel contact", maneuver.name());
                assert!(!result["recovery"].is_null(), "{id} {} {name} must recover", maneuver.name());
                assert_eq!(result["completed_ticks"], RUN_TICKS, "{id} {} {name} complete", maneuver.name());
                if matches!(maneuver, Maneuver::SteerCountersteer) {
                    assert!(number(&result["final"], "body_slip_deg").abs() < 3.0,
                        "{id} {name} must finish recentered below 3 degrees body slip");
                    assert!(number(&result["final"], "yaw_rate_rad_s").abs() < 0.1,
                        "{id} {name} must finish recentered below 0.1 rad/s yaw rate");
                    assert!(number(result, "peak_reverse_yaw_rate_after_release_rad_s") <= 0.1,
                        "{id} {name} must not develop reverse yaw above 0.1 rad/s after release");
                }
            }
            if matches!(maneuver, Maneuver::LateralImpulse) {
                assert!(number(&tuned, "body_lateral_travel_m") <= number(&baseline, "body_lateral_travel_m") * 0.95,
                    "{id} candidate must reduce identical-impulse lateral travel by at least 5%");
                assert!(number(&tuned["recovery"], "stable_start_after_release_s")
                    <= number(&baseline["recovery"], "stable_start_after_release_s"),
                    "{id} candidate impulse recovery must not be slower");
            }
        }
    }

    #[test]
    fn asti_recovery_comparison() {
        compare_car("asti");
    }

    #[test]
    fn mv6_recovery_comparison() {
        compare_car("mv6");
    }
}
