//! Frozen point selection and ordered screenshot saving for Warp desktop games.
use crate::screenshot::{self, Destination, Saved};
use render::Gpu;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use std::sync::mpsc;
use std::{path::Path, sync::Arc};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

pub struct Picker {
    texture: wgpu::Texture,
    selection: Option<Ticket>,
    hdr_overlay: Option<(render::present::Presenter, wgpu::Texture)>,
    displayed: Option<[u32; 2]>,
    hdr: Option<HdrFrame>,
}

/// The frame as an HDR screen showed it: the renderer's HDR output (copied),
/// the game's HUD over it, and the SDR white it was shown at.
pub struct HdrFrame {
    pub texture: wgpu::Texture,
    pub hud: HudDrawList,
    pub sdr_white_nits: f32,
    pub peak_nits: Option<f32>,
}

/// Where the frozen frame is shown: where it was, filling the window as the
/// game did (the hint is drawn over it), so F4 does not move or rescale it.
fn image_rect(size: [u32; 2], frame: [u32; 2]) -> [f32; 4] {
    let [w, h] = size.map(|v| v as f32);
    let scale = (w / frame[0] as f32).min(h / frame[1] as f32);
    let (iw, ih) = (
        (frame[0] as f32 * scale).round(),
        (frame[1] as f32 * scale).round(),
    );
    [(w - iw) * 0.5, (h - ih) * 0.5, iw, ih]
}

/// The presenter scales the complete game image to the physical window. Scale
/// both edges of each viewport so shared edges and split-screen gaps line up.
fn output_rect(rect: [u32; 4], render: [u32; 2], window: [u32; 2]) -> [u32; 4] {
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
    pub fn new(texture: wgpu::Texture) -> Self {
        Self {
            texture,
            displayed: None,
            selection: None,
            hdr_overlay: None,
            hdr: None,
        }
    }

    /// On an HDR screen: show the frozen frame as it was shown (its HDR
    /// output under the game's HUD) and save it as an HDR file too.
    pub fn with_hdr(mut self, frame: HdrFrame) -> Self {
        self.hdr = Some(frame);
        self
    }

    pub(crate) fn read_rgba(&self, gpu: &Gpu) -> Vec<u8> {
        gpu.read_rgba8(&self.texture)
    }

    pub fn displayed(&self) -> bool {
        self.displayed.is_some()
    }

    /// Consume modal input before passing events to the game or its menus.
    pub fn input(
        &mut self,
        event: &WindowEvent,
        cursor: Option<[f64; 2]>,
        size: [u32; 2],
    ) -> Input {
        let position = match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && event.state == ElementState::Pressed
                    && !event.repeat
                {
                    return Input::Close("Screenshot without a point".into());
                }
                return Input::Consumed;
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => cursor,
            WindowEvent::Touch(t) if t.phase == winit::event::TouchPhase::Started => {
                Some([t.location.x, t.location.y])
            }
            WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::Ime(_) => return Input::Consumed,
            _ => return Input::Pass,
        };
        if self.displayed == Some(size)
            && let Some(p) = position.and_then(|c| pixel(c, size, self.frame_size()))
            && let Some(ticket) = self.selection.take()
        {
            return Input::Close(if ticket.mark(p).is_ok() {
                format!("Marked {}, {}", p[0], p[1])
            } else {
                "Feedback save failed".into()
            });
        }
        Input::Consumed
    }

    pub(crate) fn into_crop(mut self, gpu: &Gpu) -> Result<crate::Crop, String> {
        let saved = self
            .selection
            .take()
            .ok_or("feedback unavailable")?
            .saved
            .recv()
            .map_err(|_| "feedback save failed")?;
        let size = self.frame_size();
        let picture =
            render::picture::Picture::new(size[0], size[1], gpu.read_rgba8(&self.texture))?;
        Ok(crate::Crop {
            saved,
            size,
            picture: Arc::new(picture),
            start: None,
            cursor: [0.; 2],
        })
    }

    fn frame_size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }

    pub fn resized(&mut self) {
        self.displayed = None;
    }

    fn hud(size: [u32; 2]) -> HudDrawList {
        let mut hud = HudDrawList::new(size);
        let [w, h] = size.map(|v| v as f32);
        let clip = [0., 0., w, h];
        // Over the frame, which stays where it was: a band to read it on.
        hud.rect([0., 0., w, 32.0_f32.min(h / 3.0)], [0., 0., 0., 0.6], clip);
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

impl Picker {
    pub fn show(
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
            *hud = Some(render::hud::HudRenderer::new(
                gpu,
                presenter.hud_format(config.format),
            )?);
        }
        let hud = hud.as_mut().unwrap();
        let hint = Self::hud(size);
        match &self.hdr {
            Some(frame) if presenter.is_hdr() => {
                // The game's HUD as it was, the hint over it.
                let mut list = frame.hud.clone();
                list.begin_overlay();
                list.text.extend(hint.text);
                hud.prepare(gpu, &list)?
            }
            _ => hud.prepare(gpu, &hint)?,
        };
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let target = frame.texture.create_view(&Default::default());
                let rect = image_rect(size, self.frame_size());
                if let (true, Some(frame)) = (presenter.is_hdr(), &self.hdr) {
                    // As the live frame was presented: the HDR output, the
                    // HUD at SDR white.
                    presenter.present_hdr(gpu, &self.texture, Some(&frame.texture), &target, Some(hud), None)?;
                } else if presenter.is_hdr() {
                    // First compose the picker in SDR, then display it at the HUD's SDR white.
                    let format = render::present::HDR_HUD_FORMAT;
                    if self
                        .hdr_overlay
                        .as_ref()
                        .is_none_or(|(_, t)| [t.width(), t.height()] != size)
                    {
                        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("SDR feedback on HDR display"),
                            size: wgpu::Extent3d {
                                width: size[0],
                                height: size[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: format.remove_srgb_suffix(),
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[format],
                        });
                        self.hdr_overlay =
                            Some((render::present::Presenter::new(gpu, format), texture));
                    }
                    let (sdr, overlay) = self.hdr_overlay.as_ref().unwrap();
                    let view = overlay.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(format),
                        ..Default::default()
                    });
                    sdr.present_region(gpu, &self.texture, rect, &view, hud)?;
                    presenter.present_hdr(gpu, overlay, None, &target, None, None)?;
                } else {
                    presenter.present_region(gpu, &self.texture, rect, &target, hud)?;
                }
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

pub enum Input {
    Pass,
    Consumed,
    Close(String),
}

struct Ticket {
    jobs: mpsc::Sender<Work>,
    saved: mpsc::Receiver<Saved>,
    point: Arc<std::sync::OnceLock<[u32; 2]>>,
}

impl Ticket {
    fn mark(self, p: [u32; 2]) -> Result<(), String> {
        let _ = self.point.set(p);
        self.jobs
            .send(Work::Mark(self.saved, p))
            .map_err(|_| "feedback writer stopped".into())
    }
}

enum Work {
    Save(Job, mpsc::Sender<Saved>),
    Mark(mpsc::Receiver<Saved>, [u32; 2]),
    Stop,
}
type Attachment = Box<dyn FnOnce(&Gpu, &Path, &mut serde_json::Value) -> Result<(), String> + Send>;
struct Job {
    title: String,
    destination: Destination,
    gpu: Gpu,
    texture: wgpu::Texture,
    metadata: serde_json::Value,
    replay: Result<Option<serde_json::Value>, String>,
    annotate: Attachment,
    point: Arc<std::sync::OnceLock<[u32; 2]>>,
    /// On an HDR screen: its HDR output, SDR white and peak (nits).
    hdr: Option<(wgpu::Texture, f32, Option<f32>)>,
}

/// Beside `png` (the SDR picture): `<name>-hdr.png`, the frame as the HDR
/// screen showed it, in BT.2100 PQ.
fn save_hdr(gpu: &Gpu, png: &Path, (texture, white, peak): &(wgpu::Texture, f32, Option<f32>), meta: &mut serde_json::Value) -> Result<(), String> {
    let stem = png.file_stem().and_then(|s| s.to_str()).unwrap_or("screenshot");
    let path = png.with_file_name(format!("{stem}-hdr.png"));
    render::hdr::write_pq_png(&path, texture.width(), texture.height(), &gpu.read_rgba16f(texture), *white)?;
    meta["capture"]["hdr_png"] = serde_json::json!({
        "png": path, "size": [texture.width(), texture.height()],
        "encoding": "16-bit PNG, BT.2100 PQ, BT.2020 primaries (cICP 9/16/0/1), no HUD",
        "sdr_white_nits": white, "display_peak_nits": peak,
    });
    meta["capture"]["pixels"] = serde_json::json!("the frame's SDR tonemap including the HUD (what an SDR screen shows; the HDR screen showed hdr_png), sRGB RGBA8");
    Ok(())
}

impl Job {
    fn save(self) -> Result<Saved, String> {
        let size = [self.texture.width(), self.texture.height()];
        let rgba = self.gpu.read_rgba8(&self.texture);
        let (gpu, hdr) = (self.gpu.clone(), self.hdr);
        screenshot::save_at(
            self.destination,
            screenshot::Shot {
                title: &self.title,
                size,
                rgba: &rgba,
                metadata: self.metadata,
            },
            |_| self.replay,
            |png, meta| {
                let hdr_result = hdr.as_ref().map_or(Ok(()), |h| save_hdr(&gpu, png, h, meta));
                let result = (self.annotate)(&self.gpu, png, meta).and(hdr_result);
                if let Some(p) = self.point.get() {
                    meta["player_point"] = serde_json::json!(p);
                }
                result
            },
        )
    }
}

/// Ordered writer for one game loop. A picker can outlive the writer without blocking shutdown.
pub struct Writer {
    jobs: mpsc::Sender<Work>,
    errors: mpsc::Receiver<String>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Writer {
    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Result<Self, String> {
        Err("Local feedback files require a desktop host".into())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Result<Self, String> {
        let (jobs, rx) = mpsc::channel();
        let (error_tx, errors) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("warp-feedback".into())
            .spawn(move || {
                for work in rx {
                    let result = match work {
                        Work::Save(job, reply) => job.save().map(|saved| {
                            let _ = reply.send(saved);
                        }),
                        Work::Mark(reply, p) => match reply.recv() {
                            Ok(saved) if saved.metadata["player_point"] != serde_json::json!(p) => {
                                save_point(saved, p)
                            }
                            _ => Ok(()),
                        },
                        Work::Stop => break,
                    };
                    if let Err(e) = result {
                        eprintln!("feedback save: {e}");
                        let _ = error_tx.send(e);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            jobs,
            errors,
            thread: Some(thread),
        })
    }

    /// Snapshot the optional replay on the game thread. The attachment callback runs on the writer.
    pub fn submit(
        &self,
        picker: &mut Picker,
        gpu: &Gpu,
        title: &str,
        metadata: serde_json::Value,
        replay: impl FnOnce(&Path) -> Result<Option<serde_json::Value>, String>,
        annotate: impl FnOnce(&Gpu, &Path, &mut serde_json::Value) -> Result<(), String>
        + Send
        + 'static,
    ) -> Result<(), String> {
        let (reply, saved) = mpsc::channel();
        let point = Arc::new(std::sync::OnceLock::new());
        let destination = Destination::new(title)?;
        let replay = replay(&destination.png);
        self.jobs
            .send(Work::Save(
                Job {
                    title: title.into(),
                    destination,
                    gpu: gpu.clone(),
                    texture: picker.texture.clone(),
                    metadata,
                    replay,
                    annotate: Box::new(annotate),
                    point: point.clone(),
                    hdr: picker.hdr.as_ref().map(|f| (f.texture.clone(), f.sdr_white_nits, f.peak_nits)),
                },
                reply,
            ))
            .map_err(|_| "feedback writer stopped")?;
        picker.selection = Some(Ticket {
            jobs: self.jobs.clone(),
            saved,
            point,
        });
        Ok(())
    }

    pub fn error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.jobs.send(Work::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Resolve queued GPU copies into original window coordinates.
pub fn read_visibility(
    gpu: &Gpu,
    pending: Vec<render::frame::PendingVisibility>,
    render_size: [u32; 2],
    window_size: [u32; 2],
) -> Result<Vec<render::frame::VisibilityCapture>, String> {
    pending
        .into_iter()
        .map(|copy| {
            let mut view = copy.read(gpu)?;
            view.rect = output_rect(view.rect, render_size, window_size);
            Ok(view)
        })
        .collect()
}

/// Write per-view IDs and owner/cluster tables beside a feedback PNG.
pub fn write_visibility(
    png: &Path,
    meta: &mut serde_json::Value,
    captures: &[render::frame::VisibilityCapture],
    mut source: serde_json::Value,
) -> Result<(), String> {
    use serde_json::json;
    use std::{
        fs,
        io::{BufWriter, Write},
    };
    let mut views = Vec::new();
    for (index, v) in captures.iter().enumerate() {
        let ids = png.with_extension(format!("view{index}.ids.png"));
        let table = png.with_extension(format!("view{index}.pairs"));
        render::write_png(&ids, v.size[0], v.size[1], &v.ids)?;
        let mut file = BufWriter::new(fs::File::create(&table).map_err(|e| e.to_string())?);
        for &[owner, cluster] in &v.visible {
            for word in [owner, cluster] {
                file.write_all(&word.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        views.push(json!({"rect":v.rect,"size":v.size,"ids":ids.file_name().and_then(|n|n.to_str()),"pairs":table.file_name().and_then(|n|n.to_str())}));
    }
    source["views"] = json!(views);
    meta["visibility"] = source;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
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
    #[ignore = "isolated feedback writer test; requires a headless GPU"]
    fn desktop_point_crop_and_shutdown() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("warp-feedback-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("WARP_SCREENSHOTS", &dir);
            std::env::set_var("WARP_LIVE_DIR", &dir);
        }
        crate::clipboard::memory_only();
        let gpu = Gpu::new().unwrap();
        let writer = Writer::new().unwrap();
        let shot = |title: &str| {
            let mut picker = Picker::new(render::Renderer::output_texture(&gpu, 24, 12));
            let mut path = None;
            writer
                .submit(
                    &mut picker,
                    &gpu,
                    title,
                    serde_json::json!({"state":42}),
                    |png| {
                        path = Some(png.with_extension("json"));
                        Ok(Some(serde_json::json!({"tick":123})))
                    },
                    |_, _, meta| {
                        meta["extra"] = true.into();
                        Ok(())
                    },
                )
                .unwrap();
            picker.displayed = Some([24, 44]);
            (picker, path.unwrap())
        };
        let (mut first, path) = shot("Example");
        let touch = WindowEvent::Touch(winit::event::Touch {
            device_id: winit::event::DeviceId::dummy(),
            phase: winit::event::TouchPhase::Started,
            location: winit::dpi::PhysicalPosition::new(12., 20.),
            force: None,
            id: 1,
        });
        let expected = pixel([12., 20.], [24, 44], [24, 12]).unwrap();
        assert!(matches!(
            first.input(&touch, None, [24, 44]),
            Input::Close(_)
        ));
        let (second, _) = shot("Crop");
        let crop = second.into_crop(&gpu).unwrap();
        assert_eq!(crop.saved.metadata["replay"]["tick"], 123);
        assert_eq!(crop.size, [24, 12]);
        let (_open, unmarked) = shot("Open");
        // On an HDR screen: the HDR output saved beside the SDR picture.
        let hdr = gpu.device.create_texture(&wgpu::TextureDescriptor { label: None,
            size: wgpu::Extent3d { width: 24, height: 12, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        // 4.0 (a highlight at four times SDR white) in every channel.
        gpu.queue.write_texture(hdr.as_image_copy(), &[0x00u8, 0x44].repeat(24 * 12 * 4),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(24 * 8), rows_per_image: None }, hdr.size());
        let mut picker = Picker::new(render::Renderer::output_texture(&gpu, 24, 12))
            .with_hdr(HdrFrame { texture: hdr, hud: HudDrawList::new([24, 12]), sdr_white_nits: 200.0, peak_nits: Some(1000.0) });
        let mut hdr_json = None;
        writer.submit(&mut picker, &gpu, "Hdr", serde_json::json!({}), |png| { hdr_json = Some(png.with_extension("json")); Ok(None) }, |_, _, _| Ok(())).unwrap();
        drop(writer);
        let meta: serde_json::Value = serde_json::from_slice(&fs::read(hdr_json.unwrap()).unwrap()).unwrap();
        let hdr_png = meta["capture"]["hdr_png"]["png"].as_str().unwrap();
        let bytes = fs::read(hdr_png).unwrap();
        assert!(bytes.windows(8).any(|w| w == b"cICP\x09\x10\x00\x01"), "BT.2020 PQ full-range cICP chunk");
        let image::DynamicImage::ImageRgb16(pixels) = image::load_from_memory(&bytes).unwrap() else { panic!("16-bit RGB") };
        // 4 x 200 nits = 800 nits, grey stays grey through the primaries.
        let code = pixels.get_pixel(0, 0)[0] as f32 / 65535.0;
        assert!((code - render::hdr::pq_encode(800.0)).abs() < 1e-3, "{code}");
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["player_point"], serde_json::json!(expected));
        assert_eq!(saved["extra"], true);
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(unmarked).unwrap()).unwrap();
        assert!(saved.get("player_point").is_none());
        fs::remove_dir_all(dir).unwrap();
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
        assert_eq!(pixel([0., 36.], size, frame), Some([0, 0]));
        assert_eq!(pixel([119.99, 95.99], size, frame), Some([199, 99]));
        assert_eq!(pixel([60., 66.], size, frame), Some([100, 50]));
        assert_eq!(pixel([60., 20.], size, frame), None);
        assert_eq!(pixel([60., 96.], size, frame), None);
    }

    #[test]
    fn frozen_frame_stays_where_the_game_drew_it() {
        assert_eq!(image_rect([1920, 1080], [1920, 1080]), [0., 0., 1920., 1080.]);
        // A capped render height is shown scaled to the window, as live.
        assert_eq!(image_rect([3840, 2160], [1920, 1080]), [0., 0., 3840., 2160.]);
    }
}
