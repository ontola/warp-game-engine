//! Spellstorm online: every couch runs the world itself, with rollback
//! (kern::rollback) on the inputs of the other couches' wizards.

use crate::snapshot::Snapshot;
use crate::wizard::Pad;
use crate::world::World;
use glam::Vec2;

/// One wizard's controller for one frame, as sent: sticks in 1/127ths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetPad {
    pub mv: [i8; 2],
    pub aim: [i8; 2],
    pub held: u32,
}

impl NetPad {
    pub const BYTES: usize = 8;

    pub fn from_pad(p: &Pad) -> NetPad {
        let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
        NetPad { mv: [q(p.mv.x), q(p.mv.y)], aim: [q(p.aim.x), q(p.aim.y)], held: p.held }
    }

    /// The pad a wizard reads (the same on every couch, bit for bit).
    pub fn pad(&self) -> Pad {
        let v = |a: [i8; 2]| Vec2::new(a[0] as f32 / 127.0, a[1] as f32 / 127.0);
        let aim = v(self.aim);
        Pad { mv: v(self.mv), aim, held: self.held, before: 0, fire: aim.length() > 0.5 }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend([self.mv[0] as u8, self.mv[1] as u8, self.aim[0] as u8, self.aim[1] as u8]);
        out.extend(self.held.to_le_bytes());
    }

    pub fn read(b: &[u8]) -> Option<NetPad> {
        let b: &[u8; 8] = b.get(..8)?.try_into().ok()?;
        Some(NetPad { mv: [b[0] as i8, b[1] as i8], aim: [b[2] as i8, b[3] as i8], held: u32::from_le_bytes([b[4], b[5], b[6], b[7]]) })
    }
}

/// Seat `i` is the run's wizard `i`.
impl kern::rollback::Game for World {
    type State = Snapshot;
    type Input = NetPad;
    fn save(&self) -> Snapshot { self.save_frame() }
    fn load(&mut self, s: &Snapshot) { self.load_frame(s) }
    fn step(&mut self, inputs: &[NetPad], resim: bool) {
        for (i, input) in inputs.iter().enumerate() {
            if let Some(&id) = self.wizards.get(i) { self.wiz_mut(id).input = input.pad(); }
        }
        self.paused = false;
        World::step(self);
        // A frame shown before: its sounds, effects and banners played then.
        if resim { self.events.clear(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_survives_the_wire() {
        let p = NetPad { mv: [-127, 64], aim: [3, -90], held: 0x8001 };
        let mut b = Vec::new();
        p.write(&mut b);
        assert_eq!(b.len(), NetPad::BYTES);
        assert_eq!(NetPad::read(&b), Some(p));
        assert_eq!(NetPad::from_pad(&p.pad()), p);
    }
}
