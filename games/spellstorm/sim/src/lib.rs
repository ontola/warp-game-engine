//! Spellstorm on Warp: a co-op wizard roguelite. Ported from the Godot game
//! (joepio/spellstorm). The spell rules are plain data and functions; the
//! world runs on a small ECS (hecs) with a fixed system order, so a seed and
//! the same controls always play out the same run.

pub mod balance;
pub mod el;
pub mod grammar;
pub mod queue;
pub mod relics;

pub use el::El;
pub use grammar::{Form, Shape, Spell, compose};
