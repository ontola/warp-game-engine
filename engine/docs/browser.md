# Running in a browser

`terrain_fly` builds for the browser: wasm32 with wgpu's WebGPU backend,
drawn into a canvas. It is a prototype: it loads, builds the world and runs
its frame loop in Chrome. The cull shader now compiles there, but a drawn
frame has not been confirmed since (see Open). It uses the same renderer as the native
build, without threads, gamepads, the F10 menu, screenshots, HDR or
adaptive quality.

## Build and open it

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # the version in Cargo.lock
python3 engine/tools/build-web.py --serve
```

Then open http://localhost:8790 in Chrome or Edge (WebGPU needs localhost or
HTTPS). `?style=alpine` and `?seed=2` pick the world; trees are off unless
`?trees=1` (cooking them freezes the page for minutes, see Open). Hosts
that pass only a fragment get the same from words in it: `#alpine-trees`.
Click to capture the mouse; WASD and the mouse fly, F2 to F9 work as
natively, Esc releases the mouse.

The page shows a progress bar until the first frame: the wasm download
(with bytes), the ground textures, building the world, WebGPU setup and
the first upload. The log sits in the corner.

`build/web` holds `index.html` (from `engine/crates/app/web`), `pkg/` (the
wasm and its JavaScript) and `assets/` (the ground textures). Any static
host serves it.

## Luau games: Splitscreen Club in a browser

`build-web.py --warp` builds Splitscreen Club for the browser: the Game
Room lobby first (`lobbies/game-room/examples/browser.rs`), then the Luau
games on its shelf, all on one engine download. Each Luau game is only its
folder of scripts (Neon Siege 120 KB, Stack Together 46 KB). Build and
serve it:

```sh
sudo apt install wasi-libc libc++-dev-wasm32 libclang-rt-dev-wasm32   # Luau's C and C++ library
python3 engine/tools/build-web.py --warp --serve
```

Open http://localhost:8790/ (`?players=3` for three local players, default
2: the keyboard, then pads in the order they wake up; `?game=neon-siege`
starts in that game). A small host lives in the page instead of a
Splitscreen Club host: the room's commands (Play at the TV or the shelf,
queue, skip, Resume) reach it through `Lobby::outbox`, it loads the game
with `warp::web::load` and swaps it in with `app::web::switch_keeping`,
which keeps the room as it was. Back (Select, Tab on the keyboard) or the
game's own Quit brings the room back (`app::web::on_exit`); Back keeps the
game paused, so the TV offers Resume as at home. The queue starts with
every game and never runs dry (a started or skipped game goes to the end).
The room's light is baked ahead by `game-room bake-light` into
`room-light.bin` next to the page (`Options::light_cache`): baked on the
page's one thread it took 44 s, with the file the room is built in 2 s.
The buttons at the top switch games directly (`switch_game`) and go back
to the room (`back_to_room`). `--bare` builds `examples/warp_web.rs`
(crate `warp`) instead: the games and those buttons, no room. What is
left to do is in [browser-handoff.md](browser-handoff.md).

`--games a,b` picks which `games/` folders are served (default: the six
Luau games); `games/library.json` is the shelf (titles and player counts
from each `gamenight.json`). Measured 20 November 2026 in Chromium on
SwiftShader (no GPU, so frame times there mean nothing): wasm 27.3 MB,
Neon Siege's files fetched in 0.1 to 0.3 s, scripts loaded and `init` run
in 0.2 to 0.4 s; Play to Neon Siege's first frame 1.5 s; switching Stack
Together to Neon Siege took 0.27 s to load plus 0.17 s to its first frame.

How the build differs from native:

- Luau is C++: `build-web.py --warp` compiles it against wasi-libc and
  libc++ for wasm32 (`LUAU_CXXFLAGS`, read by luau0-src) and links one
  `target/web-cxx/libstdc++.a` made of libc++, libc++abi and libc.
  `WASI_SYSROOT` points at a wasi-sdk `share/wasi-sysroot` instead of the
  Debian/Ubuntu packages.
- No C++ exceptions: Luau is compiled with `-fignore-exceptions` and
  `script/src/web.rs` defines `__cxa_throw`, which stops the game with the
  error's message. A Lua error (`error()`, the step limit, a syntax error)
  therefore ends a game in a browser instead of being caught; a game that
  runs cleanly never throws. Real exceptions need a libc++abi built with
  `-fwasm-exceptions` (wasi-sdk does not ship one).
- `script::web::init` runs the C++ static constructors once at start; it
  must come first in `main` (otherwise the linker runs them on every call
  into the module and the page hangs). The same file defines the five WASI
  calls wasi-libc makes (`fd_write` and friends), so the module imports
  nothing from `wasi_snapshot_preview1`.
- `engine/vendor/mlua` is mlua 0.12.2 without its heap dump on wasm32 (it
  used stdio); see its `WARP.md`.
- Game files come through the `vfs` crate (`engine/crates/vfs`):
  `std::fs` natively, in a browser the files the page fetched
  (`vfs::insert`). Writes stay in memory. Game saves (`storage`) are
  in-memory there too (no SQLite).
- `live` (MCP) has no server in a browser; `start` returns an error.
- Glass (transparent surfaces) needs 17 sampled textures per shader stage;
  a browser that offers WebGPU's guaranteed 16 (SwiftShader, many
  GPUs) draws the scene without it and the page logs `no glass`.
- Gamepads come from the Gamepad API (`app/src/web_pads.rs`): pads in the
  standard layout, in the order they first press a button, with
  `vibrationActuator` rumble. A browser shows a pad to the page only after
  one of its buttons is pressed.
- Sound: `app::sound::open` plays through cpal's WebAudio host on the
  page's thread (no audio thread); the example opens one synth mixer for
  the page and gives every game a sender into it. Browsers start audio only
  after a click or key press on the page (a gamepad button does not count):
  the page remembers each AudioContext and resumes it then. While the page
  thread is busy (a long frame) the sound can stutter; an AudioWorklet would
  take it off that thread.

## What the browser needs

- Engine rule (Joep, 6 November 2026): every pass fits 8 storage buffers
  per shader stage, the spec minimum. Data a pass reads is packed into a few
  shared buffers read through WGSL accessor functions (`scene_tables.rs`,
  `topology.rs`, `shade_scratch.rs`, `cull_tables.rs`, the cull `CullDraw`
  binding); optional features keep their own bind group. Check a change with
  `KERN_WEBGPU_LIMITS=spec` ([renderer switches](renderer-switches.md)).
- The GPU forest fits eight storage buffers per stage. Its preparation,
  integration, emission, and surface passes bind their own inputs. The
  budgeted refinement mode still requires ten. The default refinement
  path and both sun splat pipelines fit eight.
- At WebGPU's guaranteed sixteen sampled textures the renderer uses compact
  material lighting. Terrain, textured meshes, bark, voxel trees, and
  directional shadows use the GPU scene and its visibility buffer. The
  wider material layout enables the full lighting effects on devices that
  support it. `KERN_WEBGPU_LIMITS=spec` and `=mac` exercise those limits on
  a native adapter. These runs do not exercise a browser's shader compiler.
- Devices below eight storage buffers or sixteen sampled textures can use
  the three-buffer procedural terrain renderer. `KERN_PORTABLE=1` selects
  it explicitly.
- Optional features are used when there: timestamps; `primitive_index`
  (meadow visibility) has a fallback. There is no pipeline cache.

## How the build differs

- `app::web` is the shell: it takes the canvas `#warp` (or adds one),
  requests the adapter and device asynchronously
  (`Gpu::for_surface_async`), and draws on the browser's frame callbacks.
  The canvas has no sRGB format, so the shell renders into an sRGB view of it.
- No threads: `render::par::scope` is `std::thread::scope` natively and runs
  each spawned job at once in a browser. `terrain_fly` builds scenes,
  forests and sun bakes inline there (`background`), which stalls the page
  while it works; `app::web::yield_to_browser` lets the progress bar paint
  before each long step.
- Time comes from `web_time::Instant` (`std::time::Instant` natively).
- Files: the page fetches the textures first and hands them to
  `render::files::preload`; `render::files::read` and `open_image` find them
  by file name. Natively these read the filesystem.
- meshoptimizer's cluster LOD builder is C++ that needs parts of the C++
  standard library; `engine/vendor/meshopt` supplies them on wasm32 (see its
  `WARP.md`), and `render/src/web.rs` supplies `operator new`/`delete`.

## Open

- Trees: their cooked assets are cached on disk natively (`build/cook-cache`,
  about 260 MB for one forest); a browser cooks them at every load.
- Shaders are compiled by the browser's own WGSL compiler (Tint in Chrome),
  which runs WGSL's uniformity analysis; naga does not. In `cull.wgsl` lane 0
  decides whatever guards a barrier and shares it with
  `workgroupUniformLoad`. Other modules have not been checked the same way.
  Test in Chrome after changing a compute shader with barriers.
- Tint also follows WGSL's grammar where naga is lenient: operators the
  grammar keeps apart need parentheses (`(a * b) ^ c`, not `a * b ^ c`;
  likewise `&` with `|`, a bitwise operator or shift with anything else,
  chained comparisons, `&&` with `||`). One such expression in shade made
  the whole lighting pass invalid in every browser. `render::wgsl_lint`
  checks every `.wgsl` in the repository and the renderer's inline WGSL for
  it in the normal test run (`cargo test -p render --lib wgsl_lint`).
- Measurements and the feasibility check:
  [log entry](log/2026-11-05-browser-feasibility.md).
