# meshopt 0.7.0, patched for wasm32

A copy of the [meshopt](https://crates.io/crates/meshopt) crate 0.7.0
(MIT OR Apache-2.0, meshoptimizer itself is MIT), used through
`[patch.crates-io]` in the workspace Cargo.toml. The crates.io version leaves
meshoptimizer's cluster LOD builder (`clusterlod.h`, which Warp's cluster
DAG is built with) out of wasm32 builds, because it needs `<vector>` and
`<algorithm>` and wasm32-unknown-unknown has no C++ standard library.

Changes, all of which only matter on wasm32:

- `build.rs` compiles `src/clusterlod.cpp` on every target.
- `src/lib.rs` exposes `clusterlod` on every target.
- `include_wasm32/vector`, `algorithm`, `new`, `initializer_list`: the parts of the standard
  library `clusterlod.h` uses (a vector of trivially copyable elements,
  `std::min`/`std::max`, placement new, brace initialisation).
- `include_wasm32/math.h`: `floorf`, `ceilf`, `log2f`, `ldexpf`, `frexpf`, `cosf`,
  which newer meshoptimizer sources call.

`operator new`/`delete` come from `render/src/web.rs` on wasm32.
Drop this copy when an upstream release builds `clusterlod` for wasm32.
