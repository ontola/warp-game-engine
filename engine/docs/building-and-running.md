# Building and running

Everything runs from the repository root. You need a stable Rust toolchain
(edition 2024). On Linux the workspace links with clang and lld (see
`.cargo/config.toml`); install both. A first clean dev build is long
(minutes on a small machine: wgpu, naga and the font stack dominate); keep
`target/` between sessions.

## Check and test

```sh
cargo check --locked --workspace
cargo test --locked -p kern
cargo test --locked -p render --lib
cargo test --locked -p audio --lib
cargo test --locked -p script
cargo test --locked -p growing-guns
cargo test --locked -p cli --bin sideways
```

When you filter test output through a pipe, set `set -o pipefail`, or a
failing `cargo test` looks green.

## Run the games

Growing Guns:

```sh
cargo run -p growing-guns-app -- play                  # 1 player + 3 bots
cargo run -p growing-guns-app -- play --players 4      # split screen, pads
cargo run -p growing-guns-app -- play --coop           # wave survival
```

Sideways:

```sh
cargo run --release -p cli -- play --views 1
cargo run --release -p cli -- play --views 4 --size 3840x2160
```

On Windows, `games/sideways/tools/play-dev-windows.ps1 -Restart` builds
while an owned playtest runs and restarts it after a successful build. See
the [Sideways README](../../games/sideways/README.md).

The smallest example:

```sh
cargo run -p app --example hello
```

## Headless and software runs

The renderer never opens a window unless a game asks for one. Every game
command except `play` renders offscreen.

`KERN_ADAPTER` picks the GPU adapter:

- unset: the best adapter there is (high performance);
- `software`: the fallback adapter, a CPU rasteriser (lavapipe/llvmpipe on
  Linux, WARP on Windows);
- any other value: the first adapter whose name contains it
  (`KERN_ADAPTER=nvidia`).

With the software adapter, frames are slow but deterministic on one
machine, and they need no GPU and no display. This is how changes are
checked in a container or in CI.

```sh
# One frame of the hello example
KERN_ADAPTER=software cargo run -p app --example hello -- --shot hello.png

# One Growing Guns frame, four views
KERN_ADAPTER=software cargo run -p growing-guns-app -- shot --out s.png --views 4 --seconds 4

# One Sideways frame at tick 900 (TAA converged over 4 frames)
KERN_ADAPTER=software cargo run -p cli -- shot shot.png --size 960x540 --frames 900 --taa 4

# The simulation alone, with a trace of queried values
cargo run -p growing-guns-app -- sim --seconds 120 --trace players.0.health,round
cargo run --release -p cli -- sim --ticks 45000 --seed 1 --km 2 --trace players.0.kmh
```

`sideways shot` writes a JSON file next to the image with what was drawn
and what it cost.

## Scripted playtests

A script is a JSON file of steps. `app::script` runs it against any
`app::Game` headless: it ticks the game with controls, runs debug
commands, saves frames and state dumps, records audio, and checks values.

```json
{ "size": [640, 360],
  "steps": [
    { "ticks": 90, "mv": [0, 1], "note": "roll forward into the first gem" },
    { "expect": "/score", "min": 1 },
    { "press": ["SOUTH"], "ticks": 10, "note": "jump" },
    { "capture": "jump.png", "compare": "hello-jump.png", "tolerance": 2.0, "state": "jump.json" }
  ] }
```

The step fields:

| Field | Meaning |
| --- | --- |
| `ticks` | Ticks to run (default 1 when the step has controls). |
| `player` | Which local player the controls are for (default 0); others idle. |
| `mv`, `look` | Stick values, held every tick. `look` is radians per tick, x right and y up. |
| `hold`, `press` | Button names (`SOUTH`, `RIGHT_TRIGGER`...). `press` applies on the first tick only. |
| `cmd` | A debug command or a list of them (`Game::command`), run before the ticks. |
| `capture` | Save the frame as PNG into the output directory. |
| `compare`, `tolerance` | With `capture`: compare against a reference PNG (relative to the script). Fails when the mean absolute difference per channel (0 to 255) exceeds `tolerance` (default 1.0). A missing reference is written. |
| `state` | Save `Game::state()` as JSON. |
| `expect` | A JSON pointer (RFC 6901) into the state, checked with `equals`, or within `min`/`max`, or (with neither) that it exists. |
| `until`, `max_ticks` | Tick until such a check passes, at most `max_ticks` (default 3600). Not reaching it fails. |
| `audio` | Write the sound mixed since the previous `audio` step as a WAV and report its peak and RMS. |
| `note` | Free text, ignored. |

Every step prints one JSON line. A failed expectation fails the run.

```sh
cargo run -p app --example hello -- --script engine/crates/app/examples/hello.json --out /tmp/hello
KERN_ADAPTER=software cargo run -p growing-guns-app -- script games/growing-guns/scripts/smoke.json --out /tmp/gg
```

The hello script takes about 2 s on a CPU rasteriser and gives a
difference of 0.0 between runs. Growing Guns has scripts for rounds,
cards, mods, destruction, coop and sound in `games/growing-guns/scripts/`.

## Golden images

A golden image is a reference capture checked into the repository. A
script step with `compare` fails when the frame drifts from it.

Growing Guns keeps six (arena, stands, lava, the grown gun, a blast and
gibs) in `games/growing-guns/scripts/golden/`, made by
`scripts/golden.json`:

```sh
games/growing-guns/tools/golden.sh            # check (debug build)
games/growing-guns/tools/golden.sh --bless    # replace the references
```

The references are made with the software adapter. Another GPU or driver
renders slightly differently, so compare like with like.

## Environment variables

| Variable | Effect |
| --- | --- |
| `KERN_ADAPTER` | GPU adapter choice, see above. |
| `KERN_SHADER_DIR` | Read the shading chain's `.wgsl` files from this directory instead of the built-in copies (normally `engine/crates/render/src/shaders`). Missing files fall back to the built-in ones. `Renderer::reload_shaders` reloads them without a rebuild. |
| `KERN_SHADE_FEATURES=all` | Compile every shading path instead of only those the scene's materials reach. For checking that specialisation does not change images. |
| `KERN_STARTUP_PROFILE=1` | CPU attribution of renderer construction. |
| `KERN_CPU_SPANS=1` | Average CPU time per frame for each phase of frame output. |
| `WARP_HDR=1` | HDR output on an HDR screen, for games on the `app` shell (`0` refuses it). See [Exposure and HDR output](hdr-and-exposure.md). |
| `WARP_HDR_HEADROOM=4` | HDR highlight headroom (times SDR white) instead of what the display reports. |
| `GG_MODS=DIR` | Growing Guns: load mods from another folder. |
| `GG_CHARACTERS=off` | Growing Guns: primitive players instead of the animated characters. |
| `SIDEWAYS_GAME_ROOT` | Sideways: the game root with `assets/` and `data/`. |

Many more `KERN_*` switches turn experimental renderer paths on or off;
they are described in the engineering notes that introduced them.

## Measuring

- GPU timestamps per pass: `Renderer::set_async_gpu_profiling`; Growing
  Guns' `bench` shows how (`bench --views 4 --size 3840x2160`). Minimum rendering
  reports geometry, FX and sprites per view with inside-encoder timestamps;
  [Quest profiling](quest-profile-2026-10-31.md) documents the opt-in live capture
  and paired lava/HUD probes. `HudStats::phase_ms` separates CPU upload, shaping,
  glyph warmup, outlines and atlas preparation. Low-end games can opt into
  `HudDrawList.cached_text` for animated labels: bounded fixed-size label
  pictures scale as quads, trading a little texture memory/quad fill for
  avoiding repeated glyph rasterization at continuously changing sizes.
  Custom strokes and colour glyphs fall back to the regular text renderer.
- Sideways scenarios: `engine/tools/run-frame-scenario.py` with
  `games/sideways/scenarios/*.toml`, then `frame-budget.py`. See the
  [frame budget guide](../tools/frame-budget.md).
- Report the real resolution, reconstruction mode, sample count and scene
  with every number. CPU wall time and GPU time are separate quantities.
