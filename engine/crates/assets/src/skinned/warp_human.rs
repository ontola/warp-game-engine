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

impl Character {
    /// Reuse this character's clips, sockets and joint names with a Warp Human skin.
    pub fn warp_human(&self, person: &Instance) -> Result<Self, String> {
        self.warp_human_with_skin(person, None)
    }

    /// Override base skin in sRGB while retaining the seeded face texture.
    pub fn warp_human_with_skin(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
    ) -> Result<Self, String> {
        self.fit_human(person, skin, false)
    }

    /// Keep the hm08 upper body in its native relaxed pose for a live two-hand
    /// solver. The legs use the donor bind for the shared locomotion clips.
    pub fn warp_human_player(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
    ) -> Result<Self, String> {
        self.fit_human(person, skin, true)
    }

    fn fit_human(
        &self,
        person: &Instance,
        skin: Option<[f32; 3]>,
        player: bool,
    ) -> Result<Self, String> {
        let mut fitted_joints = self.joints.clone();
        let hips = self
            .joints
            .iter()
            .find(|(n, _)| n == "Hips")
            .ok_or("missing Hips")?
            .1
            .w_axis
            .truncate();
        let scale = person.body_scale();
        let upper_body = Affine3A::from_scale_rotation_translation(
            scale,
            Quat::IDENTITY,
            hips - scale * Vec3::new(0., 0.534, -0.012),
        );
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
                if player && keep_upper {
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
        let core = chain(
            &["Hips", "Spine", "Spine1", "Spine2", "Neck", "Head"],
            &[
                [0., 0.534, -0.012],
                [0., 0.602, -0.012],
                [0., 0.658, -0.005],
                [0., 0.750, -0.025],
                [0., 0.843, -0.017],
                [0., 0.909, -0.011],
                [0., 1., -0.011],
            ]
            .map(Vec3::from_array),
            true,
        )?;
        let mut limbs = Vec::new();
        for (side, sign) in [("Right", -1.), ("Left", 1.)] {
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
            let arm = limb(
                &["Arm", "ForeArm", "Hand"],
                &[
                    [0.1008, 0.805, -0.012],
                    [0.1878, 0.6994, -0.0132],
                    [0.259, 0.637, 0.084],
                    [0.295, 0.565, 0.14],
                ],
                true,
            )?;
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
        let (body, indices) = person.fitted_mesh(player);
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
            let next = ramp(heights[k], heights[k + 1], p.y);
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
                let mut surface = surface(person, tag, skin_color);
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
            joint_count: self.joint_count,
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

fn surface(person: &Instance, tag: u32, skin: Vec3) -> Surface {
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
        13 => (person.hair_color().lerp(skin, 0.26), Some(7)),
        20..=22 => (person.hair_color(), Some((tag - 20 + 3) as usize)),
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
        skin_hair: None,
        material: MaterialFile {
            name: format!("warp-human-{tag}"),
            albedo_texture: None,
            albedo: tint.extend(1.).into(),
            albedo_color_space: "sRGB".into(),
            roughness: if (20..=22).contains(&tag) { 0.61 } else { 0.82 },
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
                    let iris = iris.lerp(Vec3::splat(20.), ramp(13., 16., r));
                    let color = Vec3::splat(18.)
                        .lerp(iris, ramp(4.5, 6., r))
                        .lerp(Vec3::new(187., 176., 165.), ramp(15., 17., r));
                    let shade = 0.78 + 0.22 * ramp(12., 40., y as f32);
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
