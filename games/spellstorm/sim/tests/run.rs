//! The whole game with bots at the controllers (Godot's `--sim` report):
//! every mechanic comes up within a few minutes, nothing panics, and the
//! same seed plays out the same run bit for bit.

use kern::{Digest, Sim};
use spellstorm::{Seat, Settings, World};

fn bots(n: usize, seed: u64) -> World {
    let seats: Vec<Seat> = (0..n).map(Seat::bot).collect();
    World::new(&seats, seed, Settings::default())
}

fn digest(w: &World) -> String {
    let mut d = Digest::default();
    w.digest(&mut d);
    d.hex16()
}

#[test]
fn bots_play_a_level() {
    let mut w = bots(3, 7);
    let mut far = 0.0f32;
    for _ in 0..60 * 150 {
        w.step();
        far = far.max(w.lead_progress());
    }
    let s = |k: &str| w.stat(k);
    eprintln!(
        "{:?} furthest {far:.0} m, now {:?} in biome {}",
        w.stats, w.run.state, w.run.biome_index
    );
    for k in [
        "shots",
        "casts",
        "kills",
        "souls",
        "bullets",
        "swings",
        "rolls",
        "props_broken",
        "globes",
        "deflects",
    ] {
        assert!(s(k) > 0, "no {k}: {:?}", w.stats);
    }
    // Past both shrines: the wards opened once everyone took a tome.
    assert!(far > w.level.clearing_at[1], "the bots got only {far} m in");
}

#[test]
fn same_seed_same_run() {
    let mut a = bots(2, 42);
    let mut b = bots(2, 42);
    for i in 0..60 * 40 {
        a.step();
        b.step();
        if i % 60 == 0 {
            assert_eq!(digest(&a), digest(&b), "diverged at tick {i}");
        }
    }
    assert_eq!(digest(&a), digest(&b));
    let mut c = bots(2, 43);
    for _ in 0..60 * 40 {
        c.step();
    }
    assert_ne!(digest(&a), digest(&c), "another seed played the same");
}

#[test]
fn every_biome_survives_bots() {
    for b in 0..spellstorm::biome::BIOMES.len() {
        let mut w = bots(2, 3 + b as u64);
        w.run.biome_index = b;
        w.enter_level();
        for _ in 0..60 * 30 {
            w.step();
        }
        assert!(w.stat("shots") > 0, "biome {b}: {:?}", w.stats);
    }
}

/// One new player with the shipped knobs.txt: only the walls past the two
/// clearings' fights, no practice strip walls every ten metres, and they
/// start with their first two elements.
#[test]
fn a_new_player_meets_only_the_clearing_walls() {
    let errors = spellstorm::knobs::load(include_str!("../../knobs.txt"));
    assert!(errors.is_empty(), "{errors:?}");
    let w = World::new(&[Seat::player(0, "Joep")], 7, Settings::default());
    assert!(w.run.practice.is_none());
    assert_eq!(w.run.wards.len(), 2, "{:?}", w.run.wards.iter().map(|x| x.z).collect::<Vec<_>>());
    assert!(w.run.wards.iter().all(|x| x.check.is_none()));
    spellstorm::knobs::reset();
}
