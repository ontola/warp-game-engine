//! Colour filtering for alpha-tested sRGB textures. Alpha stays owned by the
//! existing coverage-preserving chain; colour never uses its boosted alpha.
use image::{Rgba, Rgba32FImage, RgbaImage, imageops::{resize, FilterType}};

fn linear(s: u8) -> f32 {
    let s = s as f32 / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

fn srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// Replace only RGB, including the initial resize, from unmodified source
/// coverage. Filtering premultiplied linear RGB excludes invisible background
/// colours and preserves radiance instead of averaging gamma-encoded bytes.
pub(super) fn correct_rgb(source: &RgbaImage, chain: &mut [RgbaImage]) {
    if chain.is_empty() || source.width() == 0 || source.height() == 0 { return; }
    let lut: [f32; 256] = std::array::from_fn(|i| linear(i as u8));
    let mut weighted = Rgba32FImage::from_fn(source.width(), source.height(), |x, y| {
        let p = source.get_pixel(x, y);
        let a = p[3] as f32 / 255.0;
        Rgba([lut[p[0] as usize] * a, lut[p[1] as usize] * a, lut[p[2] as usize] * a, a])
    });
    for output in chain {
        if weighted.dimensions() != output.dimensions() {
            weighted = resize(&weighted, output.width(), output.height(), FilterType::Triangle);
        }
        // Unpremultiply before sRGB encoding. The hardware later decodes RGB
        // before bilinear/trilinear filtering. Extend valid colour into alpha-0
        // texels so those spatial filters cannot introduce a black fringe.
        let mut valid = Vec::with_capacity(output.as_raw().len() / 4);
        for (dst, src) in output.pixels_mut().zip(weighted.pixels()) {
            let covered = src[3] > 0.0;
            valid.push(covered);
            for c in 0..3 { dst[c] = if covered { srgb(src[c] / src[3]) } else { 0 }; }
        }
        extend_edge_colour(output, &mut valid);
    }
}

/// Carry the nearest covered RGB into the transparent texels of one atlas chart.
/// Alpha and covered texels are preserved. Apply before packing the chart so a
/// neighbouring material cannot colour its edges during ordinary mip filtering.
pub fn pad_chart_colour(chart: &mut RgbaImage) {
    let mut valid: Vec<_> = chart.pixels().map(|p| p[3] > 0).collect();
    extend_edge_colour(chart, &mut valid);
}

/// Deterministic nearest Manhattan-neighbour extension, bounded O(texels).
/// Alpha and the weighted source for subsequent mips are never modified.
fn extend_edge_colour(image: &mut RgbaImage, valid: &mut [bool]) {
    let width = image.width() as usize;
    let count = valid.len();
    // Only boundary texels enter the initial queue. Fully opaque images need no
    // queue allocation; a cutout's frontier visits each transparent texel once.
    let mut queue = Vec::<u32>::new();
    for i in 0..count {
        if valid[i] && ((i % width > 0 && !valid[i - 1])
            || (i % width + 1 < width && !valid[i + 1])
            || (i >= width && !valid[i - width])
            || (i + width < count && !valid[i + width])) {
            queue.push(i as u32);
        }
    }
    let bytes = image.as_mut();
    let mut head = 0;
    while head < queue.len() {
        let i = queue[head] as usize;
        head += 1;
        let neighbours = [
            if i % width > 0 { Some(i - 1) } else { None },
            if i % width + 1 < width { Some(i + 1) } else { None },
            i.checked_sub(width),
            if i + width < count { Some(i + width) } else { None },
        ];
        for next in neighbours.into_iter().flatten() {
            if !valid[next] {
                for c in 0..3 { bytes[next * 4 + c] = bytes[i * 4 + c]; }
                valid[next] = true;
                queue.push(next as u32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_chart_extends_the_nearest_covered_colour_without_changing_coverage() {
        let mut chart = RgbaImage::from_fn(4, 1, |x, _| Rgba(match x {
            0 => [32, 95, 49, 91],
            3 => [118, 69, 44, 255],
            _ => [0; 4],
        }));
        pad_chart_colour(&mut chart);
        assert_eq!(chart.get_pixel(0, 0).0, [32, 95, 49, 91]);
        assert_eq!(chart.get_pixel(1, 0).0, [32, 95, 49, 0]);
        assert_eq!(chart.get_pixel(2, 0).0, [118, 69, 44, 0]);
        assert_eq!(chart.get_pixel(3, 0).0, [118, 69, 44, 255]);
    }

    #[test]
    fn opaque_average_preserves_linear_light_energy() {
        let source = RgbaImage::from_fn(2, 1, |x, _| Rgba(if x == 0 { [0, 0, 0, 255] } else { [255; 4] }));
        let mut chain = vec![RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 231]))];
        correct_rgb(&source, &mut chain);
        let p = chain[0].get_pixel(0, 0);
        assert!((linear(p[0]) - 0.5).abs() < 0.005, "linear mean: {:?}", p);
        assert_eq!(p[3], 231);
    }

    #[test]
    fn transparent_rgb_cannot_contaminate_base_or_later_mips() {
        let a = RgbaImage::from_fn(3, 2, |x, _| Rgba(if x == 1 { [70, 180, 35, 255] } else { [0; 4] }));
        let b = RgbaImage::from_fn(3, 2, |x, y| {
            if x == 1 { *a.get_pixel(x, y) } else { Rgba([255, 3, 240, 0]) }
        });
        let original = vec![RgbaImage::new(8, 8), RgbaImage::new(4, 4), RgbaImage::new(2, 2), RgbaImage::new(1, 1)];
        let mut ca = original.clone();
        let mut cb = original;
        correct_rgb(&a, &mut ca);
        correct_rgb(&b, &mut cb);
        assert_eq!(ca, cb);
        for img in ca {
            for p in img.pixels() { assert_eq!(p.0, [70, 180, 35, 0]); }
        }
    }

    #[test]
    fn all_existing_alpha_and_coverage_bytes_stay_exact() {
        let source = RgbaImage::from_fn(7, 5, |x, y| Rgba([60, 120, 30, ((x * 31 + y * 19) % 256) as u8]));
        let mut chain: Vec<_> = [16, 8, 4, 2, 1].into_iter().map(|size| {
            RgbaImage::from_fn(size, size, |x, y| Rgba([1, 2, 3, ((x * 71 + y * 33 + size * 9) % 256) as u8]))
        }).collect();
        let alpha: Vec<Vec<u8>> = chain.iter().map(|img| img.pixels().map(|p| p[3]).collect()).collect();
        correct_rgb(&source, &mut chain);
        for (img, expected) in chain.iter().zip(alpha) {
            assert_eq!(img.pixels().map(|p| p[3]).collect::<Vec<_>>(), expected);
        }
    }

    #[test]
    fn partial_alpha_weights_colour_without_coverage_boost_feedback() {
        let source = RgbaImage::from_fn(2, 1, |x, _| Rgba(if x == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 85] }));
        let mut chain = vec![RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]))];
        correct_rgb(&source, &mut chain);
        let p = chain[0].get_pixel(0, 0);
        assert!((linear(p[0]) - 0.75).abs() < 0.005);
        assert!((linear(p[2]) - 0.25).abs() < 0.005);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn fully_transparent_source_is_finite_black_and_preserves_alpha() {
        let source = RgbaImage::from_pixel(2, 2, Rgba([255, 1, 127, 0]));
        let mut chain = vec![RgbaImage::from_pixel(4, 4, Rgba([1, 2, 3, 0])), RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 0]))];
        correct_rgb(&source, &mut chain);
        assert!(chain.iter().flat_map(|image| image.pixels()).all(|p| p.0 == [0; 4]));
    }
}
