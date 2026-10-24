//! Bounded terrain-relative far grass, derived from the live tuft source.
//!
//! Native triangle heights/road distances remain exact. Four fixed spatial /
//! size samples estimate the live habitat probability per 2 m terrain cell;
//! this is mean coverage, not an exact copy of every hashed individual tuft.
//! The source photo and size distribution determine four vertical extinction
//! bins. No camera position, frame RNG, or additional near instances are used.

use crate::ground_cover::{HEIGHT, WIDTH};
use crate::live_ground_cover::{CELL, Ground, ROOT_OFFSET, SCALE_MIN, SCALE_SPAN};
use glam::Vec2;
use render::far_grass::FarGrassField;
use sideways::builder::{Chunk, N};
use sideways::stage::{CHUNK, GRID};
use std::{collections::HashSet, time::Instant};

/// At most 32 MiB of height/road points plus approximately 4 MiB of coverage.
/// Stage construction fails explicitly instead of silently allocating a huge
/// rectangular field for widely separated or malformed native chunks.
const MAX_POINTS: usize = 4_194_304;
const PROFILE_SCALES: usize = 16;

fn linear(value: u8) -> f64 {
    let c = value as f64 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

struct PhotoProfile {
    extinction: [f32; 4],
    max_height: f32,
    albedo: [f32; 3],
    shading: [[f32; 4]; 4],
}

fn photo_profile(photo: &image::RgbaImage) -> Result<PhotoProfile, String> {
    photo_profile_with_vegetation(photo, Default::default())
}

fn photo_profile_with_vegetation(photo: &image::RgbaImage, vegetation: sideways::vegetation::Vegetation) -> Result<PhotoProfile, String> {
    if photo.width() == 0 || photo.height() == 0 {
        return Err("far grass requires a non-empty source clump photo".into());
    }
    let mut rows = vec![0.0f64; photo.height() as usize];
    let mut row_color = vec![[0.0f64; 3]; photo.height() as usize];
    let mut color = [0.0f64; 3];
    let mut covered = 0u64;
    for (_, y, pixel) in photo.enumerate_pixels() {
        // Match the fixed 0.5 photo cutout in both live grass raster passes.
        if pixel[3] < 128 { continue; }
        rows[y as usize] += 1.0 / photo.width() as f64;
        for k in 0..3 {
            let value = linear(pixel[k]) * vegetation.grass_tint()[k] as f64;
            color[k] += value;
            row_color[y as usize][k] += value / photo.width() as f64;
        }
        covered += 1;
    }
    let albedo = color.map(|c| (c / covered.max(1) as f64) as f32);
    let max_height = HEIGHT * (SCALE_MIN + SCALE_SPAN) * vegetation.grass_scale();
    let bin_height = max_height as f64 / 4.0;
    let mut area = [0.0f64; 4];
    let mut shading_mass = [[0.0f64; 4]; 4];
    for k in 0..PROFILE_SCALES {
        let scale = (SCALE_MIN as f64 + SCALE_SPAN as f64 * (k as f64 + 0.5) / PROFILE_SCALES as f64) * vegetation.grass_scale() as f64;
        let width = WIDTH as f64 * scale;
        let height = HEIGHT as f64 * scale;
        for (row, &fraction) in rows.iter().enumerate() {
            if fraction == 0.0 { continue; }
            // Texture top is the tuft tip, bottom its root. Near roots are
            // inset 2.5 cm, so that buried area contributes no far density.
            let y0 = ROOT_OFFSET as f64 + height * (1.0 - (row + 1) as f64 / rows.len() as f64);
            let y1 = ROOT_OFFSET as f64 + height * (1.0 - row as f64 / rows.len() as f64);
            for (bin, mass) in area.iter_mut().enumerate() {
                let lower = y0.max(bin as f64 * bin_height);
                let upper = y1.min((bin + 1) as f64 * bin_height);
                let overlap = (upper - lower).max(0.0);
                // Keep the extinction accumulation unchanged. Shading carries
                // separate area moments and cannot change optical density.
                *mass += width * fraction * overlap / PROFILE_SCALES as f64;
                if overlap == 0.0 { continue; }
                let weight = width * fraction * overlap / PROFILE_SCALES as f64;
                // Near shaders use (world_y-root_y)/(HEIGHT*instance_scale),
                // not world height divided by the tallest possible plant.
                // Its linear mean is exact over this clipped source row.
                let source_height = ((lower + upper) * 0.5 - ROOT_OFFSET as f64) / height;
                for channel in 0..3 {
                    shading_mass[bin][channel] += weight * row_color[row][channel] / fraction
                        * (0.8 + 0.2 * source_height);
                }
                shading_mass[bin][3] += weight * source_height;
            }
        }
    }
    // Three upright cards, uniform world yaw: mean |normal dot ray| is
    // (2/pi)*|ray.xz|. The renderer supplies only the latter directional term.
    // Units: tufts/m2 * projected m2/tuft / bin metres = extinction/metre.
    let density = 1.0 / (CELL as f64 * CELL as f64);
    let factor = density * 3.0 * std::f64::consts::FRAC_2_PI / bin_height;
    let shading = std::array::from_fn(|bin| {
        if area[bin] == 0.0 { return [0.0; 4]; }
        shading_mass[bin].map(|value| (value / area[bin]) as f32)
    });
    Ok(PhotoProfile { extinction: area.map(|a| (a * factor) as f32), max_height, albedo, shading })
}

/// Bake once with a stage's native chunks; the renderer owns upload/lifetime.
/// Missing chunks have zero coverage and a finite -1e9 missing-height sentinel.
/// This distinguishes holes from actual flat terrain at world height zero. Road
/// distances are retained so the GPU can clip every triangle segment against
/// road(t)>=0.9 instead of spilling averaged boundary-cell grass on the road.
pub fn bake(chunks: &[Chunk], photo: &image::RgbaImage) -> Result<(FarGrassField, String), String> {
    bake_with_vegetation(chunks, photo, Default::default())
}

pub fn bake_with_vegetation(chunks: &[Chunk], photo: &image::RgbaImage, vegetation: sideways::vegetation::Vegetation) -> Result<(FarGrassField, String), String> {
    let started = Instant::now();
    let first = chunks.first().ok_or("far grass needs native terrain chunks")?;
    let mut lo = first.cell;
    let mut hi = first.cell;
    let mut seen = HashSet::with_capacity(chunks.len());
    for chunk in chunks {
        if !seen.insert(chunk.cell) { return Err("far grass has duplicate native chunks".into()); }
        if chunk.heights.len() != N * N || chunk.road_d.len() != N * N
            || chunk.heights.iter().chain(&chunk.road_d).any(|v| !v.is_finite())
            || chunk.x0 != chunk.cell.0 as f64 * CHUNK || chunk.z0 != chunk.cell.1 as f64 * CHUNK {
            return Err("far grass native grid is malformed or non-finite".into());
        }
        lo.0 = lo.0.min(chunk.cell.0); lo.1 = lo.1.min(chunk.cell.1);
        hi.0 = hi.0.max(chunk.cell.0); hi.1 = hi.1.max(chunk.cell.1);
    }
    let dimension = |minimum: i32, maximum: i32| -> Result<usize, String> {
        let count = (maximum as i64 - minimum as i64 + 1) as u64;
        let points = count.checked_mul((N - 1) as u64).and_then(|n| n.checked_add(1)).ok_or("far grass dimensions overflow")?;
        usize::try_from(points).map_err(|_| "far grass dimensions overflow".into())
    };
    let width = dimension(lo.0, hi.0)?;
    let height = dimension(lo.1, hi.1)?;
    let points = width.checked_mul(height).ok_or("far grass point count overflow")?;
    if points > MAX_POINTS { return Err(format!("far grass grid requires {points} points; explicit cap is {MAX_POINTS}")); }
    let cells = (width - 1) * (height - 1);
    let origin = [(lo.0 as f64 * CHUNK) as f32, (lo.1 as f64 * CHUNK) as f32];
    let end = [origin[0] + (width - 1) as f32 * GRID as f32,
        origin[1] + (height - 1) as f32 * GRID as f32];
    if origin.iter().chain(&end).any(|v| !v.is_finite() || v.abs() >= 10_000_000.0) {
        return Err("far grass grid exceeds the live placement coordinate range".into());
    }
    let photo = photo_profile_with_vegetation(photo, vegetation)?;
    let max_height = photo.max_height;
    let mut field = FarGrassField { origin, cell: GRID as f32, width: width as u32, height: height as u32,
        heights: vec![-1e9; points], road: vec![-1.0; points], coverage: vec![0; cells],
        profile: photo.extinction, max_height, albedo: photo.albedo, shading_profile: photo.shading };
    let ground = Ground::new(chunks);
    let mut written = vec![false; points];
    let mut native_cells = 0usize;
    let mut covered_cells = 0usize;
    let mut expected_tufts = 0.0f64;
    for chunk in chunks {
        let ox = (chunk.cell.0 as i64 - lo.0 as i64) as usize * (N - 1);
        let oz = (chunk.cell.1 as i64 - lo.1 as i64) as usize * (N - 1);
        for z in 0..N { for x in 0..N {
            let source = z * N + x;
            let target = (oz + z) * width + ox + x;
            if written[target] && ((field.heights[target] - chunk.heights[source]).abs() > 1e-4
                || (field.road[target] - chunk.road_d[source]).abs() > 1e-4) {
                return Err("far grass adjacent native chunk edges disagree".into());
            }
            field.heights[target] = chunk.heights[source];
            field.road[target] = chunk.road_d[source];
            written[target] = true;
        }}
        for z in 0..N - 1 { for x in 0..N - 1 {
            native_cells += 1;
            let mut coverage = 0.0;
            // Fixed stratification estimates both spatial habitat and size-
            // dependent road clearance. It is not camera-dependent thinning.
            for (sx, sz, scale_bin) in [(0.25, 0.25, 0), (0.75, 0.25, 2), (0.25, 0.75, 3), (0.75, 0.75, 1)] {
                let p = Vec2::new(chunk.x0 as f32 + (x as f32 + sx) * GRID as f32,
                    chunk.z0 as f32 + (z as f32 + sz) * GRID as f32);
                let scale = SCALE_MIN + SCALE_SPAN * (scale_bin as f32 + 0.5) / 4.0;
                coverage += ground.expected_coverage(p, scale) * 0.25;
            }
            let density = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            let target = (oz + z) * (width - 1) + ox + x;
            field.coverage[target] = density;
            covered_cells += usize::from(density != 0);
            expected_tufts += coverage as f64 * GRID * GRID / (CELL as f64 * CELL as f64);
        }}
    }
    let bytes = field.heights.len() * 4 + field.road.len() * 4 + field.coverage.len();
    let summary = format!("far grass volume: {covered_cells}/{native_cells} native cells, {width}x{height} grid points at {GRID} m, 4 height bins up to {max_height:.2} m, {bytes} CPU bytes, {:.0} expected tufts, {:.1} ms bake; 4 stratified expected habitat/scale samples per cell (not exact individual tuft coverage), source color/plant-height moments per bin, same CC0 photo/roots/road field, no new near instances",
        expected_tufts, started.elapsed().as_secs_f64() * 1000.0);
    Ok((field, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(cell: (i32, i32), road: f32) -> Chunk {
        Chunk { cell, x0: cell.0 as f64 * CHUNK, z0: cell.1 as f64 * CHUNK,
            heights: vec![0.0; N*N], road_d: vec![road; N*N], trees:vec![], rocks:vec![], grass:vec![], ferns:vec![], stumps:vec![] }
    }
    fn photo(alpha: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(2, 4, image::Rgba([128, 200, 60, alpha]))
    }
    #[test]
    fn seasonal_profile_matches_scaled_cards_and_keeps_native_habitat() {
        use sideways::vegetation::{Vegetation,Season};
        let v=Vegetation::new(19,0.45,Season::Winter).unwrap();
        let p=photo(255);let profile=photo_profile_with_vegetation(&p,v).unwrap();
        assert_eq!(profile.max_height,HEIGHT*(SCALE_MIN+SCALE_SPAN)*v.grass_scale());
        let expected=(0..PROFILE_SCALES).map(|i| {
            let s=(SCALE_MIN as f64+SCALE_SPAN as f64*(i as f64+0.5)/PROFILE_SCALES as f64)*v.grass_scale() as f64;
            WIDTH as f64*s*(HEIGHT as f64*s+ROOT_OFFSET as f64).max(0.0)
        }).sum::<f64>()/PROFILE_SCALES as f64*3.0*std::f64::consts::FRAC_2_PI/(CELL as f64*CELL as f64);
        let actual=profile.extinction.iter().sum::<f32>() as f64*profile.max_height as f64/4.0;
        assert!((actual-expected).abs()<1e-5);
        for c in 0..3 {assert!((profile.albedo[c] as f64-linear(p.get_pixel(0,0)[c])*v.grass_tint()[c] as f64).abs()<1e-7);}
        let chunks=[chunk((0,0),5.0)];
        let (a,_)=bake(&chunks,&p).unwrap();let (b,_)=bake_with_vegetation(&chunks,&p,v).unwrap();
        assert_eq!(a.coverage,b.coverage);assert_eq!(a.heights,b.heights);assert_eq!(a.road,b.road);
        assert!(b.max_height<a.max_height);
    }
    #[test]
    fn profile_preserves_card_area_height_and_linear_color() {
        let source = photo_profile(&photo(255)).unwrap();
        let integral = source.extinction.iter().sum::<f32>() as f64 * source.max_height as f64 / 4.0;
        let expected_area = (0..PROFILE_SCALES).map(|k| {
            let s = SCALE_MIN as f64 + SCALE_SPAN as f64 * (k as f64 + 0.5) / PROFILE_SCALES as f64;
            WIDTH as f64 * s * (HEIGHT as f64 * s + ROOT_OFFSET as f64)
        }).sum::<f64>() / PROFILE_SCALES as f64;
        let expected = expected_area * 3.0 * std::f64::consts::FRAC_2_PI / (CELL as f64 * CELL as f64);
        assert!((integral - expected).abs() < 1e-5);
        assert!((source.max_height - 0.65).abs() < 1e-6);
        assert!((source.albedo[0] as f64 - linear(128)).abs() < 1e-7);
        assert_eq!(photo_profile(&photo(127)).unwrap().extinction, [0.0;4]);
        assert_eq!(photo_profile(&photo(127)).unwrap().shading, [[0.0;4];4]);
        assert!(photo_profile(&photo(128)).unwrap().extinction.iter().any(|v| *v > 0.0));
        for bin in source.shading {
            assert!((bin[0] as f64 - linear(128) * (0.8 + 0.2 * bin[3] as f64)).abs() < 1e-7);
        }
    }
    #[test]
    fn material_moments_match_original_scaled_tuft_integrals() {
        // A red upper half and blue lower half give independent closed-form
        // color integrals. Scaling/inset mix source rows into different world
        // height bins, but their total source-material moment must be conserved.
        let mut image = image::RgbaImage::from_pixel(2, 8, image::Rgba([0, 0, 255, 255]));
        for y in 0..4 { for x in 0..2 { image.put_pixel(x, y, image::Rgba([255, 0, 0, 255])); }}
        let source = photo_profile(&image).unwrap();
        let white = photo_profile(&image::RgbaImage::from_pixel(2, 8, image::Rgba([255;4]))).unwrap();
        assert_eq!(source.extinction, white.extinction, "RGB must never alter density or alpha");
        let bin_height = source.max_height as f64 / 4.0;
        let actual: [f64;4] = std::array::from_fn(|channel| (0..4).map(|bin|
            source.extinction[bin] as f64 * bin_height * source.shading[bin][channel] as f64).sum());
        let mut expected = [0.0f64;4];
        let density = 3.0 * std::f64::consts::FRAC_2_PI / (CELL as f64 * CELL as f64);
        let primitive = |u:f64| 0.8*u + 0.1*u*u;
        for k in 0..PROFILE_SCALES {
            let scale = SCALE_MIN as f64 + SCALE_SPAN as f64 * (k as f64 + 0.5) / PROFILE_SCALES as f64;
            let height = HEIGHT as f64 * scale;
            let root = -ROOT_OFFSET as f64 / height;
            let factor = WIDTH as f64 * scale * height * density / PROFILE_SCALES as f64;
            expected[0] += factor * (primitive(1.0) - primitive(0.5));
            expected[2] += factor * (primitive(0.5) - primitive(root));
            expected[3] += factor * (1.0 - root*root) * 0.5;
        }
        for channel in 0..4 { assert!((actual[channel] - expected[channel]).abs() < 2e-6,
            "channel {channel}: {} != {}", actual[channel], expected[channel]); }
        assert!(source.shading[0][2] > source.shading[0][0]);
        assert!(source.shading[3][0] > source.shading[3][2]);
        // Material source height also rises per bin; it is not simply the
        // bin centre divided by the tallest allowed plant's world height.
        assert!(source.shading.windows(2).all(|b| b[1][3] > b[0][3]));
        assert!((source.shading[1][3] - 0.375).abs() > 0.05);
    }
    #[test]
    fn native_samples_and_missing_chunks_are_preserved() {
        let mut a = chunk((-1, 0), 5.0);
        a.heights[1] = 2.0; a.heights[N] = 4.0; a.heights[N+1] = 10.0;
        let source_row = a.heights[..N].to_vec();
        let b = chunk((1, 0), 5.0);
        let (field, _) = bake(&[a, b], &photo(255)).unwrap();
        assert_eq!(field.origin, [-64.0, 0.0]);
        assert_eq!(field.width, (3*(N-1)+1) as u32);
        assert_eq!(&field.heights[..N], &source_row);
        assert_eq!(field.heights[field.width as usize], 4.0);
        for z in 0..N-1 { for x in N-1..2*(N-1) {
            assert_eq!(field.coverage[z*(field.width as usize-1)+x], 0);
        }}
        assert_eq!(field.heights[(N/2)*field.width as usize+N+(N-1)/2],-1e9,
            "missing native chunks cannot become a sunlight-blocking floor at Y=0");
        assert_eq!(field.heights[(N/2)*field.width as usize+2*(N-1)+N/2],0.0,
            "real zero-height terrain remains valid");
        assert!(field.heights.iter().chain(&field.road).all(|v| v.is_finite()));
    }
    #[test]
    fn field_is_deterministic_and_obeys_live_road_and_slope_rejection() {
        let c = chunk((0, 0), 5.0);
        let a = bake(std::slice::from_ref(&c), &photo(255)).unwrap().0;
        let b = bake(&[c], &photo(255)).unwrap().0;
        assert_eq!(a.coverage, b.coverage); assert_eq!(a.profile, b.profile);
        assert_eq!(a.coverage[16*(N-1)+16], 255);
        let road = bake(&[chunk((0,0),0.2)], &photo(255)).unwrap().0;
        assert!(road.coverage.iter().all(|c| *c == 0));
        let mut steep = chunk((0,0),5.0);
        for z in 0..N { for x in 0..N { steep.heights[z*N+x] = x as f32 * GRID as f32 * 2.0; }}
        assert!(bake(&[steep], &photo(255)).unwrap().0.coverage.iter().all(|c| *c == 0));
    }
    #[test]
    fn invalid_or_unbounded_source_fails_explicitly() {
        assert!(bake(&[], &photo(255)).is_err());
        assert!(bake(&[chunk((0,0),5.0),chunk((0,0),5.0)], &photo(255)).is_err());
        assert!(bake(&[chunk((0,0),5.0),chunk((100_000,0),5.0)], &photo(255)).is_err());
        let mut invalid = chunk((0,0),5.0); invalid.heights[0] = f32::NAN;
        assert!(bake(&[invalid], &photo(255)).is_err());
    }
}
