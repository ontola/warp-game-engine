//! Window geometry shared by Warp's editor panels. Coordinates may be physical
//! or logical pixels, provided the viewport, pointer and HUD use the same space.
use crate::{
    editor_style::{self, EditorButton},
    hud::{FontFace, HudDrawList, TextAlign, TextRun},
};

#[derive(Clone, Copy, Debug, Default)]
pub struct EditorWindow {
    relative: Option<[f32; 4]>,
    drag: Option<([f32; 2], [f32; 4], bool, [f32; 2])>,
    last_size: [u32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct WindowFrame {
    pub rect: [f32; 4],
    pub scale: f32,
}

impl WindowFrame {
    pub fn close(self) -> EditorButton {
        let [x, y, w, _] = self.rect;
        let s = self.scale;
        EditorButton([x + w - 38. * s, y + 5. * s, 28. * s, 25. * s])
    }
    pub fn grip(self) -> EditorButton {
        let [x, y, w, h] = self.rect;
        let n = 17. * self.scale;
        EditorButton([x + w - n, y + h - n, n, n])
    }
    pub fn body(self) -> [f32; 4] {
        let [x, y, w, h] = self.rect;
        let s = self.scale;
        [
            x + 3. * s,
            y + 32. * s,
            (w - 6. * s).max(1.),
            (h - 44. * s).max(1.),
        ]
    }
    pub fn draw(self, hud: &mut HudDrawList, id: u64, key: u8, title: &str) {
        let r = self.rect;
        let s = self.scale;
        hud.begin_overlay();
        editor_style::panel(hud, r, s, r);
        hud.rect(
            [r[0] + 3. * s, r[1], r[2] - 3. * s, 32. * s],
            [0.05, 0.09, 0.12, 1.],
            r,
        );
        let width = (r[2] - 51. * s).max(1.);
        hud.text.push(TextRun {
            id,
            text: format!("F{key}   {title}"),
            position: [r[0] + 8. * s, r[1] + 3. * s],
            bounds: [r[0] + 8. * s, r[1], width, 32. * s],
            width,
            font_size: 21. * s,
            color: editor_style::FG,
            font: FontFace::Body,
            align: TextAlign::Left,
        });
        self.close().draw(hud, id + 1, "x", r, 21. * s, false, true);
        let [gx, gy, gw, gh] = self.grip().0;
        for n in [6., 12.] {
            hud.line(
                [gx + gw - n * s, gy + gh - 2. * s],
                [gx + gw - 2. * s, gy + gh - n * s],
                2. * s,
                editor_style::HIGHLIGHT,
                r,
            );
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum WindowEvent {
    Content,
    Handled,
    Close,
}

impl EditorWindow {
    pub fn cancel(&mut self) {
        self.drag = None;
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn frame(
        &self,
        size: [u32; 2],
        default: [f32; 4],
        minimum: [f32; 2],
        scale: f32,
    ) -> WindowFrame {
        let [sw, sh] = size.map(|v| v.max(1) as f32);
        let [x, y, w, h] = self
            .relative
            .map_or(default, |a| [a[0] * sw, a[1] * sh, a[2] * sw, a[3] * sh]);
        let w = w.clamp(minimum[0].min(sw), sw);
        let h = h.clamp(minimum[1].min(sh), sh);
        WindowFrame {
            rect: [x.clamp(0., sw - w), y.clamp(0., sh - h), w, h],
            scale,
        }
    }
    pub fn pointer(
        &mut self,
        size: [u32; 2],
        frame: WindowFrame,
        minimum: [f32; 2],
        position: Option<[f32; 2]>,
        pressed: bool,
        held: bool,
    ) -> WindowEvent {
        if self.last_size != size || (!pressed && !held) {
            self.cancel();
        }
        self.last_size = size;
        let Some(p) = position else {
            self.cancel();
            return WindowEvent::Content;
        };
        if pressed && EditorButton(frame.rect).contains(p) {
            if frame.close().contains(p) {
                self.cancel();
                return WindowEvent::Close;
            }
            let resize = frame.grip().contains(p);
            if resize || p[1] < frame.rect[1] + 32. * frame.scale {
                self.drag = Some((p, frame.rect, resize, minimum));
            }
        }
        if self.dragging() {
            self.move_to(p);
            return WindowEvent::Handled;
        }
        WindowEvent::Content
    }
    pub fn move_to(&mut self, p: [f32; 2]) {
        if let Some((start, [x, y, w, h], resize, minimum)) = self.drag {
            let size = self.last_size;
            let [sw, sh] = size.map(|n| n.max(1) as f32);
            let dx = p[0] - start[0];
            let dy = p[1] - start[1];
            let r = if resize {
                [
                    x,
                    y,
                    (w + dx).clamp(minimum[0].min(sw - x), sw - x),
                    (h + dy).clamp(minimum[1].min(sh - y), sh - y),
                ]
            } else {
                [
                    (x + dx).clamp(0., (sw - w).max(0.)),
                    (y + dy).clamp(0., (sh - h).max(0.)),
                    w,
                    h,
                ]
            };
            self.relative = Some([r[0] / sw, r[1] / sh, r[2] / sw, r[3] / sh]);
        }
    }
}
