//! Coarse material continuity beneath/farther than the directional meadow.
//!
//! This is an albedo approximation, not additional geometry or irradiance.
//! Fine source material and rest top-view geometric coverage are reduced once
//! per field. Coverage does not model runtime growth, wind or projected view.
//! Terrain still supplies its actual normal, roughness and lighting;
//! the unoccupied fraction retains its original moss/needle/soil material.

use crate::meadow_data::{MeadowField, MeadowGeometry, VARIANTS};
use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};

/// Explicit on/off permits a substrate-only material diagnostic. With no
/// recognized override the existing aggregate-dependent default is retained.
pub(crate) fn enabled(aggregate: bool, setting: Option<&str>) -> bool {
    match setting { Some("1") => true, Some("0") => false, _ => aggregate }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct Uniform {
    /// Enabled, stable field seed, f32 mean top-view coverage bits, reserved.
    pub control: [u32; 4],
    /// Fine source area-weighted linear RGB (including blade-height pigment),
    /// then roughness. No sun/sky/shadow, world tint or season applied twice.
    pub leaf_base: [f32; 4],
}

impl Uniform {
    /// Keep the existing rest top-view estimate, scaling its expected occupied
    /// mass for whole-patch removal. This is not an exact overlapping-patch
    /// union or a new growth-dependent optical bake. Actual source and far
    /// geometry share their deterministic retained root set independently.
    pub(crate) fn with_debug(mut self, options: crate::debug_settings::DebugRenderOptions) -> Self {
        if !options.draws_meadow() { self.control[0] = 0; }
        self.control[2] = (f32::from_bits(self.control[2]) * options.meadow_density).to_bits();
        for (color, multiplier) in self.leaf_base[..3].iter_mut().zip(options.meadow_color_multiplier) {
            *color *= multiplier;
        }
        self
    }

    pub(crate) fn from_field(field: Option<&MeadowField>, enabled: bool) -> Result<Self, String> {
        let Some(field) = field.filter(|_| enabled) else { return Ok(Self::zeroed()); };
        field.geometry.validate()?;
        let leaf_base = source_material(&field.geometry)?;
        let coverage = crate::meadow_aggregate::top_coverage(&field.geometry)?;
        if coverage.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err("invalid meadow substrate geometric coverage".into());
        }
        let mean = coverage.iter().sum::<f32>() / VARIANTS as f32;
        Ok(Self { control: [1, field.seed, mean.to_bits(), 0], leaf_base })
    }
}

/// Average only the finest source triangles. Coarse representation choices
/// therefore cannot change the substrate palette. Keep the same per-vertex
/// height pigment as meadow_vs; linear barycentric interpolation integrates
/// exactly to the three corner values' mean on each triangle.
fn source_material(geometry: &MeadowGeometry) -> Result<[f32; 4], String> {
    let mut sum = [0.0f64; 4];
    let mut area = 0.0f64;
    for variant in 0..VARIANTS {
        let [first, count] = geometry.lod_ranges[variant][0];
        for triangle in geometry.vertices[first as usize..(first + count) as usize].chunks_exact(3) {
            let points: [Vec3; 3] = std::array::from_fn(|i| Vec3::from_slice(&triangle[i].position));
            let weight = f64::from((points[1] - points[0]).cross(points[2] - points[0]).length());
            if weight <= 1e-20 { continue; }
            let mut color = Vec4::ZERO;
            for vertex in triangle {
                let height = (vertex.position[1] / geometry.max_tuft_height.max(0.001)).clamp(0.0, 1.0);
                let tint = Vec3::new(0.91, 0.94, 0.86).lerp(Vec3::new(1.10, 1.04, 0.98), height);
                color += (Vec3::from_slice(&vertex.color) * tint).extend(vertex.color[3]) / 3.0;
            }
            for (total, value) in sum.iter_mut().zip(color.to_array()) { *total += f64::from(value) * weight; }
            area += weight;
        }
    }
    if area <= 1e-20 || !area.is_finite() || sum.iter().any(|v| !v.is_finite()) {
        return Err("meadow substrate needs finite nondegenerate source triangles".into());
    }
    Ok(sum.map(|value| (value / area) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meadow_data::{GpuMeadowVertex, LODS};

    fn geometry() -> MeadowGeometry {
        let vertices = [[0.0, 0.0, 0.0], [0.2, 0.0, 0.0], [0.0, 0.0, 0.2]]
            .map(|p| GpuMeadowVertex {
                position: [p[0], p[1], p[2], 0.0], normal: [0.0, 1.0, 0.0, 0.0],
                color: [0.2, 0.4, 0.1, 0.9], anchor: [0.0; 4],
            }).to_vec();
        MeadowGeometry {
            vertices, lod_ranges: [[[0, 3]; LODS]; VARIANTS], lod_errors: [[0.0; LODS]; VARIANTS],
            branch_pivots: [[[0.0; 4]; 4]; VARIANTS], max_tuft_radius: 0.3, max_tuft_height: 1.0,
            max_anchor_radius: 0.3, max_blade_radius: 0.3,
        }
    }

    fn field() -> MeadowField {
        MeadowField {
            origin: [0.0; 2], cell: 2.0, width: 2, height: 2,
            samples: vec![[0.0, 10.0, 1.0, 0.0]; 4], seed: 123,
            growth: 1.0, dry_season: 0.0, radius: 100.0, fade_start: 80.0,
            geometry: geometry(),
        }
    }

    #[test]
    fn uniform_is_two_vec4_and_disabled_is_exact_zero() {
        assert_eq!(std::mem::size_of::<Uniform>(), 32);
        assert_eq!(std::mem::offset_of!(Uniform, leaf_base), 16);
        assert_eq!(Uniform::from_field(None, true).unwrap(), Uniform::zeroed());
        let mut disabled = field();
        disabled.geometry.vertices.clear();
        assert_eq!(Uniform::from_field(Some(&disabled), false).unwrap(), Uniform::zeroed());
        assert!(Uniform::from_field(Some(&disabled), true).is_err());
    }

    #[test]
    fn live_meadow_edits_preserve_default_bytes_and_share_tint_count_endpoints() {
        let base=Uniform::from_field(Some(&field()),true).unwrap();
        let defaults=crate::debug_settings::DebugRenderOptions::default();
        assert_eq!(bytemuck::bytes_of(&base),bytemuck::bytes_of(&base.with_debug(defaults)));
        let edited=base.with_debug(crate::debug_settings::DebugRenderOptions {
            meadow_density:0.25,meadow_color_multiplier:[0.0,0.5,2.0],..defaults
        });
        assert_eq!(&edited.control[..2],&base.control[..2]);
        assert_eq!(f32::from_bits(edited.control[2]),f32::from_bits(base.control[2])*0.25);
        assert_eq!(edited.leaf_base,[0.0,base.leaf_base[1]*0.5,base.leaf_base[2]*2.0,base.leaf_base[3]]);
        let empty=base.with_debug(crate::debug_settings::DebugRenderOptions {meadow_density:0.0,..defaults});
        assert_eq!(empty.control[0],0);
        assert_eq!(f32::from_bits(empty.control[2]),0.0);
        assert_eq!(base.with_debug(crate::debug_settings::DebugRenderOptions{meadow_enabled:false,..defaults}).control[0],0);
    }

    #[test]
    fn explicit_substrate_override_is_independent_and_default_tracks_aggregate() {
        for aggregate in [false, true] {
            assert!(enabled(aggregate, Some("1")));
            assert!(!enabled(aggregate, Some("0")));
            assert_eq!(enabled(aggregate, None), aggregate);
            assert_eq!(enabled(aggregate, Some("unrecognized")), aggregate);
        }
        // Enabling the switch does not invent material for a scene without
        // meadow data. Renderer stores the actual constructed uniform state.
        assert_eq!(Uniform::from_field(None, enabled(false, Some("1"))).unwrap().control[0], 0);
    }

    #[test]
    fn uniform_keeps_field_seed_and_geometric_gaps_without_double_season_tint() {
        let mut field = field();
        let first = Uniform::from_field(Some(&field), true).unwrap();
        assert_eq!(&first.control[..2], &[1, 123]);
        let coverage = f32::from_bits(first.control[2]);
        // A 0.2-by-0.2 triangle occupies only ~10% of one .45m patch.
        assert!((0.08..0.12).contains(&coverage), "actual geometric gaps: {coverage}");
        field.dry_season = 1.0; // Provenance cannot apply the season twice.
        assert_eq!(Uniform::from_field(Some(&field), true).unwrap(), first);
        for vertex in &mut field.geometry.vertices { vertex.color[0] *= 0.5; }
        let changed = Uniform::from_field(Some(&field), true).unwrap();
        assert_eq!(changed.control, first.control);
        assert!((changed.leaf_base[0] - first.leaf_base[0] * 0.5).abs() < 1e-7);
        assert_eq!(&changed.leaf_base[1..], &first.leaf_base[1..]);
    }

    #[test]
    fn shared_field_has_no_camera_light_or_resource_dependencies() {
        let module = naga::front::wgsl::parse_str(include_str!("shaders/meadow_field.wgsl")).unwrap();
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap();
        assert!(module.global_variables.is_empty());
        for name in ["meadow_material_tint", "meadow_habitat_coverage"] {
            assert!(module.functions.iter().any(|(_, function)| function.name.as_deref() == Some(name)));
        }
    }

    #[test]
    fn material_preserves_linear_rgb_roughness_and_source_height_gradient() {
        let mut g = geometry();
        let ground = source_material(&g).unwrap();
        for (actual, expected) in ground.into_iter().zip([0.182, 0.376, 0.086, 0.9]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        for vertex in &mut g.vertices { vertex.position[1] = 1.0; }
        let tip = source_material(&g).unwrap();
        for (actual, expected) in tip.into_iter().zip([0.22, 0.416, 0.098, 0.9]) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn coarse_material_does_not_change_source_palette_and_degenerate_is_rejected() {
        let mut g = geometry();
        let expected = source_material(&g).unwrap();
        let mut coarse = g.vertices.clone();
        for vertex in &mut coarse { vertex.color = [1.0; 4]; }
        g.vertices.extend(coarse);
        for variant in 0..VARIANTS { for lod in 1..LODS { g.lod_ranges[variant][lod] = [3, 3]; } }
        assert_eq!(source_material(&g).unwrap(), expected);
        for vertex in &mut g.vertices[..3] { vertex.position = [0.0; 4]; }
        assert!(source_material(&g).is_err());
    }

    #[test]
    fn material_is_weighted_by_actual_triangle_area() {
        let mut g = geometry();
        let mut large = g.vertices.clone();
        for vertex in &mut large {
            vertex.position[0] *= 2.0; vertex.position[2] *= 2.0;
            vertex.color = [0.8, 0.2, 0.5, 0.6];
        }
        g.vertices.extend(large);
        g.lod_ranges = [[[0, 6]; LODS]; VARIANTS];
        let actual = source_material(&g).unwrap();
        let expected = [(0.2 + 4.0 * 0.8) / 5.0 * 0.91,
            (0.4 + 4.0 * 0.2) / 5.0 * 0.94,
            (0.1 + 4.0 * 0.5) / 5.0 * 0.86, (0.9 + 4.0 * 0.6) / 5.0];
        for (a, b) in actual.into_iter().zip(expected) { assert!((a - b).abs() < 1e-6); }
    }
}
