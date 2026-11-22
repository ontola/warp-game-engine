//! The shell in a browser (wasm32 with WebGPU): a canvas instead of a
//! window, the browser's frame callbacks instead of a render loop, and an
//! adapter and device that arrive asynchronously. A smaller [`crate::run`]:
//! fixed ticks, keyboard, mouse and gamepads (Gamepad API), the game's
//! frame and HUD. No debug menu, screenshots, HDR or adaptive quality yet.

use crate::{Draw, Frame, Game, Output, Screen, input::Devices};
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

thread_local! {
    /// A game waiting to replace the running one ([`switch`]).
    static NEXT: RefCell<Option<(Parked, Option<Keep>)>> = const { RefCell::new(None) };
    /// What the page does when the game quits or someone presses Back ([`on_exit`]).
    static EXIT: RefCell<Option<Box<dyn FnMut(Exit) -> bool>>> = const { RefCell::new(None) };
    /// The page's seats for games without their own ([`on_devices`]).
    static SEATS: RefCell<Option<Box<dyn FnMut(&[(crate::Device, crate::Controls)]) -> Option<Vec<Vec<crate::Device>>>>>> = const { RefCell::new(None) };
    /// Lines the log box always shows above the rolling tail: the start-up
    /// lines (GPU, limits) and the first GPU error, which later errors would
    /// otherwise push out of the box.
    static PINNED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TAIL: RefCell<std::collections::VecDeque<String>> = const { RefCell::new(std::collections::VecDeque::new()) };
}

/// Write a line to the browser console and to the page's log element
/// (`#warp-log`, when the page has one), which shows the pinned lines
/// ([`log_pinned`]) and the newest others.
pub fn log(line: &str) {
    web_sys::console::log_1(&line.into());
    TAIL.with(|t| {
        let mut t = t.borrow_mut();
        t.extend(line.lines().map(str::to_string));
        while t.len() > 10 { t.pop_front(); }
    });
    show_log();
}

/// [`log`], kept in the box for good.
pub fn log_pinned(line: &str) {
    web_sys::console::log_1(&line.into());
    PINNED.with(|p| p.borrow_mut().extend(line.lines().map(str::to_string)));
    show_log();
}

fn show_log() {
    let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("warp-log")) else { return };
    let mut text = PINNED.with(|p| p.borrow().join("\n"));
    TAIL.with(|t| for line in t.borrow().iter() { text.push('\n'); text.push_str(line); });
    el.set_text_content(Some(&(text.trim_start().to_string() + "\n")));
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

/// A request with a method, headers and a JSON body (None: no body), for
/// a web API: the HTTP status and the response body. Not an error on a
/// non-2xx status, which an API caller tells apart (401: register again).
pub async fn request(method: &str, url: &str, headers: &[(&str, &str)], json: Option<&str>) -> Result<(u16, Vec<u8>), String> {
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    let h = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    for (k, v) in headers { h.set(k, v).map_err(|e| format!("{e:?}"))?; }
    if let Some(body) = json {
        h.set("content-type", "application/json").map_err(|e| format!("{e:?}"))?;
        init.set_body(&wasm_bindgen::JsValue::from_str(body));
    }
    init.set_headers(&h);
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(|e| format!("{url}: {e:?}"))?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(|e| format!("{url}: {e:?}"))?
        .dyn_into().map_err(|_| format!("{url}: not a response"))?;
    let buffer = JsFuture::from(response.array_buffer().map_err(|e| format!("{url}: {e:?}"))?).await.map_err(|e| format!("{url}: {e:?}"))?;
    Ok((response.status(), js_sys::Uint8Array::new(&buffer).to_vec()))
}

/// The page's `localStorage` value for `key` (None when unset or blocked).
pub fn stored(key: &str) -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item(key).ok()?
}

/// Keep `value` under `key` in `localStorage` (ignored when blocked).
pub fn store(key: &str, value: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) { let _ = s.set_item(key, value); }
}

/// The page's origin (`https://splitscreen.club`).
/// A value from the page's query string.
pub fn query_param(k: &str) -> Option<String> {
    let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    query.trim_start_matches('?').split('&').find_map(|kv| kv.split_once('=').filter(|(key, _)| *key == k).map(|(_, v)| v.to_owned()))
}

pub fn origin() -> String { web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default() }

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
    hidden_editor: Option<bool>,
    /// When the last [`switch`] happened, until its first frame is drawn.
    switched: Option<Instant>,
    /// The running game has left once ([`on_exit`]): wait for the switch.
    exited: bool,
    /// Per frame: time since the last frame, ticks, the game's draw, and the
    /// engine's encode and submit, in ms ([`Shell::stats`]).
    times: Vec<[f32; 4]>,
    /// `?render=H`: draw the 3D view at most H pixels high (to tell GPU-bound
    /// from CPU-bound), whatever the game asks.
    render_height: Option<u32>,
    /// Lowers the 3D view's resolution while frames miss the game's target
    /// (60 fps unless it names one), as the native shell does.
    quality: Option<crate::quality::AdaptiveQuality>,
}

/// Replace the running game with `game` at the next frame, keeping the
/// canvas, the GPU device, the input devices and everything the engine has
/// compiled: a switch costs only the new game's scene upload.
pub fn switch(cfg: crate::Config, game: Box<dyn Game>) {
    NEXT.with(|n| *n.borrow_mut() = Some((Parked { cfg, game, renderer: None }, None)));
}

/// A game set aside by [`switch_keeping`], with its scene still on the GPU:
/// [`resume`] puts it back on screen without uploading it again.
pub struct Parked {
    pub cfg: crate::Config,
    pub game: Box<dyn Game>,
    renderer: Option<Renderer>,
}

impl Parked {
    /// A game that never ran here (its scene uploads on its first frame).
    pub fn new(cfg: crate::Config, game: Box<dyn Game>) -> Self { Parked { cfg, game, renderer: None } }
}

/// What [`switch_keeping`] hands the replaced game to.
pub type Keep = Box<dyn FnOnce(Parked)>;

/// [`switch`], handing the replaced game to `keep` as it was (it gets no
/// ticks or frames there, its scene stays on the GPU) instead of dropping
/// it. A browser host keeps its lobby that way while a game runs, and a
/// paused game while the lobby is on screen, so either comes back at once
/// with [`resume`].
pub fn switch_keeping(cfg: crate::Config, game: Box<dyn Game>, keep: impl FnOnce(Parked) + 'static) {
    NEXT.with(|n| *n.borrow_mut() = Some((Parked::new(cfg, game), Some(Box::new(keep)))));
}

/// Bring a [`Parked`] game back on screen with its scene as it was, handing
/// the one it replaces to `keep` (when given) the same way.
pub fn resume(parked: Parked, keep: Option<Box<dyn FnOnce(Parked)>>) {
    NEXT.with(|n| *n.borrow_mut() = Some((parked, keep)));
}

/// Why the running game wants to leave ([`on_exit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Its own menu's Quit ([`Game::quit`]).
    Quit,
    /// Someone pressed Back (Select, Tab on the keyboard): the game stays
    /// paused, as the host's Back does.
    Back,
}

/// Call `f` when the running game quits or someone presses Back; a browser
/// host goes back to its lobby there (a [`switch`]) and returns true. When
/// it returns false (or without it) Back is the game's like any button and
/// a quit game keeps running.
pub fn on_exit(f: impl FnMut(Exit) -> bool + 'static) { EXIT.with(|e| *e.borrow_mut() = Some(Box::new(f))); }

/// [`on_exit`] was called: the page has a lobby to go back to.
pub fn takes_back() -> bool { EXIT.with(|e| e.borrow().is_some()) }

fn exit(why: Exit) -> bool {
    EXIT.with(|e| match e.borrow_mut().as_mut() { Some(f) => f(why), None => false })
}

/// Call `f` with every connected device's input before each tick (the
/// lobby's and every game's), for the devices bound to each seat when the
/// running game binds none itself ([`Game::seats`]). A browser host joins a
/// pad that wakes up as a player there, and keeps each player on the same
/// pad from the room into the games.
pub fn on_devices(f: impl FnMut(&[(crate::Device, crate::Controls)]) -> Option<Vec<Vec<crate::Device>>> + 'static) {
    SEATS.with(|s| *s.borrow_mut() = Some(Box::new(f)));
}

/// Run `game` in the page's canvas `#warp` (or a new one) until the page
/// closes. Returns at once: the browser drives the frames.
pub fn run(cfg: crate::Config, game: Box<dyn Game>) {
    install_panic_hook();
    let mut devices = Devices::new(cfg.keyboard);
    devices.pointer_enabled = cfg.pointer;
    let shell = Shell { cfg, game, devices, window: None, live: Rc::new(RefCell::new(None)), acc: 0.0, last: Instant::now(), frames: 0, started: Instant::now(), failed: false, announced: false, hidden_editor:None, switched: None, exited: false, times: Vec::new(),
        render_height: query_param("render").and_then(|h| h.parse().ok()), quality: None };
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
        progress(Some(0.8), "Starting the graphics");
        wasm_bindgen_futures::spawn_local(async move {
            match setup(window.clone()).await {
                Ok(l) => {
                    log_pinned(&format!("GPU: {} ({:?}), {}x{}, {:?}", l.gpu.info.name, l.gpu.info.backend, l.config.width, l.config.height, l.config.format));
                    // Below the full renderer's limits it draws with its portable
                    // path (render::portable): terrain and sky only.
                    let limits = l.gpu.device.limits();
                    let (storage, textures) = (limits.max_storage_buffers_per_shader_stage, limits.max_sampled_textures_per_shader_stage);
                    let need = render::portable::full_sampled_textures();
                    let line = if render::portable::needed(&limits) {
                        let short = if textures < need { format!(" (the full renderer needs {need})") } else { String::new() };
                        progress(Some(0.85), &format!("This device's WebGPU offers {textures} textures per shader{short}, too few for the full renderer"));
                        format!("{storage} storage buffers, {textures} sampled textures per shader stage{short}: portable renderer (terrain and sky only)")
                    } else if storage < 16 { format!("{storage} storage buffers, {textures} sampled textures per shader stage: full renderer, no trees (they need 16 storage buffers)") }
                    else { format!("{storage} storage buffers, {textures} sampled textures per shader stage: full renderer") };
                    log_pinned(&line);
                    let glass = render::portable::glass_sampled_textures();
                    if !render::portable::needed(&limits) && textures < glass { log_pinned(&format!("no glass: transparent surfaces need {glass} sampled textures per stage")); }
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
        let ms = |t: Instant| t.elapsed().as_secs_f32() * 1000.0;
        let mut live = self.live.borrow_mut();
        let Some(live) = live.as_mut() else { return Ok(()) };
        if let Some((next, keep)) = NEXT.with(|n| n.borrow_mut().take()) {
            // The old game's scene goes with its renderer, kept or dropped;
            // a new game builds its own on its first draw, a resumed one
            // brings its renderer back.
            let old = Parked { cfg: std::mem::replace(&mut self.cfg, next.cfg), game: std::mem::replace(&mut self.game, next.game), renderer: std::mem::replace(&mut live.renderer, next.renderer) };
            if let Some(keep) = keep { keep(old); }
            self.devices.keyboard_enabled = self.cfg.keyboard;
            self.devices.pointer_enabled = self.cfg.pointer;
            self.exited = false;
            self.acc = 0.0;
            self.switched = Some(Instant::now());
            self.times.clear();
            // Each game starts at full resolution, chasing its own target.
            if let Some(q) = &mut self.quality { q.set_enabled(false); q.set_enabled(true); q.idle(); q.set_target_fps(self.game.target_fps(), None); }
            if let Some(w) = &self.window { w.set_title(&self.cfg.title); }
        }
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
        self.devices.poll();
        let t_tick = Instant::now();
        while self.acc >= step && steps < 6 {
            self.acc -= step;
            steps += 1;
            self.game.pointer(self.devices.pointer(), size);
            let mut devices = self.devices.device_controls(step);
            if !self.exited && devices.iter().any(|(_, c)| c.pressed.has(crate::Buttons::SELECT)) && exit(Exit::Back) {
                // Back is the host's: the game never sees it.
                self.exited = true;
                for (_, c) in &mut devices { c.pressed.0 &= !crate::Buttons::SELECT; c.held.0 &= !crate::Buttons::SELECT; }
            }
            self.game.devices(&devices);
            let page = SEATS.with(|s| s.borrow_mut().as_mut().and_then(|f| f(&devices)));
            let seats = self.game.seats().or(page);
            let controls = crate::input::route(&devices, seats.as_deref(), self.game.local_players());
            self.game.tick(&controls);
            if !self.exited && self.game.take_back() { self.exited = exit(Exit::Back); }
            if !self.exited && self.game.quit() { self.exited = exit(Exit::Quit); }
        }
        if steps == 6 { self.acc = 0.0; }
        let tick_ms = ms(t_tick);
        let alpha = (self.acc / step).clamp(0.0, 1.0);
        let split = self.game.split();
        // The first scene upload blocks for seconds: show it first.
        if live.renderer.is_none() && !self.announced {
            self.announced = true;
            progress(Some(0.9), "Getting the world ready");
            return Ok(());
        }
        let t_draw = Instant::now();
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha, split, scene_fps: if dt > 0.0 { 1.0 / dt } else { 0.0 }, scene_worst_ms: dt * 1000.0, focused: true, hdr: None, stream_generated_textures: false, renderer: &mut live.renderer };
        let mut draw: Draw = self.game.draw(&mut frame)?;
        // The touch stick, while a thumb is on it.
        if let Some((from, knob, r)) = self.devices.touch_stick(size) {
            let clip = [0.0, 0.0, size[0] as f32, size[1] as f32];
            draw.hud.circle(from, r, [1.0, 1.0, 1.0, 0.18], clip);
            draw.hud.circle(knob, r * 0.42, [1.0, 1.0, 1.0, 0.5], clip);
        }
        let Some(renderer) = live.renderer.as_ref() else { return Ok(()) };
        let draw_ms = ms(t_draw);
        let t_out = Instant::now();
        let texture = match live.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                live.surface.configure(&live.gpu.device, &live.config);
                return Err(format!("canvas texture: {other:?}"));
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(live.config.format.add_srgb_suffix()), ..Default::default() });
        let quality = self.quality.get_or_insert_with(|| crate::quality::AdaptiveQuality::new(self.game.target_fps(), None, Instant::now()).unwrap_or_else(|| unreachable!()));
        // As natively: a large canvas starts from 1440 lines, 0 is native,
        // and the controller scales down from there.
        let height = match self.render_height {
            Some(h) => Some(h),
            None => match self.cfg.max_render_height {
                Some(0) => Some(quality.render_height(size[1])).filter(|h| *h < size[1]),
                Some(h) => Some(quality.render_height(h)),
                None => Some(quality.render_height(size[1].min(ADAPTIVE_BASE_HEIGHT))).filter(|h| *h < size[1]),
            },
        };
        live.out.draw(&live.gpu, renderer, &draw, size, split, &view, height)?;
        live.gpu.queue.present(texture);
        self.frames += 1;
        if self.switched.is_none() && self.render_height.is_none() {
            if quality.frame(f64::from(dt) * 1000.0, Instant::now()) {
                let next = match self.cfg.max_render_height { Some(0) => quality.render_height(size[1]), Some(m) => quality.render_height(m), None => quality.render_height(size[1].min(ADAPTIVE_BASE_HEIGHT)) };
                log(&format!("{}: 3D view now {} lines high (chasing {:.0} fps)", self.cfg.title, next.min(size[1]), quality.target_fps()));
            }
        }
        if self.switched.is_none() { stats(&mut self.times, &self.cfg.title, [dt * 1000.0, tick_ms, draw_ms, ms(t_out)]); }
        if self.frames == 1 { progress(None, "Ready"); }
        if let Some(at) = self.switched.take() { progress(None, "Ready"); log(&format!("{}: first frame {:.0} ms after the switch", self.cfg.title, at.elapsed().as_secs_f32() * 1000.0)); }
        if self.frames <= 4 || self.frames.is_power_of_two() && self.frames >= 64 {
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

/// Every 300 frames, log the frame times: the interval between frames
/// (what players see), and the page thread's ticks, the game's draw and
/// the engine's encode and submit. An interval well above their sum is
/// the GPU (or vsync); `?render=720` shows which.
fn stats(times: &mut Vec<[f32; 4]>, title: &str, sample: [f32; 4]) {
    times.push(sample);
    if times.len() < 300 { return; }
    let pick = |k: usize, q: f32| {
        let mut v: Vec<f32> = times.iter().map(|t| t[k]).collect();
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    };
    log(&format!("{}: frame every {:.1} ms (p95 {:.1}); page thread ticks {:.1}, draw {:.1}, encode {:.1} ms (medians)",
        title, pick(0, 0.5), pick(0, 0.95), pick(1, 0.5), pick(2, 0.5), pick(3, 0.5)));
    times.clear();
}


/// A canvas taller than this starts its 3D view here (as natively).
const ADAPTIVE_BASE_HEIGHT: u32 = 1440;

async fn setup(window: Arc<Window>) -> Result<Live, String> {
    let instance = render::gpu::instance();
    let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
    let size = window.inner_size();
    let (gpu, config, _adapter) = Gpu::for_surface_async(&instance, &surface, size.width.max(1), size.height.max(1)).await?;
    surface.configure(&gpu.device, &config);
    // After a device loss every call fails with "is lost", and after one
    // failed creation every use of the object fails "due to a previous
    // error": the first error is pinned in the log, the follow-ups only
    // counted, so the cause stays readable.
    let first = std::sync::atomic::AtomicBool::new(false);
    let lost_said = std::sync::atomic::AtomicBool::new(false);
    let follow_ups = std::sync::atomic::AtomicU32::new(0);
    gpu.device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        use std::sync::atomic::Ordering::Relaxed;
        let text = error.to_string();
        if text.contains("] is lost") && lost_said.swap(true, Relaxed) { return; }
        if text.contains("previous error") || (text.contains(" is invalid") && first.load(Relaxed)) {
            let n = follow_ups.fetch_add(1, Relaxed) + 1;
            web_sys::console::log_1(&format!("GPU error: {text}").into());
            if n == 1 || n.is_power_of_two() { log(&format!("{n} GPU errors follow from the first")); }
            return;
        }
        if first.swap(true, Relaxed) { log(&format!("GPU error: {text}")) } else { log_pinned(&format!("First GPU error: {text}")) }
    }));
    gpu.device.set_device_lost_callback(|reason, message| log_pinned(&format!("GPU device lost ({reason:?}): {message}")));
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
            WindowEvent::Touch(t) => {
                let size = self.live.borrow().as_ref().map_or([1, 1], |l| [l.config.width, l.config.height]);
                self.devices.touch(t.id, t.phase, [t.location.x as f32, t.location.y as f32], size);
            }
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
                if code == F1 {
                    if event.state == ElementState::Pressed && !event.repeat {
                        let open = self.hidden_editor.take().unwrap_or_else(|| { self.hidden_editor=Some(self.game.editor_windows_open()); false });
                        self.game.set_editor_windows_open(open);
                        if open { self.capture_mouse(false); }
                    }
                    return;
                }
                if let Some(n) = [F5,F6,F7,F8,F9,F10,F11,F12].iter().position(|&k| k == code) {
                    if event.state == ElementState::Pressed && !event.repeat { self.hidden_editor=None; self.game.function_key(n as u8 + 5); }
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
