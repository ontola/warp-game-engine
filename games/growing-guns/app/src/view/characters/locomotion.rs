//! Lower-body captures for Warp Humans. The simulation owns translation and floor contact.
use assets::skinned::Character;
use body::{Bone, Bound, Layer, Library};
use glam::{Mat4, Quat, Vec3};
use growing_guns::DT;
use std::f32::consts::{PI, TAU};

const LEGS: [(Bone, &str); 9] = [
    (Bone::Pelvis, "Hips"),
    (Bone::ThighL, "LeftUpLeg"),
    (Bone::CalfL, "LeftLeg"),
    (Bone::FootL, "LeftFoot"),
    (Bone::BallL, "LeftToeBase"),
    (Bone::ThighR, "RightUpLeg"),
    (Bone::CalfR, "RightLeg"),
    (Bone::FootR, "RightFoot"),
    (Bone::BallR, "RightToeBase"),
];

struct Cycle {
    clip: usize,
    length: f32,
    contact: f32,
    metres: f32,
}

impl Cycle {
    fn fit(lib: &Library, rig: &Bound, clip: usize) -> Result<Self, String> {
        let length = lib.get(clip).length;
        let toe = Bone::BallL as usize;
        let foot = |phase: f32| {
            let layer = Layer {
                clip,
                t: phase * length,
                looped: true,
                w: 1.,
            };
            let pose = rig.pose(lib, &[layer]);
            pose[toe].transform_point3(rig.rest[toe].unwrap()).z
                - pose[Bone::Pelvis as usize]
                    .transform_point3(rig.rest[Bone::Pelvis as usize].unwrap())
                    .z
        };
        // Align every loop at the same event: the left toe passes under the hips
        // on its backward (support) stroke. dz / dphase is metres per full cycle.
        let mut prev = foot(0.);
        for i in 1..=96 {
            let next = foot(i as f32 / 96.);
            if prev >= 0. && next < 0. {
                let contact = ((i - 1) as f32 + prev / (prev - next)) / 96.;
                let width = 1. / 48.;
                let metres = (foot(contact - width) - foot(contact + width)) / (2. * width);
                if metres > 0.1 {
                    return Ok(Self {
                        clip,
                        length,
                        contact,
                        metres,
                    });
                }
            }
            prev = next;
        }
        Err("no support stroke in Warp Human locomotion clip".into())
    }
    fn layer(&self, phase: f32, w: f32) -> Layer {
        Layer {
            clip: self.clip,
            t: (self.contact + phase) * self.length,
            looped: true,
            w,
        }
    }
}

pub(super) struct Locomotion {
    lib: Library,
    rig: Bound,
    joints: [usize; 9],
    cycles: [Cycle; 3],
    grip: Vec3,
    pub(super) palms: [Quat; 2],
    neck: (usize, Vec3),
    head: (usize, Vec3),
    idle: usize,
    launch: usize,
    land: usize,
}

#[derive(Default, Clone, Copy)]
struct Sample {
    phase: f32,
    speed: f32,
    turn: f32,
    air: f32,
    land: f32,
    flight_time: f32,
    land_time: f32,
}
impl Sample {
    fn between(self, next: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            phase: mix(self.phase, next.phase),
            speed: mix(self.speed, next.speed),
            turn: mix(self.turn, next.turn),
            air: mix(self.air, next.air),
            land: mix(self.land, next.land),
            flight_time: mix(self.flight_time, next.flight_time),
            land_time: mix(self.land_time, next.land_time),
        }
    }
}

#[derive(Default)]
pub(super) struct Stride {
    from: Sample,
    to: Sample,
    last_pos: Option<Vec3>,
    grounded: bool,
    backwards: bool,
    lift_speed: f32,
}
fn ramp(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
// Shorten the low-speed step as well as slowing its cadence.
fn movement(speed: f32) -> f32 {
    (speed / 0.85).clamp(0., 1.).sqrt()
}
fn weights(speed: f32) -> [f32; 3] {
    let jog = ramp(1.45, 4.8, speed);
    let sprint = ramp(6., 10., speed);
    [1. - jog, jog - sprint, sprint]
}

impl Locomotion {
    pub(super) fn bind(model: &Character) -> Result<Self, String> {
        let lib = Library::builtin()?;
        let find = |name: &str| {
            lib.clip(name)
                .ok_or_else(|| format!("missing locomotion capture {name}"))
        };
        let mut spots = Vec::new();
        let mut joints = [0; 9];
        for (slot, (bone, name)) in LEGS.iter().enumerate() {
            let (i, (_, bind)) = model
                .joints()
                .enumerate()
                .find(|(_, (n, _))| *n == *name)
                .ok_or_else(|| format!("missing Warp Human joint {name}"))?;
            joints[slot] = i;
            spots.push((*bone, bind.w_axis.truncate().to_array()));
        }
        let rig = Bound::from_joints(&spots);
        let cycles =
            ["Walk_Loop", "Jog_Fwd_Loop", "Sprint_Loop"].map(|n| Cycle::fit(&lib, &rig, find(n)?));
        let [walk, jog, sprint] = cycles;
        let pivot = |name| {
            model
                .joints()
                .enumerate()
                .find(|(_, (n, _))| *n == name)
                .map(|(i, (_, bind))| (i, bind.w_axis.truncate()))
                .ok_or_else(|| format!("missing {name}"))
        };
        let grip = pivot("Spine2")?.1 + Vec3::new(-0.10, -0.035, 0.23);
        Ok(Self {
            grip,
            palms: [false, true].map(|left| {
                let back = if left { Vec3::X } else { Vec3::NEG_X };
                let grip = Quat::from_mat3(&glam::Mat3::from_cols(Vec3::Z, back, Vec3::Z.cross(back)));
                grip * render::humans::hand_frame(left).inverse()
            }),
            neck: pivot("Neck")?,
            head: pivot("Head")?,
            idle: find("Idle_Loop")?,
            launch: find("Jump_Start")?,
            land: find("Jump_Land")?,
            cycles: [walk?, jog?, sprint?],
            joints,
            rig,
            lib,
        })
    }

    /// The fitted eyes face forward in head space. Split the pitch between the
    /// neck and head, using the same interpolated look as the gun.
    pub(super) fn look(&self, palette: &mut [Mat4], pitch: f32) {
        let angle = -pitch.clamp(-0.85, 0.85);
        let about = |p: Vec3, fraction| {
            let q = Quat::from_rotation_x(angle * fraction);
            Mat4::from_rotation_translation(q, p - q * p)
        };
        let (n, neck) = self.neck;
        let (h, head) = self.head;
        let nod = about(palette[n].transform_point3(neck), 0.35);
        palette[n] = nod * palette[n];
        palette[h] = nod * palette[h];
        palette[h] = about(palette[h].transform_point3(head), 0.65) * palette[h];
    }

    pub(super) fn advance(
        &self,
        s: &mut Stride,
        pos: Vec3,
        yaw: f32,
        vy: f32,
        ground: bool,
        jumped: bool,
    ) {
        let previous = s.last_pos.replace(pos);
        let delta = pos - previous.unwrap_or(pos);
        // Use the same discontinuity guard as the interpolated game root.
        let teleport = previous.is_none() || delta.length_squared() >= 36.;
        if teleport {
            *s = Stride {
                last_pos: Some(pos),
                grounded: ground,
                lift_speed: vy.max(1.),
                ..Default::default()
            };
        }
        let horizontal = if teleport {
            Vec3::ZERO
        } else {
            delta.with_y(0.)
        };
        let distance = horizontal.length();
        s.from = s.to;
        let p = &mut s.to;
        // Rebase both ends together to preserve interpolation across the loop seam.
        s.from.phase = s.from.phase.rem_euclid(1.);
        p.phase = s.from.phase;
        let speed = distance / DT;
        // Ease the change of pose at starts and stops. Distance still controls
        // the phase, including while the proportions of the blend are changing.
        p.speed += (speed - p.speed) * (1. - (-DT / 0.055).exp());
        if distance > 0.00001 {
            let local = Quat::from_rotation_y(-yaw - PI) * (horizontal / distance);
            if local.z.abs() > 0.25 {
                s.backwards = local.z < 0.;
            }
            let sign = if s.backwards { -1. } else { 1. };
            let angle = (sign * local.x).atan2(sign * local.z);
            let change = (angle - p.turn + PI).rem_euclid(TAU) - PI;
            p.turn += change.clamp(-14. * DT, 14. * DT);
            if ground {
                let metres: f32 = self
                    .cycles
                    .iter()
                    .zip(weights(p.speed))
                    .map(|(c, w)| c.metres * w)
                    .sum();
                p.phase += sign * distance / (metres * movement(p.speed));
            } else if vy <= 0. && p.air + p.land > 0.98 && p.speed > 0.5 {
                // Prepare the nearest heel strike while the legs are tucked.
                let before_midstance = sign * 0.055;
                p.phase = ((p.phase + before_midstance) * 2.).round() * 0.5 - before_midstance;
            }
        }
        if !ground {
            if jumped || s.grounded {
                s.lift_speed = vy.max(1.);
            }
            let airborne = if s.grounded {
                DT * 15.
            } else {
                (p.air + p.land + DT * 15.).min(1.)
            };
            // Frame 3 is toe-off, frame 9 the tucked flight pose. During descent
            // approach the landing contact pose; compression waits for actual contact.
            let climb = (1. - vy.max(0.) / s.lift_speed.max(1.)).clamp(0., 1.);
            let target_time = 0.10 + 0.22 * climb;
            p.flight_time = if s.grounded {
                target_time
            } else {
                p.flight_time + (target_time - p.flight_time).clamp(-DT * 2., DT * 2.)
            };
            let extension = airborne * ramp(0., s.lift_speed.max(3.) * 0.8, -vy);
            p.land = (p.land + (extension - p.land).clamp(-DT * 9., DT * 9.)).min(airborne);
            p.air = airborne - p.land;
            p.land_time = 0.;
        } else {
            if !s.grounded && !teleport {
                p.land_time = 0.;
                p.land = 1.;
            }
            p.air = 0.;
            if p.land > 0. {
                p.land_time += DT * 1.8;
                p.land = 1. - ramp(0.15, 0.70, p.land_time);
            }
        }
        s.grounded = ground;
    }

    pub(super) fn pose(&self, s: &Stride, alpha: f32, idle: f32, palette: &mut [Mat4]) -> Mat4 {
        let p = s.from.between(s.to, alpha);
        let ground = (1. - p.air - p.land).max(0.);
        let moving = movement(p.speed);
        // At running speed the supporting foot must immediately track the moving
        // root after touchdown. A standing landing can use the full knee bend.
        let running_landing = p.land * ramp(0.5, 4.5, p.speed) * 0.94;
        let stride_weight = moving * ground + running_landing;
        let gait = weights(p.speed).map(|w| w * stride_weight);
        let layers = [
            Layer {
                clip: self.idle,
                t: idle,
                looped: true,
                w: (1. - moving) * ground,
            },
            self.cycles[0].layer(p.phase, gait[0]),
            self.cycles[1].layer(p.phase, gait[1]),
            self.cycles[2].layer(p.phase, gait[2]),
            Layer {
                clip: self.launch,
                t: p.flight_time,
                looped: false,
                w: p.air,
            },
            Layer {
                clip: self.land,
                t: p.land_time,
                looped: false,
                w: p.land - running_landing,
            },
        ];
        let pose = self.rig.pose(&self.lib, &layers);
        let pelvis = pose[Bone::Pelvis as usize];
        palette.fill(pelvis);
        let turn = Quat::from_rotation_y(p.turn * stride_weight);
        for side in [1, 5] {
            let hip_bone = LEGS[side].0 as usize;
            let hip = pose[hip_bone].transform_point3(self.rig.rest[hip_bone].unwrap());
            let steer = Mat4::from_rotation_translation(turn, hip - turn * hip);
            for j in side..side + 4 {
                palette[self.joints[j]] = steer * pose[LEGS[j].0 as usize];
            }
            let resting = (1. - moving) * ground;
            if resting > 0. {
                let at = |j: usize| {
                    palette[self.joints[j]]
                        .transform_point3(self.rig.rest[LEGS[j].0 as usize].unwrap())
                };
                let knee = at(side + 1);
                let ankle = at(side + 2);
                let sign = if side == 1 { 1. } else { -1. };
                let middle = pelvis.transform_point3(self.rig.rest[Bone::Pelvis as usize].unwrap()).x;
                let target = ankle.lerp(ankle.with_x(middle + sign * 0.15), resting);
                // Pull the ankle inward along the floor. Bend the knee forward
                // to preserve both segment lengths and leave the shoe level.
                let upper = knee - hip;
                let lower = ankle - knee;
                let ray = (target - hip).normalize();
                let d = target.distance(hip).clamp(
                    (upper.length() - lower.length()).abs() + 1e-5,
                    upper.length() + lower.length(),
                );
                let foot = hip + ray * d;
                let along = (upper.length_squared() - lower.length_squared() + d * d) / (2. * d);
                let natural = upper - ray * upper.dot(ray);
                let forward = Vec3::Z - ray * ray.z;
                let bend = natural
                    .normalize_or(forward)
                    .lerp(forward, resting)
                    .normalize_or(Vec3::Z);
                let joint = hip
                    + ray * along
                    + bend * (upper.length_squared() - along * along).max(0.).sqrt();
                for (j, from, to, a, b) in [
                    (side, hip, hip, upper, joint - hip),
                    (side + 1, knee, joint, lower, foot - joint),
                ] {
                    let q = Quat::from_rotation_arc(a.normalize(), b.normalize());
                    palette[self.joints[j]] =
                        Mat4::from_rotation_translation(q, to - q * from) * palette[self.joints[j]];
                }
                let slide = Mat4::from_translation(foot - ankle);
                for &j in &self.joints[side + 2..side + 4] {
                    palette[j] = slide * palette[j];
                }
            }
        }
        pelvis * Mat4::from_translation(self.grip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn setup() -> (Character, Locomotion) {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/characters/knight.json");
        let model = Character::load(&p).unwrap();
        let rig = Locomotion::bind(&model).unwrap();
        (model, rig)
    }

    #[test]
    fn idle_stance_keeps_the_soles_down_and_the_knees_bent() {
        let (model, rig) = setup();
        let mut palette = vec![Mat4::IDENTITY; model.joint_count];
        for t in [0., 0.3, 0.8] {
            rig.pose(&Stride::default(), 1., t, &mut palette);
            let phase = rig.rig.pose(
                &rig.lib,
                &[Layer {
                    clip: rig.idle,
                    t,
                    looped: true,
                    w: 1.,
                }],
            );
            let joint = |b: usize| {
                palette[rig.joints[b]].transform_point3(rig.rig.rest[LEGS[b].0 as usize].unwrap())
            };
            let spacing = joint(3).x - joint(7).x;
            assert!((spacing - 0.30).abs() < 0.015, "ankles {spacing} m apart");
            for side in [1, 5] {
                let id = LEGS[side + 2].0 as usize;
                let floor = phase[id].transform_point3(rig.rig.rest[id].unwrap()).y;
                assert!((joint(side + 2).y - floor).abs() < 0.003);
                assert!(joint(side + 1).z > joint(side).z.min(joint(side + 2).z));
            }
        }
    }

    #[test]
    fn support_foot_matches_root_distance_in_both_directions() {
        let (model, rig) = setup();
        let mut palette = model.sample(super::super::IDLE, 0., true).unwrap();
        for (cycle, speed) in rig.cycles.iter().zip([1.2, 5., 14.]) {
            for (dir, turn, sign) in [
                (Vec3::Z, 0., 1.),
                (Vec3::NEG_Z, 0., -1.),
                (Vec3::X, PI / 2., 1.),
            ] {
                let mut s = Stride::default();
                let d = 0.015;
                let mut foot = |phase: f32, land: f32| {
                    s.to = Sample {
                        phase,
                        land,
                        speed,
                        turn,
                        ..Default::default()
                    };
                    s.from = s.to;
                    rig.pose(&s, 1., 0., &mut palette);
                    palette[rig.joints[4]]
                        .transform_point3(rig.rig.rest[Bone::BallL as usize].unwrap())
                };
                let start = foot(-sign * d / cycle.metres, 0.);
                let end = foot(sign * d / cycle.metres, 0.);
                assert!(
                    ((end - start).dot(dir) + 2. * d).abs() < 0.006,
                    "speed {speed}, dir {dir:?}"
                );
                if speed == 14. && sign > 0. {
                    let start = foot(-d / cycle.metres, 1.);
                    let end = foot(d / cycle.metres, 1.);
                    assert!(
                        ((end - start).dot(dir) + 2. * d).abs() < 0.008,
                        "running landing {dir:?}"
                    );
                }
            }
            println!("{} m/s test, {:.2} metres per cycle", speed, cycle.metres);
        }
    }

    #[test]
    fn phase_follows_displacement_and_landing_follows_contact() {
        let (_, rig) = setup();
        let mut s = Stride::default();
        let pos = Vec3::ZERO;
        rig.advance(&mut s, pos, 0., 0., true, false);
        s.to.speed = 14.;
        let next = pos + Vec3::NEG_Z * 14. * DT;
        rig.advance(&mut s, next, 0., 0., true, false);
        assert!(((s.to.phase - s.from.phase) * rig.cycles[2].metres - 14. * DT).abs() < 1e-5);
        rig.advance(&mut s, next, 0., 0., true, false);
        assert_eq!(s.to.phase, s.from.phase);
        rig.advance(&mut s, Vec3::X * 30., 0., 0., true, false);
        assert_eq!(s.to.phase, 0.);
        for v in [9., 7., 4., 2., 0., -2., -4., -8.] {
            rig.advance(&mut s, Vec3::X * 30., 0., v, false, v == 9.);
            assert_eq!(s.to.land_time, 0.);
            assert_eq!(s.to.phase, s.from.phase);
        }
        assert!(s.to.land > 0.);
        rig.advance(&mut s, Vec3::X * 30., 0., 8., false, true);
        assert!(s.to.air > s.from.air);
        rig.advance(&mut s, Vec3::X * 30., 0., 0., true, false);
        assert!(s.to.land_time > 0.);
        for _ in 0..30 {
            rig.advance(&mut s, Vec3::X * 30., 0., 0., true, false);
        }
        assert_eq!(s.to.land, 0.);
    }
}
