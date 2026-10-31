//! Giant reef animals: source-inspired mantle and eight continuously skinned
//! arms. One compute deformation serves every view and shadow pass. Placement
//! belongs to stages.luau; this module owns bounded mesh/animation mathematics.
use super::visuals::surface;
use glam::{Affine3A, Mat4, Quat, Vec3};
use render::{
    Scene,
    cluster::{ClusterMesh, Part, Vertex},
    primitives::{self, Primitive},
    skinning::{Actor, Influence},
};
const RINGS: usize = 24;
const SIDES: usize = 12;
const JOINTS: usize = 1 + 8 * (RINGS + 1);
// Arm arc length <= 5.5, root radius .8, tube radius <= .35, body <= 3.
const BOUND: f32 = 7.;
#[derive(Clone, Copy)]
pub struct Site {
    pub position: Vec3,
    pub size: f32,
    pub yaw: f32,
    pub phase: f32,
}
pub struct Octopus {
    pub actor: usize,
    pub palette: Vec<Mat4>,
    phase: f32,
}
fn influence(joint: u32) -> Influence {
    Influence {
        joints: [joint, 0, 0, 0, 0, 0, 0, 0],
        weights: [1., 0., 0., 0., 0., 0., 0., 0.],
    }
}
fn sphere(p: &mut Primitive, pos: Vec3, scale: Vec3, rot: Quat) {
    let q = primitives::sphere(p.material, 16, 24);
    let offset = p.vertices.len() as u32;
    let xf = Affine3A::from_scale_rotation_translation(scale, rot, pos);
    let normal = xf.matrix3.inverse().transpose();
    for mut v in q.vertices {
        v.pos = xf.transform_point3(v.pos.into()).to_array();
        v.normal = (normal * Vec3::from(v.normal)).normalize().to_array();
        p.vertices.push(v);
    }
    p.indices.extend(q.indices.into_iter().map(|i| i + offset));
}
impl Octopus {
    pub fn add(scene: &mut Scene, site: Site) -> Self {
        let skin = surface(scene, 19, [0.62, 0.16, 0.07], 0.5, 1.);
        let arm = surface(scene, 20, [0.62, 0.16, 0.07], 0.5, 1.);
        let eye = surface(scene, 21, [0.85, 0.62, 0.18], 0.18, 1.);
        let mut body = Primitive {
            material: skin,
            ..Default::default()
        };
        sphere(
            &mut body,
            Vec3::new(0., 1.45, 0.35),
            Vec3::new(0.85, 1.3, 0.95),
            Quat::from_rotation_x(0.5),
        );
        sphere(
            &mut body,
            Vec3::new(0., 0.35, 0.),
            Vec3::new(1.15, 0.42, 1.15),
            Quat::IDENTITY,
        );
        let mut eyes = Primitive {
            material: eye,
            ..Default::default()
        };
        for side in [-1., 1.] {
            sphere(
                &mut eyes,
                Vec3::new(side * 0.58, 0.95, -0.75),
                Vec3::splat(0.23),
                Quat::IDENTITY,
            );
        }
        let mut arms = Primitive {
            material: arm,
            ..Default::default()
        };
        let mut weights = vec![influence(0); body.vertices.len() + eyes.vertices.len()];
        for limb in 0..8 {
            let first = arms.vertices.len() as u32;
            for ring in 0..=RINGS {
                let t = ring as f32 / RINGS as f32;
                let radius = 0.05 * (1. - t * 0.97) * (1. + 0.25 * (1. - (t / 0.1).min(1.)));
                for side in 0..=SIDES {
                    let angle = side as f32 / SIDES as f32 * std::f32::consts::TAU;
                    arms.vertices.push(Vertex {
                        pos: [angle.cos() * radius, t, angle.sin() * radius],
                        normal: [angle.cos(), 0., angle.sin()],
                        uv: [side as f32 / SIDES as f32, t],
                    });
                    weights.push(influence((1 + limb * (RINGS + 1) + ring) as u32));
                }
            }
            for ring in 0..RINGS {
                for side in 0..SIDES {
                    let a = first + (ring * (SIDES + 1) + side) as u32;
                    let b = a + (SIDES + 1) as u32;
                    arms.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
                }
            }
        }
        let parts = [&body, &eyes, &arms].map(|p| Part {
            vertices: &p.vertices,
            indices: &p.indices,
            material: p.material,
        });
        let mesh = ClusterMesh::build_fixed(&parts, [0., 0., 0., BOUND]);
        let first_vertex = scene.vertices.len() as u32;
        let vertex_count = mesh.vertices.len() as u32;
        let mesh = scene.add_mesh(&mesh);
        let owner = scene.instances.len() as u32;
        scene.add_instance(
            mesh,
            Affine3A::from_scale_rotation_translation(
                Vec3::splat(site.size),
                Quat::from_rotation_y(site.yaw),
                site.position,
            ),
        );
        scene.instances[owner as usize].flags |= render::scene::SKINNED;
        let mut out = Self {
            actor: scene.skinning.actors.len(),
            palette: vec![Mat4::IDENTITY; JOINTS],
            phase: site.phase,
        };
        out.update(0.);
        scene.skinning.actors.push(Actor {
            first_vertex,
            vertex_count,
            owner,
            palette: out.palette.clone(),
            influences: weights,
        });
        out
    }
    pub fn update(&mut self, time: f32) {
        self.palette[0] = Mat4::from_scale(Vec3::new(
            1.,
            1. + 0.025 * (time * 0.8 + self.phase).sin(),
            1.,
        ));
        for limb in 0..8 {
            let angle = (limb as f32 + 0.5) * std::f32::consts::TAU / 8.;
            let out = Vec3::new(angle.sin(), 0., -angle.cos());
            let raised = limb == 0 || limb == 7;
            let base = if raised { 0.35 } else { 1.4 };
            let curl = base + if raised { 1.7 } else { -2.7 };
            let strength = if raised { 0.42 } else { 0.22 };
            let phase = self.phase + limb as f32 * 1.37;
            let length = 4.7 + 0.5 * (limb as f32 * 2.1).sin();
            let theta = |t: f32| {
                base + (curl - base) * t * t + strength * (time * 0.75 + phase + t * 5.).sin() * t
            };
            let mut center = out * 0.8 + Vec3::Y * 0.3;
            for ring in 0..=RINGS {
                let t = ring as f32 / RINGS as f32;
                if ring > 0 {
                    let a = theta(t - 0.5 / RINGS as f32);
                    center += (out * a.sin() + Vec3::Y * a.cos()) * length / RINGS as f32;
                }
                let a = theta(t);
                let radial = out * a.cos() - Vec3::Y * a.sin();
                let tangent = out * a.sin() + Vec3::Y * a.cos();
                let binormal = out.cross(Vec3::Y);
                self.palette[1 + limb * (RINGS + 1) + ring] = Mat4::from_cols(
                    (radial * length).extend(0.),
                    (tangent * length).extend(0.),
                    (binormal * length).extend(0.),
                    (center - tangent * (t * length)).extend(1.),
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arms_are_continuous_bounded_and_freeze_at_fixed_race_time() {
        let mut scene = Scene::default();
        let mut animal = Octopus::add(
            &mut scene,
            Site {
                position: Vec3::ZERO,
                size: 25.,
                yaw: 0.,
                phase: 0.7,
            },
        );
        scene
            .skinning
            .validate(&scene.vertices, &scene.instances)
            .unwrap();
        for frame in 0..90 {
            animal.update(frame as f32 * 0.17);
            let actor = &scene.skinning.actors[0];
            for (v, w) in scene.vertices.iter().zip(&actor.influences) {
                let p = render::skinning::skin_position(v.pos.into(), w, &animal.palette);
                assert!(
                    p.is_finite() && p.length() < BOUND,
                    "animated bound escaped: {p:?}"
                );
            }
        }
        let before = animal.palette.clone();
        animal.update(89. * 0.17);
        assert_eq!(before, animal.palette);
        animal.update(16.);
        assert_ne!(before, animal.palette);
    }
}
