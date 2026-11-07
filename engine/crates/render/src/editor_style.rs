//! Shared colours for Warp developer overlays. Shapes use linear RGBA and glyphs use sRGB bytes.
pub const FG: [u8; 4] = [237, 242, 248, 255];
pub const MUTED: [u8; 4] = [164, 180, 196, 255];
pub const ACCENT: [u8; 4] = [118, 221, 208, 255];
pub const PANEL: [f32; 4] = [0.019, 0.027, 0.041, 1.0];
pub const HIGHLIGHT: [f32; 4] = [0.12, 0.64, 0.55, 0.95];

/// Draw the panel and its left focus rail in the overlay's coordinate system.
pub fn panel(hud: &mut crate::hud::HudDrawList, rect: [f32; 4], scale: f32, clip: [f32; 4]) {
    hud.rect(rect, PANEL, clip);
    hud.rect([rect[0], rect[1], 3.0 * scale, rect[3]], HIGHLIGHT, clip);
}
