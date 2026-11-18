//! Fit a seeded Warp Human to an animated humanoid's bind pose once at load.
use super::*;
use glam::{Mat3, Quat};
use render::humans::{APPEARANCE_ATLAS, BodyVertex, Instance};
use std::collections::BTreeMap;
use std::sync::OnceLock;

struct Bone {
    joint: usize,
    transform: Affine3A,
    normal: Mat3,
}

// Two eye pivots and six signed translation axes reproduce the sparse lid
// target through ordinary GPU skinning. All eight inherit the animated head.
pub(super) struct FaceRig {
    first: usize,
    pub(super) head: usize,
    eyes: [Vec3; 2],
    scale: Vec3,
    phase: f32,
    pub(super) groom: render::humans::Groom,
}

const LID_RANGE: f32 = 0.016;

// Normalized hm08 joint positions (unit body height), each chain with the
// point its last joint's segment ends at.
const CORE_NAMES: [&str; 6] = ["Hips", "Spine", "Spine1", "Spine2", "Neck", "Head"];
const CORE_POINTS: [[f32; 3]; 7] = [
    [0., 0.534, -0.012],
    [0., 0.602, -0.012],
    [0., 0.658, -0.005],
    [0., 0.750, -0.025],
    [0., 0.843, -0.017],
    [0., 0.909, -0.011],
    [0., 1., -0.011],
];
const ARM_NAMES: [&str; 3] = ["Arm", "ForeArm", "Hand"];
const ARM_POINTS: [[f32; 3]; 4] = [
    [0.1008, 0.805, -0.012],
    [0.1878, 0.6994, -0.0132],
    [0.259, 0.637, 0.084],
    [0.295, 0.565, 0.14],
];
const SIDES: [(&str, f32); 2] = [("Right", -1.), ("Left", 1.)];

impl Character {
    /// Apply after the head pose, using the interpolated presentation clock.
    pub fn pose_face(&self, palette: &mut [Mat4], seconds: f32) {
        let Some(face) = &self.face else { return };
        let head = palette[face.head];
        let phase = (seconds / (3.1 + 1.8 * face.phase) + face.phase).rem_euclid(1.);
        let blink = ramp(0., 0.020, phase) * (1. - ramp(0.025, 0.064, phase));
        let clock = seconds + 17. * face.phase;
        let yaw = 0.28 * (ramp(0.20, 0.65, (clock * 1.41).sin()) - 0.5);
        let pitch = -0.13 * (ramp(0.35, 0.76, (clock * 0.92).sin()) - 0.5);
        let gaze = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
        for (i, pivot) in face.eyes.into_iter().enumerate() {
            palette[face.first+i] = head * Mat4::from_rotation_translation(gaze, pivot - gaze*pivot);
        }
        for (i, axis) in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z].into_iter().enumerate() {
            palette[face.first+2+i] = head * Mat4::from_translation(face.scale * axis * (LID_RANGE * blink));
        }
    }

    /// Reuse this character's clips, sockets and joint names with a Warp Human skin.
    pub fn warp_human(&self, person: &Instance) -> Result<Self, String> {
        self.warp_human_with_skin(person, None)
    }

    /// Override base skin in sRGB while retaining the seeded face texture.
    /// The body is stretched onto this rig's own joints, shoulders included.
    pub fn warp_human_with_skin(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
    ) -> Result<Self, String> {
        self.fit_human(person, skin, false, false)
    }

    /// Keep the hm08 upper body in its native relaxed pose for a live two-hand
    /// solver. The legs use the donor bind for the shared locomotion clips.
    pub fn warp_human_player(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
    ) -> Result<Self, String> {
        self.fit_human(person, skin, true, true)
    }

    /// A person with their own anatomy: the spine, neck, head and arm joints
    /// move to where the hm08 body has them (`warp_human_joints`), so the
    /// shoulders keep their width and height and the skin binds undeformed.
    /// Open hands, blinking eyes and close-up hair strands. The caller poses
    /// the joints this returns, not the donor's.
    pub fn warp_human_person(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
    ) -> Result<Self, String> {
        self.fit_human(person, skin, true, false)
    }

    /// Where `warp_human_person` and `warp_human_player` put this rig's
    /// joints for `person`, without fitting a mesh (no GPU, cheap).
    pub fn warp_human_joints(&self, person: &Instance) -> Result<Vec<(String, Mat4)>, String> {
        let mut joints = self.joints.clone();
        let upper_body = self.upper_body(person)?;
        let mut place = |name: &str, point: Vec3| -> Result<(), String> {
            let joint = joints
                .iter_mut()
                .find(|(n, _)| n == name)
                .ok_or_else(|| format!("Warp Human rig missing {name}"))?;
            joint.1.w_axis = upper_body.transform_point3(point).extend(1.);
            Ok(())
        };
        for (name, point) in CORE_NAMES.iter().zip(CORE_POINTS) {
            place(name, Vec3::from_array(point))?;
        }
        for (side, sign) in SIDES {
            for (name, [x, y, z]) in ARM_NAMES.iter().zip(ARM_POINTS) {
                place(&format!("{side}{name}"), Vec3::new(x * sign, y, z))?;
            }
        }
        Ok(joints)
    }

    // hm08's unit-height space onto this rig, meeting at the hips.
    fn upper_body(&self, person: &Instance) -> Result<Affine3A, String> {
        let hips = self
            .joints
            .iter()
            .find(|(n, _)| n == "Hips")
            .ok_or("missing Hips")?
            .1
            .w_axis
            .truncate();
        let scale = person.body_scale();
        Ok(Affine3A::from_scale_rotation_translation(
            scale,
            Quat::IDENTITY,
            hips - scale * Vec3::from_array(CORE_POINTS[0]),
        ))
    }

    // `native`: the upper body keeps hm08's own joints and pose. `gripping`:
    // the fingers curl around a weapon.
    fn fit_human(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
        native: bool,
        gripping: bool,
    ) -> Result<Self, String> {
        let mut fitted_joints = self.joints.clone();
        let scale = person.body_scale();
        let upper_body = self.upper_body(person)?;
        // Normalized hm08 joint positions, in the same space as the crowd rig.
        let mut chain =
            |names: &[&str], points: &[Vec3], keep_upper: bool| -> Result<Vec<Bone>, String> {
                let joints = names
                    .iter()
                    .map(|name| {
                        self.joints
                            .iter()
                            .enumerate()
                            .find(|(_, (n, _))| n == name)
                            .map(|(i, (_, bind))| (i, bind.w_axis.truncate()))
                            .ok_or_else(|| format!("Warp Human rig missing {name}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if native && keep_upper {
                    let normal = Mat3::from(upper_body.matrix3).inverse().transpose();
                    return Ok(joints
                        .iter()
                        .zip(points)
                        .map(|(&(joint, _), &point)| {
                            fitted_joints[joint].1.w_axis =
                                upper_body.transform_point3(point).extend(1.);
                            Bone {
                                joint,
                                transform: upper_body,
                                normal,
                            }
                        })
                        .collect());
                }
                let mut result = Vec::new();
                let mut previous_rotation = Quat::IDENTITY;
                for (i, &(joint, at)) in joints.iter().enumerate() {
                    let mut rotation = Quat::IDENTITY;
                    let mut stretch = 1.0;
                    let source = (points[i + 1] - points[i]).normalize();
                    if let Some(&(_, end)) = joints.get(i + 1) {
                        rotation = Quat::from_rotation_arc(source, (end - at).normalize());
                        stretch = at.distance(end) / (points[i].distance(points[i + 1]) * scale.y);
                    } else if names[i].ends_with("Hand") {
                        rotation = previous_rotation;
                    }
                    previous_rotation = rotation;
                    let along =
                        Mat3::from_cols(source * source.x, source * source.y, source * source.z);
                    let linear = Mat3::from_quat(rotation)
                        * (Mat3::IDENTITY * scale.x + along * (scale.y * stretch - scale.x));
                    let transform =
                        Affine3A::from_mat3_translation(linear, at - linear * points[i]);
                    result.push(Bone {
                        joint,
                        transform,
                        normal: linear.inverse().transpose(),
                    });
                }
                Ok(result)
            };
        let core = chain(&CORE_NAMES, &CORE_POINTS.map(Vec3::from_array), true)?;
        let mut limbs = Vec::new();
        for (side, sign) in SIDES {
            let mut limb = |names: &[&str], points: &[[f32; 3]], keep_upper| {
                let names: Vec<_> = names.iter().map(|name| format!("{side}{name}")).collect();
                let points: Vec<_> = points
                    .iter()
                    .map(|&[x, y, z]| Vec3::new(x * sign, y, z))
                    .collect();
                chain(
                    &names.iter().map(String::as_str).collect::<Vec<_>>(),
                    &points,
                    keep_upper,
                )
            };
            let arm = limb(&ARM_NAMES, &ARM_POINTS, true)?;
            let leg = limb(
                &["UpLeg", "Leg", "Foot"],
                &[
                    [0.0662, 0.5192, -0.0136],
                    [0.0949, 0.2683, -0.0018],
                    [0.1318, 0.0431, -0.0215],
                    [0.1319, 0.0039, 0.0515],
                ],
                false,
            )?;
            limbs.push((arm, leg));
        }
        let skin_color = skin.map(Vec3::from).unwrap_or_else(|| person.skin_color());
        let (hair, brows) = person.hair_and_brows(skin_color);
        let (body, indices) = person.fitted_mesh(gripping);
        let face = native.then(|| {
            let first = fitted_joints.len();
            let eyes = [false, true].map(|positive| {
                let mut lo = Vec3::splat(f32::INFINITY);
                let mut hi = Vec3::splat(f32::NEG_INFINITY);
                for v in body.iter().filter(|v| v.material == 6 && (v.position.x > 0.) == positive) {
                    lo = lo.min(v.position);
                    hi = hi.max(v.position);
                }
                upper_body.transform_point3((lo + hi) * 0.5)
            });
            for (name, pivot) in ["RightEye", "LeftEye"].into_iter().zip(eyes) {
                fitted_joints.push((name.into(), Mat4::from_translation(pivot)));
            }
            let head = core[5].joint;
            let bind = fitted_joints[head].1;
            for axis in ["Xp","Xn","Yp","Yn","Zp","Zn"] {
                fitted_joints.push((format!("Lid{axis}"), bind));
            }
            let skull = scale.y / scale.x;
            let fit = Mat4::from(upper_body) * Mat4::from_scale(Vec3::new(skull,1.,skull));
            let groom = render::humans::Groom {
                fit: fit.to_cols_array_2d(),
                weights: person.head_weights(),
                tint: Vec3::from_array(hair.to_array().map(srgb_to_linear)).extend(1.).to_array(),
                info: [0,0,person.hair_style(),(person.face_phase()*16777216.) as u32],
            };
            FaceRig { first, head, eyes, scale, phase: person.face_phase(), groom }
        });
        let skin = |v: &BodyVertex| {
            let p = v.position;
            let (arm, leg) = &limbs[usize::from(p.x > 0.)];
            let [upper, fore, thigh, shin] = v.limb_weights;
            let wrist = fore * ramp(0.245, 0.265, p.x.abs()).max(ramp(0.085, 0.115, p.z));
            let ankle = shin * (1. - ramp(0.042, 0.084, p.y));
            let rest = (1. - upper - thigh).max(0.);
            let heights = [0.534, 0.602, 0.658, 0.750, 0.843, 0.909];
            let k = heights
                .partition_point(|&y| y < p.y)
                .saturating_sub(1)
                .min(4);
            // Keep the jaw and facial muscles with the skull when aiming.
            // Only the actual neck transition blends these two bones.
            let (k, next) = if k == 4 || v.head_weight > 0. {
                (4, v.head_weight)
            } else {
                (k, ramp(heights[k], heights[k + 1], p.y))
            };
            let weights = [
                (&core[k], rest * (1. - next)),
                (&core[k + 1], rest * next),
                (&arm[0], upper - fore),
                (&arm[1], fore - wrist),
                (&arm[2], wrist),
                (&leg[0], thigh - shin),
                (&leg[1], shin - ankle),
                (&leg[2], ankle),
            ];
            let mut inf = Influence {
                joints: [0; 8],
                weights: [0.; 8],
            };
            let mut pos = Vec3::ZERO;
            let mut normal = Vec3::ZERO;
            for (i, (bone, w)) in weights.into_iter().enumerate() {
                inf.joints[i] = bone.joint as u32;
                inf.weights[i] = w;
                pos += bone.transform.transform_point3(p) * w;
                normal += (bone.normal * v.normal) * w;
            }
            if let Some(face) = &face {
                if v.material == 6 {
                    inf = Influence { joints: [0;8], weights: [0.;8] };
                    inf.joints[0] = (face.first + usize::from(p.x > 0.)) as u32;
                    inf.weights[0] = 1.;
                } else if v.lid_closure != Vec3::ZERO {
                    inf = Influence { joints: [0;8], weights: [0.;8] };
                    for (axis, delta) in v.lid_closure.to_array().into_iter().enumerate() {
                        inf.joints[axis+1] = (face.first+2+2*axis+usize::from(delta < 0.)) as u32;
                        inf.weights[axis+1] = delta.abs() / LID_RANGE;
                    }
                    inf.joints[0] = face.head as u32;
                    inf.weights[0] = 1. - inf.weights.iter().sum::<f32>();
                    debug_assert!(inf.weights[0] >= 0.);
                }
            }
            let uv = if v.material == 6 {
                [
                    (0.5 + v.normal.x * 0.46).clamp(0.001, 0.999),
                    (0.5 - v.normal.y * 0.46).clamp(0.001, 0.999),
                ]
            } else {
                v.uv
            };
            (
                Vertex {
                    pos: pos.into(),
                    normal: normal.normalize_or(Vec3::Y).into(),
                    uv,
                },
                inf,
            )
        };
        let mut groups = BTreeMap::<u32, Vec<u32>>::new();
        for tri in indices.chunks_exact(3) {
            let a = &body[tri[0] as usize];
            let tag = if a.material == 3
                && tri
                    .iter()
                    .map(|&i| body[i as usize].position.y)
                    .sum::<f32>()
                    < 3. * 0.012
            {
                17 // rubber outsole, sharing the shoe geometry
            } else {
                a.material
            };
            groups.entry(tag).or_default().extend(tri);
            // Reuse the shirt's skin weights for the small area carrying the print.
            if a.material == 12 && person.chest_print() != 0 {
                let points = [0, 1, 2].map(|k| body[tri[k] as usize].position);
                let lo = points[0].min(points[1]).min(points[2]);
                let hi = points[0].max(points[1]).max(points[2]);
                if hi.x > -0.095 && lo.x < 0.095 && hi.y > 0.668 && lo.y < 0.808 && lo.z > 0.02 {
                    let number = person.chest_print() <= 100;
                    if !number || lo.x < 0. {
                        groups.entry(15).or_default().extend(tri);
                    }
                    if number && hi.x > 0. {
                        groups.entry(16).or_default().extend(tri);
                    }
                }
            }
        }
        let skinned: Vec<_> = body.iter().map(skin).collect();
        let surfaces = groups
            .into_iter()
            .map(|(tag, mut indices)| {
                let mut surface = surface(person, tag, skin_color, hair, brows);
                let mut remap = vec![u32::MAX; body.len()];
                for index in &mut indices {
                    let source = *index as usize;
                    if remap[source] == u32::MAX {
                        remap[source] = surface.vertices.len() as u32;
                        let mut vertex = skinned[source].0;
                        if matches!(tag, 15 | 16) {
                            let p = body[source].position;
                            let print = person.chest_print();
                            let x = 0.5 + p.x / 0.19;
                            let (slot, u) = if print > 100 {
                                (print - 91, x)
                            } else if tag == 15 {
                                ((print - 1) / 10, x * 2.)
                            } else {
                                ((print - 1) % 10, x * 2. - 1.)
                            };
                            let u = u.clamp(0.01, 0.99);
                            let v = (0.5 + (0.738 - p.y) / 0.14).clamp(0.01, 0.99);
                            vertex.uv = [
                                (slot as f32 % 4. + u) * 0.25,
                                ((slot / 4) as f32 + v) * 0.25,
                            ];
                            vertex.pos = (Vec3::from(vertex.pos)
                                + Vec3::from(vertex.normal) * 0.0015)
                                .into();
                        }
                        surface.vertices.push(vertex);
                        surface.influences.push(skinned[source].1);
                    }
                    *index = remap[source];
                }
                surface.indices = indices;
                surface
            })
            .collect();
        Ok(Self {
            // Grip corrections are authored for the donor rig by name.
            name: self.name.clone(),
            joint_count: fitted_joints.len(),
            face,
            clips: self.clips.clone(),
            joints: fitted_joints,
            min: self.min.min(Vec3::splat(-1.0)),
            max: self.max.max(Vec3::new(1., 2.2, 1.)),
            triangles: indices.len() / 3,
            surfaces,
        })
    }
}

fn ramp(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

fn surface(person: &Instance, tag: u32, skin: Vec3, hair: Vec3, brows: Vec3) -> Surface {
    let face = person.face_tile();
    let cheek = [
        Vec3::new(0.855, 0.659, 0.541),
        Vec3::new(0.894, 0.741, 0.639),
        Vec3::new(0.353, 0.224, 0.149),
    ][face];
    let (tint, tile) = match tag {
        12 => (Vec3::from(person.shirt_color()), Some(8 + person.outfit())),
        14 => (
            Vec3::from(person.trouser_color()),
            Some(8 + usize::from(person.outfit() != 0)),
        ),
        15 | 16 => {
            let light = Vec3::from(person.shirt_color()).dot(Vec3::new(0.3, 0.6, 0.1));
            (Vec3::splat(if light > 0.58 { 0.14 } else { 0.88 }), Some(6))
        }
        3 => (Vec3::from(person.shoe_color()), None),
        17 => (Vec3::from(person.shoe_color()) * 0.48, None),
        4 => (skin, None),
        11 => (skin / cheek, Some(face)),
        13 => (brows, Some(7)),
        20..=22 => (hair, Some((tag - 20 + 3) as usize)),
        _ => (Vec3::ONE, None),
    };
    // Three neutral fabrics, one shared print sheet and the eye texture follow the atlas tiles.
    let slot = match tag {
        6 => Some(11),
        _ => tile,
    };
    let texture = slot.map(|i| (format!("warp-human-tile-{i}"), shared_tiles()[i].clone()));
    let cutout = matches!(tag, 13 | 15 | 16) || tag >= 20;
    Surface {
        vertices: Vec::new(),
        indices: Vec::new(),
        influences: Vec::new(),
        texture,
        normal: None,
        skin_hair: match tag {
            4 => Some(SkinHair::Skin),
            11 => Some(SkinHair::PackedMask),
            _ => None,
        },
        material: MaterialFile {
            name: format!("warp-human-{tag}"),
            albedo_texture: None,
            albedo: tint.extend(1.).into(),
            albedo_color_space: "sRGB".into(),
            roughness: match tag { 6 => 0.23, 11 => 0.46, 4 => 0.61, 20..=22 => 0.70, _ => 0.82 },
            metallic: 0.,
            normal_texture: None,
            normal_scale: 1.,
            two_sided: cutout,
            alpha_mode: if cutout { "MASK".into() } else { String::new() },
        },
    }
}

fn shared_tiles() -> &'static [std::sync::Arc<image::RgbaImage>; 12] {
    static TILES: OnceLock<[std::sync::Arc<image::RgbaImage>; 12]> = OnceLock::new();
    TILES.get_or_init(|| {
        let atlas = image::load_from_memory(APPEARANCE_ATLAS)
            .expect("Warp Humans atlas")
            .into_rgba8();
        let (w, h) = (atlas.width() / 4, atlas.height() / 2);
        std::array::from_fn(|i| {
            if i == 11 {
                return image::RgbaImage::from_fn(64, 64, |x, y| {
                    let dx = x as f32 - 31.5;
                    let dy = y as f32 - 31.5;
                    let r = dx.hypot(dy);
                    let grain = (dx.atan2(dy) * 33.).sin() * 4.;
                    let iris = Vec3::new(82. + grain, 66. + grain, 49.);
                    let iris = iris.lerp(Vec3::splat(22.), ramp(10., 13.5, r));
                    let color = Vec3::splat(18.)
                        .lerp(iris, ramp(4.5, 6., r))
                        .lerp(Vec3::new(187., 176., 165.), ramp(13., 14.5, r));
                    let shade = 0.60 + 0.40 * ramp(15., 34., y as f32);
                    let [r, g, b] = (color * shade).to_array().map(|v| v as u8);
                    image::Rgba([r, g, b, 255])
                })
                .into();
            }
            let tile = if i >= 8 { 6 } else { i as u32 };
            let mut image =
                image::imageops::crop_imm(&atlas, tile % 4 * w, tile / 4 * h, w, h).to_image();
            if (8..=10).contains(&i) {
                // Extract one monochrome fabric for the standard skinned material.
                image = image::imageops::resize(
                    &image,
                    256,
                    256,
                    image::imageops::FilterType::Triangle,
                );
                for p in image.pixels_mut() {
                    let light = p[i - 8];
                    *p = image::Rgba([light, light, light, 255]);
                }
            } else if i == 6 {
                image = image::imageops::resize(
                    &image,
                    256,
                    256,
                    image::imageops::FilterType::Triangle,
                );
                for p in image.pixels_mut() {
                    *p = image::Rgba([255, 255, 255, p[3]]);
                }
            } else if matches!(i, 3..=5 | 7) {
                for p in image.pixels_mut() {
                    // Reference highlights from the three source grooms and brows.
                    // A single exposure clips nearly the entire light bob texture.
                    let highlight = match i {
                        3 => 45.,
                        4 => 222.,
                        5 => 27.,
                        _ => 24.,
                    };
                    let luma = 0.3 * p[0] as f32 + 0.5 * p[1] as f32 + 0.2 * p[2] as f32;
                    let strand = 0.22 + 0.72 * (luma / highlight).clamp(0., 1.1).sqrt();
                    p.0[..3].fill((255. * strand) as u8);
                }
            }
            image.into()
        })
    })
}
