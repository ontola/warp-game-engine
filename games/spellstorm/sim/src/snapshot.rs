//! Snapshots of the whole world, for rollback netcode (kern::rollback):
//! save before a frame, and later load it back exactly, so that simulating
//! forward again gives bit for bit what a couch that never rolled back got.
//!
//! Two things make that exact. Entity handles come from the world itself
//! ([`Ids`]: a free list and a generation per slot, both in the snapshot),
//! not from hecs, whose allocator a copy cannot bring back. And the level,
//! which is large and fixed once built, is not copied: a snapshot keeps what
//! it was built from and its random stream, and loading builds it again only
//! when a rollback crosses a level change. Its props are the exception:
//! they break, so their state is in the snapshot.

use crate::actor::Actor;
use crate::enemy::Enemy;
use crate::ground::Field;
use crate::level::Level;
use crate::pickups::{Bullet, Globe, Soul};
use crate::rng::Pcg;
use crate::spells::{Beam, Bolt, Overload, Projectile, Sentry, Spray, Stream, Trap};
use crate::wizard::Wizard;
use crate::world::{Id, Order, World};

/// Entity handles: slot generations and the free slots, last freed first.
#[derive(Clone, Debug, Default)]
pub struct Ids {
    gens: Vec<u32>,
    free: Vec<u32>,
}

impl Ids {
    fn take(&mut self) -> Id {
        let i = self.free.pop().unwrap_or_else(|| { self.gens.push(1); self.gens.len() as u32 - 1 });
        Id::from_bits(((self.gens[i as usize] as u64) << 32) | i as u64).expect("a generation is never 0")
    }
    fn give(&mut self, id: Id) {
        let i = id.id() as usize;
        if let Some(g) = self.gens.get_mut(i) { *g = g.wrapping_add(1).max(1); self.free.push(i as u32); }
    }
}

impl World {
    /// Spawn an entity with a handle from [`Ids`].
    pub fn spawn(&mut self, components: impl hecs::DynamicBundle) -> Id {
        let id = self.ids.take();
        self.ecs.spawn_at(id, components);
        id
    }

    /// Despawn an entity; a handle already gone is ignored.
    pub fn despawn(&mut self, id: Id) {
        if self.ecs.despawn(id).is_ok() { self.ids.give(id); }
    }
}

/// Every component type an entity can have. A snapshot of an entity with
/// another one panics, so a new component cannot be forgotten silently.
macro_rules! components {
    ($($t:ty),* $(,)?) => {
        fn copy_ecs(from: &hecs::World) -> hecs::World {
            let mut to = hecs::World::new();
            for e in from.iter() {
                let mut b = hecs::EntityBuilder::new();
                let mut n = 0;
                $( if let Some(c) = e.get::<&$t>() { b.add((*c).clone()); n += 1; } )*
                assert_eq!(n, e.component_types().count(), "snapshot: an entity has a component snapshot.rs does not list");
                to.spawn_at(e.entity(), b.build());
            }
            to
        }
    };
}
components!(Order, Actor, Wizard, Enemy, Field, Globe, Bullet, Soul, Stream, Spray, Beam, Trap, Sentry, Projectile, Overload, Bolt);

/// The world at the start of a frame, without its level.
#[derive(Clone)]
pub struct Snapshot {
    ecs: std::sync::Arc<hecs::World>,
    level: (usize, u64),
    level_rng: Pcg,
    /// Each prop's hit points, whether it broke and its collision shape.
    props: Vec<(f32, bool, bool, Option<usize>)>,
    rest: Rest,
}

/// Everything else in the world, as is.
#[derive(Clone)]
struct Rest {
    run: crate::run::Run,
    settings: crate::world::Settings,
    rng: Pcg,
    tick: u64,
    time: f64,
    wizards: Vec<Id>,
    enemies: Vec<Id>,
    corpses: Vec<Id>,
    later: Vec<(f64, crate::world::Later)>,
    fields: Vec<Id>,
    fx: Vec<Id>,
    next_order: u64,
    stats: std::collections::BTreeMap<&'static str, u64>,
    grimoire: std::collections::BTreeMap<String, String>,
    confluences: std::collections::BTreeMap<String, String>,
    confluence_t: std::collections::BTreeMap<(u64, u64), f64>,
    dmg_t: f64,
    paused: bool,
    seed: u64,
    runs_ended: u32,
    ids: Ids,
}

impl World {
    /// The world as it is now (between frames, never during one).
    pub fn save_frame(&self) -> Snapshot {
        let w = self;
        Snapshot {
            ecs: std::sync::Arc::new(copy_ecs(&w.ecs)),
            level: w.level.built,
            level_rng: w.level.rng.clone(),
            props: w.level.props.iter().map(|p| (p.hp, p.broken, p.breakable, p.shape)).collect(),
            rest: Rest {
                run: w.run.clone(), settings: w.settings.clone(), rng: w.rng.clone(), tick: w.tick, time: w.time,
                wizards: w.wizards.clone(), enemies: w.enemies.clone(), corpses: w.corpses.clone(), later: w.later.clone(),
                fields: w.fields.clone(), fx: w.fx.clone(), next_order: w.next_order, stats: w.stats.clone(),
                grimoire: w.grimoire.clone(), confluences: w.confluences.clone(), confluence_t: w.confluence_t.clone(),
                dmg_t: w.dmg_t, paused: w.paused, seed: w.seed, runs_ended: w.runs_ended, ids: w.ids.clone(),
            },
        }
    }

    /// Back to a snapshot. Events of the frames undone are dropped.
    pub fn load_frame(&mut self, s: &Snapshot) {
        self.ecs = copy_ecs(&s.ecs);
        if self.level.built != s.level { self.level = Level::build(s.level.0, s.level.1); }
        self.level.rng = s.level_rng.clone();
        for (p, &(hp, broken, breakable, shape)) in self.level.props.iter_mut().zip(&s.props) {
            if p.shape.is_none() { if let Some(i) = shape { self.level.statics.put_back(i); } }
            p.hp = hp; p.broken = broken; p.breakable = breakable; p.shape = shape;
        }
        let r = s.rest.clone();
        self.run = r.run; self.settings = r.settings; self.rng = r.rng; self.tick = r.tick; self.time = r.time;
        self.wizards = r.wizards; self.enemies = r.enemies; self.corpses = r.corpses; self.later = r.later;
        self.fields = r.fields; self.fx = r.fx; self.next_order = r.next_order; self.stats = r.stats;
        self.grimoire = r.grimoire; self.confluences = r.confluences; self.confluence_t = r.confluence_t;
        self.dmg_t = r.dmg_t; self.paused = r.paused; self.seed = r.seed; self.runs_ended = r.runs_ended; self.ids = r.ids;
        self.events.clear();
    }
}
