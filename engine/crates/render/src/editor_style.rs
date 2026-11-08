//! Shared colours for Warp developer overlays. Shapes use linear RGBA and glyphs use sRGB bytes.
pub const FG: [u8; 4] = [237, 242, 248, 255];
pub const MUTED: [u8; 4] = [164, 180, 196, 255];
pub const ACCENT: [u8; 4] = [118, 221, 208, 255];
pub const PANEL: [f32; 4] = [0.019, 0.027, 0.041, 1.0];
pub const HIGHLIGHT: [f32; 4] = [0.12, 0.64, 0.55, 0.95];

pub fn font_size(viewport: [u32; 2]) -> f32 {
    18. * (viewport[1] as f32 / 900.).clamp(0.8, 2.5).min(viewport[0] as f32 / 500.)
}

/// Apply after local layout transforms when the HUD uses output pixels.
pub fn style_text(hud: &mut crate::hud::HudDrawList, first: usize) {
    let size = font_size(hud.size);
    for run in &mut hud.text[first..] { run.font_size = size; }
}

/// Draw the panel and its left focus rail in the overlay's coordinate system.
pub fn panel(hud: &mut crate::hud::HudDrawList, rect: [f32; 4], scale: f32, clip: [f32; 4]) {
    hud.rect(rect, PANEL, clip);
    hud.rect([rect[0], rect[1], 3.0 * scale, rect[3]], HIGHLIGHT, clip);
}

/// Button drawing and hit testing use the same logical rectangle. HUD labels
/// are measured in output pixels after the host applies its editor scale.
#[derive(Clone, Copy)]
pub struct EditorButton(pub [f32; 4]);

impl EditorButton {
    pub fn contains(self, point: [f32; 2]) -> bool {
        let [x, y, w, h] = self.0;
        point[0] >= x && point[1] >= y && point[0] < x + w && point[1] < y + h
    }

    pub fn draw(
        self,
        hud: &mut crate::hud::HudDrawList,
        id: u64,
        label: &str,
        clip: [f32; 4],
        font_size: f32,
        highlighted: bool,
        enabled: bool,
    ) {
        use crate::hud::{FontFace, TextAlign, TextRun};
        let [x, y, w, h] = self.0;
        let left = x.max(clip[0]);
        let top = y.max(clip[1]);
        let width = (x + w).min(clip[0] + clip[2]) - left;
        let height = (y + h).min(clip[1] + clip[3]) - top;
        if width <= 0. || height <= 0. {
            return;
        }
        let fill = if !enabled {
            [0.045, 0.052, 0.060, 1.]
        } else if highlighted {
            [0.08, 0.29, 0.24, 1.]
        } else {
            [0.10, 0.15, 0.19, 1.]
        };
        hud.rect(self.0, fill, clip);
        hud.text.push(TextRun {
            id,
            text: label.chars().take(240).collect(),
            position: [x, y + ((h - font_size) * 0.5).max(0.)],
            bounds: [left, top, width, height],
            font_size,
            color: if enabled { FG } else { MUTED },
            font: FontFace::Body,
            align: TextAlign::Center,
            width: w,
        });
    }
}
