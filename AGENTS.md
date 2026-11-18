# Warp Game Engine

Warp is a Rust game engine for couch games (up to four players on one screen)
built for fast iteration with AI: everything is code and text, simulations
replay bit for bit, rendering runs headless, and a running game can be
inspected and changed over MCP. Its focus is procedural outdoor worlds.
Start with [engine/docs/README.md](engine/docs/README.md) (the docs index),
[engine/docs/overview.md](engine/docs/overview.md) and
[engine/docs/status.md](engine/docs/status.md) (what works now, what is open).

One Rust workspace, with Cargo.toml and Cargo.lock at the repository root.

## Map

| Path | What it is |
| --- | --- |
| `engine/crates/kern` | Engine core: a game is a deterministic `Sim` in fixed ticks; state digests, divergence search, value traces. |
| `engine/crates/render` | The renderer (wgpu): headless by design, renders into textures. Clusters/LOD, shadows, GI, sky and fog, procedural terrain (`terrain_field`, `procedural_terrain`), grass (`meadow*`), trees and vegetation (`vegetation_asset`, `plant_recipe`, `foliage_*`, `voxel_*`), HUD, debug UI widgets. Shaders in `src/shaders/`. Many GPU probes in `examples/`. |
| `engine/crates/app` | The game shell: window, fixed tick, input, split screen, HUD, headless capture and `bench`. Built-in keys: F3 stats overlay unless the game uses F3 (`stats.rs`), F10 debug menu (`debug.rs`), F11 fullscreen, F12 feedback screenshot (`screenshot.rs`). Example games in `examples/` (`terrain_fly`, `hello`). |
| `engine/crates/audio` | Synthesised engine and effect sound. |
| `engine/crates/physics` | Rapier rigid bodies with enhanced determinism. |
| `engine/crates/body` | Bodies that move without hand animation: a standard skeleton, built-in motion-captured clips (CC0), `Gait` (what to blend from what a body does, pace from leg length), rigs of rigid pieces. |
| `engine/crates/assets` | Asset import for scenes. |
| `engine/crates/architecture` | Buildings inside and out from glTF (AtomicBuildings, IFC via IfcConvert): room bake grid, room and outside shots. Guide `engine/docs/architecture.md`. |
| `engine/crates/net` | Online play over iroh (QUIC). |
| `engine/crates/gamenight` | Native Splitscreen Club client (party host, seats, wire contract). |
| `engine/crates/script` | Luau scripting host with hot reload. |
| `engine/crates/live` | Live control of a running game over MCP. |
| `engine/crates/warp` | The `warp` executable: runs games made only of Luau scripts, synth patches and assets. |
| `engine/docs/` | Guides that describe how things work now, `status.md`, and `renderer-switches.md` (every `KERN_*` variable). |
| `engine/docs/log/` | The development log: dated experiments, measurements and session notes (`*-rNN.md` are iterations). Evidence, not current docs. |
| `engine/tools/` | Python diagnostics: frame budgets, hitches, scenario replay. |
| `games/sideways/` | The rally game: `sim` (simulation), `app` (renderer integration, the `cli` package builds the `sideways` executable), `determinism` (`gdcompat`: deterministic math/RNG, not Godot), assets, scenarios, tools. |
| `games/growing-guns/`, `games/spellstorm/` | Rust games with `sim` and `app` crates. Growing Guns is a port of the Godot game (github.com/ontola/growing-guns); its README's status tables say how far the port is: update them in the same commit as any change a player would notice (a test checks modifiers, settings, knobs and the card count). |
| `games/hello-warp/`, `neon-siege/`, `neon-trails/`, `stack-together/` | Luau games run by `warp run games/NAME` (no Rust). |
| `lobbies/game-room/` | Splitscreen Club's lobby game. |
| `website/` | Static site generator for the engine's site. |
| `build/` | Ignored output: screenshots (`build/screenshots`), captures, caches. |

Dependencies point one way: games depend on the engine, never the reverse.
Keep engine APIs independent of Sideways; a game supplies assets and settings.
Engine examples may read Sideways' asset files (textures, `.kmesh` trees) but
copy any game-owned data (such as recipe values) instead of depending on it.

New game code starts in Luau and moves to Rust when it loops over many
things per tick; see [engine/docs/luau-or-rust.md](engine/docs/luau-or-rust.md).

## Running things

```sh
cargo run --release -p app --example terrain_fly -- --style alpine   # procedural terrain flight
cargo run --release -p app --example terrain_fly -- --bench 120       # headless, GPU time per pass
cargo run --release -p cli --bin sideways                             # the rally game
cargo run --release -p warp -- run games/hello-warp                   # a Luau game
```

Feedback loop with a playtester: F4 opens Warp’s feedback overlay. Click or tap a point, or press Esc. The capture writer saves `build/screenshots/<game>-<ms>.png`
plus a `.json` with the game's state, camera, GPU, fps and debug-menu settings,
and copies the image to the clipboard, for every game on the `app` shell
(Sideways included). Each capture is also noted in
`<temp>/warp-live/feedback.jsonl`; the live MCP tool `feedback` (in every
`--live` game and in `warp-mcp`, also with no game running) returns the
newest captures, image and JSON together. `terrain_fly --from <that.json>`
puts the camera back where it was. F10 opens the engine debug menu (render and
light switches, LOD, render height, GPU pass times; games add their own).

For a Sideways F12 bug report, call MCP `feedback` with `diagnostics: false`
to locate the PNG, then use the JSON beside it. Sideways also accepts F12 for the shared F4 overlay. The player clicks or taps the problem, or presses Esc to skip marking. If the JSON has `player_point`, run `warp inspect F12.json` to inspect that point directly. Choose the command by task:

- Identify a tree, car or material under a pixel: `warp inspect F12.json --pixel X Y` returns JSON. Use original PNG coordinates.
- Isolate a lighting artifact on a static surface: `warp relight F12.json --without gi --without shadow_strength` uses the player point or `--pixel X Y`. Read `lighting.json` for source-triangle agreement, repeat stability and brightness. Open `comparison.png` for the crops.
- Reproduce handling or check a physics change: `warp compare F12.json --baseline main --state-only`. Read `comparison.json` for baseline agreement and the first differing tick in the captured window.
- Compare the rendered drive and per-pass timings: omit `--state-only`. The output includes synchronized before/after clips.

[Capture files, prerequisites and headless examples](engine/docs/warp-cli.md#sideways-feedback).
For Growing Guns, inspect `replay.input_history` in the F12 JSON for inputs and tick-stamped live edits. [Growing Guns replay workflow](games/growing-guns/README.md#replay-a-moment-while-editing-graphics).

Procedural terrain has one height function in two copies that must stay
line for line identical: `render/src/terrain_field.rs` (CPU: LOD error,
collision, baking) and `render/src/shaders/terrain_field.wgsl` (GPU patches).
Check parity with `cargo run --release -p render --example procedural_terrain
-- --dump --only NAME` (one shot at a time; the full suite reports false
mismatches). Surface shading lives in `shaders/terrain_surface.wgsl`. The forest
(where trees stand, and what a distant forest looks like) is
`render/src/forest_field.rs` with `shaders/forest_field.wgsl`, also line for
line identical (its `gpu_matches_cpu` test checks them); sample the terrain
at a cut above 0, since `x / 0` in the terrain fade is not portable WGSL. Far
mountain shadows are baked (`procedural_terrain::sun_heightfield` into
`Scene::terrain_sun`); keep heightfield heights at the -1e9 sentinel for
procedural terrain, real coarse heights black out pixels in GI/transmittance.

## Checkouts and build output on shared machines

Mancave (Windows, RTX) is shared by many sessions and its disks are nearly
full. Keep one clone per repository and reuse it:

- `C:\dev\warp-gg` is the only Warp clone. Do not clone again; add a worktree
  for your task with `git -C C:\dev\warp-gg worktree add C:\dev\wt\<topic>`.
  Reuse an existing worktree for the same topic. When your task is done (its
  commits pushed), remove it with `git worktree remove` in the same session.
- Do not copy a checkout into a dated folder as a baseline; use a worktree at
  that commit and remove it afterwards.
- Rust: crate sources are already shared in `%USERPROFILE%\.cargo`. Compiled
  dependencies are shared through sccache (`RUSTC_WRAPPER=sccache`, cache in
  `E:\sccache`, 30 GB cap), so a new worktree's first build reuses them. Each
  worktree keeps its own `target/`; never set a shared `CARGO_TARGET_DIR`,
  parallel builds would block on its lock and overwrite each other's binaries.
- Captures and measurements go to `C:\dev\perf\<YYYY-MM-DD-topic>`. Keep only
  what a log entry needs (a few JPEGs and the numbers; large sequences go to
  the project's shared files) and delete the rest when the task is done.
- Delete only folders your task created. Anything else needs the owner's
  approval.

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

See both READMEs, engine/docs/status.md and, for vegetation,
engine/docs/vegetation-architecture.md.

## Documentation

Docs describe how things work now; the log records how we got there.

- When a change alters how something works, update the guide for that part
  (or write one) in the same commit, and update `engine/docs/status.md` if
  what is default, opt-in or open changed. Rewrite in place: no dated
  "Update:" paragraphs, no "previously" history in a guide.
- Experiments, measurements, rejected candidates and session notes go in
  `engine/docs/log/YYYY-MM-DD-topic.md` (listed in `engine/docs/log/README.md`),
  with the commit, machine, scene and numbers. Never edit an old log entry to
  make it current.
- `engine/HANDOFF.md` is retired; its history is `engine/docs/log/handoff.md`.
  Do not recreate it.
- A new `KERN_*` switch gets a row in `engine/docs/renderer-switches.md`.
  The renderer declares every switch it reads in `render/src/switches.rs` and
  reads it only through that module; a test there fails on a raw
  `var("KERN_...")` or on a declared switch missing from the docs.
