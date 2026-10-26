//! Existing image processing, unchanged; shared by cache misses and CPU parity tests.
use super::cutout_mips;
use crate::texture_cache::Settings;
pub(crate) fn build(src:Option<&image::RgbaImage>, settings:Settings, enabled:bool, layer:usize)
    -> (Vec<image::RgbaImage>,crate::startup_profile::TextureLayerProfile) {
    let Settings{size,format,fill,preserve_alpha,linear_cutout_rgb}=settings;
    let mips=size.ilog2()+1;
        let start=enabled.then(std::time::Instant::now);
        let mut timing=crate::startup_profile::TextureLayerProfile{layer,source_size:src.map_or([0,0],|s|[s.width(),s.height()]),..Default::default()};
        let Some(src) = src else {
            let out:Vec<_>=(0..mips).map(|m| image::RgbaImage::from_pixel((size >> m).max(1), (size >> m).max(1), image::Rgba(fill))).collect();
            timing.base_resize_ms=crate::startup_profile::elapsed(start);timing.total_cpu_ms=timing.base_resize_ms;
            timing.output_bytes=out.iter().map(|i|i.as_raw().len()as u64).sum();return (out,timing);
        };
        let mut img = if src.dimensions() == (size, size) { src.clone() } else { image::imageops::resize(src, size, size, image::imageops::FilterType::Triangle) };
        timing.base_resize_ms=crate::startup_profile::elapsed(start);
        let alpha_start=enabled.then(std::time::Instant::now);
        let cutout = preserve_alpha && img.pixels().any(|p| p[3] < 250);
        let target = coverage(&img, 1.0);
        timing.alpha_analysis_ms=crate::startup_profile::elapsed(alpha_start);
        let copy_start=enabled.then(std::time::Instant::now);
        let mut out = vec![img.clone()];
        timing.chain_copy_ms=crate::startup_profile::elapsed(copy_start);
        for mip in 1..mips {
            let resize_start=enabled.then(std::time::Instant::now);
            let s = (size >> mip).max(1);
            img = image::imageops::resize(&img, s, s, image::imageops::FilterType::Triangle);
            timing.mip_resize_ms+=crate::startup_profile::elapsed(resize_start);
            let coverage_start=enabled.then(std::time::Instant::now);
            if cutout {
                preserve_coverage(&mut img, target);
            }
            timing.coverage_ms+=crate::startup_profile::elapsed(coverage_start);
            let copy_start=enabled.then(std::time::Instant::now);
            out.push(img.clone());
            timing.chain_copy_ms+=crate::startup_profile::elapsed(copy_start);
        }
        let rgb_start=enabled.then(std::time::Instant::now);
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
