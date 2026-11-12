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
        let mut pair = |gripping| {
            let (vertices, indices) = person.fitted_mesh(gripping);
            [false, true].map(|left| {
                let mesh = forearm(
                    &vertices,
                    &indices,
                    materials,
                    person.body_scale(),
                    left,
                    gripping,
                );
                scene.add_mesh(&mesh)
            })
        };
        let open = pair(false);
        let grip = pair(true);
        let first = Slots::reserve(scene, 2).first;
        for instance in &mut scene.instances[first..first + 2] {
            instance.hide_in_views(0xff);
            instance.flags |= render::scene::NO_SHADOW;
        }
        Self { first, open, grip }
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

    pub fn brace(&self, scene: &mut Scene, view: Affine3A, flight: f32) {
        let rise = (flight * std::f32::consts::PI).sin();
        for (side, sign) in [1., -1.].into_iter().enumerate() {
            let wrist = Vec3::new(
                sign * (0.27 + 0.04 * rise),
                -0.285 + 0.105 * rise,
                -0.47 - 0.05 * rise,
            );
            let lift = Quat::from_rotation_x(0.08 + 0.38 * rise);
            scene.update_instance(
                self.first + side,
                self.open[side],
                view * Affine3A::from_rotation_translation(lift, wrist),
            );
        }
    }

    pub fn hold(&self, scene: &mut Scene, gun: Affine3A, contacts: Contacts, reload: &Reload) {
        let support = contacts
            .magazine
            .lerp(contacts.foregrip, contacts.extension)
            .lerp(contacts.magazine - Vec3::Y * reload.view_drop, reload.reach);
        for (side, wrist) in [contacts.primary + Vec3::X * 0.028, support]
            .into_iter()
            .enumerate()
        {
            scene.update_instance(
                self.first + side,
                self.grip[side],
                gun * Affine3A::from_translation(wrist),
            );
        }
    }
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
) -> ClusterMesh {
    let side = if left { -1. } else { 1. };
    let wrist = Vec3::new(-side * 0.259, 0.637, 0.084) * scale;
    let elbow = Vec3::new(-side * 0.188, 0.699, -0.013) * scale;
    let axis = (elbow - wrist).normalize();
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
    let palm = Quat::from_mat3(&Mat3::from_cols(fingers, back, fingers.cross(back)))
        * humans::hand_frame(left).inverse();
    let destination = if gripping && left {
        Vec3::new(-0.58, -0.25, 0.52)
    } else {
        Vec3::new(side * 0.22, -0.28, 0.42)
    };
    let bend = Quat::from_rotation_arc(palm * axis, destination.normalize());
    let mut vertices = Vec::new();
    let mut remap = vec![u32::MAX; body.len()];
    let mut surfaces = [Vec::new(), Vec::new()];
    for tri in triangles.chunks_exact(3) {
        let tag = body[tri[0] as usize].material;
        if !matches!(tag, 4 | 12)
            || !tri.iter().all(|&i| {
                let v = &body[i as usize];
                -side * v.position.x > 0.16 && v.limb_weights[0] > 0.75
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
