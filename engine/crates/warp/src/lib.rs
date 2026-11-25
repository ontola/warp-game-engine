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
#[cfg(target_arch = "wasm32")]
pub mod online_web;
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

/// How much larger HUDs draw on this device: 700 dp over the screen's
/// short side, 1 to 1.7 (the Game Room's rule, `ui_for_dp`): a phone at
/// ~400 dp gets 1.7, a tablet ~1.2, a TV or monitor 1 (Joep, 25 November:
/// "the GUI elements are pretty small"). `WARP_PHONE=1` on a desktop
/// pretends 1.4; `WARP_UI_SCALE` overrides (0.5 to 3). Luau reads it
/// through `hud.unit()`.
pub fn ui_scale() -> f32 {
    if let Some(v) = std::env::var("WARP_UI_SCALE").ok().and_then(|v| v.parse::<f32>().ok()) { return v.clamp(0.5, 3.0); }
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    { if let Some(dp) = app::web::short_side_dp() { return (700.0 / dp.max(1.0)).clamp(1.0, 1.7); } }
    if phone() { 1.4 } else { 1.0 }
}

pub mod storage;
/// Luau games in a browser (and the Android host, app::android): fetched
/// game folders, one sound output, the page's query.
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub mod web;
