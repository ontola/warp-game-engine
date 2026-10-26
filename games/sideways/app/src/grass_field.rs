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

fn source_range(aggregate: bool, override_radius: Option<&str>) -> Result<(f32, f32), String> {
    // Keep the original source-only range. With a distant replacement, use
    // fewer expensive subpixel ribbons and blend over the same twenty metres.
    let radius = match override_radius {
        Some(value) => value.parse::<f32>().map_err(|_| "invalid SIDEWAYS_MEADOW_SOURCE_RADIUS")?,
        None => if aggregate { 80.0 } else { 100.0 },
    };
    if !radius.is_finite() || !(60.0..=100.0).contains(&radius) {
        return Err("SIDEWAYS_MEADOW_SOURCE_RADIUS must be finite and within 60..100 metres".into());
    }
    Ok((radius, radius - 20.0))
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
    let samples = terrain_samples(heightfield, chunks)?;
    // Growth scales blades about their fixed basal anchors, not the patch
    // footprint. Season is a shared material operation at cooking time.
    let dry_season = match vegetation.season {
        Season::Spring | Season::Summer => 0.0,
        Season::Autumn => 0.5,
        Season::Winter => 0.8,
    };
    let budget = blade_budget(std::env::var("SIDEWAYS_MEADOW_BLADE_BUDGET").ok().as_deref())?;
    let blades = budget.blades();
    let geometry = MeadowGeometry::cook_with_blade_budget(vegetation.seed, dry_season, budget)?;
    let lod_errors = geometry.lod_errors;
    let geometry_bytes =
        geometry.vertices.len() * std::mem::size_of::<render::meadow_data::GpuMeadowVertex>();
    let triangles = geometry
        .lod_ranges
        .map(|levels| levels.map(|range| range[1] / 3));
    let valid = samples.iter().filter(|s| s[2] == 1.0).count();
    let bytes = samples.len() * std::mem::size_of::<[f32; 4]>();
    let (radius, fade_start) = source_range(
        std::env::var("KERN_MEADOW_AGGREGATE").as_deref() == Ok("1"),
        std::env::var("SIDEWAYS_MEADOW_SOURCE_RADIUS").ok().as_deref(),
    )?;
    let field = MeadowField {
        origin: heightfield.origin,
        cell: heightfield.cell,
        width: heightfield.w,
        height: heightfield.h,
        samples,
        seed: (vegetation.seed ^ (vegetation.seed >> 32)) as u32,
        growth: vegetation.grass_scale(),
        dry_season,
        radius,
        fade_start,
        geometry,
    };
    field.validate()?;
    let summary = format!(
        "GPU meadow: full terrain field {}x{} ({} valid samples / {} MiB), fixed {:.2} m world cells, 3 shared dense patch variants x {blades} opaque ribbon blades, source triangles {:?} per LOD, {} KiB shared vertices, 4 GPU wind groups with anchored blade roots, source radius {} m / transition {}..{} m; renderer reports distant aggregate configuration; no photographed grass or CPU tuft pool",
        field.width,
        field.height,
        valid,
        bytes / (1024 * 1024),
        ROOT_CELL,
        triangles[0],
        geometry_bytes.div_ceil(1024), field.radius, field.fade_start, field.radius
    );
    let metadata = serde_json::json!({
        "mode":"gpu_meadow_field", "recipe":"meadow_patch_v2", "world_cell_metres":ROOT_CELL,
        "source_blades_per_root":blades, "source_variants":3, "triangles_per_variant_lod":triangles,
        "blade_budget":blades, "blade_width_multiplier":budget.width_multiplier(),
        "source_lod_error_metres":lod_errors,
        "blade_budget_scope":"Authored ribbons per unchanged world patch; reduced budgets sample existing anchors across each wind group/prototype and widen only transverse width. Actual source/LOD/atlas/substrate recook; silhouette union and optical parity require visual acceptance.",
        "terrain_samples":valid, "terrain_dimensions":[field.width,field.height], "terrain_bytes":bytes,
        "shared_vertex_bytes":geometry_bytes, "radius_metres":field.radius, "fade_start_metres":field.fade_start,
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
    Ok((field, summary, metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(source_range(false, None).unwrap(), (100.0, 80.0));
        assert_eq!(source_range(true, None).unwrap(), (80.0, 60.0));
        assert_eq!(source_range(true, Some("100")).unwrap(), (100.0, 80.0));
        for invalid in ["NaN", "inf", "-1", "0", "59", "101", "no"] {
            assert!(source_range(true, Some(invalid)).is_err());
        }
        assert_eq!(source_range(true, Some("60")).unwrap(), (60.0, 40.0));
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
