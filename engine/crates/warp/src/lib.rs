//! The `warp` executable's library: run a game made only of Luau scripts,
//! synth patches and assets, with no Rust and no recompiling. See
//! engine/docs/warp-cli.md.
//!
//! A game folder holds `game.toml` (optional), `main.luau`, `sounds/*.luau`
//! patches and `assets/` (glTF). [`game::LuaGame`] is an `app::Game`, so it
//! runs in a window (`app::run`), headless in real time
//! (`app::run_headless`) or from a JSON script (`app::script`), and serves
//! live control over MCP through the `live` crate.
pub mod api;
pub mod config;
pub mod face;
pub mod game;
mod grass;
pub mod humans;
mod distant_humans;
pub mod kit;
mod editor_menus;
pub use live::knobs;
pub mod luajson;
pub mod online;
pub mod party;
pub mod plants;
pub mod ragdoll;
mod room_lighting;
mod room_scene;
pub mod samples;
pub mod sky;
pub mod sounds;
pub mod starter;
pub mod story;
pub mod surface;
pub mod world;
pub mod vehicles;

pub use game::{LuaGame, Overrides};

/// A phone or tablet host (`game.phone()` in Luau): the browser or Android
/// shell's phone preset (`?quality=full` turns it off). On a desktop,
/// `WARP_PHONE=1` pretends to be one, to try a game's phone tier.
pub fn phone() -> bool {
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    { app::web::mobile() }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    { std::env::var("WARP_PHONE").is_ok_and(|v| v == "1") }
}

pub mod storage;
/// Luau games in a browser (and the Android host, app::android): fetched
/// game folders, one sound output, the page's query.
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub mod web;
