//! The Game Room lobby's simulation: the room layout and the platformer
//! where everyone in the party runs, jumps, carries and throws things while
//! they pick a game. Ported from the Godot lobby (layout.gd, world.gd).

pub mod layout;
mod sim;
pub mod world;

pub use world::{Event, Ev, Item, Kind, Player, Shot, World};
