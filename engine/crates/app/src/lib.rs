//! A game shell for Warp: the window, surface and presentation, a fixed
//! simulation tick with interpolation, local player input, split-screen
//! frames with a HUD, and the same frame path headless for screenshots and
//! benchmarks. A game implements [`Game`]; the shell owns the GPU.
//!
//! Built-in keys: click captures the mouse, Esc releases it (or quits when
//! released). F2 always opens the engine debug menu ([`debug`]), Alt+Enter toggles
//! fullscreen, F4 opens point selection on a feedback screenshot (PNG, JSON and the clipboard;
//! see [`screenshot`]); Shift+F4, or F4 twice, then a dragged rectangle
//! saves a crop of it. These reserved keys run before game/menu/text input.
//! F1 toggles all editor windows. F5 through F12 belong to games. F3 opens the engine's stats overlay ([`stats`], Shift+F3 for depth).
//!
//! Text entry: while [`Game::text_input`] is true the keyboard types
//! ([`Game::text`] gets characters, IME commits, editing keys and
//! Ctrl/Cmd+V paste) instead of pressing buttons; arrows up and down and
//! Tab still move menus. Seats: [`Game::devices`] sees every device's input
//! each tick and [`Game::seats`] can bind devices to seats, so a game can
//! let a spare pad join mid-match.
pub mod clipboard;
pub mod debug;
pub mod editor;
pub mod dev_bar;
pub mod stats;
pub mod input;
pub mod text;
pub mod script;
pub mod sound;
pub mod tween;
pub mod ui;
pub mod manners;
pub mod startup;
pub mod screenshot;
pub mod feedback;
pub mod quality;
pub mod emulate;
mod frame_log;
pub mod profile;
pub mod frame_rate;
pub mod back_chord;
pub mod bench;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// A host in this program takes the game back to its lobby (a browser page
/// with its room, [`web::on_exit`]): games offer "Back to lobby" then.
pub fn host_takes_back() -> bool {
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    { web::takes_back() }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    { false }
}
#[cfg(target_arch = "wasm32")]
mod web_pads;
/// The Android shell (a NativeActivity with Vulkan): the browser shell's
/// API ([`web`] there), so the Game Room's browser host runs natively on a
/// phone. See engine/docs/android-host.md.
#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
pub use android as web;
#[cfg(target_os = "android")]
mod android_pads;
pub use frame_rate::FrameRate;

pub mod couch;
pub mod net;
pub use couch::{couch_order, couch_views, in_view_order, seat_couch, set_seat_couch};
pub use input::{Buttons, Controls, Device, Devices, Pointer, Tuning, key_label, keyboard_split, seat_device, set_seat_devices, split_keyboard};
pub use text::{FieldEvent, TextEvent, TextField};
use render::hud::{HudDrawList, HudRenderer};
use render::present::Presenter;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::sync::Arc;
use web_time::Instant;
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
    /// The window has the keyboard focus (another app or game is not in front).
    pub focused: bool,
    /// The display, when the window shows HDR output (fp16 scRGB/EDR);
    /// `None` in SDR, headless and captures. The headroom in use is
    /// `Renderer::hdr_headroom`.
    pub hdr: Option<render::hdr::HdrDisplay>,
    stream_generated_textures: bool,
    renderer: &'a mut Option<Renderer>,
}

impl<'a> Frame<'a> {
    /// Frame access for an external presentation shell, including OpenXR.
    pub fn new(gpu: &'a Gpu, renderer: &'a mut Option<Renderer>, size: [u32; 2], dt: f32, alpha: f32) -> Self {
        Self { gpu, renderer, size, dt, alpha, stream_generated_textures: true, split: render::SplitLayout::SideBySide, scene_fps: if dt > 0.0 { 1.0 / dt } else { 0.0 }, scene_worst_ms: dt * 1000.0, focused: true, hdr: None }
    }

    /// Upload a scene: creates the renderer, or replaces it.
    pub fn set_scene(&mut self, scene: &Scene, light: &Lighting) {
        let t = web_time::Instant::now();
        *self.renderer = Some(Renderer::with_generated_streaming(self.gpu, scene, light, self.stream_generated_textures));
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

/// How often a hidden (warm) game draws a scene; see [`Game::hidden_frame_due`].
const HIDDEN_DRAW_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

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
    /// A `cmd` step of a `script` run. Defaults to [`Game::command`]; a game
    /// may keep a scripted run from writing to its source files.
    fn script_command(&mut self, cmd: &str) -> Result<String, String> { self.command(cmd) }
    /// How the window should be: the shell follows changes every frame
    /// (a party host keeps a warm game hidden until its match starts).
    fn screen(&self) -> Screen { Screen::Normal }
    /// The game wants this frame in memory (a live-control screenshot): the
    /// shell renders it offscreen as well and passes it to [`Game::captured`].
    fn wants_capture(&self) -> bool { false }
    /// The frame asked for by [`Game::wants_capture`], RGBA8, top row first.
    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) { let _ = (size, rgba); }
    /// Attach a gameplay replay beside an F4 screenshot. Called only on
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
    /// Whether any game-owned editor window is visible in this scene.
    fn scene_tuning(&self) -> Option<&editor::SceneTuning> { None }
    fn scene_tuning_mut(&mut self) -> Option<&mut editor::SceneTuning> { None }
    fn editor_shortcut(&self, key: u8) -> Option<(&'static str, bool)> {
        if key == 12 { self.scene_tuning().map(|t| (t.title, t.ui.open)) } else { None }
    }
    fn editor_windows_open(&self) -> bool { self.scene_tuning().is_some_and(|t|t.ui.open) }
    fn set_editor_windows_open(&mut self, open: bool) { if let Some(t)=self.scene_tuning_mut() { t.set_visible(open); } }
    /// Handle game shortcuts from F12 down through F5.
    fn function_key(&mut self, n: u8) -> bool {
        if n == 12 && let Some(tuning) = self.scene_tuning_mut() {
            tuning.set_visible(!tuning.ui.open);
            return true;
        }
        false
    }
    /// Lines (label, value) for the F3 stats overlay's Game section: what
    /// the game streams, builds or simulates. Asked for a few times a second
    /// while the overlay is open, never otherwise.
    fn debug_stats(&self) -> Vec<(String, String)> { Vec::new() }
    /// The game asks the shell to close (an Exit menu item).
    fn quit(&self) -> bool { false }
    /// The game's own menu asked to go back to the host's lobby, with the
    /// game kept paused (as Back does); true once per request. Only offered
    /// when [`host_takes_back`] holds.
    fn take_back(&mut self) -> bool { false }
    /// A player picked up a controller while this game runs (a browser host
    /// that showed "press A to join"): add them to the game in progress, as
    /// the next player in controls order. `name` and `color` (sRGB) are
    /// their look, when the host knows it. False: this game cannot take
    /// players mid-game (the host keeps them for the next one).
    fn player_joins(&mut self, name: Option<&str>, color: Option<[f32; 3]>) -> bool { let _ = (name, color); false }
    /// Extra menus and controls for the F2 debug menu, with ids from
    /// [`debug::GAME_IDS`] up (a control's `menu` may name an engine menu).
    fn debug(&self) -> (Vec<render::debug_ui::DebugMenu>, Vec<render::debug_ui::DebugControl>) { Default::default() }
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
    /// Whether someone picking up a controller now can join this game
    /// ([`Game::player_joins`]); the host then shows "Press A to join"
    /// instead of "you join from the next game".
    fn takes_joins(&self) -> bool { false }
    /// Play online with the couches on `wire` (net.rs): true when this game
    /// can. Called once, after loading, before the first tick.
    fn go_online(&mut self, wire: Box<dyn net::Wire>) -> bool { let _ = wire; false }
    /// A line to show along the bottom of the screen for a few seconds,
    /// asked for after every frame's ticks: under a desktop party host,
    /// "Press A to join this game" when a spare controller is touched.
    fn notice(&mut self) -> Option<String> { None }
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
    /// Keep the authored effects at every quality level while adapting the 3D resolution.
    fn adaptive_resolution_only(&self) -> bool { false }
    /// True while frame times say nothing about this game's own cost, such
    /// as the moments a lobby takes the screen back while the game that had
    /// it still renders: adaptive quality neither drops nor climbs then.
    fn hold_quality(&self) -> bool { false }
    /// Hidden behind the lobby or another game (a warm game), the shell draws
    /// a scene only about once a second, so it leaves the GPU to the game on
    /// screen. True asks for the next frame anyway: a game that reports ready
    /// once a frame of its new match is drawn.
    fn hidden_frame_due(&self) -> bool { false }
    /// Adaptive quality has been on its lowest rung and still missed the
    /// target for [`quality::TOO_HEAVY_AFTER`]: resolution and grass are spent,
    /// so a game with cheaper content (fewer effects, simpler scenery) can
    /// switch to it. Called again after as long if it is still too slow.
    fn content_too_heavy(&mut self) {}
    /// A representative scene is on screen (behind a start menu, say) and
    /// the game wants to know what it costs here: Some(key) asks the shell to
    /// measure it once per key ([`ProbeResult`] to [`Game::probed`]). The HUD
    /// keeps its frame rate meanwhile; the scene renders every other frame.
    fn probe_scene(&self) -> Option<u64> { None }
    fn probed(&mut self, _result: ProbeResult) {}
    /// A line about adaptive quality or a probe, for the game's own log.
    fn quality_event(&mut self, _line: &str) {}
    /// Target framerate for dynamic adaptive graphics quality (e.g. Some(60.0)).
    /// None falls back to `Config::target_fps`, `WARP_TARGET_FPS`, or the monitor refresh rate.
    fn target_fps(&self) -> Option<f32> { None }
    /// HDR output on an HDR screen, as the game's own settings say: Some(true)
    /// asks for it, Some(false) refuses it, None follows `WARP_HDR` (off when
    /// unset). The F2 menu overrides both. It needs a screen and driver that
    /// offer an HDR surface (Windows with HDR on, macOS EDR, some Vulkan);
    /// elsewhere it stays SDR. Screenshots and captures are always SDR.
    fn hdr(&self) -> Option<bool> { None }
}

/// `WARP_HDR`: 1/on/true asks for HDR output, 0/off/false refuses it.
fn hdr_from_env() -> Option<bool> {
    match std::env::var("WARP_HDR").ok()?.to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" | "auto" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// `WARP_HDR_HEADROOM`: the highlight headroom to use instead of what the
/// display reports (4 shows the brightest highlights 4x SDR white).
fn hdr_headroom_from_env() -> Option<f32> {
    std::env::var("WARP_HDR_HEADROOM").ok()?.parse::<f32>().ok().filter(|h| h.is_finite() && *h >= 1.0)
}

/// What a start probe measured: GPU milliseconds per scene frame (covered
/// union of the async profile, so vsync hides nothing) at adaptive
/// quality's top rung.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProbeResult {
    pub key: u64,
    pub frames: usize,
    pub gpu_ms_median: f64,
    pub gpu_ms_p90: f64,
    /// The 3D picture's size and the window's.
    pub render_size: [u32; 2],
    pub output_size: [u32; 2],
    /// The frame rate adaptive quality aims for here.
    pub target_fps: f64,
    pub adapter: String,
}

/// A probe in progress.
struct Probe { key: u64, started: Instant, render_size: [u32; 2] }
/// Probe samples before this are pipeline and cache warmup.
const PROBE_WARMUP: std::time::Duration = std::time::Duration::from_millis(300);
const PROBE_FRAMES: usize = 60;
const PROBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Engine-owned keys, shared with alternate presentation shells such as Sideways.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineShortcut { AllMenus, DebugMenu, Fullscreen, Screenshot }

/// Reserved even when the game is paused, reviewing a replay or accepting text.
pub fn engine_shortcut(code: KeyCode) -> Option<EngineShortcut> {
    match code {
        KeyCode::F1 => Some(EngineShortcut::AllMenus),
        KeyCode::F2 => Some(EngineShortcut::DebugMenu),
        KeyCode::F4 => Some(EngineShortcut::Screenshot),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum KeyRoute { Engine(EngineShortcut), Stats { shift: bool }, Consumed, Other }

fn restored_panel_open(code: KeyCode, graphics: bool, profiler: bool, game_open: impl Fn(u8) -> bool) -> bool {
    use KeyCode::*;
    match code {
        F2 => graphics,
        F3 => profiler,
        _ => [F5,F6,F7,F8,F9,F10,F11,F12].iter().position(|&key| key == code)
            .is_some_and(|i|game_open(i as u8 + 5)),
    }
}

/// Route every function-key edge before either the game or text/navigation handlers.
fn route_function_key(game: &mut dyn Game, code: KeyCode, state: ElementState, repeat: bool, shift: bool) -> KeyRoute {
    let press = state == ElementState::Pressed && !repeat;
    if let Some(action) = engine_shortcut(code) {
        return if press { KeyRoute::Engine(action) } else { KeyRoute::Consumed };
    }
    use KeyCode::*;
    if code == F3 {
        return if press { KeyRoute::Stats { shift } } else { KeyRoute::Consumed };
    }
    if let Some(n) = [F5,F6,F7,F8,F9,F10,F11,F12].iter().position(|&k| k==code) {
        if press { game.function_key(n as u8+5); }
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

impl Screen {
    /// A game under a party host: on screen only while the session runs, and
    /// only when a daemon launched it; standalone it is as configured.
    pub fn for_party(launched_by_daemon: bool, running: bool) -> Screen {
        if !launched_by_daemon { Screen::Normal } else if running { Screen::Fullscreen } else { Screen::Hidden }
    }
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
        let presenter = Presenter::new(gpu, format);
        let hud = HudRenderer::new(gpu, presenter.hud_format(format))?;
        Ok(Self { presenter, hud, picture: None, rects: Vec::new(), timings: Default::default() })
    }
    fn draw(&mut self, gpu: &Gpu, renderer: &Renderer, draw: &Draw, size: [u32; 2], split: render::SplitLayout, target: &wgpu::TextureView, max_height: Option<u32>) -> Result<(), String> {
        self.draw_with(gpu, renderer, draw, size, split, target, max_height, true)
    }
    /// With `scene` false the last scene picture is presented again under a
    /// new HUD (a debug menu stays responsive when the scene is slow).
    #[allow(clippy::too_many_arguments)]
    fn draw_with(&mut self, gpu: &Gpu, renderer: &Renderer, draw: &Draw, size: [u32; 2], split: render::SplitLayout, target: &wgpu::TextureView, max_height: Option<u32>, scene: bool) -> Result<(), String> {
        self.prepare(gpu, renderer, draw, size, split, max_height, scene)?;
        let picture = self.picture.as_ref().unwrap();
        let started = Instant::now();
        // An HDR surface shows the renderer's HDR copy when it matches this picture.
        let hdr = if self.presenter.is_hdr() { renderer.hdr_output().filter(|t| t.size() == picture.size()) } else { None };
        self.presenter.present_hdr(gpu, picture, hdr.as_ref(), target, Some(&self.hud), None)?;
        self.timings.present_ms = started.elapsed().as_secs_f64() * 1000.;
        Ok(())
    }

    fn prepare(&mut self, gpu: &Gpu, renderer: &Renderer, draw: &Draw, size: [u32; 2],
        split: render::SplitLayout, max_height: Option<u32>, scene: bool) -> Result<(), String> {
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
        (self.timings.hud_vertices, self.timings.hud_text_runs) = (draw.hud.vertices.len(), draw.hud.text.len());
        Ok(())
    }

}

/// What the F3 stats overlay shows about the renderer.
fn render_info(gpu: &Gpu, renderer: &Renderer, output: [u32; 2]) -> stats::RenderInfo {
    let upscale = renderer.upscale_status();
    stats::RenderInfo {
        adapter: gpu.info.name.clone(),
        backend: format!("{:?}", gpu.info.backend),
        output,
        views: upscale.views.iter().map(|v| (v.input, v.output)).collect(),
        upscale: upscale.mode.as_str().into(),
        foliage_pages: renderer.foliage_page_stats().map(|p| (p.resident_pages, p.pending_pages, p.allocated_bytes)),
    }
}

/// A screenshot's line along the bottom of the screen.
/// A frozen frame waiting for its crop rectangle (Shift+F4): shown dimmed
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
    /// Cap the 3D picture's height as a window does (the HUD stays native).
    pub max_height: Option<u32>,
    renderer: Option<Renderer>,
    out: Output,
    target: wgpu::Texture,
    last_draw_timings: profile::DrawTimings,
    emulator: emulate::Emulator,
    profiling: bool,
    /// The F3 stats overlay, drawn into captures when open (`WARP_STATS`).
    /// While open it takes the GPU samples: callers draining the profile
    /// themselves get none.
    pub stats: stats::Stats,
    last_render: Option<Instant>,
}

impl<'g> Headless<'g> {
    /// The GPU forest's last selection counters, when the scene has one.
    pub fn forest_diagnostics(&self) -> Result<Option<render::foliage_runtime::Diagnostics>, String> {
        match &self.renderer { Some(r) => r.gpu_forest_diagnostics(self.gpu), None => Ok(None) }
    }

    pub fn new(gpu: &'g Gpu, size: [u32; 2], tick_hz: f32) -> Result<Self, String> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        Ok(Self { gpu, size, tick_hz, audio: None, recorded: Vec::new(), max_height: None, renderer: None, out: Output::new(gpu, format)?, target: offscreen(gpu, size, format), last_draw_timings: Default::default(), emulator: Default::default(), profiling: false, stats: stats::Stats::from_env(), last_render: None })
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
        let mut frame = Frame { gpu: self.gpu, size: self.size, dt: 1.0 / self.tick_hz, alpha, split, scene_fps: self.tick_hz, scene_worst_ms: 1000.0 / self.tick_hz, focused: true, hdr: None, stream_generated_textures: false, renderer: &mut self.renderer };
        let started = Instant::now();
        let mut draw = game.draw(&mut frame)?;
        let game_ms = started.elapsed().as_secs_f64() * 1000.;
        let renderer = self.renderer.as_ref().ok_or("the game drew without a scene")?;
        if self.stats.open() {
            if !self.profiling { self.profiling = renderer.set_async_gpu_profiling(self.gpu, true).is_ok(); }
            let samples = renderer.drain_gpu_profiles(self.gpu).samples;
            self.stats.gpu(&samples);
            self.stats.memory(self.gpu);
            if self.stats.wants_info() { self.stats.set_info(render_info(self.gpu, renderer, self.size)); }
            let game = &*game;
            self.stats.draw(&mut draw.hud, || game.debug_stats());
        }
        self.out.draw(self.gpu, renderer, &draw, self.size, split, &self.target.create_view(&Default::default()), self.max_height)?;
        if self.stats.open() {
            let now = Instant::now();
            let t = self.out.timings;
            let frame_ms = self.last_render.map_or(0.0, |l| (now - l).as_secs_f64() * 1000.0);
            if frame_ms > 0.0 { self.stats.cpu(frame_ms, &[("game draw", game_ms), ("render encode + submit", t.render_ms), ("HUD", t.hud_ms), ("present", t.present_ms)]); }
            self.last_render = Some(now);
        }
        if emulate::active() {
            emulate::set_host(&self.gpu.info.name);
            if !self.profiling { self.profiling = renderer.set_async_gpu_profiling(self.gpu, true).is_ok(); }
            let ms = renderer.drain_gpu_profiles(self.gpu).samples.last().map(|s| s.covered_union_ms);
            self.emulator.scene(self.gpu, ms);
        }
        self.last_draw_timings = profile::DrawTimings { game_ms, ..self.out.timings };
        Ok(self.last_draw_timings)
    }
    /// Draw one more frame through the measuring path, which waits for the GPU
    /// and returns its counters (clusters, triangles, shadow work, pass times).
    /// For developer captures: never count this frame in steady-frame timing.
    pub fn measure(&mut self, game: &mut dyn Game) -> Result<render::FrameStats, String> {
        let split = game.split();
        let mut frame = Frame { gpu: self.gpu, size: self.size, dt: 1.0 / self.tick_hz, alpha: 1.0, split, scene_fps: self.tick_hz, scene_worst_ms: 1000.0 / self.tick_hz, focused: true, hdr: None, stream_generated_textures: false, renderer: &mut self.renderer };
        let draw = game.draw(&mut frame)?;
        let renderer = self.renderer.as_ref().ok_or("the game drew without a scene")?;
        let rects = render::split_rects_with(draw.cameras.len(), self.size[0], self.size[1], split);
        let views: Vec<ViewRect> = draw.cameras.iter().zip(&rects).map(|(&cam, &(x, y, w, h))| ViewRect { cam, x, y, w, h }).collect();
        renderer.try_render_texture_profiled(self.gpu, &views, &draw.light, self.size[0], self.size[1]).map(|(_, stats)| stats)
    }
    /// CPU submission stages of the last rendered frame, excluding pixel
    /// readback. These can include driver waits; GPU timestamps are separate.
    pub fn last_draw_timings(&self) -> profile::DrawTimings { self.last_draw_timings }
    pub fn renderer(&self) -> Option<&Renderer> { self.renderer.as_ref() }
}

/// Headless benchmark: `frames` frames at `size` with idle controls, no
/// window. Returns frames per second (wall time, including readback) and
/// the smoothed GPU milliseconds per pass.
pub fn bench(game: &mut dyn Game, gpu: &Gpu, size: [u32; 2], frames: u32, tick_hz: f32) -> Result<(f64, Vec<(&'static str, f64)>), String> {
    let mut h = Headless::new(gpu, size, tick_hz)?;
    let mut profile = debug::EngineDebug::default();
    let players = game.local_players();
    let warmup = std::env::var("WARP_BENCH_WARMUP").ok().and_then(|s|s.parse::<u32>().ok()).unwrap_or(2).min(frames.saturating_sub(1));
    let mut started = Instant::now();
    for i in 0..frames {
        // Default: omit two startup frames. WARP_BENCH_WARMUP permits a
        // longer explicit warmup for pipeline/temporal steady-state comparisons.
        if i == warmup { started = Instant::now(); }
        game.tick(&vec![Controls::default(); players]);
        h.frame(game)?;
        if let Some(r) = h.renderer() { let _ = profile.profile(gpu, r, true); }
    }
    Ok((f64::from(frames.saturating_sub(warmup)) / started.elapsed().as_secs_f64(), profile.passes().to_vec()))
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
    instance: wgpu::Instance,
    feedback_hud: Option<HudRenderer>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    adapter: wgpu::Adapter,
    /// The surface is configured for HDR (fp16 scRGB) right now.
    hdr: bool,
    /// What the surface and display offer, polled every few seconds while
    /// HDR is wanted (the OS can switch HDR on and off at any time).
    hdr_supported: bool,
    display: render::hdr::HdrDisplay,
    display_at: Option<Instant>,
    /// HDR was asked for and refused (logged once).
    hdr_refused: bool,
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
            let (gpu, config, adapter) = Gpu::for_surface_with_adapter(&instance, &surface, size.width.max(1), size.height.max(1))?;
            Ok((instance, surface, gpu, config, adapter, surfaced))
        };
        let (instance, surface, gpu, config, adapter, surfaced) = match render::gpu::window_instance().map(&open) {
            Some(Ok(v)) => v,
            Some(Err(e)) => { println!("{} Vulkan: {e}; trying the other backends", manners::stamp()); open(render::gpu::instance())? }
            None => open(render::gpu::instance())?,
        };
        let devised = t.elapsed().as_millis();
        surface.configure(&gpu.device, &config);
        let out = Output::new(&gpu, config.format)?;
        let lost = watch(&gpu);
        Ok((Live { instance, feedback_hud:None, surface, config, adapter, hdr: false, hdr_supported: false, display: Default::default(), display_at: None, hdr_refused: false, gpu, out, renderer: None, lost }, [surfaced, devised]))
    }

    fn is_lost(&self) -> bool { self.lost.load(std::sync::atomic::Ordering::Relaxed) }

    /// Switch the surface between HDR and SDR when `want` and what it shows
    /// differ, and keep the display's figures fresh. Returns a line to log
    /// when something changed.
    fn sync_hdr(&mut self, want: bool) -> Result<Option<String>, String> {
        let due = self.display_at.is_none_or(|t| t.elapsed().as_secs_f32() >= 2.0);
        if (want || self.hdr) && due {
            let caps = self.surface.get_capabilities(&self.adapter);
            self.hdr_supported = render::hdr::surface_supports_hdr(&caps);
            self.display = render::hdr::HdrDisplay::query(&self.surface, &self.adapter);
            self.display_at = Some(Instant::now());
        }
        let on = want && self.hdr_supported;
        if on == self.hdr {
            let refused = want && !self.hdr_supported;
            let say = refused && !self.hdr_refused;
            self.hdr_refused = refused;
            return Ok(say.then(|| "HDR output asked for, but this screen and driver offer no HDR surface; staying SDR".into()));
        }
        let caps = self.surface.get_capabilities(&self.adapter);
        self.hdr = render::hdr::configure_surface(&mut self.config, &caps, on);
        self.surface.configure(&self.gpu.device, &self.config);
        // Presenter and HUD pipelines are made for the surface's format.
        self.out = Output::new(&self.gpu, self.config.format)?;
        self.feedback_hud=None;
        Ok(Some(if self.hdr { format!("HDR output on ({:?}, {})", self.config.format, self.display.describe()) } else { "HDR output off".into() }))
    }
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

/// Started by or pointed at a Splitscreen Club host (GAMENIGHT=1 or GAMENIGHT_ADDR).
fn under_host() -> bool {
    std::env::var("GAMENIGHT").as_deref() == Ok("1") || std::env::var("GAMENIGHT_ADDR").is_ok_and(|a| !a.is_empty())
}

struct Shell<'g> {
    cfg: Config,
    /// The F3 stats overlay.
    stats: stats::Stats,
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
    /// When a hidden window last drew a scene; see [`Game::hidden_frame_due`].
    hidden_drawn: Option<Instant>,
    mouse_events: u32,
    mouse_delta: [f64; 2],
    mouse_peak: f64,
    cursor_events: u32,
    windows: render::editor_window::WindowStack,
    cursor_position: Option<[f32;2]>,
    // Some(None) captures a drag begun on the scene, even across a window.
    pointer_owner: Option<Option<u64>>,
    pressed_mouse_buttons: u8,
    hidden_layout: Option<[bool;4]>,
    dev_mode: bool,
    dev_bar: dev_bar::DevBar,
    bar_press: bool,
    started: Instant,
    frames: u64,
    screenshot: bool,
    /// Shift+F4: after the frame is saved, drag a rectangle of it.
    crop_requested: bool,
    crop: Option<Crop>,
    picker: Option<feedback::Picker>,
    feedback_writer: Option<feedback::Writer>,
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
    probe: Option<Probe>,
    /// The last probe key measured (or abandoned).
    probed: Option<u64>,
    emulator: emulate::Emulator,
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
    let stats=stats::Stats::from_env();
    let dev_mode=debug.open() || stats.open() || game.editor_windows_open();
    let mut shell = Shell { stats, dev_mode, dev_bar:Default::default(), bar_press:false, devices, cfg, game, live: None, window: None, screen: Screen::Normal, manners: manners::Manners::new(), focused: true, acc: 0.0, last: Instant::now(), last_frame_end: Instant::now(), hidden_drawn: None, mouse_events: 0, mouse_delta: [0.0; 2], mouse_peak: 0.0, cursor_events: 0, windows: Default::default(), cursor_position:None, pointer_owner:None, pressed_mouse_buttons:0, hidden_layout:None, started: Instant::now(), frames: 0, screenshot: false, crop_requested: false, crop: None, picker: None, feedback_writer: None, shift_down: false, toast: None, fps: 0.0, rate: FrameRate::new(), debug, quality, screen_refresh_hz: None, last_scene: Instant::now(), probe: None, probed: None, emulator: Default::default(), scene_frames: 0, loop_frames: 0, stats_at: Instant::now(), error: None, modifiers: ModifiersState::empty(), composing: false, ime: false, zero_size: false, rebuilt: None };
    let events = EventLoop::new().map_err(|e| e.to_string())?;
    events.run_app(&mut shell).map_err(|e| e.to_string())?;
    shell.error.map_or(Ok(()), Err)
}

impl Shell<'_> {
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(picker) = &mut self.picker {
            if let (Some(l), Some(w)) = (&mut self.live, &self.window) {
                if l.is_lost() {
                    self.end_picker("Feedback device lost".into());
                } else if let Err(e) = picker.show(
                    w,
                    &l.gpu,
                    &l.out.presenter,
                    &mut l.feedback_hud,
                    &mut l.surface,
                    &l.instance,
                    &l.config,
                ) {
                    self.end_picker(e);
                }
            }
            return;
        }
        if let Some(e) = self.feedback_writer.as_ref().and_then(|w| w.error()) {
            self.toast = Some((e, Instant::now() + std::time::Duration::from_secs(4)));
        }
        if let Err(e) = self.frame() { startup::note_exit(&format!("frame error: {e}")); self.error = Some(e); event_loop.exit(); return; }
        if self.game.quit() { startup::note_exit("the game quit"); event_loop.exit(); return; }
        self.frames += 1;
        if self.frames % 60 == 0 { if let (Some(w), Some(t)) = (&self.window, self.game.title()) { w.set_title(&format!("{} | {t}", self.cfg.title)); } }
        if self.cfg.frames > 0 && self.frames >= self.cfg.frames {
            let s = self.started.elapsed().as_secs_f64();
            println!("smoke: {} frames in {s:.1} s ({:.1} fps)", self.frames, self.frames as f64 / s);
            for (pass, ms) in self.debug.passes() { println!("  gpu {pass:>28} {ms:6.2} ms"); }
            event_loop.exit();
            return;
        }
        if self.picker.is_none() && let Some(w) = &self.window { w.request_redraw(); }
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
            // A probe measures the top rung; adaptation resumes after it.
            q.resolution_only = self.game.adaptive_resolution_only();
            q.set_enabled(self.debug.adaptive_quality() && !self.game.fixed_quality() && self.probe.is_none());
            let target = match self.debug.target_fps() {
                Some(user) => user,
                None => self.game.target_fps().or(self.cfg.target_fps),
            };
            q.set_target_fps(target, emulate::refresh_hz().or(self.screen_refresh_hz));
            // A hidden or parked window (a warm game behind the lobby) idles
            // at a few frames a second: that says nothing about the GPU, and
            // must not cut its resolution or content before it is shown.
            let measuring = self.focused && !self.zero_size && !self.manners.offscreen() && !self.game.hold_quality();
            if !measuring { q.idle(); }
            if measuring && q.frame(f64::from(dt) * 1000.0, now) {
                let line = format!("adaptive quality: {}", q.label());
                println!("{} {} {line}", manners::stamp(), manners::process());
                self.game.quality_event(&line);
            }
            if measuring && q.content_too_heavy(now) {
                let line = format!("adaptive quality: content too heavy at {}", q.label());
                println!("{} {} {line}", manners::stamp(), manners::process());
                self.game.quality_event(&line);
                // Resolution is spent: a forest gives up detail next (coarser
                // voxels, needles nearer by) before the game cuts content.
                if let Some(tier) = render::vegetation_quality::step_down() {
                    let line = format!("adaptive quality: vegetation {}", tier.name());
                    println!("{} {} {line}", manners::stamp(), manners::process());
                    self.game.quality_event(&line);
                }
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
            let pointer = self.devices.pointer();
            let target=self.pointer_owner.unwrap_or_else(||self.windows.hit(pointer.position,size));
            let owned=match target {
                Some(editor::scene_tuning::WINDOW_ID) => self.game.scene_tuning().is_some_and(|t|t.ui.open),
                Some(render::debug_ui::WINDOW_ID) => self.debug.open(),
                Some(stats::WINDOW_ID) if self.stats.open() => { self.stats.pointer(pointer,size); true }
                _ => false,
            };
            self.game.pointer(if owned { Pointer::default() } else { pointer }, size);
            if self.pressed_mouse_buttons==0 { self.pointer_owner=None; }
            let devices = self.devices.device_controls(step);
            let controls = seat_controls(self.game, &devices);
            if frame_log::enabled() { input_look.push(controls.iter().map(|c| c.look).collect::<Vec<_>>()); }
            self.game.tick(&controls);
            self.sync_capture();
        }
        if let Some(line) = self.game.notice() { self.toast = Some((line, Instant::now() + std::time::Duration::from_secs(4))); }
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
        // A warm game keeps ticking (host messages, loading) but must not
        // render full scenes behind the game being played: one a second.
        if self.screen == Screen::Hidden && live.renderer.is_some() {
            if !self.game.hidden_frame_due() && self.hidden_drawn.is_some_and(|t| t.elapsed() < HIDDEN_DRAW_EVERY) {
                self.last_frame_end = Instant::now();
                return Ok(());
            }
            self.hidden_drawn = Some(Instant::now());
        } else { self.hidden_drawn = None; }
        // HDR output: F2 first, then the game's settings, then WARP_HDR.
        let want_hdr = self.debug.hdr().or(self.game.hdr()).or_else(hdr_from_env).unwrap_or(false);
        if let Some(line) = live.sync_hdr(want_hdr)? { println!("{} {line}", manners::stamp()); }
        let size = [live.config.width, live.config.height];
        let split = self.game.split();
        if live.renderer.is_none() { self.game.loading(75, "building the scene"); }
        // A probe draws the scene every other frame: the HUD (a start menu)
        // keeps its rate while GPU timestamps measure the scene.
        let scene = self.probe.is_none() || self.loop_frames % 2 == 0;
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
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha: (self.acc / step).clamp(0.0, 1.0), split, scene_fps: self.fps, scene_worst_ms: self.rate.worst_ms(), focused: self.focused, hdr: live.hdr.then_some(live.display), stream_generated_textures: true, renderer: &mut live.renderer };
        let draw_started=Instant::now();
        let mut draw = self.game.draw(&mut frame)?;
        draw.hud.begin_screen_space();
        let draw_ms=draw_started.elapsed().as_secs_f64()*1000.0;
        let Some(renderer) = &live.renderer else { return Err("the game drew without a scene".into()) };
        let headroom = if live.hdr { self.debug.hdr_headroom().or_else(hdr_headroom_from_env).or(live.display.headroom).unwrap_or(1.0) } else { 1.0 };
        renderer.set_hdr_headroom(headroom);
        live.out.presenter.set_white_scale(live.display.white_scale);
        self.debug.set_display(live.hdr_supported, live.hdr, headroom, &live.display);
        self.debug.set_exposure(renderer.exposure_ev100(0), renderer.metered_ev100(0));
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
        let samples = self.debug.profile(&live.gpu, renderer, self.stats.open() || self.cfg.frames > 0 || frame_log::enabled() || self.probe.is_some() || emulate::active());
        let profile_ms = profile_started.elapsed().as_secs_f64() * 1000.0;
        if self.stats.open() {
            self.stats.gpu(&samples);
            self.stats.memory(&live.gpu);
            if self.stats.wants_info() { self.stats.set_info(render_info(&live.gpu, renderer, size)); }
        }
        if let Some(t)=self.game.scene_tuning_mut().filter(|t|t.ui.open) { t.tick(f64::from(dt),size); }
        if self.debug.open() {
            let quality_label = self.quality.as_ref().map(|q| q.label());
            self.debug.refresh(self.game.debug(), self.cfg.max_render_height, self.fps, quality_label.as_deref());
            if let Some(a) = self.debug.tick(f64::from(dt), size) { self.game.debug_action(a); }
        }
        // The hint is for a developer at a desk: never on a Splitscreen Club TV,
        // nor once a controller is plugged in.
        if !self.debug.open() && !self.stats.open() && !self.devices.mouse_captured && self.cfg.keyboard && self.game.mouse_capture().is_none() && !under_host() && self.devices.pad_count() == 0 {
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
        let render_height = match emulate::display_height() { Some(cap) if render_height.unwrap_or(size[1]) > cap => Some(cap), _ => render_height };
        let adapter = live.gpu.info.name.clone();
        emulate::set_host(&adapter);
        let seconds = self.stats_at.elapsed().as_secs_f32();
        if seconds >= 2.0 {
            println!("{} frames: scene {:.1}/s, loop {:.1}/s, render {}x{} of {}x{}, debug menu {}", manners::stamp(),
                self.scene_frames as f32 / seconds, self.loop_frames as f32 / seconds,
                render_height.map_or(size[0], |h| (size[0] as u64 * h as u64 / size[1].max(1) as u64) as u32), render_height.unwrap_or(size[1]),
                size[0], size[1], if self.debug.open() { "open" } else { "closed" });
            (self.scene_frames, self.loop_frames, self.stats_at) = (0, 0, Instant::now());
        }
        let (to_file, to_game) = (std::mem::take(&mut self.screenshot), self.game.wants_capture());
        let feedback_metadata = |game: &dyn Game, debug: &debug::EngineDebug| {
            let state = game.state().map(|s| serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s)));
            let mut metadata = serde_json::json!({
                "schema": 1, "game": self.cfg.title, "title": game.title(), "state": state,
                "cameras": draw.cameras.iter().map(|c| serde_json::json!({"position": c.pos.to_array(), "target": c.target.to_array(), "fov_y_deg": c.fov_y.to_degrees(), "near": c.near})).collect::<Vec<_>>(),
                "debug": debug.metadata(), "render": {"adapter": live.gpu.info.name, "backend": format!("{:?}", live.gpu.info.backend), "driver": live.gpu.info.driver, "size": size, "fps": self.fps, "frame": self.frames, "uptime_s": self.started.elapsed().as_secs_f32()},
            });
            if let Some(q) = &self.quality {
                metadata["render"]["adaptive_quality"] = q.metadata();
            }
                metadata
            };
        let crop_requested = std::mem::take(&mut self.crop_requested);
        if (to_game && !to_file) || crop_requested {
            let target = offscreen(&live.gpu, size, wgpu::TextureFormat::Rgba8UnormSrgb);
            Output::new(&live.gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?.draw(
                &live.gpu,
                renderer,
                &draw,
                size,
                split,
                &target.create_view(&Default::default()),
                render_height,
            )?;
            let rgba = live.gpu.read_rgba8(&target);
            if crop_requested {
                let saved = screenshot::save_full(
                    screenshot::Shot {
                        title: &self.cfg.title,
                        size,
                        rgba: &rgba,
                        metadata: feedback_metadata(self.game,&self.debug),
                    },
                    |path| {
                        self.game
                            .feedback_replay(path, (self.acc / step).clamp(0., 1.))
                    },
                );
                match saved {
                    Ok(saved) => {
                        self.crop = Some(Crop {
                            saved,
                            size,
                            picture: Arc::new(render::picture::Picture::new(
                                size[0],
                                size[1],
                                rgba.clone(),
                            )?),
                            start: None,
                            cursor: [0.; 2],
                        });
                        self.devices.reset_mouse();
                        self.devices.mouse_captured = false;
                        if let Some(w) = &self.window {
                            let _ = w.set_cursor_grab(CursorGrabMode::None);
                            w.set_cursor_visible(true);
                            w.set_cursor(winit::window::CursorIcon::Crosshair);
                        }
                    }
                    Err(e) => {
                        self.toast = Some((
                            format!("Screenshot failed: {e}"),
                            Instant::now() + std::time::Duration::from_secs(4),
                        ))
                    }
                }
            }
            if to_game {
                self.game.captured(size, rgba);
            }
        }
        if self.toast.as_ref().is_some_and(|(_, until)| Instant::now() > *until) { self.toast = None; }
        if let Some(crop) = &self.crop { draw.hud.begin_overlay(); crop.draw(&mut draw.hud); }
        if let Some((line, _)) = &self.toast { toast(&mut draw.hud, line); }
        if let Some(badge) = emulate::badge() {
            let s = (size[1] as f32 / 1080.0).max(0.5);
            draw.hud.text.push(render::hud::TextRun { id: u64::MAX - 7, text: badge, position: [16.0 * s, 12.0 * s],
                bounds: [0.0, 0.0, size[0] as f32, size[1] as f32], font_size: 18.0 * s, color: [255, 190, 90, 230], font: render::hud::FontFace::Body,
                align: render::hud::TextAlign::Left, width: size[0] as f32 * 0.6 });
        }
        if self.stats.open() { let game = &*self.game; self.stats.draw(&mut draw.hud, || game.debug_stats()); }
        if self.debug.open() { self.debug.draw(&mut draw.hud); }
        if let Some(t)=self.game.scene_tuning_mut().filter(|t|t.ui.open) { t.draw(&mut draw.hud); }
        if self.hidden_layout.is_none() { self.windows.sync(&mut draw.hud); }
        if self.dev_mode {
            self.dev_bar.draw(&mut draw.hud,self.game,self.stats.open(),self.debug.open());
        }
        // Nothing to show on a window without pixels; the game keeps ticking.
        if self.zero_size || live.is_lost() { return Ok(()); }
        if to_file && !crop_requested {
            live.out.prepare(
                &live.gpu,
                renderer,
                &draw,
                size,
                split,
                render_height,
                scene,
            )?;
            let picture = live.out.picture.as_ref().unwrap();
            let mut picker = if live.hdr {
                let format = render::present::HDR_HUD_FORMAT;
                let sdr = Presenter::new(&live.gpu, format);
                let picker = feedback::Picker::new(sdr.freeze_sdr(
                    &live.gpu,
                    picture,
                    size,
                    format,
                    Some(&live.out.hud),
                )?);
                // The frame as the HDR screen shows it, held while the
                // player marks the point and saved as an HDR file.
                match renderer.hdr_output().filter(|t| t.size() == picture.size()) {
                    Some(hdr) => {
                        let copy = live.gpu.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("frozen HDR feedback frame"), size: hdr.size(), mip_level_count: 1, sample_count: 1,
                            dimension: wgpu::TextureDimension::D2, format: hdr.format(),
                            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[] });
                        let mut enc = live.gpu.device.create_command_encoder(&Default::default());
                        enc.copy_texture_to_texture(hdr.as_image_copy(), copy.as_image_copy(), hdr.size());
                        live.gpu.queue.submit([enc.finish()]);
                        picker.with_hdr(feedback::HdrFrame { texture: copy, hud: draw.hud.clone(),
                            sdr_white_nits: live.display.white_scale * 80.0, peak_nits: live.display.peak_nits })
                    }
                    None => picker,
                }
            } else {
                feedback::Picker::new(live.out.presenter.freeze_sdr(
                    &live.gpu,
                    picture,
                    size,
                    live.config.format,
                    Some(&live.out.hud),
                )?)
            };
            let w = self.window.as_ref().unwrap();
            self.devices.reset_keyboard();
            self.devices.reset_mouse();
            self.devices.mouse_captured = false;
            self.pointer_owner = None;
            self.pressed_mouse_buttons = 0;
            self.bar_press = false;
            self.shift_down = false;
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            w.set_cursor(winit::window::CursorIcon::Crosshair);
            picker.show(
                w,
                &live.gpu,
                &live.out.presenter,
                &mut live.feedback_hud,
                &mut live.surface,
                &live.instance,
                &live.config,
            )?;
            if to_game {
                self.game.captured(size, picker.read_rgba(&live.gpu));
            }
            let metadata=feedback_metadata(self.game,&self.debug);
            let queued = (|| {
                if self.feedback_writer.is_none() {
                    self.feedback_writer = Some(feedback::Writer::new()?);
                }
                self.feedback_writer.as_ref().unwrap().submit(
                    &mut picker,
                    &live.gpu,
                    &self.cfg.title,
                    metadata,
                    |path| {
                        self.game
                            .feedback_replay(path, (self.acc / step).clamp(0., 1.))
                    },
                    |_, _, _| Ok(()),
                )
            })();
            match queued {
                Ok(()) => self.picker = Some(picker),
                Err(error) => self.end_picker(format!("Feedback save failed: {error}")),
            }
            return Ok(());
        }
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
                if scene { self.emulator.scene(&live.gpu, self.debug.last_gpu_ms.take()); }
                if let Some(w) = &self.window { self.manners.presented(w, size); }
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => live.surface.configure(&live.gpu.device, &live.config),
            wgpu::CurrentSurfaceTexture::Occluded => if let Some(w) = &self.window { self.manners.occluded(w); },
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
        if self.stats.open() {
            let t = live.out.timings;
            let presenting = present_started.elapsed().as_secs_f64() * 1000.0;
            self.stats.cpu(frame_interval_ms, &[("window + input", window_ms + input_ms), ("game ticks", tick_ms), ("game draw", draw_ms),
                ("render encode + submit", t.render_ms), ("HUD", t.hud_ms), ("acquire + present", (presenting - t.render_ms - t.hud_ms).max(0.0)),
                ("GPU timestamp readback", profile_ms)]);
        }
        self.emulator.cpu(frame_started);
        self.last_frame_end = Instant::now();
        self.step_probe(size, render_height, &adapter);
        Ok(())
    }

    /// Start, sample or finish a start probe (see [`Game::probe_scene`]).
    fn step_probe(&mut self, size: [u32; 2], render_height: Option<u32>, adapter: &str) {
        let key = if self.focused && !self.zero_size { self.game.probe_scene() } else { None };
        match (&self.probe, key) {
            (None, Some(k)) if self.probed != Some(k) => {
                let h = render_height.unwrap_or(size[1]);
                let render_size = [(size[0] as u64 * h as u64 / size[1].max(1) as u64) as u32, h];
                self.debug.covered.clear();
                self.probe = Some(Probe { key: k, started: Instant::now(), render_size });
                let line = format!("probe {k:x}: measuring {}x{} of {}x{}", render_size[0], render_size[1], size[0], size[1]);
                println!("{} {line}", manners::stamp());
                self.game.quality_event(&line);
            }
            (Some(p), k) if k != Some(p.key) => {
                let line = format!("probe {:x}: abandoned (the scene changed)", p.key);
                self.game.quality_event(&line);
                self.probed = Some(p.key);
                self.probe = None;
            }
            _ => {}
        }
        let Some(p) = &self.probe else { return };
        let elapsed = p.started.elapsed();
        if elapsed < PROBE_WARMUP { self.debug.covered.clear(); return; }
        if self.debug.covered.len() < PROBE_FRAMES && elapsed < PROBE_LIMIT { return; }
        let mut ms = std::mem::take(&mut self.debug.covered);
        ms.retain(|v| v.is_finite() && *v > 0.0);
        let p = self.probe.take().unwrap();
        self.probed = Some(p.key);
        if ms.is_empty() {
            self.game.quality_event(&format!("probe {:x}: no GPU timestamps; nothing measured", p.key));
            return;
        }
        ms.sort_by(f64::total_cmp);
        let factor = f64::from(emulate::gpu_factor());
        for v in &mut ms { *v *= factor; }
        let target_fps = self.quality.as_ref().map_or(60.0, |q| f64::from(q.target_fps()));
        let result = ProbeResult { key: p.key, frames: ms.len(), gpu_ms_median: ms[ms.len() / 2], gpu_ms_p90: ms[((ms.len() - 1) as f64 * 0.9) as usize],
            render_size: p.render_size, output_size: size, target_fps, adapter: adapter.to_string() };
        let line = format!("probe {:x}: {} frames in {:.1} s, GPU median {:.2} ms, p90 {:.2} ms at {}x{} of {}x{} ({}{}, target {:.0} fps)",
            p.key, result.frames, elapsed.as_secs_f32(), result.gpu_ms_median, result.gpu_ms_p90, p.render_size[0], p.render_size[1], size[0], size[1], adapter,
            emulate::badge().map_or_else(String::new, |b| format!(", {b}, memory {:?} MB", emulate::memory_mb())), target_fps);
        println!("{} {line}", manners::stamp());
        self.game.quality_event(&line);
        self.game.probed(result);
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

    /// F2 opens and closes the debug menu; while it is open it takes the
    /// navigation keys. True when the key was used.
    fn debug_key(&mut self, code: KeyCode, repeat: bool) -> bool {
        use render::debug_ui::DebugKey;
        let scene = code != KeyCode::F2 && self.game.scene_tuning().is_some_and(|t|t.ui.open)
            && (!self.debug.open() || self.windows.frontmost_of(render::debug_ui::WINDOW_ID,editor::scene_tuning::WINDOW_ID)==Some(editor::scene_tuning::WINDOW_ID));
        let key = match code {
            KeyCode::F2 if !repeat => DebugKey::Toggle,
            _ if !self.debug.open() && !scene => return false,
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
        if scene {
            let size=self.size();
            return self.game.scene_tuning_mut().is_some_and(|t| t.key(key,size));
        }
        let was_open = self.debug.open();
        let quality_label = self.quality.as_ref().map(|q| q.label());
        self.debug.refresh(self.game.debug(), self.cfg.max_render_height, self.fps, quality_label.as_deref());
        let (used, action) = self.debug.key(key, self.size());
        if let Some(a) = action { self.game.debug_action(a); }
        // The pointer works the menu while it is open.
        if !was_open && self.debug.open() { self.capture_mouse(false); }
        used
    }

    fn end_picker(&mut self, line:String) {
        self.picker=None;
        self.acc=0.; self.last=Instant::now(); self.last_scene=self.last;
        if let Some(w)=&self.window {w.set_cursor(winit::window::CursorIcon::Default); w.request_redraw();}
        self.toast=Some((line,self.last+std::time::Duration::from_secs(4)));
        self.sync_capture();
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

    fn shortcut(&mut self, code:KeyCode, state:ElementState, repeat:bool, shift:bool) -> bool {
        if state==ElementState::Pressed && !repeat && matches!(code,KeyCode::F2|KeyCode::F3|KeyCode::F5|KeyCode::F6|KeyCode::F7|KeyCode::F8|KeyCode::F9|KeyCode::F10|KeyCode::F11|KeyCode::F12) {
            self.dev_mode=true;
            if let Some([debug,stats,game,scene])=self.hidden_layout.take() {
                self.debug.set_visible(debug);
                self.stats.set_visible(stats);
                self.game.set_editor_windows_open(game);
                if let Some(t)=self.game.scene_tuning_mut() { t.set_visible(scene); }
                // A shortcut from hidden Dev Mode is a request to show that window.
                if restored_panel_open(code, debug, stats, |key| self.game.editor_shortcut(key).is_some_and(|(_,open)|open)) {
                    self.sync_capture();
                    return true;
                }
            }
        }
        match route_function_key(self.game, code, state, repeat, shift) {
            KeyRoute::Engine(action) => {
                match action {
                    EngineShortcut::AllMenus => {
                        let visible=[self.debug.open(),self.stats.open(),self.game.editor_windows_open(),self.game.scene_tuning().is_some_and(|t|t.is_open())];
                        let restore=if self.dev_mode {
                            self.hidden_layout=Some(visible);
                            [false;4]
                        } else {
                            self.hidden_layout.take().unwrap_or([false;4])
                        };
                        self.dev_mode=!self.dev_mode;
                        self.debug.set_visible(restore[0]);
                        self.stats.set_visible(restore[1]);
                        self.game.set_editor_windows_open(restore[2]);
                        if let Some(t)=self.game.scene_tuning_mut() { t.set_visible(restore[3]); }
                        self.pointer_owner=None;
                        self.pressed_mouse_buttons=0;
                        self.devices.reset_mouse();
                    }
                    EngineShortcut::DebugMenu => { self.debug_key(KeyCode::F2, false); }
                    EngineShortcut::Fullscreen => if let Some(w) = &self.window {
                        w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                    },
                    EngineShortcut::Screenshot => if self.crop.is_none() {
                        let shift = self.modifiers.shift_key() || self.shift_down;
                        self.screenshot = true;
                        self.crop_requested = shift;
                    },
                }
                self.sync_capture();
                return true;
            }
            KeyRoute::Stats { shift } => {
                if shift { self.stats.cycle_detail() } else { self.stats.toggle() }
                if self.stats.open() {self.capture_mouse(false);} self.sync_capture();
                return true;
            }
            KeyRoute::Consumed => { self.sync_capture(); return true; }
            KeyRoute::Other => return false,
        }
    }

    fn sync_capture(&mut self) {
        if self.crop.is_some() || self.picker.is_some() { return; }
        self.devices.pointer_enabled = self.cfg.pointer || self.dev_mode || self.stats.open() || self.game.editor_windows_open();
        let capture = if self.dev_mode || self.debug.open() || self.stats.open() || self.game.editor_windows_open() { Some(false) } else { self.game.mouse_capture() };
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
                println!("{} {} adaptive quality: {}", manners::stamp(), manners::process(), q.label());
            } else {
                self.quality = quality::AdaptiveQuality::new(target, refresh, Instant::now());
                if let Some(q) = &self.quality {
                    println!("{} {} adaptive quality: {}", manners::stamp(), manners::process(), q.label());
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
        if let Err(e) = result { startup::note_exit(&format!("error: {e}")); self.error = Some(e); event_loop.exit(); }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.picker.is_some() {return;}
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
        if self.picker.is_some() {
            let size = self.size();
            match &event {
                WindowEvent::Focused(false) | WindowEvent::Occluded(true) => {
                    self.end_picker("Screenshot without a point".into())
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.physical_key == PhysicalKey::Code(KeyCode::F4)
                        && event.state == ElementState::Pressed
                        && !event.repeat =>
                {
                    let picker = self.picker.take().unwrap();
                    match picker.into_crop(&self.live.as_ref().unwrap().gpu) {
                        Ok(crop) => {
                            self.start_crop(crop);
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        }
                        Err(e) => self.end_picker(e),
                    }
                    return;
                }
                _ => {
                    if let WindowEvent::CursorMoved { position, .. } = &event {
                        self.cursor_position = Some([position.x as f32, position.y as f32]);
                    }
                    if let WindowEvent::Resized(_) = &event {
                        self.picker.as_mut().unwrap().resized();
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                    match self.picker.as_mut().unwrap().input(
                        &event,
                        self.cursor_position.map(|p| p.map(f64::from)),
                        size,
                    ) {
                        feedback::Input::Pass => {}
                        feedback::Input::Consumed => return,
                        feedback::Input::Close(line) => {
                            self.end_picker(line);
                            return;
                        }
                    }
                }
            }
        }
        match event {
            WindowEvent::CloseRequested => { startup::note_exit("window close requested"); event_loop.exit() }
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
                if !focused { self.bar_press=false; self.pointer_owner=None; self.pressed_mouse_buttons=0; self.cursor_position=None; self.devices.reset_mouse(); self.debug.cancel_interaction(); if let Some(t)=self.game.scene_tuning_mut(){t.ui.cancel_interaction();} self.capture_mouse(false); self.devices.cursor(None); }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_events = self.cursor_events.saturating_add(1);
                let p = [position.x as f32, position.y as f32];
                if let Some(crop) = &mut self.crop { crop.cursor = p; return; }
                self.cursor_position=Some(p);
                self.debug.pointer_move(p);
                if let Some(t)=self.game.scene_tuning_mut().filter(|t|t.ui.open) {t.ui.pointer_move(p);}
                self.devices.cursor(Some(p));
            }
            WindowEvent::CursorLeft { .. } => {self.cursor_position=None; self.devices.cursor(None);},
            WindowEvent::Touch(t) => { let size = self.size(); self.devices.touch(t.id, t.phase, [t.location.x as f32, t.location.y as f32], size); }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                let size=self.size();
                if self.windows.hit(self.cursor_position,size)==Some(editor::scene_tuning::WINDOW_ID)
                    && let Some(t)=self.game.scene_tuning_mut().filter(|t|t.ui.open) {t.scroll(lines,size); return;}
                if self.debug.open() && self.windows.hit(self.cursor_position,self.size())==Some(render::debug_ui::WINDOW_ID) { self.debug.scroll(lines, self.size()); return; }
                self.devices.wheel(lines);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.crop.is_some() {
                    if button == MouseButton::Left { self.crop_drag(state); }
                    return;
                }
                if button==MouseButton::Left && state==ElementState::Released && self.bar_press {
                    self.bar_press=false;
                    return;
                }
                if self.dev_mode && button==MouseButton::Left && state==ElementState::Pressed
                    && let Some(key)=self.dev_bar.hit(self.cursor_position) {
                    self.bar_press=true;
                    self.shortcut(key,state,false,false);
                    return;
                }
                let hit=if self.hidden_layout.is_some() {None} else {self.windows.hit(self.cursor_position,self.size())};
                let mask=match button { MouseButton::Left=>1, MouseButton::Right=>2, MouseButton::Middle=>4, _=>0 };
                if state==ElementState::Pressed && mask!=0 && self.pressed_mouse_buttons==0 {
                    if let Some(id)=hit {self.windows.raise(id);}
                    self.pointer_owner=Some(hit);
                }
                let target=self.pointer_owner.unwrap_or(hit);
                if state==ElementState::Pressed {self.pressed_mouse_buttons|=mask;}
                else {self.pressed_mouse_buttons &= !mask;}
                let size=self.size();
                if target==Some(editor::scene_tuning::WINDOW_ID) && let Some(t)=self.game.scene_tuning_mut().filter(|t|t.ui.open) {
                    if button==MouseButton::Left { t.press(state==ElementState::Pressed,size); }
                    return;
                }
                if target==Some(render::debug_ui::WINDOW_ID) && self.debug.open() {
                    if button != MouseButton::Left {return;}
                    let action = if state == ElementState::Pressed { self.debug.pointer_down(self.size()).1 } else { self.debug.pointer_up() };
                    if let Some(a) = action { self.game.debug_action(a); }
                    return;
                }
                if !self.dev_mode && !self.debug.open() && !self.stats.open() && !self.game.editor_windows_open() && button == MouseButton::Left && state == ElementState::Pressed && !self.devices.mouse_captured && self.cfg.keyboard && self.game.mouse_capture().is_none() {
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
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                // Keys still held when the window gains focus arrive as
                // synthetic presses: the Escape that closed a game in front
                // must not open this window's menu or quit it.
                if is_synthetic && event.state == ElementState::Pressed { return; }
                if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) { self.shift_down = event.state == ElementState::Pressed; }
                if self.crop.is_some() {
                    if code == KeyCode::Escape && event.state == ElementState::Pressed { self.end_crop("crop cancelled".into()); }
                    return;
                }
                if code==KeyCode::Enter && self.modifiers.alt_key() {
                    if event.state==ElementState::Pressed && !event.repeat && let Some(w)=&self.window {
                        w.set_fullscreen(if w.fullscreen().is_some(){None}else{Some(Fullscreen::Borderless(None))});
                    }
                    return;
                }
                let shift = self.modifiers.shift_key() || self.shift_down;
                if self.shortcut(code,event.state,event.repeat,shift) {
                    if self.screenshot && !self.crop_requested && !self.zero_size && !self.manners.offscreen() { self.redraw(event_loop); }
                    return;
                }
                if event.state == ElementState::Pressed && self.debug_key(code, event.repeat) { return; }
                if self.game.text_input() && event.state == ElementState::Pressed {
                    // Text entry: editing keys and typed text go to the game;
                    // only menu movement still presses buttons.
                    let cmd = self.modifiers.control_key() || self.modifiers.super_key();
                    let ev = match code {
                        KeyCode::Escape if !event.repeat => Some(TextEvent::Escape),
                        KeyCode::Enter | KeyCode::NumpadEnter if !event.repeat => Some(TextEvent::Enter),
                        KeyCode::KeyA if cmd => Some(TextEvent::SelectAll),
                        KeyCode::KeyX if cmd && !event.repeat => Some(TextEvent::Cut),
                        KeyCode::ArrowLeft if shift => Some(TextEvent::SelectLeft),
                        KeyCode::ArrowRight if shift => Some(TextEvent::SelectRight),
                        KeyCode::Home if shift => Some(TextEvent::SelectHome),
                        KeyCode::End if shift => Some(TextEvent::SelectEnd),
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
                            if self.devices.mouse_captured { self.capture_mouse(false) } else { startup::note_exit("Esc"); event_loop.exit() }
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
        if self.picker.is_some() {event_loop.set_control_flow(ControlFlow::Wait);return;}
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
    fn hidden_shortcut_reveals_an_already_open_panel() {
        assert!(restored_panel_open(KeyCode::F3, false, true, |_|false));
        assert!(!restored_panel_open(KeyCode::F3, true, false, |_|true));
        assert!(restored_panel_open(KeyCode::F12, false, false, |k|k==12));
        assert!(!restored_panel_open(KeyCode::F10, false, false, |k|k==12));
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
            for (code, action) in [(KeyCode::F1, EngineShortcut::AllMenus), (KeyCode::F2, EngineShortcut::DebugMenu), (KeyCode::F4, EngineShortcut::Screenshot)] {
                assert_eq!(route_function_key(&mut game, code, ElementState::Pressed, false, false), KeyRoute::Engine(action));
                assert_eq!(route_function_key(&mut game, code, ElementState::Pressed, true, false), KeyRoute::Consumed);
                assert_eq!(route_function_key(&mut game, code, ElementState::Released, false, false), KeyRoute::Consumed);
            }
            assert!(game.received.is_empty());
            for code in [KeyCode::F12, KeyCode::F11] {
                assert_eq!(route_function_key(&mut game, code, ElementState::Pressed, false, false), KeyRoute::Consumed);
                route_function_key(&mut game, code, ElementState::Pressed, true, false);
                route_function_key(&mut game, code, ElementState::Released, false, false);
            }
            assert_eq!(game.received, [12, 11]);
            assert_eq!(route_function_key(&mut game, KeyCode::KeyW, ElementState::Pressed, false, false), KeyRoute::Other);
        }
    }

    struct NoKeys;
    impl Game for NoKeys {
        fn local_players(&self) -> usize { 0 }
        fn tick(&mut self, _: &[Controls]) {}
        fn draw(&mut self, _: &mut Frame) -> Result<Draw, String> { unreachable!() }
    }

    #[test]
    fn f3_the_game_leaves_alone_opens_the_stats_overlay() {
        assert_eq!(route_function_key(&mut NoKeys, KeyCode::F3, ElementState::Pressed, false, false), KeyRoute::Stats { shift: false });
        assert_eq!(route_function_key(&mut NoKeys, KeyCode::F3, ElementState::Pressed, false, true), KeyRoute::Stats { shift: true });
        assert_eq!(route_function_key(&mut NoKeys, KeyCode::F3, ElementState::Pressed, true, false), KeyRoute::Consumed);
        assert_eq!(route_function_key(&mut NoKeys, KeyCode::F3, ElementState::Released, false, false), KeyRoute::Consumed);
        assert_eq!(route_function_key(&mut NoKeys, KeyCode::F6, ElementState::Pressed, false, false), KeyRoute::Consumed);
    }
}
