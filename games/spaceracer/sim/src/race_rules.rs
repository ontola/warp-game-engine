//! Spatial race rules, translated from checkpoints.gd, slipstream.gd and bump.gd.
use crate::{
    handling::{Pilot, smooth, toward},
    track::Node,
};
use glam::{Mat3, Quat, Vec3};
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub origin: Vec3,
    pub basis: Mat3,
}
impl Pose {
    pub fn local(&self, p: Vec3) -> Vec3 {
        self.basis.inverse() * (p - self.origin)
    }
}
pub fn ground_pose(p: &Pilot, n: &Node, clock: f64, landing: Option<(Mat3, f32)>) -> Pose {
    let mut basis = n.frame(p.x)
        * Mat3::from_rotation_y(-p.heading as f32)
        * Mat3::from_rotation_z((-p.slip * 0.004) as f32)
        * Mat3::from_rotation_x((p.trim * if p.trim < 0.0 { 0.32 } else { 0.16 }) as f32);
    basis = basis
        * Mat3::from_rotation_x((-p.unload * 0.08) as f32)
        * Mat3::from_rotation_z((p.unload * 0.018 * (clock * 19.0 + p.slot as f64).sin()) as f32);
    if let Some((frame, blend)) = landing {
        if blend > 0.0 {
            basis = Mat3::from_quat(
                Quat::from_mat3(&basis)
                    .slerp(Quat::from_mat3(&frame), blend)
                    .normalize(),
            );
        }
    }
    Pose {
        origin: n.point(p.x, 1.3 + (-p.trim).max(0.0) * 0.45 + p.lift),
        basis,
    }
}
pub fn eligible(p: &Pilot) -> bool {
    !p.airborne
        && !p.crashed
        && !p.finished
        && p.recovery <= 0.0
        && p.warp_time <= 0.0
        && p.emp_time <= 0.0
}
/// All poses must be captured before any pilot moves this tick.
pub fn slipstream(
    pilots: &mut [Pilot],
    poses: &[Pose],
    length: f64,
    countdown: f64,
    over: bool,
    dt: f64,
) {
    assert_eq!(pilots.len(), poses.len());
    if countdown > 0.0 || over || pilots.len() < 2 {
        for p in pilots {
            p.slipstream = 0.0;
        }
        return;
    }
    for i in 0..pilots.len() {
        if !eligible(&pilots[i]) {
            pilots[i].slipstream = 0.0;
            continue;
        }
        let mut strongest = 0.0_f64;
        for j in 0..pilots.len() {
            if i == j || !eligible(&pilots[j]) {
                continue;
            }
            let gap = (pilots[j].distance - pilots[i].distance).rem_euclid(length);
            if !(10.0..=145.0).contains(&gap) {
                continue;
            }
            let local = poses[j].local(poses[i].origin);
            let behind = -local.z as f64;
            if !(10.0..=145.0).contains(&behind) || local.y.abs() > 5.0 {
                continue;
            }
            let alignment = poses[i].basis.z_axis.dot(poses[j].basis.z_axis) as f64;
            let lateral = 1.0 - smooth(2.0, 6.0 + behind * 0.035, local.x.abs() as f64);
            let fade = smooth(10.0, 20.0, behind) * (1.0 - smooth(45.0, 145.0, behind));
            let speed = smooth(100.0, 220.0, pilots[i].speed.min(pilots[j].speed));
            strongest = strongest.max(lateral * fade * speed * smooth(0.85, 0.98, alignment));
        }
        let p = &mut pilots[i];
        p.slipstream = toward(
            p.slipstream,
            strongest,
            dt * if strongest > p.slipstream { 2.0 } else { 3.5 },
        );
    }
}
#[derive(Clone, Debug)]
pub struct Gate {
    pub pose: Pose,
    pub width: f64,
    pub height: f64,
    pub distance: f64,
    pub respawn: f64,
}
#[derive(Clone, Debug, Default)]
pub struct Checkpoints {
    pub index: usize,
    pub missed: bool,
    pub flash: f64,
}
impl Checkpoints {
    pub fn advance(
        &mut self,
        p: &mut Pilot,
        gates: &[Gate],
        length: f64,
        before: Option<Vec3>,
        to: Vec3,
    ) {
        let Some(gate) = gates.get(self.index) else {
            return;
        };
        let Some(from) = before else { return };
        if p.crashed || p.recovery > 0.0 {
            return;
        }
        let a = gate.pose.local(from);
        let b = gate.pose.local(to);
        if a.z >= 0.0 || b.z < 0.0 || from.distance(to) > 120.0 {
            return;
        }
        let crossing = a.lerp(b, -a.z / (b.z - a.z));
        if crossing.x.abs() as f64 > gate.width
            || crossing.y < -4.0
            || crossing.y as f64 > gate.height
        {
            return;
        }
        self.index += 1;
        self.missed = false;
        self.flash = 0.7;
        if p.airborne {
            p.distance = p
                .distance
                .max((p.lap as f64 - 1.0) * length + gate.distance);
        }
    }
    pub fn progress(&self, p: &Pilot, gates: &[Gate], length: f64) -> f64 {
        let limit = gates.get(self.index).map_or(p.lap as f64 * length, |g| {
            (p.lap as f64 - 1.0) * length + g.distance
        });
        p.distance.min(limit)
    }
    pub fn return_distance(&self, p: &Pilot, gates: &[Gate], length: f64) -> Option<f64> {
        gates
            .get(self.index.min(gates.len().saturating_sub(1)))
            .map(|g| (p.lap as f64 - 1.0) * length + g.respawn)
    }
}
#[derive(Clone, Debug, Default)]
pub struct Bump {
    pub time: f64,
    pub cooldown: f64,
    pub guard: f64,
    pub side: f64,
    pub left_held: bool,
    pub right_held: bool,
    pub hits: Vec<usize>,
}
impl Bump {
    pub fn begin(&mut self, p: &Pilot, left: bool, right: bool, dt: f64, countdown: f64) {
        self.time = (self.time - dt).max(0.0);
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.guard = (self.guard - dt).max(0.0);
        let side = (right && !self.right_held) as i32 - (left && !self.left_held) as i32;
        self.left_held = left;
        self.right_held = right;
        if !eligible(p) {
            self.time = 0.0;
            return;
        }
        if countdown > 0.0 || self.cooldown > 0.0 || side == 0 || (left && right) {
            return;
        }
        self.side = side as f64;
        self.time = 0.24;
        self.cooldown = 0.9;
        self.hits.clear();
    }
    pub fn velocity(&self) -> f64 {
        if self.time <= 0.0 {
            0.0
        } else {
            self.side * ((1.0 - self.time / 0.24) * std::f64::consts::PI).sin() * 85.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_drafting_and_side_bump_sequences() {
        let data: serde_json::Value = serde_json::from_str(&crate::oracle("rules")).unwrap();
        for c in data["slipstream"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let length = c["length"].as_f64().unwrap();
            let mut pilots = vec![
                Pilot {
                    distance: 300.0,
                    speed: 250.0,
                    ..Default::default()
                },
                Pilot {
                    distance: 340.0,
                    speed: 240.0,
                    ..Default::default()
                },
            ];
            if name == "lapped" {
                pilots[1].distance += length;
            }
            if name == "blocked" {
                pilots[1].emp_time = 2.0;
            }
            let poses: Vec<Pose> = c["poses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| Pose {
                    origin: serde_json::from_value(p["origin"].clone()).unwrap(),
                    basis: Mat3::from_cols_array_2d(
                        &serde_json::from_value(p["basis"].clone()).unwrap(),
                    ),
                })
                .collect();
            for (tick, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                slipstream(
                    &mut pilots,
                    &poses,
                    length,
                    if name == "countdown" { 1.0 } else { 0.0 },
                    false,
                    1.0 / 120.0,
                );
                for i in 0..2 {
                    assert!(
                        (pilots[i].slipstream - f[i].as_f64().unwrap()).abs() < 0.00001,
                        "{name} {tick} seat {i}"
                    );
                }
            }
        }
        for c in data["bump"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let p = Pilot {
                airborne: name == "disabled",
                ..Default::default()
            };
            let mut bump = Bump::default();
            for (tick, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                bump.begin(
                    &p,
                    f["left"].as_bool().unwrap(),
                    f["right"].as_bool().unwrap(),
                    1.0 / 120.0,
                    if name == "countdown" { 1.0 } else { 0.0 },
                );
                for (key, v) in [
                    ("time", bump.time),
                    ("cooldown", bump.cooldown),
                    ("side", bump.side),
                    ("velocity", bump.velocity()),
                ] {
                    assert!(
                        (v - f[key].as_f64().unwrap()).abs() < 0.000001,
                        "{name} {tick} {key}"
                    );
                }
            }
        }
    }
    #[test]
    fn gates_reject_reverse_crossings_teleports_and_shortcuts() {
        let gate = Gate {
            pose: Pose {
                origin: Vec3::ZERO,
                basis: Mat3::IDENTITY,
            },
            width: 30.0,
            height: 32.0,
            distance: 1000.0,
            respawn: 900.0,
        };
        let gates = [gate];
        let mut c = Checkpoints::default();
        let mut p = Pilot {
            airborne: true,
            distance: 850.0,
            ..Default::default()
        };
        for (from, to) in [
            (Vec3::new(0.0, 2.0, 1.0), Vec3::new(0.0, 2.0, -1.0)),
            (Vec3::new(31.0, 2.0, -1.0), Vec3::new(31.0, 2.0, 1.0)),
            (Vec3::new(0.0, 40.0, -1.0), Vec3::new(0.0, 40.0, 1.0)),
            (Vec3::new(0.0, 2.0, -100.0), Vec3::new(0.0, 2.0, 100.0)),
        ] {
            c.advance(&mut p, &gates, 2000.0, Some(from), to);
            assert_eq!(c.index, 0);
        }
        c.advance(
            &mut p,
            &gates,
            2000.0,
            Some(Vec3::new(0.0, 2.0, -1.0)),
            Vec3::new(0.0, 2.0, 1.0),
        );
        assert_eq!(c.index, 1);
        assert_eq!(p.distance, 1000.0);
        assert_eq!(c.flash, 0.7);
    }
}
