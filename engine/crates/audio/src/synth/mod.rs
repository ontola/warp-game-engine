//! Procedural sound for games: units (oscillators, noise, envelopes,
//! filters, delays, resonators, strings...), patches that wire them as
//! data, voices compiled from patches, and a mixer that places voices in
//! any speaker layout (mono to 7.1) for one or more listeners, with a
//! reverb bus as wide as the layout and a limiter.
//!
//! Everything renders in blocks of [`BLOCK`] samples, deterministically
//! from a seed, offline as well as live.
pub mod expr;
#[cfg(feature = "luau")]
pub mod lua;
pub mod mixer;
pub mod patch;
pub mod reverb;
pub mod spatial;
pub mod units;

pub use mixer::{mixer, Compressor, Cull, Hall, Handle, Master, Mixer, Play, SoundSender};
pub use patch::{In, Node, Patch, Voice};
pub use spatial::{Ear, Falloff, Layout, Place, Speaker};
pub use units::{FilterMode, Mode, NoiseColor, Point, Shape, Unit};

/// Samples per block.
pub const BLOCK: usize = 64;

#[cfg(test)]
mod tests;
