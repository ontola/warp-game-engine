//! Shared blood silhouettes. Each footprint is a single clustered instance with
//! a wet core, torn edge and detached drops. The visible footprint fits the unit disc.
use glam::{Affine3A, Mat3, Quat, Vec2, Vec3};
use growing_guns::shape::YawBox;
use render::{cluster::Vertex, primitives::Primitive, scene::Material, Scene};
use std::f32::consts::TAU;

const VARIANTS: usize = 16;
#[derive(Clone, Copy)]
pub(crate) struct Splats([usize; VARIANTS]);
struct Noise(u32);
impl Noise {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f32 / 16777216.
    }
    fn between(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
}

fn vertex(p: Vec2) -> Vertex {
    Vertex {
        pos: [p.x, 0., p.y],
        normal: Vec3::Y.to_array(),
        uv: (p * 0.5 + Vec2::splat(0.5)).to_array(),
    }
}
fn fan(mesh: &mut Primitive, center: Vec2, outline: &[Vec2]) {
    let base = mesh.vertices.len() as u32;
    mesh.vertices.push(vertex(center));
    mesh.vertices.extend(outline.iter().copied().map(vertex));
    for i in 0..outline.len() as u32 {
        mesh.indices.extend([
            base,
            base + 1 + (i + 1) % outline.len() as u32,
            base + 1 + i,
        ]);
    }
}
// The minimum forward path draws material tints only. Give it actual silhouettes.
fn simple_footprint(i: usize, material: u32) -> Primitive {
    let mut rng = Noise(i as u32 * 731 + 317);
    let mut m = Primitive {
        material,
        ..Default::default()
    };
    for k in 0..8 {
        let center = if k == 0 {
            Vec2::new(-0.19, 0.)
        } else {
            Vec2::new(rng.between(-0.32, 0.73), rng.between(-0.44, 0.44))
        };
        let radius = if k == 0 {
            0.39
        } else {
            rng.between(0.025, 0.088)
        };
        let edge = (0..14)
            .map(|j| {
                let a = TAU * j as f32 / 14.;
                center + Vec2::new(a.cos(), 0.72 * a.sin()) * radius * rng.between(0.66, 1.12)
            })
            .collect::<Vec<_>>();
        fan(&mut m, center, &edge);
    }
    m
}

const ATLAS: &[u8] = include_bytes!("../../assets/blood-splats.png");
fn card(i: usize, material: u32) -> Primitive {
    let tile = Vec2::new((i % 4) as f32, (i / 4) as f32);
    let mut m = Primitive {
        material,
        indices: vec![0, 2, 1, 0, 3, 2],
        ..Default::default()
    };
    for uv in [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y] {
        let mut v = vertex(uv * 2. - Vec2::ONE);
        v.uv = ((tile + uv) * 0.25).to_array();
        m.vertices.push(v);
    }
    m
}

/// A tapered liquid filament, long axis Z (the particle pool aligns Z to velocity).
pub fn drop_mesh(material: u32) -> Primitive {
    let mut p = render::primitives::sphere(material, 4, 7);
    for v in &mut p.vertices {
        let a = Vec3::from(v.pos);
        let taper = 0.66 - 0.46 * a.z;
        let scale = Vec3::new(0.64 * taper, 0.73 * taper, 1.36);
        v.pos = (a * scale).to_array();
        v.normal = (Vec3::from(v.normal) / scale)
            .normalize_or(Vec3::Y)
            .to_array();
    }
    p
}
pub fn spray() -> render::particles::Burst {
    render::particles::Burst {
        size: [0.056, 0.021],
        life: [0.5, 1.25],
        stretch: 0.17,
        ..render::particles::Burst::drops()
    }
}
impl Splats {
    pub fn new(scene: &mut Scene) -> Self {
        let simple = crate::graphics::get().minimal_forward;
        let layer = if simple {
            -1
        } else {
            let image = image::load_from_memory(ATLAS)
                .expect("cooked blood footprints")
                .into_rgba8();
            scene.add_image("growing-guns blood splats", image)
        };
        let material = scene.add_material(Material {
            tint: if simple {
                [0.10, 0.003, 0.007]
            } else {
                [1.; 3]
            },
            layer,
            // Pigment on porous masonry scatters the stadium lamps broadly.
            roughness: 0.92,
            metal: 0.,
            scale: 1.,
            flags: if simple { 0 } else { render::scene::ALPHA_TEST },
        });
        scene.set_material_layers(material, render::MaterialLayers {
            wet: 0.0,
            ..Default::default()
        });
        Self(std::array::from_fn(|i| {
            let m = if simple {
                simple_footprint(i, material)
            } else {
                card(i, material)
            };
            scene.add_mesh(&m.mesh())
        }))
    }
    pub fn mesh(self, seed: u32) -> usize {
        self.0[seed as usize % VARIANTS]
    }
}

// Fit the footprint inside its receiving face, including narrow vertical risers.
pub fn on_face(b: &YawBox, at: Vec3, n: Vec3, travel: Vec3, radius: f32, seed: u32) -> Affine3A {
    let inv = Quat::from_rotation_y(-b.yaw);
    let normal = inv * n;
    let a = normal.abs();
    let axis = if a.x > a.y && a.x > a.z {
        0
    } else if a.y > a.z {
        1
    } else {
        2
    };
    let mut rng = Noise(seed);
    let tangent = travel - n * n.dot(travel);
    let drift = if tangent.length_squared() > 0.015 {
        tangent.normalize()
    } else {
        n.any_orthonormal_vector()
    };
    let angle = if tangent.length_squared() > 0.015 {
        rng.between(-0.31, 0.31)
    } else {
        rng.unit() * TAU
    };
    let x = Quat::from_axis_angle(n, angle) * drift;
    let z = x.cross(n);
    let mut sx = radius * rng.between(0.98, 1.35);
    let mut sz = radius * rng.between(0.61, 0.96);
    let (lx, lz) = (inv * x, inv * z);
    let mut p = inv * (at - b.center);
    for i in 0..3 {
        if i == axis {
            continue;
        }
        let extent = Vec2::new(lx[i] * sx, lz[i] * sz).length();
        let factor = (b.half[i] * 0.98 / extent.max(1e-5)).min(1.);
        sx *= factor;
        sz *= factor;
    }
    for i in 0..3 {
        if i == axis {
            p[i] = b.half[i] * normal[i].signum();
            continue;
        }
        let limit = (b.half[i] - Vec2::new(lx[i] * sx, lz[i] * sz).length()).max(0.);
        p[i] = p[i].clamp(-limit, limit);
    }
    Affine3A::from_mat3_translation(
        Mat3::from_cols(x * sx, n, z * sz),
        b.center + inv.conjugate() * p + n * rng.between(0.004, 0.008),
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprints_are_repeatable_distinct_and_fit_a_tread_or_wall() {
        let surface = YawBox::new(Vec3::new(4., 2., 1.), Vec3::new(2., 0.48, 1.3), 0.83);
        let rot = Quat::from_rotation_y(surface.yaw);
        let atlas = image::load_from_memory(ATLAS).unwrap().into_rgba8();
        let mut shapes = std::collections::HashSet::new();
        for i in 0..VARIANTS {
            let recipe = 317 + i as u32 * 731;
            let u = i as u32 % 4 * 256;
            let v = i as u32 / 4 * 256;
            let mut pixels = Vec::new();
            for y in 0..256 {
                for x in 0..256 {
                    let alpha = atlas.get_pixel(u + x, v + y)[3];
                    if alpha >= 128 {
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / 128. - Vec2::ONE;
                        assert!(p.length() < 0.95);
                        pixels.push((x, y));
                    }
                }
            }
            assert!(pixels.len() > 1000 && shapes.insert(pixels));
            let shape = simple_footprint(i, 0);
            for normal in [Vec3::Y, Vec3::Z, Vec3::NEG_X] {
                let n = rot * normal;
                let at =
                    surface.center + rot * (normal * surface.half + Vec3::new(0.8, 0.16, 0.45));
                let xf = on_face(&surface, at, n, Vec3::new(1., -0.8, 0.2), 1.1, recipe);
                for v in &shape.vertices {
                    let p = Vec3::from(v.pos);
                    assert!(Vec2::new(p.x, p.z).length() <= 1.);
                    let local = rot.conjugate() * (xf.transform_point3(p) - surface.center);
                    let excess = local.abs() - surface.half;
                    let plane = (Vec3::ONE - normal.abs()) * excess;
                    assert!(plane.max_element() < 0.001);
                    assert!((0.003..0.010).contains(&excess.dot(normal.abs())));
                }
            }
        }
    }
}
