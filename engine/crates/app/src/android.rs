//! The shell on an Android phone: a NativeActivity drawing with Vulkan,
//! with the browser shell's API ([`crate::web`] is this module on
//! Android), so a host written for the browser (the Game Room's
//! `examples/browser`) runs natively: switching games without reloading,
//! keeping the lobby and a paused game on the GPU, Back to the lobby,
//! players joining with a pad. See engine/docs/android-host.md.
//!
//! Two threads. The activity's own thread ([`main`]) only pumps Android's
//! events: window changes and input, which it hands on at once, so Android
//! never waits on a slow frame and never says the app isn't responding.
//! The render thread owns the GPU, the running game, the host's state and
//! its tasks ([`spawn`]): building the room or loading a game may take
//! seconds there without blocking any input.
//!
//! Files the browser fetches next to the page come from the APK's
//! `assets/web/`, or, in the Splitscreen Club app, which ships only the
//! library, from splitscreen.club/play/ into a copy on the phone ([`fetch`]); web APIs go over HTTPS on a helper thread
//! ([`request`]); `localStorage` is a folder in the app's data
//! ([`stored`]); the page's query string is the intent's `query` extra
//! ([`query_param`]).

use crate::{Draw, Frame, Game, Output, Screen, input::Devices};
use android_activity::input::{Axis, InputEvent, KeyAction, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use ndk::native_window::NativeWindow;
use render::hud::{FontFace, HudDrawList, HudRenderer, TextAlign, TextRun};
use render::{Gpu, Renderer};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::task::{Context, Poll};
use std::time::Duration;
use web_time::Instant;

static APP: OnceLock<AndroidApp> = OnceLock::new();
/// The intent's `query` extra, parsed once ([`query_param`]).
static QUERY: OnceLock<Vec<(String, String)>> = OnceLock::new();

thread_local! {
    static NEXT: RefCell<Option<(Parked, Option<Keep>)>> = const { RefCell::new(None) };
    static EXIT: RefCell<Option<Box<dyn FnMut(Exit) -> bool>>> = const { RefCell::new(None) };
    static JOINS: RefCell<Vec<(String, Option<[f32; 3]>)>> = const { RefCell::new(Vec::new()) };
    static TAKES: Cell<bool> = const { Cell::new(false) };
    static SEATS: RefCell<Option<Box<dyn FnMut(&[(crate::Device, crate::Controls)]) -> Option<Vec<Vec<crate::Device>>>>>> = const { RefCell::new(None) };
    static QUIET: Cell<(bool, bool)> = const { Cell::new((false, false)) };
    /// The game on screen and its input, once [`run`] was called.
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    /// The GPU, the window's surface and the current game's renderer.
    static GFX: RefCell<Option<Gfx>> = const { RefCell::new(None) };
    /// Tasks [`spawn`]ed on the render thread, and the ones spawned while
    /// those were being polled.
    static TASKS: RefCell<Vec<Pin<Box<dyn Future<Output = ()>>>>> = const { RefCell::new(Vec::new()) };
    static SPAWNED: RefCell<Vec<Pin<Box<dyn Future<Output = ()>>>>> = const { RefCell::new(Vec::new()) };
    /// The loading screen's line and bar, until the first frame.
    static LOADING: RefCell<(Option<f32>, String)> = RefCell::new((Some(0.1), "Starting".into()));
    /// A line over the picture ([`banner`]) and until when.
    static BANNER: RefCell<Option<(String, Option<Instant>)>> = const { RefCell::new(None) };
    /// The banner that stays ([`banner`] without seconds), under the passing ones.
    static NOTICE: RefCell<Option<String>> = const { RefCell::new(None) };
}

// --- The browser shell's API -------------------------------------------

/// Write a line to logcat (tag SplitscreenHost).
pub fn log(line: &str) {
    use std::ffi::CString;
    let tag = CString::new("SplitscreenHost").unwrap_or_default();
    for part in line.lines() {
        let text = CString::new(part.replace('\0', " ")).unwrap_or_default();
        // SAFETY: two valid C strings for the duration of the call.
        unsafe { ndk_sys::__android_log_write(ndk_sys::android_LogPriority::ANDROID_LOG_INFO.0 as i32, tag.as_ptr(), text.as_ptr()); }
    }
}

pub fn log_pinned(line: &str) { log(line); }

/// After the first frame, [`progress`] only logs (the host shows loading its own way).
pub fn quiet_progress() { QUIET.with(|q| q.set((true, q.get().1))); }

/// The loading screen's progress (`None`: done), drawn at once.
pub fn progress(fraction: Option<f32>, step: &str) {
    log(&format!("loading: {step}"));
    if fraction.is_some() && QUIET.with(|q| q.get().1) { return; }
    LOADING.with(|l| *l.borrow_mut() = (fraction, step.to_string()));
    let shown = SHELL.try_with(|s| s.try_borrow().map(|s| s.as_ref().is_some_and(|s| s.frames > 0)).unwrap_or(true)).unwrap_or(true);
    if !shown && fraction.is_some() { draw_loading(); }
}

/// A short line over the picture, at the top: for `seconds`, or, without,
/// until [`clear_banner`] (a notice that stays; passing lines show over it
/// and it comes back after). "Title\nsmaller line" shows two lines with a
/// controller icon, as the browser page's notice. The page shows these as HTML.
pub fn banner(text: &str, seconds: Option<f32>) {
    match seconds {
        Some(s) => BANNER.with(|b| *b.borrow_mut() = Some((text.to_string(), Some(Instant::now() + Duration::from_secs_f32(s))))),
        None => NOTICE.with(|n| *n.borrow_mut() = Some(text.to_string())),
    }
}

/// Take the line [`banner`] shows away, when it is `text` (None: any).
pub fn clear_banner(text: Option<&str>) {
    BANNER.with(|b| { let mut b = b.borrow_mut(); if text.is_none() || b.as_ref().is_some_and(|(t, _)| Some(t.as_str()) == text) { *b = None; } });
    NOTICE.with(|b| { let mut b = b.borrow_mut(); if text.is_none() || b.as_deref() == text { *b = None; } });
}

/// Nothing to yield to: frames are the render thread's own.
pub async fn yield_to_browser() {}

/// Run `task` on the render thread, between frames.
pub fn spawn(task: impl Future<Output = ()> + 'static) {
    SPAWNED.with(|s| s.borrow_mut().push(Box::pin(task)));
}

/// A future that is ready `ms` milliseconds from now.
pub async fn sleep(ms: u32) {
    let until = Instant::now() + Duration::from_millis(ms as u64);
    std::future::poll_fn(move |_| if Instant::now() >= until { Poll::Ready(()) } else { Poll::Pending }).await
}

/// Run `work` on a helper thread; ready with its result.
async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let slot: Arc<Mutex<Option<T>>> = Arc::default();
    let fill = slot.clone();
    std::thread::spawn(move || { let r = work(); *fill.lock().unwrap_or_else(|e| e.into_inner()) = Some(r); });
    std::future::poll_fn(move |_| match slot.lock().unwrap_or_else(|e| e.into_inner()).take() { Some(r) => Poll::Ready(r), None => Poll::Pending }).await
}

/// A file of the host's bundle (`games/library.json`): the APK's
/// `assets/web/` (what build-android-host.py packs), or a URL.
pub async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if url.contains("://") {
        let (status, body) = request("GET", url, &[], None).await?;
        return if (200..300).contains(&status) { Ok(body) } else { Err(format!("{url}: HTTP {status}")) };
    }
    if bundled() { asset(url) } else { remote(url).await }
}

/// Fetch a game's file ahead, as a browser does. With the bundle in the
/// APK there is nothing to do (0); otherwise it lands in the phone's copy
/// of the bundle ([`remote`]), so opening the game reads it from there.
pub async fn prefetch(url: &str) -> Result<usize, String> {
    if url.contains("://") || bundled() { return Ok(0) }
    remote(url).await.map(|b| b.len())
}

/// See [`prefetch`]: the kept files are the phone's copy of the bundle,
/// which stays.
pub fn clear_prefetched() {}

/// Whether the APK carries the bundle (`assets/web/`, the standalone host
/// build-android-host.py packs). The Splitscreen Club app ships only the
/// library and fetches the bundle instead ([`remote`]).
fn bundled() -> bool {
    static BUNDLED: OnceLock<bool> = OnceLock::new();
    *BUNDLED.get_or_init(|| APP.get().is_some_and(|app| app.asset_manager().open(c"web/games/library.json").is_some()))
}

/// Where the bundle comes from when the APK has none: the browser host's
/// own files (`bundle=URL` in the query for a test server).
const BUNDLE: &str = "https://splitscreen.club/play/";

/// A file of the bundle from [`BUNDLE`], kept in the app's data per
/// bundle version (its `version.json`), so a game downloads once and a
/// new release of the browser host replaces the copy. Offline the last
/// version's copy is used.
async fn remote(path: &str) -> Result<Vec<u8>, String> {
    let rel = path.trim_start_matches("./").split(['?', '#']).next().unwrap_or_default().to_string();
    if rel.is_empty() || rel.split('/').any(|part| part == ".." || part.is_empty()) { return Err(format!("{path}: not a bundle path")) }
    let version = bundle_version().await;
    let dir = APP.get().and_then(|a| a.internal_data_path()).ok_or("no data folder")?.join("bundle").join(&version);
    let file = dir.join(&rel);
    if let Ok(bytes) = std::fs::read(&file) { return Ok(bytes) }
    let url = format!("{}{}?v={version}", bundle_base(), encode_path(&rel));
    let (status, body) = request("GET", &url, &[], None).await.map_err(|e| format!("{rel}: {e}"))?;
    if !(200..300).contains(&status) { return Err(format!("{rel}: HTTP {status} from {}", bundle_base())) }
    if let Some(parent) = file.parent() { let _ = std::fs::create_dir_all(parent); }
    // Written whole, then renamed: a download cut off never looks complete.
    let part = file.with_extension("part");
    if std::fs::write(&part, &body).is_ok() { let _ = std::fs::rename(&part, &file); }
    Ok(body)
}

fn bundle_base() -> String {
    let base = query_param("bundle").map(|b| percent_decode(&b)).unwrap_or_else(|| BUNDLE.to_string());
    if base.ends_with('/') { base } else { base + "/" }
}

/// The bundle's version, asked once a run; the copies of older versions
/// are removed then. Offline: the version last seen.
async fn bundle_version() -> String {
    static VERSION: Mutex<Option<String>> = Mutex::new(None);
    if let Some(v) = VERSION.lock().unwrap_or_else(|e| e.into_inner()).clone() { return v }
    let url = format!("{}version.json", bundle_base());
    let fresh = match request("GET", &url, &[("cache-control", "no-cache")], None).await {
        Ok((200, body)) => serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(|v| v["version"].as_str().map(String::from)),
        _ => None,
    };
    let clean = |v: &str| v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && !v.is_empty();
    let version = match fresh.filter(|v| clean(v)) {
        Some(v) => { store("bundle-version", &v); v }
        None => stored("bundle-version").filter(|v| clean(v)).unwrap_or_else(|| "offline".into()),
    };
    log(&format!("bundle {version} from {}", bundle_base()));
    if let Some(root) = APP.get().and_then(|a| a.internal_data_path()).map(|p| p.join("bundle")) {
        for old in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if old.file_name().to_string_lossy() != version { let _ = std::fs::remove_dir_all(old.path()); }
        }
    }
    *VERSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(version.clone());
    version
}

/// A bundle path as a URL path: everything but letters, digits, `-._~/`
/// percent-encoded (game files have spaces).
fn encode_path(rel: &str) -> String {
    rel.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) { out.push(v); i += 3; continue }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The browser hands a leaving game's last frame to `done` for the room's
/// TV. Not yet on Android: the TV keeps its own picture.
pub fn capture_frame(done: Box<dyn FnOnce([u32; 2], Vec<u8>)>) { drop(done); }

/// Read `assets/web/<path>` from the APK.
pub fn asset(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let app = APP.get().ok_or("no activity")?;
    let name = std::ffi::CString::new(format!("web/{}", path.trim_start_matches("./"))).map_err(|e| e.to_string())?;
    let mut a = app.asset_manager().open(&name).ok_or_else(|| format!("{path}: not in the app"))?;
    let mut bytes = Vec::new();
    a.read_to_end(&mut bytes).map_err(|e| format!("{path}: {e}"))?;
    Ok(bytes)
}

/// An HTTPS request with a method, headers and a JSON body: the status and
/// the response body (not an error on a non-2xx status).
pub async fn request(method: &str, url: &str, headers: &[(&str, &str)], json: Option<&str>) -> Result<(u16, Vec<u8>), String> {
    let (method, url, json) = (method.to_string(), url.to_string(), json.map(String::from));
    let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    off_thread(move || http(&method, &url, &headers, json.as_deref())).await
}

fn http(method: &str, url: &str, headers: &[(String, String)], json: Option<&str>) -> Result<(u16, Vec<u8>), String> {
    use std::io::Read;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(20))).build().into());
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for (k, v) in headers { builder = builder.header(k, v); }
    if json.is_some() { builder = builder.header("content-type", "application/json"); }
    let req = builder.body(json.unwrap_or_default().to_string()).map_err(|e| format!("{url}: {e}"))?;
    let mut res = agent.run(req).map_err(|e| format!("{url}: {e}"))?;
    let status = res.status().as_u16();
    let mut body = Vec::new();
    res.body_mut().as_reader().read_to_end(&mut body).map_err(|e| format!("{url}: {e}"))?;
    Ok((status, body))
}

fn store_dir() -> Option<std::path::PathBuf> { Some(APP.get()?.internal_data_path()?.join("store")) }

/// A value kept under `key` (the browser's `localStorage`).
pub fn stored(key: &str) -> Option<String> { std::fs::read_to_string(store_dir()?.join(key)).ok() }

/// Keep `value` under `key`.
pub fn store(key: &str, value: &str) {
    let Some(dir) = store_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(key), value);
}

/// A value from the intent's `query` extra (`cloud=off&players=2`), as the
/// browser host reads its address.
pub fn query_param(k: &str) -> Option<String> {
    QUERY.get()?.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
}

/// Where the cloud room lives by default (a browser page: its own origin).
pub fn origin() -> String { "https://splitscreen.club".into() }

/// Resident memory in MB.
pub fn memory_mb() -> u32 {
    let pages: u64 = std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse().ok()).unwrap_or(0);
    (pages * 4096 >> 20) as u32
}

/// Panics go to logcat.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
}

/// A phone: the browser's phone preset (`quality=full` turns it off).
pub fn mobile() -> bool { !matches!(query_param("quality").as_deref(), Some("full" | "high")) }

const MOBILE_BASE_HEIGHT: u32 = 1080;

/// The display rate to ask for (`fps=120` in the query); None leaves the
/// system's choice (60 Hz on the Tab S9+).
pub fn requested_hz() -> Option<f32> { query_param("fps").and_then(|v| v.parse::<f32>().ok()).filter(|hz| *hz >= 20.0) }

/// Ask the display for `hz` (ANativeWindow_setFrameRate, API 30; looked up
/// at run time since the host also runs on API 26).
fn request_frame_rate(w: &NativeWindow, hz: f32) {
    unsafe extern "C" {
        fn dlopen(name: *const std::ffi::c_char, flags: i32) -> *mut std::ffi::c_void;
        fn dlsym(handle: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }
    type SetFrameRate = unsafe extern "C" fn(*mut ndk_sys::ANativeWindow, f32, i8) -> i32;
    // SAFETY: libandroid.so is loaded already; the symbol has this signature
    // (compatibility 0 = default) and the window outlives the call.
    let rc = unsafe {
        let lib = dlopen(c"libandroid.so".as_ptr(), 1);
        let f = if lib.is_null() { std::ptr::null_mut() } else { dlsym(lib, c"ANativeWindow_setFrameRate".as_ptr()) };
        if f.is_null() { log("ANativeWindow_setFrameRate missing (API < 30)"); return }
        std::mem::transmute::<*mut std::ffi::c_void, SetFrameRate>(f)(w.ptr().as_ptr(), hz, 0)
    };
    log(&format!("asked the display for {hz} Hz ({})", if rc == 0 { "ok" } else { "refused" }));
}

fn mobile_light(mut light: render::Lighting) -> render::Lighting {
    // GI stays: the compact pass only looks the baked irradiance volume up
    // (cheap), and without it Spellstorm and the room were far too dark
    // (Joep). Its expensive part, the RSM gather, is off (KERN_RSM, main).
    light.contact_occlusion = 0.0;
    light.reflection_occlusion = 0.0;
    light.traced_reflections = 0.0;
    light.bloom_boost = 0.0;
    light.lens_flare = 0.0;
    light
}

/// Replace the running game with `game` at the next frame, keeping the
/// GPU device and everything compiled on it.
pub fn switch(cfg: crate::Config, game: Box<dyn Game>) {
    NEXT.with(|n| *n.borrow_mut() = Some((Parked::new(cfg, game), None)));
}

/// A game set aside with its scene still on the GPU ([`switch_keeping`]).
pub struct Parked {
    pub cfg: crate::Config,
    pub game: Box<dyn Game>,
    renderer: Option<Renderer>,
}

impl Parked {
    pub fn new(cfg: crate::Config, game: Box<dyn Game>) -> Self { Parked { cfg, game, renderer: None } }
}

pub type Keep = Box<dyn FnOnce(Parked)>;

/// [`switch`], handing the replaced game to `keep` as it was.
pub fn switch_keeping(cfg: crate::Config, game: Box<dyn Game>, keep: impl FnOnce(Parked) + 'static) {
    NEXT.with(|n| *n.borrow_mut() = Some((Parked::new(cfg, game), Some(Box::new(keep)))));
}

/// Bring a [`Parked`] game back, handing the one it replaces to `keep`.
pub fn resume(parked: Parked, keep: Option<Box<dyn FnOnce(Parked)>>) {
    NEXT.with(|n| *n.borrow_mut() = Some((parked, keep)));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// See the browser shell's `Exit`: Back comes on release, Next/Previous are Back + RB/LB.
pub enum Exit { Quit, Back, Next, Previous, ReportBug }

/// No keyboard F4 on a phone host (see the browser's `take_report_key`).
pub fn take_report_key() -> bool { false }

thread_local! {
    static IN_GAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The host tells the shell whether a game (not its room) has the screen:
/// only then is Back the host's, read on release with its RB/LB chords
/// ([`crate::back_chord`]). In the room Back stays the room's.
pub fn set_in_game(on: bool) { IN_GAME.with(|g| g.set(on)); }

pub fn on_exit(f: impl FnMut(Exit) -> bool + 'static) { EXIT.with(|e| *e.borrow_mut() = Some(Box::new(f))); }

pub fn takes_back() -> bool { EXIT.try_with(|e| e.borrow().is_some()).unwrap_or(false) }

fn exit(why: Exit) -> bool { EXIT.with(|e| match e.borrow_mut().as_mut() { Some(f) => f(why), None => false }) }

pub fn player_joins(name: &str, color: Option<[f32; 3]>) { JOINS.with(|j| j.borrow_mut().push((name.to_string(), color))); }

pub fn game_takes_joins() -> bool { TAKES.with(|t| t.get()) }

pub fn on_devices(f: impl FnMut(&[(crate::Device, crate::Controls)]) -> Option<Vec<Vec<crate::Device>>> + 'static) {
    SEATS.with(|s| *s.borrow_mut() = Some(Box::new(f)));
}

/// Show `game` from the next frame on. Returns at once: the render
/// thread's loop drives the frames.
pub fn run(cfg: crate::Config, game: Box<dyn Game>) {
    let mut devices = Devices::new(cfg.keyboard);
    devices.pointer_enabled = cfg.pointer;
    let mobile = mobile();
    if mobile { log("phone preset: lower resolution, no RSM, bloom or contact occlusion (quality=full turns it off)"); }
    let shell = Shell { cfg, game, devices, acc: 0.0, last: Instant::now(), frames: 0, started: Instant::now(), announced: false, switched: None, exited: false, back: Default::default(),
        times: Vec::new(), render_height: query_param("render").and_then(|h| h.parse().ok()), quality: None, mobile, profiling: false, gpu_passes: Vec::new(), gpu_total: 0.0,
        upscale: query_param("upscale").and_then(|m| m.parse().map_err(|e: String| log(&e)).ok()).filter(|m| *m != render::upscale::UpscaleMode::Native),
        bench: crate::bench::Bench::from_query(query_param("bench").as_deref()) };
    SHELL.with(|s| *s.borrow_mut() = Some(shell));
}

// --- The activity --------------------------------------------------------

/// What the activity's thread hands the render thread.
enum Event {
    Window(NativeWindow),
    /// The window goes: drop the surface, then answer.
    NoWindow(mpsc::Sender<()>),
    Resized,
    Touch(u64, winit::event::TouchPhase, [f32; 2]),
    /// The phone's own Back (not a pad's).
    Back,
    Visible(bool),
}

/// Run the activity: `start` runs first on the render thread (it calls
/// [`run`] and [`spawn`]s its tasks), then the frames. Returns when
/// Android destroys the activity.
pub fn main(app: AndroidApp, start: impl FnOnce() + Send + 'static) {
    let _ = APP.set(app.clone());
    let _ = QUERY.set(intent_query(&app));
    // `env=NAME:VALUE,NAME:VALUE` sets renderer switches (KERN_...) for a test run.
    for pair in query_param("env").unwrap_or_default().split(',') {
        // SAFETY: before this process starts its render and helper threads.
        if let Some((k, v)) = pair.split_once(':') { unsafe { std::env::set_var(k.trim(), v.trim()) } }
    }
    // Phone default, measured on Adreno 740 and 830 (2026-11-23): the full
    // shade pass cost 67 ms a frame on the 740 at 378 lines, the simple one
    // 9.5 ms at 1080. `env=` overrides. Bounded draws: see `direct_draws`.
    // SAFETY: as above, no other threads yet.
    // Compact rather than simple: what a browser runs (WebGPU binds too few
    // textures for the full pass), so games look as they do there; simple
    // skips game surfaces.
    if std::env::var_os("KERN_RSM").is_none() { unsafe { std::env::set_var("KERN_RSM", "0") } }
    if std::env::var_os("KERN_SIMPLE_LIGHTING").is_none() && std::env::var_os("KERN_COMPACT_SHADE").is_none() { unsafe { std::env::set_var("KERN_COMPACT_SHADE", "1") } }
    // The cheap irradiance blend (KERN_PHONE_GI): 2026-11-25 the full DDGI
    // lookup was about a third of the compact pass on the Adreno 740.
    if std::env::var_os("KERN_PHONE_GI").is_none() { unsafe { std::env::set_var("KERN_PHONE_GI", "1") } }
    // The compact pass as two smaller kernels (KERN_SPLIT_SHADE).
    if std::env::var_os("KERN_SPLIT_SHADE").is_none() { unsafe { std::env::set_var("KERN_SPLIT_SHADE", "1") } }
    install_panic_hook();
    use android_activity::WindowManagerFlags as F;
    app.set_window_flags(F::KEEP_SCREEN_ON | F::FULLSCREEN | F::LAYOUT_IN_SCREEN, F::empty());
    for axis in [Axis::Z, Axis::Rz, Axis::HatX, Axis::HatY, Axis::Ltrigger, Axis::Rtrigger, Axis::Brake, Axis::Gas] { app.enable_motion_axis(axis); }
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let render_stop = stop.clone();
    let render_app = app.clone();
    let render = std::thread::Builder::new().name("warp-render".into()).spawn(move || {
        // cpal and others ask ndk-context for the VM: attach this thread.
        let vm = unsafe { jni::JavaVM::from_raw(render_app.vm_as_ptr().cast()) };
        let _env = vm.as_ref().ok().and_then(|vm| vm.attach_current_thread_permanently().ok());
        start();
        render_loop(rx, render_stop);
    });
    let mut checked = Instant::now();
    loop {
        let mut destroyed = false;
        app.poll_events(Some(Duration::from_millis(50)), |event| match event {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                immersive(&app);
                if let Some(w) = app.native_window() { let _ = tx.send(Event::Window(w)); }
            }
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                let (ack, done) = mpsc::channel();
                if tx.send(Event::NoWindow(ack)).is_ok() { let _ = done.recv_timeout(Duration::from_secs(3)); }
            }
            PollEvent::Main(MainEvent::WindowResized { .. } | MainEvent::ContentRectChanged { .. } | MainEvent::ConfigChanged { .. }) => { let _ = tx.send(Event::Resized); }
            PollEvent::Main(MainEvent::GainedFocus) => { immersive(&app); let _ = tx.send(Event::Visible(true)); }
            PollEvent::Main(MainEvent::Pause) => { let _ = tx.send(Event::Visible(false)); }
            PollEvent::Main(MainEvent::Resume { .. }) => { let _ = tx.send(Event::Visible(true)); }
            PollEvent::Main(MainEvent::Destroy) => destroyed = true,
            PollEvent::Main(MainEvent::InputAvailable) => input(&app, &tx),
            _ => {}
        });
        // Input can arrive without the InputAvailable wake on some phones.
        input(&app, &tx);
        if checked.elapsed() > Duration::from_secs(2) {
            checked = Instant::now();
            if let Some(ids) = device_ids(&app) { crate::android_pads::keep_only(&ids); }
        }
        if destroyed || render.as_ref().map_or(true, |r| r.is_finished()) { break; }
    }
    stop.store(true, std::sync::atomic::Ordering::Release);
    if let Ok(r) = render { let _ = r.join(); }
    // The host runs in a process of its own (the app's `:host`, or the
    // standalone APK), whose statics ([`APP`], [`QUERY`]) belong to this
    // activity: end it, so the next "Host on this phone" starts fresh
    // with its own query.
    std::process::exit(0);
}

/// Drain Android's input queue: pads into [`crate::android_pads`],
/// touches and the phone's Back to the render thread. Everything else
/// (volume keys) stays the system's.
fn input(app: &AndroidApp, tx: &mpsc::Sender<Event>) {
    let Ok(mut events) = app.input_events_iter() else { return };
    while events.next(|event| match event {
        InputEvent::KeyEvent(k) => {
            let source: u32 = k.source().into();
            let code: u32 = k.key_code().into();
            let down = k.action() == KeyAction::Down;
            // SOURCE_GAMEPAD, SOURCE_JOYSTICK or SOURCE_DPAD on a pad.
            let pad = source & 0x0000_0401 == 0x0000_0401 || source & 0x0100_0010 == 0x0100_0010;
            // Every pad key is the host's (Home and volume keys stay Android's):
            // handled, so Android never turns B or Select into its Back.
            if pad && !matches!(code, 3 | 24 | 25 | 164) {
                crate::android_pads::key(k.device_id(), code, down);
                return InputStatus::Handled;
            }
            if code == 4 {
                if down && k.repeat_count() == 0 { let _ = tx.send(Event::Back); }
                return InputStatus::Handled;
            }
            InputStatus::Unhandled
        }
        InputEvent::MotionEvent(m) => {
            let source: u32 = m.source().into();
            if source & 0x0100_0010 == 0x0100_0010 {
                let p = m.pointer_at_index(0);
                let a = |axis| p.axis_value(axis);
                crate::android_pads::motion(m.device_id(), crate::android_pads::Axes { x: a(Axis::X), y: a(Axis::Y), z: a(Axis::Z), rz: a(Axis::Rz), hat_x: a(Axis::HatX), hat_y: a(Axis::HatY),
                    ltrigger: a(Axis::Ltrigger), rtrigger: a(Axis::Rtrigger), brake: a(Axis::Brake), gas: a(Axis::Gas) });
                return InputStatus::Handled;
            }
            if source & 0x0000_1002 == 0x0000_1002 {
                use winit::event::TouchPhase as T;
                let at = |i: usize| { let p = m.pointer_at_index(i); (p.pointer_id() as u64, [p.axis_value(Axis::X), p.axis_value(Axis::Y)]) };
                match m.action() {
                    MotionAction::Down | MotionAction::PointerDown => { let (id, pos) = at(m.pointer_index()); let _ = tx.send(Event::Touch(id, T::Started, pos)); }
                    MotionAction::Up | MotionAction::PointerUp => { let (id, pos) = at(m.pointer_index()); let _ = tx.send(Event::Touch(id, T::Ended, pos)); }
                    MotionAction::Move => for i in 0..m.pointer_count() { let (id, pos) = at(i); let _ = tx.send(Event::Touch(id, T::Moved, pos)); },
                    MotionAction::Cancel => for i in 0..m.pointer_count() { let (id, pos) = at(i); let _ = tx.send(Event::Touch(id, T::Cancelled, pos)); },
                    _ => {}
                }
                return InputStatus::Handled;
            }
            InputStatus::Unhandled
        }
        _ => InputStatus::Unhandled,
    }) {}
}

/// Hide the status and navigation bars (sticky immersive), on Android's UI thread.
fn immersive(app: &AndroidApp) {
    let (vm, activity) = (app.vm_as_ptr() as usize, app.activity_as_ptr() as usize);
    app.run_on_java_main_thread(Box::new(move || {
        let run = || -> jni::errors::Result<()> {
            let vm = unsafe { jni::JavaVM::from_raw(vm as *mut _) }?;
            let mut env = vm.attach_current_thread()?;
            let activity = unsafe { jni::objects::JObject::from_raw(activity as jni::sys::jobject) };
            let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
            let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
            // IMMERSIVE_STICKY | FULLSCREEN | HIDE_NAVIGATION | LAYOUT_* | LAYOUT_STABLE
            let flags = 0x1000 | 0x4 | 0x2 | 0x400 | 0x200 | 0x100;
            env.call_method(&decor, "setSystemUiVisibility", "(I)V", &[jni::objects::JValue::Int(flags)])?;
            Ok(())
        };
        if let Err(e) = run() { log(&format!("immersive: {e}")); }
    }));
}

/// The device ids Android lists now (InputDevice.getDeviceIds).
fn device_ids(app: &AndroidApp) -> Option<Vec<i32>> {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let ids = env.call_static_method("android/view/InputDevice", "getDeviceIds", "()[I", &[]).ok()?.l().ok()?;
    let ids = jni::objects::JIntArray::from(ids);
    let n = env.get_array_length(&ids).ok()? as usize;
    let mut out = vec![0; n];
    env.get_int_array_region(&ids, 0, &mut out).ok()?;
    Some(out)
}

/// The launching intent's `query` string extra, as key=value pairs.
fn intent_query(app: &AndroidApp) -> Vec<(String, String)> {
    let read = || -> Option<String> {
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let intent = env.call_method(&activity, "getIntent", "()Landroid/content/Intent;", &[]).ok()?.l().ok()?;
        let key = env.new_string("query").ok()?;
        let value = env.call_method(&intent, "getStringExtra", "(Ljava/lang/String;)Ljava/lang/String;", &[(&key).into()]).ok()?.l().ok()?;
        if value.is_null() { return None; }
        env.get_string(&value.into()).ok().map(String::from)
    };
    let query = read().unwrap_or_default();
    if !query.is_empty() { log(&format!("query: {query}")); }
    query.trim_start_matches('?').split('&').filter_map(|kv| { let (k, v) = kv.split_once('=').unwrap_or((kv, "")); (!k.is_empty()).then(|| (k.to_string(), v.to_string())) }).collect()
}

// --- The render thread ---------------------------------------------------

struct Gfx {
    instance: wgpu::Instance,
    gpu: Gpu,
    out: Output,
    /// The window's surface; None while the app is in the background.
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration, NativeWindow)>,
    /// The game on screen's scene.
    renderer: Option<Renderer>,
    /// The loading screen's and banner's HUD.
    hud: HudRenderer,
    /// The loading screen's icon is on the HUD.
    hud_icon: bool,
    /// The size the loading screen's background picture was made for.
    hud_background: Option<[u32; 2]>,
    visible: bool,
}

fn surface_for(instance: &wgpu::Instance, window: &NativeWindow) -> Result<wgpu::Surface<'static>, String> {
    use raw_window_handle::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};
    let ptr = std::ptr::NonNull::new(window.ptr().as_ptr().cast()).ok_or("no window")?;
    // SAFETY: the NativeWindow is kept next to the surface and dropped after it.
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(ptr)),
        })
    }.map_err(|e| e.to_string())
}

/// The window's size can change without an event reaching us in time (it
/// comes up portrait and turns landscape): follow it every frame, as the
/// browser shell follows its canvas.
fn follow_window(gfx: &mut Gfx) {
    let Some((surface, config, w)) = gfx.surface.as_mut() else { return };
    let size = window_size(w);
    if [config.width, config.height] != size {
        log(&format!("window {}x{} (was {}x{})", size[0], size[1], config.width, config.height));
        config.width = size[0];
        config.height = size[1];
        surface.configure(&gfx.gpu.device, config);
    }
}

fn window_size(w: &NativeWindow) -> [u32; 2] { [w.width().max(1) as u32, w.height().max(1) as u32] }

/// Adreno 8xx drivers (the 830, 2026-11-23) drop indirect draws: only
/// emissive and transparent things showed. There the renderer draws with
/// bounded direct draws. The 740 draws indirect fine, and Spellstorm seemed
/// to lose its walls and wizards with bounded draws, so only where needed.
fn direct_draws(gpu: &str) {
    let adreno = gpu.split("Adreno (TM) ").nth(1).and_then(|r| r.trim().get(..1)).and_then(|d| d.parse::<u32>().ok());
    if adreno.is_some_and(|series| series >= 8) {
        for k in ["KERN_SIMPLE_CULL", "KERN_BOUNDED_DRAWS"] {
            // SAFETY: the renderer reads these when it is made, after this;
            // nothing else in the process reads or writes them.
            if std::env::var_os(k).is_none() { unsafe { std::env::set_var(k, "1") } }
        }
        log("bounded direct draws (this GPU drops indirect draws)");
    }
}

/// The first window: the GPU, its surface and the presenter.
fn on_window(window: NativeWindow) -> Result<(), String> {
    let size = window_size(&window);
    let instance = render::gpu::instance();
    let surface = surface_for(&instance, &window)?;
    if let Some(hz) = requested_hz() { request_frame_rate(&window, hz); }
    let (gpu, config) = Gpu::for_surface(&instance, &surface, size[0], size[1])?;
    surface.configure(&gpu.device, &config);
    log(&format!("GPU: {} ({:?}), {}x{}, {:?}", gpu.info.name, gpu.info.backend, config.width, config.height, config.format));
    direct_draws(&gpu.info.name);
    let limits = gpu.device.limits();
    log(&format!("{} storage buffers, {} sampled textures per shader stage", limits.max_storage_buffers_per_shader_stage, limits.max_sampled_textures_per_shader_stage));
    gpu.device.on_uncaptured_error(Arc::new(|e: wgpu::Error| log(&format!("GPU error: {e}"))));
    gpu.device.set_device_lost_callback(|reason, message| log(&format!("GPU device lost ({reason:?}): {message}")));
    let out = Output::new(&gpu, config.format)?;
    let hud = HudRenderer::new(&gpu, config.format)?;
    GFX.with(|g| *g.borrow_mut() = Some(Gfx { instance, gpu, out, surface: Some((surface, config, window)), renderer: None, hud, hud_icon: false, hud_background: None, visible: true }));
    Ok(())
}

fn render_loop(rx: mpsc::Receiver<Event>, stop: Arc<std::sync::atomic::AtomicBool>) {
    // The config of the last surface, for the next window.
    let mut last_config: Option<wgpu::SurfaceConfiguration> = None;
    let mut back_at: Option<Instant> = None;
    loop {
        if stop.load(std::sync::atomic::Ordering::Acquire) { break; }
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Window(w) => {
                    let has = GFX.with(|g| g.borrow().is_some());
                    if has {
                        let size = window_size(&w);
                        GFX.with(|g| {
                            let mut g = g.borrow_mut();
                            let Some(gfx) = g.as_mut() else { return };
                            match surface_for(&gfx.instance, &w) {
                                Ok(surface) => {
                                    let Some(mut config) = last_config.clone() else { return };
                                    config.width = size[0];
                                    config.height = size[1];
                                    surface.configure(&gfx.gpu.device, &config);
                                    if let Some(hz) = requested_hz() { request_frame_rate(&w, hz); }
                                    gfx.surface = Some((surface, config, w));
                                    log(&format!("window back, {}x{}", size[0], size[1]));
                                }
                                Err(e) => log(&format!("surface: {e}")),
                            }
                        });
                    } else if let Err(e) = on_window(w) {
                        log(&format!("GPU setup failed: {e}"));
                    }
                }
                Event::NoWindow(ack) => {
                    GFX.with(|g| if let Some(gfx) = g.borrow_mut().as_mut() { if let Some((_, c, _)) = gfx.surface.take() { last_config = Some(c); } });
                    let _ = ack.send(());
                }
                Event::Resized => GFX.with(|g| {
                    if let Some(gfx) = g.borrow_mut().as_mut() && let Some((surface, config, w)) = gfx.surface.as_mut() {
                        let size = window_size(w);
                        if [config.width, config.height] != size { config.width = size[0]; config.height = size[1]; surface.configure(&gfx.gpu.device, config); }
                    }
                }),
                // The phone is the TV: players hold controllers (or their own
                // phones), so a finger on the screen plays nobody. It used to
                // drive a virtual stick that joined as a keyboard player
                // ("Press Enter to join", Joep 2026-11-24).
                Event::Touch(..) => {}
                Event::Back => {
                    // A game: back to the room. The room: twice to stop hosting.
                    if exit(Exit::Back) { continue; }
                    if back_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) { finish(); } else {
                        back_at = Some(Instant::now());
                        banner("Press Back again to stop hosting", Some(3.0));
                    }
                }
                Event::Visible(on) => GFX.with(|g| if let Some(gfx) = g.borrow_mut().as_mut() { gfx.visible = on; }),
            }
        }
        if let Some(c) = GFX.with(|g| g.borrow().as_ref().and_then(|g| g.surface.as_ref().map(|s| s.1.clone()))) { last_config = Some(c); }
        // Tasks start once there is a GPU: the loading screen can show.
        let ready = GFX.with(|g| g.borrow().is_some());
        if ready { poll_tasks(); }
        let drawable = GFX.with(|g| g.borrow().as_ref().is_some_and(|g| g.surface.is_some() && g.visible));
        let running = SHELL.with(|s| s.borrow().is_some());
        if drawable && running {
            let result = SHELL.with(|s| s.borrow_mut().as_mut().map(|s| s.frame()));
            if let Some(Err(e)) = result { log(&format!("frame error: {e}")); std::thread::sleep(Duration::from_millis(100)); }
        } else {
            if drawable { draw_loading(); }
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}

/// Poll every task once (a busy-polled executor: tasks wait on time or
/// on helper threads, and the loop comes round every frame).
fn poll_tasks() {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut tasks = TASKS.with(|t| std::mem::take(&mut *t.borrow_mut()));
    tasks.extend(SPAWNED.with(|s| std::mem::take(&mut *s.borrow_mut())));
    tasks.retain_mut(|t| t.as_mut().poll(&mut cx).is_pending());
    TASKS.with(|t| { let mut t = t.borrow_mut(); tasks.append(&mut t); *t = tasks; });
}

/// Stop hosting: the activity finishes.
fn finish() {
    let Some(app) = APP.get() else { return };
    let (vm, activity) = (app.vm_as_ptr() as usize, app.activity_as_ptr() as usize);
    app.run_on_java_main_thread(Box::new(move || {
        let run = || -> jni::errors::Result<()> {
            let vm = unsafe { jni::JavaVM::from_raw(vm as *mut _) }?;
            let mut env = vm.attach_current_thread()?;
            let activity = unsafe { jni::objects::JObject::from_raw(activity as jni::sys::jobject) };
            env.call_method(&activity, "finish", "()V", &[])?;
            Ok(())
        };
        if let Err(e) = run() { log(&format!("finish: {e}")); }
    }));
}

const BRAND_BG: wgpu::Color = wgpu::Color { r: 0.0042, g: 0.0052, b: 0.013, a: 1.0 };

fn text(d: &mut HudDrawList, id: u64, s: &str, x: f32, y: f32, size: f32, color: [u8; 4], width: f32, font: FontFace, clip: [f32; 4]) {
    d.text.push(TextRun { id, text: s.into(), position: [x, y], bounds: clip, font_size: size, color, font, align: TextAlign::Center, width });
}

/// The loading screen as the browser page and the desktop room show it:
/// the icon, the Bungee wordmark "Splitscreen.Club" in amber and violet,
/// a rounded amber-to-violet bar and the step, on the brand's dark blue
/// with its two soft glows.
fn draw_loading() {
    let _ = GFX.try_with(|g| {
        let Ok(mut g) = g.try_borrow_mut() else { return };
        let Some(gfx) = g.as_mut() else { return };
        follow_window(gfx);
        let Some((surface, config, _)) = gfx.surface.as_ref() else { return };
        let size = [config.width as f32, config.height as f32];
        let (fraction, step) = LOADING.with(|l| l.borrow().clone());
        let mut d = HudDrawList::new([config.width, config.height]);
        let clip = [0.0, 0.0, size[0], size[1]];
        let s = size[1] / 720.0;
        let (wordmark, _, body) = brand_fonts();
        // The glows, one picture at the screen's own size: stacked
        // translucent circles banded (Joep: "concentrische cirkels").
        let px = [config.width, config.height];
        if gfx.hud_background != Some(px) {
            gfx.hud.set_image(&gfx.gpu, render::hud::HudImage(0x1C1), &loading_background(px), render::hud::ImageFilter::Nearest);
            gfx.hud_background = Some(px);
        }
        d.image([0.0, 0.0, size[0], size[1]], render::hud::HudImage(0x1C1), [0.0, 0.0, 1.0, 1.0], [1.0; 4], clip);
        let icon = 96.0 * s;
        if !gfx.hud_icon {
            if let Ok(pic) = render::picture::decode(include_bytes!("../../../../lobbies/game-room/assets/splitscreen-club-icon.png"), 1.0) {
                gfx.hud.set_image(&gfx.gpu, render::hud::HudImage(0x1C0), &pic, render::hud::ImageFilter::Linear);
            }
            gfx.hud_icon = true;
        }
        let top = size[1] * 0.26;
        d.image([(size[0] - icon) * 0.5, top, icon, icon], render::hud::HudImage(0x1C0), [0.0, 0.0, 1.0, 1.0], [1.0; 4], clip);
        let wf = 60.0 * s;
        let (a, b) = ("Splitscreen", ".Club");
        let (aw, bw) = (render::hud::measure(a, wordmark, wf)[0], render::hud::measure(b, wordmark, wf)[0]);
        let wx = (size[0] - aw - bw) * 0.5;
        let wy = top + icon + 22.0 * s;
        d.text.push(TextRun { id: 1, text: a.into(), position: [wx, wy], bounds: clip, font_size: wf, color: [255, 180, 84, 255], font: wordmark, align: TextAlign::Left, width: aw + 4.0 });
        d.text.push(TextRun { id: 3, text: b.into(), position: [wx + aw, wy], bounds: clip, font_size: wf, color: [167, 139, 250, 255], font: wordmark, align: TextAlign::Left, width: bw + 4.0 });
        let (w, h) = ((size[0] * 0.36).min(560.0 * s), 10.0 * s);
        let (x, y) = ((size[0] - w) * 0.5, wy + wf * 1.5);
        round_rect(&mut d, [x, y, w, h], h * 0.5, [1.0, 1.0, 1.0, 0.08], clip);
        let done = w * fraction.unwrap_or(1.0).clamp(0.0, 1.0);
        // Amber into violet, in slices.
        if done >= h {
            let n = 24;
            for i in 0..n {
                let (f0, f1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
                let t = (f0 + f1) * 0.5;
                let col = [AMBER[0] + (VIOLET[0] - AMBER[0]) * t, AMBER[1] + (VIOLET[1] - AMBER[1]) * t, AMBER[2] + (VIOLET[2] - AMBER[2]) * t, 1.0];
                let (sx, ex) = (x + (done - h) * f0 + h * 0.5, x + (done - h) * f1 + h * 0.5);
                d.rect([sx, y, ex - sx + 0.5, h], col, clip);
            }
            d.circle([x + h * 0.5, y + h * 0.5], h * 0.5, AMBER, clip);
            d.circle([x + done - h * 0.5, y + h * 0.5], h * 0.5, VIOLET, clip);
        }
        text(&mut d, 2, &step, 0.0, y + 26.0 * s, 18.0 * s, [168, 173, 196, 255], size[0], body, clip);
        let texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => { surface.configure(&gfx.gpu.device, config); return; }
        };
        let view = texture.texture.create_view(&Default::default());
        if gfx.hud.prepare(&gfx.gpu, &d).is_err() { return; }
        let mut enc = gfx.gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("loading"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(BRAND_BG), store: wgpu::StoreOp::Store } })],
                ..Default::default()
            });
            let _ = gfx.hud.render(&mut pass);
        }
        gfx.gpu.queue.submit([enc.finish()]);
        gfx.gpu.queue.present(texture);
    });
}

/// The browser page's loading background (index.html #warp-loading):
/// `radial-gradient(60% 50% at 30% 35%, rgba(255,180,84,.16), transparent 70%)`
/// over `radial-gradient(55% 50% at 72% 65%, rgba(167,139,250,.18), transparent 70%)`
/// over #0d1020, in sRGB like a browser, dithered by a pixel hash so the
/// 8-bit steps of so dark a gradient don't show as rings.
fn loading_background([w, h]: [u32; 2]) -> render::picture::Picture {
    let (wf, hf) = (w as f32, h as f32);
    let glows = [([0.30, 0.35], [0.60, 0.50], [255.0, 180.0, 84.0], 0.16), ([0.72, 0.65], [0.55, 0.50], [167.0, 139.0, 250.0], 0.18)];
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut c = [13.0f32, 16.0, 32.0];
            // The violet first: CSS paints the first gradient on top.
            for (at, radius, col, alpha) in glows.iter().rev() {
                let dx = (x as f32 + 0.5 - at[0] * wf) / (radius[0] * wf);
                let dy = (y as f32 + 0.5 - at[1] * hf) / (radius[1] * hf);
                let a = alpha * (1.0 - (dx * dx + dy * dy).sqrt() / 0.7).max(0.0);
                for k in 0..3 { c[k] += (col[k] - c[k]) * a; }
            }
            // Triangular noise of one step, from a hash of the pixel.
            let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
            n ^= n >> 15; n = n.wrapping_mul(0x2C1B_3C6D); n ^= n >> 12;
            let dither = ((n & 0xffff) as f32 + ((n >> 16) & 0xffff) as f32) / 65535.0 - 1.0;
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 { rgba[i + k] = (c[k] + dither).round().clamp(0.0, 255.0) as u8; }
            rgba[i + 3] = 255;
        }
    }
    render::picture::Picture { width: w, height: h, rgba }
}

struct Shell {
    cfg: crate::Config,
    game: Box<dyn Game>,
    devices: Devices,
    acc: f32,
    last: Instant,
    frames: u64,
    started: Instant,
    announced: bool,
    switched: Option<Instant>,
    exited: bool,
    /// Back and its chords in a game: the room comes on Back's release.
    back: crate::back_chord::BackChords,
    times: Vec<[f32; 4]>,
    render_height: Option<u32>,
    quality: Option<crate::quality::AdaptiveQuality>,
    mobile: bool,
    /// GPU timestamps are on, and the last frame's pass times.
    profiling: bool,
    gpu_passes: Vec<(&'static str, f64)>,
    gpu_total: f64,
    /// `upscale=temporal-quality|temporal-balanced|temporal-performance`:
    /// the 3D view renders below the picture and Warp's portable temporal
    /// upscaler rebuilds it, so the ladder can start at the screen's own
    /// height (tablets) instead of [`MOBILE_BASE_HEIGHT`]. None when off or
    /// when the renderer refused it.
    upscale: Option<render::upscale::UpscaleMode>,
    /// `bench=1`: fixed heights and one `BENCH {json}` line per game ([`crate::bench`]).
    bench: Option<crate::bench::Bench>,
}

impl Shell {
    fn frame(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.game.tuning(&mut self.devices.tuning);
        let step = 1.0 / self.cfg.tick_hz;
        self.acc += dt;
        let mut steps = 0;
        let ms = |t: Instant| t.elapsed().as_secs_f32() * 1000.0;
        if self.game.screen() == Screen::Hidden { return Ok(()); }
        let next = NEXT.with(|n| n.borrow_mut().take());
        let mut g = GFX.with(|g| g.borrow_mut().take()).ok_or("no GPU")?;
        let result = self.frame_with(&mut g, next, dt, step, &mut steps, ms);
        GFX.with(|slot| *slot.borrow_mut() = Some(g));
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn frame_with(&mut self, live: &mut Gfx, next: Option<(Parked, Option<Keep>)>, dt: f32, step: f32, steps: &mut i32, ms: impl Fn(Instant) -> f32) -> Result<(), String> {
        if let Some((next, keep)) = next {
            let old = Parked { cfg: std::mem::replace(&mut self.cfg, next.cfg), game: std::mem::replace(&mut self.game, next.game), renderer: std::mem::replace(&mut live.renderer, next.renderer) };
            if let Some(keep) = keep { keep(old); }
            self.devices.keyboard_enabled = self.cfg.keyboard;
            self.devices.pointer_enabled = self.cfg.pointer;
            self.exited = false;
            self.back = Default::default();
            self.acc = 0.0;
            self.switched = Some(Instant::now());
            self.times.clear();
            self.profiling = false;
            if let Some(b) = &mut self.bench { b.restart(); }
            if let Some(q) = &mut self.quality { q.set_enabled(false); q.set_enabled(true); q.idle(); q.set_target_fps(self.game.target_fps(), requested_hz().map(f64::from)); }
        }
        follow_window(live);
        let Some((_, config, _)) = live.surface.as_ref() else { return Ok(()) };
        let size = [config.width, config.height];
        self.devices.poll();
        let t_tick = Instant::now();
        while self.acc >= step && *steps < 6 {
            self.acc -= step;
            *steps += 1;
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
        if *steps == 6 { self.acc = 0.0; }
        TAKES.with(|t| t.set(self.game.takes_joins()));
        let tick_ms = ms(t_tick);
        let alpha = (self.acc / step).clamp(0.0, 1.0);
        let split = self.game.split();
        if live.renderer.is_none() && !self.announced {
            self.announced = true;
            return Ok(());
        }
        let t_draw = Instant::now();
        let mut frame = Frame { gpu: &live.gpu, size, dt, alpha, split, scene_fps: if dt > 0.0 { 1.0 / dt } else { 0.0 }, scene_worst_ms: dt * 1000.0, focused: true, hdr: None, stream_generated_textures: true, renderer: &mut live.renderer };
        let mut draw: Draw = self.game.draw(&mut frame)?;
        if let Some(q) = &self.quality {
            draw.light = q.apply_light(draw.light);
            if !q.rung().bloom { draw.light.bloom_boost = 0.0; }
        }
        if self.mobile { draw.light = mobile_light(draw.light); }
        draw_banner(&mut draw.hud, size);
        let Some(renderer) = live.renderer.as_ref() else { return Ok(()) };
        // Once per renderer in effect (set_upscale returns at once when the
        // mode holds); a refusal turns it off for the rest of the run.
        if let Some(mode) = self.upscale {
            if let Err(e) = renderer.set_upscale(&live.gpu, mode) { log(&format!("upscale {} off: {e}", mode.as_str())); self.upscale = None; }
        }
        let draw_ms = ms(t_draw);
        let t_out = Instant::now();
        let (surface, config, _) = live.surface.as_ref().ok_or("no surface")?;
        let texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                surface.configure(&live.gpu.device, config);
                return Err(format!("surface texture: {other:?}"));
            }
        };
        // Waiting for a free swapchain image is the GPU's time, not the CPU's.
        let wait_ms = ms(t_out);
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let quality = self.quality.get_or_insert_with(|| crate::quality::AdaptiveQuality::new(self.game.target_fps(), requested_hz().map(f64::from), Instant::now()).unwrap_or_else(|| unreachable!()));
        let base = |game: Option<u32>| match (self.mobile, game) {
            (true, _) => self.render_height.unwrap_or(if self.upscale.is_some() { size[1] } else { MOBILE_BASE_HEIGHT }).min(game.filter(|h| *h > 0).unwrap_or(u32::MAX)).min(size[1]),
            (false, Some(0)) => size[1],
            (false, Some(h)) => h,
            (false, None) => size[1].min(1440),
        };
        let height = match self.render_height.filter(|_| !self.mobile) {
            Some(h) => Some(h),
            None => Some(quality.render_height(base(self.cfg.max_render_height))).filter(|h| *h < size[1]),
        };
        let benching = self.bench.as_mut().and_then(|b| b.height(size));
        let height = match benching { Some(h) => Some(h).filter(|h| *h < size[1]), None => height };
        live.out.draw(&live.gpu, renderer, &draw, size, split, &view, height)?;
        live.gpu.queue.present(texture);
        // GPU pass times (timestamps, when the GPU has them), with the frame stats.
        if !self.profiling { self.profiling = renderer.set_async_gpu_profiling(&live.gpu, true).is_ok(); }
        if let Some(sample) = renderer.drain_gpu_profiles(&live.gpu).samples.pop() { self.gpu_passes = sample.gpu_ms; self.gpu_total = sample.covered_union_ms; }
        self.frames += 1;
        if self.switched.is_none() && benching.is_none() && (self.render_height.is_none() || self.mobile) && quality.frame(f64::from(dt) * 1000.0, Instant::now()) {
            let next = quality.render_height(base(self.cfg.max_render_height));
            log(&format!("{}: 3D view now {} lines high (chasing {:.0} fps)", self.cfg.title, next.min(size[1]), quality.target_fps()));
        }
        if self.switched.is_none() && let Some(b) = &mut self.bench {
            let sample = crate::bench::Sample { frame_ms: dt * 1000.0, cpu_ms: tick_ms + draw_ms + (ms(t_out) - wait_ms).max(0.0), wait_ms, gpu_ms: (self.profiling && self.gpu_total > 0.0).then_some(self.gpu_total as f32) };
            let about = crate::bench::About { game: self.cfg.title.clone(), gpu: live.gpu.info.name.clone(), platform: "android", tier: query_param("tier").unwrap_or_else(|| if self.mobile { "medium" } else { "high" }.into()), players: self.game.local_players(), memory_mb: memory_mb() };
            if let Some(line) = b.frame(sample, size, &about) { log(&line); }
        }
        if self.switched.is_none() && stats(&mut self.times, &self.cfg.title, [dt * 1000.0, tick_ms, draw_ms, ms(t_out)]) {
            let mut passes = self.gpu_passes.clone();
            passes.sort_by(|a, b| b.1.total_cmp(&a.1));
            let top: Vec<String> = passes.iter().take(8).map(|(n, ms)| format!("{n} {ms:.1}")).collect();
            let lines = height.unwrap_or(size[1]);
            let upscale = match self.upscale.and_then(|m| m.portable_scale().map(|s| (m, s))) {
                Some((m, s)) => format!(" ({} from {} lines)", m.as_str(), (lines as f32 * s).round() as u32),
                None => String::new(),
            };
            log(&format!("{}: GPU {:.1} ms at {}x{} out, 3D {lines} lines{upscale}: {}", self.cfg.title, self.gpu_total, size[0], size[1], top.join(", ")));
        }
        if self.frames == 1 { log(&format!("first frame {:.1} s after start", self.started.elapsed().as_secs_f32())); QUIET.with(|q| q.set((q.get().0, q.get().0))); }
        if let Some(at) = self.switched.take() { log(&format!("{}: first frame {:.0} ms after the switch", self.cfg.title, at.elapsed().as_secs_f32() * 1000.0)); }
        Ok(())
    }
}

/// The brand's faces (lobbies/game-room/assets/fonts, as the browser
/// page and the desktop room use): Bungee for the wordmark, Outfit for text.
fn brand_fonts() -> (FontFace, FontFace, FontFace) {
    static FACES: OnceLock<(FontFace, FontFace, FontFace)> = OnceLock::new();
    *FACES.get_or_init(|| (
        render::hud::register_font(include_bytes!("../../../../lobbies/game-room/assets/fonts/bungee-regular.ttf")),
        render::hud::register_font(include_bytes!("../../../../lobbies/game-room/assets/fonts/outfit-800.ttf")),
        render::hud::register_font(include_bytes!("../../../../lobbies/game-room/assets/fonts/outfit-600.ttf")),
    ))
}

const AMBER: [f32; 4] = [1.0, 0.706, 0.329, 1.0];
const VIOLET: [f32; 4] = [0.655, 0.545, 0.98, 1.0];
const PANEL: [f32; 4] = [0.051, 0.063, 0.125, 0.88];

/// A rectangle with round corners of radius `r`.
fn round_rect(d: &mut HudDrawList, [x, y, w, h]: [f32; 4], r: f32, color: [f32; 4], clip: [f32; 4]) {
    let r = r.min(w * 0.5).min(h * 0.5);
    d.rect([x + r, y, w - 2.0 * r, h], color, clip);
    d.rect([x, y + r, r, h - 2.0 * r], color, clip);
    d.rect([x + w - r, y + r, r, h - 2.0 * r], color, clip);
    for c in [[x + r, y + r], [x + w - r, y + r], [x + r, y + h - r], [x + w - r, y + h - r]] { d.circle(c, r, color, clip); }
}

/// The browser page's controller icon (index.html #club-notice), `size`
/// square at `at`.
fn pad_icon(d: &mut HudDrawList, at: [f32; 2], size: f32, clip: [f32; 4]) {
    let u = size / 48.0;
    let p = |x: f32, y: f32| [at[0] + x * u, at[1] + y * u];
    let dark = [0.051, 0.063, 0.125, 1.0];
    round_rect(d, [at[0] + 4.0 * u, at[1] + 12.0 * u, 40.0 * u, 22.0 * u], 10.0 * u, AMBER, clip);
    d.circle(p(12.0, 30.0), 9.0 * u, AMBER, clip);
    d.circle(p(36.0, 30.0), 9.0 * u, AMBER, clip);
    round_rect(d, [at[0] + 9.0 * u, at[1] + 21.0 * u, 10.0 * u, 3.5 * u], 1.75 * u, dark, clip);
    round_rect(d, [at[0] + 12.25 * u, at[1] + 17.75 * u, 3.5 * u, 10.0 * u], 1.75 * u, dark, clip);
    d.circle(p(33.0, 19.5), 2.6 * u, dark, clip);
    d.circle(p(38.0, 24.5), 2.6 * u, dark, clip);
}

/// The [`banner`] line, top centre, on the browser page's dark rounded
/// panel; a notice with a second line gets the controller icon.
fn draw_banner(hud: &mut HudDrawList, size: [u32; 2]) {
    let line = BANNER.with(|b| {
        let mut b = b.borrow_mut();
        if b.as_ref().is_some_and(|(_, until)| until.is_some_and(|u| Instant::now() > u)) { *b = None; }
        b.as_ref().map(|(t, _)| t.clone())
    }).or_else(|| NOTICE.with(|n| n.borrow().clone()));
    let Some(line) = line else { return };
    let (_, bold, body) = brand_fonts();
    let (w, h) = (size[0] as f32, size[1] as f32);
    let s = h / 720.0;
    let clip = [0.0, 0.0, w, h];
    let (title, sub) = match line.split_once('\n') { Some((t, u)) => (t.to_string(), Some(u.to_string())), None => (line.clone(), None) };
    let (tf, sf) = (22.0 * s, 16.0 * s);
    let tw = render::hud::measure(&title, bold, tf)[0];
    let sw = sub.as_ref().map_or(0.0, |u| render::hud::measure(u, body, sf)[0]);
    let icon = if sub.is_some() { 44.0 * s } else { 0.0 };
    let gap = if sub.is_some() { 18.0 * s } else { 0.0 };
    let (px, py) = (24.0 * s, if sub.is_some() { 16.0 * s } else { 13.0 * s });
    let text_h = if sub.is_some() { tf * 1.3 + sf * 1.4 } else { tf * 1.3 };
    let pw = (icon + gap + tw.max(sw) + 2.0 * px).min(w - 32.0 * s);
    let ph = text_h.max(icon) + 2.0 * py;
    let (x, y) = ((w - pw) * 0.5, 28.0 * s);
    hud.begin_overlay();
    round_rect(hud, [x - 1.0, y - 1.0, pw + 2.0, ph + 2.0], 20.0 * s + 1.0, [1.0, 1.0, 1.0, 0.1], clip);
    round_rect(hud, [x, y, pw, ph], 20.0 * s, PANEL, clip);
    let tx = x + px + icon + gap;
    let tcw = pw - 2.0 * px - icon - gap;
    if sub.is_some() { pad_icon(hud, [x + px, y + (ph - icon) * 0.5], icon, clip); }
    let ty = y + (ph - text_h) * 0.5;
    let align = if sub.is_some() { TextAlign::Left } else { TextAlign::Center };
    hud.text.push(TextRun { id: 0xBA77E2, text: title, position: [tx, ty], bounds: clip, font_size: tf, color: [238, 240, 252, 255], font: bold, align, width: tcw });
    if let Some(u) = sub { hud.text.push(TextRun { id: 0xBA77E3, text: u, position: [tx, ty + tf * 1.3], bounds: clip, font_size: sf, color: [168, 173, 196, 255], font: body, align, width: tcw }); }
}

/// Every 300 frames, the frame times in logcat (returns true then).
fn stats(times: &mut Vec<[f32; 4]>, title: &str, sample: [f32; 4]) -> bool {
    times.push(sample);
    // Every 300 frames, or every 5 s when frames are slow.
    if times.len() < 300 && times.iter().map(|t| t[0]).sum::<f32>() < 5000.0 { return false; }
    let pick = |k: usize, q: f32| {
        let mut v: Vec<f32> = times.iter().map(|t| t[k]).collect();
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    };
    log(&format!("{}: frame every {:.1} ms (p95 {:.1}); ticks {:.1}, draw {:.1}, encode {:.1} ms (medians), {} MB",
        title, pick(0, 0.5), pick(0, 0.95), pick(1, 0.5), pick(2, 0.5), pick(3, 0.5), memory_mb()));
    times.clear();
    true
}
