//! Owner-only arms with hands from the same fitted Human as the stadium player.
//! Keep the fitted fingers on the sockets and solve the elbows from the camera-relative shoulders.
use crate::{
    gun::{Contacts, Reload},
    party::PlayerColors,
};
use glam::{Affine3A, Mat3, Quat, Vec3};
use render::{
    cluster::{ClusterMesh, Part, Vertex},
    humans::{self, BodyVertex, Constraints, Instance},
    slots::Slots,
    Scene,
};

pub(super) fn roster_person(seed: u64, slot: usize, profile: PlayerColors) -> Instance {
    let identity = seed.wrapping_mul(0x9e3779b97f4a7c15) ^ slot as u64;
    Instance::person(
        identity,
        Vec3::ZERO,
        0.,
        &Constraints {
            height_m: Some(1.80),
            shirt: profile.shirt,
        },
    )
}

pub(super) struct Arms {
    first: usize,
    pub(super) open: [usize; 2],
    seated: [usize; 2],
    grip: [usize; 2],
    reaching: [usize; 2],
    lower: usize,
    upper: usize,
}

impl Arms {
    pub(super) const SLOTS: usize = 6;

    pub fn reserve(scene: &mut Scene, seed: u64, slot: usize, profile: PlayerColors) -> Self {
        let person = roster_person(seed, slot, profile);
        let skin = profile
            .skin
            .unwrap_or_else(|| person.skin_color().to_array());
        let materials = [
            super::material(scene, skin, 0.6),
            super::material(scene, person.shirt_color(), 0.9),
        ];
        let mut pair = |vertices: &[BodyVertex], indices: &[u32], pose| {
            [false, true].map(|left| {
                let mesh = forearm(
                    vertices,
                    indices,
                    materials,
                    person.body_scale(),
                    left,
                    pose,
                );
                scene.add_mesh(&mesh)
            })
        };
        let (relaxed_body, triangles) = person.fitted_mesh(false);
        let seated = pair(&relaxed_body, &triangles, ArmPose::Seated);
        let open = pair(&relaxed_body, &triangles, ArmPose::Flying);
        let reaching = pair(&relaxed_body, &triangles, ArmPose::Hand(false));
        let (gripping_body, triangles) = person.fitted_mesh(true);
        let grip = pair(&gripping_body, &triangles, ArmPose::Hand(true));
        let long_sleeves = relaxed_body
            .iter()
            .any(|v| v.material == 12 && v.position.x > 0.24 && v.limb_weights[0] > 0.75);
        let lower = scene.add_mesh(&arm_segment(
            materials[usize::from(long_sleeves)],
            &[
                (-0.043, 0.),
                (-0.029, 0.037),
                (0.00, 0.048),
                (0.067, 0.055),
                (0.12, 0.050),
                (0.21, 0.038),
                (0.28, 0.031),
                (0.30, 0.027),
                (0.319, 0.),
            ],
        ));
        let upper = scene.add_mesh(&arm_segment(
            materials[1],
            &[
                (-0.065, 0.),
                (-0.04, 0.059),
                (0.015, 0.079),
                (0.10, 0.075),
                (0.21, 0.063),
                (0.285, 0.056),
                (0.30, 0.054),
                (0.315, 0.043),
                (0.34, 0.),
            ],
        ));
        let first = Slots::reserve(scene, Self::SLOTS).first;
        for instance in &mut scene.instances[first..first + Self::SLOTS] {
            instance.hide_in_views(0xff);
            instance.flags |= render::scene::NO_SHADOW;
        }
        Self {
            first,
            open,
            seated,
            grip,
            reaching,
            lower,
            upper,
        }
    }

    pub fn mask(&self, scene: &mut Scene, mask: u8) {
        for instance in &mut scene.instances[self.first..self.first + Self::SLOTS] {
            instance.hide_in_views(mask);
        }
    }

    pub fn clear(&self, scene: &mut Scene) {
        for instance in &mut scene.instances[self.first..self.first + Self::SLOTS] {
            instance.cluster_count = 0;
            instance.sphere = [0.; 4];
        }
    }

    pub fn fly(
        &self,
        scene: &mut Scene,
        upright: Affine3A,
        view: Affine3A,
        flight: f32,
        wrists: [Affine3A; 2],
        in_crowd: bool,
    ) {
        let frame = blend_pose(upright, view, ease(0., 0.15, flight));
        for (side, wrist) in wrists.into_iter().enumerate() {
            if flight >= 0.16 {
                self.pose_arm(scene, side, view, frame * wrist, self.reaching[side]);
                continue;
            }
            let mesh = if in_crowd && flight < 0.04 {
                self.seated[side]
            } else {
                self.open[side]
            };
            scene.update_instance(self.first + side, mesh, frame * wrist);
        }
    }

    pub fn hold(
        &self,
        scene: &mut Scene,
        gun: Affine3A,
        contacts: Contacts,
        reload: &Reload,
        view: Affine3A,
        pickup: Option<f32>,
    ) {
        let mount = view.inverse() * gun;
        let foregrip = mount.transform_point3(contacts.foregrip);
        // A long barrel can carry its foregrip beyond this player's reach.
        // Keep supporting the magazine until the forward socket comes within reach.
        let reachable = 1. - ease(0.64, 0.75, foregrip.distance(shoulder(1)));
        let contacts = Contacts {
            extension: contacts.extension * reachable,
            ..contacts
        };
        for (side, wrist) in grip_points(contacts, reload).into_iter().enumerate() {
            let held = gun * Affine3A::from_translation(wrist);
            let (mesh, pose) = if let Some(age) = pickup {
                // The trigger hand takes the weight first; the other finds the magazine.
                let close = if side == 0 {
                    super::guns::TRIGGER_GRIP
                } else {
                    super::guns::SUPPORT_GRIP
                };
                // A rifle on its side rolls into the grip. The wrist follows its
                // socket, but the elbow still trails back towards the player's body.
                let (_, view_rotation, _) = view.to_scale_rotation_translation();
                let (_, gun_rotation, position) = held.to_scale_rotation_translation();
                let turn = ease(
                    super::guns::SUPPORT_GRIP,
                    super::guns::PICKUP_LIFT_SECONDS,
                    age,
                );
                let socket = Affine3A::from_rotation_translation(
                    view_rotation.slerp(gun_rotation, turn),
                    position,
                );
                let reach = blend_pose(view * reaching_wrist(side), socket, ease(0., close, age));
                (
                    if age < close {
                        self.reaching[side]
                    } else {
                        self.grip[side]
                    },
                    reach,
                )
            } else {
                (self.grip[side], held)
            };
            self.pose_arm(scene, side, view, pose, mesh);
        }
    }

    fn pose_arm(
        &self,
        scene: &mut Scene,
        side: usize,
        view: Affine3A,
        hand: Affine3A,
        mesh: usize,
    ) {
        let wrist = view.inverse().transform_point3(hand.translation.into());
        let sign = if side == 0 { 1. } else { -1. };
        let shoulder = shoulder(side);
        let to_wrist = wrist - shoulder;
        let distance = to_wrist.length();
        let axis = to_wrist.normalize_or(Vec3::NEG_Z);
        let hint = Vec3::new(sign * 0.64, -1., 0.15);
        let outward = (hint - axis * hint.dot(axis)).normalize_or(axis.any_orthonormal_vector());
        // Equal bone lengths give a midpoint solve. At full reach keep a small
        // bend while reaching down to the grounded weapon.
        let half = 0.5 * distance;
        let bend = (0.32_f32.powi(2) - half * half)
            .max(0.045_f32.powi(2))
            .sqrt();
        let elbow = shoulder + axis * half + outward * bend;
        let between = |a: Vec3, b: Vec3| {
            let delta = b - a;
            view * Affine3A::from_scale_rotation_translation(
                Vec3::new(1., delta.length() / 0.30, 1.),
                Quat::from_rotation_arc(Vec3::Y, delta.normalize_or(Vec3::Y)),
                a,
            )
        };
        scene.update_instance(self.first + side, mesh, hand);
        scene.update_instance(self.first + 2 + side, self.lower, between(elbow, wrist));
        scene.update_instance(self.first + 4 + side, self.upper, between(shoulder, elbow));
    }
}

fn shoulder(side: usize) -> Vec3 {
    Vec3::new(if side == 0 { 0.19 } else { -0.19 }, -0.22, 0.075)
}

fn grip_points(contacts: Contacts, reload: &Reload) -> [Vec3; 2] {
    let support = contacts
        .magazine
        .lerp(contacts.foregrip, contacts.extension)
        .lerp(contacts.magazine - Vec3::Y * reload.hand_drop, reload.reach);
    [contacts.primary + Vec3::X * 0.028, support]
}

/// Bring the rifle back along the sightline when a growing grip would straighten an arm.
pub(super) fn within_reach(mut mount: Affine3A, contacts: Contacts, reload: &Reload) -> Affine3A {
    let mut pull = 0.0_f32;
    for (side, point) in grip_points(contacts, reload).into_iter().enumerate() {
        let from_shoulder = mount.transform_point3(point) - shoulder(side);
        let forward_reach = (0.61_f32.powi(2) - from_shoulder.truncate().length_squared())
            .max(0.)
            .sqrt();
        pull = pull.max(-from_shoulder.z - forward_reach);
    }
    // Leave room in front of the lens for the receiver, stock and trigger hand.
    mount.translation.z += pull.min(0.085);
    mount
}

pub(super) fn ease(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

pub(super) fn blend_pose(from: Affine3A, to: Affine3A, t: f32) -> Affine3A {
    let (_, a, p) = from.to_scale_rotation_translation();
    let (_, b, q) = to.to_scale_rotation_translation();
    Affine3A::from_rotation_translation(a.slerp(b, t), p.lerp(q, t))
}

fn reaching_wrist(side: usize) -> Affine3A {
    let sign = if side == 0 { 1. } else { -1. };
    Affine3A::from_rotation_translation(
        Quat::from_euler(glam::EulerRot::YXZ, sign * 0.15, -0.55, -sign * 0.45),
        Vec3::new(0.09 + 0.14 * sign, -0.20, -0.45 - 0.06 * side as f32),
    )
}

// In eye space, +X is the player's right and -Z points into the arena.
// The two arms catch the launch at different times, then make alternating
// broad sweeps before both hands turn down to meet the rifle.
pub(super) fn flight_wrist(
    player: usize,
    side: usize,
    flight: f32,
    seated: Option<Affine3A>,
    eye_height: f32,
) -> Affine3A {
    use std::f32::consts::TAU;
    let sign = if side == 0 { 1. } else { -1. };
    let rest = Affine3A::from_rotation_translation(
        Quat::from_euler(glam::EulerRot::YXZ, -sign * 0.12, -1.08, -sign * 0.22),
        Vec3::new(sign * 0.25, -0.66, -0.08),
    );
    let rest = seated.map_or(rest, |wrist| {
        let (.., rotation, position) = wrist.to_scale_rotation_translation();
        let facing = Quat::from_rotation_y(std::f32::consts::PI);
        Affine3A::from_rotation_translation(
            facing * rotation * palm_rotation(side == 1, false).inverse(),
            facing * (position * 1.8) - Vec3::Y * eye_height,
        )
    });
    let delay = if side == player % 2 { 0.012 } else { 0.047 };
    let throw = ease(delay, delay + 0.17, flight);
    let phase = TAU * (1.85 * flight + 0.43 * side as f32 + 0.10 * (player % 4) as f32);
    let beat = phase.sin();
    let wrist = Vec3::new(
        sign * (0.32 + 0.087 * phase.cos()),
        -0.045 + 0.115 * beat,
        -0.42 - 0.090 * (phase + 0.48).cos(),
    );
    let twist = Quat::from_euler(
        glam::EulerRot::YXZ,
        sign * (0.20 + 0.40 * beat),
        0.35 + 0.50 * beat,
        sign * (0.22 + 0.52 * phase.cos()),
    );
    let flail = blend_pose(
        rest,
        Affine3A::from_rotation_translation(twist, wrist),
        throw,
    );
    blend_pose(
        flail,
        reaching_wrist(side),
        ease(0.73 + delay, 0.985, flight),
    )
}

fn palm_rotation(left: bool, gripping: bool) -> Quat {
    let side = if left { -1. } else { 1. };
    let fingers = if gripping {
        Vec3::NEG_Z
    } else {
        Vec3::new(-side * 0.32, 0.12, -1.).normalize()
    };
    let back = if gripping {
        Vec3::X * side
    } else {
        Vec3::new(side * 0.34, 1., 0.)
    };
    let back = (back - fingers * fingers.dot(back)).normalize();
    Quat::from_mat3(&Mat3::from_cols(fingers, back, fingers.cross(back)))
        * humans::hand_frame(left).inverse()
}

// Closed, overlapping cross sections keep the elbow and wrist covered as they flex.
fn arm_segment(material: u32, profile: &[(f32, f32)]) -> ClusterMesh {
    const SIDES: usize = 16;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (row, &(y, radius)) in profile.iter().enumerate() {
        let (y0, r0) = profile[row.saturating_sub(1)];
        let (y1, r1) = profile[(row + 1).min(profile.len() - 1)];
        for i in 0..SIDES {
            let (sin, cos) = (std::f32::consts::TAU * i as f32 / SIDES as f32).sin_cos();
            let normal = Vec3::new(cos, (r0 - r1) / (y1 - y0), sin / 0.90).normalize();
            vertices.push(Vertex {
                pos: [radius * cos, y, 0.90 * radius * sin],
                normal: normal.into(),
                uv: [0.; 2],
            });
            if row + 1 < profile.len() {
                let a = (row * SIDES + i) as u32;
                let b = (row * SIDES + (i + 1) % SIDES) as u32;
                let n = SIDES as u32;
                if radius > 0. {
                    indices.extend([a, a + n, b]);
                }
                if profile[row + 1].1 > 0. {
                    indices.extend([b, a + n, b + n]);
                }
            }
        }
    }
    ClusterMesh::build(&[Part {
        vertices: &vertices,
        indices: &indices,
        material,
    }])
}

#[derive(Clone, Copy)]
enum ArmPose {
    Seated,
    Flying,
    Hand(bool),
}

fn forearm(
    body: &[BodyVertex],
    triangles: &[u32],
    materials: [u32; 2],
    scale: Vec3,
    left: bool,
    pose: ArmPose,
) -> ClusterMesh {
    let side = if left { -1. } else { 1. };
    let wrist = Vec3::new(-side * 0.259, 0.637, 0.084) * scale;
    let elbow = Vec3::new(-side * 0.188, 0.699, -0.013) * scale;
    let axis = (elbow - wrist).normalize();
    let palm = palm_rotation(left, matches!(pose, ArmPose::Hand(true)));
    let destination = Vec3::new(side * 0.22, -0.28, 0.42);
    let bend = Quat::from_rotation_arc(palm * axis, destination.normalize());
    let mut vertices = Vec::new();
    let mut remap = vec![u32::MAX; body.len()];
    let mut surfaces = [Vec::new(), Vec::new()];
    for tri in triangles.chunks_exact(3) {
        let tag = body[tri[0] as usize].material;
        if matches!(pose, ArmPose::Hand(_)) && tag != 4 {
            continue;
        }
        if !matches!(tag, 4 | 12)
            || !tri.iter().all(|&i| {
                let v = &body[i as usize];
                -side * v.position.x > 0.16 && v.limb_weights[0] > 0.75
            })
        {
            continue;
        }
        if matches!(pose, ArmPose::Hand(_))
            && tri
                .iter()
                .all(|&i| axis.dot(body[i as usize].position * scale - wrist) >= 0.055)
        {
            continue;
        }
        let indices = &mut surfaces[usize::from(tag == 12)];
        for &i in tri {
            let index = &mut remap[i as usize];
            if *index == u32::MAX {
                let v = &body[i as usize];
                let offset = v.position * scale - wrist;
                let distance = axis.dot(offset).max(0.);
                let (pos, orientation) = match pose {
                    ArmPose::Seated => (palm * offset, palm),
                    ArmPose::Hand(_) => {
                        // Taper the small cuff inside the rounded end of the moving forearm.
                        let taper = ease(0.010, 0.055, distance);
                        (palm * offset.lerp(axis * 0.016, taper), palm)
                    }
                    ArmPose::Flying => {
                        let t = distance / wrist.distance(elbow);
                        let blend = ease(0., 0.075, distance);
                        let orientation = Quat::IDENTITY.slerp(bend, blend) * palm;
                        let pos = orientation * (offset - axis * distance)
                            + (palm * axis * distance)
                                .lerp(destination * t, (distance / 0.075).clamp(0., 1.));
                        (pos, orientation)
                    }
                };
                let normal = (orientation * (v.normal / scale)).normalize_or_zero();
                *index = vertices.len() as u32;
                vertices.push(Vertex {
                    pos: pos.into(),
                    normal: normal.into(),
                    uv: v.uv,
                });
            }
            indices.push(*index);
        }
    }
    let parts: Vec<_> = surfaces
        .iter()
        .zip(materials)
        .filter(|(indices, _)| !indices.is_empty())
        .map(|(indices, material)| Part {
            vertices: &vertices,
            indices,
            material,
        })
        .collect();
    ClusterMesh::build(&parts)
}
