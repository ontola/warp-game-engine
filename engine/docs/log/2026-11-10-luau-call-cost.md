# Luau call costs and Rust rebuild times

2026-11-10, on top of main at 75ebf62. Machine: cloud VM, 4 cores of an
Intel Xeon at 2.1 GHz, Linux, no GPU. The numbers behind
[Luau or Rust](../luau-or-rust.md).

## Crossing costs

`cargo run --release -p script --example call_cost`: best of five runs of
one million operations, inside `script::Host` (sandbox, step-budget
interrupt), then plain mlua for comparison.

| Operation | Host, ns | Plain mlua, ns |
| --- | ---: | ---: |
| Empty Luau loop iteration | 35.1 | 4.1 |
| Luau arithmetic with `math.sqrt` | 118.1 | 50.3 |
| Luau function calling Luau | 123.3 | 30.5 |
| Luau vector add and scale | 38.5 | |
| Luau calling Rust, numbers | 116.1 | 51.5 |
| Luau calling Rust, a vector | 124.6 | |
| Luau calling Rust that returns a new 3-field table | 1,780.6 | |
| One Rust call summing a Luau list, per element (incl. filling it) | 79.9 | |
| Rust calling a Luau hook through `Host::run` | 136.9 | |

The step budget's interrupt (a Rust callback at every loop back-edge and
call) costs about 30 ns per safepoint: 8x on an empty loop, 2 to 4x on
loops that do something. Building a table from Rust costs about 600 ns a
field.

## Rust rebuild times

Dev profile as in Cargo.toml (workspace crates `opt-level = 1`,
incremental; dependencies `opt-level = 3`).

| Build | Time |
| --- | ---: |
| `cargo build -p spellstorm --lib`, cold (Luau C++ included) | 70.6 s |
| Same, after a comment edit in `sim/src/grammar.rs` | 0.7 s |
| `cargo build -p spellstorm-app`, cold except the sim's dependencies | 164.9 s |
| Comment appended to `sim/src/grammar.rs` / `app/src/view.rs` / `render/src/lib.rs` | 2.0 / 2.0 / 5.4 s |
| `pub fn` appended to `sim/src/grammar.rs` | 5.5 s |
| `pub fn` appended to `app/src/view.rs` | 5.4 s |
| `pub fn` appended to `render/src/lib.rs` | 16.5 s |
| `pub fn` appended to `kern/src/lib.rs` | 16.9 s |

Each measured once, warm caches. A desktop with more cores will be faster;
the order between them holds.
