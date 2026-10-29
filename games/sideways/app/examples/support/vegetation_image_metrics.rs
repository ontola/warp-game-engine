//! CPU comparison of captured, tonemapped sRGB images. Display-linear errors
//! are not HDR radiance or a proof of optical equivalence. Coverage requires
//! genuine renderer masks; colour thresholds are deliberately not substituted.
use image::{Rgba, RgbaImage};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

fn linear(byte: u8) -> f64 {
    let x = f64::from(byte) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

pub fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty()
        || !fraction.is_finite()
        || !(0.0..=1.0).contains(&fraction)
        || values.iter().any(|v| !v.is_finite())
    {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = fraction * (sorted.len() - 1) as f64;
    let lo = position.floor() as usize;
    let hi = position.ceil() as usize;
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (position - lo as f64))
}

pub fn summarize(values: &[f64]) -> Value {
    json!({"samples":values.len(),"min":percentile(values,0.0),
        "median":percentile(values,0.5),"p95":percentile(values,0.95),"max":percentile(values,1.0)})
}

fn validate(a: &RgbaImage, b: &RgbaImage, roi: Rect) -> Result<(), String> {
    if a.dimensions() != b.dimensions() || a.width() == 0 || a.height() == 0 {
        return Err("comparison requires matching nonempty images".into());
    }
    if roi.w == 0
        || roi.h == 0
        || roi.x.checked_add(roi.w).is_none_or(|x| x > a.width())
        || roi.y.checked_add(roi.h).is_none_or(|y| y > a.height())
    {
        return Err("comparison ROI must be a nonempty in-bounds rectangle".into());
    }
    Ok(())
}

pub fn compare(a: &RgbaImage, b: &RgbaImage, roi: Rect) -> Result<Value, String> {
    validate(a, b, roi)?;
    let mut srgb_abs = 0.0;
    let mut linear_abs = 0.0;
    let mut linear_square = 0.0;
    let mut luma_bias = 0.0;
    let mut luma_reference = 0.0;
    let mut luma_candidate = 0.0;
    let mut different = 0u64;
    let mut pixel_errors = Vec::with_capacity((roi.w * roi.h) as usize);
    for y in roi.y..roi.y + roi.h {
        for x in roi.x..roi.x + roi.w {
            let source = a.get_pixel(x, y);
            let candidate = b.get_pixel(x, y);
            different += u64::from(source.0[..3] != candidate.0[..3]);
            let mut pixel_abs = 0.0;
            for (channel, weight) in [0.2126, 0.7152, 0.0722].into_iter().enumerate() {
                srgb_abs += f64::from(source[channel].abs_diff(candidate[channel]));
                let ar = linear(source[channel]);
                let br = linear(candidate[channel]);
                let delta = br - ar;
                linear_abs += delta.abs();
                linear_square += delta * delta;
                pixel_abs += delta.abs();
                luma_reference += ar * weight;
                luma_candidate += br * weight;
                luma_bias += delta * weight;
            }
            pixel_errors.push(pixel_abs / 3.0);
        }
    }
    let pixels = u64::from(roi.w) * u64::from(roi.h);
    let count = pixels as f64;
    Ok(
        json!({"roi":[roi.x,roi.y,roi.w,roi.h],"pixels":pixels,"different_rgb_pixels":different,
        "srgb_rgb_mae_255":srgb_abs/(count*3.0),"display_linear_rgb_mae":linear_abs/(count*3.0),
        "display_linear_rgb_rmse":(linear_square/(count*3.0)).sqrt(),
        "display_linear_pixel_rgb_mae_p95":percentile(&pixel_errors,0.95),
        "display_linear_luma_bias_candidate_minus_source":luma_bias/count,
        "display_linear_source_luma_mean":luma_reference/count,
        "display_linear_candidate_luma_mean":luma_candidate/count,
        "scope":"Tonemapped output RGB decoded with the sRGB transfer function. ROI includes background and occlusion; this is not a silhouette/opacity measurement or an automatic acceptance gate."}),
    )
}

/// Mask values must come from the actual renderer coverage/ownership path.
#[allow(dead_code)]
pub fn coverage(reference: &[bool], candidate: &[bool]) -> Result<Value, String> {
    if reference.is_empty() || reference.len() != candidate.len() {
        return Err("coverage requires matching nonempty renderer masks".into());
    }
    let (mut a, mut b, mut intersection, mut union) = (0u64, 0u64, 0u64, 0u64);
    for (&ar, &br) in reference.iter().zip(candidate) {
        a += u64::from(ar);
        b += u64::from(br);
        intersection += u64::from(ar && br);
        union += u64::from(ar || br);
    }
    Ok(
        json!({"source_pixels":a,"candidate_pixels":b,"intersection":intersection,"union":union,
        "iou":if union==0 {None}else{Some(intersection as f64/union as f64)},
        "coverage_ratio":if a==0 {None}else{Some(b as f64/a as f64)},
        "lost_source_pixels":a-intersection,"added_candidate_pixels":b-intersection,
        "scope":"Actual supplied binary renderer masks only; undefined ratios remain null."}),
    )
}

/// Native input opacity: opaque tree ownership unioned with the renderer's
/// foreground volume alpha. Shadows and TAA history are deliberately excluded.
pub fn opacity(reference: &[f32], candidate: &[f32]) -> Result<Value, String> {
    if reference.is_empty()
        || reference.len() != candidate.len()
        || reference
            .iter()
            .chain(candidate)
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
    {
        return Err("opacity requires matching finite unit-interval renderer layers".into());
    }
    let (mut a, mut b, mut intersection, mut union, mut absolute) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (&ar, &br) in reference.iter().zip(candidate) {
        let (ar, br) = (f64::from(ar), f64::from(br));
        a += ar;
        b += br;
        intersection += ar.min(br);
        union += ar.max(br);
        absolute += (br - ar).abs();
    }
    Ok(
        json!({"pixels":reference.len(),"source_coverage_integral":a,"candidate_coverage_integral":b,
        "coverage_ratio":if a==0.0 {None}else{Some(b/a)},
        "opacity_mae":absolute/reference.len() as f64,
        "opacity_l1_over_source_coverage":if a==0.0 {None}else{Some(absolute/a)},
        "fuzzy_iou":if union==0.0 {None}else{Some(intersection/union)},
        "lost_coverage_integral":a-intersection,"added_coverage_integral":b-intersection,
        "scope":"Native opaque tree visibility unioned with fractional foreground volume alpha. min/max fuzzy IoU; not an output-TAA silhouette or hidden geometry coverage."}),
    )
}

/// Decode the actual little-endian RGBA16F alpha channel without a new crate.
pub fn rgba16f_alpha(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.is_empty() || bytes.len() % 8 != 0 {
        return Err("invalid RGBA16F layer byte length".into());
    }
    bytes
        .chunks_exact(8)
        .map(|p| {
            let bits = u16::from_le_bytes([p[6], p[7]]);
            let exponent = (bits >> 10) & 31;
            let mantissa = bits & 1023;
            let value = match exponent {
                0 => f32::from(mantissa) * 2.0f32.powi(-24),
                31 => return Err("nonfinite RGBA16F alpha".into()),
                _ => (1.0 + f32::from(mantissa) / 1024.0) * 2.0f32.powi(i32::from(exponent) - 15),
            } * if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
            if !(0.0..=1.0).contains(&value) {
                return Err("RGBA16F alpha outside unit interval".into());
            }
            Ok(value)
        })
        .collect()
}

pub fn difference_image(a: &RgbaImage, b: &RgbaImage) -> Result<RgbaImage, String> {
    validate(
        a,
        b,
        Rect {
            x: 0,
            y: 0,
            w: a.width(),
            h: a.height(),
        },
    )?;
    let mut result = RgbaImage::new(a.width(), a.height());
    for (out, (ar, br)) in result.pixels_mut().zip(a.pixels().zip(b.pixels())) {
        // Explicit 4x absolute display RGB difference, never an altered candidate.
        *out = Rgba([
            ar[0].abs_diff(br[0]).saturating_mul(4),
            ar[1].abs_diff(br[1]).saturating_mul(4),
            ar[2].abs_diff(br[2]).saturating_mul(4),
            255,
        ]);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_linear_metric_and_luma_sign_have_known_values() {
        let a = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]));
        let b = RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255]));
        let value = compare(
            &a,
            &b,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
        )
        .unwrap();
        assert!((value["display_linear_rgb_mae"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-12);
        assert!(
            (value["display_linear_luma_bias_candidate_minus_source"]
                .as_f64()
                .unwrap()
                - 0.2126)
                .abs()
                < 1e-12
        );
        assert!((linear(128) - 0.21586050011389926).abs() < 1e-12);
    }
    #[test]
    fn roi_does_not_hide_plant_error_in_a_large_matching_background() {
        let a = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));
        let mut b = a.clone();
        b.put_pixel(50, 50, Rgba([255, 255, 255, 255]));
        let tight = compare(
            &a,
            &b,
            Rect {
                x: 50,
                y: 50,
                w: 1,
                h: 1,
            },
        )
        .unwrap();
        let full = compare(
            &a,
            &b,
            Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
        )
        .unwrap();
        assert_eq!(tight["display_linear_rgb_mae"], 1.0);
        assert_eq!(full["display_linear_rgb_mae"], 0.0001);
        assert!(
            compare(
                &a,
                &b,
                Rect {
                    x: 100,
                    y: 0,
                    w: 1,
                    h: 1
                }
            )
            .is_err()
        );
    }
    #[test]
    fn masks_detect_disappearance_and_leave_empty_ratios_undefined() {
        let missing = coverage(&[true, true, false], &[false, false, false]).unwrap();
        assert_eq!(missing["iou"], 0.0);
        assert_eq!(missing["coverage_ratio"], 0.0);
        assert_eq!(missing["lost_source_pixels"], 2);
        let empty = coverage(&[false], &[false]).unwrap();
        assert!(empty["iou"].is_null() && empty["coverage_ratio"].is_null());
    }
    #[test]
    fn timing_summary_counts_samples_and_uses_interpolated_percentiles() {
        assert_eq!(percentile(&[4.0, 1.0, 3.0, 2.0], 0.5), Some(2.5));
        assert!((percentile(&[1.0, 2.0, 3.0, 4.0], 0.95).unwrap() - 3.85).abs() < 1e-12);
        assert!(percentile(&[], 0.5).is_none());
        assert!(percentile(&[f64::NAN], 0.5).is_none());
    }
    #[test]
    fn fractional_opacity_measures_loss_without_binary_threshold_bias() {
        let value = opacity(&[1.0, 0.0], &[0.5, 0.25]).unwrap();
        assert_eq!(value["coverage_ratio"], 0.75);
        assert_eq!(value["fuzzy_iou"], 0.4);
        assert_eq!(value["lost_coverage_integral"], 0.5);
        assert_eq!(value["added_coverage_integral"], 0.25);
        assert!(opacity(&[1.0], &[f32::NAN]).is_err());
    }
    #[test]
    fn half_alpha_decodes_zero_subnormal_half_and_one_and_rejects_nonfinite() {
        let mut bytes = Vec::new();
        for bits in [0u16, 1, 0x3800, 0x3c00] {
            bytes.extend_from_slice(&[0; 6]);
            bytes.extend_from_slice(&bits.to_le_bytes());
        }
        assert_eq!(
            rgba16f_alpha(&bytes).unwrap(),
            vec![0.0, 2.0f32.powi(-24), 0.5, 1.0]
        );
        bytes[6..8].copy_from_slice(&0x7c00u16.to_le_bytes());
        assert!(rgba16f_alpha(&bytes).is_err());
    }
}
