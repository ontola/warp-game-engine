//! A game in a Web Worker (app::web_offscreen) has no WebAudio: its mixer
//! renders here on a 20 ms timer, and each block goes to the page as
//! `{type: "pcm", data}` (interleaved stereo f32 at [`RATE`]), which plays
//! it (worker-host.js). While the game waits ([`set_waiting`]) it sends
//! nothing and the page runs dry: silence.
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::{JsCast, closure::Closure};

/// The page's worker sound plays at this rate.
pub const RATE: f32 = 48_000.0;

thread_local! {
    static WAITING: Cell<bool> = const { Cell::new(false) };
}

/// True in a worker (no `document`).
pub fn in_worker() -> bool { !js_sys::Reflect::has(&js_sys::global(), &"document".into()).unwrap_or(false) }

/// The game waits (warmed, or Back): no sound until it plays again.
pub fn set_waiting(waiting: bool) { WAITING.set(waiting); }

pub(crate) fn start(mut source: Box<dyn crate::Source>, stop: Arc<AtomicBool>) {
    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let mut last = js_sys::Date::now();
    let mut buf: Vec<f32> = Vec::new();
    let id = Cell::new(None::<i32>);
    let id = std::rc::Rc::new(id);
    let me = id.clone();
    let tick = Closure::<dyn FnMut()>::new(move || {
        if stop.load(Ordering::Relaxed) {
            if let Some(i) = me.take() { js_sys::global().unchecked_into::<web_sys::WorkerGlobalScope>().clear_interval_with_handle(i); }
            return;
        }
        let now = js_sys::Date::now();
        let frames = (((now - last) / 1000.0 * RATE as f64) as usize).min(RATE as usize / 10);
        if WAITING.get() || frames == 0 { if WAITING.get() { last = now; } return; }
        last += frames as f64 * 1000.0 / RATE as f64;
        buf.resize(frames * 2, 0.0);
        source.render(&mut buf, 2);
        let data = js_sys::Float32Array::from(&buf[..]);
        let msg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&msg, &"type".into(), &"pcm".into());
        let _ = js_sys::Reflect::set(&msg, &"data".into(), &data);
        let _ = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>().post_message_with_transfer(&msg, &js_sys::Array::of1(&data.buffer()));
    });
    if let Ok(i) = scope.set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), 20) { id.set(Some(i)); }
    tick.forget();
}
