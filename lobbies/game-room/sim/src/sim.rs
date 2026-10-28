//! The world as a kern `Sim`: the determinism digest and queries for the
//! engine's tools.

use crate::world::World;
use kern::{Digest, Sim, Timings};
use serde_json::{Value, json};

impl World {
    /// What a query sees: every player, item and shot.
    pub fn snapshot(&self) -> Value {
        let players: Vec<Value> = self
            .players
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "x": p.x, "y": p.y, "vx": p.vx, "vy": p.vy,
                    "facing": p.facing, "grounded": p.grounded, "wall": p.wall,
                    "item": p.item, "stun": p.stun, "sleeping": p.sleeping,
                    "carrying": p.carrying, "carried_by": p.carried_by, "flung": p.flung,
                })
            })
            .collect();
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|i| json!({"id": i.id, "kind": i.kind.name(), "x": i.x, "y": i.y, "holder": i.holder, "fuse": i.fuse, "ammo": i.ammo}))
            .collect();
        json!({"tick": self.ticks, "time": self.time, "players": players, "items": items, "shots": self.shots.len()})
    }
}

impl Sim for World {
    fn step(&mut self, timings: &mut Timings) {
        timings.span("world", || World::step(self));
    }

    fn tick(&self) -> u64 {
        self.ticks
    }

    fn digest(&self, d: &mut Digest) {
        d.u64(self.ticks);
        for p in &self.players {
            d.bytes(p.id.as_bytes());
            for v in [p.x, p.y, p.vx, p.vy, p.facing, p.stun, p.coyote, p.buffer, p.flung] {
                d.f64(v);
            }
            d.u64(p.item.map_or(0, |i| i as u64 + 1));
            d.bytes(&[p.grounded as u8, p.carrying.is_some() as u8, p.carried_by.is_some() as u8]);
        }
        for i in &self.items {
            d.u64(i.id as u64);
            for v in [i.x, i.y, i.vx, i.vy, i.fuse] {
                d.f64(v);
            }
        }
        for s in &self.shots {
            for v in [s.x, s.y, s.vx, s.vy, s.life] {
                d.f64(v);
            }
        }
    }

    fn query(&self, path: &str) -> Option<Value> {
        let mut v = self.snapshot();
        for part in path.split('.').filter(|p| !p.is_empty()) {
            v = match part.parse::<usize>() {
                Ok(n) => v.get(n)?.clone(),
                Err(_) => v.get(part)?.clone(),
            };
        }
        Some(v)
    }
}
