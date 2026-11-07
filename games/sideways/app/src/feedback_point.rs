//! Sideways F12 selection in physical window and original PNG pixels.
use super::*;
use app::screenshot::Saved;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use std::sync::mpsc;

pub(super) struct Picker {
    texture: wgpu::Texture,
    selection: Option<mpsc::Sender<[u32; 2]>>,
    pub(super) displayed: Option<[u32; 2]>,
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

/// The presenter scales the complete game image to the physical window. Scale
/// both edges of each viewport so shared edges and split-screen gaps line up.
pub(super) fn output_rect(rect: [u32; 4], render: [u32; 2], window: [u32; 2]) -> [u32; 4] {
    let edge = |p: u32, axis: usize| {
        ((u64::from(p) * u64::from(window[axis]) + u64::from(render[axis]) / 2)
            / u64::from(render[axis])) as u32
    };
    let [x, y, w, h] = rect;
    let (left, top) = (edge(x, 0), edge(y, 1));
    [left, top, edge(x + w, 0) - left, edge(y + h, 1) - top]
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
    pub(super) fn new(texture: wgpu::Texture) -> Self {
        Self {
            texture,
            displayed: None,
            selection: None,
        }
    }

    fn frame_size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }

    pub(super) fn resized(&mut self) {
        self.displayed = None;
    }

    fn hud(size: [u32; 2]) -> HudDrawList {
        let mut hud = HudDrawList::new(size);
        let [w, h] = size.map(|v| v as f32);
        let clip = [0., 0., w, h];
        hud.text.push(TextRun {
            id: u64::MAX - 52,
            text: "Click or tap the problem   |   Esc to skip".into(),
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
}

fn save_point(mut saved: Saved, point: [u32; 2]) -> Result<(), String> {
    use std::io::Write;
    saved.metadata["player_point"] = serde_json::json!(point);
    let path = saved.png.with_extension("json");
    let tmp = path.with_extension("point.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut writer = std::io::BufWriter::new(&mut file);
        serde_json::to_writer_pretty(&mut writer, &saved.metadata)?;
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

impl App {
    pub(super) fn end_point_picker(&mut self, line: String) {
        self.point_picker = None;
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
            .and_then(|c| pixel(c, size, picker.frame_size()))
        else {
            return;
        };
        let Some(selection) = picker.selection.take() else {
            return;
        };
        let message = match selection.send(p) {
            Ok(()) => format!("Marked {}, {}", p[0], p[1]),
            Err(_) => "Could not save feedback point".into(),
        };
        self.end_point_picker(message);
    }

    pub(super) fn draw_point_picker(&mut self) -> Result<(), String> {
        if self.invisible() {
            return Ok(());
        }
        let g = self.graphics.as_mut().unwrap();
        self.point_picker.as_mut().unwrap().show(
            self.window.as_ref().unwrap(),
            &g.gpu,
            &g.presenter,
            &mut g.hud,
            &mut g.surface,
            &g.instance,
            &g.config,
        )
    }
}

impl Picker {
    pub(super) fn show(
        &mut self,
        window: &Arc<winit::window::Window>,
        gpu: &Gpu,
        presenter: &render::present::Presenter,
        hud: &mut Option<render::hud::HudRenderer>,
        surface: &mut wgpu::Surface<'static>,
        instance: &wgpu::Instance,
        config: &wgpu::SurfaceConfiguration,
    ) -> Result<(), String> {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let size = [size.width, size.height];
        if hud.is_none() {
            *hud = Some(render::hud::HudRenderer::new(gpu, config.format)?);
        }
        let hud = hud.as_mut().unwrap();
        hud.prepare(gpu, &Self::hud(size))?;
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                presenter.present_region(
                    gpu,
                    &self.texture,
                    image_rect(size, self.frame_size()),
                    &frame.texture.create_view(&Default::default()),
                    hud,
                )?;
                window.pre_present_notify();
                gpu.queue.present(frame);
                self.displayed = Some(size);
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                *surface = instance
                    .create_surface(window.clone())
                    .map_err(|e| e.to_string())?;
                surface.configure(&gpu.device, config);
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

// A single writer serializes clipboard updates and the feedback index. Dropping
// the picker releases an unmarked capture, including when the game closes.
struct Job {
    gpu: Gpu,
    texture: wgpu::Texture,
    metadata: serde_json::Value,
    visibility: Vec<render::frame::PendingVisibility>,
    render_size: [u32; 2],
    source: crate::drive_capture::VisibilitySource,
    point: mpsc::Receiver<[u32; 2]>,
}

impl Job {
    fn save(self) -> Result<(), String> {
        let size = [self.texture.width(), self.texture.height()];
        let rgba = self.gpu.read_rgba8(&self.texture);
        let mut metadata = self.metadata;
        let early_point = self.point.try_recv().ok();
        if let Some(p) = early_point {
            metadata["player_point"] = serde_json::json!(p);
        }
        let saved = app::screenshot::save_annotated(
            app::screenshot::Shot {
                title: "Sideways",
                size,
                rgba: &rgba,
                metadata,
            },
            |_| Ok(None),
            |png, meta| {
                let mut views = Vec::with_capacity(self.visibility.len());
                for pending in self.visibility {
                    let mut view = pending.read(&self.gpu)?;
                    view.rect = output_rect(view.rect, self.render_size, size);
                    views.push(view);
                }
                crate::drive_capture::write_visibility(png, meta, &views, &self.source)
            },
        )?;
        drop(rgba);
        drop(self.texture);
        drop(self.source);
        if early_point.is_none()
            && let Ok(p) = self.point.recv()
        {
            save_point(saved, p)?;
        }
        Ok(())
    }
}

pub(super) struct Writer {
    jobs: Option<mpsc::Sender<Job>>,
    errors: mpsc::Receiver<String>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    pub(super) fn new() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (errors, feedback) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("sideways-feedback".into())
            .spawn(move || {
                for job in rx {
                    if let Err(error) = job.save() {
                        eprintln!("feedback save: {error}");
                        let _ = errors.send(error);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            jobs: Some(tx),
            errors: feedback,
            thread: Some(thread),
        })
    }

    pub(super) fn submit(
        &self,
        picker: &mut Picker,
        gpu: &Gpu,
        metadata: serde_json::Value,
        visibility: Vec<render::frame::PendingVisibility>,
        render_size: [u32; 2],
        source: crate::drive_capture::VisibilitySource,
    ) -> Result<(), String> {
        let (send, point) = mpsc::channel();
        self.jobs
            .as_ref()
            .unwrap()
            .send(Job {
                gpu: gpu.clone(),
                texture: picker.texture.clone(),
                metadata,
                visibility,
                render_size,
                source,
                point,
            })
            .map_err(|_| "feedback writer stopped")?;
        picker.selection = Some(send);
        Ok(())
    }

    pub(super) fn error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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
        save_point(saved, [1, 0]).unwrap();
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
    #[ignore = "headless GPU feedback timing and color regression"]
    fn gpu_overlay_keeps_the_clean_capture() {
        let gpu = Gpu::new().unwrap();
        let size = [3840, 2160];
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let presenter = render::present::Presenter::new(&gpu, format);
        let mut hud = render::hud::HudRenderer::new(&gpu, format).unwrap();
        let scene = render::Renderer::output_texture(&gpu, size[0], size[1]);
        let color = [33, 76, 125, 255u8];
        let pixels = color.repeat((size[0] * size[1]) as usize);
        gpu.queue.write_texture(
            scene.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            scene.size(),
        );
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless overlay"),
            size: scene.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut timings = Vec::new();
        let mut picture = None;
        gpu.wait();
        for _ in 0..8 {
            let start = Instant::now();
            let frozen = presenter
                .freeze_sdr(&gpu, &scene, size, format, None)
                .unwrap();
            hud.prepare(&gpu, &Picker::hud(size)).unwrap();
            presenter
                .present_region(&gpu, &frozen, image_rect(size, size), &view, &hud)
                .unwrap();
            gpu.wait();
            timings.push(start.elapsed().as_secs_f64() * 1000.);
            picture = Some(frozen);
        }
        let clean = gpu.read_rgba8(&picture.unwrap());
        assert_eq!(
            clean.as_slice(),
            pixels.as_slice(),
            "SDR round trip changed screenshot pixels"
        );
        let overlay = gpu.read_rgba8(&target);
        let header = &overlay[..size[0] as usize * 32 * 4];
        assert!(
            header.chunks_exact(4).any(|px| px[0] > 180),
            "instruction should be visible"
        );
        assert_eq!(&overlay[4 * (1080 * 3840 + 1920)..][..4], &color);
        eprintln!("3840x2160 offscreen freeze + overlay, GPU completed, ms {timings:.2?}");
    }

    #[test]
    fn adaptive_split_edges_are_in_original_png_pixels() {
        let project = |r| output_rect(r, [1280, 720], [1920, 1080]);
        assert_eq!(project([0, 0, 1280, 358]), [0, 0, 1920, 537]);
        assert_eq!(project([0, 362, 1280, 358]), [0, 543, 1920, 537]);
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
