# Running in a browser

`terrain_fly` builds for the browser: wasm32 with wgpu's WebGPU backend,
drawn into a canvas. It is a prototype: it loads, builds the world and runs
its frame loop in Chrome. The cull shader now compiles there, but a drawn
frame has not been confirmed since (see Open). It uses the same renderer as the native
build, without threads, gamepads, the F10 menu, screenshots, HDR or
adaptive quality.

## Small updates for a dedicated playtest

After building a single-game Game Room shelf, package it against a local copy of
the exact previous deployed shelf. Use separate directories. For example:

```sh
python engine/tools/cache_web_shelf.py build/web --previous ../previous-web --report ../playtest-bytes.json
```

The packer keeps existing HTTP cache keys for identical assets and assigns each
new or edited asset its SHA-256 key. The page can fetch smaller gzip copies of
the engine and large game files from any static server. It checks the browser's
existing raw-file cache first. The byte report records both build stamps, the
game's file count and sizes, the cold engine transfer, and the changed game
payload. Other HTTP content encoding and browser cache residency can reduce the
actual transfer. Keep the previous shelf for the next packaging run.

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
Room lobby first (`lobbies/game-room/examples/browser/main.rs`), then the Luau
games on its shelf, all on one engine download. Each Luau game is only its
folder of scripts (Neon Siege 120 KB, Stack Together 46 KB). Build and
serve it:

```sh
sudo apt install wasi-libc libc++-dev-wasm32 libclang-rt-dev-wasm32   # Luau's C and C++ library
python3 engine/tools/build-web.py --warp --serve
```

Open http://localhost:8790/ (`?players=3` for three seats at the start,
default none: until someone joins, the page asks for a controller or the
QR code; `?game=neon-siege` starts in that game; `?cloud=URL` names the
Splitscreen Club cloud, `?cloud=off` plays without one). A small host lives in the page instead of a
Splitscreen Club host: the room's commands (Play at the TV or the shelf,
queue, skip, Resume) reach it through `Lobby::outbox`, it loads the game
with `warp::web::load` and swaps it in with `app::web::switch_keeping`,
which keeps the room as it was, its scene still on the GPU
(`app::web::Parked`). Back (Select, Tab on the keyboard) or the game's own
Quit brings the room back with `app::web::resume` (`app::web::on_exit`):
its first frame comes 5 to 10 ms after Back on an M5 Pro, where uploading
the room again took 5 s. Back keeps the game paused, also on the GPU, so
the TV offers Resume as at home (and starting that game again resumes it,
its first frame in about 10 ms). The queue starts with
every game and never runs dry (a started or skipped game goes to the end).
The room's light is baked ahead by `game-room bake-light` into
`room-light.bin` next to the page (`Options::light_cache`): baked on the
page's one thread it took 44 s, with the file the room is built in 2 s.

Seats (`examples/browser/main.rs`, `app::web::on_devices`): a seat is free
until a device presses a button or moves its stick; then that device
drives it, in the room and in every game after (the shell routes each
game's seats through the page unless the game binds its own). A device
with no free seat joins as a new player (up to 8), in the room at once and
in the next game. When a seat's pad disconnects, its controller leaves the
party snapshot: the room shows the player asleep with "no controller", and
after its usual 30 s (while the room is on screen) sends `leave_party`. A
player who joined then leaves the table; one of the `?players=` seats (or
a joined one between others) stays as a free seat in its own look. The
same pad coming back in time wakes its player.

Pads (`app/src/web_pads.rs`): Chrome reports most pads with the
`standard` mapping. Others, like an 8BitDo Ultimate over Bluetooth
(`2dc8:3012`: 16 buttons, 10 axes, the d-pad as a hat on axis 9), report
none, and `layout` maps their buttons, sticks and hat by vendor id. Every
connected pad counts.

In a game: a game's own menu (Start, Esc) offers "Back to lobby" in place
of Quit when the page hosts it (`app::host_takes_back`, `Game::take_back`);
the room's menus offer no Quit there. The first game shows a note for 7 s
that Back or Select (Tab) goes back to the room (`html[data-back-hint]`).
A game is not started while nobody is seated: the room shows its notice
again. The page says what is missing when the browser has no WebGPU (with
Android's own advice: Chrome, or the Splitscreen Club app) or no adapter for
the GPU.

The cloud room (`examples/browser/cloud.rs`) does what the native host's
cloud bridge (gamenight `crates/gamenight-local-web/src/cloud.rs`) does:
it registers a room with a device id the page keeps in localStorage
(`/v1/lobbies/register`), then polls every 3 s (`/v1/lobbies/poll`) with
the seats and the games this table can play, and registers again after a
401. The reply gives the room code (the room shows it with its QR code, the
join link `{cloud}/?r=CODE`), the profiles waiting at the doors, the
profiles on seats (the player there takes their name, skin, face and
colour) and a game a phone picked (first in the queue, or started at once),
or a change to the queue. A pickup at a door in the room (Y) reaches the
page through `Lobby::outbox` (`room_pickup`) and claims the profile for
that seat (`/v1/lobbies/pickup`). A phone that asks to play on the phone
alone (the cloud's phone seat) gets a player without a controller, which
the page claims for it after the next poll; the room seats it on the
bench with the phones, and it leaves when its profile leaves. The cloud sends no CORS headers, so the
page must come from the cloud's own origin (behind the same web server) or
name it with `?cloud=`; without a cloud the room shows no code and plays on.
To test locally, run `gamenight-cloud` and serve `build/web` and `/v1` on
one port.

The page (`examples/browser/index.html`, with the room's fonts and icon) is
Splitscreen Club's loading screen until the room's first frame; later
loads (a game) show as a small card at the bottom. `?debug` shows the
engine log and buttons at the top that switch games directly
(`switch_game`) and go back to the room (`back_to_room`).

Resolution follows the native shell: a canvas taller than 1440 lines
starts its 3D view at 1440, and `app::quality::AdaptiveQuality` lowers it
(down to 0.67) while frames miss the game's target (60 fps unless it names
one), logging each change. On a MacBook Pro M5 Pro (Chrome, 2992x1704
canvas) the room is GPU-bound: 19.4 ms a frame at full size with about
5 ms on the page thread, 8.3 ms (120 Hz) at 1080 lines. Every 300 frames
the page logs the frame interval and the page thread's ticks, draw and
encode; `?render=H` fixes the 3D view's height to tell GPU from CPU. As
natively, the lower rungs also drop contact occlusion and then bloom.

Phones get a cheaper preset (`app::web::mobile`): an Android, iPhone or iPad
browser, or `?quality=mobile` or `?quality=low` anywhere (`?quality=full` turns
it off). It also switches the Game Room to its low tier (`Options::low`). The
3D view starts at 540 lines (or `?render=H`, which on a phone is the start
rather than a fixed height) and adaptive quality lowers it from there, and
every frame drops GI (its gather pass), contact and reflection occlusion,
traced reflections, bloom and lens flare. The page logs "phone preset" when
it is on. `--bare` builds `examples/warp_web.rs`
(crate `warp`) instead: the games and those buttons, no room. What is
left to do is in [browser-handoff.md](browser-handoff.md).

`--games a,b` picks which `games/` folders are served (default: the six
Luau games); `games/library.json` is the shelf (titles and player counts
from each `gamenight.json`). Measured 20 November 2026 in Chromium on
SwiftShader (no GPU, so frame times there mean nothing): wasm 27.3 MB,
Neon Siege's files fetched in 0.1 to 0.3 s, scripts loaded and `init` run
in 0.2 to 0.4 s; Play to Neon Siege's first frame 1.5 s; switching Stack
Together to Neon Siege took 0.27 s to load plus 0.17 s to its first frame.
On a real GPU (MacBook Pro M5 Pro, Chrome 156, 22 November 2026): the room
built in 0.8 s and ran at 56 fps at 2800x1626 (median 16.8 ms), Hello Warp
about 113 fps, Neon Siege 109 fps (median 8.4 ms, p95 16.3 ms), Play to
Neon Siege's first frame about 0.27 s, glass on (Chrome offers 32 sampled
textures there). The room looks as it does natively.

How the build differs from native:

- Luau is C++: `build-web.py --warp` compiles it against wasi-libc and
  libc++ for wasm32 (`LUAU_CXXFLAGS`, read by luau0-src) and links one
  `target/web-cxx/libstdc++.a` made of libc++, libc++abi and libc.
  `WASI_SYSROOT` points at a wasi-sdk `share/wasi-sysroot` instead of the
  Debian/Ubuntu packages (wasi-sdk 34 keeps libc++ without exceptions in
  `lib/wasm32-wasip1/noeh`; the script finds it there). On macOS put
  Homebrew's LLVM first on `PATH` (`/opt/homebrew/opt/llvm/bin`, for clang
  and llvm-ar): Apple's clang has no wasm target.
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
- Optional features are used when there: timestamps natively (not in a
  browser: Chrome 156 on an M5 Pro, Dawn on Metal, lost the device with
  "Insufficient Memory" as soon as a pass wrote timestamps, so
  `render/src/gpu.rs` leaves them out on wasm32 and a browser build has no
  GPU pass times); `primitive_index`
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
  before each long step. Warp games' own background work (skin maps, plant cooks)
  runs inline there too, and recorded samples are not fetched (the synth
  sounds play).
- Texture layers always stream in a browser (`texture_stream.rs`): a new
  renderer starts with each layer's small mips and `Stream::pump` bakes one
  layer a frame, so a scene is up in well under a second instead of the
  20+ s it took to bake every layer and Warp Human face at once. Faces bake
  at 128 texels on the body they already have (no refit or age settling,
  which took ~1.6 s a face) and once per page: a renderer rebuilt mid-game (a new look
  when a player joins) reuses them. Every renderer build of 500 ms or more
  logs its slowest steps (`renderer built in ...`), and every layer that
  took 50 ms or more logs `texture layer N baked in ...`. Mips there are
  plain 2x2 averages (`texture_mips::resize`), without the linear-RGB
  cutout pass, and the arrays stay RGBA8 (no BC7 encoding): each took
  seconds a layer on the page thread.
- Joining mid-game: a controller nobody plays with that is touched in a
  game shows "Press A to join this game" (the keyboard: Enter), or, when
  the game's `app::Game::takes_joins` is false, that A joins from the next
  game. A then seats them in the party and the page calls
  `app::web::player_joins`, which hands the running game
  `app::Game::player_joins(name, color)` before its next tick; the new
  player drives the game's next player in controls order. Luau games take
  up to four people through the page's seats and their `player_joined`
  hook; Spellstorm adds a wizard with random tomes; other Rust games keep
  the player for the next game. Under the desktop host (the daemon) the
  same happens without the page: the daemon seats a spare controller that
  presses A, and the game shows the hint itself as a toast
  (`splitscreen_club::party::Party::watch_joins`, `app::Game::notice`).
  Luau games and Spellstorm declare `participation` with `instant_join`
  (`Party::offer_joins(true)`) and take the new seat from `party_updated`;
  Space Racer and Growing Guns say "you play from the next game".
- Loading screen: no tagline; with `?game=NAME` it names the game and shows
  its cover (from games/library.json) while everything loads. The page
  registers the cloud room itself while the engine downloads
  (`window.splitscreenRoom`, POST /v1/lobbies/register with the device id
  in localStorage) and shows the phone QR code and room code at once
  (qrcode.js, qrcode-generator 1.4.4, MIT); the engine takes that token over
  (cloud.rs `register_early`) instead of registering again.
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
