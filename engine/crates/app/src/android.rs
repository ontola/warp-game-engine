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
//! `assets/web/` ([`fetch`]); web APIs go over HTTPS on a helper thread
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

/// A short line over the picture, at the top: for `seconds` (None: until
/// replaced or [`clear_banner`]). The browser page shows these as HTML.
pub fn banner(text: &str, seconds: Option<f32>) {
    let until = seconds.map(|s| Instant::now() + Duration::from_secs_f32(s));
    BANNER.with(|b| *b.borrow_mut() = Some((text.to_string(), until)));
}

/// Take the line [`banner`] shows away, when it is `text` (None: any).
pub fn clear_banner(text: Option<&str>) {
    BANNER.with(|b| { let mut b = b.borrow_mut(); if text.is_none() || b.as_ref().is_some_and(|(t, _)| Some(t.as_str()) == text) { *b = None; } });
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
    asset(url)
}

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

const MOBILE_BASE_HEIGHT: u32 = 540;

fn mobile_light(mut light: render::Lighting) -> render::Lighting {
    light.gi = 0.0;
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
pub enum Exit { Quit, Back }

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
    if mobile { log("phone preset: lower resolution, no GI, bloom or contact occlusion (quality=full turns it off)"); }
    let shell = Shell { cfg, game, devices, acc: 0.0, last: Instant::now(), frames: 0, started: Instant::now(), announced: false, switched: None, exited: false,
        times: Vec::new(), render_height: query_param("render").and_then(|h| h.parse().ok()), quality: None, mobile, profiling: false, gpu_passes: Vec::new(), gpu_total: 0.0 };
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

/// The first window: the GPU, its surface and the presenter.
fn on_window(window: NativeWindow) -> Result<(), String> {
    let size = window_size(&window);
    let instance = render::gpu::instance();
    let surface = surface_for(&instance, &window)?;
    let (gpu, config) = Gpu::for_surface(&instance, &surface, size[0], size[1])?;
    surface.configure(&gpu.device, &config);
    log(&format!("GPU: {} ({:?}), {}x{}, {:?}", gpu.info.name, gpu.info.backend, config.width, config.height, config.format));
    let limits = gpu.device.limits();
    log(&format!("{} storage buffers, {} sampled textures per shader stage", limits.max_storage_buffers_per_shader_stage, limits.max_sampled_textures_per_shader_stage));
    gpu.device.on_uncaptured_error(Arc::new(|e: wgpu::Error| log(&format!("GPU error: {e}"))));
    gpu.device.set_device_lost_callback(|reason, message| log(&format!("GPU device lost ({reason:?}): {message}")));
    let out = Output::new(&gpu, config.format)?;
    let hud = HudRenderer::new(&gpu, config.format)?;
    GFX.with(|g| *g.borrow_mut() = Some(Gfx { instance, gpu, out, surface: Some((surface, config, window)), renderer: None, hud, visible: true }));
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
                Event::Touch(id, phase, pos) => {
                    let size = GFX.with(|g| g.borrow().as_ref().and_then(|g| g.surface.as_ref().map(|s| [s.1.width, s.1.height]))).unwrap_or([1, 1]);
                    SHELL.with(|s| if let Some(s) = s.borrow_mut().as_mut() { s.devices.touch(id, phase, pos, size); s.devices.cursor(Some(pos)); });
                }
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

/// The loading screen: the name, a bar and the step, on the brand's dark blue.
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
        text(&mut d, 1, "SPLITSCREEN CLUB", 0.0, size[1] * 0.36, 64.0 * s, [255, 255, 255, 255], size[0], FontFace::Title, clip);
        let (w, h) = (size[0] * 0.4, 8.0 * s);
        let (x, y) = ((size[0] - w) * 0.5, size[1] * 0.56);
        d.rect([x, y, w, h], [1.0, 1.0, 1.0, 0.15], clip);
        d.rect([x, y, w * fraction.unwrap_or(1.0).clamp(0.0, 1.0), h], [0.22, 0.84, 1.0, 1.0], clip);
        text(&mut d, 2, &step, 0.0, y + 24.0 * s, 26.0 * s, [210, 215, 230, 255], size[0], FontFace::Body, clip);
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
    times: Vec<[f32; 4]>,
    render_height: Option<u32>,
    quality: Option<crate::quality::AdaptiveQuality>,
    mobile: bool,
    /// GPU timestamps are on, and the last frame's pass times.
    profiling: bool,
    gpu_passes: Vec<(&'static str, f64)>,
    gpu_total: f64,
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
            self.acc = 0.0;
            self.switched = Some(Instant::now());
            self.times.clear();
            self.profiling = false;
            if let Some(q) = &mut self.quality { q.set_enabled(false); q.set_enabled(true); q.idle(); q.set_target_fps(self.game.target_fps(), None); }
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
            if !self.exited && devices.iter().any(|(_, c)| c.pressed.has(crate::Buttons::SELECT)) && exit(Exit::Back) {
                self.exited = true;
                for (_, c) in &mut devices { c.pressed.0 &= !crate::Buttons::SELECT; c.held.0 &= !crate::Buttons::SELECT; }
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
        if let Some((from, knob, r)) = self.devices.touch_stick(size) {
            let clip = [0.0, 0.0, size[0] as f32, size[1] as f32];
            draw.hud.circle(from, r, [1.0, 1.0, 1.0, 0.18], clip);
            draw.hud.circle(knob, r * 0.42, [1.0, 1.0, 1.0, 0.5], clip);
        }
        draw_banner(&mut draw.hud, size);
        let Some(renderer) = live.renderer.as_ref() else { return Ok(()) };
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
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let quality = self.quality.get_or_insert_with(|| crate::quality::AdaptiveQuality::new(self.game.target_fps(), None, Instant::now()).unwrap_or_else(|| unreachable!()));
        let base = |game: Option<u32>| match (self.mobile, game) {
            (true, _) => self.render_height.unwrap_or(MOBILE_BASE_HEIGHT).min(game.filter(|h| *h > 0).unwrap_or(u32::MAX)).min(size[1]),
            (false, Some(0)) => size[1],
            (false, Some(h)) => h,
            (false, None) => size[1].min(1440),
        };
        let height = match self.render_height.filter(|_| !self.mobile) {
            Some(h) => Some(h),
            None => Some(quality.render_height(base(self.cfg.max_render_height))).filter(|h| *h < size[1]),
        };
        live.out.draw(&live.gpu, renderer, &draw, size, split, &view, height)?;
        live.gpu.queue.present(texture);
        // GPU pass times (timestamps, when the GPU has them), with the frame stats.
        if !self.profiling { self.profiling = renderer.set_async_gpu_profiling(&live.gpu, true).is_ok(); }
        if let Some(sample) = renderer.drain_gpu_profiles(&live.gpu).samples.pop() { self.gpu_passes = sample.gpu_ms; self.gpu_total = sample.covered_union_ms; }
        self.frames += 1;
        if self.switched.is_none() && (self.render_height.is_none() || self.mobile) && quality.frame(f64::from(dt) * 1000.0, Instant::now()) {
            let next = quality.render_height(base(self.cfg.max_render_height));
            log(&format!("{}: 3D view now {} lines high (chasing {:.0} fps)", self.cfg.title, next.min(size[1]), quality.target_fps()));
        }
        if self.switched.is_none() && stats(&mut self.times, &self.cfg.title, [dt * 1000.0, tick_ms, draw_ms, ms(t_out)]) {
            let mut passes = self.gpu_passes.clone();
            passes.sort_by(|a, b| b.1.total_cmp(&a.1));
            let top: Vec<String> = passes.iter().take(8).map(|(n, ms)| format!("{n} {ms:.1}")).collect();
            log(&format!("{}: GPU {:.1} ms at {}x{} out, 3D {} lines: {}", self.cfg.title, self.gpu_total, size[0], size[1], height.unwrap_or(size[1]), top.join(", ")));
        }
        if self.frames == 1 { log(&format!("first frame {:.1} s after start", self.started.elapsed().as_secs_f32())); QUIET.with(|q| q.set((q.get().0, q.get().0))); }
        if let Some(at) = self.switched.take() { log(&format!("{}: first frame {:.0} ms after the switch", self.cfg.title, at.elapsed().as_secs_f32() * 1000.0)); }
        Ok(())
    }
}

/// The [`banner`] line, top centre, on a dark pill.
fn draw_banner(hud: &mut HudDrawList, size: [u32; 2]) {
    let Some(line) = BANNER.with(|b| {
        let mut b = b.borrow_mut();
        if b.as_ref().is_some_and(|(_, until)| until.is_some_and(|u| Instant::now() > u)) { *b = None; }
        b.as_ref().map(|(t, _)| t.clone())
    }) else { return };
    let (w, h) = (size[0] as f32, size[1] as f32);
    let s = h / 720.0;
    let font = 28.0 * s;
    let tw = render::hud::measure(&line, FontFace::Body, font)[0];
    let (pw, ph) = (tw + 48.0 * s, font * 1.8);
    let clip = [0.0, 0.0, w, h];
    hud.begin_overlay();
    hud.rect([(w - pw) * 0.5, 24.0 * s, pw, ph], [0.02, 0.03, 0.06, 0.85], clip);
    text(hud, 0xBA77E2, &line, 0.0, 24.0 * s + (ph - font) * 0.45, font, [255, 255, 255, 255], w, FontFace::Body, clip);
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
