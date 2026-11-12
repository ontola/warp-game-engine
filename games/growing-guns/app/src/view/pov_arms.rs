//! Owner-only forearms cut from the same fitted Human as the stadium player.
//! Fit the two wrist poses once. During play they ride the camera or the actual rifle sockets.
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
    open: [usize; 2],
    seated: [usize; 2],
    grip: [usize; 2],
}

impl Arms {
    pub fn reserve(scene: &mut Scene, seed: u64, slot: usize, profile: PlayerColors) -> Self {
        let person = roster_person(seed, slot, profile);
        let skin = profile
            .skin
            .unwrap_or_else(|| person.skin_color().to_array());
        let materials = [
            super::material(scene, skin, 0.6),
            super::material(scene, person.shirt_color(), 0.9),
        ];
        let mut pair = |vertices: &[BodyVertex], indices: &[u32], gripping, seated| {
            [false, true].map(|left| {
                let mesh = forearm(
                    vertices,
                    indices,
                    materials,
                    person.body_scale(),
                    left,
                    gripping,
                    seated,
                );
                scene.add_mesh(&mesh)
            })
        };
        let (relaxed_body, triangles) = person.fitted_mesh(false);
        let seated = pair(&relaxed_body, &triangles, false, true);
        let open = pair(&relaxed_body, &triangles, false, false);
        let (gripping_body, triangles) = person.fitted_mesh(true);
        let grip = pair(&gripping_body, &triangles, true, false);
        let first = Slots::reserve(scene, 2).first;
        for instance in &mut scene.instances[first..first + 2] {
            instance.hide_in_views(0xff);
            instance.flags |= render::scene::NO_SHADOW;
        }
        Self {
            first,
            open,
            seated,
            grip,
        }
    }

    pub fn mask(&self, scene: &mut Scene, mask: u8) {
        for instance in &mut scene.instances[self.first..self.first + 2] {
            instance.hide_in_views(mask);
        }
    }

    pub fn clear(&self, scene: &mut Scene) {
        for instance in &mut scene.instances[self.first..self.first + 2] {
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
        pickup: Option<(f32, Affine3A)>,
    ) {
        let support = contacts
            .magazine
            .lerp(contacts.foregrip, contacts.extension)
            .lerp(contacts.magazine - Vec3::Y * reload.view_drop, reload.reach);
        for (side, wrist) in [contacts.primary + Vec3::X * 0.028, support]
            .into_iter()
            .enumerate()
        {
            let held = gun * Affine3A::from_translation(wrist);
            let (mesh, pose) = if let Some((age, view)) = pickup {
                // The trigger hand takes the weight first; the other finds the magazine.
                let close = if side == 0 { 0.095 } else { 0.15 };
                // A rifle on its side rolls into the grip. The wrist follows its
                // socket, but the elbow still trails back towards the player's body.
                let (_, view_rotation, _) = view.to_scale_rotation_translation();
                let (_, gun_rotation, position) = held.to_scale_rotation_translation();
                let turn = ease(0.19, 0.40, age);
                let socket = Affine3A::from_rotation_translation(
                    view_rotation.slerp(gun_rotation, turn),
                    position,
                );
                let reach = blend_pose(view * reaching_wrist(side), socket, ease(0., close, age));
                (
                    if age < close {
                        self.open[side]
                    } else {
                        self.grip[side]
                    },
                    reach,
                )
            } else {
                (self.grip[side], held)
            };
            scene.update_instance(self.first + side, mesh, pose);
        }
    }
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

/// Keep the original fingers, wrist contour and shirt silhouette. Fit the
/// elbow towards the bottom of the view while preserving forearm cross sections.
fn forearm(
    body: &[BodyVertex],
    triangles: &[u32],
    materials: [u32; 2],
    scale: Vec3,
    left: bool,
    gripping: bool,
    seated: bool,
) -> ClusterMesh {
    let side = if left { -1. } else { 1. };
    let wrist = Vec3::new(-side * 0.259, 0.637, 0.084) * scale;
    let elbow = Vec3::new(-side * 0.188, 0.699, -0.013) * scale;
    let axis = (elbow - wrist).normalize();
    let palm = palm_rotation(left, gripping);
    // When holding the rifle, keep a human-length forearm and fold the sleeve
    // back from the elbow towards the shoulder. The palm stays on its socket.
    let shoulder = Vec3::new(-side * 0.1008, 0.805, -0.012) * scale;
    let upper_axis = (shoulder - elbow).normalize();
    let destination = if gripping {
        let direction = if left {
            Vec3::new(-0.28, -0.19, 0.06)
        } else {
            Vec3::new(0.04, -0.19, 0.21)
        };
        direction.normalize() * wrist.distance(elbow)
    } else {
        Vec3::new(side * 0.22, -0.28, 0.42)
    };
    let bend = Quat::from_rotation_arc(palm * axis, destination.normalize());
    let upper_direction = Vec3::new(if left { -0.19 } else { -0.095 }, 0.075, 0.26).normalize();
    let upper = Quat::from_rotation_arc(bend * palm * upper_axis, upper_direction) * bend * palm;
    let mut vertices = Vec::new();
    let mut remap = vec![u32::MAX; body.len()];
    let mut surfaces = [Vec::new(), Vec::new()];
    for tri in triangles.chunks_exact(3) {
        let tag = body[tri[0] as usize].material;
        if !matches!(tag, 4 | 12)
            || !tri.iter().all(|&i| {
                let v = &body[i as usize];
                -side * v.position.x > (if gripping { 0.087 } else { 0.16 })
                    && v.limb_weights[0] > 0.75
            })
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
                let t = distance / wrist.distance(elbow);
                let blend = (distance / 0.075).clamp(0., 1.);
                let orientation =
                    Quat::IDENTITY.slerp(bend, blend * blend * (3. - 2. * blend)) * palm;
                let pos = orientation * (offset - axis * distance)
                    + (palm * axis * distance).lerp(destination * t, blend);
                // In the stands the whole forearm follows the spectator's actual elbow.
                let (pos, orientation) = if seated {
                    (palm * offset, palm)
                } else if gripping {
                    let sleeve = 1. - v.limb_weights[1];
                    let upper_pos = destination + upper * (v.position * scale - elbow);
                    (
                        pos.lerp(upper_pos, sleeve),
                        orientation.slerp(upper, sleeve),
                    )
                } else {
                    (pos, orientation)
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
