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

// All crowd tiers share a compact GPU layout. Position covers +/-1.2 body heights.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct PackedVertex {
    position: [i16; 4],
    normal: [i8; 4],
    center: [i16; 4],
    tags: [u8; 4],
    bones: [u8; 4],
    morphs: [[i8; 4]; 4],
}
impl Vertex {
    pub(super) fn packed(&self) -> PackedVertex {
        let xyz = |p: [f32; 3], scale: f32| {
            let [x, y, z] = p.map(|v| ((v / scale).clamp(-1., 1.) * 32767.).round() as i16);
            [x, y, z, 0]
        };
        let [x, y, z] = self.normal.map(|v| (v.clamp(-1., 1.) * 127.).round() as i8);
        PackedVertex {
            position: xyz(self.pos, 1.2),
            normal: [x, y, z, 0],
            center: xyz(self.center, 1.),
            tags: [
                self.material as u8,
                (self.arm + 1.) as u8,
                self.bin as u8,
                0,
            ],
            bones: self.bones,
            morphs: self.morphs,
        }
    }
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
    pub(super) fn anatomical() -> Self {
        let bytes = include_bytes!("../../../../assets/humans/person.bin");
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[..4], b"WH04");
        let count = u32_at(4);
        let split = 32 + count * 22;
        let mut vertices: Vec<Vertex> = bytes[32..split]
            .chunks_exact(22)
            .map(|s| {
                let signed = |i| i16::from_le_bytes([s[i], s[i + 1]]) as f32 / 32767.;
                let word = |i| u16::from_le_bytes([s[i], s[i + 1]]);
                Vertex {
                    pos: [0, 2, 4].map(|i| signed(i) * 1.2),
                    normal: [6, 8, 10].map(signed),
                    center: [word(12) as f32 / 65535., word(14) as f32 / 65535., -1.],
                    bones: s[16..20].try_into().unwrap(),
                    morphs: {
                        let mut fields = [[0; 4]; 4];
                        fields[3][3] = (s[21] as f32 * (127. / 255.)).round() as i8;
                        fields
                    },
                    bin: 4,
                    material: s[20] as u32,
                    arm: 0.,
                }
            })
            .collect();
        let mut cursor = split + u32_at(8) * 2;
        let indices = bytes[split..cursor]
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes(v.try_into().unwrap()) as u32)
            .collect::<Vec<_>>();
        for channel in 0..4 {
            let end = cursor + u32_at(12 + channel * 4) * 5;
            for entry in bytes[cursor..end].chunks_exact(5) {
                let index = u16::from_le_bytes([entry[0], entry[1]]) as usize;
                vertices[index].morphs[channel][..3].copy_from_slice(&[entry[2] as i8, entry[3] as i8, entry[4] as i8]);
            }
            cursor = end;
        }
        // Store the closure target in unused components of the four GPU morphs.
        let end = cursor + u32_at(28) * 5;
        for entry in bytes[cursor..end].chunks_exact(5) {
            let v = &mut vertices[u16::from_le_bytes([entry[0], entry[1]]) as usize];
            for axis in 0..3 { v.morphs[axis][3] = entry[2 + axis] as i8; }
        }
        assert_eq!(end, bytes.len());
        Self { vertices, indices }
    }
}

/// Bind-pose geometry for a full-size actor using the same face morphs and
/// selected hair as the crowd. Arm/leg masks are cumulative along each limb.
pub struct BodyVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: [f32; 2],
    pub limb_weights: [f32; 4],
    pub lid_closure: Vec3,
    pub head_weight: f32,
    pub material: u32,
}

/// hm08 palm axes: along the fingers, out of the back of the hand, across it.
pub fn hand_frame(left: bool) -> glam::Quat {
    let sign = if left { 1. } else { -1. };
    let along = Vec3::new(sign * 0.011, -0.035, 0.032).normalize();
    let back = Vec3::new(sign * 0.83, 0.55, 0.11);
    let back = (back - along * along.dot(back)).normalize();
    glam::Quat::from_mat3(&glam::Mat3::from_cols(along, back, along.cross(back)))
}

// Bend the distal fingers around the grip at roster fitting time. The thumb
// is outside the knuckle band; the trigger finger stays more extended.
fn curled_hand(p: Vec3, n: Vec3, frame: glam::Quat) -> (Vec3, Vec3) {
    let sign = p.x.signum();
    let knuckle = Vec3::new(sign * 0.276, 0.603, 0.128);
    let local = frame.inverse() * (p - knuckle);
    let across = sign * local.z;
    if local.x <= 0. || across > 0.030 { return (p, n); }
    let radius = if sign < 0. && across > 0.010 { 0.041 } else { 0.022 };
    let angle = (local.x / radius).min(2.3);
    let (sin, cos) = angle.sin_cos();
    let tip = (local.x - radius * angle).max(0.);
    let bent = Vec3::new((radius + local.y) * sin + tip * cos,
        (radius + local.y) * cos - radius - tip * sin, local.z);
    (knuckle + frame * bent, frame * glam::Quat::from_rotation_z(-angle) * frame.inverse() * n)
}

impl super::Instance {
    // Nonnegative weights keep every landmark inside the four registered
    // reference heads. Squaring twice still allows distinctive individuals.
    pub fn head_weights(&self) -> [f32; 4] {
        let w = self.identity.map(|x| { let x2 = x * x; x2 * x2 + 0.001 });
        let sum = w.iter().sum::<f32>();
        w.map(|x| x / sum)
    }

    // XY cross-sections use the same shape as the instanced crowd shader.
    fn regional_breadths(&self) -> [f32; 3] {
        let bits = |value: f32| (value * 16777216.) as u32 & 4095;
        let packed = bits(self.size_phase[2]) | (bits(self.size_phase[3]) << 12);
        [0, 8, 16].map(|shift| (((packed >> shift) & 255u32) as f32 - 128.) * (0.15 / 127.))
    }

    fn width_at(regions: [f32; 3], y: f32) -> f32 {
        let zone = |center: f32, span: f32| {
            let t = (1. - (y - center).abs() / span).clamp(0., 1.);
            t * t * (3. - 2. * t)
        };
        1. + regions[0] * zone(0.78, 0.12) + regions[1] * zone(0.57, 0.13)
            + regions[2] * zone(0.48, 0.11)
    }
    pub fn body_mesh(&self) -> (Vec<BodyVertex>, Vec<u32>) {
        self.fitted_mesh(false)
    }

    pub fn fitted_mesh(&self, gripping: bool) -> (Vec<BodyVertex>, Vec<u32>) {
        let hands = [hand_frame(false), hand_frame(true)];
        static BODY: std::sync::OnceLock<Mesh> = std::sync::OnceLock::new();
        let mesh = BODY.get_or_init(Mesh::anatomical);
        let hair = 20 + (self.identity[2] * 3.0) as u32;
        let weights = self.head_weights();
        let regions = self.regional_breadths();
        let mut vertices = Vec::new();
        let mut remap = vec![u32::MAX; mesh.vertices.len()];
        for (i, v) in mesh.vertices.iter().enumerate() {
            let mask = v.material >> 5;
            let material = v.material & 31;
            if (mask != 0 && mask & (1 << self.outfit()) == 0)
                || (material >= 20 && material != hair)
            {
                continue;
            }
            let mut p = Vec3::from(v.pos);
            for (morph, weight) in v.morphs.iter().zip(weights) {
                let d = Vec3::new(morph[0] as f32, morph[1] as f32, morph[2] as f32);
                p += d * (0.055 / 127.0 * weight);
            }
            if material == 14 && p.y < 0.18 {
                let cuff = (1. - ((p.y - 0.071) / 0.095).abs()).clamp(0., 1.);
                let axis = p.x.signum() * (0.132 - (p.y - 0.043) * 0.16);
                p.x += (p.x - axis) * 0.25 * cuff;
                p.z += (p.z + 0.015) * 0.18 * cuff;
            }
            let mut n = Vec3::from(v.normal);
            if gripping && material == 4 && v.bones[1] > 220 && p.x.abs() > 0.22 {
                (p, n) = curled_hand(p, n, hands[usize::from(p.x > 0.)]);
            }
            let head = v.morphs[3][3] as f32 / 127.;
            let skull = 1. + head * (self.size_phase[1] / self.size_phase[0] - 1.);
            let width = 1. + (1. - head) * (Self::width_at(regions, v.pos[1]) - 1.);
            p.x *= width * skull;
            p.z *= skull;
            n.x /= width * skull;
            n.z /= skull;
            remap[i] = vertices.len() as u32;
            vertices.push(BodyVertex {
                position: p,
                normal: n.normalize_or(Vec3::Y),
                uv: [v.center[0], v.center[1]],
                head_weight: head,
                lid_closure: Vec3::from_array([0, 1, 2].map(|axis| v.morphs[axis][3] as f32 * (0.012 / 127.))),
                limb_weights: v.bones.map(|w| w as f32 / 255.0),
                material,
            });
        }
        // Rotate the artist's smooth normal by the change in surface orientation.
        // Vertices split for UVs share the same position key at a face seam.
        let mut normals = std::collections::HashMap::<[i16; 3], [Vec3; 2]>::new();
        let key = |v: &Vertex| v.pos.map(|p| (p * (32767. / 1.2)).round() as i16);
        for tri in mesh.indices.chunks_exact(3) {
            let ids = [tri[0], tri[1], tri[2]].map(|i| i as usize);
            let v = &mesh.vertices[ids[0]];
            if v.pos[1] < 0.84 || !matches!(v.material & 31, 4 | 11) || remap[ids[0]] == u32::MAX { continue; }
            let [a, b, c] = ids.map(|i| Vec3::from(mesh.vertices[i].pos));
            let [x, y, z] = ids.map(|i| vertices[remap[i] as usize].position);
            let before = (b-a).cross(c-a);
            let after = (y-x).cross(z-x);
            for i in ids {
                let pair = normals.entry(key(&mesh.vertices[i])).or_default();
                pair[0] += before;
                pair[1] += after;
            }
        }
        for (v, &i) in mesh.vertices.iter().zip(&remap) {
            if i == u32::MAX || v.pos[1] < 0.84 || !matches!(v.material & 31, 4 | 11) { continue; }
            if let Some([a,b]) = normals.get(&key(v))
                && let (Some(a), Some(b)) = (a.try_normalize(), b.try_normalize()) {
                vertices[i as usize].normal = glam::Quat::from_rotation_arc(a,b) * Vec3::from(v.normal);
            }
        }
        let indices = mesh
            .indices
            .iter()
            .map(|&i| remap[i as usize])
            .filter(|&i| i != u32::MAX)
            .collect();
        (vertices, indices)
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
    fn heights_and_body_sections_cluster_and_vary_together() {
        use super::super::{Constraints, Instance, seeded_height};
        let samples = 16384;
        let mut central = 0;
        let mut short = 0;
        let mut tall = 0;
        let mut sum = [0.; 3];
        let mut squares = [0.; 3];
        let mut products = [0.; 3];
        let mut bodies = Vec::new();
        for seed in 0..samples {
            let height = seeded_height(seed, [1.60, 1.90]);
            assert!((1.60..=1.90).contains(&height));
            assert_eq!(seeded_height(seed, [1.77, 1.77]), 1.77);
            central += usize::from((height - 1.75).abs() <= 0.05);
            short += usize::from(height < 1.65);
            tall += usize::from(height > 1.85);
            let person = Instance::person(seed, Vec3::ZERO, 0., &Constraints {
                height_m: Some(1.75), shirt: None,
            });
            let factors = [0.78, 0.57, 0.48].map(|y| {
                (Instance::width_at(person.regional_breadths(), y) * person.body_scale().x / 1.75) as f64
            });
            for i in 0..3 {
                assert!((0.68..1.38).contains(&factors[i]));
                sum[i] += factors[i];
                squares[i] += factors[i] * factors[i];
                products[i] += factors[i] * factors[(i + 1) % 3];
            }
            bodies.push(factors);
        }
        let n = samples as f64;
        let mean = sum.map(|x| x / n);
        let std: [f64; 3] = std::array::from_fn(|i| (squares[i] / n - mean[i].powi(2)).sqrt());
        let correlation: [f64; 3] = std::array::from_fn(|i| {
            let j = (i + 1) % 3;
            (products[i] / n - mean[i] * mean[j]) / (std[i] * std[j])
        });
        assert!((0.65..0.70).contains(&(central as f64 / n)));
        for tail in [short, tall] {
            assert!((0.012..0.030).contains(&(tail as f64 / n)));
        }
        assert!(mean.iter().all(|x| (0.99..1.01).contains(x)));
        assert!(std.iter().all(|x| (0.035..0.10).contains(x)));
        assert!(correlation.iter().all(|x| (0.55..0.995).contains(x)));
        let beyond_two: [f64; 3] = std::array::from_fn(|i| {
            bodies.iter().filter(|b| (b[i] - mean[i]).abs() > 2. * std[i]).count() as f64 / n
        });
        assert!(beyond_two.iter().all(|x| (0.025..0.06).contains(x)));
        println!("Chest/waist/hip fractions outside ±2σ {beyond_two:.3?}");
        println!("{samples} seeds: middle ±1σ {central}, lower/upper >2σ {short}/{tall}, chest/waist/hip correlations {correlation:.3?}");
    }

    #[test]
    fn head_blend_is_bounded_and_independent_of_build() {
        let mut slim = super::super::Instance::person(23, Vec3::ZERO, 0., &Default::default());
        for identity in [[0.; 4], [1.; 4], [0., 1., 0., 0.], slim.identity] {
            slim.identity = identity;
            let weights = slim.head_weights();
            assert!(weights.iter().all(|&w| (0.0..=1.0).contains(&w)));
            assert!((weights.iter().sum::<f32>() - 1.).abs() < 1e-6);
        }
        slim.size_phase = [1.8 * 0.84, 1.8, 0., 0.];
        let mut broad = slim;
        broad.size_phase = [1.8 * 1.15, 1.8, 1., 1.];
        let mut count = 0;
        let mut lower_face = 0;
        for (a, b) in slim.body_mesh().0.iter().zip(broad.body_mesh().0) {
            if a.material == 11 && a.position.x.abs() < 0.012 && a.position.z > 0.065
                && (0.856..0.905).contains(&a.position.y) {
                assert!(a.head_weight > 0.95, "lower face must turn with the skull");
                lower_face += 1;
            }
            if a.material == 11 && a.head_weight == 1. {
                assert!((a.position * slim.body_scale()).distance(b.position * broad.body_scale()) < 1e-5);
                count += 1;
            }
        }
        assert!(count > 1000 && lower_face > 100);
    }

    #[test]
    fn outfits_share_a_body_and_fit_the_crowd_budget() {
        let mesh = Mesh::anatomical();
        assert_eq!(std::mem::size_of::<super::super::Instance>(), 64);
        assert_eq!(std::mem::size_of::<PackedVertex>(), 44);
        assert!(mesh.vertices.len() <= 65536);
        let mut tops = [0; 3];
        for tri in mesh.indices.chunks_exact(3) {
            let tag = mesh.vertices[tri[0] as usize].material;
            assert!(
                tri.iter()
                    .all(|&i| mesh.vertices[i as usize].material == tag)
            );
            if tag & 31 == 12 {
                let mask = tag >> 5;
                assert!(matches!(mask, 1 | 2 | 4));
                tops[mask.trailing_zeros() as usize] += 1;
            }
        }
        assert!(tops.iter().all(|&n| n > 100));
        assert!(mesh.vertices.iter().any(|v| v.material == 14 + (6 << 5)));
        assert!(mesh.vertices.iter().any(|v| v.material == 3));
    }

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
                .filter(|v| (v.material & 31) == 12 && v.pos[0] * side > 0.24 && v.pos[1] > 0.55)
                .collect();
            assert!(!sleeve.is_empty());
            assert!(sleeve.iter().all(|v| v.bones[0] > 216 && v.bones[1] > 216));
        }
        assert!(
            mesh.vertices
                .iter()
                .any(|v| (v.material & 31) == 14 && v.pos[1] < 0.2 && v.bones[3] > 240)
        );
        for material in [13, 20, 21, 22] {
            assert!(mesh.vertices.iter().any(|v| v.material == material));
        }
    }
}
