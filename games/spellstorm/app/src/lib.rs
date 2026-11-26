//! Spellstorm on Warp as a library: the `spellstorm` binary (main.rs) and
//! a browser host (`web::load`, wasm32) both start the game from here.
pub mod anim;
pub mod effects;
pub mod fx;
pub mod hud;
pub mod interp;
pub mod kits;
pub mod live;
pub mod look;
pub mod marks;
pub mod menu;
pub mod net;
pub mod party;
pub mod play;
pub mod ragdoll;
pub mod rig;
pub mod shrines;
pub mod skins;
pub mod view;
pub mod web;
