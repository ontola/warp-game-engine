//! A game shell for Warp: the window, surface and presentation, a fixed
//! simulation tick with interpolation, local player input, split-screen
//! frames with a HUD, and the same frame path headless for screenshots and
//! benchmarks. A game implements [`Game`]; the shell owns the GPU.
//!
//! Built-in keys: click captures the mouse, Esc releases it (or quits when
//! released), F11 toggles fullscreen, F12 saves `screenshot-N.png`.
pub mod input;
pub mod script;
pub mod sound;
pub mod ui;

pub use input::{Buttons, Controls, Devices, Tuning};
use render::hud::{HudDrawList, HudRenderer};
use render::present::Presenter;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

/// What a game draws this frame.
pub struct Draw {
    /// One camera per view, in split-screen order (see [`render::split_rects`]).
    pub cameras: Vec<Camera>,
    pub light: Lighting,
    pub hud: HudDrawList,
}

/// Per-frame access to the renderer for the game.
pub struct Frame<'a> {
    pub gpu: &'a Gpu,
    /// Output size in pixels.
    pub size: [u32; 2],
    /// Seconds since the previous frame.
    pub dt: f32,
    /// Interpolation between the previous and the latest tick, 0..1.
    pub alpha: f32,
    renderer: &'a mut Option<Renderer>,
}

impl Frame<'_> {
    /// Upload a scene: creates the renderer, or replaces it.
    pub fn set_scene(&mut self, scene: &Scene, light: &Lighting) {
        *self.renderer = Some(Renderer::new(self.gpu, scene, light));
    }
    pub fn renderer(&self) -> Option<&Renderer> { self.renderer.as_ref() }
    /// Split-screen rectangles for `views` views, as x, y, width, height.
    pub fn rects(&self, views: usize) -> Vec<[f32; 4]> {
        render::split_rects(views, self.size[0], self.size[1]).into_iter().map(|(x, y, w, h)| [x as f32, y as f32, w as f32, h as f32]).collect()
    }
}

pub trait Game {
    /// Local players whose controls the shell gathers.
    fn local_players(&self) -> usize;
    /// One fixed simulation step.
    fn tick(&mut self, controls: &[Controls]);
    /// Pose the scene and describe the frame. Called once per displayed frame,
    /// after zero or more ticks. Create the scene with `frame.set_scene`.
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String>;
    /// A line for the window title, if any.
    fn title(&self) -> Option<String> { None }
    /// The game's state as JSON, for scripted playtests and agents.
    fn state(&self) -> Option<String> { None }
    /// A debug command from a script or console (`give 0 explosive`,
    /// `teleport 0 1 2 3`): set up scenarios without playing up to them.
    /// `help` should list what the game understands.
    fn command(&mut self, cmd: &str) -> Result<String, String> { Err(format!("this game has no commands ({cmd})")) }
    /// How the window should be: the shell follows changes every frame
    /// (a party host keeps a warm game hidden until its match starts).
    fn screen(&self) -> Screen { Screen::Normal }
}

/// What the game wants from its window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    /// As configured (windowed or fullscreen).
    #[default]
    Normal,
    /// Minimised, drawing a few frames a second, still ticking.
    Hidden,
    /// Shown, borderless fullscreen and focused.
    Fullscreen,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub title: String,
    pub size: [u32; 2],
    pub tick_hz: f32,
    pub fullscreen: bool,
    pub keyboard: bool,
    /// Smoke test: quit after this many frames and print the frame rate.
    pub frames: u64,
}

impl Default for Config {
    fn default() -> Self { Self { title: "Warp".into(), size: [1600, 900], tick_hz: 60.0, fullscreen: false, keyboard: true, frames: 0 } }
}

/// The present path shared by the window and headless capture.
struct Output { presenter: Presenter, hud: HudRenderer, picture: Option<wgpu::Texture> }

impl Output {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Result<Self, String> {
        Ok(Self { presenter: Presenter::new(gpu, format), hud: HudRenderer::new(gpu, format)?, picture: None })
    }
    fn draw(&mut self, gpu: &Gpu, renderer: &Renderer, draw: &Draw, size: [u32; 2], target: &wgpu::TextureView) -> Result<(), String> {
        let rects = render::split_rects(draw.cameras.len(), size[0], size[1]);
        let views: Vec<ViewRect> = draw.cameras.iter().zip(&rects).map(|(&cam, &(x, y, w, h))| ViewRect { cam, x, y, w, h }).collect();
        // One output texture, reused while the size holds.
        if self.picture.as_ref().is_none_or(|p| [p.width(), p.height()] != size) { self.picture = Some(Renderer::output_texture(gpu, size[0], size[1])); }
        let picture = self.picture.as_ref().unwrap();
        renderer.try_render_into(gpu, &views, &draw.light, picture)?;
        self.hud.prepare(gpu, &draw.hud)?;
        self.presenter.present(gpu, picture, target, Some(&self.hud), None)
    }
}

fn offscreen(gpu: &Gpu, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("capture"), size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    })
}

/// A game without a window: tick it with any controls and render frames to
/// memory through the same path as the window. Keeps its renderer between
/// frames. `KERN_ADAPTER=software` runs it on a CPU rasteriser.
pub struct Headless<'g> {
    pub gpu: &'g Gpu,
    pub size: [u32; 2],
    pub tick_hz: f32,
    /// Offline sound: scripts mix one tick of audio per tick into `recorded`.
    pub audio: Option<Box<dyn audio::Source>>,
    pub recorded: Vec<f32>,
    renderer: Option<Renderer>,
    out: Output,
    target: wgpu::Texture,
}

impl<'g> Headless<'g> {
    pub fn new(gpu: &'g Gpu, size: [u32; 2], tick_hz: f32) -> Result<Self, String> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        Ok(Self { gpu, size, tick_hz, audio: None, recorded: Vec::new(), renderer: None, out: Output::new(gpu, format)?, target: offscreen(gpu, size, format) })
    }
    /// Draw one frame and read it back as RGBA8.
    pub fn frame(&mut self, game: &mut dyn Game) -> Result<Vec<u8>, String> {
        let mut frame = Frame { gpu: self.gpu, size: self.size, dt: 1.0 / self.tick_hz, alpha: 1.0, renderer: &mut self.renderer };
        let draw = game.draw(&mut frame)?;
        let renderer = self.renderer.as_ref().ok_or("the game drew without a scene")?;
        self.out.draw(self.gpu, renderer, &draw, self.size, &self.target.create_view(&Default::default()))?;
        Ok(self.gpu.read_rgba8(&self.target))
    }
    pub fn renderer(&self) -> Option<&Renderer> { self.renderer.as_ref() }
}

/// Headless: run `ticks` steps with idle controls, draw one frame and return
/// it as RGBA8. No window is created.
pub fn capture(game: &mut dyn Game, gpu: &Gpu, size: [u32; 2], ticks: u32, tick_hz: f32) -> Result<Vec<u8>, String> {
    let players = game.local_players();
    for _ in 0..ticks { game.tick(&vec![Controls::default(); players]); }
    Headless::new(gpu, size, tick_hz)?.frame(game)
}

struct Live {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    out: Output,
    renderer: Option<Renderer>,
}

struct Shell<'g> {
    cfg: Config,
    game: &'g mut dyn Game,
    devices: Devices,
    live: Option<Live>,
    window: Option<Arc<Window>>,
    screen: Screen,
    acc: f32,
    last: Instant,
    started: Instant,
    frames: u64,
    screenshot: bool,
    shots: u32,
    error: Option<String>,
}

/// Open a window and run the game until it closes.
pub fn run(cfg: Config, game: &mut dyn Game) -> Result<(), String> {
    let devices = Devices::new(cfg.keyboard);
    println!("{}: {} local players, {} gamepads", cfg.title, game.local_players(), devices.pad_count());
    let mut shell = Shell { devices, cfg, game, live: None, window: None, screen: Screen::Normal, acc: 0.0, last: Instant::now(), started: Instant::now(), frames: 0, screenshot: false, shots: 0, error: None };
    let events = EventLoop::new().map_err(|e| e.to_string())?;
    events.run_app(&mut shell).map_err(|e| e.to_string())?;
    shell.error.map_or(Ok(()), Err)
}

impl Shell<'_> {
    fn frame(&mut self) -> Result<(), String> {
        let wish = self.game.screen();
        if wish != self.screen {
            self.screen = wish;
            if wish == Screen::Hidden { self.capture_mouse(false); }
            if let Some(w) = &self.window {
                match wish {
                    Screen::Hidden => w.set_minimized(true),
                    Screen::Fullscreen => { w.set_minimized(false); w.set_fullscreen(Some(Fullscreen::Borderless(None))); w.focus_window(); }
                    Screen::Normal => { w.set_minimized(false); w.set_fullscreen(self.cfg.fullscreen.then_some(Fullscreen::Borderless(None))); }
                }
            }
        }
        // Hidden: keep ticking (messages, warm loading) at a few frames a second.
        if self.screen == Screen::Hidden { std::thread::sleep(std::time::Duration::from_millis(50)); }
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.devices.poll();
        let step = 1.0 / self.cfg.tick_hz;
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= step && steps < 6 {
            self.acc -= step;
            steps += 1;
            let controls = self.devices.controls(self.game.local_players(), step);
            self.game.tick(&controls);
        }
        if steps == 6 { self.acc = 0.0; }
        let Some(live) = &mut self.live else { return Ok(()) };
        let size = [live.config.width, live.config.height];
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha: (self.acc / step).clamp(0.0, 1.0), renderer: &mut live.renderer };
        let mut draw = self.game.draw(&mut frame)?;
        let Some(renderer) = &live.renderer else { return Err("the game drew without a scene".into()) };
        if !self.devices.mouse_captured && self.cfg.keyboard {
            let s = (size[1] as f32 / 1080.0).max(0.5);
            draw.hud.text.push(render::hud::TextRun { id: u64::MAX - 1, text: "click to capture the mouse, Esc releases".into(), position: [16.0 * s, size[1] as f32 - 34.0 * s],
                bounds: [0.0, 0.0, size[0] as f32, size[1] as f32], font_size: 18.0 * s, color: [220, 220, 215, 200], font: render::hud::FontFace::Body,
                align: render::hud::TextAlign::Left, width: size[0] as f32 * 0.6 });
        }
        if std::mem::take(&mut self.screenshot) {
            let target = offscreen(&live.gpu, size, wgpu::TextureFormat::Rgba8UnormSrgb);
            Output::new(&live.gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?.draw(&live.gpu, renderer, &draw, size, &target.create_view(&Default::default()))?;
            self.shots += 1;
            let path = format!("screenshot-{}.png", self.shots);
            render::write_png(std::path::Path::new(&path), size[0], size[1], &live.gpu.read_rgba8(&target))?;
            println!("saved {path}");
        }
        match live.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => {
                live.out.draw(&live.gpu, renderer, &draw, size, &tex.texture.create_view(&Default::default()))?;
                if let Some(w) = &self.window { w.pre_present_notify(); }
                live.gpu.queue.present(tex);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => live.surface.configure(&live.gpu.device, &live.config),
            _ => {}
        }
        Ok(())
    }

    fn capture_mouse(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        if on {
            let ok = w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
            w.set_cursor_visible(!ok);
            self.devices.mouse_captured = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.devices.mouse_captured = false;
        }
    }
}

impl ApplicationHandler for Shell<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let result = (|| -> Result<(), String> {
            let attrs = Window::default_attributes().with_title(&self.cfg.title).with_inner_size(PhysicalSize::new(self.cfg.size[0], self.cfg.size[1]))
                .with_fullscreen(self.cfg.fullscreen.then_some(Fullscreen::Borderless(None)));
            let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
            let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
            let size = window.inner_size();
            let (gpu, config) = Gpu::for_surface(&instance, &surface, size.width, size.height)?;
            surface.configure(&gpu.device, &config);
            let out = Output::new(&gpu, config.format)?;
            println!("GPU {}, {}x{}", gpu.info.name, size.width, size.height);
            self.live = Some(Live { surface, config, gpu, out, renderer: None });
            self.window = Some(window.clone());
            self.last = Instant::now();
            self.started = Instant::now();
            window.request_redraw();
            Ok(())
        })();
        if let Err(e) = result { self.error = Some(e); event_loop.exit(); }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event { self.devices.mouse_motion(delta.0, delta.1); }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(l) = &mut self.live {
                    l.config.width = size.width.max(1);
                    l.config.height = size.height.max(1);
                    l.surface.configure(&l.gpu.device, &l.config);
                }
            }
            WindowEvent::Focused(false) => self.capture_mouse(false),
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left && state == ElementState::Pressed && !self.devices.mouse_captured && self.cfg.keyboard {
                    self.capture_mouse(true);
                    return;
                }
                self.devices.mouse_button(button, state);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if event.state == ElementState::Pressed && !event.repeat {
                    match code {
                        KeyCode::Escape => { if self.devices.mouse_captured { self.capture_mouse(false) } else { event_loop.exit() } return; }
                        KeyCode::F11 => {
                            if let Some(w) = &self.window { w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) }); }
                            return;
                        }
                        KeyCode::F12 => { self.screenshot = true; return; }
                        _ => {}
                    }
                }
                self.devices.key(code, event.state, event.repeat);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.frame() { self.error = Some(e); event_loop.exit(); return; }
                self.frames += 1;
                if self.frames % 60 == 0 { if let (Some(w), Some(t)) = (&self.window, self.game.title()) { w.set_title(&format!("{} | {t}", self.cfg.title)); } }
                if self.cfg.frames > 0 && self.frames >= self.cfg.frames {
                    let s = self.started.elapsed().as_secs_f64();
                    println!("smoke: {} frames in {s:.1} s ({:.1} fps)", self.frames, self.frames as f64 / s);
                    event_loop.exit();
                    return;
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) { event_loop.set_control_flow(ControlFlow::Poll); }
}
