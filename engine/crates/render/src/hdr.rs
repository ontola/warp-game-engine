//! High dynamic range output for HDR screens.
//!
//! The renderer always tonemaps to the 8-bit SDR picture that screenshots,
//! captures, live screens and SDR windows use. With a headroom above 1
//! ([`crate::Renderer::set_hdr_headroom`]) the tone pass also writes the same
//! picture linear into an `Rgba16Float` texture ([`crate::Renderer::hdr_output`]),
//! with 1.0 as SDR white: everything up to [`KNEE`] is the SDR picture
//! unchanged, and AgX's highlight shoulder above it is stretched up to the
//! display's headroom. The look (contrast, saturation, vignette, grain,
//! bloom) is the SDR look; only highlights gain range.
//!
//! A window shows it on an fp16 scRGB surface ([`configure_surface`]); the
//! presenter ([`crate::present::Presenter`]) scales SDR white to the OS's SDR
//! white level ([`HdrDisplay::white_scale`]) and composes the HUD at SDR white.

/// Display-linear SDR value (brightest channel) above which highlights are
/// stretched into the headroom. AgX shows an exposed 1.0 (a white surface in
/// the light) at about 0.59, so diffuse surfaces keep their SDR brightness.
pub const KNEE: f32 = 0.6;

/// The HDR value for a display-linear SDR value (0..1), as the tone pass
/// computes it (`expand_hdr` in `tonemap.wgsl`).
pub fn expand(sdr: [f32; 3], headroom: f32, knee: f32) -> [f32; 3] {
    let knee = knee.clamp(0.0, 0.95);
    let m = sdr[0].max(sdr[1]).max(sdr[2]);
    if headroom <= 1.0 || m <= knee { return sdr; }
    let t = (m.min(1.0) - knee) / (1.0 - knee);
    let gain = (m + (headroom - 1.0) * t * t) / m;
    sdr.map(|c| c * gain)
}

/// What a window's display can show, as the platform reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrDisplay {
    /// How much brighter than SDR white the display can go right now (1:
    /// no headroom). `None`: the platform does not say (Vulkan on Linux).
    pub headroom: Option<f32>,
    /// What SDR white is in surface units. On Windows scRGB 1.0 is 80 nits
    /// while the OS shows SDR white at its own level (the "SDR content
    /// brightness" slider): this is that level over 80. 1 elsewhere.
    pub white_scale: f32,
    /// The OS's SDR white and the display's peak, in nits, when known.
    pub sdr_white_nits: Option<f32>,
    pub peak_nits: Option<f32>,
}

impl Default for HdrDisplay {
    fn default() -> Self { HdrDisplay { headroom: None, white_scale: 1.0, sdr_white_nits: None, peak_nits: None } }
}

impl HdrDisplay {
    pub fn query(surface: &wgpu::Surface<'_>, adapter: &wgpu::Adapter) -> Self {
        Self::from_info(&surface.display_hdr_info(adapter))
    }

    pub fn from_info(info: &wgpu::DisplayHdrInfo) -> Self {
        let finite = |v: Option<f32>| v.filter(|v| v.is_finite() && *v > 0.0);
        let sdr_white_nits = finite(info.luminance.and_then(|l| l.sdr_white_nits));
        let peak_nits = finite(info.luminance.and_then(|l| l.max_nits));
        HdrDisplay {
            headroom: finite(info.tone_map_headroom()).map(|h| h.max(1.0)),
            white_scale: sdr_white_nits.map_or(1.0, |n| (n / 80.0).clamp(0.5, 12.5)),
            sdr_white_nits, peak_nits,
        }
    }

    /// One line for the debug menu and logs.
    pub fn describe(&self) -> String {
        let headroom = self.headroom.map_or("unknown headroom".to_string(), |h| format!("headroom {h:.1}x"));
        match (self.sdr_white_nits, self.peak_nits) {
            (Some(w), Some(p)) => format!("{headroom}, SDR white {w:.0} nits, peak {p:.0} nits"),
            (Some(w), None) => format!("{headroom}, SDR white {w:.0} nits"),
            _ => headroom,
        }
    }
}

/// Whether the surface can show HDR: fp16 with linear extended sRGB (scRGB
/// on Windows, EDR on macOS, `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT`).
pub fn surface_supports_hdr(caps: &wgpu::SurfaceCapabilities) -> bool {
    caps.color_spaces(wgpu::TextureFormat::Rgba16Float).contains(wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR)
}

/// Configure a window surface for HDR (fp16 scRGB) or SDR (the usual sRGB
/// format). Returns whether HDR is on: false when asked for SDR or when the
/// surface cannot show HDR.
pub fn configure_surface(config: &mut wgpu::SurfaceConfiguration, caps: &wgpu::SurfaceCapabilities, hdr: bool) -> bool {
    if hdr && surface_supports_hdr(caps) {
        config.format = wgpu::TextureFormat::Rgba16Float;
        config.color_space = wgpu::SurfaceColorSpace::ExtendedSrgbLinear;
        return true;
    }
    if let Some(format) = caps.formats.iter().find(|f| f.is_srgb()).or(caps.formats.first()) { config.format = *format; }
    config.color_space = wgpu::SurfaceColorSpace::Auto;
    false
}

/// SMPTE ST 2084 (PQ): absolute luminance in nits to a 0..1 signal.
pub fn pq_encode(nits: f32) -> f32 {
    let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
    let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 4096.0 * 32.0, 2392.0 / 4096.0 * 32.0);
    let y = (nits / 10000.0).clamp(0.0, 1.0).powf(m1);
    ((c1 + c2 * y) / (1.0 + c3 * y)).powf(m2)
}

/// IEEE half to f32 (the HDR output's texels).
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h >> 15 == 1 { -1.0 } else { 1.0 };
    let (exp, frac) = (((h >> 10) & 0x1f) as i32, (h & 0x3ff) as f32);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => if frac == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        e => sign * (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Write the HDR output ([`crate::Renderer::hdr_output`]: linear BT.709,
/// 1.0 = SDR white; `rgba` its half-float texels) as a 16-bit PNG in BT.2100
/// PQ (cICP: BT.2020 primaries, PQ, full range) with SDR white at
/// `sdr_white_nits`, the level the screen showed it at. Chrome and Windows
/// Photos show it in HDR.
pub fn write_pq_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u16], sdr_white_nits: f32) -> Result<(), String> {
    if rgba.len() != w as usize * h as usize * 4 { return Err("HDR frame size mismatch".into()); }
    // BT.709 to BT.2020 primaries (ITU-R BT.2087).
    const M: [[f32; 3]; 3] = [[0.627404, 0.329283, 0.043313], [0.069097, 0.919540, 0.011362], [0.016391, 0.088013, 0.895595]];
    let mut out = Vec::with_capacity(w as usize * h as usize * 6);
    for px in rgba.chunks_exact(4) {
        let c = [px[0], px[1], px[2]].map(|v| { let v = half_to_f32(v); if v.is_finite() { v.max(0.0) } else { 0.0 } });
        for row in M {
            let v = pq_encode((row[0] * c[0] + row[1] * c[1] + row[2] * c[2]) * sdr_white_nits);
            out.extend_from_slice(&((v * 65535.0).round() as u16).to_be_bytes());
        }
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Sixteen);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_chunk(png::chunk::ChunkType(*b"cICP"), &[9, 16, 0, 1]).map_err(|e| e.to_string())?;
    wr.write_image_data(&out).map_err(|e| e.to_string())
}

/// The renderer's HDR copy of the output.
pub(crate) struct HdrTarget {
    pub headroom: f32,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    dummy: Option<wgpu::TextureView>,
}

impl HdrTarget {
    pub fn new() -> Self { HdrTarget { headroom: 1.0, texture: None, dummy: None } }

    /// The headroom to tone for this frame and the view to bind: the output
    /// size when on, a 1x1 placeholder when off.
    pub fn prepare(&mut self, gpu: &crate::Gpu, width: u32, height: u32) -> (f32, wgpu::TextureView) {
        let make = |w: u32, h: u32| {
            let t = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("HDR output"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
            let v = t.create_view(&Default::default());
            (t, v)
        };
        if self.headroom <= 1.0 {
            self.texture = None;
            let dummy = self.dummy.get_or_insert_with(|| make(1, 1).1);
            return (1.0, dummy.clone());
        }
        if self.texture.as_ref().is_none_or(|(t, _)| (t.width(), t.height()) != (width, height)) {
            self.texture = Some(make(width.max(1), height.max(1)));
        }
        (self.headroom, self.texture.as_ref().unwrap().1.clone())
    }

    pub fn texture(&self) -> Option<wgpu::Texture> {
        if self.headroom > 1.0 { self.texture.as_ref().map(|(t, _)| t.clone()) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_signal_and_half_floats() {
        assert!((pq_encode(10000.0) - 1.0).abs() < 1e-6);
        // 100 nits is PQ code 520 of 1023 (BT.2100 reference).
        assert!((pq_encode(100.0) * 1023.0 - 520.0).abs() < 1.0);
        assert!(pq_encode(0.0) < 1e-6);
        // 1.0, 0.5, 65504, the smallest subnormal, -2.
        for (bits, v) in [(0x3c00u16, 1.0f32), (0x3800, 0.5), (0x7bff, 65504.0), (0x0001, 5.960_464_5e-8), (0xc000, -2.0)] {
            assert_eq!(half_to_f32(bits), v);
        }
    }

    #[test]
    fn pq_png_has_its_colour_space_chunk() {
        let path = std::env::temp_dir().join(format!("warp-pq-{}.png", std::process::id()));
        write_pq_png(&path, 2, 1, &[0x3c00, 0x3c00, 0x3c00, 0x3c00, 0, 0, 0, 0x3c00], 203.0).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let reader = decoder.read_info().unwrap();
        let cicp = reader.info().coding_independent_code_points.unwrap();
        assert_eq!((cicp.color_primaries, cicp.transfer_function, reader.info().bit_depth), (9, 16, png::BitDepth::Sixteen));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sdr_range_is_unchanged_and_white_reaches_the_headroom() {
        for v in [0.0, 0.1, 0.21, 0.42, KNEE] {
            assert_eq!(expand([v, v * 0.5, v * 0.25], 4.0, KNEE), [v, v * 0.5, v * 0.25]);
        }
        let white = expand([1.0; 3], 4.0, KNEE);
        assert!((white[0] - 4.0).abs() < 1e-6);
        assert_eq!(expand([1.0; 3], 1.0, KNEE), [1.0; 3], "no headroom: SDR");
    }

    #[test]
    fn highlights_stay_ordered_smooth_and_keep_their_hue() {
        let mut previous = 0.0;
        for i in 0..=1000 {
            let v = i as f32 / 1000.0;
            let out = expand([v; 3], 6.0, KNEE)[0];
            assert!(out >= previous && out.is_finite(), "{v}");
            previous = out;
        }
        // Continuous at the knee: just above it is still about the SDR value.
        let just = expand([KNEE + 1e-4; 3], 6.0, KNEE)[0];
        assert!((just - KNEE).abs() < 1e-3);
        let warm = expand([0.9, 0.6, 0.3], 4.0, KNEE);
        assert!((warm[1] / warm[0] - 0.6 / 0.9).abs() < 1e-6 && (warm[2] / warm[0] - 0.3 / 0.9).abs() < 1e-6);
    }

    #[test]
    fn display_info_maps_to_headroom_and_white() {
        let windows = wgpu::DisplayHdrInfo { luminance: Some(wgpu::DisplayLuminance { max_nits: Some(1000.0), sdr_white_nits: Some(240.0), ..Default::default() }), ..Default::default() };
        let d = HdrDisplay::from_info(&windows);
        assert!((d.headroom.unwrap() - 1000.0 / 240.0).abs() < 1e-4);
        assert!((d.white_scale - 3.0).abs() < 1e-6);
        let unknown = HdrDisplay::from_info(&wgpu::DisplayHdrInfo::default());
        assert_eq!((unknown.headroom, unknown.white_scale), (None, 1.0));
        assert!(d.describe().contains("240 nits"));
    }

    #[test]
    fn tone_shader_with_hdr_copy_validates() {
        let src = include_str!("shaders/tonemap.wgsl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert!(src.contains("fn expand_hdr"));
        let src = include_str!("shaders/present_hdr.wgsl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
    }
}
