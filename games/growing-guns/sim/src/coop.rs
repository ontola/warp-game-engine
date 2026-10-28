//! Wave survival (Godot coop): humans against waves of bot archetypes.
//! Ported in a later step; versus is unaffected.
use crate::World;

impl World {
    pub(crate) fn coop_kill(&mut self, _victim: usize) {}
    pub(crate) fn coop_next_wave(&mut self) {}
    pub(crate) fn coop_tick(&mut self) {}
}
