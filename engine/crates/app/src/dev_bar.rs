//! Shortcut strip for the native Warp developer workspace.
use crate::{Game, KeyCode};
use render::{
    editor_style::{self, EditorButton},
    hud::{FontFace, HudDrawList},
};

const KEYS: [KeyCode; 12] = [
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
];

#[derive(Default)]
pub(crate) struct DevBar {
    buttons: Vec<(KeyCode, EditorButton)>,
}
impl DevBar {
    pub fn hit(&self, p: Option<[f32; 2]>) -> Option<KeyCode> {
        let p = p?;
        self.buttons
            .iter()
            .find(|(_, b)| b.contains(p))
            .map(|(key, _)| *key)
    }
    pub fn draw(
        &mut self,
        hud: &mut HudDrawList,
        game: &dyn Game,
        stats: bool,
        graphics: bool,
    ) {
        hud.begin_foreground();
        self.buttons.clear();
        let [width, height] = hud.size.map(|v| v as f32);
        let font = editor_style::font_size(hud.size);
        let row = (font * 1.7).ceil();
        let clip = [0., 0., width, height];
        let mut x = 4.;
        let mut y = height - row - 4.;
        for (index, key) in KEYS.into_iter().enumerate() {
            let n = index as u8 + 1;
            let (name, on) = match n {
                1 => ("Warp Dev Mode", true),
                2 => ("Graphics", graphics),
                3 => ("Profiler", stats),
                4 => ("Screenshot", false),
                _ => match game.editor_shortcut(n) {
                    Some(item) => item,
                    None => continue,
                },
            };
            let label = format!("F{n}  {name}");
            let w = (render::hud::measure(&label, FontFace::Body, font)[0] + font)
                .ceil()
                .min(width);
            if x > 4. && x + w > width {
                x = 4.;
                y -= row + 2.;
            }
            let b = EditorButton([x, y, w, row]);
            b.draw(
                hud,
                0x6465760000000000 + n as u64,
                &label,
                clip,
                font,
                on,
                true,
            );
            self.buttons.push((key, b));
            x += w + 2.;
        }
    }
}
