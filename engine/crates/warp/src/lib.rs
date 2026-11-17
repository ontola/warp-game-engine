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
pub mod game;
pub mod humans;
mod editor_menus;
pub use live::knobs;
pub mod luajson;
pub mod party;
pub mod plants;
pub mod ragdoll;
pub mod sky;
pub mod sounds;
pub mod starter;
pub mod surface;
pub mod world;

pub use game::{LuaGame, Overrides};

pub mod storage;
