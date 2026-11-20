# mlua 0.12.2, patched for the browser

A copy of the [mlua](https://crates.io/crates/mlua) crate 0.12.2 (MIT), used
through `[patch.crates-io]` in the workspace Cargo.toml so that `warp` (Luau
games) builds for wasm32-unknown-unknown. Only `src`, `Cargo.toml`, the
licence and README are kept; the `[[test]]`, `[[bench]]` and `[[example]]`
targets and dev-dependencies are dropped from `Cargo.toml`.

Changes, which only matter on wasm32:

- `src/luau/heap_dump.rs`: `HeapDump::new` returns `None` on wasm32 (it used
  `libc::tmpfile` and stdio, which the libc crate does not have there).
- `src/lib.rs`: unused-code warnings are allowed on wasm32 (the heap dump's
  helpers).

Luau's C++ itself is compiled by `engine/tools/build-web.py` (see
`engine/docs/browser.md`, "Luau").
