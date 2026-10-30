# What Warp is

Warp is a Rust game engine. It started as the engine for
[Sideways](../../games/sideways/README.md), a gravel rally game for up to
four local players, and now also runs
[Growing Guns](../../games/growing-guns/README.md), a first-person arena
shooter ported from Godot. Both games live in the same Cargo workspace as
the engine.

Its focus is realistic, performant procedural outdoor worlds (terrain,
trees, grass, sky), vehicle physics and short iteration loops for people
and for coding agents. It is young: a human editor UI, complete indoor
rendering, water, buildings, people and general animation are missing.
Priorities can change.

## Design goals

### A deterministic simulation

A game is a `kern::Sim`: a state advanced in fixed ticks, where the same
inputs from the same state give the same next state, bit for bit. The
simulation lives in its own crate without rendering. Sideways steps at
120 Hz with four substeps; Growing Guns at 60 Hz.

Determinism is what the rest is built on:

- `kern::Digest` hashes a state (floats by their bits) and
  `kern::first_divergence` finds the first tick where two runs differ.
- `kern::Trace` samples any queried value (`players.0.health`) over time
  as a compact table, through `Sim::query`, without adding a print and
  rebuilding.
- Scripted playtests replay exactly, so they can be tests.
- Online play is host-authoritative with input sequence numbers; Luau mod
  hooks run inline in the host's tick, so a match with mods still replays.
- Scripts get no clock and no `math.random`; the game hands out time and
  seeded random numbers.

### Fast iteration

The loop is edit, build, look, measure, and every step is kept short:

- The dev profile builds dependencies once with full optimisation and the
  workspace crates with light optimisation, so a change in game code
  rebuilds in seconds and still plays (see `[profile.dev]` in the root
  `Cargo.toml`). Release (thin LTO) is for final timings.
- The main shade pipeline is specialised to the scene's materials, so a
  scene only compiles the shading paths it can reach.
- The shading chain can be read from disk (`KERN_SHADER_DIR`) and reloaded
  without a rebuild.
- Luau mods hot-reload while the game runs.
- `render::slots::Slots` lets objects come, go and change mesh without
  rebuilding the renderer.

Measured numbers are in [second-game-findings.md](second-game-findings.md)
and [iteration-speed-2026-10-24.md](iteration-speed-2026-10-24.md).

### Headless first

The renderer renders into textures and reads them back; it never opens a
window unless a game asks. The same frame path runs on a desktop GPU and on
a CPU rasteriser (`KERN_ADAPTER=software`), so frames, scripted playtests
and golden-image tests run in a container or in CI. See
[building and running](building-and-running.md).

### Physical units

Sun and sky are in lux, exposure in EV100, emissive materials in nits,
distances in metres. `Lighting::default()` is a sunny day. HDR output goes
through AgX tone mapping.

### Live tweaking by an AI (a direction)

A stated direction for Warp is that an AI agent, including an LLM in the
loop, can look at a running game and change it while it runs. This is a
direction, not a finished feature. What exists today are the pieces it
builds on:

- `Game::state()` returns the game's state as JSON, and `Game::command`
  takes debug commands (`give 0 explosive`, `teleport 0 1 2 3`).
- JSON scripts drive a game headless, save frames and state, and fail on a
  broken expectation.
- Growing Guns' console runs Luau against the live match
  (`lua g:give(1, 'explosive')`), and its rules and cards are Luau files
  that reload on save.
- Sideways has a live render debug panel (F3) and `shot --watch`, which
  re-renders when the car material data or shading shaders change.

The `live` crate ties these together: a game started with `--live`
serves MCP on localhost, so Claude or ChatGPT can read its state, take
screenshots, run Luau, edit mods, sound patches and shaders, and use the
game's own tools while you play. See [live control](live-control.md).

## Not goals (for now)

- An editor. Games are code and data files.
- Comparing against Godot. The Godot versions are retired as references.
- Claiming the frame-rate target before it is measured. The Sideways target
  is four players on one 4K output at 120 FPS (8.33 ms per frame); it is
  not reached. See [rendering](rendering.md).

### Gameplay attachments to feedback screenshots

The app shell calls `Game::feedback_replay` with the F12 PNG path and its
interpolation phase. A game may attach a replay using the same basename; its
metadata or error is included beside the screenshot. Growing Guns implements
a bounded state recorder and review UI (see its README). Games without this
hook retain screenshot-only behavior. `Headless::frame_at` renders a specified
sub-tick phase for repeatable captures.
