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
    /// Players who joined since the last tick ([`player_joins`]).
    static JOINS: RefCell<Vec<(String, Option<[f32; 3]>)>> = const { RefCell::new(Vec::new()) };
    /// The running game's [`crate::Game::takes_joins`], each frame.
    static TAKES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The page's seats for games without their own ([`on_devices`]).
    static SEATS: RefCell<Option<Box<dyn FnMut(&[(crate::Device, crate::Controls)]) -> Option<Vec<Vec<crate::Device>>>>>> = const { RefCell::new(None) };
    /// Lines the log box always shows above the rolling tail: the start-up
    /// lines (GPU, limits) and the first GPU error, which later errors would
    /// otherwise push out of the box.
    static PINNED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TAIL: RefCell<std::collections::VecDeque<String>> = const { RefCell::new(std::collections::VecDeque::new()) };
    /// Whether the page shows later loading itself ([`quiet_progress`]),
    /// and whether that is in effect (from the first frame on).
    static QUIET: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
    /// Changes as soon as a switch is requested. Only that scene's first GPU
    /// completion may dismiss the loading card.
    static PRESENTATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The screen's refresh rate as the page's animation frames come
    /// ([`probe_refresh`]), and whether the shell has used it yet.
    static REFRESH: std::cell::Cell<(Option<f32>, bool)> = const { std::cell::Cell::new((None, false)) };
}

/// Time the page's animation frames for about two seconds: their cadence
/// is the screen's refresh rate (60 Hz, 120 Hz on a ProMotion Mac). A long
/// frame now and then (building the room) only lengthens some, so the
/// fastest quarter decides.
fn probe_refresh() {
    use wasm_bindgen::closure::Closure;
    let times: Rc<RefCell<Vec<f64>>> = Rc::default();
    let tick: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::default();
    let again = tick.clone();
    *tick.borrow_mut() = Some(Closure::new(move |at: f64| {
        let mut t = times.borrow_mut();
        t.push(at);
        if t.len() > 120 {
            let mut gaps: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 1.0).collect();
            gaps.sort_by(f64::total_cmp);
            if let Some(&gap) = gaps.get(gaps.len() / 4) {
                let hz = (1000.0 / gap) as f32;
                REFRESH.with(|r| r.set((Some(hz), false)));
                log(&format!("screen refresh about {hz:.0} Hz"));
            }
            again.borrow_mut().take();
            return;
        }
        if let (Some(w), Some(f)) = (web_sys::window(), again.borrow().as_ref()) { let _ = w.request_animation_frame(f.as_ref().unchecked_ref()); }
    }));
    if let (Some(w), Some(f)) = (web_sys::window(), tick.borrow().as_ref()) { let _ = w.request_animation_frame(f.as_ref().unchecked_ref()); }
}

/// The frame rate adaptive quality chases for a game: its own target, but
/// never above the screen's refresh (a game asking 120 on a 60 Hz screen
/// would lower its resolution for nothing). None stays None (60).
fn chase(target: Option<f32>) -> Option<f32> {
    let hz = REFRESH.with(|r| r.get().0);
    target.map(|t| hz.map_or(t, |hz| t.min(hz)))
}

/// Once the first frame is drawn, [`progress`] only logs: the page shows
/// later loading its own way (the Game Room on its TV). A stop still shows.
pub fn quiet_progress() { QUIET.with(|q| q.set((true, q.get().1))); }

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
/// Whether the page asks to hold blocking work (`window.warpHold`), as
/// while it goes fullscreen.
fn page_holds() -> bool {
    web_sys::window().and_then(|w| js_sys::Reflect::get(&w, &"warpHold".into()).ok()).is_some_and(|v| v.is_truthy())
}

pub fn progress(fraction: Option<f32>, step: &str) {
    web_sys::console::log_1(&format!("loading: {step}").into());
    LAST_PROGRESS.with(|l| *l.borrow_mut() = (fraction, step.to_string()));
    if fraction.is_some() && QUIET.with(|q| q.get().1) { return; }
    show_progress(fraction, step);
}

fn next_presentation() -> u64 {
    PRESENTATION.with(|current| {
        let next = current.get().wrapping_add(1);
        current.set(next);
        next
    })
}

/// The game stopped with an error: shown in the box even when quiet.
fn stopped(step: &str) {
    next_presentation();
    web_sys::console::log_1(&step.into());
    show_progress(Some(1.0), step);
}

thread_local! {
    static LAST_PROGRESS: RefCell<(Option<f32>, String)> = const { RefCell::new((None, String::new())) };
}

/// The last [`progress`] reported (also after the first frame, when the
/// page may show it its own way, as the Game Room's loading card).
pub fn loading() -> (Option<f32>, String) { LAST_PROGRESS.with(|l| l.borrow().clone()) }

fn show_progress(fraction: Option<f32>, step: &str) {
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
    // A worker paints nothing: one turn of its event loop is enough.
    if web_sys::window().is_none() { return sleep(0).await; }
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

/// Run `task` on the page's event loop (natively on Android: between the
/// render thread's frames, app::android::spawn).
pub fn spawn(task: impl std::future::Future<Output = ()> + 'static) { wasm_bindgen_futures::spawn_local(task) }

/// A future that is ready `ms` milliseconds from now.
pub async fn sleep(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        match web_sys::window() {
            Some(w) => { let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32); }
            None => { let _ = js_sys::global().unchecked_into::<web_sys::WorkerGlobalScope>().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32); }
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Fetch `url` (relative to the page) as bytes.
thread_local! {
    /// Files [`prefetch`] fetched ahead, until [`fetch`] asks for them.
    static PREFETCHED: RefCell<std::collections::HashMap<String, Vec<u8>>> = RefCell::new(Default::default());
}

/// Fetch `url` now and keep it in memory: the next [`fetch`] of the same
/// URL takes it from there (once). Returns its size.
pub async fn prefetch(url: &str) -> Result<usize, String> {
    if let Some(n) = PREFETCHED.with(|p| p.borrow().get(url).map(Vec::len)) { return Ok(n); }
    let data = fetch(url).await?;
    let n = data.len();
    PREFETCHED.with(|p| p.borrow_mut().insert(url.to_string(), data));
    Ok(n)
}

thread_local! {
    /// The page's build (`data-version`), for a worker that has no page.
    static VERSION: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// In a worker: the page's build stamp, so [`fetch`] asks for this build's files.
pub fn set_version(v: &str) { VERSION.with(|s| *s.borrow_mut() = Some(v.to_string()).filter(|v| !v.is_empty())); }

/// Forget what [`prefetch`] kept that nobody fetched.
pub fn clear_prefetched() { PREFETCHED.with(|p| p.borrow_mut().clear()); }

pub async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen_futures::JsFuture;
    if let Some(data) = PREFETCHED.with(|p| p.borrow_mut().remove(url)) { return Ok(data); }
    // A file next to the page is this build's (`<html data-version>`, set
    // by build-web.py; a worker is told it, [`set_version`]): never an
    // older one from the browser's cache.
    let version = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()).and_then(|e| e.get_attribute("data-version"))
        .or_else(|| VERSION.with(|v| v.borrow().clone()));
    let versioned = match version { Some(v) if !url.contains("://") && !url.contains('?') => format!("{url}?v={v}"), _ => url.to_string() };
    let promise = match web_sys::window() {
        Some(w) => w.fetch_with_str(&versioned),
        None => js_sys::global().unchecked_into::<web_sys::WorkerGlobalScope>().fetch_with_str(&versioned),
    };
    let response: web_sys::Response = JsFuture::from(promise).await.map_err(|e| format!("{url}: {e:?}"))?
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

/// Hand an error to the page's error reporting (`window.splitscreenReport`,
/// the browser host's Sentry reporter), when the page has one.
pub fn report(kind: &str, text: &str) {
    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"splitscreenReport".into())
        && let Some(f) = f.dyn_ref::<js_sys::Function>() {
        let _ = f.call2(&wasm_bindgen::JsValue::NULL, &kind.into(), &text.into());
    }
}

/// Panics go to the console and the page, not into the void.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let line = format!("panic: {info}");
        web_sys::console::error_1(&line.clone().into());
        log(&line);
        // The page shows it (and can report it): `window.warpCrash(message)`.
        if let Some(window) = web_sys::window()
            && let Ok(f) = js_sys::Reflect::get(&window, &"warpCrash".into())
            && let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&wasm_bindgen::JsValue::NULL, &line.into());
        }
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
    /// Presentation and start time until its first visible frame has been submitted.
    switched: Option<(u64, Instant)>,
    /// The running game has left once ([`on_exit`]): wait for the switch.
    exited: bool,
    /// Back and its chords in a game: the room comes on Back's release.
    back: crate::back_chord::BackChords,
    /// Per frame: time since the last frame, ticks, the game's draw, and the
    /// engine's encode and submit, in ms ([`Shell::stats`]).
    times: Vec<[f32; 4]>,
    /// `?render=H`: draw the 3D view at most H pixels high (to tell GPU-bound
    /// from CPU-bound), whatever the game asks.
    render_height: Option<u32>,
    /// `bench=1`: fixed heights and one `BENCH {json}` line per game ([`crate::bench`]).
    bench: Option<crate::bench::Bench>,
    /// Lowers the 3D view's resolution while frames miss the game's target
    /// (60 fps unless it names one), as the native shell does.
    quality: Option<crate::quality::AdaptiveQuality>,
    /// The phone preset ([`mobile`]): a lower resolution and no bloom,
    /// contact occlusion or traced reflections.
    mobile: bool,
    /// The page held the pointer lock last frame.
    lock_seen: bool,
}

/// The window's short side in CSS pixels (a phone in landscape: about
/// 400), for sizing UI to the hand rather than the pixels.
pub fn short_side_dp() -> Option<f32> {
    let w = web_sys::window()?;
    let (iw, ih) = (w.inner_width().ok()?.as_f64()?, w.inner_height().ok()?.as_f64()?);
    Some(iw.min(ih) as f32)
}

/// Phones get a cheaper preset: an Android, iPhone or iPad browser, or
/// `?quality=mobile` anywhere (`?quality=full` turns it off). Adaptive
/// quality then starts from [`MOBILE_BASE_HEIGHT`] lines (or `?render=H`)
/// instead of 1440 and lowers from there, and the frame drops the passes
/// that cost a phone GPU most.
pub fn mobile() -> bool {
    match query_param("quality").as_deref() {
        Some("mobile" | "low") => true,
        Some("full" | "high") => false,
        _ => web_sys::window().and_then(|w| w.navigator().user_agent().ok())
            .is_some_and(|ua| ["Android", "iPhone", "iPad", "Mobile"].iter().any(|k| ua.contains(k))),
    }
}

/// Safari, or any browser on iOS: WebKit's WebGPU, not Chrome's or Firefox's.
/// Firefox and its forks (Zen, LibreWolf): Gecko's WebGPU (wgpu, naga).
fn gecko() -> bool {
    web_sys::window().and_then(|w| w.navigator().user_agent().ok()).is_some_and(|ua| ua.contains("Gecko/") && ua.contains("Firefox/") && !ua.contains("Seamonkey"))
}

fn webkit() -> bool {
    web_sys::window().and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("AppleWebKit") && !["Chrome", "Chromium", "Edg", "Android"].iter().any(|k| ua.contains(k)) || ua.contains("CriOS") || ua.contains("FxiOS") || ua.contains("EdgiOS"))
}

/// Where adaptive quality starts on a phone: about half of a 1080p panel.
const MOBILE_BASE_HEIGHT: u32 = 540;

/// The phone preset's lighting: no contact and reflection occlusion, traced
/// reflections, bloom or lens flare. GI stays: its volume is the only light
/// a character gets away from the lamps, which turned it pitch black in the
/// Game Room's dark corners (tablet playtest).
fn mobile_light(mut light: render::Lighting) -> render::Lighting {
    light.contact_occlusion = 0.0;
    light.reflection_occlusion = 0.0;
    light.traced_reflections = 0.0;
    light.bloom_boost = 0.0;
    light.lens_flare = 0.0;
    light
}

thread_local! {
    static CAPTURE: RefCell<Option<Box<dyn FnOnce([u32; 2], Vec<u8>)>>> = const { RefCell::new(None) };
}

/// Hand the frame this tick draws to `done` (size, RGBA8, top row first),
/// a little later. Asked from a tick (a game leaving for the room), it is
/// that game's last frame: the switch only happens on the next one.
pub fn capture_frame(done: Box<dyn FnOnce([u32; 2], Vec<u8>)>) {
    CAPTURE.with(|c| *c.borrow_mut() = Some(done));
}

/// Copy `texture` (the canvas) to a buffer and read it when the GPU is done.
fn capture(gpu: &Gpu, texture: &wgpu::Texture, done: Box<dyn FnOnce([u32; 2], Vec<u8>)>) {
    if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) { return; }
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("canvas capture"), size: u64::from(row) * u64::from(h), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let bgra = matches!(texture.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    let mapped = buffer.clone();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        if result.is_err() { return; }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        {
            let Ok(data) = mapped.slice(..).get_mapped_range() else { return };
            for y in 0..h as usize { rgba.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize * 4]); }
        }
        mapped.unmap();
        if bgra { for p in rgba.chunks_exact_mut(4) { p.swap(0, 2); } }
        for p in rgba.chunks_exact_mut(4) { p[3] = 255; }
        done([w, h], rgba);
    });
}

/// Replace the running game with `game` at the next frame, keeping the
/// canvas, the GPU device, the input devices and everything the engine has
/// compiled: a switch costs only the new game's scene upload.
pub fn switch(cfg: crate::Config, game: Box<dyn Game>) {
    resume(Parked::new(cfg, game), None);
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
    resume(Parked::new(cfg, game), Some(Box::new(keep)));
}

/// Bring a [`Parked`] game back on screen with its scene as it was, handing
/// the one it replaces to `keep` (when given) the same way.
pub fn resume(parked: Parked, keep: Option<Box<dyn FnOnce(Parked)>>) {
    next_presentation();
    NEXT.with(|n| *n.borrow_mut() = Some((parked, keep)));
}

/// Why the running game wants to leave ([`on_exit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Its own menu's Quit ([`Game::quit`]).
    Quit,
    /// Someone pressed and let go of Back (Select, Tab on the keyboard)
    /// without a chord: the game stays paused, as the host's Back does.
    Back,
    /// Back + RB: straight on to the next game.
    Next,
    /// Back + LB: the game played before this one.
    Previous,
    /// Back + Y, or F4: report a bug. The game waits, as for Back; the host
    /// screenshots it ([`capture_frame`]) for the report.
    ReportBug,
}

/// Call `f` when the running game quits or someone presses Back; a browser
/// host goes back to its lobby there (a [`switch`]) and returns true. When
/// it returns false (or without it) Back is the game's like any button and
/// a quit game keeps running.
thread_local! {
    static IN_GAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// F4 went down: a game leaves to report a bug on its next tick, the room reads it with [`take_report_key`].
    static REPORT_KEY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// F4 was pressed in the room since the last call: report a bug there. In
/// a game F4 leaves it with [`Exit::ReportBug`] instead.
pub fn take_report_key() -> bool { REPORT_KEY.with(|r| r.replace(false)) }

/// The host tells the shell whether a game (not its room) has the screen:
/// only then is Back the host's, read on release with its RB/LB chords
/// ([`crate::back_chord`]). In the room Back stays the room's.
pub fn set_in_game(on: bool) { IN_GAME.with(|g| g.set(on)); }

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
/// Someone joined the party while a game runs: the game gets
/// [`crate::Game::player_joins`] before its next tick, and their device
/// must then drive the game's next player in controls order.
pub fn player_joins(name: &str, color: Option<[f32; 3]>) { JOINS.with(|j| j.borrow_mut().push((name.to_string(), color))); }

/// Whether the running game takes a player who joins now.
pub fn game_takes_joins() -> bool { TAKES.with(|t| t.get()) }

pub fn on_devices(f: impl FnMut(&[(crate::Device, crate::Controls)]) -> Option<Vec<Vec<crate::Device>>> + 'static) {
    SEATS.with(|s| *s.borrow_mut() = Some(Box::new(f)));
}

/// Run `game` in the page's canvas `#warp` (or a new one) until the page
/// closes. Returns at once: the browser drives the frames.
pub fn run(cfg: crate::Config, game: Box<dyn Game>) {
    install_panic_hook();
    probe_refresh();
    let mut devices = Devices::new(cfg.keyboard);
    devices.pointer_enabled = cfg.pointer;
    let shell = Shell { cfg, game, devices, window: None, live: Rc::new(RefCell::new(None)), acc: 0.0, last: Instant::now(), frames: 0, started: Instant::now(), failed: false, announced: false, hidden_editor:None, switched: Some((next_presentation(), Instant::now())), exited: false, back: Default::default(), times: Vec::new(),
        render_height: query_param("render").and_then(|h| h.parse().ok()), quality: None, mobile: mobile(), lock_seen: false,
        bench: crate::bench::Bench::from_query(query_param("bench").as_deref()) };
    if shell.mobile { log("phone preset: lower resolution, no bloom or contact occlusion (?quality=full turns it off)"); }
    // Safari (and every iOS browser: WebKit) cannot compile the trees'
    // compare-exchange shaders, and Firefox's (Zen on Windows: DXC) fails
    // on the shade pass with them: the trees stay out there (render::gpu).
    // ?trees=0 leaves them out anywhere, ?trees=1 tries them anyway.
    let trees = query_param("trees");
    let why = if trees.as_deref() == Some("0") { Some("?trees=0") }
        else if trees.as_deref() == Some("1") { None }
        else if webkit() { Some("WebKit cannot compile their shaders") }
        else if gecko() { Some("Firefox cannot compile the shade pass with them") }
        else { None };
    if let Some(why) = why {
        render::gpu::cap_storage_buffers(10);
        log(&format!("no trees ({why}; ?trees=1 tries anyway)"));
    }
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
            let generation = PRESENTATION.with(|p| p.get());
            // The old game's scene goes with its renderer, kept or dropped;
            // a new game builds its own on its first draw, a resumed one
            // brings its renderer back.
            if next.renderer.is_none() { show_progress(Some(0.95), "Preparing graphics"); }
            let old = Parked { cfg: std::mem::replace(&mut self.cfg, next.cfg), game: std::mem::replace(&mut self.game, next.game), renderer: std::mem::replace(&mut live.renderer, next.renderer) };
            if let Some(keep) = keep { keep(old); }
            self.devices.keyboard_enabled = self.cfg.keyboard;
            self.devices.pointer_enabled = self.cfg.pointer;
            self.exited = false;
            self.back = Default::default();
            self.acc = 0.0;
            self.switched = Some((generation, Instant::now()));
            self.times.clear();
            if let Some(b) = &mut self.bench { b.restart(); }
            // Each game starts at full resolution, chasing its own target.
            if let Some(q) = &mut self.quality { q.set_enabled(false); q.set_enabled(true); q.idle(); q.set_target_fps(chase(self.game.target_fps()), None); }
            if let Some(w) = &self.window {
                w.set_title(&self.cfg.title);
                // The next game (or the room) starts with a free, visible
                // mouse: a pointer the old game locked stays hidden otherwise.
                let _ = w.set_cursor_grab(CursorGrabMode::None);
                w.set_cursor_visible(true);
            }
            self.devices.mouse_captured = false;
            self.devices.reset_mouse();
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
        // The browser let go of the pointer lock itself (Escape, a switch to
        // the room, another tab): show the cursor again.
        // (The lock arrives a little after the request: only once seen.)
        let locked = self.devices.mouse_captured && web_sys::window().and_then(|w| w.document()).is_some_and(|d| d.pointer_lock_element().is_some());
        if self.devices.mouse_captured && self.lock_seen && !locked {
            if let Some(w) = &self.window { let _ = w.set_cursor_grab(CursorGrabMode::None); w.set_cursor_visible(true); }
            self.devices.mouse_captured = false;
            self.devices.reset_mouse();
        }
        self.lock_seen = locked;
        self.devices.poll();
        let t_tick = Instant::now();
        while self.acc >= step && steps < 6 {
            self.acc -= step;
            steps += 1;
            self.game.pointer(self.devices.pointer(), size);
            let mut devices = self.devices.device_controls(step);
            // Back is the host's: the game never sees it, nor RB/LB under it.
            // The room comes on Back's release, so Back + RB/LB go straight
            // on to another game ([`crate::back_chord`]).
            if !self.exited && takes_back() && IN_GAME.with(|g| g.get()) {
                let why = match self.back.step(&mut devices) {
                    Some(crate::back_chord::BackAction::Room) => Some(Exit::Back),
                    Some(crate::back_chord::BackAction::Next) => Some(Exit::Next),
                    Some(crate::back_chord::BackAction::Previous) => Some(Exit::Previous),
                    Some(crate::back_chord::BackAction::ReportBug) => Some(Exit::ReportBug),
                    None => None,
                };
                let why = why.or_else(|| take_report_key().then_some(Exit::ReportBug));
                if let Some(why) = why && exit(why) { self.exited = true; }
            }
            self.game.devices(&devices);
            let page = SEATS.with(|s| s.borrow_mut().as_mut().and_then(|f| f(&devices)));
            for (name, color) in JOINS.with(|j| std::mem::take(&mut *j.borrow_mut())) {
                let now = self.game.player_joins(Some(&name), color);
                log(&format!("{}: {name} {}", self.cfg.title, if now { "joined the game" } else { "joins from the next game" }));
            }
            let seats = self.game.seats().or(page);
            let controls = crate::input::route(&devices, seats.as_deref(), self.game.local_players());
            self.game.tick(&controls);
            if !self.exited && self.game.take_back() { self.exited = exit(Exit::Back); }
            if !self.exited && self.game.quit() { self.exited = exit(Exit::Quit); }
        }
        if steps == 6 { self.acc = 0.0; }
        TAKES.with(|t| t.set(self.game.takes_joins()));
        let tick_ms = ms(t_tick);
        let alpha = (self.acc / step).clamp(0.0, 1.0);
        let split = self.game.split();
        // The first scene upload blocks for seconds: show it first.
        if live.renderer.is_none() && !self.announced {
            self.announced = true;
            progress(Some(0.9), "Preparing graphics");
            return Ok(());
        }
        // The page is going fullscreen: a scene upload now would block the
        // page while the window grows, leaving its new edges unpainted
        // (white) for seconds. Wait until the page says it has repainted.
        if live.renderer.is_none() && page_holds() { return Ok(()); }
        let t_draw = Instant::now();
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha, split, scene_fps: if dt > 0.0 { 1.0 / dt } else { 0.0 }, scene_worst_ms: dt * 1000.0, focused: true, hdr: None, stream_generated_textures: true, renderer: &mut live.renderer };
        let mut draw: Draw = self.game.draw(&mut frame)?;
        // As natively, the adaptive rung also drops contact occlusion and
        // bloom; a phone drops them (and GI) from the start.
        if let Some(q) = &self.quality {
            draw.light = q.apply_light(draw.light);
            if !q.rung().bloom { draw.light.bloom_boost = 0.0; }
        }
        if self.mobile { draw.light = mobile_light(draw.light); }
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
        // Waiting for a free canvas image is the GPU's time, not the CPU's.
        let wait_ms = ms(t_out);
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(live.config.format.add_srgb_suffix()), ..Default::default() });
        let quality = self.quality.get_or_insert_with(|| crate::quality::AdaptiveQuality::new(chase(self.game.target_fps()), None, Instant::now()).unwrap_or_else(|| unreachable!()));
        // The refresh rate just became known: chase what the screen can show.
        if REFRESH.with(|r| { let (hz, used) = r.get(); r.set((hz, true)); hz.is_some() && !used }) { quality.set_target_fps(chase(self.game.target_fps()), None); }
        // As natively: a large canvas starts from 1440 lines, 0 is native,
        // and the controller scales down from there.
        // A phone starts lower (`?render=H` is its start, not a fixed cap).
        let base = |game: Option<u32>| match (self.mobile, game) {
            (true, _) => self.render_height.unwrap_or(MOBILE_BASE_HEIGHT).min(game.filter(|h| *h > 0).unwrap_or(u32::MAX)).min(size[1]),
            (false, Some(0)) => size[1],
            (false, Some(h)) => h,
            (false, None) => size[1].min(ADAPTIVE_BASE_HEIGHT),
        };
        let height = match self.render_height.filter(|_| !self.mobile) {
            Some(h) => Some(h),
            None => Some(quality.render_height(base(self.cfg.max_render_height))).filter(|h| *h < size[1]),
        };
        let benching = self.bench.as_mut().and_then(|b| b.height(size));
        let height = match benching { Some(h) => Some(h).filter(|h| *h < size[1]), None => height };
        live.out.draw(&live.gpu, renderer, &draw, size, split, &view, height)?;
        if let Some(done) = CAPTURE.with(|c| c.borrow_mut().take()) { capture(&live.gpu, &texture.texture, done); }
        live.gpu.queue.present(texture);
        self.frames += 1;
        if self.switched.is_none() && benching.is_none() && (self.render_height.is_none() || self.mobile) {
            if quality.frame(f64::from(dt) * 1000.0, Instant::now()) {
                let next = quality.render_height(base(self.cfg.max_render_height));
                log(&format!("{}: 3D view now {} lines high (chasing {:.0} fps)", self.cfg.title, next.min(size[1]), quality.target_fps()));
            }
        }
        if self.switched.is_none() && let Some(b) = &mut self.bench {
            // No GPU timestamps on wasm32: the fit uses frame times.
            let sample = crate::bench::Sample { frame_ms: dt * 1000.0, cpu_ms: tick_ms + draw_ms + (ms(t_out) - wait_ms).max(0.0), wait_ms, gpu_ms: None };
            let about = crate::bench::About { game: self.cfg.title.clone(), gpu: live.gpu.info.name.clone(), platform: "browser", tier: query_param("tier").unwrap_or_else(|| if self.mobile { "medium" } else { "high" }.into()), players: self.game.local_players(), memory_mb: memory_mb() };
            if let Some(line) = b.frame(sample, size, &about) { log(&line); }
        }
        if self.switched.is_none() { stats(&mut self.times, &self.cfg.title, [dt * 1000.0, tick_ms, draw_ms, ms(t_out)]); }
        // A 1x1 canvas can precede the first layout. Ready means the actual
        // scene reached the GPU, including cold driver compilation and the
        // final color submission. Later frames never wait for this callback.
        if size[0] > 1 && size[1] > 1 && let Some((generation, at)) = self.switched.take() {
            let title = self.cfg.title.clone();
            live.gpu.queue.on_submitted_work_done(move || {
                if PRESENTATION.with(|p| p.get()) == generation {
                    progress(None, "Ready");
                    QUIET.with(|q| q.set((q.get().0, q.get().0)));
                    log(&format!("{title}: first frame completed in {:.0} ms", at.elapsed().as_secs_f32() * 1000.0));
                }
            });
        }
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
    let (gpu, mut config, _adapter) = Gpu::for_surface_async(&instance, &surface, size.width.max(1), size.height.max(1)).await?;
    // The canvas can be copied from ([`capture_frame`]).
    config.usage |= wgpu::TextureUsages::COPY_SRC;
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
        let internal = matches!(error, wgpu::Error::Internal { .. });
        let text = if internal { format!("internal: {error}") } else { error.to_string() };
        // The browser could not build something (a shader, a pipeline):
        // the page says so (`window.warpGpuProblem`), the room may stay dark.
        if internal && let Some(window) = web_sys::window()
            && let Ok(f) = js_sys::Reflect::get(&window, &"warpGpuProblem".into())
            && let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&wasm_bindgen::JsValue::NULL, &text.clone().into());
        }
        if text.contains("] is lost") && lost_said.swap(true, Relaxed) { return; }
        if text.contains("previous error") || (text.contains(" is invalid") && first.load(Relaxed)) {
            let n = follow_ups.fetch_add(1, Relaxed) + 1;
            web_sys::console::log_1(&format!("GPU error: {text}").into());
            if n == 1 || n.is_power_of_two() { log(&format!("{n} GPU errors follow from the first")); }
            return;
        }
        if first.swap(true, Relaxed) { log(&format!("GPU error: {text}")) } else {
            log_pinned(&format!("First GPU error: {text}"));
            // Internal errors reach the reporter through warpGpuProblem.
            if !internal { report("gpu-error", &text); }
        }
    }));
    gpu.device.set_device_lost_callback(|reason, message| {
        let error = format!("GPU device lost ({reason:?}): {message}");
        log_pinned(&error);
        if !matches!(reason, wgpu::DeviceLostReason::Destroyed) { report("device-lost", &error); }
        stopped(&error);
    });
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
                // F4 reports a bug, from a game or the room (as Back + Y does).
                // A game leaves on its next tick, so its screenshot is its own frame.
                if code == F4 && takes_back() {
                    if event.state == ElementState::Pressed && !event.repeat { REPORT_KEY.with(|r| r.set(true)); }
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
                    stopped(&format!("Stopped: {e}"));
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
