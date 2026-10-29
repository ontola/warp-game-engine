//! The sim ticks at 60 Hz; a 120 Hz screen shows each tick twice, so
//! anything moving judders into a double, blurred image. Before each tick
//! the moving things' places are kept, and each frame is drawn with them
//! eased between the last two ticks (the shell's `alpha`), then put back.
use glam::Vec3;
use hecs::Entity;
use spellstorm::actor::Actor;
use spellstorm::pickups::{Bullet, Globe, Soul};
use spellstorm::spells::{Bolt, Projectile};
use spellstorm::World;
use std::collections::HashMap;

/// Further than this in one tick is a jump (a new level, a revive), not
/// motion: it is not eased.
const JUMP: f32 = 3.0;

#[derive(Default)]
pub struct Interp {
    before: HashMap<Entity, (Vec3, f32)>,
    /// The places as ticked, while a frame is drawn eased.
    held: Vec<(Entity, Vec3, f32)>,
}

macro_rules! each_pos {
    ($w:expr, |$id:ident, $pos:ident, $yaw:ident| $body:block) => {{
        for ($id, a) in $w.ecs.query_mut::<&mut Actor>() {
            let $pos = &mut a.pos;
            let $yaw = Some(&mut a.yaw);
            $body
        }
        each_pos!(@plain $w, $id, $pos, $yaw, $body, Bolt, Projectile, Bullet, Globe, Soul);
    }};
    (@plain $w:expr, $id:ident, $pos:ident, $yaw:ident, $body:block, $($t:ty),*) => {
        $(for ($id, x) in $w.ecs.query_mut::<&mut $t>() {
            let $pos = &mut x.pos;
            let $yaw: Option<&mut f32> = None;
            $body
        })*
    };
}

impl Interp {
    /// Just before a tick: where everything stands now.
    pub fn keep(&mut self, w: &mut World) {
        self.before.clear();
        let before = &mut self.before;
        each_pos!(w, |id, pos, yaw| {
            before.insert(id, (*pos, yaw.map_or(0.0, |y| *y)));
        });
    }

    /// Ease everything `alpha` of the way from the tick before to the last.
    pub fn ease(&mut self, w: &mut World, alpha: f32) {
        self.held.clear();
        let (before, held) = (&self.before, &mut self.held);
        each_pos!(w, |id, pos, yaw| {
            let Some(&(p0, y0)) = before.get(&id) else { continue };
            if p0.distance(*pos) > JUMP {
                continue;
            }
            let y1 = yaw.as_ref().map_or(0.0, |y| **y);
            held.push((id, *pos, y1));
            *pos = p0.lerp(*pos, alpha);
            if let Some(y) = yaw {
                let d = (y1 - y0 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                *y = y1 - d * (1.0 - alpha);
            }
        });
    }

    /// Back to the places as ticked.
    pub fn restore(&mut self, w: &mut World) {
        let held = std::mem::take(&mut self.held);
        let map: HashMap<Entity, (Vec3, f32)> = held.iter().map(|&(e, p, y)| (e, (p, y))).collect();
        each_pos!(w, |id, pos, yaw| {
            if let Some(&(p, y1)) = map.get(&id) {
                *pos = p;
                if let Some(y) = yaw {
                    *y = y1;
                }
            }
        });
        self.held = held;
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spellstorm::{Seat, Settings};

    #[test]
    fn a_frame_between_ticks_draws_the_wizards_between_and_puts_them_back() {
        let mut w = World::new(&[Seat::bot(0)], 7, Settings::default());
        let id = w.wizards[0];
        let mut i = Interp::default();
        let p0 = w.pos(id);
        i.keep(&mut w);
        w.ecs.get::<&mut Actor>(id).unwrap().pos = p0 + Vec3::X;
        i.ease(&mut w, 0.25);
        assert!((w.pos(id) - (p0 + Vec3::X * 0.25)).length() < 1e-5);
        i.restore(&mut w);
        assert!((w.pos(id) - (p0 + Vec3::X)).length() < 1e-5);
        // A jump is drawn where it landed.
        i.keep(&mut w);
        w.ecs.get::<&mut Actor>(id).unwrap().pos = p0 + Vec3::X * 20.0;
        i.ease(&mut w, 0.5);
        assert!((w.pos(id) - (p0 + Vec3::X * 20.0)).length() < 1e-5);
    }
}
