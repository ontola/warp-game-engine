//! A game in a Web Worker with its own canvas (`game-worker.js`,
//! app::web_offscreen): it loads and draws its first frame while the page
//! keeps the room running, as the desktop host warms the next game in its
//! own process. The page starts it with [`game_in_worker`] and sends the
//! players' controls with [`worker_controls`].

use app::{Buttons, Controls, Game};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

/// Load game `folder` for `players` in this worker and draw it into
/// `canvas`. Posts `{type: "loaded", ms}` once the game is built, then
/// `first-frame` (app::web_offscreen::run). `version` is the page's build.
#[wasm_bindgen]
pub async fn game_in_worker(canvas: web_sys::OffscreenCanvas, folder: String, players: u32, version: String) -> Result<(), JsValue> {
    if web_sys::window().is_some() { return Err("game_in_worker runs in a worker".into()); }
    std::panic::set_hook(Box::new(|info| {
        let text = format!("worker game panicked: {info}");
        web_sys::console::error_1(&text.clone().into());
        app::web_offscreen::post(&json!({"type": "error", "error": text}));
    }));
    script::web::init();
    app::web::set_version(&version);
    let started = web_time::Instant::now();
    let players = players.max(1) as usize;
    let loaded = match super::builtin::load(&folder, players).await {
        Some(l) => l,
        None => warp::web::load(&folder, Some(players)).await.map(|(c, g)| (c, Box::new(g) as Box<dyn Game>)),
    };
    let (cfg, game) = loaded.map_err(|e| JsValue::from_str(&format!("{folder}: {e}")))?;
    app::web_offscreen::post(&json!({"type": "loaded", "ms": started.elapsed().as_secs_f32() * 1000.0}));
    app::web_offscreen::run(canvas, cfg, game, started).await.map_err(|e| JsValue::from_str(&e))
}

/// Each local player's controls, from the page: a JSON array of
/// `{mv:[x,y], look:[x,y], aim:[x,y], triggers:[l,r], held, pressed}`.
#[wasm_bindgen]
pub fn worker_controls(json: &str) {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(json) else { return };
    let pair = |v: &Value| [v[0].as_f64().unwrap_or(0.0) as f32, v[1].as_f64().unwrap_or(0.0) as f32];
    let players = list.iter().map(|c| Controls {
        mv: pair(&c["mv"]), look: pair(&c["look"]), aim: pair(&c["aim"]), triggers: pair(&c["triggers"]),
        held: Buttons(c["held"].as_u64().unwrap_or(0) as u32), pressed: Buttons(c["pressed"].as_u64().unwrap_or(0) as u32),
    }).collect();
    app::web_offscreen::set_controls(players);
}

/// The page resized the worker's canvas.
#[wasm_bindgen]
pub fn worker_resize(width: u32, height: u32) { app::web_offscreen::resize([width, height]); }

// The page's side: the next game warmed in a worker (worker-host.js).

#[wasm_bindgen(module = "/examples/browser/worker-host.js")]
extern "C" {
    #[wasm_bindgen(js_name = spawnGame)]
    fn spawn_game(folder: &str, players: u32) -> u32;
    #[wasm_bindgen(js_name = gameState)]
    fn game_state(id: u32) -> String;
    #[wasm_bindgen(js_name = takeExit)]
    fn take_exit(id: u32) -> String;
    #[wasm_bindgen(js_name = gameControls)]
    fn game_controls(id: u32, json: &str);
    #[wasm_bindgen(js_name = disposeGame)]
    fn dispose_game(id: u32);
}

struct Warm { folder: String, players: usize, id: u32 }

thread_local! {
    static WARM: std::cell::RefCell<Option<Warm>> = const { std::cell::RefCell::new(None) };
}

/// Warming the next game in a worker: `?warm=worker` on a computer for
/// now (no sound in the worker yet, and not opened to other couches).
pub(crate) fn enabled() -> bool { !app::web::mobile() && app::web::query_param("warm").as_deref() == Some("worker") }

/// Build `folder` for `players` in a worker, unless it already is; any
/// other warmed game goes.
pub(crate) fn warm(folder: &str, players: usize) {
    if !enabled() { return; }
    WARM.with(|w| {
        let mut w = w.borrow_mut();
        if w.as_ref().is_some_and(|w| w.folder == folder && w.players == players) { return; }
        if let Some(old) = w.take() { dispose_game(old.id); }
        app::web::log(&format!("{folder}: warming in a worker for {players}"));
        *w = Some(Warm { folder: folder.to_string(), players, id: spawn_game(folder, players as u32) });
    });
}

/// The warmed game, if it is `folder` for `players` and drew its first
/// frame; any other warmed game goes.
pub(crate) fn take(folder: &str, players: usize, title: &str) -> Option<(app::Config, Box<dyn Game>)> {
    let warm = WARM.with(|w| w.borrow_mut().take())?;
    let state = game_state(warm.id);
    if warm.folder != folder || warm.players != players || state != "ready" {
        app::web::log(&format!("{}: warmed worker not used ({state}, for {} players)", warm.folder, warm.players));
        dispose_game(warm.id);
        return None;
    }
    let cfg = app::Config { title: title.to_string(), keyboard: true, pointer: false, ..Default::default() };
    Some((cfg, Box::new(WorkerGame { id: warm.id, players, back: false, quit: false })))
}

/// The game in a worker, as the room's shell sees it: it hands over the
/// players' controls and draws nothing (the worker's canvas is over it).
struct WorkerGame { id: u32, players: usize, back: bool, quit: bool }

impl Game for WorkerGame {
    fn local_players(&self) -> usize { self.players }
    fn tick(&mut self, controls: &[Controls]) {
        let list: Vec<Value> = controls.iter().map(|c| json!({
            "mv": c.mv, "look": c.look, "aim": c.aim, "triggers": c.triggers, "held": c.held.0, "pressed": c.pressed.0,
        })).collect();
        game_controls(self.id, &Value::Array(list).to_string());
        match take_exit(self.id).as_str() {
            "back" => self.back = true,
            "quit" => self.quit = true,
            _ => {}
        }
        let state = game_state(self.id);
        if let Some(e) = state.strip_prefix("error: ") {
            app::web::log(&format!("worker game: {e}"));
            self.quit = true;
        }
    }
    fn mouse_capture(&self) -> Option<bool> { Some(false) }
    fn draw(&mut self, frame: &mut app::Frame) -> Result<app::Draw, String> {
        Ok(app::Draw { cameras: Vec::new(), light: Default::default(), hud: render::hud::HudDrawList::new(frame.size) })
    }
    fn take_back(&mut self) -> bool { std::mem::take(&mut self.back) }
    fn quit(&self) -> bool { self.quit }
}

impl Drop for WorkerGame {
    fn drop(&mut self) { dispose_game(self.id); }
}
