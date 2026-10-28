//! Patches written in Luau. Nodes are values; arithmetic builds the graph:
//!
//! ```lua
//! local size = param("size", 1)
//! local burst = noise() * env{ {0.001, 1}, {0.06, 0, 4} }
//! length(1.5)
//! return lowpass(burst, 300 + 4000 * size)
//! ```
//!
//! A file returns its sound (named after the file) and may define more with
//! `sound("name", function() ... return sig end)`. Each load runs in a fresh,
//! sandboxed VM under a step budget, so a broken edit only fails to load.
use super::patch::{In, Node, Patch};
use super::units::{FilterMode, Mode, NoiseColor, Point, Shape, Unit};
use mlua::{Function, Lua, LuaOptions, MetaMethod, StdLib, Table, UserData, UserDataMethods, Value, Variadic, VmState};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// A signal in the graph (a node output or a parameter).
#[derive(Clone)]
struct S(In);

impl UserData for S {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_function(MetaMethod::Add, |lua, (a, b): (Value, Value)| { let (a, b) = (sig(&a)?, sig(&b)?); Ok(push(lua, Unit::Add, vec![a, b])) });
        m.add_meta_function(MetaMethod::Mul, |lua, (a, b): (Value, Value)| { let (a, b) = (sig(&a)?, sig(&b)?); Ok(push(lua, Unit::Mul, vec![a, b])) });
        m.add_meta_function(MetaMethod::Sub, |lua, (a, b): (Value, Value)| {
            let (a, b) = (sig(&a)?, sig(&b)?);
            let nb = neg(lua, b);
            Ok(push(lua, Unit::Add, vec![a, nb]))
        });
        m.add_meta_function(MetaMethod::Unm, |lua, (a, _): (Value, Value)| { let a = sig(&a)?; Ok(S(neg(lua, a))) });
        m.add_meta_function(MetaMethod::Div, |lua, (a, b): (Value, Value)| {
            let (a, b) = (sig(&a)?, sig(&b)?);
            Ok(match b {
                In::K(v) if v != 0.0 => push(lua, Unit::Mul, vec![a, In::K(1.0 / v)]),
                b => push(lua, Unit::Expr { src: "a / b".into(), vars: vec!["a".into(), "b".into()] }, vec![a, b]),
            })
        });
        m.add_meta_function(MetaMethod::Pow, |lua, (a, b): (Value, Value)| {
            let (a, b) = (sig(&a)?, sig(&b)?);
            Ok(push(lua, Unit::Expr { src: "a ^ b".into(), vars: vec!["a".into(), "b".into()] }, vec![a, b]))
        });
        m.add_meta_method(MetaMethod::ToString, |_, s, ()| Ok(match &s.0 { In::N(k) => format!("node {k}"), In::P(p) => format!("param {p}"), In::K(v) => v.to_string() }));
    }
}

#[derive(Default)]
struct Builder { name: String, nodes: Vec<Node>, params: Vec<(String, f32)>, max_length: f32, looping: bool, length_param: Option<String> }

fn push(lua: &Lua, unit: Unit, inputs: Vec<In>) -> S {
    let mut b = lua.app_data_mut::<Builder>().expect("builder");
    b.nodes.push(Node { unit, inputs });
    S(In::N(b.nodes.len() - 1))
}

fn neg(lua: &Lua, x: In) -> In {
    match x { In::K(v) => In::K(-v), x => push(lua, Unit::Mul, vec![x, In::K(-1.0)]).0 }
}

fn sig(v: &Value) -> mlua::Result<In> {
    match v {
        Value::Number(n) => Ok(In::K(*n as f32)),
        Value::Integer(n) => Ok(In::K(*n as f32)),
        Value::UserData(u) => Ok(u.borrow::<S>()?.0.clone()),
        Value::Boolean(b) => Ok(In::K(*b as u8 as f32)),
        other => Err(mlua::Error::runtime(format!("expected a number or a signal, got {}", other.type_name()))),
    }
}

/// Inputs from positional arguments; nil in the middle takes the default,
/// trailing nils are left off.
fn inputs(unit: &Unit, args: &[Value]) -> mlua::Result<Vec<In>> {
    let n = args.iter().rposition(|v| !v.is_nil()).map_or(0, |k| k + 1);
    let max = unit.inputs().len();
    if n > max { return Err(mlua::Error::runtime(format!("{} takes {max} inputs ({}), got {n}", unit.name(), unit.inputs().join(", ")))); }
    args[..n].iter().enumerate().map(|(j, v)| if v.is_nil() { Ok(In::K(unit.default_input(j))) } else { sig(v) }).collect()
}

fn add_unit(lua: &Lua, g: &Table, name: &str, make: impl Fn() -> Unit + Send + 'static) -> mlua::Result<()> {
    g.raw_set(name, lua.create_function(move |lua, args: Variadic<Value>| {
        let u = make();
        let ins = inputs(&u, &args)?;
        Ok(push(lua, u, ins))
    })?)
}

/// A unit with a leading setting argument (a string or a table).
fn add_set<T: mlua::FromLua + 'static>(lua: &Lua, g: &Table, name: &str, make: impl Fn(Option<T>) -> mlua::Result<Unit> + Send + 'static) -> mlua::Result<()> {
    g.raw_set(name, lua.create_function(move |lua, args: Variadic<Value>| {
        let first = args.first().cloned().unwrap_or(Value::Nil);
        let set = if first.is_nil() { None } else { Some(T::from_lua(first, lua)?) };
        let u = make(set)?;
        let ins = inputs(&u, args.get(1..).unwrap_or(&[]))?;
        Ok(push(lua, u, ins))
    })?)
}

/// A unit fixed by the name, with the signal as first argument.
fn add_fixed(lua: &Lua, g: &Table, name: &str, unit: Unit) -> mlua::Result<()> {
    add_unit(lua, g, name, move || unit.clone())
}

/// A formula unit with fixed inputs, as a function.
fn add_formula(lua: &Lua, g: &Table, name: &str, src: &'static str, vars: &'static [&'static str]) -> mlua::Result<()> {
    add_unit(lua, g, name, move || Unit::Expr { src: src.into(), vars: vars.iter().map(|v| v.to_string()).collect() })
}

fn shape(s: &str) -> mlua::Result<Shape> {
    Ok(match s { "sine" => Shape::Sine, "tri" | "triangle" => Shape::Triangle, "saw" => Shape::Saw, "square" => Shape::Square, "random" => Shape::Random,
        _ => return Err(mlua::Error::runtime(format!("unknown shape {s} (sine, tri, saw, square, random)"))) })
}

fn api(lua: &Lua, g: &Table) -> mlua::Result<()> {
    // Oscillators.
    add_set::<String>(lua, g, "osc", |s| Ok(Unit::Osc { shape: shape(s.as_deref().unwrap_or("sine"))? }))?;
    for (name, sh) in [("sine", Shape::Sine), ("tri", Shape::Triangle), ("saw", Shape::Saw), ("square", Shape::Square), ("steps", Shape::Random)] {
        add_fixed(lua, g, name, Unit::Osc { shape: sh })?;
    }
    add_set::<String>(lua, g, "noise", |c| Ok(Unit::Noise { color: match c.as_deref().unwrap_or("white") {
        "white" => NoiseColor::White, "pink" => NoiseColor::Pink, "brown" => NoiseColor::Brown,
        c => return Err(mlua::Error::runtime(format!("unknown noise {c} (white, pink, brown)"))),
    } }))?;
    add_fixed(lua, g, "dust", Unit::Dust)?;
    // Envelopes and modulation.
    add_set::<Table>(lua, g, "env", |t| {
        let t = t.ok_or_else(|| mlua::Error::runtime("env{ {time, level, curve}, ... }"))?;
        let points = t.sequence_values::<Table>().map(|p| {
            let p = p?;
            Ok(Point { time: p.get::<f32>(1)?, level: p.get::<f32>(2)?, curve: p.get::<Option<f32>>(3)?.unwrap_or(0.0) })
        }).collect::<mlua::Result<Vec<_>>>()?;
        Ok(Unit::Env { points })
    })?;
    add_fixed(lua, g, "adsr", Unit::Adsr)?;
    add_set::<String>(lua, g, "lfo", |s| Ok(Unit::Lfo { shape: shape(s.as_deref().unwrap_or("sine"))? }))?;
    add_fixed(lua, g, "wander", Unit::Wander)?;
    add_fixed(lua, g, "rand", Unit::Rand)?;
    add_fixed(lua, g, "hold", Unit::Hold)?;
    add_formula(lua, g, "decay", "max(1 - t / d, 0) ^ p", &["d", "p"])?;
    add_formula(lua, g, "sweep", "f1 + (f0 - f1) * exp(-t * k)", &["f0", "f1", "k"])?;
    add_formula(lua, g, "hump", "sin(pi * clamp(t / d, 0, 1))", &["d"])?;
    add_formula(lua, g, "after", "step(d, t) * x", &["d", "x"])?;
    add_formula(lua, g, "expdecay", "exp(-t / tau)", &["tau"])?;
    // Math.
    add_fixed(lua, g, "clamp", Unit::Clamp)?;
    add_fixed(lua, g, "map", Unit::Map { exp: false })?;
    add_fixed(lua, g, "mapexp", Unit::Map { exp: true })?;
    g.raw_set("expr", lua.create_function(|lua, (src, vars): (String, Option<Table>)| {
        let mut named: Vec<(String, In)> = Vec::new();
        if let Some(t) = vars { for kv in t.pairs::<String, Value>() { let (k, v) = kv?; named.push((k, sig(&v)?)); } }
        named.sort_by(|a, b| a.0.cmp(&b.0));
        if named.len() > 6 { return Err(mlua::Error::runtime("expr takes at most 6 named inputs")); }
        let vars: Vec<String> = named.iter().map(|n| n.0.clone()).collect();
        super::expr::Expr::compile(&src, &vars).map_err(mlua::Error::runtime)?;
        Ok(push(lua, Unit::Expr { src, vars }, named.into_iter().map(|n| n.1).collect()))
    })?)?;
    g.raw_set("mix", lua.create_function(|lua, xs: Variadic<Value>| {
        let mut acc: Option<In> = None;
        for v in xs.iter() { let x = sig(v)?; acc = Some(match acc { None => x, Some(a) => push(lua, Unit::Add, vec![a, x]).0 }); }
        Ok(S(acc.unwrap_or(In::K(0.0))))
    })?)?;
    g.raw_set("db", lua.create_function(|_, v: f32| Ok(10f32.powf(v / 20.0)))?)?;
    // Filters and effects.
    add_set::<String>(lua, g, "filter", |m| Ok(Unit::Filter { mode: mode(m.as_deref().unwrap_or("low"))? }))?;
    for (name, m) in [("lowpass", FilterMode::Low), ("highpass", FilterMode::High), ("bandpass", FilterMode::Band), ("notch", FilterMode::Notch), ("peak", FilterMode::Peak)] {
        add_fixed(lua, g, name, Unit::Filter { mode: m })?;
    }
    add_fixed(lua, g, "lowpass1", Unit::LowPass1)?;
    add_fixed(lua, g, "highpass1", Unit::HighPass1)?;
    add_fixed(lua, g, "comb", Unit::Comb)?;
    g.raw_set("delay", lua.create_function(|lua, args: Variadic<Value>| {
        // delay(x, time, feedback, mix, max): max is the longest time (s).
        let max = args.get(4).and_then(|v| v.as_f32()).unwrap_or(1.0);
        let u = Unit::Delay { max };
        let ins = inputs(&u, args.get(..4.min(args.len())).unwrap_or(&[]))?;
        Ok(push(lua, u, ins))
    })?)?;
    add_fixed(lua, g, "drive", Unit::Drive)?;
    add_fixed(lua, g, "fold", Unit::Fold)?;
    add_fixed(lua, g, "crush", Unit::Crush)?;
    g.raw_set("modal", lua.create_function(|lua, args: Variadic<Value>| {
        // modal(x, freq, damping, { {ratio, decay, gain}, ... })
        let modes = match args.get(3) {
            Some(Value::Table(t)) => t.sequence_values::<Table>().map(|m| { let m = m?; Ok(Mode { ratio: m.get(1)?, decay: m.get(2)?, gain: m.get::<Option<f32>>(3)?.unwrap_or(1.0) }) }).collect::<mlua::Result<Vec<_>>>()?,
            _ => return Err(mlua::Error::runtime("modal(x, freq, damping, { {ratio, decay, gain}, ... })")),
        };
        let u = Unit::Modal { modes };
        let ins = inputs(&u, &args[..3.min(args.len())])?;
        Ok(push(lua, u, ins))
    })?)?;
    add_fixed(lua, g, "pluck", Unit::Pluck)?;
    // engine("ej3", rpm, throttle, load, cut, gear, speed); engine_fluid(...) for the costly model.
    add_set::<String>(lua, g, "engine", |c| Ok(Unit::Engine { car: c.unwrap_or_default(), fluid: false, spec: Default::default() }))?;
    add_set::<String>(lua, g, "engine_fluid", |c| Ok(Unit::Engine { car: c.unwrap_or_default(), fluid: true, spec: Default::default() }))?;
    add_fixed(lua, g, "tyres", Unit::Tyres)?;
    // The patch itself.
    g.raw_set("param", lua.create_function(|lua, (name, default): (String, Option<f32>)| {
        let mut b = lua.app_data_mut::<Builder>().expect("builder");
        if !b.params.iter().any(|p| p.0 == name) { b.params.push((name.clone(), default.unwrap_or(0.0))); }
        Ok(S(In::P(name)))
    })?)?;
    // length(seconds) or length(param("duration")): when the voice ends.
    g.raw_set("length", lua.create_function(|lua, v: Value| {
        let mut b = lua.app_data_mut::<Builder>().expect("builder");
        match sig(&v)? {
            In::K(s) => b.max_length = s,
            In::P(p) => b.length_param = Some(p),
            In::N(_) => return Err(mlua::Error::runtime("length takes seconds or a param, not a signal")),
        }
        Ok(())
    })?)?;
    g.raw_set("loop", lua.create_function(|lua, ()| { lua.app_data_mut::<Builder>().expect("builder").looping = true; Ok(()) })?)?;
    g.raw_set("sound", lua.create_function(|lua, (name, f): (String, Function)| {
        let outer = lua.remove_app_data::<Builder>().unwrap_or_default();
        lua.set_app_data(Builder { name, ..Default::default() });
        let r = f.call::<Value>(());
        let inner = lua.remove_app_data::<Builder>().unwrap_or_default();
        lua.set_app_data(outer);
        let out = r?;
        let p = finish(inner, &out)?;
        lua.app_data_mut::<Vec<Patch>>().expect("patches").push(p);
        Ok(())
    })?)?;
    Ok(())
}

fn mode(s: &str) -> mlua::Result<FilterMode> {
    Ok(match s { "low" => FilterMode::Low, "high" => FilterMode::High, "band" => FilterMode::Band, "notch" => FilterMode::Notch, "peak" => FilterMode::Peak,
        _ => return Err(mlua::Error::runtime(format!("unknown filter {s} (low, high, band, notch, peak)"))) })
}

fn finish(b: Builder, out: &Value) -> mlua::Result<Patch> {
    let mut b = b;
    let out = match sig(out).map_err(|_| mlua::Error::runtime(format!("{}: return the sound (a signal)", b.name)))? {
        In::N(k) => k,
        // A bare parameter or constant: route it through a node.
        x => { b.nodes.push(Node { unit: Unit::Mul, inputs: vec![x, In::K(1.0)] }); b.nodes.len() - 1 }
    };
    let p = Patch { name: b.name, nodes: b.nodes, out, params: b.params, max_length: b.max_length, looping: b.looping, length_param: b.length_param };
    p.validate().map_err(mlua::Error::runtime)?;
    Ok(p)
}

/// Helpers written in Luau itself, for ports of Godot (44.1 kHz) code.
const PRELUDE: &str = r#"
-- White noise with Godot's density: one uniform sample per 44.1 kHz tick
-- spread over more bandwidth keeps its level per hertz.
function white44() return noise() * expr("sqrt(sr / 44100)") end
-- Godot's one-pole `y = lerp(y, x, k)` at 44.1 kHz, matched at any rate:
-- the exact cutoff, slightly lowered, plus a small direct term (fits the
-- 44.1 kHz response to 0.03 dB up to 20 kHz).
function godot_lp(x, k)
  local s = expr("clamp((sr - 44100) / 3900, 0, 1)")
  local c = expr("(0.025 * k + 0.06 * k * k) * s", { k = k, s = s })
  local fc = expr("-log(1 - k) * 44100 / tau * (1 - 0.025 * k * s)", { k = k, s = s })
  return expr("c * x + (1 - c) * y", { c = c, x = x, y = lowpass1(x, fc) })
end
-- Its high-pass, x - lp(x).
function godot_hp(x, k) return x - godot_lp(x, k) end
"#;

/// Load the patches in a Luau source. `name` names the sound the chunk
/// returns (usually the file stem).
pub fn load(name: &str, src: &str) -> Result<Vec<Patch>, String> {
    let lua = Lua::new_with(StdLib::MATH | StdLib::STRING | StdLib::TABLE, LuaOptions::default()).map_err(|e| e.to_string())?;
    lua.set_memory_limit(64 << 20).map_err(|e| e.to_string())?;
    let steps = Arc::new(AtomicU32::new(2_000_000));
    lua.set_interrupt(move |_| {
        if steps.fetch_sub(1, Ordering::Relaxed) == 0 { return Err(mlua::Error::runtime("step budget used up (a loop that never ends?)")); }
        Ok(VmState::Continue)
    });
    let g = lua.globals();
    let short = |e: mlua::Error| -> String {
        let s = e.to_string();
        s.lines().next().unwrap_or(&s).replace("runtime error: ", "")
    };
    (|| -> mlua::Result<()> {
        let math: Table = g.get("math")?;
        math.raw_set("random", Value::Nil)?;
        math.raw_set("randomseed", Value::Nil)?;
        api(&lua, &g)
    })().map_err(short)?;
    lua.load(PRELUDE).set_name("=prelude").exec().map_err(short)?;
    lua.sandbox(true).map_err(short)?;
    lua.set_app_data(Builder { name: name.to_string(), ..Default::default() });
    lua.set_app_data(Vec::<Patch>::new());
    let out: Value = lua.load(src).set_name(format!("={name}")).eval().map_err(short)?;
    let mut patches = lua.remove_app_data::<Vec<Patch>>().unwrap_or_default();
    let main = lua.remove_app_data::<Builder>().unwrap_or_default();
    if !out.is_nil() { patches.insert(0, finish(main, &out).map_err(short)?); }
    if patches.is_empty() { return Err(format!("{name}: defines no sound (return a signal, or call sound(name, fn))")); }
    Ok(patches)
}

/// Load a `.luau` file; its main sound is named after the file.
pub fn load_file(path: &std::path::Path) -> Result<Vec<Patch>, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("sound");
    load(name, &src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::rms;

    #[test]
    fn luau_patches_build_graphs_with_arithmetic() {
        let src = r#"
            local size = param("size", 1)
            local burst = noise() * env{ {0.001, 1}, {0.06, 0, 4} }
            length(1.0)
            sound("tone", function() return sine(sweep(880, 220, 20)) * decay(0.3, 2) * db(-6) end)
            sound("port", function() return godot_hp(godot_lp(white44(), 0.3), 0.04) end)
            return lowpass(burst, 300 + 4000 * size) - burst / 4
        "#;
        let ps = load("shot", src).unwrap();
        assert_eq!(ps.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["shot", "tone", "port"]);
        let shot = &ps[0];
        assert_eq!(shot.params, vec![("size".to_string(), 1.0)]);
        assert_eq!(shot.max_length, 1.0);
        let x = shot.voice(&[], 1, 48000.0).render_all(1.0);
        assert!(rms(&x[..2400]) > 0.01);
        let t = ps[1].voice(&[], 1, 48000.0).render_all(1.0);
        assert!(t.len() < 30000, "tone ends after its decay: {}", t.len());
        assert!(t.iter().all(|v| v.abs() <= 0.51));
        // A length from a parameter.
        let p = load("hold", "local d = param('duration', 0.5) length(d) return sine(220)").unwrap().remove(0);
        assert_eq!(p.voice(&[("duration", 0.25)], 1, 48000.0).render_all(2.0).len(), 12032);
    }

    #[test]
    fn rand_draws_in_one_voice_are_independent() {
        // Two rand nodes; read one or the other with the same seed.
        let mut pa = load("r", "local a = rand(0, 1) local b = rand(0, 1) return a").unwrap().remove(0);
        pa.out = 0;
        let mut pb = pa.clone();
        pb.out = 1;
        let draw = |p: &Patch, seed: u32| { let mut b = [0.0; crate::synth::BLOCK]; p.voice(&[], seed, 48000.0).render(&mut b); b[0] as f64 };
        let n = 4000;
        let xs: Vec<(f64, f64)> = (0..n).map(|s| (draw(&pa, s), draw(&pb, s))).collect();
        let m = |f: &dyn Fn(&(f64, f64)) -> f64| xs.iter().map(f).sum::<f64>() / n as f64;
        let (ma, mb) = (m(&|x| x.0), m(&|x| x.1));
        let cov = m(&|x| (x.0 - ma) * (x.1 - mb));
        let r = cov / (m(&|x| (x.0 - ma).powi(2)).sqrt() * m(&|x| (x.1 - mb).powi(2)).sqrt());
        assert!(r.abs() < 0.05, "correlation {r}");
        assert!((ma - 0.5).abs() < 0.02 && (mb - 0.5).abs() < 0.02, "{ma} {mb}");
    }

    #[test]
    fn mistakes_come_back_as_short_errors() {
        assert!(load("x", "return lowpass(1, 2, 3, 4)").unwrap_err().contains("takes 3 inputs"));
        assert!(load("x", "return noise('purple')").unwrap_err().contains("unknown noise"));
        assert!(load("x", "while true do end").unwrap_err().contains("budget"));
        assert!(load("x", "return expr('a + nope', {a = 1})").unwrap_err().contains("unknown variable"));
        assert!(load("x", "local a = 1").unwrap_err().contains("defines no sound"));
        assert!(load("x", "return io").is_err());
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore]
    fn shot_cost() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/growing-guns/sounds/shot.luau")).unwrap();
        let p = super::load("shot", &src).unwrap().remove(0);
        let t = std::time::Instant::now();
        let mut n = 0;
        for s in 0..100 { n += p.voice(&[], s, 48000.0).render_all(1.0).len(); }
        let e = t.elapsed().as_secs_f64();
        eprintln!("{:.3} ms per voice-second ({} samples)", e * 1000.0 / (n as f64 / 48000.0), n);
        let (tx, mut mix) = crate::synth::mixer(crate::synth::Layout::mono(), 8, 48000.0);
        let _ = &tx;
        let t = std::time::Instant::now();
        mix.render_seconds(1.0);
        eprintln!("empty mixer: {:.3} ms per second", t.elapsed().as_secs_f64() * 1000.0);
    }
}
