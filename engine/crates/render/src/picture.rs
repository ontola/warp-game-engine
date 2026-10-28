//! Pictures that arrive while a game runs: PNG, JPEG, WebP and SVG bytes to
//! straight-alpha sRGB RGBA8, sniffed by content (not by name), with bounds.
//! Hand the result to `hud::HudRenderer::set_image` or to a world screen.

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SIDE: u32 = 4096;

#[derive(Clone, Debug, PartialEq)]
pub struct Picture { pub width: u32, pub height: u32, pub rgba: Vec<u8> }

impl Picture {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE { return Err(format!("picture size {width}x{height} out of range")); }
        if rgba.len() != (width * height * 4) as usize { return Err("picture data does not match its size".into()); }
        Ok(Self { width, height, rgba })
    }
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        Self { width, height, rgba: rgba.repeat((width * height) as usize) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Png, Jpeg, WebP, Svg }

pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { return Some(Kind::Png); }
    if bytes.starts_with(&[0xff, 0xd8]) { return Some(Kind::Jpeg); }
    if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { return Some(Kind::WebP); }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if head.contains("<svg") { return Some(Kind::Svg); }
    None
}

/// Decode PNG/JPEG/WebP at their own size. SVG uses its own size times
/// `svg_scale` (a QR code wants a crisp integer scale).
pub fn decode(bytes: &[u8], svg_scale: f32) -> Result<Picture, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES { return Err("picture is empty or too large".into()); }
    match sniff(bytes).ok_or("unrecognised picture format")? {
        Kind::Svg => svg(bytes, svg_scale, None),
        kind => {
            let format = match kind { Kind::Png => image::ImageFormat::Png, Kind::Jpeg => image::ImageFormat::Jpeg, _ => image::ImageFormat::WebP };
            let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_SIDE);
            limits.max_image_height = Some(MAX_SIDE);
            reader.limits(limits);
            let rgba = reader.decode().map_err(|e| format!("picture decode: {e}"))?.to_rgba8();
            let (width, height) = rgba.dimensions();
            Picture::new(width, height, rgba.into_raw())
        }
    }
}

/// Rasterise SVG. `fit` scales it to exactly that size instead (aspect kept,
/// centred, transparent margins).
pub fn svg(bytes: &[u8], scale: f32, fit: Option<[u32; 2]>) -> Result<Picture, String> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|e| format!("svg: {e}"))?;
    let size = tree.size();
    let (width, height, transform) = match fit {
        Some([w, h]) => {
            let s = (w as f32 / size.width()).min(h as f32 / size.height());
            (w, h, tiny_skia::Transform::from_row(s, 0.0, 0.0, s, (w as f32 - size.width() * s) / 2.0, (h as f32 - size.height() * s) / 2.0))
        }
        None => {
            let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
            ((size.width() * s).ceil() as u32, (size.height() * s).ceil() as u32, tiny_skia::Transform::from_scale(s, s))
        }
    };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE { return Err(format!("svg size {width}x{height} out of range")); }
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("svg pixmap")?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia is premultiplied; pictures are straight alpha.
    let mut rgba = pixmap.take();
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a > 0 && a < 255 { for c in &mut p[..3] { *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8; } }
    }
    Picture::new(width, height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_svg_decode_by_content() {
        let mut png = vec![];
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 128])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let picture = decode(&png, 1.0).unwrap();
        assert_eq!((picture.width, picture.height, &picture.rgba[..4]), (3, 2, &[10, 20, 30, 128][..]));
        let qr = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4" width="4" height="4"><rect width="2" height="2" fill="#000"/><rect x="2" y="2" width="2" height="2" fill="rgba(255,0,0,0.5)"/></svg>"##;
        let picture = decode(qr, 2.0).unwrap();
        assert_eq!((picture.width, picture.height), (8, 8));
        assert_eq!(&picture.rgba[..4], &[0, 0, 0, 255]);
        let red = &picture.rgba[(7 * 8 + 7) * 4..][..4];
        assert!(red[0] >= 254 && (126..=129).contains(&red[3]), "straight alpha: {red:?}");
        assert_eq!(&picture.rgba[(7 * 8) * 4..][..4], &[0, 0, 0, 0]);
        let fitted = svg(qr, 1.0, Some([16, 8])).unwrap();
        assert_eq!((fitted.width, fitted.height, fitted.rgba[3]), (16, 8, 0), "centred with margins");
        assert!(decode(b"GIF89a", 1.0).is_err());
    }
}
