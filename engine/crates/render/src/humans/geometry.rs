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
    bones: [u8; 4],
    morphs: [[i8; 4]; 4],
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
            ..Vertex::zeroed()
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
                    ..Vertex::zeroed()
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
    fn anatomical() -> Self {
        let bytes = include_bytes!("../../../../assets/humans/person.bin");
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let count = u32_at(0);
        let split = 8 + count * 38;
        let vertices = bytes[8..split]
            .chunks_exact(38)
            .map(|s| {
                let signed = |i| i16::from_le_bytes([s[i], s[i + 1]]) as f32 / 32767.;
                let word = |i| u16::from_le_bytes([s[i], s[i + 1]]);
                Vertex {
                    pos: [0, 2, 4].map(|i| signed(i) * 1.2),
                    normal: [6, 8, 10].map(signed),
                    center: [word(12) as f32 / 65535., word(14) as f32 / 65535., -1.],
                    bones: s[16..20].try_into().unwrap(),
                    morphs: std::array::from_fn(|m| {
                        std::array::from_fn(|c| s[20 + m * 4 + c] as i8)
                    }),
                    bin: 4,
                    material: word(36) as u32,
                    arm: 0.,
                }
            })
            .collect();
        let indices = bytes[split..]
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(indices.len(), u32_at(4));
        Self { vertices, indices }
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
    if detailed {
        return Mesh::anatomical();
    }
    let mut part = |center: [f32; 3], radius: [f32; 3], material, arm| {
        let (shape, mult) = match (bin, far) {
            (1, Far::Cells) => (primitives::cube(0), 2.),
            (1, Far::Impostor) => (primitives::quad(0), 2.),
            (1, Far::Mesh) => (primitives::sphere(0, 2, 5), 1.),
            _ => (primitives::sphere(0, 4, 8), 1.),
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
        if bin == 1 {
            part([s * 0.055, 0.246, 0.], [0.045, 0.225, 0.05], 2, 0.);
        } else {
            part([s * 0.062, 0.365, 0.], [0.050, 0.139, 0.055], 2, 0.);
            part([s * 0.065, 0.144, 0.], [0.035, 0.114, 0.041], 2, 0.);
        }
    }
    part([0., 0.457, 0.], [0.104, 0.085, 0.067], 2, 0.);
    if bin == 1 {
        part([0., 0.639, 0.], [0.125, 0.195, 0.079], 1, 0.);
    }
    for s in [-1., 1.] {
        part([s * 0.118, 0.719, 0.], [0.043, 0.094, 0.045], 1, s);
        part([s * 0.130, 0.592, 0.012], [0.030, 0.085, 0.034], 4, s);
        part([s * 0.133, 0.502, 0.019], [0.024, 0.036, 0.015], 4, s);
    }
    part([0., 0.832, 0.], [0.038, 0.047, 0.035], 4, 0.);
    part([0., 0.919, 0.012], [0.062, 0.082, 0.059], 4, 0.);
    part([0., 0.963, 0.], [0.064, 0.043, 0.060], 7, 0.);
    if bin != 1 {
        m.pullover(bin);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooked_body_retains_eyes_and_follows_the_limbs() {
        let mesh = Mesh::anatomical();
        assert!(
            mesh.indices
                .iter()
                .all(|&i| (i as usize) < mesh.vertices.len())
        );
        let eyes: Vec<_> = mesh.vertices.iter().filter(|v| v.material == 6).collect();
        for side in [-1., 1.] {
            assert!(eyes.iter().any(|v| v.pos[0] * side > 0.012));
            let sleeve: Vec<_> = mesh
                .vertices
                .iter()
                .filter(|v| v.material == 12 && v.pos[0] * side > 0.24 && v.pos[1] > 0.55)
                .collect();
            assert!(!sleeve.is_empty());
            assert!(sleeve.iter().all(|v| v.bones[0] > 216 && v.bones[1] > 216));
        }
        assert!(
            mesh.vertices
                .iter()
                .any(|v| v.material == 12 && v.pos[1] < 0.2 && v.bones[3] > 240)
        );
        for material in [13, 20, 21, 22] {
            assert!(mesh.vertices.iter().any(|v| v.material == material));
        }
    }
}
