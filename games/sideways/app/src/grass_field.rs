//! Stage data for the GPU meadow renderer. No CPU root list, moving plant
//! pool, photographed fallback, or bounded demonstration rectangle.
use render::{
    meadow_data::{MAX_FIELD_SAMPLES, MeadowField, MeadowGeometry, ROOT_CELL},
    foliage_assembly::MeadowBladeBudget,
    scene::Heightfield,
};
use sideways::{
    builder::{Chunk, N},
    stage::GRID,
    vegetation::{Season, Vegetation},
};

pub fn enabled() -> bool {
    std::env::var("SIDEWAYS_PROCEDURAL_GRASS").as_deref() == Ok("field")
}

fn blade_budget(value: Option<&str>) -> Result<MeadowBladeBudget, String> {
    match value {
        None => Ok(MeadowBladeBudget::Blades360),
        Some(value) => MeadowBladeBudget::try_from(value.parse::<u32>()
            .map_err(|_| "SIDEWAYS_MEADOW_BLADE_BUDGET must be 120, 180 or 360")?),
    }
}

fn blade_shading(value: Option<&str>) -> Result<&'static str, String> {
    match value {
        None | Some("flat") => Ok("flat"),
        Some("rounded") => Ok("rounded"),
        Some(_) => Err("SIDEWAYS_MEADOW_BLADE_SHADING must be flat or rounded".into()),
    }
}

fn profile(value: Option<&str>) -> Result<&'static str, String> {
    match value {
        None | Some("windswept") => Ok("windswept"),
        Some("classic") => Ok("classic"),
        Some(_) => Err("SIDEWAYS_MEADOW_PROFILE must be classic or windswept".into()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceRange {
    radius: f32,
    fade_start: f32,
    origin: &'static str,
}

impl SourceRange {
    fn metadata(self, override_radius: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "metres":[self.fade_start,self.radius],
            "origin":self.origin,
            "ignored_legacy_radius":if self.origin == "SIDEWAYS_MEADOW_SOURCE_BAND" { override_radius } else { None },
            "scope":"Authored source-to-aggregate endpoints before the live distance multiplier. This override does not change the renderer's 160..180 m outer fade; actual scaled endpoints are reported in render.meadow."
        })
    }
}

fn source_range(
    aggregate: bool,
    override_radius: Option<&str>,
    override_band: Option<&str>,
) -> Result<SourceRange, String> {
    // The explicit pair wins over a radius inherited from a saved launcher
    // preset. Never silently turn a source handoff into a source-only fade.
    if let Some(value) = override_band {
        if !aggregate {
            return Err("SIDEWAYS_MEADOW_SOURCE_BAND requires KERN_MEADOW_AGGREGATE=1".into());
        }
        let error = "SIDEWAYS_MEADOW_SOURCE_BAND must be start,end metres with finite 0 <= start < end and 10 <= end <= 100";
        let mut parts = value.split(',');
        let fade_start = parts.next().ok_or(error)?.trim().parse::<f32>().map_err(|_| error)?;
        let radius = parts.next().ok_or(error)?.trim().parse::<f32>().map_err(|_| error)?;
        if parts.next().is_some() || !fade_start.is_finite() || !radius.is_finite()
            || fade_start < 0.0 || fade_start >= radius || !(10.0..=100.0).contains(&radius)
        {
            return Err(error.into());
        }
        return Ok(SourceRange { radius, fade_start, origin: "SIDEWAYS_MEADOW_SOURCE_BAND" });
    }
    // Keep the original source-only range. With a distant replacement, use
    // fewer expensive subpixel ribbons and blend over the same twenty metres.
    let radius = match override_radius {
        Some(value) => value.parse::<f32>().map_err(|_| "invalid SIDEWAYS_MEADOW_SOURCE_RADIUS")?,
        None => if aggregate { 80.0 } else { 100.0 },
    };
    if !radius.is_finite() || !(60.0..=100.0).contains(&radius) {
        return Err("SIDEWAYS_MEADOW_SOURCE_RADIUS must be finite and within 60..100 metres".into());
    }
    Ok(SourceRange {
        radius,
        fade_start: radius - 20.0,
        origin: if override_radius.is_some() { "SIDEWAYS_MEADOW_SOURCE_RADIUS" } else { "default" },
    })
}

fn windswept_range(override_radius: Option<&str>) -> Result<SourceRange, String> {
    let radius = override_radius.map_or(Ok(250.0), |value| value.parse::<f32>())
        .map_err(|_| "invalid SIDEWAYS_MEADOW_WINDSWEPT_RADIUS")?;
    if !radius.is_finite() || !(60.0..=256.0).contains(&radius) {
        return Err("SIDEWAYS_MEADOW_WINDSWEPT_RADIUS must be finite and within 60..256 metres".into());
    }
    Ok(SourceRange { radius, fade_start: radius * 0.8,
        origin: if override_radius.is_some() { "SIDEWAYS_MEADOW_WINDSWEPT_RADIUS" } else { "windswept_geometry_lods" } })
}

fn terrain_samples(heightfield: &Heightfield, chunks: &[Chunk]) -> Result<Vec<[f32; 4]>, String> {
    let size = heightfield.w as u64 * heightfield.h as u64;
    if heightfield.w < 2
        || heightfield.h < 2
        || size > MAX_FIELD_SAMPLES as u64
        || size != heightfield.heights.len() as u64
        || heightfield.cell != GRID as f32
    {
        return Err("meadow field requires the bounded native-spacing terrain heightfield".into());
    }
    // Distant forest adds terrain to this same field after the native chunks.
    // Those points have no driving road; native road clearances overwrite
    // them below. Holes remain invalid, never a low artificial grassy plane.
    let mut samples: Vec<_> = heightfield
        .heights
        .iter()
        .map(|&height| {
            if height.is_finite() && height > -1e8 {
                [height, 100.0, 1.0, 0.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }
        })
        .collect();
    for chunk in chunks {
        if chunk.heights.len() != N * N || chunk.road_d.len() != N * N {
            return Err("meadow terrain chunk has inconsistent sample counts".into());
        }
        let offset = [
            (chunk.x0 as f32 - heightfield.origin[0]) / heightfield.cell,
            (chunk.z0 as f32 - heightfield.origin[1]) / heightfield.cell,
        ];
        if offset
            .iter()
            .any(|&v| !v.is_finite() || v < 0.0 || (v - v.round()).abs() > 1e-4)
        {
            return Err("meadow terrain chunk is not aligned with its render heightfield".into());
        }
        let [ox, oz] = offset.map(|v| v.round() as usize);
        if ox + N > heightfield.w as usize || oz + N > heightfield.h as usize {
            return Err("meadow native chunk lies outside the render heightfield".into());
        }
        for z in 0..N {
            for x in 0..N {
                let source = z * N + x;
                let target = (oz + z) * heightfield.w as usize + ox + x;
                let height = chunk.heights[source];
                let road = chunk.road_d[source];
                if !height.is_finite() || !road.is_finite() {
                    return Err(
                        "meadow native terrain contains a nonfinite height or road clearance"
                            .into(),
                    );
                }
                // Compare bits: the source and the root-support surface must not
                // silently diverge at a road shoulder or a chunk seam.
                if heightfield.heights[target].to_bits() != height.to_bits() {
                    return Err(
                        "meadow root heights differ from the rendered native heightfield".into(),
                    );
                }
                samples[target] = [height, road, 1.0, 0.0];
            }
        }
    }
    Ok(samples)
}

pub fn build(
    heightfield: &Heightfield,
    chunks: &[Chunk],
    vegetation: Vegetation,
) -> Result<(MeadowField, String, serde_json::Value), String> {
    let profile = profile(std::env::var("SIDEWAYS_MEADOW_PROFILE").ok().as_deref())?;
    let override_radius = std::env::var("SIDEWAYS_MEADOW_SOURCE_RADIUS").ok();
    let override_band = std::env::var("SIDEWAYS_MEADOW_SOURCE_BAND").ok();
    let source_range = if profile == "windswept" {
        windswept_range(std::env::var("SIDEWAYS_MEADOW_WINDSWEPT_RADIUS").ok().as_deref())?
    } else { source_range(
        std::env::var("KERN_MEADOW_AGGREGATE").as_deref() == Ok("1"),
        override_radius.as_deref(),
        override_band.as_deref(),
    )? };
    let samples = terrain_samples(heightfield, chunks)?;
    // Growth scales blades about their fixed basal anchors, not the patch
    // footprint. Season is a shared material operation at cooking time.
    let dry_season = match vegetation.season {
        Season::Spring | Season::Summer => 0.0,
        Season::Autumn => 0.5,
        Season::Winter => 0.8,
    };
    let requested_budget = std::env::var("SIDEWAYS_MEADOW_BLADE_BUDGET").ok();
    let budget = blade_budget(requested_budget.as_deref().or(if profile == "windswept" {Some("120")} else {None}))?;
    let blades = if profile == "windswept" {budget.blades() / 3} else {budget.blades()};
    let shading = blade_shading(std::env::var("SIDEWAYS_MEADOW_BLADE_SHADING").ok().as_deref())?;
    let geometry = if profile == "windswept" {
        MeadowGeometry::cook_windswept(vegetation.seed, dry_season, budget)?
    } else if shading == "rounded" {
        MeadowGeometry::cook_with_blade_budget_and_rounded_normals(vegetation.seed, dry_season, budget)?
    } else {
        MeadowGeometry::cook_with_blade_budget(vegetation.seed, dry_season, budget)?
    };
    let lod_errors = geometry.lod_errors;
    let geometry_bytes =
        geometry.vertices.len() * std::mem::size_of::<render::meadow_data::GpuMeadowVertex>();
    let triangles = geometry
        .lod_ranges
        .map(|levels| levels.map(|range| range[1] / 3));
    let valid = samples.iter().filter(|s| s[2] == 1.0).count();
    let bytes = samples.len() * std::mem::size_of::<[f32; 4]>();
    let field = MeadowField {
        origin: heightfield.origin,
        cell: heightfield.cell,
        width: heightfield.w,
        height: heightfield.h,
        samples,
        seed: (vegetation.seed ^ (vegetation.seed >> 32)) as u32,
        growth: vegetation.grass_scale(),
        dry_season,
        radius: source_range.radius,
        fade_start: source_range.fade_start,
        geometry,
    };
    field.validate()?;
    let summary = format!(
        "GPU meadow ({profile}): full terrain field {}x{} ({} valid samples / {} MiB), fixed {:.2} m world cells, 3 shared patch variants x {blades} opaque ribbon blades, source triangles {:?} per LOD, {} KiB shared vertices, anchored blade roots, source radius {} m / transition {}..{} m; renderer reports distant aggregate configuration; no photographed grass or CPU tuft pool",
        field.width,
        field.height,
        valid,
        bytes / (1024 * 1024),
        ROOT_CELL,
        triangles[0],
        geometry_bytes.div_ceil(1024), field.radius, field.fade_start, field.radius
    );
    let mut metadata = serde_json::json!({
        "mode":"gpu_meadow_field", "recipe":"meadow_patch_v2", "world_cell_metres":ROOT_CELL,
        "source_blades_per_root":blades, "source_variants":3, "triangles_per_variant_lod":triangles,
        "blade_budget":blades, "blade_width_multiplier":budget.width_multiplier(),
        "source_lod_error_metres":lod_errors,
        "blade_budget_scope":"Authored ribbons per unchanged world patch; reduced budgets sample existing anchors across each wind group/prototype and widen only transverse width. Actual source/LOD/atlas/substrate recook; silhouette union and optical parity require visual acceptance.",
        "terrain_samples":valid, "terrain_dimensions":[field.width,field.height], "terrain_bytes":bytes,
        "shared_vertex_bytes":geometry_bytes, "radius_metres":field.radius, "fade_start_metres":field.fade_start,
        "authored_source_band":source_range.metadata(override_radius.as_deref()),
        "growth_scale":field.growth, "dry_season":dry_season, "placement":"GPU world-cell habitat, native triangle height and road clearance",
        "wind":"four GPU angle groups per patch; each blade bends about its own fixed root; current and previous pose share geometry",
        "material_variation":"stable world-space fresh/olive/dry patches plus small per-root and blade-height pigment variation; shared across source LODs",
        "root_distribution":"Stratified basal roots in overlapping rounded patches with tapered edge density; optional per-patch source budget retains every world patch and balanced wind groups/prototypes; stable world-root jitter and full-angle yaw, footprint independent of growth",
        "growth_distribution":"smooth world-space variation at 9 m and 3 m scales with 5% stable per-patch variation; source and aggregate share the same growth",
        "terrain_alignment":"root triangle gradient shears current/previous branch poses with inverse-transpose normals; 5 mm soil inset",
        "source_derived_closed_lods":false, "source_derived_ribbon_lods":true, "all_blades_retained":true, "voxel_representation":false,
        "lod_error_metric":"physical affine source-to-cut ribbon correspondence; conservative scalar fallback for unsupported geometry; projected error threshold unchanged",
        "all_blades_retained_scope":"every authored blade survives source LOD selection; static road-edge habitat may reject blades before rasterization",
        "road_edge_filter":"stable per-blade road-plane habitat inside one native triangle; conservative whole-patch fallback at triangle boundaries",
        "geometry_count_scope":"selected patch source/submitted triangles and authored blades; upper bounds before per-blade habitat and raster visibility rejection",
        "nominal_blades_per_square_metre":blades as f64 / (ROOT_CELL as f64 * ROOT_CELL as f64),
        "projection_measure":"SheetCrossings; opaque leaf surfaces, not closed volumes",
        "photographed_fallback":false, "cpu_tuft_streaming":false, "bounded_demo_patch":false,
        "distant_representation_scope":"Actual constructed aggregate configuration and GPU counts are reported in render.meadow; source radius is not the aggregate draw distance.",
        "optical_parity_validated":false, "collision":"decorative only; unchanged physics"
    });
    metadata["blade_shading"] = serde_json::json!(shading);
    metadata["blade_shading_scope"] = serde_json::json!("Rounded is opt-in source shading, shared by every source LOD and the rebuilt distant atlas. Geometry, material, placement and wind remain unchanged. Distant aggregate shadow bias still uses its existing approximate resolved shading normal.");
    metadata["profile"] = serde_json::json!(profile);
    if profile == "windswept" {
        metadata["recipe"] = serde_json::json!("meadow_windswept_v1");
        metadata["blade_budget"] = serde_json::json!(budget.blades());
        metadata["blade_width_multiplier"] = serde_json::json!(1.0);
        metadata["blade_budget_scope"] = serde_json::json!("Budget levels 120/180/360 select 40/60/120 taller ribbons per fixed patch. No compensating width multiplier. Default 40 gives ~198 blades/m² before habitat and live density.");
        metadata["blade_shading"] = serde_json::json!("curved");
        metadata["blade_shading_scope"] = serde_json::json!("Six-segment source, rounded cross-blade normals, nonlinear wind with analytic normal/area transformation. Atlas representative points share deformation; its rest coverage remains approximate.");
        metadata["wind"] = serde_json::json!("Per-blade scrolling value noise; rotation grows from zero at root to full tip angle; independent previous direction, amplitude and time. Based on SimonDev Quick_Grass.");
        metadata["material_variation"] = serde_json::json!("Per-blade dark green base to yellow-green tip via h^4, stable .75..1 shade and .25+.75*h² basal darkening. Existing world pigment, season and live RGB edits still apply.");
        metadata["lod_error_metric"] = serde_json::json!("Conservative animated error bounds recorded; source morphs over authored3..6m scaled by growth to the actual animated one-triangle surface, then switches six-to-one segments at6m. Every blade is retained. The cooked three-segment cut is not selected. No projected-pixel-error guarantee for this profile.");
        metadata["material_interpolation"] = serde_json::json!("Base and tip pigment plus normalized blade height reach the fragment shader; t^4 pigment and density-one AO are evaluated per pixel identically for all LODs. Seasonal clamp precedes patch/live pigment.");
        metadata["authored_source_band"] = serde_json::json!({
            "metres":[source_range.fade_start,source_range.radius], "origin":source_range.origin,
            "ignored_legacy_radius":override_radius, "ignored_legacy_band":override_band,
            "scope":"Source-only outer fade in horizontal metres before the live distance multiplier; no distant replacement follows it. Classic source/aggregate ranges do not apply."
        });
        metadata["distant_representation_scope"] = serde_json::json!(format!(
            "Source geometry only through{}m, fading{}..{}m before live distance edits. Classic source-band and aggregate settings do not apply; no overlapping patch atlas for tall nonlinear blades.", source_range.radius, source_range.fade_start, source_range.radius));
    }
    Ok((field, summary, metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grass_profile_default_and_legacy_are_explicit() {
        assert_eq!(profile(None).unwrap(), "windswept");
        assert_eq!(profile(Some("classic")).unwrap(), "classic");
        assert_eq!(profile(Some("windswept")).unwrap(), "windswept");
        assert!(profile(Some("vibes")).is_err());
    }
    #[test]
    fn windswept_outer_range_is_explicit_bounded_and_independent_of_classic_handoff() {
        assert_eq!(windswept_range(None).unwrap(), SourceRange {
            radius:250.0, fade_start:200.0, origin:"windswept_geometry_lods" });
        for radius in [60.0,100.0,180.0,250.0,256.0] {
            let range = windswept_range(Some(&radius.to_string())).unwrap();
            assert_eq!(range.radius,radius);
            assert_eq!(range.fade_start,radius*0.8);
        }
        for invalid in ["NaN","inf","-1","0","59.99","256.01","no"] {
            assert!(windswept_range(Some(invalid)).is_err());
        }
    }
    #[test]
    fn blade_shading_requires_an_explicit_supported_mode() {
        assert_eq!(blade_shading(None).unwrap(), "flat");
        assert_eq!(blade_shading(Some("flat")).unwrap(), "flat");
        assert_eq!(blade_shading(Some("rounded")).unwrap(), "rounded");
        for invalid in ["", "1", "0", "true", "Rounded", " rounded", "smooth"] {
            assert!(blade_shading(Some(invalid)).is_err());
        }
    }
    #[test]
    fn meadow_blade_budget_defaults_and_overrides_are_explicit() {
        assert_eq!(blade_budget(None).unwrap(),MeadowBladeBudget::Blades360);
        for count in [120,180,360] {
            assert_eq!(blade_budget(Some(&count.to_string())).unwrap().blades(),count);
        }
        for value in ["0","1","119","121","180.0","NaN","361","no"] {
            assert!(blade_budget(Some(value)).is_err());
        }
    }
    #[test]
    fn source_range_preserves_reference_and_rejects_invalid_overrides() {
        assert_eq!(source_range(false, None, None).unwrap(), SourceRange { radius:100.0, fade_start:80.0, origin:"default" });
        assert_eq!(source_range(true, None, None).unwrap(), SourceRange { radius:80.0, fade_start:60.0, origin:"default" });
        assert_eq!(source_range(true, Some("100"), None).unwrap(), SourceRange { radius:100.0, fade_start:80.0, origin:"SIDEWAYS_MEADOW_SOURCE_RADIUS" });
        for invalid in ["NaN", "inf", "-1", "0", "59", "101", "no"] {
            assert!(source_range(true, Some(invalid), None).is_err());
        }
        assert_eq!(source_range(true, Some("60"), None).unwrap(), SourceRange { radius:60.0, fade_start:40.0, origin:"SIDEWAYS_MEADOW_SOURCE_RADIUS" });
    }
    #[test]
    fn explicit_source_band_overrides_legacy_radius_and_records_resolved_values() {
        for radius in [None, Some("80"), Some("100"), Some("unused")] {
            let band = source_range(true, radius, Some(" 40 , 50 ")).unwrap();
            assert_eq!(band, SourceRange { radius:50.0, fade_start:40.0, origin:"SIDEWAYS_MEADOW_SOURCE_BAND" });
            let metadata = band.metadata(radius);
            assert_eq!(metadata["metres"], serde_json::json!([40.0,50.0]));
            assert_eq!(metadata["origin"], "SIDEWAYS_MEADOW_SOURCE_BAND");
            assert_eq!(metadata["ignored_legacy_radius"], serde_json::json!(radius));
        }
        let default = source_range(true, None, None).unwrap().metadata(None);
        assert_eq!(default["metres"], serde_json::json!([60.0,80.0]));
        assert_eq!(default["origin"], "default");
        assert_eq!(default["ignored_legacy_radius"], serde_json::Value::Null);
    }
    #[test]
    fn explicit_source_band_is_bounded_and_requires_a_replacement() {
        assert!(source_range(false, None, Some("40,50")).is_err());
        for invalid in ["", "40", "40;50", ",50", "40,", "40,50,60", "40,50,",
            "NaN,50", "40,NaN", "-inf,50", "40,inf", "-1,50", "50,50", "51,50",
            "0,9.99", "40,100.01"] {
            assert!(source_range(true, None, Some(invalid)).is_err(), "accepted {invalid:?}");
        }
        for band in ["0,10", "40,50", "60,80", "80,100"] {
            assert!(source_range(true, None, Some(band)).is_ok(), "rejected {band:?}");
        }
    }
    fn chunk() -> Chunk {
        Chunk {
            cell: (0, 0),
            x0: 0.0,
            z0: 0.0,
            heights: (0..N * N)
                .map(|i| {
                    let x = (i % N) as f32;
                    let z = (i / N) as f32;
                    x * 0.13 + z * 0.21 + (x * z) * 0.003
                })
                .collect(),
            road_d: (0..N * N).map(|i| (i % N) as f32 - 4.0).collect(),
            trees: vec![],
            rocks: vec![],
            grass: vec![],
            ferns: vec![],
            stumps: vec![],
        }
    }
    #[test]
    fn native_height_bits_and_signed_road_clearance_survive_with_invalid_holes() {
        let chunk = chunk();
        let width = N + 2;
        let mut heightfield = Heightfield {
            origin: [0.0; 2],
            cell: GRID as f32,
            w: width as u32,
            h: width as u32,
            heights: vec![-1e9; width * width],
        };
        for z in 0..N {
            for x in 0..N {
                heightfield.heights[z * width + x] = chunk.heights[z * N + x];
            }
        }
        heightfield.heights[N * width + N] = 55.0; // supported background, no road
        let samples = terrain_samples(&heightfield, std::slice::from_ref(&chunk)).unwrap();
        for z in 0..N {
            for x in 0..N {
                let sample = samples[z * width + x];
                assert_eq!(sample[0].to_bits(), chunk.heights[z * N + x].to_bits());
                assert_eq!(sample[1], chunk.road_d[z * N + x]);
                assert_eq!(sample[2], 1.0);
            }
        }
        assert!(
            samples[0][1] < 0.0,
            "road interior must remain excluded by the habitat shader"
        );
        assert_eq!(samples[N * width + N], [55.0, 100.0, 1.0, 0.0]);
        assert_eq!(samples[(width - 1) * width + width - 1], [0.0; 4]);
        heightfield.heights[4] = 999.0;
        assert!(terrain_samples(&heightfield, &[chunk]).is_err());
    }
    #[test]
    fn grid_keeps_native_triangle_support_instead_of_bilinear_surface() {
        let chunk = chunk();
        let heightfield = Heightfield {
            origin: [0.0; 2],
            cell: GRID as f32,
            w: N as u32,
            h: N as u32,
            heights: chunk.heights.clone(),
        };
        let samples = terrain_samples(&heightfield, std::slice::from_ref(&chunk)).unwrap();
        for (x, z) in [(0.2, 0.3), (0.8, 0.7), (0.5, 0.5)] {
            let (ids, weights) = if x + z <= 1.0 {
                ([0, 1, N], [1.0 - x - z, x, z])
            } else {
                ([N + 1, N, 1], [x + z - 1.0, 1.0 - x, 1.0 - z])
            };
            let sampled: f32 = (0..3).map(|k| samples[ids[k]][0] * weights[k]).sum();
            let native: f32 = (0..3).map(|k| chunk.heights[ids[k]] * weights[k]).sum();
            assert_eq!(sampled.to_bits(), native.to_bits());
        }
    }
}
