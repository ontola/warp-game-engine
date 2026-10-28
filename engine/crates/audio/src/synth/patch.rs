//! Patches: a sound as a graph of units, as data. A patch compiles into a
//! [`Voice`]: the units in order, their state, and one block buffer each.
use super::units::{Sig, State, Unit};
use super::BLOCK;
use serde::{Deserialize, Serialize};

/// Where a unit input comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum In {
    /// A constant.
    K(f32),
    /// A named parameter of the patch (set when played, may glide later).
    P(String),
    /// The output of an earlier node.
    N(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub unit: Unit,
    /// One per unit input, in order; missing ones take the unit's default.
    pub inputs: Vec<In>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub name: String,
    pub nodes: Vec<Node>,
    /// The node that is the sound.
    pub out: usize,
    /// Parameters with their defaults.
    pub params: Vec<(String, f32)>,
    /// Seconds after which the voice ends anyway (0: only when done and quiet).
    #[serde(default)]
    pub max_length: f32,
    /// Plays until stopped (beds, engines).
    #[serde(default)]
    pub looping: bool,
}

impl Patch {
    /// Check the graph: inputs only from earlier nodes, known parameters,
    /// input counts within the unit's.
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() { return Err(format!("{}: no nodes", self.name)); }
        if self.out >= self.nodes.len() { return Err(format!("{}: out is node {} of {}", self.name, self.out, self.nodes.len())); }
        for (k, n) in self.nodes.iter().enumerate() {
            let names = n.unit.inputs();
            if n.inputs.len() > names.len() { return Err(format!("{}: node {k} ({}) takes {} inputs ({}), got {}", self.name, n.unit.name(), names.len(), names.join(", "), n.inputs.len())); }
            if let Unit::Expr { src, vars } = &n.unit {
                if vars.len() > 6 { return Err(format!("{}: node {k} (expr) has {} inputs, at most 6", self.name, vars.len())); }
                let e = super::expr::Expr::compile(src, vars).map_err(|e| format!("{}: node {k} (expr): {e}", self.name))?;
                if !e.fits() { return Err(format!("{}: node {k} (expr) is nested too deeply", self.name)); }
            }
            for (j, i) in n.inputs.iter().enumerate() {
                match i {
                    In::N(m) if *m >= k => return Err(format!("{}: node {k} ({}) input {} reads node {m}, which comes later (no cycles)", self.name, n.unit.name(), names[j])),
                    In::P(p) if !self.params.iter().any(|q| &q.0 == p) => return Err(format!("{}: unknown parameter {p}", self.name)),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// A playing instance. Allocates (call it off the audio thread).
    pub fn voice(&self, params: &[(&str, f32)], seed: u32, sr: f32) -> Voice {
        let values: Vec<f32> = self.params.iter().map(|(n, d)| params.iter().find(|p| p.0 == n).map_or(*d, |p| p.1)).collect();
        let nodes = self.nodes.iter().enumerate().map(|(k, n)| {
            let ins = n.inputs.iter().map(|i| match i {
                In::K(v) => Src::K(*v),
                In::P(p) => Src::P(self.params.iter().position(|q| &q.0 == p).unwrap_or(0)),
                In::N(m) => Src::N(*m),
            }).collect();
            let s = seed.wrapping_mul(0x9E37_79B9).wrapping_add((k as u32).wrapping_mul(0x85EB_CA6B)) | 1;
            (n.unit.clone(), State::new(&n.unit, sr, s), ins)
        }).collect::<Vec<_>>();
        Voice {
            bufs: vec![[0.0; BLOCK]; nodes.len()],
            konst: vec![None; nodes.len()],
            nodes, out: self.out, sr, prev: values.clone(), values,
            gate: true, time: 0.0, quiet: 0, max_length: self.max_length, looping: self.looping,
            has_env: self.nodes.iter().any(|n| matches!(n.unit, Unit::Env { .. } | Unit::Adsr | Unit::Clip { looping: false, .. })),
            names: self.params.iter().map(|p| p.0.clone()).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Src { K(f32), P(usize), N(usize) }

/// A compiled, playing patch: mono out, one block at a time.
pub struct Voice {
    nodes: Vec<(Unit, State, Vec<Src>)>,
    bufs: Vec<[f32; BLOCK]>,
    /// Nodes whose output is constant this block (inputs read them as K).
    konst: Vec<Option<f32>>,
    out: usize,
    sr: f32,
    values: Vec<f32>,
    /// Values at the start of this block (parameters glide across a block).
    prev: Vec<f32>,
    names: Vec<String>,
    gate: bool,
    time: f32,
    quiet: u32,
    max_length: f32,
    looping: bool,
    has_env: bool,
}

/// Blocks below this peak count as quiet.
const QUIET: f32 = 1e-4;
/// This many quiet blocks (about 0.1 s) after the envelopes end a voice.
const QUIET_BLOCKS: u32 = 75;

impl Voice {
    /// Set a parameter by name (it glides over the next block).
    pub fn set(&mut self, name: &str, v: f32) { if let Some(k) = self.names.iter().position(|n| n == name) { self.values[k] = v; } }
    pub fn set_index(&mut self, k: usize, v: f32) { if let Some(x) = self.values.get_mut(k) { *x = v; } }
    pub fn param_index(&self, name: &str) -> Option<usize> { self.names.iter().position(|n| n == name) }
    /// Release: ADSR units go to their release stage.
    pub fn release(&mut self) { self.gate = false; }
    pub fn time(&self) -> f32 { self.time }

    /// Render one block; returns false when the voice is over.
    pub fn render(&mut self, out: &mut [f32; BLOCK]) -> bool {
        for k in 0..self.nodes.len() {
            let (done, rest) = self.bufs.split_at_mut(k);
            let (unit, state, srcs) = &mut self.nodes[k];
            let mut ins = [Sig::K(0.0); 6];
            for (j, s) in srcs.iter().enumerate().take(6) {
                ins[j] = match *s {
                    Src::K(v) => Sig::K(v),
                    Src::P(p) => { let (a, b) = (self.prev[p], self.values[p]); if a == b { Sig::K(b) } else { Sig::Ramp(a, b) } }
                    Src::N(m) => match self.konst[m] { Some(v) => Sig::K(v), None => Sig::B(&done[m]) },
                };
            }
            let n = srcs.len().min(6);
            // Inputs past the connected ones take defaults.
            let defaults: [Sig; 6] = std::array::from_fn(|j| if j < n { ins[j] } else { Sig::K(unit.default_input(j)) });
            let ins = &defaults[..unit.inputs().len().max(n)];
            if state.pure() && ins.iter().all(|s| matches!(s, Sig::K(_))) {
                // Constant this block: one sample.
                state.run(ins, &mut rest[0][..1], self.sr, self.gate, unit);
                let v = rest[0][0];
                self.konst[k] = Some(v);
                if k == self.out { rest[0].fill(v); }
            } else {
                self.konst[k] = None;
                state.run(ins, &mut rest[0], self.sr, self.gate, unit);
            }
        }
        self.prev.copy_from_slice(&self.values);
        out.copy_from_slice(&self.bufs[self.out]);
        self.time += BLOCK as f32 / self.sr;
        if self.looping && self.gate { return true; }
        if self.max_length > 0.0 && self.time >= self.max_length { return false; }
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if !peak.is_finite() { return false; }
        let envs_done = self.nodes.iter().all(|(_, s, _)| s.finished().unwrap_or(true));
        if (envs_done || !self.has_env || !self.gate) && peak < QUIET { self.quiet += 1; } else { self.quiet = 0; }
        // Without envelopes a voice lasts until quiet (or max_length).
        self.quiet < QUIET_BLOCKS || (!envs_done && self.gate)
    }

    /// Render offline into a buffer of `seconds` (or until the voice ends).
    pub fn render_all(&mut self, seconds: f32) -> Vec<f32> {
        let n = (seconds * self.sr) as usize;
        let mut out = Vec::with_capacity(n);
        let mut b = [0.0; BLOCK];
        while out.len() < n {
            let alive = self.render(&mut b);
            out.extend_from_slice(&b[..BLOCK.min(n - out.len())]);
            if !alive { break; }
        }
        out
    }
}
