//! Rust games built into the page's engine (not Luau scripts): each is a
//! crate with a `web::load` that fetches its assets and makes the game.
//! Add a game here with one arm, a wasm32 dev-dependency in
//! lobbies/game-room/Cargo.toml and an entry in build-web.py `RUST_GAMES`
//! (its assets and shelf entry).

use app::{Config, Game};

pub(crate) type Loaded = Result<(Config, Box<dyn Game>), String>;

/// Load built-in game `folder` for `players`, or None: not one of these
/// (a Luau game). The render hooks (game surface, glow, mask) are global;
/// a game that sets them does so while it loads, the others get them
/// cleared.
pub(crate) async fn load(folder: &str, players: usize) -> Option<Loaded> {
    let seed = getrandom::u64().unwrap_or(0x5eed) % 1_000_000_000;
    match folder {
        "spellstorm" => Some(spellstorm_app::web::load(players, seed).await),
        "spaceracer" => Some(spaceracer_app::web::load(players, seed).await),
        // Only in the page (a wasm32 dependency); not in the Android host.
        #[cfg(target_arch = "wasm32")]
        "growing-guns" => Some(growing_guns_app::web::load(players, seed).await),
        _ => {
            render::clear_game_hooks();
            None
        }
    }
}

/// The page's background worker (job-worker.js, app::web_jobs): a built-in
/// game's job by name, such as Growing Guns' lighting bakes.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn worker_job(name: &str, input: &[u8]) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    let out = if name.starts_with("growing-guns/") { growing_guns_app::web::job(name, input) } else { Err(format!("unknown job {name}")) };
    out.map_err(|e| wasm_bindgen::JsValue::from_str(&e))
}
