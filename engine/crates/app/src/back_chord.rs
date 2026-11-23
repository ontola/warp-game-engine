//! Back as the browser and Android hosts read it in a game: the room comes
//! on Back's *release*, so Back + RB (next game) and Back + LB (previous
//! game) go straight on without the room showing first. Released without a
//! chord, Back goes to the room. RB held when Back goes down skips too.
//! The desktop host (gamepad-party's daemon) does the same with its pads.

use crate::{Buttons, Controls, Device};

/// What Back asks the host for this step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackAction { Room, Next, Previous }

/// Back's state across steps. Feed every step's devices to [`step`](Self::step).
#[derive(Debug, Default)]
pub struct BackChords { held: bool, chorded: bool }

const CHORDS: u32 = Buttons::RIGHT_SHOULDER | Buttons::LEFT_SHOULDER;

impl BackChords {
    /// Read this step's devices and hide Back, and the chord buttons under a
    /// held Back, from the game. Returns what the host should do, if anything.
    pub fn step(&mut self, devices: &mut [(Device, Controls)]) -> Option<BackAction> {
        let mut action = None;
        let pressed = |b| devices.iter().any(|(_, c)| c.pressed.has(b));
        let down = devices.iter().any(|(_, c)| c.held.has(Buttons::SELECT));
        if pressed(Buttons::SELECT) && !self.held {
            self.held = true;
            self.chorded = false;
            if devices.iter().any(|(_, c)| c.held.has(Buttons::SELECT) && c.held.has(Buttons::RIGHT_SHOULDER)) {
                self.chorded = true;
                action = Some(BackAction::Next);
            }
        } else if self.held && down {
            if pressed(Buttons::RIGHT_SHOULDER) { self.chorded = true; action = Some(BackAction::Next); }
            else if pressed(Buttons::LEFT_SHOULDER) { self.chorded = true; action = Some(BackAction::Previous); }
        }
        if self.held && !down {
            self.held = false;
            if !self.chorded { action = Some(BackAction::Room); }
        }
        for (_, c) in devices.iter_mut() {
            let mask = if c.held.has(Buttons::SELECT) || c.pressed.has(Buttons::SELECT) { Buttons::SELECT | CHORDS } else { 0 };
            c.pressed.0 &= !mask;
            c.held.0 &= !mask;
            // The release of Back itself never reaches the game either.
            c.pressed.0 &= !Buttons::SELECT;
            c.held.0 &= !Buttons::SELECT;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(held: u32, before: u32) -> Vec<(Device, Controls)> {
        let mut c = Controls::default();
        c.held = Buttons(held);
        c.pressed = Buttons(held & !before);
        vec![(Device::Pad(0), c)]
    }

    #[test]
    fn back_goes_to_the_room_on_release_only() {
        let mut b = BackChords::default();
        assert_eq!(b.step(&mut pad(Buttons::SELECT, 0)), None);
        assert_eq!(b.step(&mut pad(Buttons::SELECT, Buttons::SELECT)), None);
        assert_eq!(b.step(&mut pad(0, Buttons::SELECT)), Some(BackAction::Room));
        assert_eq!(b.step(&mut pad(0, 0)), None);
    }

    #[test]
    fn a_chord_skips_and_its_release_does_nothing() {
        let mut b = BackChords::default();
        let s = Buttons::SELECT;
        let rb = Buttons::RIGHT_SHOULDER;
        b.step(&mut pad(s, 0));
        let mut d = pad(s | rb, s);
        assert_eq!(b.step(&mut d), Some(BackAction::Next));
        assert_eq!(d[0].1.held.0 & (s | rb), 0, "the game sees neither");
        assert_eq!(b.step(&mut pad(0, s | rb)), None, "no room after a chord");
        b.step(&mut pad(s, 0));
        assert_eq!(b.step(&mut pad(s | Buttons::LEFT_SHOULDER, s)), Some(BackAction::Previous));
        assert_eq!(b.step(&mut pad(0, s)), None);
    }

    #[test]
    fn rb_then_back_skips_at_once() {
        let mut b = BackChords::default();
        let (s, rb) = (Buttons::SELECT, Buttons::RIGHT_SHOULDER);
        let mut d = pad(rb, 0);
        assert_eq!(b.step(&mut d), None);
        assert_eq!(d[0].1.held.0, rb, "RB alone is the game's");
        assert_eq!(b.step(&mut pad(rb | s, rb)), Some(BackAction::Next));
        assert_eq!(b.step(&mut pad(0, rb | s)), None);
    }
}
