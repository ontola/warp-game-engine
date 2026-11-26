//! Rollback netcode (GGPO style) for a deterministic sim played online.
//!
//! Every couch runs the whole game itself and draws it itself, so its own
//! players feel no lag at all. Inputs from other couches arrive late: until
//! they do, the sim guesses them (the player keeps doing what they did), and
//! when a real input differs from the guess it goes back to the last frame
//! it got right (a saved state), and simulates forward again with the real
//! inputs, all within one rendered frame. Most of the time the guess is
//! right and nothing is redone.
//!
//! [`Rollback`] only keeps frames, inputs and states; how inputs travel is
//! the caller's (see [`InputRun`] for the wire shape). The game implements
//! [`Game`]: save and load its state, and step one frame with one input per
//! seat. During a resimulation `step` is told so, to keep sound, effects and
//! banners of frames already shown from firing twice.

use std::collections::VecDeque;

/// A game that can be rolled back.
pub trait Game {
    /// Everything the next frames depend on (not the level's fixed data).
    type State: Clone;
    /// One seat's input for one frame. `Default` is "nothing pressed".
    type Input: Copy + Default + PartialEq;
    fn save(&self) -> Self::State;
    fn load(&mut self, state: &Self::State);
    /// One fixed frame, with one input per seat. `resim` is true when this
    /// frame was already shown once (a rollback): skip its effects.
    fn step(&mut self, inputs: &[Self::Input], resim: bool);
}

/// Inputs of one seat for a run of consecutive frames, as sent: each
/// packet repeats the inputs the other side has not confirmed yet, so a lost
/// packet costs nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct InputRun<I> {
    pub seat: usize,
    pub first: u32,
    pub inputs: Vec<I>,
}

/// One seat's inputs: what is known for certain, from `base` on.
struct Seat<I> {
    base: u32,
    known: VecDeque<I>,
    /// The input simulated for each frame from `base` on (known or guessed),
    /// to see whether a late input changes anything.
    used: VecDeque<I>,
    /// The last known input before `base` (the guess past the known ones).
    before: I,
}

impl<I: Copy + Default + PartialEq> Seat<I> {
    fn new(base: u32) -> Self { Seat { base, known: VecDeque::new(), used: VecDeque::new(), before: I::default() } }
    /// Frames up to (not including) this one are known.
    fn known_to(&self) -> u32 { self.base + self.known.len() as u32 }
    fn input(&self, f: u32) -> I {
        if f < self.base { return self.before; }
        let i = (f - self.base) as usize;
        self.known.get(i).or(self.known.back()).copied().unwrap_or(self.before)
    }
}

/// A couch's own inputs for one seat, kept until the other side confirms
/// them (an ack of the next frame it needs), so every packet can repeat them.
pub struct Outbox<I> {
    pub seat: usize,
    first: u32,
    inputs: VecDeque<I>,
}

impl<I: Copy> Outbox<I> {
    pub fn new(seat: usize, first: u32) -> Self { Outbox { seat, first, inputs: VecDeque::new() } }
    /// The next frame's input (frames go up by one).
    pub fn push(&mut self, input: I) { self.inputs.push_back(input); }
    /// The next frame without an input yet.
    pub fn next(&self) -> u32 { self.first + self.inputs.len() as u32 }
    /// The other side has every frame before `upto`.
    pub fn ack(&mut self, upto: u32) {
        while self.first < upto && !self.inputs.is_empty() { self.inputs.pop_front(); self.first += 1; }
    }
    /// What to send now: everything not yet acknowledged.
    pub fn run(&self) -> InputRun<I> { InputRun { seat: self.seat, first: self.first, inputs: self.inputs.iter().copied().collect() } }
}

pub struct Rollback<G: Game> {
    seats: Vec<Seat<G::Input>>,
    /// The next frame to simulate.
    frame: u32,
    /// States saved before simulating each frame from `saved_from` on.
    states: VecDeque<G::State>,
    saved_from: u32,
    /// The first frame simulated with a wrong guess.
    wrong: Option<u32>,
    /// How many frames past the last frame every seat is known for the sim
    /// may run on guesses, before it waits.
    pub max_ahead: u32,
    /// Frames this rolled back over in total (to see how often guesses fail).
    pub resimulated: u64,
}

impl<G: Game> Rollback<G> {
    /// Start at `frame` (usually 0) with `seats` seats, `game` at that frame.
    pub fn new(seats: usize, frame: u32) -> Self {
        Rollback {
            seats: (0..seats).map(|_| Seat::new(frame)).collect(),
            frame,
            states: VecDeque::new(),
            saved_from: frame,
            wrong: None,
            max_ahead: 8,
            resimulated: 0,
        }
    }

    pub fn frame(&self) -> u32 { self.frame }
    pub fn seats(&self) -> usize { self.seats.len() }
    /// Every seat's input is known up to (not including) this frame.
    pub fn confirmed(&self) -> u32 { self.seats.iter().map(Seat::known_to).min().unwrap_or(self.frame) }
    /// The saved state at the start of frame `f`, while it is kept (from
    /// the last confirmed frame on): to compare couches (a desync check).
    pub fn state_at(&self, f: u32) -> Option<&G::State> {
        if f < self.saved_from { return None; }
        self.states.get((f - self.saved_from) as usize)
    }

    /// Frames this one may still run on guesses before it waits.
    pub fn room(&self) -> u32 { (self.confirmed() + self.max_ahead).saturating_sub(self.frame) }

    /// A seat's input for frame `f`, local or from the wire. Inputs must
    /// arrive in order per seat; a repeat of a known frame is ignored, a gap
    /// is ignored too (the sender repeats unconfirmed inputs).
    pub fn input(&mut self, seat: usize, f: u32, input: G::Input) {
        let Some(s) = self.seats.get_mut(seat) else { return };
        if f != s.known_to() { return; }
        s.known.push_back(input);
        // Already simulated with a guess: was it right?
        if f < self.frame {
            let used = s.used.get((f - s.base) as usize).copied();
            if used != Some(input) { self.wrong = Some(self.wrong.map_or(f, |w| w.min(f))); }
        }
    }

    /// Feed a run from the wire.
    pub fn run(&mut self, r: &InputRun<G::Input>) {
        for (i, input) in r.inputs.iter().enumerate() { self.input(r.seat, r.first + i as u32, *input); }
    }

    /// The next frame of a seat's input this couch needs (to ack).
    pub fn needs(&self, seat: usize) -> u32 { self.seats.get(seat).map_or(0, Seat::known_to) }

    /// Roll back when a guess was wrong, then simulate the next frame if
    /// there is room (true when it did). Call once per fixed tick.
    pub fn advance(&mut self, game: &mut G) -> bool {
        self.correct(game);
        if self.room() == 0 { return false; }
        self.simulate(game, false);
        self.forget();
        true
    }

    /// Go back to the first wrongly guessed frame and simulate up to now.
    pub fn correct(&mut self, game: &mut G) {
        let Some(w) = self.wrong.take() else { return };
        let now = self.frame;
        let i = (w - self.saved_from) as usize;
        game.load(&self.states[i]);
        self.states.truncate(i);
        self.frame = w;
        while self.frame < now { self.simulate(game, true); self.resimulated += 1; }
    }

    fn simulate(&mut self, game: &mut G, resim: bool) {
        let f = self.frame;
        self.states.push_back(game.save());
        let inputs: Vec<G::Input> = self.seats.iter_mut().map(|s| {
            let input = s.input(f);
            let i = (f - s.base) as usize;
            if i < s.used.len() { s.used[i] = input; s.used.truncate(i + 1); } else { s.used.push_back(input); }
            input
        }).collect();
        game.step(&inputs, resim);
        self.frame += 1;
    }

    /// Drop states and inputs no rollback can reach any more: frames before
    /// the one every seat is known for (the state before it stays).
    fn forget(&mut self) {
        let keep = self.confirmed().min(self.frame);
        while self.saved_from < keep && self.states.len() > 1 { self.states.pop_front(); self.saved_from += 1; }
        let keep = self.saved_from;
        for s in &mut self.seats {
            while s.base < keep && !s.known.is_empty() {
                s.before = s.known.pop_front().unwrap();
                s.used.pop_front();
                s.base += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toy sim where every input matters forever: positions and a hash.
    #[derive(Clone, Default, PartialEq, Debug)]
    struct Toy { pos: [i64; 3], hash: u64, frames: u32, shown: u32 }

    impl Game for Toy {
        type State = Toy;
        type Input = i8;
        fn save(&self) -> Toy { self.clone() }
        fn load(&mut self, s: &Toy) { let shown = self.shown; *self = s.clone(); self.shown = shown; }
        fn step(&mut self, inputs: &[i8], resim: bool) {
            for (p, i) in self.pos.iter_mut().zip(inputs) { *p += *i as i64; }
            self.hash = self.hash.wrapping_mul(1_000_003).wrapping_add(self.pos.iter().fold(0u64, |h, p| h.wrapping_mul(31).wrapping_add(*p as u64)));
            self.frames += 1;
            if !resim { self.shown += 1; }
        }
    }

    /// The input a seat gives at a frame: changes now and then, like a stick.
    fn pad(seat: usize, f: u32) -> i8 { (((f / (7 + seat as u32 * 3)) as i64 * 37 + seat as i64 * 11) % 5 - 2) as i8 }

    fn reference(frames: u32) -> Toy {
        let mut t = Toy::default();
        for f in 0..frames { let i: Vec<i8> = (0..3).map(|s| pad(s, f)).collect(); t.step(&i, false); }
        t
    }

    #[test]
    fn local_only_never_rolls_back() {
        let mut g = Toy::default();
        let mut rb = Rollback::<Toy>::new(1, 0);
        for f in 0..100 { rb.input(0, f, pad(0, f) ); assert!(rb.advance(&mut g)); }
        assert_eq!(rb.resimulated, 0);
        assert_eq!(rb.states.len(), 1);
        assert_eq!(g.frames, 100);
    }

    /// Three couches, one seat each, with inputs arriving `lag[a][b]` frames
    /// late from b at a: all end in the state a lockstep game would reach.
    #[test]
    fn late_inputs_end_in_the_same_state() {
        let lag = [[0, 3, 6], [3, 0, 4], [6, 4, 0]];
        let n = 300;
        let mut games: Vec<Toy> = (0..3).map(|_| Toy::default()).collect();
        let mut rbs: Vec<Rollback<Toy>> = (0..3).map(|_| { let mut r = Rollback::new(3, 0); r.max_ahead = 10; r }).collect();
        let mut out: Vec<Outbox<i8>> = (0..3).map(|c| Outbox::new(c, 0)).collect();
        // What each couch sent at each tick, delivered `lag` ticks later.
        let mut wire: Vec<(u32, usize, InputRun<i8>)> = Vec::new();
        for t in 0..(n + 40) {
            for c in 0..3 {
                // Own input for the frame about to run.
                let f = out[c].next();
                if f < n && f <= rbs[c].frame() { out[c].push(pad(c, f)); rbs[c].input(c, f, pad(c, f)); }
                wire.push((t, c, out[c].run()));
                // Others' packets that have arrived by now.
                for (sent, from, run) in &wire {
                    if *from != c && sent + lag[c][*from] == t { rbs[c].run(run); }
                }
                // Acks travel back instantly in this test (it only delays repeats).
                for o in 0..3 { if o != c { let need = rbs[c].needs(o); if need > 0 { out[o].ack(need.min(rbs.iter().enumerate().filter(|(i, _)| *i != o).map(|(_, r)| r.needs(o)).min().unwrap())); } } }
                if rbs[c].frame() < n { rbs[c].advance(&mut games[c]); } else { rbs[c].correct(&mut games[c]); }
            }
        }
        let want = reference(n);
        for c in 0..3 {
            assert_eq!(games[c].frames, n, "couch {c}");
            assert_eq!(games[c].pos, want.pos, "couch {c}");
            assert_eq!(games[c].hash, want.hash, "couch {c}");
            assert_eq!(games[c].shown, n, "couch {c}: every frame shown once");
        }
        assert!(rbs[0].resimulated > 0, "guesses were wrong sometimes");
    }

    #[test]
    fn waits_when_too_far_ahead() {
        let mut g = Toy::default();
        let mut rb = Rollback::<Toy>::new(2, 0);
        rb.max_ahead = 4;
        let mut ran = 0;
        for f in 0..10 { rb.input(0, f, 1); if rb.advance(&mut g) { ran += 1; } }
        assert_eq!(ran, 4);
        rb.input(1, 0, 1);
        assert!(rb.advance(&mut g));
        assert!(!rb.advance(&mut g));
    }

    #[test]
    fn a_right_guess_costs_nothing() {
        let mut g = Toy::default();
        let mut rb = Rollback::<Toy>::new(2, 0);
        for f in 0..5 { rb.input(0, f, 1); rb.advance(&mut g); }
        for f in 0..5 { rb.input(1, f, 0); }
        rb.advance(&mut g);
        assert_eq!(rb.resimulated, 0);
        rb.input(0, 5, 1);
        rb.input(1, 5, 2); // already simulated frame 5 with guess 0
        rb.advance(&mut g);
        assert_eq!(rb.resimulated, 1);
    }
}
