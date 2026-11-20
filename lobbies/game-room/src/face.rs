//! Profile art as Splitscreen Club sends it (addons/gamenight/face.gd `decode`):
//! JSON `{"v": 1, "w", "h", "px": ["#rrggbb" | null, ...]}`, or the legacy
//! square array whose `#0f172a` pixels are transparent.

use image::RgbaImage;
use serde_json::Value;

const FORMAT_VERSION: i64 = 1;
const HEAD_X: f32 = 24.0;
const HEAD_Y: f32 = 28.0;
const LEGACY_TRANSPARENT: &str = "#0f172a";
const MAX_SIDE: u64 = 256;
const SIZE: f32 = 48.0;

/// The art and the point of it that sits on the head's centre.
pub struct Decoded { pub image: RgbaImage, pub center: [f32; 2] }

pub fn decode(payload: &str) -> Option<Decoded> {
    if payload.is_empty() || payload.len() > 800_000 { return None; }
    let data: Value = serde_json::from_str(payload).ok()?;
    let (width, height, pixels, legacy) = match &data {
        Value::Array(px) => { let side = (px.len() as f64).sqrt() as u64; (side, side, px.as_slice(), true) }
        Value::Object(o) if o.get("v").and_then(Value::as_i64) == Some(FORMAT_VERSION) => {
            let side = |k: &str| o.get(k).and_then(Value::as_f64).filter(|v| v.fract() == 0.0 && *v >= 1.0 && *v <= MAX_SIDE as f64).map(|v| v as u64);
            let px = o.get("px").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            (side("w")?, side("h")?, px, false)
        }
        _ => return None,
    };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || pixels.len() as u64 != width * height { return None; }
    // Godot fills with Color.TRANSPARENT: white at zero alpha.
    let mut image = RgbaImage::from_pixel(width as u32, height as u32, image::Rgba([255, 255, 255, 0]));
    for (i, p) in pixels.iter().enumerate() {
        let Some(s) = p.as_str() else { continue };
        if s.len() != 7 || !s.starts_with('#') || !s[1..].chars().all(|c| c.is_ascii_hexdigit()) { continue; }
        if legacy && s.eq_ignore_ascii_case(LEGACY_TRANSPARENT) { continue; }
        let b = |k: usize| u8::from_str_radix(&s[k..k + 2], 16).unwrap_or(0);
        image.put_pixel((i as u64 % width) as u32, (i as u64 / width) as u32, image::Rgba([b(1), b(3), b(5), 255]));
    }
    let center = [HEAD_X - offset(width, 14.0), HEAD_Y - offset(height, 15.0)];
    Some(Decoded { image, center })
}

fn offset(side: u64, small: f32) -> f32 {
    match side { 32 => small, 16 => small + 8.0, _ => (SIZE - side as f32) / 2.0 }
}

/// The real faces the Godot demo uses (assets/demo-faces.json).
pub fn demo(index: usize) -> String {
    static FACES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let faces = FACES.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!("../assets/demo-faces.json")).unwrap_or(Value::Null);
        data.as_array().map(|a| a.iter().map(|f| f["avatar"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
    });
    if faces.is_empty() { String::new() } else { faces[index % faces.len()].clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_demo_faces() {
        let face = decode(&demo(0)).expect("demo face");
        assert_eq!(face.image.dimensions(), (48, 48));
        assert_eq!(face.center, [24.0, 28.0]);
        assert!(face.image.pixels().any(|p| p[3] == 255));
        assert!(decode("[").is_none());
        assert!(decode(r##"{"v": 1, "w": 2, "h": 1, "px": ["#ff0000"]}"##).is_none());
        let legacy = decode(r##"["#0f172a", "#ff0000", "#00ff00", "#0000ff"]"##).unwrap();
        assert_eq!(legacy.image.get_pixel(0, 0)[3], 0);
        assert_eq!(legacy.image.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }
}
