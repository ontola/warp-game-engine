//! A game shell for Warp: the window, surface and presentation, a fixed
//! simulation tick with interpolation, local player input, split-screen
//! frames with a HUD, and the same frame path headless for screenshots and
//! benchmarks. A game implements [`Game`]; the shell owns the GPU.
//!
//! Built-in keys: click captures the mouse, Esc releases it (or quits when
//! released). F10 always opens the engine debug menu ([`debug`]), F11 toggles
//! fullscreen, F12 saves a feedback screenshot (PNG, JSON and the clipboard;
//! see [`screenshot`]); Shift+F12, or F12 twice, then a dragged rectangle
//! saves a crop of it. These reserved keys run before game/menu/text input.
//! F1 to F9 belong to [`Game::function_key`].
//!
//! Text entry: while [`Game::text_input`] is true the keyboard types
//! ([`Game::text`] gets characters, IME commits, editing keys and
//! Ctrl/Cmd+V paste) instead of pressing buttons; arrows up and down and
//! Tab still move menus. Seats: [`Game::devices`] sees every device's input
//! each tick and [`Game::seats`] can bind devices to seats, so a game can
//! let a spare pad join mid-match.
pub mod clipboard;
pub mod debug;
pub mod input;
pub mod text;
pub mod script;
pub mod sound;
pub mod tween;
pub mod ui;
pub mod manners;
pub mod startup;
pub mod screenshot;
pub mod quality;
mod frame_log;
pub mod profile;
pub mod frame_rate;
pub use frame_rate::FrameRate;

pub use input::{Buttons, Controls, Device, Devices, Pointer, Tuning};
pub use text::{FieldEvent, TextEvent, TextField};
use render::hud::{HudDrawList, HudRenderer};
use render::present::Presenter;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
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
    /// Frames per second that render the scene: frames that only redraw the
    /// HUD over the last scene image (debug menu open) do not count.
    pub scene_fps: f32,
    /// The longest scene frame of the last second, in ms (hitches the mean hides).
    pub scene_worst_ms: f32,
    /// How two views share the screen ([`Game::split`]).
    pub split: render::SplitLayout,
    renderer: &'a mut Option<Renderer>,
}

impl<'a> Frame<'a> {
    /// Frame access for an external presentation shell, including OpenXR.
    pub fn new(gpu: &'a Gpu, renderer: &'a mut Option<Renderer>, size: [u32; 2], dt: f32, alpha: f32) -> Self {
        Self { gpu, renderer, size, dt, alpha, split: render::SplitLayout::SideBySide, scene_fps: if dt > 0.0 { 1.0 / dt } else { 0.0 }, scene_worst_ms: dt * 1000.0 }
    }

    /// Upload a scene: creates the renderer, or replaces it.
    pub fn set_scene(&mut self, scene: &Scene, light: &Lighting) {
        let t = std::time::Instant::now();
        *self.renderer = Some(Renderer::new(self.gpu, scene, light));
        let ms = t.elapsed().as_millis();
        if ms >= 100 { println!("{} renderer: scene built in {ms} ms", manners::stamp()); }
    }
    /// Use a renderer the game built itself, e.g. on another thread with a
    /// clone of [`Frame::gpu`], so a scene swap does not stall the frame.
    pub fn set_renderer(&mut self, renderer: Renderer) { *self.renderer = Some(renderer); }
    pub fn renderer(&self) -> Option<&Renderer> { self.renderer.as_ref() }
    /// The renderer, for changes such as [`Renderer::reload_shaders`].
    pub fn renderer_mut(&mut self) -> Option<&mut Renderer> { self.renderer.as_mut() }
    /// Split-screen rectangles for `views` views, as x, y, width, height.
    pub fn rects(&self, views: usize) -> Vec<[f32; 4]> {
        render::split_rects_with(views, self.size[0], self.size[1], self.split).into_iter().map(|(x, y, w, h)| [x as f32, y as f32, w as f32, h as f32]).collect()
    }
}

pub trait Game {
    /// Local players whose controls the shell gathers.
    fn local_players(&self) -> usize;
    /// One fixed simulation step.
    fn tick(&mut self, controls: &[Controls]);
    /// Free cursor input, delivered just before each tick, in physical pixels.
    fn pointer(&mut self, _pointer: Pointer, _size: [u32; 2]) {}
    /// Some(true) captures during play, Some(false) releases for UI. None keeps
    /// the shell's traditional click-to-capture behavior.
    fn mouse_capture(&self) -> Option<bool> { None }
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
    /// The game wants this frame in memory (a live-control screenshot): the
    /// shell renders it offscreen as well and passes it to [`Game::captured`].
    fn wants_capture(&self) -> bool { false }
    /// The frame asked for by [`Game::wants_capture`], RGBA8, top row first.
    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) { let _ = (size, rgba); }
    /// Attach a gameplay replay beside an F12 screenshot. Called only on
    /// explicit capture, with its path and the interpolation phase shown.
    fn feedback_replay(&mut self, _path: &std::path::Path, _alpha: f32) -> Result<Option<serde_json::Value>, String> { Ok(None) }
    /// How two views share the screen.
    fn split(&self) -> render::SplitLayout { render::SplitLayout::Stacked }
    /// Escape was pressed: return true when the game used it (to open or
    /// close its own menu). Otherwise the shell releases the mouse, or quits
    /// when it is already released.
    fn escape(&mut self) -> bool { false }
    /// Startup progress (percent, what is happening) for a party host's
    /// loading screen: the shell reports opening the window, starting the
    /// GPU and building the first scene. Party games pass it to their host.
    fn loading(&mut self, percent: u32, step: &str) { let _ = (percent, step); }
    /// The GPU device was lost and replaced (a driver reset, the display
    /// waking up): GPU resources the game made itself are invalid. The
    /// renderer is gone too, so the next [`Game::draw`] sets the scene again.
    fn gpu_reset(&mut self) {}
    /// Function key F`n` (1..=9) was pressed, also while typing: return
    /// true when the game used it. F10, F11 and F12 stay the shell's.
    fn function_key(&mut self, _n: u8) -> bool { false }
    /// The game asks the shell to close (an Exit menu item).
    fn quit(&self) -> bool { false }
    /// Extra menus and controls for the F10 debug menu, with ids from
    /// [`debug::GAME_IDS`] up (a control's `menu` may name an engine menu).
    fn debug(&self) -> (Vec<render::debug_ui::DebugMenu>, Vec<render::debug_ui::DebugControl>) { (Vec::new(), Vec::new()) }
    /// One of the game's debug controls changed, or the menu was reset
    /// (`ResetDefaults`).
    fn debug_action(&mut self, _action: render::debug_ui::DebugAction) {}
    /// Adjust input tuning (look sensitivity) from the game's settings;
    /// called before every frame's ticks.
    fn tuning(&self, _tuning: &mut Tuning) {}
    /// Every connected device's input for the coming tick, before it is
    /// routed to seats: a spare pad's buttons (press X to join), which
    /// device was used last. Called just before each tick.
    fn devices(&mut self, _devices: &[(Device, Controls)]) {}
    /// The devices bound to each local seat (one list per seat, in seat
    /// order); devices in no list drive nobody. None: devices drive seats in
    /// order and a single seat takes every device.
    fn seats(&self) -> Option<Vec<Vec<Device>>> { None }
    /// The game has a text field focused: the keyboard types into it.
    fn text_input(&self) -> bool { false }
    /// Text typed while [`Game::text_input`] holds.
    fn text(&mut self, _event: TextEvent) {}
    /// Frame review needs fixed quality even when a slow seek misses its budget.
    fn fixed_quality(&self) -> bool { false }
    /// Adaptive quality has been on its lowest rung and still missed the
    /// target for [`quality::TOO_HEAVY_AFTER`]: resolution and grass are spent,
    /// so a game with cheaper content (fewer effects, simpler scenery) can
    /// switch to it. Called again after as long if it is still too slow.
    fn content_too_heavy(&mut self) {}
    /// Target framerate for dynamic adaptive graphics quality (e.g. Some(60.0)).
    /// None falls back to `Config::target_fps`, `WARP_TARGET_FPS`, or the monitor refresh rate.
    fn target_fps(&self) -> Option<f32> { None }
}

/// Engine-owned keys, shared with alternate presentation shells such as Sideways.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineShortcut { DebugMenu, Fullscreen, Screenshot }

/// Reserved even when the game is paused, reviewing a replay or accepting text.
pub fn engine_shortcut(code: KeyCode) -> Option<EngineShortcut> {
    match code {
        KeyCode::F10 => Some(EngineShortcut::DebugMenu),
        KeyCode::F11 => Some(EngineShortcut::Fullscreen),
        KeyCode::F12 => Some(EngineShortcut::Screenshot),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum KeyRoute { Engine(EngineShortcut), Consumed, Other }

/// Route every function-key edge before either the game or text/navigation handlers.
fn route_function_key(game: &mut dyn Game, code: KeyCode, state: ElementState, repeat: bool) -> KeyRoute {
    let press = state == ElementState::Pressed && !repeat;
    if let Some(action) = engine_shortcut(code) {
        return if press { KeyRoute::Engine(action) } else { KeyRoute::Consumed };
    }
    use KeyCode::*;
    if let Some(n) = [F1, F2, F3, F4, F5, F6, F7, F8, F9].iter().position(|&k| k == code) {
        if press { game.function_key(n as u8 + 1); }
        KeyRoute::Consumed
    } else { KeyRoute::Other }
}

/// One tick's controls through the game's seat binding: `devices` goes to
/// [`Game::devices`], then to the seats of [`Game::seats`].
pub fn seat_controls(game: &mut dyn Game, devices: &[(Device, Controls)]) -> Vec<Controls> {
    game.devices(devices);
    let seats = game.seats();
    input::route(devices, seats.as_deref(), game.local_players())
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
    /// Deliver the free cursor to [`Game::pointer`] even with `keyboard`
    /// off: the mouse points and clicks without becoming a player.
    pub pointer: bool,
    /// Smoke test: quit after this many frames and print the frame rate.
    pub frames: u64,
    /// Render the 3D views at most this many pixels tall and scale them up
    /// (the HUD stays sharp): a 5K Retina screen shades 7x the pixels of
    /// 1080p. `WARP_RENDER_HEIGHT` overrides it. Some(0) is native, also
    /// under adaptive quality (which otherwise starts from 1440p at most).
    pub max_render_height: Option<u32>,
    /// Target framerate for dynamic adaptive graphics quality.
    /// None checks `Game::target_fps`, `WARP_TARGET_FPS`, or the monitor refresh rate.
    pub target_fps: Option<f32>,
}

impl Default for Config {
    fn default() -> Self { Self { title: "Warp".into(), size: [1600, 900], tick_hz: 60.0, fullscreen: true, keyboard: true, pointer: false, frames: 0, max_render_height: None, target_fps: None } }
}

/// The present path shared by the window and headless capture.
struct Output { presenter: Presenter, hud: HudRenderer, picture: Option<wgpu::Texture>, rects: Vec<(u32, u32, u32, u32)>, timings: profile::DrawTimings }

impl Output {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Result<Self, String> {
        Ok(Self { presenter: Presenter::new(gpu, format), hud: HudRenderer::new(gpu, format)?, picture: None, rects: Vec::new(), timings: Default::default() })
    }
    fn draw(&mut self, gpu: &Gpu, renderer: &Renderer, draw: &Draw, size: [u32; 2], split: render::SplitLayout, target: &wgpu::TextureView, max_height: Option<u32>) -> Result<(), String> {
        self.draw_with(gpu, renderer, draw, size, split, target, max_height, true)
    }
    /// With `scene` false the last scene picture is presented again under a
    /// new HUD (a debug menu stays responsive when the scene is slow).
    #[allow(clippy::too_many_arguments)]
    fn draw_with(&mut self, gpu: &Gpu, renderer: &Renderer, draw: &Draw, size: [u32; 2], split: render::SplitLayout, target: &wgpu::TextureView, max_height: Option<u32>, scene: bool) -> Result<(), String> {
        let size = match max_height { Some(m) if m > 0 && size[1] > m => [(size[0] as u64 * m as u64 / size[1] as u64).max(1) as u32, m], _ => size };
        let rects = render::split_rects_with(draw.cameras.len(), size[0], size[1], split);
        let views: Vec<ViewRect> = draw.cameras.iter().zip(&rects).map(|(&cam, &(x, y, w, h))| ViewRect { cam, x, y, w, h }).collect();
        // Reuse the output while both size and layout hold. After hot-join,
        // fresh zeroed storage clears gaps and the unused fourth quadrant.
        let fresh = self.picture.as_ref().is_none_or(|p| [p.width(), p.height()] != size) || self.rects != rects;
        if fresh {
            self.picture = Some(Renderer::output_texture(gpu, size[0], size[1]));
            self.rects = rects;
            renderer.reset_history();
        }
        let picture = self.picture.as_ref().unwrap();
        let started = Instant::now();
        if scene || fresh { renderer.try_render_into(gpu, &views, &draw.light, picture)?; }
        self.timings.render_ms = started.elapsed().as_secs_f64() * 1000.;
        let started = Instant::now();
        self.hud.prepare(gpu, &draw.hud)?;
        self.timings.hud_ms = started.elapsed().as_secs_f64() * 1000.;
        let started = Instant::now();
        self.presenter.present(gpu, picture, target, Some(&self.hud), None)?;
        self.timings.present_ms = started.elapsed().as_secs_f64() * 1000.;
        Ok(())
    }
}

/// A screenshot's line along the bottom of the screen.
/// A frozen frame waiting for its crop rectangle (Shift+F12): shown dimmed
/// outside the rectangle while the mouse drags it; Esc cancels.
struct Crop {
    saved: screenshot::Saved,
    size: [u32; 2],
    picture: Arc<render::picture::Picture>,
    start: Option<[f32; 2]>,
    cursor: [f32; 2],
}

impl Crop {
    const IMAGE: render::hud::HudImage = render::hud::HudImage(u64::MAX - 40);

    /// x, y, width, height in frame pixels.
    fn rect(&self) -> Option<[f32; 4]> {
        let a = self.start?;
        let b = self.cursor;
        let (x0, y0) = (a[0].min(b[0]).max(0.0), a[1].min(b[1]).max(0.0));
        let (x1, y1) = (a[0].max(b[0]).min(self.size[0] as f32), a[1].max(b[1]).min(self.size[1] as f32));
        Some([x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)])
    }

    fn draw(&self, hud: &mut HudDrawList) {
        let [w, h] = self.size.map(|v| v as f32);
        let clip = [0.0, 0.0, w, h];
        hud.provide_image(Self::IMAGE, &self.picture, render::hud::ImageFilter::Linear);
        hud.image([0.0, 0.0, w, h], Self::IMAGE, [0.0, 0.0, 1.0, 1.0], [1.0; 4], clip);
        let dim = [0.0, 0.0, 0.0, 0.55];
        match self.rect() {
            Some([x, y, rw, rh]) => {
                hud.rect([0.0, 0.0, w, y], dim, clip);
                hud.rect([0.0, y + rh, w, h - y - rh], dim, clip);
                hud.rect([0.0, y, x, rh], dim, clip);
                hud.rect([x + rw, y, w - x - rw, rh], dim, clip);
                let white = [1.0, 1.0, 1.0, 0.9];
                for (a, b) in [([x, y], [x + rw, y]), ([x + rw, y], [x + rw, y + rh]), ([x + rw, y + rh], [x, y + rh]), ([x, y + rh], [x, y])] {
                    hud.line(a, b, 2.0, white, clip);
                }
            }
            None => hud.rect(clip, dim, clip),
        }
        let s = (h / 1080.0).clamp(0.6, 2.0);
        hud.text.push(render::hud::TextRun { id: u64::MAX - 41, text: "drag a rectangle to crop, Esc cancels".into(), position: [16.0 * s, 16.0 * s],
            bounds: clip, font_size: 22.0 * s, color: [255, 255, 255, 230], font: render::hud::FontFace::Body,
            align: render::hud::TextAlign::Left, width: w * 0.6 });
    }
}

fn toast(hud: &mut HudDrawList, line: &str) {
    let [w, h] = hud.size.map(|v| v as f32);
    let s = (h / 1080.0).clamp(0.6, 2.0);
    let (bw, bh) = ((1000.0 * s).min(w - 24.0).max(1.0), 48.0 * s);
    let (x, y) = ((w - bw) * 0.5, (h - 90.0 * s).max(0.0));
    let clip = [0.0, 0.0, w, h];
    hud.rect([x, y, bw, bh], [0.02, 0.03, 0.045, 0.92], clip);
    hud.text.push(render::hud::TextRun { id: u64::MAX - 2, text: line.into(), position: [x + 12.0 * s, y + 11.0 * s], bounds: clip,
        font_size: 22.0 * s, color: [255, 255, 255, 255], font: render::hud::FontFace::Body, align: render::hud::TextAlign::Center, width: bw - 24.0 * s });
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
        self.frame_at(game, 1.0)
    }
    /// Render an exact sub-tick phase (for saved gameplay moments).
    pub fn frame_at(&mut self, game: &mut dyn Game, alpha: f32) -> Result<Vec<u8>, String> {
        self.render_at(game, alpha)?;
        Ok(self.gpu.read_rgba8(&self.target))
    }
    /// Submit a native-resolution frame without copying its pixels to the CPU.
    /// Timings measure CPU wall time; GPU completion is deliberately separate.
    pub fn render_at(&mut self, game: &mut dyn Game, alpha: f32) -> Result<profile::DrawTimings, String> {
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) { return Err("alpha must be in 0..1".into()); }
        let split = game.split();
        let mut frame = Frame { gpu: self.gpu, size: self.size, dt: 1.0 / self.tick_hz, alpha, split, scene_fps: self.tick_hz, scene_worst_ms: 1000.0 / self.tick_hz, renderer: &mut self.renderer };
        let started = Instant::now();
        let draw = game.draw(&mut frame)?;
        let game_ms = started.elapsed().as_secs_f64() * 1000.;
        let renderer = self.renderer.as_ref().ok_or("the game drew without a scene")?;
        self.out.draw(self.gpu, renderer, &draw, self.size, split, &self.target.create_view(&Default::default()), None)?;
        Ok(profile::DrawTimings { game_ms, ..self.out.timings })
    }
    pub fn renderer(&self) -> Option<&Renderer> { self.renderer.as_ref() }
}

/// Headless benchmark: `frames` frames at `size` with idle controls, no
/// window. Returns frames per second (wall time, including readback) and
/// the smoothed GPU milliseconds per pass.
pub fn bench(game: &mut dyn Game, gpu: &Gpu, size: [u32; 2], frames: u32, tick_hz: f32) -> Result<(f64, Vec<(&'static str, f64)>), String> {
    let mut h = Headless::new(gpu, size, tick_hz)?;
    let mut profile = debug::EngineDebug::default();
    let players = game.local_players();
    let mut started = Instant::now();
    for i in 0..frames {
        // The first frames build the scene; time from the third on.
        if i == 2 { started = Instant::now(); }
        game.tick(&vec![Controls::default(); players]);
        h.frame(game)?;
        if let Some(r) = h.renderer() { profile.profile(gpu, r, true); }
    }
    Ok((f64::from(frames.saturating_sub(2)) / started.elapsed().as_secs_f64(), profile.passes().to_vec()))
}

/// Headless: run `ticks` steps with idle controls, draw one frame and return
/// it as RGBA8. No window is created.
pub fn capture(game: &mut dyn Game, gpu: &Gpu, size: [u32; 2], ticks: u32, tick_hz: f32) -> Result<Vec<u8>, String> {
    let players = game.local_players();
    for _ in 0..ticks { game.tick(&vec![Controls::default(); players]); }
    Headless::new(gpu, size, tick_hz)?.frame(game)
}

/// Run a game in real time without a window (live control on a machine
/// whose screen must stay untouched, test servers): idle controls at
/// `cfg.tick_hz`, a frame rendered only when the game asks for one
/// ([`Game::wants_capture`]), at `cfg.size`. Stops after `seconds` (0: never).
pub fn run_headless(cfg: &Config, game: &mut dyn Game, gpu: &Gpu, seconds: f32) -> Result<(), String> {
    let step = std::time::Duration::from_secs_f32(1.0 / cfg.tick_hz);
    let start = Instant::now();
    let mut headless = None;
    let mut due = start;
    loop {
        if seconds > 0.0 && start.elapsed().as_secs_f32() >= seconds { return Ok(()); }
        game.tick(&vec![Controls::default(); game.local_players()]);
        if game.wants_capture() {
            if headless.is_none() { headless = Some(Headless::new(gpu, cfg.size, cfg.tick_hz)?); }
            let h = headless.as_mut().unwrap();
            let rgba = h.frame(game)?;
            game.captured(h.size, rgba);
        }
        due += step;
        match due.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            // Fell behind (a slow software frame): do not try to catch up.
            None => due = Instant::now(),
        }
    }
}

struct Live {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    out: Output,
    renderer: Option<Renderer>,
    /// Set from wgpu's callbacks when the device is gone (a driver reset,
    /// the display sleeping): everything made on it is invalid.
    lost: Arc<std::sync::atomic::AtomicBool>,
}

impl Live {
    /// A device, surface and presenter for `window`.
    fn open(window: &Arc<Window>, t: Instant) -> Result<(Self, [u128; 2]), String> {
        let size = window.inner_size();
        let open = |instance: wgpu::Instance| -> Result<_, String> {
            let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
            let surfaced = t.elapsed().as_millis();
            let (gpu, config) = Gpu::for_surface(&instance, &surface, size.width.max(1), size.height.max(1))?;
            Ok((surface, gpu, config, surfaced))
        };
        let (surface, gpu, config, surfaced) = match render::gpu::window_instance().map(&open) {
            Some(Ok(v)) => v,
            Some(Err(e)) => { println!("{} Vulkan: {e}; trying the other backends", manners::stamp()); open(render::gpu::instance())? }
            None => open(render::gpu::instance())?,
        };
        let devised = t.elapsed().as_millis();
        surface.configure(&gpu.device, &config);
        let out = Output::new(&gpu, config.format)?;
        let lost = watch(&gpu);
        Ok((Live { surface, config, gpu, out, renderer: None, lost }, [surfaced, devised]))
    }

    fn is_lost(&self) -> bool { self.lost.load(std::sync::atomic::Ordering::Relaxed) }
}

/// Log wgpu errors instead of panicking on them, and flag a lost device so
/// the shell builds a new one instead of crashing on invalid buffers.
fn watch(gpu: &Gpu) -> Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    let lost = Arc::new(AtomicBool::new(false));
    let flag = lost.clone();
    gpu.device.set_device_lost_callback(move |reason, message| {
        eprintln!("{} GPU device lost ({reason:?}): {message}", manners::stamp());
        flag.store(true, Ordering::Relaxed);
    });
    let flag = lost.clone();
    let count = AtomicU32::new(0);
    gpu.device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        let n = count.fetch_add(1, Ordering::Relaxed);
        if n < 10 || n % 1000 == 0 { eprintln!("{} GPU error #{}: {error}", manners::stamp(), n + 1); }
        if matches!(error, wgpu::Error::OutOfMemory { .. }) || error.to_string().to_ascii_lowercase().contains("lost") { flag.store(true, Ordering::Relaxed); }
    }));
    lost
}

/// Started by or pointed at a GameNight host (GAMENIGHT=1 or GAMENIGHT_ADDR).
fn under_host() -> bool {
    std::env::var("GAMENIGHT").as_deref() == Ok("1") || std::env::var("GAMENIGHT_ADDR").is_ok_and(|a| !a.is_empty())
}

struct Shell<'g> {
    cfg: Config,
    game: &'g mut dyn Game,
    devices: Devices,
    live: Option<Live>,
    window: Option<Arc<Window>>,
    screen: Screen,
    manners: manners::Manners,
    focused: bool,
    acc: f32,
    last: Instant,
    last_frame_end: Instant,
    mouse_events: u32,
    mouse_delta: [f64; 2],
    mouse_peak: f64,
    cursor_events: u32,
    started: Instant,
    frames: u64,
    screenshot: bool,
    /// Shift+F12: after the frame is saved, drag a rectangle of it.
    crop_requested: bool,
    crop: Option<Crop>,
    /// The last F12 frame: a second F12 soon after crops it (no modifier).
    last_shot: Option<(Instant, Crop)>,
    /// Shift as seen by key events: ModifiersChanged alone missed it on macOS.
    shift_down: bool,
    /// The last screenshot's line and until when it shows.
    toast: Option<(String, Instant)>,
    /// Smoothed frames per second, for screenshot metadata.
    fps: f32,
    rate: FrameRate,
    debug: debug::EngineDebug,
    quality: Option<quality::AdaptiveQuality>,
    screen_refresh_hz: Option<f64>,
    /// When the scene was last rendered (see [`Shell::scene_due`]).
    last_scene: Instant,
    /// Scene frames and loop iterations since `stats_at`, logged every 2 s.
    scene_frames: u32,
    loop_frames: u32,
    stats_at: Instant,
    error: Option<String>,
    modifiers: ModifiersState,
    /// IME composition in progress: key text is the IME's until it commits.
    composing: bool,
    ime: bool,
    /// The window has no pixels (minimised, the display asleep): nothing to present.
    zero_size: bool,
    /// When a lost device was last rebuilt or tried.
    rebuilt: Option<Instant>,
}

/// Without a game's own cap, adaptive quality scales down from at most this
/// render height.
const ADAPTIVE_BASE_HEIGHT: u32 = 1440;

/// Open a window and run the game until it closes.
pub fn run(cfg: Config, game: &mut dyn Game) -> Result<(), String> {
    let mut devices = Devices::new(cfg.keyboard);
    devices.pointer_enabled = cfg.pointer;
    let target_fps = game.target_fps().or(cfg.target_fps);
    let quality = quality::AdaptiveQuality::new(target_fps, None, Instant::now());
    let mut debug = debug::EngineDebug::default();
    if let Some(target) = target_fps {
        debug.set_fps_target(Some(target.round() as u32));
    }
    if let Some(q) = &quality {
        println!("{}: adaptive quality enabled, {}", cfg.title, q.label());
    }
    println!("{}: {} local players, {} gamepads", cfg.title, game.local_players(), devices.pad_count());
    let mut shell = Shell { devices, cfg, game, live: None, window: None, screen: Screen::Normal, manners: manners::Manners::new(), focused: true, acc: 0.0, last: Instant::now(), last_frame_end: Instant::now(), mouse_events: 0, mouse_delta: [0.0; 2], mouse_peak: 0.0, cursor_events: 0, started: Instant::now(), frames: 0, screenshot: false, crop_requested: false, crop: None, last_shot: None, shift_down: false, toast: None, fps: 0.0, rate: FrameRate::new(), debug, quality, screen_refresh_hz: None, last_scene: Instant::now(), scene_frames: 0, loop_frames: 0, stats_at: Instant::now(), error: None, modifiers: ModifiersState::empty(), composing: false, ime: false, zero_size: false, rebuilt: None };
    let events = EventLoop::new().map_err(|e| e.to_string())?;
    events.run_app(&mut shell).map_err(|e| e.to_string())?;
    shell.error.map_or(Ok(()), Err)
}

impl Shell<'_> {
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(e) = self.frame() { self.error = Some(e); event_loop.exit(); return; }
        if self.game.quit() { event_loop.exit(); return; }
        self.frames += 1;
        if self.frames % 60 == 0 { if let (Some(w), Some(t)) = (&self.window, self.game.title()) { w.set_title(&format!("{} | {t}", self.cfg.title)); } }
        if self.cfg.frames > 0 && self.frames >= self.cfg.frames {
            let s = self.started.elapsed().as_secs_f64();
            println!("smoke: {} frames in {s:.1} s ({:.1} fps)", self.frames, self.frames as f64 / s);
            for (pass, ms) in self.debug.passes() { println!("  gpu {pass:>28} {ms:6.2} ms"); }
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window { w.request_redraw(); }
    }

    fn frame(&mut self) -> Result<(), String> {
        let frame_started = Instant::now();
        let event_gap_ms = (frame_started - self.last_frame_end).as_secs_f64() * 1000.0;
        let wish = self.game.screen();
        if self.manners.wish(wish, self.focused) && wish == Screen::Hidden { self.capture_mouse(false); }
        self.screen = self.manners.screen;
        if let Some(w) = &self.window { self.manners.step(w, self.focused, self.cfg.fullscreen); }
        // Hidden: keep ticking (messages, warm loading) at a few frames a second.
        if self.screen == Screen::Hidden { std::thread::sleep(std::time::Duration::from_millis(50)); }
        let now = Instant::now();
        let window_ms = (now - frame_started).as_secs_f64() * 1000.0;
        let frame_interval_ms=(now-self.last).as_secs_f64()*1000.0;
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        if let Some(q) = &mut self.quality {
            q.set_enabled(self.debug.adaptive_quality() && !self.game.fixed_quality());
            let target = match self.debug.target_fps() {
                Some(user) => user,
                None => self.game.target_fps().or(self.cfg.target_fps),
            };
            q.set_target_fps(target, self.screen_refresh_hz);
            if self.focused && !self.zero_size && q.frame(f64::from(dt) * 1000.0, now) {
                println!("{} adaptive quality: {}", manners::stamp(), q.label());
            }
            if q.content_too_heavy(now) {
                println!("{} adaptive quality: content too heavy at {}", manners::stamp(), q.label());
                self.game.content_too_heavy();
            }
        }
        let input_started = Instant::now();
        self.devices.poll();
        self.game.tuning(&mut self.devices.tuning);
        self.sync_capture();
        let input_ms = input_started.elapsed().as_secs_f64() * 1000.0;
        let step = 1.0 / self.cfg.tick_hz;
        self.acc += dt;
        let tick_started=Instant::now();
        let mut steps = 0;
        let mut input_look = Vec::new();
        while self.acc >= step && steps < 6 {
            self.acc -= step;
            steps += 1;
            let size = self.live.as_ref().map_or(self.cfg.size, |l| [l.config.width, l.config.height]);
            self.game.pointer(self.devices.pointer(), size);
            let devices = self.devices.device_controls(step);
            let controls = seat_controls(self.game, &devices);
            if frame_log::enabled() { input_look.push(controls.iter().map(|c| c.look).collect::<Vec<_>>()); }
            self.game.tick(&controls);
            self.sync_capture();
        }
        let tick_ms=tick_started.elapsed().as_secs_f64()*1000.0;
        if steps == 6 { self.acc = 0.0; }
        let wants_text = self.game.text_input();
        if wants_text != self.ime && let Some(w) = &self.window { w.set_ime_allowed(wants_text); self.ime = wants_text; }
        // `WARP_TEST_DEVICE_LOSS=SECONDS`: destroy the device once, to try the rebuild.
        if let Some(live) = &self.live && self.rebuilt.is_none() && std::env::var("WARP_TEST_DEVICE_LOSS").ok().and_then(|v| v.parse::<f32>().ok()).is_some_and(|t| self.started.elapsed().as_secs_f32() >= t) {
            println!("{} WARP_TEST_DEVICE_LOSS: destroying the GPU device", manners::stamp());
            self.rebuilt = Some(Instant::now() - std::time::Duration::from_secs(2));
            live.gpu.device.destroy();
        }
        if self.live.as_ref().is_some_and(Live::is_lost) && !self.recover() { return Ok(()); }
        let Some(live) = &mut self.live else { return Ok(()) };
        let size = [live.config.width, live.config.height];
        let split = self.game.split();
        if live.renderer.is_none() { self.game.loading(75, "building the scene"); }
        // While the debug menu is open the scene renders about once a second
        // and shortly after a change; frames in between only redraw the HUD,
        // so the menu answers clicks even when the scene takes 500 ms.
        let scene = !self.debug.open() || self.last_scene.elapsed().as_secs_f32() >= 1.0
            || self.debug.changed.is_some_and(|t| t > self.last_scene && t.elapsed().as_secs_f32() >= 0.25);
        // The fps shown counts scene frames only, not HUD-only redraws.
        if scene {
            // Mean scene frame time, not mean of 1/dt: that read 100+ fps
            // through 200 ms hitches on fast mouse turns.
            self.rate.push(self.last_scene.elapsed().as_secs_f32(), Instant::now());
            self.fps = self.rate.fps();
            self.last_scene = Instant::now();
            self.scene_frames += 1;
        }
        self.loop_frames += 1;
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha: (self.acc / step).clamp(0.0, 1.0), split, scene_fps: self.fps, scene_worst_ms: self.rate.worst_ms(), renderer: &mut live.renderer };
        let draw_started=Instant::now();
        let mut draw = self.game.draw(&mut frame)?;
        let draw_ms=draw_started.elapsed().as_secs_f64()*1000.0;
        let Some(renderer) = &live.renderer else { return Err("the game drew without a scene".into()) };
        draw.light = self.debug.light(draw.light);
        if let Some(q) = &self.quality {
            draw.light = q.apply_light(draw.light);
        }
        let debug_options = if let Some(q) = &self.quality {
            q.apply_options(self.debug.options())
        } else {
            self.debug.options()
        };
        self.debug.sync_options(renderer, debug_options);
        let profile_started = Instant::now();
        self.debug.profile(&live.gpu, renderer, self.debug.open() || self.cfg.frames > 0 || frame_log::enabled());
        let profile_ms = profile_started.elapsed().as_secs_f64() * 1000.0;
        if self.debug.open() {
            let quality_label = self.quality.as_ref().map(|q| q.label());
            self.debug.refresh(self.game.debug(), self.cfg.max_render_height, self.fps, quality_label.as_deref());
            if let Some(a) = self.debug.tick(f64::from(dt), size) { self.game.debug_action(a); }
        }
        // The hint is for a developer at a desk: never on a GameNight TV,
        // nor once a controller is plugged in.
        if !self.devices.mouse_captured && self.cfg.keyboard && self.game.mouse_capture().is_none() && !under_host() && self.devices.pad_count() == 0 {
            let s = (size[1] as f32 / 1080.0).max(0.5);
            draw.hud.text.push(render::hud::TextRun { id: u64::MAX - 1, text: "click to capture the mouse, Esc releases".into(), position: [16.0 * s, size[1] as f32 - 34.0 * s],
                bounds: [0.0, 0.0, size[0] as f32, size[1] as f32], font_size: 18.0 * s, color: [220, 220, 215, 200], font: render::hud::FontFace::Body,
                align: render::hud::TextAlign::Left, width: size[0] as f32 * 0.6 });
        }
        let base_height = self.debug.render_height().unwrap_or_else(|| std::env::var("WARP_RENDER_HEIGHT").ok().and_then(|v| v.parse().ok()).or(self.cfg.max_render_height));
        let render_height = match (&self.quality, base_height) {
            // 0 is native: adaptive quality scales down from the full window.
            (Some(q), Some(0)) => { let h = q.render_height(size[1]); (h < size[1]).then_some(h) }
            (Some(q), Some(h)) => Some(q.render_height(h)),
            (Some(q), None) => {
                // A 5K or 4K window starts from about 1440p, not native.
                let h = q.render_height(size[1].min(ADAPTIVE_BASE_HEIGHT));
                (h < size[1]).then_some(h)
            }
            (None, h) => h,
        };
        let seconds = self.stats_at.elapsed().as_secs_f32();
        if seconds >= 2.0 {
            println!("{} frames: scene {:.1}/s, loop {:.1}/s, render {}x{} of {}x{}, debug menu {}", manners::stamp(),
                self.scene_frames as f32 / seconds, self.loop_frames as f32 / seconds,
                render_height.map_or(size[0], |h| (size[0] as u64 * h as u64 / size[1].max(1) as u64) as u32), render_height.unwrap_or(size[1]),
                size[0], size[1], if self.debug.open() { "open" } else { "closed" });
            (self.scene_frames, self.loop_frames, self.stats_at) = (0, 0, Instant::now());
        }
        let (to_file, to_game) = (std::mem::take(&mut self.screenshot), self.game.wants_capture());
        if to_file || to_game {
            let target = offscreen(&live.gpu, size, wgpu::TextureFormat::Rgba8UnormSrgb);
            Output::new(&live.gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?.draw(&live.gpu, renderer, &draw, size, split, &target.create_view(&Default::default()), render_height)?;
            let rgba = live.gpu.read_rgba8(&target);
            if to_file {
                let state = self.game.state().map(|s| serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s)));
                let mut metadata = serde_json::json!({
                    "schema": 1, "game": self.cfg.title, "title": self.game.title(), "state": state,
                    "cameras": draw.cameras.iter().map(|c| serde_json::json!({"position": c.pos.to_array(), "target": c.target.to_array(), "fov_y_deg": c.fov_y.to_degrees(), "near": c.near})).collect::<Vec<_>>(),
                    "debug": self.debug.metadata(), "render": {"adapter": live.gpu.info.name, "backend": format!("{:?}", live.gpu.info.backend), "driver": live.gpu.info.driver, "size": size, "fps": self.fps, "frame": self.frames, "uptime_s": self.started.elapsed().as_secs_f32()},
                });
                if let Some(q) = &self.quality {
                    metadata["render"]["adaptive_quality"] = q.metadata();
                }
                let saved = screenshot::save_full(screenshot::Shot { title: &self.cfg.title, size, rgba: &rgba, metadata }, |path| self.game.feedback_replay(path, (self.acc / step).clamp(0.0, 1.0)));
                let line = match saved {
                    Ok(saved) => {
                        let line = saved.line.clone();
                        let crop = render::picture::Picture::new(size[0], size[1], rgba.clone()).ok()
                            .map(|picture| Crop { saved, size, picture: Arc::new(picture), start: None, cursor: [0.0; 2] });
                        match (crop, std::mem::take(&mut self.crop_requested)) {
                          (Some(crop), true) => {
                            self.crop = Some(crop);
                            // capture_mouse(false), field by field: `live` is borrowed.
                            self.devices.reset_mouse();
                            self.devices.mouse_captured = false;
                            if let Some(w) = &self.window {
                                let _ = w.set_cursor_grab(CursorGrabMode::None);
                                w.set_cursor_visible(true);
                                w.set_cursor(winit::window::CursorIcon::Crosshair);
                            }
                            line
                          }
                          (crop, _) => {
                            if let Some(crop) = crop { self.last_shot = Some((Instant::now(), crop)); }
                            format!("{line}  (F12 again: crop)")
                          }
                        }
                    }
                    Err(e) => { eprintln!("screenshot failed: {e}"); format!("screenshot failed: {e}") }
                };
                self.toast = Some((line, Instant::now() + std::time::Duration::from_secs(4)));
            }
            if to_game { self.game.captured(size, rgba); }
        }
        if self.toast.as_ref().is_some_and(|(_, until)| Instant::now() > *until) { self.toast = None; }
        if let Some(crop) = &self.crop { draw.hud.begin_overlay(); crop.draw(&mut draw.hud); }
        if let Some((line, _)) = &self.toast { toast(&mut draw.hud, line); }
        if self.debug.open() { self.debug.draw(&mut draw.hud); }
        // Nothing to show on a window without pixels; the game keeps ticking.
        if self.zero_size || live.is_lost() { return Ok(()); }
        let present_started=Instant::now();
        match live.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => {
                let view = tex.texture.create_view(&Default::default());
                // A device lost mid-frame panics inside the text renderer
                // (glyphon unwraps a mapped buffer): rebuild instead of crashing.
                let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| live.out.draw_with(&live.gpu, renderer, &draw, size, split, &view, render_height, scene)));
                match drawn {
                    Ok(r) => r?,
                    Err(_) => {
                        eprintln!("{} drawing the frame panicked; rebuilding the GPU device", manners::stamp());
                        live.lost.store(true, std::sync::atomic::Ordering::Relaxed);
                        return Ok(());
                    }
                }
                if let Some(w) = &self.window { w.pre_present_notify(); }
                live.gpu.queue.present(tex);
                if let Some(w) = &self.window { self.manners.presented(w, size); }
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => live.surface.configure(&live.gpu.device, &live.config),
            _ => {}
        }
        if frame_log::enabled() {
            frame_log::frame(self.frames, frame_interval_ms, tick_ms, draw_ms, present_started.elapsed().as_secs_f64()*1000.0, render_height.unwrap_or(size[1]), self.focused,
                serde_json::json!({"frame_ms": frame_started.elapsed().as_secs_f64()*1000.0, "event_gap_ms": event_gap_ms,
                    "window_ms": window_ms, "input_ms": input_ms, "profile_ms": profile_ms,
                    "mouse_events": self.mouse_events, "cursor_events": self.cursor_events,
                    "mouse_delta": self.mouse_delta, "mouse_peak": self.mouse_peak, "captured": self.devices.mouse_captured,
                    "input_look": input_look, "alpha": self.acc / step,
                    "camera_directions": draw.cameras.iter().map(|c| (c.target-c.pos).normalize_or_zero().to_array()).collect::<Vec<_>>()}));
        }
        self.mouse_events = 0;
        self.mouse_delta = [0.0; 2];
        self.mouse_peak = 0.0;
        self.cursor_events = 0;
        self.last_frame_end = Instant::now();
        Ok(())
    }

    /// Replace a lost device: a new device, surface and presenter; the game
    /// builds its scene again (its renderer is gone). Tried at most once a
    /// second while the screen has no pixels or no adapter answers. True
    /// when there is a working device again.
    fn recover(&mut self) -> bool {
        if self.zero_size { std::thread::sleep(std::time::Duration::from_millis(50)); return false; }
        if self.rebuilt.is_some_and(|t| t.elapsed().as_secs_f32() < 1.0) { std::thread::sleep(std::time::Duration::from_millis(50)); return false; }
        self.rebuilt = Some(Instant::now());
        let Some(window) = self.window.clone() else { return false };
        // The old device stays (flagged lost) until a new one answers.
        match Live::open(&window, Instant::now()) {
            Ok((live, _)) => {
                println!("{} GPU device rebuilt: {} ({}), {}x{}", manners::stamp(), live.gpu.info.name, live.gpu.info.backend, live.config.width, live.config.height);
                self.live = Some(live);
                self.game.gpu_reset();
                true
            }
            Err(e) => { eprintln!("{} GPU device rebuild failed: {e}; trying again", manners::stamp()); false }
        }
    }

    fn capture_mouse(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        self.devices.reset_mouse();
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

    fn size(&self) -> [u32; 2] { self.live.as_ref().map_or(self.cfg.size, |l| [l.config.width, l.config.height]) }

    /// F10 opens and closes the debug menu; while it is open it takes the
    /// navigation keys. True when the key was used.
    fn debug_key(&mut self, code: KeyCode, repeat: bool) -> bool {
        use render::debug_ui::DebugKey;
        let key = match code {
            KeyCode::F10 if !repeat => DebugKey::Toggle,
            _ if !self.debug.open() => return false,
            KeyCode::Escape if !repeat => DebugKey::Close,
            KeyCode::ArrowUp => DebugKey::Up,
            KeyCode::ArrowDown => DebugKey::Down,
            KeyCode::ArrowLeft => DebugKey::Left,
            KeyCode::ArrowRight => DebugKey::Right,
            KeyCode::Enter | KeyCode::NumpadEnter if !repeat => DebugKey::Enter,
            KeyCode::PageUp => DebugKey::PageUp,
            KeyCode::PageDown => DebugKey::PageDown,
            _ => return false,
        };
        let was_open = self.debug.open();
        let quality_label = self.quality.as_ref().map(|q| q.label());
        self.debug.refresh(self.game.debug(), self.cfg.max_render_height, self.fps, quality_label.as_deref());
        let (used, action) = self.debug.key(key, self.size());
        if let Some(a) = action { self.game.debug_action(a); }
        // The pointer works the menu while it is open.
        if !was_open && self.debug.open() { self.capture_mouse(false); }
        used
    }

    fn start_crop(&mut self, crop: Crop) {
        self.crop = Some(crop);
        self.capture_mouse(false);
        if let Some(w) = &self.window { w.set_cursor(winit::window::CursorIcon::Crosshair); }
    }

    /// Left button down starts the rectangle, up saves it.
    fn crop_drag(&mut self, state: ElementState) {
        let Some(crop) = &mut self.crop else { return };
        if state == ElementState::Pressed { crop.start = Some(crop.cursor); return; }
        let Some([x, y, w, h]) = crop.rect() else { return };
        if w < 4.0 || h < 4.0 { crop.start = None; return; }
        let rect = [x.round() as u32, y.round() as u32, w.round() as u32, h.round() as u32];
        let line = screenshot::save_crop(&self.cfg.title, &crop.saved, &crop.picture.rgba, crop.size, rect)
            .unwrap_or_else(|e| { eprintln!("screenshot crop failed: {e}"); format!("crop failed: {e}") });
        self.end_crop(line);
    }

    fn end_crop(&mut self, line: String) {
        self.crop = None;
        if let Some(w) = &self.window { w.set_cursor(winit::window::CursorIcon::Default); }
        self.toast = Some((line, Instant::now() + std::time::Duration::from_secs(4)));
        self.sync_capture();
    }

    fn sync_capture(&mut self) {
        if self.crop.is_some() { return; }
        let capture = if self.debug.open() { Some(false) } else { self.game.mouse_capture() };
        if let Some(want) = capture {
            let want = want && self.cfg.keyboard && self.focused && self.screen != Screen::Hidden;
            if want != self.devices.mouse_captured { self.capture_mouse(want); }
        }
    }
}

/// Bound paint starvation without starving input when drawing uses a full frame.
#[cfg(any(windows, test))]
fn input_redraw_due(frame_age: std::time::Duration, dispatch_age: std::time::Duration, hz: f64) -> bool {
    frame_age.as_secs_f64() >= 1.0 / hz && dispatch_age >= std::time::Duration::from_millis(1)
}

impl ApplicationHandler for Shell<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let result = (|| -> Result<(), String> {
            let attrs = Window::default_attributes().with_title(&self.cfg.title).with_inner_size(PhysicalSize::new(self.cfg.size[0], self.cfg.size[1]))
                .with_fullscreen(self.cfg.fullscreen.then_some(Fullscreen::Borderless(None)))
                .with_window_icon(startup::icon());
            let attrs = self.manners.first(self.game.screen(), attrs);
            self.screen = self.manners.screen;
            self.game.loading(20, "opening the window");
            let t = Instant::now();
            let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
            let made = t.elapsed().as_millis();
            self.game.loading(35, "starting the graphics card");
            let size = window.inner_size();
            let (live, [surfaced, devised]) = Live::open(&window, t)?;
            self.game.loading(55, "preparing the screen");
            println!("{} startup: window {made} ms, instance and surface {} ms, adapter and device {} ms, swapchain and presenter {} ms", manners::stamp(), surfaced - made, devised - surfaced, t.elapsed().as_millis() - devised);
            println!("{} GPU {} ({}), {}x{}", manners::stamp(), live.gpu.info.name, live.gpu.info.backend, size.width, size.height);
            let refresh = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).map(|mhz| f64::from(mhz) / 1000.0);
            self.screen_refresh_hz = refresh;
            let target = match self.debug.target_fps() {
                Some(user) => user,
                None => self.game.target_fps().or(self.cfg.target_fps),
            };
            if let Some(q) = &mut self.quality {
                q.set_target_fps(target, refresh);
                println!("{} adaptive quality: {}", manners::stamp(), q.label());
            } else {
                self.quality = quality::AdaptiveQuality::new(target, refresh, Instant::now());
                if let Some(q) = &self.quality {
                    println!("{} adaptive quality: {}", manners::stamp(), q.label());
                }
            }
            self.zero_size = size.width == 0 || size.height == 0;
            self.live = Some(live);
            self.window = Some(window.clone());
            self.last = Instant::now();
            self.started = Instant::now();
            window.request_redraw();
            Ok(())
        })();
        if let Err(e) = result { self.error = Some(e); event_loop.exit(); }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.mouse_events = self.mouse_events.saturating_add(1);
            self.mouse_delta[0] += delta.0;
            self.mouse_delta[1] += delta.1;
            self.mouse_peak = self.mouse_peak.max(delta.0.abs()).max(delta.1.abs());
            self.devices.mouse_motion(delta.0, delta.1);
            // Win32 dispatches queued input before WM_PAINT. A continuous raw
            // mouse stream can therefore starve RedrawRequested for hundreds
            // of milliseconds, even with ControlFlow::Poll. Give rendering a
            // turn at the frame deadline while continuing to accumulate every
            // mouse delta. This runs on the same event thread as normal draws.
            #[cfg(windows)]
            if self.focused && self.devices.mouse_captured && !self.zero_size
                && !self.manners.offscreen() && self.frames > 0 && !event_loop.exiting()
            {
                let target = match self.debug.target_fps() {
                    Some(user) => user,
                    None => self.game.target_fps().or(self.cfg.target_fps),
                };
                let hz = target.map(f64::from).or(self.screen_refresh_hz).unwrap_or(120.0).clamp(30.0, 240.0);
                // A slow draw can already exhaust the frame deadline. Do not
                // immediately draw again on the very next mouse packet: that
                // services one old packet per frame and starves the input queue.
                // Reserve a small dispatch slice after drawing to drain the
                // queued motion before yielding back to rendering.
                if input_redraw_due(self.last.elapsed(), self.last_frame_end.elapsed(), hz) {
                    self.redraw(event_loop);
                }
            }
            #[cfg(not(windows))]
            let _ = event_loop;
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.zero_size = size.width == 0 || size.height == 0;
                if let Some(l) = &mut self.live && !l.is_lost() {
                    l.config.width = size.width.max(1);
                    l.config.height = size.height.max(1);
                    l.surface.configure(&l.gpu.device, &l.config);
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused { self.capture_mouse(false); self.devices.cursor(None); }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_events = self.cursor_events.saturating_add(1);
                let p = [position.x as f32, position.y as f32];
                if let Some(crop) = &mut self.crop { crop.cursor = p; return; }
                self.debug.pointer_move(p);
                self.devices.cursor(Some(p));
            }
            WindowEvent::CursorLeft { .. } => self.devices.cursor(None),
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if self.debug.open() && self.debug.scroll(lines, self.size()) { return; }
                self.devices.wheel(lines);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.crop.is_some() {
                    if button == MouseButton::Left { self.crop_drag(state); }
                    return;
                }
                if self.debug.open() && button == MouseButton::Left {
                    let action = if state == ElementState::Pressed { self.debug.pointer_down(self.size()).1 } else { self.debug.pointer_up() };
                    if let Some(a) = action { self.game.debug_action(a); }
                    return;
                }
                if button == MouseButton::Left && state == ElementState::Pressed && !self.devices.mouse_captured && self.cfg.keyboard && self.game.mouse_capture().is_none() {
                    self.capture_mouse(true);
                    return;
                }
                self.devices.mouse_button(button, state);
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => self.composing = !text.is_empty(),
                Ime::Commit(text) => { self.composing = false; if self.game.text_input() && !text.is_empty() { self.game.text(TextEvent::Insert(text)); } }
                Ime::Enabled | Ime::Disabled => self.composing = false,
            },
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) { self.shift_down = event.state == ElementState::Pressed; }
                if self.crop.is_some() {
                    if code == KeyCode::Escape && event.state == ElementState::Pressed { self.end_crop("crop cancelled".into()); }
                    return;
                }
                match route_function_key(self.game, code, event.state, event.repeat) {
                    KeyRoute::Engine(action) => {
                        match action {
                            EngineShortcut::DebugMenu => { self.debug_key(KeyCode::F10, false); }
                            EngineShortcut::Fullscreen => if let Some(w) = &self.window {
                                w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                            },
                            EngineShortcut::Screenshot => if self.crop.is_none() {
                                let shift = self.modifiers.shift_key() || self.shift_down;
                                let again = self.last_shot.take().filter(|(at, _)| at.elapsed().as_secs_f32() < 1.5);
                                println!("F12: shift {shift}, again {}", again.is_some());
                                if let Some((_, crop)) = again { self.start_crop(crop); }
                                else { self.screenshot = true; self.crop_requested = shift; }
                            },
                        }
                        self.sync_capture();
                        return;
                    }
                    KeyRoute::Consumed => { self.sync_capture(); return; }
                    KeyRoute::Other => {}
                }
                if event.state == ElementState::Pressed && self.debug_key(code, event.repeat) { return; }
                if self.game.text_input() && event.state == ElementState::Pressed {
                    // Text entry: editing keys and typed text go to the game;
                    // only menu movement still presses buttons.
                    let cmd = self.modifiers.control_key() || self.modifiers.super_key();
                    let ev = match code {
                        KeyCode::Escape if !event.repeat => Some(TextEvent::Escape),
                        KeyCode::Enter | KeyCode::NumpadEnter if !event.repeat => Some(TextEvent::Enter),
                        KeyCode::Backspace => Some(TextEvent::Backspace),
                        KeyCode::Delete => Some(TextEvent::Delete),
                        KeyCode::ArrowLeft => Some(TextEvent::Left),
                        KeyCode::ArrowRight => Some(TextEvent::Right),
                        KeyCode::Home => Some(TextEvent::Home),
                        KeyCode::End => Some(TextEvent::End),
                        KeyCode::KeyV if cmd && !event.repeat => Some(TextEvent::Paste(clipboard::get())),
                        KeyCode::KeyC if cmd && !event.repeat => Some(TextEvent::Copy),
                        KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Tab => None,
                        _ => {
                            let typed = event.text.as_ref().filter(|t| !cmd && !self.composing && t.chars().any(|c| !c.is_control()));
                            if let Some(t) = typed { self.game.text(TextEvent::Insert(t.to_string())); }
                            return;
                        }
                    };
                    if let Some(ev) = ev { self.game.text(ev); self.sync_capture(); return; }
                    // Tab leaves the field: a tap of Down for the menu.
                    if code == KeyCode::Tab { self.devices.key(KeyCode::ArrowDown, ElementState::Pressed, false); self.devices.key(KeyCode::ArrowDown, ElementState::Released, false); return; }
                }
                if event.state == ElementState::Pressed && !event.repeat {
                    let cmd = self.modifiers.control_key() || self.modifiers.super_key();
                    if code == KeyCode::KeyV && cmd {
                        self.game.text(TextEvent::Paste(clipboard::get()));
                        self.sync_capture();
                        return;
                    }
                    match code {
                        KeyCode::Escape => {
                            if self.game.escape() { self.sync_capture(); return; }
                            if self.devices.mouse_captured { self.capture_mouse(false) } else { event_loop.exit() }
                            return;
                        }
                        _ => {}
                    }
                }
                self.devices.key(code, event.state, event.repeat);
            }
            // Off screen, `about_to_wait` drives the frames.
            WindowEvent::RedrawRequested if !self.manners.offscreen() => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
        // Invisible and minimised windows may get no redraw events (Win32),
        // yet a warm or paused game must keep answering its host, and a new
        // window must draw the first frame it appears with.
        if self.window.is_some() && self.manners.offscreen() && !event_loop.exiting() { self.redraw(event_loop); }
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    #[derive(Default)]
    struct ClaimsAllKeys { received: Vec<u8>, typing: bool }
    impl Game for ClaimsAllKeys {
        fn local_players(&self) -> usize { 0 }
        fn tick(&mut self, _: &[Controls]) {}
        fn draw(&mut self, _: &mut Frame) -> Result<Draw, String> { unreachable!() }
        fn text_input(&self) -> bool { self.typing }
        fn function_key(&mut self, n: u8) -> bool { self.received.push(n); true }
    }
    #[test]
    fn slow_render_leaves_time_to_drain_mouse_backlog() {
        use std::time::Duration;
        // A 12 ms draw missed 120 Hz. All 100 already-queued packets in a
        // 0.5 ms dispatch burst must reach input before another forced draw.
        for packet in 1..=100 {
            let dispatch = Duration::from_micros(packet * 5);
            assert!(!input_redraw_due(Duration::from_millis(12) + dispatch, dispatch, 120.0));
        }
        assert!(input_redraw_due(Duration::from_millis(13), Duration::from_millis(1), 120.0));
        assert!(!input_redraw_due(Duration::from_millis(7), Duration::from_millis(3), 120.0));
        assert!(input_redraw_due(Duration::from_millis(9), Duration::from_millis(5), 120.0));
    }

    #[test]
    fn engine_shortcuts_cannot_be_claimed_by_games_or_text_fields() {
        for typing in [false, true] {
            let mut game = ClaimsAllKeys { typing, ..Default::default() };
            for (code, action) in [(KeyCode::F10, EngineShortcut::DebugMenu), (KeyCode::F11, EngineShortcut::Fullscreen), (KeyCode::F12, EngineShortcut::Screenshot)] {
                assert_eq!(route_function_key(&mut game, code, ElementState::Pressed, false), KeyRoute::Engine(action));
                assert_eq!(route_function_key(&mut game, code, ElementState::Pressed, true), KeyRoute::Consumed);
                assert_eq!(route_function_key(&mut game, code, ElementState::Released, false), KeyRoute::Consumed);
            }
            assert!(game.received.is_empty());
            for code in [KeyCode::F3, KeyCode::F9] {
                assert_eq!(route_function_key(&mut game, code, ElementState::Pressed, false), KeyRoute::Consumed);
                route_function_key(&mut game, code, ElementState::Pressed, true);
                route_function_key(&mut game, code, ElementState::Released, false);
            }
            assert_eq!(game.received, [3, 9]);
            assert_eq!(route_function_key(&mut game, KeyCode::KeyW, ElementState::Pressed, false), KeyRoute::Other);
        }
    }
}
