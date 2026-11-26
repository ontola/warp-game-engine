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

/// Two couches, one wizard each, inputs arriving 5 frames late: both end in
/// the state the same inputs give without any network.
#[test]
fn two_couches_with_rollback_agree() {
    use kern::rollback::{Game as _, InputRun, Outbox, Rollback};
    use spellstorm::net::NetPad;
    let seats: Vec<Seat> = (0..2).map(|i| Seat::player(i, "W")).collect();
    let pad = |seat: usize, f: u32| {
        let t = f as f32 / 40.0 + seat as f32;
        let held = if (f / 25 + seat as u32) % 3 == 0 { 1 << spellstorm::wizard::Pad::RT } else { 0 } | if f % 90 == 10 { 1 } else { 0 };
        NetPad { mv: [(t.sin() * 120.0) as i8, (t.cos() * 120.0) as i8], aim: [(t * 1.3).cos().mul_add(127.0, 0.0) as i8, 90], held }
    };
    let n = 60 * 20;
    let mut reference = World::new(&seats, 3, Settings::default());
    for f in 0..n { let i = [pad(0, f), pad(1, f)]; kern::rollback::Game::step(&mut reference, &i, false); }
    let mut worlds = [World::new(&seats, 3, Settings::default()), World::new(&seats, 3, Settings::default())];
    let mut rbs: [Rollback<World>; 2] = [Rollback::new(2, 0), Rollback::new(2, 0)];
    let mut out = [Outbox::<NetPad>::new(0, 0), Outbox::new(1, 0)];
    let mut wire: Vec<(u32, usize, InputRun<NetPad>)> = Vec::new();
    for t in 0..n + 30 {
        for c in 0..2 {
            let f = out[c].next();
            if f < n && f <= rbs[c].frame() { out[c].push(pad(c, f)); rbs[c].input(c, f, pad(c, f)); }
            wire.push((t, c, out[c].run()));
            for (sent, from, run) in &wire { if *from != c && sent + 5 == t { rbs[c].run(run); } }
            let o = 1 - c;
            out[o].ack(rbs[c].needs(o));
            if rbs[c].frame() < n { rbs[c].advance(&mut worlds[c]); } else { rbs[c].correct(&mut worlds[c]); }
        }
    }
    eprintln!("resimulated {} and {} frames", rbs[0].resimulated, rbs[1].resimulated);
    assert!(rbs[0].resimulated > 0);
    assert_eq!(digest(&worlds[0]), digest(&reference));
    assert_eq!(digest(&worlds[1]), digest(&reference));
}
