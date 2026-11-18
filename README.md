# Warp

Warp is a Rust game engine for couch games (one to four players on one
screen) and procedural outdoor worlds. It is built for fast iteration with
AI: everything is code and text, simulations replay bit for bit, rendering
runs headless, and a running game can be inspected and changed over MCP.

It is young and moves fast. [Status](engine/docs/status.md) says per area
what is the default, what is opt-in and what is still open.

## What is in it

- **A deterministic core.** A game is a `kern::Sim` stepped in fixed ticks.
  State digests find the first tick where two runs differ, and value traces
  sample anything over time. Scripted playtests and replays build on this.
- **A GPU-driven renderer** on wgpu: cluster LOD, two-phase occlusion
  culling, a visibility buffer with compute shading, TAA and a portable
  temporal upscaler (DLSS where available), in physical units with AgX
  tone mapping. It renders headless into textures by design.
- **Procedural outdoor worlds.** One terrain height function with
  line-for-line CPU and WGSL copies, regions, zones and artist overrides,
  ditches, streams and rivers, and a GPU forest and grass route with a
  Nanite-style error-driven cut over streamed voxel pages.
- **Light.** Procedural sky with multiple scattering, volumetric clouds,
  aerial perspective, cached sun clipmap shadows (eight levels, 24 m to
  3 km), traced reflections, baked multi-bounce light for rooms in
  buildings, and opt-in froxel fog, water optics, depth of field, motion
  blur and HDR output.
- **Luau first.** Game rules and content go in hot-reloaded, sandboxed
  Luau; loops over many things go in Rust, per-pixel work on the GPU
  ([Luau or Rust](engine/docs/luau-or-rust.md)). The `warp` executable runs
  games made only of Luau scripts, synth patches and glTF assets, with no
  compiling.
- **The rest of a game:** Rapier physics with enhanced determinism, bodies
  that move from motion-captured clips and gaits, Warp Humans (people and
  crowds), synthesised audio, GPU particles, online play over iroh, and a
  native [Splitscreen Club](https://gamenight.ontola.io) client.
- **Tools in every game:** F3 stats overlay, F4 feedback capture (state,
  camera and GPU timings beside the PNG, replayable with `warp inspect`,
  `warp relight` and `warp compare`), F10 render debug menu and a
  [render suite](engine/docs/render-suite.md) for before/after timings.

It runs natively on Windows (Vulkan) and macOS (Metal), and on Linux,
including a CPU rasteriser for machines without a GPU. Growing Guns has a
native Quest build. A [browser build](engine/docs/browser.md) of
`terrain_fly` (wasm32 and WebGPU) is a prototype; every pass keeps to the
WebGPU minimum of 8 storage buffers per shader stage so it can get there.

## Games

| Game | What it is | Made in |
| --- | --- | --- |
| [Sideways](games/sideways/README.md) | Gravel rally for up to four players | Rust |
| [Growing Guns](games/growing-guns/README.md) | Arena shooter ported from Godot, with a Quest build | Rust |
| Spellstorm | Co-op wizard roguelite (port in progress) | Rust + Luau mods |
| [SpaceRacer](games/spaceracer/README.md) | Native development port of the racer | Rust + Luau rules |
| [Dead Miles](games/dead-miles/README.md) | Co-op zombie walk east, by night | Luau |
| [Neon Siege](games/neon-siege/README.md), [Neon Trails](games/neon-trails/README.md), [Stack Together](games/stack-together/README.md) | Party games ported from LÖVE | Luau |
| [Painted Meadow](games/painted-meadow/README.md) | Stylised walk with its own lighting and finish shaders | Luau |
| [Pine Lab](games/pine-lab/README.md) | Jeffrey pines regrown live as growth rules change | Luau |
| [Human Lab](games/human-lab/README.md) | Warp Humans up close: a row of people and fixed portrait cameras | Luau |
| [Hello Warp](games/hello-warp/README.md) | The starter that `warp new` writes | Luau |
| [Game Room](lobbies/game-room/README.md) | Splitscreen Club's lobby | Rust |

## Quick start

You need stable Rust (edition 2024); on Linux also clang and lld. Everything
runs from the repository root.

```sh
cargo run --release -p app --example terrain_fly       # fly over procedural terrain
cargo run --release -p warp -- run games/dead-miles    # a Luau game, hot reload included
cargo run --release -p warp -- new my-game             # start your own
cargo run --release -p cli --bin sideways              # Sideways
cargo run --release -p growing-guns-app -- play        # Growing Guns, 1 player + 3 bots
```

Add `--live` to a `warp run` to let Claude change the running game over MCP
([live control](engine/docs/live-control.md)). Renderer experiments are
behind `KERN_*` variables, listed with their defaults in
[renderer switches](engine/docs/renderer-switches.md).

## Documentation

- [Documentation index](engine/docs/README.md): every guide.
- [Building and running](engine/docs/building-and-running.md): checks, tests,
  software rendering, scripted playtests, golden images.
- [Games in Luau](engine/docs/warp-cli.md) and
  [making a game in Rust](engine/docs/making-a-game.md).
- [Development log](engine/docs/log/README.md): dated experiments and
  measurements. [`website/`](website/README.md) builds the docs into a site.
- Agents: read [AGENTS.md](AGENTS.md).

## Layout

One Cargo workspace and lockfile at the root. Engine crates are in
[`engine/crates/`](engine/README.md), games in `games/`, the lobby in
`lobbies/`. Games depend on the engine, never the reverse.

The performance target is four players in one 4K output at 120 FPS. It is
not reached yet.

[Third-party asset credits](games/sideways/CREDITS.md).
