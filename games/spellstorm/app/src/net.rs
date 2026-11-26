//! Spellstorm online, couch with couch (engine/docs/online.md): every couch
//! runs the whole run itself and draws it itself, so its own wizards react
//! at once. The other couches' controllers arrive a little later; until
//! they do the run guesses them, and goes back and plays the frames again
//! when a guess was wrong (rollback, kern::rollback).
//!
//! The host decides who plays: it starts a run with a wizard for each
//! person on each couch, and a new one whenever a couch comes or goes.
//! Messages, each `[kind][sender u64]` and then:
//!
//! - `HELLO` people `u8`: a couch says how many sit on it, until it plays.
//! - `START` seed `u64`, then per wizard its couch `u64`: a fresh run.
//! - `INPUTS` run `u32`, then runs of inputs (seat `u8`, first frame `u32`,
//!   count `u8`, pads) and what this couch still needs (seat `u8`, frame
//!   `u32`): each packet repeats every input not acknowledged yet.

use app::net::Wire;
use kern::rollback::{Outbox, Rollback};
use spellstorm::net::NetPad;
use spellstorm::World;
use std::collections::BTreeMap;

const HELLO: u8 = 1;
const START: u8 = 2;
const INPUTS: u8 = 3;

/// Frames a couch's own input waits before it plays: fewer rollbacks for a
/// delay nobody feels (2 frames, 33 ms).
const DELAY: u32 = 2;

pub struct Online {
    wire: Box<dyn Wire>,
    /// This couch's name on the wire (random; link ids differ per side).
    me: u64,
    hosting: bool,
    people: u8,
    /// Host: the couches that said hello, and how many sit on each.
    couches: BTreeMap<u64, u8>,
    /// Host: link id to couch, to see who left.
    links: BTreeMap<u64, u64>,
    pub run: Option<NetRun>,
    /// Runs started, so a late packet of an old run is ignored.
    run_id: u32,
    hello_t: u32,
    /// Host: the roster changed; start again at the next tick.
    restart: bool,
    /// Tests: play up to this frame only.
    pub stop_at: Option<u32>,
}

pub struct NetRun {
    pub rb: Rollback<World>,
    /// Each wizard's couch.
    pub owners: Vec<u64>,
    /// This couch's wizards, in its controllers' order.
    pub mine: Vec<usize>,
    out: Vec<Outbox<NetPad>>,
    /// What each other couch still needs of each seat.
    acks: BTreeMap<(u64, usize), u32>,
    pub seed: u64,
}

/// What the game should do after a tick of the session.
pub enum Do {
    /// Nothing new: keep the world.
    Nothing,
    /// A fresh run with these wizards (couch per seat) and seed.
    Start { seed: u64, owners: Vec<u64>, mine: Vec<usize> },
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> { Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?)) }
fn u32_at(b: &[u8], i: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?)) }

impl Online {
    pub fn new(wire: Box<dyn Wire>, people: usize, me: u64) -> Online {
        let hosting = wire.hosting();
        let mut o = Online { wire, me, hosting, people: people.clamp(1, 4) as u8, couches: BTreeMap::new(), links: BTreeMap::new(), run: None, run_id: 0, hello_t: 0, restart: hosting, stop_at: None };
        if hosting { o.couches.insert(me, o.people); }
        o
    }

    pub fn hosting(&self) -> bool { self.hosting }

    fn header(&self, kind: u8) -> Vec<u8> {
        let mut b = vec![kind];
        b.extend(self.me.to_le_bytes());
        b
    }

    /// Read the wire; the host also decides the run. Call once per tick
    /// before [`Online::advance`].
    pub fn poll(&mut self) -> Do {
        let mut start = Do::Nothing;
        for (link, data) in self.wire.poll() {
            let (Some(&kind), Some(from)) = (data.first(), u64_at(&data, 1)) else { continue };
            let body = &data[9..];
            match kind {
                HELLO if self.hosting => {
                    let people = body.first().copied().unwrap_or(1).clamp(1, 4);
                    self.links.insert(link, from);
                    if self.couches.insert(from, people) != Some(people) { self.restart = true; }
                }
                START if !self.hosting => {
                    let Some(seed) = u64_at(body, 0) else { continue };
                    let Some(id) = u32_at(body, 8) else { continue };
                    let owners: Vec<u64> = body[12..].chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
                    if !owners.contains(&self.me) { continue; }
                    self.run_id = id;
                    start = self.begin(seed, owners);
                }
                INPUTS => self.inputs(from, body),
                _ => {}
            }
        }
        if self.hosting {
            // A couch that left takes its wizards with it.
            let alive = self.wire.couches();
            let gone: Vec<u64> = self.links.keys().copied().filter(|l| !alive.contains(l)).collect();
            for l in gone {
                if let Some(c) = self.links.remove(&l) { self.couches.remove(&c); self.restart = true; }
            }
            if std::mem::take(&mut self.restart) {
                let seed = self.run.as_ref().map_or(self.me, |r| r.seed).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 1;
                let mut owners = Vec::new();
                for (&c, &n) in &self.couches { for _ in 0..n { if owners.len() < 4 { owners.push(c); } } }
                self.run_id += 1;
                let mut b = self.header(START);
                b.extend(seed.to_le_bytes());
                b.extend(self.run_id.to_le_bytes());
                for o in &owners { b.extend(o.to_le_bytes()); }
                self.wire.send(b);
                start = self.begin(seed, owners);
            }
        } else if self.run.is_none() {
            // Hello until the host starts a run with this couch in it.
            if self.hello_t % 30 == 0 {
                let mut b = self.header(HELLO);
                b.push(self.people);
                self.wire.send(b);
            }
            self.hello_t += 1;
        }
        start
    }

    fn begin(&mut self, seed: u64, owners: Vec<u64>) -> Do {
        let mine: Vec<usize> = owners.iter().enumerate().filter(|(_, o)| **o == self.me).map(|(i, _)| i).collect();
        let mut rb = Rollback::new(owners.len(), 0);
        rb.max_ahead = 12;
        let mut out: Vec<Outbox<NetPad>> = mine.iter().map(|&s| Outbox::new(s, 0)).collect();
        // The first frames of the delay: nothing pressed.
        for (o, &s) in out.iter_mut().zip(&mine) {
            for f in 0..DELAY { o.push(NetPad::default()); rb.input(s, f, NetPad::default()); }
        }
        let mut acks = BTreeMap::new();
        for &c in owners.iter().filter(|c| **c != self.me) { for &s in &mine { acks.insert((c, s), 0); } }
        self.run = Some(NetRun { rb, owners: owners.clone(), mine: mine.clone(), out, acks, seed });
        Do::Start { seed, owners, mine }
    }

    fn inputs(&mut self, from: u64, b: &[u8]) {
        let Some(run) = &mut self.run else { return };
        if u32_at(b, 0) != Some(self.run_id) { return; }
        let mut i = 4;
        let Some(&runs) = b.get(i) else { return };
        i += 1;
        for _ in 0..runs {
            let (Some(&seat), Some(first), Some(&n)) = (b.get(i), u32_at(b, i + 1), b.get(i + 5)) else { return };
            i += 6;
            for k in 0..n as u32 {
                let Some(p) = NetPad::read(&b[i.min(b.len())..]) else { return };
                i += NetPad::BYTES;
                if run.owners.get(seat as usize) == Some(&from) { run.rb.input(seat as usize, first + k, p); }
            }
        }
        let Some(&needs) = b.get(i) else { return };
        i += 1;
        for _ in 0..needs {
            let (Some(&seat), Some(f)) = (b.get(i), u32_at(b, i + 1)) else { return };
            i += 5;
            if let Some(a) = run.acks.get_mut(&(from, seat as usize)) { *a = (*a).max(f); }
        }
    }

    /// This couch's pads for its next frame, then one frame of the run
    /// (with any rollback), and the inputs out. False when the run waits
    /// for the other couches.
    pub fn advance(&mut self, world: &mut World, pads: &[NetPad]) -> bool {
        let mut b = self.header(INPUTS);
        let Some(run) = &mut self.run else { return false };
        // Own input for the frame DELAY ahead of the one about to run.
        let f = run.rb.frame() + DELAY;
        let stop = self.stop_at.is_some_and(|s| run.rb.frame() >= s);
        for (k, o) in run.out.iter_mut().enumerate() {
            if o.next() == f && !stop {
                let p = pads.get(k).copied().unwrap_or_default();
                o.push(p);
                run.rb.input(o.seat, f, p);
            }
        }
        for o in run.out.iter_mut() {
            let acked = run.acks.iter().filter(|((_, s), _)| *s == o.seat).map(|(_, f)| *f).min().unwrap_or(u32::MAX);
            if acked != u32::MAX { o.ack(acked); }
        }
        b.extend(self.run_id.to_le_bytes());
        b.push(run.out.len() as u8);
        for o in &run.out {
            let r = o.run();
            let n = r.inputs.len().min(255);
            b.push(r.seat as u8);
            b.extend(r.first.to_le_bytes());
            b.push(n as u8);
            for p in &r.inputs[..n] { p.write(&mut b); }
        }
        let others: Vec<usize> = (0..run.owners.len()).filter(|s| !run.mine.contains(s)).collect();
        b.push(others.len() as u8);
        for s in others {
            b.push(s as u8);
            b.extend(run.rb.needs(s).to_le_bytes());
        }
        self.wire.send(b);
        if stop { run.rb.correct(world); return false; }
        run.rb.advance(world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// Packets in flight: (deliver at tick, to couch, from link, data).
    type Air = Rc<RefCell<(u32, VecDeque<(u32, usize, u64, Vec<u8>)>)>>;

    /// Couch 0 hosts and relays; couches 1.. follow. `lag` ticks one way.
    struct Fake { air: Air, me: usize, n: usize, lag: u32 }

    impl Wire for Fake {
        fn hosting(&self) -> bool { self.me == 0 }
        fn send(&mut self, data: Vec<u8>) {
            let mut a = self.air.borrow_mut();
            let t = a.0;
            // A follower reaches the host, which passes it on to the others.
            for to in 0..self.n {
                if to == self.me { continue; }
                let hops = if self.me != 0 && to != 0 { 2 } else { 1 };
                a.1.push_back((t + self.lag * hops, to, self.me as u64, data.clone()));
            }
        }
        fn poll(&mut self) -> Vec<(u64, Vec<u8>)> {
            let mut a = self.air.borrow_mut();
            let t = a.0;
            let (now, later): (VecDeque<_>, VecDeque<_>) = a.1.drain(..).partition(|p| p.0 <= t && p.1 == self.me);
            a.1 = later;
            now.into_iter().map(|p| (p.2, p.3)).collect()
        }
        fn couches(&self) -> Vec<u64> { (1..self.n as u64).collect() }
    }

    fn digest(w: &World) -> String { let mut d = kern::Digest::default(); kern::Sim::digest(w, &mut d); d.hex16() }

    #[test]
    fn three_couches_play_one_run() {
        let air: Air = Rc::new(RefCell::new((0, VecDeque::new())));
        let n = 3;
        let mut couches: Vec<(Online, World)> = (0..n).map(|i| {
            let wire = Box::new(Fake { air: air.clone(), me: i, n, lag: 4 });
            let mut o = Online::new(wire, if i == 1 { 2 } else { 1 }, 100 + i as u64);
            o.stop_at = Some(600);
            (o, World::new(&[spellstorm::Seat::player(0, "W")], 1, spellstorm::Settings::default()))
        }).collect();
        for t in 0..1200u32 {
            air.borrow_mut().0 = t;
            for (i, (o, w)) in couches.iter_mut().enumerate() {
                if let Do::Start { seed, owners, .. } = o.poll() {
                    let seats: Vec<_> = (0..owners.len()).map(|k| spellstorm::Seat::player(k, "W")).collect();
                    *w = World::new(&seats, seed, spellstorm::Settings::default());
                }
                let k = (t / 20) as i8;
                let pads = [NetPad { mv: [k.wrapping_mul(17), 100], aim: [0, 0], held: (t / 33 % 2) << 15 }, NetPad { mv: [-100, i as i8 * 30], aim: [90, 0], held: 0 }];
                o.advance(w, &pads);
            }
        }
        let owners = couches[0].0.run.as_ref().unwrap().owners.clone();
        assert_eq!(owners.len(), 4, "a wizard per person: {owners:?}");
        let want = digest(&couches[0].1);
        for (i, (o, w)) in couches.iter().enumerate() {
            let run = o.run.as_ref().unwrap();
            assert_eq!(run.rb.frame(), 600, "couch {i}");
            assert_eq!(run.owners, owners, "couch {i}");
            assert_eq!(digest(w), want, "couch {i}");
        }
    }
}
