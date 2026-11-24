//! The world as a kern `Sim`: stepping, the determinism digest, and queries
//! for the engine's tools (`kern::run`, traces, first divergence).

use crate::world::World;
use kern::{Digest, Sim, Timings};
use serde_json::{Value, json};

impl World {
    /// What a query sees: the run, every wizard and monster, the counts.
    pub fn snapshot(&self) -> Value {
        let actor = |id| {
            let a = self.a(id);
            json!({
                "pos": [a.pos.x, a.pos.y, a.pos.z],
                "hp": a.hp,
                "max_hp": a.max_hp,
                "alive": a.alive,
                "wet": a.wet,
                "burn": a.burn,
                "chill": a.chill,
                "frozen": a.frozen,
            })
        };
        let wizards: Vec<Value> = self
            .wizards
            .iter()
            .map(|&id| {
                let mut v = actor(id);
                let w = self.wiz(id);
                v["name"] = json!(w.name);
                v["mana"] = json!(w.mana);
                v["queue"] = json!(w.queue.slots.iter().map(|e| e.name()).collect::<Vec<_>>());
                v["relics"] = json!(w.relics);
                v["unlocks"] = json!(w.unlocks);
                v
            })
            .collect();
        let enemies: Vec<Value> = self
            .enemies
            .iter()
            .map(|&id| {
                let mut v = actor(id);
                v["kind"] = json!(self.enemy(id).cfg().key);
                v
            })
            .collect();
        json!({
            "tick": self.tick,
            "biome": self.run.biome_index,
            // Enough to rebuild the same level from an F4 report.
            "seed": self.seed,
            "level_seed": self.run.seed,
            "wards": self.run.wards.iter().map(|w| json!({
                "c": [w.c.x, w.c.y], "n": [w.n.x, w.n.y], "at": w.at, "lo": w.lo, "hi": w.hi, "holds": w.holds,
            })).collect::<Vec<_>>(),
            "state": format!("{:?}", self.run.state),
            "kills": self.run.kills,
            "lead": self.lead_progress(),
            "wizards": wizards,
            "enemies": enemies,
            "fx": self.fx.len(),
            "fields": self.fields.len(),
            "stats": self.stats,
        })
    }

    pub fn digest_into(&self, d: &mut Digest) {
        d.u64(self.tick);
        d.u64(self.run.biome_index as u64);
        d.u64(self.run.kills);
        d.u64(self.run.state as u64);
        for &id in self.wizards.iter().chain(&self.enemies) {
            let a = self.a(id);
            d.f32(a.pos.x);
            d.f32(a.pos.z);
            d.f64(a.hp);
            d.u64(a.alive as u64);
        }
        for &id in &self.wizards {
            d.f64(self.wiz(id).mana);
        }
        d.u64(self.fx.len() as u64);
        d.u64(self.fields.len() as u64);
        for (k, v) in &self.stats {
            d.bytes(k.as_bytes());
            d.u64(*v);
        }
    }
}

impl Sim for World {
    fn step(&mut self, timings: &mut Timings) {
        timings.span("world", || World::step(self));
    }

    fn tick(&self) -> u64 {
        self.tick
    }

    fn digest(&self, d: &mut Digest) {
        self.digest_into(d);
    }

    fn query(&self, path: &str) -> Option<Value> {
        let v = self.snapshot();
        let parts: Vec<&str> = path.split('.').collect();
        kern::walk(&v, &parts).cloned()
    }
}
