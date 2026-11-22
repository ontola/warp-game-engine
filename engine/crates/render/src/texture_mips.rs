//! Existing image processing, unchanged; shared by cache misses and CPU parity tests.
use super::cutout_mips;
use crate::texture_cache::Settings;
pub(crate) fn build(src:Option<&image::RgbaImage>, settings:Settings, enabled:bool, layer:usize)
    -> (Vec<image::RgbaImage>,crate::startup_profile::TextureLayerProfile) {
    let Settings{size,format,fill,preserve_alpha,linear_cutout_rgb}=settings;
    let mips=size.ilog2()+1;
        let start=enabled.then(web_time::Instant::now);
        let mut timing=crate::startup_profile::TextureLayerProfile{layer,source_size:src.map_or([0,0],|s|[s.width(),s.height()]),..Default::default()};
        let Some(src) = src else {
            let out:Vec<_>=(0..mips).map(|m| image::RgbaImage::from_pixel((size >> m).max(1), (size >> m).max(1), image::Rgba(fill))).collect();
            timing.base_resize_ms=crate::startup_profile::elapsed(start);timing.total_cpu_ms=timing.base_resize_ms;
            timing.output_bytes=out.iter().map(|i|i.as_raw().len()as u64).sum();return (out,timing);
        };
        let mut img = if src.dimensions() == (size, size) { src.clone() } else { resize(src, size) };
        timing.base_resize_ms=crate::startup_profile::elapsed(start);
        let alpha_start=enabled.then(web_time::Instant::now);
        let cutout = preserve_alpha && img.pixels().any(|p| p[3] < 250);
        let target = coverage(&img, 1.0);
        timing.alpha_analysis_ms=crate::startup_profile::elapsed(alpha_start);
        let copy_start=enabled.then(web_time::Instant::now);
        let mut out = vec![img.clone()];
        timing.chain_copy_ms=crate::startup_profile::elapsed(copy_start);
        for mip in 1..mips {
            let resize_start=enabled.then(web_time::Instant::now);
            let s = (size >> mip).max(1);
            img = resize(&img, s);
            timing.mip_resize_ms+=crate::startup_profile::elapsed(resize_start);
            let coverage_start=enabled.then(web_time::Instant::now);
            if cutout {
                preserve_coverage(&mut img, target);
            }
            timing.coverage_ms+=crate::startup_profile::elapsed(coverage_start);
            let copy_start=enabled.then(web_time::Instant::now);
            out.push(img.clone());
            timing.chain_copy_ms+=crate::startup_profile::elapsed(copy_start);
        }
        let rgb_start=enabled.then(web_time::Instant::now);
        if linear_cutout_rgb && cutout && format == wgpu::TextureFormat::Rgba8UnormSrgb {
            // Keep the alpha chain byte-identical. RGB must start at the source,
            // before even the base resize can mix transparent black into leaves.
            cutout_mips::correct_rgb(src, &mut out);
        }
        timing.linear_rgb_ms=crate::startup_profile::elapsed(rgb_start);
        timing.total_cpu_ms=crate::startup_profile::elapsed(start);
        timing.output_bytes=out.iter().map(|i|i.as_raw().len()as u64).sum();
        (out,timing)
}

/// Square resize with a triangle filter. A browser bakes on its only thread
/// while the page waits, so there a power-of-two step is a plain 2x2
/// average (down) or a linear blend of neighbours (up), several times
/// faster than the general filter and nearly the same image.
fn resize(img: &image::RgbaImage, size: u32) -> image::RgbaImage {
    #[cfg(target_arch = "wasm32")]
    {
        let (w, h) = img.dimensions();
        if w == h && w.is_power_of_two() && size.is_power_of_two() {
            let mut out = std::borrow::Cow::Borrowed(img);
            while out.width() > size { out = std::borrow::Cow::Owned(half(&out)); }
            while out.width() < size { out = std::borrow::Cow::Owned(double(&out)); }
            return out.into_owned();
        }
    }
    image::imageops::resize(img, size, size, image::imageops::FilterType::Triangle)
}

#[cfg(any(target_arch = "wasm32", test))]
fn half(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let (ow, oh) = ((w / 2).max(1), (h / 2).max(1));
    let src = img.as_raw();
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh as usize {
        let (y0, y1) = ((2 * y).min(h as usize - 1), (2 * y + 1).min(h as usize - 1));
        for x in 0..ow as usize {
            let (x0, x1) = ((2 * x).min(w as usize - 1), (2 * x + 1).min(w as usize - 1));
            for c in 0..4 {
                let at = |xx: usize, yy: usize| src[(yy * w as usize + xx) * 4 + c] as u32;
                out[(y * ow as usize + x) * 4 + c] = ((at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1) + 2) / 4) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(ow, oh, out).unwrap()
}

/// Twice the size; each new texel 3:1 between its nearest source texels
/// (a bilinear upscale, edges clamped).
#[cfg(any(target_arch = "wasm32", test))]
fn double(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (ow, oh) = (w * 2, h * 2);
    let src = img.as_raw();
    let mut out = vec![0u8; ow * oh * 4];
    for y in 0..oh {
        let (ya, yb) = (y / 2, if y % 2 == 0 { (y / 2).saturating_sub(1) } else { (y / 2 + 1).min(h - 1) });
        for x in 0..ow {
            let (xa, xb) = (x / 2, if x % 2 == 0 { (x / 2).saturating_sub(1) } else { (x / 2 + 1).min(w - 1) });
            for c in 0..4 {
                let at = |xx: usize, yy: usize| src[(yy * w + xx) * 4 + c] as u32;
                let v = 9 * at(xa, ya) + 3 * at(xb, ya) + 3 * at(xa, yb) + at(xb, yb);
                out[(y * ow + x) * 4 + c] = ((v + 8) / 16) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(ow as u32, oh as u32, out).unwrap()
}

/// Fraction of texels that pass a 0.5 alpha test with alpha scaled by `k`.
fn coverage(img: &image::RgbaImage, k: f32) -> f32 {
    let n = img.pixels().filter(|p| p[3] as f32 * k >= 127.5).count();
    n as f32 / (img.width() * img.height()).max(1) as f32
}

/// Scale a mip's alpha so as many texels pass the alpha test as at full
/// size: foliage keeps its density in the distance instead of thinning out.
fn preserve_coverage(img: &mut image::RgbaImage, target: f32) {
    let (mut lo, mut hi) = (0.25f32, 8.0f32);
    for _ in 0..12 {
        let mid = (lo + hi) * 0.5;
        if coverage(img, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let k = (lo + hi) * 0.5;
    for p in img.pixels_mut() {
        p[3] = (p[3] as f32 * k).min(255.0) as u8;
    }
}

#[cfg(test)]
mod fast_resize_tests {
    use super::*;
    #[test]
    fn half_and_double_keep_a_flat_colour_and_the_average() {
        let flat = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255]));
        assert_eq!(half(&flat).get_pixel(1, 1), &image::Rgba([10, 200, 30, 255]));
        assert_eq!(double(&flat).dimensions(), (16, 16));
        assert_eq!(double(&flat).get_pixel(5, 9), &image::Rgba([10, 200, 30, 255]));
        let mut checker = image::RgbaImage::new(2, 2);
        for (x, y, p) in checker.enumerate_pixels_mut() { *p = image::Rgba([if (x + y) % 2 == 0 { 0 } else { 200 }, 0, 0, 255]); }
        assert_eq!(half(&checker).get_pixel(0, 0)[0], 100);
    }
}
