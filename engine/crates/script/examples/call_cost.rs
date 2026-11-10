//! What crossing between Luau and Rust costs, inside the real `script::Host`
//! (sandbox, step-budget interrupt and all). The numbers behind
//! engine/docs/luau-or-rust.md.
//!
//! ```sh
//! cargo run --release -p script --example call_cost
//! ```
//!
//! Each line is nanoseconds per operation, the best of five runs of one
//! million operations. Compare a line with the empty loop to see what the
//! operation itself adds.
use script::mlua::{Function, Table, Value, Vector};
use script::{Host, Limits};
use std::time::Instant;

const N: u32 = 1_000_000;

const SRC: &str = r#"
local function pure(x) return x * 0.5 + 1 end
return {
    -- Work that stays inside Luau.
    empty = function(n) for i = 1, n do end end,
    math = function(n) local s = 0 for i = 1, n do s += math.sqrt(i) * 0.5 end return s end,
    luau_call = function(n) local s = 0 for i = 1, n do s += pure(i) end return s end,
    vector_math = function(n)
        local v = vector.create(0, 0, 0)
        local d = vector.create(1, 2, 3)
        for i = 1, n do v += d * 0.016 end
        return v
    end,
    -- Calls into Rust.
    rust_number = function(n) local s = 0 for i = 1, n do s += twice(i) end return s end,
    rust_vector = function(n)
        local v = vector.create(1, 2, 3)
        for i = 1, n do v = step(v) end
        return v
    end,
    rust_table = function(n) local s = 0 for i = 1, n do s += hit(i).distance end return s end,
    -- One call that hands Rust a whole list.
    rust_batch = function(n)
        local list = table.create(n, 0)
        for i = 1, n do list[i] = i end
        return sum(list)
    end,
    -- Called from Rust once per operation.
    hook = function(x) return x + 1 end,
}
"#;

fn main() {
    let limits = Limits { memory: 512 << 20, steps: u32::MAX };
    let mut host = Host::new(limits, |lua, api| {
        api.raw_set("twice", lua.create_function(|_, x: f64| Ok(x * 2.0))?)?;
        api.raw_set(
            "step",
            lua.create_function(|_, v: Vector| Ok(Vector::new(v.x() + 0.1, v.y(), v.z())))?,
        )?;
        // Like `world.raycast`: a fresh table per answer.
        api.raw_set(
            "hit",
            lua.create_function(|lua, i: f64| {
                let t = lua.create_table()?;
                t.raw_set("id", i)?;
                t.raw_set("distance", i * 0.5)?;
                t.raw_set("point", Vector::new(1.0, 2.0, 3.0))?;
                Ok(t)
            })?,
        )?;
        api.raw_set(
            "sum",
            lua.create_function(|_, t: Table| {
                let mut s = 0.0;
                for v in t.sequence_values::<f64>() {
                    s += v?;
                }
                Ok(s)
            })?,
        )
    })
    .expect("host");
    host.add_source("bench.luau", SRC).expect("load");
    let exports = host.modules()[0].exports.clone().expect("exports");

    let rows = [
        ("empty", "empty Luau loop iteration"),
        ("math", "Luau arithmetic + math.sqrt"),
        ("luau_call", "Luau function call to Luau"),
        ("vector_math", "Luau vector add and scale"),
        ("rust_number", "Luau -> Rust call, number in and out"),
        ("rust_vector", "Luau -> Rust call, vector in and out"),
        ("rust_table", "Luau -> Rust call returning a new table"),
        ("rust_batch", "one Rust call over a list (per element)"),
    ];
    println!("ns per operation, best of 5 x {N}");
    for (name, what) in rows {
        let f: Function = exports.raw_get(name).unwrap();
        let ns = best(|| {
            host.run(0, name, |_| f.call::<Value>(N)).expect("run");
        });
        println!("{ns:7.1}  {what}");
    }
    let hook: Function = exports.raw_get("hook").unwrap();
    let ns = best(|| {
        for i in 0..N {
            host.run(0, "hook", |_| hook.call::<f64>(i as f64)).expect("run");
        }
    });
    println!("{ns:7.1}  Rust -> Luau hook call (Host::run, number in and out)");

    // The same Luau without the host: no step-budget interrupt, no sandbox.
    let lua = script::mlua::Lua::new();
    lua.globals().raw_set("twice", lua.create_function(|_, x: f64| Ok(x * 2.0)).unwrap()).unwrap();
    let bare: Table = lua.load(SRC).eval().expect("bare load");
    println!("without the step budget (plain mlua):");
    for (name, what) in [rows[0], rows[1], rows[2], rows[4]] {
        let f: Function = bare.raw_get(name).unwrap();
        let ns = best(|| {
            f.call::<Value>(N).expect("run");
        });
        println!("{ns:7.1}  {what}");
    }
}

fn best(mut f: impl FnMut()) -> f64 {
    (0..5)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1e9 / N as f64
        })
        .fold(f64::INFINITY, f64::min)
}
