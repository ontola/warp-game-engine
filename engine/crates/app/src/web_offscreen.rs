//! A game in a Web Worker, drawing into an `OffscreenCanvas` the page gave
//! it: the browser's version of the desktop host's separate game process.
//! The worker loads, builds and draws the game (its first frame included)
//! while the page's own thread keeps the room running; the page shows the
//! worker's canvas when it switches. The page reads the pads and keys (a
//! worker has no Gamepad API or key events) and sends each player's
//! [`Controls`] with [`set_controls`]; the worker posts `{type, ...}`
//! messages back (`first-frame`, `exit`). See engine/docs/browser.md.

use crate::{Controls, Frame, Game, Output};
use render::{Gpu, Renderer};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, closure::Closure};
use web_time::Instant;

thread_local! {
    /// Each local player's latest controls from the page; presses gather
    /// until a tick takes them, so a press between two ticks is never lost.
    static INPUT: RefCell<Vec<Controls>> = const { RefCell::new(Vec::new()) };
    /// A new canvas size from the page (CSS pixels times its scale).
    static RESIZE: RefCell<Option<[u32; 2]>> = const { RefCell::new(None) };
}

/// The page's controls for each local player, in seat order.
pub fn set_controls(players: Vec<Controls>) {
    INPUT.with(|i| {
        let mut i = i.borrow_mut();
        let old = std::mem::take(&mut *i);
        *i = players.into_iter().enumerate().map(|(n, mut c)| {
            if let Some(o) = old.get(n) { c.pressed.0 |= o.pressed.0; }
            c
        }).collect();
    });
}

/// The page resized the canvas the worker draws into.
pub fn resize(size: [u32; 2]) { RESIZE.with(|r| *r.borrow_mut() = Some(size)); }

fn take_controls(players: usize) -> Vec<Controls> {
    INPUT.with(|i| {
        let mut i = i.borrow_mut();
        let out: Vec<Controls> = (0..players).map(|n| i.get(n).copied().unwrap_or_default()).collect();
        for c in i.iter_mut() { c.pressed.0 = 0; }
        out
    })
}

/// Post `message` (a JSON object) to the page.
pub fn post(message: &serde_json::Value) {
    let Ok(value) = js_sys::JSON::parse(&message.to_string()) else { return };
    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let _ = scope.post_message(&value);
}

struct Live {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    out: Output,
    renderer: Option<Renderer>,
    game: Box<dyn Game>,
    tick: f32,
    acc: f32,
    last: Instant,
    started: Instant,
    frames: u64,
}

/// Run `game` in `canvas` until the worker ends. Returns once the GPU is
/// set up; the worker's animation frames drive the rest. Posts
/// `{type: "first-frame", ms}` when the first frame reached the GPU
/// (counted from `started`), and `{type: "exit", why}` when the game quits.
pub async fn run(canvas: web_sys::OffscreenCanvas, cfg: crate::Config, game: Box<dyn Game>, started: Instant) -> Result<(), String> {
    let instance = render::gpu::instance();
    let size = [canvas.width().max(1), canvas.height().max(1)];
    let surface = instance.create_surface(wgpu::SurfaceTarget::OffscreenCanvas(canvas)).map_err(|e| e.to_string())?;
    let (gpu, mut config, _adapter) = Gpu::for_surface_async(&instance, &surface, size[0], size[1]).await?;
    config.usage |= wgpu::TextureUsages::COPY_SRC;
    surface.configure(&gpu.device, &config);
    gpu.device.set_device_lost_callback(|why, msg| {
        super::web::log(&format!("worker GPU device lost: {why:?} {msg}"));
        post(&serde_json::json!({"type": "error", "error": format!("GPU device lost: {why:?} {msg}")}));
    });
    gpu.device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| super::web::log(&format!("worker GPU error: {e}"))));
    let out = Output::new(&gpu, config.format.add_srgb_suffix())?;
    let live = Rc::new(RefCell::new(Live { surface, config, gpu, out, renderer: None, game, tick: 1.0 / cfg.tick_hz, acc: 0.0, last: Instant::now(), started, frames: 0 }));
    schedule(live);
    Ok(())
}

/// Ask for the next animation frame (a worker has its own, in Chrome,
/// Firefox and Safari 16.4+), or a 16 ms timer where there is none.
fn schedule(live: Rc<RefCell<Live>>) {
    let scope = js_sys::global();
    let next = live.clone();
    let callback = Closure::once_into_js(move || {
        if let Err(e) = frame(&next) { super::web::log(&format!("worker frame: {e}")); }
        schedule(next);
    });
    let raf = js_sys::Reflect::get(&scope, &"requestAnimationFrame".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    match raf {
        Some(raf) => { let _ = raf.call1(&scope, &callback); }
        None => {
            let scope: web_sys::WorkerGlobalScope = scope.unchecked_into();
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), 16);
        }
    }
}

fn frame(live: &Rc<RefCell<Live>>) -> Result<(), String> {
    let mut guard = live.borrow_mut();
    let l = &mut *guard;
    if let Some(size) = RESIZE.with(|r| r.borrow_mut().take()) && size != [l.config.width, l.config.height] {
        l.config.width = size[0].max(1);
        l.config.height = size[1].max(1);
        l.surface.configure(&l.gpu.device, &l.config);
    }
    let now = Instant::now();
    let dt = (now - l.last).as_secs_f32().min(0.1);
    l.last = now;
    l.acc += dt;
    let mut steps = 0;
    while l.acc >= l.tick && steps < 6 {
        l.acc -= l.tick;
        steps += 1;
        let controls = take_controls(l.game.local_players());
        l.game.tick(&controls);
        if l.game.quit() { post(&serde_json::json!({"type": "exit", "why": "quit"})); }
        if l.game.take_back() { post(&serde_json::json!({"type": "exit", "why": "back"})); }
    }
    if steps == 6 { l.acc = 0.0; }
    let size = [l.config.width, l.config.height];
    let alpha = (l.acc / l.tick).clamp(0.0, 1.0);
    let split = l.game.split();
    let mut frame = Frame::new(&l.gpu, &mut l.renderer, size, dt, alpha);
    frame.split = split;
    let draw = l.game.draw(&mut frame)?;
    let Some(renderer) = l.renderer.as_ref() else { return Ok(()) };
    let texture = match l.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
        other => {
            l.surface.configure(&l.gpu.device, &l.config);
            return Err(format!("canvas texture: {other:?}"));
        }
    };
    let view = texture.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(l.config.format.add_srgb_suffix()), ..Default::default() });
    l.out.draw(&l.gpu, renderer, &draw, size, split, &view, None)?;
    l.gpu.queue.present(texture);
    l.frames += 1;
    if l.frames == 1 {
        let started = l.started;
        l.gpu.queue.on_submitted_work_done(move || {
            post(&serde_json::json!({"type": "first-frame", "ms": started.elapsed().as_secs_f32() * 1000.0}));
        });
    }
    Ok(())
}
