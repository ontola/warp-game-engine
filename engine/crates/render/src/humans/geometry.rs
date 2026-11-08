use super::Far;
use crate::primitives::{self, Primitive};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Vertex {
    pos: [f32; 3],
    material: u32,
    normal: [f32; 3],
    arm: f32,
    center: [f32; 3],
    bin: u32,
}

#[derive(Default)]
pub(super) struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}
impl Mesh {
    fn part(
        &mut self,
        shape: Primitive,
        center: Vec3,
        scale: Vec3,
        material: u32,
        arm: f32,
        bin: u32,
    ) {
        let start = self.vertices.len() as u32;
        self.vertices.extend(shape.vertices.iter().map(|v| Vertex {
            pos: (center + Vec3::from(v.pos) * scale).into(),
            material,
            normal: (Vec3::from(v.normal) / scale).normalize().into(),
            arm,
            center: center.into(),
            bin,
        }));
        self.indices.extend(shape.indices.iter().map(|i| start + i));
    }
    // Closed rings form the hem, shoulder line and collar of one pullover.
    fn pullover(&mut self, bin: u32) {
        let rings = [
            (0.470, 0.095, 0.064),
            (0.480, 0.106, 0.071),
            (0.525, 0.105, 0.072),
            (0.620, 0.100, 0.071),
            (0.716, 0.122, 0.079),
            (0.763, 0.133, 0.077),
            (0.790, 0.120, 0.066),
            (0.810, 0.078, 0.055),
            (0.829, 0.045, 0.042),
            (0.840, 0.041, 0.038),
        ];
        let sides = if bin == 4 { 24 } else { 12 };
        let first = self.vertices.len() as u32;
        for (j, &(y, rx, rz)) in rings.iter().enumerate() {
            let (prev, next) = (
                rings[j.saturating_sub(1)],
                rings[(j + 1).min(rings.len() - 1)],
            );
            for k in 0..sides {
                let (sin, cos) = (std::f32::consts::TAU * k as f32 / sides as f32).sin_cos();
                let slope = ((prev.1 - next.1) * cos * cos + (prev.2 - next.2) * sin * sin)
                    / (next.0 - prev.0);
                self.vertices.push(Vertex {
                    pos: [rx * cos, y, rz * sin],
                    normal: Vec3::new(cos, slope, sin * rx / rz).normalize().into(),
                    center: [0., 0.65, 0.],
                    material: 1,
                    arm: 0.,
                    bin,
                });
                if j > 0 {
                    let a = first + (j as u32 - 1) * sides + k;
                    let b = first + (j as u32 - 1) * sides + (k + 1) % sides;
                    self.indices
                        .extend([a, a + sides, b, b, a + sides, b + sides]);
                }
            }
        }
    }
    fn head(&mut self) {
        let data = include_bytes!("../../../../assets/humans/head.bin");
        let count = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        let positions: Vec<Vec3> = data[8..8 + 12 * count]
            .chunks_exact(12)
            .map(|v| {
                let f = |i| f32::from_le_bytes(v[i..i + 4].try_into().unwrap());
                let y = 0.824 + (f(4) - 5.55) * 0.058;
                let x = f(0) * 0.06;
                let z = (f(8) - 0.35) * 0.06;
                // The reference crop includes part of the shoulders. Tuck the
                // lower rings inside the pullover and keep the jaw unmodified.
                let radius = (x / 0.029).hypot(z / 0.026).max(1.);
                let neck = ((y - 0.844) / 0.025).clamp(0., 1.);
                let scale = 1. / radius + neck * (1. - 1. / radius);
                Vec3::new(x * scale, y, z * scale)
            })
            .collect();
        let mut normals = vec![Vec3::ZERO; count];
        let mut eyes = vec![false; count];
        let start = self.vertices.len() as u32;
        for tri in data[8 + 12 * count..].chunks_exact(8) {
            let id = |i| u16::from_le_bytes([tri[i], tri[i + 1]]) as usize;
            let [a, b, c] = [id(0), id(2), id(4)];
            let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            for i in [a, b, c] {
                normals[i] += n;
                eyes[i] = id(6) != 0;
                self.indices.push(start + i as u32);
            }
        }
        self.vertices.extend((0..count).map(|i| Vertex {
            pos: positions[i].into(),
            normal: normals[i].normalize_or_zero().into(),
            material: if eyes[i] { 6 } else { 5 },
            center: [0., 0.91, 0.],
            arm: 0.,
            bin: 4,
        }));
    }
}

pub(super) fn stand() -> Mesh {
    let mut m = Mesh::default();
    m.part(primitives::cube(0), Vec3::ZERO, Vec3::ONE, 0, 0., 0);
    m
}

// The same part centres drive the silhouette, hand path and colour assignment at every level.
pub(super) fn person(bin: u32, far: Far, detailed: bool) -> Mesh {
    let mut m = Mesh::default();
    let mut part = |center: [f32; 3], radius: [f32; 3], material, arm| {
        let (shape, mult) = match (bin, far) {
            (1, Far::Cells) => (primitives::cube(0), 2.),
            (1, Far::Impostor) => (primitives::quad(0), 2.),
            (1, Far::Mesh) => (primitives::sphere(0, 2, 5), 1.),
            _ => (
                primitives::sphere(
                    0,
                    if detailed { 10 } else { 4 },
                    if detailed { 16 } else { 8 },
                ),
                1.,
            ),
        };
        m.part(
            shape,
            center.into(),
            Vec3::from(radius) * mult,
            material,
            arm,
            bin,
        );
    };
    // Shoes, trousers and pullover overlap slightly at their seams.
    for s in [-1., 1.] {
        part([s * 0.055, 0.035, 0.018], [0.037, 0.031, 0.073], 3, 0.);
        part([s * 0.055, 0.246, 0.], [0.045, 0.225, 0.05], 2, 0.);
    }
    part([0., 0.457, 0.], [0.104, 0.085, 0.067], 2, 0.);
    if bin == 1 {
        part([0., 0.639, 0.], [0.125, 0.195, 0.079], 1, 0.);
    }
    for s in [-1., 1.] {
        part([s * 0.118, 0.719, 0.], [0.043, 0.094, 0.045], 1, s);
        part([s * 0.130, 0.592, 0.012], [0.030, 0.085, 0.034], 4, s);
        part([s * 0.133, 0.502, 0.019], [0.024, 0.036, 0.015], 4, s);
        if detailed {
            for finger in 0..4 {
                part(
                    [s * (0.117 + 0.010 * finger as f32), 0.472, 0.019],
                    [0.006, 0.025 - (finger as f32 - 1.).abs() * 0.004, 0.007],
                    4,
                    s,
                );
            }
            part([s * 0.109, 0.505, 0.023], [0.009, 0.024, 0.010], 4, s);
        }
    }
    if !detailed {
        part([0., 0.832, 0.], [0.038, 0.047, 0.035], 4, 0.);
        part([0., 0.919, 0.012], [0.062, 0.082, 0.059], 4, 0.);
        part([0., 0.963, 0.], [0.064, 0.043, 0.060], 7, 0.);
    } else {
        m.head();
    }
    if bin != 1 {
        m.pullover(bin);
    }
    m
}
