//! Sideways Shift+F12 selection in physical window and original PNG pixels.
use super::*;
use app::screenshot::Saved;
use render::hud::{FontFace, HudDrawList, HudImage, ImageFilter, TextAlign, TextRun};
use render::picture::Picture;

pub(super) struct Picker {
    saved: Saved,
    picture: Arc<Picture>,
    displayed: Option<[u32; 2]>,
}

fn image_rect(size: [u32; 2], frame: [u32; 2]) -> [f32; 4] {
    let [w, h] = size.map(|v| v as f32);
    let header = 32.0_f32.min(h / 3.0);
    let scale = (w / frame[0] as f32).min((h - header) / frame[1] as f32);
    let (iw, ih) = (
        (frame[0] as f32 * scale).round(),
        (frame[1] as f32 * scale).round(),
    );
    [(w - iw) * 0.5, header + (h - header - ih) * 0.5, iw, ih]
}

fn pixel(cursor: [f64; 2], size: [u32; 2], frame: [u32; 2]) -> Option<[u32; 2]> {
    let [x, y, w, h] = image_rect(size, frame).map(f64::from);
    let [u, v] = [(cursor[0] - x) / w, (cursor[1] - y) / h];
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    Some([
        (u * f64::from(frame[0])) as u32,
        (v * f64::from(frame[1])) as u32,
    ])
}

impl Picker {
    pub(super) fn new(saved: Saved, w: u32, h: u32, rgba: Vec<u8>) -> Result<Self, String> {
        Ok(Self {
            saved,
            picture: Arc::new(Picture::new(w, h, rgba)?),
            displayed: None,
        })
    }

    pub(super) fn resized(&mut self) {
        self.displayed = None;
    }

    fn hud(&self, size: [u32; 2]) -> HudDrawList {
        let mut hud = HudDrawList::new(size);
        let [w, h] = size.map(|v| v as f32);
        let clip = [0., 0., w, h];
        let image = HudImage(u64::MAX - 51);
        hud.rect(clip, [0.02, 0.02, 0.03, 1.0], clip);
        hud.provide_image(image, &self.picture, ImageFilter::Linear);
        hud.image(
            image_rect(size, [self.picture.width, self.picture.height]),
            image,
            [0., 0., 1., 1.],
            [1.; 4],
            clip,
        );
        hud.text.push(TextRun {
            id: u64::MAX - 52,
            text: "Click the point to report   |   Esc to skip".into(),
            position: [4., 3.],
            bounds: clip,
            font_size: 18.,
            color: [255; 4],
            font: FontFace::Body,
            align: TextAlign::Center,
            width: (w - 8.).max(1.),
        });
        hud
    }

    fn save(&mut self, point: [u32; 2]) -> Result<(), String> {
        use std::io::Write;
        self.saved.metadata["player_point"] = serde_json::json!(point);
        let path = self.saved.png.with_extension("json");
        let tmp = path.with_extension("point.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let mut writer = std::io::BufWriter::new(&mut file);
            serde_json::to_writer_pretty(&mut writer, &self.saved.metadata)?;
            writer.flush()?;
            drop(writer);
            drop(file);
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(tmp);
        }
        result.map_err(|e| e.to_string())
    }
}

impl App {
    pub(super) fn end_point_picker(&mut self, line: String) {
        self.point_picker = None;
        if let Some(hud) = self.graphics.as_mut().and_then(|g| g.hud.as_mut()) {
            hud.remove_image(HudImage(u64::MAX - 51));
        }
        self.last_frame = Instant::now();
        self.accumulator = 0.;
        self.screenshot_toast = Some((line, Instant::now() + Duration::from_secs(4)));
        if let Some(w) = &self.window {
            w.set_cursor(winit::window::CursorIcon::Default);
            w.request_redraw();
        }
    }

    pub(super) fn select_feedback_point(&mut self) {
        let picker = self.point_picker.as_mut().unwrap();
        let size = self.window.as_ref().unwrap().inner_size();
        let size = [size.width, size.height];
        if picker.displayed != Some(size) {
            return;
        }
        let Some(p) = self
            .cursor
            .and_then(|c| pixel(c, size, [picker.picture.width, picker.picture.height]))
        else {
            return;
        };
        let message = match picker.save(p) {
            Ok(()) => format!("Screenshot marked at {}, {}", p[0], p[1]),
            Err(e) => format!("Screenshot saved; point could not be saved: {e}"),
        };
        self.end_point_picker(message);
    }

    pub(super) fn draw_point_picker(&mut self) -> Result<(), String> {
        if self.invisible() {
            return Ok(());
        }
        let window = self.window.as_ref().unwrap();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let size = [size.width, size.height];
        let picker = self.point_picker.as_mut().unwrap();
        let g = self.graphics.as_mut().unwrap();
        if g.hud.is_none() {
            g.hud = Some(render::hud::HudRenderer::new(&g.gpu, g.config.format)?);
        }
        g.hud.as_mut().unwrap().prepare(&g.gpu, &picker.hud(size))?;
        match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                g.presenter.present(
                    &g.gpu,
                    self.cached_picture.as_ref().unwrap(),
                    &frame.texture.create_view(&Default::default()),
                    g.hud.as_ref(),
                    None,
                )?;
                window.pre_present_notify();
                g.gpu.queue.present(frame);
                picker.displayed = Some(size);
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                g.surface = g
                    .instance
                    .create_surface(window.clone())
                    .map_err(|e| e.to_string())?;
                g.surface.configure(&g.gpu.device, &g.config);
                window.request_redraw();
            }
            wgpu::CurrentSurfaceTexture::Timeout => window.request_redraw(),
            wgpu::CurrentSurfaceTexture::Occluded => {}
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("feedback selection surface validation failure".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mark_replaces_metadata_and_preserves_the_frame() {
        let dir = std::env::temp_dir().join(format!("sideways-mark-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("shot.png");
        std::fs::write(&png, b"original").unwrap();
        let metadata = serde_json::json!({"drive":{"end":27}});
        std::fs::write(png.with_extension("json"), metadata.to_string()).unwrap();
        let saved = Saved {
            png: png.clone(),
            metadata,
            line: "saved".into(),
        };
        let mut picker = Picker::new(saved, 2, 2, vec![0; 16]).unwrap();
        picker.save([1, 0]).unwrap();
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(png.with_extension("json")).unwrap()).unwrap();
        assert_eq!(
            result,
            serde_json::json!({"drive":{"end":27},"player_point":[1,0]})
        );
        assert_eq!(std::fs::read(png).unwrap(), b"original");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn selects_original_pixels_after_letterboxing() {
        let size = [120, 132];
        let frame = [200, 100];
        assert_eq!(pixel([0., 52.], size, frame), Some([0, 0]));
        assert_eq!(pixel([119.99, 111.99], size, frame), Some([199, 99]));
        assert_eq!(pixel([60., 82.], size, frame), Some([100, 50]));
        assert_eq!(pixel([60., 20.], size, frame), None);
        assert_eq!(pixel([60., 112.], size, frame), None);
    }
}
