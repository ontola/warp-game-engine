//! Rest source for the reference-backed windswept meadow profile.
//!
//! Technique/parameters: SimonDev Quick_Grass, commit
//! 6b56164272f213e353a4045d1439d071d5a968cd (MIT; see repository attribution).
//! The demo's dimensions are uniformly scaled by .4 for Sideways. The recipe
//! has six/three/one segments and 40/60/120 blades, rather than widening a
//! reduced blade set. Animation belongs to the shared GPU deformation path.

use crate::{foliage_assembly::{self, Limits, MeadowBladeBudget, Parameters},
    meadow_data::{BRANCHES, GpuMeadowVertex, LODS, MeadowGeometry, VARIANTS}};
use glam::{Quat, Vec3};

pub const WIDTH: f32 = 0.04;
pub const SEGMENTS: [usize; LODS] = [6, 3, 1];
pub const QUANTIZATION: f32 = 1024.0;

/// Shared Rust/WGSL contract. Source anchors are exact binary16-compatible
/// multiples of 1/1024; seed/variant differences are already in those anchors.
pub fn anchor_hash(anchor: [f32; 3], salt: u32) -> u32 {
    let qx = (anchor[0] * QUANTIZATION).round() as i32 as u32;
    let qz = (anchor[2] * QUANTIZATION).round() as i32 as u32;
    let mut x = qx.wrapping_mul(0x9e3779b1) ^ qz.wrapping_mul(0x85ebca77) ^ salt;
    x ^= x >> 16; x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15; x = x.wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}
fn random(anchor: Vec3, salt: u32) -> f32 {
    (anchor_hash(anchor.to_array(), salt) >> 8) as f32 / 16_777_216.0
}
pub fn lean(anchor: [f32; 3]) -> f32 {
    0.1 + 0.3 * ((anchor_hash(anchor, 0x68bc21eb) >> 8) as f32 / 16_777_216.0)
}

/// Bound the animated fine/coarse surface correspondence using the same
/// normalized (t,s) coordinates, with s in [-.5,.5]. For F=C(t)+sD(t), Fss=0;
/// barycentric Taylor plus Var(t)<=h²/4 and |Cov(t,s)|<=h/4 gives
/// E(h)<=Mtt*h²/8+Mts*h/4 on either strip triangle. Summing fine/coarse E bounds
/// their two-way distance, including different diagonals. The omitted zero-area
/// tip triangle has its image on the retained edge and preserves this mapping.
fn animated_cut_error(height: f32, radius: f32, segments: usize) -> f32 {
    let a = 1.875f64; // 1.25 source amplitude times supported 1.5 multiplier
    let l = 0.4f64;
    let width = f64::from(WIDTH);
    let mtt = f64::from(height) * (2.0 * (a + 2.0*l) + (a + 2.0*l).powi(2) + 2.0*l)
        + width * (1.0 + 2.0*a + 0.5*a*a);
    // |D'|<=W*(2t+A*(1-t²)); its maximum is A+1/A at t=1/A.
    let mts = width * (a + 1.0/a);
    let error = |h: f64| mtt*h*h/8.0 + mts*h/4.0;
    let bound = (error(1.0/6.0) + error(1.0/segments as f64)).min(2.0*f64::from(radius));
    // Outward conversion plus a small absolute arithmetic allowance.
    (bound + 0.00001) as f32
}

#[derive(Clone, Copy)]
struct Blade { anchor: Vec3, height: f32, yaw: Quat, lean: f32, branch: usize, base: Vec3, tip: Vec3, shade: f32 }
impl Blade {
    fn new(anchor: Vec3, branch: usize, dry: f32) -> Self {
        let anchor = (anchor * QUANTIZATION).round() / QUANTIZATION;
        // Preserve the descriptor exactly through the atlas RGBA16F payload.
        let height = ((0.6 * (0.75 + 0.75 * random(anchor, 0xb5297a4d)) * QUANTIZATION).round()
            .clamp((0.45 * QUANTIZATION).ceil(), (0.9 * QUANTIZATION).floor())) / QUANTIZATION;
        let dry_multiplier = Vec3::ONE.lerp(Vec3::new(0.24 / 0.105, 0.19 / 0.18, 0.075 / 0.039), dry);
        Self { anchor, height, yaw: Quat::from_rotation_y(random(anchor, 0x1b56c4e9) * std::f32::consts::TAU),
            lean: lean(anchor.to_array()), branch,
            base: Vec3::new(0.02, 0.075, 0.01).lerp(Vec3::new(0.025, 0.1, 0.01), random(anchor, 0x9e3779b9)) * dry_multiplier,
            tip: Vec3::new(0.65, 0.8, 0.25).lerp(Vec3::new(0.8, 0.9, 0.4), random(anchor, 0x7f4a7c15)) * dry_multiplier,
            shade: 0.75 + 0.25 * random(anchor, 0x94d049bb) }
    }
    fn vertex(self, t: f32, side: f32) -> GpuMeadowVertex {
        let angle = self.lean * t * t;
        let (sin, cos) = angle.sin_cos();
        let p = Vec3::new(side * WIDTH * (1.0 - t * t), self.height * t * cos, self.height * t * sin);
        // Analytic derivative of y=Ht*cos(lean*t²), z=Ht*sin(lean*t²).
        let tangent = Vec3::new(0.0, cos - 2.0 * angle * sin, sin + 2.0 * angle * cos).normalize();
        let centre_normal = Vec3::X.cross(tangent).normalize();
        let edge_angle = side * 2.0 * (std::f32::consts::PI * 0.3);
        let n = self.yaw * (centre_normal * edge_angle.cos() + Vec3::X * edge_angle.sin());
        let p = self.anchor + self.yaw * p;
        // The reference's density-one AO is baked once with its pigment.
        // The old small patch-height gradient must not be applied again.
        let rgb = (self.base.lerp(self.tip, t.powi(4)) * self.shade * (0.25 + 0.75 * t * t)).clamp(Vec3::ZERO, Vec3::ONE);
        GpuMeadowVertex { position: [p.x, p.y, p.z, self.branch as f32], normal: [n.x, n.y, n.z, t],
            color: [rgb.x, rgb.y, rgb.z, 0.9], anchor: [self.anchor.x, self.anchor.y, self.anchor.z, self.height] }
    }
}

pub(crate) fn cook(seed: u64, dry: f32, budget: MeadowBladeBudget) -> Result<MeadowGeometry, String> {
    if !dry.is_finite() || !(0.0..=1.0).contains(&dry) { return Err("invalid windswept season".into()); }
    let mut result = MeadowGeometry { vertices: Vec::new(), lod_ranges: [[[0; 2]; LODS]; VARIANTS],
        lod_errors: [[0.0; LODS]; VARIANTS], branch_pivots: [[[0.0; 4]; BRANCHES]; VARIANTS],
        max_tuft_radius: 0.0, max_tuft_height: 0.0, max_anchor_radius: 0.0, max_blade_radius: 0.0 };
    for (variant, salt) in [101u64, 303, 707].into_iter().enumerate() {
        let source = foliage_assembly::generate_meadow_patch_budget(Parameters::meadow_patch(seed ^ salt), budget, Limits::meadow_patch())?;
        let poses = source.branch_transforms()?;
        let mut group_indices = [0usize; BRANCHES];
        let blades: Vec<_> = source.instances.iter().filter_map(|placement| {
            let branch = placement.branch as usize - 1;
            let index = group_indices[branch]; group_indices[branch] += 1;
            if index % 3 != 0 { return None; }
            let anchor = (poses[placement.branch as usize] * placement.rest).transform_point3(Vec3::ZERO);
            Some(Blade::new(anchor, branch, dry))
        }).collect();
        if blades.len() != budget.blades() / 3 { return Err("windswept blade budget mismatch".into()); }
        let mut anchors = std::collections::BTreeSet::new();
        for blade in &blades {
            if !anchors.insert(blade.anchor.to_array().map(f32::to_bits)) { return Err("windswept anchor quantization collision".into()); }
            result.max_anchor_radius = result.max_anchor_radius.max(blade.anchor.length());
            // All rest and animated vertices are successive rotations of a
            // root-relative vector of at most this length, even with an angle
            // varying with height. The .001 guard covers cook-time f32 error.
            result.max_blade_radius = result.max_blade_radius.max((blade.height * blade.height + WIDTH * WIDTH * 0.25).sqrt() + 0.001);
        }
        for branch in 0..BRANCHES { result.branch_pivots[variant][branch] = [0.0, 0.0, 0.0, 1.25]; }
        for (lod, segments) in SEGMENTS.into_iter().enumerate() {
            let first = result.vertices.len() as u32;
            for &blade in &blades {
                for segment in 0..segments {
                    let a = segment as f32 / segments as f32;
                    let b = (segment + 1) as f32 / segments as f32;
                    let left = blade.vertex(a, -0.5); let right = blade.vertex(a, 0.5);
                    if segment + 1 == segments {
                        result.vertices.extend([left, right, blade.vertex(1.0, 0.0)]);
                    } else {
                        let next_left = blade.vertex(b, -0.5); let next_right = blade.vertex(b, 0.5);
                        result.vertices.extend([left, right, next_left, next_left, right, next_right]);
                    }
                }
            }
            result.lod_ranges[variant][lod] = [first, result.vertices.len() as u32 - first];
        }
    }
    for vertex in &result.vertices {
        result.max_tuft_radius = result.max_tuft_radius.max(vertex.position[0].hypot(vertex.position[2]));
        result.max_tuft_height = result.max_tuft_height.max(vertex.position[1].abs());
    }
    let height = result.vertices.iter().map(|v| v.anchor[3]).fold(0.0f32, f32::max);
    for errors in &mut result.lod_errors { *errors = [0.0,
        animated_cut_error(height, result.max_blade_radius, 3),
        animated_cut_error(height, result.max_blade_radius, 1)]; }
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windswept_anchor_hash_contract_matches_unsigned_reference_vectors() {
        for (qx, qz, expected) in [(0, 0, 0x484f3259), (-135, 283, 0x564e5b62), (243, -177, 0xab429e71)] {
            assert_eq!(anchor_hash([qx as f32 / 1024.0, 0.0, qz as f32 / 1024.0], 0x68bc21eb), expected);
        }
    }
    #[test]
    fn windswept_reference_formula_endpoints_and_quantized_descriptor() {
        let blade = Blade::new(Vec3::new(-0.13213, 0.0, 0.27631), 2, 0.0);
        for value in [blade.anchor.x, blade.anchor.z, blade.height] { assert_eq!((value * 1024.0).fract(), 0.0); }
        assert!((0.45..=0.9).contains(&blade.height));
        assert!((0.1..=0.4).contains(&blade.lean));
        let root = blade.vertex(0.0, 0.0); let tip = blade.vertex(1.0, 0.0);
        assert_eq!(Vec3::from_slice(&root.position), blade.anchor);
        assert!((Vec3::from_slice(&tip.position).distance(blade.anchor) - blade.height).abs() < 1e-6);
        assert!((tip.position[1] / blade.height - blade.lean.cos()).abs() < 1e-6);
        assert_eq!(root.anchor, tip.anchor);
        assert!(tip.color[0] > root.color[0] * 50.0);
        assert!((blade.vertex(0.5, -0.5).position[1] / blade.height - 0.5 * (blade.lean * 0.25).cos()).abs() < 1e-6);
        let a = Vec3::from_slice(&blade.vertex(0.5, -0.5).position);
        let b = Vec3::from_slice(&blade.vertex(0.5, 0.5).position);
        assert!((a.distance(b) - WIDTH * 0.75).abs() < 1e-6);
    }

    #[test]
    fn windswept_cuts_keep_anchors_tip_metadata_and_conservative_sweep_bounds() {
        for budget in [MeadowBladeBudget::Blades120, MeadowBladeBudget::Blades180, MeadowBladeBudget::Blades360] {
            let field = cook(73, 0.0, budget).unwrap();
            assert_eq!(field.vertices, cook(73, 0.0, budget).unwrap().vertices);
            for variant in 0..VARIANTS {
                let mut reference = std::collections::BTreeSet::new();
                for lod in 0..LODS {
                    let [first, count] = field.lod_ranges[variant][lod];
                    assert_eq!(count as usize / 3, budget.blades() / 3 * (SEGMENTS[lod] * 2 - 1));
                    let mut anchors = std::collections::BTreeSet::new();
                    for triangle in field.vertices[first as usize..][..count as usize].chunks_exact(3) {
                        assert!(triangle.iter().all(|v| v.anchor == triangle[0].anchor));
                        let p: [Vec3; 3] = std::array::from_fn(|i| Vec3::from_slice(&triangle[i].position));
                        assert!((p[1] - p[0]).cross(p[2] - p[0]).length_squared() > 1e-14);
                        for v in triangle {
                            anchors.insert(v.anchor.map(f32::to_bits));
                            assert!((0.0..=1.0).contains(&v.normal[3]));
                            let relative = Vec3::from_slice(&v.position) - Vec3::from_slice(&v.anchor);
                            for angle in [-1.875f32, 0.0, 1.875] {
                                let rotated = Quat::from_axis_angle(Vec3::new(0.6, 0.0, 0.8), angle * v.normal[3]) * relative;
                                assert!(rotated.length() <= field.max_blade_radius);
                            }
                        }
                    }
                    assert_eq!(anchors.len(), budget.blades() / 3);
                    if lod == 0 { reference = anchors; } else { assert_eq!(anchors, reference); }
                }
            }
            assert!(field.lod_errors.iter().all(|e| e[0] == 0.0 && e[1] > 0.0 && e[2] >= e[1]
                && e[2] <= 2.0 * field.max_blade_radius + 0.00002));
        }
    }

    #[test]
    fn windswept_voxel_samples_keep_height_and_baked_reference_pigment_once() {
        use crate::meadow_voxel_samples::{MeadowVoxelSamples, SampleConfig};
        let geometry = cook(17, 0.0, MeadowBladeBudget::Blades120).unwrap();
        let samples = MeadowVoxelSamples::cook(&geometry, SampleConfig {
            spacing: 10.0, max_per_triangle: 1, ..SampleConfig::default()
        }).unwrap();
        for variant in 0..VARIANTS {
            let [start, count, _, _] = samples.ranges[variant];
            let [first, vertices] = geometry.lod_ranges[variant][0];
            assert_eq!(count * 3, vertices);
            for (sample, triangle) in samples.samples[start as usize..][..count as usize].iter()
                .zip(geometry.vertices[first as usize..][..vertices as usize].chunks_exact(3)) {
                assert_eq!(sample.shading_normal[3], triangle[0].anchor[3]);
                let expected = triangle.iter().map(|v| Vec3::from_slice(&v.color)).sum::<Vec3>() / 3.0;
                assert!(Vec3::from_slice(&sample.albedo).distance(expected) < 1e-7,
                    "reference pigment/AO is not multiplied by the legacy height gradient");
            }
        }
        let summer = geometry;
        let dry = cook(17, 1.0, MeadowBladeBudget::Blades120).unwrap();
        for (a, b) in summer.vertices.iter().zip(&dry.vertices) {
            assert_eq!(a.position, b.position); assert_eq!(a.normal, b.normal); assert_eq!(a.anchor, b.anchor);
        }
        assert!(summer.vertices.iter().zip(&dry.vertices).any(|(a,b)|a.color!=b.color));
        assert!(cook(17, f32::NAN, MeadowBladeBudget::Blades120).is_err());
    }

    #[test]
    fn windswept_animated_cut_bound_encloses_dense_triangle_correspondences() {
        let interpolate = |blade: Blade, segments: usize, t: f32, s: f32, axis: Vec3, amplitude: f32| {
            let row = ((t * segments as f32).floor() as usize).min(segments - 1);
            let fraction = t * segments as f32 - row as f32;
            let a = row as f32 / segments as f32; let b = (row + 1) as f32 / segments as f32;
            let vertex = |t: f32, s: f32| {
                let p = Vec3::from_slice(&blade.vertex(t, s).position) - blade.anchor;
                Quat::from_axis_angle(axis, amplitude * t) * p
            };
            let u = s + 0.5;
            if u + fraction <= 1.0 {
                vertex(a,-0.5)*(1.0-u-fraction)+vertex(a,0.5)*u+vertex(b,-0.5)*fraction
            } else {
                vertex(b,-0.5)*(1.0-u)+vertex(a,0.5)*(1.0-fraction)+vertex(b,0.5)*(u+fraction-1.0)
            }
        };
        for lean in [0.1, 0.25, 0.4] {
            let mut blade = Blade::new(Vec3::new(-0.125,0.0,0.25),0,0.0); blade.lean=lean; blade.height=0.9;
            let radius = (blade.height*blade.height+WIDTH*WIDTH*0.25).sqrt()+0.001;
            for axis in [Vec3::X,Vec3::Z,Vec3::new(0.6,0.0,0.8)] {
                for amplitude in [-1.875,0.0,1.875] {
                    for segments in [3,1] {
                        let bound = animated_cut_error(blade.height,radius,segments);
                        for i in 0..=120 { for j in 0..=20 {
                            let t=i as f32/120.0;let s=j as f32/20.0-0.5;
                            let fine=interpolate(blade,6,t,s,axis,amplitude);
                            let coarse=interpolate(blade,segments,t,s,axis,amplitude);
                            assert!(fine.distance(coarse)<=bound,"animated {segments}-segment error escaped bound");
                        }}
                    }
                }
            }
        }
    }
}
