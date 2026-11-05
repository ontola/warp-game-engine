//! Source-derived sparse voxel assets for the GPU meadow's actual recipes.
//!
//! These are the same `VoxelAsset` records consumed by the foliage volume
//! renderer, not a terrain tint or an opaque box proxy. Every finest source
//! triangle contributes one sheet crossing; two-sided lighting does not
//! duplicate geometric mass. All mip levels retain surface/projected area.
//!
//! This is immutable asset cooking, not a whole-field renderer. A whole-patch
//! transform cannot reproduce four independent branch poses or growth about
//! each fixed blade anchor. A field/page producer must compose those poses
//! before claiming source/volume animation equivalence. Likewise, additive
//! extinction is not exact union opacity for correlated opaque grass blades.

use crate::{
    cluster::{Part, Vertex},
    foliage_voxel::{self, BakeConfig, ProjectionMeasure, VoxelAsset},
    meadow_data::{GpuMeadowVertex, MeadowGeometry, VARIANTS},
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::Vec3;
use web_time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct MeadowVoxelConfig {
    /// Uniform positive blade growth about each original basal anchor. Patch
    /// anchors do not move. Spatial growth and wind are runtime concerns.
    pub growth: f32,
    /// Existing foliage-baker limits apply unchanged. Open meadow ribbons
    /// require SheetCrossings; ClosedSurfaceEntries is explicitly rejected.
    pub bake: BakeConfig,
}

impl Default for MeadowVoxelConfig {
    fn default() -> Self {
        Self {
            growth: 1.0,
            bake: BakeConfig {
                projection_measure: ProjectionMeasure::SheetCrossings,
                cell_size: 0.04,
                levels: 6,
                samples_per_cell: 2,
                max_samples: 2_000_000,
                max_voxels: 65_536,
                max_axis_cells: 256,
                ..BakeConfig::default()
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct MeadowVoxelAsset {
    /// One asset per original recipe variant, directly packable through
    /// foliage_volume_data::pack. No per-root copies or new GPU format.
    pub assets: [VoxelAsset; VARIANTS],
    /// Actual cook configuration, including geometric scale and limits.
    pub config: MeadowVoxelConfig,
    /// CPU construction only, never a GPU performance measurement.
    pub cook_ms: f64,
}

impl MeadowVoxelAsset {
    pub fn cook(geometry: &MeadowGeometry, config: MeadowVoxelConfig) -> Result<Self, String> {
        let started = Instant::now();
        geometry.validate()?;
        if !config.growth.is_finite() || config.growth <= 0.0 {
            return Err("meadow voxel growth must be finite and positive".into());
        }
        if config.bake.projection_measure != ProjectionMeasure::SheetCrossings {
            return Err("open meadow blades require SheetCrossings, not closed-surface entry halving".into());
        }
        let mut assets = Vec::with_capacity(VARIANTS);
        for variant in 0..VARIANTS {
            let prepared = prepare(geometry, variant, config.growth)?;
            let indices = [0, 1, 2];
            let parts: Vec<_> = prepared.triangles.iter().enumerate().map(|(material, triangle)| Part {
                vertices: triangle,
                indices: &indices,
                material: material as u32,
            }).collect();
            let asset = foliage_voxel::bake(&parts, &prepared.materials, &[], config.bake)?;
            if asset.levels.is_empty() || asset.levels[0].voxels.is_empty() {
                return Err(format!("meadow variant {variant} has no nondegenerate voxel source"));
            }
            assets.push(asset);
        }
        Ok(Self {
            assets: assets.try_into().map_err(|_| "meadow voxel variant count changed")?,
            config,
            cook_ms: started.elapsed().as_secs_f64() * 1_000.0,
        })
    }

    /// Finest source triangle counts, excluding all alternative source LODs.
    pub fn source_triangles(&self) -> [u64; VARIANTS] {
        std::array::from_fn(|i| self.assets[i].stats.triangles)
    }

    /// Resident sparse cells at each mip. A missing tail level contributes
    /// zero; it does not mean that the last available level lost its mass.
    pub fn occupied_cells_per_mip(&self) -> Vec<[usize; VARIANTS]> {
        let count = self.assets.iter().map(|asset| asset.levels.len()).max().unwrap_or(0);
        (0..count).map(|level| std::array::from_fn(|variant| {
            self.assets[variant].levels.get(level).map_or(0, |level| level.voxels.len())
        })).collect()
    }
}

struct Prepared {
    triangles: Vec<[Vertex; 3]>,
    materials: Vec<Material>,
}

fn source_vertices(geometry: &MeadowGeometry, variant: usize) -> &[GpuMeadowVertex] {
    let [first, count] = geometry.lod_ranges[variant][0];
    &geometry.vertices[first as usize..][..count as usize]
}

fn prepare(geometry: &MeadowGeometry, variant: usize, growth: f32) -> Result<Prepared, String> {
    let source = source_vertices(geometry, variant);
    let mut result = Prepared {
        triangles: Vec::with_capacity(source.len() / 3),
        materials: Vec::with_capacity(source.len() / 3),
    };
    for triangle in source.chunks_exact(3) {
        if triangle[1..].iter().any(|vertex| vertex.position[3] != triangle[0].position[3]
            || vertex.anchor != triangle[0].anchor)
        {
            return Err("meadow voxel triangle needs one branch and one basal anchor".into());
        }
        let mut tint = Vec3::ZERO;
        let mut roughness = 0.0;
        let vertices = std::array::from_fn(|i| {
            let vertex = &triangle[i];
            let point = Vec3::from_slice(&vertex.position);
            let anchor = Vec3::from_slice(&vertex.anchor);
            // Identical anchored growth as meadow_vs, before yaw/wind/shear.
            let position = anchor + (point - anchor) * growth;
            // meadow_vs evaluates this pigment at source rest height, before
            // growth or wind. Integrate its linearly interpolated vertex RGB
            // exactly over each triangle. A voxel containing only part of a
            // triangle therefore uses its triangle mean, a bounded material
            // approximation rather than a newly invented distant palette.
            let height = (point.y / geometry.max_tuft_height.max(0.001)).clamp(0.0, 1.0);
            let pigment = Vec3::new(0.91, 0.94, 0.86).lerp(Vec3::new(1.10, 1.04, 0.98), height);
            tint += Vec3::from_slice(&vertex.color) * pigment / 3.0;
            roughness += vertex.color[3] / 3.0;
            Vertex {
                pos: position.to_array(),
                normal: Vec3::from_slice(&vertex.normal).to_array(),
                uv: [0.0; 2],
            }
        });
        if vertices.iter().any(|vertex| vertex.pos.iter().any(|value| !value.is_finite())) {
            return Err("meadow anchored growth produced non-finite geometry".into());
        }
        result.triangles.push(vertices);
        result.materials.push(Material {
            tint: tint.to_array(),
            roughness,
            layer: -1,
            flags: THIN_FOLIAGE | TWO_SIDED,
            scale: 1.0,
            metal: 0.0,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        foliage_volume_data::{self, PackLimits},
        foliage_voxel::{PROJECTION_DIRECTIONS, VoxelLevel},
        meadow_data::{BRANCHES, LODS},
    };
    use glam::DVec3;

    fn simple_geometry() -> MeadowGeometry {
        let anchor = Vec3::new(0.27, 0.0, -0.16);
        let points = [[0.0, 0.0, -0.015], [0.0, 0.0, 0.015], [0.0, 0.3, 0.015],
            [0.0, 0.0, -0.015], [0.0, 0.3, 0.015], [0.0, 0.3, -0.015]];
        let vertices = points.into_iter().map(|p| {
            let p = Vec3::from(p) + anchor;
            GpuMeadowVertex {
                position: p.extend(0.0).to_array(), normal: [1.0, 0.0, 0.0, 0.0],
                color: [0.15, 0.30, 0.08, 0.8], anchor: anchor.extend(0.0).to_array(),
            }
        }).collect();
        MeadowGeometry {
            vertices, variant_patch_metres: 0.0, lod_ranges: [[[0, 6]; LODS]; VARIANTS], lod_errors: [[0.0; LODS]; VARIANTS],
            branch_pivots: [[[0.0; 4]; BRANCHES]; VARIANTS], max_tuft_radius: 1.0,
            max_tuft_height: 1.0, max_anchor_radius: 1.0, max_blade_radius: 1.0,
        }
    }

    fn area(level: &VoxelLevel) -> f64 {
        level.voxels.iter().map(|voxel| f64::from(voxel.surface_area)).sum()
    }

    fn projected(level: &VoxelLevel, direction: usize) -> f64 {
        level.voxels.iter().map(|voxel| f64::from(voxel.directional_area[direction])).sum()
    }

    fn moments(level: &VoxelLevel) -> [f64; 28] {
        let mut sums = [0.0; 28];
        for voxel in &level.voxels {
            let area = f64::from(voxel.surface_area);
            for (axis, color) in voxel.albedo.iter().enumerate() { sums[axis] += area * f64::from(*color); }
            sums[3] += area * f64::from(voxel.roughness);
            for lobe in 0..6 {
                let weight = area * f64::from(voxel.normal_weights[lobe]);
                sums[4 + lobe * 4] += weight;
                for axis in 0..3 { sums[5 + lobe * 4 + axis] += weight * f64::from(voxel.normal_centroids[lobe][axis]); }
            }
        }
        sums
    }

    #[test]
    fn sheet_mass_and_two_sided_normals_survive_all_mips_without_halving() {
        let cooked = MeadowVoxelAsset::cook(&simple_geometry(), MeadowVoxelConfig::default()).unwrap();
        assert_eq!(cooked.source_triangles(), [2; VARIANTS]);
        for asset in &cooked.assets {
            assert_eq!(asset.projection_measure, ProjectionMeasure::SheetCrossings);
            assert_eq!(asset.stats.closed_parts_checked, 0);
            let first = &asset.levels[0];
            assert!((area(first) - 0.009).abs() < 1e-8);
            let expected_moments = moments(first);
            for level in &asset.levels {
                assert!((area(level) - 0.009).abs() < 1e-8);
                for (direction, vector) in PROJECTION_DIRECTIONS.iter().enumerate() {
                    assert!((projected(level, direction) - 0.009 * f64::from(vector[0].abs())).abs() < 1e-8);
                }
                for (a, b) in moments(level).into_iter().zip(expected_moments) { assert!((a - b).abs() < 1e-8); }
                for voxel in &level.voxels {
                    assert_eq!(voxel.leaf_fraction, 1.0);
                    assert_eq!(voxel.normal_weights, [0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
                    assert_eq!(voxel.normal_centroids[0], [1.0, 0.0, 0.0]);
                    assert_eq!(voxel.normal_centroids[1], [-1.0, 0.0, 0.0]);
                }
            }
        }
    }

    #[test]
    fn anchored_growth_changes_blade_extent_and_area_not_fixed_basal_midpoint_or_palette() {
        let geometry = simple_geometry();
        let before = geometry.vertices.clone();
        let rest = prepare(&geometry, 0, 1.0).unwrap();
        for growth in [0.5, 1.5] {
            let grown = prepare(&geometry, 0, growth).unwrap();
            for (triangle, source) in grown.triangles.iter().zip(source_vertices(&geometry, 0).chunks_exact(3)) {
                for (vertex, source) in triangle.iter().zip(source) {
                    let anchor = Vec3::from_slice(&source.anchor);
                    let expected = anchor + (Vec3::from_slice(&source.position) - anchor) * growth;
                    assert_eq!(vertex.pos, expected.to_array());
                    assert_eq!(vertex.normal, Vec3::from_slice(&source.normal).to_array());
                }
            }
            let basal_midpoint = (Vec3::from(grown.triangles[0][0].pos) + Vec3::from(grown.triangles[0][1].pos)) * 0.5;
            assert!(basal_midpoint.distance(Vec3::from_slice(&geometry.vertices[0].anchor)) < 1e-7);
            for (a, b) in grown.materials.iter().zip(&rest.materials) {
                assert_eq!(a.tint, b.tint); assert_eq!(a.roughness, b.roughness);
            }
            let cooked = MeadowVoxelAsset::cook(&geometry, MeadowVoxelConfig { growth, ..Default::default() }).unwrap();
            for level in &cooked.assets[0].levels {
                assert!((area(level) - 0.009 * f64::from(growth).powi(2)).abs() < 2e-8);
            }
        }
        assert_eq!(geometry.vertices, before, "cooking cannot mutate the live source");
    }

    #[test]
    fn finest_only_material_gradient_and_shared_packing_are_deterministic() {
        let mut geometry = simple_geometry();
        let first = MeadowVoxelAsset::cook(&geometry, MeadowVoxelConfig::default()).unwrap();
        let mut coarse = geometry.vertices.clone();
        for vertex in &mut coarse { vertex.color = [0.9, 0.0, 0.0, 0.2]; }
        geometry.vertices.extend(coarse);
        for variant in 0..VARIANTS { for lod in 1..LODS { geometry.lod_ranges[variant][lod] = [6, 6]; } }
        let second = MeadowVoxelAsset::cook(&geometry, MeadowVoxelConfig::default()).unwrap();
        assert_eq!(first.assets, second.assets, "alternative CLOD cannot alter the source voxel asset");
        let level = &first.assets[0].levels[0];
        let values = moments(level);
        let mean_height = 0.15f64;
        let expected = [0.15 * (0.91 + 0.19 * mean_height), 0.30 * (0.94 + 0.10 * mean_height),
            0.08 * (0.86 + 0.12 * mean_height)];
        for axis in 0..3 { assert!((values[axis] / area(level) - expected[axis]).abs() < 1e-7); }
        let forest = foliage_volume_data::pack(&first.assets, &[], &[], PackLimits {
            levels_per_asset: 6, ..Default::default()
        }).unwrap();
        assert_eq!(forest.levels.len(), first.assets.iter().map(|asset| asset.levels.len()).sum::<usize>());
        assert_eq!(forest.cells.len(), first.occupied_cells_per_mip().iter().flatten().sum::<usize>());
        assert!(forest.instances.is_empty());
        assert!(forest.byte_len() > 0);
    }

    #[test]
    fn actual_live_recipe_mass_directions_and_material_moments_match_every_mip() {
        let geometry = MeadowGeometry::cook(0x5eed, 0.2).unwrap();
        let cooked = MeadowVoxelAsset::cook(&geometry, MeadowVoxelConfig::default()).unwrap();
        assert_eq!(cooked.source_triangles(), [1_800; VARIANTS]);
        for variant in 0..VARIANTS {
            let mut expected_area = 0.0;
            let mut expected_projection = [0.0; 13];
            for triangle in source_vertices(&geometry, variant).chunks_exact(3) {
                // Independent f64 source geometry oracle, not the voxel reducer.
                let p: [DVec3; 3] = std::array::from_fn(|i| DVec3::new(triangle[i].position[0] as f64,
                    triangle[i].position[1] as f64, triangle[i].position[2] as f64));
                let cross = (p[1] - p[0]).cross(p[2] - p[0]);
                expected_area += cross.length() * 0.5;
                for (index, d) in PROJECTION_DIRECTIONS.iter().enumerate() {
                    expected_projection[index] += cross.dot(DVec3::from_array(d.map(f64::from))).abs() * 0.5;
                }
            }
            let asset = &cooked.assets[variant];
            assert_eq!(asset.stats.degenerate_triangles, 0);
            let expected_moments = moments(&asset.levels[0]);
            for level in &asset.levels {
                assert!((area(level) - expected_area).abs() <= expected_area * 1e-5);
                for (direction, expected) in expected_projection.into_iter().enumerate() {
                    assert!((projected(level, direction) - expected).abs() <= expected_area * 1e-5);
                }
                for (a, b) in moments(level).into_iter().zip(expected_moments) {
                    assert!((a - b).abs() <= expected_area * 2e-6);
                }
                for voxel in &level.voxels {
                    for pair in 0..3 {
                        assert_eq!(voxel.normal_weights[pair * 2], voxel.normal_weights[pair * 2 + 1]);
                        for axis in 0..3 {
                            assert_eq!(voxel.normal_centroids[pair * 2][axis], -voxel.normal_centroids[pair * 2 + 1][axis]);
                        }
                    }
                }
            }
        }
        let packed = foliage_volume_data::pack(&cooked.assets, &[], &[], PackLimits {
            levels_per_asset: 6, ..Default::default()
        }).unwrap();
        eprintln!("meadow voxel source={:?}, cells/mip={:?}, packed={} bytes, CPU cook={:.3}ms (not GPU)",
            cooked.source_triangles(), cooked.occupied_cells_per_mip(), packed.byte_len(), cooked.cook_ms);
    }

    #[test]
    fn malformed_growth_sheet_semantics_branch_and_budget_fail_explicitly() {
        let geometry = simple_geometry();
        for growth in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(MeadowVoxelAsset::cook(&geometry, MeadowVoxelConfig { growth, ..Default::default() }).is_err());
        }
        let mut config = MeadowVoxelConfig::default();
        config.bake.projection_measure = ProjectionMeasure::ClosedSurfaceEntries;
        assert!(MeadowVoxelAsset::cook(&geometry, config).unwrap_err().contains("SheetCrossings"));
        config = MeadowVoxelConfig::default(); config.bake.max_samples = 1;
        assert!(MeadowVoxelAsset::cook(&geometry, config).is_err());
        let mut malformed = geometry;
        malformed.vertices[1].position[3] = 1.0;
        assert!(MeadowVoxelAsset::cook(&malformed, MeadowVoxelConfig::default()).unwrap_err().contains("one branch"));
    }
}
