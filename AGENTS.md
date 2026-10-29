# Warp Game Engine

Warp is a Rust game engine for couch games (up to four players on one screen)
built for fast iteration with AI: everything is code and text, simulations
replay bit for bit, rendering runs headless, and a running game can be
inspected and changed over MCP. Its focus is procedural outdoor worlds.
Start with [engine/docs/README.md](engine/docs/README.md) (the docs index) and
[engine/docs/overview.md](engine/docs/overview.md).

One Rust workspace, with Cargo.toml and Cargo.lock at the repository root.

## Map

| Path | What it is |
| --- | --- |
| `engine/crates/kern` | Engine core: a game is a deterministic `Sim` in fixed ticks; state digests, divergence search, value traces. |
| `engine/crates/render` | The renderer (wgpu): headless by design, renders into textures. Clusters/LOD, shadows, GI, sky and fog, procedural terrain (`terrain_field`, `procedural_terrain`), grass (`meadow*`), trees and vegetation (`vegetation_asset`, `plant_recipe`, `foliage_*`, `voxel_*`), HUD, debug UI widgets. Shaders in `src/shaders/`. Many GPU probes in `examples/`. |
| `engine/crates/app` | The game shell: window, fixed tick, input, split screen, HUD, headless capture and `bench`. Built-in keys: F3 debug menu (`debug.rs`), F11 fullscreen, F12 feedback screenshot (`screenshot.rs`). Example games in `examples/` (`terrain_fly`, `hello`). |
| `engine/crates/audio` | Synthesised engine and effect sound. |
| `engine/crates/physics` | Rapier rigid bodies with enhanced determinism. |
| `engine/crates/assets` | Asset import for scenes. |
| `engine/crates/net` | Online play over iroh (QUIC). |
| `engine/crates/gamenight` | Native GameNight client (party host, seats, wire contract). |
| `engine/crates/script` | Luau scripting host with hot reload. |
| `engine/crates/live` | Live control of a running game over MCP. |
| `engine/crates/warp` | The `warp` executable: runs games made only of Luau scripts, synth patches and assets. |
| `engine/docs/` | Design notes and measurement reports (`*-rNN.md` are dated iterations; read the index first). |
| `engine/tools/` | Python diagnostics: frame budgets, hitches, scenario replay. |
| `engine/HANDOFF.md` | Running log of the latest state per topic, newest first. |
| `games/sideways/` | The rally game: `sim` (simulation), `app` (renderer integration, the `cli` package builds the `sideways` executable), `determinism` (`gdcompat`: deterministic math/RNG, not Godot), assets, scenarios, tools. |
| `games/growing-guns/`, `games/spellstorm/` | Rust games with `sim` and `app` crates. |
| `games/hello-warp/`, `bumper-royale/`, `neon-siege/`, `neon-trails/` | Luau games run by `warp run games/NAME` (no Rust). |
| `lobbies/game-room/` | GameNight's lobby game. |
| `website/` | Static site generator for the engine's site. |
| `build/` | Ignored output: screenshots (`build/screenshots`), captures, caches. |

Dependencies point one way: games depend on the engine, never the reverse.
Keep engine APIs independent of Sideways; a game supplies assets and settings.
Engine examples may read Sideways' asset files (textures, `.kmesh` trees) but
copy any game-owned data (such as recipe values) instead of depending on it.

## Running things

```sh
cargo run --release -p app --example terrain_fly -- --style alpine   # procedural terrain flight
cargo run --release -p app --example terrain_fly -- --bench 120       # headless, GPU time per pass
cargo run --release -p cli --bin sideways                             # the rally game
cargo run --release -p warp -- run games/hello-warp                   # a Luau game
```

Feedback loop with a playtester: F12 writes `build/screenshots/<game>-<ms>.png`
plus a `.json` with the game's state, camera, GPU, fps and debug-menu settings,
and copies the image to the clipboard. `terrain_fly --from <that.json>` puts
the camera back where it was. F3 opens the engine debug menu (render and
light switches, LOD, render height, GPU pass times; games add their own).

Procedural terrain has one height function in two copies that must stay
line for line identical: `render/src/terrain_field.rs` (CPU: LOD error,
collision, baking) and `render/src/shaders/terrain_field.wgsl` (GPU patches).
Check parity with `cargo run --release -p render --example procedural_terrain
-- --dump --only NAME` (one shot at a time; the full suite reports false
mismatches). Surface shading lives in `shaders/terrain_surface.wgsl`. Far
mountain shadows are baked (`procedural_terrain::sun_heightfield` into
`Scene::terrain_sun`); keep heightfield heights at the -1e9 sentinel for
procedural terrain, real coarse heights black out pixels in GI/transmittance.

## Rules

Preserve attribution, licenses and unrelated work. Never reset or stash another
task's changes. Check absolute paths and dependencies before moves/deletions.
Commit and push straight to `main` (no pull requests); never force push.

Physics stays at 120 Hz with four substeps. Reorganization must preserve
simulation output and retained asset bytes. The Godot implementation, export
pipeline and comparison workflow are retired.

Games are played on 120 Hz screens while most simulations tick at 60 Hz:
anything that moves must be drawn between its last two ticks with
`Frame::alpha`, or it looks blurred and doubled. Objects in `render::slots`
get this from `Slots::tick` (start of every tick) plus `Slots::flush_at`
(draw); cameras and anything else blend prev/cur themselves. The `warp`
Luau runtime does both for every Luau game.

Someone may be playing on the machine you run on. Measure and verify headless
(`--shot`, `--dump`, `--bench`, unit tests, render examples); open a game
window only when a person asks to play, and never start, focus or kill windows
you did not open.

## Validation and iteration

Run focused checks appropriate to the change:

```sh
cargo check --locked --workspace --all-targets
cargo test --locked -p render --lib
cargo test --locked -p app --lib
cargo test --locked -p cli --bin sideways
games/sideways/tools/test-play-dev.ps1              # Windows
games/sideways/tools/play-dev-windows.ps1 -Restart  # Windows
```

Run GPU fixtures serially without a simultaneous game. Stop only an owned
process, verified by executable path, PID and creation time. Keep a playtest
running during compilation where possible; a build is not proof of presentation.

Use engine/tools/run-frame-scenario.py with games/sideways/scenarios for replay.
JPEG previews are default; full PNG is for pixel tests and F12. Exclude warmup,
startup and readback from steady-frame results. CPU wall time and GPU time
are separate. Geometry omission deltas are non-additive. On macOS (Metal) a
fullscreen window in another Space is not presented, so its fps is meaningless;
use headless GPU pass times instead.

Target: one 4K output, four players, 120 FPS (8.33 ms); not reached. Report actual
resolution, reconstruction, sample count and scene. No further Godot comparison.

See both READMEs, engine/HANDOFF.md and engine/docs/forest-rendering-roadmap.md.
