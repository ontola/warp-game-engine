//! Party artwork without an image library: embedded `data:image/...;base64,`
//! covers (decode the returned bytes with `render::image::decode`) and the
//! profile pixel-art avatars (format v1 and the legacy square array), as in
//! `sdk/godot/addons/gamenight/{artwork,face}.gd`.

use serde_json::Value;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const AVATAR_MAX_SIDE: usize = 256;
pub const AVATAR_SIZE: f32 = 48.0;
pub const AVATAR_HEAD: [f32; 2] = [24.0, 28.0];
pub const AVATAR_HEAD_RADIUS: f32 = 12.0;
const LEGACY_TRANSPARENT: &str = "#0f172a";

/// The bytes of a `data:image/*;base64,` URL; None for anything else.
pub fn data_url(value: &str) -> Option<Vec<u8>> {
    if !value.starts_with("data:image/") || value.len() > MAX_BYTES * 4 / 3 + 64 { return None; }
    let (head, payload) = value.split_once(',')?;
    if !head.ends_with(";base64") { return None; }
    base64(payload)
}

pub fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62, b'/' | b'_' => 63, b'=' | b'\n' | b'\r' | b' ' => continue, _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    Some(out)
}

/// A decoded profile avatar: straight RGBA8, and the head centre in its pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Avatar { pub width: u32, pub height: u32, pub rgba: Vec<u8>, pub head: [f32; 2] }

/// Decode a player's `avatar` payload; None when empty or invalid.
pub fn avatar(payload: &str) -> Option<Avatar> {
    if payload.is_empty() || payload.len() > 800_000 { return None; }
    let data: Value = serde_json::from_str(payload).ok()?;
    let (width, height, pixels, legacy) = if let Some(pixels) = data.as_array() {
        let side = (pixels.len() as f64).sqrt() as usize;
        (side, side, pixels.clone(), true)
    } else if data["v"].as_u64() == Some(1) {
        let side = |key: &str| data[key].as_f64().filter(|v| v.fract() == 0.0 && *v >= 1.0 && *v <= AVATAR_MAX_SIDE as f64).map(|v| v as usize);
        (side("w")?, side("h")?, data["px"].as_array().cloned().unwrap_or_default(), false)
    } else { return None };
    if width == 0 || width > AVATAR_MAX_SIDE || height > AVATAR_MAX_SIDE || pixels.len() != width * height { return None; }
    let mut rgba = vec![0u8; width * height * 4];
    for (i, pixel) in pixels.iter().enumerate() {
        let Some(hex) = pixel.as_str() else { continue };
        if legacy && hex.eq_ignore_ascii_case(LEGACY_TRANSPARENT) { continue; }
        if let Some(rgb) = hex_color(hex) { rgba[i * 4..i * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]); }
    }
    let offset = |side: usize, small: f32| match side { 32 => small, 16 => small + 8.0, _ => (AVATAR_SIZE - side as f32) / 2.0 };
    Some(Avatar { width: width as u32, height: height as u32, rgba, head: [AVATAR_HEAD[0] - offset(width, 14.0), AVATAR_HEAD[1] - offset(height, 15.0)] })
}

/// `#rrggbb` as sRGB bytes.
pub fn hex_color(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let v = u32::from_str_radix(digits, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_decode_and_other_urls_do_not() {
        assert_eq!(data_url("data:image/png;base64,iVBORw0K").unwrap(), [0x89, b'P', b'N', b'G', 0x0d, 0x0a]);
        assert_eq!(data_url("https://example.com/a.png"), None);
        assert_eq!(data_url("data:image/png,raw"), None);
        assert_eq!(base64("aGVsbG8=").unwrap(), b"hello");
    }

    #[test]
    fn avatars_decode_v1_and_legacy_with_head_centres() {
        let v1 = avatar(r##"{"v":1,"w":2,"h":1,"px":["#ff0000",""]}"##).unwrap();
        assert_eq!((v1.width, v1.height), (2, 1));
        assert_eq!(v1.rgba, [255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(v1.head, [24.0 - 23.0, 28.0 - 23.5]);
        let legacy = avatar(&serde_json::to_string(&vec!["#0f172a"; 256]).unwrap()).unwrap();
        assert_eq!((legacy.width, legacy.head), (16, [2.0, 5.0]));
        assert!(legacy.rgba.iter().all(|&b| b == 0), "legacy dark is transparent");
        assert_eq!(avatar(r##"{"v":1,"w":2.5,"h":1,"px":[]}"##), None);
        assert_eq!(avatar(""), None);
    }
}
