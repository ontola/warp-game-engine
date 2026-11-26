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
