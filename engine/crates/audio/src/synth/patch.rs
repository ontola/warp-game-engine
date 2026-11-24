//! Patches: a sound as a graph of units, as data. A patch compiles into a
//! [`Voice`]: the units in order, their state, and one block buffer each.
use super::units::{Sig, State, Unit};
use super::BLOCK;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    /// A parameter whose value (s) ends the voice, like `max_length`.
    #[serde(default)]
    pub length_param: Option<String>,
}

impl Patch {
    /// Check the graph: inputs only from earlier nodes, known parameters,
    /// input counts within the unit's.
    pub fn validate(&self) -> Result<(), String> {
        self.validated_expressions().map(|_| ())
    }

    fn validated_expressions(&self) -> Result<Vec<Option<Arc<super::expr::Expr>>>, String> {
        if self.nodes.is_empty() { return Err(format!("{}: no nodes", self.name)); }
        if self.out >= self.nodes.len() { return Err(format!("{}: out is node {} of {}", self.name, self.out, self.nodes.len())); }
        let mut expressions = Vec::with_capacity(self.nodes.len());
        for (k, n) in self.nodes.iter().enumerate() {
            let names = n.unit.inputs();
            if n.inputs.len() > names.len() { return Err(format!("{}: node {k} ({}) takes {} inputs ({}), got {}", self.name, n.unit.name(), names.len(), names.join(", "), n.inputs.len())); }
            if let Unit::Engine { car, fluid, muffler: Some(geometry), spec } = &n.unit {
                if *fluid { return Err(format!("{}: node {k}: muffler geometry requires the pulse engine", self.name)); }
                if spec.0.is_none() && super::units::engine_spec(car).is_none() {
                    return Err(format!("{}: node {k}: unknown engine preset '{car}'", self.name));
                }
                geometry.validate().map_err(|e| format!("{}: node {k} (engine): {e}", self.name))?;
            }
            let expr = if let Unit::Expr { src, vars } = &n.unit {
                if vars.len() > 6 { return Err(format!("{}: node {k} (expr) has {} inputs, at most 6", self.name, vars.len())); }
                let e = super::expr::Expr::compile(src, vars).map_err(|e| format!("{}: node {k} (expr): {e}", self.name))?;
                if !e.fits() { return Err(format!("{}: node {k} (expr) is nested too deeply", self.name)); }
                Some(Arc::new(e))
            } else { None };
            expressions.push(expr);
            for (j, i) in n.inputs.iter().enumerate() {
                match i {
                    In::N(m) if *m >= k => return Err(format!("{}: node {k} ({}) input {} reads node {m}, which comes later (no cycles)", self.name, n.unit.name(), names[j])),
                    In::P(p) if !self.params.iter().any(|q| &q.0 == p) => return Err(format!("{}: unknown parameter {p}", self.name)),
                    _ => {}
                }
            }
        }
        if let Some(p) = &self.length_param && !self.params.iter().any(|q| &q.0 == p) { return Err(format!("{}: length from unknown parameter {p}", self.name)); }
        Ok(expressions)
    }

    /// Prepare immutable graph data once when loading or editing a patch.
    pub fn compile(&self) -> Result<CompiledPatch, String> {
        Ok(self.compile_inner(Some(self.validated_expressions()?)))
    }

    fn compile_inner(&self, expressions: Option<Vec<Option<Arc<super::expr::Expr>>>>) -> CompiledPatch {
        let nodes = self.nodes.iter().enumerate().map(|(k, n)| {
            let inputs = std::array::from_fn(|j| match n.inputs.get(j) {
                Some(In::K(v)) => Src::K(*v),
                Some(In::P(p)) => Src::P(self.params.iter().position(|q| &q.0 == p).unwrap_or(0)),
                Some(In::N(m)) => Src::N(*m),
                None => Src::K(n.unit.default_input(j)),
            });
            let expr = if let Some(expressions) = &expressions { expressions[k].clone() } else if let Unit::Expr { src, vars } = &n.unit {
                Some(Arc::new(super::expr::Expr::compile(src, vars).unwrap_or_else(|_| super::expr::Expr::compile("0", &[]).unwrap())))
            } else { None };
            let pure = match &n.unit {
                Unit::Mul | Unit::Add | Unit::Clamp | Unit::Drive | Unit::Fold | Unit::Map { .. } | Unit::Rand => true,
                Unit::Expr { .. } => expr.as_ref().is_some_and(|e| !e.uses_t()),
                _ => false,
            };
            CompiledNode { inputs, input_count: n.unit.inputs().len().max(n.inputs.len().min(6)), expr, pure }
        }).collect::<Vec<_>>().into();
        CompiledPatch {
            source: Arc::new(self.clone()), graph: nodes,
            has_env: self.nodes.iter().any(|n| matches!(n.unit, Unit::Env { .. } | Unit::Adsr | Unit::Clip { looping: false, .. })),
            length_idx: self.length_param.as_ref().and_then(|n| self.params.iter().position(|p| &p.0 == n)),
        }
    }

    /// A one-off playing instance. For repeated playback, retain [`Self::compile`]'s result.
    /// Allocates on the calling thread, as before.
    pub fn voice(&self, params: &[(&str, f32)], seed: u32, sr: f32) -> Voice {
        self.compile_inner(None).voice(params, seed, sr)
    }
}

struct CompiledNode {
    inputs: [Src; 6],
    input_count: usize,
    expr: Option<Arc<super::expr::Expr>>,
    pure: bool,
}

/// An immutable patch snapshot. Units, input routing and expression programs
/// are shared by every voice; only oscillator/filter/envelope state is private.
#[derive(Clone)]
pub struct CompiledPatch {
    source: Arc<Patch>,
    graph: Arc<[CompiledNode]>,
    has_env: bool,
    length_idx: Option<usize>,
}

impl std::ops::Deref for CompiledPatch {
    type Target = Patch;
    fn deref(&self) -> &Patch { &self.source }
}

impl CompiledPatch {
    /// A new snapshot with an edited default. Active voices keep their defaults;
    /// the compiled graph and formulas are reused because routing did not change.
    pub fn with_default(&self, param: &str, value: f32) -> Result<(Self, f32), String> {
        let Some(k) = self.params.iter().position(|p| p.0 == param) else {
            return Err(format!("{} has no parameter {param} (has {})", self.name, self.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")));
        };
        let old = self.params[k].1;
        let mut next = self.clone();
        Arc::make_mut(&mut next.source).params[k].1 = value;
        Ok((next, old))
    }

    /// Start a voice without cloning the graph or compiling expressions.
    pub fn voice(&self, params: &[(&str, f32)], seed: u32, sr: f32) -> Voice {
        let values: Vec<f32> = self.params.iter().map(|(n, d)| params.iter().find(|p| p.0 == n).map_or(*d, |p| p.1)).collect();
        let states = self.source.nodes.iter().zip(self.graph.iter()).enumerate().map(|(k, (n, compiled))| {
            // A finaliser (murmur3) so neighbouring nodes and seeds start
            // their generators uncorrelated.
            let mut s = seed.wrapping_mul(0x9E37_79B9) ^ (k as u32).wrapping_mul(0x85EB_CA6B);
            s ^= s >> 16; s = s.wrapping_mul(0x85EB_CA6B); s ^= s >> 13; s = s.wrapping_mul(0xC2B2_AE35); s ^= s >> 16;
            let s = s | 1;
            State::new(&n.unit, sr, s, compiled.expr.as_ref())
        }).collect::<Vec<_>>();
        Voice {
            bufs: vec![[0.0; BLOCK]; states.len()],
            konst: vec![None; states.len()],
            patch: self.clone(), states, out: self.out, sr, prev: values.clone(), values,
            gate: true, time: 0.0, quiet: 0, max_length: self.max_length, looping: self.looping,
            has_env: self.has_env,
            length_idx: self.length_idx,
        }
    }
}

/// Sources accepted by the mixer, including the original editable patch API.
pub trait PatchSource {
    fn voice(&self, params: &[(&str, f32)], seed: u32, sr: f32) -> Voice;
}
impl PatchSource for Patch {
    fn voice(&self, params: &[(&str, f32)], seed: u32, sr: f32) -> Voice { Patch::voice(self, params, seed, sr) }
}
impl PatchSource for CompiledPatch {
    fn voice(&self, params: &[(&str, f32)], seed: u32, sr: f32) -> Voice { CompiledPatch::voice(self, params, seed, sr) }
}

#[derive(Clone, Copy, Debug)]
enum Src { K(f32), P(usize), N(usize) }

/// A compiled, playing patch: mono out, one block at a time.
pub struct Voice {
    pub(super) patch: CompiledPatch,
    states: Vec<State>,
    bufs: Vec<[f32; BLOCK]>,
    /// Nodes whose output is constant this block (inputs read them as K).
    konst: Vec<Option<f32>>,
    out: usize,
    sr: f32,
    values: Vec<f32>,
    /// Values at the start of this block (parameters glide across a block).
    prev: Vec<f32>,
    gate: bool,
    time: f32,
    quiet: u32,
    max_length: f32,
    looping: bool,
    has_env: bool,
    length_idx: Option<usize>,
}

/// Blocks below this peak count as quiet.
const QUIET: f32 = 1e-4;
/// This many quiet blocks (about 0.1 s) after the envelopes end a voice.
const QUIET_BLOCKS: u32 = 75;

impl Voice {
    /// Set a parameter by name (it glides over the next block).
    pub fn set(&mut self, name: &str, v: f32) { if let Some(k) = self.param_index(name) { self.values[k] = v; } }
    pub fn set_index(&mut self, k: usize, v: f32) { if let Some(x) = self.values.get_mut(k) { *x = v; } }
    pub fn param_index(&self, name: &str) -> Option<usize> { self.patch.params.iter().position(|p| p.0 == name) }
    /// Release: ADSR units go to their release stage.
    pub fn release(&mut self) { self.gate = false; }
    pub fn time(&self) -> f32 { self.time }

    /// Render one block; returns false when the voice is over.
    pub fn render(&mut self, out: &mut [f32; BLOCK]) -> bool {
        for k in 0..self.states.len() {
            let (done, rest) = self.bufs.split_at_mut(k);
            let unit = &self.patch.source.nodes[k].unit;
            let state = &mut self.states[k];
            let compiled = &self.patch.graph[k];
            let mut ins = [Sig::K(0.0); 6];
            for (j, s) in compiled.inputs.iter().enumerate().take(compiled.input_count) {
                ins[j] = match *s {
                    Src::K(v) => Sig::K(v),
                    Src::P(p) => { let (a, b) = (self.prev[p], self.values[p]); if a == b { Sig::K(b) } else { Sig::Ramp(a, b) } }
                    Src::N(m) => match self.konst[m] { Some(v) => Sig::K(v), None => Sig::B(&done[m]) },
                };
            }
            let ins = &ins[..compiled.input_count];
            if compiled.pure && ins.iter().all(|s| matches!(s, Sig::K(_))) {
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
        if let Some(k) = self.length_idx && self.time >= self.values[k] { return false; }
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if !peak.is_finite() { return false; }
        let envs_done = self.states.iter().zip(self.patch.source.nodes.iter()).all(|(s, n)| s.finished(&n.unit).unwrap_or(true));
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
