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

#[test]
fn shrines_rise_ahead_and_before_their_ward() {
    // Wherever the wizards stand when a shrine rises, its tomes are not
    // behind the furthest one (Joep: they ran back for them), and never
    // past a shut ward (they could not be reached).
    for seed in [7, 11, 23] {
        let mut w = bots(3, seed);
        let mut seen = 0;
        for _ in 0..60 * 150 {
            w.step();
            if w.run.altars.len() > seen {
                let p = |v: glam::Vec3| w.level.progress_of(glam::Vec2::new(v.x, v.z));
                let front = w.wizards.iter().filter(|&&id| w.alive(id)).map(|&id| p(w.pos(id))).fold(f32::MIN, f32::max);
                for a in &w.run.altars[seen..] {
                    let at = p(a.pos);
                    // Up against a shut wall is as far ahead as anything can be.
                    let at_wall = w.run.wards.iter().any(|wd| wd.k >= 0 && !wd.opening && (a.pos.z - wd.z - 2.0).abs() < 0.5);
                    assert!(at_wall || at + 2.5 >= front, "seed {seed}: shrine {} altar at {at} m, behind the front at {front} m (clearings {:?})", seen / 4, w.level.clearing_at);
                    // The way on runs to lower z; a shut wall is a line across it.
                    for wd in w.run.wards.iter().filter(|wd| wd.k >= 0 && !wd.opening) {
                        let near = (a.pos.z - wd.z).abs() < 12.0 && a.pos.x > wd.lo - 1.0 && a.pos.x < wd.hi + 1.0;
                        assert!(!near || a.pos.z > wd.z + 1.0, "seed {seed}: an altar at {:?}, past the ward at z {}", a.pos, wd.z);
                    }
                }
                seen = w.run.altars.len();
            }
        }
        assert!(seen > 0, "seed {seed}: no shrine rose");
    }
}
