//! Coarse material continuity beneath/farther than the directional meadow.
//!
//! The default path is an albedo approximation, not additional geometry.
//! Optional analytic basal AO affects only terrain diffuse skylight; it is
//! not a source silhouette, cast sun shadow or additional irradiance.
//! Fine source material and rest top-view geometric coverage are reduced once
//! per field. Coverage does not model runtime growth, wind or projected view.
//! Terrain still supplies its actual normal, roughness and lighting;
//! the unoccupied fraction retains its original moss/needle/soil material.

use crate::meadow_data::{MeadowField, MeadowGeometry, ROOT_CELL, VARIANTS};
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
    /// Albedo enabled, seed, f32 mean top-coverage bits, f32 contact strength.
    pub control: [u32; 4],
    /// Fine source area-weighted linear RGB (including blade-height pigment),
    /// then roughness. No sun/sky/shadow, world tint or season applied twice.
    pub leaf_base: [f32; 4],
    /// Actual immutable meadow terrain footprint, minXZ/maxXZ.
    pub field_bounds: [f32; 4],
    /// Retained-root fraction, analytic basal radius, fixed anchor radius,
    /// maximum grown/wind blade reach used for a conservative road setback.
    pub contact: [f32; 4],
}

impl Uniform {
    /// Keep the existing rest top-view estimate, scaling its expected occupied
    /// mass for whole-patch removal. This is not an exact overlapping-patch
    /// union or a new growth-dependent optical bake. Actual source and far
    /// geometry share their deterministic retained root set independently.
    pub(crate) fn with_debug(mut self, options: crate::debug_settings::DebugRenderOptions) -> Self {
        if self.control[0] == 0 && self.control[3] == 0 { return self; }
        if !options.draws_meadow() { self.control[0] = 0; self.control[3] = 0; }
        self.control[2] = (f32::from_bits(self.control[2]) * options.meadow_density).to_bits();
        self.contact[0] = options.meadow_density;
        self.contact[1] = (self.contact[1] * (0.85 + 0.15 * options.meadow_size_scale))
            .min(ROOT_CELL * 0.96 - 0.0001);
        self.contact[3] *= options.meadow_size_scale;
        self.control[3] = (f32::from_bits(self.control[3])
            * (2.0 * options.meadow_size_scale / (1.0 + options.meadow_size_scale))).to_bits();
        for (color, multiplier) in self.leaf_base[..3].iter_mut().zip(options.meadow_color_multiplier) {
            *color *= multiplier;
        }
        self
    }

    /// Independent material/AO switches. A scene without valid field data
    /// remains disabled; enabling AO must not force the substrate colour on.
    pub(crate) fn with_features(mut self, substrate: bool, contact: bool) -> Self {
        if self.control[0] == 0 { return self; }
        self.control[0] = u32::from(substrate);
        self.control[3] = if contact { 0.35f32.to_bits() } else { 0 };
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
        let max_angle = field.geometry.branch_pivots.iter().flatten()
            .map(|p| p[3].abs()).fold(0.0f32, f32::max);
        let radius = field.geometry.max_anchor_radius.max(0.08).min(ROOT_CELL * 0.96 - 0.0001);
        let blade_reach = field.geometry.max_blade_radius * field.growth * 1.3 * (1.0 + max_angle * 1.5);
        Ok(Self { control: [1, field.seed, mean.to_bits(), 0], leaf_base,
            field_bounds: [field.origin[0], field.origin[1],
                field.origin[0] + field.cell * field.width.saturating_sub(1) as f32,
                field.origin[1] + field.cell * field.height.saturating_sub(1) as f32],
            contact: [1.0, radius, field.geometry.max_anchor_radius, blade_reach] })
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
    fn uniform_is_four_vec4_and_disabled_is_exact_zero() {
        assert_eq!(std::mem::size_of::<Uniform>(), 64);
        assert_eq!(std::mem::offset_of!(Uniform, leaf_base), 16);
        assert_eq!(std::mem::offset_of!(Uniform, field_bounds), 32);
        assert_eq!(std::mem::offset_of!(Uniform, contact), 48);
        assert_eq!(Uniform::from_field(None, true).unwrap(), Uniform::zeroed());
        assert_eq!(Uniform::zeroed().with_debug(Default::default()), Uniform::zeroed());
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
    fn contact_ao_is_independent_of_substrate_and_respects_live_empty_size_and_density() {
        let base=Uniform::from_field(Some(&field()),true).unwrap();
        let defaults=crate::debug_settings::DebugRenderOptions::default();
        for substrate in [false,true] {for contact in [false,true] {
            let configured=base.with_features(substrate,contact);
            assert_eq!(configured.control[0],u32::from(substrate));
            assert_eq!(f32::from_bits(configured.control[3]),if contact {0.35} else {0.0});
        }}
        let ao=base.with_features(false,true);
        assert_eq!(bytemuck::bytes_of(&ao),bytemuck::bytes_of(&ao.with_debug(defaults)));
        for options in [crate::debug_settings::DebugRenderOptions{meadow_enabled:false,..defaults},
            crate::debug_settings::DebugRenderOptions{meadow_density:0.0,..defaults}] {
            let empty=ao.with_debug(options);
            assert_eq!(empty.control[0],0);
            assert_eq!(f32::from_bits(empty.control[3]),0.0);
        }
        for density in [0.15,0.55,1.0] {for size in [0.5,1.0,2.0] {
            let edited=ao.with_debug(crate::debug_settings::DebugRenderOptions {
                meadow_density:density,meadow_size_scale:size,..defaults
            });
            assert_eq!(edited.control[0],0,"AO must not enable albedo replacement");
            assert_eq!(edited.contact[0],density);
            // The shader recovers the unscaled rest coverage before applying
            // the actual retained-cell hash, rather than thinning it twice.
            assert!((f32::from_bits(edited.control[2])/density-f32::from_bits(base.control[2])).abs()<1e-7);
            assert!((f32::from_bits(edited.control[3])-0.35*(2.0*size/(1.0+size))).abs()<1e-7);
            assert!(edited.contact[1]>0.0 && edited.contact[1]<=ROOT_CELL*0.96-0.0001);
            assert_eq!(edited.contact[3],ao.contact[3]*size);
        }}
        assert_eq!(Uniform::from_field(None,true).unwrap().with_features(false,true),Uniform::zeroed());
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
        for name in ["meadow_material_tint", "meadow_habitat_coverage", "meadow_patch_retained",
            "meadow_root_fraction", "meadow_contact_cover"] {
            assert!(module.functions.iter().any(|(_, function)| function.name.as_deref() == Some(name)));
        }
    }

    #[test]
    fn contact_ao_shared_consumers_validate_and_match_uniform_layout() {
        for source in [crate::meadow::generate_source(),crate::meadow::shader_source(),
            crate::meadow::aggregate_resolve_source(),crate::meadow_brick_field::shader_source(),
            crate::wind::shade_source(),crate::grass::shader_source(),crate::far_grass::shader_source(),
            crate::voxel_forest::shader_source()] {
            let module=naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error|panic!("{}",error.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|error|panic!("{}",error.emit_to_string(&source)));
            if let Some((_,ty))=module.types.iter().find(|(_,ty)|ty.name.as_deref()==Some("MeadowSubstrate")) {
                let naga::TypeInner::Struct{members,span}=&ty.inner else {panic!("uniform struct")};
                assert_eq!(*span,std::mem::size_of::<Uniform>() as u32);
                assert_eq!(members.iter().map(|m|m.offset).collect::<Vec<_>>(),[0,16,32,48]);
            }
            let contact=module.functions.iter().find(|(_,f)|f.name.as_deref()==Some("meadow_contact_cover")).unwrap().1;
            for name in ["meadow_patch_retained","meadow_root_fraction"] {
                let handle=module.functions.iter().find(|(_,f)|f.name.as_deref()==Some(name)).unwrap().0;
                assert!(contact.expressions.iter().any(|(_,e)|matches!(e,naga::Expression::CallResult(h) if *h==handle)));
            }
        }
        let shade=include_str!("shaders/shade.wgsl");
        // Lock the lighting scope: one neutral initializer, one terrain-only
        // assignment, and one diffuse-sky factor. No direct/specular/fog use.
        assert_eq!(shade.matches("meadow_ground_sky").count(),3);
        assert!(shade.contains("sky_irradiance(n)*sky_vis*meadow_ground_sky+bounce*(1.0-sky_vis)"));
        assert!(shade.contains("if meadow_substrate.control.x != 0u"));
    }

    fn contact_random(cell:[i32;2],salt:u32,seed:u32)->f32 {
        let mut x=(cell[0] as u32).wrapping_mul(0x9e3779b1)
            ^(cell[1] as u32).wrapping_mul(0x85ebca77)^salt.wrapping_mul(0xc2b2ae3d)^seed;
        x^=x>>16;x=x.wrapping_mul(0x7feb352d);x^=x>>15;
        x=x.wrapping_mul(0x846ca68b);x^=x>>16;
        (x>>8) as f32/16777216.0
    }

    fn contact_root(cell:[i32;2],seed:u32)->[f32;2] {
        std::array::from_fn(|axis| (cell[axis] as f32+(0.5
            +(contact_random(cell,[157,163][axis],seed)*2.0-1.0)*0.04))*ROOT_CELL)
    }

    /// CPU support oracle, using a deliberately wider neighbourhood to prove
    /// the bounded shader scan omits only zero-valued kernels. No rendered
    /// image or geometric grass coverage is inferred from this approximation.
    fn contact_cover(p:[f32;2],seed:u32,density:f32,radius:f32,padding:i32)->f32 {
        if density<=0.0 || radius<=0.0 {return 0.0;}
        let roundoff=8.0*f32::EPSILON*(p[0].abs().max(p[1].abs())+ROOT_CELL);
        let r=radius.min((ROOT_CELL*0.96-0.0001-roundoff).max(0.0));
        if r<=0.0 {return 0.0;}
        let first=p.map(|v|(v/ROOT_CELL-0.5).floor() as i32);
        let mut gap=1.0;
        for z in -padding..2+padding {for x in -padding..2+padding {
            let cell=[first[0]+x,first[1]+z];
            if density<1.0 && contact_random(cell,191,seed)>=density {continue;}
            let root=contact_root(cell,seed);
            let d2=(p[0]-root[0]).powi(2)+(p[1]-root[1]).powi(2);
            let t=((d2-0.0625*r*r)/(r*r-0.0625*r*r)).clamp(0.0,1.0);
            let footprint=1.0-t*t*(3.0-2.0*t);
            gap*=1.0-0.8*footprint;
        }}
        1.0-gap
    }

    #[test]
    fn contact_four_root_support_matches_wide_oracle_and_is_continuous_at_cell_seams() {
        let radius=ROOT_CELL*0.96-0.0001;
        // Include negative cells and the actual several-kilometre stage scale.
        for seed in [0,123,u32::MAX] {for offset in [-6000,-7,0,9,6000] {
            for z in 0..17 {for x in 0..17 {
                let p=[(offset as f32+x as f32/16.0)*ROOT_CELL,
                    (offset as f32+z as f32/16.0)*ROOT_CELL];
                for density in [0.15,0.55,1.0] {
                    assert_eq!(contact_cover(p,seed,density,radius,0),contact_cover(p,seed,density,radius,3),
                        "all omitted roots must have zero support: {p:?} seed={seed}");
                }
            }}
        }}
        for cell in -12..12 {for z in [-0.1,0.3,1.7] {
            let x=(cell as f32+0.5)*ROOT_CELL;
            let left=contact_cover([x-1e-5,z],123,0.55,radius,0);
            let right=contact_cover([x+1e-5,z],123,0.55,radius,0);
            assert!((left-right).abs()<0.001,"no lattice boundary jump");
        }}
    }

    #[test]
    fn contact_retains_real_cell_gaps_and_density_is_monotone_without_camera_inputs() {
        let mut empty_centres=0;
        let mut occupied_centres=0;
        for z in -16..16 {for x in -16..16 {
            let cell=[x,z];let p=contact_root(cell,123);
            let values=[0.0,0.15,0.55,1.0].map(|d|contact_cover(p,123,d,0.38,0));
            assert_eq!(values[0],0.0);
            assert!(values.windows(2).all(|v|v[0]<=v[1]));
            // Radius .38 is below minimum neighbour-root separation .414:
            // a deleted centre is exactly undarkened, not uniformly tinted.
            if contact_random(cell,191,123)<0.55 {
                assert!((values[2]-0.8).abs()<1e-6);occupied_centres+=1;
            } else {assert_eq!(values[2],0.0);empty_centres+=1;}
        }}
        assert!(empty_centres>100 && occupied_centres>100);
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
