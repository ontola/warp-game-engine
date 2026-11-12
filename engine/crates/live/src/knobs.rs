//! Knob blocks live in the script crate (`script::knobs`), so a game's sim
//! can read them without the live server; this re-export keeps the paths.
pub use script::knobs::*;
