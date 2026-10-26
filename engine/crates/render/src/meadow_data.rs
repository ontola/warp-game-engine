//! Reusable source geometry and terrain data for GPU-generated meadow cover.
//! No per-plant CPU instances or branch-pose pool are required. All three
//! source LODs retain every blade; they are cuts of opaque curved ribbons,
//! not alpha rectangles, density thinning, or a statistical replacement colour.

use crate::{
    cluster::{ClusterMesh, Part},
    foliage_assembly::{self, Limits, Parameters, MeadowBladeBudget},
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Vec3};

pub const VARIANTS: usize = 3;
pub const LODS: usize = 3;
pub const BRANCHES: usize = 4;
pub const BLADES: usize = 360;
pub const MAX_FIELD_SAMPLES: usize = 8_000_000;
pub const ROOT_CELL: f32 = 0.45;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuMeadowVertex {
    /// Assembly-rest XYZ; W is the zero-based branch index, exactly 0..3.
    pub position: [f32; 4],
    /// Actual inverse-transpose transformed source normal; W reserved zero.
    pub normal: [f32; 4],
    /// Linear material albedo RGB, roughness W. Season is already applied.
    pub color: [f32; 4],
    /// Unscaled basal anchor in the world-cell patch; W reserved. Wind and
    /// growth act around this blade root, not a shared pivot that lifts roots.
    pub anchor: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct MeadowGeometry {
    pub vertices: Vec<GpuMeadowVertex>,
    /// [variant][LOD] = [first vertex, vertex count], flat triangle streams.
    pub lod_ranges: [[[u32; 2]; LODS]; VARIANTS],
    /// Conservative assembly-space geometry error, before instance growth.
    pub lod_errors: [[f32; LODS]; VARIANTS],
    /// [variant][branch] = assembly-rest XYZ pivot and maximum bend angle.
    pub branch_pivots: [[[f32; 4]; BRANCHES]; VARIANTS],
    /// Rest radial and absolute-Y bounds, before root scale/branch bending.
    /// These must enclose every vertex in every variant and source LOD.
    pub max_tuft_radius: f32,
    pub max_tuft_height: f32,
    /// Rounded source-footprint radius stays fixed when growth scales blades.
    pub max_anchor_radius: f32,
    /// Largest root-to-vertex distance over every source LOD.
    pub max_blade_radius: f32,
}

impl MeadowGeometry {
    /// Cook only three reusable assets. Growth stays at one here and is
    /// applied once at the world root. A coarser requested level repeats the
    /// last accepted source level when that shape cannot simplify further.
    pub fn cook(seed: u64, dry_season: f32) -> Result<Self, String> {
        Self::cook_with_blade_budget(seed, dry_season, MeadowBladeBudget::Blades360)
    }

    /// Lower-cost authored patch. Source cuts, normals, bounds and every
    /// later aggregate/substrate cook consume this same altered geometry.
    /// The default cook remains the complete360-blade recipe.
    pub fn cook_with_blade_budget(seed: u64, dry_season: f32, budget: MeadowBladeBudget) -> Result<Self, String> {
        let mut result = Self {
            vertices: Vec::new(),
            lod_ranges: [[[0; 2]; LODS]; VARIANTS],
            lod_errors: [[0.0; LODS]; VARIANTS],
            branch_pivots: [[[0.0; 4]; BRANCHES]; VARIANTS],
            max_tuft_radius: 0.0,
            max_tuft_height: 0.0,
            max_anchor_radius: 0.0,
            max_blade_radius: 0.0,
        };
        for (variant, salt) in [101u64, 303, 707].into_iter().enumerate() {
            let mut parameters = Parameters::meadow_patch(seed ^ salt);
            parameters.season = dry_season;
            let source = foliage_assembly::generate_meadow_patch_budget(parameters, budget, Limits::meadow_patch())?;
            if source.instances.len() != budget.blades() || source.branches.len() != BRANCHES + 1 {
                return Err("meadow recipe changed its explicit blade/branch budget".into());
            }
            let poses = source.branch_transforms()?;
            for branch in 0..BRANCHES {
                let p = poses[branch + 1].transform_point3(Vec3::ZERO);
                result.branch_pivots[variant][branch] = [
                    p.x,
                    p.y,
                    p.z,
                    0.22 * (1.0 - source.branches[branch + 1].stiffness).clamp(0.15, 1.0),
                ];
            }
            let meshes: Vec<ClusterMesh> = source
                .parts
                .iter()
                .map(|geometry| {
                    crate::vegetation_clod::build_ribbon(&Part {
                        vertices: &geometry.vertices,
                        indices: &geometry.indices,
                        material: 0,
                    })
                    .ok_or_else(|| {
                        "meadow source no longer has a recognized opaque ribbon CLOD".to_string()
                    })
                })
                .collect::<Result<_, _>>()?;
            // Source/cut correspondences are cached once per shared primitive.
            // Only their few directional deviations are transformed per blade;
            // no per-placement surface search or repeated geometry cooking.
            let ribbon_errors: Vec<_> = source.parts.iter().zip(&meshes).map(|(geometry, mesh)| {
                crate::vegetation_clod::ribbon_error_bounds(&Part {
                    vertices: &geometry.vertices, indices: &geometry.indices, material: 0,
                }, mesh)
            }).collect();
            for lod in 0..LODS {
                let first = result.vertices.len() as u32;
                for placement in &source.instances {
                    if placement.branch == 0 || placement.branch as usize > BRANCHES {
                        return Err("meadow blade has no supported branch".into());
                    }
                    let mesh = &meshes[placement.part as usize];
                    let level_index = lod.min(mesh.clusters.len() - 1);
                    let level = &mesh.clusters[level_index];
                    let transform = poses[placement.branch as usize] * placement.rest;
                    let anchor = transform.transform_point3(Vec3::ZERO);
                    let linear = Mat3::from_mat4(transform);
                    let normal_matrix = linear.inverse().transpose();
                    // General fallback remains valid for twisted future assets.
                    // Recognized planar strips instead transform a proven
                    // same-height correspondence by this exact affine matrix:
                    // the small width-axis bow need not inherit height scaling.
                    let scale_bound = (linear.x_axis.length_squared()
                        + linear.y_axis.length_squared()
                        + linear.z_axis.length_squared())
                    .sqrt();
                    let scalar_error = level.own_error * scale_bound;
                    let error = ribbon_errors[placement.part as usize].as_ref()
                        .and_then(|levels| levels.get(level_index))
                        .and_then(|bound| bound.transformed_error(transform))
                        .map_or(scalar_error, |physical| physical.min(scalar_error));
                    result.lod_errors[variant][lod] =
                        result.lod_errors[variant][lod].max(error);
                    let material = source.materials[placement.material as usize];
                    let geometry = &source.parts[placement.part as usize];
                    let level_vertices: Vec<_> = if lod == 0 {
                        // Retain source triangle order too, independently of
                        // any cluster builder reordering its finest cut.
                        geometry
                            .indices
                            .iter()
                            .map(|&i| geometry.vertices[i as usize])
                            .collect()
                    } else {
                        mesh.tris[level.first_tri as usize..][..level.tri_count as usize]
                            .iter()
                            .flat_map(|&packed| {
                                [packed & 255, (packed >> 8) & 255, (packed >> 16) & 255].map(
                                    |local| {
                                        mesh.vertices[mesh.cluster_verts
                                            [(level.first_vert + local) as usize]
                                            as usize]
                                    },
                                )
                            })
                            .collect()
                    };
                    for vertex in level_vertices {
                        let p = transform.transform_point3(Vec3::from(vertex.pos));
                        let n = (normal_matrix * Vec3::from(vertex.normal)).normalize();
                        result.max_tuft_radius = result.max_tuft_radius.max(p.x.hypot(p.z));
                        result.max_tuft_height = result.max_tuft_height.max(p.y.abs());
                        result.max_anchor_radius = result.max_anchor_radius.max(anchor.length());
                        result.max_blade_radius = result.max_blade_radius.max(p.distance(anchor));
                        result.vertices.push(GpuMeadowVertex {
                            position: [p.x, p.y, p.z, (placement.branch - 1) as f32],
                            normal: [n.x, n.y, n.z, 0.0],
                            color: [
                                material.tint[0],
                                material.tint[1],
                                material.tint[2],
                                material.roughness,
                            ],
                            anchor: [anchor.x, anchor.y, anchor.z, 0.0],
                        });
                    }
                }
                result.lod_ranges[variant][lod] = [first, result.vertices.len() as u32 - first];
                // Different affine placements can have different tightest
                // bounds. Keep cut thresholds monotone without reducing any
                // individual geometric bound or altering accepted geometry.
                if lod > 0 {
                    result.lod_errors[variant][lod] = result.lod_errors[variant][lod]
                        .max(result.lod_errors[variant][lod - 1]);
                }
            }
        }
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.vertices.is_empty()
            || self.vertices.len() > 100_000
            || !self.max_tuft_radius.is_finite()
            || self.max_tuft_radius <= 0.0
            || !self.max_tuft_height.is_finite()
            || self.max_tuft_height <= 0.0
            || !self.max_anchor_radius.is_finite()
            || self.max_anchor_radius <= 0.0
            || !self.max_blade_radius.is_finite()
            || self.max_blade_radius <= 0.0
        {
            return Err("invalid meadow geometry bounds or vertex budget".into());
        }
        for v in &self.vertices {
            let n = Vec3::from_slice(&v.normal);
            if v.position
                .iter()
                .chain(&v.normal)
                .chain(&v.color)
                .chain(&v.anchor)
                .any(|x| !x.is_finite())
                || !(0.99..=1.01).contains(&n.length_squared())
                || !(0.0..BRANCHES as f32).contains(&v.position[3])
                || v.position[3].fract() != 0.0
                || v.color.iter().any(|v| !(0.0..=1.0).contains(v))
            {
                return Err("invalid meadow vertex, normal, branch or material".into());
            }
            // Frustum and current-frame Hi-Z rejection trust these bounds.
            // Validate caller-supplied recipes too, including below-root
            // vertices and coarser LODs; a positive scalar alone is unsafe.
            // Use the same f32 operations as cook, with no inward tolerance.
            if v.position[0].hypot(v.position[2]) > self.max_tuft_radius
                || v.position[1].abs() > self.max_tuft_height
                || Vec3::from_slice(&v.anchor).length() > self.max_anchor_radius
                || Vec3::from_slice(&v.position).distance(Vec3::from_slice(&v.anchor)) > self.max_blade_radius
            {
                return Err("meadow geometry bounds do not enclose all source vertices".into());
            }
        }
        for variant in 0..VARIANTS {
            for lod in 0..LODS {
                let [first, count] = self.lod_ranges[variant][lod];
                let error = self.lod_errors[variant][lod];
                if count == 0
                    || count % 3 != 0
                    || first as usize + count as usize > self.vertices.len()
                    || count != self.lod_ranges[0][lod][1]
                    || !error.is_finite()
                    || error < 0.0
                    || (lod == 0 && error != 0.0)
                    || (lod > 0
                        && (error < self.lod_errors[variant][lod - 1]
                            || count > self.lod_ranges[variant][lod - 1][1]))
                {
                    return Err("invalid or mismatched meadow LOD ranges/error bounds".into());
                }
            }
            for pivot in &self.branch_pivots[variant] {
                if pivot.iter().any(|v| !v.is_finite()) || !(0.0..=0.7854).contains(&pivot[3]) {
                    return Err("invalid meadow branch pivot".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct MeadowField {
    pub origin: [f32; 2],
    pub cell: f32,
    pub width: u32,
    pub height: u32,
    /// Terrain grid points [height, signed road clearance, valid (0/1), reserved].
    /// Interpolate the native triangles split at local x+z <= 1; never
    /// bilinear interpolation across that diagonal or through invalid holes.
    pub samples: Vec<[f32; 4]>,
    pub seed: u32,
    /// Uniform world-root multiplier; geometry remains cooked at growth 1.
    pub growth: f32,
    /// Provenance only: geometry colours already contain this season.
    pub dry_season: f32,
    pub radius: f32,
    pub fade_start: f32,
    pub geometry: MeadowGeometry,
}

impl MeadowField {
    pub fn validate(&self) -> Result<(), String> {
        let count = self.width as u64 * self.height as u64;
        if self.width < 2
            || self.height < 2
            || count > MAX_FIELD_SAMPLES as u64
            || self.samples.len() as u64 != count
            || self.origin.iter().any(|v| !v.is_finite())
            || !self.cell.is_finite()
            || self.cell <= 0.0
            || !self.growth.is_finite()
            || !(0.1..=1.5).contains(&self.growth)
            || !self.dry_season.is_finite()
            || !(0.0..=1.0).contains(&self.dry_season)
            || !self.radius.is_finite()
            || !(1.0..=256.0).contains(&self.radius)
            || !self.fade_start.is_finite()
            || !(0.0..self.radius).contains(&self.fade_start)
        {
            return Err("invalid meadow terrain field, growth or distance budget".into());
        }
        if self
            .samples
            .iter()
            .any(|s| s.iter().any(|v| !v.is_finite()) || !(s[2] == 0.0 || s[2] == 1.0))
        {
            return Err("invalid meadow terrain samples".into());
        }
        self.geometry.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meadow_blade_budget_cooks_actual_equal_lods_and_keeps_default_bytes() {
        let ordinary = MeadowGeometry::cook(73,0.0).unwrap();
        let explicit = MeadowGeometry::cook_with_blade_budget(73,0.0,MeadowBladeBudget::Blades360).unwrap();
        assert_eq!(ordinary.vertices,explicit.vertices);
        assert_eq!(ordinary.lod_ranges,explicit.lod_ranges);
        assert_eq!(ordinary.lod_errors,explicit.lod_errors);
        for budget in [MeadowBladeBudget::Blades120,MeadowBladeBudget::Blades180] {
            let cooked=MeadowGeometry::cook_with_blade_budget(73,0.0,budget).unwrap();
            cooked.validate().unwrap();
            for variant in 0..VARIANTS {
                assert_eq!(cooked.lod_ranges[variant].map(|range|range[1]/3),
                    [budget.blades() as u32*5,budget.blades() as u32*3,budget.blades() as u32*5/3]);
                for lod in 0..LODS {
                    let [first,count]=cooked.lod_ranges[variant][lod];
                    let vertices=&cooked.vertices[first as usize..][..count as usize];
                    let anchors:std::collections::BTreeSet<_>=vertices.iter()
                        .map(|v|v.anchor.map(f32::to_bits)).collect();
                    assert_eq!(anchors.len(),budget.blades(),"every selected blade survives every source cut");
                }
            }
            assert!(cooked.vertices.len()<ordinary.vertices.len());
        }
    }

    #[test]
    fn meadow_physical_lod_errors_keep_source_and_reduce_anisotropic_overestimate() {
        let packed = MeadowGeometry::cook(0, 0.0).unwrap();
        let mut legacy = [[0.0f32; LODS]; VARIANTS];
        for (variant, seed) in [101, 303, 707].into_iter().enumerate() {
            let source = foliage_assembly::generate(Parameters::meadow_patch(seed), Limits::meadow_patch()).unwrap();
            let poses = source.branch_transforms().unwrap();
            let meshes: Vec<_> = source.parts.iter().map(|p| crate::vegetation_clod::build_ribbon(&Part {
                vertices: &p.vertices, indices: &p.indices, material: 0,
            }).unwrap()).collect();
            for lod in 0..LODS {
                let mut count = 0;
                for placement in &source.instances {
                    let mesh = &meshes[placement.part as usize];
                    let level = &mesh.clusters[lod.min(mesh.clusters.len() - 1)];
                    let transform = poses[placement.branch as usize] * placement.rest;
                    let linear = Mat3::from_mat4(transform);
                    let scale = (linear.x_axis.length_squared() + linear.y_axis.length_squared()
                        + linear.z_axis.length_squared()).sqrt();
                    legacy[variant][lod] = legacy[variant][lod].max(level.own_error * scale);
                    count += level.tri_count * 3;
                }
                assert_eq!(packed.lod_ranges[variant][lod][1], count,
                    "same accepted cuts and every blade remain resident");
                if lod == 0 {
                    assert_eq!(packed.lod_errors[variant][lod], 0.0);
                } else {
                    assert!(packed.lod_errors[variant][lod] > 0.0);
                    assert!(packed.lod_errors[variant][lod] < legacy[variant][lod] * 0.5,
                        "physical width-sensitive bound should remove a material overestimate");
                }
            }
        }
        assert_eq!(packed.lod_ranges[0].map(|r| r[1] / 3), [1800, 1080, 600]);
        eprintln!("r37 meadow LOD errors metres: legacy={legacy:?}; physical={:?}; source triangles per patch={:?}",
            packed.lod_errors, packed.lod_ranges[0].map(|r| r[1] / 3));
    }

    #[test]
    fn packed_source_matches_independent_expansion_and_keeps_every_branch() {
        assert_eq!(std::mem::size_of::<GpuMeadowVertex>(), 64);
        let packed = MeadowGeometry::cook(0, 0.0).unwrap();
        for (variant, seed) in [101, 303, 707].into_iter().enumerate() {
            let source =
                foliage_assembly::generate(Parameters::meadow_patch(seed), Limits::meadow_patch())
                    .unwrap();
            let flat = source.flatten_rest().unwrap();
            let poses = source.branch_transforms().unwrap();
            let expected_anchors: Vec<_> = source.instances.iter().flat_map(|placement| {
                let anchor = (poses[placement.branch as usize] * placement.rest)
                    .transform_point3(Vec3::ZERO);
                std::iter::repeat_n(anchor, source.parts[placement.part as usize].indices.len())
            }).collect();
            let [first, count] = packed.lod_ranges[variant][0];
            assert_eq!(count as usize, BLADES * 5 * 3);
            let expected: Vec<_> = flat
                .iter()
                .flat_map(|m| m.indices.iter().map(|&i| m.vertices[i as usize]))
                .collect();
            for ((actual, expected), anchor) in packed.vertices[first as usize..][..count as usize]
                .iter()
                .zip(expected)
                .zip(expected_anchors.iter().copied())
            {
                assert_eq!(&actual.position[..3], &expected.pos);
                assert_eq!(&actual.normal[..3], &expected.normal);
                assert_eq!(&actual.anchor[..3], &anchor.to_array());
            }
            for lod in 0..LODS {
                let [first, count] = packed.lod_ranges[variant][lod];
                let mut per_branch = [0; BRANCHES];
                let mut anchors = std::collections::BTreeSet::new();
                for triangle in packed.vertices[first as usize..][..count as usize].chunks_exact(3)
                {
                    assert_eq!(triangle[0].position[3], triangle[1].position[3]);
                    assert_eq!(triangle[0].position[3], triangle[2].position[3]);
                    assert_eq!(triangle[0].anchor, triangle[1].anchor);
                    assert_eq!(triangle[0].anchor, triangle[2].anchor);
                    anchors.insert(triangle[0].anchor.map(f32::to_bits));
                    per_branch[triangle[0].position[3] as usize] += 1;
                }
                assert_eq!(anchors.len(), BLADES,
                    "every source cut must retain all rounded-footprint basal anchors");
                let finest_anchors: std::collections::BTreeSet<_> = expected_anchors.iter()
                    .map(|a| [a.x, a.y, a.z, 0.0].map(f32::to_bits)).collect();
                assert_eq!(anchors, finest_anchors,
                    "source LODs and the finest-source aggregate must share exact anchors");
                assert!(
                    per_branch
                        .iter()
                        .all(|&count| count >= BLADES / BRANCHES)
                );
            }
            assert!(packed.lod_ranges[variant][2][1] < count);
        }
    }
    #[test]
    fn meadow_fixed_anchor_bounds_cover_small_growth_and_wind_on_hills() {
        let packed = MeadowGeometry::cook(17, 0.0).unwrap();
        let slope = glam::Vec2::new(0.8, 0.6); // supported steep hill
        let shear_bound = (1.0 + 0.25 * slope.length_squared()).sqrt() + 0.5 * slope.length();
        let max_angle = packed.branch_pivots.iter().flatten().map(|p| p[3]).fold(0.0f32, f32::max) * 1.5;
        for growth in [0.1, 0.5, 1.0, 1.5] {
            let bound = (packed.max_anchor_radius + packed.max_blade_radius * growth * (1.0 + max_angle)) * shear_bound;
            for angle in [-max_angle, 0.0, max_angle] {
                let rotation = glam::Quat::from_axis_angle(Vec3::new(0.6, 0.0, -0.8), angle);
                for v in &packed.vertices {
                    let anchor = Vec3::from_slice(&v.anchor);
                    let relative = (Vec3::from_slice(&v.position) - anchor) * growth;
                    let offset = anchor + rotation * relative;
                    let actual = offset + Vec3::Y * slope.dot(glam::Vec2::new(offset.x, offset.z));
                    assert!(actual.length() <= bound + 1e-6, "wind/terrain envelope must retain the fixed basal lattice even at small growth");
                }
            }
        }
    }
    #[test]
    fn seed_is_deterministic_and_season_does_not_move_geometry() {
        let green = MeadowGeometry::cook(19, 0.0).unwrap();
        let repeated = MeadowGeometry::cook(19, 0.0).unwrap();
        let dry = MeadowGeometry::cook(19, 1.0).unwrap();
        assert_eq!(green.vertices, repeated.vertices);
        assert_eq!(green.lod_ranges, dry.lod_ranges);
        for (a, b) in green.vertices.iter().zip(&dry.vertices) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
            assert_ne!(a.color, b.color);
        }
        assert!(MeadowGeometry::cook(1, f32::NAN).is_err());
    }

    #[test]
    fn field_rejects_understated_geometry_bounds_without_inward_tolerance() {
        let geometry = MeadowGeometry::cook(23, 0.0).unwrap();
        let actual_radius = geometry.vertices.iter()
            .map(|v| v.position[0].hypot(v.position[2])).fold(0.0f32, f32::max);
        let actual_height = geometry.vertices.iter()
            .map(|v| v.position[1].abs()).fold(0.0f32, f32::max);
        assert_eq!(geometry.max_tuft_radius, actual_radius);
        assert_eq!(geometry.max_tuft_height, actual_height);
        let field = MeadowField {
            origin: [0.0; 2], cell: 2.0, width: 2, height: 2,
            samples: vec![[0.0, 10.0, 1.0, 0.0]; 4], seed: 23,
            growth: 1.0, dry_season: 0.0, radius: 100.0, fade_start: 80.0,
            geometry,
        };
        field.validate().expect("cooked bounds remain valid");

        let mut radial = field.clone();
        radial.geometry.max_tuft_radius = f32::from_bits(actual_radius.to_bits() - 1);
        assert!(radial.validate().unwrap_err().contains("bounds do not enclose"));

        let mut vertical = field.clone();
        vertical.geometry.max_tuft_height = f32::from_bits(actual_height.to_bits() - 1);
        assert!(vertical.validate().unwrap_err().contains("bounds do not enclose"));

        // A valid-looking custom recipe can extend below its root; testing
        // only max(+Y) would miss this and permit false occlusion from below.
        let mut below_root = field;
        below_root.geometry.vertices[0].position[1] = -2.0 * actual_height;
        assert!(below_root.validate().unwrap_err().contains("bounds do not enclose"));
    }
}
