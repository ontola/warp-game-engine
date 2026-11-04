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
                    let at_wall = w.run.wards.iter().any(|wd| wd.k >= 0 && !wd.opening && (wd.past(glam::Vec2::new(a.pos.x, a.pos.z)).0 + 2.0).abs() < 0.5);
                    assert!(at_wall || at + 2.5 >= front, "seed {seed}: shrine {} altar at {at} m, behind the front at {front} m (clearings {:?})", seen / 4, w.level.clearing_at);
                    for wd in w.run.wards.iter().filter(|wd| wd.k == (seen / 4) as i32 && !wd.opening) {
                        let (d, within) = wd.past(glam::Vec2::new(a.pos.x, a.pos.z));
                        assert!(!within || d < -1.0 || d > 12.0, "seed {seed}: an altar at {:?}, {d} m past the ward", a.pos);
                    }
                }
                seen = w.run.altars.len();
            }
        }
        assert!(seen > 0, "seed {seed}: no shrine rose");
    }
}

#[test]
fn nobody_gets_past_a_shut_wall_and_unlocks_wait_for_it() {
    // Joep: monsters must never be behind a wall of mist, wizards wait at it
    // until it opens, and a shrine's new element comes as you walk through.
    for seed in [7, 11, 23] {
        let mut w = bots(3, seed);
        let mut known: Vec<i32> = w.wizards.iter().map(|&id| w.wiz(id).unlocks).collect();
        for _ in 0..60 * 150 {
            w.step();
            for wd in w.run.wards.iter().filter(|wd| !wd.opening) {
                for &id in w.wizards.iter().chain(&w.enemies) {
                    if !w.alive(id) {
                        continue;
                    }
                    let p = w.pos(id);
                    let q = glam::Vec2::new(p.x, p.z);
                    let (d, within) = wd.past(q);
                    let ahead = w.level.progress_of(q) - wd.at;
                    let behind = d > 0.3 && ((within && d < 4.0 && ahead.abs() < 12.0) || (ahead > 0.3 && ahead < 25.0));
                    assert!(!behind, "seed {seed} tick {}: {} {d} m past a shut wall (k {})", w.tick, if w.a(id).wizard { "a wizard" } else { "a monster" }, wd.k);
                }
            }
            for (i, &id) in w.wizards.iter().enumerate() {
                let u = w.wiz(id).unlocks;
                if u > known[i] {
                    known[i] = u;
                    for wd in w.run.wards.iter().filter(|wd| wd.k >= 0 && !wd.opening) {
                        let p = w.pos(id);
                        let q = glam::Vec2::new(p.x, p.z);
                        let (d, within) = wd.past(q);
                        let near = (w.level.progress_of(q) - wd.at).abs() < 8.0;
                        assert!(!(within && near && d > -6.0 && d < 0.0), "seed {seed} tick {}: unlocked to {u} while {d} m before shut wall {}", w.tick, wd.k);
                    }
                }
            }
        }
    }
}

#[test]
fn a_shrine_with_no_tome_left_still_opens_its_wall() {
    // Joep, level 4: both wizards carried all 32 tomes, so the shrine raised
    // none, said "take your tome" and its wall never opened.
    let mut w = bots(2, 7);
    let n = spellstorm::relics::all().len();
    for id in w.wizards.clone() {
        for r in 0..n {
            w.give_relic(id, r);
        }
    }
    let mut opened = false;
    for _ in 0..60 * 150 {
        w.step();
        if w.run.wards.iter().any(|wd| wd.k == 0 && wd.opening) {
            opened = true;
            break;
        }
    }
    assert!(w.run.altars.is_empty(), "no tome is left to raise");
    assert!(opened, "the first wall must open with no tome to take");
}
