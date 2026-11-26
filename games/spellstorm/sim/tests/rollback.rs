//! Snapshots for rollback netcode: a world loaded back from a snapshot plays
//! on exactly as the world it was taken from (kern::rollback).

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

/// Loading a snapshot every frame changes nothing: storage order, handles
/// and random streams all come back as they were.
#[test]
fn loaded_every_frame_plays_the_same() {
    let mut a = bots(3, 7);
    let mut b = bots(3, 7);
    for i in 0..60 * 60 {
        let s = b.save_frame();
        b.load_frame(&s);
        a.step();
        b.step();
        if i % 30 == 0 { assert_eq!(digest(&a), digest(&b), "diverged at tick {i}"); }
    }
    assert_eq!(digest(&a), digest(&b));
}

/// Going back and playing the same frames again ends in the same state,
/// also across a level change.
#[test]
fn going_back_replays_exactly() {
    let mut w = bots(2, 42);
    for _ in 0..60 * 20 { w.step(); }
    let s = w.save_frame();
    let depth = w.run.depth();
    let mut ticks = 0;
    while w.run.depth() == depth && ticks < 60 * 240 { w.step(); ticks += 1; }
    for _ in 0..120 { w.step(); ticks += 1; }
    let want = digest(&w);
    let crossed = w.run.depth() != depth;
    w.load_frame(&s);
    for _ in 0..ticks { w.step(); }
    assert_eq!(digest(&w), want);
    eprintln!("replayed {ticks} ticks, crossed a level: {crossed}");
}

#[test]
fn going_back_a_little_replays_exactly() {
    let mut w = bots(2, 42);
    for _ in 0..60 * 20 { w.step(); }
    let s = w.save_frame();
    for _ in 0..60 { w.step(); }
    let want = digest(&w);
    w.load_frame(&s);
    for _ in 0..60 { w.step(); }
    assert_eq!(digest(&w), want);
}

/// What a snapshot costs: rollback takes one every frame.
#[test]
fn a_snapshot_is_cheap() {
    let mut w = bots(4, 9);
    for _ in 0..60 * 30 { w.step(); }
    let t = std::time::Instant::now();
    for _ in 0..100 { let s = w.save_frame(); std::hint::black_box(&s); }
    let save = t.elapsed() / 100;
    let s = w.save_frame();
    let t = std::time::Instant::now();
    for _ in 0..100 { w.load_frame(&s); }
    let load = t.elapsed() / 100;
    let t = std::time::Instant::now();
    for _ in 0..100 { w.step(); }
    let step = t.elapsed() / 100;
    eprintln!("{} entities: save {save:?}, load {load:?}, step {step:?}", w.ecs.len());
}

/// A rollback past a level change builds the old level again.
#[test]
fn going_back_past_a_new_level() {
    let mut w = bots(2, 5);
    for _ in 0..60 * 10 { w.step(); }
    let s = w.save_frame();
    let play = |w: &mut World| { for _ in 0..60 { w.step(); } w.run.biome_index += 1; w.enter_level(); for _ in 0..120 { w.step(); } };
    play(&mut w);
    let want = digest(&w);
    let built = w.level.built;
    w.load_frame(&s);
    assert_ne!(w.level.built, built, "the old level is back");
    play(&mut w);
    assert_eq!(digest(&w), want);
}
