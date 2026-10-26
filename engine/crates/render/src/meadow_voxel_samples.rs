//! Bounded quadrature of the actual finest meadow source, for world-brick
//! production. Samples retain fixed blade anchors and wind-group identity.
//! Area is conserved up to f32 packing; point splats are not an exact union
//! coverage or a substitute for triangle/voxel intersection. Capped triangles
//! and the largest unsolved sample radius are explicit diagnostics.

use crate::meadow_data::{GpuMeadowVertex, MeadowGeometry, VARIANTS};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};

pub const MAX_SAMPLES_PER_VARIANT: u32 = 32_768;

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuMeadowVoxelSample {
    /// Assembly-rest position and its represented one-sided geometric area.
    pub position_area: [f32; 4],
    /// Fixed basal anchor and exact zero-based wind-group index.
    pub anchor_branch: [f32; 4],
    /// Unit geometric face normal, separately from shading normal; roughness.
    pub geometric_normal_roughness: [f32; 4],
    /// Interpolated source normal, before deformation/normalization; W zero.
    pub shading_normal: [f32; 4],
    /// Interpolated source linear RGB including the existing height pigment.
    /// World-root pigment and live colour edits are applied by the producer.
    pub albedo: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct SampleConfig {
    /// Requested maximum edge length of each sample's support triangle.
    pub spacing: f32,
    /// Hard cap; unresolved support is reported, never silently discarded.
    pub max_per_triangle: u32,
    pub max_per_variant: u32,
}
impl Default for SampleConfig {
    fn default() -> Self {
        Self {
            spacing: 0.08,
            max_per_triangle: 8,
            max_per_variant: MAX_SAMPLES_PER_VARIANT,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SampleStats {
    pub source_triangles: [u32; VARIANTS],
    pub source_area: [f64; VARIANTS],
    pub packed_area: [f64; VARIANTS],
    pub capped_triangles: [u32; VARIANTS],
    pub max_sample_radius: [f32; VARIANTS],
}

#[derive(Clone, Debug)]
pub struct MeadowVoxelSamples {
    pub samples: Vec<GpuMeadowVoxelSample>,
    /// [first sample, sample count, finest source triangles, reserved].
    pub ranges: [[u32; 4]; VARIANTS],
    pub config: SampleConfig,
    pub stats: SampleStats,
}

#[derive(Clone, Copy)]
struct Cell {
    bary: [DVec3; 3],
    fraction: f64,
}
fn interpolate(points: [DVec3; 3], bary: DVec3) -> DVec3 {
    points[0] * bary.x + points[1] * bary.y + points[2] * bary.z
}
fn longest(cell: Cell, points: [DVec3; 3]) -> (usize, f64) {
    let corners = cell.bary.map(|b| interpolate(points, b));
    (0..3)
        .map(|i| (i, corners[i].distance_squared(corners[(i + 1) % 3])))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
}
fn partition(points: [DVec3; 3], config: SampleConfig) -> (Vec<Cell>, bool) {
    let mut cells = vec![Cell {
        bary: [DVec3::X, DVec3::Y, DVec3::Z],
        fraction: 1.0,
    }];
    let limit = f64::from(config.spacing).powi(2);
    loop {
        let (index, edge, length2) = cells
            .iter()
            .enumerate()
            .map(|(index, &cell)| {
                let (edge, length2) = longest(cell, points);
                (index, edge, length2)
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap();
        if length2 <= limit {
            return (cells, false);
        }
        if cells.len() >= config.max_per_triangle as usize {
            return (cells, true);
        }
        let cell = cells[index];
        let a = cell.bary[edge];
        let b = cell.bary[(edge + 1) % 3];
        let c = cell.bary[(edge + 2) % 3];
        let mid = (a + b) * 0.5;
        cells[index] = Cell {
            bary: [a, mid, c],
            fraction: cell.fraction * 0.5,
        };
        cells.push(Cell {
            bary: [mid, b, c],
            fraction: cell.fraction * 0.5,
        });
    }
}
fn vertex_albedo(vertex: GpuMeadowVertex, height: f32) -> DVec3 {
    let t = (vertex.position[1] / height.max(0.001)).clamp(0.0, 1.0);
    let tint = Vec3::new(0.91, 0.94, 0.86).lerp(Vec3::new(1.10, 1.04, 0.98), t);
    (Vec3::from_slice(&vertex.color) * tint).as_dvec3()
}

impl MeadowVoxelSamples {
    pub fn cook(geometry: &MeadowGeometry, config: SampleConfig) -> Result<Self, String> {
        geometry.validate()?;
        if !config.spacing.is_finite()
            || config.spacing <= 0.0
            || !(1..=64).contains(&config.max_per_triangle)
            || !(1..=MAX_SAMPLES_PER_VARIANT).contains(&config.max_per_variant)
        {
            return Err("invalid meadow source sample budget".into());
        }
        let mut result = Self {
            samples: Vec::new(),
            ranges: [[0; 4]; VARIANTS],
            config,
            stats: SampleStats::default(),
        };
        for variant in 0..VARIANTS {
            let [first, count] = geometry.lod_ranges[variant][0];
            if count / 3 > config.max_per_variant {
                return Err("meadow sample budget cannot retain every finest triangle".into());
            }
            let start = result.samples.len();
            for triangle in geometry.vertices[first as usize..][..count as usize].chunks_exact(3) {
                let vertices: [GpuMeadowVertex; 3] = triangle.try_into().unwrap();
                let branch = vertices[0].position[3];
                let anchor = Vec3::from_slice(&vertices[0].anchor);
                if branch.fract() != 0.0
                    || !(0.0..4.0).contains(&branch)
                    || vertices
                        .iter()
                        .any(|v| v.position[3] != branch || Vec3::from_slice(&v.anchor) != anchor)
                {
                    return Err(
                        "meadow sample triangle must retain one basal anchor and wind group".into(),
                    );
                }
                let points = vertices.map(|v| Vec3::from_slice(&v.position).as_dvec3());
                let cross = (points[1] - points[0]).cross(points[2] - points[0]);
                let area = cross.length() * 0.5;
                let packed_area = area as f32;
                if !area.is_finite() || !packed_area.is_finite() || packed_area <= 0.0 {
                    return Err("degenerate or unrepresentable meadow source triangle".into());
                }
                let geometric = cross.normalize().as_vec3();
                let (cells, capped) = partition(points, config);
                if result.samples.len() - start + cells.len() > config.max_per_variant as usize {
                    return Err(
                        "meadow source sample count exceeds the explicit variant budget".into(),
                    );
                }
                let colors = vertices.map(|v| vertex_albedo(v, geometry.max_tuft_height));
                let normals = vertices.map(|v| Vec3::from_slice(&v.normal).as_dvec3());
                for cell in cells {
                    let bary = (cell.bary[0] + cell.bary[1] + cell.bary[2]) / 3.0;
                    let position = interpolate(points, bary).as_vec3();
                    let normal = interpolate(normals, bary).as_vec3();
                    let albedo = interpolate(colors, bary).as_vec3();
                    let roughness = (0..3)
                        .map(|i| f64::from(vertices[i].color[3]) * bary[i])
                        .sum::<f64>() as f32;
                    // Bisection gives dyadic area fractions. Multiplying the
                    // rounded source area by powers of two conserves that
                    // packed triangle area without a zero/negative last bin.
                    let weight = (f64::from(packed_area) * cell.fraction) as f32;
                    if !position.is_finite()
                        || !normal.is_finite()
                        || !albedo.is_finite()
                        || !roughness.is_finite()
                        || weight <= 0.0
                    {
                        return Err("non-finite meadow source sample".into());
                    }
                    let radius = cell
                        .bary
                        .iter()
                        .map(|&b| interpolate(points, b).distance(position.as_dvec3()))
                        .fold(0.0f64, f64::max);
                    let rounded_radius = radius as f32;
                    let radius = if f64::from(rounded_radius) < radius {
                        f32::from_bits(rounded_radius.to_bits() + 1)
                    } else {
                        rounded_radius
                    };
                    result.stats.max_sample_radius[variant] =
                        result.stats.max_sample_radius[variant].max(radius);
                    result.stats.packed_area[variant] += f64::from(weight);
                    result.samples.push(GpuMeadowVoxelSample {
                        position_area: position.extend(weight).to_array(),
                        anchor_branch: anchor.extend(branch).to_array(),
                        geometric_normal_roughness: geometric.extend(roughness).to_array(),
                        shading_normal: normal.extend(0.0).to_array(),
                        albedo: albedo.extend(0.0).to_array(),
                    });
                }
                result.stats.source_area[variant] += area;
                result.stats.source_triangles[variant] += 1;
                result.stats.capped_triangles[variant] += u32::from(capped);
            }
            result.ranges[variant] = [
                start as u32,
                (result.samples.len() - start) as u32,
                result.stats.source_triangles[variant],
                0,
            ];
        }
        Ok(result)
    }
    pub fn bytes(&self) -> u64 {
        self.samples.len() as u64 * std::mem::size_of::<GpuMeadowVoxelSample>() as u64
            + std::mem::size_of_val(&self.ranges) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_assembly::MeadowBladeBudget;

    #[test]
    fn source_sample_layout_and_recipe_mass_material_identity() {
        assert_eq!(std::mem::size_of::<GpuMeadowVoxelSample>(), 80);
        assert_eq!(std::mem::align_of::<GpuMeadowVoxelSample>(), 16);
        let geometry =
            MeadowGeometry::cook_with_blade_budget(73, 0.2, MeadowBladeBudget::Blades120).unwrap();
        let a = MeadowVoxelSamples::cook(&geometry, SampleConfig::default()).unwrap();
        let b = MeadowVoxelSamples::cook(&geometry, SampleConfig::default()).unwrap();
        assert_eq!(a.samples, b.samples);
        assert_eq!(a.ranges, b.ranges);
        for variant in 0..VARIANTS {
            let [first, count, triangles, _] = a.ranges[variant];
            assert_eq!(triangles, 600);
            assert!(count >= triangles && count <= triangles * a.config.max_per_triangle);
            assert!(
                (a.stats.packed_area[variant] - a.stats.source_area[variant]).abs()
                    <= a.stats.source_area[variant] * f64::from(f32::EPSILON)
            );
            let samples = &a.samples[first as usize..][..count as usize];
            for group in 0..4 {
                assert!(samples.iter().any(|s| s.anchor_branch[3] == group as f32));
            }
            for sample in samples {
                assert!(sample.position_area[3] > 0.0);
                assert!(
                    (Vec3::from_slice(&sample.geometric_normal_roughness).length() - 1.0).abs()
                        < 2e-6
                );
                assert!(
                    sample.albedo[..3]
                        .iter()
                        .all(|v| v.is_finite() && *v >= 0.0)
                );
            }
        }
        assert!(a.bytes() <= 3 * u64::from(MAX_SAMPLES_PER_VARIANT) * 80 + 48);
    }

    #[test]
    fn narrow_long_triangle_has_linear_bounded_work_and_conserves_first_moments() {
        let points = [
            DVec3::ZERO,
            DVec3::new(0.001, 0.0, 0.0),
            DVec3::new(0.0, 1000.0, 0.0),
        ];
        let (cells, capped) = partition(points, SampleConfig::default());
        assert!(capped);
        assert_eq!(cells.len(), 8);
        assert_eq!(cells.iter().map(|c| c.fraction).sum::<f64>(), 1.0);
        let mean = cells
            .iter()
            .map(|c| interpolate(points, (c.bary[0] + c.bary[1] + c.bary[2]) / 3.0) * c.fraction)
            .sum::<DVec3>();
        assert!(mean.distance((points[0] + points[1] + points[2]) / 3.0) < 1e-12);
        // The support radius encloses the complete subtriangle, not just a
        // guessed point-size based on its very small area.
        for cell in cells {
            let corners = cell.bary.map(|b| interpolate(points, b));
            let centre = (corners[0] + corners[1] + corners[2]) / 3.0;
            let radius = corners
                .iter()
                .map(|p| p.distance(centre))
                .fold(0.0f64, f64::max);
            for i in 0..=8 {
                for j in 0..=8 - i {
                    let p = corners[0]
                        + (corners[1] - corners[0]) * (i as f64 / 8.0)
                        + (corners[2] - corners[0]) * (j as f64 / 8.0);
                    assert!(p.distance(centre) <= radius + 1e-12);
                }
            }
        }
    }

    #[test]
    fn insufficient_budget_fails_without_omitting_source_triangles() {
        let geometry =
            MeadowGeometry::cook_with_blade_budget(7, 0.0, MeadowBladeBudget::Blades120).unwrap();
        assert!(
            MeadowVoxelSamples::cook(
                &geometry,
                SampleConfig {
                    max_per_variant: 599,
                    ..Default::default()
                }
            )
            .is_err()
        );
        for spacing in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                MeadowVoxelSamples::cook(
                    &geometry,
                    SampleConfig {
                        spacing,
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn shared_deformation_keeps_anchors_and_true_surface_jacobian() {
        use glam::{DMat3, DQuat, DVec2};
        let root = DVec3::new(103.0, 7.0, -217.0);
        let anchor = DVec3::new(0.23, 0.0, -0.14);
        let source = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.02, 0.1, 0.0),
            DVec3::new(0.0, 0.3, 0.012),
        ];
        let n = (source[1] - source[0])
            .cross(source[2] - source[0])
            .normalize();
        let original_area = (source[1] - source[0])
            .cross(source[2] - source[0])
            .length()
            * 0.5;
        for slope in [DVec2::ZERO, DVec2::new(0.7, -0.35), DVec2::new(-0.9, 0.2)] {
            let shear = DMat3::from_cols(
                DVec3::new(1.0, slope.x, 0.0),
                DVec3::Y,
                DVec3::new(0.0, slope.y, 1.0),
            );
            for growth in [0.5f64, 1.0, 2.0] {
                for yaw in [-2.2f64, 0.0, 1.7] {
                    for angle in [-0.22f64, 0.0, 0.22] {
                        let y = DQuat::from_rotation_y(yaw);
                        let bend =
                            DQuat::from_axis_angle(DVec3::new(-0.3, 0.0, 0.8).normalize(), angle);
                        let deform = |p: DVec3| {
                            root + shear * (y * anchor + bend * (y * ((p - anchor) * growth)))
                        };
                        assert!(deform(anchor).distance(root + shear * (y * anchor)) < 1e-12);
                        let world = source.map(deform);
                        let cross = (world[1] - world[0]).cross(world[2] - world[0]);
                        let transformed_n = shear.inverse().transpose() * (bend * (y * n));
                        let jacobian = growth * growth * transformed_n.length();
                        assert!((cross.length() * 0.5 / original_area - jacobian).abs() < 1e-10);
                        assert!(cross.normalize().dot(transformed_n.normalize()) > 1.0 - 1e-12);
                        // Current and previous poses each preserve the anchor; their
                        // difference is computed at the SAME authored surface point.
                        let old = DQuat::from_axis_angle(
                            DVec3::new(-0.3, 0.0, 0.8).normalize(),
                            angle - 0.07,
                        );
                        let previous = root
                            + shear * (y * anchor + old * (y * ((source[2] - anchor) * growth)));
                        let delta = previous - world[2];
                        assert!((world[2] + delta).distance(previous) < 1e-12);
                    }
                }
            }
        }
    }

    #[test]
    fn source_renderer_and_brick_producer_share_deformation_without_resource_abi_changes() {
        let common = include_str!("shaders/meadow_common.wgsl");
        let render = include_str!("shaders/meadow_render.wgsl");
        assert!(render.contains("let deformed=meadow_deform_source(point.xyz,normal.xyz,normal.xyz,anchor.xyz,root,yaw,sc,"));
        assert!(!render.contains("let relative=meadow_yaw((point.xyz-anchor.xyz)*root.w,yaw.xy)"));
        assert!(common.contains("if !history_valid {out.previous_world=out.world;}"));
        assert!(common.contains("out.area_scale=root_scale.w*root_scale.w*length(cofactor)"));
        assert!(
            render.find("meadow_road_anchor_allowed").unwrap()
                < render.find("meadow_deform_source").unwrap()
        );
        let source = format!("{}\n{}", include_str!("shaders/meadow_field.wgsl"), common);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let function = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some("meadow_deform_source"))
            .unwrap()
            .1;
        assert_eq!(function.arguments.len(), 9);
        assert!(
            module.global_variables.is_empty(),
            "shared deformation must not acquire a renderer binding"
        );
    }
}
