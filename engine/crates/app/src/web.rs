//! The shell in a browser (wasm32 with WebGPU): a canvas instead of a
//! window, the browser's frame callbacks instead of a render loop, and an
//! adapter and device that arrive asynchronously. A smaller [`crate::run`]:
//! fixed ticks, keyboard and mouse, the game's frame and HUD. No gamepads,
//! debug menu, screenshots, HDR or adaptive quality yet.

use crate::{Draw, Frame, Game, Output, Screen, input::Devices, seat_controls};
use render::{Gpu, Renderer};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use wasm_bindgen::JsCast;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Write a line to the browser console and to the page's log element
/// (`#warp-log`, when the page has one).
pub fn log(line: &str) {
    web_sys::console::log_1(&line.into());
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("warp-log")) {
        let text = el.text_content().unwrap_or_default();
        el.set_text_content(Some(&format!("{text}{line}\n")));
    }
}

/// Show loading progress in the page's `#warp-progress` box (a bar
/// `#warp-bar` and a line `#warp-step`), when it has one: `fraction` 0..1,
/// `None` hides the box. Also logged to the console.
pub fn progress(fraction: Option<f32>, step: &str) {
    web_sys::console::log_1(&format!("loading: {step}").into());
    let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
    if let Some(el) = document.get_element_by_id("warp-progress") {
        let _ = if fraction.is_some() { el.remove_attribute("hidden") } else { el.set_attribute("hidden", "") };
    }
    if let (Some(f), Some(bar)) = (fraction, document.get_element_by_id("warp-bar")) {
        let _ = bar.set_attribute("style", &format!("width: {:.1}%", f.clamp(0.0, 1.0) * 100.0));
    }
    if let Some(el) = document.get_element_by_id("warp-step") { el.set_text_content(Some(step)); }
}

/// Let the browser draw (the progress bar) before a long blocking step.
pub async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let window = web_sys::window().expect("window");
        // Two frames: the first lays out the change, the second paints it.
        let paint = wasm_bindgen::closure::Closure::once_into_js(move || {
            let window = web_sys::window().expect("window");
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0);
        });
        let _ = window.request_animation_frame(paint.unchecked_ref());
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Fetch `url` (relative to the page) as bytes.
pub async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window().ok_or("no window")?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(url)).await.map_err(|e| format!("{url}: {e:?}"))?
        .dyn_into().map_err(|_| format!("{url}: not a response"))?;
    if !response.ok() { return Err(format!("{url}: HTTP {}", response.status())); }
    let buffer = JsFuture::from(response.array_buffer().map_err(|e| format!("{url}: {e:?}"))?).await.map_err(|e| format!("{url}: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// The wasm module's linear memory, in MB (it only grows).
pub fn memory_mb() -> u32 {
    wasm_bindgen::memory().dyn_into::<js_sys::WebAssembly::Memory>().ok()
        .and_then(|m| m.buffer().dyn_into::<js_sys::ArrayBuffer>().ok()).map_or(0, |b| b.byte_length() >> 20)
}

/// Panics go to the console and the page, not into the void.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let line = format!("panic: {info}");
        web_sys::console::error_1(&line.clone().into());
        log(&line);
    }));
}

struct Live {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    out: Output,
    renderer: Option<Renderer>,
}

struct Shell {
    cfg: crate::Config,
    game: Box<dyn Game>,
    devices: Devices,
    window: Option<Arc<Window>>,
    /// Filled in by the asynchronous GPU setup.
    live: Rc<RefCell<Option<Live>>>,
    acc: f32,
    last: Instant,
    frames: u64,
    started: Instant,
    failed: bool,
    /// The scene upload was announced (see [`Shell::frame`]).
    announced: bool,
}

/// Run `game` in the page's canvas `#warp` (or a new one) until the page
/// closes. Returns at once: the browser drives the frames.
pub fn run(cfg: crate::Config, game: Box<dyn Game>) {
    install_panic_hook();
    let mut devices = Devices::new(cfg.keyboard);
    devices.pointer_enabled = cfg.pointer;
    let shell = Shell { cfg, game, devices, window: None, live: Rc::new(RefCell::new(None)), acc: 0.0, last: Instant::now(), frames: 0, started: Instant::now(), failed: false, announced: false };
    match EventLoop::new() {
        Ok(events) => events.spawn_app(shell),
        Err(e) => log(&format!("event loop: {e}")),
    }
}

impl Shell {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let canvas = document.get_element_by_id("warp").and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let attributes = Window::default_attributes().with_title(self.cfg.title.clone());
        let attributes = match canvas {
            Some(c) => attributes.with_canvas(Some(c)),
            None => attributes.with_append(true),
        };
        let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
        self.window = Some(window.clone());
        let live = self.live.clone();
        progress(Some(0.8), "Starting WebGPU");
        wasm_bindgen_futures::spawn_local(async move {
            match setup(window.clone()).await {
                Ok(l) => {
                    log(&format!("GPU: {} ({:?}), {}x{}, {:?}", l.gpu.info.name, l.gpu.info.backend, l.config.width, l.config.height, l.config.format));
                    // The renderer's scene and cull layouts need 16 (browser.md).
                    let storage = l.gpu.device.limits().max_storage_buffers_per_shader_stage;
                    if storage < 16 {
                        let agent = web_sys::window().and_then(|w| w.navigator().user_agent().ok()).unwrap_or_default();
                        progress(Some(0.8), &format!("This GPU offers {storage} storage buffers per shader stage; the renderer needs 16. Try the latest Chrome or Edge on a desktop GPU.\n{agent}"));
                        return;
                    }
                    *live.borrow_mut() = Some(l);
                    window.request_redraw();
                }
                Err(e) => { progress(Some(0.8), &format!("WebGPU could not start: {e}")); log(&format!("GPU setup failed: {e}")) }
            }
        });
        Ok(())
    }

    fn frame(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.game.tuning(&mut self.devices.tuning);
        let step = 1.0 / self.cfg.tick_hz;
        self.acc += dt;
        let mut steps = 0;
        let mut live = self.live.borrow_mut();
        let Some(live) = live.as_mut() else { return Ok(()) };
        // The canvas takes its size from the page's layout, which can settle
        // after the device is made: follow it every frame.
        if let Some(w) = &self.window {
            let s = w.inner_size();
            if s.width > 0 && s.height > 0 && [s.width, s.height] != [live.config.width, live.config.height] {
                live.config.width = s.width;
                live.config.height = s.height;
                live.surface.configure(&live.gpu.device, &live.config);
                log(&format!("canvas {}x{}", s.width, s.height));
            }
        }
        let size = [live.config.width, live.config.height];
        while self.acc >= step && steps < 6 {
            self.acc -= step;
            steps += 1;
            self.game.pointer(self.devices.pointer(), size);
            let devices = self.devices.device_controls(step);
            let controls = seat_controls(self.game.as_mut(), &devices);
            self.game.tick(&controls);
        }
        if steps == 6 { self.acc = 0.0; }
        let alpha = (self.acc / step).clamp(0.0, 1.0);
        let split = self.game.split();
        // The first scene upload blocks for seconds: show it first.
        if live.renderer.is_none() && !self.announced {
            self.announced = true;
            progress(Some(0.9), "Uploading the world to the GPU");
            return Ok(());
        }
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha, split, scene_fps: if dt > 0.0 { 1.0 / dt } else { 0.0 }, scene_worst_ms: dt * 1000.0, renderer: &mut live.renderer };
        let draw: Draw = self.game.draw(&mut frame)?;
        let Some(renderer) = live.renderer.as_ref() else { return Ok(()) };
        let texture = match live.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                live.surface.configure(&live.gpu.device, &live.config);
                return Err(format!("canvas texture: {other:?}"));
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(live.config.format.add_srgb_suffix()), ..Default::default() });
        live.out.draw(&live.gpu, renderer, &draw, size, split, &view, self.cfg.max_render_height)?;
        live.gpu.queue.present(texture);
        self.frames += 1;
        if self.frames == 1 { progress(None, "Ready"); }
        if self.frames == 1 || self.frames.is_power_of_two() && self.frames >= 64 {
            log(&format!("frame {} after {:.1} s, wasm memory {} MB", self.frames, self.started.elapsed().as_secs_f32(), memory_mb()));
        }
        Ok(())
    }

    fn capture_mouse(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        self.devices.reset_mouse();
        if on {
            let ok = w.set_cursor_grab(CursorGrabMode::Locked).is_ok();
            w.set_cursor_visible(!ok);
            self.devices.mouse_captured = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.devices.mouse_captured = false;
        }
    }
}

async fn setup(window: Arc<Window>) -> Result<Live, String> {
    let instance = render::gpu::instance();
    let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
    let size = window.inner_size();
    let (gpu, config, _adapter) = Gpu::for_surface_async(&instance, &surface, size.width.max(1), size.height.max(1)).await?;
    surface.configure(&gpu.device, &config);
    gpu.device.on_uncaptured_error(Arc::new(|error: wgpu::Error| log(&format!("GPU error: {error}"))));
    gpu.device.set_device_lost_callback(|reason, message| log(&format!("GPU device lost ({reason:?}): {message}")));
    let out = Output::new(&gpu, config.format.add_srgb_suffix())?;
    Ok(Live { surface, config, gpu, out, renderer: None })
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        if let Err(e) = self.open(event_loop) { log(&format!("canvas: {e}")); }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event { self.devices.mouse_motion(delta.0, delta.1); }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(l) = self.live.borrow_mut().as_mut() {
                    l.config.width = size.width.max(1);
                    l.config.height = size.height.max(1);
                    l.surface.configure(&l.gpu.device, &l.config);
                }
            }
            WindowEvent::Focused(false) => { self.capture_mouse(false); self.devices.cursor(None); }
            WindowEvent::CursorMoved { position, .. } => self.devices.cursor(Some([position.x as f32, position.y as f32])),
            WindowEvent::CursorLeft { .. } => self.devices.cursor(None),
            WindowEvent::MouseWheel { delta, .. } => self.devices.wheel(match delta {
                winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
            }),
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left && state == ElementState::Pressed && !self.devices.mouse_captured && self.cfg.keyboard && self.game.mouse_capture().is_none() {
                    self.capture_mouse(true);
                    return;
                }
                self.devices.mouse_button(button, state);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                use KeyCode::*;
                if let Some(n) = [F1, F2, F3, F4, F5, F6, F7, F8, F9].iter().position(|&k| k == code) {
                    if event.state == ElementState::Pressed && !event.repeat { self.game.function_key(n as u8 + 1); }
                    return;
                }
                if code == Escape && event.state == ElementState::Pressed && !event.repeat {
                    if !self.game.escape() { self.capture_mouse(false); }
                    return;
                }
                self.devices.key(code, event.state, event.repeat);
            }
            WindowEvent::RedrawRequested => {
                if self.failed { return; }
                if self.game.screen() == Screen::Hidden { return; }
                if let Err(e) = self.frame() {
                    progress(Some(1.0), &format!("Stopped: {e}"));
                    log(&format!("frame error: {e}"));
                    self.failed = true;
                    return;
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }
            _ => {}
        }
    }
}
