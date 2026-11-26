//! Spellstorm on Warp: a co-op wizard roguelite. Ported from the Godot game
//! (joepio/spellstorm). The spell rules are plain data and functions; the
//! world runs on a small ECS (hecs) with a fixed system order, so a seed and
//! the same controls always play out the same run.

pub mod actor;
pub mod balance;
pub mod biome;
pub mod bot;
pub mod collide;
pub mod el;
pub mod enemy;
pub mod fx;
pub mod grammar;
pub mod knobs;
pub mod ground;
pub mod level;
pub mod pickups;
pub mod queue;
pub mod relics;
pub mod rng;
pub mod rules;
pub mod run;
pub mod sim;
pub mod snapshot;
pub mod spells;
pub mod wizard;
pub mod world;

pub use el::El;
pub use grammar::{Form, Shape, Spell, compose};
pub use world::{Seat, Settings, World};
